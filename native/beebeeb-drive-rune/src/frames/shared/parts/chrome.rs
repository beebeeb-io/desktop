//! The window of a sidebar section other than Files (`.mk-win`): caption, sidebar, the
//! section's toolbar and body, the status line, and an optional layer in front.

use super::sidebar;
use crate::State;
use crate::theme::tokens::{Theme, type_};
use crate::ui::{caption, kit};
use rune::{Cx, El, col, el, row};

/// The status line (`.mk-foot`): the PC's sync state in words at the left (`sync`, from
/// `frames::PcStatus`), this section's words at the right.
pub fn status_line(sync: &str, details: &str, t: &Theme) -> El {
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
        .child(el().key("status-dot").size(7.0, 7.0).radius(3.5).fill(t.green))
        .child(kit::label("status-text", sync, type_::STATUS_STRONG, t.text_secondary))
        .child(el().key("status-spacer").grow(1.0))
        .child(kit::label("status-details", details, type_::STATUS, t.text_tertiary))
}

/// The section window. `layer` is drawn over the whole client area (a menu, a dialog).
pub fn window(s: &State, cx: &mut Cx<State>, t: &Theme, toolbar: El, body: El, status: &str, layer: Option<El>) -> El {
    let main = col()
        .key("content-area")
        .role("main")
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .fill(t.bg)
        .child(toolbar)
        // The one banner of the whole window (`frames::global`: "on every view of the window").
        .children(crate::frames::global::banner_under_toolbar(s, t))
        .child(body.grow(1.0).min_h(0.0).w_full())
        .child(status_line(&s.frames.pc.sync_words(), status, t));
    let mut window = col()
        .key("window")
        .role("window")
        .label("Beebeeb Drive")
        .on_key(crate::frames::on_key_root)
        .w_full()
        .h_full()
        .fill(t.bg)
        .child(caption::caption_bar(t, cx))
        .child(
            row()
                .key("body")
                .grow(1.0)
                .min_h(0.0)
                .w_full()
                .child(sidebar::sidebar(
                    s,
                    sidebar::Counts {
                        waiting: s.frames.shared.source.invitations().len() as u32,
                        trash: s.frames.trash.source.totals().items as u32,
                    },
                    t,
                ))
                .child(main),
        );
    if let Some(layer) = layer {
        window = window.child(layer);
    }
    window
}
