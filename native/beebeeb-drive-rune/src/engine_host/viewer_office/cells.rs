//! The cells of an open sheet: which block to ask, and how a block's answer merges into the sheet. Child of
//! `viewer_office.rs`.

use super::*;

/// `outer` holds every cell of `inner`.
pub(super) fn covers(outer: &CellRange, inner: &CellRange) -> bool {
    let end = |start: u32, len: u32| u64::from(start) + u64::from(len);
    outer.row <= inner.row
        && outer.col <= inner.col
        && end(outer.row, outer.rows) >= end(inner.row, inner.rows)
        && end(outer.col, outer.cols) >= end(inner.col, inner.cols)
}

impl Model {
    /// The block of the active sheet to ask for the cells from (`first_row`, `first_col`), `rows` × `cols`: on the
    /// block grid, inside the sheet, cut to `office_host::MAX_ASKED_CELLS`. `None`: nothing of it is in the sheet.
    pub(super) fn block(&self, first_row: u32, first_col: u32, rows: u32, cols: u32) -> Option<CellRange> {
        let Body::Sheet(view) = &self.body else {
            return None;
        };
        let (last_row, last_col) = view.extent;
        if rows == 0 || cols == 0 {
            return None;
        }
        let up = |at: u32, len: u32, grid: u32| at.saturating_add(len).div_ceil(grid).saturating_mul(grid);
        let (r0, c0) = (first_row / BLOCK_ROWS * BLOCK_ROWS, first_col / BLOCK_COLS * BLOCK_COLS);
        let r1 = up(first_row, rows, BLOCK_ROWS).min(last_row);
        let c1 = up(first_col, cols, BLOCK_COLS).min(last_col);
        if r0 >= r1 || c0 >= c1 {
            return None;
        }
        let max = office_host::MAX_ASKED_CELLS;
        let cols = (c1 - c0).min(u32::try_from(max).unwrap_or(u32::MAX));
        let rows = (r1 - r0).min(u32::try_from(max / u64::from(cols)).unwrap_or(u32::MAX).max(1));
        Some(CellRange {
            row: r0,
            col: c0,
            rows,
            cols,
        })
    }

    pub(super) fn remember(&mut self, range: CellRange) {
        if self.asked.len() >= ASKED_BLOCKS_KEPT {
            self.asked.remove(0);
        }
        self.asked.push(range);
    }

    /// The view moved to sheet `sheet`: the cells held of the other one are overwritten and dropped. `false`: no such
    /// sheet.
    pub(super) fn switch_sheet(&mut self, sheet: u32) -> bool {
        let count = self.sheets.len() as u64 + u64::from(self.more);
        if u64::from(sheet) >= count {
            return false;
        }
        // A sheet past the named ones has no size here: the protocol's largest sheet bounds it.
        let extent = self
            .sheets
            .get(sheet as usize)
            .copied()
            .unwrap_or((check::MAX_SHEET_ROWS, check::MAX_SHEET_COLS));
        let Body::Sheet(view) = &mut self.body else {
            return false;
        };
        for cell in &mut view.cells {
            cell.text.zeroize();
            if let Some(formula) = cell.formula.as_mut() {
                formula.zeroize();
            }
        }
        view.formula.zeroize();
        view.cells.clear();
        view.col_widths.clear();
        view.row_heights.clear();
        view.merges.clear();
        view.anchors.clear();
        view.frozen = (0, 0);
        view.selection = None;
        view.missing_content = false;
        view.current = sheets::Cell { row: 0, col: 0 };
        view.top_left = sheets::Cell { row: 0, col: 0 };
        view.sheet = sheet;
        if (sheet as usize) < view.tabs.len() {
            view.active = sheet as usize;
        }
        view.extent = extent;
        self.index.clear();
        self.asked.clear();
        self.dirty = true;
        true
    }

    /// `data` (the cells of `range`) into the sheet, by place: a cell held at the same place is replaced (its text
    /// overwritten first), sizes per column and row, merges, the frozen panes and the anchors once each.
    pub(super) fn merge(&mut self, range: CellRange, data: CellData) {
        let Body::Sheet(view) = &mut self.body else {
            return;
        };
        let CellData {
            cells,
            col_widths,
            row_heights,
            merges,
            frozen,
            anchors,
            missing_content,
        } = data;
        for cell in cells {
            let new = sheets::CellText {
                row: cell.row,
                col: cell.col,
                text: cell.text,
                right_aligned: cell.align == Align::Right,
                bold: cell.bold,
                value: cell.value,
                formula: cell.formula,
            };
            match self.index.get(&(new.row, new.col)) {
                Some(&at) => {
                    let old = &mut view.cells[at];
                    old.text.zeroize();
                    if let Some(formula) = old.formula.as_mut() {
                        formula.zeroize();
                    }
                    *old = new;
                }
                None => {
                    self.index.insert((new.row, new.col), view.cells.len());
                    view.cells.push(new);
                }
            }
        }
        put_sizes(&mut view.col_widths, range.col, &col_widths, DEFAULT_COL_PT);
        put_sizes(&mut view.row_heights, range.row, &row_heights, DEFAULT_ROW_PT);
        // Held once each and capped, and the frozen panes the largest reported (`sheets_bounds`).
        view.hold_merges(merges.into_iter().map(|merge| {
            (
                sheets::Cell {
                    row: merge.row,
                    col: merge.col,
                },
                sheets::Cell {
                    row: merge.row + merge.rows.saturating_sub(1),
                    col: merge.col + merge.cols.saturating_sub(1),
                },
            )
        }));
        view.hold_frozen(frozen.rows, frozen.cols);
        for anchor in anchors {
            let placed = SheetAnchor {
                anchor: anchor.id,
                from: sheets::Cell {
                    row: anchor.from_row,
                    col: anchor.from_col,
                },
                to: sheets::Cell {
                    row: anchor.to_row,
                    col: anchor.to_col,
                },
                x_pt: anchor.x_pt,
                y_pt: anchor.y_pt,
                w_pt: anchor.w_pt,
                h_pt: anchor.h_pt,
            };
            match view.anchors.iter_mut().find(|held| held.anchor == placed.anchor) {
                Some(held) => *held = placed,
                None => view.anchors.push(placed),
            }
        }
        view.missing_content |= missing_content;
    }
}

/// Sizes from `start` on, into `sizes` (one per column or row from 0; a gap is filled with `default`).
fn put_sizes(sizes: &mut Vec<f32>, start: u32, new: &[f32], default: f32) {
    let start = start as usize;
    if sizes.len() < start + new.len() {
        sizes.resize(start + new.len(), default);
    }
    sizes[start..start + new.len()].copy_from_slice(new);
}
