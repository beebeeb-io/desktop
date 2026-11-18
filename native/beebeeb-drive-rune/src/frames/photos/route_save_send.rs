//! Photos routes: SaveToPc, SendCopy (stub: the task that builds it edits only this file).

use rune::Tx;

use super::Intent;
use crate::State;

/// Takes the intent it can; `Err` hands it back and Photos says "Not available in this version yet".
pub fn handle(_s: &mut State, _tx: &mut Tx, intent: Intent) -> Result<(), Intent> {
    Err(intent)
}
