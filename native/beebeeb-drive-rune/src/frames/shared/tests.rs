//! Shared: the switch, the invitation, selection and the intents, by pointer and by keyboard.

use super::Tab;
use crate::frames::Intent;
use crate::{State, init, view};
use rune::{Env, Key, KeyEvent, Mods, Runtime};

/// A window on a scene, wired as `main` wires the app: `AppSpec::on_key` gets the keys that arrive
/// while nothing has focus.
pub(crate) fn runtime(scene: fn(&mut State)) -> Runtime<State> {
    let mut state = init();
    scene(&mut state);
    let env = Env {
        system_dark: false,
        scale: 2.0,
    };
    let mut rt = Runtime::new(state, view, (1278.0, 798.0), env);
    rt.on_key = Some(crate::shell::host::on_key);
    settle(&mut rt);
    rt
}

/// Draw, and let what moves or leaves come to rest.
pub(crate) fn settle(rt: &mut Runtime<State>) {
    rt.frame(0.0);
    rt.settle_all();
    rt.frame(0.0);
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
    settle(rt);
}

pub(crate) fn press(rt: &mut Runtime<State>, key: Key) {
    rt.key(KeyEvent {
        key,
        mods: Mods::default(),
    });
    settle(rt);
}

#[test]
fn the_switch_shows_each_list_and_new_request_only_on_requests() {
    let mut rt = runtime(super::scenes::dw_0501);
    assert!(rt.find("card-0").is_some() && rt.find("row-3").is_some() && rt.find("row-4").is_none());
    assert!(rt.find("btn-new-request").is_none(), "New request is Requests' button");
    click(&mut rt, "seg-0-by-me");
    assert_eq!(rt.state.frames.shared.tab, Tab::ByMe);
    assert!(rt.find("item-0-2").is_some() && rt.find("item-1-2").is_some() && rt.find("row-0").is_none());
    click(&mut rt, "seg-0-requests");
    assert!(rt.find("item-0-3").is_some() && rt.find("btn-new-request").is_some());
    click(&mut rt, "btn-new-request");
    assert_eq!(rt.state.frames.intents, vec![Intent::Frame("DW-0504")]);
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
    assert_eq!(rt.state.frames.shared.selected, Some(1));
    for _ in 0..9 {
        press(&mut rt, Key::Down);
    }
    assert_eq!(rt.state.frames.shared.selected, Some(3), "stops at the last row");
    press(&mut rt, Key::Up);
    press(&mut rt, Key::Enter);
    assert_eq!(rt.state.frames.intents, vec![Intent::OpenItem("s-harbourlight".into())]);

    // By me: a share with people opens People with access.
    let mut rt = runtime(super::scenes::dw_0502);
    press(&mut rt, Key::Down);
    press(&mut rt, Key::Enter);
    assert_eq!(rt.state.frames.intents, vec![Intent::Frame("DW-0511")]);
}

#[test]
fn roles_and_names_are_the_boards() {
    let rt = runtime(super::scenes::dw_0501);
    let well = info(&rt, "seg-0");
    assert_eq!(
        (Some(well.role), well.label.as_deref()),
        (Some("tablist"), Some("Shared"))
    );
    let tab = info(&rt, "seg-0-with-me");
    assert_eq!((Some(tab.role), tab.label.as_deref()), (Some("tab"), Some("With me")));
    assert!(tab.focusable, "a tab takes the keyboard");
    let accept = info(&rt, "btn-accept");
    assert_eq!(
        (Some(accept.role), accept.label.as_deref()),
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
    assert_eq!(rt.state.frames.intents, vec![Intent::Frame("DW-0504")]);
}

#[test]
fn the_menu_is_the_boards_takes_the_keyboard_and_does_what_it_says() {
    use super::source::RequestState;
    // DW-0505: roles, names, and the keys the board shows.
    let mut rt = runtime(super::scenes::dw_0505);
    let menu = info(&rt, "menu");
    assert_eq!(
        (Some(menu.role), menu.label.as_deref()),
        (Some("menu"), Some("Shared with you"))
    );
    let leave = info(&rt, "mi-leave");
    assert_eq!(
        (Some(leave.role), leave.label.as_deref()),
        (Some("menuitem"), Some("Leave\u{2026}"))
    );
    assert!(leave.focusable);
    assert!(
        rt.find("mi-rename").is_none() && rt.find("mi-move-to").is_none(),
        "no Rename or Move on someone else's file"
    );
    // Up from Leave… is Details; Enter asks for the details panel and closes the menu.
    press(&mut rt, Key::Up);
    assert_eq!(rt.state.frames.shared.menu.map(|m| m.hot), Some(4));
    press(&mut rt, Key::Enter);
    assert_eq!(rt.state.frames.intents, vec![Intent::Frame("DW-0411")]);
    assert!(rt.find("menu").is_none(), "the menu closes");

    // Escape closes it and does nothing.
    let mut rt = runtime(super::scenes::dw_0505);
    press(&mut rt, Key::Escape);
    assert!(rt.find("menu").is_none() && rt.state.frames.intents.is_empty());

    // Make available offline changes the row's PC state.
    let mut rt = runtime(super::scenes::dw_0505);
    click(&mut rt, "mi-make-available-offline");
    assert_eq!(
        rt.state.frames.shared.source.with_me()[1].pc,
        Some(crate::ui::file_list::PcState::AvailableOffline)
    );

    // DW-0506: Stop sharing ends the link straight away; the selection stays in the list.
    let mut rt = runtime(super::scenes::dw_0506);
    click(&mut rt, "mi-stop-sharing");
    assert_eq!(rt.state.frames.shared.source.links().len(), 2);
    assert!(rt.find("menu").is_none() && rt.find("item-1-2").is_none());
    // A share with people offers People and Stop sharing only.
    let mut rt = runtime(super::scenes::dw_0502);
    rt.state.frames.shared.selected = Some(0);
    rt.state.frames.shared.menu = Some(super::parts::menu::Open::at(500.0, 200.0));
    settle(&mut rt);
    assert!(
        rt.find("mi-people").is_some() && rt.find("mi-stop-sharing").is_some() && rt.find("mi-copy-link").is_none()
    );
    click(&mut rt, "mi-people");
    assert_eq!(rt.state.frames.intents, vec![Intent::Frame("DW-0511")]);

    // DW-0507: Close request closes it; a closed request then offers View files and Remove.
    let mut rt = runtime(super::scenes::dw_0507);
    press(&mut rt, Key::End);
    press(&mut rt, Key::Enter);
    assert_eq!(rt.state.frames.shared.source.requests()[1].state, RequestState::Closed);
    rt.state.frames.shared.menu = Some(super::parts::menu::Open::at(520.0, 222.0));
    settle(&mut rt);
    assert!(rt.find("mi-view-files").is_some() && rt.find("mi-remove").is_some() && rt.find("mi-copy-link").is_none());
    click(&mut rt, "mi-remove");
    assert_eq!(rt.state.frames.shared.source.requests().len(), 3);
}
