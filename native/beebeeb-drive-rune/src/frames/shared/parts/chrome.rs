//! The window of a sidebar section other than Files (`.mk-win`): caption, sidebar, the
//! section's toolbar and body, the status line, and an optional layer in front.

use crate::State;
use crate::theme::tokens::Theme;
use crate::ui::caption;
use rune::{Cx, El, col, row};

/// The status line (`.mk-foot`): the PC's sync state in words at the left (`frames::sync_words`, with the dot
/// coloured as Files' line has it: `ui::status_line`), this section's words at the right. No words, no dot.
pub fn status_line(sync: &str, details: &str, t: &Theme) -> El {
    crate::ui::status_line::status_line((!sync.is_empty()).then_some(sync), details, t)
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
        .child(status_line(&crate::frames::sync_words(s), status, t));
    let mut window = col()
        .key("window")
        .role("window")
        .label(s.identity.main_window_title())
        .on_key(crate::frames::on_key_root)
        // The window's size, for a dialog that fits itself to it (an item's Preview), as the Files window records it.
        .on_layout(|s: &mut State, [_, _, w, h]| s.actions.window = (w, h))
        .w_full()
        .h_full()
        .fill(t.bg)
        .child(caption::caption_bar(s.identity.main_window_title(), t, cx))
        .child(
            row()
                .key("body")
                .grow(1.0)
                .min_h(0.0)
                .w_full()
                .child(crate::ui::sidebar::sidebar(s, t))
                .child(main),
        );
    if let Some(layer) = layer {
        window = window.child(layer);
    }
    // The lane's dialog over this section (`frames::shared::dialogs`), when one is open.
    let dialogs = crate::frames::shared::dialogs::layers(s, t);
    if !dialogs.is_empty() {
        // Keyboard as Windows has it in a dialog, as settings has it: Tab visits every control,
        // Space and Enter press the focused one (Rune's default leaves Enter on a button to the
        // default button, as macOS does).
        cx.web_focus();
    }
    window.children(dialogs)
}
