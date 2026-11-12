//! The status line without a connection (`.mk-foot` with `.mk-sdot.is-off`): a grey dot and
//! "Offline" where `ui::status_line` draws the green one. Same keys: `status-bar`, `status-dot`,
//! `status-text`, `status-details`.

use crate::theme::tokens::{Theme, type_};
use rune::{El, el, row};

use super::kit;

pub fn status_offline(details: &str, t: &Theme) -> El {
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
        .child(
            row()
                .key("status-left")
                .center_y()
                .gap(7.0)
                // `--color-ink-4`: nothing is moving, and nothing is wrong.
                .child(
                    el().key("status-dot")
                        .size(7.0, 7.0)
                        .radius(3.5)
                        .fill(t.text_quaternary),
                )
                .child(kit::label(
                    "status-text",
                    "Offline",
                    type_::STATUS_STRONG,
                    t.text_secondary,
                )),
        )
        .child(el().key("status-spacer").grow(1.0))
        .child(kit::label("status-details", details, type_::STATUS, t.text_tertiary))
}
