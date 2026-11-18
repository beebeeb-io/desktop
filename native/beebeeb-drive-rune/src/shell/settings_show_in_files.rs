//! Settings' `ShowInFiles(id)` (stub of w16-intent-routes; filled by w18-insights-reveal).

use rune::Tx;

use super::host::log;
use crate::State;

pub fn show(_s: &mut State, _tx: &mut Tx, _id: &str) {
    log("NotWired(ShowInFiles)");
}
