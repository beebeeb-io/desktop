//! Every answer fits one frame, and holds only what the app's checks accept.
//!
//! There are no continuation frames (`beebeeb_office_protocol::frame::fits_one_frame`), so the answers that can grow
//! say so instead: the outline comes in pages that advance, a cell range too large to send is `RangeTooLarge`, and
//! the sheets that do not fit are counted in `more`. Sizes here are counted the way each message encodes; the
//! encoding itself stays the last word ([`crate::serve`] never writes a message that does not encode).

use std::collections::BTreeSet;

use beebeeb_office_protocol::check::{
    Limits, MAX_ANCHORS, MAX_CELL_TEXT_BYTES, MAX_FORMULA_BYTES, MAX_OUTLINE_ENTRIES, MAX_OUTLINE_TEXT_BYTES,
    MAX_SHEET_NAME_CHARS, MAX_SHEETS, anchor_placeable,
};
use beebeeb_office_protocol::frame::{MAX_PAYLOAD, fits_one_frame};
use beebeeb_office_protocol::office::{CellData, CellRange, OpenedSheets, OutlineData, OutlineEntry, SheetInfo};

/// Bytes of a `u32` on the wire: a count, a length, a number.
const U32: usize = 4;

/// Cuts `text` to at most `max` bytes, at a character boundary.
fn cut_to_bytes(text: &mut String, max: usize) {
    if text.len() <= max {
        return;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
}

/// Cuts `text` to at most `max` characters (Unicode scalar values).
fn cut_to_chars(text: &mut String, max: usize) {
    if let Some((at, _)) = text.char_indices().nth(max) {
        text.truncate(at);
    }
}

/// The document's outline as the helper keeps it: at most the protocol's entry count, each text cut to its cap.
pub(crate) fn outline_entries(mut all: Vec<OutlineEntry>) -> Vec<OutlineEntry> {
    all.truncate(MAX_OUTLINE_ENTRIES);
    for entry in &mut all {
        cut_to_bytes(&mut entry.text, MAX_OUTLINE_TEXT_BYTES);
    }
    all
}

/// One outline entry on the wire: its level, its text's length and bytes, its page.
fn entry_bytes(entry: &OutlineEntry) -> usize {
    1 + U32 + entry.text.len() + U32
}

/// The outline page that answers `Outline { from }`: the entries from `from` on, as many as fit one frame.
///
/// - The rest of the outline fits: all of it, `next: None`.
/// - It does not: the largest prefix that fits with a `next`, and `next = from + n`. `n` is at least 1, because every
///   text was cut to 1 KiB ([`outline_entries`]), so a page always moves the app forward.
/// - `from` at or past the end: no entries, `next: None`.
pub(crate) fn outline_page(all: &[OutlineEntry], from: u32) -> OutlineData {
    let start = usize::try_from(from).unwrap_or(usize::MAX);
    let Some(rest) = all.get(start..).filter(|rest| !rest.is_empty()) else {
        return OutlineData {
            entries: Vec::new(),
            next: None,
        };
    };

    // All of it: the count, the entries, the `None` tag.
    let mut used = U32 + 1;
    let mut all_fit = true;
    for entry in rest {
        used = used.saturating_add(entry_bytes(entry));
        if !fits_one_frame(used) {
            all_fit = false;
            break;
        }
    }
    if all_fit {
        return OutlineData {
            entries: rest.to_vec(),
            next: None,
        };
    }

    // A prefix: the count, the entries, the `Some` tag and the next entry's number.
    let mut used = U32 + 1 + U32;
    let mut n = 0usize;
    for entry in rest {
        let size = entry_bytes(entry);
        if !fits_one_frame(used.saturating_add(size)) {
            break;
        }
        used += size;
        n += 1;
    }
    // Cut texts make n at least 1; the floor keeps `next` ahead of `from` whatever the entries hold.
    let n = n.max(1);
    // n is at most the outline's 10,000 entries.
    let next = from.saturating_add(n as u32);
    OutlineData {
        entries: rest[..n].to_vec(),
        next: Some(next),
    }
}

/// The sheets that answer `Open` for a workbook: at most the protocol's sheet count in all, each name cut to its
/// cap in characters, the largest prefix that fits one frame named, the rest counted in `more`.
pub(crate) fn sheets(mut all: Vec<SheetInfo>) -> OpenedSheets {
    all.truncate(MAX_SHEETS);
    let total = all.len();

    // The count and `more`.
    let mut used = U32 + U32;
    let mut named = 0usize;
    for sheet in &mut all {
        cut_to_chars(&mut sheet.name, MAX_SHEET_NAME_CHARS);
        // The name's length and bytes, its rows, its columns.
        let size = U32 + sheet.name.len() + U32 + U32;
        if !fits_one_frame(used.saturating_add(size)) {
            break;
        }
        used += size;
        named += 1;
    }
    all.truncate(named);
    let mut answer = OpenedSheets {
        sheets: all,
        // At most MAX_SHEETS (256).
        more: (total - named) as u32,
    };
    // The encoding is the last word: should the count above ever disagree with it, name fewer sheets.
    while answer.encode().is_err() && answer.sheets.pop().is_some() {
        answer.more += 1;
    }
    answer
}

/// The bytes of a `CellData` that are there whatever the range holds: the counts of cells, widths, heights, merges
/// and anchors, the frozen panes, `missing_content`.
const CELL_DATA_FIXED: usize = 5 * U32 + 2 * U32 + 1;

/// Whether the sizes alone of `range` (one width per column, one height per row) and the block's fixed fields can fit
/// one frame. When they cannot, the range is `RangeTooLarge` without asking the engine.
pub(crate) fn range_sizes_fit(range: &CellRange) -> bool {
    (u64::from(range.rows) + u64::from(range.cols)) * U32 as u64 + CELL_DATA_FIXED as u64 <= MAX_PAYLOAD as u64
}

/// The engine's cells for `asked`, as the app's checks accept them: every merge clipped to the asked range (a merge
/// outside it dropped), the frozen panes cut to the range's end, every cell's text and formula cut to their caps, a value
/// that is not finite dropped, and the charts and pictures held to what the app takes: those it would refuse
/// (`check::anchor_placeable`), a second one with an id already given, and those past the most a block carries are
/// dropped. Whatever is dropped, the block says `missing_content`, so the status line tells the user.
pub(crate) fn cell_data(mut data: CellData, asked: &CellRange) -> CellData {
    data.merges = data.merges.iter().filter_map(|m| clip(m, asked)).collect();
    // The sheet's frozen panes reach no further than the cells asked (the app refuses more): a block near the top says
    // fewer than a sheet with deep panes has, and the app keeps the largest it was told.
    data.frozen.rows = data.frozen.rows.min(asked.row.saturating_add(asked.rows));
    data.frozen.cols = data.frozen.cols.min(asked.col.saturating_add(asked.cols));
    for cell in &mut data.cells {
        cut_to_bytes(&mut cell.text, MAX_CELL_TEXT_BYTES);
        if let Some(formula) = &mut cell.formula {
            cut_to_bytes(formula, MAX_FORMULA_BYTES);
        }
        if cell.value.is_some_and(|v| !v.is_finite()) {
            cell.value = None;
            data.missing_content = true;
        }
    }
    let given = data.anchors.len();
    let mut ids = BTreeSet::new();
    data.anchors
        .retain(|a| anchor_placeable(a, &Limits::CONTRACT) && ids.insert(a.id));
    data.anchors.truncate(MAX_ANCHORS);
    if data.anchors.len() < given {
        data.missing_content = true;
    }
    data
}

/// The part of `merge` inside `asked`, or `None` when they do not overlap.
fn clip(merge: &CellRange, asked: &CellRange) -> Option<CellRange> {
    let top = u64::from(merge.row).max(u64::from(asked.row));
    let left = u64::from(merge.col).max(u64::from(asked.col));
    let bottom = (u64::from(merge.row) + u64::from(merge.rows)).min(u64::from(asked.row) + u64::from(asked.rows));
    let right = (u64::from(merge.col) + u64::from(merge.cols)).min(u64::from(asked.col) + u64::from(asked.cols));
    if bottom <= top || right <= left {
        return None;
    }
    // Each value lies inside `asked`, whose corners are u32 values and whose sides are u32 counts.
    Some(CellRange {
        row: u32::try_from(top).ok()?,
        col: u32::try_from(left).ok()?,
        rows: u32::try_from(bottom - top).ok()?,
        cols: u32::try_from(right - left).ok()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cut_keeps_whole_characters() {
        let mut text = "aé€".repeat(3); // 1 + 2 + 3 bytes, three times: 18 bytes, 9 characters
        cut_to_bytes(&mut text, 5);
        assert_eq!(text, "aé");
        let mut text = "aé€".repeat(3);
        cut_to_chars(&mut text, 4);
        assert_eq!(text, "aé€a");
        let mut short = String::from("ab");
        cut_to_chars(&mut short, 4);
        cut_to_bytes(&mut short, 4);
        assert_eq!(short, "ab");
    }

    #[test]
    fn a_merge_is_clipped_to_the_range() {
        let asked = CellRange {
            row: 10,
            col: 10,
            rows: 5,
            cols: 5,
        };
        let straddles = CellRange {
            row: 8,
            col: 12,
            rows: 4,
            cols: 10,
        };
        assert_eq!(
            clip(&straddles, &asked),
            Some(CellRange {
                row: 10,
                col: 12,
                rows: 2,
                cols: 3
            })
        );
        let outside = CellRange {
            row: 0,
            col: 0,
            rows: 10,
            cols: 10,
        };
        assert_eq!(clip(&outside, &asked), None, "touching the corner is not overlapping");
        let inside = CellRange {
            row: 11,
            col: 11,
            rows: 2,
            cols: 2,
        };
        assert_eq!(clip(&inside, &asked), Some(inside));
    }

    #[test]
    fn frozen_panes_are_cut_to_the_asked_range() {
        use beebeeb_office_protocol::check::{Limits, check_cells};
        use beebeeb_office_protocol::office::{Cells, Frozen};
        // Rows 5 to 14 and columns 5 to 8 asked: the range ends after row 15 and column 9.
        let asked = CellRange {
            row: 5,
            col: 5,
            rows: 10,
            cols: 4,
        };
        let block = |rows, cols| CellData {
            cells: Vec::new(),
            col_widths: vec![48.0; 4],
            row_heights: vec![15.0; 10],
            merges: Vec::new(),
            frozen: Frozen { rows, cols },
            anchors: Vec::new(),
            missing_content: false,
        };
        // A workbook's sheet-sized panes: cut, so the app's check takes the answer.
        let cut = cell_data(block(1_048_576, 16_384), &asked);
        assert_eq!(cut.frozen, Frozen { rows: 15, cols: 9 });
        let ask = Cells { sheet: 0, range: asked };
        assert_eq!(check_cells(&ask, &cut, &Limits::CONTRACT), Ok(()));
        // Panes inside the range are the file's own.
        assert_eq!(cell_data(block(1, 0), &asked).frozen, Frozen { rows: 1, cols: 0 });
        assert_eq!(cell_data(block(15, 9), &asked).frozen, Frozen { rows: 15, cols: 9 });
    }
}
