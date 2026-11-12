//! The forms of the account sections (`.au-set`, DW-A58 to DW-A83): what a page with a field,
//! a code, a secret or a result is made of.
//!
//! | Piece | Function | Keys it gives |
//! | --- | --- | --- |
//! | Field with the board's focus and error looks | `field(index, &Field, edit, on_edit, t)` | `field-<N>`, `-label`, `-icon`, `-value`, `-caret`, `-msg` |
//! | Strength line under a field | `strength(index, bars, says, t)` | `field-<N>-strength-<J>`, `field-<N>-strength` |
//! | Code boxes over ONE input | `digits(first, count, edit, input_key, name, on_edit, t)` | `digit-<N>`, `digit-<N>-text` |
//! | Link, and a sentence with a link and a countdown | `link(index, label, style, t)`, `resend(index, link, wait, t)` | `link-<N>`, `resend-<N>-0/1/time` |
//! | Buttons side by side, sharing the column | `actions(buttons)` | the buttons' own |
//! | Steps of a flow | `steps(done, of, label, t)` | `steps`, `step-<J>`, `steps-count`, `steps-label` |
//! | Two-factor set-up: QR code, key, Copy key | `setup(modules, key, copy, t)` | `secure-setup`, `qr`, `qr-code`, `setup-label`, `setup-key` |
//! | Backup codes | `codes(&[..], t)` | `secure-codes`, `code-<N>`, `code-<N>-n` |
//! | Empty state | `empty(icon, title, words, t)` | `empty`, `empty-tile`, `empty-icon`, `empty-title`, `empty-sub` |
//! | Words-only pop-up menu | `pop(name, items, t)` | `pop-0`, `pop-0-item-<I>`, `-label` |
//! | A confirm's paragraph with a bold lead | `lead_paragraph(bold, rest, t)` | `dlg-text-0`, `dlg-bold` |
//!
//! **Focus is the model's.** Rune keeps which text field has the keyboard in the field's own
//! `TextEdit::focused` (`rune/src/edit.rs:45`), so a field draws the board's focus look (the
//! Amber ring, the deep border, the lighter well) from the model, and a page opens with a
//! field in focus by setting that flag. Nothing here asks Rune which element is focused.
//!
//! **Secrets.** A password is a secure `TextEdit`: Rune draws bullets and keeps the characters
//! out of the element and accessibility trees. The set-up key and the backup codes are secure
//! text nodes shown in the clear (`.secure().revealed(true)`), each group inside ONE element
//! keyed `secure-<name>` (`src/feedback/README.md`); their characters are never noted for the
//! gate, which compares their boxes and pixels only. A typed code is drawn the same way; the
//! gate is told its digits only during a gate export, which runs on the board's sample.

use crate::State;
use crate::theme::tokens::Theme;
use rune::tok::Family;
use rune::{Color, El, Icon, Shape, TextEdit, TextStyle, canvas, col, el, row, text, text_input};

use super::account_set::{self as set, SUB};
use super::au::{self, Tone, type_au};
use super::kit;

/// 11px/500 on a 14px line (`.t-cap`).
pub const CAP: TextStyle = TextStyle::new(11.0, 500.0).lh(14.0);
/// The same in mono (`.t-cap.au-mono`): a countdown, "1 of 4".
pub const CAP_MONO: TextStyle = TextStyle::new(11.0, 500.0).family(Family::Mono).lh(14.0);
/// 22px/600 mono: a digit in its box (`.au-digit`).
pub const DIGIT: TextStyle = TextStyle::new(22.0, 600.0).family(Family::Mono).lh(29.0);
/// 15px/400 mono on 17px, tracked 0.9px: a backup code (`.au-codes`).
pub const BACKUP_CODE: TextStyle = TextStyle::new(15.0, 400.0).family(Family::Mono).lh(17.0).tracking(0.9);
/// 10px/400 mono, tracked 0.9px: a backup code's number.
pub const CODE_N: TextStyle = TextStyle::new(10.0, 400.0).family(Family::Mono).lh(17.0).tracking(0.9);
/// 11.5px/400 on 17px: the strength line.
pub const STRENGTH: TextStyle = TextStyle::new(11.5, 400.0).lh(17.0);
/// 17px/600 on 22px, tracked -0.012em (`.t-title`).
pub const TITLE: TextStyle = TextStyle::new(17.0, 600.0).lh(22.0).tracking(-0.204);
/// 13px/400 on 17px: a pop-up menu's row.
pub const POP_ITEM: TextStyle = TextStyle::new(13.0, 400.0).lh(17.0);
/// 12px/700 on 15px: the bold lead of a confirm's paragraph.
pub const SUB_BOLD: TextStyle = TextStyle::new(12.0, 700.0).lh(15.0);
/// 12px/400 on 15.25px: a paragraph of a confirm whose lead is bold.
const SUB_RICH: TextStyle = TextStyle::new(12.0, 400.0).lh(15.0);

/// What a field shows around its text.
#[derive(Default)]
pub struct Field<'a> {
    pub label: &'a str,
    pub icon: Option<&'static Icon>,
    /// The words under the field.
    pub message: Option<&'a str>,
    /// The red border and ring; the message is the error and is announced.
    pub error: bool,
    /// The text input's own key (tests, `Tx::focus`).
    pub input_key: &'static str,
}

/// A labelled one-line field (`.kit-fieldwrap` > `.kit-field-md`), with the board's three
/// looks: at rest, in focus (from `edit.focused`), in error.
pub fn field(
    index: usize,
    spec: &Field<'_>,
    edit: &TextEdit,
    on_edit: impl for<'a> Fn(&'a mut State) -> &'a mut TextEdit + 'static,
    t: &Theme,
) -> El {
    let key = format!("field-{index}");
    let hidden = edit.is_secure() && !edit.is_revealed();
    let style = if hidden {
        type_au::FIELD_SECURE
    } else {
        type_au::FIELD_VALUE
    };
    let shown = au::gate_text(edit);
    if let Some(shown) = shown.as_deref().filter(|shown| !shown.is_empty()) {
        kit::note_text(
            &format!("{key}-value"),
            &format!("{key}-value-box"),
            shown,
            style,
            t.text,
        );
    }
    let focused = edit.focused && !spec.error;
    let mut input = text_input(edit, style)
        .key(spec.input_key)
        .label(spec.label)
        .single_line()
        .color(t.text)
        .caret(Color::TRANSPARENT, t.selection)
        .invalid(spec.error)
        .on_edit(on_edit)
        // Esc in a field is the page's Esc (cancel, go back): the field does not keep it.
        .on_input_key(|s: &mut State, ev: &rune::KeyEvent, tx: &mut rune::Tx| {
            ev.key == rune::Key::Escape && crate::frames::account::on_key(s, ev, tx)
        })
        .grow(1.0)
        .min_w(0.0);
    if let Some(message) = spec.message {
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
    well = if spec.error {
        well.fill(t.field)
            .border(1.0, t.red)
            .box_shadow(kit::ring(3.5, t.red.alpha(0.16)))
    } else if focused {
        well.fill(t.card)
            .border(1.0, t.accent_deep)
            .box_shadow(kit::ring(3.0, t.accent.alpha(0.30)))
    } else {
        well.fill(t.field)
            .border(1.0, Color::TRANSPARENT)
            .box_shadow(kit::ring_inset(t.separator))
    };
    if let Some(icon) = spec.icon {
        well = well.child(kit::glyph(format!("{key}-icon"), icon, 16.0, t.text_tertiary));
    }
    // The typed text's own box, for the gate: as wide as the text Rune draws in the input.
    let width = shown.as_deref().map_or(0.0, |shown| rune::text::advance(shown, style));
    let mut inner = row().grow(1.0).min_w(0.0).h_full().center_y().child(input).child(
        el().key(format!("{key}-value-box"))
            .a11y_hidden()
            .absolute(0.0, 0.0)
            .w(width)
            .h_full(),
    );
    if focused {
        // The caret as the board draws it (`.kit-caret`: 1.5 x 17, 1 pt after the character it
        // follows), where the model says it stands. Rune's own caret is 2 pt wide and touches
        // the text, so it is left unseen (see the PR's Rune notes); this one does not blink.
        let before = edit.text[..edit.focus.min(edit.text.len())].chars().count();
        let x = match shown.as_deref() {
            Some(shown) => {
                let end = shown.char_indices().nth(before).map_or(shown.len(), |(at, _)| at);
                rune::text::advance(&shown[..end], style)
            }
            None => 0.0,
        };
        if shown.is_some() {
            inner = inner.child(
                el().key(format!("{key}-caret"))
                    .a11y_hidden()
                    .absolute(x + 1.0, 4.5)
                    .size(1.5, 17.0)
                    .radius(1.0)
                    .fill(t.accent_deep),
            );
        }
    }
    well = well.child(inner);
    let mut wrap = col().key(format!("{key}-wrap")).w_full().gap(6.0).child(
        kit::label(
            format!("{key}-label"),
            spec.label,
            type_au::FIELD_LABEL,
            t.text_secondary,
        )
        .a11y_hidden()
        .mx(2.0),
    );
    wrap = wrap.child(well);
    if let Some(message) = spec.message {
        wrap = wrap.child(
            kit::label(
                format!("{key}-msg"),
                message,
                type_au::FIELD_MSG,
                if spec.error { au::red_text(t) } else { t.text_tertiary },
            )
            .role(if spec.error { "alert" } else { "note" })
            .wrap()
            .mx(4.0),
        );
    }
    wrap
}

/// The strength line under the new password (`.kit-strength`): four bars, `bars` of them
/// green, and what core says. Read after the field (it is its own status).
pub fn strength(index: usize, bars: usize, says: &str, t: &Theme) -> El {
    let key = format!("field-{index}-strength");
    let mut r = row()
        .key(format!("{key}-row"))
        .role("status")
        .label(says)
        .h(17.0)
        .mx(2.0)
        .gap(4.0)
        .center_y();
    for j in 0..4 {
        r = r.child(
            el().key(format!("{key}-{j}"))
                .a11y_hidden()
                .grow(1.0)
                .h(4.0)
                .radius(2.0)
                .fill(if j < bars { t.green } else { t.separator }),
        );
    }
    r.child(kit::label(key, says, STRENGTH, t.text_tertiary).a11y_hidden().ml(6.0))
}

/// The boxes of a code (`.au-digits`): `count` of them, numbered from `first`, drawn from ONE
/// text input that lies over the row (`input_key`, named `name`: "6-digit code"). The box the
/// next digit goes in shows the focus look while the input has the keyboard.
pub fn digits(
    first: usize,
    count: usize,
    edit: &TextEdit,
    input_key: &'static str,
    name: &str,
    on_edit: impl for<'a> Fn(&'a mut State) -> &'a mut TextEdit + 'static,
    t: &Theme,
) -> El {
    let typed: Vec<char> = edit.text.chars().filter(char::is_ascii_digit).take(count).collect();
    let mut boxes = row().key("digits").pixel_snap(false).gap(8.0);
    for i in 0..count {
        let key = format!("digit-{}", first + i);
        let active = edit.focused && i == typed.len().min(count - 1) && typed.len() < count;
        let mut b = el()
            .key(key.clone())
            .a11y_hidden()
            .size(44.0, 52.0)
            .radius(12.0)
            .center();
        b = if active {
            b.fill(t.card)
                .box_shadow(kit::ring(3.0, t.accent.alpha(0.40)))
                .border(1.0, t.accent_deep)
        } else {
            b.fill(t.field).box_shadow(kit::ring_inset(t.separator))
        };
        if let Some(ch) = typed.get(i) {
            let digit = ch.to_string();
            kit::note_text(
                &format!("{key}-text"),
                &format!("{key}-text-box"),
                &digit,
                DIGIT,
                t.text,
            );
            b = b.child(
                el().key(format!("{key}-text-box"))
                    .child(text(digit).secure().revealed(true).font(DIGIT).color(t.text)),
            );
        }
        boxes = boxes.child(b);
    }
    // The one input: over the whole row, its own text and caret unseen (the boxes show them).
    let input = text_input(edit, DIGIT)
        .key(input_key)
        .label(name)
        .single_line()
        .color(Color::TRANSPARENT)
        .caret(Color::TRANSPARENT, Color::TRANSPARENT)
        .on_edit(on_edit)
        // Esc in a field is the page's Esc (cancel, go back): the field does not keep it.
        .on_input_key(|s: &mut State, ev: &rune::KeyEvent, tx: &mut rune::Tx| {
            ev.key == rune::Key::Escape && crate::frames::account::on_key(s, ev, tx)
        })
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0));
    // Not snapped to whole pixels: a confirm is centred on quarter points, as the board's is.
    el().key("secure-code")
        .pixel_snap(false)
        .w_full()
        .center()
        .child(el().pixel_snap(false).child(boxes).child(input))
}

/// The underline of a link, as the board's browser draws it (skipping the ink of descenders):
/// bars `(x, width)` under `label` set in `style`. (`au::link` has the same for 12px; its
/// helper is private: see the PR's requests to the design-system lane.)
fn underline_bars(label: &str, style: TextStyle) -> Vec<(f32, f32)> {
    let mut bars = Vec::new();
    let (mut from, mut x) = (0.0, 0.0);
    for (i, ch) in label.char_indices() {
        let end = rune::text::advance(&label[..i + ch.len_utf8()], style);
        let w = end - x;
        let gap = match ch {
            'g' | 'j' => Some((0.02, 0.98)),
            'p' => Some((0.02, 0.46)),
            'q' => Some((0.54, 0.98)),
            'y' => Some((0.10, 0.72)),
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

/// An underlined link (`.au-link`) in `style` (11px in a caption, 12px in a paragraph).
/// `dimmed`: not pressable now (a countdown runs). Key: `link-<N>`.
pub fn link(index: usize, label: &str, style: TextStyle, dimmed: bool, t: &Theme) -> El {
    link_in(index, label, style, dimmed, false, t)
}

/// [`link`] as a block: its box is the column's width (DW-A69's Copy key).
pub fn link_block(index: usize, label: &str, style: TextStyle, t: &Theme) -> El {
    link_in(index, label, style, false, true, t).w_full()
}

fn link_in(index: usize, label: &str, style: TextStyle, dimmed: bool, block: bool, t: &Theme) -> El {
    let key = format!("link-{index}");
    // The underline sits 2px under the baseline: 12.9px down a 14px line at 11px, 13.9 at 12px.
    let y = style.line_height - 1.1;
    let bars = underline_bars(label, style)
        .into_iter()
        .enumerate()
        .map(|(i, (x, w))| {
            el().key(format!("{key}-underline-{i}"))
                .absolute(x, y)
                .w(w)
                .h(0.875)
                .fill(t.text)
        })
        .collect::<Vec<_>>();
    let mut l = el()
        .key(format!("{key}-box"))
        .role("link")
        .label(label)
        .radius(4.0)
        .child({
            let words = kit::label(key, label, style, t.text);
            if block { words.w_full() } else { words }
        })
        .children(bars);
    if dimmed {
        l = l.disabled(true);
    } else {
        l = l
            .focusable(true)
            .focus_ring(2.0, 2.0, t.accent_deep)
            .cursor(rune::Cursor::Pointer);
    }
    l
}

/// "Didn't get it? Send again in 42 s": the link is pressable once the countdown is over (and
/// the sentence then ends at the link). `link_el` is [`link`]'s element with its press.
pub fn resend(index: usize, link_el: El, wait: u32, t: &Theme) -> El {
    let key = format!("resend-{index}");
    let mut r = row()
        .key(format!("{key}-row"))
        .w_full()
        // The mono countdown makes the board's line half a point taller than its words.
        .h(14.5)
        .items_start()
        .child(kit::label(format!("{key}-0"), "Didn't get it?", CAP, t.text_tertiary).mr(2.93))
        .child(link_el);
    if wait > 0 {
        r = r
            .child(kit::label(format!("{key}-1"), "in", CAP, t.text_tertiary).mx(2.93))
            .child(kit::label(
                format!("{key}-time"),
                format!("{wait} s"),
                CAP_MONO,
                t.text_tertiary,
            ));
    }
    r
}

/// The buttons under a form (`.au-actions.is-row`): side by side, sharing the column. The
/// board shares the free width evenly AFTER each button's own padding and border, so a
/// bordered button comes out 2 pt wider than a borderless one (as in a dialog's foot).
pub fn actions(buttons: Vec<(El, bool)>) -> El {
    let count = buttons.len().max(1) as f32;
    let fixed = |bordered: bool| 24.0 + if bordered { 2.0 } else { 0.0 };
    let free = set::COLUMN_W - 8.0 * (count - 1.0) - buttons.iter().map(|(_, b)| fixed(*b)).sum::<f32>();
    let mut r = row().key("actions").w_full().gap(8.0).center_y();
    for (button, bordered) in buttons {
        r = r.child(button.w(free / count + fixed(bordered)));
    }
    r
}

/// The steps of a flow (`.au-steps`): `of` bars, `done` of them in ink, "2 of 4" in mono and
/// the step's name. One status for assistive technology.
pub fn steps(done: usize, of: usize, label: &str, t: &Theme) -> El {
    let count = format!("{done} of {of}");
    // The bars share what the two labels leave, to the hundredth of a point (Rune rounds a
    // text's own box to half points, the board does not).
    let free = set::COLUMN_W - 20.0 - rune::text::advance(&count, CAP_MONO) - rune::text::advance(label, CAP);
    let bar = (free - 4.0 * (of.max(1) as f32 - 1.0)) / of.max(1) as f32;
    let mut bars = row().key("steps-bars").pixel_snap(false).w(free).gap(4.0).center_y();
    for j in 0..of {
        bars = bars.child(
            el().key(format!("step-{j}"))
                .w(bar)
                .h(3.0)
                .radius(2.0)
                .fill(if j < done { t.text } else { t.separator }),
        );
    }
    let count_el = exact("steps-count", &count, CAP_MONO, t);
    row()
        .key("steps")
        .pixel_snap(false)
        .role("status")
        .label(format!("Step {count}: {label}"))
        .w_full()
        .h(14.0)
        .gap(10.0)
        .center_y()
        .child(bars)
        .child(count_el)
        .child(exact("steps-label", label, CAP, t))
}

/// A caption in a box exactly as wide as its words: Rune rounds a text's own box up to half
/// a point, which would move words that end at a column's edge.
fn exact(key: &str, words: &str, style: TextStyle, t: &Theme) -> El {
    kit::note_text(key, &format!("{key}-box"), words, style, t.text_tertiary);
    el().key(format!("{key}-box"))
        .a11y_hidden()
        .pixel_snap(false)
        .w(rune::text::advance(words, style))
        .h(14.0)
        .child(text(words).font(style).color(t.text_tertiary).pixel_snap(false))
}

/// A QR code as one canvas `size` points across: each run of dark modules in a row is one
/// rectangle. Black on a white tile in both themes, so a phone's camera reads it.
fn qr_code(modules: &[Vec<bool>], size: f32) -> El {
    let n = modules.len().max(1) as f32;
    let mut shape = Shape::new().view_box(n, n);
    for (y, line) in modules.iter().enumerate() {
        let mut x = 0;
        while x < line.len() {
            if !line[x] {
                x += 1;
                continue;
            }
            let start = x;
            while x < line.len() && line[x] {
                x += 1;
            }
            let (x0, x1, y0, y1) = (start as f32, x as f32, y as f32, y as f32 + 1.0);
            shape = shape.fill(vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]], Color::BLACK);
        }
    }
    // Its text alternative is the key beside it, which stays out of every label.
    canvas(shape)
        .key("qr-code")
        .role("image")
        .label("QR code for your authenticator app. The same key is beside it.")
        .size(size, size)
}

/// Two-factor set-up (DW-A69): the QR code, the same key in mono and Copy key, as ONE secure
/// group. `key` is drawn by a secure text node, in the clear; `copy` is the link's element.
pub fn setup(modules: &[Vec<bool>], key: &str, copy: El, t: &Theme) -> El {
    row()
        .key("secure-setup")
        .w_full()
        .gap(16.0)
        .center_y()
        .child(
            el().key("qr")
                .size(144.0, 144.0)
                .padding(12.0)
                .radius(18.0)
                .fill(Color::WHITE)
                .box_shadow(kit::ring(kit::HAIR, t.separator))
                .child(qr_code(modules, 120.0)),
        )
        .child(
            col()
                .key("setup-words")
                .grow(1.0)
                .min_w(0.0)
                .gap(8.0)
                .child(kit::label("setup-label", "Or enter this key", CAP, t.text_tertiary).w_full())
                .child(
                    el().key("setup-key").w_full().h(15.0).child(
                        text(key)
                            .secure()
                            .revealed(true)
                            .font(set::SUB_MONO)
                            .color(t.text_tertiary),
                    ),
                )
                .child(copy),
        )
}

/// The backup codes (DW-A70, `.au-codes`): two columns, numbered, as ONE secure group. Each
/// code is a secure text node shown in the clear, read digit by digit.
pub fn codes(list: &[&str], t: &Theme) -> El {
    let cell = |i: usize, code: &str| {
        row()
            .key(format!("code-{i}"))
            .w(235.0)
            .h(19.0)
            .items_start()
            .child(
                kit::label(format!("code-{i}-n"), (i + 1).to_string(), CODE_N, t.text_tertiary)
                    .a11y_hidden()
                    .align(rune::Align::End)
                    .w(12.0)
                    .mt(2.0)
                    .mr(8.0),
            )
            .child(text(code).secure().revealed(true).font(BACKUP_CODE).color(t.text))
    };
    let mut grid = col()
        .key("secure-codes")
        .role("group")
        .label("Backup codes")
        .w_full()
        .py(14.0)
        .px(16.0)
        .gap(8.0)
        .radius(14.0)
        .fill(t.card)
        .box_shadow(kit::ring(kit::HAIR, t.separator));
    for (line, pair) in list.chunks(2).enumerate() {
        let mut r = row().gap(18.0);
        for (j, code) in pair.iter().enumerate() {
            r = r.child(cell(line * 2 + j, code));
        }
        grid = grid.child(r);
    }
    grid
}

/// An empty state (`.kit-empty`): a neutral tile, a title and one paragraph, centred.
pub fn empty(icon: &'static Icon, title: &str, words: &str, t: &Theme) -> El {
    col()
        .key("empty")
        .w_full()
        .py(28.0)
        .px(24.0)
        .gap(8.0)
        .items_center()
        .child(
            el().key("empty-tile")
                .a11y_hidden()
                .size(52.0, 52.0)
                .radius(15.0)
                .fill(au::tone(Tone::Neutral, t).2)
                .center()
                .child(kit::glyph("empty-icon", icon, 26.0, au::tone(Tone::Neutral, t).0)),
        )
        .child(kit::label("empty-title", title, TITLE, t.text).role("heading").level(2))
        .child(
            kit::label("empty-sub", words, SUB, t.text_tertiary)
                .wrap()
                .align(rune::Align::Center)
                // `max-width: 30ch`: thirty zeros of its own font wide.
                .w(30.0 * rune::text::advance("0", SUB)),
        )
}

/// One row of a words-only pop-up menu; `danger` in red. `pad` is the board's own padding
/// around the words (its Remove row is padded twice: 54 pt tall).
pub fn pop_item(index: usize, label: &str, danger: bool, t: &Theme) -> El {
    let key = format!("pop-0-item-{index}");
    let ink = if danger { au::red_text(t) } else { t.text };
    let words = kit::label(format!("{key}-label-text"), label, POP_ITEM, ink);
    let (row_key, label_key) = (key.clone(), format!("{key}-label"));
    kit::note_text(
        &label_key,
        if danger { &label_key } else { &row_key },
        label,
        POP_ITEM,
        ink,
    );
    let inner = el()
        .key(if danger { label_key } else { format!("{key}-pad") })
        .w_full()
        .py(9.0)
        .px(14.0)
        .child(words);
    let mut item = el()
        .key(row_key)
        .role("menuitem")
        .label(if danger {
            format!("{label}, destructive")
        } else {
            label.to_string()
        })
        .w_full()
        .hover(t.hover)
        .press(t.press)
        .focusable(true);
    if danger {
        // The board rules its Remove row off and pads it twice (its row is 53.5 pt tall).
        item = item
            .py(9.0)
            .px(14.0)
            .border_sides([kit::HAIR, 0.0, 0.0, 0.0], t.separator)
            .border_layout();
    }
    item.child(inner)
}

/// A words-only pop-up menu (`.au-menu.glass`): 180 pt wide, the paper seen through. The
/// caller places it and makes it modal (Esc and a press outside close it). Key: `pop-0`.
pub fn pop(name: &str, items: Vec<El>, t: &Theme) -> El {
    let mut m = col()
        .key("pop-0")
        .role("menu")
        .label(name)
        .w(180.0)
        .py(4.0)
        .radius(12.0)
        // `--kit-glass`: the paper, seen through.
        .fill(if t.dark {
            t.paper2.alpha(0.68)
        } else {
            t.paper.alpha(0.72)
        })
        .backdrop_blur(22.0)
        .border(kit::HAIR, t.separator)
        .border_layout()
        .box_shadow(kit::drop(10.0, 30.0, Color::BLACK.alpha(0.22)));
    for item in items {
        m = m.child(item);
    }
    m
}

/// A confirm's paragraph whose first sentence is bold (DW-A76, DW-A78). Keys: `dlg-text-0`
/// (the whole paragraph), `dlg-bold` (the lead, on the first line).
pub fn lead_paragraph(bold: &str, rest: &str, t: &Theme) -> El {
    let all = format!("{bold} {rest}");
    kit::note_text("dlg-text-0", "dlg-text-0-box", &all, SUB_RICH, t.text_secondary);
    kit::note_text("dlg-bold", "dlg-bold-box", bold, SUB_BOLD, t.text_secondary);
    let spans = vec![rune::Span::new(0, bold.len()).weight(700.0)];
    el().key("dlg-text-0-box")
        .w_full()
        .child(
            rune::rich(all, spans)
                .font(SUB_RICH)
                .color(t.text_secondary)
                .wrap()
                .w_full(),
        )
        .child(
            el().key("dlg-bold-box")
                .a11y_hidden()
                .absolute(0.0, 0.0)
                .w(rune::text::advance(bold, SUB_BOLD))
                .h(15.0),
        )
}
