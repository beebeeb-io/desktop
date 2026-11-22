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
/// The copy was written moments ago, so a later write could not be told from it.
pub const KEPT_JUST_WRITTEN: &str = "This copy was written a moment ago, so it was left here.";
/// The file's row left the state it was in (not a local copy any more, or gone) while Free up space was running.
pub const KEPT_STATE_CHANGED: &str = "This file changed while Free up space was running, so it was left here.";
/// Free up space was stopped (locked or signed out) before this copy was removed.
pub const KEPT_STOPPED: &str = "Free up space was stopped before this copy was removed, so it was left here.";
/// Another program has this copy open.
pub const KEPT_OPEN_ELSEWHERE: &str = "Another program has this copy open, so it was left here.";
/// Beebeeb cannot tell whether another program has this copy open.
pub const KEPT_NO_LEASE: &str = "Beebeeb cannot check that no other program has this copy open, so it was left here.";
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
    /// A file at or under it holds a change that is not sent (uploading, a conflict, or in error with its change still
    /// queued: an upload that failed and waits, or gave up): its bytes are the only copy and stay.
    Unsent,
    /// A file at or under it is kept on this computer (also one that was kept after the selection was read).
    Kept,
    /// A file at or under it is in error here with no change of it queued (its download failed): it was not freed, and
    /// it is no change that waits to be sent, so it is never answered as one. (A file in error whose upload failed is
    /// [`Reclaimed::Unsent`]: review of #463, P3-11.)
    Error,
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
    // P3-11: the files whose change is still queued (the "Not sent" list), read once and only when a file in error is
    // met: in error with its change queued is an upload that failed, not a download.
    let unsent = std::cell::OnceCell::new();
    let change_queued = |id: &str| unsent.get_or_init(|| files_with_unsent_changes(db)).contains(id);
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
                    FileStatus::Uploading | FileStatus::Conflict => Reclaimed::Unsent,
                    FileStatus::Error if change_queued(&row.file_id) => Reclaimed::Unsent,
                    FileStatus::Error => Reclaimed::Error,
                    FileStatus::Local if kept(&row.file_id) => Reclaimed::Kept,
                    FileStatus::Local | FileStatus::Downloading => Reclaimed::Left,
                };
                // The reason that says most, whatever the order of the rows: an unsent change, then a kept file, then a
                // file in error, then a file merely left.
                what = match (what, this) {
                    (Reclaimed::Unsent, _) | (_, Reclaimed::Unsent) => Reclaimed::Unsent,
                    (Reclaimed::Kept, _) | (_, Reclaimed::Kept) => Reclaimed::Kept,
                    (Reclaimed::Error, _) | (_, Reclaimed::Error) => Reclaimed::Error,
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

/// The files of the sync folder whose change is still queued: every row of the "Not sent" list (`unsent_changes`, the
/// same predicates as its count) that names a file, a backup's own uploads left out. A page that cannot be read ends
/// the read with what was found: a file it would have named is answered as in error, never as freed.
#[cfg_attr(not(any(test, target_os = "windows")), allow(dead_code))]
fn files_with_unsent_changes(db: &crate::state_db::StateDb) -> std::collections::HashSet<String> {
    const PAGE: u32 = 500;
    let now = now_unix_seconds();
    let mut found = std::collections::HashSet::new();
    let mut offset = 0u32;
    while let Ok(page) = db.unsent_changes(offset, PAGE, now) {
        let read = u32::try_from(page.items.len()).unwrap_or(PAGE);
        found.extend(
            page.items
                .into_iter()
                .filter(|change| !change.backup)
                .filter_map(|change| change.file_id),
        );
        offset = offset.saturating_add(read);
        if read == 0 || i64::from(offset) >= page.total {
            break;
        }
    }
    found
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
    // One pass over the places: each folder on the way is read once.
    let mut places =
        crate::drive_v2::case_place::PlaceCheck::new(db, sync_root, crate::drive_v2::case_place::CaseRule::host());

    let finished = each_while(candidates, go_on, |(file_id, rel_path, _size_bytes)| {
        // Task 2407 (core 2404): never another file this name reaches (an 8.3 short name; `canonicalize` below would
        // follow it to that file's long name), nor the entry of another row at this place (two server items with one
        // name, or one name but for case). A check that cannot finish keeps the file local.
        let Some(local_path) = crate::name_alias::reclaim_place_of(&mut places, &file_id, &rel_path) else {
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
        if let Ok(Some(contract)) = db.get_file_contract_state(&file_id)
            && contract.effective_pin_state() == crate::state_db::PinState::Pinned
        {
            tracing::debug!(file_id = %file_id, "free_up_space: file pinned after enumeration; keeping local");
            return;
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
        // Task 2199 (dc-103b): read before the dehydrate, so a fetch recorded after it keeps its candidate.
        let seq_before = crate::engine_bridge::content_seq_for_wipe(db, &file_id);
        match crate::windows_cf::placeholders::dehydrate_placeholder(&canonical) {
            Ok(freed) => {
                bytes_freed = bytes_freed.saturating_add(freed);
                // Task 2199 (dc-103b; design 1640 "Not done" (a)): the dehydrate removed every byte (it reported
                // bytes freed and the platform now shows nothing on disk), so the record of what was fetched is
                // wiped to empty, unless a fetch was recorded since `seq_before`. The next range fetch then names
                // one version instead of mixing in versions whose bytes are gone.
                let nothing_left = freed > 0
                    && matches!(
                        crate::windows_cf::placeholders::platform_view(&canonical),
                        crate::unknown_check::PlatformView::Placeholder {
                            on_disk: crate::unknown_check::OnDisk::Nothing,
                            ..
                        }
                    );
                if let Some(seq_before) = seq_before.filter(|_| nothing_left)
                    && let Err(error) = db.wipe_fetched_content_if_unchanged(&file_id, seq_before)
                {
                    tracing::warn!(file_id = %file_id, error = %error, "free_up_space: could not wipe the fetch record");
                }
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

fn now_unix_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

#[cfg(not(target_os = "windows"))]
mod unix;
#[cfg(not(target_os = "windows"))]
pub use unix::free_up_space_unix;
#[cfg(all(test, unix))]
pub(crate) use unix::{Seams, reclaim_unix};
#[cfg(all(test, unix))]
mod unix_tests;

#[cfg(test)]
mod selection_tests {
    //! P3-11 (review of #463): a file in error whose upload failed is answered as holding a change that is not sent,
    //! as the upload's failure leaves it (`engine_bridge/upload.rs`: the row in error, its op still queued, waiting or
    //! given up); a file in error whose download failed is still answered as in error. Every name is invented.
    use super::{Reclaimed, free_selection};
    use crate::state_db::{FileEntry, FileStatus, ItemKind, OperationKind, PendingOperation, StateDb};

    fn row(db: &StateDb, id: &str, path: &str, status: FileStatus) {
        let entry = FileEntry {
            file_id: id.into(),
            path: path.into(),
            status,
            size_bytes: 10,
            modified_at: 1,
            content_hash: None,
            remote_updated_at: 1,
            parent_id: None,
            item_kind: ItemKind::File,
        };
        db.upsert_file_with_contract(&entry, |contract| contract.item_kind = ItemKind::File)
            .unwrap();
    }

    fn queued(db: &StateDb, op_id: &str, file_id: &str, kind: OperationKind, attempts: i64) {
        db.enqueue_operation(&PendingOperation {
            op_id: op_id.into(),
            kind,
            file_id: Some(file_id.into()),
            parent_id: None,
            target_path: Some(format!("/{file_id}.txt")),
            metadata_json: Some(r#"{"operation":"upload_version"}"#.into()),
            payload_path: None,
            base_version: None,
            base_object_version_id: None,
            attempts,
            max_attempts: 5,
            next_retry_at: 0,
            last_error: None,
            backup_source_key: None,
            created_at: 100,
            updated_at: 100,
        })
        .unwrap();
    }

    #[test]
    fn a_file_in_error_whose_upload_failed_is_answered_as_not_sent() {
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        // The upload failed and waits for its next try.
        row(&db, "f-wait", "/wait.txt", FileStatus::Error);
        queued(&db, "op-wait", "f-wait", OperationKind::UploadVersion, 1);
        // The upload gave up (out of attempts): the change is still not sent.
        row(&db, "f-gave", "/gave.txt", FileStatus::Error);
        queued(&db, "op-gave", "f-gave", OperationKind::UploadVersion, 5);
        // The download failed: its op is a download, which is no change to send.
        row(&db, "f-down", "/down.txt", FileStatus::Error);
        queued(&db, "op-down", "f-down", OperationKind::HydrateFile, 5);
        // In error with nothing queued.
        row(&db, "f-err", "/err.txt", FileStatus::Error);
        let ids: Vec<String> = ["f-wait", "f-gave", "f-down", "f-err"].map(String::from).to_vec();
        let answer = free_selection(&db, &ids, Vec::new(), |_| {
            panic!("nothing in error is handed to the reclaim")
        });
        let told: Vec<(&str, Reclaimed)> = answer.iter().map(|a| (a.id.as_str(), a.what)).collect();
        assert_eq!(
            told,
            [
                ("f-wait", Reclaimed::Unsent),
                ("f-gave", Reclaimed::Unsent),
                ("f-down", Reclaimed::Error),
                ("f-err", Reclaimed::Error),
            ]
        );
        assert!(answer.iter().all(|a| !a.freed_some), "nothing was freed");
    }
}
