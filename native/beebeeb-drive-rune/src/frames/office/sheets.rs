//! Slice OF2 (plan40 batch 1, viewer only): the spreadsheet VIEWER, DW-1508 as the Office viewer draws it
//! (read-only: no ribbon). The window's toolbar, the cell box and the formula as read-only text, the grid with
//! column letters and row numbers, the sheet tabs (no "+"), and the status line (Sheet N of M, and Sum, Average
//! and Count of the selection; nothing without one; Encrypted).
//!
//! The grid is OURS: Rune elements drawn from a [`SheetView`] (the engine gives geometry and cells; we draw), never
//! an image. [`sheets_fixture::board`] is the board's workbook; a live host fills the same shape later.
//! DW-1559 (the Data tab) is a 0.9.1 frame (the editor): its row stays `placeholder::scene`, nothing of it is drawn.
//!
//! Wiring (the integration lane applies it, see this lane's BRANCH-NOTES): `Body::Sheet(SheetView)` in
//! `viewers::source`, one arm in `viewers::view` that calls [`window`], one `scene_file` arm for DW-1508.
//!
//! Keys: the window's own (caption, sidebar, toolbar), then `sh-cellbox`, `sh-formula` (the formula bar),
//! `sh-corner`, `sh-col-<C>` and `sh-row-<R>` (the headers), `sh-cell-<R>-<C>` and its text `sh-cell-<R>-<C>-t`
//! (zero-based), `sh-tab-<I>`, `vbar` (zoom, Open in another app…), and `st-<N>` (the status line's parts, numbered as the other Office frames').

use std::collections::HashMap;
use std::fmt;

use rune::tok::Family;
use rune::{Cx, El, Icon, Key, Mods, Span, SurfaceFit, TextStyle, col, el, rich, row};
use zeroize::Zeroize;

use crate::State;
use crate::frames::placeholder;
use crate::frames::viewers::kinds::OfficePicture;
#[cfg(test)]
use crate::frames::viewers::source::Body;
use crate::frames::viewers::{self, Ui};
use crate::theme::icons;
use crate::theme::tokens::{self, Theme, type_};
use crate::ui::{caption, kit, sidebar};

use super::docs::{ABOUT, ENCRYPTED, KEEP_FIRST, Live, OPEN_ELSEWHERE};
use super::edit::sheet as edit_sheet;
use super::states::QUIET_LINE;

#[path = "sheets_fixture.rs"]
pub mod sheets_fixture;

/// The group's board frames and the scene that shows each (`frames::scenes`).
pub const FRAMES: &[crate::ui::gate::Scene<State>] = &[
    ("DW-1508", |s| viewers::set_scene(s, "DW-1508")),
    ("DW-1559", placeholder::scene),
];

// ---------------------------------------------------------------------------
// What the viewer draws from
// ---------------------------------------------------------------------------

/// A cell's place, zero-based ("B10" is row 9, column 1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Cell {
    pub row: u32,
    pub col: u32,
}

/// The text of one cell, as the engine shows it (already formatted), and how it sits.
#[derive(Clone, PartialEq)]
pub struct CellText {
    pub row: u32,
    pub col: u32,
    pub text: String,
    pub right_aligned: bool,
    pub bold: bool,
    /// The number behind the cell: Sum, Average and Count use it, and fall back to the text only for a cell without one.
    pub value: Option<f64>,
    /// The cell's formula as text ("=SUM(B2:B9)"), shown in the formula bar while it is the current cell.
    pub formula: Option<String>,
}

/// A chart or picture the sheet places: its top-left corner `x_pt`, `y_pt` into its `from` cell (the protocol's words: a
/// cell plus an offset into it), its size, and the cells it starts and ends in.
#[derive(Clone, Debug, PartialEq)]
pub struct SheetAnchor {
    pub anchor: u32,
    pub from: Cell,
    pub to: Cell,
    pub x_pt: f32,
    pub y_pt: f32,
    pub w_pt: f32,
    pub h_pt: f32,
}

/// One sheet of a workbook as the viewer draws it. A live host fills it; only this is drawn from.
/// (A chart or a picture, when the sheet has one, is `rune::Pixels` at an anchor: not drawn yet, see the notes.)
#[derive(Clone, PartialEq)]
pub struct SheetView {
    pub tabs: Vec<String>,
    pub active: usize,
    /// Points, one per column / row drawn.
    pub col_widths: Vec<f32>,
    pub row_heights: Vec<f32>,
    pub cells: Vec<CellText>,
    /// The selected range, corners in any order. `None`: nothing selected (the status shows no figures).
    pub selection: Option<(Cell, Cell)>,
    pub current: Cell,
    /// The current cell's formula as text ("=SUM(B2:B9)"), or its value's text.
    pub formula: String,
    /// Rows and columns that stay in view, as the file says (the largest a block reported: `hold_frozen`). Only as many
    /// as fit the screen are drawn.
    pub frozen: (u32, u32),
    pub merges: Vec<(Cell, Cell)>,
    /// The sheet index `active` points at in the workbook (the tabs may be fewer than the sheets).
    pub sheet: u32,
    /// Rows and columns of the active sheet (the scroll bounds).
    pub extent: (u32, u32),
    /// The first row and column in view; the scene moves it.
    pub top_left: Cell,
    /// Charts and pictures of the active sheet.
    pub anchors: Vec<SheetAnchor>,
    /// Some content could not be drawn: the status bar says so quietly.
    pub missing_content: bool,
    /// The tabs' "more" menu (`sheets_tabs`): the sheets the renderer could not name, and whether the menu is open.
    pub more: More,
}

/// The sheet tabs' "more" menu: `unnamed` sheets of the workbook past the named tabs (the renderer's count), and
/// whether the menu is open.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct More {
    pub unnamed: u32,
    pub open: bool,
}

/// Its counts, never a cell.
impl fmt::Debug for SheetView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SheetView({} sheets, {} cells)", self.tabs.len(), self.cells.len())
    }
}

impl SheetView {
    /// Overwrite the decrypted words held here (see `OpenFile::wipe`).
    pub fn wipe(&mut self) {
        for tab in &mut self.tabs {
            tab.zeroize();
        }
        for cell in &mut self.cells {
            cell.text.zeroize();
            if let Some(f) = cell.formula.as_mut() {
                f.zeroize();
            }
            cell.value = None;
        }
        self.formula.zeroize();
        self.selection = None;
    }
}

/// "B10" for row 9, column 1; "AA1" for column 26.
pub fn a1(cell: Cell) -> String {
    format!("{}{}", col_letters(cell.col), cell.row + 1)
}

/// A column's letters: 0 is "A", 25 is "Z", 26 is "AA".
pub fn col_letters(col: u32) -> String {
    let mut n = col + 1;
    let mut out = Vec::new();
    while n > 0 {
        let rem = ((n - 1) % 26) as u8;
        out.push(b'A' + rem);
        n = (n - 1) / 26;
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

// ---------------------------------------------------------------------------
// The status line's figures
// ---------------------------------------------------------------------------

/// Sum and count of the numbers in the selection.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stats {
    pub sum: f64,
    pub count: usize,
}

impl Stats {
    pub fn average(self) -> Option<f64> {
        (self.count > 0).then(|| self.sum / self.count as f64)
    }
}

/// A number as a cell shows it ("1,652.50", "3,600"), `None` for words.
fn number(text: &str) -> Option<f64> {
    let plain: String = text.trim().chars().filter(|c| *c != ',').collect();
    if plain.is_empty() {
        return None;
    }
    plain.parse::<f64>().ok().filter(|v| v.is_finite())
}

/// The figures of the selection; `None` without a selection.
pub fn stats(view: &SheetView) -> Option<Stats> {
    let (a, b) = view.selection?;
    let (r0, r1) = (a.row.min(b.row), a.row.max(b.row));
    let (c0, c1) = (a.col.min(b.col), a.col.max(b.col));
    let mut out = Stats { sum: 0.0, count: 0 };
    for cell in &view.cells {
        if cell.row < r0 || cell.row > r1 || cell.col < c0 || cell.col > c1 {
            continue;
        }
        if let Some(v) = cell.value.or_else(|| number(&cell.text)) {
            out.sum += v;
            out.count += 1;
        }
    }
    Some(out)
}

/// `13220` as "13,220".
fn grouped(int: u64) -> String {
    let digits = int.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// A figure of the status line: whole numbers bare ("13,220"), others to two places ("1,652.50").
fn figure(v: f64) -> String {
    let sign = if v < 0.0 { "-" } else { "" };
    let v = v.abs();
    if (v - v.round()).abs() < 1e-9 {
        return format!("{sign}{}", grouped(v.round() as u64));
    }
    let hundredths = (v * 100.0).round() as u64;
    format!("{sign}{}.{:02}", grouped(hundredths / 100), hundredths % 100)
}

/// The words of the status line's left side, in order: where the sheet is, then the selection's figures.
pub fn status_words(view: &SheetView) -> Vec<String> {
    let sheets = view.tabs.len().saturating_add(view.more.unnamed as usize);
    let mut words = vec![format!("Sheet {} of {}", view.active + 1, sheets.max(1))];
    if let Some(stats) = stats(view) {
        match stats.average() {
            Some(avg) => {
                words.push(format!("Sum {}", figure(stats.sum)));
                words.push(format!("Average {}", figure(avg)));
                words.push(format!("Count {}", stats.count));
            }
            None => words.push("Count 0".to_string()),
        }
    }
    words
}

// ---------------------------------------------------------------------------
// The window
// ---------------------------------------------------------------------------

/// 12px/400 on 16px: a cell's words.
pub(crate) const CELL: TextStyle = TextStyle::new(12.0, 400.0).lh(16.0);
const CELL_BOLD: TextStyle = TextStyle::new(12.0, 650.0).lh(16.0);
/// 11.5px mono on 16px: a figure in a cell (the board's `font-mono t-num`), and its bold twin (the total row).
const CELL_MONO: TextStyle = TextStyle::new(11.5, 400.0).family(Family::Mono).lh(16.0);
const CELL_MONO_BOLD: TextStyle = TextStyle::new(11.5, 600.0).family(Family::Mono).lh(16.0);
/// 11px/500 on 14px: a column letter.
const HEADER: TextStyle = TextStyle::new(11.0, 500.0).lh(14.0);
/// 11px mono on 14px: a row number (the board's `font-mono`).
const ROW_NUMBER: TextStyle = TextStyle::new(11.0, 400.0).family(Family::Mono).lh(14.0);
/// 12px on 15px: the viewer bar's "Open in another app…" and the zoom figure's neighbours.
const BAR_LABEL: TextStyle = TextStyle::new(12.0, 400.0).lh(15.0);
const BAR_MONO: TextStyle = TextStyle::new(11.5, 400.0).family(Family::Mono).lh(15.0);
/// 12px/600 on 15px: the sheet tab that is open; 12px/500 the others.
const TAB_ON: TextStyle = TextStyle::new(12.0, 600.0).lh(15.0);
const TAB: TextStyle = TextStyle::new(12.0, 500.0).lh(15.0);
/// 11.5px mono on 17px: a cell's name and the formula (machine text).
pub(crate) const MONO: TextStyle = TextStyle::new(11.5, 400.0).family(Family::Mono).lh(17.0);
/// 11.5px/600 on 17px: "Encrypted".
const STATUS_ENC: TextStyle = TextStyle::new(11.5, 600.0).lh(17.0);

/// Widths in the board: the row numbers' column, the cell box, and the heights of the bars.
pub(crate) const ROW_HEAD_W: f32 = 40.0;
pub(crate) const HEADER_H: f32 = 24.0;
const FORMULA_BAR_H: f32 = 34.0;
/// The cell's name is plain text 92 wide on the board (no box).
const CELLBOX_W: f32 = 92.0;
const TABS_H: f32 = 30.0;
/// The viewer bar's height (`OvBar`).
const VBAR_H: f32 = 38.0;

/// A label as wide as its words.
fn fitted(key: impl Into<String>, words: &str, style: TextStyle, color: rune::Color) -> El {
    kit::label(key, words, style, color).w(rune::text::advance(words, style))
}

/// What the formula bar says of the current cell: its formula, else what the host prepared, else the cell's own text.
pub fn formula_text(view: &SheetView) -> String {
    let here = view
        .cells
        .iter()
        .find(|c| c.row == view.current.row && c.col == view.current.col);
    if let Some(f) = here.and_then(|c| c.formula.as_ref()) {
        return f.clone();
    }
    if !view.formula.is_empty() {
        return view.formula.clone();
    }
    here.map(|c| c.text.clone()).unwrap_or_default()
}

/// The formula bar: the cell's name as text, then the formula as text (no "fx" mark: the board draws none). No box around
/// either. In the editor `formula` is the element that shows and edits it (`edit::sheet::formula`); the viewer's is read-only.
fn formula_bar(view: &SheetView, formula: Option<El>, t: &Theme) -> El {
    let name = a1(view.current);
    let cellbox = row()
        .key("sh-cellbox")
        .role("group")
        .label(format!("Cell {name}"))
        .w(CELLBOX_W)
        .center_y()
        .child(kit::label("sh-cellbox-text", name.clone(), MONO, t.text));
    row()
        .key("sh-formula-bar")
        .role("group")
        .label("Formula bar")
        .w_full()
        .h(FORMULA_BAR_H)
        .px(12.0)
        .gap(8.0)
        .center_y()
        .fill(t.paper)
        .border_sides([0.0, 0.0, kit::HAIR, 0.0], t.separator)
        .border_layout()
        .child(cellbox)
        .child(formula.unwrap_or_else(|| {
            kit::label("sh-formula", formula_text(view), MONO, t.text)
                .truncate()
                .min_w(0.0)
        }))
}

static LESS: Icon = Icon {
    name: "sheets-less",
    view: 24.0,
    body: concat!(
        r#"<g fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">"#,
        r#"<path d="M5 12h14"></path></g>"#
    ),
};

static MORE: Icon = Icon {
    name: "sheets-more",
    view: 24.0,
    body: concat!(
        r#"<g fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">"#,
        r#"<path d="M12 5v14M5 12h14"></path></g>"#
    ),
};

/// Open in another app…: the file must be kept on this PC; if it is not, the press says so and does nothing else.
fn open_elsewhere(s: &mut State) {
    if let Some(ui) = viewers::ui(s) {
        if ui.file.details.offline {
            ui.intent(viewers::Intent::OpenInAnotherApp);
        } else {
            ui.toast = Some(KEEP_FIRST.to_string());
        }
    }
}

/// The viewer bar under the window's toolbar (the board's `OvBar`, no page words for a sheet): the zoom as "- 100% +"
/// at the left, "Open in another app…" as a small bordered button at the right. No ribbon.
fn viewer_bar(t: &Theme) -> El {
    let glyph = |key: &'static str, label: &'static str, icon: &'static Icon| {
        row()
            .key(key)
            .role("img")
            .label(label)
            .size(24.0, 24.0)
            .center()
            .child(kit::glyph(format!("{key}-icon"), icon, 13.0, t.text_secondary))
    };
    row()
        .key("vbar")
        .role("toolbar")
        .label("Spreadsheet")
        .w_full()
        .h(VBAR_H)
        .px(12.0)
        .gap(12.0)
        .center_y()
        .fill(t.card)
        .border_sides([0.0, 0.0, kit::HAIR, 0.0], t.separator)
        .border_layout()
        .child(
            row()
                .key("vbar-zoom")
                .role("group")
                .label("Zoom, 100 per cent")
                .gap(4.0)
                .center_y()
                .child(glyph("zoom-less", "Zoom out", &LESS))
                .child(fitted("zoom", "100%", BAR_MONO, t.text_secondary))
                .child(glyph("zoom-more", "Zoom in", &MORE)),
        )
        .child(el().key("vbar-gap").grow(1.0))
        .child(
            row()
                .key("elsewhere")
                .role("button")
                .label(OPEN_ELSEWHERE)
                .h(28.0)
                .px(10.0)
                .center_y()
                .radius(7.0)
                .fill(t.card)
                .border(1.0, t.card_border)
                .border_layout()
                .hover(t.hover)
                .press(t.press)
                .focusable(true)
                .on_press(open_elsewhere)
                .child(kit::label(
                    "elsewhere-label",
                    OPEN_ELSEWHERE,
                    BAR_LABEL,
                    t.text_secondary,
                )),
        )
}

/// One range's bounds as (first row, last row, first column, last column).
fn bounds(range: (Cell, Cell)) -> (u32, u32, u32, u32) {
    let (a, b) = range;
    (a.row.min(b.row), a.row.max(b.row), a.col.min(b.col), a.col.max(b.col))
}

/// Which rows and columns of the sheet are drawn, and where (`sheets_bounds::Side`).
pub struct Shown {
    rows: Side,
    cols: Side,
}

/// A row without a size yet, and a column without one (what the source fills gaps with).
pub(crate) const ROW_PT: f32 = 15.0;
pub(crate) const COL_PT: f32 = 48.0;

impl Shown {
    /// Everything the view holds (the fixture's workbook).
    fn all(view: &SheetView) -> Shown {
        Shown {
            rows: Side::all(&view.row_heights),
            cols: Side::all(&view.col_widths),
        }
    }

    /// The frozen rows and columns that fit `screen` (points), then those from `top_left` that fit the rest: at most a
    /// screen, however large the frozen panes, and at most `MAX_DRAWN_CELLS` cells.
    fn from_top_left(view: &SheetView, screen: (f32, f32)) -> Shown {
        let (r0, c0) = (view.top_left.row, view.top_left.col);
        let rows = Side::walk(&view.row_heights, view.frozen.0, r0, screen.1, ROW_PT);
        let mut cols = Side::walk(&view.col_widths, view.frozen.1, c0, screen.0, COL_PT);
        // The rows are at most `MAX_LINES`, so cutting the columns keeps the product within the budget.
        cols.keep((MAX_DRAWN_CELLS / rows.len().max(1)).max(1));
        Shown { rows, cols }
    }
}

/// How many rows and columns from `top_left` fill `screen` (at least one each).
pub fn fit(view: &SheetView, screen: (f32, f32)) -> (u32, u32) {
    let count = |sizes: &[f32], from: u32, room: f32, gap: f32| {
        let (mut used, mut n) = (0.0, 0u32);
        while used < room && n < MAX_LINES {
            used += sizes.get((from + n) as usize).copied().unwrap_or(gap);
            n += 1;
        }
        n.max(1)
    };
    (
        count(&view.row_heights, view.top_left.row, screen.1, ROW_PT),
        count(&view.col_widths, view.top_left.col, screen.0, COL_PT),
    )
}

/// Rows moved by Page Up and Page Down.
const PAGE_ROWS: u32 = 20;

/// The sheet's keys: arrows, Page Up and Down, Home and End move what is in view, never past the extent. `true`: taken.
pub fn scroll_key(view: &mut SheetView, key: &Key, m: &Mods) -> bool {
    if m.ctrl || m.alt || m.cmd {
        return false;
    }
    let (rows, cols) = (i64::from(view.extent.0), i64::from(view.extent.1));
    let (row, col) = (i64::from(view.top_left.row), i64::from(view.top_left.col));
    let (nr, nc) = match key {
        Key::Down => (row + 1, col),
        Key::Up => (row - 1, col),
        Key::Right => (row, col + 1),
        Key::Left => (row, col - 1),
        Key::PageDown => (row + i64::from(PAGE_ROWS), col),
        Key::PageUp => (row - i64::from(PAGE_ROWS), col),
        Key::Home => (0, 0),
        Key::End => (rows - 1, col),
        _ => return false,
    };
    view.top_left = Cell {
        row: nr.clamp(0, (rows - 1).max(0)) as u32,
        col: nc.clamp(0, (cols - 1).max(0)) as u32,
    };
    true
}

/// The charts and pictures of the sheet, each at its `from` cell plus its offset into that cell (the protocol's words),
/// from where this frame drew (or walked back to) that cell's row and column, clipped by the grid. A picture that has
/// not come draws its box empty.
fn anchors_of(view: &SheetView, shown: &Shown, live: Option<&Live>, t: &Theme) -> El {
    let mut layer = el().key("sh-anchors").absolute(0.0, 0.0).size(0.0, 0.0);
    let Some(live) = live else { return layer };
    for a in &view.anchors {
        let (Some(x), Some(y)) = (shown.cols.start_of(a.from.col), shown.rows.start_of(a.from.row)) else {
            continue;
        };
        let (x, y) = (x + a.x_pt, y + a.y_pt);
        if x + a.w_pt <= 0.0 || y + a.h_pt <= 0.0 || x >= live.screen.0 || y >= live.screen.1 {
            continue;
        }
        let picture = live.asks.get(OfficePicture::Anchor {
            sheet: view.sheet,
            anchor: a.anchor,
            zoom_pct: 100,
            dpi: live.dpi,
        });
        let mut b = el()
            .key(format!("sh-anchor-{}", a.anchor))
            .role("image")
            .label("Chart or picture")
            .absolute(ROW_HEAD_W + x, HEADER_H + y)
            .size(a.w_pt, a.h_pt);
        b = match picture {
            Some(p) => b.pixels_fit(&p, SurfaceFit::Fill),
            None => b.border(kit::HAIR, t.separator).border_layout(),
        };
        layer = layer.child(b);
    }
    layer
}

/// The grid: the corner and the column letters, then each row with its number and its cells.
fn grid(view: &SheetView, shown: &Shown, anchors: El, layer: Option<El>, t: &Theme) -> El {
    // Only the cells on a drawn row and column are indexed: a sheet held at its cell cap costs a scan, not 65,536 inserts.
    let (rows_in, cols_in): (std::collections::HashSet<u32>, std::collections::HashSet<u32>) =
        (shown.rows.iter().copied().collect(), shown.cols.iter().copied().collect());
    let by_place: HashMap<(u32, u32), &CellText> = view
        .cells
        .iter()
        .filter(|c| rows_in.contains(&c.row) && cols_in.contains(&c.col))
        .map(|c| ((c.row, c.col), c))
        .collect();
    let width = |c: u32| view.col_widths.get(c as usize).copied().unwrap_or(0.0);
    let merges = MergesShown::of(view, &shown.rows, &shown.cols, width);
    let selected = view.selection.map(bounds);
    let in_selection =
        |r: u32, c: u32| selected.is_some_and(|(r0, r1, c0, c1)| r >= r0 && r <= r1 && c >= c0 && c <= c1);
    let mut head = row().key("sh-head").h(HEADER_H).child(
        el().key("sh-corner")
            .size(ROW_HEAD_W, HEADER_H)
            .fill(t.surface)
            .border_sides([0.0, kit::HAIR, kit::HAIR, 0.0], t.separator)
            .border_layout(),
    );
    for &c in shown.cols.iter() {
        let w = &view.col_widths[c as usize];
        head = head.child(
            row()
                .key(format!("sh-col-{}", col_letters(c)))
                .w(*w)
                .h(HEADER_H)
                .center()
                .fill(t.surface)
                .border_sides([0.0, kit::HAIR, kit::HAIR, 0.0], t.separator)
                .border_layout()
                .child(fitted(
                    format!("sh-col-{}-t", col_letters(c)),
                    &col_letters(c),
                    HEADER,
                    t.text_tertiary,
                )),
        );
    }
    let mut body = col().key("sh-body");
    for (ri, &r) in shown.rows.iter().enumerate() {
        let h = &view.row_heights[r as usize];
        // A frozen boundary is a heavier line under the last frozen row (under the last one drawn when the screen cut
        // the frozen rows short).
        let cut_here = shown.rows.cut && ri + 1 == shown.rows.frozen;
        let under = if view.frozen.0 > 0 && (r + 1 == view.frozen.0 || cut_here) {
            1.0
        } else {
            kit::HAIR
        };
        let number = (r + 1).to_string();
        let mut line = row().key(format!("sh-line-{r}")).h(*h).child(
            row()
                .key(format!("sh-row-{r}"))
                .size(ROW_HEAD_W, *h)
                .center()
                .fill(t.surface)
                .border_sides([0.0, kit::HAIR, kit::HAIR, 0.0], t.separator)
                .border_layout()
                .child(fitted(format!("sh-row-{r}-t"), &number, ROW_NUMBER, t.text_tertiary)),
        );
        for (ci, &c) in shown.cols.iter().enumerate() {
            // A merge's first row spans its drawn columns; its other rows draw a blank as wide, so the cells after it
            // stay in their columns; only its last row has the line under it.
            let (w, under) = match merges.at(ri, ci) {
                AtCell::Covered => continue,
                AtCell::Own => (width(c), under),
                AtCell::Spans { w, last } => (w, if last { under } else { 0.0 }),
                AtCell::Blank { w, last } => {
                    let fill = if in_selection(r, c) { t.selection } else { t.paper };
                    line = line.child(
                        el().key(format!("sh-blank-{r}-{c}"))
                            .w(w)
                            .h(*h)
                            .fill(fill)
                            .border_sides([0.0, kit::HAIR, if last { under } else { 0.0 }, 0.0], t.separator)
                            .border_layout(),
                    );
                    continue;
                }
            };
            let here = Cell { row: r, col: c };
            let mut cell = row()
                .key(format!("sh-cell-{r}-{c}"))
                .w(w)
                .h(*h)
                .px(8.0)
                .center_y()
                .fill(if in_selection(r, c) { t.selection } else { t.paper });
            cell = if here == view.current {
                // The current cell: an ink ring, never Amber.
                cell.border(2.0, t.text).border_layout()
            } else {
                cell.border_sides([0.0, kit::HAIR, under, 0.0], t.separator)
                    .border_layout()
            };
            if let Some(text) = by_place.get(&(r, c)) {
                // The board sets figures (the right-aligned cells) in mono 11.5, words in sans 12.
                let style = match (text.right_aligned, text.bold) {
                    (true, true) => CELL_MONO_BOLD,
                    (true, false) => CELL_MONO,
                    (false, true) => CELL_BOLD,
                    (false, false) => CELL,
                };
                let label = fitted(format!("sh-cell-{r}-{c}-t"), &text.text, style, t.text);
                cell = if text.right_aligned {
                    cell.child(el().key(format!("sh-cell-{r}-{c}-gap")).grow(1.0))
                        .child(label)
                } else {
                    cell.child(label)
                };
            }
            line = line.child(cell);
        }
        body = body.child(line);
    }
    let grid = col()
        .key("sh-grid")
        .role("grid")
        .label("Sheet")
        .grow(1.0)
        .min_h(0.0)
        .w_full()
        .fill(t.paper)
        .clip_rounded(0.0)
        .child(head)
        .child(body)
        .child(anchors);
    // The editor's pointer, keys and cell editor lie over the cells (under the scroll viewports `window_with` adds).
    match layer {
        Some(layer) => grid.child(layer),
        None => grid,
    }
}

#[path = "sheets_scroll.rs"]
pub mod sheets_scroll;
pub use sheets_scroll::scroll_key_tx;

#[path = "sheets_tabs.rs"]
mod sheets_tabs;

#[path = "sheets_bounds.rs"]
mod sheets_bounds;
pub use sheets_bounds::Geometry;
use sheets_bounds::{AtCell, MAX_DRAWN_CELLS, MAX_LINES, MergesShown, Side};
use sheets_tabs::sheet_tabs;

/// A status word with its figure in mono, as the board sets it ("Sum " sans, "13,220" mono; "Count 8" is all sans).
fn status_word(key: String, word: &str, t: &Theme) -> El {
    let figure_at = ["Sum ", "Average "]
        .iter()
        .find(|lead| word.starts_with(**lead))
        .map(|lead| lead.len());
    let Some(at) = figure_at else {
        return fitted(key, word, type_::STATUS, t.text_tertiary);
    };
    let mono = type_::STATUS.family(Family::Mono);
    let width = rune::text::advance(&word[..at], type_::STATUS) + rune::text::advance(&word[at..], mono);
    kit::label(key, word, type_::STATUS, t.text_tertiary)
        .spans(vec![Span::new(at, word.len()).mono()])
        .w(width)
}

/// The status line (`OvStatus`): the sheet and the selection's figures on the left, then Encrypted, a middle dot and
/// About on the right. Its parts are keyed `st-<N>` in order, each middle dot between them numbered too: with the
/// board's four left words that is st-0 to st-6, Encrypted st-7, the dot st-8, About st-9. In the editor `kept` (how the
/// workbook is kept: "saved as version 4", "unsaved changes") follows Encrypted and its dot, then a dot more before About.
fn status(view: &SheetView, kept: Option<&str>, t: &Theme) -> El {
    let words = status_words(view);
    let mut bar = row()
        .key("status-bar")
        .role("status")
        .w_full()
        .h(28.0)
        .px(16.0)
        .gap(7.0)
        .center_y()
        .fill(t.status_bar_bg)
        .border_sides([kit::HAIR, 0.0, 0.0, 0.0], t.separator)
        .border_layout();
    let mut n = 0;
    for (i, word) in words.iter().enumerate() {
        if i > 0 {
            bar = bar.child(fitted(format!("st-{n}"), "\u{00B7}", type_::STATUS, t.text_tertiary));
            n += 1;
        }
        bar = bar.child(status_word(format!("st-{n}"), word, t));
        n += 1;
    }
    let mut said = String::new();
    if view.missing_content {
        // The board's OvStatus: after the left words, `·` and the one quiet line.
        bar = bar
            .child(fitted(format!("st-{n}"), "\u{00B7}", type_::STATUS, t.text_tertiary))
            .child(fitted(
                format!("st-{}", n + 1),
                QUIET_LINE,
                type_::STATUS,
                t.text_tertiary,
            ));
        n += 2;
        said = format!(". {QUIET_LINE}");
    }
    let enc = format!("st-{n}");
    let dot = format!("st-{}", n + 1);
    let (kept_at, about) = match kept {
        Some(_) => (
            Some((format!("st-{}", n + 2), format!("st-{}", n + 3))),
            format!("st-{}", n + 4),
        ),
        None => (None, format!("st-{}", n + 2)),
    };
    let kept_words = kept.map(|k| format!(", {k}")).unwrap_or_default();
    let mut bar = bar
        .label(format!("{}{said}. {ENCRYPTED}{kept_words}", words.join(", ")))
        .child(el().key("st-gap").grow(1.0))
        .child(
            row()
                .key(format!("{enc}-box"))
                .gap(4.0)
                .center_y()
                .child(kit::glyph(format!("{enc}-icon"), &icons::LOCK, 11.0, t.accent_deep))
                .child(fitted(enc, ENCRYPTED, STATUS_ENC, t.accent_deep)),
        )
        .child(fitted(dot, "\u{00B7}", type_::STATUS, t.text_tertiary));
    if let (Some(words), Some((kept_key, dot_key))) = (kept, kept_at) {
        bar = bar
            .child(fitted(kept_key, words, type_::STATUS, t.text_tertiary))
            .child(fitted(dot_key, "\u{00B7}", type_::STATUS, t.text_tertiary));
    }
    bar.child(
            el().key(format!("{about}-box"))
                .role("link")
                .label("About this viewer")
                .focusable(true)
                .on_press(|s: &mut State| {
                    if let Some(ui) = viewers::ui(s) {
                        ui.office_about();
                    }
                })
                .child({
                    kit::note_text(&about, &about, ABOUT, type_::STATUS, t.text_tertiary);
                    rich(ABOUT, vec![Span::new(0, ABOUT.len()).underline(t.text_tertiary)])
                        .key(about)
                        .font(type_::STATUS)
                        .color(t.text_tertiary)
                        .w(rune::text::advance(ABOUT, type_::STATUS))
                }),
        )
}

/// The window with the spreadsheet viewer: caption, sidebar, the window's toolbar, then the formula bar, the grid,
/// the sheet tabs and the status line. There is no ribbon and no edit control anywhere in it.
pub fn window(s: &State, cx: &mut Cx<State>, ui: &Ui, view: &SheetView) -> El {
    window_with(s, cx, ui, view, &Shown::all(view), None)
}

/// The window of a workbook the Office engine renders: the grid from the first row and column in view, the cells in
/// view and one screen more asked of the source each frame, the charts and pictures at their places.
pub fn window_live(
    s: &State,
    cx: &mut Cx<State>,
    ui: &Ui,
    view: &SheetView,
    live: &Live,
    cells: &dyn Fn(u32, u32, u32, u32, u32),
) -> El {
    let shown = Shown::from_top_left(view, live.screen);
    for (row, col, rows, cols) in asks(view, &shown, live.screen) {
        cells(view.sheet, row, col, rows, cols);
    }
    window_with(s, cx, ui, view, &shown, Some(live))
}

/// The blocks of cells one frame asks, as (first row, first column, rows, columns): the frozen block first (the corner,
/// then the frozen rows across the scrolled columns, then the frozen columns down the scrolled rows), then the cells in
/// view and one screen more. Together at most `MAX_DRAWN_CELLS` cells, the earlier blocks keeping theirs.
fn asks(view: &SheetView, shown: &Shown, screen: (f32, f32)) -> Vec<(u32, u32, u32, u32)> {
    let (rows, cols) = fit(view, screen);
    let (r0, c0) = (view.top_left.row, view.top_left.col);
    let (fr, fc) = (shown.rows.frozen_walked(), shown.cols.frozen_walked());
    let (sr, sc) = (rows.saturating_mul(2), cols.saturating_mul(2));
    let mut left = MAX_DRAWN_CELLS as u64;
    let mut out = Vec::new();
    for (row, col, h, w) in [(0, 0, fr, fc), (0, c0, fr, sc), (r0, 0, sr, fc), (r0, c0, sr, sc)] {
        if h == 0 || w == 0 || left == 0 {
            continue;
        }
        // A block over what is left keeps its rows and gives up columns.
        let h = u64::from(h).min(left);
        let w = u64::from(w).min((left / h).max(1));
        left -= (h * w).min(left);
        out.push((row, col, h as u32, w as u32));
    }
    out
}

fn window_with(s: &State, cx: &mut Cx<State>, ui: &Ui, view: &SheetView, shown: &Shown, live: Option<&Live>) -> El {
    let t = tokens::theme(cx.system_dark());
    // The workbook is open for editing (a live one): the grid takes the pointer and the keys, the formula bar edits.
    let editing = live.is_some()
        && ui
            .office
            .edit
            .as_ref()
            .is_some_and(|e| e.kind == beebeeb_office_protocol::office::EditKind::Xlsx);
    let (layer, formula) = if editing {
        let geometry = Geometry::of(view, &shown.rows, &shown.cols);
        let room = cx
            .layout_size("sh-grid")
            .map_or((800.0, 600.0), |[w, h]| ((w - ROW_HEAD_W).max(0.0), (h - HEADER_H).max(0.0)));
        (
            Some(edit_sheet::layer(view, geometry, room, &t)),
            Some(edit_sheet::formula(view, &t)),
        )
    } else {
        edit_sheet::reset();
        (None, None)
    };
    let mut cells = grid(view, shown, anchors_of(view, shown, live, &t), layer, &t);
    // A workbook in a renderer scrolls (`sheets_scroll`); the fixture's is the board's still sheet.
    if live.is_some() {
        let (down, across) = (cx.layout(sheets_scroll::DOWN), cx.layout(sheets_scroll::ACROSS));
        cells = cells.child(sheets_scroll::overlay(view, down, across, &t));
    }
    let room = cx.layout_size("sh-tabs").map(|[w, _]| w);
    let mut body = row().key("viewer-row").grow(1.0).min_h(0.0).w_full().child(
        col()
            .key("sheet")
            .grow(1.0)
            .min_w(0.0)
            .h_full()
            .child(viewer_bar(&t))
            .child(formula_bar(view, formula, &t))
            .child(cells)
            .child(sheet_tabs(view, live.is_some(), room, &t)),
    );
    if ui.show_details {
        body = body.child(viewers::details::panel(&ui.file, ui.knows_offline(), &t));
    }
    let main = col()
        .key("content-area")
        .role("main")
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .fill(t.bg)
        .child(viewers::office::toolbar(s, ui, false, &t));
    let main = match ui.office.edit.as_ref() {
        // The editor's frame, with the sheet's own status line: the figures of the selection, then how it is kept
        // (`edit::chrome`'s is the document's: pages and words).
        Some(edit) if editing => {
            let kept = if edit.unsaved() {
                "unsaved changes".to_string()
            } else {
                format!("saved as version {}", ui.file.version)
            };
            main.child(
                col()
                    .key("edit-chrome")
                    .grow(1.0)
                    .min_h(0.0)
                    .w_full()
                    .child(super::edit::sheet_ribbon::ribbon(ui, edit.unsaved(), &t))
                    .child(body)
                    .child(status(view, Some(&kept), &t)),
            )
        }
        Some(edit) => main.child(super::edit::chrome(s, cx, ui, edit.kind, body)),
        None => main.child(body).child(status(view, None, &t)),
    };
    let mut under = el().key("under-caption").grow(1.0).min_h(0.0).w_full().child(
        row()
            .key("body")
            .w_full()
            .h_full()
            .child(sidebar::sidebar(s, &t))
            .child(main),
    );
    if let Some(words) = &ui.toast {
        under = under.child(viewers::dialogs::toast(words, &t));
    }
    col()
        .key("window")
        .role("window")
        .label(s.identity.main_window_title())
        .w_full()
        .h_full()
        .fill(t.bg)
        .child(caption::caption_bar(s.identity.main_window_title(), &t, cx))
        .child(under)
}

#[cfg(test)]
mod tests {
    use rune::{Env, Runtime};

    use super::*;
    use crate::{init, view};

    fn runtime_of(state: State, dark: bool) -> Runtime<State> {
        let env = Env {
            system_dark: dark,
            scale: 2.0,
        };
        let mut rt = Runtime::new(state, view, (1278.0, 798.0), env);
        rt.on_key = Some(crate::shell::host::on_key);
        rt.poll = Some(crate::shell::host::poll);
        rt.frame(0.0);
        rt
    }

    fn scene_state() -> State {
        let mut state = init();
        (FRAMES[0].1)(&mut state);
        state
    }

    fn sheet_of(state: &mut State) -> &mut SheetView {
        let ui = viewers::ui(state).expect("the scene opened a file");
        match &mut ui.file.body {
            Body::Sheet(view) => view,
            other => panic!("the scene opened {other:?}, not a sheet"),
        }
    }

    fn find(rt: &Runtime<State>, key: &str) -> Option<rune::ElementInfo> {
        rt.elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
    }

    fn words(rt: &Runtime<State>, key: &str) -> String {
        find(rt, key)
            .unwrap_or_else(|| panic!("no element keyed {key}"))
            .text
            .clone()
            .unwrap_or_default()
    }

    #[test]
    fn office_sheets_viewer_draws_from_sheetview() {
        let rt = runtime_of(scene_state(), false);
        let fixture = sheets_fixture::board();
        for cell in &fixture.cells {
            let key = format!("sh-cell-{}-{}-t", cell.row, cell.col);
            assert_eq!(words(&rt, &key), cell.text, "{key}");
        }
        let drawn = rt
            .elements()
            .into_iter()
            .filter(|e| !e.exiting)
            .filter(|e| {
                e.key
                    .as_deref()
                    .is_some_and(|k| k.starts_with("sh-cell-") && k.ends_with("-t"))
            })
            .count();
        assert_eq!(drawn, fixture.cells.len(), "no cell text that is not in the SheetView");
        assert_eq!(words(&rt, "sh-cellbox-text"), "B10");
        assert_eq!(words(&rt, "sh-formula"), "=SUM(B2:B9)");
    }

    /// The scene sets what the frame names: the sheet open, its first tab, B2:B9 selected, B10 current, no ribbon;
    /// and DW-1559 (0.9.1) is still the placeholder.
    #[test]
    fn office_sheets_scenes_set_their_state() {
        let mut state = scene_state();
        let view = sheet_of(&mut state).clone();
        assert_eq!(view.active, 0);
        assert_eq!(view.selection, Some((Cell { row: 1, col: 1 }, Cell { row: 8, col: 1 })));
        assert_eq!(a1(view.current), "B10");
        let rt = runtime_of(state, false);
        assert!(find(&rt, "rib").is_none(), "a viewer has no ribbon");
        let row_1559 = FRAMES.iter().find(|(id, _)| *id == "DW-1559").expect("the DW-1559 row");
        assert_eq!(
            row_1559.1 as usize,
            placeholder::scene as usize,
            "DW-1559 is a 0.9.1 frame"
        );
    }

    #[test]
    fn office_sheets_viewer_has_no_edit_control() {
        for dark in [false, true] {
            let rt = runtime_of(scene_state(), dark);
            let banned = ["save", "undo", "redo", "bold", "insert", "data", "formulas", "+"];
            for e in rt.elements().into_iter().filter(|e| !e.exiting) {
                let key = e.key.clone().unwrap_or_default();
                assert!(!key.starts_with("rib"), "ribbon element {key}");
                for said in [e.label.clone(), e.text.clone()].into_iter().flatten() {
                    let said = said.trim().to_lowercase();
                    assert!(!banned.contains(&said.as_str()), "{key} says {said:?}");
                }
            }
        }
    }

    #[test]
    fn office_sheets_status_follows_the_selection() {
        let view = sheets_fixture::board();
        assert_eq!(
            status_words(&view),
            ["Sheet 1 of 3", "Sum 13,220", "Average 1,652.50", "Count 8"]
        );
        let mut state = scene_state();
        let rt = runtime_of(scene_state(), false);
        assert_eq!(words(&rt, "st-0"), "Sheet 1 of 3");
        assert_eq!(words(&rt, "st-2"), "Sum 13,220");
        assert_eq!(words(&rt, "st-4"), "Average 1,652.50");
        assert_eq!(words(&rt, "st-6"), "Count 8");
        // Without a selection the figures are gone, not zero.
        sheet_of(&mut state).selection = None;
        assert_eq!(status_words(sheet_of(&mut state)), ["Sheet 1 of 3"]);
        let rt = runtime_of(state, false);
        assert_eq!(words(&rt, "st-0"), "Sheet 1 of 3");
        // st-1 is Encrypted and st-2 the dot after it: no figure took the place of either.
        assert_eq!(words(&rt, "st-1"), "Encrypted");
        assert_eq!(words(&rt, "st-2"), "\u{00B7}");
        assert!(
            rt.elements()
                .iter()
                .all(|e| !e.text.as_deref().is_some_and(|w| w.starts_with("Sum "))),
            "no Sum without a selection"
        );
    }

    /// DW-1508: the selection (B2:B9) lights no header: every column letter and row number keeps the plain fill.
    #[test]
    fn office_sheets_headers_are_not_highlighted() {
        for dark in [false, true] {
            let rt = runtime_of(scene_state(), dark);
            let plain = tokens::theme(dark).surface.hex();
            for k in [
                "sh-col-A", "sh-col-B", "sh-col-C", "sh-row-0", "sh-row-1", "sh-row-5", "sh-row-8", "sh-row-9",
            ] {
                assert_eq!(find(&rt, k).expect(k).fill.hex(), plain, "{k} is highlighted");
            }
        }
    }

    #[test]
    fn office_sheets_words_from_the_board() {
        let rt = runtime_of(scene_state(), false);
        for (i, tab) in ["2026", "2025", "Notes"].iter().enumerate() {
            assert_eq!(words(&rt, &format!("sh-tab-{i}-t")), *tab);
        }
        assert!(find(&rt, "sh-tab-3").is_none(), "no fourth tab, no plus");
        assert!(find(&rt, "sh-fx").is_none(), "the board draws no fx");
        for (c, letter) in ["A", "B", "C"].iter().enumerate() {
            assert_eq!(words(&rt, &format!("sh-col-{}-t", col_letters(c as u32))), *letter);
        }
        assert_eq!(a1(Cell { row: 0, col: 26 }), "AA1");
    }

    #[test]
    fn office_sheets_no_amber_outside_primary() {
        let amber = crate::theme::tokens::amber();
        for dark in [false, true] {
            let rt = runtime_of(scene_state(), dark);
            for e in rt.elements().into_iter().filter(|e| !e.exiting) {
                let key = e.key.clone().unwrap_or_default();
                // The window's New button is the one primary action.
                if key.starts_with("new-") || key == "new-btn" {
                    continue;
                }
                assert_ne!(e.fill.hex(), amber.hex(), "{key} is Amber");
            }
        }
    }

    /// The status line ends "Encrypted · About": the board's st-7 Encrypted, st-8 the dot, st-9 About.
    #[test]
    fn office_sheets_status_bar_ends_with_encrypted_and_about() {
        for dark in [false, true] {
            let rt = runtime_of(scene_state(), dark);
            assert_eq!(words(&rt, "st-7"), "Encrypted");
            assert_eq!(words(&rt, "st-8"), "\u{00B7}");
            assert_eq!(words(&rt, "st-9"), "About");
            assert!(find(&rt, "st-9-box").is_some(), "About is a link");
            assert!(find(&rt, "st-10").is_none(), "nothing after About");
        }
    }

    /// DW-1508 shows the board's sheet: `SH_DATA` in 8 columns and 19 rows, header "Item / Q1 / Q2 / Q3 / Total".
    #[test]
    fn office_sheets_fixture_is_the_boards_sheet() {
        let view = sheets_fixture::board();
        let header: Vec<&str> = (0..5)
            .map(|c| {
                view.cells
                    .iter()
                    .find(|x| x.row == 0 && x.col == c)
                    .map(|x| x.text.as_str())
                    .unwrap_or("")
            })
            .collect();
        assert_eq!(header, ["Item", "Q1", "Q2", "Q3", "Total"]);
        assert_eq!(view.row_heights.len(), 19);
        assert_eq!(view.col_widths.len(), 8);
        let at = |r: u32, c: u32| {
            view.cells
                .iter()
                .find(|x| x.row == r && x.col == c)
                .map(|x| x.text.clone())
                .unwrap_or_default()
        };
        assert_eq!(at(1, 0), "Studio rent");
        assert_eq!(at(9, 1), "13,220");
        assert_eq!(at(12, 3), "\u{2212}115");
        // B10 is the current cell and holds the selection's sum.
        assert_eq!(a1(view.current), "B10");
        assert_eq!(stats(&view).map(|s| s.count), Some(8));
    }

    /// The open sheet's tab is a pill (a fill), not an underline; the others have no fill.
    #[test]
    fn office_sheets_current_tab_is_a_pill() {
        for dark in [false, true] {
            let rt = runtime_of(scene_state(), dark);
            let paper = tokens::theme(dark).paper.hex();
            assert!(find(&rt, "sh-tab-on").is_none(), "no underline element");
            assert_eq!(find(&rt, "sh-tab-0").expect("the open tab").fill.hex(), paper);
            assert_ne!(find(&rt, "sh-tab-1").expect("another tab").fill.hex(), paper);
        }
    }

    /// The viewer bar of the board: zoom "100%" and "Open in another app\u{2026}", no page words.
    #[test]
    fn office_sheets_viewer_bar_has_zoom_and_open_elsewhere() {
        let rt = runtime_of(scene_state(), false);
        assert_eq!(words(&rt, "zoom"), "100%");
        assert_eq!(words(&rt, "elsewhere-label"), OPEN_ELSEWHERE);
        assert!(find(&rt, "vbar-page").is_none(), "a sheet has no page words");
    }
}

#[cfg(test)]
#[path = "sheets_live_tests.rs"]
mod live_tests;

#[cfg(test)]
#[path = "sheets_budget_tests.rs"]
mod budget_tests;
