//! A card on a settings pane (`.kit-card`): a raised panel for one thing that is not a list.
//!
//! | Piece | Function | Keys it gives |
//! | --- | --- | --- |
//! | Card | `card(index, t)` | `card-<N>` |
//! | Its two lines | `head(index, text, t)`, `sub(index, text, t)` | `card-<N>-head`, `card-<N>-sub` |
//!
//! | The same as a column | `column(index, pad, gap, t)` | `card-<N>` |
//!
//! `card` is an empty row with the card's padding (16 pt) and gap (12 pt): add children.

use crate::theme::tokens::{Theme, radius, type_};
use rune::{Color, El, row};

use super::kit;
use super::settings_shell::SUB;

/// The panel (`.kit-card.p-4.flex.items-center.gap-3`). Key: `card-<N>`.
pub fn card(index: usize, t: &Theme) -> El {
    row()
        .key(format!("card-{index}"))
        .w_full()
        .center_y()
        .padding(16.0)
        .gap(12.0)
        .radius(radius::LG)
        .fill(t.card)
        .box_shadow(kit::ring(kit::HAIR, t.separator))
        .box_shadow(kit::drop(1.0, 2.0, Color::BLACK.alpha(0.04)))
}

/// A card's strong line (`.t-head`). Key: `card-<N>-head`.
pub fn head(index: usize, words: &str, t: &Theme) -> El {
    kit::label(format!("card-{index}-head"), words, type_::HEAD, t.text).w_full()
}

/// A card's second line (`.t-sub`). Key: `card-<N>-sub`.
pub fn sub(index: usize, words: &str, t: &Theme) -> El {
    kit::label(format!("card-{index}-sub"), words, SUB, t.text_tertiary).w_full()
}

/// The panel as a column (`.kit-card.flex.flex-col`): `pad` points inside, its children `gap`
/// apart. Key: `card-<N>`.
pub fn column(index: usize, pad: f32, gap: f32, t: &Theme) -> El {
    rune::col()
        .key(format!("card-{index}"))
        .w_full()
        .padding(pad)
        .gap(gap)
        .radius(radius::LG)
        .fill(t.card)
        .box_shadow(kit::ring(kit::HAIR, t.separator))
        .box_shadow(kit::drop(1.0, 2.0, Color::BLACK.alpha(0.04)))
}
