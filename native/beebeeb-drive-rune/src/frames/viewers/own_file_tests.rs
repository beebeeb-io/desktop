//! A recording's or a video's view shows only its OWN file's player state, and its Play (the button and Space, the
//! same press) goes to the viewer's source, which opens the file for the press's session. Review of #462: P1-2 (a
//! Files video never played), P2-3 (another recording's state on this one's view; the button and Space differed),
//! P2-4 (Play after a refusal did nothing). The player here is real, with seams that answer nothing: it holds another
//! file's refusal when a test needs one. Every name is invented.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use beebeeb_engine::account::SessionGeneration;
use beebeeb_media_protocol::KindHint;
use rune::{Env, Key, KeyEvent, Mods, Runtime};

use super::super::kinds::{AudioDoc, Track, VideoDoc};
use super::super::open_from;
use super::super::source::{Body, Details, OpenError, OpenFile, ViewerSource};
use crate::media::audio_out::AudioError;
use crate::media::player::{
    AudioFactory, AudioSink, Link, LinkFactory, MediaBytes, MediaItem, MediaOpener, PlayerHandle, Press, Refusal,
};
use crate::media::video::VideoSurface;
use crate::{init, view, State};

type Calls = Rc<RefCell<Vec<String>>>;

/// A source that plays: it hands the test's player and keeps what Play asked of it.
struct Plays {
    calls: Calls,
    player: PlayerHandle,
}

/// An id starting with `v` is a video, any other a recording alone in its folder.
fn file(id: &str) -> OpenFile {
    let name = format!("Invented {id}");
    let body = if id.starts_with('v') {
        Body::Video(VideoDoc { length: 0 })
    } else {
        Body::Audio(AudioDoc {
            title: name.clone(),
            artist: String::new(),
            length: 0,
            at: 0,
            playing: false,
            folder: "Invented demos".into(),
            queue: vec![Track {
                id: id.into(),
                title: name.clone(),
                length: 0,
                current: true,
            }],
        })
    };
    OpenFile {
        id: id.into(),
        path: Vec::new(),
        name,
        facts: String::new(),
        version: 1,
        body,
        details: Details {
            kind: String::new(),
            size: String::new(),
            facts: Vec::new(),
            offline: false,
        },
        newer: None,
    }
}

impl ViewerSource for Plays {
    fn open(&mut self, file_id: &str) -> Result<OpenFile, OpenError> {
        Ok(file(file_id))
    }
    fn poll(&mut self, _file_id: &str) -> Option<Body> {
        None
    }
    fn cancel(&mut self, _file_id: &str) {}
    fn retry(&mut self, file_id: &str) -> Result<OpenFile, OpenError> {
        self.open(file_id)
    }
    fn can_play(&self) -> bool {
        true
    }
    fn play(&mut self, file_id: &str, on: bool) -> bool {
        self.calls.borrow_mut().push(format!("play {file_id} {on}"));
        true
    }
    fn player(&self) -> Option<PlayerHandle> {
        Some(self.player.clone())
    }
}

struct NoOpener;
impl MediaOpener for NoOpener {
    fn open(&self, _file_id: &str) -> Result<Box<dyn MediaBytes>, Refusal> {
        Err(Refusal::CouldNotPlay)
    }
}

struct NoLinks;
impl LinkFactory for NoLinks {
    fn start(&self) -> Result<Box<dyn Link>, Refusal> {
        Err(Refusal::NotInstalled)
    }
}

struct NoAudio;
impl AudioFactory for NoAudio {
    fn start(&self, _rate_hint: u32) -> Result<Box<dyn AudioSink>, AudioError> {
        Err(AudioError::NoDevice)
    }
}

/// A player that holds nothing yet.
fn a_player() -> PlayerHandle {
    PlayerHandle::new(
        Arc::new(NoOpener),
        Arc::new(NoLinks),
        Arc::new(NoAudio),
        None,
        VideoSurface::new(),
    )
}

/// A player that holds `file_id`'s refusal ("This file could not be played."): the state of that file only.
fn holding_refused(file_id: &str, kind: KindHint) -> PlayerHandle {
    let player = a_player();
    let item = MediaItem {
        file_id: file_id.into(),
        kind,
        length_ms: 0,
        press: Press {
            session: SessionGeneration::for_test(1),
            account: Some("a@example.eu".into()),
        },
    };
    player.open(item, Vec::new()).expect("the player starts its worker");
    let began = Instant::now();
    while !player.state().is_some_and(|state| state.refused.is_some()) {
        assert!(
            began.elapsed() < Duration::from_secs(5),
            "the refusal lands in the player"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    player
}

/// The shell on the file `id`, opened from the source that plays with `player`.
fn opened(id: &str, player: PlayerHandle) -> (Runtime<State>, Calls) {
    let calls = Calls::default();
    let mut state = init();
    let source = Plays {
        calls: calls.clone(),
        player,
    };
    assert!(open_from(&mut state, Box::new(source), id), "the file opens");
    let env = Env {
        system_dark: false,
        scale: 2.0,
    };
    let mut rt = Runtime::new(state, view, (1278.0, 798.0), env);
    rt.on_key = Some(crate::shell::host::on_key);
    rt.frame(0.0);
    rt.settle_all();
    rt.frame(0.0);
    (rt, calls)
}

fn has(rt: &Runtime<State>, key: &str) -> bool {
    rt.elements()
        .iter()
        .any(|e| e.key.as_deref() == Some(key) && !e.exiting)
}

fn words(rt: &Runtime<State>, key: &str) -> String {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
        .unwrap_or_else(|| panic!("no element keyed {key}"))
        .text
        .unwrap_or_default()
}

fn space(rt: &mut Runtime<State>) {
    rt.key(KeyEvent {
        key: Key::Text(" ".into()),
        mods: Mods::default(),
    });
    rt.frame(0.0);
    rt.frame(0.0);
    rt.settle_all();
}

fn click(rt: &mut Runtime<State>, key: &str) {
    rt.settle_all();
    rt.frame(0.0);
    let r = rt
        .elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
        .unwrap_or_else(|| panic!("no element keyed {key}"))
        .rect;
    let (x, y) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(0.0);
    rt.frame(0.0);
    rt.settle_all();
}

/// P1-2: Space on a Files video the player holds nothing of asks the source to play it (the source opens it for the
/// press's session); before, Space only toggled what the player already held, so nothing ever started.
#[test]
fn mfx_space_on_a_files_video_opens_it_through_the_source() {
    let (mut rt, calls) = opened("v1", a_player());
    space(&mut rt);
    assert_eq!(
        *calls.borrow(),
        ["play v1 true"],
        "the first Space plays the video through the source"
    );
}

/// P1-2: the bar's Play button does what Space does.
#[test]
fn mfx_the_video_bars_play_opens_it_through_the_source() {
    let (mut rt, calls) = opened("v1", a_player());
    assert!(has(&rt, "video-play-icon"), "the bar is drawn");
    click(&mut rt, "video-play-icon");
    assert_eq!(
        *calls.borrow(),
        ["play v1 true"],
        "the button plays the video through the source"
    );
}

/// P1-2: while the player holds another file (here its refusal), a Files video's view draws none of it: no refusal
/// words, its own times, and its Play opens this video.
#[test]
fn mfx_a_files_video_shows_none_of_another_files_state() {
    let (mut rt, calls) = opened("v1", holding_refused("song-a", KindHint::Audio));
    assert!(!has(&rt, "refused-text"), "another file's refusal is not this video's");
    assert!(has(&rt, "video-stage"), "the video's own stage is drawn");
    assert_eq!(words(&rt, "video-elapsed"), "0:00");
    space(&mut rt);
    assert_eq!(
        *calls.borrow(),
        ["play v1 true"],
        "Play opens this video, never resumes the other file"
    );
}

/// P2-3: a recording's view draws none of another recording's state (its refusal here; its title, times and picture
/// alike): the file's own title.
#[test]
fn mfx_a_recordings_view_shows_none_of_another_files_state() {
    let (rt, _calls) = opened("b1", holding_refused("song-a", KindHint::Audio));
    assert!(
        !has(&rt, "refused-text"),
        "another recording's refusal is not this one's"
    );
    assert_eq!(words(&rt, "audio-title"), "Invented b1");
}

/// P2-3: the round button and Space are the same press: both ask the source to play THIS recording.
#[test]
fn mfx_a_recordings_button_and_space_are_the_same_press() {
    let (mut rt, calls) = opened("b1", holding_refused("song-a", KindHint::Audio));
    click(&mut rt, "pause");
    assert_eq!(*calls.borrow(), ["play b1 true"], "the button plays this recording");
    space(&mut rt);
    assert_eq!(
        *calls.borrow(),
        ["play b1 true", "play b1 true"],
        "Space asks the same as the button (the player still holds nothing of b1)"
    );
}

/// P2-4: a video whose own play was refused shows the refusal, and Space plays it again (the source opens it anew);
/// before, the press went to a player with no run and nothing happened.
#[test]
fn mfx_space_on_a_refused_video_plays_it_again() {
    let (mut rt, calls) = opened("v1", holding_refused("v1", KindHint::Video));
    assert!(has(&rt, "refused-text"), "the video's own refusal is shown");
    space(&mut rt);
    assert_eq!(
        *calls.borrow(),
        ["play v1 true"],
        "Space after a refusal opens the video again"
    );
}

// ---- fx-player-readers: the details panel reads only its own file's player state ----

/// A helper that answers `Open` with a video whose decoding is reduced on this PC, then nothing.
struct ReducedLink {
    answered: bool,
}

impl Link for ReducedLink {
    fn send(&mut self, _msg: &beebeeb_media_protocol::ToHelper) -> std::io::Result<()> {
        Ok(())
    }
    fn recv(
        &mut self,
        _timeout: Duration,
    ) -> Result<beebeeb_media_protocol::FromHelper, crate::media::transport::LinkError> {
        if !self.answered {
            self.answered = true;
            return Ok(beebeeb_media_protocol::FromHelper::Opened(
                beebeeb_media_protocol::Opened {
                    duration_ms: 60_000,
                    video: Some(beebeeb_media_protocol::VideoInfo {
                        w: 640,
                        h: 360,
                        fps_milli: 30_000,
                        codec: beebeeb_media_protocol::Codec::H264,
                    }),
                    audio: None,
                    reduced_quality: true,
                    no_sound: None,
                },
            ));
        }
        std::thread::sleep(Duration::from_millis(20));
        Err(crate::media::transport::LinkError::Timeout)
    }
    fn kill(&mut self) {}
}

struct ReducedLinks;
impl LinkFactory for ReducedLinks {
    fn start(&self) -> Result<Box<dyn Link>, Refusal> {
        Ok(Box::new(ReducedLink { answered: false }))
    }
}

struct OneByte;
impl MediaBytes for OneByte {
    fn len(&self) -> u64 {
        1
    }
    fn read_at(
        &mut self,
        _offset: u64,
        _len: u32,
        _cancel: &std::sync::atomic::AtomicBool,
    ) -> Result<zeroize::Zeroizing<Vec<u8>>, ()> {
        Ok(zeroize::Zeroizing::new(vec![0]))
    }
}

struct OpensOneByte;
impl MediaOpener for OpensOneByte {
    fn open(&self, _file_id: &str) -> Result<Box<dyn MediaBytes>, Refusal> {
        Ok(Box::new(OneByte))
    }
}

/// A player whose `file_id` is open and flagged "reduced quality on this PC".
fn holding_reduced(file_id: &str) -> PlayerHandle {
    let player = PlayerHandle::new(
        Arc::new(OpensOneByte),
        Arc::new(ReducedLinks),
        Arc::new(NoAudio),
        None,
        VideoSurface::new(),
    );
    let item = MediaItem {
        file_id: file_id.into(),
        kind: KindHint::Video,
        length_ms: 0,
        press: Press {
            session: SessionGeneration::for_test(1),
            account: Some("a@example.eu".into()),
        },
    };
    player.open(item, Vec::new()).expect("the player starts its worker");
    let began = Instant::now();
    while !player.state().is_some_and(|state| state.reduced_quality) {
        assert!(began.elapsed() < Duration::from_secs(5), "the flag lands in the player");
        std::thread::sleep(Duration::from_millis(5));
    }
    player
}

fn with_details(player: PlayerHandle) -> Runtime<State> {
    let (mut rt, _calls) = opened("v1", player);
    rt.state.viewer.ui.as_mut().expect("a file is open").toggle_details();
    rt.frame(0.0);
    rt.settle_all();
    rt.frame(0.0);
    rt
}

/// The details panel says "Reduced quality on this PC" for its own file only: another file's flag is not this one's.
#[test]
fn fxr_details_show_the_reduced_quality_of_their_own_file_only() {
    let own = with_details(holding_reduced("v1"));
    assert!(has(&own, "detail-reduced"), "this file's own flag is said");
    let other = with_details(holding_reduced("song-a"));
    assert!(has(&other, "detail-pane"), "the panel is drawn");
    assert!(
        !has(&other, "detail-reduced"),
        "another file's reduced-quality flag is not this video's"
    );
}
