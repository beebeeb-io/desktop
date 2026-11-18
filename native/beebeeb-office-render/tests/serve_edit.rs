//! The helper's loop serving an editing session, end to end in one process (as `tests/serve.rs`): through a fake
//! engine, so these hold at any pin of the Office engine. The save handshake (size, grant, copy, a stale token
//! refused), edit asks on a viewer session refused, a `Cancel` on a queued edit, the caps on an `Edited`.

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use beebeeb_office_protocol::check::{Limits, MAX_DIRTY_RECTS, MAX_SELECTION_RECTS, check_edited, check_opened};
use beebeeb_office_protocol::office::{
    self, Cancel, CellData, CellRange, Clip, Copy as CopyAsk, Dirty, DocKind, Edit, EditKind, EditOp, Edited,
    ErrorCode, Hit, HitMode, InsertImage, LayoutList, Layouts, ListKind, Marks, Message, NewDoc, Open, OpenEdit,
    Opened, OutlineEntry, PageRect, PageSize, ParaStyle, Purpose, Rect, RectPt, SaveInto, SaveReady, SaveSize,
    SavedInto, Stats, Template, TextAlign,
};
use beebeeb_office_render::{EXIT_OK, EXIT_PROTOCOL, EditingDoc, Engine, OpenDoc, Opening};

use common::{A4, FAKE_SIZE, Fake, FakeEngine, FakeMemory, Running};

const L: Limits = Limits::CONTRACT;

fn error(code: ErrorCode) -> Message {
    Message::Error(office::Error { code })
}

/// What the fake editing document did, and what it answers.
struct EditFake {
    /// The ops it was applied, by `Debug` (which shows lengths, never text).
    ops: Mutex<Vec<String>>,
    applies: AtomicUsize,
    images: Mutex<Vec<(Vec<u8>, bool)>>,
    /// What `save` serialises.
    saved: Mutex<Vec<u8>>,
    clip: Mutex<office::Clip>,
    /// The first `apply` waits for a message here (the test's gate), when set.
    gate: Mutex<Option<Receiver<()>>>,
    /// Replaces the selection and dirty area of the next `Edited`.
    selection: Mutex<Vec<PageRect>>,
    dirty: Mutex<Option<Dirty>>,
}

impl EditFake {
    fn new() -> Arc<EditFake> {
        Arc::new(EditFake {
            ops: Mutex::default(),
            applies: AtomicUsize::new(0),
            images: Mutex::default(),
            saved: Mutex::new(b"PK-the-saved-document".to_vec()),
            clip: Mutex::new(Clip {
                text: "copied".to_owned(),
                html: Some("<b>copied</b>".to_owned()),
            }),
            gate: Mutex::new(None),
            selection: Mutex::default(),
            dirty: Mutex::new(None),
        })
    }

    fn applies(&self) -> usize {
        self.applies.load(Ordering::SeqCst)
    }
}

fn rect(page: u32, x: f32, y: f32, w: f32, h: f32) -> PageRect {
    PageRect {
        page,
        rect: RectPt { x, y, w, h },
    }
}

fn marks() -> Marks {
    Marks {
        bold: false,
        italic: false,
        underline: false,
        strike: false,
        style: ParaStyle::Normal,
        align: TextAlign::Left,
        list: ListKind::None,
    }
}

struct EditEngine {
    view: FakeEngine,
    edit: Arc<EditFake>,
}

impl Engine for EditEngine {
    fn open(&self, kind: DocKind, bytes: &[u8]) -> Result<Box<dyn OpenDoc>, ErrorCode> {
        self.view.open(kind, bytes)
    }

    fn open_edit(&self, _kind: EditKind, _bytes: &[u8]) -> Result<Box<dyn EditingDoc>, ErrorCode> {
        Ok(Box::new(FakeEdit {
            fake: Arc::clone(&self.edit),
            revision: 0,
        }))
    }

    fn new_doc(&self, _kind: EditKind, _template: Template) -> Result<Box<dyn EditingDoc>, ErrorCode> {
        self.open_edit(EditKind::Docx, &[])
    }
}

struct FakeEdit {
    fake: Arc<EditFake>,
    revision: u64,
}

impl FakeEdit {
    fn edited(&mut self) -> Edited {
        self.revision += 1;
        let selection = std::mem::take(&mut *self.fake.selection.lock().expect("lock"));
        let dirty = self.fake.dirty.lock().expect("lock").take();
        Edited {
            revision: self.revision,
            caret: Some(rect(0, 72.0, 72.0, 1.0, 14.0)),
            selection,
            can_undo: true,
            can_redo: false,
            dirty: dirty.unwrap_or(Dirty::Rects(vec![rect(0, 0.0, 60.0, 595.0, 30.0)])),
            pages: None,
            cells: Vec::new(),
            slides: None,
            marks: marks(),
        }
    }
}

impl OpenDoc for FakeEdit {
    fn opened(&self) -> Opening {
        Opening::Pages(Opened {
            pages: 1,
            sizes: vec![A4],
        })
    }

    fn render_size(&self, _page: u32, _scale: f32, _region: Option<Rect>) -> Result<(u32, u32), ErrorCode> {
        Ok(FAKE_SIZE)
    }

    fn render(&self, _page: u32, _scale: f32, _region: Option<Rect>, out: &mut [u8]) -> Result<bool, ErrorCode> {
        out.fill(9);
        Ok(false)
    }

    fn outline(&self) -> Vec<OutlineEntry> {
        Vec::new()
    }

    fn words(&self) -> u32 {
        11
    }

    fn cells(&self, _sheet: u32, _range: CellRange) -> Result<CellData, ErrorCode> {
        Err(ErrorCode::Unsupported)
    }

    fn anchor_size(&self, _sheet: u32, _anchor: u32, _scale: f32) -> Result<(u32, u32), ErrorCode> {
        Err(ErrorCode::Unsupported)
    }

    fn render_anchor(&self, _sheet: u32, _anchor: u32, _scale: f32, _out: &mut [u8]) -> Result<bool, ErrorCode> {
        Err(ErrorCode::Unsupported)
    }
}

impl EditingDoc for FakeEdit {
    fn apply(&mut self, op: EditOp) -> Result<Edited, ErrorCode> {
        let earlier = self.fake.applies.fetch_add(1, Ordering::SeqCst);
        if earlier == 0
            && let Some(gate) = self.fake.gate.lock().expect("lock").take()
        {
            gate.recv_timeout(common::WAIT).expect("the test opened the gate");
        }
        self.fake.ops.lock().expect("lock").push(format!("{op:?}"));
        Ok(self.edited())
    }

    fn insert_image(&mut self, bytes: Vec<u8>, png: bool) -> Result<Edited, ErrorCode> {
        self.fake.images.lock().expect("lock").push((bytes, png));
        Ok(self.edited())
    }

    fn hit(&mut self, _page: u32, _x_pt: f32, _y_pt: f32, _mode: HitMode) -> Result<Edited, ErrorCode> {
        Ok(self.edited())
    }

    fn save(&mut self) -> Result<(Vec<u8>, u64), ErrorCode> {
        Ok((self.fake.saved.lock().expect("lock").clone(), self.revision))
    }

    fn copy(&self) -> Clip {
        self.fake.clip.lock().expect("lock").clone()
    }

    fn layouts(&self) -> Vec<String> {
        vec!["Title".to_owned(), "Two columns".to_owned()]
    }
}

/// The helper over the edit fake, an editing session open (id 0).
fn open_edit(memory: &Arc<FakeMemory>, fake: &Arc<EditFake>) -> Running {
    let engine = EditEngine {
        view: FakeEngine(Arc::new(Fake::default())),
        edit: Arc::clone(fake),
    };
    let mut helper = Running::start(Arc::clone(memory), Arc::new(engine));
    let doc = memory.section(b"a document".to_vec());
    helper.send(
        0,
        &Message::OpenEdit(OpenEdit {
            doc,
            kind: EditKind::Docx,
        }),
    );
    match helper.answer() {
        (0, Message::Opened(opened)) => {
            check_opened(&opened, &L).expect("the app's check accepts the Opened");
            assert_eq!(memory.live_reads(), 0, "the document's section was unmapped");
        }
        other => panic!("Opened for 0 was due: {other:?}"),
    }
    helper
}

fn type_text() -> Message {
    Message::Edit(Edit {
        op: EditOp::InsertText("hello".to_owned()),
    })
}

fn edited(answer: (u32, Message), id: u32) -> Edited {
    match answer {
        (got, Message::Edited(edited)) if got == id => edited,
        other => panic!("Edited for {id} was due: {other:?}"),
    }
}

fn save_size() -> Message {
    Message::SaveSize(SaveSize { purpose: Purpose::Save })
}

fn ready(answer: (u32, Message), id: u32) -> SaveReady {
    match answer {
        (got, Message::SaveReady(ready)) if got == id => ready,
        other => panic!("SaveReady for {id} was due: {other:?}"),
    }
}

#[test]
fn serve_edit_the_save_handshake_copies_into_the_granted_section_and_drops() {
    let fake = EditFake::new();
    let memory = FakeMemory::new();
    let mut helper = open_edit(&memory, &fake);

    helper.send(2, &type_text());
    let done = edited(helper.answer(), 2);
    assert_eq!(done.revision, 1);
    check_edited(&done, &[A4], &L).expect("the app's check accepts the Edited");
    assert_eq!(
        fake.ops.lock().expect("lock").as_slice(),
        ["InsertText { text_len: 5 }"]
    );

    helper.send(4, &save_size());
    let first = ready(helper.answer(), 4);
    let saved = fake.saved.lock().expect("lock").clone();
    assert_eq!((first.len, first.revision), (saved.len() as u64, 1));
    beebeeb_office_protocol::check::check_save_ready(&first).expect("the app's check accepts SaveReady");

    // The app makes a section of exactly `len` and grants it.
    let out = memory.zeroed(saved.len());
    helper.send(
        6,
        &Message::SaveInto(SaveInto {
            token: first.token,
            out,
        }),
    );
    assert_eq!(helper.answer(), (6, Message::SavedInto(SavedInto { len: first.len })));
    assert_eq!(memory.bytes(out), saved, "the copy is in the app's section");
    assert_eq!(memory.maps(out), 1, "mapped once");
    assert_eq!(memory.live_writes(), 0, "and unmapped");

    // The copy was dropped: the same token again is stale.
    let again = memory.zeroed(saved.len());
    helper.send(
        8,
        &Message::SaveInto(SaveInto {
            token: first.token,
            out: again,
        }),
    );
    assert_eq!(helper.answer(), (8, error(ErrorCode::Internal)));
    assert_eq!(memory.bytes(again), vec![0; saved.len()], "nothing was written");
    assert_eq!(memory.maps(again), 1, "its grant was closed all the same");
    assert_eq!(memory.live_writes(), 0);

    helper.close();
    assert_eq!(helper.exit_code(), EXIT_OK);
}

#[test]
fn serve_edit_a_stale_token_is_refused_and_a_newer_save_stands() {
    let fake = EditFake::new();
    let memory = FakeMemory::new();
    let mut helper = open_edit(&memory, &fake);
    let len = fake.saved.lock().expect("lock").len();

    // A token never given.
    let out = memory.zeroed(len);
    helper.send(2, &Message::SaveInto(SaveInto { token: 77, out }));
    assert_eq!(helper.answer(), (2, error(ErrorCode::Internal)));
    assert_eq!(memory.maps(out), 1);

    // Two sizes: the older token is stale, the newer one works.
    helper.send(4, &save_size());
    let older = ready(helper.answer(), 4);
    *fake.saved.lock().expect("lock") = b"PK-newer".to_vec();
    helper.send(
        6,
        &Message::SaveSize(SaveSize {
            purpose: Purpose::Snapshot,
        }),
    );
    let newer = ready(helper.answer(), 6);
    assert_ne!(older.token, newer.token);
    assert_eq!(newer.len, 8);

    let stale_out = memory.zeroed(len);
    helper.send(
        8,
        &Message::SaveInto(SaveInto {
            token: older.token,
            out: stale_out,
        }),
    );
    assert_eq!(helper.answer(), (8, error(ErrorCode::Internal)));
    assert_eq!(memory.bytes(stale_out), vec![0; len]);

    let out = memory.zeroed(8);
    helper.send(
        10,
        &Message::SaveInto(SaveInto {
            token: newer.token,
            out,
        }),
    );
    assert_eq!(helper.answer(), (10, Message::SavedInto(SavedInto { len: 8 })));
    assert_eq!(memory.bytes(out), b"PK-newer".to_vec());

    // A section of another length than `len`: refused, nothing written, and the copy is gone.
    helper.send(12, &save_size());
    let third = ready(helper.answer(), 12);
    let wrong = memory.zeroed(third.len as usize + 1);
    helper.send(
        14,
        &Message::SaveInto(SaveInto {
            token: third.token,
            out: wrong,
        }),
    );
    assert_eq!(helper.answer(), (14, error(ErrorCode::Internal)));
    assert_eq!(memory.bytes(wrong), vec![0; third.len as usize + 1]);
    assert_eq!(memory.live_writes(), 0);

    helper.close();
    assert_eq!(helper.exit_code(), EXIT_OK);
}

#[test]
fn serve_edit_edit_asks_on_a_view_session_are_refused() {
    let memory = FakeMemory::new();
    let fake = Arc::new(Fake::default());
    let mut helper = Running::start(Arc::clone(&memory), Arc::new(FakeEngine(Arc::clone(&fake))));
    helper.open_fake(&memory);

    let out = memory.zeroed(4);
    let image = memory.section(vec![1, 2, 3]);
    let asks = [
        type_text(),
        Message::Hit(Hit {
            page: 0,
            x_pt: 1.0,
            y_pt: 1.0,
            mode: HitMode::Place,
        }),
        save_size(),
        Message::SaveInto(SaveInto { token: 1, out }),
        Message::Copy(CopyAsk),
        Message::Layouts(Layouts),
        Message::InsertImage(InsertImage {
            bytes: image,
            png: true,
        }),
    ];
    for (n, ask) in asks.iter().enumerate() {
        let id = 2 + 2 * n as u32;
        helper.send(id, ask);
        assert_eq!(helper.answer(), (id, error(ErrorCode::Unsupported)), "ask {n}");
    }
    assert_eq!(memory.maps(out), 1, "the grant it carried was closed");
    assert_eq!(memory.maps(image), 1);
    assert_eq!((memory.live_reads(), memory.live_writes()), (0, 0));

    // The session goes on.
    helper.send(20, &Message::Stats(Stats));
    assert_eq!(
        helper.answer(),
        (20, Message::StatsData(office::StatsData { words: 7 }))
    );
    helper.close();
    assert_eq!(helper.exit_code(), EXIT_OK);
}

#[test]
fn serve_edit_a_cancel_read_before_a_queued_edit_starts_skips_it() {
    let fake = EditFake::new();
    let (open_gate, gate) = mpsc::channel();
    *fake.gate.lock().expect("lock") = Some(gate);
    let memory = FakeMemory::new();
    let mut helper = open_edit(&memory, &fake);

    let image = memory.section(vec![7; 3]);
    let out = memory.zeroed(8);
    helper.send(2, &type_text());
    helper.send(4, &type_text());
    helper.send(
        6,
        &Message::InsertImage(InsertImage {
            bytes: image,
            png: false,
        }),
    );
    helper.send(8, &Message::SaveInto(SaveInto { token: 1, out }));
    helper.send(4, &Message::Cancel(Cancel));
    helper.send(6, &Message::Cancel(Cancel));
    helper.send(8, &Message::Cancel(Cancel));
    helper.wait_handed_out();
    thread::sleep(Duration::from_millis(50));
    open_gate.send(()).expect("the engine waits");

    let first = edited(helper.answer(), 2);
    assert_eq!(first.revision, 1);
    assert_eq!(helper.answer(), (4, error(ErrorCode::Internal)));
    assert_eq!(helper.answer(), (6, error(ErrorCode::Internal)));
    assert_eq!(helper.answer(), (8, error(ErrorCode::Internal)));
    assert_eq!(fake.applies(), 1, "the cancelled edit never reached the engine");
    assert!(
        fake.images.lock().expect("lock").is_empty(),
        "nor the cancelled picture"
    );
    assert_eq!(memory.maps(image), 1, "its grant was closed unread");
    assert_eq!(memory.maps(out), 1);
    assert_eq!((memory.live_reads(), memory.live_writes()), (0, 0));

    helper.close();
    assert_eq!(helper.exit_code(), EXIT_OK);
}

#[test]
fn serve_edit_an_image_is_read_once_from_its_grant_and_the_grant_closed() {
    let fake = EditFake::new();
    let memory = FakeMemory::new();
    let mut helper = open_edit(&memory, &fake);
    let image = memory.section(vec![0x89, b'P', b'N', b'G']);
    helper.send(
        2,
        &Message::InsertImage(InsertImage {
            bytes: image,
            png: true,
        }),
    );
    edited(helper.answer(), 2);
    assert_eq!(
        fake.images.lock().expect("lock").as_slice(),
        [(vec![0x89, b'P', b'N', b'G'], true)]
    );
    assert_eq!(memory.maps(image), 1);
    assert_eq!(memory.live_reads(), 0);
    helper.close();
    assert_eq!(helper.exit_code(), EXIT_OK);
}

#[test]
fn serve_edit_copy_and_layouts_fit_one_frame() {
    let fake = EditFake::new();
    let memory = FakeMemory::new();
    let mut helper = open_edit(&memory, &fake);

    helper.send(2, &Message::Copy(CopyAsk));
    assert_eq!(
        helper.answer(),
        (
            2,
            Message::Clip(office::Clip {
                text: "copied".to_owned(),
                html: Some("<b>copied</b>".to_owned())
            })
        )
    );
    helper.send(4, &Message::Layouts(Layouts));
    let (_, answer) = helper.answer();
    match answer {
        Message::LayoutList(LayoutList { names }) => assert_eq!(names, ["Title", "Two columns"]),
        other => panic!("LayoutList was due: {other:?}"),
    }

    // The html alone too large: the text goes alone. The text too large: `TooLarge`.
    *fake.clip.lock().expect("lock") = Clip {
        text: "t".to_owned(),
        html: Some("h".repeat(70_000)),
    };
    helper.send(6, &Message::Copy(CopyAsk));
    assert_eq!(
        helper.answer(),
        (
            6,
            Message::Clip(office::Clip {
                text: "t".to_owned(),
                html: None
            })
        )
    );
    *fake.clip.lock().expect("lock") = Clip {
        text: "t".repeat(70_000),
        html: None,
    };
    helper.send(8, &Message::Copy(CopyAsk));
    assert_eq!(helper.answer(), (8, error(ErrorCode::TooLarge)));

    helper.close();
    assert_eq!(helper.exit_code(), EXIT_OK);
}

#[test]
fn serve_edit_an_edited_is_held_to_the_apps_caps() {
    let fake = EditFake::new();
    let memory = FakeMemory::new();
    let mut helper = open_edit(&memory, &fake);

    // Too many selection rectangles are cut; too many dirty rectangles become a full redraw.
    let one = rect(0, 10.0, 10.0, 20.0, 10.0);
    *fake.selection.lock().expect("lock") = vec![one; MAX_SELECTION_RECTS + 500];
    *fake.dirty.lock().expect("lock") = Some(Dirty::Rects(vec![one; MAX_DIRTY_RECTS + 1]));
    helper.send(2, &type_text());
    let done = edited(helper.answer(), 2);
    assert_eq!(done.selection.len(), MAX_SELECTION_RECTS);
    assert_eq!(done.dirty, Dirty::All);
    check_edited(&done, &[A4], &L).expect("the app's check accepts it");

    // A rectangle outside its page is not sent: the app would refuse it.
    *fake.dirty.lock().expect("lock") = Some(Dirty::Rects(vec![rect(0, 500.0, 10.0, 200.0, 10.0)]));
    helper.send(4, &type_text());
    assert_eq!(helper.answer(), (4, error(ErrorCode::Internal)));

    // Hit goes the same way.
    helper.send(
        6,
        &Message::Hit(Hit {
            page: 0,
            x_pt: 72.0,
            y_pt: 72.0,
            mode: HitMode::Word,
        }),
    );
    edited(helper.answer(), 6);

    helper.close();
    assert_eq!(helper.exit_code(), EXIT_OK);
}

#[test]
fn serve_edit_a_new_document_opens_without_a_grant_and_the_viewer_asks_still_work() {
    let fake = EditFake::new();
    let memory = FakeMemory::new();
    let engine = EditEngine {
        view: FakeEngine(Arc::new(Fake::default())),
        edit: Arc::clone(&fake),
    };
    let mut helper = Running::start(Arc::clone(&memory), Arc::new(engine));
    helper.send(
        0,
        &Message::NewDoc(NewDoc {
            kind: EditKind::Docx,
            template: Template::Letter,
        }),
    );
    match helper.answer() {
        (0, Message::Opened(opened)) => assert_eq!(opened.sizes, vec![PageSize { ..A4 }]),
        other => panic!("Opened for 0 was due: {other:?}"),
    }
    assert_eq!(memory.all_maps(), 0, "no grant was involved");
    // A viewer ask on an editing session is answered from the edited state.
    helper.send(2, &Message::Stats(Stats));
    assert_eq!(
        helper.answer(),
        (2, Message::StatsData(office::StatsData { words: 11 }))
    );
    helper.close();
    assert_eq!(helper.exit_code(), EXIT_OK);
}

#[test]
fn serve_edit_an_edit_ask_before_the_open_or_a_second_open_ends_the_helper() {
    // Before any open.
    let memory = FakeMemory::new();
    let mut helper = Running::start(
        Arc::clone(&memory),
        Arc::new(EditEngine {
            view: FakeEngine(Arc::new(Fake::default())),
            edit: EditFake::new(),
        }),
    );
    helper.send(0, &type_text());
    assert_eq!(helper.exit_code(), EXIT_PROTOCOL);
    helper.nothing_more();

    // A second open of any of the three kinds.
    let fake = EditFake::new();
    let memory = FakeMemory::new();
    let mut helper = open_edit(&memory, &fake);
    let doc = memory.section(b"again".to_vec());
    helper.send(
        2,
        &Message::Open(Open {
            kind: DocKind::Docx,
            doc,
        }),
    );
    assert_eq!(helper.exit_code(), EXIT_PROTOCOL);
}
