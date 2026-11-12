//! The one window toolbar (`ui::toolbar::bar`) in every window that draws it: what Open in File
//! Explorer, the list/grid switch, Sort and the details button do there, by pointer and by keyboard.

use std::sync::Arc;

use rune::{Env, Runtime};

use crate::frames::Intent;
use crate::frames::shared::tests::{click, info, press, settle, space};
use crate::shell::actions::{FixtureCall, FixtureEngine};
use crate::shell::app::GateState;
use crate::shell::host::Shell;
use crate::shell::platform::fake::FakePlatform;
use crate::shell::tray::{EngineState, TrayInputs};
use crate::{State, view};

/// The windows that show no folder of their own, each on a board frame.
const SECTIONS: [(&str, fn(&mut State)); 6] = [
    ("Shared", crate::frames::shared::scenes::dw_0501),
    ("Shared, Requests", crate::frames::shared::scenes::dw_0503),
    ("Trash", crate::frames::trash::scenes::dw_0747),
    ("Photos", |s| crate::frames::photos::set_scene(s, "DW-0601")),
    ("Sync history", crate::frames::this_pc::scenes::dw_1301),
    ("Available offline", crate::frames::this_pc::scenes::dw_0715),
];

/// A window on a scene, over an engine that records what it was asked.
fn window(scene: fn(&mut State)) -> (Arc<FixtureEngine>, Runtime<State>) {
    let engine = Arc::new(FixtureEngine::ready());
    let shell = Shell::fixture_on(engine.clone(), Arc::new(FakePlatform::default()));
    let mut state = crate::shell::state::init_with(shell);
    scene(&mut state);
    let env = Env {
        system_dark: false,
        scale: 2.0,
    };
    let mut rt = Runtime::new(state, view, (1278.0, 798.0), env);
    rt.on_key = Some(crate::shell::host::on_key);
    settle(&mut rt);
    (engine, rt)
}

/// Focus on a control, then Space (which presses a focused button).
fn press_focused(rt: &mut Runtime<State>, key: &'static str) {
    rt.update(move |_, tx| tx.focus(key, true));
    settle(rt);
    press(rt, space());
}

/// Everything a press on a dimmed control could change in these windows.
fn marks(rt: &Runtime<State>) -> String {
    let s = &rt.state;
    format!(
        "{:?} {:?} {} {} {:?} {:?}",
        s.frames.intents,
        s.actions.menu.map(|m| m.kind),
        s.show_detail_pane,
        crate::frames::global::grid::on(s),
        s.photos.dialog.is_some(),
        s.this_pc.filter,
    )
}

#[test]
fn open_in_file_explorer_is_the_last_icon_button_of_every_window_and_opens_the_beebeeb_location() {
    for (name, scene) in SECTIONS {
        let (engine, mut rt) = window(scene);
        let button = info(&rt, "open-explorer");
        assert_eq!(
            (button.role, button.label.as_deref(), button.focusable),
            ("button", Some("Open in File Explorer"), true),
            "{name}"
        );
        // After the details button, before the hairline and New: where the board draws it.
        let x = |key: &str| info(&rt, key).rect[0];
        let sep = if rt.find("toolbar-sep-2").is_some() {
            "toolbar-sep-2"
        } else {
            "toolbar-sep"
        };
        assert!(
            x("panel-toggle-btn") < x("open-explorer") && x("open-explorer") < x(sep) && x(sep) < x("new-btn"),
            "{name}"
        );
        assert_eq!(x("open-explorer") - x("panel-toggle-btn"), 36.0, "{name}");
        assert!(engine.calls().is_empty(), "{name}: nothing asked before a press");
        click(&mut rt, "open-explorer");
        assert_eq!(engine.calls(), [FixtureCall::OpenInFileExplorer], "{name}: the pointer");
        press_focused(&mut rt, "open-explorer");
        assert_eq!(
            engine.calls(),
            [FixtureCall::OpenInFileExplorer, FixtureCall::OpenInFileExplorer],
            "{name}: the keyboard"
        );
    }
}

#[test]
fn a_refused_open_in_file_explorer_changes_nothing_in_the_window() {
    // Beebeeb is not in File Explorer: the engine refuses, the window stays as it was (the refusal is logged).
    let engine = Arc::new(FixtureEngine::with(
        GateState::Ready,
        TrayInputs {
            signed_in: true,
            explorer_ready: false,
            engine: EngineState::Idle,
            ..TrayInputs::default()
        },
    ));
    let shell = Shell::fixture_on(engine.clone(), Arc::new(FakePlatform::default()));
    let mut state = crate::shell::state::init_with(shell);
    crate::frames::trash::scenes::dw_0747(&mut state);
    let env = Env {
        system_dark: false,
        scale: 2.0,
    };
    let mut rt = Runtime::new(state, view, (1278.0, 798.0), env);
    settle(&mut rt);
    let before = marks(&rt);
    click(&mut rt, "open-explorer");
    assert!(engine.calls().is_empty());
    assert_eq!(marks(&rt), before);
}

#[test]
fn the_list_grid_switch_is_dimmed_and_takes_nothing_where_a_window_has_one_layout() {
    for (name, scene) in SECTIONS {
        let (_, mut rt) = window(scene);
        let dimmed = name != "Sync history";
        let switch = info(&rt, "view-toggle");
        assert_eq!(switch.opacity, if dimmed { 0.45 } else { 1.0 }, "{name}");
        let before = marks(&rt);
        for side in ["view-list", "view-grid"] {
            assert!(!info(&rt, side).focusable, "{name}: {side} takes no focus");
            click(&mut rt, side);
        }
        assert_eq!(marks(&rt), before, "{name}: a press on the switch changes nothing");
        assert!(rt.state.frames.intents.iter().all(|i| *i != Intent::Frame("DW-1102")));
    }
}

#[test]
fn sort_opens_the_sort_frame_in_shared_and_trash_and_is_out_of_reach_in_photos_and_this_pc() {
    for (name, scene) in SECTIONS {
        let (_, mut rt) = window(scene);
        let live = matches!(name, "Shared" | "Shared, Requests" | "Trash");
        assert_eq!(info(&rt, "sort-btn").focusable, live, "{name}");
        let before = marks(&rt);
        click(&mut rt, "sort-btn");
        if live {
            assert_eq!(
                rt.state.frames.intents,
                [Intent::Frame("DW-0406")],
                "{name}: the pointer"
            );
            press_focused(&mut rt, "sort-btn");
            assert_eq!(
                rt.state.frames.intents,
                [Intent::Frame("DW-0406"), Intent::Frame("DW-0406")],
                "{name}: the keyboard"
            );
        } else {
            assert_eq!(marks(&rt), before, "{name}: a press on Sort changes nothing");
        }
    }
}

#[test]
fn the_details_button_is_out_of_reach_where_a_window_has_no_panel() {
    for (name, scene) in SECTIONS {
        let (_, mut rt) = window(scene);
        assert!(!info(&rt, "panel-toggle-btn").focusable, "{name}");
        let before = marks(&rt);
        click(&mut rt, "panel-toggle-btn");
        assert_eq!(
            marks(&rt),
            before,
            "{name}: a press on the details button changes nothing"
        );
    }
}

#[test]
fn over_an_open_photo_the_details_button_opens_details_by_pointer_and_by_keyboard() {
    for by_keyboard in [false, true] {
        let (_, mut rt) = window(|s| crate::frames::photos::set_scene(s, "DW-0605"));
        assert!(info(&rt, "panel-toggle-btn").focusable);
        assert!(rt.state.photos.dialog.is_none());
        if by_keyboard {
            press_focused(&mut rt, "panel-toggle-btn");
        } else {
            click(&mut rt, "panel-toggle-btn");
        }
        assert!(
            matches!(rt.state.photos.dialog, Some(crate::frames::photos::Dialog::Details)),
            "keyboard: {by_keyboard}"
        );
    }
}

#[test]
fn in_files_the_switch_sort_and_the_details_button_each_change_their_own_state() {
    use crate::frames::files::MenuKind;
    let (_, mut rt) = window(|_| {});
    for key in [
        "view-list",
        "view-grid",
        "sort-btn",
        "panel-toggle-btn",
        "open-explorer",
    ] {
        assert!(info(&rt, key).focusable, "{key}");
    }
    // The switch: this folder as a grid, and back, by pointer and by keyboard.
    assert!(!crate::frames::global::grid::on(&rt.state));
    click(&mut rt, "view-grid");
    assert!(crate::frames::global::grid::on(&rt.state));
    press_focused(&mut rt, "view-list");
    assert!(!crate::frames::global::grid::on(&rt.state));
    // The details panel: on in the board's Files window; a press hides it, the keyboard shows it again.
    let shown = rt.state.show_detail_pane;
    click(&mut rt, "panel-toggle-btn");
    assert_eq!(rt.state.show_detail_pane, !shown);
    press_focused(&mut rt, "panel-toggle-btn");
    assert_eq!(rt.state.show_detail_pane, shown);
    // Sort: its menu, by keyboard; a press on it again closes it.
    press_focused(&mut rt, "sort-btn");
    assert_eq!(rt.state.actions.menu.map(|m| m.kind), Some(MenuKind::Sort));
    click(&mut rt, "sort-btn");
    assert_eq!(rt.state.actions.menu.map(|m| m.kind), None);
}
