//! Long-running engine task: owns the lock file + state DB + API
//! client, runs a periodic sync tick, emits status events to the
//! WebView via Tauri.
//!
//! Phase 1 Task 3 of the desktop sync client plan. Replaces the
//! earlier `beebeeb_sync::SyncEngine` placeholder with a real loop
//! built around [`crate::engine_bridge`] + [`crate::api_client`] +
//! [`crate::state_db`].
//!
//! ## Lifecycle
//!
//! - `EngineRunner::spawn` is called from either:
//!   - `apply_session` (via `start_engine_if_possible`) immediately after
//!     the browser-login handoff installs a fresh session, or
//!   - `pick_sync_root` if a session is already in memory when the
//!     first-launch picker resolves.
//!
//! - The runner drops the prior runner first so a re-login or a
//!   sync-root change cleanly tears down the old task.
//!
//! - On `clear_session` (logout) or app shutdown, [`EngineRunner::abort`]
//!   is called: a oneshot fires, the task exits, the lock file is
//!   released via the `LockFile` `Drop` impl.
//!
//! ## Status events
//!
//! The task emits `engine-status` events. The payload, the seven `state`
//! values (`running` / `idle` / `syncing` / `paused` / `offline` / `error` /
//! `stopped`) and the rules that pick one live in [`crate::engine_status`]. The
//! tray tooltip listener (`attach_tray_status_listener` in `lib.rs`) and the
//! popover snapshot consume this stream.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::api_client::{ApiClient, HeartbeatBody};
use crate::conflict::auto_resolution_deadline;
use crate::engine_bridge::{
    ConflictDetected, EngineBridge, OperationFailureClass, WireCounters, classify_operation_error, sync_tick_outcome,
};
use crate::engine_status::{Activity, StatusTracker, compute_activity, tick_outcome};
use crate::lockfile::LockFile;
use crate::state_db::{FileStatus, StateDb};
use crate::state_paths;








/// API base URL the engine talks to.
///
/// Returns the value of the `BB_API_BASE` environment variable when it is set
/// and non-empty, falling back to the production endpoint.  A trailing slash is
/// stripped so callers that do `format!("{base}/api/v1/...")` never produce a
/// double-slash.  Leading/trailing whitespace is also trimmed.
///
/// # Local dev / testing
///
/// ```bash
/// # Point the desktop at a local API server:
/// BB_API_BASE=http://localhost:3001 bun run tauri:dev
/// # or against a WSL2 IP:
/// BB_API_BASE=http://10.100.0.239:3001 bun run tauri:dev
/// ```
///
/// Packaged release builds ship without this variable set, so they always hit
/// `https://api.beebeeb.io`. A unit-test build never does: see [`TEST_API_BASE`].
pub(crate) fn api_base_url() -> String {
    api_base_url_from(std::env::var("BB_API_BASE").ok().as_deref())
}

/// The production API, the default of every build but a unit-test build.
#[cfg_attr(test, allow(dead_code))]
const PRODUCTION_API_BASE: &str = "https://api.beebeeb.io";

/// A unit-test build has no production default: with `BB_API_BASE` unset it talks to a closed
/// loopback port, so no lib test can reach the production API (a test that needs a server starts
/// its own and sets `BB_API_BASE`). After a CLI test once ran against the developer's live session.
#[cfg(test)]
pub(crate) const TEST_API_BASE: &str = "http://127.0.0.1:9";

/// [`api_base_url`] for a given `BB_API_BASE` value.
fn api_base_url_from(env: Option<&str>) -> String {
    let trimmed = env.unwrap_or_default().trim();
    if trimmed.is_empty() {
        #[cfg(test)]
        return TEST_API_BASE.to_string();
        #[cfg(not(test))]
        return PRODUCTION_API_BASE.to_string();
    }
    trimmed.trim_end_matches('/').to_string()
}







// ── Heartbeat telemetry ───────────────────────────────────────────────────────
//
// The runner loop publishes its current high-level engine state into a shared
// `TelemetryState`; the heartbeat producer task reads it each beat, snapshots
// the state DB for live counts/bytes, and POSTs a heartbeat. Decoupling the two
// (loop writes, producer reads) keeps the heartbeat cadence independent of the
// 5s sync tick — the Bandwidth view gets a steady beat even while a tick is
// mid-flight.




















/// The loop's one wait, in the order `run`'s `select!` always had (`biased`: the cancel first, then the tick), plus the
/// send phase's wake. The phase's arm has no precondition. `select!` evaluates an `if` precondition once, when it
/// starts waiting, so an arm disabled then cannot be woken by `notify_one` until another arm fires: a phase armed while
/// the runner waits would get no queue step until the next tick, up to 30 s later. The `Notify` fires only when
/// `arm()` or the phase's driver wakes it (a wake with no phase running finds `phase_step_due` false), and `notify_one`
/// stores a permit when nobody waits, so a wake sent mid-tick is not lost.
async fn next_wake(
    cancel: &mut oneshot::Receiver<()>,
    tick: &mut tokio::time::Interval,
    send_phase: &SendPhaseLink,
) -> Wake {
    tokio::select! {
        biased;
        _ = cancel => Wake::Cancel,
        _ = tick.tick() => Wake::Tick,
        _ = send_phase.wake.notified() => Wake::Phase,
    }
}













/// The runner task body. Acquires the lock, opens the state DB,
/// builds the API client + engine bridge, ticks every
/// [`TICK_INTERVAL`] running [`sync_tick`], exits when the cancel
/// channel fires.
async fn run(
    app: AppHandle,
    sync_root: PathBuf,
    session_token: zeroize::Zeroizing<String>,
    master_key: zeroize::Zeroizing<[u8; 32]>,
    controls: RunnerControls,
) {
    // Task 1538 Codex P1: destructured immediately so the rest of this
    // (already long-standing) function body is untouched — every field
    // below is used exactly as the old flat `cancel`/`sync_paused`/
    // `auth_health`/`ipc_bind_error`/`stopping` parameters were.
    let RunnerControls {
        mut cancel,
        sync_paused,
        auth_health,
        ipc_bind_error,
        stopping,
        send_phase,
        resume,
    } = controls;

    // Only consumed inside the `#[cfg(unix)]` IPC block below — Windows has
    // no Unix-socket daemon endpoint (see that block's own doc comment), so
    // the parameter would otherwise go unused on a Windows build.
    #[cfg(not(unix))]
    let _ = &ipc_bind_error;

    #[cfg(target_os = "windows")]
    if let Err(error) = crate::windows_cf::wait_for_credential_release().await {
        emit_status(&app, "error", Some(&sync_root), Some(&error));
        return;
    }
    // Windows Cloud Files: connect the sync root before any placeholder work.
    // Beebeeb metadata now lives in the app-local state dir, but Cloud Files
    // placeholder seeding still needs a connected root later in this task.
    #[cfg(target_os = "windows")]
    crate::windows_cf::connect_root(&sync_root);

    if let Err(e) = state_paths::prepare_beebeeb_state_dir_for_sync_root(&sync_root) {
        let msg = format!("prepare app-local state dir: {e}");
        tracing::error!(error = %msg);
        emit_status(&app, "error", Some(&sync_root), Some(&msg));
        return;
    }

    // Acquire the mutual-exclusion lock in app-local state, outside the user's
    // synced files.
    let _lock = match LockFile::acquire("desktop") {
        Ok(l) => l,
        Err(msg) => {
            tracing::error!(error = %msg, "could not acquire .beebeeb-sync.lock");
            emit_status(&app, "error", Some(&sync_root), Some(&msg));
            return;
        }
    };

    // Open the state DB from app-local data. The migration step above already
    // created the directory and moved any legacy sync-root metadata out.
    let state_dir = match state_paths::beebeeb_state_dir() {
        Ok(path) => path,
        Err(e) => {
            let msg = format!("resolve state dir: {e}");
            tracing::error!(error = %msg);
            emit_status(&app, "error", Some(&sync_root), Some(&msg));
            return;
        }
    };
    let db = match StateDb::open(state_dir.join(state_paths::STATE_DB_FILENAME)) {
        Ok(d) => Arc::new(d),
        Err(e) => {
            let msg = format!("open state.db: {e}");
            tracing::error!(error = %msg);
            emit_status(&app, "error", Some(&sync_root), Some(&msg));
            return;
        }
    };
    // No runner survives a restart: clear every queue claim. On macOS, also remove
    // the staged copies the queue released and nothing references any more
    // (spec §8.7 S3, S6).
    match db.engine_start_repair() {
        Ok(repair) => {
            #[cfg(target_os = "macos")]
            crate::staged_payload::remove_released(&db, &repair.released_payloads);
            if repair.claims_cleared > 0 || !repair.released_payloads.is_empty() {
                tracing::info!(
                    claims_cleared = repair.claims_cleared,
                    released = repair.released_payloads.len(),
                    "engine start: queue claims cleared, released upload copies removed"
                );
            }
        }
        // A category only: the error's text can carry a path (spec §11).
        Err(e) => tracing::warn!(
            reason = crate::engine_bridge::error_category(&e),
            "engine start repair failed"
        ),
    }
    // Then the staged plaintext copies nothing will upload: an accept that never committed, a
    // copy whose row an older build's sign-out deleted (spec §8.7 S6). Here, after the claims
    // are cleared and before the IPC server below serves a write. A stopping engine's IPC handler
    // can still be inside an accept (its connection tasks are aborted, not awaited); the accept's
    // own transaction requires its copy's journal row, which the sweep deletes before it unlinks
    // the copy, so such an accept either committed first or queues nothing and is retried.
    #[cfg(target_os = "macos")]
    match crate::staged_payload::sweep_unreferenced(&db, &crate::engine_bridge::finder_staging_candidates()) {
        Ok(sweep) => {
            if sweep.unfinished > 0 || sweep.unjournalled > 0 {
                tracing::info!(
                    unfinished = sweep.unfinished,
                    unjournalled = sweep.unjournalled,
                    "engine start: staged copies no queue row names removed"
                );
            }
            if let Some(reason) = sweep.failure {
                tracing::warn!(
                    kept = sweep.kept,
                    reason,
                    "engine start: staged copies no queue row names kept; removal is retried at the next start"
                );
            }
        }
        Err(e) => tracing::warn!(
            reason = crate::engine_bridge::error_category(&e),
            "engine start: removing staged copies no queue row names failed"
        ),
    }
    // Uploads an earlier build queued get a write id and are marked as from an earlier build,
    // so they are never handed over; their bases are left as they are (spec §10.2–§10.3). Before
    // the reconcile below, which keeps a row with an unparked Finder upload (one with a write id)
    // uploading. macOS only: on Windows it would hand write ids to Cloud Files uploads, which have
    // no base.
    #[cfg(target_os = "macos")]
    match db.mark_earlier_build_uploads() {
        Ok(0) => {}
        Ok(marked) => tracing::info!(
            marked,
            "engine start: uploads from an earlier build marked; their bases are unchanged"
        ),
        Err(e) => tracing::warn!(
            reason = crate::engine_bridge::error_category(&e),
            "engine start: marking earlier-build uploads failed"
        ),
    }
    // The count is the rows reset; a row whose Finder upload resumes stays `Uploading` and is
    // not counted (spec §9.2). Each reset changes how its item is presented, so the first tick's
    // working-set signal carries the reset ids (below, as the signal's opening debt).
    let reset_item_ids = match db.reconcile_stale_in_flight_on_startup() {
        Ok(reset_item_ids) => {
            let reset = reset_item_ids.len();
            if reset > 0 {
                tracing::info!(
                    reset,
                    "engine start: stale in-flight rows from a previous engine session reset"
                );
            }
            reset_item_ids
        }
        Err(e) => {
            tracing::warn!(
                reason = crate::engine_bridge::error_category(&e),
                "could not reconcile stale in-flight state rows on startup"
            );
            Vec::new()
        }
    };

    // The token's buffer MOVES into the client (nothing is copied); the key is copied into it once. Both
    // wrappers are dropped right here, so the wiped originals do not live as long as this task.
    let mut session_token = session_token;
    let api = Arc::new(ApiClient::new(
        api_base_url(),
        std::mem::take(&mut *session_token),
        *master_key,
    ));
    drop(session_token);
    drop(master_key);
    #[cfg(target_os = "windows")]
    crate::windows_cf::track_credentials(&api);
    // Shares `stopping` with `EngineRunner::abort` (task 1538 Codex P1) so
    // this bridge — and every clone of it handed to the IPC socket server
    // (below) and the Windows upload watcher — observes a stop request the
    // instant `abort()` sets it, not just at this loop's next tick boundary.
    // Task 1683 slice 2: the board the popover snapshot reads (managed app state),
    // or a private one when this runner has no app state (tests).
    let transfers = app
        .try_state::<crate::popover_data::PopoverRuntime>()
        .map(|runtime| runtime.transfers.clone())
        .unwrap_or_else(crate::transfer_progress::TransferBoard::new);
    // Spec D11 (review P2): the bridge's queue reads the account's pass requests, so it sends nothing while a release
    // waits for a pass.
    let bridge = Arc::new(
        EngineBridge::new_with_stop_flag(db.clone(), api.clone(), stopping)
            .with_transfers(transfers.clone())
            .with_resume(resume.clone()),
    );
    // What each request saw of the link (offline vs server did not answer).
    let link = api.link();
    send_phase.attach(link.clone());
    let tracker = Arc::new(StatusTracker::new(Some(sync_root.to_string_lossy().into_owned())));
    // `syncing` needs a pulse: the tick awaits every due upload to completion, so
    // the post-tick counts are almost always zero (see `engine_status`).
    let _status_pulse = AbortWorkerOnDrop(
        tokio::spawn(run_status_pulse(
            app.clone(),
            tracker.clone(),
            db.clone(),
            transfers.clone(),
            sync_paused.clone(),
        ))
        .abort_handle(),
    );

    // ── Heartbeat telemetry (the WRITE/PRODUCE side of the Bandwidth view) ──
    //
    // Register THIS device + open ONE sync session, then spawn a producer task
    // that posts live telemetry on its own cadence. All best-effort: a failed
    // registration logs and leaves `heartbeat` `None`, so the sync engine runs
    // exactly as before, just without telemetry. The producer's cancel handle +
    // its `Arc<ApiClient>`/`Arc<StateDb>` clones live for the loop's lifetime;
    // teardown below (after the loop breaks) fires the cancel so the producer
    // stops — and drops its session token — on logout/lock/shutdown alongside
    // the rest of the engine.
    let telemetry = Arc::new(Mutex::new(TelemetryState {
        engine_state: "running".to_string(),
        current_file: None,
    }));
    // `session_id` is cloned into the producer task AND kept here so the
    // teardown below can post a final "stopped" beat under the same session.
    let mut heartbeat: Option<(oneshot::Sender<()>, JoinHandle<()>, String)> =
        match register_session(&api, &sync_root).await {
            Some((session_id, interval)) => {
                let (hb_cancel_tx, hb_cancel_rx) = oneshot::channel::<()>();
                let handle = spawn_heartbeat_producer(
                    api.clone(),
                    db.clone(),
                    session_id.clone(),
                    interval,
                    telemetry.clone(),
                    sync_paused.clone(),
                    bridge.wire.clone(),
                    auth_health.clone(),
                    hb_cancel_rx,
                );
                Some((hb_cancel_tx, handle, session_id))
            }
            None => None,
        };

    // Windows forced runner abort must also abort the separately spawned
    // heartbeat future; dropping a JoinHandle alone would detach its API owner.
    #[cfg(target_os = "windows")]
    let _heartbeat_abort = heartbeat
        .as_ref()
        .map(|(_, task, _)| AbortWorkerOnDrop(task.abort_handle()));

    // Task 1670 round 2: purge the macOS hydrate-cache staging dir on every
    // (re)start of this runner — this IS "daemon startup" in this
    // single-process architecture (there is no separate boot phase distinct
    // from the vault-unlock that calls `run`). Bounds any staged plaintext
    // that survived a crash between a previous session's write and its
    // per-request cleanup, or a force-quit mid-fetch, to at most one restart.
    // Runs BEFORE the IPC server below starts accepting HydrateFile requests
    // so a fresh session never inherits a stale entry. `purge_macos_hydrate_cache`
    // is a no-op on non-macOS (called unconditionally, like its other two call
    // sites in `lib.rs`, so this file doesn't need its own cfg gate).
    crate::purge_macos_hydrate_cache("daemon-startup");
    // Write contents the File Provider extension hands over arrive in the App
    // Group upload-staging directory. Harden it and purge copies orphaned by a
    // crash. Age-bound, not "everything": the extension may be staging a copy
    // right now (see `ipc_socket::UPLOAD_STAGING_MAX_AGE`).
    #[cfg(target_os = "macos")]
    crate::ipc_socket::macos_prepare_upload_staging();

    // Spawn the Unix-socket IPC server alongside the sync loop. It
    // shares the same StateDb + EngineBridge handles, so OS extensions
    // (macOS File Provider, Linux FUSE) can query status and trigger
    // hydrations without waiting for the next tick.
    //
    // Windows has no separate extension process: the desktop binary is
    // itself the Cloud Files provider and Windows calls back into this
    // runtime in-process (see `crate::windows_cf`), so there is no
    // Unix-socket server on Windows.
    //
    // Keep an explicit cancel handle so lock/logout tears down the listener
    // that holds a cloned EngineBridge with the active master key.
    #[cfg(unix)]
    let (ipc_cancel_tx, ipc_cancel_rx) = oneshot::channel::<()>();
    #[cfg(unix)]
    let mut ipc_task = {
        let db_for_ipc = db.clone();
        let bridge_for_ipc = bridge.clone();
        let ipc_bind_error = ipc_bind_error.clone();
        tokio::spawn(async move {
            // A bind failure (task 1524 — e.g. the macOS sandbox refusing a
            // socket path outside the app's container) no longer panics this
            // task silently: `serve_ipc` returns the real `std::io::Error`,
            // which is logged here AND recorded for
            // `wait_for_file_provider_ipc_ready` (lib.rs) to report verbatim
            // to the "Install Finder location" caller instead of a generic
            // timeout.
            if let Err(e) = crate::ipc_socket::serve_ipc(db_for_ipc, bridge_for_ipc, ipc_cancel_rx).await {
                let msg = format!("{e}");
                tracing::error!(
                    error = %msg,
                    path = %crate::ipc_socket::ipc_socket_path().display(),
                    "IPC socket failed to bind; Finder/File Provider cannot reach the sync daemon this session"
                );
                if let Ok(mut guard) = ipc_bind_error.lock() {
                    *guard = Some(msg);
                }
            }
        })
    };

    // Windows Cloud Files: stash the live bridge so the in-process fetch
    // callback can reach it, and seed Explorer with cloud-only placeholders
    // from the current file list. The sync root was already registered +
    // connected by `connect_root` at the top of `run` (before the lock/state.db
    // writes), so by here the root is live and the placeholder writes succeed.
    // All idempotent — safe to run on every (re)spawn.
    #[cfg(target_os = "windows")]
    crate::windows_cf::seed_placeholders(bridge.clone(), &sync_root);
    // Task 2191 (D9), revised in desktop 0.8.15: every file of unknown content that holds bytes
    // and reads in sync is marked not in sync. The pass runs BESIDE the engine, not before it:
    // 0.8.14 awaited it here, and on a PC with 37,169 such files nothing uploaded for over two
    // hours. Uploads, downloads, deletes and renames start at once; the tick's freeing work waits
    // for `STARTUP_GUARD` (spec U4). Nothing between here and the tick loop is awaited.
    #[cfg(target_os = "windows")]
    spawn_startup_guard(sync_root.clone());

    // Task 0780 — the UPLOAD trigger. On Windows the desktop binary IS the
    // Cloud Files provider and there is no extension/IPC socket to fire
    // `QueueFinderCreate` when the user drops a file in the sync root, so a
    // local create previously never uploaded. The CF NOTIFY callbacks fire only
    // for I/O on EXISTING placeholders, NOT for a brand-new plain file a foreign
    // process drops in — so the create trigger is a periodic enumeration scan
    // that reads the disk directly (which always works). `watcher::spawn` starts
    // BOTH the CF NOTIFY dispatch loop (modify of existing placeholders +
    // delete/rename) AND the enumeration scan loop (the create trigger); both
    // funnel new files through the shared `classify_local_path` gate that filters
    // out engine-written placeholders, hydration writes, and legacy `.beebeeb/`
    // internals, then into the existing encrypted upload path. macOS/Linux get
    // creates via their OS extension over `ipc_socket`, so the watcher is
    // Windows-only here.
    //
    // The handle is held for the lifetime of the loop; dropping it on
    // logout/shutdown stops BOTH the debounce task and the scan task, so the
    // cloned EngineBridge (holding the session master key) is released with the
    // rest of the engine.
    #[cfg(target_os = "windows")]
    let _upload_watcher = crate::watcher::spawn(bridge.clone(), sync_root.clone());

    emit_payload(&app, tracker.running().to_json());

    let mut tick = tokio::time::interval(TICK_INTERVAL);
    let mut sync_complete_notifications = SyncCompleteNotificationState::default();
    let mut quota_warning_notifications = QuotaWarningNotificationState::default();
    // Working-set signals a busy bridge gate dropped (Task 9 fix round 1, item B): owed to the next tick.
    // It opens with the rows the startup reconcile reset, which no signal has carried yet.
    let mut owed_working_set_signal = OwedWorkingSetSignal::owing(reset_item_ids);
    let mut last_search_index_signature: Option<u64> = None;

    // Known-folder backup (task 0797, Windows): run the source→vault mirror on a
    // SLOW cadence relative to the 30s sync tick — every `KNOWN_FOLDER_MIRROR_EVERY_N_TICKS`
    // ticks. The mirror only copies NEW/CHANGED files (mtime+size diff), so an
    // idle pass is cheap, but it still reads the whole known-folder tree, so we
    // don't want it on every tick. `0` runs the first mirror on the first tick.
    // Windows-only: on other platforms there are no Windows known folders to
    // mirror, so the counter would be unused.
    #[cfg(target_os = "windows")]
    let mut tick_count: u64 = 0;

    // Task 1670 round 3: separate counter (not `tick_count` above, which is
    // Windows-only and used for a different-purpose slow cadence) for the
    // macOS hydrate-cache TTL sweep — see `MACOS_HYDRATE_SWEEP_EVERY_N_TICKS`
    // for why this exists.
    #[cfg(target_os = "macos")]
    let mut hydrate_sweep_tick_count: u64 = 0;
    #[cfg(target_os = "macos")]
    let mut upload_staging_sweep_tick_count: u64 = 1;
    // Spec §7.1: aliases older than 30 days go once a day (engine start sweeps them too).
    let mut last_alias_sweep = std::time::Instant::now();
    // Spec 2026-10-10 §6.2: when the send phase's wake last ran the queue step.
    let mut last_phase_run: Option<std::time::Instant> = None;
    // Spec D11, D11b (task 1888): whether this engine run's first reconcile pass, with its key check and resume, has
    // committed. Until it has, the send phase's wake sends nothing.
    #[cfg(not(target_os = "macos"))]
    let mut reconciled_this_run = false;

    loop {
        match next_wake(&mut cancel, &mut tick, &send_phase).await {
            Wake::Cancel => break,
            Wake::Tick => {
                // Task 1670 round 3: run BEFORE the `sync_paused` check below
                // — bounding staged plaintext lifetime is a security property
                // independent of whether the user paused file sync, and
                // Finder can still hydrate files (via the always-running IPC
                // server) while sync is paused.
                #[cfg(target_os = "macos")]
                {
                    if hydrate_sweep_tick_count.is_multiple_of(MACOS_HYDRATE_SWEEP_EVERY_N_TICKS) {
                        let dir = crate::ipc_socket::macos_hydrate_cache_dir();
                        match crate::ipc_socket::macos_sweep_stale_hydrate_cache_entries(
                            &dir,
                            crate::ipc_socket::MACOS_HYDRATE_CACHE_TTL,
                            std::time::SystemTime::now(),
                        ) {
                            Ok(removed) if removed > 0 => {
                                tracing::debug!(
                                    removed,
                                    dir = %dir.display(),
                                    "periodic macOS hydrate-cache TTL sweep"
                                );
                            }
                            Ok(_) => {}
                            Err(e) => {
                                tracing::warn!(
                                    error = %e,
                                    dir = %dir.display(),
                                    "periodic macOS hydrate-cache TTL sweep failed (best-effort)"
                                );
                            }
                        }
                    }
                    hydrate_sweep_tick_count = hydrate_sweep_tick_count.wrapping_add(1);
                }
                // Orphaned upload-staging copies: same age bound as at
                // startup, so a copy left by a crash of either process is gone
                // within about an hour, not at the next unlock.
                #[cfg(target_os = "macos")]
                {
                    if upload_staging_sweep_tick_count.is_multiple_of(MACOS_UPLOAD_STAGING_SWEEP_EVERY_N_TICKS) {
                        crate::ipc_socket::macos_sweep_upload_staging();
                    }
                    upload_staging_sweep_tick_count = upload_staging_sweep_tick_count.wrapping_add(1);
                }
                if last_alias_sweep.elapsed() >= std::time::Duration::from_secs(86_400) {
                    // A fixed category, never the error text (spec §11).
                    if db
                        .sweep_aliases(now_secs(), crate::state_db::ALIAS_MAX_AGE_SECS)
                        .is_err()
                    {
                        tracing::warn!(reason = "database", "daily alias sweep failed");
                    }
                    last_alias_sweep = std::time::Instant::now();
                }

                // Task 2191 (D2, review-157 F2): the "not in sync" marks still to be made are local
                // work, so they run on every tick: paused, offline or not.
                #[cfg(target_os = "windows")]
                {
                    let mark_root = sync_root.clone();
                    if let Err(e) = tokio::task::spawn_blocking(move || crate::windows_cf::retry_pending_marks(&mark_root)).await {
                        tracing::warn!(error = %e, "pending-mark retry panicked; continuing");
                    }
                }

                // Skip all sync work while the user has paused sync.
                // The loop keeps running so it can receive the cancel
                // signal and so it wakes up promptly when resumed.
                let mode = RunMode::of_flag(sync_paused.load(Ordering::Relaxed));
                if mode.is_paused() {
                    emit_payload(&app, tracker.paused().to_json());
                    set_telemetry_state(&telemetry, "paused");
                    // Windows breadcrumb flyout: reflect the paused state.
                    #[cfg(target_os = "windows")]
                    refresh_status_ui_snapshot(&db, mode, false);
                    continue;
                }

                // Native completion is local work: retry even if the API is offline.
                #[cfg(target_os = "windows")]
                crate::windows_cf::upload_finalization::retry(&db, &sync_root);

                // Known-folder backup mirror (task 0797). Run BEFORE the sync
                // tick on the slow cadence so any files it copies into the sync
                // root are present when the enumeration scan next walks the tree.
                // Windows-only (SHGetKnownFolderPath); a no-op when the user has
                // opted into nothing.
                #[cfg(target_os = "windows")]
                {
                    if tick_count % KNOWN_FOLDER_MIRROR_EVERY_N_TICKS == 0 {
                        run_known_folder_mirror(&sync_root).await;
                    }
                    tick_count = tick_count.wrapping_add(1);
                }

                let syncing_before_tick = sync_in_flight_count(&db);
                let tick_started_at = now_secs() as i64;

                match bridge.refresh_shared_roots().await {
                    Ok(outcome) if !outcome.removed_shared_file_ids.is_empty() => {
                        tracing::info!(
                            removed = outcome.removed_shared_file_ids.len(),
                            "revoked shared content removed from local Finder state"
                        );
                        signal_working_set_owing(
                            &mut owed_working_set_signal,
                            &db,
                            "shared_roots_changed",
                            &outcome.removed_shared_file_ids,
                        );
                    }
                    Ok(_) => {}
                    Err(e) => {
                        tracing::warn!(error = %e, "shared root refresh failed");
                    }
                }
                match sync_tick_outcome(&*bridge, &sync_root).await {
                    Ok(tick) => {
                        // A successful tick is a real, authenticated API round
                        // trip — clears the auth-failure streak (task 1546
                        // finding 5) alongside every other post-tick bookkeeping
                        // step below.
                        auth_health.note_result(None);
                        // Task 10 — surface freshly detected conflicts.
                        // The engine bridge already flipped status to
                        // Conflict; we own the UI side: open a window
                        // per file, fire a notification, emit a Tauri
                        // event so the settings page can refresh its
                        // counts immediately.
                        for c in &tick.conflicts {
                            handle_new_conflict(&app, c);
                        }
                        // Task 13 — sweep for conflicts past their 24h
                        // deadline and apply Keep Both. Done after the
                        // detection step so a freshly-detected conflict
                        // (timestamp ≈ now) doesn't get auto-resolved
                        // on the very same tick.
                        sweep_auto_resolutions(&app, &bridge, &sync_root).await;
                        // Spec D11, D12, I6 (task 1888): the reconcile pass, then the key check and the resume of what
                        // a refused session paused, before this tick's queue step sends anything. A pass that fails
                        // resumes nothing and is tried again next tick; the queue step still runs (the parent-first
                        // gate keeps a ghost's children waiting), as before this pass existed. Only at a tick whose
                        // pull came back (review P1): `Ok` alone is also a swallowed `/sync/ops` failure (offline, a
                        // 5xx, a 401), which proves no token and leaves the rows the pass plans against stale.
                        #[cfg(not(target_os = "macos"))]
                        if tick.reached_server
                            && let Err(error) = bridge
                                .reconcile_then_resume(&sync_root, &resume, &mut reconciled_this_run)
                                .await
                        {
                            tracing::warn!(
                                %error,
                                "the reconcile pass did not complete; held changes stay paused, next tick retries"
                            );
                        }
                        let completed_sync_work = queue_step(
                            &bridge,
                            &sync_root,
                            &db,
                            &mut owed_working_set_signal,
                            &tick.applied_item_ids,
                            &send_phase,
                        )
                        .await;
                        enforce_cache_budget(&app, &bridge, &mut quota_warning_notifications);

                        // Windows Cloud Files: the one-shot `seed_placeholders`
                        // at engine spawn ran against a (usually empty) DB, so
                        // rows discovered by this and later ticks need a
                        // placeholder minted now. Idempotent — existing
                        // placeholders are a no-op (ERROR_ALREADY_EXISTS → Ok).
                        #[cfg(target_os = "windows")]
                        crate::windows_cf::refresh_placeholders(&sync_root);

                        // Reconcile native Explorer pin / "Free up space" changes
                        // (which never touch our DB) back into state_db so the app
                        // view matches and `free_up_space` respects native pins.
                        // Delta-only + idempotent — no-op when nothing changed
                        // natively. Runs after refresh so newly minted placeholders
                        // are present before we read their attributes.
                        #[cfg(target_os = "windows")]
                        crate::windows_cf::reconcile_placeholder_state(&sync_root);

                        // Stamp every DB-Local file's placeholder in-sync so
                        // Explorer shows the green ✓ overlay. Runs after
                        // reconcile (which may have just flipped rows to Local)
                        // and after refresh (so newly-minted placeholders exist
                        // on disk). Idempotent — CfSetInSyncState on an already-
                        // in-sync placeholder is a Windows no-op. Bounded at
                        // MAX_IN_SYNC_STAMPS_PER_PASS files per tick.
                        // Task 1640 (review P2-5): the pass may hash a file; off the async workers.
                        // Desktop 0.8.15: freeing work, so it waits until the start-up guard's pass
                        // has walked every file of unknown content (spec U4).
                        #[cfg(target_os = "windows")]
                        if crate::unknown_check::STARTUP_GUARD.allows_freeing() {
                            let stamp_root = sync_root.clone();
                            if let Err(e) = tokio::task::spawn_blocking(move || {
                                crate::windows_cf::stamp_local_files_in_sync(&stamp_root);
                                // Task 1640 (review N13): kept copies whose original was in use.
                                crate::windows_cf::retry_kept_originals(&stamp_root);
                            })
                            .await
                            {
                                tracing::warn!(error = %e, "stamp pass panicked; continuing");
                            }
                        }

                        let syncing_after_tick = sync_in_flight_count(&db);
                        let unresolved_after_tick = unresolved_sync_issue_count(&db);
                        let should_notify_sync_complete = sync_complete_notifications.observe(
                            syncing_before_tick > 0 || completed_sync_work > 0,
                            syncing_after_tick,
                            unresolved_after_tick,
                        );

                        // `Ok` is not the whole story: `sync_tick` swallows a failed
                        // `GET /sync/ops`, so the link monitor decides whether this
                        // was really a check that reached the server.
                        emit_payload(
                            &app,
                            finish_tick_payload(&tracker, Ok(()), &link, tick_started_at, &db, &transfers),
                        );
                        // Feed the heartbeat producer the post-tick state. It
                        // promotes this to "syncing" itself when the DB snapshot
                        // shows files in flight, so a steady-state tick reads as
                        // "watching" while an active transfer reads as "syncing".
                        set_telemetry_state(&telemetry, "idle");
                        // Windows breadcrumb flyout: in-flight counts ⇒ Syncing,
                        // conflicts ⇒ Error, else In-Sync. Not paused, no engine
                        // error on a successful tick.
                        #[cfg(target_os = "windows")]
                        refresh_status_ui_snapshot(&db, RunMode::Running, false);

                        match crate::desktop_search::local_index_signature(&db) {
                            Ok(signature) if last_search_index_signature != Some(signature) => {
                                match crate::desktop_search::sync_local_index_to_remote(&db, &api).await {
                                    Ok(summary) => {
                                        last_search_index_signature = Some(signature);
                                        tracing::debug!(
                                            indexed_file_count = summary.indexed_file_count,
                                            put_count = summary.put_count,
                                            delete_count = summary.delete_count,
                                            "desktop search index shards synced"
                                        );
                                    }
                                    Err(error) => {
                                        tracing::warn!(
                                            error = %error,
                                            "desktop search index shard sync failed; will retry after the next local mirror change"
                                        );
                                    }
                                }
                            }
                            Ok(_) => {}
                            Err(error) => {
                                tracing::warn!(error = %error, "desktop search index signature failed");
                            }
                        }

                        if should_notify_sync_complete && let Err(e) = crate::notify_sync_complete_impl(&app) {
                            tracing::warn!(error = %e, "notify_sync_complete failed");
                        }
                    }
                    Err(e) => {
                        // Network blips and 401s during token rotation
                        // are normal — log and continue, surface the
                        // error to the WebView so the tray reflects it.
                        tracing::warn!(error = %e, "sync tick failed");
                        // Task 1546 finding 5: a run of these that classify as
                        // auth failures (not network blips) is what flips
                        // `auth_health` and surfaces the persistent banner.
                        auth_health.note_result(Some(&e));
                        emit_payload(
                            &app,
                            finish_tick_payload(&tracker, Err(&e), &link, tick_started_at, &db, &transfers),
                        );
                        set_telemetry_state(&telemetry, "error");
                        // A failed tick reaches none of the signal sites after the tick. Only the
                        // shared-roots signal runs earlier in the same iteration, and a delivered
                        // signal pays the debt, so at most one signal is delivered per tick. A
                        // pending debt (the startup resets, or a signal a busy gate dropped) is
                        // paid here.
                        signal_owed_debt_after_a_failed_tick(&mut owed_working_set_signal, &db);
                        // Windows breadcrumb flyout: surface the tick failure as
                        // an Error state in the flyout too.
                        #[cfg(target_os = "windows")]
                        refresh_status_ui_snapshot(&db, RunMode::Running, true);
                    }
                }
            }
            // Spec 2026-10-10 §6.2: while a sign-out's send phase is armed, the tick's queue step alone, at most once a
            // second. Never the remote tick, the sweeps or the shared-root refresh. The queue step pays its own
            // working-set signal; there are no remote applies to merge here.
            Wake::Phase => {
                let now = std::time::Instant::now();
                if !phase_step_due(
                    send_phase.is_armed(),
                    RunMode::of_flag(sync_paused.load(Ordering::Relaxed)),
                    last_phase_run,
                    now,
                ) {
                    continue;
                }
                // Spec D11b (task 1888): nothing is sent before this run's first pass committed; the next tick runs it.
                #[cfg(not(target_os = "macos"))]
                if !reconciled_this_run {
                    continue;
                }
                last_phase_run = Some(now);
                queue_step(&bridge, &sync_root, &db, &mut owed_working_set_signal, &[], &send_phase).await;
            }
        }
    }

    emit_payload(&app, tracker.stopped().to_json());

    // Stop the heartbeat producer, then post ONE final best-effort "stopped"
    // beat. Ordering: cancel + await the producer FIRST so it can't race a
    // normal beat against this final one, then send the definitive "stopped".
    //
    // Why this works at teardown: logout (`clear_session`) and lock
    // (`lock_vault`) both call `EngineRunner::abort().await` — which fires the
    // cancel that breaks the loop and runs THIS code — BEFORE they clear the
    // in-memory session / keychain, and neither calls a server-side session-
    // revocation endpoint. So `api`'s bearer token is still valid here; the POST
    // lands.
    //
    // Time budget: `EngineRunner::abort` only waits 3s for this whole `run()`
    // task, so the two phases below are bounded to 1s (producer stop, normally
    // instant — a biased cancel) + 1.5s (final beat) = ≤2.5s, comfortably under
    // that 3s window so the beat is never cut off mid-flight by `abort` moving
    // on to clear the keychain.
    //
    // What "stopped" buys the read side: the server stores the heartbeat status
    // verbatim, so the latest heartbeat row reads `stopped`. The current web
    // Bandwidth view (`devices.tsx`) does not yet treat a `stopped`
    // HEARTBEAT_status as instant-offline — it flips offline via the wall-clock
    // staleness TTL (no heartbeat for ~interval×15) — so this primarily PREVENTS
    // a misleading final "watching/syncing" beat from lingering and records a
    // truthful terminal state a frontend follow-up can honor for an immediate
    // flip. (We deliberately do NOT send `error`, which WOULD flip offline now
    // but would mislabel a clean logout as a failure.)
    if let Some((hb_cancel_tx, hb_handle, session_id)) = heartbeat.take() {
        let _ = hb_cancel_tx.send(());
        if tokio::time::timeout(Duration::from_secs(1), hb_handle).await.is_err() {
            tracing::debug!("heartbeat producer did not stop within 1s; abandoning");
        }

        let final_beat = HeartbeatBody {
            status: "stopped".to_string(),
            // Report the last known counts so the row's progress numbers don't
            // reset to null on the terminal beat; zero speed, no current file.
            files_synced: db.count_by_status(FileStatus::Local).ok().map(i64::from),
            current_file: None,
            speed_bps: Some(0),
            ..Default::default()
        };
        // Best-effort + hard-bounded: swallow any error, never exceed ~1.5s.
        match tokio::time::timeout(
            Duration::from_millis(1500),
            api.post_heartbeat(&session_id, &final_beat),
        )
        .await
        {
            Ok(Ok(())) => tracing::debug!(session_id = %session_id, "final 'stopped' heartbeat sent"),
            Ok(Err(e)) => tracing::debug!(error = %e, "final 'stopped' heartbeat failed (best-effort)"),
            Err(_) => tracing::debug!("final 'stopped' heartbeat timed out (best-effort)"),
        }
    }

    #[cfg(unix)]
    {
        let _ = ipc_cancel_tx.send(());
        if tokio::time::timeout(Duration::from_secs(2), &mut ipc_task)
            .await
            .is_err()
        {
            ipc_task.abort();
            let _ = ipc_task.await;
            let _ = std::fs::remove_file(crate::ipc_socket::ipc_socket_path());
        }
    }
    // _lock + bridge drop here; SQLite closes, lock file deleted.
}
















#[cfg(all(test, target_os = "macos"))]
thread_local! {
    /// Test-only: how many times this thread asked the bridge to signal the working set.
    static SIGNALS_ASKED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}









/// A failed tick reaches none of the signal sites after the tick, so it pays a pending debt here. Only the
/// shared-roots signal runs earlier in the same iteration, and a delivered signal pays the debt, so at most
/// one signal is delivered per tick. The signal is local and needs no network: without it, an offline start
/// kept the startup resets (and a signal a busy gate dropped) from the system until the network came back.
/// Nothing owed, nothing sent.
fn signal_owed_debt_after_a_failed_tick(owed_working_set_signal: &mut OwedWorkingSetSignal, db: &StateDb) {
    owed_working_set_signal.take_command_debt();
    if owed_working_set_signal.is_owing() {
        signal_working_set_owing(owed_working_set_signal, db, "owed_debt", &[]);
    }
}

/// One working-set signal with the debt carried: signals `owed + ids`, and owes them again when the gate
/// was busy. Every call site in the tick loop goes through this one.
fn signal_working_set_owing(owed: &mut OwedWorkingSetSignal, db: &StateDb, reason: &str, ids: &[String]) {
    owed.take_command_debt();
    let merged = owed.merged_with(ids);
    let dropped = signal_file_provider_working_set(db, reason, &merged);
    owed.record(dropped, merged);
}


/// The tick's queue step: one run of the local operation queue and the one working-set signal it owes. `applied` is
/// the tick's remote applies, merged into that one signal (task 1697 review fix T3). Spec 2026-10-10 §6.2 (task 1887):
/// the send phase's wake runs this same step between ticks, with no remote applies, so the phase adds no signal site;
/// while a phase is armed, what each attempt met is noted on its link, the tick's attempts included. Returns how many
/// operations completed.
async fn queue_step(
    bridge: &Arc<EngineBridge>,
    sync_root: &Path,
    db: &StateDb,
    owed_working_set_signal: &mut OwedWorkingSetSignal,
    applied: &[String],
    send_phase: &SendPhaseLink,
) -> u32 {
    match bridge.process_due_operations(sync_root, now_secs()).await {
        Ok(outcome) => {
            if send_phase.is_armed() {
                send_phase.note(&outcome);
            }
            let completed = outcome.completed_op_ids.len() as u32;
            // Task 1697 review fix (T3): ONE signal per tick
            // carries the union of the local queue's
            // invalidations and the REMOTE ingestion applies —
            // in a mixed tick two signals would be redundant.
            let (reason, ids) = tick_working_set_signal(&outcome.invalidated_item_ids, applied);
            signal_working_set_owing(owed_working_set_signal, db, reason, &ids);
            if !outcome.paused_op_ids.is_empty() || !outcome.retried_op_ids.is_empty() {
                tracing::info!(
                    paused = outcome.paused_op_ids.len(),
                    retried = outcome.retried_op_ids.len(),
                    "sync operation queue processed with deferred work"
                );
            }
            completed
        }
        Err(e) => {
            tracing::warn!(error = %e, "operation queue processing failed");
            // The local queue stalled, but this tick's REMOTE
            // ingestion may still have applied server-side
            // changes — Finder must hear about them anyway.
            if !applied.is_empty() {
                signal_working_set_owing(owed_working_set_signal, db, "remote_changes_applied", applied);
            }
            0
        }
    }
}



/// Aborts a spawned task when dropped. On Windows it also covers the forced-abort
/// path of the heartbeat producer; the status pulse uses it on every platform so a
/// force-aborted `run` cannot leave the pulse running with the DB handle.
struct AbortWorkerOnDrop(tokio::task::AbortHandle);

impl Drop for AbortWorkerOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

// ── Task 10 + 11: per-conflict UI fan-out ─────────────────────────────────────


// ── Task 13: 24-hour auto-resolution timer ────────────────────────────────────



#[cfg(test)]
mod tests {
    use super::*;

    mod abort;
    mod heartbeat;
    mod tick;
    mod working_set;
}

mod telemetry;
use telemetry::*;
pub use telemetry::{AuthHealth};
pub(crate) use telemetry::{hostname_or_unknown, device_name};

mod lifecycle;
use lifecycle::*;
pub use lifecycle::{SendPhaseLink, AttemptSignals, ResumeTicket, EngineRunner, AbortOutcome};
pub(crate) use lifecycle::{phase_step_due, Wake, with_startup_guard};

mod tick_loop;
use tick_loop::*;
pub(crate) use tick_loop::RunMode;

mod working_set_signal;
use working_set_signal::*;
pub(crate) use working_set_signal::{owe_working_set_signal};
