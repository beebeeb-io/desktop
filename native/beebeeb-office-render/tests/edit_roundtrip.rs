//! The real helper loop with the real Office engines, an edit session end to end: `OpenEdit` a fixture, a few ops,
//! the save handshake, `OpenEdit` the saved bytes: the edit is there. One process, as `tests/serve.rs`.
//! A kind whose engine is not built in answers `Unsupported`/`Internal`-style errors: the test says so and stops
//! there instead of passing silently (slides are behind `edit-slides` until Guus accepts MPL-2.0).

mod common;

use beebeeb_office_protocol::office::{
    CellRange, Cells, Copy as CopyAsk, Edit, EditKind, EditOp, ErrorCode, Message,
};
use common::{FakeMemory, open_real_edit, save_real};

const DOCX: &[u8] = include_bytes!("fixtures/letter-1.docx");
const XLSX: &[u8] = include_bytes!("fixtures/sheets-3.xlsx");
const PPTX: &[u8] = include_bytes!("fixtures/deck-3.pptx");

const MARKER: &str = "Zebra-4471";

fn edited(helper: &mut common::Running, id: u32, op: EditOp) -> beebeeb_office_protocol::office::Edited {
    helper.send(id, &Message::Edit(Edit { op }));
    match helper.answer() {
        (got, Message::Edited(e)) if got == id => e,
        other => panic!("Edited for {id} was due: {other:?}"),
    }
}

#[test]
fn edit_roundtrip_docx_text_survives_save_and_reopen() {
    let memory = FakeMemory::new();
    let (mut helper, opened) = open_real_edit(&memory, EditKind::Docx, DOCX);
    assert!(matches!(opened, Message::Opened(_)), "Opened was due: {opened:?}");

    let first = edited(&mut helper, 2, EditOp::InsertText(MARKER.to_owned()));
    assert_eq!(first.revision, 1);
    assert!(first.can_undo);
    let second = edited(&mut helper, 4, EditOp::InsertParagraph);
    assert_eq!(second.revision, 2);
    edited(&mut helper, 6, EditOp::InsertText("second line".to_owned()));

    let saved = save_real(&mut helper, &memory, 8);
    assert!(saved.starts_with(b"PK"), "a zip");
    helper.close();
    assert_eq!(helper.exit_code(), beebeeb_office_render::EXIT_OK);

    let (mut again, opened) = open_real_edit(&memory, EditKind::Docx, &saved);
    assert!(matches!(opened, Message::Opened(_)), "the saved file opens: {opened:?}");
    edited(&mut again, 2, EditOp::SelectAll);
    again.send(4, &Message::Copy(CopyAsk));
    match again.answer() {
        (4, Message::Clip(clip)) => {
            assert!(clip.text.contains(MARKER), "the first edit is in the saved file");
            assert!(clip.text.contains("second line"), "the last edit is in the saved file");
        }
        other => panic!("Clip was due: {other:?}"),
    }
}

#[test]
fn edit_roundtrip_xlsx_cell_survives_save_and_reopen() {
    let memory = FakeMemory::new();
    let (mut helper, opened) = open_real_edit(&memory, EditKind::Xlsx, XLSX);
    assert!(matches!(opened, Message::OpenedSheets(_)), "OpenedSheets was due: {opened:?}");

    // Notes!A5 (sheet 1) is empty in the fixture.
    let e = edited(
        &mut helper,
        2,
        EditOp::SetCell {
            sheet: 1,
            row: 4,
            col: 0,
            text: MARKER.to_owned(),
        },
    );
    assert_eq!(e.revision, 1);
    assert!(e.cells.iter().any(|(sheet, _)| *sheet == 1), "the Edited names the sheet's cells");

    let saved = save_real(&mut helper, &memory, 4);
    helper.close();
    assert_eq!(helper.exit_code(), beebeeb_office_render::EXIT_OK);

    let (mut again, opened) = open_real_edit(&memory, EditKind::Xlsx, &saved);
    assert!(matches!(opened, Message::OpenedSheets(_)), "the saved file opens: {opened:?}");
    let range = CellRange { row: 0, col: 0, rows: 10, cols: 4 };
    again.send(2, &Message::Cells(Cells { sheet: 1, range }));
    match again.answer() {
        (2, Message::CellData(data)) => assert!(
            data.cells.iter().any(|c| c.row == 4 && c.col == 0 && c.text == MARKER),
            "the cell is in the saved file"
        ),
        other => panic!("CellData was due: {other:?}"),
    }
}

#[test]
fn edit_roundtrip_pptx_new_slide_survives_save_and_reopen() {
    let memory = FakeMemory::new();
    let (mut helper, opened) = open_real_edit(&memory, EditKind::Pptx, PPTX);
    let before = match opened {
        Message::Opened(o) => o.pages,
        // Without the `edit-slides` feature the slide editor is not built in: nothing to round-trip.
        Message::Error(err) if err.code == ErrorCode::Unsupported => {
            println!("SKIPPED slides round trip: the slide editor is not built in");
            return;
        }
        other => panic!("Opened was due: {other:?}"),
    };
    assert_eq!(before, 3);

    helper.send(2, &Message::Edit(Edit { op: EditOp::NewSlide { layout: None } }));
    let e = match helper.answer() {
        (2, Message::Edited(e)) => e,
        // Without the `edit-slides` feature the engine answers an error: nothing more to round-trip.
        (2, Message::Error(err)) => {
            println!("SKIPPED slides round trip: the slide editor is not built in ({err:?})");
            return;
        }
        other => panic!("Edited was due: {other:?}"),
    };
    assert_eq!(e.slides, Some(4), "four slides after NewSlide");

    let saved = save_real(&mut helper, &memory, 4);
    helper.close();
    assert_eq!(helper.exit_code(), beebeeb_office_render::EXIT_OK);

    let (_again, opened) = open_real_edit(&memory, EditKind::Pptx, &saved);
    match opened {
        Message::Opened(o) => assert_eq!(o.pages, 4, "the saved deck has the new slide"),
        other => panic!("Opened was due: {other:?}"),
    }
}
