//! The Finder setup reconciler, run by the app (macOS; a no-op elsewhere). `start` spawns the engine crate's
//! driver (`finder_setup::driver`) on the macOS ports (`macos_fp::ports`) on its own thread; everything else here
//! is the app's side of it: the triggers (`trigger`, `lock`, `sign_out`, `repair`, `on_app_activated`), the edge
//! detector the session code calls (`session_changed`), the view for Settings and the popover (`view`,
//! `turn_on`, `turn_off`, `try_again`) and "Copy details" with the real lifecycle tail (`copy_details`).
//!
//! Rule 6: the engine start refuses while a removal is owed (`FinderHost::start_engine`); the driver removes
//! first and asks `Session::clear_removal_owed` once it is confirmed.
//!
//! Must not: decide a state, a retry or a transition (the driver's core does), read an OS error message, log a
//! path. Tests: `finder_setup_tests.rs`, with the driver on fake ports.

#![allow(dead_code)] // Settings (dm-18), the popover and the shell (dm-19) call the rest.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::Duration;

use beebeeb_engine::finder_setup::core::Trigger;
use beebeeb_engine::finder_setup::driver::FinderSetupView;
use beebeeb_engine::finder_setup::error::FpError;
use beebeeb_engine::finder_setup::launch_location::LaunchLocation;

/// How long a sign-out or Repair waits for the removal, and a Lock for its acknowledgement.
const REMOVE_WAIT: Duration = Duration::from_secs(30);
const LOCK_WAIT: Duration = Duration::from_secs(5);

/// What the session looks like to the reconciler: the keys in memory and a stored session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) struct SessionFacts {
    pub keys: bool,
    pub auth: bool,
}

/// A change of [`SessionFacts`] that is a trigger.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Edge {
    KeysArrived,
    Lock,
    SignOut,
}

/// Which trigger a change from `before` to `now` is, if any. A sign-out wins over a lock (no session left).
pub(crate) fn edge(before: SessionFacts, now: SessionFacts) -> Option<Edge> {
    if before.auth && !now.auth {
        Some(Edge::SignOut)
    } else if before.keys && !now.keys {
        Some(Edge::Lock)
    } else if !before.keys && now.keys {
        Some(Edge::KeysArrived)
    } else {
        None
    }
}

/// The app's session as the ports read and change it. The real one is over the engine; tests put a fake here.
pub(crate) trait Session: Send + Sync + 'static {
    fn facts(&self) -> SessionFacts;
    fn removal_owed(&self) -> bool;
    fn config_path(&self) -> Option<PathBuf>;
    /// Blocking. `Ok(true)`: a new engine was started; `Ok(false)`: one was running.
    fn start_engine(&self) -> Result<bool, FpError>;
    /// Blocking.
    fn finish_ready(&self) -> Result<(), FpError>;
    /// Blocking.
    fn stop_engine(&self) -> Result<(), FpError>;
    fn clear_removal_owed(&self);
    fn clear_domain_gone(&self);
    /// The view changed: wake the window.
    fn changed(&self);
}

// ---------------------------------------------------------------------------
// The one reconciler of this process
// ---------------------------------------------------------------------------

static WINDOW_VISIBLE: AtomicBool = AtomicBool::new(false);
static KEPT_FOLDER: Mutex<Option<String>> = Mutex::new(None);
static LAST_FACTS: Mutex<SessionFacts> = Mutex::new(SessionFacts {
    keys: false,
    auth: false,
});
static CURRENT: OnceLock<Running> = OnceLock::new();

struct Running {
    handle: beebeeb_engine::finder_setup::driver::FinderSetupHandle,
    facts: Arc<dyn Fn() -> SessionFacts + Send + Sync>,
}

/// Waits for a future on a throwaway runtime: the callers are the app's worker threads, never the driver's.
fn wait<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("a current-thread runtime builds")
        .block_on(future)
}

fn current() -> Option<&'static Running> {
    CURRENT.get()
}

/// A Beebeeb window is on screen: the poll tick runs only then. The shell sets it.
pub fn set_window_visible(visible: bool) {
    WINDOW_VISIBLE.store(visible, Ordering::SeqCst);
}

/// A folder macOS kept on a removal, for Settings › Sync, once.
pub fn take_kept_folder() -> Option<String> {
    KEPT_FOLDER.lock().unwrap_or_else(PoisonError::into_inner).take()
}

fn note_kept(location: Option<String>) {
    if let Some(location) = location {
        *KEPT_FOLDER.lock().unwrap_or_else(PoisonError::into_inner) = Some(location);
    }
}

/// Sends one trigger. Quietly nothing when the reconciler is not running (another OS, a test build).
pub fn trigger(trigger: Trigger) {
    if let Some(running) = current() {
        if let Err(sentence) = running.handle.trigger(trigger) {
            tracing::warn!("finder setup: trigger {} not delivered: {sentence}", trigger.as_str());
        }
    }
}

/// Bringing the app forward: one read-only check (dm-19 calls this).
pub fn on_app_activated() {
    if let Some(running) = current() {
        running.handle.app_activated();
    }
}

/// Lock: cancels a running check and holds, acknowledged before this returns.
pub fn lock() {
    if let Some(running) = current() {
        if let Err(sentence) = wait(running.handle.lock(LOCK_WAIT)) {
            tracing::warn!("finder setup: lock not acknowledged: {sentence}");
        }
    }
}

/// Removes the domain (sign-out holds, Repair checks again); what macOS kept is shown through [`take_kept_folder`].
fn remove(trigger: Trigger) -> Result<(), String> {
    let Some(running) = current() else {
        return Err(beebeeb_engine::finder_setup::driver::NOT_RUNNING.to_string());
    };
    match wait(running.handle.remove(trigger, REMOVE_WAIT)) {
        Ok(removal) => {
            note_kept(removal.kept_location("finder setup removal"));
            Ok(())
        }
        Err(failure) => {
            let message = failure.message.clone();
            note_kept(failure.kept_location("finder setup removal"));
            Err(message)
        }
    }
}

/// Sign-out: the Finder location is removed and stays removed until keys arrive.
pub fn sign_out() -> Result<(), String> {
    remove(Trigger::SignOut)
}

/// Repair: removed, then added back by the next check.
pub fn repair() -> Result<(), String> {
    remove(Trigger::Repair)
}

/// Settings "Turn on": lifts the hold and checks.
pub fn turn_on() {
    trigger(Trigger::KeysArrived);
}

/// Settings "Turn off": removes the location (keeping unsynced files) and holds.
pub fn turn_off() -> Result<(), String> {
    remove(Trigger::SignOut)
}

/// "Try again" on a failure. `false`: held (locked or signed out), so it cannot add Beebeeb now.
pub fn try_again() -> bool {
    match current() {
        Some(running) if !running.handle.is_held() => {
            trigger(Trigger::TryAgain);
            true
        }
        _ => false,
    }
}

/// What Settings and the popover read; "Open in Finder" is offered on `Ready`.
pub fn view() -> FinderSetupView {
    match current() {
        Some(running) => running.handle.view(),
        None => FinderSetupView::initial(LaunchLocation::Applications),
    }
}

/// The text of "Copy details", with the real lifecycle lines.
pub fn copy_details(view: &FinderSetupView, app_version: &str, macos_version: &str) -> String {
    beebeeb_engine::finder_setup::driver::copy_details_from(view, app_version, macos_version, lifecycle_tail)
}

#[cfg(target_os = "macos")]
fn lifecycle_tail(n: usize) -> Vec<String> {
    crate::shell::platform::macos::macos_fp::lifecycle_log::tail(n)
}

#[cfg(not(target_os = "macos"))]
fn lifecycle_tail(_n: usize) -> Vec<String> {
    Vec::new()
}

/// The session code calls this after any step that may have changed the keys or the session (the sign-in worker,
/// after each job): the edge from the last facts is the trigger. Keys arriving lift the hold, a lock cancels and
/// holds, a sign-out removes.
pub fn session_changed() {
    let Some(running) = current() else { return };
    let now = (running.facts)();
    let before = std::mem::replace(&mut *LAST_FACTS.lock().unwrap_or_else(PoisonError::into_inner), now);
    match edge(before, now) {
        Some(Edge::KeysArrived) => trigger(Trigger::KeysArrived),
        Some(Edge::Lock) => lock(),
        Some(Edge::SignOut) => {
            if let Err(message) = sign_out() {
                tracing::warn!("finder setup: the sign-out removal did not finish: {message}");
            }
        }
        None => {}
    }
}

// ---------------------------------------------------------------------------
// macOS: the host the ports call, and the start
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
#[cfg(test)]
pub(crate) use macos::{spawn_with, FinderHost};

#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use crate::shell::platform::macos::macos_fp::lifecycle_log;
    use crate::shell::platform::macos::macos_fp::ports::{
        FpBridge, FpHost, MacosPorts, SystemBridge, BRIDGE_GATE_WAIT,
    };
    use beebeeb_engine::finder_setup::driver::{FinderSetupHandle, LifecycleEvent, SystemClock};
    use beebeeb_engine::finder_setup::error::{app_code, FailureRecord};
    use beebeeb_engine::finder_setup::policy::RetryPolicy;

    /// The ports' host over a [`Session`].
    pub(crate) struct FinderHost<S: Session> {
        session: Arc<S>,
    }

    impl<S: Session> FinderHost<S> {
        pub(crate) fn new(session: Arc<S>) -> Self {
            Self { session }
        }
    }

    impl<S: Session> FpHost for FinderHost<S> {
        fn vault_unlocked(&self) -> bool {
            self.session.facts().keys
        }
        fn auth_present(&self) -> bool {
            self.session.facts().auth
        }
        fn removal_owed(&self) -> bool {
            self.session.removal_owed()
        }
        fn config_path(&self) -> Option<PathBuf> {
            self.session.config_path()
        }
        /// Rule 6: no engine starts while a removal is owed.
        async fn start_engine(&self, _limit: Duration) -> Result<bool, FpError> {
            if self.session.removal_owed() {
                return Err(FpError::app(app_code::ENGINE_START, "a Finder removal is still owed"));
            }
            let session = self.session.clone();
            blocking(move || session.start_engine()).await
        }
        async fn finish_ready(&self) -> Result<(), FpError> {
            let session = self.session.clone();
            blocking(move || session.finish_ready()).await
        }
        async fn stop_engine(&self, _limit: Duration) -> Result<(), FpError> {
            let session = self.session.clone();
            blocking(move || session.stop_engine()).await
        }
        fn publish(&self, _view: &FinderSetupView) {
            self.session.changed();
        }
        fn window_visible(&self) -> bool {
            WINDOW_VISIBLE.load(Ordering::SeqCst)
        }
        fn surface_kept_folder(&self, location: Option<String>) {
            note_kept(location);
            self.session.changed();
        }
        fn clear_removal_owed(&self) {
            self.session.clear_removal_owed();
        }
        fn clear_domain_gone(&self) {
            self.session.clear_domain_gone();
        }
    }

    async fn blocking<T: Send + 'static>(
        job: impl FnOnce() -> Result<T, FpError> + Send + 'static,
    ) -> Result<T, FpError> {
        tokio::task::spawn_blocking(job)
            .await
            .unwrap_or_else(|_| Err(FpError::app(app_code::ENGINE_START, "the engine task failed")))
    }

    /// Spawns the driver on the ambient runtime with the macOS ports over `bridge`; `Launch` is sent at once.
    pub(crate) fn spawn_with<S: Session, B: FpBridge>(
        session: Arc<S>,
        bridge: Arc<B>,
        launch: LaunchLocation,
        persisted: Option<FailureRecord>,
    ) -> FinderSetupHandle {
        let ports = MacosPorts::with_bridge(Arc::new(FinderHost::new(session)), bridge, BRIDGE_GATE_WAIT);
        let handle =
            beebeeb_engine::finder_setup::driver::spawn(ports, SystemClock, launch, persisted, RetryPolicy::default());
        let _ = handle.trigger(Trigger::Launch);
        handle
    }

    /// The app's start: the lifecycle log, the launch line, the persisted failure, the driver on its own thread and
    /// runtime, the `Launch` trigger.
    pub(super) fn start_real<S: Session>(session: Arc<S>) {
        if CURRENT.get().is_some() {
            return;
        }
        if let Some(dir) = lifecycle_log::default_dir() {
            lifecycle_log::init(dir, beebeeb_engine::diagnostic_redaction::KnownNames::new);
        }
        let launch = beebeeb_engine::finder_setup::launch_location::current();
        lifecycle_log::event(LifecycleEvent::Launch {
            app_version: beebeeb_engine::finder_setup::driver::token(env!("CARGO_PKG_VERSION")),
            macos_version: macos_version(),
            launch_location: launch,
        });
        let persisted = session
            .config_path()
            .and_then(|path| beebeeb_engine::config::DesktopConfig::load_from(&path).ok())
            .and_then(|cfg| cfg.finder_last_failure);
        *LAST_FACTS.lock().unwrap_or_else(PoisonError::into_inner) = session.facts();
        let (sent, received) = std::sync::mpsc::channel();
        let for_thread = session.clone();
        let started = std::thread::Builder::new().name("finder-setup".into()).spawn(move || {
            let Ok(runtime) = tokio::runtime::Builder::new_current_thread().enable_time().build() else {
                return;
            };
            runtime.block_on(async move {
                let _ = sent.send(spawn_with(for_thread, Arc::new(SystemBridge), launch, persisted));
                std::future::pending::<()>().await
            });
        });
        if started.is_err() {
            tracing::warn!("finder setup: the reconciler thread did not start");
            return;
        }
        if let Ok(handle) = received.recv() {
            let facts: Arc<dyn Fn() -> SessionFacts + Send + Sync> = Arc::new(move || session.facts());
            let _ = CURRENT.set(Running { handle, facts });
        }
    }

    /// dm-41: the system's replica may date from another app generation (0.8.x) or from a state.db that was made
    /// fresh (the marker lives in the state folder, so a wiped state takes it along). When the app version differs from the marker kept in the state folder, ask the
    /// system once to list the whole tree again; the marker is written only after the bridge said yes.
    pub(super) fn reimport_once_if_changed(identity: &'static beebeeb_engine::identity::EngineIdentity) {
        let _ = std::thread::Builder::new().name("finder-reimport".into()).spawn(move || {
            let Ok(dir) = beebeeb_engine::state_paths::beebeeb_state_dir_for(identity) else { return };
            let generation = env!("CARGO_PKG_VERSION").to_string();
            let marker = dir.join("fp-reimport-generation");
            if std::fs::read_to_string(&marker).is_ok_and(|seen| seen.trim() == generation) {
                return;
            }
            match crate::shell::platform::macos::macos_fp::bridge::reimport() {
                Ok(()) => {
                    let _ = std::fs::write(&marker, generation);
                }
                Err(_) => tracing::warn!("finder setup: the reimport request did not go through; tried again next start"),
            }
        });
    }

    /// `ProductVersion` from the system's version file: no process is started.
    fn macos_version() -> String {
        std::fs::read_to_string("/System/Library/CoreServices/SystemVersion.plist")
            .ok()
            .and_then(|plist| {
                let rest = plist.split("<key>ProductVersion</key>").nth(1)?;
                let value = rest.split("<string>").nth(1)?.split("</string>").next()?;
                Some(beebeeb_engine::finder_setup::driver::token(value.trim()))
            })
            .unwrap_or_else(|| "unknown".into())
    }
}

/// The app's start: runs the reconciler on `engine`. A no-op off macOS and in unit tests (they never touch the
/// File Provider; the driver's own tests use `spawn_with` on fake ports).
#[cfg_attr(any(test, not(target_os = "macos")), allow(unused_variables))]
pub fn start<C>(
    engine: Arc<beebeeb_engine::app::Engine<C>>,
    identity: &'static beebeeb_engine::identity::EngineIdentity,
) where
    C: beebeeb_engine::app::CredentialStores + 'static,
{
    #[cfg(all(target_os = "macos", not(test)))]
    macos::start_real(Arc::new(LiveSession { engine, identity }));
    #[cfg(all(target_os = "macos", not(test)))]
    macos::reimport_once_if_changed(identity);
}

/// The real session, over the engine.
struct LiveSession<C: beebeeb_engine::app::CredentialStores> {
    engine: Arc<beebeeb_engine::app::Engine<C>>,
    identity: &'static beebeeb_engine::identity::EngineIdentity,
}

impl<C: beebeeb_engine::app::CredentialStores + 'static> Session for LiveSession<C> {
    fn facts(&self) -> SessionFacts {
        use beebeeb_engine::signin::gate::GateState;
        let gate = self.engine.gate();
        SessionFacts {
            keys: super::sync::may_start(gate),
            auth: !matches!(gate, GateState::SignedOut),
        }
    }
    /// The engine's debt reader is test-only (`StateDb::finder_removal_owed`), so a restart does not remember a debt
    /// yet (reported); the driver's removal-first step and `start_engine`'s refusal work from what this says.
    fn removal_owed(&self) -> bool {
        false
    }
    fn config_path(&self) -> Option<PathBuf> {
        beebeeb_engine::config::DesktopConfig::path_for(self.identity).ok()
    }
    fn start_engine(&self) -> Result<bool, FpError> {
        use beebeeb_engine::app::SyncStart;
        use beebeeb_engine::finder_setup::error::app_code;
        let config = beebeeb_engine::config::DesktopConfig::load_for(self.identity)
            .map_err(|_| FpError::app(app_code::ENGINE_START, "the settings could not be read"))?;
        let root = config
            .sync_root
            .unwrap_or_else(|| beebeeb_engine::config::default_sync_root_suggestion(self.identity));
        match self.engine.start_sync(root) {
            Ok(SyncStart::Started) => Ok(true),
            Ok(SyncStart::AlreadyRunning) => Ok(false),
            Ok(SyncStart::NoSession) | Err(_) => Err(FpError::app(app_code::ENGINE_START, "the engine did not start")),
        }
    }
    fn finish_ready(&self) -> Result<(), FpError> {
        Ok(())
    }
    fn stop_engine(&self) -> Result<(), FpError> {
        use beebeeb_engine::app::SyncStop;
        use beebeeb_engine::finder_setup::error::app_code;
        match self.engine.stop() {
            SyncStop::Unconfirmed => Err(FpError::app(
                app_code::ENGINE_STOP_UNCONFIRMED,
                "the engine stop was not confirmed",
            )),
            _ => Ok(()),
        }
    }
    /// The engine hands the shell no writer for the debt or the gone mark (`OwnedState` has neither; reported): until
    /// it does, a confirmed removal clears nothing on disk.
    fn clear_removal_owed(&self) {}
    fn clear_domain_gone(&self) {}
    fn changed(&self) {
        self.engine.wake_host();
    }
}

#[cfg(test)]
#[path = "finder_setup_tests.rs"]
mod tests;
