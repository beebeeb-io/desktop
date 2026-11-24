//! What this section's frames asked of the shell: Sort and the toolbar's search (`Intent::Frame("DW-0404")`,
//! `Intent::Do("search", ..)`), as `shared::actions_sort` has them for Shared.

use rune::Tx;

use crate::State;
use crate::frames::Intent;
use crate::frames::shared::actions_sort::SORT_BUTTON;
use crate::frames::shared::parts::menu::Open;

/// Takes `intent` when it is this section's; `false`: not taken (the router logs it as `NotWired`).
pub fn take(s: &mut State, _tx: &mut Tx, intent: &Intent) -> bool {
    let ui = &mut s.frames.trash;
    match intent {
        Intent::Frame("DW-0404") => {
            ui.menu = Some(Open::under(SORT_BUTTON));
            true
        }
        Intent::Do("search", typed) => {
            ui.sort.filter = typed.clone();
            ui.selected = None;
            ui.selected_item = None;
            true
        }
        _ => false,
    }
}
