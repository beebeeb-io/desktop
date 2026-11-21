//! DW-1562's Open With in the Files window (`open_with_menu.rs`), over the board's fixture: when it is offered, what
//! its submenu lists, what each line asks of the source, and its keys. The Office switches are set per test
//! (`open_with_menu::Ui::board_office`, as the DW-1562 scene does): the shipped ones are off, so with them nothing is
//! offered at all.

use super::menus::file_menu;
use super::open_with_menu::{self, OPEN_WITH};
use super::source::{AppChoice, Call, FilesFixture};
use super::tests::{Calls, NONE, asked, click, find, focused, info, press, right_click, runtime_of, settle};
use super::{MenuKind, Toast};
use crate::State;
use crate::frames::office::open_with::{
    AGREEMENT_ID, AGREEMENT_ROW, BOARD_STATUS, WORD_ON, agreement, scene_open_with,
};
use crate::frames::viewers::kinds::OfficeShips;
use crate::shell::app::Screen;
use crate::ui::menu::Entry;
use rune::{Key, Runtime};

/// Every Office type's switch on (`kinds::office_route_with(name, true, ALL_ON)`).
const ALL_ON: OfficeShips = OfficeShips {
    docx: true,
    doc: true,
    xlsx: true,
    xls: true,
    pptx: true,
};

/// The file menu of `s` with the Office switches `on` (every type) or as they ship.
/// Every Office type's switch off: the viewer shows no Office file (as before O-CP3 turned them on).
const ALL_OFF: OfficeShips = OfficeShips {
    docx: false,
    doc: false,
    xlsx: false,
    xls: false,
    pptx: false,
};

/// The file menu with every Office switch on, or every one off. (`None` is the switches as they ship: on since O-CP3.)
fn menu_with(s: &mut State, on: bool) -> Vec<Entry> {
    s.actions.open_with.board_office = Some(if on { ALL_ON } else { ALL_OFF });
    file_menu(s)
}

fn app(n: usize) -> AppChoice {
    AppChoice {
        id: format!(r"C:\Apps\app{n}.exe"),
        label: format!("App {n}"),
    }
}

/// The board's window with "Service agreement.docx" in its place (row 6), kept on this PC or not.
fn with_agreement(kept: bool) -> State {
    let mut s = crate::init();
    let mut row = agreement();
    if kept {
        row.pc_status = Some("available_offline".into());
    }
    s.listing.items.insert(AGREEMENT_ROW, row);
    s.actions.open_with.board_office = Some(ALL_ON);
    s
}

/// That window over a fixture answering `apps`, the agreement selected; what is asked of the source is kept.
fn spied_with(kept: bool, apps: Option<Vec<AppChoice>>) -> (Runtime<State>, Calls) {
    let mut s = with_agreement(kept);
    let mut fixture = FilesFixture::default();
    fixture.apps = apps;
    let calls = fixture.calls.clone();
    s.actions.source = Box::new(fixture);
    let mut rt = runtime_of(s);
    click(&mut rt, &format!("row-{AGREEMENT_ROW}-date"));
    (rt, calls)
}

/// Right-click the agreement, then "Open With": the submenu is drawn beside the file menu.
fn open_the_submenu(rt: &mut Runtime<State>) {
    right_click(rt, &format!("row-{AGREEMENT_ROW}-date"));
    assert_eq!(rt.state.actions.menu.map(|m| m.kind), Some(MenuKind::File));
    assert_eq!(info(rt, "menu-item-1-label").text.as_deref(), Some("Open With"));
    click(rt, "menu-item-1");
    settle(rt);
    assert_eq!(
        rt.state.actions.menu.and_then(|m| m.hot),
        Some(OPEN_WITH),
        "the menu stays, Open With highlighted"
    );
    assert!(find(rt, "menu-1").is_some(), "the submenu is drawn");
}

fn labels(entries: &[Entry]) -> Vec<&'static str> {
    entries
        .iter()
        .map(|entry| match entry {
            Entry::Item { label, .. } => *label,
            Entry::Head(label) => *label,
            Entry::Sep => "-",
        })
        .collect()
}

#[test]
fn open_with_offered_only_for_a_shown_office_file() {
    let mut s = with_agreement(false);
    s.select_only(AGREEMENT_ROW);
    let plain = menu_with(&mut s, false);
    // The switches off: the file menu of a Word file is the one every file has.
    assert_eq!(plain[0], Entry::keyed("open", "Open", "Enter"));
    assert!(
        plain
            .iter()
            .all(|entry| entry.id() != Some(open_with_menu::OPEN_WITH_ID))
    );
    // On: "Open (Beebeeb)", then "Open With ›", then the rest as before.
    let offered = menu_with(&mut s, true);
    assert_eq!(&labels(&offered)[..3], ["Open (Beebeeb)", "Open With", "Preview"]);
    assert_eq!(offered[0].id(), Some("open"), "the same key and action as Open");
    assert_eq!(
        offered[OPEN_WITH],
        Entry::keyed("open-with", "Open With", crate::ui::menu::SUBMENU)
    );
    assert_eq!(offered.len(), plain.len() + 1);
    assert_eq!(&offered[2..], &plain[1..], "nothing else of the menu changes");
    // As shipped: every type on since O-CP3, so the menu is the one offered.
    s.actions.open_with.board_office = None;
    assert_eq!(file_menu(&s), offered);
    // A PDF: unchanged, whatever the switches say.
    let contract = s.listing.items.iter().position(|i| i.id == "contract").unwrap();
    s.select_only(contract);
    assert_eq!(menu_with(&mut s, true), menu_with(&mut s, false));
    // Two files selected, the Word file one of them: unchanged.
    s.select_only(AGREEMENT_ROW);
    s.toggle_row(contract);
    assert_eq!(s.selected_rows().len(), 2);
    assert_eq!(menu_with(&mut s, true), menu_with(&mut s, false));
    // Word's switch alone on (the scene's): a Word file is offered it, a workbook is not.
    s.select_only(AGREEMENT_ROW);
    s.actions.open_with.board_office = Some(WORD_ON);
    assert_eq!(file_menu(&s)[OPEN_WITH].id(), Some(open_with_menu::OPEN_WITH_ID));
    let budget = s.listing.items.iter().position(|i| i.id == "budget").unwrap();
    s.select_only(budget);
    assert!(
        file_menu(&s)
            .iter()
            .all(|e| e.id() != Some(open_with_menu::OPEN_WITH_ID))
    );
    // A protected Word file (Backups): its own short menu, and no submenu beside its second line (Preview).
    s.listing.items[AGREEMENT_ROW].is_protected = true;
    s.select_only(AGREEMENT_ROW);
    s.actions.menu = Some(super::OpenMenu {
        kind: MenuKind::File,
        at: (430.0, 316.0),
        hot: Some(OPEN_WITH),
    });
    s.actions.open_with.board_office = Some(ALL_ON);
    assert_eq!(labels(&file_menu(&s))[..2], ["Open", "Preview"]);
    assert!(open_with_menu::offered_for(&s).is_none());
    assert_eq!(open_with_menu::at(&s), None, "no submenu over Preview");
}

#[test]
fn open_with_lists_beebeeb_the_apps_and_other() {
    let (mut rt, _) = spied_with(true, Some(vec![app(1), app(2)]));
    let lines = open_with_menu::entries(&rt.state);
    assert_eq!(labels(&lines), ["Beebeeb", "-", "App 1", "App 2", "-", "Other\u{2026}"]);
    // Drawn so, beside the file menu (`menu-1`, the board's second menu).
    open_the_submenu(&mut rt);
    for (n, words) in ["Beebeeb", "App 1", "App 2", "Other\u{2026}"].into_iter().enumerate() {
        assert_eq!(
            info(&rt, &format!("menu-1-item-{n}-label")).text.as_deref(),
            Some(words)
        );
    }
    assert!(find(&rt, "menu-1-item-4").is_none());
    // No app reported: Beebeeb and Other… alone, one line between them.
    let (rt, _) = spied_with(true, Some(Vec::new()));
    assert_eq!(
        labels(&open_with_menu::entries(&rt.state)),
        ["Beebeeb", "-", "Other\u{2026}"]
    );
}

#[test]
fn open_with_names_the_app_by_the_identity_it_was_handed() {
    // Each build has its own name: both of the menu's lines that name the app say the window's own, never a literal.
    use crate::shell::identity::AppIdentity;
    let [one, other] = AppIdentity::BOTH;
    assert_ne!(
        one.product_name(),
        other.product_name(),
        "the instrument tells the two apps apart"
    );
    for identity in AppIdentity::BOTH {
        let mut s = with_agreement(true);
        s.select_only(AGREEMENT_ROW);
        s.identity = identity;
        let name = identity.product_name();
        let open = format!("Open ({name})");
        assert_eq!(labels(&menu_with(&mut s, true))[0], open.as_str());
        assert_eq!(labels(&open_with_menu::entries(&s))[0], name);
    }
}

#[test]
fn an_app_press_on_a_file_not_kept_says_keep_first_and_opens_nothing() {
    for line in ["menu-1-item-1", "menu-1-item-3"] {
        let (mut rt, calls) = spied_with(false, Some(vec![app(1), app(2)]));
        open_the_submenu(&mut rt);
        click(&mut rt, line);
        settle(&mut rt);
        assert!(rt.state.actions.menu.is_none(), "{line} closes the menu");
        assert_eq!(asked(&calls), [], "{line}: nothing is opened, fetched or decrypted");
        assert_eq!(rt.state.live_toast(), Some(Toast::KeepFirst), "{line}");
        assert_eq!(
            info(&rt, "toast").label.as_deref(),
            Some(crate::frames::office::docs::KEEP_FIRST),
            "{line}"
        );
    }
}

/// Review of #467, P3-2: right after Keep on this PC the row is marked kept while the copy still comes down. Every line
/// that hands the file to another app (an app, Other…) asks the source at the press (the engine: kept AND the bytes
/// here), not the row's mark: "Keep this file on this PC first.", and nothing is opened.
#[test]
fn open_with_on_a_file_kept_but_not_here_yet_says_keep_first() {
    for line in ["menu-1-item-1", "menu-1-item-3"] {
        let mut s = with_agreement(true);
        let mut fixture = FilesFixture::default();
        fixture.apps = Some(vec![app(1), app(2)]);
        fixture.not_here = vec![AGREEMENT_ID.into()];
        let calls = fixture.calls.clone();
        s.actions.source = Box::new(fixture);
        let mut rt = runtime_of(s);
        click(&mut rt, &format!("row-{AGREEMENT_ROW}-date"));
        assert!(rt.state.selection_is_kept(), "the row says kept");
        open_the_submenu(&mut rt);
        click(&mut rt, line);
        settle(&mut rt);
        assert_eq!(asked(&calls), [], "{line}: nothing is opened, fetched or decrypted");
        assert_eq!(rt.state.live_toast(), Some(Toast::KeepFirst), "{line}");
        assert_eq!(
            info(&rt, "toast").label.as_deref(),
            Some(crate::frames::office::docs::KEEP_FIRST),
            "{line}"
        );
    }
}

#[test]
fn an_app_press_opens_the_kept_copy_in_that_app() {
    let (mut rt, calls) = spied_with(true, Some(vec![app(1), app(2)]));
    open_the_submenu(&mut rt);
    click(&mut rt, "menu-1-item-2");
    assert!(rt.state.actions.menu.is_none());
    assert_eq!(asked(&calls), [Call::OpenInApp(AGREEMENT_ID.into(), app(2).id)]);
    assert_eq!(rt.state.live_toast(), None, "nothing to say");
    // Other…: Windows' own chooser, on the same kept file.
    open_the_submenu(&mut rt);
    click(&mut rt, "menu-1-item-3");
    assert_eq!(asked(&calls).last(), Some(&Call::OpenWith(AGREEMENT_ID.into())));
    assert_eq!(asked(&calls).len(), 2);
}

#[test]
fn beebeeb_in_the_submenu_makes_the_same_open_call_as_open_beebeeb() {
    // What "Open (Beebeeb)" does…
    let (mut rt, by_open) = spied_with(true, Some(vec![app(1)]));
    right_click(&mut rt, &format!("row-{AGREEMENT_ROW}-date"));
    assert_eq!(info(&rt, "menu-item-0-label").text.as_deref(), Some("Open (Beebeeb)"));
    click(&mut rt, "menu-item-0");
    let opened = asked(&by_open);
    assert!(opened.is_empty(), "the viewer opens it, not another app: {opened:?}");
    let on_viewer = |rt: &Runtime<State>| matches!(rt.state.shell.app.screen(), Screen::Drive(route) if route.item.as_deref() == Some(AGREEMENT_ID));
    assert!(on_viewer(&rt));
    // …is what Beebeeb in the submenu does, and no other app is asked.
    let (mut rt, by_line) = spied_with(true, Some(vec![app(1)]));
    open_the_submenu(&mut rt);
    click(&mut rt, "menu-1-item-0");
    assert!(rt.state.actions.menu.is_none());
    assert_eq!(asked(&by_line), opened);
    assert!(on_viewer(&rt));
}

#[test]
fn dw_1562_scene_draws_the_boards_menu() {
    let mut s = crate::init();
    scene_open_with(&mut s);
    let rt = runtime_of(s);
    assert_eq!(
        rt.state.listing.items[AGREEMENT_ROW].name, "Service agreement.docx",
        "the board's row 6 of 12"
    );
    assert_eq!(rt.state.listing.items.len(), 12);
    // The board draws it without the details panel.
    assert!(!rt.state.show_detail_pane);
    assert!(find(&rt, "detail-pane").is_none());
    let mut with_pane = crate::init();
    scene_open_with(&mut with_pane);
    with_pane.show_detail_pane = true;
    assert!(
        find(&runtime_of(with_pane), "detail-pane").is_some(),
        "the control: the key is the pane's"
    );
    // Word's switch on for the scene alone (R-OW2 c); the status line in the board's words (R-OW2 b).
    assert_eq!(rt.state.actions.open_with.board_office, Some(WORD_ON));
    assert_eq!(info(&rt, "status-details").text.as_deref(), Some(BOARD_STATUS));
    let label = |key: &str| info(&rt, key).text;
    assert_eq!(label("menu-item-0-label").as_deref(), Some("Open (Beebeeb)"));
    assert_eq!(label("menu-item-1-label").as_deref(), Some("Open With"));
    assert!(
        find(&rt, "menu-item-1-chevron").is_some(),
        "Open With opens a menu beside it"
    );
    assert_eq!(label("menu-item-2-label").as_deref(), Some("Preview"));
    assert_eq!(label("menu-1-item-0-label").as_deref(), Some("Beebeeb"));
    assert_eq!(label("menu-1-item-1-label").as_deref(), Some("Microsoft Word"));
    assert_eq!(label("menu-1-item-2-label").as_deref(), Some("Other\u{2026}"));
    assert!(find(&rt, "menu-1-sep-0").is_some() && find(&rt, "menu-1-sep-1").is_some());
    // Beside the file menu, over its right edge, as the board places it.
    let (menu, sub) = (info(&rt, "menu"), info(&rt, "menu-1"));
    assert!(sub.rect[0] > menu.rect[0] + 200.0, "{:?} {:?}", menu.rect, sub.rect);
}

// ---- The keys (lead 03:22): Right-arrow into the submenu, Left-arrow and Escape back, Tab inside the menu ----

/// The file menu opened with the Menu key on the agreement, the keyboard on "Open With".
fn on_open_with(kept: bool) -> (Runtime<State>, Calls) {
    let (mut rt, calls) = spied_with(kept, Some(vec![app(1), app(2)]));
    press(&mut rt, Key::ContextMenu, NONE);
    press(&mut rt, Key::Down, NONE);
    press(&mut rt, Key::Down, NONE);
    settle(&mut rt);
    assert_eq!(rt.state.actions.menu.and_then(|m| m.hot), Some(OPEN_WITH));
    assert_eq!(
        focused(&rt).as_deref(),
        Some("menu-item-1"),
        "the keyboard is on Open With"
    );
    (rt, calls)
}

#[test]
fn open_with_right_arrow_opens_the_submenu_on_its_first_line() {
    let (mut rt, _) = on_open_with(true);
    assert_eq!(rt.state.actions.open_with.line, None);
    press(&mut rt, Key::Right, NONE);
    settle(&mut rt);
    assert_eq!(rt.state.actions.open_with.line, Some(0), "Beebeeb, the first line");
    assert_eq!(focused(&rt).as_deref(), Some("menu-1-item-0"));
    assert!(find(&rt, "menu-1").is_some());
    // Enter on Open With does the same.
    let (mut rt, _) = on_open_with(true);
    press(&mut rt, Key::Enter, NONE);
    settle(&mut rt);
    assert_eq!(rt.state.actions.open_with.line, Some(0));
    assert_eq!(focused(&rt).as_deref(), Some("menu-1-item-0"));
}

#[test]
fn open_with_left_arrow_goes_back_to_open_with() {
    let (mut rt, calls) = on_open_with(true);
    press(&mut rt, Key::Right, NONE);
    press(&mut rt, Key::Left, NONE);
    settle(&mut rt);
    assert_eq!(rt.state.actions.open_with.line, None);
    assert_eq!(
        rt.state.actions.menu.and_then(|m| m.hot),
        Some(OPEN_WITH),
        "the menu stays, on Open With"
    );
    assert_eq!(focused(&rt).as_deref(), Some("menu-item-1"));
    // The arrows are the file menu's again: Down goes to Preview, and the submenu goes.
    press(&mut rt, Key::Down, NONE);
    settle(&mut rt);
    assert_eq!(focused(&rt).as_deref(), Some("menu-item-2"));
    assert!(find(&rt, "menu-1").is_none());
    assert_eq!(asked(&calls), [], "nothing was picked");
}

#[test]
fn open_with_escape_goes_back_to_open_with_then_closes_the_menu() {
    let (mut rt, calls) = on_open_with(true);
    press(&mut rt, Key::Right, NONE);
    press(&mut rt, Key::Escape, NONE);
    settle(&mut rt);
    assert!(
        rt.state.actions.menu.is_some(),
        "the first Escape leaves the submenu only"
    );
    assert_eq!(rt.state.actions.open_with.line, None);
    assert_eq!(focused(&rt).as_deref(), Some("menu-item-1"));
    press(&mut rt, Key::Escape, NONE);
    settle(&mut rt);
    assert!(rt.state.actions.menu.is_none(), "the second closes the menu");
    assert_eq!(asked(&calls), []);
}

#[test]
fn open_with_arrows_and_enter_pick_a_line_of_the_submenu() {
    let (mut rt, calls) = on_open_with(true);
    press(&mut rt, Key::Right, NONE);
    // Down skips the line between Beebeeb and the apps: App 1, then App 2; Up goes back to App 1.
    press(&mut rt, Key::Down, NONE);
    assert_eq!(focused(&rt).as_deref(), Some("menu-1-item-1"));
    press(&mut rt, Key::Down, NONE);
    press(&mut rt, Key::Up, NONE);
    assert_eq!(rt.state.actions.open_with.line, Some(2), "App 1");
    // End is Other…, Home is Beebeeb.
    press(&mut rt, Key::End, NONE);
    assert_eq!(focused(&rt).as_deref(), Some("menu-1-item-3"));
    press(&mut rt, Key::Home, NONE);
    assert_eq!(focused(&rt).as_deref(), Some("menu-1-item-0"));
    press(&mut rt, Key::Down, NONE);
    press(&mut rt, Key::Enter, NONE);
    settle(&mut rt);
    assert!(rt.state.actions.menu.is_none());
    assert_eq!(asked(&calls), [Call::OpenInApp(AGREEMENT_ID.into(), app(1).id)]);
}

#[test]
fn tab_reaches_the_submenus_lines_inside_the_menu() {
    let (mut rt, _) = on_open_with(true);
    let shift = rune::Mods { shift: true, ..NONE };
    let mut seen = Vec::new();
    for _ in 0..5 {
        press(&mut rt, Key::Tab, NONE);
        settle(&mut rt);
        seen.push(focused(&rt).unwrap_or_default());
    }
    // From Open With into the submenu, then line by line, wrapping (the separators are not lines).
    assert_eq!(
        seen,
        [
            "menu-1-item-0",
            "menu-1-item-1",
            "menu-1-item-2",
            "menu-1-item-3",
            "menu-1-item-0"
        ]
    );
    assert!(rt.state.actions.menu.is_some(), "Tab never leaves the menu");
    // Shift+Tab from the first line goes back to Open With.
    press(&mut rt, Key::Tab, shift);
    settle(&mut rt);
    assert_eq!(focused(&rt).as_deref(), Some("menu-item-1"));
    assert_eq!(rt.state.actions.open_with.line, None);
}
