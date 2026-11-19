//! The engine source's editor: the `ViewerSource` editor methods over the open file's editing session
//! (`OfficeHost`, through [`OfficeView`]), the save and its conflict, and New document. Child of `viewer_office.rs`.
//!
//! - **Edits** (`edit`, `hit`, `copy_selection`, `layouts`, `insert_image`, `poll_edit`) go to the session of the open
//!   file, only while its session is the session (as every accessor of the Office arm). The answers wait in the view
//!   and come by `poll_edit`; an `Edited` changed what the view holds first (`OfficeView::apply_edited`).
//! - **Save** (`save_document`, `resolve_document`) rides the text save's road: the press takes the stamp (session,
//!   account, opening, the engine's opening epoch) and the base, asks the session for the document's bytes
//!   (`SaveSize`), and once they are here (the window's poll, every frame) a worker makes the engine call
//!   (`save_office_for`, `save_office_copy_for`) for that stamp. The answer comes by `settled`, through the same checks
//!   as a text's: another session, an earlier opening, a lock between. Until it does the source is busy, so the file is
//!   not opened again meanwhile. A failure at any step (the session refused, the helper stopped, the session moved)
//!   settles as not done and ends the busy state. The engine reads the package again before it stores anything.
//!   - Keep mine: saved from the version the file is at now (`edit_facts` on the worker). Keep both: a copy next to it,
//!     under this PC's name. Keep theirs: the file is opened again as Beebeeb has it (`begin`), which ends the session
//!     and its edits.
//!   - The answer's body is the document as it is on screen (`OfficeView::body_now`), filled in when the window takes
//!     it. The session goes on: a saved document's helper keeps running on the new version.
//! - **New document** (`create_document`): a session from the template (`new_doc`), its bytes, `create_office_for` on
//!   a worker, then the new file is opened as any file is (`begin`) and `settled` answers `Settled::Saved` under the
//!   NEW file's id, with the body `begin` gave. The session of the template is closed once its bytes are here.
//!
//! Not here: snapshots (w04-ui-recovery's). `Purpose::Snapshot` bytes are never asked.
//!
//! Must not: log a name, a cell or a byte.

#![cfg_attr(not(test), allow(dead_code))] // the editor's window reaches these through `ViewerSource` once it is wired

use std::sync::mpsc;

use beebeeb_engine::app::viewer_edit_api::{EditRefused, SaveEnd};

use super::super::change::{EARLIER_HELD, EARLIER_NOT_SENT, Stamp, this_pc};
use super::*;
use crate::frames::viewers::source::{Conflict, Keep, SaveOutcome, Settled};

/// What a save is for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum How {
    /// The document as the next version of the file, from the press's base.
    Save,
    /// Keep mine: from the version the file is at now.
    Mine,
    /// Keep both: a copy next to the file.
    Both,
}

/// A save whose bytes have been asked of the session.
struct Pending {
    file_id: String,
    stamp: Stamp,
    base: i64,
    how: How,
    request: u64,
}

/// A new document being made: its session, until the bytes are here.
struct Creating {
    view: OfficeView,
    folder: Option<String>,
    name: String,
    session: SessionGeneration,
    account: Option<String>,
    request: Option<u64>,
}

/// What the worker of a New document came to, for the session and the account it was started for.
struct Made {
    session: SessionGeneration,
    account: Option<String>,
    result: Result<String, EditRefused>,
    /// The document's own bytes: the new file opens on these, not on its copy on this PC (queued, maybe not on disk yet).
    bytes: Zeroizing<Vec<u8>>,
}

/// What the editor has on its way.
pub(super) struct Work {
    saving: Option<Pending>,
    creating: Option<Creating>,
    /// The revision of the document the save on its way was made from (the helper's, with its bytes): said to the
    /// window as `Settled::SavedRevision` just before the save's own answer.
    revision: Option<u64>,
    /// The save's own answer, held back for one `settled` while its revision is said first.
    after_revision: Option<Settled>,
    made_tx: mpsc::Sender<Made>,
    made_rx: mpsc::Receiver<Made>,
    /// The bytes of the document just made, held for the window's own open of it (`Ui::open` asks the source to open
    /// the new file once more, after `Settled::Saved`): that open reads these, never the copy on this PC (queued, not
    /// on disk, or a placeholder on Windows: "Couldn't read the copy on this PC"). Taken by that open.
    pub(super) made_on: Option<MadeOn>,
}

/// The document a New document made: the file, the session it was made for, its bytes.
pub(super) struct MadeOn {
    pub(super) file_id: String,
    pub(super) session: SessionGeneration,
    pub(super) bytes: Zeroizing<Vec<u8>>,
}

impl Work {
    /// A save waits for its bytes or its answer.
    pub(super) fn is_saving(&self) -> bool {
        self.saving.is_some()
    }
}

impl Default for Work {
    fn default() -> Self {
        let (made_tx, made_rx) = mpsc::channel();
        Work {
            saving: None,
            creating: None,
            revision: None,
            after_revision: None,
            made_tx,
            made_rx,
            made_on: None,
        }
    }
}

/// How long a New document waits for its upload to land before the window opens it.
const CREATE_LAND_WAIT: Duration = Duration::from_secs(20);
const CREATE_LAND_POLL: Duration = Duration::from_millis(100);

/// A new file is only queued when `create_office` answers: the local state holds no version for it (0) until its
/// upload lands, and the window offers the editor only for a file with a version (`office_begin`). Opened at once, on
/// Windows the new document came up as the read-only viewer, and Ctrl+S saved nothing (w24, VM 401). A worker's:
/// bounded, and it ends at once when the version cannot be read.
fn wait_until_landed<E: ViewerEngine>(engine: &E, session: SessionGeneration, file_id: &str) {
    let deadline = Instant::now() + CREATE_LAND_WAIT;
    while Instant::now() < deadline {
        match engine.edit_facts(session, file_id, 0) {
            Some(facts) if facts.version == 0 => std::thread::sleep(CREATE_LAND_POLL),
            _ => return,
        }
    }
}

/// "09:41", this PC's clock.
fn hhmm() -> String {
    chrono::Local::now().format("%H:%M").to_string()
}

/// The body of a save's answer until the window takes it (`office_fill` puts the document's own there): nothing
/// of the document.
fn placeholder() -> Body {
    Body::OfficeState(StateBody::new(ViewState::Opening, false))
}

/// What a save came to, as `settle_save` words a text's: the answer and the version the view then holds.
fn settle(saved: Result<SaveEnd, EditRefused>, id: &str, name: &str) -> (Settled, Option<i64>) {
    match saved {
        Ok(SaveEnd::Saved { version }) => (
            Settled::Saved {
                id: id.to_string(),
                name: name.to_string(),
                body: placeholder(),
                version: u32::try_from(version).unwrap_or(0),
                at: hhmm(),
            },
            Some(version),
        ),
        Ok(SaveEnd::Queued { base }) => (
            Settled::Queued {
                body: placeholder(),
                version: u32::try_from(base).unwrap_or(0),
            },
            Some(base),
        ),
        Ok(SaveEnd::Conflict { theirs }) => (
            Settled::Conflict(Conflict {
                from: String::new(),
                at: hhmm(),
                version: u32::try_from(theirs).unwrap_or(0),
            }),
            None,
        ),
        Err(EditRefused::EarlierChangeNotSent) => (Settled::Refused(EARLIER_NOT_SENT.to_string()), None),
        Err(EditRefused::EarlierChangeHeld) => (Settled::Refused(EARLIER_HELD.to_string()), None),
        Err(_) => (Settled::Failed, None),
    }
}

impl<E: ViewerEngine> EngineViewerSource<E> {
    pub(in crate::engine_host::viewer) fn office_can_edit(&self, file_id: &str) -> bool {
        self.office_view(file_id).is_some_and(OfficeView::can_edit)
    }

    pub(in crate::engine_host::viewer) fn office_edit(&mut self, file_id: &str, op: EditOp) -> bool {
        self.office_view_mut(file_id).is_some_and(|view| view.edit(op))
    }

    pub(in crate::engine_host::viewer) fn office_hit(
        &mut self,
        file_id: &str,
        page: u32,
        x_pt: f32,
        y_pt: f32,
        mode: HitMode,
    ) -> bool {
        self.office_view_mut(file_id)
            .is_some_and(|view| view.hit(page, x_pt, y_pt, mode))
    }

    pub(in crate::engine_host::viewer) fn office_copy(&mut self, file_id: &str) -> bool {
        self.office_view_mut(file_id).is_some_and(OfficeView::copy)
    }

    pub(in crate::engine_host::viewer) fn office_layouts(&mut self, file_id: &str) -> bool {
        self.office_view_mut(file_id).is_some_and(OfficeView::layouts)
    }

    pub(in crate::engine_host::viewer) fn office_insert_image(
        &mut self,
        file_id: &str,
        bytes: Zeroizing<Vec<u8>>,
        png: bool,
    ) -> bool {
        self.office_view_mut(file_id)
            .is_some_and(|view| view.insert_image(bytes, png))
    }

    pub(in crate::engine_host::viewer) fn office_poll_edit(&mut self, file_id: &str) -> Option<EditAnswer> {
        self.office_view_mut(file_id)?.next_answer()
    }

    // -----------------------------------------------------------------------------------------------------------
    // Save
    // -----------------------------------------------------------------------------------------------------------

    pub(in crate::engine_host::viewer) fn office_save_document(&mut self, file_id: &str, base: u32) -> SaveOutcome {
        self.office_ask_save(file_id, i64::from(base), How::Save)
    }

    pub(in crate::engine_host::viewer) fn office_resolve_document(&mut self, file_id: &str, keep: Keep) -> SaveOutcome {
        match keep {
            // My edit is dropped; the file as Beebeeb has it is opened again (the editing session ends with it).
            Keep::Theirs => match self.begin(file_id) {
                Ok(file) => SaveOutcome::Opened(file),
                Err(_) => SaveOutcome::Failed,
            },
            Keep::Mine => self.office_ask_save(file_id, 0, How::Mine),
            Keep::Both => self.office_ask_save(file_id, 0, How::Both),
        }
    }

    /// The press: the stamp and the base now, the session asked for the document's bytes. `Working`: they come on a
    /// later poll (`office_pump_save`).
    fn office_ask_save(&mut self, file_id: &str, base: i64, how: How) -> SaveOutcome {
        let writable = self
            .change_facts()
            .is_some_and(|(_, facts)| facts.can_write && facts.version > 0);
        let Some(stamp) = self.stamp_of(file_id).filter(|_| writable) else {
            return SaveOutcome::Failed;
        };
        let work = &self.office.work;
        if self.changes.busy || work.saving.is_some() || work.creating.is_some() {
            return SaveOutcome::Failed;
        }
        let Some(request) = self
            .office_view_mut(file_id)
            .and_then(|view| view.save(Purpose::Save))
        else {
            return SaveOutcome::Failed;
        };
        self.office.work.saving = Some(Pending {
            file_id: file_id.to_string(),
            stamp,
            base,
            how,
            request,
        });
        // The file is not opened again while its save is on its way (`begin`), and the window says so (`saving`).
        self.changes.busy = true;
        SaveOutcome::Working
    }

    /// The save waiting for its bytes: the engine call starts when they are here, the save ends as not done when
    /// they never will be.
    pub(in crate::engine_host::viewer) fn office_pump_save(&mut self, file_id: &str) {
        let Some(pending) = self.office.work.saving.as_ref().filter(|pending| pending.file_id == file_id) else {
            return;
        };
        let request = pending.request;
        let wait = match self
            .office
            .shown
            .as_mut()
            .filter(|shown| shown.file_id == file_id)
            .and_then(|shown| shown.view.as_mut())
        {
            Some(view) => view.save_wait(request),
            None => SaveWait::Failed,
        };
        match wait {
            SaveWait::Waiting => {}
            SaveWait::Failed => self.office_fail_save(),
            SaveWait::Done(bytes, revision) => {
                self.office.work.revision = Some(revision);
                self.office_start_save(bytes);
            }
        }
    }

    /// The save ends as not done, through the road of every answer (so the busy state ends where it always does).
    pub(in crate::engine_host::viewer) fn office_fail_save(&mut self) {
        let Some(pending) = self.office.work.saving.take() else {
            return;
        };
        if !self.spawn(&pending.file_id, pending.stamp, |_| (Settled::Failed, None)) {
            self.changes.busy = false;
        }
    }

    fn office_start_save(&mut self, bytes: Zeroizing<Vec<u8>>) {
        let Some(pending) = self.office.work.saving.take() else {
            return;
        };
        let Pending {
            file_id,
            stamp,
            base,
            how,
            ..
        } = pending;
        let (session, epoch, name, id) = (stamp.session, stamp.epoch, stamp.name.clone(), file_id.clone());
        let started = self.spawn(&file_id, stamp, move |engine| {
            let end = match how {
            How::Save => settle(engine.save_office(session, &id, &bytes, base, epoch), &id, &name),
            // Mine becomes the newest version: saved from the version the file is at now.
            How::Mine => match engine.edit_facts(session, &id, epoch) {
                Some(now) => settle(engine.save_office(session, &id, &bytes, now.version, epoch), &id, &name),
                None => (Settled::Failed, None),
            },
            // Mine next to theirs, under this PC's name. The copy is another file: what the view holds of this one
            // does not move.
            How::Both => {
                let at = hhmm();
                match engine.save_office_copy(session, &id, &bytes, &this_pc(), &at) {
                    Ok(copy) => (
                        Settled::Saved {
                            id: copy.id,
                            name: copy.name,
                            body: placeholder(),
                            version: 1,
                            at,
                        },
                        None,
                    ),
                    Err(_) => (Settled::Failed, None),
                }
            }
            };
            // The save landed: the file's recovery blob is not needed any more.
            if matches!(end.0, Settled::Saved { .. }) {
                let _ = engine.recovery_discard(session, &id);
            }
            end
        });
        if !started {
            self.changes.busy = false;
        }
    }

    // -----------------------------------------------------------------------------------------------------------
    // New document
    // -----------------------------------------------------------------------------------------------------------

    pub(in crate::engine_host::viewer) fn office_create(
        &mut self,
        folder: Option<&str>,
        name: &str,
        kind: EditKind,
        template: Template,
    ) -> bool {
        let work = &self.office.work;
        if !(self.office.edit_on && self.office.edits.allows(kind))
            || self.changes.busy
            || work.saving.is_some()
            || work.creating.is_some()
            || !self.engine.has_session()
        {
            return false;
        }
        let Some(helper) = (self.office.helper)() else {
            return false;
        };
        let waker = self.engine.clone();
        let wake: Wake = Arc::new(move || waker.wake());
        let scratch = self.engine.office_scratch(self.engine.session_turn());
        let view = (self.office.new_opener)(&helper, kind, template, wake, scratch);
        self.office.work.creating = Some(Creating {
            view,
            folder: folder.map(str::to_string),
            name: name.to_string(),
            session: self.engine.session_turn(),
            account: self.engine.account(),
            request: None,
        });
        self.changes.busy = true;
        true
    }

    /// The New document on its way: the template's session opens, its bytes are asked and, once here, a worker makes
    /// the file. `Some`: it ended.
    fn office_pump_create(&mut self) -> Option<Settled> {
        if let Ok(made) = self.office.work.made_rx.try_recv() {
            return Some(self.office_made(made));
        }
        let creating = self.office.work.creating.as_mut()?;
        let (engine, session, account) = (&self.engine, creating.session, creating.account.clone());
        if !(engine.has_session() && engine.session_turn() == session && engine.account() == account) {
            // Nothing was created yet: the session of the template goes (closed, wiped).
            self.office.work.creating = None;
            self.changes.busy = false;
            return Some(Settled::Failed);
        }
        creating.view.poll();
        enum Step {
            Wait,
            Fail,
            Go(Zeroizing<Vec<u8>>),
        }
        let step = match creating.request {
            None if creating.view.can_edit() => match creating.view.save(Purpose::Save) {
                Some(request) => {
                    creating.request = Some(request);
                    Step::Wait
                }
                None => Step::Fail,
            },
            None if creating.view.ended() => Step::Fail,
            None => Step::Wait,
            Some(request) => match creating.view.save_wait(request) {
                SaveWait::Waiting => Step::Wait,
                SaveWait::Failed => Step::Fail,
                SaveWait::Done(bytes, _) => Step::Go(bytes),
            },
        };
        match step {
            Step::Wait => None,
            Step::Fail => {
                self.office.work.creating = None;
                self.changes.busy = false;
                Some(Settled::Failed)
            }
            Step::Go(bytes) => {
                // The template's session is over: its bytes are here.
                let creating = self.office.work.creating.take()?;
                let (engine, tx) = (self.engine.clone(), self.office.work.made_tx.clone());
                let started = std::thread::Builder::new().name("viewer-office-create".into()).spawn(move || {
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        let id = engine.create_office(
                            creating.session,
                            creating.folder.as_deref(),
                            &creating.name,
                            &bytes,
                        )?;
                        wait_until_landed(&*engine, creating.session, &id);
                        Ok(id)
                    }))
                    .unwrap_or(Err(EditRefused::Failed));
                    let _ = tx.send(Made {
                        session: creating.session,
                        account: creating.account,
                        result,
                        bytes,
                    });
                    engine.wake();
                });
                if started.is_err() {
                    log("viewer: could not start the worker for a new document");
                    self.changes.busy = false;
                    return Some(Settled::Failed);
                }
                None
            }
        }
    }

    /// The new file's reader (started by `begin`, on its copy on this PC) is replaced by the bytes just made: that
    /// copy can be queued and not on disk yet, which showed "Couldn't read the copy on this PC".
    fn office_open_on_made_bytes(&mut self, file_id: &str, session: SessionGeneration, bytes: Zeroizing<Vec<u8>>) {
        let Some(shown) = self.office.shown.as_mut().filter(|shown| shown.file_id == file_id) else {
            return;
        };
        if shown.inbox.is_none() || shown.session != session {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let _ = tx.send(Ok((session, bytes)));
        shown.inbox = Some(rx);
        self.engine.wake();
    }

    /// The worker of a New document ended: the file is opened as any file is, and answered under its own id.
    fn office_made(&mut self, made: Made) -> Settled {
        self.changes.busy = false;
        let current = self.engine.has_session()
            && self.engine.session_turn() == made.session
            && self.engine.account() == made.account;
        if !current {
            return Settled::Said("Your new document was made. Find it in Files.".to_string());
        }
        match made.result {
            Ok(id) => match self.begin(&id) {
                Ok(file) => {
                    // The window opens this file again (`Ui::open`): that open too is on these bytes.
                    self.office.work.made_on = Some(MadeOn {
                        file_id: file.id.clone(),
                        session: made.session,
                        bytes: made.bytes.clone(),
                    });
                    self.office_open_on_made_bytes(&file.id, made.session, made.bytes);
                    Settled::Saved {
                        id: file.id,
                        name: file.name,
                        body: file.body,
                        version: file.version.max(1),
                        at: hhmm(),
                    }
                }
                Err(_) => Settled::Said("Your new document was made, but it could not be opened here.".to_string()),
            },
            Err(EditRefused::ReadOnly) => Settled::Refused("You can't add files to this folder.".to_string()),
            Err(_) => Settled::Failed,
        }
    }

    // -----------------------------------------------------------------------------------------------------------
    // What settled
    // -----------------------------------------------------------------------------------------------------------

    /// The answer of a change or a New document. A save's answer carries the document as it is on screen.
    pub(in crate::engine_host::viewer) fn office_settled(&mut self, file_id: &str) -> Option<Settled> {
        if let Some(settled) = self.office_pump_create() {
            return Some(settled);
        }
        if let Some(settled) = self.office.work.after_revision.take() {
            return Some(self.office_fill(file_id, settled));
        }
        let settled = self.take_settled(file_id)?;
        // A save that landed (or is stored here to be sent) says first which revision it holds: what was typed after
        // the bytes were taken stays unsaved in the window.
        let landed = matches!(settled, Settled::Saved { .. } | Settled::Queued { .. });
        match self.office.work.revision.take() {
            Some(revision) if landed => {
                self.office.work.after_revision = Some(settled);
                Some(Settled::SavedRevision(revision))
            }
            _ => Some(self.office_fill(file_id, settled)),
        }
    }

    /// A save of the open Office file answers with the body it holds: the placeholder of the worker is replaced.
    fn office_fill(&self, file_id: &str, settled: Settled) -> Settled {
        let Some(view) = self
            .office
            .shown
            .as_ref()
            .filter(|shown| shown.file_id == file_id)
            .and_then(|shown| shown.view.as_ref())
        else {
            return settled;
        };
        match settled {
            Settled::Saved {
                id,
                name,
                body: Body::OfficeState(_),
                version,
                at,
            } => Settled::Saved {
                id,
                name,
                body: view.body_now(),
                version,
                at,
            },
            Settled::Queued {
                body: Body::OfficeState(_),
                version,
            } => Settled::Queued {
                body: view.body_now(),
                version,
            },
            other => other,
        }
    }
}

#[cfg(test)]
#[path = "edit_window_tests.rs"]
mod window_tests;
