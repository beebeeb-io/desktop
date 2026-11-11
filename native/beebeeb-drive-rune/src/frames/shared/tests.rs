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

/// The key of the element that has keyboard focus.
pub(crate) fn focused(rt: &Runtime<State>) -> Option<String> {
    rt.elements()
        .into_iter()
        .find(|e| e.focused && !e.exiting)
        .and_then(|e| e.key)
}

/// A secondary press at a point (window points).
pub(crate) fn right_click_at(rt: &mut Runtime<State>, x: f32, y: f32) {
    rt.secondary_down(x, y);
    rt.secondary_up(x, y);
    settle(rt);
}

pub(crate) fn press_with(rt: &mut Runtime<State>, key: Key, mods: Mods) {
    rt.key(KeyEvent { key, mods });
    settle(rt);
}

pub(crate) fn alt() -> Mods {
    Mods {
        alt: true,
        ..Mods::default()
    }
}

pub(crate) fn space() -> Key {
    Key::Text(" ".into())
}

/// The section reached as in the running app: from Files, by the sidebar.
pub(crate) fn visited(section: crate::shell::app::Section) -> Runtime<State> {
    let mut state = init();
    crate::frames::go(&mut state, section);
    let env = Env {
        system_dark: false,
        scale: 2.0,
    };
    let mut rt = Runtime::new(state, view, (1278.0, 798.0), env);
    rt.on_key = Some(crate::shell::host::on_key);
    settle(&mut rt);
    rt
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
    assert_eq!(
        rt.state.frames.intents,
        vec![Intent::About("DW-0511", "p-lisbon".into())]
    );
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
    assert_eq!(rt.state.frames.shared.menu.as_ref().map(|m| m.hot), Some(4));
    press(&mut rt, Key::Enter);
    assert_eq!(
        rt.state.frames.intents,
        vec![Intent::About("DW-0411", "s-brand-review".into())]
    );
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
    assert_eq!(
        rt.state.frames.intents,
        vec![Intent::About("DW-0511", "p-lisbon".into())]
    );

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

// ---- The review of PR 293 ----

/// P1-1: the Menu key acts on the item that is highlighted, never on the one that was clicked.
#[test]
fn the_menu_key_opens_the_menu_of_the_highlighted_item() {
    // Click Budget 2026.xlsx, Down: Harbour.heic is highlighted AND focused.
    let mut rt = runtime(super::scenes::dw_0502);
    click(&mut rt, "item-1-0");
    press(&mut rt, Key::Down);
    assert_eq!(rt.state.frames.shared.selected, Some(4));
    assert_eq!(
        focused(&rt).as_deref(),
        Some("item-1-1"),
        "focus goes with the selection"
    );
    press(&mut rt, Key::ContextMenu);
    assert!(rt.find("menu").is_some());
    assert_eq!(rt.state.frames.shared.selected, Some(4), "still Harbour.heic");
    press(&mut rt, Key::End);
    press(&mut rt, Key::Enter);
    let left: Vec<String> = rt
        .state
        .frames
        .shared
        .source
        .links()
        .into_iter()
        .map(|l| l.id)
        .collect();
    assert_eq!(
        left,
        ["l-budget", "l-quote"],
        "the link to Harbour.heic is the one stopped"
    );

    // Arrows only, focus nowhere before: the Menu key opens the selected item's menu.
    let mut rt = runtime(super::scenes::dw_0501);
    press(&mut rt, Key::Down);
    press(&mut rt, Key::Down);
    press(&mut rt, Key::ContextMenu);
    assert!(rt.find("menu").is_some(), "a selection made by arrows has a menu");
    assert_eq!(rt.state.frames.shared.selected, Some(1));
    // Shift+F10 is the same key.
    press(&mut rt, Key::Escape);
    assert!(rt.find("menu").is_none());
    assert_eq!(
        focused(&rt).as_deref(),
        Some("row-1"),
        "Escape gives focus back to the row"
    );
    press_with(
        &mut rt,
        Key::Function(10),
        Mods {
            shift: true,
            ..Mods::default()
        },
    );
    assert!(rt.find("menu").is_some());
    // The keyboard's menu hangs under its row, inside the window.
    let (menu, row) = (info(&rt, "menu").rect, info(&rt, "row-1").rect);
    assert!(
        (menu[1] - (row[1] + row[3])).abs() <= 3.0,
        "under the row: {menu:?} {row:?}"
    );
}

/// P2-1: a menu opened at the window's edge stays inside the window.
#[test]
fn a_menu_opened_at_the_edge_stays_inside_the_window() {
    let mut rt = runtime(super::scenes::dw_0501);
    // Rates 2026.xlsx, 18 points from the right edge.
    let row = info(&rt, "row-3").rect;
    right_click_at(&mut rt, 1260.0, row[1] + row[3] / 2.0);
    let menu = info(&rt, "menu").rect;
    assert!(
        menu[0] >= 0.0 && menu[0] + menu[2] <= 1278.0,
        "inside left and right: {menu:?}"
    );
    // At the bottom edge it opens upward.
    let mut rt = runtime(super::scenes::dw_0501);
    right_click_at(&mut rt, 600.0, 790.0);
    if let Some(menu) = rt.find("menu").map(|_| info(&rt, "menu").rect) {
        assert!(
            menu[1] >= 0.0 && menu[1] + menu[3] <= 798.0,
            "inside top and bottom: {menu:?}"
        );
    }
    // The arithmetic, at each edge and in the middle.
    use super::parts::menu::fit;
    assert_eq!(fit((430.0, 262.0), (220.0, 205.0), (1278.0, 798.0)), (430.0, 262.0));
    assert_eq!(fit((1260.0, 262.0), (220.0, 205.0), (1278.0, 798.0)), (1040.0, 262.0));
    assert_eq!(fit((430.0, 700.0), (220.0, 205.0), (1278.0, 798.0)), (430.0, 495.0));
    assert_eq!(fit((100.0, 100.0), (220.0, 205.0), (200.0, 150.0)), (0.0, 0.0));
}

/// P2-2: in the menu, Tab moves the lit item with focus, and the pointer lights one item.
#[test]
fn in_the_menu_tab_and_the_pointer_move_the_one_lit_item() {
    let mut rt = runtime(super::scenes::dw_0501);
    click(&mut rt, "row-1");
    press(&mut rt, Key::ContextMenu);
    assert_eq!(rt.state.frames.shared.menu.as_ref().map(|m| m.hot), Some(0));
    for _ in 0..3 {
        press(&mut rt, Key::Tab);
    }
    assert_eq!(rt.state.frames.shared.menu.as_ref().map(|m| m.hot), Some(3));
    assert_eq!(
        focused(&rt).as_deref(),
        Some("mi-show-in-file-explorer"),
        "focus is on the lit item"
    );
    press(&mut rt, Key::Enter);
    assert_eq!(
        rt.state.frames.intents,
        vec![Intent::About("DW-1180", "s-brand-review".into())],
        "Enter picks the item Tab went to"
    );

    // The pointer: the item it enters is the lit one, and the only one washed.
    let mut rt = runtime(super::scenes::dw_0505);
    let r = info(&rt, "mi-preview").rect;
    rt.pointer_move(r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
    settle(&mut rt);
    assert_eq!(rt.state.frames.shared.menu.as_ref().map(|m| m.hot), Some(1));
    press(&mut rt, Key::Enter);
    assert_eq!(
        rt.state.frames.intents,
        vec![Intent::Do("preview", "s-brand-review".into())]
    );
}

/// P2-3: the keys the menu shows are keys.
#[test]
fn the_keys_the_menu_shows_do_what_its_items_do() {
    let mut rt = runtime(super::scenes::dw_0501);
    press(&mut rt, Key::Down);
    press_with(&mut rt, Key::Enter, alt());
    assert_eq!(
        rt.state.frames.intents,
        vec![Intent::About("DW-0411", "s-studio-assets".into())],
        "Alt+Enter is Details, not Open"
    );
    press(&mut rt, space());
    assert_eq!(
        rt.state.frames.intents.last(),
        Some(&Intent::Do("preview", "s-studio-assets".into()))
    );
    // A link's menu shows neither key: they do nothing there.
    let mut rt = runtime(super::scenes::dw_0502);
    click(&mut rt, "item-1-0");
    press_with(&mut rt, Key::Enter, alt());
    press(&mut rt, space());
    assert!(rt.state.frames.intents.is_empty(), "{:?}", rt.state.frames.intents);
}

/// P2-4: the three dots are a button that opens the item's menu.
#[test]
fn the_more_button_opens_its_items_menu() {
    let mut rt = runtime(super::scenes::dw_0502);
    let more = info(&rt, "item-0-0-more-btn");
    assert_eq!(
        (Some(more.role), more.label.as_deref()),
        (Some("button"), Some("More actions for Lisbon trip"))
    );
    click(&mut rt, "item-0-0-more-btn");
    assert_eq!(rt.state.frames.shared.selected, Some(0));
    assert!(rt.find("mi-people").is_some() && rt.find("mi-stop-sharing").is_some());
    let menu = info(&rt, "menu").rect;
    assert!(menu[0] + menu[2] <= 1278.0, "inside the window: {menu:?}");
    // A link's and a request's too.
    press(&mut rt, Key::Escape);
    click(&mut rt, "item-1-2-more-btn");
    assert_eq!(rt.state.frames.shared.selected, Some(5));
    assert!(rt.find("mi-copy-link").is_some());
    let mut rt = runtime(super::scenes::dw_0503);
    click(&mut rt, "item-0-3-more-btn");
    assert_eq!(rt.state.frames.shared.selected, Some(3));
    assert!(rt.find("menu").is_some());
}

/// P2-5: "Click the people: People with access DW-0511".
#[test]
fn a_click_on_the_people_asks_for_people_with_access() {
    let mut rt = runtime(super::scenes::dw_0502);
    let people = info(&rt, "item-0-1-people");
    assert_eq!(
        (Some(people.role), people.label.as_deref()),
        (Some("button"), Some("People with access to Invoices 2026"))
    );
    click(&mut rt, "item-0-1-people");
    assert_eq!(
        rt.state.frames.intents,
        vec![Intent::About("DW-0511", "p-invoices".into())]
    );
    assert_eq!(rt.state.frames.shared.selected, Some(1));
}

/// P2-6: the switch moves with Left and Right, Down goes into the list, and the list and the
/// switch are one Tab stop each.
#[test]
fn the_switch_and_the_list_take_the_arrows_and_are_one_tab_stop_each() {
    let mut rt = runtime(super::scenes::dw_0501);
    click(&mut rt, "seg-0-by-me");
    assert_eq!(focused(&rt).as_deref(), Some("seg-0-by-me"));
    press(&mut rt, Key::Down);
    assert_eq!(
        rt.state.frames.shared.selected,
        Some(0),
        "Down from the switch goes into the list"
    );
    assert_eq!(focused(&rt).as_deref(), Some("item-0-0"));

    let mut rt = runtime(super::scenes::dw_0501);
    click(&mut rt, "seg-0-by-me");
    press(&mut rt, Key::Right);
    assert_eq!(rt.state.frames.shared.tab, Tab::Requests);
    assert_eq!(focused(&rt).as_deref(), Some("seg-0-requests"));
    press(&mut rt, Key::Right);
    assert_eq!(rt.state.frames.shared.tab, Tab::Requests, "stops at the last tab");
    press(&mut rt, Key::Left);
    press(&mut rt, Key::Left);
    assert_eq!(rt.state.frames.shared.tab, Tab::WithMe);

    // Tab from the switch: one stop in the list, then out of it.
    let mut rt = runtime(super::scenes::dw_0503);
    click(&mut rt, "seg-0-requests");
    let mut stops = Vec::new();
    for _ in 0..40 {
        press(&mut rt, Key::Tab);
        stops.push(focused(&rt).unwrap_or_default());
    }
    let in_list = stops
        .iter()
        .filter(|k| k.starts_with("item-0-"))
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(in_list.len(), 1, "the four requests are one Tab stop: {stops:?}");
    let tabs = stops
        .iter()
        .filter(|k| k.starts_with("seg-0-"))
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(tabs.len(), 1, "the switch is one Tab stop: {stops:?}");
}

/// P2-7: the status line's words and a photo's thumbnail are the source's, not a view's.
#[test]
fn the_status_words_and_thumbnails_come_from_the_sources() {
    struct OtherPc;
    impl crate::frames::PcStatus for OtherPc {
        fn sync_words(&self) -> String {
            "Syncing 3 items".into()
        }
        fn summary(&self) -> String {
            "1 on this PC".into()
        }
    }
    let mut rt = runtime(super::scenes::dw_0508);
    assert_eq!(info(&rt, "status-text").text.as_deref(), Some("Up to date"));
    rt.state.frames.pc = Box::new(OtherPc);
    settle(&mut rt);
    assert_eq!(info(&rt, "status-text").text.as_deref(), Some("Syncing 3 items"));
    assert_eq!(info(&rt, "status-details").text.as_deref(), Some("1 on this PC"));

    // A photo without a thumbnail draws the plain tile with a glyph, not the board's sample.
    use super::parts::table::{Kind, Thumb};
    let rt = crate::frames::shared::tests::runtime(crate::frames::trash::scenes::dw_0747);
    assert!(
        rt.find("row-3-icon").is_none(),
        "the fixture's photo has its sample picture"
    );
    assert_eq!(
        rt.state.frames.trash.source.items()[3].kind,
        Kind::Photo(Some(Thumb::BoardLeaves))
    );
    let mut rt = rt;
    let mut items = rt.state.frames.trash.source.items();
    items[3].kind = Kind::Photo(None);
    rt.state.frames.trash.source = Box::new(crate::frames::trash::source::BoardTrash::of(items));
    settle(&mut rt);
    assert!(rt.find("row-3-icon").is_some(), "no pixels: a glyph, not a picture");
}

/// P2-8: the search field of these sections is their own, and Enter asks for the search.
#[test]
fn the_search_field_is_not_files_and_enter_asks_for_the_search() {
    let mut rt = runtime(super::scenes::dw_0501);
    click(&mut rt, crate::ui::keys::SEARCH_KEY);
    for c in ["c", "o", "n"] {
        press(&mut rt, Key::Text(c.into()));
    }
    assert_eq!(rt.state.frames.search.text, "con");
    assert_eq!(rt.state.search.text, "", "Files' field is not typed into");
    press(&mut rt, Key::Enter);
    assert_eq!(rt.state.frames.intents, vec![Intent::Do("search", "con".into())]);
}

/// P2-9: Back is live when there is somewhere to go back to, and goes there.
#[test]
fn back_is_live_after_files_and_goes_back() {
    // The board's frame: the window's first place, Back and Forward dimmed.
    let rt = runtime(super::scenes::dw_0501);
    assert!(!info(&rt, "back").focusable && !info(&rt, "forward").focusable);
    // The running app: Files, then Shared.
    let mut rt = visited(crate::shell::app::Section::Shared);
    assert!(info(&rt, "back").focusable, "Back is live after Files");
    assert!(!info(&rt, "forward").focusable);
    click(&mut rt, "back");
    assert!(
        rt.state.sidebar_section == crate::ui::sidebar::SidebarSection::Files,
        "Back went to Files"
    );
}
