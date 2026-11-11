//! A toast (`.mk-toast > .kit-toast.glass`): a short message over the bottom of the content
//! area, 42 pt above its lower edge, centred. With an action (`Undo`) it is a button too.
//!
//! Keys: `toast`, `toast-icon`, `toast-text`, and with an action `toast-action`, `toast-action-text`.

use crate::State;
use crate::theme::tokens::Theme;
use rune::{Color, El, Icon, Shadow, TextStyle, Tx, el, row};

use super::kit;

/// `.mk-toast .kit-toast`
pub const TEXT: TextStyle = TextStyle::new(12.5, 550.0).lh(17.0);
/// `.mk-toast .kit-toast b`
pub const ACTION: TextStyle = TextStyle::new(12.5, 600.0).lh(17.0);

/// What the glyph says (`.tone-neutral`, `.tone-green`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Neutral,
    Done,
}

/// The layer a toast sits in: as wide as the content area, 34 pt tall, `bottom` points above
/// the area's lower edge. Put it last in a column that is the content area, so it overlays.
pub fn layer(toast: El) -> El {
    row()
        .key("toast-layer")
        .absolute(0.0, 0.0)
        .w_full()
        .h_full()
        .pb(42.0)
        .justify_center()
        .items_end()
        .child(toast)
}

/// The toast. `action`: its label ("Undo  Ctrl+Z") and what it does.
pub fn toast(
    ic: &'static Icon,
    tone: Tone,
    message: &str,
    action: Option<(&str, fn(&mut State, &mut Tx))>,
    t: &Theme,
) -> El {
    // `.glass`: the board's paper at 72 % (68 % in dark, where its paper is the theme's paper-2) over a blur.
    let glass = if t.dark {
        t.paper2.alpha(0.68)
    } else {
        t.paper.alpha(0.72)
    };
    let mut toast = row()
        .key("toast")
        .role("status")
        .label(message)
        .h(34.0)
        .pl(14.0)
        .pr(8.0)
        .gap(10.0)
        .center_y()
        .radius(17.0)
        .backdrop_blur(22.0)
        .fill(glass)
        .border(kit::HAIR, t.separator)
        .border_layout()
        .box_shadow(kit::drop(6.0, 22.0, Color::BLACK.alpha(0.10)))
        .box_shadow(Shadow {
            x: 0.0,
            y: kit::HAIR,
            blur: 0.0,
            spread: 0.0,
            color: t.highlight,
            inset: true,
        })
        .child(kit::glyph(
            "toast-icon",
            ic,
            15.0,
            match tone {
                Tone::Neutral => t.text_secondary,
                Tone::Done => t.green,
            },
        ))
        .child(kit::label("toast-text", message, TEXT, t.text));
    if let Some((label, run)) = action {
        toast = toast.child(
            el().key("toast-action")
                .role("button")
                .label(label)
                .h(24.0)
                .px(8.0)
                .ml(3.25 - 10.0)
                .radius(12.0)
                .fill(t.selection)
                .hover(t.hover)
                .press(t.press)
                .focusable(true)
                .center()
                .on_press_tx(move |s: &mut State, tx: &mut Tx| run(s, tx))
                .child(kit::label("toast-action-text", label, ACTION, t.text)),
        );
    } else {
        toast = toast.pr(8.0);
    }
    toast
}
