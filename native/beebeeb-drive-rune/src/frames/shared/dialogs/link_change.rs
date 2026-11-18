//! Stub (w16-shared-api): the card of Change expiry and Password on a link. Only its title and Cancel are drawn; the
//! controls, and the secure field (`Fields::link_password`), are w17-shared-links'.

use super::kit;
use super::{Dialog, dismiss};
use crate::frames::shared::parts::controls::{ButtonKind, button_md};
use crate::theme::tokens::Theme;
use rune::{El, row};

/// The card of a `LinkExpiry` or `LinkPassword` dialog.
pub(super) fn card(dialog: &Dialog, t: &Theme) -> El {
    let title = match dialog {
        Dialog::LinkPassword { has_password: true, .. } => "Change password",
        Dialog::LinkPassword { .. } => "Add a password",
        _ => "Change expiry",
    };
    let foot = row().center_y().justify_end().child(
        button_md("Cancel", ButtonKind::Outline, None, t)
            .autofocus()
            .on_press_tx(dismiss),
    );
    kit::card("link-change", 420.0, Some(kit::title(title, t)), Vec::new(), foot, t)
}
