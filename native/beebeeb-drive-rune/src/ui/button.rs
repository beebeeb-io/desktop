//! A plain button (`.kit-btn.kit-btn-md`, outlined): "Add files" in an empty folder.
//!
//! Keys: `<key>`, `<key>-icon`, `<key>-label`.

use crate::theme::tokens::Theme;
use rune::{El, Icon, TextStyle, row};

use super::kit;

/// `.kit-btn-md span`: 13/500 on the button's 10/7 line, tracked -0.005em.
pub const LABEL_MD: TextStyle = TextStyle::new(13.0, 500.0).lh(18.5714).tracking(-0.065);
/// The button. The caller adds `.on_press(..)`.
pub fn outline_button(key: &str, label: &str, ic: Option<&'static Icon>, t: &Theme) -> El {
    // `.kit-btn-md`: 28 pt tall, a 16 pt glyph.
    let (h, glyph, style) = (28.0, 16.0, LABEL_MD);
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
