//! Tests of the two states before Files (DW-0009 locked, DW-0014 no plan yet): what shows, what
//! each button asks for, that nothing else can be reached, and that the window moves on with the
//! engine's gate. The window runs on the shell's fixture engine, as `shell/host_tests.rs` does.

use std::sync::Arc;

use super::before_files::{CHECK_PLAN, OPEN_PLANS, SIGN_OUT};
use crate::State;
use crate::frames::GateNotice;
use crate::shell::actions::FixtureEngine;
use crate::shell::app::{GateState, Screen};
use crate::shell::host::{self, Shell};
use crate::shell::platform::fake::FakePlatform;
use crate::shell::{registry, state};
use rune::{Env, Key, KeyEvent, Mods, Runtime};

/// The main window with the engine's gate at `gate`.
fn window(gate: GateState, dark: bool) -> (Arc<FixtureEngine>, Runtime<State>) {
    let engine = Arc::new(FixtureEngine::ready());
    engine.set_gate(gate);
    let platform = Arc::new(FakePlatform::default());
    let state = state::init_with(Shell::fixture_on(engine.clone(), platform));
    let env = Env {
        system_dark: dark,
        scale: 2.0,
    };
    let mut rt = Runtime::new(state, registry::view, (1278.0, 798.0), env);
    rt.on_key = Some(host::on_key);
    rt.poll = Some(host::poll);
    rt.frame(0.0);
    rt.frame(0.016);
    (engine, rt)
}

fn find(rt: &Runtime<State>, key: &str) -> Option<rune::ElementInfo> {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
}

fn says(rt: &Runtime<State>, key: &str) -> String {
    find(rt, key)
        .unwrap_or_else(|| panic!("no element keyed {key}"))
        .text
        .unwrap_or_default()
}

fn settle(rt: &mut Runtime<State>) {
    rt.frame(0.016);
    rt.settle_all();
    rt.frame(0.016);
}

fn key(rt: &mut Runtime<State>, key: Key) {
    rt.key(KeyEvent {
        key,
        mods: Mods::default(),
    });
    settle(rt);
}

fn click(rt: &mut Runtime<State>, key: &str) {
    let r = find(rt, key).unwrap_or_else(|| panic!("no element keyed {key}")).rect;
    let (x, y) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    settle(rt);
}

fn notice(rt: &Runtime<State>) -> Option<GateNotice> {
    rt.state.frames.gate_notice.clone()
}

/// Every key Tab stops on, in order of first visit.
fn tab_stops(rt: &mut Runtime<State>) -> Vec<String> {
    let mut stops: Vec<String> = Vec::new();
    for _ in 0..40 {
        key(rt, Key::Tab);
        if let Some(now) = rt
            .elements()
            .into_iter()
            .find(|e| e.focused && !e.exiting)
            .and_then(|e| e.key)
            && !stops.contains(&now)
        {
            stops.push(now);
        }
    }
    stops
}

#[test]
fn both_states_say_what_the_board_says_in_both_themes() {
    for dark in [false, true] {
        let (_, rt) = window(GateState::Locked, dark);
        assert_eq!(says(&rt, "gate-title"), "Beebeeb is locked");
        assert_eq!(says(&rt, "gate-text"), "Unlock to see your files.");
        assert_eq!(says(&rt, "folder-title"), "Beebeeb is locked");
        assert!(find(&rt, "item-count").is_none(), "no count of items while locked");
        assert_eq!(says(&rt, "status-text"), "Locked \u{00B7} sync keeps running");
        assert_eq!(says(&rt, "gate-btn-0-label"), "Unlock");

        let (_, rt) = window(GateState::NoPlan, dark);
        assert_eq!(says(&rt, "gate-title"), "Choose a plan");
        assert_eq!(
            says(&rt, "gate-text"),
            "Your account is ready. Choose a plan on beebeeb.io, including the free trial. Beebeeb continues here by itself."
        );
        assert_eq!(
            (says(&rt, "folder-title"), says(&rt, "item-count")),
            ("Files".into(), "No plan yet".into())
        );
        assert_eq!(says(&rt, "status-text"), "Not syncing \u{00B7} no plan");
        assert_eq!(says(&rt, "storage-value"), "No plan");
        assert_eq!(says(&rt, "storage-hint"), "Choose a plan on beebeeb.io");
        for gone in [
            "Shared-badge",
            "Trash-badge",
            "Available offline-badge",
            "storage-files",
        ] {
            assert!(find(&rt, gone).is_none(), "{gone} is not drawn without a plan");
        }
    }
}

/// The sidebar shows the source's account (the fixture's) for NoPlan, and blanks the account
/// details for Locked (decision 2483: LOCKED_BLANKS_ACCOUNT).
#[test]
fn the_sidebar_shows_the_sources_account_and_locked_blanks_it() {
    // NoPlan: the fixture's account shows in the sidebar.
    let (_, rt) = window(GateState::NoPlan, false);
    assert_eq!(
        says(&rt, "account-name"),
        "Anna Visser",
        "NoPlan shows the fixture account"
    );
    assert_eq!(says(&rt, "account-email"), "anna@northwind.studio");

    // Locked: the account details are blanked while LOCKED_BLANKS_ACCOUNT is true.
    let (_, rt) = window(GateState::Locked, false);
    assert_eq!(says(&rt, "account-name"), "", "locked: account name is blank");
    assert!(
        find(&rt, "account-email").is_none() || says(&rt, "account-email").is_empty(),
        "locked: account email is blank or absent"
    );
    // The storage meter keeps its shape but shows no figure.
    assert!(says(&rt, "storage-value").is_empty(), "locked: storage value is blank");
    // Section counts are hidden.
    for badge in ["Files-badge", "Shared-badge", "Trash-badge"] {
        assert!(find(&rt, badge).is_none(), "locked: {badge} is not drawn");
    }
}

#[test]
fn the_scenes_are_the_two_gate_states() {
    for (frame, gate) in [("DW-0009", GateState::Locked), ("DW-0014", GateState::NoPlan)] {
        let mut state = crate::init();
        let scene = super::before_files::FRAMES
            .iter()
            .find(|(id, _)| *id == frame)
            .unwrap_or_else(|| panic!("{frame} is a scene"));
        (scene.1)(&mut state);
        assert_eq!(state.shell.app.screen(), Screen::Gate(gate), "{frame}");
    }
}

/// Unlock is bound (`lock_tests.rs` has its answers): a press raises no "Not available yet", and the gate stays
/// the engine's.
#[test]
fn unlock_is_not_a_not_built_button_and_changes_no_gate_by_itself() {
    let (_, mut rt) = window(GateState::Locked, false);
    click(&mut rt, "gate-btn-0");
    key(&mut rt, Key::Enter);
    assert!(
        notice(&rt).is_none(),
        "Unlock asks Windows Hello; it is not a not-built action"
    );
    assert_eq!(rt.state.shell.app.screen(), Screen::Gate(GateState::Locked));
}

#[test]
fn each_button_without_a_plan_shows_not_available() {
    let (_, mut rt) = window(GateState::NoPlan, false);
    for (key, action) in [
        ("gate-btn-0", OPEN_PLANS),
        ("gate-btn-1", CHECK_PLAN),
        ("gate-btn-2", SIGN_OUT),
    ] {
        rt.state.frames.gate_notice = None;
        click(&mut rt, key);
        let n = notice(&rt).unwrap_or_else(|| panic!("{action}: pressing {key} sets the notice"));
        assert_eq!(n.message, "Not available yet", "{action}");
        assert_eq!(n.gate, GateState::NoPlan, "{action}: stamped with the gate");
    }
    let name = find(&rt, "gate-btn-0").and_then(|e| e.label).unwrap_or_default();
    assert!(
        name.contains("opens in your browser"),
        "the button says it leaves the app: {name}"
    );
    assert!(rt.state.frames.intents.is_empty(), "no dead intent was queued");
}

#[test]
fn nothing_but_the_blocks_buttons_can_be_reached() {
    for (gate, buttons) in [
        (GateState::Locked, vec!["gate-btn-0"]),
        (GateState::NoPlan, vec!["gate-btn-0", "gate-btn-1", "gate-btn-2"]),
    ] {
        let (_, mut rt) = window(gate, false);
        let mut stops = tab_stops(&mut rt);
        // The caption's own buttons are Windows' window controls.
        stops.retain(|k| !k.starts_with("cap-"));
        assert_eq!(stops, buttons, "{gate:?}: Tab reaches the block's buttons only");
        let section = rt.state.sidebar_section;
        for dead in [
            "Shared-row",
            "Trash-row",
            "Sync history-row",
            "settings-btn",
            "new-btn",
            "sort-btn",
            "view-grid",
        ] {
            click(&mut rt, dead);
        }
        assert!(
            rt.state.sidebar_section == section,
            "{gate:?}: the sidebar selects nothing"
        );
        assert_eq!(
            rt.state.shell.app.screen(),
            Screen::Gate(gate),
            "{gate:?}: and the window stays"
        );
        assert!(notice(&rt).is_none(), "{gate:?}: nothing was asked for");
        click(&mut rt, "search-input");
        key(&mut rt, Key::Text("budget".into()));
        assert!(rt.state.search.text.is_empty(), "{gate:?}: nothing is searched");
    }
}

/// DW-0009's rule: "No file names in the window". The fixture's Files folder has eleven.
#[test]
fn a_locked_window_shows_no_file_name() {
    let (_, rt) = window(GateState::Locked, false);
    let names: Vec<String> = rt.state.listing.items.iter().map(|i| i.name.clone()).collect();
    let tree: String = rt
        .elements()
        .iter()
        .map(|e| format!("{:?}|{:?}\n", e.label, e.text))
        .collect();
    let a11y = rt.accessibility_json().to_string();
    assert_eq!(names.len(), 11, "the fixture's folder");
    // "Backups" is also a section's name in the sidebar: a folder of that name proves nothing.
    for name in names.iter().filter(|n| n.as_str() != "Backups") {
        assert!(!tree.contains(name.as_str()), "{name} is in the element tree");
        assert!(!a11y.contains(name.as_str()), "{name} is in the accessibility tree");
    }
    assert!(
        find(&rt, "row-0").is_none() && find(&rt, "column-headers").is_none(),
        "no list at all"
    );
}

/// The notice a button raised is not drawn after the gate changes: it was stamped with the old
/// gate, so the view's filter drops it.
#[test]
fn the_notice_is_not_drawn_after_a_gate_change() {
    let (engine, mut rt) = window(GateState::NoPlan, false);
    click(&mut rt, "gate-btn-0");
    assert!(find(&rt, "gate-notice").is_some(), "the notice is drawn");
    engine.set_gate(GateState::Ready);
    rt.frame(1.0);
    settle(&mut rt);
    assert!(
        find(&rt, "gate-notice").is_none(),
        "the notice is gone after the gate changed"
    );
}

/// The board's flow: Windows Hello recognises the person, or a plan arrives, and the window
/// shows Files (DW-0401). The gate is the engine's.
#[test]
fn the_window_shows_files_when_the_engines_gate_opens() {
    for gate in [GateState::Locked, GateState::NoPlan] {
        let (engine, mut rt) = window(gate, false);
        assert!(find(&rt, "gate-title").is_some());
        engine.set_gate(GateState::Ready);
        rt.frame(1.0);
        settle(&mut rt);
        assert_eq!(
            rt.state.shell.app.screen(),
            Screen::Drive(crate::shell::app::Route::section(crate::shell::app::Section::Files)),
            "{gate:?}"
        );
        assert!(find(&rt, "gate-title").is_none(), "{gate:?}: the block is gone");
        assert!(find(&rt, "column-headers").is_some(), "{gate:?}: the list is back");
    }
}

#[test]
fn the_lock_screens_line_follows_the_engine() {
    use crate::shell::tray::{EngineState, TrayInputs};
    let (engine, mut rt) = window(GateState::Locked, false);
    assert_eq!(says(&rt, "status-text"), "Locked \u{00B7} sync keeps running");
    engine.set_inputs(TrayInputs {
        engine: EngineState::NotRunning,
        locked: true,
        signed_in: true,
        ..Default::default()
    });
    rt.frame(1.0);
    rt.frame(0.016);
    assert_eq!(says(&rt, "status-text"), "Locked \u{00B7} nothing syncs");
}
