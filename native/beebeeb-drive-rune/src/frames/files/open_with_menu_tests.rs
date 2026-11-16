//! DW-1562's Open With in the Files window (`open_with_menu.rs`), over the board's fixture: when it is offered, what
//! its submenu lists, and what each line asks of the source. The Office route is set per test (`test_route`): the
//! shipped switches are off, so with them nothing is offered at all.

use super::menus::file_menu;
use super::open_with_menu::{self, OPEN_WITH, test_route};
use super::source::{AppChoice, Call, FilesFixture};
use super::tests::{Calls, asked, click, find, ids, info, right_click, runtime_of, settle};
use super::{MenuKind, Toast};
use crate::State;
use crate::frames::office::open_with::{AGREEMENT_ID, AGREEMENT_ROW, agreement, scene_open_with};
use crate::ui::menu::Entry;
use rune::Runtime;

/// The switches on: the route of `kinds::office_route_with(name, true, every type)` (V1-S): Word, Excel and
/// PowerPoint files are shown by the viewer.
fn switches_on(name: &str) -> bool {
    let ext = name.rsplit_once('.').map(|(_, ext)| ext.to_ascii_lowercase());
    matches!(
        ext.as_deref(),
        Some("docx" | "docm" | "dotx" | "dotm" | "doc" | "dot" | "xlsx" | "xlsm" | "xltx" | "xls" | "pptx" | "pptm")
    )
}

fn switches_off(_: &str) -> bool {
    false
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
    let plain = test_route::with(switches_off, || file_menu(&s));
    // The switches off: the file menu of a Word file is the one every file has.
    assert_eq!(plain[0], Entry::keyed("open", "Open", "Enter"));
    assert!(
        plain
            .iter()
            .all(|entry| entry.id() != Some(open_with_menu::OPEN_WITH_ID))
    );
    // On: "Open (Beebeeb)", then "Open With ›", then the rest as before.
    let offered = test_route::with(switches_on, || file_menu(&s));
    assert_eq!(&labels(&offered)[..3], ["Open (Beebeeb)", "Open With", "Preview"]);
    assert_eq!(offered[0].id(), Some("open"), "the same key and action as Open");
    assert_eq!(
        offered[OPEN_WITH],
        Entry::keyed("open-with", "Open With", crate::ui::menu::SUBMENU)
    );
    assert_eq!(offered.len(), plain.len() + 1);
    assert_eq!(&offered[2..], &plain[1..], "nothing else of the menu changes");
    // A PDF: unchanged, whatever the switches say.
    let contract = s.listing.items.iter().position(|i| i.id == "contract").unwrap();
    s.select_only(contract);
    assert_eq!(
        test_route::with(switches_on, || file_menu(&s)),
        test_route::with(switches_off, || file_menu(&s))
    );
    // Two files selected, the Word file one of them: unchanged.
    s.select_only(AGREEMENT_ROW);
    s.toggle_row(contract);
    assert_eq!(s.selected_rows().len(), 2);
    assert_eq!(
        test_route::with(switches_on, || file_menu(&s)),
        test_route::with(switches_off, || file_menu(&s))
    );
    // A protected Word file (Backups): its own short menu, and no submenu beside its second line (Preview).
    s.listing.items[AGREEMENT_ROW].is_protected = true;
    s.select_only(AGREEMENT_ROW);
    s.actions.menu = Some(super::OpenMenu {
        kind: MenuKind::File,
        at: (430.0, 316.0),
        hot: Some(OPEN_WITH),
    });
    test_route::with(switches_on, || {
        assert_eq!(labels(&file_menu(&s))[..2], ["Open", "Preview"]);
        assert!(open_with_menu::offered_for(&s).is_none());
        assert_eq!(open_with_menu::at(&s), None, "no submenu over Preview");
    });
}

#[test]
fn open_with_lists_beebeeb_the_apps_and_other() {
    test_route::with(switches_on, || {
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
    });
}

#[test]
fn an_app_press_on_a_file_not_kept_says_keep_first_and_opens_nothing() {
    test_route::with(switches_on, || {
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
    });
}

#[test]
fn an_app_press_opens_the_kept_copy_in_that_app() {
    test_route::with(switches_on, || {
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
    });
}

#[test]
fn beebeeb_in_the_submenu_opens_the_viewer() {
    test_route::with(switches_on, || {
        // What "Open (Beebeeb)" does…
        let (mut rt, by_open) = spied_with(true, Some(vec![app(1)]));
        right_click(&mut rt, &format!("row-{AGREEMENT_ROW}-date"));
        assert_eq!(info(&rt, "menu-item-0-label").text.as_deref(), Some("Open (Beebeeb)"));
        click(&mut rt, "menu-item-0");
        let opened = asked(&by_open);
        assert_eq!(opened, [Call::Open(ids(&[AGREEMENT_ID]))]);
        // …is what Beebeeb in the submenu does, and no other app is asked.
        let (mut rt, by_line) = spied_with(true, Some(vec![app(1)]));
        open_the_submenu(&mut rt);
        click(&mut rt, "menu-1-item-0");
        assert!(rt.state.actions.menu.is_none());
        assert_eq!(asked(&by_line), opened);
    });
}

#[test]
fn dw_1562_scene_draws_the_boards_menu() {
    test_route::with(switches_on, || {
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
    });
}
