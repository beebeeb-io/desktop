//! Tests of settings inside the window: structure, keyboard, roles and names.
//!
//! Input is scripted: keys and presses go through Rune's `Runtime`, as a person's would.

use super::source::ThemeChoice;
use super::{Intent, Menu, Pane};
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

#[test]
fn ctrl_comma_opens_settings_on_general_and_esc_goes_back_to_drive() {
    let mut rt = runtime("");
    assert!(find(&rt, "set-main").is_none(), "the window starts on Drive");
    rt.key(KeyEvent {
        key: Key::Command(','),
        mods: Mods::default(),
    });
    rt.frame(0.0);
    assert!(rt.state.settings.open);
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
    assert!(!rt.state.settings.open, "Esc goes back");
    assert!(find(&rt, "toolbar").is_some());
}

#[test]
fn back_to_drive_leaves_settings() {
    let mut rt = runtime("DW-0701");
    click(&mut rt, "Back to Drive-row");
    assert!(!rt.state.settings.open);
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

/// Every control the General pane draws does something: a pane, a menu, or a request to the shell.
#[test]
fn every_row_of_general_acts() {
    let expect: [(&str, Intent); 10] = [
        ("item-1-0", Intent::Show("DW-0380")),
        ("item-1-1", Intent::Show("DW-0782")),
        ("item-1-2", Intent::Show("DW-0790")),
        ("item-2-0", Intent::Open("privacy")),
        ("item-2-1", Intent::Open("export-my-data")),
        ("item-2-2", Intent::Show("DW-0788")),
        ("item-3-0", Intent::Web("terms-and-privacy-policy")),
        ("item-3-1", Intent::Show("DW-0751")),
        ("Account-row", Intent::Show("DW-A51")),
        ("Bandwidth-row", Intent::Show("DW-0785")),
    ];
    for (row, intent) in expect {
        let mut rt = runtime("DW-0701");
        assert!(info(&rt, row).focusable, "{row} takes focus");
        click(&mut rt, row);
        assert_eq!(rt.state.settings.intents, vec![intent.clone()], "{row} by pointer");

        let mut rt = runtime("DW-0701");
        tab_to(&mut rt, row);
        key(&mut rt, Key::Enter);
        assert_eq!(rt.state.settings.intents, vec![intent], "{row} by keyboard");
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
        Some(Menu::Theme { hot: 0 }),
        "opens on the current choice"
    );
    assert_eq!(info(&rt, "menu-0").role, "menu");
    assert!(find(&rt, "menu-0-item-0-check").is_some(), "System is checked");
    assert!(find(&rt, "menu-0-item-1-check").is_none());

    key(&mut rt, Key::Up);
    assert_eq!(
        rt.state.settings.menu,
        Some(Menu::Theme { hot: 0 }),
        "Up stops at the first item"
    );
    key(&mut rt, Key::Down);
    key(&mut rt, Key::Down);
    key(&mut rt, Key::Down);
    assert_eq!(
        rt.state.settings.menu,
        Some(Menu::Theme { hot: 2 }),
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
    assert!(rt.state.settings.open, "settings stayed");
    assert_eq!(
        rt.state.settings.source.general().theme,
        ThemeChoice::System,
        "nothing chosen"
    );
    key(&mut rt, Key::Escape);
    assert!(!rt.state.settings.open);
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
