//! Slice B2 (plan40 batch 1): Find in a PDF (DW-1506), from the PDF's own text. Part of [`EngineViewerSource`]
//! (`viewer.rs`), which hands `finds` and `find` here through `not_wired(…)`.
//!
//! Not bound yet: the adapter's body and the trait's default of before the scaffold, moved unchanged. The text of a
//! PDF is not searched here, and a search finds nothing. Session-bound like its neighbours once bound: what is found
//! belongs to the opening it was asked for, and is dropped when that opening ends.

use super::*;

impl<E: ViewerEngine> EngineViewerSource<E> {
    pub(super) fn viewer_find_finds(&self) -> bool {
        false
    }

    pub(super) fn viewer_find_find(
        &mut self,
        _file_id: &str,
        _words: &str,
    ) -> Vec<crate::frames::viewers::source::Match> {
        Vec::new()
    }
}
