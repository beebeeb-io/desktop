//! What sign-up draws that the account kit (`ui::au`, `ui::au_entry`) does not have yet: the
//! progress line (`.au-steps`), a checkbox with its sentence, a suggestion chip (`.kit-chip`),
//! the strength meter with sign-up's states, a small outline button (`.kit-btn-sm`), the
//! "Next" card of DW-A44, the 64-point ring of DW-A43, and a field that can be focused and in
//! error at once (DW-A33, DW-A38). Measurements are the board's computed CSS (the goldens).
//!
//! Secrets: as `ui::au_entry`. A field over a secure, revealed `TextEdit` is measured by its
//! character count (mono), never with `rune::text::advance`; a suggestion is a secure text node.

use rune::tok::Family;
use rune::{
    Color, El, Icon, KeyEvent, LineCap, Shape, TextEdit, TextStyle, Tx, canvas, col, el, row, text, text_input,
};

use super::Ui;
use crate::State;
use crate::theme::tokens::{Theme, type_};
use crate::ui::au::{self, Keys, Tone, type_au};
use crate::ui::au_entry::{self, mono_width};
use crate::ui::kit;

/// The checkbox's tick: the board's path at its own 2.5 stroke (the kit's check is 1.6).
pub static TICK: Icon = Icon {
    name: "tick",
    view: 24.0,
    body: r#"<g fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="M20 6L9 17l-5-5"></path></g>"#,
};
pub static COPY: Icon = Icon {
    name: "copy",
    view: 24.0,
    body: r#"<g fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><path d="M20 9h-9a2 2 0 0 0-2 2v9a2 2 0 0 0 2 2h9a2 2 0 0 0 2-2v-9a2 2 0 0 0-2-2zM5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"></path></g>"#,
};
pub static DOWNLOAD: Icon = Icon {
    name: "download",
    view: 24.0,
    body: r#"<g fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4M7 10l5 5 5-5M12 15V3"></path></g>"#,
};

/// `.kit-btn-sm`'s label: 12px/500 on its own line.
const BUTTON_SM: TextStyle = TextStyle::new(12.0, 500.0).lh(17.1429).tracking(-0.06);
/// A checkbox's sentence: 12px on a relaxed line.
const CHECK_LABEL: TextStyle = TextStyle::new(12.0, 400.0).lh(19.5);
/// `.kit-chip`: 12px/600.
const CHIP: TextStyle = TextStyle::new(12.0, 600.0).lh(14.5);
const CAPTION_MONO: TextStyle = TextStyle::new(11.0, 500.0).family(Family::Mono).lh(14.0);

/// A control's handler on a sign-up screen: `super::act`, and the field a scene pictured as
/// focused is a real one again.
pub fn cact(f: fn(&mut Ui)) -> impl Fn(&mut State, &mut Tx) + 'static {
    move |s: &mut State, tx: &mut Tx| {
        if let Some(ui) = s.signin.as_mut() {
            ui.create.pictured = None;
            f(ui);
            ui.pump();
            ui.settle(tx);
        }
    }
}

/// A field's change handler (as `view_steps::then`).
pub fn changed(f: fn(&mut Ui)) -> impl Fn(&mut State, &mut Tx) + 'static {
    cact(f)
}

/// The progress line: six bars, `step` of them dark, "2 of 6" and the step's name.
pub fn steps_line(step: usize, name: &str, t: &Theme) -> El {
    let mut bars = row().grow(1.0).gap(4.0).center_y();
    for i in 0..6 {
        bars = bars.child(
            el().key(format!("step-{i}"))
                .grow(1.0)
                .h(3.0)
                .radius(2.0)
                .fill(if i < step { t.text } else { t.separator }),
        );
    }
    row()
        .key("au-steps")
        .role("status")
        .label(format!("Step {step} of 6: {name}"))
        .w_full()
        .h(14.0)
        .gap(10.0)
        .center_y()
        .child(bars)
        .child(kit::label("steps-count", format!("{step} of 6"), CAPTION_MONO, t.text_tertiary).a11y_hidden())
        .child(kit::label("steps-name", name, type_::CAPTION, t.text_tertiary).a11y_hidden())
}

/// A real checkbox with its full sentence as its name. Keys `check-<i>`, `-icon`, `-label`.
pub fn checkbox(k: &mut Keys, on: bool, sentence: &str, t: &Theme) -> El {
    let key = k.next("check");
    let mut tick = el()
        .key(key.clone())
        .size(16.0, 16.0)
        .mt(2.0)
        .radius(4.0)
        .center()
        .border(1.0, if on { t.accent_deep } else { t.card_border })
        .fill(if on { t.accent } else { t.card });
    if on {
        tick = tick.child(kit::glyph(format!("{key}-icon"), &TICK, 12.0, Color::WHITE));
    }
    row()
        .key(format!("{key}-row"))
        .role("checkbox")
        .label(sentence)
        .checked(on)
        .w_full()
        .gap(8.0)
        .items_start()
        .focusable(true)
        .focus_ring(2.0, 2.0, t.accent_deep)
        .radius(4.0)
        .cursor(rune::Cursor::Pointer)
        .child(tick)
        .child(
            kit::label(format!("{key}-label"), sentence, CHECK_LABEL, t.text_secondary)
                .a11y_hidden()
                .wrap()
                .grow(1.0)
                .min_w(0.0),
        )
}

/// A suggestion (`.kit-chip`): the word as a secure text node, because which words are
/// offered follows what was typed. Its name says its place, not the word.
pub fn chip(i: usize, word: &str, t: &Theme) -> El {
    let (_, _, wash) = au::tone(Tone::Neutral, t);
    row()
        .key(format!("chip-{i}"))
        .role("button")
        .label(format!("Use suggestion {}", i + 1))
        .h(26.0)
        .px(10.0)
        .center()
        .radius(13.0)
        .fill(wash)
        .focusable(true)
        .focus_ring(2.0, 2.0, t.accent_deep)
        .cursor(rune::Cursor::Pointer)
        .child(au_entry::shown_secret(
            &format!("chip-{i}-text"),
            word,
            CHIP,
            t.text_secondary,
        ))
}

/// The strength meter (`.kit-strength`) under field `key`: four bars, `lit` of them coloured
/// (one is red, two grey, three and four green), and the engine's words. Colour is never the
/// only signal: the words are the meter's name.
pub fn meter(key: &str, lit: u8, words: &str, t: &Theme) -> El {
    let lit = usize::from(lit.min(4));
    let ink = match lit {
        1 => t.red,
        2 => t.text_tertiary,
        _ => t.green,
    };
    let mut bars = row().grow(1.0).gap(4.0).center_y();
    for i in 0..4 {
        bars = bars.child(
            el().key(format!("{key}-strength-{i}"))
                .grow(1.0)
                .h(4.0)
                .radius(2.0)
                .fill(if i < lit { ink } else { t.separator }),
        );
    }
    row()
        .key(format!("{key}-meter"))
        .role("status")
        .label(format!("Password strength: {words}"))
        .h(17.0)
        .mx(2.0)
        .gap(10.0)
        .center_y()
        .child(bars)
        .child(kit::label(format!("{key}-strength"), words, type_::STATUS, t.text_tertiary).a11y_hidden())
}

/// A small outline button that is as wide as its words (`.kit-btn-sm`): Copy, Save as file, Print.
pub fn small_button(k: &mut Keys, label: &str, icon: &'static Icon, t: &Theme) -> El {
    let key = k.next("btn");
    row()
        .key(key.clone())
        .role("button")
        .label(label)
        .h(22.0)
        .px(12.0)
        .gap(7.0)
        .center()
        .radius(6.0)
        .fill(t.paper)
        .border(1.0, t.card_border)
        .border_layout()
        .hover(t.paper2)
        .press(t.paper3)
        .focusable(true)
        .focus_ring(2.0, 2.0, t.accent_deep)
        .cursor(rune::Cursor::Pointer)
        .child(kit::glyph(format!("{key}-icon"), icon, 14.0, t.text))
        .child(kit::label(format!("{key}-label"), label, BUTTON_SM, t.text))
}

/// The "Next" card of DW-A44: the product's own step, named. Keys `next-0`, `-tile`, `-icon`,
/// `-cap`, `-title`.
pub fn next_card(icon: &'static Icon, step: &str, t: &Theme) -> El {
    row()
        .key("next-0")
        .w_full()
        .h(66.0)
        .px(16.0)
        .gap(10.0)
        .center_y()
        .radius(10.0)
        .fill(t.card)
        .box_shadow(kit::ring(kit::HAIR, t.separator))
        .child(
            el().key("next-0-tile")
                .a11y_hidden()
                .size(40.0, 40.0)
                .radius(11.0)
                .fill(t.separator)
                .center()
                .child(kit::glyph("next-0-icon", icon, 20.0, t.text)),
        )
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .child(kit::label("next-0-cap", "Next", type_::CAPTION, t.text_tertiary))
                .child(kit::label("next-0-title", step, type_::HEAD, t.text)),
        )
}

/// The ring of DW-A43 (`.kit-ring` at 64): a 4-wide track and `fraction` of it in ink.
pub fn hero_ring(fraction: f32, t: &Theme) -> El {
    let (d, c, r) = (64.0_f32, 32.0_f32, 30.0_f32);
    let arc = |to: f32| -> Vec<[f32; 2]> {
        let steps = (to * 96.0).ceil().max(1.0) as usize;
        (0..=steps)
            .map(|i| {
                let a = to * i as f32 / steps as f32 * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
                [c + r * a.cos(), c + r * a.sin()]
            })
            .collect()
    };
    let shape = Shape::new()
        .view_box(d, d)
        .stroke_with(arc(1.0), true, 4.0, LineCap::Butt, t.separator)
        .stroke_with(arc(fraction.clamp(0.0005, 1.0)), false, 4.0, LineCap::Round, t.text);
    el().key("hero")
        .a11y_hidden()
        .self_center()
        .size(d, d)
        .mb(4.0)
        .child(canvas(shape).key("hero-ring").a11y_hidden().size(d, d))
}

/// What [`field`] draws beside the text.
pub struct Field<'a> {
    /// The words above the field.
    pub label: &'a str,
    /// The field's name for assistive technology, where it says more than the label.
    pub name: Option<&'a str>,
    pub icon: Option<&'static Icon>,
    /// The words under the field: a hint, or the problem when `error`.
    pub message: Option<&'a str>,
    pub error: bool,
    pub mono: bool,
    /// Draw the board's focused look and caret as shapes (a scene).
    pub pictured: bool,
}

/// `au_entry::focused_field`, with the two things sign-up adds: a field can hold the keyboard
/// and be in error at once (the red line inside the amber ring, DW-A33 and DW-A38), and
/// `under` sits between the field and its message (the strength meter).
#[allow(clippy::too_many_arguments)]
pub fn field(
    keys: &mut Keys,
    spec: Field<'_>,
    edit: &TextEdit,
    input_key: &'static str,
    on_edit: impl for<'a> Fn(&'a mut State) -> &'a mut TextEdit + 'static,
    on_change: impl Fn(&mut State, &mut Tx) + 'static,
    on_key: fn(&mut State, &KeyEvent, &mut Tx) -> bool,
    trailing: Option<El>,
    under: Option<El>,
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
    let (value_key, box_key) = (format!("{key}-value"), format!("{key}-value-box"));
    // What the gate is told, and how wide the typed text is. A secure field that is shown is
    // never measured as text: its width is its character count (mono).
    let width = match au::gate_text(edit) {
        Some(shown) if !shown.is_empty() => {
            kit::note_text(&value_key, &box_key, &shown, style, t.text);
            rune::text::advance(&shown, style)
        }
        None if chars > 0 => {
            debug_assert!(spec.mono, "a shown secret is set in mono");
            kit::note_text(&value_key, &box_key, &edit.text, style, t.text);
            mono_width(chars, style)
        }
        _ => 0.0,
    };
    let mut input = text_input(edit, style)
        .key(input_key)
        .label(spec.name.unwrap_or(spec.label))
        .single_line()
        .color(t.text)
        .caret(t.accent_deep, t.selection)
        .invalid(spec.error)
        .on_edit(on_edit)
        .on_input_change(on_change)
        .on_input_key(on_key)
        .grow(1.0)
        .min_w(0.0);
    if let Some(message) = spec.message {
        input = input.description(message);
    }
    let line = if spec.error { t.red } else { t.accent_deep };
    let mut well = row()
        .key(key.clone())
        .w_full()
        .h(28.0)
        .px(8.0)
        .gap(6.0)
        .center_y()
        .radius(6.0)
        .border_layout();
    well = if spec.pictured {
        well.fill(t.card)
            .border(1.0, line)
            .box_shadow(kit::ring(3.0, t.accent.alpha(0.30)))
    } else if spec.error {
        well.fill(t.field)
            .border(1.0, t.red)
            .box_shadow(kit::ring(3.5, t.red.alpha(0.16)))
    } else {
        well.fill(t.field)
            .border(1.0, Color::TRANSPARENT)
            .box_shadow(kit::ring_inset(t.separator))
            .focus_within_ring(1.0, -1.0, t.accent_deep)
    };
    if let Some(icon) = spec.icon {
        well = well.child(kit::glyph(format!("{key}-icon"), icon, 16.0, t.text_tertiary));
    }
    let mut typed = row()
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .center_y()
        .child(input)
        .child(el().key(box_key).a11y_hidden().absolute(0.0, 0.0).w(width).h_full());
    if spec.pictured {
        // As `au_entry::focused_field`: the caret hangs a quarter point higher beside mono text.
        let top = if spec.mono { 3.5 } else { 4.0 };
        typed = typed.child(
            el().key(format!("{key}-caret"))
                .a11y_hidden()
                .absolute(width + 1.0, top)
                .size(1.5, 17.0)
                .radius(1.0)
                .fill(t.accent_deep),
        );
    }
    well = well.child(typed);
    if let Some(trailing) = trailing {
        well = well.child(trailing.key(format!("{key}-act")));
    }
    let ringed = if spec.error || spec.pictured {
        well
    } else {
        el().w_full()
            .radius(6.0)
            .focus_within_ring(3.0, 0.0, t.accent.alpha(0.30))
            .child(well)
    };
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
    wrap = wrap.child(ringed);
    if let Some(under) = under {
        wrap = wrap.child(under);
    }
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

/// A dim sentence that ends in a figure set in mono ("You can ask for a new code in 0:42"),
/// centred. Keys `<key>` (of `<key>-box`) and `<key>-time`.
pub fn sentence_with_time(key: &str, lead: &str, time: &str, t: &Theme) -> El {
    let box_key = format!("{key}-box");
    kit::note_text(key, &box_key, lead, type_au::SUB, t.text_tertiary);
    // The board's text node ends in the space before the figure.
    let width = rune::text::advance(lead, type_au::SUB) + 3.33;
    el().w_full().h(15.0).center().child(
        row()
            .center_y()
            .child(
                el().key(box_key)
                    .w(width)
                    .child(text(lead).font(type_au::SUB).color(t.text_tertiary)),
            )
            .child(kit::label(
                format!("{key}-time"),
                time,
                type_au::SUB_MONO,
                t.text_tertiary,
            )),
    )
}
