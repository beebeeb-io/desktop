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
//! in the order asked, and sends each answer down one channel, which [`ThisPcSource::pump`] drains
//! (`frames::this_pc::poll`, every frame). The window keeps the last answer itself.
//!
//! Whose answer (review of desktop#377, P1; the model is `engine_host/settings.rs`): every job carries the account
//! and the engine's session it was asked under, and a generation. ONE rule decides whether an answer counts
//! ([`counts`]): its generation is the window's, and the account and the session were the same after the work as at
//! the ask. The window drops what it holds the moment the gate, the account or the session changes (in `pump`, never
//! through a worker job that may not be asked), and every reader hands out nothing unless the vault is open and the
//! signed-in account is the one the answer was read for. A change (Try again, Remove from this PC) runs only while
//! the session is still the one of the press.
//!
//! What the window says when it has no lists ([`ThisPcSource::standing`]): reading, could not be read, or this PC
//! still holds another account's local state. Never an empty list that reads as "nothing here".
//!
//! Whose state (task 2488): the local state database can still belong to an EARLIER account after a sign-out that
//! keeps local data. Nothing is read from it, and nothing is changed in it, unless its recorded owner is the signed-in
//! account (the engine's account-bound handle, `Engine::local_state`); otherwise the lists are empty.
//!
//! When it reads (P2-2): when the gate or the account changes to an open vault, when a section opens and the last
//! answer is older than [`FRESH_FOR`], and after each change. Never on a timer; never a query per row (the kept items
//! are one query, `StateDb::kept_rows`).
//!
//! Must not: show a name while the vault is locked, no one is signed in, or the local state is another account's;
//! log a name, a path or an id; show a value it has not read.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use beebeeb_engine::account::SessionGeneration;
use beebeeb_engine::app::{CredentialStores, Engine, LocalStateRefused};
use beebeeb_engine::backup_protect::BackupsTree;
use beebeeb_engine::drive_v2::model;
use beebeeb_engine::drive_v2::status::{self, SyncProblem};
use beebeeb_engine::signin::gate::GateState;
use beebeeb_engine::state_db::{FileEntry, FileStatus, StateDb};
use beebeeb_engine::status_views;
use chrono::{NaiveDate, TimeZone};

use super::log;
use crate::frames::this_pc::source::{
    Change, Contains, Kept, KeptStatus, Kind, Origin, Pumped, Reason, Standing, ThisPcSource, What,
};

#[cfg(test)]
#[path = "this_pc_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "this_pc_engine_tests.rs"]
mod engine_tests;

/// Whether This PC answers in a real build. It does: the window around it draws the signed-in account and the
/// storage it read (`frames/this_pc/chrome.rs`), never the board's sample. Turned off, the shell hands the window the
/// `Unwired` source (`shell/state.rs`), so nothing of this adapter runs: no worker, no read, no sidebar count.
pub const TURNED_ON: bool = true;

/// The source the running app hands This PC: the engine's while [`TURNED_ON`], else `Unwired` (review of desktop#377,
/// P2-1: a constant that turns the views off must turn the source off too, or the sidebar's count, the worker and its
/// reads would still run).
pub fn source_for<E: ThisPcEngine>(engine: Arc<E>) -> Box<dyn ThisPcSource> {
    source_when(TURNED_ON, engine)
}

/// [`source_for`] with the switch as a value, so both of its answers are tested.
pub(crate) fn source_when<E: ThisPcEngine>(on: bool, engine: Arc<E>) -> Box<dyn ThisPcSource> {
    if on {
        Box::new(EngineThisPcSource::new(engine))
    } else {
        Box::new(crate::frames::this_pc::source::Unwired)
    }
}

/// An answer read this recently is not read again when a section opens.
pub const FRESH_FOR: Duration = Duration::from_secs(30);
/// How far back the history goes (the board: "kept on this PC only", 30 days).
const HISTORY_DAYS: i64 = 30;
/// How many queued changes are read (the 0.8 app's Sync history reads as many).
const QUEUE_LIMIT: u32 = 500;
/// How many finished transfers and local events are read: everything the state database keeps of them.
const TRANSFERS: usize = 100;
const EVENTS: usize = 200;

/// What the window says when a change the person asked for did not happen. Fixed sentences: never a name or an
/// engine's words.
pub const NOT_SYNCING: &str = "Sync isn't running, so nothing was removed from this PC.";
pub const NOT_REMOVED: &str = "Couldn't remove every item from this PC. Try again.";
pub const NOT_RETRIED: &str = "Couldn't try every item again.";

/// What the adapter asks of the engine handle. The app's is `Engine`; a test's is a state database in a folder of
/// its own.
pub trait ThisPcEngine: Send + Sync + 'static {
    /// Someone is signed in and the vault is open. Never waits.
    fn unlocked(&self) -> bool;
    /// Whose account is signed in (the stored email, else the user id the server named). Never waits.
    fn account(&self) -> Option<String>;
    /// The engine's session, which moves on at every sign-in, lock and sign-out, also back to the same account
    /// (`Engine::session_turn`). Never waits.
    fn session(&self) -> SessionGeneration;
    /// Read the signed-in account's local state, through the engine's account-bound handle (task 2488). `Err`: it
    /// is not handed out, and why (nothing synced yet, another account's, the session changed, unreadable).
    fn local_state<T>(&self, read: impl FnOnce(&StateDb) -> T) -> Result<T, LocalStateRefused>;
    /// The same for an operation that writes into it.
    fn change_local_state<T>(&self, change: impl FnOnce(&StateDb) -> T) -> Result<T, LocalStateRefused>;
    /// Sync runs for this handle. May wait: a worker's.
    fn syncing(&self) -> bool;
    /// Keep an item on this PC, or stop keeping it. Blocks: a worker's.
    fn set_kept(&self, file_id: &str, kept: bool) -> Result<(), String>;
}

impl<C: CredentialStores> ThisPcEngine for Engine<C> {
    fn unlocked(&self) -> bool {
        self.view().gate == GateState::Ready
    }
    fn account(&self) -> Option<String> {
        self.account_email().or_else(|| self.user_id())
    }
    fn session(&self) -> SessionGeneration {
        self.session_turn()
    }
    fn local_state<T>(&self, read: impl FnOnce(&StateDb) -> T) -> Result<T, LocalStateRefused> {
        Engine::local_state(self).and_then(|state| state.with(read))
    }
    fn change_local_state<T>(&self, change: impl FnOnce(&StateDb) -> T) -> Result<T, LocalStateRefused> {
        Engine::local_state_for_changes(self).and_then(|state| state.with(change))
    }
    fn syncing(&self) -> bool {
        self.is_syncing()
    }
    fn set_kept(&self, file_id: &str, kept: bool) -> Result<(), String> {
        Engine::set_kept(self, file_id, kept)
    }
}

/// What the lists hold, as last read.
#[derive(Default, Clone, PartialEq)]
pub(crate) struct Snapshot {
    pub history: Vec<Change>,
    pub problems: Vec<Change>,
    /// The queue ids Try again applies to.
    pub retryable: Vec<String>,
    pub kept: Vec<Kept>,
}

/// How many of each, never a name, a place or an id: a snapshot that reaches a log line or a failed assertion says
/// nothing about a person's files.
impl std::fmt::Debug for Snapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Snapshot {{ history: {}, problems: {}, retryable: {}, kept: {} }}",
            self.history.len(),
            self.problems.len(),
            self.retryable.len(),
            self.kept.len()
        )
    }
}

/// Why a read has no lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NoRead {
    /// The local state could not be opened or read.
    Unreadable,
    /// This PC's local state is not provably this account's (task 2488): nothing of it is read.
    AnotherAccounts,
    /// The session was not one to read under at that moment (it changed, ended, or a change of it was under way):
    /// nothing is known, nothing held changes, and the next opening asks again.
    NotNow,
}

/// Whose session something was asked under, or read under.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Whose {
    account: String,
    session: SessionGeneration,
}

/// The session the engine is in now: `None` unless someone is signed in with the vault open.
fn whose_now<E: ThisPcEngine>(engine: &E) -> Option<Whose> {
    let session = engine.session();
    let account = engine.account()?;
    // The session is read before and after the rest: one that moved on in between is no session to answer for.
    (engine.unlocked() && engine.session() == session).then_some(Whose { account, session })
}

/// THE rule of what the window keeps of an answer: it was asked in the window's current generation, and the session
/// it was read under (`after`, as the worker saw it once its work was done) is the one the window is in (`seen`).
/// Every answer goes through here, a read and a refusal alike.
fn counts(generation: u64, current: u64, after: Option<&Whose>, seen: Option<&Whose>) -> bool {
    generation == current && after.is_some() && after == seen
}

enum Ask {
    Read,
    Retry(Vec<String>),
    Release(Vec<String>),
}

/// One job: what is asked, under whose session, in which generation.
struct Job {
    ask: Ask,
    whose: Whose,
    generation: u64,
}

/// One job's answer: the generation it was asked in, the session it is for (`None`: the account or the session
/// changed, or the vault closed, during the work: never shown), a change that did not happen in words the window
/// says, and what was read after it (`Err`: no lists, and why).
struct Answer {
    generation: u64,
    whose: Option<Whose>,
    refused: Option<&'static str>,
    read: Result<Snapshot, NoRead>,
}

/// This PC on the real engine. See the module's doc.
pub struct EngineThisPcSource<E: ThisPcEngine> {
    engine: Arc<E>,
    jobs: Option<mpsc::Sender<Job>>,
    answers: mpsc::Sender<Answer>,
    inbox: mpsc::Receiver<Answer>,
    /// Jobs asked and not finished.
    pending: Arc<AtomicUsize>,
    /// What the window holds, and whose account it was read for (always `seen`'s, or nothing).
    held: Option<Snapshot>,
    /// Why nothing is held, once an answer said so (`None`: no answer yet).
    no_read: Option<NoRead>,
    /// The session the last `pump` saw (`None`: signed out, or locked): a change drops `held` and starts a new
    /// generation.
    seen: Option<Whose>,
    generation: u64,
    read_at: Option<Instant>,
    /// A read is out in this generation: no second one is asked.
    reading: bool,
}

impl<E: ThisPcEngine> EngineThisPcSource<E> {
    /// The source of the running app. Starts nothing: the first `pump` that sees an open vault asks the first read.
    pub fn new(engine: Arc<E>) -> Self {
        let (answers, inbox) = mpsc::channel();
        Self {
            engine,
            jobs: None,
            answers,
            inbox,
            pending: Arc::default(),
            held: None,
            no_read: None,
            seen: None,
            generation: 0,
            read_at: None,
            reading: false,
        }
    }

    /// Put an answer in the inbox as the worker would (tests of what `pump` does with one).
    #[cfg(test)]
    fn inbox_for_test(&self, answer: Answer) {
        let _ = self.answers.send(answer);
    }

    /// No job is waiting or running (tests wait on it).
    #[cfg(test)]
    pub(crate) fn idle(&self) -> bool {
        self.pending.load(Ordering::SeqCst) == 0
    }

    /// Send `ask` for the session the window sees now. Nothing is sent while no one is signed in with the vault open.
    fn send(&mut self, ask: Ask) -> bool {
        let Some(whose) = self.seen.clone() else {
            return false;
        };
        if self.jobs.is_none() {
            let (tx, rx) = mpsc::channel::<Job>();
            let (engine, answers, pending) = (self.engine.clone(), self.answers.clone(), self.pending.clone());
            let started = std::thread::Builder::new().name("this-pc".into()).spawn(move || {
                // Ends when the source is dropped (its sender with it).
                for job in rx {
                    let (generation, whose) = (job.generation, job.whose.clone());
                    let done = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(&*engine, &answers, job)));
                    if done.is_err() {
                        log("this pc: a read or a change ended early");
                        // Said as "could not be read" (for the session that asked, if it is still the window's).
                        let _ = answers.send(Answer {
                            generation,
                            whose: Some(whose),
                            refused: None,
                            read: Err(NoRead::Unreadable),
                        });
                    }
                    pending.fetch_sub(1, Ordering::SeqCst);
                }
            });
            match started {
                Ok(_) => self.jobs = Some(tx),
                Err(_) => {
                    log("this pc: could not start its worker");
                    return false;
                }
            }
        }
        self.pending.fetch_add(1, Ordering::SeqCst);
        let job = Job {
            ask,
            whose,
            generation: self.generation,
        };
        let sent = self.jobs.as_ref().is_some_and(|tx| tx.send(job).is_ok());
        if !sent {
            self.pending.fetch_sub(1, Ordering::SeqCst);
        }
        sent
    }

    fn ask_read(&mut self) {
        if !self.reading {
            self.reading = self.send(Ask::Read);
        }
    }

    /// The session the window saw is still the engine's (the vault open, the same account, the same session).
    fn current(&self) -> bool {
        self.seen.is_some() && whose_now(&*self.engine) == self.seen
    }

    /// What the window holds, while the session is still the one it was read under.
    fn held(&self) -> Option<&Snapshot> {
        self.held.as_ref().filter(|_| self.current())
    }
}

/// The session `whose` names is still the engine's, with the vault open.
fn still_for<E: ThisPcEngine>(engine: &E, whose: &Whose) -> bool {
    whose_now(engine).as_ref() == Some(whose)
}

/// One job, on the worker. A change runs only under the session it was asked in, on a state database that account
/// owns; every job ends with a read, so the lists show what the engine has after it.
fn run<E: ThisPcEngine>(engine: &E, answers: &mpsc::Sender<Answer>, job: Job) {
    let Job { ask, whose, generation } = job;
    let refused = match ask {
        Ask::Read => None,
        Ask::Retry(ids) => match retry(engine, &whose, &ids) {
            Some(0) => None,
            Some(failed) => {
                log(format!(
                    "this pc: {failed} of {} changes could not be tried again",
                    ids.len()
                ));
                Some(NOT_RETRIED)
            }
            None => {
                log("this pc: nothing tried again (no local state of this account)");
                Some(NOT_RETRIED)
            }
        },
        Ask::Release(ids) => {
            if !engine.syncing() {
                log("this pc: not removed from this PC: sync does not run");
                Some(NOT_SYNCING)
            } else {
                // Each item under the session of the press: one that moved on stops the rest.
                let failed = ids
                    .iter()
                    .filter(|id| !(owns_state(engine, &whose) && engine.set_kept(id, false).is_ok()))
                    .count();
                (failed > 0).then(|| {
                    log(format!(
                        "this pc: {failed} of {} items were not removed from this PC",
                        ids.len()
                    ));
                    NOT_REMOVED
                })
            }
        }
    };
    let read = read_for(engine, &whose);
    // The session must be the same after the work as before it: an answer that outlived its session is dropped.
    let whose = still_for(engine, &whose).then_some(whose);
    let _ = answers.send(Answer {
        generation,
        whose,
        refused,
        read,
    });
}

/// Tries `ids` again in the local state of `whose` account: how many could not be. `None`: the session moved on, or
/// the local state is not handed out to it (the account-bound handle, task 2488).
fn retry<E: ThisPcEngine>(engine: &E, whose: &Whose, ids: &[String]) -> Option<usize> {
    if !still_for(engine, whose) {
        return None;
    }
    let now = chrono::Utc::now().timestamp();
    engine
        .change_local_state(|db| ids.iter().filter(|id| db.retry_unsent_change(id, now).is_err()).count())
        .ok()
}

/// The session is still `whose` and the local state is handed out to it.
fn owns_state<E: ThisPcEngine>(engine: &E, whose: &Whose) -> bool {
    still_for(engine, whose) && engine.local_state(|_| ()).is_ok()
}

/// Everything the sections show under `whose` session. A PC with nothing synced yet reads as empty lists; another
/// account's local state is never read and says so (task 2488); a state database that could not be opened or read
/// is `Unreadable` (the window keeps what it holds of this account).
fn read_for<E: ThisPcEngine>(engine: &E, whose: &Whose) -> Result<Snapshot, NoRead> {
    if !still_for(engine, whose) {
        // Dropped by `counts` (the session moved on); nothing is read for it.
        return Err(NoRead::NotNow);
    }
    match engine.local_state(|db| read(db, chrono::Utc::now().timestamp())) {
        Ok(read) => read.map_err(|_| {
            log("this pc: the local history could not be read");
            NoRead::Unreadable
        }),
        Err(LocalStateRefused::NothingSynced) => Ok(Snapshot::default()),
        Err(refused) if refused.not_this_accounts() => Err(NoRead::AnotherAccounts),
        // Signed out, locked, ended or changing: not a failure to read, and not this answer's to say.
        Err(
            LocalStateRefused::SessionChanged
            | LocalStateRefused::SessionEnded
            | LocalStateRefused::SignedOut
            | LocalStateRefused::Locked,
        ) => Err(NoRead::NotNow),
        Err(_) => {
            log("this pc: the local state could not be opened");
            Err(NoRead::Unreadable)
        }
    }
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

/// The top-most kept items; "Up to date" only when every file in it is on this PC. One query: the kept rows only
/// (a kept folder's contents are kept too, so its count and its status come from the same rows).
fn kept(db: &StateDb, backups: &BackupsTree) -> Result<Vec<Kept>, String> {
    let rows: Vec<(FileEntry, bool, beebeeb_engine::drive_v2::dto::DriveItem)> = db
        .kept_rows()
        .map_err(|_| "kept")?
        .into_iter()
        .filter(|entry| !backups.is_hidden(&entry.path))
        .map(|entry| {
            let item = model::item_from(&entry, None, None);
            (entry, true, item)
        })
        .collect();
    let by_id: std::collections::HashMap<&str, &FileEntry> =
        rows.iter().map(|(e, _, _)| (e.file_id.as_str(), e)).collect();
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

    /// The gate, the account or the session changed: what the window held goes now, and an open vault is read
    /// again. Then the worker's answers: one that [`counts`] replaces what is held, and its refusal is said once.
    fn pump(&mut self) -> Pumped {
        let mut pumped = Pumped {
            // Before the inbox is drained: when nothing is out, every answer is already in it.
            busy: self.pending.load(Ordering::SeqCst) > 0,
            ..Pumped::default()
        };
        let now = whose_now(&*self.engine);
        if now != self.seen {
            self.seen = now;
            self.held = None;
            self.no_read = None;
            self.read_at = None;
            self.reading = false;
            self.generation += 1;
            pumped.changed = true;
            self.ask_read();
        }
        while let Ok(answer) = self.inbox.try_recv() {
            let Answer {
                generation,
                whose,
                refused,
                read,
            } = answer;
            if generation == self.generation {
                // The read of this generation is back, whoever it was for: another may be asked.
                self.reading = false;
                pumped.changed = true;
            }
            if !counts(generation, self.generation, whose.as_ref(), self.seen.as_ref()) {
                continue;
            }
            if let Some(words) = refused {
                pumped.refusal = Some(words);
            }
            self.read_at = Some(Instant::now());
            match read {
                Ok(snapshot) => {
                    self.held = Some(snapshot);
                    self.no_read = None;
                }
                // Another account's local state: nothing of an earlier answer stays either.
                Err(NoRead::AnotherAccounts) => {
                    self.held = None;
                    self.no_read = Some(NoRead::AnotherAccounts);
                }
                // A read that failed keeps what is held of this account (before the first answer: it says so).
                Err(NoRead::Unreadable) => self.no_read = Some(NoRead::Unreadable),
                // Nothing was learned: what is held and what is said stay; the next opening asks again.
                Err(NoRead::NotNow) => self.read_at = None,
            }
        }
        pumped.busy |= self.pending.load(Ordering::SeqCst) > 0;
        pumped
    }

    /// What the sections say while there are no lists. Lists that are held win over a later read that failed.
    fn standing(&self) -> Standing {
        if !self.current() {
            return Standing::Reading;
        }
        match (&self.held, self.no_read) {
            (Some(_), _) => Standing::Ready,
            (None, Some(NoRead::AnotherAccounts)) => Standing::AnotherAccounts,
            (None, Some(NoRead::Unreadable)) => Standing::Unreadable,
            (None, Some(NoRead::NotNow) | None) => Standing::Reading,
        }
    }

    /// A section opened: read again when what is held is older than [`FRESH_FOR`].
    fn opened(&mut self) {
        let fresh = self.read_at.is_some_and(|at| at.elapsed() < FRESH_FOR) && self.held().is_some();
        if !fresh {
            self.ask_read();
        }
    }

    fn today(&self) -> NaiveDate {
        chrono::Local::now().date_naive()
    }

    fn history(&self) -> Vec<Change> {
        self.held().map(|s| s.history.clone()).unwrap_or_default()
    }

    fn problems(&self) -> Vec<Change> {
        self.held().map(|s| s.problems.clone()).unwrap_or_default()
    }

    /// Made due now by the queue's own rule; one that is held, parked or blocked keeps its own action. The row
    /// leaves the list once the read after it no longer finds it a problem.
    fn retry(&mut self, problem: &str) {
        let retryable = self.held().is_some_and(|s| s.retryable.iter().any(|id| id == problem));
        if retryable {
            self.send(Ask::Retry(vec![problem.to_string()]));
        }
    }

    fn retry_all(&mut self) {
        let ids = self.held().map(|s| s.retryable.clone()).unwrap_or_default();
        if !ids.is_empty() {
            self.send(Ask::Retry(ids));
        }
    }

    fn kept(&self) -> Vec<Kept> {
        self.held().map(|s| s.kept.clone()).unwrap_or_default()
    }

    /// The count the sidebar draws, without a copy of the list.
    fn kept_count(&self) -> usize {
        self.held().map_or(0, |s| s.kept.len())
    }

    /// The pin change: the item stays in Beebeeb, its downloaded copies may leave this PC again. Only an item the
    /// list holds; refused, and said, while sync does not run (the list then still holds it).
    fn remove_from_pc(&mut self, id: &str) {
        let listed = self.held().is_some_and(|s| s.kept.iter().any(|k| k.id == id));
        if listed {
            self.send(Ask::Release(vec![id.to_string()]));
        }
    }

    fn remove_all_from_pc(&mut self) {
        let ids: Vec<String> = self
            .held()
            .map(|s| s.kept.iter().map(|k| k.id.clone()).collect())
            .unwrap_or_default();
        if !ids.is_empty() {
            self.send(Ask::Release(ids));
        }
    }
}
