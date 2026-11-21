//! The one button (`.kit-btn`): four variants, three sizes, four states.
//!
//! Keys: `<key>`, `<key>-icon`, `<key>-label` (see [`keys`]), or any three a caller names.
//! The look is the app's main sidebar and settings look: on paper or transparent, hover `t.hover`
//! and press `t.press`; a filled button darkens 5 % / 10 %. Every enabled button takes focus, draws
//! the 2/2 accent ring and shows the pointer.

use crate::theme::tokens::{self, Theme, radius, type_};
use rune::{Color, El, Icon, TextStyle, row};

use super::kit;

/// `.kit-btn-md span`: 13/500 on the button's 10/7 line, tracked -0.005em.
pub const LABEL_MD: TextStyle = TextStyle::new(13.0, 500.0).lh(18.5714).tracking(-0.065);
/// `.kit-btn-lg span`: 13/500 on 19.5, tracked -0.005em.
pub const LABEL_LG: TextStyle = super::au::type_au::BUTTON_LG;

/// Side padding, and the gap between the icon and the words.
const PX: f32 = 12.0;
const GAP: f32 = 7.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Variant {
    /// `t.accent` fill, `t.accent_text_on` words.
    Primary,
    /// `t.paper` fill, a 1 pt `t.card_border` border, `t.text` words.
    Secondary,
    /// Transparent, `t.text_secondary` words.
    Ghost,
    /// `tokens::red_fill()`, white words, in both themes.
    Danger,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Size {
    /// 22 high, 14 pt icon, `type_::BUTTON_SM`.
    Sm,
    /// 28 high, 16 pt icon, [`LABEL_MD`].
    Md,
    /// 32 high, 16 pt icon, [`LABEL_LG`].
    Lg,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ButtonState {
    #[default]
    Enabled,
    /// Half strength, still pressable.
    Dim,
    /// Work is running: full strength, takes no press and no focus.
    Busy,
    /// Not yet possible: half strength, takes no press and no focus.
    Disabled,
}

/// The three keys a button draws: the button, its icon and its words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ButtonKeys {
    pub root: String,
    pub icon: String,
    pub label: String,
}

/// `key`, `key-icon`, `key-label`.
pub fn keys(key: &str) -> ButtonKeys {
    ButtonKeys {
        root: key.to_string(),
        icon: format!("{key}-icon"),
        label: format!("{key}-label"),
    }
}

/// Height, icon size and words style of a size.
fn metrics(size: Size) -> (f32, f32, TextStyle) {
    match size {
        Size::Sm => (22.0, 14.0, type_::BUTTON_SM),
        Size::Md => (28.0, 16.0, LABEL_MD),
        Size::Lg => (32.0, 16.0, LABEL_LG),
    }
}

/// The width a button hugs: its words, the icon and its gap, the side padding, and Secondary's
/// border.
pub fn natural_width(label: &str, icon: bool, variant: Variant, size: Size) -> f32 {
    let (_, glyph, style) = metrics(size);
    let mut w = rune::text::advance(label, style) + 2.0 * PX;
    if icon {
        w += glyph + GAP;
    }
    if variant == Variant::Secondary {
        w += 2.0;
    }
    w
}

/// The button. The caller chains `.on_press(..)` (it does not run for Busy or Disabled), and
/// `.w(..)`, `.w_full()`, `.label(..)` or `.pixel_snap(false)` where it needs them.
pub fn button(
    keys: ButtonKeys,
    label: &str,
    icon: Option<&'static Icon>,
    variant: Variant,
    size: Size,
    state: ButtonState,
    t: &Theme,
) -> El {
    let (h, glyph, style) = metrics(size);
    let mut b = row()
        .key(keys.root)
        .role("button")
        .label(label)
        .h(h)
        .px(PX)
        .gap(GAP)
        .center()
        .radius(radius::MD);
    let ink = match variant {
        Variant::Primary => {
            let fill = t.accent;
            b = b
                .fill(fill)
                .hover(fill.mix(Color::BLACK, 0.05))
                .press(fill.mix(Color::BLACK, 0.10));
            t.accent_text_on
        }
        Variant::Secondary => {
            b = b
                .fill(t.paper)
                .border(1.0, t.card_border)
                .border_layout()
                .hover(t.hover)
                .press(t.press);
            t.text
        }
        Variant::Ghost => {
            b = b.hover(t.hover).press(t.press);
            t.text_secondary
        }
        Variant::Danger => {
            let fill = tokens::red_fill();
            b = b
                .fill(fill)
                .hover(fill.mix(Color::BLACK, 0.05))
                .press(fill.mix(Color::BLACK, 0.10));
            t.white
        }
    };
    if matches!(state, ButtonState::Dim | ButtonState::Disabled) {
        b = b.opacity(0.5);
    }
    b = if matches!(state, ButtonState::Busy | ButtonState::Disabled) {
        // `inert`: a press the caller chains runs neither from the pointer nor from the keyboard.
        b.disabled(true).inert()
    } else {
        b.focusable(true)
            .focus_ring(super::focus::RING_WIDTH, super::focus::RING_OFFSET, t.accent_deep)
            .cursor(rune::Cursor::Pointer)
    };
    if let Some(ic) = icon {
        b = b.child(kit::glyph(keys.icon, ic, glyph, ink));
    }
    b.child(kit::label(keys.label, label, style, ink))
}

/// The outlined button ("Add files" in an empty folder): Secondary, Md, Enabled. The caller adds
/// `.on_press(..)`.
pub fn outline_button(key: &str, label: &str, ic: Option<&'static Icon>, t: &Theme) -> El {
    button(keys(key), label, ic, Variant::Secondary, Size::Md, ButtonState::Enabled, t)
}

#[cfg(test)]
#[path = "button_tests.rs"]
mod tests;
