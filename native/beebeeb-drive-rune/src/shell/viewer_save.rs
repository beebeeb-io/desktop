//! The viewer's SaveCopy and DownloadTo (stub of w16-intent-routes; filled by w17-viewer-save).

use rune::Tx;

use crate::State;
use crate::frames::viewers::Intent;

/// Takes SaveCopy or DownloadTo; `false`: not taken.
pub fn take(_s: &mut State, _tx: &mut Tx, _intent: Intent) -> bool {
    false
}
