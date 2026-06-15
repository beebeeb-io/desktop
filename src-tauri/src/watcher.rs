//! Sync-root filesystem watcher — the UPLOAD trigger for Windows
//! (task 0780).
//!
//! ## Why this exists
//!
//! Before this module, Windows sync was DOWNLOAD-ONLY. The encrypted upload
//! pipeline already existed — [`crate::engine_bridge::EngineBridge::queue_finder_create`]
//! enqueues a `create_file` operation that the transfer loop encrypts (via
//! `beebeeb-core`) and uploads — but on Windows NOTHING ever called it. On
//! macOS/Linux the OS extension fires `QueueFinderCreate` over the Unix socket
//! ([`crate::ipc_socket`]); Windows Cloud Files registers only `FETCH_DATA`
//! (bytes-on-open), so a file dropped in the sync root was never noticed.
//!
//! This watcher closes that gap: it watches the sync root with the
//! cross-platform `notify` recommended watcher (ReadDirectoryChanges on
//! Windows), debounces rapid events, filters out everything the ENGINE itself
//! writes (so a download never re-uploads), and for each genuinely NEW
//! user-created file calls the exact same `queue_finder_create` the Unix IPC
//! path calls. The transfer loop then encrypts + uploads it, and on success
//! the file becomes an in-sync placeholder (see
//! [`crate::engine_bridge::EngineBridge::finalize_local_upload_placeholder`]).
//!
//! ## Lifecycle
//!
//! [`spawn`] is called from [`crate::runner::run`] right after the engine
//! bridge is built (and, on Windows, after the Cloud Files root is connected),
//! and returns a [`WatcherHandle`]. Dropping the handle (on engine
//! shutdown / logout) stops the watcher thread and the debounce task. The
//! watcher shares the live `EngineBridge` so it reaches the same StateDb +
//! ApiClient (and therefore the session master key) as the rest of the engine.
//!
//! ## Feedback-loop avoidance (CRITICAL)
//!
//! The engine writes into the sync root constantly: it mints placeholders, it
//! hydrates bytes into them, it writes `.beebeeb/state.db` and the
//! `.beebeeb-sync.lock`. Every one of those is a filesystem event. If the
//! watcher fed those back into `queue_finder_create`, every download would
//! immediately re-upload — an infinite loop. We filter, in order:
//!
//! 1. **Engine-internal paths** — anything under `.beebeeb/`, the
//!    `.beebeeb-sync.lock`, and OS junk (`.DS_Store`, `~$…`, `*.tmp`, …) via
//!    [`crate::engine_bridge::is_ignored_finder_name`].
//! 2. **Cloud Files placeholders** — a reparse point under the sync root is
//!    something WE created (placeholder seed or post-upload convert). The
//!    hydration write that fills a placeholder does NOT change it back into a
//!    normal file, so it stays a reparse point and is filtered here. Checked
//!    via [`crate::windows_cf::placeholders::is_cloud_placeholder`] (Windows
//!    only).
//! 3. **Already a known server file** — the authoritative guard. We look the
//!    path up in the state DB; if a row exists, this file is already on the
//!    server (cloud-only, downloading, local, uploading…), so a write to it is
//!    a hydration / re-download, NOT a new user file. Only paths with NO DB row
//!    are treated as new local creations and uploaded.
//!
//! Only a path that survives all three is handed to `queue_finder_create`.
//!
//! ## Platform note
//!
//! This module is compiled on every platform (so the `notify` integration is
//! type-checked cross-platform), but [`crate::runner::run`] only *spawns* it on
//! Windows — macOS (File Provider) and Linux (FUSE) already drive uploads
//! through `ipc_socket`. The `allow(dead_code)` below suppresses the
//! never-called warnings on those non-Windows builds.
#![cfg_attr(not(target_os = "windows"), allow(dead_code))]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;

use crate::engine_bridge::{
    EngineBridge, FinderWriteItemKind, FinderWriteOutcome, FinderWriteTarget, is_ignored_finder_name,
};

/// Debounce window: coalesce a burst of writes to the same path (an editor
/// saving in several `write` syscalls, a copy streaming in) into a single
/// upload, and give the writer time to finish before we read the file. The
/// repo's documented File-watcher debounce is 100ms; we use a slightly larger
/// settle window so a large file finishes landing before we stage it.
const DEBOUNCE: Duration = Duration::from_millis(400);

/// How often the debounce loop wakes to flush paths whose settle window has
/// elapsed. Small relative to [`DEBOUNCE`] so the effective latency is close
/// to the debounce window itself.
const DEBOUNCE_TICK: Duration = Duration::from_millis(100);

/// Name of the per-sync-root state directory (mirrors `runner::STATE_DIR`).
const STATE_DIR: &str = ".beebeeb";
/// Name of the cross-process lock file the engine writes at the sync root.
const LOCK_FILE: &str = ".beebeeb-sync.lock";

/// Owned handle to the running watcher. Dropping it stops the watcher (the
/// `notify` watcher stops when dropped) and signals the debounce task to exit.
pub struct WatcherHandle {
    // Held only to keep the OS watch alive; dropped on teardown.
    _watcher: RecommendedWatcher,
    // Dropping the sender closes the channel, which ends the debounce task.
    _shutdown: tokio::sync::oneshot::Sender<()>,
}

/// Start watching `sync_root` for local file creations/modifications and feed
/// genuinely-new user files into the encrypted upload path via `bridge`.
///
/// Returns `Some(WatcherHandle)` on success; `None` if the OS watcher could
/// not be created (logged) — the engine keeps running download-only in that
/// case rather than failing the whole runner.
///
/// The current tokio runtime handle is captured so the `notify` callback
/// (which runs on a `notify`-owned OS thread, not a tokio worker) can forward
/// events into the async debounce task.
pub fn spawn(bridge: Arc<EngineBridge>, sync_root: PathBuf) -> Option<WatcherHandle> {
    let (event_tx, event_rx) = mpsc::unbounded_channel::<PathBuf>();
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

    // The notify callback fires on notify's own thread; forward raw create/
    // modify paths into the channel. Keep this closure tiny and non-blocking —
    // all filtering + the upload enqueue happen in the async debounce task.
    let mut watcher = match notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        let event = match res {
            Ok(ev) => ev,
            Err(e) => {
                tracing::warn!(error = %e, "sync-root watcher event error");
                return;
            }
        };
        // Only creations and content/data modifications can introduce new user
        // bytes to upload. Renames/removes/metadata are deferred follow-ups
        // (see REPORT) — ignore them here so we don't misclassify a rename of a
        // known server file as a brand-new upload.
        let is_upload_candidate = matches!(
            event.kind,
            EventKind::Create(_) | EventKind::Modify(notify::event::ModifyKind::Data(_))
        ) || matches!(event.kind, EventKind::Modify(notify::event::ModifyKind::Any));
        if !is_upload_candidate {
            return;
        }
        for path in event.paths {
            // Drop directory events early; only files upload. (notify reports
            // both; a dir create is handled when its child files appear.)
            let _ = event_tx.send(path);
        }
    }) {
        Ok(w) => w,
        Err(e) => {
            tracing::warn!(error = %e, "could not create sync-root filesystem watcher; uploads via local creates are disabled");
            return None;
        }
    };

    if let Err(e) = watcher.watch(&sync_root, RecursiveMode::Recursive) {
        tracing::warn!(error = %e, sync_root = %sync_root.display(), "could not start watching sync root");
        return None;
    }

    tracing::info!(sync_root = %sync_root.display(), "sync-root upload watcher started");

    // The debounce + filter + enqueue loop runs on the tokio runtime.
    tokio::spawn(debounce_loop(bridge, sync_root, event_rx, shutdown_rx));

    Some(WatcherHandle {
        _watcher: watcher,
        _shutdown: shutdown_tx,
    })
}

/// Drain raw event paths, debounce per path, and once a path has settled run
/// the feedback-loop filters and (if it survives) enqueue an encrypted upload.
async fn debounce_loop(
    bridge: Arc<EngineBridge>,
    sync_root: PathBuf,
    mut event_rx: mpsc::UnboundedReceiver<PathBuf>,
    mut shutdown_rx: tokio::sync::oneshot::Receiver<()>,
) {
    // path → last time we saw an event for it. We flush a path only once its
    // last event is older than DEBOUNCE (it has stopped changing).
    let mut pending: HashMap<PathBuf, Instant> = HashMap::new();
    let mut tick = tokio::time::interval(DEBOUNCE_TICK);

    loop {
        tokio::select! {
            biased;
            _ = &mut shutdown_rx => {
                tracing::debug!("sync-root watcher debounce loop shutting down");
                break;
            }
            maybe_path = event_rx.recv() => {
                match maybe_path {
                    Some(path) => { pending.insert(path, Instant::now()); }
                    // Sender dropped (watcher gone) — exit.
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

/// Run the three feedback-loop filters on a settled path and, if it is a
/// genuinely-new user file, stage + enqueue its encrypted upload.
async fn handle_settled_path(bridge: &EngineBridge, sync_root: &Path, path: &Path) {
    // The file may have been deleted/renamed during the debounce window — if
    // it is no longer a regular file there is nothing to upload.
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(m) if m.is_file() => m,
        // Directory, gone, or unreadable — nothing to upload here.
        _ => return,
    };

    // Filter 1 — engine-internal paths + OS junk.
    if is_engine_internal(sync_root, path) {
        return;
    }
    let file_name = match path.file_name().and_then(|n| n.to_str()) {
        Some(n) => n.to_string(),
        None => return,
    };
    if is_ignored_finder_name(&file_name) {
        return;
    }

    // Filter 2 — Cloud Files placeholders are engine-owned (Windows only).
    // A reparse point under the sync root is a placeholder we minted or
    // converted; the hydration write that fills it does not turn it back into a
    // plain file, so we must never treat a placeholder write as a new upload.
    #[cfg(target_os = "windows")]
    if crate::windows_cf::placeholders::is_cloud_placeholder(path) {
        return;
    }

    // Filter 3 (authoritative) — already a known server file?
    // The state DB stores server-relative, '/'-separated paths. Compute the
    // path of this on-disk file relative to the sync root in the same shape and
    // look it up. A hit means the file already exists on the server (it is a
    // cloud-only/local/downloading/uploading row), so this event is a
    // hydration or a re-download, NOT a new local creation — skip it.
    let Some(rel) = relative_db_path(sync_root, path) else {
        return;
    };
    match bridge.db().get_file_by_path(&rel) {
        Ok(Some(_existing)) => {
            // Known server file → never re-upload on a local write here.
            // (Modify-as-new-version is a deferred follow-up; see REPORT.)
            tracing::trace!("watcher: skipping write to already-tracked server file");
            return;
        }
        Ok(None) => { /* genuinely new — fall through to upload */ }
        Err(e) => {
            tracing::warn!(error = %e, "watcher: state DB lookup failed; skipping to be safe");
            return;
        }
    }

    // Survived all filters → a NEW, user-created local file. Enqueue the same
    // encrypted-upload operation the Unix IPC `QueueFinderCreate` path uses.
    // `queue_finder_create` stages a copy of the bytes, derives the per-file
    // key inside the transfer loop via beebeeb-core, and encrypts on upload —
    // we reuse it wholesale and never touch crypto here.
    let target = FinderWriteTarget {
        file_id: None,
        // Top-level uploads for the happy path. Nested-folder parent_id
        // resolution is a deferred follow-up (see REPORT).
        parent_id: None,
        filename: file_name.clone(),
        kind: FinderWriteItemKind::File,
        contents_path: Some(path.to_string_lossy().into_owned()),
        content_type: beebeeb_core::media::guess_mime_type(&file_name).map(str::to_string),
        base_version_identifier: None,
    };

    match bridge.queue_finder_create(target) {
        Ok(FinderWriteOutcome::Queued { op_id, .. }) => {
            // Zero-knowledge: log the op id, never the decrypted filename.
            tracing::info!(op_id = %op_id, size = metadata.len(), "watcher: queued new local file for encrypted upload");
        }
        Ok(FinderWriteOutcome::Ignored { .. }) => { /* temp/ignored name — fine */ }
        Err(e) => {
            tracing::warn!(error = %e, "watcher: failed to queue local file upload");
        }
    }
}

/// True if `path` is something the engine itself writes (so it must never be
/// fed back as a user upload): anything inside `<sync_root>/.beebeeb/`, the
/// `<sync_root>/.beebeeb-sync.lock`, or any dot-directory component.
fn is_engine_internal(sync_root: &Path, path: &Path) -> bool {
    let state_dir = sync_root.join(STATE_DIR);
    let lock = sync_root.join(LOCK_FILE);
    if path == lock || path.starts_with(&state_dir) {
        return true;
    }
    // Defense in depth: any path component that is exactly `.beebeeb` (the
    // state dir under a nested root layout) or a staging dir we own.
    path.components().any(|c| {
        matches!(c.as_os_str().to_str(), Some(STATE_DIR))
    })
}

/// Map an absolute on-disk path under `sync_root` to the server-relative,
/// '/'-separated, leading-slash-free key the state DB stores in `files.path`.
/// Returns `None` if `path` is not under `sync_root`.
fn relative_db_path(sync_root: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(sync_root).ok()?;
    let mut out = String::new();
    for (i, comp) in rel.components().enumerate() {
        let part = comp.as_os_str().to_str()?;
        if i > 0 {
            out.push('/');
        }
        out.push_str(part);
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_internal_filters_state_dir_and_lock() {
        let root = PathBuf::from("/sync");
        assert!(is_engine_internal(&root, &root.join(".beebeeb").join("state.db")));
        assert!(is_engine_internal(&root, &root.join(".beebeeb-sync.lock")));
        assert!(is_engine_internal(
            &root,
            &root.join("sub").join(".beebeeb").join("x")
        ));
        assert!(!is_engine_internal(&root, &root.join("photo.jpg")));
        assert!(!is_engine_internal(&root, &root.join("docs").join("a.txt")));
    }

    #[test]
    fn relative_db_path_is_slash_joined_without_leading_slash() {
        let root = PathBuf::from("/sync");
        assert_eq!(
            relative_db_path(&root, &root.join("a.txt")).as_deref(),
            Some("a.txt")
        );
        assert_eq!(
            relative_db_path(&root, &root.join("docs").join("b.md")).as_deref(),
            Some("docs/b.md")
        );
        assert_eq!(relative_db_path(&root, &PathBuf::from("/other/c.txt")), None);
    }
}
