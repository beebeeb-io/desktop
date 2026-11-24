//! Backups on the real engine: [`EngineBackupsSource`], the `BackupsSource` of the running app (DW-0737), and
//! [`may_back_up`], the one rule that needs the app's identity.
//!
//! What it answers, and from where (all of it this PC's own, nothing asked of Beebeeb):
//! - The Backups root and this PC's folder in it: the backup layout in the signed-in account's local state
//!   (`Engine::backups_overview`). Protected always: the layout's root IS the Backups root the engine protects;
//!   before a layout exists the root is still drawn protected (fail closed), with no id.
//! - The folders: Windows' six known folders, their switch from this PC's settings file, the stop and the last
//!   pass from the local state, a bounded count of the files (`Engine::backups_overview`). A folder this PC does not
//!   have is left out (a switch for it would back up nothing).
//! - A folder's switch: `Engine::set_backup_folder` (the 0.8 app's switch: off purges the folder's pending uploads
//!   and deletes nothing in Beebeeb; on clears a stop, "Back up again").
//! - The status card: from the runner's own flags (`scheduler::pass_running`, a waiting "Back up now") and the last
//!   pass of each folder that is on. It says "All backed up" only when every folder that is on has a last pass and
//!   none stopped; otherwise it says what it knows in words ([`Status::Plain`]). No count of the files, no size.
//! - "Back up now": `scheduler::request_pass_now`, which the runner's next tick honours; only while a folder is on.
//! - What could not back up: the folders that turned themselves off, with core's words for why (the engine keeps no
//!   list of single files it gave up on). "Try again" switches the folder on again (clears the stop).
//! - Other devices and Delete this backup: `engine_host::backups_devices` (`DeviceCalls`: the engine's session-bound
//!   read of the device manifests, and the delete behind the account password).
//! - A folder the person chose (DW-0737 "chosen by you"): listed after the six from the same read; Add a folder…
//!   (DW-0738) and Stop backing up (DW-1410) are in `engine_host/backup_folders.rs`.
//! - Not bound (`wired(part)` is `false`): Mail Backup (a proposal), and a device page's "Open in Photos" / "Open in
//!   Files" (the shell takes Backups' folder picker only).
//!
//! Thread rule: nothing here waits on the window's thread. One worker per source does every read and every switch,
//! in the order asked, and sends each answer down one channel, which [`BackupsSource::pump`] drains.
//!
//! Whose answer (the review of desktop#377; the model is `engine_host/this_pc.rs`): every job carries the account it
//! was asked for and a generation; an answer counts only when the account was the same before and after the work
//! and the generation is still current. The window drops what it holds the moment the gate or the account changes
//! (in `pump`), and every reader hands out nothing unless the vault is open and the signed-in account is the one the
//! answer was read for. The local state is read only through the engine's account-bound handle (task 2488).
//!
//! A switch shows what was asked for until the engine's read of it lands, so a second press asks for the opposite
//! (on, then off again before the first read is not dropped). A switch the engine refuses shows the engine's own
//! value again, with the section's fixed sentence.
//!
//! Must not: log a name, a path or an id; show a value it has not read; switch a folder for another account.

use std::collections::BTreeMap;
use std::path::{Component, Path};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use beebeeb_engine::app::backups_api::{BackupFolderRow, BackupsError, BackupsOverview, FolderState};
use beebeeb_engine::app::{CredentialStores, Engine};
use beebeeb_engine::identity::EngineIdentity;
use beebeeb_engine::signin::gate::GateState;

use super::backups_devices::{DeviceCalls, DevicesEngine};
use super::log;
use crate::frames::backups::source::{
    BackupsRoot, BackupsSource, Device, FileKind, Folder, NotBackedUp, Part, Pumped, Status,
};

#[cfg(test)]
#[path = "backups_tests.rs"]
mod tests;
// Add a folder and Stop backing up.
#[path = "backup_folders.rs"]
mod backup_folders;

/// Whether Backups answers in a real build. On: the view draws a loading state, a refusal and only the parts a
/// source binds (`BackupsSource::wired`), and the two ledger items of the Backups switch (2468: the settings file
/// written before the purge; on-then-off dropped) are closed. While it is `false` [`source_for`] hands the shell no
/// source.
pub const TURNED_ON: bool = true;

/// The source the running app hands Backups: the engine's while [`TURNED_ON`], else none (the "not built yet"
/// frame). A constant that turns the view off turns the source off too (review of desktop#377, P2-1).
pub fn source_for<E: BackupsEngine>(engine: Arc<E>) -> Option<Box<dyn BackupsSource>> {
    TURNED_ON.then(|| Box::new(EngineBackupsSource::new(engine)) as Box<dyn BackupsSource>)
}

/// An answer read this recently is not read again when the section opens.
pub const FRESH_FOR: Duration = Duration::from_secs(30);

/// What the section says when this PC's local state is another account's.
pub const NOT_YOURS: &str = concat!(beebeeb_engine::device_word_cap!(), " holds another account's files. Backups shows none of them.");
/// What the section says when this PC's backup could not be read.
pub const NOT_READ: &str = concat!("Couldn't read ", beebeeb_engine::device_word!(), "'s backup. Try again in a moment.");
/// What the section says when a switch did not happen (the list then shows what the engine has).
pub const NOT_SWITCHED: &str = "Couldn't change this folder's backup.";

/// The engine handle as the adapter uses it. The real one is [`Engine`]; tests pass a fake.
pub trait BackupsEngine: DevicesEngine + Send + Sync + 'static {
    /// Someone is signed in and the vault is open. Never waits.
    fn unlocked(&self) -> bool;
    /// Whose account is signed in (the stored email, else the user id the server named). Never waits.
    fn account(&self) -> Option<String>;
    /// This PC's backup for the signed-in account (`Engine::backups_overview`). BLOCKS: a worker's.
    fn overview(&self) -> Result<BackupsOverview, BackupsError>;
    /// Switch one known folder (`Engine::set_backup_folder`). BLOCKS: a worker's.
    fn set_folder(&self, key: &str, on: bool) -> Result<(), BackupsError>;
    /// The runner is passing the backed-up folders now. Never waits.
    fn pass_running(&self) -> bool;
    /// A "Back up now" is waiting for the runner's next tick. Never waits.
    fn pass_requested(&self) -> bool;
    /// Ask the runner for a pass now (`scheduler::request_pass_now`). Never waits. Does nothing while sync is not
    /// running: no runner would ever clear the ask.
    fn request_pass(&self);
    /// Sync runs now (a runner exists to take a pass). Never waits.
    fn sync_running(&self) -> bool {
        true
    }
    /// The engine's words for a switch refused because it would back a folder up twice (`BackupsError::Nested`), naming
    /// the other folder (`Engine::switch_on_refused`, P3-3). `None`: nothing nests with it now. BLOCKS: a worker's. The
    /// default has none.
    fn nested_words(&self, _key: &str) -> Option<String> {
        None
    }
    /// Add a chosen folder for `session` (`Engine::add_backup_folder_for`). BLOCKS: a worker's. The default
    /// adds nothing.
    fn add_folder_for(
        &self,
        _session: beebeeb_engine::account::SessionGeneration,
        _path: &str,
    ) -> Result<(), beebeeb_engine::app::bindings::backup_folders::AddRefused> {
        Err(beebeeb_engine::app::bindings::backup_folders::AddRefused::Backups(
            BackupsError::Settings,
        ))
    }
    /// Stop backing up a chosen folder for `session`, its backup to Trash after the password is proven when one is
    /// given (`Engine::remove_backup_folder_for`). BLOCKS: a worker's. The default stops nothing.
    fn remove_folder_for(
        &self,
        _session: beebeeb_engine::account::SessionGeneration,
        _id: &str,
        _trash_password: Option<&[u8]>,
    ) -> Result<(), beebeeb_engine::app::bindings::backup_folders::RemoveRefused> {
        Err(beebeeb_engine::app::bindings::backup_folders::RemoveRefused::Backups(
            BackupsError::Settings,
        ))
    }
}

impl<C: CredentialStores> BackupsEngine for Engine<C> {
    fn unlocked(&self) -> bool {
        self.view().gate == GateState::Ready
    }
    fn account(&self) -> Option<String> {
        self.account_email().or_else(|| self.user_id())
    }
    fn overview(&self) -> Result<BackupsOverview, BackupsError> {
        self.backups_overview()
    }
    fn set_folder(&self, key: &str, on: bool) -> Result<(), BackupsError> {
        self.set_backup_folder(key, on)
    }
    fn pass_running(&self) -> bool {
        self.sync_running() && beebeeb_engine::backup::scheduler::pass_running()
    }
    fn pass_requested(&self) -> bool {
        self.sync_running() && beebeeb_engine::backup::scheduler::pass_requested()
    }
    fn request_pass(&self) {
        if self.sync_running() {
            beebeeb_engine::backup::scheduler::request_pass_now();
        }
    }
    fn sync_running(&self) -> bool {
        self.view().start == beebeeb_engine::sync_view::Start::Running
    }
    fn nested_words(&self, key: &str) -> Option<String> {
        self.switch_on_refused(key).map(|refused| refused.words())
    }
    fn add_folder_for(
        &self,
        session: beebeeb_engine::account::SessionGeneration,
        path: &str,
    ) -> Result<(), beebeeb_engine::app::bindings::backup_folders::AddRefused> {
        self.add_backup_folder_for(session, Path::new(path)).map(|_| ())
    }
    fn remove_folder_for(
        &self,
        session: beebeeb_engine::account::SessionGeneration,
        id: &str,
        trash_password: Option<&[u8]>,
    ) -> Result<(), beebeeb_engine::app::bindings::backup_folders::RemoveRefused> {
        self.remove_backup_folder_for(session, id, trash_password)
    }
}

enum Ask {
    Read,
    Switch(String, bool),
    /// "Back up now": asks the runner for a pass, then reads.
    RunNow,
}

/// One job: what is asked, for whose account, in which generation.
struct Job {
    ask: Ask,
    account: String,
    generation: u64,
}

enum Answer {
    /// A read: the generation it was asked in, the account it is for (`None`: the account changed, or the vault
    /// closed, during the work: never shown), and what was read.
    Read {
        generation: u64,
        whose: Option<String>,
        read: Result<BackupsOverview, BackupsError>,
        /// The folder whose switch this read follows (every switch job ends with exactly one read).
        switched: Option<String>,
    },
    /// A switch that did not happen, and what the window says: [`NOT_SWITCHED`], or the engine's words naming the
    /// folder it would back up twice (P3-3).
    Refused { generation: u64, words: &'static str },
}

/// Backups on the real engine. See the module's doc.
pub struct EngineBackupsSource<E: BackupsEngine> {
    engine: Arc<E>,
    jobs: Option<mpsc::Sender<Job>>,
    answers: mpsc::Sender<Answer>,
    inbox: mpsc::Receiver<Answer>,
    /// Jobs asked and not finished.
    pending: Arc<AtomicUsize>,
    /// What the window holds, read for `seen`'s account (or nothing).
    held: Option<BackupsOverview>,
    /// Why nothing (or an older answer) is shown, for `seen`'s account.
    refused: Option<&'static str>,
    /// The gate and the account the last `pump` saw: a change drops `held` and starts a new generation.
    seen: (bool, Option<String>),
    generation: u64,
    read_at: Option<Instant>,
    /// A read is out in this generation: no second one is asked.
    reading: bool,
    /// Switches asked and not yet read back, per folder: the value asked for last and how many are out. A folder
    /// shows, and the next press compares with, what was asked for last, not the last read (on, then off again
    /// before the first read lands: both are sent, in order).
    switching: BTreeMap<String, (bool, u32)>,
    /// What `pass_running` said at the last `pump`: a pass that just ended is read again (its time changed).
    was_running: bool,
    /// A "Back up now" was sent and its read has not come back: no second one is sent.
    run_out: bool,
    /// Other devices and Delete this backup (`engine_host::backups_devices`).
    devices: DeviceCalls<E>,
    /// `backup_folders.rs`: what it keeps between a press and its answer. Empty until it binds.
    backup_folders: backup_folders::BackupFolderCalls,
}

impl<E: BackupsEngine> EngineBackupsSource<E> {
    /// Starts nothing: the first `pump` that sees an open vault asks the first read.
    pub fn new(engine: Arc<E>) -> Self {
        let (answers, inbox) = mpsc::channel();
        Self {
            devices: DeviceCalls::new(engine.clone()),
            engine,
            jobs: None,
            answers,
            inbox,
            pending: Arc::default(),
            held: None,
            refused: None,
            seen: (false, None),
            generation: 0,
            read_at: None,
            reading: false,
            switching: BTreeMap::new(),
            was_running: false,
            run_out: false,
            backup_folders: Default::default(),
        }
    }

    /// No job is waiting or running (tests wait on it).
    #[cfg(test)]
    pub(crate) fn idle(&self) -> bool {
        self.pending.load(Ordering::SeqCst) == 0 && self.devices.idle()
    }

    /// Send `ask` for the account the window sees now. Nothing is sent while no one is signed in with the vault open.
    fn send(&mut self, ask: Ask) -> bool {
        let (true, Some(account)) = self.seen.clone() else {
            return false;
        };
        if self.jobs.is_none() {
            let (tx, rx) = mpsc::channel::<Job>();
            let (engine, answers, pending) = (self.engine.clone(), self.answers.clone(), self.pending.clone());
            let started = std::thread::Builder::new().name("backups".into()).spawn(move || {
                // Ends when the source is dropped (its sender with it).
                for job in rx {
                    let generation = job.generation;
                    let switched = match &job.ask {
                        Ask::Switch(key, _) => Some(key.clone()),
                        _ => None,
                    };
                    let done = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(&*engine, &answers, job)));
                    if done.is_err() {
                        log("backups: a read or a switch ended early");
                        let _ = answers.send(Answer::Read {
                            generation,
                            whose: None,
                            read: Err(BackupsError::State),
                            switched,
                        });
                    }
                    pending.fetch_sub(1, Ordering::SeqCst);
                }
            });
            match started {
                Ok(_) => self.jobs = Some(tx),
                Err(_) => {
                    log("backups: could not start its worker");
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

    fn ask_read(&mut self) {
        if !self.reading {
            self.reading = self.send(Ask::Read);
        }
    }

    /// Whether the window may show what it holds: the vault is open and the signed-in account is the one it was
    /// read for. Asked of the engine on every read, never only on `pump`.
    fn current(&self) -> bool {
        if !self.engine.unlocked() {
            return false;
        }
        let account = self.engine.account();
        account.is_some() && account == self.seen.1
    }

    fn held(&self) -> Option<&BackupsOverview> {
        self.held.as_ref().filter(|_| self.current())
    }

    /// The switch of `row` as the window shows it and the next press compares with: the last one asked for, else the
    /// last one read.
    fn effective_on(&self, row: &BackupFolderRow) -> bool {
        self.switching.get(row.key.as_str()).map_or(row.on, |(want, _)| *want)
    }

    /// The rows the section draws: the folders this PC has, and every folder the person chose (one gone from this PC
    /// stays, so it can be removed).
    fn rows(&self) -> impl Iterator<Item = &BackupFolderRow> {
        self.held()
            .into_iter()
            .flat_map(|overview| overview.folders.iter())
            .filter(|row| row.state != FolderState::Unavailable || row.chosen_by_you())
    }
}

/// The account `account` is still the signed-in one with the vault open.
fn still_for<E: BackupsEngine>(engine: &E, account: &str) -> bool {
    engine.unlocked() && engine.account().as_deref() == Some(account)
}

/// One job, on the worker. A switch runs only for the account it was asked for; every job ends with a read, so
/// the list shows what the engine has after it.
/// `words` as the window's `&'static str` (its refusals are fixed sentences): each distinct sentence is kept once for the
/// life of the process, so what is kept is bounded by the folder names the engine's refusals name (P3-3; never a path).
fn kept(words: String) -> &'static str {
    static KEPT: std::sync::Mutex<std::collections::BTreeSet<&'static str>> =
        std::sync::Mutex::new(std::collections::BTreeSet::new());
    let mut held = KEPT.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(found) = held.get(words.as_str()) {
        return found;
    }
    let words: &'static str = Box::leak(words.into_boxed_str());
    held.insert(words);
    words
}

fn run<E: BackupsEngine>(engine: &E, answers: &mpsc::Sender<Answer>, job: Job) {
    let Job {
        ask,
        account,
        generation,
    } = job;
    let switched_key = match &ask {
        Ask::Switch(key, _) => Some(key.clone()),
        _ => None,
    };
    if matches!(ask, Ask::RunNow) && still_for(engine, &account) {
        engine.request_pass();
    }
    if let Ask::Switch(key, on) = ask {
        let switched = if still_for(engine, &account) {
            engine.set_folder(&key, on)
        } else {
            Err(BackupsError::Refused(
                beebeeb_engine::app::LocalStateRefused::SessionChanged,
            ))
        };
        if let Err(error) = switched {
            log(format!("backups: a folder was not switched ({})", error.kind()));
            // P3-3: a switch that would back a folder up twice names that folder, in the engine's words, while the
            // account is still the one asked for.
            let named = match error {
                BackupsError::Nested => engine.nested_words(&key).filter(|_| still_for(engine, &account)),
                _ => None,
            };
            let words = named.map_or(NOT_SWITCHED, kept);
            let _ = answers.send(Answer::Refused { generation, words });
        }
    }
    let read = if still_for(engine, &account) {
        engine.overview()
    } else {
        Err(BackupsError::Refused(
            beebeeb_engine::app::LocalStateRefused::SessionChanged,
        ))
    };
    // The account must be the same after the work as before it: an answer that outlived its session is dropped.
    let whose = still_for(engine, &account).then_some(account);
    let _ = answers.send(Answer::Read {
        generation,
        whose,
        read,
        switched: switched_key,
    });
}

/// "Today, 09:30", "12 Nov, 09:30" in this PC's time zone, from a pass's time (server clock, milliseconds).
pub(crate) fn last_backup_words(at_ms: i64, today: chrono::NaiveDate) -> String {
    use chrono::TimeZone;
    match chrono::Local.timestamp_millis_opt(at_ms).single() {
        Some(then) if then.date_naive() == today => format!("Today, {}", then.format("%H:%M")),
        Some(then) => then.format("%-d %b, %H:%M").to_string(),
        None => String::new(),
    }
}

fn folder_of(row: &BackupFolderRow, today: chrono::NaiveDate) -> Folder {
    Folder {
        id: row.key.clone(),
        name: row.name.clone(),
        // Not counted (inside OneDrive's folder, or not on Windows): 0, and the PR names the trait's gap.
        items: row.items.map_or(0, |n| u32::try_from(n).unwrap_or(u32::MAX)),
        // The engine has no size of a backed-up folder (PR body): never a guess.
        size: String::new(),
        chosen_by_you: row.chosen_by_you(),
        on: row.on,
        last_backup: row
            .last_pass_at_ms
            .filter(|_| row.state == FolderState::On)
            .map(|at| last_backup_words(at, today)),
    }
}

impl<E: BackupsEngine> BackupsSource for EngineBackupsSource<E> {
    fn wired(&self, part: Part) -> bool {
        // Not bound: Mail Backup (a proposal) and a device page's links (the shell takes Backups' folder picker only).
        matches!(
            part,
            Part::Root
                | Part::Folders
                | Part::Status
                | Part::NotBackedUp
                | Part::BackUpNow
                | Part::AddFolder
                | Part::RemoveFolder
                | Part::Retry
                | Part::Devices
                | Part::DeleteDevice
        )
    }

    /// The gate or the account changed: what the window held goes now, and an open vault is read again. Then the
    /// worker's answers: a current one replaces what is held, a refusal is said once.
    fn pump(&mut self) -> Pumped {
        let mut pumped = Pumped {
            busy: self.pending.load(Ordering::SeqCst) > 0,
            ..Pumped::default()
        };
        let now = (self.engine.unlocked(), self.engine.account());
        if now != self.seen {
            self.seen = now;
            self.held = None;
            self.refused = None;
            self.read_at = None;
            self.reading = false;
            self.switching.clear();
            self.run_out = false;
            self.generation += 1;
            pumped.changed = true;
            self.ask_read();
        }
        // The runner's own pass: the card follows it, and a pass that just ended is read again (its time moved).
        let running = self.engine.pass_running();
        if running != self.was_running {
            self.was_running = running;
            pumped.changed = true;
            if !running && self.seen.0 {
                self.ask_read();
            }
        }
        pumped.busy |= running || self.engine.pass_requested();
        while let Ok(answer) = self.inbox.try_recv() {
            match answer {
                Answer::Read {
                    generation,
                    whose,
                    read,
                    switched,
                } => {
                    if generation != self.generation {
                        continue;
                    }
                    self.reading = false;
                    self.run_out = false;
                    pumped.changed = true;
                    // The switch this read follows is read back: what it asked for is now what the engine has.
                    if let Some(key) = switched
                        && let Some((_, out)) = self.switching.get_mut(&key)
                    {
                        *out = out.saturating_sub(1);
                        if *out == 0 {
                            self.switching.remove(&key);
                        }
                    }
                    let mine = self.seen.0 && whose.is_some() && whose == self.seen.1;
                    if !mine {
                        continue;
                    }
                    self.read_at = Some(Instant::now());
                    match read {
                        Ok(overview) => {
                            self.held = Some(overview);
                            self.refused = None;
                        }
                        // Another account's local state: nothing of it, and the section says why.
                        Err(BackupsError::Refused(refused)) if refused.not_this_accounts() => {
                            log(format!("backups: not read ({})", refused.kind()));
                            self.held = None;
                            self.refused = Some(NOT_YOURS);
                        }
                        // Could not be read: what is held of this account stays, and the section says so.
                        Err(error) => {
                            log(format!("backups: not read ({})", error.kind()));
                            self.refused = Some(NOT_READ);
                        }
                    }
                }
                Answer::Refused { generation, words } => {
                    if generation == self.generation {
                        pumped.refusal = Some(words);
                        pumped.changed = true;
                    }
                }
            }
        }
        pumped.busy |= self.pending.load(Ordering::SeqCst) > 0;
        self.backup_folders_pump(&mut pumped);
        let devices = self.devices.pump();
        pumped.changed |= devices.changed;
        pumped.busy |= devices.busy;
        pumped.deleted = devices.deleted;
        pumped
    }

    /// The section opened: read again when what is held is older than [`FRESH_FOR`].
    fn opened(&mut self) {
        let fresh = self.read_at.is_some_and(|at| at.elapsed() < FRESH_FOR) && self.held().is_some();
        if !fresh {
            self.ask_read();
        }
        self.devices.opened();
    }

    fn read_again(&mut self) {
        self.refused = None;
        self.ask_read();
        self.devices.opened();
    }

    fn devices_note(&self) -> Option<&'static str> {
        self.devices.note()
    }

    fn deleting(&self) -> bool {
        self.devices.deleting()
    }

    fn loading(&self) -> bool {
        self.reading && self.held().is_none()
    }

    fn refusal(&self) -> Option<&'static str> {
        self.refused.filter(|_| self.current())
    }

    /// Protected always (module doc); the id and this PC's folder only while the layout is read for this account.
    fn root(&self) -> BackupsRoot {
        match self.held().and_then(|overview| overview.root.as_ref()) {
            Some(root) => BackupsRoot {
                id: root.root_id.clone(),
                is_protected: true,
                device_folder: root.device_folder.clone(),
            },
            None => BackupsRoot {
                id: String::new(),
                is_protected: true,
                device_folder: String::new(),
            },
        }
    }

    /// From what is held and the runner's own flags (`scheduler::pass_running`, a waiting "Back up now"): never a
    /// value that was not read. A card that cannot say "All backed up" says what it knows in words.
    fn status(&self) -> Status {
        let plain = |title: &str, sub: &str, can_run: bool| Status::Plain {
            title: title.into(),
            sub: sub.into(),
            can_run,
        };
        let rows: Vec<&BackupFolderRow> = self.rows().collect();
        if self.held().is_none() {
            return plain("Backups", concat!("Reading ", beebeeb_engine::device_word!(), "'s backup."), false);
        }
        if self.engine.pass_running() || self.engine.pass_requested() {
            return plain("Backing up", "Beebeeb is checking your folders for changes.", false);
        }
        let on: Vec<&&BackupFolderRow> = rows.iter().filter(|row| self.effective_on(row)).collect();
        if !on.is_empty() && !self.engine.sync_running() {
            return plain(
                "Sync is not running",
                "Your folders are not backing up until Beebeeb syncs again. Unlock Beebeeb or check Sync in Settings.",
                false,
            );
        }
        let stopped = rows.iter().filter(|row| row.state == FolderState::Stopped).count();
        if on.is_empty() && stopped == 0 {
            return plain("Nothing is backing up", "Turn on a folder below to back it up.", false);
        }
        if on.is_empty() {
            return plain(
                "Backups turned themselves off",
                "See what could not back up below, then back up again.",
                false,
            );
        }
        let waiting = on
            .iter()
            .any(|row| row.last_pass_at_ms.is_none() || row.state == FolderState::Waiting);
        if waiting {
            return plain(
                "Waiting for the first backup",
                "It starts on its own. Back up now starts it sooner.",
                true,
            );
        }
        if stopped > 0 {
            return plain(
                "Some backups turned themselves off",
                "See what could not back up below, then back up again.",
                true,
            );
        }
        // The oldest of the folders' last passes: every folder that is on was passed at least this recently.
        let oldest = on.iter().filter_map(|row| row.last_pass_at_ms).min();
        Status::AllBackedUp {
            last: oldest
                .map(|at| last_backup_words(at, chrono::Local::now().date_naive()))
                .unwrap_or_default(),
            // The engine counts a folder's files up to a bound and has no size: never a guess.
            from_this_pc: String::new(),
        }
    }

    fn folders(&self) -> Vec<Folder> {
        let today = chrono::Local::now().date_naive();
        self.rows()
            .map(|row| {
                let mut folder = folder_of(row, today);
                folder.on = self.effective_on(row);
                folder
            })
            .collect()
    }

    /// The account's other devices, from their manifests (`engine_host::backups_devices`).
    fn devices(&self) -> Vec<Device> {
        self.devices.devices()
    }

    /// What the engine knows could not back up: a folder that turned itself off, with core's words for why (the
    /// engine keeps no list of single files it gave up on; nothing else is listed or made up).
    fn not_backed_up(&self) -> Vec<NotBackedUp> {
        self.rows()
            .filter(|row| row.state == FolderState::Stopped)
            .filter_map(|row| {
                Some(NotBackedUp {
                    id: row.key.to_string(),
                    name: row.name.to_string(),
                    folder: concat!(beebeeb_engine::device_word_cap!()).into(),
                    why: row.stop_words.clone()?,
                    excluded: false,
                    kind: FileKind::Document,
                })
            })
            .collect()
    }

    /// Not bound: Mail Backup is not bound here.
    fn mail_backup_on(&self) -> bool {
        false
    }

    /// Asks the runner for a pass of every folder that is on (the scheduler honours it at its next tick). Nothing is
    /// asked while nothing is on, or while a pass or an ask is already out.
    fn back_up_now(&mut self) {
        let any_on = self.rows().any(|row| self.effective_on(row));
        if any_on
            && self.engine.sync_running()
            && !self.run_out
            && !self.engine.pass_running()
            && !self.engine.pass_requested()
        {
            self.run_out = self.send(Ask::RunNow);
        }
    }

    /// Only a folder the list holds for this account, and only a change: sent once, and the list shows the new
    /// switch after the read that follows it.
    fn set_folder(&mut self, id: &str, on: bool) {
        let listed = self.rows().any(|row| row.key == id && self.effective_on(row) != on);
        if listed && self.send(Ask::Switch(id.to_string(), on)) {
            let entry = self.switching.entry(id.to_string()).or_insert((on, 0));
            *entry = (on, entry.1 + 1);
        }
    }

    /// `backup_folders.rs`: the folder picker's answer, asked of the engine.
    fn add_folder(&mut self, path: &str) -> bool {
        self.backup_folders_add_folder(path)
    }

    /// `backup_folders.rs`: "Stop and keep the backup"; a move to Trash needs the password.
    fn remove_folder(&mut self, id: &str, move_copy_to_trash: bool) {
        self.backup_folders_remove_folder(id, move_copy_to_trash)
    }

    /// The move to Trash proves the account password (DW-1410's second step).
    fn trash_needs_password(&self) -> bool {
        true
    }

    fn remove_folder_to_trash(&mut self, id: &str, password: &str) -> bool {
        self.backup_folders_remove_folder_to_trash(id, password)
    }

    fn stopping(&self) -> bool {
        self.backup_folders_stopping()
    }

    /// "Back up again" for a folder that turned itself off: switched on, which clears its stop and starts it over
    /// (`Engine::set_backup_folder`). Only a folder the list shows as stopped.
    fn retry(&mut self, id: &str) {
        let stopped = self
            .rows()
            .any(|row| row.key == id && row.state == FolderState::Stopped);
        if stopped {
            self.set_folder(id, true);
        }
    }

    fn retry_all(&mut self) {
        let keys: Vec<String> = self
            .rows()
            .filter(|row| row.state == FolderState::Stopped)
            .map(|row| row.key.clone())
            .collect();
        for key in keys {
            self.set_folder(&key, true);
        }
    }

    /// The account password proven, then one move to Trash (`engine_host::backups_devices`). `true`: sent; the
    /// answer comes through `pump`.
    fn delete_device_backup(&mut self, id: &str, code: &str) -> bool {
        self.devices.delete(id, code)
    }
}

/// Whether `path` may be chosen as a backup folder by the app `identity`, whose sync folder is `sync_root`.
/// Refused: the sync folder itself, anything inside it, and any folder with the identity's own sync-folder name
/// in its path (the suggested location, also before sync was ever started). Names compare without case, as
/// Windows does. (In a real build the engine's rule decides, `Engine::add_backup_folder_for`; this one stays for its
/// tests.)
#[cfg_attr(not(test), allow(dead_code))]
pub fn may_back_up(identity: &EngineIdentity, sync_root: Option<&Path>, path: &Path) -> bool {
    let same =
        |a: &std::ffi::OsStr, b: &std::ffi::OsStr| a.to_string_lossy().eq_ignore_ascii_case(&b.to_string_lossy());
    let own_name = std::ffi::OsStr::new(identity.sync_folder_name());
    let names_own_folder = path
        .components()
        .any(|part| matches!(part, Component::Normal(name) if same(name, own_name)));
    let inside_root = sync_root.is_some_and(|root| {
        let (root, path): (Vec<_>, Vec<_>) = (root.components().collect(), path.components().collect());
        !root.is_empty()
            && path.len() >= root.len()
            && root.iter().zip(&path).all(|(a, b)| same(a.as_os_str(), b.as_os_str()))
    });
    !names_own_folder && !inside_root
}
