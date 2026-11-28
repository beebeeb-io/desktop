//! A PDF's pages for the viewer, drawn by the confined helper (`beebeeb-office-render`, [`DocKind::Pdf`]) through
//! the Office host (`office_host.rs`), never by this process, which holds the keys. Find still reads the text here.
//!
//! - One helper per open PDF, started on the worker that read the file. The bytes go to the host (moved in, wiped
//!   once copied into the helper's section); a second copy is kept here, wiped on drop, for find alone.
//! - Page by page, only when asked: a page is asked of the helper when the view shows it, at [`scale_for`] its
//!   size (the helper's checked sizes; the host refuses a raster past the protocol's limits). An answer has passed
//!   the host's checks before it gets here. `missing_content` is the page's "lacks text" ([`Rendered::lacks_text`]).
//! - A helper that ends (a crash on a hostile file, a timeout, a failed check) ends only itself: the worker says
//!   "stopped" once and returns, the window says the helper stopped, with Try again (a new open, a new helper).
//! - A lock or a sign-out ends the helper twice over: the host's own registry (`office_host::end_session`) kills
//!   every helper of the process, and this worker drops its host as soon as it sees `cancel` (at most
//!   [`IDLE_WAIT`] later), with the bytes kept for find.
//! - Find ([`Document::find`], `viewer_pdf_find.rs`) is the one thing a pure-Rust reader (`hayro`) still does in
//!   this process: the document is opened by it only when the person first searches, on this same worker, asked
//!   down the same channel ([`Ask::Find`]). Open (named in the report): a file made to break that reader can
//!   still end the app at the first find; a panic is caught, a stack overflow or an abort is not.
//! - Not done here: a password prompt (a protected PDF says so and stays closed).
//!
//! Must not: log a byte, a title or a page's text; write anything.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Duration;

use beebeeb_office_protocol::office::PageSize;
use hayro::hayro_syntax::content::ops::TypedInstruction;
use hayro::hayro_syntax::object::{Array, Dict, Name};
use hayro::hayro_syntax::page::Page;
use hayro::hayro_syntax::{DecryptionError, LoadPdfError, Pdf};
use zeroize::Zeroizing;

use crate::engine_host::office_host::{self, CannotOpen, OfficeState};
use crate::frames::viewers::source::Match;

/// What the window asks of the worker that holds the document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ask {
    /// Render page N (from 1).
    Page(u32),
    /// Where the words are in the text; the answer carries `serial` back, so the window takes only the one it waits for.
    Find { serial: u64, words: String },
}

/// The most pages a PDF may have to be opened here (the view lists every page).
pub const PAGES_MAX: u32 = 2_000;
/// How wide a page is rendered, in pixels: sharp at the view's 100 % on a 2x display and at 150 % on a 1x one.
pub const PAGE_WIDTH_PX: f32 = 1_260.0;
/// The most pixels of one rendered page (a very tall or very wide page is rendered smaller).
pub const PAGE_PIXELS_MAX: f32 = 8_000_000.0;
/// The longest side of a rendered page, in pixels (under the protocol's 4,096 a side).
pub const PAGE_SIDE_MAX: f32 = 4_000.0;

/// The decrypted bytes, as the renderer reads them: wiped when the document lets go of them.
struct Wiped(Zeroizing<Vec<u8>>);

impl AsRef<[u8]> for Wiped {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

/// Why a PDF is not opened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotOpened {
    /// It asks for a password.
    Locked,
    /// It has more pages than the viewer lists.
    TooLong { pages: u32 },
    /// Not a PDF the renderer can read (damaged, cut short, or protected in a way it does not name).
    Unreadable,
    /// There is no confined renderer on this PC (not installed, no sandbox on this system): nothing was parsed.
    NoRenderer,
    /// The renderer stopped, or could not be started, or its session ended: Failed, with Try again.
    Stopped,
}

/// One rendered page: straight RGBA, rows top first. No name, no text: a `{:?}` prints its size.
pub struct Rendered {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    /// The page writes in a font the file does not carry and the renderer has no stand-in for: that text is
    /// not in the pixels as it was written.
    pub lacks_text: bool,
}

impl std::fmt::Debug for Rendered {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Rendered({}x{})", self.width, self.height)
    }
}

/// An open PDF as find reads it, on the thread that opened it.
pub struct Document {
    pdf: Pdf,
}

/// The scale a page of `width` x `height` points is rendered at: [`PAGE_WIDTH_PX`] wide, less when that would
/// pass [`PAGE_PIXELS_MAX`] or [`PAGE_SIDE_MAX`]. `None`: the page has no size to render.
pub fn scale_for(width: f32, height: f32) -> Option<f32> {
    if !(width.is_finite() && height.is_finite()) || width < 1.0 || height < 1.0 {
        return None;
    }
    let by_width = PAGE_WIDTH_PX / width;
    let by_pixels = (PAGE_PIXELS_MAX / (width * height)).sqrt();
    let by_side = PAGE_SIDE_MAX / width.max(height);
    Some(by_width.min(by_pixels).min(by_side))
}

/// Whether the page's own content writes in a font that is addressed by character or glyph numbers (a
/// composite font) and is not in the file. Asked of the page itself, because the renderer does not always ask
/// its resolver in a way that tells: when such a font's NAME holds a word like "Sans" or "Serif" (most
/// Chinese, Japanese and Korean fonts' names do), it asks for a standard Latin font by name, exactly as it
/// does for a real Helvetica. Not seen by this: text inside a form the page draws (its fonts are that form's).
/// Find skips such a page: a box on it would point at the wrong place.
fn writes_in_a_numbered_font_not_in_the_file(page: &Page<'_>) -> bool {
    let resources = page.resources();
    let mut operations = page.typed_operations();
    while let Some(operation) = operations.next() {
        if let TypedInstruction::TextFont(chosen) = operation
            && resources
                .get_font(chosen.0)
                .is_some_and(|font| is_numbered_and_not_in_the_file(&font))
        {
            return true;
        }
    }
    false
}

fn is_numbered_and_not_in_the_file(font: &Dict<'_>) -> bool {
    if font
        .get::<Name<'_>>("Subtype")
        .is_none_or(|kind| kind.as_str() != "Type0")
    {
        return false;
    }
    let Some(descendant) = font
        .get::<Array<'_>>("DescendantFonts")
        .and_then(|fonts| fonts.iter::<Dict<'_>>().next())
    else {
        return false;
    };
    let descriptor = descendant.get::<Dict<'_>>("FontDescriptor").unwrap_or_default();
    !["FontFile", "FontFile2", "FontFile3"]
        .iter()
        .any(|program| descriptor.contains_key(program))
}

impl Document {
    /// Open the PDF in `bytes` (taken, not copied; wiped when the document is dropped).
    pub fn open(bytes: Zeroizing<Vec<u8>>) -> Result<Document, NotOpened> {
        let opened = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| Pdf::new(Arc::new(Wiped(bytes)))));
        let pdf = match opened {
            Ok(Ok(pdf)) => pdf,
            Ok(Err(LoadPdfError::Decryption(DecryptionError::PasswordProtected))) => return Err(NotOpened::Locked),
            Ok(Err(_)) | Err(_) => return Err(NotOpened::Unreadable),
        };
        let pages = u32::try_from(pdf.pages().len()).unwrap_or(u32::MAX);
        if pages == 0 {
            return Err(NotOpened::Unreadable);
        }
        if pages > PAGES_MAX {
            return Err(NotOpened::TooLong { pages });
        }
        Ok(Document { pdf })
    }

    #[cfg(test)]
    pub fn pages(&self) -> u32 {
        u32::try_from(self.pdf.pages().len()).unwrap_or(u32::MAX)
    }
}

/// How long the worker waits for an ask while the helper owes it an answer, and while it owes none (then only a
/// stop or a lock is news).
const BUSY_WAIT: Duration = Duration::from_millis(15);
const IDLE_WAIT: Duration = Duration::from_millis(200);

/// What the helper's host handed over since the last poll.
pub(crate) enum Came {
    /// The state changed: read [`Renderer::state`].
    State,
    /// Page `page` (from 1), checked by the host.
    Page { page: u32, rendered: Rendered },
}

/// The calls the worker makes of the helper's host: [`office_host::OfficeHost`] in the app, a stand-in in the
/// tests. Every call returns at once. Dropping it ends the helper. It stays on the worker that started it.
pub(crate) trait Renderer {
    fn state(&self) -> OfficeState;
    fn poll(&mut self) -> Vec<Came>;
    /// Ask for `page` (from 1) at `scale` pixels a point. `false`: not asked (no such page, or past a limit).
    fn ask(&mut self, page: u32, scale: f32) -> bool;
}

#[cfg_attr(test, allow(dead_code))]
struct Live(office_host::OfficeHost);

impl Renderer for Live {
    fn state(&self) -> OfficeState {
        self.0.state()
    }

    fn poll(&mut self) -> Vec<Came> {
        let mut came = Vec::new();
        for event in self.0.poll() {
            match event {
                office_host::Event::State => came.push(Came::State),
                office_host::Event::Page {
                    page,
                    w,
                    h,
                    rgba,
                    missing_content,
                    ..
                } => came.push(Came::Page {
                    page: page.saturating_add(1),
                    rendered: Rendered {
                        width: w,
                        height: h,
                        rgba,
                        lacks_text: missing_content,
                    },
                }),
                // Nothing else is asked of a PDF's helper.
                _ => {}
            }
        }
        came
    }

    fn ask(&mut self, page: u32, scale: f32) -> bool {
        page.checked_sub(1)
            .is_some_and(|page| self.0.request_page(page, scale, None).is_some())
    }
}

/// Start the confined helper on `bytes` (moved in). Returns at once; the state says how it went (a helper that is
/// not beside the app, or a system with no sandbox for it, is a state too: nothing is ever parsed unconfined).
#[cfg(not(test))]
pub(crate) fn start(bytes: Zeroizing<Vec<u8>>) -> Box<dyn Renderer> {
    let helper = office_host::helper_exe().unwrap_or_else(|| office_host::HELPER_EXE.into());
    // The worker asks on its own beat ([`BUSY_WAIT`]): nobody is woken.
    let host = office_host::OfficeHost::open(&helper, office_host::DocKind::Pdf, bytes, Arc::new(|| {}));
    Box::new(Live(host))
}

/// In the tests the helper is stood in for (`viewer_pdf_tests.rs`): no test starts a process.
#[cfg(test)]
pub(crate) fn start(bytes: Zeroizing<Vec<u8>>) -> Box<dyn Renderer> {
    tests::stand_in(bytes)
}

/// What find reads: the bytes until the first find, the document after it.
enum Text {
    Bytes(Zeroizing<Vec<u8>>),
    Open(Document),
    /// The reader here could not open it: every find finds nothing.
    Closed,
}

/// An open PDF on its worker: the helper that draws its pages, and what find reads.
pub struct Session {
    renderer: Box<dyn Renderer>,
    sizes: Vec<PageSize>,
    pages: u32,
    text: Text,
}

impl Session {
    /// Wait for `renderer` to open the document (the host's own timer ends a helper that never answers), until
    /// `cancel` is set. `bytes` are kept for find (wiped when the session is dropped).
    pub(crate) fn open(
        mut renderer: Box<dyn Renderer>,
        bytes: Zeroizing<Vec<u8>>,
        cancel: &AtomicBool,
    ) -> Result<Session, NotOpened> {
        loop {
            if cancel.load(Ordering::SeqCst) {
                return Err(NotOpened::Stopped);
            }
            drop(renderer.poll());
            let (pages, sizes) = match renderer.state() {
                OfficeState::Opening => {
                    std::thread::sleep(Duration::from_millis(5));
                    continue;
                }
                OfficeState::Pages { pages, sizes } => (pages, sizes),
                OfficeState::NoPreview(office_host::NoPreview::Password) => return Err(NotOpened::Locked),
                OfficeState::NoPreview(_) => return Err(NotOpened::NoRenderer),
                // Over a limit of the helper's contract is not a page count to name: said as not readable here.
                OfficeState::CannotOpen(CannotOpen::Damaged | CannotOpen::TooLarge) | OfficeState::Sheets { .. } => {
                    tracing::warn!("viewer open failed: the helper could not read the file");
                    return Err(NotOpened::Unreadable);
                }
                OfficeState::Stopped { why, .. } => {
                    // One line a failure, the kind only (RC2-3: every PDF failed here and the log said nothing).
                    match why {
                        office_host::Stop::Exited => tracing::warn!("viewer open failed: the helper exited"),
                        office_host::Stop::TimedOut => tracing::warn!("viewer open failed: the helper timed out"),
                        office_host::Stop::Protocol => {
                            tracing::warn!("viewer open failed: the helper answer failed a check");
                        }
                        office_host::Stop::Failed => tracing::warn!("viewer open failed: the helper could not start"),
                    }
                    return Err(NotOpened::Stopped);
                }
                // A lock or a sign-out, not a failure.
                OfficeState::Closed(_) => return Err(NotOpened::Stopped),
            };
            return match pages {
                0 => Err(NotOpened::Unreadable),
                pages if pages > PAGES_MAX => Err(NotOpened::TooLong { pages }),
                pages => Ok(Session {
                    renderer,
                    sizes,
                    pages,
                    text: Text::Bytes(bytes),
                }),
            };
        }
    }

    pub fn pages(&self) -> u32 {
        self.pages
    }

    /// Serve the pages and finds asked down `asks`, until the asker is gone, `cancel` is set or the helper stops.
    /// A page's answer goes to `deliver` (its pixels, or `None` when it cannot be asked for), a find's matches to
    /// `found` with the serial it was asked under; neither once `cancel` is set. A helper that stopped is said
    /// once, to `stopped`, and nothing comes after it. Returning drops the session: the helper is ended, and the
    /// bytes kept for find are overwritten.
    pub(crate) fn serve(
        mut self,
        asks: &mpsc::Receiver<Ask>,
        cancel: &AtomicBool,
        deliver: &dyn Fn(u32, Option<Rendered>),
        found: &dyn Fn(u64, Vec<Match>),
        stopped: &dyn Fn(),
    ) {
        // The pages asked of the helper and not answered.
        let mut out: Vec<u32> = Vec::new();
        loop {
            let (mut page, mut find) = (None, None);
            let mut take = |ask: Ask| match ask {
                Ask::Page(asked) => page = Some(asked),
                Ask::Find { serial, words } => find = Some((serial, words)),
            };
            match asks.recv_timeout(if out.is_empty() { IDLE_WAIT } else { BUSY_WAIT }) {
                Ok(first) => {
                    take(first);
                    // Only the newest page and the newest find waiting are served.
                    while let Ok(newer) = asks.try_recv() {
                        take(newer);
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
            if cancel.load(Ordering::SeqCst) {
                return;
            }
            if let Some(page) = page.filter(|page| !out.contains(page)) {
                let scale = page
                    .checked_sub(1)
                    .and_then(|index| self.sizes.get(index as usize))
                    .and_then(|size| scale_for(size.w_pt, size.h_pt));
                match scale {
                    Some(scale) if page <= self.pages && self.renderer.ask(page, scale) => out.push(page),
                    _ => deliver(page, None),
                }
            }
            for came in self.renderer.poll() {
                match came {
                    Came::Page { page, rendered } => {
                        out.retain(|asked| *asked != page);
                        if cancel.load(Ordering::SeqCst) {
                            return;
                        }
                        deliver(page, Some(rendered));
                    }
                    Came::State => {}
                }
            }
            if !matches!(self.renderer.state(), OfficeState::Pages { .. }) {
                if !cancel.load(Ordering::SeqCst) {
                    stopped();
                }
                return;
            }
            if let Some((serial, words)) = find {
                let matches = self.find(&words, cancel);
                if cancel.load(Ordering::SeqCst) {
                    return;
                }
                found(serial, matches);
            }
        }
    }

    /// Where `words` are: the reader opens the document at the first find (a panic in it finds nothing).
    fn find(&mut self, words: &str, cancel: &AtomicBool) -> Vec<Match> {
        if let Text::Bytes(_) = &self.text {
            let Text::Bytes(bytes) = std::mem::replace(&mut self.text, Text::Closed) else {
                return Vec::new();
            };
            if let Ok(document) = Document::open(bytes) {
                self.text = Text::Open(document);
            }
        }
        let Text::Open(document) = &self.text else {
            return Vec::new();
        };
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| document.find(words, cancel))).unwrap_or_default()
    }
}

#[path = "viewer_pdf_find.rs"]
mod find;

#[cfg(test)]
#[path = "viewer_pdf_tests.rs"]
pub(crate) mod tests;
