//! The Files window's Two versions dialog on the real engine: [`ConflictCalls`], the part of `EngineFilesSource` behind
//! `conflicts` and `keep_version`.
//!
//! - The files that wait for a choice are the engine's `FileStatus::Conflict` rows ([`Engine::conflicts_for`]), read
//!   on a worker and held. "This PC" is the file on this PC (its last change and length on disk); "Another device" is
//!   the server's current version (when, size). The server names no device, so none is said: the mark is the
//!   Computer's. Held edits and the other kinds of the review feed stay in This PC's list.
//! - Nothing is drawn before the engine answered: until then `conflicts` is empty. A file whose side could not be
//!   read is not drawn (the engine counts it).
//! - An answer ([`Engine::resolve_conflict_for`]) goes out once per press, on a worker, bound to the session AND the
//!   account of its press. It is accepted at once; while it runs the file is not offered again; if it did not work
//!   the file waits again and the window is told ([`Change::Failed`]). Keep there replaces a file on this PC: the
//!   engine keeps the replaced copy beside it first.
//! - What is held, and every answer on its way, goes the moment the session or the account moves; an answer that
//!   lands under another session or account is dropped.
//!
//! Thread rule: nothing here blocks the window. Each ask has a worker of its own, and wakes the window when its
//! answer is in.
//!
//! Must not: log a name, a path or an id; draw a side the engine did not read; repeat an answer.

use std::collections::HashMap;
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use beebeeb_engine::account::SessionGeneration;
use beebeeb_engine::app::conflicts_api::{
    ConflictChoice, Conflicts, ConflictsRefused, ResolveRefused, WaitingConflict,
};
use beebeeb_engine::app::settings_api::AccountCallError;
use beebeeb_engine::app::{CredentialStores, Engine};

use super::log;
use crate::frames::files::actions_data::{Conflict, Device, Keep, Side};
use crate::frames::files::source::{Action, ActionError, Change};

#[cfg(test)]
#[path = "files_conflicts_tests.rs"]
mod tests;

/// The list is read again when it is this old.
const REFRESH: Duration = Duration::from_secs(20);

/// What the adapter asks of the engine handle. The app's is `Engine`; a test's is its own.
pub trait ConflictEngine: Send + Sync + 'static {
    /// Someone is signed in and the vault is open. Never waits.
    fn unlocked(&self) -> bool;
    /// The session in memory now. Never waits.
    fn session(&self) -> SessionGeneration;
    /// Whose account is signed in. Never waits.
    fn account(&self) -> Option<String>;
    /// The files that wait for a choice. Blocks: a worker's.
    fn waiting(&self, session: SessionGeneration) -> Result<Conflicts, ConflictsRefused>;
    /// Apply an answer. Blocks: a worker's.
    fn resolve(&self, session: SessionGeneration, file: &str, choice: ConflictChoice) -> Result<(), ResolveRefused>;
    /// Redraw the window: an answer is there.
    fn wake(&self);
}

impl<C: CredentialStores> ConflictEngine for Engine<C> {
    fn unlocked(&self) -> bool {
        self.has_session()
    }
    fn session(&self) -> SessionGeneration {
        self.session_turn()
    }
    fn account(&self) -> Option<String> {
        self.account_email().or_else(|| self.user_id())
    }
    fn waiting(&self, session: SessionGeneration) -> Result<Conflicts, ConflictsRefused> {
        self.conflicts_for(session)
    }
    fn resolve(&self, session: SessionGeneration, file: &str, choice: ConflictChoice) -> Result<(), ResolveRefused> {
        self.resolve_conflict_for(session, file, choice)
    }
    fn wake(&self) {
        self.wake_host();
    }
}

/// The session and the account of a press.
type Stamp = (SessionGeneration, Option<String>);

enum Answer {
    List {
        pressed: Stamp,
        epoch: u64,
        list: Result<Vec<Conflict>, ConflictsRefused>,
    },
    Resolved {
        pressed: Stamp,
        file: String,
        result: Result<(), ResolveRefused>,
    },
}

/// See the module's doc.
pub struct ConflictCalls<E: ConflictEngine> {
    engine: Arc<E>,
    answers: mpsc::Sender<Answer>,
    inbox: mpsc::Receiver<Answer>,
    /// The session and account the last look saw: when they move, everything held goes.
    seen: Option<Stamp>,
    /// What the engine listed, and when. `None`: not read (yet).
    held: Option<(Vec<Conflict>, Instant)>,
    /// A read is on its way.
    reading: bool,
    /// An answer was applied: the list is read again now, whatever its age.
    due: bool,
    /// Moves on every answer applied, so a read that started before it is not trusted after.
    epoch: u64,
    /// The files an answer is on its way for, each with the session of its press.
    resolving: HashMap<String, SessionGeneration>,
    /// What failed and the window has not taken yet.
    changes: Vec<Change>,
}

impl<E: ConflictEngine> ConflictCalls<E> {
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
            due: false,
            epoch: 0,
            resolving: HashMap::new(),
            changes: Vec::new(),
        }
    }

    fn stamp(&self) -> Stamp {
        (self.engine.session(), self.engine.account())
    }

    /// The session or the account is not the one the last look saw: nothing of the earlier one is kept, here, now.
    fn look(&mut self, now: &Stamp) {
        if self.seen.as_ref() == Some(now) {
            return;
        }
        self.seen = Some(now.clone());
        self.held = None;
        self.reading = false;
        self.due = false;
        self.epoch += 1;
        self.resolving.clear();
        self.changes.clear();
    }

    /// The files that wait for a choice, the first to answer first; only while the session and the account are the
    /// ones that were read, and never a file an answer is on its way for.
    pub fn conflicts(&self) -> Vec<Conflict> {
        if !self.engine.unlocked() || self.seen.as_ref() != Some(&self.stamp()) {
            return Vec::new();
        }
        match &self.held {
            Some((list, _)) => list
                .iter()
                .filter(|conflict| !self.resolving.contains_key(&conflict.file))
                .cloned()
                .collect(),
            None => Vec::new(),
        }
    }

    /// Answer the conflict of `file`. `Ok`: accepted and sent on a worker; a failure comes back as a
    /// [`Change::Failed`]. Refused with nothing sent: no session, an unknown file, an answer already on its way.
    pub fn keep(&mut self, file: &str, keep: Keep) -> Result<(), ActionError> {
        let pressed = self.stamp();
        self.look(&pressed);
        if !self.engine.unlocked() {
            return Err(ActionError::Unavailable);
        }
        if self.resolving.contains_key(file) {
            return Err(ActionError::InFlight);
        }
        if !self
            .held
            .as_ref()
            .is_some_and(|(list, _)| list.iter().any(|c| c.file == file))
        {
            return Err(ActionError::Failed);
        }
        let choice = match keep {
            Keep::Here => ConflictChoice::KeepHere,
            Keep::There => ConflictChoice::KeepThere,
            Keep::Both => ConflictChoice::KeepBoth,
        };
        let (engine, answers, id) = (self.engine.clone(), self.answers.clone(), file.to_string());
        let sent = pressed.clone();
        let started = std::thread::Builder::new()
            .name("files-conflict".into())
            .spawn(move || {
                // A panic must not leave an answer in flight for good: it answers as failed.
                let result =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| engine.resolve(sent.0, &id, choice)))
                        .unwrap_or_else(|_| {
                            log("files: an answer to two versions ended early");
                            Err(ResolveRefused::Failed)
                        });
                let _ = answers.send(Answer::Resolved {
                    pressed: sent,
                    file: id,
                    result,
                });
                engine.wake();
            });
        if started.is_err() {
            log("files: could not start the worker");
            return Err(ActionError::Failed);
        }
        self.resolving.insert(file.to_string(), pressed.0);
        Ok(())
    }

    /// Take in what the workers answered, and read the list when it is not there or is old. Every frame.
    pub fn take(&mut self) -> Vec<Change> {
        let now = self.stamp();
        self.look(&now);
        let here = self.engine.unlocked();
        while let Ok(answer) = self.inbox.try_recv() {
            match answer {
                Answer::List { pressed, epoch, list } => {
                    if epoch == self.epoch && here && pressed == now {
                        self.reading = false;
                        // A failed read holds nothing: a list that cannot be trusted is not drawn.
                        self.held = list.ok().map(|list| (list, Instant::now()));
                        self.due = false;
                    } else if pressed == now {
                        // Older than an answer applied since: read again.
                        self.reading = false;
                    }
                }
                Answer::Resolved { pressed, file, result } => {
                    if self.resolving.get(&file) == Some(&pressed.0) {
                        self.resolving.remove(&file);
                    }
                    if !(here && pressed == now) {
                        log("files: the answer of an earlier session was dropped");
                        continue;
                    }
                    self.epoch += 1;
                    // Whatever happened, the engine's rows decide what waits: read again.
                    self.due = true;
                    match result {
                        Ok(()) | Err(ResolveRefused::NotWaiting) => {
                            if let Some((list, _)) = &mut self.held {
                                list.retain(|conflict| conflict.file != file);
                            }
                        }
                        Err(why) => self.changes.push(Change::Failed(Action::KeepVersion, refusal(why))),
                    }
                }
            }
        }
        if here && !self.reading && (self.due || self.held.as_ref().is_none_or(|(_, at)| at.elapsed() >= REFRESH)) {
            self.read(now);
        }
        std::mem::take(&mut self.changes)
    }

    fn read(&mut self, pressed: Stamp) {
        let (engine, answers, epoch) = (self.engine.clone(), self.answers.clone(), self.epoch);
        let started = std::thread::Builder::new()
            .name("files-conflicts".into())
            .spawn(move || {
                let list = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| engine.waiting(pressed.0)))
                    .unwrap_or_else(|_| {
                        log("files: a read of two versions ended early");
                        Err(ConflictsRefused::NoLocalState)
                    })
                    .map(|read| read.files.iter().map(shown).collect());
                let changed = !matches!(&list, Ok(list) if list.is_empty());
                let _ = answers.send(Answer::List { pressed, epoch, list });
                // An empty answer changes nothing on the screen: no redraw for it.
                if changed {
                    engine.wake();
                }
            });
        if started.is_ok() {
            self.reading = true;
        } else {
            log("files: could not start the worker");
        }
    }
}

fn refusal(why: ResolveRefused) -> ActionError {
    match why {
        ResolveRefused::NotSignedIn | ResolveRefused::NoLocalState | ResolveRefused::NoSyncFolder => {
            ActionError::Unavailable
        }
        ResolveRefused::Call(AccountCallError::Offline) => ActionError::Offline,
        _ => ActionError::Failed,
    }
}

/// A waiting file as the dialog draws it. The server names no device, so none is invented.
fn shown(waiting: &WaitingConflict) -> Conflict {
    Conflict {
        file: waiting.file_id.clone(),
        name: waiting.name.clone(),
        here: Side {
            device: "This PC".to_string(),
            kind: Device::Computer,
            when: crate::ui::file_list::format_date(&waiting.here_modified),
            size: crate::source::format_size(waiting.here_size.min(i64::MAX as u64) as i64),
        },
        there: Side {
            device: "Another device".to_string(),
            kind: Device::Computer,
            when: crate::ui::file_list::format_date(&waiting.there_created),
            size: crate::source::format_size(waiting.there_size.min(i64::MAX as u64) as i64),
        },
    }
}
