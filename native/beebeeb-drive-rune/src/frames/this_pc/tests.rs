//! Tests of Sync history and Available offline: every built frame's content, the filters, the
//! search, Try again, the row menu, the keyboard, names and roles.

use rune::{Env, Key, KeyEvent, Mods, Runtime};

use super::source::{FixtureSource, ThisPcSource};
use super::{Filter, Intent, scenes};
use crate::State;
use crate::shell::registry::view;
use crate::shell::state::init;

fn runtime(scene: fn(&mut State), dark: bool) -> Runtime<State> {
    let mut state = init();
    scene(&mut state);
    let env = Env {
        system_dark: dark,
        scale: 2.0,
    };
    let mut rt = Runtime::new(state, view, (1278.0, 798.0), env);
    // As the app runs: the shell's keys first, then this group's through the registry.
    rt.on_key = Some(crate::shell::host::on_key);
    rt.frame(0.0);
    rt
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

fn text(rt: &Runtime<State>, key: &str) -> String {
    info(rt, key).text.unwrap_or_default()
}

fn count(rt: &Runtime<State>, prefix: &str, suffix: &str) -> usize {
    rt.elements()
        .iter()
        .filter(|e| !e.exiting)
        .filter_map(|e| e.key.as_deref())
        .filter(|k| k.starts_with(prefix) && k.ends_with(suffix))
        .count()
}

fn press(rt: &mut Runtime<State>, key: Key) {
    rt.key(KeyEvent {
        key,
        mods: Mods::default(),
    });
    rt.frame(0.0);
}

fn click(rt: &mut Runtime<State>, key: &str) {
    let r = info(rt, key).rect;
    let (x, y) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(0.0);
}

fn focused(rt: &Runtime<State>) -> Option<String> {
    rt.elements().into_iter().find(|e| e.focused).and_then(|e| e.key)
}

/// Tab until `key` has focus.
fn tab_to(rt: &mut Runtime<State>, key: &str) {
    for _ in 0..80 {
        if focused(rt).as_deref() == Some(key) {
            return;
        }
        press(rt, Key::Tab);
    }
    panic!("Tab never reached {key}; focus is on {:?}", focused(rt));
}

const SCENES: [(&str, fn(&mut State)); 9] = [
    ("DW-1301", scenes::dw_1301),
    ("DW-1302", scenes::dw_1302),
    ("DW-1303", scenes::dw_1303),
    ("DW-0428", scenes::dw_0428),
    ("DW-1304", scenes::dw_1304),
    ("DW-1305", scenes::dw_1305),
    ("DW-0715", scenes::dw_0715),
    ("DW-1306", scenes::dw_1306),
    ("DW-0120", scenes::dw_0120),
];

#[test]
fn every_frame_draws_the_window_in_both_themes() {
    for (frame, scene) in SCENES {
        for dark in [false, true] {
            let rt = runtime(scene, dark);
            for key in ["caption-bar", "sidebar", "toolbar", "status-bar", "search-box"] {
                assert!(has(&rt, key), "{frame}: {key} (dark: {dark})");
            }
        }
    }
}

#[test]
fn the_board_fixture_holds_19_changes_3_problems_and_3_kept_items() {
    let source = FixtureSource::board();
    assert_eq!(source.history().len(), 19);
    assert_eq!(source.problems().len(), 3);
    assert_eq!(source.kept().len(), 3);
    assert_eq!(FixtureSource::nothing_kept().kept().len(), 0);
    assert_eq!(FixtureSource::just_signed_in().history().len(), 0);
}

#[test]
fn each_filter_lists_the_boards_changes() {
    // (scene, changes, days): DW-1301 19 over 5 days, DW-1302 9 over 4, DW-1303 10 over 5, DW-0428 3 on 1.
    for (scene, changes, days) in [
        (scenes::dw_1301 as fn(&mut State), 19, 5),
        (scenes::dw_1302, 9, 4),
        (scenes::dw_1303, 10, 5),
        (scenes::dw_0428, 3, 1),
        (scenes::dw_1304, 1, 1),
    ] {
        let rt = runtime(scene, false);
        assert_eq!(rt.state.this_pc.changes().len(), changes);
        assert_eq!(count(&rt, "day-", "-box"), days);
    }
    let rt = runtime(scenes::dw_1304, false);
    assert_eq!(text(&rt, "change-0-name"), "Contract draft.pdf");
    assert_eq!(text(&rt, "label-yesterday"), "YESTERDAY");
}

#[test]
fn a_click_on_a_filter_narrows_the_list_and_checks_its_tab() {
    let mut rt = runtime(scenes::dw_1301, false);
    assert_eq!(text(&rt, "seg-0-problems-3-label"), "Problems \u{00B7} 3");
    click(&mut rt, "seg-0-this-pc");
    assert_eq!(rt.state.this_pc.filter, Filter::ThisPc);
    assert!(rt.state.this_pc.changes().iter().all(|c| c.origin.label() == "This PC"));
    assert!(!has(&rt, "problems-bar"));
    click(&mut rt, "seg-0-problems-3");
    assert_eq!(rt.state.this_pc.filter, Filter::Problems);
    assert!(has(&rt, "problems-bar"), "the Can't sync bar is the Problems filter's");
    assert_eq!(info(&rt, "seg-0").role, "tablist");
    assert_eq!(info(&rt, "seg-0-problems-3").role, "tab");
}

#[test]
fn the_keyboard_reaches_a_filter_and_space_chooses_it() {
    let mut rt = runtime(scenes::dw_1301, false);
    tab_to(&mut rt, "seg-0-other-devices");
    press(&mut rt, Key::Text(" ".into()));
    assert_eq!(rt.state.this_pc.filter, Filter::OtherDevices);
}

#[test]
fn try_again_takes_the_item_off_the_list_and_the_counts_follow() {
    let mut rt = runtime(scenes::dw_0428, false);
    assert_eq!(text(&rt, "Sync history-badge"), "3");
    assert_eq!(text(&rt, "status-text"), "Sync stopped");
    assert_eq!(info(&rt, "btn-try-again").role, "button");
    assert_eq!(info(&rt, "btn-try-again").label.as_deref(), Some("Try again"));
    click(&mut rt, "btn-try-again");
    assert_eq!(rt.state.this_pc.problem_count(), 2);
    assert_eq!(text(&rt, "Sync history-badge"), "2");
    assert_eq!(text(&rt, "seg-0-problems-2-label"), "Problems \u{00B7} 2");
    assert_eq!(text(&rt, "change-0-name"), "Tram 28.mov");
    // The keyboard: Try all again.
    tab_to(&mut rt, "btn-try-all-again");
    press(&mut rt, Key::Text(" ".into()));
    assert_eq!(rt.state.this_pc.problem_count(), 0);
    assert!(!has(&rt, "Sync history-pill"), "no red count without problems");
    assert!(!has(&rt, "problems-bar"));
    assert_eq!(text(&rt, "status-text"), "Up to date");
}

#[test]
fn a_problem_says_why_in_words_and_that_nothing_was_lost() {
    let rt = runtime(scenes::dw_0428, false);
    for i in 0..3 {
        let why = text(&rt, &format!("change-{i}-sub"));
        assert!(why.ends_with("Nothing was lost."), "{why}");
        assert!(
            !why.chars().any(|c| c.is_ascii_digit()) || why.contains("5 tries"),
            "no error code: {why}"
        );
    }
    // Try again on the history's own problem row reaches the same item.
    let mut rt = runtime(scenes::dw_1301, false);
    click(&mut rt, "btn-try-again");
    assert_eq!(rt.state.this_pc.problem_count(), 2);
    assert_eq!(rt.state.this_pc.changes().len(), 18);
}

#[test]
fn search_matches_names_and_folders_and_esc_clears_it() {
    let mut rt = runtime(scenes::dw_1304, false);
    assert!(
        has(&rt, "search-clear"),
        "words in the field: the clear button replaces the hint"
    );
    assert!(!has(&rt, "kbd-hint"));
    press(&mut rt, Key::Escape);
    assert_eq!(rt.state.this_pc.search.text, "");
    assert_eq!(rt.state.this_pc.changes().len(), 19);
    assert!(has(&rt, "kbd-hint"));
    // A folder's name matches too, and the filter still applies.
    rt.state.this_pc.search = rune::TextEdit::new("invoices 2026");
    assert_eq!(rt.state.this_pc.changes().len(), 3);
    rt.state.this_pc.set_filter(Filter::ThisPc);
    assert_eq!(rt.state.this_pc.changes().len(), 0);
}

#[test]
fn enter_on_a_change_asks_to_show_its_file() {
    let mut rt = runtime(scenes::dw_1301, false);
    tab_to(&mut rt, "change-0");
    press(&mut rt, Key::Enter);
    assert_eq!(
        rt.state.this_pc.take_intents(),
        vec![Intent::ShowInBeebeeb("c-01".into())]
    );
    // On the Problems filter a row shows the file in File Explorer (DW-0428 > DW-1180).
    let mut rt = runtime(scenes::dw_0428, false);
    tab_to(&mut rt, "change-2");
    press(&mut rt, Key::Enter);
    assert_eq!(
        rt.state.this_pc.take_intents(),
        vec![Intent::ShowInFileExplorer("c-21".into())]
    );
}

#[test]
fn a_new_pc_has_no_history_and_says_so() {
    let rt = runtime(scenes::dw_1305, false);
    assert_eq!(text(&rt, "empty-title"), "No changes yet");
    assert_eq!(text(&rt, "seg-0-problems-0-label"), "Problems \u{00B7} 0");
    assert!(!has(&rt, "Sync history-pill"));
    assert_eq!(count(&rt, "change-", "-name"), 0);
}

#[test]
fn available_offline_lists_what_is_kept_with_its_total() {
    let rt = runtime(scenes::dw_0715, false);
    assert_eq!(text(&rt, "item-count"), "3 items \u{00B7} 1.4 GB");
    assert_eq!(
        text(&rt, "status-details"),
        "3 available offline \u{00B7} 1.4 GB on this PC"
    );
    assert_eq!(text(&rt, "row-0-size"), "1.4 GB");
    assert_eq!(text(&rt, "row-1-size"), "27.8 MB");
    assert_eq!(text(&rt, "row-2-size"), "84 KB");
    assert_eq!(text(&rt, "row-2-dash"), "Files");
    assert_eq!(info(&rt, "row-0-pc").label.as_deref(), Some("Available offline"));
    assert_eq!(info(&rt, "btn-remove-all-from-this-pc").role, "button");
}

#[test]
fn remove_all_asks_first_and_removes_only_after_the_yes() {
    let mut rt = runtime(scenes::dw_0715, false);
    click(&mut rt, "btn-remove-all-from-this-pc");
    assert_eq!(rt.state.this_pc.take_intents(), vec![Intent::AskRemoveAll]);
    assert_eq!(rt.state.this_pc.kept_count(), 3, "nothing is removed before the answer");
    rt.update(|s, _| s.this_pc.remove_all_confirmed());
    rt.frame(0.0);
    assert_eq!(text(&rt, "empty-title"), "Nothing available offline");
    assert!(
        !has(&rt, "btn-remove-all-from-this-pc"),
        "Remove all is hidden when nothing is kept"
    );
    assert!(!has(&rt, "Available offline-badge"));
}

#[test]
fn the_row_menu_opens_from_the_keyboard_and_removes_the_copy() {
    let mut rt = runtime(scenes::dw_0715, false);
    click(&mut rt, "row-1");
    assert_eq!(rt.state.this_pc.selected, Some(1));
    press(&mut rt, Key::ContextMenu);
    assert!(has(&rt, "menu"), "the Menu key opens the row's menu");
    assert_eq!(info(&rt, "menu").role, "menu");
    assert_eq!(
        info(&rt, "mi-remove-from-this-pc").label.as_deref(),
        Some("Remove from this PC")
    );
    press(&mut rt, Key::Down);
    press(&mut rt, Key::Down);
    press(&mut rt, Key::Enter);
    assert!(!has(&rt, "menu"));
    assert_eq!(rt.state.this_pc.kept_count(), 2);
    assert_eq!(text(&rt, "row-1-name"), "Budget 2026.xlsx");
    assert_eq!(text(&rt, "item-count"), "2 items \u{00B7} 1.4 GB");
}

/// Review of PR 306, P2: DW-1306 draws Files' counts in the status line under the open menu,
/// DW-0715 (no menu) the section's own. Each frame reads as the board draws it.
#[test]
fn the_status_line_under_the_row_menu_reads_as_dw_1306_draws_it() {
    const FILES: &str = "4 on this PC \u{00B7} 3 available offline \u{00B7} 4 online only";
    const SECTION: &str = "3 available offline \u{00B7} 1.4 GB on this PC";
    for dark in [false, true] {
        let mut rt = runtime(scenes::dw_1306, dark);
        assert!(has(&rt, "menu"));
        assert_eq!(text(&rt, "status-details"), FILES, "DW-1306, dark {dark}");
        assert_eq!(text(&rt, "status-text"), "Up to date");
        assert_eq!(count(&rt, "status-details", ""), 1, "one status line, never two");
        press(&mut rt, Key::Escape);
        assert!(!has(&rt, "menu"));
        assert_eq!(
            text(&rt, "status-details"),
            SECTION,
            "DW-0715 again once the menu closes"
        );
    }
}

#[test]
fn the_row_menu_closes_on_esc_and_its_items_ask_for_their_frames() {
    let mut rt = runtime(scenes::dw_1306, false);
    assert_eq!(rt.state.this_pc.menu.map(|m| m.hot), Some(2));
    press(&mut rt, Key::Escape);
    assert!(!has(&rt, "menu"));
    assert_eq!(rt.state.this_pc.kept_count(), 3, "Esc removes nothing");
    rt.state.this_pc.open_menu(0, (430.0, 190.0));
    rt.state.this_pc.pick(1);
    rt.state.this_pc.open_menu(0, (430.0, 190.0));
    rt.state.this_pc.pick(0);
    assert_eq!(
        rt.state.this_pc.take_intents(),
        vec![
            Intent::ShowInFileExplorer("k-documents".into()),
            Intent::Open("k-documents".into())
        ]
    );
}

#[test]
fn arrows_move_the_kept_selection_and_enter_opens() {
    let mut rt = runtime(scenes::dw_0715, false);
    press(&mut rt, Key::Down);
    press(&mut rt, Key::Down);
    assert_eq!(rt.state.this_pc.selected, Some(1));
    press(&mut rt, Key::Up);
    press(&mut rt, Key::Up);
    assert_eq!(rt.state.this_pc.selected, Some(0), "it stops at the first row");
    press(&mut rt, Key::Enter);
    assert_eq!(
        rt.state.this_pc.take_intents(),
        vec![Intent::Open("k-documents".into())]
    );
}

#[test]
fn the_sidebar_goes_to_the_other_section_and_the_gear_to_settings() {
    let mut rt = runtime(scenes::dw_1301, false);
    click(&mut rt, "Available offline-row");
    assert_eq!(
        rt.state.shell.app.route().section,
        crate::shell::app::Section::AvailableOffline
    );
    assert!(has(&rt, "column-headers"));
    click(&mut rt, "Files-row");
    assert_eq!(rt.state.shell.app.route().section, crate::shell::app::Section::Files);
}

#[test]
fn controls_the_section_has_no_use_for_are_disabled_not_focusable() {
    let rt = runtime(scenes::dw_1301, false);
    for key in ["view-list", "view-grid", "sort-btn", "panel-toggle-btn"] {
        assert!(!info(&rt, key).focusable, "{key} takes no focus in Sync history");
    }
    assert!(info(&rt, "new-btn").focusable);
    let mut rt = runtime(scenes::dw_0715, false);
    click(&mut rt, "new-btn");
    assert_eq!(rt.state.this_pc.take_intents(), vec![Intent::New]);
}

#[test]
fn day_labels_read_today_yesterday_then_the_day_in_words() {
    use chrono::NaiveDate;
    let d = |day, month| NaiveDate::from_ymd_opt(2026, month, day).expect("a date");
    let today = d(3, 11);
    assert_eq!(super::history::day_label(d(3, 11), today), "Today");
    assert_eq!(super::history::day_label(d(2, 11), today), "Yesterday");
    assert_eq!(super::history::day_label(d(1, 11), today), "Sunday 1 Nov");
    assert_eq!(super::history::day_label(d(31, 10), today), "Saturday 31 Oct");
}
