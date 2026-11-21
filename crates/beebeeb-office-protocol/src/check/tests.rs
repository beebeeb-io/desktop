//! Each check refuses every violation it names, one violation at a time, and accepts the exact ask.

use std::collections::BTreeSet;
use std::time::Duration;

use super::*;
use crate::office::{Align, CellRange, CellText, Frozen, GrantRef, OutlineEntry, Rect, SheetInfo};

const OUT: GrantRef = GrantRef { wire: 9, len: 0 };
/// 100 % zoom on a 96 DPI monitor: 4/3 device pixels per point (`0x3faaaaab` as f32, the shared vector's 4/3).
const S100: f32 = 4.0 / 3.0;
/// A4 as a document states it, in points: 794 x 1123 px at [`S100`].
const A4: PageSize = PageSize {
    w_pt: 595.3,
    h_pt: 841.9,
};
/// 4,267 x 134 px at [`S100`]: wider than one tile.
const WIDE: PageSize = PageSize {
    w_pt: 3200.0,
    h_pt: 100.0,
};
/// 134 x 4,267 px at [`S100`]: taller than one tile.
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

/// A tile of the page at [`S100`].
fn tile(x: u32, y: u32, w: u32, h: u32) -> Render {
    Render {
        page: 0,
        scale: S100,
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
    assert_eq!(page_pixels(&A4, S100), Some((794, 1123)));
    assert_eq!(page_pixels(&A4, 2.0 * S100), Some((1588, 2246)));

    assert_eq!(
        check_rendered(&whole(S100), &A4, len(794, 1123), &rendered(794, 1123), &L),
        Ok(())
    );
    assert_eq!(
        check_rendered(&whole(2.0 * S100), &A4, len(1588, 2246), &rendered(1588, 2246), &L),
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
    // Only the width differs, and the section length agrees with the wrong size: the width comparison alone must
    // refuse it.
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
        check_rendered(&whole(S100), &A4, len(1123, 794), &rendered(1123, 794), &L),
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
    // One pixel over the limit, and the section length agrees: the limit is held before the size is compared, so it
    // refuses as over the limit.
    let ask = tile(0, 0, 4096, 10);
    assert_eq!(
        check_rendered(&ask, &WIDE, len(4097, 10), &rendered(4097, 10), &L),
        Err(Refused::TooLarge)
    );
    let ask = Render {
        page: 0,
        scale: S100,
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
    assert_eq!(asked_size(&whole(S100), &WIDE, &L), Err(Refused::TooLarge));
    assert_eq!(asked_size(&whole(6.0), &A4, &L), Err(Refused::TooLarge));
    assert_eq!(asked_size(&tile(0, 0, 4097, 10), &WIDE, &L), Err(Refused::TooLarge));
    assert_eq!(
        check_rendered(&whole(S100), &WIDE, len(4267, 134), &rendered(4267, 134), &L),
        Err(Refused::TooLarge)
    );
}

/// US Letter, in points: 816 x 1056 px at [`S100`] in single precision (817 x 1057 in double).
const LETTER: PageSize = PageSize {
    w_pt: 612.0,
    h_pt: 792.0,
};

/// F7: there is no rounding slack. The app and the helper size a page by one rule, so a side one pixel off is a fault,
/// even when the section's length agrees with it.
#[test]
fn a_rendered_page_must_equal_the_ask_exactly() {
    // A 3 x 2 tile answered 2 x 3: the same 24 bytes, each side one pixel off.
    let ask = tile(0, 0, 3, 2);
    assert_eq!(
        len(3, 2),
        len(2, 3),
        "the premise: the section length cannot tell them apart"
    );
    assert_eq!(
        check_rendered(&ask, &A4, len(2, 3), &rendered(2, 3), &L),
        Err(Refused::WrongSize)
    );
    assert_eq!(check_rendered(&ask, &A4, len(3, 2), &rendered(3, 2), &L), Ok(()));

    // A whole Letter page at 100 %: 816 x 1056 is the ask; one pixel more or less on either side is refused.
    assert_eq!(page_pixels(&LETTER, S100), Some((816, 1056)));
    for (w, h) in [(817, 1056), (815, 1056), (816, 1057), (816, 1055), (817, 1057)] {
        assert_eq!(
            check_rendered(&whole(S100), &LETTER, len(w, h), &rendered(w, h), &L),
            Err(Refused::WrongSize),
            "{w} x {h}"
        );
    }
    assert_eq!(
        check_rendered(&whole(S100), &LETTER, len(816, 1056), &rendered(816, 1056), &L),
        Ok(())
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
    // f32::MAX: the side overflows to infinity. (A tiny scale is not here: it sizes 1 px, as the engine does.)
    for scale in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 0.0, -0.0, -1.0, f32::MAX] {
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

/// The page-pixel vector, the same file in the Office engine's tests.
const PAGE_PIXELS_VECTORS: &str = include_str!("../../tests/page_pixels_vectors.txt");

/// A `0x`-prefixed f32 bit pattern from the vector.
fn f32_from_bits(field: &str) -> f32 {
    let hex = field.strip_prefix("0x").expect("bits start with 0x");
    f32::from_bits(u32::from_str_radix(hex, 16).expect("bits are hex"))
}

#[test]
fn page_pixels_follow_the_shared_vector() {
    let mut rows = 0;
    for line in PAGE_PIXELS_VECTORS.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.split_whitespace().collect();
        assert_eq!(fields.len(), 6, "label, pt, bits, scale, bits, px: {line}");
        let pt = f32_from_bits(fields[2]);
        let scale = f32_from_bits(fields[4]);
        // The decimals people read are the values the bits hold.
        let pt_dec: f32 = fields[1].parse().expect("pt is a number");
        let scale_dec: f32 = fields[3].parse().expect("scale is a number");
        assert_eq!(pt_dec.to_bits(), pt.to_bits(), "pt: {line}");
        assert_eq!(scale_dec.to_bits(), scale.to_bits(), "scale: {line}");
        let px: u32 = fields[5].parse().expect("px is a whole number");
        // The row's side as both sides of a square page, so each row checks the width and the height rule.
        let square = PageSize { w_pt: pt, h_pt: pt };
        assert_eq!(page_pixels(&square, scale), Some((px, px)), "{line}");
        rows += 1;
    }
    assert_eq!(rows, 20, "every row of the vector was read");
    // The vector's 4/3 is this file's 100 %.
    assert_eq!(S100.to_bits(), 0x3faa_aaab);
}

#[test]
fn page_pixels_keep_the_engines_floor_and_height_clamp() {
    // A product that underflows to 0 in f32 still sizes 1 px, as the engine's floor of 1.
    let tiny = PageSize {
        w_pt: 1e-30,
        h_pt: 1e-30,
    };
    assert_eq!(page_pixels(&tiny, 1e-30), Some((1, 1)));
    // Any positive product rounds up, so a very small scale sizes 1 px instead of being refused.
    assert_eq!(page_pixels(&A4, 1e-9), Some((1, 1)));
    // A page taller than 1,000,000 pt is sized at that height, as the engine clamps it: 2,001 px at 0.002, not 4,001.
    let tall = PageSize { w_pt: 10.0, h_pt: 2e6 };
    assert_eq!(page_pixels(&tall, 0.002), Some((1, 2001)));
    // A side of 2^32 px or more does not fit a u32: refused, never saturated.
    assert_eq!(page_pixels(&A4, 1e7), None);
    assert_eq!(page_pixels(&A4, f32::MAX), None);
}

#[test]
fn render_scale_is_zoom_times_dpi_over_72() {
    assert_eq!(
        render_scale(1.0, 96.0).to_bits(),
        S100.to_bits(),
        "100 % at 96 DPI is 4/3"
    );
    assert_eq!(render_scale(1.5, 144.0).to_bits(), 3.0f32.to_bits(), "150 % at 144 DPI");
    assert_eq!(render_scale(1.0, 72.0).to_bits(), 1.0f32.to_bits(), "100 % at 72 DPI");
    // 100 % at 96 DPI sizes A4 at 794 x 1123 px.
    assert_eq!(page_pixels(&A4, render_scale(1.0, 96.0)), Some((794, 1123)));
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

fn named(sheets: Vec<SheetInfo>) -> OpenedSheets {
    OpenedSheets { sheets, more: 0 }
}

#[test]
fn sheets_exact_is_accepted() {
    let most = named(vec![sheet(255, MAX_SHEET_ROWS, MAX_SHEET_COLS); 256]);
    assert_eq!(check_sheets(&most, &L), Ok(()));
    assert_eq!(check_sheets(&named(Vec::new()), &L), Ok(()));
    assert_eq!(check_sheets(&named(vec![sheet(0, 0, 0)]), &L), Ok(()));
    // 255 characters of three bytes each (765 bytes): the cap counts characters, not bytes.
    let euros = named(vec![SheetInfo {
        name: "€".repeat(255),
        rows: 1,
        cols: 1,
    }]);
    assert_eq!(check_sheets(&euros, &L), Ok(()));
    // The sheets named here and those counted in `more`: 256 together.
    let split = OpenedSheets {
        sheets: vec![sheet(6, 10, 10); 200],
        more: 56,
    };
    assert_eq!(check_sheets(&split, &L), Ok(()));
    let unnamed = OpenedSheets {
        sheets: Vec::new(),
        more: 256,
    };
    assert_eq!(check_sheets(&unnamed, &L), Ok(()));
}

#[test]
fn sheets_over_256_are_refused() {
    let many = named(vec![sheet(6, 10, 10); 257]);
    assert_eq!(check_sheets(&many, &L), Err(Refused::TooManySheets));
}

#[test]
fn sheets_counted_in_more_are_held_to_256() {
    // One over, however the count is split between named sheets and `more`.
    for (named_count, more) in [(200usize, 57u32), (0, 257), (256, 1), (1, u32::MAX)] {
        let got = OpenedSheets {
            sheets: vec![sheet(6, 10, 10); named_count],
            more,
        };
        assert_eq!(
            check_sheets(&got, &L),
            Err(Refused::TooManySheets),
            "{named_count} named, {more} more"
        );
    }
}

#[test]
fn sheets_with_a_long_name_are_refused() {
    let long = named(vec![sheet(6, 10, 10), sheet(256, 10, 10)]);
    assert_eq!(check_sheets(&long, &L), Err(Refused::NameTooLong));
    let euros = named(vec![SheetInfo {
        name: "€".repeat(256),
        rows: 1,
        cols: 1,
    }]);
    assert_eq!(check_sheets(&euros, &L), Err(Refused::NameTooLong));
}

#[test]
fn sheets_larger_than_the_format_are_refused() {
    let rows = named(vec![sheet(6, MAX_SHEET_ROWS + 1, 10)]);
    assert_eq!(check_sheets(&rows, &L), Err(Refused::SheetTooLarge));
    let cols = named(vec![sheet(6, 10, MAX_SHEET_COLS + 1)]);
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
        value: Some(1.25),
        formula: None,
    }
}

fn data(cells: Vec<CellText>) -> CellData {
    CellData {
        cells,
        col_widths: vec![48.0; 4],
        row_heights: vec![15.0; 10],
        merges: Vec::new(),
        frozen: Frozen { rows: 0, cols: 0 },
        anchors: Vec::new(),
        missing_content: false,
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
    // Frozen panes up to the range's end (after row 15 and column 9).
    d.frozen = Frozen { rows: 15, cols: 9 };
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

#[test]
fn cells_frozen_past_the_asked_range_is_refused() {
    // The range ends after row 15 and column 9 (rows 5 to 14, columns 5 to 8): panes up to there are the block's to
    // say; past it, a crafted workbook's sheet-sized panes reach the view.
    let mut d = data(Vec::new());
    for (rows, cols) in [(15, 9), (15, 0), (0, 9), (1, 1)] {
        d.frozen = Frozen { rows, cols };
        assert_eq!(check_cells(&ask(), &d, &L), Ok(()), "{rows} x {cols}");
    }
    for (rows, cols) in [(16, 0), (0, 10), (16, 10), (MAX_SHEET_ROWS, MAX_SHEET_COLS)] {
        d.frozen = Frozen { rows, cols };
        assert_eq!(
            check_cells(&ask(), &d, &L),
            Err(Refused::FrozenTooLarge),
            "{rows} x {cols}"
        );
    }
}

fn range(row: u32, col: u32, rows: u32, cols: u32) -> CellRange {
    CellRange { row, col, rows, cols }
}

#[test]
fn cells_range_too_large_is_halved() {
    // 10 rows x 4 columns: split across the rows, 5 and 5, on the same sheet.
    let [a, b] = halve_range(&ask()).expect("40 cells halve");
    assert_eq!(a.range, range(5, 5, 5, 4));
    assert_eq!(b.range, range(10, 5, 5, 4));
    assert_eq!((a.sheet, b.sheet), (1, 1));
    // 1 row x 7 columns: across the columns, the first half the smaller.
    let wide = Cells {
        sheet: 0,
        range: range(2, 0, 1, 7),
    };
    let [a, b] = halve_range(&wide).expect("7 cells halve");
    assert_eq!(a.range, range(2, 0, 1, 3));
    assert_eq!(b.range, range(2, 3, 1, 4));

    // Halving again and again ends in single cells, each cell of the range exactly once: 40 cells, 39 splits.
    let mut todo = vec![ask()];
    let mut seen = BTreeSet::new();
    let mut splits = 0;
    while let Some(c) = todo.pop() {
        match halve_range(&c) {
            Ok([a, b]) => {
                splits += 1;
                todo.push(a);
                todo.push(b);
            }
            Err(refused) => {
                assert_eq!(refused, Refused::CannotHalve);
                assert_eq!(c.range.area(), 1, "{:?}", c.range);
                assert!(seen.insert((c.range.row, c.range.col)), "{:?} twice", c.range);
            }
        }
    }
    assert_eq!(splits, 39);
    assert_eq!(seen.len(), 40);
    assert!(seen.iter().all(|&(r, c)| ask().range.contains(r, c)));
}

#[test]
fn cells_that_cannot_be_halved_are_refused() {
    for (rows, cols) in [(1, 1), (0, 0), (0, 9), (9, 0)] {
        let c = Cells {
            sheet: 0,
            range: range(3, 3, rows, cols),
        };
        assert_eq!(halve_range(&c), Err(Refused::CannotHalve), "{rows} x {cols}");
    }
    // A half past the last row a u32 can name is the app's own bad ask, never a wrap to row 0.
    let edge = Cells {
        sheet: 0,
        range: range(u32::MAX - 1, 0, 4, 1),
    };
    assert_eq!(halve_range(&edge), Err(Refused::BadAsk));

    // Why one cell never needs halving: one cell with text at the cap, its width, its height and a merge fits one
    // frame with room to spare.
    let one = Cells {
        sheet: 0,
        range: range(0, 0, 1, 1),
    };
    let mut d = data(vec![cell(0, 0)]);
    d.cells[0].text = "x".repeat(MAX_CELL_TEXT_BYTES);
    d.col_widths = vec![48.0];
    d.row_heights = vec![15.0];
    d.merges = vec![one.range];
    // The most frozen panes a one-cell range at A1 may carry (its end); their size on the wire is fixed whatever they are.
    d.frozen = Frozen { rows: 1, cols: 1 };
    assert_eq!(check_cells(&one, &d, &L), Ok(()));
    let payload = d.encode().expect("one cell fits one frame");
    // Counts 4, the cell 16 + its text + its value 8, widths 8, heights 8, merges 20, frozen 8, anchors 4,
    // `missing_content` 1: half a frame and 77 bytes. With a formula and charts at their caps too, see
    // `one_cell_at_every_cap_fits_one_frame`.
    assert_eq!(payload.len(), MAX_CELL_TEXT_BYTES + 77);
    assert!(payload.len() < crate::frame::MAX_PAYLOAD);
}

fn entry(text_len: usize, page: u32) -> OutlineEntry {
    OutlineEntry {
        level: 1,
        text: "h".repeat(text_len),
        page,
    }
}

/// The first page of the outline.
const FIRST: Outline = Outline { from: 0 };

/// A page with no more to come.
fn last(entries: Vec<OutlineEntry>) -> OutlineData {
    OutlineData { entries, next: None }
}

#[test]
fn outline_exact_is_accepted() {
    let o = last(vec![entry(0, 0), entry(MAX_OUTLINE_TEXT_BYTES, 2)]);
    assert_eq!(check_outline(&FIRST, &o, 3, &L), Ok(()));
    let most = last(vec![entry(10, 1999); MAX_OUTLINE_ENTRIES]);
    assert_eq!(check_outline(&FIRST, &most, 2000, &L), Ok(()));
    assert_eq!(check_outline(&FIRST, &last(Vec::new()), 0, &L), Ok(()));
}

#[test]
fn outline_pages_are_accepted_in_turn() {
    // The app's loop: ask from 0, then from each `next`, until it is `None`.
    let first = OutlineData {
        entries: vec![entry(5, 0), entry(5, 1)],
        next: Some(2),
    };
    assert_eq!(check_outline(&FIRST, &first, 3, &L), Ok(()));
    assert_eq!(
        check_outline(&Outline { from: 2 }, &last(vec![entry(5, 2)]), 3, &L),
        Ok(())
    );
    // The last page may end exactly at the entry limit, and the page before it may say there is one more.
    let before = OutlineData {
        entries: vec![entry(5, 0)],
        next: Some(9_999),
    };
    assert_eq!(check_outline(&Outline { from: 9_998 }, &before, 1, &L), Ok(()));
    assert_eq!(
        check_outline(&Outline { from: 9_999 }, &last(vec![entry(5, 0)]), 1, &L),
        Ok(())
    );
    // An ask past the end is answered with nothing more.
    assert_eq!(check_outline(&Outline { from: 40 }, &last(Vec::new()), 1, &L), Ok(()));
}

#[test]
fn outline_page_that_does_not_advance_is_refused() {
    let from = Outline { from: 4 };
    // No entries, yet more to come: the app would ask from 4 again for ever.
    let stuck = OutlineData {
        entries: Vec::new(),
        next: Some(4),
    };
    assert_eq!(check_outline(&from, &stuck, 10, &L), Err(Refused::BadNext));
    // Two entries from 4, so the next is 6: back to the start, the same page, one short, one past.
    for next in [0, 4, 5, 7, u32::MAX] {
        let page = OutlineData {
            entries: vec![entry(5, 0), entry(5, 1)],
            next: Some(next),
        };
        assert_eq!(
            check_outline(&from, &page, 10, &L),
            Err(Refused::BadNext),
            "next {next}"
        );
    }
}

#[test]
fn outline_page_past_the_end_is_refused() {
    let at_end = last(vec![entry(5, 0), entry(5, 3)]);
    assert_eq!(check_outline(&FIRST, &at_end, 3, &L), Err(Refused::PagePastEnd));
    let far = last(vec![entry(5, u32::MAX)]);
    assert_eq!(check_outline(&FIRST, &far, 3, &L), Err(Refused::PagePastEnd));
    let no_pages = last(vec![entry(5, 0)]);
    assert_eq!(check_outline(&FIRST, &no_pages, 0, &L), Err(Refused::PagePastEnd));
}

#[test]
fn outline_with_too_many_entries_is_refused() {
    let many = last(vec![entry(1, 0); MAX_OUTLINE_ENTRIES + 1]);
    assert_eq!(check_outline(&FIRST, &many, 1, &L), Err(Refused::TooManyEntries));
    // Over all the pages: two entries from 9,999 end past the limit, and so does any entry from u32::MAX.
    let two = last(vec![entry(1, 0), entry(1, 0)]);
    assert_eq!(
        check_outline(&Outline { from: 9_999 }, &two, 1, &L),
        Err(Refused::TooManyEntries)
    );
    let one = last(vec![entry(1, 0)]);
    assert_eq!(
        check_outline(&Outline { from: u32::MAX }, &one, 1, &L),
        Err(Refused::TooManyEntries)
    );
    // A page that ends at the limit and still says there is more.
    let more = OutlineData {
        entries: vec![entry(1, 0)],
        next: Some(10_000),
    };
    assert_eq!(
        check_outline(&Outline { from: 9_999 }, &more, 1, &L),
        Err(Refused::TooManyEntries)
    );
}

#[test]
fn outline_text_over_the_cap_is_refused() {
    let long = last(vec![entry(4, 0), entry(MAX_OUTLINE_TEXT_BYTES + 1, 0)]);
    assert_eq!(check_outline(&FIRST, &long, 1, &L), Err(Refused::TextTooLong));
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
    assert_eq!(MAX_SHEET_NAME_CHARS, 255);
    assert_eq!(MAX_FORMULA_BYTES, 8_192);
    assert_eq!(MAX_ANCHORS, 256);
    assert_eq!(MAX_ANCHOR_SIDE_PT.to_bits(), 20_000.0f32.to_bits());
    assert_eq!(
        Limits::CONTRACT,
        Limits {
            max_side_px: 4_096,
            max_pages: 2_000,
            max_sheets: 256,
            max_sheet_rows: 1_048_576,
            max_sheet_cols: 16_384,
            max_sheet_name_chars: 255,
            max_cell_text_bytes: 32_768,
            max_outline_entries: 10_000,
            max_outline_text_bytes: 1_024,
            max_formula_bytes: 8_192,
            max_anchors: 256,
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
        Refused::BadValue,
        Refused::FormulaTooLong,
        Refused::TooManyAnchors,
        Refused::BadAnchor,
        Refused::DuplicateAnchor,
        Refused::TooManyEntries,
        Refused::BadNext,
        Refused::PagePastEnd,
        Refused::CannotHalve,
        Refused::NotAnAnswer,
        Refused::UnsentId,
    ];
    let reasons: BTreeSet<&str> = all.iter().map(|r| r.reason()).collect();
    assert_eq!(reasons.len(), all.len(), "every reason is its own");
    for r in all {
        assert_eq!(r.to_string(), r.reason());
    }
}

// ---- The editor's checks ---------------------------------------------------------------------------------------

mod edit_checks {
    use super::*;
    use crate::office::{
        Clip, Dirty, EditKind, Edited, LayoutList, ListKind, Marks, ParaStyle, PageRect, RectPt, SaveReady, SavedInto,
        TextAlign,
    };

    const PAGES: [PageSize; 2] = [A4, WIDE];

    fn rect(page: u32, x: f32, y: f32, w: f32, h: f32) -> PageRect {
        PageRect {
            page,
            rect: RectPt { x, y, w, h },
        }
    }

    fn inside() -> PageRect {
        rect(0, 72.0, 100.0, 10.0, 14.0)
    }

    fn edited() -> Edited {
        Edited {
            revision: 3,
            caret: Some(inside()),
            selection: vec![inside(), rect(1, 0.0, 0.0, 3200.0, 100.0)],
            can_undo: true,
            can_redo: false,
            dirty: Dirty::Rects(vec![inside()]),
            pages: None,
            cells: vec![(
                0,
                CellRange {
                    row: 0,
                    col: 0,
                    rows: 2,
                    cols: 2,
                },
            )],
            slides: Some(3),
            marks: Marks {
                bold: false,
                italic: false,
                underline: false,
                strike: false,
                style: ParaStyle::Normal,
                align: TextAlign::Left,
                list: ListKind::None,
            },
        }
    }

    #[test]
    fn the_editor_numbers_are_fixed() {
        assert_eq!(MAX_SELECTION_RECTS, 1_024);
        assert_eq!(MAX_DIRTY_RECTS, 256);
        assert_eq!(MAX_OP_TEXT_BYTES, 32 * 1024);
        assert_eq!(MAX_SAVE_BYTES, 2 * MAX_INPUT_BYTES);
        // The widest legal Edited rects stay in one frame.
        assert!((MAX_SELECTION_RECTS + MAX_DIRTY_RECTS) * PageRect::BYTES < crate::frame::MAX_PAYLOAD / 2);
        let _ = EditKind::Docx;
    }

    #[test]
    fn edited_accepts_a_good_answer() {
        assert_eq!(check_edited(&edited(), &PAGES, &L), Ok(()));
        // A caret on the page's edge.
        let mut e = edited();
        e.caret = Some(rect(0, 595.3 - 1.0, 841.9 - 14.0, 1.0, 14.0));
        assert_eq!(check_edited(&e, &PAGES, &L), Ok(()));
    }

    #[test]
    fn edited_refuses_a_rect_outside_its_page() {
        type Make = fn(PageRect) -> Edited;
        let makers: [Make; 3] = [
            |r| Edited {
                caret: Some(r),
                ..edited()
            },
            |r| Edited {
                selection: vec![inside(), r],
                ..edited()
            },
            |r| Edited {
                dirty: Dirty::Rects(vec![inside(), r]),
                ..edited()
            },
        ];
        let bad = [
            rect(2, 0.0, 0.0, 1.0, 1.0),                   // a page the document does not have
            rect(0, 595.0, 0.0, 10.0, 1.0),                // past the right edge
            rect(0, 0.0, 841.0, 1.0, 10.0),                // past the bottom
            rect(0, -1.0, 0.0, 1.0, 1.0),                  // negative x
            rect(0, 0.0, -1.0, 1.0, 1.0),                  // negative y
            rect(0, 0.0, 0.0, -1.0, 1.0),                  // negative width
            rect(0, 0.0, 0.0, 1.0, -1.0),                  // negative height
            rect(0, f32::MAX, 0.0, f32::MAX, 1.0),         // overflowing sum
        ];
        for make in makers {
            for r in bad {
                assert_eq!(check_edited(&make(r), &PAGES, &L), Err(Refused::BadRect), "{r:?}");
            }
        }
        // Non-finite values cannot come off the wire, but the check refuses them too.
        let mut e = edited();
        e.caret = Some(rect(0, f32::NAN, 0.0, 1.0, 1.0));
        assert_eq!(check_edited(&e, &PAGES, &L), Err(Refused::BadRect));
    }

    #[test]
    fn edited_rects_are_held_to_the_new_page_sizes_when_it_brings_them() {
        let mut e = edited();
        e.pages = Some(Opened {
            pages: 1,
            sizes: vec![PageSize { w_pt: 50.0, h_pt: 50.0 }],
        });
        // The old rects (page 1, x 72) no longer fit.
        assert_eq!(check_edited(&e, &PAGES, &L), Err(Refused::BadRect));
        e.caret = Some(rect(0, 1.0, 1.0, 1.0, 1.0));
        e.selection = vec![rect(0, 0.0, 0.0, 50.0, 50.0)];
        e.dirty = Dirty::All;
        assert_eq!(check_edited(&e, &[], &L), Ok(()));
        // The new sizes are checked as an `Opened`.
        e.pages = Some(Opened {
            pages: 2,
            sizes: vec![PageSize { w_pt: 50.0, h_pt: 50.0 }],
        });
        assert_eq!(check_edited(&e, &PAGES, &L), Err(Refused::PageCountMismatch));
    }

    #[test]
    fn edited_refuses_too_many_rects() {
        let many = |n: usize| vec![inside(); n];
        let mut e = edited();
        e.selection = many(MAX_SELECTION_RECTS);
        assert_eq!(check_edited(&e, &PAGES, &L), Ok(()));
        e.selection = many(MAX_SELECTION_RECTS + 1);
        assert_eq!(check_edited(&e, &PAGES, &L), Err(Refused::TooManyRects));
        let mut e = edited();
        e.dirty = Dirty::Rects(many(MAX_DIRTY_RECTS));
        assert_eq!(check_edited(&e, &PAGES, &L), Ok(()));
        e.dirty = Dirty::Rects(many(MAX_DIRTY_RECTS + 1));
        assert_eq!(check_edited(&e, &PAGES, &L), Err(Refused::TooManyRects));
    }

    #[test]
    fn edited_refuses_bad_cells_and_slides() {
        let cell = |row, col, rows, cols| {
            vec![(
                0,
                CellRange { row, col, rows, cols },
            )]
        };
        let mut e = edited();
        for bad in [
            cell(0, 0, 0, 1),
            cell(0, 0, 1, 0),
            cell(MAX_SHEET_ROWS, 0, 1, 1),
            cell(0, MAX_SHEET_COLS, 1, 1),
            cell(u32::MAX, 0, 2, 1),
            cell(0, u32::MAX, 1, 2),
            vec![cell(0, 0, 1, 1)[0]; MAX_DIRTY_RECTS + 1],
        ] {
            e.cells = bad;
            assert_eq!(check_edited(&e, &PAGES, &L), Err(Refused::BadCells));
        }
        e.cells = cell(MAX_SHEET_ROWS - 1, MAX_SHEET_COLS - 1, 1, 1);
        assert_eq!(check_edited(&e, &PAGES, &L), Ok(()));
        e.slides = Some(MAX_PAGES);
        assert_eq!(check_edited(&e, &PAGES, &L), Ok(()));
        e.slides = Some(MAX_PAGES + 1);
        assert_eq!(check_edited(&e, &PAGES, &L), Err(Refused::TooManyPages));
    }

    #[test]
    fn save_ready_is_held_to_the_save_cap() {
        let ready = |len| SaveReady {
            token: 1,
            len,
            revision: 5,
        };
        assert_eq!(check_save_ready(&ready(0)), Ok(()));
        assert_eq!(check_save_ready(&ready(MAX_SAVE_BYTES)), Ok(()));
        assert_eq!(check_save_ready(&ready(MAX_SAVE_BYTES + 1)), Err(Refused::SaveTooLarge));
        assert_eq!(check_save_ready(&ready(u64::MAX)), Err(Refused::SaveTooLarge));
    }

    #[test]
    fn saved_into_must_equal_the_granted_length() {
        let got = |len| SavedInto { len };
        assert_eq!(check_saved_into(&got(4096), 4096), Ok(()));
        assert_eq!(check_saved_into(&got(0), 0), Ok(()));
        for wrong in [4095, 4097, 0, u64::MAX] {
            assert_eq!(check_saved_into(&got(wrong), 4096), Err(Refused::SavedLengthMismatch));
        }
    }

    #[test]
    fn layout_lists_are_capped() {
        let list = |n: usize, len: usize| LayoutList {
            names: vec!["n".repeat(len); n],
        };
        assert_eq!(check_layouts(&list(MAX_LAYOUTS, MAX_LAYOUT_NAME_BYTES)), Ok(()));
        assert_eq!(check_layouts(&list(MAX_LAYOUTS + 1, 1)), Err(Refused::TooManyLayouts));
        assert_eq!(
            check_layouts(&list(1, MAX_LAYOUT_NAME_BYTES + 1)),
            Err(Refused::TooManyLayouts)
        );
        let _ = Clip {
            text: String::new(),
            html: None,
        };
    }
}
