//! Settings lists in a runtime: each list numbers its own rows, a `roving_list` is one Tab stop
//! that roves and type-selects, and a plain `list` leaves every switch inside it a Tab stop.

use std::sync::Arc;

use rune::{Cx, El, Env, Key, KeyEvent, Mods, Runtime, col, el};
use serde_json::Value;

use super::*;
use crate::State;
use crate::frames::files::source::FilesFixture;
use crate::shell::actions::FixtureEngine;
use crate::shell::host::Shell;
use crate::shell::platform::fake::FakePlatform;
use crate::theme::tokens::theme;
use crate::ui::account_set::{self, Lead, Row};

/// A control outside the lists, so Tab has somewhere to come from and go to.
fn stop(key: &'static str) -> El {
    el().key(key).role("button").label(key).focusable(true).size(40.0, 20.0)
}

/// Two plain rows, each holding a switch (a focusable control of its own), then four rows that
/// are each the control, in a roving list, then account rows in a second roving list.
fn page(_: &State, _: &mut Cx<State>) -> El {
    let t = theme(false);
    let plain = list(
        0,
        Some("Plain"),
        (0..2)
            .map(|i| {
                let sw = el()
                    .key(format!("sw-{i}"))
                    .role("switch")
                    .label(format!("Switch {i}"))
                    .focusable(true)
                    .size(26.0, 15.0);
                item_with(0, i, &Item::new(None, format!("Row {i}")), vec![sw], &t)
            })
            .collect(),
        None,
        &t,
    );
    let rows = [
        Item::new(None, "Alpha").opens(),
        Item::new(None, "Bravo").role(Role::Radio(true)),
        Item::new(None, "Charlie").opens(),
        Item::new(None, "Delta").opens(),
    ];
    let roving = roving_list(
        1,
        Some("Roving"),
        rows.iter().enumerate().map(|(i, it)| item(1, i, it, &t)).collect(),
        None,
        &t,
    );
    let accounts = roving_list(
        2,
        None,
        vec![
            account_set::item(2, 0, &Row::new(Lead::None, "Laptop").link(true), vec![], &t),
            account_set::item(2, 1, &Row::new(Lead::None, "Phone").link(false).selected(true), vec![], &t),
        ],
        None,
        &t,
    );
    col()
        .key("page")
        .w(400.0)
        .gap(8.0)
        .child(stop("before"))
        .child(plain)
        .child(stop("middle"))
        .child(roving)
        .child(accounts)
        .child(stop("after"))
}

fn window() -> Runtime<State> {
    let engine = Arc::new(FixtureEngine::ready());
    let shell = Shell::fixture_on(engine, Arc::new(FakePlatform::default()));
    let mut state = crate::shell::state::init_with(shell);
    state.actions.source = Box::new(FilesFixture::default());
    let env = Env {
        system_dark: false,
        scale: 2.0,
    };
    let mut rt = Runtime::new(state, page, (400.0, 800.0), env);
    settle(&mut rt);
    rt
}

fn settle(rt: &mut Runtime<State>) {
    rt.frame(0.0);
    rt.settle_all();
    rt.frame(0.0);
}

fn press(rt: &mut Runtime<State>, key: Key, shift: bool) {
    rt.key(KeyEvent {
        key,
        mods: Mods { shift, ..Mods::default() },
    });
    settle(rt);
}

fn focus_on(rt: &mut Runtime<State>, key: &'static str) {
    rt.update(move |_, tx| tx.focus(key, true));
    settle(rt);
}

fn focused(rt: &Runtime<State>) -> String {
    rt.elements()
        .into_iter()
        .find(|e| e.focused && !e.exiting)
        .and_then(|e| e.key)
        .unwrap_or_default()
}

/// The accessibility node keyed `key`.
fn node(tree: &Value, key: &str) -> Option<Value> {
    if tree["key"].as_str() == Some(key) {
        return Some(tree.clone());
    }
    tree["children"].as_array()?.iter().find_map(|c| node(c, key))
}

#[test]
fn each_list_numbers_its_own_rows() {
    let rt = window();
    let tree = rt.accessibility_json();
    for (key, position, size) in [
        ("item-0-0", 1, 2),
        ("item-0-1", 2, 2),
        ("item-1-0", 1, 4),
        ("item-1-3", 4, 4),
        ("item-2-1", 2, 2),
    ] {
        let n = node(&tree, key).unwrap_or_else(|| panic!("no node {key}"));
        assert_eq!(n["position"], position, "{key}: {n}");
        assert_eq!(n["setSize"], size, "{key}: {n}");
    }
}

#[test]
fn a_roving_list_is_one_tab_stop_that_lands_on_the_selected_row() {
    let mut rt = window();
    focus_on(&mut rt, "middle");
    press(&mut rt, Key::Tab, false);
    assert_eq!(focused(&rt), "item-1-1", "Tab lands on the chosen row");
    press(&mut rt, Key::Tab, false);
    assert_eq!(focused(&rt), "item-2-1", "the next list's selected account row");
    press(&mut rt, Key::Tab, false);
    assert_eq!(focused(&rt), "after", "one Tab stop per roving list");
    press(&mut rt, Key::Tab, true);
    press(&mut rt, Key::Tab, true);
    assert_eq!(focused(&rt), "item-1-1", "Shift-Tab comes back to the same row");
}

#[test]
fn a_roving_list_roves_on_arrows_home_end_and_page_keys() {
    let mut rt = window();
    focus_on(&mut rt, "middle");
    press(&mut rt, Key::Tab, false);
    press(&mut rt, Key::Down, false);
    assert_eq!(focused(&rt), "item-1-2");
    press(&mut rt, Key::Up, false);
    assert_eq!(focused(&rt), "item-1-1");
    press(&mut rt, Key::End, false);
    assert_eq!(focused(&rt), "item-1-3");
    press(&mut rt, Key::Down, false);
    assert_eq!(focused(&rt), "item-1-3", "a list stops at its end");
    press(&mut rt, Key::Home, false);
    assert_eq!(focused(&rt), "item-1-0");
    press(&mut rt, Key::PageDown, false);
    assert_eq!(focused(&rt), "item-1-3");
    press(&mut rt, Key::PageUp, false);
    assert_eq!(focused(&rt), "item-1-0");
}

#[test]
fn typing_in_a_roving_list_picks_the_row_by_its_name() {
    // A fresh window each time: letters typed without a pause build one prefix.
    for (letter, row) in [("d", "item-1-3"), ("c", "item-1-2"), ("a", "item-1-0")] {
        let mut rt = window();
        focus_on(&mut rt, "middle");
        press(&mut rt, Key::Tab, false);
        press(&mut rt, Key::Text(letter.into()), false);
        assert_eq!(focused(&rt), row, "typed {letter}");
    }
}

#[test]
fn tab_reaches_every_switch_in_a_plain_list() {
    let mut rt = window();
    focus_on(&mut rt, "before");
    press(&mut rt, Key::Tab, false);
    assert_eq!(focused(&rt), "sw-0");
    press(&mut rt, Key::Tab, false);
    assert_eq!(focused(&rt), "sw-1");
    press(&mut rt, Key::Tab, false);
    assert_eq!(focused(&rt), "middle");
}
