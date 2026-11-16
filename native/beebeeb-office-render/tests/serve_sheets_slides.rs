//! The helper's loop for workbooks and decks, end to end in one process as `tests/serve.rs`: a cell's value and
//! formula, a block's charts and pictures, and the request that draws one, through a fake engine; then the Office
//! engine's own workbook and deck (`sheets-3.xlsx`, `deck-3.pptx`).
//!
//! The two real-engine tests are EXPECTED RED at the S0 pin (its workbooks and decks open nothing yet) and green once
//! O-CP3 moves the pin to the merged sheets and slides head: first seen there.

mod common;

use std::collections::HashSet;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use beebeeb_office as eng;
use beebeeb_office_protocol::check::{
    Limits, MAX_ANCHORS, MAX_FORMULA_BYTES, asked_anchor_size, asked_size, check_cells, check_opened, check_outline,
    check_rendered, check_rendered_anchor, check_sheets, render_scale,
};
use beebeeb_office_protocol::office::{
    self, Align, Anchor, AnchorKind, Cancel, CellData, CellRange, CellText, Cells, DocKind, ErrorCode, Frozen, Message,
    Open, Outline, Render, RenderAnchor, Rendered, SheetInfo,
};
use beebeeb_office_render::{EXIT_OK, Office, Opening, block_cells};

use common::{
    FAKE_ANCHOR_BYTES, FAKE_ANCHOR_SIZE, FAKE_PAGE_BYTES, FAKE_SIZE, Fake, FakeEngine, FakeMemory, Gate, NO_GRANT,
    Running, render_into,
};

const SHEETS: &[u8] = include_bytes!("fixtures/sheets-3.xlsx");
const DECK: &[u8] = include_bytes!("fixtures/deck-3.pptx");
const L: Limits = Limits::CONTRACT;

/// A 10 x 10 range from A1.
const RANGE: CellRange = CellRange {
    row: 0,
    col: 0,
    rows: 10,
    cols: 10,
};

fn error(code: ErrorCode) -> Message {
    Message::Error(office::Error { code })
}

/// The distinct RGBA values in `pixels`.
fn colours(pixels: &[u8]) -> usize {
    let (px, _) = pixels.as_chunks::<4>();
    px.iter().collect::<HashSet<_>>().len()
}

/// The helper with a fake engine (`fake`, its opening replaced by one 50 x 50 sheet), the workbook open.
fn open_fake_sheets(memory: &Arc<FakeMemory>, fake: Fake) -> (Running, Arc<Fake>) {
    let fake = Arc::new(Fake {
        opening: Ok(Opening::Sheets(vec![SheetInfo {
            name: String::from("Sheet1"),
            rows: 50,
            cols: 50,
        }])),
        ..fake
    });
    let mut helper = Running::start(Arc::clone(memory), Arc::new(FakeEngine(Arc::clone(&fake))));
    match helper.open_fake(memory) {
        Message::OpenedSheets(sheets) => assert_eq!(sheets.sheets.len(), 1),
        other => panic!("OpenedSheets was due: {other:?}"),
    }
    (helper, fake)
}

/// The helper with the real Office engine, the file open (id 0): the helper and what it answered.
fn open_real(memory: &Arc<FakeMemory>, kind: DocKind, bytes: &[u8]) -> (Running, Message) {
    let mut helper = Running::start(Arc::clone(memory), Arc::new(Office));
    let doc = memory.section(bytes.to_vec());
    helper.send(0, &Message::Open(Open { kind, doc }));
    let (id, answer) = helper.answer();
    assert_eq!(id, 0, "the answer carries the Open's id");
    (helper, answer)
}

/// A block for [`RANGE`] holding `cells` and `anchors`.
fn block(cells: Vec<CellText>, anchors: Vec<Anchor>) -> CellData {
    CellData {
        cells,
        col_widths: vec![48.0; 10],
        row_heights: vec![15.0; 10],
        merges: Vec::new(),
        frozen: Frozen { rows: 0, cols: 0 },
        anchors,
        missing_content: false,
    }
}

fn chart(id: u32) -> Anchor {
    Anchor {
        id,
        kind: AnchorKind::Chart,
        from_row: 3,
        from_col: 4,
        to_row: 8,
        to_col: 9,
        x_pt: 2.0,
        y_pt: 1.5,
        w_pt: 300.0,
        h_pt: 90.0,
    }
}

/// Asks [`RANGE`] of sheet 0 as request `id` and returns the checked answer.
fn ask_cells(helper: &mut Running, id: u32) -> CellData {
    let asked = Cells { sheet: 0, range: RANGE };
    helper.send(id, &Message::Cells(asked));
    match helper.answer() {
        (got, Message::CellData(data)) if got == id => {
            check_cells(&asked, &data, &L).expect("the app's check accepts the block");
            data
        }
        other => panic!("CellData for {id} was due: {other:?}"),
    }
}

/// A block as the Office engine gives it: values behind formatted numbers, formulas, a chart and a picture.
fn engine_block() -> eng::CellBlock {
    let cell = |row: u32, col: u32, text: &str, value: Option<f64>, formula: Option<&str>| eng::CellText {
        row,
        col,
        text: text.to_owned(),
        align: if value.is_some() {
            eng::CellAlign::Right
        } else {
            eng::CellAlign::Left
        },
        bold: row == 9,
        value,
        formula: formula.map(str::to_owned),
    };
    eng::CellBlock {
        cells: vec![
            cell(1, 1, "1,234.50 €", Some(1234.5), None),
            cell(2, 1, "12.5%", Some(0.125), Some("=B2/B9")),
            cell(9, 0, "Total", None, None),
            cell(9, 1, "13,220", Some(13_220.0), Some("=SUM(B2:B9)")),
        ],
        col_widths: vec![48.0; 10],
        row_heights: vec![15.0; 10],
        merges: Vec::new(),
        frozen: (1, 0),
        anchors: vec![
            eng::Anchor {
                id: 0,
                kind: eng::AnchorKind::Chart,
                from: (3, 4),
                to: (8, 9),
                x_pt: 2.0,
                y_pt: 1.5,
                w_pt: 300.0,
                h_pt: 90.0,
            },
            eng::Anchor {
                id: 1,
                kind: eng::AnchorKind::Picture,
                from: (0, 5),
                to: (0, 6),
                x_pt: 0.0,
                y_pt: 3.0,
                w_pt: 64.0,
                h_pt: 12.0,
            },
        ],
        missing_content: true,
    }
}

/// A cell's value and formula, and the block's charts and pictures, reach the app field for field: the engine's block
/// mapped (`block_cells`), then sent over the pipe.
#[test]
fn a_cell_block_carries_values_formulas_and_anchors() {
    let data = block_cells(engine_block());
    let cell = |row: u32, col: u32, text: &str, value: Option<f64>, formula: Option<&str>, align: Align, bold: bool| {
        CellText {
            row,
            col,
            text: text.to_owned(),
            align,
            bold,
            value,
            formula: formula.map(str::to_owned),
        }
    };
    let want_cells = vec![
        cell(1, 1, "1,234.50 €", Some(1234.5), None, Align::Right, false),
        cell(2, 1, "12.5%", Some(0.125), Some("=B2/B9"), Align::Right, false),
        cell(9, 0, "Total", None, None, Align::Left, true),
        cell(9, 1, "13,220", Some(13_220.0), Some("=SUM(B2:B9)"), Align::Right, true),
    ];
    assert!(
        data.cells == want_cells,
        "every cell's text, value and formula as the engine gave them"
    );
    let picture = Anchor {
        id: 1,
        kind: AnchorKind::Picture,
        from_row: 0,
        from_col: 5,
        to_row: 0,
        to_col: 6,
        x_pt: 0.0,
        y_pt: 3.0,
        w_pt: 64.0,
        h_pt: 12.0,
    };
    assert_eq!(data.anchors, vec![chart(0), picture]);
    assert_eq!(data.frozen, Frozen { rows: 1, cols: 0 });
    assert!(data.missing_content);

    // Over the pipe, as the app gets it.
    let memory = FakeMemory::new();
    let (mut helper, fake) = open_fake_sheets(
        &memory,
        Fake {
            cells: Ok(data.clone()),
            ..Fake::default()
        },
    );
    let got = ask_cells(&mut helper, 2);
    assert!(got == data, "the block is sent as the engine gave it");
    let values: Vec<Option<f64>> = got.cells.iter().map(|c| c.value).collect();
    assert_eq!(values, [Some(1234.5), Some(0.125), None, Some(13_220.0)]);
    let formulas: Vec<Option<&str>> = got.cells.iter().map(|c| c.formula.as_deref()).collect();
    assert_eq!(formulas, [None, Some("=B2/B9"), None, Some("=SUM(B2:B9)")]);
    // What Sum and Average add: the numbers behind the shown text, a currency and a percent cell included.
    let sum: f64 = got.cells.iter().filter_map(|c| c.value).sum();
    assert_eq!(sum, 1234.5 + 0.125 + 13_220.0);
    assert_eq!(fake.cells_asked(), 1);
    helper.close();
    assert_eq!(helper.exit_code(), EXIT_OK);
}

/// A formula over 8 KiB is cut before the cap, at a character boundary, like a cell's text; the app's check passes.
#[test]
fn a_long_formula_is_cut_on_a_char_boundary() {
    // "=", then 8,190 bytes of "A", then a 4-byte character that straddles the cap: 8,195 bytes.
    let long = format!("={}{}", "A".repeat(MAX_FORMULA_BYTES - 2), '\u{1D11E}');
    assert_eq!(long.len(), MAX_FORMULA_BYTES + 3);
    assert!(
        !long.is_char_boundary(MAX_FORMULA_BYTES),
        "the premise: the cap falls inside the character"
    );
    let one = CellText {
        row: 0,
        col: 0,
        text: "1".to_owned(),
        align: Align::Right,
        bold: false,
        value: Some(1.0),
        formula: Some(long.clone()),
    };
    let memory = FakeMemory::new();
    let (mut helper, _) = open_fake_sheets(
        &memory,
        Fake {
            cells: Ok(block(vec![one], Vec::new())),
            ..Fake::default()
        },
    );
    let got = ask_cells(&mut helper, 2);
    let formula = got.cells[0].formula.as_deref().expect("the formula is kept");
    assert_eq!(
        formula.len(),
        MAX_FORMULA_BYTES - 1,
        "cut before the character, not inside it"
    );
    assert_eq!(formula, &long[..MAX_FORMULA_BYTES - 1]);
    assert_eq!(got.cells[0].value, Some(1.0));
    helper.close();
    assert_eq!(helper.exit_code(), EXIT_OK);
}

/// Anchors past the 256 a block carries, and one larger than 20,000 pt, are dropped, and the block says
/// `missing_content`; a block whose anchors all fit says nothing.
#[test]
fn too_many_or_too_large_anchors_are_dropped_and_said() {
    let mut anchors: Vec<Anchor> = (0..300).map(chart).collect();
    anchors[7].w_pt = 30_000.0;
    let memory = FakeMemory::new();
    let (mut helper, _) = open_fake_sheets(
        &memory,
        Fake {
            cells: Ok(block(Vec::new(), anchors)),
            ..Fake::default()
        },
    );
    let got = ask_cells(&mut helper, 2);
    assert_eq!(got.anchors.len(), MAX_ANCHORS);
    let ids: Vec<u32> = got.anchors.iter().map(|a| a.id).collect();
    assert!(!ids.contains(&7), "the 30,000 pt wide one is dropped");
    assert_eq!(
        ids,
        (0..=256u32).filter(|id| *id != 7).collect::<Vec<_>>(),
        "the first 256 that fit, in order"
    );
    assert!(got.missing_content, "what was dropped is said");
    helper.close();
    assert_eq!(helper.exit_code(), EXIT_OK);

    let memory = FakeMemory::new();
    let (mut helper, _) = open_fake_sheets(
        &memory,
        Fake {
            cells: Ok(block(Vec::new(), (0..3).map(chart).collect())),
            ..Fake::default()
        },
    );
    let got = ask_cells(&mut helper, 2);
    assert_eq!(got.anchors, (0..3).map(chart).collect::<Vec<_>>());
    assert!(!got.missing_content, "nothing dropped, nothing said");
    helper.close();
    assert_eq!(helper.exit_code(), EXIT_OK);
}

/// The fake's chart as the app holds it: sized 5 x 4 px at scale 1, the fake engine's size.
fn fake_chart(id: u32) -> Anchor {
    Anchor {
        w_pt: FAKE_ANCHOR_SIZE.0 as f32,
        h_pt: FAKE_ANCHOR_SIZE.1 as f32,
        ..chart(id)
    }
}

fn draw_into(anchor: u32, out: office::GrantRef) -> RenderAnchor {
    RenderAnchor {
        sheet: 0,
        anchor,
        scale: 1.0,
        out,
    }
}

/// `RenderAnchor` → `Rendered` at the engine's size, the pixels in the buffer, the mapping dropped before the answer.
#[test]
fn an_anchor_renders_through_the_pipe() {
    let memory = FakeMemory::new();
    let (mut helper, fake) = open_fake_sheets(&memory, Fake::default());
    let out = memory.zeroed(FAKE_ANCHOR_BYTES);
    let ask = draw_into(3, out);
    helper.send(2, &Message::RenderAnchor(ask));
    let rendered = Rendered {
        w: FAKE_ANCHOR_SIZE.0,
        h: FAKE_ANCHOR_SIZE.1,
        missing_content: true,
    };
    assert_eq!(helper.answer(), (2, Message::Rendered(rendered)));
    assert_eq!(memory.live_writes(), 0, "the mapping was dropped before the answer");
    assert_eq!(memory.maps(out), 1, "mapped exactly once");
    assert_eq!(fake.anchor_renders(), 1);
    assert!(colours(&memory.bytes(out)) >= 10, "the drawing reached the buffer");
    check_rendered_anchor(&ask, &fake_chart(3), out.len, &rendered, &L).expect("the app's check accepts it");
    helper.close();
    assert_eq!(helper.exit_code(), EXIT_OK);
}

/// A buffer whose length is not the engine's size is an error, never a render into part of it; the grant is closed.
#[test]
fn an_anchor_size_mismatch_is_an_error_not_a_guess() {
    let memory = FakeMemory::new();
    let (mut helper, fake) = open_fake_sheets(&memory, Fake::default());
    // 1 px wide instead of 5: 1 x 4 px.
    let narrow = memory.zeroed(4 * 4);
    helper.send(2, &Message::RenderAnchor(draw_into(3, narrow)));
    assert_eq!(helper.answer(), (2, error(ErrorCode::Internal)));
    assert_eq!(fake.anchor_renders(), 0, "the engine was not asked to render");
    assert_eq!(memory.bytes(narrow), vec![0; 16], "nothing was written");
    assert_eq!(
        memory.maps(narrow),
        1,
        "the refused buffer was mapped once, to close it"
    );
    assert_eq!(memory.live_writes(), 0);

    // The engine's own refusal travels as its code, and closes the grant too.
    let refused = Fake {
        anchor_size: Err(ErrorCode::TooLarge),
        ..Fake::default()
    };
    let memory2 = FakeMemory::new();
    let (mut helper2, fake2) = open_fake_sheets(&memory2, refused);
    let out = memory2.zeroed(FAKE_ANCHOR_BYTES);
    helper2.send(2, &Message::RenderAnchor(draw_into(3, out)));
    assert_eq!(helper2.answer(), (2, error(ErrorCode::TooLarge)));
    assert_eq!(fake2.anchor_renders(), 0);
    assert_eq!(memory2.maps(out), 1);
    helper2.close();
    assert_eq!(helper2.exit_code(), EXIT_OK);

    // The same anchor into a buffer of the right size renders.
    let right = memory.zeroed(FAKE_ANCHOR_BYTES);
    helper.send(4, &Message::RenderAnchor(draw_into(3, right)));
    assert!(matches!(helper.answer(), (4, Message::Rendered(_))));
    assert_eq!(fake.anchor_renders(), 1);
    helper.close();
    assert_eq!(helper.exit_code(), EXIT_OK);
}

/// A `Cancel` read before a `RenderAnchor` starts skips it: an `Error`, no engine call, nothing drawn, and its buffer
/// mapped once to close the grant.
#[test]
fn a_cancelled_anchor_is_skipped() {
    let gate = Gate::default();
    let memory = FakeMemory::new();
    let (mut helper, fake) = open_fake_sheets(
        &memory,
        Fake {
            gate: Some(gate.clone()),
            ..Fake::default()
        },
    );
    // A page render first, held at the gate, so the anchor is still queued when its Cancel is read.
    let page = memory.zeroed(FAKE_PAGE_BYTES);
    let anchor = memory.zeroed(FAKE_ANCHOR_BYTES);
    helper.send(2, &render_into(page));
    helper.send(4, &Message::RenderAnchor(draw_into(3, anchor)));
    helper.send(4, &Message::Cancel(Cancel));
    helper.wait_handed_out();
    thread::sleep(Duration::from_millis(50));
    gate.open();

    let rendered = Rendered {
        w: FAKE_SIZE.0,
        h: FAKE_SIZE.1,
        missing_content: false,
    };
    assert_eq!(helper.answer(), (2, Message::Rendered(rendered)));
    assert_eq!(helper.answer(), (4, error(ErrorCode::Internal)));
    assert_eq!(
        fake.anchor_renders(),
        0,
        "the cancelled anchor never reached the engine"
    );
    assert_eq!(
        memory.bytes(anchor),
        vec![0; FAKE_ANCHOR_BYTES],
        "nothing was drawn for it"
    );
    assert_eq!(memory.maps(anchor), 1, "its buffer was mapped once, to close it");
    assert_eq!(memory.live_writes(), 0);
    helper.close();
    assert_eq!(helper.exit_code(), EXIT_OK);
    helper.nothing_more();
}

/// The Office engine's workbook through the helper: texts as Excel shows them, the values and the formula behind
/// them, the frozen row, the picture drawn at its size, a merge on the second sheet. FIRST SEEN AT O-CP3.
#[test]
fn a_real_workbook_answers_its_cells() {
    let memory = FakeMemory::new();
    let (mut helper, answer) = open_real(&memory, DocKind::Xlsx, SHEETS);
    let sheets = match answer {
        Message::OpenedSheets(sheets) => sheets,
        other => panic!("OpenedSheets was due: {other:?}"),
    };
    check_sheets(&sheets, &L).expect("the app's check accepts the sheets");
    let names: Vec<&str> = sheets.sheets.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["Budget", "Notes", "Empty"]);
    assert_eq!(sheets.more, 0);

    let data = ask_cells(&mut helper, 2);
    let at = |row: u32, col: u32| {
        data.cells
            .iter()
            .find(|c| c.row == row && c.col == col)
            .unwrap_or_else(|| panic!("no cell at row {row}, column {col}"))
    };
    let shown = |row, col| {
        let c = at(row, col);
        (c.text.as_str(), c.value, c.formula.as_deref())
    };
    assert_eq!(shown(0, 0), ("Item", None, None));
    assert!(at(0, 0).bold);
    assert_eq!(shown(1, 1), ("1,234.50", Some(1234.5), None));
    assert_eq!(shown(1, 2), ("12.5%", Some(0.125), None));
    assert_eq!(shown(2, 1), ("2,765.25", Some(2765.25), None));
    assert_eq!(shown(3, 1), ("362.00", Some(362.0), None));
    assert_eq!(shown(4, 1), ("4,361.75", Some(4361.75), Some("=SUM(B2:B4)")));
    // What the status bar's Sum adds: the values, equal to the file's own total.
    let sum: f64 = (1..=3).filter_map(|row| at(row, 1).value).sum();
    assert_eq!(sum, 4361.75);
    assert_eq!(data.frozen, Frozen { rows: 1, cols: 0 });
    assert!(!data.missing_content, "everything in the range is shown");

    // The picture: over D2:F7 (its far corner lies on the top edge of row 8, which it does not cover; `to` is
    // inclusive), drawn at the size the app asks, in its colours.
    assert_eq!(data.anchors.len(), 1, "one picture touches the range");
    let picture = data.anchors[0].clone();
    assert_eq!(
        (
            picture.kind,
            picture.from_row,
            picture.from_col,
            picture.to_row,
            picture.to_col
        ),
        (AnchorKind::Picture, 1, 3, 6, 5)
    );
    let mut ask = draw_into(picture.id, NO_GRANT);
    ask.scale = render_scale(1.0, 96.0);
    let (w, h) = asked_anchor_size(&ask, &picture, &L).expect("the app can size the picture");
    ask.out = memory.zeroed(w as usize * h as usize * 4);
    helper.send(4, &Message::RenderAnchor(ask));
    match helper.answer() {
        (4, Message::Rendered(rendered)) => {
            check_rendered_anchor(&ask, &picture, ask.out.len, &rendered, &L).expect("the app's check accepts it");
            assert!(!rendered.missing_content, "the embedded picture decodes");
            assert!(
                colours(&memory.bytes(ask.out)) >= 10,
                "the picture's 16 colours, not a blank"
            );
        }
        other => panic!("Rendered for 4 was due: {other:?}"),
    }

    // The second sheet: its text and its merge.
    let notes = Cells { sheet: 1, range: RANGE };
    helper.send(6, &Message::Cells(notes));
    match helper.answer() {
        (6, Message::CellData(data)) => {
            check_cells(&notes, &data, &L).expect("the app's check accepts it");
            assert!(
                data.cells
                    .iter()
                    .any(|c| (c.row, c.col, c.text.as_str()) == (0, 0, "Plain text"))
            );
            let merged = CellRange {
                row: 2,
                col: 0,
                rows: 1,
                cols: 3,
            };
            assert_eq!(data.merges, vec![merged]);
        }
        other => panic!("CellData for 6 was due: {other:?}"),
    }
    helper.close();
    assert_eq!(helper.exit_code(), EXIT_OK);
}

/// The Office engine's deck through the helper: three 16:9 slides, slide 1 drawn at the size the app asks, the titles
/// as the outline. FIRST SEEN AT O-CP3.
#[test]
fn a_real_deck_renders_slide_one() {
    let memory = FakeMemory::new();
    let (mut helper, answer) = open_real(&memory, DocKind::Pptx, DECK);
    let opened = match answer {
        Message::Opened(opened) => opened,
        other => panic!("Opened was due: {other:?}"),
    };
    check_opened(&opened, &L).expect("the app's check accepts the Opened");
    assert_eq!(opened.pages, 3);
    for size in &opened.sizes {
        assert_eq!((size.w_pt, size.h_pt), (960.0, 540.0), "16:9");
    }

    let mut ask = Render {
        page: 0,
        scale: render_scale(1.0, 96.0),
        region: None,
        out: NO_GRANT,
    };
    let (w, h) = asked_size(&ask, &opened.sizes[0], &L).expect("the app can size the slide");
    assert_eq!((w, h), (1280, 720));
    ask.out = memory.zeroed(w as usize * h as usize * 4);
    helper.send(2, &Message::Render(ask));
    match helper.answer() {
        (2, Message::Rendered(rendered)) => {
            check_rendered(&ask, &opened.sizes[0], ask.out.len, &rendered, &L).expect("the app's check accepts it");
            assert!(colours(&memory.bytes(ask.out)) >= 10, "the slide is drawn, not blank");
        }
        other => panic!("Rendered for 2 was due: {other:?}"),
    }

    let from = Outline { from: 0 };
    helper.send(4, &Message::Outline(from));
    match helper.answer() {
        (4, Message::OutlineData(outline)) => {
            check_outline(&from, &outline, opened.pages, &L).expect("the app's check accepts it");
            let titles: Vec<(&str, u32, u8)> = outline
                .entries
                .iter()
                .map(|e| (e.text.as_str(), e.page, e.level))
                .collect();
            assert_eq!(
                titles,
                [("Q3 plan", 0, 1), ("Thanks", 2, 1)],
                "one entry per titled slide"
            );
            assert_eq!(outline.next, None);
        }
        other => panic!("OutlineData for 4 was due: {other:?}"),
    }
    helper.close();
    assert_eq!(helper.exit_code(), EXIT_OK);
}
