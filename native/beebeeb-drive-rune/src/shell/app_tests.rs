use super::*;

fn files() -> Route {
    Route::section(Section::Files)
}
fn ready() -> AppMachine {
    AppMachine::new(GateState::Ready)
}
fn go(m: &mut AppMachine, route: Route) -> Outcome {
    m.apply(Event::Navigate(route))
}

#[test]
fn starts_on_files_with_no_history() {
    let m = ready();
    assert_eq!(m.screen(), Screen::Drive(files()));
    assert!(!m.can_go_back());
    assert!(!m.can_go_forward());
}

#[test]
fn every_section_is_reachable_and_names_itself() {
    let mut m = ready();
    for section in Section::ALL {
        go(&mut m, Route::section(section));
        assert_eq!(m.screen(), Screen::Drive(Route::section(section)));
        assert_eq!(Section::from_slug(section.slug()), Some(section));
        assert!(!section.label().is_empty());
    }
    // Files was the start, so six moves were remembered.
    assert_eq!(m.back.len(), 6);
    for pane in SettingsPane::ALL {
        assert_eq!(SettingsPane::from_slug(pane.slug()), Some(pane));
    }
    assert_eq!(Section::from_slug("Files"), None);
    assert_eq!(SettingsPane::from_slug(""), None);
}

#[test]
fn navigating_to_where_the_window_is_changes_nothing() {
    let mut m = ready();
    assert_eq!(go(&mut m, files()), Outcome::Unchanged);
    assert!(!m.can_go_back());
}

#[test]
fn back_and_forward_walk_the_history() {
    let mut m = ready();
    let docs = Route::folder(Section::Files, "f-docs");
    let photos = Route::section(Section::Photos);
    go(&mut m, docs.clone());
    go(&mut m, photos.clone());
    assert_eq!(m.apply(Event::Back), Outcome::Changed);
    assert_eq!(m.route(), &docs);
    assert!(m.can_go_forward());
    assert_eq!(m.apply(Event::Back), Outcome::Changed);
    assert_eq!(m.route(), &files());
    assert_eq!(m.apply(Event::Back), Outcome::Unchanged);
    assert_eq!(m.apply(Event::Forward), Outcome::Changed);
    assert_eq!(m.apply(Event::Forward), Outcome::Changed);
    assert_eq!(m.route(), &photos);
    assert_eq!(m.apply(Event::Forward), Outcome::Unchanged);
}

#[test]
fn a_new_place_after_back_drops_the_forward_branch() {
    let mut m = ready();
    go(&mut m, Route::section(Section::Shared));
    m.apply(Event::Back);
    assert!(m.can_go_forward());
    go(&mut m, Route::section(Section::Trash));
    assert!(!m.can_go_forward());
    m.apply(Event::Back);
    assert_eq!(m.route(), &files());
}

#[test]
fn history_is_bounded() {
    let mut m = ready();
    for i in 0..(HISTORY_LIMIT + 25) {
        go(&mut m, Route::folder(Section::Files, format!("f{i}")));
    }
    assert_eq!(m.back.len(), HISTORY_LIMIT);
    // The oldest places fell off, the newest are intact.
    assert_eq!(m.back.last().unwrap().folder.as_deref(), Some("f123"));
    assert_eq!(m.back.first().unwrap().folder.as_deref(), Some("f24"));
}

#[test]
fn settings_is_a_mode_over_the_route_and_esc_or_back_leaves_it() {
    let mut m = ready();
    go(&mut m, Route::section(Section::Photos));
    assert_eq!(m.apply(Event::OpenSettings(SettingsPane::General)), Outcome::Changed);
    assert_eq!(m.screen(), Screen::Settings(SettingsPane::General));
    assert_eq!(m.apply(Event::OpenSettings(SettingsPane::General)), Outcome::Unchanged);
    assert_eq!(m.apply(Event::OpenSettings(SettingsPane::Security)), Outcome::Changed);
    assert!(m.can_go_back());
    assert!(!m.can_go_forward());
    assert_eq!(m.apply(Event::Forward), Outcome::Unchanged);
    assert_eq!(m.apply(Event::Escape), Outcome::Changed);
    assert_eq!(m.screen(), Screen::Drive(Route::section(Section::Photos)));
    m.apply(Event::OpenSettings(SettingsPane::Storage));
    assert_eq!(m.apply(Event::Back), Outcome::Changed);
    // Back left settings; it did not also step the Drive history.
    assert_eq!(m.screen(), Screen::Drive(Route::section(Section::Photos)));
    // A sidebar click from settings mode goes to that section.
    m.apply(Event::OpenSettings(SettingsPane::Account));
    assert_eq!(go(&mut m, Route::section(Section::Photos)), Outcome::Changed);
    assert_eq!(m.screen(), Screen::Drive(Route::section(Section::Photos)));
}

#[test]
fn opening_an_item_is_a_step_and_esc_returns_to_its_folder() {
    let mut m = ready();
    let docs = Route::folder(Section::Files, "f-docs");
    go(&mut m, docs.clone());
    assert_eq!(m.apply(Event::OpenItem("i-contract".into())), Outcome::Changed);
    assert_eq!(m.route(), &docs.with_item("i-contract"));
    assert_eq!(m.apply(Event::OpenItem("i-contract".into())), Outcome::Unchanged);
    assert_eq!(m.apply(Event::Escape), Outcome::Changed);
    assert_eq!(m.route(), &docs);
    // Esc stepped back, so Forward opens the item again.
    assert!(m.can_go_forward());
    assert_eq!(m.apply(Event::Escape), Outcome::Unchanged);
    // Settings has no viewer.
    m.apply(Event::OpenSettings(SettingsPane::General));
    assert_eq!(m.apply(Event::OpenItem("x".into())), Outcome::Refused);
}

#[test]
fn esc_on_an_item_reached_from_outside_goes_to_its_folder() {
    let mut m = ready();
    let item = Route::folder(Section::Shared, "f1").with_item("i1");
    m.apply(Event::Request(Intent::Go(item)));
    assert_eq!(m.apply(Event::Escape), Outcome::Changed);
    assert_eq!(m.route(), &Route::folder(Section::Shared, "f1"));
}

#[test]
fn a_dialog_is_modal_and_esc_closes_it_first() {
    let mut m = ready();
    m.apply(Event::OpenSettings(SettingsPane::General));
    assert_eq!(m.apply(Event::OpenDialog(Dialog::Shortcuts)), Outcome::Changed);
    assert_eq!(m.dialog(), Some(&Dialog::Shortcuts));
    assert_eq!(go(&mut m, Route::section(Section::Trash)), Outcome::Refused);
    assert_eq!(m.apply(Event::Back), Outcome::Refused);
    assert_eq!(m.apply(Event::OpenSettings(SettingsPane::Account)), Outcome::Refused);
    assert!(!m.can_go_back());
    assert_eq!(m.apply(Event::Escape), Outcome::Changed);
    assert_eq!(m.dialog(), None);
    assert_eq!(m.screen(), Screen::Settings(SettingsPane::General));
    assert_eq!(m.apply(Event::Escape), Outcome::Changed);
    assert_eq!(m.screen(), Screen::Drive(files()));
}

#[test]
fn signed_out_shows_only_sign_in_and_refuses_everything_else() {
    let mut m = AppMachine::new(GateState::SignedOut);
    assert_eq!(m.screen(), Screen::SignIn);
    for event in [
        Event::Navigate(Route::section(Section::Photos)),
        Event::OpenItem("i".into()),
        Event::Back,
        Event::Forward,
        Event::OpenSettings(SettingsPane::General),
        Event::OpenDialog(Dialog::Shortcuts),
        Event::Escape,
    ] {
        assert_eq!(m.apply(event.clone()), Outcome::Refused, "{event:?}");
        assert_eq!(m.screen(), Screen::SignIn);
    }
    assert!(!m.can_go_back());
}

#[test]
fn each_not_ready_gate_has_its_own_screen() {
    for gate in [
        GateState::Locked,
        GateState::NeedsRecoveryPhrase,
        GateState::NoPlan,
        GateState::KeyStoreUnavailable,
    ] {
        let m = AppMachine::new(gate);
        assert_eq!(m.screen(), Screen::Gate(gate));
    }
}

#[test]
fn locking_mid_navigation_hides_the_account_and_unlocking_returns_to_the_same_place() {
    let mut m = ready();
    let place = Route::folder(Section::Files, "f-docs").with_item("i1");
    go(&mut m, Route::folder(Section::Files, "f-docs"));
    go(&mut m, place.clone());
    m.apply(Event::OpenDialog(Dialog::Conflict {
        file: Some("i1".into()),
    }));
    assert_eq!(m.apply(Event::Gate(GateState::Locked)), Outcome::Changed);
    assert_eq!(m.screen(), Screen::Gate(GateState::Locked));
    assert_eq!(m.dialog(), None, "no dialog shows through the lock");
    assert!(!m.can_go_back());
    assert_eq!(m.apply(Event::Back), Outcome::Refused);
    assert_eq!(m.apply(Event::Gate(GateState::Locked)), Outcome::Unchanged);
    assert_eq!(m.apply(Event::Gate(GateState::Ready)), Outcome::Changed);
    assert_eq!(m.screen(), Screen::Drive(place));
    assert_eq!(
        m.dialog(),
        Some(&Dialog::Conflict {
            file: Some("i1".into())
        })
    );
    m.apply(Event::Escape);
    assert!(m.can_go_back());
}

#[test]
fn signing_out_mid_navigation_forgets_the_place_the_history_and_the_dialog() {
    let mut m = ready();
    go(&mut m, Route::folder(Section::Files, "f-docs"));
    go(&mut m, Route::section(Section::Photos));
    m.apply(Event::Back);
    m.apply(Event::OpenSettings(SettingsPane::Account));
    m.apply(Event::OpenDialog(Dialog::Shortcuts));
    assert_eq!(m.apply(Event::Gate(GateState::SignedOut)), Outcome::Changed);
    assert_eq!(m.screen(), Screen::SignIn);
    // The next account starts clean.
    m.apply(Event::Gate(GateState::Ready));
    assert_eq!(m.screen(), Screen::Drive(files()));
    assert_eq!(m.dialog(), None);
    assert!(!m.can_go_back());
    assert!(!m.can_go_forward());
}

#[test]
fn a_request_while_locked_waits_for_the_unlock() {
    let mut m = AppMachine::new(GateState::Locked);
    let target = Route::folder(Section::Shared, "f9");
    assert_eq!(m.apply(Event::Request(Intent::Go(target.clone()))), Outcome::Deferred);
    assert_eq!(m.screen(), Screen::Gate(GateState::Locked));
    // The latest request wins.
    assert_eq!(
        m.apply(Event::Request(Intent::Settings(SettingsPane::Backups))),
        Outcome::Deferred
    );
    m.apply(Event::Gate(GateState::Ready));
    assert_eq!(m.screen(), Screen::Settings(SettingsPane::Backups));
    assert_eq!(m.pending(), None);
}

#[test]
fn a_request_while_signed_out_is_kept_only_when_it_names_no_account_data() {
    let mut m = AppMachine::new(GateState::SignedOut);
    assert_eq!(
        m.apply(Event::Request(Intent::Go(Route::folder(Section::Files, "f1")))),
        Outcome::Refused
    );
    assert_eq!(
        m.apply(Event::Request(Intent::ReviewConflict { file: None })),
        Outcome::Refused
    );
    assert_eq!(m.pending(), None);
    assert_eq!(
        m.apply(Event::Request(Intent::Go(Route::section(Section::Trash)))),
        Outcome::Deferred
    );
    m.apply(Event::Gate(GateState::NeedsRecoveryPhrase));
    assert_eq!(m.screen(), Screen::Gate(GateState::NeedsRecoveryPhrase));
    m.apply(Event::Gate(GateState::Ready));
    assert_eq!(m.screen(), Screen::Drive(Route::section(Section::Trash)));
}

#[test]
fn a_waiting_request_that_names_account_data_dies_with_the_session() {
    let mut m = AppMachine::new(GateState::Locked);
    m.apply(Event::Request(Intent::Go(Route::folder(Section::Files, "f1"))));
    m.apply(Event::Gate(GateState::SignedOut));
    assert_eq!(m.pending(), None);
    m.apply(Event::Gate(GateState::Ready));
    assert_eq!(m.screen(), Screen::Drive(files()));

    let mut m = AppMachine::new(GateState::Locked);
    m.apply(Event::Request(Intent::Settings(SettingsPane::General)));
    m.apply(Event::Gate(GateState::SignedOut));
    assert_eq!(m.pending(), Some(&Intent::Settings(SettingsPane::General)));
}

#[test]
fn a_request_from_outside_replaces_an_open_dialog_and_leaves_settings_for_a_conflict() {
    let mut m = ready();
    m.apply(Event::OpenSettings(SettingsPane::General));
    m.apply(Event::OpenDialog(Dialog::Shortcuts));
    assert_eq!(
        m.apply(Event::Request(Intent::ReviewConflict { file: None })),
        Outcome::Changed
    );
    assert_eq!(m.dialog(), Some(&Dialog::Conflict { file: None }));
    assert_eq!(m.screen(), Screen::Drive(files()));
    // Asking for the place the window is already on still closes the dialog.
    assert_eq!(m.apply(Event::Request(Intent::Go(files()))), Outcome::Changed);
    assert_eq!(m.dialog(), None);
    assert_eq!(m.apply(Event::Request(Intent::Go(files()))), Outcome::Unchanged);
}
