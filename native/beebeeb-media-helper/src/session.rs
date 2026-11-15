//! One playback: `Open` → the container's reader-based opener over [`crate::read::PipeReader`] → `Opened`; then
//! `WantVideo` / `WantAudio` / `Seek` answered in presentation order until the end (`Ended`) or `Close`.
//! Before a stream's decoder opens, its codec's route ([`crate::routes::check`]) decides: `Rust` decodes here,
//! `System` is `NoSystemDecoder` (no system module yet), `Unavailable` is `Unsupported`.
//!
//! Never logs, never writes anything but protocol messages, holds no key, no name and no path.

use std::io::{Read, Write};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use beebeeb_media_protocol::{
    AudioInfo, Codec, ErrorCode, FromHelper, MAX_AUDIO_FRAMES, MAX_CHANNELS, MAX_FRAME_H, MAX_FRAME_W, MAX_RATE,
    MAX_SOURCE_SIDE, Opened, ToHelper, VideoInfo,
};
use filmcraft_codecs::{CodecError, Mp4Source};
use filmcraft_media::{FrameRequest, MediaError, SharedReader, SharedSource, VideoStreamInfo};

use crate::decode::{display_size, mix_channels, scale_letterbox};
use crate::read::{PipeReader, ReadFlags, SharedWire, Wire, lock};
use crate::routes::{check, is_reduced_quality};
use crate::{ROUTES, Route};

/// A file no streaming opener takes is read whole only up to this many bytes; a larger one is `TooLarge`.
pub const MAX_WHOLE: u64 = 384 << 20;
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
}

/// Serve one app over standard input and output with the shipped route table.
pub fn run(input: impl Read + Send + 'static, output: impl Write + Send + 'static) {
    run_with(input, output, ROUTES);
}

/// Serve one app with `table` as the routes (a test hands its own). Returns when the app closes, the pipe ends or
/// a message is not one this protocol reads.
pub fn run_with(input: impl Read + Send + 'static, output: impl Write + Send + 'static, table: &[(Codec, Route)]) {
    let wire: SharedWire = Arc::new(Mutex::new(Wire::new(input, output)));
    let flags = Arc::new(ReadFlags::default());
    let mut playing: Option<Playing> = None;
    loop {
        let msg = lock(&wire).recv();
        let reply = match msg {
            Err(_) | Ok(ToHelper::Close) => return,
            Ok(ToHelper::Open { len, .. }) => {
                if playing.is_some() {
                    vec![FromHelper::Error(ErrorCode::Internal)]
                } else {
                    match open(&wire, &flags, len, table) {
                        Ok((p, opened)) => {
                            playing = Some(p);
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
        for msg in reply {
            if lock(&wire).send(&msg).is_err() {
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
    len: u64,
    table: &[(Codec, Route)],
) -> Result<(Playing, Opened), ErrorCode> {
    if len == 0 {
        return Err(ErrorCode::Corrupt);
    }
    // First open sets the pool's size; the helper opens one file.
    filmcraft_codecs::set_frame_budget(FRAME_BUDGET);
    let reader: SharedReader = Arc::new(PipeReader::new(wire.clone(), len, flags.clone()));
    let source = filmcraft_media::reader::open_reader_within(
        "",
        reader,
        &filmcraft_codecs::reader_openers(),
        &filmcraft_codecs::openers(),
        MAX_WHOLE,
    )
    .map_err(|e| map_error(&e, flags))?;
    let info = source.info().clone();

    let video = match &info.video {
        Some(v) => {
            let codec = check(table, &v.codec)?;
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
        None => None,
    };
    let (audio, reduced_quality) = match info.audio() {
        Some(a) => {
            let codec = check(table, &a.codec)?;
            let channels = a.channels.clamp(1, u32::from(MAX_CHANNELS)) as u16;
            let rate = a.sample_rate.clamp(1, MAX_RATE);
            (Some(AudioInfo { rate, channels, codec }), is_reduced_quality(&a.codec))
        }
        None => (None, false),
    };
    if video.is_none() && audio.is_none() {
        return Err(ErrorCode::NoDecoder);
    }
    let duration_ms = (info.duration.seconds() * 1000.0).round().max(0.0) as u64;
    let opened = Opened {
        duration_ms,
        video: video.as_ref().map(|(v, _)| *v),
        audio,
        reduced_quality,
    };
    let playing = Playing {
        source,
        video: video.map(|(_, stream)| stream),
        has_audio: audio.is_some(),
        duration_ms,
        video_index: 0,
        audio_rate: audio.map_or(48_000, |a| a.rate),
        audio_origin_ms: 0,
        audio_done: 0,
        flags: flags.clone(),
    };
    Ok((playing, opened))
}

fn want_video(p: &mut Playing, w: u32, h: u32) -> Result<FromHelper, ErrorCode> {
    if w == 0 || h == 0 || w > MAX_FRAME_W || h > MAX_FRAME_H {
        return Err(ErrorCode::Internal);
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
        MediaError::Unsupported(s) if s.contains("too large") => ErrorCode::TooLarge,
        MediaError::Unsupported(_) | MediaError::NoStream(_) => ErrorCode::NoDecoder,
        MediaError::Cancelled => ErrorCode::Internal,
    }
}
