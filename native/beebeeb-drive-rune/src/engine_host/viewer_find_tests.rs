//! Find in a PDF (DW-1506) on PDFs built here (Helvetica, which the file names and does not carry; a few objects, an
//! xref table). Every word is invented. Run at the checkpoint, by filter `find`.

use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use zeroize::Zeroizing;

use super::*;
use crate::engine_host::viewer::viewer_pdf::tests::{sample_pdf, session};
use crate::engine_host::viewer::viewer_pdf::{Ask, Document};
use crate::frames::viewers::source::Match;

/// A Letter PDF, one page per entry: the line of text it writes at 72 / 700 in 12 pt Helvetica; an empty entry is a
/// page that only fills a rectangle (a scan has no text, only a picture: this is that page without the picture).
fn text_pdf(pages: &[&str]) -> Vec<u8> {
    text_pdf_on((612, 792), (72, 700), pages)
}

/// [`text_pdf`] on pages of `size` points (width, height), each line written at `at` (x, its baseline's height above
/// the bottom of the page, as a PDF counts it).
fn text_pdf_on(size: (u32, u32), at: (u32, u32), pages: &[&str]) -> Vec<u8> {
    const FIRST_PAGE: usize = 4;
    let (width, height) = size;
    let (x, baseline) = at;
    let kids: Vec<String> = (0..pages.len())
        .map(|i| format!("{} 0 R", FIRST_PAGE + 2 * i))
        .collect();
    let mut objects: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        format!("<< /Type /Pages /Kids [{}] /Count {} >>", kids.join(" "), pages.len()),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".into(),
    ];
    for (i, line) in pages.iter().enumerate() {
        objects.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {width} {height}] /Resources << /Font << /F1 3 0 R >> >> \
             /Contents {} 0 R >>",
            FIRST_PAGE + 1 + 2 * i
        ));
        let content = if line.is_empty() {
            "0 0 0 rg\n0 0 306 396 re\nf\n".to_string()
        } else {
            format!("BT /F1 12 Tf {x} {baseline} Td ({line}) Tj ET\n")
        };
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

fn open(bytes: Vec<u8>) -> Document {
    Document::open(Zeroizing::new(bytes)).expect("the PDF opens")
}

fn found(document: &Document, words: &str) -> Vec<Match> {
    document.find(words, &AtomicBool::new(false))
}

#[test]
fn find_matches_words_on_the_right_page() {
    let document = open(text_pdf(&["Invented harbour ledger", "Second page tide table"]));
    let matches = found(&document, "tide");
    assert_eq!(matches.len(), 1, "one match: {matches:?}");
    let m = matches[0];
    assert_eq!(m.page, 2);
    // The line is written at 72 pt from the left, its baseline 92 pt from the top (792 - 700): the box is on it, in
    // the page's own points with the top left corner the origin, one line of 12 pt Helvetica high.
    assert!(m.x > 72.0 && m.x < 200.0, "x {m:?}");
    assert!(m.y > 78.0 && m.y < 96.0, "y {m:?}");
    assert!(m.w > 5.0 && m.w < 60.0, "w {m:?}");
    assert!(m.h > 8.0 && m.h < 16.0, "h {m:?}");
    // Words on another page are found there, and several words that are together are one match.
    let ledger = found(&document, "harbour ledger");
    assert_eq!(ledger.iter().map(|m| m.page).collect::<Vec<_>>(), [1]);
    // Nowhere: nothing.
    assert!(found(&document, "invented harbour tide").is_empty());
    assert!(found(&document, "   ").is_empty());
}

#[test]
fn find_is_case_insensitive() {
    let document = open(text_pdf(&["Invented Harbour ledger", "second page: HARBOUR tide"]));
    let pages: Vec<u32> = found(&document, "hArBoUr").iter().map(|m| m.page).collect();
    assert_eq!(pages, [1, 2], "every page, in page order");
    assert_eq!(found(&document, "HARBOUR").len(), 2);
}

#[test]
fn find_on_a_scanned_page_finds_nothing() {
    // Page 1 has no text at all (the rectangle stands for the scan's picture), page 2 has.
    let document = open(text_pdf(&["", "Second page tide table"]));
    let matches = found(&document, "page");
    assert_eq!(matches.iter().map(|m| m.page).collect::<Vec<_>>(), [2]);
    // A document with no text on any page: nothing, never a panic.
    let blank = open(sample_pdf(2, 612, 792));
    assert!(found(&blank, "tide").is_empty());
}

#[test]
fn finds_false_for_non_pdf() {
    // The opening of a text, a picture or a table holds no pages: it cannot be searched, and the bar says so.
    assert!(!pages_search(None));
    // A PDF's opening holds its pages from the open on.
    let (pages, _ends) = Pages::open("1 kB".to_string());
    assert!(pages_search(Some(&pages)));
}

#[test]
fn a_find_asked_down_the_channel_is_answered_under_its_serial() {
    // The session reads the text at this first find (its pages are the helper's; a stand-in here).
    let session = session(text_pdf(&["Invented harbour ledger"])).expect("the PDF opens");
    let (ask, asks) = mpsc::channel();
    ask.send(Ask::Find {
        serial: 5,
        words: "harbour".into(),
    })
    .unwrap();
    drop(ask);
    let answers = Mutex::new(Vec::new());
    let found = |serial, matches: Vec<Match>| answers.lock().unwrap().push((serial, matches.len()));
    session.serve(&asks, &AtomicBool::new(false), &|_, _| {}, &found, &|| {});
    assert_eq!(answers.into_inner().unwrap(), [(5, 1)]);
}

#[test]
fn a_find_answered_after_the_document_closed_is_dropped() {
    let (pages, ends) = Pages::open("1 kB".to_string());
    let here = Match {
        page: 1,
        x: 1.0,
        y: 1.0,
        w: 1.0,
        h: 1.0,
    };
    // An answer for a search that is not the one waited for (an earlier one) is dropped, not taken for this one.
    ends.found.send((41, vec![here])).unwrap();
    assert_eq!(pages.await_found(1, Duration::from_millis(30)), None);
    // The right serial is taken.
    ends.found.send((1, vec![here])).unwrap();
    assert_eq!(pages.await_found(1, Duration::from_millis(30)), Some(vec![here]));
    // The document closes (the opening drops its side): a worker that answers now has nowhere to send it.
    drop(pages);
    assert!(ends.found.send((2, vec![here])).is_err(), "nobody reads it any more");

    // And a worker whose session ended (cancel) does not deliver the find it was running: positive control first.
    for (cancelled, expected) in [(false, 1), (true, 0)] {
        let session = session(text_pdf(&["Invented harbour ledger"])).expect("the PDF opens");
        let (ask, asks) = mpsc::channel();
        ask.send(Ask::Find {
            serial: 9,
            words: "harbour".into(),
        })
        .unwrap();
        drop(ask);
        let delivered = Mutex::new(0);
        let found = |_, _| *delivered.lock().unwrap() += 1;
        session.serve(&asks, &AtomicBool::new(cancelled), &|_, _| {}, &found, &|| {});
        assert_eq!(delivered.into_inner().unwrap(), expected, "cancelled: {cancelled}");
    }
}

#[test]
fn find_puts_the_box_on_the_words_on_an_a4_page() {
    // An A4 page (595 x 842 pt) whose line is written low: its baseline 120 pt above the bottom, so 722 pt from the
    // top. The box is in the page's own points with the top left corner the origin, one line of 12 pt Helvetica high,
    // its top above the baseline by at most that line. Where a box would land if the page were not turned top-down
    // (y about 118) or turned as if it were Letter (y about 662) is outside that band.
    const HEIGHT: f32 = 842.0;
    const BASELINE: f32 = 120.0;
    let document = open(text_pdf_on((595, 842), (72, 120), &["Invented quay tide table"]));
    let matches = found(&document, "tide");
    assert_eq!(matches.len(), 1, "one match: {matches:?}");
    let m = matches[0];
    assert_eq!(m.page, 1);
    let baseline_from_top = HEIGHT - BASELINE;
    assert!(m.y > baseline_from_top - 16.0 && m.y < baseline_from_top, "y {m:?}");
    assert!(
        m.y + m.h > baseline_from_top,
        "the box reaches below the baseline: {m:?}"
    );
    assert!(m.x > 72.0 && m.x < 300.0, "x {m:?}");
    assert!(m.w > 5.0 && m.w < 60.0, "w {m:?}");
    assert!(m.h > 8.0 && m.h < 16.0, "h {m:?}");
}
