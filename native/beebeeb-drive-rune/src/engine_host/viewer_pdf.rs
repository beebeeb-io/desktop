//! A PDF's pages for the viewer, rendered in this process by a pure-Rust renderer (`hayro`): from the decrypted
//! bytes IN MEMORY to pixels, one page at a time, on the worker that opened the file.
//!
//! - Memory only: the renderer is handed the bytes (no copy: it reads the buffer the engine decrypted into, which
//!   is overwritten when the document is dropped); it opens no file and no connection, and nothing is written.
//!   What the renderer decodes of the document on the way (streams, glyphs) is its own memory: released with the
//!   document, not overwritten. Rendered pixels are the toolkit's, as a photo's are.
//! - Page by page, only when asked: the document is opened once; a page is rendered when the view shows it. A
//!   request that was overtaken (the person paged on) is skipped.
//! - Its own limits, asked BEFORE rendering: more pages than [`PAGES_MAX`] is not opened; a page is rendered at
//!   most [`PAGE_WIDTH_PX`] wide and never more than [`PAGE_PIXELS_MAX`] pixels, from the page's own box.
//! - A page that makes the renderer panic is that page's failure ([`PageState::Failed`]), not the app's. (A stack
//!   overflow or an out-of-memory abort in the renderer is not caught by this; the limits above are the bound.)
//! - Text the renderer has no font for is SAID: a PDF may name a font without carrying it. The renderer's own
//!   fonts are the 14 standard Latin ones; for a font addressed by character or glyph numbers (every Chinese,
//!   Japanese or Korean one, and others) they are no stand-in, and its text is drawn wrong or not at all. The
//!   page is then handed over as lacking text ([`Rendered::lacks_text`]), and the view says so under it.
//!   (Measured: the renderer asks its font resolver for such a font and gives no warning of its own; text in a
//!   Japanese or Chinese font that is not in the file came out as empty space.)
//! - Not done here: a password prompt (a protected PDF says so and stays closed), text search, drawing text in
//!   a font the file does not carry beyond the Latin stand-ins (no other font is bundled). Not noticed: a
//!   one-byte font that is not in the file and writes another script than the stand-ins hold (the renderer
//!   picks the stand-in without asking).
//!
//! The renderer's page cache is not `Send` and borrows the document: both live and die on the worker's thread.
//! Must not: log a byte, a title or a page's text; write anything.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;

use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_interpret::font::FontQuery;
use hayro::hayro_syntax::{DecryptionError, LoadPdfError, Pdf};
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::{PixmapSettings, RenderCache, RenderSettings};
use zeroize::Zeroizing;

/// The most pages a PDF may have to be opened here (the view lists every page).
pub const PAGES_MAX: u32 = 2_000;
/// How wide a page is rendered, in pixels: sharp at the view's 100 % on a 2x display and at 150 % on a 1x one.
pub const PAGE_WIDTH_PX: f32 = 1_260.0;
/// The most pixels of one rendered page (a very tall or very wide page is rendered smaller).
pub const PAGE_PIXELS_MAX: f32 = 8_000_000.0;
/// The longest side of a rendered page, in pixels.
pub const PAGE_SIDE_MAX: f32 = 8_000.0;

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

/// An open PDF, on the thread that opened it.
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

/// Whether `query` asks for a font the renderer's own fonts cannot stand in for. The renderer asks for one of
/// the 14 standard fonts by name (it has them), and otherwise only for a font that is not in the file and is
/// addressed by character or glyph numbers: every Chinese, Japanese or Korean font, and any other written that
/// way. What it gets is a Latin font in another order.
fn has_no_stand_in(query: &FontQuery) -> bool {
    match query {
        FontQuery::Standard(_) => false,
        FontQuery::Fallback(_) => true,
    }
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

    pub fn pages(&self) -> u32 {
        u32::try_from(self.pdf.pages().len()).unwrap_or(u32::MAX)
    }

    /// Render the pages asked for down `asks` (from 1), until the asker is gone or `cancel` is set. Each answer
    /// goes to `deliver`: the page, and its pixels or `None` when it could not be rendered. Only the newest
    /// request waiting is rendered.
    pub fn serve(&self, asks: &mpsc::Receiver<u32>, cancel: &AtomicBool, deliver: &dyn Fn(u32, Option<Rendered>)) {
        // The renderer asks for a font the file names and does not carry. Where the answer is a Latin stand-in
        // that cannot draw the text, the page says so.
        let lacking = Arc::new(AtomicBool::new(false));
        let settings = {
            let defaults = InterpreterSettings::default();
            let (stand_in, lacking) = (defaults.font_resolver.clone(), lacking.clone());
            InterpreterSettings {
                font_resolver: Arc::new(move |query| {
                    if has_no_stand_in(query) {
                        lacking.store(true, Ordering::SeqCst);
                    }
                    stand_in(query)
                }),
                ..defaults
            }
        };
        let render_settings = RenderSettings::default();
        let mut cache = RenderCache::new();
        // The cache keeps a font once asked for, so a later page would not ask again: from the first page that
        // lacks text on, every page starts from an empty cache and is asked for itself.
        let mut ask_each_page = false;
        while let Ok(mut page) = asks.recv() {
            while let Ok(newer) = asks.try_recv() {
                page = newer;
            }
            if cancel.load(Ordering::SeqCst) {
                return;
            }
            if ask_each_page {
                cache = RenderCache::new();
            }
            lacking.store(false, Ordering::SeqCst);
            let rendered = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                self.render(page, &cache, &settings, &render_settings)
            }));
            let lacks_text = lacking.load(Ordering::SeqCst);
            ask_each_page |= lacks_text;
            let rendered = match rendered {
                Ok(rendered) => rendered.map(|rendered| Rendered { lacks_text, ..rendered }),
                Err(_) => {
                    // What the cache held half-made goes with the page that broke.
                    cache = RenderCache::new();
                    None
                }
            };
            if cancel.load(Ordering::SeqCst) {
                return;
            }
            deliver(page, rendered);
        }
    }

    fn render<'a>(
        &'a self,
        page: u32,
        cache: &RenderCache<'a>,
        settings: &InterpreterSettings,
        render_settings: &RenderSettings,
    ) -> Option<Rendered> {
        let pages = self.pdf.pages();
        let page = pages.get(usize::try_from(page.checked_sub(1)?).ok()?)?;
        let (width, height) = page.render_dimensions();
        let scale = scale_for(width, height)?;
        let pixmap = hayro::render(
            page,
            cache,
            settings,
            render_settings,
            &PixmapSettings {
                x_scale: scale,
                y_scale: scale,
                bg_color: WHITE,
            },
        );
        let (width, height) = (u32::from(pixmap.width()), u32::from(pixmap.height()));
        if width == 0 || height == 0 {
            return None;
        }
        // On a white ground every pixel is opaque: premultiplied is straight.
        Some(Rendered {
            width,
            height,
            rgba: pixmap.data_as_u8_slice().to_vec(),
            lacks_text: false,
        })
    }
}

#[cfg(test)]
#[path = "viewer_pdf_tests.rs"]
pub(crate) mod tests;
