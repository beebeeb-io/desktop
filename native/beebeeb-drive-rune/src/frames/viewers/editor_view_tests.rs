//! The editor's seam (ux-12): gutter numbers that follow wrapped lines, a field that paints coloured spans, and
//! the live preview's toggle and pane. Every name and text is invented.

use rune::{Env, Key, KeyEvent, Mods, Runtime};

use super::super::source::{Body, Details, MdDoc, OpenError, OpenFile, TextDoc, ViewerSource, plain_lines};
use super::super::{EDITOR_KEY, open_from};
use super::live_preview::{PreviewKind, kind_of};
use crate::theme::tokens;
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

fn info(rt: &Runtime<State>, key: &str) -> rune::ElementInfo {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
        .unwrap_or_else(|| panic!("no element keyed {key}"))
}

fn has(rt: &Runtime<State>, key: &str) -> bool {
    rt.elements().iter().any(|e| e.key.as_deref() == Some(key) && !e.exiting)
}

#[test]
fn editor_numbers_three_lines_and_the_field_carries_coloured_spans() {
    let rt = editing("main.rs", "fn main() {\n    let x = 1;\n}");
    for i in 0..3 {
        assert_eq!(info(&rt, &format!("ln-{i}")).text.as_deref(), Some((i + 1).to_string().as_str()));
    }
    // The reader's text ends each line with a break: the caret's empty last line is numbered, nothing more.
    let lines = rt.state.viewer.ui.as_ref().unwrap().editor.as_ref().unwrap().edit.text.split('\n').count();
    assert!(lines <= 4 && !has(&rt, &format!("ln-{lines}")));
    assert!(has(&rt, EDITOR_KEY));
    let ui = rt.state.viewer.ui.as_ref().unwrap();
    let t = tokens::theme(false);
    let spans = super::field_spans(ui, ui.editor.as_ref().unwrap(), &t);
    assert!(spans.iter().any(|s| s.color.is_some()), "the field paints coloured spans: {spans:?}");
}

#[test]
fn editor_numbers_stay_level_with_wrapped_lines() {
    let long = "word ".repeat(400);
    let rt = editing("notes.txt", &format!("{long}\nsecond\nthird"));
    let (one, two) = (info(&rt, "ln-0").rect, info(&rt, "ln-1").rect);
    let first_row = info(&rt, "line-0").rect;
    assert!(first_row[3] >= 42.0, "line 1 wraps onto more rows: {first_row:?}");
    let second_row = info(&rt, "line-1").rect;
    assert_eq!(two[1], second_row[1], "number 2 sits at the top of logical line 2");
    assert!(two[1] - one[1] >= first_row[3] - 0.5, "number 2 is below all of line 1's rows");
}

#[test]
fn editor_preview_toggle_and_pane_only_for_a_preview_kind() {
    assert_eq!(kind_of("a.md"), Some(PreviewKind::Markdown));
    assert_eq!(kind_of("a.csv"), Some(PreviewKind::Table));
    assert_eq!(kind_of("a.svg"), Some(PreviewKind::Svg));
    assert_eq!(kind_of("a.rs"), None);

    let rt = editing("main.rs", "fn main() {}");
    assert!(!has(&rt, "tb-preview"));
    assert!(!has(&rt, "live-preview"));

    let mut rt = editing("notes.md", "# Invented\n\nSome *text*.");
    assert!(has(&rt, "tb-preview"));
    assert!(has(&rt, "live-preview"));
    rt.settle_all();
    rt.frame(0.0);
    let r = info(&rt, "tb-preview").rect;
    let (x, y) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(0.0);
    rt.frame(0.0);
    rt.settle_all();
    assert!(!rt.state.viewer.ui.as_ref().unwrap().editor_view.preview_open);
    assert!(!has(&rt, "live-preview"), "the toggle hides the pane");
}

#[test]
fn editor_opens_csv_and_svg_as_text() {
    use super::super::kinds::{Viewer, viewer_of};
    assert_eq!(viewer_of("art.svg"), Some(Viewer::Text));
    let rt = editing("rows.csv", "a,b\n1,2");
    assert!(has(&rt, "live-preview"), "a csv edits with its table preview");
}
