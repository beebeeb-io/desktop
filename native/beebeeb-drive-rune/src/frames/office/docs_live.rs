//! The live half of the document scene (V3-5): what is asked of the renderer, and how a page is drawn from its answers.

use std::cell::RefCell;
use std::collections::HashMap;

use rune::{El, Pixels, SurfaceFit, el};

use super::{DocBody, PAGE_GAP, box_pt, paper_box, shell};
use crate::frames::viewers::kinds::{OfficePicture, TILE_PX, uses_tiles};

/// What a live document asks its renderer for: every key at most once per frame (the answer is kept for the frame),
/// whoever in the frame asks.
pub struct Asks<'a> {
    ask: &'a dyn Fn(OfficePicture) -> Option<Pixels>,
    seen: RefCell<HashMap<OfficePicture, Option<Pixels>>>,
}

impl<'a> Asks<'a> {
    pub fn new(ask: &'a dyn Fn(OfficePicture) -> Option<Pixels>) -> Self {
        Asks {
            ask,
            seen: RefCell::new(HashMap::new()),
        }
    }

    /// The picture of `which`: asked of the source the first time this frame, the same answer after.
    pub fn get(&self, which: OfficePicture) -> Option<Pixels> {
        if let Some(held) = self.seen.borrow().get(&which) {
            return held.clone();
        }
        let answer = (self.ask)(which);
        self.seen.borrow_mut().insert(which, answer.clone());
        answer
    }
}

/// A live scene's inputs: how it asks, the window's DPI (96 at a scale of 1) and the screen it draws on, in points.
pub struct Live<'a> {
    pub asks: &'a Asks<'a>,
    pub dpi: u16,
    pub screen: (f32, f32),
}

/// The pages whose pictures are asked: those on screen, one screen above and one screen below (1-based, first to
/// last), from the page in view (where the stage's viewport stands, `docs_scroll`).
pub fn asked_pages(doc: &DocBody, screen_h: f32) -> (u32, u32) {
    let pages = doc.view.pages.max(1);
    let first = doc.page.clamp(1, pages);
    let mut lo = first;
    let mut above = 0.0;
    while lo > 1 && above < screen_h {
        lo -= 1;
        above += box_pt(doc, lo).1 + PAGE_GAP;
    }
    let mut hi = first;
    let mut below = box_pt(doc, first).1 + PAGE_GAP;
    while hi < pages && below < 2.0 * screen_h {
        hi += 1;
        below += box_pt(doc, hi).1 + PAGE_GAP;
    }
    (lo, hi)
}

/// Page `n` drawn from the renderer, its box as the file sizes it. Below the tile scale one whole picture; at or above
/// it the tiles that touch the asked band, each at its own place (integer pixels over the device factor), over the
/// held page at the lowest scale while any tile of this scale is still on its way. `top`: where the page starts, in
/// points below the stage's top as it was last seen scrolled (negative above it). Its box never depends on whether a
/// picture has come, so an answer never moves the pages.
pub(super) fn paper_live(n: u32, doc: &DocBody, live: &Live, top: f32) -> El {
    let zoom = doc.view.zoom;
    let page0 = n - 1;
    if !uses_tiles(zoom, live.dpi) {
        return paper_box(
            n,
            doc,
            live.asks.get(OfficePicture::Page {
                page: page0,
                zoom_pct: zoom,
                dpi: live.dpi,
            }),
        );
    }
    let (w, h) = box_pt(doc, n);
    let device = f32::from(live.dpi) / 96.0;
    let (cols, rows) = (
        ((w * device).ceil() as u32).div_ceil(TILE_PX),
        ((h * device).ceil() as u32).div_ceil(TILE_PX),
    );
    // The band, in this page's own points from its top: one screen above the first page, one below the screen.
    let (band_top, band_bottom) = (-live.screen.1 - top, 2.0 * live.screen.1 - top);
    let side = TILE_PX as f32 / device;
    let mut page = shell(n, doc).h(h);
    let mut tiles = Vec::new();
    // Only the rows that touch the band: the page's size is the file's, so the loop never walks every row of it.
    let first = (band_top / side).floor().max(0.0) as u32;
    let last = ((band_bottom / side).ceil().max(0.0) as u32).min(rows);
    for row_ in first..last {
        for col_ in 0..cols {
            tiles.push((
                col_,
                row_,
                live.asks.get(OfficePicture::Tile {
                    page: page0,
                    zoom_pct: zoom,
                    dpi: live.dpi,
                    col: col_,
                    row: row_,
                }),
            ));
        }
    }
    if tiles.iter().any(|(_, _, p)| p.is_none()) {
        // No blank while they come: the page as held at the lowest scale, under them.
        let held = live.asks.get(OfficePicture::Page {
            page: page0,
            zoom_pct: 100,
            dpi: live.dpi,
        });
        if let Some(p) = held {
            page = page.child(
                el().key(format!("page-held-{n}"))
                    .absolute(0.0, 0.0)
                    .size(w, h)
                    .pixels_fit(&p, SurfaceFit::Fill),
            );
        }
    }
    for (col_, row_, pixels) in tiles {
        let Some(p) = pixels else { continue };
        let (pw, ph) = p.size();
        page = page.child(
            el().key(format!("page-{n}-tile-{col_}-{row_}"))
                .absolute((col_ * TILE_PX) as f32 / device, (row_ * TILE_PX) as f32 / device)
                .size(pw as f32 / device, ph as f32 / device)
                .pixels_fit(&p, SurfaceFit::Fill),
        );
    }
    page
}
