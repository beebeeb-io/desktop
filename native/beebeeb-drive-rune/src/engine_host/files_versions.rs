//! The Files window's Version history on the real engine: [`VersionCalls`], the part of `EngineFilesSource` behind
//! `ask_versions`, `versions_state`, `versions` and `restore_version`.
//!
//! - A file's versions are read from Beebeeb ([`Engine::file_versions_for`]) when the Versions or the Details dialog
//!   of it opens, on a worker, and held for [`FRESH_FOR`]; a read that failed is not held as fresh, so the next
//!   opening asks again. Nothing is drawn before the engine answered: until then `versions` is empty and the
//!   state says it is loading.
//! - A restore ([`Engine::restore_file_version_for`]) goes out once per press, for the version of the held list
//!   that is not the current one; while one is on its way, a second press of that file sends nothing and says so.
//!   After ANY answer of a restore the list is read again, on the same worker, before anything is said; a restore
//!   whose answer was lost is called failed only if the list shows it was not made.
//! - Every call is bound to the session AND the account of its press: both are read on the window's thread at the
//!   press, before a worker starts, and the engine sends nothing unless the session in memory is still that one.
//!   An answer that lands under another session or account is dropped: nothing of it is held or drawn.
//!
//! Thread rule: nothing here blocks the window. Each ask has a worker of its own; it sends its answer down one
//! channel and wakes the window ([`Engine::wake_host`]).
//!
//! Must not: log a name, a path or an id; draw a version Beebeeb did not list; repeat a restore (each one mints a
//! version).

use std::collections::HashMap;
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use beebeeb_engine::account::SessionGeneration;
use beebeeb_engine::app::settings_api::AccountCallError;
use beebeeb_engine::app::versions_api::VersionsRefused;
use beebeeb_engine::app::{CredentialStores, Engine};
use beebeeb_engine::drive_v2::dto::VersionEntry;

use super::log;
use super::stamp::{Stamp, stamp};
use crate::frames::files::actions_data::Version;
use crate::frames::files::source::{Action, ActionError, Change, Read};
use crate::source::DriveItem;

#[cfg(test)]
#[path = "files_versions_tests.rs"]
mod tests;

/// A list read this recently is not read again when a dialog opens.
const FRESH_FOR: Duration = Duration::from_secs(30);
/// How many files' lists are held at once (the oldest asked is dropped).
const HELD: usize = 8;

/// What is held of one file.
#[derive(Clone, Debug)]
enum Slot {
    /// Asked, not answered.
    Loading,
    /// Read at that time.
    Known(Vec<Version>, Instant),
    /// Tried, did not work.
    Failed(ActionError),
}

/// What a worker sends back: the stamp of its press, the file, what the restore said (when one was asked) with the
/// id of the current version at the press, and the list as it read it.
struct Answer {
    pressed: Stamp,
    id: String,
    restore: Option<(Result<(), VersionsRefused>, Option<String>)>,
    list: Result<Vec<Version>, ActionError>,
}

/// See the module's doc.
pub struct VersionCalls<C: CredentialStores> {
    engine: Arc<Engine<C>>,
    answers: mpsc::Sender<Answer>,
    inbox: mpsc::Receiver<Answer>,
    /// The session and account the last look saw: when they move, everything held goes.
    seen: Option<Stamp>,
    /// Per file, the oldest asked first.
    files: Vec<(String, Slot)>,
    /// The files a restore is on its way for, each with the session of its press.
    restoring: HashMap<String, SessionGeneration>,
    /// What Beebeeb answered and the window has not taken yet.
    changes: Vec<Change>,
    /// Tests only: the next call waits on the first, on its worker, before it asks the engine anything, and says on
    /// the second that its answer is in the inbox.
    #[cfg(test)]
    next_call: Option<(mpsc::Receiver<()>, mpsc::Sender<()>)>,
}

impl<C: CredentialStores> VersionCalls<C> {
    /// Starts no thread and reads nothing.
    pub fn new(engine: Arc<Engine<C>>) -> Self {
        let (answers, inbox) = mpsc::channel();
        Self {
            engine,
            answers,
            inbox,
            seen: None,
            files: Vec::new(),
            restoring: HashMap::new(),
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
        self.files.clear();
        self.restoring.clear();
        self.changes.clear();
    }

    fn slot(&self, id: &str) -> Option<&Slot> {
        self.files.iter().find(|(held, _)| held == id).map(|(_, slot)| slot)
    }

    fn set(&mut self, id: &str, slot: Slot) {
        match self.files.iter_mut().find(|(held, _)| held == id) {
            Some((_, held)) => *held = slot,
            None => {
                if self.files.len() >= HELD {
                    self.files.remove(0);
                }
                self.files.push((id.to_string(), slot));
            }
        }
    }

    /// The dialog of this file opened: read its versions unless a read is on its way or the list is fresh.
    pub fn ask(&mut self, item: &DriveItem) {
        let pressed = stamp(&self.engine);
        self.look(&pressed);
        if !self.engine.has_session() || item.is_folder {
            return;
        }
        match self.slot(&item.id) {
            Some(Slot::Loading) => return,
            Some(Slot::Known(_, at)) if at.elapsed() < FRESH_FOR => return,
            _ => {}
        }
        if self.start(&item.id, pressed, None) {
            self.set(&item.id, Slot::Loading);
        } else {
            self.set(&item.id, Slot::Failed(ActionError::Failed));
        }
    }

    /// The list, only while it is the session's and the account's that are here now.
    pub fn versions(&self, item: &DriveItem) -> Vec<Version> {
        if !self.engine.has_session() || self.seen.as_ref() != Some(&stamp(&self.engine)) {
            return Vec::new();
        }
        match self.slot(&item.id) {
            Some(Slot::Known(list, _)) => list.clone(),
            _ => Vec::new(),
        }
    }

    pub fn state(&self, item: &DriveItem) -> Read {
        if !self.engine.has_session() || self.seen.as_ref() != Some(&stamp(&self.engine)) {
            return Read::Loading;
        }
        match self.slot(&item.id) {
            Some(Slot::Failed(why)) => Read::Failed(*why),
            // Known and empty, or not asked yet: nothing to show, and nothing is wrong that can be said.
            Some(Slot::Known(..)) => Read::Answered,
            Some(Slot::Loading) | None => Read::Loading,
        }
    }

    /// Make this version the current one. `Ok`: sent; the list shows it once it is read again. Refused, with nothing
    /// sent: no session, a restore of this file on its way, or a version that is not a non-current one of the list.
    pub fn restore(&mut self, item: &DriveItem, version: &str) -> Result<(), ActionError> {
        let pressed = stamp(&self.engine);
        self.look(&pressed);
        if !self.engine.has_session() {
            return Err(ActionError::Unavailable);
        }
        if self.restoring.contains_key(&item.id) {
            return Err(ActionError::InFlight);
        }
        let Some(Slot::Known(list, _)) = self.slot(&item.id) else {
            return Err(ActionError::Unavailable);
        };
        if !list.iter().any(|held| held.id == version && !held.current) {
            return Err(ActionError::Unavailable);
        }
        let held_current = list.iter().find(|held| held.current).map(|held| held.id.clone());
        if !self.start(&item.id, pressed.clone(), Some((version.to_string(), held_current))) {
            return Err(ActionError::Failed);
        }
        self.restoring.insert(item.id.clone(), pressed.0);
        Ok(())
    }

    /// Starts the worker of one ask. `restore`: the version to restore and the current id at the press.
    fn start(&mut self, id: &str, pressed: Stamp, restore: Option<(String, Option<String>)>) -> bool {
        let (engine, answers) = (self.engine.clone(), self.answers.clone());
        let id = id.to_string();
        #[cfg(test)]
        let hooks = self.next_call.take();
        let started = std::thread::Builder::new()
            .name("files-versions".into())
            .spawn(move || {
                #[cfg(test)]
                if let Some((hold, _)) = &hooks {
                    let _ = hold.recv();
                }
                // The session of the press is handed over and the engine decides (atomically with the token copy). A
                // panic must not leave a restore in flight for good: it answers as failed.
                let session = pressed.0;
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let restored = restore
                        .as_ref()
                        .map(|(version, held)| (engine.restore_file_version_for(session, &id, version), held.clone()));
                    let list = read(&engine, session, &id);
                    (restored, list)
                }));
                let (restore, list) = outcome.unwrap_or_else(|_| {
                    log("files: a read of versions ended early");
                    (
                        restore.map(|(_, held)| (Err(VersionsRefused::Unconfirmed), held)),
                        Err(ActionError::Failed),
                    )
                });
                let _ = answers.send(Answer {
                    pressed,
                    id,
                    restore,
                    list,
                });
                #[cfg(test)]
                if let Some((_, ended)) = &hooks {
                    let _ = ended.send(());
                }
                engine.wake_host();
            });
        if started.is_err() {
            log("files: could not start the worker");
        }
        started.is_ok()
    }

    /// Take in what the workers answered. An answer counts only for the session AND the account of its press.
    fn pump(&mut self) {
        let now = stamp(&self.engine);
        self.look(&now);
        let here = self.engine.has_session();
        while let Ok(answer) = self.inbox.try_recv() {
            // The mark of this press goes on every outcome; a mark a later session's press put there stays.
            if self.restoring.get(&answer.id) == Some(&answer.pressed.0) {
                self.restoring.remove(&answer.id);
            }
            // The session's turn only moves forward, so a stamp that is current now was current for the whole call.
            if !(here && answer.pressed == now) {
                log("files: the answer of an earlier session was dropped");
                continue;
            }
            let made_now = match &answer.list {
                Ok(list) => list.iter().find(|held| held.current).map(|held| held.id.clone()),
                Err(_) => None,
            };
            let slot = match answer.list {
                Ok(list) => Slot::Known(list, Instant::now()),
                // A failed read never keeps the older list as fresh.
                Err(why) => Slot::Failed(why),
            };
            self.set(&answer.id, slot);
            if let Some((restore, held_current)) = answer.restore {
                let why = match restore {
                    Ok(()) => None,
                    // The request went out and no answer says whether it was made: the list decides.
                    Err(VersionsRefused::Unconfirmed) => {
                        let made = made_now.is_some() && made_now != held_current;
                        (!made).then_some(ActionError::Failed)
                    }
                    Err(VersionsRefused::NotSignedIn | VersionsRefused::NoLocalState) => Some(ActionError::Unavailable),
                    Err(VersionsRefused::Call(AccountCallError::Offline)) => Some(ActionError::Offline),
                    Err(_) => Some(ActionError::Failed),
                };
                if let Some(why) = why {
                    self.changes.push(Change::Failed(Action::RestoreVersion, why));
                }
            }
        }
    }

    /// What the restores said since the last call, oldest first: only what failed (a restore that was made shows in
    /// the list).
    pub fn take(&mut self) -> Vec<Change> {
        self.pump();
        std::mem::take(&mut self.changes)
    }
}

/// On a worker: the file's versions, for `session` only, as the window shows them.
fn read<C: CredentialStores>(
    engine: &Engine<C>,
    session: SessionGeneration,
    id: &str,
) -> Result<Vec<Version>, ActionError> {
    match engine.file_versions_for(session, id) {
        Ok(listed) => Ok(listed.versions.iter().map(shown).collect()),
        Err(VersionsRefused::NotSignedIn | VersionsRefused::NoLocalState) => Err(ActionError::Unavailable),
        Err(VersionsRefused::Call(AccountCallError::Offline)) => Err(ActionError::Offline),
        Err(_) => Err(ActionError::Failed),
    }
}

/// A version as the dialog draws it. The server names no device, so none is invented.
fn shown(entry: &VersionEntry) -> Version {
    Version {
        id: entry.id.clone(),
        when: crate::ui::file_list::format_date(&entry.created_at),
        size: crate::source::format_size(entry.size),
        current: entry.current,
        changed_on: None,
    }
}
