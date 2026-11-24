//! The document's continuous scroll (lead ruling 2026-11-16, "the scroll container decision"): the model the in-app PDF
//! viewer uses. The stage is a Rune scroll viewport (`El::scroll` + `El::scroll_xy`, as `viewers::pdf`'s page area), so
//! the wheel, a touchpad and Shift+wheel move it with Rune's own momentum; `rune-kit`'s scroll bar sits on the same
//! viewport and drags its offset (the PDF viewer draws no bar of its own: Rune's viewport has none, so the bar is the
//! kit's). Keys, a heading in the outline and the zoom move the same viewport (`Tx::scroll_key_to`), never a second offset.
//!
//! Where the viewport stands comes back into the document's state as the pages' column moves (`El::on_layout_tx`): the
//! page in view and the scroll offset, which the asks band uses (`docs_live`). The view runs again only when the page in
//! view changes or the offset has moved by [`SCROLL_STEP`] since the last run; every other frame of a scroll runs none.

use std::rc::Rc;

use rune::{El, KeyEvent, ScrollSpec, Tx, col, row};
use rune_kit::{ScrollBar, ScrollBarStyle};

use beebeeb_office_protocol::office::PageRect;

use super::{DocBody, PAGE_GAP, box_pt, size_pt};
use crate::State;
use crate::frames::viewers;
use crate::theme::tokens::Theme;

/// The stage's key: the scroll viewport the pages sit in.
pub const STAGE: &str = "stage";
/// The room above the first page (the pages' column's top padding); a page gone to stands this far under the top.
pub const PAD_TOP: f32 = 24.0;
/// How far the stage scrolls before the view runs again (the asks band is a screen wide either way, so this is fine
/// grain for it).
pub const SCROLL_STEP: f32 = 64.0;

/// Page `n`'s key: page 1 is `page`, a later page `page-{n}`.
pub fn page_key(n: u32) -> String {
    if n == 1 {
        "page".to_string()
    } else {
        format!("page-{n}")
    }
}

/// The page in view at scroll offset `y`: the last page whose top is at or above the stage's top padding. `heights`:
/// each page's height as drawn, in page order.
pub fn page_at(heights: &[f32], y: f32) -> u32 {
    let mut page = 1;
    let mut top = PAD_TOP;
    for (i, h) in heights.iter().enumerate() {
        // A point of slack: the layout puts a page on whole pixels, the arithmetic does not.
        if top > y + PAD_TOP + 1.0 {
            break;
        }
        page = i + 1;
        top += h + PAGE_GAP;
    }
    u32::try_from(page).unwrap_or(u32::MAX)
}

impl DocBody {
    /// Where page `n`'s top is in the stage's content, in points, from the pages' boxes (the top padding included).
    pub fn page_top(&self, n: u32) -> f32 {
        PAD_TOP
            + (1..n.clamp(1, self.view.pages.max(1)))
                .map(|k| box_pt(self, k).1 + PAGE_GAP)
                .sum::<f32>()
    }

    /// Layout points per point of page `n` (the page's box over the file's size).
    pub fn page_scale(&self, n: u32) -> f32 {
        let n = n.clamp(1, self.view.pages.max(1));
        box_pt(self, n).0 / size_pt(self, n).0
    }

    /// A point of the pages' column (layout points from its top-left, the top padding included) as a point of a page:
    /// the page (0-based; the gap between two pages goes to the nearer one, above the first and below the last to
    /// those), then x and y in the page's points, kept on the page.
    pub fn page_point(&self, x: f32, y: f32) -> (u32, f32, f32) {
        let count = self.view.pages.max(1);
        let (x, y) = (if x.is_finite() { x } else { 0.0 }, if y.is_finite() { y } else { 0.0 });
        let (mut n, mut top) = (1, PAD_TOP);
        while n < count {
            let foot = top + box_pt(self, n).1;
            if y < foot + PAGE_GAP / 2.0 {
                break;
            }
            top = foot + PAGE_GAP;
            n += 1;
        }
        let (w_pt, h_pt) = size_pt(self, n);
        let k = self.page_scale(n);
        (n - 1, (x / k).clamp(0.0, w_pt), ((y - top) / k).clamp(0.0, h_pt))
    }

    /// A rectangle of a page, as `[x, y, w, h]` in layout points of the pages' column (the inverse of
    /// [`Self::page_point`]). A page the document does not have is its last.
    pub fn page_rect(&self, r: &PageRect) -> [f32; 4] {
        let n = r.page.saturating_add(1).clamp(1, self.view.pages.max(1));
        let k = self.page_scale(n);
        [r.rect.x * k, self.page_top(n) + r.rect.y * k, r.rect.w * k, r.rect.h * k]
    }

    /// The offset the asks band reads: where the stage was last seen scrolled to, or, when that is not where the page in
    /// view stands (a body that came in with the page carried over), that page's top.
    pub fn view_y(&self) -> f32 {
        let heights: Vec<f32> = (1..=self.view.pages.max(1)).map(|n| box_pt(self, n).1).collect();
        if page_at(&heights, self.scroll_y) == self.page {
            self.scroll_y
        } else {
            self.page_top(self.page) - PAD_TOP
        }
    }

    /// The stage was scrolled to `y` (points from the content's top). `true`: the page in view or the band moved, and
    /// the view runs again; `false`: nothing the view draws changed.
    pub fn scrolled_to(&mut self, y: f32, heights: &[f32]) -> bool {
        let y = if y.is_finite() { y.max(0.0) } else { 0.0 };
        let page = page_at(heights, y);
        if page == self.page && (y - self.scroll_y).abs() < SCROLL_STEP {
            return false;
        }
        self.page = page;
        self.scroll_y = y;
        true
    }
}

/// After a key, a heading or the zoom changed the page in view: the stage shows that page from its top. The page is in
/// the commit this causes (every page keeps its box), so the viewport lands on it in that frame.
pub fn show_page(doc: &DocBody, tx: &mut Tx) {
    tx.scroll_key_to(page_key(doc.page), PAD_TOP);
}

/// A press that changes the document and then shows its page (the outline's headings, the zoom buttons).
pub fn with_doc_tx(act: impl Fn(&mut DocBody) + 'static) -> impl Fn(&mut State, &mut Tx) + 'static {
    move |s: &mut State, tx: &mut Tx| {
        if let Some(doc) = viewers::doc_mut(s) {
            act(doc);
            show_page(doc, tx);
        }
    }
}

/// The document's keys ([`DocBody::on_key`]), then the stage follows the page they went to. `true`: taken.
pub fn on_key(s: &mut State, ev: &KeyEvent, tx: &mut Tx) -> bool {
    // An editor with nothing else focused takes its keys at the window too (the surface takes them when it is focused).
    if tx.focused().is_none() && super::super::edit::word::on_key(s, tx, ev) {
        return true;
    }
    let Some(doc) = viewers::doc_mut(s) else {
        return false;
    };
    if !doc.on_key(&ev.key, &ev.mods) {
        return false;
    }
    show_page(doc, tx);
    true
}

/// The stage: the pages' column in a viewport that scrolls both ways, with the kit's scroll bar on it. `stage_y`: the
/// stage's top in the window as the last frame laid it out (`None` on a first build: nothing to measure from yet);
/// `heights`: each page's height as drawn, which turns an offset into the page in view.
pub fn viewport(pages: El, stage_y: Option<f32>, heights: Rc<[f32]>, t: &Theme) -> El {
    let mut pages = pages;
    if let Some(top) = stage_y {
        // The column is the viewport's content: its top in the window is the stage's top less the offset.
        pages = pages.on_layout_tx(move |s: &mut State, r: [f32; 4], tx: &mut Tx| {
            let moved = viewers::doc_mut(s).is_some_and(|d| d.scrolled_to(top - r[1], &heights));
            if !moved {
                tx.live_only();
            }
        });
    }
    col()
        .key(STAGE)
        .role("group")
        .label("Document")
        .grow(1.0)
        .min_w(0.0)
        .min_h(0.0)
        .h_full()
        .clip_rounded(0.0)
        .fill(super::super::states::stage_fill(t))
        .scroll(ScrollSpec { stick_bottom: false })
        .scroll_xy()
        // The pages are centred while they fit; wider than the stage (a zoom past its width) they start at its left
        // edge, where scrolling reaches all of them (the PDF viewer's page holder).
        .child(row().key("pages-holder").w_full().justify_center().child(pages))
        .scroll_bar_styled(ScrollBarStyle::neutral(t.dark).thumb(t.scrollbar))
}

#[cfg(test)]
#[path = "docs_scroll_tests.rs"]
mod tests;
