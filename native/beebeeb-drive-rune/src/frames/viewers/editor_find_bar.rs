//! Find and replace in the editor (ux-14): the bar in the banner slot, the matches as spans, and the steps.
//! The needle and the replacement are rune fields; the toggles live in `FindState::query`.
#![allow(dead_code)] // the key handler (ux-15) calls open, close and step.

use std::cell::Cell;

use rune::{El, Key, KeyEvent, Span, row, text_input};

use super::super::{Ui, style};
use super::EditorView;
use crate::State;
use crate::text_kit::find::{self, FindQuery};
use crate::theme::tokens::Theme;
use crate::ui::{kit, pill};

thread_local! {
    /// The match count of the last step or paint: `bar` has no text to count in.
    static COUNT: Cell<usize> = const { Cell::new(0) };
}

/// The query as the fields hold it now.
fn query(view: &EditorView) -> FindQuery {
    FindQuery {
        needle: view.find.needle.text.clone(),
        ..view.find.query.clone()
    }
}

fn sync(view: &mut EditorView, text: &str) -> Vec<std::ops::Range<usize>> {
    view.find.query.needle = view.find.needle.text.clone();
    let ms = find::find_all(text, &view.find.query);
    COUNT.with(|c| c.set(ms.len()));
    if view.find.current.is_some_and(|i| i >= ms.len()) {
        view.find.current = None;
    }
    ms
}

/// "1 of 2", "No results", or nothing while the needle is empty.
pub fn status(view: &EditorView) -> String {
    if view.find.needle.text.is_empty() {
        return String::new();
    }
    let n = COUNT.with(|c| c.get());
    match (n, view.find.current) {
        (0, _) => "No results".to_string(),
        (n, Some(i)) => format!("{} of {n}", i + 1),
        (n, None) => format!("{n} found"),
    }
}

/// Open find (or find and replace); a selection within one line seeds the needle.
pub fn open(view: &mut EditorView, edit: &mut rune::TextEdit, replace: bool) {
    view.find.open = true;
    view.find.replace_open = replace;
    let sel = edit.selected();
    if !sel.is_empty() && !sel.contains('\n') {
        let sel = sel.to_string();
        view.find.needle.replace_text(sel);
    }
    view.find.needle.focused = true;
    view.find.replacement.focused = false;
    edit.focused = false;
    let text = edit.text.clone();
    sync(view, &text);
}

/// Close find and give the focus back to the editor.
pub fn close(view: &mut EditorView, edit: &mut rune::TextEdit) {
    view.find.open = false;
    view.find.replace_open = false;
    view.find.needle.focused = false;
    view.find.replacement.focused = false;
    edit.focused = true;
}

/// Select the next (or previous) match from the caret and make it the current one.
pub fn step(view: &mut EditorView, edit: &mut rune::TextEdit, backwards: bool) {
    let ms = sync(view, &edit.text);
    let (a, b) = edit.range();
    let caret = if backwards { a } else { b };
    match find::next_match(&ms, caret, backwards) {
        Some(i) => {
            edit.set_selection(ms[i].start, ms[i].end);
            view.find.current = Some(i);
        }
        None => view.find.current = None,
    }
}

/// Replace the current match (when the selection is one), then step.
pub fn replace_current(view: &mut EditorView, edit: &mut rune::TextEdit) {
    let ms = sync(view, &edit.text);
    let (a, b) = edit.range();
    if ms.iter().any(|m| m.start == a && m.end == b) {
        let with = view.find.replacement.text.clone();
        edit.insert(&with);
        edit.mark_edited();
    }
    step(view, edit, false);
}

/// Replace every match; the words for the toast ("Replaced 2").
pub fn replace_all(view: &mut EditorView, edit: &mut rune::TextEdit) -> String {
    view.find.query.needle = view.find.needle.text.clone();
    let (new, n) = find::replace_all(&edit.text, &view.find.query, &view.find.replacement.text);
    if n > 0 {
        let caret = edit.range().1.min(new.len());
        edit.set_selection(0, edit.text.len());
        edit.insert(&new);
        edit.set_caret(caret);
        edit.mark_edited();
    }
    view.find.current = None;
    let text = edit.text.clone();
    sync(view, &text);
    format!("Replaced {n}")
}

/// The matches as background spans over `text`: all in the highlight, the current one in the selection.
pub fn match_spans(view: &EditorView, text: &str, t: &Theme) -> Vec<Span> {
    if !view.find.open {
        return Vec::new();
    }
    let ms = find::find_all(text, &query(view));
    COUNT.with(|c| c.set(ms.len()));
    ms.iter()
        .enumerate()
        .map(|(i, m)| {
            let c = if view.find.current == Some(i) {
                t.selection
            } else {
                t.highlight
            };
            Span::new(m.start, m.end).bg(c, 0.0)
        })
        .collect()
}

/// Run `f` on the open file's editor and its find state.
fn with(s: &mut State, f: impl FnOnce(&mut Ui, &mut rune::TextEdit) -> Option<String>) {
    let Some(ui) = super::super::ui(s) else { return };
    let Some(mut editor) = ui.editor.take() else { return };
    if let Some(words) = f(ui, &mut editor.edit) {
        ui.toast = Some(words);
    }
    ui.editor = Some(editor);
}

fn go(s: &mut State, backwards: bool) {
    with(s, |ui, e| {
        step(&mut ui.editor_view, e, backwards);
        None
    });
}

fn needle_edit(s: &mut State) -> &mut rune::TextEdit {
    match s.viewer.ui.as_mut() {
        Some(ui) => &mut ui.editor_view.find.needle,
        None => &mut s.viewer.stray,
    }
}

fn replacement_edit(s: &mut State) -> &mut rune::TextEdit {
    match s.viewer.ui.as_mut() {
        Some(ui) => &mut ui.editor_view.find.replacement,
        None => &mut s.viewer.stray,
    }
}

/// Enter steps (Shift+Enter backwards), Esc closes.
fn needle_key(s: &mut State, ev: &KeyEvent, _tx: &mut rune::Tx) -> bool {
    match ev.key {
        Key::Enter => {
            go(s, ev.mods.shift);
            true
        }
        Key::Escape => {
            with(s, |ui, e| {
                close(&mut ui.editor_view, e);
                None
            });
            true
        }
        _ => false,
    }
}

fn chip(key: &str, label: &str, on: bool, t: &Theme) -> El {
    let tone = if on { pill::Tone::Accent } else { pill::Tone::Neutral };
    pill::pill(key, label, tone, false, t)
        .role("button")
        .h(22.0)
        .focusable(true)
        .checked(on)
        .hover(t.paper2)
        .press(t.paper3)
        .cursor(rune::Cursor::Pointer)
}

fn field(key: &'static str, edit: &rune::TextEdit, t: &Theme) -> El {
    let f = text_input(edit, style::CODE)
        .key(key)
        .label(if key == "find-needle" { "Find" } else { "Replace with" })
        .single_line()
        .color(t.text)
        .caret(t.accent_deep, t.selection)
        .w(220.0)
        .h(26.0)
        .px(8.0)
        .radius(6.0)
        .fill(t.paper)
        .border(1.0, t.card_border)
        .border_layout();
    if key == "find-needle" {
        f.on_edit(needle_edit).on_input_key(needle_key)
    } else {
        f.on_edit(replacement_edit)
    }
}

/// The find bar, in the window's banner slot while find is open.
pub fn bar(view: &EditorView, t: &Theme) -> Option<El> {
    if !view.find.open {
        return None;
    }
    let q = &view.find.query;
    let line1 = row()
        .key("find-row")
        .gap(6.0)
        .center_y()
        .child(field("find-needle", &view.find.needle, t))
        .child(kit::label("find-status", status(view), style::SUB, t.text_secondary))
        .child(chip("find-prev", "Previous", false, t).on_press(|s: &mut State| go(s, true)))
        .child(chip("find-next", "Next", false, t).on_press(|s: &mut State| go(s, false)))
        .child(chip("find-case", "Aa", q.match_case, t).on_press(|s: &mut State| {
            with(s, |ui, _| {
                let q = &mut ui.editor_view.find.query;
                q.match_case = !q.match_case;
                None
            })
        }))
        .child(chip("find-word", "ab", q.whole_word, t).on_press(|s: &mut State| {
            with(s, |ui, _| {
                let q = &mut ui.editor_view.find.query;
                q.whole_word = !q.whole_word;
                None
            })
        }))
        .child(chip("find-close", "\u{00d7}", false, t).on_press(|s: &mut State| {
            with(s, |ui, e| {
                close(&mut ui.editor_view, e);
                None
            })
        }));
    let mut b = rune::col()
        .key("find-bar")
        .mx(14.0)
        .mt(10.0)
        .px(12.0)
        .py(8.0)
        .gap(6.0)
        .radius(10.0)
        .fill(style::banner(t))
        .child(line1);
    if view.find.replace_open {
        b = b.child(
            row()
                .key("find-replace-row")
                .gap(6.0)
                .center_y()
                .child(field("find-replacement", &view.find.replacement, t))
                .child(chip("find-replace", "Replace", false, t).on_press(|s: &mut State| {
                    with(s, |ui, e| {
                        replace_current(&mut ui.editor_view, e);
                        None
                    })
                }))
                .child(
                    chip("find-replace-all", "Replace all", false, t)
                        .on_press(|s: &mut State| with(s, |ui, e| Some(replace_all(&mut ui.editor_view, e)))),
                ),
        );
    }
    Some(b)
}

#[cfg(test)]
#[path = "editor_find_bar_tests.rs"]
mod tests;
