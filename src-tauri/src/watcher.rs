//! Windows sync-root upload driver: CF notifications plus an enumeration fallback.
//!
//! CF callbacks reliably report existing-placeholder closes and namespace changes;
//! the native audit found that new plain-file creates need the periodic scan.
//! Both paths inspect tracked resident content before invoking the new-file
//! classifier. A confirmed server hash or latest staged hash suppresses read /
//! hydration feedback. Every distinct settled edit is durably staged as a version.
//!
//! Close, delete and rename events settle for 400ms: editors may briefly remove
//! the destination or rename it to a backup while replacing its bytes. A tracked
//! path occupied again after settling keeps its original server identity.
//!
//! The scan skips engine internals, symlinks, disabled backups and unsupported
//! entries. Previously queued tracked paths remain eligible for later saves.
//! Native last-write in-sync tracking protects edits before the debounce fires;
//! reclamation independently verifies queue state and confirmed bytes.
//!
//! This module compiles everywhere for portable tests but is spawned only on
//! Windows. macOS File Provider and Linux IPC continue to drive their own writes.
#![cfg_attr(not(target_os = "windows"), allow(dead_code))]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::mpsc;

// ── Engine-delete suppression set (task 0806) ──────────────────────────────────
//
// The REMOTE→local deletion reconcile (`engine_bridge` → `windows_cf::delete_placeholder`)
// removes an on-disk CF placeholder when a file/folder was trashed/deleted on
// another client. Removing that placeholder fires `NOTIFY_DELETE_COMPLETION`,
// which lands here as `NotifyEvent::Delete` → `handle_delete` → a redundant server
// `TrashFile` op. That would be wrong (the server ALREADY trashed it) and is a
// feedback loop.
//
// The PRIMARY guard is ordering: the engine removes the DB ROW *before* the
// placeholder, so `handle_delete`'s `get_file_by_path` returns `Ok(None)` and
// queues nothing. This set is the DETERMINISTIC second guard for the window where
// the row may not be observable yet (and to drop the event before `handle_delete`
// runs at all): the engine registers each path it is about to delete via
// [`suppress_engine_delete`], and the debounce loop consults [`take_engine_delete_suppressed`]
// on every `Delete` event — a hit means "engine-originated, do not propagate".
//
// Mirrors the existing `in_flight` CREATE-suppression set (scan_loop) and the
// `windows_cf::NOTIFY_TX` static: an `extern "system"` callback / a deep engine
// call site can't thread a handle, so a process-global is the simplest hop. The
// set is small (only paths mid-delete) and self-draining (entries are consumed on
// the matching Delete event, or swept by [`prune_stale_engine_suppressions`]).
static ENGINE_DELETE_SUPPRESS: OnceLock<Mutex<HashMap<PathBuf, Instant>>> = OnceLock::new();

fn engine_delete_suppress() -> &'static Mutex<HashMap<PathBuf, Instant>> {
    ENGINE_DELETE_SUPPRESS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Register `path` as an ENGINE-ORIGINATED delete so the watcher drops the
/// `NOTIFY_DELETE_COMPLETION` it will fire instead of queuing a redundant server
/// trash. Call this IMMEDIATELY BEFORE `windows_cf::delete_placeholder` on the
/// remote-deletion reconcile path (task 0806). Idempotent; refreshes the timer.
pub fn suppress_engine_delete(path: &Path) {
    if let Ok(mut set) = engine_delete_suppress().lock() {
        set.insert(path.to_path_buf(), Instant::now());
    }
}

/// If `path` was registered by [`suppress_engine_delete`], CONSUME the entry and
/// return `true` (the delete is engine-originated → the watcher must NOT queue a
/// server trash). Returns `false` for a genuine user delete. Consuming on read
/// keeps the set self-draining so a later user delete of the same path is honoured.
fn take_engine_delete_suppressed(path: &Path) -> bool {
    match engine_delete_suppress().lock() {
        Ok(mut set) => set.remove(path).is_some(),
        Err(_) => false,
    }
}

/// Drop suppression entries older than this. A registered engine delete whose
/// NOTIFY never arrives (revert/remove failed, or the OS coalesced the event)
/// must not linger and swallow a genuine LATER user delete of the same path.
const ENGINE_SUPPRESS_TTL: Duration = Duration::from_secs(30);

/// Evict suppression entries older than [`ENGINE_SUPPRESS_TTL`]. Called from the
/// debounce loop's periodic tick so the set can never grow unbounded or shadow a
/// real user delete indefinitely.
fn prune_stale_engine_suppressions() {
    if let Ok(mut set) = engine_delete_suppress().lock() {
        let now = Instant::now();
        set.retain(|_, registered| now.duration_since(*registered) < ENGINE_SUPPRESS_TTL);
    }
}

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

/// How often the enumeration scan walks the whole sync root looking for new
/// local files (the CREATE trigger — see the module docs on why CF NOTIFY
/// callbacks don't fire for brand-new plain files). 8s is the sweet spot: short
/// enough that a dropped file uploads within roughly one interval, long enough
/// that the recursive `read_dir` walk of a (small) vault is negligible overhead.
/// Each scan is awaited to completion before the next is scheduled (the loop
/// ticks AFTER the previous walk returns), so scans can never overlap or pile up
/// even if a walk ever runs long.
const SCAN_INTERVAL: Duration = Duration::from_secs(8);

/// Hard cap on directory recursion depth, so a pathological deep tree (or a
/// symlink cycle that slipped past the symlink skip) can never make a scan run
/// unbounded. The vault nests a handful of levels in practice; 64 is far beyond
/// any real layout while still finite.
const MAX_SCAN_DEPTH: usize = 64;

/// Max NEW file uploads a single scan pass may enqueue (task 0811 throttle).
///
/// A first enable of a large known folder (e.g. Music, 5,344 files) would
/// otherwise enqueue thousands of `upload_version` ops in ONE pass, flooding the
/// queue and the upload workers and pinning the machine — exactly the overload
/// the founder hit. Bounding new enqueues per pass spreads the work across scans
/// (one pass every `SCAN_INTERVAL`), so backup ramps up gradually instead of
/// thundering. The on-disk mirror already caps its copy volume per pass; this is
/// the matching cap on the upload side. Files not enqueued this pass are simply
/// picked up by the next one (the walk re-sees them — nothing is lost). Folder
/// scaffolding is NOT counted against this cap: folders are cheap metadata ops
/// and must keep pace with their files so parents always exist first.
const MAX_NEW_UPLOADS_PER_SCAN: usize = 256;

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

/// Owned handle to the running upload driver. Dropping it signals BOTH the
/// debounce task and the enumeration scan task to exit; once the debounce task
/// exits the registered NOTIFY sender is closed, so the Cloud Files callbacks
/// find no receiver and drop events harmlessly.
pub struct WatcherHandle {
    // Dropping these senders closes their oneshot channels, which ends the
    // debounce loop and the scan loop respectively.
    _shutdown_debounce: tokio::sync::oneshot::Sender<()>,
    _shutdown_scan: tokio::sync::oneshot::Sender<()>,
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
    let (shutdown_debounce_tx, shutdown_debounce_rx) = tokio::sync::oneshot::channel::<()>();
    let (shutdown_scan_tx, shutdown_scan_rx) = tokio::sync::oneshot::channel::<()>();

    // Hand the sender to the Cloud Files callback layer. The `extern "system"`
    // NOTIFY callbacks can't capture state, so they reach this sender through a
    // OnceLock (set here). Windows-only; a no-op on other platforms.
    #[cfg(target_os = "windows")]
    crate::windows_cf::set_notify_sender(event_tx.clone());

    // Keep `event_tx` alive for the lifetime of the loop on every platform so
    // the channel doesn't close immediately on non-Windows builds (where the
    // sender is never registered). The debounce loop owns the receiver.
    let _keep_tx = event_tx;

    tracing::info!(
        sync_root = %sync_root.display(),
        scan_interval_secs = SCAN_INTERVAL.as_secs(),
        "sync-root upload driver started (CF NOTIFY dispatch + enumeration scan)"
    );

    tokio::spawn(debounce_loop(
        bridge.clone(),
        sync_root.clone(),
        event_rx,
        shutdown_debounce_rx,
        _keep_tx,
    ));
    tokio::spawn(scan_loop(bridge, sync_root, shutdown_scan_rx));

    Some(WatcherHandle {
        _shutdown_debounce: shutdown_debounce_tx,
        _shutdown_scan: shutdown_scan_tx,
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
    let mut pending_deletes: HashMap<PathBuf, Instant> = HashMap::new();
    let mut pending_renames: HashMap<PathBuf, (PathBuf, Instant)> = HashMap::new();
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
                        // task 0806: if the ENGINE just removed this placeholder to
                        // reconcile a REMOTE deletion, this NOTIFY is an echo of OUR
                        // own delete — drop it (consuming the suppression entry) so we
                        // never queue a redundant server trash. A genuine user delete
                        // has no entry and falls through to handle_delete as before.
                        if take_engine_delete_suppressed(&path) {
                            tracing::debug!("upload driver: dropping engine-originated delete (remote-deletion reconcile)");
                        } else {
                            pending_deletes.insert(path, Instant::now());
                        }
                    }
                    Some(NotifyEvent::Rename { source, target }) => {
                        // A rename invalidates a pending close-completion for the
                        // OLD path; the NEW path's close (if any) will arrive on
                        // its own event.
                        pending.remove(&source);
                        pending_renames.insert(source, (target, Instant::now()));
                    }
                    // Sender dropped (handle gone) — exit.
                    None => break,
                }
            }
            _ = tick.tick() => {
                // task 0806: sweep stale engine-delete suppressions so a registered
                // delete whose NOTIFY never arrived can't shadow a later user delete.
                prune_stale_engine_suppressions();
                let now = Instant::now();
                let deletes: Vec<_> = pending_deletes.iter().filter(|(_, t)| now.duration_since(**t) >= DEBOUNCE)
                    .map(|(p, _)| p.clone()).collect();
                for path in deletes {
                    pending_deletes.remove(&path);
                    handle_delete(&bridge, &sync_root, &path).await;
                }
                let renames: Vec<_> = pending_renames.iter().filter(|(_, (_, t))| now.duration_since(*t) >= DEBOUNCE)
                    .map(|(p, (target, _))| (p.clone(), target.clone())).collect();
                for (source, target) in renames {
                    pending_renames.remove(&source);
                    handle_rename(&bridge, &sync_root, &source, &target).await;
                }
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

/// The CREATE trigger. Every [`SCAN_INTERVAL`] (debounced by awaiting each walk
/// before scheduling the next), recursively walk `sync_root`, run the shared
/// [`EngineBridge::classify_local_path`] on every regular file, and upload the
/// survivors (genuinely-new plain files). This is what catches a file a foreign
/// process drops into the sync root — the case the CF NOTIFY callbacks miss
/// entirely (see module docs).
///
/// Why a scan and not just the callbacks: reading the disk always works; it does
/// not depend on any filter callback firing. The cost is a recursive `read_dir`
/// of a small vault every 8s, which is negligible — and the heavy lifting (the
/// walk + classify + the synchronous `queue_finder_create`, which touches the
/// SQLite DB and stages bytes) runs inside [`tokio::task::spawn_blocking`] so it
/// never stalls the async runtime.
///
/// Dedupe: `classify_local_path`'s filter-3 already rejects any path with a DB
/// row, and `queue_finder_create` writes that row synchronously before
/// returning — so a file queued by scan N is rejected by scan N+1. The
/// `in_flight` set is a belt-and-braces second guard covering the in-process
/// window between dispatch and the row being observable, and it stops repeated
/// `convert_to_unsynced_placeholder` calls on a path already handed off. A path
/// that disappears from disk is pruned from the set so a later same-named file
/// can still be picked up.
async fn scan_loop(
    bridge: Arc<EngineBridge>,
    sync_root: PathBuf,
    mut shutdown_rx: tokio::sync::oneshot::Receiver<()>,
) {
    // Paths this engine lifetime has already dispatched for upload. Lives across
    // ticks so a file mid-upload isn't re-dispatched in the window before its
    // `Uploading` DB row is observable to filter-3.
    let mut in_flight: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();

    loop {
        // Wait one interval OR a shutdown signal. We sleep FIRST so the very
        // first scan happens one interval after startup — by then the initial
        // `seed_placeholders` + first sync tick have run, so the DB rows that
        // filter-3 needs to suppress just-downloaded files are already present.
        tokio::select! {
            biased;
            _ = &mut shutdown_rx => {
                tracing::debug!("enumeration scan loop shutting down");
                break;
            }
            _ = tokio::time::sleep(SCAN_INTERVAL) => {}
        }

        // Run the blocking walk + classify + queue off the async runtime. We
        // move the in-flight set in and get it back so it persists across ticks
        // without a lock. `bridge`/`sync_root` are cheap to clone (Arc + PathBuf).
        let bridge_for_scan = bridge.clone();
        let root_for_scan = sync_root.clone();
        let taken = std::mem::take(&mut in_flight);
        match tokio::task::spawn_blocking(move || {
            let mut set = taken;
            run_one_scan(&bridge_for_scan, &root_for_scan, &mut set);
            set
        })
        .await
        {
            Ok(set) => in_flight = set,
            Err(e) => {
                // The blocking task panicked — log and continue with an empty
                // set (filter-3 + filter-2 still prevent double uploads; the set
                // is only an optimisation). Never let one bad scan kill the loop.
                tracing::warn!(error = %e, "enumeration scan task panicked; continuing");
                in_flight = std::collections::HashSet::new();
            }
        }
    }
}

/// One pass of the enumeration scan: walk the tree under `sync_root`, dispatch
/// every genuinely-new file for upload, and keep `in_flight` in sync with disk
/// reality (add dispatched paths, prune vanished ones). Synchronous — called
/// inside [`tokio::task::spawn_blocking`] from [`scan_loop`].
fn run_one_scan(bridge: &EngineBridge, sync_root: &std::path::Path, in_flight: &mut std::collections::HashSet<PathBuf>) {
    let mut seen_on_disk: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    let mut dispatched = 0usize;
    tracing::debug!(
        sync_root = %sync_root.display(),
        in_flight = in_flight.len(),
        "enumeration scan: starting pass"
    );

    // Live known-folder-backup gate (task 0811 review fix for the disable race).
    // The mirror copies files onto disk, but it can be racing a DISABLE: a file
    // mirrored while enabled is still on disk after the user disables that folder
    // and the disable purges the queue. Without a gate the very next scan would
    // re-classify those orphaned vault files as NEW (no DB row) and re-enqueue the
    // whole backlog — uploads outliving the disable. We resolve the live enabled
    // set ONCE per pass (cheap config read) and skip dispatch/scaffold for any
    // backup-originated path whose folder is no longer enabled. A `None` config
    // (read failure) is treated as "nothing disabled" — fail-open keeps a normal
    // user's uploads flowing; the purge already removed the in-flight backlog.
    let enabled_backup_keys: Option<std::collections::HashSet<String>> =
        crate::config::DesktopConfig::load()
            .ok()
            .map(|cfg| cfg.known_folder_backup.iter().cloned().collect());

    // `true` if `rel`-derived path belongs to a backup folder that is currently
    // DISABLED (so its mirrored-but-orphaned files must not be (re)enqueued).
    let backup_disabled = |path: &std::path::Path| -> bool {
        let Some(keys) = enabled_backup_keys.as_ref() else {
            return false; // config unreadable → fail-open
        };
        match crate::engine_bridge::relative_db_path(sync_root, path)
            .as_deref()
            .and_then(crate::known_folder::backup_source_key_for_this_device)
        {
            Some(key) => !keys.contains(&key),
            None => false, // not a backup path → never gated
        }
    };

    walk_dir(
        sync_root,
        sync_root,
        0,
        // on_dir (PRE-ORDER, parents before children): scaffold a server vault
        // folder for every directory that has no row yet, so a nested file
        // dispatched just after resolves THIS folder as its parent and the server
        // vault mirrors the on-disk `Backup/<device>/<folder>/<sub>` hierarchy
        // (task 0811 — without this, nested files uploaded FLAT to the root).
        // Idempotent: `ensure_local_folder` returns the existing id on later scans.
        &mut |dir_path| {
            // Don't scaffold (enqueue CreateFolder for) a disabled backup folder's
            // subtree — same disable-race gate as files.
            if backup_disabled(dir_path) {
                tracing::debug!(
                    path = %dir_path.display(),
                    "enumeration scan: skipped disabled backup directory"
                );
                return;
            }
            bridge.ensure_local_folder(sync_root, dir_path);
        },
        &mut |file_path| {
            tracing::debug!(
                path = %file_path.display(),
                "enumeration scan: found file"
            );
            seen_on_disk.insert(file_path.to_path_buf());

            // Already dispatched this lifetime and not yet reflected as a DB row?
            // Skip the redundant classify/convert. classify_local_path would also
            // reject it once the row lands, but this avoids the work + repeated
            // CfConvertToPlaceholder churn in the meantime.
            if in_flight.contains(file_path) && bridge.db().get_file_by_path(
                &crate::engine_bridge::relative_db_path(sync_root, file_path).unwrap_or_default()
            ).ok().flatten().is_none() {
                tracing::debug!(
                    path = %file_path.display(),
                    "enumeration scan: skipped in-flight file"
                );
                return;
            }

            // Disable-race gate (task 0811 review fix): a file the mirror copied
            // while its folder was enabled is still on disk after a disable+purge.
            // Skip it so the purge is final — the persisted config flip means it
            // never re-enqueues, even across restarts. (When the user re-enables,
            // the key returns to the set and the next scan picks it back up.)
            if backup_disabled(file_path) {
                tracing::debug!(
                    path = %file_path.display(),
                    "enumeration scan: skipped disabled backup file"
                );
                return;
            }

            // Throttle (task 0811): cap NEW enqueues per pass so a first enable of
            // a huge folder ramps up gradually instead of flooding the queue. Keep
            // walking (so `seen_on_disk` stays complete for the prune below and
            // parent folders keep getting scaffolded) but stop dispatching files —
            // the next pass picks up the remainder.
            if dispatched >= MAX_NEW_UPLOADS_PER_SCAN {
                tracing::debug!(
                    path = %file_path.display(),
                    max_new_uploads = MAX_NEW_UPLOADS_PER_SCAN,
                    "enumeration scan: skipped file due to per-scan upload cap"
                );
                return;
            }

            if dispatch_local_create(bridge, sync_root, file_path, "scan") {
                in_flight.insert(file_path.to_path_buf());
                dispatched += 1;
            } else {
                tracing::debug!(
                    path = %file_path.display(),
                    "enumeration scan: file was not queued"
                );
            }
        },
    );

    // Prune in-flight entries whose files are gone (deleted, renamed, or the
    // upload finished and the placeholder no longer matches a plain file we'd
    // re-dispatch). We only retain entries we actually saw this walk; anything
    // else is no longer a path we need to suppress. This keeps the set bounded
    // and lets a NEW file at a previously-used path be picked up later.
    in_flight.retain(|p| seen_on_disk.contains(p));

    if dispatched > 0 {
        tracing::debug!(dispatched, "enumeration scan queued new local files");
    }
}

/// Depth-first recursive walk of `dir`, invoking `on_file` for every regular
/// FILE entry. Skips engine-internal paths (`.beebeeb/`, the lock file) via the
/// shared [`crate::engine_bridge::path_is_engine_internal`] guard, never follows
/// symlinks, and is bounded by [`MAX_SCAN_DEPTH`].
///
/// Errors (an unreadable directory, a racing delete) are logged at trace and
/// skipped — a scan must never abort the whole walk for one bad entry; the next
/// scan retries. Pure `std::fs`, so it works on every platform (the scan task is
/// only spawned on Windows, but keeping the walk platform-agnostic means it
/// type-checks and unit-tests everywhere).
fn walk_dir(
    sync_root: &std::path::Path,
    dir: &std::path::Path,
    depth: usize,
    on_dir: &mut dyn FnMut(&std::path::Path),
    on_file: &mut dyn FnMut(&std::path::Path),
) {
    if depth > MAX_SCAN_DEPTH {
        tracing::warn!(depth, "enumeration scan hit max depth; not descending further");
        return;
    }

    // Skip the whole engine-internal subtree up front (`.beebeeb/` etc.) so we
    // never even read its children — cheaper and impossible to misclassify.
    if crate::engine_bridge::path_is_engine_internal(sync_root, dir) {
        tracing::debug!(
            path = %dir.display(),
            "enumeration scan: skipped engine-internal directory"
        );
        return;
    }

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            tracing::debug!(
                path = %dir.display(),
                error = %e,
                "enumeration scan: read_dir failed; skipping directory"
            );
            return;
        }
    };

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                tracing::debug!(
                    path = %dir.display(),
                    error = %e,
                    "enumeration scan: dir entry error; skipping"
                );
                continue;
            }
        };
        let path = entry.path();

        // Engine-internal guard per entry too (the lock file at the root, a
        // nested `.beebeeb`). Cheap and defensive.
        if crate::engine_bridge::path_is_engine_internal(sync_root, &path) {
            tracing::debug!(
                path = %path.display(),
                "enumeration scan: skipped engine-internal entry"
            );
            continue;
        }

        // Use symlink_metadata so a symlink is classified as a symlink (we skip
        // it) rather than followed — no symlink-cycle risk, and we never upload
        // through a link to outside the vault.
        let meta = match std::fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(e) => {
                tracing::debug!(
                    path = %path.display(),
                    error = %e,
                    "enumeration scan: stat failed; skipping entry"
                );
                continue;
            }
        };
        let ft = meta.file_type();
        if ft.is_symlink() {
            tracing::debug!(
                path = %path.display(),
                "enumeration scan: skipped symlink"
            );
            continue;
        }
        if ft.is_dir() {
            // PRE-ORDER: ensure this directory's vault folder exists BEFORE we
            // descend, so a nested child resolves its parent (task 0811).
            on_dir(&path);
            walk_dir(sync_root, &path, depth + 1, on_dir, on_file);
        } else if ft.is_file() {
            on_file(&path);
        } else {
            tracing::debug!(
                path = %path.display(),
                "enumeration scan: skipped non-file entry"
            );
        }
        // Other entry kinds (sockets, devices) are ignored.
    }
}

/// Classify a settled create/modify path through the one shared, parent-aware
/// [`EngineBridge::classify_local_path`] and, if it is a genuinely-new user
/// file, enqueue its encrypted upload. Thin wrapper over [`dispatch_local_create`]
/// — the CF NOTIFY close-completion path and the enumeration scan share the same
/// convert→queue sequence.
async fn handle_settled_path(bridge: &EngineBridge, sync_root: &std::path::Path, path: &std::path::Path) {
    let _ = dispatch_local_create(bridge, sync_root, path, "notify");
}

/// Classify `path` and, if it survives the feedback filters (a genuinely-new
/// user file), convert it to an unsynced placeholder and enqueue its encrypted
/// upload. The ONE place the convert→queue sequence lives, shared by both the
/// CF NOTIFY close-completion path ([`handle_settled_path`]) and the periodic
/// enumeration scan ([`scan_loop`]).
///
/// All feedback-loop filtering + `parent_id` resolution lives in
/// [`EngineBridge::classify_local_path`] — this function only does the
/// placeholder convert + the queue, never re-implements a filter.
///
/// Returns `true` iff an upload was actually queued (a survivor). The scan loop
/// uses that to record the path in its in-flight set so it is not re-dispatched
/// before the `Uploading` DB row makes filter-3 reject it. `source` is a static
/// tag (`"notify"` / `"scan"`) used only for log attribution.
fn dispatch_local_create(
    bridge: &EngineBridge,
    sync_root: &std::path::Path,
    path: &std::path::Path,
    source: &'static str,
) -> bool {
    match bridge.queue_windows_tracked_edit(sync_root, path) {
        Ok(Some(queued)) => return queued,
        Ok(None) => {},
        Err(error) => {
            tracing::warn!(%error, "tracked Windows edit retained for retry");
            return false;
        }
    }
    let Some(target) = bridge.classify_local_path(sync_root, path) else {
        return false;
    };

    // Capture size for the log before `target` is moved into the queue call.
    let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let nested = target.parent_id.is_some();

    // `queue_finder_create` stages a copy of the bytes, derives the per-file key
    // inside the transfer loop via beebeeb-core, and encrypts on upload — we
    // reuse it wholesale and never touch crypto here. It also upserts an
    // `Uploading` DB row synchronously before returning, so the very next
    // `classify_local_path` (filter 3) rejects this path → no double upload.
    match bridge.queue_finder_create(target) {
        Ok(FinderWriteOutcome::Queued { op_id, .. }) => {
            // Windows: queue/stage FIRST, then hand the local file to Cloud Files
            // as an UNSYNCED placeholder. If staging ever fails, the file stays a
            // plain local file and a later scan can retry. Converting before the
            // row exists can poison that retry path: the next scan sees a
            // placeholder with no DB row and `classify_local_path` correctly
            // rejects it as engine-owned.
            #[cfg(target_os = "windows")]
            {
                let local_id = uuid::Uuid::new_v4().to_string();
                if let Err(e) = crate::windows_cf::placeholders::convert_to_unsynced_placeholder(path, &local_id) {
                    tracing::warn!(
                        error = %e,
                        op_id = %op_id,
                        source,
                        "upload driver: could not post-queue convert new file to unsynced placeholder"
                    );
                }
            }
            // Zero-knowledge: log the op id, never the decrypted filename.
            tracing::info!(op_id = %op_id, size, nested, source, "upload driver: queued new local file for encrypted upload");
            true
        }
        Ok(FinderWriteOutcome::Ignored { .. }) => false, // temp/ignored name — fine
        Err(e) => {
            tracing::warn!(error = %e, source, "upload driver: failed to queue local file upload");
            false
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
    // Atomic replace can deliver DELETE for the old placeholder after the new
    // bytes already occupy its name. Keep the destination's stable identity.
    if path.is_file() {
        handle_settled_path(bridge, sync_root, path).await;
        return;
    }
    match bridge.db().get_file_by_path(&rel) {
        Ok(Some(entry)) => match bridge.queue_finder_delete(&entry.file_id, None) {
            Ok(FinderWriteOutcome::Queued { op_id, .. }) => {
                // CRITICAL (task 0802): mark the row `Trashing` so the Windows
                // placeholder seeder (`populate_placeholders`, which only mints
                // for `CloudOnly`) does NOT re-create the on-disk placeholder
                // the user just deleted before the queued TrashFile op
                // round-trips. The TrashFile op deletes this row on success; a
                // permanent trash failure leaves it `Trashing` (recoverable —
                // the file still exists on the server) rather than reappearing.
                if let Err(e) = bridge.db().set_status(&entry.file_id, crate::state_db::FileStatus::Trashing) {
                    tracing::warn!(error = %e, "upload driver: could not mark row trashing after local delete");
                }
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

    if let Some(target_rel) = crate::engine_bridge::relative_db_path(sync_root, target) {
        if bridge.db().get_file_by_path(&target_rel).ok().flatten().is_some() {
            handle_settled_path(bridge, sync_root, target).await;
            return;
        }
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

    // Editors can rename the old destination to a backup, then put new bytes
    // at the original name. After settling, that is a content save, not a move.
    if source.is_file() {
        handle_settled_path(bridge, sync_root, source).await;
        return;
    }

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
        // A metadata-only move/rename: the server path is derived from
        // parent_id + name, and the local row's path is re-keyed by the next
        // sync_tick. No new-file path key to carry here.
        rel_path: None,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api_client::ApiClient;
    use crate::state_db::{FileStatus, OperationKind, StateDb};
    use std::collections::HashSet;
    use std::fs;
    use std::sync::Arc;

    // Task 1640: these tests also run, unchanged, against origin/main.
    fn tracked_edit_fixture(root: &Path, db: &StateDb) -> PathBuf {
        use crate::state_db::{FileEntry, ItemKind};
        use sha2::{Digest, Sha256};
        let path = root.join("tracked.txt");
        fs::write(&path, b"server bytes").unwrap();
        db.upsert_file(&FileEntry {
            file_id: "tracked-id".into(), path: "tracked.txt".into(),
            status: FileStatus::Local, size_bytes: 12, modified_at: 1,
            content_hash: None, remote_updated_at: 1, parent_id: None,
            item_kind: ItemKind::File,
        }).unwrap();
        let mut contract = db.get_file_contract_state("tracked-id").unwrap().unwrap();
        contract.current_version = 7;
        contract.local_base_version = 7;
        contract.local_hash = Some(format!("{:x}", Sha256::digest(b"server bytes")));
        db.set_file_contract_state(&contract).unwrap();
        path
    }

    #[tokio::test]
    async fn regression_1640_tracked_edit_is_durable_version() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("root");
        fs::create_dir(&root).unwrap();
        let db_path = temp.path().join("state.db");
        let (db, bridge) = test_bridge(&db_path);
        let path = tracked_edit_fixture(&root, &db);
        fs::write(&path, b"edited bytes").unwrap(); // same size, no size-only heuristic
        handle_settled_path(&bridge, &root, &path).await;
        let queued = db.list_review_operations().unwrap();
        assert_eq!(queued.len(), 1, "tracked edit must queue exactly one version");
        assert_eq!(queued[0].kind, OperationKind::UploadVersion);
        assert_eq!(queued[0].file_id.as_deref(), Some("tracked-id"));
        assert_eq!(queued[0].base_version, Some(7));
        assert_eq!(fs::read(queued[0].payload_path.as_ref().unwrap()).unwrap(), b"edited bytes");
        handle_settled_path(&bridge, &root, &path).await;
        assert_eq!(db.list_review_operations().unwrap().len(), 1, "duplicate close must not upload twice");
        drop(bridge); drop(db);
        let reopened = StateDb::open(&db_path).unwrap();
        let resumed = reopened.list_due_operations(i64::MAX).unwrap();
        assert_eq!(resumed.len(), 1);
        assert_eq!(fs::read(resumed[0].payload_path.as_ref().unwrap()).unwrap(), b"edited bytes");
    }

    #[tokio::test]
    async fn regression_1640_atomic_replace_preserves_identity() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("root"); fs::create_dir(&root).unwrap();
        let (db, bridge) = test_bridge(&temp.path().join("state.db"));
        let path = tracked_edit_fixture(&root, &db);
        let source = root.join("save.tmp");
        fs::write(&source, b"atomic edit").unwrap();
        fs::rename(&source, &path).unwrap();
        handle_rename(&bridge, &root, &source, &path).await;
        let ops = db.list_review_operations().unwrap();
        assert_eq!(ops.len(), 1, "atomic replacement must queue a version");
        assert_eq!(ops[0].file_id.as_deref(), Some("tracked-id"));
        assert_eq!(ops[0].base_version, Some(7));
        assert_eq!(fs::read(ops[0].payload_path.as_ref().unwrap()).unwrap(), b"atomic edit");
    }

    #[test]
    fn regression_1640_dehydrate_never_selects_queued_bytes() {
        use crate::state_db::PendingOperation;
        let temp = tempfile::tempdir().unwrap();
        let (db, _) = test_bridge(&temp.path().join("state.db"));
        tracked_edit_fixture(temp.path(), &db);
        db.enqueue_operation(&PendingOperation {
            op_id: "offline-edit".into(), kind: OperationKind::UploadVersion,
            file_id: Some("tracked-id".into()), parent_id: None,
            target_path: Some("tracked.txt".into()), metadata_json: None,
            payload_path: Some("durable-payload".into()), base_version: Some(7),
            base_object_version_id: None, attempts: 1, max_attempts: 25,
            next_retry_at: i64::MAX, last_error: Some("offline".into()),
            backup_source_key: None, created_at: 1, updated_at: 1,
        }).unwrap();
        // A stale Local row must never override the durable queue's dirty state.
        assert_eq!(db.unpinned_local_files_for_dehydration().unwrap().len(), 0,
            "dehydrate must exclude queued bytes even when status is Local");
        assert_eq!(db.mark_cloud_only_after_dehydrate(&["tracked-id".into()], 2).unwrap(), 0);
        assert_eq!(db.get_file("tracked-id").unwrap().unwrap().status, FileStatus::Local);
        db.mark_cached("tracked-id", "cached-payload", 12, 1).unwrap();
        assert_eq!(db.disposable_unpinned_cache_paths().unwrap().len(), 0);
        assert_eq!(db.evict_unpinned_cache_until_under(0, 2).unwrap().len(), 0);
        assert_eq!(db.get_file("tracked-id").unwrap().unwrap().status, FileStatus::Local);
    }

    #[tokio::test]
    async fn regression_1640_read_close_and_hydration_do_not_upload() {
        let temp = tempfile::tempdir().unwrap();
        let (db, bridge) = test_bridge(&temp.path().join("state.db"));
        let path = tracked_edit_fixture(temp.path(), &db);
        handle_settled_path(&bridge, temp.path(), &path).await;
        handle_settled_path(&bridge, temp.path(), &path).await;
        assert_eq!(db.list_review_operations().unwrap().len(), 0);
        assert!(crate::windows_edits::content_is_confirmed(&db, "tracked-id",
            &crate::windows_edits::hash_file(&path).unwrap()).unwrap());
        fs::write(&path, b"user writes!").unwrap();
        // Immediate free-up-space, before any debounce or queue work.
        assert!(!crate::windows_edits::content_is_confirmed(&db, "tracked-id",
            &crate::windows_edits::hash_file(&path).unwrap()).unwrap());
    }

    #[test]
    fn regression_1640_scan_preserves_every_settled_save_and_conflict() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("root"); fs::create_dir(&root).unwrap();
        let db_path = temp.path().join("state.db");
        let (db, bridge) = test_bridge(&db_path);
        let path = tracked_edit_fixture(&root, &db);
        let mut flight = HashSet::new();
        fs::write(&path, b"first edit").unwrap();
        run_one_scan(&bridge, &root, &mut flight);
        fs::write(&path, b"second edit").unwrap();
        run_one_scan(&bridge, &root, &mut flight);
        run_one_scan(&bridge, &root, &mut flight);
        let mut ops = db.list_review_operations().unwrap();
        ops.sort_by_key(|op| op.base_version);
        assert_eq!(ops.len(), 2);
        assert_eq!(ops.iter().map(|op| op.base_version).collect::<Vec<_>>(), vec![Some(7), Some(8)]);
        assert_eq!(fs::read(ops[0].payload_path.as_ref().unwrap()).unwrap(), b"first edit");
        assert_eq!(fs::read(ops[1].payload_path.as_ref().unwrap()).unwrap(), b"second edit");
        bridge.preserve_windows_edit_conflict(&ops[0], &root, 10).unwrap();
        bridge.preserve_windows_edit_conflict(&ops[0], &root, 10).unwrap();
        let copies = fs::read_dir(&root).unwrap().filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().contains("(conflict ")).collect::<Vec<_>>();
        assert_eq!(copies.len(), 1);
        assert_eq!(fs::read(copies[0].path()).unwrap(), b"first edit");
        assert_eq!(fs::read(&path).unwrap(), b"second edit");
        assert_eq!(db.get_file("tracked-id").unwrap().unwrap().status, FileStatus::Conflict);
        assert_eq!(db.list_review_operations().unwrap().len(), 2);
        drop(bridge); drop(db);
        let reopened = StateDb::open(&db_path).unwrap();
        assert_eq!(reopened.list_review_operations().unwrap().len(), 2);
        assert_eq!(reopened.unpinned_local_files_for_dehydration().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn regression_1640_atomic_delete_gap_is_not_server_trash() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("root"); fs::create_dir(&root).unwrap();
        let (db, bridge) = test_bridge(&temp.path().join("state.db"));
        let path = tracked_edit_fixture(&root, &db);
        let (tx, rx) = mpsc::unbounded_channel();
        let (stop, stopped) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(debounce_loop(Arc::new(bridge), root.clone(), rx, stopped, tx.clone()));
        fs::remove_file(&path).unwrap();
        tx.send(NotifyEvent::Delete(path.clone())).unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        fs::write(&path, b"atomic replacement after delete").unwrap();
        tx.send(NotifyEvent::CloseCompletion(path)).unwrap();
        tokio::time::sleep(Duration::from_millis(900)).await;
        stop.send(()).unwrap(); task.await.unwrap();
        let ops = db.list_review_operations().unwrap();
        assert_eq!(ops.len(), 1, "settled save must not also queue server trash");
        assert_eq!(ops[0].kind, OperationKind::UploadVersion);
        assert_eq!(ops[0].file_id.as_deref(), Some("tracked-id"));
    }

    #[test]
    fn engine_delete_suppression_is_consumed_once() {
        // task 0806: an engine-originated delete is registered, then the FIRST
        // matching Delete event consumes the entry (returns true → drop it). A
        // SECOND event for the same path is a genuine user delete (false →
        // propagate). Use unique paths so the process-global static can't collide
        // with another test.
        let p = std::path::Path::new("/sync/engine-deleted-0806-unique-a.txt");
        assert!(!take_engine_delete_suppressed(p), "unregistered path is not suppressed");

        suppress_engine_delete(p);
        assert!(take_engine_delete_suppressed(p), "first event after register is suppressed");
        assert!(
            !take_engine_delete_suppressed(p),
            "suppression is consumed once — a later user delete of the same path propagates"
        );
    }

    #[test]
    fn engine_delete_suppression_distinguishes_paths() {
        // Registering one path must not suppress a delete of a DIFFERENT path.
        let registered = std::path::Path::new("/sync/engine-deleted-0806-unique-b.txt");
        let user = std::path::Path::new("/sync/user-deleted-0806-unique-c.txt");
        suppress_engine_delete(registered);
        assert!(
            !take_engine_delete_suppressed(user),
            "a genuine user delete of an unregistered path must NOT be suppressed"
        );
        // The registered entry is still there (untouched by the miss above).
        assert!(take_engine_delete_suppressed(registered));
    }

    #[test]
    fn engine_delete_suppression_prunes_stale_entries() {
        // A registered delete whose NOTIFY never arrives must eventually be swept
        // so it can't shadow a later user delete. Force-insert with an OLD instant
        // and confirm the prune evicts it.
        let p = std::path::Path::new("/sync/engine-deleted-0806-unique-d.txt");
        {
            let mut set = engine_delete_suppress().lock().unwrap();
            set.insert(p.to_path_buf(), Instant::now() - ENGINE_SUPPRESS_TTL - Duration::from_secs(1));
        }
        prune_stale_engine_suppressions();
        assert!(
            !take_engine_delete_suppressed(p),
            "a stale suppression entry must be pruned so a later user delete propagates"
        );
    }

    /// Collect every regular file the enumeration scan's [`walk_dir`] visits,
    /// as paths relative to `root` with '/'-separators, so assertions read
    /// naturally regardless of platform separator.
    fn walked_rel(root: &std::path::Path) -> HashSet<String> {
        let mut found: HashSet<String> = HashSet::new();
        walk_dir(root, root, 0, &mut |_dir| {}, &mut |p| {
            let rel = p.strip_prefix(root).unwrap();
            let joined = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            found.insert(joined);
        });
        found
    }

    #[test]
    fn walk_visits_nested_files_and_skips_engine_internals() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        // User files: one at the root, one nested two levels deep.
        fs::write(root.join("top.txt"), b"hello").unwrap();
        fs::create_dir_all(root.join("docs").join("sub")).unwrap();
        fs::write(root.join("docs").join("a.txt"), b"a").unwrap();
        fs::write(root.join("docs").join("sub").join("b.txt"), b"b").unwrap();

        // Engine-internal: the state dir + lock file must NOT be visited.
        fs::create_dir_all(root.join(".beebeeb")).unwrap();
        fs::write(root.join(".beebeeb").join("state.db"), b"db").unwrap();
        fs::write(root.join(".beebeeb-sync.lock"), b"lock").unwrap();

        let found = walked_rel(root);

        assert!(found.contains("top.txt"), "root-level file should be visited");
        assert!(found.contains("docs/a.txt"), "nested file should be visited");
        assert!(
            found.contains("docs/sub/b.txt"),
            "deeply-nested file should be visited"
        );
        assert!(
            !found.iter().any(|p| p.contains(".beebeeb")),
            "no .beebeeb/* path may be visited, got: {found:?}"
        );
        assert!(
            !found.contains(".beebeeb-sync.lock"),
            "the lock file must never be visited"
        );
        assert_eq!(found.len(), 3, "exactly the three user files, got: {found:?}");
    }

    #[test]
    fn walk_skips_empty_root() {
        let dir = tempfile::tempdir().unwrap();
        let found = walked_rel(dir.path());
        assert!(found.is_empty(), "an empty root yields no files");
    }

    fn test_bridge(db_path: &std::path::Path) -> (Arc<StateDb>, EngineBridge) {
        let db = Arc::new(StateDb::open(db_path).unwrap());
        let api = Arc::new(ApiClient::new(
            "https://api.beebeeb.io".into(),
            "token".into(),
            [17u8; 32],
        ));
        let bridge = EngineBridge::new(db.clone(), api);
        (db, bridge)
    }

    fn assert_uploading_row(db: &StateDb, rel_path: &str, expected_bytes: i64) {
        let row = db
            .get_file_by_path(rel_path)
            .unwrap()
            .unwrap_or_else(|| panic!("expected Uploading state.db row for {rel_path}"));
        assert_eq!(row.path, rel_path);
        assert_eq!(row.status, FileStatus::Uploading);
        assert_eq!(row.size_bytes, expected_bytes);

        let queued = db.list_due_operations(i64::MAX).unwrap();
        assert!(
            queued.iter().any(|op| {
                op.kind == OperationKind::UploadVersion && op.target_path.as_deref() == Some(rel_path)
            }),
            "expected queued upload_version operation for {rel_path}, got: {queued:?}"
        );
    }

    #[test]
    fn scan_queues_direct_explorer_drop_as_uploading_row() {
        let temp = tempfile::tempdir().unwrap();
        let sync_root = temp.path().join("sync-root");
        let state_dir = temp.path().join("state");
        fs::create_dir_all(&sync_root).unwrap();
        fs::create_dir_all(&state_dir).unwrap();
        let (db, bridge) = test_bridge(&state_dir.join("state.db"));

        let file = sync_root.join("direct-write.txt");
        fs::write(&file, b"direct explorer payload").unwrap();

        let mut in_flight = HashSet::new();
        run_one_scan(&bridge, &sync_root, &mut in_flight);

        assert_uploading_row(&db, "direct-write.txt", 23);
    }

    #[test]
    fn scan_queues_atomic_temp_rename_explorer_drop_as_uploading_row() {
        let temp = tempfile::tempdir().unwrap();
        let sync_root = temp.path().join("sync-root");
        let state_dir = temp.path().join("state");
        fs::create_dir_all(&sync_root).unwrap();
        fs::create_dir_all(&state_dir).unwrap();
        let (db, bridge) = test_bridge(&state_dir.join("state.db"));

        let temp_file = sync_root.join("renamed-write.txt.tmp");
        let final_file = sync_root.join("renamed-write.txt");
        fs::write(&temp_file, b"atomic rename explorer payload").unwrap();

        let mut in_flight = HashSet::new();
        run_one_scan(&bridge, &sync_root, &mut in_flight);
        assert!(
            db.get_file_by_path("renamed-write.txt.tmp").unwrap().is_none(),
            "temporary Explorer copy name must not be queued"
        );

        fs::rename(&temp_file, &final_file).unwrap();
        run_one_scan(&bridge, &sync_root, &mut in_flight);

        assert_uploading_row(&db, "renamed-write.txt", 30);
    }

    #[cfg(unix)]
    #[test]
    fn walk_does_not_follow_symlinks() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        // A real file, plus a symlink pointing at an OUTSIDE directory. The walk
        // must not follow the link (no upload-through-link, no cycle risk).
        fs::write(root.join("real.txt"), b"x").unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("secret.txt"), b"s").unwrap();
        symlink(outside.path(), root.join("link")).unwrap();

        let found = walked_rel(root);
        assert!(found.contains("real.txt"));
        assert!(
            !found.iter().any(|p| p.contains("secret") || p.starts_with("link")),
            "must not descend through a symlink, got: {found:?}"
        );
    }
}
