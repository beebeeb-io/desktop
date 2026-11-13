//! Open files: the states (DW-1516 decrypting, DW-1517 too large, DW-1518 failed). One tile, a
//! title, a sentence, and what the person can do, centred on the stage.
//!
//! Two more states are the engine's, not the board's (no golden): opening stopped for a reason that is
//! not a failed decryption ([`Stopped`]), and a file that is in Beebeeb and is not shown here
//! ([`NoPreview`]). They are drawn with the same parts as the board's three.

use rune::{Align, El, Icon, col, el, row};

use super::chrome::{ButtonKind, Dot, Status, button};
use super::source::{Body, NoPreview, OpenFile, Stopped, percent, progress_words, size_words};
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
        Body::Stopped(why) => Status {
            dot: Dot::Warn,
            text: match why {
                Stopped::Offline => "Not opened: Beebeeb can't be reached",
                Stopped::SessionEnded => "Not opened: the session ended",
                Stopped::SessionChanged => "Closed: its content left memory",
                Stopped::Gone => "Not opened: the file is gone",
            }
            .into(),
            details: "The copy in Beebeeb is unchanged".into(),
        },
        Body::NotPreviewable(why) => Status {
            dot: Dot::Warn,
            text: "No preview here".into(),
            details: match why {
                NoPreview::NoViewer | NoPreview::TooBig { .. } => "Nothing was downloaded",
                NoPreview::Unreadable => "Decrypted in memory only; nothing was kept",
            }
            .into(),
        },
        // Each has its own view (`text.rs`, `markdown.rs`, `photo.rs`, …).
        _ => return None,
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
        // Closing cancels: nothing is kept half-decrypted.
        .child(button("state-btn-0", "Cancel", ButtonKind::Ghost, true, t).on_press(super::close));
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

/// Opening stopped, and it was not a failed decryption: what happened, and the one thing to do.
fn stopped(why: Stopped, t: &Theme) -> El {
    let (title, words) = match why {
        Stopped::Offline => (
            "Can't reach Beebeeb",
            "Opening downloads the file, and Beebeeb can't be reached right now. Nothing changed.",
        ),
        Stopped::SessionEnded => (
            "Your session ended",
            "Sign in again to open this file. Nothing changed.",
        ),
        Stopped::SessionChanged => (
            "This file was closed",
            "Beebeeb was locked or the account changed while it was open, so its content was cleared from memory.",
        ),
        Stopped::Gone => (
            "This file is no longer in Beebeeb",
            "It may have been moved to Trash or deleted on another device.",
        ),
    };
    let act = if why.can_retry() {
        let label = if why == Stopped::Offline {
            "Try again"
        } else {
            "Open it again"
        };
        button("state-btn-0", label, ButtonKind::Outline, false, t).on_press(|s: &mut State| {
            if let Some(ui) = ui(s) {
                ui.retry();
            }
        })
    } else {
        button("state-btn-0", "Close", ButtonKind::Outline, false, t).on_press(super::close)
    };
    let content = col()
        .key("state")
        .role("alert")
        .items_center()
        .gap(8.0)
        .child(tile(&icons::ALERT, false, t))
        .child(heading(title, t))
        .child(sentence(words.into(), t))
        .child(actions(vec![act]));
    stage(content, t)
}

/// The file is in Beebeeb and is not shown here: why, and the two ways out.
fn not_previewable(file: &OpenFile, why: NoPreview, t: &Theme) -> El {
    let (title, words) = match why {
        NoPreview::NoViewer => (
            "No preview for this kind of file yet",
            format!(
                "{} is safe in Beebeeb. This app can't show this kind of file yet, so nothing was downloaded.",
                file.name
            ),
        ),
        NoPreview::TooBig { size, limit } => (
            "Too large to preview here",
            format!(
                "{} is {}. A preview is decrypted in memory, up to {}, so nothing was downloaded.",
                file.name,
                crate::source::format_size(i64::try_from(size).unwrap_or(i64::MAX)),
                crate::source::format_size(i64::try_from(limit).unwrap_or(i64::MAX))
            ),
        ),
        NoPreview::Unreadable => (
            "Can't show this file",
            "It was decrypted on this PC, but its content isn't what its name says. Nothing was kept.".to_string(),
        ),
    };
    let content = col()
        .key("state")
        .items_center()
        .gap(8.0)
        .child(tile(&icons::INFO, false, t))
        .child(heading(title, t))
        .child(sentence(words, t))
        .child(actions(vec![
            button(
                "state-btn-0",
                "Open in another app\u{2026}",
                ButtonKind::Outline,
                false,
                t,
            )
            .on_press(|s: &mut State| {
                if let Some(ui) = ui(s) {
                    ui.intent(Intent::OpenInAnotherApp);
                }
            }),
            button("state-btn-1", "Save a copy\u{2026}", ButtonKind::Outline, false, t).on_press(|s: &mut State| {
                if let Some(ui) = ui(s) {
                    ui.intent(Intent::SaveCopy);
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
        Body::Stopped(why) => stopped(*why, t),
        Body::NotPreviewable(why) => not_previewable(file, *why, t),
        // Each has its own view (`text.rs`, `markdown.rs`, `photo.rs`, …).
        _ => return None,
    })
}
