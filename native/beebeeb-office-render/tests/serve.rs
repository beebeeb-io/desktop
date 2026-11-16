//! The helper's loop, end to end in one process: `serve` on its own thread, frames over an in-memory pipe each way,
//! a fake memory that counts mappings, and the real Office engine or a fake one.

mod common;

use std::collections::HashSet;
use std::io::ErrorKind;
use std::net::TcpListener;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use beebeeb_office::{OpenError, limits};
use beebeeb_office_protocol::check::{
    Limits, MAX_INPUT_BYTES, MAX_SHEET_NAME_CHARS, asked_size, check_cells, check_opened, check_outline,
    check_rendered, check_sheets, page_pixels, render_scale,
};
use beebeeb_office_protocol::frame::{MAX_PAYLOAD, write_frame};
use beebeeb_office_protocol::office::{
    self, Align, Cancel, CellData, CellRange, CellText, Cells, DocKind, EncodeError, ErrorCode, Frozen, GrantRef,
    KIND_RENDER, KIND_STATS, Message, Open, Opened, Outline, OutlineData, OutlineEntry, PageSize, Rect, Render,
    Rendered, SheetInfo, Stats,
};
use beebeeb_office_render::{EXIT_OK, EXIT_PROTOCOL, Engine, Office, OpenDoc, Opening, open_code};

use common::{FAKE_PAGE_BYTES, FAKE_SIZE, Fake, FakeEngine, FakeMemory, Gate, NO_GRANT, Running, render_into};

/// The shared page-pixel vector: the same file in the Office engine's tests.
const VECTORS: &str = include_str!("../../../crates/beebeeb-office-protocol/tests/page_pixels_vectors.txt");
const LETTER: &[u8] = include_bytes!("fixtures/letter-1.docx");
const SPIKE: &[u8] = include_bytes!("fixtures/spike.docx");
const REMOTE: &[u8] = include_bytes!("fixtures/remote.docx");
const SHEETS: &[u8] = include_bytes!("fixtures/sheets-3.xlsx");
const DECK: &[u8] = include_bytes!("fixtures/deck-3.pptx");

/// The port `remote.docx` links its picture to.
const REMOTE_PORT: u16 = 47477;

fn error(code: ErrorCode) -> Message {
    Message::Error(office::Error { code })
}

/// The helper with the real Office engine, the document already open (id 0): the helper and what it answered.
fn open_real(memory: &Arc<FakeMemory>, kind: DocKind, bytes: &[u8]) -> (Running, Message) {
    let mut helper = Running::start(Arc::clone(memory), Arc::new(Office));
    let doc = memory.section(bytes.to_vec());
    helper.send(0, &Message::Open(Open { kind, doc }));
    let (id, answer) = helper.answer();
    assert_eq!(id, 0, "the answer carries the Open's id");
    (helper, answer)
}

fn opened(answer: Message) -> Opened {
    match answer {
        Message::Opened(opened) => {
            check_opened(&opened, &Limits::CONTRACT).expect("the app's check accepts the Opened");
            opened
        }
        other => panic!("Opened was due: {other:?}"),
    }
}

fn first_page(doc: &dyn OpenDoc) -> PageSize {
    match doc.opened() {
        Opening::Pages(opened) => opened.sizes[0],
        other => panic!("pages were due: {other:?}"),
    }
}

/// A whole-page ask of page 0 at `scale`, sized only.
fn whole_page(scale: f32) -> Render {
    Render {
        page: 0,
        scale,
        region: None,
        out: NO_GRANT,
    }
}

/// The helper with a fake engine that opens a workbook of one 50 x 50 sheet and answers every `Cells` with `cells`.
fn open_fake_sheets(memory: &Arc<FakeMemory>, cells: CellData) -> (Running, Arc<Fake>) {
    let fake = Arc::new(Fake {
        opening: Ok(Opening::Sheets(vec![SheetInfo {
            name: String::from("Sheet1"),
            rows: 50,
            cols: 50,
        }])),
        cells: Ok(cells),
        ..Fake::default()
    });
    let mut helper = Running::start(Arc::clone(memory), Arc::new(FakeEngine(Arc::clone(&fake))));
    match helper.open_fake(memory) {
        Message::OpenedSheets(sheets) => assert_eq!(sheets.sheets.len(), 1),
        other => panic!("OpenedSheets was due: {other:?}"),
    }
    (helper, fake)
}

fn cell(row: u32, col: u32, text_len: usize) -> CellText {
    CellText {
        row,
        col,
        text: "x".repeat(text_len),
        align: Align::Left,
        bold: false,
        value: None,
        formula: None,
    }
}

fn no_merges_block(cells: Vec<CellText>, rows: usize, cols: usize) -> CellData {
    CellData {
        cells,
        col_widths: vec![48.0; cols],
        row_heights: vec![15.0; rows],
        merges: Vec::new(),
        frozen: Frozen { rows: 0, cols: 0 },
        anchors: Vec::new(),
        missing_content: false,
    }
}

/// Rule 1: a page travels. `Open` → `Opened`; `Render` → `Rendered` at the protocol's size, pixels in the buffer.
#[test]
fn a_docx_opens_and_its_page_renders_through_the_pipe() {
    let memory = FakeMemory::new();
    let (mut helper, answer) = open_real(&memory, DocKind::Docx, SPIKE);
    let opened = opened(answer);
    assert_eq!(opened.pages, 3, "spike.docx has three pages");

    let ask = whole_page(render_scale(1.0, 96.0));
    let page = opened.sizes[0];
    let (w, h) = asked_size(&ask, &page, &Limits::CONTRACT).expect("the app can size the page");
    let out = memory.zeroed(w as usize * h as usize * 4);
    let ask = Render { out, ..ask };
    helper.send(2, &Message::Render(ask));
    let rendered = match helper.answer() {
        (2, Message::Rendered(rendered)) => rendered,
        other => panic!("Rendered for 2 was due: {other:?}"),
    };
    assert_eq!((rendered.w, rendered.h), (w, h), "the page is the size the app asked");
    check_rendered(&ask, &page, out.len, &rendered, &Limits::CONTRACT).expect("the app's check accepts it");

    let pixels = memory.bytes(out);
    let colours: HashSet<&[u8; 4]> = pixels.as_chunks::<4>().0.iter().collect();
    assert!(
        colours.len() >= 10,
        "the page was drawn into the buffer: {} distinct colours",
        colours.len()
    );

    helper.close();
    assert_eq!(helper.exit_code(), EXIT_OK);
}

/// Rule 2: one pixel rule where both crates link. The protocol's `page_pixels` and `asked_size` equal the Office
/// engine's `render_size` for Letter and A4 at every scale the app uses, and every row of the shared vector agrees with
/// `render_size` on the fixture whose page it names.
#[test]
fn the_helper_and_the_protocol_size_a_page_alike() {
    let letter = Office.open(DocKind::Docx, LETTER).expect("letter-1.docx opens");
    let a4 = Office.open(DocKind::Docx, SPIKE).expect("spike.docx opens");
    let letter_page = first_page(&*letter);
    let a4_page = first_page(&*a4);
    assert!(
        (letter_page.w_pt - 612.0).abs() <= 0.01 && (letter_page.h_pt - 792.0).abs() <= 0.01,
        "letter-1.docx is a Letter page: {letter_page:?}"
    );
    assert!(
        (a4_page.w_pt - 595.3).abs() <= 0.01 && (a4_page.h_pt - 841.9).abs() <= 0.01,
        "spike.docx is an A4 page: {a4_page:?}"
    );

    let scales = [
        1.0,
        1.25,
        1.5,
        2.0,
        render_scale(1.0, 96.0),
        render_scale(1.25, 96.0),
        render_scale(1.5, 144.0),
    ];
    let tile = Rect {
        x: 0,
        y: 0,
        w: 512,
        h: 512,
    };
    let mut compared = 0;
    for (doc, page) in [(&letter, letter_page), (&a4, a4_page)] {
        for scale in scales {
            let engine = doc.render_size(0, scale, None).expect("the engine sizes the page");
            assert_eq!(page_pixels(&page, scale), Some(engine), "{page:?} at {scale}");
            let ask = whole_page(scale);
            assert_eq!(
                asked_size(&ask, &page, &Limits::CONTRACT),
                Ok(engine),
                "{page:?} at {scale}"
            );
            let tiled = Render {
                region: Some(tile),
                ..ask
            };
            assert_eq!(
                doc.render_size(0, scale, Some(tile)),
                Ok((512, 512)),
                "{page:?} at {scale}"
            );
            assert_eq!(
                asked_size(&tiled, &page, &Limits::CONTRACT),
                Ok((512, 512)),
                "{page:?} at {scale}"
            );
            compared += 1;
        }
    }
    assert_eq!(compared, 14, "two pages at seven scales");

    // The shared vector, by its bits: label, pt, pt bits, scale, scale bits, px.
    let bits = |field: &str| -> f32 {
        let hex = field.strip_prefix("0x").expect("bits start with 0x");
        f32::from_bits(u32::from_str_radix(hex, 16).expect("bits are hex"))
    };
    let mut matched = 0;
    for line in VECTORS.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.split_whitespace().collect();
        assert_eq!(fields.len(), 6, "label, pt, bits, scale, bits, px: {line}");
        let (label, pt, scale) = (fields[0], bits(fields[2]), bits(fields[4]));
        let px: u32 = fields[5].parse().expect("px is a whole number");
        let (doc, page) = if label.starts_with("letter-") {
            (&letter, letter_page)
        } else if label.starts_with("a4-") {
            (&a4, a4_page)
        } else {
            panic!("a row that names no fixture: {line}");
        };
        let (w, h) = doc.render_size(0, scale, None).expect("the engine sizes the page");
        let (side_pt, side_px) = if label.ends_with("-w") {
            (page.w_pt, w)
        } else if label.ends_with("-h") {
            (page.h_pt, h)
        } else {
            panic!("a row that names no side: {line}");
        };
        assert!(
            (side_pt - pt).abs() <= 0.01,
            "the fixture's side is {side_pt} pt, the row's {pt}: {line}"
        );
        assert_eq!(side_px, px, "{line}");
        matched += 1;
    }
    assert_eq!(matched, 20, "every row of the vector matched a fixture");

    // Through the pipe: Letter at 100 % on a 96 DPI screen, the buffer sized from the app's ask.
    let memory = FakeMemory::new();
    let (mut helper, answer) = open_real(&memory, DocKind::Docx, LETTER);
    let opened = opened(answer);
    let ask = whole_page(render_scale(1.0, 96.0));
    let (w, h) = asked_size(&ask, &opened.sizes[0], &Limits::CONTRACT).expect("the app can size the page");
    assert_eq!((w, h), (816, 1056), "Letter at 4/3 in single precision");
    let out = memory.zeroed(w as usize * h as usize * 4);
    helper.send(2, &Message::Render(Render { out, ..ask }));
    match helper.answer() {
        (2, Message::Rendered(rendered)) => assert_eq!((rendered.w, rendered.h), (816, 1056)),
        other => panic!("Rendered for 2 was due: {other:?}"),
    }
    helper.close();
    assert_eq!(helper.exit_code(), EXIT_OK);
}

/// Outline entries whose whole page (`next: None`) encodes to exactly `MAX_PAYLOAD + extra` bytes: 63 entries of
/// 1 KiB and a last one that makes up the rest.
fn outline_of(extra: usize) -> Vec<OutlineEntry> {
    let entry = |text_len: usize| OutlineEntry {
        level: 1,
        text: "h".repeat(text_len),
        page: 0,
    };
    // The count (4) and the `None` tag (1), then per entry its level (1), text length (4), text, page (4).
    let mut entries = vec![entry(1024); 63];
    let used = 4 + 1 + 63 * (1 + 4 + 1024 + 4);
    entries.push(entry(MAX_PAYLOAD + extra - used - (1 + 4 + 4)));
    entries
}

/// Rule 3: the outline comes in pages that advance; together they are the outline.
#[test]
fn an_outline_over_one_frame_comes_in_pages() {
    // The premise, by the encoding itself: one is exactly a frame, the other one byte over.
    let exact = OutlineData {
        entries: outline_of(0),
        next: None,
    };
    assert_eq!(exact.encode().map(|p| p.len()), Ok(MAX_PAYLOAD));
    let over = OutlineData {
        entries: outline_of(1),
        next: None,
    };
    assert_eq!(over.encode(), Err(EncodeError::OverFrame));

    for (extra, pages) in [(0usize, 1usize), (1, 2)] {
        let entries = outline_of(extra);
        let fake = Arc::new(Fake {
            outline: entries.clone(),
            ..Fake::default()
        });
        let memory = FakeMemory::new();
        let mut helper = Running::start(Arc::clone(&memory), Arc::new(FakeEngine(Arc::clone(&fake))));
        opened(helper.open_fake(&memory));

        let mut got = Vec::new();
        let mut from = 0;
        let mut answers = 0;
        loop {
            let id = 2 + 2 * answers as u32;
            let asked = Outline { from };
            helper.send(id, &Message::Outline(asked));
            let page = match helper.answer() {
                (got_id, Message::OutlineData(page)) if got_id == id => page,
                other => panic!("OutlineData for {id} was due: {other:?}"),
            };
            check_outline(&asked, &page, 1, &Limits::CONTRACT).expect("the app's check accepts the page");
            answers += 1;
            got.extend(page.entries);
            match page.next {
                Some(next) => {
                    assert!(next > from, "a page moves the app forward");
                    from = next;
                }
                None => break,
            }
            assert!(answers < 4, "the outline ends");
        }
        assert_eq!(answers, pages, "pages for {extra} byte(s) over");
        assert!(got == entries, "the pages together are the outline");
        assert_eq!(fake.outlines(), 1, "the outline is worked out once");

        helper.close();
        assert_eq!(helper.exit_code(), EXIT_OK);
    }
}

/// Rule 3: a cell block that does not fit one frame is `RangeTooLarge`; one that fits exactly, and one cell of 32 KiB,
/// are sent.
#[test]
fn a_cell_block_over_one_frame_answers_range_too_large() {
    let range = CellRange {
        row: 0,
        col: 0,
        rows: 2,
        cols: 2,
    };
    // A 2 x 2 block with two cells: count (4), two cells (16 bytes and the text each: no value, no formula), widths
    // (4 + 8), heights (4 + 8), no merges (4), frozen panes (8), no anchors (4), `missing_content` (1): 77 bytes and
    // the two texts.
    let block = |second: usize| no_merges_block(vec![cell(0, 0, 32 * 1024), cell(0, 1, second)], 2, 2);
    let fits = MAX_PAYLOAD - 77 - 32 * 1024;
    assert_eq!(
        block(fits).encode().map(|p| p.len()),
        Ok(MAX_PAYLOAD),
        "the premise: exactly one frame"
    );
    assert_eq!(
        block(fits + 1).encode(),
        Err(EncodeError::OverFrame),
        "the premise: one byte over"
    );

    let memory = FakeMemory::new();
    let asked = Cells { sheet: 0, range };

    let (mut helper, _) = open_fake_sheets(&memory, block(fits));
    helper.send(2, &Message::Cells(asked));
    match helper.answer() {
        (2, Message::CellData(data)) => {
            check_cells(&asked, &data, &Limits::CONTRACT).expect("the app's check accepts it");
            assert!(data == block(fits), "the block is sent as the engine gave it");
        }
        other => panic!("CellData for 2 was due: {other:?}"),
    }
    helper.close();
    assert_eq!(helper.exit_code(), EXIT_OK);

    let (mut helper, _) = open_fake_sheets(&memory, block(fits + 1));
    helper.send(2, &Message::Cells(asked));
    assert_eq!(helper.answer(), (2, error(ErrorCode::RangeTooLarge)));
    helper.close();
    assert_eq!(helper.exit_code(), EXIT_OK);

    // One cell of 32 KiB of text always fits.
    let one = CellRange {
        row: 0,
        col: 0,
        rows: 1,
        cols: 1,
    };
    let single = no_merges_block(vec![cell(0, 0, 32 * 1024)], 1, 1);
    let (mut helper, _) = open_fake_sheets(&memory, single.clone());
    let asked_one = Cells { sheet: 0, range: one };
    helper.send(2, &Message::Cells(asked_one));
    match helper.answer() {
        (2, Message::CellData(data)) => {
            check_cells(&asked_one, &data, &Limits::CONTRACT).expect("the app's check accepts it");
            assert!(data == single);
        }
        other => panic!("CellData for 2 was due: {other:?}"),
    }
    // A range whose widths and heights alone overflow a frame: refused without asking the engine.
    let tall = Cells {
        sheet: 0,
        range: CellRange {
            row: 0,
            col: 0,
            rows: 16_384,
            cols: 1,
        },
    };
    helper.send(4, &Message::Cells(tall));
    assert_eq!(helper.answer(), (4, error(ErrorCode::RangeTooLarge)));
    helper.close();
    assert_eq!(helper.exit_code(), EXIT_OK);
}

/// Rule 3: sheet names are cut to 255 characters, the sheets that fit are named, the rest counted in `more`.
#[test]
fn sheet_names_are_cut_and_the_rest_counted() {
    // 300 characters of 3 bytes each: 900 bytes a name before the cut.
    let long = "€".repeat(300);
    let sheets: Vec<SheetInfo> = (0..256)
        .map(|_| SheetInfo {
            name: long.clone(),
            rows: 10,
            cols: 10,
        })
        .collect();
    let fake = Arc::new(Fake {
        opening: Ok(Opening::Sheets(sheets)),
        ..Fake::default()
    });
    let memory = FakeMemory::new();
    let mut helper = Running::start(Arc::clone(&memory), Arc::new(FakeEngine(fake)));
    let got = match helper.open_fake(&memory) {
        Message::OpenedSheets(got) => got,
        other => panic!("OpenedSheets was due: {other:?}"),
    };
    assert!(got.encode().is_ok(), "the answer is one frame");
    check_sheets(&got, &Limits::CONTRACT).expect("the app's check accepts it");
    assert_eq!(
        got.sheets.len() + got.more as usize,
        256,
        "every sheet is named or counted"
    );
    let cut = "€".repeat(MAX_SHEET_NAME_CHARS);
    assert!(
        got.sheets.iter().all(|s| s.name == cut),
        "each name is cut to 255 whole characters"
    );
    // A named sheet takes 4 + 765 + 4 + 4 bytes; with the count and `more` (8): (65,536 - 8) / 777 = 84.
    assert_eq!(got.sheets.len(), 84);
    assert_eq!(got.more, 172);

    helper.close();
    assert_eq!(helper.exit_code(), EXIT_OK);
}

/// A merge that sticks out of the asked range is clipped to it; one outside the range is dropped.
#[test]
fn merges_are_clipped_to_the_asked_range() {
    let range = CellRange {
        row: 10,
        col: 10,
        rows: 5,
        cols: 5,
    };
    let straddles = CellRange {
        row: 8,
        col: 12,
        rows: 4,
        cols: 10,
    };
    let outside = CellRange {
        row: 0,
        col: 0,
        rows: 3,
        cols: 3,
    };
    let mut block = no_merges_block(Vec::new(), 5, 5);
    block.merges = vec![straddles, outside];
    let memory = FakeMemory::new();
    let (mut helper, _) = open_fake_sheets(&memory, block);
    let asked = Cells { sheet: 0, range };
    helper.send(2, &Message::Cells(asked));
    match helper.answer() {
        (2, Message::CellData(data)) => {
            let clipped = CellRange {
                row: 10,
                col: 12,
                rows: 2,
                cols: 3,
            };
            assert_eq!(data.merges, vec![clipped]);
            check_cells(&asked, &data, &Limits::CONTRACT).expect("the app's check accepts it");
        }
        other => panic!("CellData for 2 was due: {other:?}"),
    }
    helper.close();
    assert_eq!(helper.exit_code(), EXIT_OK);
}

/// Rule 4: a `Cancel` read before its request starts skips it: an `Error`, and no engine call.
#[test]
fn a_cancel_read_before_its_request_starts_skips_it() {
    let gate = Gate::default();
    let fake = Arc::new(Fake {
        gate: Some(gate.clone()),
        ..Fake::default()
    });
    let memory = FakeMemory::new();
    let mut helper = Running::start(Arc::clone(&memory), Arc::new(FakeEngine(Arc::clone(&fake))));
    opened(helper.open_fake(&memory));

    let first = memory.zeroed(FAKE_PAGE_BYTES);
    let second = memory.zeroed(FAKE_PAGE_BYTES);
    helper.send(2, &render_into(first));
    helper.send(4, &render_into(second));
    helper.send(4, &Message::Cancel(Cancel));
    // The first render waits until the whole Cancel frame was handed to the helper, and 50 ms more.
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
    assert_eq!(fake.renders(), 1, "the cancelled render never reached the engine");
    assert_eq!(
        memory.bytes(second),
        vec![0; FAKE_PAGE_BYTES],
        "nothing was drawn for it"
    );

    helper.close();
    assert_eq!(helper.exit_code(), EXIT_OK);
    helper.nothing_more();
}

/// Rule 4: exactly one answer per request, in request order, a cancelled one included.
#[test]
fn answers_come_in_request_order() {
    let gate = Gate::default();
    let fake = Arc::new(Fake {
        gate: Some(gate.clone()),
        ..Fake::default()
    });
    let memory = FakeMemory::new();
    let mut helper = Running::start(Arc::clone(&memory), Arc::new(FakeEngine(Arc::clone(&fake))));
    opened(helper.open_fake(&memory));

    helper.send(2, &render_into(memory.zeroed(FAKE_PAGE_BYTES)));
    helper.send(4, &Message::Stats(Stats));
    helper.send(6, &Message::Outline(Outline { from: 0 }));
    helper.send(8, &render_into(memory.zeroed(FAKE_PAGE_BYTES)));
    helper.send(10, &Message::Stats(Stats));
    helper.send(8, &Message::Cancel(Cancel));
    helper.wait_handed_out();
    thread::sleep(Duration::from_millis(50));
    gate.open();

    let mut ids = Vec::new();
    let mut kinds = Vec::new();
    for _ in 0..5 {
        let (id, answer) = helper.answer();
        ids.push(id);
        kinds.push(answer.kind());
    }
    assert_eq!(ids, [2, 4, 6, 8, 10], "one answer per request, in request order");
    assert_eq!(
        kinds,
        [
            office::KIND_RENDERED,
            office::KIND_STATS_DATA,
            office::KIND_OUTLINE_DATA,
            office::KIND_ERROR,
            office::KIND_STATS_DATA
        ]
    );
    assert_eq!(fake.renders(), 1);

    helper.close();
    assert_eq!(helper.exit_code(), EXIT_OK);
    helper.nothing_more();
}

/// Rule 5: every mapping is dropped after its request, the document's right after `Open`; and every page buffer is
/// mapped exactly once, on an error and on a skipped request too, because mapping is the only way to close its grant.
#[test]
fn every_mapping_is_dropped_after_its_request() {
    let gate = Gate::default();
    let fake = Arc::new(Fake {
        gate: Some(gate.clone()),
        ..Fake::default()
    });
    let memory = FakeMemory::new();
    let mut helper = Running::start(Arc::clone(&memory), Arc::new(FakeEngine(fake)));

    let doc = memory.section(b"a document".to_vec());
    helper.send(
        0,
        &Message::Open(Open {
            kind: DocKind::Docx,
            doc,
        }),
    );
    assert!(matches!(helper.answer(), (0, Message::Opened(_))));
    assert_eq!(
        memory.live_reads(),
        0,
        "the document's mapping is gone once Opened is written"
    );
    assert_eq!(memory.maps(doc), 1);

    // A render, and a skipped one behind it.
    let drawn = memory.zeroed(FAKE_PAGE_BYTES);
    let skipped = memory.zeroed(FAKE_PAGE_BYTES);
    helper.send(2, &render_into(drawn));
    helper.send(4, &render_into(skipped));
    helper.send(4, &Message::Cancel(Cancel));
    helper.wait_handed_out();
    thread::sleep(Duration::from_millis(50));
    gate.open();
    assert!(matches!(helper.answer(), (2, Message::Rendered(_))));
    assert_eq!(memory.maps(drawn), 1);
    assert_eq!(helper.answer(), (4, error(ErrorCode::Internal)));
    assert_eq!(memory.live_writes(), 0, "no page mapping outlives its answer");
    assert_eq!(
        memory.maps(skipped),
        1,
        "the skipped request's buffer was mapped once, to close it"
    );

    // An error: a buffer of the wrong length.
    let short = memory.zeroed(FAKE_PAGE_BYTES - 1);
    helper.send(6, &render_into(short));
    assert_eq!(helper.answer(), (6, error(ErrorCode::Internal)));
    assert_eq!(memory.live_writes(), 0, "no page mapping outlives its error");
    assert_eq!(memory.maps(short), 1, "the refused buffer was mapped once, to close it");

    // Another render after all that.
    let again = memory.zeroed(FAKE_PAGE_BYTES);
    helper.send(8, &render_into(again));
    assert!(matches!(helper.answer(), (8, Message::Rendered(_))));
    assert_eq!(memory.live_writes(), 0);
    assert_eq!(memory.live_reads(), 0);
    assert_eq!(memory.maps(again), 1);

    helper.close();
    assert_eq!(helper.exit_code(), EXIT_OK);
}

/// Rule 6: a frame the helper cannot trust ends it with `EXIT_PROTOCOL` and no answer.
#[test]
fn a_bad_frame_ends_the_helper() {
    let cases: [&str; 8] = [
        "an unknown kind",
        "a payload that does not decode",
        "an odd id",
        "an answer kind from the app",
        "a request before Open",
        "a second Open",
        "a truncated frame",
        "an over-long frame",
    ];
    let mut ended = 0;
    for case in cases {
        let memory = FakeMemory::new();
        let fake = Arc::new(Fake::default());
        let mut helper = Running::start(Arc::clone(&memory), Arc::new(FakeEngine(Arc::clone(&fake))));
        if case != "a request before Open" {
            opened(helper.open_fake(&memory));
        }
        match case {
            "an unknown kind" => helper.send_frame(0x0199, 2, &[]),
            "a payload that does not decode" => helper.send_frame(KIND_RENDER, 2, &[1, 2, 3]),
            "an odd id" => helper.send(3, &Message::Stats(Stats)),
            "an answer kind from the app" => helper.send(
                2,
                &Message::Rendered(Rendered {
                    w: 1,
                    h: 1,
                    missing_content: false,
                }),
            ),
            "a request before Open" => helper.send(0, &Message::Stats(Stats)),
            "a second Open" => {
                let doc = memory.section(b"another document".to_vec());
                helper.send(
                    2,
                    &Message::Open(Open {
                        kind: DocKind::Docx,
                        doc,
                    }),
                );
            }
            "a truncated frame" => {
                let mut whole = Vec::new();
                write_frame(&mut whole, KIND_STATS, 2, &[]).expect("a frame in memory");
                helper.send_raw(&whole[..whole.len() - 3]);
            }
            "an over-long frame" => {
                let declared = (6 + MAX_PAYLOAD + 1) as u32;
                helper.send_raw(&declared.to_le_bytes());
            }
            _ => unreachable!(),
        }
        // The end of the input follows the bad frame: a helper that read on would end with EXIT_OK instead.
        helper.close();
        assert_eq!(helper.exit_code(), EXIT_PROTOCOL, "{case}");
        helper.nothing_more();
        assert_eq!(fake.renders(), 0, "{case}");
        ended += 1;
    }
    assert_eq!(ended, 8, "every case ran");
}

/// Rule 6: the end of the input ends the helper with `EXIT_OK`, before `Open` or after it.
#[test]
fn end_of_input_ends_the_helper_cleanly() {
    let memory = FakeMemory::new();
    let mut helper = Running::start(Arc::clone(&memory), Arc::new(FakeEngine(Arc::new(Fake::default()))));
    helper.close();
    assert_eq!(helper.exit_code(), EXIT_OK, "closed before Open");
    helper.nothing_more();

    let mut helper = Running::start(Arc::clone(&memory), Arc::new(FakeEngine(Arc::new(Fake::default()))));
    opened(helper.open_fake(&memory));
    helper.send(2, &Message::Stats(Stats));
    assert_eq!(helper.answer(), (2, Message::StatsData(office::StatsData { words: 7 })));
    helper.close();
    assert_eq!(helper.exit_code(), EXIT_OK, "closed after a request");
    helper.nothing_more();
}

/// Open errors travel as their fixed codes, and the helper ends once the answer is written.
#[test]
fn open_errors_map_to_their_codes() {
    // Each of the engine's open errors, through the helper's own mapping.
    let engine_errors = [
        (OpenError::Damaged, ErrorCode::Damaged),
        (OpenError::TooLarge(limits::UNPACKED), ErrorCode::TooLarge),
        (OpenError::Unsupported, ErrorCode::Unsupported),
        (OpenError::Password, ErrorCode::Password),
    ];
    let mut codes: Vec<(ErrorCode, ErrorCode)> = engine_errors.iter().map(|(e, c)| (open_code(*e), *c)).collect();
    // And a code the engine side gives as it is.
    codes.push((ErrorCode::Internal, ErrorCode::Internal));
    for (given, expected) in codes {
        let memory = FakeMemory::new();
        let fake = Arc::new(Fake {
            opening: Err(given),
            ..Fake::default()
        });
        let mut helper = Running::start(Arc::clone(&memory), Arc::new(FakeEngine(fake)));
        assert_eq!(helper.open_fake(&memory), error(expected), "{given:?}");
        assert_eq!(helper.exit_code(), EXIT_OK, "the helper ends after a failed Open");
        helper.nothing_more();
        assert_eq!(memory.live_reads(), 0);
    }

    // The real engine. EXPECTED RED at the S0 pin (its workbooks and decks open nothing yet: Unsupported) and GREEN
    // once O-CP3 moves the pin to the merged sheets and slides head (first seen there): our own workbook and deck open;
    // bytes that are no file at all are Damaged (helper decision 10); a workbook package handed as `.xls` is not a
    // compound file, so the engine's content check refuses it as Unsupported (no `.xls` writer here: stdlib only).
    let not_a_file: Vec<u8> = (0..64u32).map(|i| (i * 37 + 11) as u8).collect();
    let real = [
        (DocKind::Docx, not_a_file.as_slice(), Some(ErrorCode::Damaged)),
        (DocKind::Xlsx, not_a_file.as_slice(), Some(ErrorCode::Damaged)),
        (DocKind::Pptx, not_a_file.as_slice(), Some(ErrorCode::Damaged)),
        (DocKind::Xls, SHEETS, Some(ErrorCode::Unsupported)),
        (DocKind::Pptx, DECK, None),
        (DocKind::Xlsx, SHEETS, None),
    ];
    for (kind, bytes, expected) in real {
        let memory = FakeMemory::new();
        let (mut helper, answer) = open_real(&memory, kind, bytes);
        match (expected, answer) {
            (Some(code), answer) => {
                assert_eq!(answer, error(code), "{kind:?}");
                assert_eq!(helper.exit_code(), EXIT_OK);
            }
            (None, Message::Opened(opened)) if kind == DocKind::Pptx => {
                check_opened(&opened, &Limits::CONTRACT).expect("the app's check accepts the Opened");
                assert_eq!(opened.pages, 3, "deck-3.pptx has three slides");
                helper.close();
                assert_eq!(helper.exit_code(), EXIT_OK);
            }
            (None, Message::OpenedSheets(sheets)) if kind == DocKind::Xlsx => {
                check_sheets(&sheets, &Limits::CONTRACT).expect("the app's check accepts the OpenedSheets");
                assert_eq!(sheets.sheets.len(), 3, "sheets-3.xlsx has three sheets");
                helper.close();
                assert_eq!(helper.exit_code(), EXIT_OK);
            }
            (None, other) => panic!("{kind:?}: an open document was due: {other:?}"),
        }
    }

    // Over the app's 100 MiB input limit: TooLarge, and nothing is mapped.
    let memory = FakeMemory::new();
    let mut helper = Running::start(Arc::clone(&memory), Arc::new(Office));
    let too_large = GrantRef {
        wire: 4_096,
        len: MAX_INPUT_BYTES + 1,
    };
    helper.send(
        0,
        &Message::Open(Open {
            kind: DocKind::Docx,
            doc: too_large,
        }),
    );
    assert_eq!(helper.answer(), (0, error(ErrorCode::TooLarge)));
    assert_eq!(helper.exit_code(), EXIT_OK);
    assert_eq!(memory.all_maps(), 0, "no mapping was made");
}

/// A buffer whose length is not the engine's page size is an error, never a render into part of it.
#[test]
fn a_page_size_mismatch_is_an_error_not_a_guess() {
    let fake = Arc::new(Fake::default());
    let memory = FakeMemory::new();
    let mut helper = Running::start(Arc::clone(&memory), Arc::new(FakeEngine(Arc::clone(&fake))));
    opened(helper.open_fake(&memory));

    // One row short: 4 px of 4 bytes.
    let short = memory.zeroed(FAKE_PAGE_BYTES - 16);
    helper.send(2, &render_into(short));
    assert_eq!(helper.answer(), (2, error(ErrorCode::Internal)));
    assert_eq!(fake.renders(), 0, "the engine was not asked to render");
    assert_eq!(
        memory.bytes(short),
        vec![0; FAKE_PAGE_BYTES - 16],
        "nothing was written"
    );

    // The same page into a buffer of the right size renders.
    let right = memory.zeroed(FAKE_PAGE_BYTES);
    helper.send(4, &render_into(right));
    assert!(matches!(helper.answer(), (4, Message::Rendered(_))));
    assert_eq!(fake.renders(), 1);

    helper.close();
    assert_eq!(helper.exit_code(), EXIT_OK);
}

/// A picture the document only links to is reported as missing content, and nothing is fetched.
#[test]
fn a_remote_picture_says_missing_content_and_fetches_nothing() {
    let listener = TcpListener::bind(("127.0.0.1", REMOTE_PORT)).ok();
    if let Some(listener) = &listener {
        listener.set_nonblocking(true).expect("a non-blocking listener");
    }

    let memory = FakeMemory::new();
    let (mut helper, answer) = open_real(&memory, DocKind::Docx, REMOTE);
    let opened = opened(answer);
    let ask = whole_page(render_scale(1.0, 96.0));
    let (w, h) = asked_size(&ask, &opened.sizes[0], &Limits::CONTRACT).expect("the app can size the page");
    let out = memory.zeroed(w as usize * h as usize * 4);
    helper.send(2, &Message::Render(Render { out, ..ask }));
    match helper.answer() {
        (2, Message::Rendered(rendered)) => {
            assert!(
                rendered.missing_content,
                "the linked picture is reported as content not shown"
            );
        }
        other => panic!("Rendered for 2 was due: {other:?}"),
    }
    helper.close();
    assert_eq!(helper.exit_code(), EXIT_OK);

    let Some(listener) = listener else {
        println!(
            "SKIPPED a_remote_picture_says_missing_content_and_fetches_nothing: port {REMOTE_PORT} busy; the connection count was not taken"
        );
        return;
    };
    // A late connection would still be queued: give it a moment, then count.
    thread::sleep(Duration::from_millis(100));
    let mut connections = 0;
    loop {
        match listener.accept() {
            Ok(_) => connections += 1,
            Err(e) if e.kind() == ErrorKind::WouldBlock => break,
            Err(e) => panic!("the listener failed: {e}"),
        }
    }
    assert_eq!(connections, 0, "the linked picture was fetched");
}
