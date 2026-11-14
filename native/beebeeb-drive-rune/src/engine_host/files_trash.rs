//! The Files window's Move to Trash and its Undo, on the real engine: [`TrashCalls`], the part of
//! `EngineFilesSource` behind `trash` and `restore`.
//!
//! - Move to Trash: [`Engine::trash_items_for`], one item at a time. Undo: [`Engine::restore_items_for`]. Each is
//!   bound to the session AND the account of its press: both are read on the window's thread, at the press, before
//!   anything else; the engine sends nothing unless the session in memory is still that one (asked under its
//!   session-write lock, with the token copy).
//! - Undo keeps the stamp of the move it undoes: an item is put back only under the session it was moved to Trash
//!   in. What Undo may put back is held here, by id, with that stamp; an id this source did not move to Trash in the
//!   session that is here is refused with nothing sent.
//! - The call itself only says "accepted" or why not; what was done arrives through `FilesSource::changes`
//!   (`Change::Trash`), per item, once Beebeeb answered. Nothing is drawn before that.
//! - An answer that lands under another session or account is dropped: no row, no message, nothing kept of it.
//! - One change per item: while a change of an item is on its way, a second one of it sends nothing and says so.
//!
//! Thread rule: nothing here blocks the window. Each press has a worker of its own (the check against this PC's
//! local state and the calls); it sends its answer down one channel and wakes the window ([`Engine::wake_host`]).
//!
//! Must not: log a name, a path or an id; draw a change Beebeeb did not make; read the session on a worker.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, mpsc};

use beebeeb_engine::account::SessionGeneration;
use beebeeb_engine::app::settings_api::AccountCallError;
use beebeeb_engine::app::trash_api::NotTrashed;
use beebeeb_engine::app::{CredentialStores, Engine};
use beebeeb_engine::file_ops;

use super::stamp::{Stamp, stamp};
use super::{log, map_drive_err};
use crate::frames::files::source::{Action, ActionError, Change, TrashOutcome};
use crate::source::DriveItem;

#[cfg(test)]
#[path = "files_trash_tests.rs"]
mod tests;

/// What a worker sends back: the action, the stamp of its press, and per item asked whether it was made.
struct Answer {
    action: Action,
    pressed: Stamp,
    items: Vec<TrashOutcome>,
}

/// See the module's doc.
pub struct TrashCalls<C: CredentialStores> {
    engine: Arc<Engine<C>>,
    answers: mpsc::Sender<Answer>,
    inbox: mpsc::Receiver<Answer>,
    /// The session and account the last look saw: when they move, everything held goes.
    seen: Option<Stamp>,
    /// The items a change is on its way for, each with the session of its press.
    in_flight: HashMap<String, SessionGeneration>,
    /// What Undo may put back: the items this source moved to Trash in the session that is here, and the stamp of
    /// those presses (one stamp: everything held goes when it moves).
    undo: Option<(Stamp, HashSet<String>)>,
    /// What Beebeeb answered and the window has not taken yet.
    changes: Vec<Change>,
    /// Tests only: the next call waits on the first, on its worker, before it asks the engine anything, and says on
    /// the second that its answer is in the inbox.
    #[cfg(test)]
    next_call: Option<(mpsc::Receiver<()>, mpsc::Sender<()>)>,
}

impl<C: CredentialStores> TrashCalls<C> {
    /// Starts no thread and reads nothing.
    pub fn new(engine: Arc<Engine<C>>) -> Self {
        let (answers, inbox) = mpsc::channel();
        Self {
            engine,
            answers,
            inbox,
            seen: None,
            in_flight: HashMap::new(),
            undo: None,
            changes: Vec::new(),
            #[cfg(test)]
            next_call: None,
        }
    }

    /// The session or the account is not the one the last look saw: nothing of the earlier one is kept, here, now.
    fn look(&mut self, now: &Stamp) {
        if self.seen.as_ref() == Some(now) {
            return;
        }
        self.seen = Some(now.clone());
        self.in_flight.clear();
        self.undo = None;
        self.changes.clear();
    }

    /// Move these to Trash. `Ok`: accepted; what was done arrives through [`Self::take`]. Refused, with nothing
    /// sent: no session, or a change of one of them is still on its way.
    pub fn trash(&mut self, items: &[DriveItem]) -> Result<(), ActionError> {
        // The session and account of the press, read before anything else: the items are sent for it or not at all.
        let pressed = stamp(&self.engine);
        self.look(&pressed);
        let ids = items.iter().map(|item| item.id.clone()).collect();
        self.start(Action::Trash, pressed, ids)
    }

    /// Put these back (Undo), under the session they were moved to Trash in. Refused, with nothing sent: one of
    /// them was not moved to Trash by this source in the session that is here.
    pub fn restore(&mut self, items: &[DriveItem]) -> Result<(), ActionError> {
        let now = stamp(&self.engine);
        self.look(&now);
        // The stamp of the move to Trash, never one taken at this press. (`look` just dropped what was held if the
        // stamp moved, so the two are equal here; the held one is sent so that does not rest on `look` alone.)
        let Some((moved_under, held)) = &self.undo else {
            return Err(ActionError::Unavailable);
        };
        if !items.iter().all(|item| held.contains(&item.id)) {
            return Err(ActionError::Unavailable);
        }
        let ids = items.iter().map(|item| item.id.clone()).collect();
        self.start(Action::Restore, moved_under.clone(), ids)
    }

    fn start(&mut self, action: Action, pressed: Stamp, ids: Vec<String>) -> Result<(), ActionError> {
        if !self.engine.has_session() {
            return Err(ActionError::Unavailable);
        }
        if ids.iter().any(|id| self.in_flight.contains_key(id)) {
            return Err(ActionError::InFlight);
        }
        if ids.is_empty() {
            return Ok(());
        }
        let (engine, answers) = (self.engine.clone(), self.answers.clone());
        let (stamped, asked) = (pressed.clone(), ids.clone());
        #[cfg(test)]
        let hooks = self.next_call.take();
        let started = std::thread::Builder::new().name("files-trash".into()).spawn(move || {
            #[cfg(test)]
            if let Some((hold, _)) = &hooks {
                let _ = hold.recv();
            }
            // The session of the press is handed over and the engine decides (atomically with the token copy): a
            // check here could not be. A panic must not leave the items in flight for good: it answers as failed.
            let session = stamped.0;
            let items =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(&engine, action, session, &asked)))
                    .unwrap_or_else(|_| {
                        log("files: a move to or from Trash ended early");
                        asked.iter().map(|id| outcome(id, Some(ActionError::Failed))).collect()
                    });
            let _ = answers.send(Answer {
                action,
                pressed: stamped,
                items,
            });
            #[cfg(test)]
            if let Some((_, ended)) = &hooks {
                let _ = ended.send(());
            }
            engine.wake_host();
        });
        if started.is_err() {
            log("files: could not start the worker");
            return Err(ActionError::Failed);
        }
        self.in_flight.extend(ids.into_iter().map(|id| (id, pressed.0)));
        Ok(())
    }

    /// Take in what the workers answered. An answer counts only for the session AND the account of its press.
    fn pump(&mut self) {
        let now = stamp(&self.engine);
        self.look(&now);
        let here = self.engine.has_session();
        while let Ok(answer) = self.inbox.try_recv() {
            // The marks of this press go on every outcome; a mark a later session's press put there stays.
            for item in &answer.items {
                if self.in_flight.get(&item.id) == Some(&answer.pressed.0) {
                    self.in_flight.remove(&item.id);
                }
            }
            // The session's turn only moves forward, so a stamp that is current now was current for the whole call.
            if !(here && answer.pressed == now) {
                log("files: the answer of an earlier session was dropped");
                continue;
            }
            let done = answer.items.iter().filter(|item| item.left.is_none());
            match answer.action {
                Action::Trash => {
                    let done: HashSet<String> = done.map(|item| item.id.clone()).collect();
                    // Added to what is held (the window may still hold rows of an earlier move for Undo); all of
                    // it is this session's, or `look` would have dropped it.
                    match &mut self.undo {
                        Some((_, held)) => held.extend(done),
                        None if !done.is_empty() => self.undo = Some((answer.pressed, done)),
                        None => {}
                    }
                }
                _ => {
                    if let Some((_, held)) = &mut self.undo {
                        for item in done {
                            held.remove(&item.id);
                        }
                    }
                }
            }
            self.changes.push(Change::Trash {
                action: answer.action,
                items: answer.items,
            });
        }
    }

    /// What the moves to and from Trash did since the last call, oldest first.
    pub fn take(&mut self) -> Vec<Change> {
        self.pump();
        std::mem::take(&mut self.changes)
    }
}

fn outcome(id: &str, left: Option<ActionError>) -> TrashOutcome {
    TrashOutcome {
        id: id.to_string(),
        left,
    }
}

/// On a worker: each item in turn, for `session` only. An item this PC's local state refuses (an upload not
/// finished, a protected item) is left, unsent. After a failure that is the call's and not the item's (no session,
/// no connection, a session Beebeeb ended) the items after it are left the same way, unsent.
fn run<C: CredentialStores>(
    engine: &Engine<C>,
    action: Action,
    session: SessionGeneration,
    ids: &[String],
) -> Vec<TrashOutcome> {
    let mut stopped: Option<ActionError> = None;
    ids.iter()
        .map(|id| {
            if let Some(why) = stopped {
                return outcome(id, Some(why));
            }
            let result = match action {
                Action::Trash => match refused_here(engine, id) {
                    Some(why) => return outcome(id, Some(why)),
                    None => engine.trash_items_for(session, &[id.as_str()]),
                },
                _ => engine
                    .restore_items_for(session, &[id.as_str()])
                    .map_err(NotTrashed::Call),
            };
            let left = result.err().map(|refused| {
                let (why, stops) = match refused {
                    NotTrashed::Protected => (ActionError::Protected, false),
                    NotTrashed::Call(AccountCallError::NotSignedIn) => (ActionError::Unavailable, true),
                    NotTrashed::Call(AccountCallError::Offline) => (ActionError::Offline, true),
                    NotTrashed::Call(AccountCallError::SessionEnded) => (ActionError::Failed, true),
                    NotTrashed::Call(_) => (ActionError::Failed, false),
                };
                if stops {
                    stopped = Some(why);
                }
                why
            });
            outcome(id, left)
        })
        .collect()
}

/// Does this PC's local state refuse a move to Trash of this item (it is still uploading, or protected)? A best-
/// effort guard: with no local state for the signed-in account, or no such row, Beebeeb's own checks decide.
fn refused_here<C: CredentialStores>(engine: &Engine<C>, id: &str) -> Option<ActionError> {
    let state = engine.local_state().ok()?;
    state
        .with(|db| {
            let entry = file_ops::resolve(db, id).ok()?;
            file_ops::check_trashable(db, &entry).err().map(map_drive_err)
        })
        .ok()
        .flatten()
}
