//! The checks the app runs on every answer from the Office helper before it uses it, and the limits they hold.
//!
//! Every function here is pure: the app passes what it asked and what came back, and gets `Ok(())` or a
//! [`Refused`] with a fixed reason. A refusal is a protocol error: the app kills the helper and shows "Renderer
//! stopped". Nothing the helper sent is kept in a refusal, so a reason can be logged without leaking content.

use std::collections::BTreeSet;
use std::fmt;
use std::time::Duration;

use crate::frame::Side;
use crate::office::{
    Anchor, CellData, CellRange, Cells, Opened, OpenedSheets, Outline, OutlineData, PageSize, Render, RenderAnchor,
    Rendered,
};

// The viewer contract §5.2, one constant per number. MB and GB are binary here, as in the sandbox crate's 1 GiB.

/// The largest file the app hands to the viewer: 100 MB, checked by the app before it decrypts for the viewer.
pub const MAX_INPUT_BYTES: u64 = 100 * 1024 * 1024;
/// The largest unpacked size of a document's container: 512 MB (enforced inside the helper by the viewer).
pub const MAX_UNPACKED_BYTES: u64 = 512 * 1024 * 1024;
/// The most entries in a document's container: 10,000 (enforced inside the helper by the viewer).
pub const MAX_ENTRIES: u32 = 10_000;
/// The most pages or slides of one document: 2,000.
pub const MAX_PAGES: u32 = 2_000;
/// The most sheets of one workbook: 256.
pub const MAX_SHEETS: usize = 256;
/// The most non-empty cells of one workbook: 2,000,000 (enforced inside the helper by the viewer).
pub const MAX_CELLS: u32 = 2_000_000;
/// The largest side of one decoded image: 16,384 px (enforced inside the helper by the viewer).
pub const MAX_IMAGE_SIDE_PX: u32 = 16_384;
/// The most pixels of one decoded image: 64 megapixels (enforced inside the helper by the viewer).
pub const MAX_IMAGE_PIXELS: u64 = 64 * 1024 * 1024;
/// The largest side of one rendered page or tile: 4,096 px (64 MiB of RGBA8), in the app's ask and on the answer.
pub const MAX_SIDE_PX: u32 = 4_096;
/// The helper's committed memory: 1 GiB (the job object; the sandbox crate's `DEFAULT_MEMORY_LIMIT_BYTES`).
pub const HELPER_MEMORY_BYTES: u64 = 1024 * 1024 * 1024;
/// Open (parse and layout): 30 s of wall clock, the app's timer; then the helper is killed.
pub const OPEN_TIMEOUT: Duration = Duration::from_secs(30);
/// Any single request (one page, one cell range): 15 s of wall clock, the app's timer; then the helper is killed.
/// The contract's "10 s of CPU" per request has no constant: a job object's CPU limit is cumulative, so the wall timer
/// and the kill enforce it (office-viewer plan §2.2).
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

// Caps on what an answer may carry that §5.2 does not number.

/// The longest text of one cell: 32 KiB (a spreadsheet cell holds at most 32,767 characters).
pub const MAX_CELL_TEXT_BYTES: usize = 32 * 1024;
/// The most entries of one outline, over all its pages: 10,000.
pub const MAX_OUTLINE_ENTRIES: usize = 10_000;
/// The longest text of one outline entry: 1 KiB.
pub const MAX_OUTLINE_TEXT_BYTES: usize = 1024;
/// The most rows of one sheet: the .xlsx format's own maximum, 1,048,576.
pub const MAX_SHEET_ROWS: u32 = 1_048_576;
/// The most columns of one sheet: the .xlsx format's own maximum, 16,384.
pub const MAX_SHEET_COLS: u32 = 16_384;
/// The longest sheet name: 255 characters (Unicode scalar values, not bytes). A spreadsheet allows 31; the helper
/// cuts a longer name to this before it answers.
pub const MAX_SHEET_NAME_CHARS: usize = 255;
/// The longest formula of one cell: 8 KiB (a spreadsheet formula holds at most 8,192 characters). The helper cuts a
/// longer one, at a character boundary, before it answers.
pub const MAX_FORMULA_BYTES: usize = 8 * 1024;
/// The most charts and pictures in one block of cells. The helper drops those past it and says `missing_content`.
pub const MAX_ANCHORS: usize = 256;
/// The largest side of a chart or picture, and the farthest its corner may sit into its first cell: 20,000 pt (about
/// 7 m). The helper drops a larger one and says `missing_content`.
pub const MAX_ANCHOR_SIDE_PT: f32 = 20_000.0;

/// Points per inch. A `scale` is device pixels per point: zoom x DPI / 72 ([`render_scale`]).
pub const POINTS_PER_INCH: f32 = 72.0;

/// The tallest page height the Office engine sizes: it clamps a page's height to this before it multiplies by the
/// scale, and so does [`page_pixels`].
pub const MAX_PAGE_HEIGHT_PT: f32 = 1e6;

/// The limits the checks hold. [`Limits::CONTRACT`] is the only value the app uses; tests may tighten it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub max_side_px: u32,
    pub max_pages: u32,
    pub max_sheets: usize,
    pub max_sheet_rows: u32,
    pub max_sheet_cols: u32,
    pub max_sheet_name_chars: usize,
    pub max_cell_text_bytes: usize,
    pub max_outline_entries: usize,
    pub max_outline_text_bytes: usize,
    pub max_formula_bytes: usize,
    pub max_anchors: usize,
}

impl Limits {
    /// The viewer contract's numbers.
    pub const CONTRACT: Limits = Limits {
        max_side_px: MAX_SIDE_PX,
        max_pages: MAX_PAGES,
        max_sheets: MAX_SHEETS,
        max_sheet_rows: MAX_SHEET_ROWS,
        max_sheet_cols: MAX_SHEET_COLS,
        max_sheet_name_chars: MAX_SHEET_NAME_CHARS,
        max_cell_text_bytes: MAX_CELL_TEXT_BYTES,
        max_outline_entries: MAX_OUTLINE_ENTRIES,
        max_outline_text_bytes: MAX_OUTLINE_TEXT_BYTES,
        max_formula_bytes: MAX_FORMULA_BYTES,
        max_anchors: MAX_ANCHORS,
    };
}

impl Default for Limits {
    fn default() -> Self {
        Limits::CONTRACT
    }
}

/// Why an answer (or the app's own ask) was refused. Fixed reasons only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    /// The app's own ask cannot be sized or split: a `Render` scale or page size that is not finite and positive, a
    /// region that is empty or not inside the page at that scale, a `RenderAnchor` for another anchor than the one
    /// given or at a scale that sizes no raster, or a cell range whose half would lie past the last row or column a
    /// `u32` can name.
    BadAsk,
    /// The rendered width or height is not exactly what was asked, or is 0.
    WrongSize,
    /// A rendered side over the limit.
    TooLarge,
    /// The rendered `w * h * 4` is not the section's length.
    LengthMismatch,
    /// More pages than the limit.
    TooManyPages,
    /// The number of page sizes is not the page count.
    PageCountMismatch,
    /// A page size that is not finite and positive.
    BadPageSize,
    /// More sheets than the limit, the named ones and those counted in `more` together.
    TooManySheets,
    /// A sheet with more rows or columns than the format allows.
    SheetTooLarge,
    /// A sheet name over the limit of characters.
    NameTooLong,
    /// More cells than the asked range holds.
    TooManyCells,
    /// A cell outside the asked range.
    CellOutsideRange,
    /// A cell's or an outline entry's text over the limit.
    TextTooLong,
    /// A merged area that is empty or not inside the asked range.
    MergeOutsideRange,
    /// Not one column width per column and one row height per row of the asked range.
    SizesMismatch,
    /// A column width or row height that is not finite or is negative.
    BadCellSize,
    /// Frozen panes larger than a sheet can be.
    FrozenTooLarge,
    /// A cell's value that is not finite.
    BadValue,
    /// A cell's formula over the limit.
    FormulaTooLong,
    /// More charts and pictures in one block than the limit.
    TooManyAnchors,
    /// A chart or picture whose cells run backwards or past a sheet, or whose size or offset is not finite, not
    /// positive (size) or negative (offset), or over the limit.
    BadAnchor,
    /// Two charts or pictures with one id in a block.
    DuplicateAnchor,
    /// More outline entries than the limit, over all the pages so far or announced by `next`.
    TooManyEntries,
    /// An outline page whose `next` does not follow it: no entries, or not `from` plus the entries sent.
    BadNext,
    /// An outline entry on a page the document does not have.
    PagePastEnd,
    /// `RangeTooLarge` for a range of at most one cell, which always fits one frame: there is nothing to halve.
    CannotHalve,
    /// An odd id: a request the helper opened, never an answer to the app.
    NotAnAnswer,
    /// An answer to an id the app has no request out for.
    UnsentId,
}

impl Refused {
    /// The fixed reason, for a log line.
    pub fn reason(self) -> &'static str {
        match self {
            Refused::BadAsk => "the ask cannot be sized or split",
            Refused::WrongSize => "rendered size differs from the ask",
            Refused::TooLarge => "rendered side over the limit",
            Refused::LengthMismatch => "rendered size does not fill the section",
            Refused::TooManyPages => "too many pages",
            Refused::PageCountMismatch => "page sizes do not match the page count",
            Refused::BadPageSize => "a page size that is not finite and positive",
            Refused::TooManySheets => "too many sheets",
            Refused::SheetTooLarge => "a sheet larger than the format allows",
            Refused::NameTooLong => "a sheet name over the limit",
            Refused::TooManyCells => "more cells than the range holds",
            Refused::CellOutsideRange => "a cell outside the asked range",
            Refused::TextTooLong => "text over the limit",
            Refused::MergeOutsideRange => "a merge outside the asked range",
            Refused::SizesMismatch => "column widths or row heights do not match the range",
            Refused::BadCellSize => "a column width or row height that is not finite or is negative",
            Refused::FrozenTooLarge => "frozen panes larger than a sheet",
            Refused::BadValue => "a cell value that is not finite",
            Refused::FormulaTooLong => "a formula over the limit",
            Refused::TooManyAnchors => "too many charts and pictures",
            Refused::BadAnchor => "a chart or picture that cannot be placed",
            Refused::DuplicateAnchor => "two charts or pictures with one id",
            Refused::TooManyEntries => "too many outline entries",
            Refused::BadNext => "an outline page whose next entry does not follow it",
            Refused::PagePastEnd => "an outline entry past the last page",
            Refused::CannotHalve => "a range too large to send that cannot be halved",
            Refused::NotAnAnswer => "a helper request where an answer was due",
            Refused::UnsentId => "an answer to a request never sent",
        }
    }
}

impl fmt::Display for Refused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.reason())
    }
}

impl std::error::Error for Refused {}

fn finite_positive(v: f32) -> bool {
    v.is_finite() && v > 0.0
}

/// The `scale` the app asks a page at: device pixels per point, `zoom * dpi / 72`. 100 % zoom on a 96 DPI monitor is
/// 4/3; on a 144 DPI monitor, 2.
pub fn render_scale(zoom: f32, dpi: f32) -> f32 {
    zoom * dpi / POINTS_PER_INCH
}

/// The pixel size of a whole page at `scale` (device pixels per point): `ceil(pt * scale)` a side, at least 1 px, the
/// product taken in `f32`. This is the Office engine's own rule (the expressions its renderer sizes a raster with),
/// so the app and the helper size the same raster; the height is first clamped to [`MAX_PAGE_HEIGHT_PT`], as the
/// engine does. `None` when the scale or the page size is not finite and positive, or a side does not fit a `u32`.
/// The helper renders at exactly this size; `tests/page_pixels_vectors.txt` pins it, the same file in both repos.
pub fn page_pixels(page: &PageSize, scale: f32) -> Option<(u32, u32)> {
    if !finite_positive(scale) || !finite_positive(page.w_pt) || !finite_positive(page.h_pt) {
        return None;
    }
    Some((
        side_pixels(page.w_pt, scale)?,
        side_pixels(page.h_pt.min(MAX_PAGE_HEIGHT_PT), scale)?,
    ))
}

/// One side of the raster: `ceil(pt * scale)`, at least 1 px, the product in `f32`; `None` past a `u32`. The one rule
/// for a page ([`page_pixels`]) and an anchor ([`anchor_pixels`]).
fn side_pixels(pt: f32, scale: f32) -> Option<u32> {
    // In f32, as the engine: in f64 a Letter page at 4/3 would be 817 px wide instead of the engine's 816.
    let px = (pt * scale).ceil().max(1.0);
    // Below 2^32 and a whole number, so the cast is exact.
    (px.is_finite() && px < 4_294_967_296.0).then_some(px as u32)
}

/// The pixel size of a chart or picture at `scale`: `ceil(w_pt * scale)` x `ceil(h_pt * scale)`, at least 1 px a
/// side, the product in `f32`: the page rule ([`page_pixels`]) without the page's height clamp, and the Office
/// engine's own (`Workbook::anchor_size`). `None` when the scale or the anchor's size is not finite and positive, or a
/// side does not fit a `u32`. `tests/page_pixels_vectors.txt` pins it too.
pub fn anchor_pixels(anchor: &Anchor, scale: f32) -> Option<(u32, u32)> {
    if !finite_positive(scale) || !finite_positive(anchor.w_pt) || !finite_positive(anchor.h_pt) {
        return None;
    }
    Some((side_pixels(anchor.w_pt, scale)?, side_pixels(anchor.h_pt, scale)?))
}

/// The pixel size a `RenderAnchor` asks for, of `anchor` (from the checked `CellData` the app holds). The app sizes the
/// section `out` as `w * h * 4` from this, before it sends the ask. Refuses an ask for another anchor than `anchor`,
/// one that cannot be sized, and one over the limit.
pub fn asked_anchor_size(asked: &RenderAnchor, anchor: &Anchor, limits: &Limits) -> Result<(u32, u32), Refused> {
    if asked.anchor != anchor.id {
        return Err(Refused::BadAsk);
    }
    let (w, h) = anchor_pixels(anchor, asked.scale).ok_or(Refused::BadAsk)?;
    if w > limits.max_side_px || h > limits.max_side_px {
        return Err(Refused::TooLarge);
    }
    Ok((w, h))
}

/// A `Rendered` answer to a `RenderAnchor`, whose pixels are in a section of `out_len` bytes: each side exactly the
/// ask ([`asked_anchor_size`]) and under the limit, and `w * h * 4 == out_len`.
pub fn check_rendered_anchor(
    asked: &RenderAnchor,
    anchor: &Anchor,
    out_len: u64,
    got: &Rendered,
    limits: &Limits,
) -> Result<(), Refused> {
    let want = asked_anchor_size(asked, anchor, limits)?;
    check_rendered_size(want, out_len, got, limits)
}

/// The pixel size a `Render` asks for: its region, or the whole page at its scale. The app sizes the section `out`
/// as `w * h * 4` from this, before it sends the ask. Refuses an ask that cannot be sized or is over the limit.
pub fn asked_size(asked: &Render, page: &PageSize, limits: &Limits) -> Result<(u32, u32), Refused> {
    let (page_w, page_h) = page_pixels(page, asked.scale).ok_or(Refused::BadAsk)?;
    let (w, h) = match asked.region {
        None => (page_w, page_h),
        Some(r) => {
            let inside = r.w > 0
                && r.h > 0
                && u64::from(r.x) + u64::from(r.w) <= u64::from(page_w)
                && u64::from(r.y) + u64::from(r.h) <= u64::from(page_h);
            if !inside {
                return Err(Refused::BadAsk);
            }
            (r.w, r.h)
        }
    };
    if w > limits.max_side_px || h > limits.max_side_px {
        return Err(Refused::TooLarge);
    }
    Ok((w, h))
}

/// A `Rendered` answer to `asked`, for a page of size `page` (from the checked `Opened`), whose pixels are in a section
/// of `out_len` bytes: each side exactly the ask and under the limit, and `w * h * 4 == out_len`. There is no rounding
/// slack: the app and the helper size the raster by the same rule ([`page_pixels`]), so any other size is a fault.
pub fn check_rendered(
    asked: &Render,
    page: &PageSize,
    out_len: u64,
    got: &Rendered,
    limits: &Limits,
) -> Result<(), Refused> {
    let want = asked_size(asked, page, limits)?;
    check_rendered_size(want, out_len, got, limits)
}

/// A rendered raster against the size asked: under the limit, not empty, each side exactly the ask, filling the
/// section.
fn check_rendered_size(
    (want_w, want_h): (u32, u32),
    out_len: u64,
    got: &Rendered,
    limits: &Limits,
) -> Result<(), Refused> {
    if got.w > limits.max_side_px || got.h > limits.max_side_px {
        return Err(Refused::TooLarge);
    }
    if got.w == 0 || got.h == 0 {
        return Err(Refused::WrongSize);
    }
    if got.w != want_w {
        return Err(Refused::WrongSize);
    }
    if got.h != want_h {
        return Err(Refused::WrongSize);
    }
    if u64::from(got.w) * u64::from(got.h) * 4 != out_len {
        return Err(Refused::LengthMismatch);
    }
    Ok(())
}

/// An `Opened` answer: at most the page limit, one size per page, every size finite and positive.
pub fn check_opened(got: &Opened, limits: &Limits) -> Result<(), Refused> {
    if got.pages > limits.max_pages {
        return Err(Refused::TooManyPages);
    }
    if got.sizes.len() as u64 != u64::from(got.pages) {
        return Err(Refused::PageCountMismatch);
    }
    if got
        .sizes
        .iter()
        .any(|s| !finite_positive(s.w_pt) || !finite_positive(s.h_pt))
    {
        return Err(Refused::BadPageSize);
    }
    Ok(())
}

/// An `OpenedSheets` answer: at most the sheet limit, the named sheets and those counted in `more` together; every
/// name at most its cap in characters; rows and columns under the format's.
pub fn check_sheets(got: &OpenedSheets, limits: &Limits) -> Result<(), Refused> {
    if got.sheets.len() as u64 + u64::from(got.more) > limits.max_sheets as u64 {
        return Err(Refused::TooManySheets);
    }
    for sheet in &got.sheets {
        if sheet.name.chars().count() > limits.max_sheet_name_chars {
            return Err(Refused::NameTooLong);
        }
        if sheet.rows > limits.max_sheet_rows || sheet.cols > limits.max_sheet_cols {
            return Err(Refused::SheetTooLarge);
        }
    }
    Ok(())
}

/// A `CellData` answer to `asked`: no more cells than the range holds, every cell inside it with its text and its
/// formula under their caps and its value finite, every merge inside it, one finite non-negative width per column and
/// height per row, frozen panes in a sheet, at most [`MAX_ANCHORS`] charts and pictures, each [`anchor_placeable`]
/// and each with its own id.
pub fn check_cells(asked: &Cells, got: &CellData, limits: &Limits) -> Result<(), Refused> {
    let range = &asked.range;
    if got.cells.len() as u64 > range.area() {
        return Err(Refused::TooManyCells);
    }
    for cell in &got.cells {
        if !range.contains(cell.row, cell.col) {
            return Err(Refused::CellOutsideRange);
        }
        if cell.text.len() > limits.max_cell_text_bytes {
            return Err(Refused::TextTooLong);
        }
        if cell.value.is_some_and(|v| !v.is_finite()) {
            return Err(Refused::BadValue);
        }
        if cell
            .formula
            .as_ref()
            .is_some_and(|f| f.len() > limits.max_formula_bytes)
        {
            return Err(Refused::FormulaTooLong);
        }
    }
    if got.merges.iter().any(|m| !range.holds(m)) {
        return Err(Refused::MergeOutsideRange);
    }
    if got.col_widths.len() as u64 != u64::from(range.cols) || got.row_heights.len() as u64 != u64::from(range.rows) {
        return Err(Refused::SizesMismatch);
    }
    if got
        .col_widths
        .iter()
        .chain(&got.row_heights)
        .any(|v| !v.is_finite() || *v < 0.0)
    {
        return Err(Refused::BadCellSize);
    }
    if got.frozen.rows > limits.max_sheet_rows || got.frozen.cols > limits.max_sheet_cols {
        return Err(Refused::FrozenTooLarge);
    }
    check_anchors(&got.anchors, limits)
}

/// The charts and pictures of a block: at most the limit, each one [`anchor_placeable`], every id its own.
fn check_anchors(anchors: &[Anchor], limits: &Limits) -> Result<(), Refused> {
    if anchors.len() > limits.max_anchors {
        return Err(Refused::TooManyAnchors);
    }
    let mut ids = BTreeSet::new();
    for a in anchors {
        if !anchor_placeable(a, limits) {
            return Err(Refused::BadAnchor);
        }
        if !ids.insert(a.id) {
            return Err(Refused::DuplicateAnchor);
        }
    }
    Ok(())
}

/// Whether the app's check accepts one chart or picture where a sheet places it: its cells in order (`from` at or
/// before `to` on both axes) and inside a sheet the format allows; its size finite, positive and at most
/// [`MAX_ANCHOR_SIDE_PT`]; its offset into the `from` cell finite, not negative and at most the same. The helper drops
/// an anchor this refuses, so an honest helper is never refused.
pub fn anchor_placeable(a: &Anchor, limits: &Limits) -> bool {
    let size = |v: f32| finite_positive(v) && v <= MAX_ANCHOR_SIDE_PT;
    let offset = |v: f32| v.is_finite() && (0.0..=MAX_ANCHOR_SIDE_PT).contains(&v);
    a.from_row <= a.to_row
        && a.from_col <= a.to_col
        && a.to_row < limits.max_sheet_rows
        && a.to_col < limits.max_sheet_cols
        && size(a.w_pt)
        && size(a.h_pt)
        && offset(a.x_pt)
        && offset(a.y_pt)
}

/// One page of the outline, the answer to `asked`, for a document of `pages` pages (from the checked `Opened`).
///
/// The entries up to the end of this page (`asked.from` plus the entries here) stay within the entry limit. When
/// there is more (`next` is `Some`), the page holds at least one entry, `next` is the entry right after it, and the
/// limit leaves room for that entry: so every page moves the app forward, and its asking ends. Every text is under its
/// cap and every page one the document has.
pub fn check_outline(asked: &Outline, got: &OutlineData, pages: u32, limits: &Limits) -> Result<(), Refused> {
    let end = u64::from(asked.from) + got.entries.len() as u64;
    let cap = limits.max_outline_entries as u64;
    if end > cap {
        return Err(Refused::TooManyEntries);
    }
    if let Some(next) = got.next {
        if end >= cap {
            return Err(Refused::TooManyEntries);
        }
        if got.entries.is_empty() {
            return Err(Refused::BadNext);
        }
        if u64::from(next) != end {
            return Err(Refused::BadNext);
        }
    }
    for entry in &got.entries {
        if entry.text.len() > limits.max_outline_text_bytes {
            return Err(Refused::TextTooLong);
        }
        if entry.page >= pages {
            return Err(Refused::PagePastEnd);
        }
    }
    Ok(())
}

/// The app's answer to [`RangeTooLarge`](crate::office::ErrorCode::RangeTooLarge) for `asked`: the two halves to ask
/// instead, split across the longer side (rows when there are at least as many rows as columns), the first half the
/// smaller one when that side is odd. Each half is smaller than the range, so halving again ends.
///
/// Refused ([`Refused::CannotHalve`]) for a range of at most one cell: one cell always fits a frame (its text, its
/// formula and the block's charts and pictures are capped: [`MAX_CELL_TEXT_BYTES`], [`MAX_FORMULA_BYTES`] and
/// [`MAX_ANCHORS`] together stay under one frame), so that answer is the helper's error.
pub fn halve_range(asked: &Cells) -> Result<[Cells; 2], Refused> {
    let r = asked.range;
    if r.area() <= 1 {
        return Err(Refused::CannotHalve);
    }
    let (first, second) = if r.rows >= r.cols {
        let top = r.rows / 2;
        let row = r.row.checked_add(top).ok_or(Refused::BadAsk)?;
        (
            CellRange { rows: top, ..r },
            CellRange {
                row,
                rows: r.rows - top,
                ..r
            },
        )
    } else {
        let left = r.cols / 2;
        let col = r.col.checked_add(left).ok_or(Refused::BadAsk)?;
        (
            CellRange { cols: left, ..r },
            CellRange {
                col,
                cols: r.cols - left,
                ..r
            },
        )
    };
    Ok([
        Cells {
            sheet: asked.sheet,
            range: first,
        },
        Cells {
            sheet: asked.sheet,
            range: second,
        },
    ])
}

/// A frame the app reads as an answer: its id must be an app id (even) that `was_sent` reports outstanding.
/// Use: `check_answer_id(frame.id, |id| pending.contains_key(&id))`.
pub fn check_answer_id(id: u32, was_sent: impl FnOnce(u32) -> bool) -> Result<(), Refused> {
    if Side::of(id) != Side::App {
        return Err(Refused::NotAnAnswer);
    }
    if !was_sent(id) {
        return Err(Refused::UnsentId);
    }
    Ok(())
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod anchor_tests;
