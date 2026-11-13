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
    // Nothing asked, the asker gone: `serve` returns (this test would hang otherwise).
    assert!(served(&document, &[], false).is_empty());
}
