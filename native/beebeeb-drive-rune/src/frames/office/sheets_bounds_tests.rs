//! Tests of the sheet's bounds (the Office viewer's review, P2-1 and P2-2): frozen panes, hidden and tiny lines and
//! merges cost at most a screen per frame; a chart or picture sits at its `from` cell plus its offset into that cell.

use super::super::*;
use super::*;
use crate::frames::office::docs::live_tests::{find, live_runtime, rebuild};
use crate::frames::viewers::source::Body;

/// The window `live_runtime` draws in, in points: no frame can need more rows or columns than these fill.
const WINDOW: (f32, f32) = (1278.0, 798.0);

/// A sheet of `rows` by `cols` of known size (80 x 24 pt), nothing held but what a test puts in.
fn sheet(rows: usize, cols: usize) -> SheetView {
    let mut view = sheets_fixture::board();
    view.cells.clear();
    view.col_widths = vec![80.0; cols];
    view.row_heights = vec![24.0; rows];
    view.extent = (rows as u32, cols as u32);
    view.selection = None;
    view.formula = String::new();
    view.merges.clear();
    view.anchors.clear();
    view.frozen = (0, 0);
    view
}

/// The cells the grid drew, by place (`sh-cell-<R>-<C>`, not their text or gap).
fn cells_drawn(rt: &rune::Runtime<State>) -> Vec<(u32, u32)> {
    rt.elements()
        .into_iter()
        .filter(|e| !e.exiting)
        .filter_map(|e| {
            let key = e.key?;
            let (r, c) = key.strip_prefix("sh-cell-")?.split_once('-')?;
            Some((r.parse().ok()?, c.parse().ok()?))
        })
        .collect()
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.6
}

#[test]
fn office_sheets_bounds_frozen_panes_at_a_sheets_size_lay_out_one_screen() {
    let (rows, cols) = (1_048_576, 16_384);
    let mut view = sheet(rows, cols);
    view.row_heights = vec![15.0; rows];
    view.col_widths = vec![48.0; cols];
    view.frozen = (rows as u32, cols as u32);
    view.top_left = Cell {
        row: 1_000_000,
        col: 16_000,
    };
    // Laid out first, never drawn when it is not bounded (a million rows a frame would stall the test as the window).
    let shown = Shown::from_top_left(&view, (1000.0, 600.0));
    // 600 pt of 15 pt rows is 40, 1000 pt of 48 pt columns 21 (the line across the edge counts), and one scrolled line.
    assert!(shown.rows.len() <= 41, "rows laid out: {}", shown.rows.len());
    assert!(shown.cols.len() <= 22, "columns laid out: {}", shown.cols.len());
    let (rt, _log) = live_runtime(Body::Sheet(view), 1.0, None);
    let drawn = cells_drawn(&rt).len();
    let most = (WINDOW.1 / 15.0) as usize + 2;
    let most = most * ((WINDOW.0 / 48.0) as usize + 2);
    assert!(drawn > 0 && drawn <= most, "cells drawn: {drawn}, at most {most}");
    assert!(find(&rt, "sh-cell-0-0").is_some(), "the frozen corner is drawn");
}

#[test]
fn office_sheets_bounds_hidden_and_tiny_lines_stay_within_the_budget() {
    // Hidden rows and columns (0 pt) take no room and are not drawn.
    let mut view = sheet(400, 30);
    for r in 0..10 {
        view.row_heights[r] = 0.0;
    }
    view.col_widths[1] = 0.0;
    let shown = Shown::from_top_left(&view, (1000.0, 600.0));
    assert!(
        !shown.rows.contains(&3) && shown.rows.contains(&10),
        "rows: {:?}",
        &shown.rows[..4]
    );
    assert!(
        !shown.cols.contains(&1) && shown.cols.contains(&2),
        "columns: {:?}",
        &shown.cols[..3]
    );
    // Lines of a hundredth of a point (a crafted workbook): the walk's bound per side, and the cells drawn bounded.
    let mut view = sheet(8_192, 8_192);
    view.row_heights = vec![0.01; 8_192];
    view.col_widths = vec![0.01; 8_192];
    let shown = Shown::from_top_left(&view, (1000.0, 600.0));
    let cells = shown.rows.len() * shown.cols.len();
    assert!(cells <= MAX_DRAWN_CELLS, "{cells} cells laid out");
    assert!(!shown.cols.is_empty() && !shown.rows.is_empty());
}

#[test]
fn office_sheets_bounds_many_merges_cost_per_frame_is_bounded() {
    let mut view = sheet(400, 30);
    // 100,000 merged areas held, far down a sheet longer than the screen, and one on screen.
    view.merges = (0..100_000u32)
        .map(|i| (Cell { row: 1_000 + i, col: 0 }, Cell { row: 1_000 + i, col: 1 }))
        .collect();
    view.merges.push((Cell { row: 1, col: 1 }, Cell { row: 1, col: 2 }));
    let held = view.merges.len() as u64;
    let (mut rt, log) = live_runtime(Body::Sheet(view), 1.0, None);
    MERGES_LOOKED.with(|n| n.set(0));
    rebuild(&mut rt, &log, |_| {});
    let looked = MERGES_LOOKED.with(|n| n.get());
    let drawn = cells_drawn(&rt).len() as u64;
    // One look per merge held per build (a frame may build twice), never one per merge per cell.
    assert!(
        looked <= 2 * (held + drawn),
        "{looked} looks for {held} merges and {drawn} cells"
    );
    let b2 = find(&rt, "sh-cell-1-1").expect("B2 starts the merge");
    assert!(close(b2.rect[2], 160.0), "B2 spans two columns: {:?}", b2.rect);
    assert!(find(&rt, "sh-cell-1-2").is_none(), "C2 is under B2");
}

#[test]
fn office_sheets_bounds_overlapping_merges_cost_per_frame_is_bounded() {
    // A crafted workbook (or helper): K21 merged alone, then 5,000 areas over A1:K21 that overlap it only at their last
    // cell. Each would read 231 drawn cells before it is turned away; a frame reads at most a few per drawn cell.
    let mut view = sheet(400, 30);
    view.merges.push((Cell { row: 20, col: 10 }, Cell { row: 20, col: 10 }));
    view.merges
        .extend((0..5_000).map(|_| (Cell { row: 0, col: 0 }, Cell { row: 20, col: 10 })));
    let (mut rt, log) = live_runtime(Body::Sheet(view), 1.0, None);
    CELLS_SCANNED.with(|n| n.set(0));
    rebuild(&mut rt, &log, |_| {});
    let scanned = CELLS_SCANNED.with(|n| n.get());
    let drawn = cells_drawn(&rt).len() as u64;
    // Two drawn cells' worth per build at most (a frame may build twice), never 231 per area held (1,155,000).
    assert!(scanned <= 2 * 2 * drawn, "{scanned} cells read for {drawn} drawn");
    assert!(
        find(&rt, "sh-cell-0-0").is_some() && find(&rt, "sh-cell-20-10").is_some(),
        "the overlapping areas are not drawn; A1 and K21 are"
    );
}

#[test]
fn office_sheets_bounds_a_merge_keeps_the_cells_beside_it_in_their_columns() {
    let mut view = sheet(400, 30);
    // B2:C3, two rows: the second row's D3 stays under D2.
    view.merges = vec![(Cell { row: 1, col: 1 }, Cell { row: 2, col: 2 })];
    let (rt, _log) = live_runtime(Body::Sheet(view), 1.0, None);
    let (d2, d3) = (
        find(&rt, "sh-cell-1-3").expect("D2"),
        find(&rt, "sh-cell-2-3").expect("D3"),
    );
    assert!(
        close(d2.rect[0], d3.rect[0]),
        "D3 at {:?}, D2 at {:?}",
        d3.rect,
        d2.rect
    );
    assert!(
        find(&rt, "sh-cell-2-1").is_none() && find(&rt, "sh-cell-2-2").is_none(),
        "B3 and C3 are under B2"
    );
    let b2 = find(&rt, "sh-cell-1-1").expect("B2 starts the merge");
    assert!(close(b2.rect[2], 160.0), "B2 spans two columns: {:?}", b2.rect);
}

#[test]
fn office_sheets_bounds_merges_held_are_capped_and_kept_once() {
    let mut view = sheet(400, 30);
    let at = |i: u32| (Cell { row: i, col: 0 }, Cell { row: i, col: 1 });
    view.hold_merges((0..10).map(at));
    view.hold_merges((0..10).map(at));
    assert_eq!(view.merges.len(), 10, "a merge a later block repeats is held once");
    assert!(!view.missing_content);
    view.hold_merges((0..MAX_MERGES_HELD as u32 + 100).map(at));
    assert_eq!(view.merges.len(), MAX_MERGES_HELD, "no more than the cap is held");
    assert!(view.missing_content, "the status line says some content is not shown");
}

#[test]
fn office_sheets_bounds_frozen_held_is_the_largest_a_block_reported() {
    // The renderer cuts frozen panes to each block's end: a later block near the top must not shrink them.
    let mut view = sheet(400, 30);
    view.hold_frozen(100, 2);
    view.hold_frozen(64, 1);
    assert_eq!(view.frozen, (100, 2));
}

#[test]
fn office_sheets_bounds_an_anchor_at_d10_stays_on_d10_when_scrolled() {
    let mut view = sheet(400, 30);
    view.frozen = (1, 1);
    // At D10, 12 pt across and 5 pt down into the cell.
    view.anchors.push(SheetAnchor {
        anchor: 3,
        from: Cell { row: 9, col: 3 },
        to: Cell { row: 14, col: 6 },
        x_pt: 12.0,
        y_pt: 5.0,
        w_pt: 200.0,
        h_pt: 100.0,
    });
    // At D4, scrolled out above: its lower part still shows, from where D4 would be (D4 and D5 are 48 pt above D6).
    view.anchors.push(SheetAnchor {
        anchor: 4,
        from: Cell { row: 3, col: 3 },
        to: Cell { row: 20, col: 4 },
        x_pt: 0.0,
        y_pt: 10.0,
        w_pt: 100.0,
        h_pt: 400.0,
    });
    view.top_left = Cell { row: 5, col: 2 };
    let (rt, _log) = live_runtime(Body::Sheet(view), 1.0, None);
    let d10 = find(&rt, "sh-cell-9-3").expect("D10 is drawn");
    let at = find(&rt, "sh-anchor-3").expect("the anchor at D10 is drawn");
    assert!(
        close(at.rect[0], d10.rect[0] + 12.0) && close(at.rect[1], d10.rect[1] + 5.0),
        "anchor {:?}, D10 {:?}",
        at.rect,
        d10.rect
    );
    let d6 = find(&rt, "sh-cell-5-3").expect("D6, the first row scrolled to");
    let up = find(&rt, "sh-anchor-4").expect("the anchor at D4 reaches into view");
    assert!(
        close(up.rect[0], d6.rect[0]) && close(up.rect[1], d6.rect[1] - 48.0 + 10.0),
        "anchor {:?}, D6 {:?}",
        up.rect,
        d6.rect
    );
}
