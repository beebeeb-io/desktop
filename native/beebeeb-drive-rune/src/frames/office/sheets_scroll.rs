//! A workbook's scroll (lead ruling 2026-11-16, "the scroll container decision": the in-app PDF viewer's model). The grid
//! stays drawn from `top_left` (the cells in view are asked of the renderer by that window, so the whole sheet is never
//! laid out); over it lie two clear Rune scroll viewports, one down (`El::scroll`) and one across (`El::scroll_x`), each
//! with `rune-kit`'s scroll bar. Rune moves them: the wheel goes down, a sideways delta (Shift+wheel on Windows, which
//! the platform layer turns sideways; a touchpad) goes across, with Rune's momentum. Each viewport scrolls a spacer one
//! [`ROW_Q`] per row (one [`COL_Q`] per column) long, and where the spacer stands is turned back into `top_left`.
//!
//! One truth: every other writer of `top_left` (the keys, a sheet tab) moves the viewports to it in the same handler
//! ([`follow`]), so the next wheel turn starts from there. A selection is never changed by a scroll.
//!
//! Two viewports rather than one that scrolls both ways: `rune-kit` draws one bar per viewport.

use rune::{El, Key, Mods, ScrollSpec, Tx, col, el, row};
use rune_kit::{ScrollBar, ScrollBarStyle};

use super::{Cell, SheetView, scroll_key};
use crate::State;
use crate::frames::viewers::{self, source::Body};
use crate::theme::tokens::Theme;

/// Points of scroll per row, and per column.
pub const ROW_Q: f32 = 20.0;
pub const COL_Q: f32 = 64.0;
/// The viewports' keys.
pub const DOWN: &str = "sh-scroll-v";
pub const ACROSS: &str = "sh-scroll-x";
/// A workbook's own limits (rows, columns): an extent past them is not scrolled to.
const MAX_ROWS: u32 = 1_048_576;
const MAX_COLS: u32 = 16_384;

/// The viewports follow `top_left` (after a key or a tab moved it).
pub fn follow(view: &SheetView, tx: &mut Tx) {
    tx.scroll_to_y(DOWN, view.top_left.row as f32 * ROW_Q);
    tx.scroll_to_x(ACROSS, view.top_left.col as f32 * COL_Q);
}

/// The sheet's keys ([`scroll_key`]), with the viewports following. `true`: taken.
pub fn scroll_key_tx(view: &mut SheetView, key: &Key, m: &Mods, tx: &mut Tx) -> bool {
    if !scroll_key(view, key, m) {
        return false;
    }
    follow(view, tx);
    true
}

/// The row (or column) a viewport scrolled `by` points shows first, never past the last of `extent`.
fn index_of(by: f32, q: f32, extent: u32) -> u32 {
    let at = if by.is_finite() { (by / q).round().max(0.0) } else { 0.0 };
    (at as u32).min(extent.saturating_sub(1))
}

/// The open workbook's `top_left` moved to `row` / `col` by a scroll. `true`: it changed (the view runs again).
fn scrolled(s: &mut State, row: Option<u32>, col: Option<u32>) -> bool {
    let Some(Body::Sheet(view)) = viewers::ui(s).map(|ui| &mut ui.file.body) else {
        return false;
    };
    let mut to: Cell = view.top_left;
    if let Some(r) = row {
        to.row = r.min(view.extent.0.min(MAX_ROWS).saturating_sub(1));
    }
    if let Some(c) = col {
        to.col = c.min(view.extent.1.min(MAX_COLS).saturating_sub(1));
    }
    if to == view.top_left {
        return false;
    }
    view.top_left = to;
    true
}

/// The two clear viewports over the grid. `down` / `across`: their rects as the last frame laid them out (`None` on a
/// first build).
pub fn overlay(view: &SheetView, down: Option<[f32; 4]>, across: Option<[f32; 4]>, t: &Theme) -> El {
    let rows = view.extent.0.min(MAX_ROWS);
    let cols = view.extent.1.min(MAX_COLS);
    let tall = down.map_or(600.0, |r| r[3]);
    let wide = across.map_or(800.0, |r| r[2]);
    // Long enough that the last row (column) can be scrolled to the top (left): one quantum per row past the first, and
    // the viewport's own size.
    let mut down_body = el()
        .key("sh-scroll-v-body")
        .w_full()
        .h(rows.saturating_sub(1) as f32 * ROW_Q + tall);
    if let Some(r) = down {
        let top = r[1];
        down_body = down_body.on_layout_tx(move |s: &mut State, b: [f32; 4], tx: &mut Tx| {
            let row = index_of(top - b[1], ROW_Q, MAX_ROWS);
            if !scrolled(s, Some(row), None) {
                tx.live_only();
            }
        });
    }
    let mut across_body = el()
        .key("sh-scroll-x-body")
        .h_full()
        .w(cols.saturating_sub(1) as f32 * COL_Q + wide);
    if let Some(r) = across {
        let left = r[0];
        across_body = across_body.on_layout_tx(move |s: &mut State, b: [f32; 4], tx: &mut Tx| {
            let col = index_of(left - b[0], COL_Q, MAX_COLS);
            if !scrolled(s, None, Some(col)) {
                tx.live_only();
            }
        });
    }
    let bar = ScrollBarStyle::neutral(t.dark).thumb(t.scrollbar);
    let across_port = row()
        .key(ACROSS)
        .a11y_hidden()
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .scroll_x()
        .child(across_body)
        .scroll_bar_styled(bar.horizontal());
    // Over the sideways one, short of its bar's strip at the bottom so that bar can still be taken.
    let down_port = col()
        .key(DOWN)
        .a11y_hidden()
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(bar.strip()))
        .scroll(ScrollSpec { stick_bottom: false })
        .child(down_body)
        .scroll_bar_styled(bar);
    el().key("sh-scroll")
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .child(across_port)
        .child(down_port)
}

#[cfg(test)]
#[path = "sheets_scroll_tests.rs"]
mod tests;
