//! Caption bar (`.kit-wincap`): the Beebeeb mark, the window title, and the window's
//! minimize / maximize / close buttons. The whole band drags the window; a double click
//! maximizes it (Rune's `.window_drag()`).
//!
//! On Windows, Rune's `Titlebar` draws the three caption buttons itself over the trailing
//! edge: 46 pt each at any DPI, the close button red under the pointer, the snap-layouts
//! flyout over maximize, named Minimize / Maximize or Restore / Close for assistive
//! technology. This band only keeps `Cx::chrome_insets().trailing` clear for them. Where no
//! system draws them (a headless render, a platform without `App::titlebar`), the band draws
//! its own three, as on the board, and they work.

use crate::State;
use crate::TITLEBAR_H;
use crate::theme::icons;
use crate::theme::tokens::{Theme, type_};
use rune::{Color, Cx, El, Icon, Tx, el, row};

use super::kit;

/// A caption button is 46 pt wide on Windows 11 (and on the board).
pub const CAPTION_BUTTON_W: f32 = 46.0;

pub fn caption_bar(t: &Theme, cx: &mut Cx<State>) -> El {
    let insets = cx.chrome_insets();
    cx.caption_color(if t.dark { Color::WHITE } else { Color::BLACK });

    let mut bar = row()
        .key("caption-bar")
        .role("group")
        .label("Title bar")
        .w_full()
        .h(TITLEBAR_H)
        .fill(t.paper2)
        .border_sides([0.0, 0.0, 1.0, 0.0], t.card_border)
        .border_layout()
        .window_drag()
        .center_y()
        .gap(9.0)
        .pl(insets.leading + 12.0)
        .child(kit::mark("cap-mark", 16.0))
        .child(
            kit::label("cap-title", "Beebeeb Drive", type_::BODY, t.text_secondary)
                .grow(1.0)
                .truncate(),
        );

    if insets.height > 0.0 {
        // The system's caption buttons (or the Mac's traffic lights at the leading edge).
        bar = bar.child(el().key("cap-system").w(insets.trailing).h_full());
    } else {
        bar = bar.child(
            row()
                .key("cap-buttons")
                .h_full()
                .child(
                    caption_button("cap-min", "Minimize", &icons::CAPTION_MINIMIZE, false, t)
                        .on_press_tx(|_: &mut State, tx: &mut Tx| tx.minimize()),
                )
                .child(
                    caption_button("cap-max", "Maximize", &icons::CAPTION_MAXIMIZE, false, t)
                        .on_press_tx(|_: &mut State, tx: &mut Tx| tx.toggle_maximized()),
                )
                .child(
                    caption_button("cap-close", "Close", &icons::CAPTION_CLOSE, true, t)
                        .on_press_tx(|_: &mut State, tx: &mut Tx| tx.close_window()),
                ),
        );
    }
    bar
}

fn caption_button(key: &'static str, name: &'static str, ic: &'static Icon, close: bool, t: &Theme) -> El {
    let glyph = kit::glyph(format!("{key}-icon"), ic, 10.0, t.text);
    el().key(key)
        .role("button")
        .label(name)
        .w(CAPTION_BUTTON_W)
        .h_full()
        .center()
        .no_window_drag()
        .hover(if close { t.caption_close } else { t.caption_hover })
        .press(if close { t.caption_close.alpha(0.9) } else { t.press })
        .child(if close { glyph.group_color(Color::WHITE) } else { glyph })
}
