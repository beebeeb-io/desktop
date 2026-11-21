//! Contrast ratio (stub: filled by drive-complete-024) and the system's contrast theme (035).
#![allow(dead_code, unused_variables)]

use super::tokens::Theme;
use rune::Color;

/// WCAG contrast ratio of two colours (1.0 until filled).
pub fn contrast_ratio(a: Color, b: Color) -> f32 {
    1.0
}

/// The Windows contrast theme as a [`Theme`]; `None` elsewhere (and until filled).
pub fn system_contrast() -> Option<Theme> {
    None
}
