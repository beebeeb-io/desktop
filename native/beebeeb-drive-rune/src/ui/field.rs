//! A one-line field without a label above it (`.kit-fieldwrap` > `.kit-field`), as a dialog holds
//! it (DW-0417: New folder), and the small message that hangs under a field in a row (`.mk-tip`).
//!
//! | Piece | Function | Keys it gives |
//! | --- | --- | --- |
//! | Field | `bare_field(spec, edit, input_key, on_edit, t)` | `<key>-wrap`, `<key>`, `<key>-value`, `<key>-caret`, `<key>-msg` |
//! | Tip | `tip(key, icon, words, t)` | `<key>`, `<key>-icon`, `<key>-text` |
//!
//! `ui::au::field` is the labelled one of the account windows; this one has the board's focused
//! look as the dialog draws it: a 1 pt Amber-deep border and a 3 pt Amber ring, the selection in
//! Amber at 30 %. The caller adds `.on_submit(..)` through `input`.

use crate::State;
use crate::theme::tokens::Theme;
use rune::{Color, El, Icon, TextEdit, TextStyle, col, el, row, text_input};

use super::{au, kit};

/// `.kit-field-value`: 13px/400 on a 17px line.
pub const VALUE: TextStyle = TextStyle::new(13.0, 400.0).lh(17.0);
/// `.kit-field-msg`: 12px/400 on a 16px line.
pub const MSG: TextStyle = TextStyle::new(12.0, 400.0).lh(16.0);
/// `.mk-tip`: 11.5px/400 on a 17px line.
pub const TIP: TextStyle = TextStyle::new(11.5, 400.0).lh(17.0);

/// How wide `text` draws; nothing for no text (`rune::text::advance` gives an empty string a space's width).
pub fn advance(text: &str, style: TextStyle) -> f32 {
    if text.is_empty() {
        0.0
    } else {
        rune::text::advance(text, style)
    }
}

/// To the device pixel of a 200 % display, as the browser places a box: a 1.5 pt caret at a
/// fraction of a point would be smeared over four pixels.
pub fn snap(v: f32) -> f32 {
    (v * 2.0).round() / 2.0
}

/// The selection inside a field: Amber at 30 % (the board's `color-mix(.. amber 30%, transparent)`).
pub fn selection(t: &Theme) -> Color {
    t.accent.alpha(0.30)
}

/// A ring `width` wide around `inner` (CSS `box-shadow: 0 0 0 <width>`), drawn as a mat under it
/// that takes no room. A `box_shadow` ring stops at the box's edge, so where that edge covers
/// half a pixel (a dialog centred on a quarter point) the border is mixed with what is behind
/// the ring; the browser mixes it with the ring. The mat reaches under the border, as there.
pub fn ringed(inner: El, key: &str, width: f32, radius: f32, color: Color) -> El {
    el().key(key.to_string())
        .mx(-width)
        .my(-width)
        .px(width)
        .py(width)
        .radius(radius + width)
        .fill(color)
        .child(inner)
}

/// What a bare field shows beside its text.
pub struct Bare<'a> {
    /// The gate key of the field's box.
    pub key: &'a str,
    /// What assistive technology calls the field (it has no drawn label).
    pub name: &'a str,
    /// The line under the field: a note, or the words of an error.
    pub message: Option<&'a str>,
    pub error: bool,
}

/// The caret of a field that holds a selection (`.kit-caret`): Windows draws it at the end the
/// selection was extended to. `h`: 17 in a dialog's field, 14 in a row's.
pub fn caret_mark(key: String, h: f32, t: &Theme) -> El {
    el().key(key).a11y_hidden().w(1.5).h(h).radius(1.0).fill(t.accent_deep)
}

/// The field. `input`: what the caller adds to the text input itself (`on_submit`, `on_input_key`).
pub fn bare_field(
    spec: Bare<'_>,
    edit: &TextEdit,
    input_key: &'static str,
    on_edit: impl for<'a> Fn(&'a mut State) -> &'a mut TextEdit + 'static,
    input: impl FnOnce(El) -> El,
    t: &Theme,
) -> El {
    let key = spec.key;
    let (from, to) = edit.range();
    // A secure field has no "selection" for the gate: none of its text is read or measured here.
    let selected = edit.focused && from != to && !edit.is_secure();
    // The gate reads the text of the selection, as the board marks it up; else the whole text.
    // A secure field's characters never leave the field: the gate is told the bullets' count
    // (`au::gate_text`), and nothing at all while the field is revealed.
    let shown: std::borrow::Cow<'_, str> = if edit.is_secure() {
        super::au::gate_text(edit).unwrap_or_default()
    } else if selected {
        std::borrow::Cow::Borrowed(&edit.text[from..to])
    } else {
        std::borrow::Cow::Borrowed(edit.text.as_str())
    };
    if !shown.is_empty() {
        kit::note_text(
            &format!("{key}-value"),
            &format!("{key}-value-box"),
            &shown,
            VALUE,
            t.text,
        );
    }
    let mut field = text_input(edit, VALUE)
        .key(input_key)
        .label(spec.name)
        .single_line()
        .color(t.text)
        .caret(t.accent_deep, selection(t))
        .selection_style(rune::SelectionStyle::new(selection(t)))
        .invalid(spec.error)
        .on_edit(on_edit)
        .grow(1.0)
        .min_w(0.0);
    if let Some(message) = spec.message {
        field = field.description(message);
    }
    let field = input(field);

    // The ring of a focused field, or of an error, is a mat UNDER the field (`ringed`); at rest
    // the field has its hairline only.
    let (border, ring) = match (spec.error, edit.focused) {
        (true, _) => (t.red, Some(t.red.alpha(0.16))),
        (false, true) => (t.accent_deep, Some(t.accent.alpha(0.30))),
        (false, false) => (Color::TRANSPARENT, None),
    };
    let before = if selected {
        advance(&edit.text[..from], VALUE)
    } else {
        0.0
    };
    let width = advance(&shown, VALUE);
    let mut text_row = row()
        .key(format!("{key}-main"))
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .center_y()
        .child(field)
        // The shown text's own box, for the gate: where Rune draws it inside the input.
        .child(
            el().key(format!("{key}-value-box"))
                .a11y_hidden()
                .absolute(if selected { before } else { 0.0 }, 0.0)
                .w(width)
                .h_full(),
        );
    if selected {
        text_row = text_row.child(
            // `.kit-caret` stands at the top of the 18 pt text line, which is centred in the 26 pt well.
            el().key(format!("{key}-caret-at"))
                .a11y_hidden()
                .absolute(snap(before + width + 1.0), 4.0)
                .child(caret_mark(format!("{key}-caret"), 17.0, t)),
        );
    }
    let well = row()
        .key(key.to_string())
        .w_full()
        .h(28.0)
        .px(8.0)
        .gap(6.0)
        .center_y()
        .radius(6.0)
        .fill(t.card)
        .border(1.0, border)
        .border_layout()
        .child(text_row);
    let well = match ring {
        Some(color) => ringed(well, &format!("{key}-ring"), 3.0, 6.0, color),
        None => well.box_shadow(kit::ring_inset(t.separator)),
    };
    let mut wrap = col().key(format!("{key}-wrap")).w_full().gap(6.0).child(well);
    if let Some(message) = spec.message {
        wrap = wrap.child(
            el().key(format!("{key}-msg-box")).w_full().px(4.0).child(
                kit::label(
                    format!("{key}-msg"),
                    message,
                    MSG,
                    if spec.error { au::red_text(t) } else { t.text_tertiary },
                )
                .role(if spec.error { "alert" } else { "note" })
                .wrap()
                .w_full(),
            ),
        );
    }
    wrap
}

/// A small raised message (`.mk-tip`): the reason a name in a row cannot be given (DW-0410b).
/// Red words: it is the message of an error. The caller places it.
pub fn tip(key: &str, icon: &'static Icon, words: &str, t: &Theme) -> El {
    let ink = au::red_text(t);
    row()
        .key(key.to_string())
        .role("alert")
        .label(words)
        .h(25.0)
        .px(8.0)
        .gap(6.0)
        .center_y()
        .radius(6.0)
        .fill(t.card)
        .box_shadow(kit::drop(4.0, 14.0, Color::BLACK.alpha(0.16)))
        .box_shadow(kit::ring(kit::HAIR, t.separator))
        .child(kit::glyph(format!("{key}-icon"), icon, 13.0, ink))
        .child(kit::label(format!("{key}-text"), words, TIP, ink))
}
