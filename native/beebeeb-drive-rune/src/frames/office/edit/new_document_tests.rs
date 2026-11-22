//! Tests of New document (DW-1558): the dialog's words and choices, the name's checks, what Create asks of the
//! source, how a refusal is said, and that the made file opens in the editor.

use std::sync::{Arc, Mutex};

use beebeeb_office_protocol::office::{EditKind, Template};
use rune::{Env, Key, KeyEvent, Mods, Runtime};

use super::*;
use crate::frames::viewers::source::{Body, FixtureViewerSource, OpenError, OpenFile, Settled, ViewerSource, board};
use crate::frames::viewers::{self, new_file::Step};
use crate::{init, view};

type Asked = Arc<Mutex<Vec<(Option<String>, String, EditKind, Template)>>>;

/// The board's sample source, taking New document (or refusing it), answering with the board's Word file.
struct Maker {
    inner: FixtureViewerSource,
    asked: Asked,
    take: bool,
    answer: Option<Settled>,
}

impl ViewerSource for Maker {
    fn open(&mut self, id: &str) -> Result<OpenFile, OpenError> {
        self.inner.open(id)
    }
    fn poll(&mut self, id: &str) -> Option<Body> {
        self.inner.poll(id)
    }
    fn cancel(&mut self, id: &str) {
        self.inner.cancel(id)
    }
    fn retry(&mut self, id: &str) -> Result<OpenFile, OpenError> {
        self.inner.retry(id)
    }
    fn create_document(&mut self, folder: Option<&str>, name: &str, kind: EditKind, template: Template) -> bool {
        self.asked
            .lock()
            .unwrap()
            .push((folder.map(str::to_string), name.to_string(), kind, template));
        self.take
    }
    fn settled(&mut self, _id: &str) -> Option<Settled> {
        if self.asked.lock().unwrap().is_empty() {
            None
        } else {
            self.answer.take()
        }
    }
}

fn saved() -> Settled {
    let file = FixtureViewerSource::new().open(board::DOCX).ok().unwrap();
    Settled::Saved {
        id: file.id.clone(),
        name: file.name.clone(),
        body: file.body,
        version: file.version,
        at: "now".into(),
    }
}

fn runtime(take: bool, answer: Option<Settled>) -> (Runtime<crate::State>, Asked) {
    let asked = Asked::default();
    let mut state = init();
    state.actions = crate::frames::files::Actions::new(Box::new(crate::frames::files::source::FilesFixture::default()));
    let (a, answer) = (asked.clone(), Arc::new(Mutex::new(answer)));
    viewers::wire(
        &mut state,
        Box::new(move || {
            Box::new(Maker {
                inner: FixtureViewerSource::new(),
                asked: a.clone(),
                take,
                answer: answer.lock().unwrap().take(),
            })
        }),
    );
    let mut rt = Runtime::new(
        state,
        view,
        (1278.0, 798.0),
        Env {
            system_dark: false,
            scale: 2.0,
        },
    );
    rt.on_key = Some(crate::shell::host::on_key);
    rt.poll = Some(crate::shell::host::poll);
    rt.frame(0.0);
    (rt, asked)
}

fn info(rt: &Runtime<crate::State>, key: &str) -> rune::ElementInfo {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
        .unwrap_or_else(|| panic!("no element keyed {key}"))
}

fn has(rt: &Runtime<crate::State>, key: &str) -> bool {
    rt.elements()
        .iter()
        .any(|e| e.key.as_deref() == Some(key) && !e.exiting)
}

fn open(rt: &mut Runtime<crate::State>, kind: Option<Kind>) {
    assert!(begin(&mut rt.state, kind, Some("folder1".into())), "opens over Files");
    rt.frame(0.0);
}

fn step(rt: &Runtime<crate::State>) -> Option<Step> {
    rt.state.viewer.new_file.as_ref().map(|nf| nf.step)
}

fn name(rt: &Runtime<crate::State>) -> String {
    rt.state.viewer.new_file.as_ref().unwrap().name.text.clone()
}

/// A click on Create, as the person presses it, then the polls that follow.
fn press_create(rt: &mut Runtime<crate::State>) {
    rt.frame(1.0);
    let [x, y, w, h] = info(rt, "btn-0").rect;
    rt.pointer_down(x + w / 2.0, y + h / 2.0);
    rt.pointer_up(x + w / 2.0, y + h / 2.0);
    rt.frame(0.0);
    rt.frame(0.0);
}

#[test]
fn new_document_dialog_says_the_boards_words() {
    let (mut rt, _) = runtime(true, None);
    open(&mut rt, None);
    assert_eq!(step(&rt), Some(Step::Document));
    assert_eq!(info(&rt, "dlg-title").text.as_deref(), Some("New document"));
    for (i, (_, title, words)) in TEMPLATES.iter().enumerate() {
        assert_eq!(info(&rt, &format!("tpl-{i}-title")).text.as_deref(), Some(*title));
        assert_eq!(info(&rt, &format!("tpl-{i}-words")).text.as_deref(), Some(*words));
    }
    assert_eq!(TEMPLATES[0].2, "Start from nothing");
    for (i, word) in ["Docs", "Sheets", "Slides"].into_iter().enumerate() {
        assert_eq!(info(&rt, &format!("doc-tab-{i}-label")).text.as_deref(), Some(word));
    }
    assert_eq!(
        info(&rt, "doc-encrypted").text.as_deref(),
        Some("Everything you create here is encrypted before it leaves this device.")
    );
    assert_eq!(name(&rt), "Untitled document.docx");
    assert!(has(&rt, "btn-0") && has(&rt, "btn-1"));
}

#[test]
fn new_document_sheets_and_slides_have_blank_only_and_their_own_extension() {
    let (mut rt, _) = runtime(true, None);
    open(&mut rt, None);
    choose_template(&mut rt.state, Template::Invoice);
    choose_kind(&mut rt.state, Kind::Excel);
    rt.frame(0.0);
    assert_eq!(name(&rt), "Untitled document.xlsx");
    assert!(has(&rt, "tpl-0") && !has(&rt, "tpl-1"), "Sheets: Blank only");
    choose_kind(&mut rt.state, Kind::PowerPoint);
    assert_eq!(name(&rt), "Untitled document.pptx");
    choose_kind(&mut rt.state, Kind::Word);
    let d = rt.state.viewer.new_file.as_ref().unwrap().doc.as_ref().unwrap();
    assert_eq!(
        d.template,
        Template::Blank,
        "the template goes back to Blank with the app"
    );
}

#[test]
fn new_document_create_asks_the_source_and_opens_the_made_file() {
    let (mut rt, asked) = runtime(true, Some(saved()));
    open(&mut rt, Some(Kind::Word));
    choose_template(&mut rt.state, Template::Invoice);
    press_create(&mut rt);
    assert_eq!(
        *asked.lock().unwrap(),
        [(
            Some("folder1".to_string()),
            "Untitled document.docx".to_string(),
            EditKind::Docx,
            Template::Invoice
        )]
    );
    assert_eq!(step(&rt), None, "the dialog closed once the file was made");
    assert!(rt.state.viewer.ui.is_some(), "the new file is open");
}

#[test]
fn new_document_waits_for_the_answer_and_asks_once() {
    let (mut rt, asked) = runtime(true, None);
    open(&mut rt, None);
    press_create(&mut rt);
    press_create(&mut rt);
    assert_eq!(
        asked.lock().unwrap().len(),
        1,
        "a second press while it works asks nothing"
    );
    assert_eq!(step(&rt), Some(Step::Document));
    assert_eq!(info(&rt, "btn-0").label.as_deref(), Some("Creating\u{2026}"));
}

#[test]
fn new_document_refusal_is_said_and_the_dialog_stays() {
    let (mut rt, asked) = runtime(false, None);
    open(&mut rt, None);
    press_create(&mut rt);
    assert_eq!(asked.lock().unwrap().len(), 1);
    assert_eq!(step(&rt), Some(Step::Document));
    assert_eq!(info(&rt, "doc-failed").text.as_deref(), Some("Couldn't make the file."));
    // A failed creation (the engine answered not done) is said the same way.
    let (mut rt, _) = runtime(true, Some(Settled::Failed));
    open(&mut rt, None);
    press_create(&mut rt);
    assert_eq!(info(&rt, "doc-failed").text.as_deref(), Some("Couldn't make the file."));
    assert_eq!(
        info(&rt, "btn-0").label.as_deref(),
        Some("Create"),
        "Create can be pressed again"
    );
}

#[test]
fn new_document_names_are_checked_like_new_files() {
    let (mut rt, asked) = runtime(true, None);
    open(&mut rt, None);
    rt.state.viewer.new_file.as_mut().unwrap().name = rune::TextEdit::new("   ");
    press_create(&mut rt);
    rt.state.viewer.new_file.as_mut().unwrap().name = rune::TextEdit::new("a/b.docx");
    press_create(&mut rt);
    rt.state.viewer.new_file.as_mut().unwrap().name = rune::TextEdit::new(".docx");
    press_create(&mut rt);
    assert!(asked.lock().unwrap().is_empty(), "no bad name reaches the source");
    assert!(has(&rt, "field-0-msg"), "the reason is said");
    // The app's extension is added when it was left off.
    rt.state.viewer.new_file.as_mut().unwrap().name = rune::TextEdit::new("Plan");
    press_create(&mut rt);
    assert_eq!(asked.lock().unwrap()[0].1, "Plan.docx");
}

#[test]
fn new_document_cancel_and_escape_make_nothing() {
    let (mut rt, asked) = runtime(true, None);
    open(&mut rt, None);
    rt.key(KeyEvent {
        key: Key::Escape,
        mods: Mods::default(),
    });
    rt.frame(0.0);
    assert_eq!(step(&rt), None);
    assert!(asked.lock().unwrap().is_empty());
}

#[test]
fn new_document_scene_draws_the_dialog() {
    let mut s = init();
    s.actions = crate::frames::files::Actions::new(Box::new(crate::frames::files::source::FilesFixture::default()));
    crate::frames::office::dialogs::scene_new_document(&mut s);
    assert!(s.viewer.new_file.as_ref().is_some_and(|nf| nf.step == Step::Document));
}
