//! Free up space: make unpinned local files online-only again. One implementation ([`free_up_space_in`]); the
//! native shell and the Windows Explorer flyout reach it through the engine handle (`app::Engine::free_up_space`,
//! the account-bound handle; the flyout by `crate::explorer_command`, task 2490). Must not touch a pinned file, an
//! unsent change, or a path outside the allowed roots.

use std::path::PathBuf;

#[derive(Debug, serde::Serialize)]
pub struct FreeUpSpaceResult {
    pub bytes_freed: u64,
    /// Copies the reclaim left on this computer because their bytes could not be shown to equal the synced
    /// content (non-Windows: [`free_up_space_unix`]); each with its plain words. Empty on Windows.
    pub not_freed: Vec<NotFreed>,
}

/// One copy [`free_up_space_unix`] kept, and why (words fit to show a person).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct NotFreed {
    pub file_id: String,
    pub reason: &'static str,
}

/// The copy's bytes are not the synced content: it was edited here after the sync.
pub const KEPT_EDITED: &str = "This copy was changed on this computer after it was last synced, so it was left here.";
/// The row holds no synced content to compare the copy with.
pub const KEPT_NO_AGREED: &str =
    "Beebeeb cannot confirm that this copy matches the synced version, so it was left here.";
/// A change of the file is waiting to be sent.
pub const KEPT_UPLOAD_QUEUED: &str = "A change to this file is waiting to be sent, so it was left here.";
/// The copy could not be opened or read.
pub const KEPT_UNREADABLE: &str = "Beebeeb could not read this copy to check it, so it was left here.";
/// The state database could not answer, so the copy was never opened.
pub const KEPT_CHECK_FAILED: &str = "Beebeeb could not check this copy, so it was left here.";
/// The copy is a link or a special file, not a plain file.
pub const KEPT_NOT_A_FILE: &str = "This is not a plain file (a link or a special file), so it was left here.";
/// The copy was written, or replaced, while Free up space was running.
pub const KEPT_CHANGED: &str = "This copy changed while Free up space was running, so it was left here.";
/// The file was kept, or changed state, while Free up space was running.
pub const KEPT_NOW_KEPT: &str =
    "This file was kept on this computer while Free up space was running, so it was left here.";
/// The operating system refused to remove the copy.
pub const KEPT_REMOVE_FAILED: &str = "Beebeeb could not remove this copy, so it was left here.";

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
        return Ok(FreeUpSpaceResult {
            bytes_freed: 0,
            not_freed: Vec::new(),
        });
    };
    let db_path = crate::state_paths::beebeeb_state_dir_for(identity)?.join(crate::state_paths::STATE_DB_FILENAME);
    if !db_path.exists() {
        return Ok(FreeUpSpaceResult {
            bytes_freed: 0,
            not_freed: Vec::new(),
        });
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
        // Each file is decided and removed together in `free_up_space_unix`, which asks `go_on` before each one:
        // a lock or sign-out ends it there, and what was freed until then is the answer.
        free_up_space_unix(identity, db, sync_root, go_on)
    }
}

/// Free up space for a SELECTION: the downloaded copies of `ids` (a file, or a folder with everything in it) become
/// online only; nothing else on this computer is touched. Answers per id, in the order asked, from the rows as they
/// are AFTER the reclaim ([`Reclaimed`]): freed only when every file at or under it is online only.
///
/// The same per-file guards as the whole reclaim (`dehydrate_these`): a kept file, a file with a change that is not
/// sent and a path outside the allowed roots are never touched. `go_on` is asked before each file.
/// Windows only today ([`SELECTION_RECLAIM`]): elsewhere the engine has no reclaim for a selection, and says so
/// before anything is read.
pub(crate) fn free_up_space_of(
    identity: &'static crate::identity::EngineIdentity,
    db: &crate::state_db::StateDb,
    sync_root: &std::path::Path,
    ids: &[String],
    go_on: &dyn Fn() -> bool,
) -> Result<Vec<SelectionAnswer>, String> {
    #[cfg(target_os = "windows")]
    {
        let candidates = db
            .unpinned_local_files_for_dehydration()
            .map_err(|e| format!("list dehydration candidates: {e}"))?;
        Ok(free_selection(db, ids, candidates, |wanted| {
            dehydrate_these(identity, db, sync_root, wanted, go_on);
        }))
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (identity, db, sync_root, ids, go_on);
        Err(NO_SELECTION_RECLAIM.to_string())
    }
}

/// Can a selection be reclaimed on this system ([`free_up_space_of`])? A host asks BEFORE it changes anything for a
/// Free up space (a kept item is released first): where this is `false`, nothing is released and nothing is asked.
pub const SELECTION_RECLAIM: bool = cfg!(target_os = "windows");

/// What [`free_up_space_of`] answers where a selection cannot be reclaimed (not Windows).
pub const NO_SELECTION_RECLAIM: &str = "freeing up space for a selection is not available on this system";

/// One selected id's answer of [`free_up_space_of`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectionAnswer {
    pub id: String,
    pub what: Reclaimed,
    /// At least one file at or under it was made online only by THIS call. Also `true` beside a `what` that is
    /// not `Freed`: a folder that was freed in part. A host that released a pin for the reclaim keeps the item
    /// again only where this is `false` (keeping a folder again downloads what was just freed).
    pub freed_some: bool,
}

/// What a selection's reclaim did for ONE selected id, read from the rows after it ran.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reclaimed {
    /// Every file at or under it is online only now (also when it already was, or when it holds no file). A file
    /// that was deleted here and waits for the server's trash has no bytes here and does not count.
    Freed,
    /// No row has this id.
    NoSuchItem,
    /// A file at or under it holds a change that is not sent (uploading, a conflict, an error): its bytes are the
    /// only copy and stay.
    Unsent,
    /// A file at or under it is kept on this computer (also one that was kept after the selection was read).
    Kept,
    /// A file at or under it is still downloaded: the reclaim did not reach it or could not free it (open in an
    /// app, the session ended first, a call that failed), or the rows could not be read to prove otherwise.
    Left,
}

/// Which of `candidates` (id, path, size) a selection names, handed to `reclaim` (which makes them online only and
/// records that in the rows), and the answer per selected id. The answer is decided from the rows AFTER `reclaim`,
/// never from what was handed over: an id with nothing to hand over (a file that is uploading, in conflict, in
/// error, or kept since) is not "freed" while its bytes are here. A folder ROW is not a file to free: it never
/// counts against its folder.
///
/// Reads: a selection of files alone reads its own rows (one per id, before and after). Only a selected FOLDER
/// reads every row once before and once after, to find what is under it (the state database has no query for the
/// files under a path in any status yet).
#[cfg_attr(not(any(test, target_os = "windows")), allow(dead_code))]
pub(crate) fn free_selection(
    db: &crate::state_db::StateDb,
    ids: &[String],
    candidates: Vec<(String, String, i64)>,
    reclaim: impl FnOnce(Vec<(String, String, i64)>),
) -> Vec<SelectionAnswer> {
    use crate::state_db::{FileEntry, FileStatus, PinState};
    // Each selected id's place: its path, without slashes at the ends, and whether it is a folder. `None`: no row.
    let places: Vec<Option<(String, bool)>> = ids
        .iter()
        .map(|id| match db.get_file(id) {
            Ok(Some(entry)) => Some((entry.path.trim_matches('/').to_string(), entry.is_dir())),
            _ => None,
        })
        .collect();
    let under = |path: &str, place: &str| {
        let path = path.trim_matches('/');
        path == place || (path.len() > place.len() && path.starts_with(place) && path.as_bytes()[place.len()] == b'/')
    };
    let a_folder_is_selected = places.iter().flatten().any(|(_, is_dir)| *is_dir);
    // The files (never a folder's row) at or under a selected place, as the rows say now.
    let selected_files = || -> Option<Vec<FileEntry>> {
        if a_folder_is_selected {
            let rows = db.list_files().ok()?;
            return Some(
                rows.into_iter()
                    .filter(|row| !row.is_dir() && places.iter().flatten().any(|(place, _)| under(&row.path, place)))
                    .collect(),
            );
        }
        let mut rows = Vec::new();
        for (id, place) in ids.iter().zip(&places) {
            if place.is_some() {
                rows.extend(db.get_file(id).ok()?);
            }
        }
        Some(rows)
    };
    let files: std::collections::HashSet<String> = selected_files()
        .unwrap_or_default()
        .into_iter()
        .map(|row| row.file_id)
        .collect();
    let wanted: Vec<(String, String, i64)> = candidates.into_iter().filter(|(id, _, _)| files.contains(id)).collect();
    let handed: std::collections::HashSet<String> = wanted.iter().map(|(id, _, _)| id.clone()).collect();
    if !wanted.is_empty() {
        reclaim(wanted);
    }
    // What is there now decides. Rows that cannot be read prove nothing: nothing is called freed.
    let after = selected_files();
    let kept = |id: &str| matches!(db.get_file_contract_state(id), Ok(Some(contract)) if contract.effective_pin_state() == PinState::Pinned);
    ids.iter()
        .zip(&places)
        .map(|(id, place)| {
            let (Some((place, _)), Some(after)) = (place, &after) else {
                let what = if place.is_none() {
                    Reclaimed::NoSuchItem
                } else {
                    Reclaimed::Left
                };
                return SelectionAnswer {
                    id: id.clone(),
                    what,
                    freed_some: false,
                };
            };
            let (mut what, mut freed_some) = (Reclaimed::Freed, false);
            for row in after.iter().filter(|row| under(&row.path, place)) {
                let this = match row.status {
                    FileStatus::CloudOnly => {
                        freed_some |= handed.contains(&row.file_id);
                        continue;
                    }
                    // Deleted here, waiting for the server's trash: its placeholder is gone, nothing to free.
                    FileStatus::Trashing => continue,
                    FileStatus::Uploading | FileStatus::Conflict | FileStatus::Error => Reclaimed::Unsent,
                    FileStatus::Local if kept(&row.file_id) => Reclaimed::Kept,
                    FileStatus::Local | FileStatus::Downloading => Reclaimed::Left,
                };
                // The reason that says most: an unsent change before a kept file before a file merely left.
                what = match (what, this) {
                    (Reclaimed::Unsent, _) | (_, Reclaimed::Unsent) => Reclaimed::Unsent,
                    (Reclaimed::Kept, _) | (_, Reclaimed::Kept) => Reclaimed::Kept,
                    _ => Reclaimed::Left,
                };
            }
            SelectionAnswer {
                id: id.clone(),
                what,
                freed_some,
            }
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
    (
        FreeUpSpaceResult {
            bytes_freed,
            not_freed: Vec::new(),
        },
        dehydrated_ids,
    )
}

/// Unix reclaim path (macOS File Provider / Linux FUSE): the engine stores hydrated copies as separate files, so a
/// `remove_file` genuinely reclaims the bytes. `go_on` is asked before each file: once it says no, no further
/// file is touched, and what was freed until then is the answer.
///
/// Task 2526: a copy INSIDE the sync folder (Linux: the pin's hydrate writes the folder file itself and records it
/// as the cache copy; nothing watches it) may hold an edit nobody detected. Each such copy is decided and removed
/// on its own, in this order, and any failed step keeps the copy and its row and lists it in `not_freed`:
/// 1. it is a plain file (not a link, not a pipe), and the row holds agreed content (`local_hash`) with no upload
///    of the file queued;
/// 2. its [`ContentFingerprint`] (size, times, device and inode) is the same before and after its SHA-256 is
///    streamed, the open handle is the file the path names, and the hash equals `local_hash`;
/// 3. the row is still unpinned and `Local`, and the fingerprint is taken once more, right before the unlink;
/// 4. the file is removed, and only then is its row marked online only.
///
/// What remains is POSIX's own gap: a write that lands between the last fingerprint and the `unlink` is deleted.
/// That is microseconds, and no check from outside can close it (Windows holds an exclusive handle across its
/// check and its action; a POSIX file cannot be locked against another program's write). A crash between the
/// unlink and the row's mark leaves a `Local` row with no file, which the next reclaim skips (its path is gone).
/// Copies in the engine's own cache directories are the engine's alone and need no check.
#[cfg(not(target_os = "windows"))]
pub fn free_up_space_unix(
    identity: &crate::identity::EngineIdentity,
    db: &crate::state_db::StateDb,
    sync_root: &std::path::Path,
    go_on: &dyn Fn() -> bool,
) -> Result<FreeUpSpaceResult, String> {
    reclaim_unix(
        identity,
        db,
        sync_root,
        go_on,
        &Seams {
            remove: &|path| std::fs::remove_file(path),
            at: &|_, _| {},
        },
    )
}

/// What [`reclaim_unix`] does that a test replaces: the unlink, and a call at two points of one copy's check.
#[cfg(not(target_os = "windows"))]
pub(crate) struct Seams<'a> {
    pub remove: &'a dyn Fn(&std::path::Path) -> std::io::Result<()>,
    /// Called with `("read", file_id)` after the bytes were hashed and before the second fingerprint, and with
    /// `("delete", file_id)` right before the last fingerprint.
    pub at: &'a dyn Fn(&str, &str),
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn reclaim_unix(
    identity: &crate::identity::EngineIdentity,
    db: &crate::state_db::StateDb,
    sync_root: &std::path::Path,
    go_on: &dyn Fn() -> bool,
    seams: &Seams<'_>,
) -> Result<FreeUpSpaceResult, String> {
    // Snapshot every UNPINNED, LOCAL file's cache copy; each row is looked at again right before it is touched.
    let rows = db
        .disposable_unpinned_cache_rows()
        .map_err(|e| format!("list cache paths: {e}"))?;
    let allowed_roots = free_up_space_allowed_roots(identity, sync_root);
    let sync_canonical = sync_root.canonicalize().unwrap_or_else(|_| sync_root.to_path_buf());

    let mut bytes_freed: u64 = 0;
    let mut not_freed: Vec<NotFreed> = Vec::new();
    let mut freed_ids: Vec<String> = Vec::new();
    for (file_id, path, agreed) in rows {
        if !go_on() {
            tracing::info!("free_up_space: stopped before the last file (the session that asked is gone)");
            break;
        }
        let canonical = PathBuf::from(&path).canonicalize().ok();
        let outcome = match canonical {
            // Unresolvable path, or outside every allowed cache root: nothing is deleted (the row still goes
            // online only, as before).
            None => Ok(0),
            Some(ref c) if !allowed_roots.iter().any(|root| c.starts_with(root)) => Ok(0),
            Some(ref c) if c.starts_with(&sync_canonical) => {
                reclaim_sync_copy(db, &file_id, std::path::Path::new(&path), c, agreed.as_deref(), seams)
            }
            // The engine's own cache directory: no byte check, by design.
            Some(ref c) => {
                let size = std::fs::metadata(c).map(|m| m.len()).unwrap_or(0);
                match (seams.remove)(c) {
                    Ok(()) => Ok(size),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(0),
                    Err(error) => {
                        tracing::warn!(error = %error, "free_up_space: could not remove cache file");
                        Err(KEPT_REMOVE_FAILED)
                    }
                }
            }
        };
        match outcome {
            Ok(freed) => {
                bytes_freed = bytes_freed.saturating_add(freed);
                // Only a copy that is gone (or has none to remove) has its row marked online only.
                match db.mark_cloud_only_after_dehydrate(std::slice::from_ref(&file_id), now_unix_seconds()) {
                    Ok(1) => freed_ids.push(file_id),
                    Ok(_) => {
                        tracing::warn!(file_id = %file_id, "free_up_space: row no longer eligible after its copy was removed")
                    }
                    Err(error) => {
                        tracing::warn!(file_id = %file_id, error = %error, "free_up_space: could not mark row online only")
                    }
                }
            }
            Err(reason) => {
                tracing::warn!(file_id = %file_id, reason, "free_up_space: kept a copy");
                not_freed.push(NotFreed { file_id, reason });
            }
        }
    }
    #[cfg(target_os = "linux")]
    remove_linux_freedesktop_thumbnails_for_evicted_files(db, sync_root, &freed_ids);

    tracing::info!(
        bytes_freed,
        kept = not_freed.len(),
        "free_up_space evicted unpinned cache"
    );
    Ok(FreeUpSpaceResult { bytes_freed, not_freed })
}

/// Steps 1 to 4 of [`free_up_space_unix`] for one sync folder copy. `Ok(bytes removed)`, or the plain words for
/// why the copy stays. `original` is the path as the row records it, `canonical` what it resolves to.
#[cfg(not(target_os = "windows"))]
fn reclaim_sync_copy(
    db: &crate::state_db::StateDb,
    file_id: &str,
    original: &std::path::Path,
    canonical: &std::path::Path,
    agreed: Option<&str>,
    seams: &Seams<'_>,
) -> Result<u64, &'static str> {
    use crate::engine_bridge::ContentFingerprint;
    use crate::state_db::{FileStatus, PinState};
    use sha2::{Digest, Sha256};
    use std::os::unix::fs::MetadataExt;

    // 1. A plain file, never a link or a pipe: opening a pipe blocks, and a link would act on its target. Looked
    // at before anything is opened.
    for path in [original, canonical] {
        match std::fs::symlink_metadata(path) {
            Ok(meta) if meta.file_type().is_file() => {}
            Ok(_) => return Err(KEPT_NOT_A_FILE),
            Err(_) => return Err(KEPT_UNREADABLE),
        }
    }
    let Some(agreed) = agreed else {
        return Err(KEPT_NO_AGREED);
    };
    // Anti-goal 3 (as the Windows stamp): the agreed content counts only while no upload of the file is queued.
    // A database error is not an answer: keep the copy, in its own words.
    if db.sync_folder_upload_queued(file_id).map_err(|_| KEPT_CHECK_FAILED)? {
        return Err(KEPT_UPLOAD_QUEUED);
    }

    // 2. The same file, unchanged, across the read.
    let before = ContentFingerprint::of(canonical).map_err(|_| KEPT_UNREADABLE)?;
    let mut file = std::fs::File::open(canonical).map_err(|_| KEPT_UNREADABLE)?;
    let mut hasher = Sha256::new();
    // Stream it; never load a large file into memory.
    std::io::copy(&mut file, &mut hasher).map_err(|_| KEPT_UNREADABLE)?;
    (seams.at)("read", file_id);
    let after = ContentFingerprint::of(canonical).map_err(|_| KEPT_CHANGED)?;
    let handle = file.metadata().map_err(|_| KEPT_UNREADABLE)?;
    let named = std::fs::symlink_metadata(canonical).map_err(|_| KEPT_CHANGED)?;
    if after != before || (handle.dev(), handle.ino()) != (named.dev(), named.ino()) {
        return Err(KEPT_CHANGED);
    }
    if format!("{:x}", hasher.finalize()) != agreed {
        return Err(KEPT_EDITED);
    }

    // 3. The row as it is now, then the file once more.
    let still_free = match (db.get_file(file_id), db.get_file_contract_state(file_id)) {
        (Ok(Some(entry)), Ok(Some(contract))) => {
            entry.status == FileStatus::Local && contract.effective_pin_state() != PinState::Pinned
        }
        (Ok(_), Ok(_)) => false,
        _ => return Err(KEPT_CHECK_FAILED),
    };
    if !still_free {
        return Err(KEPT_NOW_KEPT);
    }
    if db.sync_folder_upload_queued(file_id).map_err(|_| KEPT_CHECK_FAILED)? {
        return Err(KEPT_UPLOAD_QUEUED);
    }
    (seams.at)("delete", file_id);
    if ContentFingerprint::of(canonical).map_err(|_| KEPT_CHANGED)? != before {
        return Err(KEPT_CHANGED);
    }

    // 4. Gone, or kept: the caller marks the row only for a copy that is gone.
    match (seams.remove)(canonical) {
        Ok(()) => Ok(handle.len()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(error) => {
            tracing::warn!(error = %error, "free_up_space: could not remove a sync folder copy");
            Err(KEPT_REMOVE_FAILED)
        }
    }
}

fn now_unix_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

#[cfg(all(test, not(target_os = "windows")))]
mod unix_tests {
    //! Task 2526: the unix reclaim removes a sync folder copy only when its bytes are the agreed content, and
    //! checks again right before it deletes.
    use super::*;
    use crate::state_db::{FileEntry, FileStatus, ItemKind, OperationKind, PendingOperation, StateDb};
    use sha2::{Digest, Sha256};
    use std::cell::Cell;
    use std::path::Path;

    struct Root {
        _dir: tempfile::TempDir,
        root: PathBuf,
        db: StateDb,
    }

    fn sha(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }

    fn kept(id: &str, reason: &'static str) -> NotFreed {
        NotFreed {
            file_id: id.into(),
            reason,
        }
    }

    impl Root {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().join("sync");
            std::fs::create_dir(&root).unwrap();
            let db = StateDb::open(dir.path().join("state.db")).unwrap();
            Root { _dir: dir, root, db }
        }

        /// A kept-then-unkept file: a `Local`, unpinned row whose cache copy is the sync folder file `name`
        /// holding `on_disk`, with `agreed` as the row's agreed content.
        fn seed(&self, id: &str, name: &str, on_disk: &[u8], agreed: Option<String>) -> PathBuf {
            let path = self.root.join(name);
            std::fs::write(&path, on_disk).unwrap();
            self.seed_row(id, name, &path, on_disk.len(), agreed);
            path
        }

        fn seed_row(&self, id: &str, name: &str, cache_path: &Path, len: usize, agreed: Option<String>) {
            self.db
                .upsert_file_with_contract(
                    &FileEntry {
                        file_id: id.into(),
                        path: format!("/{name}"),
                        status: FileStatus::Local,
                        size_bytes: len as i64,
                        modified_at: 0,
                        content_hash: None,
                        remote_updated_at: 0,
                        parent_id: None,
                        item_kind: ItemKind::File,
                    },
                    |contract| contract.local_hash = agreed,
                )
                .unwrap();
            self.db
                .mark_cached(id, &cache_path.to_string_lossy(), len as i64, 1)
                .unwrap();
        }

        fn status(&self, id: &str) -> FileStatus {
            self.db.get_file(id).unwrap().unwrap().status
        }

        fn free(&self) -> FreeUpSpaceResult {
            free_up_space_in(&crate::identity::REAL, &self.db, &self.root, &|| true).unwrap()
        }

        /// The reclaim with a write (or any act) at the named point of a copy's check, and an unlink to choose.
        fn free_with(
            &self,
            go_on: &dyn Fn() -> bool,
            remove: &dyn Fn(&Path) -> std::io::Result<()>,
            at: &dyn Fn(&str, &str),
        ) -> FreeUpSpaceResult {
            reclaim_unix(
                &crate::identity::REAL,
                &self.db,
                &self.root,
                go_on,
                &Seams { remove, at },
            )
            .unwrap()
        }

        fn free_at(&self, at: &dyn Fn(&str, &str)) -> FreeUpSpaceResult {
            self.free_with(&|| true, &|p| std::fs::remove_file(p), at)
        }
    }

    /// Red before the fix: an edit nobody detected (no watcher) in a copy that is no longer kept is deleted.
    #[test]
    fn an_edited_unkept_copy_survives_free_up_space() {
        let r = Root::new();
        let path = r.seed(
            "edited",
            "edited.txt",
            b"my edit, never sent",
            Some(sha(b"version one")),
        );
        let result = r.free();
        assert_eq!(
            std::fs::read(&path).unwrap(),
            b"my edit, never sent",
            "the edit is still on disk"
        );
        assert_eq!(r.status("edited"), FileStatus::Local, "the row still says it is here");
        assert_eq!(result.bytes_freed, 0);
        assert_eq!(result.not_freed, vec![kept("edited", KEPT_EDITED)]);
    }

    #[test]
    fn an_agreed_copy_is_freed() {
        let r = Root::new();
        let path = r.seed("agreed", "agreed.txt", b"version one", Some(sha(b"version one")));
        let result = r.free();
        assert!(!path.exists(), "the agreed copy is removed");
        assert_eq!(r.status("agreed"), FileStatus::CloudOnly);
        assert_eq!(result.bytes_freed, b"version one".len() as u64);
        assert!(result.not_freed.is_empty());
    }

    #[test]
    fn a_copy_with_no_agreed_content_survives() {
        let r = Root::new();
        let path = r.seed("unknown", "unknown.txt", b"anything", None);
        let result = r.free();
        assert_eq!(std::fs::read(&path).unwrap(), b"anything");
        assert_eq!(r.status("unknown"), FileStatus::Local);
        assert_eq!(result.not_freed, vec![kept("unknown", KEPT_NO_AGREED)]);
    }

    /// One run, three rows: the kept ones do not hold back the agreed one, and the counts say which is which.
    #[test]
    fn a_mixed_run_frees_exactly_the_agreed_copies() {
        let r = Root::new();
        let good = r.seed("good", "good.txt", b"same", Some(sha(b"same")));
        let bad = r.seed("bad", "bad.txt", b"edited", Some(sha(b"same")));
        let none = r.seed("none", "none.txt", b"same", None);
        let result = r.free();
        assert_eq!(
            (good.exists(), bad.exists(), none.exists()),
            (false, true, true),
            "freed, kept, kept"
        );
        assert_eq!(result.not_freed.len(), 2);
        assert_eq!(result.bytes_freed, 4);
    }

    /// P2-4: an agreed copy with a change of the file waiting to be sent is not the agreed content any more.
    #[test]
    fn an_agreed_copy_with_an_upload_queued_survives() {
        let r = Root::new();
        let path = r.seed("queued", "queued.txt", b"same", Some(sha(b"same")));
        r.db.enqueue_operation(&PendingOperation {
            op_id: "op-1".into(),
            kind: OperationKind::UploadVersion,
            file_id: Some("queued".into()),
            parent_id: None,
            target_path: Some("queued.txt".into()),
            metadata_json: None,
            payload_path: None,
            base_version: Some(1),
            base_object_version_id: None,
            attempts: 0,
            max_attempts: 5,
            next_retry_at: 0,
            last_error: None,
            backup_source_key: None,
            created_at: 1,
            updated_at: 1,
        })
        .unwrap();
        let result = r.free();
        assert!(path.exists());
        assert_eq!(r.status("queued"), FileStatus::Local);
        assert_eq!(result.not_freed, vec![kept("queued", KEPT_UPLOAD_QUEUED)]);
    }

    /// P1-1 (a): a same-size write while the bytes are being read (the hash is of the old bytes and matches).
    #[test]
    fn a_write_while_the_copy_is_read_keeps_it() {
        let r = Root::new();
        let path = r.seed("busy", "busy.txt", b"version one", Some(sha(b"version one")));
        let result = r.free_at(&|stage, _| {
            if stage == "read" {
                std::fs::write(&path, b"version TWO!").unwrap();
            }
        });
        assert_eq!(std::fs::read(&path).unwrap(), b"version TWO!");
        assert_eq!(r.status("busy"), FileStatus::Local);
        assert_eq!(result.bytes_freed, 0);
        assert_eq!(result.not_freed, vec![kept("busy", KEPT_CHANGED)]);
    }

    /// P1-1: a write that keeps the size and sets the times back, after the hash and before the delete.
    #[test]
    fn a_same_size_write_with_the_times_set_back_before_the_delete_keeps_it() {
        let r = Root::new();
        let path = r.seed("sneaky", "sneaky.txt", b"version one", Some(sha(b"version one")));
        let meta = std::fs::metadata(&path).unwrap();
        let result = r.free_at(&|stage, _| {
            if stage == "delete" {
                let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
                use std::io::Write;
                (&file).write_all(b"version two").unwrap();
                file.set_modified(meta.modified().unwrap()).unwrap();
            }
        });
        assert_eq!(std::fs::read(&path).unwrap(), b"version two", "the edit is on disk");
        assert_eq!(r.status("sneaky"), FileStatus::Local);
        assert_eq!(result.not_freed, vec![kept("sneaky", KEPT_CHANGED)]);
    }

    /// P1-1 (b): an editor that saves to a temp file and renames it over the path, after the hash.
    #[test]
    fn a_save_by_rename_before_the_delete_keeps_the_new_file() {
        let r = Root::new();
        let path = r.seed("renamed", "renamed.txt", b"version one", Some(sha(b"version one")));
        let result = r.free_at(&|stage, _| {
            if stage == "delete" {
                let temp = path.with_extension("tmp");
                std::fs::write(&temp, b"my edit").unwrap();
                std::fs::rename(&temp, &path).unwrap();
            }
        });
        assert_eq!(std::fs::read(&path).unwrap(), b"my edit");
        assert_eq!(r.status("renamed"), FileStatus::Local);
        assert_eq!(result.not_freed, vec![kept("renamed", KEPT_CHANGED)]);
    }

    /// P1-1 (b), during the read: the handle still holds the old file, the path names the new one.
    #[test]
    fn a_save_by_rename_during_the_read_keeps_the_new_file() {
        let r = Root::new();
        let path = r.seed("renamed2", "renamed2.txt", b"version one", Some(sha(b"version one")));
        let result = r.free_at(&|stage, _| {
            if stage == "read" {
                let temp = path.with_extension("tmp");
                std::fs::write(&temp, b"version one").unwrap();
                std::fs::rename(&temp, &path).unwrap();
            }
        });
        assert!(path.exists(), "the file now at the path is not the one that was hashed");
        assert_eq!(r.status("renamed2"), FileStatus::Local);
        assert_eq!(result.not_freed, vec![kept("renamed2", KEPT_CHANGED)]);
    }

    /// P2-1: a keep made during the run (the row is pinned after the snapshot) keeps its file, and the row.
    #[test]
    fn a_keep_made_during_the_run_keeps_the_file() {
        let r = Root::new();
        let first = r.seed("a-first", "a.txt", b"aaaa", Some(sha(b"aaaa")));
        let second = r.seed("b-second", "b.txt", b"bbbb", Some(sha(b"bbbb")));
        let calls = Cell::new(0);
        let result = r.free_with(
            &|| {
                calls.set(calls.get() + 1);
                if calls.get() == 2 {
                    r.db.set_recursive_pin("b-second", true, 5).unwrap();
                }
                true
            },
            &|p| std::fs::remove_file(p),
            &|_, _| {},
        );
        assert!(!first.exists(), "the first copy was freed");
        assert!(second.exists(), "the copy kept during the run stays");
        assert_eq!(r.status("b-second"), FileStatus::Local);
        assert_eq!(result.not_freed, vec![kept("b-second", KEPT_NOW_KEPT)]);
        assert_eq!(result.bytes_freed, 4);
    }

    /// P3-3: a delete the system refuses leaves the copy, and its row says it is here.
    #[test]
    fn a_refused_delete_leaves_the_row_local() {
        let r = Root::new();
        let path = r.seed("refused", "refused.txt", b"same", Some(sha(b"same")));
        let result = r.free_with(
            &|| true,
            &|_| Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied)),
            &|_, _| {},
        );
        assert!(path.exists());
        assert_eq!(r.status("refused"), FileStatus::Local);
        assert_eq!(result.bytes_freed, 0);
        assert_eq!(result.not_freed, vec![kept("refused", KEPT_REMOVE_FAILED)]);
    }

    /// P2-2: a session that ended stops the run before the next file.
    #[test]
    fn a_session_that_ends_stops_the_run_before_the_next_file() {
        let r = Root::new();
        let first = r.seed("a-first", "a.txt", b"aaaa", Some(sha(b"aaaa")));
        let second = r.seed("b-second", "b.txt", b"bbbb", Some(sha(b"bbbb")));
        let calls = Cell::new(0);
        let result = r.free_with(
            &|| {
                calls.set(calls.get() + 1);
                calls.get() < 2
            },
            &|p| std::fs::remove_file(p),
            &|_, _| {},
        );
        assert_eq!((first.exists(), second.exists()), (false, true));
        assert_eq!(r.status("b-second"), FileStatus::Local);
        assert!(
            result.not_freed.is_empty(),
            "nothing was looked at, so nothing is reported"
        );
        assert_eq!(result.bytes_freed, 4);
    }

    /// P3-2: a link at the copy's place is not followed: its target (another file) is not deleted.
    #[test]
    fn a_link_in_the_sync_folder_is_never_followed() {
        let r = Root::new();
        let target = r.root.join("other.txt");
        std::fs::write(&target, b"same").unwrap();
        let link = r.root.join("link.txt");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        r.seed_row("linked", "link.txt", &link, 4, Some(sha(b"same")));
        let result = r.free();
        assert_eq!(std::fs::read(&target).unwrap(), b"same", "the target is untouched");
        assert!(link.symlink_metadata().is_ok(), "the link is untouched");
        assert_eq!(r.status("linked"), FileStatus::Local);
        assert_eq!(result.not_freed, vec![kept("linked", KEPT_NOT_A_FILE)]);
    }

    /// P3-4: a copy that cannot be opened is kept, in the words for that.
    #[test]
    fn an_unopenable_copy_is_kept() {
        use std::os::unix::fs::PermissionsExt;
        let r = Root::new();
        let path = r.seed("locked", "locked.txt", b"same", Some(sha(b"same")));
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
        if std::fs::File::open(&path).is_ok() {
            eprintln!("skipped: this user can open a mode 000 file (root)");
            return;
        }
        let result = r.free();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(path.exists());
        assert_eq!(r.status("locked"), FileStatus::Local);
        assert_eq!(result.not_freed, vec![kept("locked", KEPT_UNREADABLE)]);
    }

    /// P3-4: the claim that the engine's own cache directory is freed with no byte check (no agreed content).
    #[test]
    fn a_copy_in_the_engines_own_cache_directory_is_still_freed() {
        let r = Root::new();
        let cache = dirs::cache_dir()
            .expect("a cache dir")
            .join(crate::identity::TEST.config_dir_name());
        std::fs::create_dir_all(&cache).unwrap();
        let dir = tempfile::Builder::new().prefix("free-up-").tempdir_in(&cache).unwrap();
        let path = dir.path().join("staged.bin");
        std::fs::write(&path, b"staged copy").unwrap();
        r.seed_row("staged", "staged.bin", &path, 11, None);
        let result = free_up_space_in(&crate::identity::TEST, &r.db, &r.root, &|| true).unwrap();
        assert!(!path.exists());
        assert_eq!(r.status("staged"), FileStatus::CloudOnly);
        assert_eq!(result.bytes_freed, 11);
        assert!(result.not_freed.is_empty());
    }
}
