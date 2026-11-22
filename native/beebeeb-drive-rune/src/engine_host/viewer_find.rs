//! Find in a PDF (DW-1506), from the PDF's own text. Part of [`EngineViewerSource`]
//! (`viewer.rs`), which hands `finds` and `find` here.
//!
//! The text is read on the worker that holds the document (`viewer_pdf_find.rs`), asked down the channel the pages
//! are, and answered under a serial (`viewer_pages.rs`). The frame asks once when the words change and wants the
//! answer then, so `find` waits for it, at most [`FIND_WAIT`]; a worker that is still busy past that gives
//! nothing (the frame shows no matches: the limit is named in the pull request). What is found belongs to the
//! opening it was asked for: the opening's end drops the channels, and an answer that comes after it has nowhere to go.
//! Must not: log the words asked for.

use std::time::Duration;

use super::*;

/// How long `find` waits for the worker (a page it is rendering is finished first; a very long PDF's text takes a while).
pub(super) const FIND_WAIT: Duration = Duration::from_secs(8);

/// Only a PDF has pages held (a text, a picture, a table has none): only a PDF can be searched.
fn pages_search(pages: Option<&Pages>) -> bool {
    pages.is_some()
}

impl<E: ViewerEngine> EngineViewerSource<E> {
    /// The open file is a PDF whose pages are held, for the session it was opened under.
    pub(super) fn viewer_find_finds(&self) -> bool {
        self.open.as_ref().is_some_and(|opening| {
            !opening.ended && pages_search(opening.pages.as_ref()) && Self::is_current(&self.engine, opening)
        })
    }

    pub(super) fn viewer_find_find(
        &mut self,
        file_id: &str,
        words: &str,
    ) -> Vec<crate::frames::viewers::source::Match> {
        let Some(pages) = self.current(file_id).and_then(|opening| opening.pages.as_ref()) else {
            return Vec::new();
        };
        // The worker has not opened the document yet (the view was not told): there is no text to look in.
        if pages.doc.borrow().is_none() {
            return Vec::new();
        }
        pages.find(words, FIND_WAIT).unwrap_or_default()
    }
}

#[cfg(test)]
#[path = "viewer_find_tests.rs"]
mod find_tests;
