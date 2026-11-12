//! `vault_tree` items split out of `lib.rs`.

use super::*;

/// Resolve and dehydrate the local, unpinned files under each newly-excluded
/// folder. Returns total bytes reclaimed. Shared by Windows (real CF
/// dehydrate) and non-Windows (DB-only flip) via the inner cfg blocks —
/// mirrors [`free_up_space_blocking`]. Best-effort: a missing sync root /
/// DB is `Ok(0)`, a per-file dehydrate failure logs and continues.
pub(super) fn dehydrate_excluded_subtrees_blocking(newly_excluded: &[String]) -> Result<u64, String> {
    let cfg = DesktopConfig::load()?;
    #[cfg(target_os = "windows")]
    let Some(sync_root) = cfg.sync_root.clone() else {
        return Ok(0);
    };
    #[cfg(not(target_os = "windows"))]
    if cfg.sync_root.is_none() {
        return Ok(0);
    }
    let db_path = state_paths::beebeeb_state_dir()?.join(state_paths::STATE_DB_FILENAME);
    if !db_path.exists() {
        return Ok(0);
    }
    let db = state_db::StateDb::open(&db_path).map_err(|e| format!("open state.db: {e}"))?;

    // Flat (id, parent_id, is_folder) view → resolve the union of subtree files.
    let entries = db.list_files().map_err(|e| format!("list files: {e}"))?;
    let flat: Vec<(String, Option<String>, bool)> = entries
        .iter()
        .map(|e| {
            (
                e.file_id.clone(),
                e.parent_id.clone(),
                e.item_kind == state_db::ItemKind::Folder,
            )
        })
        .collect();
    let file_ids = subtree_file_ids(&flat, newly_excluded);
    if file_ids.is_empty() {
        return Ok(0);
    }

    #[cfg(target_os = "windows")]
    {
        dehydrate_excluded_subtrees_windows(&db, &sync_root, &entries, &file_ids)
    }
    #[cfg(not(target_os = "windows"))]
    {
        // Non-Windows: no in-process CF dehydrate. Flip the local+unpinned
        // rows to cloud_only so the DB reflects the exclusion; the engine's
        // own cache eviction reclaims the bytes on its schedule. Pinned rows
        // are excluded by the predicate inside `mark_cloud_only_after_dehydrate`.
        let local_unpinned: Vec<String> = entries
            .iter()
            .filter(|e| file_ids.iter().any(|f| f == &e.file_id))
            .filter(|e| e.status == state_db::FileStatus::Local)
            .filter(|e| {
                db.get_file_contract_state(&e.file_id)
                    .ok()
                    .flatten()
                    .map(|c| c.effective_pin_state() != state_db::PinState::Pinned)
                    .unwrap_or(true)
            })
            .map(|e| e.file_id.clone())
            .collect();
        let now = now_unix_seconds();
        if let Err(e) = db.mark_cloud_only_after_dehydrate(&local_unpinned, now) {
            tracing::warn!(error = %e, "set_selective_sync: could not flip excluded files cloud_only");
        }
        Ok(0)
    }
}

/// Windows reclaim path for newly-excluded subtrees. Mirrors
/// [`free_up_space_windows`] exactly: reconstruct each file's placeholder
/// path under the sync root, path-safety-guard it, skip effectively-pinned
/// files (with a TOCTOU re-check immediately before the destructive
/// dehydrate), `CfDehydratePlaceholder`, then mark cloud_only ONLY for the
/// files that actually dehydrated.
#[cfg(target_os = "windows")]
pub(super) fn dehydrate_excluded_subtrees_windows(
    db: &state_db::StateDb,
    sync_root: &std::path::Path,
    entries: &[state_db::FileEntry],
    file_ids: &[String],
) -> Result<u64, String> {
    use std::collections::HashSet;
    let target: HashSet<&str> = file_ids.iter().map(String::as_str).collect();
    let allowed_roots = free_up_space_allowed_roots(crate::APP, sync_root);
    let mut bytes_freed: u64 = 0;
    let mut dehydrated_ids: Vec<String> = Vec::new();

    for entry in entries {
        if !target.contains(entry.file_id.as_str()) {
            continue;
        }
        // Only hydrated files have bytes to reclaim.
        if entry.status != state_db::FileStatus::Local {
            continue;
        }
        // Skip effectively-pinned files: an explicit pin wins over the exclusion.
        if let Ok(Some(contract)) = db.get_file_contract_state(&entry.file_id) {
            if contract.effective_pin_state() == state_db::PinState::Pinned {
                tracing::debug!(file_id = %entry.file_id, "set_selective_sync: file pinned; keeping local despite exclusion");
                continue;
            }
        }
        // Task 2407 (core 2404): never another file this name reaches (an 8.3 short name; `canonicalize` below would
        // follow it to that file's long name). A check that cannot finish keeps the file local.
        let Some(local_path) = crate::name_alias::reclaim_place(sync_root, &entry.path) else {
            continue;
        };
        let Ok(canonical) = local_path.canonicalize() else {
            continue;
        };
        if !allowed_roots.iter().any(|root| canonical.starts_with(root)) {
            continue;
        }
        // Task 1640: an unsent change stays on disk; see `free_up_space_windows`.
        if !crate::windows_cf::placeholders::local_bytes_agreed(db, &entry.file_id, &canonical) {
            tracing::warn!(file_id = %entry.file_id, "set_selective_sync: the file holds a change that is not sent; keeping it local");
            continue;
        }
        match crate::windows_cf::placeholders::dehydrate_placeholder(&canonical) {
            Ok(freed) => {
                bytes_freed = bytes_freed.saturating_add(freed);
                dehydrated_ids.push(entry.file_id.clone());
            }
            Err(error) => {
                tracing::warn!(file_id = %entry.file_id, error = %error, "set_selective_sync: CfDehydratePlaceholder failed; keeping file local");
            }
        }
    }

    let now = now_unix_seconds();
    if let Err(e) = db.mark_cloud_only_after_dehydrate(&dehydrated_ids, now) {
        tracing::warn!(error = %e, "set_selective_sync: could not mark dehydrated files cloud_only");
    }
    Ok(bytes_freed)
}

/// One node in the SelectiveSync folder tree (task 0090 → wave-2 nesting).
///
/// Each node is a FOLDER the user can toggle. `children` holds nested
/// sub-folders (built from `parent_id` linkage), and the byte/count
/// fields are AGGREGATES over the whole subtree's *files* (folders carry
/// no bytes of their own), so a row can render "12 files · 4.2 GB · 1.1 GB
/// on disk" without the TS side walking the tree.
///
/// JSON shape serialized to the frontend (serde default camel? — NO,
/// serde keeps snake_case here; the existing `is_folder` field already
/// ships snake_case and the TS view reads `is_folder`, so we stay
/// consistent):
/// ```json
/// {
///   "id": "uuid",
///   "name": "Documents",
///   "is_folder": true,
///   "excluded": false,
///   "size_bytes": 4509715660,
///   "file_count": 12,
///   "on_disk_bytes": 1181116006,
///   "children": [ { ...same shape... } ]
/// }
/// ```
/// - `id` / `name` / `is_folder`: as before. `is_folder` is always `true`
///   for a node we emit (we only surface folders as toggle rows); files
///   are folded into the aggregates, never emitted as their own node.
/// - `excluded`: `true` if this folder id ∈ `get_selective_sync()`.
/// - `size_bytes`: sum of `size_bytes` of every file at or below this
///   folder (logical plaintext bytes).
/// - `file_count`: number of files at or below this folder.
/// - `on_disk_bytes`: sum of `size_bytes` of files in the subtree that are
///   currently hydrated locally (`status == Local`) — drives the
///   "X on disk / Y online-only" footer.
/// - `children`: nested sub-folders, sorted by name (case-insensitive)
///   then id for a stable order.
///
/// `name` is taken from the state_db `path` leaf segment (the
/// already-resolved, decrypted relative path the engine wrote when it
/// synced the folder), falling back to a stable id-derived label
/// (`"Folder 7f3a1c…"`) when the path is empty. When we fall back to the
/// top-level API path (empty DB), names are decrypted via
/// [`owner_keys::display_name`] as before.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub(super) struct VaultItem {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) is_folder: bool,
    pub(super) excluded: bool,
    pub(super) size_bytes: i64,
    pub(super) file_count: i64,
    pub(super) on_disk_bytes: i64,
    pub(super) children: Vec<VaultItem>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub(super) struct DesktopSharedRoot {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) owner_email: Option<String>,
    pub(super) permission: String,
    pub(super) kind: String,
    pub(super) finder_path: Option<String>,
}

/// Lightweight, decode-free row used by the pure tree-builder. Decoupling
/// from `state_db::FileEntry` keeps [`build_vault_tree`] trivially unit
/// testable without constructing full DB rows.
#[derive(Debug, Clone)]
pub(super) struct VaultEntryRow {
    pub(super) id: String,
    pub(super) parent_id: Option<String>,
    pub(super) is_folder: bool,
    /// Display label (folder leaf name); ignored for files.
    pub(super) name: String,
    /// Logical plaintext size in bytes (files only; folders contribute 0).
    pub(super) size_bytes: i64,
    /// Whether this file is currently hydrated on disk (`status == Local`).
    /// Always `false` for folders.
    pub(super) on_disk: bool,
    /// The row's path. Rows of the same kind at one path are one entry in the tree (0.8.15, [`one_entry_per_path`]).
    pub(super) path: String,
    /// Whether the server knows the row (`current_version` or `remote_updated_at` set). A folder this PC created and
    /// never sent has neither.
    pub(super) server_known: bool,
    /// Whether the row's status is `local`, the status a folder this PC created and never sent keeps.
    pub(super) local_status: bool,
}

/// Mutable accumulator for one folder while we aggregate the subtree.
pub(super) struct FolderAgg {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) parent_id: Option<String>,
    pub(super) children: Vec<String>,
    /// Files DIRECTLY in this folder (not descendants).
    pub(super) direct_size: i64,
    pub(super) direct_count: i64,
    pub(super) direct_on_disk: i64,
}

pub(super) use beebeeb_engine::vault_tree::{folder_fallback_label, folder_leaf_name};

pub(super) fn shared_permission_label(bits: i64) -> &'static str {
    if bits & state_db::PERMISSION_OWNER != 0 {
        "admin"
    } else if bits & state_db::PERMISSION_WRITE != 0 {
        "write"
    } else {
        "read"
    }
}

pub(super) fn local_path_under_sync_root(sync_root: &Path, rel_path: &str) -> Option<String> {
    let rel = rel_path.trim_matches('/');
    if rel.is_empty()
        || rel
            .split('/')
            .any(|segment| segment.is_empty() || segment == "." || segment == "..")
    {
        return None;
    }
    Some(
        sync_root
            .join(rel.replace('/', std::path::MAIN_SEPARATOR_STR))
            .to_string_lossy()
            .into_owned(),
    )
}

pub(super) fn shared_roots_from_db(
    db: &state_db::StateDb,
    sync_root: Option<&Path>,
) -> Result<Vec<DesktopSharedRoot>, String> {
    let contracts = db
        .list_contract_states_by_namespace(state_db::Namespace::SharedWithMe)
        .map_err(|e| format!("list shared roots: {e}"))?;
    let mut roots = Vec::new();

    for contract in contracts.into_iter().filter(|contract| contract.parent_id.is_none()) {
        let Some(entry) = db
            .get_file(&contract.file_id)
            .map_err(|e| format!("read shared root row: {e}"))?
        else {
            continue;
        };
        roots.push(DesktopSharedRoot {
            id: entry.file_id.clone(),
            name: folder_leaf_name(&entry.path).unwrap_or_else(|| folder_fallback_label(&entry.file_id)),
            owner_email: contract.owner_email,
            permission: shared_permission_label(contract.permission_bits).to_string(),
            kind: match contract.item_kind {
                state_db::ItemKind::Folder => "folder".to_string(),
                state_db::ItemKind::File => "file".to_string(),
            },
            finder_path: sync_root.and_then(|root| local_path_under_sync_root(root, &entry.path)),
        });
    }

    Ok(roots)
}

#[tauri::command]
pub(super) fn list_shared_roots() -> Result<Vec<DesktopSharedRoot>, String> {
    let cfg = DesktopConfig::load()?;
    let sync_root = cfg.sync_root.clone();
    let Some(db) = state_db_for_config(&cfg)? else {
        return Ok(Vec::new());
    };
    shared_roots_from_db(&db, sync_root.as_deref())
}

/// One state DB row as the tree builder reads it.
pub(super) fn vault_entry_row(entry: &state_db::TreeRow) -> VaultEntryRow {
    VaultEntryRow {
        id: entry.file_id.clone(),
        parent_id: entry.parent_id.clone(),
        is_folder: entry.item_kind == state_db::ItemKind::Folder,
        name: folder_leaf_name(&entry.path).unwrap_or_else(|| folder_fallback_label(&entry.file_id)),
        size_bytes: entry.size_bytes,
        on_disk: entry.status == state_db::FileStatus::Local,
        path: entry.path.clone(),
        server_known: entry.current_version > 0 || entry.remote_updated_at > 0,
        local_status: entry.status == state_db::FileStatus::Local,
    }
}

/// One entry per path in the Selective sync tree (0.8.15). Display only.
///
/// On Guus's PC 2,828 folder paths had two rows: a folder this PC created on 4 Oct whose `create_folder` never landed
/// (`local`, no server version, a locally minted id, its operation paused) and the server's folder at the same path,
/// which arrived later with a snapshot. The tree listed both: the same folder twice, once with 0 B and 0 files.
///
/// Here a row the server does not know folds into the row that stays at its path: the server's (a row the server
/// knows, then one whose status is not `local`, then the smallest id, so the choice is stable). The folded row is not
/// shown, its children hang under the row that stays, and nothing is counted twice. Two rows the server knows are two
/// server items, and both stay. Rows with an empty path are left alone.
///
/// The state DB and the queue are untouched: nothing is deleted, re-parented on disk or resumed. 0.8.16 repairs the data.
pub(super) fn one_entry_per_path(rows: &[VaultEntryRow]) -> Vec<VaultEntryRow> {
    use std::cmp::Reverse;
    use std::collections::HashMap;

    let mut at_path: HashMap<(bool, &str), Vec<&VaultEntryRow>> = HashMap::new();
    for row in rows {
        let path = row.path.trim_matches('/');
        if !path.is_empty() {
            at_path.entry((row.is_folder, path)).or_default().push(row);
        }
    }
    // Folded row id -> the id of the row that stays at its path. A row that stays never folds, so one lookup resolves.
    let mut folded_into: HashMap<&str, &str> = HashMap::new();
    for same_path in at_path.values().filter(|rows| rows.len() > 1) {
        let Some(stays) = same_path
            .iter()
            .max_by_key(|row| (row.server_known, !row.local_status, Reverse(row.id.as_str())))
        else {
            continue;
        };
        for row in same_path {
            if row.id != stays.id && !row.server_known {
                folded_into.insert(row.id.as_str(), stays.id.as_str());
            }
        }
    }
    rows.iter()
        .filter(|row| !folded_into.contains_key(row.id.as_str()))
        .map(|row| {
            let mut row = row.clone();
            if let Some(stays) = row.parent_id.as_deref().and_then(|parent| folded_into.get(parent)) {
                row.parent_id = Some((*stays).to_string());
            }
            row
        })
        .collect()
}

/// Build the nested SelectiveSync folder tree from a flat set of local
/// rows (already nested via `parent_id`, from task 0784).
///
/// Pure + deterministic: no DB, no network, no clock — so the unit tests
/// can drive it directly. Algorithm:
///   1. Index every FOLDER row by id into a `FolderAgg` (children linked
///      by `parent_id`); roots are folders whose `parent_id` is `None`
///      OR points at a non-folder / unknown id (defensive: a dangling
///      parent must still surface its folder as a root, never drop it).
///   2. Fold every FILE row into its parent folder's *direct* totals
///      (size / count / on-disk). A file whose parent is unknown is
///      dropped from the tree's aggregates (it has no folder to attach
///      to) — acceptable for a folder-picker view.
///   3. Recursively materialize `VaultItem`s, summing each folder's
///      direct totals with its descendants' aggregates. Cycle-safe via a
///      `visiting` guard (a corrupt parent cycle can't infinite-loop).
///   4. `excluded` is set per-node from the `excluded` id set.
///
/// Children are sorted by lowercased name then id for a stable UI order.
///
/// First, one entry per path: a folder this PC created and never sent folds into the server's at its path (0.8.15,
/// [`one_entry_per_path`]).
pub(super) fn build_vault_tree(rows: &[VaultEntryRow], excluded: &std::collections::HashSet<String>) -> Vec<VaultItem> {
    build_vault_tree_of_rows(&one_entry_per_path(rows), excluded)
}

/// [`build_vault_tree`] over the rows as they are, with no merge: what 0.8.14 showed.
pub(super) fn build_vault_tree_of_rows(
    rows: &[VaultEntryRow],
    excluded: &std::collections::HashSet<String>,
) -> Vec<VaultItem> {
    use std::collections::HashMap;

    // 1. Index folders.
    let mut folders: HashMap<String, FolderAgg> = HashMap::new();
    for row in rows {
        if row.is_folder {
            folders.entry(row.id.clone()).or_insert_with(|| FolderAgg {
                id: row.id.clone(),
                name: row.name.clone(),
                parent_id: row.parent_id.clone(),
                children: Vec::new(),
                direct_size: 0,
                direct_count: 0,
                direct_on_disk: 0,
            });
        }
    }

    // Link folder → child-folder. A folder whose parent is not a known
    // folder becomes a root (collected below).
    let folder_ids: std::collections::HashSet<String> = folders.keys().cloned().collect();
    let mut roots: Vec<String> = Vec::new();
    let child_links: Vec<(String, String)> = folders
        .values()
        .filter_map(|f| match &f.parent_id {
            Some(p) if folder_ids.contains(p) => Some((p.clone(), f.id.clone())),
            _ => None,
        })
        .collect();
    for (parent, child) in child_links {
        if let Some(p) = folders.get_mut(&parent) {
            p.children.push(child);
        }
    }
    for f in folders.values() {
        let is_root = match &f.parent_id {
            None => true,
            Some(p) => !folder_ids.contains(p),
        };
        if is_root {
            roots.push(f.id.clone());
        }
    }

    // 2. Fold files into their parent folder's direct totals.
    for row in rows {
        if row.is_folder {
            continue;
        }
        if let Some(parent) = row.parent_id.as_ref()
            && let Some(folder) = folders.get_mut(parent)
        {
            folder.direct_size = folder.direct_size.saturating_add(row.size_bytes.max(0));
            folder.direct_count = folder.direct_count.saturating_add(1);
            if row.on_disk {
                folder.direct_on_disk = folder.direct_on_disk.saturating_add(row.size_bytes.max(0));
            }
        }
    }

    // 3 + 4. Materialize recursively with a cycle guard.
    let mut visiting: std::collections::HashSet<String> = std::collections::HashSet::new();
    roots.sort_by_key(|a| folder_sort_key(&folders, a));
    roots
        .iter()
        .filter_map(|id| materialize_folder(id, &folders, excluded, &mut visiting))
        .collect()
}

/// Sort key for a folder id: (lowercased name, id). Stable + case-insensitive.
pub(super) fn folder_sort_key(folders: &std::collections::HashMap<String, FolderAgg>, id: &str) -> (String, String) {
    match folders.get(id) {
        Some(f) => (f.name.to_lowercase(), f.id.clone()),
        None => (String::new(), id.to_string()),
    }
}

/// Recursively build a `VaultItem`, summing direct totals with descendants.
/// Returns `None` only on a cycle re-entry (defensive against corrupt data).
pub(super) fn materialize_folder(
    id: &str,
    folders: &std::collections::HashMap<String, FolderAgg>,
    excluded: &std::collections::HashSet<String>,
    visiting: &mut std::collections::HashSet<String>,
) -> Option<VaultItem> {
    let folder = folders.get(id)?;
    if !visiting.insert(id.to_string()) {
        // Already on the current path — a parent cycle. Stop here.
        return None;
    }

    let mut child_ids = folder.children.clone();
    child_ids.sort_by_key(|a| folder_sort_key(folders, a));
    let children: Vec<VaultItem> = child_ids
        .iter()
        .filter_map(|cid| materialize_folder(cid, folders, excluded, visiting))
        .collect();

    let mut size_bytes = folder.direct_size;
    let mut file_count = folder.direct_count;
    let mut on_disk_bytes = folder.direct_on_disk;
    for child in &children {
        size_bytes = size_bytes.saturating_add(child.size_bytes);
        file_count = file_count.saturating_add(child.file_count);
        on_disk_bytes = on_disk_bytes.saturating_add(child.on_disk_bytes);
    }

    visiting.remove(id);

    Some(VaultItem {
        id: folder.id.clone(),
        name: folder.name.clone(),
        is_folder: true,
        excluded: excluded.contains(&folder.id),
        size_bytes,
        file_count,
        on_disk_bytes,
        children,
    })
}

/// Folder ids that are present in `new_excluded` but NOT in `old_excluded`.
/// These are the subtrees that must be dehydrated on this `set_selective_sync`.
/// Order-stable on `new_excluded`'s order; de-duplicated.
pub(super) fn newly_excluded_ids(old_excluded: &[String], new_excluded: &[String]) -> Vec<String> {
    let old: std::collections::HashSet<&str> = old_excluded.iter().map(String::as_str).collect();
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    new_excluded
        .iter()
        .filter(|id| !old.contains(id.as_str()) && seen.insert(id.as_str()))
        .cloned()
        .collect()
}

/// Resolve the FILE ids that live at or below any of `root_folder_ids`,
/// from a flat (id, parent_id, is_folder) view of the vault. Pure mirror
/// of the recursive-CTE descent in [`state_db::StateDb::set_recursive_pin`]
/// so the diff/subtree logic is unit-testable without SQLite.
///
/// Folders are descended into but never returned (we dehydrate files, not
/// folders). Cycle-safe. The result is de-duplicated even when the
/// excluded roots overlap (one folder nested under another).
pub(super) fn subtree_file_ids(rows: &[(String, Option<String>, bool)], root_folder_ids: &[String]) -> Vec<String> {
    use std::collections::{HashMap, HashSet, VecDeque};

    // children index: parent_id → [child rows]
    let mut children: HashMap<&str, Vec<&(String, Option<String>, bool)>> = HashMap::new();
    for row in rows {
        if let Some(parent) = row.1.as_deref() {
            children.entry(parent).or_default().push(row);
        }
    }

    let mut visited: HashSet<&str> = HashSet::new();
    let mut out_files: Vec<String> = Vec::new();
    let mut out_seen: HashSet<&str> = HashSet::new();
    let mut queue: VecDeque<&str> = VecDeque::new();

    for root in root_folder_ids {
        queue.push_back(root.as_str());
    }
    while let Some(id) = queue.pop_front() {
        if !visited.insert(id) {
            continue;
        }
        if let Some(kids) = children.get(id) {
            for kid in kids {
                if kid.2 {
                    // folder — descend
                    queue.push_back(kid.0.as_str());
                } else if out_seen.insert(kid.0.as_str()) {
                    out_files.push(kid.0.clone());
                }
            }
        }
    }
    out_files
}

/// Try to decrypt a single file/folder's `name_encrypted` blob using
/// the master key. Returns `None` on any failure — caller falls back
/// to a stable id-derived label. We deliberately swallow errors here
/// because the SelectiveSync page treats these as best-effort: a
/// folder created by an older client with a missing/garbled blob, or
/// an entry encrypted under a different key, should still appear in
/// the list (with a fallback name) rather than abort the whole call.
///
/// Routes through core's resolver (`beebeeb_core::owned_keys::resolve_owned_key`),
/// the single decision path all clients share. The served id class decides
/// (task 2275, L12): an owner id's key is derived and must authenticate the
/// name, and any served attribution or envelope is ignored; a foreign id opens
/// only through its L8 envelope. Unlike the previous `decrypt_name` call this
/// covers the string, binary and read-only uppercase-repair conventions and
/// never treats a foreign row as an owner row. We deliberately swallow the
/// error to `None` so the SelectiveSync row falls back to a stable id label.
/// Build the nested SelectiveSync folder tree.
///
/// Wave-2 change (task 0090 backing): the tree is built from the LOCAL
/// state DB (`StateDb::list_files()`), which already carries the full
/// nested set (`parent_id` + `item_kind`, from task 0784) the engine has
/// synced. We deliberately do NOT recurse the API folder-by-folder — that
/// hammered the server and tripped the 429 rate limit (the 0789 problem).
/// One local SQLite read replaces N network round-trips.
///
/// Each returned [`VaultItem`] is a folder the user can toggle, with its
/// subtree's file size / count / on-disk bytes aggregated up (see the
/// struct doc for the exact JSON shape).
///
/// Returns an empty `Vec` — not `Err` — when:
///   - the user is not signed in (no in-memory session)
///   - no sync root / state DB exists yet (nothing synced)
///
/// Fallback: if the user IS signed in but the local DB is missing or has
/// no folders yet (fresh install, first sync not finished), we fall back
/// to the OLD top-level API path (`api.list_files(None)`, folders only) so
/// the picker isn't empty during the first-sync window. That fallback is
/// FLAT (no nesting, zeroed aggregates) — it is a stopgap, and the tree
/// fills in once the engine has populated state.db. An API failure in the
/// fallback still surfaces as an empty list, not an error banner.
#[tauri::command]
pub(super) async fn list_vault_folders(state: State<'_, AppState>) -> Result<Vec<VaultItem>, String> {
    session_command!(async {
        // Lift the session pieces we need without holding the mutex over any
        // async API call.
        let acct = state.active_account()?;
        let creds = {
            let guard = acct.session.lock().map_err(|_| "session mutex poisoned".to_string())?;
            guard.as_ref().map(|s| (s.token.clone(), s.master_key))
        };
        let Some((token, master_key)) = creds else {
            // Not signed in — surface as empty rather than an error.
            return Ok(Vec::new());
        };

        let excluded: std::collections::HashSet<String> = DesktopConfig::load()
            .ok()
            .and_then(|c| c.excluded_folder_ids)
            .unwrap_or_default()
            .into_iter()
            .collect();

        // Primary path: build from the local, already-nested state DB.
        if let Ok(Some(db)) = DesktopConfig::load().and_then(|cfg| state_db_for_config(&cfg))
            && let Ok(entries) = db.list_tree_rows()
        {
            let rows: Vec<VaultEntryRow> = entries.iter().map(vault_entry_row).collect();
            let tree = build_vault_tree(&rows, &excluded);
            if !tree.is_empty() {
                return Ok(tree);
            }
            // DB present but no folders yet — fall through to the API stopgap.
        }

        // Fallback: top-level API folders, FLAT, zeroed aggregates. Only used
        // until the first sync populates the local DB.
        let api = api_client::ApiClient::new(runner::api_base_url(), token, master_key);
        let raw = match api.list_files(None).await {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(error = %e, "list_vault_folders: list_files fallback failed");
                return Ok(Vec::new());
            }
        };

        let mut out: Vec<VaultItem> = Vec::with_capacity(raw.len());
        for f in &raw {
            let is_folder = f.get("is_folder").and_then(|v| v.as_bool()).unwrap_or(false);
            if !is_folder {
                continue;
            }
            let Some(id) = f.get("id").and_then(|v| v.as_str()) else {
                continue;
            };

            // Display name: prefer the decrypted `name_encrypted` blob
            // (the canonical zero-knowledge path), then fall back to a
            // server-provided plaintext `path` if present (some legacy
            // entries surface plaintext here), then to a stable, short
            // id-based label so the user can still tell folders apart.
            let display = owner_keys::display_name(f, id, &master_key)
                .or_else(|| {
                    if !f["acted_via_share_id"].is_null()
                        || !f["uploaded_by"].is_null()
                        || !f["owner_wrapped_file_key"].is_null()
                    {
                        return None;
                    }
                    let path = f.get("path").and_then(|v| v.as_str()).unwrap_or("");
                    if path.is_empty() {
                        None
                    } else {
                        Some(path.trim_start_matches('/').to_string())
                    }
                })
                .unwrap_or_else(|| folder_fallback_label(id));

            out.push(VaultItem {
                id: id.to_string(),
                name: display,
                is_folder: true,
                excluded: excluded.contains(id),
                size_bytes: 0,
                file_count: 0,
                on_disk_bytes: 0,
                children: Vec::new(),
            });
        }
        Ok(out)
    })
}

#[tauri::command]
pub(super) async fn list_remote_tree(state: State<'_, AppState>) -> Result<Vec<VaultItem>, String> {
    list_vault_folders(state).await
}

#[tauri::command]
pub(super) fn set_recursive_pin(item_id: String, pinned: bool) -> Result<engine_bridge::PinUpdateOutcome, String> {
    let cfg = DesktopConfig::load()?;
    let sync_root = cfg
        .sync_root
        .ok_or_else(|| "Choose a sync folder before changing offline availability.".to_string())?;
    let db_path = state_paths::beebeeb_state_dir()?.join(state_paths::STATE_DB_FILENAME);
    if !db_path.exists() {
        return Err("Beebeeb hasn’t started syncing on this Mac yet. Try again once it has.".to_string());
    }

    let db = std::sync::Arc::new(state_db::StateDb::open(&db_path).map_err(|e| format!("open state.db: {e}"))?);
    let api = std::sync::Arc::new(api_client::ApiClient::new(
        runner::api_base_url(),
        String::new(),
        [0u8; 32],
    ));
    let bridge = engine_bridge::EngineBridge::new(crate::APP, db, api);
    // `sync_root` is threaded through so the Windows path can resolve the
    // affected placeholder(s) and set the OS-level pin state (CfSetPinState);
    // it is unused on macOS/Linux.
    bridge
        .set_recursive_pin(&sync_root, &item_id, pinned)
        .map_err(|e| format!("set recursive pin: {e}"))
}

/// Best-effort lookup of the signed-in user's email, used by
/// `repos/desktop/src/pages/Account.tsx`.
///
/// Currently returns `Ok(None)` whenever no email is reachable from
/// the daemon's session struct — which is always, because Phase 1's
/// `Session` only carries `token` + `master_key` (no email). Surfacing
/// the email here requires either:
///
///   - extending `set_session` to also push a plaintext email, or
///   - calling `GET /api/v1/me` from the Rust side and caching it
///
/// Both are tracked under the auth-persistence work; until one lands
/// the Account page falls back to a "Signed in" placeholder. The
/// command intentionally returns `Ok(None)` rather than `Err(...)` so
/// the TS side doesn't render a noisy error banner for a known gap.
#[tauri::command]
pub(super) fn account_email(state: State<'_, AppState>) -> Result<Option<String>, String> {
    let acct = state.active_account()?;
    let guard = acct.session.lock().map_err(|_| "session mutex poisoned".to_string())?;
    if let Some(email) = guard.as_ref().and_then(|s| s.email.clone()) {
        return Ok(Some(email));
    }
    drop(guard);
    Ok(acct.auth_email.lock().ok().and_then(|guard| guard.clone()))
}
