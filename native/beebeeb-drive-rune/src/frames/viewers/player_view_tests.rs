//! Tests of the player's views (slice M6): the recording and the video draw from the player's state when the source
//! hands one, and exactly as before when it hands none. The views are `audio.rs` and `video.rs`.
//!
//! Written without a build (the media plan's code-only batch); first seen at the media checkpoint.

use beebeeb_media_protocol::Codec;
use rune::{Env, Runtime};

use super::set_scene;
use super::video::{Times, ask_size, letterbox, refusal_words, space_toggles};
use crate::media::player::{PlayState, Refusal};
use crate::{State, init, view};

fn scene(frame: &str) -> Runtime<State> {
    let mut state = init();
    set_scene(&mut state, frame);
    let env = Env {
        system_dark: false,
        scale: 2.0,
    };
    let mut rt = Runtime::new(state, view, (1278.0, 798.0), env);
    rt.frame(0.0);
    rt
}

fn keys(rt: &Runtime<State>) -> Vec<String> {
    rt.elements()
        .into_iter()
        .filter(|e| !e.exiting)
        .filter_map(|e| e.key)
        .collect()
}

fn state(at_ms: u64, length_ms: u64) -> PlayState {
    PlayState {
        file_id: "f".into(),
        at_ms,
        length_ms,
        playing: true,
        buffering: false,
        title: "Harbour sessions".into(),
        artist: "Anna".into(),
        reduced_quality: false,
        refused: None,
    }
}

/// 1. The board's DW-1503: a source that hands no player draws today's elements, none of the player's.
#[test]
fn audio_view_without_player_draws_the_fixture() {
    let rt = scene("DW-1503");
    let keys = keys(&rt);
    for key in [
        "artwork",
        "audio-title",
        "scrub-track",
        "scrub-fill",
        "time-elapsed",
        "time-remain",
        "prev",
        "pause",
        "next",
        "audio-caption",
        "queue",
        "queue-body",
        "track-0",
    ] {
        assert!(keys.iter().any(|k| k == key), "the fixture draws {key}");
    }
    for key in ["audio-reduced", "refused-words", "refused-open", "video-stage"] {
        assert!(!keys.iter().any(|k| k == key), "{key} belongs to the player's views");
    }
    let track = rt
        .elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some("scrub-track") && !e.exiting)
        .expect("the scrubber is drawn");
    assert_eq!(track.role, "progressbar", "the fixture's scrubber stays a progress bar");
}

/// 2. The player's state in the bar's words: 1:12 elapsed, 2:50 left, "1:12 of 4:02" spoken.
#[test]
fn audio_view_with_player_shows_its_state() {
    let s = state(72_000, 242_000);
    let times = Times::of(&s);
    assert_eq!(times.elapsed(), "1:12");
    assert_eq!(times.remaining(), "\u{2212}2:50");
    assert_eq!(times.spoken(), "1:12 of 4:02");
    assert!(times.playing);
    assert!((times.part() - 72.0 / 242.0).abs() < 1e-6);
    // Before the length is known the scrubber is empty, never divided by zero.
    assert_eq!(Times::of(&state(5_000, 0)).part(), 0.0);
    // Past the end clamps: nothing negative is said.
    assert_eq!(Times::of(&state(300_000, 242_000)).remaining(), "\u{2212}0:00");
}

/// 3. Space is the player's unless a text field has the keys.
#[test]
fn space_does_not_toggle_in_a_text_field() {
    assert!(space_toggles(false));
    assert!(!space_toggles(true));
}

/// 4. A 1920 x 1080 frame in a 900 x 700 stage is drawn 900 x 506 (the toolkit centres it).
#[test]
fn video_view_letterboxes_to_fit() {
    assert_eq!(letterbox((1920, 1080), (900.0, 700.0)), (900.0, 506.0));
    // A tall frame fits by height.
    assert_eq!(letterbox((1080, 1920), (900.0, 700.0)), (394.0, 700.0));
    // No frame, no stage: nothing.
    assert_eq!(letterbox((0, 0), (900.0, 700.0)), (0.0, 0.0));
    assert_eq!(letterbox((1920, 1080), (0.0, 700.0)), (0.0, 0.0));
    // The player is asked for whole, non-zero pixels.
    let (w, h) = ask_size(900.0, 700.0);
    assert!(w >= 900 && h >= 700);
    assert_eq!(ask_size(0.0, 0.0), (1, 1));
}

/// 5. A refusal names the FORMAT (never the file), in the board's words.
#[test]
fn refusal_words_name_the_format() {
    assert_eq!(
        refusal_words(&Refusal::NoDecoder("ProRes".into())),
        "This PC can't play ProRes files here."
    );
    assert_eq!(
        refusal_words(&Refusal::NoSystemDecoder(Codec::Hevc)),
        "This PC can't play HEVC video without Microsoft's HEVC extension."
    );
    assert_eq!(refusal_words(&Refusal::CouldNotPlay), "This file could not be played.");
    assert_eq!(
        refusal_words(&Refusal::NoSandbox),
        "Playback isn't available on this PC."
    );
    assert_eq!(
        refusal_words(&Refusal::NotInstalled),
        "Playback isn't available on this PC."
    );
}
