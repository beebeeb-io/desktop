//! Measures on a settings pane: a meter, a share bar, a slider, a chip, a skeleton, and a
//! line of words with a machine's value inside it.
//!
//! | Piece | Function | Keys it gives |
//! | --- | --- | --- |
//! | Meter (`.kit-meter`) | `meter(index, width, height, parts, t)` | `meter-<N>`, `meter-<N>-part-<J>` |
//! | Share bar (`.mk-insbar`) | `bar(index, width, share, ink, t)` | `insbar-<N>`, `insbar-<N>-fill` |
//! | Slider (`.mk-slider`) | `slider(name, width, at, t)` | `slider`, `slider-track`, `slider-fill`, `slider-knob` |
//! | Chip (`.kit-chip`) | `chip(index, icon, words, t)` | `chip-<N>`, `chip-<N>-icon`, `chip-<N>-label` |
//! | Skeleton (`.mk-skel`) | `skeleton(index, width, height, t)` | `skel-<N>` |
//! | Words with a value | `mixed(key, words, mono, style, ink)` | the key |
//!
//! Nothing here is Amber: a measure is ink on a tint of ink.

use crate::theme::tokens::Theme;
use rune::{Color, El, Icon, Span, TextStyle, el, rich, row};

use super::kit;

/// 12px/600 on a 17px line: a chip's words (`.kit-chip`).
pub const CHIP: TextStyle = TextStyle::new(12.0, 600.0).lh(17.0);

/// A meter (`.kit-meter`): a pill-shaped track `width` wide with its parts end to end, each
/// a share of the whole (0 to 1) in its own ink. Keys: `meter-<N>`, `meter-<N>-part-<J>`.
pub fn meter(index: usize, width: f32, height: f32, parts: &[(f32, Color)], t: &Theme) -> El {
    let key = format!("meter-{index}");
    let mut m = row()
        .key(key.clone())
        .role("meter")
        .size(width, height)
        .fill(t.separator)
        .clip_rounded(height / 2.0);
    for (j, (share, ink)) in parts.iter().enumerate() {
        m = m.child(
            el().key(format!("{key}-part-{j}"))
                .size(width * share.clamp(0.0, 1.0), height)
                .fill(*ink),
        );
    }
    m
}

/// A share bar (`.mk-insbar`): one kind's size against the largest kind's.
/// Keys: `insbar-<N>`, `insbar-<N>-fill`.
pub fn bar(index: usize, width: f32, share: f32, ink: Color, t: &Theme) -> El {
    let key = format!("insbar-{index}");
    el().key(key.clone())
        .size(width, 9.0)
        .fill(t.text.alpha(0.08))
        .clip_rounded(4.5)
        .child(
            el().key(format!("{key}-fill"))
                .size(width * share.clamp(0.0, 1.0), 9.0)
                .radius(4.5)
                .fill(ink),
        )
}

/// A slider (`.mk-slider`), `width` wide, its knob `at` (0 to 1) along the track. The caller
/// gives it its value, its steps and its drag (`range_value`, `on_step`, `on_step_limit`,
/// `on_drag`). Keys: `slider`, `slider-track`, `slider-fill`, `slider-knob`.
pub fn slider(name: &str, width: f32, at: f32, t: &Theme) -> El {
    let at = at.clamp(0.0, 1.0);
    el().key("slider")
        .role("slider")
        .label(name)
        .size(width, 22.0)
        .focusable(true)
        .child(
            el().key("slider-track")
                .pin(Some(0.0), Some(9.0), None, None)
                .size(width, 4.0)
                .radius(2.0)
                .fill(t.text.alpha(0.12)),
        )
        .child(
            el().key("slider-fill")
                .pin(Some(0.0), Some(9.0), None, None)
                .size(width * at, 4.0)
                .radius(2.0)
                .fill(t.text_secondary),
        )
        .child(
            el().key("slider-knob")
                .pin(Some(width * at - 9.0), Some(2.0), None, None)
                .size(18.0, 18.0)
                .radius(9.0)
                .fill(Color::WHITE)
                .box_shadow(kit::drop(1.0, 3.0, Color::BLACK.alpha(0.30)))
                .box_shadow(kit::ring(kit::HAIR, t.separator)),
        )
}

/// A chip (`.kit-chip.tone-neutral`): a word on a pill of ink tint, a glyph before it where
/// the board has one. Keys: `chip-<N>`, `chip-<N>-icon`, `chip-<N>-label`.
pub fn chip(index: usize, icon: Option<&'static Icon>, words: &str, t: &Theme) -> El {
    let key = format!("chip-{index}");
    let mut c = row()
        .key(key.clone())
        .h(26.0)
        .px(10.0)
        .gap(5.0)
        .center_y()
        .radius(13.0)
        .fill(t.text.alpha(0.07));
    if let Some(icon) = icon {
        c = c.child(kit::glyph(format!("{key}-icon"), icon, 13.0, t.text_secondary));
    }
    c.child(kit::label(format!("{key}-label"), words, CHIP, t.text_secondary))
}

/// A skeleton (`.mk-skel`): the shape of something that is still loading. Key: `skel-<N>`.
pub fn skeleton(index: usize, width: Option<f32>, height: f32, t: &Theme) -> El {
    let s = el()
        .key(format!("skel-{index}"))
        .h(height)
        .radius(5.0)
        .fill(t.text.alpha(0.08));
    match width {
        Some(width) => s.w(width),
        None => s.w_full(),
    }
}

/// A line of words with machine values inside it (`mono`: byte ranges drawn in JetBrains
/// Mono), under one key: the board's element holds both, so the gate reads it as one text.
pub fn mixed(key: &str, words: &str, mono: &[std::ops::Range<usize>], style: TextStyle, ink: Color) -> El {
    kit::note_text(key, key, words, style, ink);
    let spans = mono.iter().map(|r| Span::new(r.start, r.end).mono()).collect();
    rich(words, spans).key(key).font(style).color(ink)
}

/// `mixed` for the common case: the value comes first, the words follow ("240 GB used").
pub fn value_then(key: &str, value: &str, words: &str, style: TextStyle, ink: Color) -> El {
    mixed(key, &format!("{value}{words}"), &[0..value.len()], style, ink)
}

/// `mixed` with the value last ("of 1 TB").
pub fn then_value(key: &str, words: &str, value: &str, style: TextStyle, ink: Color) -> El {
    mixed(
        key,
        &format!("{words}{value}"),
        &[words.len()..words.len() + value.len()],
        style,
        ink,
    )
}
