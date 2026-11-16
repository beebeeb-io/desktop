//! The checks W0 added: a cell's value and formula, the charts and pictures of a block, and their rasters. Each check
//! refuses every violation it names and accepts the exact answer.

use super::*;
use crate::office::{Align, Anchor, AnchorKind, CellData, CellRange, CellText, Cells, Frozen, GrantRef, RenderAnchor};

const L: Limits = Limits::CONTRACT;
/// 100 % zoom on a 96 DPI monitor, as the shared vector's 4/3.
const S100: f32 = 4.0 / 3.0;

fn ask() -> Cells {
    Cells {
        sheet: 0,
        range: CellRange {
            row: 0,
            col: 0,
            rows: 4,
            cols: 3,
        },
    }
}

fn cell(formula: Option<String>, value: Option<f64>) -> CellText {
    CellText {
        row: 1,
        col: 1,
        text: "13,220".to_owned(),
        align: Align::Right,
        bold: false,
        value,
        formula,
    }
}

fn block(cells: Vec<CellText>, anchors: Vec<Anchor>) -> CellData {
    CellData {
        cells,
        col_widths: vec![48.0; 3],
        row_heights: vec![15.0; 4],
        merges: Vec::new(),
        frozen: Frozen { rows: 0, cols: 0 },
        anchors,
        missing_content: false,
    }
}

fn anchor(id: u32) -> Anchor {
    Anchor {
        id,
        kind: AnchorKind::Chart,
        from_row: 2,
        from_col: 1,
        to_row: 20,
        to_col: 8,
        x_pt: 3.0,
        y_pt: 0.0,
        w_pt: 480.0,
        h_pt: 288.0,
    }
}

#[test]
fn a_formula_over_the_cap_is_refused() {
    let at_cap = "=".to_owned() + &"A".repeat(MAX_FORMULA_BYTES - 1);
    assert_eq!(at_cap.len(), MAX_FORMULA_BYTES);
    assert_eq!(
        check_cells(&ask(), &block(vec![cell(Some(at_cap.clone()), Some(1.0))], vec![]), &L),
        Ok(())
    );
    let over = at_cap + "A";
    assert_eq!(
        check_cells(&ask(), &block(vec![cell(Some(over), Some(1.0))], vec![]), &L),
        Err(Refused::FormulaTooLong)
    );
    // Counted in bytes, not characters: 2,731 three-byte characters are 8,193 bytes.
    let wide = "€".repeat(2_731);
    assert_eq!(wide.len(), MAX_FORMULA_BYTES + 1);
    assert_eq!(
        check_cells(&ask(), &block(vec![cell(Some(wide), None)], vec![]), &L),
        Err(Refused::FormulaTooLong)
    );
    // A tighter limit is held, not the constant.
    let tight = Limits {
        max_formula_bytes: 4,
        ..L
    };
    assert_eq!(
        check_cells(
            &ask(),
            &block(vec![cell(Some("=A1+1".to_owned()), None)], vec![]),
            &tight
        ),
        Err(Refused::FormulaTooLong)
    );
}

#[test]
fn cells_with_a_value_that_is_not_finite_are_refused() {
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert_eq!(
            check_cells(&ask(), &block(vec![cell(None, Some(bad))], vec![]), &L),
            Err(Refused::BadValue),
            "{bad}"
        );
    }
    for good in [0.0, -0.0, f64::MAX, f64::MIN, f64::MIN_POSITIVE] {
        assert_eq!(
            check_cells(&ask(), &block(vec![cell(None, Some(good))], vec![]), &L),
            Ok(()),
            "{good}"
        );
    }
}

#[test]
fn a_bad_anchor_is_refused() {
    // Every anchor the check takes: one of each kind, a one-cell one at the sheet's last cell, sizes at the cap.
    let mut last = anchor(1);
    last.kind = AnchorKind::Picture;
    (last.from_row, last.from_col) = (MAX_SHEET_ROWS - 1, MAX_SHEET_COLS - 1);
    (last.to_row, last.to_col) = (MAX_SHEET_ROWS - 1, MAX_SHEET_COLS - 1);
    let mut capped = anchor(2);
    (capped.w_pt, capped.h_pt) = (MAX_ANCHOR_SIDE_PT, MAX_ANCHOR_SIDE_PT);
    (capped.x_pt, capped.y_pt) = (MAX_ANCHOR_SIDE_PT, 0.0);
    let good = vec![anchor(0), last, capped];
    assert_eq!(check_cells(&ask(), &block(vec![], good.clone()), &L), Ok(()));

    let bad = |name: &str, change: &dyn Fn(&mut Anchor)| {
        let mut a = anchor(9);
        change(&mut a);
        assert!(!anchor_placeable(&a, &L), "{name}");
        let mut anchors = good.clone();
        anchors.push(a);
        assert_eq!(
            check_cells(&ask(), &block(vec![], anchors), &L),
            Err(Refused::BadAnchor),
            "{name}"
        );
    };
    bad("from below to", &|a| a.from_row = a.to_row + 1);
    bad("from right of to", &|a| a.from_col = a.to_col + 1);
    bad("past the last row", &|a| a.to_row = MAX_SHEET_ROWS);
    bad("past the last column", &|a| a.to_col = MAX_SHEET_COLS);
    bad("no width", &|a| a.w_pt = 0.0);
    bad("negative width", &|a| a.w_pt = -1.0);
    bad("NaN width", &|a| a.w_pt = f32::NAN);
    bad("infinite height", &|a| a.h_pt = f32::INFINITY);
    bad("no height", &|a| a.h_pt = 0.0);
    bad("width over the cap", &|a| a.w_pt = MAX_ANCHOR_SIDE_PT * 1.5);
    bad("height just over the cap", &|a| a.h_pt = 20_000.002);
    bad("negative x", &|a| a.x_pt = -0.5);
    bad("NaN y", &|a| a.y_pt = f32::NAN);
    bad("x past the cap", &|a| a.x_pt = 1e30);

    // Two with one id.
    let mut twice = good.clone();
    twice.push(anchor(0));
    assert_eq!(
        check_cells(&ask(), &block(vec![], twice), &L),
        Err(Refused::DuplicateAnchor)
    );

    // 256 is the limit; 257 are refused.
    let many = |n: u32| (0..n).map(anchor).collect::<Vec<_>>();
    assert_eq!(check_cells(&ask(), &block(vec![], many(256)), &L), Ok(()));
    assert_eq!(
        check_cells(&ask(), &block(vec![], many(257)), &L),
        Err(Refused::TooManyAnchors)
    );
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

fn draw(anchor: u32, scale: f32) -> RenderAnchor {
    RenderAnchor {
        sheet: 0,
        anchor,
        scale,
        out: GrantRef { wire: 8, len: 0 },
    }
}

#[test]
fn a_rendered_anchor_must_equal_the_ask_exactly() {
    // 480 x 288 pt at 4/3: 640 x 384 px.
    let chart = anchor(5);
    let ask = draw(5, S100);
    assert_eq!(anchor_pixels(&chart, S100), Some((640, 384)));
    assert_eq!(asked_anchor_size(&ask, &chart, &L), Ok((640, 384)));
    assert_eq!(
        check_rendered_anchor(&ask, &chart, len(640, 384), &rendered(640, 384), &L),
        Ok(())
    );
    // One pixel off on either side, the section length agreeing with the wrong size.
    for (w, h) in [(641, 384), (639, 384), (640, 385), (640, 383), (384, 640)] {
        assert_eq!(
            check_rendered_anchor(&ask, &chart, len(w, h), &rendered(w, h), &L),
            Err(Refused::WrongSize),
            "{w} x {h}"
        );
    }
    // Nothing, and a section that is not the raster.
    assert_eq!(
        check_rendered_anchor(&ask, &chart, 0, &rendered(0, 384), &L),
        Err(Refused::WrongSize)
    );
    assert_eq!(
        check_rendered_anchor(&ask, &chart, len(640, 384) + 4, &rendered(640, 384), &L),
        Err(Refused::LengthMismatch)
    );
    // An ask for another anchor than the one given, or at a scale that sizes nothing.
    assert_eq!(asked_anchor_size(&draw(6, S100), &chart, &L), Err(Refused::BadAsk));
    for scale in [0.0, -1.0, f32::NAN, f32::INFINITY, f32::MAX] {
        assert_eq!(
            asked_anchor_size(&draw(5, scale), &chart, &L),
            Err(Refused::BadAsk),
            "{scale}"
        );
        assert_eq!(
            check_rendered_anchor(&draw(5, scale), &chart, 4, &rendered(1, 1), &L),
            Err(Refused::BadAsk),
            "{scale}"
        );
    }
    // Over the side limit: the ask itself, and an answer one pixel over it.
    assert_eq!(asked_anchor_size(&draw(5, 9.0), &chart, &L), Err(Refused::TooLarge));
    let mut wide = anchor(5);
    wide.w_pt = 3072.0;
    assert_eq!(asked_anchor_size(&ask, &wide, &L), Ok((4096, 384)));
    assert_eq!(
        check_rendered_anchor(&ask, &wide, len(4097, 384), &rendered(4097, 384), &L),
        Err(Refused::TooLarge)
    );
}

/// The page-pixel vector, the same file in the Office engine's tests.
const PAGE_PIXELS_VECTORS: &str = include_str!("../../tests/page_pixels_vectors.txt");

fn f32_from_bits(field: &str) -> f32 {
    let hex = field.strip_prefix("0x").expect("bits start with 0x");
    f32::from_bits(u32::from_str_radix(hex, 16).expect("bits are hex"))
}

/// An anchor sized like a page: the shared vector's rows hold for anchors too, with the product in `f32`.
#[test]
fn anchor_pixels_follow_the_shared_vector() {
    // (pt, scale) -> px from the vector.
    let mut rows = Vec::new();
    for line in PAGE_PIXELS_VECTORS.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let f: Vec<&str> = line.split_whitespace().collect();
        assert_eq!(f.len(), 6, "{line}");
        rows.push((
            f32_from_bits(f[2]),
            f32_from_bits(f[4]),
            f[5].parse::<u32>().expect("px"),
        ));
    }
    assert_eq!(rows.len(), 20, "every row of the vector was read");

    let sized = |w_pt: f32, h_pt: f32| Anchor {
        w_pt,
        h_pt,
        ..anchor(0)
    };
    let mut checked = 0;
    // A 100 pt side at the vector's scales, worked out by hand: 4/3 is 1.33333337 in f32, so 133.333337 rounds up.
    let band = |scale: f32| match scale.to_bits() {
        0x3f80_0000 => 100,
        0x3fa0_0000 => 125,
        0x3fc0_0000 => 150,
        0x4000_0000 => 200,
        0x3faa_aaab => 134,
        other => panic!("a scale the vector did not have: {other:#x}"),
    };
    for &(pt, scale, px) in &rows {
        // Three anchor shapes per row: square, and the row's side as the width or the height of a 100 pt band.
        let band = band(scale);
        assert_eq!(anchor_pixels(&sized(pt, pt), scale), Some((px, px)), "{pt} at {scale}");
        assert_eq!(
            anchor_pixels(&sized(pt, 100.0), scale),
            Some((px, band)),
            "{pt} wide at {scale}"
        );
        assert_eq!(
            anchor_pixels(&sized(100.0, pt), scale),
            Some((band, px)),
            "{pt} high at {scale}"
        );
        checked += 3;
    }
    assert_eq!(checked, 60);

    // No page clamp: 2,000,000 pt at 0.5 is 1,000,000 px; a page of that height is clamped to 1,000,000 pt first.
    assert_eq!(anchor_pixels(&sized(10.0, 2e6), 0.5), Some((5, 1_000_000)));
    let page = PageSize { w_pt: 10.0, h_pt: 2e6 };
    assert_eq!(page_pixels(&page, 0.5), Some((5, 500_000)));
    // The engine's floor of 1 px, and None where a page is None.
    assert_eq!(anchor_pixels(&sized(1e-30, 1e-30), 1e-30), Some((1, 1)));
    assert_eq!(anchor_pixels(&sized(480.0, 288.0), 1e7), None);
    assert_eq!(anchor_pixels(&sized(0.0, 288.0), 1.0), None);
    assert_eq!(anchor_pixels(&sized(480.0, f32::NAN), 1.0), None);
}

/// `halve_range` refuses `RangeTooLarge` for one cell, because one cell always fits a frame: its text, its formula,
/// its value, one merge and the block's charts and pictures, every one at its cap.
#[test]
fn one_cell_at_every_cap_fits_one_frame() {
    let one = Cells {
        sheet: 0,
        range: CellRange {
            row: 7,
            col: 7,
            rows: 1,
            cols: 1,
        },
    };
    let fullest = CellData {
        cells: vec![CellText {
            row: 7,
            col: 7,
            text: "x".repeat(MAX_CELL_TEXT_BYTES),
            align: Align::Left,
            bold: true,
            value: Some(f64::MAX),
            formula: Some("=".repeat(MAX_FORMULA_BYTES)),
        }],
        col_widths: vec![48.0],
        row_heights: vec![15.0],
        merges: vec![one.range],
        frozen: Frozen { rows: 1, cols: 1 },
        anchors: (0..MAX_ANCHORS as u32).map(anchor).collect(),
        missing_content: true,
    };
    assert_eq!(check_cells(&one, &fullest, &L), Ok(()));
    let payload = fullest.encode().expect("one cell at every cap fits one frame");
    assert!(payload.len() <= crate::frame::MAX_PAYLOAD, "{} bytes", payload.len());
    // With room to spare: under 52 KiB of the 64 KiB frame.
    assert!(payload.len() < 52 * 1024, "{} bytes", payload.len());
    assert_eq!(halve_range(&one), Err(Refused::CannotHalve));
}
