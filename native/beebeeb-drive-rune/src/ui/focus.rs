//! The one keyboard focus ring: 2 pt wide, 2 pt off the border box, in `accent_deep`
//! ([`RING_WIDTH`], [`RING_OFFSET`]). `ui/button.rs` and the other controls that name a ring
//! use [`apply`]; [`install`] sets it as the view's default so a focusable control that names
//! none (a row, a link, a card, a viewer control) draws the same ring, not AppKit's.

use rune::{Color, Cx, El};

use crate::theme::tokens;

pub const RING_WIDTH: f32 = 2.0;
pub const RING_OFFSET: f32 = 2.0;

/// The ring on one element.
pub fn apply(el: El, color: Color) -> El {
    el.focus_ring(RING_WIDTH, RING_OFFSET, color)
}

/// The view's default ring. Call it at the top of every window view, on every run.
pub fn install<S>(cx: &mut Cx<S>) {
    cx.focus_ring(RING_WIDTH, RING_OFFSET, tokens::theme(cx.system_dark()).accent_deep);
}

#[cfg(test)]
mod tests {
    use super::*;
    use rune::{Env, Runtime, col, el};

    fn view(_: &(), cx: &mut Cx<()>) -> El {
        install(cx);
        // A control that names no ring, and one that names the shared one.
        col()
            .child(el().key("bare").size(40.0, 20.0).focusable(true))
            .child(apply(el().key("named").size(40.0, 20.0).focusable(true), tokens::theme(false).accent_deep))
    }

    #[test]
    fn a_control_with_no_ring_of_its_own_shows_focus_from_the_keyboard_as_a_named_one_does() {
        let env = Env { system_dark: false, scale: 2.0 };
        let mut rt = Runtime::new((), view, (200.0, 100.0), env);
        for key in ["bare", "named"] {
            rt.update(move |_, tx| tx.focus(key, true));
            rt.frame(0.0);
            rt.settle_all();
            rt.frame(0.0);
            let e = rt.elements().into_iter().find(|e| e.key.as_deref() == Some(key)).expect(key);
            assert!(e.focused && e.focus_visible, "{key}: the ring shows");
        }
    }
}
