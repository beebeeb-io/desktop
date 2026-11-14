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

// ---- What the sections ask of the window (`carry_out`), and what they say without lists ----

use super::source::{Change, Kept, Standing};
use crate::frames::files::source::{Action, ActionError, Call, FilesFixture};

type Calls = std::sync::Arc<std::sync::Mutex<Vec<Call>>>;
type Fails = std::sync::Arc<std::sync::Mutex<Vec<(Action, ActionError)>>>;

/// A scene with a Files source that records what it is asked.
fn recording(scene: fn(&mut State)) -> (Runtime<State>, Calls, Fails) {
    let mut rt = runtime(scene, false);
    let fixture = FilesFixture::default();
    let (calls, fails) = (fixture.calls.clone(), fixture.fail.clone());
    rt.state.actions.source = Box::new(fixture);
    (rt, calls, fails)
}

/// One poll as the shell runs it, then a frame.
fn poll(rt: &mut Runtime<State>) {
    rt.update(|s, tx| {
        super::poll(s, tx);
    });
    rt.frame(0.0);
}

#[test]
fn show_in_file_explorer_asks_the_windows_files_source_and_says_a_refusal() {
    let (mut rt, calls, fails) = recording(scenes::dw_0428);
    rt.state.this_pc.ask(Intent::ShowInFileExplorer("c-21".into()));
    poll(&mut rt);
    assert_eq!(*calls.lock().unwrap(), [Call::ShowInFileExplorer(vec!["c-21".into()])]);
    assert!(rt.state.this_pc.refusal.is_none());
    assert!(rt.state.this_pc.take_intents().is_empty(), "carried out once");

    // Refused (sync does not run, the file is not on this PC): the window says so, in fixed words.
    fails
        .lock()
        .unwrap()
        .push((Action::ShowInFileExplorer, ActionError::Unavailable));
    rt.state.this_pc.ask(Intent::ShowInFileExplorer("c-21".into()));
    poll(&mut rt);
    assert_eq!(rt.state.this_pc.refusal.map(|(words, _)| words), Some(super::NOT_SHOWN));
    assert!(
        rt.elements()
            .iter()
            .any(|e| e.text.as_deref() == Some(super::NOT_SHOWN) && !e.exiting),
        "the refusal is on screen"
    );
}

#[test]
fn show_in_beebeeb_goes_to_trash_for_a_trashed_file_and_to_files_otherwise() {
    let (mut rt, _, _) = recording(scenes::dw_1301);
    let history = rt.state.this_pc.source.history();
    let trashed = history
        .iter()
        .find(|c| matches!(c.what, super::What::MovedToTrash { .. }))
        .expect("the board has a file moved to Trash")
        .id
        .clone();
    let other = history
        .iter()
        .find(|c| matches!(c.what, super::What::Uploaded { .. }))
        .expect("the board has an upload")
        .id
        .clone();
    rt.state.this_pc.ask(Intent::ShowInBeebeeb(trashed));
    poll(&mut rt);
    assert_eq!(rt.state.shell.app.route().section, super::Section::Trash);

    rt.state.this_pc.ask(Intent::ShowInBeebeeb(other));
    poll(&mut rt);
    assert_eq!(rt.state.shell.app.route().section, super::Section::Files);
}

#[test]
fn open_goes_into_a_kept_folder_and_opens_a_kept_file_through_the_files_source() {
    let (mut rt, calls, fails) = recording(scenes::dw_0715);
    let kept = rt.state.this_pc.kept();
    let folder = kept
        .iter()
        .find(|k| k.kind == super::source::Kind::Folder)
        .unwrap()
        .id
        .clone();
    let file = kept
        .iter()
        .find(|k| k.kind != super::source::Kind::Folder)
        .unwrap()
        .id
        .clone();

    // A file: its own app, through the Files source (no viewer is routed in this section).
    rt.state.this_pc.ask(Intent::Open(file.clone()));
    poll(&mut rt);
    assert_eq!(*calls.lock().unwrap(), [Call::Open(vec![file.clone()])]);
    assert_eq!(rt.state.shell.app.route().section, super::Section::AvailableOffline);

    // Refused: said.
    fails.lock().unwrap().push((Action::Open, ActionError::Offline));
    rt.state.this_pc.ask(Intent::Open(file));
    poll(&mut rt);
    assert_eq!(
        rt.state.this_pc.refusal.map(|(words, _)| words),
        Some(super::NOT_OPENED)
    );

    // An id the list does not hold: nothing is asked of anyone.
    let before = calls.lock().unwrap().len();
    rt.state.this_pc.ask(Intent::Open("not-kept".into()));
    poll(&mut rt);
    assert_eq!(calls.lock().unwrap().len(), before);

    // A folder: Files, inside it.
    rt.state.this_pc.ask(Intent::Open(folder.clone()));
    poll(&mut rt);
    let route = rt.state.shell.app.route();
    assert_eq!(route.section, super::Section::Files);
    assert_eq!(route.folder, Some(folder));
}

#[test]
fn new_goes_to_files_with_its_new_menu_open() {
    let (mut rt, _, _) = recording(scenes::dw_1301);
    rt.state.this_pc.ask(Intent::New);
    poll(&mut rt);
    assert_eq!(rt.state.shell.app.route().section, super::Section::Files);
    assert_eq!(
        rt.state.actions.menu.map(|m| m.kind),
        Some(crate::frames::files::MenuKind::New)
    );
}

#[test]
fn remove_all_shows_its_ask_and_removes_only_on_the_yes() {
    let (mut rt, _, _) = recording(scenes::dw_0715);
    click(&mut rt, "btn-remove-all-from-this-pc");
    poll(&mut rt);
    assert!(has(&rt, "dlg"), "the ask is on screen");
    assert_eq!(info(&rt, "dlg").role, "dialog");
    assert_eq!(text(&rt, "dlg-title"), "Remove all from this PC?");
    assert_eq!(
        text(&rt, "dlg-sub"),
        "3 items will be online only. Nothing is deleted from Beebeeb."
    );
    assert_eq!(rt.state.this_pc.kept_count(), 3, "nothing is removed before the answer");

    // Esc says no; so does Cancel. Nothing is removed, and a list key does not reach the list under the ask.
    press(&mut rt, Key::Escape);
    assert!(!has(&rt, "dlg"));
    assert_eq!(rt.state.this_pc.kept_count(), 3);
    click(&mut rt, "btn-remove-all-from-this-pc");
    poll(&mut rt);
    let selected = rt.state.this_pc.selected;
    press(&mut rt, Key::Down);
    assert_eq!(rt.state.this_pc.selected, selected, "the list is under the ask");
    click(&mut rt, "btn-remove-all-cancel");
    assert!(!has(&rt, "dlg"));
    assert_eq!(rt.state.this_pc.kept_count(), 3);

    // Yes: everything goes, the ask with it.
    click(&mut rt, "btn-remove-all-from-this-pc");
    poll(&mut rt);
    click(&mut rt, "btn-remove-all-yes");
    assert!(!has(&rt, "dlg"));
    assert_eq!(rt.state.this_pc.kept_count(), 0);
    assert_eq!(text(&rt, "empty-title"), "Nothing available offline");
}

#[test]
fn the_ask_does_not_outlive_its_section() {
    let (mut rt, _, _) = recording(scenes::dw_0715);
    click(&mut rt, "btn-remove-all-from-this-pc");
    poll(&mut rt);
    assert!(rt.state.this_pc.confirm_remove_all);
    rt.update(|s, _| {
        crate::shell::host::dispatch(
            s,
            crate::shell::app::Event::Navigate(crate::shell::app::Route::section(super::Section::SyncHistory)),
        );
    });
    poll(&mut rt);
    assert!(!rt.state.this_pc.confirm_remove_all, "gone with the section");
}

/// A source with no lists, in one standing; real, not a sample (the status line never says the board's counts).
struct NoLists(Standing);

impl ThisPcSource for NoLists {
    fn standing(&self) -> Standing {
        self.0
    }
    fn today(&self) -> chrono::NaiveDate {
        chrono::NaiveDate::from_ymd_opt(2026, 11, 1).unwrap()
    }
    fn history(&self) -> Vec<Change> {
        Vec::new()
    }
    fn problems(&self) -> Vec<Change> {
        Vec::new()
    }
    fn retry(&mut self, _: &str) {}
    fn retry_all(&mut self) {}
    fn kept(&self) -> Vec<Kept> {
        Vec::new()
    }
    fn remove_from_pc(&mut self, _: &str) {}
    fn remove_all_from_pc(&mut self) {}
}

fn with_source(section: super::Section, source: Box<dyn ThisPcSource>) -> Runtime<State> {
    with_state(section, source, |_| {})
}

/// A window on `section` with `source`, its state prepared before the first frame (nothing animates in).
fn with_state(section: super::Section, source: Box<dyn ThisPcSource>, prepare: fn(&mut State)) -> Runtime<State> {
    let mut state = init();
    crate::shell::host::dispatch(
        &mut state,
        crate::shell::app::Event::Navigate(crate::shell::app::Route::section(section)),
    );
    state.this_pc = super::Ui::new(source);
    prepare(&mut state);
    let env = Env {
        system_dark: false,
        scale: 2.0,
    };
    let mut rt = Runtime::new(state, view, (958.0, 638.0), env);
    rt.frame(0.0);
    rt
}

#[test]
fn without_lists_each_section_says_why_and_never_that_the_pc_is_empty() {
    let cases = [
        (Standing::Reading, "Reading this PC"),
        (Standing::Unreadable, "Couldn't read this PC"),
        (Standing::AnotherAccounts, "Not available for this account"),
    ];
    for (standing, title) in cases {
        for section in [super::Section::SyncHistory, super::Section::AvailableOffline] {
            let rt = with_source(section, Box::new(NoLists(standing)));
            assert_eq!(text(&rt, "empty-title"), title, "{standing:?} in {section:?}");
            assert!(has(&rt, "empty-sub"));
            for said in rt.elements().iter().filter_map(|e| e.text.clone()) {
                assert!(
                    !["No changes yet", "Nothing available offline", "No items"].contains(&said.as_str()),
                    "{standing:?} in {section:?} reads as an empty PC: {said}"
                );
            }
            assert!(
                !text(&rt, "status-details").contains("available offline"),
                "no count of something nobody read"
            );
        }
    }
    // Ready with nothing: the board's own empty states.
    let rt = with_source(super::Section::SyncHistory, Box::new(NoLists(Standing::Ready)));
    assert_eq!(text(&rt, "empty-title"), "No changes yet");
    let rt = with_source(super::Section::AvailableOffline, Box::new(NoLists(Standing::Ready)));
    assert_eq!(text(&rt, "empty-title"), "Nothing available offline");
}

#[test]
fn a_real_pc_never_draws_the_boards_sample_status_line() {
    const SAMPLE: &str = "4 on this PC \u{00B7} 3 available offline \u{00B7} 4 online only";
    // Nothing kept, and kept items with the row menu open: the two places the board keeps Files' sample counts.
    let rt = with_source(super::Section::AvailableOffline, Box::new(NoLists(Standing::Ready)));
    assert_eq!(text(&rt, "status-details"), "Nothing available offline");
    let mut rt = with_source(super::Section::AvailableOffline, Box::new(FixtureSource::synced()));
    rt.update(|s, _| s.this_pc.open_menu(0, (430.0, 190.0)));
    rt.frame(0.0);
    assert!(has(&rt, "menu"));
    assert_ne!(text(&rt, "status-details"), SAMPLE);
    assert!(text(&rt, "status-details").starts_with("3 available offline"));
}

#[test]
fn the_status_line_says_the_sources_sync_word_or_none() {
    // The window's own source has no word for sync (nothing syncs here): no dot, no word, never "Up to date".
    let mut rt = with_source(super::Section::SyncHistory, Box::new(FixtureSource::board()));
    rt.update(|s, _| {
        s.this_pc = super::Ui::new(Box::new(NoLists(Standing::Ready)));
        s.actions.source = Box::new(crate::frames::files::source::NoEngine);
    });
    rt.frame(0.0);
    assert!(!has(&rt, "status-text") && !has(&rt, "status-dot"));
    // The board's own source says "Up to date": that word, with its dot.
    rt.update(|s, _| s.actions.source = Box::new(FilesFixture::default()));
    rt.frame(0.0);
    assert_eq!(text(&rt, "status-text"), "Up to date");
    assert!(has(&rt, "status-dot"));

    // An item that did not sync: "Sync stopped", whatever the source's word.
    let rt = runtime(scenes::dw_0428, false);
    assert_eq!(text(&rt, "status-text"), "Sync stopped");
}

#[test]
fn the_sidebar_foot_draws_the_account_and_the_storage_the_window_read() {
    // The board's scene: the fixture's account and split, as before.
    let rt = runtime(scenes::dw_1301, false);
    assert_eq!(text(&rt, "account-name"), "Anna Visser");
    assert_eq!(text(&rt, "storage-files"), "Files 24 GB");

    // A window whose source names no account and has read no storage: no sample name, no figure, no breakdown.
    let rt = with_state(super::Section::SyncHistory, Box::new(NoLists(Standing::Ready)), |s| {
        s.actions.source = Box::new(crate::frames::files::source::NoEngine);
        s.storage_known = false;
        s.storage_split = None;
    });
    assert_eq!(text(&rt, "account-name"), "");
    assert!(has(&rt, "account-email-off") && !has(&rt, "account-email") && !has(&rt, "avatar"));
    assert_eq!(text(&rt, "storage-value"), "Unavailable");
    assert!(!has(&rt, "storage-breakdown"));
    assert_eq!(
        info(&rt, "meter-fill-0").rect[2],
        0.0,
        "no fill for storage nobody read"
    );
    for said in rt.elements().iter().filter_map(|e| e.text.clone()) {
        assert!(
            !said.contains("Anna") && !said.contains("northwind"),
            "a sample name: {said}"
        );
    }

    // Storage read, no split from the source: the total, one segment, no Files/Photos line.
    let rt = with_state(super::Section::SyncHistory, Box::new(NoLists(Standing::Ready)), |s| {
        s.actions.source = Box::new(crate::frames::files::source::NoEngine);
        s.storage_known = true;
        s.storage_split = None;
        s.storage.used_bytes = 240_000_000_000;
        s.storage.quota_bytes = 1_000_000_000_000;
    });
    assert!(text(&rt, "storage-value").contains(" of "));
    assert!(!has(&rt, "storage-breakdown"));
    assert!(info(&rt, "meter-fill-0").rect[2] > 0.0);
    assert_eq!(info(&rt, "meter-fill-1").rect[2], 0.0);
}
