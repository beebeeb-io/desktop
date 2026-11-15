//! One opened item's worker (the player's docs, [`super`], say what it does and why): it opens the item's bytes,
//! starts the helper, checks every answer against what was asked, answers the helper's reads, paces the frames on the
//! clock, feeds the OS's media controls, and ends at the first rule the helper breaks or when its run is cancelled.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, Weak, mpsc};
use std::thread;
use std::time::Duration;

use beebeeb_engine::identity::EngineIdentity;
use beebeeb_media_protocol::{
    Codec, ErrorCode, FromHelper, MAX_ART, MAX_AUDIO_FRAMES, MAX_CHANNELS, MAX_FRAME_H, MAX_FRAME_W,
    MAX_IN_FLIGHT_READS, MAX_RATE, MAX_READ, MAX_SOURCE_SIDE, Opened, Tags, ToHelper, cut_text,
};
use zeroize::{Zeroize, Zeroizing};

use super::{
    ANSWER_WAIT_MS, AUDIO_PARTS_PER_SECOND, AudioFactory, BATCH_MAX, Clock, Cmd, DEFAULT_FRAME_MS, HELD_FRAMES,
    LinkFactory, MAX_DURATION_MS, MediaBytes, MediaItem, MediaOpener, NOW_PLAYING_EVERY_MS, OPEN_WAIT_MS,
    PLAYED_AFTER_MS, Phase, Player, PlayerHandle, Refusal, SHOW_LATE_AFTER_MS, Slots, TICK, lock, zero_out,
};
use crate::media::os_controls::{NowPlaying, OsButton};
use crate::media::transport::{LinkError, wipe};

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

/// A frame that passed the checks, waiting for its time.
struct Held {
    pts: u64,
    w: u32,
    h: u32,
    rgba: Vec<u8>,
}

/// What the test build's `media.played` line counts in the first second of playback.
#[derive(Default)]
struct Played {
    /// The time playback started at (the first frame shown or sample played).
    start: Option<u64>,
    frames: u32,
    sum_sq: f64,
    samples: u64,
    written: bool,
}

impl Played {
    fn begin(&mut self, at: u64) {
        self.start.get_or_insert(at);
    }

    fn in_first_second(&self, pts: u64) -> bool {
        self.start
            .is_some_and(|start| pts >= start && pts < start.saturating_add(PLAYED_AFTER_MS))
    }

    fn frame(&mut self, pts: u64) {
        if self.in_first_second(pts) {
            self.frames += 1;
        }
    }

    fn sound(&mut self, samples: &[f32], pts: u64, rate: u32, channels: u16) {
        if !self.in_first_second(pts) {
            return;
        }
        let second = u64::from(rate) * u64::from(channels);
        for sample in samples {
            if self.samples >= second {
                break;
            }
            self.sum_sq += f64::from(*sample) * f64::from(*sample);
            self.samples += 1;
        }
    }

    fn rms(&self) -> f64 {
        if self.samples == 0 {
            return 0.0;
        }
        let rms = (self.sum_sq / self.samples as f64).sqrt();
        (rms * 10_000.0).round() / 10_000.0
    }
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
    /// The sound device's rate and channels; `None`: no sound (no audio track, or no device for a video).
    out: Option<(u32, u16)>,
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
    /// The player says `Ended`.
    ended_shown: bool,
    /// Something was shown or heard since the open or the last seek.
    presented: bool,
    last_shown_at: Option<u64>,
    now: NowPlaying,
    now_dirty: bool,
    now_shown_at: Option<u64>,
    played: Played,
}

impl Worker {
    pub(super) fn new(start: Start) -> Worker {
        let length_ms = start.item.length_ms;
        Worker {
            generation: start.generation,
            player: start.player,
            item: start.item,
            opener: start.opener,
            links: start.links,
            audio: start.audio,
            clock: start.clock,
            slots: start.slots,
            commands: start.commands,
            previous: start.previous,
            bytes: None,
            len: 0,
            opened: None,
            opened_by: None,
            frame_ms: DEFAULT_FRAME_MS,
            out: None,
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
            },
            now_dirty: false,
            now_shown_at: None,
            played: Played::default(),
        }
    }

    pub(super) fn run(mut self) {
        if let Some(previous) = self.previous.take() {
            // The helper of the run before is gone before this one starts its own.
            let _ = previous.join();
        }
        let exit = match self.play() {
            Ok(()) => Exit::Done,
            Err(exit) => exit,
        };
        self.slots.end();
        self.bytes = None;
        self.release();
        if let Exit::Refused(refusal) = exit {
            self.publish(|player| player.failed(refusal));
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
        if self.slots.cancelled() {
            return Err(Exit::Done);
        }
        let link = self.links.start()?;
        if !self.slots.put_link(link) {
            return Err(Exit::Done);
        }
        self.send(&ToHelper::Open {
            len: self.len,
            hint: self.item.kind,
        })?;
        self.opened_by = Some(self.clock.now_ms().saturating_add(OPEN_WAIT_MS));
        loop {
            self.tick()?;
        }
    }

    fn tick(&mut self) -> Result<(), Exit> {
        if self.slots.cancelled() {
            return Err(Exit::Done);
        }
        self.take_commands()?;
        for msg in self.receive()? {
            self.handle(msg)?;
        }
        self.answer_reads()?;
        self.check_deadlines()?;
        self.ask()?;
        self.present()?;
        self.progress()
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
                Err(LinkError::Gone | LinkError::Protocol(_)) => return Err(broke()),
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
            FromHelper::Error(code) => Err(Exit::Refused(refusal_of(code))),
        }
    }

    fn on_opened(&mut self, opened: Opened) -> Result<(), Exit> {
        if self.opened.is_some()
            || opened.duration_ms > MAX_DURATION_MS
            || (opened.video.is_none() && opened.audio.is_none())
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
        if let Some(audio) = opened.audio {
            self.start_sound(audio.rate, opened.video.is_some())?;
        }
        if self.playing && self.out.is_none() {
            self.mono_from = Some(self.clock.now_ms());
        }
        self.now.length_ms = opened.duration_ms;
        self.now_dirty = true;
        self.publish(|player| {
            if let Some(state) = player.shown.as_mut() {
                state.length_ms = opened.duration_ms;
                state.reduced_quality = opened.reduced_quality;
            }
        })
        .ok_or(Exit::Done)
    }

    /// Opens the sound device. Without one, a video plays its picture on the wall clock; a recording cannot play.
    fn start_sound(&mut self, rate_hint: u32, has_video: bool) -> Result<(), Exit> {
        let no_sound = || if has_video { Ok(()) } else { Err(broke()) };
        let Ok(mut sound) = self.audio.start(rate_hint) else {
            return no_sound();
        };
        let (rate, channels) = sound.format();
        if rate == 0 || rate > MAX_RATE || channels == 0 || channels > MAX_CHANNELS {
            return no_sound();
        }
        if !self.playing {
            sound.pause();
        }
        if !self.slots.put_sound(sound) {
            return Err(Exit::Done);
        }
        self.out = Some((rate, channels));
        Ok(())
    }

    fn on_frame(&mut self, pts: u64, w: u32, h: u32, rgba: Vec<u8>) -> Result<(), Exit> {
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
        if want.stale {
            return Ok(());
        }
        #[cfg(test)]
        let _ = self.publish(|player| player.record.received.push(pts));
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
        if want.stale {
            return Ok(());
        }
        self.pcm = samples;
        self.pcm_at = 0;
        self.pcm_base = pts;
        self.pcm_pts = pts;
        Ok(())
    }

    fn on_tags(&mut self, tags: Tags) -> Result<(), Exit> {
        let Tags { title, artist, art } = tags;
        if let Some((w, h, rgba)) = &art
            && (*w == 0
                || *h == 0
                || *w > MAX_ART
                || *h > MAX_ART
                || rgba.len() as u64 != u64::from(*w) * u64::from(*h) * 4)
        {
            return Err(broke());
        }
        let title = clean_text(&title);
        let artist = clean_text(&artist);
        let picture = art.as_ref().map(|(w, h, rgba)| {
            let picture = rune::Pixels::new();
            picture.set(*w, *h, rgba.clone());
            picture
        });
        self.publish(|player| {
            if let Some(state) = player.shown.as_mut() {
                state.title.clone_from(&title);
                state.artist.clone_from(&artist);
            }
            if let Some(before) = std::mem::replace(&mut player.art, picture) {
                zero_out(&before);
            }
        })
        .ok_or(Exit::Done)?;
        self.now.title = title;
        self.now.artist = artist;
        if let Some((_, _, mut before)) = std::mem::replace(&mut self.now.art, art) {
            before.zeroize();
        }
        self.now_dirty = true;
        Ok(())
    }

    /// Answers the reads asked for, oldest first. A read that fails ends playback.
    fn answer_reads(&mut self) -> Result<(), Exit> {
        while let Some(read) = self.reads.pop_front() {
            let started = self.clock.now_ms();
            let got = match self.bytes.as_mut() {
                Some(bytes) => bytes.read_at(read.offset, read.len, &self.slots.cancel),
                None => return Err(Exit::Done),
            };
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
        let clock = self.clock_ms();
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
            if superseded || (late && !starving) {
                continue;
            }
            self.show(frame, now)?;
        }
        self.check_ended(clock)
    }

    fn show(&mut self, frame: Held, now: u64) -> Result<(), Exit> {
        let Held { pts, w, h, rgba } = frame;
        self.publish(|player| {
            player.surface.show(w, h, rgba);
            player.has_frame = true;
            #[cfg(test)]
            player.record.shown.push(pts);
        })
        .ok_or(Exit::Done)?;
        self.last_shown_at = Some(now);
        self.presented = true;
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
        let taken = {
            let left = &self.pcm[self.pcm_at..];
            let mut slot = lock(&self.slots.sound);
            let Some(sound) = slot.as_mut() else {
                return Err(Exit::Done);
            };
            sound.push(self.pcm_pts, left).min(left.len())
        };
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
        for button in buttons.unwrap_or_default() {
            self.press(button)?;
        }
        Ok(())
    }

    /// The test build's one line per item, after the first second of playback.
    fn write_played(&mut self, clock: u64) {
        let due = self
            .played
            .start
            .is_some_and(|start| clock >= start.saturating_add(PLAYED_AFTER_MS));
        if self.played.written || !due {
            return;
        }
        self.played.written = true;
        let codec = self
            .opened
            .and_then(|opened| {
                opened
                    .video
                    .map(|video| video.codec)
                    .or_else(|| opened.audio.map(|audio| audio.codec))
            })
            .unwrap_or(Codec::Other);
        let confined = lock(&self.slots.link).as_ref().is_some_and(|link| link.confined());
        log_played(codec, self.played.frames, self.played.rms(), confined);
    }
}

/// Whether this build writes the `media.played` line: the TEST build only (the VM 401 rung reads it), never the
/// real app, in any profile. Asked of the identity fixed when the binary was built.
pub(super) fn writes_played_line(identity: &EngineIdentity) -> bool {
    !identity.is_real()
}

/// The `media.played` line: the codec as its protocol number (`Codec::ALL`'s order), the frames shown and the RMS of
/// the samples played in the first second, and whether the helper ran confined. Nothing else.
fn log_played(codec: Codec, frames_1s: u32, rms_1s: f64, confined: bool) {
    if !writes_played_line(crate::shell::identity::engine_identity()) {
        return;
    }
    tracing::info!(codec = codec as u8, frames_1s, rms_1s, confined, "media.played");
}

/// The words the view shows for a helper's error code.
fn refusal_of(code: ErrorCode) -> Refusal {
    match code {
        ErrorCode::NoSystemDecoder(codec) => Refusal::NoSystemDecoder(codec),
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
