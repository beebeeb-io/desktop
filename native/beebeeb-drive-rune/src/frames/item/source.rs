//! What the read-only Preview of an item that is not in Files' list reads ([`ItemPreviewSource`]): a file in Trash
//! (from Beebeeb only) or one shared with the person (this PC's copy first). The running app's source is
//! `engine_host::item_preview::EngineItemPreview`; a window without an engine has [`Unwired`], which shows nothing;
//! the scenes and tests have [`BoardPreview`].

use std::cell::RefCell;
use std::rc::Rc;

use crate::frames::files::actions_data::Previewed;
use crate::frames::files::source::ActionError;

/// Where a preview reads the file from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadFrom {
    /// Downloaded from Beebeeb and decrypted in memory; nothing on this PC is read (a file in Trash).
    Beebeeb,
    /// The copy wholly on this PC when there is one, else downloaded (a file shared with the person).
    ThisPcFirst,
}

/// One preview to start: the file, its name (for the page's title and what decides its kind), where to read it.
#[derive(Clone, PartialEq, Eq)]
pub struct PreviewAsk {
    pub id: String,
    pub name: String,
    pub from: ReadFrom,
}

/// Neither the name nor the id in a `{:?}`.
impl std::fmt::Debug for PreviewAsk {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreviewAsk")
            .field("from", &self.from)
            .finish_non_exhaustive()
    }
}

/// ONE preview at a time. `start` stops the one before it (what it read is wiped); `later` is asked every frame
/// while the panel waits (`Shown` is handed over once); `cancel` stops it and wipes what it read, and is a no-op
/// when nothing was asked.
pub trait ItemPreviewSource {
    fn start(&mut self, ask: &PreviewAsk);
    /// Where the preview of `id` stands. Any other id than the one started: `Failed(Unavailable)`.
    fn later(&mut self, id: &str) -> Previewed;
    fn cancel(&mut self);
}

/// A window without an engine: nothing is read and nothing is shown, never a sample.
pub struct Unwired;

impl ItemPreviewSource for Unwired {
    fn start(&mut self, _ask: &PreviewAsk) {}
    fn later(&mut self, _id: &str) -> Previewed {
        Previewed::Failed(ActionError::Unavailable)
    }
    fn cancel(&mut self) {}
}

/// What [`BoardPreview`] answers and what was asked of it (a test reads and sets it between polls).
#[derive(Debug)]
pub struct BoardCounts {
    /// The answer to the file asked.
    pub answer: Previewed,
    pub starts: usize,
    /// Cancels of a preview that was asked (a cancel with nothing asked is not counted).
    pub cancels: usize,
    pub last_from: Option<ReadFrom>,
    asked: Option<String>,
}

/// The scenes' and tests' source: answers `answer` for the file asked.
pub struct BoardPreview(pub Rc<RefCell<BoardCounts>>);

impl BoardPreview {
    /// A source that answers `answer`, and the handle a test reads it by.
    pub fn new(answer: Previewed) -> (Self, Rc<RefCell<BoardCounts>>) {
        let counts = Rc::new(RefCell::new(BoardCounts {
            answer,
            starts: 0,
            cancels: 0,
            last_from: None,
            asked: None,
        }));
        (BoardPreview(counts.clone()), counts)
    }
}

impl ItemPreviewSource for BoardPreview {
    fn start(&mut self, ask: &PreviewAsk) {
        let mut counts = self.0.borrow_mut();
        if counts.asked.take().is_some() {
            counts.cancels += 1;
        }
        counts.starts += 1;
        counts.last_from = Some(ask.from);
        counts.asked = Some(ask.id.clone());
    }
    fn later(&mut self, id: &str) -> Previewed {
        let counts = self.0.borrow();
        if counts.asked.as_deref() == Some(id) {
            counts.answer.clone()
        } else {
            Previewed::Failed(ActionError::Unavailable)
        }
    }
    fn cancel(&mut self) {
        let mut counts = self.0.borrow_mut();
        if counts.asked.take().is_some() {
            counts.cancels += 1;
        }
    }
}
