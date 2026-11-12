//! An empty state (`.kit-empty`): a tile with a glyph, a title, one sentence. The caller adds
//! what comes under it (a button, a hint) with `.child(..)`.
//!
//! Keys: `<key>`, `<key>-tile`, `<key>-icon`, `<key>-title`, `<key>-text`.

use crate::theme::tokens::Theme;
use rune::{Align, El, Icon, TextStyle, col, el};

use super::kit;

/// `.t-title`: 17/600, tracked -0.012em.
pub const TITLE: TextStyle = TextStyle::new(17.0, 600.0).lh(22.0).tracking(-0.204);
/// `.t-sub`
pub const SUB: TextStyle = TextStyle::new(12.0, 400.0).lh(15.0);
/// `.t-cap`
pub const CAP: TextStyle = TextStyle::new(11.0, 500.0).lh(14.0);

/// `.kit-empty`: 28/24 padding, 8 between its parts, everything centred; the sentence wraps at 227 pt.
pub fn empty_state(key: &str, ic: &'static Icon, title: &str, sentence: &str, t: &Theme) -> El {
    col()
        .key(key)
        .role("group")
        .label(title)
        .py(28.0)
        .px(24.0)
        .gap(8.0)
        .items_center()
        .child(
            el().key(format!("{key}-tile"))
                .size(52.0, 52.0)
                .radius(15.0)
                .fill(t.tile)
                .center()
                .child(kit::glyph(format!("{key}-icon"), ic, 26.0, t.text_secondary)),
        )
        .child(kit::label(format!("{key}-title"), title, TITLE, t.text))
        .child(
            kit::label(format!("{key}-text"), sentence, SUB, t.text_tertiary)
                .wrap()
                .w(227.11)
                .align(Align::Center),
        )
}
