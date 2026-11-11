//! Text and code (DW-1512, DW-1527): the file in JetBrains Mono with line numbers, each run in
//! the ink the source gave it; and the toolbar controls and the banner of a text file.

use rune::{Align, Color, El, Icon, Span, col, row};

use super::chrome::{Dot, Status};
use super::source::{Ink, Run, TextDoc, count_words};
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
            kit::label(format!("ln-{i}"), (i + 1).to_string(), style::CODE, t.text_tertiary)
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

/// One line that wraps at the window's width (Wrap): one text in which each run keeps its ink.
fn wrapped_line(i: usize, runs: &[Run], t: &Theme) -> El {
    let mut content = String::new();
    let mut spans = Vec::new();
    for run in runs {
        let mut span = Span::new(content.len(), content.len() + run.text.len()).color(ink(run.ink, t));
        if run.bold {
            span = span.weight(650.0);
        }
        if run.italic {
            span = span.italic();
        }
        if run.underline {
            span = span.underline(t.separator_strong);
        }
        spans.push(span);
        content.push_str(&run.text);
    }
    // KNOWN DEFECT (PR body, "Not done"): the text wraps at the room's width (measured 986 x 63
    // for a 400-character line), but this row keeps one row's height, so the lines after a
    // wrapped line draw over its second row. Five constructions were tried (the group's wrap
    // test names what each measured); the row's height must follow the text before Wrap is done.
    row()
        .key(format!("line-{i}"))
        .w_full()
        .min_h(LINE_H)
        .items_start()
        .child(
            kit::label(format!("ln-{i}"), (i + 1).to_string(), style::CODE, t.text_tertiary)
                .w(GUTTER_W)
                .px(16.0)
                .align(Align::End),
        )
        .child(
            col().key(format!("line-{i}-room")).grow(1.0).shrink().pr(16.0).child(
                rune::rich(content, spans)
                    .key(format!("line-{i}-text"))
                    .font(style::CODE)
                    .color(t.text_secondary)
                    .wrap()
                    .w_full(),
            ),
        )
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
        .clip_rounded(0.0);
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

/// The note over a file that opened read-only (DW-1527).
pub fn banner(doc: &TextDoc, t: &Theme) -> Option<El> {
    doc.truncated?;
    Some(banner_el(
        &icons::INFO,
        "Read-only: this file is larger than 2 MB. Open it with another app to edit it.",
        None,
        t,
    ))
}

/// A banner under the toolbar (`.mk-banner`): an icon, a sentence, and a button if given.
/// Keys: `banner`, `banner-icon`, `banner-text`; the button is the caller's.
pub fn banner_el(icon: &'static Icon, words: &str, action: Option<El>, t: &Theme) -> El {
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
        .child(kit::label("banner-text", words, style::BANNER, t.text_secondary).grow(1.0));
    if let Some(action) = action {
        b = b.child(action);
    }
    b
}

/// The status line of a text file.
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
