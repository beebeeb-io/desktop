//! Tests of the first-time guide: where it starts, what Got it, Next, Skip, Done and Esc do, that
//! each ring is on the real control, and that the window under a tip stays usable.

use rune::{Env, Key, KeyEvent, Mods, Runtime};

use super::guide::{self, COACH_KEY, FRAMES, NEXT_KEY, RING_KEY, SKIP_KEY, Step, TIPS, TRAY_TEXT, TRAY_TITLE};
use crate::shell::app::{Event, GateState, Screen, Section, SettingsPane};
use crate::shell::host;
use crate::{State, init, view};

/// The main window's client area on the board.
const WINDOW: (f32, f32) = (1278.0, 798.0);

fn window(state: State) -> Runtime<State> {
    window_sized(state, WINDOW)
}

fn window_sized(state: State, size: (f32, f32)) -> Runtime<State> {
    let env = Env {
        system_dark: false,
        scale: 2.0,
    };
    let mut rt = Runtime::new(state, view, size, env);
    rt.on_key = Some(host::on_key);
    rt.frame(0.0);
    rt
}

fn at_scene(frame: &str) -> Runtime<State> {
    at_scene_sized(frame, WINDOW)
}

fn at_scene_sized(frame: &str, size: (f32, f32)) -> Runtime<State> {
    let mut state = init();
    let scene = FRAMES
        .iter()
        .find(|(id, _)| *id == frame)
        .unwrap_or_else(|| panic!("no scene {frame}"));
    (scene.1)(&mut state);
    window_sized(state, size)
}

/// The guide started as the app starts it, on the board's Files.
fn started() -> Runtime<State> {
    let mut state = init();
    guide::start(&mut state);
    window(state)
}

fn rect(rt: &Runtime<State>, key: &str) -> Option<[f32; 4]> {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
        .map(|e| e.rect)
}

fn text(rt: &Runtime<State>, key: &str) -> Option<String> {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
        .and_then(|e| e.text)
}

fn click(rt: &mut Runtime<State>, key: &str) {
    rt.settle_all();
    rt.frame(0.0);
    let r = rect(rt, key).unwrap_or_else(|| panic!("no element keyed {key}"));
    let (x, y) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(0.0);
    rt.settle_all();
    rt.frame(0.0);
}

fn press(rt: &mut Runtime<State>, key: Key) {
    rt.key(KeyEvent {
        key,
        mods: Mods::default(),
    });
    rt.frame(0.0);
}

fn section(rt: &Runtime<State>) -> Option<Section> {
    match rt.state.shell.app.screen() {
        Screen::Drive(route) => Some(route.section),
        _ => None,
    }
}

/// The ring stands around its target: no side of the target is more than `slack` outside it.
fn ring_holds(rt: &Runtime<State>, target: &str, slack: f32) {
    let ring = rect(rt, RING_KEY).expect("the ring is drawn");
    let t = rect(rt, target).unwrap_or_else(|| panic!("no element keyed {target}"));
    assert!(
        ring[0] <= t[0] + 0.01
            && ring[1] <= t[1] + 0.01
            && ring[0] + ring[2] >= t[0] + t[2] - 0.01
            && ring[1] + ring[3] >= t[1] + t[3] - slack,
        "the ring {ring:?} is not around {target} {t:?}"
    );
}

#[test]
fn every_board_frame_draws_its_step_with_its_words() {
    let words = [
        ("DW-0380", TRAY_TEXT, None, "Got it"),
        ("DW-0301", TIPS[0], Some("1 of 5"), "Next"),
        ("DW-0302", TIPS[1], Some("2 of 5"), "Next"),
        ("DW-0303", TIPS[2], Some("3 of 5"), "Next"),
        ("DW-0304", TIPS[3], Some("4 of 5"), "Next"),
        ("DW-0305", TIPS[4], Some("5 of 5"), "Done"),
    ];
    assert_eq!(FRAMES.len(), 6);
    for (frame, body, counter, button) in words {
        let rt = at_scene(frame);
        assert_eq!(rt.state.guide.step.map(Step::frame), Some(frame));
        assert_eq!(text(&rt, "coach-text").as_deref(), Some(body), "{frame}");
        assert_eq!(text(&rt, "coach-counter").as_deref(), counter, "{frame}");
        assert_eq!(text(&rt, "coach-next-label").as_deref(), Some(button), "{frame}");
        // Skip is on tips 1 to 4 only; DW-0380 has its heading and no ring.
        let tip = counter.is_some();
        assert_eq!(rect(&rt, SKIP_KEY).is_some(), tip && frame != "DW-0305", "{frame}");
        assert_eq!(rect(&rt, RING_KEY).is_some(), tip, "{frame}");
        assert_eq!(
            text(&rt, "coach-title").as_deref(),
            (!tip).then_some(TRAY_TITLE),
            "{frame}"
        );
    }
}

#[test]
fn each_ring_is_on_the_real_control_and_follows_it() {
    for (frame, target) in [
        ("DW-0301", "new-btn"),
        ("DW-0302", "row-6"),
        ("DW-0303", "Shared-row"),
        ("DW-0304", "Photos-row"),
    ] {
        ring_holds(&at_scene(frame), target, 0.0);
    }
    // The board's ring ends 1.5 above the gear button's lower edge.
    ring_holds(&at_scene("DW-0305"), "settings-btn", 1.5);

    // A window of another size: the gear moved, and the ring with it.
    let before = rect(&at_scene("DW-0305"), RING_KEY).unwrap();
    let rt = at_scene_sized("DW-0305", (1100.0, 700.0));
    let after = rect(&rt, RING_KEY).unwrap();
    assert!((before[1] - after[1] - 98.0).abs() < 0.5, "{before:?} -> {after:?}");
    ring_holds(&rt, "settings-btn", 1.5);
}

#[test]
fn got_it_and_next_step_through_the_five_tips_and_done_ends_on_files() {
    let mut rt = started();
    assert_eq!(rt.state.guide.step, Some(Step::Tray));
    let mut seen = Vec::new();
    for _ in 0..6 {
        seen.push((rt.state.guide.step.unwrap(), section(&rt).unwrap()));
        assert!(
            rect(&rt, COACH_KEY).is_some(),
            "{:?} draws its card",
            rt.state.guide.step
        );
        click(&mut rt, NEXT_KEY);
    }
    assert_eq!(
        seen,
        [
            (Step::Tray, Section::Files),
            (Step::Tip(1), Section::Files),
            (Step::Tip(2), Section::Files),
            // "Switches to the real Shared view by itself."
            (Step::Tip(3), Section::Shared),
            (Step::Tip(4), Section::Photos),
            (Step::Tip(5), Section::Files),
        ]
    );
    // Done: no guide, and the window is on Files.
    assert_eq!(rt.state.guide.step, None);
    assert_eq!(section(&rt), Some(Section::Files));
    assert!(rect(&rt, COACH_KEY).is_none() && rect(&rt, RING_KEY).is_none());
}

#[test]
fn tip_2_is_left_out_when_the_folder_has_no_file() {
    let mut state = init();
    state.listing.items.retain(|item| item.is_folder);
    assert!(!state.listing.items.is_empty(), "the board's folders are still there");
    state.clear_selection();
    guide::start(&mut state);
    let mut rt = window(state);
    click(&mut rt, NEXT_KEY);
    assert_eq!(rt.state.guide.step, Some(Step::Tip(1)));
    click(&mut rt, NEXT_KEY);
    assert_eq!(rt.state.guide.step, Some(Step::Tip(3)));
    assert_eq!(section(&rt), Some(Section::Shared));
}

#[test]
fn skip_and_esc_end_the_whole_guide_where_the_window_is() {
    let mut rt = at_scene("DW-0303");
    click(&mut rt, SKIP_KEY);
    assert_eq!(rt.state.guide.step, None);
    assert_eq!(section(&rt), Some(Section::Shared));
    assert!(rect(&rt, COACH_KEY).is_none());

    let mut rt = at_scene("DW-0301");
    press(&mut rt, Key::Escape);
    assert_eq!(rt.state.guide.step, None);
    assert!(rect(&rt, COACH_KEY).is_none() && rect(&rt, RING_KEY).is_none());
    // With no guide, Esc is the window's again: it is not swallowed by a guide that is gone.
    assert!(!guide::escape(&mut rt.state));
}

#[test]
fn the_window_under_a_tip_stays_usable() {
    // A row under the dim takes its click.
    let mut rt = at_scene("DW-0301");
    assert_eq!(rt.state.selected, None);
    click(&mut rt, "row-9");
    assert_eq!(rt.state.selected, Some(9));
    assert_eq!(rt.state.guide.step, Some(Step::Tip(1)), "and the tip is still there");

    // The control inside the ring takes its click too: the gear opens settings.
    let mut rt = at_scene("DW-0305");
    click(&mut rt, "settings-btn");
    assert_eq!(rt.state.shell.app.screen(), Screen::Settings(SettingsPane::General));
    // A tip belongs to its place: over settings it is not drawn, and Esc is settings' own.
    assert!(rect(&rt, COACH_KEY).is_none());
    press(&mut rt, Key::Escape);
    assert_eq!(section(&rt), Some(Section::Files));
    assert_eq!(rt.state.guide.step, Some(Step::Tip(5)));
    assert!(rect(&rt, COACH_KEY).is_some(), "back on Files the tip shows again");
}

#[test]
fn open_my_beebeeb_at_the_end_of_the_first_run_starts_the_guide() {
    let mut state = init();
    super::set_scene(&mut state, "DW-0208");
    let ui = state.setup.as_mut().unwrap();
    // A row of "You're set up" is not the guide's to take.
    ui.open_setting(super::Setting::AppLock);
    let mut rt = window(state);
    rt.update(|s, tx| {
        host::poll(s, 0.0, tx);
    });
    assert_eq!(rt.state.guide.step, None, "nothing starts the guide before the press");

    rt.update(|s, _| s.setup.as_mut().unwrap().finish());
    rt.update(|s, tx| {
        host::poll(s, 0.0, tx);
    });
    assert_eq!(rt.state.guide.step, Some(Step::Tray));
    assert_eq!(
        rt.state.setup.as_mut().unwrap().take_intents(),
        [super::Intent::OpenSetting(super::Setting::AppLock)]
    );
    // The first-run window shows no tip; the main window on Files does.
    rt.frame(0.0);
    assert!(rect(&rt, COACH_KEY).is_none());
    rt.update(|s, _| {
        host::dispatch(s, Event::Gate(GateState::Ready));
    });
    rt.frame(0.0);
    assert_eq!(text(&rt, "coach-title").as_deref(), Some(TRAY_TITLE));
}

#[test]
fn show_the_guide_again_in_settings_runs_it_on_files() {
    let mut state = init();
    host::dispatch(&mut state, Event::OpenSettings(SettingsPane::General));
    let mut rt = window(state);
    assert_eq!(text(&rt, "item-1-0-title").as_deref(), Some("Show the guide again"));
    click(&mut rt, "item-1-0");
    assert_eq!(section(&rt), Some(Section::Files), "settings closed");
    assert_eq!(rt.state.guide.step, Some(Step::Tray));
    assert_eq!(text(&rt, "coach-title").as_deref(), Some(TRAY_TITLE));
    assert!(
        rt.state.settings.take_intents().is_empty(),
        "nothing is left asked of the shell"
    );
}

#[test]
fn the_cards_have_names_and_their_buttons_take_the_keyboard() {
    let rt = at_scene("DW-0302");
    let card = rt
        .elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(COACH_KEY))
        .unwrap();
    assert_eq!(card.label.as_deref(), Some("Guide, tip 2 of 5"));
    for key in [SKIP_KEY, NEXT_KEY] {
        let button = rt
            .elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some(key))
            .unwrap();
        assert!(button.focusable, "{key} takes Tab");
    }
}
