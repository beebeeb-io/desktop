//! The messages between the desktop app and its sandboxed media helper (`native/beebeeb-media-helper`), and their
//! checked encoding on the helper's two pipes.
//!
//! The boundary (the media plan, (d)): the app holds the keys, the audio device, the clock, the OS media session and
//! the file's name; the helper holds none of them. Into the helper go the decrypted bytes of the ONE file it was
//! started for, only the ranges it asked for ([`FromHelper::ReadAt`] → [`ToHelper::Data`]), and what to make of them
//! (a frame of the drawn size, PCM at the device's rate). Out of it come RGBA frames of exactly the asked size, f32
//! PCM, two short tags and a small picture, and errors as codes from a fixed list. Never a name, a path or a URL.
//!
//! **The helper is untrusted.** [`read_to_app`] checks every length against this crate's limits BEFORE it allocates
//! anything for it (a message's length against its kind's maximum, then a frame's `w` and `h`, then that the rest is
//! exactly `w * h * 4` bytes, and so on); anything else is a [`WireError`], and the app kills the helper. What the
//! limits cannot know (that a frame is the size the app ASKED for, that a read is inside the file and within the
//! reads in flight, that PCM is finite) the app's player checks (`native/…/media/player.rs`).
//!
//! The wire: a 4-byte little-endian length (of what follows), a tag byte, then fixed little-endian fields; the bulk
//! of a message (a range's bytes, a frame's pixels, the picture's pixels) is its last field and is written straight
//! from the caller's buffer, never copied into one of this crate's (the app's decrypted bytes stay in its own
//! `Zeroizing` buffer). The two directions use disjoint tags, so a message sent the wrong way is refused.

#![forbid(unsafe_code)]

use std::fmt;
use std::io::{self, Read, Write};

#[cfg(test)]
mod tests;

// ---- Limits ----

/// The largest control message (everything that is not bulk), in bytes.
pub const MAX_CONTROL: usize = 64 * 1024;
/// The largest range one [`FromHelper::ReadAt`] may ask for, and one [`ToHelper::Data`] may carry.
pub const MAX_READ: u32 = 4 << 20;
/// The largest frame the app asks for and accepts: 1920 × 1080 (a 4K file is scaled in the helper).
pub const MAX_FRAME_W: u32 = 1920;
pub const MAX_FRAME_H: u32 = 1080;
/// The most audio frames (samples per channel) one [`ToHelper::WantAudio`] asks for: one second at 48 kHz.
pub const MAX_AUDIO_FRAMES: u32 = 48_000;
/// The longest title or artist, in characters.
pub const MAX_TEXT: usize = 256;
/// The largest side of the artwork, in pixels.
pub const MAX_ART: u32 = 1024;
/// How many [`FromHelper::ReadAt`] may be unanswered at once (the app's player counts them).
pub const MAX_IN_FLIGHT_READS: usize = 2;
/// The most channels of PCM the app asks for (7.1).
pub const MAX_CHANNELS: u16 = 8;
/// The highest output rate the app asks for, in Hz.
pub const MAX_RATE: u32 = 192_000;
/// The largest side a file's own video may say it has ([`VideoInfo`]; a 4K file says 3840 × 2160). Only the
/// frames the app asks for are held to [`MAX_FRAME_W`] × [`MAX_FRAME_H`].
pub const MAX_SOURCE_SIDE: u32 = 16_384;
/// [`MAX_TEXT`] characters of UTF-8 are at most this many bytes.
pub const MAX_TEXT_BYTES: usize = MAX_TEXT * 4;

const FRAME_BYTES_MAX: usize = MAX_FRAME_W as usize * MAX_FRAME_H as usize * 4;
const AUDIO_BYTES_MAX: usize = MAX_AUDIO_FRAMES as usize * MAX_CHANNELS as usize * 4;
const ART_BYTES_MAX: usize = MAX_ART as usize * MAX_ART as usize * 4;
/// `Opened`: duration, the video's flag and four fields, the audio's flag and three, the reduced-quality flag, the
/// no-sound flag and its codec.
const OPENED_BODY: usize = 8 + (1 + 4 + 4 + 4 + 1) + (1 + 4 + 2 + 1) + 1 + (1 + 1);
/// `Tags` without its picture: the two texts with their lengths, the picture's flag and size.
const TAGS_HEAD: usize = 2 * (2 + MAX_TEXT_BYTES) + 1 + 4 + 4;
const _: () = assert!(OPENED_BODY <= MAX_CONTROL && TAGS_HEAD <= MAX_CONTROL);

// ---- Messages ----

/// What kind of file the app opens, as its name says (the helper may find otherwise).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum KindHint {
    Audio,
    Video,
}

/// A stream's codec. The helper's route table (`beebeeb_media_helper::ROUTES`) says how each is decoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Codec {
    H264 = 0,
    Hevc = 1,
    Vp8 = 2,
    Vp9 = 3,
    Av1 = 4,
    /// MPEG-1 and MPEG-2 video.
    Mpeg2 = 5,
    ProRes = 6,
    Dnx = 7,
    Mp3 = 8,
    Aac = 9,
    Flac = 10,
    Alac = 11,
    Opus = 12,
    Vorbis = 13,
    Pcm = 14,
    Ac3 = 15,
    Eac3 = 16,
    /// MPEG-1 / MPEG-2 audio, layers I and II (MP3 is [`Codec::Mp3`]).
    Mp2 = 17,
    /// MPEG-4 Part 2 video (the DivX / Xvid kind).
    Mpeg4Part2 = 18,
    /// VC-1, and the Windows Media Video it is.
    Vc1 = 19,
    /// Windows Media Audio.
    Wma = 20,
    /// A codec this list does not name.
    Other = 21,
}

impl Codec {
    /// Every codec, in the order of their wire bytes.
    pub const ALL: [Codec; 22] = [
        Codec::H264,
        Codec::Hevc,
        Codec::Vp8,
        Codec::Vp9,
        Codec::Av1,
        Codec::Mpeg2,
        Codec::ProRes,
        Codec::Dnx,
        Codec::Mp3,
        Codec::Aac,
        Codec::Flac,
        Codec::Alac,
        Codec::Opus,
        Codec::Vorbis,
        Codec::Pcm,
        Codec::Ac3,
        Codec::Eac3,
        Codec::Mp2,
        Codec::Mpeg4Part2,
        Codec::Vc1,
        Codec::Wma,
        Codec::Other,
    ];

    /// The format's name as people know it ("HEVC", "AC-3"), for the words that say it cannot be played here.
    /// `Other` has no name: the app says "this kind of file" instead.
    pub fn name(self) -> Option<&'static str> {
        Some(match self {
            Codec::H264 => "H.264",
            Codec::Hevc => "HEVC",
            Codec::Vp8 => "VP8",
            Codec::Vp9 => "VP9",
            Codec::Av1 => "AV1",
            Codec::Mpeg2 => "MPEG-2",
            Codec::ProRes => "ProRes",
            Codec::Dnx => "DNxHD",
            Codec::Mp3 => "MP3",
            Codec::Aac => "AAC",
            Codec::Flac => "FLAC",
            Codec::Alac => "ALAC",
            Codec::Opus => "Opus",
            Codec::Vorbis => "Vorbis",
            Codec::Pcm => "PCM",
            Codec::Ac3 => "AC-3",
            Codec::Eac3 => "E-AC-3",
            Codec::Mp2 => "MP2",
            Codec::Mpeg4Part2 => "MPEG-4 Part 2",
            Codec::Vc1 => "VC-1",
            Codec::Wma => "WMA",
            Codec::Other => return None,
        })
    }

    fn byte(self) -> u8 {
        self as u8
    }

    fn of(byte: u8) -> Result<Codec, WireError> {
        Codec::ALL.get(usize::from(byte)).copied().ok_or(WireError::Malformed)
    }
}

/// Why the helper plays nothing. A code from this list, never text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorCode {
    /// The codec's route is `Unavailable` (decision D-MED-2: a row that is off, or a codec with no decoder here).
    Unsupported(Codec),
    /// The codec's route is the system's decoder and this PC has none for it (HEVC on a Windows PC without
    /// Microsoft's HEVC extension). The helper never decodes it with its own decoder instead.
    NoSystemDecoder(Codec),
    /// The codec's route is the system's decoder and this Windows has no media codecs at all (an N or KN edition
    /// without the Media Feature Pack). The same honest state as [`ErrorCode::NoSystemDecoder`], with words that say
    /// why; never the helper's own decoder instead.
    NoSystemMedia(Codec),
    /// No decoder for what the file holds (a format the decoders do not read).
    NoDecoder,
    /// The file is damaged or is not what its name says.
    Corrupt,
    /// A format read whole that is larger than the helper reads whole.
    TooLarge,
    /// The app answered a read with `DataFailed`.
    ReadFailed,
    /// Decoding took longer than the helper allows itself.
    Timeout,
    /// Anything else inside the helper.
    Internal,
    /// A 10-bit H.264 stream that no decoder on this PC takes (ux-24): said as itself, not as "no decoder".
    TenBit,
    /// The file's video is a codec this build does not recognise at all, in any container (an unknown codec in a
    /// Matroska file, ux-24): said as itself, not as "no decoder".
    UnknownCodec,
}

impl ErrorCode {
    /// The code byte and the codec byte (0 when the code names no codec).
    fn bytes(self) -> [u8; 2] {
        match self {
            ErrorCode::Unsupported(codec) => [0, codec.byte()],
            ErrorCode::NoSystemDecoder(codec) => [1, codec.byte()],
            ErrorCode::NoDecoder => [2, 0],
            ErrorCode::Corrupt => [3, 0],
            ErrorCode::TooLarge => [4, 0],
            ErrorCode::ReadFailed => [5, 0],
            ErrorCode::Timeout => [6, 0],
            ErrorCode::Internal => [7, 0],
            ErrorCode::NoSystemMedia(codec) => [8, codec.byte()],
            ErrorCode::TenBit => [9, 0],
            ErrorCode::UnknownCodec => [10, 0],
        }
    }

    fn of([code, codec]: [u8; 2]) -> Result<ErrorCode, WireError> {
        let plain = |code| {
            if codec == 0 {
                Ok(code)
            } else {
                Err(WireError::Malformed)
            }
        };
        match code {
            0 => Ok(ErrorCode::Unsupported(Codec::of(codec)?)),
            1 => Ok(ErrorCode::NoSystemDecoder(Codec::of(codec)?)),
            2 => plain(ErrorCode::NoDecoder),
            3 => plain(ErrorCode::Corrupt),
            4 => plain(ErrorCode::TooLarge),
            5 => plain(ErrorCode::ReadFailed),
            6 => plain(ErrorCode::Timeout),
            7 => plain(ErrorCode::Internal),
            8 => Ok(ErrorCode::NoSystemMedia(Codec::of(codec)?)),
            9 => plain(ErrorCode::TenBit),
            10 => plain(ErrorCode::UnknownCodec),
            _ => Err(WireError::Malformed),
        }
    }
}

// ---- Routes and the sandbox profile they need ----

/// How the helper decodes a codec: three kinds (decision D-MED-2, the owner's ruling of 2026-11-15 17:30). The
/// helper's table (`beebeeb_media_helper::ROUTES`) gives each codec one; the type lives here so the app, which links
/// this crate and never the helper's, can name it too.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route {
    /// The helper's own Rust decoder, in the helper.
    Rust,
    /// The operating system's decoder, inside the (confined) helper: Media Foundation on Windows, Apple's on macOS.
    /// A PC without one answers [`ErrorCode::NoSystemDecoder`] or [`ErrorCode::NoSystemMedia`]. NEVER the
    /// helper's own decoder in its place: not as a fallback, not silently.
    System,
    /// Not played here: [`ErrorCode::Unsupported`], and the app offers "Open in another app…".
    Unavailable,
}

/// Whether a stream whose route is [`Route::System`] is played at all. It needs the helper to start in the
/// sandbox's system-decoder profile, which may read the registry (decision 2541). ON (the owner's ruling of
/// 2026-11-20, ux-02: with it off no HEVC clip played at all, nor an interlaced or 10-bit H.264 one, and an H.264
/// clip with AAC sound played silent, on a PC that has Windows' decoders for all of them). The PC's own decoder
/// plays such a stream when the PC has one; a PC without one answers [`ErrorCode::NoSystemDecoder`] from the
/// system-decoder profile, and the app says so with "Open in default app" one press away. With the switch off,
/// [`profile_for`] answers `None`, the app keeps the default profile and no such stream plays.
pub const MEDIA_SYSTEM_DECODER_ON: bool = true;

/// What the sandbox's system-decoder profile must allow for one file. The app makes it
/// `beebeeb_office_sandbox::Profile::SystemDecoder { hevc }` (this crate has no dependency, so it cannot name it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SystemDecoderProfile {
    /// The file is HEVC: the profile adds the four capabilities of Microsoft's Store decoder.
    pub hevc: bool,
}

/// The profile to restart the helper in, for a stream of `codec` whose route is `route`; `None` keeps the default
/// profile. The app learns both from the helper's first answer: `ErrorCode::NoSystemDecoder(codec)` from a helper in
/// the default profile is a `System` stream (the helper checks the video stream first, so a file with HEVC video
/// names `Hevc`). Restart once; a helper already in this profile that answers the same is the PC's honest answer.
pub fn profile_for(route: Route, codec: Codec) -> Option<SystemDecoderProfile> {
    profile_for_with(MEDIA_SYSTEM_DECODER_ON, route, codec)
}

/// [`profile_for`] with the switch given: the rule itself, so a test holds it at both values.
pub fn profile_for_with(on: bool, route: Route, codec: Codec) -> Option<SystemDecoderProfile> {
    match route {
        Route::System if on => Some(SystemDecoderProfile {
            hevc: codec == Codec::Hevc,
        }),
        Route::System | Route::Rust | Route::Unavailable => None,
    }
}

/// From the app to the helper.
#[derive(Clone, PartialEq, Eq)]
pub enum ToHelper {
    /// The one file this helper is for: its length in bytes, and its kind by its name.
    Open { len: u64, hint: KindHint },
    /// The answer to `ReadAt { id, .. }`: exactly the bytes asked for.
    Data { id: u32, bytes: Vec<u8> },
    /// The answer to `ReadAt { id, .. }` when the app could not read them.
    DataFailed { id: u32 },
    /// The next frame in presentation order, scaled to exactly `w × h` (the drawn size in device pixels).
    WantVideo { w: u32, h: u32 },
    /// The next `frames` frames of interleaved f32 PCM at `rate`, with `channels` channels.
    WantAudio { rate: u32, channels: u16, frames: u32 },
    /// The next answers start at `at_ms`.
    Seek { at_ms: u64 },
    /// Stop; the helper exits.
    Close,
    /// Sent before `Open` to the helper the app started in the system decoder's profile: no other helper follows
    /// this one. Where the system's decoders do not start in it either, that is this PC's answer: a video this
    /// crate's own decoder plays is played without the audio track that needed them (`Opened::no_sound`), and
    /// nothing else changes (ux-57).
    LastHelper,
}

/// Counts and kinds, never a byte of the file.
impl fmt::Debug for ToHelper {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ToHelper::Open { len, hint } => write!(f, "Open({len} bytes, {hint:?})"),
            ToHelper::Data { id, bytes } => write!(f, "Data(#{id}, {} bytes)", bytes.len()),
            ToHelper::DataFailed { id } => write!(f, "DataFailed(#{id})"),
            ToHelper::WantVideo { w, h } => write!(f, "WantVideo({w}x{h})"),
            ToHelper::WantAudio { rate, channels, frames } => {
                write!(f, "WantAudio({frames} at {rate} Hz, {channels} ch)")
            }
            ToHelper::Seek { at_ms } => write!(f, "Seek({at_ms} ms)"),
            ToHelper::Close => f.write_str("Close"),
            ToHelper::LastHelper => f.write_str("LastHelper"),
        }
    }
}

/// From the helper to the app. Untrusted: see the module doc.
#[derive(Clone, PartialEq)]
pub enum FromHelper {
    /// Bytes `offset .. offset + len` of the file, please; answered by `Data { id }` or `DataFailed { id }`.
    ReadAt { id: u32, offset: u64, len: u32 },
    /// The file opened: what it holds.
    Opened(Opened),
    /// A frame, `w × h` RGBA (exactly `w * h * 4` bytes), shown at `pts_ms`.
    Frame { pts_ms: u64, w: u32, h: u32, rgba: Vec<u8> },
    /// Interleaved f32 PCM, `samples.len() / channels` frames, played from `pts_ms`.
    Audio {
        pts_ms: u64,
        channels: u16,
        samples: Vec<f32>,
    },
    /// The file's title, artist and picture.
    Tags(Tags),
    /// The end of the file.
    Ended,
    /// Nothing more will play.
    Error(ErrorCode),
}

/// Counts and kinds, never a pixel, a sample or a word of a tag.
impl fmt::Debug for FromHelper {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FromHelper::ReadAt { id, offset, len } => write!(f, "ReadAt(#{id}, {len} bytes at {offset})"),
            FromHelper::Opened(opened) => write!(f, "{opened:?}"),
            FromHelper::Frame { pts_ms, w, h, .. } => write!(f, "Frame({w}x{h} at {pts_ms} ms)"),
            FromHelper::Audio {
                pts_ms,
                channels,
                samples,
            } => {
                write!(f, "Audio({} samples, {channels} ch, at {pts_ms} ms)", samples.len())
            }
            FromHelper::Tags(tags) => write!(f, "{tags:?}"),
            FromHelper::Ended => f.write_str("Ended"),
            FromHelper::Error(code) => write!(f, "Error({code:?})"),
        }
    }
}

/// What an opened file holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Opened {
    pub duration_ms: u64,
    pub video: Option<VideoInfo>,
    pub audio: Option<AudioInfo>,
    /// Played below the file's own quality on this PC (HE-AAC at its AAC-LC core: decision D-MED-7).
    pub reduced_quality: bool,
    /// The file's audio track that does not play here (its codec): the video plays without sound. Only with a video
    /// and without `audio`; any other shape is refused on the wire.
    pub no_sound: Option<Codec>,
}

/// The file's own video: its size (not the size the app asks for), its rate in frames per 1000 s, its codec.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VideoInfo {
    pub w: u32,
    pub h: u32,
    pub fps_milli: u32,
    pub codec: Codec,
}

/// The file's own audio.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioInfo {
    pub rate: u32,
    pub channels: u16,
    pub codec: Codec,
}

/// The file's tags: at most [`MAX_TEXT`] characters each, and the first picture as RGBA, at most [`MAX_ART`] a side.
#[derive(Clone, PartialEq, Eq)]
pub struct Tags {
    pub title: String,
    pub artist: String,
    pub art: Option<(u32, u32, Vec<u8>)>,
}

/// Lengths, never a word.
impl fmt::Debug for Tags {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let art = self.art.as_ref().map(|(w, h, _)| (*w, *h));
        write!(
            f,
            "Tags(title {} chars, artist {} chars, art {art:?})",
            self.title.chars().count(),
            self.artist.chars().count()
        )
    }
}

/// `text` cut to [`MAX_TEXT`] characters (on a character boundary): what the helper sends as a tag.
pub fn cut_text(text: &str) -> String {
    text.chars().take(MAX_TEXT).collect()
}

// ---- Errors ----

/// Why a message was not read (or not written).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WireError {
    /// The pipe ended between two messages.
    Closed,
    /// The pipe ended inside a message.
    Truncated,
    /// A length over this crate's limits (a message's, a frame's, a text's): nothing was allocated for it.
    TooLarge,
    /// A tag this direction does not have.
    UnknownTag(u8),
    /// A field out of its range, a length that does not match the fields, text that is not UTF-8.
    Malformed,
    /// The pipe failed.
    Io(io::ErrorKind),
}

impl fmt::Display for WireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WireError::Closed => f.write_str("the pipe closed"),
            WireError::Truncated => f.write_str("the pipe ended inside a message"),
            WireError::TooLarge => f.write_str("a length over the protocol's limits"),
            WireError::UnknownTag(tag) => write!(f, "unknown message tag {tag}"),
            WireError::Malformed => f.write_str("a malformed message"),
            WireError::Io(kind) => write!(f, "the pipe failed: {kind}"),
        }
    }
}

impl std::error::Error for WireError {}

// ---- The checks both sides share (the writer refuses what the reader would) ----

fn check_read_len(len: u32) -> Result<(), WireError> {
    match len {
        0 => Err(WireError::Malformed),
        len if len > MAX_READ => Err(WireError::TooLarge),
        _ => Ok(()),
    }
}

fn check_size(w: u32, h: u32, max_w: u32, max_h: u32) -> Result<(), WireError> {
    if w == 0 || h == 0 {
        Err(WireError::Malformed)
    } else if w > max_w || h > max_h {
        Err(WireError::TooLarge)
    } else {
        Ok(())
    }
}

fn check_frame(w: u32, h: u32) -> Result<(), WireError> {
    check_size(w, h, MAX_FRAME_W, MAX_FRAME_H)
}

fn check_art(w: u32, h: u32) -> Result<(), WireError> {
    check_size(w, h, MAX_ART, MAX_ART)
}

/// `w * h * 4` of a size that passed [`check_size`] (it fits a `usize`).
fn pixel_bytes(w: u32, h: u32) -> usize {
    w as usize * h as usize * 4
}

fn check_channels(channels: u16) -> Result<(), WireError> {
    match channels {
        0 => Err(WireError::Malformed),
        c if c > MAX_CHANNELS => Err(WireError::TooLarge),
        _ => Ok(()),
    }
}

fn check_want_audio(rate: u32, channels: u16, frames: u32) -> Result<(), WireError> {
    check_channels(channels)?;
    match (rate, frames) {
        (0, _) | (_, 0) => Err(WireError::Malformed),
        (rate, frames) if rate > MAX_RATE || frames > MAX_AUDIO_FRAMES => Err(WireError::TooLarge),
        _ => Ok(()),
    }
}

/// `samples` interleaved samples of `channels` channels: whole frames, at most [`MAX_AUDIO_FRAMES`] of them.
fn check_audio(channels: u16, samples: usize) -> Result<(), WireError> {
    check_channels(channels)?;
    let channels = usize::from(channels);
    if samples % channels != 0 {
        Err(WireError::Malformed)
    } else if samples / channels > MAX_AUDIO_FRAMES as usize {
        Err(WireError::TooLarge)
    } else {
        Ok(())
    }
}

fn check_text(text: &str) -> Result<(), WireError> {
    if text.len() > MAX_TEXT_BYTES || text.chars().count() > MAX_TEXT {
        Err(WireError::TooLarge)
    } else {
        Ok(())
    }
}

fn check_opened(opened: &Opened) -> Result<(), WireError> {
    if let Some(video) = opened.video {
        check_size(video.w, video.h, MAX_SOURCE_SIDE, MAX_SOURCE_SIDE)?;
    }
    if let Some(audio) = opened.audio
        && (audio.rate == 0 || audio.channels == 0)
    {
        return Err(WireError::Malformed);
    }
    // A track without sound is named only beside a video that plays, and never beside audio that plays.
    if opened.no_sound.is_some() && (opened.audio.is_some() || opened.video.is_none()) {
        return Err(WireError::Malformed);
    }
    Ok(())
}

// ---- Writing ----

const T_OPEN: u8 = 0x01;
const T_DATA: u8 = 0x02;
const T_DATA_FAILED: u8 = 0x03;
const T_WANT_VIDEO: u8 = 0x04;
const T_WANT_AUDIO: u8 = 0x05;
const T_SEEK: u8 = 0x06;
const T_CLOSE: u8 = 0x07;
const T_LAST_HELPER: u8 = 0x08;

const T_READ_AT: u8 = 0x81;
const T_OPENED: u8 = 0x82;
const T_FRAME: u8 = 0x83;
const T_AUDIO: u8 = 0x84;
const T_TAGS: u8 = 0x85;
const T_ENDED: u8 = 0x86;
const T_ERROR: u8 = 0x87;

const NO_BULK: &[u8] = &[];

mod sealed {
    pub trait Sealed {}
    impl Sealed for super::ToHelper {}
    impl Sealed for super::FromHelper {}
}

/// A message of either direction ([`ToHelper`], [`FromHelper`]).
pub trait Message: sealed::Sealed {
    /// Appends the tag and the fixed fields to `head` and answers the bulk bytes that follow them, after checking the
    /// message against the limits its reader holds it to.
    #[doc(hidden)]
    fn encode<'a>(&'a self, head: &mut Vec<u8>) -> Result<&'a [u8], WireError>;
}

fn put_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn put_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn put_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn put_text(out: &mut Vec<u8>, text: &str) -> Result<(), WireError> {
    check_text(text)?;
    // `check_text` bounds it to MAX_TEXT_BYTES, which fits a u16.
    put_u16(out, text.len() as u16);
    out.extend_from_slice(text.as_bytes());
    Ok(())
}

impl Message for ToHelper {
    fn encode<'a>(&'a self, head: &mut Vec<u8>) -> Result<&'a [u8], WireError> {
        match self {
            ToHelper::Open { len, hint } => {
                head.push(T_OPEN);
                put_u64(head, *len);
                head.push(match hint {
                    KindHint::Audio => 0,
                    KindHint::Video => 1,
                });
            }
            ToHelper::Data { id, bytes } => {
                if bytes.len() > MAX_READ as usize {
                    return Err(WireError::TooLarge);
                }
                head.push(T_DATA);
                put_u32(head, *id);
                put_u32(head, bytes.len() as u32);
                return Ok(bytes.as_slice());
            }
            ToHelper::DataFailed { id } => {
                head.push(T_DATA_FAILED);
                put_u32(head, *id);
            }
            ToHelper::WantVideo { w, h } => {
                check_frame(*w, *h)?;
                head.push(T_WANT_VIDEO);
                put_u32(head, *w);
                put_u32(head, *h);
            }
            ToHelper::WantAudio { rate, channels, frames } => {
                check_want_audio(*rate, *channels, *frames)?;
                head.push(T_WANT_AUDIO);
                put_u32(head, *rate);
                put_u16(head, *channels);
                put_u32(head, *frames);
            }
            ToHelper::Seek { at_ms } => {
                head.push(T_SEEK);
                put_u64(head, *at_ms);
            }
            ToHelper::Close => head.push(T_CLOSE),
            ToHelper::LastHelper => head.push(T_LAST_HELPER),
        }
        Ok(NO_BULK)
    }
}

impl Message for FromHelper {
    fn encode<'a>(&'a self, head: &mut Vec<u8>) -> Result<&'a [u8], WireError> {
        match self {
            FromHelper::ReadAt { id, offset, len } => {
                check_read_len(*len)?;
                head.push(T_READ_AT);
                put_u32(head, *id);
                put_u64(head, *offset);
                put_u32(head, *len);
            }
            FromHelper::Opened(opened) => {
                check_opened(opened)?;
                head.push(T_OPENED);
                put_u64(head, opened.duration_ms);
                match opened.video {
                    None => head.push(0),
                    Some(video) => {
                        head.push(1);
                        put_u32(head, video.w);
                        put_u32(head, video.h);
                        put_u32(head, video.fps_milli);
                        head.push(video.codec.byte());
                    }
                }
                match opened.audio {
                    None => head.push(0),
                    Some(audio) => {
                        head.push(1);
                        put_u32(head, audio.rate);
                        put_u16(head, audio.channels);
                        head.push(audio.codec.byte());
                    }
                }
                head.push(u8::from(opened.reduced_quality));
                match opened.no_sound {
                    None => head.push(0),
                    Some(codec) => {
                        head.push(1);
                        head.push(codec.byte());
                    }
                }
            }
            FromHelper::Frame { pts_ms, w, h, rgba } => {
                check_frame(*w, *h)?;
                if rgba.len() != pixel_bytes(*w, *h) {
                    return Err(WireError::Malformed);
                }
                head.push(T_FRAME);
                put_u64(head, *pts_ms);
                put_u32(head, *w);
                put_u32(head, *h);
                return Ok(rgba.as_slice());
            }
            FromHelper::Audio {
                pts_ms,
                channels,
                samples,
            } => {
                check_audio(*channels, samples.len())?;
                head.push(T_AUDIO);
                put_u64(head, *pts_ms);
                put_u16(head, *channels);
                // `check_audio` bounds it to MAX_AUDIO_FRAMES * MAX_CHANNELS, which fits a u32.
                put_u32(head, samples.len() as u32);
                for sample in samples {
                    head.extend_from_slice(&sample.to_le_bytes());
                }
            }
            FromHelper::Tags(tags) => {
                head.push(T_TAGS);
                put_text(head, &tags.title)?;
                put_text(head, &tags.artist)?;
                match &tags.art {
                    None => head.push(0),
                    Some((w, h, rgba)) => {
                        check_art(*w, *h)?;
                        if rgba.len() != pixel_bytes(*w, *h) {
                            return Err(WireError::Malformed);
                        }
                        head.push(1);
                        put_u32(head, *w);
                        put_u32(head, *h);
                        return Ok(rgba.as_slice());
                    }
                }
            }
            FromHelper::Ended => head.push(T_ENDED),
            FromHelper::Error(code) => {
                head.push(T_ERROR);
                head.extend_from_slice(&code.bytes());
            }
        }
        Ok(NO_BULK)
    }
}

/// Writes `msg` and flushes. A message this crate's reader would refuse is not written: `InvalidInput`.
pub fn write_msg<W: Write + ?Sized, M: Message>(w: &mut W, msg: &M) -> io::Result<()> {
    let refused = |e: WireError| io::Error::new(io::ErrorKind::InvalidInput, e);
    let mut head = Vec::with_capacity(64);
    head.extend_from_slice(&[0; 4]);
    let bulk = msg.encode(&mut head).map_err(refused)?;
    let len = u32::try_from(head.len() - 4 + bulk.len()).map_err(|_| refused(WireError::TooLarge))?;
    head[..4].copy_from_slice(&len.to_le_bytes());
    w.write_all(&head)?;
    w.write_all(bulk)?;
    w.flush()
}

// ---- Reading ----

/// The most a message of kind `tag` from the app holds after its tag. `None`: no such kind.
fn to_helper_max(tag: u8) -> Option<usize> {
    Some(match tag {
        T_OPEN => 8 + 1,
        T_DATA => 4 + 4 + MAX_READ as usize,
        T_DATA_FAILED => 4,
        T_WANT_VIDEO => 4 + 4,
        T_WANT_AUDIO => 4 + 2 + 4,
        T_SEEK => 8,
        T_CLOSE | T_LAST_HELPER => 0,
        _ => return None,
    })
}

/// The most a message of kind `tag` from the helper holds after its tag. `None`: no such kind.
fn from_helper_max(tag: u8) -> Option<usize> {
    Some(match tag {
        T_READ_AT => 4 + 8 + 4,
        T_OPENED => OPENED_BODY,
        T_FRAME => 8 + 4 + 4 + FRAME_BYTES_MAX,
        T_AUDIO => 8 + 2 + 4 + AUDIO_BYTES_MAX,
        T_TAGS => TAGS_HEAD + ART_BYTES_MAX,
        T_ENDED => 0,
        T_ERROR => 2,
        _ => return None,
    })
}

fn fill<R: Read + ?Sized>(r: &mut R, buf: &mut [u8]) -> Result<(), WireError> {
    r.read_exact(buf).map_err(|e| match e.kind() {
        io::ErrorKind::UnexpectedEof => WireError::Truncated,
        kind => WireError::Io(kind),
    })
}

/// The length prefix. The pipe ending before its first byte is [`WireError::Closed`], inside it `Truncated`.
fn length<R: Read + ?Sized>(r: &mut R) -> Result<usize, WireError> {
    let mut buf = [0u8; 4];
    let mut got = 0;
    while got < buf.len() {
        match r.read(&mut buf[got..]) {
            Ok(0) if got == 0 => return Err(WireError::Closed),
            Ok(0) => return Err(WireError::Truncated),
            Ok(n) => got += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(WireError::Io(e.kind())),
        }
    }
    Ok(u32::from_le_bytes(buf) as usize)
}

/// A message's fields after its tag, read one by one: never more than `left`, and nothing allocated for a field
/// before its length was checked.
struct Body<'r, R: ?Sized> {
    r: &'r mut R,
    left: usize,
}

impl<R: Read + ?Sized> Body<'_, R> {
    fn array<const N: usize>(&mut self) -> Result<[u8; N], WireError> {
        if N > self.left {
            return Err(WireError::Malformed);
        }
        let mut buf = [0u8; N];
        fill(&mut *self.r, &mut buf)?;
        self.left -= N;
        Ok(buf)
    }

    fn u8(&mut self) -> Result<u8, WireError> {
        Ok(self.array::<1>()?[0])
    }

    fn u16(&mut self) -> Result<u16, WireError> {
        self.array().map(u16::from_le_bytes)
    }

    fn u32(&mut self) -> Result<u32, WireError> {
        self.array().map(u32::from_le_bytes)
    }

    fn u64(&mut self) -> Result<u64, WireError> {
        self.array().map(u64::from_le_bytes)
    }

    fn flag(&mut self) -> Result<bool, WireError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(WireError::Malformed),
        }
    }

    /// `n` bytes, `n` checked against what is left before anything is allocated.
    fn bytes(&mut self, n: usize) -> Result<Vec<u8>, WireError> {
        if n > self.left {
            return Err(WireError::Malformed);
        }
        let mut buf = vec![0u8; n];
        fill(&mut *self.r, &mut buf)?;
        self.left -= n;
        Ok(buf)
    }

    /// The bulk that ends the message: exactly `n` bytes must be left.
    fn rest(&mut self, n: usize) -> Result<Vec<u8>, WireError> {
        if n != self.left {
            return Err(WireError::Malformed);
        }
        self.bytes(n)
    }

    fn text(&mut self) -> Result<String, WireError> {
        let n = usize::from(self.u16()?);
        if n > MAX_TEXT_BYTES {
            return Err(WireError::TooLarge);
        }
        let text = String::from_utf8(self.bytes(n)?).map_err(|_| WireError::Malformed)?;
        check_text(&text)?;
        Ok(text)
    }

    fn finish(self) -> Result<(), WireError> {
        if self.left == 0 {
            Ok(())
        } else {
            Err(WireError::Malformed)
        }
    }
}

/// Reads a length and a tag, and refuses a kind this direction does not have or a length over its maximum.
fn open<'r, R: Read + ?Sized>(
    r: &'r mut R,
    max_after_tag: fn(u8) -> Option<usize>,
) -> Result<(u8, Body<'r, R>), WireError> {
    let len = length(&mut *r)?;
    if len == 0 {
        return Err(WireError::Malformed);
    }
    let mut tag = [0u8; 1];
    fill(&mut *r, &mut tag)?;
    let tag = tag[0];
    let max = max_after_tag(tag).ok_or(WireError::UnknownTag(tag))?;
    let left = len - 1;
    if left > max {
        return Err(WireError::TooLarge);
    }
    Ok((tag, Body { r, left }))
}

/// Reads one message from the helper. Every length is checked before anything is allocated for it.
pub fn read_to_app<R: Read + ?Sized>(r: &mut R) -> Result<FromHelper, WireError> {
    let (tag, mut body) = open(r, from_helper_max)?;
    let msg = match tag {
        T_READ_AT => {
            let id = body.u32()?;
            let offset = body.u64()?;
            let len = body.u32()?;
            check_read_len(len)?;
            FromHelper::ReadAt { id, offset, len }
        }
        T_OPENED => {
            let duration_ms = body.u64()?;
            let video = match body.flag()? {
                false => None,
                true => Some(VideoInfo {
                    w: body.u32()?,
                    h: body.u32()?,
                    fps_milli: body.u32()?,
                    codec: Codec::of(body.u8()?)?,
                }),
            };
            let audio = match body.flag()? {
                false => None,
                true => Some(AudioInfo {
                    rate: body.u32()?,
                    channels: body.u16()?,
                    codec: Codec::of(body.u8()?)?,
                }),
            };
            let reduced_quality = body.flag()?;
            let no_sound = match body.flag()? {
                false => None,
                true => Some(Codec::of(body.u8()?)?),
            };
            let opened = Opened {
                duration_ms,
                video,
                audio,
                reduced_quality,
                no_sound,
            };
            check_opened(&opened)?;
            FromHelper::Opened(opened)
        }
        T_FRAME => {
            let pts_ms = body.u64()?;
            let w = body.u32()?;
            let h = body.u32()?;
            check_frame(w, h)?;
            let rgba = body.rest(pixel_bytes(w, h))?;
            FromHelper::Frame { pts_ms, w, h, rgba }
        }
        T_AUDIO => {
            let pts_ms = body.u64()?;
            let channels = body.u16()?;
            let count = body.u32()? as usize;
            check_audio(channels, count)?;
            let raw = body.rest(count * 4)?;
            let samples = raw
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                .collect();
            FromHelper::Audio {
                pts_ms,
                channels,
                samples,
            }
        }
        T_TAGS => {
            let title = body.text()?;
            let artist = body.text()?;
            let art = match body.flag()? {
                false => None,
                true => {
                    let w = body.u32()?;
                    let h = body.u32()?;
                    check_art(w, h)?;
                    Some((w, h, body.rest(pixel_bytes(w, h))?))
                }
            };
            FromHelper::Tags(Tags { title, artist, art })
        }
        T_ENDED => FromHelper::Ended,
        T_ERROR => FromHelper::Error(ErrorCode::of(body.array()?)?),
        _ => return Err(WireError::UnknownTag(tag)),
    };
    body.finish()?;
    Ok(msg)
}

/// Reads one message from the app.
pub fn read_to_helper<R: Read + ?Sized>(r: &mut R) -> Result<ToHelper, WireError> {
    let (tag, mut body) = open(r, to_helper_max)?;
    let msg = match tag {
        T_OPEN => ToHelper::Open {
            len: body.u64()?,
            hint: match body.u8()? {
                0 => KindHint::Audio,
                1 => KindHint::Video,
                _ => return Err(WireError::Malformed),
            },
        },
        T_DATA => {
            let id = body.u32()?;
            let n = body.u32()?;
            if n > MAX_READ {
                return Err(WireError::TooLarge);
            }
            ToHelper::Data {
                id,
                bytes: body.rest(n as usize)?,
            }
        }
        T_DATA_FAILED => ToHelper::DataFailed { id: body.u32()? },
        T_WANT_VIDEO => {
            let w = body.u32()?;
            let h = body.u32()?;
            check_frame(w, h)?;
            ToHelper::WantVideo { w, h }
        }
        T_WANT_AUDIO => {
            let rate = body.u32()?;
            let channels = body.u16()?;
            let frames = body.u32()?;
            check_want_audio(rate, channels, frames)?;
            ToHelper::WantAudio { rate, channels, frames }
        }
        T_SEEK => ToHelper::Seek { at_ms: body.u64()? },
        T_CLOSE => ToHelper::Close,
        T_LAST_HELPER => ToHelper::LastHelper,
        _ => return Err(WireError::UnknownTag(tag)),
    };
    body.finish()?;
    Ok(msg)
}
