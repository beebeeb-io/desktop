//! A plain button (`.kit-btn`, outlined): "Add files" in an empty folder, the selection bar's actions.
//!
//! Keys: `<key>`, `<key>-icon`, `<key>-label`.

use crate::theme::tokens::Theme;
use rune::{El, Icon, TextStyle, row};

use super::kit;

/// `.kit-btn-md` (28 pt tall, a 16 pt glyph) and `.kit-btn-sm` (22 pt, 14 pt).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Size {
    Md,
    Sm,
}

/// `.kit-btn-md span`: 13/500 on the button's 10/7 line, tracked -0.005em.
pub const LABEL_MD: TextStyle = TextStyle::new(13.0, 500.0).lh(18.5714).tracking(-0.065);
/// `.kit-btn-sm span`
pub const LABEL_SM: TextStyle = TextStyle::new(12.0, 500.0).lh(17.1429).tracking(-0.06);

/// The button. The caller adds `.on_press(..)`.
pub fn outline_button(key: &str, label: &str, ic: Option<&'static Icon>, size: Size, t: &Theme) -> El {
    let (h, glyph, style) = match size {
        Size::Md => (28.0, 16.0, LABEL_MD),
        Size::Sm => (22.0, 14.0, LABEL_SM),
    };
    let mut button = row()
        .key(key)
        .role("button")
        .label(label)
        .h(h)
        .px(12.0)
        .gap(7.0)
        .center()
        .radius(6.0)
        .fill(t.paper)
        .border(1.0, t.card_border)
        .border_layout()
        // `hover:bg-paper-2 active:bg-paper-3`, as a wash of ink over the paper.
        .hover(t.hover)
        .press(t.press)
        .focusable(true);
    if let Some(ic) = ic {
        button = button.child(kit::glyph(format!("{key}-icon"), ic, glyph, t.text));
    }
    button.child(kit::label(format!("{key}-label"), label, style, t.text))
}
