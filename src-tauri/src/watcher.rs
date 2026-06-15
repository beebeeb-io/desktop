//! Sync-root upload driver — the UPLOAD trigger for Windows (task 0780).
//!
//! ## Why this exists
//!
//! Before this module, Windows sync was DOWNLOAD-ONLY. The encrypted upload
//! pipeline already existed — [`crate::engine_bridge::EngineBridge::queue_finder_create`]
//! enqueues a `create_file` operation that the transfer loop encrypts (via
//! `beebeeb-core`) and uploads — but on Windows NOTHING ever called it. On
//! macOS/Linux the OS extension fires `QueueFinderCreate` over the Unix socket
//! ([`crate::ipc_socket`]); the Windows Cloud Files root needs an in-process
//! trigger.
//!
//! ## Why a `notify`/ReadDirectoryChanges watcher does NOT work here
//!
//! The first cut of this module watched the sync root with the cross-platform
//! `notify` crate (ReadDirectoryChanges on Windows). That watcher does **not
//! fire** on a folder that has been handed to the Cloud Files filter via
//! `CfConnectSyncRoot`: the CF filter sits between the filesystem and the
//! ReadDirectoryChanges machinery, so user writes into a connected sync root
//! never surface as `notify` events. The result was a permanently download-only
//! sync — a file dropped in the folder was never noticed.
//!
//! The Cloud Files–native replacement is the `CF_CALLBACK_TYPE_NOTIFY_*`
//! callbacks (registered in [`crate::windows_cf::connect_callbacks`]). Those
//! DO fire on a connected root. This module is now the **debounce + dispatch**
//! half of that path: the CF callbacks push events into a channel; this module
//! debounces create/modify bursts, classifies each settled path through the one
//! shared [`EngineBridge::classify_local_path`], and enqueues the encrypted
//! upload / delete / move-or-rename via the existing `queue_finder_*` ops. The
//! transfer loop then encrypts + uploads; on success a new local file becomes an
//! in-sync placeholder (see
//! [`crate::engine_bridge::EngineBridge::finalize_local_upload_placeholder`]).
//!
//! ## Lifecycle
//!
//! [`spawn`] is called from [`crate::runner::run`] right after the engine
//! bridge is built (and, on Windows, after the Cloud Files root is connected),
//! and returns a [`WatcherHandle`]. It registers an [`mpsc`] sender into the
//! [`crate::windows_cf`] callback layer (via [`crate::windows_cf::set_notify_sender`])
//! and spawns the debounce loop. Dropping the handle (on engine shutdown /
//! logout) signals the debounce task to exit; the callbacks then find no live
//! receiver and drop events harmlessly.
//!
//! ## Feedback-loop avoidance (CRITICAL)
//!
//! The engine writes into the sync root constantly: it mints placeholders, it
//! hydrates bytes into them, it writes `.beebeeb/state.db` and the
//! `.beebeeb-sync.lock`. Every one of those can fire a NOTIFY callback. If we
//! fed those back into `queue_finder_create`, every download would immediately
//! re-upload — an infinite loop. ALL of that filtering now lives in the one
//! shared [`EngineBridge::classify_local_path`] (engine-internal paths, Cloud
//! Files reparse-point placeholders, and the authoritative "already a known
//! server file" DB guard), so the retired `notify` path and these CF callbacks
//! share ONE correct, parent-aware code path. See that method's docs.
//!
//! ## Platform note
//!
//! This module is compiled on every platform (so the `mpsc` plumbing is
//! type-checked cross-platform), but [`crate::runner::run`] only *spawns* it on
//! Windows — macOS (File Provider) and Linux (FUSE) already drive uploads
//! through `ipc_socket`. The `allow(dead_code)` below suppresses the
//! never-called warnings on those non-Windows builds.
#![cfg_attr(not(target_os = "windows"), allow(dead_code))]

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::mpsc;

use crate::engine_bridge::{EngineBridge, FinderWriteOutcome};

/// Debounce window: coalesce a burst of writes to the same path (an editor
/// saving in several `write` syscalls, a copy streaming in) into a single
/// upload, and give the writer time to finish before we stage the file. The
/// repo's documented file-watcher debounce is 100ms; we use a slightly larger
/// settle window so a large file finishes landing before we stage it.
///
/// `CF_CALLBACK_TYPE_NOTIFY_FILE_CLOSE_COMPLETION` is far less chatty than
/// ReadDirectoryChanges (one event per handle-close, not per write syscall),
/// but a single logical save can still close several handles, so we still
/// path-key debounce.
const DEBOUNCE: Duration = Duration::from_millis(400);

/// How often the debounce loop wakes to flush paths whose settle window has
/// elapsed. Small relative to [`DEBOUNCE`] so the effective latency is close to
/// the debounce window itself.
const DEBOUNCE_TICK: Duration = Duration::from_millis(100);

/// An event delivered by a Windows Cloud Files NOTIFY callback. Push-only from
/// the callback side ([`crate::windows_cf::callbacks`]); consumed by
/// [`debounce_loop`].
#[derive(Debug, Clone)]
pub enum NotifyEvent {
    /// A handle that may have written a new/modified file has closed
    /// (`NOTIFY_FILE_CLOSE_COMPLETION`). The create/modify trigger. Debounced
    /// per path, then classified + uploaded if it is a genuinely-new user file.
    CloseCompletion(PathBuf),
    /// A file/dir was deleted (`NOTIFY_DELETE_COMPLETION`). Point-in-time — the
    /// op already happened, so no debounce: dispatched immediately.
    Delete(PathBuf),
    /// A file/dir was renamed or moved (`NOTIFY_RENAME_COMPLETION`). `source`
    /// is the old absolute path, `target` the new absolute path. Point-in-time,
    /// dispatched immediately.
    Rename { source: PathBuf, target: PathBuf },
}

/// Owned handle to the running upload driver. Dropping it signals the debounce
/// task to exit; once it exits the registered NOTIFY sender is closed, so the
/// Cloud Files callbacks find no receiver and drop events harmlessly.
pub struct WatcherHandle {
    // Dropping the sender closes the channel, which ends the debounce task.
    _shutdown: tokio::sync::oneshot::Sender<()>,
}

/// Start the upload driver for `sync_root`: register the NOTIFY-event sender so
/// the Cloud Files callbacks can push create/modify/delete/rename events, and
/// spawn the debounce + classify + enqueue loop.
///
/// Returns `Some(WatcherHandle)`. (It is infallible today — there is no OS
/// watcher to fail — but the `Option` return is kept so the call site in
/// `runner::run` is unchanged and a future fallible setup step can short-circuit
/// to `None`.)
pub fn spawn(bridge: Arc<EngineBridge>, sync_root: PathBuf) -> Option<WatcherHandle> {
    let (event_tx, event_rx) = mpsc::unbounded_channel::<NotifyEvent>();
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

    // Hand the sender to the Cloud Files callback layer. The `extern "system"`
    // NOTIFY callbacks can't capture state, so they reach this sender through a
    // OnceLock (set here). Windows-only; a no-op on other platforms.
    #[cfg(target_os = "windows")]
    crate::windows_cf::set_notify_sender(event_tx.clone());

    // Keep `event_tx` alive for the lifetime of the loop on every platform so
    // the channel doesn't close immediately on non-Windows builds (where the
    // sender is never registered). The debounce loop owns the receiver.
    let _keep_tx = event_tx;

    tracing::info!(sync_root = %sync_root.display(), "sync-root upload driver started (Cloud Files NOTIFY)");

    tokio::spawn(debounce_loop(bridge, sync_root, event_rx, shutdown_rx, _keep_tx));

    Some(WatcherHandle {
        _shutdown: shutdown_tx,
    })
}

/// Drain NOTIFY events: debounce close-completions per path and dispatch
/// deletes/renames immediately. Once a close-completion path has settled, run
/// the shared classifier and (if it survives) enqueue an encrypted upload.
async fn debounce_loop(
    bridge: Arc<EngineBridge>,
    sync_root: PathBuf,
    mut event_rx: mpsc::UnboundedReceiver<NotifyEvent>,
    mut shutdown_rx: tokio::sync::oneshot::Receiver<()>,
    // Held so the channel stays open for the whole loop lifetime even if no
    // callback sender is registered (non-Windows / pre-callback startup).
    _keep_tx: mpsc::UnboundedSender<NotifyEvent>,
) {
    // path → last time we saw a close-completion for it. We flush a path only
    // once its last event is older than DEBOUNCE (it has stopped changing).
    let mut pending: HashMap<PathBuf, Instant> = HashMap::new();
    let mut tick = tokio::time::interval(DEBOUNCE_TICK);

    loop {
        tokio::select! {
            biased;
            _ = &mut shutdown_rx => {
                tracing::debug!("sync-root upload driver shutting down");
                break;
            }
            maybe_event = event_rx.recv() => {
                match maybe_event {
                    Some(NotifyEvent::CloseCompletion(path)) => {
                        // Debounce: record/refresh the settle timer for this path.
                        pending.insert(path, Instant::now());
                    }
                    Some(NotifyEvent::Delete(path)) => {
                        // The file is already gone — handle immediately. Drop any
                        // pending close-completion for the same path so a stale
                        // settle doesn't try to upload a now-deleted file.
                        pending.remove(&path);
                        handle_delete(&bridge, &sync_root, &path).await;
                    }
                    Some(NotifyEvent::Rename { source, target }) => {
                        // A rename invalidates a pending close-completion for the
                        // OLD path; the NEW path's close (if any) will arrive on
                        // its own event.
                        pending.remove(&source);
                        handle_rename(&bridge, &sync_root, &source, &target).await;
                    }
                    // Sender dropped (handle gone) — exit.
                    None => break,
                }
            }
            _ = tick.tick() => {
                let now = Instant::now();
                let ready: Vec<PathBuf> = pending
                    .iter()
                    .filter(|(_, seen)| now.duration_since(**seen) >= DEBOUNCE)
                    .map(|(p, _)| p.clone())
                    .collect();
                for path in ready {
                    pending.remove(&path);
                    handle_settled_path(&bridge, &sync_root, &path).await;
                }
            }
        }
    }
}

/// Classify a settled create/modify path through the one shared, parent-aware
/// [`EngineBridge::classify_local_path`] and, if it is a genuinely-new user
/// file, enqueue its encrypted upload. All feedback-loop filtering + parent_id
/// resolution lives in the classifier — this function only enqueues.
async fn handle_settled_path(bridge: &EngineBridge, sync_root: &std::path::Path, path: &std::path::Path) {
    let Some(target) = bridge.classify_local_path(sync_root, path) else {
        return;
    };

    // Capture size for the log before `target` is moved into the queue call.
    let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let nested = target.parent_id.is_some();

    // Windows: hand the new plain file to the Cloud Files filter as an UNSYNCED
    // placeholder BEFORE we queue the upload, so the filter doesn't
    // reclaim/dehydrate it while the transfer loop reads its bytes. No
    // MARK_IN_SYNC — it isn't on the server yet; `finalize_local_upload_placeholder`
    // re-stamps it in-sync (with the real server file_id) once the upload lands.
    // Best-effort: a failure leaves a working plain file that still uploads.
    #[cfg(target_os = "windows")]
    {
        let local_id = uuid::Uuid::new_v4().to_string();
        if let Err(e) = crate::windows_cf::placeholders::convert_to_unsynced_placeholder(path, &local_id) {
            tracing::warn!(error = %e, "upload driver: could not pre-convert new file to unsynced placeholder");
        }
    }

    // `queue_finder_create` stages a copy of the bytes, derives the per-file key
    // inside the transfer loop via beebeeb-core, and encrypts on upload — we
    // reuse it wholesale and never touch crypto here.
    match bridge.queue_finder_create(target) {
        Ok(FinderWriteOutcome::Queued { op_id, .. }) => {
            // Zero-knowledge: log the op id, never the decrypted filename.
            tracing::info!(op_id = %op_id, size, nested, "upload driver: queued new local file for encrypted upload");
        }
        Ok(FinderWriteOutcome::Ignored { .. }) => { /* temp/ignored name — fine */ }
        Err(e) => {
            tracing::warn!(error = %e, "upload driver: failed to queue local file upload");
        }
    }
}

/// A local delete fired. If the path maps to a known server file, enqueue the
/// existing trash op so the server deletes it too. If there is no DB row, the
/// delete was of an untracked local file (or an engine-internal file) — nothing
/// to propagate.
async fn handle_delete(bridge: &EngineBridge, sync_root: &std::path::Path, path: &std::path::Path) {
    // Engine-internal deletes (state.db churn, lock file) must never propagate.
    if crate::engine_bridge::path_is_engine_internal(sync_root, path) {
        return;
    }
    let Some(rel) = crate::engine_bridge::relative_db_path(sync_root, path) else {
        return;
    };
    match bridge.db().get_file_by_path(&rel) {
        Ok(Some(entry)) => match bridge.queue_finder_delete(&entry.file_id, None) {
            Ok(FinderWriteOutcome::Queued { op_id, .. }) => {
                tracing::info!(op_id = %op_id, "upload driver: queued server delete for locally-removed file");
            }
            Ok(FinderWriteOutcome::Ignored { .. }) => {}
            Err(e) => {
                tracing::warn!(error = %e, "upload driver: failed to queue server delete");
            }
        },
        Ok(None) => { /* untracked local file removed — nothing to propagate */ }
        Err(e) => {
            tracing::warn!(error = %e, "upload driver: delete DB lookup failed; skipping");
        }
    }
}

/// A local rename/move fired. Resolve the SOURCE path to a known server file and
/// enqueue a metadata update describing its new name + new parent. The
/// `queue_finder_modify` metadata path already maps a present `parent_id`
/// (changed parent) → MoveFile and a name-only change → RenameFile, so we hand
/// it the new filename + the resolved new parent and let it pick.
async fn handle_rename(
    bridge: &EngineBridge,
    sync_root: &std::path::Path,
    source: &std::path::Path,
    target: &std::path::Path,
) {
    // A rename whose target is engine-internal (or whose source was) is not a
    // user action we propagate.
    if crate::engine_bridge::path_is_engine_internal(sync_root, source)
        || crate::engine_bridge::path_is_engine_internal(sync_root, target)
    {
        return;
    }

    let Some(source_rel) = crate::engine_bridge::relative_db_path(sync_root, source) else {
        return;
    };
    let existing = match bridge.db().get_file_by_path(&source_rel) {
        Ok(Some(entry)) => entry,
        // Source not tracked. The rename may have brought an untracked local
        // file to a new name — classify the TARGET as a possible new upload
        // instead (it will no-op if it is engine-owned / already tracked).
        Ok(None) => {
            handle_settled_path(bridge, sync_root, target).await;
            return;
        }
        Err(e) => {
            tracing::warn!(error = %e, "upload driver: rename source DB lookup failed; skipping");
            return;
        }
    };

    let Some(new_name) = target.file_name().and_then(|n| n.to_str()).map(str::to_string) else {
        return;
    };
    if crate::engine_bridge::is_ignored_finder_name(&new_name) {
        return;
    }

    // Resolve the NEW parent folder id from the target's parent directory. The
    // classifier's parent resolver is reused so move-into-subfolder gets the
    // right server parent; `None` means moved to (or kept at) the root. We pass
    // it through verbatim — `queue_finder_modify` treats `Some(parent)` as a
    // move and `None` as a rename-in-place.
    let new_parent_id = bridge.resolve_parent_id_for(sync_root, target);

    let modify = crate::engine_bridge::FinderWriteTarget {
        file_id: Some(existing.file_id.clone()),
        parent_id: new_parent_id,
        filename: new_name,
        kind: crate::engine_bridge::FinderWriteItemKind::File,
        // No new bytes — this is a metadata-only move/rename.
        contents_path: None,
        content_type: None,
        base_version_identifier: None,
    };

    match bridge.queue_finder_modify(modify) {
        Ok(FinderWriteOutcome::Queued { op_id, .. }) => {
            tracing::info!(op_id = %op_id, "upload driver: queued server rename/move for locally-renamed file");
        }
        Ok(FinderWriteOutcome::Ignored { .. }) => {}
        Err(e) => {
            tracing::warn!(error = %e, "upload driver: failed to queue server rename/move");
        }
    }
}
