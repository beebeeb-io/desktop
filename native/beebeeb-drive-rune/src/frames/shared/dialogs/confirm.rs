//! The confirmations of Leave, Stop sharing and Close request. Drawn as a plain alert with the way out first in the
//! keyboard (Esc and Enter never act); the answer is asked as `Intent::Do("confirm-leave" | "confirm-stop" |
//! "confirm-close", id)`, which `actions_items` takes (only for the confirmation open for that id).

use super::kit::{self, Foot};
use super::{Dialog, ask, dismiss};
use crate::State;
use crate::frames::Intent;
use crate::frames::shared::parts::controls::{ButtonKind, button_md};
use crate::theme::icons_settings as ic;
use crate::theme::tokens::Theme;
use crate::ui::settings_notice::{Tone, tile_sized};
use rune::{El, el};

/// The alert of a `Confirm*` dialog.
pub(super) fn alert(dialog: &Dialog, t: &Theme) -> El {
    let (tag, id, title, body, button): (&'static str, &str, &str, &str, &str) = match dialog {
        Dialog::ConfirmLeave { id } => (
            "confirm-leave",
            id,
            "Leave this share?",
            "It leaves your Shared list and you can no longer open it. Whoever shared it can invite you again.",
            "Leave",
        ),
        Dialog::ConfirmStop { id } => (
            "confirm-stop",
            id,
            "Stop sharing?",
            "Nobody can open it through this share anymore. Copies people already saved stay with them.",
            "Stop sharing",
        ),
        Dialog::ConfirmClose { id } => (
            "confirm-close",
            id,
            "Close this request?",
            "No new files can be sent through it. What has arrived stays.",
            "Close request",
        ),
        _ => return el(),
    };
    let id = id.to_string();
    let buttons = vec![
        button_md(button, ButtonKind::Danger, None, t)
            .on_press(move |s: &mut State| ask(s, Intent::Do(tag, id.clone()))),
        button_md("Cancel", ButtonKind::Ghost, None, t)
            .autofocus()
            .on_press_tx(dismiss),
    ];
    kit::alert(
        tag,
        380.0,
        tile_sized(0, Tone::Red, &ic::SHIELD, 36.0, t),
        title,
        body,
        Foot::Even(buttons),
        false,
        super::on_key,
        dismiss,
        t,
    )
}
