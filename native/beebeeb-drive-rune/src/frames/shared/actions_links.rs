//! Shared's copy-link, change-expiry and link-password (w17-shared-links).
//!
//! Copy link asks the source once (`copy_row_link`): the link reaches the clipboard through the source's `take_link`
//! and a `Tx`, never through this file. Change expiry and Password… open their dialog (`dialogs::link_change`); the
//! dialog sends the change. Nothing is said to have happened before the source answered.

use rune::Tx;

use super::dialogs::Dialog;
use super::source::Notice;
use crate::State;
use crate::frames::Intent;

/// Takes `intent` when it is one of these; `false`: not taken.
pub fn take(s: &mut State, _tx: &mut Tx, intent: &Intent) -> bool {
    let ui = &mut s.frames.shared;
    match intent {
        Intent::Do("copy-link", id) => {
            if !ui.source.copy_row_link(id) && ui.notice.is_none() {
                ui.notice = Some(Notice::NotAvailable);
            }
            true
        }
        Intent::Do("change-expiry", id) => {
            ui.open_dialog(Dialog::LinkExpiry { share_id: id.clone() });
            true
        }
        Intent::Do("link-password", id) => {
            ui.open_dialog(Dialog::LinkPassword {
                share_id: id.clone(),
                has_password: false,
            });
            true
        }
        _ => false,
    }
}
