//! What a settings pane says about its own state: a notice in the column, a toast over the
//! pane, a toned tile, and the card for something that could not load.
//!
//! | Piece | Function | Keys it gives |
//! | --- | --- | --- |
//! | Notice (`.mk-banner`) | `note(index, Tone, icon, strong, words, t)` | `note-<N>`, `note-<N>-icon`, `note-<N>-text` |
//! | Toast (`.mk-toast > .kit-toast`) | `toast(index, words, t)` | `toast-<N>`, `toast-<N>-icon`, `toast-<N>-text` |
//! | Toned tile (`.kit-tile`) | `tile(index, Tone, icon, t)` | `tile-<N>`, `tile-<N>-icon` |
//! | Could-not-load card | `problem(card, tile, head, words, action, t)` | the card's and the tile's keys |
//!
//! A tone is a meaning, not a decoration: `Warn` for something the person should know
//! about and can live with, `Red` for something that failed. Neither is Amber.

use crate::theme::icons_settings as ic;
use crate::theme::tokens::Theme;
use rune::{Color, El, Icon, Span, TextStyle, col, el, oklch, rich, row};

use super::kit;
use super::settings_card as cards;

/// 12.5px/400 on a 17px line: a notice's words (`.mk-banner`).
pub const NOTE: TextStyle = TextStyle::new(12.5, 400.0).lh(17.0);
/// 12.5px/550 on a 17px line: a toast's words (`.mk-toast .kit-toast`).
pub const TOAST: TextStyle = TextStyle::new(12.5, 550.0).lh(17.0);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tone {
    /// Informs (`.mk-banner` without a tone, `.tone-neutral`).
    Neutral,
    /// Worth knowing, not a failure (`.tone-warn`: `--mk-warn`).
    Warn,
    /// Failed (`.tone-red`).
    Red,
}

/// `--mk-warn`: Amber deep 45 % into red, in oklch.
pub fn warn(t: &Theme) -> Color {
    if t.dark {
        oklch(0.6995, 0.183, 46.15)
    } else {
        oklch(0.575, 0.1785, 46.15)
    }
}

fn tone_ink(tone: Tone, t: &Theme) -> Color {
    match tone {
        Tone::Neutral => t.text_secondary,
        Tone::Warn => warn(t),
        Tone::Red => t.red,
    }
}

/// A notice in a pane's column (`.mk-banner`): a glyph, a strong first sentence, the rest.
/// It is 14 pt in from the column's sides and 10 pt lower than the column's gap puts it.
pub fn note(index: usize, tone: Tone, icon: &'static Icon, strong: &str, words: &str, t: &Theme) -> El {
    let key = format!("note-{index}");
    let fill = match tone {
        Tone::Neutral => t.text.alpha(0.06),
        tone => tone_ink(tone, t).alpha(0.12),
    };
    let all = if words.is_empty() {
        strong.to_string()
    } else {
        format!("{strong} {words}")
    };
    let text_key = format!("{key}-text");
    kit::note_text(&text_key, &text_key, &all, NOTE, t.text_secondary);
    let spans = vec![Span::new(0, strong.len()).weight(600.0).color(t.text)];
    el().key(format!("{key}-margin")).w_full().pt(10.0).px(14.0).child(
        row()
            .key(key.clone())
            .role("status")
            .label(all.clone())
            .w_full()
            .center_y()
            .gap(10.0)
            .py(9.0)
            .pl(12.0)
            .pr(10.0)
            .radius(10.0)
            .fill(fill)
            .child(kit::glyph(format!("{key}-icon"), icon, 16.0, tone_ink(tone, t)))
            .child(
                rich(all, spans)
                    .key(text_key)
                    .font(NOTE)
                    .color(t.text_secondary)
                    .grow(1.0)
                    .min_w(0.0),
            ),
    )
}

/// A toast over the pane (`.mk-toast`): a pill, centred, 42 pt above the pane's foot. It
/// says that something the person just did took effect. The caller adds it as an overlay.
pub fn toast(index: usize, words: &str, t: &Theme) -> El {
    let key = format!("toast-{index}");
    // `--kit-glass`: the paper, see-through. (The board blurs what is behind it; behind a
    // toast is the pane's flat ground.)
    let glass = if t.dark {
        t.paper2.alpha(0.68)
    } else {
        t.paper.alpha(0.72)
    };
    row()
        .key(format!("{key}-line"))
        .pin(Some(super::settings_shell::SIDEBAR_W), None, Some(0.0), Some(42.0))
        .justify_center()
        .child(
            row()
                .key(key.clone())
                .role("status")
                .label(words)
                .h(34.0)
                .pl(14.0)
                .pr(8.0)
                .gap(10.0)
                .center_y()
                .radius(17.0)
                .fill(glass)
                .border(kit::HAIR, t.separator)
                .border_layout()
                .box_shadow(kit::drop(6.0, 22.0, Color::BLACK.alpha(0.10)))
                // `inset 0 .5px 0 var(--kit-hi)`: the glass catches the light along its top edge.
                .box_shadow(rune::Shadow {
                    x: 0.0,
                    y: kit::HAIR,
                    blur: 0.0,
                    spread: 0.0,
                    color: Color::WHITE.alpha(if t.dark { 0.08 } else { 0.70 }),
                    inset: true,
                })
                .child(kit::glyph(format!("{key}-icon"), &ic::CHECK, 15.0, t.green))
                .child(kit::label(format!("{key}-text"), words, TOAST, t.text)),
        )
}

/// A glyph on its tone's tint (`.kit-tile`), 36 pt, corners 10. Keys: `tile-<N>`, `tile-<N>-icon`.
pub fn tile(index: usize, tone: Tone, icon: &'static Icon, t: &Theme) -> El {
    let ink = tone_ink(tone, t);
    let fill = match tone {
        Tone::Neutral => t.tile,
        _ => ink.alpha(0.12),
    };
    el().key(format!("tile-{index}"))
        .size(36.0, 36.0)
        .radius(10.0)
        .fill(fill)
        .center()
        .child(kit::glyph(format!("tile-{index}-icon"), icon, 18.0, ink))
}

/// The card for something that could not load: what failed, what is unchanged, and one
/// way to try again (the caller's button). Says what happened; never blames the person.
pub fn problem(card: usize, tile_index: usize, head: &str, words: &str, action: Option<El>, t: &Theme) -> El {
    let mut text = col()
        .key(format!("card-{card}-words"))
        .grow(1.0)
        .min_w(0.0)
        .gap(4.0)
        .child(cards::head(card, head, t))
        .child(cards::sub(card, words, t).wrap());
    if let Some(action) = action {
        text = text.child(row().key(format!("card-{card}-actions")).pt(8.0).child(action));
    }
    cards::card(card, t)
        .items_start()
        .role("alert")
        .label(format!("{head}. {words}"))
        .child(tile(tile_index, Tone::Red, &ic::WARN_CIRCLE, t))
        .child(text)
}
