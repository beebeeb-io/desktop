//! A video in Photos plays in the shell's player (M8): which press opens it, which never does, and what a late open
//! under another session does. The player here is a handle whose seams are never used: `play_with` is handed the
//! open as a closure that records what it was asked, so no helper starts and nothing is decoded. Every name is invented.

use std::cell::RefCell;
use std::sync::Arc;

use beebeeb_media_protocol::KindHint;

use super::*;
use crate::media::audio_out::AudioError;
use crate::media::player::{
    AudioFactory, AudioSink, Link, LinkFactory, MediaBytes, MediaItem, MediaOpener, PlayerHandle, Refusal,
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
