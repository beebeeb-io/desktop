//! Shared's OpenItem, About DW-1180, leave, About DW-0511 on a link, and the stop/close confirm (stub of w16-intent-routes; filled by w17-shared-items).

use rune::Tx;

use crate::State;
use crate::frames::Intent;

/// Takes `intent` when it is one of these; `false`: not taken.
pub fn take(_s: &mut State, _tx: &mut Tx, _intent: &Intent) -> bool {
    false
}
