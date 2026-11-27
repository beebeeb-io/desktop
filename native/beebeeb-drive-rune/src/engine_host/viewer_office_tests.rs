//! The Office viewer's glue (`viewer_office.rs`) on a fake host: the route, the states, the asks, the cap, the locks.
//! The source-level tests run `EngineViewerSource` on an engine of their own (`TestEngine`) with the switches turned
//! on for that source only; the shipped constants stay off. Every name, id and byte is invented.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use beebeeb_engine::account::SessionGeneration;
use beebeeb_engine::app::office_recovery::{RecoveryEntry, RecoveryRefused};
use beebeeb_engine::app::viewer_api::{LocalCopy, OpenFileError, OpenFileFacts, OpenedFile};
use beebeeb_engine::app::viewer_edit_api::{EditFacts, EditRefused, NewCopy, SaveEnd};
use beebeeb_office_protocol::check;
use beebeeb_office_protocol::office::{
    Align, Anchor, AnchorKind, CellText as WireCell, Dirty, EditKind, EditOp, Edited, Frozen, HitMode, ListKind, Marks,
    Opened as WireOpened, PageRect, ParaStyle, Purpose, RectPt, Template, TextAlign,
};
use zeroize::Zeroizing;

use super::viewer_office::{EditShips, Host, HostEdit, HostEvent, OfficeView, TEXTURE_CAP_BYTES};
use super::*;
use crate::engine_host::office_host::{
    CannotOpen, CellData, CellRange, CloseWhy, NoPreview as HostNoPreview, OfficeState as HostState, PageSize, Rect,
    SheetInfo, Stop,
};
use crate::frames::office::sheets::{Cell, SheetView};
use crate::frames::office::states::{OfficeState as ViewState, Reason, StateBody};
use crate::frames::viewers::kinds::{
    self, office_route_with, OfficeKind, OfficePicture, OfficeRoute, OfficeShips, OFFICE_SHIPS, OFFICE_VIEWER_ON,
};
use crate::frames::viewers::source::{Keep, SaveOutcome};
use crate::frames::viewers::Ui;

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
    /// An editing session: the edit calls are taken.
    editing: bool,
    /// What was asked of the editor, in words ("edit", "hit", "save", ...) and the id it was given.
    asked: Vec<(&'static str, u64)>,
    /// Handed at the next `poll_edit`.
    answers: Vec<HostEdit>,
    /// Where the editor said its unsaved work is sealed at a lock (the real host's worker calls it).
    sink: Option<crate::engine_host::office_host::FlushSink>,
}

#[derive(Clone, Default)]
pub(super) struct Fake(Arc<Mutex<Log>>);

impl Fake {
    fn log(&self) -> MutexGuard<'_, Log> {
        self.0.lock().unwrap()
    }

    pub(super) fn view(&self, kind: OfficeKind) -> OfficeView {
        OfficeView::on(Some(Box::new(self.clone())), kind)
    }

    /// An editing session of `kind`: the host is open for editing once its state says so.
    pub(super) fn editor(&self, kind: EditKind) -> OfficeView {
        self.log().editing = true;
        OfficeView::editing_on(Box::new(self.clone()), kind)
    }

    /// The id the editor was given for its `what`-th ask of that kind.
    fn id_of(&self, what: &str) -> u64 {
        self.log()
            .asked
            .iter()
            .rev()
            .find(|(asked, _)| *asked == what)
            .unwrap_or_else(|| panic!("no {what} was asked"))
            .1
    }

    pub(super) fn asked_count(&self, what: &str) -> usize {
        self.log().asked.iter().filter(|(asked, _)| *asked == what).count()
    }

    /// The door the editor registered for a lock's snapshot.
    pub(super) fn flush_sink(&self) -> Option<crate::engine_host::office_host::FlushSink> {
        self.log().sink.clone()
    }

    pub(super) fn answer(&self, answer: HostEdit) {
        self.log().answers.push(answer);
    }

    /// The editor answers a save with `bytes` at `revision`.
    pub(super) fn saved(&self, bytes: &[u8], revision: u64) {
        let id = self.id_of("save");
        self.answer(HostEdit::Saved {
            id,
            purpose: Purpose::Save,
            revision,
            bytes: Zeroizing::new(bytes.to_vec()),
        });
    }

    fn ask(&self, what: &'static str) -> Option<u64> {
        let mut log = self.log();
        if !log.editing || !matches!(log.state, Some(HostState::Pages { .. } | HostState::Sheets { .. })) {
            return None;
        }
        let id = log.next;
        log.next += 1;
        log.asked.push((what, id));
        Some(id)
    }

    /// The host moves to `state` and says so at the next poll.
    pub(super) fn set(&self, state: HostState) {
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
    fn set_flush_sink(&mut self, sink: crate::engine_host::office_host::FlushSink) {
        self.log().sink = Some(sink);
    }

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

    fn edit(&mut self, _op: EditOp) -> Option<u64> {
        self.ask("edit")
    }

    fn hit(&mut self, _page: u32, _x_pt: f32, _y_pt: f32, _mode: HitMode) -> Option<u64> {
        self.ask("hit")
    }

    fn copy(&mut self) -> Option<u64> {
        self.ask("copy")
    }

    fn layouts(&mut self) -> Option<u64> {
        self.ask("layouts")
    }

    fn insert_image(&mut self, _bytes: Zeroizing<Vec<u8>>, _png: bool) -> Option<u64> {
        self.ask("image")
    }

    fn save_bytes(&mut self, _purpose: Purpose) -> Option<u64> {
        self.ask("save")
    }

    fn poll_edit(&mut self) -> Vec<HostEdit> {
        std::mem::take(&mut self.log().answers)
    }
}

pub(super) fn pages(n: u32) -> HostState {
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
    /// The version each file is at (a file not in it: 3), and the files the account may not change.
    versions: std::collections::HashMap<String, i64>,
    read_only: Vec<String>,
    /// Office documents stored: (file id, bytes, the base they were saved from). A conflict stores nothing.
    stored: Vec<(String, Vec<u8>, i64)>,
    /// Keep both copies: (file id, bytes).
    copies: Vec<(String, Vec<u8>)>,
    /// New documents: (folder, name, bytes).
    created: Vec<(Option<String>, String, Vec<u8>)>,
    /// The copy on this PC of a file made by New document cannot be read yet (the real engine: queued, not yet on disk).
    new_unreadable: bool,
    /// Answer saves with this instead of storing (`None`: store).
    save_says: Option<Result<SaveEnd, EditRefused>>,
    /// The recovery blobs this PC keeps: file id -> (time, base, bytes). Survives a source (a restart of the app).
    blobs: std::collections::BTreeMap<String, (i64, i64, Vec<u8>)>,
    /// The version the file is at is one this opening's saves became (`EditFacts::own`): a queued save that landed.
    own: bool,
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
                versions: Default::default(),
                read_only: Vec::new(),
                stored: Vec::new(),
                copies: Vec::new(),
                created: Vec::new(),
                new_unreadable: false,
                save_says: None,
                blobs: Default::default(),
                own: false,
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
            on_this_pc: (self.s().new_unreadable && file_id.starts_with("new"))
                .then_some(beebeeb_engine::app::viewer_api::OnThisPc::NotSentYet),
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

    fn local(&self, file_id: &str, _max_bytes: u64, _cancel: &AtomicBool) -> Result<LocalCopy, OpenFileError> {
        if self.s().new_unreadable && file_id.starts_with("new") {
            return Err(OpenFileError::CopyNotReadable);
        }
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

    fn edit_facts(&self, _session: SessionGeneration, file_id: &str, _opening: u64) -> Option<EditFacts> {
        let s = self.s();
        Some(EditFacts {
            version: s.versions.get(file_id).copied().unwrap_or(3),
            kept: false,
            can_write: !s.read_only.iter().any(|id| id == file_id),
            own: s.own,
        })
    }

    /// The engine's rule in short: a save from a version older than the file's is a conflict and stores nothing.
    fn save_office(
        &self,
        _session: SessionGeneration,
        file_id: &str,
        bytes: &Zeroizing<Vec<u8>>,
        base: i64,
        _opening: u64,
    ) -> Result<SaveEnd, EditRefused> {
        let mut s = self.s();
        if let Some(says) = s.save_says {
            return says;
        }
        let now = s.versions.get(file_id).copied().unwrap_or(3);
        if base < now {
            return Ok(SaveEnd::Conflict { theirs: now });
        }
        s.stored.push((file_id.to_string(), bytes.to_vec(), base));
        s.versions.insert(file_id.to_string(), now + 1);
        Ok(SaveEnd::Saved { version: now + 1 })
    }

    fn save_office_copy(
        &self,
        _session: SessionGeneration,
        file_id: &str,
        bytes: &Zeroizing<Vec<u8>>,
        _device: &str,
        _hhmm: &str,
    ) -> Result<NewCopy, EditRefused> {
        let mut s = self.s();
        s.copies.push((file_id.to_string(), bytes.to_vec()));
        Ok(NewCopy {
            id: "copy1".into(),
            name: concat!("Invented (", beebeeb_engine::device_word_cap!(), ", 09:41).docx").into(),
        })
    }

    fn recovery_put(
        &self,
        _session: SessionGeneration,
        file_id: &str,
        base: i64,
        bytes: &Zeroizing<Vec<u8>>,
    ) -> Result<(), RecoveryRefused> {
        let at = 86_400 * 100 + 14 * 3600 + 32 * 60;
        self.s().blobs.insert(file_id.to_string(), (at, base, bytes.to_vec()));
        Ok(())
    }

    fn recovery_list(&self, _session: SessionGeneration) -> Result<Vec<RecoveryEntry>, RecoveryRefused> {
        Ok(self
            .s()
            .blobs
            .iter()
            .map(|(id, (at, base, _))| RecoveryEntry {
                file_id: id.clone(),
                saved_at: *at,
                base: *base,
            })
            .collect())
    }

    fn recovery_take(&self, _session: SessionGeneration, file_id: &str) -> Result<Zeroizing<Vec<u8>>, RecoveryRefused> {
        self.s()
            .blobs
            .get(file_id)
            .map(|b| Zeroizing::new(b.2.clone()))
            .ok_or(RecoveryRefused::NotFound)
    }

    fn recovery_discard(&self, _session: SessionGeneration, file_id: &str) -> Result<(), RecoveryRefused> {
        self.s()
            .blobs
            .remove(file_id)
            .map(|_| ())
            .ok_or(RecoveryRefused::NotFound)
    }

    fn create_office(
        &self,
        _session: SessionGeneration,
        folder: Option<&str>,
        name: &str,
        bytes: &Zeroizing<Vec<u8>>,
    ) -> Result<String, EditRefused> {
        let mut s = self.s();
        s.created
            .push((folder.map(str::to_string), name.to_string(), bytes.to_vec()));
        let id = format!("new{}", s.created.len());
        let size = bytes.len() as u64;
        s.files.push((id.clone(), name.to_string(), size));
        Ok(id)
    }
}

/// What the opener was handed: the kind and how many bytes.
type Opened = Arc<Mutex<Vec<(OfficeKind, usize)>>>;

/// What the editing opener was handed: the kind and how many bytes.
type Edits = Arc<Mutex<Vec<(EditKind, usize)>>>;

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
    // The editor is off unless a test turns it on (on Windows its default is on).
    source.office.edit_on = false;
    source
}

/// As [`source_with`], with the editor on for every type: a writable file opens in an editing session on `fake`.
fn editor_with(
    engine: Arc<TestEngine>,
    fake: &Fake,
    opened: Opened,
    edits: Edits,
    template: &Fake,
) -> EngineViewerSource<TestEngine> {
    let mut source = source_with(engine, fake, opened);
    source.office.edit_on = true;
    source.office.edits = EditShips {
        docx: true,
        xlsx: true,
        pptx: true,
    };
    let session = fake.clone();
    source.office.edit_opener = Box::new(move |_, kind, bytes, _, _| {
        edits.lock().unwrap().push((kind, bytes.len()));
        session.editor(kind)
    });
    let template = template.clone();
    source.office.new_opener = Box::new(move |_, kind, _, _, _| template.editor(kind));
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

// ---------------------------------------------------------------------------------------------------------------
// The editor
// ---------------------------------------------------------------------------------------------------------------

/// An `Edited` at `revision` (a caret on page one, nothing selected).
pub(super) fn edited(revision: u64, dirty: Dirty) -> Edited {
    Edited {
        revision,
        caret: Some(PageRect {
            page: 0,
            rect: RectPt {
                x: 72.0,
                y: 96.0,
                w: 1.0,
                h: 14.0,
            },
        }),
        selection: Vec::new(),
        can_undo: true,
        can_redo: false,
        dirty,
        pages: None,
        cells: Vec::new(),
        slides: None,
        marks: Marks {
            bold: false,
            italic: false,
            underline: false,
            strike: false,
            style: ParaStyle::Normal,
            align: TextAlign::Left,
            list: ListKind::None,
        },
    }
}

fn page_rect(page: u32) -> PageRect {
    PageRect {
        page,
        rect: RectPt {
            x: 0.0,
            y: 0.0,
            w: 10.0,
            h: 10.0,
        },
    }
}

fn typed() -> EditOp {
    EditOp::InsertText("An invented line.".into())
}

/// Frames until the source settles something (at most 20 s); `each` runs between two frames.
fn until_settled(
    source: &mut EngineViewerSource<TestEngine>,
    id: &str,
    mut each: impl FnMut(),
) -> crate::frames::viewers::source::Settled {
    let end = Instant::now() + Duration::from_secs(20);
    loop {
        source.poll(id);
        match source.settled(id) {
            // The revision a landed save holds comes first (its own test); the save's answer is the next one.
            Some(crate::frames::viewers::source::Settled::SavedRevision(_)) => continue,
            Some(settled) => return settled,
            None => {}
        }
        each();
        assert!(Instant::now() < end, "nothing settled");
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// Everything an editing test holds.
struct Editor {
    fake: Fake,
    engine: Arc<TestEngine>,
    source: EngineViewerSource<TestEngine>,
    viewer_opened: Opened,
    edits: Edits,
}

/// `name` open in an editor on a fake host that is open (its pages or sheets known).
fn editor(name: &str, read_only: bool) -> Editor {
    let engine = TestEngine::new(&[("f1", name, 64)]);
    if read_only {
        engine.s().read_only.push("f1".into());
    }
    editor_over(engine, name)
}

/// [`editor`] over an existing engine (a second one is the app after a restart: the blobs stayed).
fn editor_over(engine: Arc<TestEngine>, name: &str) -> Editor {
    let fake = Fake::default();
    let template = Fake::default();
    let (viewer_opened, edits) = (Opened::default(), Edits::default());
    let mut source = editor_with(engine.clone(), &fake, viewer_opened.clone(), edits.clone(), &template);
    source.open("f1").unwrap();
    first_body(&mut source, "f1");
    let kind = kinds::office_of(name).unwrap();
    fake.set(open_state(kind));
    source.poll("f1");
    Editor {
        fake,
        engine,
        source,
        viewer_opened,
        edits,
    }
}

/// A save that landed says first which revision its bytes are (what is typed after them stays unsaved in the window),
/// then answers as saved. A copy the helper cannot answer at all is said, once, and another refusal is not.
#[test]
fn a_landed_save_says_its_revision_first_and_a_copy_too_large_is_said() {
    use crate::frames::viewers::source::{EditAnswer, Settled};
    let mut e = editor("Invented.docx", false);
    assert!(e.source.edit("f1", typed()));
    e.fake.answer(HostEdit::Edited(Box::new(edited(4, Dirty::All))));
    assert!(e.source.poll_edit("f1").is_some());
    assert!(matches!(e.source.save_document("f1", 3), SaveOutcome::Working));
    e.source.poll("f1");
    e.fake.saved(b"PK invented package", 4);
    let end = Instant::now() + Duration::from_secs(20);
    let first = loop {
        e.source.poll("f1");
        if let Some(settled) = e.source.settled("f1") {
            break settled;
        }
        assert!(Instant::now() < end, "nothing settled");
        std::thread::sleep(Duration::from_millis(2));
    };
    assert!(matches!(first, Settled::SavedRevision(4)), "the revision comes first");
    assert!(
        matches!(e.source.settled("f1"), Some(Settled::Saved { version: 4, .. })),
        "then the save"
    );
    assert!(e.source.settled("f1").is_none());

    // Copy: the helper says the selection is too large.
    assert!(e.source.copy_selection("f1"));
    let id = e.fake.id_of("copy");
    e.fake.answer(HostEdit::Failed { id, too_large: true });
    assert!(matches!(e.source.poll_edit("f1"), Some(EditAnswer::CopyTooLarge)));
    assert!(e.source.poll_edit("f1").is_none());
    // A copy refused for another reason says nothing.
    assert!(e.source.copy_selection("f1"));
    let id = e.fake.id_of("copy");
    e.fake.answer(HostEdit::Failed { id, too_large: false });
    assert!(e.source.poll_edit("f1").is_none());
}

/// A writable .docx, .xlsx or .pptx opens in an editing session and everything else opens in the viewer, as before:
/// a file the account may not change, an older format, the editor switched off, and a type the editor does not ship.
#[test]
fn a_writable_file_opens_in_the_editor_and_the_rest_in_the_viewer() {
    for (name, kind) in [
        ("Invented.docx", EditKind::Docx),
        ("Invented.xlsx", EditKind::Xlsx),
        ("Invented.pptx", EditKind::Pptx),
    ] {
        let e = editor(name, false);
        assert_eq!(
            e.edits.lock().unwrap().iter().map(|(k, _)| *k).collect::<Vec<_>>(),
            [kind],
            "{name}"
        );
        assert!(e.viewer_opened.lock().unwrap().is_empty(), "{name}: not the viewer");
        assert!(e.source.can_edit("f1"), "{name}");
    }
    // Read-only, and the older formats: the viewer, and never the editor.
    for (name, read_only) in [
        ("Invented.docx", true),
        ("Invented.xlsx", true),
        ("Invented.doc", false),
        ("Invented.xls", false),
    ] {
        let mut e = editor(name, read_only);
        assert!(e.edits.lock().unwrap().is_empty(), "{name}: the editor never started");
        assert_eq!(e.viewer_opened.lock().unwrap().len(), 1, "{name}: the viewer did");
        assert!(!e.source.can_edit("f1"), "{name}");
        assert!(!e.source.edit("f1", typed()), "{name}");
        assert!(!e.source.hit("f1", 0, 1.0, 1.0, HitMode::Place), "{name}");
        assert!(!e.source.copy_selection("f1"), "{name}");
        assert!(matches!(e.source.save_document("f1", 3), SaveOutcome::Failed), "{name}");
        assert!(
            matches!(e.source.resolve_document("f1", Keep::Mine), SaveOutcome::Failed),
            "{name}"
        );
        assert!(!e.source.saving(), "{name}");
        assert_eq!(e.engine.s().writes, 0, "{name}");
        assert!(e.engine.s().stored.is_empty(), "{name}: nothing was written");
    }
    // Switched off (a PC without the sandbox), and a type that is not shipped to the editor: the viewer.
    let fake = Fake::default();
    let engine = TestEngine::new(&[("f1", "Invented.docx", 64)]);
    let (opened, edits) = (Opened::default(), Edits::default());
    let mut source = editor_with(engine, &fake, opened.clone(), edits.clone(), &Fake::default());
    source.office.edit_on = false;
    source.open("f1").unwrap();
    first_body(&mut source, "f1");
    assert!(
        edits.lock().unwrap().is_empty() && opened.lock().unwrap().len() == 1,
        "off: the viewer"
    );
    let fake = Fake::default();
    let engine = TestEngine::new(&[("f1", "Invented.pptx", 64)]);
    let (opened, edits) = (Opened::default(), Edits::default());
    let mut source = editor_with(engine, &fake, opened.clone(), edits.clone(), &Fake::default());
    source.office.edits.pptx = false;
    source.open("f1").unwrap();
    first_body(&mut source, "f1");
    assert!(
        edits.lock().unwrap().is_empty() && opened.lock().unwrap().len() == 1,
        "pptx off: the viewer"
    );
    // The shipped switches: the editor on where the sandbox is, slides too (MPL-2.0 accepted 2026-11-18).
    const {
        assert!(EditShips::SHIPPED.docx && EditShips::SHIPPED.xlsx && EditShips::SHIPPED.pptx);
    }
}

#[test]
fn the_editors_calls_reach_the_session_and_its_answers_come_back_in_order() {
    let mut e = editor("Invented.docx", false);
    assert!(e.source.edit("f1", typed()));
    assert!(e.source.hit("f1", 0, 10.0, 20.0, HitMode::Place));
    assert!(e.source.copy_selection("f1"));
    assert!(e.source.layouts("f1"));
    assert!(e.source.insert_image("f1", Zeroizing::new(vec![1, 2, 3]), true));
    for what in ["edit", "hit", "copy", "layouts", "image"] {
        assert_eq!(e.fake.asked_count(what), 1, "{what}");
    }
    e.fake
        .answer(HostEdit::Edited(Box::new(edited(1, Dirty::Rects(vec![page_rect(0)])))));
    e.fake.answer(HostEdit::Clip {
        text: "invented".into(),
        html: None,
    });
    e.fake.answer(HostEdit::Layouts(vec!["Title".into()]));
    assert!(matches!(e.source.poll_edit("f1"), Some(EditAnswer::Edited(got)) if got.revision == 1));
    assert!(matches!(e.source.poll_edit("f1"), Some(EditAnswer::Clip { text, .. }) if text == "invented"));
    assert!(matches!(e.source.poll_edit("f1"), Some(EditAnswer::Layouts(names)) if names == ["Title"]));
    assert!(e.source.poll_edit("f1").is_none());
    // Another file's calls are not this one's.
    assert!(!e.source.edit("f2", typed()) && e.source.poll_edit("f2").is_none());
    // The renderer stopping is told once.
    e.fake.set(HostState::Stopped {
        why: Stop::Exited,
        can_retry: false,
    });
    e.source.poll("f1");
    assert!(matches!(e.source.poll_edit("f1"), Some(EditAnswer::Stopped)));
    assert!(!e.source.can_edit("f1") && !e.source.edit("f1", typed()));
}

/// An edited docx saves as the next version: the bytes the engine gets are the bytes the helper saved, from the base
/// the press named, and the window gets the document as it is on screen. The helper keeps running.
#[test]
fn an_edited_docx_saves_as_the_next_version() {
    let mut e = editor("Invented.docx", false);
    assert!(e.source.edit("f1", typed()));
    e.fake.answer(HostEdit::Edited(Box::new(edited(4, Dirty::All))));
    assert!(e.source.poll_edit("f1").is_some());
    assert!(matches!(e.source.save_document("f1", 3), SaveOutcome::Working));
    assert!(e.source.saving(), "the file is not opened again meanwhile");
    assert!(e.source.open("f1").is_err(), "an open waits for the save");
    // The session has not handed the bytes yet: nothing is stored.
    e.source.poll("f1");
    assert_eq!(e.fake.asked_count("save"), 1);
    assert!(e.engine.s().stored.is_empty());
    let helper_saved = b"PK invented package";
    e.fake.saved(helper_saved, 4);
    let fake = e.fake.clone();
    let settled = until_settled(&mut e.source, "f1", || {
        let _ = &fake;
    });
    let crate::frames::viewers::source::Settled::Saved { id, version, body, .. } = settled else {
        panic!("saved")
    };
    assert_eq!((id.as_str(), version), ("f1", 4));
    assert!(
        matches!(body, Body::Doc(_)),
        "the document on screen, not a placeholder"
    );
    let stored = e.engine.s().stored.clone();
    assert_eq!(stored, vec![("f1".to_string(), helper_saved.to_vec(), 3)]);
    assert!(!e.source.saving());
    // The session goes on, on the new version.
    assert!(e.source.can_edit("f1") && e.source.edit("f1", typed()));
    assert!(e.fake.log().closes.is_empty());
}

/// A save from a base older than the file's answers Conflict and writes nothing; the edit stays open, and the three
/// answers each do what they say.
#[test]
fn a_save_from_a_stale_base_answers_conflict_and_the_three_answers_work() {
    let mut e = editor("Invented.docx", false);
    e.engine.s().versions.insert("f1".into(), 5);
    assert!(matches!(e.source.save_document("f1", 3), SaveOutcome::Working));
    e.fake.saved(b"mine v1", 1);
    let settled = until_settled(&mut e.source, "f1", || {});
    let crate::frames::viewers::source::Settled::Conflict(conflict) = settled else {
        panic!("a conflict")
    };
    assert_eq!(conflict.version, 5);
    assert!(e.engine.s().stored.is_empty(), "nothing was written");
    assert!(!e.source.saving());
    assert!(e.source.can_edit("f1"), "the edit stays open");

    // Keep mine: from the version the file is at now (5), not the stale base.
    assert!(matches!(
        e.source.resolve_document("f1", Keep::Mine),
        SaveOutcome::Working
    ));
    e.fake.saved(b"mine v2", 2);
    let settled = until_settled(&mut e.source, "f1", || {});
    let crate::frames::viewers::source::Settled::Saved { version, .. } = settled else {
        panic!("saved")
    };
    assert_eq!(version, 6);
    assert_eq!(e.engine.s().stored, vec![("f1".to_string(), b"mine v2".to_vec(), 5)]);

    // Keep both: a copy next to the file, under its own id; the file as it was.
    assert!(matches!(
        e.source.resolve_document("f1", Keep::Both),
        SaveOutcome::Working
    ));
    e.fake.saved(b"mine v3", 3);
    let settled = until_settled(&mut e.source, "f1", || {});
    let crate::frames::viewers::source::Settled::Saved { id, version, .. } = settled else {
        panic!("a copy")
    };
    assert_eq!((id.as_str(), version), ("copy1", 1));
    assert_eq!(e.engine.s().copies, vec![("f1".to_string(), b"mine v3".to_vec())]);
    assert_eq!(e.engine.s().stored.len(), 1, "the file itself was not saved again");

    // Keep theirs: the file is opened again as Beebeeb has it, and the session with its edits ends.
    let before = e.edits.lock().unwrap().len();
    let SaveOutcome::Opened(file) = e.source.resolve_document("f1", Keep::Theirs) else {
        panic!("opened")
    };
    assert_eq!(file.id, "f1");
    // (The fake host is shared by both sessions: the old one's close is seen by the new one, so it is opened again.)
    let end = Instant::now() + Duration::from_secs(20);
    while e.edits.lock().unwrap().len() == before {
        e.source.poll("f1");
        assert!(Instant::now() < end, "no new session started");
        std::thread::sleep(Duration::from_millis(2));
    }
    e.fake.set(pages(2));
    assert!(matches!(e.source.poll("f1"), Some(Body::Doc(_))));
    assert!(e.source.can_edit("f1"), "a new session on the file's bytes");
}

/// Every way a save can fail before the engine is asked settles as not done and ends the busy state; no stale answer
/// of an earlier opening is ever drawn.
#[test]
fn a_save_that_cannot_get_its_bytes_settles_failed_and_ends_the_busy_state() {
    // The helper stops while the bytes are asked.
    let mut e = editor("Invented.docx", false);
    assert!(matches!(e.source.save_document("f1", 3), SaveOutcome::Working));
    e.fake.set(HostState::Stopped {
        why: Stop::Exited,
        can_retry: false,
    });
    let settled = until_settled(&mut e.source, "f1", || {});
    assert!(matches!(settled, crate::frames::viewers::source::Settled::Failed));
    assert!(!e.source.saving() && e.engine.s().stored.is_empty());
    // The helper refuses the save size.
    let mut e = editor("Invented.docx", false);
    assert!(matches!(e.source.save_document("f1", 3), SaveOutcome::Working));
    let id = e.fake.id_of("save");
    e.fake.answer(HostEdit::Failed { id, too_large: false });
    let settled = until_settled(&mut e.source, "f1", || {});
    assert!(matches!(settled, crate::frames::viewers::source::Settled::Failed));
    assert!(!e.source.saving());
    // The engine refuses (an earlier change not sent yet): said, nothing stored.
    let mut e = editor("Invented.docx", false);
    e.engine.s().save_says = Some(Err(EditRefused::EarlierChangeNotSent));
    assert!(matches!(e.source.save_document("f1", 3), SaveOutcome::Working));
    e.fake.saved(b"x", 1);
    let settled = until_settled(&mut e.source, "f1", || {});
    assert!(matches!(settled, crate::frames::viewers::source::Settled::Refused(_)));
    assert!(!e.source.saving());
    // A second press while one is on its way is not taken.
    let mut e = editor("Invented.docx", false);
    assert!(matches!(e.source.save_document("f1", 3), SaveOutcome::Working));
    assert!(matches!(e.source.save_document("f1", 3), SaveOutcome::Failed));
}

/// New document: a session from the template, its bytes to the engine, then the file is opened as any file is.
#[test]
fn new_document_creates_the_file_and_opens_it() {
    let fake = Fake::default();
    let template = Fake::default();
    let engine = TestEngine::new(&[]);
    let (opened, edits) = (Opened::default(), Edits::default());
    let mut source = editor_with(engine.clone(), &fake, opened.clone(), edits.clone(), &template);
    assert!(source.create_document(Some("folder1"), "Invented.docx", EditKind::Docx, Template::Blank));
    assert!(source.saving());
    assert!(
        !source.create_document(None, "Another.docx", EditKind::Docx, Template::Blank),
        "one at a time"
    );
    template.set(pages(1));
    let mut delivered = false;
    let settled = until_settled(&mut source, "", || {
        if !delivered && template.asked_count("save") == 1 {
            template.saved(b"PK blank letter", 1);
            delivered = true;
        }
    });
    let crate::frames::viewers::source::Settled::Saved { id, name, version, .. } = settled else {
        panic!("created")
    };
    assert_eq!((id.as_str(), name.as_str(), version), ("new1", "Invented.docx", 3));
    assert_eq!(
        engine.s().created,
        vec![(
            Some("folder1".to_string()),
            "Invented.docx".to_string(),
            b"PK blank letter".to_vec()
        )]
    );
    assert!(!source.saving());
    assert!(template.log().closes.len() == 1, "the template's session is closed");
    // Opened: the new file is the open one, in an editing session on its bytes.
    first_body(&mut source, "new1");
    assert_eq!(edits.lock().unwrap().len(), 1);
    fake.set(pages(1));
    assert!(source.poll("new1").is_some() && source.can_edit("new1"));
}

/// v3 bug 2 (DW-1558): the new file's copy on this PC is not readable yet (queued, not on disk). The editor opens on
/// the bytes the app just made, not on the copy: no "Couldn't read the copy on this PC".
#[test]
fn new_document_opens_on_its_own_bytes_when_the_local_copy_is_not_readable_yet() {
    let fake = Fake::default();
    let template = Fake::default();
    let engine = TestEngine::new(&[]);
    engine.s().new_unreadable = true;
    let (opened, edits) = (Opened::default(), Edits::default());
    let mut source = editor_with(engine.clone(), &fake, opened, edits.clone(), &template);
    assert!(source.create_document(None, "Invented.docx", EditKind::Docx, Template::Blank));
    template.set(pages(1));
    let mut delivered = false;
    let settled = until_settled(&mut source, "", || {
        if !delivered && template.asked_count("save") == 1 {
            template.saved(b"PK blank letter", 1);
            delivered = true;
        }
    });
    assert!(matches!(settled, crate::frames::viewers::source::Settled::Saved { .. }));
    fake.set(pages(1));
    let end = Instant::now() + Duration::from_secs(20);
    loop {
        if source.can_edit("new1") {
            break;
        }
        if let Some(body) = source.poll("new1") {
            assert!(
                !matches!(body, Body::Stopped(Stopped::CopyNotReadable)),
                "the new file's editor stopped on its own copy"
            );
        }
        assert!(Instant::now() < end, "the editor never opened");
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(
        edits.lock().unwrap().as_slice(),
        &[(EditKind::Docx, b"PK blank letter".len())]
    );
}

/// w23 (the w20 fix held only on the fake platform): the window opens the made file with `Ui::open`, that is a SECOND
/// `open` of the same source after `Settled::Saved`. That open used to read the copy on this PC again (Windows: queued,
/// not on disk, or a placeholder: "Couldn't read the copy on this PC", 3 of 3 on the Windows test machine). It opens on the made bytes.
#[test]
fn new_document_opened_by_the_window_after_saved_uses_its_own_bytes() {
    let fake = Fake::default();
    let template = Fake::default();
    let engine = TestEngine::new(&[]);
    engine.s().new_unreadable = true;
    let (opened, edits) = (Opened::default(), Edits::default());
    let mut source = editor_with(engine.clone(), &fake, opened, edits.clone(), &template);
    assert!(source.create_document(None, "Invented.docx", EditKind::Docx, Template::Blank));
    template.set(pages(1));
    let mut delivered = false;
    let settled = until_settled(&mut source, "", || {
        if !delivered && template.asked_count("save") == 1 {
            template.saved(b"PK blank letter", 1);
            delivered = true;
        }
    });
    let crate::frames::viewers::source::Settled::Saved { id, .. } = settled else {
        panic!("created")
    };
    // What `new_file::open_made` does next: `Ui::open(source, id)` -> `source.open(id)`.
    assert!(source.open(&id).is_ok());
    fake.set(pages(1));
    let end = Instant::now() + Duration::from_secs(20);
    loop {
        if source.can_edit(&id) {
            break;
        }
        if let Some(body) = source.poll(&id) {
            assert!(
                !matches!(body, Body::Stopped(Stopped::CopyNotReadable)),
                "the new file's editor stopped on its own copy"
            );
        }
        assert!(Instant::now() < end, "the editor never opened");
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(
        edits.lock().unwrap().as_slice(),
        &[(EditKind::Docx, b"PK blank letter".len())]
    );
}

#[test]
fn new_document_ends_as_failed_when_the_template_never_opens() {
    let template = Fake::default();
    let engine = TestEngine::new(&[]);
    let mut source = editor_with(
        engine.clone(),
        &Fake::default(),
        Opened::default(),
        Edits::default(),
        &template,
    );
    assert!(source.create_document(None, "Invented.docx", EditKind::Docx, Template::Blank));
    template.set(HostState::Stopped {
        why: Stop::TimedOut,
        can_retry: false,
    });
    let settled = until_settled(&mut source, "", || {});
    assert!(matches!(settled, crate::frames::viewers::source::Settled::Failed));
    assert!(!source.saving() && engine.s().created.is_empty());
    // The editor off: nothing starts.
    let mut off = EngineViewerSource::on(TestEngine::new(&[]));
    off.office.edit_on = false;
    assert!(!off.create_document(None, "Invented.docx", EditKind::Docx, Template::Blank));
}

// ---------------------------------------------------------------------------------------------------------------
// Stale pixels and cells
// ---------------------------------------------------------------------------------------------------------------

fn page_key(page: u32) -> OfficePicture {
    OfficePicture::Page {
        page,
        zoom_pct: 100,
        dpi: 96,
    }
}

/// An editing view of `kind` over `fake`, open with `state`.
fn editing_view(fake: &Fake, kind: EditKind, state: HostState) -> OfficeView {
    let mut view = fake.editor(kind);
    fake.set(state);
    view.poll();
    view
}

/// Ask for `which` and let it come: held.
fn hold(fake: &Fake, view: &mut OfficeView, which: OfficePicture) {
    let _ = view.picture(which);
    if !view.holds(which) {
        fake.deliver(fake.last_page(), 8, 8, false);
        view.poll();
    }
    assert!(view.holds(which));
}

#[test]
fn an_edited_drops_the_pictures_it_names_and_nothing_older_is_drawn() {
    let fake = Fake::default();
    let mut view = editing_view(&fake, EditKind::Docx, pages(3));
    for page in 0..3 {
        hold(&fake, &mut view, page_key(page));
    }
    // Page one was redrawn: only its pictures go, at every scale.
    let other_scale = OfficePicture::Page {
        page: 0,
        zoom_pct: 150,
        dpi: 96,
    };
    hold(&fake, &mut view, other_scale);
    fake.answer(HostEdit::Edited(Box::new(edited(7, Dirty::Rects(vec![page_rect(0)])))));
    view.poll();
    assert!(
        !view.holds(page_key(0)) && !view.holds(other_scale),
        "the named page's pictures are gone"
    );
    assert!(view.holds(page_key(1)) && view.holds(page_key(2)));
    // The picture is not found for the old revision either: the old scale is not drawn in its place.
    assert!(view.picture(page_key(0)).is_none(), "no stale picture stands in");
    fake.deliver(fake.last_page(), 8, 8, false);
    view.poll();
    assert_eq!(view.held_at(page_key(0)), Some(7), "held under the revision it claims");
    // Everything redrawn: every picture of the older revision goes.
    fake.answer(HostEdit::Edited(Box::new(edited(8, Dirty::All))));
    view.poll();
    assert_eq!(view.held_pictures().0, 0);
}

#[test]
fn what_was_asked_before_an_edit_is_not_drawn_after_it() {
    let fake = Fake::default();
    let mut view = editing_view(&fake, EditKind::Docx, pages(2));
    assert!(view.picture(page_key(1)).is_none());
    let asked = fake.last_page();
    assert!(view.edit(typed()));
    assert!(fake.log().cancels.contains(&asked), "the ask older than the edit ends");
    // Its answer still comes (the helper rendered it before the edit): never held.
    fake.deliver(asked, 8, 8, false);
    view.poll();
    assert_eq!(view.held_pictures().0, 0);
    // Asked again after the edit: that one is drawn.
    assert!(view.picture(page_key(1)).is_none());
    fake.deliver(fake.last_page(), 8, 8, false);
    view.poll();
    assert!(view.holds(page_key(1)));
}

#[test]
fn a_changed_page_count_or_size_updates_the_body_in_place() {
    let fake = Fake::default();
    let mut view = editing_view(&fake, EditKind::Docx, pages(2));
    hold(&fake, &mut view, page_key(0));
    let wide = PageSize {
        w_pt: 700.0,
        h_pt: 842.0,
    };
    // Three pages now; the host's state says so before the `Edited` does.
    let mut change = edited(2, Dirty::All);
    change.pages = Some(WireOpened {
        pages: 3,
        sizes: vec![A4, A4, wide],
    });
    fake.set(HostState::Pages {
        pages: 3,
        sizes: vec![A4, A4, wide],
    });
    fake.answer(HostEdit::Edited(Box::new(change)));
    let Some(Body::Doc(doc)) = view.poll() else {
        panic!("still the document, not a state")
    };
    assert_eq!(doc.view.pages, 3);
    assert_eq!(doc.view.page_sizes_pt.last(), Some(&(700.0, 842.0)));
    assert_eq!(view.held_pictures().0, 0, "the pictures of the old pages went");
    // Same count, another size: not ignored.
    fake.set(HostState::Pages {
        pages: 3,
        sizes: vec![A4, wide, wide],
    });
    let Some(Body::Doc(doc)) = view.poll() else {
        panic!("still the document")
    };
    assert_eq!(doc.view.page_sizes_pt[1], (700.0, 842.0));
    // A deck's count follows too, and the slide on the stage stays.
    let deck_fake = Fake::default();
    let mut deck = editing_view(&deck_fake, EditKind::Pptx, pages(3));
    let mut change = edited(1, Dirty::All);
    change.slides = Some(5);
    change.pages = Some(WireOpened {
        pages: 5,
        sizes: vec![A4; 5],
    });
    deck_fake.set(pages(5));
    deck_fake.answer(HostEdit::Edited(Box::new(change)));
    let Some(Body::Deck(body)) = deck.poll() else {
        panic!("still the deck")
    };
    assert_eq!((body.view.count, body.view.slides.len(), body.view.current), (5, 5, 1));
}

fn sheet_view(fake: &Fake) -> OfficeView {
    editing_view(fake, EditKind::Xlsx, sheets(100, 10))
}

fn cells_of(view: &mut OfficeView) -> Vec<(u32, u32, String)> {
    let Body::Sheet(sheet) = view.held_body() else {
        panic!("a sheet")
    };
    let mut cells: Vec<_> = sheet.cells.iter().map(|c| (c.row, c.col, c.text.clone())).collect();
    cells.sort();
    cells
}

#[test]
fn an_edited_drops_the_cell_blocks_it_changed_and_a_re_asked_block_replaces_the_old_one_whole() {
    let fake = Fake::default();
    let mut view = sheet_view(&fake);
    assert_eq!(fake.log().cells, vec![(0, range(0, 0, 64, 10))]);
    fake.cells(
        0,
        range(0, 0, 64, 10),
        vec![
            cell(0, 0, "a", None, None),
            cell(1, 1, "b", None, None),
            cell(70, 1, "far", None, None),
        ],
        Vec::new(),
    );
    view.poll();
    assert_eq!(cells_of(&mut view).len(), 3);
    // Cell B2 changed.
    let mut change = edited(3, Dirty::Rects(Vec::new()));
    change.cells = vec![(0, range(1, 1, 1, 1))];
    fake.answer(HostEdit::Edited(Box::new(change)));
    view.poll();
    assert_eq!(
        cells_of(&mut view),
        vec![(70, 1, "far".to_string())],
        "the block it touched is dropped"
    );
    // The next draw asks the block again; the answer replaces it whole (A1 is empty now).
    view.wants_cells(0, 0, 0, 10, 4);
    assert_eq!(fake.log().cells.len(), 2, "asked again");
    fake.cells(0, range(0, 0, 64, 10), vec![cell(1, 1, "b2", None, None)], Vec::new());
    view.poll();
    assert_eq!(
        cells_of(&mut view),
        vec![(1, 1, "b2".to_string()), (70, 1, "far".to_string())],
        "a cell missing from the answer is empty now"
    );
    // A cell edited past the sheet's last row grows the sheet.
    let mut change = edited(4, Dirty::Rects(Vec::new()));
    change.cells = vec![(0, range(120, 0, 1, 1))];
    fake.answer(HostEdit::Edited(Box::new(change)));
    let Some(Body::Sheet(sheet)) = view.poll() else {
        panic!("a sheet")
    };
    assert_eq!(sheet.extent.0, 121);
}

#[test]
fn a_block_of_cells_asked_before_an_edit_is_not_drawn_after_it() {
    let fake = Fake::default();
    let mut view = sheet_view(&fake);
    // The first block is on its way; an edit is sent; the old block's answer comes.
    assert!(view.edit(EditOp::SetCell {
        sheet: 0,
        row: 0,
        col: 0,
        text: "new".into(),
    }));
    fake.cells(0, range(0, 0, 64, 10), vec![cell(0, 0, "old", None, None)], Vec::new());
    view.poll();
    assert!(cells_of(&mut view).is_empty(), "older than the edit: dropped");
    // It is asked again, and that answer is drawn.
    view.wants_cells(0, 0, 0, 10, 4);
    assert_eq!(fake.log().cells.len(), 2);
    fake.cells(0, range(0, 0, 64, 10), vec![cell(0, 0, "new", None, None)], Vec::new());
    view.poll();
    assert_eq!(cells_of(&mut view), vec![(0, 0, "new".to_string())]);
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
    assert!(view
        .picture(OfficePicture::Page {
            page: 0,
            zoom_pct: 100,
            dpi: 96
        })
        .is_none());
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
    // A deck's slides are the size the file says (4:3 here), not the 16:9 the view starts from.
    let four_three = HostState::Pages {
        pages: 2,
        sizes: vec![
            PageSize {
                w_pt: 720.0,
                h_pt: 540.0
            };
            2
        ],
    };
    assert!(matches!(body_for(Pptx, four_three), Some(Body::Deck(deck)) if deck.view.page_pt == (720, 540)));
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
            h: 512
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
    // The view scrolled on: every build from here wants the first page, held, and never the seventh.
    let first = OfficePicture::Page {
        page: 0,
        zoom_pct: 100,
        dpi: 96,
    };
    view.poll(); // the frame that asked
    view.picture(first);
    fake.deliver(fake.last_page(), 8, 8, false);
    view.poll(); // one build without it
    assert!(fake.log().cancels.is_empty());
    view.picture(first);
    view.poll(); // a second
    assert_eq!(fake.log().cancels, vec![id]);
    view.picture(first);
    view.poll();
    assert_eq!(fake.log().cancels, vec![id], "cancelled once");
    // Wanted again later: asked anew.
    view.picture(seventh);
    assert_eq!(fake.log().pages.len(), 3);
}

/// A frame that changes nothing polls and does not build (a pointer that moves, a popover that closes, 240 of them
/// a second on a fast screen). Such a poll says nothing about what the view wants: an ask the last build wanted
/// stays out until its answer comes, however many of them pass (dc-R3: at 144 dpi every tile an `Edited` asked for
/// was cancelled on the third poll, and nothing asked again until the next build, so the page stayed as it was).
#[test]
fn a_poll_without_a_build_cancels_nothing_the_last_build_wanted() {
    let fake = Fake::default();
    let mut view = editing_view(&fake, EditKind::Docx, pages(1));
    // A 150 % display at 100 % zoom: the page is drawn in tiles (3 across, 4 down for A4).
    assert!(kinds::uses_tiles(100, 144));
    let tiles: Vec<OfficePicture> = (0..4)
        .flat_map(|row| {
            (0..3).map(move |col| OfficePicture::Tile {
                page: 0,
                zoom_pct: 100,
                dpi: 144,
                col,
                row,
            })
        })
        .collect();
    let build = |view: &OfficeView| tiles.iter().filter(|tile| view.picture(**tile).is_some()).count();

    // An edit redrew page one (a text colour): the poll says so, and the build that follows asks its tiles.
    fake.answer(HostEdit::Edited(Box::new(edited(3, Dirty::Rects(vec![page_rect(0)])))));
    assert!(view.poll().is_some(), "an Edited builds the view again");
    assert_eq!(build(&view), 0, "nothing of the older revision is drawn");
    let asked: Vec<u64> = fake.log().pages.iter().map(|page| page.0).collect();
    assert_eq!(asked.len(), tiles.len(), "every tile is asked once");

    // Three frames that change nothing, before the helper answered.
    for _ in 0..3 {
        assert!(view.poll().is_none(), "nothing changed: no build");
    }
    assert!(
        fake.log().cancels.is_empty(),
        "no ask the last build wanted is cancelled"
    );

    // The answers come: each is drawn.
    for id in &asked {
        fake.deliver(*id, 8, 8, false);
    }
    assert!(view.poll().is_some(), "the answers build the view again");
    assert_eq!(build(&view), tiles.len(), "every tile of the edited page is drawn");
    assert_eq!(fake.log().pages.len(), tiles.len(), "and none was asked twice");
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
    assert!(sheet
        .cells
        .iter()
        .all(|cell| cell.text.is_empty() && cell.value.is_none() && cell.formula.as_deref().is_none_or(str::is_empty)));
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
    assert_eq!(source.office_live(), cfg!(any(windows, target_os = "macos")));
}

// ---------------------------------------------------------------------------------------------------------------
// Recovery: snapshots are kept, survive a restart and restore; a save clears them (w05-recovery-spill-wire)
// ---------------------------------------------------------------------------------------------------------------

fn snapshot(fake: &Fake, bytes: &[u8], revision: u64) {
    fake.answer(HostEdit::Saved {
        id: 900 + revision,
        purpose: Purpose::Snapshot,
        revision,
        bytes: Zeroizing::new(bytes.to_vec()),
    });
}

#[test]
fn a_snapshot_is_kept_survives_a_restart_and_restores() {
    let mut e = editor("Invented.docx", false);
    snapshot(&e.fake, b"SNAP-ONE", 2);
    snapshot(&e.fake, b"SNAP-TWO", 3);
    assert!(
        e.source.poll_edit("f1").is_none(),
        "a snapshot is no answer to the window"
    );
    let kept = e.engine.s().blobs.get("f1").cloned().expect("kept under the file's id");
    assert_eq!(
        (kept.1, kept.2),
        (3, b"SNAP-TWO".to_vec()),
        "one blob per file: the newest, from the file's version"
    );

    // The app restarts: a new source over the same engine (this PC's blobs stayed).
    let engine = e.engine.clone();
    drop(e);
    let mut again = editor_over(engine, "Invented.docx");
    let offer = again.source.office_recovery("f1").expect("offered after the restart");
    let pending = again.source.office_recovery_pending();
    assert_eq!(pending.len(), 1);
    assert_eq!(
        (pending[0].file_id.as_str(), pending[0].at.as_str()),
        ("f1", offer.at.as_str())
    );
    assert!(again.source.office_restore("f1"));
    // A new editing session opened on the blob's bytes, and the editor was told.
    assert_eq!(again.edits.lock().unwrap().last().map(|(_, len)| *len), Some(8));
    assert!(matches!(
        again.source.poll_edit("f1"),
        Some(EditAnswer::Restored { .. })
    ));
    assert!(
        again.engine.s().blobs.contains_key("f1"),
        "the blob stays until a save or a discard"
    );
    // Not for another file, and not twice without a blob.
    assert!(!again.source.office_restore("f2"));
    again.source.office_discard_recovery("f1");
    assert!(again.source.office_recovery("f1").is_none() && !again.source.office_restore("f1"));
}

#[test]
fn a_restore_saves_from_the_blobs_version_so_a_newer_one_conflicts() {
    // The blob was edited from version 2; the file is at 3 when it is opened again.
    let engine = TestEngine::new(&[("f1", "Invented.docx", 64)]);
    engine
        .s()
        .blobs
        .insert("f1".into(), (86_400 * 100, 2, b"KEPT".to_vec()));
    let mut e = editor_over(engine, "Invented.docx");
    assert!(e.source.office_restore("f1"));
    // The restored editing session opens (the fake is shared with the one the restore closed, so it says closed until
    // then) and a frame reads that: Save is refused before.
    e.fake.set(open_state(OfficeKind::Docx));
    e.source.poll("f1");
    assert!(matches!(e.source.save_document("f1", 3), SaveOutcome::Working));
    e.source.poll("f1");
    e.fake.saved(b"PK saved package", 4);
    let settled = until_settled(&mut e.source, "f1", || {});
    assert!(
        matches!(settled, crate::frames::viewers::source::Settled::Conflict(_)),
        "the newer version is not overwritten: {settled:?}"
    );
    assert!(e.engine.s().stored.is_empty());
}

#[test]
fn a_landed_save_deletes_the_blob() {
    let mut e = editor("Invented.docx", false);
    snapshot(&e.fake, b"SNAP", 2);
    let _ = e.source.poll_edit("f1");
    assert!(e.engine.s().blobs.contains_key("f1"));
    assert!(matches!(e.source.save_document("f1", 3), SaveOutcome::Working));
    e.source.poll("f1");
    e.fake.saved(b"PK saved package", 4);
    let settled = until_settled(&mut e.source, "f1", || {});
    assert!(matches!(settled, crate::frames::viewers::source::Settled::Saved { .. }));
    assert!(e.engine.s().blobs.is_empty(), "the blob is gone with the save");
}

#[test]
fn a_viewer_and_a_read_only_file_keep_nothing() {
    let mut e = editor("Invented.docx", true);
    snapshot(&e.fake, b"SNAP", 2);
    let _ = e.source.poll_edit("f1");
    assert!(e.engine.s().blobs.is_empty());
    assert!(!e.source.office_restore("f1"));
}

/// v5 bug 7: a save stored here and not sent yet keeps the blob (nothing is in Beebeeb), and once its upload landed
/// the blob goes, so reopening never offers Restore/Discard for a save the server has.
#[test]
fn a_queued_save_keeps_the_blob_until_it_landed() {
    use crate::frames::viewers::source::Settled;
    let mut e = editor("Invented.docx", false);
    snapshot(&e.fake, b"SNAP", 2);
    let _ = e.source.poll_edit("f1");
    e.engine.s().save_says = Some(Ok(SaveEnd::Queued { base: 3 }));
    assert!(matches!(e.source.save_document("f1", 3), SaveOutcome::Working));
    e.source.poll("f1");
    e.fake.saved(b"PK saved package", 4);
    let settled = until_settled(&mut e.source, "f1", || {});
    assert!(matches!(settled, Settled::Queued { .. }));
    assert!(
        e.engine.s().blobs.contains_key("f1"),
        "not in Beebeeb yet: the blob stays"
    );
    // Not landed: still there after the poll's looks.
    let end = Instant::now() + Duration::from_millis(2500);
    while Instant::now() < end {
        e.source.poll("f1");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(e.engine.s().blobs.contains_key("f1"), "still queued: the blob stays");
    // The upload lands.
    e.engine.s().own = true;
    let end = Instant::now() + Duration::from_secs(10);
    while e.engine.s().blobs.contains_key("f1") {
        assert!(Instant::now() < end, "the blob went when the save landed");
        e.source.poll("f1");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// A save confirmed later does not delete a blob kept after the person pressed Save: it may hold what was typed since.
#[test]
fn a_save_confirmed_later_keeps_a_blob_made_after_the_press() {
    let mut e = editor("Invented.docx", false);
    snapshot(&e.fake, b"SNAP", 2);
    let _ = e.source.poll_edit("f1");
    e.engine.s().save_says = Some(Ok(SaveEnd::Queued { base: 3 }));
    assert!(matches!(e.source.save_document("f1", 3), SaveOutcome::Working));
    e.source.poll("f1");
    e.fake.saved(b"PK saved package", 4);
    let settled = until_settled(&mut e.source, "f1", || {});
    assert!(matches!(
        settled,
        crate::frames::viewers::source::Settled::Queued { .. }
    ));
    // A blob from the far future of this clock is "newer than the press"; then the upload lands.
    e.engine
        .s()
        .blobs
        .insert("f1".into(), (i64::MAX / 2, 3, b"TYPED-AFTER".to_vec()));
    e.engine.s().own = true;
    let end = Instant::now() + Duration::from_millis(3500);
    while Instant::now() < end {
        e.source.poll("f1");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        e.engine.s().blobs.contains_key("f1"),
        "what was typed after the press is not in the saved version"
    );
}
