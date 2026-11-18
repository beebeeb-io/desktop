//! Shared's New request (DW-0504), create-request and New (DW-0414) (stub of w16-intent-routes; filled by w17-shared-requests).

use rune::Tx;

use crate::State;
use crate::frames::Intent;

/// Takes `intent` when it is one of these; `false`: not taken.
pub fn take(_s: &mut State, _tx: &mut Tx, _intent: &Intent) -> bool {
    false
}
