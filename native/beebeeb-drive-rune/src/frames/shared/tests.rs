//! Shared: the switch, the invitation, selection and the intents, by pointer and by keyboard.

use super::Tab;
use crate::frames::Intent;
use crate::{State, init, view};
use rune::{Env, Key, KeyEvent, Mods, Runtime};

pub(crate) fn runtime(scene: fn(&mut State)) -> Runtime<State> {
    let mut state = init();
    scene(&mut state);
    let mut rt = Runtime::new(
        state,
        view,
        (1278.0, 798.0),
        Env {
            system_dark: false,
            scale: 2.0,
        },
    );
    rt.frame(0.0);
    rt
}

pub(crate) fn info(rt: &Runtime<State>, key: &str) -> rune::ElementInfo {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
        .unwrap_or_else(|| panic!("no element keyed {key}"))
}

pub(crate) fn click(rt: &mut Runtime<State>, key: &str) {
    let r = info(rt, key).rect;
    let (x, y) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(0.0);
}

pub(crate) fn press(rt: &mut Runtime<State>, key: Key) {
    rt.key(KeyEvent {
        key,
        mods: Mods::default(),
    });
    rt.frame(0.0);
}

#[test]
fn the_switch_shows_each_list_and_new_request_only_on_requests() {
    let mut rt = runtime(super::scenes::dw_0501);
    assert!(rt.find("card-0").is_some() && rt.find("row-3").is_some() && rt.find("row-4").is_none());
    assert!(rt.find("btn-new-request").is_none(), "New request is Requests' button");
    click(&mut rt, "seg-0-by-me");
    assert_eq!(rt.state().frames.shared.tab, Tab::ByMe);
    assert!(rt.find("item-0-2").is_some() && rt.find("item-1-2").is_some() && rt.find("row-0").is_none());
    click(&mut rt, "seg-0-requests");
    assert!(rt.find("item-0-3").is_some() && rt.find("btn-new-request").is_some());
    click(&mut rt, "btn-new-request");
    assert_eq!(rt.state().frames.intents, vec![Intent::Frame("DW-0504")]);
}

#[test]
fn accept_moves_the_invitation_into_the_list_and_decline_removes_it() {
    let mut rt = runtime(super::scenes::dw_0501);
    click(&mut rt, "btn-accept");
    assert!(rt.find("card-0").is_none(), "the card goes");
    assert_eq!(info(&rt, "row-0-name").text.as_deref(), Some("Autumn shoot"));
    assert!(rt.find("row-4").is_some(), "five rows now");
    assert!(
        rt.find("Shared-pill").is_none(),
        "nothing waits: no pill in the sidebar"
    );

    let mut rt = runtime(super::scenes::dw_0501);
    click(&mut rt, "btn-decline");
    assert!(rt.find("card-0").is_none() && rt.find("row-4").is_none() && rt.find("row-3").is_some());
}

#[test]
fn arrows_select_and_enter_asks_to_open_what_is_selected() {
    let mut rt = runtime(super::scenes::dw_0501);
    press(&mut rt, Key::Down);
    press(&mut rt, Key::Down);
    assert_eq!(rt.state().frames.shared.selected, Some(1));
    for _ in 0..9 {
        press(&mut rt, Key::Down);
    }
    assert_eq!(rt.state().frames.shared.selected, Some(3), "stops at the last row");
    press(&mut rt, Key::Up);
    press(&mut rt, Key::Enter);
    assert_eq!(
        rt.state().frames.intents,
        vec![Intent::OpenItem("s-harbourlight".into())]
    );

    // By me: a share with people opens People with access.
    let mut rt = runtime(super::scenes::dw_0502);
    press(&mut rt, Key::Down);
    press(&mut rt, Key::Enter);
    assert_eq!(rt.state().frames.intents, vec![Intent::Frame("DW-0511")]);
}

#[test]
fn roles_and_names_are_the_boards() {
    let rt = runtime(super::scenes::dw_0501);
    let well = info(&rt, "seg-0");
    assert_eq!(
        (well.role.as_deref(), well.label.as_deref()),
        (Some("tablist"), Some("Shared"))
    );
    let tab = info(&rt, "seg-0-with-me");
    assert_eq!(
        (tab.role.as_deref(), tab.label.as_deref()),
        (Some("tab"), Some("With me"))
    );
    assert!(tab.focusable, "a tab takes the keyboard");
    let accept = info(&rt, "btn-accept");
    assert_eq!(
        (accept.role.as_deref(), accept.label.as_deref()),
        (Some("button"), Some("Accept"))
    );
    assert!(accept.focusable);
    assert_eq!(info(&rt, "row-0").label.as_deref(), Some("Studio assets"));
    assert_eq!(
        info(&rt, "card-0").label.as_deref(),
        Some("Waiting for you: Autumn shoot")
    );
}

#[test]
fn an_account_with_nothing_shared_shows_the_three_empty_states() {
    for (scene, title) in [
        (super::scenes::dw_0508 as fn(&mut State), "Nothing shared with you"),
        (super::scenes::dw_0509, "You haven't shared anything"),
        (super::scenes::dw_0510, "No file requests"),
    ] {
        let rt = runtime(scene);
        assert_eq!(info(&rt, "empty-title").text.as_deref(), Some(title));
        assert_eq!(info(&rt, "item-count").text.as_deref(), Some("No items"));
        assert!(rt.find("row-0").is_none() && rt.find("item-0-0").is_none());
    }
    let mut rt = runtime(super::scenes::dw_0510);
    click(&mut rt, "btn-new-request-2");
    assert_eq!(rt.state().frames.intents, vec![Intent::Frame("DW-0504")]);
}
