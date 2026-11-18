//! Tests of the office editor's frame (DW-1507, DW-1511, DW-1550 to DW-1554, DW-1556): every
//! frame's words, that a save, a conflict and a search are drawn from the source's answer and
//! never before it, the keys, and that nothing of a document reaches a `Debug`.

use std::cell::RefCell;
use std::rc::Rc;

use rune::{Env, Key, KeyEvent, Mods, Runtime};

use super::office::board::{Case, QUERY};
use super::office::{Kept, Tab};
use super::office_over::{CommandGroup, NOT_SAVED, Over, SAVED_FIRST};
use super::source::{Body, FixtureViewerSource, Keep, OpenError, OpenFile, SaveOutcome, ViewerSource, board};
use super::{open_from, set_scene};
use crate::shell::app::Screen;
use crate::{State, init, view};

type Calls = Rc<RefCell<Vec<String>>>;

/// The fixture, recording what the editor's frame asked. `editor: false` is a source with no
/// editor behind it: it opens the file and answers none of the editor's questions (the trait's
/// defaults).
struct Recording {
    inner: FixtureViewerSource,
    calls: Calls,
    editor: bool,
}

impl ViewerSource for Recording {
    fn open(&mut self, id: &str) -> Result<OpenFile, OpenError> {
        self.inner.open(id)
    }
    fn poll(&mut self, id: &str) -> Option<Body> {
        self.inner.poll(id)
    }
    fn cancel(&mut self, _: &str) {}
    fn retry(&mut self, id: &str) -> Result<OpenFile, OpenError> {
        self.calls.borrow_mut().push(format!("retry {id}"));
        self.inner.retry(id)
    }
    fn save_document(&mut self, id: &str, base: u32) -> SaveOutcome {
        self.calls.borrow_mut().push(format!("save {id} from {base}"));
        if self.editor {
            self.inner.save_document(id, base)
        } else {
            SaveOutcome::Failed
        }
    }
    fn resolve_document(&mut self, id: &str, keep: Keep) -> SaveOutcome {
        self.calls.borrow_mut().push(format!("resolve {id} {keep:?}"));
        self.inner.resolve_document(id, keep)
    }
    fn commands(&self, id: &str, query: &str) -> Vec<CommandGroup> {
        if self.editor {
            self.inner.commands(id, query)
        } else {
            Vec::new()
        }
    }
}

fn runtime_of(state: State, dark: bool) -> Runtime<State> {
    let env = Env {
        system_dark: dark,
        scale: 2.0,
    };
    let mut rt = Runtime::new(state, view, (1278.0, 798.0), env);
    rt.on_key = Some(crate::shell::host::on_key);
    rt.poll = Some(crate::shell::host::poll);
    rt.frame(0.0);
    rt
}

fn scene(frame: &str, dark: bool) -> Runtime<State> {
    let mut state = init();
    set_scene(&mut state, frame);
    runtime_of(state, dark)
}

/// The agreement, open on a recording source.
fn agreement(case: Case, conflict: bool, editor: bool) -> (Runtime<State>, Calls) {
    let calls = Calls::default();
    let mut state = init();
    let source = Recording {
        inner: FixtureViewerSource {
            conflict,
            office: case,
            ..FixtureViewerSource::new()
        },
        calls: calls.clone(),
        editor,
    };
    assert!(open_from(&mut state, Box::new(source), board::DOCX));
    (runtime_of(state, false), calls)
}

fn find(rt: &Runtime<State>, key: &str) -> Option<rune::ElementInfo> {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
}

fn has(rt: &Runtime<State>, key: &str) -> bool {
    find(rt, key).is_some()
}

fn words(rt: &Runtime<State>, key: &str) -> String {
    find(rt, key)
        .unwrap_or_else(|| panic!("no element keyed {key}"))
        .text
        .unwrap_or_default()
}

fn press_with(rt: &mut Runtime<State>, key: Key, mods: Mods) {
    rt.key(KeyEvent { key, mods });
    rt.frame(0.0);
}

fn press(rt: &mut Runtime<State>, key: Key) {
    press_with(rt, key, Mods::default());
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

fn focused(rt: &Runtime<State>) -> Option<String> {
    rt.elements().into_iter().find(|e| e.focused).and_then(|e| e.key)
}

/// Every key Tab stops on, once around (at most 80 stops).
fn tab_stops(rt: &mut Runtime<State>) -> Vec<String> {
    let mut stops: Vec<String> = Vec::new();
    for _ in 0..80 {
        press(rt, Key::Tab);
        match focused(rt) {
            Some(key) if stops.contains(&key) => break,
            Some(key) => stops.push(key),
            None => {}
        }
    }
    stops
}

/// Tab to a control and press it with Space.
fn operate(rt: &mut Runtime<State>, key: &str) {
    for _ in 0..80 {
        if focused(rt).as_deref() == Some(key) {
            press(rt, Key::Text(" ".into()));
            return;
        }
        press(rt, Key::Tab);
    }
    panic!("Tab never reached {key}; focus is on {:?}", focused(rt));
}

fn ui(rt: &Runtime<State>) -> &super::Ui {
    rt.state.viewer.ui.as_ref().expect("a file is open")
}

fn on_a_file(rt: &Runtime<State>) -> bool {
    matches!(rt.state.shell.app.screen(), Screen::Drive(route) if route.item.is_some())
}

fn version(rt: &Runtime<State>) -> u32 {
    ui(rt).file.version
}

const SAVED_7: &str = "saved as version 7 \u{00B7} 09:12";

#[test]
fn every_frame_of_the_editor_draws_its_words_in_both_themes() {
    let frames: &[(&str, &[(&str, &str)])] = &[
        ("DW-1507e", &[("st-7", SAVED_7), ("pg-0", "Service agreement")]),
        (
            "DW-1511",
            &[("banner-text", super::office::OLDER_FORMAT), ("st-0", "Page 1 of 1")],
        ),
        (
            "DW-1550",
            &[
                ("rt-3-label", "Page break"),
                ("tbl-1-1-t", "Alfama"),
                ("st-7", "unsaved changes"),
            ],
        ),
        (
            "DW-1551",
            &[("pal-g0-t", "STYLES"), ("pal-0", "Heading 1"), ("pal-3-kbd", "Ctrl+B")],
        ),
        (
            "DW-1552",
            &[("st-7", "unsaved changes"), ("crumb-name", "Service agreement.docx")],
        ),
        (
            "DW-1553",
            &[
                ("st-7", "saved as version 8 \u{00B7} 09:41"),
                ("toast-text", "Saved \u{00B7} encrypted \u{00B7} version 8"),
            ],
        ),
        (
            "DW-1554",
            &[("dlg-title", SAVED_FIRST), ("st-7", "not saved \u{00B7} conflict")],
        ),
        (
            "DW-1556",
            &[
                ("empty-title", super::office_over::NO_EDITOR_TITLE),
                ("st-2", "read-only for now"),
            ],
        ),
    ];
    for (frame, expected) in frames {
        for dark in [false, true] {
            let rt = scene(frame, dark);
            for (key, text) in *expected {
                assert_eq!(words(&rt, key), *text, "{frame} {key} (dark: {dark})");
            }
            for chrome in ["caption-bar", "toolbar", "rib", "status-bar"] {
                assert!(has(&rt, chrome), "{frame} has {chrome}");
            }
        }
    }
}

#[test]
fn the_unsaved_dot_shows_only_while_there_are_changes_no_version_holds() {
    for (frame, dot) in [
        ("DW-1507e", false),
        ("DW-1511", false),
        ("DW-1552", true),
        ("DW-1553", false),
        ("DW-1554", true),
        ("DW-1556", false),
    ] {
        assert_eq!(has(&scene(frame, false), "crumb-dot"), dot, "{frame}");
    }
}

#[test]
fn save_asks_nothing_when_nothing_changed() {
    let (mut rt, calls) = agreement(Case::Selected, false, true);
    ctrl(&mut rt, 's');
    assert!(calls.borrow().is_empty(), "{:?}", calls.borrow());
    assert_eq!(words(&rt, "st-7"), SAVED_7);
    assert!(!has(&rt, "toast"));
}

#[test]
fn ctrl_s_saves_and_the_frame_shows_what_the_source_answered() {
    let (mut rt, calls) = agreement(Case::Unsaved, false, true);
    assert_eq!(words(&rt, "st-7"), "unsaved changes");
    ctrl(&mut rt, 's');
    assert_eq!(*calls.borrow(), [format!("save {} from 7", board::DOCX)]);
    assert_eq!(version(&rt), 8);
    assert_eq!(words(&rt, "st-7"), "saved as version 8 \u{00B7} 09:41");
    assert_eq!(words(&rt, "toast-text"), "Saved \u{00B7} encrypted \u{00B7} version 8");
    assert!(!has(&rt, "crumb-dot"));
    // Saved: a second Ctrl+S has nothing to save.
    ctrl(&mut rt, 's');
    assert_eq!(calls.borrow().len(), 1);
}

#[test]
fn a_save_that_did_not_happen_is_never_drawn_as_saved() {
    let (mut rt, calls) = agreement(Case::Unsaved, false, false);
    ctrl(&mut rt, 's');
    assert_eq!(calls.borrow().len(), 1);
    assert_eq!(version(&rt), 7);
    assert_eq!(words(&rt, "st-7"), "unsaved changes");
    assert!(has(&rt, "crumb-dot"));
    assert_eq!(words(&rt, "toast-text"), NOT_SAVED);
    assert!(matches!(&ui(&rt).file.body, Body::Word(doc) if doc.kept == Kept::Unsaved));
}

#[test]
fn a_newer_version_in_the_way_is_a_question_and_escape_keeps_the_edit() {
    let (mut rt, calls) = agreement(Case::Unsaved, true, true);
    ctrl(&mut rt, 's');
    assert_eq!(words(&rt, "dlg-title"), SAVED_FIRST);
    assert!(words(&rt, "dlg-sub").contains("saved as version 8"));
    assert!(words(&rt, "dlg-sub").contains("you opened version 7"));
    assert_eq!(words(&rt, "btn-2-label"), "Discard my changes and reopen version 8");
    assert_eq!(words(&rt, "st-7"), "not saved \u{00B7} conflict");
    assert_eq!(version(&rt), 7);
    // Ctrl+S under the question asks nothing again.
    ctrl(&mut rt, 's');
    assert_eq!(calls.borrow().len(), 1);
    // Esc: back to the document, on the file, the edit still unsaved.
    press(&mut rt, Key::Escape);
    press(&mut rt, Key::Tab);
    assert!(!has(&rt, "dlg"));
    assert!(on_a_file(&rt), "Esc on the question does not leave the document");
    assert_eq!(words(&rt, "st-7"), "unsaved changes");
    assert!(has(&rt, "crumb-dot"));
    assert_eq!(calls.borrow().len(), 1, "nothing was resolved: {:?}", calls.borrow());
}

#[test]
fn each_answer_to_the_question_is_asked_of_the_source_and_drawn_from_its_answer() {
    for (button, keep, version_after, name) in [
        ("btn-0", "Mine", 9, "Service agreement.docx"),
        ("btn-1", "Both", 1, "Service agreement (Anna-Laptop).docx"),
        ("btn-2", "Theirs", 8, "Service agreement.docx"),
    ] {
        let (mut rt, calls) = agreement(Case::Unsaved, true, true);
        ctrl(&mut rt, 's');
        operate(&mut rt, button);
        assert_eq!(
            calls.borrow().last().map(String::as_str),
            Some(format!("resolve {} {keep}", board::DOCX).as_str()),
            "{button}"
        );
        assert!(!has(&rt, "dlg"), "{button}");
        assert_eq!(version(&rt), version_after, "{button}");
        assert_eq!(words(&rt, "crumb-here"), name, "{button}");
        assert!(!has(&rt, "crumb-dot"), "{button}");
    }
    // Cancel resolves nothing.
    let (mut rt, calls) = agreement(Case::Unsaved, true, true);
    ctrl(&mut rt, 's');
    operate(&mut rt, "btn-3");
    assert!(!has(&rt, "dlg"));
    assert_eq!(calls.borrow().len(), 1);
    assert_eq!(version(&rt), 7);
}

#[test]
fn the_command_search_shows_only_what_the_source_finds() {
    let (mut rt, _) = agreement(Case::Selected, false, true);
    ctrl(&mut rt, 'k');
    assert!(has(&rt, "pal"));
    assert_eq!(
        focused(&rt).as_deref(),
        Some("pal-input"),
        "the search takes the keyboard"
    );
    assert!(!has(&rt, "pal-0"), "nothing typed, nothing found");
    let type_query = |rt: &mut Runtime<State>| {
        rt.update(|s, _| {
            if let Some(Over::Palette(palette)) = s.viewer.ui.as_mut().and_then(|ui| ui.office.over.as_mut()) {
                palette.query.insert(QUERY);
            }
        });
        rt.frame(0.0);
    };
    type_query(&mut rt);
    assert_eq!(words(&rt, "pal-g0-t"), "STYLES");
    assert_eq!(words(&rt, "pal-4"), "Northwind brand guide.pdf");
    assert!(!has(&rt, "pal-5"));
    // The arrows move over all results and wrap.
    let hot = |rt: &Runtime<State>| match &ui(rt).office.over {
        Some(Over::Palette(palette)) => palette.hot,
        _ => panic!("the search is closed"),
    };
    press(&mut rt, Key::Down);
    assert_eq!(hot(&rt), 1);
    press(&mut rt, Key::Up);
    press(&mut rt, Key::Up);
    assert_eq!(hot(&rt), 4);
    // Esc closes the search and stays on the document.
    press(&mut rt, Key::Escape);
    press(&mut rt, Key::Tab);
    assert!(!has(&rt, "pal"));
    assert!(on_a_file(&rt));
    assert!(has(&rt, "page"));

    // No editor behind the source: the same words find nothing, and no sample is shown.
    let (mut rt, _) = agreement(Case::Selected, false, false);
    ctrl(&mut rt, 'k');
    type_query(&mut rt);
    assert!(matches!(&ui(&rt).office.over, Some(Over::Palette(palette)) if palette.query.text == QUERY));
    assert!(!has(&rt, "pal-0"));
    assert!(!has(&rt, "pal-g0-t"));
}

#[test]
fn back_under_the_question_closes_the_question_and_keeps_the_document() {
    let (mut rt, calls) = agreement(Case::Unsaved, true, true);
    ctrl(&mut rt, 's');
    assert!(has(&rt, "dlg"));
    press_with(
        &mut rt,
        Key::Left,
        Mods {
            alt: true,
            ..Mods::default()
        },
    );
    press(&mut rt, Key::Tab);
    assert!(
        on_a_file(&rt),
        "Alt+Left under the question does not leave the document"
    );
    assert!(has(&rt, "page"));
    assert!(!has(&rt, "dlg"));
    assert_eq!(calls.borrow().len(), 1, "nothing was resolved");
}

#[test]
fn enter_with_nothing_found_runs_nothing_and_a_failed_answer_is_said_over_the_question() {
    let (mut rt, _) = agreement(Case::Selected, false, false);
    ctrl(&mut rt, 'k');
    press(&mut rt, Key::Enter);
    assert!(has(&rt, "pal"), "nothing found: Enter leaves the search open");
    assert!(!has(&rt, "toast"));
    // A save in the table case keeps the table.
    let (mut rt, _) = agreement(Case::Table, false, true);
    ctrl(&mut rt, 's');
    assert_eq!(words(&rt, "tbl-1-1-t"), "Alfama");
    assert_eq!(version(&rt), 8);
    // The outline has no way back yet, so its button does not take it away.
    operate(&mut rt, "ol-close");
    assert!(has(&rt, "ol"));
}

#[test]
fn the_editors_keys_belong_to_the_editors_frame_only() {
    let mut state = init();
    set_scene(&mut state, "DW-1513");
    let mut rt = runtime_of(state, false);
    ctrl(&mut rt, 'k');
    assert!(ui(&rt).office.over.is_none());
    assert!(!has(&rt, "pal"));
    // And with no editor (DW-1556) there is no search to open.
    let mut rt = scene("DW-1556", false);
    ctrl(&mut rt, 'k');
    assert!(!has(&rt, "pal"));
}

#[test]
fn with_no_editor_the_ribbon_takes_no_key_and_try_again_asks_the_source() {
    let (mut rt, calls) = agreement(Case::NoEditor, false, true);
    assert!(!has(&rt, "page"));
    assert!(!has(&rt, "ol"));
    let stops = tab_stops(&mut rt);
    assert!(stops.contains(&"empty-add".to_string()), "{stops:?}");
    assert!(
        !stops
            .iter()
            .any(|key| key.starts_with("rib") || key.starts_with("rt-") || key.starts_with("rp-")),
        "{stops:?}"
    );
    // Ctrl+S has nothing to save.
    ctrl(&mut rt, 's');
    assert!(calls.borrow().is_empty());
    operate(&mut rt, "empty-add");
    assert_eq!(*calls.borrow(), [format!("retry {}", board::DOCX)]);
    assert!(has(&rt, "page"), "the editor started: the document shows");
    assert!(!has(&rt, "empty"));
}

#[test]
fn the_tabs_switch_the_tools_and_the_search_opens_from_the_ribbon() {
    let (mut rt, _) = agreement(Case::Selected, false, true);
    assert!(has(&rt, "rp-0"));
    operate(&mut rt, "rib-tab-1-cell");
    assert_eq!(ui(&rt).office.tab, Tab::Insert);
    assert_eq!(words(&rt, "rt-0-label"), "Table\u{2026}");
    assert!(!has(&rt, "rp-0"));
    operate(&mut rt, "rib-find");
    assert!(has(&rt, "pal"));
}

#[test]
fn nothing_of_a_document_reaches_a_debug_and_wipe_clears_its_cells() {
    let mut doc = super::office::board::service_agreement_as(Case::Table);
    let shown = format!("{doc:?} {:?}", Body::Word(doc.clone()));
    for word in ["Service", "Northwind", "Alfama", "Scope"] {
        assert!(!shown.contains(word), "{shown}");
    }
    let found = super::office::board::commands(QUERY);
    let shown = format!("{found:?}");
    assert!(!shown.contains("Northwind") && !shown.contains("Heading"), "{shown}");
    doc.wipe();
    assert!(doc.title.is_empty());
    assert!(
        doc.sections
            .iter()
            .all(|s| s.text.is_empty() && s.table.iter().flatten().flatten().all(String::is_empty))
    );
}

// ---------------------------------------------------------------------------
// The live word ribbon (`frames/office/edit/word_ribbon.rs`)
// ---------------------------------------------------------------------------

mod live_ribbon {
    use beebeeb_office_protocol::office::{
        Dirty, EditKind, EditOp, Edited, ListKind, Mark, Marks, PageRect, ParaStyle, RectPt, TextAlign,
    };

    use super::*;
    use crate::frames::office::docs::docs_fixture::DOCX_ID;
    use crate::frames::office::edit::{self, EditUi, word_ribbon};
    use crate::frames::viewers;
    use crate::frames::viewers::source::EditAnswer;

    type Ops = Rc<RefCell<Vec<EditOp>>>;
    type Answers = Rc<RefCell<Vec<EditAnswer>>>;

    /// A live-looking document whose editor records every op it is given and answers what the test queued.
    struct Editing {
        inner: FixtureViewerSource,
        ops: Ops,
        answers: Answers,
        takes: bool,
    }

    impl ViewerSource for Editing {
        fn open(&mut self, id: &str) -> Result<OpenFile, OpenError> {
            self.inner.open(id)
        }
        fn poll(&mut self, id: &str) -> Option<Body> {
            self.inner.poll(id)
        }
        fn cancel(&mut self, _: &str) {}
        fn retry(&mut self, id: &str) -> Result<OpenFile, OpenError> {
            self.inner.retry(id)
        }
        fn can_edit(&self, _: &str) -> bool {
            true
        }
        fn edit(&mut self, _: &str, op: EditOp) -> bool {
            self.ops.borrow_mut().push(op);
            self.takes
        }
        fn poll_edit(&mut self, _: &str) -> Option<EditAnswer> {
            let mut answers = self.answers.borrow_mut();
            (!answers.is_empty()).then(|| answers.remove(0))
        }
    }

    pub fn marks() -> Marks {
        Marks {
            bold: false,
            italic: false,
            underline: false,
            strike: false,
            style: ParaStyle::Normal,
            align: TextAlign::Left,
            list: ListKind::None,
        }
    }

    pub fn edited(revision: u64, marks: Marks, selection: Vec<PageRect>) -> Edited {
        Edited {
            revision,
            caret: None,
            selection,
            can_undo: true,
            can_redo: false,
            dirty: Dirty::All,
            pages: None,
            cells: Vec::new(),
            slides: None,
            marks,
        }
    }

    pub fn selection() -> Vec<PageRect> {
        vec![PageRect {
            page: 0,
            rect: RectPt {
                x: 72.0,
                y: 300.0,
                w: 120.0,
                h: 14.0,
            },
        }]
    }

    /// The agreement open for editing: the editor's state is on, the source records.
    fn editing(takes: bool) -> (Runtime<State>, Ops, Answers) {
        let (ops, answers) = (Ops::default(), Answers::default());
        let mut state = init();
        let source = Editing {
            inner: FixtureViewerSource::new(),
            ops: ops.clone(),
            answers: answers.clone(),
            takes,
        };
        assert!(open_from(&mut state, Box::new(source), DOCX_ID));
        state.viewer.ui.as_mut().expect("open").office.edit = Some(EditUi::new(EditKind::Docx));
        (runtime_of(state, false), ops, answers)
    }

    /// The helper answered `e`: the editor's poll takes it, the frame is drawn again.
    pub fn answer(rt: &mut Runtime<State>, answers: &Answers, e: Edited) {
        answers.borrow_mut().push(EditAnswer::Edited(e));
        rt.update(|s, tx| {
            edit::poll(s, tx);
        });
        // The lit fill is a short transition: let it land.
        for _ in 0..4 {
            rt.frame(1.0);
        }
    }

    fn checked(rt: &Runtime<State>, key: &str) -> bool {
        find(rt, key).unwrap_or_else(|| panic!("no element keyed {key}")).fill != rune::Color::TRANSPARENT
    }

    #[test]
    fn the_tools_are_lit_from_what_the_helper_said_and_send_the_opposite() {
        let (mut rt, ops, answers) = editing(true);
        // Nothing was said yet: nothing is lit, and Undo and Redo have nothing to do.
        assert!(!checked(&rt, "rt-2"));
        let undo = find(&rt, "rt-0").expect("Undo");
        assert!(!undo.focusable, "no revision to go back to yet");
        let mut m = marks();
        m.bold = true;
        m.align = TextAlign::Center;
        m.list = ListKind::Bullet;
        m.style = ParaStyle::Heading2;
        answer(&mut rt, &answers, edited(1, m, Vec::new()));
        assert!(checked(&rt, "rt-2"), "Bold is lit");
        assert!(!checked(&rt, "rt-3"), "Italic is not");
        assert!(checked(&rt, "rt-11"), "Align centre is lit");
        assert!(!checked(&rt, "rt-10"));
        assert!(checked(&rt, "rt-8"), "Bulleted list is lit");
        assert_eq!(words(&rt, "rp-0-label"), "Heading 2");
        assert!(find(&rt, "rt-0").expect("Undo").focusable);
        assert!(!find(&rt, "rt-1").expect("Redo").focusable);
        // A lit Bold turns Bold off; an unlit Italic turns it on.
        operate(&mut rt, "rt-2");
        operate(&mut rt, "rt-3");
        operate(&mut rt, "rt-0");
        // A lit list tool turns the list off; Numbered turns it on.
        operate(&mut rt, "rt-8");
        operate(&mut rt, "rt-9");
        operate(&mut rt, "rt-12");
        operate(&mut rt, "rt-15");
        assert_eq!(
            *ops.borrow(),
            [
                EditOp::Mark {
                    mark: Mark::Bold,
                    on: false
                },
                EditOp::Mark {
                    mark: Mark::Italic,
                    on: true
                },
                EditOp::Undo,
                EditOp::List(ListKind::None),
                EditOp::List(ListKind::Number),
                EditOp::Align(TextAlign::Right),
                EditOp::Indent(1),
            ]
        );
    }

    #[test]
    fn a_source_that_does_not_take_the_edit_says_so_and_never_nothing() {
        let (mut rt, ops, _) = editing(false);
        operate(&mut rt, "rt-2");
        assert_eq!(ops.borrow().len(), 1);
        // The shell's router answers the intent ("Not built yet: ..."): the press is never nothing.
        let pushed = rt.state.viewer.ui.as_mut().expect("open").take_intents();
        assert!(!pushed.is_empty(), "the press said it did nothing");
    }

    #[test]
    fn the_style_pop_up_picks_with_the_arrows_and_closes() {
        let (mut rt, ops, _) = editing(true);
        operate(&mut rt, "rp-0");
        assert!(has(&rt, "rib-menu"), "the menu hangs under the Style button");
        assert_eq!(ui(&rt).office.ribbon.open, Some(word_ribbon::Pop::Style));
        for _ in 0..3 {
            press(&mut rt, Key::Down);
        }
        press(&mut rt, Key::Enter);
        assert_eq!(*ops.borrow(), [EditOp::ParaStyle(ParaStyle::Heading1)]);
        assert!(!has(&rt, "rib-menu"));
        assert!(ui(&rt).office.ribbon.open.is_none());
    }

    #[test]
    fn colour_font_size_and_table_pop_ups_send_their_edits() {
        let (mut rt, ops, _) = editing(true);
        for (button, downs) in [("rp-1", 2), ("rp-2", 5), ("rt-6", 3), ("rt-18", 2)] {
            operate(&mut rt, button);
            assert!(has(&rt, "rib-menu"), "{button} opens a menu");
            for _ in 0..downs {
                press(&mut rt, Key::Down);
            }
            press(&mut rt, Key::Enter);
        }
        let ops = ops.borrow();
        assert_eq!(ops[0], EditOp::FontFamily("Arial".into()));
        assert_eq!(ops[1], EditOp::FontSize(12.0));
        assert!(matches!(ops[2], EditOp::TextColour(c) if (c.r, c.g, c.b) == (0xC0, 0x39, 0x2B)));
        assert_eq!(ops[3], EditOp::Table { rows: 3, cols: 3 });
    }

    #[test]
    fn the_link_pop_up_takes_a_web_or_mail_address_only() {
        assert_eq!(
            word_ribbon::link_address("https://beebeeb.io/x"),
            Some("https://beebeeb.io/x".into())
        );
        assert_eq!(
            word_ribbon::link_address("beebeeb.io"),
            Some("https://beebeeb.io".into())
        );
        assert_eq!(word_ribbon::link_address("a@b.nl"), Some("mailto:a@b.nl".into()));
        assert_eq!(word_ribbon::link_address("MAILTO:a@b.nl"), Some("MAILTO:a@b.nl".into()));
        for wrong in [
            "",
            "javascript:alert(1)",
            "file:///etc/passwd",
            "ftp://x.y",
            "two words.nl",
            "plain",
        ] {
            assert_eq!(word_ribbon::link_address(wrong), None, "{wrong}");
        }
        let (mut rt, ops, _) = editing(true);
        operate(&mut rt, "rt-16");
        assert!(has(&rt, "rib-link-input"));
        press(&mut rt, Key::Text("javascript:x".into()));
        press(&mut rt, Key::Enter);
        assert!(
            ops.borrow().is_empty(),
            "an address the engine would refuse is not sent"
        );
        assert!(has(&rt, "rib-link-wrong"));
        rt.update(|s, _| {
            if let Some(link) = s.viewer.ui.as_mut().and_then(|u| u.office.ribbon.link.as_mut()) {
                link.replace_text("beebeeb.io");
            }
        });
        rt.frame(0.0);
        press(&mut rt, Key::Enter);
        assert_eq!(
            *ops.borrow(),
            [EditOp::Link {
                text: String::new(),
                url: "https://beebeeb.io".into()
            }]
        );
        assert!(ui(&rt).office.ribbon.link.is_none(), "the typed address is not kept");
    }

    #[test]
    fn the_selection_tools_show_over_a_selection_only() {
        let (mut rt, ops, answers) = editing(true);
        assert!(!has(&rt, "float"));
        answer(&mut rt, &answers, edited(1, marks(), Vec::new()));
        assert!(!has(&rt, "float"), "a caret is not a selection");
        let mut m = marks();
        m.underline = true;
        answer(&mut rt, &answers, edited(2, m, selection()));
        assert!(has(&rt, "float"));
        for (i, name) in ["Bold", "Italic", "Underline", "Text colour", "Insert link"]
            .iter()
            .enumerate()
        {
            assert_eq!(
                find(&rt, &format!("ft-{i}")).expect("a tool").label.as_deref(),
                Some(*name)
            );
        }
        assert!(checked(&rt, "ft-2"), "Underline is lit");
        operate(&mut rt, "ft-0");
        operate(&mut rt, "ft-2");
        assert_eq!(
            *ops.borrow(),
            [
                EditOp::Mark {
                    mark: Mark::Bold,
                    on: true
                },
                EditOp::Mark {
                    mark: Mark::Underline,
                    on: false
                },
            ]
        );
        // Over the first selected line, scaled to the page at this zoom, inside the page.
        let (x, y, page_w) = word_ribbon::float_at(ui(&rt)).expect("on screen");
        assert!(x >= 0.0 && x + 160.0 <= page_w + 1.0 && y >= 0.0);
    }

    #[test]
    fn a_selection_scrolled_out_of_view_has_no_tools() {
        let (mut rt, _, answers) = editing(true);
        answer(&mut rt, &answers, edited(1, marks(), selection()));
        assert!(word_ribbon::float_at(ui(&rt)).is_some());
        rt.update(|s, _| {
            if let Some(d) = viewers::doc_mut(s) {
                d.go_to_page(3);
            }
        });
        assert!(word_ribbon::float_at(ui(&rt)).is_none());
    }

    #[test]
    fn the_status_line_is_the_live_documents() {
        let (mut rt, _, answers) = editing(true);
        assert_eq!(words(&rt, "st-0"), "Page 1 of 4");
        assert!(words(&rt, "st-2").ends_with(" words"));
        assert_eq!(words(&rt, "st-4"), "English (UK)");
        assert_eq!(words(&rt, "st-7"), "saved as version 7");
        assert!(!has(&rt, "status-text"), "never Preparing");
        assert_eq!(words(&rt, "st-8-zoom"), "100%");
        answer(&mut rt, &answers, edited(1, marks(), Vec::new()));
        assert_eq!(words(&rt, "st-7"), "unsaved changes");
        // The zoom's steps move the document's stage.
        operate(&mut rt, "st-8-more-btn");
        assert_eq!(words(&rt, "st-8-zoom"), "110%");
        operate(&mut rt, "st-8-less-btn");
        operate(&mut rt, "st-8-less-btn");
        assert_eq!(words(&rt, "st-8-zoom"), "90%");
    }

    #[test]
    fn layout_and_review_keep_only_what_the_web_editor_has() {
        let (mut rt, _, _) = editing(true);
        operate(&mut rt, "rib-tab-2-cell");
        assert_eq!(words(&rt, "rt-0-label"), "Page layout\u{2026}");
        assert!(!has(&rt, "rt-1"));
        operate(&mut rt, "rib-tab-3-cell");
        assert_eq!(words(&rt, "rt-0-label"), "Word count\u{2026}");
        operate(&mut rt, "rt-0");
        let said = ui(&rt).toast.clone().expect("it says the count");
        assert!(said.contains("words") && said.contains("4 pages"), "{said}");
    }

    #[test]
    fn the_image_tool_goes_to_inserts_picker_and_the_fixture_ribbon_is_unchanged() {
        let (mut rt, ops, _) = editing(true);
        operate(&mut rt, "rt-17");
        assert_eq!(ui(&rt).office.tab, Tab::Insert);
        assert!(ops.borrow().is_empty());
        // The board's own scene keeps its fixture tools and words.
        let (rt, _) = agreement(Case::Selected, false, true);
        assert_eq!(words(&rt, "rp-0-label"), "Normal text");
        assert_eq!(words(&rt, "rp-1-label"), "Inter");
        assert_eq!(words(&rt, "rp-2-label"), "11");
        assert!(!find(&rt, "rt-1").expect("Redo").focusable, "the board's Redo is off");
        assert_eq!(words(&rt, "st-0"), "Page 1 of 4");
    }
}
