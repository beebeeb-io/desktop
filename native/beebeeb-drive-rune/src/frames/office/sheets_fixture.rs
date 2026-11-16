//! The board's spreadsheet (DW-1508): "Budget 2026", three sheets, B2:B9 selected, B10 the current cell.
//! A development build and the gate draw it; a live host fills a [`SheetView`] from the document engine's
//! geometry and the decrypted cells instead (the grid is drawn by us from this shape either way).

use super::{Cell, CellText, SheetView};

/// The file's name on the board.
pub const FILE_NAME: &str = "Budget 2026.xlsx";

/// Column widths in points: the board's eight columns (`SH_COLS`, A to H), 118 wide each.
const COL_WIDTHS: [f32; 8] = [118.0; 8];
/// Row height in points: the board's 24 for every row, header row included.
const ROW_HEIGHT: f32 = 24.0;
/// The board draws 19 rows (`Array.from({ length: 19 })`).
const ROWS: usize = 19;

/// The board's `SH_DATA`, row by row, exactly: the header row, the eight items, the total, an empty row, the budget
/// and what is left. Q1's eight items (B2:B9) are selected; B10 holds their sum, 13,220 (the status line computes
/// Sum, Average and Count from the selection; they are never typed in).
const DATA: [&[&str]; 13] = [
    &["Item", "Q1", "Q2", "Q3", "Total"],
    &["Studio rent", "4,200", "4,200", "4,200", "12,600"],
    &["Equipment", "3,150", "820", "1,940", "5,910"],
    &["Travel", "640", "1,880", "2,410", "4,930"],
    &["Software", "410", "410", "455", "1,275"],
    &["Insurance", "1,120", "0", "0", "1,120"],
    &["Freelancers", "2,800", "3,600", "5,150", "11,550"],
    &["Prints and frames", "380", "0", "1,260", "1,640"],
    &["Marketing", "520", "640", "700", "1,860"],
    &["Total", "13,220", "11,550", "16,115", "40,885"],
    &[],
    &["Budget", "14,000", "14,000", "16,000", "44,000"],
    &["Left", "780", "2,450", "\u{2212}115", "3,115"],
];

fn text(row: u32, col: u32, text: &str, right_aligned: bool, bold: bool) -> CellText {
    CellText {
        row,
        col,
        text: text.to_string(),
        right_aligned,
        bold,
        // The board's cells carry no separate number: the status line reads their text, as it always did.
        value: None,
        formula: None,
    }
}

/// The board's workbook as a [`SheetView`].
pub fn board() -> SheetView {
    let mut cells = Vec::new();
    for (r, line) in DATA.iter().enumerate() {
        for (c, word) in line.iter().enumerate() {
            // The board: the header row and the total row are 600; the figures (every column after A, below the
            // header) sit right.
            let bold = r == 0 || r == 9;
            cells.push(text(r as u32, c as u32, word, c > 0 && r > 0, bold));
        }
    }
    SheetView {
        tabs: vec!["2026".into(), "2025".into(), "Notes".into()],
        active: 0,
        col_widths: COL_WIDTHS.to_vec(),
        row_heights: vec![ROW_HEIGHT; ROWS],
        cells,
        // B2:B9.
        selection: Some((Cell { row: 1, col: 1 }, Cell { row: 8, col: 1 })),
        // B10.
        current: Cell { row: 9, col: 1 },
        formula: "=SUM(B2:B9)".into(),
        // The board draws no frozen line.
        frozen: (0, 0),
        merges: Vec::new(),
        sheet: 0,
        extent: (ROWS as u32, COL_WIDTHS.len() as u32),
        top_left: Cell { row: 0, col: 0 },
        anchors: vec![],
        missing_content: false,
        more: super::More::default(),
    }
}
