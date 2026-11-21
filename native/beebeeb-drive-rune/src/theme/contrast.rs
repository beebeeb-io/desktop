//! Contrast ratio (stub: filled by drive-complete-024) and the system's contrast theme (035).
#![allow(dead_code, unused_variables)]

use super::tokens::Theme;
use rune::Color;

/// WCAG 2 relative luminance of an opaque colour (rune's `Color` is already linear sRGB).
fn relative_luminance(c: Color) -> f32 {
    0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b
}

/// `top` laid over an opaque `under`, in linear light.
fn over(top: Color, under: Color) -> Color {
    let k = |t: f32, u: f32| t * top.a + u * (1.0 - top.a);
    Color { r: k(top.r, under.r), g: k(top.g, under.g), b: k(top.b, under.b), a: 1.0 }
}

/// WCAG contrast ratio of two colours, 1.0 to 21.0, order-free. A translucent `b` is laid over white,
/// then a translucent `a` over that.
pub fn contrast_ratio(a: Color, b: Color) -> f32 {
    let b = over(b, Color::WHITE);
    let a = over(a, b);
    let (la, lb) = (relative_luminance(a), relative_luminance(b));
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

#[cfg(test)]
#[path = "contrast_tests.rs"]
mod tests;

/// The Windows contrast theme as a [`Theme`]; `None` elsewhere (and until filled).
pub fn system_contrast() -> Option<Theme> {
    None
}
