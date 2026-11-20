//! Photos routes: DownloadOriginal, StopOriginal. The source downloads into memory and answers through `pump`
//! (`Answered::Original*`, taken in by `session::answered`); a source that cannot hands the intent back.

use rune::Tx;

use super::{Intent, Original};
use crate::State;

/// Takes the intent it can; `Err` hands it back and Photos says "Not available in this version yet".
pub fn handle(s: &mut State, _tx: &mut Tx, intent: Intent) -> Result<(), Intent> {
    match intent {
        Intent::DownloadOriginal(id) => {
            // w31: a format this PC cannot decode (HEIC, RAW) is said, and nothing is downloaded: before, the whole
            // original came down and was then refused.
            if let Some(words) = s.photos.source.original_refused(id) {
                if let Some(viewer) = &mut s.photos.viewer
                    && viewer.id == id
                {
                    viewer.original = Original::Preview;
                }
                s.photos.toast = Some(super::Toast::Words(words));
                return Ok(());
            }
            if s.photos.source.download_original(id) {
                return Ok(());
            }
            // No download is started: the tab does not say one is running.
            if let Some(viewer) = &mut s.photos.viewer
                && viewer.id == id
            {
                viewer.original = Original::Preview;
            }
            Err(intent)
        }
        Intent::StopOriginal(id) => {
            s.photos.source.stop_original(id);
            Ok(())
        }
        other => Err(other),
    }
}
