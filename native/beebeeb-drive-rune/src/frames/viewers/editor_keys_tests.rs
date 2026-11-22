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

// ---------------------------------------------------------------------------
// drive-complete-160: the editor on the keyboard layer and to a screen reader
// ---------------------------------------------------------------------------

fn focused(rt: &rune::Runtime<crate::State>) -> String {
    rt.elements()
        .into_iter()
        .find(|e| e.focused && !e.exiting)
        .and_then(|e| e.key)
        .unwrap_or_default()
}

fn settle(rt: &mut rune::Runtime<crate::State>) {
    rt.frame(0.0);
    rt.settle_all();
    rt.frame(0.0);
}

/// The accessibility node keyed `key`.
fn node(tree: &serde_json::Value, key: &str) -> Option<serde_json::Value> {
    if tree["key"].as_str() == Some(key) {
        return Some(tree.clone());
    }
    tree["children"].as_array()?.iter().find_map(|c| node(c, key))
}

fn plain() -> Mods {
    Mods::default()
}

fn shift() -> Mods {
    Mods { shift: true, ..Mods::default() }
}

#[test]
fn editor_a11y_the_field_is_named_after_the_file_and_tells_its_text_and_selection() {
    let mut rt = editing("notes.md", "hello\n");
    edit_mut(&mut rt).set_caret(0);
    settle(&mut rt);
    assert_eq!(focused(&rt), super::EDITOR_KEY, "the editor has the keyboard");
    press(&mut rt, Key::Right, shift());
    settle(&mut rt);
    let tree = rt.accessibility_json();
    let field = node(&tree, super::EDITOR_KEY).unwrap_or_else(|| panic!("no editor node in {tree}"));
    assert_eq!(field["name"], "Editing notes.md", "{field}");
    assert_eq!(field["value"], "hello\n", "the screen reader reads the text: {field}");
    let sel = field["selection"].as_array().unwrap_or_else(|| panic!("no selection: {field}"));
    let (anchor, focus) = (sel[1].as_u64().unwrap(), sel[3].as_u64().unwrap());
    assert_eq!((anchor.min(focus), anchor.max(focus)), (0, 1), "Shift+Right selected one character: {field}");
}

#[test]
fn editor_keys_f6_cycles_the_field_the_find_bar_and_the_preview() {
    let mut rt = editing("notes.md", "one two\n");
    settle(&mut rt);
    assert_eq!(focused(&rt), super::EDITOR_KEY);
    // Field only plus preview: F6 goes to the preview and back.
    press(&mut rt, Key::Function(6), plain());
    settle(&mut rt);
    assert_eq!(focused(&rt), "live-preview", "F6 from the field: the preview");
    press(&mut rt, Key::Function(6), plain());
    settle(&mut rt);
    assert_eq!(focused(&rt), super::EDITOR_KEY, "F6 wraps back to the field");
    // With find open: field, find bar, preview.
    press(&mut rt, Key::Command('f'), ctrl());
    settle(&mut rt);
    assert_eq!(focused(&rt), "find-needle", "Ctrl+F puts the keyboard in the find field");
    press(&mut rt, Key::Function(6), plain());
    settle(&mut rt);
    assert_eq!(focused(&rt), "live-preview", "F6 from the find bar: the preview");
    press(&mut rt, Key::Function(6), plain());
    settle(&mut rt);
    assert_eq!(focused(&rt), super::EDITOR_KEY, "then the field");
    press(&mut rt, Key::Function(6), plain());
    settle(&mut rt);
    assert_eq!(focused(&rt), "find-needle", "then the find bar's first field");
    press(&mut rt, Key::Function(6), shift());
    settle(&mut rt);
    assert_eq!(focused(&rt), super::EDITOR_KEY, "Shift+F6 goes back");
    assert_eq!(text(&rt), "one two\n", "F6 typed nothing");
}

#[test]
fn editor_keys_escape_in_the_find_bar_gives_the_field_the_keyboard() {
    let mut rt = editing("a.txt", "one\ntwo\n");
    press(&mut rt, Key::Command('f'), ctrl());
    settle(&mut rt);
    assert_eq!(focused(&rt), "find-needle");
    press(&mut rt, Key::Escape, plain());
    settle(&mut rt);
    assert!(!rt.state.viewer.ui.as_ref().unwrap().editor_view.find.open, "Esc closed find");
    assert!(rt.state.viewer.ui.as_ref().unwrap().editor.is_some(), "the editor stays");
    assert_eq!(focused(&rt), super::EDITOR_KEY, "the keyboard is back in the field");
}

#[test]
fn editor_keys_f2_and_delete_edit_the_text() {
    let mut rt = editing("a.txt", "abc\n");
    edit_mut(&mut rt).set_caret(0);
    settle(&mut rt);
    press(&mut rt, Key::Function(2), plain());
    press(&mut rt, Key::Delete, plain());
    settle(&mut rt);
    assert!(rt.state.viewer.ui.as_ref().is_some_and(|ui| ui.editor.is_some()), "still editing the same file");
    assert_eq!(text(&rt), "bc\n", "Delete removed the character after the caret, nothing was renamed or trashed");
}

#[test]
fn unsaved_question_traps_focus_closes_on_escape_and_gives_the_field_back() {
    let mut rt = editing("a.txt", "abc\n");
    edit_mut(&mut rt).set_caret(0);
    press(&mut rt, Key::Text("x".into()), plain());
    settle(&mut rt);
    assert!(rt.state.viewer.ui.as_ref().unwrap().dirty(), "the edit is unsaved");
    rt.update(|s, _| {
        assert!(super::unsaved_guard::ask(s, super::unsaved_guard::Then::Quit));
    });
    settle(&mut rt);
    let tree = rt.accessibility_json();
    assert!(tree.to_string().contains("\"modal\""), "the question is modal: {tree}");
    // Tab stays inside the question.
    for _ in 0..4 {
        press(&mut rt, Key::Tab, plain());
        settle(&mut rt);
        assert!(focused(&rt).starts_with("btn-"), "Tab stays on the question's buttons, not {}", focused(&rt));
    }
    press(&mut rt, Key::Escape, plain());
    settle(&mut rt);
    assert!(rt.state.viewer.leave.is_none(), "Esc is Cancel");
    assert!(rt.state.viewer.ui.as_ref().unwrap().dirty(), "nothing was dropped");
    assert_eq!(focused(&rt), super::EDITOR_KEY, "the keyboard is back in the editor");
}
