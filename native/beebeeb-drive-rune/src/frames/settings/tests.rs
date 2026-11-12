//! Tests of settings inside the window: structure, keyboard, roles and names.
//!
//! Input is scripted: keys and presses go through Rune's `Runtime`, as a person's would.

use super::source::{self, BoardSettings, Channel, Locations, SettingsSource, ThemeChoice, Unwired, UpdateCheck};
use super::{Dialog, INTENT_LIMIT, Intent, Menu, MenuKind, Pane};
use crate::shell::app::{Event, Screen, SettingsPane};
use crate::shell::host;
use crate::{State, init, view};
use rune::{Env, Key, KeyEvent, Mods, Runtime};

/// A window showing board frame `frame` (`""`: the Files window, as the app starts).
fn runtime(frame: &str) -> Runtime<State> {
    runtime_sized(frame, (958.0, 638.0))
}

fn runtime_sized(frame: &str, size: (f32, f32)) -> Runtime<State> {
    let mut state = init();
    if !frame.is_empty() {
        assert!(super::scene(frame, &mut state), "{frame} is a built frame");
    }
    window(state, size)
}

fn window(state: State, size: (f32, f32)) -> Runtime<State> {
    let env = Env {
        system_dark: false,
        scale: 2.0,
    };
    let mut rt = Runtime::new(state, view, size, env);
    // The app's own wiring (`shell::run`): the shell's keys first, then the frame's.
    rt.on_key = Some(host::on_key);
    rt.poll = Some(host::poll);
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
    settle(rt);
}

fn space() -> Key {
    Key::Text(" ".into())
}

/// The window as a person opens settings: from Drive, with Ctrl+, (the shell's own key).
/// Nothing of this lane is told; it has to follow the shell's route.
fn opened_by_the_shell() -> Runtime<State> {
    let mut rt = runtime("");
    ctrl_comma(&mut rt);
    assert_eq!(on_settings(&rt), Some(SettingsPane::General), "Ctrl+, opens settings");
    rt
}

fn ctrl_comma(rt: &mut Runtime<State>) {
    rt.key(KeyEvent {
        key: Key::Command(','),
        mods: Mods {
            cmd: true,
            ctrl: true,
            ..Mods::default()
        },
    });
    settle(rt);
}

/// The Tab stops of the window, in order, from nothing focused until focus comes round.
fn tab_stops(rt: &mut Runtime<State>) -> Vec<String> {
    let mut stops: Vec<String> = Vec::new();
    for _ in 0..120 {
        key(rt, Key::Tab);
        let Some(now) = rt
            .elements()
            .into_iter()
            .find(|e| e.focused && !e.exiting)
            .and_then(|e| e.key)
        else {
            continue;
        };
        if stops.contains(&now) {
            break;
        }
        stops.push(now);
    }
    stops
}

/// Lets leaving elements finish leaving: Rune keeps them in the tree while they animate out.
fn settle(rt: &mut Runtime<State>) {
    rt.frame(0.016);
    rt.settle_all();
    rt.frame(0.016);
}

fn click(rt: &mut Runtime<State>, key: &str) {
    let r = info(rt, key).rect;
    let (x, y) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    settle(rt);
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
    rt.update(|s, _| {
        host::dispatch(s, Event::OpenSettings(SettingsPane::General));
    });
    settle(&mut rt);
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
    click(&mut rt, "Backups-row");
    assert_eq!(on_settings(&rt), Some(SettingsPane::Backups));
    assert_eq!(
        info(&rt, "set-title").text.as_deref(),
        Some("Backups"),
        "the Backups lane's pane, shown by the shell"
    );
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
    let asks: [(&str, Intent); 6] = [
        ("item-1-0", Intent::Show("DW-0380")),
        ("item-1-2", Intent::Show("DW-0790")),
        ("item-2-0", Intent::Open("privacy")),
        ("item-2-1", Intent::Open("export-my-data")),
        ("item-3-0", Intent::Web("terms-and-privacy-policy")),
        ("Bandwidth-row", Intent::Show("DW-0785")),
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
    assert_eq!(
        info(&rt, "item-0-0").role,
        "combobox",
        "a row that opens a menu of choices"
    );
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
    assert_eq!(
        super::FRAMES.len(),
        31,
        "the frames the settings lanes built (28, and Photo previews' 3)"
    );
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
    let leads = [("item-0-1", "DW-0501"), ("item-1-0", "DW-0503"), ("item-2-0", "DW-A77")];
    for (row, frame) in leads {
        let mut rt = runtime("DW-0791");
        assert_eq!(info(&rt, row).role, "link");
        tab_to(&mut rt, row);
        key(&mut rt, Key::Enter);
        assert_eq!(rt.state.settings.intents, vec![Intent::Show(frame)], "{row}");
    }
    // A place the shell routes itself (Settings › Devices and sessions): no request needed.
    let mut rt = runtime("DW-0791");
    tab_to(&mut rt, "item-0-0");
    key(&mut rt, Key::Enter);
    assert_eq!(on_settings(&rt), Some(SettingsPane::DevicesAndSessions));
    assert!(rt.state.settings.intents.is_empty());
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

// --- Review of PR 292: each finding's killing test ---

/// P1.1: opened the way a person opens it, every in-lane navigation works: settings follows
/// the shell's route and keeps no "open" flag of its own.
#[test]
fn opened_by_the_shell_every_row_goes_where_it_points() {
    let rows: [(&str, Pane, &str, Option<SettingsPane>); 4] = [
        ("item-3-1", Pane::About, "About Beebeeb", Some(SettingsPane::General)),
        (
            "item-2-2",
            Pane::StorageLocation,
            "Storage location",
            Some(SettingsPane::General),
        ),
        (
            "Notifications-row",
            Pane::Notifications,
            "Notifications",
            Some(SettingsPane::Notifications),
        ),
        ("General-row", Pane::General, "General", Some(SettingsPane::General)),
    ];
    for (row, pane, title, route) in rows {
        let mut rt = opened_by_the_shell();
        click(&mut rt, row);
        assert_eq!(on_settings(&rt), route, "{row}: the window stays in settings");
        assert_eq!(rt.state.settings.pane, pane, "{row}");
        assert_eq!(info(&rt, "set-title").text.as_deref(), Some(title), "{row}");
    }
    // Two levels in, and back out by both crumbs.
    let mut rt = opened_by_the_shell();
    click(&mut rt, "Notifications-row");
    click(&mut rt, "item-1-0");
    assert_eq!(info(&rt, "set-title").text.as_deref(), Some("Recent notifications"));
    click(&mut rt, "crumb-row");
    assert_eq!(info(&rt, "set-title").text.as_deref(), Some("Notifications"));
    click(&mut rt, "General-row");
    click(&mut rt, "item-3-1");
    click(&mut rt, "crumb-row");
    assert_eq!(info(&rt, "set-title").text.as_deref(), Some("General"));
    assert_eq!(on_settings(&rt), Some(SettingsPane::General));
}

/// P1.1, second half: what opens over a pane is DRAWN when settings was opened by the shell.
#[test]
fn opened_by_the_shell_menus_and_dialogs_are_drawn() {
    let mut rt = opened_by_the_shell();
    click(&mut rt, "item-0-0");
    assert!(find(&rt, "menu-0").is_some(), "the Theme menu shows");
    assert!(find(&rt, "menu-0-item-1").is_some());
    key(&mut rt, Key::Down);
    key(&mut rt, Key::Enter);
    assert_eq!(rt.state.settings.source.general().theme, ThemeChoice::Light);
    assert!(find(&rt, "menu-0").is_none());

    let mut rt = opened_by_the_shell();
    click(&mut rt, "item-1-1");
    assert_eq!(info(&rt, "dlg").label.as_deref(), Some("Report a problem"));

    let mut rt = opened_by_the_shell();
    click(&mut rt, "item-3-1");
    click(&mut rt, "item-1-1");
    assert_eq!(info(&rt, "menu-0").label.as_deref(), Some("Release channel"));
    key(&mut rt, Key::Escape);
    click(&mut rt, "item-1-2");
    assert_eq!(info(&rt, "dlg-title").text.as_deref(), Some("Beebeeb 0.9.2"));
}

/// P1.2: Space AND Enter press the focused control, whatever it is (Windows' keyboard).
#[test]
fn space_and_enter_both_press_the_focused_control() {
    for press in [Key::Enter, space()] {
        // A link row.
        let mut rt = opened_by_the_shell();
        tab_to(&mut rt, "item-3-1");
        key(&mut rt, press.clone());
        assert_eq!(rt.state.settings.pane, Pane::About, "{press:?} on a row");
        // A button.
        tab_to(&mut rt, "btn-1");
        key(&mut rt, press.clone());
        assert_eq!(
            rt.state.settings.source.about().check,
            UpdateCheck::Checking,
            "{press:?} on a button"
        );
        // A sidebar link.
        tab_to(&mut rt, "Notifications-row");
        key(&mut rt, press.clone());
        assert_eq!(
            rt.state.settings.pane,
            Pane::Notifications,
            "{press:?} on a sidebar item"
        );
        // A switch.
        tab_to(&mut rt, "item-3-1-switch");
        key(&mut rt, press.clone());
        assert!(
            rt.state.settings.source.notifications().groups[0].1[1].on,
            "{press:?} on a switch"
        );
        // A pop-up row opens its menu.
        let mut rt = opened_by_the_shell();
        tab_to(&mut rt, "item-0-0");
        key(&mut rt, press.clone());
        assert!(find(&rt, "menu-0").is_some(), "{press:?} opens the Theme menu");
    }
}

/// P2.3: a pop-up row is ONE control and one Tab stop; the pop-up button in it is its picture.
#[test]
fn a_pop_up_row_is_one_tab_stop_and_every_tab_stop_acts() {
    let mut rt = opened_by_the_shell();
    let pop = info(&rt, "item-0-0-pop");
    assert!(!pop.focusable, "the pop-up button is not a control of its own");
    assert!(
        pop.role != "button" && pop.role != "combobox" && pop.label.is_none(),
        "and is not announced as one: role {}, label {:?}",
        pop.role,
        pop.label
    );
    assert_eq!(info(&rt, "item-0-0").role, "combobox");
    let stops = tab_stops(&mut rt);
    // KNOWN, NOT FIXED HERE: the caption's three buttons (`ui::caption`, the design-system
    // lane's) come first under `Cx::web_focus`; Windows' own caption buttons are not Tab
    // stops. They need `.focusable(false)` there (the pull request asks for it).
    assert_eq!(stops[..3], ["cap-min", "cap-max", "cap-close"]);
    assert_eq!(
        stops[3..],
        [
            "Back to Drive-row",
            "General-row",
            "Account-row",
            "Security-row",
            "Devices and sessions-row",
            "Notifications-row",
            "Storage-row",
            "Backups-row",
            "Sync and File Explorer-row",
            "Thumbnails and previews-row",
            "Bandwidth-row",
            "item-0-0",
            "item-1-0",
            "item-1-1",
            "item-1-2",
            "item-2-0",
            "item-2-1",
            "item-2-2",
            "item-3-0",
            "item-3-1",
        ],
        "the Tab order of General: the sidebar, then the rows, top to bottom; nothing twice"
    );
    // About has the other pop-up row.
    click(&mut rt, "item-3-1");
    assert!(!info(&rt, "item-1-1-pop").focusable);
    let stops = tab_stops(&mut rt);
    assert_eq!(
        stops.iter().filter(|k| k.starts_with("item-1-1")).count(),
        1,
        "{stops:?}"
    );
}

/// P2.4: a server answer without a single location is shown as a list that could not load.
#[test]
fn a_list_of_locations_without_choices_never_panics_and_says_it_could_not_load() {
    struct Empty(BoardSettings);
    impl SettingsSource for Empty {
        fn locations(&self) -> Option<Locations> {
            Some(Locations {
                choices: Vec::new(),
                chosen: 0,
                only_here: true,
                unavailable: true,
            })
        }
        fn mark_recent_seen(&mut self) {}
        fn check_for_updates(&mut self) {}
        fn recent_alerts(&self) -> Vec<(String, Vec<source::RecentAlert>)> {
            self.0.recent_alerts()
        }
        fn notifications(&self) -> source::Notifications {
            self.0.notifications()
        }
        fn set_allow_notifications(&mut self, _: bool) {}
        fn set_alert(&mut self, _: usize, _: usize, _: bool) {}
        fn choose_location(&mut self, _: usize) -> bool {
            false
        }
        fn set_only_here(&mut self, _: bool) {}
        fn reload_locations(&mut self) {}
        fn about(&self) -> source::About {
            self.0.about()
        }
        fn set_channel(&mut self, _: Channel) {}
        fn general(&self) -> source::General {
            self.0.general()
        }
        fn set_theme(&mut self, _: ThemeChoice) {}
    }
    let empty = Locations {
        choices: Vec::new(),
        chosen: 3,
        only_here: false,
        unavailable: false,
    };
    assert!(empty.chosen().is_none());
    let mut state = init();
    state.settings.source = Box::new(Empty(BoardSettings::new()));
    super::open_on(&mut state, "DW-0788");
    let mut rt = window(state, (958.0, 638.0));
    assert_eq!(info(&rt, "card-0").role, "alert");
    assert!(find(&rt, "list-0-body").is_none(), "no empty list");
    click(&mut rt, "crumb-row");
    assert_eq!(
        info(&rt, "set-title").text.as_deref(),
        Some("General"),
        "General draws too"
    );
}

/// P2.5, DW-0751: "while checking the row says Checking…; a check that fails says so with
/// Try again."
#[test]
fn the_update_check_says_checking_and_says_when_it_failed() {
    let mut rt = runtime("DW-0751");
    assert_eq!(
        info(&rt, "item-1-0-sub").text.as_deref(),
        Some("0.9.1 is the newest version · checked today, 09:12")
    );
    click(&mut rt, "btn-1");
    assert_eq!(info(&rt, "item-1-0-sub").text.as_deref(), Some("Checking…"));
    assert!(!info(&rt, "btn-1").focusable, "the button waits while a check runs");
    click(&mut rt, "btn-1");
    assert_eq!(
        rt.state.settings.intents,
        vec![Intent::CheckForUpdates],
        "one check asked, not two"
    );

    let mut state = init();
    state.settings.source = Box::new(BoardSettings::new().with_check_failed());
    super::open_on(&mut state, "DW-0751");
    let mut rt = window(state, (958.0, 638.0));
    assert_eq!(
        info(&rt, "item-1-0-sub").text.as_deref(),
        Some("Couldn’t check for updates. Try again in a moment.")
    );
    tab_to(&mut rt, "btn-1");
    key(&mut rt, Key::Enter);
    assert_eq!(info(&rt, "item-1-0-sub").text.as_deref(), Some("Checking…"));
}

/// P2.5, DW-0789: another channel forgets the last answer and checks again; a channel whose
/// newest version is older says so and links the notes, and installs nothing.
#[test]
fn another_channel_checks_again_and_an_older_channel_says_so() {
    let mut rt = runtime("DW-0751");
    click(&mut rt, "item-1-1");
    click(&mut rt, "menu-0-item-1");
    assert_eq!(rt.state.settings.source.about().channel, Channel::Beta);
    assert_eq!(info(&rt, "item-1-0-sub").text.as_deref(), Some("Checking…"));

    let mut state = init();
    state.settings.source = Box::new(BoardSettings::new().with_older_channel());
    super::open_on(&mut state, "DW-0751");
    let mut rt = window(state, (958.0, 738.0));
    assert_eq!(
        info(&rt, "item-1-2-title").text.as_deref(),
        Some("The Stable channel is at version 0.9.0")
    );
    assert_eq!(
        info(&rt, "item-1-2-sub").text.as_deref(),
        Some(
            "This PC runs 0.9.1, from the Beta channel. Read the release notes before you go back to an older version."
        )
    );
    assert_eq!(info(&rt, "item-1-2").role, "link");
    assert_eq!(info(&rt, "item-1-3-title").text.as_deref(), Some("Release notes"));
    click(&mut rt, "item-1-2");
    assert_eq!(
        rt.state.settings.dialog,
        Some(Dialog::ReleaseNotes),
        "it links the notes"
    );
    assert!(
        !rt.state.settings.intents.contains(&Intent::DownloadUpdate),
        "nothing is installed or downloaded"
    );
}

/// P2.5, DW-0791: "New ones are marked until this page has been opened; the count shows on
/// the Notifications row."
#[test]
fn recent_alerts_are_new_until_the_page_has_been_opened() {
    let mut rt = runtime("DW-0750");
    assert_eq!(info(&rt, "item-1-0-value").text.as_deref(), Some("2 new"));
    click(&mut rt, "item-1-0");
    assert!(
        find(&rt, "pill-0").is_some(),
        "marked while the page shows them the first time"
    );
    assert!(find(&rt, "pill-1").is_some());
    click(&mut rt, "crumb-row");
    assert!(
        find(&rt, "item-1-0-value").is_none(),
        "the count is gone from the Notifications row"
    );
    click(&mut rt, "item-1-0");
    assert!(find(&rt, "pill-0").is_none(), "and nothing is marked the second time");
    assert!(
        !info(&rt, "item-0-0").label.is_some_and(|l| l.starts_with("New. ")),
        "in words too"
    );
}

/// P2.6: what settings asks of the shell is a queue the shell takes, and it never grows
/// without bound while nothing takes it.
#[test]
fn requests_to_the_shell_are_taken_once_and_never_pile_up() {
    let mut rt = runtime("DW-0701");
    for _ in 0..(INTENT_LIMIT + 9) {
        click(&mut rt, "item-2-0");
    }
    assert_eq!(rt.state.settings.intents.len(), INTENT_LIMIT);
    click(&mut rt, "item-3-0");
    let taken = rt.state.settings.take_intents();
    assert_eq!(taken.len(), INTENT_LIMIT);
    assert_eq!(
        taken.last(),
        Some(&Intent::Web("terms-and-privacy-policy")),
        "the newest is kept"
    );
    assert!(rt.state.settings.take_intents().is_empty(), "taken once");
}

/// P2.7: a build that ships never opens settings on the board's sample content.
#[test]
fn a_build_that_ships_never_shows_the_sample_content() {
    assert!(
        source::first(true).sample(),
        "tooling and tests start on the board's content"
    );
    let shipped = source::first(false);
    assert!(!shipped.sample());
    assert!(!shipped.wired());

    let mut state = init();
    state.settings.source = Box::new(Unwired);
    let mut rt = window(state, (958.0, 638.0));
    ctrl_comma(&mut rt);
    assert_eq!(on_settings(&rt), Some(SettingsPane::General));
    assert!(find(&rt, "not-built").is_some(), "the shell's not-built frame");
    assert!(find(&rt, "set-main").is_none(), "and no pane");
    for e in rt.elements() {
        let words = format!("{} {}", e.text.unwrap_or_default(), e.label.unwrap_or_default());
        for sample in ["0.9.1", "311", "2084927", "09:12", "Anna", "Lotte", "Priya", "2 new"] {
            assert!(!words.contains(sample), "an unwired build says {sample}: {words}");
        }
    }
    key(&mut rt, Key::Escape);
    assert_eq!(on_settings(&rt), None, "Esc still leaves");
}

/// P2.8: what was open when the window left settings is not there on the next visit,
/// whoever closed settings.
#[test]
fn nothing_stays_open_across_leaving_settings_and_coming_back() {
    // The shell leaves (its own Escape, as when nothing is focused) with a dialog open.
    let mut rt = opened_by_the_shell();
    click(&mut rt, "item-1-1");
    assert!(find(&rt, "dlg").is_some());
    rt.update(|s, _| {
        host::dispatch(s, Event::Escape);
    });
    settle(&mut rt);
    assert_eq!(on_settings(&rt), None);
    ctrl_comma(&mut rt);
    assert!(find(&rt, "dlg").is_none(), "no dialog from the last visit");
    assert_eq!(rt.state.settings.dialog, None);

    // A sub-pane, left with Esc: the next visit starts on General.
    click(&mut rt, "item-3-1");
    assert_eq!(info(&rt, "set-title").text.as_deref(), Some("About Beebeeb"));
    key(&mut rt, Key::Escape);
    assert_eq!(on_settings(&rt), None);
    ctrl_comma(&mut rt);
    assert_eq!(info(&rt, "set-title").text.as_deref(), Some("General"));

    // A menu, and the shell moved to another lane's section and back.
    click(&mut rt, "item-0-0");
    assert!(find(&rt, "menu-0").is_some());
    rt.update(|s, _| {
        host::dispatch(s, Event::Escape);
        host::dispatch(s, Event::OpenSettings(SettingsPane::Notifications));
    });
    settle(&mut rt);
    assert_eq!(info(&rt, "set-title").text.as_deref(), Some("Notifications"));
    assert!(
        find(&rt, "menu-0").is_none(),
        "a menu of General is not drawn over Notifications"
    );
    key(&mut rt, Key::Down);
    key(&mut rt, Key::Enter);
    assert_eq!(
        rt.state.settings.source.general().theme,
        ThemeChoice::System,
        "and keys do not reach a menu nobody can see"
    );
}

/// P2.9: a pane taller than the window scrolls, and Tab brings the focused control into view.
#[test]
fn a_pane_taller_than_the_window_scrolls_to_the_focused_control() {
    let mut rt = runtime("DW-0750");
    let last = "item-5-1-switch";
    let below = info(&rt, last).rect;
    assert!(
        below[1] + below[3] > 638.0,
        "Backup needs attention starts below the window"
    );
    tab_to(&mut rt, last);
    settle(&mut rt);
    let shown = info(&rt, last).rect;
    assert!(
        shown[1] >= 32.0 && shown[1] + shown[3] <= 638.0,
        "in view after Tab: {shown:?}"
    );
    key(&mut rt, space());
    assert!(
        !rt.state.settings.source.notifications().groups[2].1[1].on,
        "and it turns"
    );
}

/// P2.9: a menu drops over its own pop-up button wherever the row is: another window size,
/// a scrolled pane.
#[test]
fn a_menu_stays_with_its_row() {
    for size in [(958.0, 638.0), (1400.0, 900.0), (958.0, 420.0)] {
        let mut rt = runtime_sized("DW-0751", size);
        tab_to(&mut rt, "item-1-1");
        key(&mut rt, Key::Enter);
        let (row, menu) = (info(&rt, "item-1-1").rect, info(&rt, "menu-0").rect);
        assert!(
            (menu[0] - (row[0] + 448.0)).abs() < 0.6,
            "{size:?}: the menu at {menu:?} with its row at {row:?}"
        );
        assert!(
            menu[1] >= 0.0 && menu[1] + menu[3] <= size.1,
            "{size:?}: all of the menu is inside the window: {menu:?}"
        );
        // Where there is room it drops over its pop-up button, as the board draws it; where
        // there is not (the short window) it opens upward from the same point.
        let drops = (menu[1] - (row[1] + 1.0)).abs() < 0.6;
        let rises = (menu[1] + menu[3] - (row[1] + 1.0)).abs() < 0.6;
        assert!(
            if size.1 > 500.0 { drops } else { rises },
            "{size:?}: {menu:?} / {row:?}"
        );
    }
}

/// P3.14, P3.15: a dialog keeps Tab inside, Enter is its primary button, Enter on the focused
/// Cancel is Cancel, and Esc closes only the dialog.
#[test]
fn a_dialog_keeps_the_keyboard_inside_and_enter_is_its_primary_button() {
    let mut rt = opened_by_the_shell();
    click(&mut rt, "item-1-1");
    let stops = tab_stops(&mut rt);
    assert_eq!(stops, ["btn-0", "btn-1"], "Tab stays inside the dialog");

    let mut rt = opened_by_the_shell();
    click(&mut rt, "item-1-1");
    key(&mut rt, Key::Enter);
    assert_eq!(
        rt.state.settings.intents,
        vec![Intent::SaveBundleAndWriteEmail, Intent::Show("DW-0790b")],
        "Enter is the primary button"
    );

    let mut rt = opened_by_the_shell();
    click(&mut rt, "item-1-1");
    tab_to(&mut rt, "btn-1");
    key(&mut rt, Key::Enter);
    assert_eq!(rt.state.settings.dialog, None, "Enter on Cancel cancels");
    assert!(rt.state.settings.intents.is_empty(), "and saves nothing");

    let mut rt = opened_by_the_shell();
    click(&mut rt, "item-1-1");
    key(&mut rt, Key::Escape);
    assert!(find(&rt, "dlg").is_none());
    assert_eq!(
        on_settings(&rt),
        Some(SettingsPane::General),
        "Esc closed the dialog only"
    );
}

/// P3.16: a menu index the menu does not have chooses nothing.
#[test]
fn an_index_outside_a_menu_chooses_nothing() {
    let mut rt = runtime("DW-0787");
    rt.update(|s, _| s.settings.choose(MenuKind::Theme, 3));
    assert_eq!(rt.state.settings.source.general().theme, ThemeChoice::System);
    assert_eq!(rt.state.settings.menu, None);
}
