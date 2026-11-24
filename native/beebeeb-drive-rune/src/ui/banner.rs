//! The banner under the toolbar (`.mk-banner`): one sentence about the whole window, an action
//! where there is a place to solve it, and a way to dismiss it where the board allows that.
//!
//! Keys: `banner`, `banner-icon`, `banner-text` (the sentence's box; its figures are
//! `banner-figure-<i>`), and where drawn `banner-action`, `banner-action-label`,
//! `banner-dismiss`, `banner-dismiss-icon`.
//!
//! It is a status (`role="status"` on the board): read when there is a pause, never an alert.

use crate::State;
use crate::theme::tokens::Theme;
use rune::tok::Family;
use rune::{Color, El, Icon, Key, KeyEvent, LiveRegion, TextStyle, Tx, el, oklch, row};

use super::button::{self as btn, ButtonKeys, ButtonState, Size, Variant};
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

/// A button that Enter presses too, as Windows users expect. Rune presses a focused control on
/// Space, and on Return only a row, a link or the window's default button (rune README, "Key
/// routing", step 5), so the button hears Return itself.
pub fn pressable(button: El, run: Handler) -> El {
    button.on_press_tx(move |s: &mut State, tx: &mut Tx| run(s, tx)).on_key(
        move |s: &mut State, ev: &KeyEvent, tx: &mut Tx| {
            let plain = !(ev.mods.shift || ev.mods.cmd || ev.mods.alt || ev.mods.ctrl);
            if ev.key == Key::Enter && plain {
                run(s, tx);
                return true;
            }
            false
        },
    )
}

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
    // One text element holding the whole sentence; the runs inside stay out of the tree so the
    // sentence is not read twice or in pieces. A text element's UIA Name is its VALUE (AccessKit
    // `label_comes_from_value`), and Rune sets that only from the element's own text or
    // `a11y_value`; a label alone leaves the Text with no Name (RC2-15b missed this).
    let mut line = row()
        .key(box_key)
        .role("text")
        .label(sentence(runs))
        .a11y_value(sentence(runs))
        .items_center();
    let mut n = 0;
    for (i, run) in runs.iter().enumerate() {
        line = line.child(match run {
            Run::Words(s) => rune::text(s.clone())
                .key(format!("{key}-run-{i}"))
                .font(words)
                .color(ink)
                .a11y_hidden(),
            Run::Figure(s) => {
                n += 1;
                kit::label(format!("{figure}-{}", n - 1), s.clone(), figures, ink).a11y_hidden()
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
        out = out.child(pressable(
            btn::button(
                ButtonKeys {
                    root: "banner-action".into(),
                    icon: "banner-action-icon".into(),
                    label: "banner-action-label".into(),
                },
                word,
                None,
                Variant::Secondary,
                Size::Sm,
                ButtonState::Enabled,
                t,
            ),
            run,
        ));
    }
    if let Some(run) = b.dismiss {
        out = out.child(pressable(
            el().key("banner-dismiss")
                .role("button")
                .label("Dismiss")
                .size(13.0, 13.0)
                .radius(3.0)
                .focusable(true)
                .focus_ring(2.0, 2.0, t.accent_deep)
                .cursor(rune::Cursor::Pointer)
                .child(kit::glyph(
                    "banner-dismiss-icon",
                    &global_icons::CLOSE,
                    13.0,
                    t.text_tertiary,
                )),
            run,
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::tokens::theme;
    use rune::{Cx, Env, Runtime};
    use serde_json::Value;

    struct Page;

    fn runs() -> Vec<Run> {
        vec![
            Run::Words("Rebuilt at ".into()),
            Run::Figure("14:02".into()),
            Run::Words(" from your files.".into()),
        ]
    }

    fn view(_: &Page, _: &mut Cx<Page>) -> El {
        let r = runs();
        let b = Banner { tone: Tone::Neutral, icon: &global_icons::CLOSE, runs: &r, action: None, dismiss: None };
        row().key("page").w(500.0).child(banner(&b, &theme(false)))
    }

    fn walk<'a>(v: &'a Value, out: &mut Vec<&'a Value>) {
        out.push(v);
        for c in v["children"].as_array().into_iter().flatten() {
            walk(c, out);
        }
    }

    #[test]
    fn banner_uia_sentence_is_a_text_element_and_the_row_stays_a_status() {
        let env = Env { system_dark: false, scale: 2.0 };
        let mut rt = Runtime::new(Page, view, (500.0, 200.0), env);
        rt.frame(0.0);
        rt.settle_all();
        rt.frame(0.0);
        let tree = rt.accessibility_json();
        let mut all = Vec::new();
        walk(&tree, &mut all);
        let sentence = "Rebuilt at 14:02 from your files.";
        // What Windows UIA reads: a Text's Name is its value, so check the raw nodes, not the
        // dump's name (which falls back to the label and hid this in RC2-15b).
        let raw = rt.accessibility_tree();
        let texts: Vec<_> = raw
            .nodes
            .iter()
            .filter(|(_, n)| format!("{:?}", n.role()) == "Label" && n.value() == Some(sentence))
            .collect();
        assert_eq!(texts.len(), 1, "exactly one Text node holds the sentence as its value");
        let named: Vec<_> = all.iter().filter(|n| n["label"] == sentence || n["name"] == sentence).collect();
        assert!(
            named.iter().any(|n| n["role"] != "status"),
            "no text node with the sentence: {tree:#}"
        );
        assert!(named.iter().any(|n| n["role"] == "status"), "the status row keeps its label");
    }
}
