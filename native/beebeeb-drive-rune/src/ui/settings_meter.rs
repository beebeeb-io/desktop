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
    let r = height / 2.0;
    let mut m = row()
        .key(key.clone())
        .role("meter")
        .size(width, height)
        .radius(r)
        .fill(t.separator);
    let mut drawn = 0.0;
    for (j, (share, ink)) in parts.iter().enumerate() {
        let w = width * share.clamp(0.0, 1.0);
        let left = j == 0 && w >= height;
        let right = drawn + w >= width - 0.01 && w >= height;
        drawn += w;
        let part_key = format!("{key}-part-{j}");
        m = m.child(if left || right {
            capped(&part_key, w, height, left, right, *ink)
        } else {
            el().key(part_key).size(w, height).fill(*ink)
        });
    }
    m
}

/// A filled circle as a vector shape. Rune draws a box's rounded corner by distance from its
/// outline (`crates/rune/src/shader.wgsl:180`, `clamp(0.5 - d, 0.0, 1.0)`), a glyph by the
/// area it covers (`outline.wgsl`, `outline_coverage`); on a radius of a few points the two
/// rims differ, and the board's (Chromium's) is by area.
pub static DISC: Icon = Icon {
    name: "disc",
    view: 24.0,
    body: r#"<circle cx="12" cy="12" r="12" fill="currentColor"></circle>"#,
};

/// A small disc (a pill's dot), `d` across, drawn as a vector circle. The keyed box carries
/// the declared fill for the element tree and draws nothing itself. Key: the one given.
pub fn disc(key: &str, d: f32, ink: Color) -> El {
    el().key(format!("{key}-shape"))
        .size(d, d)
        .child(
            el().key(key.to_string())
                .pin(Some(0.0), Some(0.0), None, None)
                .size(d, d)
                .radius(d / 2.0)
                .fill(ink)
                .opacity(0.0),
        )
        .child(
            rune::icon(&DISC, d)
                .key(format!("{key}-disc"))
                .color(ink)
                .absolute(0.0, 0.0),
        )
}

/// A bar `w` by `h` in an opaque ink with a round end at its left, its right or both, the
/// ends drawn as vector circles (see `DISC`). The keyed box carries the declared fill.
fn capped(key: &str, w: f32, h: f32, left: bool, right: bool, ink: Color) -> El {
    let r = h / 2.0;
    let (x0, x1) = (if left { r } else { 0.0 }, if right { w - r } else { w });
    let mut bar = el()
        .key(format!("{key}-shape"))
        .size(w, h)
        .child(
            el().key(key.to_string())
                .pin(Some(0.0), Some(0.0), None, None)
                .size(w, h)
                .fill(ink)
                .opacity(0.0),
        )
        .child(
            el().key(format!("{key}-body"))
                .pin(Some(x0), Some(0.0), None, None)
                .size(x1 - x0, h)
                .fill(ink),
        );
    if left {
        bar = bar.child(
            rune::icon(&DISC, h)
                .key(format!("{key}-cap-l"))
                .color(ink)
                .absolute(0.0, 0.0),
        );
    }
    if right {
        bar = bar.child(
            rune::icon(&DISC, h)
                .key(format!("{key}-cap-r"))
                .color(ink)
                .absolute(w - h, 0.0),
        );
    }
    bar
}

/// A share bar (`.mk-insbar`): one kind's size against the largest kind's.
/// Keys: `insbar-<N>`, `insbar-<N>-fill`.
pub fn bar(index: usize, width: f32, share: f32, ink: Color, t: &Theme) -> El {
    let key = format!("insbar-{index}");
    let w = width * share.clamp(0.0, 1.0);
    let fill_key = format!("{key}-fill");
    // (A 9 pt bar's ends stay Rune's own rounded clip: as vector circles they come out
    // further from the board's than this does; an 8 pt meter's come out nearer.)
    el().key(key.clone())
        .size(width, 9.0)
        .fill(t.text.alpha(0.08))
        .clip_rounded(4.5)
        .child(el().key(fill_key).size(w, 9.0).radius(4.5).fill(ink))
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
            disc("slider-knob", 18.0, Color::WHITE)
                .pin(Some(width * at - 9.0), Some(2.0), None, None)
                // The knob sits where the value is, between device pixels if need be.
                .pixel_snap(false)
                .radius(9.0)
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
