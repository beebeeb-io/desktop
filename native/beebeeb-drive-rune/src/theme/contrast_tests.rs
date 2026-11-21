//! WCAG contrast of the theme's pairs, light and dark. Pairs that fail today are in [`KNOWN_FAILING`];
//! that list may only shrink (drive-complete-035 fixes them): a listed pair that now passes fails the
//! test until it is removed from the list.

use super::contrast_ratio;
use crate::theme::tokens::{Theme, theme};
use rune::Color;

#[test]
fn ratio_matches_wcag() {
    assert!((contrast_ratio(Color::BLACK, Color::WHITE) - 21.0).abs() < 0.01);
    assert!((contrast_ratio(Color::WHITE, Color::WHITE) - 1.0).abs() < 1e-4);
    let (a, b) = (theme(false).text, theme(false).bg);
    assert_eq!(contrast_ratio(a, b), contrast_ratio(b, a));
}

/// (name, foreground, background, minimum ratio).
type Pair = (&'static str, Color, Color, f32);

fn pairs(t: &Theme) -> Vec<Pair> {
    let mut v: Vec<Pair> = Vec::new();
    for (bn, bg) in [("bg", t.bg), ("surface", t.surface), ("card", t.card), ("sidebar_bg", t.sidebar_bg)] {
        v.push(("text", t.text, bg, 4.5));
        v.push(("text_secondary", t.text_secondary, bg, 4.5));
        v.push(("text_tertiary", t.text_tertiary, bg, 4.5));
        v.push(("text_quaternary", t.text_quaternary, bg, 3.0));
        v.push(("icon", t.icon, bg, 3.0));
        v.push(("icon_secondary", t.icon_secondary, bg, 3.0));
        v.push(("card_border", t.card_border, bg, 3.0));
        v.push(("focus_ring", t.accent_deep, bg, 3.0));
        let _ = bn;
    }
    v.push(("accent_text_on", t.accent_text_on, t.accent, 4.5));
    v
}

/// Today's failures: "<light|dark>/<fg>/<bg>". May only shrink.
const KNOWN_FAILING: &[&str] = &[
    "dark/card_border/bg",
    "dark/card_border/surface",
    "dark/text_quaternary/bg",
    "dark/text_quaternary/surface",
    "light/card_border/bg",
    "light/card_border/surface",
    "light/text_quaternary/bg",
    "light/text_quaternary/surface",
];

fn key(dark: bool, p: &Pair, bg_name: &str) -> String {
    format!("{}/{}/{}", if dark { "dark" } else { "light" }, p.0, bg_name)
}

#[test]
fn a11y_walk_contrast_pairs_pass_or_are_listed() {
    let mut failing = Vec::new();
    for dark in [false, true] {
        let t = theme(dark);
        let bgs = [t.bg, t.surface, t.card, t.sidebar_bg];
        let names = ["bg", "surface", "card", "sidebar_bg"];
        for p in pairs(&t) {
            let bg_name = bgs
                .iter()
                .position(|b| *b == p.2)
                .map_or("accent", |i| names[i]);
            let r = contrast_ratio(p.1, p.2);
            if r < p.3 {
                failing.push((key(dark, &p, bg_name), r, p.3));
            }
        }
    }
    failing.sort_by(|a, b| a.0.cmp(&b.0));
    failing.dedup_by(|a, b| a.0 == b.0);
    let new: Vec<_> = failing.iter().filter(|(k, ..)| !KNOWN_FAILING.contains(&k.as_str())).collect();
    let stale: Vec<_> = KNOWN_FAILING.iter().filter(|k| !failing.iter().any(|(f, ..)| f == *k)).collect();
    let listing: String = failing.iter().map(|(k, r, m)| format!("    \"{k}\", // {r:.2} < {m}\n")).collect();
    assert!(new.is_empty(), "pairs below their minimum (add to KNOWN_FAILING only if unfixable now):\n{listing}");
    assert!(stale.is_empty(), "pairs that now pass: remove from KNOWN_FAILING: {stale:?}");
}
