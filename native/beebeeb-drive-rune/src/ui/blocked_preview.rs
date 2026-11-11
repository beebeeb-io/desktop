//! The preview panel when the file cannot be shown (DW-0121): the panel's bar with the file's
//! name and its cross, and under it what is needed, with one way forward.
//!
//! Keys: `preview-card`, `preview-bar`, `preview-close`, `preview-close-icon`, `preview-title`,
//! `blocked`, `blocked-tile`, `blocked-icon`, `blocked-title`, `blocked-text`, `blocked-retry`,
//! `blocked-retry-label`.

use crate::State;
use crate::theme::tokens::{Theme, type_};
use rune::{Color, El, Icon, Tx, col, el, row};

use super::toolbar::{ToolState, tool_button};
use super::{button, empty, global_icons, kit};

pub const CARD_W: f32 = 480.0;
pub const CARD_H: f32 = 330.0;

pub type Handler = fn(&mut State, &mut Tx);

/// The panel. `name` is the file; `title` and `sentence` say what is needed; `retry` is the
/// button's word and what it does; `close` is the cross.
pub fn blocked_preview(
    name: &str,
    ic: &'static Icon,
    title: &str,
    sentence: &str,
    retry: (&str, Handler),
    close: Handler,
    t: &Theme,
) -> El {
    let (retry_word, retry_run) = retry;
    col()
        .key("preview-card")
        .role("dialog")
        .label(format!("{name}: {title}"))
        .description(sentence)
        .size(CARD_W, CARD_H)
        .radius(12.0)
        .fill(t.card)
        .box_shadow(kit::drop(30.0, 80.0, Color::BLACK.alpha(0.34)))
        .box_shadow(kit::ring(kit::HAIR, t.separator))
        .child(
            row()
                .key("preview-bar")
                .h(44.0)
                .w_full()
                .px(12.0)
                .gap(8.0)
                .items_center()
                .border_sides([0.0, 0.0, kit::HAIR, 0.0], t.separator)
                .child(
                    tool_button("preview-close", "Close", &global_icons::CLOSE, 14.0, ToolState::Idle, t)
                        .shortcut_hint("Esc")
                        .on_press_tx(move |s: &mut State, tx: &mut Tx| close(s, tx)),
                )
                .child(
                    el().key("preview-title-box").grow(1.0).min_w(0.0).px(4.0).child(
                        kit::label("preview-title", name, type_::TITLE, t.text)
                            .truncate()
                            .w_full(),
                    ),
                ),
        )
        .child(
            empty::empty_state("blocked", ic, title, sentence, t).w_full().child(
                button::outline_button("blocked-retry", retry_word, None, t)
                    .autofocus()
                    .focus_ring(2.0, 2.0, t.accent_deep)
                    .on_press_tx(move |s: &mut State, tx: &mut Tx| retry_run(s, tx)),
            ),
        )
}
