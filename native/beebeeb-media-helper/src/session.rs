//! One playback: `Open` → the container's reader-based opener over [`crate::read::PipeReader`] → `Opened`; then
//! `WantVideo` / `WantAudio` / `Seek` answered in presentation order until the end (`Ended`) or `Close`.
//!
//! Before any decoder runs, one route is chosen for the whole file ([`crate::routes::decide`], the owner's ruling of
//! 2026-11-15 19:50): FilmCraft decodes every track that plays, or the operating system's decoder does
//! ([`crate::system::SystemStreams`], over the same ranges), or the file is refused with a code. An audio track that
//! does not play beside a video that does is named in `Opened::no_sound`, and the video plays without it.
//!
//! A bare audio stream (FLAC, MP3, ADTS, AIFF; [`crate::sniff`]) is known by its first bytes: its codec's route is
//! checked first, and only then does FilmCraft's whole-file opener get the made-up name it picks a decoder by. AC-3
//! has no such opener; its answer is the route table's. A bare stream on the system's route (ADTS AAC, E-AC-3) goes
//! to the operating system's reader where it runs, which also describes it (FilmCraft has no decoder for it). While a message is answered the [`Gate`] is open, so the
//! system decoder's own threads read only then.
//!
//! Never logs, never writes anything but protocol messages, holds no key, no name and no path.

use std::io::{Read, Write};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use beebeeb_media_protocol::{
    AudioInfo, Codec, ErrorCode, FromHelper, MAX_AUDIO_FRAMES, MAX_CHANNELS, MAX_FRAME_H, MAX_FRAME_W, MAX_RATE,
    MAX_SOURCE_SIDE, MEDIA_SYSTEM_DECODER_ON, Opened, Tags, ToHelper, VideoInfo,
};
use filmcraft_codecs::{CodecError, Mp4Source};
use filmcraft_media::reader::{HEAD_LEN, read_range};
use filmcraft_media::{FrameRequest, MediaError, SharedReader, SharedSource, VideoStreamInfo};

use crate::decode::{display_size, mix_channels, scale_letterbox};
use crate::mf::Unavailable;
use crate::mf::gate::Gate;
use crate::read::{PipeReader, ReadFlags, SharedWire, Wire, lock};
use crate::routes::{Decision, Track, decide, is_reduced_quality, state_for};
use crate::sniff::{Bare, program_stream_mpeg_audio, sniff};
use crate::system::{SystemOpen, SystemStreams};
use crate::{ROUTES, Route, SYSTEM_FALLBACK, system, tags};

/// A file no streaming opener takes is read whole only up to this many bytes; a larger one is `TooLarge`.
pub const MAX_WHOLE: u64 = 384 << 20;
/// FilmCraft's only words for a file over [`MAX_WHOLE`] that no streaming opener takes (`open_reader_within`, in
/// filmcraft-media's `reader.rs`: "… and it is too large to read whole here"). It has no typed error for it, so the
/// text is matched; `whole_file_formats_over_the_cap_answer_too_large` holds the match to FilmCraft's words.
const TOO_LARGE_WORDS: &str = "too large to read whole";
/// How FilmCraft's words for a stream its H.264 decoder refuses begin (`h264_error` in filmcraft-codecs `video.rs`:
/// "H.264: interlaced (field / MBAFF) coding", "H.264: bit depth luma 10 …"; no other message starts so). MPEG-TS and
/// MPEG-PS decode their first picture while the container opens, so there the refusal is the open's own error. It has
/// no typed error, so the words are matched: `an_h264_refusal_at_open_is_known_by_filmcrafts_words` holds this side,
/// the fixture row `h264-interlaced.ts` (`fixture_h264_interlaced`) FilmCraft's.
const H264_REFUSED: &str = "H.264: ";
/// The decoded-frame cache of FilmCraft's GOP decoder.
pub const FRAME_BUDGET: usize = 256 << 20;

/// An MP4 or MOV over the app's ranges: only the index is read when it opens, samples when they are asked for. The
/// helper has no name for the file: FilmCraft's errors name it "".
pub fn open_mp4(reader: PipeReader) -> Result<Mp4Source, CodecError> {
    let shared: SharedReader = Arc::new(reader);
    Mp4Source::open_reader("", shared)
}

struct Playing {
    /// FilmCraft's source; `None` only for a bare stream that the system's decoder alone reads (row 14 of MCP2).
    source: Option<SharedSource>,
    /// The video stream's own description: its frame rate is the clock of the frames (no direct dependency on the
    /// time crate: `frame_rate.sane().tick_of(n)` is all this needs).
    video: Option<VideoStreamInfo>,
    has_audio: bool,
    duration_ms: u64,
    video_index: i64,
    /// The audio cursor: `origin_ms` plus `done` frames at `rate`.
    audio_rate: u32,
    audio_origin_ms: u64,
    audio_done: u64,
    flags: Arc<ReadFlags>,
    /// The operating system's decoder, when the whole file is on the system's route; FilmCraft's otherwise.
    system: Option<SystemStreams>,
}

/// Serve one app over standard input and output with the shipped route table.
pub fn run(input: impl Read + Send + 'static, output: impl Write + Send + 'static) {
    run_with(input, output, ROUTES);
}

/// Serve one app with `table` as the routes (a test hands its own) and the shipped switch for the system's decoders
/// (`MEDIA_SYSTEM_DECODER_ON`). Returns when the app closes, the pipe ends or a message is not one this protocol
/// reads.
pub fn run_with(input: impl Read + Send + 'static, output: impl Write + Send + 'static, table: &[(Codec, Route)]) {
    run_with_system(input, output, table, MEDIA_SYSTEM_DECODER_ON);
}

/// [`run_with`] with the switch given: `system_on` false never starts the operating system's decoders, whatever
/// this process could do (contract C6; the Windows tests of the system route pass `true`).
pub fn run_with_system(
    input: impl Read + Send + 'static,
    output: impl Write + Send + 'static,
    table: &[(Codec, Route)],
    system_on: bool,
) {
    let wire: SharedWire = Arc::new(Mutex::new(Wire::new(input, output)));
    let flags = Arc::new(ReadFlags::default());
    let gate = Arc::new(Gate::new());
    let mut playing: Option<Playing> = None;
    serve(&wire, &flags, &gate, &mut playing, table, system_on);
    // The system decoder's threads may still wait to read: from now on their reads fail at once, so releasing the
    // decoders does not wait on them.
    gate.shut();
    drop(playing);
}

fn serve(
    wire: &SharedWire,
    flags: &Arc<ReadFlags>,
    gate: &Arc<Gate>,
    playing: &mut Option<Playing>,
    table: &[(Codec, Route)],
    system_on: bool,
) {
    loop {
        let msg = lock(wire).recv();
        // Reads reach the app only while this message is answered (also the system decoder's, from its threads).
        gate.open();
        let reply = match msg {
            Err(_) | Ok(ToHelper::Close) => return,
            Ok(ToHelper::Open { len, .. }) => {
                if playing.is_some() {
                    vec![FromHelper::Error(ErrorCode::Internal)]
                } else {
                    match open(wire, flags, gate, len, table, system_on) {
                        Ok((p, opened, tags)) => {
                            *playing = Some(p);
                            // The title and artist follow `Opened` (the app shows them once it has the file).
                            let mut reply = vec![FromHelper::Opened(opened)];
                            reply.extend(tags.map(FromHelper::Tags));
                            reply
                        }
                        Err(code) => vec![FromHelper::Error(code)],
                    }
                }
            }
            Ok(ToHelper::WantVideo { w, h }) => vec![answer(playing.as_mut().map(|p| want_video(p, w, h)))],
            Ok(ToHelper::WantAudio { rate, channels, frames }) => {
                vec![answer(playing.as_mut().map(|p| want_audio(p, rate, channels, frames)))]
            }
            Ok(ToHelper::Seek { at_ms }) => {
                if let Some(p) = playing.as_mut() {
                    seek(p, at_ms);
                }
                Vec::new()
            }
            // An answer nobody asked for: ignored.
            Ok(ToHelper::Data { .. } | ToHelper::DataFailed { .. }) => Vec::new(),
        };
        // A read still running finishes first; none starts until the next message.
        gate.close();
        for msg in reply {
            if lock(wire).send(&msg).is_err() {
                return;
            }
        }
    }
}

fn answer(result: Option<Result<FromHelper, ErrorCode>>) -> FromHelper {
    match result {
        None => FromHelper::Error(ErrorCode::Internal),
        Some(Ok(msg)) => msg,
        Some(Err(code)) => FromHelper::Error(code),
    }
}

fn open(
    wire: &SharedWire,
    flags: &Arc<ReadFlags>,
    gate: &Arc<Gate>,
    len: u64,
    table: &[(Codec, Route)],
    system_on: bool,
) -> Result<(Playing, Opened, Option<Tags>), ErrorCode> {
    if len == 0 {
        return Err(ErrorCode::Corrupt);
    }
    // First open sets the pool's size; the helper opens one file.
    filmcraft_codecs::set_frame_budget(FRAME_BUDGET);
    // One reader of the app's ranges for FilmCraft and the system's decoder alike.
    let pipe = Arc::new(PipeReader::new(wire.clone(), len, flags.clone()));
    let reader: SharedReader = pipe.clone();

    // The same first bytes FilmCraft sniffs (its read is then the cache's): a bare stream's route comes first, before
    // FilmCraft reads it whole and decodes it.
    let head = read_range(&*reader, 0, HEAD_LEN).map_err(|_| ErrorCode::ReadFailed)?;
    let bare = sniff(&head);
    let program_layer = program_stream_mpeg_audio(&head);
    drop(head);
    if let Some(bare) = bare {
        match bare_route(table, bare, system_on, || system::availability(bare.codec())) {
            BareRoute::Ours => {}
            BareRoute::System(codec) => return open_bare_on_system(&pipe, gate, flags, bare, codec),
            BareRoute::Refused(code) => return Err(code),
        }
    }
    let source = match filmcraft_source(reader, bare) {
        Ok(source) => source,
        Err(e) if h264_refused(&e) && map_error(&e, flags) != ErrorCode::ReadFailed => {
            return Err(refused_at_open(table, system_on, h264_ten_bit(&e)));
        }
        Err(e) => return Err(map_error(&e, flags)),
    };
    let info = source.info().clone();

    let video_track = match &info.video {
        Some(v) => Some(track_of_video(table, &source, v, flags)?),
        None => None,
    };
    let audio_track = info
        .audio()
        .map(|a| track_of_audio(table, &a.codec, &info.container, program_layer));
    let needs_codec = [video_track, audio_track]
        .into_iter()
        .flatten()
        .find(|t| t.needs_system())
        .map_or(Codec::Other, |t| t.codec);
    let state = state_for(video_track, audio_track, system_on, || {
        system::availability(needs_codec)
    });
    let (plan, on_system) = match decide(video_track, audio_track, state) {
        Decision::Ours(plan) => (plan, false),
        Decision::System(plan) => (plan, true),
        Decision::Error(code) => return Err(code),
    };

    let video = match (&info.video, plan.video) {
        (Some(v), Some(codec)) => {
            if v.width == 0 || v.height == 0 {
                return Err(ErrorCode::Corrupt);
            }
            if v.width > MAX_SOURCE_SIDE || v.height > MAX_SOURCE_SIDE {
                return Err(ErrorCode::TooLarge);
            }
            let fps_milli = (v.frame_rate.sane().as_f64() * 1000.0)
                .round()
                .clamp(1.0, f64::from(u32::MAX)) as u32;
            Some((
                VideoInfo {
                    w: v.width,
                    h: v.height,
                    fps_milli,
                    codec,
                },
                v.clone(),
            ))
        }
        _ => None,
    };
    let (mut audio, mut reduced_quality) = match (info.audio(), plan.audio) {
        (Some(a), Some(codec)) => {
            let channels = a.channels.clamp(1, u32::from(MAX_CHANNELS)) as u16;
            let rate = a.sample_rate.clamp(1, MAX_RATE);
            let reduced = reduced_quality(&a.codec, on_system);
            (Some(AudioInfo { rate, channels, codec }), reduced)
        }
        _ => (None, false),
    };
    if video.is_none() && audio.is_none() {
        return Err(ErrorCode::NoDecoder);
    }
    // The wire takes a silent track only beside a video that plays and without audio; the decision gives no other.
    let mut no_sound = plan.no_sound.filter(|_| video.is_some() && audio.is_none());
    let duration_ms = (info.duration.seconds() * 1000.0).round().max(0.0) as u64;
    let system = if on_system {
        let open = |audio: Option<Codec>| {
            SystemStreams::open(SystemOpen {
                pipe: &pipe,
                gate,
                flags,
                container: &info.container,
                video: plan.video.filter(|_| video.is_some()),
                audio,
            })
        };
        let want_audio = plan.audio.filter(|_| audio.is_some());
        Some(match open(want_audio) {
            // ux-57: Windows has no decoder for the audio, but the video plays: open the video alone and say so.
            // A video that cannot open either answers its own refusal from the second try.
            Err(ErrorCode::NoSystemDecoder(codec) | ErrorCode::NoSystemMedia(codec))
                if video.is_some() && want_audio == Some(codec) =>
            {
                let streams = open(None)?;
                audio = None;
                reduced_quality = false;
                no_sound = Some(codec);
                streams
            }
            other => other?,
        })
    } else {
        None
    };
    let opened = Opened {
        duration_ms,
        video: video.as_ref().map(|(v, _)| *v),
        audio,
        reduced_quality,
        no_sound,
    };
    // An audio file's title and artist (D-MB2-1 (a)); a video shows its file name.
    let tags = if opened.video.is_none() {
        tags::read(pipe.clone())
    } else {
        None
    };
    let playing = Playing {
        source: Some(source),
        video: video.map(|(_, stream)| stream),
        // No audio stream at all when the file's sound does not play here.
        has_audio: audio.is_some(),
        duration_ms,
        video_index: 0,
        audio_rate: audio.map_or(48_000, |a| a.rate),
        audio_origin_ms: 0,
        audio_done: 0,
        flags: flags.clone(),
        system,
    };
    Ok((playing, opened, tags))
}

/// Where a bare stream goes: FilmCraft's whole-file opener, the operating system's decoder, or a refusal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BareRoute {
    Ours,
    System(Codec),
    Refused(ErrorCode),
}

/// The route of a bare stream of `bare`'s codec: the same decision as a file's audio-only track ([`decide`]), with
/// `ask` (this process's Media Foundation) asked only when the switch is on and the codec needs the system's decoder.
fn bare_route(
    table: &[(Codec, Route)],
    bare: Bare,
    system_on: bool,
    ask: impl FnOnce() -> Result<(), Unavailable>,
) -> BareRoute {
    let track = Some(Track::of(table, bare.codec()));
    let state = state_for(None, track, system_on, ask);
    match decide(None, track, state) {
        Decision::Ours(_) => BareRoute::Ours,
        Decision::System(plan) => BareRoute::System(plan.audio.unwrap_or(bare.codec())),
        Decision::Error(code) => BareRoute::Refused(code),
    }
}

/// A bare stream on the system's route (ADTS AAC or E-AC-3, with the switch on and Media Foundation running; row 14
/// of MCP2): Windows' reader opens it over the same ranges and describes it, since FilmCraft has no decoder for it. A
/// PC without that decoder answers `NoSystemDecoder(codec)` (the reader's own refusal), a stream Windows cannot read
/// its code; nothing else does.
fn open_bare_on_system(
    pipe: &Arc<PipeReader>,
    gate: &Arc<Gate>,
    flags: &Arc<ReadFlags>,
    bare: Bare,
    codec: Codec,
) -> Result<(Playing, Opened, Option<Tags>), ErrorCode> {
    let system = SystemStreams::open(SystemOpen {
        pipe,
        gate,
        flags,
        container: bare.container(),
        video: None,
        audio: Some(codec),
    })?;
    let (rate, channels) = system.audio_format().ok_or(ErrorCode::NoSystemDecoder(codec))?;
    let audio = AudioInfo {
        rate: rate.clamp(1, MAX_RATE),
        channels: channels.clamp(1, MAX_CHANNELS),
        codec,
    };
    let duration_ms = system.duration_ms().unwrap_or(0);
    let opened = Opened {
        duration_ms,
        video: None,
        audio: Some(audio),
        // The system's decoder decodes the whole stream (HE-AAC with its SBR).
        reduced_quality: reduced_quality("", true),
        no_sound: None,
    };
    let playing = Playing {
        source: None,
        video: None,
        has_audio: true,
        duration_ms,
        video_index: 0,
        audio_rate: audio.rate,
        audio_origin_ms: 0,
        audio_done: 0,
        flags: flags.clone(),
        system: Some(system),
    };
    let tags = tags::read(pipe.clone());
    Ok((playing, opened, tags))
}

/// Whether the audio plays below the file's own quality here (`Opened::reduced_quality`, decision D-MED-7): HE-AAC
/// that FilmCraft decodes, at its AAC-LC core (it decodes no SBR). Never on the system's decoder, which decodes HE-AAC
/// in full (row 17 of MCP2). Only explicit signalling counts: FilmCraft's "HE-AAC" label (an MP4 whose
/// AudioSpecificConfig says SBR or PS). HE-AAC that only a decoder finds (implicit SBR in ADTS, LATM or Matroska,
/// labelled plain "AAC") is never guessed.
fn reduced_quality(label: &str, on_system: bool) -> bool {
    !on_system && is_reduced_quality(label)
}

/// FilmCraft's source over the app's ranges. A sniffed bare stream gets the name its whole-file opener picks a
/// decoder by ([`Bare::name`]); every other file keeps the name "" (the helper has none). The name goes nowhere else.
fn filmcraft_source(reader: SharedReader, bare: Option<Bare>) -> Result<SharedSource, MediaError> {
    let name = bare.and_then(Bare::name).unwrap_or("");
    filmcraft_media::reader::open_reader_within(
        name,
        reader,
        &filmcraft_codecs::reader_openers(),
        &filmcraft_codecs::openers(),
        MAX_WHOLE,
    )
}

/// Whether FilmCraft's error is its H.264 decoder refusing the stream ([`H264_REFUSED`]).
fn h264_refused(e: &MediaError) -> bool {
    matches!(e, MediaError::Unsupported(s) if s.starts_with(H264_REFUSED))
}

/// Whether FilmCraft's H.264 refusal is the stream's bit depth ("H.264: bit depth luma 10 ...").
fn h264_ten_bit(e: &MediaError) -> bool {
    matches!(e, MediaError::Unsupported(s) if s.starts_with("H.264: bit depth"))
}

/// The answer for a container whose H.264 video FilmCraft refused while it opened (MPEG-TS/PS: interlaced, 10-bit).
/// The video decides first, so the audio (unknown here) does not change it: a refused H.264 track needs the system's
/// decoder (row 19), and without it here the answer names H.264. Where the system's decoder runs, Windows' reader
/// over a file FilmCraft could not describe is not wired: the same honest `NoSystemDecoder(H264)`.
fn refused_at_open(table: &[(Codec, Route)], system_on: bool, ten_bit: bool) -> ErrorCode {
    let video = Some(Track {
        refused: true,
        ten_bit,
        ..Track::of(table, Codec::H264)
    });
    let state = state_for(video, None, system_on, || system::availability(Codec::H264));
    match decide(video, None, state) {
        Decision::Error(code) => code,
        Decision::Ours(_) | Decision::System(_) => ErrorCode::NoSystemDecoder(Codec::H264),
    }
}

/// FilmCraft's names for MPEG-TS and MPEG-PS (`Format::name` in filmcraft-mpegts).
const PROGRAM_STREAMS: [&str; 4] = ["MPEG-2 TS", "MPEG-2 TS (BDAV/AVCHD)", "MPEG-1 System", "MPEG-2 PS"];

/// The file's audio track for the decision. MPEG-TS and MPEG-PS name every MPEG audio layer "MPEG Audio", which reads
/// as MP1/MP2; there the frame header in the file's first audio packets says the layer
/// ([`program_stream_mpeg_audio`], row 15 of MCP2). Undetermined, it stays MP2 (both are FilmCraft's).
fn track_of_audio(table: &[(Codec, Route)], label: &str, container: &str, program_layer: Option<Bare>) -> Track {
    let track = Track::of_label(table, label);
    if track.codec == Codec::Mp2
        && label.eq_ignore_ascii_case("MPEG Audio")
        && PROGRAM_STREAMS.contains(&container)
        && program_layer == Some(Bare::Mp3)
    {
        return Track::of(table, Codec::Mp3);
    }
    track
}

/// The file's video track for the decision. FilmCraft's H.264 decoder checks the stream's parameters (interlaced,
/// 10-bit) only when it decodes, and MP4, Matroska and AVI open without decoding, so a codec that falls back to the
/// system's decoder ([`SYSTEM_FALLBACK`]) has its first frame decoded here: refused, it is a track that needs the
/// system's decoder (row 19). The frame stays in FilmCraft's cache for the first `WantVideo`. A size outside the
/// protocol's limits is not decoded at all: `open` refuses it as before.
fn track_of_video(
    table: &[(Codec, Route)],
    source: &SharedSource,
    v: &VideoStreamInfo,
    flags: &ReadFlags,
) -> Result<Track, ErrorCode> {
    let mut track = Track::of_label(table, &v.codec);
    let sized = (1..=MAX_SOURCE_SIDE).contains(&v.width) && (1..=MAX_SOURCE_SIDE).contains(&v.height);
    if track.route != Route::Rust || !SYSTEM_FALLBACK.contains(&track.codec) || !sized {
        return Ok(track);
    }
    match source.video_frame(FrameRequest::full(v.frame_rate.sane().tick_of(0))) {
        Ok(_) => {}
        Err(e) => match map_error(&e, flags) {
            ErrorCode::ReadFailed => return Err(ErrorCode::ReadFailed),
            // A damaged first frame is answered where it always was: at the first `WantVideo`.
            _ => {
                track.refused = matches!(e, MediaError::Unsupported(_));
                track.ten_bit = track.refused && h264_ten_bit(&e);
            }
        },
    }
    Ok(track)
}

fn want_video(p: &mut Playing, w: u32, h: u32) -> Result<FromHelper, ErrorCode> {
    if w == 0 || h == 0 || w > MAX_FRAME_W || h > MAX_FRAME_H {
        return Err(ErrorCode::Internal);
    }
    if let Some(system) = p.system.as_mut().filter(|s| s.has_video()) {
        return system.video_frame(w, h);
    }
    let Some(video) = p.video.as_ref() else {
        return Ok(FromHelper::Ended);
    };
    let tick = video.frame_rate.sane().tick_of(p.video_index);
    let pts_ms = (tick.seconds() * 1000.0).round().max(0.0) as u64;
    if p.duration_ms > 0 && pts_ms >= p.duration_ms {
        return Ok(FromHelper::Ended);
    }
    let frame = p
        .source
        .as_ref()
        .ok_or(ErrorCode::Internal)?
        .video_frame(FrameRequest::full(tick))
        .map_err(|e| map_error(&e, &p.flags))?;
    p.video_index += 1;
    let rgba = frame.to_rgba8();
    let shown = display_size(frame.width, frame.height, frame.par);
    let rgba = scale_letterbox(&rgba, frame.width, frame.height, shown, w, h).ok_or(ErrorCode::Corrupt)?;
    Ok(FromHelper::Frame { pts_ms, w, h, rgba })
}

fn want_audio(p: &mut Playing, rate: u32, channels: u16, frames: u32) -> Result<FromHelper, ErrorCode> {
    if rate == 0 || rate > MAX_RATE || channels == 0 || channels > MAX_CHANNELS || frames == 0 {
        return Err(ErrorCode::Internal);
    }
    if let Some(system) = p.system.as_mut().filter(|s| s.has_audio()) {
        return system.audio(rate, channels, frames);
    }
    if !p.has_audio {
        return Ok(FromHelper::Ended);
    }
    if rate != p.audio_rate {
        // The app asks at another rate: carry the position over in milliseconds.
        p.audio_origin_ms += p.audio_done * 1000 / u64::from(p.audio_rate);
        p.audio_done = 0;
        p.audio_rate = rate;
    }
    let start = p.audio_origin_ms * u64::from(rate) / 1000 + p.audio_done;
    let start_ms = start * 1000 / u64::from(rate);
    if p.duration_ms > 0 && start_ms >= p.duration_ms {
        return Ok(FromHelper::Ended);
    }
    let asked = frames.min(MAX_AUDIO_FRAMES) as usize;
    let buffer = p
        .source
        .as_ref()
        .ok_or(ErrorCode::Internal)?
        .audio(i64::try_from(start).unwrap_or(i64::MAX), asked, rate)
        .map_err(|e| map_error(&e, &p.flags))?;
    if buffer.frames() == 0 {
        return Ok(FromHelper::Ended);
    }
    let samples = mix_channels(&buffer.channels, channels, asked);
    p.audio_done += (samples.len() / usize::from(channels)) as u64;
    Ok(FromHelper::Audio {
        pts_ms: start_ms,
        channels,
        samples,
    })
}

fn seek(p: &mut Playing, at_ms: u64) {
    let at_ms = if p.duration_ms > 0 {
        at_ms.min(p.duration_ms)
    } else {
        at_ms
    };
    if let Some(system) = p.system.as_mut() {
        system.seek(at_ms);
    }
    if let Some(video) = p.video.as_ref() {
        let rate = video.frame_rate.sane();
        let index = i128::from(at_ms) * i128::from(rate.num) / (1000 * i128::from(rate.den));
        p.video_index = i64::try_from(index).unwrap_or(i64::MAX);
    }
    p.audio_origin_ms = at_ms;
    p.audio_done = 0;
}

/// FilmCraft's error as the protocol's code. What the pipe did (a failed range, the app gone) wins over what the
/// decoder made of it.
fn map_error(e: &MediaError, flags: &ReadFlags) -> ErrorCode {
    if flags.failed.load(Ordering::Relaxed) || flags.closed.load(Ordering::Relaxed) {
        return ErrorCode::ReadFailed;
    }
    match e {
        MediaError::Io(_) | MediaError::Offline(_) => ErrorCode::ReadFailed,
        MediaError::Decode(_) => ErrorCode::Corrupt,
        MediaError::Unsupported(s) if s.contains(TOO_LARGE_WORDS) => ErrorCode::TooLarge,
        MediaError::Unsupported(_) | MediaError::NoStream(_) => ErrorCode::NoDecoder,
        MediaError::Cancelled => ErrorCode::Internal,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;

    use super::*;

    /// A file of `MAX_WHOLE + 1` bytes whose first bytes are a FLAC stream's (`fLaC`, then zeros): every read is
    /// answered, and the largest one is kept.
    struct HugeFlac {
        largest: AtomicUsize,
    }

    impl filmcraft_media::ByteReader for HugeFlac {
        fn len(&self) -> u64 {
            MAX_WHOLE + 1
        }

        fn read_at(&self, offset: u64, buf: &mut [u8]) -> std::io::Result<()> {
            let end = offset
                .checked_add(buf.len() as u64)
                .ok_or(std::io::ErrorKind::InvalidInput)?;
            if end > self.len() {
                return Err(std::io::ErrorKind::UnexpectedEof.into());
            }
            self.largest.fetch_max(buf.len(), Ordering::Relaxed);
            buf.fill(0);
            let magic = b"fLaC";
            if let Ok(start) = usize::try_from(offset)
                && start < magic.len()
            {
                let n = (magic.len() - start).min(buf.len());
                buf[..n].copy_from_slice(&magic[start..start + n]);
            }
            Ok(())
        }
    }

    /// Row 39: a bare FLAC stream over the whole-file cap is `TooLarge` (FilmCraft refuses before it reads the file
    /// whole: `reader.rs:242`), never `NoDecoder`; FilmCraft's words are what the match holds to.
    #[test]
    fn whole_file_formats_over_the_cap_answer_too_large() {
        let reader = Arc::new(HugeFlac {
            largest: AtomicUsize::new(0),
        });
        let shared: SharedReader = reader.clone();
        let head = read_range(&*shared, 0, HEAD_LEN).expect("the head reads");
        let bare = sniff(&head);
        assert_eq!(bare, Some(Bare::Flac));
        let answer = filmcraft_source(shared, bare)
            .err()
            .map(|e| map_error(&e, &ReadFlags::default()));
        assert_eq!(answer, Some(ErrorCode::TooLarge));
        let largest = reader.largest.load(Ordering::Relaxed);
        assert!(largest <= HEAD_LEN, "a read of {largest} bytes: more than the head");
    }

    /// Row 16 (MCP2): what row 19's open-time probe costs, per committed fixture: the first-frame decode of
    /// `track_of_video`, timed around the function itself, three times each with a fresh source (the first is cold).
    /// A measurement, not a gate: its numbers go to the branch notes. The lead's ruling: over 100 ms on any fixture,
    /// the probe moves to the first play.
    #[test]
    #[ignore = "a measurement (row 16): cargo test --lib open_time_probe -- --ignored --nocapture"]
    fn open_time_probe_cost_per_fixture() {
        use std::time::Instant;

        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        let manifest = std::fs::read_to_string(dir.join("manifest.tsv")).expect("the fixtures are committed");
        let names: Vec<&str> = manifest
            .lines()
            .filter(|line| !line.starts_with('#'))
            .filter_map(|line| line.split('\t').next())
            .collect();
        assert_eq!(names.len(), 26, "the 26 fixture rows");
        let mut probed = 0;
        println!("file\topen ms\tprobe ms (3 runs)\twhat");
        for name in names {
            let bytes: Arc<[u8]> = std::fs::read(dir.join(name)).expect(name).into();
            let mut runs = Vec::new();
            let mut what = String::new();
            let mut open_ms = 0.0;
            for _ in 0..3 {
                let reader: SharedReader = Arc::new(filmcraft_media::reader::MemReader(bytes.clone()));
                let head = read_range(&*reader, 0, HEAD_LEN).expect("the head reads");
                let started = Instant::now();
                let source = match filmcraft_source(reader, sniff(&head)) {
                    Ok(source) => source,
                    Err(e) => {
                        what = format!("refused at open ({:?})", map_error(&e, &ReadFlags::default()));
                        break;
                    }
                };
                open_ms = started.elapsed().as_secs_f64() * 1000.0;
                let Some(v) = source.info().video.clone() else {
                    what = "no video: no probe".into();
                    break;
                };
                let track = Track::of_label(ROUTES, &v.codec);
                if track.route != Route::Rust || !SYSTEM_FALLBACK.contains(&track.codec) {
                    what = format!("{:?} on {:?}: no probe", track.codec, track.route);
                    break;
                }
                let started = Instant::now();
                let probe = track_of_video(ROUTES, &source, &v, &ReadFlags::default());
                runs.push(started.elapsed().as_secs_f64() * 1000.0);
                what = format!("{} {}x{}: {probe:?}", v.codec, v.width, v.height);
            }
            if !runs.is_empty() {
                probed += 1;
            }
            let runs: Vec<String> = runs.iter().map(|ms| format!("{ms:.1}")).collect();
            println!("{name}\t{open_ms:.1}\t{}\t{what}", runs.join(" / "));
        }
        println!("probed: {probed} of 26");
        assert!(probed >= 4, "the H.264 fixtures in MP4, Matroska and AVI are probed");
    }

    /// Row 14 (MCP2): a bare ADTS AAC or E-AC-3 stream goes to the system's decoder where it runs (it was refused
    /// with `NoSystemDecoder` before); `NoSystemDecoder` / `NoSystemMedia` only where the system's decoder is not
    /// there, and with the switch off Media Foundation is never asked.
    #[test]
    fn a_bare_system_stream_goes_to_the_system_decoder_where_it_runs() {
        const TABLE: &[(Codec, Route)] = &[
            (Codec::Aac, Route::System),
            (Codec::Eac3, Route::System),
            (Codec::Flac, Route::Rust),
            (Codec::Ac3, Route::Unavailable),
        ];
        for bare in [Bare::Aac, Bare::Eac3] {
            let codec = bare.codec();
            assert_eq!(bare_route(TABLE, bare, true, || Ok(())), BareRoute::System(codec));
            assert_eq!(
                bare_route(TABLE, bare, true, || Err(Unavailable::NotHere)),
                BareRoute::Refused(ErrorCode::NoSystemDecoder(codec))
            );
            assert_eq!(
                bare_route(TABLE, bare, true, || Err(Unavailable::NoSystemMedia)),
                BareRoute::Refused(ErrorCode::NoSystemMedia(codec))
            );
            assert_eq!(
                bare_route(TABLE, bare, false, || panic!(
                    "the switch is off: Media Foundation is never asked"
                )),
                BareRoute::Refused(ErrorCode::NoSystemDecoder(codec))
            );
        }
        // FilmCraft's own codecs and the compiled-out ones never reach the system's decoder, and never ask.
        assert_eq!(
            bare_route(TABLE, Bare::Flac, true, || panic!("FilmCraft's own")),
            BareRoute::Ours
        );
        assert_eq!(
            bare_route(TABLE, Bare::Ac3, true, || panic!("not played here")),
            BareRoute::Refused(ErrorCode::Unsupported(Codec::Ac3))
        );
    }

    /// Row 17 (MCP2): HE-AAC is "reduced quality" only where FilmCraft decodes it and only from the explicit label;
    /// on the system's decoder (the bare-stream path included) never.
    #[test]
    fn he_aac_is_reduced_only_on_filmcrafts_decoder_and_only_when_labelled() {
        assert!(reduced_quality("HE-AAC", false));
        assert!(!reduced_quality("HE-AAC", true));
        for label in ["AAC", "AAC (ADTS)", "AAC (LATM)", "", "MP3"] {
            assert!(!reduced_quality(label, false), "{label}");
            assert!(!reduced_quality(label, true), "{label}");
        }
    }

    /// Row 15 (MCP2): in MPEG-TS/PS an "MPEG Audio" track whose packets say Layer III is MP3; anywhere else, or
    /// undetermined, the label's answer stands.
    #[test]
    fn mp3_in_a_transport_or_program_stream_reads_as_mp3() {
        for container in PROGRAM_STREAMS {
            let track = track_of_audio(ROUTES, "MPEG Audio", container, Some(Bare::Mp3));
            assert_eq!(track.codec, Codec::Mp3, "{container}");
            assert_eq!(track.route, Route::Rust);
            let undetermined = track_of_audio(ROUTES, "MPEG Audio", container, None);
            assert_eq!(undetermined.codec, Codec::Mp2, "{container}");
            let layer2 = track_of_audio(ROUTES, "MPEG Audio", container, Some(Bare::Mp2));
            assert_eq!(layer2.codec, Codec::Mp2, "{container}");
        }
        // Matroska, MP4 and AVI name MP3 apart: "MPEG Audio" there is Layer I/II, whatever bytes look like.
        for container in ["Matroska", "MPEG-4", "AVI"] {
            let track = track_of_audio(ROUTES, "MPEG Audio", container, Some(Bare::Mp3));
            assert_eq!(track.codec, Codec::Mp2, "{container}");
        }
        // Other labels in a transport stream are untouched.
        let aac = track_of_audio(ROUTES, "AAC (ADTS)", "MPEG-2 TS", Some(Bare::Mp3));
        assert_eq!(aac.codec, Codec::Aac);
        // FilmCraft's own names for the two committed MPEG-TS / MPEG-PS fixtures are the ones matched.
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        for name in ["h264-aac.ts", "mpeg2.mpg"] {
            let bytes: Arc<[u8]> = std::fs::read(dir.join(name)).expect(name).into();
            let source = filmcraft_source(Arc::new(filmcraft_media::reader::MemReader(bytes)), None)
                .unwrap_or_else(|e| panic!("{name} opens: {e}"));
            let container = source.info().container.clone();
            assert!(PROGRAM_STREAMS.contains(&container.as_str()), "{name}: {container}");
        }
    }

    /// Row 19 in MPEG-TS/PS: FilmCraft's own words for an H.264 stream its decoder refuses (as `h264_error` writes
    /// them) are a refused H.264 track, and the answer names H.264 by the system state; any other refusal is not.
    #[test]
    fn an_h264_refusal_at_open_is_known_by_filmcrafts_words() {
        for words in [
            "H.264: interlaced (field / MBAFF) coding",
            "H.264: bit depth luma 10 chroma 10 (only 8-bit is implemented)",
        ] {
            assert!(h264_refused(&MediaError::Unsupported(words.into())), "{words}");
        }
        for other in [
            "H264 video is not in this build",
            ": unrecognised format",
            "HEVC: interlaced",
            "no decoder for H.264: video",
        ] {
            assert!(!h264_refused(&MediaError::Unsupported(other.into())), "{other}");
        }
        assert!(!h264_refused(&MediaError::Decode("H.264: bad slice".into())));
        assert!(h264_ten_bit(&MediaError::Unsupported("H.264: bit depth luma 10 chroma 10".into())));
        assert!(!h264_ten_bit(&MediaError::Unsupported("H.264: interlaced (field / MBAFF) coding".into())));
        assert_eq!(refused_at_open(ROUTES, false, true), ErrorCode::TenBit);

        // H.264 is FilmCraft's and falls back: with the switch off the refusal names it (M9's interlaced row).
        assert_eq!(refused_at_open(ROUTES, false, false), ErrorCode::NoSystemDecoder(Codec::H264));
        assert_eq!(
            refused_at_open(&[(Codec::H264, Route::Rust)], false, false),
            ErrorCode::NoSystemDecoder(Codec::H264)
        );
        // An H.264 row that is off answers as any video that is not played here.
        assert_eq!(
            refused_at_open(&[(Codec::H264, Route::Unavailable)], false, false),
            ErrorCode::Unsupported(Codec::H264)
        );
    }
}
