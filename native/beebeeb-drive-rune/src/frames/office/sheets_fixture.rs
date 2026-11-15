//! The board's spreadsheet (DW-1508): "Budget 2026", three sheets, B2:B9 selected, B10 the current cell.
//! A development build and the gate draw it; a live host fills a [`SheetView`] from the document engine's
//! geometry and the decrypted cells instead (the grid is drawn by us from this shape either way).

use super::{Cell, CellText, SheetView};

/// The file's name on the board.
pub const FILE_NAME: &str = "Budget 2026.xlsx";

/// Column widths in points: A is the category, B the amount, the rest are the sheet's empty columns.
const COL_WIDTHS: [f32; 6] = [188.0, 112.0, 96.0, 96.0, 96.0, 96.0];
/// Row heights in points: 24 for the header row, 22 for the rest.
const ROW_HEIGHT: f32 = 22.0;
const ROWS: usize = 16;

/// The eight lines of the budget: B2:B9 sum to 13,220, average 1,652.50, count 8 (the board's status line).
const LINES: [(&str, &str); 8] = [
    ("Rent", "3,600"),
    ("Studio", "2,400"),
    ("Software", "1,850"),
    ("Travel", "1,500"),
    ("Equipment", "1,280"),
    ("Insurance", "1,120"),
    ("Accountant", "900"),
    ("Other", "570"),
];

fn text(row: u32, col: u32, text: &str, right_aligned: bool, bold: bool) -> CellText {
    CellText {
        row,
        col,
        text: text.to_string(),
        right_aligned,
        bold,
    }
}

/// The board's workbook as a [`SheetView`].
pub fn board() -> SheetView {
    let mut cells = vec![
        text(0, 0, "Category", false, true),
        text(0, 1, "Planned (EUR)", true, true),
    ];
    for (i, (name, amount)) in LINES.iter().enumerate() {
        let row = i as u32 + 1;
        cells.push(text(row, 0, name, false, false));
        cells.push(text(row, 1, amount, true, false));
    }
    cells.push(text(9, 0, "Total", false, true));
    cells.push(text(9, 1, "13,220", true, true));
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
        frozen: (1, 0),
        merges: Vec::new(),
    }
}
