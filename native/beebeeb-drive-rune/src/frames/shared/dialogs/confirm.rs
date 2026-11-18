//! Stub (w16-shared-api): the confirmations of Leave, Stop sharing and Close request. Drawn as a plain alert with
//! the way out first in the keyboard (Esc and Enter never act); the answer is asked as an `Intent::Do` the later task
//! (w17-shared-items) takes. Its words and tile are that task's.

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
    let (tag, id, title, button): (&'static str, &str, &str, &str) = match dialog {
        Dialog::ConfirmLeave { id } => ("confirm-leave", id, "Leave this share?", "Leave"),
        Dialog::ConfirmStop { id } => ("confirm-stop", id, "Stop sharing?", "Stop sharing"),
        Dialog::ConfirmClose { id } => ("confirm-close", id, "Close this request?", "Close request"),
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
        "",
        Foot::Even(buttons),
        false,
        super::on_key,
        dismiss,
        t,
    )
}
