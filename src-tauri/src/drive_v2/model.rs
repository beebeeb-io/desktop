//! What the read commands show, computed from the local state DB (the decrypted mirror the engine
//! keeps). Pure over `&StateDb`, so every function here is tested against a temporary database.
//!
//! The list never loads from the network (DW-0401 Rules): it is the copy on this PC.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use serde::Deserialize;

use super::DriveError;
use super::dto::*;
use crate::state_db::{self, FileContractState, FileEntry, FileStatus, ItemKind, Namespace, PinState, StateDb};

/// The root crumb's name (the board's "Files").
pub(crate) const ROOT_NAME: &str = "Files";
/// A provisional id: an item this PC created whose row the watcher has not minted yet.
pub(crate) const LOCAL_ID_PREFIX: &str = "local:";
/// Photos page size.
pub(crate) const PHOTOS_PAGE: usize = 200;

pub(crate) fn iso(secs: i64) -> String {
    chrono::DateTime::from_timestamp(secs.max(0), 0)
        .unwrap_or_default()
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

pub(crate) fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// The display name of a row: its path's leaf, or the stable id label the rest of the app uses.
/// A file whose name never decrypted (empty path, vault was locked) shows the existing
/// "Unavailable file <hex>" label rather than vanishing from the list (task 2445).
pub(crate) fn leaf_name(entry: &FileEntry) -> String {
    crate::vault_tree::folder_leaf_name(&entry.path).unwrap_or_else(|| {
        if entry.is_dir() {
            crate::vault_tree::folder_fallback_label(&entry.file_id)
        } else {
            // Display only. The path stays empty, so this label is never uploaded.
            crate::owner_keys::blocked_name(&entry.file_id)
        }
    })
}

fn extension(name: &str) -> Option<String> {
    let (stem, ext) = name.rsplit_once('.')?;
    if stem.is_empty() || ext.is_empty() {
        return None;
    }
    Some(ext.to_ascii_lowercase())
}

/// The mime type a name implies; the content type the server recorded wins when there is one.
pub(crate) fn mime_for_name(name: &str) -> Option<&'static str> {
    Some(match extension(name)?.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "heic" => "image/heic",
        "heif" => "image/heif",
        "avif" => "image/avif",
        "bmp" => "image/bmp",
        "tif" | "tiff" => "image/tiff",
        "dng" => "image/x-adobe-dng",
        "svg" => "image/svg+xml",
        "mp4" | "m4v" => "video/mp4",
        "mov" => "video/quicktime",
        "webm" => "video/webm",
        "mkv" => "video/x-matroska",
        "avi" => "video/x-msvideo",
        "3gp" => "video/3gpp",
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "wav" => "audio/wav",
        "flac" => "audio/flac",
        "pdf" => "application/pdf",
        "txt" | "log" => "text/plain",
        "md" | "markdown" => "text/markdown",
        "csv" => "text/csv",
        "json" => "application/json",
        "html" | "htm" => "text/html",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "odt" => "application/vnd.oasis.opendocument.text",
        "ods" => "application/vnd.oasis.opendocument.spreadsheet",
        "zip" => "application/zip",
        _ => return None,
    })
}

/// Images and videos: what Photos shows and what the storage meter counts as Photos.
pub(crate) fn is_photo_or_video(name: &str) -> bool {
    mime_for_name(name).is_some_and(|m| m.starts_with("image/") || m.starts_with("video/"))
}

fn has_thumbnail(name: &str) -> bool {
    mime_for_name(name).is_some_and(|m| m.starts_with("image/") || m.starts_with("video/") || m == "application/pdf")
}

pub(crate) fn initials(name: &str) -> String {
    let base = name.split('@').next().unwrap_or(name);
    let mut out: String = base
        .split(|c: char| c == '.' || c == ' ' || c == '_' || c == '-')
        .filter_map(|part| part.chars().next())
        .take(2)
        .flat_map(char::to_uppercase)
        .collect();
    if out.is_empty() {
        out = "?".to_string();
    }
    out
}

/// Where a row's bytes are on this device. `None` for a folder that is not kept offline.
pub(crate) fn local_state(status: &FileStatus, is_folder: bool, pinned: bool) -> Option<LocalState> {
    if is_folder {
        return pinned.then_some(LocalState::AvailableOffline);
    }
    Some(match status {
        FileStatus::CloudOnly => LocalState::OnlineOnly,
        FileStatus::Local if pinned => LocalState::AvailableOffline,
        FileStatus::Local => LocalState::Downloaded,
        FileStatus::Downloading | FileStatus::Uploading => LocalState::Syncing,
        FileStatus::Conflict | FileStatus::Error | FileStatus::Trashing => LocalState::Failed,
    })
}

fn is_pinned(contract: Option<&FileContractState>) -> bool {
    contract.is_some_and(|c| c.effective_pin_state() == PinState::Pinned)
}

/// One row as the window shows it.
pub(crate) fn item_from(
    entry: &FileEntry,
    contract: Option<&FileContractState>,
    child_count: Option<i64>,
) -> DriveItem {
    let name = leaf_name(entry);
    let is_folder = entry.is_dir();
    let shared_with_me = contract.is_some_and(|c| c.namespace == Namespace::SharedWithMe);
    let shared = match contract
        .and_then(|c| c.owner_email.as_deref())
        .filter(|_| shared_with_me)
    {
        Some(owner) => SharedState::People {
            people: vec![Person {
                name: owner.to_string(),
                initials: initials(owner),
            }],
        },
        None => SharedState::None,
    };
    let read_only = (shared_with_me && contract.is_some_and(|c| c.permission_bits & state_db::PERMISSION_WRITE == 0))
        .then_some(true);
    let mime = if is_folder {
        None
    } else {
        contract
            .and_then(|c| c.content_type.clone())
            .filter(|t| !t.is_empty())
            .or_else(|| mime_for_name(&name).map(str::to_string))
    };
    DriveItem {
        id: entry.file_id.clone(),
        parent_id: entry.parent_id.clone(),
        is_folder,
        size: (!is_folder).then_some(entry.size_bytes.max(0)),
        child_count: if is_folder { child_count } else { None },
        modified_at: iso(entry.modified_at),
        mime,
        shared,
        local: local_state(&entry.status, is_folder, is_pinned(contract)),
        read_only,
        has_thumbnail: !is_folder && has_thumbnail(&name),
        name,
    }
}

/// An item read fresh for one id (contract and child count included).
pub(crate) fn item_for_entry(db: &StateDb, entry: &FileEntry) -> Result<DriveItem, DriveError> {
    let contract = db.get_file_contract_state(&entry.file_id).map_err(DriveError::db)?;
    let count = if entry.is_dir() {
        Some(db.child_item_count(&entry.file_id).map_err(DriveError::db)?)
    } else {
        None
    };
    Ok(item_from(entry, contract.as_ref(), count))
}

/// The row behind an id: a server id, or a provisional `local:<path>` id for an item this PC just
/// created. A row being trashed is gone from the window.
pub(crate) fn resolve(db: &StateDb, id: &str) -> Result<FileEntry, DriveError> {
    let found = match id.strip_prefix(LOCAL_ID_PREFIX) {
        Some(path) => db.get_file_by_path(path).map_err(DriveError::db)?,
        None => db.get_file(id).map_err(DriveError::db)?,
    };
    found
        .filter(|e| e.status != FileStatus::Trashing)
        .ok_or(DriveError::NotFound)
}

pub(crate) fn resolve_folder(db: &StateDb, id: Option<&str>) -> Result<Option<FileEntry>, DriveError> {
    match id {
        None => Ok(None),
        Some(id) => {
            let entry = resolve(db, id)?;
            if entry.is_dir() {
                Ok(Some(entry))
            } else {
                Err(DriveError::NotFound)
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub(crate) enum SortKey {
    #[default]
    Name,
    Modified,
    Size,
    Kind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub(crate) enum SortDir {
    #[default]
    Asc,
    Desc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
pub(crate) struct Sort {
    #[serde(default)]
    pub(crate) key: SortKey,
    #[serde(default)]
    pub(crate) dir: SortDir,
}

fn by_name(a: &DriveItem, b: &DriveItem) -> Ordering {
    a.name
        .to_lowercase()
        .cmp(&b.name.to_lowercase())
        .then_with(|| a.name.cmp(&b.name))
        .then_with(|| a.id.cmp(&b.id))
}

/// Folders first, always; then the chosen key, ties by name.
pub(crate) fn sort_items(items: &mut [DriveItem], sort: Sort) {
    items.sort_by(|a, b| {
        let folders = b.is_folder.cmp(&a.is_folder);
        if folders != Ordering::Equal {
            return folders;
        }
        let key = match sort.key {
            SortKey::Name => Ordering::Equal,
            SortKey::Modified => a.modified_at.cmp(&b.modified_at),
            SortKey::Size => a.size.unwrap_or(0).cmp(&b.size.unwrap_or(0)),
            SortKey::Kind => extension(&a.name).cmp(&extension(&b.name)),
        };
        let ordered = key.then_with(|| by_name(a, b));
        if sort.dir == SortDir::Desc {
            ordered.reverse()
        } else {
            ordered
        }
    });
}

/// Every live row (a row being trashed is gone), with one row per path and kind: a row the server
/// never confirmed folds into the server's row at the same path, as Selective sync shows them.
pub(crate) fn live_rows(db: &StateDb) -> Result<Vec<FileEntry>, DriveError> {
    let rows = db.file_overview_rows().map_err(DriveError::db)?;
    let mut best: HashMap<(bool, String), FileEntry> = HashMap::new();
    let mut unkeyed = Vec::new();
    for (entry, _) in rows {
        if entry.status == FileStatus::Trashing {
            continue;
        }
        let path = entry.path.trim_matches('/').to_lowercase();
        if path.is_empty() {
            unkeyed.push(entry);
            continue;
        }
        let key = (entry.is_dir(), path);
        match best.get(&key) {
            Some(kept) if !prefer(&entry, kept) => {}
            _ => {
                best.insert(key, entry);
            }
        }
    }
    unkeyed.extend(best.into_values());
    Ok(unkeyed)
}

/// Whether `a` should stay at a shared path instead of `b`: the server's row (a version or a remote
/// time recorded), then a row that is not merely local, then the smaller id.
fn prefer(a: &FileEntry, b: &FileEntry) -> bool {
    let known = |e: &FileEntry| e.remote_updated_at > 0;
    let not_local = |e: &FileEntry| e.status != FileStatus::Local;
    (known(a), not_local(a), std::cmp::Reverse(&a.file_id)) > (known(b), not_local(b), std::cmp::Reverse(&b.file_id))
}

/// Drop the rows a freshly applied snapshot no longer has. `prune_absent` keeps a row whose
/// operation is still queued; a terminal-failed operation (out of attempts, not paused) is not in
/// flight, so its row must leave with the snapshot instead of staying forever (slice 58: a failed
/// upload left an "Unavailable file …" row that could not be trashed). Returns every pruned row,
/// children before parents, so the caller can remove each on-disk placeholder.
pub(crate) fn drop_absent_rows(
    db: &StateDb,
    seen: &HashSet<String>,
    snapshot_fetched_at: i64,
) -> rusqlite::Result<Vec<state_db::PrunedRow>> {
    let mut pruned = db.prune_absent(seen, snapshot_fetched_at)?;
    // `prune_absent` refuses an empty snapshot; never drop on one here either.
    if seen.is_empty() {
        return Ok(pruned);
    }
    // A row leaves only when every op it still has is a terminal failure. `waiting_rows` is the
    // queue as the sign-out path reads it: one row per op, with its attempts and pause.
    let mut ops_by_file: HashMap<String, Vec<state_db::WaitingRow>> = HashMap::new();
    for op in db.waiting_rows()? {
        if let Some(file_id) = &op.file_id {
            ops_by_file.entry(file_id.clone()).or_default().push(op);
        }
    }
    for (file_id, ops) in ops_by_file {
        if seen.contains(&file_id) {
            continue;
        }
        if ops.iter().any(|op| op.paused || op.attempts < op.max_attempts) {
            continue; // still in flight: the snapshot cannot contradict it
        }
        let Some(entry) = db.get_file(&file_id)? else {
            continue;
        };
        if entry.status == FileStatus::Uploading || entry.status == FileStatus::Trashing {
            continue;
        }
        if entry.remote_updated_at >= snapshot_fetched_at {
            continue; // touched at/after the snapshot: the snapshot predates it
        }
        let contract = db.get_file_contract_state(&file_id)?;
        if contract.is_some_and(|c| c.namespace != Namespace::MyFiles) {
            continue;
        }
        if let Some(parent) = entry.parent_id.as_deref()
            && db.get_file(parent)?.is_some_and(|p| p.status == FileStatus::Trashing)
        {
            continue; // the trash view's content, not a dead row
        }
        pruned.extend(db.delete_file_subtree(&file_id)?);
    }
    Ok(pruned)
}

/// The crumbs from the root to `folder` (both included). A cycle or a missing parent ends the walk.
pub(crate) fn crumbs(db: &StateDb, folder: Option<&FileEntry>) -> Result<Vec<Crumb>, DriveError> {
    let mut chain = Vec::new();
    let mut next = folder.cloned();
    let mut seen = std::collections::HashSet::new();
    while let Some(entry) = next {
        if !seen.insert(entry.file_id.clone()) || seen.len() > 256 {
            break;
        }
        chain.push(Crumb {
            id: Some(entry.file_id.clone()),
            name: leaf_name(&entry),
        });
        next = match entry.parent_id.as_deref() {
            Some(parent) => db.get_file(parent).map_err(DriveError::db)?,
            None => None,
        };
    }
    chain.push(Crumb {
        id: None,
        name: ROOT_NAME.to_string(),
    });
    chain.reverse();
    Ok(chain)
}

/// `drive_list`: one folder's children from the copy on this PC, using the
/// `idx_files_parent_id` index (task 2445). A 70k-row DB reads only one
/// folder's children instead of every row.
pub(crate) fn list(db: &StateDb, folder_id: Option<&str>, sort: Sort, stale: bool) -> Result<Listing, DriveError> {
    let folder = resolve_folder(db, folder_id)?;
    let want_parent = folder.as_ref().map(|f| f.file_id.as_str());
    let children = db.children_of(want_parent).map_err(DriveError::db)?;
    // Dedupe: a row the server never confirmed folds into the server's row at the same path.
    let mut best: HashMap<(bool, String), (FileEntry, bool)> = HashMap::new();
    let mut unkeyed: Vec<(FileEntry, bool)> = Vec::new();
    for (entry, pinned) in children {
        // Shared-with-me roots have no parent; listed in Shared, not in Files.
        if want_parent.is_none() {
            let contract = db.get_file_contract_state(&entry.file_id).map_err(DriveError::db)?;
            if contract
                .as_ref()
                .is_some_and(|c| c.namespace == Namespace::SharedWithMe)
            {
                continue;
            }
        }
        let path = entry.path.trim_matches('/').to_lowercase();
        if path.is_empty() {
            unkeyed.push((entry, pinned));
            continue;
        }
        let key = (entry.is_dir(), path);
        match best.get(&key) {
            Some((kept, _)) if !prefer(&entry, kept) => {}
            _ => {
                best.insert(key, (entry, pinned));
            }
        }
    }
    let all_entries: Vec<(FileEntry, bool)> = {
        let mut v: Vec<_> = best.into_values().collect();
        v.extend(unkeyed);
        v
    };
    // Batch child counts for every folder in this listing.
    let folder_ids: Vec<&str> = all_entries
        .iter()
        .filter(|(e, _)| e.is_dir())
        .map(|(e, _)| e.file_id.as_str())
        .collect();
    let counts = db.child_counts(&folder_ids).map_err(DriveError::db)?;
    let mut items: Vec<DriveItem> = Vec::with_capacity(all_entries.len());
    for (entry, _pinned) in &all_entries {
        let contract = db.get_file_contract_state(&entry.file_id).map_err(DriveError::db)?;
        let child_count = if entry.is_dir() {
            Some(counts.get(&entry.file_id).copied().unwrap_or(0))
        } else {
            None
        };
        items.push(item_from(entry, contract.as_ref(), child_count));
    }
    sort_items(&mut items, sort);
    let path = crumbs(db, folder.as_ref())?;
    let folder_crumb = path.last().cloned().unwrap_or(Crumb {
        id: None,
        name: ROOT_NAME.to_string(),
    });
    Ok(Listing {
        folder: folder_crumb,
        path,
        items,
        as_of: now_iso(),
        stale,
    })
}

/// Task 2445: a paged listing for the native virtual list. `children_of` with sort, offset and
/// limit, plus the total count for the caller's scroll bar. Pure over `StateDb`; no Tauri types.
pub(crate) fn children_of(
    db: &StateDb,
    folder_id: Option<&str>,
    sort: Sort,
    offset: usize,
    limit: usize,
) -> Result<(Vec<DriveItem>, usize), DriveError> {
    let folder = resolve_folder(db, folder_id)?;
    let want_parent = folder.as_ref().map(|f| f.file_id.as_str());
    let children = db.children_of(want_parent).map_err(DriveError::db)?;
    // Dedupe (same logic as list).
    let mut best: HashMap<(bool, String), (FileEntry, bool)> = HashMap::new();
    let mut unkeyed: Vec<(FileEntry, bool)> = Vec::new();
    for (entry, pinned) in children {
        if want_parent.is_none() {
            let contract = db.get_file_contract_state(&entry.file_id).map_err(DriveError::db)?;
            if contract
                .as_ref()
                .is_some_and(|c| c.namespace == Namespace::SharedWithMe)
            {
                continue;
            }
        }
        let path = entry.path.trim_matches('/').to_lowercase();
        if path.is_empty() {
            unkeyed.push((entry, pinned));
            continue;
        }
        let key = (entry.is_dir(), path);
        match best.get(&key) {
            Some((kept, _)) if !prefer(&entry, kept) => {}
            _ => {
                best.insert(key, (entry, pinned));
            }
        }
    }
    let mut all_entries: Vec<(FileEntry, bool)> = best.into_values().collect();
    all_entries.extend(unkeyed);
    let folder_ids: Vec<&str> = all_entries
        .iter()
        .filter(|(e, _)| e.is_dir())
        .map(|(e, _)| e.file_id.as_str())
        .collect();
    let counts = db.child_counts(&folder_ids).map_err(DriveError::db)?;
    let mut items: Vec<DriveItem> = Vec::with_capacity(all_entries.len());
    for (entry, _) in &all_entries {
        let contract = db.get_file_contract_state(&entry.file_id).map_err(DriveError::db)?;
        let child_count = if entry.is_dir() {
            Some(counts.get(&entry.file_id).copied().unwrap_or(0))
        } else {
            None
        };
        items.push(item_from(entry, contract.as_ref(), child_count));
    }
    sort_items(&mut items, sort);
    let total = items.len();
    let page: Vec<DriveItem> = items.into_iter().skip(offset).take(limit).collect();
    Ok((page, total))
}

/// `drive_details`. `versionCount` is the current version number the server recorded (the version
/// list itself is `drive_versions`); a folder has none.
pub(crate) fn details(db: &StateDb, id: &str) -> Result<ItemDetails, DriveError> {
    let entry = resolve(db, id)?;
    let item = item_for_entry(db, &entry)?;
    let parent = match entry.parent_id.as_deref() {
        Some(p) => db.get_file(p).map_err(DriveError::db)?,
        None => None,
    };
    let path = crumbs(db, parent.as_ref())?;
    let version_count = if entry.is_dir() {
        0
    } else {
        db.get_file_contract_state(&entry.file_id)
            .map_err(DriveError::db)?
            .map_or(1, |c| c.current_version.max(1))
    };
    Ok(ItemDetails {
        item,
        path,
        changed_on: None,
        version_count,
    })
}

/// `drive_recent`: the newest files, newest first.
pub(crate) fn recent(db: &StateDb, limit: usize) -> Result<Vec<DriveItem>, DriveError> {
    let limit = limit.clamp(1, 200);
    let rows = db.newest_file_rows(limit + 16).map_err(DriveError::db)?;
    let mut out = Vec::new();
    for (entry, _) in rows
        .into_iter()
        .filter(|(e, _)| e.status != FileStatus::Trashing)
        .take(limit)
    {
        out.push(item_for_entry(db, &entry)?);
    }
    Ok(out)
}

/// `drive_photos`: images and videos, newest first, a page at a time. The cursor is an offset.
/// `takenAt` is the modified time: the capture time is inside the encrypted file, not in the mirror.
pub(crate) fn photos(db: &StateDb, cursor: Option<&str>) -> Result<PhotoPage, DriveError> {
    let start: usize = match cursor {
        None => 0,
        Some(c) => c.parse().map_err(|_| DriveError::Failed("cursor"))?,
    };
    let mut rows: Vec<FileEntry> = live_rows(db)?
        .into_iter()
        .filter(|r| !r.is_dir() && is_photo_or_video(&leaf_name(r)))
        .collect();
    rows.sort_by(|a, b| b.modified_at.cmp(&a.modified_at).then_with(|| a.path.cmp(&b.path)));
    let total = rows.len();
    let mut entries = Vec::new();
    for entry in rows.into_iter().skip(start).take(PHOTOS_PAGE) {
        let item = item_for_entry(db, &entry)?;
        entries.push(PhotoEntry {
            taken_at: item.modified_at.clone(),
            item,
        });
    }
    let end = start + entries.len();
    Ok(PhotoPage {
        entries,
        next: (end < total).then(|| end.to_string()),
    })
}

/// `drive_search`: names from the local index (the quick-open index), matched rows only.
pub(crate) fn search(db: &StateDb, query: &str) -> Result<Vec<DriveItem>, DriveError> {
    if query.trim().is_empty() {
        return Ok(Vec::new());
    }
    let index = crate::desktop_search::build_local_index(db).map_err(|_| DriveError::Failed("search index"))?;
    let response = crate::desktop_search::query_local_index(&index, query, 50, false);
    let mut out = Vec::new();
    for hit in response.results {
        if let Ok(entry) = resolve(db, &hit.file_id) {
            out.push(item_for_entry(db, &entry)?);
        }
    }
    Ok(out)
}

fn role_for_bits(bits: i64) -> ShareRole {
    if bits & state_db::PERMISSION_WRITE != 0 {
        ShareRole::Editor
    } else {
        ShareRole::Viewer
    }
}

fn role_for_label(label: &str) -> ShareRole {
    match label {
        "write" | "edit" | "editor" | "admin" => ShareRole::Editor,
        _ => ShareRole::Viewer,
    }
}

/// `drive_shared`, the "with me" half: the shared roots in the mirror, each with its owner.
pub(crate) fn shared_with_me(db: &StateDb) -> Result<Vec<ShareEntry>, DriveError> {
    let contracts = db
        .list_contract_states_by_namespace(Namespace::SharedWithMe)
        .map_err(DriveError::db)?;
    let mut out = Vec::new();
    for contract in contracts.into_iter().filter(|c| c.parent_id.is_none()) {
        let Some(entry) = db.get_file(&contract.file_id).map_err(DriveError::db)? else {
            continue;
        };
        if entry.status == FileStatus::Trashing {
            continue;
        }
        let item = item_for_entry(db, &entry)?;
        let people = contract
            .owner_email
            .iter()
            .map(|owner| SharePerson {
                name: owner.clone(),
                email: Some(owner.clone()),
                role: role_for_bits(contract.permission_bits),
            })
            .collect();
        out.push(ShareEntry {
            item,
            direction: ShareDirection::WithMe,
            people,
            has_link: false,
        });
    }
    Ok(out)
}

/// `drive_shared`, the "by me" half: the invites the server lists as sent, joined to the mirror for
/// the decrypted name (the invite's own name field is ciphertext and is never read here). Revoked
/// and declined invites are left out; an item not in the mirror is left out.
pub(crate) fn shared_by_me(db: &StateDb, sent: &serde_json::Value) -> Result<Vec<ShareEntry>, DriveError> {
    let invites = sent
        .get("invites")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let mut order: Vec<String> = Vec::new();
    let mut people: HashMap<String, Vec<SharePerson>> = HashMap::new();
    for invite in &invites {
        let status = invite.get("status").and_then(|v| v.as_str()).unwrap_or("");
        if matches!(status, "revoked" | "declined" | "expired") {
            continue;
        }
        let (Some(file_id), Some(email)) = (
            invite.get("file_id").and_then(|v| v.as_str()),
            invite.get("recipient_email").and_then(|v| v.as_str()),
        ) else {
            continue;
        };
        let role = role_for_label(invite.get("permission").and_then(|v| v.as_str()).unwrap_or(""));
        if !people.contains_key(file_id) {
            order.push(file_id.to_string());
        }
        people.entry(file_id.to_string()).or_default().push(SharePerson {
            name: email.to_string(),
            email: Some(email.to_string()),
            role,
        });
    }
    let mut out = Vec::new();
    for file_id in order {
        let Ok(entry) = resolve(db, &file_id) else { continue };
        let mut item = item_for_entry(db, &entry)?;
        let list = people.remove(&file_id).unwrap_or_default();
        item.shared = SharedState::People {
            people: list
                .iter()
                .map(|p| Person {
                    name: p.name.clone(),
                    initials: initials(&p.name),
                })
                .collect(),
        };
        out.push(ShareEntry {
            item,
            direction: ShareDirection::ByMe,
            people: list,
            has_link: false,
        });
    }
    Ok(out)
}

/// `drive_trash`: the server's trash list (names already decrypted in Rust), with where each item was.
pub(crate) fn trash_entries(db: Option<&StateDb>, raw: Vec<crate::account_dto::DesktopTrashItem>) -> Vec<TrashEntry> {
    raw.into_iter()
        .map(|t| {
            let parent = t
                .parent_id
                .as_deref()
                .and_then(|p| db.and_then(|db| db.get_file(p).ok().flatten()));
            let original_path = match &parent {
                Some(p) if !p.path.trim_matches('/').is_empty() => format!("{ROOT_NAME}/{}", p.path.trim_matches('/')),
                _ => ROOT_NAME.to_string(),
            };
            let item = DriveItem {
                has_thumbnail: !t.is_folder && has_thumbnail(&t.name),
                mime: (!t.is_folder)
                    .then(|| mime_for_name(&t.name).map(str::to_string))
                    .flatten(),
                id: t.id,
                parent_id: t.parent_id,
                is_folder: t.is_folder,
                size: (!t.is_folder).then_some(t.size_bytes.max(0)),
                child_count: None,
                modified_at: t.updated_at.clone(),
                shared: SharedState::None,
                local: None,
                read_only: None,
                name: t.name,
            };
            TrashEntry {
                item,
                trashed_at: t.updated_at,
                original_path,
            }
        })
        .collect()
}

/// `drive_versions`: the server's version list (`list_file_versions`'s payload) as the window shows it.
pub(crate) fn versions_from_payload(body: &serde_json::Value) -> Result<Vec<VersionEntry>, DriveError> {
    let list = body
        .get("versions")
        .and_then(|v| v.as_array())
        .ok_or(DriveError::Failed("versions"))?;
    let current = body.get("current_version").and_then(|v| v.as_i64());
    let mut out: Vec<VersionEntry> = list
        .iter()
        .filter_map(|v| {
            let id = v.get("id").and_then(|x| x.as_str())?.to_string();
            let number = v.get("version_number").and_then(|x| x.as_i64());
            Some(VersionEntry {
                id,
                created_at: v
                    .get("created_at")
                    .and_then(|x| x.as_str())
                    .unwrap_or_default()
                    .to_string(),
                size: v.get("size_bytes").and_then(|x| x.as_i64()).unwrap_or(0),
                current: number.is_some() && number == current,
            })
        })
        .collect();
    if current.is_none()
        && let Some(first) = out.first_mut()
    {
        first.current = true;
    }
    Ok(out)
}

/// Files, Photos and Backups bytes from the mirror (the meter's split). `used` and `quota` are the
/// account's, from the server; the split is what this PC's copy knows.
pub(crate) fn storage_split(db: &StateDb) -> Result<(i64, i64, i64), DriveError> {
    let backups: Vec<String> = db
        .backups_roots()
        .map_err(DriveError::db)?
        .into_iter()
        .map(|(_, p)| p.trim_matches('/').to_lowercase())
        .filter(|p| !p.is_empty())
        .collect();
    let (mut files, mut photos, mut backup_bytes) = (0i64, 0i64, 0i64);
    for row in live_rows(db)?.iter().filter(|r| !r.is_dir()) {
        let path = row.path.trim_matches('/').to_lowercase();
        let size = row.size_bytes.max(0);
        if backups.iter().any(|b| path == *b || path.starts_with(&format!("{b}/"))) {
            backup_bytes += size;
        } else if is_photo_or_video(&leaf_name(row)) {
            photos += size;
        } else {
            files += size;
        }
    }
    Ok((files, photos, backup_bytes))
}

/// The counts the status line shows, read through indexed counts.
pub(crate) struct SyncCounts {
    pub(crate) on_device: u32,
    pub(crate) online_only: u32,
    pub(crate) moving: u32,
    pub(crate) conflicts: u32,
    pub(crate) on_hold: u32,
}

pub(crate) fn sync_counts(db: &StateDb) -> SyncCounts {
    let count = |s| db.count_by_status(s).unwrap_or(0);
    SyncCounts {
        on_device: count(FileStatus::Local),
        online_only: count(FileStatus::CloudOnly),
        moving: count(FileStatus::Downloading) + count(FileStatus::Uploading),
        conflicts: count(FileStatus::Conflict),
        on_hold: db.count_operations_on_hold().unwrap_or(0),
    }
}

/// Files kept on this device (pinned, own or inherited). Reads every row, so the emitter asks only
/// after a change.
pub(crate) fn available_offline_count(db: &StateDb) -> u32 {
    db.file_overview_rows()
        .map(|rows| {
            rows.iter()
                .filter(|(e, pinned)| *pinned && e.item_kind == ItemKind::File)
                .count() as u32
        })
        .unwrap_or(0)
}

/// `drive_sync_summary`'s phase, from the engine's mirrored state and the counts.
pub(crate) fn sync_phase(signed_in: bool, engine_raw: &str, paused: bool, counts: &SyncCounts) -> SyncPhase {
    if !signed_in {
        return SyncPhase::Stopped;
    }
    if paused || engine_raw == "paused" {
        return SyncPhase::Paused;
    }
    match engine_raw {
        "offline" => SyncPhase::Offline,
        "error" | "stopped" => SyncPhase::Stopped,
        "running" | "idle" | "syncing" => {
            if counts.conflicts > 0 {
                SyncPhase::NeedsDecision
            } else if engine_raw == "syncing" || counts.moving > 0 {
                SyncPhase::Syncing
            } else {
                SyncPhase::UpToDate
            }
        }
        _ => SyncPhase::Checking,
    }
}

pub(crate) fn sync_summary(phase: SyncPhase, counts: &SyncCounts, available_offline: u32) -> SyncSummary {
    SyncSummary {
        phase,
        on_device: counts.on_device,
        available_offline,
        online_only: counts.online_only,
        pending: counts.moving + counts.on_hold,
    }
}
