//! RC2-8: the window's keys from every section, and the menu buttons on the keyboard.
//! Ctrl+1…7 are the shell's (not only the Files window's); Enter on New, Sort and the account pill opens their menu;
//! Escape from New and Sort gives the keyboard back to the button; the switcher's arrows reach its last row.

use std::sync::Arc;

use rune::{Env, Key, KeyEvent, Mods, Runtime};

use super::super::actions::FixtureEngine;
use super::super::app::Screen;
use super::super::platform::fake::FakePlatform;
use super::super::registry;
use super::super::state;
use super::*;
use crate::frames::account::switcher::{self, Listed, Switcher};
use crate::frames::files::MenuKind;

const ENV: Env = Env {
    system_dark: false,
    scale: 2.0,
};

fn fixture() -> Runtime<State> {
    let engine = Arc::new(FixtureEngine::ready());
    let platform = Arc::new(FakePlatform::default());
    let mut rt = Runtime::new(
        state::init_with(Shell::fixture_on(engine, platform)),
        registry::view,
        (1280.0, 800.0),
        ENV,
    );
    rt.on_key = Some(on_key);
    rt.poll = Some(poll);
    rt.frame(0.0);
    settle(&mut rt);
    rt
}

fn settle(rt: &mut Runtime<State>) {
    rt.frame(0.016);
    rt.settle_all();
    rt.frame(0.016);
}

fn press(rt: &mut Runtime<State>, key: Key, mods: Mods) {
    rt.key(KeyEvent { key, mods });
    settle(rt);
}

fn ctrl() -> Mods {
    Mods {
        cmd: true,
        ctrl: true,
        ..Mods::default()
    }
}

fn focus(rt: &mut Runtime<State>, key: &'static str) {
    rt.update(|_, tx| tx.focus(key, true));
    settle(rt);
    assert_eq!(focused(rt).as_deref(), Some(key), "{key} takes the keyboard");
}

fn focused(rt: &Runtime<State>) -> Option<String> {
    rt.elements()
        .into_iter()
        .find(|e| e.focused && !e.exiting)
        .and_then(|e| e.key)
}

fn menu(rt: &Runtime<State>) -> Option<MenuKind> {
    rt.state.actions.menu.map(|open| open.kind)
}

/// Where the window starts each run: every section, Settings and Account.
fn starts() -> Vec<(&'static str, Event)> {
    let mut starts: Vec<(&'static str, Event)> = Section::ALL
        .iter()
        .map(|&section| (section.label(), Event::Navigate(Route::section(section))))
        .collect();
    starts.push(("Settings", Event::OpenSettings(SettingsPane::General)));
    starts.push(("Account", Event::OpenSettings(SettingsPane::Account)));
    starts
}

#[test]
fn rc2_keys_ctrl_3_goes_to_photos_from_trash_settings_and_account() {
    let mut rt = fixture();
    for (name, event) in starts() {
        rt.update(|s, _| {
            dispatch(s, event.clone());
        });
        settle(&mut rt);
        press(&mut rt, Key::Command('3'), ctrl());
        assert_eq!(
            rt.state.shell.app.screen(),
            Screen::Drive(Route::section(Section::Photos)),
            "Ctrl+3 from {name}"
        );
    }
}

#[test]
fn rc2_keys_ctrl_1_to_7_reach_every_section_from_every_section() {
    let mut rt = fixture();
    for (name, event) in starts() {
        for (n, &section) in Section::ALL.iter().enumerate() {
            rt.update(|s, _| {
                dispatch(s, event.clone());
            });
            settle(&mut rt);
            let digit = char::from(b'1' + n as u8);
            press(&mut rt, Key::Command(digit), ctrl());
            assert_eq!(
                rt.state.shell.app.screen(),
                Screen::Drive(Route::section(section)),
                "Ctrl+{digit} from {name}"
            );
        }
    }
}

#[test]
fn rc2_keys_an_open_menu_keeps_ctrl_digits() {
    let mut rt = fixture();
    rt.update(|s, _| s.toggle_menu(MenuKind::Sort));
    settle(&mut rt);
    press(&mut rt, Key::Command('3'), ctrl());
    assert_eq!(
        rt.state.shell.app.screen(),
        Screen::Drive(Route::section(Section::Files)),
        "an open menu is modal"
    );
}

#[test]
fn rc2_keys_enter_on_new_opens_it_and_esc_returns_to_it() {
    let mut rt = fixture();
    focus(&mut rt, "new-btn");
    press(&mut rt, Key::Enter, Mods::default());
    assert_eq!(menu(&rt), Some(MenuKind::New), "Enter on New opens its menu");
    press(&mut rt, Key::Escape, Mods::default());
    assert_eq!(menu(&rt), None, "Esc closes it");
    assert_eq!(
        focused(&rt).as_deref(),
        Some("new-btn"),
        "Esc gives the keyboard back to New"
    );
}

#[test]
fn rc2_keys_enter_on_sort_opens_it_and_esc_returns_to_it() {
    let mut rt = fixture();
    focus(&mut rt, "sort-btn");
    press(&mut rt, Key::Enter, Mods::default());
    assert_eq!(menu(&rt), Some(MenuKind::Sort), "Enter on Sort opens its menu");
    press(&mut rt, Key::Escape, Mods::default());
    assert_eq!(menu(&rt), None, "Esc closes it");
    assert_eq!(
        focused(&rt).as_deref(),
        Some("sort-btn"),
        "Esc gives the keyboard back to Sort"
    );
}

/// Two accounts on this PC: the current one, and one to switch to.
struct TwoAccounts;

impl Switcher for TwoAccounts {
    fn accounts(&self) -> Vec<Listed> {
        vec![
            Listed {
                id: "a".into(),
                email: "anna@example.eu".into(),
                current: true,
            },
            Listed {
                id: "b".into(),
                email: "bram@example.eu".into(),
                current: false,
            },
        ]
    }
    fn switch_to(&self, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn add_another(&self) -> Result<(), String> {
        Ok(())
    }
    fn sign_out(&self, _: &str) -> Result<(), String> {
        Ok(())
    }
}

#[test]
fn rc2_keys_the_switcher_opens_with_enter_and_its_arrows_reach_the_last_row() {
    switcher::install(Arc::new(TwoAccounts));
    let mut rt = fixture();
    assert!(rt.find("account-pill").is_some(), "the pill shows");
    focus(&mut rt, "account-pill");
    press(&mut rt, Key::Enter, Mods::default());
    assert!(switcher::is_open(), "Enter on the pill opens the switcher");
    // The list is read off the window's thread.
    let read = std::time::Instant::now();
    while rt.find("switcher-account-1").is_none() && read.elapsed() < std::time::Duration::from_secs(5) {
        std::thread::sleep(std::time::Duration::from_millis(10));
        settle(&mut rt);
    }
    press(&mut rt, Key::Down, Mods::default());
    let first = focused(&rt);
    assert!(
        first.as_deref().is_some_and(|key| key.starts_with("switcher-")),
        "Down from the pill goes into the list: {first:?}"
    );
    let mut seen = vec![first.clone()];
    for _ in 0..4 {
        press(&mut rt, Key::Down, Mods::default());
        seen.push(focused(&rt));
    }
    assert!(
        seen.iter().any(|key| key.as_deref() == Some("switcher-add")),
        "the arrows reach Add another account: {seen:?}"
    );
    press(&mut rt, Key::Escape, Mods::default());
    assert!(!switcher::is_open(), "Esc closes the switcher");
    assert_eq!(
        focused(&rt).as_deref(),
        Some("account-pill"),
        "and gives the keyboard back to the pill"
    );
}
