//! The source's Office arm: the route in `begin` (a writable .docx / .xlsx / .pptx goes to an editing session, the
//! rest to the viewer), the read on a worker, the renderer started with the bytes, the session rule. Child of
//! `viewer_office.rs`.

use super::*;
use crate::engine_host::office_host::{EditKind, Template};
use beebeeb_engine::app::office_scratch::ScratchStore;

/// The read's one answer: the decrypted bytes and the session they were read for.
type Read = Result<(SessionGeneration, Zeroizing<Vec<u8>>), OpenFileError>;

/// Starts a renderer for the bytes (the app's: [`OfficeView::open`]; a test's fake).
pub(crate) type Opener = Box<dyn Fn(&Path, OfficeKind, Zeroizing<Vec<u8>>, Wake) -> OfficeView + Send>;

/// Starts an editing session on the bytes (the app's: [`OfficeView::open_edit`]; a test's fake).
pub(crate) type EditOpener = Box<dyn Fn(&Path, EditKind, Zeroizing<Vec<u8>>, Wake, Option<ScratchStore>) -> OfficeView + Send>;

/// Starts a new document from a template (the app's: [`OfficeView::new_doc`]; a test's fake).
pub(crate) type NewOpener = Box<dyn Fn(&Path, EditKind, Template, Wake, Option<ScratchStore>) -> OfficeView + Send>;

/// Which types open in the editor when the file may be changed. The rest open in the viewer, as before.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct EditShips {
    pub(crate) docx: bool,
    pub(crate) xlsx: bool,
    /// Slides edit only in a helper built with desktop-office's `edit-slides` (MPL-2.0, accepted 2026-11-18, and the
    /// helper's build turns it on): a helper without it answers `Unsupported`, which would show "No preview".
    pub(crate) pptx: bool,
}

impl EditShips {
    pub(crate) const SHIPPED: EditShips = EditShips {
        docx: true,
        xlsx: true,
        pptx: true,
    };

    pub(super) fn allows(self, kind: EditKind) -> bool {
        match kind {
            EditKind::Docx => self.docx,
            EditKind::Xlsx => self.xlsx,
            EditKind::Pptx => self.pptx,
        }
    }
}

/// The source's Office part: the switches, how a renderer starts, and the Office file open now.
pub(crate) struct Office {
    /// `OFFICE_VIEWER_ON` and `OFFICE_SHIPS` (a test turns them on for its own source only).
    pub(crate) on: bool,
    pub(crate) ships: OfficeShips,
    /// A file the account may change opens in the editor, where the sandbox exists (`office_sandbox_here`; a test
    /// turns it on for its own source only, like `on`).
    pub(crate) edit_on: bool,
    pub(crate) edits: EditShips,
    /// Where the helper is (`office_host::helper_exe`).
    pub(crate) helper: fn() -> Option<PathBuf>,
    pub(crate) opener: Opener,
    pub(crate) edit_opener: EditOpener,
    pub(crate) new_opener: NewOpener,
    pub(super) shown: Option<Shown>,
    /// What the editor's saves and New document have on their way (`edit.rs`).
    pub(super) work: edit::Work,
}

impl Default for Office {
    fn default() -> Self {
        Office {
            on: OFFICE_VIEWER_ON,
            ships: OFFICE_SHIPS,
            edit_on: super::super::office_sandbox_here(),
            edits: EditShips::SHIPPED,
            helper: office_host::helper_exe,
            opener: Box::new(OfficeView::open),
            edit_opener: Box::new(OfficeView::open_edit),
            new_opener: Box::new(OfficeView::new_doc),
            shown: None,
            work: edit::Work::default(),
        }
    }
}

/// The Office file open now, and the session it was opened for.
pub(super) struct Shown {
    pub(super) file_id: String,
    kind: OfficeKind,
    pub(super) kept: bool,
    pub(super) session: SessionGeneration,
    pub(super) account: Option<String>,
    pub(super) helper: PathBuf,
    cancel: Arc<AtomicBool>,
    /// What it opens as: an editing session (the account may change the file), or the viewer (`None`).
    pub(super) edit: Option<EditKind>,
    /// The version the editor's file was opened from (the recovery blob's base).
    pub(super) base: i64,
    /// The read's answer, while it is on its way.
    inbox: Option<mpsc::Receiver<Read>>,
    pub(super) view: Option<OfficeView>,
    pub(super) ended: bool,
}

impl Shown {
    /// Nothing of this opening stays: the read is cancelled (bytes on their way are wiped when dropped), the renderer
    /// ends, every picture and the body's text go.
    fn end(&mut self, why: CloseWhy) {
        self.cancel.store(true, Ordering::SeqCst);
        self.inbox = None;
        if let Some(mut view) = self.view.take() {
            view.close(why);
        }
        self.ended = true;
    }
}

impl<E: ViewerEngine> EngineViewerSource<E> {
    /// Where an Office file goes, under this source's switches. `None`: not an Office file, or the viewer is off.
    pub(in crate::engine_host::viewer) fn office_route(&self, name: &str) -> Option<OfficeRoute> {
        kinds::office_route_with(name, self.office.on, self.office.ships)
    }

    /// `begin`'s Office arm: the body shown now. The read starts on a worker only for a type that ships, within the
    /// limit, with the helper installed; it shares the opening's cancel.
    pub(in crate::engine_host::viewer) fn office_begin(
        &mut self,
        route: OfficeRoute,
        opening: &Opening,
        facts: &OpenFileFacts,
        recorded: Recorded,
    ) -> Body {
        self.office_left();
        let kept = facts.on_this_pc.is_some() && self.engine.is_syncing();
        let state = |state| Body::OfficeState(StateBody::new(state, kept));
        let kind = match route {
            OfficeRoute::Shows(kind) => kind,
            OfficeRoute::NoPreview(_) => return state(ViewState::NoPreview(Reason::UnsupportedType)),
        };
        // The contract's limit is the app's, before decrypting: nothing is read.
        if facts.size > check::MAX_INPUT_BYTES {
            return state(ViewState::NoPreview(Reason::TooLarge));
        }
        let Some(helper) = (self.office.helper)() else {
            return state(ViewState::Stopped { retried: true });
        };
        // Writable .docx / .xlsx / .pptx go to an editing session; everything else (read-only, no write right, an older
        // format) to the viewer, as before. The version must have been read, as the text editor's.
        let edit = match kind {
            OfficeKind::Docx => Some(EditKind::Docx),
            OfficeKind::Xlsx => Some(EditKind::Xlsx),
            OfficeKind::Pptx => Some(EditKind::Pptx),
            OfficeKind::Doc | OfficeKind::Xls | OfficeKind::Ppt => None,
        }
        .filter(|edit| self.office.edit_on && self.office.edits.allows(*edit))
        .filter(|_| opening.edit.is_some_and(|facts| facts.can_write && facts.version > 0));
        let (answers, inbox) = mpsc::channel();
        let (engine, file_id, cancel) = (self.engine.clone(), opening.file_id.clone(), opening.cancel.clone());
        let worker_cancel = cancel.clone();
        let started = std::thread::Builder::new()
            .name("viewer-office-open".into())
            .spawn(move || {
                let cancel = worker_cancel;
                let answer: Read = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    read(
                        &*engine,
                        &file_id,
                        recorded,
                        check::MAX_INPUT_BYTES,
                        &|_: u64, _: u64| {},
                        &cancel,
                    )
                    .map(|(session, bytes, _)| (session, bytes))
                }))
                .unwrap_or(Err(OpenFileError::Failed));
                // Nobody waits any more: the bytes are dropped here (wiped), never queued.
                let answer = if cancel.load(Ordering::SeqCst) {
                    drop(answer);
                    Err(OpenFileError::Cancelled)
                } else {
                    answer
                };
                let _ = answers.send(answer);
                engine.wake();
            })
            .is_ok();
        if !started {
            log("viewer: could not start a thread for an open file");
            return Body::Failed;
        }
        self.office.shown = Some(Shown {
            file_id: opening.file_id.clone(),
            kind,
            kept,
            session: opening.session,
            account: opening.account.clone(),
            helper,
            cancel,
            edit,
            base: opening.edit.map_or(0, |facts| facts.version),
            inbox: Some(inbox),
            view: None,
            ended: false,
        });
        state(ViewState::Opening)
    }

    /// The poll's Office arm. `None`: no Office file of this id is open here (the rest of the poll runs).
    pub(in crate::engine_host::viewer) fn office_poll(&mut self, file_id: &str) -> Option<Option<Body>> {
        let shown = self.office.shown.as_mut().filter(|shown| shown.file_id == file_id)?;
        if shown.ended {
            return Some(None);
        }
        let engine = &self.engine;
        let same_account = engine.account() == shown.account;
        if !(engine.has_session() && engine.session_turn() == shown.session && same_account) {
            // A lock, a sign-out, another account: everything of it goes NOW, on the window's side.
            shown.end(if same_account && engine.has_session() {
                CloseWhy::Locked
            } else {
                CloseWhy::SignedOut
            });
            if let Some(opening) = self.open.as_mut().filter(|opening| opening.file_id == file_id) {
                opening.end();
            }
            // A save that waited for the editor's bytes can never get them: it ends as not done.
            self.office_fail_save();
            return Some(Some(Body::Stopped(Stopped::SessionChanged)));
        }
        if let Some(inbox) = &shown.inbox {
            match inbox.try_recv() {
                Ok(Ok((session, bytes))) if session == shown.session => {
                    shown.inbox = None;
                    let waker = self.engine.clone();
                    let wake: Wake = Arc::new(move || waker.wake());
                    let mut view = match shown.edit {
                        Some(edit) => {
                            let scratch = engine.office_scratch(shown.session);
                            (self.office.edit_opener)(&shown.helper, edit, bytes, wake, scratch)
                        }
                        None => (self.office.opener)(&shown.helper, shown.kind, bytes, wake),
                    };
                    if shown.edit.is_some() {
                        let recovery = recovery::Recovery::new(
                            Arc::new(recovery::EngineBlobs(self.engine.clone())),
                            shown.session,
                        );
                        view.set_recovery(recovery, &shown.file_id, shown.base);
                    }
                    view.set_kept(shown.kept);
                    shown.view = Some(view);
                }
                // Read for another session than the one this file was opened for: never shown (dropped, wiped).
                Ok(Ok(_)) => {
                    shown.end(CloseWhy::SignedOut);
                    return Some(Some(Body::Stopped(Stopped::SessionChanged)));
                }
                Ok(Err(error)) => {
                    shown.inbox = None;
                    return Some(match error {
                        OpenFileError::TooLarge { .. } | OpenFileError::OverLimit => Some(Body::OfficeState(
                            StateBody::new(ViewState::NoPreview(Reason::TooLarge), shown.kept),
                        )),
                        error => body_of(error),
                    });
                }
                Err(mpsc::TryRecvError::Empty) => return Some(None),
                Err(mpsc::TryRecvError::Disconnected) => {
                    shown.inbox = None;
                    return Some(Some(Body::Failed));
                }
            }
        }
        let body = shown.view.as_mut().and_then(OfficeView::poll);
        // The editor's save, once its bytes are here.
        self.office_pump_save(file_id);
        Some(body)
    }

    /// The Office file of `file_id`, while its session is the session.
    pub(super) fn office_view(&self, file_id: &str) -> Option<&OfficeView> {
        let shown = self.office.shown.as_ref()?;
        let current = self.engine.has_session()
            && self.engine.session_turn() == shown.session
            && self.engine.account() == shown.account;
        (shown.file_id == file_id && !shown.ended && current)
            .then_some(shown.view.as_ref())
            .flatten()
    }

    /// As [`Self::office_view`], to change it.
    pub(super) fn office_view_mut(&mut self, file_id: &str) -> Option<&mut OfficeView> {
        let current = self.engine.has_session()
            && self.office.shown.as_ref().is_some_and(|shown| {
                self.engine.session_turn() == shown.session && self.engine.account() == shown.account
            });
        let shown = self.office.shown.as_mut()?;
        (shown.file_id == file_id && !shown.ended && current)
            .then_some(shown.view.as_mut())
            .flatten()
    }

    pub(in crate::engine_host::viewer) fn office_picture_of(
        &self,
        file_id: &str,
        which: OfficePicture,
    ) -> Option<rune::Pixels> {
        self.office_view(file_id)?.picture(which)
    }

    pub(in crate::engine_host::viewer) fn office_cells_of(
        &self,
        file_id: &str,
        sheet: u32,
        row: u32,
        col: u32,
        rows: u32,
        cols: u32,
    ) {
        if let Some(view) = self.office_view(file_id) {
            view.wants_cells(sheet, row, col, rows, cols);
        }
    }

    /// Try again on an Office file whose renderer stopped: the same renderer starts once more (the host's rule), with
    /// the bytes it holds. `None`: not that case (the source opens the file again).
    pub(in crate::engine_host::viewer) fn office_retry(
        &mut self,
        file_id: &str,
    ) -> Option<Result<OpenFile, OpenError>> {
        let shown = self
            .office
            .shown
            .as_mut()
            .filter(|shown| shown.file_id == file_id && !shown.ended)?;
        let kept = shown.kept;
        if !shown.view.as_mut()?.retry() {
            return None;
        }
        let edit = self.open.as_ref().and_then(|opening| opening.edit);
        Some(self.facts_now(file_id).map(|(_, _, facts)| {
            let mut state = StateBody::new(ViewState::Opening, kept);
            state.tries = 1;
            file_of(file_id, facts, Body::OfficeState(state), edit)
        }))
    }

    /// The Office file is left: its renderer ends, its pictures and text go.
    pub(in crate::engine_host::viewer) fn office_left(&mut self) {
        if let Some(mut shown) = self.office.shown.take() {
            shown.end(CloseWhy::Closed);
        }
    }
}
