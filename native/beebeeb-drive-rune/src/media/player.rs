//! The player: one per shell ([`super::set_shell_player`]), outliving the view. For the item it plays it starts ONE
//! confined helper, answers the helper's reads from the item's [`MediaBytes`], shows its frames on the audio clock and
//! hands its PCM to the sound device; it stops everything on a close, a queue change, a lock, a sign-out, an account
//! change, a timeout or any answer that breaks the protocol.
//!
//! The seams (traits), so the player's tests run without a device, a helper or a window: [`Link`] / [`LinkFactory`]
//! (the helper; `transport::HelperLink`), [`AudioSink`] / [`AudioFactory`] (the sound; `audio_out::AudioOut`),
//! [`NowPlayingSink`] (the OS's media controls; `os_controls::OsControls`), [`MediaOpener`] / [`MediaBytes`] (the
//! file's bytes; the engine's range reader, bound in `engine_host::media`).
//!
//! Slice M2 (`media-host`) fills the state machine. The scaffold's player plays nothing: `open` answers
//! [`Refusal::CouldNotPlay`], the rest are no-ops or `None`; `stop` already blanks the frame and clears the OS's
//! media controls.

use std::fmt;
use std::io;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use beebeeb_media_protocol::{Codec, FromHelper, KindHint, ToHelper};
use zeroize::Zeroizing;

use super::StopReason;
use super::audio_out::AudioError;
use super::os_controls::{NowPlaying, OsButton};
use super::transport::LinkError;
use super::video::VideoSurface;

// ---- The seams ----

/// The pipes to ONE running helper.
pub trait Link: Send {
    fn send(&mut self, msg: &ToHelper) -> io::Result<()>;
    /// The next message, checked by the protocol's decoder; nothing within `timeout`: [`LinkError::Timeout`].
    fn recv(&mut self, timeout: Duration) -> Result<FromHelper, LinkError>;
    /// Ends the helper now (it is not asked).
    fn kill(&mut self);
}

/// Starts a helper for one item.
pub trait LinkFactory: Send + Sync {
    fn start(&self) -> Result<Box<dyn Link>, Refusal>;
}

/// The sound device: interleaved f32 PCM at its own format, and the clock everything else follows.
pub trait AudioSink: Send {
    /// The device's rate and channels (the helper resamples to them).
    fn format(&self) -> (u32, u16);
    /// Queues samples that start at `pts_ms`; answers how many were taken.
    fn push(&mut self, pts_ms: u64, samples: &[f32]) -> usize;
    /// Where playback is, in the file's time. `None` before the first sample played.
    fn clock_ms(&self) -> Option<u64>;
    fn pause(&mut self);
    fn resume(&mut self);
    /// Drops what is queued (a seek).
    fn flush(&mut self);
}

/// Opens the sound device.
pub trait AudioFactory: Send + Sync {
    fn start(&self, rate_hint: u32) -> Result<Box<dyn AudioSink>, AudioError>;
}

/// The OS's media controls (DW-1504).
pub trait NowPlayingSink: Send {
    fn show(&mut self, now: &NowPlaying);
    /// Removes the title, the artist and the picture, and turns the controls off.
    fn clear(&mut self);
    /// The buttons pressed since the last call.
    fn take_buttons(&mut self) -> Vec<OsButton>;
}

/// Opens an item's bytes (the engine's range reader, for the session of the press).
pub trait MediaOpener: Send + Sync {
    fn open(&self, file_id: &str) -> Result<Box<dyn MediaBytes>, Refusal>;
}

/// One item's decrypted bytes, by range.
pub trait MediaBytes: Send {
    fn len(&self) -> u64;
    /// Bytes `offset .. offset + len`, in a buffer wiped when it is dropped. `Err`: not read (the player stops).
    fn read_at(&mut self, offset: u64, len: u32, cancel: &AtomicBool) -> Result<Zeroizing<Vec<u8>>, ()>;
}

// ---- What the player plays, and what it says ----

/// One file the player can play.
#[derive(Clone, PartialEq, Eq)]
pub struct MediaItem {
    pub file_id: String,
    pub kind: KindHint,
    /// The length the window already knows (the queue's, a thumbnail's); the helper's `Opened` replaces it.
    pub length_ms: u64,
}

/// The kind and the length, never the id.
impl fmt::Debug for MediaItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "MediaItem({:?}, {} ms)", self.kind, self.length_ms)
    }
}

/// Why nothing plays. The view says it with "Open in another app…" one press away.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// No decoder here for this format, or its route is off: "This PC can't play <name> files here." The name is
    /// the format's (`Codec::name`), never the file's.
    NoDecoder(String),
    /// The codec's route is the system's decoder and this PC has none for it. HEVC (decision D-MED-2): "This PC
    /// can't play HEVC video without Microsoft's HEVC extension."; any other `System` codec (H.264 streams FilmCraft
    /// cannot decode, AAC, E-AC-3, MPEG-4 Part 2, VC-1, WMA): "This PC can't play <name> files here.", the name
    /// from `Codec::name`. Never the helper's own decoder in its place.
    NoSystemDecoder(Codec),
    /// The codec's route is the system's decoder and this Windows has no media codecs at all (an N or KN edition
    /// without the Media Feature Pack; the legal brief of 2026-11-15, F3 caveat 1): "This PC can't play <name>
    /// files: this edition of Windows has no media codecs." The same honest state, never the helper's own decoder.
    NoSystemMedia(Codec),
    /// "This file could not be played." (the helper failed, broke the protocol or timed out; a read failed).
    CouldNotPlay,
    /// No session to read the file for.
    NotSignedIn,
    /// The helper is not next to the app's exe: "Playback isn't available on this PC."
    NotInstalled,
    /// This OS has no sandbox for the helper (macOS until its module, Linux): "Playback isn't available on this PC."
    NoSandbox,
}

/// What the view draws of the player.
#[derive(Clone, PartialEq, Eq)]
pub struct PlayState {
    pub file_id: String,
    pub at_ms: u64,
    pub length_ms: u64,
    pub playing: bool,
    /// Waiting for bytes or the first frame (DW-1516 while the first 2 s buffer).
    pub buffering: bool,
    /// The tag's title; before `Opened`, empty (the view shows the file's name).
    pub title: String,
    pub artist: String,
    /// "Reduced quality on this PC" (decision D-MED-7).
    pub reduced_quality: bool,
    pub refused: Option<Refusal>,
}

/// Times and flags, never the id or a word of a tag.
impl fmt::Debug for PlayState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PlayState")
            .field("at_ms", &self.at_ms)
            .field("length_ms", &self.length_ms)
            .field("playing", &self.playing)
            .field("buffering", &self.buffering)
            .field("reduced_quality", &self.reduced_quality)
            .field("refused", &self.refused)
            .finish_non_exhaustive()
    }
}

// ---- The player ----

struct Player {
    opener: Arc<dyn MediaOpener>,
    links: Arc<dyn LinkFactory>,
    audio: Arc<dyn AudioFactory>,
    now_playing: Option<Box<dyn NowPlayingSink>>,
    surface: VideoSurface,
    /// The drawn stage in device pixels (`set_view_size`); frames are asked at this size, capped at 1920 x 1080.
    view: (u32, u32),
}

/// The shell's player. Clones share it.
#[derive(Clone)]
pub struct PlayerHandle {
    inner: Arc<Mutex<Player>>,
}

impl fmt::Debug for PlayerHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PlayerHandle")
    }
}

impl PlayerHandle {
    pub fn new(
        opener: Arc<dyn MediaOpener>,
        links: Arc<dyn LinkFactory>,
        audio: Arc<dyn AudioFactory>,
        now_playing: Option<Box<dyn NowPlayingSink>>,
        surface: VideoSurface,
    ) -> PlayerHandle {
        PlayerHandle {
            inner: Arc::new(Mutex::new(Player {
                opener,
                links,
                audio,
                now_playing,
                surface,
                view: (0, 0),
            })),
        }
    }

    fn with<T>(&self, f: impl FnOnce(&mut Player) -> T) -> T {
        f(&mut self.inner.lock().unwrap_or_else(PoisonError::into_inner))
    }

    /// The same player (not a copy of its state).
    pub(super) fn same_as(&self, other: &PlayerHandle) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }

    /// The drawn stage's size in device pixels: the size frames are asked for.
    pub fn set_view_size(&self, w: u32, h: u32) {
        self.with(|player| player.view = (w, h));
    }

    /// Plays `item`, with `queue` (the folder's other recordings, in its order) for Next and Previous. Whatever
    /// played before is stopped first.
    pub fn open(&self, item: MediaItem, queue: Vec<MediaItem>) -> Result<(), Refusal> {
        let _ = (item, queue);
        Err(Refusal::CouldNotPlay)
    }

    pub fn play(&self) {}

    pub fn pause(&self) {}

    pub fn seek(&self, at_ms: u64) {
        let _ = at_ms;
    }

    /// The queue's next item. `false`: there is none.
    pub fn next(&self) -> bool {
        false
    }

    /// The queue's previous item (or the start of this one). `false`: nothing moved.
    pub fn previous(&self) -> bool {
        false
    }

    /// What plays, or `None` when nothing was opened.
    pub fn state(&self) -> Option<PlayState> {
        None
    }

    /// The video's current frame.
    pub fn frame(&self) -> Option<rune::Pixels> {
        None
    }

    /// The recording's picture from its tags.
    pub fn art(&self) -> Option<rune::Pixels> {
        None
    }

    /// Stops what plays: the helper killed, the sound dropped, the bytes released, the frame blanked, the OS's media
    /// controls cleared.
    pub fn stop(&self, reason: StopReason) {
        let _ = reason;
        self.with(|player| {
            player.surface.blank_and_release();
            if let Some(controls) = player.now_playing.as_mut() {
                controls.clear();
            }
        });
    }
}
