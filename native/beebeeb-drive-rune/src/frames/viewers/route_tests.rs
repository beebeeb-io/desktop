//! What the window draws for a file that could not be opened, where the content on screen came from, and which
//! app a list's Open goes to: each through a source of the test's own that answers what the test says, driven by
//! the events the app gets (the route, a click) and read from the element tree. Every name and word is invented.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use rune::{Env, Runtime};

use super::source::{
    Body, Details, NoPreview, OpenError, OpenFile, Origin, Stopped, TextDoc, ViewerSource, plain_lines,
};
use super::{chrome, open_item, text, wire};
use crate::shell::app::{Event, Screen};
use crate::shell::host;
use crate::{State, init, view};

const FILE: &str = "f-invented-route";
const WORDS: &str = "an-invented-line-of-the-routed-file";

fn text_file(id: &str) -> OpenFile {
    OpenFile {
        id: id.to_string(),
        path: vec!["Files".into()],
        name: "Invented routed.txt".into(),
        facts: "Text \u{00B7} 35 B".into(),
        version: 0,
        body: Body::Text(TextDoc {
            lines: plain_lines(WORDS),
            truncated: None,
        }),
        details: Details::default(),
        newer: None,
    }
}

/// What the source answers and what it was asked, shared with the test.
#[derive(Default)]
struct Script {
    /// The next answers of `open` and `retry`, oldest first; when it is empty, the text file.
    answers: VecDeque<Result<Body, OpenError>>,
    asked: Vec<&'static str>,
    origin: Option<Origin>,
    kept_here: bool,
    cannot_save: bool,
    no_copies: bool,
}

struct Scripted(Rc<RefCell<Script>>);

impl Scripted {
    fn answer(&mut self, what: &'static str, id: &str) -> Result<OpenFile, OpenError> {
        let mut script = self.0.borrow_mut();
        script.asked.push(what);
        let body = script.answers.pop_front().unwrap_or_else(|| Ok(text_file(id).body))?;
        Ok(OpenFile { body, ..text_file(id) })
    }
}

impl ViewerSource for Scripted {
    fn open(&mut self, id: &str) -> Result<OpenFile, OpenError> {
        self.answer("open", id)
    }
    fn poll(&mut self, _: &str) -> Option<Body> {
        None
    }
    fn cancel(&mut self, _: &str) {}
    fn retry(&mut self, id: &str) -> Result<OpenFile, OpenError> {
        self.answer("retry", id)
    }
    fn kept_on_this_pc(&self, _: &str) -> bool {
        self.0.borrow_mut().asked.push("kept?");
        self.0.borrow().kept_here
    }
    fn origin(&self, _: &str) -> Origin {
        self.0.borrow().origin.unwrap_or(Origin::Beebeeb)
    }
    fn can_save(&self) -> bool {
        !self.0.borrow().cannot_save
    }
    fn saves_copies(&self) -> bool {
        !self.0.borrow().no_copies
    }
}

/// A window whose route's files come from the script.
fn window(script: Script) -> (Runtime<State>, Rc<RefCell<Script>>) {
    let script = Rc::new(RefCell::new(script));
    let mut state = init();
    let for_source = script.clone();
    wire(&mut state, Box::new(move || Box::new(Scripted(for_source.clone()))));
    let env = Env {
        system_dark: false,
        scale: 2.0,
    };
    let mut rt = Runtime::new(state, view, (1278.0, 798.0), env);
    rt.on_key = Some(host::on_key);
    rt.poll = Some(host::poll);
    rt.frame(0.0);
    (rt, script)
}

/// The route names `FILE`, as Search's Open, Back and Forward and the reopen after an unlock do.
fn route_to_file(rt: &mut Runtime<State>) {
    rt.update(|s, _| {
        host::dispatch(s, Event::OpenItem(FILE.to_string()));
    });
    frames(rt, 6);
}

fn frames(rt: &mut Runtime<State>, count: usize) {
    for _ in 0..count {
        rt.frame(0.016);
    }
}

fn drawn(rt: &Runtime<State>) -> String {
    rt.elements()
        .into_iter()
        .filter(|e| !e.exiting)
        .flat_map(|e| [e.text, e.label].into_iter().flatten())
        .collect::<Vec<_>>()
        .join("\n")
}

#[track_caller]
fn click(rt: &mut Runtime<State>, key: &str) {
    let rect = rt
        .elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
        .unwrap_or_else(|| panic!("no element keyed {key}; the window draws:\n{}", drawn(rt)))
        .rect;
    let (x, y) = (rect[0] + rect[2] / 2.0, rect[1] + rect[3] / 2.0);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    frames(rt, 4);
}

fn route_item(rt: &Runtime<State>) -> Option<String> {
    match rt.state.shell.app.screen() {
        Screen::Drive(route) => route.item,
        _ => None,
    }
}

fn opens(script: &Rc<RefCell<Script>>, what: &str) -> usize {
    script.borrow().asked.iter().filter(|asked| **asked == what).count()
}

#[test]
fn a_route_to_a_file_that_is_gone_says_so_and_close_goes_back() {
    let (mut rt, script) = window(Script {
        answers: VecDeque::from([Err(OpenError::NotFound)]),
        ..Script::default()
    });
    route_to_file(&mut rt);
    let words = drawn(&rt);
    assert!(words.contains("This file is no longer here"), "{words}");
    assert!(words.contains("moved to Trash or deleted on another device"), "{words}");
    assert!(
        !words.contains("Not built yet") && !words.contains("not built"),
        "{words}"
    );
    assert!(
        !words.contains("Try again"),
        "a file that is gone is not asked for again"
    );
    // Asked once, not on every frame.
    frames(&mut rt, 20);
    assert_eq!(opens(&script, "open"), 1);
    // Close: back to the folder.
    click(&mut rt, "state-btn-0");
    assert_eq!(route_item(&rt), None);
    assert!(rt.state.viewer.ui.is_none());
}

#[test]
fn a_file_that_could_not_be_read_right_now_says_so_and_try_again_opens_it() {
    let (mut rt, script) = window(Script {
        answers: VecDeque::from([Err(OpenError::NotNow)]),
        ..Script::default()
    });
    route_to_file(&mut rt);
    let words = drawn(&rt);
    assert!(words.contains("Couldn't open this file right now"), "{words}");
    assert!(
        !words.contains("no longer here") && !words.contains("Not built yet"),
        "{words}"
    );
    assert!(!words.contains(WORDS));
    frames(&mut rt, 20);
    assert_eq!(opens(&script, "open"), 1, "not asked again on every frame");
    assert_eq!(opens(&script, "retry"), 0);
    // Try again: the refusal passed, and the file opens.
    click(&mut rt, "state-btn-0");
    assert_eq!(opens(&script, "retry"), 1);
    let words = drawn(&rt);
    assert!(words.contains(WORDS), "{words}");
    assert!(!words.contains("Couldn't open this file right now"), "{words}");
}

#[test]
fn try_again_on_a_file_that_is_gone_by_now_says_gone_in_place() {
    let (mut rt, _script) = window(Script {
        answers: VecDeque::from([Err(OpenError::NotNow), Err(OpenError::NotFound)]),
        ..Script::default()
    });
    route_to_file(&mut rt);
    click(&mut rt, "state-btn-0");
    let words = drawn(&rt);
    assert!(words.contains("This file is no longer here"), "{words}");
    assert!(!words.contains("Couldn't open this file right now"), "{words}");
}

#[test]
fn a_session_beebeeb_ended_is_said_and_no_session_draws_nothing_of_a_file() {
    let (mut rt, _script) = window(Script {
        answers: VecDeque::from([Err(OpenError::SessionEnded)]),
        ..Script::default()
    });
    route_to_file(&mut rt);
    let words = drawn(&rt);
    assert!(words.contains("Your session ended"), "{words}");
    assert!(!words.contains("no longer here"), "{words}");

    // No session in memory: nothing of this group is drawn, and the source is not asked on every frame.
    let (mut rt, script) = window(Script {
        answers: VecDeque::from([Err(OpenError::SignedOut)]),
        ..Script::default()
    });
    route_to_file(&mut rt);
    frames(&mut rt, 20);
    assert!(rt.state.viewer.ui.is_none());
    assert_eq!(opens(&script, "open"), 1);
    let words = drawn(&rt);
    assert!(!words.contains("no longer here") && !words.contains(WORDS), "{words}");
}

#[test]
fn the_status_line_says_when_the_content_is_the_copy_on_this_pc_and_when_it_is_not_what_beebeeb_has() {
    let status = |origin: Option<Origin>| {
        let (mut rt, _script) = window(Script {
            origin,
            ..Script::default()
        });
        route_to_file(&mut rt);
        let words = drawn(&rt);
        assert!(words.contains(WORDS), "{words}");
        words
    };
    // Downloaded: the words every open file had.
    let words = status(None);
    assert!(words.contains("Decrypted on this PC"), "{words}");
    assert!(!words.contains(chrome::FROM_THIS_PC), "{words}");
    // The copy kept here.
    let words = status(Some(Origin::ThisPc { unsent: false }));
    assert!(words.contains(chrome::FROM_THIS_PC), "{words}");
    assert!(!words.contains("aren't in Beebeeb yet"), "{words}");
    assert!(!words.contains("Decrypted on this PC"), "{words}");
    // A copy with a change that is not sent: said, never shown as what Beebeeb has.
    let words = status(Some(Origin::ThisPc { unsent: true }));
    assert!(words.contains(chrome::FROM_THIS_PC_UNSENT), "{words}");
    assert!(!words.contains("Decrypted on this PC"), "{words}");
}

#[test]
fn a_state_keeps_its_own_words_whatever_the_origin_says() {
    let (mut rt, _script) = window(Script {
        answers: VecDeque::from([Ok(Body::Failed)]),
        origin: Some(Origin::ThisPc { unsent: true }),
        ..Script::default()
    });
    route_to_file(&mut rt);
    let words = drawn(&rt);
    assert!(words.contains("Not decrypted: nothing changed"), "{words}");
    assert!(!words.contains(chrome::FROM_THIS_PC), "{words}");
}

fn item(name: &str) -> crate::source::DriveItem {
    crate::source::DriveItem {
        id: FILE.into(),
        parent_id: None,
        name: name.into(),
        is_folder: false,
        size: Some(35),
        child_count: None,
        modified_at: String::new(),
        mime: None,
        has_thumbnail: false,
        shared_type: None,
        shared_badges: Vec::new(),
        pc_status: None,
        is_protected: false,
    }
}

#[test]
fn a_lists_open_of_a_file_kept_on_this_pc_is_left_to_its_own_app_and_another_file_opens_here() {
    // Kept on this PC: not this group's, the source is not asked to open it, the route does not move.
    let (mut rt, script) = window(Script {
        kept_here: true,
        ..Script::default()
    });
    let mut opened = true;
    rt.update(|s, _| opened = open_item(s, &item("Invented routed.txt")));
    frames(&mut rt, 4);
    assert!(!opened);
    assert_eq!(opens(&script, "open"), 0);
    assert_eq!(opens(&script, "kept?"), 1);
    assert_eq!(route_item(&rt), None);
    assert!(rt.state.viewer.ui.is_none());

    // Not on this PC: the viewer.
    let (mut rt, script) = window(Script::default());
    let mut opened = false;
    rt.update(|s, _| opened = open_item(s, &item("Invented routed.txt")));
    frames(&mut rt, 4);
    assert!(opened);
    assert_eq!(opens(&script, "open"), 1);
    assert_eq!(route_item(&rt).as_deref(), Some(FILE));
    assert!(drawn(&rt).contains(WORDS));
}

#[test]
fn save_a_copy_is_drawn_only_where_it_has_somewhere_to_go() {
    let buttons = |no_copies: bool| {
        let (mut rt, _script) = window(Script {
            answers: VecDeque::from([Ok(Body::NotPreviewable(NoPreview::NoViewer))]),
            no_copies,
            ..Script::default()
        });
        route_to_file(&mut rt);
        drawn(&rt)
    };
    let with = buttons(false);
    assert!(
        with.contains("Open in another app\u{2026}") && with.contains("Save a copy\u{2026}"),
        "{with}"
    );
    let without = buttons(true);
    assert!(without.contains("Open in another app\u{2026}"), "{without}");
    assert!(!without.contains("Save a copy"), "{without}");
}

#[test]
fn a_picture_that_declares_too_much_says_its_size_and_that_it_was_not_decoded() {
    let (mut rt, _script) = window(Script {
        answers: VecDeque::from([Ok(Body::NotPreviewable(NoPreview::PictureTooBig {
            width: 9000,
            height: 7000,
        }))]),
        ..Script::default()
    });
    route_to_file(&mut rt);
    let words = drawn(&rt);
    assert!(words.contains("Too large to preview here"), "{words}");
    assert!(words.contains("9000 \u{00D7} 7000 pixels"), "{words}");
    assert!(words.contains("not decoded"), "{words}");
}

#[test]
fn a_text_a_source_cannot_save_does_not_name_the_edit_key() {
    let hint = |cannot_save: bool| {
        let (mut rt, _script) = window(Script {
            cannot_save,
            ..Script::default()
        });
        route_to_file(&mut rt);
        drawn(&rt)
    };
    assert!(hint(false).contains("Ctrl+E to edit"));
    let read_only = hint(true);
    assert!(read_only.contains(text::READ_ONLY_HINT), "{read_only}");
    assert!(!read_only.contains("Ctrl+E to edit"), "{read_only}");
}

#[test]
fn each_refusal_to_open_has_its_own_state_and_no_session_has_none() {
    assert_eq!(OpenError::NotFound.stopped(), Some(Stopped::Gone));
    assert_eq!(OpenError::SessionEnded.stopped(), Some(Stopped::SessionEnded));
    assert_eq!(OpenError::NotNow.stopped(), Some(Stopped::NotNow));
    assert_eq!(OpenError::SignedOut.stopped(), None);
    assert!(Stopped::NotNow.can_retry() && !Stopped::Gone.can_retry());
}

#[test]
fn a_file_that_is_left_takes_its_name_its_folders_and_its_details_and_none_is_printed() {
    let mut file = text_file(FILE);
    file.path = vec!["Files".into(), "Invented private folder".into()];
    file.details.facts = vec![("Where", "Files \u{203A} Invented private folder".into(), false)];
    file.newer = Some(super::source::Newer {
        from: "An invented person's phone".into(),
        at: "09:14".into(),
    });
    let printed = format!("{file:?} {:?} {:?}", file.details, file.newer);
    for secret in ["Invented", "private", "person"] {
        assert!(!printed.contains(secret), "{printed}");
    }
    file.wipe();
    assert!(file.name.is_empty() && file.facts.is_empty());
    assert!(file.path.iter().all(String::is_empty));
    assert!(file.details.facts.iter().all(|(_, value, _)| value.is_empty()));
    assert!(file.newer.as_ref().is_some_and(|newer| newer.from.is_empty()));
}
