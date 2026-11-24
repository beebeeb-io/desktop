//! Open files: the states (DW-1516 decrypting, DW-1517 too large, DW-1518 failed). One tile, a
//! title, a sentence, and what the person can do, centred on the stage.
//!
//! Two more states are the engine's, not the board's (no golden): opening stopped for a reason that is
//! not a failed decryption ([`Stopped`]), and a file that is in Beebeeb and is not shown here
//! ([`NoPreview`]). They are drawn with the same parts as the board's three.

use rune::{col, row, El, Icon};

use super::chrome::{Dot, Status};
use super::source::{percent, progress_words, size_words, Body, NoPreview, OpenFile, Origin, Stopped};
use super::{icons, style, ui, Intent};
use crate::theme::scale;
use crate::theme::tokens::Theme;
use crate::ui::button::{button, keys, ButtonState, Size, Variant};
use crate::ui::view_state::{self, view_state, ViewState};
use crate::State;

/// The key focus moves to when a state replaces what was on screen: the state's first button (`ui::view_state`
/// titles are not focus stops).
pub const TITLE_KEY: &str = "state-btn-0";

fn stage(content: El, t: &Theme) -> El {
    col()
        .key("viewer-body")
        .grow(1.0)
        .h_full()
        .center()
        .fill(style::stage(t))
        .child(content)
}

/// The button row under a state's words.
fn actions(buttons: Vec<El>) -> El {
    row().key("state-actions").gap(scale::space::S8).children(buttons)
}

/// A state's words and actions through `ui::view_state`, centred on the stage.
fn empty_view(icon: &'static Icon, title: &str, body: &str, buttons: Vec<El>, t: &Theme) -> El {
    stage(
        view_state(
            ViewState::Empty {
                key: "state",
                icon,
                title,
                body,
                action: (!buttons.is_empty()).then(|| actions(buttons)),
            },
            t,
        ),
        t,
    )
}

fn failed_view(title: &str, why: &str, buttons: Vec<El>, t: &Theme) -> El {
    stage(
        view_state(
            ViewState::Failed {
                key: "state",
                title,
                why,
                retry: (!buttons.is_empty()).then(|| actions(buttons)),
            },
            t,
        ),
        t,
    )
}

/// A file with nothing in it (zero bytes, an archive with no entries): said, not drawn as a blank page.
pub fn empty_file(name: &str, t: &Theme) -> El {
    empty_view(
        &icons::INFO,
        "This file is empty",
        &format!("{name} has nothing in it, so there is nothing to show. Open it in another app to add something."),
        vec![button(
            keys("state-btn-0"),
            "Open in another app\u{2026}",
            None,
            Variant::Secondary,
            Size::Md,
            ButtonState::Enabled,
            t,
        )
        .on_press(|s: &mut State| {
            if let Some(ui) = ui(s) {
                ui.intent(Intent::OpenInAnotherApp);
            }
        })],
        t,
    )
}

/// The status line's right side for bytes that were decrypted, looked at, and dropped.
pub const DECRYPTED_NOT_KEPT: &str = "Decrypted in memory only; nothing was kept";
/// The same for bytes read from the copy on this PC: nothing was decrypted.
pub const READ_HERE_NOT_KEPT: &str = concat!(
    "Read from the copy on ",
    beebeeb_engine::device_word!(),
    "; nothing was kept"
);

/// The same for a PDF that arrived whole and could not be read as a PDF.
pub const PDF_NOT_KEPT: &str = "Held in memory only; nothing was kept";

/// Whether content from `origin` was decrypted on this PC just now (downloaded from Beebeeb). The copy kept on
/// this PC was read as it is: nothing was decrypted, and no state says so.
fn was_decrypted(origin: Origin) -> bool {
    match origin {
        Origin::Beebeeb | Origin::BeebeebNotThisPcs | Origin::BeebeebUnconfirmedHere => true,
        Origin::ThisPc { .. } | Origin::ThisPcSyncOff => false,
    }
}

/// The status line's words for a state. `origin`: where the bytes a state speaks of were read from.
pub fn status(body: &Body, origin: Origin) -> Option<Status> {
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
                concat!("{} needed \u{00B7} {} free on ", beebeeb_engine::device_word!()),
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
                Stopped::NotNow => "Not opened: couldn't read it right now",
                Stopped::CopyNotReadable => concat!(
                    "Not opened: the copy on ",
                    beebeeb_engine::device_word!(),
                    " couldn't be read"
                ),
                Stopped::NothingSynced => concat!(
                    "Not opened: nothing has synced on ",
                    beebeeb_engine::device_word!(),
                    " yet"
                ),
                Stopped::NotThisAccounts => concat!(
                    "Not opened: ",
                    beebeeb_engine::device_word!(),
                    "'s data couldn't be confirmed as this account's"
                ),
                Stopped::NoAccess => "Not opened: you no longer have access to this file",
                Stopped::Changed => "Not opened: it changed while it was opening",
                Stopped::HelperStopped => "Not shown: the viewer's helper stopped",
            }
            .into(),
            details: "The copy in Beebeeb is unchanged".into(),
        },
        Body::NotPreviewable(why) => Status {
            dot: Dot::Warn,
            text: "No preview here".into(),
            details: match why {
                NoPreview::NoViewer | NoPreview::Heic | NoPreview::TooBig { .. } => "Nothing was downloaded",
                NoPreview::OverLimit { .. } => "Stopped; nothing was kept",
                // The file arrived whole and was read as it is: the trouble is the PDF's own content, and the line
                // does not speak of decrypting (RC2-3).
                NoPreview::PdfUnreadable if was_decrypted(origin) => PDF_NOT_KEPT,
                NoPreview::Unreadable
                | NoPreview::PictureTooBig { .. }
                | NoPreview::PdfLocked
                | NoPreview::PdfUnreadable
                | NoPreview::PdfTooLong { .. } => {
                    if was_decrypted(origin) {
                        DECRYPTED_NOT_KEPT
                    } else {
                        READ_HERE_NOT_KEPT
                    }
                }
            }
            .into(),
        },
        // Each has its own view (`text.rs`, `markdown.rs`, `photo.rs`, …).
        _ => return None,
    })
}

/// DW-1516: the loading strip with the amount, and Cancel.
fn decrypting(file: &OpenFile, done: u64, total: u64, t: &Theme) -> El {
    let words = format!(
        concat!(
            "Decrypting {} on ",
            beebeeb_engine::device_word!(),
            " \u{00B7} {} ({} percent)"
        ),
        file.name,
        progress_words(done, total),
        percent(done, total)
    );
    let content = col()
        .key("state")
        .w(300.0)
        .items_center()
        .gap(scale::space::S8)
        .child(view_state(ViewState::Loading { words: &words }, t))
        // Closing cancels: nothing is kept half-decrypted.
        .child(
            button(
                keys("state-btn-0"),
                "Cancel",
                None,
                Variant::Ghost,
                Size::Sm,
                ButtonState::Enabled,
                t,
            )
            .on_press(super::close),
        );
    stage(content, t)
}

/// DW-1517.
fn too_large(file: &OpenFile, size: u64, free: u64, t: &Theme) -> El {
    let size = size_words(size);
    let free = size_words(free);
    empty_view(
        &icons::FILE_VIDEO,
        "Too large to preview here",
        &format!(
            concat!(
                "{} is {size}. Opening it decrypts it on ",
                beebeeb_engine::device_word!(),
                ", which needs that much free space; there's {free}."
            ),
            file.name
        ),
        vec![
            button(
                keys("state-btn-0"),
                "Free up space\u{2026}",
                None,
                Variant::Secondary,
                Size::Md,
                ButtonState::Enabled,
                t,
            )
            .on_press(|s: &mut State| {
                if let Some(ui) = ui(s) {
                    ui.intent(Intent::FreeUpSpace);
                }
            }),
            button(
                keys("state-btn-1"),
                "Download to\u{2026}",
                None,
                Variant::Secondary,
                Size::Md,
                ButtonState::Enabled,
                t,
            )
            .on_press(|s: &mut State| {
                if let Some(ui) = ui(s) {
                    ui.intent(Intent::DownloadTo);
                }
            }),
        ],
        t,
    )
}

/// Try again / Open it again: asked of the source only while the shell's session gate is open (a window still
/// on screen when its session ended does not act; the refusal is counted).
fn retry(s: &mut State) {
    if crate::shell::session_gate::refuse(s) {
        return;
    }
    if let Some(ui) = ui(s) {
        ui.retry();
    }
}

/// DW-1518.
fn failed(t: &Theme) -> El {
    failed_view(
        "Couldn't decrypt this file",
        "Nothing changed: the copy in Beebeeb is untouched. This can happen when a download is cut off.",
        vec![
            button(
                keys("state-btn-0"),
                view_state::TRY_AGAIN,
                None,
                Variant::Secondary,
                Size::Md,
                ButtonState::Enabled,
                t,
            )
            .on_press(retry),
            button(
                keys("state-btn-1"),
                "Report a problem",
                None,
                Variant::Ghost,
                Size::Md,
                ButtonState::Enabled,
                t,
            )
            .on_press(|s: &mut State| {
                if let Some(ui) = ui(s) {
                    ui.intent(Intent::ReportProblem);
                }
            }),
        ],
        t,
    )
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
            "This file is no longer here",
            "It may have been moved to Trash or deleted on another device.",
        ),
        Stopped::CopyNotReadable => (
            concat!("Couldn't read the copy on ", beebeeb_engine::device_word!()),
            concat!(
                beebeeb_engine::device_word_cap!(),
                " has changes to this file that aren't in Beebeeb yet, and its copy couldn't be read just now: \
             another app may have it open. The older version in Beebeeb isn't shown in its place."
            ),
        ),
        Stopped::NotNow => (
            "Couldn't open this file right now",
            concat!(
                "What ",
                beebeeb_engine::device_word!(),
                " knows about the file couldn't be read at this moment. Nothing changed."
            ),
        ),
        Stopped::NothingSynced => (
            concat!("Nothing has synced on ", beebeeb_engine::device_word!(), " yet"),
            concat!(
                beebeeb_engine::device_word_cap!(),
                " doesn't know your files yet, so it can't open this one. The file in Beebeeb is unchanged. \
             Try again once the first sync has run."
            ),
        ),
        Stopped::NotThisAccounts => (
            concat!(
                "Couldn't confirm ",
                beebeeb_engine::device_word!(),
                "'s data is this account's"
            ),
            concat!(
                "What ",
                beebeeb_engine::device_word!(),
                " holds of Beebeeb couldn't be confirmed as this account's, so nothing of it was read. \
             This says nothing about the file: the copy in Beebeeb is unchanged."
            ),
        ),
        Stopped::NoAccess => (
            "You no longer have access to this file",
            "Beebeeb refused it for this account: it may no longer be shared with you. Nothing was downloaded.",
        ),
        Stopped::Changed => (
            "This file changed while it was opening",
            "A newer version arrived each time it was read, so nothing of it was kept. Try again.",
        ),
        Stopped::HelperStopped => (
            "The viewer stopped while showing this file",
            "The file arrived whole. The separate helper that draws it stopped, so there is nothing to show. \
             Nothing changed: the copy in Beebeeb is untouched. Try again, or open it in another app.",
        ),
    };
    let act = if why.can_retry() {
        let label = if why == Stopped::SessionChanged {
            "Open it again"
        } else {
            "Try again"
        };
        button(
            keys("state-btn-0"),
            label,
            None,
            Variant::Secondary,
            Size::Md,
            ButtonState::Enabled,
            t,
        )
        .on_press(retry)
    } else {
        button(
            keys("state-btn-0"),
            "Close",
            None,
            Variant::Secondary,
            Size::Md,
            ButtonState::Enabled,
            t,
        )
        .on_press(super::close)
    };
    failed_view(title, words, vec![act], t)
}

/// The file is in Beebeeb and is not shown here: why, and the ways out. `copies`: "Save a copy…" has somewhere to
/// go (`ViewerSource::saves_copies`); a button with no target is not drawn.
fn not_previewable(file: &OpenFile, why: NoPreview, copies: bool, origin: Origin, t: &Theme) -> El {
    // What happened to the bytes before this app looked at them: decrypted, or read from the copy on this PC.
    let it_was = if was_decrypted(origin) {
        concat!("It was decrypted on ", beebeeb_engine::device_word!())
    } else {
        concat!("It was read from the copy on ", beebeeb_engine::device_word!())
    };
    let (title, words) = match why {
        NoPreview::NoViewer => (
            "No preview for this kind of file yet",
            format!(
                "{} is safe in Beebeeb. This app can't show this kind of file yet, so nothing was downloaded.",
                file.name
            ),
        ),
        NoPreview::Heic => (
            "HEIC photos can\u{2019}t be shown here yet",
            format!(
                concat!(
                    "{} is safe in Beebeeb. HEIC photos can\u{2019}t be shown at full size on ",
                    beebeeb_engine::device_word!(),
                    " yet, so nothing was \
                 downloaded. Open it in your default app."
                ),
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
        // No size is said: none is known, and what had arrived is gone.
        NoPreview::OverLimit { limit } => (
            "Too large to preview here",
            format!(
                "{} turned out to be larger than {}, the most a preview decrypts in memory. Opening it was \
                 stopped, and nothing was kept.",
                file.name,
                crate::source::format_size(i64::try_from(limit).unwrap_or(i64::MAX))
            ),
        ),
        NoPreview::Unreadable => (
            "Can't show this file",
            format!(
                "{it_was}, but this app can't read its content: text that isn't UTF-8, or a picture it can't \
                 decode. Nothing was kept."
            ),
        ),
        // Its own words, and none about decrypting: the file arrived whole, and it is the PDF that is not readable.
        NoPreview::PdfUnreadable => (
            "Can't read this PDF",
            format!(
                "{}, but this app can't read it: it may be damaged or cut short, or protected in a way this \
                 app doesn't open. Open it in your PDF app. Nothing was kept.",
                if was_decrypted(origin) {
                    "It arrived whole from Beebeeb"
                } else {
                    it_was
                }
            ),
        ),
        NoPreview::PdfLocked => (
            "This PDF asks for a password",
            "It is protected with a password of its own, and this app can't ask for it yet. Open it in your PDF app. \
             Nothing was kept."
                .to_string(),
        ),
        NoPreview::PdfTooLong { pages } => (
            "Too long to preview here",
            format!(
                "This PDF has {pages} pages, more than a preview lists. Open it in your PDF app. Nothing was kept."
            ),
        ),
        NoPreview::PictureTooBig { width, height } => (
            "Too large to preview here",
            format!(
                "This picture is {width} \u{00D7} {height} pixels, more than a preview decodes in memory. It was \
                 not decoded, and nothing was kept."
            ),
        ),
    };
    let mut buttons = vec![button(
        keys("state-btn-0"),
        &heic_button(why),
        None,
        Variant::Secondary,
        Size::Md,
        ButtonState::Enabled,
        t,
    )
    .on_press(|s: &mut State| {
        if let Some(ui) = ui(s) {
            ui.intent(Intent::OpenInAnotherApp);
        }
    })];
    if copies {
        buttons.push(
            button(
                keys("state-btn-1"),
                "Save a copy\u{2026}",
                None,
                Variant::Secondary,
                Size::Md,
                ButtonState::Enabled,
                t,
            )
            .on_press(|s: &mut State| {
                if let Some(ui) = ui(s) {
                    ui.intent(Intent::SaveCopy);
                }
            }),
        );
    }
    empty_view(&icons::INFO, title, &words, buttons, t)
}

/// The first button of the no-preview state: a HEIC names the system's app, as Photos does.
fn heic_button(why: NoPreview) -> String {
    match why {
        NoPreview::Heic => format!("Open in {}", crate::frames::show_in_folder::os_app_label()),
        _ => "Open in another app\u{2026}".to_string(),
    }
}

/// The body of a state. `copies`: see [`not_previewable`]. `origin`: where the bytes a state speaks of were
/// read from.
pub fn body(file: &OpenFile, copies: bool, origin: Origin, t: &Theme) -> Option<El> {
    Some(match &file.body {
        Body::Decrypting { done, total } => decrypting(file, *done, *total, t),
        Body::TooLarge { size, free } => too_large(file, *size, *free, t),
        Body::Failed => failed(t),
        Body::Stopped(why) => stopped(*why, t),
        Body::NotPreviewable(why) => not_previewable(file, *why, copies, origin, t),
        // Each has its own view (`text.rs`, `markdown.rs`, `photo.rs`, …).
        _ => return None,
    })
}

/// CP2 row 10.3 (L266.4): a 403 and a version that kept changing have their own words, never "Not decrypted".
#[cfg(test)]
mod refusal_words_tests {
    use super::*;

    fn words(why: Stopped) -> String {
        status(&Body::Stopped(why), Origin::Beebeeb)
            .map(|s| s.text)
            .unwrap_or_default()
    }

    #[test]
    fn no_access_and_changed_say_what_happened() {
        assert_eq!(
            words(Stopped::NoAccess),
            "Not opened: you no longer have access to this file"
        );
        assert_eq!(words(Stopped::Changed), "Not opened: it changed while it was opening");
        let failed = status(&Body::Failed, Origin::Beebeeb)
            .map(|s| s.text)
            .unwrap_or_default();
        assert_ne!(words(Stopped::NoAccess), failed);
        assert_ne!(words(Stopped::Changed), failed);
    }

    /// RC2-3: a PDF that is not readable, a failed decryption and a stopped helper are three sentences, and only the
    /// failed decryption speaks of decrypting.
    #[test]
    fn viewer_pdf_failures_each_have_their_own_words() {
        let line = |body: Body| {
            status(&body, Origin::Beebeeb)
                .map(|s| format!("{} {}", s.text, s.details))
                .unwrap_or_default()
        };
        let decrypt = line(Body::Failed);
        let helper = line(Body::Stopped(Stopped::HelperStopped));
        let pdf = line(Body::NotPreviewable(NoPreview::PdfUnreadable));
        assert!(decrypt.contains("Not decrypted"), "{decrypt}");
        assert!(helper.contains("helper stopped"), "{helper}");
        for other in [&helper, &pdf] {
            assert!(!other.to_lowercase().contains("decrypt"), "{other}");
        }
        assert_ne!(helper, pdf);
        assert!(Stopped::HelperStopped.can_retry(), "Try again starts a new helper");
    }
}
