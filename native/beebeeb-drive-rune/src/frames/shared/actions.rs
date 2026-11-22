//! Routes each intent Shared's frames queued to the file that owns it. Every file answers `false` until its task
//! (w17-shared-links, w17-shared-items, w17-shared-requests, w18-shared-trash-chrome) fills it in.

use rune::Tx;

use crate::State;
use crate::frames::Intent;

/// Takes `intent` when one of the files below does; `false`: not taken (the router logs it as `NotWired`).
pub fn take(s: &mut State, tx: &mut Tx, intent: &Intent) -> bool {
    match intent {
        Intent::Do("copy-link" | "change-expiry" | "link-password", _) => super::actions_links::take(s, tx, intent),
        Intent::OpenItem(_)
        | Intent::About("DW-1180" | "DW-0511", _)
        | Intent::Do("confirm-leave" | "confirm-stop" | "confirm-close", _) => {
            super::actions_items::take(s, tx, intent)
        }
        Intent::Frame("DW-0504" | "DW-0414") | Intent::Do("create-request", _) => {
            super::actions_new::take(s, tx, intent)
        }
        Intent::Frame("DW-0404") | Intent::Do("search", _) => super::actions_sort::take(s, tx, intent),
        _ => false,
    }
}
