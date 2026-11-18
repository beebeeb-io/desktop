//! Settings' Open targets file-explorer, support-bundle and choose-photo (stub of w16-intent-routes; filled by w17-settings-places).

use rune::Tx;

use crate::State;

/// Opens `target`; `false`: not taken.
pub fn open(_s: &mut State, _tx: &mut Tx, _target: &str) -> bool {
    false
}
