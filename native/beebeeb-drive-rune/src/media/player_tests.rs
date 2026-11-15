//! The player's rules (`player.rs`), against a fake helper thread that speaks the protocol over two in-memory pipes
//! (`HelperLink::from_pipes`: the real reader and writer threads, the real decoder), a fake sound device, fake OS
//! media controls and a wall clock the tests move. Each test builds its own `PlayerHandle`: the shell's one
//! (`media::SHELL`) is process-wide, and the shell's own tests lock and sign out through it.
//!
//! The gate's filter: `player_|helper_|lock_|next_|tags_|no_sandbox|played_`.

use std::io::{self, BufReader, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering::SeqCst};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use beebeeb_media_protocol::{
    AudioInfo, Codec, FromHelper, KindHint, MAX_READ, Opened, Tags, ToHelper, VideoInfo, read_to_helper, write_msg,
};
use zeroize::Zeroizing;

use super::worker::writes_played_line;
use super::{
    AudioFactory, AudioSink, Clock, Link, LinkFactory, MediaBytes, MediaItem, MediaOpener, NowPlayingSink, Phase,
    PlayerHandle, Refusal, lock,
};
use crate::media::StopReason;
use crate::media::audio_out::AudioError;
use crate::media::os_controls::{NowPlaying, OsButton};
use crate::media::transport::{HelperLink, LinkError};
use crate::media::video::VideoSurface;

/// The protocol's tag bytes of the two messages these tests write by hand (the protocol's writer refuses them).
const READ_AT_TAG: u8 = 0x81;
const TAGS_TAG: u8 = 0x85;

// ---- The fake helper ----

/// What a fake helper does with each message it reads.
type Script = Box<dyn FnMut(&ToHelper, &mut Reply) + Send>;

fn script(f: impl FnMut(&ToHelper, &mut Reply) + Send + 'static) -> Script {
    Box::new(f)
}

/// The fake helper's standard output.
struct Reply(io::PipeWriter);

impl Reply {
    fn send(&mut self, msg: FromHelper) {
        let _ = write_msg(&mut self.0, &msg);
    }

    /// Bytes as they are, in ONE write: messages that arrive together, or one the protocol's writer refuses.
    fn raw(&mut self, bytes: &[u8]) {
        let _ = self.0.write_all(bytes);
        let _ = self.0.flush();
    }
}

/// Messages encoded back to back (each one the protocol writes), for one `raw` write.
fn together(msgs: &[FromHelper]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for msg in msgs {
        write_msg(&mut bytes, msg).expect("a message the protocol writes");
    }
    bytes
}

/// A message by hand: its length, its tag, its body.
fn raw_message(tag: u8, body: &[u8]) -> Vec<u8> {
    let mut bytes = u32::try_from(body.len() + 1)
        .expect("a short message")
        .to_le_bytes()
        .to_vec();
    bytes.push(tag);
    bytes.extend_from_slice(body);
    bytes
}

/// A fake helper thread on two in-memory pipes. It reads until its input ends (the link let go of it).
fn fake_helper(mut script: Script, seen: Arc<Mutex<Vec<ToHelper>>>) -> HelperLink {
    let (from_helper, helper_out) = io::pipe().expect("a pipe");
    let (helper_in, to_helper) = io::pipe().expect("a pipe");
    thread::spawn(move || {
        let mut input = BufReader::new(helper_in);
        let mut reply = Reply(helper_out);
        while let Ok(msg) = read_to_helper(&mut input) {
            seen.lock().unwrap().push(msg.clone());
            script(&msg, &mut reply);
        }
    });
    HelperLink::from_pipes(Box::new(from_helper), Box::new(to_helper))
}

#[derive(Default)]
struct LinkProbe {
    started: AtomicUsize,
    alive: AtomicUsize,
    max_alive: AtomicUsize,
    killed: AtomicUsize,
}

/// A fake helper's link, counted: started, alive (started and not yet killed or dropped), killed.
struct Counted {
    inner: HelperLink,
    probe: Arc<LinkProbe>,
    live: bool,
}

impl Counted {
    fn gone(&mut self) {
        if std::mem::take(&mut self.live) {
            self.probe.alive.fetch_sub(1, SeqCst);
        }
    }
}

impl Link for Counted {
    fn send(&mut self, msg: &ToHelper) -> io::Result<()> {
        self.inner.send(msg)
    }

    fn recv(&mut self, timeout: Duration) -> Result<FromHelper, LinkError> {
        self.inner.recv(timeout)
    }

    fn kill(&mut self) {
        self.inner.kill();
        self.probe.killed.fetch_add(1, SeqCst);
        self.gone();
    }
}

impl Drop for Counted {
    fn drop(&mut self) {
        self.gone();
    }
}

/// Starts a fake helper with a fresh script from `make`.
struct Links {
    make: Box<dyn Fn() -> Script + Send + Sync>,
    seen: Arc<Mutex<Vec<ToHelper>>>,
    probe: Arc<LinkProbe>,
}

impl LinkFactory for Links {
    fn start(&self) -> Result<Box<dyn Link>, Refusal> {
        let inner = fake_helper((self.make)(), Arc::clone(&self.seen));
        let alive = self.probe.alive.fetch_add(1, SeqCst) + 1;
        self.probe.max_alive.fetch_max(alive, SeqCst);
        self.probe.started.fetch_add(1, SeqCst);
        Ok(Box::new(Counted {
            inner,
            probe: Arc::clone(&self.probe),
            live: true,
        }))
    }
}

// ---- The other fakes ----

#[derive(Default)]
struct SoundProbe {
    started: AtomicUsize,
    dropped: AtomicUsize,
    /// What the device's clock says.
    clock: AtomicU64,
    pushed: AtomicUsize,
}

struct FakeSound(Arc<SoundProbe>);

impl AudioSink for FakeSound {
    fn format(&self) -> (u32, u16) {
        (48_000, 2)
    }

    fn push(&mut self, _pts_ms: u64, samples: &[f32]) -> usize {
        self.0.pushed.fetch_add(samples.len(), SeqCst);
        samples.len()
    }

    fn clock_ms(&self) -> Option<u64> {
        Some(self.0.clock.load(SeqCst))
    }

    fn pause(&mut self) {}

    fn resume(&mut self) {}

    fn flush(&mut self) {}
}

impl Drop for FakeSound {
    fn drop(&mut self) {
        self.0.dropped.fetch_add(1, SeqCst);
    }
}

struct Sounds(Arc<SoundProbe>);

impl AudioFactory for Sounds {
    fn start(&self, _rate_hint: u32) -> Result<Box<dyn AudioSink>, AudioError> {
        self.0.started.fetch_add(1, SeqCst);
        Ok(Box::new(FakeSound(Arc::clone(&self.0))))
    }
}

#[derive(Default)]
struct ControlsProbe {
    shown: AtomicUsize,
    cleared: AtomicUsize,
}

struct Controls(Arc<ControlsProbe>);

impl NowPlayingSink for Controls {
    fn show(&mut self, _now: &NowPlaying) {
        self.0.shown.fetch_add(1, SeqCst);
    }

    fn clear(&mut self) {
        self.0.cleared.fetch_add(1, SeqCst);
    }

    fn take_buttons(&mut self) -> Vec<OsButton> {
        Vec::new()
    }
}

/// An item of `len` bytes; counts the reads.
struct Bytes {
    len: u64,
    reads: Arc<AtomicUsize>,
}

impl MediaBytes for Bytes {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_at(&mut self, _offset: u64, len: u32, _cancel: &AtomicBool) -> Result<Zeroizing<Vec<u8>>, ()> {
        self.reads.fetch_add(1, SeqCst);
        Ok(Zeroizing::new(vec![0x5a; len as usize]))
    }
}

struct Opener {
    len: u64,
    reads: Arc<AtomicUsize>,
}

impl MediaOpener for Opener {
    fn open(&self, _file_id: &str) -> Result<Box<dyn MediaBytes>, Refusal> {
        Ok(Box::new(Bytes {
            len: self.len,
            reads: Arc::clone(&self.reads),
        }))
    }
}

/// The wall clock, still until a test moves it.
#[derive(Default)]
struct Wall(AtomicU64);

impl Clock for Wall {
    fn now_ms(&self) -> u64 {
        self.0.load(SeqCst)
    }
}

// ---- The rig ----

struct Rig {
    handle: PlayerHandle,
    links: Arc<LinkProbe>,
    sound: Arc<SoundProbe>,
    controls: Arc<ControlsProbe>,
    wall: Arc<Wall>,
    seen: Arc<Mutex<Vec<ToHelper>>>,
    reads: Arc<AtomicUsize>,
}

/// A player over the fakes: an item of `len` bytes; each helper it starts runs a fresh script from `make`.
fn rig(len: u64, make: impl Fn() -> Script + Send + Sync + 'static) -> Rig {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let links = Arc::new(LinkProbe::default());
    let sound = Arc::new(SoundProbe::default());
    let controls = Arc::new(ControlsProbe::default());
    let wall = Arc::new(Wall::default());
    let reads = Arc::new(AtomicUsize::new(0));
    let handle = PlayerHandle::new(
        Arc::new(Opener {
            len,
            reads: Arc::clone(&reads),
        }),
        Arc::new(Links {
            make: Box::new(make),
            seen: Arc::clone(&seen),
            probe: Arc::clone(&links),
        }),
        Arc::new(Sounds(Arc::clone(&sound))),
        Some(Box::new(Controls(Arc::clone(&controls)))),
        VideoSurface::new(),
    );
    handle.set_clock(wall.clone());
    Rig {
        handle,
        links,
        sound,
        controls,
        wall,
        seen,
        reads,
    }
}

impl Rig {
    fn open(&self, id: &str, kind: KindHint) {
        self.handle.open(item(id, kind), Vec::new()).expect("the worker starts");
    }

    fn shown(&self) -> Vec<u64> {
        lock(&self.handle.inner).record.shown.clone()
    }

    fn received(&self) -> Vec<u64> {
        lock(&self.handle.inner).record.received.clone()
    }

    fn blanked(&self) -> usize {
        lock(&self.handle.inner).record.blanked
    }

    fn refused(&self) -> Option<Refusal> {
        self.handle.state().and_then(|state| state.refused)
    }

    fn killed(&self) -> usize {
        self.links.killed.load(SeqCst)
    }

    fn seen(&self) -> Vec<ToHelper> {
        self.seen.lock().unwrap().clone()
    }

    fn reads(&self) -> usize {
        self.reads.load(SeqCst)
    }

    /// Waits for the helper to be killed for breaking a rule: "This file could not be played."
    fn wait_refused(&self) {
        wait_until("the helper refused", || self.refused() == Some(Refusal::CouldNotPlay));
        assert_eq!(self.handle.phase(), Phase::Stopped(StopReason::Failed));
        assert!(self.killed() >= 1, "the helper was killed");
    }
}

fn item(id: &str, kind: KindHint) -> MediaItem {
    MediaItem {
        file_id: id.to_owned(),
        kind,
        length_ms: 0,
    }
}

fn wait_until(what: &str, done: impl Fn() -> bool) {
    let end = Instant::now() + Duration::from_secs(10);
    while !done() {
        assert!(Instant::now() < end, "timed out waiting for: {what}");
        thread::sleep(Duration::from_millis(5));
    }
}

fn audio_only() -> Opened {
    Opened {
        duration_ms: 60_000,
        video: None,
        audio: Some(AudioInfo {
            rate: 48_000,
            channels: 2,
            codec: Codec::Aac,
        }),
        reduced_quality: false,
    }
}

fn video_only(w: u32, h: u32) -> Opened {
    Opened {
        duration_ms: 60_000,
        video: Some(VideoInfo {
            w,
            h,
            fps_milli: 30_000,
            codec: Codec::H264,
        }),
        audio: None,
        reduced_quality: false,
    }
}

fn frame(pts_ms: u64, w: u32, h: u32) -> FromHelper {
    FromHelper::Frame {
        pts_ms,
        w,
        h,
        rgba: vec![0x80; w as usize * h as usize * 4],
    }
}

/// One frame of silence.
fn silence(channels: u16) -> FromHelper {
    FromHelper::Audio {
        pts_ms: 0,
        channels,
        samples: vec![0.0; usize::from(channels)],
    }
}

/// Opens as a recording and answers every want for PCM.
fn plays_audio() -> Script {
    script(|msg, out| match msg {
        ToHelper::Open { .. } => out.send(FromHelper::Opened(audio_only())),
        ToHelper::WantAudio { channels, .. } => out.send(silence(*channels)),
        _ => {}
    })
}

/// A `ReadAt` by hand (the protocol's writer refuses one over `MAX_READ`).
fn read_at_by_hand(id: u32, offset: u64, len: u32) -> Vec<u8> {
    let mut body = id.to_le_bytes().to_vec();
    body.extend_from_slice(&offset.to_le_bytes());
    body.extend_from_slice(&len.to_le_bytes());
    raw_message(READ_AT_TAG, &body)
}

// ---- The rules ----

#[test]
fn player_plays_frames_on_the_audio_clock() {
    let r = rig(1_000, || {
        let mut times = [0u64, 33, 66].into_iter();
        script(move |msg, out| match msg {
            // A 4K file: its own size is not held to the 1920 x 1080 the frames are.
            ToHelper::Open { .. } => out.send(FromHelper::Opened(Opened {
                duration_ms: 10_000,
                video: Some(VideoInfo {
                    w: 3840,
                    h: 2160,
                    fps_milli: 30_000,
                    codec: Codec::H264,
                }),
                audio: Some(AudioInfo {
                    rate: 48_000,
                    channels: 2,
                    codec: Codec::Aac,
                }),
                reduced_quality: false,
            })),
            ToHelper::WantVideo { w, h } => {
                if let Some(pts) = times.next() {
                    out.send(frame(pts, *w, *h));
                }
            }
            ToHelper::WantAudio { channels, .. } => out.send(silence(*channels)),
            _ => {}
        })
    });
    r.handle.set_view_size(4, 2);
    r.open("v", KindHint::Video);
    // The audio clock at 0: the frame at 0 shows.
    wait_until("the frame at 0 shown", || r.shown() == [0]);
    // At 40 ms: the frame at 33 shows; the one at 66 waits for its time.
    r.sound.clock.store(40, SeqCst);
    wait_until("the frame at 66 received", || r.received().contains(&66));
    wait_until("the frame at 33 shown", || r.shown() == [0, 33]);
    thread::sleep(Duration::from_millis(100));
    assert_eq!(r.shown(), [0, 33], "the frame at 66 is after the clock");
    assert_eq!(r.handle.phase(), Phase::Playing);
    assert_eq!(r.refused(), None);
    assert!(
        r.seen().contains(&ToHelper::WantVideo { w: 4, h: 2 }),
        "frames are asked at the drawn size"
    );
    r.handle.stop(StopReason::Closed);
}

#[test]
fn helper_frame_of_wrong_size_kills() {
    let r = rig(1_000, || {
        script(|msg, out| match msg {
            ToHelper::Open { .. } => out.send(FromHelper::Opened(video_only(640, 360))),
            // Asked 640 x 360, answered one pixel wider: well inside the protocol's limits.
            ToHelper::WantVideo { w, h } => out.send(frame(0, *w + 1, *h)),
            _ => {}
        })
    });
    r.handle.set_view_size(640, 360);
    r.open("v", KindHint::Video);
    r.wait_refused();
    assert!(r.seen().contains(&ToHelper::WantVideo { w: 640, h: 360 }));
    assert!(r.shown().is_empty(), "nothing of it was shown");
}

#[test]
fn helper_read_over_limit_kills() {
    let r = rig(64 << 20, || {
        script(|msg, out| {
            if let ToHelper::Open { .. } = msg {
                out.raw(&read_at_by_hand(1, 0, MAX_READ + 1));
            }
        })
    });
    r.open("a", KindHint::Audio);
    r.wait_refused();
    assert_eq!(r.reads(), 0, "nothing was read for it");
}

#[test]
fn helper_read_past_end_kills() {
    // 200 bytes at 900 of a 1000-byte file.
    let past = rig(1_000, || {
        script(|msg, out| {
            if let ToHelper::Open { .. } = msg {
                out.send(FromHelper::ReadAt {
                    id: 1,
                    offset: 900,
                    len: 200,
                });
            }
        })
    });
    past.open("a", KindHint::Audio);
    past.wait_refused();
    // An end that does not fit 64 bits.
    let wraps = rig(1_000, || {
        script(|msg, out| {
            if let ToHelper::Open { .. } = msg {
                out.send(FromHelper::ReadAt {
                    id: 1,
                    offset: u64::MAX - 1,
                    len: 4,
                });
            }
        })
    });
    wraps.open("a", KindHint::Audio);
    wraps.wait_refused();
    assert_eq!(past.reads() + wraps.reads(), 0, "nothing was read");
}

#[test]
fn helper_third_read_in_flight_kills() {
    // `n` reads of 16 bytes, sent together: the player sees them in one batch.
    let asks = |n: u32| {
        move || {
            script(move |msg, out| {
                if let ToHelper::Open { .. } = msg {
                    let asked: Vec<FromHelper> = (1..=n)
                        .map(|id| FromHelper::ReadAt {
                            id,
                            offset: u64::from(id) * 16,
                            len: 16,
                        })
                        .collect();
                    out.raw(&together(&asked));
                }
            })
        }
    };
    // Two at once: both answered, the helper lives.
    let two = rig(1_000, asks(2));
    two.open("a", KindHint::Audio);
    wait_until("both reads answered", || {
        two.seen()
            .iter()
            .filter(|msg| matches!(msg, ToHelper::Data { .. }))
            .count()
            == 2
    });
    assert_eq!(two.refused(), None);
    assert_eq!(two.killed(), 0);
    // Three at once: one too many, and none of them is answered.
    let three = rig(1_000, asks(3));
    three.open("a", KindHint::Audio);
    three.wait_refused();
    assert_eq!(three.reads(), 0, "nothing was read for them");
}

#[test]
fn helper_silence_times_out() {
    let r = rig(1_000, || script(|_, _| {}));
    r.open("a", KindHint::Audio);
    wait_until("Open sent", || {
        r.seen().iter().any(|msg| matches!(msg, ToHelper::Open { .. }))
    });
    // 9.999 s of silence: still waiting.
    r.wall.0.store(9_999, SeqCst);
    thread::sleep(Duration::from_millis(100));
    assert_eq!(r.handle.phase(), Phase::Opening);
    assert_eq!(r.killed(), 0);
    // Past 10 s: killed.
    r.wall.0.store(10_001, SeqCst);
    r.wait_refused();
}

#[test]
fn lock_kills_helper_and_stops_audio() {
    let r = rig(1_000, plays_audio);
    r.open("a", KindHint::Audio);
    wait_until("playing", || {
        r.handle.phase() == Phase::Playing && r.sound.started.load(SeqCst) == 1
    });
    let blanked = r.blanked();
    let cleared = r.controls.cleared.load(SeqCst);
    // What the shell's poll does on a lock (`media::stop_all`), on this test's own player.
    r.handle.stop(StopReason::Locked);
    // All of it before `stop` returned.
    assert_eq!(r.killed(), 1, "the helper was killed");
    assert_eq!(r.sound.dropped.load(SeqCst), 1, "the sound was dropped");
    assert_eq!(r.handle.phase(), Phase::Stopped(StopReason::Locked));
    assert_eq!(r.blanked(), blanked + 1, "the frame was blanked");
    assert_eq!(
        r.controls.cleared.load(SeqCst),
        cleared + 1,
        "the OS's media controls were cleared"
    );
    assert_eq!(r.handle.state(), None, "nothing of the file is said any more");
    assert!(r.handle.frame().is_none() && r.handle.art().is_none());
}

#[test]
fn next_kills_the_old_helper_first() {
    let r = rig(1_000, plays_audio);
    let queue = vec![item("a", KindHint::Audio), item("b", KindHint::Audio)];
    r.handle
        .open(item("a", KindHint::Audio), queue)
        .expect("the worker starts");
    wait_until("a plays", || r.handle.phase() == Phase::Playing);
    assert!(r.handle.next(), "there is a next item");
    wait_until("b plays", || {
        r.links.started.load(SeqCst) == 2 && r.handle.phase() == Phase::Playing
    });
    assert_eq!(
        r.links.max_alive.load(SeqCst),
        1,
        "two helpers were never alive at once"
    );
    assert_eq!(r.killed(), 1, "a's helper was killed");
    assert_eq!(r.handle.state().map(|state| state.file_id), Some("b".to_owned()));
    assert!(!r.handle.next(), "b is the last");
}

#[test]
fn tags_are_bounded() {
    // At the protocol's limit or under it, with control characters and a direction mark in it.
    let title = format!("A\u{7}B\u{1b}[31mC\u{202e}D{}", "x".repeat(240));
    assert!(title.chars().count() <= 256);
    let r = rig(1_000, move || {
        let title = title.clone();
        script(move |msg, out| {
            if let ToHelper::Open { .. } = msg {
                out.send(FromHelper::Opened(audio_only()));
                out.send(FromHelper::Tags(Tags {
                    title: title.clone(),
                    artist: "E\nF".to_owned(),
                    art: None,
                }));
            }
        })
    });
    r.open("a", KindHint::Audio);
    wait_until("the tags shown", || {
        r.handle.state().is_some_and(|state| !state.title.is_empty())
    });
    let state = r.handle.state().expect("a state");
    assert_eq!(state.title, format!("AB[31mCD{}", "x".repeat(240)));
    assert_eq!(state.artist, "EF");
    assert!(state.title.chars().count() <= 256);
    // 257 characters: the protocol's decoder refuses it, and the helper is killed.
    let long = rig(1_000, || {
        script(|msg, out| {
            if let ToHelper::Open { .. } = msg {
                let title = "a".repeat(257);
                let mut body = u16::try_from(title.len()).expect("short").to_le_bytes().to_vec();
                body.extend_from_slice(title.as_bytes());
                body.extend_from_slice(&0u16.to_le_bytes()); // no artist
                body.push(0); // no picture
                out.raw(&raw_message(TAGS_TAG, &body));
            }
        })
    });
    long.open("a", KindHint::Audio);
    long.wait_refused();
}

#[cfg(not(windows))]
#[test]
fn no_sandbox_no_playback() {
    // An existing, runnable program: were there an unconfined way to start it, this would.
    let exe = std::env::current_exe().expect("the test's own exe");
    assert_eq!(HelperLink::start(&exe).err(), Some(Refusal::NoSandbox));
}

#[test]
fn helper_non_finite_pcm_kills() {
    let r = rig(1_000, || {
        script(|msg, out| match msg {
            ToHelper::Open { .. } => out.send(FromHelper::Opened(audio_only())),
            ToHelper::WantAudio { channels, .. } => {
                let mut samples = vec![0.25; usize::from(*channels)];
                samples[0] = f32::NAN;
                out.send(FromHelper::Audio {
                    pts_ms: 0,
                    channels: *channels,
                    samples,
                });
            }
            _ => {}
        })
    });
    r.open("a", KindHint::Audio);
    r.wait_refused();
    assert_eq!(r.sound.pushed.load(SeqCst), 0, "nothing of it reached the device");
}

#[test]
fn helper_second_answer_to_one_want_kills() {
    // Two frames for one WantVideo, sent together.
    let frames = rig(1_000, || {
        script(|msg, out| match msg {
            ToHelper::Open { .. } => out.send(FromHelper::Opened(video_only(2, 2))),
            ToHelper::WantVideo { w, h } => out.raw(&together(&[frame(0, *w, *h), frame(33, *w, *h)])),
            _ => {}
        })
    });
    frames.handle.set_view_size(2, 2);
    frames.open("v", KindHint::Video);
    frames.wait_refused();
    // Two PCM answers for one WantAudio.
    let pcm = rig(1_000, || {
        script(|msg, out| match msg {
            ToHelper::Open { .. } => out.send(FromHelper::Opened(audio_only())),
            ToHelper::WantAudio { channels, .. } => out.raw(&together(&[silence(*channels), silence(*channels)])),
            _ => {}
        })
    });
    pcm.open("a", KindHint::Audio);
    pcm.wait_refused();
}

#[test]
fn played_line_only_in_test_builds() {
    use crate::shell::identity::AppIdentity;
    assert!(
        !writes_played_line(AppIdentity::Real.engine()),
        "the released app never writes it"
    );
    assert!(writes_played_line(AppIdentity::Test.engine()), "a test build writes it");
    // The one call that writes it asks that first, of this binary's identity, and of nothing else.
    let source = include_str!("player/worker.rs");
    let line = ["\"media", ".played\""].concat();
    assert_eq!(source.matches(line.as_str()).count(), 1, "one call writes the line");
    let call = source.find(line.as_str()).expect("the call");
    let guard = "if !writes_played_line(crate::shell::identity::engine_identity()) {";
    let asked = source[..call].rfind(guard).expect("the call is behind the question");
    let between: String = source[asked + guard.len()..call].split_whitespace().collect();
    assert!(
        between.starts_with("return;}tracing::info!("),
        "nothing stands between the question and the call: {between}"
    );
}
