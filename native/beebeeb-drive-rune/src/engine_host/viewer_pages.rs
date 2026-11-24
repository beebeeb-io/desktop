//! The window's side of an open PDF, and the worker's ends of its channels (moved out of `viewer.rs`, which has no
//! lines to spare). Child of `viewer.rs`.
//!
//! Pages are asked of the worker that holds the document (it asks the confined helper: `viewer_pdf.rs`) and the last few that came back are kept. A find is asked
//! down the same channel and answered on its own, under a serial: an answer for any other serial than the one the
//! window waits for (an earlier search, or one asked of a document that has since closed) is dropped here.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use super::PAGES_KEPT;
use super::viewer_pdf::Ask;
use crate::frames::viewers::source::Match;

/// One page's answer from the worker: the page (from 1), its pixels or `None` when it could not be rendered, and
/// whether text of the page is not in the pixels (`viewer_pdf::Rendered::lacks_text`).
pub(super) type PageAnswer = (u32, Option<rune::Pixels>, bool);

/// A find's answer from the worker: the serial it was asked under, and where the words are.
pub(super) type FoundAnswer = (u64, Vec<Match>);

/// The window's side of an open PDF: pages are asked of the worker that holds the document, and the last few that
/// came back are kept. Asking happens while the view is drawn (`ViewerSource::picture` takes `&self`), so what
/// changes then sits in cells; this is the window's thread's alone.
pub(super) struct Pages {
    ask: mpsc::Sender<Ask>,
    pub(super) got: mpsc::Receiver<PageAnswer>,
    /// The newest last. `None` pixels: that page could not be rendered.
    pub(super) kept: RefCell<VecDeque<PageAnswer>>,
    /// The page the worker was last asked for and has not answered.
    pub(super) asked: Cell<Option<u32>>,
    /// The document as the view was told, once the worker opened it.
    pub(super) doc: RefCell<Option<crate::frames::viewers::kinds::PdfDoc>>,
    found: mpsc::Receiver<FoundAnswer>,
    /// The serial of the last find asked.
    serial: Cell<u64>,
    /// The helper that draws the pages stopped (the worker said so, once): no page comes any more.
    stopped: Arc<AtomicBool>,
}

impl Pages {
    /// The window's side and the worker's ends of a new PDF's channels.
    pub(super) fn open(size: String) -> (Pages, PdfEnds) {
        let ((ask, asks), (page_answers, got), (found_answers, found)) =
            (mpsc::channel(), mpsc::channel(), mpsc::channel());
        let pages = Pages {
            ask,
            got,
            kept: RefCell::default(),
            asked: Cell::new(None),
            doc: RefCell::new(None),
            found,
            serial: Cell::new(0),
            stopped: Arc::default(),
        };
        let ends = PdfEnds {
            asks,
            answers: page_answers,
            found: found_answers,
            stopped: pages.stopped.clone(),
            size,
        };
        (pages, ends)
    }

    /// The helper stopped since the last call (said once): the view shows Failed, with Try again.
    pub(super) fn take_stopped(&self) -> bool {
        self.stopped.swap(false, Ordering::SeqCst)
    }

    /// Take what the worker rendered since the last call. `true`: something came.
    pub(super) fn take_in(&self) -> bool {
        let mut kept = self.kept.borrow_mut();
        let mut came = false;
        while let Ok((page, pixels, lacks_text)) = self.got.try_recv() {
            kept.retain(|(held, ..)| *held != page);
            kept.push_back((page, pixels, lacks_text));
            while kept.len() > PAGES_KEPT {
                kept.pop_front();
            }
            if self.asked.get() == Some(page) {
                self.asked.set(None);
            }
            came = true;
        }
        came
    }

    pub(super) fn held(&self, page: u32) -> Option<Option<rune::Pixels>> {
        self.kept
            .borrow()
            .iter()
            .find(|(held, ..)| *held == page)
            .map(|(_, pixels, _)| pixels.clone())
    }

    /// Whether the kept picture of `page` lacks text the renderer had no font for.
    pub(super) fn lacks_text(&self, page: u32) -> bool {
        self.kept
            .borrow()
            .iter()
            .any(|(held, pixels, lacks_text)| *held == page && pixels.is_some() && *lacks_text)
    }

    /// Ask the worker for `page`, once until it answers.
    pub(super) fn ask_page(&self, page: u32) {
        if self.asked.get() != Some(page) && self.ask.send(Ask::Page(page)).is_ok() {
            self.asked.set(Some(page));
        }
    }

    /// Ask the worker where `words` are, and wait for the answer at most `wait`. `None`: the worker is gone or did
    /// not answer in time (nothing was found out: the caller must not say "no matches").
    pub(super) fn find(&self, words: &str, wait: Duration) -> Option<Vec<Match>> {
        let serial = self.serial.get() + 1;
        self.serial.set(serial);
        let ask = Ask::Find {
            serial,
            words: words.to_string(),
        };
        self.ask.send(ask).ok()?;
        self.await_found(serial, wait)
    }

    /// The answer for `serial`, waiting at most `wait`; answers for other serials that come first are dropped.
    pub(super) fn await_found(&self, serial: u64, wait: Duration) -> Option<Vec<Match>> {
        let end = Instant::now() + wait;
        loop {
            let left = end.checked_duration_since(Instant::now())?;
            match self.found.recv_timeout(left) {
                Ok((answered, matches)) if answered == serial => return Some(matches),
                Ok(_) => {}
                Err(_) => return None,
            }
        }
    }
}

/// The worker's ends of an open PDF's channels, and the size the view is told.
pub(super) struct PdfEnds {
    pub(super) asks: mpsc::Receiver<Ask>,
    pub(super) answers: mpsc::Sender<PageAnswer>,
    pub(super) found: mpsc::Sender<FoundAnswer>,
    pub(super) stopped: Arc<AtomicBool>,
    pub(super) size: String,
}
