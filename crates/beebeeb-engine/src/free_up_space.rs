//! Free up space: make unpinned local files online-only again. One implementation ([`free_up_space_in`]); the
//! native shell and the Windows Explorer flyout reach it through the engine handle (`app::Engine::free_up_space`,
//! the account-bound handle; the flyout by `crate::explorer_command`, task 2490). Must not touch a pinned file, an
//! unsent change, or a path outside the allowed roots.

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

/// The Tauri shell's reclaim (its `free_up_space` IPC). A synchronous blocking fn (DB + filesystem work).
///
/// Task 2488: `pub` only with the `direct-state-db` feature (the Tauri shell): it opens the state database itself,
/// asking nobody whose it is. Task 2490: Explorer's command no longer calls it; it asks the engine handle
/// (`crate::explorer_command`), whose reclaim runs through the account-bound handle.
#[cfg(any(test, feature = "direct-state-db"))]
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
    free_up_space_in(identity, &db, &sync_root, &|| true)
}

/// `each` for every item, in order, asking `go_on` before each: the first no ends it, and the items after it are
/// left alone. `true`: every item was reached.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(crate) fn each_while<T>(
    items: impl IntoIterator<Item = T>,
    go_on: &dyn Fn() -> bool,
    mut each: impl FnMut(T),
) -> bool {
    for item in items {
        if !go_on() {
            return false;
        }
        each(item);
    }
    true
}

/// The reclaim itself, in a state database the caller already holds: the engine handle's is the signed-in
/// account's (`app::Engine::free_up_space`, through the account-bound handle).
///
/// `go_on` is asked before each file: once it says no (the session that asked is gone: a lock, a sign-out), no
/// further file is made online only, and what was freed until then is the answer.
pub(crate) fn free_up_space_in(
    identity: &'static crate::identity::EngineIdentity,
    db: &crate::state_db::StateDb,
    sync_root: &std::path::Path,
    go_on: &dyn Fn() -> bool,
) -> Result<FreeUpSpaceResult, String> {
    #[cfg(target_os = "windows")]
    {
        free_up_space_windows_while(identity, db, sync_root, go_on)
    }
    #[cfg(not(target_os = "windows"))]
    {
        // The whole choice of what leaves this computer is ONE query here (`free_up_space_unix`), made right after
        // the handle's own check; the loop after it only removes the copies of rows that query already made online
        // only, and is not cut short (a copy left behind would be a file no row knows of).
        let _ = go_on;
        // Snapshot the on-disk cache paths of every UNPINNED, LOCAL file BEFORE
        // any DB transition (eviction nulls `cache_path`). Pinned files are
        // excluded by the query, so they are always kept.
        let paths = db
            .disposable_unpinned_cache_paths()
            .map_err(|e| format!("list cache paths: {e}"))?;
        free_up_space_unix(identity, db, sync_root, paths)
    }
}

/// Free up space for a SELECTION: the downloaded copies of `ids` (a file, or a folder with everything in it) become
/// online only; nothing else on this computer is touched. Answers per id, in the order asked: `true` when nothing
/// that could be freed is left at or under it (also when nothing was there to free). `false`: unknown id, or at
/// least one file under it stayed (kept, an unsent change, a failed call, or the session ended before it).
///
/// The same per-file guards as the whole reclaim (`dehydrate_these`): a kept file, a file with a change that is not
/// sent and a path outside the allowed roots are never touched. `go_on` is asked before each file.
/// Windows only today: elsewhere the engine has no reclaim for a selection, and says so.
pub(crate) fn free_up_space_of(
    identity: &'static crate::identity::EngineIdentity,
    db: &crate::state_db::StateDb,
    sync_root: &std::path::Path,
    ids: &[String],
    go_on: &dyn Fn() -> bool,
) -> Result<Vec<(String, bool)>, String> {
    #[cfg(target_os = "windows")]
    {
        let candidates = db
            .unpinned_local_files_for_dehydration()
            .map_err(|e| format!("list dehydration candidates: {e}"))?;
        Ok(free_selection(db, ids, candidates, |wanted| {
            dehydrate_these(identity, db, sync_root, wanted, go_on).1
        }))
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (identity, db, sync_root, ids, go_on);
        Err(NO_SELECTION_RECLAIM.to_string())
    }
}

/// What [`free_up_space_of`] answers where a selection cannot be reclaimed (not Windows).
pub const NO_SELECTION_RECLAIM: &str = "freeing up space for a selection is not available on this system";

/// Which of `candidates` (id, path, size) a selection names, handed to `reclaim` (which answers the ids it made
/// online only), and the answer per selected id. One read per selected id; the candidates are matched by path in
/// memory. A folder ROW among the candidates is not a file to free: it never counts against its folder.
#[cfg_attr(not(any(test, target_os = "windows")), allow(dead_code))]
pub(crate) fn free_selection(
    db: &crate::state_db::StateDb,
    ids: &[String],
    candidates: Vec<(String, String, i64)>,
    reclaim: impl FnOnce(Vec<(String, String, i64)>) -> Vec<String>,
) -> Vec<(String, bool)> {
    use crate::state_db::ItemKind;
    // Each selected id's place: its path, without slashes at the ends. `None`: no such row.
    let places: Vec<Option<String>> = ids
        .iter()
        .map(|id| match db.get_file(id) {
            Ok(Some(entry)) => Some(entry.path.trim_matches('/').to_string()),
            _ => None,
        })
        .collect();
    let under = |path: &str, place: &str| {
        let path = path.trim_matches('/');
        path == place || (path.len() > place.len() && path.starts_with(place) && path.as_bytes()[place.len()] == b'/')
    };
    let is_file = |id: &str| matches!(db.get_file(id), Ok(Some(entry)) if entry.item_kind == ItemKind::File);
    let wanted: Vec<(String, String, i64)> = candidates
        .into_iter()
        .filter(|(id, path, _)| places.iter().flatten().any(|place| under(path, place)) && is_file(id))
        .collect();
    let asked: Vec<(String, String)> = wanted.iter().map(|(id, path, _)| (id.clone(), path.clone())).collect();
    let freed: std::collections::HashSet<String> = if wanted.is_empty() {
        Default::default()
    } else {
        reclaim(wanted).into_iter().collect()
    };
    ids.iter()
        .zip(&places)
        .map(|(id, place)| {
            let done = place.as_ref().is_some_and(|place| {
                asked
                    .iter()
                    .filter(|(_, path)| under(path, place))
                    .all(|(file, _)| freed.contains(file))
            });
            (id.clone(), done)
        })
        .collect()
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
    free_up_space_windows_while(identity, db, sync_root, &|| true)
}

/// [`free_up_space_windows`], asking `go_on` before each file: after the first no, no further file is made online
/// only; the files already made online only are recorded as such, and their bytes are the answer.
#[cfg(target_os = "windows")]
pub(crate) fn free_up_space_windows_while(
    identity: &crate::identity::EngineIdentity,
    db: &crate::state_db::StateDb,
    sync_root: &std::path::Path,
    go_on: &dyn Fn() -> bool,
) -> Result<FreeUpSpaceResult, String> {
    let candidates = db
        .unpinned_local_files_for_dehydration()
        .map_err(|e| format!("list dehydration candidates: {e}"))?;
    Ok(dehydrate_these(identity, db, sync_root, candidates, go_on).0)
}

/// The reclaim of `candidates` (rows of `StateDb::unpinned_local_files_for_dehydration`, all of them or the ones a
/// selection names): the bytes freed, and the ids of the files that are online only now. Every guard is here, once,
/// for both callers.
#[cfg(target_os = "windows")]
fn dehydrate_these(
    identity: &crate::identity::EngineIdentity,
    db: &crate::state_db::StateDb,
    sync_root: &std::path::Path,
    candidates: Vec<(String, String, i64)>,
    go_on: &dyn Fn() -> bool,
) -> (FreeUpSpaceResult, Vec<String>) {
    // Reclaim targets are the UNPINNED, LOCAL files — addressed by their
    // server-relative path, NOT by `cache_path`. On Windows CF the bytes live
    // inside the placeholder in the sync root, and `cache_path` records the
    // already-deleted `%TEMP%` decrypt path from the fetch callback, so it is
    // useless as a dehydration target. We reconstruct the real placeholder path
    // exactly as `windows_cf::populate_placeholders` does.
    let allowed_roots = free_up_space_allowed_roots(identity, sync_root);
    let mut bytes_freed: u64 = 0;
    let mut dehydrated_ids: Vec<String> = Vec::new();

    let finished = each_while(candidates, go_on, |(file_id, rel_path, _size_bytes)| {
        // Task 2407 (core 2404): never another file this name reaches (an 8.3 short name; `canonicalize` below would
        // follow it to that file's long name). A check that cannot finish keeps the file local.
        let Some(local_path) = crate::name_alias::reclaim_place(sync_root, &rel_path) else {
            return;
        };
        // Path-safety: only dehydrate placeholders that actually resolve under
        // an allowed engine root (the sync root). A corrupt `path` row can
        // never point us at a file outside the vault.
        let Ok(canonical) = local_path.canonicalize() else {
            // Placeholder missing on disk — nothing to reclaim; leave the row
            // for the next reconcile to re-seed.
            return;
        };
        if !allowed_roots.iter().any(|root| canonical.starts_with(root)) {
            return;
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
                return;
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
            return;
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
            return;
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
    });
    if !finished {
        tracing::info!("free_up_space: stopped before the last file (the session that asked is gone)");
    }

    // Mark cloud_only ONLY for the files we actually dehydrated.
    let now = now_unix_seconds();
    if let Err(e) = db.mark_cloud_only_after_dehydrate(&dehydrated_ids, now) {
        tracing::warn!(error = %e, "free_up_space: could not mark dehydrated files cloud_only");
    }

    tracing::info!(bytes_freed, "free_up_space dehydrated unpinned placeholders");
    (FreeUpSpaceResult { bytes_freed }, dehydrated_ids)
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
