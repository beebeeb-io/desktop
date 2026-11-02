//! The macOS `Ports` (spec §9): the bridge, the engine, the app's event bus, the lifecycle log
//! and `desktop.toml`. The only part of the reconciler that touches the OS; every decision is in
//! `core`.
//!
//! Every call that can wait on `fileproviderd` or the engine's lock has a limit from
//! `driver::op_limit` (lead ruling T7-1), so a stuck call ends in a `timeout` failure instead of a
//! reconciler that never answers sign-out or lock. How the limit applies depends on what a cut-off
//! would leave behind:
//!
//! - A bridge call is cut off by `driver::within`. Its OS call cannot be cancelled, so the
//!   [`BridgeGate`] lets exactly one run at a time and frees its slot only when the OS call returns.
//! - An engine start or stop is NOT cut off as a whole: a future dropped after it spawned an engine
//!   loses the `started` answer, and one dropped after it took the engine out of its slot detaches
//!   the engine task, which keeps running untracked with the session's keys. Their limit covers the
//!   wait for the engine slot only (`lib.rs`: `start_check_engine`, `stop_check_engine`).

use std::future::Future;
use std::sync::Arc;

use tauri::{Emitter, Manager};
use tokio::sync::{Semaphore, oneshot};

use super::core::{Observation, Op, OpResult, SessionFacts};
use super::driver::{FINDER_SETUP_CHANGED_EVENT, FinderSetupView, Ports, op_limit, within};
use super::error::{FailureRecord, FpError, app_code};
use crate::AppState;
use crate::config::DesktopConfig;
use crate::finder_removal::{DomainRemoval, KeptFolder};
use crate::lifecycle_log::{self, LifecycleEvent};
use crate::macos_file_provider::{self, BridgeRemovalFailure};
use crate::macos_workspace;

pub struct MacosPorts {
    app: tauri::AppHandle,
    gate: BridgeGate,
}

impl MacosPorts {
    pub fn new(app: tauri::AppHandle) -> Self {
        // The process-wide gate, not a private one: the reconciler's calls and every other bridge call
        // in the app share it (lead ruling T8-gate-all).
        Self {
            app,
            gate: BridgeGate::shared(),
        }
    }
}

/// How long the removal and the launch read wait for a busy gate (Task 9 fix round 2): long enough for a
/// bridge call that is merely in flight (a `getDomains` read, the startup sweep, a working-set signal) to
/// return, short enough that a stuck `fileproviderd` still ends in a `timeout`. Inside the operation's
/// own limit (`op_limit`: 10 s for `Observe`, 15 s for `RemoveDomain`), so it adds nothing to the bound
/// Lock and Sign-out derive their acknowledgement waits from. The two reads a person waits on ("Open in
/// Finder" and the `userEnabled` read) wait for the same time (F7 follow-up): they have no operation limit.
const BRIDGE_GATE_WAIT: std::time::Duration = std::time::Duration::from_secs(3);

/// One bridge call at a time, on a blocking thread off the async executor (Task 8 fix round 1).
///
/// Several ObjC primitives wait forever for `fileproviderd`, and the driver cuts a call off at its
/// limit, but a cut-off cannot cancel the OS call: its thread keeps waiting. Without a gate every
/// cut-off call would leave one more such thread (the UserDisabled poll alone would fill the
/// blocking pool in about an hour and a half while `fileproviderd` is stuck), and a second
/// `addDomain` or `removeDomain` could run beside the stuck one and put Beebeeb back in Finder after
/// sign-out.
///
/// The permit is taken BEFORE the blocking spawn and moved into the closure, so it is released when
/// the OS call returns and at no earlier point, whatever happens to the future waiting for it. A
/// busy gate fails at once with `OP_TIMEOUT` (it never queues behind a stuck call), except for the
/// calls that must not be lost to a call that is merely in flight: a removal (sign-out, Repair), which
/// the core never retries, and the launch read that heals a leftover domain. Those use
/// [`BridgeGate::run_waiting`], which waits a bounded time for the permit without a thread of its own.
/// The two reads a person clicked and waits on ("Open in Finder", the `userEnabled` read) use its
/// synchronous form, [`BridgeGate::run_sync_waiting`].
#[derive(Clone)]
struct BridgeGate(Arc<Semaphore>);

impl BridgeGate {
    fn new() -> Self {
        Self(Arc::new(Semaphore::new(1)))
    }

    /// The one gate for the whole process (lead ruling T8-gate-all). The reconciler's operations, the
    /// startup sweep, the working-set signal and the reads for "Open in Finder" all take this permit,
    /// so no two File Provider calls ever run at once, and none starts beside a stuck one. Tests that
    /// need a gate of their own use [`BridgeGate::new`].
    pub(crate) fn shared() -> Self {
        static SHARED: std::sync::OnceLock<BridgeGate> = std::sync::OnceLock::new();
        SHARED.get_or_init(Self::new).clone()
    }

    /// For a caller that is already on its own thread and calls the bridge directly: the permit is
    /// taken before the call and released when it returns, so it is held for exactly as long as the
    /// OS call runs. A busy gate fails at once, like [`BridgeGate::run`]; it never queues.
    pub(crate) fn run_sync<T>(&self, call: impl FnOnce() -> Result<T, FpError>) -> Result<T, FpError> {
        let Ok(permit) = self.0.clone().try_acquire_owned() else {
            return Err(FpError::app(
                app_code::OP_TIMEOUT,
                "an earlier bridge call has not returned",
            ));
        };
        let result = call();
        drop(permit);
        result
    }

    /// Like [`BridgeGate::run_sync`], but a busy gate is waited for, at most `wait`, before it answers `OP_TIMEOUT`:
    /// [`BridgeGate::run_waiting`]'s wait, for a caller that cannot await. The call runs on the caller's thread, as
    /// with `run_sync`, and the caller's thread is blocked for the wait. The callers are synchronous: the work of two
    /// async commands and of the menu's "Open in Finder", each handed to the blocking pool so that no wait runs on the
    /// main thread (lib.rs `on_the_blocking_pool`), and the upload menu action inside a task, so neither `block_on`
    /// nor a tokio timer is available. The wait is therefore made here, parked on the semaphore's own FIFO queue.
    pub(crate) fn run_sync_waiting<T>(
        &self,
        wait: std::time::Duration,
        call: impl FnOnce() -> Result<T, FpError>,
    ) -> Result<T, FpError> {
        let permit = self.acquire_parked(wait)?;
        let result = call();
        drop(permit);
        result
    }

    /// The permit, waited for on this thread for at most `wait`. A waiter that gives up drops its place in the queue
    /// (and any permit already handed to it), as a cut-off `run_waiting` does.
    fn acquire_parked(&self, wait: std::time::Duration) -> Result<tokio::sync::OwnedSemaphorePermit, FpError> {
        use std::task::{Context, Poll, Wake, Waker};
        /// Wakes the parked caller when the semaphore hands it the permit.
        struct Unpark(std::thread::Thread);
        impl Wake for Unpark {
            fn wake(self: Arc<Self>) {
                self.0.unpark();
            }
        }
        let deadline = std::time::Instant::now() + wait;
        let waker = Waker::from(Arc::new(Unpark(std::thread::current())));
        let mut cx = Context::from_waker(&waker);
        // `unconstrained`: inside a task, that task's cooperative budget must not turn a free permit into a wait.
        let mut acquire = std::pin::pin!(tokio::task::unconstrained(self.0.clone().acquire_owned()));
        loop {
            if let Poll::Ready(acquired) = acquire.as_mut().poll(&mut cx) {
                return acquired
                    .map_err(|_closed| FpError::app(app_code::UNEXPECTED_BRIDGE_RETURN, "the bridge gate was closed"));
            }
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            if left.is_zero() {
                return Err(FpError::app(
                    app_code::OP_TIMEOUT,
                    "an earlier bridge call has not returned",
                ));
            }
            // A spurious or early wake-up only polls again.
            std::thread::park_timeout(left);
        }
    }

    async fn run<T: Send + 'static>(
        &self,
        call: impl FnOnce() -> Result<T, FpError> + Send + 'static,
    ) -> Result<T, FpError> {
        let Ok(permit) = self.0.clone().try_acquire_owned() else {
            return Err(FpError::app(
                app_code::OP_TIMEOUT,
                "an earlier bridge call has not returned",
            ));
        };
        Self::blocking_holding(permit, call).await
    }

    /// Like [`BridgeGate::run`], but a busy gate is waited for, at most `wait`, before it answers
    /// `OP_TIMEOUT`. The wait is a bounded async wait on the semaphore: no thread of its own, so nothing
    /// leaks when it times out, and tokio's semaphore is FIFO-fair, so a fail-fast caller cannot barge
    /// ahead of a waiter. A waiter that gives up leaves the queue.
    async fn run_waiting<T: Send + 'static>(
        &self,
        wait: std::time::Duration,
        call: impl FnOnce() -> Result<T, FpError> + Send + 'static,
    ) -> Result<T, FpError> {
        let permit = match tokio::time::timeout(wait, self.0.clone().acquire_owned()).await {
            Ok(Ok(permit)) => permit,
            Ok(Err(_closed)) => {
                return Err(FpError::app(
                    app_code::UNEXPECTED_BRIDGE_RETURN,
                    "the bridge gate was closed",
                ));
            }
            Err(_elapsed) => {
                return Err(FpError::app(
                    app_code::OP_TIMEOUT,
                    "an earlier bridge call has not returned",
                ));
            }
        };
        Self::blocking_holding(permit, call).await
    }

    /// The one blocking spawn: the permit moves into the closure and is dropped when the OS call returns.
    async fn blocking_holding<T: Send + 'static>(
        permit: tokio::sync::OwnedSemaphorePermit,
        call: impl FnOnce() -> Result<T, FpError> + Send + 'static,
    ) -> Result<T, FpError> {
        tauri::async_runtime::spawn_blocking(move || {
            let result = call();
            drop(permit);
            result
        })
        .await
        .unwrap_or_else(|error| {
            Err(FpError::app(
                app_code::UNEXPECTED_BRIDGE_RETURN,
                format!("bridge task failed: {error}"),
            ))
        })
    }

    #[cfg(test)]
    fn is_idle(&self) -> bool {
        self.0.available_permits() == 1
    }
}

// The bridge calls everyone else in the app makes (lead ruling T8-gate-all): each takes the shared
// gate, and a busy gate answers `OP_TIMEOUT` instead of running beside a stuck call: at once, or for the
// two reads a person waits on, after `BRIDGE_GATE_WAIT`. No other file names a bridge function;
// `no_file_provider_bridge_call_is_made_outside_the_bridge_gate` (lib.rs) pins it.

/// Whether `error` is the gate answering "busy" (an earlier bridge call has not returned), as opposed to
/// the OS failing. The engine's working-set signal owes its ids to the next tick in that case.
pub(crate) fn is_gate_busy(error: &FpError) -> bool {
    error.domain == super::error::APP_DOMAIN && error.code == app_code::OP_TIMEOUT
}

/// Run `f` with the shared gate held, as if a bridge call were in flight (tests only). The gate is one per
/// process and tests run in parallel, so another test may hold it for a moment: this waits for it, at
/// most 5 s, instead of failing on a gate a sibling test holds.
#[cfg(test)]
pub(crate) fn with_shared_gate_held<T>(f: impl FnOnce() -> T) -> T {
    BridgeGate::shared()
        .run_sync_waiting(std::time::Duration::from_secs(5), || Ok(f()))
        .expect("the shared gate is free within 5 s")
}

/// "Open in Finder" (task 1885): opens the Beebeeb folder in Finder through `NSWorkspace`, with the scoped URL macOS
/// returned for it, and returns when LaunchServices has answered. `Ok(false)` is "macOS reports no location". A person
/// clicked and waits for it, so the resolve waits a little for the gate: the `Ready` poll reads the domain every 3 s,
/// and a click that meets that read must not fail. The gate covers the resolve only (fix round 1, I1): see
/// [`resolve_then`].
pub(crate) fn open_location() -> Result<bool, FpError> {
    resolve_then(
        &BridgeGate::shared(),
        macos_file_provider::resolve_location,
        macos_workspace::open_scoped,
    )
}

/// "Show in Finder" for one item (task 1885): selects the item, by its File Provider identifier, in a Finder window.
/// The resolve waits a little for the gate, for the same reason as [`open_location`], and the gate is released before
/// the reveal.
pub(crate) fn reveal_item(item_id: &str) -> Result<bool, FpError> {
    resolve_then(
        &BridgeGate::shared(),
        || macos_file_provider::resolve_item(item_id),
        macos_workspace::reveal_scoped,
    )
}

/// A resolve under the gate, then the open, with the gate released in between (task 1885 fix round 1, I1).
///
/// The resolve is a File Provider call and takes the permit, waiting up to [`BRIDGE_GATE_WAIT`]. The open is
/// `NSWorkspace`, which is not, and which can stall for seconds in LaunchServices: a sign-out's removal waits only
/// `BRIDGE_GATE_WAIT` for the gate, is never retried, and would answer `OP_TIMEOUT` and leave Beebeeb in Finder if an
/// open held the permit across that stall. The permit is therefore dropped when `gate.run_sync_waiting` returns, before
/// `then` runs. `Ok(false)` is "no location": nothing is opened. Both errors are the caller's.
fn resolve_then<H>(
    gate: &BridgeGate,
    resolve: impl FnOnce() -> Result<Option<H>, FpError>,
    then: impl FnOnce(H) -> Result<(), FpError>,
) -> Result<bool, FpError> {
    let resolved = gate.run_sync_waiting(BRIDGE_GATE_WAIT, resolve)?;
    match resolved {
        None => Ok(false),
        Some(handle) => then(handle).map(|()| true),
    }
}

/// The domain's state, read from the OS (the `finder_domain_user_enabled` command). Waits a little for the gate,
/// for the same reason as [`open_location`].
pub(crate) fn domain_state() -> Result<super::core::DomainState, FpError> {
    BridgeGate::shared().run_sync_waiting(BRIDGE_GATE_WAIT, macos_file_provider::domain_state)
}

/// The engine's best-effort working-set signal (task 1697).
pub(crate) fn signal_working_set() -> Result<macos_file_provider::WorkingSetSignalOutcome, FpError> {
    BridgeGate::shared().run_sync(macos_file_provider::signal_working_set)
}

/// The startup sweep of stale domains (task 1698), off the async executor like the reconciler's calls.
pub(crate) async fn cleanup_stale_domains() -> Result<macos_file_provider::StaleDomainCleanup, FpError> {
    BridgeGate::shared()
        .run(macos_file_provider::cleanup_stale_domains)
        .await
}

/// What `macos_file_provider::remove` answers.
type Removal = Result<DomainRemoval, BridgeRemovalFailure>;

/// The removal's call for the gate (rebase re-review I1): it runs `remove` and sends the answer on `answer`.
///
/// The reconciler stops waiting for a removal at its op limit (`within`), but the OS call cannot be cut off: it goes
/// on, and when it returns, macOS may have kept files. The blocking task's return value is read by nobody after a
/// cut-off, so the answer goes on a channel instead:
/// - while the reconciler still listens, it gets the answer, and the driver hands the folder to the waiter or to
///   `Ports::kept_folder`, as before;
/// - once it has stopped listening, the send fails, and the blocking thread hands the folder to `late` itself.
///
/// Exactly one of the two gets it: [`removal_answer`] closes the channel before it reads it. The late sink runs before
/// the gate is freed, for a few milliseconds: one config save under the config-write lock, whose holders never wait
/// for the gate, and an alert that does not block.
fn answering(
    answer: oneshot::Sender<Removal>,
    remove: impl FnOnce() -> Removal + Send + 'static,
    late: impl FnOnce(KeptFolder) + Send + 'static,
) -> impl FnOnce() -> Result<(), FpError> + Send + 'static {
    move || {
        if let Err(unheard) = answer.send(remove()) {
            late(match unheard {
                Ok(removal) => removal.kept,
                Err(failure) => failure.kept,
            });
        }
        Ok(())
    }
}

/// What the reconciler makes of a removal run with [`answering`], once `within` returned `cut`.
///
/// The channel is closed first. A removal that finishes from then on cannot send, so it goes to its late sink. An
/// answer that is already there is read, also when `within` cut the call off just after it sent: the domain is gone,
/// and saying so is the truth (`tokio::time::timeout` itself prefers a ready answer to its deadline).
fn removal_answer(cut: Result<(), FpError>, mut answered: oneshot::Receiver<Removal>) -> OpResult {
    answered.close();
    match (answered.try_recv(), cut) {
        (Ok(Ok(removal)), _) => OpResult::Removed(Ok(()), removal.kept),
        (Ok(Err(failure)), _) => OpResult::Removed(Err(failure.error), failure.kept),
        // Cut off (if the OS call keeps files later, they go to the late sink), a busy gate, or a failed task.
        (Err(_), Err(error)) => OpResult::Removed(Err(error), KeptFolder::default()),
        // The call returned without an answer, which `answering`'s call never does.
        (Err(_), Ok(())) => OpResult::Removed(
            Err(FpError::app(
                app_code::UNEXPECTED_BRIDGE_RETURN,
                "the removal returned no answer",
            )),
            KeptFolder::default(),
        ),
    }
}

/// A folder a reconciler removal kept, shown to the person the way the app-start sweep shows one: saved for the
/// Settings › Sync row, then the alert (task 1882). Used by the port for a removal nobody took the answer of, and by
/// the removal itself when it finished after the reconciler stopped waiting (rebase re-review I1). Logs never carry
/// the folder.
fn surface_reconciler_kept(app: &tauri::AppHandle, kept: KeptFolder, context: &'static str) {
    let location = DomainRemoval { kept }.kept_location(context);
    crate::surface_kept_folder(app, location.as_deref());
}

/// §5.1's facts. Fail safe: an unreadable config never counts as "signed out by choice", so it
/// never removes the domain.
fn facts_from(
    vault_unlocked: bool,
    auth_present: bool,
    removal_owed: bool,
    config: Result<DesktopConfig, String>,
) -> SessionFacts {
    let signed_out_by_choice = config.map(|cfg| cfg.finder_signed_out_by_choice).unwrap_or(false);
    SessionFacts {
        vault_unlocked,
        auth_present,
        signed_out_by_choice,
        removal_owed,
    }
}

fn session_facts(app: &tauri::AppHandle) -> SessionFacts {
    let state = app.state::<AppState>();
    let vault_unlocked = state
        .active_account()
        .ok()
        .and_then(|acct| acct.session.lock().ok().map(|guard| guard.is_some()))
        .unwrap_or(false);
    let auth_present = state.auth_present.lock().map(|guard| *guard).unwrap_or(false);
    facts_from(
        vault_unlocked,
        auth_present,
        crate::finder_removal_owed(),
        DesktopConfig::load(),
    )
}

/// Each returns whether `cfg` changed, so a repeat writes nothing.
fn set_failure(cfg: &mut DesktopConfig, record: Option<FailureRecord>) -> bool {
    if cfg.finder_last_failure == record {
        return false;
    }
    cfg.finder_last_failure = record;
    true
}

fn set_signed_out_by_choice(cfg: &mut DesktopConfig, value: bool) -> bool {
    if cfg.finder_signed_out_by_choice == value {
        return false;
    }
    cfg.finder_signed_out_by_choice = value;
    true
}

fn update_config(change: impl FnOnce(&mut DesktopConfig) -> bool) {
    match DesktopConfig::path() {
        Ok(path) => update_config_at(&path, change),
        Err(error) => tracing::warn!(%error, "finder setup: could not find desktop.toml"),
    }
}

/// [`update_config`] on the config file at `path` (a seam for the tests). One load-change-save under the
/// config-write lock (`DesktopConfig::update_at`, task 1882 r5; rebase re-review Minor 3): a load here and a save
/// after another writer's would put back the copy loaded first, and the kept folder a Repair saved meanwhile would be
/// lost. A change that is not saved leaves the file untouched.
fn update_config_at(path: &std::path::Path, change: impl FnOnce(&mut DesktopConfig) -> bool) {
    if let Err(error) = DesktopConfig::update_at(path, |cfg| (change(cfg), ())) {
        tracing::warn!(%error, "finder setup: could not update desktop.toml");
    }
}

impl Ports for MacosPorts {
    fn run(&mut self, op: Op) -> impl Future<Output = OpResult> + Send {
        let app = self.app.clone();
        let gate = self.gate.clone();
        async move {
            let limit = op_limit(op);
            match op {
                Op::Observe => {
                    let facts = session_facts(&app);
                    // Waits a little for the gate: a launch read that fails to a call merely in flight (the
                    // startup sweep) would leave a domain that should be removed until the next launch.
                    let domain = within(
                        op,
                        limit,
                        gate.run_waiting(BRIDGE_GATE_WAIT, macos_file_provider::domain_state),
                    )
                    .await;
                    OpResult::Observed(Observation { facts, domain })
                }
                // Not `within`: `limit` bounds the wait for the engine slot inside, and nothing after
                // the engine is spawned (see the module comment).
                Op::StartEngine => OpResult::EngineStarted(
                    async {
                        let root = crate::config::default_sync_root_suggestion();
                        crate::config::ensure_directory(&root)
                            .map_err(|error| FpError::app(app_code::ENGINE_START, error))?;
                        let state = app.state::<AppState>();
                        let acct = state
                            .active_account()
                            .map_err(|error| FpError::app(app_code::ENGINE_START, error))?;
                        crate::start_check_engine(app.clone(), &state, &acct, root, limit).await
                    }
                    .await,
                ),
                Op::AddDomain => OpResult::Added(within(op, limit, gate.run(macos_file_provider::add_domain)).await),
                Op::ReadDomain => {
                    OpResult::Domain(within(op, limit, gate.run(macos_file_provider::domain_state)).await)
                }
                Op::WaitStable(timeout) => OpResult::Stable(
                    within(
                        op,
                        limit,
                        gate.run(move || macos_file_provider::wait_for_domain_ready(timeout)),
                    )
                    .await,
                ),
                // Cut off as a whole, which is safe only because `ensure_sync_root_and_engine` has one await,
                // the engine slot, before it spawns (pinned by a test in `lib.rs`).
                Op::FinishReady => OpResult::Finished(
                    within(op, limit, async {
                        let state = app.state::<AppState>();
                        let root = crate::config::default_sync_root_suggestion();
                        crate::ensure_sync_root_and_engine(app.clone(), &state, root)
                            .await
                            .map_err(|error| FpError::app(app_code::FINISH_READY, error))
                    })
                    .await,
                ),
                // Not `within`, for the same reason as `StartEngine`: once the engine is out of its slot
                // its `abort()` must run to the end (it gives up on its own after 5 s). `Err` when the
                // slot could not be taken in time or the stop could not be confirmed; the core never
                // claims a stop that did not happen.
                Op::StopEngine => OpResult::EngineStopped(match app.state::<AppState>().active_account() {
                    Ok(acct) => crate::stop_check_engine(&acct, limit).await,
                    Err(_) => Ok(()), // no account, so no engine
                }),
                // Waits a little for the gate (inside `limit`): the core never retries a removal, so one that
                // failed to a bridge call merely in flight would leave the domain and its replica in Finder
                // after sign-out. Task 1882: the removal keeps the files that never reached the server, and
                // the answer carries the folder macOS kept them in, also when the removal failed (review M2).
                // Rebase re-review I1: a removal that finishes after `limit` still shows its folder, from the
                // blocking thread (`answering`).
                Op::RemoveDomain => {
                    let late_app = app.clone();
                    let late =
                        move |kept| surface_reconciler_kept(&late_app, kept, "finder reconciler, after its time limit");
                    let (answer, answered) = oneshot::channel();
                    let cut = within(
                        op,
                        limit,
                        gate.run_waiting(BRIDGE_GATE_WAIT, answering(answer, macos_file_provider::remove, late)),
                    )
                    .await;
                    removal_answer(cut, answered)
                }
            }
        }
    }

    fn publish(&mut self, view: &FinderSetupView) {
        if let Err(error) = self.app.emit(FINDER_SETUP_CHANGED_EVENT, view) {
            tracing::warn!(%error, "finder setup: could not emit the changed event");
        }
    }

    fn log(&mut self, event: LifecycleEvent) {
        lifecycle_log::event(event);
    }

    fn persist_failure(&mut self, record: Option<FailureRecord>) {
        update_config(move |cfg| set_failure(cfg, record));
    }

    fn persist_signed_out_by_choice(&mut self, value: bool) {
        update_config(move |cfg| set_signed_out_by_choice(cfg, value));
    }

    fn domain_removal_confirmed(&mut self) {
        crate::clear_finder_removal_owed();
    }

    fn domain_added(&mut self) {
        crate::clear_finder_domain_gone();
    }

    /// Task 1882: a removal nobody waited for (or a waiter that gave up) is shown like the app-start sweep's:
    /// saved for the Settings › Sync row, then the alert. Logs never carry the folder.
    fn kept_folder(&mut self, kept: KeptFolder) {
        surface_reconciler_kept(&self.app, kept, "finder reconciler");
    }

    fn window_visible(&self) -> bool {
        self.app
            .webview_windows()
            .values()
            .any(|window| window.is_visible().unwrap_or(false))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source_pin::squeeze;
    use crate::surfaces::phase::FinderFailureReason;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    fn production() -> String {
        let all = include_str!("macos_ports.rs").replace("\r\n", "\n");
        all[..all.find("#[cfg(test)]\nmod tests {").expect("the tests follow")].to_string()
    }

    /// The text of one `Op::` arm of `Ports::run`: from its pattern to the next arm's.
    fn arm<'a>(run: &'a str, pattern: &str) -> &'a str {
        let from = run.find(pattern).unwrap_or_else(|| panic!("an arm for {pattern}"));
        let rest = &run[from + pattern.len()..];
        let to = rest.find("\n                Op::").map_or(rest.len(), |at| at);
        &rest[..to]
    }

    mod config;
    mod gate;
}