//! The banner under the toolbar (`.mk-banner`): one sentence about the whole window, an action
//! where there is a place to solve it, and a way to dismiss it where the board allows that.
//!
//! Keys: `banner`, `banner-icon`, `banner-text` (the sentence's box; its figures are
//! `banner-figure-<i>`), and where drawn `banner-action`, `banner-action-label`,
//! `banner-dismiss`, `banner-dismiss-icon`.
//!
//! It is a status (`role="status"` on the board): read when there is a pause, never an alert.

use crate::State;
use crate::theme::tokens::{Theme, type_};
use rune::tok::Family;
use rune::{Color, El, Icon, LiveRegion, TextStyle, Tx, el, oklch, row};

use super::{global_icons, kit};

/// `.mk-banner`: 12.5/400 on a 17 line.
pub const TEXT: TextStyle = TextStyle::new(12.5, 400.0).lh(17.0);
/// A figure inside the sentence (`.font-mono`): a time, a size.
pub const FIGURE: TextStyle = TextStyle::new(12.5, 400.0).family(Family::Mono).lh(17.0);

/// What the banner's wash and glyph say (`.mk-banner`, `.tone-warn`, `.tone-red`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    /// Nothing is wrong with the files (offline).
    Neutral,
    /// Needs the person soon (storage almost full, the session ended).
    Warn,
    /// Something stopped or failed.
    Red,
}

/// `--mk-warn`: "color-mix(in oklch, var(--color-amber-deep) 45%, var(--color-red))".
pub fn warn(t: &Theme) -> Color {
    if t.dark {
        oklch(0.6995, 0.183, 46.15)
    } else {
        oklch(0.575, 0.1785, 46.15)
    }
}

/// The wash and the glyph's colour of a tone.
pub fn tone(tone: Tone, t: &Theme) -> (Color, Color) {
    match tone {
        Tone::Neutral => (t.text.alpha(0.06), t.text_secondary),
        Tone::Warn => (warn(t).alpha(0.12), warn(t)),
        Tone::Red => (t.red.alpha(0.10), t.red),
    }
}

/// A piece of the sentence: words for people (Inter), or a figure (JetBrains Mono).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Run {
    Words(String),
    Figure(String),
}

/// The sentence as one string: what assistive technology reads, and what the gate compares.
pub fn sentence(runs: &[Run]) -> String {
    runs.iter()
        .map(|run| match run {
            Run::Words(s) | Run::Figure(s) => s.as_str(),
        })
        .collect()
}

pub type Handler = fn(&mut State, &mut Tx);

/// One banner. `action`: the button's word and what it does. `dismiss`: what the cross does
/// (`None`: this banner cannot be dismissed, and no cross is drawn).
pub struct Banner<'a> {
    pub tone: Tone,
    pub icon: &'static Icon,
    pub runs: &'a [Run],
    pub action: Option<(&'a str, Handler)>,
    pub dismiss: Option<Handler>,
}

/// The sentence: its runs side by side on one line. Keys: `<key>-box` (the line's box, which
/// the gate reads as `<key>`) and `<figure>-<i>` per figure.
pub fn runs_line(key: &str, figure: &str, runs: &[Run], words: TextStyle, figures: TextStyle, ink: Color) -> El {
    let box_key = format!("{key}-box");
    kit::note_text(key, &box_key, &sentence(runs), words, ink);
    let mut line = row().key(box_key).a11y_hidden().items_center();
    let mut n = 0;
    for (i, run) in runs.iter().enumerate() {
        line = line.child(match run {
            Run::Words(s) => rune::text(s.clone())
                .key(format!("{key}-run-{i}"))
                .font(words)
                .color(ink),
            Run::Figure(s) => {
                n += 1;
                kit::label(format!("{figure}-{}", n - 1), s.clone(), figures, ink)
            }
        });
    }
    line
}

/// The banner, as wide as the content area less its 14 pt margins, 10 pt under the toolbar.
pub fn banner(b: &Banner, t: &Theme) -> El {
    let (wash, mark) = tone(b.tone, t);
    let mut out = row()
        .key("banner")
        .role("status")
        .label(sentence(b.runs))
        .live_region(LiveRegion::Polite)
        .mt(10.0)
        .mx(14.0)
        .pt(9.0)
        .pb(9.0)
        .pl(12.0)
        .pr(10.0)
        .gap(10.0)
        .items_center()
        .radius(10.0)
        .fill(wash)
        .child(kit::glyph("banner-icon", b.icon, 16.0, mark))
        .child(
            runs_line("banner-text", "banner-figure", b.runs, TEXT, FIGURE, t.text_secondary)
                .grow(1.0)
                .min_w(0.0),
        );
    if let Some((word, run)) = b.action {
        // `.kit-btn.kit-btn-sm`, outlined.
        out = out.child(
            row()
                .key("banner-action")
                .role("button")
                .label(word)
                .h(22.0)
                .px(12.0)
                .center()
                .radius(6.0)
                .fill(t.paper)
                .border(1.0, t.card_border)
                .border_layout()
                .hover(t.hover)
                .press(t.press)
                .focusable(true)
                .focus_ring(2.0, 2.0, t.accent_deep)
                .cursor(rune::Cursor::Pointer)
                .on_press_tx(move |s: &mut State, tx: &mut Tx| run(s, tx))
                .child(kit::label("banner-action-label", word, type_::BUTTON_SM, t.text)),
        );
    }
    if let Some(run) = b.dismiss {
        out = out.child(
            el().key("banner-dismiss")
                .role("button")
                .label("Dismiss")
                .size(13.0, 13.0)
                .radius(3.0)
                .focusable(true)
                .focus_ring(2.0, 2.0, t.accent_deep)
                .cursor(rune::Cursor::Pointer)
                .on_press_tx(move |s: &mut State, tx: &mut Tx| run(s, tx))
                .child(kit::glyph(
                    "banner-dismiss-icon",
                    &global_icons::CLOSE,
                    13.0,
                    t.text_tertiary,
                )),
        );
    }
    out
}
