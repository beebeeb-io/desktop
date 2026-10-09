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
use tokio::sync::Semaphore;

use super::core::{Observation, Op, OpResult, SessionFacts};
use super::driver::{FINDER_SETUP_CHANGED_EVENT, FinderSetupView, Ports, op_limit, within};
use super::error::{FailureRecord, FpError, app_code};
use crate::AppState;
use crate::config::DesktopConfig;
use crate::finder_removal::KeptFolder;
use crate::lifecycle_log::{self, LifecycleEvent};
use crate::macos_file_provider;

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
/// Lock and Sign-out derive their acknowledgement waits from.
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
/// busy gate fails at once with `OP_TIMEOUT` (it never queues behind a stuck call), except for the two
/// calls that must not be lost to a call that is merely in flight: a removal (sign-out, Repair), which
/// the core never retries, and the launch read that heals a leftover domain. Those use
/// [`BridgeGate::run_waiting`], which waits a bounded time for the permit without a thread of its own.
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
// gate, and a busy gate answers `OP_TIMEOUT` at once instead of running beside a stuck call. No
// other file names a bridge function; `no_file_provider_bridge_call_is_made_outside_the_bridge_gate`
// (lib.rs) pins it.

/// Whether `error` is the gate answering "busy" (an earlier bridge call has not returned), as opposed to
/// the OS failing. The engine's working-set signal owes its ids to the next tick in that case.
pub(crate) fn is_gate_busy(error: &FpError) -> bool {
    error.domain == super::error::APP_DOMAIN && error.code == app_code::OP_TIMEOUT
}

/// Run `f` with the shared gate held, as if a bridge call were in flight (tests only).
#[cfg(test)]
pub(crate) fn with_shared_gate_held<T>(f: impl FnOnce() -> T) -> T {
    BridgeGate::shared()
        .run_sync(|| Ok(f()))
        .expect("the shared gate is free at the start of the test")
}

/// The URL of the Beebeeb folder in Finder, for "Open in Finder".
pub(crate) fn visible_url() -> Result<Option<String>, FpError> {
    BridgeGate::shared().run_sync(macos_file_provider::visible_url)
}

/// The domain's state, read from the OS (the `finder_domain_user_enabled` command).
pub(crate) fn domain_state() -> Result<super::core::DomainState, FpError> {
    BridgeGate::shared().run_sync(macos_file_provider::domain_state)
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
    match DesktopConfig::load() {
        Ok(mut cfg) => {
            if change(&mut cfg)
                && let Err(error) = cfg.save()
            {
                tracing::warn!(%error, "finder setup: could not save desktop.toml");
            }
        }
        Err(error) => tracing::warn!(%error, "finder setup: could not load desktop.toml"),
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
                Op::RemoveDomain => match within(
                    op,
                    limit,
                    gate.run_waiting(BRIDGE_GATE_WAIT, || Ok(macos_file_provider::remove())),
                )
                .await
                {
                    Ok(Ok(removal)) => OpResult::Removed(Ok(()), removal.kept),
                    Ok(Err(failure)) => OpResult::Removed(Err(failure.error), failure.kept),
                    Err(error) => OpResult::Removed(Err(error), KeptFolder::default()),
                },
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
        let location = crate::finder_removal::DomainRemoval { kept }.kept_location("finder reconciler");
        crate::surface_kept_folder(&self.app, location.as_deref());
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

    /// Lead ruling T7-1, shape corrected in fix round 1: a bridge call is cut off by `within`; an
    /// engine start or stop is NOT, because a future dropped after it spawned or took the engine
    /// loses it (the engine would keep running untracked, holding the session's keys). Those two
    /// limit only their wait for the engine slot, inside `start_check_engine` / `stop_check_engine`.
    #[test]
    fn every_operation_is_bounded_in_the_shape_that_cannot_orphan_an_engine() {
        let source = production();
        let run = &source[source.find("fn run(&mut self, op: Op)").expect("run")..];
        let run = &run[..run.find("fn publish(").expect("publish follows run")];
        assert!(
            run.contains("let limit = op_limit(op);"),
            "one limit per operation, from the driver's table"
        );
        for pattern in [
            "Op::Observe =>",
            "Op::AddDomain =>",
            "Op::ReadDomain =>",
            "Op::WaitStable(timeout) =>",
            "Op::RemoveDomain =>",
        ] {
            let body = arm(run, pattern);
            // Read through `squeeze`: rustfmt wraps `within(op, limit, ..)` over lines.
            assert!(
                squeeze(body).contains(&squeeze("within(op, limit,")),
                "{pattern} is not bounded:\n{body}"
            );
            assert!(
                body.contains("gate.run(") || body.contains("gate.run_waiting("),
                "{pattern} does not go through the bridge gate:\n{body}"
            );
        }
        // The engine slot is the only await of `ensure_sync_root_and_engine` (pinned in lib.rs), so
        // cutting the whole of FinishReady off can only cut off that wait.
        assert!(squeeze(arm(run, "Op::FinishReady =>")).contains(&squeeze("within(op, limit,")));
        for (pattern, call) in [
            ("Op::StartEngine =>", "start_check_engine("),
            ("Op::StopEngine =>", "stop_check_engine("),
        ] {
            let body = arm(run, pattern);
            assert!(
                body.contains(call) && body.contains("limit"),
                "{pattern} must pass its limit to {call}:\n{body}"
            );
            assert!(
                !body.contains("within("),
                "{pattern} must not be wrapped in a cut-off:\n{body}"
            );
        }
        // And no bridge call is made outside the gate and `within`.
        assert!(
            run.matches("gate.run(").count() + run.matches("gate.run_waiting(").count() >= 5,
            "the bridge calls are in `run`"
        );
        for call in run
            .match_indices("gate.run(")
            .chain(run.match_indices("gate.run_waiting("))
        {
            // The call is the third argument of the nearest `within(` before it, however rustfmt wraps that call.
            let before = &run[..call.0];
            let opened = before.rfind("within(").unwrap_or(0);
            assert_eq!(
                squeeze(&before[opened..]),
                squeeze("within(op, limit,"),
                "an unbounded bridge call, after: {}",
                &before[opened..]
            );
        }
        assert_eq!(
            source.matches("spawn_blocking").count(),
            1,
            "the one blocking spawn is the gate's"
        );
    }

    /// Fix round 1 (finding 2): the gate's permit is taken BEFORE the blocking spawn and moved
    /// into the blocking closure, so it is released when the OS call returns and at no earlier
    /// point (a cut-off, a drop of the waiting future).
    #[test]
    fn the_gate_takes_its_permit_before_the_spawn_and_the_blocking_closure_owns_it() {
        let source = production();
        // Both ways in take the permit first and hand it to the ONE blocking spawn, whose closure owns it.
        let blocking = &source[source
            .find("async fn blocking_holding<T: Send + 'static>")
            .expect("the one blocking spawn")..];
        let blocking = &blocking[..blocking.find("\n    }\n").expect("it ends")];
        assert!(blocking.contains("spawn_blocking("), "{blocking}");
        assert!(
            blocking.contains("move ||") && blocking.contains("drop(permit)"),
            "the closure owns and drops the permit:\n{blocking}"
        );
        let fail_fast = &source[source.find("async fn run<T: Send + 'static>").expect("the gate's run")..];
        let fail_fast = &fail_fast[..fail_fast.find("\n    }\n").expect("run ends")];
        let acquire = fail_fast
            .find("try_acquire_owned()")
            .expect("the permit is taken without queueing");
        assert!(
            acquire < fail_fast.find("blocking_holding(").expect("then handed to the spawn"),
            "{fail_fast}"
        );
        // Read through `squeeze`: rustfmt puts `.await` on a line of its own in a longer chain.
        assert!(
            !squeeze(fail_fast).contains(&squeeze("acquire_owned().await")),
            "a busy gate fails at once, it never queues:\n{fail_fast}"
        );
        let waiting = &source[source
            .find("async fn run_waiting<T: Send + 'static>")
            .expect("the waiting run")..];
        let waiting = &waiting[..waiting.find("\n    }\n").expect("run_waiting ends")];
        assert!(
            waiting.find("acquire_owned()").expect("it waits for the permit")
                < waiting.find("blocking_holding(").expect("then spawns"),
            "{waiting}"
        );
        assert!(
            waiting.contains("tokio::time::timeout("),
            "the wait is bounded:\n{waiting}"
        );
        assert!(
            !waiting.contains("spawn_blocking"),
            "waiting costs no thread:\n{waiting}"
        );
    }

    async fn until(what: &str, mut done: impl FnMut() -> bool) {
        for _ in 0..1000 {
            if done() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("timed out waiting for: {what}");
    }

    /// Fix round 1 (finding 2): ONE bridge call at a time. A call stuck in the OS holds the gate;
    /// the next call fails at once with `OP_TIMEOUT`, queues behind nothing, and spawns no thread.
    #[tokio::test]
    async fn a_stuck_bridge_call_holds_the_gate_so_the_next_call_fails_fast_and_spawns_nothing() {
        let gate = BridgeGate::new();
        let (release, hold) = std::sync::mpsc::channel::<()>();
        let started = Arc::new(AtomicUsize::new(0));
        let stuck = {
            let (gate, started) = (gate.clone(), started.clone());
            tokio::spawn(async move {
                gate.run(move || {
                    started.fetch_add(1, Ordering::SeqCst);
                    let _ = hold.recv();
                    Ok(1)
                })
                .await
            })
        };
        until("the stuck call to be inside the OS", || {
            started.load(Ordering::SeqCst) == 1
        })
        .await;

        let ran = Arc::new(AtomicUsize::new(0));
        let ran_by_second = ran.clone();
        let second = tokio::time::timeout(
            Duration::from_secs(5),
            gate.run(move || {
                ran_by_second.fetch_add(1, Ordering::SeqCst);
                Ok(2)
            }),
        )
        .await
        .expect("a busy gate fails at once; it never waits behind the stuck call");
        let error = second.expect_err("the gate is held");
        assert_eq!(
            (error.domain.as_str(), error.code),
            (crate::finder_setup::error::APP_DOMAIN, app_code::OP_TIMEOUT)
        );
        assert_eq!(error.message, "an earlier bridge call has not returned");
        assert_eq!(
            crate::finder_setup::policy::classify(&error),
            crate::surfaces::phase::FinderFailureReason::Timeout
        );
        assert_eq!(ran.load(Ordering::SeqCst), 0, "no second blocking call was started");

        release.send(()).unwrap();
        assert_eq!(stuck.await.unwrap(), Ok(1));
    }

    /// Fix round 1 (finding 2): the permit is released when the stuck call RETURNS, not when the
    /// driver's limit cut it off. Between the two the OS call is still running, and a second one
    /// must not start beside it.
    #[tokio::test]
    async fn the_gate_is_released_when_the_stuck_call_returns_not_when_it_was_cut_off() {
        let gate = BridgeGate::new();
        let (release, hold) = std::sync::mpsc::channel::<()>();
        let started = Arc::new(AtomicUsize::new(0));
        let inside = started.clone();
        // Cut off at 30 ms, as the driver cuts off an `addDomain` that never answers.
        let cut = crate::finder_setup::driver::within(
            Op::AddDomain,
            Duration::from_millis(30),
            gate.run(move || {
                inside.fetch_add(1, Ordering::SeqCst);
                let _ = hold.recv();
                Ok(())
            }),
        )
        .await;
        assert_eq!(
            cut.expect_err("cut off").message,
            "AddDomain did not answer within 30ms"
        );
        until("the cut-off call to be inside the OS", || {
            started.load(Ordering::SeqCst) == 1
        })
        .await;

        // The driver gave up; the OS call did not. The gate is still held.
        assert!(
            !gate.is_idle(),
            "the cut-off must not free the gate while the OS call is still running"
        );
        let ran = Arc::new(AtomicUsize::new(0));
        let ran_by_next = ran.clone();
        let next = gate
            .run(move || {
                ran_by_next.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
            .await;
        assert_eq!(next.expect_err("still held").code, app_code::OP_TIMEOUT);
        assert_eq!(
            ran.load(Ordering::SeqCst),
            0,
            "no second bridge call ran beside the stuck one"
        );

        // The OS call finally returns: now, and only now, the gate is free.
        release.send(()).unwrap();
        until("the gate to be released", || gate.is_idle()).await;
        assert_eq!(gate.run(|| Ok(5)).await, Ok(5));
    }

    /// Lead ruling T8-gate-all: one gate for the process. A synchronous caller (the engine's working-set
    /// signal, "Open in Finder") takes the same permit as the reconciler, holds it for the whole call and
    /// frees it after, and a busy gate fails at once.
    #[tokio::test]
    async fn a_synchronous_bridge_call_shares_the_one_gate() {
        assert!(
            Arc::ptr_eq(&BridgeGate::shared().0, &BridgeGate::shared().0),
            "one gate for the process"
        );
        let gate = BridgeGate::new();
        // Inside the call the gate is held, so a second synchronous call is refused at once.
        let inner = gate.clone();
        let result = gate.run_sync(|| {
            assert!(!gate.is_idle(), "the permit is held while the OS call runs");
            Ok(inner.run_sync(|| Ok(1)))
        });
        let nested = result.expect("the outer call ran").expect_err("the gate was held");
        assert_eq!(
            (nested.domain.as_str(), nested.code),
            (crate::finder_setup::error::APP_DOMAIN, app_code::OP_TIMEOUT)
        );
        assert!(gate.is_idle(), "the permit is back after the call");
        assert_eq!(gate.run_sync(|| Ok(7)), Ok(7));

        // A call stuck in the async path holds it against a synchronous one, too.
        let (release, hold) = std::sync::mpsc::channel::<()>();
        let started = Arc::new(AtomicUsize::new(0));
        let inside = started.clone();
        let stuck = {
            let gate = gate.clone();
            tokio::spawn(async move {
                gate.run(move || {
                    inside.fetch_add(1, Ordering::SeqCst);
                    let _ = hold.recv();
                    Ok(())
                })
                .await
            })
        };
        until("the stuck call to be inside the OS", || {
            started.load(Ordering::SeqCst) == 1
        })
        .await;
        let ran = Arc::new(AtomicUsize::new(0));
        let ran_by_sync = ran.clone();
        let refused = gate
            .run_sync(move || {
                ran_by_sync.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
            .expect_err("a synchronous call does not run beside a stuck one");
        assert_eq!(refused.code, app_code::OP_TIMEOUT);
        assert_eq!(ran.load(Ordering::SeqCst), 0);
        release.send(()).unwrap();
        stuck.await.unwrap().unwrap();
    }

    /// The reconciler and everyone else share ONE gate: the ports take it from `shared()`.
    #[test]
    fn the_ports_use_the_shared_gate_and_the_wrappers_are_the_only_other_callers() {
        let source = production();
        let new = &source[source.find("pub fn new(app: tauri::AppHandle)").expect("new")..];
        let new = &new[..new.find("\n    }\n").unwrap()];
        assert!(
            new.contains("BridgeGate::shared()") && !new.contains("BridgeGate::new()"),
            "{new}"
        );
        for wrapper in [
            "fn visible_url(",
            "fn domain_state(",
            "fn signal_working_set(",
            "fn cleanup_stale_domains(",
        ] {
            let body = &source[source.find(wrapper).unwrap_or_else(|| panic!("{wrapper}"))..];
            let body = &body[..body.find("\n}\n").unwrap()];
            assert!(
                body.contains("BridgeGate::shared()"),
                "{wrapper} takes the shared gate:\n{body}"
            );
        }
    }

    // ---- Task 9 fix round 2: the removal and the launch read wait a little for the gate ----

    /// A removal that finds the gate busy for a second still happens: it waits for the permit (no thread
    /// of its own while it waits) instead of failing at once. Before this, sign-out left the domain in
    /// Finder whenever a bridge call happened to be in flight, and the core never retries a removal.
    #[tokio::test]
    async fn a_waiting_call_gets_the_gate_when_the_holder_returns_inside_the_wait() {
        let gate = BridgeGate::new();
        let (release, hold) = std::sync::mpsc::channel::<()>();
        let started = Arc::new(AtomicUsize::new(0));
        let inside = started.clone();
        let holder = {
            let gate = gate.clone();
            tokio::spawn(async move {
                gate.run(move || {
                    inside.fetch_add(1, Ordering::SeqCst);
                    let _ = hold.recv();
                    Ok(())
                })
                .await
            })
        };
        until("the holder to be inside the OS", || started.load(Ordering::SeqCst) == 1).await;
        // The holder returns after one second.
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(1)).await;
            let _ = release.send(());
        });
        let began = std::time::Instant::now();
        let waited = gate.run_waiting(BRIDGE_GATE_WAIT, || Ok(5)).await;
        assert_eq!(
            waited,
            Ok(5),
            "a gate busy for 1 s inside a {BRIDGE_GATE_WAIT:?} wait does not fail the call"
        );
        assert!(
            began.elapsed() >= Duration::from_millis(900),
            "it really waited: {:?}",
            began.elapsed()
        );
        assert!(
            began.elapsed() < BRIDGE_GATE_WAIT,
            "and did not wait out the whole bound: {:?}",
            began.elapsed()
        );
        holder.await.unwrap().unwrap();
        assert!(gate.is_idle(), "the permit is back");
    }

    /// Busy for longer than the wait: the call fails with the busy error, runs nothing and spawns nothing.
    #[tokio::test]
    async fn a_waiting_call_that_outlasts_the_wait_fails_with_the_busy_error_and_runs_nothing() {
        let gate = BridgeGate::new();
        let (release, hold) = std::sync::mpsc::channel::<()>();
        let started = Arc::new(AtomicUsize::new(0));
        let inside = started.clone();
        let holder = {
            let gate = gate.clone();
            tokio::spawn(async move {
                gate.run(move || {
                    inside.fetch_add(1, Ordering::SeqCst);
                    let _ = hold.recv();
                    Ok(())
                })
                .await
            })
        };
        until("the holder to be inside the OS", || started.load(Ordering::SeqCst) == 1).await;
        let ran = Arc::new(AtomicUsize::new(0));
        let ran_by_waiter = ran.clone();
        let began = std::time::Instant::now();
        let error = gate
            .run_waiting(Duration::from_millis(200), move || {
                ran_by_waiter.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
            .await
            .expect_err("the holder never returned inside the wait");
        assert_eq!(
            (error.domain.as_str(), error.code),
            (crate::finder_setup::error::APP_DOMAIN, app_code::OP_TIMEOUT)
        );
        assert_eq!(error.message, "an earlier bridge call has not returned");
        assert!(is_gate_busy(&error));
        assert!(
            began.elapsed() >= Duration::from_millis(200),
            "it waited its bound: {:?}",
            began.elapsed()
        );
        assert!(
            began.elapsed() < Duration::from_secs(5),
            "and no longer: {:?}",
            began.elapsed()
        );
        assert_eq!(ran.load(Ordering::SeqCst), 0, "nothing ran beside the stuck call");
        assert!(!gate.is_idle(), "the stuck call still holds the permit");
        // The waiter left the queue: when the holder returns the gate is free for the next call.
        release.send(()).unwrap();
        holder.await.unwrap().unwrap();
        until("the gate to be released", || gate.is_idle()).await;
        assert_eq!(gate.run(|| Ok(7)).await, Ok(7));
    }

    /// Lead ruling (fix round 2): ONLY the removal and the launch read wait. The working-set signal, the
    /// other reads, the UserDisabled poll and the startup sweep keep failing fast, so a busy gate never
    /// stalls them and the engine's tick never waits.
    #[tokio::test]
    async fn everything_but_the_removal_and_the_launch_read_still_fails_fast() {
        // The signal and the reads: `run_sync` on the shared gate, held. They answer at once.
        super::with_shared_gate_held(|| {
            let began = std::time::Instant::now();
            for busy in [
                signal_working_set().map(|_| ()),
                visible_url().map(|_| ()),
                domain_state().map(|_| ()),
            ] {
                assert!(is_gate_busy(&busy.expect_err("the gate is held")));
            }
            assert!(
                began.elapsed() < Duration::from_millis(500),
                "they failed fast: {:?}",
                began.elapsed()
            );
        });
        // And by shape: exactly two bridge calls in the reconciler wait, and they are the ones named.
        let source = production();
        let run = &source[source.find("fn run(&mut self, op: Op)").expect("run")..];
        let run = &run[..run.find("fn publish(").expect("publish follows run")];
        assert_eq!(
            run.matches("gate.run_waiting(BRIDGE_GATE_WAIT,").count(),
            2,
            "Observe and RemoveDomain, nothing else"
        );
        for waits in ["Op::Observe =>", "Op::RemoveDomain =>"] {
            assert!(
                arm(run, waits).contains("gate.run_waiting(BRIDGE_GATE_WAIT,"),
                "{waits} waits for the gate"
            );
        }
        for fails_fast in ["Op::AddDomain =>", "Op::ReadDomain =>", "Op::WaitStable(timeout) =>"] {
            let body = arm(run, fails_fast);
            assert!(
                body.contains("gate.run(") && !body.contains("run_waiting"),
                "{fails_fast} fails fast:\n{body}"
            );
        }
        for wrapper in [
            "fn visible_url(",
            "fn domain_state(",
            "fn signal_working_set(",
            "fn cleanup_stale_domains(",
        ] {
            let body = &source[source.find(wrapper).unwrap()..];
            let body = &body[..body.find("\n}\n").unwrap()];
            assert!(!body.contains("run_waiting"), "{wrapper} fails fast:\n{body}");
        }
    }

    /// The wait sits INSIDE `within`'s limit for both operations, so it adds nothing to the bound the
    /// Lock and Sign-out acknowledgement waits are derived from (`FINDER_LOCK_TIMEOUT`: 38 s for the longest
    /// operation, `FINDER_REMOVE_TIMEOUT`: that plus `RemoveDomain`'s 15 s): the operation's limit is
    /// unchanged and still covers the wait and the call.
    #[test]
    fn the_gate_wait_fits_inside_the_limits_it_runs_under() {
        use crate::finder_setup::core::Op;
        use crate::finder_setup::driver::op_limit;
        assert!(BRIDGE_GATE_WAIT <= Duration::from_secs(3));
        assert!(
            BRIDGE_GATE_WAIT < op_limit(Op::Observe),
            "inside Observe's {:?}",
            op_limit(Op::Observe)
        );
        assert!(
            BRIDGE_GATE_WAIT < op_limit(Op::RemoveDomain),
            "inside RemoveDomain's {:?}",
            op_limit(Op::RemoveDomain)
        );
        assert_eq!(
            op_limit(Op::RemoveDomain),
            Duration::from_secs(15),
            "the limit the 53 s derivation uses is unchanged"
        );
        assert_eq!(op_limit(Op::Observe), Duration::from_secs(10));
        // `within` wraps the waiting call, so the wait is part of the limit, not added to it.
        let source = production();
        for (pattern, call) in [
            ("Op::Observe =>", "macos_file_provider::domain_state"),
            ("Op::RemoveDomain =>", "|| Ok(macos_file_provider::remove())"),
        ] {
            let run = &source[source.find("fn run(&mut self, op: Op)").unwrap()..];
            let body = arm(&run[..run.find("fn publish(").unwrap()], pattern);
            // Read through `squeeze`: rustfmt wraps `within(op, limit, ..)` over lines.
            let squeezed = squeeze(body);
            let within = squeezed.find(&squeeze("within(op, limit,")).expect("bounded");
            assert!(
                squeezed[within..].contains(&squeeze(&format!("gate.run_waiting(BRIDGE_GATE_WAIT, {call}"))),
                "{pattern}: the wait is inside `within`:\n{body}"
            );
        }
    }

    /// Spec §6.1, lead ruling 11: nothing the OS said leaves the ports except through the typed
    /// view, the redacted lifecycle log and stdout tracing.
    #[test]
    fn no_os_message_leaves_the_ports_except_to_the_log_and_stdout() {
        let source = production();
        assert!(!source.contains(".message"), "the ports never read an OS message");
        assert!(
            !source.contains("error.to_string()"),
            "no Display text of an error is built"
        );
        let publish = &source[source.find("fn publish(").unwrap()..];
        let publish = &publish[..publish.find("fn log(").unwrap()];
        assert!(
            publish.contains("emit(FINDER_SETUP_CHANGED_EVENT, view)"),
            "the event is the whole view, nothing else:\n{publish}"
        );
        // `FailureRecord` is what reaches `desktop.toml`: a reason, a domain, a code, a time.
        let persist = &source[source.find("fn persist_failure(").unwrap()..];
        assert!(persist.starts_with("fn persist_failure(&mut self, record: Option<FailureRecord>)"));
    }

    /// F3: the macOS port releases the persisted "removal owed" flag when the core says a removal is confirmed, and
    /// the facts the core observes carry the same flag the engine start reads.
    #[test]
    fn the_macos_port_releases_the_removal_debt_and_the_facts_carry_it() {
        let source = include_str!("macos_ports.rs").replace("\r\n", "\n");
        let source = &source[..source.find("#[cfg(test)]\nmod tests {").unwrap()];
        let release = &source[source
            .find("fn domain_removal_confirmed(")
            .expect("the port implements it")..];
        let release = &release[..release.find("\n    }\n").unwrap()];
        assert!(release.contains("crate::clear_finder_removal_owed();"), "{release}");
        let facts = &source[source.find("fn session_facts(").unwrap()..];
        let facts = &facts[..facts.find("\n}\n").unwrap()];
        assert!(facts.contains("crate::finder_removal_owed()"), "{facts}");
    }

    /// Fix round 3: the macOS port clears the persisted "domain gone" mark when the core says the domain was
    /// registered.
    #[test]
    fn the_macos_port_clears_the_domain_gone_mark_when_the_domain_is_added() {
        let source = include_str!("macos_ports.rs").replace("\r\n", "\n");
        let source = &source[..source.find("#[cfg(test)]\nmod tests {").unwrap()];
        let added = &source[source.find("fn domain_added(").expect("the port implements it")..];
        let added = &added[..added.find("\n    }\n").unwrap()];
        assert!(added.contains("crate::clear_finder_domain_gone();"), "{added}");
    }

    #[test]
    fn an_unreadable_config_never_counts_as_signed_out_by_choice() {
        // Fail safe: "signed out by choice" removes the domain, so only a config we could read and
        // that says so may produce it.
        let unreadable = facts_from(false, false, false, Err("could not read desktop.toml".to_string()));
        assert_eq!(
            unreadable,
            SessionFacts {
                vault_unlocked: false,
                auth_present: false,
                signed_out_by_choice: false,
                removal_owed: false
            }
        );
        let cfg = DesktopConfig {
            finder_signed_out_by_choice: true,
            ..DesktopConfig::default()
        };
        assert!(facts_from(false, false, false, Ok(cfg)).signed_out_by_choice);
        let readable = facts_from(true, true, false, Ok(DesktopConfig::default()));
        assert_eq!(
            readable,
            SessionFacts {
                vault_unlocked: true,
                auth_present: true,
                signed_out_by_choice: false,
                removal_owed: false
            }
        );
    }

    #[test]
    fn the_config_is_written_only_when_a_value_changes() {
        let mut cfg = DesktopConfig::default();
        let record = FailureRecord {
            reason: FinderFailureReason::FolderTaken,
            domain: "NSCocoaErrorDomain".into(),
            code: 516,
            at: 1_791_291_909,
        };
        assert!(
            !set_failure(&mut cfg, None),
            "no failure saved, none to clear: nothing to write"
        );
        assert!(set_failure(&mut cfg, Some(record.clone())));
        assert_eq!(cfg.finder_last_failure, Some(record.clone()));
        assert!(
            !set_failure(&mut cfg, Some(record.clone())),
            "the same record again writes nothing"
        );
        assert!(set_failure(&mut cfg, None), "Ready clears it");
        assert_eq!(cfg.finder_last_failure, None);

        assert!(!set_signed_out_by_choice(&mut cfg, false));
        assert!(set_signed_out_by_choice(&mut cfg, true));
        assert!(cfg.finder_signed_out_by_choice);
        assert!(!set_signed_out_by_choice(&mut cfg, true));
        assert!(set_signed_out_by_choice(&mut cfg, false));
    }
}
