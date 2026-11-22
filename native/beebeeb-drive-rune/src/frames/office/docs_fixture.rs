//! The board's two Word files for the viewer: DW-1507 "Service agreement.docx" (four pages) and DW-1511
//! "Proposal 2012.doc" (one page, an older format). The picture of page 1 is the board's own canvas, exported by the
//! goldens lane (`tests/fixtures/office/`); the other pages have none yet and draw as blank paper (a fixture never
//! invents a page). The headings come from the editor's sample documents, so the two frames never disagree on a word.
//! Nothing here is a document of the account: the fixture stands in for the live host's answer at O-CP3.

use std::sync::OnceLock;

use rune::Pixels;

use super::{DocBody, DocView};
use crate::frames::viewers::office::board;
use crate::frames::viewers::source::{Body, Details, OpenFile};

/// The fixture source's ids of the two files (`viewers::source::board`).
pub const DOCX_ID: &str = "service-agreement-read.docx";
pub const DOC_ID: &str = "proposal-2012-read.doc";

/// A4 in points.
const A4: (f32, f32) = (595.0, 842.0);

/// Page 1 of each file, light: the dark export is the same page (a page is paper).
static DOCX_CANVAS: &[u8] = include_bytes!("../../../tests/fixtures/office/DW-1507-canvas.png");
static DOC_CANVAS: &[u8] = include_bytes!("../../../tests/fixtures/office/DW-1511-canvas.png");

fn decode(file: &[u8]) -> Pixels {
    let mut reader = png::Decoder::new(std::io::Cursor::new(file))
        .read_info()
        .expect("the canvas fixture is a PNG");
    let mut buf = vec![0; reader.output_buffer_size().expect("the canvas fixture is small")];
    let info = reader.next_frame(&mut buf).expect("the canvas fixture decodes");
    let count = (info.width * info.height) as usize;
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf[..count * 4].to_vec(),
        png::ColorType::Rgb => buf[..count * 3]
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        other => panic!("the canvas fixture is RGB or RGBA, not {other:?}"),
    };
    let pixels = Pixels::new();
    pixels.set(info.width, info.height, rgba);
    pixels
}

fn canvas(older: bool) -> Pixels {
    static DOCX: OnceLock<Pixels> = OnceLock::new();
    static DOC: OnceLock<Pixels> = OnceLock::new();
    if older {
        DOC.get_or_init(|| decode(DOC_CANVAS)).clone()
    } else {
        DOCX.get_or_init(|| decode(DOCX_CANVAS)).clone()
    }
}

/// The source's picture of page `n` (1-based) of the file: page 1 is the board's; the others are not drawn yet.
pub fn page(older: bool, n: u32) -> Option<Pixels> {
    (n == 1).then(|| canvas(older))
}

/// The headings of a sample document as the viewer's outline: the title (level 1) on page 1, section `i` (level 2)
/// on page `i` (the last page at most).
fn outline_of(headings: &[String], pages: u32) -> Vec<(u8, String, u32)> {
    headings
        .iter()
        .enumerate()
        .map(|(i, title)| {
            let level = if i == 0 { 1 } else { 2 };
            (level, title.clone(), (i as u32).clamp(1, pages.max(1)))
        })
        .collect()
}

/// DW-1507: four pages, 1,284 words, 100 %.
pub fn view() -> DocView {
    let doc = board::service_agreement();
    DocView {
        pages: 4,
        page_sizes_pt: vec![A4; 4],
        outline: outline_of(&doc.outline, 4),
        words: 1284,
        zoom: 100,
        older_format: false,
    }
}

/// DW-1511: one page, 96 words, an older format.
pub fn view_older() -> DocView {
    let doc = board::proposal();
    DocView {
        pages: 1,
        page_sizes_pt: vec![A4],
        outline: outline_of(&doc.outline, 1),
        words: 96,
        zoom: 100,
        older_format: true,
    }
}

/// The body the scenes open: the board marks the first section ("1. Scope of work"; "Who we are") in Outline.
fn body_of(view: DocView) -> DocBody {
    let mut body = DocBody::new(view, "English (UK)");
    body.at = 1;
    body
}

/// The file the scenes open (`older`: the .doc of DW-1511).
pub fn file(older: bool) -> OpenFile {
    if older {
        OpenFile {
            id: DOC_ID.into(),
            path: vec!["Files".into(), "Documents".into()],
            name: "Proposal 2012.doc".into(),
            facts: "Word 97\u{2013}2004 document \u{00B7} 220 KB".into(),
            version: 2,
            body: Body::Doc(body_of(view_older())),
            details: Details::default(),
            newer: None,
        }
    } else {
        OpenFile {
            id: DOCX_ID.into(),
            path: vec!["Files".into()],
            name: "Service agreement.docx".into(),
            facts: "Word document \u{00B7} 38 KB".into(),
            version: 7,
            body: Body::Doc(body_of(view())),
            details: Details::default(),
            newer: None,
        }
    }
}
