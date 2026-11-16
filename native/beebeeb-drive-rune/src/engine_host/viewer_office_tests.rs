//! The Office viewer's glue (`viewer_office.rs`) on a fake host: the route, the states, the asks, the cap, the locks.
//! The source-level tests run `EngineViewerSource` on an engine of their own (`TestEngine`) with the switches turned
//! on for that source only; the shipped constants stay off. Every name, id and byte is invented.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use beebeeb_engine::account::SessionGeneration;
use beebeeb_engine::app::viewer_api::{LocalCopy, OpenFileError, OpenFileFacts, OpenedFile};
use beebeeb_engine::app::viewer_edit_api::{EditRefused, NewCopy, SaveEnd};
use beebeeb_office_protocol::check;
use beebeeb_office_protocol::office::{Align, Anchor, AnchorKind, CellText as WireCell, Frozen};
use zeroize::Zeroizing;

use super::viewer_office::{Host, HostEvent, OfficeView, TEXTURE_CAP_BYTES};
use super::*;
use crate::engine_host::office_host::{
    CannotOpen, CellData, CellRange, CloseWhy, NoPreview as HostNoPreview, OfficeState as HostState, PageSize, Rect,
    SheetInfo, Stop,
};
use crate::frames::office::sheets::{Cell, SheetView};
use crate::frames::office::states::{OfficeState as ViewState, Reason, StateBody};
use crate::frames::viewers::Ui;
use crate::frames::viewers::kinds::{
    self, OFFICE_SHIPS, OFFICE_VIEWER_ON, OfficeKind, OfficePicture, OfficeRoute, OfficeShips, office_route_with,
};
use crate::frames::viewers::source::{Keep, SaveOutcome};

const A4: PageSize = PageSize {
    w_pt: 595.0,
    h_pt: 842.0,
};
const ALL: OfficeShips = OfficeShips {
    docx: true,
    doc: true,
    xlsx: true,
    xls: true,
    pptx: true,
};
const NONE: OfficeShips = OfficeShips {
    docx: false,
    doc: false,
    xlsx: false,
    xls: false,
    pptx: false,
};

// ---------------------------------------------------------------------------------------------------------------
// The fake host
// ---------------------------------------------------------------------------------------------------------------

#[derive(Default)]
struct Log {
    /// `None`: Opening.
    state: Option<HostState>,
    /// Handed at the next poll.
    events: Vec<HostEvent>,
    /// (id, page, scale, region)
    pages: Vec<(u64, u32, f32, Option<Rect>)>,
    /// (id, sheet, anchor, scale)
    anchors: Vec<(u64, u32, u32, f32)>,
    cancels: Vec<u64>,
    cells: Vec<(u32, CellRange)>,
    outlines: usize,
    stats: usize,
    retries: usize,
    closes: Vec<CloseWhy>,
    next: u64,
}

#[derive(Clone, Default)]
struct Fake(Arc<Mutex<Log>>);

impl Fake {
    fn log(&self) -> MutexGuard<'_, Log> {
        self.0.lock().unwrap()
    }

    fn view(&self, kind: OfficeKind) -> OfficeView {
        OfficeView::on(Some(Box::new(self.clone())), kind)
    }

    /// The host moves to `state` and says so at the next poll.
    fn set(&self, state: HostState) {
        let mut log = self.log();
        log.state = Some(state);
        log.events.push(HostEvent::State);
    }

    fn deliver(&self, id: u64, w: u32, h: u32, missing_content: bool) {
        self.log().events.push(HostEvent::Picture {
            id,
            w,
            h,
            rgba: vec![7; (w * h * 4) as usize],
            missing_content,
        });
    }

    fn cells(&self, sheet: u32, range: CellRange, cells: Vec<WireCell>, anchors: Vec<Anchor>) {
        let data = CellData {
            cells,
            col_widths: vec![50.0; range.cols as usize],
            row_heights: vec![16.0; range.rows as usize],
            merges: Vec::new(),
            frozen: Frozen { rows: 0, cols: 0 },
            anchors,
            missing_content: false,
        };
        self.log().events.push(HostEvent::Cells { sheet, range, data });
    }

    fn last_page(&self) -> u64 {
        self.log().pages.last().expect("a page was asked").0
    }
}

impl Host for Fake {
    fn state(&self) -> HostState {
        self.log().state.clone().unwrap_or(HostState::Opening)
    }

    fn poll(&mut self) -> Vec<HostEvent> {
        std::mem::take(&mut self.log().events)
    }

    fn request_page(&mut self, page: u32, scale: f32, region: Option<Rect>) -> Option<u64> {
        let mut log = self.log();
        let id = log.next;
        log.next += 1;
        log.pages.push((id, page, scale, region));
        Some(id)
    }

    fn request_anchor(&mut self, sheet: u32, anchor: u32, scale: f32) -> Option<u64> {
        let mut log = self.log();
        let id = log.next;
        log.next += 1;
        log.anchors.push((id, sheet, anchor, scale));
        Some(id)
    }

    fn cancel(&mut self, id: u64) {
        self.log().cancels.push(id);
    }

    fn request_outline(&mut self) -> bool {
        self.log().outlines += 1;
        true
    }

    fn request_stats(&mut self) -> bool {
        self.log().stats += 1;
        true
    }

    fn request_cells(&mut self, sheet: u32, range: CellRange) -> bool {
        self.log().cells.push((sheet, range));
        true
    }

    fn retry(&mut self) -> bool {
        let mut log = self.log();
        if !matches!(log.state, Some(HostState::Stopped { can_retry: true, .. })) {
            return false;
        }
        log.retries += 1;
        log.state = Some(HostState::Opening);
        true
    }

    fn close(&mut self, why: CloseWhy) {
        let mut log = self.log();
        log.closes.push(why);
        if !matches!(log.state, Some(HostState::Closed(_))) {
            log.state = Some(HostState::Closed(why));
        }
    }
}

fn pages(n: u32) -> HostState {
    HostState::Pages {
        pages: n,
        sizes: vec![A4; n as usize],
    }
}

fn sheets(rows: u32, cols: u32) -> HostState {
    HostState::Sheets {
        sheets: vec![SheetInfo {
            name: "Invented sheet".into(),
            rows,
            cols,
        }],
        more: 0,
    }
}

fn open_state(kind: OfficeKind) -> HostState {
    match kind {
        OfficeKind::Xlsx | OfficeKind::Xls => sheets(10, 4),
        _ => pages(2),
    }
}

fn state_of(body: &Body) -> Option<ViewState> {
    match body {
        Body::OfficeState(state) => Some(state.state),
        _ => None,
    }
}

fn cell(row: u32, col: u32, text: &str, value: Option<f64>, formula: Option<&str>) -> WireCell {
    WireCell {
        row,
        col,
        text: text.into(),
        align: Align::Left,
        bold: false,
        value,
        formula: formula.map(str::to_string),
    }
}

fn chart(id: u32) -> Anchor {
    Anchor {
        id,
        kind: AnchorKind::Chart,
        from_row: 0,
        from_col: 0,
        to_row: 3,
        to_col: 3,
        x_pt: 0.0,
        y_pt: 0.0,
        w_pt: 100.0,
        h_pt: 60.0,
    }
}

fn range(row: u32, col: u32, rows: u32, cols: u32) -> CellRange {
    CellRange { row, col, rows, cols }
}

// ---------------------------------------------------------------------------------------------------------------
// The engine of the source-level tests
// ---------------------------------------------------------------------------------------------------------------

struct EngineState {
    turn: u64,
    /// (id, name, size)
    files: Vec<(String, String, u64)>,
    fetches: usize,
    /// The `max_bytes` each fetch was asked with.
    max_asked: Vec<u64>,
    /// Saves, copies and keeps asked of the engine.
    writes: usize,
}

struct TestEngine {
    s: Mutex<EngineState>,
}

impl TestEngine {
    fn new(files: &[(&str, &str, u64)]) -> Arc<Self> {
        Arc::new(TestEngine {
            s: Mutex::new(EngineState {
                turn: 1,
                files: files
                    .iter()
                    .map(|(id, name, size)| (id.to_string(), name.to_string(), *size))
                    .collect(),
                fetches: 0,
                max_asked: Vec::new(),
                writes: 0,
            }),
        })
    }

    fn s(&self) -> MutexGuard<'_, EngineState> {
        self.s.lock().unwrap()
    }

    fn size_of(&self, file_id: &str) -> Option<(String, u64)> {
        let s = self.s();
        s.files
            .iter()
            .find(|(id, ..)| id == file_id)
            .map(|(_, name, size)| (name.clone(), *size))
    }
}

impl ViewerEngine for TestEngine {
    fn session_turn(&self) -> SessionGeneration {
        SessionGeneration::for_test(self.s().turn)
    }

    fn has_session(&self) -> bool {
        true
    }

    fn account(&self) -> Option<String> {
        Some("a@example.eu".into())
    }

    fn facts(&self, file_id: &str) -> Result<OpenFileFacts, OpenFileError> {
        let (name, size) = self.size_of(file_id).ok_or(OpenFileError::NotFound)?;
        Ok(OpenFileFacts {
            session: self.session_turn(),
            name,
            folders: Vec::new(),
            size,
            on_this_pc: None,
            unsettled_here: false,
        })
    }

    fn fetch(
        &self,
        file_id: &str,
        max_bytes: u64,
        _progress: &(dyn Fn(u64, u64) + Send + Sync + 'static),
        _cancel: &AtomicBool,
    ) -> Result<OpenedFile, OpenFileError> {
        {
            let mut s = self.s();
            s.fetches += 1;
            s.max_asked.push(max_bytes);
        }
        let (_, size) = self.size_of(file_id).ok_or(OpenFileError::NotFound)?;
        if size > max_bytes {
            return Err(OpenFileError::TooLarge { size });
        }
        Ok(OpenedFile {
            session: self.session_turn(),
            bytes: Zeroizing::new(vec![0; size as usize]),
        })
    }

    fn local(&self, _file_id: &str, _max_bytes: u64, _cancel: &AtomicBool) -> Result<LocalCopy, OpenFileError> {
        Ok(LocalCopy::NotHere)
    }

    fn is_syncing(&self) -> bool {
        true
    }

    fn open_elsewhere(&self, _file_id: &str) -> Elsewhere {
        Elsewhere::Refused
    }

    fn wake(&self) {}

    fn set_kept(&self, _session: SessionGeneration, _file_id: &str, _kept: bool) -> Result<(), ()> {
        self.s().writes += 1;
        Err(())
    }

    fn save_text(
        &self,
        _session: SessionGeneration,
        _file_id: &str,
        _text: &str,
        _base: i64,
        _opening: u64,
    ) -> Result<SaveEnd, EditRefused> {
        self.s().writes += 1;
        Err(EditRefused::Failed)
    }

    fn save_copy(
        &self,
        _session: SessionGeneration,
        _file_id: &str,
        _text: &str,
        _device: &str,
        _hhmm: &str,
    ) -> Result<NewCopy, EditRefused> {
        self.s().writes += 1;
        Err(EditRefused::Failed)
    }
}

/// What the opener was handed: the kind and how many bytes.
type Opened = Arc<Mutex<Vec<(OfficeKind, usize)>>>;

/// A source on `engine` with every Office type switched on (for this source only), whose renderer is `fake`.
fn source_with(engine: Arc<TestEngine>, fake: &Fake, opened: Opened) -> EngineViewerSource<TestEngine> {
    let mut source = EngineViewerSource::on(engine);
    source.office.on = true;
    source.office.ships = ALL;
    source.office.helper = || Some(PathBuf::from("beebeeb-office-render"));
    let fake = fake.clone();
    source.office.opener = Box::new(move |_, kind, bytes, _| {
        opened.lock().unwrap().push((kind, bytes.len()));
        fake.view(kind)
    });
    source
}

/// The first body after the read answered (the renderer's first state).
fn first_body(source: &mut EngineViewerSource<TestEngine>, id: &str) -> Body {
    let end = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(body) = source.poll(id) {
            return body;
        }
        assert!(Instant::now() < end, "the read never answered");
        std::thread::sleep(Duration::from_millis(2));
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The locks and the route
// ---------------------------------------------------------------------------------------------------------------

/// Lock 1: no route to the editor's body. Every kind, opened through the engine source, through a stop and a retry.
#[test]
fn office_editor_body_never_from_the_engine_source() {
    let kinds = [
        ("Invented.docx", OfficeKind::Docx),
        ("Invented.doc", OfficeKind::Doc),
        ("Invented.xlsx", OfficeKind::Xlsx),
        ("Invented.xls", OfficeKind::Xls),
        ("Invented.pptx", OfficeKind::Pptx),
    ];
    for (name, kind) in kinds {
        let fake = Fake::default();
        let mut source = source_with(TestEngine::new(&[("f1", name, 64)]), &fake, Opened::default());
        let mut seen = vec![source.open("f1").unwrap().body];
        seen.push(first_body(&mut source, "f1"));
        fake.set(open_state(kind));
        seen.extend(source.poll("f1"));
        fake.set(HostState::Stopped {
            why: Stop::Exited,
            can_retry: true,
        });
        seen.extend(source.poll("f1"));
        seen.push(source.retry("f1").unwrap().body);
        fake.set(open_state(kind));
        seen.extend(source.poll("f1"));
        assert!(
            seen.iter().all(|body| !matches!(body, Body::Word(_) | Body::NoEditor)),
            "{name}: {seen:?}"
        );
        // The document was reached twice, so the check above looked at it.
        let shown = seen
            .iter()
            .filter(|body| matches!(body, Body::Doc(_) | Body::Deck(_) | Body::Sheet(_)))
            .count();
        assert_eq!(shown, 2, "{name}: {seen:?}");
        assert_eq!(fake.log().retries, 1, "{name}");
    }
}

/// Lock 3: the engine source saves nothing of an Office file.
#[test]
fn office_engine_source_saves_nothing() {
    let fake = Fake::default();
    let engine = TestEngine::new(&[("f1", "Invented.docx", 64)]);
    let mut source = source_with(engine.clone(), &fake, Opened::default());
    source.open("f1").unwrap();
    first_body(&mut source, "f1");
    fake.set(pages(2));
    assert!(matches!(source.poll("f1"), Some(Body::Doc(_))));
    assert!(matches!(source.save_document("f1", 1), SaveOutcome::Failed));
    assert!(matches!(source.resolve_document("f1", Keep::Mine), SaveOutcome::Failed));
    assert!(source.commands("f1", "").is_empty());
    assert!(!source.can_save());
    assert_eq!(engine.s().writes, 0);
}

#[test]
fn the_route_follows_both_switches() {
    let all = [
        ("a.docx", OfficeKind::Docx),
        ("a.doc", OfficeKind::Doc),
        ("a.xlsx", OfficeKind::Xlsx),
        ("a.xls", OfficeKind::Xls),
        ("a.pptx", OfficeKind::Pptx),
        ("a.ppt", OfficeKind::Ppt),
    ];
    for (name, kind) in all {
        assert_eq!(office_route_with(name, false, ALL), None, "{name}");
        assert_eq!(
            office_route_with(name, true, NONE),
            Some(OfficeRoute::NoPreview(kind)),
            "{name}"
        );
    }
    let docx_only = OfficeShips { docx: true, ..NONE };
    assert_eq!(
        office_route_with("a.docx", true, docx_only),
        Some(OfficeRoute::Shows(OfficeKind::Docx))
    );
    assert_eq!(
        office_route_with("B.DOCX", true, docx_only),
        Some(OfficeRoute::Shows(OfficeKind::Docx))
    );
    for (name, kind) in all.into_iter().filter(|(_, kind)| *kind != OfficeKind::Docx) {
        assert_eq!(
            office_route_with(name, true, docx_only),
            Some(OfficeRoute::NoPreview(kind)),
            "{name}"
        );
    }
    assert_eq!(
        office_route_with("a.ppt", true, ALL),
        Some(OfficeRoute::NoPreview(OfficeKind::Ppt))
    );
    assert_eq!(office_route_with("a.txt", true, ALL), None);
    assert_eq!(office_route_with("no extension", true, ALL), None);
}

/// O-CP3 turned the viewer on and shipped the five types that met 90 % of their corpus pages on Windows; `.ppt` still
/// has no preview here (DW-1556).
#[test]
fn the_shipped_constants_are_what_o_cp3_measured() {
    assert!(std::hint::black_box(OFFICE_VIEWER_ON));
    assert_eq!(std::hint::black_box(OFFICE_SHIPS), ALL);
    for (name, kind) in [
        ("a.docx", OfficeKind::Docx),
        ("a.doc", OfficeKind::Doc),
        ("a.xlsx", OfficeKind::Xlsx),
        ("a.xls", OfficeKind::Xls),
        ("a.pptx", OfficeKind::Pptx),
    ] {
        assert_eq!(kinds::office_route(name), Some(OfficeRoute::Shows(kind)), "{name}");
    }
    assert_eq!(
        kinds::office_route("a.ppt"),
        Some(OfficeRoute::NoPreview(OfficeKind::Ppt))
    );
}

#[test]
fn over_100_mb_fetches_nothing() {
    let fake = Fake::default();
    let engine = TestEngine::new(&[("f1", "Invented.docx", check::MAX_INPUT_BYTES + 1)]);
    let opened = Opened::default();
    let mut source = source_with(engine.clone(), &fake, opened.clone());
    let body = source.open("f1").unwrap().body;
    assert_eq!(state_of(&body), Some(ViewState::NoPreview(Reason::TooLarge)));
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(source.poll("f1"), None);
    assert_eq!(engine.s().fetches, 0);
    assert!(opened.lock().unwrap().is_empty());
}

#[test]
fn ppt_and_unshipped_types_fetch_nothing() {
    for (name, ships) in [("Invented.ppt", ALL), ("Invented.docx", NONE)] {
        let fake = Fake::default();
        let engine = TestEngine::new(&[("f1", name, 64)]);
        let opened = Opened::default();
        let mut source = source_with(engine.clone(), &fake, opened.clone());
        source.office.ships = ships;
        assert!(source.previews(name), "{name}: the viewer says why");
        let body = source.open("f1").unwrap().body;
        assert_eq!(
            state_of(&body),
            Some(ViewState::NoPreview(Reason::UnsupportedType)),
            "{name}"
        );
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(source.poll("f1"), None, "{name}");
        assert_eq!(engine.s().fetches, 0, "{name}");
        assert!(opened.lock().unwrap().is_empty(), "{name}: no renderer started");
    }
    // A view of a `.ppt` never uses a host, even when handed one.
    let fake = Fake::default();
    let mut view = fake.view(OfficeKind::Ppt);
    assert_eq!(
        view.poll().as_ref().and_then(state_of),
        Some(ViewState::NoPreview(Reason::UnsupportedType))
    );
    assert!(
        view.picture(OfficePicture::Page {
            page: 0,
            zoom_pct: 100,
            dpi: 96
        })
        .is_none()
    );
    assert!(fake.log().pages.is_empty());
}

/// The lead's ruling (2026-11-16 02:47): an Office file between the viewer's 64 MiB and the contract's 100 MiB is read
/// whole and handed to the renderer.
#[test]
fn an_office_file_between_64_and_100_mib_is_read_whole() {
    let size = 80 * 1024 * 1024;
    assert!(size > IN_MEMORY_MAX && size <= check::MAX_INPUT_BYTES);
    let fake = Fake::default();
    let engine = TestEngine::new(&[("f1", "Invented.docx", size)]);
    let opened = Opened::default();
    let mut source = source_with(engine.clone(), &fake, opened.clone());
    source.open("f1").unwrap();
    first_body(&mut source, "f1");
    assert_eq!(*opened.lock().unwrap(), vec![(OfficeKind::Docx, size as usize)]);
    assert_eq!(engine.s().max_asked, vec![check::MAX_INPUT_BYTES]);
}

// ---------------------------------------------------------------------------------------------------------------
// States
// ---------------------------------------------------------------------------------------------------------------

fn body_for(kind: OfficeKind, state: HostState) -> Option<Body> {
    let fake = Fake::default();
    let mut view = fake.view(kind);
    fake.set(state);
    view.poll()
}

fn view_state(kind: OfficeKind, state: HostState) -> Option<ViewState> {
    body_for(kind, state).as_ref().and_then(state_of)
}

#[test]
fn host_states_become_their_bodies() {
    use OfficeKind::{Doc, Docx, Pptx, Xlsx};
    assert_eq!(view_state(Docx, HostState::Opening), Some(ViewState::Opening));
    match body_for(Docx, pages(2)) {
        Some(Body::Doc(doc)) => {
            assert_eq!(doc.view.pages, 2);
            assert_eq!(doc.view.page_sizes_pt, vec![(595.0, 842.0); 2]);
            assert!(!doc.view.older_format);
            assert_eq!(doc.view.zoom, 100);
            assert!(doc.language.is_empty(), "no language is made up");
        }
        other => panic!("{other:?}"),
    }
    assert!(matches!(body_for(Doc, pages(1)), Some(Body::Doc(doc)) if doc.view.older_format));
    assert!(matches!(body_for(Pptx, pages(3)), Some(Body::Deck(deck)) if deck.view.count == 3));
    match body_for(Xlsx, sheets(10, 4)) {
        Some(Body::Sheet(sheet)) => {
            assert_eq!(sheet.tabs, vec!["Invented sheet".to_string()]);
            assert_eq!((sheet.active, sheet.sheet, sheet.extent), (0, 0, (10, 4)));
        }
        other => panic!("{other:?}"),
    }
    let rows = [
        (
            HostState::NoPreview(HostNoPreview::ThisType),
            ViewState::NoPreview(Reason::UnsupportedType),
        ),
        (
            HostState::NoPreview(HostNoPreview::Password),
            ViewState::NoPreview(Reason::UnsupportedType),
        ),
        (
            HostState::NoPreview(HostNoPreview::NoSandbox),
            ViewState::NoPreview(Reason::NotOnThisComputer),
        ),
        (
            HostState::NoPreview(HostNoPreview::NeedsWindowsUpdate),
            ViewState::NoPreview(Reason::NeedsWindowsUpdate),
        ),
        (
            HostState::NoPreview(HostNoPreview::NotInstalled),
            ViewState::Stopped { retried: true },
        ),
        (
            HostState::CannotOpen(CannotOpen::Damaged),
            ViewState::NoPreview(Reason::DamagedOrOverLimit),
        ),
        (
            HostState::CannotOpen(CannotOpen::TooLarge),
            ViewState::NoPreview(Reason::TooLarge),
        ),
        (
            HostState::Stopped {
                why: Stop::TimedOut,
                can_retry: true,
            },
            ViewState::Stopped { retried: false },
        ),
        (
            HostState::Stopped {
                why: Stop::Protocol,
                can_retry: false,
            },
            ViewState::Stopped { retried: true },
        ),
    ];
    for (host, shown) in rows {
        assert_eq!(view_state(Docx, host.clone()), Some(shown), "{host:?}");
    }
    // Closed by a lock or a sign-out: said once; closed by the view itself: nothing to say.
    for (why, said) in [
        (CloseWhy::Locked, Some(Body::Stopped(Stopped::SessionChanged))),
        (CloseWhy::SignedOut, Some(Body::Stopped(Stopped::SessionChanged))),
        (CloseWhy::Closed, None),
    ] {
        let fake = Fake::default();
        let mut view = fake.view(Xlsx);
        fake.set(sheets(10, 4));
        view.poll();
        fake.set(HostState::Closed(why));
        assert_eq!(view.poll(), said, "{why:?}");
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Asking while drawn
// ---------------------------------------------------------------------------------------------------------------

fn doc_view(fake: &Fake, n: u32) -> OfficeView {
    let mut view = fake.view(OfficeKind::Docx);
    fake.set(pages(n));
    view.poll();
    view
}

#[test]
fn a_page_is_asked_once_and_drawn_when_it_comes() {
    let fake = Fake::default();
    let mut view = doc_view(&fake, 2);
    let page = OfficePicture::Page {
        page: 0,
        zoom_pct: 100,
        dpi: 96,
    };
    assert!(view.picture(page).is_none());
    assert!(view.picture(page).is_none());
    {
        let log = fake.log();
        assert_eq!(log.pages.len(), 1, "asked once");
        let (_, asked, scale, region) = log.pages[0];
        assert_eq!((asked, region), (0, None));
        assert_eq!(scale, check::render_scale(1.0, 96.0));
    }
    fake.deliver(fake.last_page(), 8, 8, false);
    assert!(view.poll().is_some(), "the window draws it");
    assert_eq!(view.picture(page).map(|pixels| pixels.size()), Some((8, 8)));
    assert_eq!(fake.log().pages.len(), 1);
}

#[test]
fn tiles_ask_their_region() {
    let fake = Fake::default();
    let view = doc_view(&fake, 1);
    let tile = |col, row| OfficePicture::Tile {
        page: 0,
        zoom_pct: 150,
        dpi: 96,
        col,
        row,
    };
    assert!(kinds::uses_tiles(150, 96));
    // A4 at 150 % and 96 dpi: scale 2.0, a raster of 1190 × 1684.
    view.picture(tile(1, 2));
    view.picture(tile(2, 0));
    view.picture(tile(3, 0));
    let log = fake.log();
    assert_eq!(log.pages.len(), 2, "a tile past the page is not asked");
    assert_eq!(log.pages[0].2, 2.0);
    assert_eq!(
        log.pages[0].3,
        Some(Rect {
            x: 512,
            y: 1024,
            w: 512,
            h: 512.min(1684 - 1024)
        })
    );
    // The last column, clipped to the raster.
    assert_eq!(
        log.pages[1].3,
        Some(Rect {
            x: 1024,
            y: 0,
            w: 1190 - 1024,
            h: 512
        })
    );
}

#[test]
fn a_thumbnail_is_asked_at_its_width() {
    let fake = Fake::default();
    let mut view = fake.view(OfficeKind::Pptx);
    fake.set(pages(3));
    assert!(matches!(view.poll(), Some(Body::Deck(_))));
    let thumb = OfficePicture::Thumb { slide: 1, dpi: 144 };
    assert!(view.picture(thumb).is_none());
    let log = fake.log();
    assert_eq!(log.pages.len(), 1);
    let (_, slide, scale, region) = log.pages[0];
    assert_eq!((slide, region), (1, None));
    // `THUMB_W_PT` wide at 144 dpi: 140 × 144 / 96 = 210 px across a 595 pt slide.
    assert_eq!(scale, kinds::THUMB_W_PT * 144.0 / 96.0 / 595.0);
    assert_eq!(check::page_pixels(&A4, scale).map(|(w, _)| w), Some(210));
}

#[test]
fn a_new_scale_keeps_the_old_picture_until_the_new_one_comes() {
    let fake = Fake::default();
    let mut view = doc_view(&fake, 1);
    let at = |zoom_pct| OfficePicture::Page {
        page: 0,
        zoom_pct,
        dpi: 96,
    };
    view.picture(at(100));
    fake.deliver(fake.last_page(), 8, 8, false);
    view.poll();
    let shown = view.picture(at(125)).expect("the old picture stays up");
    assert_eq!(shown.size(), (8, 8));
    assert_eq!(fake.log().pages.len(), 2, "the new scale is asked");
    fake.deliver(fake.last_page(), 10, 10, false);
    view.poll();
    assert_eq!(view.picture(at(125)).map(|pixels| pixels.size()), Some((10, 10)));
}

#[test]
fn what_scrolled_past_is_cancelled() {
    let fake = Fake::default();
    let mut view = doc_view(&fake, 10);
    let seventh = OfficePicture::Page {
        page: 7,
        zoom_pct: 100,
        dpi: 96,
    };
    view.picture(seventh);
    let id = fake.last_page();
    view.poll(); // the frame that asked
    view.poll(); // one poll without it
    assert!(fake.log().cancels.is_empty());
    view.poll(); // a second
    assert_eq!(fake.log().cancels, vec![id]);
    view.poll();
    assert_eq!(fake.log().cancels, vec![id], "cancelled once");
    // Wanted again later: asked anew.
    view.picture(seventh);
    assert_eq!(fake.log().pages.len(), 2);
}

#[test]
fn the_texture_cap_drops_the_least_recently_wanted() {
    assert_eq!(TEXTURE_CAP_BYTES, 256 * 1024 * 1024);
    // The rule at 1/1024 of the size: a 256 KiB cap and pictures of 64 KiB (128 × 128 × 4), as 256 MiB and 64 MiB.
    let fake = Fake::default();
    let mut view = doc_view(&fake, 5);
    view.set_cap(256 * 1024);
    let page = |page| OfficePicture::Page {
        page,
        zoom_pct: 100,
        dpi: 96,
    };
    for n in 0..5 {
        view.picture(page(n));
        fake.deliver(fake.last_page(), 128, 128, false);
        view.poll();
    }
    let (count, bytes) = view.held_pictures();
    assert_eq!(count, 4);
    assert!(bytes <= 256 * 1024);
    assert!(!view.holds(page(0)), "the least recently wanted went first");
    assert!((1..5).all(|n| view.holds(page(n))));
    // Every held picture wanted since the last poll: none is dropped, even over the cap.
    for n in 1..5 {
        view.picture(page(n));
    }
    view.picture(page(0));
    fake.deliver(fake.last_page(), 128, 128, false);
    view.poll();
    assert_eq!(view.held_pictures().0, 5);
}

#[test]
fn cells_merge_into_the_sheet_by_place() {
    let fake = Fake::default();
    let mut view = fake.view(OfficeKind::Xlsx);
    fake.set(sheets(100, 10));
    assert!(matches!(view.poll(), Some(Body::Sheet(_))));
    assert_eq!(
        fake.log().cells,
        vec![(0, range(0, 0, 64, 10))],
        "the first block at once"
    );
    fake.cells(
        0,
        range(0, 0, 2, 2),
        vec![cell(0, 0, "a", None, None), cell(1, 1, "old", Some(2.0), Some("=1+1"))],
        vec![chart(1)],
    );
    fake.cells(
        0,
        range(1, 1, 2, 2),
        vec![cell(1, 1, "new", Some(3.0), Some("=1+2")), cell(2, 2, "c", None, None)],
        vec![chart(1)],
    );
    let Some(Body::Sheet(sheet)) = view.poll() else {
        panic!("a sheet")
    };
    assert_eq!(sheet.cells.len(), 3);
    let at = |row, col| {
        sheet
            .cells
            .iter()
            .find(|cell| cell.row == row && cell.col == col)
            .unwrap()
    };
    assert_eq!((at(1, 1).text.as_str(), at(1, 1).value), ("new", Some(3.0)));
    assert_eq!(at(1, 1).formula.as_deref(), Some("=1+2"));
    assert_eq!((at(0, 0).text.as_str(), at(2, 2).text.as_str()), ("a", "c"));
    assert_eq!(sheet.anchors.len(), 1, "an anchor is listed once");
    assert_eq!(sheet.col_widths, vec![50.0; 3]);
    assert_eq!(sheet.row_heights, vec![16.0; 3]);
}

#[test]
fn a_lock_drops_every_picture_and_wipes_the_body() {
    let fake = Fake::default();
    let mut view = fake.view(OfficeKind::Xlsx);
    fake.set(sheets(10, 4));
    view.poll();
    fake.cells(
        0,
        range(0, 0, 1, 1),
        vec![cell(0, 0, "invented secret", Some(1.0), Some("=A2"))],
        vec![chart(1)],
    );
    view.poll();
    let picture = OfficePicture::Anchor {
        sheet: 0,
        anchor: 1,
        zoom_pct: 100,
        dpi: 96,
    };
    assert!(view.picture(picture).is_none());
    let id = fake.log().anchors[0].0;
    fake.deliver(id, 16, 16, false);
    view.poll();
    assert_eq!(view.held_pictures().0, 1);
    fake.set(HostState::Closed(CloseWhy::Locked));
    assert_eq!(view.poll(), Some(Body::Stopped(Stopped::SessionChanged)));
    assert_eq!(view.held_pictures(), (0, 0));
    let Body::Sheet(sheet) = view.held_body() else {
        panic!("the sheet, wiped")
    };
    assert!(sheet.tabs.iter().all(String::is_empty));
    assert!(
        sheet.cells.iter().all(|cell| cell.text.is_empty()
            && cell.value.is_none()
            && cell.formula.as_deref().is_none_or(str::is_empty))
    );
    assert!(view.picture(picture).is_none());
    assert_eq!(fake.log().anchors.len(), 1, "nothing is asked after the close");
}

// ---------------------------------------------------------------------------------------------------------------
// The person's place survives a new body of the same file (lead's ruling 2026-11-16 02:47; `Body::carry_view`)
// ---------------------------------------------------------------------------------------------------------------

/// A source that is one open Office view, so the test can ask pictures while `Ui` owns the source.
struct ViewSource {
    view: Rc<RefCell<OfficeView>>,
    file: OpenFile,
}

impl ViewerSource for ViewSource {
    fn open(&mut self, _file_id: &str) -> Result<OpenFile, OpenError> {
        Ok(self.file.clone())
    }

    fn poll(&mut self, _file_id: &str) -> Option<Body> {
        self.view.borrow_mut().poll()
    }

    fn cancel(&mut self, _file_id: &str) {}

    fn retry(&mut self, _file_id: &str) -> Result<OpenFile, OpenError> {
        Ok(self.file.clone())
    }
}

fn ui_on(fake: &Fake, kind: OfficeKind, id: &str) -> (Ui, Rc<RefCell<OfficeView>>) {
    let view = Rc::new(RefCell::new(fake.view(kind)));
    let file = OpenFile {
        id: id.into(),
        path: vec!["Files".into()],
        name: "Invented".into(),
        facts: String::new(),
        version: 1,
        body: Body::OfficeState(StateBody::new(ViewState::Opening, false)),
        details: Details::default(),
        newer: None,
    };
    let source = ViewSource {
        view: view.clone(),
        file,
    };
    (Ui::open(Box::new(source), id).unwrap(), view)
}

#[test]
fn a_document_keeps_its_page_and_zoom_when_a_picture_comes() {
    let fake = Fake::default();
    let (mut ui, view) = ui_on(&fake, OfficeKind::Docx, "f1");
    fake.set(pages(3));
    assert!(ui.pump());
    let Body::Doc(doc) = &mut ui.file.body else {
        panic!("{:?}", ui.file.body)
    };
    (doc.page, doc.view.zoom, doc.outline_open) = (3, 150, false);
    let page = OfficePicture::Page {
        page: 2,
        zoom_pct: 150,
        dpi: 96,
    };
    assert!(view.borrow().picture(page).is_none());
    fake.deliver(fake.last_page(), 8, 8, false);
    assert!(ui.pump());
    let Body::Doc(doc) = &ui.file.body else {
        panic!("{:?}", ui.file.body)
    };
    assert_eq!((doc.page, doc.view.zoom, doc.outline_open), (3, 150, false));
    assert!(view.borrow().picture(page).is_some());
}

#[test]
fn a_deck_keeps_its_slide_and_zoom_when_a_picture_comes() {
    let fake = Fake::default();
    let (mut ui, view) = ui_on(&fake, OfficeKind::Pptx, "f1");
    fake.set(pages(5));
    assert!(ui.pump());
    let Body::Deck(deck) = &mut ui.file.body else {
        panic!("{:?}", ui.file.body)
    };
    (deck.view.current, deck.view.zoom) = (4, 125);
    view.borrow().picture(OfficePicture::Page {
        page: 3,
        zoom_pct: 125,
        dpi: 96,
    });
    fake.deliver(fake.last_page(), 8, 8, false);
    assert!(ui.pump());
    let Body::Deck(deck) = &ui.file.body else {
        panic!("{:?}", ui.file.body)
    };
    assert_eq!((deck.view.current, deck.view.zoom), (4, 125));
}

#[test]
fn a_sheet_keeps_its_scroll_and_selection_when_cells_come() {
    let fake = Fake::default();
    let (mut ui, view) = ui_on(&fake, OfficeKind::Xlsx, "f1");
    fake.set(sheets(100, 10));
    assert!(ui.pump());
    let Body::Sheet(sheet) = &mut ui.file.body else {
        panic!("{:?}", ui.file.body)
    };
    sheet.top_left = Cell { row: 40, col: 3 };
    sheet.selection = Some((Cell { row: 40, col: 3 }, Cell { row: 42, col: 4 }));
    sheet.current = Cell { row: 41, col: 4 };
    view.borrow().wants_cells(0, 40, 3, 30, 8);
    let (sheet_no, block) = *fake.log().cells.last().unwrap();
    assert_eq!((sheet_no, block), (0, range(32, 0, 64, 10)));
    fake.cells(0, block, vec![cell(41, 4, "x", Some(1.0), None)], Vec::new());
    assert!(ui.pump());
    let Body::Sheet(sheet) = &ui.file.body else {
        panic!("{:?}", ui.file.body)
    };
    assert_eq!(sheet.top_left, Cell { row: 40, col: 3 });
    assert_eq!(
        sheet.selection,
        Some((Cell { row: 40, col: 3 }, Cell { row: 42, col: 4 }))
    );
    assert_eq!(sheet.current, Cell { row: 41, col: 4 });
    assert_eq!(sheet.cells.len(), 1);
}

#[test]
fn another_file_carries_nothing() {
    // File A, zoomed and on page 3, is left; file B opens at its own start.
    let fake_a = Fake::default();
    let (mut a, _) = ui_on(&fake_a, OfficeKind::Docx, "a");
    fake_a.set(pages(3));
    a.pump();
    if let Body::Doc(doc) = &mut a.file.body {
        (doc.page, doc.view.zoom) = (3, 150);
    }
    drop(a);
    let fake_b = Fake::default();
    let (mut b, view_b) = ui_on(&fake_b, OfficeKind::Docx, "b");
    fake_b.set(pages(3));
    assert!(b.pump());
    view_b.borrow().picture(OfficePicture::Page {
        page: 0,
        zoom_pct: 100,
        dpi: 96,
    });
    fake_b.deliver(fake_b.last_page(), 8, 8, false);
    assert!(b.pump());
    let Body::Doc(doc) = &b.file.body else {
        panic!("{:?}", b.file.body)
    };
    assert_eq!((doc.page, doc.view.zoom), (1, 100));
    // Another kind, or another sheet of the workbook, carries nothing either.
    let mut deck = body_for(OfficeKind::Pptx, pages(2)).unwrap();
    let doc = Body::Doc(doc.clone());
    deck.carry_view(&doc);
    assert!(matches!(&deck, Body::Deck(d) if d.view.current == 1 && d.view.zoom == 100));
    let Some(Body::Sheet(first)) = body_for(OfficeKind::Xlsx, sheets(10, 4)) else {
        panic!("a sheet")
    };
    let scrolled = SheetView {
        top_left: Cell { row: 5, col: 2 },
        ..first.clone()
    };
    let mut other = Body::Sheet(SheetView { sheet: 1, ..first });
    other.carry_view(&Body::Sheet(scrolled));
    assert!(matches!(&other, Body::Sheet(sheet) if sheet.top_left == Cell { row: 0, col: 0 }));
}

// ---------------------------------------------------------------------------------------------------------------
// CP2 row 10.3 (L266.4): the viewer's words for a 403 and a version that kept changing
// ---------------------------------------------------------------------------------------------------------------

#[test]
fn pinned_route_403_is_not_said_as_not_decrypted() {
    assert_eq!(body_of(OpenFileError::NoAccess), Some(Body::Stopped(Stopped::NoAccess)));
    assert_ne!(body_of(OpenFileError::NoAccess), Some(Body::Failed));
    assert!(!Stopped::NoAccess.can_retry(), "trying again does not give access back");
}

#[test]
fn a_version_that_kept_changing_says_try_again() {
    assert_eq!(
        body_of(OpenFileError::ChangedWhileOpening),
        Some(Body::Stopped(Stopped::Changed))
    );
    assert!(Stopped::Changed.can_retry());
}

/// P3-7: only the five measured types route to the viewer; the eight others are their own app's.
#[test]
fn the_eight_unmeasured_types_have_no_office_route() {
    for ext in ["docm", "dotx", "dotm", "dot", "xlsm", "xltx", "pptm", "ppsx"] {
        let name = format!("a.{ext}");
        assert_eq!(office_route_with(&name, true, ALL), None, "{name}");
        assert_eq!(kinds::office_route(&name), None, "{name}");
    }
    assert_eq!(
        office_route_with("a.pps", true, ALL),
        Some(OfficeRoute::NoPreview(OfficeKind::Ppt))
    );
}

/// P3-6: a kept `.ppt` has no preview, so the source does not claim it; a kept `.docx` it does.
#[test]
fn a_kept_file_opens_here_only_where_the_viewer_shows_it() {
    let fake = Fake::default();
    let source = source_with(
        TestEngine::new(&[("f1", "Invented.docx", 64)]),
        &fake,
        Opened::default(),
    );
    for name in ["a.docx", "a.doc", "a.xlsx", "a.xls", "a.pptx"] {
        assert!(source.opens_here_when_kept(name), "{name}");
    }
    for name in ["a.ppt", "a.pps", "a.docm", "a.ppsx"] {
        assert!(!source.opens_here_when_kept(name), "{name}");
    }
}

/// P2-3: the Office renderer is live only where the sandbox crate has a module (Windows).
#[test]
fn office_is_live_only_where_the_sandbox_exists() {
    let fake = Fake::default();
    let source = source_with(
        TestEngine::new(&[("f1", "Invented.docx", 64)]),
        &fake,
        Opened::default(),
    );
    assert_eq!(source.office_live(), cfg!(windows));
}
