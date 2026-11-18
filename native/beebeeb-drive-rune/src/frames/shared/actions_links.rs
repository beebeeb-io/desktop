//! Shared's copy-link, change-expiry and link-password (stub of w16-intent-routes; filled by w17-shared-links).

use rune::Tx;

use crate::State;
use crate::frames::Intent;

/// Takes `intent` when it is one of these; `false`: not taken.
pub fn take(_s: &mut State, _tx: &mut Tx, _intent: &Intent) -> bool {
    false
}
