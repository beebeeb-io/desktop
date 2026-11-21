//! The one status pill: words on a wash, with or without a dot, read as a status.
//!
//! Keys: `<key>` (the pill), `<key>-dot` (where drawn), `<key>-label`.

use crate::theme::tokens::{Theme, type_};
use rune::{Color, El, el, row};

use super::kit;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Neutral,
    Good,
    Warn,
    Bad,
    Accent,
}

/// The one tone table: the wash and the ink (the dot is the ink).
pub fn colors(tone: Tone, t: &Theme) -> (Color, Color) {
    match tone {
        Tone::Neutral => (t.badge, t.text_secondary),
        Tone::Good => (t.green_bg, t.green),
        Tone::Warn => (t.accent_wash, t.accent_deep),
        Tone::Bad => (t.red.alpha(0.10), t.red),
        Tone::Accent => (t.accent_bg, t.accent_deep),
    }
}

pub fn pill(key: &str, words: &str, tone: Tone, dot: bool, t: &Theme) -> El {
    let (fill, ink) = colors(tone, t);
    let mut p = row()
        .key(key)
        .role("status")
        .label(words)
        .h(18.0)
        .px(7.0)
        .gap(5.0)
        .center_y()
        .radius(9.0)
        .fill(fill);
    if dot {
        p = p.child(el().key(format!("{key}-dot")).w(6.0).h(6.0).radius(3.0).fill(ink));
    }
    p.child(kit::label(format!("{key}-label"), words, type_::BADGE, ink))
}

#[cfg(test)]
#[path = "pill_tests.rs"]
mod tests;
