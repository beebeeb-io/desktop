//! The sheet ribbon's tools (DW-1508e's ribbon, DW-1559's Data tab) and the cell box's go-to.
//!
//! Home: Undo, Redo, Font, Bold, Italic, Underline (lit from `Marks`), alignment, Currency, .00 and %, Fill colour, Sum
//! (Alt+=), Sort and AutoFilter, each one `EditOp` on the selection the surface told the helper (`SelectCells`). Data:
//! Sort A to Z, Filter, Table ([`data_tools`]; the shared `Tab` has no Data tab yet, see the branch notes). Insert and
//! Formulas keep only what the web editor has: Table and Sum.
//!
//! The cell box ([`go_to`]): a cell or a range, Enter, and the grid goes there ("Went to B5"); anything else says "Not a
//! valid cell or range" (the board's words, never the engine's).
#![allow(dead_code)]

use std::cell::Cell as Slot;

use beebeeb_office_protocol::office::{EditOp, Mark, Marks, NumberFormat, Rgb, TextAlign};
use rune::{El, Key, KeyEvent, Tx, row};

use super::sheet;
use crate::State;
use crate::frames::office::sheets::{self, Cell, SheetView};
use crate::frames::office::sheets::sheets_scroll;
use crate::frames::viewers::office::{self as vo, Face, Tool, ico};
use crate::frames::viewers::source::Body;
use crate::frames::viewers::{self, Intent, Ui};
use crate::theme::tokens::Theme;
use crate::ui::kit;
use crate::ui::menu::{self, Entry};

/// What the cell box says when it cannot go there.
pub const NOT_VALID: &str = "Not a valid cell or range";

const MENU: &str = "rib-menu";
/// Under the ribbon's tool row, in the ribbon's own coordinates.
const MENU_Y: f32 = 68.0;
const TOOL_WORD: rune::TextStyle = rune::TextStyle::new(12.0, 400.0).lh(15.0);

/// The pop-ups of the Home tab, one open at a time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pop {
    Font,
    Fill,
}

thread_local! {
    /// The open pop-up and its highlighted entry. The shared `OfficeUi.ribbon` is the text ribbon's own; this stays with
    /// the window's thread.
    static OPEN: Slot<Option<Pop>> = const { Slot::new(None) };
    static HOT: Slot<Option<usize>> = const { Slot::new(None) };
}

pub fn open_pop() -> Option<Pop> {
    OPEN.with(Slot::get)
}

pub(crate) fn set_open(pop: Option<Pop>) {
    OPEN.with(|o| o.set(pop));
    HOT.with(|h| h.set(None));
}

const FONTS: [&str; 6] = ["Inter", "Arial", "Calibri", "Georgia", "Times New Roman", "Courier New"];
/// Fill colours (ids are the hex); "none" clears.
const FILLS: [(&str, &str); 7] = [
    ("FFF2CC", "Light yellow"),
    ("D9EAD3", "Light green"),
    ("CFE2F3", "Light blue"),
    ("F4CCCC", "Light red"),
    ("EAD1DC", "Light purple"),
    ("D9D9D9", "Light grey"),
    ("FFFF00", "Yellow"),
];

fn rgb(hex: &str) -> Option<Rgb> {
    let n = u32::from_str_radix(hex, 16).ok()?;
    Some(Rgb {
        r: (n >> 16) as u8,
        g: (n >> 8) as u8,
        b: n as u8,
    })
}

// ---------------------------------------------------------------------------
// Sending an edit
// ---------------------------------------------------------------------------

/// Hand `op` to the editor through the source. A source that does not take it says "Not built yet", never nothing.
pub fn send(s: &mut State, tx: &mut Tx, op: EditOp, asked: &'static str) {
    tx.instant();
    if let Some(ui) = viewers::ui(s) {
        let (source, id) = ui.edit_source();
        let id = id.to_string();
        if !source.edit(&id, op) {
            ui.intent(Intent::Office(asked));
        }
    }
}

fn ops(op: EditOp, asked: &'static str) -> impl Fn(&mut State, &mut Tx) + 'static {
    move |s: &mut State, tx: &mut Tx| {
        // A tool pressed while a pop-up is open closes it.
        set_open(None);
        send(s, tx, op.clone(), asked);
    }
}

fn open(pop: Pop) -> impl Fn(&mut State, &mut Tx) + 'static {
    move |_: &mut State, tx: &mut Tx| {
        tx.instant();
        let was = open_pop();
        set_open((was != Some(pop)).then_some(pop));
    }
}

// ---------------------------------------------------------------------------
// The tools
// ---------------------------------------------------------------------------

/// The Home row as it is built: its tools, and where along the row each pop-up's button starts.
struct Tools<'a> {
    t: &'a Theme,
    tools: Vec<El>,
    x: f32,
    n: usize,
    seps: usize,
    at: Vec<(Pop, f32)>,
}

impl<'a> Tools<'a> {
    fn new(t: &'a Theme) -> Self {
        Tools {
            t,
            tools: Vec::new(),
            // The row's own padding.
            x: 8.0,
            n: 0,
            seps: 0,
            at: Vec::new(),
        }
    }

    fn key(&mut self) -> String {
        let key = format!("rt-{}", self.n);
        self.n += 1;
        key
    }

    fn tool(&mut self, name: &'static str, face: Face, state: Tool, act: impl Fn(&mut State, &mut Tx) + 'static) {
        let key = self.key();
        self.tools.push(vo::tool_with(&key, name, face, state, act, self.t));
        self.x += 28.0 + 2.0;
    }

    fn tool_pop(&mut self, name: &'static str, face: Face, pop: Pop) {
        self.at.push((pop, self.x));
        self.tool(name, face, Tool::Idle, open(pop));
    }

    fn sep(&mut self) {
        self.tools.push(vo::hairline(format!("rsep-{}", self.seps), self.t));
        self.seps += 1;
        self.x += kit::HAIR + 4.0 + 2.0;
    }

    fn pop(&mut self, name: &'static str, choice: &str, w: f32, pop: Pop) {
        let key = format!("rp-{}", self.at.len());
        self.at.push((pop, self.x));
        self.tools.push(vo::pop_with(&key, name, choice, w, open(pop), self.t));
        self.x += w + 2.0;
    }

    /// A tool with its word: 28 high, the glyph, the word.
    fn worded(
        &mut self,
        name: &'static str,
        word: &'static str,
        icon: &'static rune::Icon,
        hint: Option<&'static str>,
        act: impl Fn(&mut State, &mut Tx) + 'static,
    ) {
        let key = self.key();
        let t = self.t;
        let w = rune::text::advance(word, TOOL_WORD);
        let mut b = row()
            .key(key.clone())
            .role("button")
            .label(name)
            .h(28.0)
            .px(6.0)
            .gap(4.0)
            .center_y()
            .radius(7.0)
            .hover(t.hover)
            .press(t.press)
            .focusable(true)
            .preserve_focus_on_press(true)
            .on_press_tx(act)
            .child(kit::glyph(format!("{key}-icon"), icon, 15.0, t.text_secondary))
            .child(kit::label(format!("{key}-label"), word, TOOL_WORD, t.text).w(w).a11y_hidden());
        if let Some(hint) = hint {
            b = b.shortcut_hint(hint);
        }
        self.tools.push(b);
        self.x += 6.0 * 2.0 + 4.0 + 15.0 + w + 2.0;
    }
}

fn on(lit: bool) -> Tool {
    if lit { Tool::On } else { Tool::Idle }
}

/// Home's tools, lit from what the helper last said of the selection.
fn home_tools(ui: &Ui, t: &Theme) -> (Vec<El>, Vec<(Pop, f32)>) {
    let edit = ui.office.edit.as_ref();
    let marks = edit.and_then(|e| e.marks);
    let (can_undo, can_redo) = edit.map_or((false, false), |e| (e.can_undo, e.can_redo));
    let lit = |f: fn(&Marks) -> bool| marks.as_ref().is_some_and(f);
    let glyph = |icon: &'static rune::Icon| Face::Glyph(icon, 15.0);
    let align = marks.map(|m| m.align);
    let mut r = Tools::new(t);
    r.tool(
        "Undo",
        glyph(&ico::UNDO),
        if can_undo { Tool::Idle } else { Tool::Off },
        ops(EditOp::Undo, "Undo"),
    );
    r.tool(
        "Redo",
        glyph(&ico::REDO),
        if can_redo { Tool::Idle } else { Tool::Off },
        ops(EditOp::Redo, "Redo"),
    );
    r.sep();
    // The helper does not report the font: the button says what it is for.
    r.pop("Font", "Font", 92.0, Pop::Font);
    r.sep();
    for (name, letter, style, mark, is_on) in [
        ("Bold", "B", vo::LETTER_HEAVY, Mark::Bold, lit(|m| m.bold)),
        ("Italic", "I", vo::LETTER, Mark::Italic, lit(|m| m.italic)),
        ("Underline", "U", vo::LETTER, Mark::Underline, lit(|m| m.underline)),
    ] {
        r.tool(
            name,
            Face::Letter(letter, style),
            on(is_on),
            ops(EditOp::Mark { mark, on: !is_on }, name),
        );
    }
    r.sep();
    for (name, icon, to) in [
        ("Align left", &ico::ALIGN_LEFT, TextAlign::Left),
        ("Align centre", &ico::ALIGN_CENTRE, TextAlign::Center),
        ("Align right", &ico::ALIGN_RIGHT, TextAlign::Right),
    ] {
        r.tool(name, glyph(icon), on(align == Some(to)), ops(EditOp::Align(to), name));
    }
    r.sep();
    for (name, letter, format) in [
        ("Currency", "\u{20AC}", NumberFormat::Currency),
        ("More decimals", ".00", NumberFormat::MoreDecimals),
        ("Percent", "%", NumberFormat::Percent),
    ] {
        r.tool(
            name,
            Face::Letter(letter, vo::LETTER),
            Tool::Idle,
            ops(EditOp::NumberFormat(format), name),
        );
    }
    r.sep();
    r.tool_pop("Fill colour", glyph(&ico::HIGHLIGHT), Pop::Fill);
    r.sep();
    r.tool(
        "Sum",
        Face::Letter("\u{03A3}", vo::LETTER),
        Tool::Idle,
        ops(EditOp::AutoSum, "Sum"),
    );
    r.worded("Sort", "Sort", &ico::LIST_NUMBERS, None, ops(EditOp::Sort { ascending: true }, "Sort"));
    r.worded("AutoFilter", "AutoFilter", &ico::TABLE, None, ops(EditOp::Filter, "AutoFilter"));
    (r.tools, r.at)
}

/// Alt+= is Sum, as in the sheet programs: `true` when `ev` is that key.
pub fn is_auto_sum(ev: &KeyEvent) -> bool {
    ev.mods.alt && !ev.mods.ctrl && !ev.mods.cmd && matches!(&ev.key, Key::Text(t) if t.as_str() == "=")
}

/// Data's tools: Sort A to Z, Filter, Table.
pub fn data_tools(t: &Theme) -> Vec<El> {
    let mut r = Tools::new(t);
    r.worded("Sort A to Z", "Sort A to Z", &ico::LIST_NUMBERS, None, ops(EditOp::Sort { ascending: true }, "Sort A to Z"));
    r.worded("Filter", "Filter", &ico::TABLE, None, ops(EditOp::Filter, "Filter"));
    r.worded("Table", "Table", &ico::TABLE, None, ops(EditOp::MakeTable, "Table"));
    r.tools
}

/// What the web editor has under Insert: Table.
fn insert_tools(t: &Theme) -> Vec<El> {
    let mut r = Tools::new(t);
    r.worded("Table", "Table", &ico::TABLE, None, ops(EditOp::MakeTable, "Table"));
    r.tools
}

/// What the web editor has under Formulas: Sum.
pub fn formulas_tools(t: &Theme) -> Vec<El> {
    let mut r = Tools::new(t);
    r.worded("Sum", "Sum", &ico::LIST_NUMBERS, Some("Alt+="), ops(EditOp::AutoSum, "Sum"));
    r.tools
}

/// The ribbon of a live workbook: the tabs, the open tab's tools and, over everything, the open pop-up.
pub fn ribbon(ui: &Ui, unsaved: bool, t: &Theme) -> El {
    use vo::Tab;
    let mut at = Vec::new();
    let tools = match ui.office.tab {
        Tab::Home => {
            let (tools, places) = home_tools(ui, t);
            at = places;
            tools
        }
        Tab::Insert => insert_tools(t),
        Tab::Data => data_tools(t),
        Tab::Formulas => formulas_tools(t),
        // Layout and Review are a text document's: never on a workbook's row.
        Tab::Layout | Tab::Review => Vec::new(),
    };
    let mut rib = vo::ribbon_of(ui, unsaved, super::word_ribbon::tool_group(tools), t);
    if let Some(pop) = open_pop() {
        if at.is_empty() {
            at = home_tools(ui, t).1;
        }
        let x = at.iter().find(|(p, _)| *p == pop).map_or(8.0, |(_, x)| *x);
        rib = rib.child(popup(pop, x, t));
    }
    rib
}

// ---------------------------------------------------------------------------
// The pop-ups
// ---------------------------------------------------------------------------

fn entries(pop: Pop) -> Vec<Entry> {
    match pop {
        Pop::Font => FONTS.iter().map(|f| Entry::item(f, f)).collect(),
        Pop::Fill => {
            let mut all: Vec<Entry> = FILLS.iter().map(|(id, name)| Entry::item(id, name)).collect();
            all.push(Entry::Sep);
            all.push(Entry::item("none", "No fill"));
            all
        }
    }
}

/// The edit an entry stands for.
fn op_of(pop: Pop, id: &str) -> Option<(EditOp, &'static str)> {
    Some(match pop {
        Pop::Font => (EditOp::FontFamily(FONTS.iter().find(|f| **f == id)?.to_string()), "Fonts"),
        Pop::Fill if id == "none" => (EditOp::FillColour(None), "Fill colour"),
        Pop::Fill => (EditOp::FillColour(Some(rgb(id)?)), "Fill colour"),
    })
}

fn name_of(pop: Pop) -> (&'static str, f32) {
    match pop {
        Pop::Font => ("Font", 180.0),
        Pop::Fill => ("Fill colour", 160.0),
    }
}

fn pick(s: &mut State, index: usize, tx: &mut Tx) {
    let Some(pop) = open_pop() else { return };
    let all = entries(pop);
    let Some(id) = all.get(index).and_then(Entry::id) else { return };
    set_open(None);
    if let Some((op, asked)) = op_of(pop, id) {
        send(s, tx, op, asked);
    }
}

fn hover(_: &mut State, index: usize) {
    HOT.with(|h| h.set(Some(index)));
}

fn close(_: &mut State, tx: &mut Tx) {
    tx.instant();
    set_open(None);
}

fn menu_key(s: &mut State, ev: &KeyEvent, tx: &mut Tx) -> bool {
    let Some(pop) = open_pop() else { return false };
    let all = entries(pop);
    let hot = HOT.with(Slot::get);
    let to = match ev.key {
        Key::Down => menu::step(&all, hot, 1),
        Key::Up => menu::step(&all, hot, -1),
        Key::Home => menu::step(&all, None, 1),
        Key::End => menu::step(&all, None, -1),
        Key::Enter => {
            if let Some(hot) = hot {
                pick(s, hot, tx);
            }
            return true;
        }
        Key::Escape => {
            close(s, tx);
            return true;
        }
        // A menu is modal: no other key reaches the grid under it.
        _ => return true,
    };
    if let Some(to) = to {
        HOT.with(|h| h.set(Some(to)));
        if let Some(key) = menu::item_key(MENU, &all, to) {
            tx.focus(key, false);
        }
    }
    true
}

/// The pop-up, hung under its button and over everything else.
fn popup(pop: Pop, x: f32, t: &Theme) -> El {
    let (name, w) = name_of(pop);
    menu::menu(MENU, name, w, &entries(pop), HOT.with(Slot::get), t, pick, hover, close)
        .on_key(menu_key)
        .absolute(x.max(0.0), MENU_Y)
        .raise()
}

// ---------------------------------------------------------------------------
// The cell box
// ---------------------------------------------------------------------------

/// A column's letters as a number: "A" is 0, "AA" is 26. `None`: not letters, or past the last column (XFD).
fn column_of(letters: &str) -> Option<u32> {
    if letters.is_empty() || letters.len() > 3 || !letters.bytes().all(|b| b.is_ascii_alphabetic()) {
        return None;
    }
    let n = letters
        .bytes()
        .fold(0u32, |n, b| n * 26 + u32::from(b.to_ascii_uppercase() - b'A') + 1);
    (n <= 16_384).then(|| n - 1)
}

/// One cell's name ("B5", "$B$5") as a cell, zero-based. A leading zero in the row is not a row.
fn cell_of(name: &str) -> Option<Cell> {
    let name: String = name.chars().filter(|c| *c != '$').collect();
    let split = name.find(|c: char| !c.is_ascii_alphabetic())?;
    let (letters, digits) = name.split_at(split);
    if digits.is_empty() || digits.len() > 7 || !digits.bytes().all(|b| b.is_ascii_digit()) || digits.starts_with('0') {
        return None;
    }
    let row: u32 = digits.parse().ok()?;
    Some(Cell {
        row: row - 1,
        col: column_of(letters)?,
    })
}

/// What the cell box was typed: a cell ("B5") or a range ("B2:C4"), inside a sheet of `extent` (rows, columns). The
/// corners come back as typed (the first is the current cell). `None`: not a reference or outside the sheet.
pub fn parse_reference(typed: &str, extent: (u32, u32)) -> Option<(Cell, Cell)> {
    let typed = typed.trim();
    let (a, b) = match typed.split_once(':') {
        Some((a, b)) => (cell_of(a.trim())?, cell_of(b.trim())?),
        None => {
            let a = cell_of(typed)?;
            (a, a)
        }
    };
    let inside = |c: Cell| c.row < extent.0 && c.col < extent.1;
    (inside(a) && inside(b)).then_some((a, b))
}

/// "B5", or "B2:C4".
fn reference_words(a: Cell, b: Cell) -> String {
    if a == b {
        sheets::a1(a)
    } else {
        format!("{}:{}", sheets::a1(a), sheets::a1(b))
    }
}

/// The cell box's Enter: go to what was typed, show it, tell the helper and say where ("Went to B5"). Anything else
/// leaves the grid alone and says [`NOT_VALID`]. `room`: the grid's size in points (to scroll the cell into view).
/// `true`: it went.
pub fn go_to(s: &mut State, tx: &mut Tx, typed: &str, room: (f32, f32)) -> bool {
    let Some(Body::Sheet(view)) = viewers::ui(s).map(|ui| &mut ui.file.body) else {
        return false;
    };
    let Some((a, b)) = parse_reference(typed, view.extent) else {
        if let Some(ui) = viewers::ui(s) {
            ui.toast = Some(NOT_VALID.to_string());
        }
        return false;
    };
    // A merged cell is entered at its origin, as a press does.
    let a = view.merge_origin(a);
    sheet::select(view, a, b);
    sheet::reveal(view, a, room);
    sheets_scroll::follow(view, tx);
    let (sheet_no, range) = (view.sheet, sheet::range_of(view));
    // Told to the helper as a settled selection is; a source that does not take it changes nothing the person sees.
    if let Some(ui) = viewers::ui(s) {
        let (source, id) = ui.edit_source();
        let id = id.to_string();
        source.edit(&id, EditOp::SelectCells { sheet: sheet_no, range });
    }
    if let Some(ui) = viewers::ui(s) {
        ui.toast = Some(format!("Went to {}", reference_words(a, b)));
    }
    true
}

// ---------------------------------------------------------------------------
// The board's frames
// ---------------------------------------------------------------------------

/// DW-1508e and DW-1559: the workbook of DW-1508 with the editor's ribbon over it, on Home.
pub fn scene(s: &mut State) {
    viewers::set_scene(s, "DW-1508");
    set_open(None);
    if let Some(ui) = viewers::ui(s) {
        ui.office.tab = vo::Tab::Home;
        ui.office.edit = Some(super::EditUi::new(beebeeb_office_protocol::office::EditKind::Xlsx));
    }
}

// The tests of this file and of `sheets_tabs.rs` are one file, mounted there (`sheets_tabs` is private to `sheets`).
