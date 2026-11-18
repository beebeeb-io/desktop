//! The app's side of an editing session (W2 of the editor epic): the requests of `OfficeHost` (`open_edit`, `new_doc`,
//! `edit`, `hit`, `copy`, `layouts`, `insert_image`, `save_bytes`), sent by the document's worker like any request
//! (same order, same timers), and the checks every answer passes before use.
//!
//! - Every answer is checked by the protocol (`check_edited`, `check_layouts`, `check_save_ready`,
//!   `check_saved_into`); a failure ends the helper ("Renderer stopped"). One helper per document, never shared; an
//!   editing session has no Try again (a new helper would not hold the edits).
//! - **Save handshake.** `SaveSize` -> `SaveReady{len}`: over `MAX_SAVE_BYTES` (or empty) is refused; else a section of
//!   exactly `len`, granted read-write, `SaveInto`, `SavedInto{len}` equal to the granted length; the bytes are read
//!   out with the raw copy into `Zeroizing<Vec<u8>>` and the section dropped. While a save is out, edits wait here, so
//!   the helper's token stays current.
//! - The document's bytes stay in memory (`held.rs` comes in W3). Answers reach the window through
//!   [`OfficeHost::poll_edit`], never through [`super::super::Event`], which the viewer matches exhaustively.

use std::fmt;

use beebeeb_office_protocol::check;
use beebeeb_office_protocol::office::{self, EncodeError, ErrorCode, Message};
use beebeeb_office_sandbox as sandbox;
use zeroize::{Zeroize, Zeroizing};

use std::path::Path;

use super::super::{
    Confined, DocKind, EditKind, EditOp, Edited, HitMode, OfficeHost, OfficeState, Purpose, Registry, RequestId,
    Template, Timeouts, Wake, lock, open_mode, open_mode_held,
};
use super::restart::Cadence;
use super::{Asked, Command, Doc, Out, Phase, Starter, Stop, Worker};
use beebeeb_engine::app::office_scratch::ScratchStore;

/// How a document starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    /// The viewer: `Open`.
    View,
    /// An editor of the bytes the host holds: `OpenEdit`.
    Edit(EditKind),
    /// A new document from a template: `NewDoc`, no bytes.
    New(EditKind, Template),
}

fn doc_kind(kind: EditKind) -> DocKind {
    match kind {
        EditKind::Docx => DocKind::Docx,
        EditKind::Xlsx => DocKind::Xlsx,
        EditKind::Pptx => DocKind::Pptx,
    }
}

/// Why an edit call was not sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditRefused {
    /// Not an editing session, or the document is not open (yet, any more).
    NotEditing,
    /// A paste over `MAX_OP_TEXT_BYTES` (0.9.0): say so in words ([`EditRefused::words`]).
    PasteTooLarge,
    /// Text of an op over `MAX_OP_TEXT_BYTES`.
    TooMuchText,
    /// An image that is empty or over the input limit.
    BadImage,
    /// Something that cannot be sent (a page that is not there, a point that is not a number).
    Invalid,
}

impl EditRefused {
    /// The words for the window.
    pub fn words(self) -> &'static str {
        match self {
            EditRefused::NotEditing => "This document is not open for editing.",
            EditRefused::PasteTooLarge => "That is too much to paste at once.",
            EditRefused::TooMuchText => "That is too much text to add at once.",
            EditRefused::BadImage => "That picture can't be added.",
            EditRefused::Invalid => "That can't be done here.",
        }
    }
}

/// Why the helper could not do a request that left the document as it was.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditFailure {
    /// "Too much to copy at once."
    TooLarge,
    /// The helper could not do it.
    Failed,
}

/// What [`OfficeHost::poll_edit`] hands the window. Every payload has passed its check; `id` is the request's.
pub enum EditEvent {
    /// An edit, a click or an image: what changed.
    Edited { id: RequestId, edited: Box<Edited> },
    /// The selection as text and, when it fitted, html.
    Clip {
        id: RequestId,
        text: String,
        html: Option<String>,
    },
    /// The names of the slide layouts.
    Layouts { id: RequestId, names: Vec<String> },
    /// The serialised document, and the revision it is.
    Saved {
        id: RequestId,
        purpose: Purpose,
        revision: u64,
        bytes: Zeroizing<Vec<u8>>,
    },
    /// A request the helper refused and the session goes on.
    Failed { id: RequestId, why: EditFailure },
    /// The helper died and a new one opened the last snapshot (`from_snapshot`; else the bytes the document was opened
    /// with), which is from `at`. Edits since then and the undo history are gone ([`super::restart::RESTORED_WORDS`]).
    /// Requests that were out got [`EditEvent::Failed`].
    Restored { at: std::time::SystemTime, from_snapshot: bool },
    /// The helper died twice within a minute: the document is open read-only now (`edit` calls are refused) and `copy`
    /// is the last snapshot to keep as a copy, when there is one ([`super::restart::READ_ONLY_WORDS`]).
    ReadOnly {
        at: std::time::SystemTime,
        copy: Option<Zeroizing<Vec<u8>>>,
    },
}

/// Document text and bytes: lengths only.
impl fmt::Debug for EditEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EditEvent::Edited { id, edited } => f.debug_struct("Edited").field("id", id).field("edited", edited).finish(),
            EditEvent::Clip { id, text, html } => f
                .debug_struct("Clip")
                .field("id", id)
                .field("text_len", &text.len())
                .field("html_len", &html.as_ref().map(String::len))
                .finish(),
            EditEvent::Layouts { id, names } => f.debug_struct("Layouts").field("id", id).field("names", &names.len()).finish(),
            EditEvent::Saved {
                id,
                purpose,
                revision,
                bytes,
            } => f
                .debug_struct("Saved")
                .field("id", id)
                .field("purpose", purpose)
                .field("revision", revision)
                .field("len", &bytes.len())
                .finish(),
            EditEvent::Failed { id, why } => f.debug_struct("Failed").field("id", id).field("why", why).finish(),
            EditEvent::Restored { at, from_snapshot } => f
                .debug_struct("Restored")
                .field("at", at)
                .field("from_snapshot", from_snapshot)
                .finish(),
            EditEvent::ReadOnly { at, copy } => f
                .debug_struct("ReadOnly")
                .field("at", at)
                .field("copy_len", &copy.as_ref().map(|copy| copy.len()))
                .finish(),
        }
    }
}

/// Wipes what an event holds (counted before the wipe).
pub(crate) fn wipe_edit_event(event: &mut EditEvent) -> usize {
    match event {
        EditEvent::Clip { text, html, .. } => {
            let mut n = text.len();
            text.zeroize();
            if let Some(html) = html.as_mut() {
                n += html.len();
                html.zeroize();
            }
            n
        }
        EditEvent::Layouts { names, .. } => names
            .iter_mut()
            .map(|name| {
                let n = name.len();
                name.zeroize();
                n
            })
            .sum(),
        EditEvent::Saved { bytes, .. } => {
            let n = bytes.len();
            bytes.zeroize();
            n
        }
        EditEvent::ReadOnly { copy, .. } => copy.as_mut().map_or(0, |copy| {
            let n = copy.len();
            copy.zeroize();
            n
        }),
        EditEvent::Edited { .. } | EditEvent::Failed { .. } | EditEvent::Restored { .. } => 0,
    }
}

/// What the window asked of an editing session.
pub(crate) enum EditCommand {
    /// `Edit`, `Hit`, `Copy` or `Layouts`.
    Send { id: RequestId, message: Message },
    Image {
        id: RequestId,
        bytes: Zeroizing<Vec<u8>>,
        png: bool,
    },
    Save { id: RequestId, purpose: Purpose },
}

/// An editing request out: what its answer must be.
pub(crate) enum EditAsked<T> {
    Op { id: RequestId },
    Copy { id: RequestId },
    Layouts { id: RequestId },
    /// The image's section stays until the helper answered.
    Image { id: RequestId, section: T },
    SaveSize { id: RequestId, purpose: Purpose },
    SaveInto {
        id: RequestId,
        purpose: Purpose,
        revision: u64,
        len: u64,
        section: T,
    },
}

// ---------------------------------------------------------------------------------------------------------------
// The window's side.
// ---------------------------------------------------------------------------------------------------------------

impl OfficeHost {
    /// Opens `bytes` (a document of `kind`) for editing with the helper at `helper`. Never blocks. The state goes
    /// `Opening` -> `Pages` or `Sheets` as for a viewer; the `edit` calls work once it is open.
    pub fn open_edit(helper: &Path, kind: EditKind, bytes: Zeroizing<Vec<u8>>, wake: Wake) -> OfficeHost {
        Self::open_edit_in(&super::super::PROCESS, helper, kind, bytes, wake)
    }

    /// [`OfficeHost::open_edit`] joining `registry`.
    pub(crate) fn open_edit_in(
        registry: &Registry,
        helper: &Path,
        kind: EditKind,
        bytes: Zeroizing<Vec<u8>>,
        wake: Wake,
    ) -> OfficeHost {
        Self::open_edit_with(registry, helper, kind, bytes, wake, None)
    }

    pub(crate) fn open_edit_with(
        registry: &Registry,
        helper: &Path,
        kind: EditKind,
        bytes: Zeroizing<Vec<u8>>,
        wake: Wake,
        scratch: Option<ScratchStore>,
    ) -> OfficeHost {
        let starter = Confined {
            helper: helper.to_path_buf(),
        };
        open_mode_held(
            starter,
            registry,
            Timeouts::PRODUCTION,
            Cadence::PRODUCTION,
            Mode::Edit(kind),
            doc_kind(kind),
            Some(bytes),
            wake,
            scratch,
        )
    }

    /// A new document of `kind` from `template`, edited with the helper at `helper`. Never blocks.
    pub fn new_doc(helper: &Path, kind: EditKind, template: Template, wake: Wake) -> OfficeHost {
        Self::new_doc_in(&super::super::PROCESS, helper, kind, template, wake)
    }

    /// [`OfficeHost::open_edit`] with the session's scratch store (the app's: `ViewerEngine::scratch`): the opened
    /// bytes and the snapshots above `SPILL_ABOVE_BYTES` are kept sealed on disk, not in memory.
    pub fn open_edit_scratch(
        helper: &Path,
        kind: EditKind,
        bytes: Zeroizing<Vec<u8>>,
        wake: Wake,
        scratch: Option<ScratchStore>,
    ) -> OfficeHost {
        Self::open_edit_with(&super::super::PROCESS, helper, kind, bytes, wake, scratch)
    }

    /// [`OfficeHost::new_doc`] with the session's scratch store.
    pub fn new_doc_scratch(
        helper: &Path,
        kind: EditKind,
        template: Template,
        wake: Wake,
        scratch: Option<ScratchStore>,
    ) -> OfficeHost {
        Self::new_doc_with(&super::super::PROCESS, helper, kind, template, wake, scratch)
    }

    /// [`OfficeHost::new_doc`] joining `registry`.
    pub(crate) fn new_doc_in(
        registry: &Registry,
        helper: &Path,
        kind: EditKind,
        template: Template,
        wake: Wake,
    ) -> OfficeHost {
        Self::new_doc_with(registry, helper, kind, template, wake, None)
    }

    pub(crate) fn new_doc_with(
        registry: &Registry,
        helper: &Path,
        kind: EditKind,
        template: Template,
        wake: Wake,
        scratch: Option<ScratchStore>,
    ) -> OfficeHost {
        let starter = Confined {
            helper: helper.to_path_buf(),
        };
        open_mode_held(
            starter,
            registry,
            Timeouts::PRODUCTION,
            Cadence::PRODUCTION,
            Mode::New(kind, template),
            doc_kind(kind),
            None,
            wake,
            scratch,
        )
    }

    /// One edit. The text of an op (a paste too) is at most `MAX_OP_TEXT_BYTES`; a paste over it is
    /// [`EditRefused::PasteTooLarge`]. The answer is an [`EditEvent::Edited`] with the returned id.
    pub fn edit(&mut self, op: EditOp) -> Result<RequestId, EditRefused> {
        self.ready()?;
        let paste = matches!(op, EditOp::Paste { .. });
        let edit = office::Edit { op };
        match edit.encode() {
            Ok(_) => {}
            Err(EncodeError::TextTooLong) if paste => return Err(EditRefused::PasteTooLarge),
            Err(EncodeError::TextTooLong) => return Err(EditRefused::TooMuchText),
            Err(_) => return Err(EditRefused::Invalid),
        }
        Ok(self.send_edit(Message::Edit(edit)))
    }

    /// What a click on `page` at (`x_pt`, `y_pt`) does to the selection. Answered by [`EditEvent::Edited`].
    pub fn hit(&mut self, page: u32, x_pt: f32, y_pt: f32, mode: HitMode) -> Result<RequestId, EditRefused> {
        self.ready()?;
        let on_page = self.read(|state| match state {
            OfficeState::Pages { pages, .. } => page < *pages,
            _ => page == 0,
        });
        if !on_page || !x_pt.is_finite() || !y_pt.is_finite() {
            return Err(EditRefused::Invalid);
        }
        Ok(self.send_edit(Message::Hit(office::Hit { page, x_pt, y_pt, mode })))
    }

    /// The selection as text and html. Answered by [`EditEvent::Clip`], or [`EditEvent::Failed`] with
    /// [`EditFailure::TooLarge`] ("Too much to copy at once").
    pub fn copy(&mut self) -> Result<RequestId, EditRefused> {
        self.ready()?;
        Ok(self.send_edit(Message::Copy(office::Copy)))
    }

    /// The names of the slide layouts. Answered by [`EditEvent::Layouts`].
    pub fn layouts(&mut self) -> Result<RequestId, EditRefused> {
        self.ready()?;
        Ok(self.send_edit(Message::Layouts(office::Layouts)))
    }

    /// Inserts an image (`png`, else jpeg) at the caret: the host makes the read-only section the helper reads.
    /// Answered by [`EditEvent::Edited`].
    pub fn insert_image(&mut self, bytes: Zeroizing<Vec<u8>>, png: bool) -> Result<RequestId, EditRefused> {
        self.ready()?;
        if bytes.is_empty() || bytes.len() as u64 > check::MAX_INPUT_BYTES {
            return Err(EditRefused::BadImage);
        }
        let id = self.next_id();
        self.command(Command::Edit(EditCommand::Image { id, bytes, png }));
        Ok(id)
    }

    /// The document's bytes and the revision they are, for a save or a snapshot. Answered by [`EditEvent::Saved`].
    /// One save at a time is out; edits asked meanwhile are sent after it.
    pub fn save_bytes(&mut self, purpose: Purpose) -> Result<RequestId, EditRefused> {
        self.ready()?;
        let id = self.next_id();
        self.command(Command::Edit(EditCommand::Save { id, purpose }));
        Ok(id)
    }

    /// The editor's answers since the last call (call [`OfficeHost::poll`] first: it moves them here). Never blocks.
    pub fn poll_edit(&mut self) -> Vec<EditEvent> {
        std::mem::take(&mut lock(&self.shared.inner).edit_events).into()
    }

    /// After a second crash within a minute the document is open read-only ([`EditEvent::ReadOnly`]).
    pub fn read_only(&self) -> bool {
        lock(&self.shared.inner).read_only
    }

    /// An editing session whose document is open.
    fn ready(&self) -> Result<(), EditRefused> {
        if self.editing && !lock(&self.shared.inner).read_only && self.is_open() {
            Ok(())
        } else {
            Err(EditRefused::NotEditing)
        }
    }

    fn send_edit(&mut self, message: Message) -> RequestId {
        let id = self.next_id();
        self.command(Command::Edit(EditCommand::Send { id, message }));
        id
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The worker's side.
// ---------------------------------------------------------------------------------------------------------------

impl<S: Starter> Worker<S> {
    /// The message that starts the document: `Open`, `OpenEdit` (the document's section granted read-only) or
    /// `NewDoc`. `None` when the section is gone or cannot be granted.
    pub(super) fn first_message(&self) -> Option<Message> {
        let run = self.run.as_ref()?;
        if let Mode::New(kind, template) = self.mode {
            return Some(Message::NewDoc(office::NewDoc { kind, template }));
        }
        let Doc::Section(section) = &self.doc else {
            return None;
        };
        let granted = self.starter.grant(&run.helper, section, sandbox::Access::Read).ok()?;
        let doc = office::GrantRef {
            wire: granted.wire,
            len: granted.len,
        };
        Some(match self.mode {
            Mode::Edit(kind) => Message::OpenEdit(office::OpenEdit { doc, kind }),
            _ => Message::Open(office::Open { kind: self.kind, doc }),
        })
    }

    pub(super) fn editable(&self) -> bool {
        !matches!(self.mode, Mode::View) && matches!(self.phase, Phase::Pages { .. } | Phase::Sheets) && self.run.is_some()
    }

    /// A request of the window. While a save is out it waits (in order); out of an open editing session it is dropped.
    pub(super) fn edit_command(&mut self, command: EditCommand) {
        if !self.editable() {
            return;
        }
        if matches!(&command, EditCommand::Image { .. })
            || matches!(&command, EditCommand::Send { message: Message::Edit(_), .. })
        {
            self.note_edit();
        }
        let Some(run) = self.run.as_mut() else {
            return;
        };
        if run.save_busy {
            run.deferred.push_back(Command::Edit(command));
            return;
        }
        match command {
            EditCommand::Send { id, message } => {
                let asked = match &message {
                    Message::Copy(_) => EditAsked::Copy { id },
                    Message::Layouts(_) => EditAsked::Layouts { id },
                    _ => EditAsked::Op { id },
                };
                self.ask(message, Asked::Edit(asked));
            }
            EditCommand::Image { id, bytes, png } => self.send_image(id, bytes, png),
            EditCommand::Save { id, purpose } => {
                let message = Message::SaveSize(office::SaveSize { purpose });
                if self.ask(message, Asked::Edit(EditAsked::SaveSize { id, purpose })) {
                    self.note_save_asked();
                    if let Some(run) = self.run.as_mut() {
                        run.save_busy = true;
                    }
                }
            }
        }
    }

    /// The image in a section of exactly its size, granted read-only; the section stays until the answer.
    fn send_image(&mut self, id: RequestId, bytes: Zeroizing<Vec<u8>>, png: bool) {
        let Ok(mut section) = self.starter.section(bytes.len()) else {
            return self.stopped(Stop::Failed);
        };
        self.starter.fill(&mut section, &bytes);
        drop(bytes);
        let granted = self
            .run
            .as_ref()
            .map(|run| self.starter.grant(&run.helper, &section, sandbox::Access::Read));
        let Some(Ok(granted)) = granted else {
            return self.stopped(Stop::Failed);
        };
        let request = office::InsertImage {
            bytes: office::GrantRef {
                wire: granted.wire,
                len: granted.len,
            },
            png,
        };
        self.ask(
            Message::InsertImage(request),
            Asked::Edit(EditAsked::Image { id, section }),
        );
    }

    /// The save ended (answered, refused or failed): the edits that waited go out, in order.
    fn save_done(&mut self) {
        let queued = match self.run.as_mut() {
            Some(run) => {
                run.save_busy = false;
                std::mem::take(&mut run.deferred)
            }
            None => return,
        };
        for command in queued {
            if let Command::Edit(command) = command {
                self.edit_command(command);
            }
        }
    }

    fn push_edit(&self, event: EditEvent) {
        self.shared.push(Out::Edit(event));
    }

    /// The answer to the oldest request, an editing one: it must be the kind that request takes, then pass its check.
    pub(super) fn edit_answer(&mut self, asked: EditAsked<S::Section>, message: Message) {
        match (asked, message) {
            (EditAsked::Op { id } | EditAsked::Image { id, .. }, Message::Edited(got)) => self.edited(id, got),
            (EditAsked::Copy { id }, Message::Clip(clip)) => {
                if self.handles_ok() {
                    self.push_edit(EditEvent::Clip {
                        id,
                        text: clip.text,
                        html: clip.html,
                    });
                } else {
                    let mut clip = clip;
                    clip.text.zeroize();
                    if let Some(html) = clip.html.as_mut() {
                        html.zeroize();
                    }
                }
            }
            (EditAsked::Layouts { id }, Message::LayoutList(mut list)) => {
                if check::check_layouts(&list).is_err() {
                    for name in &mut list.names {
                        name.zeroize();
                    }
                    return self.stopped(Stop::Protocol);
                }
                if self.handles_ok() {
                    self.push_edit(EditEvent::Layouts { id, names: list.names });
                }
            }
            (EditAsked::SaveSize { id, purpose }, Message::SaveReady(got)) => self.save_ready(id, purpose, got),
            (
                EditAsked::SaveInto {
                    id,
                    purpose,
                    revision,
                    len,
                    section,
                },
                Message::SavedInto(got),
            ) => self.saved_into(id, purpose, revision, len, section, got),
            // A refused save size: nothing was made, the session goes on.
            (EditAsked::SaveSize { id, .. }, Message::Error(error)) => self.refused(id, error.code, true),
            // The helper broke its own handshake (a stale token is `Internal`): the helper is ended.
            (EditAsked::SaveInto { .. }, Message::Error(_)) => self.stopped(Stop::Failed),
            (
                EditAsked::Op { id } | EditAsked::Copy { id } | EditAsked::Layouts { id } | EditAsked::Image { id, .. },
                Message::Error(error),
            ) => self.refused(id, error.code, false),
            (_, mut other) => {
                super::wipe_message(&mut other);
                self.stopped(Stop::Protocol);
            }
        }
    }

    /// An error answer to a request of the session: the document is as it was, the session goes on. `RangeTooLarge`
    /// is only ever the answer to `Cells`.
    fn refused(&mut self, id: RequestId, code: ErrorCode, was_save: bool) {
        if was_save {
            self.save_done();
        }
        if code == ErrorCode::RangeTooLarge {
            return self.stopped(Stop::Protocol);
        }
        // A snapshot the worker asked itself: nobody is waiting for it; not again at once.
        if self.is_own_id(id) {
            self.snapshot_backoff();
            if self.rs.flush_id == Some(id) {
                self.flush_complete();
            }
            return;
        }
        if was_save {
            self.snapshot_backoff();
        }
        if !self.handles_ok() {
            return;
        }
        let why = if code == ErrorCode::TooLarge {
            EditFailure::TooLarge
        } else {
            EditFailure::Failed
        };
        self.push_edit(EditEvent::Failed { id, why });
    }

    fn edited(&mut self, id: RequestId, got: Edited) {
        let pages = match &self.phase {
            Phase::Pages { sizes } => sizes.as_slice(),
            Phase::Sheets => &[],
            _ => return self.stopped(Stop::Protocol),
        };
        if check::check_edited(&got, pages, &check::Limits::CONTRACT).is_err() {
            return self.stopped(Stop::Protocol);
        }
        // New page sizes belong to a paged document only.
        if got.pages.is_some() && !matches!(self.phase, Phase::Pages { .. }) {
            return self.stopped(Stop::Protocol);
        }
        if !self.handles_ok() {
            return;
        }
        if let Some(new) = &got.pages {
            self.phase = Phase::Pages {
                sizes: new.sizes.clone(),
            };
            self.shared.push(Out::State(OfficeState::Pages {
                pages: new.pages,
                sizes: new.sizes.clone(),
            }));
        }
        self.push_edit(EditEvent::Edited {
            id,
            edited: Box::new(got),
        });
    }

    /// `SaveReady`: at most `MAX_SAVE_BYTES` and not empty, else the helper is ended; then a section of exactly `len`,
    /// granted read-write, and `SaveInto`.
    fn save_ready(&mut self, id: RequestId, purpose: Purpose, got: office::SaveReady) {
        if check::check_save_ready(&got).is_err() || got.len == 0 {
            return self.stopped(Stop::Protocol);
        }
        if !self.handles_ok() {
            return;
        }
        let Ok(len) = usize::try_from(got.len) else {
            return self.stopped(Stop::Protocol);
        };
        let Ok(section) = self.starter.section(len) else {
            return self.stopped(Stop::Failed);
        };
        let granted = self
            .run
            .as_ref()
            .map(|run| self.starter.grant(&run.helper, &section, sandbox::Access::ReadWrite));
        let Some(Ok(granted)) = granted else {
            return self.stopped(Stop::Failed);
        };
        let request = office::SaveInto {
            token: got.token,
            out: office::GrantRef {
                wire: granted.wire,
                len: granted.len,
            },
        };
        self.ask(
            Message::SaveInto(request),
            Asked::Edit(EditAsked::SaveInto {
                id,
                purpose,
                revision: got.revision,
                len: got.len,
                section,
            }),
        );
    }

    /// `SavedInto`: its length is the granted section's; the bytes are copied out (raw copy) into a wiped-on-drop
    /// buffer and the section is dropped.
    fn saved_into(
        &mut self,
        id: RequestId,
        purpose: Purpose,
        revision: u64,
        len: u64,
        section: S::Section,
        got: office::SavedInto,
    ) {
        if check::check_saved_into(&got, len).is_err() {
            return self.stopped(Stop::Protocol);
        }
        if !self.handles_ok() {
            return;
        }
        let mut bytes = Zeroizing::new(vec![0u8; self.starter.section_len(&section)]);
        if !self.starter.copy_out(&section, &mut bytes) {
            // Never a document of zeros handed out as the document.
            return self.stopped(Stop::Failed);
        }
        drop(section);
        self.keep(&bytes, revision);
        if purpose == Purpose::Save && self.rs.save_started == self.rs.edit_seq {
            self.rs.unsaved = false;
        }
        if self.rs.flush_id == Some(id) {
            // The snapshot a lock asked for: sealed here, on this thread, before the window could poll it.
            if let Some(sink) = self.shared.flush_sink() {
                sink(&bytes);
            }
            self.flush_complete();
        }
        self.push_edit(EditEvent::Saved {
            id,
            purpose,
            revision,
            bytes,
        });
        self.save_done();
    }
}
