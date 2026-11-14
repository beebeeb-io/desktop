//! Drive's own settings panes on the real engine: [`EngineDriveSettings`], the `DriveSettings` of the running app.
//!
//! Wired (draws what it read): Storage insights (DW-0783, b, c, d): the account's use from Beebeeb
//! ([`Engine::storage_info`]), the breakdown by type and the largest files worked out on this PC from the file list
//! in the local state database (decrypted names, in memory only).
//!
//! Bound but NOT wired (each pane keeps the shell's "not built yet" frame; the PR body names what each one lacks):
//! - Sync and File Explorer: the sync state from the handle's view, Pause and Resume ([`Engine::pause`] /
//!   [`Engine::resume`]), Open at login through the shell's own autostart call (`Platform::set_autostart`, no
//!   registry write of its own), the downloaded and kept sizes, Free up space
//!   ([`Engine::free_up_space`], in the signed-in account's local state through the account-bound handle).
//!   The limit on downloaded copies is stored through the engine ([`Engine::set_transfer_limits_for`]; eviction
//!   works to it). Missing: a repair and a turn-off of File Explorer in the engine crate, a pane that can say "no
//!   limit" (the engine's default), an engine that honours "Show sync status".
//! - Storage: `storage()` holds only what the engine has (use, downloaded, kept); the per-kind use, the Trash size,
//!   the previews' size and limit have no engine call.
//! - Keep on this PC (a dialog of Sync and Storage): the folder tree with sizes and the kept set from the local
//!   state, and Apply through the pin change ([`Engine::set_kept`]), refused while sync does not run.
//! - Bandwidth: the two speed limits, read from and stored in this PC's settings file through the engine (its
//!   upload and download paths honour them), and the last 20 hours of traffic from the signed-in account's local
//!   state. Read when the pane opens and after a change, never at launch. Missing: the live figures, the other
//!   devices, a metered state the engine reads and honours, a speed test.
//! - Thumbnails and previews, Diagnostics: the trait's defaults (no engine module).
//!
//! Whose press: a limit is this PC's own setting, not an account's. It is still changed only under the session it
//! was pressed in (`Engine::session_turn`, read on the window's thread at the press): a session that moved on
//! before the worker ran changes nothing, and says so.
//!
//! Thread rule: nothing here waits on the window's thread but the handle's `view()` (never waits). One worker per
//! source does every read and every change in the order asked and sends each answer down one channel, which
//! [`DriveSettings::pump`] drains every frame.
//!
//! Whose answer (review of desktop#377, P2-5; the model is `engine_host/this_pc.rs`): every job carries the account
//! it was asked for and a generation, and every answer the generation and the account it is for (none when the
//! account was not the same after the work as before it). `pump` compares the gate and the account FIRST: a change
//! drops what the window holds and starts a new generation, and only then are the worker's answers taken in, each
//! only when its generation is the current one and its account the one the window sees. Every reader hands out
//! nothing unless the vault is open and the signed-in account is the one the answer was read for. A change (Pause,
//! Open at login, Free up space, Keep) runs only while the account that asked is still signed in with the vault
//! open; a change that did not happen is said in the window's toast.
//!
//! Whose state (task 2488): the local state database can still belong to an EARLIER account after a sign-out that
//! keeps local data. Nothing is read from it unless its recorded owner is the signed-in account
//! ([`DriveEngine::owns`]); otherwise the local sizes and the folder tree answer empty.
//!
//! When it reads (P2-3): when the gate or the account changes, and when Insights opens and what is held is older than
//! [`INSIGHTS_FRESH_FOR`]. No periodic timer.
//!
//! Must not: log a name, a path or an id; keep a name in memory while the vault is locked or no one is signed in;
//! draw a value it has not read.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use beebeeb_engine::account::SessionGeneration;
use beebeeb_engine::app::settings_api::AccountCallError;
use beebeeb_engine::app::speed_test_api::{SpeedFigure, SpeedStep};
use beebeeb_engine::app::support_bundle_api::{BundleRefused, SavedBundle};
use beebeeb_engine::app::transfer_limits_api::{TransferLimits, TransferLimitsChange};
use beebeeb_engine::app::{CredentialStores, Engine, ExplorerChangeRefused, LocalStateRefused};
use beebeeb_engine::backup_protect::BackupsTree;
use beebeeb_engine::config::DesktopConfig;
use beebeeb_engine::drive_v2::{dto::DriveItem, model};
use beebeeb_engine::signin::gate::GateState;
use beebeeb_engine::state_db::{BandwidthSample, FileEntry, PinState, StateDb};
use beebeeb_engine::sync_view::{Explorer, SyncView};

use super::log;
use crate::frames::settings::Pane;
use crate::frames::settings::bandwidth_data::{Bandwidth, Direction, History, SpeedTest, Step};
use crate::frames::settings::diagnostics::{Bundle, Diagnostics};
use crate::frames::settings::drive::{
    Control, DriveSettings, Insights, InsightsData, Kept, KeptOnPc, KindShare, LargeFile, Limit, Repair, Storage,
    SyncPane, SyncStatus,
};
use crate::frames::settings::keep::{Folder, Keep};
use crate::shell::platform::Platform;

#[path = "drive_settings_reads.rs"]
mod reads;
#[path = "drive_settings_tools.rs"]
mod tools;
pub(crate) use reads::*;
use tools::Os;
pub use tools::SPEED_TEST_ON;

#[cfg(test)]
#[path = "drive_settings_tests.rs"]
mod tests;

/// How recently insights must have been read for `opened` not to ask again.
pub const INSIGHTS_FRESH_FOR: Duration = Duration::from_secs(30);
/// The largest files the page lists by name; the rest of the top 50 is "And N more, down to …".
const LARGEST_SHOWN: usize = 7;
const LARGEST_TOP: usize = 50;
/// How long a path may be before the page cuts it in the middle (the file name always stays whole).
const PATH_SHOWN: usize = 49;

/// What Storage insights says when it cannot load. Fixed sentences: never a server's words or a path.
pub const INSIGHTS_OFFLINE: &str = "Couldn't reach Beebeeb. Check your connection and try again.";
pub const INSIGHTS_FAILED: &str = "Couldn't work out where your storage goes. Try again.";
pub const INSIGHTS_LOCKED: &str = "Unlock Beebeeb to see where your storage goes.";

/// What the window says when a change the person asked for did not happen. Fixed sentences.
pub const KEEP_NOT_SYNCING: &str = "Sync isn't running, so the folders were not changed.";
pub const KEEP_FAILED: &str = "Couldn't change what is kept on this PC. Try again.";
pub const FREE_UP_FAILED: &str = "Couldn't free up space on this PC. Try again.";
pub const PAUSE_FAILED: &str = "Couldn't pause or resume sync. Try again.";
pub const OPEN_AT_LOGIN_FAILED: &str = "Couldn't change Open at login. Try again.";
/// A change asked while no one is signed in with the vault open, or by an account that has left since.
pub const CHANGE_LOCKED: &str = "Unlock Beebeeb to change this.";
pub const LIMIT_FAILED: &str = "Couldn't change the limit. Try again.";
/// A press on a control this version does not draw: said, never done.
pub const METERED_NOT_AVAILABLE: &str = "Not available in this version yet.";
/// How far back the Bandwidth pane's traffic goes, and how long one of its buckets is.
const TRAFFIC_HOURS: i64 = 20;
const BUCKET_MINUTES: i64 = 20;

/// What the adapter asks of the engine handle. The app's is `Engine`; a test's is a state database in a folder of
/// its own.
pub trait DriveEngine: Send + Sync + 'static {
    /// The handle's view. Never waits.
    fn view(&self) -> SyncView;
    /// Whose account is signed in (the stored email, else the user id the server named). Never waits.
    fn account(&self) -> Option<String>;
    /// Read the signed-in account's local state, through the engine's account-bound handle (task 2488). `Err`: it
    /// is not handed out, and why (nothing synced yet, another account's, the session changed, unreadable).
    fn local_state<T>(&self, read: impl FnOnce(&StateDb) -> T) -> Result<T, LocalStateRefused>;
    /// Used and quota bytes from Beebeeb. A network call: a worker's.
    fn storage_info(&self) -> Result<(i64, i64), String>;
    fn set_paused(&self, paused: bool) -> Result<(), String>;
    /// Sync runs for this handle. May wait: a worker's.
    fn syncing(&self) -> bool;
    fn set_kept(&self, file_id: &str, kept: bool) -> Result<(), String>;
    /// Make every downloaded copy that is not kept online only again: the bytes freed.
    fn free_up_space(&self) -> Result<u64, String>;
    /// The settings file's limit on downloaded copies (0: no limit) and its "Show sync status on files".
    fn local_settings(&self) -> Result<(i64, bool), String>;
    /// The engine's session, which moves on at every sign-in, lock and sign-out (`Engine::session_turn`). Never
    /// waits.
    fn session(&self) -> SessionGeneration;
    /// This PC's transfer limits, from the settings file. A worker's.
    fn limits(&self) -> Result<TransferLimits, String>;
    /// WRITES the settings file: change the limits, under `session` and no other. A worker's.
    fn set_limits(&self, session: SessionGeneration, change: TransferLimitsChange) -> Result<TransferLimits, String>;
    /// Repair Beebeeb in File Explorer under `session`: sync stops and starts again, which registers the folder.
    /// BLOCKS: a worker's.
    fn repair_explorer(&self, session: SessionGeneration) -> Result<(), ExplorerChangeRefused>;
    /// Turn Beebeeb off in File Explorer under `session`; refused while changes wait to upload. BLOCKS.
    fn turn_off_explorer(&self, session: SessionGeneration) -> Result<(), ExplorerChangeRefused>;
    /// One step of the speed test under `session`. BLOCKS: a worker's.
    fn speed_step(
        &self,
        session: SessionGeneration,
        step: SpeedStep,
        progress: &(dyn Fn(u32, u32) + Sync),
    ) -> Result<SpeedFigure, AccountCallError>;
    /// Save the support bundle under `session` (already redacted by the engine). BLOCKS: a worker's.
    fn export_bundle(&self, session: SessionGeneration) -> Result<SavedBundle, BundleRefused>;
}

impl<C: CredentialStores> DriveEngine for Engine<C> {
    fn view(&self) -> SyncView {
        Engine::view(self)
    }
    fn account(&self) -> Option<String> {
        self.account_email().or_else(|| self.user_id())
    }
    fn local_state<T>(&self, read: impl FnOnce(&StateDb) -> T) -> Result<T, LocalStateRefused> {
        Engine::local_state(self).and_then(|state| state.with(read))
    }
    fn storage_info(&self) -> Result<(i64, i64), String> {
        Engine::storage_info(self)
    }
    fn set_paused(&self, paused: bool) -> Result<(), String> {
        if paused { self.pause() } else { self.resume() }
    }
    fn syncing(&self) -> bool {
        self.is_syncing()
    }
    fn set_kept(&self, file_id: &str, kept: bool) -> Result<(), String> {
        Engine::set_kept(self, file_id, kept)
    }
    fn free_up_space(&self) -> Result<u64, String> {
        // The engine's own, in the signed-in account's local state (the account-bound handle, task 2488).
        Engine::free_up_space(self)
    }
    fn local_settings(&self) -> Result<(i64, bool), String> {
        let config = DesktopConfig::load_for(crate::shell::identity::engine_identity())?;
        Ok((config.local_cache_limit_bytes, config.sync_overlays.unwrap_or(true)))
    }
    fn session(&self) -> SessionGeneration {
        self.session_turn()
    }
    fn limits(&self) -> Result<TransferLimits, String> {
        self.transfer_limits().map_err(|error| error.words().to_string())
    }
    fn set_limits(&self, session: SessionGeneration, change: TransferLimitsChange) -> Result<TransferLimits, String> {
        self.set_transfer_limits_for(session, change)
            .map_err(|error| error.words().to_string())
    }
    fn repair_explorer(&self, session: SessionGeneration) -> Result<(), ExplorerChangeRefused> {
        Engine::repair_explorer_for(self, session)
    }
    fn turn_off_explorer(&self, session: SessionGeneration) -> Result<(), ExplorerChangeRefused> {
        Engine::turn_off_explorer_for(self, session)
    }
    fn speed_step(
        &self,
        session: SessionGeneration,
        step: SpeedStep,
        progress: &(dyn Fn(u32, u32) + Sync),
    ) -> Result<SpeedFigure, AccountCallError> {
        Engine::speed_step_for(self, session, step, progress)
    }
    fn export_bundle(&self, session: SessionGeneration) -> Result<SavedBundle, BundleRefused> {
        Engine::export_support_bundle_for(self, session)
    }
}

/// What this PC holds, as last read.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
struct OnThisPc {
    downloaded: i64,
    kept_items: usize,
    kept_bytes: i64,
    limit_bytes: i64,
    status_on_files: bool,
}

enum Ask {
    /// The local sizes, the settings, Open at login and the folder tree.
    Read,
    Insights,
    Pause(bool),
    OpenAtLogin(bool),
    FreeUp,
    Keep(Vec<String>, Vec<String>),
    /// The Bandwidth pane's limits and traffic.
    Bandwidth,
    /// Change this PC's transfer limits.
    Limits(TransferLimitsChange),
    /// Repair, and turn off, Beebeeb in File Explorer.
    Repair,
    TurnOff,
    /// Save the support bundle and hand it to the OS.
    Export,
}

/// One job: what is asked, for whose account, under which engine session (as the window's thread read it when it
/// was asked: the session of the press), in which generation.
struct Job {
    ask: Ask,
    account: String,
    session: SessionGeneration,
    generation: u64,
}

enum Answer {
    /// The limits the settings file holds, and the traffic of the last 20 hours (`None`: the local state did not
    /// hand it out). `None`: the limits could not be read.
    Bandwidth(Option<(TransferLimits, Option<History>)>),
    OnThisPc(Option<OnThisPc>),
    OpenAtLogin(Option<bool>),
    Keep(Keep),
    Insights(Insights, Option<(i64, i64)>),
    /// A change that did not happen, in words the window says (P2-6).
    Refused(&'static str),
    /// Where a repair ended.
    Repaired(Repair),
    /// Where the speed test stands.
    Speed(SpeedTest),
    /// Where the last export ended.
    Bundle(Bundle),
}

/// A worker's answer: the generation it was asked in and the account it is for (`None`: the account that asked was
/// no longer signed in with the vault open when the work ended).
struct Tagged {
    generation: u64,
    whose: Option<String>,
    answer: Answer,
}

/// Drive's settings panes on the real engine. See the module's doc.
pub struct EngineDriveSettings<E: DriveEngine> {
    engine: Arc<E>,
    platform: Arc<dyn Platform>,
    jobs: Option<mpsc::Sender<Job>>,
    pending: Arc<AtomicUsize>,
    answers: mpsc::Sender<Tagged>,
    inbox: mpsc::Receiver<Tagged>,
    on_pc: Option<OnThisPc>,
    open_at_login: Option<bool>,
    usage: Option<(i64, i64)>,
    /// The Bandwidth pane's limits and traffic, as last read (`None`: not read yet, or not readable).
    bandwidth: Option<(TransferLimits, Option<History>)>,
    insights: Insights,
    keep: Keep,
    /// When insights were last answered: `opened(Insights)` re-asks when this is older than [`INSIGHTS_FRESH_FOR`].
    insights_at: Option<Instant>,
    /// Whether an insights read is out: no second one is asked.
    insights_reading: bool,
    /// The (gate, account) the last pump saw: a change drops everything held and starts a new generation.
    seen: (GateState, Option<String>),
    generation: u64,
    /// What `insights()` lends while what is held is not the signed-in account's.
    locked: Insights,
    loading: Insights,
    /// A refused change: the window's toast says it once.
    toast: Option<&'static str>,
    /// Where a repair of File Explorer stands.
    repair: Repair,
    /// Where the speed test stands (nothing until pressed).
    speed: SpeedTest,
    /// Where the last support-bundle export ended.
    bundle: Bundle,
    /// The OS's hand-offs of an exported bundle.
    os: Os,
    /// The pane last opened: read again for the new account after a change of gate or account.
    open: Option<Pane>,
    /// When the local sizes and the folder tree were last answered, and whether that read is out.
    local_at: Option<Instant>,
    local_reading: bool,
}

impl<E: DriveEngine> EngineDriveSettings<E> {
    /// The source of the running app. Starts nothing: a pane reads when it opens.
    pub fn new(engine: Arc<E>, platform: Arc<dyn Platform>) -> Self {
        let (answers, inbox) = mpsc::channel();
        let gate = engine.view().gate;
        let account = engine.account();
        let source = Self {
            engine,
            platform,
            jobs: None,
            pending: Arc::default(),
            answers,
            inbox,
            on_pc: None,
            open_at_login: None,
            usage: None,
            bandwidth: None,
            insights: Insights::Loading,
            keep: Keep::Failed,
            insights_at: None,
            insights_reading: false,
            seen: (gate, account),
            generation: 0,
            locked: Insights::Failed(INSIGHTS_LOCKED.into()),
            loading: Insights::Loading,
            toast: None,
            repair: Repair::Idle,
            speed: SpeedTest::Idle,
            bundle: Bundle::None,
            os: Os::real(),
            open: None,
            local_at: None,
            local_reading: false,
        };
        // Nothing is read at launch: a pane reads when it opens (`opened`), never before (module doc).
        source
    }

    /// Test builds: the OS's hand-offs of an exported bundle are `os`'s.
    #[cfg(test)]
    pub(crate) fn with_os(mut self, os: Os) -> Self {
        self.os = os;
        self
    }

    /// The local sizes, the settings and the folder tree: asked when they are not held or are old, never twice.
    fn ask_local(&mut self) {
        let fresh = self.local_at.is_some_and(|at| at.elapsed() < INSIGHTS_FRESH_FOR) && self.mine();
        if !fresh && !self.local_reading {
            self.local_reading = self.send(Ask::Read);
        }
    }

    /// Send `ask` for the account the window sees now. Nothing is sent while no one is signed in with the vault open.
    fn send(&mut self, ask: Ask) -> bool {
        let (GateState::Ready, Some(account)) = self.seen.clone() else {
            return false;
        };
        if self.jobs.is_none() {
            let (tx, rx) = mpsc::channel::<Job>();
            let (engine, platform, answers, pending, os) = (
                self.engine.clone(),
                self.platform.clone(),
                self.answers.clone(),
                self.pending.clone(),
                self.os,
            );
            let started = std::thread::Builder::new()
                .name("drive-settings".into())
                .spawn(move || {
                    // Ends when the source is dropped (its sender with it).
                    for job in rx {
                        let done = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            run(&*engine, &*platform, os, &answers, job)
                        }));
                        if done.is_err() {
                            log("drive settings: a read or a change ended early");
                        }
                        pending.fetch_sub(1, Ordering::SeqCst);
                    }
                });
            match started {
                Ok(_) => self.jobs = Some(tx),
                Err(_) => {
                    log("drive settings: could not start its worker");
                    return false;
                }
            }
        }
        self.pending.fetch_add(1, Ordering::SeqCst);
        let job = Job {
            ask,
            account,
            // The session of the press: read here, on the window's thread, before any worker runs.
            session: self.engine.session(),
            generation: self.generation,
        };
        let sent = self.jobs.as_ref().is_some_and(|tx| tx.send(job).is_ok());
        if !sent {
            self.pending.fetch_sub(1, Ordering::SeqCst);
        }
        sent
    }

    /// Send a change the person asked for; one that cannot be asked (no one signed in with the vault open) is said.
    fn change(&mut self, ask: Ask) {
        if !self.mine() || !self.send(ask) {
            self.toast = Some(CHANGE_LOCKED);
        }
    }

    fn unlocked(&self) -> bool {
        self.engine.view().gate == GateState::Ready
    }

    /// Whether what the window holds may be handed out: the vault is open, and the signed-in account is the one the
    /// last `pump` saw (what is held was read for that account, or is nothing). Asked on every reader, so nothing of
    /// an account that left is drawn even before the next `pump` drops it.
    fn mine(&self) -> bool {
        if self.seen.0 != GateState::Ready || !self.unlocked() {
            return false;
        }
        let account = self.engine.account();
        account.is_some() && account == self.seen.1
    }

    fn ask_insights(&mut self) {
        if !self.insights_reading {
            self.insights_reading = self.send(Ask::Insights);
            if !self.insights_reading {
                self.insights = Insights::Failed(INSIGHTS_LOCKED.into());
            }
        }
    }
}

impl<E: DriveEngine> EngineDriveSettings<E> {
    /// The speed test, on a thread of its own: three steps, each reported as it starts and as it moves, then the
    /// result or the step that failed. Under the session of the press, for the account the window sees, in this
    /// generation (each answer carries them like every other).
    fn start_speed_test(&mut self) -> bool {
        let (GateState::Ready, Some(account)) = self.seen.clone() else {
            return false;
        };
        let (engine, answers, pending) = (self.engine.clone(), self.answers.clone(), self.pending.clone());
        let (session, generation) = (self.engine.session(), self.generation);
        pending.fetch_add(1, Ordering::SeqCst);
        let started = std::thread::Builder::new().name("speed-test".into()).spawn({
            let pending = pending.clone();
            move || {
                let ended = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    run_speed_test(&*engine, &answers, &account, session, generation)
                }));
                if ended.is_err() {
                    log("drive settings: the speed test ended early");
                }
                pending.fetch_sub(1, Ordering::SeqCst);
            }
        });
        if started.is_err() {
            pending.fetch_sub(1, Ordering::SeqCst);
            log("drive settings: could not start the speed test");
        }
        started.is_ok()
    }
}

/// The speed test's three steps. Every report says whose it is, checked at the moment it is sent.
fn run_speed_test<E: DriveEngine>(
    engine: &E,
    answers: &mpsc::Sender<Tagged>,
    account: &str,
    session: SessionGeneration,
    generation: u64,
) {
    let report = |test: SpeedTest| {
        let _ = answers.send(Tagged {
            generation,
            whose: still_for(engine, account).then(|| account.to_string()),
            answer: Answer::Speed(test),
        });
    };
    let mut steps: Vec<Step> = Vec::new();
    let mut moved = 0u64;
    for (at, step) in tools::STEPS.into_iter().enumerate() {
        let before = steps.clone();
        report(tools::running(&before, at, 0, tools::parts(step)));
        let progress = |done: u32, of: u32| report(tools::running(&before, at, done, of));
        match engine.speed_step(session, step, &progress) {
            Ok(figure) => {
                if let SpeedFigure::Throughput { bytes, .. } = figure {
                    moved += bytes;
                }
                steps.push(tools::done(figure));
            }
            Err(why) => {
                log("drive settings: a speed test step did not finish");
                return report(SpeedTest::Failed(tools::step_failed(step, why)));
            }
        }
    }
    let Ok(done) = <[Step; 3]>::try_from(steps) else {
        return;
    };
    report(tools::finished(done, moved));
}

/// The account `job` was asked for is still the signed-in one, with the vault open.
fn still_for<E: DriveEngine>(engine: &E, account: &str) -> bool {
    engine.view().gate == GateState::Ready && engine.account().as_deref() == Some(account)
}

/// One job, on the worker. A change runs only while the account that asked is still signed in with the vault open,
/// and is followed by the read it changes. Every answer says the generation it was asked in and, checked after the
/// work, whose it is.
fn run<E: DriveEngine>(engine: &E, platform: &dyn Platform, os: Os, answers: &mpsc::Sender<Tagged>, job: Job) {
    let Job {
        ask,
        account,
        session,
        generation,
    } = job;
    let send = |answer| {
        let whose = still_for(engine, &account).then(|| account.clone());
        let _ = answers.send(Tagged {
            generation,
            whose,
            answer,
        });
    };
    let refuse = |words| send(Answer::Refused(words));
    if !matches!(ask, Ask::Read | Ask::Insights | Ask::Bandwidth) && !still_for(engine, &account) {
        // The account that asked has left, or the vault is locked: nothing is changed for whoever is here now.
        log("drive settings: a change was not made (the account that asked is not signed in)");
        return refuse(CHANGE_LOCKED);
    }
    match ask {
        Ask::Read => read_local(engine, platform, &account, &send),
        Ask::Insights => {
            let (insights, usage) = insights(engine, &account);
            send(Answer::Insights(insights, usage));
        }
        Ask::Pause(paused) => {
            if engine.set_paused(paused).is_err() {
                log("drive settings: sync was not paused or resumed (the settings file)");
                refuse(PAUSE_FAILED);
            }
        }
        Ask::OpenAtLogin(on) => {
            if platform.set_autostart(on).is_err() {
                log("drive settings: open at login was not changed");
                refuse(OPEN_AT_LOGIN_FAILED);
            }
            send(Answer::OpenAtLogin(platform.autostart_enabled().ok()));
        }
        Ask::Bandwidth => send(Answer::Bandwidth(read_bandwidth(engine, &account))),
        Ask::Limits(change) => {
            // Under the session of the press and no other: the engine asks it again before it writes.
            if engine.set_limits(session, change).is_err() {
                log("drive settings: a limit was not changed (the session moved on, or the settings file)");
                refuse(LIMIT_FAILED);
            }
            // What the file holds now, changed or not: the pane never keeps a limit the engine did not take.
            send(Answer::Bandwidth(read_bandwidth(engine, &account)));
            if change.downloaded_copies_bytes.is_some() {
                read_local(engine, platform, &account, &send);
            }
        }
        Ask::Repair => match engine.repair_explorer(session) {
            Ok(()) => send(Answer::Repaired(Repair::Idle)),
            Err(ExplorerChangeRefused::Failed) => {
                log("drive settings: File Explorer was not repaired");
                send(Answer::Repaired(Repair::Failed));
            }
            Err(refused) => {
                log("drive settings: File Explorer was not repaired (refused)");
                send(Answer::Repaired(Repair::Idle));
                refuse(refused.words());
            }
        },
        Ask::TurnOff => {
            if let Err(refused) = engine.turn_off_explorer(session) {
                log("drive settings: File Explorer was not turned off");
                refuse(refused.words());
            }
        }
        Ask::Export => match engine.export_bundle(session) {
            Ok(saved) => {
                let shown = (os.reveal)(&saved.path);
                let draft = (os.open_draft)(&tools::draft_for(&saved.file_name));
                if !shown {
                    log("drive settings: the bundle was saved but not shown in File Explorer");
                }
                send(Answer::Bundle(Bundle::Saved {
                    folder: saved.folder.to_string(),
                    file: saved.file_name,
                    draft_opened: draft,
                }));
            }
            Err(why) => {
                log("drive settings: the support bundle was not saved");
                send(Answer::Bundle(Bundle::Failed(why.words().to_string())));
            }
        },
        Ask::FreeUp => {
            if engine.free_up_space().is_err() {
                log("drive settings: free up space did not finish");
                refuse(FREE_UP_FAILED);
            }
            read_local(engine, platform, &account, &send);
        }
        Ask::Keep(keep, release) => {
            if !engine.syncing() {
                log("drive settings: keep on this PC was not changed: sync does not run");
                refuse(KEEP_NOT_SYNCING);
            } else {
                let changes = keep
                    .iter()
                    .map(|id| (id, true))
                    .chain(release.iter().map(|id| (id, false)));
                // Each folder only while the account that asked is still the one signed in.
                let failed = changes
                    .filter(|(id, on)| !still_for(engine, &account) || engine.set_kept(id, *on).is_err())
                    .count();
                if failed > 0 {
                    log(format!("drive settings: {failed} folders were not changed"));
                    refuse(KEEP_FAILED);
                }
            }
            read_local(engine, platform, &account, &send);
        }
    }
}

impl<E: DriveEngine> DriveSettings for EngineDriveSettings<E> {
    /// Storage insights only: every other pane keeps the shell's frame (the module's doc says why).
    fn wired(&self, pane: Pane) -> bool {
        matches!(pane, Pane::Insights | Pane::Sync | Pane::Bandwidth | Pane::Diagnostics)
    }

    /// Controls the engine cannot honour in this version are not drawn (the triage's honest states).
    fn shows(&self, control: Control) -> bool {
        match control {
            // The engine stores the setting and nothing honours it.
            Control::StatusOnFiles => false,
            // No metered-network detection: a switch would do nothing.
            Control::SyncOnMetered => false,
            Control::SpeedTest => SPEED_TEST_ON,
        }
    }

    /// The gate or the account changed: what the window held goes now, a new generation starts, and an open vault
    /// is read again. Only then the worker's answers: one counts when it was asked in this generation and is for
    /// the account the window sees.
    fn pump(&mut self, _seconds: f64) -> (bool, bool) {
        let mut changed = false;
        let now = (self.engine.view().gate, self.engine.account());
        if now != self.seen {
            self.seen = now;
            self.generation += 1;
            self.on_pc = None;
            self.open_at_login = None;
            self.usage = None;
            self.bandwidth = None;
            self.keep = Keep::Failed;
            self.insights_at = None;
            self.insights_reading = false;
            self.local_at = None;
            self.local_reading = false;
            self.repair = Repair::Idle;
            self.speed = SpeedTest::Idle;
            self.bundle = Bundle::None;
            self.insights = if self.seen.0 == GateState::Ready {
                Insights::Loading
            } else {
                Insights::Failed(INSIGHTS_LOCKED.into())
            };
            // Only what the open pane shows is read again for the new account.
            match self.open {
                Some(Pane::Insights) => self.ask_insights(),
                Some(Pane::Sync | Pane::Storage) => self.ask_local(),
                Some(Pane::Bandwidth) => {
                    self.send(Ask::Bandwidth);
                }
                _ => {}
            }
            changed = true;
        }
        while let Ok(Tagged {
            generation,
            whose,
            answer,
        }) = self.inbox.try_recv()
        {
            if generation != self.generation {
                // Asked before the gate or the account changed: dropped, never drawn.
                continue;
            }
            changed = true;
            let mine = self.seen.0 == GateState::Ready && whose.is_some() && whose == self.seen.1;
            match answer {
                Answer::Refused(words) => self.toast = Some(words),
                Answer::Insights(insights, usage) => {
                    self.insights_reading = false;
                    if mine {
                        self.insights_at = Some(Instant::now());
                        self.insights = insights;
                        self.usage = usage;
                    } else {
                        // The account left while it was worked out: nothing of it is kept; Try again asks anew.
                        self.insights = Insights::Failed(INSIGHTS_FAILED.into());
                    }
                }
                // The local read is over (`Keep` is its last answer, whichever way it ended).
                Answer::Keep(keep) => {
                    self.local_reading = false;
                    if mine {
                        self.local_at = Some(Instant::now());
                        self.keep = keep;
                    }
                }
                _ if !mine => {}
                Answer::Bandwidth(read) => self.bandwidth = read,
                Answer::OnThisPc(on_pc) => self.on_pc = on_pc,
                Answer::OpenAtLogin(on) => self.open_at_login = on,
                Answer::Repaired(repair) => self.repair = repair,
                Answer::Speed(test) => self.speed_test_reported(test),
                Answer::Bundle(bundle) => self.bundle = bundle,
            }
        }
        (changed, !self.idle())
    }

    /// A pane of Drive's settings opened: insights reads again when what it holds is old (P2-3).
    fn opened(&mut self, pane: Pane) {
        self.open = Some(pane);
        // The local sizes and the folder tree: Sync and Storage show them.
        if matches!(pane, Pane::Sync | Pane::Storage) {
            self.ask_local();
        }
        if pane == Pane::Insights {
            let fresh = self.insights_at.is_some_and(|at| at.elapsed() < INSIGHTS_FRESH_FOR) && self.mine();
            if !fresh {
                self.ask_insights();
            }
        }
        // Bandwidth is read when its pane opens (and after a change), never at launch.
        if pane == Pane::Bandwidth {
            self.send(Ask::Bandwidth);
        }
    }

    /// A refused change: the window's toast says it once (P2-6).
    fn take_toast(&mut self) -> Option<&'static str> {
        self.toast.take()
    }

    fn storage(&self) -> Storage {
        let mine = self.mine();
        let on_pc = self.on_pc.filter(|_| mine);
        let usage = self.usage.filter(|_| mine);
        let limit_gb = on_pc.map_or(0, |p| {
            u32::try_from(p.limit_bytes.max(0) / 1_000_000_000).unwrap_or(u32::MAX)
        });
        Storage {
            used: usage.map(|(used, _)| size_words(used)).unwrap_or_default(),
            total: usage.map(|(_, quota)| size_words(quota)).unwrap_or_default(),
            shares: [
                usage
                    .filter(|(_, quota)| *quota > 0)
                    .map_or(0.0, |(used, quota)| (used as f64 / quota as f64).clamp(0.0, 1.0) as f32),
                0.0,
            ],
            kinds: Vec::new(),
            trash: String::new(),
            downloaded: KeptOnPc {
                used: on_pc.map(|p| size_words(p.downloaded)).unwrap_or_default(),
                limit: Limit {
                    gb: limit_gb,
                    min_gb: 1,
                    max_gb: limit_gb.max(1),
                    at: 1.0,
                    says: String::new(),
                },
            },
            previews: KeptOnPc {
                used: String::new(),
                limit: Limit {
                    gb: 0,
                    min_gb: 0,
                    max_gb: 0,
                    at: 0.0,
                    says: String::new(),
                },
            },
            offline: on_pc.map(|p| size_words(p.kept_bytes)).unwrap_or_default(),
        }
    }

    /// Downloaded copies: stored in this PC's settings file through the engine, under the session of the press; the
    /// engine's eviction works to it. Previews: no engine call (they have no limit of their own yet).
    fn set_limit(&mut self, which: Kept, gb: u32) {
        if which == Kept::Downloaded {
            self.change(Ask::Limits(TransferLimitsChange {
                downloaded_copies_bytes: Some(i64::from(gb) * 1_000_000_000),
                ..TransferLimitsChange::default()
            }));
        }
    }

    /// Downloaded copies: the engine's Free up space (never a kept file, never an unsent change). Previews: no engine
    /// call.
    fn clear(&mut self, which: Kept) {
        if which == Kept::Downloaded {
            self.change(Ask::FreeUp);
        }
    }

    fn sync(&self) -> SyncPane {
        let view = self.engine.view();
        let mine = self.mine();
        let on_pc = self.on_pc.filter(|_| mine);
        let says = super::sync::sync_word(&view).unwrap_or("").to_string();
        SyncPane {
            in_explorer: view.explorer == Explorer::Added,
            status_on_files: on_pc.is_some_and(|p| p.status_on_files),
            repair: if mine { self.repair } else { Repair::Idle },
            status: if view.paused {
                SyncStatus::Paused
            } else {
                SyncStatus::Running(says)
            },
            open_at_login: self.open_at_login.filter(|_| mine).unwrap_or(false),
            downloaded: on_pc.map(|p| size_words(p.downloaded)).unwrap_or_default(),
            limit_gb: on_pc.map_or(0, |p| {
                u32::try_from(p.limit_bytes.max(0) / 1_000_000_000).unwrap_or(u32::MAX)
            }),
            offline: on_pc
                .map(|p| {
                    format!(
                        "{} · {}",
                        count_words(p.kept_items, "item", "items"),
                        size_words(p.kept_bytes)
                    )
                })
                .unwrap_or_default(),
            not_uploaded: None,
        }
    }

    /// Not bound: the engine stores the setting and nothing honours it yet (PR body).
    fn set_status_on_files(&mut self, _on: bool) {}

    fn set_open_at_login(&mut self, on: bool) {
        self.change(Ask::OpenAtLogin(on));
    }

    /// The handle's view says paused at once when the settings file took it.
    fn set_paused(&mut self, paused: bool) {
        self.change(Ask::Pause(paused));
    }

    /// Not bound: no metered state is read here, so `sync()` never says sync waits on one.
    fn sync_anyway(&mut self) {}

    /// Sync stops and starts again into the configured folder (the registration path Setup's "Try again" asks);
    /// the row says "Repairing..." while it runs, and a refusal says why.
    fn repair(&mut self) {
        if self.repair == Repair::Running {
            return;
        }
        if self.mine() && self.send(Ask::Repair) {
            self.repair = Repair::Running;
        } else {
            self.toast = Some(CHANGE_LOCKED);
        }
    }

    /// Refused (and said) while changes wait to upload; else sync stops and the folder's registration is removed.
    fn turn_off_explorer(&mut self) {
        self.change(Ask::TurnOff);
    }

    /// The two limits as the settings file holds them and the last 20 hours of traffic, only while they are the
    /// signed-in account's read. Everything the engine has no answer for stays as the empty pane draws it: no live
    /// figure, no device, no speed test; the metered switch is the pane's default and is not the engine's to read.
    fn bandwidth(&self) -> Bandwidth {
        let Some((limits, history)) = self.bandwidth.clone().filter(|_| self.mine()) else {
            return Bandwidth::default();
        };
        Bandwidth {
            upload_limit: mbps(limits.upload_kib_per_s),
            download_limit: mbps(limits.download_kib_per_s),
            history: history.unwrap_or_default(),
            test: self.speed.clone(),
            ..Bandwidth::default()
        }
    }

    /// No metered-network detection exists: no switch is drawn (`shows`), and a press that arrives anyway changes
    /// nothing and says so.
    fn set_sync_on_metered(&mut self, _on: bool) {
        self.toast = Some(METERED_NOT_AVAILABLE);
    }

    /// Start the speed test against Beebeeb's own servers on a thread of its own (it takes up to 90 seconds and
    /// must not hold Pause or Keep up); one test at a time. Its steps arrive through `pump`.
    fn run_speed_test(&mut self) {
        if self.speed.running() {
            return;
        }
        if self.mine() && self.start_speed_test() {
            self.speed = tools::started();
        } else {
            self.toast = Some(CHANGE_LOCKED);
        }
    }

    /// Where the speed test stands, as the window is told it. A test of an earlier generation never gets here
    /// (`pump` drops it); one reported while no account is the window's is ignored.
    fn speed_test_reported(&mut self, now: SpeedTest) {
        if self.mine() {
            self.speed = now;
        }
    }

    /// What the page can say today: where the last export ended. The app's own CPU, memory and transfer rates are
    /// not sampled in this version: no list is drawn for them, and the page says so.
    fn diagnostics(&self) -> Diagnostics {
        Diagnostics {
            activity: Vec::new(),
            bundle: if self.mine() { self.bundle.clone() } else { Bundle::None },
        }
    }

    /// Save the bundle (queue counts, an error code, the last error message, all redacted by the engine), show it
    /// in File Explorer and open a draft to support. Called once per press, never by showing a page.
    fn export_bundle(&mut self) {
        self.change(Ask::Export);
    }

    /// Stored in this PC's settings file through the engine, under the session of the press; the engine's upload
    /// and download paths read it for every transfer. The pane shows the limit the file holds after the change.
    fn set_speed_limit(&mut self, direction: Direction, limit: Option<u32>) {
        let limit = Some(kib_per_s(limit));
        self.change(Ask::Limits(match direction {
            Direction::Upload => TransferLimitsChange {
                upload_kib_per_s: limit,
                ..TransferLimitsChange::default()
            },
            Direction::Download => TransferLimitsChange {
                download_kib_per_s: limit,
                ..TransferLimitsChange::default()
            },
        }));
    }

    /// What is held, only while it is the signed-in account's (P2-5): a locked vault lends the locked sentence, an
    /// account the last `pump` has not seen yet lends "loading", whatever is still stored.
    fn insights(&self) -> &Insights {
        if self.mine() {
            &self.insights
        } else if self.unlocked() {
            &self.loading
        } else {
            &self.locked
        }
    }

    fn reload_insights(&mut self) {
        self.insights = Insights::Loading;
        self.ask_insights();
    }

    fn keep(&self) -> Keep {
        if self.mine() { self.keep.clone() } else { Keep::Failed }
    }

    fn reload_keep(&mut self) {
        self.send(Ask::Read);
    }

    /// Only the folders named: each newly kept or newly released folder alone, through the pin change. Refused
    /// while sync does not run (the tree then still shows the old choice).
    fn apply_keep(&mut self, keep: &[String], release: &[String]) {
        if keep.is_empty() && release.is_empty() {
            return;
        }
        self.change(Ask::Keep(keep.to_vec(), release.to_vec()));
    }
}

impl<E: DriveEngine> EngineDriveSettings<E> {
    /// No job is waiting or running.
    pub(crate) fn idle(&self) -> bool {
        self.pending.load(Ordering::SeqCst) == 0
    }
}
