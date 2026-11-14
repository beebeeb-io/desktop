//! The Files window's Uploads panel on the real engine: [`UploadCalls`], the part of `EngineFilesSource` behind
//! `uploads`, `retry_upload`, `cancel_upload`, `pause_uploads` and `forget_uploads`.
//!
//! - What the panel shows is ONE read of the engine ([`UploadsEngine::uploads_for`]) held here with the session and
//!   account it was read under. [`UploadCalls::uploads`] only answers from what is held, never reads (it is on a draw
//!   path), and says nothing under another stamp.
//! - A read is asked when the panel opens or the window comes to the front, after an accepted upload, after every
//!   answer of Try again, Pause and Cancel, and (in [`UploadCalls::take`], on the engine's wake) again while
//!   something moves or waits and what is held is older than a second. No timer thread.
//! - Try again: one queued change, by the queue's own rule; Pause all is the sync pause (the board's DW-0427/DW-1204:
//!   downloads wait too); Cancel is for a NEW file only: its copy goes to the Recycle Bin, the queued change is
//!   dropped, nothing is sent to Beebeeb. Nothing is drawn before the engine answered.
//! - Forget drops what is held; it never deletes a queued change (what becomes of unsent changes is the engine's
//!   sign-out's).
//! - An answer under another session or account is dropped.
//!
//! Must not: block the window; log or print a name or a folder; draw a state the engine did not report.

use std::collections::HashSet;
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use beebeeb_engine::account::SessionGeneration;
use beebeeb_engine::app::uploads_api::{CancelOutcome, UploadStands, UploadsRead};
use beebeeb_engine::app::{CredentialStores, Engine, LocalStateRefused};
use beebeeb_engine::state_db::RetryOutcome;

use super::log;
use super::stamp::{Stamp, stamp};
use crate::frames::files::source::{Action, ActionError, Change};
use crate::frames::files::uploads::{Upload, UploadState, Uploads};
use crate::source::format_size;

#[cfg(test)]
#[path = "files_uploads_tests.rs"]
mod tests;

/// How old a held read may be before the engine's wake reads again, while something moves or waits.
const STALE: Duration = Duration::from_secs(1);

/// What the calls need of the engine: the real handle, or a double in a test.
pub trait UploadsEngine: Send + Sync + 'static {
    fn stamp(&self) -> Stamp;
    fn has_session(&self) -> bool;
    fn uploads_for(&self, session: SessionGeneration) -> Result<UploadsRead, LocalStateRefused>;
    fn retry_upload_for(&self, session: SessionGeneration, change: &str) -> Result<RetryOutcome, LocalStateRefused>;
    fn cancel_upload_for(&self, session: SessionGeneration, change: &str) -> Result<CancelOutcome, LocalStateRefused>;
    /// The sync pause (`Engine::pause` / `resume`): writes a file, so a worker's.
    fn set_paused(&self, paused: bool) -> Result<(), String>;
    fn wake(&self);
}

impl<C: CredentialStores> UploadsEngine for Engine<C> {
    fn stamp(&self) -> Stamp {
        stamp(self)
    }
    fn has_session(&self) -> bool {
        Engine::has_session(self)
    }
    fn uploads_for(&self, session: SessionGeneration) -> Result<UploadsRead, LocalStateRefused> {
        Engine::uploads_for(self, session)
    }
    fn retry_upload_for(&self, session: SessionGeneration, change: &str) -> Result<RetryOutcome, LocalStateRefused> {
        Engine::retry_upload_for(self, session, change)
    }
    fn cancel_upload_for(&self, session: SessionGeneration, change: &str) -> Result<CancelOutcome, LocalStateRefused> {
        Engine::cancel_upload_for(
            self,
            session,
            change,
            &beebeeb_engine::app::uploads_api::move_to_recycle_bin,
        )
    }
    fn set_paused(&self, paused: bool) -> Result<(), String> {
        if paused { self.pause() } else { self.resume() }
    }
    fn wake(&self) {
        self.wake_host();
    }
}

enum Said {
    Read(Result<UploadsRead, LocalStateRefused>),
    Retry {
        change: String,
        result: Result<RetryOutcome, LocalStateRefused>,
    },
    Cancel {
        change: String,
        file: String,
        result: Result<CancelOutcome, LocalStateRefused>,
    },
    Pause(Result<(), String>),
}

struct Answer {
    pressed: Stamp,
    epoch: u64,
    said: Said,
}

/// See the module's doc.
pub struct UploadCalls<E: UploadsEngine> {
    engine: Arc<E>,
    answers: mpsc::Sender<Answer>,
    inbox: mpsc::Receiver<Answer>,
    /// The session and account the last look saw: when they move, everything held goes.
    seen: Option<Stamp>,
    /// The last read, with the stamp it was made under and when it landed.
    held: Option<(Stamp, UploadsRead, Instant)>,
    /// A read is on its way.
    reading: bool,
    /// The changes a Try again or a Cancel is on its way for.
    in_flight: HashSet<String>,
    /// A pause or a resume is on its way.
    pausing: bool,
    changes: Vec<Change>,
    /// Moves at every forget: an answer of an earlier one is dropped.
    epoch: u64,
}

impl<E: UploadsEngine> UploadCalls<E> {
    /// Starts no thread and reads nothing.
    pub fn new(engine: Arc<E>) -> Self {
        let (answers, inbox) = mpsc::channel();
        Self {
            engine,
            answers,
            inbox,
            seen: None,
            held: None,
            reading: false,
            in_flight: HashSet::new(),
            pausing: false,
            changes: Vec::new(),
            epoch: 0,
        }
    }

    fn look(&mut self, now: &Stamp) {
        if self.seen.as_ref() == Some(now) {
            return;
        }
        self.seen = Some(now.clone());
        self.held = None;
        self.reading = false;
        self.in_flight.clear();
        self.pausing = false;
        self.changes.clear();
    }

    /// Read again now (the panel opened, the window came to the front). One read at a time.
    pub fn ask(&mut self) {
        let now = self.engine.stamp();
        self.look(&now);
        self.start_read(now);
    }

    fn start_read(&mut self, pressed: Stamp) {
        if self.reading || !self.engine.has_session() {
            return;
        }
        self.reading = true;
        let session = pressed.0;
        self.work(pressed, move |engine| Said::Read(engine.uploads_for(session)));
    }

    /// A worker for one call: its answer goes down the channel, then the window is woken.
    fn work(&self, pressed: Stamp, call: impl FnOnce(&E) -> Said + Send + 'static) {
        let (engine, answers, epoch) = (self.engine.clone(), self.answers.clone(), self.epoch);
        let started = std::thread::Builder::new().name("files-uploads".into()).spawn(move || {
            let said = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| call(&engine)));
            match said {
                Ok(said) => {
                    let _ = answers.send(Answer { pressed, epoch, said });
                }
                Err(_) => {
                    log("files: an uploads call ended early");
                    let _ = answers.send(Answer {
                        pressed,
                        epoch,
                        said: Said::Read(Err(LocalStateRefused::Unreadable)),
                    });
                }
            }
            engine.wake();
        });
        if started.is_err() {
            log("files: could not start the worker");
        }
    }

    /// What the panel shows: from the held read only, and only under the stamp it was read for.
    pub fn uploads(&self) -> Option<Uploads> {
        let (read_under, read, _) = self.held.as_ref()?;
        if *read_under != self.engine.stamp() {
            return None;
        }
        words(read)
    }

    /// The change an upload id stands for, among the rows held, with the row's state.
    fn row_of(&self, id: &str) -> Option<&beebeeb_engine::app::uploads_api::UploadRow> {
        self.held.as_ref()?.1.rows.iter().find(|row| row.file_id == id)
    }

    pub fn retry(&mut self, id: &str) -> Result<(), ActionError> {
        let pressed = self.engine.stamp();
        self.look(&pressed);
        let row = self
            .row_of(id)
            .filter(|_| self.held.as_ref().is_some_and(|h| h.0 == pressed));
        let Some(row) = row else {
            return Err(ActionError::Unavailable);
        };
        if !matches!(row.stands, UploadStands::Stopped { retryable: true, .. }) {
            return Err(ActionError::Unavailable);
        }
        let change = row.change_id.clone();
        if !self.engine.has_session() {
            return Err(ActionError::Unavailable);
        }
        if !self.in_flight.insert(change.clone()) {
            return Err(ActionError::InFlight);
        }
        let session = pressed.0;
        self.work(pressed, move |engine| Said::Retry {
            result: engine.retry_upload_for(session, &change),
            change,
        });
        Ok(())
    }

    /// Only for a NEW file (the engine says which rows are); anything else is `Unavailable`, nothing asked.
    pub fn cancel(&mut self, id: &str) -> Result<(), ActionError> {
        let pressed = self.engine.stamp();
        self.look(&pressed);
        let row = self
            .row_of(id)
            .filter(|_| self.held.as_ref().is_some_and(|h| h.0 == pressed));
        let Some(row) = row.filter(|row| row.new_file) else {
            return Err(ActionError::Unavailable);
        };
        let (change, file) = (row.change_id.clone(), row.file_id.clone());
        if !self.engine.has_session() {
            return Err(ActionError::Unavailable);
        }
        if !self.in_flight.insert(change.clone()) {
            return Err(ActionError::InFlight);
        }
        let session = pressed.0;
        self.work(pressed, move |engine| Said::Cancel {
            result: engine.cancel_upload_for(session, &change),
            change,
            file,
        });
        Ok(())
    }

    /// Whether the row of this upload may be cancelled: held, this stamp, a new file.
    pub fn cancels(&self, id: &str) -> bool {
        self.held.as_ref().is_some_and(|h| h.0 == self.engine.stamp())
            && self.row_of(id).is_some_and(|row| row.new_file)
    }

    /// Pause all (`true`) or Resume all: the sync pause, on a worker, one at a time; drawn at the re-read.
    pub fn pause(&mut self, pause: bool) -> Result<(), ActionError> {
        let pressed = self.engine.stamp();
        self.look(&pressed);
        if self.pausing {
            return Err(ActionError::InFlight);
        }
        self.pausing = true;
        self.work(pressed, move |engine| Said::Pause(engine.set_paused(pause)));
        Ok(())
    }

    /// Drop everything held. Nothing is sent to the engine; the queue is not touched.
    pub fn forget(&mut self) {
        self.held = None;
        self.reading = false;
        self.in_flight.clear();
        self.pausing = false;
        self.changes.clear();
        // Whatever is on its way now carries the earlier epoch: dropped.
        self.epoch += 1;
    }

    /// Take in what the workers answered, read again when what is held is old, and hand over what the engine did.
    pub fn take(&mut self) -> Vec<Change> {
        let now = self.engine.stamp();
        self.look(&now);
        let here = self.engine.has_session();
        let mut reread = false;
        while let Ok(answer) = self.inbox.try_recv() {
            let mine = here && answer.pressed == now && answer.epoch == self.epoch;
            match answer.said {
                Said::Read(result) => {
                    if mine {
                        self.reading = false;
                        match result {
                            Ok(read) => self.held = Some((now.clone(), read, Instant::now())),
                            Err(_) => log("files: the uploads could not be read"),
                        }
                    }
                }
                Said::Retry { change, result } => {
                    self.in_flight.remove(&change);
                    if !mine {
                        continue;
                    }
                    match result {
                        Ok(RetryOutcome::Retried | RetryOutcome::AlreadyQueued) => {}
                        Ok(_) => self
                            .changes
                            .push(Change::Failed(Action::RetryUpload, ActionError::Failed)),
                        Err(_) => self
                            .changes
                            .push(Change::Failed(Action::RetryUpload, ActionError::Unavailable)),
                    }
                    reread = true;
                }
                Said::Cancel { change, file, result } => {
                    self.in_flight.remove(&change);
                    if !mine {
                        continue;
                    }
                    match result {
                        Ok(CancelOutcome::Cancelled) => self.changes.push(Change::Cancelled { id: file }),
                        Ok(CancelOutcome::Busy) => self
                            .changes
                            .push(Change::Failed(Action::CancelUpload, ActionError::InFlight)),
                        Ok(CancelOutcome::NotNew) => self
                            .changes
                            .push(Change::Failed(Action::CancelUpload, ActionError::Unavailable)),
                        Ok(CancelOutcome::Unknown | CancelOutcome::RecycleFailed) => self
                            .changes
                            .push(Change::Failed(Action::CancelUpload, ActionError::Failed)),
                        Err(_) => self
                            .changes
                            .push(Change::Failed(Action::CancelUpload, ActionError::Unavailable)),
                    }
                    reread = true;
                }
                Said::Pause(result) => {
                    self.pausing = false;
                    if !mine {
                        continue;
                    }
                    if result.is_err() {
                        self.changes
                            .push(Change::Failed(Action::PauseUploads, ActionError::Failed));
                    }
                    reread = true;
                }
            }
        }
        let busy = self.held.as_ref().is_some_and(|(_, read, at)| {
            at.elapsed() > STALE
                && read
                    .rows
                    .iter()
                    .any(|row| !matches!(row.stands, UploadStands::Stopped { .. }))
        });
        if reread || busy {
            self.start_read(now);
        }
        std::mem::take(&mut self.changes)
    }
}

/// The panel's and the status line's words for one read; `None`: nothing to show.
fn words(read: &UploadsRead) -> Option<Uploads> {
    if read.rows.is_empty() {
        return None;
    }
    let mut items = Vec::with_capacity(read.rows.len());
    let (mut active, mut stopped, mut left) = (0usize, 0usize, 0u64);
    for row in &read.rows {
        let state = match &row.stands {
            UploadStands::Stopped { words, .. } => {
                stopped += 1;
                UploadState::Failed(words.clone())
            }
            UploadStands::Moving { sent, total } => {
                active += 1;
                left += total.saturating_sub(*sent);
                let (sent_words, total_words) = (format_size(*sent as i64), format_size(*total as i64));
                UploadState::Uploading {
                    words: format!("{sent_words} of {total_words}"),
                    row: format!("Uploading, {sent_words} of {total_words}"),
                    done: if *total == 0 {
                        0.0
                    } else {
                        (*sent as f32 / *total as f32).clamp(0.0, 1.0)
                    },
                }
            }
            UploadStands::Waiting => {
                active += 1;
                left += row.size;
                let folder = if row.folder.is_empty() {
                    "Files"
                } else {
                    row.folder.as_str()
                };
                UploadState::Waiting(format!("{} \u{00B7} To {folder}", format_size(row.size as i64)))
            }
        };
        items.push(Upload {
            id: row.file_id.clone(),
            name: row.name.clone(),
            state,
            picture: None,
        });
    }
    let (status, figures) = if active == 0 {
        let line = if stopped == 1 {
            "1 upload needs attention".to_string()
        } else {
            format!("{stopped} uploads need attention")
        };
        (line.clone(), line)
    } else {
        let count = if active == 1 {
            "1 file".to_string()
        } else {
            format!("{active} files")
        };
        let figures = format!("{count} \u{00B7} {} left", format_size(left as i64));
        let status = if read.paused {
            format!("Paused \u{00B7} {figures}")
        } else {
            format!("Uploading {figures}")
        };
        (status, figures)
    };
    Some(Uploads {
        status,
        figures,
        items,
        paused: read.paused,
    })
}
