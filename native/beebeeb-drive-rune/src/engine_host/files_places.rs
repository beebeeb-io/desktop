//! What the Files window says of the Backups tree on the real engine: [`Places`], the part of `EngineFilesSource`
//! behind `folder_shown`, `protection` and `backup_entry`.
//!
//! - When the window shows a folder it tells the source (`folder_shown`, once per folder and per session); the
//!   source reads, on a worker, the place of that folder and of its rows in the Backups tree
//!   ([`Engine::backup_places_for`]: the app rule is asked there, never re-derived here) and holds that ONE answer.
//! - `protection` and `backup_entry` only look at what is held, on every frame and for every row: nothing is read,
//!   opened or locked in them. Before an answer landed they say nothing; a change the window then sends is refused
//!   by the engine (`DriveError::ReadOnly`), so the answer is the early word, never the enforcement.
//! - Whose: the answer carries the session AND the account it was asked under (both read on the window's thread at
//!   the event, before the worker started). A look under another drops it, here, now; an answer that lands under
//!   another is dropped, as is an answer for a folder that is no longer the one asked.
//!
//! Must not: log a name, a path or an id; answer "outside the tree" because nothing is known yet (nothing known is
//! `None`, and the engine decides); name a device's kind or status it does not know (this PC's is the only one).

use std::sync::{Arc, mpsc};

use beebeeb_engine::app::backup_places_api::{BackupPlace, BackupPlaces};
use beebeeb_engine::app::{CredentialStores, Engine, LocalStateRefused};

use super::log;
use super::stamp::{Stamp, stamp};
use crate::frames::files::protected::{BackupEntry, BackupKind, Protection};
use crate::source::DriveItem;

#[cfg(test)]
#[path = "files_places_tests.rs"]
mod tests;

/// What a worker sends back: the stamp of its event and what the engine said.
struct Answer {
    pressed: Stamp,
    places: Result<BackupPlaces, LocalStateRefused>,
}

/// See the module's doc.
pub struct Places<C: CredentialStores> {
    engine: Arc<Engine<C>>,
    answers: mpsc::Sender<Answer>,
    inbox: mpsc::Receiver<Answer>,
    /// The session and account the last look saw: when they move, everything held goes.
    seen: Option<Stamp>,
    /// The folder the last event asked for (`Some(None)`: the top of Files); only its answer is held.
    asked: Option<Option<String>>,
    /// The one answer held: the folder shown and its rows.
    held: Option<BackupPlaces>,
    /// Tests only: the next read waits on the first, on its worker, before it asks the engine anything, and says on
    /// the second that its answer is in the inbox.
    #[cfg(test)]
    next_call: Option<(mpsc::Receiver<()>, mpsc::Sender<()>)>,
}

impl<C: CredentialStores> Places<C> {
    /// Starts no thread and reads nothing.
    pub fn new(engine: Arc<Engine<C>>) -> Self {
        let (answers, inbox) = mpsc::channel();
        Self {
            engine,
            answers,
            inbox,
            seen: None,
            asked: None,
            held: None,
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
        self.held = None;
    }

    /// The window shows `folder` now: read the places of it and of its rows.
    pub fn shown(&mut self, folder: Option<&str>) {
        let pressed = stamp(&self.engine);
        self.look(&pressed);
        self.held = None;
        self.asked = None;
        if !self.engine.has_session() {
            return;
        }
        let (engine, answers) = (self.engine.clone(), self.answers.clone());
        let folder_id = folder.map(str::to_string);
        #[cfg(test)]
        let hooks = self.next_call.take();
        let started = std::thread::Builder::new().name("files-places".into()).spawn(move || {
            #[cfg(test)]
            if let Some((hold, _)) = &hooks {
                let _ = hold.recv();
            }
            // The press's session is handed to the engine, which refuses a read for another one.
            let places = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                engine.backup_places_for(pressed.0, folder_id.as_deref())
            }))
            .unwrap_or_else(|_| {
                log("files: a read of the Backups places ended early");
                Err(LocalStateRefused::Unreadable)
            });
            let _ = answers.send(Answer { pressed, places });
            #[cfg(test)]
            if let Some((_, ended)) = &hooks {
                let _ = ended.send(());
            }
            engine.wake_host();
        });
        if started.is_err() {
            log("files: could not start the read of the Backups places");
            return;
        }
        self.asked = Some(folder.map(str::to_string));
    }

    /// Take in what the worker answered. An answer counts only for the session AND the account of its event, and only
    /// for the folder asked last.
    pub fn pump(&mut self) {
        let now = stamp(&self.engine);
        self.look(&now);
        let here = self.engine.has_session();
        while let Ok(answer) = self.inbox.try_recv() {
            if !(here && answer.pressed == now) {
                log("files: the answer of an earlier session was dropped");
                continue;
            }
            match answer.places {
                Ok(places) if self.asked.as_ref() == Some(&places.folder) => {
                    self.held = Some(places);
                }
                // A read that failed holds nothing: `protection` says nothing and the engine refuses a change.
                _ => {}
            }
        }
    }

    /// What is held, only while it is the session's and the account's that are here now.
    fn current(&self) -> Option<&BackupPlaces> {
        let now = stamp(&self.engine);
        self.held
            .as_ref()
            .filter(|_| self.engine.has_session() && self.seen.as_ref() == Some(&now))
    }

    /// Why `item` (with no item: anything in `folder`) cannot be renamed, moved or created in.
    pub fn protection(&self, folder: Option<&str>, item: Option<&DriveItem>) -> Option<Protection> {
        let held = self.current()?;
        let place = match item {
            Some(item) => match held.rows.iter().find(|(id, _)| *id == item.id) {
                Some((_, place)) => place.clone(),
                // A row the read does not hold, in the folder it read (the list was read again after a sync): it
                // takes the folder's own place. Never "outside" inside the tree.
                None if item.parent_id.as_deref() == held.folder.as_deref() => held.of_folder.clone(),
                None => return None,
            },
            None if folder == held.folder.as_deref() => held.of_folder.clone(),
            // A drop on a row: the row is the destination.
            None => held.rows.iter().find(|(id, _)| Some(id.as_str()) == folder)?.1.clone(),
        };
        match place {
            BackupPlace::Outside => None,
            BackupPlace::Root => Some(Protection::BackupsRoot),
            BackupPlace::Device { id, .. } => Some(Protection::DeviceBackup(id)),
            BackupPlace::Inside { device, .. } => Some(Protection::DeviceBackup(device)),
        }
    }

    /// This PC's own backup says so; another device's kind and status are not known here (they live in that
    /// device's manifest, which is not read), so every other row draws as the folder it is.
    pub fn backup_entry(&self, item: &DriveItem) -> Option<BackupEntry> {
        let held = self.current()?;
        match held.rows.iter().find(|(id, _)| *id == item.id) {
            Some((_, BackupPlace::Device { this_pc: true, .. })) => Some(BackupEntry {
                kind: BackupKind::Laptop,
                status: Some("This PC".into()),
                managed: false,
                note: None,
            }),
            _ => None,
        }
    }
}
