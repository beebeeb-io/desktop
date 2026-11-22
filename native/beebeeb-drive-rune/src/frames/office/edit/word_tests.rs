//! Tests of the word surface: the keys and the pointer become the closed `EditOp`s and hits, the caret and the
//! selection are drawn from what the editor answered, the stage follows the caret, and a document that is only
//! viewed has none of it.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use beebeeb_office_protocol::office::{Dirty, ListKind, ParaStyle, PageRect, RectPt, TextAlign};
use beebeeb_office_protocol::office::{EditKind, Edited};
use rune::{Env, Mods, Pixels, Runtime, WheelEvent};

use super::*;
use crate::frames::office::docs::docs_fixture;
use crate::frames::office::docs::live_tests::{doc_of, find};
use crate::frames::office::edit::{self, EditUi};
use crate::frames::viewers::kinds::OfficePicture;
use crate::frames::viewers::source::{Body, EditAnswer, OpenError, OpenFile};
use crate::{init, view};

/// What the surface asked of the editor, in order.
#[derive(Debug, Clone, PartialEq)]
enum Call {
    Op(EditOp),
    Hit(u32, f32, f32, HitMode),
}

#[derive(Default)]
struct Shared {
    calls: RefCell<Vec<Call>>,
    answers: RefCell<VecDeque<EditAnswer>>,
}

struct EditSource {
    file: OpenFile,
    shared: Rc<Shared>,
}

impl ViewerSource for EditSource {
    fn open(&mut self, _id: &str) -> Result<OpenFile, OpenError> {
        Ok(self.file.clone())
    }
    fn poll(&mut self, _id: &str) -> Option<Body> {
        None
    }
    fn cancel(&mut self, _id: &str) {}
    fn retry(&mut self, _id: &str) -> Result<OpenFile, OpenError> {
        Ok(self.file.clone())
    }
    fn office_live(&self) -> bool {
        true
    }
    fn office_picture(&self, _id: &str, _which: OfficePicture) -> Option<Pixels> {
        let p = Pixels::new();
        p.set(8, 8, vec![255; 8 * 8 * 4]);
        Some(p)
    }
    fn can_edit(&self, _id: &str) -> bool {
        true
    }
    fn edit(&mut self, _id: &str, op: EditOp) -> bool {
        self.shared.calls.borrow_mut().push(Call::Op(op));
        true
    }
    fn hit(&mut self, _id: &str, page: u32, x: f32, y: f32, mode: HitMode) -> bool {
        self.shared.calls.borrow_mut().push(Call::Hit(page, x, y, mode));
        true
    }
    fn poll_edit(&mut self, _id: &str) -> Option<EditAnswer> {
        self.shared.answers.borrow_mut().pop_front()
    }
}

fn marks() -> Marks {
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

fn at(page: u32, x: f32, y: f32) -> PageRect {
    PageRect {
        page,
        rect: RectPt { x, y, w: 1.0, h: 14.0 },
    }
}

fn edited(revision: u64, caret: Option<PageRect>, selection: Vec<PageRect>, marks: Marks) -> Edited {
    Edited {
        revision,
        caret,
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

/// A four-page document open for editing from a source that answers what the test queues.
fn editor(editing: bool) -> (Runtime<State>, Rc<Shared>) {
    editor_at(editing, 100, 1.0)
}

/// [`editor`] with the stage at `zoom` per cent on a display of `scale` (1.5: Windows at 150 %).
fn editor_at(editing: bool, zoom: u16, scale: f32) -> (Runtime<State>, Rc<Shared>) {
    let mut file = docs_fixture::file(false);
    file.body = doc_of(4, zoom, 1);
    let shared = Rc::new(Shared::default());
    let mut state = init();
    let id = file.id.clone();
    let source = EditSource {
        file,
        shared: shared.clone(),
    };
    assert!(viewers::open_from(&mut state, Box::new(source), &id));
    if editing {
        viewers::ui(&mut state).expect("open").office.edit = Some(EditUi::new(EditKind::Docx));
    }
    let env = Env {
        system_dark: false,
        scale,
    };
    let mut rt = Runtime::new(state, view, (1278.0, 798.0), env);
    rt.on_key = Some(crate::shell::host::on_key);
    rt.frame(0.0);
    rt.frame(0.0);
    (rt, shared)
}

/// The editor answers `e`; the surface takes it in as the shell's poll will.
fn answer(rt: &mut Runtime<State>, shared: &Shared, e: Edited) {
    shared.answers.borrow_mut().push_back(EditAnswer::Edited(e));
    rt.update(|s, tx| {
        edit::poll(s, tx);
    });
    rt.frame(0.0);
    rt.frame(0.0);
}

fn focus_surface(rt: &mut Runtime<State>) {
    rt.update(|_, tx| focus(tx));
    rt.frame(0.0);
}

fn press(rt: &mut Runtime<State>, key: Key, mods: Mods) {
    rt.key(KeyEvent { key, mods });
    rt.frame(0.0);
}

fn plain(rt: &mut Runtime<State>, key: Key) {
    press(rt, key, Mods::default());
}

fn ctrl() -> Mods {
    Mods {
        ctrl: true,
        cmd: true,
        ..Mods::default()
    }
}

fn shift() -> Mods {
    Mods {
        shift: true,
        ..Mods::default()
    }
}

fn calls(shared: &Shared) -> Vec<Call> {
    shared.calls.borrow().clone()
}

fn ops(shared: &Shared) -> Vec<EditOp> {
    calls(shared)
        .into_iter()
        .filter_map(|c| match c {
            Call::Op(op) => Some(op),
            Call::Hit(..) => None,
        })
        .collect()
}

fn rect(rt: &Runtime<State>, key: &str) -> [f32; 4] {
    find(rt, key).unwrap_or_else(|| panic!("{key} is drawn")).rect
}

fn key_event(key: Key, mods: Mods) -> KeyEvent {
    KeyEvent { key, mods }
}

/// The points of a page are 520 / 595 layout points at 100 %.
const K: f32 = 520.0 / 595.0;

#[test]
fn keys_become_ops() {
    let m = marks();
    let op = |key: Key, mods: Mods| op_for(&key_event(key, mods), Some(&m));
    assert_eq!(op(Key::Text("a".into()), Mods::default()), Some(EditOp::InsertText("a".into())));
    assert_eq!(op(Key::Text(" ".into()), Mods::default()), Some(EditOp::InsertText(" ".into())));
    assert_eq!(op(Key::Enter, Mods::default()), Some(EditOp::InsertParagraph));
    assert_eq!(op(Key::Enter, shift()), Some(EditOp::InsertLineBreak));
    assert_eq!(op(Key::Backspace, Mods::default()), Some(EditOp::Delete { back: true, word: false }));
    assert_eq!(op(Key::Backspace, ctrl()), Some(EditOp::Delete { back: true, word: true }));
    assert_eq!(op(Key::Delete, Mods::default()), Some(EditOp::Delete { back: false, word: false }));
    assert_eq!(op(Key::Delete, ctrl()), Some(EditOp::Delete { back: false, word: true }));
    for (key, to) in [
        (Key::Left, Motion::Left),
        (Key::Right, Motion::Right),
        (Key::Up, Motion::Up),
        (Key::Down, Motion::Down),
        (Key::Home, Motion::LineStart),
        (Key::End, Motion::LineEnd),
        (Key::PageUp, Motion::PageUp),
        (Key::PageDown, Motion::PageDown),
    ] {
        assert_eq!(op(key.clone(), Mods::default()), Some(EditOp::Move { to, extend: false }));
        assert_eq!(op(key, shift()), Some(EditOp::Move { to, extend: true }));
    }
    assert_eq!(op(Key::Left, ctrl()), Some(EditOp::Move { to: Motion::WordLeft, extend: false }));
    assert_eq!(
        op(Key::Right, Mods { shift: true, ..ctrl() }),
        Some(EditOp::Move { to: Motion::WordRight, extend: true })
    );
    assert_eq!(op(Key::Home, ctrl()), Some(EditOp::Move { to: Motion::DocStart, extend: false }));
    assert_eq!(
        op(Key::End, Mods { shift: true, ..ctrl() }),
        Some(EditOp::Move { to: Motion::DocEnd, extend: true })
    );
    assert_eq!(op(Key::Command('a'), ctrl()), Some(EditOp::SelectAll));
    assert_eq!(op(Key::Command('z'), ctrl()), Some(EditOp::Undo));
    assert_eq!(op(Key::Command('y'), ctrl()), Some(EditOp::Redo));
    assert_eq!(op(Key::Command('z'), Mods { shift: true, ..ctrl() }), Some(EditOp::Redo));
    assert_eq!(
        op(Key::Command('b'), ctrl()),
        Some(EditOp::Mark { mark: Mark::Bold, on: true })
    );
    assert_eq!(
        op(Key::Command('i'), ctrl()),
        Some(EditOp::Mark { mark: Mark::Italic, on: true })
    );
    assert_eq!(
        op(Key::Command('u'), ctrl()),
        Some(EditOp::Mark { mark: Mark::Underline, on: true })
    );
}

/// What is not the surface's: other chords, Alt keys, control characters, Tab and Escape (focus moves with them).
#[test]
fn other_keys_are_left_alone() {
    let op = |key: Key, mods: Mods| op_for(&key_event(key, mods), None);
    assert_eq!(op(Key::Command('k'), ctrl()), None);
    assert_eq!(op(Key::Command('s'), ctrl()), None);
    assert_eq!(op(Key::Command('+'), ctrl()), None);
    assert_eq!(op(Key::Command('b'), Mods { shift: true, ..ctrl() }), None);
    assert_eq!(op(Key::Tab, Mods::default()), None);
    assert_eq!(op(Key::Escape, Mods::default()), None);
    assert_eq!(op(Key::Text("\u{8}".into()), Mods::default()), None);
    assert_eq!(op(Key::Text(String::new()), Mods::default()), None);
    assert_eq!(op(Key::Left, Mods { alt: true, ..Mods::default() }), None);
    assert_eq!(op(Key::Up, ctrl()), None);
    assert_eq!(op(Key::PageDown, ctrl()), None);
    // Text typed with Ctrl is a chord; with Ctrl and Alt (AltGr) it types.
    assert_eq!(op(Key::Text("v".into()), ctrl()), None);
    if !cfg!(target_os = "macos") {
        let altgr = Mods {
            alt: true,
            ..ctrl()
        };
        assert_eq!(op(Key::Text("@".into()), altgr), Some(EditOp::InsertText("@".into())));
    }
}

/// Ctrl+B turns bold off where the caret is in bold.
#[test]
fn a_mark_toggles_from_where_the_caret_is() {
    let bold = Marks { bold: true, ..marks() };
    assert_eq!(
        op_for(&key_event(Key::Command('b'), ctrl()), Some(&bold)),
        Some(EditOp::Mark { mark: Mark::Bold, on: false })
    );
    assert_eq!(
        op_for(&key_event(Key::Command('i'), ctrl()), Some(&bold)),
        Some(EditOp::Mark { mark: Mark::Italic, on: true })
    );
}

#[test]
fn a_press_picks_its_mode() {
    assert_eq!(hit_mode(1, false), HitMode::Place);
    assert_eq!(hit_mode(1, true), HitMode::Extend);
    assert_eq!(hit_mode(2, false), HitMode::Word);
    assert_eq!(hit_mode(3, false), HitMode::Paragraph);
    assert_eq!(hit_mode(4, true), HitMode::Paragraph);
}

/// A focused surface sends what is typed and moved; the editor's answers are not waited for.
#[test]
fn the_focused_surface_sends_ops() {
    let (mut rt, shared) = editor(true);
    focus_surface(&mut rt);
    plain(&mut rt, Key::Text("h".into()));
    plain(&mut rt, Key::Text("i".into()));
    plain(&mut rt, Key::Enter);
    press(&mut rt, Key::Left, shift());
    press(&mut rt, Key::Command('a'), ctrl());
    assert_eq!(
        ops(&shared),
        vec![
            EditOp::InsertText("h".into()),
            EditOp::InsertText("i".into()),
            EditOp::InsertParagraph,
            EditOp::Move { to: Motion::Left, extend: true },
            EditOp::SelectAll,
        ]
    );
}

/// Ctrl+B asks from the marks the editor answered last.
#[test]
fn bold_follows_the_answered_marks() {
    let (mut rt, shared) = editor(true);
    focus_surface(&mut rt);
    answer(&mut rt, &shared, edited(1, Some(at(0, 72.0, 96.0)), Vec::new(), Marks { bold: true, ..marks() }));
    press(&mut rt, Key::Command('b'), ctrl());
    assert_eq!(ops(&shared), vec![EditOp::Mark { mark: Mark::Bold, on: false }]);
}

/// A ribbon tool pressed with the pointer takes the keyboard with it; once it acted the keyboard is the page's again,
/// so what is typed next is typed (dc-R3, on a 150 % display and any other: after Text colour the keys went to the
/// tool, nothing was typed, and Space opened its menu again).
#[test]
fn a_ribbon_tool_gives_the_keyboard_back_to_the_page() {
    use beebeeb_office_protocol::office::Mark;
    let (mut rt, shared) = editor(true);
    focus_surface(&mut rt);
    let bold = rt
        .elements()
        .into_iter()
        .find(|e| !e.exiting && e.focusable && e.label.as_deref() == Some("Bold"))
        .expect("Bold is on the ribbon");
    let (x, y) = (bold.rect[0] + bold.rect[2] / 2.0, bold.rect[1] + bold.rect[3] / 2.0);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(0.0);
    rt.frame(0.0);
    assert!(
        ops(&shared).iter().any(|op| matches!(op, EditOp::Mark { mark: Mark::Bold, .. })),
        "the tool acted: {:?}",
        ops(&shared)
    );
    assert!(find(&rt, SURFACE).is_some_and(|e| e.focused), "the page has the keyboard again");
    // What is typed next reaches the document.
    plain(&mut rt, Key::Text("x".into()));
    assert!(
        ops(&shared).iter().any(|op| matches!(op, EditOp::InsertText(t) if t == "x")),
        "typed after the tool: {:?}",
        ops(&shared)
    );
}

/// With nothing focused the window passes the keys to the editor; with something else focused it does not.
#[test]
fn the_window_gives_the_editor_its_keys_when_nothing_has_focus() {
    let (mut rt, shared) = editor(true);
    plain(&mut rt, Key::Text("x".into()));
    assert_eq!(ops(&shared), vec![EditOp::InsertText("x".into())]);
    // Ctrl+Plus is still the viewer's zoom.
    press(&mut rt, Key::Command('+'), ctrl());
    assert_eq!(viewers::doc_mut(&mut rt.state).map(|d| d.view.zoom), Some(110));
    assert_eq!(ops(&shared).len(), 1);
}

/// A document that is only viewed has no surface, no caret and sends nothing.
#[test]
fn a_viewed_document_has_no_surface() {
    let (mut rt, shared) = editor(false);
    assert!(find(&rt, SURFACE).is_none());
    plain(&mut rt, Key::Text("x".into()));
    plain(&mut rt, Key::Enter);
    assert!(calls(&shared).is_empty());
    assert!(find(&rt, CARET).is_none());
}

/// An editor that stopped is no longer live: no surface, nothing sent.
#[test]
fn a_stopped_editor_takes_nothing() {
    let (mut rt, shared) = editor(true);
    shared.answers.borrow_mut().push_back(EditAnswer::Stopped);
    rt.update(|s, tx| {
        edit::poll(s, tx);
    });
    rt.frame(0.0);
    rt.frame(0.0);
    assert!(find(&rt, SURFACE).is_none());
    plain(&mut rt, Key::Text("x".into()));
    assert!(calls(&shared).is_empty());
}

/// A press places the caret (the point converted to the page's points); Shift extends; a double and a triple click
/// take a word and a paragraph.
#[test]
fn a_press_sends_a_hit() {
    let (mut rt, shared) = editor(true);
    let page = rect(&rt, "page");
    let (x, y) = (page[0] + 100.0, page[1] + 50.0);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(0.0);
    let calls = calls(&shared);
    let Some(Call::Hit(0, hx, hy, HitMode::Place)) = calls.first().cloned() else {
        panic!("a place on page 1: {calls:?}");
    };
    assert!((hx - 100.0 / K).abs() < 0.5 && (hy - 50.0 / K).abs() < 0.5, "{hx} {hy}");
}

/// The display's scale changes nothing of what a press means: at 150 % (144 dpi) a press on a point of the page is
/// that point, and the caret answered for it is drawn under the press, with the page a whole picture (90 %) and in
/// tiles (100 %: 100 x 144 = 14 400).
#[test]
fn a_press_is_the_same_point_of_the_page_at_150_per_cent_display_scaling() {
    for zoom in [90u16, 100] {
        let (mut rt, shared) = editor_at(true, zoom, 1.5);
        let k = K * f32::from(zoom) / 100.0;
        let page = rect(&rt, "page");
        let (x, y) = (page[0] + 180.0, page[1] + 240.0);
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(0.0);
        let calls = calls(&shared);
        let Some(Call::Hit(0, hx, hy, HitMode::Place)) = calls.first().cloned() else {
            panic!("a place on page 1 at {zoom} %: {calls:?}");
        };
        assert!(
            (hx - 180.0 / k).abs() < 0.5 && (hy - 240.0 / k).abs() < 0.5,
            "{zoom} %: {hx} {hy}"
        );
        // The editor answers a caret at that point: it is drawn where the press was.
        answer(&mut rt, &shared, edited(1, Some(at(0, hx, hy)), Vec::new(), marks()));
        let caret = rect(&rt, CARET);
        let page = rect(&rt, "page");
        assert!(
            (caret[0] - page[0] - 180.0).abs() < 0.7 && (caret[1] - page[1] - 240.0).abs() < 0.7,
            "{zoom} %: the caret {caret:?} on the page {page:?}"
        );
    }
}

#[test]
fn double_and_triple_clicks_take_a_word_and_a_paragraph() {
    let (mut rt, shared) = editor(true);
    let page = rect(&rt, "page");
    let (x, y) = (page[0] + 100.0, page[1] + 50.0);
    for _ in 0..3 {
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(0.0);
    }
    let modes: Vec<HitMode> = calls(&shared)
        .into_iter()
        .filter_map(|c| match c {
            Call::Hit(_, _, _, m) => Some(m),
            Call::Op(_) => None,
        })
        .collect();
    assert_eq!(modes, vec![HitMode::Place, HitMode::Word, HitMode::Paragraph]);
}

/// A drag extends from the press, once for each point of the page the pointer reaches; a move under a point asks
/// nothing; the press focuses the surface.
#[test]
fn a_drag_extends_the_selection() {
    let (mut rt, shared) = editor(true);
    let page = rect(&rt, "page");
    let (x, y) = (page[0] + 100.0, page[1] + 50.0);
    rt.pointer_down(x, y);
    rt.frame(0.0);
    rt.pointer_move(x + 0.2, y);
    rt.frame(0.0);
    rt.pointer_move(x + 40.0, y);
    rt.frame(0.0);
    rt.pointer_move(x + 40.2, y);
    rt.pointer_up(x + 40.2, y);
    rt.frame(0.0);
    let modes: Vec<HitMode> = calls(&shared)
        .into_iter()
        .filter_map(|c| match c {
            Call::Hit(_, _, _, m) => Some(m),
            Call::Op(_) => None,
        })
        .collect();
    assert_eq!(modes, vec![HitMode::Place, HitMode::Extend]);
    assert!(find(&rt, SURFACE).is_some_and(|e| e.focused), "the press focused the surface");
}

/// A press in the gap or past the last page lands on the nearer page, kept on it.
#[test]
fn a_press_off_the_pages_lands_on_a_page() {
    let (mut rt, shared) = editor(true);
    let page = rect(&rt, "page");
    // Left of the page and above it: the top-left corner of page 1.
    rt.pointer_down(page[0] + 1.0, page[1] + 1.0);
    rt.pointer_up(page[0] + 1.0, page[1] + 1.0);
    rt.frame(0.0);
    let Some(Call::Hit(0, x, y, _)) = calls(&shared).first().cloned() else {
        panic!("a hit on page 1");
    };
    assert!(x < 3.0 && y < 3.0, "{x} {y}");
}

/// The caret is drawn where the editor says, in the page's points over the page's box.
#[test]
fn the_caret_is_drawn_from_the_answer() {
    let (mut rt, shared) = editor(true);
    assert!(find(&rt, CARET).is_none(), "no caret before the editor says where it is");
    answer(&mut rt, &shared, edited(1, Some(at(0, 72.0, 96.0)), Vec::new(), marks()));
    let page = rect(&rt, "page");
    let caret = rect(&rt, CARET);
    assert!((caret[0] - (page[0] + 72.0 * K)).abs() < 0.6, "{caret:?} on {page:?}");
    assert!((caret[1] - (page[1] + 96.0 * K)).abs() < 0.6, "{caret:?} on {page:?}");
    assert!((caret[3] - 14.0 * K).abs() < 0.6);
    // A caret on a later page is drawn on that page.
    answer(&mut rt, &shared, edited(2, Some(at(1, 72.0, 96.0)), Vec::new(), marks()));
    let second = rect(&rt, "page-2");
    let caret = rect(&rt, CARET);
    assert!((caret[1] - (second[1] + 96.0 * K)).abs() < 0.6, "{caret:?} on {second:?}");
}

/// A selection is its rectangles, and a range has no caret.
#[test]
fn a_selection_is_drawn_from_the_answer() {
    let (mut rt, shared) = editor(true);
    let pieces = vec![at(0, 72.0, 96.0), at(0, 72.0, 112.0)];
    answer(&mut rt, &shared, edited(1, Some(at(0, 100.0, 112.0)), pieces, marks()));
    let page = rect(&rt, "page");
    for (i, y) in [96.0, 112.0].into_iter().enumerate() {
        let r = rect(&rt, &format!("sel-{i}"));
        assert!((r[0] - (page[0] + 72.0 * K)).abs() < 0.6 && (r[1] - (page[1] + y * K)).abs() < 0.6, "{r:?}");
    }
    assert!(find(&rt, "sel-2").is_none());
    assert!(find(&rt, CARET).is_none(), "a range selection has no caret");
    answer(&mut rt, &shared, edited(2, Some(at(0, 100.0, 112.0)), Vec::new(), marks()));
    assert!(find(&rt, "sel-0").is_none() && find(&rt, CARET).is_some());
}

/// The caret blinks while the editor has the keyboard and rests dimmed while it does not.
#[test]
fn the_caret_blinks_only_with_the_keyboard() {
    let (mut rt, shared) = editor(true);
    answer(&mut rt, &shared, edited(1, Some(at(0, 72.0, 96.0)), Vec::new(), marks()));
    let idle = find(&rt, CARET).expect("caret");
    assert!(idle.opacity < 1.0 && idle.loop_phase.is_none(), "an idle caret rests dimmed: {}", idle.opacity);
    focus_surface(&mut rt);
    rt.update(|s, tx| {
        edit::poll(s, tx);
    });
    rt.frame(0.0);
    rt.frame(0.0);
    assert!(viewers::ui(&mut rt.state).and_then(|u| u.office.edit.as_ref()).is_some_and(|e| e.has_keyboard));
    assert!(find(&rt, CARET).expect("caret").loop_phase.is_some(), "a caret with the keyboard blinks");
}

/// A caret the editor moved off screen is scrolled into view, and not before the editor has answered.
#[test]
fn the_caret_is_scrolled_into_view() {
    let (mut rt, shared) = editor(true);
    focus_surface(&mut rt);
    answer(&mut rt, &shared, edited(1, Some(at(0, 72.0, 96.0)), Vec::new(), marks()));
    // A move down: asked, and the editor answers with the caret on page 4.
    plain(&mut rt, Key::PageDown);
    assert!(viewers::doc_mut(&mut rt.state).is_some_and(|d| d.reveal.is_some()), "waiting for the editor's answer");
    answer(&mut rt, &shared, edited(2, Some(at(3, 72.0, 400.0)), Vec::new(), marks()));
    for _ in 0..4 {
        rt.frame(0.0);
    }
    let stage = rect(&rt, scroll::STAGE);
    let caret = rect(&rt, CARET);
    assert!(
        caret[1] >= stage[1] && caret[1] + caret[3] <= stage[1] + stage[3],
        "the caret {caret:?} is in the stage {stage:?}"
    );
    assert!(viewers::doc_mut(&mut rt.state).is_some_and(|d| d.reveal.is_none()), "no longer waiting");
}

/// A caret that is in view does not move the stage.
#[test]
fn a_visible_caret_does_not_scroll() {
    let (mut rt, shared) = editor(true);
    focus_surface(&mut rt);
    answer(&mut rt, &shared, edited(1, Some(at(0, 72.0, 96.0)), Vec::new(), marks()));
    let before = rect(&rt, "page")[1];
    plain(&mut rt, Key::Right);
    answer(&mut rt, &shared, edited(2, Some(at(0, 80.0, 96.0)), Vec::new(), marks()));
    for _ in 0..3 {
        rt.frame(0.0);
    }
    assert!((rect(&rt, "page")[1] - before).abs() < 0.6);
}

/// The wheel scrolls (the surface does not take it) and drops a reveal still waited for; Ctrl + wheel zooms.
#[test]
fn the_wheel_scrolls_and_ctrl_wheel_zooms() {
    let (mut rt, shared) = editor(true);
    answer(&mut rt, &shared, edited(1, Some(at(0, 72.0, 96.0)), Vec::new(), marks()));
    focus_surface(&mut rt);
    plain(&mut rt, Key::Down);
    assert!(viewers::doc_mut(&mut rt.state).is_some_and(|d| d.reveal.is_some()));
    let stage = rect(&rt, scroll::STAGE);
    let (x, y) = (stage[0] + stage[2] / 2.0, stage[1] + stage[3] / 2.0);
    let before = rect(&rt, "page")[1];
    rt.wheel_event(WheelEvent::new(0.0, -120.0).at(x, y).precise(true));
    for _ in 0..20 {
        rt.frame(1.0 / 60.0);
    }
    assert!(rect(&rt, "page")[1] < before, "the stage scrolled");
    assert!(viewers::doc_mut(&mut rt.state).is_some_and(|d| d.reveal.is_none()), "a scroll away is not taken back");
    rt.wheel_event(WheelEvent::new(0.0, 40.0).at(x, y).mods(ctrl()));
    rt.frame(0.0);
    assert_eq!(viewers::doc_mut(&mut rt.state).map(|d| d.view.zoom), Some(110));
    rt.wheel_event(WheelEvent::new(0.0, -40.0).at(x, y).mods(ctrl()));
    rt.frame(0.0);
    assert_eq!(viewers::doc_mut(&mut rt.state).map(|d| d.view.zoom), Some(100));
}

/// Every key and press of the surface says `instant`: the caret does not glide to its place.
#[test]
fn the_surface_is_a_focusable_text_area() {
    let (rt, _shared) = editor(true);
    let surface = find(&rt, SURFACE).expect("the surface");
    assert!(surface.focusable);
    assert_eq!(surface.label.as_deref(), Some("Document text"));
    assert!(SURFACE.starts_with(edit::FOCUS_PREFIX));
}

// ---------------------------------------------------------------------------
// The command search (dc-175, as RC-c for a deck): with the page's surface holding the keyboard, Ctrl+K and the
// ribbon's search draw it over the document, its field takes the keyboard, and the typed words are not sent as text.
// ---------------------------------------------------------------------------

fn focused_key(rt: &Runtime<State>) -> Option<String> {
    rt.elements()
        .into_iter()
        .find(|e| e.focused && !e.exiting)
        .and_then(|e| e.key)
}

fn palette_shows_and_takes_the_keys(rt: &mut Runtime<State>, shared: &Shared) {
    rt.frame(0.0);
    assert!(find(rt, "pal").is_some(), "the command search is drawn over the document");
    assert_eq!(focused_key(rt).as_deref(), Some("pal-input"), "and its field has the keyboard");
    plain(rt, Key::Text("b".into()));
    assert!(
        !ops(shared).iter().any(|op| matches!(op, EditOp::InsertText(_))),
        "typed words are the search's: {:?}",
        ops(shared)
    );
    plain(rt, Key::Escape);
    rt.frame(0.0);
    assert!(find(rt, "pal").is_none(), "Esc closed the search");
    assert!(viewers::ui(&mut rt.state).is_some(), "not the file");
}

#[test]
fn office_docs_palette_ctrl_k_from_the_page_shows_the_command_search() {
    let (mut rt, shared) = editor(true);
    focus_surface(&mut rt);
    press(&mut rt, Key::Command('k'), ctrl());
    palette_shows_and_takes_the_keys(&mut rt, &shared);
}

#[test]
fn office_docs_palette_the_search_button_shows_the_command_search() {
    let (mut rt, shared) = editor(true);
    focus_surface(&mut rt);
    let r = rect(&rt, "rib-find");
    let (x, y) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    palette_shows_and_takes_the_keys(&mut rt, &shared);
}
