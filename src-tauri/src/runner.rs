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
//!   - the `set_session` IPC handler immediately after the WebView
//!     pushes a fresh session, or
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
//! The task emits `engine-status` events with payload
//! `{state, sync_root, error?, files_remaining?}`. `state` is one of
//! `idle` / `syncing` / `error` / `stopped`. The WebView and the tray
//! tooltip listener (`attach_tray_status_listener` in `lib.rs`) both
//! consume this stream.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tauri::{AppHandle, Emitter};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::api_client::ApiClient;
use crate::conflict::auto_resolution_deadline;
use crate::engine_bridge::{CachePolicy, ConflictDetected, EngineBridge, sync_tick};
use crate::lockfile::LockFile;
use crate::state_db::{FileStatus, StateDb};

/// How often the runner pulls the file list from the server and
/// refreshes the state DB. Same cadence as the previous engine; small
/// enough that newly-uploaded remote files appear in the local mirror
/// quickly without hammering the API.
const TICK_INTERVAL: Duration = Duration::from_secs(5);

/// Subdirectory inside the sync root where the daemon keeps its
/// SQLite state DB. Hidden by leading dot so it doesn't pollute the
/// user's view of "files in their vault".
const STATE_DIR: &str = ".beebeeb";

/// API base URL the engine talks to. Overridden by `BEEBEEB_API_URL`
/// for local dev (so a developer can point at `http://localhost:3001`
/// without rebuilding the binary). Public to the crate so the
/// `resolve_conflict` IPC in `lib.rs` can build a fresh ApiClient
/// without duplicating the env-var read.
pub(crate) fn api_base_url() -> String {
    std::env::var("BEEBEEB_API_URL").unwrap_or_else(|_| "https://api.beebeeb.io".to_string())
}

/// Owned handle to a running engine task. Drop it (or call
/// [`Self::abort`]) to stop the engine.
pub struct EngineRunner {
    cancel: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<()>>,
}

impl EngineRunner {
    /// Spawn the runner on a background tokio task. Returns the handle
    /// synchronously — actual startup (lock acquire, DB open, first
    /// tick) happens inside the task.
    pub fn spawn(app: AppHandle, sync_root: PathBuf, session_token: String, master_key: [u8; 32]) -> Self {
        let (tx, rx) = oneshot::channel::<()>();

        let task = tokio::spawn(async move {
            run(app, sync_root, session_token, master_key, rx).await;
        });

        Self {
            cancel: Some(tx),
            task: Some(task),
        }
    }

    /// Signal the runner to stop and wait for it to do so. Drops the
    /// lock file as part of teardown. Idempotent — calling twice is a
    /// no-op.
    pub async fn abort(mut self) {
        if let Some(tx) = self.cancel.take() {
            // Ignore send errors — receiver may have already exited.
            let _ = tx.send(());
        }
        if let Some(handle) = self.task.take() {
            // Bound the wait so a misbehaving tick can't hang
            // shutdown. The lock file's Drop releases regardless once
            // the task is forced down.
            let _ = tokio::time::timeout(Duration::from_secs(3), handle).await;
        }
    }
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

/// The runner task body. Acquires the lock, opens the state DB,
/// builds the API client + engine bridge, ticks every
/// [`TICK_INTERVAL`] running [`sync_tick`], exits when the cancel
/// channel fires.
async fn run(
    app: AppHandle,
    sync_root: PathBuf,
    session_token: String,
    master_key: [u8; 32],
    mut cancel: oneshot::Receiver<()>,
) {
    // Acquire the mutual-exclusion lock with the CLI's `bb watch`.
    // If a CLI agent is alive in the same folder, we don't start.
    let _lock = match LockFile::acquire(&sync_root, "desktop") {
        Ok(l) => l,
        Err(msg) => {
            tracing::error!(error = %msg, "could not acquire .beebeeb-sync.lock");
            emit_status(&app, "error", Some(&sync_root), Some(&msg));
            return;
        }
    };

    // Open the state DB at <sync_root>/.beebeeb/state.db. Create the
    // directory if missing — first-launch path.
    let state_dir = sync_root.join(STATE_DIR);
    if let Err(e) = std::fs::create_dir_all(&state_dir) {
        let msg = format!("create state dir: {e}");
        tracing::error!(error = %msg);
        emit_status(&app, "error", Some(&sync_root), Some(&msg));
        return;
    }
    let db = match StateDb::open(state_dir.join("state.db")) {
        Ok(d) => Arc::new(d),
        Err(e) => {
            let msg = format!("open state.db: {e}");
            tracing::error!(error = %msg);
            emit_status(&app, "error", Some(&sync_root), Some(&msg));
            return;
        }
    };

    let api = Arc::new(ApiClient::new(api_base_url(), session_token, master_key));
    let bridge = Arc::new(EngineBridge::new(db.clone(), api));

    // Spawn the Unix-socket IPC server alongside the sync loop. It
    // shares the same StateDb + EngineBridge handles, so OS extensions
    // (macOS File Provider, Windows Cloud Files, Linux FUSE) can query
    // status and trigger hydrations without waiting for the next tick.
    //
    // We don't keep an explicit cancel handle on this task: serve_ipc
    // owns the listener, on the next runner spawn the listener bind
    // calls remove_file() first to clear stale sockets. When the
    // process exits, the OS reaps the task. Logout-time cleanup is
    // best-effort — the in-flight hydrates would only succeed if they
    // already had the master key from the previous session, and the
    // socket re-binds on the next login.
    {
        let db_for_ipc = db.clone();
        let bridge_for_ipc = bridge.clone();
        tokio::spawn(async move {
            crate::ipc_socket::serve_ipc(db_for_ipc, bridge_for_ipc).await;
        });
    }

    emit_status(&app, "running", Some(&sync_root), None);

    let mut tick = tokio::time::interval(TICK_INTERVAL);

    loop {
        tokio::select! {
            biased;
            _ = &mut cancel => break,
            _ = tick.tick() => {
                match bridge.refresh_shared_roots().await {
                    Ok(outcome) if !outcome.removed_shared_file_ids.is_empty() => {
                        tracing::info!(
                            removed = outcome.removed_shared_file_ids.len(),
                            "revoked shared content removed from local Finder state"
                        );
                    }
                    Ok(_) => {}
                    Err(e) => {
                        tracing::warn!(error = %e, "shared root refresh failed");
                    }
                }
                match sync_tick(&*bridge).await {
                    Ok(conflicts) => {
                        // Task 10 — surface freshly detected conflicts.
                        // The engine bridge already flipped status to
                        // Conflict; we own the UI side: open a window
                        // per file, fire a notification, emit a Tauri
                        // event so the settings page can refresh its
                        // counts immediately.
                        for c in &conflicts {
                            handle_new_conflict(&app, c);
                        }
                        // Task 13 — sweep for conflicts past their 24h
                        // deadline and apply Keep Both. Done after the
                        // detection step so a freshly-detected conflict
                        // (timestamp ≈ now) doesn't get auto-resolved
                        // on the very same tick.
                        sweep_auto_resolutions(&app, &bridge, &sync_root).await;
                        enforce_cache_budget(&bridge);
                        emit_status(&app, "idle", Some(&sync_root), None);
                    }
                    Err(e) => {
                        // Network blips and 401s during token rotation
                        // are normal — log and continue, surface the
                        // error to the WebView so the tray reflects it.
                        tracing::warn!(error = %e, "sync tick failed");
                        emit_status(&app, "error", Some(&sync_root), Some(&e.to_string()));
                    }
                }
            }
        }
    }

    emit_status(&app, "stopped", Some(&sync_root), None);
    // _lock + bridge drop here; SQLite closes, lock file deleted.
}

/// Emit an `engine-status` event the WebView + tray listen to.
/// Best-effort: a missing main window or serialisation issue is
/// logged but doesn't break the runner.
fn emit_status(app: &AppHandle, state: &str, sync_root: Option<&PathBuf>, error: Option<&str>) {
    let payload = serde_json::json!({
        "state": state,
        "sync_root": sync_root.map(|p| p.to_string_lossy().into_owned()),
        "error": error,
        "files_remaining": serde_json::Value::Null,
    });
    if let Err(e) = app.emit("engine-status", payload) {
        tracing::warn!(error = %e, "failed to emit engine-status event");
    }
}

fn enforce_cache_budget(bridge: &Arc<EngineBridge>) {
    match bridge.enforce_smart_cache(CachePolicy::default()) {
        Ok(outcome) if !outcome.evicted_file_ids.is_empty() => {
            tracing::info!(
                evicted = outcome.evicted_file_ids.len(),
                "smart cache cleanup evicted unpinned files"
            );
        }
        Ok(_) => {}
        Err(e) => tracing::warn!(error = %e, "smart cache cleanup failed"),
    }
}

// ── Task 10 + 11: per-conflict UI fan-out ─────────────────────────────────────

/// Surface a freshly-detected conflict to the user: open the
/// resolution window, fire a native notification, emit a Tauri event
/// so any open settings page can refresh its conflict counter without
/// waiting for the next 5 s tick. All three are best-effort — a failure
/// in any one path is logged and the others still run, since the
/// underlying state DB has already been updated and the conflict won't
/// be silently dropped.
fn handle_new_conflict(app: &AppHandle, c: &ConflictDetected) {
    if let Err(e) = crate::open_conflict_window_impl(app, &c.file_id, &c.file_name, c.is_text) {
        tracing::warn!(error = %e, file_id = %c.file_id, "open_conflict_window failed");
    }
    if let Err(e) = crate::notify_conflict_impl(app, &c.file_name) {
        // Don't escalate — Linux without a notification daemon, or
        // macOS where the user denied the permission prompt, will
        // both fail here. The window + event still fired.
        tracing::warn!(error = %e, file_id = %c.file_id, "notify_conflict failed");
    }
    if let Err(e) = app.emit(
        "engine-conflict",
        serde_json::json!({
            "file_id": c.file_id,
            "file_name": c.file_name,
            "is_text": c.is_text,
        }),
    ) {
        tracing::warn!(error = %e, "failed to emit engine-conflict event");
    }
}

// ── Task 13: 24-hour auto-resolution timer ────────────────────────────────────

/// Walk every file currently in `Conflict` status. For any whose
/// detection timestamp (stored in `modified_at` after Task 10 anchored
/// it on detect) is past the 24 h deadline, apply Keep Both: the local
/// copy gets a `(conflict - <hostname> - <date>)` suffix, the remote
/// becomes the new authoritative file, status flips back to `Local`.
///
/// Errors are logged per-file and do not stop the sweep — one bad file
/// shouldn't keep the rest from being resolved. The bridge's
/// [`crate::engine_bridge::EngineBridge::auto_resolve_keep_both`] is
/// engineered so that a partial failure leaves the local copy on disk
/// (renamed) plus the row in `Error` status — the user never loses
/// data, the next tick retries the remote hydrate.
async fn sweep_auto_resolutions(app: &AppHandle, bridge: &Arc<EngineBridge>, sync_root: &Path) {
    let conflicts = match bridge.db().list_by_status(FileStatus::Conflict) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "list_by_status(Conflict) failed");
            return;
        }
    };
    if conflicts.is_empty() {
        return;
    }

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    for entry in conflicts {
        // `modified_at` was anchored to the detection time when the
        // tick flipped this row to Conflict. A 0 here would mean
        // "detected at epoch" which is also fine (immediate auto-
        // resolve on launch is not a real scenario, since the row
        // had to be written by an earlier sync_tick).
        let detected = entry.modified_at as u64;
        if now < auto_resolution_deadline(detected) {
            continue;
        }

        tracing::info!(
            file_id = %entry.file_id,
            path = %entry.path,
            elapsed_secs = now.saturating_sub(detected),
            "auto-resolving conflict (24h elapsed) via Keep Both"
        );
        match bridge.auto_resolve_keep_both(sync_root, &entry).await {
            Ok(conflict_copy_name) => {
                if let Err(e) = app.emit(
                    "conflict-auto-resolved",
                    serde_json::json!({
                        "file_id": entry.file_id,
                        "file_name": entry.path,
                        "conflict_copy_name": conflict_copy_name,
                    }),
                ) {
                    tracing::warn!(error = %e, "failed to emit conflict-auto-resolved event");
                }
            }
            Err(e) => {
                tracing::warn!(
                    file_id = %entry.file_id,
                    error = %e,
                    "auto_resolve_keep_both failed — file kept on disk; will retry next tick"
                );
            }
        }
    }
}

// Re-export Path for parity with the previous file's signature surface.
// `_path` parameter type kept for forward-compat with the eventual
// per-file emit calls from engine_bridge.rs.
#[allow(dead_code)]
fn _unused_path(_p: &Path) {}
