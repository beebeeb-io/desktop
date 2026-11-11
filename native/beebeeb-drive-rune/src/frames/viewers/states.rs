//! Open files: the states (DW-1516 decrypting, DW-1517 too large, DW-1518 failed). One tile, a
//! title, a sentence, and what the person can do, centred on the stage.

use rune::{Align, El, Icon, col, el, row};

use super::chrome::{ButtonKind, Dot, Status, button};
use super::source::{Body, OpenFile, percent, progress_words, size_words};
use super::{Intent, icons, style, ui};
use crate::State;
use crate::theme::tokens::Theme;
use crate::ui::kit;

/// The title's key: focus moves to it when a state replaces what was on screen.
pub const TITLE_KEY: &str = "state-title";

fn stage(content: El, t: &Theme) -> El {
    col()
        .key("viewer-body")
        .grow(1.0)
        .h_full()
        .center()
        .fill(style::stage(t))
        .child(content)
}

fn tile(icon: &'static Icon, red: bool, t: &Theme) -> El {
    let (wash, ink) = if red {
        (style::tile_red(t), style::red(t))
    } else {
        (style::tile_neutral(t), t.text_secondary)
    };
    el().key("state-tile")
        .size(52.0, 52.0)
        .radius(15.0)
        .fill(wash)
        .center()
        .child(kit::glyph("state-icon", icon, 26.0, ink))
}

fn heading(words: &str, t: &Theme) -> El {
    kit::label(TITLE_KEY, words, style::TITLE, t.text)
        .role("heading")
        .level(1)
        .focusable(true)
        .tab_index(-1)
        .no_focus_ring()
}

/// A sentence at most 30 characters of the type wide (`.kit-empty .t-sub`), centred.
fn sentence(words: String, t: &Theme) -> El {
    let width = 30.0 * rune::text::advance("0", style::SUB);
    kit::label("state-sub", words, style::SUB, t.text_secondary)
        .wrap()
        .w(width)
        .align(Align::Center)
}

/// The status line's words for a state.
pub fn status(body: &Body) -> Option<Status> {
    Some(match body {
        Body::Decrypting { done, total } => Status {
            dot: Dot::Warn,
            text: format!("Decrypting \u{00B7} {}", progress_words(*done, *total)),
            details: "Esc cancels; nothing is kept half-decrypted".into(),
        },
        Body::TooLarge { size, free } => Status {
            dot: Dot::Warn,
            text: "Not enough space to decrypt".into(),
            details: format!(
                "{} needed \u{00B7} {} free on this PC",
                size_words(*size),
                size_words(*free)
            ),
        },
        Body::Failed => Status {
            dot: Dot::Red,
            text: "Not decrypted: nothing changed".into(),
            details: "The copy in Beebeeb is unchanged".into(),
        },
    })
}

/// DW-1516.
fn decrypting(file: &OpenFile, done: u64, total: u64, t: &Theme) -> El {
    // The amount is a machine's number: mono, inside the sentence (`.t-sub .font-mono`).
    let lead = format!("{} \u{00B7}", file.name);
    let lead_w = rune::text::advance(&format!("{lead} "), style::SUB);
    let sentence = row()
        .key("state-sub-row")
        .child(kit::label("state-sub", lead, style::SUB, t.text_secondary).w(lead_w))
        .child(kit::label(
            "state-sub-mono",
            progress_words(done, total),
            style::SUB_MONO,
            t.text_secondary,
        ));
    let pct = percent(done, total);
    let content = col()
        .key("state")
        .w(300.0)
        .items_center()
        .gap(8.0)
        .child(tile(&icons::LOCK, false, t))
        .child(heading("Decrypting on this PC", t).mt(4.0))
        .child(sentence)
        .child(
            el().key("state-bar")
                .role("progressbar")
                .label(format!("Decrypting, {pct} percent"))
                .w_full()
                .h(4.0)
                .mt(4.0)
                .radius(2.0)
                .fill(style::bar_track(t))
                .child(
                    el().key("state-bar-fill")
                        .w(300.0 * pct as f32 / 100.0)
                        .h(4.0)
                        .radius(2.0)
                        .fill(t.text_secondary),
                ),
        )
        .child(
            button("state-btn-0", "Cancel", ButtonKind::Ghost, true, t).on_press(|s: &mut State| {
                if let Some(ui) = ui(s) {
                    ui.cancel();
                }
                super::close(s);
            }),
        );
    stage(content, t)
}

fn actions(buttons: Vec<El>) -> El {
    row().key("state-actions").gap(8.0).mt(8.0).children(buttons)
}

/// DW-1517.
fn too_large(file: &OpenFile, size: u64, free: u64, t: &Theme) -> El {
    let size = size_words(size);
    let free = size_words(free);
    let content = col()
        .key("state")
        .items_center()
        .gap(8.0)
        .child(tile(&icons::FILE_VIDEO, false, t))
        .child(heading("Too large to preview here", t))
        .child(sentence(
            format!(
                "{} is {size}. Opening it decrypts it on this PC, which needs that much free space; there's {free}.",
                file.name
            ),
            t,
        ))
        .child(actions(vec![
            button("state-btn-0", "Free up space\u{2026}", ButtonKind::Outline, false, t).on_press(|s: &mut State| {
                if let Some(ui) = ui(s) {
                    ui.intent(Intent::FreeUpSpace);
                }
            }),
            button("state-btn-1", "Download to\u{2026}", ButtonKind::Outline, false, t).on_press(|s: &mut State| {
                if let Some(ui) = ui(s) {
                    ui.intent(Intent::DownloadTo);
                }
            }),
        ]));
    stage(content, t)
}

/// DW-1518.
fn failed(t: &Theme) -> El {
    let content = col()
        .key("state")
        .role("alert")
        .items_center()
        .gap(8.0)
        .child(tile(&icons::ALERT, true, t))
        .child(heading("Couldn't decrypt this file", t))
        .child(sentence(
            "Nothing changed: the copy in Beebeeb is untouched. This can happen when a download is cut off.".into(),
            t,
        ))
        .child(actions(vec![
            button("state-btn-0", "Try again", ButtonKind::Outline, false, t).on_press(|s: &mut State| {
                if let Some(ui) = ui(s) {
                    ui.retry();
                }
            }),
            button("state-btn-1", "Report a problem", ButtonKind::Ghost, false, t).on_press(|s: &mut State| {
                if let Some(ui) = ui(s) {
                    ui.intent(Intent::ReportProblem);
                }
            }),
        ]));
    stage(content, t)
}

/// The body of a state.
pub fn body(file: &OpenFile, t: &Theme) -> Option<El> {
    Some(match &file.body {
        Body::Decrypting { done, total } => decrypting(file, *done, *total, t),
        Body::TooLarge { size, free } => too_large(file, *size, *free, t),
        Body::Failed => failed(t),
    })
}
