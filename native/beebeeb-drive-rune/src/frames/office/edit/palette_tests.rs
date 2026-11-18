use std::cell::RefCell;
use std::rc::Rc;

use beebeeb_office_protocol::office::{EditKind, EditOp, ListKind, Mark, Marks, ParaStyle, TextAlign};
use rune::{Key, KeyEvent, Mods, Runtime};

use super::*;
use crate::frames::office::edit::EditUi;
use crate::frames::viewers::office_over::Over;
use crate::frames::viewers::source::{Body, FixtureViewerSource, OpenError, OpenFile, ViewerSource, board};
use crate::frames::viewers::{self, Intent};
use crate::{State, init, view};

fn names(groups: &[CommandGroup]) -> Vec<String> {
    groups.iter().flat_map(|g| g.commands.iter().map(|c| c.name.clone())).collect()
}

fn files(names: &[&str]) -> Vec<CommandGroup> {
    vec![CommandGroup {
        title: FILES.into(),
        commands: names.iter().map(|n| Command { name: n.to_string(), shortcut: None }).collect(),
    }]
}

#[test]
fn each_kind_lists_its_own_operations_and_typing_filters() {
    let word = names(&found(Some(EditKind::Docx), "", Vec::new()));
    assert!(word.contains(&"Bold".to_string()) && word.contains(&"Heading 1".to_string()));
    assert!(!word.contains(&"Sort A to Z".to_string()));
    let sheet = names(&found(Some(EditKind::Xlsx), "sort", Vec::new()));
    assert_eq!(sheet, ["Sort A to Z", "Sort Z to A"]);
    let slides = names(&found(Some(EditKind::Pptx), "NEW sl", Vec::new()));
    assert_eq!(slides, ["New slide"]);
    let bold = found(Some(EditKind::Docx), "bold", Vec::new());
    assert_eq!(bold[0].commands[0].shortcut.as_deref(), Some("Ctrl+B"));
    // Without an editor only the source's answer,: the source filters its own names.
    assert!(found(None, "bold", Vec::new()).is_empty());
    assert_eq!(names(&found(Some(EditKind::Docx), "plan", files(&["Plan.pdf", "Other.pdf"]))), ["Plan.pdf", "Other.pdf"]);
    // A name the document's styles share with ours is listed once.
    let styles = vec![CommandGroup {
        title: STYLES.into(),
        commands: vec![Command { name: "Heading 1".into(), shortcut: None }],
    }];
    assert_eq!(names(&found(Some(EditKind::Docx), "heading 1", styles)), ["Heading 1"]);
}

#[test]
fn a_result_resolves_to_its_operation_and_a_mark_toggles() {
    let k = Some(EditKind::Docx);
    assert_eq!(resolve(k, "Data", "Sort A to Z", None), Run::Unknown);
    assert_eq!(resolve(k, STYLES, "Heading 2", None), Run::Op(EditOp::ParaStyle(ParaStyle::Heading2)));
    assert_eq!(resolve(k, "Format", "Bold", None), Run::Op(EditOp::Mark { mark: Mark::Bold, on: true }));
    let bold = Marks {
        bold: true,
        italic: false,
        underline: false,
        strike: false,
        style: ParaStyle::Normal,
        align: TextAlign::Left,
        list: ListKind::None,
    };
    assert_eq!(resolve(k, "Format", "Bold", Some(&bold)), Run::Op(EditOp::Mark { mark: Mark::Bold, on: false }));
    assert_eq!(resolve(k, FILES, "A.pdf", None), Run::File("A.pdf".into()));
    assert_eq!(resolve(Some(EditKind::Pptx), "Slides", "New slide", None), Run::Op(EditOp::NewSlide { layout: None }));
}

struct Recording {
    inner: FixtureViewerSource,
    ops: Rc<RefCell<Vec<EditOp>>>,
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
        self.ops.borrow_mut().push(op);
        true
    }
    fn commands(&self, _: &str, _: &str) -> Vec<CommandGroup> {
        files(&["Northwind brand guide.pdf"])
    }
}

fn open(live: bool) -> (Runtime<State>, Rc<RefCell<Vec<EditOp>>>) {
    let ops = Rc::new(RefCell::new(Vec::new()));
    let mut state = init();
    let source = Recording { inner: FixtureViewerSource::new(), ops: ops.clone() };
    assert!(viewers::open_from(&mut state, Box::new(source), board::DOCX));
    if live {
        viewers::ui(&mut state).expect("open").office.edit = Some(EditUi::new(EditKind::Docx));
    }
    let env = rune::Env { system_dark: false, scale: 2.0 };
    let mut rt = Runtime::new(state, view, (1278.0, 798.0), env);
    rt.on_key = Some(crate::shell::host::on_key);
    rt.frame(0.0);
    (rt, ops)
}

fn key(rt: &mut Runtime<State>, key: Key, ctrl: bool) {
    let mods = Mods { cmd: ctrl, ..Mods::default() };
    rt.key(KeyEvent { key, mods });
    rt.frame(0.0);
}

fn type_in(rt: &mut Runtime<State>, words: &str) {
    rt.update(|s, _| {
        if let Some(Over::Palette(p)) = viewers::ui(s).and_then(|u| u.office.over.as_mut()) {
            p.query.insert(words);
        }
    });
    rt.frame(0.0);
}

/// The whole path: Ctrl+K, typed words, Enter reaches the source as the edit; a file name opens that file;
/// Escape closes with nothing sent.
#[test]
fn ctrl_k_types_and_enter_runs_through_the_source() {
    let (mut rt, ops) = open(true);
    key(&mut rt, Key::Command('k'), true);
    type_in(&mut rt, "heading 2");
    key(&mut rt, Key::Enter, false);
    assert!(viewers::ui(&mut rt.state).unwrap().office.over.is_none());
    assert_eq!(ops.borrow().len(), 1);
    assert!(matches!(ops.borrow()[0], EditOp::ParaStyle(_)));

    key(&mut rt, Key::Command('k'), true);
    type_in(&mut rt, "northwind");
    key(&mut rt, Key::Enter, false);
    let intents = viewers::ui(&mut rt.state).unwrap().take_intents();
    assert_eq!(intents, [Intent::OpenInDrive("Northwind brand guide.pdf".into())]);

    key(&mut rt, Key::Command('k'), true);
    type_in(&mut rt, "bold");
    key(&mut rt, Key::Escape, false);
    assert!(viewers::ui(&mut rt.state).unwrap().office.over.is_none());
    assert_eq!(ops.borrow().len(), 1, "Escape sends nothing");
}

#[test]
fn without_a_live_editor_no_operation_is_listed() {
    let (mut rt, ops) = open(false);
    key(&mut rt, Key::Command('k'), true);
    type_in(&mut rt, "bold");
    key(&mut rt, Key::Enter, false);
    assert!(ops.borrow().is_empty());
}
