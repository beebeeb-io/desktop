//! What this section's frames asked of the shell: Sort and the toolbar's search (`Intent::Frame("DW-0404")`,
//! `Intent::Do("search", ..)`). Nothing takes them yet (stub of w16-intent-routes; filled by w18).

use rune::Tx;

use crate::State;
use crate::frames::Intent;

/// Takes `intent` when it is this section's; `false`: not taken (the router logs it as `NotWired`).
pub fn take(_s: &mut State, _tx: &mut Tx, _intent: &Intent) -> bool {
    false
}
