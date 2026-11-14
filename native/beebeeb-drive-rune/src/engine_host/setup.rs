//! The first run on the real engine: [`EngineSetupSource`], the `SetupSource` of the running app.
//!
//! The contract is slice S2 of `docs/superpowers/specs/2026-11-12-native-sync-start-and-sign-out.md`.
//!
//! What it answers, and from where:
//! - The sync folder (the contract's ruling Q2; `setup_folder.rs`): while the settings name none, the first run
//!   asks for it before the first start. The person keeps the suggestion or picks another folder with Windows'
//!   own picker; the engine's rule decides whether it may be the sync folder
//!   ([`Engine::check_sync_folder`]); no start is made before a folder is chosen, and the folder is written to
//!   the settings by the start that worked, once ([`start_now`]).
//! - Beebeeb in File Explorer (DW-0206, DW-0206b, DW-0280): the engine's own view of the sync start and of the
//!   folder's entry in File Explorer ([`SetupEngine::view`], which never waits). Windows refusing to register
//!   the folder is DW-0280 with Windows' code; any other start the engine refused is said as "sync didn't
//!   start" ([`Explorer::NotSyncing`]), never as "adding"; the shell's refusal modal says what happened. "Try
//!   again": while sync runs it asks Windows again for the entry and leaves sync running; while it does not
//!   run, one start is asked for through the shell's one start path. "Show in File Explorer" opens the sync
//!   folder.
//! - Notifications (DW-0207): this PC's "show notifications at all" switch in the settings file
//!   ([`Engine::set_notifications_allowed_for`]).
//! - Open at login (DW-0281): the platform's start-at-login entry, the one Drive's settings switch.
//! - "Open my Beebeeb": the first run is marked done for this PC ([`Engine::mark_first_run_done_for`]), and the
//!   shell's gate lets the main window through ([`FirstRun`]).
//!
//! What it does NOT answer (each is a step that keeps its place and says it is not in this version yet,
//! [`SetupSource::available`], never a step that looks done): a passkey, the app lock, backing up folders,
//! previews. See each method's own line below.
//!
//! Session rule: the source is made for one session of one account ([`SetupEngine::stamp`] when it was made). A
//! press under any other session or account sends nothing; a change is sent for the session of the press; an
//! answer is taken in only while that is still the engine's session and account. The shell drops the whole
//! first-run state when the gate or the account changes.
//!
//! Thread rule: every call that reads or writes a file, asks Windows or the engine's start runs on a worker of
//! its own; its answer comes down one channel, taken in by `poll`, and the worker wakes the window. A switch's
//! value is drawn only once its change was answered ([`SetupSource::take_outcomes`]).
//!
//! Must not: log a path, an email or an id. A path is shown to the person who chose it (the step and its label)
//! and is never logged or printed by `Debug`. Nothing but the engine and the platform write to disk.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError, mpsc};
use std::time::{Duration, Instant};

use beebeeb_engine::account::SessionGeneration;
use beebeeb_engine::app::setup_api::SetupRefused;
use beebeeb_engine::app::{CredentialStores, Engine, SyncStart, SyncStartRefused};
use beebeeb_engine::signin::gate::GateState;
use beebeeb_engine::sync_folder_rule::SyncFolderRefused;
use beebeeb_engine::sync_view::{Explorer as EngineExplorer, Start, SyncView};

use super::log;
use crate::frames::signin::setup::Step;
use crate::frames::signin::setup::source::{
    BUNDLED_SAMPLE_PNG, Explorer, Folder, FolderAsk, LockAfter, Outcome, PreviewFormat, PreviewSize, Sample,
    SetupSource, Switch,
};
use crate::shell::platform::Platform;

#[path = "setup_folder.rs"]
mod folder;
#[cfg(test)]
#[path = "setup_tests.rs"]
mod tests;

pub use folder::start_now;

/// Whether this PC still has its first run to do, as the shell's gate reads it on every frame: in memory, set
/// once at start from the settings file (before the window's first frame), cleared when the first run ends.
/// `false` unless the app's start wired a first run ([`FirstRun::read`]): a window that never asked shows none.
///
/// It also holds the sync folder a person chose in this first run, for the start worker ([`start_now`]): with
/// the session and account it was chosen under, so no other session's start uses it.
#[derive(Default)]
pub struct FirstRun {
    due: AtomicBool,
    /// The app's start wired a first run into this build (Windows): the folder rule is this build's to ask.
    wired: AtomicBool,
    /// The folder, and whose choice it was (the account: a lock and an unlock keep the person's choice).
    folder: Mutex<Option<(Option<String>, PathBuf)>>,
    /// The turn between a folder choice and a start, and what started and what was not saved.
    turn: Mutex<Turn>,
    turn_changed: Condvar,
}

/// How long a choice waits for a start that is on its way.
pub const TURN_WAIT: Duration = Duration::from_secs(30);

/// The state behind [`FirstRun::choosing`] and [`FirstRun::starting`]. Counts and lists, never flags: a guard
/// that is released late must not clear another's mark.
#[derive(Default)]
struct Turn {
    /// The accounts a folder is being chosen under, one entry per choice.
    choosers: Vec<Option<String>>,
    /// Starts on their way.
    starting: usize,
    /// A start was held back by a choice.
    held_back: bool,
    /// Choices blocked on a start right now.
    waiting: usize,
    /// The accounts a start worked for in this run of the app.
    started: Vec<String>,
    /// A started folder the settings did not take, and whose it is.
    unsaved: Option<(String, PathBuf)>,
}

/// A folder is being chosen under an account: no start of that account is made until it drops.
pub struct Choosing<'a> {
    first_run: &'a FirstRun,
    account: Option<String>,
}

/// A start is on its way: no choice is made until it drops.
pub struct Starting<'a>(&'a FirstRun);

impl Drop for Choosing<'_> {
    fn drop(&mut self) {
        let mut turn = self.first_run.turn();
        if let Some(at) = turn.choosers.iter().position(|account| *account == self.account) {
            turn.choosers.remove(at);
        }
        self.first_run.turn_changed.notify_all();
    }
}

impl Drop for Starting<'_> {
    fn drop(&mut self) {
        let mut turn = self.0.turn();
        turn.starting = turn.starting.saturating_sub(1);
        self.0.turn_changed.notify_all();
    }
}

impl FirstRun {
    /// Read the settings file's flag, once, at the app's start. A file that does not read is "not due": a first
    /// run is never shown because a file failed (and the main window is never held back by it).
    pub fn read<E: SetupEngine>(&self, engine: &E) {
        let due = engine.first_run_due().unwrap_or_else(|refused| {
            log(format!("first run: could not be read ({})", refused.kind()));
            false
        });
        self.due.store(due, Ordering::SeqCst);
        self.wired.store(true, Ordering::SeqCst);
    }

    /// Whether the app's start wired a first run at all.
    pub fn wired(&self) -> bool {
        self.wired.load(Ordering::SeqCst)
    }

    pub fn due(&self) -> bool {
        self.due.load(Ordering::SeqCst)
    }

    fn end(&self) {
        self.due.store(false, Ordering::SeqCst);
    }

    fn turn(&self) -> MutexGuard<'_, Turn> {
        self.turn.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A folder is about to be chosen under the account of `stamp`: BLOCKS (a worker's) until no start is on its
    /// way, at most [`TURN_WAIT`] (`None` after that). Until the guard drops, no start of that account is made.
    pub fn choosing(&self, stamp: &Stamp) -> Option<Choosing<'_>> {
        let mut turn = self.turn();
        turn.waiting += 1;
        let (mut turn, _) = self
            .turn_changed
            .wait_timeout_while(turn, TURN_WAIT, |turn| turn.starting > 0)
            .unwrap_or_else(PoisonError::into_inner);
        turn.waiting -= 1;
        if turn.starting > 0 {
            return None;
        }
        turn.choosers.push(stamp.1.clone());
        Some(Choosing {
            first_run: self,
            account: stamp.1.clone(),
        })
    }

    /// A start is about to be made for the account of `stamp`. Never waits: `None` while a folder is being
    /// chosen under the same account (and the start is noted as held back).
    pub fn starting(&self, stamp: &Stamp) -> Option<Starting<'_>> {
        let mut turn = self.turn();
        if stamp.1.is_some() && turn.choosers.contains(&stamp.1) {
            turn.held_back = true;
            return None;
        }
        turn.starting += 1;
        Some(Starting(self))
    }

    /// Whether a start was held back since the last call (it is then asked once the choice ends).
    pub fn take_held_back(&self) -> bool {
        std::mem::take(&mut self.turn().held_back)
    }

    /// How many `choosing` calls are blocked right now.
    #[cfg(test)]
    pub fn choosers_waiting(&self) -> usize {
        self.turn().waiting
    }

    /// A start worked for the account of `stamp` in this run of the app.
    pub fn note_started(&self, stamp: &Stamp) {
        if let Some(account) = &stamp.1 {
            let mut turn = self.turn();
            if !turn.started.contains(account) {
                turn.started.push(account.clone());
            }
        }
    }

    pub fn started_for(&self, stamp: &Stamp) -> bool {
        stamp
            .1
            .as_ref()
            .is_some_and(|account| self.turn().started.contains(account))
    }

    /// The folder a start worked in could not be written to the settings.
    pub fn note_unsaved(&self, stamp: &Stamp, folder: PathBuf) {
        if let Some(account) = &stamp.1 {
            self.turn().unsaved = Some((account.clone(), folder));
        }
    }

    pub fn unsaved_for(&self, stamp: &Stamp) -> Option<PathBuf> {
        let turn = self.turn();
        turn.unsaved
            .as_ref()
            .filter(|(account, _)| stamp.1.as_ref() == Some(account))
            .map(|(_, folder)| folder.clone())
    }

    /// The settings took the folder.
    pub fn saved(&self) {
        self.turn().unsaved = None;
    }

    /// The folder was chosen, by the account of this stamp. An account that is not known chooses nothing.
    fn choose(&self, stamp: Stamp, folder: PathBuf) {
        if stamp.1.is_some() {
            *self.folder.lock().unwrap_or_else(PoisonError::into_inner) = Some((stamp.1, folder));
        }
    }

    /// The folder chosen in this run of the app, when the account of `stamp` chose it: the person's choice
    /// holds over a lock and an unlock, and is never another account's.
    fn chosen_for(&self, stamp: &Stamp) -> Option<PathBuf> {
        let chosen = self.folder.lock().unwrap_or_else(PoisonError::into_inner);
        chosen
            .as_ref()
            .filter(|(account, _)| stamp.1.is_some() && *account == stamp.1)
            .map(|(_, folder)| folder.clone())
    }
}

/// The session and account a press or an answer is for.
type Stamp = (SessionGeneration, Option<String>);

/// The engine handle as the adapter uses it. The real one is [`Engine`]; tests pass a fake.
pub trait SetupEngine: Send + Sync + 'static {
    /// The gate, the sync start and the File Explorer entry. Never waits.
    fn view(&self) -> SyncView;
    /// The session in memory and whose it is. Never waits.
    fn stamp(&self) -> Stamp;
    /// Wake the window: a worker has an answer.
    fn wake(&self);
    /// Whether this PC still has its first run to do (`Engine::first_run_due`). BLOCKS.
    fn first_run_due(&self) -> Result<bool, SetupRefused>;
    /// `Engine::mark_first_run_done_for`. BLOCKS.
    fn mark_done_for(&self, session: SessionGeneration) -> Result<(), SetupRefused>;
    /// `Engine::set_notifications_allowed_for`. BLOCKS.
    fn set_notifications_for(&self, session: SessionGeneration, on: bool) -> Result<(), SetupRefused>;
    /// The sync folder (`Engine::sync_root`). BLOCKS. Never logged.
    fn sync_folder(&self) -> Option<PathBuf>;
    /// The sync folder the settings name, if they name one (`Engine::settings_sync_root`). BLOCKS.
    fn settings_folder(&self) -> Result<Option<PathBuf>, SetupRefused>;
    /// The folder a first run suggests (`Engine::sync_root_suggestion`).
    fn suggested_folder(&self) -> PathBuf;
    /// Whether the folder may be the sync folder (`Engine::check_sync_folder`). BLOCKS.
    fn check_folder(&self, folder: &Path) -> Result<(), SyncFolderRefused>;
    /// Start sync into the folder (`Engine::start_sync`). BLOCKS.
    fn start(&self, folder: PathBuf) -> Result<SyncStart, SyncStartRefused>;
    /// Write the folder to the settings when they name none (`Engine::keep_sync_root`). BLOCKS.
    fn keep_folder(&self, folder: &Path) -> Result<bool, SetupRefused>;
    /// Ask Windows again for the folder's entry in File Explorer; never stops or starts sync. BLOCKS.
    fn ask_explorer_again(&self);
}

impl<C: CredentialStores> SetupEngine for Engine<C> {
    fn view(&self) -> SyncView {
        Engine::view(self)
    }
    fn stamp(&self) -> Stamp {
        (self.session_turn(), self.account_email().or_else(|| self.user_id()))
    }
    fn wake(&self) {
        self.wake_host();
    }
    fn first_run_due(&self) -> Result<bool, SetupRefused> {
        Engine::first_run_due(self)
    }
    fn mark_done_for(&self, session: SessionGeneration) -> Result<(), SetupRefused> {
        self.mark_first_run_done_for(session)
    }
    fn set_notifications_for(&self, session: SessionGeneration, on: bool) -> Result<(), SetupRefused> {
        self.set_notifications_allowed_for(session, on)
    }
    fn sync_folder(&self) -> Option<PathBuf> {
        self.sync_root().ok()
    }
    fn settings_folder(&self) -> Result<Option<PathBuf>, SetupRefused> {
        self.settings_sync_root()
    }
    fn suggested_folder(&self) -> PathBuf {
        self.sync_root_suggestion()
    }
    fn check_folder(&self, folder: &Path) -> Result<(), SyncFolderRefused> {
        self.check_sync_folder(folder)
    }
    fn start(&self, folder: PathBuf) -> Result<SyncStart, SyncStartRefused> {
        self.start_sync(folder)
    }
    fn keep_folder(&self, folder: &Path) -> Result<bool, SetupRefused> {
        self.keep_sync_root(folder)
    }
    fn ask_explorer_again(&self) {
        let _ = self.add_to_file_explorer_again();
    }
}

/// What the adapter asks of the shell around it.
#[derive(Clone)]
pub struct Hooks {
    /// The shell's one way to ask for a sync start (`EngineActions::start_sync`: one start at a time, on its own
    /// worker). `false`: it could not be asked.
    pub start_sync: Arc<dyn Fn() -> bool + Send + Sync>,
    /// Open a folder in File Explorer. `false`: it did not open.
    pub open_folder: fn(&Path) -> bool,
    /// The start-at-login entry.
    pub platform: Arc<dyn Platform>,
    /// The gate's "first run due".
    pub first_run: Arc<FirstRun>,
    /// The File Explorer step exists on this OS (Windows).
    pub explorer_step: bool,
    /// Wake the window once, after this long (a start that never came is said without waiting for a press).
    pub look_later: Arc<dyn Fn(Duration) + Send + Sync>,
}

/// How long a start that was just asked for may still read as "not asked" in the engine's view before the step
/// says sync did not start (the start worker sets the view a moment after it was asked).
pub const START_GRACE: Duration = Duration::from_secs(5);

#[derive(Clone)]
enum Answer {
    Switched(Outcome),
    /// What a worker found out about the sync folder ([`folder::Found`]).
    Folder(folder::Found),
    /// "Open my Beebeeb" could not be written to the settings file (or its worker did not run): the first run is
    /// over for this session all the same. (A write that worked ends it on the worker, for every session.)
    Finished,
    /// Something the view reads through the engine's view may have changed (a start was asked).
    Looked,
    /// "Try again" did its work.
    Retried,
    /// A write of the unsaved folder was tried.
    Saved,
}

/// The first run on the real engine. See the module's doc.
pub struct EngineSetupSource<E: SetupEngine> {
    engine: Arc<E>,
    hooks: Hooks,
    /// The session and account this first run is for.
    made_for: Stamp,
    /// What `explorer` said at the last poll: a change draws again.
    seen: Explorer,
    /// When a sync start was last asked for on this first run's behalf (the shell asks one as it opens).
    start_asked_at: Instant,
    /// The sync folder's question, as the step draws it.
    folder: FolderAsk,
    /// The folder the question shows: the suggestion.
    suggested: String,
    /// This first run asked for the folder (the settings named none when it began).
    asked_folder: bool,
    /// Sync was seen running in this first run: the folder is settled.
    ran: bool,
    /// A folder choice is on its way (the rule's check, or Windows' picker is open): no second one is asked.
    choosing: bool,
    /// A "Try again" is on its way: no second one is asked.
    retrying: bool,
    /// A write of the unsaved folder is on its way.
    saving: bool,
    /// What `folder_unsaved` said at the last poll: a change draws again.
    unsaved_seen: bool,
    /// Workers started by `ask` (a test's proof that a press started none).
    #[cfg(test)]
    spawned: usize,
    outcomes: Vec<Outcome>,
    changed: bool,
    answers: mpsc::Sender<(Stamp, Answer)>,
    inbox: mpsc::Receiver<(Stamp, Answer)>,
}

/// The engine's view as the step draws it. A start the engine refused is not "adding": nothing is being added.
/// Nor is a start that was never made (`start_due`: none is on its way any more): sync is off, and the step says
/// so, with its Try again.
fn explorer_of(view: &SyncView, start_due: bool) -> Explorer {
    let code = |code: Option<i32>| {
        // As Windows writes an HRESULT: "0x8007017C".
        code.map(|code| format!("0x{:08X}", code as u32)).unwrap_or_default()
    };
    match (view.start, view.explorer) {
        // Windows refused to register the folder: DW-0280, with Windows' code.
        (Start::Refused(SyncStartRefused::RegistrationFailed { code: windows }), _) => {
            Explorer::Failed { code: code(windows) }
        }
        (Start::Refused(_), _) => Explorer::NotSyncing,
        (Start::NotAsked, _) if !start_due => Explorer::NotSyncing,
        (_, EngineExplorer::Added) => Explorer::Added,
        (_, EngineExplorer::Failed { code: windows }) => Explorer::Failed { code: code(windows) },
        (Start::NotAsked | Start::Starting | Start::Running, EngineExplorer::Adding | EngineExplorer::Unknown) => {
            Explorer::Adding
        }
    }
}

impl<E: SetupEngine> EngineSetupSource<E> {
    /// For the session and account that are the engine's now. Nothing is asked of the engine here.
    pub fn new(engine: Arc<E>, hooks: Hooks) -> Self {
        let (answers, inbox) = mpsc::channel();
        let (made_for, seen) = (engine.stamp(), explorer_of(&engine.view(), true));
        let mut source = Self {
            engine,
            hooks,
            made_for,
            seen,
            start_asked_at: Instant::now(),
            folder: FolderAsk::Settled,
            suggested: String::new(),
            asked_folder: false,
            ran: false,
            choosing: false,
            retrying: false,
            saving: false,
            unsaved_seen: false,
            #[cfg(test)]
            spawned: 0,
            outcomes: Vec::new(),
            changed: false,
            answers,
            inbox,
        };
        source.look_after_the_grace();
        source.read_folder();
        source
    }

    /// A start was asked for now: the view may say "not asked" for a moment longer. When the grace is over the
    /// window is woken once, so a start that never came is said without waiting for a press.
    fn look_after_the_grace(&mut self) {
        self.start_asked_at = Instant::now();
        (self.hooks.look_later)(START_GRACE + Duration::from_millis(50));
    }

    /// Whether the engine's session and account are still the ones this first run was made for, with the vault
    /// open.
    fn mine(&self) -> bool {
        self.engine.view().gate == GateState::Ready && self.engine.stamp() == self.made_for
    }

    /// Run `work` on a worker named `name`, for the session of this press; its answer goes down the channel
    /// with that session, and the window is woken. A worker that could not start, or that panicked, answers
    /// `refused` (never a press that ends in nothing). A press under another session sends and says nothing.
    fn ask(
        &mut self,
        name: &str,
        work: impl FnOnce(&E, SessionGeneration) -> Answer + Send + 'static,
        refused: Option<Answer>,
    ) {
        let or_else = refused.clone();
        let refuse = |source: &mut Self| {
            if let Some(answer) = refused {
                let _ = source.answers.send((source.made_for.clone(), answer));
                source.changed = true;
            }
        };
        if !self.mine() {
            // Another session's press: nothing is sent, and nothing is said to it either (its answer would be
            // dropped by the poll: it is not this first run's session).
            return;
        }
        let (engine, answers, stamp) = (self.engine.clone(), self.answers.clone(), self.made_for.clone());
        let started = std::thread::Builder::new().name(name.into()).spawn(move || {
            // A worker that panicked answers as a press that was refused: never a switch left on "Saving".
            let answer = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(&engine, stamp.0)))
                .ok()
                .or(or_else);
            if let Some(answer) = answer {
                let _ = answers.send((stamp, answer));
            }
            engine.wake();
        });
        if started.is_err() {
            log("first run: could not start a thread");
            refuse(self);
        } else {
            #[cfg(test)]
            {
                self.spawned += 1;
            }
        }
    }

    /// How many workers `ask` has started: read on the window's thread, never "after a moment".
    #[cfg(test)]
    pub(super) fn workers_started(&self) -> usize {
        self.spawned
    }
}

impl<E: SetupEngine> SetupSource for EngineSetupSource<E> {
    /// The steps something answers for: File Explorer (on Windows), notifications, open at login.
    fn available(&self, step: Step) -> bool {
        match step {
            Step::Explorer => self.hooks.explorer_step,
            Step::Notifications | Step::Login => true,
            // Passkey, Lock, Backup, Previews: see the methods below. Done is not a step of the list.
            Step::Passkey | Step::Lock | Step::Backup | Step::Previews | Step::Done => false,
        }
    }

    /// The first-run window has no way to a setting yet (the main window's settings open after it): the rows of
    /// "You're set up" are values.
    fn opens_settings(&self) -> bool {
        false
    }

    /// Every change here is made on a worker.
    fn answers_later(&self) -> bool {
        true
    }

    fn take_outcomes(&mut self) -> Vec<Outcome> {
        std::mem::take(&mut self.outcomes)
    }

    fn explorer(&self) -> Explorer {
        explorer_of(&self.engine.view(), self.start_asked_at.elapsed() < START_GRACE)
    }

    fn folder_ask(&self) -> FolderAsk {
        self.folder.clone()
    }

    fn keep_folder(&mut self) {
        self.keep_suggested();
    }

    fn choose_folder(&mut self) {
        self.pick_another();
    }

    /// After a start that was refused, while this first run asked for the folder and sync never ran in it: the
    /// settings name no folder yet, and the folder may be what was refused.
    fn can_choose_again(&self) -> bool {
        self.asked_folder
            && !self.ran
            && !self.hooks.first_run.started_for(&self.made_for)
            && matches!(self.engine.view().start, Start::Refused(_))
    }

    /// A start worked, its folder could not be written to the settings, and sync runs: the step says so. While
    /// sync does not run the step shows what the engine says; the folder stays noted and the next start that
    /// finds sync running writes it.
    fn folder_unsaved(&self) -> bool {
        self.hooks.first_run.unsaved_for(&self.made_for).is_some() && self.engine.view().start == Start::Running
    }

    /// "Try again" on the unsaved folder: write it. One at a time.
    fn save_folder(&mut self) {
        if self.saving || !self.mine() {
            return;
        }
        self.saving = true;
        let (first_run, stamp) = (self.hooks.first_run.clone(), self.made_for.clone());
        self.ask(
            "first-run-save",
            move |engine, _| {
                if let Some(folder) = first_run.unsaved_for(&stamp) {
                    match engine.keep_folder(&folder) {
                        Ok(_) => first_run.saved(),
                        Err(refused) => log(format!(
                            "first run: the sync folder could not be written ({})",
                            refused.kind()
                        )),
                    }
                }
                Answer::Saved
            },
            Some(Answer::Saved),
        );
    }

    fn dismiss_folder_note(&mut self) {
        if self.folder == FolderAsk::AlreadySyncs {
            self.folder = FolderAsk::Settled;
            self.changed = true;
        }
    }

    /// "Try again" (DW-0280). Sync runs: Windows is asked again for the entry and sync keeps running (nothing
    /// here can stop it). Sync does not run: one start is asked for, the shell's way.
    /// One at a time: a second press while the first is on its way asks nothing.
    fn retry_explorer(&mut self) {
        if self.retrying || !self.mine() {
            return;
        }
        self.retrying = true;
        let start = self.hooks.start_sync.clone();
        self.look_after_the_grace();
        self.ask(
            "first-run-explorer",
            move |engine, _| {
                if engine.view().start == Start::Running {
                    engine.ask_explorer_again();
                } else if !start() {
                    log("first run: a sync start could not be asked for");
                }
                Answer::Retried
            },
            Some(Answer::Retried),
        );
    }

    /// "Show in File Explorer": the sync folder, opened. The path stays on the worker.
    fn show_in_explorer(&mut self) {
        let open = self.hooks.open_folder;
        self.ask(
            "first-run-show",
            move |engine, _| {
                let opened = engine.sync_folder().is_some_and(|folder| open(&folder));
                if !opened {
                    log("first run: the sync folder did not open in File Explorer");
                }
                Answer::Looked
            },
            None,
        );
    }

    /// Step 6, for the session of the press. The answer says what is set: on a failure, not known.
    fn set_notifications(&mut self, on: bool) {
        let not_done = Outcome {
            switch: Switch::Notifications,
            done: false,
            now: None,
        };
        self.ask(
            "first-run-notifications",
            move |engine, session| match engine.set_notifications_for(session, on) {
                Ok(()) => Answer::Switched(Outcome {
                    done: true,
                    now: Some(on),
                    ..not_done
                }),
                Err(refused) => {
                    log(format!(
                        "first run: notifications were not changed ({})",
                        refused.kind()
                    ));
                    Answer::Switched(not_done)
                }
            },
            Some(Answer::Switched(not_done)),
        );
    }

    /// Step 7: the platform's start-at-login entry; the answer is what the entry is after the change.
    fn set_open_at_login(&mut self, on: bool) {
        let platform = self.hooks.platform.clone();
        let not_done = Outcome {
            switch: Switch::OpenAtLogin,
            done: false,
            now: None,
        };
        self.ask(
            "first-run-login",
            move |_, _| {
                let done = platform.set_autostart(on).is_ok();
                if !done {
                    log("first run: open at login was not changed");
                }
                Answer::Switched(Outcome {
                    switch: Switch::OpenAtLogin,
                    done,
                    now: platform.autostart_enabled().ok(),
                })
            },
            Some(Answer::Switched(not_done)),
        );
    }

    /// "Open my Beebeeb": mark the first run done for this PC, for the session of the press. The main window
    /// opens once that was answered. Written: the first run is over for this PC, whoever is signed in by then
    /// (the worker says so itself, so a lock that lands before the next poll cannot bring it back). Not
    /// written (the settings file, or no worker): it is over for this session all the same (it is shown again
    /// at the next start; it never holds a person out of their files). A session that moved on: nothing.
    fn finish(&mut self) {
        let first_run = self.hooks.first_run.clone();
        self.ask(
            "first-run-finish",
            move |engine, session| match engine.mark_done_for(session) {
                Ok(()) => {
                    first_run.end();
                    Answer::Looked
                }
                Err(SetupRefused::SessionMoved) => Answer::Looked,
                Err(refused) => {
                    log(format!("first run: could not be marked done ({})", refused.kind()));
                    Answer::Finished
                }
            },
            Some(Answer::Finished),
        );
    }

    /// Take in what the workers answered (only for this first run's session and account), and see whether the
    /// engine's view of File Explorer moved.
    fn poll(&mut self) -> bool {
        let mut changed = std::mem::take(&mut self.changed);
        let mine = self.mine();
        while let Ok((stamp, answer)) = self.inbox.try_recv() {
            if !mine || stamp != self.made_for {
                continue;
            }
            changed = true;
            match answer {
                Answer::Switched(outcome) => self.outcomes.push(outcome),
                Answer::Folder(found) => self.take_in_folder(found),
                Answer::Finished => self.hooks.first_run.end(),
                Answer::Looked => {}
                Answer::Retried => self.retrying = false,
                Answer::Saved => self.saving = false,
            }
        }
        let unsaved = self.folder_unsaved();
        if unsaved != std::mem::replace(&mut self.unsaved_seen, unsaved) {
            changed = true;
        }
        self.ran |= mine && self.engine.view().start == Start::Running;
        let now = self.explorer();
        if now != self.seen {
            self.seen = now;
            changed = true;
        }
        changed
    }

    // ---- Not bound. Each belongs to a step this source says is not available (`available`): the step says so
    // and its controls are not drawn, so nothing reaches these; each does nothing and claims nothing.

    /// Backing up folders (step 3): Backups is switched off in the running app (`engine_host::backups::TURNED_ON`),
    /// so a backup turned on here could not be seen or stopped afterwards. Binds with that section.
    fn folders(&self) -> Vec<Folder> {
        Vec::new()
    }
    fn start_backup(&mut self, _folders: &[String]) {}

    /// Previews (step 4, and the compare view): the engine has no function that measures a preview, estimates
    /// the account's photos, or hands out one of them. No weight, no estimate and no limit is made up.
    fn original_weight(&self) -> String {
        String::new()
    }
    fn weight(&self, _size: PreviewSize, _format: PreviewFormat) -> Option<String> {
        None
    }
    fn fit_line(&self, _size: PreviewSize, _format: PreviewFormat) -> String {
        String::new()
    }
    fn space_limit(&self) -> String {
        String::new()
    }
    /// The picture that ships inside the app, said as that: none of the account's photos is read here.
    fn sample(&self) -> Sample {
        Sample {
            original_png: BUNDLED_SAMPLE_PNG,
            preview_png: BUNDLED_SAMPLE_PNG,
            bundled: true,
        }
    }
    fn set_previews(&mut self, _size: PreviewSize, _format: PreviewFormat) {}
    fn choose_photo(&mut self) {}

    /// A passkey (step 1): not offered (`available`). The engine adds one ([`beebeeb_engine::app::Engine::add_passkey`]),
    /// but core's flow needs the account password for its step-up, and this step has no password to hand over and
    /// no field to ask it in: it stays Settings > Passkeys, where the pane asks.
    fn add_passkey(&mut self) -> bool {
        false
    }
    /// The app lock (step 2): a lane of its own (Windows Hello); nothing is switched on here.
    fn set_app_lock(&mut self, _on: bool, _after: LockAfter) {}
}
