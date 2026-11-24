use super::*;

#[test]
fn a_mac_keeps_running_when_its_last_window_closes_and_quits_when_asked() {
    assert!(keeps_running(true, None));
    assert!(!keeps_running(true, Some(0)));
    assert!(!keeps_running(false, None));
}

#[test]
fn the_dock_icon_follows_the_beebeeb_window_and_changes_only_when_it_changes() {
    assert!(dock_icon_wanted(true));
    assert!(!dock_icon_wanted(false));
    assert_eq!(dock_change(None, true), Some(true));
    assert_eq!(dock_change(Some(true), true), None);
    assert_eq!(dock_change(Some(true), false), Some(false));
    assert_eq!(dock_change(Some(false), false), None);
}

#[test]
fn opening_a_window_on_a_mac_unhides_the_app_first() {
    assert_eq!(open_steps(true, true)[0], Step::UnhideApp);
    assert_eq!(open_steps(false, false), vec![Step::ShowWindow]);
}

#[test]
fn opening_the_window_turns_the_dock_icon_on_before_it_shows_and_activates_after() {
    assert_eq!(
        open_steps(true, false),
        vec![Step::UnhideApp, Step::DockIconOn, Step::ShowWindow, Step::ActivateApp]
    );
    assert_eq!(
        open_steps(true, true),
        vec![Step::UnhideApp, Step::ShowWindow, Step::ActivateApp]
    );
}

#[test]
fn a_reopen_shows_the_beebeeb_window() {
    assert_eq!(reopen(true), Reopen::BringForward);
    assert_eq!(reopen(false), Reopen::OpenOnHome);
}

#[test]
fn the_tray_hide_hides_the_app_on_a_mac_and_the_window_elsewhere() {
    assert_eq!(hide_kind(true), HideKind::App);
    assert_eq!(hide_kind(false), HideKind::Window);
}

#[test]
fn a_manual_launch_opens_the_window_and_a_login_launch_shows_nothing() {
    assert_eq!(start_window(Launch::Manual), StartWindow::Open);
    assert_eq!(start_window(Launch::LoginItem), StartWindow::None);
    assert!(starts_accessory(start_window(Launch::LoginItem)));
    assert!(!starts_accessory(start_window(Launch::Manual)));
}

#[test]
fn an_updater_relaunch_shows_what_the_person_had_open_and_nothing_else() {
    let r = |signed_in, window_was_open| {
        start_window(Launch::UpdaterRelaunch {
            signed_in,
            window_was_open,
        })
    };
    assert_eq!(r(true, true), StartWindow::Open);
    assert_eq!(r(true, false), StartWindow::None);
    assert_eq!(r(false, false), StartWindow::Open);
    assert_eq!(r(false, true), StartWindow::Open);
}
