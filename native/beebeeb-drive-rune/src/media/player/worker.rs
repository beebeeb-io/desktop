//! One opened item's worker (the player's docs, [`super`], say what it does and why): it opens the item's bytes,
//! starts the helper, checks every answer against what was asked, answers the helper's reads, paces the frames on the
//! clock, feeds the OS's media controls, and ends at the first rule the helper breaks or when its run is cancelled.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, Weak, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use beebeeb_media_protocol::{
    ErrorCode, FromHelper, KindHint, MAX_ART, MAX_AUDIO_FRAMES, MAX_CHANNELS, MAX_FRAME_H, MAX_FRAME_W,
    MAX_IN_FLIGHT_READS, MAX_RATE, MAX_READ, MAX_SOURCE_SIDE, Opened, Tags, ToHelper, cut_text,
};
use beebeeb_office_sandbox::Profile;
use zeroize::{Zeroize, Zeroizing};

use super::{
    ANSWER_WAIT_MS, AUDIO_PARTS_PER_SECOND, AudioFactory, BATCH_MAX, Clock, Cmd, DEFAULT_FRAME_MS, HELD_FRAMES,
    LinkFactory, MAX_DURATION_MS, MediaBytes, MediaItem, MediaOpener, NOW_PLAYING_EVERY_MS, NoSound, OPEN_WAIT_MS,
    Phase, Player, PlayerHandle, Refusal, SHOW_LATE_AFTER_MS, Slots, TICK, Waker, lock,
};
use crate::media::os_controls::{NowPlaying, OsButton};
use crate::media::transport::{LinkError, profile_for_file_with, wipe, wipe_answer};

/// Why a worker stopped.
enum Exit {
    /// Stopped, replaced or let go: nothing to say.
    Done,
    /// The item cannot be played: said in the state.
    Refused(Refusal),
}

impl From<Refusal> for Exit {
    fn from(refusal: Refusal) -> Exit {
        Exit::Refused(refusal)
    }
}

/// The helper broke a rule (or a read failed): it is killed, and "This file could not be played." is shown.
fn broke() -> Exit {
    Exit::Refused(Refusal::CouldNotPlay)
}

/// A read the helper asked for and this side has not answered.
struct Read {
    id: u32,
    offset: u64,
    len: u32,
}

/// The frame asked for and not answered yet.
struct VideoWant {
    w: u32,
    h: u32,
    /// The answer is due by this wall-clock time.
    by: u64,
    /// Asked before a seek: its answer is checked, then dropped.
    stale: bool,
}

/// The PCM asked for and not answered yet.
struct AudioWant {
    channels: u16,
    frames: u32,
    by: u64,
    stale: bool,
}

/// A frame that passed the checks, waiting for its time. Its pixels are zeroed when it is dropped (a seek, a stop, a
/// frame too late to show); a frame shown hands them to the view's surface.
struct Held {
    pts: u64,
    w: u32,
    h: u32,
    rgba: Zeroizing<Vec<u8>>,
}

/// What a view draws of a run: when it changes, the window is woken. The place counts in quarter seconds (the
/// scrubber moves; the text says seconds).
#[derive(Clone, Copy, PartialEq, Eq)]
struct Drawn {
    phase: Phase,
    playing: bool,
    buffering: bool,
    quarter: u64,
    length_ms: u64,
    no_sound: Option<NoSound>,
    framed: bool,
    tagged: bool,
}

/// What a worker is started with.
pub(super) struct Start {
    pub(super) generation: u64,
    pub(super) player: Weak<Mutex<Player>>,
    pub(super) item: MediaItem,
    pub(super) opener: Arc<dyn MediaOpener>,
    pub(super) links: Arc<dyn LinkFactory>,
    pub(super) audio: Arc<dyn AudioFactory>,
    pub(super) clock: Arc<dyn Clock>,
    pub(super) waker: Option<Waker>,
    /// Decision 2541's switch, as the player holds it.
    pub(super) system_on: bool,
    pub(super) slots: Arc<Slots>,
    pub(super) commands: mpsc::Receiver<Cmd>,
    pub(super) previous: Option<thread::JoinHandle<()>>,
}

/// One opened item: its bytes, its helper's session, its sound and its frames. Runs on its own thread.
pub(super) struct Worker {
    generation: u64,
    player: Weak<Mutex<Player>>,
    item: MediaItem,
    opener: Arc<dyn MediaOpener>,
    links: Arc<dyn LinkFactory>,
    audio: Arc<dyn AudioFactory>,
    clock: Arc<dyn Clock>,
    waker: Option<Waker>,
    /// What the window was last woken for.
    drawn: Option<Drawn>,
    /// A frame was shown since the open.
    framed: bool,
    system_on: bool,
    /// The profile the current helper was started in: the default one, or (after a restart) the system decoder's.
    profile: Profile,
    /// How many helpers were started for this item (at most 2).
    starts: u32,
    slots: Arc<Slots>,
    commands: mpsc::Receiver<Cmd>,
    previous: Option<thread::JoinHandle<()>>,
    /// The item's bytes, from the opener; dropped (with the engine's chunk cache) when the worker ends.
    bytes: Option<Box<dyn MediaBytes>>,
    len: u64,
    opened: Option<Opened>,
    /// `Opened` is due by this wall-clock time.
    opened_by: Option<u64>,
    frame_ms: u64,
    /// The sound device's rate and channels; `None`: no sound (no audio track, no device for a video, a track that
    /// does not play here, or a device that went away).
    out: Option<(u32, u16)>,
    /// A recording's sound device went away: Play asks for a device again.
    device_lost: bool,
    reads: VecDeque<Read>,
    want_video: Option<VideoWant>,
    want_audio: Option<AudioWant>,
    held: VecDeque<Held>,
    /// The last PCM answer, checked and clamped; `pcm_at` of it was taken by the device.
    pcm: Zeroizing<Vec<f32>>,
    pcm_at: usize,
    /// The time of `pcm`'s first sample, and of its first sample not taken yet.
    pcm_base: u64,
    pcm_pts: u64,
    playing: bool,
    /// Where the clock stands while nothing moves it (the start, a seek, a pause without sound).
    base_ms: u64,
    /// Without sound: the wall-clock time the picture started moving from `base_ms`.
    mono_from: Option<u64>,
    /// The helper said `Ended`.
    ended: bool,
    /// The item's one `Tags` came.
    tags_taken: bool,
    /// The player says `Ended`.
    ended_shown: bool,
    /// Something was shown or heard since the open or the last seek.
    presented: bool,
    last_shown_at: Option<u64>,
    now: NowPlaying,
    now_dirty: bool,
    now_shown_at: Option<u64>,
    played: Played,
    trace: Trace,
}

impl Worker {
    pub(super) fn new(start: Start) -> Worker {
        let length_ms = start.item.length_ms;
        let video = start.item.kind == KindHint::Video;
        Worker {
            generation: start.generation,
            player: start.player,
            item: start.item,
            opener: start.opener,
            links: start.links,
            audio: start.audio,
            clock: start.clock,
            waker: start.waker,
            drawn: None,
            framed: false,
            system_on: start.system_on,
            profile: Profile::default(),
            starts: 0,
            slots: start.slots,
            commands: start.commands,
            previous: start.previous,
            bytes: None,
            len: 0,
            opened: None,
            opened_by: None,
            frame_ms: DEFAULT_FRAME_MS,
            out: None,
            device_lost: false,
            reads: VecDeque::new(),
            want_video: None,
            want_audio: None,
            held: VecDeque::new(),
            pcm: Zeroizing::new(Vec::new()),
            pcm_at: 0,
            pcm_base: 0,
            pcm_pts: 0,
            playing: true,
            base_ms: 0,
            mono_from: None,
            ended: false,
            tags_taken: false,
            ended_shown: false,
            presented: false,
            last_shown_at: None,
            now: NowPlaying {
                title: String::new(),
                artist: String::new(),
                art: None,
                playing: false,
                at_ms: 0,
                length_ms,
                video,
            },
            now_dirty: false,
            now_shown_at: None,
            played: Played::default(),
            trace: Trace::new(),
        }
    }

    pub(super) fn run(mut self) {
        if let Some(previous) = self.previous.take() {
            // The helper of the run before is gone before this one starts its own.
            let _ = previous.join();
        }
        self.trace.mark("joined");
        let exit = match self.play() {
            Ok(()) => Exit::Done,
            Err(exit) => exit,
        };
        self.write_start();
        self.slots.end();
        self.bytes = None;
        self.release();
        if let Exit::Refused(refusal) = exit
            && self.publish(|player| player.failed(refusal)).is_some()
        {
            self.wake();
        }
    }

    /// Wakes the window's loop. Never called with the player held.
    fn wake(&self) {
        if let Some(waker) = self.waker.as_ref() {
            waker();
        }
    }

    /// Wipes what the worker still holds of the item.
    fn release(&mut self) {
        self.pcm.zeroize();
        self.pcm_at = 0;
        if let Some((_, _, rgba)) = self.now.art.as_mut() {
            rgba.zeroize();
        }
        self.now.art = None;
        self.held.clear();
        self.reads.clear();
    }

    /// Runs `f` on the player while this run is its current one. `None`: stopped, replaced or let go.
    fn publish<T>(&self, f: impl FnOnce(&mut Player) -> T) -> Option<T> {
        let inner = self.player.upgrade()?;
        let mut player = lock(&inner);
        if player.generation != self.generation {
            return None;
        }
        Some(f(&mut player))
    }

    fn play(&mut self) -> Result<(), Exit> {
        if self.slots.cancelled() {
            return Err(Exit::Done);
        }
        let bytes = self.opener.open(&self.item.file_id)?;
        self.len = bytes.len();
        self.bytes = Some(bytes);
        self.trace.mark("bytes");
        if self.slots.cancelled() {
            return Err(Exit::Done);
        }
        self.start_helper()?;
        loop {
            self.tick()?;
        }
    }

    /// Starts a helper in `self.profile`, hands it to the run and sends it `Open`.
    fn start_helper(&mut self) -> Result<(), Exit> {
        let link = self.links.start_in(self.profile)?;
        self.starts += 1;
        self.trace.mark("helper");
        if !self.slots.put_link(link) {
            return Err(Exit::Done);
        }
        self.send(&ToHelper::Open {
            len: self.len,
            hint: self.item.kind,
        })?;
        self.opened_by = Some(self.clock.now_ms().saturating_add(OPEN_WAIT_MS));
        Ok(())
    }

    /// The helper's refusal. `NoSystemDecoder` before `Opened`, from a helper in the default profile, names a stream
    /// whose route is `System`: while the switch is on, ONE new helper is started in the system decoder's profile
    /// (decision 2541). The same answer from that one, or with the switch off, is the PC's answer.
    fn on_error(&mut self, code: ErrorCode) -> Result<(), Exit> {
        if let ErrorCode::NoSystemDecoder(codec) = code
            && self.opened.is_none()
            && self.profile == Profile::NoCapabilities
            && let Some(profile) = profile_for_file_with(self.system_on, Some(codec))
        {
            return self.restart_in(profile);
        }
        Err(Exit::Refused(refusal_of(code)))
    }

    /// Kills the helper, then starts the new one in `profile` and sends `Open` again: the same length and hint, the
    /// reads start over from the item's bytes (which stay open, with their reader's budget).
    fn restart_in(&mut self, profile: Profile) -> Result<(), Exit> {
        let old = lock(&self.slots.link).take();
        match old {
            Some(mut old) => old.kill(),
            None => return Err(Exit::Done),
        }
        if self.slots.cancelled() {
            return Err(Exit::Done);
        }
        self.reads.clear();
        self.want_video = None;
        self.want_audio = None;
        self.opened_by = None;
        self.profile = profile;
        self.start_helper()
    }

    fn tick(&mut self) -> Result<(), Exit> {
        if self.slots.cancelled() {
            return Err(Exit::Done);
        }
        self.take_commands()?;
        self.trace.ticks += 1;
        let mut batch = self.receive()?.into_iter();
        let handled = self.handle_batch(&mut batch);
        // What was not handled (the rule broken before it, a restart) is zeroed before it is let go.
        for mut rest in batch {
            wipe_answer(&mut rest);
        }
        handled?;
        self.answer_reads()?;
        self.check_deadlines()?;
        self.ask()?;
        self.present()?;
        self.progress()
    }

    /// Handles the batch's answers in order, until one breaks a rule or a restart begins (the rest of the batch came
    /// from the helper the restart killed): what is left stays in `batch`.
    fn handle_batch(&mut self, batch: &mut std::vec::IntoIter<FromHelper>) -> Result<(), Exit> {
        let starts = self.starts;
        for msg in batch.by_ref() {
            self.handle(msg)?;
            if self.starts != starts {
                break;
            }
        }
        Ok(())
    }

    fn send(&self, msg: &ToHelper) -> Result<(), Exit> {
        let mut slot = lock(&self.slots.link);
        let Some(link) = slot.as_mut() else {
            return Err(Exit::Done);
        };
        link.send(msg).map_err(|_| broke())
    }

    /// What the helper sent: waits at most a tick for the first message, then takes what is already here (the rest
    /// of its batch), so the checks see together what the helper sent together.
    fn receive(&mut self) -> Result<Vec<FromHelper>, Exit> {
        let mut batch = Vec::new();
        let mut wait = TICK;
        while batch.len() < BATCH_MAX {
            let got = {
                let mut slot = lock(&self.slots.link);
                let Some(link) = slot.as_mut() else {
                    return Err(Exit::Done);
                };
                link.recv(wait)
            };
            match got {
                Ok(msg) => batch.push(msg),
                Err(LinkError::Timeout) => break,
                Err(LinkError::Gone | LinkError::Protocol(_)) => {
                    batch.iter_mut().for_each(wipe_answer);
                    return Err(broke());
                }
            }
            wait = Duration::ZERO;
        }
        Ok(batch)
    }

    fn handle(&mut self, msg: FromHelper) -> Result<(), Exit> {
        match msg {
            FromHelper::ReadAt { id, offset, len } => {
                if len == 0 || len > MAX_READ {
                    return Err(broke());
                }
                match offset.checked_add(u64::from(len)) {
                    Some(end) if end <= self.len => {}
                    _ => return Err(broke()),
                }
                self.reads.push_back(Read { id, offset, len });
                if self.reads.len() > MAX_IN_FLIGHT_READS {
                    return Err(broke());
                }
                Ok(())
            }
            FromHelper::Opened(opened) => self.on_opened(opened),
            FromHelper::Frame { pts_ms, w, h, rgba } => self.on_frame(pts_ms, w, h, rgba),
            FromHelper::Audio {
                pts_ms,
                channels,
                samples,
            } => self.on_audio(pts_ms, channels, samples),
            FromHelper::Tags(tags) => self.on_tags(tags),
            FromHelper::Ended => {
                if self.opened.is_none() {
                    return Err(broke());
                }
                self.ended = true;
                // Nothing more comes: what was asked is not owed any more.
                self.want_video = None;
                self.want_audio = None;
                Ok(())
            }
            FromHelper::Error(code) => self.on_error(code),
        }
    }

    fn on_opened(&mut self, opened: Opened) -> Result<(), Exit> {
        if self.opened.is_some()
            || opened.duration_ms > MAX_DURATION_MS
            || (opened.video.is_none() && opened.audio.is_none())
            // A track without sound is said only beside a video that plays, never beside a track that plays (the
            // protocol's decoder refuses both; a link that skips it is held to it here).
            || (opened.no_sound.is_some() && (opened.video.is_none() || opened.audio.is_some()))
        {
            return Err(broke());
        }
        // The file's own size (a 4K file says 3840 x 2160): only the frames are held to the size asked.
        if let Some(video) = opened.video
            && (video.w == 0 || video.h == 0 || video.w > MAX_SOURCE_SIDE || video.h > MAX_SOURCE_SIDE)
        {
            return Err(broke());
        }
        self.frame_ms = opened.video.map_or(DEFAULT_FRAME_MS, |video| frame_ms(video.fps_milli));
        self.opened = Some(opened);
        self.opened_by = None;
        self.trace.mark("opened");
        // Without a device a video plays its picture on the wall clock (and says so); a recording cannot play.
        if let Some(audio) = opened.audio {
            let sound = self.open_sound(audio.rate)?;
            self.trace.sound = sound;
            self.trace.mark("sound");
            if !sound && opened.video.is_none() {
                return Err(broke());
            }
        }
        if self.playing && self.out.is_none() {
            self.mono_from = Some(self.clock.now_ms());
        }
        let no_sound = match opened.no_sound {
            Some(codec) => Some(NoSound::Codec(codec)),
            None if opened.audio.is_some() && self.out.is_none() => Some(NoSound::Device),
            None => None,
        };
        self.now.length_ms = opened.duration_ms;
        self.now_dirty = true;
        self.publish(|player| {
            if let Some(state) = player.shown.as_mut() {
                state.length_ms = opened.duration_ms;
                state.reduced_quality = opened.reduced_quality;
                state.no_sound = no_sound;
            }
        })
        .ok_or(Exit::Done)
    }

    /// Opens the sound device. `false`: there is none (or it named a format outside the protocol's limits).
    fn open_sound(&mut self, rate_hint: u32) -> Result<bool, Exit> {
        let Ok(mut sound) = self.audio.start(rate_hint) else {
            return Ok(false);
        };
        let (rate, channels) = sound.format();
        if rate == 0 || rate > MAX_RATE || channels == 0 || channels > MAX_CHANNELS {
            return Ok(false);
        }
        if !self.playing {
            sound.pause();
        }
        if !self.slots.put_sound(sound) {
            return Err(Exit::Done);
        }
        self.out = Some((rate, channels));
        Ok(true)
    }

    /// The sound device went away for good (`AudioSink::failed`: its own code already tried to rebuild it). It is
    /// dropped and the state says so, never silent: a video goes on on the wall clock from `at`; a recording pauses
    /// at `at`.
    fn sound_lost(&mut self, at: u64) -> Result<(), Exit> {
        drop(lock(&self.slots.sound).take());
        self.out = None;
        self.pcm.zeroize();
        self.pcm_at = 0;
        if let Some(want) = self.want_audio.as_mut() {
            // Its answer is checked when it comes, then dropped.
            want.stale = true;
        }
        self.base_ms = at;
        if self.opened.is_none_or(|opened| opened.video.is_none()) {
            self.playing = false;
            self.device_lost = true;
        }
        self.mono_from = self.playing.then(|| self.clock.now_ms());
        self.now_dirty = true;
        self.publish(|player| {
            if let Some(state) = player.shown.as_mut() {
                state.no_sound = Some(NoSound::Device);
            }
        })
        .ok_or(Exit::Done)
    }

    /// Play on a recording whose device went away: asks for a device again (the default one may be another by now)
    /// and plays on from where it paused. `false`: still none; it stays paused, and the state still says why.
    fn sound_back(&mut self) -> Result<bool, Exit> {
        let Some(audio) = self.opened.and_then(|opened| opened.audio) else {
            return Ok(true);
        };
        if !self.device_lost {
            return Ok(true);
        }
        if !self.open_sound(audio.rate)? {
            return Ok(false);
        }
        self.device_lost = false;
        // The helper's PCM went on past where the sound stopped: it starts again from there.
        self.seek(self.base_ms)?;
        self.publish(|player| {
            if let Some(state) = player.shown.as_mut() {
                state.no_sound = None;
            }
        })
        .ok_or(Exit::Done)?;
        Ok(true)
    }

    /// The latest time an answer may carry (P3-1 of the #462 review): the item's length (48 h when it has none) and
    /// one frame-time. Past it the helper is broken: a time near the end of `u64` would stall the picture behind two
    /// frames that never show, and run the clock's sums to their end.
    fn latest_pts(&self) -> u64 {
        let length = match self.opened.map_or(0, |opened| opened.duration_ms) {
            0 => MAX_DURATION_MS,
            length => length,
        };
        length.saturating_add(self.frame_ms)
    }

    fn on_frame(&mut self, pts: u64, w: u32, h: u32, rgba: Vec<u8>) -> Result<(), Exit> {
        // Zeroed whenever it is dropped from here on: refused, stale, or too late to show.
        let rgba = Zeroizing::new(rgba);
        // No want open: a second answer to one want, or a frame nobody asked for.
        let Some(want) = self.want_video.take() else {
            return Err(broke());
        };
        if (w, h) != (want.w, want.h) {
            return Err(broke());
        }
        if rgba.len() as u64 != u64::from(w) * u64::from(h) * 4 {
            return Err(broke());
        }
        if pts > self.latest_pts() {
            return Err(broke());
        }
        if want.stale {
            return Ok(());
        }
        #[cfg(test)]
        let _ = self.publish(|player| player.record.received.push(pts));
        self.trace.mark("frame");
        self.trace.received += 1;
        if let Some(asked) = self.trace.want_at.take() {
            self.trace.answer_us += Trace::us(asked);
        }
        self.held.push_back(Held { pts, w, h, rgba });
        Ok(())
    }

    fn on_audio(&mut self, pts: u64, channels: u16, samples: Vec<f32>) -> Result<(), Exit> {
        let mut samples = Zeroizing::new(samples);
        let Some(want) = self.want_audio.take() else {
            return Err(broke());
        };
        let per_frame = usize::from(channels);
        if channels != want.channels
            || per_frame == 0
            || samples.len() % per_frame != 0
            || samples.len() / per_frame > want.frames as usize
        {
            return Err(broke());
        }
        for sample in samples.iter_mut() {
            if !sample.is_finite() {
                return Err(broke());
            }
            *sample = sample.clamp(-1.0, 1.0);
        }
        if pts > self.latest_pts() {
            return Err(broke());
        }
        if want.stale {
            return Ok(());
        }
        self.pcm = samples;
        self.pcm_at = 0;
        self.pcm_base = pts;
        self.pcm_pts = pts;
        Ok(())
    }

    /// The item's title and artist (P3-2 of the #462 review): ONE `Tags` per item, and only after `Opened`, as the
    /// helper sends it; another one is a broken rule. 0.9.0 shows no artwork: a picture is checked, zeroed and never
    /// kept, so none reaches the view or Windows' media controls.
    fn on_tags(&mut self, tags: Tags) -> Result<(), Exit> {
        let Tags {
            mut title,
            mut artist,
            art,
        } = tags;
        let mut art_ok = true;
        if let Some((w, h, mut rgba)) = art {
            art_ok =
                w > 0 && h > 0 && w <= MAX_ART && h <= MAX_ART && rgba.len() as u64 == u64::from(w) * u64::from(h) * 4;
            rgba.zeroize();
        }
        let in_order = self.opened.is_some() && !self.tags_taken;
        let shown_title = clean_text(&title);
        let shown_artist = clean_text(&artist);
        title.zeroize();
        artist.zeroize();
        if !art_ok || !in_order {
            return Err(broke());
        }
        self.tags_taken = true;
        self.publish(|player| {
            if let Some(state) = player.shown.as_mut() {
                state.title.clone_from(&shown_title);
                state.artist.clone_from(&shown_artist);
            }
        })
        .ok_or(Exit::Done)?;
        self.now.title = shown_title;
        self.now.artist = shown_artist;
        self.now_dirty = true;
        Ok(())
    }

    /// Answers the reads asked for, oldest first. A read that fails ends playback.
    fn answer_reads(&mut self) -> Result<(), Exit> {
        while let Some(read) = self.reads.pop_front() {
            let started = self.clock.now_ms();
            let reading = Instant::now();
            let got = match self.bytes.as_mut() {
                Some(bytes) => bytes.read_at(read.offset, read.len, &self.slots.cancel),
                None => return Err(Exit::Done),
            };
            self.trace.read_us += Trace::us(reading);
            self.not_counted(self.clock.now_ms().saturating_sub(started));
            if self.slots.cancelled() {
                return Err(Exit::Done);
            }
            let mut buf = match got {
                Ok(buf) if buf.len() == read.len as usize => buf,
                _ => return Err(broke()),
            };
            let mut msg = ToHelper::Data {
                id: read.id,
                bytes: std::mem::take(&mut *buf),
            };
            let sent = self.send(&msg);
            wipe(&mut msg);
            sent?;
        }
        Ok(())
    }

    /// The time this side spent reading is not the helper's: its deadlines move by it.
    fn not_counted(&mut self, spent: u64) {
        if let Some(by) = self.opened_by.as_mut() {
            *by = by.saturating_add(spent);
        }
        if let Some(want) = self.want_video.as_mut() {
            want.by = want.by.saturating_add(spent);
        }
        if let Some(want) = self.want_audio.as_mut() {
            want.by = want.by.saturating_add(spent);
        }
    }

    fn check_deadlines(&self) -> Result<(), Exit> {
        let now = self.clock.now_ms();
        let opened_late = self.opened_by.is_some_and(|by| now > by);
        let video_late = self.want_video.as_ref().is_some_and(|want| now > want.by);
        let audio_late = self.want_audio.as_ref().is_some_and(|want| now > want.by);
        if opened_late || video_late || audio_late {
            return Err(broke());
        }
        Ok(())
    }

    /// Asks for the next frame and the next PCM, one of each at a time.
    fn ask(&mut self) -> Result<(), Exit> {
        let Some(opened) = self.opened else {
            return Ok(());
        };
        if self.ended {
            return Ok(());
        }
        if opened.video.is_some() && self.want_video.is_none() && self.held.len() < HELD_FRAMES {
            let view = self.publish(|player| player.view).ok_or(Exit::Done)?;
            if let Some((w, h)) = asked_size(view) {
                self.trace.mark("asked");
                self.trace.want_at = Some(Instant::now());
                self.send(&ToHelper::WantVideo { w, h })?;
                self.want_video = Some(VideoWant {
                    w,
                    h,
                    by: self.clock.now_ms().saturating_add(ANSWER_WAIT_MS),
                    stale: false,
                });
            }
        }
        if let Some((rate, channels)) = self.out
            && self.want_audio.is_none()
            && self.pcm_at >= self.pcm.len()
        {
            let frames = (rate / AUDIO_PARTS_PER_SECOND).clamp(1, MAX_AUDIO_FRAMES);
            self.send(&ToHelper::WantAudio { rate, channels, frames })?;
            self.want_audio = Some(AudioWant {
                channels,
                frames,
                by: self.clock.now_ms().saturating_add(ANSWER_WAIT_MS),
                stale: false,
            });
        }
        Ok(())
    }

    /// Where playback is: the sound device's clock, or (no sound) the wall clock since Play.
    fn clock_ms(&self) -> u64 {
        if self.out.is_some() {
            return lock(&self.slots.sound)
                .as_ref()
                .and_then(|sound| sound.clock_ms())
                .unwrap_or(self.base_ms);
        }
        match self.mono_from {
            Some(from) => self.base_ms.saturating_add(self.clock.now_ms().saturating_sub(from)),
            None => self.base_ms,
        }
    }

    /// Hands PCM to the device and shows the frames whose time came.
    fn present(&mut self) -> Result<(), Exit> {
        self.push_sound()?;
        let mut clock = self.clock_ms();
        let now = self.clock.now_ms();
        while self.held.front().is_some_and(|frame| frame.pts <= clock) {
            let Some(frame) = self.held.pop_front() else {
                break;
            };
            let superseded = self.held.front().is_some_and(|next| next.pts <= clock);
            let late = clock - frame.pts > self.frame_ms;
            let starving = self
                .last_shown_at
                .is_none_or(|at| now.saturating_sub(at) >= SHOW_LATE_AFTER_MS);
            // Sound keeps its own time, so a late frame is dropped to stay in step with it. Without sound the
            // picture is all there is: a late frame is shown, and the wall clock goes on from its time.
            let follows_sound = self.out.is_some();
            if superseded || (late && follows_sound && !starving) {
                self.trace.dropped += 1;
                continue;
            }
            if late && !follows_sound {
                self.base_ms = frame.pts;
                self.mono_from = self.playing.then_some(now);
                clock = frame.pts;
            }
            self.show(frame, now)?;
        }
        self.check_ended(clock)
    }

    fn show(&mut self, frame: Held, now: u64) -> Result<(), Exit> {
        let Held { pts, w, h, mut rgba } = frame;
        // The surface takes the pixels themselves (the view's frame); nothing is left behind here.
        let rgba = std::mem::take(&mut *rgba);
        let handing = Instant::now();
        self.publish(|player| {
            player.surface.show(w, h, rgba);
            player.has_frame = true;
            #[cfg(test)]
            player.record.shown.push(pts);
        })
        .ok_or(Exit::Done)?;
        self.trace.show_us += Trace::us(handing);
        self.last_shown_at = Some(now);
        self.presented = true;
        self.framed = true;
        self.trace.mark("shown");
        self.trace.shown += 1;
        self.trace.shown_pts = pts;
        self.write_start();
        self.played.begin(pts);
        self.played.frame(pts);
        Ok(())
    }

    fn push_sound(&mut self) -> Result<(), Exit> {
        let Some((rate, channels)) = self.out else {
            return Ok(());
        };
        if self.pcm_at >= self.pcm.len() {
            return Ok(());
        }
        let (taken, lost_at) = {
            let left = &self.pcm[self.pcm_at..];
            let mut slot = lock(&self.slots.sound);
            let Some(sound) = slot.as_mut() else {
                return Err(Exit::Done);
            };
            let taken = sound.push(self.pcm_pts, left).min(left.len());
            // Asked after each push: a device gone for good is said, never played on in silence.
            let lost_at = sound.failed().then(|| sound.clock_ms().unwrap_or(self.base_ms));
            (taken, lost_at)
        };
        if let Some(at) = lost_at {
            return self.sound_lost(at);
        }
        if taken == 0 {
            return Ok(());
        }
        self.played.begin(self.pcm_pts);
        self.played.sound(
            &self.pcm[self.pcm_at..self.pcm_at + taken],
            self.pcm_pts,
            rate,
            channels,
        );
        self.pcm_at += taken;
        let frames = (self.pcm_at / usize::from(channels)) as u64;
        self.pcm_pts = self.pcm_base.saturating_add(frames * 1000 / u64::from(rate));
        self.presented = true;
        if self.pcm_at >= self.pcm.len() {
            self.pcm.zeroize();
            self.pcm_at = 0;
        }
        Ok(())
    }

    /// The helper ended and all of it was shown and heard: the player says `Ended`.
    fn check_ended(&mut self, clock: u64) -> Result<(), Exit> {
        let sound_done = self.out.is_none() || clock >= self.pcm_pts;
        if !self.ended || self.ended_shown || !self.held.is_empty() || self.pcm_at < self.pcm.len() || !sound_done {
            return Ok(());
        }
        self.ended_shown = true;
        self.set_playing(false)?;
        self.now_dirty = true;
        Ok(())
    }

    fn take_commands(&mut self) -> Result<(), Exit> {
        loop {
            match self.commands.try_recv() {
                Ok(Cmd::Play) => self.set_playing(true)?,
                Ok(Cmd::Pause) => self.set_playing(false)?,
                Ok(Cmd::Seek(at)) => self.seek(at)?,
                Err(mpsc::TryRecvError::Empty) => return Ok(()),
                Err(mpsc::TryRecvError::Disconnected) => return Err(Exit::Done),
            }
        }
    }

    fn press(&mut self, button: OsButton) -> Result<(), Exit> {
        match button {
            OsButton::Play => self.set_playing(true),
            OsButton::Pause => self.set_playing(false),
            OsButton::Toggle => self.set_playing(!self.playing || self.ended_shown),
            OsButton::Next => {
                self.step(true);
                Ok(())
            }
            OsButton::Previous => {
                self.step(false);
                Ok(())
            }
        }
    }

    /// Next or Previous from the OS's controls: it replaces this run, so it is done by the handle on a thread of
    /// its own (this worker is cancelled by it like by any other caller).
    fn step(&self, forward: bool) {
        let Some(inner) = self.player.upgrade() else {
            return;
        };
        let handle = PlayerHandle { inner };
        let _ = thread::Builder::new().name("media-step".into()).spawn(move || {
            let _moved = if forward { handle.next() } else { handle.previous() };
        });
    }

    fn set_playing(&mut self, on: bool) -> Result<(), Exit> {
        if on && !self.sound_back()? {
            return Ok(());
        }
        if on && self.ended_shown {
            // Play at the end plays it again.
            self.seek(0)?;
        }
        if on == self.playing {
            return Ok(());
        }
        let clock = self.clock_ms();
        self.playing = on;
        self.now_dirty = true;
        if self.out.is_some() {
            let mut slot = lock(&self.slots.sound);
            match slot.as_mut() {
                Some(sound) if on => sound.resume(),
                Some(sound) => sound.pause(),
                None => {}
            }
        } else if self.opened.is_some() {
            if on {
                self.mono_from = Some(self.clock.now_ms());
            } else {
                self.base_ms = clock;
                self.mono_from = None;
            }
        }
        Ok(())
    }

    /// The next answers start at `at` (clamped to the length). What was asked before is checked when it comes and
    /// dropped; what waits to be shown or heard is dropped now.
    fn seek(&mut self, at: u64) -> Result<(), Exit> {
        let Some(opened) = self.opened else {
            return Ok(());
        };
        let at = at.min(opened.duration_ms);
        self.send(&ToHelper::Seek { at_ms: at })?;
        if let Some(sound) = lock(&self.slots.sound).as_mut() {
            sound.flush();
        }
        self.held.clear();
        self.pcm.zeroize();
        self.pcm_at = 0;
        if let Some(want) = self.want_video.as_mut() {
            want.stale = true;
        }
        if let Some(want) = self.want_audio.as_mut() {
            want.stale = true;
        }
        self.base_ms = at;
        self.pcm_base = at;
        self.pcm_pts = at;
        self.mono_from = (self.playing && self.out.is_none()).then(|| self.clock.now_ms());
        self.ended = false;
        self.ended_shown = false;
        self.presented = false;
        self.last_shown_at = None;
        self.now_dirty = true;
        Ok(())
    }

    /// Publishes where playback is, feeds the OS's media controls, takes their buttons; writes the test build's
    /// `media.played` line once.
    fn progress(&mut self) -> Result<(), Exit> {
        let clock = self.clock_ms();
        let now = self.clock.now_ms();
        self.write_played(clock);
        let phase = match self.opened {
            None => Phase::Opening,
            Some(_) if self.ended_shown => Phase::Ended,
            Some(_) if self.playing => Phase::Playing,
            Some(_) => Phase::Paused,
        };
        let length = self.opened.map(|opened| opened.duration_ms);
        let at_ms = length.map_or(0, |length| clock.min(length));
        self.write_pace(clock, at_ms);
        let playing = self.playing && !self.ended_shown;
        let buffering = !self.presented && !self.ended_shown;
        let feed = length.is_some()
            && (self.now_dirty
                || self
                    .now_shown_at
                    .is_none_or(|at| now.saturating_sub(at) >= NOW_PLAYING_EVERY_MS));
        self.now.playing = playing;
        self.now.at_ms = at_ms;
        let now_playing = &self.now;
        let buttons = self
            .publish(|player| {
                player.phase = phase;
                if let Some(state) = player.shown.as_mut() {
                    state.at_ms = at_ms;
                    state.playing = playing;
                    state.buffering = buffering;
                }
                let controls = player.now_playing.as_mut()?;
                if feed {
                    controls.show(now_playing);
                }
                Some(controls.take_buttons())
            })
            .ok_or(Exit::Done)?;
        if feed {
            self.now_dirty = false;
            self.now_shown_at = Some(now);
        }
        let drawn = Drawn {
            phase,
            playing,
            buffering,
            quarter: at_ms / 250,
            length_ms: length.unwrap_or(0),
            no_sound: self
                .publish(|player| player.shown.as_ref().and_then(|state| state.no_sound))
                .flatten(),
            framed: self.framed,
            tagged: self.tags_taken,
        };
        if self.drawn != Some(drawn) {
            self.drawn = Some(drawn);
            self.wake();
        }
        for button in buttons.unwrap_or_default() {
            self.press(button)?;
        }
        Ok(())
    }
}

/// The words the view shows for a helper's error code.
pub(super) fn refusal_of(code: ErrorCode) -> Refusal {
    match code {
        ErrorCode::NoSystemDecoder(codec) => Refusal::NoSystemDecoder(codec),
        ErrorCode::NoSystemMedia(codec) => Refusal::NoSystemMedia(codec),
        ErrorCode::Unsupported(codec) => Refusal::NoDecoder(codec.name().unwrap_or_default().to_string()),
        ErrorCode::NoDecoder => Refusal::NoDecoder(String::new()),
        ErrorCode::Corrupt | ErrorCode::TooLarge | ErrorCode::ReadFailed | ErrorCode::Timeout | ErrorCode::Internal => {
            Refusal::CouldNotPlay
        }
    }
}

/// A frame-time, from the helper's frames per 1000 s.
fn frame_ms(fps_milli: u32) -> u64 {
    match fps_milli {
        0 => DEFAULT_FRAME_MS,
        fps_milli => (1_000_000 / u64::from(fps_milli)).max(1),
    }
}

/// The frame size to ask for: the drawn size, scaled down (keeping its shape) to fit 1920 x 1080. `None` while the
/// view has not said its size.
fn asked_size((w, h): (u32, u32)) -> Option<(u32, u32)> {
    if w == 0 || h == 0 {
        return None;
    }
    if w <= MAX_FRAME_W && h <= MAX_FRAME_H {
        return Some((w, h));
    }
    let (w, h) = (u64::from(w), u64::from(h));
    let (max_w, max_h) = (u64::from(MAX_FRAME_W), u64::from(MAX_FRAME_H));
    let fit = |n: u64, limit: u32| u32::try_from(n.max(1)).unwrap_or(limit).min(limit);
    if w * max_h >= h * max_w {
        Some((MAX_FRAME_W, fit(h * max_w / w, MAX_FRAME_H)))
    } else {
        Some((fit(w * max_h / h, MAX_FRAME_W), MAX_FRAME_H))
    }
}

/// A tag as the app shows it: no control characters (nor the ones that turn text around), at most 256 characters.
fn clean_text(text: &str) -> String {
    let kept: String = text
        .chars()
        .filter(|c| !c.is_control() && !turns_text_around(*c))
        .collect();
    cut_text(&kept)
}

/// The marks that change the direction text is laid out in (a title could show reversed, or move the words after
/// it).
fn turns_text_around(c: char) -> bool {
    matches!(c, '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

mod lines;

pub(super) use lines::log_seen;
#[cfg(test)]
pub(super) use lines::writes_played_line;
use lines::{Played, Trace};

#[cfg(test)]
#[path = "worker_tests.rs"]
mod tests;
