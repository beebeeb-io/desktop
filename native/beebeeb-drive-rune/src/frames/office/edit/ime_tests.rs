//! The input method and the clipboard of the live editor, through Rune's headless `Runtime` (the sequences are
//! the input check's: a commit, a cancel as an empty commit, a pre-edit with a byte range, copy and paste through
//! `rune::clipboard::memory()`). The window spec here sets `on_ime` and `poll` itself, as the shell's will.

use std::cell::RefCell;
use std::rc::Rc;

use beebeeb_office_protocol::check::MAX_OP_TEXT_BYTES;
use beebeeb_office_protocol::office::{EditKind, EditOp, PageRect, RectPt};
use rune::clipboard::{MemoryClipboard, memory};
use rune::{App, AppSpec, ClipboardContent, Color, Cx, El, Env, Ime, Key, KeyEvent, Mods, Runtime, Tx, el};

use super::{EditUi, clipboard, ime};
use crate::frames::viewers::source::{Body, EditAnswer, FixtureViewerSource, OpenError, OpenFile, ViewerSource, board};
use crate::frames::viewers::{self, open_from};
use crate::{State, init};

/// What the source was asked, in order.
#[derive(Debug, Clone, PartialEq)]
enum Asked {
    Edit(EditOp),
    Copy,
}

type Log = Rc<RefCell<Vec<Asked>>>;

/// The fixture, writing down the editor's calls. `copy` is what a copy answers (`None`: the call is not taken).
struct Recording {
    inner: FixtureViewerSource,
    log: Log,
    copy: Option<(String, Option<String>)>,
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
        self.inner.retry(id)
    }
    fn can_edit(&self, _: &str) -> bool {
        true
    }
    fn edit(&mut self, _: &str, op: EditOp) -> bool {
        self.log.borrow_mut().push(Asked::Edit(op));
        true
    }
    fn copy_selection(&mut self, _: &str) -> bool {
        self.log.borrow_mut().push(Asked::Copy);
        let Some((text, html)) = self.copy.clone() else { return false };
        self.inner.edit_answers.push(EditAnswer::Clip { text, html });
        true
    }
    fn poll_edit(&mut self, id: &str) -> Option<EditAnswer> {
        self.inner.poll_edit(id)
    }
}

/// The editor's page: a focusable surface that takes the clipboard keys first, with the caret element and the
/// pre-edit where a real surface would put them.
fn view(s: &State, _: &mut Cx<State>) -> El {
    let mut page = el().key("edit-page").size(300.0, 200.0).focusable(true).on_key(
        |s: &mut State, ev: &KeyEvent, tx: &mut Tx| clipboard::on_clip_key(s, tx, ev),
    );
    if let Some(caret) = ime::caret_el(s) {
        page = page.child(caret.absolute(40.0, 50.0));
    }
    if let Some(pre) = ime::preedit_el(s, 14.0, Color::new(0.1, 0.1, 0.12, 1.0)) {
        page = page.child(pre.absolute(40.0, 70.0));
    }
    page
}

fn poll(s: &mut State, _: f64, tx: &mut Tx) -> bool {
    let edits = super::poll(s, tx);
    clipboard::flush_clip(s, tx) | edits
}

fn on_ime(s: &mut State, ime: &Ime, tx: &mut Tx) -> bool {
    ime::on_ime(s, tx, ime.clone());
    true
}

struct Harness {
    rt: Runtime<State>,
    log: Log,
    clipboard: MemoryClipboard,
}

fn harness(kind: EditKind, copy: Option<(&str, Option<&str>)>) -> Harness {
    let log = Log::default();
    let spec = AppSpec::<State> {
        poll: Some(poll),
        ..AppSpec::new("ime", (400.0, 300.0), init, view)
    };
    let mut rt = App::new(spec).on_ime(on_ime).headless_runtime((400.0, 300.0), Env::new(false, 1.0));
    let clipboard = memory();
    rt.use_clipboard(&clipboard);
    let source = Recording {
        inner: FixtureViewerSource::new(),
        log: log.clone(),
        copy: copy.map(|(t, h)| (t.to_string(), h.map(str::to_string))),
    };
    rt.update(|s, _| {
        assert!(open_from(s, Box::new(source), board::DOCX));
        let mut edit = EditUi::new(kind);
        edit.caret = Some(PageRect { page: 0, rect: RectPt { x: 72.0, y: 96.0, w: 1.0, h: 14.0 } });
        viewers::ui(s).expect("a file").office.edit = Some(edit);
    });
    rt.frame(0.0);
    Harness { rt, log, clipboard }
}

impl Harness {
    fn settle(&mut self) {
        for _ in 0..4 {
            self.rt.frame(1.0 / 60.0);
        }
    }

    /// A press on the page gives it the keyboard.
    fn focus(&mut self) {
        self.rt.pointer_move(100.0, 100.0);
        self.rt.pointer_down(100.0, 100.0);
        self.rt.pointer_up(100.0, 100.0);
        self.settle();
    }

    fn ime(&mut self, ev: Ime) {
        self.rt.ime(ev);
        self.settle();
    }

    fn ctrl(&mut self, c: char) {
        self.rt.key(KeyEvent { key: Key::Command(c), mods: Mods { cmd: true, ctrl: true, ..Mods::default() } });
        self.settle();
    }

    fn asked(&self) -> Vec<Asked> {
        self.log.borrow().clone()
    }

    fn edit(&mut self) -> EditUi {
        viewers::ui(&mut self.rt.state).and_then(|ui| ui.office.edit.clone()).expect("an editor")
    }

    fn has(&self, key: &str) -> bool {
        self.rt.elements().into_iter().any(|e| e.key.as_deref() == Some(key) && !e.exiting)
    }
}

fn insert(text: &str) -> Asked {
    Asked::Edit(EditOp::InsertText(text.to_string()))
}

fn paste(text: &str) -> Asked {
    Asked::Edit(EditOp::Paste { text: text.to_string() })
}

const DELETE: Asked = Asked::Edit(EditOp::Delete { back: false, word: false });

// The input method.

#[test]
fn a_commit_inserts_its_text_once_and_a_preedit_sends_nothing() {
    let mut h = harness(EditKind::Docx, None);
    h.focus();
    h.ime(Ime::Enabled);
    h.ime(Ime::Preedit("か".into(), Some((3, 3))));
    h.ime(Ime::Preedit("かな".into(), Some((0, 6))));
    assert_eq!(h.asked(), [], "nothing crosses to the helper while composing");
    assert_eq!(h.edit().preedit.as_deref(), Some("かな"));
    // The input method's own commit: the pre-edit emptied, the result, then off.
    h.ime(Ime::Preedit(String::new(), None));
    h.ime(Ime::Commit("仮名".into()));
    h.ime(Ime::Disabled);
    assert_eq!(h.asked(), [insert("仮名")]);
    assert_eq!(h.edit().preedit, None);
}

#[test]
fn a_cancelled_composition_is_an_empty_commit_and_inserts_nothing() {
    let mut h = harness(EditKind::Docx, None);
    h.focus();
    h.ime(Ime::Preedit("か".into(), Some((3, 3))));
    assert!(h.has("office-preedit"));
    // Escape, as the input check recorded it.
    h.ime(Ime::Preedit(String::new(), None));
    h.ime(Ime::Preedit(String::new(), None));
    h.ime(Ime::Commit(String::new()));
    h.ime(Ime::Disabled);
    assert_eq!(h.asked(), []);
    assert_eq!(h.edit().preedit, None);
    assert!(!h.has("office-preedit"), "the composition is gone from the page");
}

#[test]
fn the_preedit_range_is_in_bytes_and_marks_the_clause_being_converted() {
    let mut h = harness(EditKind::Docx, None);
    h.focus();
    // Typing: the range is a caret, nothing is marked.
    h.ime(Ime::Preedit("かな".into(), Some((6, 6))));
    assert_eq!(ime::preedit_range(&h.edit()), None);
    assert!(h.has("office-preedit"));
    // Converting: the first character's three bytes are the clause.
    h.ime(Ime::Preedit("かな".into(), Some((0, 3))));
    assert_eq!(ime::preedit_range(&h.edit()), Some((0, 3)));
    // A range that is not on character boundaries (or past the end) is not marked, and cannot panic the drawing.
    h.ime(Ime::Preedit("かな".into(), Some((1, 4))));
    assert_eq!(ime::preedit_range(&h.edit()), None);
    assert!(h.has("office-preedit"));
    h.ime(Ime::Preedit("かな".into(), Some((0, 99))));
    assert_eq!(ime::preedit_range(&h.edit()), None);
    // No range at all.
    h.ime(Ime::Preedit("か".into(), None));
    assert_eq!(ime::preedit_range(&h.edit()), None);
    // The end of the composition takes the range with it.
    h.ime(Ime::Preedit("かな".into(), Some((0, 3))));
    h.ime(Ime::Preedit(String::new(), None));
    assert_eq!(ime::preedit_range(&h.edit()), None);
    assert_eq!(h.asked(), []);
}

#[test]
fn a_long_commit_goes_in_pieces_the_protocol_takes_whole() {
    let mut h = harness(EditKind::Docx, None);
    h.focus();
    // Two-byte characters: a piece must never be cut inside one.
    let long = "é".repeat(MAX_OP_TEXT_BYTES + 3);
    h.ime(Ime::Commit(long.clone()));
    let pieces: Vec<String> = h
        .asked()
        .into_iter()
        .map(|a| match a {
            Asked::Edit(EditOp::InsertText(t)) => t,
            other => panic!("not an insert: {other:?}"),
        })
        .collect();
    assert!(pieces.len() >= 2);
    assert!(pieces.iter().all(|p| p.len() <= MAX_OP_TEXT_BYTES));
    assert_eq!(pieces.concat(), long);
}

#[test]
fn only_a_caret_the_editor_has_the_keyboard_for_asks_for_the_input_method() {
    let mut h = harness(EditKind::Docx, None);
    assert_eq!(h.rt.ime_area, None, "the page has not the keyboard yet");
    h.focus();
    let area = h.rt.ime_area.expect("the caret's rectangle");
    assert_eq!((area[0], area[1], area[3]), (40.0, 50.0, 14.0), "the surface placed it, the helper sized it");
    assert!(h.edit().has_keyboard);
    h.rt.update(|_, tx| tx.blur());
    h.settle();
    assert!(!h.edit().has_keyboard);
    assert_eq!(h.rt.ime_area, None, "the keyboard went elsewhere: the input method is off");
}

#[test]
fn a_stopped_editor_takes_no_input_method_text() {
    let mut h = harness(EditKind::Docx, None);
    h.focus();
    h.ime(Ime::Preedit("か".into(), Some((3, 3))));
    h.rt.update(|s, _| viewers::ui(s).unwrap().office.edit.as_mut().unwrap().stopped = true);
    h.ime(Ime::Commit("x".into()));
    assert_eq!(h.asked(), []);
    assert_eq!(h.edit().preedit, None);
    assert!(h.rt.ime_area.is_none(), "a stopped editor has no caret to give");
}

#[test]
fn without_an_editor_the_input_method_does_nothing() {
    let mut h = harness(EditKind::Docx, None);
    h.rt.update(|s, _| viewers::ui(s).unwrap().office.edit = None);
    h.ime(Ime::Preedit("か".into(), Some((3, 3))));
    h.ime(Ime::Commit("か".into()));
    assert_eq!(h.asked(), []);
}

// The clipboard.

#[test]
fn copy_asks_the_editor_and_writes_text_and_html_when_the_answer_comes() {
    let mut h = harness(EditKind::Docx, Some(("Total due", Some("<b>Total due</b>"))));
    h.focus();
    h.ctrl('c');
    assert_eq!(h.asked(), [Asked::Copy]);
    assert_eq!(h.clipboard.text().as_deref().map(String::as_str), Some("Total due"));
    assert_eq!(
        h.clipboard.content().and_then(|c| c.html().map(str::to_owned)).as_deref(),
        Some("<b>Total due</b>")
    );
    assert_eq!(h.edit().clip, None, "the clip was taken");
}

#[test]
fn copy_without_html_writes_the_text_alone_and_an_empty_answer_leaves_the_clipboard() {
    let mut h = harness(EditKind::Docx, Some(("plain", None)));
    h.focus();
    h.ctrl('c');
    let written = h.clipboard.content().expect("content");
    assert_eq!((written.text(), written.html()), ("plain", None));
    // Nothing selected: the helper answers an empty text, and what is on the clipboard stays.
    let mut h = harness(EditKind::Docx, Some(("", None)));
    h.clipboard.set_text("keep me");
    h.focus();
    h.ctrl('c');
    assert_eq!(h.clipboard.text().as_deref().map(String::as_str), Some("keep me"));
}

#[test]
fn cut_copies_first_and_then_deletes() {
    let mut h = harness(EditKind::Docx, Some(("gone", None)));
    h.rt.update(|s, _| {
        let rect = PageRect { page: 0, rect: RectPt { x: 72.0, y: 96.0, w: 30.0, h: 14.0 } };
        viewers::ui(s).unwrap().office.edit.as_mut().unwrap().selection = vec![rect];
    });
    h.focus();
    h.ctrl('x');
    assert_eq!(h.asked(), [Asked::Copy, DELETE]);
    assert_eq!(h.clipboard.text().as_deref().map(String::as_str), Some("gone"));
}

#[test]
fn a_cut_with_nothing_selected_in_a_document_deletes_nothing() {
    let mut h = harness(EditKind::Docx, Some(("", None)));
    h.focus();
    h.ctrl('x');
    assert_eq!(h.asked(), [Asked::Copy], "a caret is not a selection: Delete would take the next character");
}

#[test]
fn a_cut_whose_copy_was_not_taken_deletes_nothing() {
    let mut h = harness(EditKind::Xlsx, None);
    h.focus();
    h.ctrl('x');
    assert_eq!(h.asked(), [Asked::Copy]);
}

#[test]
fn a_cut_in_a_sheet_clears_the_cells_it_copied() {
    // Cells and shapes have no selection rectangles of their own to look at.
    let mut h = harness(EditKind::Xlsx, Some(("1\t2", None)));
    h.focus();
    h.ctrl('x');
    assert_eq!(h.asked(), [Asked::Copy, DELETE]);
}

#[test]
fn paste_sends_the_clipboards_plain_text_even_when_html_is_there() {
    let mut h = harness(EditKind::Docx, None);
    h.focus();
    h.clipboard.set_content(ClipboardContent::new("one\r\ntwo\rthree").with_html("<b>one</b>"));
    h.ctrl('v');
    assert_eq!(h.asked(), [paste("one\ntwo\nthree")]);
}

#[test]
fn an_empty_clipboard_pastes_nothing() {
    let mut h = harness(EditKind::Docx, None);
    h.focus();
    h.clipboard.set_content(ClipboardContent::default());
    h.ctrl('v');
    assert_eq!(h.asked(), []);
}

#[test]
fn a_paste_over_what_an_edit_carries_is_refused_with_words() {
    let mut h = harness(EditKind::Docx, None);
    h.focus();
    h.clipboard.set_text(&"a".repeat(MAX_OP_TEXT_BYTES + 1));
    h.ctrl('v');
    assert_eq!(h.asked(), []);
    let toast = viewers::ui(&mut h.rt.state).and_then(|ui| ui.toast.clone());
    assert_eq!(toast.as_deref(), Some(clipboard::PASTE_TOO_LARGE));
    // The largest one that fits goes.
    h.clipboard.set_text(&"a".repeat(MAX_OP_TEXT_BYTES));
    h.ctrl('v');
    assert_eq!(h.asked().len(), 1);
}

#[test]
fn other_keys_are_not_taken_and_neither_is_a_chord_with_alt_or_a_stopped_editor() {
    let mut h = harness(EditKind::Docx, None);
    let key = |key, mods| KeyEvent { key, mods };
    let cmd = Mods { cmd: true, ctrl: true, ..Mods::default() };
    let mut taken = Vec::new();
    h.rt.update(|s, tx| {
        taken.push(clipboard::on_clip_key(s, tx, &key(Key::Text("c".into()), Mods::default())));
        taken.push(clipboard::on_clip_key(s, tx, &key(Key::Command('z'), cmd)));
        taken.push(clipboard::on_clip_key(s, tx, &key(Key::Command('c'), Mods { alt: true, ..cmd })));
        taken.push(clipboard::on_clip_key(s, tx, &key(Key::Command('c'), Mods { shift: true, ..cmd })));
        taken.push(clipboard::on_clip_key(s, tx, &key(Key::Command('c'), cmd)));
        viewers::ui(s).unwrap().office.edit.as_mut().unwrap().stopped = true;
        taken.push(clipboard::on_clip_key(s, tx, &key(Key::Command('c'), cmd)));
        viewers::ui(s).unwrap().office.edit = None;
        taken.push(clipboard::on_clip_key(s, tx, &key(Key::Command('c'), cmd)));
    });
    assert_eq!(taken, [false, false, false, false, true, false, false]);
    assert_eq!(h.asked(), [Asked::Copy], "only the one taken chord asked the editor");
}
