//! Search on the real engine: [`EngineSearchSource`], in the running app (`shell::state::SEARCH_ON_ENGINE`, task
//! 2488). The index is built only through the engine's account-bound handle (`Engine::local_state`): a local state
//! that is not the session's account's (a sign-out keeps the data) builds no index, and Search says it is not
//! available, never another account's names.
//!
//! What it answers, and from where:
//! - `search`: names in this computer's copy of the list (`desktop_search::build_local_index` over the local state,
//!   then `query_local_index`), any case, narrowed by kind, at most [`SHOWN`] results. Folders are not in the
//!   engine's index, so the Folders pill finds nothing yet; shared items are left out (ruling 1701). While no index
//!   is held, it says why when the engine refused this PC's local state as not this account's
//!   (`Answer::NotThisAccounts`: the engine's own word, `LocalStateRefused::not_this_accounts`); any other refusal is
//!   `Unavailable`.
//! - `details`: the facts the index holds for a result (kind, size, changed, where, on this PC or online only).
//! - `recent`, `remember`, `clear_recent` (DW-0801, slice A8): the account's recent searches as the engine keeps them
//!   in its own local state (`Engine::recent_searches_for` and its neighbours: per account, through the same
//!   account-bound handle; deleted at sign-out and by Clear). Read when Search opens, changed on a worker, one piece
//!   of work at a time in the order asked; `recent` shows only what the engine answered, for the session of the
//!   moment. Nothing is kept anywhere else.
//!
//! Whose answer (review of desktop#378): an index and a result belong to one SESSION, the engine's own turn
//! (`Engine::session_turn`: it moves on at every sign-in, unlock, lock and sign-out) together with the account. A
//! worker says the session it began in and ended in; the window takes an answer in only when that is the session
//! it sees now. An account's name alone is not enough: it stays the same across a lock.
//!
//! What is kept, and until when: everything this source holds (the index, the last result, the query asked, the
//! recent searches, the work on its way) belongs to ONE session. `opened`, `ask`, `remember`, `clear_recent` and
//! `pump` first look whether that is still the session of the moment and drop all of it when it is not (the
//! window's side, no worker asked); `forget` drops it when the window says so; and every reader hands out nothing of
//! a session that is gone even before either ran.
//!
//! When it works (never from a draw): the index is built when Search opens (`opened`) and what is held is older
//! than [`FRESH_FOR`]; a query runs when the window says the words or the pill changed (`ask`), or when the index
//! it waited for lands (`pump`). `wired`, `search` and `details` only read what is held.
//!
//! Thread rule: the index is built on a worker and kept in memory. A query runs on a worker too, with a limit; its
//! result answers only the query that asked (the words, the pill and the index it ran on), and `search` says
//! `Searching` until it lands. The window is woken when either lands.
//!
//! Must not: log a query, a recent search, a name or a path; keep a name or a search of a session that ended.

use std::collections::VecDeque;
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use beebeeb_engine::account::SessionGeneration;
use beebeeb_engine::app::{CredentialStores, Engine, LocalStateRefused};
use beebeeb_engine::desktop_search::{LocalSearchIndex, LocalSearchRecord, build_local_index, query_local_index};
use beebeeb_engine::drive_v2::model::{ROOT_NAME, mime_for_name};
use chrono::Local;

use super::log;
use super::trash::{kind_of, when};
use crate::frames::files::source::Details;
use crate::frames::search::source::{Answer, Hit, Narrow, Pumped, SearchSource};
use crate::ui::file_list::PcState;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_accounts;

/// An index this recent is not built again when Search opens.
pub const FRESH_FOR: Duration = Duration::from_secs(30);

/// A refused build (nothing synced yet, an unreadable list) is built again once it is this old, while Search is open.
pub const RETRY_AFTER: Duration = Duration::from_secs(3);

/// At most this many results are shown.
pub const SHOWN: usize = 100;

/// What Search needs of the engine: whose session it is, the local index (disk work: a worker calls it), and a way
/// to wake the window. The running app's is the engine handle; a test's reads a state database of its own.
pub trait SearchHost: Send + Sync + 'static {
    /// The engine's session turn: it moves on at every sign-in, unlock, lock and sign-out.
    type Turn: Copy + Eq + Send + 'static;
    /// The signed-in account (in memory, never waits); `None`: signed out.
    fn account(&self) -> Option<String>;
    /// The session's turn now. In memory, never waits.
    fn turn(&self) -> Self::Turn;
    /// A session is in memory (signed in and unlocked). Never waits.
    fn unlocked(&self) -> bool;
    /// The index of this computer's local state. `Err`: why there is none, as the engine's account-bound handle says
    /// it (another account's local state, nothing synced yet, …); a list that cannot be read is `Unreadable`.
    fn index(&self) -> Result<LocalSearchIndex, LocalStateRefused>;
    /// The recent searches of the account of the session `turn`, newest first (disk work: a worker calls it). `Err`:
    /// none can be read (another account's local state, nothing synced yet, the session moved on).
    fn recent_searches(&self, turn: Self::Turn) -> Result<Vec<String>, ()>;
    /// WRITES (disk work): `query` is the newest recent search of the account of the session `turn`. Answers the list
    /// as it is now kept; `Err`: nothing was kept.
    fn remember_search(&self, turn: Self::Turn, query: &str) -> Result<Vec<String>, ()>;
    /// WRITES (disk work, Clear): the recent searches of the account of the session `turn` are deleted. `Err`: they
    /// were not.
    fn clear_recent_searches(&self, turn: Self::Turn) -> Result<(), ()>;
    fn wake(&self);
}

impl<C: CredentialStores> SearchHost for Engine<C> {
    type Turn = SessionGeneration;

    fn account(&self) -> Option<String> {
        self.account_email().or_else(|| self.user_id())
    }

    fn turn(&self) -> SessionGeneration {
        self.session_turn()
    }

    fn unlocked(&self) -> bool {
        self.has_session()
    }

    fn index(&self) -> Result<LocalSearchIndex, LocalStateRefused> {
        // Task 2488: the signed-in account's local state only; any refusal builds nothing, and keeps its kind.
        let state = self.local_state()?;
        state
            .with(|db| build_local_index(db))?
            .map_err(|_| LocalStateRefused::Unreadable)
    }

    fn recent_searches(&self, turn: SessionGeneration) -> Result<Vec<String>, ()> {
        self.recent_searches_for(turn).map_err(|_| ())
    }

    fn remember_search(&self, turn: SessionGeneration, query: &str) -> Result<Vec<String>, ()> {
        self.remember_search_for(turn, query).map_err(|_| ())
    }

    fn clear_recent_searches(&self, turn: SessionGeneration) -> Result<(), ()> {
        self.clear_recent_searches_for(turn).map_err(|_| ())
    }

    fn wake(&self) {
        self.wake_host();
    }
}

/// One session: the engine's turn and the account signed in with the vault open.
#[derive(Clone, PartialEq, Eq)]
struct Session<T> {
    turn: T,
    account: String,
}

/// The session `host` is in now; `None`: no one is signed in with the vault open.
fn session_of<H: SearchHost>(host: &H) -> Option<Session<H::Turn>> {
    if !host.unlocked() {
        return None;
    }
    let account = host.account()?;
    Some(Session {
        turn: host.turn(),
        account,
    })
}

/// A query as it was asked: the words, the pill, and the index it ran on (by number). A result answers only the
/// query that asked for it, on the index it ran on.
type Asked = (String, Narrow, u64);

/// The index held, the session it was built in and its number among the indexes installed.
struct Held<T> {
    of: Session<T>,
    index: Arc<LocalSearchIndex>,
    number: u64,
}

/// A worker's answer, with the number of the job that asked.
enum Landed<T> {
    /// The index or why there is none, and the session it was built in (`None`: the session was not the same after
    /// the build as before).
    Built(u64, Option<Session<T>>, Result<LocalSearchIndex, LocalStateRefused>),
    /// A query's results, and the session whose index it ran on.
    Found(u64, Session<T>, Asked, Vec<Hit>),
    /// The recent searches as the engine keeps them after a read or a change, and the session the work ran in
    /// (`None`: the session was not the same after the work as before).
    Recent(u64, Option<Session<T>>, Result<Vec<String>, ()>),
}

/// A piece of work on the recent searches, in the order the window asked for it.
enum RecentWork {
    /// Read them (Search opened).
    Read,
    /// Keep a search the person ran as the newest.
    Remember(String),
    /// Delete them (Clear).
    Clear,
}

/// Search on the real engine. See the module's doc.
pub struct EngineSearchSource<H: SearchHost> {
    host: Arc<H>,
    answers: mpsc::Sender<Landed<H::Turn>>,
    inbox: mpsc::Receiver<Landed<H::Turn>>,
    /// The session everything below belongs to (`None`: no one signed in with the vault open, and nothing held).
    of: Option<Session<H::Turn>>,
    held: Option<Held<H::Turn>>,
    /// Numbers the indexes installed.
    installed: u64,
    /// Numbers the jobs sent: an answer counts only for the job that is still waited for.
    jobs: u64,
    /// When the last build was answered (a failure too).
    built_at: Option<Instant>,
    /// How old a refusal is before it is built again.
    retry_after: Duration,
    /// Search was opened in this session: only then does a refusal come back for another build.
    watching: bool,
    /// The build on its way.
    building: Option<u64>,
    /// Why the last build of a session built nothing, and that session (`None`: it built, or none was refused). Said
    /// only to that session (`not_this_accounts`).
    refused: Option<(Session<H::Turn>, LocalStateRefused)>,
    /// What the window asked for last: run again when a new index lands.
    wanted: Option<(String, Narrow)>,
    /// The query on its way.
    asked: Option<(u64, Asked)>,
    /// The last result, and the query it answers.
    found: Option<(Asked, Vec<Hit>)>,
    /// The recent searches as the engine last answered them, newest first, and the session that work ran in.
    recent: Option<(Session<H::Turn>, Vec<String>)>,
    /// The work on the recent searches that is on its way (one at a time), by job number.
    recent_out: Option<u64>,
    /// The work on the recent searches that waits for the one on its way, in the order asked.
    recent_next: VecDeque<RecentWork>,
}

#[cfg(test)]
thread_local! {
    /// How many Search adapters this thread made (a test proves the shell made none).
    static MADE_HERE: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// How many Search adapters this thread made.
#[cfg(test)]
pub(crate) fn made_here() -> usize {
    MADE_HERE.with(std::cell::Cell::get)
}

impl<H: SearchHost> EngineSearchSource<H> {
    /// Nothing is read until Search opens.
    pub fn new(host: Arc<H>) -> Self {
        #[cfg(test)]
        MADE_HERE.with(|made| made.set(made.get() + 1));
        let (answers, inbox) = mpsc::channel();
        Self {
            host,
            answers,
            inbox,
            of: None,
            held: None,
            installed: 0,
            jobs: 0,
            built_at: None,
            retry_after: RETRY_AFTER,
            watching: false,
            building: None,
            refused: None,
            wanted: None,
            asked: None,
            found: None,
            recent: None,
            recent_out: None,
            recent_next: VecDeque::new(),
        }
    }

    /// The index, while the session it was built in is the current one. Asked on every reader: an index of a
    /// session that ended is never handed out, even before the next `pump` drops it.
    fn current(&self) -> Option<&Held<H::Turn>> {
        let now = session_of(&*self.host)?;
        self.held.as_ref().filter(|held| held.of == now)
    }

    /// The current session's last build was refused because this PC's local state is not provably this account's
    /// (the engine's `not_this_accounts`). Asked on every reader, as `current`: a refusal of a session that ended is
    /// never said, even before the next `pump` drops it.
    fn not_this_accounts(&self) -> bool {
        let Some(now) = session_of(&*self.host) else {
            return false;
        };
        self.refused
            .as_ref()
            .is_some_and(|(of, refused)| *of == now && refused.not_this_accounts())
    }

    /// The current session's last build was refused for a reason that passes (not this account's state: that stays
    /// said, and is never built again by itself).
    fn refused_for_now(&self) -> bool {
        let Some(now) = session_of(&*self.host) else {
            return false;
        };
        self.refused
            .as_ref()
            .is_some_and(|(of, refused)| *of == now && !refused.not_this_accounts())
    }

    /// Nothing is held and the first sync has not given this PC a list yet (`NothingSynced`): the files are being got
    /// ready, which is not "not available".
    fn getting_ready(&self) -> bool {
        let Some(now) = session_of(&*self.host) else {
            return false;
        };
        self.current().is_none()
            && self
                .refused
                .as_ref()
                .is_some_and(|(of, refused)| *of == now && matches!(refused, LocalStateRefused::NothingSynced))
    }

    /// Follow the session of the moment: when it is not the one this source's state belongs to, all of that state
    /// goes now. `true`: something a draw reads went.
    fn follow(&mut self) -> bool {
        let now = session_of(&*self.host);
        if self.of == now {
            return false;
        }
        let held = self.held.is_some()
            || self.found.is_some()
            || self.refused.is_some()
            || self.building.is_some()
            || self.asked.is_some()
            || self.recent.is_some()
            || self.recent_out.is_some();
        self.forget();
        self.of = now;
        held
    }

    /// A build for the session of the moment is on its way.
    fn building_now(&self) -> bool {
        self.building.is_some() && self.of.is_some() && self.of == session_of(&*self.host)
    }

    /// Build the index on a worker, for the session of the moment.
    fn build(&mut self) {
        if self.building.is_some() || self.of.is_none() {
            return;
        }
        self.jobs += 1;
        let (host, answers, job) = (self.host.clone(), self.answers.clone(), self.jobs);
        let started = std::thread::Builder::new().name("search-index".into()).spawn(move || {
            let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let before = session_of(&*host);
                let index = host.index();
                // The same session after the work as before it, or the index is nobody's.
                (
                    before.filter(|before| session_of(&*host).as_ref() == Some(before)),
                    index,
                )
            }));
            let (of, index) = built.unwrap_or((None, Err(LocalStateRefused::Unreadable)));
            let _ = answers.send(Landed::Built(job, of, index));
            host.wake();
        });
        if started.is_ok() {
            self.building = Some(job);
        } else {
            log("search: could not start a thread for the index");
            self.built_at = Some(Instant::now());
        }
    }

    /// Run what the window asked for on a worker, on the current session's index: at most [`SHOWN`] results of the
    /// pill's kind, sent back with the query they answer. Nothing while no index of this session is held, or the
    /// answer to this very query is here or on its way.
    fn run_wanted(&mut self) {
        let Some((query, narrow)) = self.wanted.clone() else {
            return;
        };
        let Some(held) = self.current() else {
            return;
        };
        let asked: Asked = (query, narrow, held.number);
        let here = self.found.as_ref().is_some_and(|(of, _)| *of == asked);
        let coming = self.asked.as_ref().is_some_and(|(_, of)| *of == asked);
        if here || coming {
            return;
        }
        let (index, of) = (held.index.clone(), held.of.clone());
        self.jobs += 1;
        let (host, answers, job, sent) = (self.host.clone(), self.answers.clone(), self.jobs, asked.clone());
        let started = std::thread::Builder::new().name("search-query".into()).spawn(move || {
            let (query, narrow, _) = &sent;
            // A pill narrows after the engine's limit: ask for more, keep SHOWN.
            let limit = if *narrow == Narrow::All { SHOWN } else { SHOWN * 10 };
            let hits = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                query_local_index(&index, query, limit, true)
                    .results
                    .into_iter()
                    .filter_map(|result| index.records.get(&result.file_id))
                    .map(hit_of)
                    .filter(|hit| narrow.keeps(hit.kind))
                    .take(SHOWN)
                    .collect::<Vec<_>>()
            }))
            .unwrap_or_default();
            drop(index);
            let _ = answers.send(Landed::Found(job, of, sent, hits));
            host.wake();
        });
        if started.is_ok() {
            self.asked = Some((job, asked));
        } else {
            log("search: could not start a thread for a query");
        }
    }

    /// Queue `work` on the recent searches for the session of the moment (none signed in with the vault open:
    /// nothing is queued), and start it when nothing else on them is on its way.
    fn ask_recent(&mut self, work: RecentWork) {
        self.follow();
        if self.of.is_none() {
            return;
        }
        if matches!(work, RecentWork::Clear) {
            // What waits would be deleted by this Clear anyway: it goes now.
            self.recent_next.clear();
        }
        self.recent_next.push_back(work);
        self.start_recent();
    }

    /// The next work on the recent searches, on a worker, for the session this source's state belongs to: the
    /// worker asks the engine only while that is still the session, under that session's turn, and its answer counts
    /// only when the session after the work is the same as before. One at a time, in the order asked.
    fn start_recent(&mut self) {
        if self.recent_out.is_some() {
            return;
        }
        let Some(of) = self.of.clone() else {
            self.recent_next.clear();
            return;
        };
        let Some(work) = self.recent_next.pop_front() else {
            return;
        };
        self.jobs += 1;
        let (host, answers, job) = (self.host.clone(), self.answers.clone(), self.jobs);
        let started = std::thread::Builder::new().name("search-recent".into()).spawn(move || {
            let done = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let Some(before) = session_of(&*host).filter(|now| *now == of) else {
                    return (None, Err(()));
                };
                let list = match &work {
                    RecentWork::Read => host.recent_searches(before.turn),
                    RecentWork::Remember(query) => host.remember_search(before.turn, query),
                    RecentWork::Clear => host.clear_recent_searches(before.turn).map(|()| Vec::new()),
                };
                // The same session after the work as before it, or the answer is nobody's.
                let same = session_of(&*host).as_ref() == Some(&before);
                (same.then_some(before), list)
            }));
            let (of, list) = done.unwrap_or((None, Err(())));
            let _ = answers.send(Landed::Recent(job, of, list));
            host.wake();
        });
        if started.is_ok() {
            self.recent_out = Some(job);
        } else {
            log("search: could not start a thread for the recent searches");
            self.recent_next.clear();
        }
    }

    fn take_in(&mut self, landed: Landed<H::Turn>) {
        match landed {
            Landed::Built(job, of, result) => {
                if self.building != Some(job) {
                    // A build asked before a forget: nobody waits for it.
                    return;
                }
                self.building = None;
                if of.is_none() || of != self.of {
                    // Built for a session that is gone, or across a change of session: dropped, never held. The
                    // session of the moment builds its own when Search opens.
                    self.built_at = None;
                    return;
                }
                self.built_at = Some(Instant::now());
                match (of, result) {
                    (Some(of), Ok(index)) => {
                        self.installed += 1;
                        self.held = Some(Held {
                            of,
                            index: Arc::new(index),
                            number: self.installed,
                        });
                        self.refused = None;
                        self.found = None;
                        self.asked = None;
                        self.run_wanted();
                    }
                    // Kept: the index before still answers its own session. None before: not wired, and why is kept
                    // for `search` to say (only to this session).
                    (Some(of), Err(refused)) => {
                        self.refused = Some((of, refused));
                        log("search: the list on this PC could not be read");
                    }
                    // Not reached: a build of no session returned above.
                    (None, _) => {}
                }
            }
            Landed::Found(job, of, asked, hits) => {
                if self.asked.as_ref().map(|(waited, _)| *waited) != Some(job) {
                    return;
                }
                self.asked = None;
                let mine = Some(&of) == self.of.as_ref() && self.current().is_some_and(|held| held.number == asked.2);
                if mine {
                    self.found = Some((asked, hits));
                }
            }
            Landed::Recent(job, of, list) => {
                if self.recent_out != Some(job) {
                    // Asked before a forget: nobody waits for it.
                    return;
                }
                self.recent_out = None;
                match (of, list) {
                    // Only what the engine answered, for the session this source's state belongs to.
                    (Some(of), Ok(list)) if Some(&of) == self.of.as_ref() => self.recent = Some((of, list)),
                    // Not read or not kept: what is held still says what the engine keeps (a change that failed
                    // changed nothing there). Across a change of session: nobody's.
                    (Some(of), Err(())) if Some(&of) == self.of.as_ref() => {
                        log("search: the recent searches could not be read or kept")
                    }
                    _ => {}
                }
                self.start_recent();
            }
        }
    }
}

impl<H: SearchHost> SearchSource for EngineSearchSource<H> {
    /// An index of the current session is held, or is being built (a query then reads `Searching`). Reads only.
    fn wired(&self) -> bool {
        self.current().is_some() || self.building_now() || self.getting_ready()
    }

    /// The recent searches the engine answered for the session of the moment, newest first; none while it has not
    /// (yet) answered, or for another session. Reads only: never asks the engine (a draw calls it).
    fn recent(&self) -> Vec<String> {
        let Some(now) = session_of(&*self.host) else {
            return Vec::new();
        };
        match &self.recent {
            Some((of, list)) if *of == now => list.clone(),
            _ => Vec::new(),
        }
    }

    /// Clear: the engine deletes the account's recent searches (on a worker); the list goes when it answered.
    fn clear_recent(&mut self) {
        self.ask_recent(RecentWork::Clear);
    }

    /// A search the person ran: the engine keeps it as the newest (on a worker); the list shows it when it answered.
    fn remember(&mut self, query: &str) {
        let query = query.trim();
        if query.is_empty() {
            return;
        }
        self.ask_recent(RecentWork::Remember(query.to_string()));
    }

    /// Search opened: the recent searches are read when none of this session are held; the index is built when none
    /// of this session is held, or what is held is old.
    fn opened(&mut self) {
        self.follow();
        let recent_held = self.recent.as_ref().is_some_and(|(of, _)| Some(of) == self.of.as_ref());
        if !recent_held && self.recent_out.is_none() && self.recent_next.is_empty() {
            self.ask_recent(RecentWork::Read);
        }
        self.watching = self.of.is_some();
        // A refusal is never "fresh": opening Search builds again.
        let fresh = self.built_at.is_some_and(|at| at.elapsed() < FRESH_FOR) && !self.refused_for_now();
        if !fresh {
            self.build();
        }
    }

    /// The words or the pill changed: the query runs on a worker (now, or when the index lands).
    fn ask(&mut self, query: &str, narrow: Narrow) {
        self.follow();
        let query = query.trim();
        if query.is_empty() {
            self.wanted = None;
            return;
        }
        self.wanted = Some((query.to_string(), narrow));
        self.run_wanted();
    }

    /// The session moved on: everything held goes now, on the window's side. Then what the workers sent, each only
    /// for the session the window sees.
    fn pump(&mut self) -> Pumped {
        let mut changed = self.follow();
        while let Ok(landed) = self.inbox.try_recv() {
            self.take_in(landed);
            changed = true;
        }
        // A refusal that passes (the first sync, a list that could not be read) is built again after a short wait.
        let waiting = self.watching && self.building.is_none() && self.current().is_none() && self.refused_for_now();
        if waiting && self.built_at.is_none_or(|at| at.elapsed() >= self.retry_after) {
            self.build();
        }
        Pumped {
            changed,
            busy: self.building.is_some() || self.asked.is_some() || self.recent_out.is_some() || waiting,
        }
    }

    /// Every name and every search goes: the index, the last result, what was asked, the recent searches and the
    /// work on them that waits. A worker still out answers nobody.
    fn forget(&mut self) {
        self.held = None;
        self.found = None;
        self.asked = None;
        self.wanted = None;
        self.building = None;
        self.built_at = None;
        self.watching = false;
        self.refused = None;
        self.recent = None;
        self.recent_out = None;
        self.recent_next.clear();
    }

    /// The result of THIS query on the current session's index, else `Searching` while it runs. With no index of this
    /// session: `NotThisAccounts` when its build was refused as another account's local state, else `Unavailable`.
    /// Reads only: the query is started by `ask`, never here.
    fn search(&self, query: &str, narrow: Narrow) -> Answer {
        let Some(held) = self.current() else {
            // The first index of this session is being built: the query is answered when it lands.
            return if self.building_now() || self.getting_ready() {
                Answer::Searching
            } else if self.not_this_accounts() {
                // This PC's local state is another account's: Search says why; nothing of it was read.
                Answer::NotThisAccounts
            } else {
                Answer::Unavailable
            };
        };
        let query = query.trim();
        if query.is_empty() {
            return Answer::Found {
                hits: Vec::new(),
                loading: None,
            };
        }
        match &self.found {
            Some(((words, pill, number), hits)) if words == query && *pill == narrow && *number == held.number => {
                Answer::Found {
                    hits: hits.clone(),
                    loading: None,
                }
            }
            _ => Answer::Searching,
        }
    }

    fn details(&self, id: &str) -> Option<Details> {
        let held = self.current()?;
        let record = held.index.records.get(id).filter(|record| !record.shared_with_me)?;
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
