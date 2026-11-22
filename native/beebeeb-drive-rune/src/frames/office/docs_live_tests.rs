//! Tests of the Office scenes drawn from a renderer (V3-5): a source that answers `office_picture` and records every
//! ask, the document's virtual page list, and the helper the sheet and deck tests share.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use rune::{Env, Pixels, Runtime};

use super::*;
use crate::frames::viewers::kinds::{OfficePicture, TILE_PX};
use crate::frames::viewers::source::{Body, OpenError, OpenFile, ViewerSource};
use crate::{init, view};

/// What a live source was asked, in order.
/// A cell picture the engine was asked for (five numbers, as the source takes them).
pub(crate) type CellAsk = (u32, u32, u32, u32, u32);

#[derive(Default)]
pub(crate) struct Log {
    pub pictures: RefCell<Vec<OfficePicture>>,
    pub cells: RefCell<Vec<CellAsk>>,
    /// Answer every picture with this size (px); `None`: nothing has come yet.
    pub answer: Cell<Option<(u32, u32)>>,
    pub retries: Cell<u32>,
    /// Tiles have not come (whole pages have).
    pub tiles_missing: Cell<bool>,
}

/// A source that holds the open file in a "renderer": it says so, and records what the scenes ask of it.
pub(crate) struct LiveSource {
    pub file: OpenFile,
    pub log: Rc<Log>,
}

fn pixels(w: u32, h: u32) -> Pixels {
    let p = Pixels::new();
    p.set(w, h, vec![255; (w * h * 4) as usize]);
    p
}

impl ViewerSource for LiveSource {
    fn open(&mut self, _file_id: &str) -> Result<OpenFile, OpenError> {
        Ok(self.file.clone())
    }
    fn poll(&mut self, _file_id: &str) -> Option<Body> {
        None
    }
    fn cancel(&mut self, _file_id: &str) {}
    fn retry(&mut self, _file_id: &str) -> Result<OpenFile, OpenError> {
        self.log.retries.set(self.log.retries.get() + 1);
        Ok(self.file.clone())
    }
    fn office_live(&self) -> bool {
        true
    }
    fn office_picture(&self, _file_id: &str, which: OfficePicture) -> Option<Pixels> {
        self.log.pictures.borrow_mut().push(which);
        if self.log.tiles_missing.get() && matches!(which, OfficePicture::Tile { .. }) {
            return None;
        }
        self.log.answer.get().map(|(w, h)| pixels(w, h))
    }
    fn office_cells(&self, _file_id: &str, sheet: u32, row: u32, col: u32, rows: u32, cols: u32) {
        self.log.cells.borrow_mut().push((sheet, row, col, rows, cols));
    }
}

/// A runtime on a window of 1278 x 798 points at `scale`, with `body` open from a live source.
pub(crate) fn live_runtime(body: Body, scale: f32, answer: Option<(u32, u32)>) -> (Runtime<State>, Rc<Log>) {
    live_runtime_with(body, scale, answer, |_| {})
}

pub(crate) fn live_runtime_with(
    body: Body,
    scale: f32,
    answer: Option<(u32, u32)>,
    setup: impl Fn(&Log),
) -> (Runtime<State>, Rc<Log>) {
    let mut file = docs_fixture::file(false);
    file.body = body;
    let log = Rc::new(Log::default());
    log.answer.set(answer);
    setup(&log);
    let mut state = init();
    let id = file.id.clone();
    let opened = viewers::open_from(&mut state, Box::new(LiveSource { file, log: log.clone() }), &id);
    assert!(opened, "the live source opened");
    let env = Env {
        system_dark: false,
        scale,
    };
    let mut rt = Runtime::new(state, view, (1278.0, 798.0), env);
    rt.frame(0.0);
    // The first frame builds twice (it reads the window's size back); what is logged is ONE build of the settled window.
    rebuild(&mut rt, &log, |_| {});
    (rt, log)
}

/// Change the state, draw once, and keep only what that one build asked.
pub(crate) fn rebuild(rt: &mut Runtime<State>, log: &Log, change: impl FnOnce(&mut State)) {
    log.pictures.borrow_mut().clear();
    log.cells.borrow_mut().clear();
    rt.update(|s, _| change(s));
    rt.frame(0.0);
}

pub(crate) fn find(rt: &Runtime<State>, key: &str) -> Option<rune::ElementInfo> {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
}

pub(crate) fn doc_of(pages: u32, zoom: u16, at: u32) -> Body {
    let mut view = docs_fixture::view();
    view.pages = pages;
    view.page_sizes_pt = vec![(595.0, 842.0); pages as usize];
    view.zoom = zoom;
    let mut body = DocBody::new(view, "English (UK)");
    body.page = at;
    Body::Doc(body)
}

/// The stage scrolled to page `at` (1-based) as a key does it (the viewport and the page in view together), drawn
/// once; only what that build asked is kept.
pub(crate) fn go_to(rt: &mut Runtime<State>, log: &Log, at: u32) {
    log.pictures.borrow_mut().clear();
    rt.update(|s, tx| {
        let d = viewers::doc_mut(s).expect("a document");
        d.go_to_page(at);
        scroll::show_page(d, tx);
    });
    rt.frame(0.0);
}

pub(crate) fn asked_pages_of(log: &Log) -> Vec<u32> {
    let mut pages: Vec<u32> = log
        .pictures
        .borrow()
        .iter()
        .filter_map(|p| match p {
            OfficePicture::Page { page, .. } | OfficePicture::Tile { page, .. } => Some(*page),
            _ => None,
        })
        .collect();
    pages.sort_unstable();
    pages.dedup();
    pages
}

#[test]
fn office_docs_asks_only_the_pages_near_the_screen() {
    // 60 pages, page 31 (zero-based 30) in view at 100 %: the stage scrolled there as a key does it.
    let (mut rt, log) = live_runtime(doc_of(60, 100, 1), 1.0, Some((100, 140)));
    go_to(&mut rt, &log, 31);
    assert_eq!(viewers::doc_mut(&mut rt.state).map(|d| d.page), Some(31));
    let asked = asked_pages_of(&log);
    assert!(!asked.is_empty(), "the pages near the screen are asked");
    assert!(
        asked.iter().all(|p| (28..=32).contains(p)),
        "only the pages near the screen are asked, not {asked:?}"
    );
    assert!(
        !asked.contains(&0) && !asked.contains(&59),
        "never page 0 or 59: {asked:?}"
    );
    for n in 1..=60 {
        assert!(find(&rt, &scroll::page_key(n)).is_some(), "page {n} keeps its box");
    }
}

#[test]
fn office_docs_asks_tiles_at_150_and_whole_pages_below() {
    let (_rt, log) = live_runtime(doc_of(4, 150, 1), 1.0, None);
    let keys = log.pictures.borrow().clone();
    assert!(!keys.is_empty());
    assert!(
        keys.iter().all(|k| matches!(
            k,
            OfficePicture::Tile { .. } | OfficePicture::Page { zoom_pct: 100, .. }
        )),
        "at 150 % tiles (and the held page at the lowest scale), never a whole page at 150 %: {keys:?}"
    );
    assert!(
        keys.iter()
            .any(|k| matches!(k, OfficePicture::Tile { zoom_pct: 150, .. }))
    );
    let (_rt, log) = live_runtime(doc_of(4, 125, 1), 1.0, None);
    let keys = log.pictures.borrow().clone();
    assert!(!keys.is_empty());
    assert!(
        keys.iter()
            .all(|k| matches!(k, OfficePicture::Page { zoom_pct: 125, .. })),
        "at 125 % whole pages only: {keys:?}"
    );
}

/// Page 1's box and tile (`col`, `row`) of it, as `[x, y, w, h]` from the page's corner.
fn tile_in_page(rt: &Runtime<State>, col: u32, row: u32) -> Option<[f32; 4]> {
    let page = find(rt, "page").expect("page 1");
    let tile = find(rt, &format!("page-1-tile-{col}-{row}"))?;
    Some([
        tile.rect[0] - page.rect[0],
        tile.rect[1] - page.rect[1],
        tile.rect[2],
        tile.rect[3],
    ])
}

#[test]
fn office_docs_tiles_sit_at_their_place() {
    // A4 at 150 % and 96 dpi is a raster of 1190 x 1684 px (2 px a point) on a box of 780 points: a pixel is
    // 780 / 1190 of a point, so tile (1, 2) starts at (512, 1024) px = (335.6, 671.2) points, on whole pixels.
    let (rt, _log) = live_runtime(doc_of(4, 150, 1), 1.0, Some((TILE_PX, TILE_PX)));
    let tile = tile_in_page(&rt, 1, 2).expect("tile (1, 2) of page 1 is drawn");
    assert!((tile[0] - 336.0).abs() < 0.6, "x: {tile:?}");
    assert!((tile[1] - 671.0).abs() < 0.6, "y: {tile:?}");
}

#[test]
fn office_docs_tiles_cover_the_page_box_at_150_per_cent_display_scaling() {
    // Windows at 150 % (144 dpi), the stage at 100 %: tiles (100 x 144 = 14 400). The renderer's raster of A4 is
    // 1190 x 1684 px (2 px a point): 3 columns and 4 rows of tiles, which together are the page's box and no more.
    let (rt, log) = live_runtime(doc_of(1, 100, 1), 1.5, Some((TILE_PX, TILE_PX)));
    let page = find(&rt, "page").expect("page 1");
    let (w, h) = (page.rect[2], page.rect[3]);
    assert!((w - 520.0).abs() < 0.6, "the box is the board's width: {w}");
    let last = tile_in_page(&rt, 2, 3).expect("the last tile (2, 3) is drawn");
    assert!(
        (last[0] + last[2] - w).abs() < 0.7,
        "the last column ends at the page's right edge: {last:?} in {w}"
    );
    assert!(
        (last[1] + last[3] - h).abs() < 0.7,
        "the last row ends at the page's foot: {last:?} in {h}"
    );
    assert!(
        log.pictures.borrow().iter().all(|k| !matches!(
            k,
            OfficePicture::Tile { col: 3.., .. } | OfficePicture::Tile { row: 4.., .. }
        )),
        "no tile past the raster is asked"
    );
    // Two tiles share their edge: no gap and no overlap.
    let (a, b) = (
        tile_in_page(&rt, 0, 0).expect("tile (0, 0)"),
        tile_in_page(&rt, 1, 1).expect("tile (1, 1)"),
    );
    assert!(
        (a[0] + a[2] - b[0]).abs() < 0.01 && (a[1] + a[3] - b[1]).abs() < 0.01,
        "{a:?} {b:?}"
    );
}

#[test]
fn office_docs_a_tile_is_drawn_at_the_scale_the_pointer_and_the_caret_use() {
    // What the person sees and what a press means are one scale. Tile (1, 1) starts at pixel (512, 512) of a raster
    // of 2 px a point, the page's point (256, 256): drawn where the caret of that point is drawn, and a press there
    // is that point.
    let (mut rt, _log) = live_runtime(doc_of(1, 100, 1), 1.5, Some((TILE_PX, TILE_PX)));
    let tile = tile_in_page(&rt, 1, 1).expect("tile (1, 1)");
    let doc = viewers::doc_mut(&mut rt.state).expect("a document");
    let caret = doc.page_rect(&beebeeb_office_protocol::office::PageRect {
        page: 0,
        rect: beebeeb_office_protocol::office::RectPt {
            x: 256.0,
            y: 256.0,
            w: 1.0,
            h: 12.0,
        },
    });
    let top = doc.page_top(1);
    assert!((tile[0] - caret[0]).abs() < 0.7, "x: tile {tile:?}, caret {caret:?}");
    assert!(
        (tile[1] - (caret[1] - top)).abs() < 0.7,
        "y: tile {tile:?}, caret {caret:?}"
    );
    let (page, x, y) = doc.page_point(tile[0], top + tile[1]);
    assert_eq!(page, 0);
    assert!(
        (x - 256.0).abs() < 1.0 && (y - 256.0).abs() < 1.0,
        "a press on it is ({x}, {y})"
    );
}

#[test]
fn office_docs_tiles_divide_by_the_device_factor() {
    // At 192 dpi (scale 2) A4 at 100 % is a raster of 1587 px on a box of 520 points: tile (1, 1) starts at pixel
    // 512, which is 512 x 520 / 1587 = 167.8 points, on a whole device pixel (half a point).
    let (rt, _log) = live_runtime(doc_of(4, 100, 1), 2.0, Some((TILE_PX, TILE_PX)));
    let tile = tile_in_page(&rt, 1, 1).expect("tile (1, 1) is drawn at 200 % scale");
    assert!((tile[0] - 167.8).abs() < 0.6, "{tile:?}");
    assert!((tile[1] - 167.8).abs() < 0.6, "{tile:?}");
    // On whole device pixels: half points.
    assert!(
        (tile[0] * 2.0).fract().abs() < 0.01 && (tile[1] * 2.0).fract().abs() < 0.01,
        "{tile:?}"
    );
}

#[test]
fn office_docs_a_zoom_change_asks_again() {
    let (mut rt, log) = live_runtime(doc_of(4, 100, 1), 1.0, None);
    assert!(
        log.pictures
            .borrow()
            .iter()
            .all(|k| matches!(k, OfficePicture::Page { zoom_pct: 100, .. }))
    );
    rebuild(&mut rt, &log, |s| {
        viewers::doc_mut(s).expect("a document").view.zoom = 125
    });
    let keys = log.pictures.borrow().clone();
    assert!(
        keys.iter()
            .any(|k| matches!(k, OfficePicture::Page { zoom_pct: 125, .. })),
        "new keys at 125 %: {keys:?}"
    );
}

#[test]
fn office_docs_an_absurd_page_size_is_bounded() {
    let mut body = doc_of(2, 150, 1);
    if let Body::Doc(d) = &mut body {
        d.view.page_sizes_pt = vec![(1.0, 1.0e9), (f32::NAN, 842.0)];
    }
    let (_rt, log) = live_runtime(body, 1.0, None);
    assert!(
        log.pictures.borrow().len() < 200,
        "bounded: {} asks",
        log.pictures.borrow().len()
    );
}

#[test]
fn office_docs_a_tall_page_asks_only_the_tile_rows_near_the_screen() {
    // A page the size rule allows (20:1) but far taller than the screen: at 150 % it is 47 tile rows. Only the rows that
    // touch the band (one screen above, the screen, one below) may be asked, however tall the page is.
    const WINDOW_H: f32 = 798.0; // live_runtime's window
    let (w, h) = (595.0, 11_900.0);
    let mut body = doc_of(3, 150, 1);
    if let Body::Doc(d) = &mut body {
        d.view.page_sizes_pt = vec![(w, h); 3];
    }
    let (_rt, log) = live_runtime(body, 1.0, None);
    // The raster is 2 px a point (150 % at 96 dpi) on a box 780 points wide, so a tile is TILE_PX x 780 / 1190 points;
    // the band ends at most two screens below page 1's top.
    let side = TILE_PX as f32 * 780.0 / 1190.0;
    let page_rows = (h * 2.0 / TILE_PX as f32).ceil() as u32;
    let band_rows = (2.0 * WINDOW_H / side).ceil() as u32;
    assert!(
        page_rows > 4 * band_rows,
        "the instrument: the page ({page_rows} rows) is far taller than the band ({band_rows})"
    );
    let rows: Vec<(u32, u32)> = log
        .pictures
        .borrow()
        .iter()
        .filter_map(|p| match p {
            OfficePicture::Tile { page, row, .. } => Some((*page, *row)),
            _ => None,
        })
        .collect();
    assert!(!rows.is_empty(), "tiles were asked");
    assert!(
        rows.iter().all(|&(_, row)| row < band_rows),
        "every asked tile row touches the band ({band_rows} rows): {rows:?}"
    );
}

#[test]
fn office_docs_asks_each_key_once_per_frame() {
    let (_rt, log) = live_runtime(doc_of(6, 150, 1), 1.0, None);
    let keys = log.pictures.borrow().clone();
    for (i, k) in keys.iter().enumerate() {
        assert!(!keys[..i].contains(k), "{k:?} asked twice in one frame");
    }
}

#[test]
fn office_docs_a_missing_tile_draws_the_held_page_under_it() {
    // The tiles have not come; the page held at the lowest scale has: it is drawn under them, no blank.
    let (rt, _log) = live_runtime_with(doc_of(4, 150, 1), 1.0, Some((200, 280)), |log| {
        log.tiles_missing.set(true)
    });
    assert!(
        find(&rt, "page-held-1").is_some(),
        "the held page is drawn under the tiles"
    );
}

#[test]
fn office_fixture_scenes_keep_their_fixture_pictures() {
    // DW-1507 and DW-1509 open from the fixture source: no renderer is asked, the fixture's pictures are drawn.
    let mut state = init();
    scene_docx(&mut state);
    let env = Env {
        system_dark: false,
        scale: 1.0,
    };
    let mut rt = Runtime::new(state, view, (1278.0, 798.0), env);
    rt.frame(0.0);
    assert!(find(&rt, "page-picture-1").is_some(), "page 1 of the board's document");
    let mut state = init();
    (crate::frames::office::slides::FRAMES[0].1)(&mut state);
    let mut rt = Runtime::new(state, view, (1278.0, 798.0), env);
    rt.frame(0.0);
    assert!(find(&rt, "slide").is_some() && find(&rt, "filmstrip").is_some());
}

/// The scroll gate (PLAN §2.4): the docs scene scrolling through the 60-page latency fixture with the real helper
/// keeps the UI frame time at p95 <= 16 ms, at 100 % and at 150 %. Run on the Windows test machine with `BB_OFFICE_HELPER` set.
#[test]
#[ignore = "O-CP3: the Windows test machine, the real helper"]
fn office_docs_scroll_frame_time_p95() {
    use std::time::{Duration, Instant};

    use crate::engine_host::viewer::viewer_office::OfficeView;
    use crate::frames::viewers::kinds::OfficeKind;

    struct Real(RefCell<OfficeView>, OpenFile);
    impl ViewerSource for Real {
        fn open(&mut self, _: &str) -> Result<OpenFile, OpenError> {
            Ok(self.1.clone())
        }
        fn poll(&mut self, _: &str) -> Option<Body> {
            self.0.borrow_mut().poll()
        }
        fn cancel(&mut self, _: &str) {}
        fn retry(&mut self, _: &str) -> Result<OpenFile, OpenError> {
            Ok(self.1.clone())
        }
        fn office_live(&self) -> bool {
            true
        }
        fn office_picture(&self, _: &str, which: OfficePicture) -> Option<Pixels> {
            self.0.borrow().picture(which)
        }
    }

    let helper = std::env::var("BB_OFFICE_HELPER")
        .expect("BB_OFFICE_HELPER names the real helper: this is the Windows test machine rung");
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../beebeeb-office-render/tests/fixtures/latency/big-60.docx");
    let bytes = zeroize::Zeroizing::new(std::fs::read(&fixture).expect("the 60-page latency fixture"));
    let mut open = OfficeView::open(
        std::path::Path::new(&helper),
        OfficeKind::Docx,
        bytes,
        std::sync::Arc::new(|| {}),
    );
    let started = Instant::now();
    let doc = loop {
        match open.poll() {
            Some(Body::Doc(doc)) => break doc,
            _ if started.elapsed() > Duration::from_secs(60) => panic!("the helper did not open the fixture in 60 s"),
            _ => std::thread::sleep(Duration::from_millis(20)),
        }
    };
    for zoom in [100u16, 150] {
        let mut file = docs_fixture::file(false);
        let mut body = doc.clone();
        body.view.zoom = zoom;
        file.body = Body::Doc(body);
        let id = file.id.clone();
        let mut state = init();
        let source = Real(
            RefCell::new(OfficeView::open(
                std::path::Path::new(&helper),
                OfficeKind::Docx,
                zeroize::Zeroizing::new(std::fs::read(&fixture).expect("fixture")),
                std::sync::Arc::new(|| {}),
            )),
            file,
        );
        assert!(viewers::open_from(&mut state, Box::new(source), &id));
        let env = Env {
            system_dark: false,
            scale: 1.0,
        };
        let mut rt = Runtime::new(state, view, (1278.0, 798.0), env);
        rt.poll = Some(crate::shell::host::poll);
        rt.frame(0.0);
        rt.frame(0.0);
        // The wheel over the stage, as a reader scrolls: one screen per 30 frames.
        let stage = find(&rt, scroll::STAGE).expect("the stage").rect;
        let (x, y) = (stage[0] + stage[2] / 2.0, stage[1] + stage[3] / 2.0);
        let per_frame = stage[3] / 30.0;
        let mut times = Vec::with_capacity(600);
        for _ in 0..600u32 {
            rt.wheel(x, y, -per_frame);
            let t = Instant::now();
            rt.frame(1.0 / 60.0);
            times.push(t.elapsed());
            std::thread::sleep(Duration::from_millis(4));
        }
        times.sort();
        let p95 = times[times.len() * 95 / 100];
        println!(
            "office_docs_scroll_frame_time_p95: zoom {zoom} n {} median {:?} p95 {:?} max {:?}",
            times.len(),
            times[times.len() / 2],
            p95,
            times[times.len() - 1]
        );
        assert!(p95 <= Duration::from_millis(16), "zoom {zoom}: p95 {p95:?} over 16 ms");
    }
}

#[test]
fn office_states_try_again_asks_the_live_source_once() {
    use crate::frames::office::states::{OfficeState, StateBody};
    let stopped = Body::OfficeState(StateBody::new(OfficeState::Stopped { retried: false }, true));
    let (mut rt, log) = live_runtime(stopped, 1.0, None);
    let button = find(&rt, "empty-retry").expect("Try again is offered after a first stop");
    rt.pointer_down(button.rect[0] + 4.0, button.rect[1] + 4.0);
    rt.pointer_up(button.rect[0] + 4.0, button.rect[1] + 4.0);
    rt.frame(0.0);
    assert_eq!(log.retries.get(), 1, "the source opened the file again");
}

#[test]
fn office_docs_a_refused_page_size_draws_the_default_box() {
    // Every size the file may state that a page cannot be draws the default page's box (A4's proportions), the page's
    // own box when it can be; the ratio rule is 20:1 either way, the edge itself allowed.
    let a4 = (595.0, 842.0);
    let refused = [
        (f32::NAN, 842.0),
        (595.0, f32::NAN),
        (f32::INFINITY, 842.0),
        (595.0, f32::INFINITY),
        (0.0, 842.0),
        (595.0, 0.0),
        (-595.0, 842.0),
        (595.0, -842.0),
        (1.0, 20.5),
        (20.5, 1.0),
        (1.0, 1.0e9),
    ];
    for size in refused {
        let mut view = docs_fixture::view();
        view.pages = 2;
        view.page_sizes_pt = vec![(600.0, 300.0), size];
        let doc = DocBody::new(view, "English (UK)");
        assert_eq!(size_pt(&doc, 2), a4, "{size:?} is refused");
        let (w, h) = box_pt(&doc, 2);
        assert!(
            (h / w - 842.0 / 595.0).abs() < 1e-4,
            "{size:?} draws A4's proportions: {w} x {h}"
        );
        assert_eq!(size_pt(&doc, 1), (600.0, 300.0), "a page's own size is kept");
    }
    for ok in [(1.0, 20.0), (20.0, 1.0), (300.0, 600.0)] {
        let mut view = docs_fixture::view();
        view.pages = 1;
        view.page_sizes_pt = vec![ok];
        assert_eq!(size_pt(&DocBody::new(view, "English (UK)"), 1), ok, "{ok:?} is allowed");
    }
}

#[test]
fn office_docs_asks_a_repeated_key_once() {
    // A build that names a key twice (the same page asked by two parts of the frame) reaches the source once, and a
    // key with no answer yet is not asked again either.
    let count = Cell::new(0u32);
    let answer = |_: OfficePicture| {
        count.set(count.get() + 1);
        Some(pixels(2, 3))
    };
    let asks = Asks::new(&answer);
    let key = OfficePicture::Page {
        page: 0,
        zoom_pct: 100,
        dpi: 96,
    };
    let (a, b) = (asks.get(key), asks.get(key));
    assert_eq!(count.get(), 1, "one ask for a repeated key");
    assert!(a.is_some() && b.is_some(), "the same answer after");
    let other = OfficePicture::Page {
        page: 1,
        zoom_pct: 100,
        dpi: 96,
    };
    asks.get(other);
    assert_eq!(count.get(), 2, "another key is asked");

    let none_count = Cell::new(0u32);
    let none = |_: OfficePicture| {
        none_count.set(none_count.get() + 1);
        None
    };
    let asks = Asks::new(&none);
    assert!(asks.get(key).is_none() && asks.get(key).is_none());
    assert_eq!(none_count.get(), 1, "a missing answer is kept for the frame too");
}
