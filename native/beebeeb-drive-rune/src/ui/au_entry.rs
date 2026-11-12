//! What a person types a secret into on the account screens, beyond `au::field`: the six digit
//! boxes of a one-time code (`.au-digit`), the twelve numbered boxes of a recovery phrase
//! (`.au-word`), and a field drawn as the board draws one that holds the keyboard.
//!
//! Secrets. A digit, a recovery word and a backup code are drawn by Rune's secure text alone
//! (`text(..).secure().revealed(true)` and a secure `TextEdit`): no accessibility text, no
//! inspector text, no shared layout cache, wiped on drop. Nothing here puts one in an element
//! label, measures one with `rune::text::advance` (the shared cache) or copies one into a
//! `String`: a mono text's width is its character count times the font's fixed advance. The
//! frame gate is told the characters only while its export records (`kit::note_text` copies
//! inside the recording branch), and a gate export only ever runs on a fixture scene.
//!
//! Focus. Rune gives a view no way to ask which element holds the keyboard, and a headless
//! render has no focus at all, so each piece has two ways to look focused: the real one
//! (`focus_within_ring`, Rune's own caret) and `pictured`, which draws the board's focused box
//! and caret as plain shapes. Only a scene sets `pictured`; the first press or key clears it.

use rune::tok::Family;
use rune::{Color, El, Icon, Shadow, TextEdit, TextStyle, Tx, col, el, row, text, text_input};

use super::au::{self, Keys, type_au};
use super::kit;
use crate::State;
use crate::theme::tokens::Theme;

pub mod type_entry {
    use rune::TextStyle;
    use rune::tok::Family;

    /// `.au-digit`: JetBrains Mono 22px/600 on its normal line (29.04px).
    pub const DIGIT: TextStyle = TextStyle::new(22.0, 600.0).family(Family::Mono).lh(29.04);
    /// `.au-word em`: mono 13px on 17px.
    pub const WORD: TextStyle = TextStyle::new(13.0, 400.0).family(Family::Mono).lh(17.0);
    /// `.au-word span`: the box's number, mono 11px on 17px.
    pub const WORD_N: TextStyle = TextStyle::new(11.0, 400.0).family(Family::Mono).lh(17.0);
}

/// JetBrains Mono's advance, as a fraction of the font size: every character is this wide.
const MONO_ADVANCE: f32 = 0.6;

/// The width of `chars` characters of a mono style, without measuring the text itself.
pub fn mono_width(chars: usize, style: TextStyle) -> f32 {
    debug_assert!(matches!(style.family, Family::Mono));
    chars as f32 * MONO_ADVANCE * style.size
}

fn inset(width: f32, color: Color) -> Shadow {
    Shadow {
        x: 0.0,
        y: 0.0,
        blur: 0.0,
        spread: width,
        color,
        inset: true,
    }
}

/// The caret the board draws in a focused field (`.kit-caret`): 1.5 by 17, in deep amber.
fn pictured_caret(key: String, x: f32, y: f32, t: &Theme) -> El {
    el().key(key)
        .a11y_hidden()
        .absolute(x, y)
        .size(1.5, 17.0)
        .radius(1.0)
        .fill(t.accent_deep)
}

/// A secret the screen shows as it was typed (a digit of the code): a secure text node keyed
/// `<key>-box`. The gate is told its characters under `key`, while it records.
pub fn shown_secret(key: &str, content: &str, style: TextStyle, color: Color) -> El {
    let box_key = format!("{key}-box");
    kit::note_text(key, &box_key, content, style, color);
    text(content)
        .secure()
        .revealed(true)
        .key(box_key)
        .font(style)
        .color(color)
        .a11y_hidden()
}

// ---------------------------------------------------------------------------
// The one-time code (`.au-digits`)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DigitsState {
    /// The box of the next digit is lit.
    Entering,
    /// The code is being checked: the digits are dimmed (DW-A26's rule).
    Checking,
    /// DW-A27: every box ringed in red.
    Wrong,
}

/// How wide `count` digit boxes are, with the 8 between them.
pub fn digits_width(count: usize) -> f32 {
    count as f32 * 44.0 + count.saturating_sub(1) as f32 * 8.0
}

/// The boxes that display a one-time code: `count` of them, 44 by 52, the typed digits in the
/// first ones. They are a picture of the one input above them (`code_input`), hidden from
/// assistive technology as the board asks ("the boxes are aria-hidden").
pub fn digits(code: &str, count: usize, state: DigitsState, t: &Theme) -> El {
    let typed = code.chars().count();
    let mut boxes = row().key("au-digits").a11y_hidden().gap(8.0);
    let mut at = code.char_indices();
    for i in 0..count {
        let digit = at.next().map(|(from, c)| &code[from..from + c.len_utf8()]);
        let mut b = el().key(format!("digit-{i}")).size(44.0, 52.0).radius(12.0).center();
        b = match state {
            DigitsState::Wrong => b.fill(t.field).box_shadow(inset(1.5, t.red)),
            DigitsState::Entering if i == typed => b
                .fill(t.card)
                .box_shadow(kit::ring(3.0, t.accent.alpha(0.4)))
                .box_shadow(inset(1.0, t.accent_deep)),
            _ => b.fill(t.field).box_shadow(kit::ring_inset(t.separator)),
        };
        if let Some(digit) = digit {
            let ink = if state == DigitsState::Checking {
                t.text_tertiary
            } else {
                t.text
            };
            b = b.child(
                shown_secret(&format!("digit-{i}-text"), digit, type_entry::DIGIT, ink)
                    .w(mono_width(1, type_entry::DIGIT)),
            );
        }
        boxes = boxes.child(b);
    }
    boxes
}

/// The one input a code is typed into, laid over its boxes and drawn in no colour: the boxes
/// are its display. `edit` is a secure `TextEdit`, so the code is in no tree.
pub fn code_input(
    edit: &TextEdit,
    input_key: &'static str,
    name: &str,
    count: usize,
    on_edit: impl for<'a> Fn(&'a mut State) -> &'a mut TextEdit + 'static,
) -> El {
    text_input(edit, type_entry::DIGIT)
        .key(input_key)
        .label(name)
        .single_line()
        .color(Color::TRANSPARENT)
        .caret(Color::TRANSPARENT, Color::TRANSPARENT)
        .on_edit(on_edit)
        .absolute(0.0, 0.0)
        .size(digits_width(count), 52.0)
}

// ---------------------------------------------------------------------------
// The recovery phrase (`.au-words` > `.au-word`)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WordState {
    Rest,
    /// The board's focused box: paper, a 2px deep amber ring, a caret after the word.
    Pictured,
    /// DW-A24: not a word of the list. Ringed in red and announced.
    NotAWord,
}

/// A word box's width: a third of the 344 column, 6 between the boxes.
pub const WORD_WIDTH: f32 = (344.0 - 12.0) / 3.0;

/// How tall a word box is by itself, as the board's CSS lays it out (its number and its word
/// sit on one baseline, in two sizes): 33 empty, 34 with a word, 35 with the pictured caret.
pub fn word_height(filled: bool, state: WordState) -> f32 {
    match (filled, state) {
        (false, WordState::Pictured) => 35.0,
        (true, _) => 34.0,
        (false, _) => 33.0,
    }
}

/// One numbered box of the phrase, `h` tall (its row's height). `input` is the box's text
/// input over a secure, revealed `TextEdit` (the caller makes it, with its handlers).
///
/// Keys: `word-<i>`, `word-<i>-n`, `word-<i>-text` (of `word-<i>-text-box`), `word-<i>-caret`.
pub fn word_box(i: usize, edit: &TextEdit, input: El, state: WordState, h: f32, t: &Theme) -> El {
    let key = format!("word-{i}");
    let filled = !edit.text.is_empty();
    let chars = edit.text.chars().count();
    let width = mono_width(chars, type_entry::WORD);
    // The number sits on the word's baseline: 1 lower beside a word, 2 beside the caret.
    let n_top = match (filled, state) {
        (false, WordState::Pictured) => 10.0,
        (true, _) => 9.0,
        (false, _) => 8.0,
    };
    let mut b = el()
        .key(key.clone())
        .w(WORD_WIDTH)
        .h(h)
        .radius(11.0)
        .child(
            kit::label(
                format!("{key}-n"),
                (i + 1).to_string(),
                type_entry::WORD_N,
                t.text_tertiary,
            )
            .a11y_hidden()
            .absolute(10.0, n_top)
            .w(16.0),
        )
        .child(input.absolute(32.0, 8.0).w(WORD_WIDTH - 42.0).h(17.0));
    b = match state {
        WordState::Rest => b
            .fill(t.field)
            .box_shadow(kit::ring_inset(t.separator))
            .focus_within_ring(2.0, 0.0, t.accent_deep),
        WordState::Pictured => b.fill(t.card).box_shadow(kit::ring(2.0, t.accent_deep)),
        WordState::NotAWord => {
            b.fill(t.field)
                .box_shadow(kit::ring(1.5, t.red))
                .focus_within_ring(2.0, 0.0, t.accent_deep)
        }
    };
    if filled {
        kit::note_text(
            &format!("{key}-text"),
            &format!("{key}-text-box"),
            &edit.text,
            type_entry::WORD,
            t.text,
        );
        b = b.child(
            el().key(format!("{key}-text-box"))
                .a11y_hidden()
                .absolute(32.0, 8.0)
                .size(width, 17.0),
        );
    }
    if state == WordState::Pictured {
        // The board's caret sits inside the word's own box (`em`, `overflow: hidden`), which
        // cuts it at 1.1 times the 13px text: 17 tall as an element, 14.3 as drawn.
        b = b.child(
            el().a11y_hidden()
                .absolute(33.0 + width, 8.0)
                .size(1.5, 14.3)
                .clip_rounded(0.0)
                .child(pictured_caret(format!("{key}-caret"), 0.0, 0.0, t)),
        );
    }
    b
}

/// The text input of one word box. The caller adds `.on_edit`, `.on_input_change` and
/// `.on_input_paste`.
pub fn word_input(edit: &TextEdit, input_key: &'static str, i: usize, not_a_word: bool, t: &Theme) -> El {
    let mut input = text_input(edit, type_entry::WORD)
        .key(input_key)
        .label(format!("Recovery word {}", i + 1))
        .single_line()
        .color(t.text)
        .caret(t.accent_deep, t.selection)
        .invalid(not_a_word);
    if not_a_word {
        input = input.description("not a recovery word");
    }
    input
}

// ---------------------------------------------------------------------------
// A field as the board draws it holding the keyboard
// ---------------------------------------------------------------------------

/// What `focused_field` draws beside the text.
pub struct FocusedSpec<'a> {
    pub label: &'a str,
    pub icon: Option<&'static Icon>,
    /// The words under the field when it has a problem (red, an alert).
    pub error: Option<&'a str>,
    /// Mono text (a backup code).
    pub mono: bool,
    /// Draw the board's focused look and caret as shapes (a scene: see the module's note).
    pub pictured: bool,
}

/// `au::field`, for a screen whose one field holds the keyboard when it opens (DW-A28,
/// DW-A30): the 1px deep amber line inside a 3px amber ring, on real focus and when `pictured`.
///
/// A secure `edit` that is hidden gives the gate its bullets; one that is revealed (a backup
/// code, shown as typed) gives it the characters while the gate records, and is measured by
/// count (`mono` is then required).
#[allow(clippy::too_many_arguments)]
pub fn focused_field(
    keys: &mut Keys,
    spec: FocusedSpec<'_>,
    edit: &TextEdit,
    input_key: &'static str,
    on_edit: impl for<'a> Fn(&'a mut State) -> &'a mut TextEdit + 'static,
    on_change: impl Fn(&mut State, &mut Tx) + 'static,
    trailing: Option<El>,
    t: &Theme,
) -> El {
    let key = keys.next("field");
    let hidden = edit.is_secure() && !edit.is_revealed();
    let style = if hidden {
        type_au::FIELD_SECURE
    } else if spec.mono {
        type_au::FIELD_MONO
    } else {
        type_au::FIELD_VALUE
    };
    let chars = edit.text.chars().count();
    // What the gate is told, and how wide the typed text is.
    let width = match au::gate_text(edit) {
        Some(shown) if !shown.is_empty() => {
            kit::note_text(
                &format!("{key}-value"),
                &format!("{key}-value-box"),
                &shown,
                style,
                t.text,
            );
            rune::text::advance(&shown, style)
        }
        None if chars > 0 => {
            kit::note_text(
                &format!("{key}-value"),
                &format!("{key}-value-box"),
                &edit.text,
                style,
                t.text,
            );
            mono_width(chars, style)
        }
        _ => 0.0,
    };
    let error = spec.error.is_some();
    let mut input = text_input(edit, style)
        .key(input_key)
        .label(spec.label)
        .single_line()
        .color(t.text)
        .caret(t.accent_deep, t.selection)
        .invalid(error)
        .on_edit(on_edit)
        .on_input_change(on_change)
        .grow(1.0)
        .min_w(0.0);
    if let Some(message) = spec.error {
        input = input.description(message);
    }
    let mut well = row()
        .key(key.clone())
        .w_full()
        .h(28.0)
        .px(8.0)
        .gap(6.0)
        .center_y()
        .radius(6.0)
        .border_layout();
    well = if error {
        well.fill(t.field)
            .border(1.0, t.red)
            .box_shadow(kit::ring(3.5, t.red.alpha(0.16)))
    } else if spec.pictured {
        well.fill(t.card)
            .border(1.0, t.accent_deep)
            .box_shadow(kit::ring(3.0, t.accent.alpha(0.30)))
    } else {
        well.fill(t.field)
            .border(1.0, Color::TRANSPARENT)
            .box_shadow(kit::ring_inset(t.separator))
            .focus_within_ring(1.0, -1.0, t.accent_deep)
    };
    if let Some(icon) = spec.icon {
        well = well.child(kit::glyph(format!("{key}-icon"), icon, 16.0, t.text_tertiary));
    }
    let mut typed = row().grow(1.0).min_w(0.0).h_full().center_y().child(input).child(
        el().key(format!("{key}-value-box"))
            .a11y_hidden()
            .absolute(0.0, 0.0)
            .w(width)
            .h_full(),
    );
    if spec.pictured && !error {
        typed = typed.child(pictured_caret(format!("{key}-caret"), width + 1.0, 4.0, t));
    }
    well = well.child(typed);
    if let Some(trailing) = trailing {
        well = well.child(trailing.key(format!("{key}-act")));
    }
    let label = kit::label(
        format!("{key}-label"),
        spec.label,
        type_au::FIELD_LABEL,
        t.text_secondary,
    )
    .a11y_hidden()
    .mx(2.0);
    // The 3px ring follows real focus too: it is the wrap's, around the well's own 1px line.
    let ringed = if error || spec.pictured {
        well
    } else {
        el().w_full()
            .radius(6.0)
            .focus_within_ring(3.0, 0.0, t.accent.alpha(0.30))
            .child(well)
    };
    let mut wrap = col()
        .key(format!("{key}-wrap"))
        .w_full()
        .gap(6.0)
        .child(label)
        .child(ringed);
    if let Some(message) = spec.error {
        wrap = wrap.child(
            kit::label(format!("{key}-msg"), message, type_au::FIELD_MSG, au::red_text(t))
                .role("alert")
                .wrap()
                .mx(4.0),
        );
    }
    wrap
}

// ---------------------------------------------------------------------------
// Links `au::link` does not draw: an address, a link inside a sentence
// ---------------------------------------------------------------------------

/// The bars of an underline under `label` set in `style`, as the board's browser draws one
/// (`text-underline-offset: 2px`, skipping the ink that reaches below the baseline): `(x, width)`.
/// `au::link`'s own bars, and the at sign of an address, which dips below the line too.
pub fn underline_bars(label: &str, style: TextStyle) -> Vec<(f32, f32)> {
    let mut bars = Vec::new();
    let (mut from, mut x) = (0.0, 0.0);
    for (i, ch) in label.char_indices() {
        let end = rune::text::advance(&label[..i + ch.len_utf8()], style);
        let w = end - x;
        // The part of a glyph that reaches below the baseline, as a fraction of its advance.
        let gap = match ch {
            'g' | 'j' => Some((0.02, 0.98)),
            'p' => Some((0.02, 0.46)),
            'q' => Some((0.54, 0.98)),
            'y' => Some((0.10, 0.72)),
            '@' => Some((0.19, 0.72)),
            ',' | ';' => Some((0.10, 0.90)),
            _ => None,
        };
        if let Some((a, b)) = gap {
            if x + a * w > from {
                bars.push((from, x + a * w - from));
            }
            from = x + b * w;
        }
        x = end;
    }
    if x > from {
        bars.push((from, x - from));
    }
    bars
}

/// An underlined link in `style` (`au::link` is 12px on 15px only): `top` is how far the
/// underline sits below the line's top, `thick` how thick it is. Keys as `au::link`'s.
pub fn link_in(keys: &mut Keys, label: &str, style: TextStyle, top: f32, thick: f32, t: &Theme) -> El {
    let key = keys.next("link");
    let bars = underline_bars(label, style).into_iter().enumerate().map(|(i, (x, w))| {
        el().key(format!("{key}-underline-{i}"))
            .absolute(x, top)
            .w(w)
            .h(thick)
            .fill(t.text)
    });
    row()
        .key(format!("{key}-box"))
        .role("link")
        .label(label)
        .center_y()
        .focusable(true)
        .focus_ring(2.0, 2.0, t.accent_deep)
        .radius(4.0)
        .cursor(rune::Cursor::Pointer)
        .child(
            el().child(kit::label(key.clone(), label, style, t.text))
                .children(bars.collect::<Vec<_>>()),
        )
}
