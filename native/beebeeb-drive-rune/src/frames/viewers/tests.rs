//! Tests of the open-file group: every built frame's words, the keyboard, names and roles, what
//! each control asks of the source or the shell, and that decrypted content leaves memory when
//! the file is left.

use std::cell::RefCell;
use std::rc::Rc;

use rune::{Env, Key, KeyEvent, Mods, Runtime};

use super::source::{
    Body, FixtureViewerSource, Ink, Keep, OpenError, OpenFile, Run, SaveOutcome, ViewerSource, board, count_words,
    percent, plain_lines, progress_words, size_words,
};
use super::{Dialog, Intent, markdown, md_source, open, open_from, scene_file, set_scene};
use crate::shell::app::{Event, Screen};
use crate::{State, init, view};

/// Every built frame, and its first words.
const BUILT: [(&str, &str, &str); 11] = [
    ("DW-1513", "md-h1", "Meeting notes, 4 Nov"),
    (
        "DW-1519",
        "banner-text",
        "A newer version arrived from Anna's iPhone, 09:14.",
    ),
    ("DW-1523", "run-12-1", "Ask Priya about the invoices"),
    ("DW-1524", "toast-text", "Saved \u{00B7} encrypted \u{00B7} version 4"),
    ("DW-1525", "dlg-title", "You have unsaved changes"),
    ("DW-1526", "dlg-title", "Two versions"),
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
    fn set_offline(&mut self, id: &str, on: bool) -> bool {
        self.calls.borrow_mut().push(format!("offline {id} {on}"));
        self.inner.set_offline(id, on)
    }
    fn save(&mut self, id: &str, text: &str, base: u32) -> SaveOutcome {
        self.calls
            .borrow_mut()
            .push(format!("save {id} from {base}, {} lines", text.lines().count()));
        self.inner.save(id, text, base)
    }
    fn resolve(&mut self, id: &str, text: &str, keep: Keep) -> SaveOutcome {
        self.calls.borrow_mut().push(format!("resolve {id} {keep:?}"));
        self.inner.resolve(id, text, keep)
    }
    fn newer(&mut self, id: &str) -> Result<OpenFile, OpenError> {
        self.calls.borrow_mut().push(format!("newer {id}"));
        self.inner.newer(id)
    }
    fn colours(&self, file: &OpenFile, text: &str) -> Vec<Vec<Run>> {
        self.inner.colours(file, text)
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
    rt.poll = Some(crate::shell::host::poll);
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
    assert!(
        rt.state.viewer.as_ref().unwrap().editor.is_some(),
        "Edit opens the editor"
    );
    assert_eq!(info(&rt, "tb-btn-0").label.as_deref(), Some("Save"));
    // Nothing was typed: Save leaves the editor and saves no version.
    ctrl(&mut rt, 's');
    assert!(rt.state.viewer.as_ref().unwrap().editor.is_none());
    assert_eq!(rt.state.viewer.as_ref().unwrap().file.version, 1, "no new version");
    ctrl(&mut rt, 'e');
    assert!(rt.state.viewer.as_ref().unwrap().editor.is_some(), "Ctrl+E");
    assert_eq!(
        focused(&rt).as_deref(),
        Some(super::EDITOR_KEY),
        "the keyboard is the editor's"
    );
}

fn ctrl(rt: &mut Runtime<State>, key: char) {
    press_with(
        rt,
        Key::Command(key),
        Mods {
            cmd: true,
            ..Mods::default()
        },
    );
}

fn viewer(rt: &Runtime<State>) -> &super::Ui {
    rt.state.viewer.as_ref().expect("a file is open")
}

/// The note, open on a source that records what is asked of it.
fn note(newer: bool, conflict: bool) -> (Runtime<State>, Calls) {
    let calls = Calls::default();
    let mut state = init();
    let source = Recording {
        inner: FixtureViewerSource {
            newer,
            conflict,
            ..FixtureViewerSource::new()
        },
        calls: calls.clone(),
    };
    assert!(open_from(&mut state, Box::new(source), board::NOTES));
    (runtime_of(state, false), calls)
}

/// Open the editor on the note and type one more line at its end.
fn edit_and_type(rt: &mut Runtime<State>) {
    ctrl(rt, 'e');
    assert_eq!(focused(rt).as_deref(), Some(super::EDITOR_KEY));
    for ch in "- Ask".chars() {
        press(rt, Key::Text(ch.to_string()));
    }
    assert!(
        viewer(rt).dirty(),
        "typing reaches the editor: {:?}",
        viewer(rt).editor.as_ref().map(|e| e.edit.text.clone())
    );
}

#[test]
fn markdown_is_parsed_into_headings_paragraphs_lists_code_and_links() {
    let blocks = markdown::parse(board::NOTES_TEXT);
    assert_eq!(blocks.len(), 6, "{blocks:?}");
    let markdown::Block::List(items) = &blocks[3] else {
        panic!("the fourth block is the first list: {:?}", blocks[3]);
    };
    assert_eq!(items.len(), 3);
    assert!(items[0].iter().any(|p| p.bold && p.text == "Friday"));
    assert!(items[1].iter().any(|p| p.code && p.text == "Shoot selects/graded"));
    // A link whose target has a space in it is a link, as people write a file's name.
    let link = items[2].iter().find(|p| p.link.is_some()).expect("a link");
    assert_eq!(link.text, "Service agreement");
    assert_eq!(link.link.as_deref(), Some("Service agreement.docx"));
    assert!(!markdown::is_web("Service agreement.docx"));
    assert!(markdown::is_web("https://example.org/a"));
    // Raw HTML is shown as the text it is, never run or dropped.
    let html = markdown::parse("before <script>alert(1)</script> after");
    let markdown::Block::Para(pieces) = &html[0] else {
        panic!("{html:?}")
    };
    let shown: String = pieces.iter().map(|p| p.text.as_str()).collect();
    assert_eq!(shown, "before <script>alert(1)</script> after");
}

#[test]
fn a_notes_source_dims_its_marks_and_keeps_every_character() {
    for line in board::NOTES_TEXT
        .lines()
        .chain(["plain", "**bold** and `code`", "# ", "[a](b) [c"])
    {
        let runs = md_source::line(line);
        let joined: String = runs.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(joined, line, "no character is lost or added");
    }
    let runs = md_source::line("- Final selects due **Friday**");
    assert_eq!(
        (runs[0].text.as_str(), runs[0].ink, runs[0].italic),
        ("- ", Ink::Faint, true)
    );
    assert_eq!(
        (runs[3].text.as_str(), runs[3].ink, runs[3].bold),
        ("Friday", Ink::Strong, true)
    );
}

#[test]
fn a_note_is_read_rendered_or_as_source_and_its_links_and_panel_work_from_the_keyboard() {
    let (mut rt, calls) = note(false, false);
    assert_eq!(info(&rt, "seg-0").label.as_deref(), Some("Rendered"));
    assert_eq!(info(&rt, "md-h1").role, "heading");
    assert_eq!(info(&rt, "md-li-0").role, "listitem");
    // The link is a link, reached by Tab, and asks the shell to open the file in Drive.
    let link = rt
        .elements()
        .into_iter()
        .find(|e| e.role == "link" && e.label.as_deref() == Some("Service agreement"))
        .expect("the link");
    let key = link.key.clone().expect("the link has a key");
    operate(&mut rt, &key);
    assert_eq!(intents(&mut rt), [Intent::OpenInDrive("Service agreement.docx".into())]);
    // Source shows the text; Rendered brings the note back.
    operate(&mut rt, "seg-1");
    assert!(viewer(&rt).md_source);
    assert_eq!(info(&rt, "ln-11").text.as_deref(), Some("12"));
    assert!(!has(&rt, "md-h1"));
    operate(&mut rt, "seg-0");
    assert!(has(&rt, "md-h1"));
    // The details panel: Alt+Shift+P, then the switch and the two actions.
    assert!(!has(&rt, "detail-pane"));
    press_with(
        &mut rt,
        Key::Text("P".into()),
        Mods {
            alt: true,
            shift: true,
            ..Mods::default()
        },
    );
    assert_eq!(info(&rt, "detail-pane").role, "complementary");
    assert_eq!(info(&rt, "detail-name").text.as_deref(), Some("Meeting notes 4 Nov.md"));
    assert_eq!(
        info(&rt, "detail-note").text.as_deref(),
        Some(super::details::DECRYPTED_HERE)
    );
    assert_eq!(info(&rt, "offline-card").role, "switch");
    operate(&mut rt, "offline-card");
    assert_eq!(
        calls.borrow().last().map(String::as_str),
        Some("offline meeting-notes-4-nov.md false")
    );
    assert!(!viewer(&rt).file.details.offline);
    operate(&mut rt, "share-box");
    operate(&mut rt, "versions-box");
    assert_eq!(intents(&mut rt), [Intent::Share, Intent::VersionHistory]);
}

#[test]
fn a_route_to_a_file_opens_it_and_an_unknown_file_leaves_the_placeholder() {
    // What Files does for Enter on a file: the shell's event, nothing of this group's.
    let mut state = init();
    crate::shell::host::dispatch(&mut state, Event::OpenItem(board::NOTES.to_string()));
    let rt = runtime_of(state, false);
    assert_eq!(viewer(&rt).file.name, "Meeting notes 4 Nov.md");
    assert!(has(&rt, "md-h1"));
    let mut state = init();
    crate::shell::host::dispatch(&mut state, Event::OpenItem("no-such-file".to_string()));
    let rt = runtime_of(state, false);
    assert!(rt.state.viewer.is_none());
    assert!(!has(&rt, "viewer-body"));
}

#[test]
fn a_newer_version_waits_for_show_it() {
    let (mut rt, calls) = note(true, false);
    assert_eq!(info(&rt, "banner").role, "status");
    assert_eq!(viewer(&rt).file.version, 3, "the version on screen stays");
    assert!(!calls.borrow().iter().any(|c| c.starts_with("newer")));
    operate(&mut rt, "banner-btn");
    assert_eq!(viewer(&rt).file.version, 4);
    assert!(!has(&rt, "banner"), "the note is gone with the version it was about");
    assert_eq!(
        calls.borrow().last().map(String::as_str),
        Some("newer meeting-notes-4-nov.md")
    );
}

#[test]
fn typing_marks_the_edit_unsaved_and_ctrl_s_saves_a_new_version() {
    let (mut rt, calls) = note(false, false);
    edit_and_type(&mut rt);
    assert_eq!(info(&rt, "crumb-dot").label.as_deref(), Some("Not saved"));
    let unsaved = "Changes not saved yet \u{00B7} version 3";
    assert_eq!(info(&rt, "item-count").text.as_deref(), Some(unsaved));
    assert_eq!(info(&rt, "status-text").text.as_deref(), Some(unsaved));
    assert_eq!(
        info(&rt, "status-details").text.as_deref(),
        Some("Markdown \u{00B7} UTF-8 \u{00B7} line 13")
    );
    assert_eq!(info(&rt, "tb-btn-0").label.as_deref(), Some("Save"));
    assert!(!has(&rt, "seg"), "the editor has no Rendered / Source");
    ctrl(&mut rt, 's');
    assert_eq!(
        calls.borrow().last().map(String::as_str),
        Some("save meeting-notes-4-nov.md from 3, 13 lines")
    );
    let ui = viewer(&rt);
    assert!(ui.editor.is_none(), "saving leaves the editor");
    assert_eq!(ui.file.version, 4);
    assert_eq!(info(&rt, "toast").role, "status");
    assert_eq!(
        info(&rt, "status-text").text.as_deref(),
        Some("Saved \u{00B7} encrypted \u{00B7} version 4 \u{00B7} 09:41")
    );
    assert_eq!(
        info(&rt, "md-li-5").label.as_deref(),
        Some("Ask"),
        "the new line is in the note"
    );
    assert!(!has(&rt, "crumb-dot"));
}

#[test]
fn leaving_with_changes_asks_first_and_each_answer_does_what_it_says() {
    // The shell leaves the file (its Back, a section of the sidebar) with unsaved changes: the
    // route comes back to the file before the next frame, and the question is up. (In the
    // editor Esc and Alt+Left are the text field's own keys.)
    let (mut rt, calls) = note(false, false);
    edit_and_type(&mut rt);
    crate::shell::host::dispatch(&mut rt.state, Event::Escape);
    assert!(!on_a_file(&rt), "the shell left the file");
    rt.invalidate();
    rt.frame(0.0);
    assert!(on_a_file(&rt), "the route is still on the file");
    assert_eq!(viewer(&rt).dialog, Some(Dialog::Unsaved));
    assert_eq!(info(&rt, "dlg").role, "alertdialog");
    assert_eq!(info(&rt, "dlg-btn-1").label.as_deref(), Some("Discard changes"));
    // Keep editing: back to the edit, still not saved.
    operate(&mut rt, "dlg-btn-2");
    assert_eq!(viewer(&rt).dialog, None);
    assert!(viewer(&rt).dirty());
    // The toolbar's Back asks too; Discard changes drops the edit and keeps version 3.
    operate(&mut rt, "back");
    assert_eq!(viewer(&rt).dialog, Some(Dialog::Unsaved));
    operate(&mut rt, "dlg-btn-1");
    let ui = viewer(&rt);
    assert!(ui.editor.is_none() && ui.dialog.is_none());
    assert_eq!(ui.file.version, 3);
    assert!(
        !calls.borrow().iter().any(|c| c.starts_with("save")),
        "nothing was saved"
    );
    // Save from the question saves.
    edit_and_type(&mut rt);
    operate(&mut rt, "back");
    operate(&mut rt, "dlg-btn-0");
    assert_eq!(viewer(&rt).file.version, 4);
    assert!(viewer(&rt).dialog.is_none());
    // With nothing unsaved, Back leaves the file and its content leaves memory.
    operate(&mut rt, "back");
    assert!(rt.state.viewer.is_none());
    assert!(!on_a_file(&rt));
}

#[test]
fn a_save_that_meets_another_devices_version_overwrites_nothing() {
    for (button, keep, version, editing) in [
        ("dlg-btn-0", Some(Keep::Both), 1, false),
        ("dlg-btn-1", Some(Keep::Mine), 5, false),
        ("dlg-btn-2", Some(Keep::Theirs), 4, false),
        ("dlg-btn-3", None, 3, true),
    ] {
        let (mut rt, calls) = note(false, true);
        edit_and_type(&mut rt);
        ctrl(&mut rt, 's');
        let Some(Dialog::Conflict(other)) = viewer(&rt).dialog.clone() else {
            panic!("the two-versions question is up");
        };
        assert_eq!(
            (other.from.as_str(), other.at.as_str(), other.version),
            ("Anna's iPhone", "09:14", 4)
        );
        assert_eq!(info(&rt, "dlg").role, "dialog");
        assert_eq!(
            info(&rt, "dlg-card-0-sub").text.as_deref(),
            Some("this PC \u{00B7} not saved \u{00B7} +1 line")
        );
        assert_eq!(info(&rt, "dlg-btn-2").label.as_deref(), Some("Keep Anna's iPhone's"));
        assert_eq!(
            viewer(&rt).file.version,
            3,
            "nothing is written while the question is up"
        );
        operate(&mut rt, button);
        let ui = viewer(&rt);
        assert_eq!(ui.dialog, None, "{button}");
        assert_eq!(ui.editor.is_some(), editing, "{button}");
        assert_eq!(ui.file.version, version, "{button}");
        match keep {
            Some(keep) => assert_eq!(
                calls.borrow().last().cloned(),
                Some(format!("resolve meeting-notes-4-nov.md {keep:?}"))
            ),
            None => assert!(
                !calls.borrow().iter().any(|c| c.starts_with("resolve")),
                "Cancel settles nothing"
            ),
        }
        if keep == Some(Keep::Both) {
            assert_eq!(ui.file.name, "Meeting notes 4 Nov (Anna-Laptop).md");
        }
    }
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
    // The shell asks the open file for progress before every frame (`host::poll`).
    let mut rt = runtime_of(state, false);
    assert_eq!(info(&rt, "state-sub-mono").text.as_deref(), Some("90 of 120 MB"));
    assert_eq!(info(&rt, "state-bar").label.as_deref(), Some("Decrypting, 75 percent"));
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
    // Rune moves a keyed element to its new place over a few frames: read it at rest.
    rt.settle_all();
    rt.frame(0.0);
    let (row, last) = (info(&rt, "line-0").rect, info(&rt, "run-0-w79").rect);
    assert!(
        last[0] + last[2] <= window_right,
        "Wrap: the last word is inside the window ({last:?})"
    );
    assert!(row[3] >= 42.0, "Wrap: the line takes more rows ({row:?})");
    assert_eq!(
        last[1] + last[3],
        row[1] + row[3],
        "the row ends with its last word's row"
    );
    assert_eq!(info(&rt, "ln-0").rect[0], row[0], "the number stays in the gutter");
    assert_eq!(
        info(&rt, "run-0-w0").rect[0],
        row[0] + 56.0,
        "the text starts after the gutter"
    );
    let next = info(&rt, "line-1").rect;
    assert_eq!(next[3], 21.0, "a short line stays one row");
    assert_eq!(next[1], row[1] + row[3], "the next line starts under the wrapped one");
    assert_eq!(info(&rt, "run-1-w0").text.as_deref(), Some("import"));
    operate(&mut rt, "seg-1");
    assert_eq!(info(&rt, "run-0-0").rect[3], 21.0, "No wrap again");
}
