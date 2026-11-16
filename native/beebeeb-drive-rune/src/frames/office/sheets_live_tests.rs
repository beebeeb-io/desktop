//! Tests of the live workbook (V3-5): the cells asked, the scroll, the formula bar, the figures, the anchors and the
//! quiet line, drawn from a source that records what it is asked (`docs_live_tests`).

use rune::{Key, Mods};

use super::*;
use crate::frames::office::docs::live_tests::{find, live_runtime};
use crate::frames::viewers::source::Body;

fn cell(row: u32, col: u32, text: &str, value: Option<f64>) -> CellText {
    CellText {
        row,
        col,
        text: text.into(),
        right_aligned: value.is_some(),
        bold: false,
        value,
        formula: None,
    }
}

/// A big workbook: 400 rows by 30 columns of known size, nothing held but what a test puts in.
fn big() -> SheetView {
    let mut view = sheets_fixture::board();
    view.cells.clear();
    view.col_widths = vec![80.0; 30];
    view.row_heights = vec![24.0; 400];
    view.extent = (400, 30);
    view.selection = None;
    view.formula = String::new();
    view
}

fn sheet_in(rt: &mut rune::Runtime<State>) -> &mut SheetView {
    match &mut viewers::ui(&mut rt.state).expect("a file").file.body {
        Body::Sheet(v) => v,
        other => panic!("{other:?} is not a sheet"),
    }
}

/// The first row and column in view moved to `to` as a key moves them (the scroll viewports follow, one truth), drawn
/// once; only what that build asked is kept.
fn scroll_to(rt: &mut rune::Runtime<State>, log: &crate::frames::office::docs::live_tests::Log, to: Cell) {
    log.cells.borrow_mut().clear();
    log.pictures.borrow_mut().clear();
    rt.update(|s, tx| {
        if let Some(Body::Sheet(view)) = viewers::ui(s).map(|ui| &mut ui.file.body) {
            view.top_left = to;
            sheets_scroll::follow(view, tx);
        }
    });
    rt.frame(0.0);
}

#[test]
fn office_sheets_asks_the_cells_in_view() {
    let (mut rt, log) = live_runtime(Body::Sheet(big()), 1.0, None);
    scroll_to(&mut rt, &log, Cell { row: 100, col: 3 });
    rt.frame(0.0);
    assert_eq!(
        sheet_in(&mut rt).top_left,
        Cell { row: 100, col: 3 },
        "the viewports agree"
    );
    let asked = log.cells.borrow().clone();
    assert_eq!(asked.len(), 1, "one ask per frame: {asked:?}");
    let (sheet, row, col, rows, cols) = asked[0];
    assert_eq!((sheet, row, col), (0, 100, 3), "from the first row and column in view");
    assert!(rows >= 34 && cols >= 14, "at least the view: {rows} x {cols}");
}

#[test]
fn office_sheets_draws_from_the_top_left() {
    let mut view = big();
    view.cells.push(cell(100, 3, "here", None));
    view.cells.push(cell(0, 0, "far away", None));
    let (mut rt, log) = live_runtime(Body::Sheet(view), 1.0, None);
    scroll_to(&mut rt, &log, Cell { row: 100, col: 3 });
    rt.frame(0.0);
    assert!(
        find(&rt, "sh-cell-100-3-t").is_some(),
        "the first cell in view is drawn"
    );
    assert!(find(&rt, "sh-cell-0-0-t").is_none(), "what scrolled past is not");
    assert!(find(&rt, "sh-line-399").is_none(), "only the rows that fit are drawn");
}

#[test]
fn office_sheets_scroll_stops_at_the_extent() {
    let mut view = big();
    view.extent = (100, 8);
    view.top_left = Cell { row: 99, col: 7 };
    assert!(
        scroll_key(&mut view, &Key::PageDown, &Mods::default()),
        "the key is taken"
    );
    assert_eq!(
        view.top_left,
        Cell { row: 99, col: 7 },
        "Page Down at the last screen stays"
    );
    assert!(scroll_key(&mut view, &Key::Right, &Mods::default()));
    assert_eq!(
        view.top_left,
        Cell { row: 99, col: 7 },
        "Right at the last column stays"
    );
    assert!(scroll_key(&mut view, &Key::Home, &Mods::default()));
    assert_eq!(view.top_left, Cell { row: 0, col: 0 });
    assert!(scroll_key(&mut view, &Key::PageUp, &Mods::default()));
    assert_eq!(view.top_left, Cell { row: 0, col: 0 }, "Page Up at the top stays");
}

#[test]
fn office_sheets_keys_scroll_a_live_sheet() {
    let (mut rt, _log) = live_runtime(Body::Sheet(big()), 1.0, None);
    rt.on_key = Some(crate::shell::host::on_key);
    rt.key(rune::KeyEvent {
        key: Key::PageDown,
        mods: Mods::default(),
    });
    rt.frame(0.0);
    assert_eq!(sheet_in(&mut rt).top_left, Cell { row: PAGE_ROWS, col: 0 });
}

#[test]
fn office_sheets_formula_bar_shows_the_current_cells_formula() {
    let mut view = big();
    let mut b10 = cell(9, 1, "13,220", Some(13220.0));
    b10.formula = Some("=SUM(B2:B9)".into());
    view.cells.push(b10);
    view.current = Cell { row: 9, col: 1 };
    let (rt, _log) = live_runtime(Body::Sheet(view), 1.0, None);
    let bar = find(&rt, "sh-formula").expect("the formula bar");
    assert_eq!(bar.text.clone().unwrap_or_default(), "=SUM(B2:B9)");
}

#[test]
fn office_sheets_formula_bar_falls_back_to_the_cells_text() {
    let mut view = big();
    view.cells.push(cell(0, 0, "Item", None));
    view.current = Cell { row: 0, col: 0 };
    assert_eq!(formula_text(&view), "Item");
}

#[test]
fn office_sheets_stats_use_the_values() {
    let mut view = big();
    view.cells.push(cell(0, 0, "\u{20AC}1,000.00", Some(1000.0)));
    view.cells.push(cell(1, 0, "12.5%", Some(0.125)));
    view.selection = Some((Cell { row: 0, col: 0 }, Cell { row: 1, col: 0 }));
    let stats = stats(&view).expect("a selection");
    assert_eq!(stats.count, 2, "both cells count: their numbers, not their text");
    assert_eq!(status_words(&view)[1], "Sum 1,000.13");
    assert!(status_words(&view).contains(&"Count 2".to_string()));
}

#[test]
fn office_sheets_an_anchor_draws_its_picture_at_its_place() {
    let mut view = big();
    view.anchors.push(SheetAnchor {
        anchor: 7,
        from: Cell { row: 2, col: 1 },
        to: Cell { row: 8, col: 4 },
        x_pt: 100.0,
        y_pt: 60.0,
        w_pt: 200.0,
        h_pt: 120.0,
    });
    let (rt, log) = live_runtime(Body::Sheet(view), 1.0, Some((400, 240)));
    assert!(
        log.pictures.borrow().iter().any(|k| matches!(
            k,
            OfficePicture::Anchor {
                sheet: 0,
                anchor: 7,
                ..
            }
        )),
        "the anchor's picture is asked"
    );
    let grid = find(&rt, "sh-grid").expect("the grid");
    let at = find(&rt, "sh-anchor-7").expect("the anchor is drawn");
    assert!(
        (at.rect[0] - grid.rect[0] - (ROW_HEAD_W + 100.0)).abs() < 0.6,
        "x: {:?} in {:?}",
        at.rect,
        grid.rect
    );
    assert!(
        (at.rect[1] - grid.rect[1] - (HEADER_H + 60.0)).abs() < 0.6,
        "y: {:?} in {:?}",
        at.rect,
        grid.rect
    );
    assert!(
        (at.rect[2] - 200.0).abs() < 0.6 && (at.rect[3] - 120.0).abs() < 0.6,
        "its size in points"
    );
}

#[test]
fn office_sheets_an_anchor_without_its_picture_draws_its_box_empty() {
    let mut view = big();
    view.anchors.push(SheetAnchor {
        anchor: 1,
        from: Cell { row: 0, col: 0 },
        to: Cell { row: 3, col: 3 },
        x_pt: 10.0,
        y_pt: 10.0,
        w_pt: 100.0,
        h_pt: 60.0,
    });
    let (rt, _log) = live_runtime(Body::Sheet(view), 1.0, None);
    assert!(
        find(&rt, "sh-anchor-1").is_some(),
        "the box is there before the picture"
    );
}

#[test]
fn office_sheets_quiet_line_when_content_is_missing() {
    let mut view = big();
    view.missing_content = true;
    let (rt, _) = live_runtime(Body::Sheet(view), 1.0, None);
    assert!(
        rt.elements()
            .into_iter()
            .any(|e| e.text.as_deref() == Some(QUIET_LINE) && !e.exiting),
        "the quiet line is in the status bar"
    );
    let (rt, _) = live_runtime(Body::Sheet(big()), 1.0, None);
    assert!(
        !rt.elements().into_iter().any(|e| e.text.as_deref() == Some(QUIET_LINE)),
        "and only then"
    );
}

#[test]
fn office_sheets_a_tab_switches_the_sheet() {
    let mut view = big();
    view.tabs = vec!["A".into(), "B".into()];
    let (mut rt, _) = live_runtime(Body::Sheet(view), 1.0, None);
    let tab = find(&rt, "sh-tab-1").expect("the second tab");
    rt.pointer_down(tab.rect[0] + 4.0, tab.rect[1] + 4.0);
    rt.pointer_up(tab.rect[0] + 4.0, tab.rect[1] + 4.0);
    rt.frame(0.0);
    let view = sheet_in(&mut rt);
    assert_eq!((view.sheet, view.active), (1, 1));
}
