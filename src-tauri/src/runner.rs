//! Long-running engine task: owns a [`SyncEngine`] on a tokio task,
//! holds the [`crate::lockfile::LockFile`], and emits status events
//! to the WebView via Tauri.
//!
//! ## Lifecycle
//!
//! - `EngineRunner::spawn` is called from either:
//!   - `setup()` if `desktop.toml` already has a sync_root AND we
//!     somehow already have a session (only the in-memory case
//!     today; persisted-session work lands in a later step), or
//!   - the `set_session` IPC handler immediately after the WebView
//!     pushes a fresh session.
//!
//! - The runner drops the prior runner first so a re-login or a
//!   sync-root change cleanly tears down the old engine.
//!
//! - On `clear_session` (logout) or app shutdown, [`EngineRunner::abort`]
//!   is called: a oneshot fires, the engine task exits, the lock file
//!   is released via the `LockFile` `Drop` impl.
//!
//! ## Status events
//!
//! The task emits `engine-status` events to the main window with a
//! JSON payload `{state, sync_root, error?}`. The WebView listens
//! with `listen("engine-status", …)` and updates its UI. Tray-icon
//! state animation (spec 030 §5) consumes the same stream.

use std::path::{Path, PathBuf};
use std::time::Duration;

use beebeeb_sync::{SyncConfig, SyncEngine};
use beebeeb_sync::SyncStatus;
use tauri::{AppHandle, Emitter};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::lockfile::LockFile;

/// How often the engine polls the file watcher and runs a sync cycle.
/// Keep small so changes are quickly picked up; the actual sync work
/// is internally idempotent and will skip files with no pending state.
const TICK_INTERVAL: Duration = Duration::from_secs(5);

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
    /// Spawn the engine on a background tokio task. Returns the
    /// handle synchronously — actual engine startup (lock acquire,
    /// watcher boot) happens inside the task.
    pub fn spawn(
        app: AppHandle,
        sync_root: PathBuf,
        session_token: String,
    ) -> Self {
        let (tx, rx) = oneshot::channel::<()>();

        let task = tokio::spawn(async move {
            run(app, sync_root, session_token, rx).await;
        });

        Self {
            cancel: Some(tx),
            task: Some(task),
        }
    }

    /// Signal the engine task to stop and wait for it to do so. Drops
    /// the lock file as part of teardown. Idempotent — calling twice
    /// is a no-op.
    pub async fn abort(mut self) {
        if let Some(tx) = self.cancel.take() {
            // Ignore send errors — receiver may have already exited.
            let _ = tx.send(());
        }
        if let Some(handle) = self.task.take() {
            // Bound the wait so a misbehaving engine can't hang
            // shutdown. The engine `Drop` releases the lock either
            // way once the task is forced down.
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

/// The engine task body. Acquires the lock, builds and starts the
/// engine, ticks every [`TICK_INTERVAL`] running a sync cycle, exits
/// when the cancel channel fires.
async fn run(
    app: AppHandle,
    sync_root: PathBuf,
    session_token: String,
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

    let config = SyncConfig::new(sync_root.clone(), api_base_url(), session_token);
    let mut engine = SyncEngine::new(config);

    if let Err(e) = engine.start() {
        tracing::error!(error = %e, "sync engine failed to start");
        emit_status(
            &app,
            "error",
            Some(&sync_root),
            Some(&format!("engine start failed: {e}")),
        );
        return;
    }
    emit_engine_status(&app, &sync_root, engine.state().status.clone());

    let mut tick = tokio::time::interval(TICK_INTERVAL);

    loop {
        tokio::select! {
            biased;
            _ = &mut cancel => break,
            _ = tick.tick() => {
                engine.poll_watcher();
                if let Err(e) = engine.sync_once() {
                    // A single failed cycle isn't fatal — log and
                    // continue. The next tick retries.
                    tracing::warn!(error = %e, "sync cycle failed");
                }
                // Mirror the engine's own status to the WebView and
                // tray. Each tick currently produces an event even
                // if the status didn't change — the listener side
                // is cheap (tooltip update + JSON parse) and this
                // lets the tray "tick" visibly during sustained
                // sync activity. Optimise to dedup if needed later.
                emit_engine_status(&app, &sync_root, engine.state().status.clone());
            }
        }
    }

    engine.stop();
    emit_status(&app, "stopped", Some(&sync_root), None);
    // _lock dropped here releases .beebeeb-sync.lock
}

/// Translate a `SyncStatus` into the shape the WebView + tray expect.
fn emit_engine_status(app: &AppHandle, sync_root: &Path, status: SyncStatus) {
    let (state, error, files_remaining) = match status {
        SyncStatus::Idle => ("idle", None, None),
        SyncStatus::Syncing { files_remaining, .. } => {
            ("syncing", None, Some(files_remaining))
        }
        SyncStatus::Paused => ("paused", None, None),
        SyncStatus::Error(msg) => ("error", Some(msg), None),
        SyncStatus::Offline => ("offline", None, None),
    };
    let payload = serde_json::json!({
        "state": state,
        "sync_root": sync_root.to_string_lossy(),
        "error": error,
        "files_remaining": files_remaining,
    });
    if let Err(e) = app.emit("engine-status", payload) {
        tracing::warn!(error = %e, "failed to emit engine-status event");
    }
}

/// Emit an engine-status event the WebView can listen to. Best-effort:
/// a missing main window or serialization issue is logged but doesn't
/// break the engine.
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
    });
    if let Err(e) = app.emit("engine-status", payload) {
        tracing::warn!(error = %e, "failed to emit engine-status event");
    }
}
