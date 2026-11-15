//! The checks the app runs on every answer from the Office helper before it uses it, and the limits they hold.
//!
//! Every function here is pure: the app passes what it asked and what came back, and gets `Ok(())` or a
//! [`Refused`] with a fixed reason. A refusal is a protocol error: the app kills the helper and shows "Renderer
//! stopped". Nothing the helper sent is kept in a refusal, so a reason can be logged without leaking content.

use std::fmt;
use std::time::Duration;

use crate::frame::Side;
use crate::office::{CellData, Cells, Opened, OpenedSheets, OutlineData, PageSize, Render, Rendered};

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
/// The most entries of one outline: 10,000.
pub const MAX_OUTLINE_ENTRIES: usize = 10_000;
/// The longest text of one outline entry: 1 KiB.
pub const MAX_OUTLINE_TEXT_BYTES: usize = 1024;
/// The most rows of one sheet: the .xlsx format's own maximum, 1,048,576.
pub const MAX_SHEET_ROWS: u32 = 1_048_576;
/// The most columns of one sheet: the .xlsx format's own maximum, 16,384.
pub const MAX_SHEET_COLS: u32 = 16_384;
/// The longest sheet name: 256 bytes (a spreadsheet allows 31 characters, at most 124 bytes of UTF-8).
pub const MAX_SHEET_NAME_BYTES: usize = 256;

/// Pixels per point at scale 1.0: 100 % zoom on a 96 DPI monitor.
pub const PX_PER_PT: f64 = 96.0 / 72.0;

/// The limits the checks hold. [`Limits::CONTRACT`] is the only value the app uses; tests may tighten it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub max_side_px: u32,
    pub max_pages: u32,
    pub max_sheets: usize,
    pub max_sheet_rows: u32,
    pub max_sheet_cols: u32,
    pub max_sheet_name_bytes: usize,
    pub max_cell_text_bytes: usize,
    pub max_outline_entries: usize,
    pub max_outline_text_bytes: usize,
}

impl Limits {
    /// The viewer contract's numbers.
    pub const CONTRACT: Limits = Limits {
        max_side_px: MAX_SIDE_PX,
        max_pages: MAX_PAGES,
        max_sheets: MAX_SHEETS,
        max_sheet_rows: MAX_SHEET_ROWS,
        max_sheet_cols: MAX_SHEET_COLS,
        max_sheet_name_bytes: MAX_SHEET_NAME_BYTES,
        max_cell_text_bytes: MAX_CELL_TEXT_BYTES,
        max_outline_entries: MAX_OUTLINE_ENTRIES,
        max_outline_text_bytes: MAX_OUTLINE_TEXT_BYTES,
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
    /// The app's `Render` cannot be sized: a scale or page size that is not finite and positive, or a region that is
    /// empty or not inside the page at that scale.
    BadAsk,
    /// The rendered width or height is not what was asked (beyond ±1 px of rounding), or is 0.
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
    /// More sheets than the limit.
    TooManySheets,
    /// A sheet with more rows or columns than the format allows.
    SheetTooLarge,
    /// A sheet name over the limit.
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
    /// More outline entries than the limit.
    TooManyEntries,
    /// An outline entry on a page the document does not have.
    PagePastEnd,
    /// An odd id: a request the helper opened, never an answer to the app.
    NotAnAnswer,
    /// An answer to an id the app has no request out for.
    UnsentId,
}

impl Refused {
    /// The fixed reason, for a log line.
    pub fn reason(self) -> &'static str {
        match self {
            Refused::BadAsk => "the render ask cannot be sized",
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
            Refused::TooManyEntries => "too many outline entries",
            Refused::PagePastEnd => "an outline entry past the last page",
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

/// The pixel size of a whole page at `scale`: `round(pt * scale * 96 / 72)` a side. `None` when the scale or the page
/// size is not finite and positive, or a side rounds below 1 px. The helper renders at exactly this size.
pub fn page_pixels(page: &PageSize, scale: f32) -> Option<(u32, u32)> {
    if !finite_positive(scale) || !finite_positive(page.w_pt) || !finite_positive(page.h_pt) {
        return None;
    }
    let side = |pt: f32| -> Option<u32> {
        let px = (f64::from(pt) * f64::from(scale) * PX_PER_PT).round();
        // In range, so the cast is exact.
        (1.0..=f64::from(u32::MAX)).contains(&px).then_some(px as u32)
    };
    Some((side(page.w_pt)?, side(page.h_pt)?))
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
/// of `out_len` bytes: each side within ±1 px of the ask and under the limit, and `w * h * 4 == out_len`.
pub fn check_rendered(
    asked: &Render,
    page: &PageSize,
    out_len: u64,
    got: &Rendered,
    limits: &Limits,
) -> Result<(), Refused> {
    let (want_w, want_h) = asked_size(asked, page, limits)?;
    if got.w > limits.max_side_px || got.h > limits.max_side_px {
        return Err(Refused::TooLarge);
    }
    if got.w == 0 || got.h == 0 {
        return Err(Refused::WrongSize);
    }
    if got.w.abs_diff(want_w) > 1 {
        return Err(Refused::WrongSize);
    }
    if got.h.abs_diff(want_h) > 1 {
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

/// An `OpenedSheets` answer: at most the sheet limit, every name under its cap, rows and columns under the format's.
pub fn check_sheets(got: &OpenedSheets, limits: &Limits) -> Result<(), Refused> {
    if got.sheets.len() > limits.max_sheets {
        return Err(Refused::TooManySheets);
    }
    for sheet in &got.sheets {
        if sheet.name.len() > limits.max_sheet_name_bytes {
            return Err(Refused::NameTooLong);
        }
        if sheet.rows > limits.max_sheet_rows || sheet.cols > limits.max_sheet_cols {
            return Err(Refused::SheetTooLarge);
        }
    }
    Ok(())
}

/// A `CellData` answer to `asked`: no more cells than the range holds, every cell inside it with its text under the
/// cap, every merge inside it, one finite non-negative width per column and height per row, frozen panes in a sheet.
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
    Ok(())
}

/// An `OutlineData` answer for a document of `pages` pages (from the checked `Opened`): at most the entry limit,
/// every text under its cap, every page one the document has.
pub fn check_outline(got: &OutlineData, pages: u32, limits: &Limits) -> Result<(), Refused> {
    if got.entries.len() > limits.max_outline_entries {
        return Err(Refused::TooManyEntries);
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
