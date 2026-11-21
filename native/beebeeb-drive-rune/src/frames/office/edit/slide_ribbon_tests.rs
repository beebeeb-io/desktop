//! Tests of the slide ribbon: Home's tools send their edits, Design lists the file's layouts and applies one.

use std::cell::RefCell;
use std::rc::Rc;

use beebeeb_office_protocol::office::{EditKind, EditOp};
use rune::{Env, Runtime};

use super::*;
use crate::frames::office::edit::EditUi;
use crate::frames::office::slides::{DeckBody, DeckView, slides_fixture};
use crate::frames::viewers::source::{Body, EditAnswer, OpenError, OpenFile, ViewerSource};
use crate::{init, view};

#[derive(Default)]
struct Log {
    edits: Vec<EditOp>,
    asked_layouts: u32,
}

struct Source {
    file: OpenFile,
    log: Rc<RefCell<Log>>,
}

impl ViewerSource for Source {
    fn open(&mut self, _: &str) -> Result<OpenFile, OpenError> {
        Ok(self.file.clone())
    }
    fn poll(&mut self, _: &str) -> Option<Body> {
        None
    }
    fn cancel(&mut self, _: &str) {}
    fn retry(&mut self, _: &str) -> Result<OpenFile, OpenError> {
        Ok(self.file.clone())
    }
    fn office_live(&self) -> bool {
        true
    }
    fn can_edit(&self, _: &str) -> bool {
        true
    }
    fn edit(&mut self, _: &str, op: EditOp) -> bool {
        self.log.borrow_mut().edits.push(op);
        true
    }
    fn layouts(&mut self, _: &str) -> bool {
        self.log.borrow_mut().asked_layouts += 1;
        true
    }
    fn poll_edit(&mut self, _: &str) -> Option<EditAnswer> {
        None
    }
}

fn editing(tab: Tab, layouts: bool) -> (Runtime<State>, Rc<RefCell<Log>>) {
    let mut file = slides_fixture::file();
    file.body = Body::Deck(DeckBody::new(DeckView::of(3)));
    let log = Rc::new(RefCell::new(Log::default()));
    let mut state = init();
    let id = file.id.clone();
    assert!(viewers::open_from(
        &mut state,
        Box::new(Source { file, log: log.clone() }),
        &id
    ));
    let ui = viewers::ui(&mut state).expect("a file");
    let mut edit = EditUi::new(EditKind::Pptx);
    if layouts {
        edit.layouts = NEW_DECK_LAYOUTS.iter().map(|n| (*n).to_string()).collect();
    }
    ui.office.edit = Some(edit);
    ui.office.tab = tab;
    let env = Env {
        system_dark: false,
        scale: 1.0,
    };
    let mut rt = Runtime::new(state, view, (1278.0, 798.0), env);
    rt.frame(0.0);
    rt.frame(0.0);
    APPLIED.with(|a| a.set(None));
    (rt, log)
}

fn by_label(rt: &Runtime<State>, label: &str) -> Option<rune::ElementInfo> {
    rt.elements().into_iter().find(|e| {
        e.label.as_deref() == Some(label) && !e.exiting && e.key.as_deref().is_some_and(|k| k.starts_with("r"))
    })
}

fn press(rt: &mut Runtime<State>, label: &str) {
    let r = by_label(rt, label).unwrap_or_else(|| panic!("{label} is drawn")).rect;
    let (x, y) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
    rt.pointer_move(x, y);
    rt.frame(0.0);
    rt.pointer_down(x, y);
    rt.frame(0.0);
    rt.pointer_up(x, y);
    rt.frame(0.0);
    rt.frame(0.0);
}

fn has(rt: &Runtime<State>, key: &str) -> bool {
    rt.elements()
        .iter()
        .any(|e| e.key.as_deref() == Some(key) && !e.exiting)
}

fn deck(rt: &mut Runtime<State>) -> (u32, u32) {
    let d = viewers::deck_mut(&mut rt.state).expect("a deck");
    (d.view.current, d.view.count)
}

#[test]
fn office_slides_ribbon_home_tools_send_their_edits() {
    let (mut rt, log) = editing(Tab::Home, false);
    press(&mut rt, "Bold");
    press(&mut rt, "Italic");
    press(&mut rt, "Align centre");
    assert_eq!(log.borrow().edits.len(), 3);
    assert!(matches!(log.borrow().edits[0], EditOp::Mark { on: true, .. }));
    assert_eq!(log.borrow().edits[2], EditOp::Align(TextAlign::Center));
    // Undo is off until the helper says there is something to undo.
    press(&mut rt, "Undo");
    assert_eq!(log.borrow().edits.len(), 3);
}

#[test]
fn office_slides_ribbon_slide_tools_keep_the_deck_in_step() {
    let (mut rt, log) = editing(Tab::Home, false);
    press(&mut rt, "New slide");
    assert_eq!(log.borrow().edits.last(), Some(&EditOp::NewSlide { layout: None }));
    assert_eq!(deck(&mut rt), (2, 4));
    press(&mut rt, "Duplicate slide");
    assert_eq!(log.borrow().edits.last(), Some(&EditOp::DuplicateSlide(1)));
    assert_eq!(deck(&mut rt), (3, 5));
    press(&mut rt, "Delete slide");
    assert_eq!(log.borrow().edits.last(), Some(&EditOp::DeleteSlide(2)));
    assert_eq!(deck(&mut rt).1, 4);
}

#[test]
fn office_slides_ribbon_present_starts_the_show() {
    let (mut rt, _) = editing(Tab::Home, false);
    press(&mut rt, "Present");
    assert!(viewers::deck_mut(&mut rt.state).expect("a deck").presenting);
}

#[test]
fn office_slides_ribbon_design_lists_layouts_and_applies_one() {
    let (mut rt, log) = editing(Tab::Layout, true);
    for name in NEW_DECK_LAYOUTS {
        assert!(by_label(&rt, name).is_some(), "{name} is offered");
    }
    // A new deck's first slide shows Title slide.
    assert!(has(&rt, "rl-0-on"));
    press(&mut rt, "Title only");
    assert_eq!(
        log.borrow().edits.last(),
        Some(&EditOp::SetLayout { slide: 0, layout: 3 })
    );
    assert!(has(&rt, "rl-3-on") && !has(&rt, "rl-0-on"));
}

#[test]
fn office_slides_ribbon_design_without_layouts_asks_for_them() {
    let (mut rt, log) = editing(Tab::Layout, false);
    assert!(by_label(&rt, "Title slide").is_none());
    // Opening the deck asked once on its own; the button asks again.
    assert_eq!(log.borrow().asked_layouts, 1);
    press(&mut rt, "Show layouts");
    assert_eq!(log.borrow().asked_layouts, 2);
}

#[test]
fn office_slides_ribbon_an_open_deck_asks_for_its_layouts_once() {
    let (mut rt, log) = editing(Tab::Home, false);
    for _ in 0..3 {
        rt.frame(0.0);
    }
    assert_eq!(log.borrow().asked_layouts, 1);
    let (mut rt, log) = editing(Tab::Home, true);
    rt.frame(0.0);
    assert_eq!(log.borrow().asked_layouts, 0, "layouts already in: nothing to ask");
}

#[test]
fn office_slides_ribbon_tab_words_are_design_and_transitions() {
    let words: Vec<&str> = Tab::ALL
        .iter()
        .map(|t| crate::frames::office::edit::slide::tab_label(*t))
        .collect();
    assert_eq!(words, ["Home", "Insert", "Design", "Transitions"]);
}

#[test]
fn office_slides_ribbon_transitions_say_nothing_to_edit() {
    let (rt, _) = editing(Tab::Review, true);
    assert!(rt.elements().iter().any(|e| e.key.as_deref() == Some("rt-none")));
}

#[test]
fn office_slides_ribbon_design_scene_is_not_the_placeholder() {
    let mut state = init();
    scene_design(&mut state);
    let ui = viewers::ui(&mut state).expect("a file");
    assert_eq!(ui.office.tab, Tab::Layout);
    assert_eq!(ui.office.edit.as_ref().map(|e| e.layouts.len()), Some(5));
}
