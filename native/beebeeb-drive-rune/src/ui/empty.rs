//! An empty state (`.kit-empty`): a tile with a glyph, a title, one sentence. The caller adds
//! what comes under it (a button, a hint) with `.child(..)`.
//!
//! Keys: `<key>`, `<key>-tile`, `<key>-icon`, `<key>-title`, `<key>-text`.

use crate::theme::tokens::Theme;
use rune::{El, Icon, TextStyle};

use super::view_state::{ViewState, view_state};

/// `.t-title`: 17/600, tracked -0.012em.
pub const TITLE: TextStyle = TextStyle::new(17.0, 600.0).lh(22.0).tracking(-0.204);
/// `.t-sub`
pub const SUB: TextStyle = TextStyle::new(12.0, 400.0).lh(15.0);
/// `.t-cap`
pub const CAP: TextStyle = TextStyle::new(11.0, 500.0).lh(14.0);

/// `.kit-empty`: 28/24 padding, 8 between its parts, everything centred; the sentence wraps at 227 pt.
/// Drawn by [`super::view_state`] as `ViewState::Empty` with no action.
pub fn empty_state(key: &str, ic: &'static Icon, title: &str, sentence: &str, t: &Theme) -> El {
    view_state(
        ViewState::Empty {
            key,
            icon: ic,
            title,
            body: sentence,
            action: None,
        },
        t,
    )
}
