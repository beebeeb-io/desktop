//! The account screens' kit (the board's `.au-*` layout and the `.kit-btn`, `.kit-field`,
//! `.kit-banner`, `.kit-pill`, `.kit-item`, `.kit-ring`, `.kit-tile` controls at the desktop
//! density): what sign-in, recovery and the screens before Files are built from.
//!
//! Keys count in document order, as `scripts/keymap-signin.js` counts the board's elements:
//! the second button of a frame is `btn-1` on both sides. A frame builds its column top to
//! bottom with one [`Keys`], so the numbers fall out of the order it draws in.
//!
//! Every measurement is the board's computed CSS, read from the goldens (`css` on each box).

use std::collections::BTreeMap;

use rune::{
    Align, Color, Cx, El, Icon, LineCap, Shape, Span, TextEdit, TextStyle, canvas, col, el, oklch, row, text,
    text_input,
};

use super::{caption, kit};
use crate::State;
use crate::theme::tokens::{Theme, type_};

// ---------------------------------------------------------------------------
// Type (the board's `.kit-p-desktop` scale, where `tokens::type_` has no entry yet)
// ---------------------------------------------------------------------------

pub mod type_au {
    use rune::TextStyle;
    use rune::tok::Family;

    /// `.t-large`: 22px/650 on 28px, tracked -0.022em.
    pub const LARGE: TextStyle = TextStyle::new(22.0, 650.0).lh(28.0).tracking(-0.484);
    /// `.t-sub`: 12px/400 on 15px.
    pub const SUB: TextStyle = TextStyle::new(12.0, 400.0).lh(15.0);
    /// `.t-sub .au-mono`: the countdown's digits.
    pub const SUB_MONO: TextStyle = TextStyle::new(12.0, 400.0).family(Family::Mono).lh(15.0);
    /// `.kit-btn-lg` label: 13px/500 on 19.5px.
    pub const BUTTON_LG: TextStyle = TextStyle::new(13.0, 500.0).lh(19.5).tracking(-0.065);
    /// `.kit-btn-md` label: 13px/500 on 18.57px.
    pub const BUTTON_MD: TextStyle = TextStyle::new(13.0, 500.0).lh(18.5714).tracking(-0.065);
    /// `.kit-field-outer`: 11px/500 on 15px.
    pub const FIELD_LABEL: TextStyle = TextStyle::new(11.0, 500.0).lh(15.0);
    /// `.kit-field-value`: 13px/400.
    pub const FIELD_VALUE: TextStyle = TextStyle::new(13.0, 400.0).lh(16.0);
    /// `.kit-field-value.is-secure`: tracked 0.12em.
    pub const FIELD_SECURE: TextStyle = TextStyle::new(13.0, 400.0).lh(16.0).tracking(1.56);
    /// `.kit-field-value.font-mono`: 11.5px mono.
    pub const FIELD_MONO: TextStyle = TextStyle::new(11.5, 400.0).family(Family::Mono).lh(16.0);
    /// `.kit-field-msg`: 12px/400 on 16px.
    pub const FIELD_MSG: TextStyle = TextStyle::new(12.0, 400.0).lh(16.0);
    /// `.kit-banner-text b`: 13px/600 on 17px.
    pub const BANNER_TITLE: TextStyle = TextStyle::new(13.0, 600.0).lh(17.0);
    /// `.kit-banner-text`: 12.5px/400 on 17px.
    pub const BANNER_TEXT: TextStyle = TextStyle::new(12.5, 400.0).lh(17.0);
    /// `.kit-pill`: 11px/600, tracked 0.01em.
    pub const PILL: TextStyle = TextStyle::new(11.0, 600.0).lh(13.0).tracking(0.11);
    /// `.kit-item-value`: 13px/400 on 17px.
    pub const ITEM_VALUE: TextStyle = TextStyle::new(13.0, 400.0).lh(17.0);
    /// `.au-code` at 22px: mono 600, tracked 0.14em.
    pub const CODE: TextStyle = TextStyle::new(22.0, 600.0).family(Family::Mono).lh(17.0).tracking(3.08);
    /// The wordmark beside a mark `mark` points across (the board scales it with the mark).
    pub const fn wordmark(mark: f32) -> TextStyle {
        let size = mark * 15.6923 / 24.0;
        TextStyle::new(size, 700.0).lh(size).tracking(-size / 36.0)
    }
    /// Initials in an avatar `d` points across: 15px in 40, 11px in 28.
    pub const fn initials(d: f32) -> TextStyle {
        TextStyle::new(if d >= 40.0 { 15.0 } else { 11.0 }, 650.0)
    }
}

// ---------------------------------------------------------------------------
// Tones (`.tone-*`) and the colours the account screens add to the theme
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Neutral,
    Ink,
    Green,
    Red,
    /// Encryption state only (the brand rule): "keys arrived encrypted".
    Amber,
}

/// `(--tn, --tnt, --tb)`: the tone's mark colour, its text colour and its wash.
pub fn tone(tone: Tone, t: &Theme) -> (Color, Color, Color) {
    match tone {
        Tone::Neutral => (t.text_secondary, t.text_secondary, t.text.alpha(0.07)),
        Tone::Ink => (t.text, t.text, t.text.alpha(0.09)),
        Tone::Green => (t.green, green_text(t), t.green.alpha(0.14)),
        Tone::Red => (t.red, red_text(t), t.red.alpha(0.12)),
        Tone::Amber => (t.accent_deep, t.accent_deep, t.accent.alpha(0.22)),
    }
}

/// `--color-red-text`.
pub fn red_text(t: &Theme) -> Color {
    if t.dark {
        oklch(0.70, 0.19, 25.0)
    } else {
        oklch(0.48, 0.19, 25.0)
    }
}

/// `--color-green-text`.
pub fn green_text(t: &Theme) -> Color {
    if t.dark {
        oklch(0.72, 0.16, 155.0)
    } else {
        oklch(0.46, 0.13, 155.0)
    }
}

// ---------------------------------------------------------------------------
// Keys
// ---------------------------------------------------------------------------

/// Running numbers per kind of element, in the order a frame draws them.
#[derive(Default)]
pub struct Keys {
    counts: BTreeMap<&'static str, usize>,
}

impl Keys {
    pub fn new() -> Self {
        Keys::default()
    }

    /// `btn-0`, `btn-1`, …: the next key of this kind.
    pub fn next(&mut self, kind: &'static str) -> String {
        let n = self.counts.entry(kind).or_insert(0);
        let key = format!("{kind}-{n}");
        *n += 1;
        key
    }
}

// ---------------------------------------------------------------------------
// The window: caption, brand panel, the column
// ---------------------------------------------------------------------------

/// The three plain facts of the brand panel. Copy says Europe, never a provider or a city.
pub const FACTS: [&str; 3] = [
    "Encrypted on this PC before anything syncs. Beebeeb can't read it.",
    "One Beebeeb account for Drive, Pass, VPN, Chat and Mail.",
    "Stored in Europe.",
];

/// The lockup: the mark, "beebeeb.io" and the product name. `prefix` is `side` or `col`.
pub fn lockup(prefix: &str, mark: f32, product: &str, t: &Theme) -> El {
    let word = "beebeeb.io";
    let io = Span {
        start: 7,
        end: word.len() as u32,
        color: Some(t.accent),
        ..Span::default()
    };
    row()
        .key(format!("{prefix}-lockup"))
        .role("image")
        .label(format!("Beebeeb {product}"))
        .gap(mark * 6.461_54 / 24.0)
        .center_y()
        .child(kit::mark(format!("{prefix}-mark"), mark))
        .child(
            col()
                .gap(mark * 1.846_15 / 24.0)
                .child(kit::label(format!("{prefix}-word"), word, type_au::wordmark(mark), t.text).spans(vec![io]))
                .child(kit::label(
                    format!("{prefix}-product"),
                    product.to_uppercase(),
                    type_::CAPTION_MONO_SM,
                    t.text_secondary,
                )),
        )
}

fn side_panel(product: &str, icons: [&'static Icon; 3], t: &Theme) -> El {
    let mut facts = col().key("facts").role("list").gap(12.0);
    for (i, (fact, ic)) in FACTS.iter().zip(icons).enumerate() {
        facts = facts.child(
            row()
                .role("listitem")
                .gap(10.0)
                .items_start()
                .min_h(16.0)
                .child(kit::glyph(format!("fact-{i}-icon"), ic, 15.0, t.text_tertiary).mt(1.0))
                .child({
                    // 320 less the panel's padding, the hairline, the icon and the gap.
                    let room = 320.0 - 2.0 * 26.0 - kit::HAIR - 15.0 - 10.0;
                    let words = kit::label(format!("fact-{i}"), *fact, type_au::SUB, t.text_secondary);
                    // A fact that fits one line is as wide as its words; a longer one wraps in the room.
                    if rune::text::advance(fact, type_au::SUB) <= room {
                        words
                    } else {
                        words.wrap().grow(1.0).min_w(0.0)
                    }
                }),
        );
    }
    col()
        .key("au-side")
        .role("complementary")
        .label("About Beebeeb")
        .w(320.0)
        .h_full()
        .px(26.0)
        .py(28.0)
        .gap(14.0)
        .fill(t.card)
        .border_sides([0.0, kit::HAIR, 0.0, 0.0], t.separator)
        .border_layout()
        .child(lockup("side", 34.0, product, t))
        .child(kit::label(
            "side-tagline",
            "Your files, encrypted on your devices.",
            type_au::SUB,
            t.text_secondary,
        ))
        .child(el().grow(1.0))
        .child(facts)
}

/// The account window (`.au-mac`): caption, the brand panel, and `column` centred in the
/// ground. `column` comes from [`column`].
pub fn page(product: &str, fact_icons: [&'static Icon; 3], column: El, t: &Theme, cx: &mut Cx<State>) -> El {
    col()
        .key("window")
        .role("window")
        .label("Beebeeb Drive")
        .w_full()
        .h_full()
        .fill(t.surface)
        .child(caption::caption_bar(t, cx))
        .child(
            row()
                .key("au-body")
                .grow(1.0)
                .min_h(0.0)
                .w_full()
                .child(side_panel(product, fact_icons, t))
                .child(
                    el().key("au-main")
                        .role("main")
                        .grow(1.0)
                        .min_w(0.0)
                        .h_full()
                        .px(20.0)
                        .py(20.0)
                        .fill(t.surface)
                        .center()
                        .child(column),
                ),
        )
}

/// The column (`.au-mac-col`): 344 wide, 11 between its parts.
pub fn column() -> El {
    col().key("au-col").w(344.0).gap(11.0)
}

// ---------------------------------------------------------------------------
// Text
// ---------------------------------------------------------------------------

/// "‹ Sign in" above the heading: back to where the screen was opened from.
pub fn back_link(label: &str, chevron: &'static Icon, t: &Theme) -> El {
    row()
        .key("back-link")
        .role("link")
        .label(format!("Back to {label}"))
        .self_start()
        .gap(4.0)
        .center_y()
        .h(15.0)
        .focusable(true)
        .focus_ring(2.0, 2.0, t.accent_deep)
        .radius(4.0)
        .cursor(rune::Cursor::Pointer)
        .child(kit::glyph("back-icon", chevron, 14.0, t.text_tertiary))
        .child(kit::label("back", label, type_au::SUB, t.text_tertiary))
}

/// The heading (`.au-h`): the title and one paragraph under it. The title is the screen's
/// first element for assistive technology.
pub fn heading(title: &str, subtitle: Option<&str>, centered: bool, t: &Theme) -> El {
    let align = if centered { Align::Center } else { Align::Start };
    // Focus can be moved to the title (a result that replaces the screen: DW-A08); Tab skips it.
    let title = kit::label("title", title, type_au::LARGE, t.text)
        .role("heading")
        .level(1)
        .focusable(true)
        .tab_index(-1)
        .no_focus_ring();
    let mut h = col().key("au-h").gap(6.0).w_full();
    h = if centered {
        h.items_center().child(title)
    } else {
        h.child(title.w_full())
    };
    if let Some(subtitle) = subtitle {
        h = h.child(
            kit::label("subtitle", subtitle, type_au::SUB, t.text_secondary)
                .wrap()
                .w_full()
                .align(align),
        );
    }
    h
}

/// The underline of a link, as the board's browser draws it (`text-underline-offset: 2px`,
/// skipping the ink of descenders): bars `(x, width)` under `label` set in `style`.
/// Rune's own span underline sits lower and runs through descenders (see the PR's Rune notes).
fn underline_bars(label: &str, style: TextStyle) -> Vec<(f32, f32)> {
    let mut bars = Vec::new();
    let mut from = 0.0;
    let mut x = 0.0;
    for (i, ch) in label.char_indices() {
        let end = rune::text::advance(&label[..i + ch.len_utf8()], style);
        let w = end - x;
        // The part of a glyph that reaches below the baseline, as a fraction of its advance.
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

/// An underlined link (`.au-link`), with an icon before it if `icon` is given.
pub fn link(keys: &mut Keys, label: &str, icon: Option<&'static Icon>, t: &Theme) -> El {
    let key = keys.next("link");
    let mut r = row()
        .key(format!("{key}-box"))
        .role("link")
        .label(label)
        .gap(5.0)
        .center_y()
        .focusable(true)
        .focus_ring(2.0, 2.0, t.accent_deep)
        .radius(4.0)
        .cursor(rune::Cursor::Pointer);
    if let Some(icon) = icon {
        r = r.child(kit::glyph(format!("{key}-icon"), icon, 15.0, t.text));
    }
    // 12px Inter on a 15px line: the baseline is 11.875 down, the underline 2 below it.
    let bars = underline_bars(label, type_au::SUB)
        .into_iter()
        .enumerate()
        .map(|(i, (x, w))| {
            el().key(format!("{key}-underline-{i}"))
                .absolute(x, 13.875)
                .w(w)
                .h(0.875)
                .fill(t.text)
        });
    r.child(
        el().child(kit::label(key.clone(), label, type_au::SUB, t.text))
            .children(bars.collect::<Vec<_>>()),
    )
}

/// "New to Beebeeb? Create an account": a dim sentence that ends in a link, centred.
/// Returns the row; the link inside is keyed `link-N` and takes the press.
pub fn sentence_link(keys: &mut Keys, lead: &str, label: &str, t: &Theme, link_el: impl FnOnce(El) -> El) -> El {
    let note_key = keys.next("note");
    let row_key = format!("{note_key}-row");
    kit::note_text(
        &note_key,
        &row_key,
        &format!("{lead} {label}"),
        type_au::SUB,
        t.text_tertiary,
    );
    el().key(row_key).w_full().h(15.0).center().child(
        row()
            .gap(3.33)
            .center_y()
            .child(text(lead).font(type_au::SUB).color(t.text_tertiary).a11y_hidden())
            .child(link_el(link(keys, label, None, t))),
    )
}

/// The divider with words in it (`.au-or`).
pub fn or_divider(keys: &mut Keys, words: &str, t: &Theme) -> El {
    let key = keys.next("or");
    let line = |i: usize| el().key(format!("{key}-line-{i}")).grow(1.0).h(1.0).fill(t.separator);
    row()
        .key(format!("{key}-row"))
        .a11y_hidden()
        .w_full()
        .h(14.0)
        .gap(10.0)
        .center_y()
        .child(line(0))
        .child(kit::label(key.clone(), words, type_::CAPTION, t.text_tertiary))
        .child(line(1))
}

// ---------------------------------------------------------------------------
// Buttons (`.kit-btn`)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonKind {
    /// Amber: the one primary action of a screen.
    Primary,
    /// Paper with a 1px line.
    Outline,
    /// No fill, secondary ink.
    Ghost,
}

/// How a button stands: the board draws a working button at half strength when pressing it
/// only re-checks (offline), and a running one at full strength with its handler off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonState {
    Enabled,
    /// Half strength, still pressable (DW-A20: "dimmed but tappable").
    Dim,
    /// Work is running: full strength, takes no press and no focus (DW-A18).
    Busy,
    /// Not yet possible: half strength, takes no press and no focus.
    Disabled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonSize {
    /// 32 high.
    Lg,
    /// 28 high.
    Md,
}

/// A full-width button. The caller adds `.on_press(..)` (not for `Busy` or `Disabled`).
pub fn button(
    keys: &mut Keys,
    label: &str,
    icon: Option<&'static Icon>,
    kind: ButtonKind,
    size: ButtonSize,
    state: ButtonState,
    t: &Theme,
) -> El {
    let key = keys.next("btn");
    let (ink, style, h) = (
        match kind {
            ButtonKind::Primary => t.accent_text_on,
            ButtonKind::Outline => t.text,
            ButtonKind::Ghost => t.text_secondary,
        },
        if size == ButtonSize::Lg {
            type_au::BUTTON_LG
        } else {
            type_au::BUTTON_MD
        },
        if size == ButtonSize::Lg { 32.0 } else { 28.0 },
    );
    let mut b = row()
        .key(key.clone())
        .role("button")
        .label(label)
        .w_full()
        .h(h)
        .px(12.0)
        .gap(7.0)
        .center()
        .radius(6.0);
    b = match kind {
        ButtonKind::Primary => b
            .fill(t.accent)
            .hover(oklch(0.78, 0.17, 84.0))
            .press(oklch(0.74, 0.17, 84.0)),
        ButtonKind::Outline => b
            .fill(t.paper)
            .border(1.0, t.card_border)
            .border_layout()
            .hover(t.paper2)
            .press(t.paper3),
        ButtonKind::Ghost => b.hover(t.hover).press(t.press),
    };
    if matches!(state, ButtonState::Dim | ButtonState::Disabled) {
        b = b.opacity(0.5);
    }
    b = if matches!(state, ButtonState::Busy | ButtonState::Disabled) {
        b.disabled(true)
    } else {
        b.focusable(true)
            .focus_ring(2.0, 2.0, t.accent_deep)
            .cursor(rune::Cursor::Pointer)
    };
    if let Some(icon) = icon {
        b = b.child(kit::glyph(format!("{key}-icon"), icon, 16.0, ink));
    }
    b.child(kit::label(format!("{key}-label"), label, style, ink))
}

// ---------------------------------------------------------------------------
// Banner (`.kit-banner`), pill (`.kit-pill`)
// ---------------------------------------------------------------------------

/// A banner: an icon, a title (optional) and a sentence. `alert` announces it at once (an
/// error); otherwise it is a polite status (the board: "Banner is a status, not an alert").
pub fn banner(
    keys: &mut Keys,
    tone_: Tone,
    icon: &'static Icon,
    title: Option<&str>,
    body: Option<&str>,
    alert: bool,
    t: &Theme,
) -> El {
    let key = keys.next("banner");
    let (mark, _, wash) = tone(tone_, t);
    let mut words = col().gap(2.0).grow(1.0).min_w(0.0);
    if let Some(title) = title {
        words = words.child(
            kit::label(format!("{key}-title"), title, type_au::BANNER_TITLE, t.text)
                .wrap()
                .w_full(),
        );
    }
    if let Some(body) = body {
        words = words.child(
            kit::label(format!("{key}-text"), body, type_au::BANNER_TEXT, t.text_secondary)
                .wrap()
                .w_full(),
        );
    }
    row()
        .key(key.clone())
        .role(if alert { "alert" } else { "status" })
        .live_region(if alert {
            rune::LiveRegion::Assertive
        } else {
            rune::LiveRegion::Polite
        })
        .w_full()
        .px(13.0)
        .py(11.0)
        .gap(10.0)
        .items_start()
        .radius(13.0)
        .fill(wash)
        .child(kit::glyph(format!("{key}-icon"), icon, 16.0, mark).mt(1.0))
        .child(words)
}

/// A pill: a short state, with a dot before it when `dot`.
pub fn pill(key: &str, label: &str, tone_: Tone, dot: bool, t: &Theme) -> El {
    let (mark, ink, wash) = tone(tone_, t);
    let mut p = row()
        .key(key.to_string())
        .role("status")
        .label(label)
        .h(20.0)
        .px(8.0)
        .gap(5.0)
        .center_y()
        .radius(10.0)
        .fill(wash);
    if dot {
        p = p.child(el().key("status-dot").size(6.0, 6.0).radius(3.0).fill(mark));
    }
    p.child(kit::label(format!("{key}-text"), label, type_au::PILL, ink))
}

// ---------------------------------------------------------------------------
// Fields (`.kit-fieldwrap` > `.kit-field`)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FieldState {
    #[default]
    Rest,
    /// The red border and ring; the message under the field is announced.
    Error,
}

/// What a field shows beside its text.
pub struct FieldSpec<'a> {
    pub label: &'a str,
    pub icon: Option<&'static Icon>,
    pub state: FieldState,
    /// The message under the field (an error's words).
    pub message: Option<&'a str>,
    /// Mono text (a backup code).
    pub mono: bool,
}

/// What the frame gate is told a field holds, and what this file measures: a plain field's
/// text (borrowed), a hidden secure field's bullets (their count, never the characters), and
/// NOTHING for a secure field that is shown. A secret is drawn by Rune's secure text input
/// alone, which keeps it out of the shared layout cache (`rune/src/text/rich_layout.rs:137-139`)
/// and wipes its buffers; it is never copied into a `String` here, measured with
/// `rune::text::advance` (the shared cache, `rune/src/text/measure.rs:216`) or noted.
pub fn gate_text(edit: &TextEdit) -> Option<std::borrow::Cow<'_, str>> {
    match (edit.is_secure(), edit.is_revealed()) {
        (false, _) => Some(std::borrow::Cow::Borrowed(edit.text.as_str())),
        (true, false) => Some(std::borrow::Cow::Owned("\u{2022}".repeat(edit.text.chars().count()))),
        (true, true) => None,
    }
}

/// A labelled one-line field. `input_key` is the text input's own key (focus, tests).
/// `trailing` is the button at the field's end (show or hide the password), if any.
///
/// A secure `edit` is drawn by Rune as bullets, and its text is in neither the element tree
/// nor the accessibility tree (`rune/src/el/input.rs:139-146`): the gate is told the bullets'
/// count, never the characters.
pub fn field(
    keys: &mut Keys,
    spec: FieldSpec<'_>,
    edit: &TextEdit,
    input_key: &'static str,
    on_edit: impl for<'a> Fn(&'a mut State) -> &'a mut TextEdit + 'static,
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
    let shown = gate_text(edit);
    if let Some(shown) = shown.as_deref().filter(|shown| !shown.is_empty()) {
        kit::note_text(
            &format!("{key}-value"),
            &format!("{key}-value-box"),
            shown,
            style,
            t.text,
        );
    }
    let error = spec.state == FieldState::Error;
    let mut input = text_input(edit, style)
        .key(input_key)
        .label(spec.label)
        .single_line()
        .color(t.text)
        .caret(t.accent_deep, t.selection)
        .invalid(error)
        .on_edit(on_edit)
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
        .fill(t.field)
        .border(1.0, if error { t.red } else { Color::TRANSPARENT })
        .border_layout();
    well = if error {
        well.box_shadow(kit::ring(3.5, t.red.alpha(0.16)))
    } else {
        well.box_shadow(kit::ring_inset(t.separator))
            .focus_within_ring(3.5, 0.0, t.accent.alpha(0.30))
    };
    if let Some(icon) = spec.icon {
        well = well.child(kit::glyph(format!("{key}-icon"), icon, 16.0, t.text_tertiary));
    }
    // The typed text's own box, for the gate: as wide as the text Rune draws in the input.
    let typed = el()
        .key(format!("{key}-value-box"))
        .a11y_hidden()
        .absolute(0.0, 0.0)
        .w(shown.as_deref().map_or(0.0, |shown| rune::text::advance(shown, style)))
        .h_full();
    well = well.child(row().grow(1.0).min_w(0.0).h_full().center_y().child(input).child(typed));
    if let Some(trailing) = trailing {
        well = well.child(trailing.key(format!("{key}-act")));
    }
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
                if error { red_text(t) } else { t.text_tertiary },
            )
            .role(if error { "alert" } else { "note" })
            .wrap()
            .mx(4.0),
        );
    }
    wrap
}

/// The button at a field's end (`.kit-field-act`): 22 square, an icon, a name.
/// `field` is the field's key (`field-1`): the icon is keyed `<field>-act-icon`.
pub fn field_action(field: &str, name: &str, icon: &'static Icon, t: &Theme) -> El {
    el().role("button")
        .label(name)
        .size(22.0, 22.0)
        .radius(9.0)
        .center()
        .hover(t.selection)
        .press(t.press)
        .focusable(true)
        .focus_ring(2.0, 0.0, t.accent_deep)
        .child(kit::glyph(format!("{field}-act-icon"), icon, 16.0, t.text_tertiary))
}

// ---------------------------------------------------------------------------
// Card (`.au-card`), avatar, list (`.kit-list`)
// ---------------------------------------------------------------------------

/// A raised card: 16 radius, a hairline ring, `pad` inside, `gap` between its parts.
pub fn card(keys: &mut Keys, pad: (f32, f32), gap: f32, t: &Theme) -> (String, El) {
    let key = keys.next("card");
    let c = col()
        .key(key.clone())
        .w_full()
        .px(pad.0)
        .py(pad.1)
        .gap(gap)
        .radius(16.0)
        .fill(t.card)
        .box_shadow(kit::ring(kit::HAIR, t.separator));
    (key, c)
}

/// A person's initials on their tint, ringed in `surface` (`.kit-avatar` at 40 and 28: the
/// initials are 15px and 11px, where `kit::avatar` draws 10px and 8px).
pub fn avatar(key: &str, text_key: &str, initials: &str, d: f32, hue: f32, surface: Color, t: &Theme) -> El {
    let style = type_au::initials(d);
    el().key(key.to_string())
        .a11y_hidden()
        .size(d, d)
        .radius(d / 2.0)
        .fill(t.avatar_tint(oklch(0.88, 0.05, hue)))
        .box_shadow(kit::ring(2.0, surface))
        .center()
        .child(kit::label(text_key.to_string(), initials, style, t.avatar_ink()))
}

/// The grouped list's body (`.kit-list-body`): a card that holds rows.
pub fn list(keys: &mut Keys, rows: Vec<El>, t: &Theme) -> El {
    col()
        .key(keys.next("list"))
        .role("list")
        .w_full()
        .radius(10.0)
        .fill(t.card)
        .box_shadow(kit::ring(kit::HAIR, t.separator))
        .children(rows)
}

/// One row of a list (`.kit-item`): icon, title, an optional second line, an optional value
/// at the end, an optional chevron. `danger` draws the title in red text.
#[allow(clippy::too_many_arguments)]
pub fn item(
    keys: &mut Keys,
    icon: &'static Icon,
    title: &str,
    sub: Option<&str>,
    value: Option<&str>,
    chevron: Option<&'static Icon>,
    danger: bool,
    t: &Theme,
) -> El {
    let key = keys.next("item");
    let first = key.ends_with("-0");
    let mut words = col().gap(1.0).grow(1.0).min_w(0.0).child(kit::label(
        format!("{key}-title"),
        title,
        type_::ITEM_TITLE,
        if danger { red_text(t) } else { t.text },
    ));
    if let Some(sub) = sub {
        words = words.child(kit::label(format!("{key}-sub"), sub, type_::CAPTION, t.text_tertiary));
    }
    let mut r = row()
        .key(key.clone())
        .role("listitem")
        .w_full()
        .min_h(if sub.is_some() { 40.0 } else { 28.0 })
        .px(10.0)
        .gap(8.0)
        .center_y()
        .child(
            el().w(30.0)
                .center()
                .child(kit::glyph(format!("{key}-icon"), icon, 18.0, t.text_secondary)),
        )
        .child(words);
    if !first {
        r = r.child(
            el().key(format!("{key}-rule"))
                .absolute(48.0, 0.0)
                .w(296.0)
                .h(kit::HAIR)
                .fill(t.separator),
        );
    }
    if let Some(value) = value {
        r = r.child(kit::label(
            format!("{key}-value"),
            value,
            type_au::ITEM_VALUE,
            t.text_tertiary,
        ));
    }
    if let Some(chevron) = chevron {
        r = r.child(kit::glyph(format!("{key}-chev"), chevron, 14.0, t.text_quaternary));
    }
    r
}

// ---------------------------------------------------------------------------
// Hero: a tile (`.kit-tile`) or a ring (`.kit-ring`) with an icon in it
// ---------------------------------------------------------------------------

/// The 72-point tile above a result's heading: the tone's wash, its icon 36 across.
pub fn hero_tile(icon: &'static Icon, tone_: Tone, top: f32, t: &Theme) -> El {
    let (mark, _, wash) = tone(tone_, t);
    el().key("hero").a11y_hidden().self_center().mt(top).mb(4.0).child(
        el().key("hero-tile")
            .size(72.0, 72.0)
            .radius(20.0)
            .fill(wash)
            .center()
            .child(kit::glyph("hero-icon", icon, 36.0, mark)),
    )
}

/// A progress ring `d` across, 3 wide: the hairline track and `fraction` of it in `color`,
/// from twelve o'clock, clockwise, round ends (`.kit-ring`).
pub fn ring(key: &str, d: f32, fraction: f32, color: Color, t: &Theme) -> El {
    let r = d / 2.0 - 1.5;
    let c = d / 2.0;
    let arc = |from: f32, to: f32| -> Vec<[f32; 2]> {
        let steps = ((to - from) * 96.0).ceil().max(1.0) as usize;
        (0..=steps)
            .map(|i| {
                let a = (from + (to - from) * i as f32 / steps as f32) * std::f32::consts::TAU
                    - std::f32::consts::FRAC_PI_2;
                [c + r * a.cos(), c + r * a.sin()]
            })
            .collect()
    };
    let mut shape = Shape::new()
        .view_box(d, d)
        .stroke_with(arc(0.0, 1.0), true, 3.0, LineCap::Butt, t.separator);
    // A zero-length dash with round caps is a dot at twelve o'clock, as the board draws it.
    shape = shape.stroke_with(arc(0.0, fraction.clamp(0.0005, 1.0)), false, 3.0, LineCap::Round, color);
    canvas(shape).key(key.to_string()).a11y_hidden().size(d, d)
}

/// The 104-point ring with an icon 46 across in it (Windows Hello).
pub fn hero_ring(icon: &'static Icon, fraction: f32, tone_: Tone, t: &Theme) -> El {
    let (mark, _, _) = tone(tone_, t);
    el().key("hero")
        .a11y_hidden()
        .self_center()
        .size(104.0, 104.0)
        .mt(8.0)
        .mb(4.0)
        .center()
        .child(ring("hero-ring", 104.0, fraction, mark, t).absolute(0.0, 0.0))
        .child(kit::glyph("hero-icon", icon, 46.0, t.text))
}
