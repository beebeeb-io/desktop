//! Tests of the slide surface: the pointer and keys of the stage, the filmstrip's menu, what is drawn on the slide and
//! the status line. A recording source stands in for the helper; its answers are fed to `EditUi` by hand.

use std::cell::RefCell;
use std::rc::Rc;

use beebeeb_office_protocol::office::{
    Dirty, EditKind, EditOp, Edited, HitMode, ListKind, Marks, Motion, PageRect, ParaStyle, RectPt, TextAlign,
};
use rune::{Env, Key, KeyEvent, Mods, Runtime};

use super::*;
use crate::frames::office::slides::{DeckView, menu_entries as entries, slides_fixture};
use crate::frames::viewers::source::{Body, EditAnswer, OpenError, OpenFile, ViewerSource};
use crate::ui::menu::Entry;
use crate::{init, view};

#[derive(Default)]
struct Log {
    edits: Vec<EditOp>,
    hits: Vec<(u32, f32, f32, HitMode)>,
}

struct EditSource {
    file: OpenFile,
    log: Rc<RefCell<Log>>,
}

impl ViewerSource for EditSource {
    fn open(&mut self, _file_id: &str) -> Result<OpenFile, OpenError> {
        Ok(self.file.clone())
    }
    fn poll(&mut self, _file_id: &str) -> Option<Body> {
        None
    }
    fn cancel(&mut self, _file_id: &str) {}
    fn retry(&mut self, _file_id: &str) -> Result<OpenFile, OpenError> {
        Ok(self.file.clone())
    }
    fn office_live(&self) -> bool {
        true
    }
    fn can_edit(&self, _file_id: &str) -> bool {
        true
    }
    fn edit(&mut self, _file_id: &str, op: EditOp) -> bool {
        self.log.borrow_mut().edits.push(op);
        true
    }
    fn hit(&mut self, _file_id: &str, page: u32, x_pt: f32, y_pt: f32, mode: HitMode) -> bool {
        self.log.borrow_mut().hits.push((page, x_pt, y_pt, mode));
        true
    }
    fn poll_edit(&mut self, _file_id: &str) -> Option<EditAnswer> {
        None
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

fn rect(page: u32, x: f32, y: f32, w: f32, h: f32) -> PageRect {
    PageRect {
        page,
        rect: RectPt { x, y, w, h },
    }
}

fn edited(revision: u64, caret: Option<PageRect>, selection: Vec<PageRect>) -> Edited {
    Edited {
        revision,
        caret,
        selection,
        can_undo: revision > 0,
        can_redo: false,
        dirty: Dirty::All,
        pages: None,
        cells: Vec::new(),
        slides: None,
        marks: marks(),
    }
}

/// The deck of three slides open in the live editor, on slide 1.
fn editing() -> (Runtime<State>, Rc<RefCell<Log>>) {
    let mut file = slides_fixture::file();
    file.body = Body::Deck(DeckBody::new(DeckView::of(3)));
    let log = Rc::new(RefCell::new(Log::default()));
    let mut state = init();
    let id = file.id.clone();
    let source = EditSource { file, log: log.clone() };
    assert!(viewers::open_from(&mut state, Box::new(source), &id));
    viewers::ui(&mut state).expect("a file").office.edit = Some(EditUi::new(EditKind::Pptx));
    let env = Env {
        system_dark: false,
        scale: 1.0,
    };
    let mut rt = Runtime::new(state, view, (1278.0, 798.0), env);
    rt.on_key = Some(crate::shell::host::on_key);
    rt.frame(0.0);
    rt.frame(0.0);
    (rt, log)
}

fn find(rt: &Runtime<State>, key: &str) -> Option<rune::ElementInfo> {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
}

fn rect_of(rt: &Runtime<State>, key: &str) -> [f32; 4] {
    find(rt, key).unwrap_or_else(|| panic!("{key} is drawn")).rect
}

/// The helper answered: the editor's state takes it.
fn answer(rt: &mut Runtime<State>, e: Edited) {
    rt.update(|s, _| {
        if let Some(edit) = viewers::ui(s).and_then(|ui| ui.office.edit.as_mut()) {
            edit.apply(&e);
        }
    });
    rt.frame(0.0);
    rt.frame(0.0);
}

fn click_at(rt: &mut Runtime<State>, x: f32, y: f32) {
    rt.pointer_move(x, y);
    rt.frame(0.0);
    rt.pointer_down(x, y);
    rt.frame(0.0);
    rt.pointer_up(x, y);
    rt.frame(0.0);
}

/// A click at the fraction (`fx`, `fy`) of the element `key`.
fn click_in(rt: &mut Runtime<State>, key: &str, fx: f32, fy: f32) {
    let r = rect_of(rt, key);
    click_at(rt, r[0] + r[2] * fx, r[1] + r[3] * fy);
}

fn key(rt: &mut Runtime<State>, key: Key, mods: Mods) {
    rt.key(KeyEvent { key, mods });
    rt.frame(0.0);
    rt.frame(0.0);
}

fn plain(rt: &mut Runtime<State>, k: Key) {
    key(rt, k, Mods::default());
}

fn ctrl() -> Mods {
    Mods {
        cmd: true,
        ctrl: true,
        ..Mods::default()
    }
}

fn shift() -> Mods {
    Mods {
        shift: true,
        ..Mods::default()
    }
}

fn deck(rt: &mut Runtime<State>) -> (u32, u32) {
    let d = viewers::deck_mut(&mut rt.state).expect("a deck");
    (d.view.current, d.view.count)
}

/// A shape's text is in hand: the caret is on slide 1, and the stage has the keyboard.
fn with_caret(rt: &mut Runtime<State>, log: &Rc<RefCell<Log>>) {
    answer(rt, edited(1, Some(rect(0, 100.0, 100.0, 0.0, 20.0)), Vec::new()));
    click_in(rt, "edit-slide", 0.5, 0.5);
    log.borrow_mut().edits.clear();
    log.borrow_mut().hits.clear();
}

#[test]
fn office_slides_edit_a_viewed_deck_has_no_editing_surface() {
    let mut state = init();
    assert!(viewers::open_deck(&mut state, slides_fixture::file()));
    let env = Env {
        system_dark: false,
        scale: 1.0,
    };
    let mut rt = Runtime::new(state, view, (1278.0, 798.0), env);
    rt.frame(0.0);
    assert!(find(&rt, "edit-slide").is_none());
    assert!(find(&rt, "edit-chrome").is_none());
    assert!(find(&rt, "vbar").is_some(), "the viewer's bar is still there");
}

#[test]
fn office_slides_edit_a_click_sends_select_shape_in_slide_points() {
    let (mut rt, log) = editing();
    click_in(&mut rt, "edit-slide", 0.25, 0.5);
    let edits = log.borrow().edits.clone();
    assert_eq!(edits.len(), 1, "{edits:?}");
    match &edits[0] {
        EditOp::SelectShape { slide, x_pt, y_pt } => {
            assert_eq!(*slide, 0, "slide numbers are 0-based");
            // The slide is 960 x 540 points on the stage: a quarter across, halfway down.
            assert!((x_pt - 240.0).abs() < 1.0, "x {x_pt}");
            assert!((y_pt - 270.0).abs() < 1.0, "y {y_pt}");
        }
        other => panic!("{other:?}"),
    }
    assert!(log.borrow().hits.is_empty());
}

#[test]
fn office_slides_edit_a_click_in_text_in_hand_places_the_caret() {
    let (mut rt, log) = editing();
    answer(&mut rt, edited(1, Some(rect(0, 100.0, 100.0, 0.0, 20.0)), Vec::new()));
    click_in(&mut rt, "edit-slide", 0.5, 0.5);
    let hits = log.borrow().hits.clone();
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!((hits[0].0, hits[0].3), (0, HitMode::Place));
    assert!((hits[0].1 - 480.0).abs() < 1.0 && (hits[0].2 - 270.0).abs() < 1.0);
    assert!(
        log.borrow().edits.is_empty(),
        "no SelectShape while the text is in hand"
    );
}

#[test]
fn office_slides_edit_typing_goes_to_the_text() {
    let (mut rt, log) = editing();
    with_caret(&mut rt, &log);
    plain(&mut rt, Key::Text("a".into()));
    plain(&mut rt, Key::Backspace);
    plain(&mut rt, Key::Delete);
    plain(&mut rt, Key::Enter);
    key(&mut rt, Key::Enter, shift());
    plain(&mut rt, Key::Left);
    key(&mut rt, Key::Right, shift());
    key(&mut rt, Key::Left, ctrl());
    key(&mut rt, Key::Backspace, ctrl());
    key(&mut rt, Key::Command('a'), ctrl());
    assert_eq!(
        log.borrow().edits,
        vec![
            EditOp::InsertText("a".into()),
            EditOp::Delete {
                back: true,
                word: false
            },
            EditOp::Delete {
                back: false,
                word: false
            },
            EditOp::InsertParagraph,
            EditOp::InsertLineBreak,
            EditOp::Move {
                to: Motion::Left,
                extend: false
            },
            EditOp::Move {
                to: Motion::Right,
                extend: true
            },
            EditOp::Move {
                to: Motion::WordLeft,
                extend: false
            },
            EditOp::Delete { back: true, word: true },
            EditOp::SelectAll,
        ]
    );
}

#[test]
fn office_slides_edit_undo_redo_and_marks() {
    let (mut rt, log) = editing();
    with_caret(&mut rt, &log);
    key(&mut rt, Key::Command('z'), ctrl());
    key(&mut rt, Key::Command('y'), ctrl());
    key(&mut rt, Key::Command('z'), Mods { shift: true, ..ctrl() });
    key(&mut rt, Key::Command('b'), ctrl());
    let edits = log.borrow().edits.clone();
    assert_eq!(edits[..3], [EditOp::Undo, EditOp::Redo, EditOp::Redo]);
    assert!(matches!(
        edits[3],
        EditOp::Mark {
            mark: beebeeb_office_protocol::office::Mark::Bold,
            on: true
        }
    ));
}

#[test]
fn office_slides_edit_without_text_in_hand_typing_sends_nothing_and_arrows_step() {
    let (mut rt, log) = editing();
    click_in(&mut rt, "edit-slide", 0.5, 0.5);
    log.borrow_mut().edits.clear();
    plain(&mut rt, Key::Text("a".into()));
    plain(&mut rt, Key::Backspace);
    assert!(log.borrow().edits.is_empty(), "{:?}", log.borrow().edits);
    assert_eq!(deck(&mut rt), (1, 3));
    plain(&mut rt, Key::Right);
    assert_eq!(deck(&mut rt), (2, 3), "the arrow steps to the next slide");
    assert_eq!(
        log.borrow().edits,
        vec![EditOp::ClearShape],
        "a shape selected on the slide left is let go"
    );
    plain(&mut rt, Key::Left);
    assert_eq!(deck(&mut rt), (1, 3));
}

#[test]
fn office_slides_edit_escape_clears_the_shape_only_when_one_is_selected() {
    let (mut rt, log) = editing();
    click_in(&mut rt, "edit-slide", 0.5, 0.5);
    log.borrow_mut().edits.clear();
    answer(&mut rt, edited(1, None, vec![rect(0, 80.0, 60.0, 400.0, 120.0)]));
    plain(&mut rt, Key::Escape);
    assert_eq!(log.borrow().edits, vec![EditOp::ClearShape]);
    // Nothing selected: Escape is the window's (it is not taken here).
    let (mut rt, log) = editing();
    click_in(&mut rt, "edit-slide", 0.5, 0.5);
    log.borrow_mut().edits.clear();
    plain(&mut rt, Key::Escape);
    assert!(log.borrow().edits.is_empty(), "{:?}", log.borrow().edits);
}

#[test]
fn office_slides_edit_present_keys_are_not_taken() {
    let (mut rt, log) = editing();
    with_caret(&mut rt, &log);
    key(&mut rt, Key::Enter, ctrl());
    assert!(
        log.borrow().edits.is_empty(),
        concat!(beebeeb_engine::primary_key!(), "Enter is Present's, not a paragraph")
    );
    let d = viewers::deck_mut(&mut rt.state).expect("a deck");
    assert!(d.presenting);
}

#[test]
fn office_slides_edit_ctrl_m_adds_a_slide_and_shows_it() {
    let (mut rt, log) = editing();
    click_in(&mut rt, "edit-slide", 0.5, 0.5);
    log.borrow_mut().edits.clear();
    key(&mut rt, Key::Command('m'), ctrl());
    assert_eq!(log.borrow().edits, vec![EditOp::NewSlide { layout: None }]);
    assert_eq!(deck(&mut rt), (2, 4));
    assert!(find(&rt, "fs-3").is_some(), "the strip lists the new slide");
    assert_eq!(find(&rt, "st-0").and_then(|e| e.text).as_deref(), Some("Slide 2 of 4"));
}

#[test]
fn office_slides_edit_the_viewer_keeps_its_keys() {
    let mut state = init();
    assert!(viewers::open_deck(&mut state, slides_fixture::file()));
    let env = Env {
        system_dark: false,
        scale: 1.0,
    };
    let mut rt = Runtime::new(state, view, (1278.0, 798.0), env);
    rt.on_key = Some(crate::shell::host::on_key);
    rt.frame(0.0);
    key(&mut rt, Key::Command('m'), ctrl());
    assert_eq!(deck(&mut rt).1, 3, "no editor: no new slide");
}

#[test]
fn office_slides_edit_the_caret_and_the_selection_are_drawn_on_the_slide() {
    let (mut rt, _log) = editing();
    answer(
        &mut rt,
        edited(
            1,
            Some(rect(0, 480.0, 270.0, 0.0, 27.0)),
            vec![rect(0, 96.0, 54.0, 192.0, 27.0), rect(1, 10.0, 10.0, 50.0, 10.0)],
        ),
    );
    let slide = rect_of(&rt, "edit-slide");
    // 960 x 540 points on 640 x 360 points: two thirds.
    let caret = rect_of(&rt, "edit-caret");
    assert!((caret[0] - (slide[0] + 320.0)).abs() < 0.6, "{caret:?} on {slide:?}");
    assert!((caret[1] - (slide[1] + 180.0)).abs() < 0.6);
    assert!((caret[3] - 18.0).abs() < 0.6, "the caret is the line's height");
    let sel = rect_of(&rt, "edit-sel-0");
    assert!(
        (sel[0] - (slide[0] + 64.0)).abs() < 0.6 && (sel[2] - 128.0).abs() < 0.6,
        "{sel:?}"
    );
    assert!(
        find(&rt, "edit-sel-1").is_none(),
        "a rectangle of another slide is not drawn"
    );
    assert!(
        find(&rt, "edit-shape-0").is_none(),
        "no outline while the text is in hand"
    );
}

#[test]
fn office_slides_edit_a_selected_shape_has_an_outline_and_eight_handles() {
    let (mut rt, _log) = editing();
    answer(&mut rt, edited(1, None, vec![rect(0, 96.0, 54.0, 480.0, 90.0)]));
    let slide = rect_of(&rt, "edit-slide");
    let outline = rect_of(&rt, "edit-shape-0");
    assert!((outline[0] - (slide[0] + 64.0)).abs() < 0.6 && (outline[2] - 320.0).abs() < 0.6);
    for h in 0..8 {
        assert!(find(&rt, &format!("edit-shape-0-h{h}")).is_some(), "handle {h}");
    }
    let (nw, se) = (rect_of(&rt, "edit-shape-0-h0"), rect_of(&rt, "edit-shape-0-h4"));
    assert!(nw[0] < se[0] && nw[1] < se[1]);
    assert!(find(&rt, "edit-caret").is_none());
    // Another slide on the stage: the outline is not drawn.
    plain(&mut rt, Key::Right);
    assert!(find(&rt, "edit-shape-0").is_none());
}

#[test]
fn office_slides_edit_status_line_says_the_slide_and_how_it_is_kept() {
    let (mut rt, _log) = editing();
    assert_eq!(find(&rt, "st-0").and_then(|e| e.text).as_deref(), Some("Slide 1 of 3"));
    assert_eq!(find(&rt, "st-1").and_then(|e| e.text).as_deref(), Some("Encrypted"));
    assert_eq!(
        find(&rt, "st-3").and_then(|e| e.text).as_deref(),
        Some("saved as version 5"),
        "the fixture's file is version 5"
    );
    assert!(
        find(&rt, "st-6").is_none() && find(&rt, "st-5").is_some(),
        "About ends the line"
    );
    answer(&mut rt, edited(3, None, Vec::new()));
    assert_eq!(
        find(&rt, "st-3").and_then(|e| e.text).as_deref(),
        Some("unsaved changes")
    );
    let texts: Vec<String> = rt
        .elements()
        .into_iter()
        .filter(|e| e.key.as_deref() == Some("status-bar"))
        .filter_map(|e| e.label)
        .collect();
    assert_eq!(texts.len(), 1, "one status line, not two");
}

#[test]
fn office_slides_edit_right_click_opens_the_slide_menu() {
    let (mut rt, log) = editing();
    let r = rect_of(&rt, "fs-1");
    rt.secondary_down(r[0] + 20.0, r[1] + 20.0);
    rt.frame(0.0);
    rt.secondary_up(r[0] + 20.0, r[1] + 20.0);
    rt.frame(0.0);
    rt.frame(0.0);
    assert_eq!(deck(&mut rt).0, 2, "the stage shows the slide that was clicked");
    assert!(find(&rt, "slide-menu").is_some(), "the menu is open");
    let words: Vec<String> = (0..4)
        .map(|i| {
            find(&rt, &format!("slide-menu-item-{i}"))
                .and_then(|e| e.label)
                .unwrap_or_default()
        })
        .collect();
    assert!(words[0].contains("Duplicate slide"), "{words:?}");
    assert!(words[1].contains("Delete slide"));
    assert!(words[2].contains("Move slide up"));
    assert!(words[3].contains("Move slide down"));
    log.borrow_mut().edits.clear();
    // Duplicate slide.
    let item = rect_of(&rt, "slide-menu-item-0");
    click_at(&mut rt, item[0] + item[2] / 2.0, item[1] + item[3] / 2.0);
    assert_eq!(log.borrow().edits, vec![EditOp::DuplicateSlide(1)]);
    assert_eq!(deck(&mut rt), (3, 4), "the copy follows its slide and shows");
    assert!(find(&rt, "slide-menu").is_none(), "the menu closed");
}

#[test]
fn office_slides_edit_menu_delete_and_moves() {
    let (mut rt, log) = editing();
    let open = |rt: &mut Runtime<State>, key: &str| {
        let r = rect_of(rt, key);
        rt.secondary_down(r[0] + 20.0, r[1] + 20.0);
        rt.frame(0.0);
        rt.secondary_up(r[0] + 20.0, r[1] + 20.0);
        rt.frame(0.0);
        rt.frame(0.0);
    };
    let pick = |rt: &mut Runtime<State>, i: usize| {
        let item = rect_of(rt, &format!("slide-menu-item-{i}"));
        click_at(rt, item[0] + item[2] / 2.0, item[1] + item[3] / 2.0);
    };
    open(&mut rt, "fs-1");
    pick(&mut rt, 2);
    assert_eq!(log.borrow().edits.last(), Some(&EditOp::MoveSlide { from: 1, to: 0 }));
    assert_eq!(deck(&mut rt), (1, 3), "the stage follows the moved slide");
    open(&mut rt, "fs-2");
    pick(&mut rt, 1);
    assert_eq!(log.borrow().edits.last(), Some(&EditOp::DeleteSlide(2)));
    assert_eq!(deck(&mut rt).1, 2);
    assert!(find(&rt, "fs-2").is_none(), "the strip lists two slides");
}

#[test]
fn office_slides_edit_menu_entries_dim_at_the_ends() {
    let off = |list: &[Entry], id: &str| list.iter().any(|e| e.id() == Some(id) && e.is_off());
    let one = entries(1, 1);
    assert!(off(&one, "delete") && off(&one, "up") && off(&one, "down"));
    assert!(!off(&one, "duplicate"));
    let first = entries(1, 3);
    assert!(off(&first, "up") && !off(&first, "down") && !off(&first, "delete"));
    let last = entries(3, 3);
    assert!(off(&last, "down") && !off(&last, "up"));
}

#[test]
fn office_slides_edit_deleting_the_last_slide_is_not_sent() {
    let (mut rt, log) = editing();
    rt.update(|s, _| {
        if let Some(d) = viewers::deck_mut(s) {
            d.set_count(1);
        }
    });
    rt.frame(0.0);
    delete_slide_via_state(&mut rt, 1);
    assert!(log.borrow().edits.is_empty(), "{:?}", log.borrow().edits);
}

fn delete_slide_via_state(rt: &mut Runtime<State>, n: u32) {
    rt.update(|s, _| delete_slide(s, n));
    rt.frame(0.0);
}

#[test]
fn office_slides_edit_set_count_keeps_the_stage_and_the_strip_inside_the_deck() {
    let mut d = DeckBody::new(DeckView::of(10));
    d.go(10);
    d.set_count(4);
    assert_eq!(d.view.count, 4);
    assert_eq!(d.view.slides.len(), 4);
    assert_eq!(d.view.current, 4, "clamped to the last slide");
    assert_eq!(d.strip_y, 0, "four slides fit the strip");
    d.set_count(7);
    assert_eq!(d.view.slides.last().map(|t| t.n), Some(7));
    d.set_count(0);
    assert_eq!((d.view.count, d.view.current), (1, 1), "a deck keeps one slide");
}

#[test]
fn office_slides_edit_dragging_in_text_extends_the_selection() {
    let (mut rt, log) = editing();
    answer(&mut rt, edited(1, Some(rect(0, 100.0, 100.0, 0.0, 20.0)), Vec::new()));
    let r = rect_of(&rt, "edit-slide");
    let (x0, y0) = (r[0] + r[2] * 0.3, r[1] + r[3] * 0.5);
    rt.pointer_move(x0, y0);
    rt.frame(0.0);
    rt.pointer_down(x0, y0);
    rt.frame(0.0);
    for step in 1..=3 {
        let x = x0 + step as f32 * 30.0;
        rt.pointer_move(x, y0);
        rt.frame(0.0);
    }
    rt.pointer_up(x0 + 90.0, y0);
    rt.frame(0.0);
    let hits = log.borrow().hits.clone();
    assert_eq!(hits[0].3, HitMode::Place);
    assert!(hits.len() >= 2, "{hits:?}");
    assert!(hits[1..].iter().all(|h| h.3 == HitMode::Extend), "{hits:?}");
}

fn run_cmd(rt: &mut Runtime<State>, cmd: crate::frames::office::edit::palette::SlideCmd) {
    rt.update(|s, _| run_command(s, cmd));
    rt.frame(0.0);
}

#[test]
fn office_slides_edit_the_stage_is_what_the_palette_reads() {
    let (mut rt, _log) = editing();
    rt.update(|s, _| viewers::deck_mut(s).unwrap().go(2));
    let st = stage(&rt.state).expect("a live deck has a current slide");
    assert_eq!((st.current, st.count), (2, 3));
}

#[test]
fn office_slides_edit_palette_duplicate_and_delete_act_on_the_current_slide() {
    use crate::frames::office::edit::palette::SlideCmd;
    let (mut rt, log) = editing();
    rt.update(|s, _| viewers::deck_mut(s).unwrap().go(2));
    click_in(&mut rt, "edit-slide", 0.5, 0.5);
    run_cmd(&mut rt, SlideCmd::Duplicate);
    assert_eq!(log.borrow().edits.last(), Some(&EditOp::DuplicateSlide(1)));
    assert_eq!(deck(&mut rt), (3, 4), "the copy shows");
    run_cmd(&mut rt, SlideCmd::Delete);
    assert_eq!(log.borrow().edits.last(), Some(&EditOp::DeleteSlide(2)));
    assert_eq!(deck(&mut rt), (3, 3));
    // Undo goes through the existing history.
    key(&mut rt, Key::Command('z'), ctrl());
    assert_eq!(log.borrow().edits.last(), Some(&EditOp::Undo));
}

#[test]
fn office_slides_edit_keys_duplicate_and_delete_the_current_slide() {
    let (mut rt, log) = editing();
    rt.update(|s, _| viewers::deck_mut(s).unwrap().go(2));
    click_in(&mut rt, "edit-slide", 0.5, 0.5);
    key(&mut rt, Key::Command('d'), ctrl());
    assert_eq!(log.borrow().edits.last(), Some(&EditOp::DuplicateSlide(1)));
    assert_eq!(deck(&mut rt), (3, 4));
    plain(&mut rt, Key::Delete);
    assert_eq!(log.borrow().edits.last(), Some(&EditOp::DeleteSlide(2)));
    assert_eq!(deck(&mut rt), (3, 3));
    // With text in hand Delete edits the text, not the deck.
    with_caret(&mut rt, &log);
    plain(&mut rt, Key::Delete);
    assert!(matches!(log.borrow().edits.last(), Some(EditOp::Delete { .. })));
    assert_eq!(deck(&mut rt), (3, 3));
}

// ---------------------------------------------------------------------------
// The keys as the window gets them (drive-complete RC1, check 11): the keyboard is where a click left it. A click on
// a thumbnail leaves it on the filmstrip row, not on the stage; every key below goes through the runtime's own
// routing (the focused element, its ancestors, the window's `on_key`), as a key pressed on the installed build does.
// ---------------------------------------------------------------------------

/// [`editing`] with the shell's poll, which takes the viewer's intents (a palette command is one) and moves the
/// keyboard where a press asked for it, as in the running app.
fn editing_polled() -> (Runtime<State>, Rc<RefCell<Log>>) {
    crate::shell::intents::poll_drains_for_this_test();
    let (mut rt, log) = editing();
    rt.poll = Some(crate::shell::host::poll);
    rt.frame(0.0);
    (rt, log)
}

fn focused(rt: &Runtime<State>) -> Option<String> {
    rt.elements()
        .into_iter()
        .find(|e| e.focused && !e.exiting)
        .and_then(|e| e.key)
}

/// A click on thumbnail `n` (1-based): the stage shows the slide and the keyboard is on the filmstrip row. What the
/// click itself sent (a shape let go on the slide it left) is dropped from the log.
fn click_thumbnail(rt: &mut Runtime<State>, log: &Rc<RefCell<Log>>, n: u32) {
    let row = format!("fs-{}", n - 1);
    click_in(rt, &format!("{row}-slide"), 0.5, 0.5);
    assert_eq!(
        focused(rt).as_deref(),
        Some(row.as_str()),
        "a click on a thumbnail leaves the keyboard on its row"
    );
    assert_eq!(deck(rt).0, n);
    log.borrow_mut().edits.clear();
}

fn type_words(rt: &mut Runtime<State>, words: &str) {
    for c in words.chars() {
        plain(rt, Key::Text(c.to_string()));
    }
}

fn palette_query(rt: &mut Runtime<State>) -> Option<String> {
    use crate::frames::viewers::office_over::Over;
    match viewers::ui(&mut rt.state).and_then(|ui| ui.office.over.as_ref()) {
        Some(Over::Palette(palette)) => Some(palette.query.text.clone()),
        _ => None,
    }
}

#[test]
fn office_slides_window_keys_ctrl_k_from_a_thumbnail_shows_the_palette_and_duplicate_runs_and_undoes() {
    let (mut rt, log) = editing_polled();
    click_thumbnail(&mut rt, &log, 2);
    key(&mut rt, Key::Command('k'), ctrl());
    assert!(
        find(&rt, "pal").is_some(),
        concat!(beebeeb_engine::primary_key!(), "K draws the command search over the deck")
    );
    assert_eq!(
        focused(&rt).as_deref(),
        Some("pal-input"),
        "and its field has the keyboard"
    );
    type_words(&mut rt, "Duplicate slide");
    assert_eq!(
        palette_query(&mut rt).as_deref(),
        Some("Duplicate slide"),
        "the typed words land in the search"
    );
    assert!(
        log.borrow().edits.is_empty(),
        "typing runs nothing: {:?}",
        log.borrow().edits
    );
    plain(&mut rt, Key::Enter);
    assert_eq!(log.borrow().edits.last(), Some(&EditOp::DuplicateSlide(1)));
    assert_eq!(deck(&mut rt), (3, 4), "the copy shows");
    assert!(
        find(&rt, "pal").is_none() && palette_query(&mut rt).is_none(),
        "the search closed"
    );
    key(&mut rt, Key::Command('z'), ctrl());
    assert_eq!(
        log.borrow().edits.last(),
        Some(&EditOp::Undo),
        concat!(beebeeb_engine::primary_key!(), "Z after the palette undoes")
    );
}

#[test]
fn office_slides_window_keys_the_search_button_shows_the_palette_and_delete_runs_and_undoes() {
    let (mut rt, log) = editing_polled();
    click_thumbnail(&mut rt, &log, 2);
    click_in(&mut rt, "rib-find", 0.3, 0.5);
    assert!(
        find(&rt, "pal").is_some(),
        "the search button draws the command search over the deck"
    );
    assert_eq!(focused(&rt).as_deref(), Some("pal-input"));
    type_words(&mut rt, "Delete slide");
    plain(&mut rt, Key::Enter);
    assert_eq!(log.borrow().edits.as_slice(), &[EditOp::DeleteSlide(1)]);
    assert_eq!(deck(&mut rt), (2, 2));
    key(&mut rt, Key::Command('z'), ctrl());
    assert_eq!(log.borrow().edits.last(), Some(&EditOp::Undo));
    // Esc closes a search that ran nothing, and the deck's keys are the deck's again.
    key(&mut rt, Key::Command('k'), ctrl());
    assert!(find(&rt, "pal").is_some());
    plain(&mut rt, Key::Escape);
    assert!(find(&rt, "pal").is_none() && palette_query(&mut rt).is_none());
    assert!(
        viewers::ui(&mut rt.state).is_some(),
        "Esc closed the search, not the file"
    );
}

#[test]
fn office_slides_window_keys_ctrl_d_and_ctrl_z_from_a_thumbnail() {
    let (mut rt, log) = editing_polled();
    click_thumbnail(&mut rt, &log, 1);
    key(&mut rt, Key::Command('d'), ctrl());
    assert_eq!(
        log.borrow().edits.as_slice(),
        &[EditOp::DuplicateSlide(0)],
        concat!(beebeeb_engine::primary_key!(), "D on a filmstrip row")
    );
    assert_eq!(deck(&mut rt), (2, 4));
    key(&mut rt, Key::Command('z'), ctrl());
    assert_eq!(
        log.borrow().edits.last(),
        Some(&EditOp::Undo),
        concat!(beebeeb_engine::primary_key!(), "Z on a filmstrip row")
    );
    key(&mut rt, Key::Command('y'), ctrl());
    assert_eq!(log.borrow().edits.last(), Some(&EditOp::Redo));
    key(&mut rt, Key::Command('z'), Mods { shift: true, ..ctrl() });
    assert_eq!(log.borrow().edits.last(), Some(&EditOp::Redo));
    // Delete on the row, then Ctrl+Z: the case the installed build left without an undo.
    click_thumbnail(&mut rt, &log, 2);
    plain(&mut rt, Key::Delete);
    key(&mut rt, Key::Command('z'), ctrl());
    assert_eq!(log.borrow().edits.as_slice(), &[EditOp::DeleteSlide(1), EditOp::Undo]);
}

#[test]
fn office_slides_window_keys_work_with_the_keyboard_nowhere_and_on_a_ribbon_button() {
    let (mut rt, log) = editing_polled();
    assert_eq!(focused(&rt), None, "nothing has the keyboard in a deck just opened");
    key(&mut rt, Key::Command('d'), ctrl());
    key(&mut rt, Key::Command('z'), ctrl());
    key(&mut rt, Key::Command('m'), ctrl());
    assert_eq!(
        log.borrow().edits.as_slice(),
        &[
            EditOp::DuplicateSlide(0),
            EditOp::Undo,
            EditOp::NewSlide { layout: None }
        ]
    );
    // The stage still takes each chord once: its own handler, not that and the window's.
    click_in(&mut rt, "edit-slide", 0.5, 0.5);
    log.borrow_mut().edits.clear();
    key(&mut rt, Key::Command('z'), ctrl());
    key(&mut rt, Key::Command('d'), ctrl());
    let (current, _) = deck(&mut rt);
    assert_eq!(
        log.borrow().edits.as_slice(),
        &[EditOp::Undo, EditOp::DuplicateSlide(current - 2)]
    );
}

#[test]
fn office_slides_window_keys_are_not_the_decks_under_the_palette_or_with_text_in_hand() {
    let (mut rt, log) = editing_polled();
    click_thumbnail(&mut rt, &log, 1);
    // Under the command search: its field's keys, never the deck's. The arrows move its mark, not the stage.
    key(&mut rt, Key::Command('k'), ctrl());
    key(&mut rt, Key::Command('d'), ctrl());
    key(&mut rt, Key::Command('m'), ctrl());
    plain(&mut rt, Key::Down);
    assert!(log.borrow().edits.is_empty(), "{:?}", log.borrow().edits);
    assert_eq!(deck(&mut rt), (1, 3));
    assert!(find(&rt, "pal").is_some(), "the search is still open");
    plain(&mut rt, Key::Escape);
    assert!(find(&rt, "pal").is_none());
    // A shape's text in hand: Ctrl+D copies no slide.
    with_caret(&mut rt, &log);
    key(&mut rt, Key::Command('d'), ctrl());
    assert!(log.borrow().edits.is_empty(), "{:?}", log.borrow().edits);
    assert_eq!(deck(&mut rt), (1, 3));
}

#[test]
fn office_slides_window_keys_present_opens_no_command_search_and_changes_no_deck() {
    let (mut rt, log) = editing_polled();
    rt.update(|s, _| viewers::deck_mut(s).expect("a deck").present());
    rt.frame(0.0);
    key(&mut rt, Key::Command('k'), ctrl());
    key(&mut rt, Key::Command('d'), ctrl());
    key(&mut rt, Key::Command('z'), ctrl());
    assert!(palette_query(&mut rt).is_none(), "no search opens behind the slide");
    assert!(log.borrow().edits.is_empty(), "{:?}", log.borrow().edits);
    assert_eq!(deck(&mut rt), (1, 3));
}

#[test]
fn office_slides_window_keys_a_viewed_deck_takes_none() {
    let mut file = slides_fixture::file();
    file.body = Body::Deck(DeckBody::new(DeckView::of(3)));
    let log = Rc::new(RefCell::new(Log::default()));
    let mut state = init();
    let id = file.id.clone();
    assert!(viewers::open_from(
        &mut state,
        Box::new(EditSource { file, log: log.clone() }),
        &id
    ));
    let env = Env {
        system_dark: false,
        scale: 1.0,
    };
    let mut rt = Runtime::new(state, view, (1278.0, 798.0), env);
    rt.on_key = Some(crate::shell::host::on_key);
    rt.frame(0.0);
    rt.frame(0.0);
    key(&mut rt, Key::Command('d'), ctrl());
    key(&mut rt, Key::Command('z'), ctrl());
    assert!(log.borrow().edits.is_empty() && deck(&mut rt) == (1, 3));
}
