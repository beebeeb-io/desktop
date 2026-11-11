//! Tests of the Files window: structure, keyboard, accessibility names.

use crate::{State, init, view};
use rune::{Env, Key, KeyEvent, Mods, Runtime};

fn runtime(dark: bool) -> Runtime<State> {
    let mut rt = Runtime::new(
        init(),
        view,
        (1278.0, 798.0),
        Env {
            system_dark: dark,
            scale: 2.0,
        },
    );
    rt.frame(0.0);
    rt
}

#[test]
fn the_window_has_its_regions_in_both_themes() {
    for dark in [false, true] {
        let rt = runtime(dark);
        for key in [
            "caption-bar",
            "sidebar",
            "toolbar",
            "column-headers",
            "rows",
            "detail-pane",
            "status-bar",
        ] {
            assert!(rt.find(key).is_some(), "{key} exists (dark: {dark})");
        }
    }
}

fn info(rt: &Runtime<State>, key: &str) -> rune::ElementInfo {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
        .unwrap_or_else(|| panic!("no element keyed {key}"))
}

fn press(rt: &mut Runtime<State>, key: Key, mods: Mods) {
    rt.key(KeyEvent { key, mods });
    rt.frame(0.0);
}

fn click(rt: &mut Runtime<State>, key: &str) {
    let r = info(rt, key).rect;
    let (x, y) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(0.0);
}

#[test]
fn arrows_move_the_selection_and_stop_at_the_ends() {
    let mut rt = runtime(false);
    click(&mut rt, "row-6-date");
    assert_eq!(rt.state.selected, Some(6));
    press(&mut rt, Key::Down, Mods::default());
    assert_eq!(rt.state.selected, Some(7), "Down selects Harbour.heic");
    assert_eq!(
        info(&rt, "detail-name").text.as_deref(),
        Some("Harbour.heic"),
        "the details panel follows the selection"
    );
    for _ in 0..20 {
        press(&mut rt, Key::Down, Mods::default());
    }
    assert_eq!(rt.state.selected, Some(10), "Down stops at the last row");
    for _ in 0..20 {
        press(&mut rt, Key::Up, Mods::default());
    }
    assert_eq!(rt.state.selected, Some(0), "Up stops at the first row");
}

#[test]
fn a_click_selects_the_row_it_lands_on() {
    let mut rt = runtime(false);
    click(&mut rt, "row-2-date");
    assert_eq!(rt.state.selected, Some(2));
    // The fill is the presented colour, which springs: let it land.
    rt.frame(1.0);
    rt.frame(1.0);
    assert!(info(&rt, "row-2").fill.a > 0.0, "the selected row is filled");
    assert_eq!(info(&rt, "row-6").fill.a, 0.0, "and the row selected before is not");
}

#[test]
fn enter_opens_a_folder_and_back_returns() {
    let mut rt = runtime(false);
    click(&mut rt, "row-1-date");
    assert!(!rt.state.can_go_back());
    press(&mut rt, Key::Enter, Mods::default());
    assert!(rt.state.can_go_back(), "Enter on Documents opens it");
    assert_eq!(rt.state.selected, None);
    press(
        &mut rt,
        Key::Left,
        Mods {
            alt: true,
            ..Mods::default()
        },
    );
    assert!(!rt.state.can_go_back(), "Alt+Left goes back");

    click(&mut rt, "row-6-date");
    press(&mut rt, Key::Enter, Mods::default());
    assert!(!rt.state.can_go_back(), "Enter on a file opens no folder");
}

#[test]
fn ctrl_f_puts_focus_in_the_search_field() {
    let mut rt = runtime(false);
    click(&mut rt, "row-3-date");
    assert!(!info(&rt, "search-input").focused);
    press(
        &mut rt,
        Key::Command('f'),
        Mods {
            cmd: true,
            ..Mods::default()
        },
    );
    assert!(info(&rt, "search-input").focused, "Ctrl+F focuses Search names");
    let before = rt.state.selected;
    press(&mut rt, Key::Down, Mods::default());
    assert_eq!(
        rt.state.selected, before,
        "arrows belong to the field while it has focus"
    );
}

#[test]
fn caption_buttons_and_controls_have_their_roles_and_names() {
    let rt = runtime(false);
    for (key, role, label) in [
        ("cap-min", "button", "Minimize"),
        ("cap-max", "button", "Maximize"),
        ("cap-close", "button", "Close"),
        ("back", "button", "Back"),
        ("forward", "button", "Forward"),
        ("view-toggle", "radiogroup", "View"),
        ("view-list", "radio", "as List"),
        ("sort-btn", "button", "Sort"),
        ("panel-toggle-btn", "button", "Details panel"),
        ("new-btn", "button", "New"),
        ("search-input", "textbox", "Search names"),
        ("settings-btn", "button", "Settings"),
        ("Files-row", "button", "Files"),
        ("rows", "list", "Files"),
        ("row-6", "listitem", "Contract draft.pdf"),
        ("row-1-pc", "image", "Available offline"),
        ("row-0-pc", "image", "Online only"),
        ("row-7-pc", "image", "Downloaded"),
        ("row-0-lock", "image", "Protected"),
        ("offline-switch", "switch", "Available offline"),
        ("detail-pane", "complementary", "Details"),
    ] {
        let e = info(&rt, key);
        assert_eq!((e.role, e.label.as_deref()), (role, Some(label)), "{key}");
    }
}

#[test]
fn every_pc_state_has_its_own_glyph() {
    use super::file_list::PcState;
    let states = [PcState::OnlineOnly, PcState::Downloaded, PcState::AvailableOffline];
    let glyphs: std::collections::BTreeSet<&str> = states.iter().map(|s| s.icon().name).collect();
    assert_eq!(glyphs.len(), 3);
    assert_eq!(PcState::of(Some("on_this_pc")), Some(PcState::Downloaded));
    assert_eq!(PcState::of(Some("online_only")).map(PcState::id), Some("online-only"));
    assert_eq!(
        PcState::of(Some("available_offline")).map(PcState::id),
        Some("available-offline")
    );
    assert_eq!(PcState::of(None), None);
}

#[test]
fn dates_read_as_the_board_writes_them() {
    use super::file_list::format_date;
    assert_eq!(format_date("2026-11-05T07:02:00Z"), "Today, 07:02");
    assert_eq!(format_date("2026-11-04T16:42:00Z"), "Yesterday, 16:42");
    assert_eq!(format_date("2026-11-01T14:00:00Z"), "Sunday");
    assert_eq!(format_date("2026-10-28T10:00:00Z"), "28 Oct 2026");
    assert_eq!(format_date("not a date"), "");
}

#[test]
fn the_gate_export_knows_every_keyed_text_and_icon() {
    let doc = super::gate::export(
        "DW-0401",
        init(),
        view,
        (1278.0, 798.0),
        false,
        crate::FONTS,
        crate::FAMILIES,
    );
    let els = doc["elements"].as_object().unwrap();
    assert_eq!(els["row-9-size"]["text"], "22.6 MB");
    assert_eq!(els["row-9-size"]["family"], "mono");
    assert_eq!(els["folder-title"]["weight"], 650);
    assert_eq!(els["row-1-pc"]["state"], "available-offline");
    assert_eq!(els["row-1-pc"]["filled"], true);
    assert_eq!(els["search-placeholder"]["text"], "Search names");
    assert!(els.len() >= 220, "at least the board's 220 keys, got {}", els.len());
}
