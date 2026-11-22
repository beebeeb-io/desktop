//! Tests of the sheet surface (`edit/sheet.rs`): the pointer and the keys move the selection, typing opens the cell editor,
//! Enter or Tab keeps it with one `SetCell`, Escape drops it, the formula bar edits the current cell, the status line
//! shows the figures and how the workbook is kept. The source here records the edits it is handed, as the helper's door
//! would. Named `office_sheets_*`: the done check filters on `sheets`.

use std::cell::RefCell;
use std::rc::Rc;

use beebeeb_office_protocol::office::{EditKind, EditOp};
use rune::{Env, Key, KeyEvent, Mods, PhysicalInput, Runtime};

use super::*;
use crate::frames::office::docs::docs_fixture;
use crate::frames::office::edit::EditUi;
use crate::frames::viewers::source::{OpenError, OpenFile, ViewerSource};
use crate::{init, view};

type Ops = Rc<RefCell<Vec<EditOp>>>;

/// A source that holds the open workbook "in a helper" and records every edit it is handed.
struct Source {
    file: OpenFile,
    ops: Ops,
    take: Rc<std::cell::Cell<bool>>,
    /// The file has an editing session (the window's poll starts the editor on one).
    can: Rc<std::cell::Cell<bool>>,
}

impl ViewerSource for Source {
    fn open(&mut self, _file_id: &str) -> Result<OpenFile, OpenError> {
        Ok(self.file.clone())
    }
    fn poll(&mut self, _file_id: &str) -> Option<Body> {
        None
    }
    fn cancel(&mut self, _file_id: &str) {}
    fn retry(&mut self, _file_id: &str) -> Result<OpenFile, OpenError> {
        Ok(self.file.clone())
    }
    fn office_live(&self) -> bool {
        true
    }
    fn can_edit(&self, _file_id: &str) -> bool {
        self.can.get()
    }
    fn edit(&mut self, _file_id: &str, op: EditOp) -> bool {
        if !self.take.get() {
            return false;
        }
        self.ops.borrow_mut().push(op);
        true
    }
}

fn cell(row: u32, col: u32, text: &str, value: Option<f64>, formula: Option<&str>) -> sheets::CellText {
    sheets::CellText {
        row,
        col,
        text: text.into(),
        right_aligned: value.is_some(),
        bold: false,
        value,
        formula: formula.map(String::from),
    }
}

/// 400 rows by 30 columns of 24 x 80 points; A1:A6 words, B1:B6 the numbers 10 to 60 (B3 a formula), nothing selected.
fn workbook() -> SheetView {
    let mut view = sheets::sheets_fixture::board();
    view.cells = (0..6)
        .flat_map(|r| {
            let n = f64::from(10 * (r + 1));
            let formula = (r == 2).then_some("=B1+B2");
            [
                cell(r, 0, &format!("Item {}", r + 1), None, None),
                cell(r, 1, &format!("{n}"), Some(n), formula),
            ]
        })
        .collect();
    view.col_widths = vec![80.0; 30];
    view.row_heights = vec![24.0; 400];
    view.extent = (400, 30);
    view.selection = None;
    view.current = Cell { row: 0, col: 0 };
    view.formula = String::new();
    view.top_left = Cell { row: 0, col: 0 };
    view.frozen = (0, 0);
    view.merges.clear();
    view
}

struct Rig {
    rt: Runtime<State>,
    ops: Ops,
    take: Rc<std::cell::Cell<bool>>,
    can: Rc<std::cell::Cell<bool>>,
}

fn rig_with(body: SheetView) -> Rig {
    let mut file = docs_fixture::file(false);
    file.version = 4;
    file.body = Body::Sheet(body);
    let ops: Ops = Rc::default();
    let take = Rc::new(std::cell::Cell::new(true));
    let can = Rc::new(std::cell::Cell::new(true));
    let mut state = init();
    let id = file.id.clone();
    let source = Source {
        file,
        ops: ops.clone(),
        take: take.clone(),
        can: can.clone(),
    };
    assert!(
        viewers::open_from(&mut state, Box::new(source), &id),
        "the source opened"
    );
    viewers::ui(&mut state).expect("a file").office.edit = Some(EditUi::new(EditKind::Xlsx));
    let env = Env {
        system_dark: false,
        scale: 1.0,
    };
    let mut rt = Runtime::new(state, view, (1278.0, 798.0), env);
    rt.on_key = Some(crate::shell::host::on_key);
    rt.poll = Some(crate::shell::host::poll);
    let mut rig = Rig { rt, ops, take, can };
    rig.settle();
    rig
}

fn rig() -> Rig {
    rig_with(workbook())
}

impl Rig {
    fn settle(&mut self) {
        for _ in 0..6 {
            self.rt.frame(1.0 / 60.0);
        }
    }

    fn editing(&mut self) -> Option<Editing> {
        super::editing(&self.rt.state)
    }

    fn typed(&mut self) -> String {
        super::typed(&self.rt.state)
    }

    fn sheet(&mut self) -> SheetView {
        match &viewers::ui(&mut self.rt.state).expect("a file").file.body {
            Body::Sheet(v) => v.clone(),
            other => panic!("{other:?} is not a sheet"),
        }
    }

    fn find(&self, key: &str) -> Option<rune::ElementInfo> {
        self.rt
            .elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
    }

    fn words(&self, key: &str) -> String {
        self.find(key)
            .unwrap_or_else(|| panic!("no element keyed {key}"))
            .text
            .unwrap_or_default()
    }

    /// The middle of a cell as drawn.
    fn middle(&self, row: u32, col: u32) -> (f32, f32) {
        let r = self
            .find(&format!("sh-cell-{row}-{col}"))
            .expect("the cell is drawn")
            .rect;
        (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0)
    }

    fn click(&mut self, row: u32, col: u32) {
        let (x, y) = self.middle(row, col);
        self.rt.pointer_down(x, y);
        self.rt.pointer_up(x, y);
        self.settle();
    }

    fn key(&mut self, key: Key) {
        self.key_with(key, Mods::default());
    }

    fn key_with(&mut self, key: Key, mods: Mods) {
        self.rt.key(KeyEvent { key, mods });
        self.settle();
    }

    fn text(&mut self, text: &str) {
        self.key(Key::Text(text.into()));
    }

    fn shift(&mut self, on: bool) {
        self.rt.physical_input(PhysicalInput::Modifiers(Mods {
            shift: on,
            ..Mods::default()
        }));
    }

    fn ops(&self) -> Vec<EditOp> {
        self.ops.borrow().clone()
    }

    /// The `SetCell`s handed over, as (row, col, text).
    fn sets(&self) -> Vec<(u32, u32, String)> {
        self.ops()
            .into_iter()
            .filter_map(|op| match op {
                EditOp::SetCell { row, col, text, .. } => Some((row, col, text)),
                _ => None,
            })
            .collect()
    }

    fn selections(&self) -> Vec<CellRange> {
        self.ops()
            .into_iter()
            .filter_map(|op| match op {
                EditOp::SelectCells { range, .. } => Some(range),
                _ => None,
            })
            .collect()
    }
}

fn ctrl() -> Mods {
    Mods {
        ctrl: true,
        cmd: true,
        ..Mods::default()
    }
}

fn shifted_keys() -> Mods {
    Mods {
        shift: true,
        ..Mods::default()
    }
}

// ---------------------------------------------------------------------------
// The pointer
// ---------------------------------------------------------------------------

#[test]
fn office_sheets_a_press_selects_the_cell_under_it() {
    let mut rig = rig();
    rig.click(2, 1);
    let view = rig.sheet();
    assert_eq!(view.current, Cell { row: 2, col: 1 });
    assert_eq!(
        view.selection, None,
        "one cell is no selection: the status shows no figures"
    );
    assert_eq!(rig.words(NAME_KEY), "B3");
    // The helper is told once the selection settled.
    assert_eq!(
        rig.selections(),
        [CellRange {
            row: 2,
            col: 1,
            rows: 1,
            cols: 1
        }]
    );
    assert!(rig.sets().is_empty(), "a press changes nothing");
}

#[test]
fn office_sheets_a_drag_selects_a_range_and_tells_the_helper_once() {
    let mut rig = rig();
    let (x0, y0) = rig.middle(0, 1);
    let (x1, y1) = rig.middle(3, 1);
    rig.rt.pointer_down(x0, y0);
    rig.rt.pointer_move(x1, (y0 + y1) / 2.0);
    rig.rt.pointer_move(x1, y1);
    rig.rt.pointer_up(x1, y1);
    rig.settle();
    let view = rig.sheet();
    assert_eq!(view.current, Cell { row: 0, col: 1 });
    assert_eq!(view.selection, Some((Cell { row: 0, col: 1 }, Cell { row: 3, col: 1 })));
    assert_eq!(
        rig.selections(),
        [CellRange {
            row: 0,
            col: 1,
            rows: 4,
            cols: 1
        }],
        "once, at the release"
    );
    // The status line shows the figures of the selection: 10 + 20 + 30 + 40.
    assert_eq!(rig.words("st-2"), "Sum 100");
    assert_eq!(rig.words("st-4"), "Average 25");
    assert_eq!(rig.words("st-6"), "Count 4");
}

#[test]
fn office_sheets_shift_press_extends_from_the_current_cell() {
    let mut rig = rig();
    rig.click(1, 0);
    rig.shift(true);
    rig.click(4, 1);
    rig.shift(false);
    let view = rig.sheet();
    assert_eq!(view.current, Cell { row: 1, col: 0 });
    assert_eq!(view.selection, Some((Cell { row: 1, col: 0 }, Cell { row: 4, col: 1 })));
}

#[test]
fn office_sheets_no_selection_no_figures_and_the_status_says_how_it_is_kept() {
    let rig = rig();
    assert!(
        rig.rt
            .elements()
            .iter()
            .all(|e| !e.text.as_deref().is_some_and(|w| w.starts_with("Sum "))),
        "nothing selected: no Sum"
    );
    assert_eq!(rig.words("st-0"), "Sheet 1 of 3");
    assert_eq!(rig.words("st-3"), "saved as version 4");
    assert_eq!(rig.words("st-1"), "Encrypted");
    assert_eq!(rig.words("st-5"), "About");
    assert!(rig.find("edit-chrome").is_some(), "the editor's frame is drawn");
}

// ---------------------------------------------------------------------------
// The keys
// ---------------------------------------------------------------------------

#[test]
fn office_sheets_arrows_tab_enter_and_home_move_the_current_cell() {
    let mut rig = rig();
    rig.click(1, 1);
    rig.key(Key::Down);
    assert_eq!(rig.sheet().current, Cell { row: 2, col: 1 });
    rig.key(Key::Right);
    assert_eq!(rig.sheet().current, Cell { row: 2, col: 2 });
    rig.key(Key::Up);
    rig.key(Key::Left);
    assert_eq!(rig.sheet().current, Cell { row: 1, col: 1 });
    rig.key(Key::Tab);
    assert_eq!(rig.sheet().current, Cell { row: 1, col: 2 });
    rig.key_with(Key::Tab, shifted_keys());
    assert_eq!(rig.sheet().current, Cell { row: 1, col: 1 });
    rig.key(Key::Enter);
    assert_eq!(rig.sheet().current, Cell { row: 2, col: 1 });
    rig.key_with(Key::Enter, shifted_keys());
    assert_eq!(rig.sheet().current, Cell { row: 1, col: 1 });
    rig.key(Key::Home);
    assert_eq!(rig.sheet().current, Cell { row: 1, col: 0 });
    rig.key_with(Key::Home, ctrl());
    assert_eq!(rig.sheet().current, Cell { row: 0, col: 0 });
    // Never off the sheet.
    rig.key(Key::Up);
    rig.key(Key::Left);
    assert_eq!(rig.sheet().current, Cell { row: 0, col: 0 });
    assert!(rig.sets().is_empty(), "moving changes nothing");
}

#[test]
fn office_sheets_shift_arrows_extend_and_escape_leaves_the_range() {
    let mut rig = rig();
    rig.click(1, 1);
    rig.key_with(Key::Down, shifted_keys());
    rig.key_with(Key::Down, shifted_keys());
    rig.key_with(Key::Right, shifted_keys());
    let view = rig.sheet();
    assert_eq!(
        view.current,
        Cell { row: 1, col: 1 },
        "the current cell stays the anchor"
    );
    assert_eq!(view.selection, Some((Cell { row: 1, col: 1 }, Cell { row: 3, col: 2 })));
    rig.key_with(Key::Up, shifted_keys());
    assert_eq!(
        rig.sheet().selection,
        Some((Cell { row: 1, col: 1 }, Cell { row: 2, col: 2 }))
    );
    rig.key(Key::Escape);
    let view = rig.sheet();
    assert_eq!((view.current, view.selection), (Cell { row: 1, col: 1 }, None));
}

#[test]
fn office_sheets_ctrl_arrows_jump_over_runs_of_cells() {
    let view = workbook();
    let b = |row| Cell { row, col: 1 };
    // From inside the run B1:B6 to its end, then on to the edge of the sheet.
    assert_eq!(jump(&view, b(0), 1, 0), b(5));
    assert_eq!(jump(&view, b(5), 1, 0), b(399));
    // From past the run back to its end, then to the top.
    assert_eq!(jump(&view, b(20), -1, 0), b(5));
    assert_eq!(jump(&view, b(5), -1, 0), b(0));
    // Across: A1 and B1 are filled, C1 is not.
    assert_eq!(jump(&view, Cell { row: 0, col: 0 }, 0, 1), Cell { row: 0, col: 1 });
    assert_eq!(jump(&view, Cell { row: 0, col: 1 }, 0, 1), Cell { row: 0, col: 29 });
    // Nothing held in the row: the edge.
    assert_eq!(jump(&view, Cell { row: 50, col: 3 }, 0, -1), Cell { row: 50, col: 0 });
    assert_eq!(jump(&view, Cell { row: 399, col: 1 }, 1, 0), Cell { row: 399, col: 1 });
}

#[test]
fn office_sheets_moving_past_the_view_scrolls_it_to_the_cell() {
    let mut rig = rig();
    rig.click(0, 0);
    for _ in 0..60 {
        rig.rt.key(KeyEvent {
            key: Key::Down,
            mods: Mods::default(),
        });
    }
    rig.settle();
    let view = rig.sheet();
    assert_eq!(view.current.row, 60);
    assert!(view.top_left.row > 0, "the view followed the cell");
    assert!(
        rig.find(&format!("sh-cell-{}-0", view.current.row)).is_some(),
        "the current cell is drawn"
    );
    // Back to the top: the view goes back too.
    rig.key_with(Key::Home, ctrl());
    assert_eq!(rig.sheet().top_left, Cell { row: 0, col: 0 });
}

#[test]
fn office_sheets_the_wheel_still_scrolls_and_leaves_the_selection() {
    let mut rig = rig();
    rig.click(1, 1);
    let grid = rig.find("sh-grid").expect("the grid").rect;
    rig.rt.wheel(grid[0] + grid[2] / 2.0, grid[1] + grid[3] / 2.0, -100.0);
    // The rows glide into place (a moving cell is where the glide has it, not where it will stand).
    for _ in 0..40 {
        rig.settle();
    }
    let view = rig.sheet();
    assert_eq!(view.top_left.row, 5, "100 points down: 5 rows");
    assert_eq!(
        view.current,
        Cell { row: 1, col: 1 },
        "a scroll never changes the selection"
    );
    // A press after the scroll lands on the cell drawn there.
    rig.click(8, 2);
    assert_eq!(rig.sheet().current, Cell { row: 8, col: 2 });
}

#[test]
fn office_sheets_first_line_in_view_follows_a_cell() {
    let sizes = vec![24.0; 100];
    // Already in view: unchanged. Above: the line. Below: the line at the far edge. Frozen: unchanged.
    assert_eq!(first_for(&sizes, 15.0, 0, 5, 8, 240.0), 5);
    assert_eq!(first_for(&sizes, 15.0, 0, 5, 2, 240.0), 2);
    assert_eq!(
        first_for(&sizes, 15.0, 0, 0, 20, 240.0),
        11,
        "ten rows fit in 240 points: 11..=20"
    );
    assert_eq!(first_for(&sizes, 15.0, 2, 7, 1, 240.0), 7);
    assert_eq!(
        first_for(&sizes, 15.0, 2, 5, 30, 240.0),
        23,
        "two frozen rows take room: eight rows fit"
    );
}

// ---------------------------------------------------------------------------
// The cell editor
// ---------------------------------------------------------------------------

#[test]
fn office_sheets_typing_opens_the_editor_and_enter_keeps_it_with_one_setcell() {
    let mut rig = rig();
    rig.click(7, 1);
    rig.text("4");
    assert!(rig.find(EDITOR_KEY).is_some(), "the editor is over the cell");
    assert_eq!(rig.editing().map(|e| e.cell), Some(Cell { row: 7, col: 1 }));
    rig.text("2");
    assert_eq!(rig.typed(), "42", "the field took the keys");
    assert!(rig.sets().is_empty(), "nothing is sent while typing");
    assert_eq!(rig.words("sh-formula"), "42", "the formula bar shows what is typed");
    rig.key(Key::Enter);
    assert_eq!(rig.sets(), [(7, 1, "42".to_string())], "exactly one SetCell");
    assert!(rig.find(EDITOR_KEY).is_none(), "the editor is gone");
    assert_eq!(rig.sheet().current, Cell { row: 8, col: 1 }, "Enter moves down");
    assert_eq!(rig.typed(), "", "the words are dropped");
}

#[test]
fn office_sheets_tab_keeps_and_moves_right_shift_tab_left() {
    let mut rig = rig();
    rig.click(7, 2);
    rig.text("x");
    rig.key(Key::Tab);
    assert_eq!(rig.sets(), [(7, 2, "x".to_string())]);
    assert_eq!(rig.sheet().current, Cell { row: 7, col: 3 });
    rig.text("y");
    rig.key_with(Key::Tab, shifted_keys());
    assert_eq!(rig.sets().len(), 2);
    assert_eq!(rig.sheet().current, Cell { row: 7, col: 2 });
}

#[test]
fn office_sheets_escape_drops_the_words_and_sends_nothing() {
    let mut rig = rig();
    rig.click(7, 1);
    rig.text("9");
    rig.text("9");
    assert_eq!(rig.typed(), "99");
    rig.key(Key::Escape);
    assert!(rig.find(EDITOR_KEY).is_none());
    assert!(rig.sets().is_empty(), "nothing was changed");
    assert_eq!(rig.editing(), None);
    assert_eq!(rig.typed(), "", "the words are gone");
    assert_eq!(
        rig.sheet().current,
        Cell { row: 7, col: 1 },
        "the cell stays where it was"
    );
}

#[test]
fn office_sheets_f2_and_a_double_press_edit_the_cells_own_words() {
    let mut rig = rig();
    rig.click(2, 1);
    rig.key(Key::Function(2));
    assert_eq!(rig.typed(), "=B1+B2", "F2 opens on the formula");
    rig.key(Key::Escape);
    assert!(rig.sets().is_empty());
    // A double press: on a cell with words only.
    let (x, y) = rig.middle(1, 0);
    rig.rt.pointer_down(x, y);
    rig.rt.pointer_up(x, y);
    rig.rt.pointer_down(x, y);
    rig.rt.pointer_up(x, y);
    rig.settle();
    assert_eq!(rig.editing().map(|e| e.cell), Some(Cell { row: 1, col: 0 }));
    assert_eq!(rig.typed(), "Item 2");
    // Kept as it is, it is still one SetCell (Excel does the same).
    rig.key(Key::Enter);
    assert_eq!(rig.sets(), [(1, 0, "Item 2".to_string())]);
}

#[test]
fn office_sheets_pressing_another_cell_keeps_the_edit() {
    let mut rig = rig();
    rig.click(7, 1);
    rig.text("5");
    rig.click(9, 3);
    assert_eq!(rig.sets(), [(7, 1, "5".to_string())]);
    assert!(rig.find(EDITOR_KEY).is_none());
    assert_eq!(rig.sheet().current, Cell { row: 9, col: 3 });
}

#[test]
fn office_sheets_a_change_the_helper_does_not_take_stays_in_the_editor() {
    let mut rig = rig();
    rig.click(7, 1);
    rig.text("7");
    rig.take.set(false);
    rig.key(Key::Enter);
    assert!(rig.sets().is_empty());
    assert_eq!(rig.editing().map(|e| e.cell), Some(Cell { row: 7, col: 1 }), "still open");
    assert_eq!(rig.typed(), "7", "nothing was lost");
    let toast = viewers::ui(&mut rig.rt.state).and_then(|ui| ui.toast.clone());
    assert_eq!(toast.as_deref(), Some(NOT_TAKEN));
    rig.take.set(true);
    rig.key(Key::Enter);
    assert_eq!(rig.sets(), [(7, 1, "7".to_string())]);
}

#[test]
fn office_sheets_words_over_the_op_limit_are_refused_not_sent() {
    let mut rig = rig();
    rig.click(7, 1);
    rig.text("a");
    let long = "x".repeat(MAX_OP_TEXT_BYTES + 1);
    with(&rig.rt.state, |g| g.edit.replace_text(long));
    rig.key(Key::Enter);
    assert!(rig.sets().is_empty());
    assert!(rig.editing().is_some(), "kept open, nothing lost");
    let toast = viewers::ui(&mut rig.rt.state).and_then(|ui| ui.toast.clone());
    assert_eq!(toast.as_deref(), Some(TOO_LONG));
    rig.key(Key::Escape);
}

#[test]
fn office_sheets_delete_clears_the_cells_of_the_selection() {
    let mut rig = rig();
    rig.click(0, 1);
    rig.key_with(Key::Down, shifted_keys());
    rig.key(Key::Delete);
    assert_eq!(rig.sets(), [(0, 1, String::new()), (1, 1, String::new())]);
    // An empty cell asks for nothing.
    rig.click(30, 4);
    rig.key(Key::Delete);
    assert_eq!(rig.sets().len(), 2);
}

#[test]
fn office_sheets_a_shortcut_does_not_start_an_edit() {
    let mut rig = rig();
    rig.click(7, 1);
    rig.key_with(Key::Command('c'), ctrl());
    rig.key_with(Key::Text("a".into()), ctrl());
    assert_eq!(rig.editing(), None);
    assert!(rig.find(EDITOR_KEY).is_none());
}

// ---------------------------------------------------------------------------
// The formula bar
// ---------------------------------------------------------------------------

#[test]
fn office_sheets_the_formula_bar_shows_and_edits_the_current_cell() {
    let mut rig = rig();
    rig.click(2, 1);
    assert_eq!(rig.words("sh-formula"), "=B1+B2", "the formula, not the value");
    assert_eq!(rig.words(NAME_KEY), "B3");
    let open = rig.find(BAR_OPEN_KEY).expect("the formula is pressable").rect;
    rig.rt.pointer_down(open[0] + 6.0, open[1] + open[3] / 2.0);
    rig.rt.pointer_up(open[0] + 6.0, open[1] + open[3] / 2.0);
    rig.settle();
    assert_eq!(rig.editing().map(|e| e.in_bar), Some(true));
    assert!(rig.find(BAR_KEY).is_some(), "the bar holds the text field");
    assert!(rig.find(EDITOR_KEY).is_none(), "and the cell does not");
    assert_eq!(rig.typed(), "=B1+B2");
    rig.text("1");
    rig.key(Key::Enter);
    assert_eq!(rig.sets(), [(2, 1, "=B1+B21".to_string())]);
    assert_eq!(rig.sheet().current, Cell { row: 3, col: 1 });
}

#[test]
fn office_sheets_the_formula_bar_escape_cancels() {
    let mut rig = rig();
    rig.click(2, 1);
    let open = rig.find(BAR_OPEN_KEY).expect("pressable").rect;
    rig.rt.pointer_down(open[0] + 6.0, open[1] + open[3] / 2.0);
    rig.rt.pointer_up(open[0] + 6.0, open[1] + open[3] / 2.0);
    rig.settle();
    rig.text("z");
    rig.key(Key::Escape);
    assert!(rig.sets().is_empty());
    assert_eq!(rig.editing(), None);
    assert_eq!(rig.words("sh-formula"), "=B1+B2");
}

// ---------------------------------------------------------------------------
// Words kept, the viewer unchanged, the budget
// ---------------------------------------------------------------------------

#[test]
fn office_sheets_a_tab_press_drops_words_typed_on_the_other_sheet() {
    let mut rig = rig();
    rig.click(7, 1);
    rig.text("s");
    assert_eq!(rig.typed(), "s");
    rig.rt.update(|s, _| {
        if let Some(Body::Sheet(v)) = viewers::ui(s).map(|ui| &mut ui.file.body) {
            v.sheet = 1;
        }
    });
    rig.settle();
    assert_eq!(rig.editing(), None);
    assert_eq!(rig.typed(), "");
    assert!(rig.sets().is_empty());
}

#[test]
fn office_sheets_unsaved_changes_are_said_in_the_status_line() {
    let mut rig = rig();
    rig.rt.update(|s, _| {
        if let Some(edit) = viewers::ui(s).and_then(|ui| ui.office.edit.as_mut()) {
            edit.revision = 3;
            edit.saved_revision = 2;
        }
    });
    rig.settle();
    assert_eq!(rig.words("st-3"), "unsaved changes");
}

#[test]
fn office_sheets_the_viewer_without_the_editor_has_no_surface() {
    let mut rig = rig();
    // No editing session (a file the account may not change): the window's poll starts no editor.
    rig.can.set(false);
    rig.rt.update(|s, _| {
        if let Some(ui) = viewers::ui(s) {
            ui.office.edit = None;
        }
    });
    rig.settle();
    assert!(rig.find(GRID_KEY).is_none(), "no layer over a viewed sheet");
    assert!(rig.find(BAR_OPEN_KEY).is_none());
    assert_eq!(rig.words("st-1"), "Encrypted");
}

#[test]
fn office_sheets_editing_keeps_the_frame_budget_at_the_cell_cap() {
    use std::time::{Duration, Instant};
    let cells = 65_536u32;
    let cols = 256u32;
    let mut view = workbook();
    view.cells = (0..cells)
        .map(|i| cell(i / cols, i % cols, &format!("{}", 1000 + i), Some(f64::from(i)), None))
        .collect();
    view.col_widths = vec![80.0; cols as usize];
    view.row_heights = vec![24.0; (cells / cols) as usize];
    view.extent = (cells / cols, cols);
    view.selection = Some((
        Cell { row: 0, col: 0 },
        Cell {
            row: cells / cols - 1,
            col: cols - 1,
        },
    ));
    let mut rig = rig_with(view);
    rig.click(3, 3);
    let best = (0..5)
        .map(|_| {
            let t = Instant::now();
            rig.rt.update(|_, _| {});
            rig.rt.frame(1.0 / 60.0);
            t.elapsed()
        })
        .min()
        .expect("a run");
    eprintln!("sheet editing frame at the cell cap: {best:?}");
    assert!(
        best < Duration::from_millis(100),
        "a frame with the editor on took {best:?}"
    );
}

#[test]
fn office_sheets_geometry_hits_the_cell_it_drew() {
    let mut rig = rig();
    rig.settle();
    let grid = rig.find("sh-grid").expect("the grid").rect;
    let geometry = with(&rig.rt.state, |g| g.geometry.clone());
    for (r, c) in [(0, 0), (3, 2), (10, 5)] {
        let (x, y) = rig.middle(r, c);
        assert_eq!(
            geometry.hit(x - grid[0], y - grid[1], false),
            Some(Cell { row: r, col: c }),
            "{r},{c}"
        );
    }
    // The headers hit nothing; a drag past the edge takes the nearest cell.
    assert_eq!(geometry.hit(5.0, 5.0, false), None);
    assert_eq!(geometry.hit(-50.0, -50.0, true), Some(Cell { row: 0, col: 0 }));
}

// ---------------------------------------------------------------------------
// Wired into the window (w05): the cell box, Alt+=, the first focus and the tabs' rename, all through the real frame
// ---------------------------------------------------------------------------

fn press_key(rig: &mut Rig, key: &str) {
    let r = rig.find(key).unwrap_or_else(|| panic!("no element keyed {key}")).rect;
    let (x, y) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
    rig.rt.pointer_down(x, y);
    rig.rt.pointer_up(x, y);
    rig.settle();
}

#[test]
fn office_sheets_the_cell_box_in_the_window_goes_to_what_is_typed() {
    let mut rig = rig();
    press_key(&mut rig, NAME_KEY);
    rig.text("B5");
    rig.key(Key::Enter);
    let v = rig.sheet();
    assert_eq!(v.current, Cell { row: 4, col: 1 });
    let toast = viewers::ui(&mut rig.rt.state).and_then(|ui| ui.toast.clone());
    assert_eq!(toast.as_deref(), Some("Went to B5"));
    assert!(matches!(rig.ops.borrow().last(), Some(EditOp::SelectCells { .. })));
    // A wrong reference says why and leaves the grid where it was.
    press_key(&mut rig, NAME_KEY);
    rig.text("Q#9");
    rig.key(Key::Enter);
    assert_eq!(rig.sheet().current, Cell { row: 4, col: 1 });
    let toast = viewers::ui(&mut rig.rt.state).and_then(|ui| ui.toast.clone());
    assert_eq!(toast.as_deref(), Some("Not a valid cell or range"));
}

#[test]
fn office_sheets_alt_equals_in_the_grid_sums_the_selection() {
    let mut rig = rig();
    rig.click(0, 1);
    rig.ops.borrow_mut().clear();
    let alt = Mods {
        alt: true,
        ..Mods::default()
    };
    rig.key_with(Key::Text("=".into()), alt);
    assert_eq!(rig.ops.borrow().last(), Some(&EditOp::AutoSum));
    assert!(rig.editing().is_none(), "Alt+= does not open the cell editor");
}

#[test]
fn office_sheets_the_grid_asks_for_the_keyboard_once_when_opened() {
    let mut rig = rig();
    // The window's poll hands the ask to Rune (`take_focus`, next to the viewer's own one-shot focus): the grid has
    // the keyboard, and nothing asks again.
    assert!(!with(&rig.rt.state, |g| g.wants_focus), "the ask was taken by the window's poll");
    let mut focused = None;
    rig.rt.update(|_, tx| focused = tx.focused().map(str::to_string));
    assert_eq!(focused.as_deref(), Some(GRID_KEY), "the grid has the keyboard when the file opens");
}

#[test]
fn office_sheets_the_window_renames_a_tab_on_a_double_press() {
    let mut rig = rig();
    let r = rig.find("sh-tab-0").expect("the first tab").rect;
    let (x, y) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
    for _ in 0..2 {
        rig.rt.pointer_down(x, y);
        rig.rt.pointer_up(x, y);
    }
    rig.settle();
    assert!(rig.find("edit-sheet-tab-name").is_some(), "the rename field is in the window's tabs");
}

// ---------------------------------------------------------------------------
// The command search (dc-175, as RC-c for a deck): Ctrl+K and the ribbon's search draw it over the workbook, its
// field has the keyboard, typed words land in it and not in a cell, Esc closes it and not the file.
// ---------------------------------------------------------------------------

impl Rig {
    fn focused(&self) -> Option<String> {
        self.rt
            .elements()
            .into_iter()
            .find(|e| e.focused && !e.exiting)
            .and_then(|e| e.key)
    }

    fn palette_query(&mut self) -> Option<String> {
        use crate::frames::viewers::office_over::Over;
        match viewers::ui(&mut self.rt.state).and_then(|ui| ui.office.over.as_ref()) {
            Some(Over::Palette(palette)) => Some(palette.query.text.clone()),
            _ => None,
        }
    }

    fn press_key(&mut self, key: &str) {
        let r = self.find(key).unwrap_or_else(|| panic!("no element keyed {key}")).rect;
        let (x, y) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
        self.rt.pointer_down(x, y);
        self.rt.pointer_up(x, y);
        self.settle();
    }

    fn assert_palette_works(&mut self) {
        assert!(
            self.find("pal").is_some(),
            "the command search is drawn over the workbook"
        );
        assert_eq!(
            self.focused().as_deref(),
            Some("pal-input"),
            "and its field has the keyboard"
        );
        for c in "Bold".chars() {
            self.text(&c.to_string());
        }
        assert_eq!(
            self.palette_query().as_deref(),
            Some("Bold"),
            "the typed words land in the search"
        );
        assert!(
            self.editing().is_none() && self.sets().is_empty(),
            "no cell was edited: {:?}",
            self.ops()
        );
        self.key(Key::Escape);
        assert!(
            self.find("pal").is_none() && self.palette_query().is_none(),
            "Esc closed the search"
        );
        assert!(viewers::ui(&mut self.rt.state).is_some(), "not the file");
    }
}

#[test]
fn office_sheets_palette_ctrl_k_after_a_cell_click_shows_the_command_search() {
    let mut rig = rig();
    rig.click(1, 1);
    rig.key_with(Key::Command('k'), ctrl());
    rig.assert_palette_works();
}

#[test]
fn office_sheets_palette_ctrl_k_with_the_keyboard_nowhere_shows_the_command_search() {
    let mut rig = rig();
    rig.key_with(Key::Command('k'), ctrl());
    rig.assert_palette_works();
}

#[test]
fn office_sheets_palette_the_search_button_shows_the_command_search() {
    let mut rig = rig();
    rig.click(2, 0);
    rig.press_key("rib-find");
    rig.assert_palette_works();
}

/// RC2-11: the command search closed over a sheet hands the keyboard back to the grid (the current cell), never to
/// the ribbon's search: typing then goes into the cell.
#[test]
fn office_focus_palette_close_returns_to_the_sheets_grid() {
    use crate::frames::viewers::office_over::Over;
    let mut rig = rig();
    rig.click(2, 1);
    assert_eq!(rig.focused().as_deref(), Some(GRID_KEY));
    let cmd = Mods {
        cmd: true,
        ..Mods::default()
    };
    rig.key_with(Key::Command('k'), cmd);
    let open = matches!(
        viewers::ui(&mut rig.rt.state).expect("a file").office.over,
        Some(Over::Palette(_))
    );
    assert!(open, "Ctrl+K from the grid opened the command search");
    assert_ne!(rig.focused().as_deref(), Some(GRID_KEY), "the search took the keyboard");
    rig.key(Key::Escape);
    assert!(
        viewers::ui(&mut rig.rt.state).expect("a file").office.over.is_none(),
        "Esc closed it"
    );
    assert_eq!(
        rig.focused().as_deref(),
        Some(GRID_KEY),
        "the grid has the keyboard again, not the ribbon's search"
    );
    rig.text("7");
    assert!(rig.editing().is_some(), "typing goes into the current cell");
    assert_eq!(rig.typed(), "7");
}

/// RC2-11: after each arrow, what a screen reader is told is focused is the current cell, named by its address and
/// value (the grid layer keeps the keyboard and points at the cell as its active descendant).
#[test]
fn office_focus_an_arrow_makes_the_cell_the_focused_element() {
    let mut rig = rig();
    rig.click(1, 1);
    for key in [Key::Down, Key::Right, Key::Down, Key::Left, Key::Up] {
        rig.key(key.clone());
        let current = rig.sheet().current;
        assert_eq!(
            rig.focused().as_deref(),
            Some(GRID_KEY),
            "{key:?}: the grid keeps the keyboard"
        );
        let tree = rig.rt.accessibility_tree();
        let node = |id| tree.nodes.iter().find(|(n, _)| *n == id).map(|(_, node)| node);
        let layer = node(tree.focus).expect("the focused node is in the tree");
        assert_eq!(
            layer.author_id(),
            Some(GRID_KEY),
            "{key:?}: the layer is the focused node"
        );
        let cell = layer
            .active_descendant()
            .and_then(node)
            .unwrap_or_else(|| panic!("{key:?}: the layer points at no cell"));
        let want = format!("sh-cell-{}-{}", current.row, current.col);
        assert_eq!(
            cell.author_id(),
            Some(want.as_str()),
            "{key:?}: the focused element is the current cell"
        );
        let name = cell.label().unwrap_or_default().to_string();
        assert!(
            name.starts_with(&format!("{}, ", sheets::a1(current))),
            "{key:?}: the cell is named by its address and value: {name:?}"
        );
    }
}
