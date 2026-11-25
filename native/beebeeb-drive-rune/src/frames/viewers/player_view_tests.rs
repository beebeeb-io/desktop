//! Tests of the player's views (slice M6): the recording and the video draw from the player's state when the source
//! hands one, and exactly as before when it hands none. The views are `audio.rs` and `video.rs`.
//!
//! Written without a build (the media plan's code-only batch); first seen at the media checkpoint.

use beebeeb_media_protocol::Codec;
use rune::{Env, Runtime};

use super::super::set_scene;
use super::{
    ask_size, letterbox, no_sound_line, no_sound_words, refusal_words, refusal_words_with, seek_target, space_toggles,
    toggle_for, Times, Toggle,
};
use crate::media::player::{NoSound, PlayState, Refusal};
use crate::theme::tokens::theme;
use crate::{init, view, State};

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
        net: Default::default(),
        title: "Harbour sessions".into(),
        artist: "Anna".into(),
        reduced_quality: false,
        no_sound: None,
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
    for key in ["audio-reduced", "refused-text", "refused-open", "video-stage"] {
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

/// 5. A refusal names the FORMAT (never the file) and the cause; the table's own tests are in `refusal_text`.
#[test]
fn refusal_words_name_the_format() {
    assert_eq!(
        refusal_words(&Refusal::NoDecoder("ProRes".into())),
        "This file uses ProRes, which Beebeeb doesn't support yet. Open it in another app."
    );
    assert!(refusal_words_with(true, &Refusal::NoSystemDecoder(Codec::Hevc)).contains("HEVC Video Extensions"));
    assert!(refusal_words_with(true, &Refusal::NoSystemDecoder(Codec::Aac)).contains("AAC"));
    assert!(refusal_words(&Refusal::NoSystemMedia(Codec::Aac)).contains("no media codecs"));
    assert!(refusal_words(&Refusal::CouldNotPlay).contains("damaged"));
    assert_ne!(
        refusal_words(&Refusal::NoSandbox),
        refusal_words(&Refusal::NotInstalled)
    );
}

/// A window that draws only the no-sound line, so the test needs no player.
fn no_sound_view(_: &State, _: &mut rune::Cx<State>) -> rune::El {
    rune::col().key("window").child(no_sound_line(
        "video",
        NoSound::Codec(Codec::Aac),
        rune::Color::WHITE,
        &theme(false),
    ))
}

fn no_sound_scene() -> Runtime<State> {
    let env = Env {
        system_dark: false,
        scale: 2.0,
    };
    let mut rt = Runtime::new(init(), no_sound_view, (600.0, 200.0), env);
    rt.frame(0.0);
    rt
}

/// 6. The no-sound line names the missing track's FORMAT (never the file), and the device's own words.
#[test]
fn no_sound_line_names_the_missing_track() {
    assert_eq!(
        no_sound_words(NoSound::Codec(Codec::Aac)),
        concat!(
            "No sound: ",
            beebeeb_engine::device_word!(),
            " has no decoder for AAC audio"
        )
    );
    assert_eq!(
        no_sound_words(NoSound::Device),
        concat!("Sound isn't available on ", beebeeb_engine::device_word!())
    );
    let rt = no_sound_scene();
    let words = rt
        .elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some("video-no-sound-words") && !e.exiting)
        .expect("the line is drawn");
    assert_eq!(
        words.text.as_deref(),
        Some(concat!(
            "No sound: ",
            beebeeb_engine::device_word!(),
            " has no decoder for AAC audio"
        ))
    );
}

/// 7. The line offers "Open in another app\u{2026}" one press away.
#[test]
fn the_no_sound_line_offers_another_app() {
    let rt = no_sound_scene();
    let keys = keys(&rt);
    assert!(keys.iter().any(|k| k == "video-no-sound"), "the line is drawn");
    assert!(
        keys.iter().any(|k| k == "no-sound-open"),
        "Open in another app is one press away"
    );
}

/// 8. D-MED-7: "Reduced quality on this PC" is the details panel's line, not the caption's.
#[test]
fn reduced_quality_is_said_in_the_details() {
    fn panel_view(s: &State, _: &mut rune::Cx<State>) -> rune::El {
        let ui = s.viewer.ui.as_ref().expect("a file is open");
        super::super::details::panel_with(&ui.file, false, true, &theme(false))
    }
    let mut state = init();
    set_scene(&mut state, "DW-1503");
    let env = Env {
        system_dark: false,
        scale: 2.0,
    };
    let mut rt = Runtime::new(state, panel_view, (600.0, 800.0), env);
    rt.frame(0.0);
    let line = rt
        .elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some("detail-reduced") && !e.exiting)
        .expect("the details panel carries the line");
    assert_eq!(
        line.text.as_deref(),
        Some(concat!("Reduced quality on ", beebeeb_engine::device_word!()))
    );
}

/// A source with one video and a player that counts how often the view asked for it.
struct OneVideo {
    asked: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    /// Set when the source is dropped: what the engine's source does to stop the player (`closing_a_video_stops_it`).
    dropped: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
}

impl Drop for OneVideo {
    fn drop(&mut self) {
        if let Some(flag) = &self.dropped {
            flag.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }
}

impl super::super::source::ViewerSource for OneVideo {
    fn open(&mut self, file_id: &str) -> Result<super::super::source::OpenFile, super::super::source::OpenError> {
        Ok(super::super::source::OpenFile {
            id: file_id.to_string(),
            path: Vec::new(),
            name: "Clip.mp4".into(),
            facts: "Video".into(),
            version: 1,
            body: super::super::source::Body::Video(super::super::kinds::VideoDoc { length: 0 }),
            details: super::super::source::Details {
                kind: "Video".into(),
                size: String::new(),
                facts: Vec::new(),
                offline: false,
            },
            newer: None,
        })
    }
    fn poll(&mut self, _file_id: &str) -> Option<super::super::source::Body> {
        None
    }
    fn cancel(&mut self, _file_id: &str) {}
    fn retry(&mut self, file_id: &str) -> Result<super::super::source::OpenFile, super::super::source::OpenError> {
        self.open(file_id)
    }
    fn player(&self) -> Option<crate::media::player::PlayerHandle> {
        self.asked.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        None
    }
    fn can_play(&self) -> bool {
        true
    }
    /// Space reaches the source's Play (`Ui::toggle_play`), which opens the video for the press's session.
    fn play(&mut self, _file_id: &str, _on: bool) -> bool {
        self.asked.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        true
    }
}

/// 9. Space on a video opened from Files is the viewer's (it reaches the source's Play, which opens the video for the
///    press's session), and another key is not.
#[test]
fn space_on_a_files_video_toggles_its_player() {
    use std::sync::atomic::Ordering;
    let asked = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut state = init();
    let ui = super::super::Ui::open(
        Box::new(OneVideo {
            asked: asked.clone(),
            dropped: None,
        }),
        "v",
    )
    .expect("the video opens");
    state.viewer.ui = Some(ui);
    let before = asked.load(Ordering::SeqCst);
    let key = |text: &str| rune::KeyEvent {
        key: rune::Key::Text(text.into()),
        mods: rune::Mods::default(),
    };
    assert!(
        super::super::media::on_key(&mut state, &key(" ")),
        "Space is the video's"
    );
    assert!(
        asked.load(Ordering::SeqCst) > before,
        "Space went to the video's source"
    );
    assert!(
        !super::super::media::on_key(&mut state, &key("x")),
        "another key is not"
    );
}

/// 10. Space is play when paused and pause when playing, whichever way the bar's button or the key asks; nothing
///     before the player has a state (a fake source's `None` never reaches `play` or `pause`).
#[test]
fn space_on_a_video_is_play_then_pause() {
    let mut s = state(1_000, 10_000);
    s.playing = true;
    assert_eq!(toggle_for(Some(&s)), Some(Toggle::Pause), "a playing video pauses");
    s.playing = false;
    assert_eq!(toggle_for(Some(&s)), Some(Toggle::Play), "a paused video plays");
    assert_eq!(toggle_for(None), None, "no state, no command");
}

/// 11. Esc on a video opened from Files: the shell's Esc leaves the route, the viewer sync closes the file, and the
///     source (which owns the player's stop) is dropped with it.
#[test]
fn esc_on_a_files_video_closes_it_and_stops_the_player() {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    let dropped = std::sync::Arc::new(AtomicBool::new(false));
    let source = OneVideo {
        asked: std::sync::Arc::new(AtomicUsize::new(0)),
        dropped: Some(dropped.clone()),
    };
    let mut state = init();
    state.viewer.ui = Some(super::super::Ui::open(Box::new(source), "v").expect("the video opens"));
    assert!(!dropped.load(Ordering::SeqCst), "it is open and playing");
    // The route is not on the file (the shell's Esc ran): the viewer closes it.
    super::super::sync(&mut state);
    assert!(state.viewer.ui.is_none(), "the file is closed");
    assert!(dropped.load(Ordering::SeqCst), "closing it stopped its player");
}

/// 12. A press or drag on the scrubber lands proportionally, clamped; no length or no width: nowhere.
#[test]
fn the_scrubber_seeks_to_where_it_is_pressed() {
    assert_eq!(seek_target(200_000, 50.0, 200.0), Some(50_000));
    assert_eq!(
        seek_target(200_000, -10.0, 200.0),
        Some(0),
        "left of the track is the start"
    );
    assert_eq!(
        seek_target(200_000, 500.0, 200.0),
        Some(200_000),
        "right of it is the end"
    );
    assert_eq!(seek_target(0, 50.0, 200.0), None, "the length is not known yet");
    assert_eq!(seek_target(200_000, 50.0, 0.0), None, "no width");
    assert_eq!(seek_target(200_000, f32::NAN, 200.0), None);
}

/// 13. Row 29: while the system decoders are not used, no refusal tells a person to install Microsoft's extension.
#[test]
fn hevc_words_do_not_ask_for_an_extension_while_the_switch_is_off() {
    let off = refusal_words_with(false, &Refusal::NoSystemDecoder(Codec::Hevc));
    assert!(off.contains("HEVC (H.265)") && off.contains("can't play yet"), "{off}");
    assert!(!off.contains("Microsoft") && !off.contains("extension"));
    let aac = refusal_words_with(false, &Refusal::NoSystemDecoder(Codec::Aac));
    assert!(
        !aac.contains(concat!(beebeeb_engine::device_word_cap!(), " can't")),
        "the cause is Beebeeb's, not the PC's: {aac}"
    );
    // With the switch on, the PC's own answer: what to install.
    assert!(refusal_words_with(true, &Refusal::NoSystemDecoder(Codec::Hevc)).contains("HEVC Video Extensions"));
    // The app's own words follow the protocol's constant.
    assert_eq!(
        refusal_words(&Refusal::NoSystemDecoder(Codec::Hevc)),
        refusal_words_with(
            beebeeb_media_protocol::MEDIA_SYSTEM_DECODER_ON,
            &Refusal::NoSystemDecoder(Codec::Hevc)
        )
    );
}

/// 14. Row 18: the details panel's flag is the player's `reduced_quality`, and false without a player.
#[test]
fn the_details_flag_follows_the_players_state() {
    let mut s = state(0, 1_000);
    s.reduced_quality = true;
    assert!(super::super::details::is_reduced(Some(&s)));
    s.reduced_quality = false;
    assert!(!super::super::details::is_reduced(Some(&s)));
    assert!(!super::super::details::is_reduced(None));
}

#[test]
fn a_too_old_windows_says_so_in_the_media_view() {
    assert_eq!(
        refusal_words(&Refusal::NeedsWindowsUpdate),
        "This file can't be shown until Windows is updated."
    );
}

/// ux-02: the helper's `NoDecoder` names no format; the sentence had a hole ("can't play  files here").
#[test]
fn media_no_decoder_without_a_name_says_this_kind_of_file() {
    let words = refusal_words(&Refusal::NoDecoder(String::new()));
    assert_eq!(
        words,
        "Beebeeb doesn't support this kind of file yet. Open it in another app."
    );
    assert!(!words.contains("  "), "{words}");
}

/// ux-02 (the owner's ruling of 2026-11-20): as shipped, a PC without the system decoder for a format says so in
/// plain words, and the button under them is "Open in default app". Every other refusal keeps its own button, and
/// so does every refusal while the switch is held off (the PC was never asked).
#[test]
fn media_a_pc_without_the_system_decoder_says_so_and_offers_open_in_default_app() {
    use super::{refusal_action, refusal_action_with, OPEN_DEFAULT, OPEN_ELSEWHERE};
    assert_eq!(OPEN_DEFAULT, "Open in default app");
    let hevc = Refusal::NoSystemDecoder(Codec::Hevc);
    let words = refusal_words(&hevc);
    assert!(
        words.contains("HEVC (H.265)") && words.contains("doesn't have the decoder"),
        "{words}"
    );
    assert!(words.contains("Microsoft Store"), "{words}");
    assert!(words.contains("default app"), "{words}");
    assert!(
        !words.contains("yet"),
        "the PC has no decoder; it is not Beebeeb that cannot yet: {words}"
    );
    for why in [
        hevc.clone(),
        Refusal::NoSystemDecoder(Codec::H264),
        Refusal::NoSystemDecoder(Codec::Aac),
        Refusal::NoSystemMedia(Codec::Hevc),
    ] {
        assert_eq!(refusal_action(&why), OPEN_DEFAULT, "{why:?}");
        assert!(refusal_words(&why).contains("default app"), "{why:?}");
        assert_eq!(refusal_action_with(false, &why), OPEN_ELSEWHERE, "{why:?}");
    }
    for why in [
        Refusal::NoDecoder("ProRes".into()),
        Refusal::CouldNotPlay,
        Refusal::NotSignedIn,
        Refusal::NotInstalled,
        Refusal::NoSandbox,
    ] {
        assert_eq!(refusal_action(&why), OPEN_ELSEWHERE, "{why:?}");
    }
    // The button on screen carries the words.
    fn refused_view(_: &State, _: &mut rune::Cx<State>) -> rune::El {
        rune::col().key("window").child(super::refused_for(
            &Refusal::NoSystemDecoder(Codec::Hevc),
            &theme(false),
        ))
    }
    let env = Env {
        system_dark: false,
        scale: 2.0,
    };
    let mut rt = Runtime::new(init(), refused_view, (900.0, 400.0), env);
    rt.frame(0.0);
    let texts: Vec<String> = rt
        .elements()
        .into_iter()
        .filter(|e| !e.exiting)
        .filter_map(|e| e.text)
        .collect();
    assert!(texts.iter().any(|text| text == OPEN_DEFAULT), "the button: {texts:?}");
    assert!(texts.contains(&words), "the words: {texts:?}");
    assert!(
        !texts.iter().any(|text| text.contains("another app")),
        "one button, the default app's: {texts:?}"
    );
}

/// Up / Down, M and F on a recording or a video (task 159): the volume in 10 % steps (0 to 100 %), mute, each said in
/// the toast; F is the window's full-screen key on a video only. The player is real, with seams that answer nothing.
mod media_keys {
    use std::sync::Arc;

    use rune::{Key, KeyEvent, Mods};

    use super::super::super::kinds::{AudioDoc, Track, VideoDoc};
    use super::super::super::source::{Body, Details, OpenError, OpenFile, ViewerSource};
    use crate::media::audio_out::AudioError;
    use crate::media::player::{
        AudioFactory, AudioSink, Link, LinkFactory, MediaBytes, MediaOpener, PlayerHandle, Refusal,
    };
    use crate::media::video::VideoSurface;
    use crate::{init, State};

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

    /// A source that hands the test's player; an id starting with `v` is a video, any other a recording.
    struct Plays(PlayerHandle);
    impl ViewerSource for Plays {
        fn open(&mut self, id: &str) -> Result<OpenFile, OpenError> {
            let body = if id.starts_with('v') {
                Body::Video(VideoDoc { length: 0 })
            } else {
                Body::Audio(AudioDoc {
                    title: "Invented take".into(),
                    artist: String::new(),
                    length: 0,
                    at: 0,
                    playing: false,
                    folder: "Invented demos".into(),
                    queue: vec![Track {
                        id: id.into(),
                        title: "Invented take".into(),
                        length: 0,
                        current: true,
                    }],
                })
            };
            Ok(OpenFile {
                id: id.into(),
                path: Vec::new(),
                name: format!("Invented {id}"),
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
            })
        }
        fn poll(&mut self, _: &str) -> Option<Body> {
            None
        }
        fn cancel(&mut self, _: &str) {}
        fn retry(&mut self, id: &str) -> Result<OpenFile, OpenError> {
            self.open(id)
        }
        fn can_play(&self) -> bool {
            true
        }
        fn player(&self) -> Option<PlayerHandle> {
            Some(self.0.clone())
        }
    }

    fn opened(id: &str) -> (State, PlayerHandle) {
        let player = PlayerHandle::new(
            Arc::new(NoOpener),
            Arc::new(NoLinks),
            Arc::new(NoAudio),
            None,
            VideoSurface::new(),
        );
        let mut state = init();
        state.viewer.ui = Some(super::super::super::Ui::open(Box::new(Plays(player.clone())), id).expect("it opens"));
        (state, player)
    }

    fn key(key: Key) -> KeyEvent {
        KeyEvent {
            key,
            mods: Mods::default(),
        }
    }

    fn toast(state: &State) -> Option<String> {
        state.viewer.ui.as_ref().and_then(|ui| ui.toast.clone())
    }

    fn on_key(state: &mut State, k: Key) -> bool {
        super::super::super::media::on_key(state, &key(k))
    }

    #[test]
    fn up_and_down_step_the_volume_by_ten_and_stop_at_the_ends() {
        for id in ["a1", "v1"] {
            let (mut state, player) = opened(id);
            assert!(on_key(&mut state, Key::Down), "Down is the player's on {id}");
            assert_eq!(super::super::super::media::volume_percent(&player), 90);
            assert_eq!(toast(&state).as_deref(), Some("Volume 90 %"));
            assert!(on_key(&mut state, Key::Up));
            assert!(on_key(&mut state, Key::Up));
            assert_eq!(
                super::super::super::media::volume_percent(&player),
                100,
                "stops at 100 %"
            );
            assert_eq!(toast(&state).as_deref(), Some("Volume 100 %"));
            for _ in 0..12 {
                on_key(&mut state, Key::Down);
            }
            assert_eq!(super::super::super::media::volume_percent(&player), 0, "stops at 0 %");
            assert_eq!(toast(&state).as_deref(), Some("Volume 0 %"));
        }
    }

    #[test]
    fn a_volume_set_off_the_grid_steps_back_onto_it() {
        let (mut state, player) = opened("a1");
        player.set_volume(0.43);
        assert!(on_key(&mut state, Key::Up));
        assert_eq!(super::super::super::media::volume_percent(&player), 50);
    }

    #[test]
    fn m_mutes_and_unmutes_and_says_so_keeping_the_volume() {
        for id in ["a1", "v1"] {
            let (mut state, player) = opened(id);
            on_key(&mut state, Key::Down);
            assert!(on_key(&mut state, Key::Text("m".into())), "M is the player's on {id}");
            assert!(player.muted());
            assert_eq!(toast(&state).as_deref(), Some("Muted"));
            assert_eq!(
                super::super::super::media::volume_percent(&player),
                90,
                "muting keeps the volume"
            );
            assert!(on_key(&mut state, Key::Text("M".into())));
            assert!(!player.muted());
            assert_eq!(toast(&state).as_deref(), Some("Sound on"));
            // A volume step while muted is heard.
            on_key(&mut state, Key::Text("m".into()));
            on_key(&mut state, Key::Up);
            assert!(!player.muted(), "a volume step unmutes");
        }
    }

    #[test]
    fn ctrl_m_and_shift_up_are_not_the_players() {
        let (mut state, player) = opened("a1");
        let ctrl = KeyEvent {
            key: Key::Text("m".into()),
            mods: Mods {
                ctrl: true,
                ..Mods::default()
            },
        };
        assert!(!super::super::super::media::on_key(&mut state, &ctrl));
        let shift_up = KeyEvent {
            key: Key::Up,
            mods: Mods {
                shift: true,
                ..Mods::default()
            },
        };
        assert!(!super::super::super::media::on_key(&mut state, &shift_up));
        assert!(!player.muted());
        assert_eq!(super::super::super::media::volume_percent(&player), 100);
    }

    #[test]
    fn f_is_full_screen_on_a_video_only() {
        let (video, _) = opened("v1");
        let (audio, _) = opened("a1");
        let f = key(Key::Text("f".into()));
        assert!(super::super::super::chrome::full_screen_key(&video, &f));
        assert!(
            !super::super::super::chrome::full_screen_key(&audio, &f),
            "a recording has no picture"
        );
        let ctrl_f = KeyEvent {
            key: Key::Text("f".into()),
            mods: Mods {
                ctrl: true,
                ..Mods::default()
            },
        };
        assert!(!super::super::super::chrome::full_screen_key(&video, &ctrl_f));
    }
}
