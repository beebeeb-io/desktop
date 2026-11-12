//! Tests of the open-file group: every built frame's words, the keyboard, names and roles, what
//! each control asks of the source or the shell, and that decrypted content leaves memory when
//! the file is left.

use std::cell::RefCell;
use std::rc::Rc;

use rune::{Env, Key, KeyEvent, Mods, Runtime};

use super::source::{
    Body, FixtureViewerSource, OpenError, OpenFile, ViewerSource, board, count_words, percent, plain_lines,
    progress_words, size_words,
};
use super::{Intent, open, open_from, scene_file, set_scene};
use crate::shell::app::{Event, Screen};
use crate::{State, init, view};

/// Every built frame, and its first words.
const BUILT: [(&str, &str, &str); 5] = [
    ("DW-1516", "state-title", "Decrypting on this PC"),
    ("DW-1517", "state-title", "Too large to preview here"),
    ("DW-1518", "state-title", "Couldn't decrypt this file"),
    ("DW-1512", "run-0-0", "# Export this month's invoices as one CSV"),
    (
        "DW-1527",
        "banner-text",
        "Read-only: this file is larger than 2 MB. Open it with another app to edit it.",
    ),
];

type Calls = Rc<RefCell<Vec<String>>>;

/// The fixture, with what the viewer asked of it kept where a test can read it.
struct Recording {
    inner: FixtureViewerSource,
    calls: Calls,
}

impl ViewerSource for Recording {
    fn open(&mut self, id: &str) -> Result<OpenFile, OpenError> {
        self.calls.borrow_mut().push(format!("open {id}"));
        self.inner.open(id)
    }
    fn poll(&mut self, id: &str) -> Option<Body> {
        self.inner.poll(id)
    }
    fn cancel(&mut self, id: &str) {
        self.calls.borrow_mut().push(format!("cancel {id}"));
    }
    fn retry(&mut self, id: &str) -> Result<OpenFile, OpenError> {
        self.calls.borrow_mut().push(format!("retry {id}"));
        self.inner.retry(id)
    }
}

fn runtime_of(state: State, dark: bool) -> Runtime<State> {
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

fn runtime(frame: &str) -> Runtime<State> {
    let mut state = init();
    set_scene(&mut state, frame);
    runtime_of(state, false)
}

fn recording(file: &str) -> (Runtime<State>, Calls) {
    let calls = Calls::default();
    let mut state = init();
    let source = Recording {
        inner: FixtureViewerSource::new(),
        calls: calls.clone(),
    };
    assert!(open_from(&mut state, Box::new(source), file));
    (runtime_of(state, false), calls)
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
    rt.frame(0.0);
}

fn press(rt: &mut Runtime<State>, key: Key) {
    press_with(rt, key, Mods::default());
}

fn focused(rt: &Runtime<State>) -> Option<String> {
    rt.elements().into_iter().find(|e| e.focused).and_then(|e| e.key)
}

/// Tab until `key` has focus (at most 30 stops).
fn tab_to(rt: &mut Runtime<State>, key: &str) {
    for _ in 0..30 {
        if focused(rt).as_deref() == Some(key) {
            return;
        }
        press(rt, Key::Tab);
    }
    panic!("Tab never reached {key}; focus is on {:?}", focused(rt));
}

/// Tab to a control and press it with Space.
fn operate(rt: &mut Runtime<State>, key: &str) {
    tab_to(rt, key);
    press(rt, Key::Text(" ".into()));
}

fn intents(rt: &mut Runtime<State>) -> Vec<Intent> {
    rt.state.viewer.as_mut().expect("a file is open").take_intents()
}

fn on_a_file(rt: &Runtime<State>) -> bool {
    matches!(rt.state.shell.app.screen(), Screen::Drive(route) if route.item.is_some())
}

#[test]
fn every_built_frame_draws_its_words_in_both_themes() {
    for (frame, key, words) in BUILT {
        for dark in [false, true] {
            let mut state = init();
            set_scene(&mut state, frame);
            let rt = runtime_of(state, dark);
            assert_eq!(info(&rt, key).text.as_deref(), Some(words), "{frame} (dark: {dark})");
            // The window around it is the Files window's: caption, sidebar, toolbar, status line.
            for chrome in ["caption-bar", "toolbar", "crumb-name", "status-text", "more-btn"] {
                assert!(has(&rt, chrome), "{frame} has {chrome}");
            }
        }
    }
}

#[test]
fn the_toolbar_names_the_file_by_its_path_and_facts() {
    let rt = runtime("DW-1516");
    assert_eq!(info(&rt, "crumb-0").text.as_deref(), Some("Files"));
    assert_eq!(info(&rt, "crumb-1").text.as_deref(), Some("Lisbon trip"));
    assert_eq!(info(&rt, "crumb-name").text.as_deref(), Some("Tram 28.mov"));
    assert_eq!(info(&rt, "item-count").text.as_deref(), Some("Video \u{00B7} 120 MB"));
    assert_eq!(info(&rt, "toolbar").role, "toolbar");
    assert_eq!(info(&rt, "toolbar").label.as_deref(), Some("Tram 28.mov"));
    assert_eq!(info(&rt, "more-btn").label.as_deref(), Some("More actions"));
}

#[test]
fn an_unknown_file_does_not_move_the_route() {
    let mut state = init();
    assert!(!open(&mut state, "no-such-file"));
    assert!(state.viewer.is_none());
    assert!(matches!(state.shell.app.screen(), Screen::Drive(route) if route.item.is_none()));
    assert_eq!(scene_file("DW-9999"), None);
}

#[test]
fn decrypting_shows_the_amount_and_cancel_stops_it_from_the_keyboard() {
    let (mut rt, calls) = recording(board::DECRYPTING);
    assert_eq!(info(&rt, "state-sub-mono").text.as_deref(), Some("46 of 120 MB"));
    assert_eq!(info(&rt, "state-bar").role, "progressbar");
    assert_eq!(info(&rt, "state-bar").label.as_deref(), Some("Decrypting, 38 percent"));
    assert_eq!(
        info(&rt, "status-text").text.as_deref(),
        Some("Decrypting \u{00B7} 46 of 120 MB")
    );
    assert_eq!(info(&rt, "state-btn-0").role, "button");
    assert_eq!(info(&rt, "state-btn-0").label.as_deref(), Some("Cancel"));
    operate(&mut rt, "state-btn-0");
    assert_eq!(*calls.borrow(), ["open tram-28.mov", "cancel tram-28.mov"]);
    assert!(rt.state.viewer.is_none(), "nothing of the file is kept");
    assert!(!on_a_file(&rt), "back in the folder");
    assert!(has(&rt, "file-list"), "the folder's list is drawn again");
}

#[test]
fn escape_leaves_the_file_and_the_next_key_drops_what_was_decrypted() {
    let (mut rt, calls) = recording(board::DECRYPTING);
    // The shell's Esc runs first and leaves the route; this group then cancels and forgets.
    press(&mut rt, Key::Escape);
    assert!(!on_a_file(&rt));
    assert!(!has(&rt, "state-title"), "a file the route left is not drawn");
    super::sync(&mut rt.state);
    assert!(rt.state.viewer.is_none());
    assert_eq!(calls.borrow().last().map(String::as_str), Some("cancel tram-28.mov"));
}

#[test]
fn a_file_the_route_does_not_name_is_never_drawn() {
    let mut state = init();
    open(&mut state, board::CODE);
    // The route moves to another item without this group being asked.
    crate::shell::host::dispatch(&mut state, Event::Escape);
    crate::shell::host::dispatch(&mut state, Event::OpenItem("something-else".into()));
    let rt = runtime_of(state, false);
    assert!(
        !has(&rt, "run-0-0"),
        "the Python file's text is not under another file's route"
    );
}

#[test]
fn too_large_says_the_numbers_and_both_ways_out_are_intents() {
    let mut rt = runtime("DW-1517");
    assert_eq!(
        info(&rt, "state-sub").text.as_deref(),
        Some(
            "Studio walkthrough 4K.mov is 18.2 GB. Opening it decrypts it on this PC, which needs that much free space; there's 12.4 GB."
        )
    );
    assert_eq!(
        info(&rt, "status-details").text.as_deref(),
        Some("18.2 GB needed \u{00B7} 12.4 GB free on this PC")
    );
    operate(&mut rt, "state-btn-0");
    operate(&mut rt, "state-btn-1");
    assert_eq!(intents(&mut rt), [Intent::FreeUpSpace, Intent::DownloadTo]);
    // The board's rule: no "Open in another app" here (it would fail the same way).
    assert!(!has(&rt, "tb-btn-0"));
}

#[test]
fn a_failure_is_an_alert_and_try_again_decrypts_again() {
    let (mut rt, calls) = recording(board::FAILED);
    assert_eq!(info(&rt, "state").role, "alert");
    assert_eq!(info(&rt, "state-title").role, "heading");
    operate(&mut rt, "state-btn-1");
    assert_eq!(intents(&mut rt), [Intent::ReportProblem]);
    operate(&mut rt, "state-btn-0");
    assert_eq!(
        calls.borrow().last().map(String::as_str),
        Some("retry invoice-0139.pdf")
    );
    assert_eq!(
        info(&rt, "state-title").text.as_deref(),
        Some("Decrypting on this PC"),
        "DW-1516"
    );
}

#[test]
fn code_has_wrap_copy_and_edit_and_each_works_from_the_keyboard() {
    let mut rt = runtime("DW-1512");
    assert_eq!(info(&rt, "seg").role, "tablist");
    assert_eq!(info(&rt, "seg-0").label.as_deref(), Some("Wrap"));
    assert_eq!(info(&rt, "tool-0").label.as_deref(), Some("Copy"));
    assert_eq!(info(&rt, "tb-btn-0").label.as_deref(), Some("Edit"));
    assert_eq!(info(&rt, "viewer-body").role, "document");
    // 18 lines, numbered from 1.
    assert_eq!(info(&rt, "ln-17").text.as_deref(), Some("18"));
    assert!(!has(&rt, "ln-18"));

    assert!(!rt.state.viewer.as_ref().unwrap().wrap);
    operate(&mut rt, "seg-0");
    assert!(rt.state.viewer.as_ref().unwrap().wrap, "Wrap is chosen");
    operate(&mut rt, "seg-1");
    assert!(!rt.state.viewer.as_ref().unwrap().wrap, "No wrap is chosen");

    operate(&mut rt, "tb-btn-0");
    assert_eq!(intents(&mut rt), [Intent::Edit]);
    press_with(
        &mut rt,
        Key::Command('e'),
        Mods {
            cmd: true,
            ..Mods::default()
        },
    );
    assert_eq!(intents(&mut rt), [Intent::Edit], "Ctrl+E");
}

#[test]
fn a_text_over_the_limit_is_read_only_with_one_way_out() {
    let mut rt = runtime("DW-1527");
    assert_eq!(info(&rt, "banner").role, "status");
    assert_eq!(
        info(&rt, "status-details").text.as_deref(),
        Some("Showing the first 500 of 41,206 lines")
    );
    // No Edit, no Wrap, no Copy.
    for absent in ["seg", "tool-0"] {
        assert!(!has(&rt, absent), "{absent} is not drawn on a read-only text");
    }
    assert_eq!(
        info(&rt, "tb-btn-0").label.as_deref(),
        Some("Open in another app\u{2026}")
    );
    operate(&mut rt, "tb-btn-0");
    assert_eq!(intents(&mut rt), [Intent::OpenInAnotherApp]);
    press_with(
        &mut rt,
        Key::Command('e'),
        Mods {
            cmd: true,
            ..Mods::default()
        },
    );
    assert_eq!(intents(&mut rt), [], "Ctrl+E does nothing on a read-only text");
}

#[test]
fn alt_shift_p_and_the_button_toggle_the_details_panel() {
    let mut rt = runtime("DW-1512");
    assert_eq!(info(&rt, "panel-toggle-btn").label.as_deref(), Some("Details panel"));
    press_with(
        &mut rt,
        Key::Text("P".into()),
        Mods {
            alt: true,
            shift: true,
            ..Mods::default()
        },
    );
    assert!(rt.state.viewer.as_ref().unwrap().show_details);
    operate(&mut rt, "panel-toggle-btn");
    assert!(!rt.state.viewer.as_ref().unwrap().show_details);
}

#[test]
fn the_folder_controls_that_do_nothing_for_a_file_cannot_be_operated() {
    let mut rt = runtime("DW-1516");
    let mut stops = Vec::new();
    for _ in 0..30 {
        press(&mut rt, Key::Tab);
        if let Some(key) = focused(&rt) {
            stops.push(key);
        }
    }
    for dead in ["view-list", "view-grid", "sort-btn", "forward"] {
        assert!(!stops.iter().any(|k| k == dead), "{dead} takes no focus: {stops:?}");
    }
    for live in ["back", "more-btn", "panel-toggle-btn", "new-btn", "state-btn-0"] {
        assert!(stops.iter().any(|k| k == live), "{live} is reached by Tab: {stops:?}");
    }
}

#[test]
fn back_and_new_leave_through_the_shell() {
    let mut rt = runtime("DW-1512");
    operate(&mut rt, "new-btn");
    assert_eq!(intents(&mut rt), [Intent::NewMenu]);
    operate(&mut rt, "back");
    assert!(!on_a_file(&rt));
    assert!(
        rt.state.viewer.is_none(),
        "the decrypted text left memory with the file"
    );
}

#[test]
fn copy_takes_the_text_as_it_is_in_the_file() {
    let mut source = FixtureViewerSource::new();
    let Body::Text(doc) = source.open(board::CODE).unwrap().body else {
        panic!("the Python file is text")
    };
    let text = doc.plain();
    assert!(text.starts_with(
        "# Export this month's invoices as one CSV\nimport csv\nfrom pathlib import Path\n\ndef export_invoices("
    ));
    assert_eq!(text.lines().count(), 18);
    assert_eq!(plain_lines("a\n\nb").len(), 3);
}

#[test]
fn amounts_read_as_the_board_writes_them() {
    assert_eq!(size_words(18_200_000_000), "18.2 GB");
    assert_eq!(size_words(120_000_000), "120 MB");
    assert_eq!(size_words(220_000), "220 KB");
    assert_eq!(progress_words(46_000_000, 120_000_000), "46 of 120 MB");
    assert_eq!(progress_words(1_500_000_000, 18_200_000_000), "1.5 of 18.2 GB");
    assert_eq!(percent(46, 120), 38);
    assert_eq!(percent(5, 0), 0);
    assert_eq!(percent(200, 100), 100);
    assert_eq!(count_words(41_206), "41,206");
    assert_eq!(count_words(500), "500");
    assert_eq!(count_words(1_000_000), "1,000,000");
}

#[test]
fn no_file_content_is_in_an_accessibility_label() {
    // What is on screen is text; nothing copies a file's content into a name a tool reads twice.
    let rt = runtime("DW-1512");
    for e in rt.elements() {
        if let Some(label) = &e.label {
            assert!(
                !label.contains("export_invoices("),
                "{:?} carries code in its label",
                e.key
            );
            assert!(
                !label.contains("Exported {count}"),
                "{:?} carries code in its label",
                e.key
            );
        }
    }
}

#[test]
fn progress_from_the_engine_moves_the_bar_and_a_finished_file_replaces_the_state() {
    struct Stepping(Vec<Body>);
    impl ViewerSource for Stepping {
        fn open(&mut self, id: &str) -> Result<OpenFile, OpenError> {
            FixtureViewerSource::new().open(id)
        }
        fn poll(&mut self, _: &str) -> Option<Body> {
            self.0.pop()
        }
        fn cancel(&mut self, _: &str) {}
        fn retry(&mut self, id: &str) -> Result<OpenFile, OpenError> {
            self.open(id)
        }
    }
    let mut state = init();
    let steps = vec![
        Body::Failed,
        Body::Decrypting {
            done: 90_000_000,
            total: 120_000_000,
        },
    ];
    assert!(open_from(&mut state, Box::new(Stepping(steps)), board::DECRYPTING));
    let mut rt = runtime_of(state, false);
    assert!(rt.state.viewer.as_mut().unwrap().pump());
    rt.invalidate();
    rt.frame(0.0);
    assert_eq!(info(&rt, "state-sub-mono").text.as_deref(), Some("90 of 120 MB"));
    assert_eq!(info(&rt, "state-bar").label.as_deref(), Some("Decrypting, 75 percent"));
    assert!(rt.state.viewer.as_mut().unwrap().pump());
    rt.invalidate();
    rt.frame(0.0);
    assert_eq!(
        info(&rt, "state-title").text.as_deref(),
        Some("Couldn't decrypt this file")
    );
    assert!(!rt.state.viewer.as_mut().unwrap().pump(), "nothing new");
}

#[test]
fn wrap_breaks_a_long_lines_text_at_the_windows_width_and_no_wrap_keeps_it_on_one() {
    /// The Python file with its first line made 400 characters long.
    struct LongLine;
    impl ViewerSource for LongLine {
        fn open(&mut self, id: &str) -> Result<OpenFile, OpenError> {
            let mut file = FixtureViewerSource::new().open(id)?;
            if let Body::Text(doc) = &mut file.body {
                doc.lines[0] = plain_lines(&"word ".repeat(80)).remove(0);
            }
            Ok(file)
        }
        fn poll(&mut self, _: &str) -> Option<Body> {
            None
        }
        fn cancel(&mut self, _: &str) {}
        fn retry(&mut self, id: &str) -> Result<OpenFile, OpenError> {
            self.open(id)
        }
    }
    let mut state = init();
    assert!(open_from(&mut state, Box::new(LongLine), board::CODE));
    let mut rt = runtime_of(state, false);
    let window_right = 1278.0;
    let text = info(&rt, "run-0-0").rect;
    assert!(text[0] + text[2] > window_right, "No wrap: the line runs past the edge");
    assert_eq!(text[3], 21.0, "No wrap: one row");
    operate(&mut rt, "seg-0");
    let text = info(&rt, "line-0-text").rect;
    assert!(
        text[0] + text[2] <= window_right,
        "Wrap: the text stays inside the window ({text:?})"
    );
    assert!(text[3] >= 42.0, "Wrap: the text takes more rows ({text:?})");
    assert_eq!(info(&rt, "line-1-text").rect[3], 21.0, "a short line stays one row");
    assert_eq!(info(&rt, "run-1-w0").text.as_deref(), Some("import"));
    // The row is as high as its words' rows, so the next line starts under it.
    let (row, next) = (info(&rt, "line-0").rect, info(&rt, "line-1").rect);
    assert_eq!(row[3], text[3], "the row is as high as its text");
    assert_eq!(next[1], row[1] + row[3], "the next line starts under the wrapped one");
    operate(&mut rt, "seg-1");
    assert_eq!(info(&rt, "run-0-0").rect[3], 21.0, "No wrap again");
}
