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

/// What the glyph says (`.tone-neutral`, `.tone-green`, `.tone-red`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Neutral,
    Done,
    /// Something failed (DW-0109).
    Failed,
}

/// A toast's button: the words drawn on it ("Undo Ctrl+Z"), the name assistive technology reads
/// ("Undo"), its shortcut if it has one ("Ctrl+Z"), and what it does.
#[derive(Clone, Copy)]
pub struct ToastAction {
    pub text: &'static str,
    pub name: &'static str,
    pub shortcut: Option<&'static str>,
    pub run: fn(&mut State, &mut Tx),
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

/// The toast. A failure is read at once (`alert`); the others when there is a pause (`status`).
pub fn toast(ic: &'static Icon, tone: Tone, message: &str, action: Option<ToastAction>, t: &Theme) -> El {
    toast_keyed("toast", ic, tone, message, action, t)
}

/// [`toast`] under keys of its own (`<key>`, `<key>-icon`, `<key>-text`, `<key>-action`, `<key>-action-text`),
/// for a window that can show two at once.
pub fn toast_keyed(
    key: &str,
    ic: &'static Icon,
    tone: Tone,
    message: &str,
    action: Option<ToastAction>,
    t: &Theme,
) -> El {
    // `.glass`: the board's paper at 72 % (68 % in dark, where its paper is the theme's paper-2) over a blur.
    let glass = if t.dark {
        t.paper2.alpha(0.68)
    } else {
        t.paper.alpha(0.72)
    };
    let mut toast = row()
        .key(key.to_string())
        .role(if tone == Tone::Failed { "alert" } else { "status" })
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
            format!("{key}-icon"),
            ic,
            15.0,
            match tone {
                Tone::Neutral => t.text_secondary,
                Tone::Done => t.green,
                Tone::Failed => t.red,
            },
        ))
        .child(kit::label(format!("{key}-text"), message, TEXT, t.text));
    if let Some(action) = action {
        let (label, run) = (action.text, action.run);
        let mut button = el().key(format!("{key}-action")).role("button").label(action.name);
        if let Some(keys) = action.shortcut {
            button = button.shortcut_hint(keys);
        }
        toast = toast.child(
            button
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
                .child(kit::label(format!("{key}-action-text"), label, ACTION, t.text)),
        );
    } else {
        toast = toast.pr(8.0);
    }
    toast
}
