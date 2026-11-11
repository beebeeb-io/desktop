//! Headless tests of the shell in a Rune runtime: scripted keys and state, no window, no OS.

use std::sync::Arc;

use rune::{Env, Key, KeyEvent, Mods, Runtime};

use super::super::actions::{FixtureCall, FixtureEngine};
use super::super::app::{GateState, Screen};
use super::super::flyout_window;
use super::super::platform::fake::FakePlatform;
use super::super::registry::{self, FrameGroup};
use super::super::state;
use super::super::tray::{TrayIconState, TrayInputs};
use super::*;

const ENV: Env = Env {
    system_dark: false,
    scale: 2.0,
};

fn runtime(state: State) -> Runtime<State> {
    let mut rt = Runtime::new(state, registry::view, (1280.0, 800.0), ENV);
    rt.on_key = Some(on_key);
    rt.poll = Some(poll);
    rt.frame(0.0);
    rt
}

fn fixture() -> (Arc<FixtureEngine>, Arc<FakePlatform>, Runtime<State>) {
    let engine = Arc::new(FixtureEngine::ready());
    let platform = Arc::new(FakePlatform::default());
    let rt = runtime(state::init_with(Shell::fixture_on(engine.clone(), platform.clone())));
    (engine, platform, rt)
}

fn press(rt: &mut Runtime<State>, key: Key, mods: Mods) {
    rt.key(KeyEvent { key, mods });
    settle(rt);
}
/// Lets leaving elements finish leaving: Rune keeps them in the tree while they animate out.
fn settle(rt: &mut Runtime<State>) {
    rt.frame(0.016);
    rt.settle_all();
    rt.frame(0.016);
}
fn alt() -> Mods {
    Mods {
        alt: true,
        ..Mods::default()
    }
}
fn ctrl() -> Mods {
    Mods {
        cmd: true,
        ctrl: true,
        ..Mods::default()
    }
}
fn texts(rt: &Runtime<State>) -> Vec<String> {
    rt.elements().into_iter().filter_map(|e| e.text).collect()
}
fn go(rt: &mut Runtime<State>, section: Section) {
    rt.update(|s, _| {
        dispatch(s, Event::Navigate(Route::section(section)));
    });
    settle(rt);
}

#[test]
fn files_is_the_registered_frame_and_draws_dw_0401() {
    let (_, _, rt) = fixture();
    assert_eq!(
        registry::group_for(&rt.state.shell.app),
        FrameGroup::Section(Section::Files)
    );
    assert!(rt.find("file-list").is_some());
    assert!(rt.find("not-built").is_none(), "a built frame shows no placeholder");
}

#[test]
fn an_unbuilt_route_draws_the_placeholder_that_names_its_group() {
    let (_, _, mut rt) = fixture();
    go(&mut rt, Section::Shared);
    assert!(rt.find("not-built").is_some());
    assert!(
        rt.find("file-list").is_none(),
        "no Files rows under another section's name"
    );
    assert!(rt.find("detail-pane").is_none());
    let texts = texts(&rt);
    assert!(texts.iter().any(|t| t == "NOT BUILT YET"), "{texts:?}");
    assert!(texts.iter().any(|t| t == "Board: Shared (DW-0501)"), "{texts:?}");
    // The window's real chrome stays: caption and sidebar.
    assert!(rt.find("caption-bar").is_some());
    assert!(rt.find("sidebar").is_some());
}

#[test]
fn every_state_without_a_frame_has_a_placeholder_and_only_files_is_built() {
    let (engine, _, mut rt) = fixture();
    let mut unbuilt = 0;
    for section in Section::ALL {
        go(&mut rt, section);
        let built = registry::frame(FrameGroup::Section(section)).is_some();
        assert_eq!(rt.find("not-built").is_none(), built, "{section:?}");
        unbuilt += usize::from(!built);
    }
    assert_eq!(unbuilt, 6, "six sections wait for their lanes");
    for gate in [
        GateState::SignedOut,
        GateState::Locked,
        GateState::NeedsRecoveryPhrase,
        GateState::NoPlan,
        GateState::KeyStoreUnavailable,
    ] {
        engine.set_gate(gate);
        rt.update(|s, _| {
            s.shell.refresh();
        });
        rt.frame(0.016);
        assert!(rt.find("not-built").is_some(), "{gate:?}");
        // Behind the gate there is no sidebar and no list: nothing of the account.
        assert!(rt.find("sidebar").is_none(), "{gate:?}");
        assert!(rt.find("file-list").is_none(), "{gate:?}");
    }
}

#[test]
fn the_placeholder_never_shows_fixture_rows() {
    let (_, _, mut rt) = fixture();
    go(&mut rt, Section::Photos);
    let texts = texts(&rt);
    for name in ["Contract draft.pdf", "Lisbon trip", "Budget 2026.xlsx"] {
        assert!(
            !texts.iter().any(|t| t.contains(name)),
            "{name} leaked into a placeholder"
        );
    }
}

#[test]
fn alt_arrows_walk_the_history() {
    let (_, _, mut rt) = fixture();
    go(&mut rt, Section::Shared);
    go(&mut rt, Section::Trash);
    press(&mut rt, Key::Left, alt());
    assert_eq!(rt.state.shell.app.route(), &Route::section(Section::Shared));
    press(&mut rt, Key::Left, alt());
    assert_eq!(rt.state.shell.app.route(), &Route::section(Section::Files));
    assert!(rt.find("file-list").is_some());
    press(&mut rt, Key::Right, alt());
    assert_eq!(rt.state.shell.app.route(), &Route::section(Section::Shared));
    assert!(rt.find("not-built").is_some());
    // The sidebar's highlight follows the route.
    assert!(rt.state.sidebar_section == SidebarSection::Shared);
}

#[test]
fn plain_arrows_still_belong_to_the_files_frame() {
    let (_, _, mut rt) = fixture();
    assert_eq!(rt.state.selected, Some(6));
    press(&mut rt, Key::Down, Mods::default());
    assert_eq!(rt.state.selected, Some(7));
    // Alt+Left with no history is the shell's, and moves no selection.
    press(&mut rt, Key::Left, alt());
    assert_eq!(rt.state.selected, Some(7));
}

#[test]
fn ctrl_comma_opens_settings_and_esc_returns() {
    let (_, _, mut rt) = fixture();
    go(&mut rt, Section::Photos);
    press(&mut rt, Key::Command(','), ctrl());
    assert_eq!(rt.state.shell.app.screen(), Screen::Settings(SettingsPane::General));
    assert!(texts(&rt).iter().any(|t| t == "Settings: General"));
    assert!(
        rt.find("sidebar").is_none(),
        "settings has its own sidebar, not Drive's"
    );
    press(&mut rt, Key::Escape, Mods::default());
    assert_eq!(
        rt.state.shell.app.screen(),
        Screen::Drive(Route::section(Section::Photos))
    );
}

#[test]
fn ctrl_slash_opens_the_shortcuts_dialog_and_esc_closes_it() {
    let (_, _, mut rt) = fixture();
    press(&mut rt, Key::Command('/'), ctrl());
    assert_eq!(registry::group_for(&rt.state.shell.app), FrameGroup::ShortcutsDialog);
    press(&mut rt, Key::Escape, Mods::default());
    assert_eq!(
        registry::group_for(&rt.state.shell.app),
        FrameGroup::Section(Section::Files)
    );
}

#[test]
fn ctrl_slash_works_where_slash_is_a_shifted_key() {
    let (_, _, mut rt) = fixture();
    press(&mut rt, Key::Command('/'), Mods { shift: true, ..ctrl() });
    assert_eq!(registry::group_for(&rt.state.shell.app), FrameGroup::ShortcutsDialog);
}

#[test]
fn signed_out_takes_no_shortcut_to_the_account() {
    let engine = Arc::new(FixtureEngine::signed_out());
    let mut rt = runtime(state::init_with(Shell::fixture_on(
        engine,
        Arc::new(FakePlatform::default()),
    )));
    assert_eq!(rt.state.shell.app.screen(), Screen::SignIn);
    press(&mut rt, Key::Command(','), ctrl());
    press(&mut rt, Key::Left, alt());
    press(&mut rt, Key::Command('/'), ctrl());
    assert_eq!(rt.state.shell.app.screen(), Screen::SignIn);
    assert_eq!(registry::group_for(&rt.state.shell.app), FrameGroup::SignIn);
}

#[test]
fn a_sidebar_selection_becomes_a_navigation() {
    let (_, _, mut rt) = fixture();
    // What `ui::sidebar` does on a click.
    rt.update(|s, _| s.sidebar_section = SidebarSection::Trash);
    rt.frame(0.016);
    rt.frame(0.016);
    assert_eq!(rt.state.shell.app.route(), &Route::section(Section::Trash));
    assert!(rt.state.shell.app.can_go_back());
    assert!(rt.find("not-built").is_some());
}

#[test]
fn the_gate_closing_mid_navigation_hides_the_account_and_reopens_in_place() {
    let (engine, _, mut rt) = fixture();
    go(&mut rt, Section::Shared);
    engine.set_gate(GateState::Locked);
    rt.update(|s, _| assert!(s.shell.refresh()));
    rt.frame(0.016);
    assert_eq!(registry::group_for(&rt.state.shell.app), FrameGroup::Locked);
    assert_eq!(rt.state.shell.tray_model().tooltip, "Beebeeb: locked");
    engine.set_gate(GateState::Ready);
    rt.update(|s, _| assert!(s.shell.refresh()));
    assert_eq!(
        rt.state.shell.app.screen(),
        Screen::Drive(Route::section(Section::Shared))
    );
}

#[test]
fn menu_picks_reach_the_actions_and_the_b_follows() {
    let (engine, platform, mut rt) = fixture();
    rt.update(|s, tx| on_menu(s, MenuAction::PauseSyncing, tx));
    assert_eq!(rt.state.shell.tray_model().icon, TrayIconState::Paused);
    rt.update(|s, tx| on_menu(s, MenuAction::ResumeSyncing, tx));
    assert_eq!(rt.state.shell.tray_model().icon, TrayIconState::UpToDate);
    rt.update(|s, tx| on_menu(s, MenuAction::OpenInFileExplorer, tx));
    assert_eq!(
        engine.calls(),
        vec![FixtureCall::Pause, FixtureCall::Resume, FixtureCall::OpenInFileExplorer]
    );
    rt.update(|s, tx| on_menu(s, MenuAction::Settings, tx));
    assert_eq!(rt.state.shell.app.screen(), Screen::Settings(SettingsPane::General));
    // No tray was created in this test, so nothing was pushed to one.
    assert!(platform.applied.lock().unwrap().is_empty());
}

#[test]
fn a_refused_menu_action_changes_nothing() {
    let engine = Arc::new(FixtureEngine::with(
        GateState::Ready,
        TrayInputs {
            signed_in: true,
            explorer_ready: false,
            ..TrayInputs::default()
        },
    ));
    let mut rt = runtime(state::init_with(Shell::fixture_on(
        engine.clone(),
        Arc::new(FakePlatform::default()),
    )));
    rt.update(|s, tx| on_menu(s, MenuAction::OpenInFileExplorer, tx));
    assert!(engine.calls().is_empty());
    assert_eq!(
        rt.state.shell.app.screen(),
        Screen::Drive(Route::section(Section::Files))
    );
}

#[test]
fn a_link_only_ever_navigates_and_a_bad_one_does_nothing() {
    let (engine, _, mut rt) = fixture();
    rt.update(|s, tx| on_open_url(s, "beebeeb://open/trash", tx));
    assert_eq!(rt.state.shell.app.route(), &Route::section(Section::Trash));
    for bad in [
        "beebeeb://quit",
        "beebeeb://pause",
        "https://example.com/",
        "beebeeb://open/../settings",
    ] {
        rt.update(|s, tx| on_open_url(s, bad, tx));
        assert_eq!(rt.state.shell.app.route(), &Route::section(Section::Trash), "{bad}");
    }
    assert!(engine.calls().is_empty(), "no link reached an action");
}

#[test]
fn a_link_while_locked_waits_and_one_naming_a_folder_dies_with_a_sign_out() {
    let (engine, _, mut rt) = fixture();
    engine.set_gate(GateState::Locked);
    rt.update(|s, tx| {
        s.shell.refresh();
        on_open_url(s, "beebeeb://open/files?folder=f-123", tx);
    });
    assert_eq!(rt.state.shell.app.screen(), Screen::Gate(GateState::Locked));
    engine.set_gate(GateState::SignedOut);
    rt.update(|s, _| {
        s.shell.refresh();
    });
    engine.set_gate(GateState::Ready);
    rt.update(|s, _| {
        s.shell.refresh();
    });
    assert_eq!(
        rt.state.shell.app.screen(),
        Screen::Drive(Route::section(Section::Files))
    );
}

#[test]
fn closing_the_window_without_a_b_has_nowhere_to_come_back_from() {
    let (_, _, rt) = fixture();
    // The fixture shell has no tray: `on_close` then quits (see its doc). With one, it stays.
    assert!(!rt.state.shell.has_tray());
}

#[test]
fn the_tray_starts_only_when_wired_and_then_follows_the_model() {
    let engine = Arc::new(FixtureEngine::ready());
    let platform = Arc::new(FakePlatform::default());
    let mut shell = Shell::fixture_on(engine.clone(), platform.clone());
    shell.start_tray();
    assert!(!shell.has_tray(), "no app handle, no tray");
    // The model still follows the engine, so a tray added later starts right.
    engine.set_inputs(TrayInputs {
        signed_in: true,
        conflicts: 1,
        ..TrayInputs::default()
    });
    shell.refresh();
    assert_eq!(shell.tray_model().icon, TrayIconState::NeedsDecision);
}

#[test]
fn the_flyout_draws_its_placeholder_and_says_where_its_state_comes_from() {
    let mut rt = Runtime::new(flyout_window::fixture_state(), flyout_window::view, (340.0, 520.0), ENV);
    rt.frame(0.0);
    assert!(rt.find("flyout").is_some());
    let texts: Vec<String> = rt.elements().into_iter().filter_map(|e| e.text).collect();
    assert!(texts.iter().any(|t| t == "Flyout"), "{texts:?}");
    assert!(
        texts.iter().any(|t| t == "Tray state (fixture engine): Up to date"),
        "{texts:?}"
    );
    assert!(
        texts
            .iter()
            .any(|t| t == "Board: The notification area: the b and its flyout (DW-1201)"),
        "{texts:?}"
    );
}
