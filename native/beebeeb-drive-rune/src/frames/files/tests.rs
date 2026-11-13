//! Tests of the Files frames: keyboard and pointer (scripted input), roles and names, and what each
//! action asks of the source.

use super::source::{Action, ActionError, Call, FilesFixture};
use super::{Intent, MenuKind, Toast};
use crate::source::{DriveSource, LargeFixtureSource};
use crate::{State, init, load, view};
use rune::{Env, Key, KeyEvent, Mods, PhysicalInput, Runtime};
use std::sync::{Arc, Mutex};

thread_local! {
    static CLOCK: std::cell::Cell<f64> = const { std::cell::Cell::new(0.0) };
}

/// Draw frames while time moves on, so that what an input just mounted (a menu, a toast) has
/// arrived and can be pointed at: a `frame(0.0)` alone never ends its entrance.
pub(super) fn settle(rt: &mut Runtime<State>) {
    for _ in 0..3 {
        let now = CLOCK.with(|c| {
            c.set(c.get() + 0.4);
            c.get()
        });
        rt.frame(now);
    }
}

pub(super) fn runtime_of(state: State) -> Runtime<State> {
    let env = Env {
        system_dark: false,
        scale: 2.0,
    };
    let mut rt = Runtime::new(state, view, (1278.0, 798.0), env);
    rt.frame(0.0);
    settle(&mut rt);
    rt
}

/// The board's window, with focus in the list (on the selected row), as after a click there.
pub(super) fn runtime() -> Runtime<State> {
    let mut rt = runtime_of(init());
    click(&mut rt, "row-6-date");
    rt
}

pub(super) type Calls = Arc<Mutex<Vec<Call>>>;
pub(super) type Fails = Arc<Mutex<Vec<(Action, ActionError)>>>;

/// The board's window over a fixture the test keeps its handles on: what was asked of the source,
/// and what it is told to refuse.
pub(super) fn spied() -> (Runtime<State>, Calls, Fails) {
    spied_on(init())
}

pub(super) fn spied_on(mut state: State) -> (Runtime<State>, Calls, Fails) {
    let fixture = FilesFixture::default();
    let (calls, fails) = (fixture.calls.clone(), fixture.fail.clone());
    state.actions.source = Box::new(fixture);
    let mut rt = runtime_of(state);
    if find(&rt, "row-6-date").is_some() {
        click(&mut rt, "row-6-date");
    }
    (rt, calls, fails)
}

pub(super) fn asked(calls: &Calls) -> Vec<Call> {
    calls.lock().unwrap().clone()
}

/// The key of the element that has keyboard focus.
pub(super) fn focused(rt: &Runtime<State>) -> Option<String> {
    rt.elements()
        .into_iter()
        .find(|e| e.focused && !e.exiting)
        .and_then(|e| e.key)
}

pub(super) fn ids(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

pub(super) fn info(rt: &Runtime<State>, key: &str) -> rune::ElementInfo {
    find(rt, key).unwrap_or_else(|| panic!("no element keyed {key}"))
}

pub(super) fn find(rt: &Runtime<State>, key: &str) -> Option<rune::ElementInfo> {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
}

pub(super) const NONE: Mods = Mods {
    shift: false,
    cmd: false,
    alt: false,
    ctrl: false,
    logo: false,
};
pub(super) const CTRL: Mods = Mods {
    cmd: true,
    ctrl: true,
    ..NONE
};
pub(super) const SHIFT: Mods = Mods { shift: true, ..NONE };
pub(super) const ALT: Mods = Mods { alt: true, ..NONE };

pub(super) fn press(rt: &mut Runtime<State>, key: Key, mods: Mods) {
    rt.key(KeyEvent { key, mods });
    settle(rt);
}

pub(super) fn centre(rt: &Runtime<State>, key: &str) -> (f32, f32) {
    let r = info(rt, key).rect;
    (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0)
}

pub(super) fn click(rt: &mut Runtime<State>, key: &str) {
    click_with(rt, key, NONE);
}

pub(super) fn click_with(rt: &mut Runtime<State>, key: &str, mods: Mods) {
    let (x, y) = centre(rt, key);
    rt.physical_input(PhysicalInput::Modifiers(mods));
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    settle(rt);
    rt.physical_input(PhysicalInput::Modifiers(NONE));
}

pub(super) fn right_click(rt: &mut Runtime<State>, key: &str) {
    let (x, y) = centre(rt, key);
    rt.secondary_down(x, y);
    rt.secondary_up(x, y);
    settle(rt);
}

pub(super) fn names(rt: &Runtime<State>) -> Vec<String> {
    rt.state.listing.items.iter().map(|i| i.name.clone()).collect()
}

/// The labels of the open menu's items, in order (headings and separators left out).
pub(super) fn menu_labels(rt: &Runtime<State>) -> Vec<String> {
    rt.elements()
        .into_iter()
        .filter(|e| !e.exiting && matches!(e.role, "menuitem" | "menuitemcheckbox"))
        .filter_map(|e| e.label)
        .collect()
}

// ---- The known gaps of DW-0401 ----

#[test]
fn arrows_scroll_the_selected_row_into_view_in_a_5000_row_folder() {
    let mut state = init();
    let source = Box::new(LargeFixtureSource::new(5000));
    (state.listing, state.list_error) = load(source.as_ref(), None);
    state.source = source;
    state.clear_selection();
    state.show_detail_pane = false;
    let mut rt = runtime_of(state);
    click(&mut rt, "row-0-date");
    press(&mut rt, Key::Up, NONE);
    let rows = rt.state.listing.items.len();
    assert!(rows >= 5000, "{rows} rows");
    let list = info(&rt, "rows").rect;
    let in_view = |rt: &Runtime<State>, row: usize| {
        find(rt, &format!("row-{row}"))
            .is_some_and(|e| e.rect[1] >= list[1] - 0.5 && e.rect[1] + e.rect[3] <= list[1] + list[3] + 0.5)
    };
    for _ in 0..40 {
        press(&mut rt, Key::Down, NONE);
        settle(&mut rt);
    }
    assert_eq!(rt.state.selected, Some(40));
    assert!(!in_view(&rt, 0), "the list has scrolled");
    assert!(in_view(&rt, 40), "row 40 is in view after 40 times Down");
    press(&mut rt, Key::End, NONE);
    settle(&mut rt);
    settle(&mut rt);
    assert_eq!(rt.state.selected, Some(rows - 1));
    assert!(in_view(&rt, rows - 1), "End shows the last row");
    press(&mut rt, Key::Home, NONE);
    settle(&mut rt);
    settle(&mut rt);
    assert!(in_view(&rt, 0), "Home shows the first row");
}

#[test]
fn the_details_panel_shows_the_selected_item_from_the_source() {
    let mut rt = runtime();
    assert_eq!(info(&rt, "detail-name").text.as_deref(), Some("Contract draft.pdf"));
    click(&mut rt, "row-7-date");
    assert_eq!(info(&rt, "detail-name").text.as_deref(), Some("Harbour.heic"));
    assert_eq!(
        info(&rt, "detail-kind").text.as_deref(),
        Some("HEIC image \u{00B7} 3.1 MB")
    );
    assert_eq!(
        info(&rt, "detail-shared-value").text.as_deref(),
        Some("Anyone with the link")
    );
    press(&mut rt, Key::Up, NONE);
    press(&mut rt, Key::Up, NONE);
    assert_eq!(info(&rt, "detail-name").text.as_deref(), Some("Budget 2026.xlsx"));
    // A row the board never selects shows what the row itself says, and no invented facts.
    assert!(find(&rt, "detail-changed-on-value").is_none());
    assert!(find(&rt, "detail-version-history-value").is_none());
}

#[test]
fn enter_on_a_folder_shows_that_folders_contents_and_back_and_forward_return() {
    let mut rt = runtime();
    click(&mut rt, "row-3-date");
    assert_eq!(rt.state.listing.items[3].name, "Lisbon trip");
    press(&mut rt, Key::Enter, NONE);
    assert_eq!(
        names(&rt),
        [
            "Day 1",
            "Day 2",
            "Alfama.heic",
            "Belem.heic",
            "Sunset.heic",
            "Tickets.pdf",
            "Tram 28.mov"
        ]
    );
    assert_eq!(info(&rt, "crumb-0").text.as_deref(), Some("Files"));
    assert_eq!(info(&rt, "crumb-here").text.as_deref(), Some("Lisbon trip"));
    assert_eq!(info(&rt, "item-count").text.as_deref(), Some("7 items"));
    assert!(!info(&rt, "back").inert && rt.state.can_go_back());

    press(&mut rt, Key::Left, ALT);
    assert_eq!(names(&rt).len(), 11, "Alt+Left goes back to Files");
    assert!(find(&rt, "crumb-here").is_none());
    press(&mut rt, Key::Right, ALT);
    assert_eq!(names(&rt).len(), 7, "Alt+Right goes forward again");
    click(&mut rt, "crumb-0");
    assert_eq!(names(&rt).len(), 11, "a click on Files in the path goes up");
    click(&mut rt, "back");
    assert_eq!(
        names(&rt).len(),
        7,
        "the Back button returns to where the path was clicked"
    );
    click(&mut rt, "back");
    click(&mut rt, "forward");
    assert_eq!(names(&rt).len(), 7, "the Forward button works too");
}

#[test]
fn the_board_fixture_honours_the_folder_id() {
    let drive = super::source::BoardDrive::new();
    assert_eq!(drive.list(None).unwrap().items.len(), 11);
    let lisbon = drive.list(Some("f-lisbon")).unwrap();
    assert_eq!(
        (lisbon.items.len(), lisbon.folder.name.as_str(), lisbon.path.len()),
        (7, "Lisbon trip", 2)
    );
    assert_eq!(drive.list(Some(super::source::NEW_FOLDER_ID)).unwrap().items.len(), 0);
    assert!(drive.list(Some("contract")).is_err(), "a file is not a folder");
    assert!(drive.list(Some("nowhere")).is_err());
}

#[test]
fn enter_on_a_file_asks_to_open_it_and_space_asks_for_the_preview() {
    let (mut rt, calls, _) = spied();
    // A file of a type the window has no viewer for goes to its own app. (A PDF, a photo, …
    // open in the window: `frames::viewers::open_item`, tested there.)
    click(&mut rt, "row-9-date");
    press(&mut rt, Key::Enter, NONE);
    click(&mut rt, "row-6-date");
    press(&mut rt, Key::Text(" ".into()), NONE);
    // The preview is over the window (DW-0408; `actions_tests.rs`): Esc closes it.
    press(&mut rt, Key::Escape, NONE);
    // F2 renames in the row (DW-0410; `rest_tests.rs`), Esc leaves it.
    press(&mut rt, Key::Function(2), NONE);
    assert!(rt.state.actions.rename.is_some());
    press(&mut rt, Key::Escape, NONE);
    press(&mut rt, Key::Enter, ALT);
    assert_eq!(
        rt.state.actions.intents,
        [Intent::Frame("DW-0408"), Intent::Frame("DW-0411")]
    );
    assert!(rt.state.actions.dialog.is_some(), "Details is open (DW-0411)");
    assert_eq!(asked(&calls), [Call::Open(ids(&["q4plan"]))]);
}

// ---- Selection ----

#[test]
fn ctrl_click_shift_click_and_the_keyboard_select_several_rows() {
    let mut rt = runtime();
    click(&mut rt, "row-2-date");
    assert_eq!(rt.state.selected_rows(), [2]);
    click_with(&mut rt, "row-5-date", CTRL);
    assert_eq!(rt.state.selected_rows(), [2, 5], "Ctrl+click adds a row");
    click_with(&mut rt, "row-2-date", CTRL);
    assert_eq!(rt.state.selected_rows(), [5], "and takes one out");
    click_with(&mut rt, "row-8-date", SHIFT);
    assert_eq!(
        rt.state.selected_rows(),
        [2, 3, 4, 5, 6, 7, 8],
        "Shift+click selects the range from the row clicked last, as File Explorer does"
    );
    for row in [2, 5, 8] {
        assert!(
            info(&rt, &format!("row-{row}")).fill.a > 0.0,
            "row {row} is drawn selected"
        );
    }
    assert_eq!(info(&rt, "row-9").fill.a, 0.0);

    click(&mut rt, "row-1-date");
    press(&mut rt, Key::Down, SHIFT);
    press(&mut rt, Key::Down, SHIFT);
    assert_eq!(rt.state.selected_rows(), [1, 2, 3], "Shift+Down extends");
    press(&mut rt, Key::Up, SHIFT);
    assert_eq!(rt.state.selected_rows(), [1, 2], "Shift+Up takes it back");
    press(&mut rt, Key::Command('a'), CTRL);
    assert_eq!(rt.state.selected_rows().len(), 11, "Ctrl+A selects everything");
    press(&mut rt, Key::Escape, NONE);
    assert!(rt.state.selected_rows().is_empty(), "Escape clears the selection");
}

// ---- The file menu (DW-0402, DW-1104) ----

#[test]
fn a_right_click_the_menu_key_and_shift_f10_open_the_file_menu() {
    let mut rt = runtime();
    right_click(&mut rt, "row-7-date");
    assert_eq!(rt.state.selected, Some(7), "a right-click selects the row under it");
    let menu = info(&rt, "menu");
    assert_eq!((menu.role, menu.label.as_deref()), ("menu", Some("File")));
    assert_eq!(
        menu_labels(&rt),
        [
            "Open",
            "Preview",
            "Share\u{2026}",
            "Copy link",
            "Make available offline",
            // Harbour.heic is downloaded: there is a copy on this PC to free (PR 305).
            "Free up space",
            "Show in File Explorer",
            "Details",
            "Rename",
            "Move\u{2026}",
            "Duplicate",
            "Version history\u{2026}",
            "Move to Trash"
        ]
    );
    press(&mut rt, Key::Escape, NONE);
    assert!(find(&rt, "menu").is_none(), "Escape closes it");
    assert_eq!(rt.state.selected, Some(7), "and the selection stays");

    press(&mut rt, Key::ContextMenu, NONE);
    assert!(find(&rt, "menu").is_some(), "the Menu key opens it");
    press(&mut rt, Key::Escape, NONE);
    press(&mut rt, Key::Function(10), SHIFT);
    assert!(find(&rt, "menu").is_some(), "Shift+F10 opens it");
    let m = info(&rt, "menu").rect;
    assert!(
        m[0] >= 0.0 && m[1] >= 0.0 && m[0] + m[2] <= 1278.0 && m[1] + m[3] <= 798.0,
        "inside the window"
    );
}

#[test]
fn the_menu_is_worked_with_the_arrows_and_enter_and_takes_every_other_key() {
    let mut rt = runtime();
    press(&mut rt, Key::ContextMenu, NONE);
    press(&mut rt, Key::Down, NONE);
    press(&mut rt, Key::Down, NONE);
    press(&mut rt, Key::Down, NONE);
    let hot = rt.state.actions.menu.unwrap().hot.unwrap();
    assert_eq!(
        super::menus::file_menu(&rt.state)[hot].id(),
        Some("share"),
        "the separator is skipped"
    );
    press(&mut rt, Key::Up, NONE);
    press(&mut rt, Key::Up, NONE);
    press(&mut rt, Key::Up, NONE);
    let hot = rt.state.actions.menu.unwrap().hot.unwrap();
    assert_eq!(
        super::menus::file_menu(&rt.state)[hot].id(),
        Some("trash"),
        "Up from the first wraps to the last"
    );
    // Delete under an open menu does not reach the list.
    press(&mut rt, Key::Delete, NONE);
    assert_eq!(names(&rt).len(), 11);
    press(&mut rt, Key::Text("d".into()), NONE);
    let hot = rt.state.actions.menu.unwrap().hot.unwrap();
    assert_eq!(
        super::menus::file_menu(&rt.state)[hot].id(),
        Some("details"),
        "a letter goes to the next item that starts with it"
    );
    press(&mut rt, Key::Text("d".into()), NONE);
    press(&mut rt, Key::Enter, NONE);
    assert!(rt.state.actions.menu.is_none(), "Enter picks and closes");
    assert_eq!(names(&rt)[7], "Contract draft copy.pdf", "Duplicate was picked");
}

#[test]
fn a_protected_folder_and_several_items_get_the_menus_the_board_gives_them() {
    let mut rt = runtime();
    right_click(&mut rt, "row-0-date");
    assert_eq!(
        menu_labels(&rt),
        ["Open", "Preview", "Show in File Explorer", "Details"]
    );
    press(&mut rt, Key::Escape, NONE);

    click(&mut rt, "row-5-date");
    click_with(&mut rt, "row-6-date", CTRL);
    right_click(&mut rt, "row-6-date");
    assert_eq!(
        rt.state.selected_rows(),
        [5, 6],
        "a right-click inside the selection keeps it"
    );
    let labels = menu_labels(&rt);
    for gone in ["Rename", "Version history\u{2026}", "Details"] {
        assert!(
            !labels.iter().any(|l| l == gone),
            "{gone} is left out for several items"
        );
    }
    assert!(labels.iter().any(|l| l == "Move to Trash"));
    press(&mut rt, Key::Escape, NONE);

    // An item kept offline offers to remove it instead.
    click(&mut rt, "row-1-date");
    right_click(&mut rt, "row-1-date");
    assert!(menu_labels(&rt).iter().any(|l| l == "Remove from this PC"));
}

#[test]
fn menu_entries_pick_with_a_click_and_go_where_the_board_sends_them() {
    let (mut rt, calls, _) = spied();
    for (label, intent) in [
        ("Share\u{2026}", Intent::Frame("DW-0407")),
        ("Preview", Intent::Frame("DW-0408")),
        ("Details", Intent::Frame("DW-0411")),
        ("Version history\u{2026}", Intent::Frame("DW-0413")),
        // These two are calls on the source, not frames: the last frame asked for stays.
        ("Show in File Explorer", Intent::Frame("DW-0413")),
        ("Copy link", Intent::Frame("DW-0413")),
    ] {
        right_click(&mut rt, "row-6-date");
        let item = rt
            .elements()
            .into_iter()
            .find(|e| e.role == "menuitem" && e.label.as_deref() == Some(label))
            .unwrap_or_else(|| panic!("no menu item {label}"));
        let key = item.key.clone().unwrap();
        click(&mut rt, &key);
        assert!(rt.state.actions.menu.is_none(), "{label} closes the menu");
        assert_eq!(rt.state.actions.intents.last(), Some(&intent), "{label}");
        // Share, Preview, Details and Version history are dialogs over the window now
        // (`actions_tests.rs`): Esc closes each before the next pick.
        if rt.state.actions.dialog.is_some() {
            press(&mut rt, Key::Escape, NONE);
        }
    }
    assert_eq!(
        asked(&calls),
        [
            Call::ShowInFileExplorer(ids(&["contract"])),
            Call::CopyLink(ids(&["contract"]))
        ]
    );
    assert_eq!(rt.state.live_toast(), Some(Toast::LinkCopied), "Copy link says so");
    assert_eq!(info(&rt, "toast").role, "status");
    assert_eq!(info(&rt, "toast").label.as_deref(), Some("Link copied"));
}

// ---- Trash and Undo (DW-0405), Duplicate (DW-0406), offline (DW-0412) ----

#[test]
fn delete_moves_to_trash_without_a_question_and_undo_puts_it_back() {
    let mut rt = runtime();
    assert_eq!(info(&rt, "Trash-badge").text.as_deref(), Some("12"));
    press(&mut rt, Key::Delete, NONE);
    assert_eq!(names(&rt).len(), 10);
    assert!(!names(&rt).iter().any(|n| n == "Contract draft.pdf"));
    assert_eq!(info(&rt, "Trash-badge").text.as_deref(), Some("13"));
    assert_eq!(rt.state.live_toast(), Some(Toast::Trashed));
    assert_eq!(info(&rt, "toast-action").role, "button");
    press(&mut rt, Key::Command('z'), CTRL);
    assert_eq!(names(&rt)[6], "Contract draft.pdf", "Ctrl+Z puts it back in its place");
    assert_eq!(rt.state.selected, Some(6));
    assert_eq!(info(&rt, "Trash-badge").text.as_deref(), Some("12"));
    assert!(find(&rt, "toast").is_none());

    // Several rows, and Undo from the toast's button.
    click(&mut rt, "row-5-date");
    click_with(&mut rt, "row-8-date", CTRL);
    press(&mut rt, Key::Delete, NONE);
    assert_eq!(names(&rt).len(), 9);
    click(&mut rt, "toast-action");
    assert_eq!(names(&rt).len(), 11);
    assert_eq!(
        (names(&rt)[5].as_str(), names(&rt)[8].as_str()),
        ("Budget 2026.xlsx", "Northwind brand guide.pdf")
    );
    press(&mut rt, Key::Command('z'), CTRL);
    assert_eq!(names(&rt).len(), 11, "nothing is left to undo");
}

#[test]
fn backups_cannot_be_trashed_and_the_reason_is_asked_for() {
    let mut rt = runtime();
    click(&mut rt, "row-0-date");
    press(&mut rt, Key::Delete, NONE);
    assert_eq!(names(&rt).len(), 11);
    assert_eq!(rt.state.actions.intents, [Intent::Frame("DW-1110")]);
    assert!(rt.state.live_toast().is_none());
    assert_eq!(info(&rt, "Trash-badge").text.as_deref(), Some("12"));
}

#[test]
fn a_toast_goes_away_after_its_time() {
    let mut rt = runtime();
    press(&mut rt, Key::Delete, NONE);
    let until = rt.state.actions.toast_until.expect("a toast from an action has an end");
    let stay = until - std::time::Instant::now();
    assert!(
        stay > std::time::Duration::from_secs(5) && stay <= std::time::Duration::from_secs(6),
        "{stay:?}"
    );
    rt.update(|s, _| s.actions.toast_until = Some(std::time::Instant::now()));
    settle(&mut rt);
    assert!(rt.state.live_toast().is_none());
    assert!(find(&rt, "toast").is_none());
}

#[test]
fn duplicate_puts_the_copy_next_to_the_original_under_a_free_name() {
    let mut rt = runtime();
    press(&mut rt, Key::Command('d'), CTRL);
    assert_eq!(names(&rt)[7], "Contract draft copy.pdf");
    assert_eq!(rt.state.selected, Some(7), "the copy is selected");
    assert_eq!(info(&rt, "row-7-date").text.as_deref(), Some("Just now"));
    assert_eq!(
        info(&rt, "detail-shared-value").text.as_deref(),
        Some("Only you"),
        "a copy is not shared"
    );
    assert_eq!(rt.state.live_toast(), Some(Toast::Duplicated));
    click(&mut rt, "row-6-date");
    press(&mut rt, Key::Command('d'), CTRL);
    assert_eq!(names(&rt)[7], "Contract draft copy 2.pdf", "a taken name gets a number");
    assert_eq!(names(&rt).len(), 13);
    assert_eq!(super::source::copy_name("notes", &["notes copy"]), "notes copy 2");
}

#[test]
fn make_available_offline_keeps_the_file_and_the_menu_then_offers_to_remove_it() {
    let mut rt = runtime();
    assert_eq!(rt.state.listing.items[6].pc_status.as_deref(), Some("online_only"));
    rt.update(|s, tx| {
        s.toggle_keep_offline(tx);
    });
    settle(&mut rt);
    assert_eq!(
        rt.state.listing.items[6].pc_status.as_deref(),
        Some("available_offline")
    );
    assert_eq!(rt.state.live_toast(), Some(Toast::Keeping));
    right_click(&mut rt, "row-6-date");
    assert!(menu_labels(&rt).iter().any(|l| l == "Remove from this PC"));
}

// ---- Sort (DW-0404) and New (DW-0414) ----

#[test]
fn the_sort_menu_orders_the_rows_and_keeps_backups_first() {
    let mut rt = runtime();
    click(&mut rt, "sort-btn");
    assert_eq!(rt.state.actions.menu.map(|m| m.kind), Some(MenuKind::Sort));
    assert_eq!(info(&rt, "menu").label.as_deref(), Some("Sort"));
    let checked = |rt: &Runtime<State>| -> Vec<String> {
        rt.elements()
            .into_iter()
            .filter(|e| !e.exiting && e.role == "menuitemcheckbox")
            .filter_map(|e| e.label)
            .collect()
    };
    assert_eq!(checked(&rt), ["Name", "A to Z", "Folders first"]);

    let pick = |rt: &mut Runtime<State>, label: &str| {
        let key = rt
            .elements()
            .into_iter()
            .find(|e| e.label.as_deref() == Some(label) && e.role.starts_with("menuitem"))
            .and_then(|e| e.key)
            .unwrap_or_else(|| panic!("no {label}"));
        click(rt, &key);
    };
    pick(&mut rt, "Size");
    assert!(rt.state.actions.menu.is_some(), "a menu of ticks stays open");
    assert_eq!(
        checked(&rt),
        ["Size", "Largest first", "Folders first"],
        "the Order labels follow the key"
    );
    assert_eq!(names(&rt)[0], "Backups", "Backups stays the first row");
    assert_eq!(
        names(&rt)[5],
        "Q4 plan.key",
        "the largest file first, after the folders"
    );
    assert_eq!(
        rt.state.listing.items[rt.state.selected.unwrap()].name,
        "Contract draft.pdf",
        "the selection follows its row"
    );
    pick(&mut rt, "Smallest first");
    assert_eq!(names(&rt)[5], "Budget 2026.xlsx");
    pick(&mut rt, "Name");
    pick(&mut rt, "Z to A");
    assert_eq!(names(&rt)[1], "Shoot selects", "folders first, Z to A");
    pick(&mut rt, "A to Z");
    pick(&mut rt, "Folders first");
    assert_eq!(
        names(&rt)[..3],
        ["Backups", "Budget 2026.xlsx", "Contract draft.pdf"],
        "without Folders first, by name alone"
    );
    click(&mut rt, "sort-btn");
    assert!(rt.state.actions.menu.is_none(), "the button closes its own menu");
}

#[test]
fn new_opens_its_menu_and_every_entry_and_shortcut_goes_to_its_frame() {
    let mut rt = runtime();
    click(&mut rt, "new-btn");
    assert_eq!(info(&rt, "menu").label.as_deref(), Some("New"));
    assert_eq!(
        menu_labels(&rt),
        [
            "New folder",
            "New file\u{2026}",
            "New document\u{2026}",
            "Upload files\u{2026}",
            "Upload a folder\u{2026}"
        ]
    );
    for (n, expected) in ["DW-0417", "DW-1521", "DW-1558", "DW-0415", "DW-0480"]
        .into_iter()
        .enumerate()
    {
        if rt.state.actions.menu.is_none() {
            click(&mut rt, "new-btn");
        }
        for _ in 0..=n {
            press(&mut rt, Key::Down, NONE);
        }
        press(&mut rt, Key::Enter, NONE);
        if n == 0 {
            // New folder is this group's own dialog (DW-0417; `rest_tests.rs`).
            assert!(matches!(
                rt.state.actions.dialog,
                Some(super::dialogs::Dialog::NewFolder(_))
            ));
            press(&mut rt, Key::Escape, NONE);
            continue;
        }
        assert_eq!(rt.state.actions.intents.last(), Some(&Intent::Frame(expected)));
    }
    rt.state.actions.intents.clear();
    press(&mut rt, Key::Command('n'), Mods { shift: true, ..CTRL });
    assert!(rt.state.actions.dialog.is_some(), "Ctrl+Shift+N: New folder");
    press(&mut rt, Key::Escape, NONE);
    press(&mut rt, Key::Command('n'), CTRL);
    press(&mut rt, Key::Command('o'), CTRL);
    assert_eq!(
        rt.state.actions.intents,
        [Intent::Frame("DW-1521"), Intent::Frame("DW-0415")]
    );
}

// ---- Shortcuts a tooltip names ----

#[test]
fn every_shortcut_a_tooltip_names_does_something() {
    let mut rt = runtime();
    // Details panel  Alt+Shift+P
    assert!(find(&rt, "detail-pane").is_some());
    press(&mut rt, Key::Text("P".into()), Mods { shift: true, ..ALT });
    assert!(find(&rt, "detail-pane").is_none());
    // Ctrl+1..5: the sections.
    press(&mut rt, Key::Command('5'), CTRL);
    assert!(rt.state.sidebar_section == crate::ui::sidebar::SidebarSection::Trash);
    press(&mut rt, Key::Command('1'), CTRL);
    assert!(rt.state.sidebar_section == crate::ui::sidebar::SidebarSection::Files);
    // Forward is off, and says so, until Back has left a folder.
    assert!(!rt.state.can_go_forward());
    press(&mut rt, Key::Right, ALT);
    assert_eq!(names(&rt).len(), 11);
}

// ---- An empty folder (DW-1106) and drops (DW-0481) ----

#[test]
fn an_empty_folder_says_so_and_add_files_opens_new() {
    let (mut rt, calls, _) = spied();
    rt.update(|s, _| s.open_folder(Some(super::source::NEW_FOLDER_ID.to_string())));
    settle(&mut rt);
    assert_eq!(info(&rt, "empty-title").text.as_deref(), Some("Nothing here yet"));
    assert_eq!(info(&rt, "item-count").text.as_deref(), Some("No items"));
    assert_eq!(
        info(&rt, "status-details").text.as_deref(),
        Some("Nothing in this folder yet")
    );
    assert!(find(&rt, "column-headers").is_none() && find(&rt, "detail-pane").is_none());
    let add = info(&rt, "empty-add");
    assert_eq!(
        (add.role, add.label.as_deref(), add.focusable),
        ("button", Some("Add files"), true)
    );
    click(&mut rt, "empty-add");
    assert_eq!(rt.state.actions.menu.map(|m| m.kind), Some(MenuKind::New));
    press(&mut rt, Key::Escape, NONE);

    let (x, y) = centre(&rt, "empty-title");
    let files = vec![std::path::PathBuf::from("a.txt"), std::path::PathBuf::from("b.txt")];
    rt.drop_files(files.clone(), Some((x, y)));
    settle(&mut rt);
    assert_eq!(
        asked(&calls),
        [Call::Upload(Some(super::source::NEW_FOLDER_ID.to_string()), files)]
    );
}

#[test]
fn files_dropped_on_the_list_upload_into_the_folder_shown() {
    let (mut rt, calls, _) = spied();
    let (x, y) = centre(&rt, "row-9-date");
    let files = vec![std::path::PathBuf::from("photo.heic")];
    rt.drop_files(files.clone(), Some((x, y)));
    settle(&mut rt);
    assert_eq!(asked(&calls), [Call::Upload(None, files)]);
}

// ---- Roles and names ----

#[test]
fn menus_toasts_and_the_path_have_their_roles_and_names() {
    let mut rt = runtime();
    click(&mut rt, "sort-btn");
    let roles: Vec<&str> = rt
        .elements()
        .into_iter()
        .filter(|e| {
            e.key.as_deref().is_some_and(|k| {
                k.starts_with("menu-item-") && !k.contains("-label") && !k.contains("-tick") && !k.contains("-check")
            })
        })
        .map(|e| e.role)
        .collect();
    assert_eq!(
        roles.iter().filter(|r| **r == "presentation").count(),
        2,
        "two headings"
    );
    assert_eq!(
        roles.iter().filter(|r| **r == "menuitemcheckbox").count(),
        3,
        "three ticks"
    );
    assert_eq!(roles.iter().filter(|r| **r == "menuitem").count(), 4);
    assert_eq!(
        rt.elements()
            .into_iter()
            .filter(|e| !e.exiting
                && e.role == "separator"
                && e.key.as_deref().is_some_and(|k| k.starts_with("menu-sep-")))
            .count(),
        2
    );
    press(&mut rt, Key::Escape, NONE);

    rt.update(|s, _| s.open_folder(Some("f-lisbon".to_string())));
    settle(&mut rt);
    let up = info(&rt, "crumb-0");
    assert_eq!((up.role, up.focusable), ("link", true));
    assert_eq!(info(&rt, "rows").label.as_deref(), Some("Lisbon trip"));
}
