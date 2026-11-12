//! Headless tests of the shell in a Rune runtime: scripted keys and state, no window, no OS.

use std::sync::Arc;

use rune::{Env, Key, KeyEvent, Mods, Runtime};

use super::super::actions::{FixtureCall, FixtureEngine};
use super::super::app::{GateState, Screen};
use super::super::control::Request;
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
    // A window whose source knows no Backups folder (`FilesSource::backups_root` is `None`).
    rt.update(|s, _| s.actions = crate::frames::files::Actions::default());
    go(&mut rt, Section::Backups);
    assert!(rt.find("not-built").is_some());
    assert!(
        rt.find("file-list").is_none(),
        "no Files rows under another section's name"
    );
    assert!(rt.find("detail-pane").is_none());
    let texts = texts(&rt);
    assert!(texts.iter().any(|t| t == "NOT BUILT YET"), "{texts:?}");
    assert!(
        texts.iter().any(|t| t == "Board: Backups on Windows (DW-0737)"),
        "{texts:?}"
    );
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
    assert_eq!(
        unbuilt, 0,
        "every section has its frame (Backups is the Files window on the Backups folder: `frames::files::protected`)"
    );
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
        // As for the sections above: the placeholder, unless the group's lane registered its frame.
        let built = registry::frame(registry::group_for(&rt.state.shell.app)).is_some();
        assert_eq!(rt.find("not-built").is_none(), built, "{gate:?}");
        // Behind the gate there is no sidebar and no list: nothing of the account.
        assert!(rt.find("sidebar").is_none(), "{gate:?}");
        assert!(rt.find("file-list").is_none(), "{gate:?}");
    }
}

#[test]
fn the_placeholder_never_shows_fixture_rows() {
    let (_, _, mut rt) = fixture();
    // A window whose source knows no Backups folder (`FilesSource::backups_root` is `None`).
    rt.update(|s, _| s.actions = crate::frames::files::Actions::default());
    go(&mut rt, Section::Backups);
    assert!(
        rt.find("not-built").is_some(),
        "Backups without a source that knows its folder"
    );
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
    assert!(
        rt.find("file-list").is_none() && rt.find("seg-0").is_some(),
        "Shared's own frame"
    );
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
    assert!(rt.find("Photos-row").is_some(), "Drive's sidebar, before");
    press(&mut rt, Key::Command(','), ctrl());
    assert_eq!(rt.state.shell.app.screen(), Screen::Settings(SettingsPane::General));
    // The settings lane draws General now (`frames::settings`): its heading, not the placeholder's.
    assert!(texts(&rt).iter().any(|t| t == "General"));
    // Both sidebars are keyed `sidebar` (the board's name for either): tell them by their items.
    assert!(
        rt.find("Photos-row").is_none() && rt.find("settings-nav").is_some(),
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
    assert_eq!(
        registry::dialog_group_for(&rt.state.shell.app),
        Some(FrameGroup::ShortcutsDialog)
    );
    press(&mut rt, Key::Escape, Mods::default());
    assert_eq!(registry::dialog_group_for(&rt.state.shell.app), None);
    assert_eq!(
        registry::group_for(&rt.state.shell.app),
        FrameGroup::Section(Section::Files)
    );
}

/// P2-8. A dialog draws OVER the frame behind it: Files stays drawn, the scrim covers the
/// window under its caption, the dialog's card is on top, and the frame behind gets no keys.
#[test]
fn a_dialog_is_layered_over_the_frame_behind_it() {
    let (_, _, mut rt) = fixture();
    let selected = rt.state.selected;
    press(&mut rt, Key::Command('/'), ctrl());
    // Behind: the Files frame, still the group of the state and still in the tree.
    assert_eq!(
        registry::group_for(&rt.state.shell.app),
        FrameGroup::Section(Section::Files)
    );
    assert!(rt.find("file-list").is_some(), "Files is still drawn behind the dialog");
    assert!(rt.find("sidebar").is_some());
    // In front: the scrim over the window under its caption (the board's `.mk-scrim` in
    // DW-1232: the 1278 x 798 client area less the 32 px caption band), then the card.
    let rect_of = |rt: &Runtime<State>, key: &str| {
        rt.elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some(key))
            .map(|e| e.rect)
            .expect(key)
    };
    let (scrim, window) = (rect_of(&rt, "dialog-scrim"), rect_of(&rt, "window"));
    assert_eq!(
        scrim,
        [
            window[0],
            window[1] + crate::TITLEBAR_H,
            window[2],
            window[3] - crate::TITLEBAR_H
        ],
        "the scrim covers the window under the caption"
    );
    assert!(
        rt.elements()
            .iter()
            .any(|e| e.key.as_deref() == Some("file-list") && e.inert)
    );
    assert!(rt.find("dialog-layer").is_some());
    assert!(texts(&rt).contains(&"Commands and keyboard shortcuts".to_owned()));
    // The frame behind takes no keys while the dialog is open.
    press(&mut rt, Key::Down, Mods::default());
    assert_eq!(rt.state.selected, selected);
    press(&mut rt, Key::Escape, Mods::default());
    assert!(rt.find("dialog-scrim").is_none());
    assert!(rt.find("file-list").is_some());
    press(&mut rt, Key::Down, Mods::default());
    assert_ne!(rt.state.selected, selected, "and gets them again once it is closed");
}

/// A dialog over Settings: closing it leaves Settings open.
#[test]
fn closing_a_dialog_does_not_close_what_is_behind_it() {
    let (_, _, mut rt) = fixture();
    press(&mut rt, Key::Command(','), ctrl());
    press(&mut rt, Key::Command('/'), ctrl());
    assert!(rt.find("dialog-scrim").is_some());
    assert_eq!(
        registry::group_for(&rt.state.shell.app),
        FrameGroup::Settings(SettingsPane::General)
    );
    press(&mut rt, Key::Escape, Mods::default());
    assert!(rt.find("dialog-scrim").is_none());
    assert_eq!(rt.state.shell.app.screen(), Screen::Settings(SettingsPane::General));
    assert_eq!(registry::scrim(false).a, 0.28);
    assert_eq!(registry::scrim(true).a, 0.55);
}

#[test]
fn ctrl_slash_works_where_slash_is_a_shifted_key() {
    let (_, _, mut rt) = fixture();
    press(&mut rt, Key::Command('/'), Mods { shift: true, ..ctrl() });
    assert_eq!(
        registry::dialog_group_for(&rt.state.shell.app),
        Some(FrameGroup::ShortcutsDialog)
    );
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
    // A window whose source knows no Backups folder (`FilesSource::backups_root` is `None`).
    rt.update(|s, _| s.actions = crate::frames::files::Actions::default());
    // What `ui::sidebar` does on a click.
    rt.update(|s, _| s.sidebar_section = SidebarSection::Backups);
    rt.frame(0.016);
    rt.frame(0.016);
    assert_eq!(rt.state.shell.app.route(), &Route::section(Section::Backups));
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
    assert_eq!(
        rt.state.shell.tray_model().tooltip,
        format!(
            "{}: locked",
            crate::shell::identity::AppIdentity::CURRENT.product_name()
        )
    );
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
    rt.update(|s, tx| on_open_url(s, &crate::shell::deeplink::link("open/trash"), tx));
    assert_eq!(rt.state.shell.app.route(), &Route::section(Section::Trash));
    let link = crate::shell::deeplink::link;
    for bad in [
        link("quit"),
        link("pause"),
        "https://example.com/".to_string(),
        link("open/../settings"),
    ] {
        let bad = bad.as_str();
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
        on_open_url(s, &crate::shell::deeplink::link("open/files?folder=f-123"), tx);
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
    assert!(rt.state.shell.close_quits());
}

fn no_events() -> platform::TrayHandler {
    Arc::new(|_| {})
}

/// P2-1. A tray whose icon was never added is not a b: closing the window quits. When the
/// icon appears, closing keeps the app; when it goes again, closing quits again.
#[test]
fn a_tray_whose_icon_was_not_added_is_not_a_way_back() {
    let platform = Arc::new(FakePlatform::without_notification_area());
    let mut shell = Shell::fixture_on(Arc::new(FixtureEngine::ready()), platform.clone());
    shell.attach_tray(no_events());
    assert!(shell.tray.is_some(), "the tray exists and keeps trying");
    assert!(!shell.has_tray(), "but there is no icon");
    assert!(shell.close_quits(), "never a live process with no surface");
    platform.icon_present.store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(shell.has_tray());
    assert!(!shell.close_quits());
    platform.icon_present.store(false, std::sync::atomic::Ordering::SeqCst);
    assert!(shell.close_quits());
    // A platform with no tray at all.
    let mut shell = Shell::fixture_on(Arc::new(FixtureEngine::ready()), Arc::new(FakePlatform::unbuilt()));
    shell.attach_tray(no_events());
    assert!(shell.tray.is_none() && shell.close_quits());
}

/// P2-2. The quit hook (which Rune also runs when Windows only ASKS to shut down) takes the
/// icon away without dropping the tray and stops the engine; a cancelled shutdown resumes it,
/// and the b is still the way back.
#[test]
fn the_quit_hook_keeps_the_tray_so_a_cancelled_shutdown_can_bring_the_b_back() {
    let engine = Arc::new(FixtureEngine::ready());
    let platform = Arc::new(FakePlatform::default());
    let mut shell = Shell::fixture_on(engine.clone(), platform.clone());
    shell.attach_tray(no_events());
    let mut rt = runtime(state::init_with(shell));
    rt.update(on_quit);
    assert!(
        rt.state.shell.tray.is_some(),
        "the tray is not dropped in the query phase"
    );
    assert_eq!(*platform.removed_for_quit.lock().unwrap(), 1);
    assert_eq!(engine.calls(), vec![FixtureCall::StopForQuit]);
    // Another app refused the shutdown.
    rt.update(|s, tx| on_tray(s, TrayEvent::ShutdownCancelled, tx));
    assert_eq!(
        engine.calls(),
        vec![FixtureCall::StopForQuit, FixtureCall::ResumeAfterCancelledQuit]
    );
    assert!(rt.state.shell.has_tray() && !rt.state.shell.close_quits());
    // A second cancel (nothing was stopped) resumes nothing.
    rt.update(|s, tx| on_tray(s, TrayEvent::ShutdownCancelled, tx));
    assert_eq!(engine.calls().len(), 2);
}

/// P2-6 at the shell: the engine reports another account between two refreshes, with no
/// sign-out in between.
#[test]
fn another_account_between_two_refreshes_clears_the_window() {
    let (engine, _, mut rt) = fixture();
    go(&mut rt, Section::Photos);
    press(&mut rt, Key::Command(','), ctrl());
    engine.set_account("user-b");
    rt.update(|s, _| assert!(s.shell.refresh(), "the screen changed"));
    assert_eq!(rt.state.shell.app.account(), Some("user-b"));
    assert_eq!(
        rt.state.shell.app.screen(),
        Screen::Drive(Route::section(Section::Files))
    );
    assert!(!rt.state.shell.app.can_go_back());
    rt.update(|s, _| assert!(!s.shell.refresh(), "and stays put for the same account"));
}

/// P2-4. What the control channel may ask: show, a link (navigation only), quit.
#[test]
fn a_control_request_shows_navigates_or_quits_and_nothing_else() {
    let (engine, _, mut rt) = fixture();
    rt.update(|s, tx| on_control(s, Request::Show, tx));
    assert_eq!(
        rt.state.shell.app.screen(),
        Screen::Drive(Route::section(Section::Files))
    );
    rt.update(|s, tx| on_control(s, Request::Open(crate::shell::deeplink::link("open/photos")), tx));
    assert_eq!(
        rt.state.shell.app.screen(),
        Screen::Drive(Route::section(Section::Photos))
    );
    // A link that is not on the list (it cannot come out of `Request::decode`, and is still
    // checked again here) changes nothing.
    rt.update(|s, tx| on_control(s, Request::Open(crate::shell::deeplink::link("quit")), tx));
    assert_eq!(
        rt.state.shell.app.screen(),
        Screen::Drive(Route::section(Section::Photos))
    );
    assert!(engine.calls().is_empty(), "no request reaches the engine by itself");
    // quit-for-update is the normal quit: the hook is what stops the engine.
    rt.update(|s, tx| on_control(s, Request::QuitForUpdate, tx));
    rt.update(on_quit);
    assert_eq!(engine.calls(), vec![FixtureCall::StopForQuit]);
}

/// The first-run window: signed out and the first-run gate belong to it; the one window takes
/// its size when the kind changes, and its place is never saved as the main window's.
#[test]
fn the_window_becomes_the_first_run_window_and_back() {
    let (engine, _, mut rt) = fixture();
    assert_eq!(rt.state.shell.window_kind(), surfaces::WindowKind::Main);
    engine.set_gate(GateState::FirstRun);
    rt.update(|s, _| assert!(s.shell.refresh()));
    settle(&mut rt);
    assert_eq!(registry::group_for(&rt.state.shell.app), FrameGroup::FirstRun);
    assert_eq!(rt.state.shell.window_kind(), surfaces::WindowKind::FirstRun);
    assert!(texts(&rt).contains(&"First run".to_owned()), "the honest placeholder");
    assert!(
        rt.find("file-list").is_none(),
        "nothing of the account is drawn behind the gate"
    );
    engine.set_gate(GateState::SignedOut);
    rt.update(|s, _| assert!(s.shell.refresh()));
    settle(&mut rt);
    assert_eq!(rt.state.shell.window_kind(), surfaces::WindowKind::FirstRun);
    engine.set_gate(GateState::Ready);
    rt.update(|s, _| assert!(s.shell.refresh()));
    settle(&mut rt);
    assert_eq!(rt.state.shell.window_kind(), surfaces::WindowKind::Main);
    // A shell that starts signed out starts as the first-run window.
    let shell = Shell::fixture_on(Arc::new(FixtureEngine::signed_out()), Arc::new(FakePlatform::default()));
    assert_eq!(shell.window_kind(), surfaces::WindowKind::FirstRun);
}

/// P2-9. Starting sync goes through the action that refuses while the old app holds the lock.
#[test]
fn the_shell_does_not_start_sync_over_another_agents_lock() {
    let engine = Arc::new(FixtureEngine::ready());
    engine.set_profile_holder(Some(crate::shell::instance::ProfileHolder {
        agent: "desktop".into(),
        pid: 4242,
    }));
    let shell = Shell::fixture_on(engine.clone(), Arc::new(FakePlatform::default()));
    shell.start_sync();
    assert!(engine.calls().is_empty());
    engine.set_profile_holder(None);
    shell.start_sync();
    assert_eq!(engine.calls(), vec![FixtureCall::StartSync]);
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

/// Review of #334, P2-2: a hidden flyout is not read. Each read the shell asks of the flyout is
/// one read of the engine and of this PC's state database when Rune runs it; over 20 changes of
/// the b's inputs while the flyout is hidden the shell asks for none, and the click that opens it
/// asks for one, before it opens. (A flyout that shows is not reached here: only Rune's window
/// host says that a window shows.)
#[test]
fn a_hidden_flyout_is_not_read_and_the_click_that_opens_it_reads_it_once() {
    let engine = Arc::new(FixtureEngine::ready());
    let platform = Arc::new(FakePlatform::default());
    let flyout = rune::App::new(rune::AppSpec::new(
        "flyout",
        WindowKind::Flyout.default_size(),
        flyout_window::fixture_state,
        flyout_window::view,
    ))
    .handle();
    let wiring = Wiring {
        flyout: Some(flyout.clone()),
        ..Wiring::default()
    };
    let actions = Arc::new(HostActions::new(engine.clone(), platform.clone()));
    let mut shell = Shell::new(actions, platform, wiring);
    assert!(!flyout.is_visible(), "never shown");
    for files in 1..=20 {
        engine.set_inputs(TrayInputs {
            signed_in: true,
            explorer_ready: true,
            files_remaining: Some(files),
            ..TrayInputs::default()
        });
        shell.refresh();
    }
    assert_eq!(shell.tray_model().label, "Syncing", "the b followed the changes");
    assert_eq!(shell.flyout_reads, 0, "20 changes while the flyout is hidden: no read");
    shell.toggle_flyout(None);
    assert_eq!(shell.flyout_reads, 1, "the click that opens it reads it, once");
}

/// The flyout draws the tray lane's frames (`registry::FLYOUT`), not the placeholder, in the
/// state the b shows. Its own tests are `frames/tray/tests.rs`.
#[test]
fn the_flyout_draws_the_tray_lanes_frame_in_the_state_the_b_shows() {
    let mut rt = Runtime::new(flyout_window::fixture_state(), flyout_window::view, (340.0, 520.0), ENV);
    rt.frame(0.0);
    assert!(rt.find("fly").is_some(), "the tray lane's flyout");
    assert!(rt.find("flyout").is_none(), "not the placeholder");
    assert_eq!(rt.state.model.label, "Up to date");
    let texts: Vec<String> = rt.elements().into_iter().filter_map(|e| e.text).collect();
    assert!(texts.iter().any(|t| t == "Up to date"), "{texts:?}");
    assert!(!texts.iter().any(|t| t == "Flyout"), "{texts:?}");
}

/// The real engine's wiring in the poll: what the sign-in screens queued is taken on the next frame (not
/// left to grow), and a gate that opens reads the folder again from the root and asks for sync, once.
#[test]
fn the_poll_drains_the_sign_in_queue_and_a_gate_that_opens_reloads_and_starts_sync() {
    use crate::frames::signin::Intent as SignIn;
    let engine = Arc::new(FixtureEngine::signed_out());
    let platform = Arc::new(FakePlatform::default());
    let mut rt = runtime(state::init_with(Shell::fixture_on(engine.clone(), platform)));
    let starts = |engine: &FixtureEngine| engine.calls().iter().filter(|c| **c == FixtureCall::StartSync).count();
    rt.update(|s, _| {
        s.selected = Some(2);
        s.signin
            .as_mut()
            .expect("the sign-in screens")
            .leave(SignIn::CreateAccount);
    });
    rt.frame(1.0);
    rt.update(|s, _| {
        assert!(
            s.signin.as_mut().unwrap().take_intents().is_empty(),
            "the poll took the intent"
        );
        assert!(!drain_signin_intents(s), "nothing left to take");
        assert_eq!(s.selected, Some(2), "no gate change, nothing reloaded");
    });
    assert_eq!(starts(&engine), 0, "signed out: sync is not asked for");

    engine.set_gate(GateState::Ready);
    rt.frame(2.0);
    rt.update(|s, _| {
        assert_eq!(s.shell.app.gate(), GateState::Ready);
        assert_eq!(s.selected, None, "the folder was read again under the new gate");
        assert!(s.list_error.is_none());
    });
    assert_eq!(starts(&engine), 1);
    rt.frame(3.0);
    assert_eq!(starts(&engine), 1, "once per change of the gate, not once per frame");

    engine.set_gate(GateState::SignedOut);
    rt.frame(4.0);
    assert_eq!(starts(&engine), 1, "a gate that closes starts nothing");
}

// ---------------------------------------------------------------------------
// Capture exclusion (task 2460): the window is excluded from screen capture
// while a secret route is on screen. The FakePlatform now receives the call
// directly (with `window: None` in headless tests); it records every call
// and can simulate failure.
// ---------------------------------------------------------------------------

/// P1-1 fix: an app that starts on sign-in applies capture exclusion on the first poll.
/// `capture_excluded` starts `false` ("platform not told"); the first poll sees the secret
/// route and calls the platform.
#[test]
fn startup_on_sign_in_applies_capture_exclusion_on_the_first_poll() {
    let engine = Arc::new(FixtureEngine::signed_out());
    let platform = Arc::new(FakePlatform::default());
    let mut rt = runtime(state::init_with(Shell::fixture_on(engine.clone(), platform.clone())));
    // After the first frame (runtime() calls frame(0.0)), the platform was told "exclude".
    let calls = platform.capture_exclusions.lock().unwrap().clone();
    assert_eq!(calls, vec![true], "exactly one exclude call on startup");
    assert!(
        rt.state.shell.capture_excluded,
        "flag tracks what the platform was told"
    );

    // A second frame with the same state makes no call.
    rt.frame(1.0);
    let calls = platform.capture_exclusions.lock().unwrap().clone();
    assert_eq!(calls, vec![true], "no duplicate call");
}

/// Entering a secret route from a non-secret route calls the platform with `true`; leaving
/// calls it with `false`.
#[test]
fn entering_a_secret_route_sets_capture_exclusion_and_leaving_clears_it() {
    let engine = Arc::new(FixtureEngine::ready());
    let platform = Arc::new(FakePlatform::default());
    let mut rt = runtime(state::init_with(Shell::fixture_on(engine.clone(), platform.clone())));
    // Starts on Files (non-secret): capture_excluded starts false, desired is false → no call.
    assert!(!rt.state.shell.capture_excluded);
    assert!(platform.capture_exclusions.lock().unwrap().is_empty());

    // Gate → SignedOut → sign-in (secret).
    engine.set_gate(GateState::SignedOut);
    rt.frame(1.0);
    assert_eq!(
        platform.capture_exclusions.lock().unwrap().clone(),
        vec![true],
        "entering sign-in calls the platform with exclude"
    );
    assert!(rt.state.shell.capture_excluded);

    // Gate → Ready → Files (non-secret).
    engine.set_gate(GateState::Ready);
    rt.frame(2.0);
    assert_eq!(
        platform.capture_exclusions.lock().unwrap().clone(),
        vec![true, false],
        "leaving sign-in calls the platform with clear"
    );
    assert!(!rt.state.shell.capture_excluded);
}

/// Secret-to-secret (sign-in → locked) keeps exclusion: no false→true gap.
#[test]
fn secret_to_secret_keeps_exclusion_without_a_gap() {
    let engine = Arc::new(FixtureEngine::signed_out());
    let platform = Arc::new(FakePlatform::default());
    let mut rt = runtime(state::init_with(Shell::fixture_on(engine.clone(), platform.clone())));
    // The first poll applied exclude (startup on sign-in).
    let before = platform.capture_exclusions.lock().unwrap().len();
    assert_eq!(before, 1, "one call on startup");

    // Sign-in → locked (both secret).
    engine.set_gate(GateState::Locked);
    rt.frame(1.0);
    let after = platform.capture_exclusions.lock().unwrap().len();
    assert_eq!(after, before, "no transition call when both routes are secret");
    assert!(rt.state.shell.capture_excluded);
}

/// P2-1: the failure path is actually exercised. When the platform call fails, the flag stays
/// `false` (the platform was NOT told), and content stays drawn.
#[test]
fn a_failed_platform_call_keeps_the_flag_false_and_content_draws() {
    use std::sync::atomic::Ordering;
    let engine = Arc::new(FixtureEngine::ready());
    let platform = Arc::new(FakePlatform::default());
    platform.capture_exclusion_fails.store(true, Ordering::SeqCst);
    let mut rt = runtime(state::init_with(Shell::fixture_on(engine.clone(), platform.clone())));
    // Starts on Files, desired false, applied false → no call.
    assert!(!rt.state.shell.capture_excluded);

    // Enter sign-in (secret): the platform call fails.
    engine.set_gate(GateState::SignedOut);
    rt.frame(1.0);
    // The flag stays false (the platform was NOT successfully told).
    assert!(!rt.state.shell.capture_excluded, "flag stays false on failure");
    assert!(
        rt.state.shell.capture_exclusion_retries > 0,
        "retry counter incremented"
    );
    // Content stays drawn.
    assert_eq!(rt.state.shell.app.screen(), Screen::SignIn);
    // The platform was called (and failed): no successful record.
    assert!(platform.capture_exclusions.lock().unwrap().is_empty());
}

/// P2-2: after a failed call, subsequent polls retry (bounded at 3).
#[test]
fn after_failure_subsequent_polls_retry_bounded() {
    use std::sync::atomic::Ordering;
    let engine = Arc::new(FixtureEngine::ready());
    let platform = Arc::new(FakePlatform::default());
    platform.capture_exclusion_fails.store(true, Ordering::SeqCst);
    let mut rt = runtime(state::init_with(Shell::fixture_on(engine.clone(), platform.clone())));

    // Enter sign-in: first call fails.
    engine.set_gate(GateState::SignedOut);
    rt.frame(1.0);
    assert_eq!(rt.state.shell.capture_exclusion_retries, 1);

    // Second and third poll: still failing.
    rt.frame(2.0);
    assert_eq!(rt.state.shell.capture_exclusion_retries, 2);
    rt.frame(3.0);
    assert_eq!(rt.state.shell.capture_exclusion_retries, 3);

    // Fourth poll: retries exhausted, no more calls.
    rt.frame(4.0);
    assert_eq!(rt.state.shell.capture_exclusion_retries, 3, "capped at 3");
    assert!(!rt.state.shell.capture_excluded, "still not applied");

    // Fix the platform: the user navigates away and back, resetting the counter.
    platform.capture_exclusion_fails.store(false, Ordering::SeqCst);
    engine.set_gate(GateState::Ready);
    rt.frame(5.0);
    // Desired false, applied false → match → retries reset.
    assert_eq!(rt.state.shell.capture_exclusion_retries, 0);

    // Re-enter sign-in: now the call succeeds.
    engine.set_gate(GateState::SignedOut);
    rt.frame(6.0);
    assert!(rt.state.shell.capture_excluded, "applied after recovery");
    assert_eq!(
        platform.capture_exclusions.lock().unwrap().clone(),
        vec![true],
        "one successful call"
    );
}

/// `route_shows_secret` checks both the frame behind and the dialog in front.
#[test]
fn route_shows_secret_checks_the_dialog_layer_too() {
    assert!(!FrameGroup::ConflictDialog.shows_secret());
    assert!(!FrameGroup::ShortcutsDialog.shows_secret());
    assert!(FrameGroup::SignIn.shows_secret());
    assert!(FrameGroup::Locked.shows_secret());
    assert!(FrameGroup::RecoveryPhrase.shows_secret());
    assert!(FrameGroup::FirstRun.shows_secret());
}

/// Per-pane and per-dialog secret classification within Settings.
#[test]
fn the_account_panes_change_password_and_2fa_setup_are_secret() {
    use crate::frames::account::{CodeFor, Dialog as AccountDialog, Pane};
    assert!(Pane::Email.shows_secret(), "DW-A65");
    assert!(Pane::Password.shows_secret(), "DW-A67");
    assert!(Pane::TwoFactorSetup.shows_secret(), "DW-A69");
    assert!(Pane::BackupCodes.shows_secret(), "DW-A70");
    assert!(Pane::Delete.shows_secret(), "DW-A58");
    assert!(Pane::DeleteFactor.shows_secret(), "DW-A80");
    assert!(AccountDialog::Code(CodeFor::TurnOffTwoFactor).shows_secret(), "DW-A72");
    assert!(!Pane::Account.shows_secret(), "DW-A51");
    assert!(!Pane::Security.shows_secret(), "DW-A52");
    assert!(!Pane::Checkup.shows_secret(), "DW-A84");
    assert!(!Pane::Sessions.shows_secret(), "DW-A55");
    assert!(!Pane::Name.shows_secret(), "DW-A64");
    assert!(!Pane::EmailCode.shows_secret(), "DW-A66");
    assert!(!Pane::TwoFactor.shows_secret(), "DW-A71");
    assert!(!Pane::Passkeys.shows_secret(), "DW-A73");
    assert!(!Pane::Activity.shows_secret(), "DW-A77");
    assert!(!Pane::DeleteLink.shows_secret(), "DW-A81");
    assert!(!Pane::DeleteScheduled.shows_secret(), "DW-A82");
    assert!(!Pane::DeleteUndone.shows_secret(), "DW-A83");
    assert!(!AccountDialog::SignOutApp.shows_secret(), "DW-A57");
    assert!(!AccountDialog::LeaveCodes.shows_secret());
    // The plan (size, use, renewal day) and App lock's delay menu hold no key material.
    assert!(!AccountDialog::Plan.shows_secret(), "DW-0708");
    assert!(!AccountDialog::LockDelay(Some(1)).shows_secret(), "DW-0716");
}

/// DW-0401 to DW-1201: the window's status line shows the b's flyout, as a click on the b does.
/// The click asks the flyout to read the engine once and open (that a window shows is Rune's
/// window host's to say, as above); the Search window's status line does the same.
#[test]
fn the_status_line_of_the_window_opens_the_flyout() {
    let shell_with_flyout = || {
        let flyout = rune::App::new(rune::AppSpec::new(
            "flyout",
            WindowKind::Flyout.default_size(),
            flyout_window::fixture_state,
            flyout_window::view,
        ))
        .handle();
        let wiring = Wiring {
            flyout: Some(flyout),
            ..Wiring::default()
        };
        let platform = Arc::new(FakePlatform::default());
        let actions = Arc::new(HostActions::new(Arc::new(FixtureEngine::ready()), platform.clone()));
        Shell::new(actions, platform, wiring)
    };
    let click_status = |rt: &mut Runtime<State>| {
        let r = rt
            .elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some("status-text") && !e.exiting)
            .expect("the status line")
            .rect;
        let (x, y) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        settle(rt);
    };
    // Files.
    let mut rt = runtime(state::init_with(shell_with_flyout()));
    assert_eq!(rt.state.shell.flyout_reads, 0);
    click_status(&mut rt);
    assert_eq!(
        rt.state.shell.flyout_reads, 1,
        "the click reads the flyout and opens it"
    );
    // Search (the Files window while searching).
    let mut state = state::init_with(shell_with_flyout());
    crate::frames::search::dw_0802(&mut state);
    let mut rt = runtime(state);
    click_status(&mut rt);
    assert_eq!(rt.state.shell.flyout_reads, 1);
}

// ---------------------------------------------------------------------------
// Snapshot gate: scenes that set a non-Ready gate must survive the poll's
// refresh, which reads the fixture engine. Regression d6506fb7.
// ---------------------------------------------------------------------------

/// The snapshot path: a scene sets the gate to SignedOut, poll runs, and the gate stays
/// SignedOut. Before the fix, `refresh()` read `Ready` from the fixture engine and flipped
/// the sign-in gate to Files.
#[test]
fn a_scene_gate_survives_the_polls_refresh() {
    let engine = Arc::new(FixtureEngine::ready());
    let platform = Arc::new(FakePlatform::default());
    let mut rt = runtime(state::init_with(Shell::fixture_on(engine.clone(), platform)));
    assert_eq!(
        rt.state.shell.app.screen(),
        Screen::Drive(Route::section(Section::Files))
    );
    // Apply a scene that sets the gate to SignedOut (the sign-in scene's mechanism).
    rt.update(|s, _| {
        dispatch(s, Event::Gate(GateState::SignedOut));
    });
    assert_eq!(rt.state.shell.app.screen(), Screen::SignIn);
    // Run two frames: the poll's refresh must not flip the gate back to Ready.
    rt.frame(1.0);
    assert_eq!(
        rt.state.shell.app.screen(),
        Screen::SignIn,
        "the scene's SignedOut gate must survive the first poll"
    );
    rt.frame(2.0);
    assert_eq!(
        rt.state.shell.app.screen(),
        Screen::SignIn,
        "the scene's SignedOut gate must survive the second poll"
    );
    // The app machine's gate is still SignedOut (the fixture engine says so too).
    assert_eq!(rt.state.shell.app.gate(), GateState::SignedOut);
    // The sidebar must not be drawn behind the gate.
    assert!(rt.find("sidebar").is_none(), "no sidebar behind sign-in");
    assert!(rt.find("file-list").is_none(), "no file list behind sign-in");
}

/// Every gate scene keeps its gate through two frames of poll.
#[test]
fn every_gate_scene_survives_two_polls() {
    for (gate, name) in [
        (GateState::SignedOut, "signed-out"),
        (GateState::Locked, "locked"),
        (GateState::NoPlan, "no-plan"),
        (GateState::FirstRun, "first-run"),
    ] {
        let engine = Arc::new(FixtureEngine::ready());
        let platform = Arc::new(FakePlatform::default());
        let mut rt = runtime(state::init_with(Shell::fixture_on(engine, platform)));
        rt.update(|s, _| {
            dispatch(s, Event::Gate(gate));
        });
        rt.frame(1.0);
        rt.frame(2.0);
        assert_ne!(
            rt.state.shell.app.screen(),
            Screen::Drive(Route::section(Section::Files)),
            "scene {name}: Files must not show behind a {gate:?} gate"
        );
        assert!(
            rt.find("file-list").is_none(),
            "scene {name}: no file list behind a {gate:?} gate"
        );
    }
}

/// The guard: with an engine that is the authority (the live-engine stand-in), dispatching
/// `Event::Gate(Ready)` and polling does NOT show Files. This proves that in the real app
/// the engine's gate cannot be overridden by a dispatch.
#[test]
fn dispatching_gate_ready_does_not_override_the_engines_authority() {
    for gate in [GateState::Locked, GateState::SignedOut, GateState::NeedsRecoveryPhrase] {
        let engine = Arc::new(FixtureEngine::live_stand_in(gate));
        let platform = Arc::new(FakePlatform::default());
        let mut rt = runtime(state::init_with(Shell::fixture_on(engine.clone(), platform)));
        // The engine says `gate`; dispatch tries to set Ready.
        rt.update(|s, _| {
            dispatch(s, Event::Gate(GateState::Ready));
        });
        // The dispatch set the app machine to Ready momentarily, but the very next poll's
        // refresh reads the engine (still `gate`) and overrides it.
        rt.frame(1.0);
        assert_eq!(
            rt.state.shell.app.gate(),
            gate,
            "the engine's {gate:?} must win over a dispatched Ready"
        );
        assert_ne!(
            rt.state.shell.app.screen(),
            Screen::Drive(Route::section(Section::Files)),
            "Files must not show when the engine says {gate:?}"
        );
        assert!(
            rt.find("file-list").is_none(),
            "no file list when the engine says {gate:?}"
        );
    }
}
