//! A video in Beebeeb's player (decision D-MED-6: a video opened from a list plays here; the board has no frame for
//! it yet, one is requested; decision D-MED-4: DW-0611's player bar, drawn by the toolkit in the board's geometry and
//! words, since a toolkit window cannot host Windows' own player).
//!
//! Slice M6 (`media-surface`) fills it: the stage (black, the frame scaled to fit, letterboxed), the bar under it
//! (play/pause, elapsed, the scrubber, remaining; times in JetBrains Mono), and the stage's size in device pixels to
//! the player (`PlayerHandle::set_view_size`). Until then a video draws what a kind without a viewer draws ("No
//! preview for this kind of file yet", nothing downloaded), and nothing makes a `Body::Video` before the player is
//! bound (M7).

use rune::{El, el};

use super::chrome::{Dot, Status};
use super::source::{Body, NoPreview};
use super::{Ui, states};
use crate::theme::tokens::Theme;

/// The video's stage and bar.
pub fn view(ui: &Ui, t: &Theme) -> El {
    // The state of a kind without a viewer, for this file. The copy's name is overwritten when it is drawn.
    let mut shown = ui.file.clone();
    shown.body = Body::NotPreviewable(NoPreview::NoViewer);
    let drawn = states::body(&shown, ui.saves_copies(), ui.origin(), t).unwrap_or_else(el);
    shown.wipe();
    drawn
}

/// The status line under the video.
pub fn status(ui: &Ui) -> Status {
    states::status(&Body::NotPreviewable(NoPreview::NoViewer), ui.origin()).unwrap_or(Status {
        dot: Dot::Warn,
        text: String::new(),
        details: String::new(),
    })
}
