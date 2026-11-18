//! Photos routes: DownloadOriginal, StopOriginal (stub: the task that builds it edits only this file).

use rune::Tx;

use super::{Intent, Original};
use crate::State;

/// Takes the intent it can; `Err` hands it back and Photos says "Not available in this version yet".
pub fn handle(s: &mut State, _tx: &mut Tx, intent: Intent) -> Result<(), Intent> {
    // No download is started: the tab does not say one is running.
    if let (Intent::DownloadOriginal(_), Some(viewer)) = (&intent, &mut s.photos.viewer) {
        viewer.original = Original::Preview;
    }
    Err(intent)
}
