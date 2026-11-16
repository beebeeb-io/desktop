//! What one frame of a sheet lays out stays within the screen, whatever a workbook (and so the renderer) says. Child of
//! `sheets.rs`.
//!
//! - Rows and columns ([`Side::walk`]): the frozen ones first, then those from `top_left`, together no more than fill
//!   the screen, and never more than [`MAX_LINES`] walked per side. A hidden row or column (0 pt) takes no room and is
//!   not drawn. A frame draws at most [`MAX_DRAWN_CELLS`] cells.
//! - Merges: those held are capped ([`MAX_MERGES_HELD`]) and held once each. Each frame indexes the ones that touch the
//!   drawn cells in one pass ([`MergesShown`]), so no cell walks the list.
//! - Charts and pictures: each at its `from` cell plus its offset into that cell (the protocol's words), placed from
//!   the frame's running sums of the rows and columns it walked ([`Side::start_of`]), never from a sum over the sheet
//!   above the view.

use std::collections::HashSet;

use beebeeb_office_protocol::check::MAX_ANCHOR_SIDE_PT;

use super::{Cell, SheetView, bounds};

/// The most rows, or columns, one frame walks per side (frozen and scrolled together), whatever sizes are held.
pub(super) const MAX_LINES: u32 = 4_096;

/// The most cells one frame draws: as many as one ask of cells may hold (`office_host::MAX_ASKED_CELLS`). Only lines of
/// a fraction of a point reach it; the columns past it are then not drawn.
pub(super) const MAX_DRAWN_CELLS: usize = 65_536;

/// The most merged areas a sheet holds (they gather across the blocks asked). One past it is not held, and the status
/// line says some content is not shown.
pub const MAX_MERGES_HELD: usize = 16_384;

/// How far above (or left of) the view a chart's corner may sit and still reach into it: the protocol's farthest
/// offset into a cell plus its largest side.
const REACH_PT: f32 = 2.0 * MAX_ANCHOR_SIDE_PT;

/// One side (the rows, or the columns) of what a frame lays out: the lines drawn, the frozen ones among them, and where
/// each line walked starts, in points from the grid's first line.
pub(super) struct Side {
    /// The lines drawn, in ascending order: the frozen ones, then those from `first`. Hidden lines, and lines whose
    /// size is not held yet, are walked but not drawn.
    pub lines: Vec<u32>,
    /// How many of `lines` are frozen.
    pub frozen: usize,
    /// The frozen lines ended before the frozen panes did (the screen, or the walk's bound, cut them).
    pub cut: bool,
    /// The frozen panes as drawn: `0..frozen_end` (never past `first`).
    frozen_end: u32,
    /// Where each frozen line walked starts, and where the last ends.
    frozen_at: Vec<f32>,
    /// The first line scrolled to (`top_left`), and where each line walked from it starts, and where the last ends.
    first: u32,
    scrolled_at: Vec<f32>,
    /// How far above `first` each line before it starts (the line just before `first` at index 0), walked back as far
    /// as a chart can reach from: `scrolled_at[0] - back[i]` is where line `first - 1 - i` would start.
    back: Vec<f32>,
}

impl Side {
    /// The lines of one side: frozen `0..min(frozen, first)`, then from `first`, together filling `room` points (the line
    /// across the edge counts; at least one scrolled line), at most [`MAX_LINES`] walked. `sizes` holds a size per line
    /// from 0; a line past them counts `gap` toward the room and is not drawn.
    pub(super) fn walk(sizes: &[f32], frozen: u32, first: u32, room: f32, gap: f32) -> Side {
        let held = |line: u32| sizes.get(line as usize).copied();
        let frozen_end = frozen.min(first);
        let mut side = Side {
            lines: Vec::new(),
            frozen: 0,
            cut: false,
            frozen_end,
            frozen_at: vec![0.0],
            first,
            scrolled_at: Vec::new(),
            back: Vec::new(),
        };
        // `used` is the room taken (a line not held takes `gap`); `laid` is where the next drawn line starts.
        let (mut used, mut laid, mut walked) = (0.0f32, 0.0f32, 0u32);
        let mut line = 0;
        while line < frozen_end && used < room && walked < MAX_LINES {
            used += held(line).unwrap_or(gap);
            laid += side.draw(line, held(line));
            side.frozen_at.push(laid);
            line += 1;
            walked += 1;
        }
        side.frozen = side.lines.len();
        side.cut = line < frozen_end;
        side.scrolled_at.push(laid);
        let (mut line, mut scrolled) = (first, 0u32);
        while (scrolled == 0 || used < room) && walked < MAX_LINES {
            used += held(line).unwrap_or(gap);
            laid += side.draw(line, held(line));
            side.scrolled_at.push(laid);
            line = line.saturating_add(1);
            scrolled += 1;
            walked += 1;
        }
        // Back from `first`, over the lines scrolled out (never into the frozen ones drawn above them).
        let mut up = 0.0f32;
        let mut line = first;
        while line > frozen_end && up <= REACH_PT && side.back.len() < MAX_LINES as usize {
            line -= 1;
            up += held(line).unwrap_or(gap);
            side.back.push(up);
        }
        side
    }

    /// Every line of `sizes`, none frozen (the fixture's whole sheet).
    pub(super) fn all(sizes: &[f32]) -> Side {
        let scrolled_at = std::iter::once(0.0)
            .chain(sizes.iter().scan(0.0, |at, s| {
                *at += s;
                Some(*at)
            }))
            .collect();
        Side {
            lines: (0..sizes.len() as u32).collect(),
            frozen: 0,
            cut: false,
            frozen_end: 0,
            frozen_at: vec![0.0],
            first: 0,
            scrolled_at,
            back: Vec::new(),
        }
    }

    /// `line` drawn when its size is held and above 0: the room it takes in the layout.
    fn draw(&mut self, line: u32, size: Option<f32>) -> f32 {
        match size {
            Some(s) if s > 0.0 => {
                self.lines.push(line);
                s
            }
            _ => 0.0,
        }
    }

    /// Keep the first `n` lines drawn (the cell budget).
    pub(super) fn keep(&mut self, n: usize) {
        self.lines.truncate(n);
        self.frozen = self.frozen.min(self.lines.len());
    }

    /// Where `line` starts in this frame, in points from the grid's first line: a line walked by its running sum, one
    /// scrolled out above the view by the walk back. `None`: below or right of what is drawn, a frozen line the screen
    /// cut off, or farther above than a chart can reach from.
    pub(super) fn start_of(&self, line: u32) -> Option<f32> {
        let frozen_walked = self.frozen_at.len() - 1;
        if (line as usize) < frozen_walked {
            return Some(self.frozen_at[line as usize]);
        }
        if line >= self.first {
            let i = (line - self.first) as usize;
            return if i + 1 < self.scrolled_at.len() {
                Some(self.scrolled_at[i])
            } else {
                None
            };
        }
        if line < self.frozen_end {
            return None;
        }
        let i = (self.first - 1 - line) as usize;
        self.back.get(i).map(|up| self.scrolled_at[0] - up)
    }
}

/// A side reads as its lines drawn.
impl std::ops::Deref for Side {
    type Target = [u32];
    fn deref(&self) -> &[u32] {
        &self.lines
    }
}

/// What the grid draws at one of its cells, from the merges held.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum AtCell {
    /// No merge: the cell at its own width.
    Own,
    /// A merge starts here: the cell is `w` points wide (the drawn columns the merge covers); `last`: the merge's last
    /// drawn row (it has the bottom line).
    Spans { w: f32, last: bool },
    /// A row of a merge whose first cell is not in this row, or is not drawn: a blank `w` points wide, so the cells
    /// after it stay in their columns.
    Blank { w: f32, last: bool },
    /// Inside the span or the blank of the cell before it in the row: nothing drawn.
    Covered,
}

/// The merges that touch the drawn cells, for one frame: built in one pass over the merges held, then one lookup per
/// cell. Merges that overlap one taken before them are not drawn (a valid workbook has none); a frame spends at most a
/// few looks per drawn cell on them whatever is held.
pub(super) struct MergesShown {
    cols: usize,
    at: Vec<AtCell>,
}

impl MergesShown {
    /// The merges of `view` over the drawn `rows` and `cols` (each ascending), `width` of a drawn column.
    pub(super) fn of(view: &SheetView, rows: &[u32], cols: &[u32], width: impl Fn(u32) -> f32) -> MergesShown {
        let n = cols.len();
        if view.merges.is_empty() {
            return MergesShown {
                cols: n,
                at: Vec::new(),
            };
        }
        let mut at = vec![AtCell::Own; rows.len() * n];
        // The drawn part of `lo..=hi` on one side: one run of indexes, as the lines drawn ascend.
        let run =
            |lines: &[u32], lo: u32, hi: u32| (lines.partition_point(|&l| l < lo), lines.partition_point(|&l| l <= hi));
        let mut spend = 4 * at.len();
        for &m in &view.merges {
            looked();
            let (r0, r1, c0, c1) = bounds(m);
            let ((ra, rb), (ca, cb)) = (run(rows, r0, r1), run(cols, c0, c1));
            if ra >= rb || ca >= cb {
                continue;
            }
            let area = (rb - ra) * (cb - ca);
            if 2 * area > spend {
                break;
            }
            spend -= 2 * area;
            if (ra..rb).any(|ri| at[ri * n + ca..ri * n + cb].iter().any(|a| *a != AtCell::Own)) {
                continue;
            }
            let w: f32 = cols[ca..cb].iter().map(|&c| width(c)).sum();
            for ri in ra..rb {
                let last = ri + 1 == rb;
                at[ri * n + ca] = if rows[ri] == r0 && cols[ca] == c0 {
                    AtCell::Spans { w, last }
                } else {
                    AtCell::Blank { w, last }
                };
                at[ri * n + ca + 1..ri * n + cb].fill(AtCell::Covered);
            }
        }
        MergesShown { cols: n, at }
    }

    /// What is drawn at the `ri`th drawn row's `ci`th drawn column.
    pub(super) fn at(&self, ri: usize, ci: usize) -> AtCell {
        self.at.get(ri * self.cols + ci).copied().unwrap_or(AtCell::Own)
    }
}

impl SheetView {
    /// Merged areas from a block, by corners, into those held: each once, at most [`MAX_MERGES_HELD`]. One past the cap
    /// is not held, and sets `missing_content` (the status line's quiet line).
    pub(crate) fn hold_merges(&mut self, merges: impl IntoIterator<Item = (Cell, Cell)>) {
        let mut held: Option<HashSet<(Cell, Cell)>> = None;
        for m in merges {
            let held = held.get_or_insert_with(|| self.merges.iter().copied().collect());
            if held.contains(&m) {
                continue;
            }
            if self.merges.len() >= MAX_MERGES_HELD {
                self.missing_content = true;
                break;
            }
            held.insert(m);
            self.merges.push(m);
        }
    }

    /// The frozen panes a block reports, kept as the largest reported for this sheet: the renderer cuts them to each
    /// block's end, so a block near the top may report fewer than the sheet has. A sheet switch sets them back to none.
    pub(crate) fn hold_frozen(&mut self, rows: u32, cols: u32) {
        self.frozen = (self.frozen.0.max(rows), self.frozen.1.max(cols));
    }
}

#[cfg(test)]
thread_local! {
    /// How many merges the grid looked at (a test counts a frame's).
    pub(super) static MERGES_LOOKED: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// One merge looked at (counted for the tests).
pub(super) fn looked() {
    #[cfg(test)]
    MERGES_LOOKED.with(|n| n.set(n.get() + 1));
}

#[cfg(test)]
#[path = "sheets_bounds_tests.rs"]
mod tests;
