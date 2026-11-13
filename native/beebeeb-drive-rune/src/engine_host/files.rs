//! The Files window's changes of what this PC holds, on the real engine: [`LocalCopies`], the part of
//! `EngineFilesSource` behind `keep_offline`, `free_up_space`, `changes` and `backups_root`.
//!
//! - Keep on this PC / Remove from this PC: the pin change (`Engine::set_kept`), one item at a time.
//! - Free up space: for a SELECTION (`Engine::free_up_space_of`), never the whole account. Where this system has
//!   no reclaim for a selection, it is refused at once in its own words and NOTHING is changed. An item that was
//!   kept is released for the reclaim (which never touches a kept file) and KEPT AGAIN when the reclaim freed
//!   nothing of it: a Free up space that did nothing does not cost the pin. A folder that was freed IN PART stays
//!   released (keeping it again would download what was just freed) and is said as left, with the reason. Not kept
//!   again: when the vault locked in between (no session to do it in; logged). A file with a change that is not
//!   sent is never touched (the engine's guards, inside the reclaim).
//! - `changes`: what those two did, per item, once the engine answered: whether it was done, and what the engine
//!   says the item is NOW (read after the work, the way a listing reads it). It is NOT an activity feed.
//! - A job that ended early (a caught panic, a lock and an unlock in between) is never dropped and never trusted:
//!   one read of what the engine says of its items now decides, per item, whether the change is there.
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
use std::time::{Duration, Instant};

use beebeeb_engine::app::{CredentialStores, Engine, LocalStateRefused};
use beebeeb_engine::backup_protect::BackupsTree;
use beebeeb_engine::free_up_space::{Reclaimed, SelectionAnswer};
use beebeeb_engine::signin::gate::GateState;
use beebeeb_engine::sync_view::Start;

use super::log;
use crate::frames::files::source::{Action, ActionError, Change, LocalOutcome};

#[cfg(test)]
#[path = "files_tests.rs"]
mod tests;

/// Whether the Backups folder's id is handed to the Files window in a real build. `false` until `protection`,
/// `backup_entry` and `folder_notice` answer too: with the id alone the sidebar's Backups would open the tree as a
/// plain folder, with no device rows and no words for why nothing in it can change. While it is `false` nothing is
/// read for it either.
pub const BACKUPS_SECTION_ON: bool = false;

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
    /// Can this system free up space for a selection at all? Never waits.
    fn can_free_selection(&self) -> bool;
    /// The copies of these on this PC go; per id, what the engine says of it afterwards. Blocks: a worker's.
    fn free_up_space_of(&self, ids: &[String]) -> Result<Vec<SelectionAnswer>, String>;
    /// Per id: the row's mark of what this PC holds, as a listing reads it now (`Some(None)`: no mark). `None`: no
    /// such row, or it could not be read. Blocks: a worker's.
    fn states_of(&self, ids: &[String]) -> Result<Vec<Option<Option<String>>>, LocalStateRefused>;
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
    fn can_free_selection(&self) -> bool {
        beebeeb_engine::free_up_space::SELECTION_RECLAIM
    }
    fn free_up_space_of(&self, ids: &[String]) -> Result<Vec<SelectionAnswer>, String> {
        Engine::free_up_space_of(self, ids)
    }
    fn states_of(&self, ids: &[String]) -> Result<Vec<Option<Option<String>>>, LocalStateRefused> {
        Engine::local_state(self)?.with(|db| marks_in(db, ids))
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

/// Per id, the row's mark of what this PC holds as the rows say now, by the listing's own two steps (the engine's
/// row model, then the window's mark of it), with one difference: a file that is KEPT reads "available_offline"
/// also while its download has not finished. The listing's model has no word for that and says "online only",
/// which would leave the menu offering Keep on this PC for an item that is kept, and never Remove from this PC.
/// `None`: no such row, or it could not be read.
fn marks_in(db: &beebeeb_engine::state_db::StateDb, ids: &[String]) -> Vec<Option<Option<String>>> {
    use beebeeb_engine::state_db::PinState;
    ids.iter()
        .map(|id| {
            let entry = db.get_file(id).ok().flatten()?;
            let contract = db.get_file_contract_state(id).ok()?;
            let kept = contract
                .as_ref()
                .is_some_and(|contract| contract.effective_pin_state() == PinState::Pinned);
            if kept && !entry.is_dir() {
                return Some(Some("available_offline".to_string()));
            }
            let row = beebeeb_engine::drive_v2::model::item_from(&entry, contract.as_ref(), None);
            Some(crate::source_engine::item(row).pc_status)
        })
        .collect()
}

/// Is what the action asked for there, by the mark the engine gives the item now? For a job whose own word is not
/// trusted (it ended early). A folder's mark does not say whether its copies are gone: Free up space of it is not
/// called done from here.
fn reached(action: Action, now: &Option<Option<String>>) -> bool {
    let Some(mark) = now else { return false };
    match action {
        Action::KeepOffline => mark.as_deref() == Some("available_offline"),
        Action::RemoveOffline => mark.as_deref() != Some("available_offline"),
        _ => mark.as_deref() == Some("online_only"),
    }
}

enum Ask {
    /// These items (`true`: the row said it is kept), and what to do with them.
    Change(Action, Vec<(String, bool)>),
    /// After a change that ended early: what the engine says of these now. Nothing is changed.
    Look(Action, Vec<String>),
    BackupsRoot,
}

struct Job {
    ask: Ask,
    account: String,
    generation: u64,
}

enum Answer {
    /// A change, per item asked: what the engine says of it now, and why it was left if it was.
    Changed {
        generation: u64,
        /// The account it was asked for (not "the account at the end": that is `ended`).
        account: String,
        /// The job did not end the way it began: a panic was caught, or the vault was not open for this account
        /// when it finished. What it says of the items is not trusted.
        ended: bool,
        /// The answer of a [`Ask::Look`].
        looked: bool,
        action: Action,
        items: Vec<LocalOutcome>,
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
    /// The one timer that wakes the window when a message's time is over ([`Self::wake_after`]).
    wakes: Option<mpsc::Sender<Instant>>,
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
            wakes: None,
        }
    }

    /// Wake the window once more after this long (a message set between presses goes at its own time). One
    /// thread for all of them, started at the first; it ends with the source.
    pub fn wake_after(&mut self, after: Duration) {
        if self.wakes.is_none() {
            let (tx, rx) = mpsc::channel::<Instant>();
            let engine = self.engine.clone();
            let started = std::thread::Builder::new()
                .name("files-wake".into())
                .spawn(move || wake_when_due(&rx, || engine.wake()));
            if started.is_ok() {
                self.wakes = Some(tx);
            }
        }
        if let Some(wakes) = &self.wakes {
            let _ = wakes.send(Instant::now() + after);
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
    /// nothing sent: no open vault, no engine on the folder, a change of one of them is still on its way, or (Free
    /// up space) this system has no reclaim for a selection.
    pub fn ask(&mut self, action: Action, items: Vec<(String, bool)>) -> Result<(), ActionError> {
        self.look();
        if !self.seen.0 || self.seen.1.is_none() || !self.engine.running() {
            return Err(ActionError::Unavailable);
        }
        // Before anything is released for it: a reclaim that cannot run here must not cost a pin.
        if action == Action::FreeUpSpace && !self.engine.can_free_selection() {
            return Err(ActionError::NotOnThisSystem);
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
                    account,
                    ended,
                    looked,
                    action,
                    items,
                } => {
                    // An answer of an earlier session: its marks went with that session, and it is not said.
                    if generation != self.generation || Some(&account) != self.seen.1.as_ref() {
                        continue;
                    }
                    if !looked {
                        for item in &items {
                            self.in_flight.remove(&item.id);
                        }
                    }
                    // It was accepted in THIS session and ended early: never dropped, and its own word is not trusted.
                    // One read of what the engine says of the items now decides per item.
                    if ended && !looked {
                        let ids = items.iter().map(|item| item.id.clone()).collect();
                        if self.send(Ask::Look(action, ids)) {
                            continue;
                        }
                    }
                    changed = true;
                    self.changes.push(Change::Local { action, items });
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

/// The timer behind [`LocalCopies::wake_after`]: `wake` at each moment sent, until the sender is gone.
fn wake_when_due(due: &mpsc::Receiver<Instant>, wake: impl Fn()) {
    let mut waiting: Vec<Instant> = Vec::new();
    loop {
        let got = match waiting.iter().min() {
            None => due.recv().map_err(|_| mpsc::RecvTimeoutError::Disconnected),
            Some(next) => due.recv_timeout(next.saturating_duration_since(Instant::now())),
        };
        match got {
            Ok(at) => waiting.push(at),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let now = Instant::now();
                waiting.retain(|at| *at > now);
                wake();
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
    }
}

/// Every item of a job as "not done, and nothing known of it".
fn all_left(ids: impl Iterator<Item = String>) -> Vec<LocalOutcome> {
    ids.map(|id| LocalOutcome {
        id,
        now: None,
        left: Some(ActionError::Failed),
    })
    .collect()
}

/// The answer of a job that ended early: nothing is claimed.
fn gave_up(job: &Job) -> Answer {
    let (action, ids, looked): (Action, Vec<String>, bool) = match &job.ask {
        Ask::Change(action, items) => (*action, items.iter().map(|(id, _)| id.clone()).collect(), false),
        Ask::Look(action, ids) => (*action, ids.clone(), true),
        Ask::BackupsRoot => {
            return Answer::BackupsRoot {
                generation: job.generation,
                whose: None,
                root: None,
            };
        }
    };
    Answer::Changed {
        generation: job.generation,
        account: job.account.clone(),
        ended: true,
        looked,
        action,
        items: all_left(ids.into_iter()),
    }
}

/// What the engine says of these now, for this account alone. `None` per id: not known.
fn states<E: FilesEngine>(engine: &E, account: &str, ids: &[String]) -> Vec<Option<Option<String>>> {
    let read = if still_for(engine, account) {
        engine.states_of(ids).ok()
    } else {
        None
    };
    read.filter(|read| read.len() == ids.len())
        .unwrap_or_else(|| vec![None; ids.len()])
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
        // A change ended early: per item, done only where what the engine says of it now is what was asked.
        Ask::Look(action, ids) => {
            let now = states(engine, account, ids);
            let mut items = all_left(ids.iter().cloned());
            for (item, now) in items.iter_mut().zip(now) {
                if reached(*action, &now) {
                    item.left = None;
                }
                item.now = now;
            }
            return Answer::Changed {
                generation,
                account: account.to_string(),
                ended: !still_for(engine, account),
                looked: true,
                action: *action,
                items,
            };
        }
        Ask::Change(action, items) => (*action, items),
    };
    let asked: Vec<String> = items.iter().map(|(id, _)| id.clone()).collect();
    // Per item: why it was left. Not done until the engine said so.
    let mut left: Vec<Option<ActionError>> = vec![Some(ActionError::Failed); asked.len()];
    // Nothing in the Backups tree is changed from here; an id no row has is not changed either.
    let places = if still_for(engine, account) {
        engine.in_backups(&asked).ok()
    } else {
        None
    };
    let places = places
        .filter(|places| places.len() == asked.len())
        .unwrap_or_else(|| vec![None; asked.len()]);
    // Free up space: the items to reclaim, and the ones of them that were released for it.
    let (mut free, mut released) = (Vec::new(), Vec::new());
    for (at, ((id, kept), place)) in items.iter().zip(places).enumerate() {
        if place == Some(true) {
            left[at] = Some(ActionError::Protected);
        }
        if place != Some(false) || !still_for(engine, account) {
            continue;
        }
        match action {
            Action::KeepOffline | Action::RemoveOffline => {
                if engine.set_kept(id, action == Action::KeepOffline).is_ok() {
                    left[at] = None;
                }
            }
            // The reclaim never touches a kept file: what was kept is released for it, and kept again below
            // when the reclaim did not free it.
            _ => {
                if *kept {
                    if engine.set_kept(id, false).is_err() {
                        continue;
                    }
                    released.push(at);
                }
                free.push(at);
            }
        }
    }
    // Per item: did the reclaim free something of it? What was freed in part is not kept again.
    let mut freed_some = vec![false; asked.len()];
    // The reclaim panicked: what it did is not known, so the answer is not trusted (`ended`).
    let mut reclaim_ended = false;
    if !free.is_empty() && still_for(engine, account) {
        let ids: Vec<String> = free.iter().map(|&at| asked[at].clone()).collect();
        // Caught here, not only around the job: the pins released above are still to be put back.
        let reclaim = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| engine.free_up_space_of(&ids)));
        match reclaim {
            Ok(Ok(per_item)) => {
                for answer in per_item {
                    let Some(at) = free.iter().copied().find(|&at| asked[at] == answer.id) else {
                        continue;
                    };
                    freed_some[at] = answer.freed_some;
                    left[at] = match answer.what {
                        Reclaimed::Freed => None,
                        Reclaimed::Unsent => Some(ActionError::NotSent),
                        Reclaimed::Kept => Some(ActionError::HoldsKept),
                        Reclaimed::NoSuchItem | Reclaimed::Left => Some(ActionError::Failed),
                    };
                }
            }
            Ok(Err(_)) => log("files: the selection's copies were not removed from this PC"),
            Err(_) => {
                log("files: the reclaim of a selection ended early");
                reclaim_ended = true;
            }
        }
    }
    // A Free up space that freed nothing of an item does not cost its pin: it is kept again. Not what was freed in
    // part: keeping a folder again would download what was just freed.
    for at in released {
        if left[at].is_none() || freed_some[at] {
            continue;
        }
        if !still_for(engine, account) {
            log("files: an item released for Free up space was not kept again (the vault is no longer open)");
        } else if engine.set_kept(&asked[at], true).is_err() {
            log("files: an item released for Free up space could not be kept again");
        }
    }
    let failed = left.iter().filter(|left| left.is_some()).count();
    if failed > 0 {
        log(format!("files: {failed} of {} items were not changed", asked.len()));
    }
    // What the rows read now comes from the engine, after the work: changed or left, never from the action.
    let now = states(engine, account, &asked);
    Answer::Changed {
        generation,
        account: account.to_string(),
        // The account must be the same after the work as before it.
        ended: reclaim_ended || !still_for(engine, account),
        looked: false,
        action,
        items: asked
            .into_iter()
            .zip(now)
            .zip(left)
            .map(|((id, now), left)| LocalOutcome { id, now, left })
            .collect(),
    }
}
