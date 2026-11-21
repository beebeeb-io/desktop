//! ux-15: the editor's find, indent and outdent keys.
use rune::{Key, KeyEvent, Mods};

use rune::{Env, Runtime};

use super::source::{Body, Details, MdDoc, OpenError, OpenFile, TextDoc, ViewerSource, plain_lines};
use super::open_from;
use crate::{State, init, view};

/// A source that opens one invented file and can save it.
struct One {
    name: String,
    text: String,
}

impl ViewerSource for One {
    fn open(&mut self, id: &str) -> Result<OpenFile, OpenError> {
        let body = if self.name.ends_with(".md") {
            Body::Markdown(MdDoc {
                source: self.text.clone(),
            })
        } else {
            Body::Text(TextDoc {
                lines: plain_lines(&self.text),
                truncated: None,
            })
        };
        Ok(OpenFile {
            id: id.into(),
            path: Vec::new(),
            name: self.name.clone(),
            facts: String::new(),
            version: 1,
            body,
            details: Details {
                kind: String::new(),
                size: String::new(),
                facts: Vec::new(),
                offline: false,
            },
            newer: None,
        })
    }
    fn poll(&mut self, _id: &str) -> Option<Body> {
        None
    }
    fn cancel(&mut self, _id: &str) {}
    fn retry(&mut self, id: &str) -> Result<OpenFile, OpenError> {
        self.open(id)
    }
}

/// The editor open on `name` holding `text`.
fn editing(name: &str, text: &str) -> Runtime<State> {
    let mut state = init();
    let source = One {
        name: name.into(),
        text: text.into(),
    };
    assert!(open_from(&mut state, Box::new(source), "invented-1"));
    let env = Env {
        system_dark: false,
        scale: 2.0,
    };
    let mut rt = Runtime::new(state, view, (1278.0, 798.0), env);
    rt.on_key = Some(crate::shell::host::on_key);
    rt.poll = Some(crate::shell::host::poll);
    rt.frame(0.0);
    rt.key(KeyEvent {
        key: Key::Command('e'),
        mods: Mods {
            cmd: true,
            ..Mods::default()
        },
    });
    rt.frame(0.0);
    rt.settle_all();
    rt.frame(0.0);
    assert!(rt.state.viewer.ui.as_ref().unwrap().editor.is_some(), "Ctrl+E opens the editor");
    rt
}


fn press(rt: &mut rune::Runtime<crate::State>, key: Key, mods: Mods) {
    rt.key(KeyEvent { key, mods });
    rt.frame(0.0);
}

fn ctrl() -> Mods {
    Mods { cmd: true, ..Mods::default() }
}

fn text(rt: &rune::Runtime<crate::State>) -> String {
    rt.state.viewer.ui.as_ref().unwrap().editor.as_ref().unwrap().edit.text.clone()
}

fn edit_mut(rt: &mut rune::Runtime<crate::State>) -> &mut rune::TextEdit {
    &mut rt.state.viewer.ui.as_mut().unwrap().editor.as_mut().unwrap().edit
}

#[test]
fn editor_keys_ctrl_f_opens_and_escape_closes_find() {
    let mut rt = editing("a.txt", "one\ntwo\n");
    press(&mut rt, Key::Command('f'), ctrl());
    assert!(rt.state.viewer.ui.as_ref().unwrap().editor_view.find.open);
    press(&mut rt, Key::Escape, Mods::default());
    assert!(!rt.state.viewer.ui.as_ref().unwrap().editor_view.find.open);
    assert!(rt.state.viewer.ui.as_ref().unwrap().editor.is_some());
}

#[test]
fn editor_keys_tab_inserts_the_files_indent_unit() {
    let mut rt = editing("a.py", "def f():\n    x = 1\n    y = 2\n");
    edit_mut(&mut rt).set_caret(0);
    press(&mut rt, Key::Tab, Mods::default());
    assert!(text(&rt).starts_with("    def f():"));
}

#[test]
fn editor_keys_tab_indents_selected_lines_and_shift_tab_undoes() {
    let src = "\ta\n\tb\n\tc\n";
    let mut rt = editing("a.txt", src);
    let end = src.len();
    edit_mut(&mut rt).set_selection(0, end);
    press(&mut rt, Key::Tab, Mods::default());
    assert_eq!(text(&rt), "\t\ta\n\t\tb\n\t\tc\n");
    press(&mut rt, Key::Tab, Mods { shift: true, ..Mods::default() });
    assert_eq!(text(&rt), src);
}

#[test]
fn editor_keys_ctrl_s_still_saves() {
    // Nothing changed: Ctrl+S goes back to the reader (the editor closes) rather than typing anything.
    let mut rt = editing("a.txt", "hi\n");
    press(&mut rt, Key::Command('s'), ctrl());
    assert!(rt.state.viewer.ui.as_ref().is_some_and(|ui| ui.editor.is_none()));
}
