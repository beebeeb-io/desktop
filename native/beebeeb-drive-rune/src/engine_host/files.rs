//! The Files window's changes of what this PC holds, on the real engine: [`LocalCopies`], the part of
//! `EngineFilesSource` behind `keep_offline`, `free_up_space`, `changes` and `backups_root`.
//!
//! - Keep on this PC / Remove from this PC: the pin change (`Engine::set_kept`), one item at a time.
//! - Free up space: for a SELECTION (`Engine::free_up_space_of`), never the whole account. An item that was kept
//!   is released first (the window says "no longer kept"). A file with a change that is not sent is never touched
//!   (the engine's `local_bytes_agreed` guard, inside the reclaim).
//! - `changes`: what those two did, per item, once the engine answered. It is NOT an activity feed.
//! - The Backups folder's id, from this PC's local state (`StateDb::backups_roots`), read once per session.
//!
//! Thread rule: both changes block (the state database, a Cloud Files call), so one worker does them in the order
//! asked and wakes the window when an answer is there. The call itself only says "accepted" or why not.
//!
//! Whose answer (`engine_host/this_pc.rs` is the model): every job carries the account it was asked for and a
//! generation; an answer counts only when the account was the same before and after the work and the generation is
//! current. What is held (the in-flight marks, the answers not yet taken, the Backups id) is dropped on the
//! window's side the moment the gate or the account changes ([`LocalCopies::pump`], every frame).
//!
//! One change per item: while a change of an item is on its way, a second one of it sends nothing and says so.
//! Protected: nothing in the Backups tree is kept or freed from here, whatever the window asked.
//!
//! Must not: log a name, a path or an id; draw a change the engine did not make.

use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Duration;

use beebeeb_engine::app::{CredentialStores, Engine, LocalStateRefused};
use beebeeb_engine::backup_protect::BackupsTree;
use beebeeb_engine::signin::gate::GateState;
use beebeeb_engine::sync_view::Start;

use super::log;
use crate::frames::files::source::{Action, ActionError, Change};

#[cfg(test)]
#[path = "files_tests.rs"]
mod tests;

/// Whether the Backups folder's id is handed to the Files window in a real build. `false` until `protection`,
/// `backup_entry` and `folder_notice` answer too: with the id alone the sidebar's Backups would open the tree as a
/// plain folder, with no device rows and no words for why nothing in it can change. While it is `false` nothing is
/// read for it either.
pub const BACKUPS_SECTION_ON: bool = false;

/// How long a message of the window stays at most: the window is woken once more after it, so it goes.
const MESSAGE_STAYS: Duration = Duration::from_millis(8_100);

/// What the adapter asks of the engine handle. The app's is `Engine`; a test's is its own.
pub trait FilesEngine: Send + Sync + 'static {
    /// Someone is signed in and the vault is open. Never waits.
    fn unlocked(&self) -> bool;
    /// Whose account is signed in. Never waits.
    fn account(&self) -> Option<String>;
    /// An engine runs on the sync folder. Never waits (the view).
    fn running(&self) -> bool;
    /// Per id: is it in the Backups tree? `None`: no such row. Blocks: a worker's.
    fn in_backups(&self, ids: &[String]) -> Result<Vec<Option<bool>>, LocalStateRefused>;
    /// Keep an item on this PC, or stop keeping it. Blocks: a worker's.
    fn set_kept(&self, id: &str, kept: bool) -> Result<(), String>;
    /// The copies of these on this PC go; per id, whether nothing is left of it here. Blocks: a worker's.
    fn free_up_space_of(&self, ids: &[String]) -> Result<Vec<(String, bool)>, String>;
    /// The id of the Backups folder (`None`: this account has none here). Blocks: a worker's.
    fn backups_root(&self) -> Result<Option<String>, LocalStateRefused>;
    /// Redraw the window: an answer is there.
    fn wake(&self);
}

impl<C: CredentialStores> FilesEngine for Engine<C> {
    fn unlocked(&self) -> bool {
        self.view().gate == GateState::Ready
    }
    fn account(&self) -> Option<String> {
        self.account_email().or_else(|| self.user_id())
    }
    fn running(&self) -> bool {
        self.view().start == Start::Running
    }
    fn in_backups(&self, ids: &[String]) -> Result<Vec<Option<bool>>, LocalStateRefused> {
        Engine::local_state(self)?.with(|db| {
            let tree = BackupsTree::read(db);
            ids.iter()
                .map(|id| match db.get_file(id) {
                    Ok(Some(entry)) => Some(tree.below_root(&entry.path).is_some()),
                    _ => None,
                })
                .collect()
        })
    }
    fn set_kept(&self, id: &str, kept: bool) -> Result<(), String> {
        Engine::set_kept(self, id, kept)
    }
    fn free_up_space_of(&self, ids: &[String]) -> Result<Vec<(String, bool)>, String> {
        Engine::free_up_space_of(self, ids)
    }
    fn backups_root(&self) -> Result<Option<String>, LocalStateRefused> {
        Engine::local_state(self)?.with(|db| {
            db.backups_roots()
                .map(|roots| roots.into_iter().next().map(|(id, _)| id))
                .map_err(|_| LocalStateRefused::Unreadable)
        })?
    }
    fn wake(&self) {
        self.wake_host();
    }
}

enum Ask {
    /// These items (`true`: the row said it is kept), and what to do with them.
    Change(Action, Vec<(String, bool)>),
    BackupsRoot,
}

struct Job {
    ask: Ask,
    account: String,
    generation: u64,
}

enum Answer {
    /// A change: every id asked, the ones the engine changed, how many it did not and why (the last reason).
    Changed {
        generation: u64,
        whose: Option<String>,
        action: Action,
        asked: Vec<String>,
        done: Vec<String>,
        failed: usize,
        why: ActionError,
    },
    BackupsRoot {
        generation: u64,
        whose: Option<String>,
        root: Option<String>,
    },
}

/// See the module's doc.
pub struct LocalCopies<E: FilesEngine> {
    engine: Arc<E>,
    jobs: Option<mpsc::Sender<Job>>,
    answers: mpsc::Sender<Answer>,
    inbox: mpsc::Receiver<Answer>,
    pending: Arc<AtomicUsize>,
    /// The gate and the account the last look saw: a change drops everything held and starts a new generation.
    seen: (bool, Option<String>),
    generation: u64,
    /// The items a change is on its way for, in this generation.
    in_flight: HashSet<String>,
    /// What the engine answered and the window has not taken yet.
    changes: Vec<Change>,
    /// The Backups folder's id, read for `seen`'s account. The outer `None`: not read (or not readable).
    root: Option<Option<String>>,
}

impl<E: FilesEngine> LocalCopies<E> {
    /// Starts nothing and reads nothing (the tray makes a source of its own for one call).
    pub fn new(engine: Arc<E>) -> Self {
        let (answers, inbox) = mpsc::channel();
        Self {
            engine,
            jobs: None,
            answers,
            inbox,
            pending: Arc::default(),
            seen: (false, None),
            generation: 0,
            in_flight: HashSet::new(),
            changes: Vec::new(),
            root: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn idle(&self) -> bool {
        self.pending.load(Ordering::SeqCst) == 0
    }

    /// The gate or the account changed: nothing of the earlier session is kept, here, now.
    fn look(&mut self) -> bool {
        let now = (self.engine.unlocked(), self.engine.account());
        if now == self.seen {
            return false;
        }
        self.seen = now;
        self.generation += 1;
        self.in_flight.clear();
        self.changes.clear();
        self.root = None;
        if BACKUPS_SECTION_ON {
            self.send(Ask::BackupsRoot);
        }
        true
    }

    fn send(&mut self, ask: Ask) -> bool {
        let (true, Some(account)) = self.seen.clone() else {
            return false;
        };
        if self.jobs.is_none() {
            let (tx, rx) = mpsc::channel::<Job>();
            let (engine, answers, pending) = (self.engine.clone(), self.answers.clone(), self.pending.clone());
            let started = std::thread::Builder::new().name("files-local".into()).spawn(move || {
                // Ends when the source is dropped (its sender with it).
                for job in rx {
                    let answer = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(&*engine, &job))) {
                        Ok(answer) => answer,
                        Err(_) => {
                            log("files: a change of what this PC holds ended early");
                            gave_up(&job)
                        }
                    };
                    let _ = answers.send(answer);
                    pending.fetch_sub(1, Ordering::SeqCst);
                    engine.wake();
                }
            });
            match started {
                Ok(_) => self.jobs = Some(tx),
                Err(_) => {
                    log("files: could not start the worker");
                    return false;
                }
            }
        }
        self.pending.fetch_add(1, Ordering::SeqCst);
        let job = Job {
            ask,
            account,
            generation: self.generation,
        };
        let sent = self.jobs.as_ref().is_some_and(|tx| tx.send(job).is_ok());
        if !sent {
            self.pending.fetch_sub(1, Ordering::SeqCst);
        }
        sent
    }

    /// Ask for a change of these items. `Ok`: accepted; what it did arrives through [`Self::take`]. Refused, with
    /// nothing sent: no open vault, no engine on the folder, or a change of one of them is still on its way.
    pub fn ask(&mut self, action: Action, items: Vec<(String, bool)>) -> Result<(), ActionError> {
        self.look();
        if !self.seen.0 || self.seen.1.is_none() || !self.engine.running() {
            return Err(ActionError::Unavailable);
        }
        if items.iter().any(|(id, _)| self.in_flight.contains(id)) {
            return Err(ActionError::InFlight);
        }
        let ids: Vec<String> = items.iter().map(|(id, _)| id.clone()).collect();
        if !self.send(Ask::Change(action, items)) {
            return Err(ActionError::Failed);
        }
        self.in_flight.extend(ids);
        Ok(())
    }

    /// Every frame: a session change drops what is held; the worker's answers come in. `true`: something changed.
    pub fn pump(&mut self) -> bool {
        let mut changed = self.look();
        while let Ok(answer) = self.inbox.try_recv() {
            match answer {
                Answer::Changed {
                    generation,
                    whose,
                    action,
                    asked,
                    done,
                    failed,
                    why,
                } => {
                    // An answer of an earlier session: its marks went with that session, and it is not said.
                    if generation != self.generation {
                        continue;
                    }
                    for id in &asked {
                        self.in_flight.remove(id);
                    }
                    if whose.is_none() || whose != self.seen.1 {
                        continue;
                    }
                    changed = true;
                    self.changes.push(if done.is_empty() {
                        Change::Failed(action, why)
                    } else {
                        Change::Local { action, done, failed }
                    });
                    // The window's message about it goes after its time, also when nothing else wakes the window.
                    let engine = self.engine.clone();
                    let _ = std::thread::Builder::new().name("files-wake".into()).spawn(move || {
                        std::thread::sleep(MESSAGE_STAYS);
                        engine.wake();
                    });
                }
                Answer::BackupsRoot {
                    generation,
                    whose,
                    root,
                } => {
                    if generation == self.generation && whose.is_some() && whose == self.seen.1 {
                        self.root = Some(root);
                        changed = true;
                    }
                }
            }
        }
        changed
    }

    /// What the engine answered since the last call, oldest first.
    pub fn take(&mut self) -> Vec<Change> {
        self.pump();
        std::mem::take(&mut self.changes)
    }

    /// The Backups folder's id, while the vault is open for the account it was read for (and the section is on).
    pub fn backups_root(&self) -> Option<String> {
        if !BACKUPS_SECTION_ON || !self.engine.unlocked() {
            return None;
        }
        let account = self.engine.account();
        if account.is_none() || account != self.seen.1 {
            return None;
        }
        self.root.clone().flatten()
    }
}

fn still_for<E: FilesEngine>(engine: &E, account: &str) -> bool {
    engine.unlocked() && engine.account().as_deref() == Some(account)
}

/// The answer of a job that ended early: nothing is claimed.
fn gave_up(job: &Job) -> Answer {
    match &job.ask {
        Ask::Change(action, items) => Answer::Changed {
            generation: job.generation,
            whose: None,
            action: *action,
            asked: items.iter().map(|(id, _)| id.clone()).collect(),
            done: Vec::new(),
            failed: items.len(),
            why: ActionError::Failed,
        },
        Ask::BackupsRoot => Answer::BackupsRoot {
            generation: job.generation,
            whose: None,
            root: None,
        },
    }
}

/// One job, on the worker: for the account it was asked for alone, asked again before each item.
fn run<E: FilesEngine>(engine: &E, job: &Job) -> Answer {
    let (account, generation) = (job.account.as_str(), job.generation);
    let (action, items) = match &job.ask {
        Ask::BackupsRoot => {
            let root = if still_for(engine, account) {
                engine.backups_root().ok()
            } else {
                None
            };
            return match root {
                Some(root) => Answer::BackupsRoot {
                    generation,
                    whose: still_for(engine, account).then(|| account.to_string()),
                    root,
                },
                None => gave_up(job),
            };
        }
        Ask::Change(action, items) => (*action, items),
    };
    let asked: Vec<String> = items.iter().map(|(id, _)| id.clone()).collect();
    let mut done = Vec::new();
    let mut why = ActionError::Failed;
    // Nothing in the Backups tree is changed from here; an id no row has is not changed either.
    let places = if still_for(engine, account) {
        engine.in_backups(&asked).ok()
    } else {
        None
    };
    let mut free = Vec::new();
    for ((id, kept), place) in items.iter().zip(places.unwrap_or_default()) {
        if place == Some(true) {
            why = ActionError::Protected;
        }
        if place != Some(false) || !still_for(engine, account) {
            continue;
        }
        match action {
            Action::KeepOffline | Action::RemoveOffline => {
                if engine.set_kept(id, action == Action::KeepOffline).is_ok() {
                    done.push(id.clone());
                }
            }
            // What was kept is no longer kept (the window's words); then its copies may go.
            _ => {
                if !*kept || engine.set_kept(id, false).is_ok() {
                    free.push(id.clone());
                }
            }
        }
    }
    if !free.is_empty() && still_for(engine, account) {
        match engine.free_up_space_of(&free) {
            Ok(per_item) => done.extend(per_item.into_iter().filter(|(_, freed)| *freed).map(|(id, _)| id)),
            Err(_) => log("files: the selection's copies were not removed from this PC"),
        }
    }
    let failed = asked.len() - done.len();
    if failed > 0 {
        log(format!("files: {failed} of {} items were not changed", asked.len()));
    }
    Answer::Changed {
        generation,
        // The account must be the same after the work as before it.
        whose: still_for(engine, account).then(|| account.to_string()),
        action,
        asked,
        done,
        failed,
        why,
    }
}
