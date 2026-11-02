//! Task 2267 (fix round 2): what the person decides about the changes Beebeeb found while sync was paused. The
//! ownership proof (`own_data_proof.rs`) holds every queued change it cannot tie to the account's proven data as
//! `found_while_paused`; Settings › Sync shows them in one entry with two actions, and neither deletes anything:
//!
//! - The card first reads them by kind ([`review`]); "Upload to my Beebeeb" releases exactly the changes it showed
//!   (`StateDb::release_found_while_paused`), never one held after that read (lead review round 3, P2-2 and P3-1).
//! - "Keep on this PC only" (the lead's ruling (a)) moves each item made on this PC that a held change acts on out of
//!   the Beebeeb folder, into `Beebeeb - not uploaded\YYYY-MM-DD\` next to it, and forgets its rows and queued
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

/// The folder next to the Beebeeb folder that kept items move into (plain hyphen, lead ruling (a) 1). Its name, by the
/// lead's ruling on self-review F9 (2026-10-29), is true whether or not OneDrive or Dropbox syncs the folder it is made in.
pub const KEPT_FOLDER_NAME: &str = "Beebeeb - not uploaded";

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
mod tests;
