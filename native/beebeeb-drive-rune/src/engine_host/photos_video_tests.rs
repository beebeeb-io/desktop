//! A video in Photos plays in the shell's player (M8): which press opens it, which never does, and what a late open
//! under another session does. The player here is a handle whose seams are never used: `play_with` is handed the
//! open as a closure that records what it was asked, so no helper starts and nothing is decoded. Every name is invented.

use std::cell::RefCell;
use std::sync::Arc;

use beebeeb_media_protocol::KindHint;

use super::*;
use crate::media::StopReason;
use crate::media::audio_out::AudioError;
use crate::media::player::{
    AudioFactory, AudioSink, Link, LinkFactory, MediaBytes, MediaItem, MediaOpener, PlayerHandle, Press, Refusal,
};
use crate::media::video::VideoSurface;

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

/// A player whose seams answer nothing: only what `play_with` asks of it through the closure is seen.
fn a_player() -> PlayerHandle {
    PlayerHandle::new(
        Arc::new(NoOpener),
        Arc::new(NoLinks),
        Arc::new(NoAudio),
        None,
        VideoSurface::new(),
    )
}

/// The number of a file in the library that is here.
fn number(source: &Source, file_id: &str) -> PhotoId {
    *source.numbers.get(file_id).expect("the file has a number")
}

#[test]
fn video_in_photos_opens_the_shell_player() {
    let fake = fake("video-opens");
    let (_engine, mut source, _store, _dir) = signed_in(&fake);
    open(&mut source);
    let tram = number(&source, TRAM);

    let asked: RefCell<Vec<MediaItem>> = RefCell::new(Vec::new());
    let player = a_player();
    let got = source.play_with(
        tram,
        player,
        true,
        |_, item| {
            asked.borrow_mut().push(item);
            Ok(())
        },
        || {},
    );

    assert!(got.is_some(), "the shell's player is handed to the stage");
    let asked = asked.into_inner();
    assert_eq!(asked.len(), 1, "one open for one press");
    assert_eq!(asked[0].file_id, TRAM);
    assert_eq!(asked[0].kind, KindHint::Video);
    // The length is the thumbnail's length in seconds, in milliseconds (none is held yet in this test: 0).
    assert_eq!(asked[0].length_ms % 1000, 0);
}

#[test]
fn a_photo_never_opens_the_player() {
    let fake = fake("video-photo");
    let (_engine, mut source, _store, _dir) = signed_in(&fake);
    open(&mut source);
    let harbour = number(&source, HARBOUR);

    let got = source.play_with(
        harbour,
        a_player(),
        true,
        |_, _| panic!("a photo was opened in the player"),
        || {},
    );
    assert!(got.is_none());
}

#[test]
fn video_without_playback_returns_none() {
    let fake = fake("video-no-playback");
    let (_engine, mut source, _store, _dir) = signed_in(&fake);
    open(&mut source);
    let tram = number(&source, TRAM);

    // No playback on this PC (not Windows, or the helper is not installed): the stage keeps its poster.
    let got = source.play_with(
        tram,
        a_player(),
        false,
        |_, _| panic!("the player was opened without playback"),
        || {},
    );
    assert!(got.is_none());
    // And with playback but a player that refuses: still nothing is handed to the stage.
    let refused = source.play_with(tram, a_player(), true, |_, _| Err(Refusal::CouldNotPlay), || {});
    assert!(refused.is_none());
}

#[test]
fn late_video_open_under_another_session_dropped() {
    let fake = fake("video-late");
    let (engine, mut source, _store, _dir) = signed_in(&fake);
    open(&mut source);
    let tram = number(&source, TRAM);

    // The open is accepted, and the second account signs in before it is taken in.
    let got = source.play_with(
        tram,
        a_player(),
        true,
        |_, _| Ok(()),
        || {
            engine.sign_out_keeping_local_data().expect("signed out");
            sign_in(&engine, &fake.second, &fake.second_phrase);
        },
    );
    assert!(got.is_none(), "nothing is handed to the stage of the second account");
}

/// Waits up to 10 s for the player to settle on a refusal (the open runs on the player's worker), and answers it.
fn refusal_of_player(player: &PlayerHandle) -> Option<Refusal> {
    let began = std::time::Instant::now();
    loop {
        if let Some(refused) = player.state().and_then(|state| state.refused) {
            return Some(refused);
        }
        assert!(
            began.elapsed() < std::time::Duration::from_secs(10),
            "the player settles on an answer"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

/// P1-3: a video pressed in Photos is opened by the REAL opener for the press's session, though no other press was
/// ever made in this process: it is never refused as "Sign in to play this file.". (Its helper is not installed here,
/// so the run ends at the helper's start, after the bytes were opened.) Before, the opener read a process-wide stamp
/// that only a Files press wrote, and refused.
#[test]
fn mfx_photos_video_opens_for_the_press_not_a_stamp() {
    let fake = fake("mfx-video-press");
    let (engine, mut source, _store, _dir) = signed_in(&fake);
    open(&mut source);
    let tram = number(&source, TRAM);
    let player = PlayerHandle::new(
        Arc::new(crate::engine_host::media::EngineMediaOpener::new(engine.clone())),
        Arc::new(NoLinks),
        Arc::new(NoAudio),
        None,
        VideoSurface::new(),
    );
    let got = source.play_with(
        tram,
        player.clone(),
        true,
        |player, item| player.open(item, Vec::new()),
        || {},
    );
    assert!(got.is_some(), "the shell's player is handed to the stage");
    let refused = refusal_of_player(&player);
    player.stop(StopReason::Closed);
    assert_ne!(
        refused,
        Some(Refusal::NotSignedIn),
        "a signed-in person's video is opened for their press, never refused as not signed in"
    );
}

/// An opener that keeps the press each open was made for.
struct PressRecorder(std::sync::Mutex<Vec<Press>>);
impl MediaOpener for PressRecorder {
    fn open(&self, _file_id: &str) -> Result<Box<dyn MediaBytes>, Refusal> {
        panic!("an item was opened without its press")
    }
    fn open_for(&self, press: &Press, _file_id: &str) -> Result<Box<dyn MediaBytes>, Refusal> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(press.clone());
        Err(Refusal::CouldNotPlay)
    }
}

/// P1-3: the press made in Photos travels WITH the item through the player into the opener: the session and the
/// account the press was made under.
#[test]
fn mfx_photos_video_carries_its_press_into_the_opener() {
    let fake = fake("mfx-video-carries");
    let (engine, mut source, _store, _dir) = signed_in(&fake);
    open(&mut source);
    let tram = number(&source, TRAM);
    let recorder = Arc::new(PressRecorder(std::sync::Mutex::new(Vec::new())));
    let player = PlayerHandle::new(
        recorder.clone(),
        Arc::new(NoLinks),
        Arc::new(NoAudio),
        None,
        VideoSurface::new(),
    );
    let pressed = crate::engine_host::stamp::stamp(&engine);
    source.play_with(
        tram,
        player.clone(),
        true,
        |player, item| player.open(item, Vec::new()),
        || {},
    );
    assert_eq!(refusal_of_player(&player), Some(Refusal::CouldNotPlay));
    let presses = recorder
        .0
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    assert_eq!(presses.len(), 1, "one open, through the press's opener");
    assert_eq!(presses[0].session, pressed.0, "the press's session");
    assert_eq!(presses[0].account, pressed.1, "the press's account");
    assert!(presses[0].account.is_some(), "a signed-in press names its account");
}

/// P1-3: the engine's opener opens for a press of the account that is signed in, and refuses a press without one, a
/// press of another account (a stale generation can equal this account's: each account counts from its own start),
/// and an open with no press at all.
#[test]
fn mfx_engine_opener_opens_only_for_the_press_account() {
    let fake = fake("mfx-video-account");
    let (engine, _source, _store, _dir) = signed_in(&fake);
    let opener = crate::engine_host::media::EngineMediaOpener::new(engine.clone());
    let (session, account) = crate::engine_host::stamp::stamp(&engine);
    assert!(account.is_some(), "the test signs an account in");
    assert_eq!(
        opener.open(TRAM).err(),
        Some(Refusal::NotSignedIn),
        "no press: nothing is opened"
    );
    let others = Press {
        session,
        account: Some("someone-else@example.eu".into()),
    };
    assert_eq!(
        opener.open_for(&others, TRAM).err(),
        Some(Refusal::NotSignedIn),
        "another account's press opens nothing"
    );
    let nobodys = Press { session, account: None };
    assert_eq!(
        opener.open_for(&nobodys, TRAM).err(),
        Some(Refusal::NotSignedIn),
        "a press without an account opens nothing"
    );
    let own = Press { session, account };
    assert_ne!(
        opener.open_for(&own, TRAM).err(),
        Some(Refusal::NotSignedIn),
        "the signed-in account's press opens its file"
    );
}
