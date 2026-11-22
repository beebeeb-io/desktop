//! The editor's seam (ux-12): what the editor shows besides the field's own text. The gutter numbers and the
//! current-line fill under the field ([`underlay`]), the spans the field paints its text with ([`field_spans`]),
//! and the layout of the editor with its find bar and live preview ([`parts`]). Every later editor feature fills
//! its own stub file (`editor_find_bar.rs`, `live_*.rs`).

use rune::{Align, Color, El, Span, col, el, row};

use super::source::{Ink, Run};
use super::{Editor, Ui, style, text};
use crate::text_kit::find::FindQuery;
use crate::theme::scale;
use crate::theme::tokens::Theme;
use crate::ui::kit;

#[path = "editor_find_bar.rs"]
pub mod editor_find_bar;
#[path = "live_html.rs"]
mod live_html;
#[path = "live_json.rs"]
mod live_json;
#[path = "live_md.rs"]
mod live_md;
#[path = "live_preview.rs"]
pub mod live_preview;
#[path = "live_svg.rs"]
mod live_svg;
#[path = "live_table.rs"]
mod live_table;

#[cfg(test)]
#[path = "editor_view_tests.rs"]
mod tests;

/// The gutter: the line number's box, 16 pt of padding on both sides.
pub const GUTTER_W: f32 = 56.0;

/// The editor's F6 stops (`ui::focus::pane`), in the order F6 visits them: the field, the find bar while it is
/// open, the live preview while it shows.
pub const FIELD_PANE: &str = "editor-field";
pub const FIND_PANE: &str = "find-bar";
pub const PREVIEW_PANE: &str = "live-preview-pane";

/// `inner` inside the F6 stop `pane`: F6 / Shift+F6 on a control in it go to the editor's next / previous stop
/// before the window's pane cycle sees the key, so the field, the find bar and the preview cycle among themselves.
pub fn stop(inner: El, pane: &'static str) -> El {
    let inner = inner.on_key(move |s: &mut crate::State, ev: &rune::KeyEvent, tx: &mut rune::Tx| {
        let Some(back) = crate::ui::focus::f6(ev) else { return false };
        super::editor_f6(s, pane, back, tx);
        true
    });
    crate::ui::focus::pane(el().child(inner), pane)
}

/// The stop after (`back`: before) `from`, wrapping; `None` when the field is the only one.
pub fn f6_stop(ui: &Ui, from: &str, back: bool) -> Option<&'static str> {
    let mut stops = vec![FIELD_PANE];
    if ui.editor_view.find.open {
        stops.push(FIND_PANE);
    }
    if ui.editor_view.preview_open && live_preview::kind_of(&ui.file.name).is_some() {
        stops.push(PREVIEW_PANE);
    }
    let n = stops.len();
    if n < 2 {
        return None;
    }
    let i = stops.iter().position(|k| *k == from).unwrap_or(0);
    Some(stops[if back { (i + n - 1) % n } else { (i + 1) % n }])
}

/// What the editor shows around its text, kept on [`Ui`] across edits.
pub struct EditorView {
    pub preview_open: bool,
    #[allow(dead_code)] // ux-14 reads it.
    pub find: FindState,
}

impl Default for EditorView {
    fn default() -> Self {
        EditorView {
            preview_open: true,
            find: FindState::default(),
        }
    }
}

/// Find and replace in the editor (ux-14 fills it).
#[derive(Default)]
#[allow(dead_code)] // ux-14 reads it.
pub struct FindState {
    pub open: bool,
    pub replace_open: bool,
    pub query: FindQuery,
    pub needle: rune::TextEdit,
    pub replacement: rune::TextEdit,
    pub current: Option<usize>,
}

/// The line the caret is on, from 0.
fn caret_line(edit: &rune::TextEdit) -> usize {
    let at = edit.focus.min(edit.text.len());
    edit.text.as_bytes()[..at].iter().filter(|b| **b == b'\n').count()
}

/// The gutter numbers and the current-line fill, under the field. Each logical line is an invisible wrapped copy
/// of itself as wide as the field, in the field's type, so its row is as tall as the line is on screen and every
/// number stays level with its line when lines wrap.
pub fn underlay(edit: &rune::TextEdit, _file_name: &str, t: &Theme) -> El {
    let current = edit.focused.then(|| caret_line(edit));
    let mut lines = col().key("editor-lines").w_full();
    for (i, line) in edit.text.split('\n').enumerate() {
        let on = current == Some(i);
        // An empty line still takes one row.
        let copy = if line.is_empty() { " " } else { line };
        let mut r = row()
            .key(format!("line-{i}"))
            .w_full()
            .items_start()
            .child(
                kit::label(
                    format!("ln-{i}"),
                    (i + 1).to_string(),
                    style::CODE,
                    if on { t.text_secondary } else { t.text_tertiary },
                )
                .w(GUTTER_W)
                .px(scale::space::S16)
                .align(Align::End),
            )
            .child(
                kit::label(format!("line-{i}-copy"), copy, style::CODE, Color::TRANSPARENT)
                    .wrap()
                    .grow(1.0)
                    .min_w(0.0)
                    .a11y_hidden(),
            );
        if on {
            // Where the caret's line is: the editor scrolls it into view.
            r = r
                .fill(style::current_line(t))
                .child(el().key(format!("caret-{i}")).absolute(GUTTER_W, 0.0).size(1.0, 21.0));
        }
        lines = lines.child(r);
    }
    lines
}

/// A run's colour, as the reader inks it.
fn ink(ink: Ink, t: &Theme) -> Color {
    match ink {
        Ink::Strong => t.text,
        Ink::Plain => t.text_secondary,
        Ink::Faint => t.text_tertiary,
        Ink::Green => crate::ui::au::green_text(t),
        Ink::Red => crate::ui::au::red_text(t),
    }
}

/// A Markdown note's runs (`ViewerSource::colours`) as spans over the whole text. The marks stay dim, except on
/// the caret's line, where they are what is being typed.
pub fn markdown_spans(lines: &[Vec<Run>], caret: Option<usize>, text_len: usize, t: &Theme) -> Vec<Span> {
    let mut out = Vec::new();
    let mut at = 0usize;
    for (i, runs) in lines.iter().enumerate() {
        for run in runs {
            let end = (at + run.text.len()).min(text_len);
            if at < end {
                let shown = if run.ink == Ink::Faint && caret == Some(i) {
                    Ink::Plain
                } else {
                    run.ink
                };
                let mut span = Span::new(at, end).color(ink(shown, t));
                if run.bold {
                    span.weight = Some(650.0);
                }
                if run.italic {
                    span = span.italic();
                }
                if run.underline {
                    span = span.underline(t.separator_strong);
                }
                out.push(span);
            }
            at += run.text.len();
        }
        // The line break.
        at += 1;
    }
    out
}

/// The spans the field paints its text with: Markdown's runs for a note, the highlighter's for any other file,
/// then the find bar's matches.
pub fn field_spans(ui: &Ui, editor: &Editor, t: &Theme) -> Vec<Span> {
    let text = &editor.edit.text;
    let mut spans = if live_preview::kind_of(&ui.file.name) == Some(live_preview::PreviewKind::Markdown) {
        let lines = ui.source.colours(&ui.file, text);
        let caret = editor.edit.focused.then(|| editor.caret_line());
        markdown_spans(&lines, caret, text.len(), t)
    } else {
        crate::text_kit::highlight::spans(&ui.file.name, text, t.dark)
    };
    spans.extend(editor_find_bar::match_spans(&ui.editor_view, text, t));
    spans
}

/// The editor's toolbar controls, banner and body: the editor alone, or beside its live preview.
pub fn parts(ui: &Ui, editor: &Editor, t: &Theme) -> (Vec<El>, Option<El>, El) {
    let name = &ui.file.name;
    let kind = live_preview::kind_of(name);
    let tools = text::editor_tools(kind.is_some(), ui.editor_view.preview_open, t);
    // The field first: the F6 stops go in the order they are built (field, find bar, preview).
    let field = text::editor(editor, name, field_spans(ui, editor, t), t)
        .on_scroll_offset(super::editor_minimap::note_scroll);
    let banner = text::refused_banner(editor.refused.as_deref(), t)
        .or_else(|| editor_find_bar::bar(&ui.editor_view, t));
    let body = match kind {
        Some(kind) if ui.editor_view.preview_open => row()
            .key("editor-split")
            .grow(1.0)
            .shrink()
            .min_w(0.0)
            .h_full()
            .child(el().key("editor-half").w_frac(0.5).h_full().child(field))
            .child(
                el().key("preview-half")
                    .w_frac(0.5)
                    .h_full()
                    .child(live_preview::pane(kind, &editor.edit.text, ui, t)),
            ),
        _ => field,
    };
    // The minimap's slot, at the editor's right edge.
    let map = super::editor_minimap::minimap(&editor.edit, name, super::editor_minimap::current_scroll(), t);
    let body = row().key("editor-with-map").grow(1.0).shrink().min_w(0.0).h_full().child(body).child(map);
    (tools, banner, body)
}
