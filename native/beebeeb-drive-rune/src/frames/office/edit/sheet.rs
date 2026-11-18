//! The sheet surface's keys and pointer (a later task fills this).
#![allow(dead_code)]

// What the surface does (DW-1508e's grid and formula bar): click a cell, Shift+click or drag a range, the arrows, Tab,
// Enter, Home, Page Up and Down and Ctrl+arrows move (Shift extends), typing, F2 or a double press opens the cell editor
// (a Rune text field over the cell), Enter or Tab commits it with ONE `SetCell`, Escape cancels. The formula bar shows
// the current cell's formula or text and edits it the same way.
//
// What the surface holds is the workbook's own: the current cell is the selection's anchor and `selection` runs from it
// to the far corner (`current` and `selection` are what a new body of the same sheet keeps, `source.rs`), so a click is
// `selection = None` (the status line shows no figures for one cell). The text being typed is the one thing more:
// `Editing` and its text field, kept per window thread (see `cell_edit`).
//
// The surface lies over the grid as one focusable layer (`edit-sheet-grid`): one `on_key`, one `on_drag` and the hit
// table of the frame (`sheets::Geometry`), never a handler per cell. A settled selection (a key's move, a press, the end
// of a drag) is told to the helper with one `SelectCells`, never one per pointer move: the ribbon's number formats,
// fill, AutoSum and Sort act on it. What a `SetCell` changes comes back as `Edited.cells` and the source re-asks only
// those blocks; nothing here draws a value that was not asked of the renderer.

use std::cell::UnsafeCell;

use beebeeb_office_protocol::check::MAX_OP_TEXT_BYTES;
use beebeeb_office_protocol::office::{CellRange, EditOp};
use rune::{Drag, Key, KeyEvent, Phase, TextEdit, Tx, el, text_input};

use crate::State;
use crate::frames::office::sheets::{self, Cell, Geometry, SheetView, sheets_scroll};
use crate::frames::viewers;
use crate::frames::viewers::source::Body;
use crate::theme::tokens::Theme;

/// The surface over the grid: the keys that start with `edit-` are the editor's (`edit::FOCUS_PREFIX`).
pub const GRID_KEY: &str = "edit-sheet-grid";
/// The text field over the current cell while it is edited there, and the formula bar's while it is edited there.
pub const EDITOR_KEY: &str = "edit-sheet-cell";
pub const BAR_KEY: &str = "edit-sheet-formula";
/// The formula bar's text before it is pressed (it opens the editor on the formula).
pub const BAR_OPEN_KEY: &str = "edit-sheet-formula-open";
/// The cell box: type a cell or a range, Enter goes there.
pub const NAME_KEY: &str = "edit-sheet-cellbox";

/// What a Delete over a selection clears at most (one `SetCell` each); more is refused with words.
pub const CLEAR_MAX: usize = 256;

pub const TOO_LONG: &str = "That is too long for one cell.";
pub const NOT_TAKEN: &str = "The change was not taken. Try again.";
pub const TOO_MANY: &str = "Too many cells to clear at once. Select fewer.";

/// The editor is at least this wide whatever the column is (a narrow column still shows what is typed).
const EDITOR_MIN_W: f32 = 96.0;

/// Where the text being typed goes: the cell itself, or the formula bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Editing {
    pub sheet: u32,
    pub cell: Cell,
    pub in_bar: bool,
}

/// The grid's state that is not the workbook's.
struct Grid {
    editing: Option<Editing>,
    edit: TextEdit,
    /// A press on a cell holds the pointer: the drag extends the selection.
    dragging: bool,
    /// What the last frame drew, for the handlers that run between frames.
    geometry: Geometry,
    room: (f32, f32),
    /// The cell box's field, and whether it is being typed in (else it shows the current cell's name).
    name: TextEdit,
    naming: bool,
    /// The grid has been drawn since the file was opened, and still owes the keyboard its first focus.
    opened: bool,
    wants_focus: bool,
}

impl Grid {
    fn new() -> Grid {
        Grid {
            editing: None,
            edit: TextEdit::new(String::new()),
            dragging: false,
            geometry: Geometry::default(),
            room: (800.0, 600.0),
            name: TextEdit::new(String::new()),
            naming: false,
            opened: false,
            wants_focus: false,
        }
    }
}

thread_local! {
    /// The window's one grid. Rune's text field reaches its `TextEdit` through a function of the app state
    /// ([`cell_edit`]), and the state of a sheet's window has no place for one that this task owns; it stays with the
    /// window's thread, the only one that builds the view and runs its handlers.
    static GRID: UnsafeCell<Grid> = UnsafeCell::new(Grid::new());
}

/// Run `f` on the grid's state. Never nested, and never held across a call back into Rune.
fn with<R>(f: impl FnOnce(&mut Grid) -> R) -> R {
    // SAFETY: the cell is thread-local, so no other thread reaches it; every use is one short call of `f`, which does
    // not call `with` again (nothing here nests), so there is never a second reference to it alive.
    GRID.with(|g| f(unsafe { &mut *g.get() }))
}

/// The text field's `TextEdit`, for `on_edit`. The field and the key handlers never hold it together.
pub fn cell_edit(_s: &mut State) -> &mut TextEdit {
    // SAFETY: as `with`: the window's thread only. Rune takes this borrow for one edit and lets it go before it calls
    // anything of ours, and `with` is never open while Rune runs.
    GRID.with(|g| unsafe { &mut (*g.get()).edit })
}

/// The cell being edited and where, if one is.
pub fn editing() -> Option<Editing> {
    with(|g| g.editing)
}

/// The words typed so far (a copy).
pub fn typed() -> String {
    with(|g| g.edit.text.clone())
}

/// Drop what was typed, overwriting it first, and the pointer's hold: the file closed, or the editor is not shown.
pub fn reset() {
    with(|g| {
        if g.editing.is_some() || !g.edit.text.is_empty() {
            g.edit.clear();
            g.edit.focused = false;
        }
        g.editing = None;
        g.dragging = false;
        g.name.clear();
        g.name.focused = false;
        g.naming = false;
        g.opened = false;
        g.wants_focus = false;
    });
}

/// The keyboard goes to the grid the first time it is shown for an open file (the shell calls this each turn, next to the
/// viewer's own one-shot focus). `true`: it moved.
pub fn take_focus(tx: &mut Tx) -> bool {
    if !with(|g| std::mem::take(&mut g.wants_focus)) {
        return false;
    }
    tx.focus(GRID_KEY, false);
    true
}

/// The cell box's field, for `on_edit`.
pub fn namebox_edit(_s: &mut State) -> &mut TextEdit {
    // SAFETY: as `cell_edit`.
    GRID.with(|g| unsafe { &mut (*g.get()).name })
}

fn leave_namebox(tx: &mut Tx) {
    with(|g| {
        g.name.clear();
        g.name.focused = false;
        g.naming = false;
    });
    tx.focus(GRID_KEY, false);
}

/// The cell box's keys: Enter goes to what was typed (a wrong reference stays in the box, with the words saying why),
/// Escape gives the keyboard back to the grid.
fn namebox_key(s: &mut State, ev: &KeyEvent, tx: &mut Tx) -> bool {
    match ev.key {
        Key::Enter => {
            let typed = with(|g| g.name.text.clone());
            let room = with(|g| g.room);
            if super::sheet_ribbon::go_to(s, tx, &typed, room) {
                leave_namebox(tx);
            }
            true
        }
        Key::Escape => {
            leave_namebox(tx);
            true
        }
        _ => false,
    }
}

/// The cell box of the open workbook: the current cell's name, which a press turns into a field to type a reference in.
pub fn cell_box(view: &SheetView, t: &Theme) -> rune::El {
    let name = sheets::a1(view.current);
    with(|g| {
        if !g.naming {
            g.name.replace_text(name.clone());
        }
    });
    with(|g| text_input(&g.name, sheets::MONO))
        .key(NAME_KEY)
        .label(format!("Cell {name}"))
        .single_line()
        .color(t.text)
        .caret(t.accent_deep, t.selection)
        .on_edit(namebox_edit)
        .on_input_key(namebox_key)
        .on_press_tx(|_: &mut State, _: &mut Tx| {
            with(|g| {
                if !g.naming {
                    g.naming = true;
                    g.name.clear();
                }
                g.name.focused = true;
            });
        })
        .w(sheets::CELLBOX_W)
}

// ---------------------------------------------------------------------------
// The workbook: selection and movement
// ---------------------------------------------------------------------------

/// The far corner of the selection (the current cell without one).
pub fn head(view: &SheetView) -> Cell {
    match view.selection {
        Some((a, b)) if a == view.current => b,
        Some((a, _)) => a,
        None => view.current,
    }
}

/// The last row and column the sheet has.
fn last(view: &SheetView) -> Cell {
    Cell {
        row: view.extent.0.saturating_sub(1),
        col: view.extent.1.saturating_sub(1),
    }
}

/// `cell` inside the sheet.
fn clamp(view: &SheetView, cell: Cell) -> Cell {
    let end = last(view);
    Cell {
        row: cell.row.min(end.row),
        col: cell.col.min(end.col),
    }
}

/// `anchor` is the current cell and the selection runs to `head`; one cell is no selection.
pub fn select(view: &mut SheetView, anchor: Cell, head: Cell) {
    view.current = anchor;
    view.selection = (head != anchor).then_some((anchor, head));
}

/// The selection as the helper is told it: its bounds, or the current cell.
pub fn range_of(view: &SheetView) -> CellRange {
    let (a, b) = view.selection.unwrap_or((view.current, view.current));
    let (r0, r1) = (a.row.min(b.row), a.row.max(b.row));
    let (c0, c1) = (a.col.min(b.col), a.col.max(b.col));
    CellRange {
        row: r0,
        col: c0,
        rows: r1 - r0 + 1,
        cols: c1 - c0 + 1,
    }
}

/// `from` moved by (`dr`, `dc`) cells, kept inside the sheet.
fn shifted(view: &SheetView, from: Cell, dr: i64, dc: i64) -> Cell {
    let at = |v: u32, d: i64, end: u32| (i64::from(v) + d).clamp(0, i64::from(end)) as u32;
    let end = last(view);
    Cell {
        row: at(from.row, dr, end.row),
        col: at(from.col, dc, end.col),
    }
}

/// Whether a held cell has words or a formula.
fn filled(c: &sheets::CellText) -> bool {
    !c.text.is_empty() || c.formula.is_some()
}

/// Ctrl+arrow: along (`dr`, `dc`) from `from` to the end of the run of filled cells it is in, else to the next filled
/// cell, else to the edge of the sheet. (Only the cells held are known: past them a run is not seen, and the jump goes
/// on to the next one held or the edge.)
pub fn jump(view: &SheetView, from: Cell, dr: i64, dc: i64) -> Cell {
    // Where along the line walked a held cell is: its distance from `from` in the direction of the walk.
    let along = |c: &sheets::CellText| {
        if dc == 0 {
            (i64::from(c.row) - i64::from(from.row)) * dr
        } else {
            (i64::from(c.col) - i64::from(from.col)) * dc
        }
    };
    let on_line = |c: &&sheets::CellText| filled(c) && if dc == 0 { c.col == from.col } else { c.row == from.row };
    let mut ahead: Vec<i64> = view
        .cells
        .iter()
        .filter(on_line)
        .map(along)
        .filter(|d| *d >= 0)
        .collect();
    ahead.sort_unstable();
    ahead.dedup();
    let full = |d: i64| ahead.binary_search(&d).is_ok();
    let at = |d: i64| shifted(view, from, dr * d, dc * d);
    let edge = if dr > 0 {
        Cell {
            row: last(view).row,
            col: from.col,
        }
    } else if dr < 0 {
        Cell { row: 0, col: from.col }
    } else if dc > 0 {
        Cell {
            row: from.row,
            col: last(view).col,
        }
    } else {
        Cell { row: from.row, col: 0 }
    };
    if at(1) == from {
        return from;
    }
    if full(0) && full(1) {
        // To the end of the run.
        let mut d = 1;
        while full(d + 1) {
            d += 1;
        }
        return at(d);
    }
    // To the next filled cell (the next one when it is filled and this one is not); else the edge.
    match ahead.iter().copied().find(|d| *d >= 1) {
        Some(d) => at(d),
        None => edge,
    }
}

/// The first line in view so that `line` is wholly in view in `room` points: `first` if it already is, the line itself
/// when it is above `first`, else the first line that leaves `line` at the far edge. Frozen lines stay in view whatever
/// is scrolled, and take their room.
fn first_for(sizes: &[f32], gap: f32, frozen: u32, first: u32, line: u32, room: f32) -> u32 {
    let size = |l: u32| sizes.get(l as usize).copied().unwrap_or(gap);
    if line < frozen {
        return first;
    }
    if line < first {
        return line;
    }
    let taken: f32 = (0..frozen.min(first)).take(4096).map(size).sum();
    let avail = (room - taken).max(size(line));
    let (mut used, mut top) = (size(line), line);
    let floor = first.max(frozen);
    while top > floor && used + size(top - 1) <= avail {
        top -= 1;
        used += size(top);
    }
    top.max(first)
}

/// The sheet scrolled so `cell` shows, in a grid `room` points of cells wide and high.
pub fn reveal(view: &mut SheetView, cell: Cell, room: (f32, f32)) {
    let row = first_for(
        &view.row_heights,
        sheets::ROW_PT,
        view.frozen.0,
        view.top_left.row,
        cell.row,
        room.1,
    );
    let col = first_for(
        &view.col_widths,
        sheets::COL_PT,
        view.frozen.1,
        view.top_left.col,
        cell.col,
        room.0,
    );
    view.top_left = Cell { row, col };
}

// ---------------------------------------------------------------------------
// Calls into the window's state
// ---------------------------------------------------------------------------

/// The open sheet.
fn with_sheet<R>(s: &mut State, f: impl FnOnce(&mut SheetView) -> R) -> Option<R> {
    match &mut viewers::ui(s)?.file.body {
        Body::Sheet(view) => Some(f(view)),
        _ => None,
    }
}

/// Hand one edit to the editor behind the open file. `false`: not taken.
fn send(s: &mut State, op: EditOp) -> bool {
    let Some(ui) = viewers::ui(s) else { return false };
    let (source, id) = ui.edit_source();
    let id = id.to_string();
    source.edit(&id, op)
}

fn say(s: &mut State, words: &str) {
    if let Some(ui) = viewers::ui(s) {
        ui.toast = Some(words.to_string());
    }
}

/// Tell the helper the selection that settled.
fn settle(s: &mut State) {
    let Some((sheet, range)) = with_sheet(s, |v| (v.sheet, range_of(v))) else {
        return;
    };
    send(s, EditOp::SelectCells { sheet, range });
}

// ---------------------------------------------------------------------------
// The cell editor
// ---------------------------------------------------------------------------

/// How the editor opens.
pub enum Begin {
    /// With these words typed over what the cell held (a key).
    Typed(String),
    /// On the cell's own formula or text (F2, a double press, a press on the formula bar).
    Existing { in_bar: bool },
}

/// Open the editor on the current cell. `false`: no sheet is open.
pub fn begin(s: &mut State, tx: &mut Tx, how: Begin) -> bool {
    let room = with(|g| g.room);
    let Some((sheet, cell, held_text)) = with_sheet(s, |v| {
        reveal(v, v.current, room);
        sheets_scroll::follow(v, tx);
        (v.sheet, v.current, sheets::formula_text(v))
    }) else {
        return false;
    };
    let (text, in_bar) = match how {
        Begin::Typed(text) => (text, false),
        Begin::Existing { in_bar } => (held_text, in_bar),
    };
    with(|g| {
        g.edit.replace_text(text);
        g.edit.focused = true;
        g.editing = Some(Editing { sheet, cell, in_bar });
    });
    true
}

/// Close the editor and drop its words.
fn close(tx: &mut Tx) {
    with(|g| {
        g.edit.clear();
        g.edit.focused = false;
        g.editing = None;
    });
    tx.focus(GRID_KEY, false);
}

/// Escape: the words go, the cell stays as it was.
pub fn cancel(tx: &mut Tx) {
    close(tx);
}

/// Enter or Tab (or a press on another cell): the words become the cell's, with one `SetCell`, and the current cell
/// moves by `advance` (rows, columns). Words the helper cannot take stay in the editor with a line saying why. `true`:
/// the editor was open.
pub fn commit(s: &mut State, tx: &mut Tx, advance: Option<(i64, i64)>) -> bool {
    let Some(at) = editing() else { return false };
    let text = typed();
    if text.len() > MAX_OP_TEXT_BYTES {
        say(s, TOO_LONG);
        return true;
    }
    let op = EditOp::SetCell {
        sheet: at.sheet,
        row: at.cell.row,
        col: at.cell.col,
        text,
    };
    if !send(s, op) {
        say(s, NOT_TAKEN);
        return true;
    }
    close(tx);
    if let Some((dr, dc)) = advance {
        go(s, tx, false, |v, from| shifted(v, from, dr, dc));
    }
    true
}

/// The keys of the text field (the cell's and the formula bar's): Enter and Tab commit and move, Escape cancels. The
/// rest are the field's. `true`: taken.
pub fn editor_key(s: &mut State, ev: &KeyEvent, tx: &mut Tx) -> bool {
    let back = ev.mods.shift;
    match ev.key {
        Key::Enter => commit(s, tx, Some((if back { -1 } else { 1 }, 0))),
        Key::Tab => commit(s, tx, Some((0, if back { -1 } else { 1 }))),
        Key::Escape if editing().is_some() => {
            cancel(tx);
            true
        }
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Keys
// ---------------------------------------------------------------------------

/// Move the current cell (or, extending, the far corner of the selection) to what `target` makes of where it is, show
/// it, and tell the helper. `true`: taken.
fn go(s: &mut State, tx: &mut Tx, extend: bool, target: impl FnOnce(&SheetView, Cell) -> Cell) -> bool {
    let room = with(|g| g.room);
    let moved = with_sheet(s, |view| {
        let from = if extend { head(view) } else { view.current };
        let to = clamp(view, target(view, from));
        if extend {
            select(view, view.current, to);
        } else {
            select(view, to, to);
        }
        reveal(view, to, room);
        sheets_scroll::follow(view, tx);
    });
    if moved.is_none() {
        return false;
    }
    settle(s);
    true
}

/// Delete over the selection: each held filled cell is set empty, at most [`CLEAR_MAX`].
fn clear(s: &mut State) -> bool {
    let Some((sheet, cells)) = with_sheet(s, |view| {
        let r = range_of(view);
        let mut cells: Vec<Cell> = view
            .cells
            .iter()
            .filter(|c| c.row >= r.row && c.row < r.row + r.rows && c.col >= r.col && c.col < r.col + r.cols)
            .filter(|c| filled(c))
            .map(|c| Cell { row: c.row, col: c.col })
            .collect();
        cells.sort_by_key(|c| (c.row, c.col));
        (view.sheet, cells)
    }) else {
        return false;
    };
    if cells.len() > CLEAR_MAX {
        say(s, TOO_MANY);
        return true;
    }
    for cell in cells {
        let (row, col) = (cell.row, cell.col);
        send(
            s,
            EditOp::SetCell {
                sheet,
                row,
                col,
                text: String::new(),
            },
        );
    }
    true
}

/// Text a key typed that starts an edit: something to show, not a control, and not a shortcut (Ctrl or Cmd without Alt;
/// Ctrl+Alt is AltGr, which types).
fn typed_text(ev: &KeyEvent) -> Option<&str> {
    let Key::Text(text) = &ev.key else { return None };
    let shortcut = (ev.mods.cmd || ev.mods.ctrl) && !ev.mods.alt;
    (!shortcut && !text.is_empty() && !text.chars().any(char::is_control)).then_some(text.as_str())
}

/// A key for the surface. `false`: not taken.
pub fn on_key(s: &mut State, tx: &mut Tx, ev: &KeyEvent) -> bool {
    // While the editor is open the field has the keys (`editor_key` is the field's own).
    if editing().is_some() {
        return false;
    }
    let m = ev.mods;
    if super::sheet_ribbon::is_auto_sum(ev) {
        super::sheet_ribbon::send(s, tx, EditOp::AutoSum, "Sum");
        return true;
    }
    if let Some(text) = typed_text(ev) {
        return begin(s, tx, Begin::Typed(text.to_string()));
    }
    if m.alt {
        return false;
    }
    let jump_by = m.ctrl || m.cmd;
    let extend = m.shift;
    let room = with(|g| g.room);
    let by = |dr: i64, dc: i64| {
        move |v: &SheetView, from: Cell| {
            if jump_by {
                jump(v, from, dr, dc)
            } else {
                shifted(v, from, dr, dc)
            }
        }
    };
    match ev.key {
        Key::Up => go(s, tx, extend, by(-1, 0)),
        Key::Down => go(s, tx, extend, by(1, 0)),
        Key::Left => go(s, tx, extend, by(0, -1)),
        Key::Right => go(s, tx, extend, by(0, 1)),
        Key::Home => go(s, tx, extend, move |_, from| {
            if jump_by {
                Cell { row: 0, col: 0 }
            } else {
                Cell { row: from.row, col: 0 }
            }
        }),
        Key::PageDown | Key::PageUp => {
            let down = ev.key == Key::PageDown;
            go(s, tx, extend, move |v, from| {
                let rows = i64::from(sheets::fit(v, room).0.saturating_sub(1).max(1));
                shifted(v, from, if down { rows } else { -rows }, 0)
            })
        }
        Key::Enter => go(s, tx, false, move |v, from| {
            shifted(v, from, if extend { -1 } else { 1 }, 0)
        }),
        Key::Tab => go(s, tx, false, move |v, from| {
            shifted(v, from, 0, if extend { -1 } else { 1 })
        }),
        Key::Function(2) => begin(s, tx, Begin::Existing { in_bar: false }),
        Key::Delete | Key::Backspace if !jump_by => clear(s),
        // Escape leaves a range for its first cell; with none it is the window's (Esc closes).
        Key::Escape => {
            let ranged = with_sheet(s, |v| {
                let ranged = v.selection.is_some();
                select(v, v.current, v.current);
                ranged
            });
            if ranged == Some(true) {
                settle(s);
                true
            } else {
                false
            }
        }
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// The pointer
// ---------------------------------------------------------------------------

/// One press, move or release on the grid: a press selects the cell (Shift: the range from the current cell; a second
/// press opens the editor on it), a drag extends the selection (to the nearest cell past the grid's edge), the release
/// tells the helper.
fn on_drag(s: &mut State, d: &mut Drag, tx: &mut Tx) {
    match d.phase {
        Phase::Down => {
            let Some(hit) = with(|g| g.geometry.hit(d.x, d.y, false)) else {
                return;
            };
            with(|g| {
                g.naming = false;
                g.name.focused = false;
            });
            // Pressing another cell ends an edit by keeping it (as Excel does); words the helper refused stay.
            if editing().is_some() {
                commit(s, tx, None);
                if editing().is_some() {
                    return;
                }
            }
            let extend = tx.mods().shift && d.clicks < 2;
            with_sheet(s, |v| {
                let cell = v.merge_origin(hit);
                if extend {
                    select(v, v.current, cell);
                } else {
                    select(v, cell, cell);
                }
            });
            with(|g| g.dragging = true);
            tx.focus(GRID_KEY, false);
            if d.clicks >= 2 {
                begin(s, tx, Begin::Existing { in_bar: false });
            }
        }
        Phase::Move => {
            if !with(|g| g.dragging) {
                return;
            }
            let Some(hit) = with(|g| g.geometry.hit(d.x, d.y, true)) else {
                return;
            };
            with_sheet(s, |v| {
                let cell = v.merge_origin(hit);
                select(v, v.current, cell);
            });
        }
        Phase::Up => {
            if !with(|g| std::mem::replace(&mut g.dragging, false)) {
                return;
            }
            if !d.cancelled() {
                settle(s);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// What the grid and the formula bar draw
// ---------------------------------------------------------------------------

/// The surface lying over the grid for the frame drawn with `geometry`, in a grid of `room` points of cells: the keys,
/// the pointer, and the cell editor over the current cell while it is open there.
pub fn layer(view: &SheetView, geometry: Geometry, room: (f32, f32), t: &Theme) -> rune::El {
    // Words typed on another sheet are not this one's: a tab was pressed, so they go.
    if editing().is_some_and(|e| e.sheet != view.sheet) {
        reset();
    }
    let cell_at = editing().filter(|e| !e.in_bar).and_then(|e| geometry.rect_of(e.cell));
    with(|g| {
        g.geometry = geometry;
        g.room = room;
        if !g.opened {
            g.opened = true;
            g.wants_focus = true;
        }
    });
    let mut layer = el()
        .key(GRID_KEY)
        .a11y_hidden()
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .focusable(true)
        .no_focus_ring()
        .on_key(|s: &mut State, ev: &KeyEvent, tx: &mut Tx| on_key(s, tx, ev))
        .on_drag(|s: &mut State, d: &mut Drag, tx: &mut Tx| on_drag(s, d, tx));
    if let Some([x, y, w, h]) = cell_at {
        let name = sheets::a1(view.current);
        layer = layer.child(
            with(|g| text_input(&g.edit, sheets::CELL))
                .key(EDITOR_KEY)
                .label(format!("Cell {name}"))
                .single_line()
                .color(t.text)
                .caret(t.accent_deep, t.selection)
                .on_edit(cell_edit)
                .on_input_key(editor_key)
                .absolute(x, y)
                .w(w.max(EDITOR_MIN_W))
                .h(h)
                .px(8.0)
                .fill(t.paper)
                .border(2.0, t.text)
                .border_layout(),
        );
    }
    layer
}

/// The formula bar's formula: the words of the current cell, which a press opens for editing; while the editor is open it
/// is the text field (from the bar) or shows what is typed (from the cell).
pub fn formula(view: &SheetView, t: &Theme) -> rune::El {
    use crate::ui::kit;
    match editing() {
        Some(e) if e.in_bar => with(|g| text_input(&g.edit, sheets::MONO))
            .key(BAR_KEY)
            .label("Formula bar")
            .single_line()
            .color(t.text)
            .caret(t.accent_deep, t.selection)
            .on_edit(cell_edit)
            .on_input_key(editor_key)
            .grow(1.0)
            .min_w(0.0),
        Some(_) => kit::label("sh-formula", typed(), sheets::MONO, t.text)
            .truncate()
            .min_w(0.0),
        None => rune::row()
            .key(BAR_OPEN_KEY)
            .role("button")
            .label("Edit formula")
            .grow(1.0)
            .min_w(0.0)
            .h_full()
            .center_y()
            .focusable(true)
            .on_press_tx(|s: &mut State, tx: &mut Tx| {
                begin(s, tx, Begin::Existing { in_bar: true });
            })
            .child(
                kit::label("sh-formula", sheets::formula_text(view), sheets::MONO, t.text)
                    .truncate()
                    .min_w(0.0),
            ),
    }
}

#[cfg(test)]
#[path = "sheet_tests.rs"]
mod tests;
