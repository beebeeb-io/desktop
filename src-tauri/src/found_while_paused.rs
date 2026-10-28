//! Task 2267 (fix round 2): what the person decides about the changes Beebeeb found while sync was paused. The
//! ownership proof (`own_data_proof.rs`) holds every queued change it cannot tie to the account's proven data as
//! `found_while_paused`; Settings › Sync shows them in one entry with two actions, and neither deletes anything:
//!
//! - "Upload to my Beebeeb" releases them (`StateDb::release_found_while_paused`).
//! - "Keep on this PC only" (the lead's ruling (a)) moves each item made on this PC that a held change acts on out of
//!   the Beebeeb folder, into `Beebeeb - kept on this PC\YYYY-MM-DD\` next to it, and forgets its rows and queued
//!   changes:
//!   1. On the same volume only, by rename only: an item on another volume, or one whose rename fails, stays where it
//!      is, still held, and the person is told. Never copy-then-delete.
//!   2. Never over a name already there: " (2)", " (3)" … before the extension.
//!   3. Disk first, then the rows and changes in one transaction, per item, with a journal written before the move,
//!      so a crash in between is finished (moved) or undone (not moved) at the next start ([`recover`]).
//!   4. A change to an item the server already has (a file in a shared folder, say) cannot be kept off Beebeeb by a
//!      move: it stays held. So does an item whose upload had already begun, or one whose bytes are not all on this
//!      PC.
//!
//! Names and paths are never logged; the result carries counts and the one folder the person is shown.
#![cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]

use crate::state_db::{ProofOp, ProofRow, StateDb};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// The folder next to the Beebeeb folder that kept items move into (plain hyphen, lead ruling (a) 1).
pub const KEPT_FOLDER_NAME: &str = "Beebeeb - kept on this PC";

/// What "Keep on this PC only" did, for Settings › Sync (`src/windows/foundWhilePaused.ts`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct KeepResult {
    /// Files inside the items moved.
    pub moved_files: usize,
    /// Items moved (a folder counts once).
    pub moved_items: usize,
    /// The dated folder they moved into; `None` when nothing moved.
    pub kept_path: Option<String>,
    /// Items that could not be moved: they stay where they are, still held.
    pub not_moved: usize,
    /// Held changes to items the server already has (or whose upload had begun): still held.
    pub stays_held: usize,
}

/// One item to move: the topmost row made on this PC that held changes act on, with every row under it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeepItem {
    pub root_id: String,
    /// Its path under the sync root, as `files.path` holds it.
    pub rel_path: String,
    /// The item's row and every row under it.
    pub ids: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeepPlan {
    pub items: Vec<KeepItem>,
    /// Held changes that no move can keep off Beebeeb.
    pub stays_held: usize,
}

/// The move in progress, kept in `sync_state` (`StateDb::keep_journal`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Journal {
    item: KeepItem,
    /// Where the item is being moved.
    target: String,
    /// The dated folder it moves into.
    kept_dir: String,
}

/// What [`recover`] found at a start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recovery {
    /// No move was in progress.
    Nothing,
    /// The item had moved: its rows and changes are forgotten now.
    Finished,
    /// The item had not moved (or the move cannot be told): it stays where it is, still held.
    Cleared,
}

/// Which items the held changes act on, and which held changes no move can keep off Beebeeb. A held change acts on
/// its own row, else on the folder it acts in; the item is the topmost row made on this PC above that. A change whose
/// row the server named, or that has no row, stays held; so do the changes of an item that holds (or shares its
/// path with) a row the server named, or a row whose upload had begun (`StateDb::files_with_upload_begun`).
pub fn plan(rows: &[ProofRow], held: &[ProofOp], upload_begun: &HashSet<String>) -> KeepPlan {
    let tree = Tree::new(rows);
    let mut plan = KeepPlan::default();
    let mut roots: Vec<&str> = Vec::new();
    let mut held_of: HashMap<&str, usize> = HashMap::new();
    for op in held {
        let start = [op.file_id.as_deref(), op.parent_id.as_deref()]
            .into_iter()
            .flatten()
            .find_map(|id| tree.local.get(id).copied());
        let Some(start) = start.filter(|row| !row.server_named) else {
            plan.stays_held += 1;
            continue;
        };
        let mut top = start;
        let mut seen: HashSet<&str> = HashSet::from([top.file_id.as_str()]);
        while let Some(parent) = tree.parent_of(top).and_then(|id| tree.local.get(id).copied()) {
            if parent.server_named || !seen.insert(parent.file_id.as_str()) {
                break;
            }
            top = parent;
        }
        let root = top.file_id.as_str();
        if !held_of.contains_key(root) {
            roots.push(root);
        }
        *held_of.entry(root).or_default() += 1;
    }
    for root in roots {
        let top = tree.local[root];
        let ids = tree.subtree(top);
        let server_inside = ids.iter().any(|id| tree.local[id.as_str()].server_named);
        let begun = ids.iter().any(|id| upload_begun.contains(id));
        if server_inside || begun || top.path.trim_matches('/').is_empty() {
            plan.stays_held += held_of[root];
            continue;
        }
        plan.items.push(KeepItem { root_id: root.to_string(), rel_path: top.path.clone(), ids });
    }
    plan
}

/// The local rows, indexed by id, by path (the server's row wins where two share one) and by folder.
struct Tree<'a> {
    rows: &'a [ProofRow],
    local: HashMap<&'a str, &'a ProofRow>,
    by_path: HashMap<&'a str, &'a ProofRow>,
    children: HashMap<&'a str, Vec<&'a ProofRow>>,
}

impl<'a> Tree<'a> {
    fn new(rows: &'a [ProofRow]) -> Self {
        let local = rows.iter().map(|row| (row.file_id.as_str(), row)).collect();
        let by_path = rows
            .iter()
            .filter(|row| !row.server_named)
            .chain(rows.iter().filter(|row| row.server_named))
            .map(|row| (row.path.trim_matches('/'), row))
            .collect();
        let mut tree = Tree { rows, local, by_path, children: HashMap::new() };
        let mut children: HashMap<&'a str, Vec<&'a ProofRow>> = HashMap::new();
        for row in rows {
            if let Some(parent) = tree.parent_of(row) {
                children.entry(parent).or_default().push(row);
            }
        }
        tree.children = children;
        tree
    }

    /// The folder `row` is in: its parent link, or, for a row without one, the row at its path's folder.
    fn parent_of(&self, row: &'a ProofRow) -> Option<&'a str> {
        if let Some(parent) = row.parent_id.as_deref() {
            return Some(parent);
        }
        let (folder, _) = row.path.trim_matches('/').rsplit_once('/')?;
        self.by_path.get(folder).map(|parent| parent.file_id.as_str())
    }

    /// `top` and every row under it: by parent links, and by path (the same path, or one below it; NTFS compares
    /// names without case), so a row is found however it is linked.
    fn subtree(&self, top: &'a ProofRow) -> Vec<String> {
        let mut found: Vec<&str> = vec![top.file_id.as_str()];
        let mut seen: HashSet<&str> = HashSet::from([top.file_id.as_str()]);
        let mut next = 0;
        while next < found.len() {
            for child in self.children.get(found[next]).into_iter().flatten() {
                if seen.insert(child.file_id.as_str()) {
                    found.push(child.file_id.as_str());
                }
            }
            next += 1;
        }
        let top_path = top.path.trim_matches('/').to_lowercase();
        for row in self.rows {
            let path = row.path.trim_matches('/').to_lowercase();
            let below = path == top_path || path.strip_prefix(&top_path).is_some_and(|rest| rest.starts_with('/'));
            if below && seen.insert(row.file_id.as_str()) {
                found.push(row.file_id.as_str());
            }
        }
        found.into_iter().map(str::to_string).collect()
    }
}

/// "Keep on this PC only", for every change held as `found_while_paused`. `today` names the dated folder
/// (`YYYY-MM-DD`); `rename` moves one item and never replaces (production: [`rename_no_replace`]).
pub fn keep(
    db: &StateDb,
    sync_root: &Path,
    today: &str,
    rename: &dyn Fn(&Path, &Path) -> std::io::Result<()>,
) -> Result<KeepResult, String> {
    recover(db, sync_root)?;
    let rows = db.own_data_proof_rows().map_err(|e| format!("read the local rows: {e}"))?;
    let held = db.found_while_paused_ops().map_err(|e| format!("read the held changes: {e}"))?;
    let begun = db.files_with_upload_begun().map_err(|e| format!("read the uploads: {e}"))?;
    let plan = plan(&rows, &held, &begun);
    let mut result = KeepResult { stays_held: plan.stays_held, ..KeepResult::default() };
    let Some(parent) = sync_root.parent().filter(|_| !plan.items.is_empty()) else {
        result.not_moved = plan.items.len();
        return Ok(result);
    };
    let kept_dir = parent.join(KEPT_FOLDER_NAME).join(today);
    for item in plan.items {
        let source = match crate::engine_bridge::local_file_path_under_sync_root(sync_root, item.rel_path.trim_matches('/')) {
            Ok(source) if source != sync_root && std::fs::symlink_metadata(&source).is_ok() => source,
            _ => {
                result.not_moved += 1;
                continue;
            }
        };
        if !all_bytes_here(&source) {
            tracing::warn!("keep: an item's bytes are not all on this PC; it stays where it is, held");
            result.not_moved += 1;
            continue;
        }
        // Moved out of the sync root, a Cloud Files placeholder stays one (measured: the test below), so the item is
        // made plain first. Its bytes are all here (checked above), so reverting loses nothing.
        #[cfg(target_os = "windows")]
        if let Err(error) = revert_placeholders(&source) {
            tracing::warn!(kind = ?error.kind(), os = ?error.raw_os_error(), "keep: an item could not be made plain; it stays where it is, held");
            result.not_moved += 1;
            continue;
        }
        let files = count_files(&source);
        let Some(name) = source.file_name().map(|name| name.to_string_lossy().into_owned()) else {
            result.not_moved += 1;
            continue;
        };
        if let Err(error) = std::fs::create_dir_all(&kept_dir) {
            tracing::warn!(kind = ?error.kind(), "keep: the kept folder could not be made; the item stays, held");
            result.not_moved += 1;
            continue;
        }
        let journal = Journal {
            item,
            target: unique_target(&kept_dir, &name).to_string_lossy().into_owned(),
            kept_dir: kept_dir.to_string_lossy().into_owned(),
        };
        let encoded = serde_json::to_string(&journal).map_err(|e| format!("write the keep journal: {e}"))?;
        db.set_keep_journal(&encoded).map_err(|e| format!("write the keep journal: {e}"))?;
        if let Err(error) = rename(&source, Path::new(&journal.target)) {
            tracing::warn!(kind = ?error.kind(), os = ?error.raw_os_error(), "keep: an item was not moved; it stays where it is, held");
            db.clear_keep_journal().map_err(|e| format!("clear the keep journal: {e}"))?;
            result.not_moved += 1;
            continue;
        }
        finish(db, sync_root, &journal)?;
        result.moved_items += 1;
        result.moved_files += files;
        result.kept_path = Some(journal.kept_dir);
    }
    Ok(result)
}

/// After the move: forget the item's rows and changes in one transaction, then remove the staged copies its uploads
/// held. A copy in the Beebeeb folder or the kept folder is a person's file and is never removed.
fn finish(db: &StateDb, sync_root: &Path, journal: &Journal) -> Result<(), String> {
    let staged = db
        .finish_keep(&journal.item.ids, &journal.kept_dir)
        .map_err(|e| format!("forget the kept item: {e}"))?;
    let kept_root = Path::new(&journal.kept_dir).parent().unwrap_or(Path::new(&journal.kept_dir));
    for copy in staged.iter().map(Path::new) {
        if copy.is_absolute() && !inside(copy, sync_root) && !inside(copy, kept_root) {
            crate::staged_payload::remove_or_log(db, copy);
        }
    }
    Ok(())
}

/// At a start, before sync runs: finish or undo the move a crash interrupted (lead ruling (a) 4). Moved (the target
/// is there and the source is not): finish. Anything else (not moved, or both or neither there): the journal goes and
/// the item stays held, so nothing is forgotten that is not out of the Beebeeb folder.
pub fn recover(db: &StateDb, sync_root: &Path) -> Result<Recovery, String> {
    let Some(raw) = db.keep_journal().map_err(|e| format!("read the keep journal: {e}"))? else {
        return Ok(Recovery::Nothing);
    };
    let journal = serde_json::from_str::<Journal>(&raw).ok();
    let moved = journal.as_ref().is_some_and(|journal| {
        let source_there = crate::engine_bridge::local_file_path_under_sync_root(sync_root, journal.item.rel_path.trim_matches('/'))
            .map_or(true, |source| std::fs::symlink_metadata(source).is_ok());
        !source_there && std::fs::symlink_metadata(&journal.target).is_ok()
    });
    match journal.filter(|_| moved) {
        Some(journal) => {
            finish(db, sync_root, &journal)?;
            Ok(Recovery::Finished)
        }
        None => {
            db.clear_keep_journal().map_err(|e| format!("clear the keep journal: {e}"))?;
            Ok(Recovery::Cleared)
        }
    }
}

/// Make every Cloud Files placeholder under `path` (children first, then `path`) a plain file or folder, keeping its
/// bytes. A link (a symlink or junction) is left as it is and not followed.
#[cfg(target_os = "windows")]
fn revert_placeholders(path: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::fs::MetadataExt;
    use windows::Win32::Storage::CloudFilters::{
        CF_OPEN_FILE_FLAG_EXCLUSIVE, CF_OPEN_FILE_FLAG_WRITE_ACCESS, CF_REVERT_FLAG_NONE, CfCloseHandle,
        CfOpenFileWithOplock, CfRevertPlaceholder,
    };
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    const NOT_A_CLOUD_FILE: windows::core::HRESULT = windows::core::HRESULT(0x8007_0178_u32 as i32);
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        return Ok(());
    }
    if metadata.is_dir() {
        for entry in std::fs::read_dir(path)? {
            revert_placeholders(&entry?.path())?;
        }
    }
    if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0 {
        return Ok(());
    }
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: `wide` outlives both calls; the handle is closed on every path.
    unsafe {
        let handle = CfOpenFileWithOplock(
            windows::core::PCWSTR(wide.as_ptr()),
            CF_OPEN_FILE_FLAG_EXCLUSIVE | CF_OPEN_FILE_FLAG_WRITE_ACCESS,
        )
        .map_err(|e| std::io::Error::from_raw_os_error(e.code().0 & 0xFFFF))?;
        let reverted = CfRevertPlaceholder(handle, CF_REVERT_FLAG_NONE, None);
        CfCloseHandle(handle);
        match reverted {
            Ok(()) => Ok(()),
            Err(error) if error.code() == NOT_A_CLOUD_FILE => Ok(()),
            Err(error) => Err(std::io::Error::from_raw_os_error(error.code().0 & 0xFFFF)),
        }
    }
}

/// Whether `path` is `dir` or under it, ignoring case (NTFS).
fn inside(path: &Path, dir: &Path) -> bool {
    let (path, dir) = (path.to_string_lossy().to_lowercase(), dir.to_string_lossy().to_lowercase());
    let dir = dir.trim_end_matches(['/', '\\']);
    path == dir || path.strip_prefix(dir).is_some_and(|rest| rest.starts_with(['/', '\\']))
}

/// The first of `name`, `stem (2).ext`, `stem (3).ext` … that does not exist in `dir` (lead ruling (a) 3). A name
/// that starts with its only dot has no extension.
pub fn unique_target(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if std::fs::symlink_metadata(&first).is_err() {
        return first;
    }
    let (stem, extension) = match name.rfind('.') {
        Some(0) | None => (name, ""),
        Some(dot) => (&name[..dot], &name[dot..]),
    };
    (2..)
        .map(|n| dir.join(format!("{stem} ({n}){extension}")))
        .find(|candidate| std::fs::symlink_metadata(candidate).is_err())
        .expect("an unused name")
}

/// Every file under `path` (or 1 for a file), without following links.
fn count_files(path: &Path) -> usize {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => std::fs::read_dir(path)
            .map(|entries| entries.flatten().map(|entry| count_files(&entry.path())).sum())
            .unwrap_or(0),
        Ok(_) => 1,
        Err(_) => 0,
    }
}

/// Whether every byte of every file under `path` is on this PC: no Cloud Files placeholder in it still fetches its
/// bytes on access (`RECALL_ON_DATA_ACCESS`, `RECALL_ON_OPEN`, `OFFLINE`). Read from the attributes only, so nothing
/// is fetched to answer.
fn all_bytes_here(path: &Path) -> bool {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::fs::MetadataExt;
        const NOT_HERE: u32 = 0x0040_0000 | 0x0004_0000 | 0x0000_1000;
        let Ok(metadata) = std::fs::symlink_metadata(path) else {
            return false;
        };
        if metadata.file_attributes() & NOT_HERE != 0 {
            return false;
        }
        !metadata.is_dir()
            || std::fs::read_dir(path).is_ok_and(|entries| {
                entries.into_iter().all(|entry| entry.is_ok_and(|entry| all_bytes_here(&entry.path())))
            })
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = path;
        true
    }
}

/// Move `from` to `to` without replacing anything at `to`, on the same volume only: Windows `MoveFileExW` without
/// `MOVEFILE_REPLACE_EXISTING` or `MOVEFILE_COPY_ALLOWED` (another volume fails with `ERROR_NOT_SAME_DEVICE`).
/// Elsewhere (tests): refuse an existing target, then `rename`, which fails across devices.
pub fn rename_no_replace(from: &Path, to: &Path) -> std::io::Result<()> {
    #[cfg(target_os = "windows")]
    {
        crate::engine_bridge::move_no_replace(from, to)
    }
    #[cfg(not(target_os = "windows"))]
    {
        if std::fs::symlink_metadata(to).is_ok() {
            return Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, "the target exists"));
        }
        std::fs::rename(from, to)
    }
}

/// Today's local date, as the dated folder is named.
pub fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account_binding::Identity;
    use crate::state_db::{FileEntry, FileStatus, ItemKind, Namespace, OperationKind, PendingOperation};
    use std::sync::Arc;

    const TODAY: &str = "2026-10-28";

    /// A PC: `<temp>\Beebeeb` is the sync root, its database in `<temp>\state`, a staging folder in `<temp>\staging`.
    struct Pc {
        db: Arc<StateDb>,
        root: PathBuf,
        staging: PathBuf,
        temp: tempfile::TempDir,
    }

    impl Pc {
        fn new() -> Self {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().join("Beebeeb");
            let staging = temp.path().join("staging");
            for dir in [&root, &staging, &temp.path().join("state")] {
                std::fs::create_dir_all(dir).unwrap();
            }
            let db = Arc::new(StateDb::open(temp.path().join("state").join("state.db")).unwrap());
            Self { db, root, staging, temp }
        }

        fn kept_dir(&self) -> PathBuf {
            self.temp.path().join(KEPT_FOLDER_NAME).join(TODAY)
        }

        /// A row: `server` says the server named it (a version), `None` parent at the top level.
        fn row(&self, id: &str, path: &str, parent: Option<&str>, folder: bool, server: bool, namespace: Namespace) {
            let kind = if folder { ItemKind::Folder } else { ItemKind::File };
            self.db
                .upsert_file_with_contract(
                    &FileEntry {
                        file_id: id.to_string(),
                        path: path.to_string(),
                        status: FileStatus::Local,
                        size_bytes: 3,
                        modified_at: 1_700_000_000,
                        content_hash: None,
                        remote_updated_at: 0,
                        parent_id: parent.map(str::to_string),
                        item_kind: kind.clone(),
                    },
                    |contract| {
                        contract.parent_id = parent.map(str::to_string);
                        contract.namespace = namespace;
                        contract.item_kind = kind;
                        contract.current_version = if server { 3 } else { 0 };
                    },
                )
                .unwrap();
        }

        /// A new folder made on this PC, on disk and in the database, with its create queued.
        fn new_folder(&self, id: &str, path: &str, parent: Option<&str>) {
            std::fs::create_dir_all(self.on_disk(path)).unwrap();
            self.row(id, path, parent, true, false, Namespace::MyFiles);
            self.queue(&format!("op-{id}"), OperationKind::CreateFolder, Some(id), parent, None);
        }

        /// A new file made on this PC, on disk and in the database, with its upload queued from a staged copy.
        fn new_file(&self, id: &str, path: &str, parent: Option<&str>, bytes: &[u8]) {
            std::fs::write(self.on_disk(path), bytes).unwrap();
            self.row(id, path, parent, false, false, Namespace::MyFiles);
            let staged = self.staging.join(format!("{id}.staged"));
            std::fs::write(&staged, bytes).unwrap();
            self.db.track_staged_payload(&staged.to_string_lossy(), None, false).unwrap();
            self.queue(&format!("op-{id}"), OperationKind::UploadVersion, Some(id), parent, Some(&staged));
        }

        fn queue(&self, op_id: &str, kind: OperationKind, file_id: Option<&str>, parent: Option<&str>, payload: Option<&Path>) {
            self.db
                .enqueue_operation(&PendingOperation {
                    op_id: op_id.to_string(),
                    kind,
                    file_id: file_id.map(str::to_string),
                    parent_id: parent.map(str::to_string),
                    target_path: Some("x".to_string()),
                    metadata_json: None,
                    payload_path: payload.map(|path| path.to_string_lossy().into_owned()),
                    base_version: None,
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
        }

        /// Hold every queued change for review, as an adoption that proved none of them does.
        fn hold_all(&self) {
            let owner = Identity::new(Some("user-owner"), Some("owner@example.eu"));
            assert!(
                self.db
                    .adopt_owner_holding(None, &owner, &HashSet::new(), 5)
                    .unwrap()
                    .is_some_and(|held| held > 0),
                "precondition: changes held"
            );
        }

        fn on_disk(&self, path: &str) -> PathBuf {
            self.root.join(path.replace('/', std::path::MAIN_SEPARATOR_STR))
        }

        fn held(&self) -> Vec<String> {
            let mut held: Vec<String> = self.db.found_while_paused_ops().unwrap().into_iter().map(|op| op.op_id).collect();
            held.sort();
            held
        }

        fn row_ids(&self) -> Vec<String> {
            let mut ids: Vec<String> = self.db.list_files().unwrap().into_iter().map(|row| row.file_id).collect();
            ids.sort();
            ids
        }

        fn ops(&self) -> usize {
            self.db.own_data_proof_ops().unwrap().len()
        }

        fn keep(&self) -> KeepResult {
            keep(&self.db, &self.root, TODAY, &rename_no_replace).expect("keep runs")
        }
    }

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| item.to_string()).collect()
    }

    /// The lead's case: a new folder (with a file and a subfolder) at the top level and a new file at the top level,
    /// both held. Keep moves both next to the Beebeeb folder, in the dated folder, and forgets their rows and changes:
    /// nothing is uploaded and nothing is deleted. The server's rows are untouched.
    #[test]
    fn keep_moves_new_items_out_of_the_beebeeb_folder_and_forgets_them() {
        let pc = Pc::new();
        pc.row("server-file", "Report.pdf", None, false, true, Namespace::MyFiles);
        pc.new_folder("new-folder", "Trip", None);
        pc.new_folder("new-sub", "Trip/Day 1", Some("new-folder"));
        pc.new_file("new-photo", "Trip/Day 1/photo.jpg", Some("new-sub"), b"jpg");
        pc.new_file("new-note", "Trip/note.txt", Some("new-folder"), b"txt");
        pc.new_file("new-top", "todo.txt", None, b"todo");
        pc.hold_all();
        let staged: Vec<PathBuf> = ["new-photo", "new-note", "new-top"]
            .iter()
            .map(|id| pc.staging.join(format!("{id}.staged")))
            .collect();

        let result = pc.keep();

        let kept = pc.kept_dir();
        assert_eq!(
            result,
            KeepResult {
                moved_files: 3,
                moved_items: 2,
                kept_path: Some(kept.to_string_lossy().into_owned()),
                not_moved: 0,
                stays_held: 0,
            }
        );
        assert_eq!(std::fs::read(kept.join("Trip").join("Day 1").join("photo.jpg")).unwrap(), b"jpg");
        assert_eq!(std::fs::read(kept.join("Trip").join("note.txt")).unwrap(), b"txt");
        assert_eq!(std::fs::read(kept.join("todo.txt")).unwrap(), b"todo");
        assert!(!pc.on_disk("Trip").exists() && !pc.on_disk("todo.txt").exists(), "out of the Beebeeb folder");
        assert!(!kept.starts_with(&pc.root), "next to the Beebeeb folder, never in it");
        assert_eq!(pc.row_ids(), strings(&["server-file"]), "only the server's row is left");
        assert_eq!(pc.ops(), 0, "nothing queued, nothing to upload");
        assert!(staged.iter().all(|path| !path.exists()), "the staged copies of kept files are removed");
        assert_eq!(pc.db.keep_journal().unwrap(), None);
        assert_eq!(pc.db.last_kept_dir().unwrap(), Some(kept.to_string_lossy().into_owned()));
    }

    /// A name already in the dated folder is never overwritten: the item gets " (2)" (before the extension), and the
    /// one already there keeps its bytes.
    #[test]
    fn keep_never_overwrites_a_name_already_in_the_kept_folder() {
        let pc = Pc::new();
        let kept = pc.kept_dir();
        std::fs::create_dir_all(kept.join("Trip")).unwrap();
        std::fs::write(kept.join("Trip").join("old.txt"), b"kept earlier").unwrap();
        std::fs::write(kept.join("todo.txt"), b"kept earlier").unwrap();
        pc.new_folder("new-folder", "Trip", None);
        pc.new_file("new-top", "todo.txt", None, b"new");
        pc.hold_all();

        let result = pc.keep();

        assert_eq!((result.moved_items, result.not_moved), (2, 0), "{result:?}");
        assert!(kept.join("Trip (2)").is_dir());
        assert_eq!(std::fs::read(kept.join("todo (2).txt")).unwrap(), b"new");
        assert_eq!(std::fs::read(kept.join("todo.txt")).unwrap(), b"kept earlier");
        assert_eq!(std::fs::read(kept.join("Trip").join("old.txt")).unwrap(), b"kept earlier");
    }

    #[test]
    fn a_unique_target_counts_up_before_the_extension() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(unique_target(dir.path(), "a.txt"), dir.path().join("a.txt"));
        std::fs::write(dir.path().join("a.txt"), b"").unwrap();
        std::fs::write(dir.path().join("a (2).txt"), b"").unwrap();
        assert_eq!(unique_target(dir.path(), "a.txt"), dir.path().join("a (3).txt"));
        std::fs::create_dir(dir.path().join("Folder")).unwrap();
        assert_eq!(unique_target(dir.path(), "Folder"), dir.path().join("Folder (2)"));
        std::fs::write(dir.path().join(".env"), b"").unwrap();
        assert_eq!(unique_target(dir.path(), ".env"), dir.path().join(".env (2)"));
        std::fs::write(dir.path().join("archive.tar.gz"), b"").unwrap();
        assert_eq!(unique_target(dir.path(), "archive.tar.gz"), dir.path().join("archive.tar (2).gz"));
    }

    /// Lead ruling (a) 2: an item on another volume (or whose rename fails for any reason) is never copied. It stays
    /// where it is, its changes stay held, its rows stay, and the person is told.
    #[test]
    fn an_item_that_cannot_be_renamed_stays_where_it_is_and_stays_held() {
        let pc = Pc::new();
        pc.new_folder("new-folder", "Trip", None);
        pc.new_file("new-note", "Trip/note.txt", Some("new-folder"), b"txt");
        pc.new_file("new-top", "todo.txt", None, b"todo");
        pc.hold_all();
        let held = pc.held();
        const ERROR_NOT_SAME_DEVICE: i32 = 17;
        let other_volume = |from: &Path, to: &Path| {
            if from.ends_with("Trip") {
                Err(std::io::Error::from_raw_os_error(ERROR_NOT_SAME_DEVICE))
            } else {
                rename_no_replace(from, to)
            }
        };

        let result = keep(&pc.db, &pc.root, TODAY, &other_volume).unwrap();

        assert_eq!((result.moved_items, result.moved_files, result.not_moved), (1, 1, 1), "{result:?}");
        assert_eq!(std::fs::read(pc.on_disk("Trip/note.txt")).unwrap(), b"txt", "left where it was");
        assert!(!pc.kept_dir().join("Trip").exists(), "nothing copied");
        assert_eq!(pc.held(), strings(&["op-new-folder", "op-new-note"]), "still held");
        assert!(held.contains(&"op-new-top".to_string()));
        assert_eq!(pc.row_ids(), strings(&["new-folder", "new-note"]));
        assert_eq!(pc.db.keep_journal().unwrap(), None);
    }

    /// Lead ruling (a) 4: a crash after the move and before the transaction. The next start finds the item moved and
    /// finishes: its rows and changes are forgotten, nothing is queued again, nothing is lost, and the scan of the
    /// Beebeeb folder finds nothing to upload.
    #[test]
    fn a_crash_between_the_move_and_the_transaction_is_finished_at_the_next_start() {
        let pc = Pc::new();
        pc.new_folder("new-folder", "Trip", None);
        pc.new_file("new-note", "Trip/note.txt", Some("new-folder"), b"txt");
        pc.hold_all();
        let crash_after_moving = |from: &Path, to: &Path| -> std::io::Result<()> {
            rename_no_replace(from, to).unwrap();
            panic!("the process ends here");
        };
        let crashed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            keep(&pc.db, &pc.root, TODAY, &crash_after_moving)
        }));
        assert!(crashed.is_err(), "precondition: the keep was interrupted");
        assert!(pc.db.keep_journal().unwrap().is_some(), "precondition: the journal says a move was in progress");
        assert_eq!(pc.ops(), 2, "precondition: the transaction did not run");

        assert_eq!(recover(&pc.db, &pc.root).unwrap(), Recovery::Finished);

        assert_eq!(std::fs::read(pc.kept_dir().join("Trip").join("note.txt")).unwrap(), b"txt", "nothing lost");
        assert_eq!(pc.row_ids(), Vec::<String>::new());
        assert_eq!(pc.ops(), 0, "nothing queued again");
        assert_eq!(pc.db.keep_journal().unwrap(), None);
        assert_eq!(recover(&pc.db, &pc.root).unwrap(), Recovery::Nothing, "once");
        scan(&pc);
        assert_eq!(pc.ops(), 0, "the scan finds nothing to upload");
    }

    /// A crash before the move: the next start finds the item where it was and leaves it held.
    #[test]
    fn a_crash_before_the_move_leaves_the_item_held() {
        let pc = Pc::new();
        pc.new_folder("new-folder", "Trip", None);
        pc.hold_all();
        let crash_before_moving = |_: &Path, _: &Path| -> std::io::Result<()> { panic!("the process ends here") };
        let crashed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            keep(&pc.db, &pc.root, TODAY, &crash_before_moving)
        }));
        assert!(crashed.is_err() && pc.db.keep_journal().unwrap().is_some(), "precondition");

        assert_eq!(recover(&pc.db, &pc.root).unwrap(), Recovery::Cleared);

        assert!(pc.on_disk("Trip").is_dir());
        assert_eq!(pc.held(), strings(&["op-new-folder"]));
        assert_eq!(pc.db.keep_journal().unwrap(), None);
    }

    /// The kept folder is next to the Beebeeb folder, so the scan of the Beebeeb folder never picks it up: after a
    /// keep, a scan queues nothing and writes no row.
    #[test]
    fn the_scan_never_picks_up_the_kept_folder() {
        let pc = Pc::new();
        pc.new_folder("new-folder", "Trip", None);
        pc.new_file("new-note", "Trip/note.txt", Some("new-folder"), b"txt");
        pc.new_file("new-top", "todo.txt", None, b"todo");
        pc.hold_all();
        assert_eq!(pc.keep().moved_items, 2);

        scan(&pc);
        scan(&pc);

        assert_eq!(pc.ops(), 0);
        assert_eq!(pc.row_ids(), Vec::<String>::new());
        assert!(pc.kept_dir().join("todo.txt").is_file());
    }

    /// A held change to an item the server already has (an edit of a file in a folder shared with this account) cannot
    /// be kept off Beebeeb by a move: it stays held, and the file stays. A new file made in that shared folder is the
    /// person's own, made here: it moves.
    #[test]
    fn a_change_to_an_item_the_server_has_stays_held_and_a_new_file_next_to_it_moves() {
        let pc = Pc::new();
        std::fs::create_dir_all(pc.on_disk("Shared")).unwrap();
        std::fs::write(pc.on_disk("Shared/plan.txt"), b"theirs").unwrap();
        pc.row("shared-folder", "Shared", None, true, true, Namespace::SharedWithMe);
        pc.row("shared-file", "Shared/plan.txt", Some("shared-folder"), false, true, Namespace::SharedWithMe);
        pc.queue("op-edit", OperationKind::UploadVersion, Some("shared-file"), None, None);
        pc.new_file("new-in-shared", "Shared/mine.txt", Some("shared-folder"), b"mine");
        pc.hold_all();

        let result = pc.keep();

        assert_eq!((result.moved_items, result.moved_files, result.stays_held), (1, 1, 1), "{result:?}");
        assert_eq!(std::fs::read(pc.kept_dir().join("mine.txt")).unwrap(), b"mine");
        assert_eq!(std::fs::read(pc.on_disk("Shared/plan.txt")).unwrap(), b"theirs");
        assert_eq!(pc.held(), strings(&["op-edit"]));
        assert_eq!(pc.row_ids(), strings(&["shared-file", "shared-folder"]));
    }

    /// A folder made here that holds a row the server named is not only this PC's: its changes stay held, nothing in
    /// it moves.
    #[test]
    fn a_new_folder_holding_a_server_row_stays_held() {
        let pc = Pc::new();
        pc.new_folder("new-folder", "Trip", None);
        std::fs::write(pc.on_disk("Trip/synced.txt"), b"s").unwrap();
        pc.row("server-file", "Trip/synced.txt", Some("new-folder"), false, true, Namespace::MyFiles);
        pc.hold_all();

        let result = pc.keep();

        assert_eq!((result.moved_items, result.stays_held), (0, 1), "{result:?}");
        assert!(pc.on_disk("Trip/synced.txt").is_file());
        assert_eq!(pc.held(), strings(&["op-new-folder"]));
    }

    /// An item part of which the server may already hold is not "not uploaded": it stays held. Here a file in the
    /// new folder landed (its finalization replaced its queued change), and another file has a resumable upload.
    #[test]
    fn an_item_whose_upload_had_begun_stays_held() {
        let pc = Pc::new();
        pc.new_folder("new-folder", "Trip", None);
        pc.new_file("landed", "Trip/landed.txt", Some("new-folder"), b"l");
        pc.new_file("resuming", "todo.txt", None, b"todo");
        pc.hold_all();
        pc.db
            .put_upload_finalization(&crate::state_db::UploadFinalization {
                op_id: "op-landed".to_string(),
                local_file_id: "landed".to_string(),
                server_file_id: "server-id".to_string(),
                target_path: "Trip/landed.txt".to_string(),
                payload_path: "staged".to_string(),
                stamped: false,
            })
            .unwrap();
        pc.db
            .put_upload_resume(&crate::state_db::UploadResume {
                op_id: "op-resuming".to_string(),
                payload_path: "staged".to_string(),
                payload_size: 4,
                payload_mtime_ns: 1,
                upload_session_id: "session".to_string(),
                server_file_id: "server-id-2".to_string(),
                object_version_id: "version".to_string(),
                chunk_size_bytes: 4,
                chunk_count: 1,
                acked_chunks: 0,
                metadata_applied: false,
                is_create: true,
                completed_version: None,
                completed_object_version_id: None,
                completed_mime_type: None,
            })
            .unwrap();

        let result = pc.keep();

        assert_eq!((result.moved_items, result.stays_held), (0, 2), "{result:?}");
        assert!(pc.on_disk("Trip/landed.txt").is_file() && pc.on_disk("todo.txt").is_file());
        assert_eq!(pc.held(), strings(&["op-new-folder", "op-resuming"]));
    }

    /// A held change whose item has no row at all (it is gone) has nothing to move: it stays held, and nothing else
    /// changes.
    #[test]
    fn a_change_whose_item_has_no_row_stays_held() {
        let pc = Pc::new();
        pc.queue("op-orphan", OperationKind::TrashFile, Some("gone"), None, None);
        pc.new_file("new-top", "todo.txt", None, b"todo");
        pc.hold_all();
        let result = pc.keep();
        assert_eq!((result.moved_items, result.stays_held), (1, 1), "{result:?}");
        assert_eq!(pc.held(), strings(&["op-orphan"]));
    }

    /// A staged copy is removed only when it is not in the Beebeeb folder or the kept folder: a person's file is never
    /// deleted, whatever a queued change names as its copy.
    #[test]
    fn a_payload_inside_the_beebeeb_folder_is_never_deleted() {
        let pc = Pc::new();
        std::fs::write(pc.on_disk("todo.txt"), b"todo").unwrap();
        std::fs::write(pc.on_disk("keep-me.txt"), b"mine").unwrap();
        pc.row("new-top", "todo.txt", None, false, false, Namespace::MyFiles);
        pc.queue("op-new-top", OperationKind::UploadVersion, Some("new-top"), None, Some(&pc.on_disk("keep-me.txt")));
        pc.hold_all();
        assert_eq!(pc.keep().moved_items, 1);
        assert_eq!(std::fs::read(pc.on_disk("keep-me.txt")).unwrap(), b"mine");
    }

    /// Nothing held: nothing moves, no folder is made.
    #[test]
    fn nothing_held_moves_nothing() {
        let pc = Pc::new();
        pc.new_file("new-top", "todo.txt", None, b"todo");
        assert_eq!(pc.keep(), KeepResult::default());
        assert!(!pc.temp.path().join(KEPT_FOLDER_NAME).exists());
        assert!(pc.on_disk("todo.txt").is_file());
    }

    /// Self-review F3: a change queued after the hold to an item the person has not decided about yet is held too: the
    /// item saved again, and a new file in a held new folder. A change to anything else is due, and a held change that is
    /// queued again stays held.
    #[test]
    fn a_change_queued_after_the_hold_to_a_held_item_is_held_too() {
        let pc = Pc::new();
        pc.new_folder("new-folder", "Trip", None);
        pc.new_file("new-top", "todo.txt", None, b"todo");
        pc.queue("op-no-row", OperationKind::CreateFolder, None, None, None);
        pc.hold_all();
        assert_eq!(pc.held(), strings(&["op-new-folder", "op-new-top", "op-no-row"]), "precondition");

        pc.queue("op-saved-again", OperationKind::UploadVersion, Some("new-top"), None, None);
        pc.queue("op-new-inside", OperationKind::UploadVersion, Some("new-inside"), Some("new-folder"), None);
        pc.queue("op-new-top", OperationKind::UploadVersion, Some("new-top"), None, None);
        pc.queue("op-no-row", OperationKind::CreateFolder, None, None, None);
        pc.queue("op-elsewhere", OperationKind::UploadVersion, Some("own-file"), Some("own-folder"), None);

        assert_eq!(
            pc.held(),
            strings(&["op-new-folder", "op-new-inside", "op-new-top", "op-no-row", "op-saved-again"]),
            "held: what acts on a held item, and a held change queued again"
        );
        let due: Vec<String> = pc.db.list_due_operations(i64::MAX).unwrap().into_iter().map(|op| op.op_id).collect();
        assert_eq!(due, strings(&["op-elsewhere"]), "a change to anything else is due");
    }

    /// The real scan of the Beebeeb folder, as the watcher runs it.
    fn scan(pc: &Pc) {
        let api = Arc::new(crate::api_client::ApiClient::new("https://api.invalid".into(), "token".into(), [17u8; 32]));
        let bridge = crate::engine_bridge::EngineBridge::new(pc.db.clone(), api);
        crate::watcher::run_one_scan(&bridge, &pc.root);
    }

    // ---- Windows: a file made in the Beebeeb folder is an unsynced Cloud Files placeholder ----

    /// A sync root registered for the test, unregistered at the end.
    #[cfg(target_os = "windows")]
    struct Registered(PathBuf);

    #[cfg(target_os = "windows")]
    impl Drop for Registered {
        fn drop(&mut self) {
            let _ = crate::windows_cf::unregister_sync_root(&self.0);
        }
    }

    /// A connection to it with no callbacks, disconnected at the end. Nothing answers a fetch, so anything that asks for
    /// a cloud-only file's bytes blocks until Cloud Files gives up (about a minute).
    /// The connection's key, and its callback table, which Cloud Files reads for as long as the root is connected.
    #[cfg(target_os = "windows")]
    struct Connected(
        windows::Win32::Storage::CloudFilters::CF_CONNECTION_KEY,
        #[allow(dead_code)] Box<[windows::Win32::Storage::CloudFilters::CF_CALLBACK_REGISTRATION; 1]>,
    );

    #[cfg(target_os = "windows")]
    impl Drop for Connected {
        fn drop(&mut self) {
            // SAFETY: the key of the connection `connect` made, disconnected once.
            let _ = unsafe { windows::Win32::Storage::CloudFilters::CfDisconnectSyncRoot(self.0) };
        }
    }

    /// Register and connect `root` as a Cloud Files sync root for one test (a tuple drops in order: disconnected
    /// first, then unregistered).
    #[cfg(target_os = "windows")]
    fn sync_root(root: &Path) -> (Connected, Registered) {
        use std::os::windows::ffi::OsStrExt;
        use windows::Win32::Storage::CloudFilters::{
            CF_CALLBACK_REGISTRATION, CF_CALLBACK_TYPE_NONE, CF_CONNECT_FLAG_NONE, CfConnectSyncRoot,
        };
        crate::windows_cf::register_sync_root(root).unwrap();
        let registered = Registered(root.to_path_buf());
        let table = Box::new([CF_CALLBACK_REGISTRATION { Type: CF_CALLBACK_TYPE_NONE, Callback: None }]);
        let wide: Vec<u16> = root.as_os_str().encode_wide().chain(Some(0)).collect();
        // SAFETY: the path and the table outlive the call.
        let key = unsafe { CfConnectSyncRoot(windows::core::PCWSTR(wide.as_ptr()), table.as_ptr(), None, CF_CONNECT_FLAG_NONE) }
            .unwrap();
        (Connected(key, table), registered)
    }

    /// The watcher turns a new file into an unsynced placeholder when it queues its upload. Kept, it must be a plain
    /// file the person can open, outside any sync root, with its bytes.
    #[cfg(target_os = "windows")]
    #[test]
    fn a_kept_unsynced_placeholder_is_a_plain_file_with_its_bytes() {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        let pc = Pc::new();
        let _root = sync_root(&pc.root);
        pc.new_folder("new-folder", "Trip", None);
        pc.new_file("new-note", "Trip/note.txt", Some("new-folder"), b"made here");
        pc.new_file("new-top", "todo.txt", None, b"todo");
        for (path, id) in [("Trip/note.txt", "new-note"), ("todo.txt", "new-top")] {
            crate::windows_cf::placeholders::convert_to_unsynced_placeholder(&pc.on_disk(path), id).unwrap();
            let attributes = std::fs::symlink_metadata(pc.on_disk(path)).unwrap().file_attributes();
            assert_ne!(attributes & FILE_ATTRIBUTE_REPARSE_POINT, 0, "precondition: a placeholder");
        }
        pc.hold_all();

        let result = pc.keep();

        assert_eq!((result.moved_items, result.not_moved), (2, 0), "{result:?}");
        for (path, bytes) in [(pc.kept_dir().join("Trip").join("note.txt"), &b"made here"[..]), (pc.kept_dir().join("todo.txt"), b"todo")] {
            let attributes = std::fs::symlink_metadata(&path).unwrap().file_attributes();
            assert_eq!(attributes & FILE_ATTRIBUTE_REPARSE_POINT, 0, "a plain file");
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
        }
    }

    /// An item holding a file whose bytes are not all on this PC (a cloud-only placeholder) is not moved: it would
    /// leave the Beebeeb folder without them. Nothing asks for its bytes to find that out: made plain, it would be
    /// downloaded first (here, where nothing answers, that blocks for about a minute), and it stays cloud-only.
    #[cfg(target_os = "windows")]
    #[test]
    fn an_item_whose_bytes_are_not_all_here_is_not_moved() {
        let pc = Pc::new();
        let _root = sync_root(&pc.root);
        pc.new_folder("new-folder", "Trip", None);
        assert!(
            crate::windows_cf::placeholders::create_placeholder(&pc.on_disk("Trip"), "cloud", "cloud.bin", 1 << 20, 1_700_000_000, false)
                .unwrap()
        );
        pc.row("cloud", "Trip/cloud.bin", Some("new-folder"), false, false, Namespace::MyFiles);
        pc.hold_all();

        let started = std::time::Instant::now();
        let result = pc.keep();
        let took = started.elapsed();

        assert_eq!((result.moved_items, result.not_moved), (0, 1), "{result:?}");
        assert!(pc.on_disk("Trip").is_dir());
        assert_eq!(pc.held(), strings(&["op-new-folder"]));
        assert!(took < std::time::Duration::from_secs(20), "nothing asked for the bytes: {took:?}");
        let attributes = std::os::windows::fs::MetadataExt::file_attributes(
            &std::fs::symlink_metadata(pc.on_disk("Trip/cloud.bin")).unwrap(),
        );
        assert_ne!(attributes & 0x0040_0000, 0, "still cloud-only (RECALL_ON_DATA_ACCESS)");
    }
}
