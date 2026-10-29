//! Task 2267 (fix round 2): what the person decides about the changes Beebeeb found while sync was paused. The
//! ownership proof (`own_data_proof.rs`) holds every queued change it cannot tie to the account's proven data as
//! `found_while_paused`; Settings › Sync shows them in one entry with two actions, and neither deletes anything:
//!
//! - The card first reads them by kind ([`review`]); "Upload to my Beebeeb" releases exactly the changes it showed
//!   (`StateDb::release_found_while_paused`), never one held after that read (lead review round 3, P2-2 and P3-1).
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
//!   5. Only a staged copy the journal names is removed after the move; one whose live file is gone is the only copy
//!      of those bytes and is copied into the dated folder first (lead review round 3, P3-3).
//!   6. When OneDrive or Dropbox syncs the folder the kept folder is made in, the card says so instead of "on this
//!      PC" ([`synced_by`], P3-4).
//!
//! Names and paths are never logged; the result carries counts and the one folder the person is shown.
#![cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]

use crate::state_db::{HeldChange, ProofOp, ProofRow, StateDb};
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
    /// The service that syncs the folder the kept folder is in ("OneDrive", "Dropbox"), if one does: the kept items
    /// are not uploaded to Beebeeb, but may not stay on this PC only (lead review round 3, P3-4).
    pub synced_by: Option<String>,
}

/// Task 2267 (lead review round 3, P2-2 and P3-1): what the card shows before the person decides, by kind, and the
/// held changes "Upload to my Beebeeb" releases: exactly these, never one held after the card read them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Review {
    pub new_folders: usize,
    pub new_files: usize,
    /// Changes to items that exist: new content, a rename, a move.
    pub edits: usize,
    /// Items the change moves to the trash.
    pub deletions: usize,
    /// Any change to an item in a folder shared with this account (counted here only).
    pub in_shared_folders: usize,
    pub op_ids: Vec<String>,
    /// As [`KeepResult::synced_by`], for the question before a keep.
    pub synced_by: Option<String>,
}

/// The card's counts for the held `changes`, in order.
pub fn review(changes: &[HeldChange]) -> Review {
    let mut review = Review::default();
    for change in changes {
        let count = if change.shared {
            &mut review.in_shared_folders
        } else if change.kind == "create_folder" {
            &mut review.new_folders
        } else if change.creates {
            &mut review.new_files
        } else if change.kind == "trash_file" {
            &mut review.deletions
        } else {
            &mut review.edits
        };
        *count += 1;
        review.op_ids.push(change.op_id.clone());
    }
    review
}

/// Which service syncs `dir`: the name of the first of `roots` it is in (or is), ignoring case.
pub fn synced_by(dir: &Path, roots: &[(PathBuf, &'static str)]) -> Option<&'static str> {
    roots.iter().find(|(root, _)| inside(dir, root)).map(|(_, name)| *name)
}

/// The folders Dropbox syncs, from its `info.json` (`{"personal": {"path": …}, "business": {"path": …}}`).
fn dropbox_roots(info_json: &str) -> Vec<PathBuf> {
    let Ok(serde_json::Value::Object(accounts)) = serde_json::from_str::<serde_json::Value>(info_json) else {
        return Vec::new();
    };
    accounts
        .values()
        .filter_map(|account| account.get("path")?.as_str())
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .collect()
}

/// The folders OneDrive and Dropbox sync on this PC: OneDrive's from its environment variables, Dropbox's from its
/// `info.json` in the roaming and the local application data. Whatever cannot be read is left out.
pub fn cloud_synced_roots() -> Vec<(PathBuf, &'static str)> {
    let mut roots: Vec<(PathBuf, &'static str)> = ["OneDrive", "OneDriveConsumer", "OneDriveCommercial"]
        .into_iter()
        .filter_map(std::env::var_os)
        .filter(|root| !root.is_empty())
        .map(|root| (PathBuf::from(root), "OneDrive"))
        .collect();
    for base in ["APPDATA", "LOCALAPPDATA"].into_iter().filter_map(std::env::var_os) {
        let info = PathBuf::from(base).join("Dropbox").join("info.json");
        if let Ok(raw) = std::fs::read_to_string(info) {
            roots.extend(dropbox_roots(&raw).into_iter().map(|root| (root, "Dropbox")));
        }
    }
    roots
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

/// "Keep on this PC only", for the changes held as `found_while_paused` that the card showed (`shown`, op ids from
/// [`review`]; self-review of round 3, F7). `today` names the dated folder (`YYYY-MM-DD`); `rename` moves one item and
/// never replaces (production: [`rename_no_replace`]).
pub fn keep(
    db: &StateDb,
    sync_root: &Path,
    today: &str,
    rename: &dyn Fn(&Path, &Path) -> std::io::Result<()>,
    shown: &[String],
) -> Result<KeepResult, String> {
    recover(db, sync_root)?;
    let rows = db.own_data_proof_rows().map_err(|e| format!("read the local rows: {e}"))?;
    let shown: HashSet<&str> = shown.iter().map(String::as_str).collect();
    let held: Vec<ProofOp> = db
        .found_while_paused_ops()
        .map_err(|e| format!("read the held changes: {e}"))?
        .into_iter()
        .filter(|op| shown.contains(op.op_id.as_str()))
        .collect();
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

/// After the move: keep the only copies, forget the item's rows and changes in one transaction, then remove the staged
/// copies its uploads held. Only a copy the journal names (`staged_payloads`) is removed, never one in the Beebeeb
/// folder or the kept folder (a person's file). A copy whose live file is gone holds the only copy of those bytes: it
/// is copied into the dated kept folder BEFORE the forget (self-review of round 3, F3), never over a name, and kept
/// where it is when that copy fails (lead review round 3, P3-3).
fn finish(db: &StateDb, sync_root: &Path, journal: &Journal) -> Result<(), String> {
    let staged = db
        .keep_staged_copies(&journal.item.ids)
        .map_err(|e| format!("read the kept item's staged copies: {e}"))?;
    let kept_dir = Path::new(&journal.kept_dir);
    let kept_root = kept_dir.parent().unwrap_or(kept_dir);
    let moved_from = crate::engine_bridge::local_file_path_under_sync_root(sync_root, journal.item.rel_path.trim_matches('/')).ok();
    let mut remove: Vec<&Path> = Vec::new();
    for copy in staged.iter().map(Path::new) {
        if !copy.is_absolute() || inside(copy, sync_root) || inside(copy, kept_root) {
            continue;
        }
        let source = match db.staged_payload_source(&copy.to_string_lossy()) {
            Ok(Some(source)) => source,
            Ok(None) => {
                tracing::info!("keep: a copy no journal names is left where it is");
                continue;
            }
            Err(error) => {
                tracing::warn!(%error, "keep: a staged copy's journal could not be read; the copy stays");
                continue;
            }
        };
        let live = source.as_deref().map(|source| match moved_from.as_deref() {
            Some(from) if inside(Path::new(source), from) => {
                Path::new(&journal.target).join(Path::new(source).components().skip(from.components().count()).collect::<PathBuf>())
            }
            _ => PathBuf::from(source),
        });
        if live.is_some_and(|live| std::fs::symlink_metadata(live).is_ok()) {
            remove.push(copy);
            continue;
        }
        let name = source.as_deref().and_then(|source| Path::new(source).file_name()).or_else(|| copy.file_name());
        let Some(name) = name.map(|name| name.to_string_lossy().into_owned()) else {
            continue;
        };
        match keep_only_copy(copy, kept_dir, &name) {
            Ok(()) => remove.push(copy),
            Err(error) => tracing::warn!(kind = ?error.kind(), "keep: an only copy could not be kept; it stays where it is"),
        }
    }
    #[cfg(test)]
    run_before_forget();
    db.finish_keep(&journal.item.ids, &journal.kept_dir)
        .map_err(|e| format!("forget the kept item: {e}"))?;
    for copy in remove {
        crate::staged_payload::remove_or_log(db, copy);
    }
    Ok(())
}

#[cfg(test)]
thread_local! {
    /// Runs once, on this thread, right before a keep forgets an item's rows and changes (tests only).
    static BEFORE_FORGET: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
}

/// Run `hook` at the next keep on this thread, right before it forgets an item's rows and changes.
#[cfg(test)]
fn before_forget_for_test(hook: impl FnOnce() + 'static) {
    BEFORE_FORGET.with(|slot| *slot.borrow_mut() = Some(Box::new(hook)));
}

#[cfg(test)]
fn run_before_forget() {
    if let Some(hook) = BEFORE_FORGET.with(|slot| slot.borrow_mut().take()) {
        hook();
    }
}

/// Keep the only copy `copy` in `dir` as `name`, never over a name. A file already there under one of its numbered
/// names with the same bytes is that copy, made by a keep that stopped before it forgot the item (self-review of
/// round 3, F3): it is not made twice.
fn keep_only_copy(copy: &Path, dir: &Path, name: &str) -> std::io::Result<()> {
    for candidate in numbered(dir, name) {
        match std::fs::symlink_metadata(&candidate) {
            Err(_) => return copy_no_replace(copy, &candidate),
            Ok(metadata) if metadata.is_file() && crate::staged_payload::same_bytes(copy, &candidate).unwrap_or(false) => {
                return Ok(());
            }
            Ok(_) => {}
        }
    }
    unreachable!("the numbered names never end")
}

/// Copy `from` to a new file at `to`, never over one; a partial copy is removed.
fn copy_no_replace(from: &Path, to: &Path) -> std::io::Result<()> {
    let mut source = std::fs::File::open(from)?;
    let mut target = std::fs::OpenOptions::new().write(true).create_new(true).open(to)?;
    let copied = std::io::copy(&mut source, &mut target).and_then(|_| target.sync_all());
    if copied.is_err() {
        drop(target);
        let _ = std::fs::remove_file(to);
    }
    copied
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

/// The first of `name`, `stem (2).ext`, `stem (3).ext` … that does not exist in `dir` (lead ruling (a) 3).
pub fn unique_target(dir: &Path, name: &str) -> PathBuf {
    numbered(dir, name)
        .find(|candidate| std::fs::symlink_metadata(candidate).is_err())
        .expect("an unused name")
}

/// `name`, `stem (2).ext`, `stem (3).ext` … in `dir`. A name that starts with its only dot has no extension.
fn numbered<'n>(dir: &'n Path, name: &'n str) -> impl Iterator<Item = PathBuf> + 'n {
    let (stem, extension) = match name.rfind('.') {
        Some(0) | None => (name, ""),
        Some(dot) => (&name[..dot], &name[dot..]),
    };
    std::iter::once(dir.join(name)).chain((2..).map(move |n| dir.join(format!("{stem} ({n}){extension}"))))
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
            self.db
                .track_staged_payload(&staged.to_string_lossy(), Some(&self.on_disk(path).to_string_lossy()), false)
                .unwrap();
            self.queue_create_file(&format!("op-{id}"), Some(id), parent, Some(&staged));
        }

        /// A queued change to an item that already exists (no create).
        fn queue(&self, op_id: &str, kind: OperationKind, file_id: Option<&str>, parent: Option<&str>, payload: Option<&Path>) {
            self.enqueue(op_id, kind, file_id, parent, payload, None);
        }

        /// A new file's upload, which says it creates the file, as the scan queues it (`engine_bridge`'s
        /// `"operation": "create_file"`): only then does its row count as made on this PC (lead review round 2, P2-1).
        fn queue_create_file(&self, op_id: &str, file_id: Option<&str>, parent: Option<&str>, payload: Option<&Path>) {
            self.enqueue(op_id, OperationKind::UploadVersion, file_id, parent, payload, Some(r#"{"operation":"create_file"}"#));
        }

        fn enqueue(
            &self,
            op_id: &str,
            kind: OperationKind,
            file_id: Option<&str>,
            parent: Option<&str>,
            payload: Option<&Path>,
            metadata: Option<&str>,
        ) {
            self.db
                .enqueue_operation(&PendingOperation {
                    op_id: op_id.to_string(),
                    kind,
                    file_id: file_id.map(str::to_string),
                    parent_id: parent.map(str::to_string),
                    target_path: Some("x".to_string()),
                    metadata_json: metadata.map(str::to_string),
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

        /// Keep what is held, all of it shown on the card.
        fn keep(&self) -> KeepResult {
            keep(&self.db, &self.root, TODAY, &rename_no_replace, &self.held()).expect("keep runs")
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
                synced_by: None,
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

        let result = keep(&pc.db, &pc.root, TODAY, &other_volume, &pc.held()).unwrap();

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
            keep(&pc.db, &pc.root, TODAY, &crash_after_moving, &pc.held())
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
            keep(&pc.db, &pc.root, TODAY, &crash_before_moving, &pc.held())
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
        pc.queue_create_file("op-new-top", Some("new-top"), None, Some(&pc.on_disk("keep-me.txt")));
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
        pc.queue_create_file("op-new-inside", Some("new-inside"), Some("new-folder"), None);
        pc.queue_create_file("op-new-top", Some("new-top"), None, None);
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

    /// Lead review round 3, P2-2: the card counts the held changes by kind (a change in a folder shared with this
    /// account under that kind only) and names exactly the changes it counted.
    #[test]
    fn the_review_counts_the_held_changes_by_kind() {
        let pc = Pc::new();
        pc.row("server-file", "Report.pdf", None, false, true, Namespace::MyFiles);
        pc.row("server-old", "Old.pdf", None, false, true, Namespace::MyFiles);
        pc.row("shared-folder", "Shared", None, true, true, Namespace::SharedWithMe);
        pc.row("shared-file", "Shared/Plan.txt", Some("shared-folder"), false, true, Namespace::SharedWithMe);
        pc.new_folder("new-folder", "Trip", None);
        pc.new_file("new-photo", "Trip/photo.jpg", Some("new-folder"), b"jpg");
        pc.queue("op-edit", OperationKind::UploadVersion, Some("server-file"), None, None);
        pc.queue("op-rename", OperationKind::RenameFile, Some("server-file"), None, None);
        pc.queue("op-trash", OperationKind::TrashFile, Some("server-old"), None, None);
        pc.queue_create_file("op-new-in-shared", Some("new-in-shared"), Some("shared-folder"), None);
        pc.queue("op-shared-edit", OperationKind::UploadVersion, Some("shared-file"), Some("shared-folder"), None);
        pc.hold_all();

        let review = review(&pc.db.found_while_paused_changes().unwrap());
        assert_eq!(
            (review.new_folders, review.new_files, review.edits, review.deletions, review.in_shared_folders),
            (1, 1, 2, 1, 2),
            "{review:?}"
        );
        let mut shown = review.op_ids.clone();
        shown.sort();
        assert_eq!(shown, pc.held(), "the card names exactly the held changes");
        assert_eq!(review.synced_by, None, "the backend says which service syncs the kept folder, not the review");
    }

    /// Self-review of round 3 (F8): a change anywhere inside a folder shared with this account counts as such, however
    /// deep: here a new file in a new folder made here inside the shared folder (the new rows are My Files rows).
    #[test]
    fn the_review_counts_a_change_deep_in_a_shared_folder_as_shared() {
        let pc = Pc::new();
        pc.row("shared-folder", "Shared", None, true, true, Namespace::SharedWithMe);
        pc.new_folder("new-folder", "Shared/Trip", Some("shared-folder"));
        pc.new_file("new-photo", "Shared/Trip/photo.jpg", Some("new-folder"), b"jpg");
        pc.hold_all();

        let review = review(&pc.db.found_while_paused_changes().unwrap());

        assert_eq!((review.new_folders, review.new_files, review.in_shared_folders), (0, 0, 2), "{review:?}");
    }

    /// Lead review round 3, P3-1: "Upload to my Beebeeb" releases the changes the card showed: never one held after
    /// the card read them, and a change released once is not counted again.
    #[test]
    fn upload_releases_only_the_changes_the_card_showed() {
        let pc = Pc::new();
        pc.new_folder("new-folder", "Trip", None);
        pc.new_file("new-top", "todo.txt", None, b"todo");
        pc.hold_all();
        let shown = pc.held();
        assert_eq!(shown, strings(&["op-new-folder", "op-new-top"]), "precondition");
        pc.queue_create_file("op-new-inside", Some("new-inside"), Some("new-folder"), None);
        assert!(pc.held().contains(&"op-new-inside".to_string()), "precondition: held after the card read them");

        assert_eq!(pc.db.release_found_while_paused(&shown, 5).unwrap(), 2);
        assert_eq!(pc.held(), strings(&["op-new-inside"]), "the change the card did not show stays held");
        assert_eq!(pc.db.release_found_while_paused(&shown, 5).unwrap(), 0, "released once");
    }

    /// Lead review round 3, P3-3: a staged copy whose live file is gone holds the only copy of those bytes. Keep copies
    /// it into the kept folder (never over a name there) before it removes it; a copy whose live file moved with the
    /// item goes.
    #[test]
    fn an_orphaned_staged_copy_is_copied_into_the_kept_folder_never_destroyed() {
        let pc = Pc::new();
        pc.new_folder("new-folder", "Trip", None);
        pc.new_file("new-photo", "Trip/photo.jpg", Some("new-folder"), b"only copy");
        pc.new_file("new-note", "Trip/note.txt", Some("new-folder"), b"note");
        std::fs::remove_file(pc.on_disk("Trip/photo.jpg")).unwrap();
        std::fs::create_dir_all(pc.kept_dir()).unwrap();
        std::fs::write(pc.kept_dir().join("photo.jpg"), b"already there").unwrap();
        pc.hold_all();

        assert_eq!(pc.keep().moved_items, 1);
        assert_eq!(std::fs::read(pc.kept_dir().join("photo (2).jpg")).unwrap(), b"only copy", "the only copy is kept");
        assert_eq!(std::fs::read(pc.kept_dir().join("photo.jpg")).unwrap(), b"already there", "never over a name");
        assert_eq!(std::fs::read(pc.kept_dir().join("Trip").join("note.txt")).unwrap(), b"note");
        assert!(!pc.staging.join("new-photo.staged").exists(), "then the staged copy goes");
        assert!(!pc.staging.join("new-note.staged").exists(), "a copy whose file moved with the item goes");
        assert!(pc.db.staged_payloads_for_signout().unwrap().is_empty(), "the journal forgets both");
    }

    /// Self-review of round 3 (F7): Keep moves only the items of the changes the card showed; a change held after the
    /// card read them stays held, and its item stays where it is.
    #[test]
    fn keep_moves_only_the_items_of_the_changes_the_card_showed() {
        let pc = Pc::new();
        pc.new_folder("shown", "Shown", None);
        pc.new_folder("later", "Later", None);
        pc.hold_all();

        let result = keep(&pc.db, &pc.root, TODAY, &rename_no_replace, &strings(&["op-shown"])).unwrap();

        assert_eq!(result.moved_items, 1);
        assert!(pc.kept_dir().join("Shown").exists());
        assert!(pc.on_disk("Later").exists(), "not shown: not moved");
        assert_eq!(pc.held(), strings(&["op-later"]));
    }

    /// Self-review of round 3 (F3): the only copy of a file's bytes is in the kept folder before the keep forgets the
    /// item, so a crash right after the forget cannot leave that copy named by nothing.
    #[test]
    fn an_only_copy_is_kept_before_the_item_is_forgotten() {
        let pc = Pc::new();
        pc.new_folder("new-folder", "Trip", None);
        pc.new_file("new-photo", "Trip/photo.jpg", Some("new-folder"), b"only copy");
        std::fs::remove_file(pc.on_disk("Trip/photo.jpg")).unwrap();
        pc.hold_all();
        let seen = std::rc::Rc::new(std::cell::RefCell::new(None));
        let (kept, db, seen_in_hook) = (pc.kept_dir().join("photo.jpg"), pc.db.clone(), seen.clone());
        before_forget_for_test(move || {
            *seen_in_hook.borrow_mut() = Some((std::fs::read(&kept).ok(), db.own_data_proof_ops().unwrap().len()));
        });

        assert_eq!(pc.keep().moved_items, 1);

        assert_eq!(
            seen.borrow().clone(),
            Some((Some(b"only copy".to_vec()), 2)),
            "copied while the item's changes were still there"
        );
    }

    /// Self-review of round 3 (F3): a keep that stopped after copying an only copy and before forgetting the item is
    /// finished at the next start, and the copy already in the kept folder is not made twice.
    #[test]
    fn a_keep_stopped_after_the_copy_is_finished_at_the_next_start_with_one_copy() {
        let pc = Pc::new();
        pc.new_folder("new-folder", "Trip", None);
        pc.new_file("new-photo", "Trip/photo.jpg", Some("new-folder"), b"only copy");
        std::fs::remove_file(pc.on_disk("Trip/photo.jpg")).unwrap();
        pc.hold_all();
        before_forget_for_test(|| panic!("the process ends here"));
        let crashed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| pc.keep()));
        assert!(crashed.is_err(), "precondition: the keep was interrupted");

        assert_eq!(recover(&pc.db, &pc.root).unwrap(), Recovery::Finished);

        assert_eq!(std::fs::read(pc.kept_dir().join("photo.jpg")).unwrap(), b"only copy");
        assert!(!pc.kept_dir().join("photo (2).jpg").exists(), "one copy, not two");
        assert!(!pc.staging.join("new-photo.staged").exists());
        assert!(pc.db.staged_payloads_for_signout().unwrap().is_empty());
    }

    /// Lead review round 3, P3-3: Keep removes only the staged copies the journal names; any other file a queued change
    /// names as its copy is left where it is.
    #[test]
    fn a_staged_copy_the_journal_does_not_name_is_never_deleted() {
        let pc = Pc::new();
        pc.new_folder("new-folder", "Trip", None);
        std::fs::write(pc.on_disk("Trip/photo.jpg"), b"jpg").unwrap();
        pc.row("new-photo", "Trip/photo.jpg", Some("new-folder"), false, false, Namespace::MyFiles);
        let unjournalled = pc.staging.join("not-journalled.bin");
        std::fs::write(&unjournalled, b"someone's file").unwrap();
        pc.queue_create_file("op-new-photo", Some("new-photo"), Some("new-folder"), Some(&unjournalled));
        pc.hold_all();

        assert_eq!(pc.keep().moved_items, 1);
        assert_eq!(std::fs::read(&unjournalled).unwrap(), b"someone's file");
    }

    /// Lead review round 3, P3-4: the kept folder is made next to the Beebeeb folder. When OneDrive or Dropbox syncs
    /// that place, the card says which instead of claiming the items stay on this PC.
    #[test]
    fn a_folder_in_a_onedrive_or_dropbox_folder_says_which_syncs_it() {
        let roots = vec![
            (PathBuf::from("/home/user/OneDrive - Initlabs"), "OneDrive"),
            (PathBuf::from("/home/user/Dropbox"), "Dropbox"),
        ];
        assert_eq!(synced_by(Path::new("/home/user/OneDrive - Initlabs"), &roots), Some("OneDrive"), "the root itself");
        assert_eq!(synced_by(Path::new("/home/user/onedrive - initlabs/Docs"), &roots), Some("OneDrive"), "any case");
        assert_eq!(synced_by(Path::new("/home/user/Dropbox/Work"), &roots), Some("Dropbox"));
        assert_eq!(synced_by(Path::new("/home/user/Dropbox Old"), &roots), None, "a name that only starts the same");
        assert_eq!(synced_by(Path::new("/home/user"), &roots), None, "above it");
    }

    #[test]
    fn dropbox_names_its_personal_and_business_folders_in_info_json() {
        let info = r#"{"personal": {"path": "C:\\Users\\a\\Dropbox", "host": 1}, "business": {"path": "C:\\Users\\a\\Dropbox (Initlabs)"}, "other": 3}"#;
        let mut roots = dropbox_roots(info);
        roots.sort();
        assert_eq!(roots, vec![PathBuf::from(r"C:\Users\a\Dropbox"), PathBuf::from(r"C:\Users\a\Dropbox (Initlabs)")]);
        assert!(dropbox_roots("not json").is_empty());
        assert!(dropbox_roots(r#"{"personal": {"path": ""}}"#).is_empty(), "an empty path is no folder");
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
        // Made on this PC: a queued create names it (lead review round 2, P2-1); else it is the server's and the
        // item stays held for that reason instead.
        pc.queue_create_file("op-cloud", Some("cloud"), Some("new-folder"), None);
        pc.hold_all();

        let started = std::time::Instant::now();
        let result = pc.keep();
        let took = started.elapsed();

        assert_eq!((result.moved_items, result.not_moved), (0, 1), "{result:?}");
        assert!(pc.on_disk("Trip").is_dir());
        assert_eq!(pc.held(), strings(&["op-cloud", "op-new-folder"]));
        assert!(took < std::time::Duration::from_secs(20), "nothing asked for the bytes: {took:?}");
        let attributes = std::os::windows::fs::MetadataExt::file_attributes(
            &std::fs::symlink_metadata(pc.on_disk("Trip/cloud.bin")).unwrap(),
        );
        assert_ne!(attributes & 0x0040_0000, 0, "still cloud-only (RECALL_ON_DATA_ACCESS)");
    }
}
