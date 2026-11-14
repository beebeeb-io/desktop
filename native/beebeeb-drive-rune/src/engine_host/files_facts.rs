//! What the Files window says beside its list, on the real engine: [`Facts`], the part of `EngineFilesSource`
//! behind `counts`, `storage_split`, `sort_of`, `set_sort` and `changed_label`.
//!
//! - `counts`: files kept on this PC, from the signed-in account's local state ([`Engine::files_facts`]). The two
//!   other counts (shares that wait for an answer, items in Trash) are not known here: the engine has no count
//!   call for either, and counting means listing and decrypting each from Beebeeb, which a badge must not do.
//!   They are 0, and the sidebar draws no badge for 0.
//! - `storage_split`: the bytes of files and of photos and videos outside Backups, from the same read.
//! - `sort_of` / `set_sort`: each folder's sort, kept here in memory for the session (the engine has no setting
//!   for it): a folder keeps its sort for as long as the account is signed in and the app runs.
//! - `changed_label`: "Just now" for an item changed within the last minute, from the row's own time.
//!
//! When it reads: never from a draw. `counts` and `storage_split` only hand out what is held and note that it is
//! wanted; [`Facts::pump`] (the window's poll, through `FilesSource::changes`) starts the read, on a worker: the
//! first time it is wanted in a session, after a change this source made was answered, and when the engine's sync
//! word changes (something synced). A read that fails keeps nothing and is tried again at the next of those.
//!
//! Whose: what is held carries the session AND the account it was read under. A look under another drops it, the
//! sorts with it; an answer that lands under another is dropped.
//!
//! Must not: log a name, an id or a figure.

use std::cell::Cell;
use std::collections::HashMap;
use std::sync::{Arc, mpsc};

use beebeeb_engine::app::files_facts_api::FilesFacts;
use beebeeb_engine::app::{CredentialStores, Engine};
use chrono::{DateTime, Utc};

use super::stamp::{Stamp, stamp};
use super::{log, sync};
use crate::frames::files::Sort;
use crate::frames::files::source::{Counts, StorageSplit};
use crate::source::DriveItem;

#[cfg(test)]
#[path = "files_facts_tests.rs"]
mod tests;

/// An item changed this recently reads "Just now".
const JUST_NOW_SECONDS: i64 = 60;

/// See the module's doc.
pub struct Facts<C: CredentialStores> {
    engine: Arc<Engine<C>>,
    answers: mpsc::Sender<(Stamp, Option<FilesFacts>)>,
    inbox: mpsc::Receiver<(Stamp, Option<FilesFacts>)>,
    /// The session and account the last look saw: when they move, everything held goes.
    seen: Option<Stamp>,
    held: Option<FilesFacts>,
    /// A draw asked for a figure since the last pump.
    wanted: Cell<bool>,
    /// A read is on its way.
    reading: bool,
    /// What is held is older than something that happened: read again.
    old: bool,
    /// The engine's sync word at the last pump.
    word: Option<&'static str>,
    /// Each folder's sort (`None`: the root), for this session.
    sorts: HashMap<Option<String>, Sort>,
    /// Test builds only: the next read waits on its worker for the first before it asks the engine anything, and
    /// says on the second that its answer is in the inbox.
    #[cfg(test)]
    next_read: Option<(mpsc::Receiver<()>, mpsc::Sender<()>)>,
}

impl<C: CredentialStores> Facts<C> {
    /// Starts no thread and reads nothing.
    pub fn new(engine: Arc<Engine<C>>) -> Self {
        let (answers, inbox) = mpsc::channel();
        Self {
            engine,
            answers,
            inbox,
            seen: None,
            held: None,
            wanted: Cell::new(false),
            reading: false,
            old: true,
            word: None,
            sorts: HashMap::new(),
            #[cfg(test)]
            next_read: None,
        }
    }

    /// What is held, if it is the session's and the account's that are here now.
    fn current(&self) -> Option<&FilesFacts> {
        self.wanted.set(true);
        let now = stamp(&self.engine);
        self.held
            .as_ref()
            .filter(|facts| self.engine.has_session() && self.seen.as_ref() == Some(&now) && facts.session == now.0)
    }

    pub fn counts(&self) -> Option<Counts> {
        self.current().map(|facts| Counts {
            shared_waiting: 0,
            trash: 0,
            available_offline: facts.available_offline,
        })
    }

    pub fn storage_split(&self) -> Option<StorageSplit> {
        self.current().map(|facts| StorageSplit {
            files_bytes: facts.files_bytes,
            photos_bytes: facts.photos_bytes,
        })
    }

    pub fn sort_of(&self, folder: Option<&str>) -> Option<Sort> {
        if !self.engine.has_session() || self.seen.as_ref() != Some(&stamp(&self.engine)) {
            return None;
        }
        self.sorts.get(&folder.map(str::to_string)).copied()
    }

    pub fn set_sort(&mut self, folder: Option<&str>, sort: Sort) {
        let now = stamp(&self.engine);
        self.look(&now);
        if self.engine.has_session() {
            self.sorts.insert(folder.map(str::to_string), sort);
        }
    }

    /// The session or the account is not the one the last look saw: nothing of the earlier one is kept.
    fn look(&mut self, now: &Stamp) {
        if self.seen.as_ref() == Some(now) {
            return;
        }
        self.seen = Some(now.clone());
        self.held = None;
        self.sorts.clear();
        self.old = true;
        // A read that is on its way is the earlier session's: its answer will be dropped.
        self.reading = false;
    }

    /// The window's poll. `changed`: a change this source made was answered since the last call. `true`: a figure
    /// the window draws may have changed.
    pub fn pump(&mut self, changed: bool) -> bool {
        let now = stamp(&self.engine);
        self.look(&now);
        let here = self.engine.has_session();
        let mut drawn = false;
        while let Ok((read_under, facts)) = self.inbox.try_recv() {
            // Only for the session AND the account it was started under, and only while they are here.
            if !(here && read_under == now) {
                continue;
            }
            self.reading = false;
            match facts {
                Some(facts) if facts.session == now.0 => {
                    drawn |= self.held != Some(facts);
                    self.held = Some(facts);
                }
                // Not read: nothing is kept, and the next change or sync asks again.
                _ => {
                    drawn |= self.held.take().is_some();
                }
            }
        }
        let word = sync::sync_word(&self.engine.view());
        if changed || word != self.word {
            self.old = true;
        }
        self.word = word;
        if here && self.old && !self.reading && self.wanted.replace(false) {
            self.read(now);
        }
        drawn
    }

    fn read(&mut self, read_under: Stamp) {
        let (engine, answers) = (self.engine.clone(), self.answers.clone());
        #[cfg(test)]
        let hook = self.next_read.take();
        let started = std::thread::Builder::new().name("files-facts".into()).spawn(move || {
            #[cfg(test)]
            let ended = hook.map(|(hold, ended)| {
                let _ = hold.recv_timeout(std::time::Duration::from_secs(30));
                ended
            });
            let facts =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| engine.files_facts().ok())).unwrap_or(None);
            let _ = answers.send((read_under, facts));
            #[cfg(test)]
            if let Some(ended) = ended {
                let _ = ended.send(());
            }
            engine.wake_host();
        });
        if started.is_err() {
            log("files: could not start the read of the sidebar's figures");
            return;
        }
        self.reading = true;
        self.old = false;
    }

    /// Is a read on its way?
    #[cfg(test)]
    fn busy(&self) -> bool {
        self.reading
    }
}

/// "Just now" for an item changed within the last minute of `now`; the list's own words otherwise.
pub fn changed_label(item: &DriveItem, now: DateTime<Utc>) -> Option<String> {
    let changed = DateTime::parse_from_rfc3339(&item.modified_at).ok()?;
    let ago = now.signed_duration_since(changed).num_seconds();
    (0..JUST_NOW_SECONDS).contains(&ago).then(|| "Just now".to_string())
}
