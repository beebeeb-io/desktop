//! The PDF page worker's rules on PDFs built here (a few objects, an xref table, one filled rectangle a page): what
//! opens, what a page asked of the helper comes back as, the limits asked before rendering, and when the worker
//! stops. No test starts a process: the helper is stood in for ([`stand_in`], which draws with the reader find
//! uses) or scripted ([`Script`]).

use std::cell::RefCell;
use std::sync::Mutex;

use hayro::hayro_interpret::InterpreterSettings;
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::{PixmapSettings, RenderCache, RenderSettings};

use crate::engine_host::office_host::{NoPreview as HostNoPreview, Stop};

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

/// Put at the end of a PDF's bytes: the stand-in "crashes" at the first page asked of it, as a helper does on a file
/// made to break it.
pub(crate) const STOPS_AT_A_PAGE: &[u8] = b"\n%the-stand-in-stops-at-a-page\n";

/// The helper's stand-in: the same answers the host gives for a PDF, drawn in this process (tests only).
struct StandIn {
    document: Result<Document, NotOpened>,
    stops: bool,
    stopped: bool,
    came: Vec<Came>,
}

pub(crate) fn stand_in(bytes: Zeroizing<Vec<u8>>) -> Box<dyn Renderer> {
    let stops = bytes.ends_with(STOPS_AT_A_PAGE);
    Box::new(StandIn {
        document: Document::open(bytes),
        stops,
        stopped: false,
        came: Vec::new(),
    })
}

impl Renderer for StandIn {
    fn state(&self) -> OfficeState {
        if self.stopped {
            return OfficeState::Stopped {
                why: Stop::Exited,
                can_retry: true,
            };
        }
        match &self.document {
            Ok(document) => OfficeState::Pages {
                pages: document.pages(),
                sizes: document
                    .pdf
                    .pages()
                    .iter()
                    .map(|page| {
                        let (w_pt, h_pt) = page.render_dimensions();
                        PageSize { w_pt, h_pt }
                    })
                    .collect(),
            },
            Err(NotOpened::Locked) => OfficeState::NoPreview(HostNoPreview::Password),
            Err(NotOpened::TooLong { pages }) => OfficeState::Pages {
                pages: *pages,
                sizes: Vec::new(),
            },
            Err(_) => OfficeState::CannotOpen(CannotOpen::Damaged),
        }
    }

    fn poll(&mut self) -> Vec<Came> {
        std::mem::take(&mut self.came)
    }

    fn ask(&mut self, page: u32, scale: f32) -> bool {
        let Ok(document) = &self.document else { return false };
        let pages = document.pdf.pages();
        let Some(at) = page.checked_sub(1).and_then(|index| pages.get(index as usize)) else {
            return false;
        };
        if self.stops {
            self.stopped = true;
            self.came.push(Came::State);
            return true;
        }
        let pixmap = hayro::render(
            at,
            &RenderCache::new(),
            &InterpreterSettings::default(),
            &RenderSettings::default(),
            &PixmapSettings {
                x_scale: scale,
                y_scale: scale,
                bg_color: WHITE,
            },
        );
        self.came.push(Came::Page {
            page,
            rendered: Rendered {
                width: u32::from(pixmap.width()),
                height: u32::from(pixmap.height()),
                rgba: pixmap.data_as_u8_slice().to_vec(),
                lacks_text: writes_in_a_numbered_font_not_in_the_file(at),
            },
        });
        true
    }
}

/// An open session on the stand-in (the find tests ask theirs of it).
pub(crate) fn session(bytes: Vec<u8>) -> Result<Session, NotOpened> {
    let bytes = Zeroizing::new(bytes);
    Session::open(stand_in(bytes.clone()), bytes, &AtomicBool::new(false))
}

/// What a scripted helper was asked and is told to be.
#[derive(Default)]
struct Told {
    state: Option<OfficeState>,
    /// The state it takes when a page is asked of it.
    at_a_page: Option<OfficeState>,
    asked: Vec<(u32, f32)>,
    dropped: bool,
}

/// A helper that says what the test tells it to.
struct Script(Arc<Mutex<Told>>);

fn script(state: OfficeState) -> (Arc<Mutex<Told>>, Box<dyn Renderer>) {
    let told = Arc::new(Mutex::new(Told {
        state: Some(state),
        ..Told::default()
    }));
    (told.clone(), Box::new(Script(told)))
}

impl Renderer for Script {
    fn state(&self) -> OfficeState {
        self.0.lock().unwrap().state.clone().expect("a state")
    }
    fn poll(&mut self) -> Vec<Came> {
        Vec::new()
    }
    fn ask(&mut self, page: u32, scale: f32) -> bool {
        let mut told = self.0.lock().unwrap();
        told.asked.push((page, scale));
        if let Some(next) = told.at_a_page.take() {
            told.state = Some(next);
        }
        true
    }
}

impl Drop for Script {
    fn drop(&mut self) {
        self.0.lock().unwrap().dropped = true;
    }
}

fn letter(pages: u32) -> OfficeState {
    OfficeState::Pages {
        pages,
        sizes: vec![PageSize { w_pt: 612.0, h_pt: 792.0 }; pages as usize],
    }
}

fn not_opened(state: OfficeState) -> Option<NotOpened> {
    Session::open(script(state).1, Zeroizing::new(Vec::new()), &AtomicBool::new(false)).err()
}

/// What a served session said: the pages delivered, the finds answered, how often it said "stopped".
#[derive(Default)]
struct Said {
    pages: Vec<(u32, Option<Rendered>)>,
    found: Vec<(u64, usize)>,
    stopped: u32,
}

/// Serve `session`: `asks` wait before it starts; the asker goes once `until` holds of what was said (at once when
/// nothing is asked).
fn served(session: Session, asks: &[Ask], cancel: bool, until: impl Fn(&Said) -> bool) -> Said {
    let (ask, receiver) = mpsc::channel();
    for one in asks {
        ask.send(one.clone()).unwrap();
    }
    let asker = RefCell::new((!asks.is_empty()).then_some(ask));
    let said = RefCell::new(Said::default());
    let check = || {
        if until(&said.borrow()) {
            asker.borrow_mut().take();
        }
    };
    session.serve(
        &receiver,
        &AtomicBool::new(cancel),
        &|page, rendered| {
            said.borrow_mut().pages.push((page, rendered));
            check();
        },
        &|serial, matches| {
            said.borrow_mut().found.push((serial, matches.len()));
            check();
        },
        &|| said.borrow_mut().stopped += 1,
    );
    said.into_inner()
}

fn pixel(page: &Rendered, x: u32, y: u32) -> [u8; 4] {
    let at = ((y * page.width + x) * 4) as usize;
    page.rgba[at..at + 4].try_into().unwrap()
}

#[test]
fn a_pdf_opens_in_the_helper_and_a_page_asked_of_it_comes_back_as_its_pixels() {
    let session = session(sample_pdf(3, 612, 792)).expect("it opens");
    assert_eq!(session.pages(), 3);
    let mut said = served(session, &[Ask::Page(2)], false, |said| !said.pages.is_empty());
    assert_eq!((said.pages.len(), said.stopped), (1, 0));
    let (page, rendered) = said.pages.pop().unwrap();
    let rendered = rendered.expect("page 2 rendered");
    assert_eq!(page, 2);
    // A Letter page at the viewer's width, in its own proportions.
    assert!((1_259..=1_261).contains(&rendered.width), "{rendered:?}");
    assert!((1_629..=1_632).contains(&rendered.height), "{rendered:?}");
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
fn the_page_is_asked_of_the_helper_at_the_viewers_scale_and_only_once_while_it_is_out() {
    let (told, helper) = script(letter(3));
    let session = Session::open(helper, Zeroizing::new(Vec::new()), &AtomicBool::new(false)).expect("it opens");
    // Page 2 asked twice over and nothing answered: the asker goes, the worker ends, the helper is let go.
    let (ask, asks) = mpsc::channel();
    ask.send(Ask::Page(2)).unwrap();
    let asker = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(60));
        ask.send(Ask::Page(2)).unwrap();
        std::thread::sleep(Duration::from_millis(60));
    });
    session.serve(&asks, &AtomicBool::new(false), &|_, _| panic!("no page came"), &|_, _| {}, &|| {
        panic!("it did not stop")
    });
    asker.join().unwrap();
    let told = told.lock().unwrap();
    assert_eq!(told.asked, [(2, PAGE_WIDTH_PX / 612.0)]);
    assert!(told.dropped, "the worker's end ends the helper");
}

#[test]
fn what_is_not_a_pdf_and_a_pdf_with_too_many_pages_are_not_opened() {
    for bytes in [b"%PDF-1.7 invented".to_vec(), b"plain words".to_vec(), Vec::new()] {
        assert_eq!(open(bytes.clone()).err(), Some(NotOpened::Unreadable));
        assert_eq!(session(bytes).err(), Some(NotOpened::Unreadable));
    }
    assert!(
        session(sample_pdf(PAGES_MAX as usize, 100, 100)).is_ok(),
        "at the limit it opens"
    );
    let long = sample_pdf(PAGES_MAX as usize + 1, 100, 100);
    assert_eq!(open(long.clone()).err(), Some(NotOpened::TooLong { pages: PAGES_MAX + 1 }));
    assert_eq!(session(long).err(), Some(NotOpened::TooLong { pages: PAGES_MAX + 1 }));
}

#[test]
fn what_the_helper_says_at_the_open_is_what_the_viewer_says() {
    assert_eq!(not_opened(OfficeState::NoPreview(HostNoPreview::Password)), Some(NotOpened::Locked));
    // No helper beside the app, no sandbox on this system, a Windows too old for it: nothing is parsed here instead.
    for none in [
        HostNoPreview::NotInstalled,
        HostNoPreview::NoSandbox,
        HostNoPreview::NeedsWindowsUpdate,
        HostNoPreview::ThisType,
    ] {
        assert_eq!(not_opened(OfficeState::NoPreview(none)), Some(NotOpened::NoRenderer), "{none:?}");
    }
    for cannot in [CannotOpen::Damaged, CannotOpen::TooLarge] {
        assert_eq!(not_opened(OfficeState::CannotOpen(cannot)), Some(NotOpened::Unreadable));
    }
    assert_eq!(not_opened(letter(0)), Some(NotOpened::Unreadable));
    assert_eq!(not_opened(letter(PAGES_MAX + 1)), Some(NotOpened::TooLong { pages: PAGES_MAX + 1 }));
    // A helper that crashed, timed out or could not be started while it opened the file: Failed, with Try again.
    for why in [Stop::Exited, Stop::TimedOut, Stop::Protocol, Stop::Failed] {
        let stopped = OfficeState::Stopped { why, can_retry: true };
        assert_eq!(not_opened(stopped), Some(NotOpened::Stopped), "{why:?}");
    }
    assert_eq!(
        not_opened(OfficeState::Closed(office_host::CloseWhy::Locked)),
        Some(NotOpened::Stopped)
    );
    // A session that ends while the helper is still opening: the wait ends (this test would hang otherwise), and
    // the helper is let go.
    let (told, helper) = script(OfficeState::Opening);
    let opened = Session::open(helper, Zeroizing::new(Vec::new()), &AtomicBool::new(true));
    assert_eq!(opened.err(), Some(NotOpened::Stopped));
    assert!(told.lock().unwrap().dropped);
}

#[test]
fn a_helper_that_stops_at_a_page_ends_only_itself_and_the_worker_says_so_once() {
    // Scripted: the helper is gone when page 1 is asked of it.
    let (told, helper) = script(letter(2));
    told.lock().unwrap().at_a_page = Some(OfficeState::Stopped {
        why: Stop::Exited,
        can_retry: true,
    });
    let session = Session::open(helper, Zeroizing::new(Vec::new()), &AtomicBool::new(false)).expect("it opens");
    // The asker stays (`ask` is alive past the call): the worker ends on its own.
    let (ask, asks) = mpsc::channel();
    ask.send(Ask::Page(1)).unwrap();
    let stopped = RefCell::new(0);
    session.serve(&asks, &AtomicBool::new(false), &|_, _| panic!("no page came"), &|_, _| {}, &|| {
        *stopped.borrow_mut() += 1
    });
    assert_eq!(stopped.into_inner(), 1);
    assert!(told.lock().unwrap().dropped, "the stopped helper is let go");
    drop(ask);

    // The stand-in the window tests use does the same on a file marked for it; unmarked, the page comes.
    let mut hostile = sample_pdf(2, 612, 792);
    hostile.extend_from_slice(STOPS_AT_A_PAGE);
    let (ask, asks) = mpsc::channel();
    ask.send(Ask::Page(1)).unwrap();
    let stopped = RefCell::new(0);
    session_of(hostile).serve(&asks, &AtomicBool::new(false), &|_, _| panic!("no page came"), &|_, _| {}, &|| {
        *stopped.borrow_mut() += 1
    });
    assert_eq!(stopped.into_inner(), 1);
    drop(ask);
}

fn session_of(bytes: Vec<u8>) -> Session {
    session(bytes).expect("it opens")
}

#[test]
fn a_helper_that_is_closed_by_a_lock_while_the_pdf_is_open_ends_the_worker_within_its_idle_wait() {
    let (told, helper) = script(letter(2));
    let session = Session::open(helper, Zeroizing::new(Vec::new()), &AtomicBool::new(false)).expect("it opens");
    // The host's registry closed the helper (a lock): nothing was asked, the asker stays.
    told.lock().unwrap().state = Some(OfficeState::Closed(office_host::CloseWhy::Locked));
    let (ask, asks) = mpsc::channel::<Ask>();
    let stopped = RefCell::new(0);
    session.serve(&asks, &AtomicBool::new(false), &|_, _| {}, &|_, _| {}, &|| *stopped.borrow_mut() += 1);
    assert_eq!(stopped.into_inner(), 1);
    assert!(told.lock().unwrap().dropped);
    drop(ask);
}

#[test]
fn a_page_is_never_asked_past_the_pixel_limits_whatever_its_box_says() {
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
    // The longest side stays under what the protocol sizes a raster to (4,096 px a side), rounding up included.
    assert!(PAGE_SIDE_MAX + 1.0 < 4_096.0);
    // And a real page with such a box comes back within them.
    let said = served(session_of(sample_pdf(1, 200, 14_400)), &[Ask::Page(1)], false, |said| {
        !said.pages.is_empty()
    });
    let rendered = said.pages[0].1.as_ref().expect("it renders");
    assert!(u64::from(rendered.width) * u64::from(rendered.height) <= PAGE_PIXELS_MAX as u64 + 8_000);
    assert!(rendered.height <= PAGE_SIDE_MAX as u32 + 1);
}

#[test]
fn only_the_newest_page_waiting_is_asked_and_a_page_that_is_not_there_is_said() {
    let asks = [Ask::Page(1), Ask::Page(2), Ask::Page(3)];
    let said = served(session_of(sample_pdf(3, 300, 300)), &asks, false, |said| !said.pages.is_empty());
    let pages: Vec<u32> = said.pages.iter().map(|(page, _)| *page).collect();
    assert_eq!(pages, [3]);
    for missing in [0, 4, u32::MAX] {
        let said = served(session_of(sample_pdf(3, 300, 300)), &[Ask::Page(missing)], false, |said| {
            !said.pages.is_empty()
        });
        assert!(matches!(said.pages[..], [(page, None)] if page == missing), "{missing}");
    }
}

#[test]
fn a_cancelled_worker_asks_and_delivers_nothing_and_ends_although_the_asker_stays() {
    let (told, helper) = script(letter(2));
    let session = Session::open(helper, Zeroizing::new(Vec::new()), &AtomicBool::new(false)).expect("it opens");
    let (ask, asks) = mpsc::channel();
    ask.send(Ask::Page(1)).unwrap();
    ask.send(Ask::Find {
        serial: 1,
        words: "tide".into(),
    })
    .unwrap();
    session.serve(
        &asks,
        &AtomicBool::new(true),
        &|_, _| panic!("no page"),
        &|_, _| panic!("no find"),
        &|| panic!("a session's end is not a stopped helper"),
    );
    let told = told.lock().unwrap();
    assert!(told.asked.is_empty());
    assert!(told.dropped, "the helper goes with the session");
    drop(ask);
    // Nothing asked, the asker gone: `serve` returns (this test would hang otherwise).
    let said = served(session_of(sample_pdf(1, 300, 300)), &[], false, |_| true);
    assert!(said.pages.is_empty());
}

#[test]
fn a_page_the_helper_says_misses_content_lacks_text_and_no_other_page_does() {
    let pdf = fonts_pdf(&[
        Writes::Latin,
        Writes::Japanese,
        Writes::Nothing,
        Writes::ByGlyphNumber,
        Writes::JapaneseSans,
    ]);
    for (page, lacks) in [(1, false), (2, true), (3, false), (4, true), (5, true)] {
        let said = served(session_of(pdf.clone()), &[Ask::Page(page)], false, |said| !said.pages.is_empty());
        let rendered = said.pages[0].1.as_ref().expect("it renders");
        assert_eq!(rendered.lacks_text, lacks, "page {page}");
    }
}
