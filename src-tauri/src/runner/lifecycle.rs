//! `lifecycle` items split out of `runner.rs`.

use super::*;

/// Spec 2026-10-10 §6.2 (task 1887): the sign-out's send phase, as the runner sees it. Armed only while a phase runs;
/// the runner's phase wake (`Wake::Phase`) runs the queue step alone then.
#[derive(Default)]
pub struct SendPhaseLink {
    pub(super) armed: AtomicBool,
    pub(crate) wake: tokio::sync::Notify,
    pub(super) link: std::sync::OnceLock<Arc<crate::link_health::LinkMonitor>>,
    pub(super) signals: Mutex<AttemptSignals>,
}

/// What the phase's attempts met, since it was armed (§6.4: `offline`, `session`, and, by the §15 question 4 ruling,
/// `account`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AttemptSignals {
    pub link_failed: bool,
    pub session_refused: bool,
    pub account_refused: bool,
}

// The phase's side (`arm`, `disarm`, `wake`, `link_failed`, `signals`, and `EngineRunner::send_phase`) is called from
// the sign-out flow, which only a Mac runs (W5); the runner's side (`attach`, `is_armed`, `note`) runs everywhere.
impl SendPhaseLink {
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    pub fn arm(&self) {
        *self.signals.lock().unwrap_or_else(|p| p.into_inner()) = AttemptSignals::default();
        self.armed.store(true, Ordering::SeqCst);
        self.wake.notify_one();
    }
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    pub fn disarm(&self) {
        self.armed.store(false, Ordering::SeqCst);
    }
    pub fn is_armed(&self) -> bool {
        self.armed.load(Ordering::SeqCst)
    }
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    pub fn wake(&self) {
        self.wake.notify_one();
    }
    /// §6.1 condition 3: the link monitor has a failure since its last good answer.
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    pub fn link_failed(&self) -> bool {
        self.link.get().is_some_and(|link| link.snapshot().failure.is_some())
    }
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    pub fn signals(&self) -> AttemptSignals {
        *self.signals.lock().unwrap_or_else(|p| p.into_inner())
    }
    pub(crate) fn attach(&self, link: Arc<crate::link_health::LinkMonitor>) {
        let _ = self.link.set(link);
    }
    pub(crate) fn note(&self, outcome: &crate::engine_bridge::TransferLoopOutcome) {
        let mut signals = self.signals.lock().unwrap_or_else(|p| p.into_inner());
        signals.link_failed |= outcome.link_failed;
        signals.session_refused |= outcome.session_refused;
        signals.account_refused |= outcome.account_refused;
    }
}

/// The phase arm's gate (U35): armed, not paused, and at most once a second. (`runner.rs` imports `Duration` but not
/// `Instant`.)
pub(crate) fn phase_step_due(
    armed: bool,
    mode: RunMode,
    last_run: Option<std::time::Instant>,
    now: std::time::Instant,
) -> bool {
    armed && !mode.is_paused() && last_run.is_none_or(|last| now.duration_since(last) >= Duration::from_secs(1))
}

/// What woke the runner's loop (spec 2026-10-10 §6.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Wake {
    Cancel,
    Tick,
    Phase,
}

/// Spec D11 (task 1888): the account's resume requests, shared with the engine. `asked` counts the sign-ins that asked
/// for the operations paused for `auth` to be due again (`ask_to_resume_auth_paused`); `done` is the ask the last
/// resume served. Off macOS the engine serves them itself, after its reconcile pass and the key check
/// ([`crate::engine_bridge::EngineBridge::reconcile_then_resume`]); a Mac serves them in the engine start.
#[derive(Debug, Clone, Default)]
pub struct ResumeTicket {
    pub asked: Arc<AtomicU64>,
    pub done: Arc<AtomicU64>,
    /// Review P2 (task 1888): the releases that asked for a pass ("Upload to my Beebeeb" on a held change, Windows).
    /// Separate from `asked`, so a release never resumes the operations paused for `auth`.
    pub pass_wanted: Arc<AtomicU64>,
    /// The release ask the last committed pass served: a pass serves what it read before it planned.
    pub pass_served: Arc<AtomicU64>,
}

/// Owned handle to a running engine task. Drop it (or call
/// [`Self::abort`]) to stop the engine.
pub struct EngineRunner {
    pub(super) cancel: Option<oneshot::Sender<()>>,
    pub(super) task: Option<JoinHandle<()>>,
    /// `None` until (and unless) the Unix-socket IPC accept loop fails to
    /// bind (task 1524). Shared with the spawned task so a caller waiting on
    /// Finder/File-Provider readiness — [`wait_for_file_provider_ipc_ready`]
    /// in `lib.rs` — can surface the REAL `std::io::Error` (e.g. the macOS
    /// sandbox refusing the path) instead of just a generic connect timeout.
    pub(super) ipc_bind_error: Arc<Mutex<Option<String>>>,
    /// Cooperative stop flag (task 1538 Codex P1). Shared with the
    /// [`EngineBridge`] `run` builds, so `abort()` can flip it BEFORE even
    /// sending the tick loop's cancel oneshot.
    pub(super) stopping: Arc<AtomicBool>,
    /// The sign-out's send phase (spec 2026-10-10 §6.2), shared with `run`'s loop.
    pub(super) send_phase: Arc<SendPhaseLink>,
}

impl EngineRunner {
    /// Spawn the runner on a background tokio task. Returns the handle
    /// synchronously — actual startup (lock acquire, DB open, first
    /// tick) happens inside the task.
    ///
    /// `sync_paused` is shared with [`crate::AppState`] so the
    /// `tray_pause_sync` / `tray_resume_sync` IPC commands can signal
    /// the loop without restarting the runner. `auth_health` is likewise
    /// shared with the account runtime so `sync_status` can read the
    /// consecutive-401 streak this task feeds (task 1546 finding 5).
    ///
    /// The token and the key arrive as `Zeroizing` values (R10, lead ruling 2): the copy that travelled
    /// from the session to here is wiped when `run` is done with it, and only `ApiClient` (which wipes
    /// its own on drop) keeps a plain one.
    ///
    /// `resume` is the account's resume requests (spec D11): off macOS the engine serves them after its reconcile pass.
    pub fn spawn(
        app: AppHandle,
        sync_root: PathBuf,
        session_token: zeroize::Zeroizing<String>,
        master_key: zeroize::Zeroizing<[u8; 32]>,
        sync_paused: Arc<AtomicBool>,
        auth_health: Arc<AuthHealth>,
        resume: ResumeTicket,
    ) -> Self {
        let (tx, rx) = oneshot::channel::<()>();
        let ipc_bind_error: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let ipc_bind_error_for_task = ipc_bind_error.clone();
        let stopping: Arc<AtomicBool> = Arc::new(AtomicBool::new(false));
        let stopping_for_task = stopping.clone();
        let send_phase = Arc::new(SendPhaseLink::default());
        let send_phase_for_task = send_phase.clone();

        let task = tokio::spawn(async move {
            run(
                app,
                sync_root,
                session_token,
                master_key,
                RunnerControls {
                    cancel: rx,
                    sync_paused,
                    auth_health,
                    ipc_bind_error: ipc_bind_error_for_task,
                    stopping: stopping_for_task,
                    send_phase: send_phase_for_task,
                    resume,
                },
            )
            .await;
        });

        Self {
            cancel: Some(tx),
            task: Some(task),
            ipc_bind_error,
            stopping,
            send_phase,
        }
    }

    /// A cheap clone of the shared IPC-bind-status cell (task 1524). `None`
    /// until the accept loop's bind attempt has run AND failed; still `None`
    /// after a successful bind (there is nothing to report — the readiness
    /// probe's own socket connect is the success signal). Callers poll this
    /// alongside that probe so a real bind error surfaces immediately
    /// instead of only after the full readiness timeout.
    pub fn ipc_bind_error_handle(&self) -> Arc<Mutex<Option<String>>> {
        self.ipc_bind_error.clone()
    }

    /// The sign-out's send phase link (spec 2026-10-10 §6.2): the phase arms it, and this runner's loop runs its queue
    /// step while it is armed.
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    pub fn send_phase(&self) -> Arc<SendPhaseLink> {
        self.send_phase.clone()
    }

    /// Whether the runner's task is still going. A task that ended by itself (startup could not prepare or open the
    /// state dir, or take the lock) leaves this handle in the account's slot, and a slot that is occupied is not a
    /// running engine (PR #130 review). A runner already told to stop is not running either.
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    pub fn is_running(&self) -> bool {
        !self.stopping.load(Ordering::SeqCst) && self.task.as_ref().is_some_and(|task| !task.is_finished())
    }

    /// Test-only constructor: wraps an already-spawned task in a runner
    /// handle so `lib.rs`'s sign-out tests can install a real (stoppable)
    /// engine in an account's engine slot without a Tauri `AppHandle`. The
    /// cancel channel's receiver is dropped immediately, so `abort()`'s
    /// cancel send is a no-op — the wrapped task's own body decides when
    /// (or whether) it stops.
    #[cfg(test)]
    pub(crate) fn for_test_with_task(task: JoinHandle<()>) -> Self {
        let (tx, _rx) = oneshot::channel::<()>();
        Self {
            cancel: Some(tx),
            task: Some(task),
            ipc_bind_error: Arc::new(Mutex::new(None)),
            stopping: Arc::new(AtomicBool::new(false)),
            send_phase: Arc::new(SendPhaseLink::default()),
        }
    }

    /// Signal the runner to stop and wait for CONFIRMED termination. Drops
    /// the lock file as part of teardown. Idempotent — calling twice is a
    /// no-op (the second call has nothing left to wait on and reports
    /// [`AbortOutcome::Stopped`] immediately).
    ///
    /// Returns a per-stage [`AbortOutcome`], never a bare bool: the old
    /// `bool` conflated "the engine task is gone" with "the Windows Cloud
    /// Files callback revocation succeeded", which misattributed revocation
    /// failures (Explorer-held handles, 3s drain timeout) to the sync
    /// engine. Callers that need the task-terminated guarantee —
    /// `clear_session_impl` gating its cross-account purge on it — must
    /// check [`AbortOutcome::task_confirmed`]; callers that need the old
    /// all-stages-success bool use [`AbortOutcome::is_stopped`].
    pub async fn abort(mut self) -> AbortOutcome {
        // Flip the cooperative flag FIRST, before the cancel oneshot even
        // sends: `EngineBridge::is_stopping()` (checked by
        // `process_due_operations` before every operation and by
        // `queue_finder_create`/`_modify`/`_delete` before enqueuing) then
        // observes the stop request immediately, without waiting for the
        // tick loop to next reach its `tokio::select!` boundary.
        self.stopping.store(true, Ordering::SeqCst);
        #[cfg(target_os = "windows")]
        let revoke_before = crate::windows_cf::revoke_callbacks().await;
        let stopped = stop_task_and_confirm(
            self.cancel.take(),
            self.task.take(),
            GRACEFUL_ABORT_TIMEOUT,
            FORCE_ABORT_TIMEOUT,
        )
        .await;
        #[cfg(target_os = "windows")]
        {
            // Do not lose an unconfirmed task behind an empty engine slot and
            // let a second lock attempt claim success. Such a task can still
            // hold startup credentials before an ApiClient exists.
            if !stopped {
                crate::windows_cf::refuse_unconfirmed_stop();
            }
            let revoke_after = crate::windows_cf::revoke_callbacks().await;
            return classify_abort_result(
                stopped,
                revoke_before.as_ref().err().map(|e| e.to_string()),
                revoke_after.as_ref().err().map(|e| e.to_string()),
            );
        }
        #[cfg(not(target_os = "windows"))]
        if stopped {
            AbortOutcome::Stopped
        } else {
            AbortOutcome::TaskUnconfirmed
        }
    }
}

/// What [`EngineRunner::abort`] actually achieved, per stage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AbortOutcome {
    /// The engine task is CONFIRMED terminated AND (Windows) both Cloud
    /// Files callback revocations succeeded. The exact `true` of the old
    /// `abort() -> bool`.
    Stopped,
    /// The engine task itself IS confirmed terminated, but a Windows Cloud
    /// Files callback revocation failed. `stage` names which revocation
    /// attempt failed and `source` carries its error — the sync engine is
    /// NOT the thing that failed here.
    RevokeFailed { stage: &'static str, source: String },
    /// Even a forced abort could not confirm the task terminated — it may
    /// still be running and must be treated as alive.
    TaskUnconfirmed,
}

impl AbortOutcome {
    /// The exact boolean the pre-enum `abort() -> bool` returned, kept so
    /// existing call sites preserve their gating semantics unchanged:
    /// `true` only when the task is confirmed stopped AND (Windows) every
    /// revocation succeeded.
    pub fn is_stopped(&self) -> bool {
        matches!(self, AbortOutcome::Stopped)
    }

    /// `true` when the engine task itself was CONFIRMED terminated — the
    /// guarantee a cross-account purge actually needs. A Cloud Files
    /// revocation failure does not void it; an unconfirmed task does.
    pub fn task_confirmed(&self) -> bool {
        !matches!(self, AbortOutcome::TaskUnconfirmed)
    }
}

/// Per-stage classification of [`EngineRunner::abort`]'s raw results,
/// preserving the old `revoke_before.is_ok() && stopped && revoke_after.is_ok()`
/// semantics while attributing the failure to the stage that actually
/// failed. Pure so it is unit-testable off-Windows (the real revocation
/// calls cannot run in CI).
///
/// Stage precedence: an unconfirmed task dominates everything (the caller
/// must treat the engine as alive); otherwise the FIRST failing revocation
/// stage is reported, matching the old all-must-succeed conjunction.
#[cfg(any(target_os = "windows", test))]
pub(super) fn classify_abort_result(
    task_stopped: bool,
    revoke_before_error: Option<String>,
    revoke_after_error: Option<String>,
) -> AbortOutcome {
    if !task_stopped {
        return AbortOutcome::TaskUnconfirmed;
    }
    if let Some(source) = revoke_before_error {
        return AbortOutcome::RevokeFailed {
            stage: "initial Cloud Files callback revocation",
            source,
        };
    }
    if let Some(source) = revoke_after_error {
        return AbortOutcome::RevokeFailed {
            stage: "final Cloud Files callback revocation",
            source,
        };
    }
    AbortOutcome::Stopped
}

/// How long [`EngineRunner::abort`] waits for the tick loop to reach its
/// cancel-select boundary on its own after the cooperative stop flag + the
/// cancel oneshot are both signaled.
pub(super) const GRACEFUL_ABORT_TIMEOUT: Duration = Duration::from_secs(3);

/// How long, after a forced [`tokio::task::JoinHandle::abort`], we wait for
/// the runtime to confirm the task is actually gone. Aborting a task that is
/// suspended mid-`.await` (the common case — mid network call) drops its
/// future essentially immediately; this is a generous upper bound, not the
/// expected latency.
pub(super) const FORCE_ABORT_TIMEOUT: Duration = Duration::from_secs(2);

/// Signal `cancel` (if any), wait up to `graceful` for `task` to finish on
/// its own, and — only if that times out — force-abort it and wait up to
/// `force` for the runtime to confirm it has actually terminated.
///
/// Returns `true` only when the task is CONFIRMED gone (finished gracefully,
/// or the forced abort was observed to complete within `force`); `false`
/// only in the pathological case where even a forced abort couldn't be
/// confirmed within `force` (e.g. the task is blocked in non-async code with
/// no `.await` point to cancel at).
///
/// Standalone and Tauri-independent on purpose: this is the part of
/// [`EngineRunner::abort`] worth unit-testing directly, without spinning up
/// a real `AppHandle` + sync engine.
pub(super) async fn stop_task_and_confirm(
    cancel: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<()>>,
    graceful: Duration,
    force: Duration,
) -> bool {
    if let Some(tx) = cancel {
        // Ignore send errors — receiver may have already exited.
        let _ = tx.send(());
    }
    let Some(mut handle) = task else {
        // Nothing left to wait on — either already aborted, or the task
        // never finished spawning. Either way there is nothing that could
        // still be running.
        return true;
    };

    if tokio::time::timeout(graceful, &mut handle).await.is_ok() {
        return true;
    }

    // Graceful stop timed out. This is where the pre-fix code just dropped
    // `handle` here, detaching the task instead of cancelling it. Force it
    // down for real, then wait to confirm — not just fire-and-hope.
    tracing::warn!("engine graceful stop timed out; force-aborting the task");
    handle.abort();
    let confirmed = tokio::time::timeout(force, handle).await.is_ok();
    if !confirmed {
        tracing::error!("engine did not confirm termination even after a forced abort; it may still be running");
    }
    confirmed
}

impl Drop for EngineRunner {
    fn drop(&mut self) {
        if let Some(tx) = self.cancel.take() {
            let _ = tx.send(());
        }
        if let Some(handle) = self.task.take() {
            handle.abort();
        }
    }
}

/// Shared control handles [`run`]'s tick loop reads/writes for its whole
/// lifetime, bundled into one struct rather than passed as separate `run`
/// parameters — task 1538 Codex P1: adding `stopping` pushed the previous
/// flat parameter list to 8, past clippy's `too_many_arguments` threshold.
/// `auth_health` (task 1546 finding 5) joined the struct for the same
/// reason rather than reopening that flat list.
pub(super) struct RunnerControls {
    pub(super) cancel: oneshot::Receiver<()>,
    pub(super) sync_paused: Arc<AtomicBool>,
    pub(super) auth_health: Arc<AuthHealth>,
    pub(super) ipc_bind_error: Arc<Mutex<Option<String>>>,
    pub(super) stopping: Arc<AtomicBool>,
    /// The sign-out's send phase (spec 2026-10-10 §6.2): the loop's third wake.
    pub(super) send_phase: Arc<SendPhaseLink>,
    /// Spec D11: the account's resume requests, served after the reconcile pass (Windows and Linux).
    pub(super) resume: ResumeTicket,
}

/// Desktop 0.8.15: every `engine-status` payload carries the start-up guard's progress as
/// `startup_guard`: `{running, done, total}`, counts only.
pub(crate) fn with_startup_guard(mut payload: serde_json::Value) -> serde_json::Value {
    if let Some(object) = payload.as_object_mut() {
        object.insert(
            "startup_guard".to_string(),
            serde_json::json!(crate::unknown_check::STARTUP_GUARD.progress()),
        );
    }
    payload
}

/// Task 2191 (D9), desktop 0.8.15: begin the start-up guard's pass for this engine and walk it in
/// its own task, off the async workers. Nothing waits for it: freeing work asks
/// `STARTUP_GUARD.allows_freeing()`. The pass stops between rows when the engine stops (a lock or
/// a sign-out revokes the callback bridge it holds); one that stopped or panicked leaves freeing
/// held until the next engine start's pass has walked every row.
#[cfg(target_os = "windows")]
pub(super) fn spawn_startup_guard(sync_root: PathBuf) {
    let generation = crate::unknown_check::STARTUP_GUARD.begin();
    let guard_root = sync_root;
    tokio::spawn(async move {
        let pass = tokio::task::spawn_blocking(move || {
            crate::windows_cf::guard_unknown_placeholders(&guard_root, generation)
        });
        if let Err(e) = pass.await {
            crate::unknown_check::STARTUP_GUARD.abandon(generation);
            tracing::warn!(error = %e, "unknown-content guard panicked; freeing stays held until the next start");
        }
    });
}
