//! Tests of settings inside the window: structure, keyboard, roles and names.
//!
//! Input is scripted: keys and presses go through Rune's `Runtime`, as a person's would.

use super::source::{BoardSettings, Channel, ThemeChoice};
use super::{Dialog, Intent, Menu, MenuKind, Pane};
use crate::shell::app::{Event, Screen, SettingsPane};
use crate::shell::host;
use crate::{State, init, view};
use rune::{Env, Key, KeyEvent, Mods, Runtime};

/// A window showing board frame `frame` (`""`: the Files window, as the app starts).
fn runtime(frame: &str) -> Runtime<State> {
    let mut state = init();
    if !frame.is_empty() {
        assert!(super::scene(frame, &mut state), "{frame} is a built frame");
    }
    let env = Env {
        system_dark: false,
        scale: 2.0,
    };
    let mut rt = Runtime::new(state, view, (958.0, 638.0), env);
    rt.frame(0.0);
    rt
}

fn find(rt: &Runtime<State>, key: &str) -> Option<rune::ElementInfo> {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
}

fn info(rt: &Runtime<State>, key: &str) -> rune::ElementInfo {
    find(rt, key).unwrap_or_else(|| panic!("no element keyed {key}"))
}

fn key(rt: &mut Runtime<State>, key: Key) {
    rt.key(KeyEvent {
        key,
        mods: Mods::default(),
    });
    rt.frame(0.0);
}

fn click(rt: &mut Runtime<State>, key: &str) {
    let r = info(rt, key).rect;
    let (x, y) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(0.0);
}

/// Tab until the element keyed `target` has focus; panics when focus never gets there.
fn tab_to(rt: &mut Runtime<State>, target: &str) {
    for _ in 0..80 {
        if find(rt, target).is_some_and(|e| e.focused) {
            return;
        }
        key(rt, Key::Tab);
    }
    panic!("Tab never reaches {target}");
}

/// Is the window on settings (the shell's route), and on which of the shell's sections?
fn on_settings(rt: &Runtime<State>) -> Option<SettingsPane> {
    match rt.state.shell.app.screen() {
        Screen::Settings(pane) => Some(pane),
        _ => None,
    }
}

#[test]
fn the_shell_opens_settings_on_general_and_esc_goes_back_to_drive() {
    let mut rt = runtime("");
    assert!(find(&rt, "set-main").is_none(), "the window starts on Drive");
    // What Ctrl+, does (`shell::host::on_key`).
    host::dispatch(&mut rt.state, Event::OpenSettings(SettingsPane::General));
    rt.frame(0.0);
    assert_eq!(info(&rt, "set-title").text.as_deref(), Some("General"));
    assert!(
        find(&rt, "toolbar").is_none(),
        "settings takes the place of the Files view"
    );
    assert!(
        find(&rt, "caption-bar").is_some(),
        "in the same window: the caption stays"
    );
    key(&mut rt, Key::Escape);
    assert_eq!(on_settings(&rt), None, "Esc goes back");
    assert!(find(&rt, "toolbar").is_some());
}

/// A sidebar section another lane draws is the shell's to route: this lane asks, the shell shows it.
#[test]
fn a_section_of_another_lane_is_routed_by_the_shell() {
    let mut rt = runtime("DW-0701");
    click(&mut rt, "Storage-row");
    assert_eq!(on_settings(&rt), Some(SettingsPane::Storage));
    assert!(find(&rt, "set-main").is_none(), "this lane does not draw Storage");
}

#[test]
fn back_to_drive_leaves_settings() {
    let mut rt = runtime("DW-0701");
    click(&mut rt, "Back to Drive-row");
    assert_eq!(on_settings(&rt), None);
}

#[test]
fn the_sidebar_is_navigation_of_links_with_the_current_one_selected() {
    let rt = runtime("DW-0701");
    assert_eq!(info(&rt, "settings-nav").role, "navigation");
    let labels = [
        "Back to Drive",
        "General",
        "Account",
        "Security",
        "Devices and sessions",
        "Notifications",
        "Storage",
        "Backups",
        "Sync and File Explorer",
        "Thumbnails and previews",
        "Bandwidth",
    ];
    for label in labels {
        let row = info(&rt, &format!("{label}-row"));
        assert_eq!(row.role, "link", "{label}");
        assert_eq!(row.label.as_deref(), Some(label));
        assert!(row.focusable, "{label} takes focus");
    }
    assert!(info(&rt, "General-row").fill.a > 0.0, "General is selected");
    assert_eq!(info(&rt, "Account-row").fill.a, 0.0);
    assert_eq!(info(&rt, "set-title").role, "heading");
}

/// Every control the General pane draws does something: a pane, a dialog, a menu, or a
/// request to the shell. By pointer and by keyboard.
#[test]
fn every_row_of_general_acts() {
    let asks: [(&str, Intent); 7] = [
        ("item-1-0", Intent::Show("DW-0380")),
        ("item-1-2", Intent::Show("DW-0790")),
        ("item-2-0", Intent::Open("privacy")),
        ("item-2-1", Intent::Open("export-my-data")),
        ("item-3-0", Intent::Web("terms-and-privacy-policy")),
        ("Bandwidth-row", Intent::Show("DW-0785")),
        ("Account-row", Intent::Show("DW-A51")),
    ];
    for (row, intent) in asks {
        let mut rt = runtime("DW-0701");
        assert!(info(&rt, row).focusable, "{row} takes focus");
        click(&mut rt, row);
        assert_eq!(rt.state.settings.intents, vec![intent.clone()], "{row} by pointer");

        let mut rt = runtime("DW-0701");
        tab_to(&mut rt, row);
        key(&mut rt, Key::Enter);
        assert_eq!(rt.state.settings.intents, vec![intent], "{row} by keyboard");
    }
    let shows: [(&str, Pane, Option<Dialog>, &str); 4] = [
        ("item-1-1", Pane::General, Some(Dialog::ReportProblem), "General"),
        ("item-2-2", Pane::StorageLocation, None, "Storage location"),
        ("item-3-1", Pane::About, None, "About Beebeeb"),
        ("Notifications-row", Pane::Notifications, None, "Notifications"),
    ];
    for (row, pane, dialog, title) in shows {
        let mut rt = runtime("DW-0701");
        tab_to(&mut rt, row);
        key(&mut rt, Key::Enter);
        assert_eq!(rt.state.settings.pane, pane, "{row}");
        assert_eq!(rt.state.settings.dialog, dialog, "{row}");
        assert_eq!(info(&rt, "set-title").text.as_deref(), Some(title));
        assert_eq!(on_settings(&rt), Some(super::shell_pane(pane.section())), "{row}");
        assert!(rt.state.settings.intents.is_empty(), "{row} stays in settings");
    }
}

#[test]
fn rows_name_what_they_say() {
    let rt = runtime("DW-0701");
    assert_eq!(
        info(&rt, "item-0-0").label.as_deref(),
        Some("Theme, System. Follows Windows.")
    );
    assert_eq!(info(&rt, "item-0-0").role, "button");
    assert_eq!(info(&rt, "item-2-2").label.as_deref(), Some("Storage location, Europe"));
    assert_eq!(info(&rt, "item-2-2").role, "link");
    assert_eq!(
        info(&rt, "item-3-0").label.as_deref(),
        Some("Terms and privacy policy, opens in the browser")
    );
    assert_eq!(
        info(&rt, "item-3-1").label.as_deref(),
        Some("About Beebeeb, Version 0.9.1")
    );
}

#[test]
fn the_theme_menu_opens_checks_the_choice_and_follows_the_keyboard() {
    let mut rt = runtime("DW-0701");
    assert!(find(&rt, "menu-0").is_none());
    tab_to(&mut rt, "item-0-0");
    key(&mut rt, Key::Enter);
    assert_eq!(
        rt.state.settings.menu,
        Some(Menu {
            kind: MenuKind::Theme,
            hot: 0
        }),
        "opens on the current choice"
    );
    assert_eq!(info(&rt, "menu-0").role, "menu");
    assert!(find(&rt, "menu-0-item-0-check").is_some(), "System is checked");
    assert!(find(&rt, "menu-0-item-1-check").is_none());

    key(&mut rt, Key::Up);
    assert_eq!(
        rt.state.settings.menu,
        Some(Menu {
            kind: MenuKind::Theme,
            hot: 0
        }),
        "Up stops at the first item"
    );
    key(&mut rt, Key::Down);
    key(&mut rt, Key::Down);
    key(&mut rt, Key::Down);
    assert_eq!(
        rt.state.settings.menu,
        Some(Menu {
            kind: MenuKind::Theme,
            hot: 2
        }),
        "Down stops at the last item"
    );
    key(&mut rt, Key::Enter);
    assert_eq!(rt.state.settings.menu, None, "choosing closes the menu");
    assert_eq!(rt.state.settings.source.general().theme, ThemeChoice::Dark);
    assert_eq!(info(&rt, "item-0-0-pop-label").text.as_deref(), Some("Dark"));
    assert_eq!(info(&rt, "item-0-0-sub").text.as_deref(), Some("Always dark."));
    assert!(
        rt.state.settings.dark(false),
        "the choice applies at once, whatever the system says"
    );
}

#[test]
fn esc_closes_the_menu_first_and_settings_second() {
    let mut rt = runtime("DW-0787");
    assert!(find(&rt, "menu-0").is_some());
    key(&mut rt, Key::Escape);
    assert!(find(&rt, "menu-0").is_none(), "the menu closed");
    assert!(on_settings(&rt).is_some(), "settings stayed");
    assert_eq!(
        rt.state.settings.source.general().theme,
        ThemeChoice::System,
        "nothing chosen"
    );
    key(&mut rt, Key::Escape);
    assert_eq!(on_settings(&rt), None);
}

#[test]
fn a_menu_item_is_chosen_by_pointer() {
    let mut rt = runtime("DW-0787");
    click(&mut rt, "menu-0-item-1");
    assert_eq!(rt.state.settings.source.general().theme, ThemeChoice::Light);
    assert_eq!(rt.state.settings.menu, None);
    assert!(!rt.state.settings.dark(true), "Light stays light on a dark system");
}

#[test]
fn an_available_update_is_said_once_with_one_primary_action() {
    let rt = runtime("DW-0701");
    assert!(
        find(&rt, "banner-0").is_none(),
        "no notice when this is the newest version"
    );

    let mut rt = runtime("DW-0781");
    assert_eq!(
        info(&rt, "banner-0-title").text.as_deref(),
        Some("Version 0.9.2 is available")
    );
    assert_eq!(
        info(&rt, "item-3-1-value").text.as_deref(),
        Some("0.9.1 · update available")
    );
    assert_eq!(info(&rt, "btn-0").role, "button");
    tab_to(&mut rt, "btn-0");
    key(&mut rt, Key::Enter);
    assert_eq!(rt.state.settings.intents, vec![Intent::DownloadUpdate]);
    assert_eq!(rt.state.settings.pane, Pane::General);
}

/// Every frame of the table renders settings in the window, with a title, and says nothing
/// it must not: no city, no provider.
#[test]
fn every_built_frame_renders_and_names_no_city_and_no_provider() {
    assert_eq!(super::FRAMES.len(), 15, "the frames this lane built");
    for (frame, _) in super::FRAMES {
        let rt = runtime(frame);
        assert!(find(&rt, "set-main").is_some(), "{frame}");
        assert!(find(&rt, "caption-bar").is_some(), "{frame}: the same window");
        assert_eq!(info(&rt, "set-title").role, "heading", "{frame}");
        for e in rt.elements() {
            let words = format!("{} {}", e.text.unwrap_or_default(), e.label.unwrap_or_default()).to_lowercase();
            for banned in [
                "hetzner",
                "falkenstein",
                "nuremberg",
                "helsinki",
                "frankfurt",
                "amsterdam",
            ] {
                assert!(!words.contains(banned), "{frame} says {banned}: {words}");
            }
        }
    }
}

#[test]
fn a_dialog_is_modal_takes_esc_first_and_its_buttons_act() {
    let mut rt = runtime("DW-0782");
    assert_eq!(info(&rt, "dlg").role, "dialog");
    assert_eq!(info(&rt, "dlg").label.as_deref(), Some("Report a problem"));
    key(&mut rt, Key::Escape);
    assert!(find(&rt, "dlg").is_none(), "Esc closes the dialog");
    assert!(on_settings(&rt).is_some(), "and settings stays");

    let mut rt = runtime("DW-0782");
    click(&mut rt, "btn-1");
    assert_eq!(rt.state.settings.dialog, None, "Cancel closes it");
    assert!(rt.state.settings.intents.is_empty(), "and nothing was saved or sent");

    let mut rt = runtime("DW-0782");
    tab_to(&mut rt, "btn-0");
    key(&mut rt, Key::Enter);
    assert_eq!(
        rt.state.settings.intents,
        vec![Intent::SaveBundleAndWriteEmail, Intent::Show("DW-0790b")]
    );
}

#[test]
fn about_goes_back_copies_checks_and_shows_the_notes() {
    let mut rt = runtime("DW-0751");
    assert_eq!(info(&rt, "crumb-row").role, "link");
    assert!(info(&rt, "General-row").fill.a > 0.0, "About sits under General");
    assert_eq!(info(&rt, "item-0-0-value").text.as_deref(), Some("0.9.1 (311)"));
    tab_to(&mut rt, "btn-0");
    key(&mut rt, Key::Enter);
    tab_to(&mut rt, "btn-1");
    key(&mut rt, Key::Enter);
    assert_eq!(
        rt.state.settings.intents,
        vec![
            Intent::Copy("Beebeeb Drive 0.9.1 (311)\nEncryption core 2084927".into()),
            Intent::CheckForUpdates
        ]
    );
    tab_to(&mut rt, "item-1-2");
    key(&mut rt, Key::Enter);
    assert_eq!(rt.state.settings.dialog, Some(Dialog::ReleaseNotes));
    assert_eq!(info(&rt, "dlg-title").text.as_deref(), Some("Beebeeb 0.9.2"));
    click(&mut rt, "btn-3");
    assert_eq!(rt.state.settings.dialog, None);
    click(&mut rt, "crumb-row");
    assert_eq!(rt.state.settings.pane, Pane::General);
}

#[test]
fn the_release_channel_is_chosen_from_its_menu_by_keyboard() {
    let mut rt = runtime("DW-0751");
    tab_to(&mut rt, "item-1-1");
    key(&mut rt, Key::Enter);
    assert_eq!(
        rt.state.settings.menu,
        Some(Menu {
            kind: MenuKind::Channel,
            hot: 0
        })
    );
    key(&mut rt, Key::Down);
    key(&mut rt, Key::Enter);
    assert_eq!(rt.state.settings.source.about().channel, Channel::Beta);
    assert_eq!(
        info(&rt, "item-1-1-sub").text.as_deref(),
        Some("Earlier builds for release testing.")
    );
    assert!(find(&rt, "menu-0").is_none());
}

#[test]
fn a_storage_location_is_a_country_chosen_by_keyboard_and_said_in_a_toast() {
    let mut rt = runtime("DW-0788");
    assert_eq!(info(&rt, "item-0-0").role, "radio");
    assert!(find(&rt, "item-0-0-check").is_some(), "the default is chosen");
    assert!(
        !info(&rt, "item-0-3").focusable,
        "the Netherlands is coming soon: not a choice"
    );
    assert!(
        find(&rt, "list-1-body").is_none(),
        "no fallback question for the default"
    );
    tab_to(&mut rt, "item-0-1");
    key(&mut rt, Key::Enter);
    assert!(find(&rt, "item-0-1-check").is_some(), "Germany is chosen");
    assert!(find(&rt, "item-0-0-check").is_none());
    assert_eq!(
        info(&rt, "toast-0-text").text.as_deref(),
        Some("Storage location updated")
    );
    assert_eq!(rt.state.settings.source.general().storage_location, "Germany");

    assert_eq!(info(&rt, "item-1-0-switch").role, "switch");
    tab_to(&mut rt, "item-1-0-switch");
    key(&mut rt, Key::Enter);
    assert!(rt.state.settings.source.locations().is_some_and(|l| l.only_here));

    click(&mut rt, "item-0-3");
    assert_eq!(
        rt.state.settings.source.locations().map(|l| l.chosen),
        Some(1),
        "a press on a country that is not live changes nothing"
    );
}

#[test]
fn locations_that_could_not_load_say_so_and_try_again_loads_them() {
    let mut rt = runtime("DW-0788d");
    assert_eq!(info(&rt, "card-0").role, "alert");
    assert!(find(&rt, "list-0-body").is_none());
    tab_to(&mut rt, "btn-0");
    key(&mut rt, Key::Enter);
    assert!(find(&rt, "list-0-body").is_some(), "the list is back");
    assert!(find(&rt, "card-0").is_none());
}

#[test]
fn alert_switches_turn_by_keyboard_and_are_never_amber() {
    let mut rt = runtime("DW-0750");
    let on = info(&rt, "item-3-0-switch");
    assert_eq!(on.role, "switch");
    assert_eq!(on.label.as_deref(), Some("Shared with you"));
    let amber = crate::theme::tokens::amber();
    assert_ne!(on.fill.hex(), amber.hex(), "a switch is ink, not the accent");
    tab_to(&mut rt, "item-3-1-switch");
    key(&mut rt, Key::Enter);
    assert!(
        rt.state.settings.source.notifications().groups[0].1[1].on,
        "File updates is on"
    );
    click(&mut rt, "item-3-1-switch");
    assert!(!rt.state.settings.source.notifications().groups[0].1[1].on);
    click(&mut rt, "item-1-0");
    assert_eq!(rt.state.settings.pane, Pane::Recent);
    assert_eq!(on_settings(&rt), Some(SettingsPane::Notifications));
}

#[test]
fn notifications_off_in_windows_keep_the_choices_out_of_reach_and_open_windows_settings() {
    let mut rt = runtime("DW-0750b");
    assert_eq!(info(&rt, "banner-0").role, "status");
    assert!(
        !info(&rt, "item-2-0-switch").focusable,
        "a dimmed switch does not take focus"
    );
    click(&mut rt, "item-2-0-switch");
    assert!(
        rt.state.settings.source.notifications().groups[0].1[0].on,
        "and a press changes nothing"
    );
    assert_eq!(info(&rt, "item-1-0-value").text.as_deref(), Some("Off in Windows"));
    tab_to(&mut rt, "btn-0");
    key(&mut rt, Key::Enter);
    assert_eq!(rt.state.settings.intents, vec![Intent::OpenWindowsNotificationSettings]);
}

#[test]
fn recent_alerts_lead_where_they_point_and_an_empty_list_says_so() {
    let leads = [
        ("item-0-0", "DW-A55"),
        ("item-0-1", "DW-0501"),
        ("item-1-0", "DW-0503"),
        ("item-2-0", "DW-A77"),
    ];
    for (row, frame) in leads {
        let mut rt = runtime("DW-0791");
        assert_eq!(info(&rt, row).role, "link");
        tab_to(&mut rt, row);
        key(&mut rt, Key::Enter);
        assert_eq!(rt.state.settings.intents, vec![Intent::Show(frame)], "{row}");
    }
    let rt = runtime("DW-0791");
    assert!(
        info(&rt, "item-0-0").label.is_some_and(|l| l.starts_with("New. ")),
        "a new alert says so in words, not only with a pill"
    );
    let mut rt = runtime("DW-0791b");
    assert_eq!(info(&rt, "card-0-head").text.as_deref(), Some("No notifications"));
    click(&mut rt, "crumb-row");
    assert_eq!(rt.state.settings.pane, Pane::Notifications);
}

/// The fixture holds the board's content; a source can be swapped for another.
#[test]
fn the_fixture_is_the_board() {
    let s = BoardSettings::new();
    use super::source::SettingsSource;
    assert_eq!(s.general().version, "0.9.1");
    assert_eq!(s.about().core, "2084927");
    let places: Vec<String> = s
        .locations()
        .map(|l| l.choices.into_iter().map(|c| c.name).collect())
        .unwrap_or_default();
    assert_eq!(places, ["Default", "Germany", "Finland", "Netherlands"]);
    assert_eq!(s.recent_alerts().iter().map(|(_, a)| a.len()).sum::<usize>(), 4);
}
