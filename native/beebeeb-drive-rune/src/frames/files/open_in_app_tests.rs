//! w29: every open from inside the app goes to the app's own viewer first. The viewer here is a source that says
//! "kept on this PC" for every file (the case that used to go to Edge, Photos or Word) and shows each kind or its
//! "No preview" frame; the OS is the spy's `Call::Open`, which must stay empty without the press on
//! "Open in another app".

use super::source::Call;
use super::tests::*;
use crate::frames::viewers::source::{Body, Details, OpenError, OpenFile, TextDoc, ViewerSource, plain_lines};
use crate::shell::app::Screen;
use crate::{State, init};
use rune::{Key, Runtime};

/// Shows every file as text and says every file is kept here (the case that used to go to Edge, Photos or Word).
/// The "No preview" frame of a kind with no viewer is the engine source's (`viewer_tests`).
struct Kept;

impl ViewerSource for Kept {
    fn open(&mut self, id: &str) -> Result<OpenFile, OpenError> {
        let body = Body::Text(TextDoc {
            lines: plain_lines("shown in the app"),
            truncated: None,
        });
        Ok(OpenFile {
            id: id.to_string(),
            path: vec!["Files".into()],
            name: "x".into(),
            facts: String::new(),
            version: 0,
            body,
            details: Details::default(),
            newer: None,
        })
    }
    fn poll(&mut self, _: &str) -> Option<Body> {
        None
    }
    fn cancel(&mut self, _: &str) {}
    fn retry(&mut self, id: &str) -> Result<OpenFile, OpenError> {
        self.open(id)
    }
    fn kept_on_this_pc(&self, _: &str) -> bool {
        true
    }
    fn opens_here_when_kept(&self, _: &str) -> bool {
        false
    }
}

fn window(names: [&str; 2]) -> (Runtime<State>, Calls) {
    let mut state = init();
    crate::frames::viewers::wire(&mut state, Box::new(|| Box::new(Kept)));
    let (mut rt, calls, _) = spied_on(state);
    rt.update(|s, _| {
        s.listing.items[6].name = names[0].into();
        s.listing.items[7].name = names[1].into();
        s.select_only(6);
    });
    settle(&mut rt);
    (rt, calls)
}

fn in_viewer(rt: &Runtime<State>, id: &str) -> bool {
    matches!(rt.state.shell.app.screen(), Screen::Drive(route) if route.item.as_deref() == Some(id))
}

const KINDS: [&str; 8] = [
    "Invented.pdf",
    "Invented.png",
    "Invented.mp4",
    "Invented.txt",
    "Invented.docx",
    "Invented.xlsx",
    "Invented.pptx",
    "Invented.blend",
];

#[test]
fn enter_on_every_kind_kept_on_this_pc_opens_the_viewer_and_never_the_os() {
    for name in KINDS {
        let (mut rt, calls) = window([name, "Invented other.txt"]);
        let id = rt.state.listing.items[6].id.clone();
        press(&mut rt, Key::Enter, NONE);
        settle(&mut rt);
        assert!(asked(&calls).is_empty(), "{name}: {:?}", asked(&calls));
        assert!(in_viewer(&rt, &id), "{name}");
    }
}

#[test]
fn a_double_click_on_every_kind_opens_the_viewer_and_never_the_os() {
    for name in KINDS {
        let (mut rt, calls) = window([name, "Invented other.txt"]);
        let id = rt.state.listing.items[6].id.clone();
        rt.update(|s, tx| {
            s.open_row(6, tx);
        });
        settle(&mut rt);
        assert!(asked(&calls).is_empty(), "{name}: {:?}", asked(&calls));
        assert!(in_viewer(&rt, &id), "{name}");
    }
}

#[test]
fn the_details_panes_open_goes_to_the_viewer_too() {
    for name in KINDS {
        let (mut rt, calls) = window([name, "Invented other.txt"]);
        let id = rt.state.listing.items[6].id.clone();
        rt.update(|s, tx| s.open_from_preview(tx));
        settle(&mut rt);
        assert!(asked(&calls).is_empty(), "{name}: {:?}", asked(&calls));
        assert!(in_viewer(&rt, &id), "{name}");
    }
}

#[test]
fn several_files_open_the_first_in_the_viewer_and_none_in_the_os() {
    let (mut rt, calls) = window(["Invented one.pdf", "Invented two.png"]);
    let first = rt.state.listing.items[6].id.clone();
    rt.update(|s, _| {
        s.actions.selection.insert(7);
    });
    press(&mut rt, Key::Enter, NONE);
    settle(&mut rt);
    assert!(asked(&calls).is_empty(), "{:?}", asked(&calls));
    assert!(in_viewer(&rt, &first));
}

#[test]
fn without_a_viewer_wired_a_file_is_still_never_handed_to_the_os() {
    let (mut rt, calls, _) = spied();
    press(&mut rt, Key::Enter, NONE);
    settle(&mut rt);
    assert!(!asked(&calls).iter().any(|c| matches!(c, Call::Open(_))), "{:?}", asked(&calls));
}
