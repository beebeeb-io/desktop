//! The board's deck for DW-1509 and DW-1561, "Lisbon trip recap.pptx": three slides, slide 2 on the stage. The
//! picture of slide 2 is the board's own canvas, exported by the goldens lane (`tests/fixtures/office/`); the other
//! two slides are drawn from their words (the board's own). Nothing here is a document of the
//! account: the fixture is the stand-in for the live host's answer at O-CP3.

use std::sync::OnceLock;

use rune::Pixels;

use super::{DeckBody, DeckView, SlidePage, SlideWords};
use crate::frames::viewers::source::{Body, Details, OpenFile};

/// The slide the board has on the stage.
pub const BOARD_SLIDE: u32 = 2;
pub const NAME: &str = "Lisbon trip recap.pptx";

/// The board's slide 2, light: 1280 x 720 (the dark export is the same slide: a slide is a page).
static CANVAS: &[u8] = include_bytes!("../../../tests/fixtures/office/DW-1509-canvas.png");

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

fn canvas() -> Pixels {
    static DECODED: OnceLock<Pixels> = OnceLock::new();
    DECODED.get_or_init(|| decode(CANVAS)).clone()
}

/// The board's `SLIDES` and `SLIDE_POINTS`: each slide's words (slides 1 and 2 carry the picture strip).
fn words(n: u32) -> Option<SlideWords> {
    let shot = [
        "212 photos over two days \u{00B7} Alfama, Bel\u{00E9}m, the harbour",
        "60 edited selects for the hotel",
        "A 90-second film",
    ];
    let (title, sub, points, picture): (&str, &str, Vec<&str>, bool) = match n {
        1 => (
            "Lisbon trip recap",
            "Northwind Studio \u{00B7} October 2026",
            vec![],
            true,
        ),
        2 => ("What we shot", "", shot.to_vec(), true),
        3 => (
            "Next steps",
            "",
            vec!["Deliver selects by 26 October", "Hotel review call on 29 October"],
            false,
        ),
        _ => return None,
    };
    Some(SlideWords {
        title: title.into(),
        sub: sub.into(),
        points: points.into_iter().map(String::from).collect(),
        picture,
    })
}

/// The source's answer for slide `n` (1-based): every slide has its words; only slide 2 has the board's picture.
pub fn page(n: u32) -> Option<SlidePage> {
    Some(SlidePage {
        pixels: (n == BOARD_SLIDE).then(canvas),
        words: words(n),
    })
}

/// The file the scenes open: the deck at slide 1 (the scene then goes to the board's slide).
pub fn file() -> OpenFile {
    OpenFile {
        id: "lisbon-trip-recap.pptx".into(),
        path: vec!["Files".into(), "Lisbon trip".into()],
        name: NAME.into(),
        facts: "PowerPoint presentation \u{00B7} 18.4 MB".into(),
        version: 5,
        body: Body::Deck(DeckBody::new(DeckView::of(3))),
        details: Details::default(),
        newer: None,
    }
}
