//! The helper's loop: frames in, one answer per request out.
//!
//! - A **reader thread** owns the input. It reads frames ahead into a channel, so a `Cancel` queued behind a
//!   request is seen before that request starts, and the app's writes never wait on a full pipe. The end of the
//!   input, or a frame it cannot read, ends it; the worker is told.
//! - The **worker** (the calling thread, which also holds the open document) takes one request at a time, in the
//!   order received, and answers each exactly once with the request's id in the frame header. Before it starts a
//!   request it takes everything the reader has queued, without waiting: a `Cancel` marks the queued request with
//!   its id, and a marked request is answered `Error { Internal }` without any work (the app discards an answer to
//!   an id it cancelled). A request that has started finishes and is answered normally.
//! - The first frame must be `Open` (a viewer session), `OpenEdit` or `NewDoc` (an editing session; it answers every
//!   viewer request too, on the current state of the edit). The edit asks are served on an editing session only: on a
//!   viewer session each is answered `Error { Unsupported }`. Anything the helper cannot trust ends it at once with [`EXIT_PROTOCOL`] and no
//!   answer; the end of the input ends it at once with [`EXIT_OK`] (queued requests are dropped: the app is gone or
//!   is killing the helper); a failed write ends it with [`EXIT_IO`].
//! - It never sends a request of its own: the helper-side (odd) ids stay unused.

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use beebeeb_office_protocol::check::{Limits, MAX_INPUT_BYTES, MAX_SAVE_BYTES, check_edited};
use beebeeb_office_protocol::frame::{Frame, FrameError, Side, read_frame, write_frame};
use beebeeb_office_protocol::office::{
    self, CellData, Cells, Clip, Edit, Edited, ErrorCode, GrantRef, Hit, InsertImage, KIND_CELL_DATA, KIND_CLIP,
    KIND_ERROR, Message, NewDoc, Open, OpenEdit, Outline, OutlineEntry, PageSize, Render, RenderAnchor, Rendered,
    SaveInto, SaveReady, SavedInto, StatsData,
};

use crate::engine::{EditingDoc, Engine, OpenDoc, Opening};
use crate::memory::Memory;
use crate::save::{Refused, Saves};
use crate::{EXIT_IO, EXIT_OK, EXIT_PROTOCOL, fit};

/// Serves one document: reads frames from `input`, answers on `output`, maps grants through `memory`, opens and
/// renders through `engine`. Returns the exit code ([`crate::EXIT_OK`], [`crate::EXIT_PROTOCOL`],
/// [`crate::EXIT_IO`]).
pub fn serve(input: Box<dyn Read + Send>, output: &mut dyn Write, memory: &dyn Memory, engine: &dyn Engine) -> u8 {
    let (tx, rx) = mpsc::channel();
    // Never joined: on a protocol error it may still be waiting in a read; the process exit (or, in tests, the end of
    // the test's pipe) ends it.
    let _detached = thread::spawn(move || read_ahead(input, tx));
    match work(&rx, output, memory, engine) {
        Ok(()) => EXIT_OK,
        Err(code) => code,
    }
}

/// What the reader hands the worker.
enum Input {
    Frame(Frame),
    End(FrameError),
}

fn read_ahead(mut input: Box<dyn Read + Send>, tx: Sender<Input>) {
    loop {
        match read_frame(&mut input) {
            Ok(frame) => {
                if tx.send(Input::Frame(frame)).is_err() {
                    // The worker has ended.
                    return;
                }
            }
            Err(end) => {
                // Whether the worker is still there to hear it does not matter: the reader is done either way.
                let _ = tx.send(Input::End(end));
                return;
            }
        }
    }
}

/// A request the app may ask, once the document is open.
enum Ask {
    Open(Open),
    OpenEdit(OpenEdit),
    NewDoc(NewDoc),
    Edit(Edit),
    Hit(Hit),
    SaveSize,
    SaveInto(SaveInto),
    Copy,
    Layouts,
    InsertImage(InsertImage),
    Render(Render),
    RenderAnchor(RenderAnchor),
    Cells(Cells),
    Outline(Outline),
    Stats,
}

/// A request received and not started yet.
struct Pending {
    id: u32,
    ask: Ask,
    cancelled: bool,
}

/// The requests received and not started, in order.
#[derive(Default)]
struct Queue {
    pending: VecDeque<Pending>,
    /// An `Open`, `OpenEdit` or `NewDoc` has been received: every frame before it, and every one of the three after
    /// it, is a protocol error.
    opened: bool,
}

/// The document and what is kept of it between requests.
struct Session {
    doc: Doc,
    /// Worked out on the first `Outline`, then kept for the pages after it.
    outline: Option<Vec<OutlineEntry>>,
    /// The page sizes the app holds, for the check on every `Edited` (empty for a workbook).
    pages: Vec<PageSize>,
    /// The serialised copy awaiting its section (an editing session's).
    saves: Saves,
}

/// A viewer session's document, or an editing session's.
enum Doc {
    View(Box<dyn OpenDoc>),
    Edit(Box<dyn EditingDoc>),
}

impl Doc {
    /// What every session answers.
    fn view(&self) -> &dyn OpenDoc {
        match self {
            Doc::View(doc) => &**doc,
            Doc::Edit(doc) => &**doc,
        }
    }
}

/// `Err(code)` everywhere below ends the helper with `code`.
type Step = Result<(), u8>;

fn work(rx: &Receiver<Input>, output: &mut dyn Write, memory: &dyn Memory, engine: &dyn Engine) -> Step {
    let mut queue = Queue::default();
    let mut answers = Answers { out: output };
    let mut session: Option<Session> = None;
    loop {
        if queue.pending.is_empty() {
            // Nothing to do: wait for the next frame. A reader gone without a word is the input gone.
            let input = rx.recv().map_err(|_| EXIT_OK)?;
            take(&mut queue, input)?;
        }
        // Everything already read, without waiting: a `Cancel` behind a queued request is seen before it starts.
        while let Ok(input) = rx.try_recv() {
            take(&mut queue, input)?;
        }
        let Some(next) = queue.pending.pop_front() else {
            continue;
        };
        match next.ask {
            ask @ (Ask::Open(_) | Ask::OpenEdit(_) | Ask::NewDoc(_)) => {
                if next.cancelled {
                    // Cancelled before it started: nothing is opened, and with nothing open the helper is done.
                    answers.error(next.id, ErrorCode::Internal)?;
                    return Err(EXIT_OK);
                }
                session = Some(open_document(next.id, ask, &mut answers, memory, engine)?);
            }
            ask => {
                // `take` lets nothing but an `Open` in first, and a failed `Open` ends the helper.
                let Some(current) = session.as_mut() else {
                    return Err(EXIT_PROTOCOL);
                };
                if next.cancelled {
                    skip(next.id, &ask, &mut answers, memory)?;
                } else {
                    answer(next.id, ask, current, &mut answers, memory)?;
                }
            }
        }
    }
}

/// One frame from the reader: queued, a `Cancel` noted, or the end of the helper.
fn take(queue: &mut Queue, input: Input) -> Step {
    let frame = match input {
        Input::Frame(frame) => frame,
        Input::End(FrameError::Closed) => return Err(EXIT_OK),
        Input::End(FrameError::Io(_)) => return Err(EXIT_IO),
        Input::End(FrameError::TooLong | FrameError::TooShort | FrameError::Truncated) => return Err(EXIT_PROTOCOL),
    };
    // The app opens requests with even ids; an odd id is the helper's side, never the app's.
    if Side::of(frame.id) != Side::App {
        return Err(EXIT_PROTOCOL);
    }
    // An unknown kind or a payload that does not decode.
    let message = Message::decode(frame.kind, &frame.payload).map_err(|_| EXIT_PROTOCOL)?;
    let ask = match message {
        Message::Open(_) | Message::OpenEdit(_) | Message::NewDoc(_) if queue.opened => return Err(EXIT_PROTOCOL),
        Message::Open(open) => {
            queue.opened = true;
            Ask::Open(open)
        }
        Message::OpenEdit(open) => {
            queue.opened = true;
            Ask::OpenEdit(open)
        }
        Message::NewDoc(new) => {
            queue.opened = true;
            Ask::NewDoc(new)
        }
        // The first frame must open the document.
        _ if !queue.opened => return Err(EXIT_PROTOCOL),
        Message::Cancel(_) => {
            // Only a request still queued can be skipped; one already started or answered is not touched.
            if let Some(queued) = queue.pending.iter_mut().find(|p| p.id == frame.id) {
                queued.cancelled = true;
            }
            return Ok(());
        }
        Message::Render(render) => Ask::Render(render),
        Message::RenderAnchor(ask) => Ask::RenderAnchor(ask),
        Message::Cells(cells) => Ask::Cells(cells),
        Message::Outline(outline) => Ask::Outline(outline),
        Message::Stats(_) => Ask::Stats,
        Message::Edit(edit) => Ask::Edit(edit),
        Message::Hit(hit) => Ask::Hit(hit),
        Message::SaveSize(_) => Ask::SaveSize,
        Message::SaveInto(into) => Ask::SaveInto(into),
        Message::Copy(_) => Ask::Copy,
        Message::Layouts(_) => Ask::Layouts,
        Message::InsertImage(image) => Ask::InsertImage(image),
        // An answer kind from the app.
        Message::Edited(_)
        | Message::SaveReady(_)
        | Message::SavedInto(_)
        | Message::Clip(_)
        | Message::LayoutList(_)
        | Message::EditGuarded(_)
        | Message::Opened(_)
        | Message::OpenedSheets(_)
        | Message::Rendered(_)
        | Message::CellData(_)
        | Message::OutlineData(_)
        | Message::StatsData(_)
        | Message::Error(_) => return Err(EXIT_PROTOCOL),
    };
    queue.pending.push_back(Pending {
        id: frame.id,
        ask,
        cancelled: false,
    });
    Ok(())
}

/// Opens the document and answers `Opened` or `OpenedSheets`. A document that cannot be opened is answered with its
/// code and ends the helper ([`EXIT_OK`] once the answer is written).
fn open_document(
    id: u32,
    ask: Ask,
    answers: &mut Answers<'_>,
    memory: &dyn Memory,
    engine: &dyn Engine,
) -> Result<Session, u8> {
    match opened(ask, memory, engine) {
        Ok((doc, first)) => {
            let pages = match (&first, doc.view().opened()) {
                (Message::Opened(_), Opening::Pages(opened)) => opened.sizes,
                _ => Vec::new(),
            };
            answers.send(id, &first)?;
            Ok(Session {
                doc,
                outline: None,
                pages,
                saves: Saves::default(),
            })
        }
        Err(NotOpened::Code(code)) => {
            answers.error(id, code)?;
            Err(EXIT_OK)
        }
        // The editor would drop parts of this file on save: no editing session; the app opens the viewer.
        Err(NotOpened::Guarded(dropped)) => {
            answers.send(id, &Message::EditGuarded(office::EditGuarded { dropped }))?;
            Err(EXIT_OK)
        }
    }
}

/// Why no document session started: an error code, or the edit-open guard ([`crate::edit_guard`]).
enum NotOpened {
    Code(ErrorCode),
    Guarded(Vec<office::DroppedPart>),
}

impl From<ErrorCode> for NotOpened {
    fn from(code: ErrorCode) -> Self {
        NotOpened::Code(code)
    }
}

fn opened(ask: Ask, memory: &dyn Memory, engine: &dyn Engine) -> Result<(Doc, Message), NotOpened> {
    let doc = match ask {
        Ask::Open(open) => {
            // Over the app's own input limit: refused without mapping anything (the helper ends right after).
            if open.doc.len > MAX_INPUT_BYTES {
                return Err(ErrorCode::TooLarge.into());
            }
            let mapped = memory.read(open.doc)?;
            let doc = engine.open(open.kind, mapped.as_slice());
            // The open document keeps what it needs; the document's section is unmapped, and its grant closed, right
            // away.
            drop(mapped);
            Doc::View(doc?)
        }
        Ask::OpenEdit(open) => {
            if open.doc.len > MAX_INPUT_BYTES {
                return Err(ErrorCode::TooLarge.into());
            }
            let mapped = memory.read(open.doc)?;
            let dropped = crate::edit_guard::dropped_parts(mapped.as_slice());
            if !dropped.is_empty() {
                return Err(NotOpened::Guarded(dropped));
            }
            let doc = engine.open_edit(open.kind, mapped.as_slice());
            drop(mapped);
            Doc::Edit(doc?)
        }
        // No grant: a new document is built from a template the engine holds.
        Ask::NewDoc(new) => Doc::Edit(engine.new_doc(new.kind, new.template)?),
        // `work` calls this for the three opening asks only.
        _ => return Err(ErrorCode::Internal.into()),
    };
    let first = match doc.view().opened() {
        Opening::Pages(pages) => {
            // At most the engine's 2,000 pages, which fit one frame; more than a frame holds is more than allowed.
            if pages.encode().is_err() {
                return Err(ErrorCode::TooLarge.into());
            }
            Message::Opened(pages)
        }
        Opening::Sheets(sheets) => Message::OpenedSheets(fit::sheets(sheets)),
    };
    Ok((doc, first))
}

/// A request cancelled before it started: no work, an `Error { Internal }`. A pixel buffer (a page's, a chart's or a
/// picture's) is still mapped and dropped at once, because that is the only way to close its grant.
fn skip(id: u32, ask: &Ask, answers: &mut Answers<'_>, memory: &dyn Memory) -> Step {
    release_grants(ask, memory);
    answers.error(id, ErrorCode::Internal)
}

/// Closes the grants an ask carries and will not use.
fn release_grants(ask: &Ask, memory: &dyn Memory) {
    match ask {
        Ask::Render(render) => release(memory, render.out),
        Ask::RenderAnchor(anchor) => release(memory, anchor.out),
        Ask::SaveInto(into) => release(memory, into.out),
        Ask::InsertImage(image) => release_read(memory, image.bytes),
        Ask::Open(_)
        | Ask::OpenEdit(_)
        | Ask::NewDoc(_)
        | Ask::Cells(_)
        | Ask::Outline(_)
        | Ask::Stats
        | Ask::Edit(_)
        | Ask::Hit(_)
        | Ask::SaveSize
        | Ask::Copy
        | Ask::Layouts => {}
    }
}

fn answer(id: u32, ask: Ask, session: &mut Session, answers: &mut Answers<'_>, memory: &dyn Memory) -> Step {
    match ask {
        Ask::Render(render) => answer_render(id, render, session.doc.view(), answers, memory),
        Ask::RenderAnchor(anchor) => answer_render_anchor(id, anchor, session.doc.view(), answers, memory),
        Ask::Cells(cells) => answer_cells(id, cells, session.doc.view(), answers),
        Ask::Outline(outline) => {
            if session.outline.is_none() {
                session.outline = Some(fit::outline_entries(session.doc.view().outline()));
            }
            let entries = session.outline.as_deref().unwrap_or_default();
            answers.send(id, &Message::OutlineData(fit::outline_page(entries, outline.from)))
        }
        Ask::Stats => answers.send(
            id,
            &Message::StatsData(StatsData {
                words: session.doc.view().words(),
            }),
        ),
        // `take` queues an opening ask only as the first request, and `work` handles it there.
        Ask::Open(_) | Ask::OpenEdit(_) | Ask::NewDoc(_) => Err(EXIT_PROTOCOL),
        edit => answer_edit(id, edit, session, answers, memory),
    }
}

/// The edit asks. On a viewer session each is refused with `Unsupported` (a grant it carries is closed first).
fn answer_edit(id: u32, ask: Ask, session: &mut Session, answers: &mut Answers<'_>, memory: &dyn Memory) -> Step {
    let Session {
        doc: Doc::Edit(doc),
        pages,
        saves,
        ..
    } = session
    else {
        release_grants(&ask, memory);
        return answers.error(id, ErrorCode::Unsupported);
    };
    match ask {
        Ask::Edit(edit) => {
            let done = doc.apply(edit.op);
            send_edited(id, done, pages, answers)
        }
        Ask::Hit(hit) => {
            let done = doc.hit(hit.page, hit.x_pt, hit.y_pt, hit.mode);
            send_edited(id, done, pages, answers)
        }
        Ask::InsertImage(image) => {
            // The picture is copied out of the host's section and the section unmapped, grant closed, before the engine
            // is asked: mapped once, like every grant.
            if image.bytes.len > MAX_INPUT_BYTES {
                release_read(memory, image.bytes);
                return answers.error(id, ErrorCode::TooLarge);
            }
            let bytes = match memory.read(image.bytes) {
                Ok(mapped) => mapped.as_slice().to_vec(),
                Err(code) => return answers.error(id, code),
            };
            let done = doc.insert_image(bytes, image.png);
            send_edited(id, done, pages, answers)
        }
        Ask::SaveSize => match doc.save() {
            Ok((bytes, revision)) => {
                let len = bytes.len() as u64;
                if len > MAX_SAVE_BYTES {
                    return answers.error(id, ErrorCode::TooLarge);
                }
                let token = saves.hold(bytes);
                answers.send(id, &Message::SaveReady(SaveReady { token, len, revision }))
            }
            Err(code) => answers.error(id, code),
        },
        Ask::SaveInto(into) => answer_save_into(id, into, saves, answers, memory),
        Ask::Copy => {
            let clip = doc.copy();
            let clip = Clip {
                text: clip.text,
                html: clip.html,
            };
            // The html goes first when it does not fit with the text; the text alone not fitting is too much to copy.
            match clip.encode() {
                Ok(payload) => answers.frame(id, KIND_CLIP, &payload),
                Err(office::EncodeError::OverFrame) => answers.error(id, ErrorCode::TooLarge),
                Err(office::EncodeError::TextTooLong) => answers.error(id, ErrorCode::Internal),
            }
        }
        Ask::Layouts => answers.send(id, &Message::LayoutList(fit::layouts(doc.layouts()))),
        // The viewer's asks are answered by `answer`, the opening asks by `work`.
        _ => Err(EXIT_PROTOCOL),
    }
}

/// An `Edited` (or the code it failed with) to the app: held to the caps, checked as the app will check it, and kept
/// from the app when it would be refused (the page sizes the app holds are the helper's own record of them).
fn send_edited(id: u32, done: Result<Edited, ErrorCode>, pages: &mut Vec<PageSize>, answers: &mut Answers<'_>) -> Step {
    let edited = match done {
        Ok(edited) => fit::edited(edited),
        Err(code) => return answers.error(id, code),
    };
    if check_edited(&edited, pages, &Limits::CONTRACT).is_err() {
        return answers.error(id, ErrorCode::Internal);
    }
    if let Some(new) = &edited.pages {
        pages.clone_from(&new.sizes);
    }
    answers.send(id, &Message::Edited(edited))
}

/// `SaveInto`: the held copy, by token, into the section the app granted (exactly its length), then dropped. A token
/// that is not the one held, or a section of another length, is `Error { Internal }`; the section is mapped and
/// dropped either way, which closes its grant.
fn answer_save_into(
    id: u32,
    into: SaveInto,
    saves: &mut Saves,
    answers: &mut Answers<'_>,
    memory: &dyn Memory,
) -> Step {
    let bytes = match saves.take(into.token, into.out.len) {
        Ok(bytes) => bytes,
        Err(Refused::Stale | Refused::WrongLength) => {
            release(memory, into.out);
            return answers.error(id, ErrorCode::Internal);
        }
    };
    let len = bytes.len() as u64;
    let copied = match memory.write(into.out) {
        Ok(mut mapped) => {
            let section = mapped.as_mut_slice();
            let fits = section.len() == bytes.len();
            if fits {
                section.copy_from_slice(&bytes);
            }
            drop(mapped);
            fits
        }
        // A refused map has closed the grant.
        Err(_) => false,
    };
    // The copy is dropped here, written or not.
    drop(bytes);
    if copied {
        answers.send(id, &Message::SavedInto(SavedInto { len }))
    } else {
        answers.error(id, ErrorCode::Internal)
    }
}

/// `Render`: a page, or a region of it, through [`draw`].
fn answer_render(id: u32, render: Render, doc: &dyn OpenDoc, answers: &mut Answers<'_>, memory: &dyn Memory) -> Step {
    let size = doc.render_size(render.page, render.scale, render.region);
    draw(id, render.out, size, answers, memory, |out| {
        doc.render(render.page, render.scale, render.region, out)
    })
}

/// `RenderAnchor`: a chart or a picture of a sheet, through [`draw`], the `Render` arm's twin.
fn answer_render_anchor(
    id: u32,
    ask: RenderAnchor,
    doc: &dyn OpenDoc,
    answers: &mut Answers<'_>,
    memory: &dyn Memory,
) -> Step {
    let size = doc.anchor_size(ask.sheet, ask.anchor, ask.scale);
    draw(id, ask.out, size, answers, memory, |out| {
        doc.render_anchor(ask.sheet, ask.anchor, ask.scale, out)
    })
}

/// Pixels into the buffer `out`: the engine has sized them (`size`); a buffer of any other length is an error, never
/// a guess. The buffer is mapped once (on an error too, which closes its grant), rendered into, and unmapped before
/// the answer is written.
fn draw(
    id: u32,
    out: GrantRef,
    size: Result<(u32, u32), ErrorCode>,
    answers: &mut Answers<'_>,
    memory: &dyn Memory,
    render: impl FnOnce(&mut [u8]) -> Result<bool, ErrorCode>,
) -> Step {
    let (w, h) = match size {
        Ok(size) => size,
        Err(code) => {
            release(memory, out);
            return answers.error(id, code);
        }
    };
    let want = u64::from(w).checked_mul(u64::from(h)).and_then(|px| px.checked_mul(4));
    if want != Some(out.len) {
        // The app and the engine disagree on the size: the seam fails loudly.
        release(memory, out);
        return answers.error(id, ErrorCode::Internal);
    }
    let drawn = match memory.write(out) {
        Ok(mut mapped) => {
            let drawn = render(mapped.as_mut_slice());
            drop(mapped);
            drawn
        }
        // A refused map has closed the grant.
        Err(code) => Err(code),
    };
    match drawn {
        Ok(missing_content) => answers.send(id, &Message::Rendered(Rendered { w, h, missing_content })),
        Err(code) => answers.error(id, code),
    }
}

/// `Cells`: a range whose sizes alone overflow a frame, or whose cells do not fit one, is `RangeTooLarge` (the app
/// asks for each half). Merges are clipped to the range and texts cut to their cap, for any engine.
fn answer_cells(id: u32, cells: Cells, doc: &dyn OpenDoc, answers: &mut Answers<'_>) -> Step {
    if !fit::range_sizes_fit(&cells.range) {
        return answers.error(id, ErrorCode::RangeTooLarge);
    }
    let data: CellData = match doc.cells(cells.sheet, cells.range) {
        Ok(data) => fit::cell_data(data, &cells.range),
        Err(code) => return answers.error(id, code),
    };
    match data.encode() {
        Ok(payload) => answers.frame(id, KIND_CELL_DATA, &payload),
        Err(office::EncodeError::OverFrame) => answers.error(id, ErrorCode::RangeTooLarge),
        // Cell texts are cut to their cap first: not reachable, and a failure of the helper if it were.
        Err(office::EncodeError::TextTooLong) => answers.error(id, ErrorCode::Internal),
    }
}

/// Maps a page buffer this request will not use and drops the mapping at once: the only way to close its grant.
/// A refused map has closed it too, so the result is not needed.
fn release(memory: &dyn Memory, out: GrantRef) {
    drop(memory.write(out));
}

/// The same for a read-only section (a picture the host made).
fn release_read(memory: &dyn Memory, bytes: GrantRef) {
    drop(memory.read(bytes));
}

/// The answers, on the helper's standard output: frames only.
struct Answers<'a> {
    out: &'a mut dyn Write,
}

impl Answers<'_> {
    /// Writes `message` as the answer to `id`. A message that does not encode is never written: the app gets an
    /// `Error { Internal }` for it instead.
    fn send(&mut self, id: u32, message: &Message) -> Step {
        match message.encode() {
            Ok(payload) => self.frame(id, message.kind(), &payload),
            Err(office::EncodeError::OverFrame | office::EncodeError::TextTooLong) => {
                self.error(id, ErrorCode::Internal)
            }
        }
    }

    fn error(&mut self, id: u32, code: ErrorCode) -> Step {
        // Two bytes: always within one frame.
        let payload = office::Error { code }.encode().map_err(|_| EXIT_IO)?;
        self.frame(id, KIND_ERROR, &payload)
    }

    fn frame(&mut self, id: u32, kind: u16, payload: &[u8]) -> Step {
        write_frame(&mut self.out, kind, id, payload).map_err(|_| EXIT_IO)
    }
}
