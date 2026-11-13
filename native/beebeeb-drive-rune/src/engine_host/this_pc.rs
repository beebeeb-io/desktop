//! This PC on the real engine: [`EngineThisPcSource`], the `ThisPcSource` of the running app (Sync history,
//! DW-1301 to DW-1305 and DW-0428; Available offline, DW-0715, DW-1306, DW-0120).
//!
//! What it answers, and from where (all of it this PC's own local state, nothing asked of Beebeeb):
//! - The history: the finished uploads and downloads (`StateDb::list_recent_transfer_activity`), the files moved
//!   to Trash (`status_views::file_overview_activity`), and the changes that did not sync, at the time they were
//!   queued; the last 30 days, newest first. A device folder's manifest in Backups never shows (LD-0113).
//! - The items that did not sync: the queue's problems (`drive_v2::status::sync_problems`), each with the engine's
//!   own plain words for why. Try again: the queue's `StateDb::retry_unsent_change`, the 0.8 app's rule.
//! - What is kept: the top-most kept items (`drive_v2::status::offline_entries`). Remove from this PC: the pin
//!   change (`Engine::set_kept`), refused while sync does not run.
//!
//! Thread rule: nothing here waits on the window's thread. One worker per source does every read and every change,
//! in the order asked; the window reads the last answer from memory. A read is asked when the last one is older
//! than [`FRESH_FOR`] and none is waiting.
//!
//! Must not: show a name while the vault is locked or no one is signed in (the lists are empty then, as the 0.8 app's
//! `shown_transfers` rule); log a name, a path or an id; show a value it has not read.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError, mpsc};
use std::time::{Duration, Instant};

use beebeeb_engine::app::{CredentialStores, Engine};
use beebeeb_engine::backup_protect::BackupsTree;
use beebeeb_engine::drive_v2::status::{self, SyncProblem};
use beebeeb_engine::drive_v2::{dto::DriveItem, model};
use beebeeb_engine::signin::gate::GateState;
use beebeeb_engine::state_db::{FileEntry, FileStatus, PinState, StateDb};
use beebeeb_engine::status_views;
use chrono::{NaiveDate, TimeZone};

use super::log;
use crate::frames::this_pc::source::{Change, Contains, Kept, KeptStatus, Kind, Origin, Reason, ThisPcSource, What};

#[cfg(test)]
#[path = "this_pc_tests.rs"]
mod tests;

/// Whether the section draws this source in a real build. `false` until the window around it is honest: the This
/// PC chrome (`frames/this_pc/chrome.rs`) still draws the board's sample account name, email, initials and a fixed
/// Files/Photos split, which would sit beside a real account (PR body). Flip it with that fix; every method below
/// answers from the engine already.
pub const TURNED_ON: bool = false;

/// An answer read this recently is not read again when the window asks.
pub const FRESH_FOR: Duration = Duration::from_secs(5);
/// How far back the history goes (the board: "kept on this PC only", 30 days).
const HISTORY_DAYS: i64 = 30;
/// How many queued changes are read (the 0.8 app's Sync history reads as many).
const QUEUE_LIMIT: u32 = 500;
/// How many finished transfers and local events are read: everything the state database keeps of them.
const TRANSFERS: usize = 100;
const EVENTS: usize = 200;

/// What the adapter asks of the engine handle. The app's is `Engine`; a test's is a state database in a folder of
/// its own.
pub trait ThisPcEngine: Send + Sync + 'static {
    /// Someone is signed in and the vault is open. Never waits.
    fn unlocked(&self) -> bool;
    /// The local state database. `Err`: nothing synced on this computer yet, or it could not be opened.
    fn state_db(&self) -> Result<StateDb, String>;
    /// Sync runs for this handle. May wait: a worker's.
    fn syncing(&self) -> bool;
    /// Keep an item on this PC, or stop keeping it. Blocks: a worker's.
    fn set_kept(&self, file_id: &str, kept: bool) -> Result<(), String>;
}

impl<C: CredentialStores> ThisPcEngine for Engine<C> {
    fn unlocked(&self) -> bool {
        self.view().gate == GateState::Ready
    }
    fn state_db(&self) -> Result<StateDb, String> {
        Engine::state_db(self)
    }
    fn syncing(&self) -> bool {
        self.is_syncing()
    }
    fn set_kept(&self, file_id: &str, kept: bool) -> Result<(), String> {
        Engine::set_kept(self, file_id, kept)
    }
}

/// What the lists hold, as last read.
#[derive(Debug, Default, Clone, PartialEq)]
pub(crate) struct Snapshot {
    pub history: Vec<Change>,
    pub problems: Vec<Change>,
    /// The queue ids Try again applies to.
    pub retryable: Vec<String>,
    pub kept: Vec<Kept>,
}

enum Job {
    Read,
    Retry(Vec<String>),
    Release(Vec<String>),
}

/// What the worker and the window share.
#[derive(Default)]
struct Shared {
    snapshot: Mutex<Snapshot>,
    read_at: Mutex<Option<Instant>>,
    /// A read is waiting in the queue: no second one is asked.
    read_asked: AtomicBool,
    /// Jobs asked and not finished.
    pending: AtomicUsize,
}

/// This PC on the real engine. See the module's doc.
pub struct EngineThisPcSource<E: ThisPcEngine> {
    engine: Arc<E>,
    shared: Arc<Shared>,
    jobs: Mutex<Option<mpsc::Sender<Job>>>,
}

impl<E: ThisPcEngine> EngineThisPcSource<E> {
    /// The source of the running app. The first read is asked now, so the section has its lists when it opens.
    pub fn new(engine: Arc<E>) -> Self {
        let source = Self {
            engine,
            shared: Arc::default(),
            jobs: Mutex::new(None),
        };
        source.ask_read();
        source
    }

    /// No job is waiting or running (tests wait on it).
    #[cfg(test)]
    pub(crate) fn idle(&self) -> bool {
        self.shared.pending.load(Ordering::SeqCst) == 0
    }

    fn send(&self, job: Job) -> bool {
        let mut jobs = self.jobs.lock().unwrap_or_else(PoisonError::into_inner);
        if jobs.is_none() {
            let (tx, rx) = mpsc::channel::<Job>();
            let (engine, shared) = (self.engine.clone(), self.shared.clone());
            let started = std::thread::Builder::new().name("this-pc".into()).spawn(move || {
                // Ends when the source is dropped (its sender with it).
                for job in rx {
                    let done = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(&*engine, &shared, job)));
                    if done.is_err() {
                        log("this pc: a read or a change ended early");
                        shared.read_asked.store(false, Ordering::SeqCst);
                    }
                    shared.pending.fetch_sub(1, Ordering::SeqCst);
                }
            });
            match started {
                Ok(_) => *jobs = Some(tx),
                Err(_) => {
                    log("this pc: could not start its worker");
                    return false;
                }
            }
        }
        self.shared.pending.fetch_add(1, Ordering::SeqCst);
        let sent = jobs.as_ref().is_some_and(|tx| tx.send(job).is_ok());
        if !sent {
            self.shared.pending.fetch_sub(1, Ordering::SeqCst);
        }
        sent
    }

    fn ask_read(&self) {
        if self.shared.read_asked.swap(true, Ordering::SeqCst) {
            return;
        }
        if !self.send(Job::Read) {
            self.shared.read_asked.store(false, Ordering::SeqCst);
        }
    }

    /// The last answer, after asking for a new one when it is stale. Empty while the vault is not open.
    fn snapshot<T>(&self, part: impl FnOnce(&Snapshot) -> T, empty: T) -> T {
        if !self.engine.unlocked() {
            return empty;
        }
        let stale = self
            .shared
            .read_at
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_none_or(|at| at.elapsed() >= FRESH_FOR);
        if stale {
            self.ask_read();
        }
        part(&self.shared.snapshot.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

/// One job, on the worker. Every change is followed by a read, so the lists show what the engine has after it.
fn run<E: ThisPcEngine>(engine: &E, shared: &Shared, job: Job) {
    match job {
        Job::Read => {
            shared.read_asked.store(false, Ordering::SeqCst);
        }
        Job::Retry(ids) => match engine.state_db() {
            Ok(db) => {
                let now = chrono::Utc::now().timestamp();
                let failed = ids.iter().filter(|id| db.retry_unsent_change(id, now).is_err()).count();
                if failed > 0 {
                    log(format!(
                        "this pc: {failed} of {} changes could not be tried again",
                        ids.len()
                    ));
                }
            }
            Err(_) => log("this pc: nothing to try again (no local state)"),
        },
        Job::Release(ids) => {
            if !engine.syncing() {
                log("this pc: not removed from this PC: sync does not run");
            } else {
                let failed = ids.iter().filter(|id| engine.set_kept(id, false).is_err()).count();
                if failed > 0 {
                    log(format!(
                        "this pc: {failed} of {} items were not removed from this PC",
                        ids.len()
                    ));
                }
            }
        }
    }
    read_into(engine, shared);
}

fn read_into<E: ThisPcEngine>(engine: &E, shared: &Shared) {
    let answer = if !engine.unlocked() {
        // Locked or signed out: nothing with a name stays in memory.
        Some(Snapshot::default())
    } else {
        match engine
            .state_db()
            .and_then(|db| read(&db, chrono::Utc::now().timestamp()))
        {
            Ok(snapshot) => Some(snapshot),
            Err(_) => {
                // The last answer stays (before the first one: nothing): never an empty list in place of one that
                // could not be read. Nothing synced here yet reads as nothing too.
                log("this pc: the local history could not be read");
                None
            }
        }
    };
    if let Some(snapshot) = answer {
        *shared.snapshot.lock().unwrap_or_else(PoisonError::into_inner) = snapshot;
    }
    *shared.read_at.lock().unwrap_or_else(PoisonError::into_inner) = Some(Instant::now());
}

/// Everything the two sections show, from the local state database at `now` (Unix seconds).
pub(crate) fn read(db: &StateDb, now: i64) -> Result<Snapshot, String> {
    let backups = BackupsTree::read(db);
    let listable = |rel: Option<&str>| rel.is_none_or(|rel| !backups.is_hidden(rel));
    let since = now - HISTORY_DAYS * 86_400;

    let queue = db.unsent_changes(0, QUEUE_LIMIT, now).map_err(|_| "queue")?.items;
    let found = status::sync_problems(&queue);
    let retryable = found.iter().filter(|p| p.retryable).map(|p| p.id.clone()).collect();
    let mut problems: Vec<(i64, Change)> = found
        .iter()
        .map(|p| {
            let at = queued_at(&queue, &p.id);
            (at, problem(p, at))
        })
        .collect();
    problems.sort_by_key(|(at, _)| std::cmp::Reverse(*at));

    let mut history: Vec<(i64, Change)> = Vec::new();
    for t in db.list_recent_transfer_activity(TRANSFERS).map_err(|_| "transfers")? {
        if t.occurred_at < since || !listable(t.rel_path.as_deref()) {
            continue;
        }
        let place = place_of(t.rel_path.as_deref());
        let what = match t.direction.as_str() {
            "up" => What::Uploaded { place },
            "down" => What::Downloaded { place },
            _ => continue,
        };
        history.push((
            t.occurred_at,
            change(t.file_id, &t.file_name, what, t.occurred_at, history.len()),
        ));
    }
    for event in status_views::file_overview_activity(db, EVENTS)? {
        // Restored and kept-a-copy events have no line in the board's history yet (PR body): left out, never
        // drawn as something else.
        if event.event_type != "moved_to_trash" || event.occurred_at < since {
            continue;
        }
        let what = What::MovedToTrash {
            place: place_of(event.rel_path.as_deref()),
        };
        let n = history.len();
        history.push((
            event.occurred_at,
            change(None, &event.file_name, what, event.occurred_at, n),
        ));
    }
    // A change that did not sync is in the history too, at the time it was queued: Try again reaches it from both.
    history.extend(problems.iter().filter(|(at, _)| *at >= since || *at == 0).cloned());
    history.sort_by_key(|(at, _)| std::cmp::Reverse(*at));

    Ok(Snapshot {
        history: history.into_iter().map(|(_, c)| c).collect(),
        problems: problems.into_iter().map(|(_, c)| c).collect(),
        retryable,
        kept: kept(db, &backups)?,
    })
}

fn queued_at(queue: &[beebeeb_engine::state_db::UnsentChange], id: &str) -> i64 {
    queue.iter().find(|c| c.id == id).map_or(0, |c| c.queued_at)
}

/// A problem row. `at` 0: the queue did not record when (its time stays empty, its day is today's).
fn problem(p: &SyncProblem, at: i64) -> Change {
    let (date, time) = when(at);
    Change {
        id: p.file_id.clone().unwrap_or_else(|| format!("change:{}", p.id)),
        name: p.name.clone(),
        kind: kind_of(&p.name, false),
        what: What::Failed {
            problem: p.id.clone(),
            reason: Reason::Said(p.reason),
        },
        origin: Origin::ThisPc,
        date,
        time,
    }
}

fn change(file_id: Option<String>, name: &str, what: What, at: i64, n: usize) -> Change {
    let (date, time) = when(at);
    Change {
        id: file_id.unwrap_or_else(|| format!("event:{n}")),
        name: name.to_string(),
        kind: kind_of(name, false),
        what,
        origin: Origin::ThisPc,
        date,
        time,
    }
}

/// The day and the 24-hour time in this PC's time zone. 0 (not recorded): today, no time.
fn when(at: i64) -> (NaiveDate, String) {
    match chrono::Local.timestamp_opt(at, 0).single().filter(|_| at > 0) {
        Some(then) => (then.date_naive(), then.format("%H:%M").to_string()),
        None => (chrono::Local::now().date_naive(), String::new()),
    }
}

/// The folder a path is in, by its name; the top of the sync folder is "Files" (the window's name for it).
pub(crate) fn place_of(rel: Option<&str>) -> String {
    let rel = rel.unwrap_or("").trim_matches(|c| c == '/' || c == '\\');
    let parent = rel.rfind(['/', '\\']).map(|i| &rel[..i]).unwrap_or("");
    match parent.rfind(['/', '\\']) {
        Some(i) => parent[i + 1..].to_string(),
        None if parent.is_empty() => "Files".to_string(),
        None => parent.to_string(),
    }
}

/// The glyph a name gets. A picture is a document here: `Kind::Photo` draws the board's sample picture until the
/// engine hands over pixels, and a sample is never drawn for a real file.
pub(crate) fn kind_of(name: &str, folder: bool) -> Kind {
    if folder {
        return Kind::Folder;
    }
    let ext = name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "xls" | "xlsx" | "xlsm" | "csv" | "ods" | "numbers" | "tsv" => Kind::Sheet,
        "ppt" | "pptx" | "key" | "odp" => Kind::Deck,
        "mov" | "mp4" | "m4v" | "avi" | "mkv" | "webm" | "wmv" => Kind::Video,
        "mp3" | "m4a" | "wav" | "flac" | "aac" | "ogg" | "opus" | "aiff" => Kind::Audio,
        _ => Kind::Document,
    }
}

/// The top-most kept items; "Up to date" only when every file in it is on this PC.
fn kept(db: &StateDb, backups: &BackupsTree) -> Result<Vec<Kept>, String> {
    let mut rows: Vec<(FileEntry, bool, DriveItem)> = Vec::new();
    for entry in model::live_rows(db).map_err(|_| "rows")? {
        if backups.is_hidden(&entry.path) {
            continue;
        }
        let contract = db.get_file_contract_state(&entry.file_id).map_err(|_| "contract")?;
        let is_kept = contract
            .as_ref()
            .is_some_and(|c| c.effective_pin_state() == PinState::Pinned);
        let item = model::item_from(&entry, contract.as_ref(), None);
        rows.push((entry, is_kept, item));
    }
    let by_id: HashMap<&str, &FileEntry> = rows.iter().map(|(e, _, _)| (e.file_id.as_str(), e)).collect();
    Ok(status::offline_entries(&rows)
        .into_iter()
        .filter_map(|entry| {
            let row = *by_id.get(entry.item.id.as_str())?;
            let here = if row.is_dir() {
                let prefix = format!("{}/", row.path.trim_matches('/'));
                rows.iter()
                    .map(|(e, _, _)| e)
                    .filter(|e| !e.is_dir() && e.path.trim_start_matches('/').starts_with(&prefix))
                    .all(|e| e.status == FileStatus::Local)
            } else {
                row.status == FileStatus::Local
            };
            Some(Kept {
                id: entry.item.id.clone(),
                name: entry.item.name.clone(),
                kind: kind_of(&entry.item.name, entry.item.is_folder),
                bytes: entry.bytes,
                contains: match entry.items {
                    Some(n) => Contains::Items(u32::try_from(n).unwrap_or(u32::MAX)),
                    None => Contains::In(place_of(Some(&row.path))),
                },
                status: if here {
                    KeptStatus::UpToDate
                } else {
                    KeptStatus::NotOnPcYet
                },
            })
        })
        .collect())
}

impl<E: ThisPcEngine> ThisPcSource for EngineThisPcSource<E> {
    fn wired(&self) -> bool {
        TURNED_ON
    }

    fn today(&self) -> NaiveDate {
        chrono::Local::now().date_naive()
    }

    fn history(&self) -> Vec<Change> {
        self.snapshot(|s| s.history.clone(), Vec::new())
    }

    fn problems(&self) -> Vec<Change> {
        self.snapshot(|s| s.problems.clone(), Vec::new())
    }

    /// Made due now by the queue's own rule; one that is held, parked or blocked keeps its own action. The row
    /// leaves the list once the read after it no longer finds it a problem.
    fn retry(&mut self, problem: &str) {
        let retryable = self.snapshot(|s| s.retryable.iter().any(|id| id == problem), false);
        if retryable {
            self.send(Job::Retry(vec![problem.to_string()]));
        }
    }

    fn retry_all(&mut self) {
        let ids = self.snapshot(|s| s.retryable.clone(), Vec::new());
        if !ids.is_empty() {
            self.send(Job::Retry(ids));
        }
    }

    fn kept(&self) -> Vec<Kept> {
        self.snapshot(|s| s.kept.clone(), Vec::new())
    }

    /// The pin change: the item stays in Beebeeb, its downloaded copies may leave this PC again. Only an item the
    /// list holds; refused while sync does not run (the list then still holds it).
    fn remove_from_pc(&mut self, id: &str) {
        let listed = self.snapshot(|s| s.kept.iter().any(|k| k.id == id), false);
        if listed {
            self.send(Job::Release(vec![id.to_string()]));
        }
    }

    fn remove_all_from_pc(&mut self) {
        let ids: Vec<String> = self.snapshot(|s| s.kept.iter().map(|k| k.id.clone()).collect(), Vec::new());
        if !ids.is_empty() {
            self.send(Job::Release(ids));
        }
    }
}
