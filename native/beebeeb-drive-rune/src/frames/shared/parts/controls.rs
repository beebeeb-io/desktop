//! Buttons (`.kit-btn`), worded segments (`.kit-seg`) and pills (`.kit-pill`).

use super::{green_text, red_text, style};
use crate::theme::tokens::{Theme, radius, type_};
use crate::ui::kit;
use rune::{Color, El, Icon, el, row};

fn slug(words: &str) -> String {
    let mut out = String::new();
    for c in words.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_string()
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ButtonKind {
    /// Paper with a drawn line.
    Outline,
    /// Words only, in red: a destructive action that still asks.
    GhostDanger,
    /// Words only, unavailable for now: drawn at 40 %, not a Tab stop.
    GhostOff,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ButtonSize {
    /// `.kit-btn-sm`: 22 pt, in a toolbar or a card.
    Small,
    /// `.kit-btn-md`: 28 pt, an empty state's or a dialog's.
    Medium,
}

/// A small button (`.kit-btn.kit-btn-sm`, 22 pt). Keys: `btn-<words>`, `-icon`, `-label`.
pub fn button(words: &str, kind: ButtonKind, ic: Option<&'static Icon>, t: &Theme) -> El {
    button_keyed(format!("btn-{}", slug(words)), words, kind, ButtonSize::Small, ic, t)
}

/// A button under the key `k` (the second button with the same words in a frame is `btn-<words>-2`).
pub fn button_keyed(
    k: String,
    words: &str,
    kind: ButtonKind,
    size: ButtonSize,
    ic: Option<&'static Icon>,
    t: &Theme,
) -> El {
    let ink = match kind {
        ButtonKind::Outline => t.text,
        ButtonKind::GhostDanger => red_text(t),
        ButtonKind::GhostOff => t.text_secondary,
    };
    let (h, glyph, type_style) = match size {
        ButtonSize::Small => (22.0, 14.0, type_::BUTTON_SM),
        ButtonSize::Medium => (28.0, 16.0, style::BUTTON_MD),
    };
    let mut b = row()
        .key(k.clone())
        .role("button")
        .label(words)
        .h(h)
        .px(12.0)
        .gap(7.0)
        .center()
        .radius(radius::MD);
    b = if kind == ButtonKind::GhostOff {
        b.disabled(true).opacity(0.4)
    } else {
        b.hover(t.hover).press(t.press).focusable(true)
    };
    if kind == ButtonKind::Outline {
        b = b.fill(t.paper).border(1.0, t.card_border).border_layout();
    }
    if let Some(ic) = ic {
        b = b.child(kit::glyph(format!("{k}-icon"), ic, glyph, ink));
    }
    b.child(kit::label(format!("{k}-label"), words, type_style, ink))
}

/// The key of the tab worded `words` in the switch `n`.
pub fn segment_key(n: usize, words: &str) -> String {
    format!("seg-{n}-{}", slug(words))
}

/// A worded segmented control (`.kit-seg.kit-seg-sm`): `tabs` are (words, chosen). Keys:
/// `seg-<n>`, each tab `seg-<n>-<words>` and `-label`. The caller adds each tab's `on_press`
/// through `each`.
pub fn segments(n: usize, name: &str, tabs: &[(&'static str, bool)], t: &Theme, each: impl Fn(usize, El) -> El) -> El {
    let mut well = row()
        .key(format!("seg-{n}"))
        .role("tablist")
        .label(name)
        .padding(2.0)
        .radius(10.0)
        .fill(t.field)
        .box_shadow(kit::ring_inset(t.separator));
    for (i, (words, on)) in tabs.iter().enumerate() {
        let k = segment_key(n, words);
        let mut tab = el()
            .key(k.clone())
            .role("tab")
            .label(*words)
            .checked(*on)
            .h(24.0)
            // The board's 10 pt, less the quarter point Rune adds to a text on average: it reports a
            // text's width rounded up to the next half point ("With me" 48.0 for 47.59), and three
            // tabs in a row would put the control's left edge 1.2 pt off.
            .px(9.75)
            .center()
            .radius(8.0)
            .focusable(true)
            // One Tab stop for the switch: the chosen tab. Left and Right move between tabs.
            .tab_index(if *on { 0 } else { -1 })
            .child(kit::label(
                format!("{k}-label"),
                *words,
                style::SEG,
                if *on { t.text } else { t.text_secondary },
            ));
        if *on {
            tab = tab
                .fill(t.card)
                .box_shadow(kit::drop(1.0, 2.0, t.shadow_raised))
                .box_shadow(kit::ring(kit::HAIR, t.separator));
        } else {
            tab = tab.hover(t.hover).press(t.press);
        }
        well = well.child(each(i, tab));
    }
    well
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Neutral,
    Green,
}

/// A pill (`.kit-pill`), with its dot when the state is live. Keys: `<key>-pill`, `-pill-dot`, `-pill-label`.
pub fn pill(key: &str, words: &str, tone: Tone, dot: bool, t: &Theme) -> El {
    let (wash, ink, dot_ink): (Color, Color, Color) = match tone {
        Tone::Neutral => (t.tile, t.text_secondary, t.text_secondary),
        Tone::Green => (t.green.alpha(0.14), green_text(t), t.green),
    };
    let mut p = row()
        .key(format!("{key}-pill"))
        .h(20.0)
        .px(8.0)
        .gap(5.0)
        .center_y()
        .radius(10.0)
        .fill(wash);
    if dot {
        p = p.child(
            el().key(format!("{key}-pill-dot"))
                .size(6.0, 6.0)
                .radius(3.0)
                .fill(dot_ink),
        );
    }
    // The words at their own width (Rune lays a text out half a point wider at most).
    p.child(kit::label(format!("{key}-pill-label"), words, style::PILL, ink).w(rune::text::advance(words, style::PILL)))
}
