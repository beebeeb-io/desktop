//! What the open viewer asked of the shell (`frames::viewers::Intent`), taken by the poll (stub of w16-intent-routes;
//! filled by w17-viewer-actions). SaveCopy and DownloadTo go to `viewer_save`.

use rune::Tx;

use crate::State;
use crate::frames::viewers::Intent;

/// Takes `intent`; `false`: nothing takes it yet (the router then shows "Not built yet: <what>" where it has a name).
pub fn take(s: &mut State, tx: &mut Tx, intent: Intent) -> bool {
    match intent {
        Intent::SaveCopy | Intent::DownloadTo => super::viewer_save::take(s, tx, intent),
        _ => false,
    }
}
