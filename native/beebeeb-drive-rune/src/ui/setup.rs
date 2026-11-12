//! The setup window (the board's first run on Windows, DW-0201 to DW-0209): the account window
//! (`ui::au`) with the step list in its side panel, a wider column, and the controls the steps
//! add to the account kit.
//!
//! | Piece | Function | Keys it gives |
//! | --- | --- | --- |
//! | Window | `page(product, steps, status, skip, column, over, t, cx)` | `window`, `caption-bar`, `au-side`, `side-*`, `step-<I>-dot/-check/-n`, `step-<I>`, `side-status(-icon)`, `au-main`, `skip`, `scrim` |
//! | Column | `column()` | `au-col` (420 wide) |
//! | Segmented choice | `segmented(key, name, labels, chosen, size, t)` | `<key>`, `<key>-on`, `<key>-<J>` (+`-tab`) |
//! | Small button | `small_button(key, label, t)` | `<key>`, `<key>-label` |
//! | Row | `item(keys, icon, title, sub, trailing, t)` | `item-<N>`, `-icon`, `-title`, `-sub`, `-rule` |
//! | List | `list(keys, rows, foot, t)` | `list-<N>`, `note-<N>` |
//!
//! Keys count in document order, as `scripts/keymap-signin.js` and `keymap-signin-setup.js` count
//! the board's elements. Every measurement is the board's computed CSS, read from the goldens.

use rune::tok::Family;
use rune::{Color, Cx, El, Icon, TextStyle, col, el, row};

use super::au::{self, Keys, type_au};
use super::{caption, kit, settings_dialog};
use crate::State;
use crate::theme::tokens::{Theme, type_};

/// The side panel is 300 wide in the setup window (320 on the sign-in screens).
pub const SIDE_W: f32 = 300.0;
/// `.au-mac-col.is-wide`.
pub const COLUMN_W: f32 = 420.0;

/// A step's label: 12px on a 15px line; the step being done is 600.
const STEP: TextStyle = TextStyle::new(12.0, 400.0).lh(15.0);
const STEP_NOW: TextStyle = TextStyle::new(12.0, 600.0).lh(15.0);
/// The number in a step's dot: mono 10px.
const STEP_N: TextStyle = TextStyle::new(10.0, 400.0).family(Family::Mono).lh(13.0);
const STEP_N_NOW: TextStyle = TextStyle::new(10.0, 600.0).family(Family::Mono).lh(13.0);
/// `.kit-seg span`: 12.5px/550; `.kit-seg-sm span`: 12px/550.
const SEG: TextStyle = TextStyle::new(12.5, 550.0).lh(15.0);
const SEG_SM: TextStyle = TextStyle::new(12.0, 550.0).lh(14.5);
/// `.kit-list-foot` and a note under a list: 12px on a 16px line.
pub const FOOT: TextStyle = TextStyle::new(12.0, 400.0).lh(16.0);
/// `.kit-item-sub` at the desktop density: 12px on a 15px line.
const ITEM_SUB: TextStyle = TextStyle::new(12.0, 400.0).lh(15.0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepState {
    Done,
    Now,
    Ahead,
}

/// What the foot of the side panel says File Explorer is doing.
pub struct Status<'a> {
    pub icon: &'static Icon,
    pub words: &'a str,
    /// A failure: red text, announced.
    pub failed: bool,
}

/// The lockup as `au::lockup` draws it, with the product name on its natural line: the board's
/// "DRIVE" is `line-height: normal`, 11.5 for 9px JetBrains Mono, where `type_::CAPTION_MONO_SM`
/// has 12. Half a point matters where the lockup is in a centred column: it moves everything
/// under it a quarter point, off the board's pixel grid.
pub fn lockup(prefix: &str, mark: f32, name: &str, product: &str, t: &Theme) -> El {
    let word = "beebeeb.io";
    let io = rune::Span {
        color: Some(t.accent),
        ..rune::Span::new(7, word.len())
    };
    row()
        .key(format!("{prefix}-lockup"))
        .role("image")
        .label(format!("{name} {product}"))
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
                    type_::CAPTION_MONO_SM.lh(11.5),
                    t.text_secondary,
                )),
        )
}

fn steps_list(steps: &[(&str, StepState)], check: &'static Icon, t: &Theme) -> El {
    let ring = t.text.alpha(0.18);
    let mut list = col().key("steps").role("list").label("Setup steps").gap(4.0);
    for (i, (label, state)) in steps.iter().enumerate() {
        let mut dot = el()
            .key(format!("step-{i}-dot"))
            .a11y_hidden()
            .size(18.0, 18.0)
            .radius(9.0)
            .center();
        dot = match state {
            StepState::Done => {
                dot.border(1.0, ring)
                    .child(kit::glyph(format!("step-{i}-check"), check, 10.0, t.text_tertiary))
            }
            StepState::Now => dot.fill(t.text).child(kit::label(
                format!("step-{i}-n"),
                (i + 1).to_string(),
                STEP_N_NOW,
                t.paper,
            )),
            StepState::Ahead => dot.border(1.0, ring).child(kit::label(
                format!("step-{i}-n"),
                (i + 1).to_string(),
                STEP_N,
                t.text_tertiary,
            )),
        };
        let (style, ink, said) = match state {
            StepState::Done => (STEP, t.text_secondary, "done"),
            StepState::Now => (STEP_NOW, t.text, "current step"),
            StepState::Ahead => (STEP, t.text_tertiary, "not done yet"),
        };
        list = list.child(
            row()
                .key(format!("step-{i}-row"))
                .role("listitem")
                .label(format!("Step {} of {}: {label}, {said}", i + 1, steps.len()))
                .h(24.0)
                .gap(10.0)
                .items_start()
                .child(dot)
                .child(kit::label(format!("step-{i}"), *label, style, ink).a11y_hidden()),
        );
    }
    list
}

fn side_panel(
    name: &str,
    product: &str,
    steps: &[(&str, StepState)],
    check: &'static Icon,
    status: &Status<'_>,
    t: &Theme,
) -> El {
    let ink = if status.failed {
        au::red_text(t)
    } else {
        t.text_tertiary
    };
    col()
        .key("au-side")
        .role("complementary")
        .label("Setup progress")
        .w(SIDE_W)
        .h_full()
        .px(26.0)
        .py(28.0)
        .gap(14.0)
        .fill(t.card)
        .border_sides([0.0, kit::HAIR, 0.0, 0.0], t.separator)
        .border_layout()
        .child(lockup("side", 34.0, name, product, t))
        .child(kit::label(
            "side-tagline",
            format!("Set up {product} on this PC"),
            type_au::SUB,
            t.text_secondary,
        ))
        .child(steps_list(steps, check, t))
        .child(el().grow(1.0))
        .child(
            row()
                .key("side-status-row")
                .role("status")
                .live_region(if status.failed {
                    rune::LiveRegion::Assertive
                } else {
                    rune::LiveRegion::Polite
                })
                .label(status.words)
                .h(14.0)
                .gap(8.0)
                .center_y()
                .child(kit::glyph("side-status-icon", status.icon, 14.0, ink))
                .child(kit::label("side-status", status.words, type_::CAPTION, ink).a11y_hidden()),
        )
}

/// The setup window: ONE caption bar, the step list at the left, `column` centred in the
/// ground, `skip` at the main part's top right, and `over` (a sheet, from [`sheet`]) above
/// everything under the caption, on the scrim.
#[allow(clippy::too_many_arguments)]
pub fn page(
    name: &str,
    product: &str,
    steps: &[(&str, StepState)],
    check: &'static Icon,
    status: &Status<'_>,
    skip: Option<El>,
    column: El,
    over: Option<El>,
    t: &Theme,
    cx: &mut Cx<State>,
) -> El {
    let mut main = el()
        .key("au-main")
        .role("main")
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .px(20.0)
        .py(20.0)
        .fill(t.surface)
        .center()
        .child(column);
    if let Some(skip) = skip {
        main = main.child(el().key("skip-at").pin(None, Some(16.0), Some(20.0), None).child(skip));
    }
    let mut body = row()
        .key("au-body")
        .grow(1.0)
        .min_h(0.0)
        .w_full()
        .child(side_panel(name, product, steps, check, status, t))
        .child(main);
    if let Some(over) = over {
        body = body
            .child(
                el().key("scrim")
                    .a11y_hidden()
                    .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                    .fill(settings_dialog::scrim_color(t)),
            )
            .child(
                row()
                    .key("sheet-at")
                    .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                    // The sheet's shadow stays under the caption and inside the window.
                    .clip_rounded(0.0)
                    .items_start()
                    .justify_center()
                    .child(over),
            );
    }
    col()
        .key("window")
        .role("window")
        .label(format!("{name} {product}"))
        .w_full()
        .h_full()
        .fill(t.surface)
        .child(caption::caption_bar(&format!("{name} {product}"), t, cx))
        .child(body)
}

/// The column (`.au-mac-col.is-wide`): 420 wide, 11 between its parts.
pub fn column() -> El {
    col().key("au-col").w(COLUMN_W).gap(11.0)
}

/// "Skip setup", a link.
pub fn skip_link(t: &Theme) -> El {
    kit::label("skip", "Skip setup", type_au::SUB, t.text_tertiary)
        .role("link")
        .focusable(true)
        .focus_ring(2.0, 2.0, t.accent_deep)
        .radius(4.0)
        .cursor(rune::Cursor::Pointer)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SegSize {
    /// 28-point segments that share the control's whole width.
    Md,
    /// 24-point segments as wide as their words.
    Sm,
}

/// What a segmented control is drawn on.
#[derive(Clone, Copy)]
pub struct SegInk {
    pub track: Color,
    pub rest: Color,
}

/// A segmented choice (`.kit-seg`): one of `labels` is chosen. A radio group: the arrow keys
/// are the caller's (`on_key`), each segment is pressable. Returns the track and the segments
/// keyed `<key>-<J>-tab`, to which the caller adds `.on_press_tx`.
pub fn segmented(
    key: &str,
    name: &str,
    labels: &[&str],
    chosen: usize,
    size: SegSize,
    ink: Option<SegInk>,
    t: &Theme,
    mut press: impl FnMut(usize, El) -> El,
) -> El {
    let (h, px, style) = match size {
        SegSize::Md => (28.0, 12.0, SEG),
        SegSize::Sm => (24.0, 10.0, SEG_SM),
    };
    let ink = ink.unwrap_or(SegInk {
        track: t.field,
        rest: t.text_secondary,
    });
    let mut track = row()
        .key(key.to_string())
        .role("radiogroup")
        .label(name)
        .padding(2.0)
        .radius(10.0)
        .fill(ink.track)
        .box_shadow(kit::ring_inset(t.separator));
    if size == SegSize::Md {
        track = track.w_full();
    }
    for (j, label) in labels.iter().enumerate() {
        let on = j == chosen;
        let mut tab = row()
            .key(if on {
                format!("{key}-on")
            } else {
                format!("{key}-{j}-tab")
            })
            .role("radio")
            .label(*label)
            .checked(on)
            .h(h)
            .px(px)
            .center()
            .radius(8.0)
            .focusable(true)
            .focus_ring(2.0, 0.0, t.accent_deep)
            .cursor(rune::Cursor::Pointer);
        if size == SegSize::Md {
            tab = tab.w(0.0).grow(1.0);
        }
        if on {
            tab = tab
                .fill(t.card)
                .box_shadow(kit::drop(1.0, 2.0, Color::BLACK.alpha(0.12)))
                .box_shadow(kit::ring(kit::HAIR, t.separator));
        }
        tab = tab
            .child(kit::label(format!("{key}-{j}"), *label, style, if on { t.text } else { ink.rest }).a11y_hidden());
        track = track.child(press(j, tab));
    }
    track
}

/// A small outline button (`.kit-btn-sm`: 22 high, as wide as its label).
pub fn small_button(key: &str, label: &str, t: &Theme) -> El {
    row()
        .key(key.to_string())
        .role("button")
        .label(label)
        .h(22.0)
        .px(12.0)
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
        .child(kit::label(
            format!("{key}-label"),
            label,
            type_::BUTTON_SM.lh(17.1429),
            t.text,
        ))
}

/// One row of a list (`.kit-item` at the desktop density): 28 high, 40 with a second line (or
/// `min_h`). `trailing` sits at its end (a switch, a value, a segmented choice). `ruled`: a hairline
/// above it (the board rules a row off from the one above where the two are siblings in its markup).
pub fn item(
    keys: &mut Keys,
    icon: Option<&'static Icon>,
    title: &str,
    sub: Option<&str>,
    trailing: impl FnOnce(&str) -> Vec<El>,
    min_h: Option<f32>,
    ruled: bool,
    t: &Theme,
) -> El {
    let key = keys.next("item");
    let mut r = row()
        .key(key.clone())
        .role("listitem")
        .w_full()
        .min_h(min_h.unwrap_or(if sub.is_some() { 40.0 } else { 28.0 }))
        .px(10.0)
        .gap(8.0)
        .center_y();
    if let Some(icon) = icon {
        r = r.child(
            el().w(30.0)
                .center()
                .child(kit::glyph(format!("{key}-icon"), icon, 18.0, t.text_secondary)),
        );
    }
    let mut words = col().gap(1.0).grow(1.0).min_w(0.0).child(
        kit::label(format!("{key}-title"), title, type_::ITEM_TITLE, t.text)
            .w_full()
            .truncate(),
    );
    if let Some(sub) = sub {
        words = words.child(
            kit::label(format!("{key}-sub"), sub, ITEM_SUB, t.text_tertiary)
                .w_full()
                .truncate(),
        );
    }
    r = r.child(words);
    for piece in trailing(&key) {
        r = r.child(piece);
    }
    // The board's rule is the row's `::before`, positioned: it paints over what the row holds.
    if ruled {
        r = r.child(
            el().key(format!("{key}-rule"))
                .pin(Some(10.0), Some(0.0), Some(0.0), None)
                .h(kit::HAIR)
                .fill(t.separator),
        );
    }
    r
}

/// A row's value at its end (`.kit-item-value`).
pub fn value(row_key: &str, words: &str, t: &Theme) -> El {
    kit::label(format!("{row_key}-value"), words, type_au::ITEM_VALUE, t.text_tertiary)
}

/// A list (`.kit-list`): the card of rows and, under it, a note (`.kit-list-foot`).
pub fn list(keys: &mut Keys, rows: Vec<El>, foot: Option<&str>, t: &Theme) -> El {
    let body = col()
        .key(keys.next("list"))
        .role("list")
        .w_full()
        .radius(10.0)
        .fill(t.card)
        .box_shadow(kit::ring(kit::HAIR, t.separator))
        .child(col().w_full().clip_rounded(10.0).children(rows));
    let mut l = col().w_full().gap(6.0).child(body);
    if let Some(foot) = foot {
        l = l.child(note(keys, foot, FOOT, 16.0, t));
    }
    l
}

/// A paragraph that stands by itself in the column (`note-<N>`), `inset` from both edges.
pub fn note(keys: &mut Keys, words: &str, style: TextStyle, inset: f32, t: &Theme) -> El {
    kit::label(keys.next("note"), words, style, t.text_tertiary)
        .wrap()
        .w_full()
        .px(inset)
}

/// A sheet over the setup window (`.mk-sheet`): hangs from the caption, `w` by `h`, its lower
/// corners rounded. A dialog: it holds the keyboard while it is up.
pub fn sheet(name: &str, w: f32, h: f32, content: El, t: &Theme) -> El {
    col()
        .key("sheet")
        .role("dialog")
        .label(name)
        .size(w, h)
        .corners([0.0, 0.0, 14.0, 14.0])
        .fill(t.card)
        .box_shadow(kit::drop(24.0, 60.0, Color::BLACK.alpha(0.3)))
        .box_shadow(kit::ring(kit::HAIR, t.separator))
        .child(content)
}
