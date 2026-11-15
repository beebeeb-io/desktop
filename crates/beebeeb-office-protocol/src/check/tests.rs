//! Each check refuses every violation it names, one violation at a time, and accepts the exact ask.

use std::collections::BTreeSet;
use std::time::Duration;

use super::*;
use crate::office::{Align, CellRange, CellText, Frozen, GrantRef, OutlineEntry, Rect, SheetInfo};

const OUT: GrantRef = GrantRef { wire: 9, len: 0 };
/// A4 in points: 794 x 1123 px at scale 1.0.
const A4: PageSize = PageSize {
    w_pt: 595.3,
    h_pt: 841.9,
};
/// 4,267 x 133 px at scale 1.0: wider than one tile.
const WIDE: PageSize = PageSize {
    w_pt: 3200.0,
    h_pt: 100.0,
};
/// 133 x 4,267 px at scale 1.0: taller than one tile.
const TALL: PageSize = PageSize {
    w_pt: 100.0,
    h_pt: 3200.0,
};
const L: Limits = Limits::CONTRACT;

fn whole(scale: f32) -> Render {
    Render {
        page: 0,
        scale,
        region: None,
        out: OUT,
    }
}

fn tile(x: u32, y: u32, w: u32, h: u32) -> Render {
    Render {
        page: 0,
        scale: 1.0,
        region: Some(Rect { x, y, w, h }),
        out: OUT,
    }
}

fn rendered(w: u32, h: u32) -> Rendered {
    Rendered {
        w,
        h,
        missing_content: false,
    }
}

fn len(w: u32, h: u32) -> u64 {
    u64::from(w) * u64::from(h) * 4
}

#[test]
fn rendered_exact_ask_is_accepted() {
    assert_eq!(page_pixels(&A4, 1.0), Some((794, 1123)));
    assert_eq!(page_pixels(&A4, 2.0), Some((1587, 2245)));

    assert_eq!(
        check_rendered(&whole(1.0), &A4, len(794, 1123), &rendered(794, 1123), &L),
        Ok(())
    );
    assert_eq!(
        check_rendered(&whole(2.0), &A4, len(1587, 2245), &rendered(1587, 2245), &L),
        Ok(())
    );
    // ±1 px of rounding on either side.
    assert_eq!(
        check_rendered(&whole(1.0), &A4, len(793, 1122), &rendered(793, 1122), &L),
        Ok(())
    );
    assert_eq!(
        check_rendered(&whole(1.0), &A4, len(795, 1124), &rendered(795, 1124), &L),
        Ok(())
    );
    // A tile inside the page, and one that ends exactly at its corner.
    assert_eq!(
        check_rendered(&tile(0, 512, 512, 300), &A4, len(512, 300), &rendered(512, 300), &L),
        Ok(())
    );
    assert_eq!(
        check_rendered(&tile(282, 611, 512, 512), &A4, len(512, 512), &rendered(512, 512), &L),
        Ok(())
    );
    // A tile at the limit.
    assert_eq!(
        check_rendered(&tile(0, 0, 4096, 10), &WIDE, len(4096, 10), &rendered(4096, 10), &L),
        Ok(())
    );
    assert_eq!(asked_size(&tile(0, 0, 4096, 10), &WIDE, &L), Ok((4096, 10)));
}

#[test]
fn rendered_with_another_size_is_refused() {
    // Only the width differs (by more than the rounding), and the section length agrees with the wrong size: the
    // width comparison alone must refuse it.
    let ask = tile(0, 0, 100, 200);
    assert_eq!(
        check_rendered(&ask, &A4, len(120, 200), &rendered(120, 200), &L),
        Err(Refused::WrongSize)
    );
    assert_eq!(
        check_rendered(&ask, &A4, len(98, 200), &rendered(98, 200), &L),
        Err(Refused::WrongSize)
    );
    // Only the height differs.
    assert_eq!(
        check_rendered(&ask, &A4, len(100, 230), &rendered(100, 230), &L),
        Err(Refused::WrongSize)
    );
    assert_eq!(
        check_rendered(&ask, &A4, len(100, 198), &rendered(100, 198), &L),
        Err(Refused::WrongSize)
    );
    // A whole page answered with the sides swapped.
    assert_eq!(
        check_rendered(&whole(1.0), &A4, len(1123, 794), &rendered(1123, 794), &L),
        Err(Refused::WrongSize)
    );
    // Nothing at all, for an ask of one pixel.
    let one = tile(0, 0, 1, 1);
    assert_eq!(
        check_rendered(&one, &A4, 0, &rendered(0, 1), &L),
        Err(Refused::WrongSize)
    );
    assert_eq!(
        check_rendered(&one, &A4, 0, &rendered(1, 0), &L),
        Err(Refused::WrongSize)
    );
}

#[test]
fn rendered_over_4096_is_refused() {
    // One pixel over the limit is within the rounding of the ask, and the section length agrees: the limit alone
    // must refuse it.
    let ask = tile(0, 0, 4096, 10);
    assert_eq!(
        check_rendered(&ask, &WIDE, len(4097, 10), &rendered(4097, 10), &L),
        Err(Refused::TooLarge)
    );
    let ask = Render {
        page: 0,
        scale: 1.0,
        region: Some(Rect {
            x: 0,
            y: 0,
            w: 10,
            h: 4096,
        }),
        out: OUT,
    };
    assert_eq!(
        check_rendered(&ask, &TALL, len(10, 4097), &rendered(10, 4097), &L),
        Err(Refused::TooLarge)
    );
    // The ask itself is refused over the limit, whole page or tile.
    assert_eq!(asked_size(&whole(1.0), &WIDE, &L), Err(Refused::TooLarge));
    assert_eq!(asked_size(&whole(6.0), &A4, &L), Err(Refused::TooLarge));
    assert_eq!(asked_size(&tile(0, 0, 4097, 10), &WIDE, &L), Err(Refused::TooLarge));
    assert_eq!(
        check_rendered(&whole(1.0), &WIDE, len(4267, 133), &rendered(4267, 133), &L),
        Err(Refused::TooLarge)
    );
}

#[test]
fn rendered_len_mismatch_is_refused() {
    let ask = tile(0, 0, 100, 200);
    for out_len in [len(100, 200) + 4, len(100, 200) - 4, len(100, 200) * 2, 0, u64::MAX] {
        assert_eq!(
            check_rendered(&ask, &A4, out_len, &rendered(100, 200), &L),
            Err(Refused::LengthMismatch),
            "section of {out_len} bytes"
        );
    }
}

#[test]
fn render_ask_that_cannot_be_sized_is_refused() {
    for scale in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 0.0, -0.0, -1.0, 1e-9] {
        assert_eq!(
            asked_size(&whole(scale), &A4, &L),
            Err(Refused::BadAsk),
            "scale {scale}"
        );
    }
    for bad in [f32::NAN, f32::INFINITY, 0.0, -10.0] {
        let page = PageSize { w_pt: bad, h_pt: 842.0 };
        assert_eq!(asked_size(&whole(1.0), &page, &L), Err(Refused::BadAsk), "width {bad}");
        let page = PageSize { w_pt: 595.0, h_pt: bad };
        assert_eq!(asked_size(&whole(1.0), &page, &L), Err(Refused::BadAsk), "height {bad}");
    }
    // Regions that are empty or not inside the 794 x 1123 page.
    for (x, y, w, h) in [
        (0, 0, 0, 10),
        (0, 0, 10, 0),
        (700, 0, 100, 10),
        (0, 1100, 10, 24),
        (794, 0, 1, 1),
        (u32::MAX, 0, 1, 1),
        (0, u32::MAX, 1, 1),
        (1, 1, u32::MAX, 1),
    ] {
        assert_eq!(
            asked_size(&tile(x, y, w, h), &A4, &L),
            Err(Refused::BadAsk),
            "region {x},{y} {w}x{h}"
        );
    }
    // An ask that cannot be sized refuses every answer.
    assert_eq!(
        check_rendered(&whole(f32::NAN), &A4, 4, &rendered(1, 1), &L),
        Err(Refused::BadAsk)
    );
}

fn pages(n: u32) -> Opened {
    Opened {
        pages: n,
        sizes: vec![A4; n as usize],
    }
}

#[test]
fn opened_exact_is_accepted() {
    assert_eq!(check_opened(&pages(0), &L), Ok(()));
    assert_eq!(check_opened(&pages(1), &L), Ok(()));
    assert_eq!(check_opened(&pages(2000), &L), Ok(()));
    let mixed = Opened {
        pages: 2,
        sizes: vec![
            A4,
            PageSize {
                w_pt: 960.0,
                h_pt: 540.0,
            },
        ],
    };
    assert_eq!(check_opened(&mixed, &L), Ok(()));
}

#[test]
fn opened_over_2000_pages_is_refused() {
    assert_eq!(check_opened(&pages(2001), &L), Err(Refused::TooManyPages));
    let lying = Opened {
        pages: u32::MAX,
        sizes: Vec::new(),
    };
    assert_eq!(check_opened(&lying, &L), Err(Refused::TooManyPages));
}

#[test]
fn opened_with_a_count_mismatch_is_refused() {
    assert_eq!(
        check_opened(
            &Opened {
                pages: 2,
                sizes: vec![A4]
            },
            &L
        ),
        Err(Refused::PageCountMismatch)
    );
    assert_eq!(
        check_opened(
            &Opened {
                pages: 1,
                sizes: vec![A4, A4]
            },
            &L
        ),
        Err(Refused::PageCountMismatch)
    );
    assert_eq!(
        check_opened(
            &Opened {
                pages: 0,
                sizes: vec![A4]
            },
            &L
        ),
        Err(Refused::PageCountMismatch)
    );
}

#[test]
fn opened_with_a_bad_page_size_is_refused() {
    for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 0.0, -0.0, -1.0] {
        let w = Opened {
            pages: 2,
            sizes: vec![A4, PageSize { w_pt: bad, h_pt: 842.0 }],
        };
        assert_eq!(check_opened(&w, &L), Err(Refused::BadPageSize), "width {bad}");
        let h = Opened {
            pages: 1,
            sizes: vec![PageSize { w_pt: 595.0, h_pt: bad }],
        };
        assert_eq!(check_opened(&h, &L), Err(Refused::BadPageSize), "height {bad}");
    }
}

fn sheet(name_len: usize, rows: u32, cols: u32) -> SheetInfo {
    SheetInfo {
        name: "s".repeat(name_len),
        rows,
        cols,
    }
}

#[test]
fn sheets_exact_is_accepted() {
    let most = OpenedSheets {
        sheets: vec![sheet(256, MAX_SHEET_ROWS, MAX_SHEET_COLS); 256],
    };
    assert_eq!(check_sheets(&most, &L), Ok(()));
    assert_eq!(check_sheets(&OpenedSheets { sheets: Vec::new() }, &L), Ok(()));
    assert_eq!(
        check_sheets(
            &OpenedSheets {
                sheets: vec![sheet(0, 0, 0)]
            },
            &L
        ),
        Ok(())
    );
}

#[test]
fn sheets_over_256_are_refused() {
    let many = OpenedSheets {
        sheets: vec![sheet(6, 10, 10); 257],
    };
    assert_eq!(check_sheets(&many, &L), Err(Refused::TooManySheets));
}

#[test]
fn sheets_with_a_long_name_are_refused() {
    let long = OpenedSheets {
        sheets: vec![sheet(6, 10, 10), sheet(257, 10, 10)],
    };
    assert_eq!(check_sheets(&long, &L), Err(Refused::NameTooLong));
}

#[test]
fn sheets_larger_than_the_format_are_refused() {
    let rows = OpenedSheets {
        sheets: vec![sheet(6, MAX_SHEET_ROWS + 1, 10)],
    };
    assert_eq!(check_sheets(&rows, &L), Err(Refused::SheetTooLarge));
    let cols = OpenedSheets {
        sheets: vec![sheet(6, 10, MAX_SHEET_COLS + 1)],
    };
    assert_eq!(check_sheets(&cols, &L), Err(Refused::SheetTooLarge));
}

/// Rows 5..=14, columns 5..=8: 10 x 4 cells.
fn ask() -> Cells {
    Cells {
        sheet: 1,
        range: CellRange {
            row: 5,
            col: 5,
            rows: 10,
            cols: 4,
        },
    }
}

fn cell(row: u32, col: u32) -> CellText {
    CellText {
        row,
        col,
        text: "1.250".to_owned(),
        align: Align::Right,
        bold: false,
    }
}

fn data(cells: Vec<CellText>) -> CellData {
    CellData {
        cells,
        col_widths: vec![48.0; 4],
        row_heights: vec![15.0; 10],
        merges: Vec::new(),
        frozen: Frozen { rows: 0, cols: 0 },
    }
}

fn every_cell() -> Vec<CellText> {
    (5..15).flat_map(|r| (5..9).map(move |c| cell(r, c))).collect()
}

#[test]
fn cells_exact_ask_is_accepted() {
    assert_eq!(check_cells(&ask(), &data(Vec::new()), &L), Ok(()));
    assert_eq!(check_cells(&ask(), &data(every_cell()), &L), Ok(()));

    let mut d = data(vec![cell(5, 5), cell(14, 8)]);
    d.cells[0].text = "x".repeat(MAX_CELL_TEXT_BYTES);
    d.merges = vec![
        CellRange {
            row: 5,
            col: 5,
            rows: 10,
            cols: 4,
        },
        CellRange {
            row: 6,
            col: 6,
            rows: 2,
            cols: 2,
        },
    ];
    d.col_widths = vec![0.0, 48.0, 120.5, 3.0];
    d.frozen = Frozen {
        rows: MAX_SHEET_ROWS,
        cols: MAX_SHEET_COLS,
    };
    assert_eq!(check_cells(&ask(), &d, &L), Ok(()));
}

#[test]
fn cells_outside_the_range_are_refused() {
    // Three cells, well under the range's 40: only the position check can refuse them.
    for (r, c) in [(4, 5), (5, 4), (15, 5), (5, 9), (15, 9), (0, 0), (u32::MAX, u32::MAX)] {
        let d = data(vec![cell(5, 5), cell(r, c), cell(6, 6)]);
        assert_eq!(
            check_cells(&ask(), &d, &L),
            Err(Refused::CellOutsideRange),
            "cell {r},{c}"
        );
    }
    // A range at the far corner of the sheet: no overflow.
    let corner = Cells {
        sheet: 0,
        range: CellRange {
            row: u32::MAX - 1,
            col: u32::MAX - 1,
            rows: 1,
            cols: 1,
        },
    };
    let mut d = data(vec![cell(u32::MAX, u32::MAX - 1)]);
    d.col_widths = vec![48.0];
    d.row_heights = vec![15.0];
    assert_eq!(check_cells(&corner, &d, &L), Err(Refused::CellOutsideRange));
}

#[test]
fn cells_more_than_the_range_holds_are_refused() {
    let mut cells = every_cell();
    cells.push(cell(5, 5));
    assert_eq!(check_cells(&ask(), &data(cells), &L), Err(Refused::TooManyCells));
}

#[test]
fn cells_with_text_over_the_cap_are_refused() {
    let mut d = data(vec![cell(5, 5), cell(6, 6)]);
    d.cells[1].text = "x".repeat(MAX_CELL_TEXT_BYTES + 1);
    assert_eq!(check_cells(&ask(), &d, &L), Err(Refused::TextTooLong));
}

#[test]
fn cells_merge_outside_the_range_is_refused() {
    for m in [
        CellRange {
            row: 4,
            col: 5,
            rows: 2,
            cols: 2,
        },
        CellRange {
            row: 5,
            col: 4,
            rows: 2,
            cols: 2,
        },
        CellRange {
            row: 13,
            col: 5,
            rows: 3,
            cols: 1,
        },
        CellRange {
            row: 5,
            col: 7,
            rows: 1,
            cols: 3,
        },
        CellRange {
            row: 5,
            col: 5,
            rows: 0,
            cols: 1,
        },
        CellRange {
            row: 5,
            col: 5,
            rows: 1,
            cols: 0,
        },
        CellRange {
            row: 5,
            col: 5,
            rows: u32::MAX,
            cols: 1,
        },
    ] {
        let mut d = data(vec![cell(5, 5)]);
        d.merges = vec![m];
        assert_eq!(check_cells(&ask(), &d, &L), Err(Refused::MergeOutsideRange), "{m:?}");
    }
}

#[test]
fn cells_sizes_that_do_not_match_the_range_are_refused() {
    for (cols, rows) in [(3, 10), (5, 10), (4, 9), (4, 11), (0, 0)] {
        let mut d = data(Vec::new());
        d.col_widths = vec![48.0; cols];
        d.row_heights = vec![15.0; rows];
        assert_eq!(
            check_cells(&ask(), &d, &L),
            Err(Refused::SizesMismatch),
            "{cols} widths, {rows} heights"
        );
    }
}

#[test]
fn cells_bad_sizes_are_refused() {
    for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1.0] {
        let mut d = data(Vec::new());
        d.col_widths[2] = bad;
        assert_eq!(check_cells(&ask(), &d, &L), Err(Refused::BadCellSize), "width {bad}");
        let mut d = data(Vec::new());
        d.row_heights[9] = bad;
        assert_eq!(check_cells(&ask(), &d, &L), Err(Refused::BadCellSize), "height {bad}");
    }
}

#[test]
fn cells_frozen_past_a_sheet_is_refused() {
    let mut d = data(Vec::new());
    d.frozen = Frozen {
        rows: MAX_SHEET_ROWS + 1,
        cols: 0,
    };
    assert_eq!(check_cells(&ask(), &d, &L), Err(Refused::FrozenTooLarge));
    d.frozen = Frozen {
        rows: 0,
        cols: MAX_SHEET_COLS + 1,
    };
    assert_eq!(check_cells(&ask(), &d, &L), Err(Refused::FrozenTooLarge));
}

fn entry(text_len: usize, page: u32) -> OutlineEntry {
    OutlineEntry {
        level: 1,
        text: "h".repeat(text_len),
        page,
    }
}

#[test]
fn outline_exact_is_accepted() {
    let o = OutlineData {
        entries: vec![entry(0, 0), entry(MAX_OUTLINE_TEXT_BYTES, 2)],
    };
    assert_eq!(check_outline(&o, 3, &L), Ok(()));
    let most = OutlineData {
        entries: vec![entry(10, 1999); MAX_OUTLINE_ENTRIES],
    };
    assert_eq!(check_outline(&most, 2000, &L), Ok(()));
    assert_eq!(check_outline(&OutlineData { entries: Vec::new() }, 0, &L), Ok(()));
}

#[test]
fn outline_page_past_the_end_is_refused() {
    let at_end = OutlineData {
        entries: vec![entry(5, 0), entry(5, 3)],
    };
    assert_eq!(check_outline(&at_end, 3, &L), Err(Refused::PagePastEnd));
    let far = OutlineData {
        entries: vec![entry(5, u32::MAX)],
    };
    assert_eq!(check_outline(&far, 3, &L), Err(Refused::PagePastEnd));
    let no_pages = OutlineData {
        entries: vec![entry(5, 0)],
    };
    assert_eq!(check_outline(&no_pages, 0, &L), Err(Refused::PagePastEnd));
}

#[test]
fn outline_with_too_many_entries_is_refused() {
    let many = OutlineData {
        entries: vec![entry(1, 0); MAX_OUTLINE_ENTRIES + 1],
    };
    assert_eq!(check_outline(&many, 1, &L), Err(Refused::TooManyEntries));
}

#[test]
fn outline_text_over_the_cap_is_refused() {
    let long = OutlineData {
        entries: vec![entry(4, 0), entry(MAX_OUTLINE_TEXT_BYTES + 1, 0)],
    };
    assert_eq!(check_outline(&long, 1, &L), Err(Refused::TextTooLong));
}

#[test]
fn answer_to_an_unsent_id_is_refused() {
    let sent: BTreeSet<u32> = [0, 2, 4].into_iter().collect();
    for id in [0, 2, 4] {
        assert_eq!(check_answer_id(id, |id| sent.contains(&id)), Ok(()), "id {id}");
    }
    for id in [6, 8, 1000, u32::MAX - 1] {
        assert_eq!(
            check_answer_id(id, |id| sent.contains(&id)),
            Err(Refused::UnsentId),
            "id {id}"
        );
    }
    // An odd id is a request the helper opened, never an answer, even if the app's set somehow holds it.
    let odd: BTreeSet<u32> = [1, 3].into_iter().collect();
    for id in [1, 3, 5, u32::MAX] {
        assert_eq!(
            check_answer_id(id, |id| odd.contains(&id)),
            Err(Refused::NotAnAnswer),
            "id {id}"
        );
    }
}

#[test]
fn limits_are_the_contracts() {
    // The viewer contract §5.2, one assert per number.
    assert_eq!(MAX_INPUT_BYTES, 104_857_600, "input file: 100 MB");
    assert_eq!(MAX_UNPACKED_BYTES, 536_870_912, "unpacked container: 512 MB");
    assert_eq!(MAX_ENTRIES, 10_000, "container entries: 10,000");
    assert_eq!(MAX_PAGES, 2_000, "pages or slides: 2,000");
    assert_eq!(MAX_SHEETS, 256, "sheets: 256");
    assert_eq!(MAX_CELLS, 2_000_000, "non-empty cells per workbook: 2,000,000");
    assert_eq!(MAX_IMAGE_SIDE_PX, 16_384, "decoded image: 16,384 px a side");
    assert_eq!(MAX_IMAGE_PIXELS, 67_108_864, "decoded image: 64 megapixels");
    assert_eq!(MAX_SIDE_PX, 4_096, "rendered page or tile: 4,096 px a side");
    assert_eq!(
        u64::from(MAX_SIDE_PX) * u64::from(MAX_SIDE_PX) * 4,
        64 * 1024 * 1024,
        "64 MB of pixels"
    );
    assert_eq!(HELPER_MEMORY_BYTES, 1_073_741_824, "helper memory: 1 GB committed");
    assert_eq!(OPEN_TIMEOUT, Duration::from_secs(30), "open: 30 s wall clock");
    assert_eq!(REQUEST_TIMEOUT, Duration::from_secs(15), "one request: 15 s wall clock");

    // The caps this crate adds, and the one value of `Limits` the app uses.
    assert_eq!(MAX_CELL_TEXT_BYTES, 32_768);
    assert_eq!(MAX_OUTLINE_ENTRIES, 10_000);
    assert_eq!(MAX_OUTLINE_TEXT_BYTES, 1_024);
    assert_eq!(MAX_SHEET_ROWS, 1_048_576);
    assert_eq!(MAX_SHEET_COLS, 16_384);
    assert_eq!(MAX_SHEET_NAME_BYTES, 256);
    assert_eq!(
        Limits::CONTRACT,
        Limits {
            max_side_px: 4_096,
            max_pages: 2_000,
            max_sheets: 256,
            max_sheet_rows: 1_048_576,
            max_sheet_cols: 16_384,
            max_sheet_name_bytes: 256,
            max_cell_text_bytes: 32_768,
            max_outline_entries: 10_000,
            max_outline_text_bytes: 1_024,
        }
    );
    assert_eq!(Limits::default(), Limits::CONTRACT);
}

#[test]
fn refused_reasons_are_fixed_and_distinct() {
    let all = [
        Refused::BadAsk,
        Refused::WrongSize,
        Refused::TooLarge,
        Refused::LengthMismatch,
        Refused::TooManyPages,
        Refused::PageCountMismatch,
        Refused::BadPageSize,
        Refused::TooManySheets,
        Refused::SheetTooLarge,
        Refused::NameTooLong,
        Refused::TooManyCells,
        Refused::CellOutsideRange,
        Refused::TextTooLong,
        Refused::MergeOutsideRange,
        Refused::SizesMismatch,
        Refused::BadCellSize,
        Refused::FrozenTooLarge,
        Refused::TooManyEntries,
        Refused::PagePastEnd,
        Refused::NotAnAnswer,
        Refused::UnsentId,
    ];
    let reasons: BTreeSet<&str> = all.iter().map(|r| r.reason()).collect();
    assert_eq!(reasons.len(), all.len(), "every reason is its own");
    for r in all {
        assert_eq!(r.to_string(), r.reason());
    }
}
