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
use std::time::Duration;

use tauri::{AppHandle, Emitter};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::api_client::ApiClient;
use crate::engine_bridge::{sync_tick, EngineBridge};
use crate::lockfile::LockFile;
use crate::state_db::StateDb;

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
/// without rebuilding the binary).
fn api_base_url() -> String {
    std::env::var("BEEBEEB_API_URL")
        .unwrap_or_else(|_| "https://api.beebeeb.io".to_string())
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
    pub fn spawn(
        app: AppHandle,
        sync_root: PathBuf,
        session_token: String,
        master_key: [u8; 32],
    ) -> Self {
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
    let bridge = EngineBridge::new(db, api);
    emit_status(&app, "running", Some(&sync_root), None);

    let mut tick = tokio::time::interval(TICK_INTERVAL);

    loop {
        tokio::select! {
            biased;
            _ = &mut cancel => break,
            _ = tick.tick() => {
                match sync_tick(&bridge).await {
                    Ok(()) => emit_status(&app, "idle", Some(&sync_root), None),
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
fn emit_status(
    app: &AppHandle,
    state: &str,
    sync_root: Option<&PathBuf>,
    error: Option<&str>,
) {
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

// Re-export Path for parity with the previous file's signature surface.
// `_path` parameter type kept for forward-compat with the eventual
// per-file emit calls from engine_bridge.rs.
#[allow(dead_code)]
fn _unused_path(_p: &Path) {}
