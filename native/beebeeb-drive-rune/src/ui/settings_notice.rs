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
/// 11px/600 on a 17px line, tracked .01em: a pill's word (`.kit-pill`).
pub const PILL: TextStyle = TextStyle::new(11.0, 600.0).lh(17.0).tracking(0.11);
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
    note_with(index, tone, icon, strong, words, None, t)
}

/// `note` with one action at its trailing edge (a small button of the caller's).
pub fn note_with(
    index: usize,
    tone: Tone,
    icon: &'static Icon,
    strong: &str,
    words: &str,
    action: Option<El>,
    t: &Theme,
) -> El {
    let key = format!("note-{index}");
    let fill = match tone {
        Tone::Neutral => t.text.alpha(0.06),
        // `.mk-banner.tone-red` is the red at 10 %, `.tone-warn` the warning ink at 12 %.
        Tone::Red => t.red.alpha(0.10),
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
    let mut line = row()
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
        );
    if let Some(action) = action {
        line = line.child(action);
    }
    el().key(format!("{key}-margin")).w_full().pt(10.0).px(14.0).child(line)
}

/// A toast over the pane (`.mk-toast`): a pill, centred, 42 pt above the pane's foot. It
/// says that something the person just did took effect. The caller adds it as an overlay.
pub fn toast(index: usize, words: &str, t: &Theme) -> El {
    toast_on(index, words, 0, t)
}

/// How far apart two lines of toasts are (a pill is 34 pt).
const TOAST_LINE: f32 = 42.0;

/// Something that was not done, or that a pane could not read or confirm, said where the toast sits
/// (`ui::toast`: its own mark, read at once), with its button when there is a way to go on. The one rule of the
/// account's panes and of Notifications: it is over the pane, so it is in view however far the pane is scrolled,
/// and no row moves when it comes or goes. `key`: the line's key; `line`: 0 is the toast's own place, 1 the line
/// above it.
pub fn standing(key: &str, words: &str, action: Option<crate::ui::toast::ToastAction>, line: usize, t: &Theme) -> El {
    row()
        .key(key.to_string())
        .pin(
            Some(super::settings_shell::SIDEBAR_W),
            None,
            Some(0.0),
            Some(42.0 + TOAST_LINE * line as f32),
        )
        .justify_center()
        .child(crate::ui::toast::toast(
            &ic::WARN_CIRCLE,
            crate::ui::toast::Tone::Failed,
            words,
            action,
            t,
        ))
}

/// The room a pane leaves under its last row for `lines` lines of toasts that stay (`standing`), so the row can
/// be scrolled clear of them. Key: `toast-room`.
pub fn toast_room(lines: usize) -> El {
    el().key("toast-room").w_full().h(34.0 + TOAST_LINE * lines as f32)
}

/// [`toast`] on line `line` (0: its own place; 1: above a notice that stays).
pub fn toast_on(index: usize, words: &str, line: usize, t: &Theme) -> El {
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
        .pin(
            Some(super::settings_shell::SIDEBAR_W),
            None,
            Some(0.0),
            Some(42.0 + TOAST_LINE * line as f32),
        )
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
    tile_sized(index, tone, icon, 36.0, t)
}

/// The same tile `size` points across (40 on an empty state): corners and glyph in proportion.
pub fn tile_sized(index: usize, tone: Tone, icon: &'static Icon, size: f32, t: &Theme) -> El {
    let ink = tone_ink(tone, t);
    let fill = match tone {
        Tone::Neutral => t.text.alpha(0.07),
        _ => ink.alpha(0.12),
    };
    el().key(format!("tile-{index}"))
        .size(size, size)
        .radius(if size >= 40.0 { 11.0 } else { 10.0 })
        .fill(fill)
        .center()
        .child(kit::glyph(format!("tile-{index}-icon"), icon, size / 2.0, ink))
}

/// A small pill with a dot (`.kit-pill.tone-neutral`): "New". Keys: `pill-<N>`, `-dot`, `-label`.
pub fn pill(index: usize, words: &str, t: &Theme) -> El {
    let key = format!("pill-{index}");
    row()
        .key(key.clone())
        .h(20.0)
        .px(8.0)
        .gap(5.0)
        .center_y()
        .radius(10.0)
        .fill(t.text.alpha(0.07))
        .child(super::settings_meter::disc(
            &format!("{key}-dot"),
            6.0,
            t.text_secondary,
        ))
        .child(kit::label(format!("{key}-label"), words, PILL, t.text_secondary))
}

/// The card for a pane with nothing to show (an empty state): a tile, what is empty, and
/// what will show up here. Centred.
pub fn empty(card: usize, tile_index: usize, icon: &'static Icon, head: &str, words: &str, t: &Theme) -> El {
    // `max-width: 46ch`.
    empty_sized(card, tile_index, icon, head, words, 348.23, t)
}

/// `empty` with the words wrapped in `max_w` points (the board sets it per frame, in `ch`).
pub fn empty_sized(
    card: usize,
    tile_index: usize,
    icon: &'static Icon,
    head: &str,
    words: &str,
    max_w: f32,
    t: &Theme,
) -> El {
    col()
        .key(format!("card-{card}"))
        .role("status")
        .label(format!("{head}. {words}"))
        .w_full()
        .items_center()
        .py(44.0)
        .px(36.0)
        .gap(8.0)
        .radius(crate::theme::tokens::radius::LG)
        .fill(t.card)
        .box_shadow(kit::ring(kit::HAIR, t.separator))
        .box_shadow(kit::drop(1.0, 2.0, Color::BLACK.alpha(0.04)))
        .child(tile_sized(tile_index, Tone::Neutral, icon, 40.0, t))
        .child(kit::label(
            format!("card-{card}-head"),
            head,
            crate::theme::tokens::type_::HEAD,
            t.text,
        ))
        .child(
            kit::label(
                format!("card-{card}-sub"),
                words,
                super::settings_shell::SUB,
                t.text_tertiary,
            )
            .wrap()
            .align(rune::Align::Center)
            .w(max_w),
        )
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
