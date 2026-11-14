//! Tests of "Commands and keyboard shortcuts": what each page lists, the keys that open, turn
//! and close it, Close by pointer and by keyboard, names and roles, and a low window.

use rune::{Env, Key, KeyEvent, Mods, Runtime};

use super::commands::{APP, EDITOR, WINDOW};
use super::shortcuts::{self, BODY_KEY, CLOSE_KEY, List, Page};
use crate::shell::app::{Dialog, Event, Route, Section};
use crate::shell::host;
use crate::shell::registry::{self, FrameGroup};
use crate::{State, init};

fn runtime_sized(size: (f32, f32)) -> Runtime<State> {
    let env = Env {
        system_dark: false,
        scale: 2.0,
    };
    let mut rt = Runtime::new(init(), crate::view, size, env);
    rt.on_key = Some(host::on_key);
    rt.frame(0.0);
    rt
}

fn runtime() -> Runtime<State> {
    runtime_sized((1278.0, 798.0))
}

fn info(rt: &Runtime<State>, key: &str) -> rune::ElementInfo {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
        .unwrap_or_else(|| panic!("no element keyed {key}"))
}

fn has(rt: &Runtime<State>, key: &str) -> bool {
    rt.elements()
        .iter()
        .any(|e| e.key.as_deref() == Some(key) && !e.exiting)
}

fn press_with(rt: &mut Runtime<State>, key: Key, mods: Mods) {
    rt.key(KeyEvent { key, mods });
    rt.frame(0.016);
    rt.settle_all();
    rt.frame(0.016);
}

fn press(rt: &mut Runtime<State>, key: Key) {
    press_with(rt, key, Mods::default());
}

fn open(rt: &mut Runtime<State>) {
    let ctrl = Mods {
        ctrl: true,
        cmd: true,
        ..Mods::default()
    };
    press_with(rt, Key::Command('/'), ctrl);
    assert!(is_open(rt), "Ctrl+/ opens the list");
}

fn is_open(rt: &Runtime<State>) -> bool {
    rt.state.shell.app.dialog() == Some(&Dialog::Shortcuts)
}

/// The names of the lists the card draws, read from the tree (their headings, in capitals).
fn drawn(rt: &Runtime<State>) -> Vec<String> {
    (0..8)
        .filter(|c| has(rt, &format!("sc-{c}-head")))
        .map(|c| info(rt, &format!("sc-{c}-head")).text.unwrap_or_default())
        .collect()
}

fn capitals(lists: &[List]) -> Vec<String> {
    lists.iter().map(|list| list.name.to_uppercase()).collect()
}

fn count(lists: &[List]) -> usize {
    lists.iter().map(|list| list.commands.len()).sum()
}

#[test]
fn the_three_contents_hold_the_boards_lists_and_counts() {
    assert_eq!(capitals(APP), ["BEEBEEB", "FILE", "EDIT"]);
    assert_eq!(capitals(WINDOW), ["VIEW", "GO", "WINDOW", "HELP"]);
    assert_eq!(capitals(EDITOR), ["EDIT", "VIEW", "FORMAT", "INSERT"]);
    assert_eq!(APP.iter().map(|l| l.commands.len()).collect::<Vec<_>>(), [8, 22, 7]);
    assert_eq!(
        WINDOW.iter().map(|l| l.commands.len()).collect::<Vec<_>>(),
        [8, 10, 4, 6]
    );
    assert_eq!(
        EDITOR.iter().map(|l| l.commands.len()).collect::<Vec<_>>(),
        [11, 7, 16, 6]
    );
    assert_eq!((count(APP), count(WINDOW), count(EDITOR)), (37, 28, 40));
    // No command without its key or its home, and no Ctrl+Alt key (AltGr types characters).
    for (name, home) in [APP, WINDOW, EDITOR]
        .iter()
        .flat_map(|p| p.iter())
        .flat_map(|l| l.commands)
    {
        // ONE row has no home on the board: Free up space (board b4ffef881; golden DW-1232,
        // `sc-1-13-home` empty). It is drawn as the board draws it and named to the board's owner.
        let homeless = *name == "Free up space";
        assert!(!name.is_empty() && (home.is_empty() == homeless), "{name}: {home}");
        assert!(!home.contains("Ctrl+Alt"), "{name}: {home}");
    }
}

#[test]
fn ctrl_slash_opens_the_first_page_with_every_command_and_its_home() {
    let mut rt = runtime();
    open(&mut rt);
    assert_eq!(drawn(&rt), capitals(APP));
    for (c, list) in APP.iter().enumerate() {
        for (r, (name, home)) in list.commands.iter().enumerate() {
            assert_eq!(info(&rt, &format!("sc-{c}-{r}-name")).text.as_deref(), Some(*name));
            assert_eq!(info(&rt, &format!("sc-{c}-{r}-home")).text.as_deref(), Some(*home));
            assert!(has(&rt, &format!("sc-{c}-{r}-line")));
        }
    }
    assert_eq!(info(&rt, "sc-title").text.as_deref(), Some(shortcuts::TITLE));
    assert_eq!(info(&rt, "sc-sub").text.as_deref(), Some(shortcuts::SUB));
    // The frame behind stays drawn (Files), and the card is 1120 wide in the board's window.
    assert!(has(&rt, "sidebar"));
    assert_eq!(info(&rt, "sc-dlg").rect[2], shortcuts::WIDTH);
}

#[test]
fn page_down_and_the_arrows_turn_the_page_and_page_up_turns_back() {
    let mut rt = runtime();
    open(&mut rt);
    for (forward, back) in [
        (Key::PageDown, Key::PageUp),
        (Key::Right, Key::Left),
        (Key::Down, Key::Up),
    ] {
        press(&mut rt, forward);
        assert_eq!(rt.state.shortcuts.page, Page::Window);
        assert_eq!(drawn(&rt), capitals(WINDOW));
        assert_eq!(info(&rt, "sc-1-0-home").text.as_deref(), Some("Alt+\u{2190}"));
        press(&mut rt, back);
        assert_eq!(rt.state.shortcuts.page, Page::App);
        assert_eq!(drawn(&rt), capitals(APP));
    }
    assert!(is_open(&rt), "turning a page never closes the list");
}

#[test]
fn esc_closes_it_and_the_next_opening_is_the_first_page_again() {
    let mut rt = runtime();
    open(&mut rt);
    press(&mut rt, Key::PageDown);
    assert_eq!(rt.state.shortcuts.page, Page::Window);
    press(&mut rt, Key::Escape);
    assert!(!is_open(&rt));
    assert!(!has(&rt, "sc-dlg"));
    assert_eq!(rt.state.shortcuts.page, Page::App);
    open(&mut rt);
    assert_eq!(drawn(&rt), capitals(APP));
}

#[test]
fn close_closes_it_by_pointer_and_by_keyboard() {
    let mut rt = runtime();
    open(&mut rt);
    let r = info(&rt, CLOSE_KEY).rect;
    let (x, y) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(0.016);
    assert!(!is_open(&rt), "a click on Close");
    assert_eq!(
        registry::group_for(&rt.state.shell.app),
        FrameGroup::Section(Section::Files),
        "and only the list closed"
    );

    open(&mut rt);
    let mut stops = Vec::new();
    for _ in 0..6 {
        press(&mut rt, Key::Tab);
        let focus = rt.elements().into_iter().find(|e| e.focused).and_then(|e| e.key);
        stops.push(focus.clone());
        if focus.as_deref() == Some(CLOSE_KEY) {
            break;
        }
    }
    assert_eq!(
        stops.last().cloned().flatten().as_deref(),
        Some(CLOSE_KEY),
        "Tab reaches Close and nothing behind the list: {stops:?}"
    );
    press(&mut rt, Key::Text(" ".into()));
    assert!(!is_open(&rt), "Space on Close");
}

#[test]
fn a_document_open_shows_the_editors_lists_and_has_no_second_page() {
    let mut rt = runtime();
    rt.update(|s, _| {
        host::dispatch(s, Event::Navigate(Route::section(Section::Files).with_item("doc-1")));
    });
    assert_eq!(registry::group_for(&rt.state.shell.app), FrameGroup::Viewer);
    open(&mut rt);
    assert_eq!(drawn(&rt), capitals(EDITOR));
    assert_eq!(info(&rt, "sc-0-0-name").text.as_deref(), Some("Undo Typing"));
    press(&mut rt, Key::PageDown);
    assert_eq!(drawn(&rt), capitals(EDITOR));
    assert_eq!(rt.state.shortcuts.page, Page::App);
}

#[test]
fn names_and_roles_are_a_dialog_with_one_list_per_menu() {
    let mut rt = runtime();
    open(&mut rt);
    let dlg = info(&rt, "sc-dlg");
    assert_eq!(dlg.role, "dialog");
    assert_eq!(
        dlg.label.as_deref(),
        Some("Commands and keyboard shortcuts: Beebeeb, File, Edit")
    );
    assert_eq!(info(&rt, "sc-title").role, "heading");
    for (c, list) in APP.iter().enumerate() {
        let column = info(&rt, &format!("sc-{c}"));
        assert_eq!((column.role, column.label.as_deref()), ("list", Some(list.name)));
    }
    // Narrator reads each command with its key or its home.
    let rename = info(&rt, "sc-1-16");
    assert_eq!((rename.role, rename.label.as_deref()), ("listitem", Some("Rename: F2")));
    // The board's 22nd File command (b4ffef881) has no home: its name alone.
    let free_up = info(&rt, "sc-1-13");
    assert_eq!(
        (free_up.role, free_up.label.as_deref()),
        ("listitem", Some("Free up space"))
    );
    assert_eq!(info(&rt, "sc-1-13-name").text.as_deref(), Some("Free up space"));
    let close = info(&rt, CLOSE_KEY);
    assert_eq!((close.role, close.label.as_deref()), ("button", Some("Close")));
}

#[test]
fn in_the_smallest_window_the_card_stays_inside_it_and_the_lists_scroll() {
    let (w, h) = crate::shell::surfaces::MAIN_MIN_SIZE;
    let mut rt = runtime_sized((w, h));
    open(&mut rt);
    let card = info(&rt, "sc-dlg").rect;
    assert!(card[0] >= 24.0 && card[0] + card[2] <= w - 24.0 + 0.01, "{card:?}");
    assert!(
        card[1] >= crate::TITLEBAR_H + 24.0 - 0.01 && card[1] + card[3] <= h - 24.0 + 0.01,
        "{card:?}"
    );
    // Close stays in view under the lists, which give up the height.
    let close = info(&rt, CLOSE_KEY).rect;
    assert!(close[1] + close[3] <= card[1] + card[3]);
    let body = info(&rt, BODY_KEY).rect;
    assert!(
        body[3] < 536.0,
        "the lists' viewport is lower than their content: {body:?}"
    );
}

/// DW-1234's scene, held here besides the frame gate: a document open, the dialog in front, the editor's four
/// lists in the board's order with the board's first and last commands, and Close and Esc go
/// back to the document.
#[test]
fn the_scene_of_dw_1234_is_the_editors_four_lists_over_an_open_document() {
    let scene = super::FRAMES
        .iter()
        .find(|(id, _)| *id == "DW-1234")
        .expect("DW-1234 is a scene of the tray group")
        .1;
    let mut rt = runtime();
    rt.update(|s, _| scene(s));
    rt.frame(0.0);
    assert_eq!(registry::group_for(&rt.state.shell.app), FrameGroup::Viewer);
    assert!(is_open(&rt), "the dialog is in front");
    assert_eq!(drawn(&rt), ["EDIT", "VIEW", "FORMAT", "INSERT"]);
    assert_eq!(drawn(&rt), capitals(EDITOR));
    assert_eq!(info(&rt, "sc-title").text.as_deref(), Some(shortcuts::TITLE));
    for (c, list) in EDITOR.iter().enumerate() {
        let last = list.commands.len() - 1;
        for r in [0, last] {
            let (name, home) = list.commands[r];
            assert_eq!(info(&rt, &format!("sc-{c}-{r}-name")).text.as_deref(), Some(name));
            assert_eq!(info(&rt, &format!("sc-{c}-{r}-home")).text.as_deref(), Some(home));
        }
        assert!(
            !has(&rt, &format!("sc-{c}-{}", last + 1)),
            "{}: no row past the last",
            list.name
        );
    }
    assert!(!has(&rt, "sc-4"), "four lists");
    let dialog = info(&rt, "sc-dlg");
    assert_eq!(dialog.role, "dialog");
    assert_eq!(
        dialog.label.as_deref(),
        Some("Commands and keyboard shortcuts: Edit, View, Format, Insert")
    );
    press(&mut rt, Key::Escape);
    assert!(!is_open(&rt), "Esc closes it");
    assert_eq!(
        registry::group_for(&rt.state.shell.app),
        FrameGroup::Viewer,
        "and the document is still open"
    );
    // Ctrl+/ from the document opens the same lists again.
    open(&mut rt);
    assert_eq!(drawn(&rt), capitals(EDITOR));
}
