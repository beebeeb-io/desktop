//! Tests of the sheet ribbon (`edit/sheet_ribbon.rs`) and the sheet tabs' edits (`sheets_tabs.rs`): each tool is one
//! `EditOp`, the lit tools follow `Marks`, the cell box goes to a cell or range with the board's words, a double press
//! renames a sheet with one `RenameSheet`, and the "+" is dimmed with its words. The source records the edits it is
//! handed. Named `office_sheets_*`: the done check filters on `sheet`.

use std::cell::RefCell;
use std::rc::Rc;

use beebeeb_office_protocol::office::{EditKind, EditOp, ListKind, Mark, Marks, NumberFormat, ParaStyle, Rgb, TextAlign};
use rune::{Env, Key, KeyEvent, Runtime};

use super::*;
use super as sheets_tabs;
use crate::frames::office::docs::docs_fixture;
use crate::frames::office::edit::EditUi;
use crate::frames::office::edit::sheet_ribbon::{
    NOT_VALID, data_tools, go_to, is_auto_sum, parse_reference, ribbon, scene, set_open,
};
use crate::frames::office::sheets;
use crate::frames::viewers::source::{OpenError, OpenFile, ViewerSource};
use crate::theme::tokens;
use crate::{init, view};

type Ops = Rc<RefCell<Vec<EditOp>>>;

struct Source {
    file: OpenFile,
    ops: Ops,
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
        true
    }
    fn edit(&mut self, _file_id: &str, op: EditOp) -> bool {
        self.ops.borrow_mut().push(op);
        true
    }
}

fn workbook() -> SheetView {
    let mut view = sheets::sheets_fixture::board();
    view.cells.clear();
    view.col_widths = vec![80.0; 30];
    view.row_heights = vec![24.0; 400];
    view.extent = (400, 30);
    view.selection = None;
    view.current = Cell { row: 0, col: 0 };
    view.top_left = Cell { row: 0, col: 0 };
    view.frozen = (0, 0);
    view.merges.clear();
    view
}

fn open_state(marks: Option<Marks>) -> (State, Ops) {
    let mut file = docs_fixture::file(false);
    file.version = 4;
    file.body = Body::Sheet(workbook());
    let ops: Ops = Rc::default();
    let mut state = init();
    let id = file.id.clone();
    let source = Source {
        file,
        ops: ops.clone(),
    };
    assert!(viewers::open_from(&mut state, Box::new(source), &id));
    let mut edit = EditUi::new(EditKind::Xlsx);
    edit.marks = marks;
    edit.can_undo = true;
    viewers::ui(&mut state).expect("a file").office.edit = Some(edit);
    (state, ops)
}

fn env() -> Env {
    Env {
        system_dark: false,
        scale: 1.0,
    }
}

fn find(rt: &Runtime<State>, key: &str) -> Option<rune::ElementInfo> {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
}

fn press(rt: &mut Runtime<State>, key: &str) {
    let r = find(rt, key).unwrap_or_else(|| panic!("no element keyed {key}")).rect;
    let (x, y) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    settle(rt);
}

fn settle(rt: &mut Runtime<State>) {
    for _ in 0..6 {
        rt.frame(1.0 / 60.0);
    }
}

/// The ribbon of the open workbook on its own.
fn ribbon_view(s: &State, cx: &mut rune::Cx<State>) -> rune::El {
    let t = tokens::theme(cx.system_dark());
    let ui = s.viewer.ui.as_ref().expect("a file");
    rune::col().w_full().h_full().child(ribbon(ui, false, &t))
}

fn ribbon_rt(marks: Option<Marks>) -> (Runtime<State>, Ops) {
    set_open(None);
    let (state, ops) = open_state(marks);
    let mut rt = Runtime::new(state, ribbon_view, (1278.0, 400.0), env());
    settle(&mut rt);
    (rt, ops)
}

/// The key of the tool with this name.
fn tool_key(rt: &Runtime<State>, name: &str) -> String {
    rt.elements()
        .into_iter()
        .find(|e| !e.exiting && e.label.as_deref() == Some(name))
        .and_then(|e| e.key)
        .unwrap_or_else(|| panic!("no tool named {name}"))
}

fn marks(bold: bool, align: TextAlign) -> Marks {
    Marks {
        bold,
        italic: false,
        underline: false,
        strike: false,
        style: ParaStyle::Normal,
        align,
        list: ListKind::None,
    }
}

#[test]
fn office_sheets_ribbon_tools_are_one_edit_each() {
    let (mut rt, ops) = ribbon_rt(None);
    for (name, op) in [
        ("Undo", EditOp::Undo),
        ("Bold", EditOp::Mark { mark: Mark::Bold, on: true }),
        ("Italic", EditOp::Mark { mark: Mark::Italic, on: true }),
        ("Underline", EditOp::Mark { mark: Mark::Underline, on: true }),
        ("Align centre", EditOp::Align(TextAlign::Center)),
        ("Currency", EditOp::NumberFormat(NumberFormat::Currency)),
        ("More decimals", EditOp::NumberFormat(NumberFormat::MoreDecimals)),
        ("Percent", EditOp::NumberFormat(NumberFormat::Percent)),
        ("Sum", EditOp::AutoSum),
        ("Sort", EditOp::Sort { ascending: true }),
        ("AutoFilter", EditOp::Filter),
    ] {
        let key = tool_key(&rt, name);
        press(&mut rt, &key);
        assert_eq!(ops.borrow().last(), Some(&op), "{name}");
    }
    assert_eq!(ops.borrow().len(), 11, "one edit per press");
}

#[test]
fn office_sheets_ribbon_lit_tools_follow_the_marks() {
    let (mut rt, ops) = ribbon_rt(Some(marks(true, TextAlign::Right)));
    let lit = tokens::theme(false).selection;
    let fill = |rt: &Runtime<State>, name: &str| find(rt, &tool_key(rt, name)).expect(name).fill;
    assert_eq!(fill(&rt, "Bold"), lit, "Bold is lit");
    assert_eq!(fill(&rt, "Align right"), lit, "Align right is lit");
    assert_ne!(fill(&rt, "Align left"), lit);
    assert_ne!(fill(&rt, "Italic"), lit);
    // A lit tool turns its mark off.
    let key = tool_key(&rt, "Bold");
    press(&mut rt, &key);
    assert_eq!(ops.borrow().last(), Some(&EditOp::Mark { mark: Mark::Bold, on: false }));
}

#[test]
fn office_sheets_ribbon_fill_pop_up_sends_the_colour() {
    let (mut rt, ops) = ribbon_rt(None);
    let key = tool_key(&rt, "Fill colour");
    press(&mut rt, &key);
    assert!(find(&rt, "rib-menu").is_some(), "the pop-up opens");
    press(&mut rt, "rib-menu-item-1");
    assert_eq!(
        ops.borrow().last(),
        Some(&EditOp::FillColour(Some(Rgb { r: 0xD9, g: 0xEA, b: 0xD3 })))
    );
    assert!(find(&rt, "rib-menu").is_none(), "and closes");
    // No fill is its own entry, after a separator.
    press(&mut rt, &key);
    press(&mut rt, "rib-menu-item-7");
    assert_eq!(ops.borrow().last(), Some(&EditOp::FillColour(None)));
}

#[test]
fn office_sheets_data_tools_are_sort_filter_and_table() {
    let t = tokens::theme(false);
    let tools = data_tools(&t);
    assert_eq!(tools.len(), 3);
    let (state, ops) = open_state(None);
    fn data_view(_: &State, cx: &mut rune::Cx<State>) -> rune::El {
        let t = tokens::theme(cx.system_dark());
        rune::row().w_full().children(data_tools(&t))
    }
    let mut rt = Runtime::new(state, data_view, (800.0, 200.0), env());
    settle(&mut rt);
    for (name, op) in [
        ("Sort A to Z", EditOp::Sort { ascending: true }),
        ("Filter", EditOp::Filter),
        ("Table", EditOp::MakeTable),
    ] {
        let key = tool_key(&rt, name);
        press(&mut rt, &key);
        assert_eq!(ops.borrow().last(), Some(&op), "{name}");
    }
}

#[test]
fn office_sheets_alt_equals_is_sum() {
    let key = |k: &str, alt: bool| KeyEvent {
        key: Key::Text(k.into()),
        mods: rune::Mods { alt, ..Default::default() },
    };
    assert!(is_auto_sum(&key("=", true)));
    assert!(!is_auto_sum(&key("=", false)));
    assert!(!is_auto_sum(&key("-", true)));
}

// ---------------------------------------------------------------------------
// The cell box
// ---------------------------------------------------------------------------

#[test]
fn office_sheets_references_are_cells_or_ranges_inside_the_sheet() {
    let at = |row, col| Cell { row, col };
    let ext = (400, 30);
    assert_eq!(parse_reference("B5", ext), Some((at(4, 1), at(4, 1))));
    assert_eq!(parse_reference(" b5 ", ext), Some((at(4, 1), at(4, 1))));
    assert_eq!(parse_reference("$B$5", ext), Some((at(4, 1), at(4, 1))));
    assert_eq!(parse_reference("B2:C4", ext), Some((at(1, 1), at(3, 2))));
    assert_eq!(parse_reference("C4:B2", ext), Some((at(3, 2), at(1, 1))));
    assert_eq!(parse_reference("AD400", ext), Some((at(399, 29), at(399, 29))));
    for wrong in [
        "", "B", "5", "B0", "B05", "5B", "B5:", ":B5", "B5:C", "B5C", "A1:B2:C3", "AE1", "A401", "A-1", "B 5", "Sheet1!A1",
        "ABCD1", "A99999999", "=A1",
    ] {
        assert_eq!(parse_reference(wrong, ext), None, "{wrong:?}");
    }
    // The sheet's own bounds, not the grid's: XFD1048576 is valid only in a sheet that big.
    assert_eq!(parse_reference("XFD1", (1_048_576, 16_384)), Some((at(0, 16_383), at(0, 16_383))));
    assert_eq!(parse_reference("XFE1", (1_048_576, 16_384)), None);
}

thread_local! {
    static TYPED: RefCell<String> = const { RefCell::new(String::new()) };
}

/// A stand-in for the cell box: Enter goes to what `TYPED` holds.
fn enter_goes(s: &mut State, ev: &KeyEvent, tx: &mut rune::Tx) -> bool {
    if ev.key == Key::Enter {
        let typed = TYPED.with(|t| t.borrow().clone());
        go_to(s, tx, &typed, (800.0, 600.0));
        return true;
    }
    false
}

fn go(rt: &mut Runtime<State>, typed: &str) {
    TYPED.with(|t| *t.borrow_mut() = typed.to_string());
    rt.key(KeyEvent {
        key: Key::Enter,
        mods: Default::default(),
    });
    settle(rt);
}

fn toast(rt: &mut Runtime<State>) -> Option<String> {
    viewers::ui(&mut rt.state).and_then(|ui| ui.toast.clone())
}

fn sheet_of(rt: &mut Runtime<State>) -> SheetView {
    match &viewers::ui(&mut rt.state).expect("a file").file.body {
        Body::Sheet(v) => v.clone(),
        other => panic!("{other:?} is not a sheet"),
    }
}

#[test]
fn office_sheets_the_cell_box_goes_there_or_says_why_not() {
    let (state, ops) = open_state(None);
    let mut rt = Runtime::new(state, view, (1278.0, 798.0), env());
    rt.on_key = Some(enter_goes);
    settle(&mut rt);
    go(&mut rt, "B5");
    let v = sheet_of(&mut rt);
    assert_eq!(v.current, Cell { row: 4, col: 1 });
    assert_eq!(v.selection, None);
    assert_eq!(toast(&mut rt).as_deref(), Some("Went to B5"));
    go(&mut rt, "b2:c4");
    let v = sheet_of(&mut rt);
    assert_eq!(v.current, Cell { row: 1, col: 1 });
    assert_eq!(v.selection, Some((Cell { row: 1, col: 1 }, Cell { row: 3, col: 2 })));
    assert_eq!(toast(&mut rt).as_deref(), Some("Went to B2:C4"));
    assert_eq!(
        ops.borrow().last(),
        Some(&EditOp::SelectCells {
            sheet: 0,
            range: beebeeb_office_protocol::office::CellRange { row: 1, col: 1, rows: 3, cols: 2 }
        })
    );
    // Far away: the grid scrolls to it.
    go(&mut rt, "Z300");
    assert!(sheet_of(&mut rt).top_left.row > 0, "the cell is brought into view");
    // A wrong reference leaves everything where it was, in the board's words and never the engine's.
    let before = sheet_of(&mut rt);
    let sent = ops.borrow().len();
    go(&mut rt, "Q#9");
    assert_eq!(toast(&mut rt).as_deref(), Some("Not a valid cell or range"));
    assert_eq!(sheet_of(&mut rt).current, before.current);
    assert_eq!(ops.borrow().len(), sent, "nothing is sent");
    assert!(!NOT_VALID.contains("Reference isn't valid"));
}

// ---------------------------------------------------------------------------
// The sheet tabs
// ---------------------------------------------------------------------------

fn tabs_view(s: &State, cx: &mut rune::Cx<State>) -> rune::El {
    let t = tokens::theme(cx.system_dark());
    let Some(Body::Sheet(v)) = s.viewer.ui.as_ref().map(|ui| &ui.file.body) else {
        panic!("a sheet is open")
    };
    rune::col()
        .w_full()
        .h_full()
        .child(sheets_tabs::sheet_tabs_edit(v, true, Some(800.0), true, &t))
}

fn tabs_rt() -> (Runtime<State>, Ops) {
    sheets_tabs::reset();
    let (state, ops) = open_state(None);
    let mut rt = Runtime::new(state, tabs_view, (800.0, 200.0), env());
    settle(&mut rt);
    (rt, ops)
}

fn double_press(rt: &mut Runtime<State>, key: &str) {
    let r = find(rt, key).unwrap_or_else(|| panic!("no element keyed {key}")).rect;
    let (x, y) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
    for _ in 0..2 {
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
    }
    settle(rt);
}

#[test]
fn office_sheets_a_double_press_renames_the_tab_with_one_edit() {
    let (mut rt, ops) = tabs_rt();
    double_press(&mut rt, "sh-tab-1");
    assert_eq!(sheets_tabs::renaming(), Some(1));
    assert_eq!(sheets_tabs::typed(), "2025", "the field holds the name");
    assert!(find(&rt, "edit-sheet-tab-name").is_some());
    rt.key(KeyEvent {
        key: Key::Text("!".into()),
        mods: Default::default(),
    });
    settle(&mut rt);
    assert_eq!(sheets_tabs::typed(), "2025!");
    rt.key(KeyEvent {
        key: Key::Enter,
        mods: Default::default(),
    });
    settle(&mut rt);
    assert_eq!(
        ops.borrow().iter().filter(|o| matches!(o, EditOp::RenameSheet { .. })).count(),
        1
    );
    assert!(ops.borrow().contains(&EditOp::RenameSheet { sheet: 1, name: "2025!".into() }));
    assert_eq!(sheets_tabs::renaming(), None);
    assert!(find(&rt, "sh-tab-1").is_some(), "the tab is back");
    assert_eq!(sheet_of(&mut rt).tabs[1], "2025!");
}

#[test]
fn office_sheets_escape_keeps_the_name_and_a_bad_name_stays_open() {
    let (mut rt, ops) = tabs_rt();
    double_press(&mut rt, "sh-tab-2");
    rt.key(KeyEvent {
        key: Key::Escape,
        mods: Default::default(),
    });
    settle(&mut rt);
    assert_eq!(sheets_tabs::renaming(), None);
    assert!(ops.borrow().is_empty());
    // A name another sheet has stays in the field, with words, and sends nothing.
    double_press(&mut rt, "sh-tab-2");
    for _ in 0..5 {
        rt.key(KeyEvent {
            key: Key::Backspace,
            mods: Default::default(),
        });
    }
    for c in "2025".chars() {
        rt.key(KeyEvent {
            key: Key::Text(c.to_string()),
            mods: Default::default(),
        });
    }
    rt.key(KeyEvent {
        key: Key::Enter,
        mods: Default::default(),
    });
    settle(&mut rt);
    assert_eq!(sheets_tabs::renaming(), Some(2), "still being renamed");
    assert!(ops.borrow().is_empty(), "nothing sent");
    assert_eq!(toast(&mut rt).as_deref(), Some("Another sheet has that name"));
}

#[test]
fn office_sheets_names_are_checked_before_they_are_sent() {
    let others = ["2026", "Notes"];
    assert_eq!(sheets_tabs::name_problem("", &others), Some("A sheet needs a name"));
    assert!(sheets_tabs::name_problem(&"x".repeat(32), &others).is_some());
    assert!(sheets_tabs::name_problem(&"x".repeat(31), &others).is_none());
    for bad in ["a:b", "a\\b", "a/b", "a?b", "a*b", "[a]"] {
        assert!(sheets_tabs::name_problem(bad, &others).is_some(), "{bad}");
    }
    assert!(sheets_tabs::name_problem("notes", &others).is_some(), "names differ by more than case");
    assert!(sheets_tabs::name_problem("Budget", &others).is_none());
}

#[test]
fn office_sheets_the_plus_is_dimmed_and_says_why() {
    let (mut rt, ops) = tabs_rt();
    let add = find(&rt, "sh-tab-add").expect("the + is there");
    assert_eq!(add.label.as_deref(), Some("Add sheet"));
    assert_eq!(add.help.as_deref(), Some("Adding sheets isn't available in this editor yet"));
    press(&mut rt, "sh-tab-add");
    assert_eq!(
        toast(&mut rt).as_deref(),
        Some("Adding sheets isn't available in this editor yet")
    );
    assert!(ops.borrow().is_empty(), "no edit exists for it");
}

#[test]
fn office_sheets_the_viewers_tabs_have_no_plus_and_no_rename() {
    sheets_tabs::reset();
    let t = tokens::theme(false);
    fn viewer_tabs(s: &State, cx: &mut rune::Cx<State>) -> rune::El {
        let t = tokens::theme(cx.system_dark());
        let Some(Body::Sheet(v)) = s.viewer.ui.as_ref().map(|ui| &ui.file.body) else {
            panic!("a sheet is open")
        };
        rune::col().w_full().h_full().child(sheets_tabs::sheet_tabs(v, true, Some(800.0), &t))
    }
    let _ = t;
    let (state, _) = open_state(None);
    let mut rt = Runtime::new(state, viewer_tabs, (800.0, 200.0), env());
    settle(&mut rt);
    assert!(find(&rt, "sh-tab-add").is_none());
    double_press(&mut rt, "sh-tab-1");
    assert_eq!(sheets_tabs::renaming(), None);
}

#[test]
fn office_sheets_ribbon_scene_is_the_workbook_with_its_ribbon() {
    let mut state = init();
    scene(&mut state);
    let mut rt = Runtime::new(state, view, (1278.0, 798.0), env());
    settle(&mut rt);
    assert!(find(&rt, "rib").is_some(), "the ribbon");
    assert!(find(&rt, "sh-cellbox").is_some(), "over the workbook");
}
