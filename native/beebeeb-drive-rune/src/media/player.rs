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
//! **The states:** [`Phase`]: `Idle → Opening → Playing / Paused → Ended`, or `Stopped(reason)` from any of them.
//!
//! **One worker thread per opened item** (`Worker`, in [`worker`]). It opens the item's bytes, starts the helper, sends `Open`, and
//! then, every tick (at most [`TICK`]):
//! 1. takes what the helper sent, as the batch it arrived in, and checks every message against what was asked:
//!    a read inside the item and at most `MAX_IN_FLIGHT_READS` unanswered; one `Opened` (at most 48 h; the file's
//!    own video size is not held to the asked size); a `Frame` exactly the size of the want it answers; `Audio` of the
//!    asked channels, at most the asked frames, every sample finite (then clamped to [-1, 1]); one answer per want;
//!    tags with control characters taken out. Anything else kills the helper: "This file could not be played.";
//! 2. answers the reads from [`MediaBytes`]; the answer's buffer is wiped once it is queued;
//! 3. kills the helper when `Opened` did not come within 10 s, or a want was not answered within 5 s (the time this
//!    side spends reading bytes is not counted against the helper);
//! 4. asks for the next frame (`WantVideo` at the drawn size in device pixels, capped at 1920 x 1080; asked only once
//!    the view said its size) and the next PCM (`WantAudio` at the device's format), one of each at a time;
//! 5. shows a frame when its time is at or before the clock (the sound device's clock; without sound, a wall clock
//!    started at Play). With sound, a frame more than one frame-time late is dropped (unless nothing was shown for
//!    250 ms: a slow PC shows a halting picture, not a black one). Without sound nothing has to stay in step with
//!    the picture, so a late frame is shown and the wall clock goes on from that frame's time: a PC that decodes
//!    slower than the file plays shows every frame, slower, and the place said is the place of the picture.
//!
//! **Stopping is never a request to the worker.** What a run holds that must end at once (the helper's link, the
//! sound) sits in its `Slots`; `stop` (a lock, a sign-out, an account change, a close), `next`, `previous` and `open`
//! take them and end them on the caller's thread: the helper killed, the sound dropped. The worker sees its run
//! cancelled at its next tick, lets go of the item's bytes (and with them the chunk cache) and ends. The frame is
//! blanked and the OS's media controls cleared on the same call. A new run waits for the thread of the one before
//! it before it starts a helper: two helpers are never alive at once.
//!
//! **Session rule:** every answer a worker publishes carries its run's generation; a run that was stopped or
//! replaced publishes nothing more.
//!
//! **No sound is said, never silent:** a file whose audio track does not play here (`Opened::no_sound`) plays its
//! video on the wall clock, and a sound device that went away for good (`AudioSink::failed`) is dropped: a video
//! goes on on the wall clock, a recording pauses where it was. Either way the state's `no_sound` says why.
//!
//! **The system-decoder profile** (decision 2541): a helper in the default profile that answers `NoSystemDecoder`
//! before `Opened` is killed and ONE new helper is started in the profile `transport::profile_for_file` names (only
//! while the switch is on); the same answer from that one is the PC's answer. Never a third helper for one item.
//!
//! **The window is woken** ([`PlayerHandle::set_waker`]) whenever what a view draws of the player changed, so a
//! frame, a place or a refusal reaches the screen without waiting for a press.
//!
//! **TEST builds only** (the build's identity, never the real app), read by the Windows test machine rungs: after 1 s of playback,
//! one `media.played` line (the codec's protocol number, the frames shown and the RMS of the sound in that second,
//! whether the helper was confined); at the first frame, one `media.start` line (each step's time since the press:
//! the bytes, the helper, `Opened`, the sound device, the first want, frame and show) and, when a view first takes
//! that frame, one `media.seen` line; once a second, a `media.pace` line (frames received, shown and dropped, the
//! clock, the time of the frame on screen, the place said). Milliseconds and counts only: no name, id, tag or byte
//! is logged, here or anywhere in the player.

use std::fmt;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use beebeeb_engine::account::SessionGeneration;
use beebeeb_media_protocol::{Codec, FromHelper, KindHint, MEDIA_SYSTEM_DECODER_ON, ToHelper};
use beebeeb_office_sandbox::Profile;
use zeroize::Zeroizing;

use super::StopReason;
use super::audio_out::AudioError;
use super::os_controls::{NowPlaying, OsButton};
use super::transport::LinkError;
use super::video::VideoSurface;

mod worker;

#[cfg(test)]
mod vm_probe;

use worker::{Start, Worker};

// ---- The seams ----

/// The pipes to ONE running helper.
pub trait Link: Send {
    fn send(&mut self, msg: &ToHelper) -> io::Result<()>;
    /// The next message, checked by the protocol's decoder; nothing within `timeout`: [`LinkError::Timeout`].
    fn recv(&mut self, timeout: Duration) -> Result<FromHelper, LinkError>;
    /// Ends the helper now (it is not asked).
    fn kill(&mut self);
    /// Whether the helper runs confined (`transport::HelperLink`: started through the sandbox). A link of the tests
    /// is not.
    fn confined(&self) -> bool {
        false
    }
}

/// Starts a helper for one item.
pub trait LinkFactory: Send + Sync {
    fn start(&self) -> Result<Box<dyn Link>, Refusal>;
    /// Starts a helper in `profile`. Default: the plain start (fakes that never see a System answer).
    fn start_in(&self, profile: Profile) -> Result<Box<dyn Link>, Refusal> {
        let _ = profile;
        self.start()
    }
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
    /// The device is gone for good (lost and not rebuilt). Asked after each push. Default: never.
    fn failed(&self) -> bool {
        false
    }
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
///
/// The player hands its worker this opener bound to the press that came WITH the item ([`MediaItem::press`]), so the
/// worker's `open` is [`MediaOpener::open_for`] with that press: a file is opened for the session and the account of
/// the press that asked for it, never for a process-wide stamp and never for "now".
pub trait MediaOpener: Send + Sync {
    /// Opens `file_id` with no press known. The engine's opener refuses it (`Refusal::NotSignedIn`); the tests'
    /// fakes, which have no session, answer it.
    fn open(&self, file_id: &str) -> Result<Box<dyn MediaBytes>, Refusal>;
    /// Opens `file_id` for `press`. Default: [`MediaOpener::open`] (a fake without a session).
    fn open_for(&self, press: &Press, file_id: &str) -> Result<Box<dyn MediaBytes>, Refusal> {
        let _ = press;
        self.open(file_id)
    }
}

/// The opener a run's worker is handed: the player's opener with the item's press, so the worker's `open` opens for
/// that press and for no other.
struct PressedOpener {
    inner: Arc<dyn MediaOpener>,
    press: Press,
}

impl MediaOpener for PressedOpener {
    fn open(&self, file_id: &str) -> Result<Box<dyn MediaBytes>, Refusal> {
        self.inner.open_for(&self.press, file_id)
    }
}

/// One item's decrypted bytes, by range.
pub trait MediaBytes: Send {
    fn len(&self) -> u64;
    /// Bytes `offset .. offset + len`, in a buffer wiped when it is dropped. `Err`: not read (the player stops).
    fn read_at(&mut self, offset: u64, len: u32, cancel: &AtomicBool) -> Result<Zeroizing<Vec<u8>>, ()>;
}

// ---- What the player plays, and what it says ----

/// The press that asked for an item: the session and the account it was made under, taken on the window's thread at
/// the press. It travels WITH the item to the opener (each item of a queue carries the press that made the queue), so
/// the bytes are opened for that session of that account or not at all.
#[derive(Clone, PartialEq, Eq)]
pub struct Press {
    pub session: SessionGeneration,
    /// Whose session it was (the email, else the user id), as `engine_host::stamp` names an account.
    pub account: Option<String>,
}

/// Never the account.
impl fmt::Debug for Press {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Press({:?})", self.session)
    }
}

/// One file the player can play.
#[derive(Clone, PartialEq, Eq)]
pub struct MediaItem {
    pub file_id: String,
    pub kind: KindHint,
    /// The length the window already knows (the queue's, a thumbnail's); the helper's `Opened` replaces it.
    pub length_ms: u64,
    /// The press the item is opened for.
    pub press: Press,
}

/// The kind and the length, never the id.
impl fmt::Debug for MediaItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "MediaItem({:?}, {} ms)", self.kind, self.length_ms)
    }
}

/// Why nothing plays. The view says it with "Open in another app…" one press away ("Open in default app" where the
/// PC has no system decoder). The sentences are the view's (`frames::viewers::video::refusal_words`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// No decoder here for this format, or its route is off: "This PC can't play <name> files here." The name is
    /// the format's (`Codec::name`), never the file's; empty when the format has no name (the view says "this
    /// kind of file").
    NoDecoder(String),
    /// A 10-bit H.264 stream no decoder on this PC takes (ux-24).
    TenBit,
    /// The video is a codec Beebeeb does not recognise at all, e.g. in a Matroska file (ux-24).
    UnknownCodec,
    /// A HEIC photo: the picture decoder here does not open it (ux-24).
    Heic,
    /// A camera RAW photo: the picture decoder here does not open it (ux-24).
    Raw,
    /// The codec's route is the system's decoder and this PC has none for it. HEVC (decision D-MED-2): the words
    /// name Microsoft's "HEVC Video Extensions"; any other `System` codec (H.264 streams FilmCraft cannot decode,
    /// AAC, E-AC-3, MPEG-4 Part 2, VC-1, WMA): "Windows has no <name> decoder on this PC.", the name from
    /// `Codec::name`. Never the helper's own decoder in its place.
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
    /// This Windows is older than the sandbox needs: "This file can't be shown until Windows is updated."
    NeedsWindowsUpdate,
}

/// Why a file plays without sound. The view says it ("Sound isn't available on this PC", with the codec's name for
/// [`NoSound::Codec`]) with "Open in another app…" one press away; the picture keeps playing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoSound {
    /// The file's audio track is a codec this PC cannot play here (`Opened::no_sound`).
    Codec(Codec),
    /// The sound device went away and was not rebuilt (`AudioSink::failed`).
    Device,
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
    /// The file plays without sound, and why. `None`: its sound plays (or it has no audio track).
    pub no_sound: Option<NoSound>,
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
            .field("no_sound", &self.no_sound)
            .finish_non_exhaustive()
    }
}

/// Where the player is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Nothing was opened.
    Idle,
    /// The bytes, the helper or the helper's `Opened` are not there yet.
    Opening,
    Playing,
    Paused,
    /// The helper said the file ended and all of it was shown and heard.
    Ended,
    /// Stopped: by a close, a lock, a sign-out, an account change, or a failure (the state's `refused` says which).
    Stopped(StopReason),
}

// ---- Limits and times ----

/// The longest a worker waits for the helper before it looks at its own state again.
const TICK: Duration = Duration::from_millis(10);
/// `Opened` within 10 s of `Open`.
const OPEN_WAIT_MS: u64 = 10_000;
/// Every want answered within 5 s.
const ANSWER_WAIT_MS: u64 = 5_000;
/// The longest recording or video the player accepts: 48 h.
const MAX_DURATION_MS: u64 = 48 * 60 * 60 * 1000;
/// A frame-time when the helper gave no frame rate.
const DEFAULT_FRAME_MS: u64 = 40;
/// Frames waiting to be shown, at most.
const HELD_FRAMES: usize = 2;
/// After this long without a frame shown, a late frame is shown instead of dropped.
const SHOW_LATE_AFTER_MS: u64 = 250;
/// One `WantAudio` asks for a tenth of a second.
const AUDIO_PARTS_PER_SECOND: u32 = 10;
/// The OS's media controls get the position this often.
const NOW_PLAYING_EVERY_MS: u64 = 1_000;
/// The test build's `media.played` line comes after this much playback.
const PLAYED_AFTER_MS: u64 = 1_000;
/// Previous past this point starts the item again; before it, goes to the item before.
const RESTART_AFTER_MS: u64 = 3_000;
/// The most messages a worker takes in one tick.
const BATCH_MAX: usize = 128;

// ---- The wall clock ----

/// The wall clock the timeouts (and a video without sound) run on. A trait so the tests can hold time still.
trait Clock: Send + Sync {
    fn now_ms(&self) -> u64;
}

struct Monotonic(Instant);

impl Clock for Monotonic {
    fn now_ms(&self) -> u64 {
        u64::try_from(self.0.elapsed().as_millis()).unwrap_or(u64::MAX)
    }
}

fn lock<T: ?Sized>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

// ---- The player ----

struct Player {
    /// This player, for the workers (they hold it weakly: a player nobody holds ends its run).
    me: Weak<Mutex<Player>>,
    opener: Arc<dyn MediaOpener>,
    links: Arc<dyn LinkFactory>,
    audio: Arc<dyn AudioFactory>,
    now_playing: Option<Box<dyn NowPlayingSink>>,
    /// Wakes the window's loop ([`PlayerHandle::set_waker`]); `None`: nobody draws this player (its tests).
    waker: Option<Waker>,
    surface: VideoSurface,
    /// The drawn stage in device pixels (`set_view_size`); frames are asked at this size, capped at 1920 x 1080.
    view: (u32, u32),
    clock: Arc<dyn Clock>,
    /// Whether a helper may be restarted in the system-decoder profile (decision 2541's switch,
    /// `MEDIA_SYSTEM_DECODER_ON`; the tests set it).
    system_on: bool,
    /// Bumped by every open, stop and failure: a worker of an older generation publishes nothing.
    generation: u64,
    phase: Phase,
    /// What [`PlayerHandle::state`] answers.
    shown: Option<PlayState>,
    /// The picture from the tags.
    art: Option<rune::Pixels>,
    /// A frame was shown since the last blank.
    has_frame: bool,
    /// When the current item was opened (the press), and whether a view took a frame of it since
    /// ([`PlayerHandle::frame`]): the TEST build's `media.seen` line.
    pressed: Option<Instant>,
    seen: bool,
    /// The items Next and Previous move through, and where the player is in it.
    queue: Vec<MediaItem>,
    at: usize,
    run: Option<Run>,
    /// The thread of the run before: the next run waits for it before it starts a helper.
    previous: Option<thread::JoinHandle<()>>,
    /// What the tests read. Filled in test builds only.
    record: Record,
}

/// Wakes the window's loop, from a worker's thread: what a view draws of the player changed.
pub type Waker = Arc<dyn Fn() + Send + Sync>;

/// What the tests read of the player (`player_tests.rs`). Nothing is put in it outside the tests.
#[derive(Default)]
#[cfg_attr(not(test), allow(dead_code))]
struct Record {
    /// The times of the frames shown, in order.
    shown: Vec<u64>,
    /// The times of the frames that passed every check.
    received: Vec<u64>,
    /// How often the frame was blanked.
    blanked: usize,
    /// The worker's steps from the press to the first frame shown: the step and its time in ms.
    steps: Vec<(&'static str, u64)>,
    /// One row a second: ms since the press, frames received, shown, dropped, the clock, the time of the frame on
    /// screen, the position told.
    pace: Vec<[u64; 7]>,
}

/// One opened item's worker, as the player holds it.
struct Run {
    slots: Arc<Slots>,
    commands: mpsc::Sender<Cmd>,
    thread: Option<thread::JoinHandle<()>>,
}

/// What a run holds that a stop must end at once, from any thread, without the worker.
#[derive(Default)]
struct Slots {
    /// Set by [`Slots::end`]; the worker's reads take it too.
    cancel: AtomicBool,
    link: Mutex<Option<Box<dyn Link>>>,
    sound: Mutex<Option<Box<dyn AudioSink>>>,
}

impl Slots {
    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }

    /// Cancels the run: the helper killed, the sound dropped, now. Fine to call more than once.
    fn end(&self) {
        self.cancel.store(true, Ordering::SeqCst);
        let link = lock(&self.link).take();
        if let Some(mut link) = link {
            link.kill();
        }
        drop(lock(&self.sound).take());
    }

    /// Hands the started helper to the run. A run cancelled meanwhile kills it instead: `false`.
    fn put_link(&self, mut link: Box<dyn Link>) -> bool {
        let mut slot = lock(&self.link);
        if self.cancelled() {
            drop(slot);
            link.kill();
            return false;
        }
        *slot = Some(link);
        true
    }

    /// Hands the opened sound to the run. A run cancelled meanwhile drops it instead: `false`.
    fn put_sound(&self, sound: Box<dyn AudioSink>) -> bool {
        let mut slot = lock(&self.sound);
        if self.cancelled() {
            return false;
        }
        *slot = Some(sound);
        true
    }
}

/// What the handle asks of a worker.
enum Cmd {
    Play,
    Pause,
    Seek(u64),
}

impl Player {
    /// Ends the current run: its helper killed and its sound dropped now; its thread is kept for the next run to
    /// wait for.
    fn end_run(&mut self) {
        if let Some(mut run) = self.run.take() {
            run.slots.end();
            if let Some(thread) = run.thread.take() {
                self.previous = Some(thread);
            }
        }
    }

    /// The frame blanked and let go; the tags' picture zeroed and let go.
    fn blank(&mut self) {
        self.surface.blank_and_release();
        self.has_frame = false;
        if let Some(art) = self.art.take() {
            zero_out(&art);
        }
        #[cfg(test)]
        {
            self.record.blanked += 1;
        }
    }

    fn clear_controls(&mut self) {
        if let Some(controls) = self.now_playing.as_mut() {
            controls.clear();
        }
    }

    /// Starts a worker for the queue's current item; whatever played before is ended first.
    fn start_current(&mut self) -> Result<(), Refusal> {
        let Some(item) = self.queue.get(self.at).cloned() else {
            return Err(Refusal::CouldNotPlay);
        };
        self.end_run();
        self.generation += 1;
        self.blank();
        self.pressed = Some(Instant::now());
        self.seen = false;
        self.phase = Phase::Opening;
        self.shown = Some(PlayState {
            file_id: item.file_id.clone(),
            at_ms: 0,
            length_ms: item.length_ms,
            playing: true,
            buffering: true,
            title: String::new(),
            artist: String::new(),
            reduced_quality: false,
            refused: None,
            no_sound: None,
        });
        let slots = Arc::new(Slots::default());
        let (commands, taken) = mpsc::channel();
        // The worker opens the item for the press that came with it, through this run's own opener.
        let opener: Arc<dyn MediaOpener> = Arc::new(PressedOpener {
            inner: Arc::clone(&self.opener),
            press: item.press.clone(),
        });
        let worker = Worker::new(Start {
            generation: self.generation,
            player: self.me.clone(),
            item,
            opener,
            links: Arc::clone(&self.links),
            audio: Arc::clone(&self.audio),
            clock: Arc::clone(&self.clock),
            waker: self.waker.clone(),
            system_on: self.system_on,
            slots: Arc::clone(&slots),
            commands: taken,
            previous: self.previous.take(),
        });
        match thread::Builder::new()
            .name("media-player".into())
            .spawn(move || worker.run())
        {
            Ok(thread) => {
                self.run = Some(Run {
                    slots,
                    commands,
                    thread: Some(thread),
                });
                Ok(())
            }
            Err(_) => {
                super::refusal_log::not_started(super::refusal_log::NotStarted::WorkerThread);
                self.failed(Refusal::CouldNotPlay);
                Err(Refusal::CouldNotPlay)
            }
        }
    }

    /// Stops what plays (see [`PlayerHandle::stop`]).
    fn stop(&mut self, reason: StopReason) {
        self.end_run();
        self.generation += 1;
        if self.phase != Phase::Idle {
            self.phase = Phase::Stopped(reason);
        }
        self.shown = None;
        self.queue.clear();
        self.at = 0;
        self.blank();
        self.clear_controls();
    }

    /// The current item failed: the run ended, the refusal shown. The queue stays (Next skips the item).
    fn failed(&mut self, refusal: Refusal) {
        self.end_run();
        self.generation += 1;
        self.phase = Phase::Stopped(StopReason::Failed);
        if let Some(state) = self.shown.as_mut() {
            state.playing = false;
            state.buffering = false;
            state.title.clear();
            state.artist.clear();
            state.no_sound = None;
            state.refused = Some(refusal);
        }
        self.blank();
        self.clear_controls();
    }

    fn command(&self, cmd: Cmd) {
        if let Some(run) = &self.run {
            let _ = run.commands.send(cmd);
        }
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        if let Some(run) = self.run.take() {
            run.slots.end();
        }
        self.clear_controls();
    }
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
            inner: Arc::new_cyclic(|me| {
                Mutex::new(Player {
                    me: me.clone(),
                    opener,
                    links,
                    audio,
                    now_playing,
                    waker: None,
                    surface,
                    view: (0, 0),
                    clock: Arc::new(Monotonic(Instant::now())),
                    system_on: MEDIA_SYSTEM_DECODER_ON,
                    generation: 0,
                    phase: Phase::Idle,
                    shown: None,
                    art: None,
                    has_frame: false,
                    pressed: None,
                    seen: false,
                    queue: Vec::new(),
                    at: 0,
                    run: None,
                    previous: None,
                    record: Record::default(),
                })
            }),
        }
    }

    fn with<T>(&self, f: impl FnOnce(&mut Player) -> T) -> T {
        f(&mut lock(&self.inner))
    }

    /// The tests' wall clock, before anything is opened.
    #[cfg(test)]
    fn set_clock(&self, clock: Arc<dyn Clock>) {
        self.with(|player| player.clock = clock);
    }

    /// The tests' switch for the system-decoder profile, before anything is opened.
    #[cfg(test)]
    fn set_system_decoder(&self, on: bool) {
        self.with(|player| player.system_on = on);
    }

    /// The window's waker. A worker calls it (never holding the player) whenever what a view draws of the player
    /// changed: the file opened, its first frame, the place (four times a second), play and pause, the end, no
    /// sound, a refusal. The view is not run by a frame alone: the toolkit redraws a picture only where a scene
    /// already shows it, and the first frame of a run is in no scene yet.
    pub fn set_waker(&self, waker: Waker) {
        self.with(|player| player.waker = Some(waker));
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
    /// played before is stopped first. `item` need not be in `queue`; when it is not, it goes first. A refusal that
    /// comes later (no bytes, no helper, the helper's answer) is the state's `refused`.
    pub fn open(&self, item: MediaItem, queue: Vec<MediaItem>) -> Result<(), Refusal> {
        let (queue, at) = with_item(item, queue);
        self.with(|player| {
            player.queue = queue;
            player.at = at;
            player.start_current()
        })
    }

    pub fn play(&self) {
        self.with(|player| player.command(Cmd::Play));
    }

    pub fn pause(&self) {
        self.with(|player| player.command(Cmd::Pause));
    }

    pub fn seek(&self, at_ms: u64) {
        self.with(|player| player.command(Cmd::Seek(at_ms)));
    }

    /// The queue's next item. `false`: there is none.
    pub fn next(&self) -> bool {
        self.with(|player| {
            if player.at + 1 >= player.queue.len() {
                return false;
            }
            player.at += 1;
            let _ = player.start_current();
            true
        })
    }

    /// The queue's previous item (or the start of this one). `false`: nothing moved.
    pub fn previous(&self) -> bool {
        self.with(|player| {
            if player.queue.is_empty() {
                return false;
            }
            let at_ms = player.shown.as_ref().map_or(0, |state| state.at_ms);
            if at_ms > RESTART_AFTER_MS || player.at == 0 {
                if at_ms > 0 && player.run.is_some() {
                    player.command(Cmd::Seek(0));
                    return true;
                }
                return false;
            }
            player.at -= 1;
            let _ = player.start_current();
            true
        })
    }

    /// What plays, or `None` when nothing was opened (or it was stopped by a close, a lock, a sign-out or an
    /// account change).
    pub fn state(&self) -> Option<PlayState> {
        self.with(|player| player.shown.clone())
    }

    /// What the player says of `file_id`: its state while it is the player's current item (playing, paused, ended or
    /// refused), else `None`. A view draws only this: another file's times, tags, picture or refusal are never its own.
    pub fn state_of(&self, file_id: &str) -> Option<PlayState> {
        self.with(|player| player.shown.clone().filter(|state| state.file_id == file_id))
    }

    /// Whether the player holds a live run of `file_id`: it is the current item, its worker runs, and it was not
    /// refused. Play and Pause then go to that run; otherwise a Play opens the file again (a refusal ends the run but
    /// keeps the state, so the state alone is no answer).
    pub fn holds(&self, file_id: &str) -> bool {
        self.with(|player| {
            player.run.is_some()
                && player
                    .shown
                    .as_ref()
                    .is_some_and(|state| state.file_id == file_id && state.refused.is_none())
        })
    }

    /// Where the player is.
    pub fn phase(&self) -> Phase {
        self.with(|player| player.phase)
    }

    /// The video's current frame, for the view that draws it. The first take of a run is the moment the picture
    /// reaches the window (the TEST build's `media.seen` line says how long after the press).
    pub fn frame(&self) -> Option<rune::Pixels> {
        self.with(|player| {
            if !player.has_frame {
                return None;
            }
            if !std::mem::replace(&mut player.seen, true)
                && let Some(pressed) = player.pressed
            {
                worker::log_seen(u64::try_from(pressed.elapsed().as_millis()).unwrap_or(u64::MAX));
            }
            Some(player.surface.pixels())
        })
    }

    /// Whether there is a frame to draw (asked by the poll; the view's own take is [`PlayerHandle::frame`]).
    pub fn has_frame(&self) -> bool {
        self.with(|player| player.has_frame)
    }

    /// The recording's picture from its tags.
    pub fn art(&self) -> Option<rune::Pixels> {
        self.with(|player| player.art.clone())
    }

    /// Stops what plays: the helper killed, the sound dropped, the frame blanked, the OS's media controls cleared,
    /// all before this returns; the item's bytes (and the engine's chunk cache) are let go by the worker at its next
    /// tick.
    pub fn stop(&self, reason: StopReason) {
        self.with(|player| player.stop(reason));
    }
}

/// Sets `pixels` to zeros of the same size before they are let go (the toolkit's atlas copy is overwritten only if
/// the view is drawn again: the gap the photo viewer states too).
fn zero_out(pixels: &rune::Pixels) {
    let (w, h) = pixels.size();
    if w > 0 && h > 0 {
        pixels.set(w, h, vec![0; w as usize * h as usize * 4]);
    }
}

/// The queue with `item` in it, and where `item` is.
fn with_item(item: MediaItem, mut queue: Vec<MediaItem>) -> (Vec<MediaItem>, usize) {
    match queue.iter().position(|other| other.file_id == item.file_id) {
        Some(at) => (queue, at),
        None => {
            queue.insert(0, item);
            (queue, 0)
        }
    }
}

#[cfg(test)]
#[path = "player_tests.rs"]
mod tests;
