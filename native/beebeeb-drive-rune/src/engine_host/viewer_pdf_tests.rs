//! The PDF page worker's rules on PDFs built here (a few objects, an xref table, one filled rectangle a page):
//! what opens, what a page renders to, the limits asked before rendering, and when the worker stops.

use std::sync::Mutex;

use super::*;

/// A PDF of `pages` pages of `width` x `height` points; on each, the lower-left quarter is filled black.
pub(crate) fn sample_pdf(pages: usize, width: u32, height: u32) -> Vec<u8> {
    let mut objects: Vec<String> = Vec::new();
    let kids: Vec<String> = (0..pages).map(|i| format!("{} 0 R", 3 + 2 * i)).collect();
    objects.push("<< /Type /Catalog /Pages 2 0 R >>".into());
    objects.push(format!("<< /Type /Pages /Kids [{}] /Count {pages} >>", kids.join(" ")));
    for i in 0..pages {
        objects.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {width} {height}] /Resources << >> /Contents {} 0 R >>",
            4 + 2 * i
        ));
        let content = format!("0 0 0 rg\n0 0 {} {} re\nf\n", width / 2, height / 2);
        objects.push(format!("<< /Length {} >>\nstream\n{content}endstream", content.len()));
    }
    file_of(&objects)
}

/// What a page of [`fonts_pdf`] writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Writes {
    /// A filled rectangle, no text.
    Nothing,
    /// A line in a Latin font the file names and does not carry (one of the standard 14, and one that is not).
    Latin,
    /// A line in a Japanese font the file names and does not carry.
    Japanese,
    /// A line in a font the file names and does not carry, written by glyph number (no script is named).
    ByGlyphNumber,
    /// A line in a Japanese font the file names and does not carry, whose NAME holds "Sans": the renderer asks
    /// for it as it asks for Helvetica.
    JapaneseSans,
}

/// A Letter PDF with one page per entry of `pages`. Every page lists the same four fonts (none is in the file):
/// what differs is which one its content uses.
pub(crate) fn fonts_pdf(pages: &[Writes]) -> Vec<u8> {
    const FIRST_PAGE: usize = 13;
    let kids: Vec<String> = (0..pages.len())
        .map(|i| format!("{} 0 R", FIRST_PAGE + 2 * i))
        .collect();
    let mut objects: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        format!("<< /Type /Pages /Kids [{}] /Count {} >>", kids.join(" "), pages.len()),
        "<< /Type /Font /Subtype /Type0 /BaseFont /KozMinPr6N-Regular /Encoding /UniJIS-UCS2-H \
         /DescendantFonts [4 0 R] >>"
            .into(),
        "<< /Type /Font /Subtype /CIDFontType0 /BaseFont /KozMinPr6N-Regular \
         /CIDSystemInfo << /Registry (Adobe) /Ordering (Japan1) /Supplement 6 >> /FontDescriptor 5 0 R >>"
            .into(),
        "<< /Type /FontDescriptor /FontName /KozMinPr6N-Regular /Flags 6 /FontBBox [0 -120 1000 880] \
         /ItalicAngle 0 /Ascent 880 /Descent -120 /CapHeight 700 /StemV 80 >>"
            .into(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Garamond /FirstChar 32 /LastChar 32 /Widths [250] \
         /FontDescriptor 8 0 R >>"
            .into(),
        "<< /Type /FontDescriptor /FontName /Garamond /Flags 34 /FontBBox [0 -200 1000 800] \
         /ItalicAngle 0 /Ascent 800 /Descent -200 /CapHeight 700 /StemV 80 >>"
            .into(),
        "<< /Type /Font /Subtype /Type0 /BaseFont /Invented-Grotesk /Encoding /Identity-H \
         /DescendantFonts [10 0 R] >>"
            .into(),
        "<< /Type /Font /Subtype /CIDFontType2 /BaseFont /Invented-Grotesk \
         /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /FontDescriptor 8 0 R >>"
            .into(),
        "<< /Type /Font /Subtype /Type0 /BaseFont /NotoSansCJKjp-Regular /Encoding /UniJIS-UCS2-H \
         /DescendantFonts [12 0 R] >>"
            .into(),
        "<< /Type /Font /Subtype /CIDFontType0 /BaseFont /NotoSansCJKjp-Regular \
         /CIDSystemInfo << /Registry (Adobe) /Ordering (Japan1) /Supplement 6 >> /FontDescriptor 5 0 R >>"
            .into(),
    ];
    for (i, writes) in pages.iter().enumerate() {
        objects.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] \
             /Resources << /Font << /F1 3 0 R /F2 6 0 R /F3 7 0 R /F4 9 0 R /F5 11 0 R >> >> /Contents {} 0 R >>",
            FIRST_PAGE + 2 * i + 1
        ));
        let content = match writes {
            Writes::Nothing => "0 0 0 rg\n0 0 306 396 re\nf\n".to_string(),
            Writes::Latin => "BT /F2 24 Tf 40 600 Td (Invented) Tj /F3 24 Tf 0 -40 Td (words) Tj ET\n".to_string(),
            Writes::Japanese => "BT /F1 24 Tf 40 600 Td <65E5672C8A9E> Tj ET\n".to_string(),
            Writes::JapaneseSans => "BT /F5 24 Tf 40 600 Td <65E5672C8A9E> Tj ET\n".to_string(),
            Writes::ByGlyphNumber => "BT /F4 24 Tf 40 600 Td <002B004800580059> Tj ET\n".to_string(),
        };
        objects.push(format!("<< /Length {} >>\nstream\n{content}endstream", content.len()));
    }
    file_of(&objects)
}

/// The file of `objects` (numbered from 1, the first is the catalog), with its xref table.
fn file_of(objects: &[String]) -> Vec<u8> {
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, object) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for offset in offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
}

fn open(bytes: Vec<u8>) -> Result<Document, NotOpened> {
    Document::open(Zeroizing::new(bytes))
}

/// Ask for `pages` (all waiting before the worker starts), then let the asker go: what was delivered.
fn served(document: &Document, pages: &[u32], cancel: bool) -> Vec<(u32, Option<Rendered>)> {
    let (ask, asks) = mpsc::channel();
    for page in pages {
        ask.send(*page).unwrap();
    }
    drop(ask);
    let delivered = Mutex::new(Vec::new());
    document.serve(&asks, &AtomicBool::new(cancel), &|page, rendered| {
        delivered.lock().unwrap().push((page, rendered));
    });
    delivered.into_inner().unwrap()
}

/// Ask for `pages` one after the other, each when the one before was answered (a person paging): per page asked,
/// whether it rendered and whether it lacks text.
fn served_in_turn(document: &Document, pages: &[u32]) -> Vec<(u32, bool, bool)> {
    let (ask, asks) = mpsc::channel();
    let mut rest = pages.iter().copied();
    ask.send(rest.next().expect("a page to ask for")).unwrap();
    let asker = Mutex::new((Some(ask), rest));
    let delivered = Mutex::new(Vec::new());
    document.serve(&asks, &AtomicBool::new(false), &|page, rendered| {
        let lacks_text = rendered.as_ref().is_some_and(|rendered| rendered.lacks_text);
        delivered.lock().unwrap().push((page, rendered.is_some(), lacks_text));
        let mut asker = asker.lock().unwrap();
        match asker.1.next() {
            Some(next) => asker.0.as_ref().expect("the asker").send(next).unwrap(),
            // The asker goes: `serve` returns.
            None => asker.0 = None,
        }
    });
    delivered.into_inner().unwrap()
}

fn pixel(page: &Rendered, x: u32, y: u32) -> [u8; 4] {
    let at = ((y * page.width + x) * 4) as usize;
    page.rgba[at..at + 4].try_into().unwrap()
}

#[test]
fn a_pdf_opens_from_memory_and_a_page_renders_to_its_pixels() {
    let document = open(sample_pdf(3, 612, 792)).expect("it opens");
    assert_eq!(document.pages(), 3);
    let mut delivered = served(&document, &[2], false);
    assert_eq!(delivered.len(), 1);
    let (page, rendered) = delivered.pop().unwrap();
    let rendered = rendered.expect("page 2 rendered");
    assert_eq!(page, 2);
    // A Letter page at the viewer's width, in its own proportions.
    assert!((1_259..=1_260).contains(&rendered.width), "{rendered:?}");
    assert!((1_629..=1_631).contains(&rendered.height), "{rendered:?}");
    assert_eq!(rendered.rgba.len(), (rendered.width * rendered.height * 4) as usize);
    // The lower-left quarter is black, the rest the white page: opaque either way.
    let (w, h) = (rendered.width, rendered.height);
    assert_eq!(pixel(&rendered, w / 4, h * 3 / 4), [0, 0, 0, 255]);
    assert_eq!(pixel(&rendered, w * 3 / 4, h / 4), [255, 255, 255, 255]);
    assert_eq!(pixel(&rendered, w * 3 / 4, h * 3 / 4), [255, 255, 255, 255]);
    // A `{:?}` of a page prints its size, never its pixels.
    assert_eq!(format!("{rendered:?}"), format!("Rendered({w}x{h})"));
}

#[test]
fn what_is_not_a_pdf_and_a_pdf_with_too_many_pages_are_not_opened() {
    assert_eq!(open(b"%PDF-1.7 invented".to_vec()).err(), Some(NotOpened::Unreadable));
    assert_eq!(open(b"plain words".to_vec()).err(), Some(NotOpened::Unreadable));
    assert_eq!(open(Vec::new()).err(), Some(NotOpened::Unreadable));
    assert!(
        open(sample_pdf(PAGES_MAX as usize, 100, 100)).is_ok(),
        "at the limit it opens"
    );
    assert_eq!(
        open(sample_pdf(PAGES_MAX as usize + 1, 100, 100)).err(),
        Some(NotOpened::TooLong { pages: PAGES_MAX + 1 })
    );
}

#[test]
fn a_page_is_never_rendered_past_the_pixel_limits_whatever_its_box_says() {
    // The scale, from the box alone.
    let within = |width: f32, height: f32| {
        let scale = scale_for(width, height).expect("a scale");
        let (w, h) = (width * scale, height * scale);
        assert!(w <= PAGE_WIDTH_PX + 0.5, "{width} x {height}: {w}");
        assert!(w * h <= PAGE_PIXELS_MAX * 1.001, "{width} x {height}: {}", w * h);
        assert!(w.max(h) <= PAGE_SIDE_MAX + 0.5, "{width} x {height}");
        scale
    };
    assert_eq!(
        within(612.0, 792.0),
        PAGE_WIDTH_PX / 612.0,
        "an ordinary page: by its width alone"
    );
    // A wall map, a receipt roll and a banner: each is held by another limit.
    assert!(within(4_000.0, 3_000.0) < PAGE_WIDTH_PX / 4_000.0 + f32::EPSILON);
    assert!(
        within(200.0, 14_400.0) < PAGE_WIDTH_PX / 200.0,
        "a tall page: by its pixels or its side"
    );
    assert!(within(100.0, 14_400.0) * 14_400.0 <= PAGE_SIDE_MAX + 0.5);
    // A page five and a half times as tall as wide: within the side limit at the viewer's width, and past the
    // pixel limit there, so the pixel limit alone holds it.
    let tall = within(200.0, 1_100.0);
    assert!(
        tall < PAGE_WIDTH_PX / 200.0 && 1_100.0 * tall < PAGE_SIDE_MAX - 1.0,
        "{tall}"
    );
    for (width, height) in [(0.0, 100.0), (100.0, 0.5), (f32::NAN, 10.0), (f32::INFINITY, 10.0)] {
        assert_eq!(scale_for(width, height), None, "{width} x {height}");
    }
    // And a real page with such a box renders within them.
    let document = open(sample_pdf(1, 200, 14_400)).unwrap();
    let rendered = served(&document, &[1], false).pop().unwrap().1.expect("it renders");
    assert!(u64::from(rendered.width) * u64::from(rendered.height) <= PAGE_PIXELS_MAX as u64);
    assert!(rendered.height <= PAGE_SIDE_MAX as u32);
}

#[test]
fn only_the_newest_page_waiting_is_rendered_and_a_page_that_is_not_there_is_said() {
    let document = open(sample_pdf(4, 300, 300)).unwrap();
    // Three asks waiting when the worker gets to them (the person paged on twice): one render.
    let delivered = served(&document, &[1, 2, 3], false);
    assert_eq!(delivered.iter().map(|(page, _)| *page).collect::<Vec<_>>(), [3]);
    assert!(delivered[0].1.is_some());
    // No page 0 and no page 5: answered as not rendered, not skipped (the view waits for an answer).
    for missing in [0, 5] {
        let delivered = served(&document, &[missing], false);
        assert_eq!(delivered.len(), 1, "{missing}");
        assert!(delivered[0].1.is_none(), "{missing}");
    }
}

#[test]
fn a_cancelled_worker_renders_and_delivers_nothing_and_ends_when_the_asker_goes() {
    let document = open(sample_pdf(2, 300, 300)).unwrap();
    assert!(served(&document, &[1], true).is_empty());
    // Not rendered and then held back: the renderer was never asked (a page can take seconds, and the document
    // is to leave memory when its session does).
    assert_eq!(document.renders.load(Ordering::SeqCst), 0);
    assert_eq!(served(&document, &[1], false).len(), 1);
    assert_eq!(document.renders.load(Ordering::SeqCst), 1);
    // Nothing asked, the asker gone: `serve` returns (this test would hang otherwise).
    assert!(served(&document, &[], false).is_empty());
}

#[test]
fn a_page_whose_text_is_in_a_font_the_file_does_not_carry_says_so_and_no_other_page_does() {
    use Writes::{Japanese, Latin, Nothing};
    let document = open(fonts_pdf(&[Nothing, Japanese, Japanese, Latin, Nothing])).expect("it opens");
    // Paged through and back, on one worker (one font cache): each page answers for itself, also the second
    // page in the same font, a page after them, and a page shown again.
    let delivered = served_in_turn(&document, &[1, 2, 3, 4, 5, 3, 1, 2]);
    assert_eq!(
        delivered,
        [
            (1, true, false),
            (2, true, true),
            (3, true, true),
            (4, true, false),
            (5, true, false),
            (3, true, true),
            (1, true, false),
            (2, true, true),
        ]
    );
    // A Latin font that is not in the file has a stand-in: nothing is said of it, on a worker of its own too.
    let latin = open(fonts_pdf(&[Latin])).expect("it opens");
    assert_eq!(served_in_turn(&latin, &[1]), [(1, true, false)]);
    // A font written by glyph number has none, whatever its script: said, and the Latin page after it is not.
    let numbered = open(fonts_pdf(&[Writes::ByGlyphNumber, Latin])).expect("it opens");
    assert_eq!(served_in_turn(&numbered, &[1, 2]), [(1, true, true), (2, true, false)]);
    // A Japanese font named "…Sans…": the renderer asks for it as for Helvetica, and the page's own content
    // says it. The Latin pages around it, which really ask for Helvetica, say nothing.
    let sans = open(fonts_pdf(&[Latin, Writes::JapaneseSans, Latin, Nothing])).expect("it opens");
    assert_eq!(
        served_in_turn(&sans, &[1, 2, 3, 4, 2]),
        [
            (1, true, false),
            (2, true, true),
            (3, true, false),
            (4, true, false),
            (2, true, true)
        ]
    );
    // A page that is not there lacks no text: it is not rendered.
    assert_eq!(served_in_turn(&document, &[2, 9]), [(2, true, true), (9, false, false)]);
}
