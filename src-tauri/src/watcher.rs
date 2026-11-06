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
//! ## Why a `notify`/ReadDirectoryChanges watcher does NOT work here — and why
//! the CF NOTIFY callbacks alone do NOT cover CREATE
//!
//! The first cut of this module watched the sync root with the cross-platform
//! `notify` crate (ReadDirectoryChanges on Windows). That watcher does **not
//! fire** on a folder that has been handed to the Cloud Files filter via
//! `CfConnectSyncRoot`: the CF filter sits between the filesystem and the
//! ReadDirectoryChanges machinery, so user writes into a connected sync root
//! never surface as `notify` events.
//!
//! The Cloud Files–native `CF_CALLBACK_TYPE_NOTIFY_*` callbacks (registered in
//! [`crate::windows_cf`]) DO fire on a connected root — but, as task 0780 E2E
//! proved, only for I/O on EXISTING CF placeholders. They do **not** fire for a
//! brand-new plain file a foreign process drops into the sync root: across six
//! native writes, zero `NOTIFY_FILE_CLOSE_COMPLETION` callbacks were observed
//! and nothing uploaded. The delete/rename NOTIFY callbacks DO fire (those act
//! on existing synced placeholders) — they stay and work; the gap is purely the
//! CREATE trigger for new local files.
//!
//! A OneDrive-class provider therefore has to run its OWN local change detector
//! for creates. This module provides two halves:
//!
//! 1. **Debounce + dispatch of CF NOTIFY events** — the CF callbacks push events
//!    into a channel; [`debounce_loop`] debounces close-completion bursts and
//!    dispatches deletes/renames immediately, classifying each through the one
//!    shared [`EngineBridge::classify_local_path`]. This half catches MODIFIES of
//!    existing placeholders and is the path delete/rename ride on. It does NOT
//!    catch new plain files (the callbacks never fire for them).
//! 2. **Periodic enumeration scan** ([`scan_loop`]) — the actual CREATE trigger.
//!    Every [`SCAN_INTERVAL`] it recursively walks the sync root (reading the
//!    disk directly always works — it does not depend on any filter callback),
//!    runs the SAME [`EngineBridge::classify_local_path`] on every regular file
//!    not found settled with the same listing and row before ([`ScanMemo`]; all of
//!    them every [`FULL_RECHECK_INTERVAL`]), and for each survivor (a genuinely-new plain file) converts it to an
//!    unsynced placeholder and queues the encrypted upload — the exact pipeline
//!    the (non-firing) NOTIFY create callback would have used.
//!
//! Both halves funnel through [`dispatch_local_create`], so the feedback filters
//! and the convert→queue sequence live in one place. The transfer loop then
//! encrypts + uploads; on success a new local file becomes an in-sync
//! placeholder (see
//! [`crate::engine_bridge::EngineBridge::defer_local_upload_finalization`]).
//!
//! ## Lifecycle
//!
//! [`spawn`] is called from [`crate::runner`]'s `run` right after the engine
//! bridge is built (and, on Windows, after the Cloud Files root is connected),
//! and returns a [`WatcherHandle`]. It registers an [`mpsc`] sender into the
//! [`crate::windows_cf`] callback layer (via [`crate::windows_cf`]'s `set_notify_sender`)
//! and spawns TWO background tasks: the debounce loop (CF NOTIFY dispatch) and
//! the enumeration scan loop (the CREATE trigger). Dropping the handle (on engine
//! shutdown / logout) signals BOTH tasks to exit; the callbacks then find no live
//! receiver and drop events harmlessly, and the scan loop stops walking the root.
//!
//! ## Feedback-loop avoidance (CRITICAL)
//!
//! The engine writes into the sync root constantly: it mints placeholders, it
//! hydrates bytes into them, it writes `.beebeeb/state.db` and the
//! `.beebeeb-sync.lock`. Every one of those can fire a NOTIFY callback AND every
//! one is seen by the periodic scan's `read_dir` walk. If we fed those back into
//! `queue_finder_create`, every download would immediately re-upload — an
//! infinite loop. ALL of that filtering lives in the one shared
//! [`EngineBridge::classify_local_path`] (engine-internal paths, Cloud Files
//! reparse-point placeholders, and the authoritative "already a known server
//! file" DB guard), so the CF-callback path AND the enumeration scan share ONE
//! correct, parent-aware gate. See that method's docs.
//!
//! The scan adds no new feedback risk precisely because it reuses that gate:
//!
//! - A **downloaded / hydrated** file is a reparse-point placeholder (filter 2)
//!   AND has a server DB row (filter 3) → filtered out → never re-uploaded.
//! - A file **mid-upload** has an `Uploading` DB row written synchronously by
//!   `queue_finder_create` BEFORE it returns (filter 3) → filtered out on the
//!   very next scan. It is also a placeholder by then (we convert it pre-queue)
//!   → filter 2 catches it too.
//! - The scan additionally tracks an in-flight set of paths it has dispatched
//!   this engine lifetime, so even within the single tick between dispatch and
//!   the DB row landing it is never queued twice.
//!
//! ## Layout
//!
//! `driver` owns the NOTIFY event intake and the task lifecycle ([`spawn`],
//! [`debounce_loop`]); `scan` the enumeration walk and its memo; `suppression`
//! the engine-delete set; `tuning` the knobs. This file keeps the shared
//! classify→convert→queue sequence ([`dispatch_local_create_verdict`]), its two
//! thin callers, and the delete/rename reconcile, and re-exports the surface
//! `watcher.rs` had before the split. Tests: `watcher/tests/`, split by area.
//!
//! ## Platform note
//!
//! This module is compiled on every platform (so the `mpsc` plumbing is
//! type-checked cross-platform), but [`crate::runner`]'s `run` only *spawns* it on
//! Windows — macOS (File Provider) and Linux (FUSE) already drive uploads
//! through `ipc_socket`. The `allow(dead_code)` below suppresses the
//! never-called warnings on those non-Windows builds.
#![cfg_attr(not(target_os = "windows"), allow(dead_code))]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::mpsc;

use crate::engine_bridge::{EngineBridge, FinderWriteOutcome, LocalPathVerdict};

mod driver;
#[cfg(target_os = "windows")]
pub use driver::{spawn, NotifyEvent};

mod scan;
use scan::*;
pub(crate) use scan::{DiskStamp, ScanMemo};

mod suppression;
use suppression::*;
pub use suppression::{cancel_engine_delete_suppression, suppress_engine_delete};

mod tuning;
use tuning::*;

/// One pass of the enumeration scan: walk the tree under `sync_root` and
/// dispatch every genuinely-new file for upload, and every change of a tracked
/// one (task 1640). Synchronous — called inside [`tokio::task::spawn_blocking`]
/// from [`scan_loop`].
pub(crate) fn run_one_scan(bridge: &EngineBridge, sync_root: &std::path::Path) {
    run_one_scan_with(bridge, sync_root, &mut ScanMemo::default());
}

/// [`run_one_scan`] remembering, in `memo`, the files earlier passes found settled (task 2229).
pub(crate) fn run_one_scan_with(bridge: &EngineBridge, sync_root: &std::path::Path, memo: &mut ScanMemo) {
    // Task 2240: the pass looks up every row by path. With SQLite's default page cache those lookups re-read the
    // state DB from disk on every pass; sized to the DB, the second pass reads nothing.
    if let Err(e) = bridge.db().fit_page_cache() {
        tracing::debug!(error = %e, "enumeration scan: could not size the page cache; scanning with the current one");
    }
    let now = Instant::now();
    if memo
        .since
        .is_none_or(|since| now.duration_since(since) >= FULL_RECHECK_INTERVAL)
    {
        memo.settled.clear();
        memo.since = Some(now);
    }
    memo.classified = 0;
    memo.skipped = 0;
    let mut dispatched = 0usize;
    tracing::debug!(sync_root = %sync_root.display(), "enumeration scan: starting pass");

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
        &mut |file_path, meta| {
            tracing::debug!(
                path = %file_path.display(),
                "enumeration scan: found file"
            );

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

            // Task 2229: a file found settled whose listing and row are both unchanged needs
            // nothing, and skipping it opens no handle. The row stamp is read before
            // classifying, so a write to the row during the pass makes the next pass ask again.
            let disk = DiskStamp::of(meta);
            let row = crate::engine_bridge::relative_db_path(sync_root, file_path)
                .and_then(|rel| bridge.db().file_row_stamp_by_path(&rel).ok().flatten());
            if let (Some(row), Some(&(seen_disk, seen_row))) = (row, memo.settled.get(file_path))
                && seen_disk == disk
                && seen_row == row
            {
                memo.skipped += 1;
                return;
            }

            // Throttle (task 0811): cap NEW enqueues per pass so a first enable of
            // a huge folder ramps up gradually instead of flooding the queue. Keep
            // walking (so parent folders keep getting scaffolded) but stop
            // dispatching files — the next pass picks up the remainder.
            if dispatched >= MAX_NEW_UPLOADS_PER_SCAN {
                tracing::debug!(
                    path = %file_path.display(),
                    max_new_uploads = MAX_NEW_UPLOADS_PER_SCAN,
                    "enumeration scan: skipped file due to per-scan upload cap"
                );
                return;
            }

            // A file queued here is a tracked file from now on (its row is written before
            // the queue call returns): a later pass queues nothing while its upload is
            // queued, and sends a later change of it (task 1640).
            memo.classified += 1;
            match (dispatch_local_create_verdict(bridge, sync_root, file_path, "scan"), row) {
                (ScanDispatch::Settled, Some(row)) => {
                    memo.settled.insert(file_path.to_path_buf(), (disk, row));
                }
                (verdict, _) => {
                    memo.settled.remove(file_path);
                    if verdict == ScanDispatch::Queued {
                        dispatched += 1;
                    } else {
                        tracing::debug!(
                            path = %file_path.display(),
                            "enumeration scan: file was not queued"
                        );
                    }
                }
            }
        },
    );

    if dispatched > 0 {
        tracing::debug!(dispatched, "enumeration scan queued new local files");
    }
    // Task 2244: one line for the edits this pass held, never one per file; each is listed for review.
    // Counted by reason: an edit with no known base needs a look; a file synced before 1640 does not.
    let (content_unknown, base_unknown) = bridge.take_newly_held();
    if content_unknown + base_unknown > 0 {
        tracing::warn!(
            held = content_unknown + base_unknown,
            content_unknown,
            base_unknown,
            "sync folder: changes to synced files are kept on this PC, not sent; each is listed for review"
        );
    }
    tracing::debug!(
        classified = memo.classified,
        skipped = memo.skipped,
        "enumeration scan: pass done"
    );
}

/// Classify a settled create/modify path through the one shared, parent-aware
/// [`EngineBridge::classify_local_path`] and, if it is a genuinely-new user
/// file, enqueue its encrypted upload. Thin wrapper over [`dispatch_local_create`]
/// — the CF NOTIFY close-completion path and the enumeration scan share the same
/// convert→queue sequence.
fn handle_settled_path(bridge: &EngineBridge, sync_root: &std::path::Path, path: &std::path::Path) {
    #[cfg(target_os = "windows")]
    crate::windows_cf::upload_finalization::retry(bridge.db(), sync_root);
    let _ = dispatch_local_create(bridge, sync_root, path, "notify");
}

/// [`dispatch_local_create_verdict`]: `true` iff an upload was queued.
pub(crate) fn dispatch_local_create(
    bridge: &EngineBridge,
    sync_root: &std::path::Path,
    path: &std::path::Path,
    source: &'static str,
) -> bool {
    matches!(
        dispatch_local_create_verdict(bridge, sync_root, path, source),
        ScanDispatch::Queued
    )
}

/// Task 2229: what [`dispatch_local_create_verdict`] did with a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ScanDispatch {
    /// An upload was queued.
    Queued,
    /// A tracked file that needs nothing while its row and its bytes stay as they are.
    Settled,
    /// Nothing queued; ask again next pass.
    NotQueued,
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
/// Returns [`ScanDispatch::Queued`] iff an upload was actually queued (a survivor), and
/// [`ScanDispatch::Settled`] for a tracked file that needs nothing while its row and its bytes
/// stay as they are (task 2229: the scan skips it until either changes). `source` is a static
/// tag (`"notify"` / `"scan"`) used only for log attribution.
pub(crate) fn dispatch_local_create_verdict(
    bridge: &EngineBridge,
    sync_root: &std::path::Path,
    path: &std::path::Path,
    source: &'static str,
) -> ScanDispatch {
    let target = match bridge.classify_local_path_verdict(sync_root, path) {
        LocalPathVerdict::Write(target) => target,
        LocalPathVerdict::Settled => return ScanDispatch::Settled,
        LocalPathVerdict::Skip => return ScanDispatch::NotQueued,
    };

    // Capture size for the log before `target` is moved into the queue call.
    let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let nested = target.parent_id.is_some();
    // Task 1640: a target with a file id is an edit of a synced file (`classify_tracked_edit`):
    // a content modify of that file, sent with its base. Only a new file is created (and, on
    // Windows, converted to an unsynced placeholder below).
    if target.file_id.is_some() {
        return match bridge.queue_finder_modify(target) {
            Ok(FinderWriteOutcome::Queued { op_id, .. }) => {
                tracing::info!(op_id = %op_id, size, nested, source, "upload driver: queued an edit of a synced file for encrypted upload");
                ScanDispatch::Queued
            }
            Ok(FinderWriteOutcome::Ignored { .. }) => ScanDispatch::NotQueued,
            Err(e) => {
                tracing::warn!(error = %e, source, "upload driver: failed to queue an edit of a synced file");
                ScanDispatch::NotQueued
            }
        };
    }

    // `queue_finder_create` stages a copy of the bytes, derives the per-file key
    // inside the transfer loop via beebeeb-core, and encrypts on upload — we
    // reuse it wholesale and never touch crypto here. It also upserts an
    // `Uploading` DB row synchronously before returning, so the very next
    // `classify_local_path` (filter 3) rejects this path → no double upload.
    match bridge.queue_finder_create(target) {
        Ok(FinderWriteOutcome::Queued { op_id, file_id: local_id, .. }) => {
            // Windows: queue/stage FIRST, then hand the local file to Cloud Files
            // as an UNSYNCED placeholder. If staging ever fails, the file stays a
            // plain local file and a later scan can retry. Converting before the
            // row exists can poison that retry path: the next scan sees a
            // placeholder with no DB row and `classify_local_path` correctly
            // rejects it as engine-owned.
            #[cfg(not(target_os = "windows"))]
            let _ = &local_id;
            #[cfg(target_os = "windows")]
            if let Some(local_id) = local_id {
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
            ScanDispatch::Queued
        }
        Ok(FinderWriteOutcome::Ignored { .. }) => ScanDispatch::NotQueued, // temp/ignored name — fine
        Err(e) => {
            tracing::warn!(error = %e, source, "upload driver: failed to queue local file upload");
            ScanDispatch::NotQueued
        }
    }
}

/// A local delete fired. If the path maps to a known server file, enqueue the
/// existing trash op so the server deletes it too. If there is no DB row, the
/// delete was of an untracked local file (or an engine-internal file) — nothing
/// to propagate.
pub(crate) async fn handle_delete(bridge: &EngineBridge, sync_root: &std::path::Path, path: &std::path::Path) {
    handle_delete_with(bridge, sync_root, path, ProtectionCheck::Check).await;
}

/// LD-0113: whether a delete or rename that already happened is checked against the protection rule.
/// The engine's own changes (this process) never are, as the pre-operation callbacks never refuse them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProtectionCheck {
    Check,
    Skip,
}

impl ProtectionCheck {
    pub(crate) fn unless(by_this_process: bool) -> Self {
        if by_this_process { Self::Skip } else { Self::Check }
    }
}

/// [`handle_delete`], with the LD-0113 check as told.
pub(crate) async fn handle_delete_with(
    bridge: &EngineBridge,
    sync_root: &std::path::Path,
    path: &std::path::Path,
    check: ProtectionCheck,
) {
    // Engine-internal deletes (state.db churn, lock file) must never propagate.
    if crate::engine_bridge::path_is_engine_internal(sync_root, path) {
        return;
    }
    let Some(rel) = crate::engine_bridge::relative_db_path(sync_root, path) else {
        return;
    };
    // LD-0113 §6.3 (refused after the fact): the delete of a protected item got through (the app was
    // not running, or the callback could not answer). Nothing is sent; its placeholder comes back.
    if check == ProtectionCheck::Check
        && let Some(words) = protected(bridge, &rel, beebeeb_core::backup::Mutation::Trash)
    {
        restore_protected(bridge, sync_root, &rel, &words);
        return;
    }
    // Task 2204 S0.4: a safe save renamed the original away, put the new bytes at its name and
    // now deletes the renamed file. While that rename is unsent, it folds back: the row returns
    // to the vacated path and the new bytes there are an edit of the original. A file that was
    // only renamed then deleted (nothing at the vacated path), or a vacated path another tracked
    // file holds (a manual swap), is a delete as before.
    if let Ok(Some(entry)) = bridge.db().get_file_by_path(&rel)
        && let Ok(Some(vacated)) = bridge.db().pending_local_rename_vacated(&entry.file_id)
        && let Ok(vacated_on_disk) = crate::engine_bridge::local_file_path_under_sync_root(sync_root, &vacated)
        && vacated_on_disk.is_file()
        // Review-155 P1-B: only onto a row that can take the new bytes as an edit (a synced file
        // with a known base); a file synced before task 1640 (unknown content) or not on this PC
        // keeps the delete, and its new bytes upload as a new file once the rename is sent.
        && entry.status == crate::state_db::FileStatus::Local
        && matches!(
            crate::engine_bridge::sync_folder_content(bridge.db(), &entry.file_id, &vacated_on_disk, None),
            Ok(crate::engine_bridge::SyncFolderContent::Agreed
                | crate::engine_bridge::SyncFolderContent::Changed { base: Some(_) })
        )
        && bridge.db().fold_local_rename(&entry.file_id, &vacated).unwrap_or(false)
    {
        tracing::info!("upload driver: a safe save kept the file (rename folded back)");
        handle_settled_path(bridge, sync_root, &vacated_on_disk);
        return;
    }
    match bridge.db().get_file_by_path(&rel) {
        // The drive rule was applied above (or skipped on purpose for the engine's own change).
        Ok(Some(entry)) => match bridge.queue_finder_delete_decided(&entry.file_id) {
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
/// `queue_finder_modify` metadata path reads a parent that differs from the row's
/// (the root included) as a move → MoveFile, and the row's own parent as a rename
/// → RenameFile (spec 2026-10-09 §7.2), so we hand it the new filename and the new
/// parent, and pass the row's own parent when the new one cannot be resolved.
pub(crate) async fn handle_rename(
    bridge: &EngineBridge,
    sync_root: &std::path::Path,
    source: &std::path::Path,
    target: &std::path::Path,
) {
    handle_rename_with(bridge, sync_root, source, target, ProtectionCheck::Check).await;
}

/// [`handle_rename`], with the LD-0113 check as told.
pub(crate) async fn handle_rename_with(
    bridge: &EngineBridge,
    sync_root: &std::path::Path,
    source: &std::path::Path,
    target: &std::path::Path,
    check: ProtectionCheck,
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
    // LD-0113 §6.3, Amendment 6 (refused after the fact): a rename or move of an item in the Backups
    // tree, or a move of an item into it, that got through is not sent; the item goes back.
    if check == ProtectionCheck::Check
        && let Some(words) = crate::backup_protect::move_refusal(
            bridge.db(),
            &crate::backup_protect::own_backups(),
            &source_rel,
            crate::backup_protect::rename_mutation(source, target),
            crate::engine_bridge::relative_db_path(sync_root, target).as_deref(),
        )
    {
        let moved_back = !source.exists()
            && target.exists()
            && std::fs::rename(target, source)
                .inspect_err(|error| {
                    // Zero-knowledge: the error kind only, never a path.
                    tracing::warn!(kind = ?error.kind(), "protected item: could not move it back");
                })
                .is_ok();
        if moved_back || !target.exists() {
            restore_protected(bridge, sync_root, &source_rel, &words);
            return;
        }
        // The item, bytes and all, stays where it was put and cannot go back: it is sent as made, so the
        // drive and the server agree. Minting it again at its old place would leave this copy untracked,
        // to be uploaded as a new item (self-review, 2026-11-06).
        tracing::warn!("protected item: it could not be put back; the change is sent as made");
    }
    let existing = match bridge.db().get_file_by_path(&source_rel) {
        Ok(Some(entry)) => entry,
        // Source not tracked. The rename may have brought an untracked local
        // file to a new name — classify the TARGET as a possible new upload
        // instead (it will no-op if it is engine-owned / already tracked).
        Ok(None) => {
            handle_settled_path(bridge, sync_root, target);
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
    // right server parent. Spec §7.2 (final review): "the root" and "not found"
    // are different answers. The new parent is the root (`None`) only when the
    // target's directory IS the sync root. A lookup that misses (no folder row yet,
    // a path left stale by a folder rename, a database error) keeps the row's own
    // parent, so the op is a rename and sends no parent; read as the root, the
    // miss would move the file to the top level on every device.
    let resolved_parent = if target.parent() == Some(sync_root) {
        Some(None)
    } else {
        bridge.resolve_parent_id_for(sync_root, target).map(Some)
    };
    let new_parent_id = resolved_parent.clone().unwrap_or_else(|| existing.parent_id.clone());
    // Task 2204 S0.1: a FILE whose new parent resolved, renamed onto a path no other row holds,
    // has its row moved in the transaction that queues the op, so a new file at the old name is
    // a new file. A folder (its descendants would move too), a replace, and an unresolved
    // parent keep the old behaviour: the row is re-keyed by the pull after the op lands.
    let rekey_to = crate::engine_bridge::relative_db_path(sync_root, target).filter(|target_rel| {
        !existing.is_dir()
            && resolved_parent.is_some()
            && matches!(bridge.db().get_file_by_path(target_rel), Ok(None))
    });

    let modify = crate::engine_bridge::FinderWriteTarget {
        file_id: Some(existing.file_id.clone()),
        parent_id: new_parent_id,
        filename: new_name,
        // A metadata-only move/rename: the server path is derived from
        // parent_id + name. The row is re-keyed by `queue_local_rename` (task 2204 S0.1), else
        // by the pull after the op lands. No new-file path key to carry here.
        rel_path: None,
        kind: crate::engine_bridge::FinderWriteItemKind::File,
        // No new bytes — this is a metadata-only move/rename.
        contents_path: None,
        content_type: None,
        base_version_identifier: None,
    };

    let queued = match rekey_to.as_deref() {
        // The drive rule was applied above (skipped for the engine's own change, or sent as made).
        Some(target_rel) => bridge.queue_local_rename_decided(modify, target_rel),
        None => bridge.queue_finder_modify_decided(modify),
    };
    match queued {
        Ok(FinderWriteOutcome::Queued { op_id, .. }) => {
            tracing::info!(op_id = %op_id, "upload driver: queued server rename/move for locally-renamed file");
        }
        Ok(FinderWriteOutcome::Ignored { .. }) => {}
        Err(e) => {
            tracing::warn!(error = %e, "upload driver: failed to queue server rename/move");
        }
    }
}

/// LD-0113: the words `rel` is refused `mutation` with, from the state DB and what this device backs
/// up now (`crate::backup_protect`); `None` when it is not protected.
fn protected(bridge: &EngineBridge, rel: &str, mutation: beebeeb_core::backup::Mutation) -> Option<String> {
    crate::backup_protect::protected_reason(bridge.db(), &crate::backup_protect::own_backups(), rel, mutation)
}

/// LD-0113 §6.3, Amendment 6: a protected item a local change took away comes back from the state DB,
/// with everything under it. Its rows stay; each one whose bytes went with it (a downloaded file, a
/// folder) is cloud-only again, so its placeholder is minted again where it is missing, and nothing is
/// queued for the server. The person sees the words.
fn restore_protected(bridge: &EngineBridge, sync_root: &std::path::Path, rel: &str, words: &str) {
    let flipped = crate::backup_protect::mark_missing_cloud_only(bridge.db(), sync_root, rel);
    #[cfg(target_os = "windows")]
    {
        // One placeholder pass per burst: a folder's events arrive one per item, and each pass walks
        // every cloud-only row. What a skipped event needs, the next pass (or the tick's refresh) mints.
        static LAST_RESTORE: std::sync::Mutex<Option<Instant>> = std::sync::Mutex::new(None);
        let due = LAST_RESTORE.lock().is_ok_and(|mut last| {
            let due = last.is_none_or(|at| at.elapsed() >= Duration::from_secs(2));
            if due {
                *last = Some(Instant::now());
            }
            due
        });
        if due {
            crate::backup_protect::restore_placeholders_soon(bridge.db_arc(), sync_root);
        }
        crate::backup_protect::note_refusal(words);
        // Zero-knowledge: counts, never the item or the words (they carry a path).
        tracing::info!(flipped, pass = due, "protected item: a local change was not sent; its placeholder comes back");
    }
    #[cfg(not(target_os = "windows"))]
    let _ = (words, flipped);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api_client::ApiClient;
    use crate::state_db::{FileStatus, OperationKind, StateDb};
    use std::collections::HashSet;
    use std::fs;
    use std::sync::Arc;

    mod scan;
    mod suppression;
    mod walk;
}
