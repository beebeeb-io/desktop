//! A dialog over a settings pane (`.mk-scrim` + `.wn-dlg`), and the dialog-size button.
//!
//! | Piece | Function | Keys it gives |
//! | --- | --- | --- |
//! | Dialog on its scrim | `dialog(name, width, title, sub, body, foot, t)` | `scrim`, `dlg`, `dlg-title`, `dlg-sub`, `dlg-body`, `dlg-foot` |
//! | A paragraph of the body | `para_rich(index, words, mono, t)`, `caption(index, words, t)`, `bullets(lines, t)` | `dlg-text-<I>`, `dlg-li-<I>` |
//! | Button (`.kit-btn.kit-btn-md`, 28 pt) | `button(index, label, Weight, t)` | `btn-<N>`, `btn-<N>-label` |
//!
//! The dialog sits in the middle of the window's body, above a scrim that takes every press
//! outside it. It is modal: Tab stays inside, Esc is the caller's `on_dismiss`.
//! Buttons are numbered with every other `.kit-btn` of the frame, in the board's order.

use crate::theme::tokens::{Theme, radius, type_};
use rune::{Color, El, Span, TextStyle, col, rich, row};

use super::dialog;
use super::kit;
use super::settings_shell::SUB;

/// 17px/600 on a 22px line, tracked -0.012em: a dialog's title (`.t-title`).
pub const DIALOG_TITLE: TextStyle = TextStyle::new(17.0, 600.0).lh(22.0).tracking(-0.204);
/// 13px/500, tracked -0.005em: a dialog-size button's label (`.kit-btn-md`).
pub const BUTTON_MD: TextStyle = TextStyle::new(13.0, 500.0).lh(18.5714).tracking(-0.065);

/// `--kit-scrim`: black at 28 % (light), 55 % (dark).
pub fn scrim_color(t: &Theme) -> Color {
    Color::BLACK.alpha(if t.dark { 0.55 } else { 0.28 })
}

/// How much a dialog button asks for attention.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Weight {
    /// The one primary action: Amber.
    Primary,
    /// The way out (Cancel, Close): words only, a tint under the pointer.
    Quiet,
}

/// A dialog-size button. Keys: `btn-<N>`, `btn-<N>-label`.
pub fn button(index: usize, label: &str, weight: Weight, t: &Theme) -> El {
    let key = format!("btn-{index}");
    let (fill, ink) = match weight {
        Weight::Primary => (t.accent, t.accent_text_on),
        Weight::Quiet => (Color::TRANSPARENT, t.text_secondary),
    };
    let b = row()
        .key(key.clone())
        .role("button")
        .label(label)
        .h(28.0)
        .px(12.0)
        .center()
        .radius(radius::MD)
        .fill(fill)
        .focusable(true);
    let b = match weight {
        Weight::Quiet => b.hover(t.paper2).press(t.paper3),
        _ => b
            .hover(fill.mix(Color::BLACK, 0.05))
            .press(fill.mix(Color::BLACK, 0.10)),
    };
    b.child(kit::label(format!("{key}-label"), label, BUTTON_MD, ink))
}

/// A small print line of a dialog's body (`.wn-dlg-body > p.t-cap`). Key: `dlg-text-<I>`.
pub fn caption(index: usize, words: &str, t: &Theme) -> El {
    kit::label(format!("dlg-text-{index}"), words, type_::CAPTION, t.text_tertiary).wrap()
}

/// A list of lines in a dialog's body (`ul.t-sub` > `li`): indented, and without a marker,
/// as the board draws it. Keys: `dlg-li-<I>`.
pub fn bullets(lines: &[String], t: &Theme) -> El {
    let mut list = col().key("dlg-list").role("list").w_full().pl(18.0).gap(6.0);
    for (i, line) in lines.iter().enumerate() {
        list = list.child(
            row().key(format!("dlg-li-{i}-row")).role("listitem").w_full().child(
                kit::label(format!("dlg-li-{i}"), line.clone(), SUB, t.text_secondary)
                    .wrap()
                    .grow(1.0),
            ),
        );
    }
    list
}

/// A paragraph with a machine's value inside it (`spans`: byte ranges in mono).
pub fn para_rich(index: usize, words: &str, mono: &[std::ops::Range<usize>], t: &Theme) -> El {
    let key = format!("dlg-text-{index}");
    kit::note_text(&key, &key, words, SUB, t.text_tertiary);
    let spans = mono.iter().map(|r| Span::new(r.start, r.end).mono()).collect();
    rich(words, spans).key(key).font(SUB).color(t.text_tertiary)
}

/// A dialog in the middle of the body, on its scrim. `foot` holds the buttons, the primary
/// first, the way out last, as the board orders them.
pub fn dialog(name: &str, width: f32, title: &str, sub: Option<&str>, body: Vec<El>, foot: Vec<El>, t: &Theme) -> El {
    // The card and the scrim are `ui::dialog`'s, the same for every window's dialogs.
    let mut f = row().center_y().justify_end();
    for child in foot {
        f = f.child(child);
    }
    let card = dialog::card(name, width, title, sub.map(dialog::Sub::Wrap), body, f, t);
    dialog::scrim(0.0, card, t)
}
