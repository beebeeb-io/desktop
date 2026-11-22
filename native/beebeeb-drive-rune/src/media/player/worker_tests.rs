//! The worker's checks on what the helper answers, called on ONE `Worker` directly: no thread, no helper, no device.
//! Each seam is a fake that refuses (nothing is opened or started here); the player is a real one, held by the test
//! so the worker's `publish` reaches it. The rows are the #462 review's.

use std::sync::{Arc, mpsc};

use beebeeb_media_protocol::{AudioInfo, Codec, FromHelper, KindHint, Opened, Tags, VideoInfo};

use super::{AudioWant, Exit, Start, VideoWant, Worker};
use crate::media::audio_out::AudioError;
use crate::media::player::{
    AudioFactory, AudioSink, Clock, Cmd, Link, LinkFactory, MediaBytes, MediaItem, MediaOpener, PlayerHandle, Refusal,
    Slots,
};
use crate::media::video::VideoSurface;

struct NoBytes;

impl MediaOpener for NoBytes {
    fn open(&self, _file_id: &str) -> Result<Box<dyn MediaBytes>, Refusal> {
        Err(Refusal::CouldNotPlay)
    }
}

struct NoHelper;

impl LinkFactory for NoHelper {
    fn start(&self) -> Result<Box<dyn Link>, Refusal> {
        Err(Refusal::NotInstalled)
    }
}

struct NoDevice;

impl AudioFactory for NoDevice {
    fn start(&self, _rate_hint: u32) -> Result<Box<dyn AudioSink>, AudioError> {
        Err(AudioError::NoDevice)
    }
}

/// Time stands still at 0.
struct Still;

impl Clock for Still {
    fn now_ms(&self) -> u64 {
        0
    }
}

/// One second of 25 fps video, or one second of stereo sound.
fn opened(kind: KindHint) -> Opened {
    let video = (kind == KindHint::Video).then_some(VideoInfo {
        w: 32,
        h: 24,
        fps_milli: 25_000,
        codec: Codec::Vp9,
    });
    let audio = (kind == KindHint::Audio).then_some(AudioInfo {
        rate: 48_000,
        channels: 2,
        codec: Codec::Flac,
    });
    Opened {
        duration_ms: 1_000,
        video,
        audio,
        reduced_quality: false,
        no_sound: None,
    }
}

/// A worker for one `kind` item, before `Opened`, with the player it publishes to (held here) and its command
/// channel (held so the worker's commands stay connected).
fn worker(kind: KindHint) -> (Worker, PlayerHandle, mpsc::Sender<Cmd>) {
    let handle = PlayerHandle::new(
        Arc::new(NoBytes),
        Arc::new(NoHelper),
        Arc::new(NoDevice),
        None,
        VideoSurface::new(),
    );
    let (commands, taken) = mpsc::channel();
    let worker = Worker::new(Start {
        generation: 0,
        player: Arc::downgrade(&handle.inner),
        item: MediaItem {
            file_id: "item".to_owned(),
            kind,
            length_ms: 0,
            press: crate::media::player::Press {
                session: beebeeb_engine::account::SessionGeneration::for_test(1),
                account: None,
            },
        },
        opener: Arc::new(NoBytes),
        links: Arc::new(NoHelper),
        audio: Arc::new(NoDevice),
        clock: Arc::new(Still),
        waker: None,
        system_on: false,
        slots: Arc::new(Slots::default()),
        commands: taken,
        previous: None,
    });
    (worker, handle, commands)
}

/// The same worker after its `Opened` (set as the worker's own `on_opened` sets it, without a sound device).
fn opened_worker(kind: KindHint) -> (Worker, PlayerHandle, mpsc::Sender<Cmd>) {
    let (mut worker, handle, commands) = worker(kind);
    let opened = opened(kind);
    worker.frame_ms = 40;
    worker.opened = Some(opened);
    (worker, handle, commands)
}

fn broke(result: Result<(), Exit>) -> bool {
    matches!(result, Err(Exit::Refused(Refusal::CouldNotPlay)))
}

fn want_frame(worker: &mut Worker) {
    worker.want_video = Some(VideoWant {
        w: 32,
        h: 24,
        by: u64::MAX,
        stale: false,
    });
}

fn frame(pts_ms: u64) -> FromHelper {
    FromHelper::Frame {
        pts_ms,
        w: 32,
        h: 24,
        rgba: vec![7; 32 * 24 * 4],
    }
}

/// P3-1: a frame or sound whose time lies past the item's length and one frame-time breaks the rules: the helper
/// is ended ("This file could not be played."), never a frame that waits for ever or a clock run to the end of
/// `u64`. The last frame-time itself is fine.
#[test]
fn an_answer_timed_past_the_end_ends_the_helper() {
    let (mut w, _player, _commands) = opened_worker(KindHint::Video);
    want_frame(&mut w);
    assert!(w.handle(frame(1_040)).is_ok(), "the length and one frame-time");
    assert_eq!(w.held.len(), 1);
    want_frame(&mut w);
    assert!(broke(w.handle(frame(1_041))), "past it");
    want_frame(&mut w);
    assert!(broke(w.handle(frame(u64::MAX - 1))));

    let (mut w, _player, _commands) = opened_worker(KindHint::Audio);
    let want_sound = |w: &mut Worker| {
        w.want_audio = Some(AudioWant {
            channels: 2,
            frames: 4,
            by: u64::MAX,
            stale: false,
        })
    };
    let sound = |pts_ms| FromHelper::Audio {
        pts_ms,
        channels: 2,
        samples: vec![0.25; 8],
    };
    want_sound(&mut w);
    assert!(w.handle(sound(900)).is_ok());
    want_sound(&mut w);
    assert!(broke(w.handle(sound(u64::MAX - 1))), "a time near the end of u64");
}

/// P3-1: an item that says no length is held to the longest the player plays (48 h).
#[test]
fn an_item_without_a_length_is_held_to_48_hours() {
    let (mut w, _player, _commands) = opened_worker(KindHint::Video);
    w.opened = w.opened.map(|opened| Opened {
        duration_ms: 0,
        ..opened
    });
    want_frame(&mut w);
    assert!(w.handle(frame(3_600_000)).is_ok(), "an hour in");
    want_frame(&mut w);
    assert!(broke(w.handle(frame(u64::MAX / 2))));
}

fn tags(art: Option<(u32, u32, Vec<u8>)>) -> FromHelper {
    FromHelper::Tags(Tags {
        title: "A title".to_owned(),
        artist: "An artist".to_owned(),
        art,
    })
}

/// P3-2: the tags come once, after `Opened`. Before it, or a second time, the helper broke the rules.
#[test]
fn one_tags_per_item_and_only_after_opened() {
    let (mut w, _player, _commands) = worker(KindHint::Audio);
    assert!(broke(w.handle(tags(None))), "before Opened");

    let (mut w, _player, _commands) = opened_worker(KindHint::Audio);
    assert!(w.handle(tags(None)).is_ok());
    assert_eq!(w.now.title, "A title");
    assert_eq!(w.now.artist, "An artist");
    assert!(broke(w.handle(tags(None))), "a second Tags");
}

/// P3-2: 0.9.0 shows no artwork. A picture in the tags is checked and dropped: the view and Windows' media controls
/// never get it. A picture that breaks the protocol's limits still ends the helper.
#[test]
fn a_picture_from_the_helper_is_never_kept() {
    let (mut w, player, _commands) = opened_worker(KindHint::Audio);
    assert!(w.handle(tags(Some((2, 2, vec![200; 16])))).is_ok());
    assert!(player.art().is_none(), "the view has no picture");
    assert!(w.now.art.is_none(), "Windows' controls get none");
    assert_eq!(w.now.title, "A title", "the words still count");

    let (mut w, _player, _commands) = opened_worker(KindHint::Audio);
    assert!(
        broke(w.handle(tags(Some((2, 2, vec![200; 15]))))),
        "not 2 x 2 x 4 bytes"
    );
}

/// ux-02: a REAL build writes why nothing plays, and nothing of the file. The id stands in for a name here (the
/// worker is given nothing else of the file).
#[test]
fn a_refusal_in_a_real_build_is_logged_without_the_file_name() {
    let (mut worker, _handle, _commands) = worker(KindHint::Video);
    worker.item.file_id = "Holiday-Rome-Anna.mp4".to_owned();
    let text = crate::media::refusal_log::tests::logged(|| worker.run());
    assert!(text.contains("media refused"), "the refusal is written: {text}");
    assert!(text.contains("reason=failed"), "{text}");
    for word in ["Holiday", "Rome", "Anna", "mp4"] {
        assert!(!text.contains(word), "nothing of the file: {text}");
    }
}

/// ux-02: a rule the helper broke names the kind of rule and the worker's line, and the helper's own code is
/// written before it is folded into "could not play".
#[test]
fn a_broken_rule_and_a_helper_code_are_logged() {
    let (mut worker, _handle, _commands) = worker(KindHint::Video);
    let text = crate::media::refusal_log::tests::logged(|| {
        assert!(broke(worker.handle(FromHelper::Ended)));
        let _ = worker.on_error(beebeeb_media_protocol::ErrorCode::NoSystemDecoder(Codec::Hevc));
    });
    assert!(
        text.contains("media helper stopped by the player what=check at="),
        "{text}"
    );
    assert!(
        text.contains("media helper answered an error answer=no system decoder codec=hevc"),
        "{text}"
    );
}
