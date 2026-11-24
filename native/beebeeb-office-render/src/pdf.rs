//! PDFs, rendered in the helper by the pure-Rust renderer `hayro` (2501): a hostile PDF ends this process, never the
//! app, and a page the renderer never finishes is ended with it.
//!
//! The limits, and who holds them:
//! - pages: more than [`MAX_PAGES`] is not opened (`TooLarge`), here;
//! - pixels: a page or tile is at most [`MAX_SIDE_PX`] a side (`TooLarge`), here, before anything is drawn;
//! - memory: what the renderer allocates while it decodes a stream or an image is bounded by the sandbox's job
//!   (`HELPER_MEMORY_BYTES`); past it the helper dies, the app says "Renderer stopped";
//! - time: the app's timers (`OPEN_TIMEOUT`, `REQUEST_TIMEOUT`) kill the helper, and the decrypted document with it.
//!
//! A page is sized by the protocol's page rule ([`page_pixels`]: `ceil(pt * scale)` a side), so the buffer the app
//! sizes from the `Opened` sizes is the one rendered into; a region is a rectangle of that full-page raster.
//!
//! Text in a font the file does not carry and the renderer has no stand-in for (a composite font: Chinese, Japanese,
//! Korean) is drawn in a Latin stand-in that cannot draw it: the page says `missing_content`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use beebeeb_office_protocol::check::{MAX_PAGES, MAX_SIDE_PX, page_pixels};
use beebeeb_office_protocol::office::{CellData, CellRange, ErrorCode, Opened, OutlineEntry, PageSize, Rect};
use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_interpret::font::FontQuery;
use hayro::hayro_interpret::util::TransformExt;
use hayro::hayro_syntax::{DecryptionError, LoadPdfError, Pdf};
use hayro::kurbo::Affine;
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::vello_cpu::{PixmapMut, RasterizerSettings, RenderContext, Resources, TargetInit};
use hayro::{RenderCache, RenderSettings};

use crate::engine::{OpenDoc, Opening};

/// An open PDF. The bytes are the renderer's own copy; the app's section is unmapped right after the open.
pub(crate) struct PdfDoc {
    pdf: Pdf,
    sizes: Vec<PageSize>,
    settings: InterpreterSettings,
    /// Set by the font resolver when it is asked for a font it has no stand-in for; cleared before each render.
    lacking: Arc<AtomicBool>,
}

/// Opens `bytes` as a PDF: `Password` when it asks for one, `Damaged` when the renderer cannot read it or it has no
/// page, `TooLarge` past [`MAX_PAGES`].
pub(crate) fn open(bytes: &[u8]) -> Result<PdfDoc, ErrorCode> {
    let pdf = match Pdf::new(bytes.to_vec()) {
        Ok(pdf) => pdf,
        Err(LoadPdfError::Decryption(DecryptionError::PasswordProtected)) => return Err(ErrorCode::Password),
        Err(_) => return Err(ErrorCode::Damaged),
    };
    let count = pdf.pages().len();
    if count == 0 {
        return Err(ErrorCode::Damaged);
    }
    if u32::try_from(count).map_or(true, |n| n > MAX_PAGES) {
        return Err(ErrorCode::TooLarge);
    }
    let sizes = pdf
        .pages()
        .iter()
        .map(|page| {
            let (w_pt, h_pt) = page.render_dimensions();
            PageSize { w_pt, h_pt }
        })
        .collect();
    let lacking = Arc::new(AtomicBool::new(false));
    let settings = {
        let defaults = InterpreterSettings::default();
        let (stand_in, lacking) = (defaults.font_resolver.clone(), lacking.clone());
        InterpreterSettings {
            font_resolver: Arc::new(move |query| {
                if let FontQuery::Fallback(_) = query {
                    lacking.store(true, Ordering::SeqCst);
                }
                stand_in(query)
            }),
            ..defaults
        }
    };
    Ok(PdfDoc {
        pdf,
        sizes,
        settings,
        lacking,
    })
}

impl PdfDoc {
    fn page_size(&self, page: u32) -> Result<&PageSize, ErrorCode> {
        usize::try_from(page)
            .ok()
            .and_then(|n| self.sizes.get(n))
            .ok_or(ErrorCode::Internal)
    }
}

/// The pixel size of `page` (whole, or `region` of it) at `scale`, and the region's origin. A page or region over
/// [`MAX_SIDE_PX`] a side is `TooLarge`; a region not inside the page, a page of no size or a scale that is not
/// finite and positive is `Internal` (the app never asks it).
fn sized(page: &PageSize, scale: f32, region: Option<Rect>) -> Result<(u32, u32, u32, u32), ErrorCode> {
    let (pw, ph) = page_pixels(page, scale).ok_or(ErrorCode::Internal)?;
    let (x, y, w, h) = match region {
        None => (0, 0, pw, ph),
        Some(r) => {
            let inside = r.w > 0
                && r.h > 0
                && r.x.checked_add(r.w).is_some_and(|right| right <= pw)
                && r.y.checked_add(r.h).is_some_and(|bottom| bottom <= ph);
            if !inside {
                return Err(ErrorCode::Internal);
            }
            (r.x, r.y, r.w, r.h)
        }
    };
    if w > MAX_SIDE_PX || h > MAX_SIDE_PX {
        return Err(ErrorCode::TooLarge);
    }
    Ok((x, y, w, h))
}

impl OpenDoc for PdfDoc {
    fn opened(&self) -> Opening {
        Opening::Pages(Opened {
            // At most MAX_PAGES, checked at open.
            pages: u32::try_from(self.sizes.len()).unwrap_or(u32::MAX),
            sizes: self.sizes.clone(),
        })
    }

    fn render_size(&self, page: u32, scale: f32, region: Option<Rect>) -> Result<(u32, u32), ErrorCode> {
        let (_, _, w, h) = sized(self.page_size(page)?, scale, region)?;
        Ok((w, h))
    }

    fn render(&self, page: u32, scale: f32, region: Option<Rect>, out: &mut [u8]) -> Result<bool, ErrorCode> {
        let (x, y, w, h) = sized(self.page_size(page)?, scale, region)?;
        // At most MAX_SIDE_PX (4,096) a side: fits a u16.
        let (w16, h16) = (
            u16::try_from(w).map_err(|_| ErrorCode::TooLarge)?,
            u16::try_from(h).map_err(|_| ErrorCode::TooLarge)?,
        );
        let target = PixmapMut::new(w16, h16, out).ok_or(ErrorCode::Internal)?;
        let pages = self.pdf.pages();
        let page = usize::try_from(page)
            .ok()
            .and_then(|n| pages.get(n))
            .ok_or(ErrorCode::Internal)?;
        self.lacking.store(false, Ordering::SeqCst);
        // A fresh cache per page: the resolver is asked for every font the page uses, so its note is this page's.
        let cache = RenderCache::new();
        let mut ctx = RenderContext::new(w16, h16);
        let transform = Affine::translate((-f64::from(x), -f64::from(y)))
            * Affine::scale(f64::from(scale))
            * page.initial_transform(true).to_kurbo();
        hayro::render_into(
            page,
            &cache,
            &self.settings,
            &RenderSettings::default(),
            &mut ctx,
            transform,
        );
        ctx.flush();
        ctx.render_with(
            target,
            &mut Resources::default(),
            RasterizerSettings {
                target_init: TargetInit::Clear(WHITE),
                ..Default::default()
            },
        );
        // On a white ground every pixel is opaque: premultiplied is straight RGBA8.
        Ok(self.lacking.load(Ordering::SeqCst))
    }

    fn outline(&self) -> Vec<OutlineEntry> {
        Vec::new()
    }

    fn words(&self) -> u32 {
        0
    }

    fn cells(&self, _sheet: u32, _range: CellRange) -> Result<CellData, ErrorCode> {
        Err(ErrorCode::Unsupported)
    }

    fn anchor_size(&self, _sheet: u32, _anchor: u32, _scale: f32) -> Result<(u32, u32), ErrorCode> {
        Err(ErrorCode::Unsupported)
    }

    fn render_anchor(&self, _sheet: u32, _anchor: u32, _scale: f32, _out: &mut [u8]) -> Result<bool, ErrorCode> {
        Err(ErrorCode::Unsupported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A PDF of one page per `(width, height, content)`, with a correct cross-reference table.
    fn pdf_of(pages: &[(u32, u32, &str)]) -> Vec<u8> {
        let mut objects: Vec<String> = Vec::new();
        let n = pages.len();
        // 1: catalog, 2: page tree, then a page and its content per page.
        objects.push("<< /Type /Catalog /Pages 2 0 R >>".to_owned());
        let kids: Vec<String> = (0..n).map(|i| format!("{} 0 R", 3 + 2 * i)).collect();
        objects.push(format!("<< /Type /Pages /Kids [{}] /Count {n} >>", kids.join(" ")));
        for (i, (w, h, content)) in pages.iter().enumerate() {
            objects.push(format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {w} {h}] /Contents {} 0 R /Resources << >> >>",
                4 + 2 * i
            ));
            objects.push(format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()));
        }
        let mut out = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (i, body) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
        }
        let xref = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
        for off in offsets {
            out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                objects.len() + 1
            )
            .as_bytes(),
        );
        out
    }

    /// Page 0: 200 x 100 pt, a black square at (0,0)-(50,50) in PDF space (the bottom left). Page 1: Letter, empty.
    fn two_pages() -> PdfDoc {
        open(&pdf_of(&[(200, 100, "0 0 0 rg 0 0 50 50 re f"), (612, 792, "")])).expect("opens")
    }

    fn pixel(buf: &[u8], w: u32, x: u32, y: u32) -> [u8; 4] {
        let at = ((y * w + x) * 4) as usize;
        [buf[at], buf[at + 1], buf[at + 2], buf[at + 3]]
    }

    const BLACK: [u8; 4] = [0, 0, 0, 255];
    const WHITE_PX: [u8; 4] = [255, 255, 255, 255];

    #[test]
    fn pdf_opens_as_pages_with_their_sizes() {
        match two_pages().opened() {
            Opening::Pages(opened) => {
                assert_eq!(opened.pages, 2);
                assert_eq!(
                    opened.sizes,
                    vec![
                        PageSize {
                            w_pt: 200.0,
                            h_pt: 100.0
                        },
                        PageSize {
                            w_pt: 612.0,
                            h_pt: 792.0
                        }
                    ]
                );
            }
            other => panic!("pages were due: {other:?}"),
        }
    }

    #[test]
    fn pdf_page_is_sized_by_the_protocol_rule() {
        let doc = two_pages();
        for scale in [1.0, 4.0 / 3.0, 2.0, 0.37] {
            for (page, size) in doc.sizes.iter().enumerate() {
                assert_eq!(doc.render_size(page as u32, scale, None).ok(), page_pixels(size, scale));
            }
        }
        let tile = Some(Rect {
            x: 10,
            y: 20,
            w: 30,
            h: 40,
        });
        assert_eq!(doc.render_size(0, 1.0, tile), Ok((30, 40)));
    }

    #[test]
    fn pdf_refuses_what_it_cannot_render() {
        let doc = two_pages();
        // No such page, a bad scale, a region off the page or empty.
        assert_eq!(doc.render_size(2, 1.0, None), Err(ErrorCode::Internal));
        assert_eq!(doc.render_size(0, f32::NAN, None), Err(ErrorCode::Internal));
        assert_eq!(doc.render_size(0, 0.0, None), Err(ErrorCode::Internal));
        let off = Some(Rect {
            x: 190,
            y: 0,
            w: 20,
            h: 10,
        });
        assert_eq!(doc.render_size(0, 1.0, off), Err(ErrorCode::Internal));
        let empty = Some(Rect {
            x: 0,
            y: 0,
            w: 0,
            h: 10,
        });
        assert_eq!(doc.render_size(0, 1.0, empty), Err(ErrorCode::Internal));
        let wraps = Some(Rect {
            x: u32::MAX,
            y: 0,
            w: 2,
            h: 1,
        });
        assert_eq!(doc.render_size(0, 1.0, wraps), Err(ErrorCode::Internal));
        // Over the side limit, as a whole page and as a tile.
        assert_eq!(doc.render_size(1, 6.0, None), Err(ErrorCode::TooLarge));
        let big = Some(Rect {
            x: 0,
            y: 0,
            w: MAX_SIDE_PX + 1,
            h: 1,
        });
        assert_eq!(doc.render_size(1, 8.0, big), Err(ErrorCode::TooLarge));
        // A buffer of another size is refused, never written past.
        let mut out = vec![0u8; 7];
        assert_eq!(doc.render(0, 1.0, None, &mut out), Err(ErrorCode::Internal));
        // No sheets, no anchors.
        assert_eq!(doc.anchor_size(0, 0, 1.0), Err(ErrorCode::Unsupported));
    }

    #[test]
    fn pdf_renders_the_page_and_its_tiles() {
        let doc = two_pages();
        let (w, h) = doc.render_size(0, 2.0, None).expect("sized");
        assert_eq!((w, h), (400, 200));
        let mut page = vec![0u8; (w * h * 4) as usize];
        assert_eq!(doc.render(0, 2.0, None, &mut page), Ok(false));
        // The square is at the bottom left (rows top first): 100 x 100 px at scale 2.
        assert_eq!(pixel(&page, w, 10, 190), BLACK);
        assert_eq!(pixel(&page, w, 90, 110), BLACK);
        assert_eq!(pixel(&page, w, 10, 10), WHITE_PX);
        assert_eq!(pixel(&page, w, 300, 190), WHITE_PX);
        // A tile is the same pixels as the page it is cut from.
        let r = Rect {
            x: 60,
            y: 80,
            w: 100,
            h: 70,
        };
        let mut tile = vec![0u8; (r.w * r.h * 4) as usize];
        assert_eq!(doc.render(0, 2.0, Some(r), &mut tile), Ok(false));
        for ty in 0..r.h {
            for tx in 0..r.w {
                assert_eq!(
                    pixel(&tile, r.w, tx, ty),
                    pixel(&page, w, r.x + tx, r.y + ty),
                    "tile pixel ({tx}, {ty})"
                );
            }
        }
        // An empty Letter page at 4/3 is white.
        let (w1, h1) = doc.render_size(1, 4.0 / 3.0, None).expect("sized");
        let mut letter = vec![0u8; (w1 * h1 * 4) as usize];
        assert_eq!(doc.render(1, 4.0 / 3.0, None, &mut letter), Ok(false));
        assert!(letter.as_chunks::<4>().0.iter().all(|p| *p == WHITE_PX));
    }

    #[test]
    fn pdf_open_refuses_garbage_no_pages_and_too_many() {
        assert_eq!(open(b"not a pdf at all").err(), Some(ErrorCode::Damaged));
        assert_eq!(open(b"").err(), Some(ErrorCode::Damaged));
        assert_eq!(open(&pdf_of(&[])).err(), Some(ErrorCode::Damaged));
        let at_limit: Vec<(u32, u32, &str)> = vec![(10, 10, ""); MAX_PAGES as usize];
        assert!(open(&pdf_of(&at_limit)).is_ok());
        let over: Vec<(u32, u32, &str)> = vec![(10, 10, ""); MAX_PAGES as usize + 1];
        assert_eq!(open(&pdf_of(&over)).err(), Some(ErrorCode::TooLarge));
    }

    /// RC2-3: the two kinds of PDF that did not open on the Windows test machine, as files (the app's fixtures).
    #[test]
    fn pdf_the_two_rc2_fixtures_open_with_their_page_counts() {
        let files: [(&[u8], u32); 2] = [
            (include_bytes!("../../beebeeb-drive-rune/tests/fixtures/pdf/text-3-pages.pdf"), 3),
            (include_bytes!("../../beebeeb-drive-rune/tests/fixtures/pdf/minimal-1-page.pdf"), 1),
        ];
        for (bytes, pages) in files {
            match open(bytes).expect("opens").opened() {
                Opening::Pages(opened) => assert_eq!((opened.pages, opened.sizes.len() as u32), (pages, pages)),
                Opening::Sheets(_) => panic!("a PDF opens as pages"),
            }
        }
        // Cut short: damaged, said as that.
        assert_eq!(open(&files[0].0[..64]).err(), Some(ErrorCode::Damaged));
    }
}
