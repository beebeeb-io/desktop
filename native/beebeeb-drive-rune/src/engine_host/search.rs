//! Search on the real engine: [`EngineSearchSource`], the `SearchSource` of the running app.
//!
//! What it answers, and from where:
//! - `search`: names in this computer's copy of the list (`desktop_search::build_local_index` over the local state,
//!   then `query_local_index`), any case, narrowed by kind, at most [`SHOWN`] results. Folders are not in the
//!   engine's index, so the Folders pill finds nothing yet; shared items are left out (they are not in File
//!   Explorer either: ruling 1701).
//! - `details`: the facts the index holds for a result (kind, size, changed, where, on this PC or online only).
//! - Not bound: `recent`, `clear_recent`, `remember`. A search term is the person's content and there is no store
//!   for it yet; nothing is kept, in memory or on disk, until the lead rules where it lives.
//!
//! Thread rule: reading the local state is disk work, so the index is built on a worker and kept in memory; a query
//! then answers at once on the window's thread, from memory. The index is built again on a worker when a search
//! finds it older than [`FRESH_FOR`]; the answers come from the one before until the new one lands, and the window
//! is woken when it does.
//!
//! Honest states: until an index of the signed-in account's computer exists (signed out, before the first sync,
//! still building, a failed build) `wired()` is false and `search` answers `Unavailable`: never "no results" for a
//! list it has not read, never the board's sample.
//!
//! Must not: log a query, a name or a path.

use std::sync::{Arc, Mutex, PoisonError, mpsc};
use std::time::{Duration, Instant};

use beebeeb_engine::app::{CredentialStores, Engine};
use beebeeb_engine::desktop_search::{LocalSearchIndex, LocalSearchRecord, build_local_index, query_local_index};
use beebeeb_engine::drive_v2::model::{ROOT_NAME, mime_for_name};
use chrono::Local;

use super::log;
use super::trash::{kind_of, when};
use crate::frames::files::source::Details;
use crate::frames::search::source::{Answer, Hit, Narrow, SearchSource};
use crate::ui::file_list::PcState;

#[cfg(test)]
mod tests;

/// An index this recent is not built again when a search asks.
pub const FRESH_FOR: Duration = Duration::from_secs(30);

/// At most this many results are shown.
pub const SHOWN: usize = 100;

/// What Search needs of the engine: whose computer it is, the local index (disk work: a worker calls it), and a way
/// to wake the window. The running app's is the engine handle; a test's reads a state database of its own.
pub trait SearchHost: Send + Sync + 'static {
    /// The signed-in account (in memory, never waits); `None`: signed out.
    fn account(&self) -> Option<String>;
    /// The index of this computer's local state. `Err`: there is none (nothing synced yet) or it cannot be read.
    fn index(&self) -> Result<LocalSearchIndex, ()>;
    fn wake(&self);
}

impl<C: CredentialStores> SearchHost for Engine<C> {
    fn account(&self) -> Option<String> {
        self.account_email().or_else(|| self.user_id())
    }

    fn index(&self) -> Result<LocalSearchIndex, ()> {
        let db = self.state_db().map_err(|_| ())?;
        build_local_index(&db).map_err(|_| ())
    }

    fn wake(&self) {
        self.wake_host();
    }
}

/// Search on the real engine. See the module's doc.
pub struct EngineSearchSource<H: SearchHost> {
    host: Arc<H>,
    state: Mutex<Inner>,
    answers: mpsc::Sender<Built>,
    inbox: Mutex<mpsc::Receiver<Built>>,
}

#[derive(Default)]
struct Inner {
    /// The last index built, and whose account's computer it is.
    index: Option<(String, Arc<LocalSearchIndex>)>,
    /// When the last build was answered (a failure too).
    built_at: Option<Instant>,
    building: bool,
}

/// A worker's answer: the account it was built for (`None`: it changed during the build) and the index, or none.
struct Built(Option<String>, Result<LocalSearchIndex, ()>);

impl<H: SearchHost> EngineSearchSource<H> {
    /// The first index is built on a worker now, when someone is signed in.
    pub fn new(host: Arc<H>) -> Self {
        let (answers, inbox) = mpsc::channel();
        let source = Self {
            host,
            state: Mutex::new(Inner::default()),
            answers,
            inbox: Mutex::new(inbox),
        };
        drop(source.sync());
        source
    }

    /// Take in a built index, then build again when there is none for this account, or it is old.
    fn sync(&self) -> std::sync::MutexGuard<'_, Inner> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let inbox = self.inbox.lock().unwrap_or_else(PoisonError::into_inner);
        while let Ok(Built(whose, result)) = inbox.try_recv() {
            state.building = false;
            state.built_at = Some(Instant::now());
            match (whose, result) {
                (Some(whose), Ok(index)) => state.index = Some((whose, Arc::new(index))),
                (None, Ok(_)) => state.index = None,
                // Kept: the index before still answers. None before: not wired.
                (_, Err(())) => log("search: the list on this PC could not be read"),
            }
        }
        drop(inbox);
        let whose = self.host.account();
        let stale = state.built_at.is_none_or(|at| at.elapsed() >= FRESH_FOR);
        let other = state.index.as_ref().is_some_and(|(of, _)| Some(of) != whose.as_ref());
        if whose.is_some() && !state.building && (stale || other) {
            self.build(&mut state);
        }
        state
    }

    fn build(&self, state: &mut Inner) {
        let (host, answers) = (self.host.clone(), self.answers.clone());
        let started = std::thread::Builder::new().name("search-index".into()).spawn(move || {
            let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let before = host.account();
                let index = host.index();
                (before.filter(|whose| host.account().as_ref() == Some(whose)), index)
            }));
            let built = built.unwrap_or((None, Err(())));
            let _ = answers.send(Built(built.0, built.1));
            host.wake();
        });
        state.building = started.is_ok();
        if started.is_err() {
            log("search: could not start a thread for the index");
            state.built_at = Some(Instant::now());
        }
    }

    /// The signed-in account's index.
    fn index(&self) -> Option<Arc<LocalSearchIndex>> {
        let state = self.sync();
        let whose = self.host.account()?;
        state
            .index
            .as_ref()
            .filter(|(of, _)| *of == whose)
            .map(|(_, index)| index.clone())
    }
}

impl<H: SearchHost> SearchSource for EngineSearchSource<H> {
    fn wired(&self) -> bool {
        self.index().is_some()
    }

    /// Not bound: no store for search terms yet.
    fn recent(&self) -> Vec<String> {
        Vec::new()
    }

    /// Not bound: nothing is kept.
    fn clear_recent(&mut self) {}

    /// Not bound: the term is not kept.
    fn remember(&mut self, _query: &str) {}

    fn search(&self, query: &str, narrow: Narrow) -> Answer {
        let Some(index) = self.index() else {
            return Answer::Unavailable;
        };
        let hits = query_local_index(&index, query, usize::MAX, true)
            .results
            .into_iter()
            .filter_map(|result| index.records.get(&result.file_id))
            .map(hit_of)
            .filter(|hit| narrow.keeps(hit.kind))
            .take(SHOWN)
            .collect();
        Answer::Found { hits, loading: None }
    }

    fn details(&self, id: &str) -> Option<Details> {
        let index = self.index()?;
        let record = index.records.get(id).filter(|record| !record.shared_with_me)?;
        Some(details_of(record))
    }
}

/// The PC column of a record: online only, or on this PC. Anything else (syncing, an error): not said.
fn pc_of(status: &str) -> Option<PcState> {
    match status {
        "cloud_only" => Some(PcState::OnlineOnly),
        "local" => Some(PcState::Downloaded),
        _ => None,
    }
}

/// Where a record is: its folders, "Documents › Legal"; "Files" at the top.
fn place_of(rel_path: &str) -> String {
    let parts: Vec<&str> = rel_path.split(['/', '\\']).filter(|part| !part.is_empty()).collect();
    match parts.split_last() {
        Some((_, folders)) if !folders.is_empty() => folders.join(" \u{203A} "),
        _ => ROOT_NAME.to_string(),
    }
}

fn hit_of(record: &LocalSearchRecord) -> Hit {
    Hit {
        id: record.file_id.clone(),
        name: record.name.clone(),
        kind: kind_of(false, mime_for_name(&record.name)),
        changed: when(record.modified_at, Local::now()),
        size: crate::source::format_size(record.size_bytes.max(0)),
        place: place_of(&record.rel_path),
        pc: pc_of(&record.status),
    }
}

/// The details panel's facts: only what the index holds. Who it is shared with, its versions and the device that
/// changed it are not known here, and stay empty; whether it is kept offline is not in the index either (`false`).
fn details_of(record: &LocalSearchRecord) -> Details {
    let mime = mime_for_name(&record.name);
    let size = crate::source::format_size(record.size_bytes.max(0));
    let offline_note = match pc_of(&record.status) {
        Some(PcState::OnlineOnly) => format!("Online only now \u{00B7} {size}"),
        Some(_) => format!("Downloaded \u{00B7} {size}"),
        None => size.clone(),
    };
    let item = crate::source::DriveItem {
        id: record.file_id.clone(),
        parent_id: None,
        name: record.name.clone(),
        is_folder: false,
        size: Some(record.size_bytes),
        child_count: None,
        modified_at: String::new(),
        mime: mime.map(str::to_string),
        has_thumbnail: false,
        shared_type: None,
        shared_badges: Vec::new(),
        pc_status: None,
        is_protected: false,
    };
    Details {
        kind: crate::frames::files::source::kind_of(&item).to_string(),
        size,
        changed: when(record.modified_at, Local::now()),
        changed_on: None,
        added: None,
        where_: place_of(&record.rel_path),
        versions: None,
        shared: String::new(),
        offline_note,
        kept_offline: false,
        // No picture of the file is read here: the page preview, never a sample picture.
        picture: false,
    }
}
