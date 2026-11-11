//! Status bar: green dot + "Up to date" left, counts right-aligned.
//!
//! Board: 28 pt tall, paper-2 bg, green dot + "Up to date" flush left,
//! "4 on this PC · 3 available offline · 4 online only" flush right.

use crate::theme::tokens::{Theme, space, type_};
use rune::{El, el, row, text};

const STATUS_H: f32 = 28.0;

pub fn status_bar(t: &Theme) -> El {
    row()
        .key("status-bar")
        .h(STATUS_H)
        .w_full()
        .fill(t.surface)
        .px(space::LG)
        .center_y()
        .justify_between()
        // Left: green dot + "Up to date"
        .child(
            row()
                .key("status-left")
                .center_y()
                .gap(space::XS)
                .child(el().key("status-dot").size(6.0, 6.0).radius(3.0).fill(t.green))
                .child(
                    text("Up to date")
                        .key("status-text")
                        .font(type_::STATUS)
                        .color(t.text_tertiary),
                ),
        )
        // Right: counts
        .child(
            text("4 on this PC \u{00B7} 3 available offline \u{00B7} 4 online only")
                .key("status-details")
                .font(type_::STATUS)
                .color(t.text_tertiary),
        )
}
