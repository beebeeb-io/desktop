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
//! - Not done here: a password prompt (a protected PDF says so and stays closed), text search, fonts that the PDF
//!   does not embed beyond the 14 standard Latin ones (such text is not drawn; the renderer gives no warning for
//!   it, so the view cannot say it).
//!
//! The renderer's page cache is not `Send` and borrows the document: both live and die on the worker's thread.
//! Must not: log a byte, a title or a page's text; write anything.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;

use hayro::hayro_interpret::InterpreterSettings;
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
        let settings = InterpreterSettings::default();
        let render_settings = RenderSettings::default();
        let mut cache = RenderCache::new();
        while let Ok(mut page) = asks.recv() {
            while let Ok(newer) = asks.try_recv() {
                page = newer;
            }
            if cancel.load(Ordering::SeqCst) {
                return;
            }
            let rendered = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                self.render(page, &cache, &settings, &render_settings)
            }));
            let rendered = match rendered {
                Ok(rendered) => rendered,
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
        })
    }
}

#[cfg(test)]
#[path = "viewer_pdf_tests.rs"]
pub(crate) mod tests;
