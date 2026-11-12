//! The one dialog card (`.wn-dlg`) and its scrim (`.mk-scrim`), for every window.
//!
//! | Piece | Function | Keys it gives |
//! | --- | --- | --- |
//! | The card: title, line under it, body, foot | `card(name, width, title, sub, body, foot, t)` | `dlg`, `dlg-clip`, `dlg-head`, `dlg-title`, `dlg-sub`, `dlg-body`, `dlg-foot` |
//! | The scrim with the card in its middle | `scrim(top, card, t)` | `scrim` |
//!
//! `settings_dialog::dialog` (a row of buttons, the scrim over the whole pane) and the Files
//! window's dialogs (buttons one under the other, the scrim under the caption bar) are both
//! this card: its radius, shadow, ring, paddings and the foot's hairline are written here once.
//! The caller gives the foot as it lays its buttons out (`row()` or `col()`, with its children);
//! the card gives it the key, the gap, the padding, the fill and the hairline.

use crate::theme::tokens::Theme;
use rune::{Color, El, col};

use super::kit;
use super::settings_dialog::{DIALOG_TITLE, scrim_color};
use super::settings_shell::SUB;

/// The card's corner (`.wn-dlg`), also the clip that rounds the foot's corners.
pub const RADIUS: f32 = 8.0;

/// What the line under the title does when it is longer than the card.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sub<'a> {
    /// A sentence: it goes on to the next line.
    Wrap(&'a str),
    /// A name (a file's, a folder's): one line, cut with an ellipsis.
    Truncate(&'a str),
}

/// A dialog's card. `foot`: the buttons in the caller's layout, the primary first, the way out
/// last, as the board orders them.
pub fn card(name: &str, width: f32, title: &str, sub: Option<Sub>, body: Vec<El>, foot: El, t: &Theme) -> El {
    let mut head = col().key("dlg-head").pt(20.0).px(24.0).pb(4.0).gap(3.0).child(
        kit::label("dlg-title", title, DIALOG_TITLE, t.text)
            .role("heading")
            .level(2)
            .w_full(),
    );
    match sub {
        Some(Sub::Wrap(words)) => head = head.child(kit::label("dlg-sub", words, SUB, t.text_tertiary).wrap()),
        Some(Sub::Truncate(words)) => head = head.child(kit::label("dlg-sub", words, SUB, t.text_tertiary).truncate()),
        None => {}
    }
    let mut b = col().key("dlg-body").pt(10.0).px(24.0).pb(18.0).gap(12.0).min_h(0.0);
    for child in body {
        b = b.child(child);
    }
    let foot = foot
        .key("dlg-foot")
        .gap(8.0)
        .py(16.0)
        .px(24.0)
        .fill(t.surface)
        .border_sides([kit::HAIR, 0.0, 0.0, 0.0], t.separator)
        .border_layout();
    // The shadow and the hairline ring are outside the card; the clip that rounds the foot's
    // corners would cut them, so it sits one level in.
    col()
        .key("dlg")
        .role("dialog")
        .label(name)
        // The foot's hairline makes a dialog's height end in a half point, so centring puts
        // its top on a quarter point. The board draws it there, edges half covered; so does this.
        .pixel_snap(false)
        .w(width)
        .radius(RADIUS)
        .fill(t.card)
        .box_shadow(kit::drop(24.0, 60.0, Color::BLACK.alpha(0.28)))
        .box_shadow(kit::ring(kit::HAIR, t.separator))
        .child(
            col()
                .key("dlg-clip")
                .w_full()
                .clip_rounded(RADIUS)
                .child(head)
                .child(b)
                .child(foot),
        )
}

/// The scrim that takes every press outside the card, the card in its middle. `top`: how far
/// under its parent's top it starts (a window's caption bar stays free, so the window can still
/// be moved and closed).
pub fn scrim(top: f32, card: El, t: &Theme) -> El {
    col()
        .key("scrim")
        .pin(Some(0.0), Some(top), Some(0.0), Some(0.0))
        .center()
        .fill(scrim_color(t))
        .child(card)
}
