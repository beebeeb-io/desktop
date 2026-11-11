//! Status line (`.mk-foot`): a state dot and its word at the left, the counts at the right.

use crate::theme::tokens::{Theme, type_};
use rune::{El, el, row};

use super::kit;

const STATUS_H: f32 = 28.0;

pub fn status_bar(t: &Theme) -> El {
    row()
        .key("status-bar")
        .role("status")
        .h(STATUS_H)
        .w_full()
        .fill(t.status_bar_bg)
        .border_sides([kit::HAIR, 0.0, 0.0, 0.0], t.separator)
        .border_layout()
        .px(16.0)
        .gap(7.0)
        .center_y()
        .child(
            row()
                .key("status-left")
                .center_y()
                .gap(7.0)
                .child(el().key("status-dot").size(7.0, 7.0).radius(3.5).fill(t.green))
                .child(kit::label(
                    "status-text",
                    "Up to date",
                    type_::STATUS_STRONG,
                    t.text_secondary,
                )),
        )
        .child(el().key("status-spacer").grow(1.0))
        .child(kit::label(
            "status-details",
            "4 on this PC \u{00B7} 3 available offline \u{00B7} 4 online only",
            type_::STATUS,
            t.text_tertiary,
        ))
}
