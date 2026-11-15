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
//! has no such opener; its answer is the route table's. While a message is answered the [`Gate`] is open, so the
//! system decoder's own threads read only then.
//!
//! Never logs, never writes anything but protocol messages, holds no key, no name and no path.

use std::io::{Read, Write};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use beebeeb_media_protocol::{
    AudioInfo, Codec, ErrorCode, FromHelper, MAX_AUDIO_FRAMES, MAX_CHANNELS, MAX_FRAME_H, MAX_FRAME_W, MAX_RATE,
    MAX_SOURCE_SIDE, MEDIA_SYSTEM_DECODER_ON, Opened, ToHelper, VideoInfo,
};
use filmcraft_codecs::{CodecError, Mp4Source};
use filmcraft_media::reader::{HEAD_LEN, read_range};
use filmcraft_media::{FrameRequest, MediaError, SharedReader, SharedSource, VideoStreamInfo};

use crate::decode::{display_size, mix_channels, scale_letterbox};
use crate::mf::gate::Gate;
use crate::read::{PipeReader, ReadFlags, SharedWire, Wire, lock};
use crate::routes::{Decision, Track, decide, is_reduced_quality, state_for};
use crate::sniff::{Bare, sniff};
use crate::system::{SystemOpen, SystemStreams};
use crate::{ROUTES, Route, SYSTEM_FALLBACK, system};

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
    source: SharedSource,
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
                        Ok((p, opened)) => {
                            *playing = Some(p);
                            vec![FromHelper::Opened(opened)]
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
) -> Result<(Playing, Opened), ErrorCode> {
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
    drop(head);
    if let Some(bare) = bare {
        let track = Some(Track::of(table, bare.codec()));
        let state = state_for(None, track, system_on, || system::availability(bare.codec()));
        match decide(None, track, state) {
            Decision::Ours(_) => {}
            // Windows' reader over a bare elementary stream is not wired, and FilmCraft cannot describe one it has
            // no decoder for: the honest answer for this helper.
            Decision::System(_) => return Err(ErrorCode::NoSystemDecoder(bare.codec())),
            Decision::Error(code) => return Err(code),
        }
    }
    let source = match filmcraft_source(reader, bare) {
        Ok(source) => source,
        Err(e) if h264_refused(&e) && map_error(&e, flags) != ErrorCode::ReadFailed => {
            return Err(refused_at_open(table, system_on));
        }
        Err(e) => return Err(map_error(&e, flags)),
    };
    let info = source.info().clone();

    let video_track = match &info.video {
        Some(v) => Some(track_of_video(table, &source, v, flags)?),
        None => None,
    };
    let audio_track = info.audio().map(|a| Track::of_label(table, &a.codec));
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
    let (audio, reduced_quality) = match (info.audio(), plan.audio) {
        (Some(a), Some(codec)) => {
            let channels = a.channels.clamp(1, u32::from(MAX_CHANNELS)) as u16;
            let rate = a.sample_rate.clamp(1, MAX_RATE);
            // Windows decodes HE-AAC in full; only FilmCraft plays it at its AAC-LC core.
            let reduced = is_reduced_quality(&a.codec) && !on_system;
            (Some(AudioInfo { rate, channels, codec }), reduced)
        }
        _ => (None, false),
    };
    if video.is_none() && audio.is_none() {
        return Err(ErrorCode::NoDecoder);
    }
    // The wire takes a silent track only beside a video that plays and without audio; the decision gives no other.
    let no_sound = plan.no_sound.filter(|_| video.is_some() && audio.is_none());
    let duration_ms = (info.duration.seconds() * 1000.0).round().max(0.0) as u64;
    let system = if on_system {
        Some(SystemStreams::open(SystemOpen {
            pipe: &pipe,
            gate,
            flags,
            container: &info.container,
            video: plan.video.filter(|_| video.is_some()),
            audio: plan.audio.filter(|_| audio.is_some()),
        })?)
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
    let playing = Playing {
        source,
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
    Ok((playing, opened))
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

/// The answer for a container whose H.264 video FilmCraft refused while it opened (MPEG-TS/PS: interlaced, 10-bit).
/// The video decides first, so the audio (unknown here) does not change it: a refused H.264 track needs the system's
/// decoder (row 19), and without it here the answer names H.264. Where the system's decoder runs, Windows' reader
/// over a file FilmCraft could not describe is not wired: the same honest `NoSystemDecoder(H264)`.
fn refused_at_open(table: &[(Codec, Route)], system_on: bool) -> ErrorCode {
    let video = Some(Track {
        refused: true,
        ..Track::of(table, Codec::H264)
    });
    let state = state_for(video, None, system_on, || system::availability(Codec::H264));
    match decide(video, None, state) {
        Decision::Error(code) => code,
        Decision::Ours(_) | Decision::System(_) => ErrorCode::NoSystemDecoder(Codec::H264),
    }
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
            _ => track.refused = matches!(e, MediaError::Unsupported(_)),
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

        // H.264 is FilmCraft's and falls back: with the switch off the refusal names it (M9's interlaced row).
        assert_eq!(refused_at_open(ROUTES, false), ErrorCode::NoSystemDecoder(Codec::H264));
        assert_eq!(
            refused_at_open(&[(Codec::H264, Route::Rust)], false),
            ErrorCode::NoSystemDecoder(Codec::H264)
        );
        // An H.264 row that is off answers as any video that is not played here.
        assert_eq!(
            refused_at_open(&[(Codec::H264, Route::Unavailable)], false),
            ErrorCode::Unsupported(Codec::H264)
        );
    }
}
