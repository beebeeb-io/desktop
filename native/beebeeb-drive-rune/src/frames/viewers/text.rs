//! Text and code (DW-1512, DW-1527): the file in JetBrains Mono with line numbers, each run in
//! the ink the source gave it; and the toolbar controls and the banner of a text file.

use rune::{Align, Color, El, Icon, Span, col, el, row, text_input};

use super::chrome::{Dot, Status};
use super::source::{Cut, Ink, OpenFile, Run, TextDoc, count_words};
use super::{Intent, icons, style, ui};
use crate::State;
use crate::theme::tokens::Theme;
use crate::ui::toolbar::{ToolState, tool_button};
use crate::ui::{au, kit};

/// The gutter: the line number's box, 16 pt of padding on both sides.
const GUTTER_W: f32 = 56.0;
const LINE_H: f32 = 21.0;

fn ink(ink: Ink, t: &Theme) -> Color {
    match ink {
        Ink::Strong => t.text,
        Ink::Plain => t.text_secondary,
        Ink::Faint => t.text_tertiary,
        Ink::Green => au::green_text(t),
        Ink::Red => au::red_text(t),
    }
}

/// One run: a label as wide as its characters (the type is monospaced; spaces at either end
/// keep their width).
fn run_el(key: String, run: &Run, t: &Theme) -> El {
    let text_style = if run.bold { style::CODE_BOLD } else { style::CODE };
    let color = ink(run.ink, t);
    let width = run.text.chars().count() as f32 * rune::text::advance("0", style::CODE);
    let mut label = kit::label(key, run.text.clone(), text_style, color).w(width);
    let mut span = Span::new(0, run.text.len());
    if run.italic {
        span = span.italic();
    }
    if run.underline {
        span = span.underline(t.separator_strong);
    }
    if run.italic || run.underline {
        label = label.spans(vec![span]);
    }
    label
}

/// One line: its number in the gutter, then its runs.
pub fn line(i: usize, runs: &[Run], current: bool, t: &Theme) -> El {
    let mut text = row().key(format!("line-{i}-text")).h(LINE_H);
    for (j, run) in runs.iter().enumerate() {
        text = text.child(run_el(format!("run-{i}-{j}"), run, t));
    }
    let mut line = row()
        .key(format!("line-{i}"))
        .w_full()
        .h(LINE_H)
        .child(
            kit::label(
                format!("ln-{i}"),
                (i + 1).to_string(),
                style::CODE,
                if current { t.text_secondary } else { t.text_tertiary },
            )
            .w(GUTTER_W)
            .px(16.0)
            .align(Align::End),
        )
        .child(text);
    if current {
        line = line.fill(style::current_line(t));
    }
    line
}

/// One line that wraps at the window's width (Wrap): a wrapping row of words, each in its
/// run's ink, so the row is as tall as the rows its words take (at rest: Rune moves a keyed row to its new height over a few frames). A word longer than the room
/// (no space in it) runs past the edge and is clipped, as `white-space: pre-wrap` does.
fn wrapped_line(i: usize, runs: &[Run], t: &Theme) -> El {
    // The row itself wraps, and the number sits over the row's own left padding.
    let mut line = row()
        .key(format!("line-{i}"))
        .w_full()
        .pl(GUTTER_W)
        .pr(16.0)
        .flex_wrap()
        .items_start()
        .child(
            kit::label(format!("ln-{i}"), (i + 1).to_string(), style::CODE, t.text_tertiary)
                .w(GUTTER_W)
                .px(16.0)
                .align(Align::End)
                .absolute(0.0, 0.0),
        );
    let mut n = 0;
    for run in runs {
        for word in run.text.split_inclusive(' ') {
            let part = Run {
                text: word.to_string(),
                ..run.clone()
            };
            line = line.child(run_el(format!("run-{i}-w{n}"), &part, t).h(LINE_H));
            n += 1;
        }
    }
    line
}

/// The text of a file, read-only. `wrap`: long lines wrap instead of running past the edge.
pub fn body(doc: &TextDoc, wrap: bool, t: &Theme) -> El {
    let mut lines = col()
        .key("viewer-body")
        .role("document")
        .grow(1.0)
        .shrink()
        .h_full()
        .py(12.0)
        .fill(t.bg)
        .clip_rounded(0.0)
        // A file longer than the window is read by scrolling (the wheel, a touchpad).
        .min_h(0.0)
        .scroll(rune::ScrollSpec { stick_bottom: false });
    for (i, runs) in doc.lines.iter().enumerate() {
        lines = lines.child(if wrap {
            wrapped_line(i, runs, t)
        } else {
            line(i, runs, false, t)
        });
    }
    lines
}

/// One half of a small segmented control (`.kit-seg-sm [role=tab]`).
fn seg_tab(key: String, label: &str, on: bool, t: &Theme) -> El {
    let mut tab = row()
        .key(key.clone())
        .role("tab")
        .label(label)
        .checked(on)
        .h(24.0)
        .px(10.0)
        .center()
        .radius(8.0)
        .focusable(true)
        .focus_ring(2.0, 0.0, t.accent_deep)
        .child(style::fitted(
            format!("{key}-label"),
            label,
            style::SEG,
            if on { t.text } else { t.text_secondary },
        ));
    if on {
        tab = tab
            .fill(t.card)
            .box_shadow(kit::drop(1.0, 2.0, t.shadow_raised))
            .box_shadow(kit::ring(kit::HAIR, t.separator));
    } else {
        tab = tab.hover(t.hover).press(t.press);
    }
    tab
}

/// A small segmented control of two choices (`.kit-seg.kit-seg-sm`). The caller adds each
/// tab's `.on_press(..)`. Keys: `seg`, `seg-<i>`, `seg-<i>-label`.
pub fn seg(name: &str, tabs: [(&str, bool); 2], t: &Theme) -> (El, [El; 2]) {
    let well = row()
        .key("seg")
        .role("tablist")
        .label(name)
        .padding(2.0)
        .radius(10.0)
        .fill(t.field)
        .box_shadow(kit::ring_inset(t.separator));
    let [a, b] = tabs;
    (
        well,
        [
            seg_tab("seg-0".into(), a.0, a.1, t),
            seg_tab("seg-1".into(), b.0, b.1, t),
        ],
    )
}

/// A small outline button of the toolbar with an icon, a word and a mono shortcut after it
/// (`Edit Ctrl+E`). Keys: `<key>`, `<key>-icon`, `<key>-label`, `<key>-hint`.
pub fn tool_action(key: &str, label: &str, icon: Option<&'static Icon>, hint: Option<&str>, t: &Theme) -> El {
    let mut b = row()
        .key(key.to_string())
        .role("button")
        .label(label)
        .h(22.0)
        .px(12.0)
        .gap(7.0)
        .center()
        .radius(6.0)
        .fill(t.paper)
        .border(1.0, t.card_border)
        .border_layout()
        .hover(t.paper2)
        .press(t.paper3)
        .focusable(true)
        .focus_ring(2.0, 2.0, t.accent_deep)
        .cursor(rune::Cursor::Pointer);
    if let Some(icon) = icon {
        b = b.child(kit::glyph(format!("{key}-icon"), icon, 14.0, t.text));
    }
    let mut words = row()
        .key(format!("{key}-words"))
        .items_end()
        .gap(4.0)
        .child(style::fitted(format!("{key}-label"), label, style::BUTTON_SM, t.text));
    if let Some(hint) = hint {
        b = b.shortcut_hint(hint);
        words = words.child(style::fitted(format!("{key}-hint"), hint, style::BUTTON_HINT, t.text).opacity(0.6));
    }
    b.child(words)
}

/// The toolbar's controls of a text file.
pub fn tools(doc: &TextDoc, wrap: bool, t: &Theme) -> Vec<El> {
    if doc.truncated.is_some() {
        // Too big to edit here: no Edit, no Copy of a part; the way out is another app.
        return vec![
            tool_action("tb-btn-0", "Open in another app\u{2026}", None, None, t).on_press(|s: &mut State| {
                if let Some(ui) = ui(s) {
                    ui.intent(Intent::OpenInAnotherApp);
                }
            }),
        ];
    }
    let (well, [wrap_tab, no_wrap_tab]) = seg("Long lines", [("Wrap", wrap), ("No wrap", !wrap)], t);
    let set_wrap = |on: bool| {
        move |s: &mut State| {
            if let Some(ui) = ui(s) {
                ui.wrap = on;
            }
        }
    };
    vec![
        well.child(wrap_tab.on_press(set_wrap(true)))
            .child(no_wrap_tab.on_press(set_wrap(false))),
        tool_button("tool-0", "Copy", &icons::COPY, 14.0, ToolState::Idle, t)
            .shortcut_hint("Ctrl+C")
            .on_press_tx(super::copy_text),
        tool_action("tb-btn-0", "Edit", Some(&icons::EDIT), Some("Ctrl+E"), t).on_press(|s: &mut State| {
            if let Some(ui) = ui(s) {
                ui.edit();
            }
        }),
    ]
}

/// The toolbar's controls of a Markdown note: Rendered / Source, Edit.
pub fn md_tools(source: bool, t: &Theme) -> Vec<El> {
    let (well, [rendered, as_source]) = seg("View", [("Rendered", !source), ("Source", source)], t);
    let set = |on: bool| {
        move |s: &mut State| {
            if let Some(ui) = ui(s) {
                ui.md_source = on;
            }
        }
    };
    vec![
        well.child(rendered.on_press(set(false)))
            .child(as_source.on_press(set(true))),
        tool_action("tb-btn-0", "Edit", Some(&icons::EDIT), Some("Ctrl+E"), t).on_press(|s: &mut State| {
            if let Some(ui) = ui(s) {
                ui.edit();
            }
        }),
    ]
}

/// The editor (DW-1523): the text in JetBrains Mono with line numbers, the line the caret is on
/// marked. The coloured lines are drawn as the reader draws them; over them lies the text
/// field that takes the typing (its own glyphs clear, its caret and selection showing), set in
/// the same type on the same lines, so every key, the pointer, the clipboard, undo and the
/// input method are the field's.
pub fn editor(editor: &super::Editor, lines: &[Vec<Run>], t: &Theme) -> El {
    // The line the caret is on, while the editor has the keyboard (not under a dialog).
    let current = editor.edit.focused.then(|| editor.caret_line());
    let mut under = col().key("editor-lines").w_full();
    for (i, runs) in lines.iter().enumerate() {
        if current == Some(i) {
            // On the caret's line the marks are not dimmed: they are what is being typed.
            let shown: Vec<Run> = runs
                .iter()
                .map(|run| Run {
                    ink: if run.ink == Ink::Faint { Ink::Plain } else { run.ink },
                    ..run.clone()
                })
                .collect();
            under = under.child(line(i, &shown, true, t));
        } else {
            under = under.child(line(i, runs, false, t));
        }
    }
    // The caret as the board draws it: 1.5 wide, 15 high, one point after the text.
    let at = editor.edit.focus.min(editor.edit.text.len());
    let column = editor.edit.text[..at]
        .rsplit('\n')
        .next()
        .unwrap_or_default()
        .chars()
        .count();
    let caret_x = GUTTER_W + column as f32 * rune::text::advance("0", style::CODE) + 1.0;
    let mut field = el().key("editor-field").w_full().child(under).child(
        text_input(&editor.edit, style::CODE)
            .key(super::EDITOR_KEY)
            .label("Text")
            .shortcut_hint("Ctrl+S")
            .color(Color::TRANSPARENT)
            .caret(Color::TRANSPARENT, t.selection)
            .on_edit(super::editor_edit)
            .pin(Some(GUTTER_W), Some(0.0), Some(0.0), Some(0.0)),
    );
    if let Some(current) = current {
        field = field.child(
            el().key(format!("caret-{current}"))
                .ime_cursor()
                .absolute(caret_x, current as f32 * LINE_H + 3.0)
                .size(1.5, 15.0)
                .radius(1.0)
                .fill(t.accent_deep),
        );
    }
    col()
        .key("viewer-body")
        .role("document")
        .grow(1.0)
        .shrink()
        .min_w(0.0)
        .h_full()
        .py(12.0)
        .fill(t.bg)
        .clip_rounded(0.0)
        .child(field)
}

/// The toolbar's control of the editor: Save, a plain button with the lock (New stays the
/// window's one Amber control).
pub fn editor_tools(t: &Theme) -> Vec<El> {
    vec![
        tool_action("tb-btn-0", "Save", Some(&icons::LOCK), Some("Ctrl+S"), t).on_press(|s: &mut State| {
            if let Some(ui) = ui(s) {
                ui.save();
            }
        }),
    ]
}

/// DW-1519: another device saved a newer version while this one is open. The version on
/// screen stays until Show it.
pub fn newer_banner(file: &OpenFile, t: &Theme) -> Option<El> {
    let newer = file.newer.as_ref()?;
    let words = format!("A newer version arrived from {}, {}.", newer.from, newer.at);
    // The time is a figure: mono.
    let time = words.len() - newer.at.len() - 1;
    let show = super::chrome::button("banner-btn", "Show it", super::chrome::ButtonKind::Outline, true, t).on_press(
        |s: &mut State| {
            if let Some(ui) = ui(s) {
                ui.show_newer();
            }
        },
    );
    Some(banner_spans(
        &icons::REFRESH,
        &words,
        vec![Span::new(time, time + newer.at.len()).mono()],
        Some(show),
        t,
    ))
}

/// The note over a file that opened read-only (DW-1527).
pub fn banner(doc: &TextDoc, t: &Theme) -> Option<El> {
    let words = match doc.truncated?.why {
        Cut::OverTwoMb => "Read-only: this file is larger than 2 MB. Open it with another app to edit it.",
        Cut::TooLong => {
            "Read-only: this file is too long to show whole here. Open it with another app to see all of it."
        }
    };
    Some(banner_el(&icons::INFO, words, None, t))
}

/// A banner under the toolbar (`.mk-banner`): an icon, a sentence, and a button if given.
/// Keys: `banner`, `banner-icon`, `banner-text`; the button is the caller's.
pub fn banner_el(icon: &'static Icon, words: &str, action: Option<El>, t: &Theme) -> El {
    banner_spans(icon, words, Vec::new(), action, t)
}

/// [`banner_el`] with spans in its sentence (a figure in mono).
pub fn banner_spans(icon: &'static Icon, words: &str, spans: Vec<Span>, action: Option<El>, t: &Theme) -> El {
    let mut b = row()
        .key("banner")
        .role("status")
        .mx(14.0)
        .mt(10.0)
        .pl(12.0)
        .pr(10.0)
        .py(9.0)
        .gap(10.0)
        .center_y()
        .radius(10.0)
        .fill(style::banner(t))
        .child(kit::glyph("banner-icon", icon, 16.0, t.text_secondary))
        .child({
            let sentence = kit::label("banner-text", words, style::BANNER, t.text_secondary).grow(1.0);
            if spans.is_empty() {
                sentence
            } else {
                sentence.spans(spans)
            }
        });
    if let Some(action) = action {
        b = b.child(action);
    }
    b
}

/// The status line of a text file.
/// What the status line says at the right for a text that is shown whole by a source that cannot save an edit.
pub const READ_ONLY_HINT: &str = "Space for Preview \u{00B7} read-only here";

pub fn status(doc: &TextDoc) -> Status {
    Status {
        dot: Dot::Green,
        text: "Decrypted on this PC \u{00B7} the key never left it".into(),
        details: match doc.truncated {
            Some(cut) => format!(
                "Showing the first {} of {} lines",
                count_words(cut.shown),
                count_words(cut.total)
            ),
            None => "Space for Preview \u{00B7} Ctrl+E to edit".into(),
        },
    }
}
