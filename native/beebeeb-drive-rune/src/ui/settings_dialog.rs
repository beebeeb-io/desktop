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

use crate::theme::tokens::{Theme, type_};
use rune::{Color, El, Span, TextStyle, col, rich, row};

use super::button::{self as btn, ButtonState, Size, Variant};
use super::dialog::{Dialog, Foot, Head, Sub, scrim};
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
    let variant = match weight {
        Weight::Primary => Variant::Primary,
        Weight::Quiet => Variant::Ghost,
    };
    btn::button(btn::keys(&format!("btn-{index}")), label, None, variant, Size::Md, ButtonState::Enabled, t)
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
    let d = Dialog {
        head: Head::Title { title, sub: sub.map(Sub::Wrap) },
        body,
        foot: Foot::Row(foot),
        ..Dialog::new("dlg", name, width)
    };
    scrim(0.0, dialog::dialog(d, t), t)
}

/// A dialog's foot (`.wn-dlg-foot`): the buttons at its trailing edge, or `even` (`.is-even`):
/// each button that many points wide, side by side across the whole foot.
pub fn foot_row(buttons: Vec<El>, even: Option<f32>, t: &Theme) -> El {
    let mut f = row()
        .key("dlg-foot")
        .center_y()
        .justify_end()
        .gap(8.0)
        .py(16.0)
        .px(24.0)
        .fill(t.surface)
        .border_sides([kit::HAIR, 0.0, 0.0, 0.0], t.separator)
        .border_layout();
    for child in buttons {
        f = f.child(match even {
            Some(each) => child.w(each),
            None => child,
        });
    }
    f
}

/// A dialog that asks before something happens (`.wn-dlg-alert`, `role="alertdialog"`): a
/// toned tile, the question, what it means; the two answers side by side, the same width.
/// Keys: `dlg-title`, `dlg-sub`, the tile's own.
pub fn alert(name: &str, width: f32, tile: El, title: &str, sub: &str, foot: Vec<El>, t: &Theme) -> El {
    let d = Dialog {
        alert: true,
        head: Head::Alert { tile, title, sub },
        foot: Foot::Even(foot),
        ..Dialog::new("dlg", name, width)
    };
    scrim(0.0, dialog::dialog(d, t), t)
}

/// A dialog-size button for a destructive answer (`.kit-btn-md` on red): white on red, the
/// same in both themes. Keys: `btn-<N>`, `btn-<N>-label`.
pub fn button_danger(index: usize, label: &str, t: &Theme) -> El {
    btn::button(btn::keys(&format!("btn-{index}")), label, None, Variant::Danger, Size::Md, ButtonState::Enabled, t)
}
