//! One `System` stream through Media Foundation's source reader, answered as the protocol wants it: a frame scaled and
//! letterboxed to the asked size, PCM at the asked rate and channels, a seek. Each stream has a source reader of its
//! own over its own byte stream (both read the same app ranges), so a stream FilmCraft decodes can sit beside it.
#![allow(unsafe_code)]

use std::ptr::null_mut;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use beebeeb_media_protocol::{
    Codec, ErrorCode, FromHelper, MAX_AUDIO_FRAMES, MAX_CHANNELS, MAX_FRAME_H, MAX_FRAME_W, MAX_RATE,
};
use windows::Win32::Foundation::{E_FAIL, E_POINTER};
use windows::Win32::Media::MediaFoundation::{
    IMF2DBuffer, IMFMediaBuffer, IMFMediaSource, IMFMediaType, IMFSample, IMFSourceReader, MF_MT_AUDIO_NUM_CHANNELS,
    MF_MT_AUDIO_SAMPLES_PER_SECOND, MF_MT_DEFAULT_STRIDE, MF_MT_FRAME_SIZE, MF_MT_MAJOR_TYPE,
    MF_MT_MINIMUM_DISPLAY_APERTURE, MF_MT_PIXEL_ASPECT_RATIO, MF_MT_SUBTYPE, MF_MT_VIDEO_NOMINAL_RANGE,
    MF_MT_VIDEO_ROTATION, MF_MT_YUV_MATRIX, MF_OBJECT_TYPE, MF_PD_DURATION,
    MF_RESOLUTION_CONTENT_DOES_NOT_HAVE_TO_MATCH_EXTENSION_OR_MIME_TYPE, MF_RESOLUTION_MEDIASOURCE, MF_RESOLUTION_READ,
    MF_SOURCE_READER_ALL_STREAMS, MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING,
    MF_SOURCE_READER_FIRST_AUDIO_STREAM, MF_SOURCE_READER_FIRST_VIDEO_STREAM, MF_SOURCE_READER_MEDIASOURCE,
    MF_SOURCE_READERF_CURRENTMEDIATYPECHANGED, MF_SOURCE_READERF_ENDOFSTREAM, MF_SOURCE_READERF_ERROR,
    MFAudioFormat_Float, MFMediaType_Audio, MFMediaType_Video, MFVideoFormat_NV12, MFVideoFormat_P010,
};
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
use windows::Win32::System::Variant::{VT_I8, VT_UI8};
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
use windows::core::{GUID, IUnknown, Interface, PCWSTR};

use super::Unavailable;
use super::api::{Api, api};
use super::convert::{
    Area, CODEC_NOT_FOUND, Matrix, Planes, area_of, container_hint, failure, full_range_of, halves, hundred_ns,
    matrix_of, pts_ms, quarter_turns, rotate_rgba, yuv420_to_rgba,
};
use super::gate::Gate;
use super::pcm::{Resampler, deinterleave};
use super::stream::byte_stream;
use crate::decode::{display_size, mix_channels, scale_letterbox};
use crate::read::{PipeReader, ReadFlags};
use crate::system::refusal;

const FIRST_VIDEO: u32 = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
const FIRST_AUDIO: u32 = MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32;
const ALL_STREAMS: u32 = MF_SOURCE_READER_ALL_STREAMS.0 as u32;
const MEDIA_SOURCE: u32 = MF_SOURCE_READER_MEDIASOURCE.0 as u32;
const FLAG_ERROR: u32 = MF_SOURCE_READERF_ERROR.0 as u32;
const FLAG_END: u32 = MF_SOURCE_READERF_ENDOFSTREAM.0 as u32;
const FLAG_TYPE_CHANGED: u32 = MF_SOURCE_READERF_CURRENTMEDIATYPECHANGED.0 as u32;
/// Reads in a row that give no sample (a gap, a stream tick) before the stream counts as broken.
const EMPTY_READS: u32 = 64;
/// Frames read and dropped after a seek, at most, to reach the asked time from the key frame before it.
const SKIP_READS: u32 = 1200;

/// What failed, as the protocol says it: the app's own reads first (a failed range, the pipe gone), then Media
/// Foundation's code.
fn code_of(e: &windows::core::Error, codec: Codec, flags: &ReadFlags) -> ErrorCode {
    if flags.failed.load(Ordering::Relaxed) || flags.closed.load(Ordering::Relaxed) {
        return ErrorCode::ReadFailed;
    }
    failure(e.code().0 as u32, codec)
}

/// The started functions, or the honest refusal for `codec`.
fn started(codec: Codec) -> Result<&'static Api, ErrorCode> {
    api().map_err(|why: Unavailable| refusal(why, codec))
}

/// A source reader over the file with only `stream` selected.
fn open_reader(
    api: &'static Api,
    pipe: Arc<PipeReader>,
    gate: Arc<Gate>,
    container: &str,
    stream: u32,
) -> windows::core::Result<IMFSourceReader> {
    let bytes = byte_stream(pipe, gate, api);
    let resolver = api.resolver()?;
    let hint: Vec<u16> = container_hint(container).encode_utf16().chain([0]).collect();
    let flags = (MF_RESOLUTION_MEDIASOURCE.0
        | MF_RESOLUTION_CONTENT_DOES_NOT_HAVE_TO_MATCH_EXTENSION_OR_MIME_TYPE.0
        | MF_RESOLUTION_READ.0) as u32;
    let mut kind = MF_OBJECT_TYPE::default();
    let mut object: Option<IUnknown> = None;
    // SAFETY: a byte stream of ours, a terminated name hint that lives past the call, no properties, and places of
    // ours for the two answers.
    unsafe {
        resolver.CreateObjectFromByteStream(
            &bytes,
            PCWSTR(hint.as_ptr()),
            flags,
            None::<&IPropertyStore>,
            &mut kind,
            &mut object,
        )?
    };
    let source: IMFMediaSource = object.ok_or_else(|| windows::core::Error::from(E_POINTER))?.cast()?;
    let attributes = api.attributes(1)?;
    // SAFETY: plain calls on objects of this function's own.
    unsafe {
        attributes.SetUINT32(&MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, 1)?;
        let reader = api.reader(&source, &attributes)?;
        reader.SetStreamSelection(ALL_STREAMS, false)?;
        reader.SetStreamSelection(stream, true)?;
        Ok(reader)
    }
}

/// Moves the reader to `at_ms`. Media Foundation goes to the key frame at or before it; the caller drops what lies
/// before.
fn set_position(reader: &IMFSourceReader, at_ms: u64) -> windows::core::Result<()> {
    let mut position = PROPVARIANT::default();
    // SAFETY: a fresh, zeroed variant made a 64-bit integer (its tag and that member; it owns nothing to free); the
    // time format is the default one (all zeros: 100-nanosecond units).
    unsafe {
        let inner = &mut *position.Anonymous.Anonymous;
        inner.vt = VT_I8;
        inner.Anonymous.hVal = hundred_ns(at_ms);
        reader.SetCurrentPosition(&GUID::zeroed(), &position)
    }
}

/// How a decoded picture of the current media type lies in its buffer, and how to show it.
#[derive(Clone, Copy, Debug)]
struct Picture {
    planes: Planes,
    /// The stride of a buffer that is not two-dimensional (`MF_MT_DEFAULT_STRIDE`), when the type names one.
    stride: Option<u32>,
    area: Area,
    par: (u32, u32),
    matrix: Matrix,
    full_range: bool,
}

fn picture_of(t: &IMFMediaType, ten_bit: bool) -> windows::core::Result<Picture> {
    // SAFETY: plain attribute reads of a media type the reader handed out.
    unsafe {
        let (width, height) = halves(t.GetUINT64(&MF_MT_FRAME_SIZE)?);
        let mut blob = [0u8; 16];
        let aperture = t
            .GetBlob(&MF_MT_MINIMUM_DISPLAY_APERTURE, &mut blob, None)
            .ok()
            .map(|()| blob);
        let par = t.GetUINT64(&MF_MT_PIXEL_ASPECT_RATIO).map(halves).unwrap_or((1, 1));
        let stride = t.GetUINT32(&MF_MT_DEFAULT_STRIDE).ok().filter(|s| (*s as i32) > 0);
        let bytes_per_sample = if ten_bit { 2 } else { 1 };
        Ok(Picture {
            planes: Planes {
                width,
                height,
                // A two-dimensional buffer's contiguous copy has the minimum stride.
                stride: width.saturating_mul(bytes_per_sample),
                ten_bit,
            },
            stride,
            area: area_of(aperture, width, height),
            par,
            matrix: matrix_of(t.GetUINT32(&MF_MT_YUV_MATRIX).ok(), height),
            full_range: full_range_of(t.GetUINT32(&MF_MT_VIDEO_NOMINAL_RANGE).ok()),
        })
    }
}

/// The bytes of a sample's (single, contiguous) buffer, copied out, and the stride they were laid out with.
fn sample_bytes(sample: &IMFSample, picture: &Picture) -> windows::core::Result<(Vec<u8>, u32)> {
    // SAFETY: plain calls on a sample the reader handed out; a locked buffer is read within its reported length and
    // unlocked before it is released.
    unsafe {
        let buffer: IMFMediaBuffer = sample.ConvertToContiguousBuffer()?;
        if let Ok(two_d) = buffer.cast::<IMF2DBuffer>() {
            let mut copy = vec![0u8; two_d.GetContiguousLength()? as usize];
            two_d.ContiguousCopyTo(&mut copy)?;
            return Ok((copy, picture.planes.stride));
        }
        let mut base: *mut u8 = null_mut();
        let mut len = 0u32;
        buffer.Lock(&mut base, None, Some(&raw mut len))?;
        let copy = if base.is_null() {
            Vec::new()
        } else {
            std::slice::from_raw_parts(base, len as usize).to_vec()
        };
        let _ = buffer.Unlock();
        Ok((copy, picture.stride.unwrap_or(picture.planes.stride)))
    }
}

/// A video stream decoded by Windows.
pub struct SystemVideo {
    reader: IMFSourceReader,
    codec: Codec,
    flags: Arc<ReadFlags>,
    picture: Picture,
    /// Clockwise quarter turns that show the stored frame upright (the container's display matrix, as FilmCraft
    /// applies it).
    turns: u8,
    ended: bool,
    /// After a seek: frames before this time are read and dropped.
    skip_before_ms: Option<u64>,
}

impl SystemVideo {
    /// Opens the file's first video stream as 8-bit NV12, or P010 for a 10-bit stream. A PC without the stream's
    /// decoder answers `NoSystemDecoder(codec)` (measured: "codec not found" on every output type).
    pub fn open(
        pipe: Arc<PipeReader>,
        gate: Arc<Gate>,
        flags: Arc<ReadFlags>,
        container: &str,
        codec: Codec,
    ) -> Result<SystemVideo, ErrorCode> {
        let api = started(codec)?;
        let fail = |e: windows::core::Error| code_of(&e, codec, &flags);
        let reader = open_reader(api, pipe, gate, container, FIRST_VIDEO).map_err(fail)?;
        let mut refused: Option<windows::core::Error> = None;
        let mut chosen = None;
        for subtype in [MFVideoFormat_NV12, MFVideoFormat_P010] {
            let wanted = api.media_type().map_err(fail)?;
            // SAFETY: plain calls on a media type of ours and on the reader.
            let set = unsafe {
                wanted
                    .SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
                    .and_then(|()| wanted.SetGUID(&MF_MT_SUBTYPE, &subtype))
                    .and_then(|()| reader.SetCurrentMediaType(FIRST_VIDEO, None, &wanted))
            };
            match set {
                Ok(()) => {
                    chosen = Some(subtype == MFVideoFormat_P010);
                    break;
                }
                // "Codec not found" is the honest answer; keep it over a later, vaguer one.
                Err(e) if refused.as_ref().is_some_and(|r| r.code().0 as u32 == CODEC_NOT_FOUND) => drop(e),
                Err(e) => refused = Some(e),
            }
        }
        let Some(ten_bit) = chosen else {
            return Err(fail(refused.unwrap_or_else(|| E_FAIL.into())));
        };
        // SAFETY: the reader's current and native types of the stream it reads.
        let (current, native) = unsafe {
            (
                reader.GetCurrentMediaType(FIRST_VIDEO).map_err(fail)?,
                reader.GetNativeMediaType(FIRST_VIDEO, 0).map_err(fail)?,
            )
        };
        let picture = picture_of(&current, ten_bit).map_err(fail)?;
        // SAFETY: a plain attribute read of the stream's own type.
        let turns = quarter_turns(unsafe { native.GetUINT32(&MF_MT_VIDEO_ROTATION) }.ok());
        Ok(SystemVideo {
            reader,
            codec,
            flags,
            picture,
            turns,
            ended: false,
            skip_before_ms: None,
        })
    }

    fn fail(&self, e: windows::core::Error) -> ErrorCode {
        code_of(&e, self.codec, &self.flags)
    }

    /// The next frame, scaled to fit `w × h` with its own aspect and padded with black to exactly that size.
    pub fn frame(&mut self, w: u32, h: u32) -> Result<FromHelper, ErrorCode> {
        if w == 0 || h == 0 || w > MAX_FRAME_W || h > MAX_FRAME_H {
            return Err(ErrorCode::Internal);
        }
        if self.ended {
            return Ok(FromHelper::Ended);
        }
        let mut empty = 0;
        let mut skipped = 0;
        loop {
            let mut flags = 0u32;
            let mut time = 0i64;
            let mut sample: Option<IMFSample> = None;
            // SAFETY: places of ours for the answers.
            unsafe {
                self.reader.ReadSample(
                    FIRST_VIDEO,
                    0,
                    None,
                    Some(&raw mut flags),
                    Some(&raw mut time),
                    Some(&raw mut sample),
                )
            }
            .map_err(|e| self.fail(e))?;
            if flags & FLAG_ERROR != 0 {
                return Err(self.fail(E_FAIL.into()));
            }
            if flags & FLAG_TYPE_CHANGED != 0 {
                // SAFETY: the reader's current type of the stream it reads.
                let current = unsafe { self.reader.GetCurrentMediaType(FIRST_VIDEO) }.map_err(|e| self.fail(e))?;
                self.picture = picture_of(&current, self.picture.planes.ten_bit).map_err(|e| self.fail(e))?;
            }
            if let Some(sample) = sample {
                let at_ms = pts_ms(time);
                match self.skip_before_ms {
                    Some(target) if at_ms < target && skipped < SKIP_READS => {
                        skipped += 1;
                        continue;
                    }
                    _ => self.skip_before_ms = None,
                }
                return self.convert(&sample, at_ms, w, h);
            }
            if flags & FLAG_END != 0 {
                self.ended = true;
                return Ok(FromHelper::Ended);
            }
            empty += 1;
            if empty >= EMPTY_READS {
                return Err(ErrorCode::Corrupt);
            }
        }
    }

    fn convert(&self, sample: &IMFSample, pts_ms: u64, w: u32, h: u32) -> Result<FromHelper, ErrorCode> {
        let picture = self.picture;
        let (bytes, stride) = sample_bytes(sample, &picture).map_err(|e| self.fail(e))?;
        let planes = Planes {
            stride,
            ..picture.planes
        };
        let area = picture.area;
        let rgba =
            yuv420_to_rgba(&bytes, planes, area, picture.matrix, picture.full_range).ok_or(ErrorCode::Corrupt)?;
        let (rgba, rw, rh) = rotate_rgba(&rgba, area.w, area.h, self.turns).ok_or(ErrorCode::Corrupt)?;
        let par = if self.turns % 2 == 1 {
            (picture.par.1, picture.par.0)
        } else {
            picture.par
        };
        let shown = display_size(rw, rh, par);
        let rgba = scale_letterbox(&rgba, rw, rh, shown, w, h).ok_or(ErrorCode::Corrupt)?;
        Ok(FromHelper::Frame { pts_ms, w, h, rgba })
    }

    /// The next frame is the one at `at_ms` (or the first after it).
    pub fn seek(&mut self, at_ms: u64) {
        if set_position(&self.reader, at_ms).is_ok() {
            self.ended = false;
            self.skip_before_ms = Some(at_ms);
        }
    }
}

/// An audio stream decoded by Windows: float PCM at the stream's own rate, brought to the asked rate and channels.
pub struct SystemAudio {
    reader: IMFSourceReader,
    codec: Codec,
    flags: Arc<ReadFlags>,
    /// The decoder's own rate and channels.
    rate: u32,
    channels: u16,
    resampler: Resampler,
    /// The rate the app asks for, and where its audio clock stands: `origin_ms` plus `done` frames at that rate.
    out_rate: u32,
    origin_ms: u64,
    done: u64,
    /// Media Foundation said the stream ended (what the resampler still holds is given out first).
    source_ended: bool,
    /// After a seek: samples before this time are dropped.
    skip_before: Option<i64>,
}

impl SystemAudio {
    /// Opens the file's first audio stream, decoded to float PCM. A PC without its decoder answers
    /// `NoSystemDecoder(codec)`.
    pub fn open(
        pipe: Arc<PipeReader>,
        gate: Arc<Gate>,
        flags: Arc<ReadFlags>,
        container: &str,
        codec: Codec,
    ) -> Result<SystemAudio, ErrorCode> {
        let api = started(codec)?;
        let fail = |e: windows::core::Error| code_of(&e, codec, &flags);
        let reader = open_reader(api, pipe, gate, container, FIRST_AUDIO).map_err(fail)?;
        let wanted = api.media_type().map_err(fail)?;
        // SAFETY: plain calls on a media type of ours and on the reader.
        let current = unsafe {
            wanted
                .SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)
                .and_then(|()| wanted.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_Float))
                .and_then(|()| reader.SetCurrentMediaType(FIRST_AUDIO, None, &wanted))
                .and_then(|()| reader.GetCurrentMediaType(FIRST_AUDIO))
        }
        .map_err(fail)?;
        // SAFETY: plain attribute reads of the reader's current type.
        let (rate, channels) = unsafe {
            (
                current.GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND).map_err(fail)?,
                current.GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS).map_err(fail)?,
            )
        };
        if rate == 0 || rate > MAX_RATE || channels == 0 || channels > 64 {
            return Err(ErrorCode::Corrupt);
        }
        let channels = channels as u16;
        let mut audio = SystemAudio {
            reader,
            codec,
            flags,
            rate,
            channels,
            resampler: Resampler::new(rate, rate, channels),
            out_rate: rate,
            origin_ms: 0,
            done: 0,
            source_ended: false,
            skip_before: None,
        };
        // ux-57: Windows may load a track's decoder only when the first sample is read (a PC without the AAC decoder
        // opens, then fails there). Read one now, so a missing decoder is an answer of `open`, where a video beside
        // it can go on without sound. The sample stays in the resampler.
        audio.read_more()?;
        Ok(audio)
    }

    fn fail(&self, e: windows::core::Error) -> ErrorCode {
        code_of(&e, self.codec, &self.flags)
    }

    /// The decoder's own rate and channels (what `Opened` says when Windows describes the file: row 14 of MCP2).
    pub fn format(&self) -> (u32, u16) {
        (self.rate, self.channels)
    }

    /// The file's duration as Windows' reader gives it (`MF_PD_DURATION`), when it knows it.
    pub fn duration_ms(&self) -> Option<u64> {
        // SAFETY: a plain attribute read of the reader's media source. The variant is read as a 64-bit unsigned
        // integer only when its tag says it is one, and that kind owns nothing to free.
        unsafe {
            let value = self
                .reader
                .GetPresentationAttribute(MEDIA_SOURCE, &MF_PD_DURATION)
                .ok()?;
            let inner = &*value.Anonymous.Anonymous;
            (inner.vt == VT_UI8).then(|| pts_ms(i64::try_from(inner.Anonymous.uhVal).unwrap_or(i64::MAX)))
        }
    }

    /// One more decoded sample into the resampler, or the end of the stream.
    fn read_more(&mut self) -> Result<(), ErrorCode> {
        for _ in 0..EMPTY_READS {
            let mut flags = 0u32;
            let mut time = 0i64;
            let mut sample: Option<IMFSample> = None;
            // SAFETY: places of ours for the answers.
            unsafe {
                self.reader.ReadSample(
                    FIRST_AUDIO,
                    0,
                    None,
                    Some(&raw mut flags),
                    Some(&raw mut time),
                    Some(&raw mut sample),
                )
            }
            .map_err(|e| self.fail(e))?;
            if flags & FLAG_ERROR != 0 {
                return Err(self.fail(E_FAIL.into()));
            }
            if let Some(sample) = sample {
                let pcm = self.floats(&sample)?;
                let channels = usize::from(self.channels);
                // After a seek, drop the part before the asked time.
                let mut from = 0;
                if let Some(target) = self.skip_before {
                    if time < target {
                        let late = (target - time) as u128 * u128::from(self.rate) / 10_000_000;
                        from = (late as usize).min(pcm.len() / channels);
                    }
                    if from < pcm.len() / channels {
                        self.skip_before = None;
                    }
                }
                self.resampler.push(&pcm[from * channels..]);
                return Ok(());
            }
            if flags & FLAG_END != 0 {
                self.source_ended = true;
                return Ok(());
            }
        }
        Err(ErrorCode::Corrupt)
    }

    /// A sample's float PCM, copied out (a sample that is not finite is silence).
    fn floats(&self, sample: &IMFSample) -> Result<Vec<f32>, ErrorCode> {
        // SAFETY: a locked buffer is read within its reported length and unlocked before it is released.
        unsafe {
            let buffer = sample.ConvertToContiguousBuffer().map_err(|e| self.fail(e))?;
            let mut base: *mut u8 = null_mut();
            let mut len = 0u32;
            buffer
                .Lock(&mut base, None, Some(&raw mut len))
                .map_err(|e| self.fail(e))?;
            let pcm = if base.is_null() {
                Vec::new()
            } else {
                std::slice::from_raw_parts(base, len as usize)
                    .chunks_exact(4)
                    .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                    .map(|v| if v.is_finite() { v } else { 0.0 })
                    .collect()
            };
            let _ = buffer.Unlock();
            Ok(pcm)
        }
    }

    /// Up to `frames` frames of interleaved PCM at `rate` with `channels` channels.
    pub fn audio(&mut self, rate: u32, channels: u16, frames: u32) -> Result<FromHelper, ErrorCode> {
        if rate == 0 || rate > MAX_RATE || channels == 0 || channels > MAX_CHANNELS || frames == 0 {
            return Err(ErrorCode::Internal);
        }
        if rate != self.out_rate {
            // The app asks at another rate: carry the position over in milliseconds.
            self.origin_ms += self.done * 1000 / u64::from(self.out_rate);
            self.done = 0;
            self.out_rate = rate;
            self.resampler = Resampler::new(self.rate, rate, self.channels);
        }
        let asked = frames.min(MAX_AUDIO_FRAMES) as usize;
        while self.resampler.ready() < asked && !self.source_ended {
            self.read_more()?;
        }
        let pcm = self.resampler.take(asked, self.source_ended);
        if pcm.is_empty() {
            return Ok(FromHelper::Ended);
        }
        let samples = mix_channels(&deinterleave(&pcm, self.channels), channels, asked);
        let pts_ms = self.origin_ms + self.done * 1000 / u64::from(self.out_rate);
        self.done += (samples.len() / usize::from(channels)) as u64;
        Ok(FromHelper::Audio {
            pts_ms,
            channels,
            samples,
        })
    }

    /// The next PCM starts at `at_ms`.
    pub fn seek(&mut self, at_ms: u64) {
        if set_position(&self.reader, at_ms).is_ok() {
            self.resampler = Resampler::new(self.rate, self.out_rate, self.channels);
            self.origin_ms = at_ms;
            self.done = 0;
            self.source_ended = false;
            self.skip_before = Some(hundred_ns(at_ms));
        }
    }
}
