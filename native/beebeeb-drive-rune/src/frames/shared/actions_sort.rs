//! Shared's Sort (DW-0404) and the toolbar's search (stub of w16-intent-routes; filled by w18-shared-trash-chrome).

use rune::Tx;

use crate::State;
use crate::frames::Intent;

/// Takes `intent` when it is one of these; `false`: not taken.
pub fn take(_s: &mut State, _tx: &mut Tx, _intent: &Intent) -> bool {
    false
}
