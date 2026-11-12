//! Free up space: make unpinned local files online-only again. One implementation, two callers: the shell's
//! command and the Windows Explorer flyout (`windows_cf::status_ui`). Must not touch a pinned file, an unsent
//! change, or a path outside the allowed roots.

use std::path::PathBuf;

#[derive(Debug, serde::Serialize)]
pub struct FreeUpSpaceResult {
    pub bytes_freed: u64,
}

/// Allowed roots a cache file may live under for `free_up_space` to delete it.
/// The engine writes hydrated cache copies under the sync root (Windows
/// Cloud Files placeholders) and under the system cache dir
/// (`dirs::cache_dir()/beebeeb`, e.g. the macOS File Provider staging area).
/// Both are trusted, engine-written locations.
pub fn free_up_space_allowed_roots(
    identity: &crate::identity::EngineIdentity,
    sync_root: &std::path::Path,
) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Ok(canon) = sync_root.canonicalize() {
        roots.push(canon);
    } else {
        roots.push(sync_root.to_path_buf());
    }
    if let Some(cache_dir) = dirs::cache_dir() {
        let beebeeb_cache = cache_dir.join(identity.config_dir_name());
        match beebeeb_cache.canonicalize() {
            Ok(canon) => roots.push(canon),
            Err(_) => roots.push(beebeeb_cache),
        }
    }
    // Task 1670 round 2, Codex P1 (PR #75): same reasoning as
    // `disposable_cache_roots` above — a macOS hydrate-cache `cache_path`
    // must be an allowed target for the smart-cache "free up space" eviction
    // path too, not just sign-out's purge.
    #[cfg(target_os = "macos")]
    {
        let hydrate_dir = crate::ipc_socket::macos_hydrate_cache_dir(identity);
        match hydrate_dir.canonicalize() {
            Ok(canon) => roots.push(canon),
            Err(_) => roots.push(hydrate_dir),
        }
    }
    roots
}

#[cfg(target_os = "linux")]
pub fn remove_linux_freedesktop_thumbnails_for_evicted_files(
    db: &crate::state_db::StateDb,
    sync_root: &std::path::Path,
    file_ids: &[String],
) {
    for file_id in file_ids {
        let entry = match db.get_file(file_id) {
            Ok(Some(entry)) if !entry.is_dir() => entry,
            Ok(_) => continue,
            Err(error) => {
                tracing::debug!(file_id = %file_id, error = %error, "linux thumbnail cleanup skipped: state row unavailable");
                continue;
            }
        };
        let Some(source_path) = crate::linux_thumbnail::source_path_under_sync_root(sync_root, &entry.path) else {
            continue;
        };
        if let Err(error) = crate::linux_thumbnail::remove_freedesktop_thumbnails(&source_path) {
            tracing::warn!(file_id = %file_id, error = %error, "linux freedesktop thumbnail cleanup failed");
        }
    }
}

// `pub(crate)` so the Windows Explorer status-flyout command
// (`windows_cf::status_ui::FreeUpSpaceCommand::Invoke`) can reuse the exact same
// reclaim path the `free_up_space` IPC uses — one implementation, two callers
// (the WebView IPC and the shell flyout button). It is a synchronous blocking fn
// (DB + filesystem work), so the COM `Invoke` can call it directly off the
// arbitrary thread Windows delivers the command on, without a tokio runtime.
pub fn free_up_space_blocking(identity: &'static crate::identity::EngineIdentity) -> Result<FreeUpSpaceResult, String> {
    let cfg = crate::config::DesktopConfig::load_for(identity)?;
    let Some(sync_root) = cfg.sync_root.clone() else {
        return Ok(FreeUpSpaceResult { bytes_freed: 0 });
    };
    let db_path = crate::state_paths::beebeeb_state_dir_for(identity)?.join(crate::state_paths::STATE_DB_FILENAME);
    if !db_path.exists() {
        return Ok(FreeUpSpaceResult { bytes_freed: 0 });
    }
    let db = crate::state_db::StateDb::open(&db_path).map_err(|e| format!("open state.db: {e}"))?;

    #[cfg(target_os = "windows")]
    {
        free_up_space_windows(identity, &db, &sync_root)
    }
    #[cfg(not(target_os = "windows"))]
    {
        // Snapshot the on-disk cache paths of every UNPINNED, LOCAL file BEFORE
        // any DB transition (eviction nulls `cache_path`). Pinned files are
        // excluded by the query, so they are always kept.
        let paths = db
            .disposable_unpinned_cache_paths()
            .map_err(|e| format!("list cache paths: {e}"))?;
        free_up_space_unix(identity, &db, &sync_root, paths)
    }
}

/// Windows reclaim path (task 0781). On Cloud Files the hydrated bytes live
/// INSIDE the placeholder's data stream, not in a separate cache copy — so the
/// old Unix-style `remove_file` would either delete the user's whole file or
/// (on a still-present placeholder) no-op while the DB falsely flipped the row
/// to cloud_only with `bytes_freed = 0` (fictional free).
///
/// Here we `CfDehydratePlaceholder` each unpinned file, computing `bytes_freed`
/// from the REAL reclaimed size, and only mark a row cloud_only AFTER its
/// dehydrate succeeds. Pinned files are already excluded from `paths`. The same
/// allowed-roots guard is kept: we only touch placeholders under the sync root
/// (or the engine cache dir), never an arbitrary path from a corrupt DB row.
#[cfg(target_os = "windows")]
pub fn free_up_space_windows(
    identity: &crate::identity::EngineIdentity,
    db: &crate::state_db::StateDb,
    sync_root: &std::path::Path,
) -> Result<FreeUpSpaceResult, String> {
    // Reclaim targets are the UNPINNED, LOCAL files — addressed by their
    // server-relative path, NOT by `cache_path`. On Windows CF the bytes live
    // inside the placeholder in the sync root, and `cache_path` records the
    // already-deleted `%TEMP%` decrypt path from the fetch callback, so it is
    // useless as a dehydration target. We reconstruct the real placeholder path
    // exactly as `windows_cf::populate_placeholders` does.
    let candidates = db
        .unpinned_local_files_for_dehydration()
        .map_err(|e| format!("list dehydration candidates: {e}"))?;

    let allowed_roots = free_up_space_allowed_roots(identity, sync_root);
    let mut bytes_freed: u64 = 0;
    let mut dehydrated_ids: Vec<String> = Vec::new();

    for (file_id, rel_path, _size_bytes) in candidates {
        // Task 2407 (core 2404): never another file this name reaches (an 8.3 short name; `canonicalize` below would
        // follow it to that file's long name). A check that cannot finish keeps the file local.
        let Some(local_path) = crate::name_alias::reclaim_place(sync_root, &rel_path) else {
            continue;
        };
        // Path-safety: only dehydrate placeholders that actually resolve under
        // an allowed engine root (the sync root). A corrupt `path` row can
        // never point us at a file outside the vault.
        let Ok(canonical) = local_path.canonicalize() else {
            // Placeholder missing on disk — nothing to reclaim; leave the row
            // for the next reconcile to re-seed.
            continue;
        };
        if !allowed_roots.iter().any(|root| canonical.starts_with(root)) {
            continue;
        }
        // Pin TOCTOU re-check: `unpinned_local_files_for_dehydration()` filtered
        // on pin state at query time, but the user could have PINNED this file in
        // the window between that query and now. Dehydrating it would evict bytes
        // the user just asked to keep offline. `mark_cloud_only_after_dehydrate`
        // already re-applies the pin predicate (so a newly-pinned file is never
        // flipped to cloud_only — it self-heals on next open), but that runs
        // AFTER the destructive dehydrate. Re-read the row's effective pin state
        // immediately before dehydrating and skip if it is now pinned, so we
        // never reclaim a pinned file's bytes in the first place. A read error
        // here is non-fatal: fall through to the existing dehydrate + the
        // post-dehydrate predicate guard.
        if let Ok(Some(contract)) = db.get_file_contract_state(&file_id) {
            if contract.effective_pin_state() == crate::state_db::PinState::Pinned {
                tracing::debug!(file_id = %file_id, "free_up_space: file pinned after enumeration; keeping local");
                continue;
            }
        }
        // Task 2240 (spec D2, the lead's ruling on self-review #1): confirm before evict. A pin made in Explorer
        // reaches the DB only when the tick's folder listing shows it, and a listing can lag while another process
        // holds the file open, so the DB's "unpinned" may be stale. The file's own attributes decide: pinned on disk
        // (or unreadable) keeps it local, and its OS pin is never cleared below.
        let direct = crate::windows_cf::placeholders::read_os_placeholder_state(&canonical);
        let pinned_on_disk = |attrs: u32| {
            crate::state_db::decode_os_state(attrs, crate::state_db::FileStatus::Local).1
                == Some(crate::state_db::PinState::Pinned)
        };
        if direct.is_none_or(pinned_on_disk) {
            tracing::info!(file_id = %file_id, "free_up_space: the file is pinned on disk (or unreadable); keeping it local");
            continue;
        }
        // OS-level unpin BEFORE dehydrating. This file passed both pin checks
        // above, so it is MEANT to be reclaimed — but its placeholder may still
        // carry a pin it inherits from a pinned folder, which the DB does not
        // reflect (an explicit pin on the file itself was refused just above).
        // `CfDehydratePlaceholder` fails with ERROR_CLOUD_FILE_PINNED on
        // any OS-pinned placeholder, so we clear the OS pin first (non-recursive —
        // this is a single file). Best-effort: a failure here is logged (never the
        // path) and we still attempt the dehydrate, which simply no-ops/errors if
        // the file was genuinely pinned.
        // Task 1640: never free a file whose bytes are a change that is not sent (an edit, a
        // queued upload) or are not known to be the agreed content: they are the only copy.
        if !crate::windows_cf::placeholders::local_bytes_agreed(db, &file_id, &canonical) {
            tracing::warn!(file_id = %file_id, "free_up_space: the file holds a change that is not sent; keeping it local");
            continue;
        }
        if let Err(error) = crate::windows_cf::placeholders::set_pin_state(&canonical, false, false) {
            tracing::warn!(file_id = %file_id, error = %error, "free_up_space: OS unpin before dehydrate failed; attempting dehydrate anyway");
        }
        match crate::windows_cf::placeholders::dehydrate_placeholder(&canonical) {
            Ok(freed) => {
                bytes_freed = bytes_freed.saturating_add(freed);
                // Only NOW is this file genuinely cloud-only on disk — record
                // it so the DB flip below applies ONLY to files we actually
                // dehydrated. (`freed == 0` still means "successfully cloud-only
                // / already dehydrated", a valid end state.)
                dehydrated_ids.push(file_id);
            }
            Err(error) => {
                // Leave the DB row `local` so the next sweep retries; never
                // claim space we did not actually reclaim. Zero-knowledge: log
                // the file_id only.
                tracing::warn!(file_id = %file_id, error = %error, "free_up_space: CfDehydratePlaceholder failed; keeping file local");
            }
        }
    }

    // Mark cloud_only ONLY for the files we actually dehydrated.
    let now = now_unix_seconds();
    if let Err(e) = db.mark_cloud_only_after_dehydrate(&dehydrated_ids, now) {
        tracing::warn!(error = %e, "free_up_space: could not mark dehydrated files cloud_only");
    }

    tracing::info!(bytes_freed, "free_up_space dehydrated unpinned placeholders");
    Ok(FreeUpSpaceResult { bytes_freed })
}

/// Unix reclaim path (macOS File Provider / Linux FUSE). Unchanged from the
/// original: the engine stores hydrated copies as separate cache files, so a
/// `remove_file` genuinely reclaims the bytes. The DB eviction flips rows to
/// cloud_only and the on-disk cache copies are unlinked.
#[cfg(not(target_os = "windows"))]
pub fn free_up_space_unix(
    identity: &crate::identity::EngineIdentity,
    db: &crate::state_db::StateDb,
    sync_root: &std::path::Path,
    paths: Vec<String>,
) -> Result<FreeUpSpaceResult, String> {
    // Flip every unpinned local file to cloud_only in the DB. A budget of 0
    // means "evict everything unpinned."
    let now = now_unix_seconds();
    let evicted_ids = db
        .evict_unpinned_cache_until_under(0, now)
        .map_err(|e| format!("evict unpinned cache: {e}"))?;
    #[cfg(target_os = "linux")]
    remove_linux_freedesktop_thumbnails_for_evicted_files(db, sync_root, &evicted_ids);

    // Delete the actual cache files, bounded to paths under one of the allowed
    // engine cache roots so a bad cache_path can't delete outside the vault.
    // Sum the real file sizes we remove so `bytes_freed` is an exact count, not
    // an estimate. A missing file contributes 0 (already gone — nothing
    // reclaimed by us).
    let allowed_roots = free_up_space_allowed_roots(identity, sync_root);
    let mut bytes_freed: u64 = 0;
    for path in paths {
        let path_buf = PathBuf::from(&path);
        let Ok(canonical) = path_buf.canonicalize() else {
            // Unresolvable path — never delete.
            continue;
        };
        if !allowed_roots.iter().any(|root| canonical.starts_with(root)) {
            // Outside every allowed cache root — never delete.
            continue;
        }
        let size = std::fs::metadata(&canonical).map(|m| m.len()).unwrap_or(0);
        match std::fs::remove_file(&canonical) {
            Ok(()) => bytes_freed = bytes_freed.saturating_add(size),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                tracing::warn!(error = %error, path = %canonical.display(), "free_up_space: could not remove cache file");
            }
        }
    }

    tracing::info!(bytes_freed, "free_up_space evicted unpinned cache");
    Ok(FreeUpSpaceResult { bytes_freed })
}

fn now_unix_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}
