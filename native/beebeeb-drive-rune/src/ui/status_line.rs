//! The status line with its words from the caller (`.mk-foot`): `status_bar::status_bar` says the
//! DW-0401 sample's; this one says what the folder holds. Same keys: `status-bar`, `status-dot`,
//! `status-text`, `status-details`. `state` is the sync state's word ("Up to date"); `None` when
//! nothing syncs here: then there is no dot and no word, never a sample.

use crate::theme::tokens::{Theme, type_};
use rune::{El, el, row};

use super::kit;

pub fn status_line(state: Option<&str>, details: &str, t: &Theme) -> El {
    let mut left = row().key("status-left").center_y().gap(7.0);
    if let Some(state) = state {
        left = left
            .child(el().key("status-dot").size(7.0, 7.0).radius(3.5).fill(t.green))
            .child(kit::label("status-text", state, type_::STATUS_STRONG, t.text_secondary));
    }
    row()
        .key("status-bar")
        .role("status")
        .h(28.0)
        .w_full()
        .fill(t.status_bar_bg)
        .border_sides([kit::HAIR, 0.0, 0.0, 0.0], t.separator)
        .border_layout()
        .px(16.0)
        .gap(7.0)
        .center_y()
        .child(left)
        .child(el().key("status-spacer").grow(1.0))
        .child(kit::label("status-details", details, type_::STATUS, t.text_tertiary))
}
