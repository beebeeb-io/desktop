//! Create, rename, move, duplicate, upload, read and save as operations on the sync root: the same
//! change File Explorer would make, so the engine (watcher, Cloud Files callbacks, the upload path
//! and core's encryption rules) does the rest and the window and Explorer always agree.
//!
//! Every path is built from a state DB path, never from the web view, and checked twice: the
//! string checks of `reject_unsafe_rel_path`, then containment under the sync root. The Backups
//! tree keeps its protection (LD-0113): `backup_protect` answers before anything is changed.

use std::path::{Path, PathBuf};

use beebeeb_core::backup::Mutation;
use sha2::{Digest, Sha256};

use super::DriveError;
use crate::state_db::{FileEntry, StateDb};

/// The largest file `drive_read_text` opens (the text editor's limit).
pub(crate) const TEXT_LIMIT: u64 = 8 * 1024 * 1024;
/// The largest file `drive_read_bytes` opens (viewers and Office).
pub(crate) const BYTES_LIMIT: u64 = 256 * 1024 * 1024;

const WINDOWS_RESERVED: [&str; 22] = [
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8", "com9", "lpt1", "lpt2",
    "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// A name the user typed: one path segment that Windows, macOS and Linux all accept.
pub(crate) fn validate_name(name: &str) -> Result<String, DriveError> {
    let name = name.trim();
    let bad_char = |c: char| c.is_control() || matches!(c, '/' | '\\' | '<' | '>' | ':' | '"' | '|' | '?' | '*');
    let stem = name.split('.').next().unwrap_or("").to_ascii_lowercase();
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.len() > 255
        || name.chars().any(bad_char)
        || name.ends_with('.')
        || WINDOWS_RESERVED.contains(&stem.as_str())
    {
        return Err(DriveError::InvalidName);
    }
    Ok(name.to_string())
}

fn join_rel(parent_rel: &str, name: &str) -> String {
    let parent = parent_rel.trim_matches('/');
    if parent.is_empty() {
        name.to_string()
    } else {
        format!("{parent}/{name}")
    }
}

/// The state DB path of a folder (`""` for the root).
pub(crate) fn folder_rel(folder: Option<&FileEntry>) -> String {
    folder.map(|f| f.path.trim_matches('/').to_string()).unwrap_or_default()
}

/// `sync_root` + a state DB path, refused when it could leave the root.
pub(crate) fn full_path(sync_root: &Path, rel: &str) -> Result<PathBuf, DriveError> {
    let rel = rel.trim_matches('/');
    if rel.is_empty() {
        return Ok(sync_root.to_path_buf());
    }
    crate::file_open_commands::reject_unsafe_rel_path(rel).map_err(|_| DriveError::NotFound)?;
    let full = sync_root.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
    // Not on disk yet (a new name, or a folder the engine has not placed): the nearest ancestor that
    // is must be inside the root; the segments below it passed the string checks.
    let anchor = full
        .ancestors()
        .find(|p| p.exists())
        .map(Path::to_path_buf)
        .unwrap_or_default();
    if !crate::file_open_commands::is_contained(sync_root, &anchor) {
        return Err(DriveError::NotFound);
    }
    Ok(full)
}

/// Whether a child named `name` exists in `parent_rel`, as Windows and macOS compare names
/// (case-insensitively): on disk, or in the mirror (a cloud-only item has no bytes but has a name).
pub(crate) fn name_taken(
    db: &StateDb,
    sync_root: &Path,
    parent_id: Option<&str>,
    parent_rel: &str,
    name: &str,
) -> bool {
    let lower = name.to_lowercase();
    if db
        .child_names_lowercase(parent_id)
        .map(|names| names.contains(&lower))
        .unwrap_or(false)
    {
        return true;
    }
    let Ok(dir) = full_path(sync_root, parent_rel) else {
        return true;
    };
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .any(|e| e.file_name().to_string_lossy().to_lowercase() == lower)
        })
        .unwrap_or(false)
}

fn refuse_if_protected(db: &StateDb, rel: &str, mutation: Mutation) -> Result<(), DriveError> {
    let own = crate::backup_protect::own_backups();
    match crate::backup_protect::protected_reason(db, &own, rel, mutation) {
        Some(_) => Err(DriveError::ReadOnly),
        None => Ok(()),
    }
}

fn refuse_if_shared_read_only(db: &StateDb, entry: Option<&FileEntry>) -> Result<(), DriveError> {
    let Some(entry) = entry else { return Ok(()) };
    let contract = db.get_file_contract_state(&entry.file_id).map_err(DriveError::db)?;
    let read_only = contract.is_some_and(|c| {
        c.namespace == crate::state_db::Namespace::SharedWithMe
            && c.permission_bits & crate::state_db::PERMISSION_WRITE == 0
    });
    if read_only { Err(DriveError::ReadOnly) } else { Ok(()) }
}

/// `drive_create_folder`: a new folder in `parent`. Returns its state DB path.
pub(crate) fn create_folder(
    db: &StateDb,
    sync_root: &Path,
    parent: Option<&FileEntry>,
    name: &str,
) -> Result<String, DriveError> {
    let name = validate_name(name)?;
    refuse_if_shared_read_only(db, parent)?;
    let parent_rel = folder_rel(parent);
    let rel = join_rel(&parent_rel, &name);
    if crate::backup_protect::protected_destination(db, &rel).is_some() {
        return Err(DriveError::ReadOnly);
    }
    if name_taken(db, sync_root, parent.map(|p| p.file_id.as_str()), &parent_rel, &name) {
        return Err(DriveError::NameTaken);
    }
    std::fs::create_dir(full_path(sync_root, &rel)?).map_err(DriveError::io)?;
    Ok(rel)
}

/// `drive_create_file`: a new file named `name` in `parent` holding `bytes` (New file, DW-1522; the editors' "Keep
/// both"). The engine uploads it as any new file in the folder. A taken name is refused, never overwritten: the file
/// is opened `create_new`, after the name was checked against the folder and against what else it reaches on Windows
/// (an 8.3 short name, task 2407). Returns its state DB path.
pub(crate) fn create_file(
    db: &StateDb,
    sync_root: &Path,
    parent: Option<&FileEntry>,
    name: &str,
    bytes: &[u8],
) -> Result<String, DriveError> {
    let name = validate_name(name)?;
    if bytes.len() as u64 > BYTES_LIMIT {
        return Err(DriveError::TooLarge);
    }
    refuse_if_shared_read_only(db, parent)?;
    let parent_rel = folder_rel(parent);
    let rel = join_rel(&parent_rel, &name);
    if crate::backup_protect::protected_destination(db, &rel).is_some() {
        return Err(DriveError::ReadOnly);
    }
    if name_taken(db, sync_root, parent.map(|p| p.file_id.as_str()), &parent_rel, &name) {
        return Err(DriveError::NameTaken);
    }
    let path = full_path(sync_root, &rel)?;
    match crate::name_alias::conflict_below(sync_root, &path) {
        Ok(None) => {}
        Ok(Some(_)) => return Err(DriveError::NameTaken),
        Err(_) => return Err(DriveError::Failed("name check")),
    }
    // `create_new`: an entry that appeared since the checks is a taken name (AlreadyExists), never truncated.
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(DriveError::io)?;
    // A write that fails part way (the disk filled) leaves nothing behind: a half file would be uploaded, and would take
    // the name the retry needs. The file is ours: create_new made it a moment ago.
    if let Err(error) = std::io::Write::write_all(&mut file, bytes).and_then(|()| file.sync_all()) {
        drop(file);
        let _ = std::fs::remove_file(&path);
        return Err(DriveError::io(error));
    }
    Ok(rel)
}

/// `drive_rename`: a new name in the same folder. Returns the new state DB path.
pub(crate) fn rename(db: &StateDb, sync_root: &Path, entry: &FileEntry, name: &str) -> Result<String, DriveError> {
    let name = validate_name(name)?;
    let old_rel = entry.path.trim_matches('/').to_string();
    let parent_rel = old_rel.rsplit_once('/').map_or("", |(p, _)| p).to_string();
    if crate::vault_tree::folder_leaf_name(&old_rel).as_deref() == Some(name.as_str()) {
        return Ok(old_rel);
    }
    refuse_if_shared_read_only(db, Some(entry))?;
    refuse_if_protected(db, &old_rel, Mutation::Rename)?;
    // A change of case only is the same name to Windows: no clash with itself.
    let same_but_case =
        crate::vault_tree::folder_leaf_name(&old_rel).is_some_and(|old| old.to_lowercase() == name.to_lowercase());
    if !same_but_case && name_taken(db, sync_root, entry.parent_id.as_deref(), &parent_rel, &name) {
        return Err(DriveError::NameTaken);
    }
    let new_rel = join_rel(&parent_rel, &name);
    std::fs::rename(full_path(sync_root, &old_rel)?, full_path(sync_root, &new_rel)?).map_err(DriveError::io)?;
    Ok(new_rel)
}

/// `drive_move`: each item into `target`. All are checked before the first one moves.
pub(crate) fn move_items(
    db: &StateDb,
    sync_root: &Path,
    entries: &[FileEntry],
    target: Option<&FileEntry>,
) -> Result<(), DriveError> {
    let target_rel = folder_rel(target);
    let target_id = target.map(|t| t.file_id.as_str());
    refuse_if_shared_read_only(db, target)?;
    let mut plan = Vec::new();
    for entry in entries {
        if entry.parent_id.as_deref() == target_id {
            continue;
        }
        let old_rel = entry.path.trim_matches('/').to_string();
        // A folder never moves into itself or below itself.
        let lower_old = old_rel.to_lowercase();
        let lower_target = target_rel.to_lowercase();
        if entry.is_dir() && (lower_target == lower_old || lower_target.starts_with(&format!("{lower_old}/"))) {
            return Err(DriveError::Conflict);
        }
        refuse_if_shared_read_only(db, Some(entry))?;
        let name = crate::vault_tree::folder_leaf_name(&old_rel).ok_or(DriveError::NotFound)?;
        let new_rel = join_rel(&target_rel, &name);
        let own = crate::backup_protect::own_backups();
        if crate::backup_protect::move_refusal(db, &own, &old_rel, Mutation::Move, Some(&new_rel)).is_some() {
            return Err(DriveError::ReadOnly);
        }
        if name_taken(db, sync_root, target_id, &target_rel, &name) {
            return Err(DriveError::NameTaken);
        }
        plan.push((full_path(sync_root, &old_rel)?, full_path(sync_root, &new_rel)?));
    }
    for (from, to) in plan {
        std::fs::rename(from, to).map_err(DriveError::io)?;
    }
    Ok(())
}

/// The copy's name as File Explorer makes it: `Report - Copy.pdf`, then `Report - Copy (2).pdf`.
pub(crate) fn copy_name(name: &str, attempt: u32) -> String {
    let (stem, ext) = match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem, Some(ext)),
        _ => (name, None),
    };
    let suffix = if attempt <= 1 {
        " - Copy".to_string()
    } else {
        format!(" - Copy ({attempt})")
    };
    match ext {
        Some(ext) => format!("{stem}{suffix}.{ext}"),
        None => format!("{stem}{suffix}"),
    }
}

fn copy_recursive(from: &Path, to: &Path) -> std::io::Result<usize> {
    let meta = std::fs::symlink_metadata(from)?;
    if meta.file_type().is_symlink() {
        return Ok(0);
    }
    if meta.is_dir() {
        std::fs::create_dir(to)?;
        let mut n = 0;
        for child in std::fs::read_dir(from)? {
            let child = child?;
            n += copy_recursive(&child.path(), &to.join(child.file_name()))?;
        }
        Ok(n)
    } else {
        std::fs::copy(from, to)?;
        Ok(1)
    }
}

/// `drive_duplicate`: a copy next to the item. Returns the copy's state DB path.
pub(crate) fn duplicate(db: &StateDb, sync_root: &Path, entry: &FileEntry) -> Result<String, DriveError> {
    let old_rel = entry.path.trim_matches('/').to_string();
    let parent_rel = old_rel.rsplit_once('/').map_or("", |(p, _)| p).to_string();
    let name = crate::vault_tree::folder_leaf_name(&old_rel).ok_or(DriveError::NotFound)?;
    let parent = match entry.parent_id.as_deref() {
        Some(p) => db.get_file(p).map_err(DriveError::db)?,
        None => None,
    };
    refuse_if_shared_read_only(db, parent.as_ref())?;
    if crate::backup_protect::protected_destination(db, &join_rel(&parent_rel, &name)).is_some() {
        return Err(DriveError::ReadOnly);
    }
    for attempt in 1..1000 {
        let candidate = copy_name(&name, attempt);
        if name_taken(db, sync_root, entry.parent_id.as_deref(), &parent_rel, &candidate) {
            continue;
        }
        let new_rel = join_rel(&parent_rel, &candidate);
        copy_recursive(&full_path(sync_root, &old_rel)?, &full_path(sync_root, &new_rel)?).map_err(DriveError::io)?;
        return Ok(new_rel);
    }
    Err(DriveError::NameTaken)
}

/// `drive_upload_pick` / `drive_upload_dropped`: copy files and folders from this PC into `parent`;
/// the engine uploads them. A clash gets `Name 2.ext`, as the menu's upload does. Returns how many
/// top-level items were copied in.
pub(crate) fn copy_in(
    db: &StateDb,
    sync_root: &Path,
    parent: Option<&FileEntry>,
    sources: &[PathBuf],
) -> Result<usize, DriveError> {
    refuse_if_shared_read_only(db, parent)?;
    let parent_rel = folder_rel(parent);
    let dir = full_path(sync_root, &parent_rel)?;
    if !dir.is_dir() {
        return Err(DriveError::NotFound);
    }
    let canonical_root = dunce_canonical(sync_root);
    let mut started = 0;
    for source in sources {
        let Ok(meta) = std::fs::symlink_metadata(source) else {
            continue;
        };
        if meta.file_type().is_symlink() {
            continue;
        }
        // A file already in the Beebeeb folder is not uploaded again from itself.
        if let Some(root) = &canonical_root
            && dunce_canonical(source).is_some_and(|s| s.starts_with(root))
        {
            continue;
        }
        let Some(name) = source.file_name().map(|n| n.to_string_lossy().into_owned()) else {
            continue;
        };
        if validate_name(&name).is_err() {
            continue;
        }
        if crate::backup_protect::protected_destination(db, &join_rel(&parent_rel, &name)).is_some() {
            return Err(DriveError::ReadOnly);
        }
        let destination =
            crate::settings_commands::unused_child_path(&dir, &name).map_err(|_| DriveError::NameTaken)?;
        copy_recursive(source, &destination).map_err(DriveError::io)?;
        started += 1;
    }
    Ok(started)
}

fn dunce_canonical(path: &Path) -> Option<PathBuf> {
    std::fs::canonicalize(path).ok()
}

/// A file's bytes from the sync root (reading a cloud-only placeholder downloads and decrypts it
/// through the engine first), refused above `limit`.
pub(crate) fn read_capped(path: &Path, limit: u64) -> Result<Vec<u8>, DriveError> {
    let meta = std::fs::metadata(path).map_err(DriveError::io)?;
    if !meta.is_file() {
        return Err(DriveError::NotFound);
    }
    if meta.len() > limit {
        return Err(DriveError::TooLarge);
    }
    std::fs::read(path).map_err(DriveError::io)
}

/// The version a read was taken from: the server's version number and the content's hash. A save
/// names it; content that changed since (a newer version synced in, or another app's edit) is a
/// conflict, never an overwrite. The hash is of the plaintext the web view already holds.
pub(crate) fn version_tag(current_version: i64, bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let hex: String = digest.iter().take(12).map(|b| format!("{b:02x}")).collect();
    format!("v{current_version}-{hex}")
}

fn content_of_tag(tag: &str) -> Option<&str> {
    tag.split_once('-').map(|(_, hash)| hash)
}

/// Replace a file's content when it is still the content `base` was read from. Returns the tag of
/// the new content. Written in place (no temp file in the sync root for the engine to pick up).
pub(crate) fn save_checked(
    db: &StateDb,
    sync_root: &Path,
    entry: &FileEntry,
    base: &str,
    bytes: &[u8],
    limit: u64,
) -> Result<String, DriveError> {
    if entry.is_dir() {
        return Err(DriveError::NotFound);
    }
    if bytes.len() as u64 > limit {
        return Err(DriveError::TooLarge);
    }
    refuse_if_shared_read_only(db, Some(entry))?;
    let rel = entry.path.trim_matches('/').to_string();
    refuse_if_protected(db, &rel, Mutation::Write)?;
    let path = full_path(sync_root, &rel)?;
    let now = read_capped(&path, limit.max(TEXT_LIMIT))?;
    let current_version = db
        .get_file_contract_state(&entry.file_id)
        .map_err(DriveError::db)?
        .map_or(0, |c| c.current_version);
    let now_tag = version_tag(current_version, &now);
    if content_of_tag(&now_tag) != content_of_tag(base) {
        return Err(DriveError::Conflict);
    }
    std::fs::write(&path, bytes).map_err(DriveError::io)?;
    Ok(version_tag(current_version, bytes))
}

/// `drive_save_copy`: write `src`'s bytes to `dest`, a path the person picked outside the sync
/// root. Never overwrites, never writes inside the sync root (the engine would upload that as a new
/// file), and streams through a temp file next to `dest` so a failure leaves nothing behind.
pub(crate) fn copy_out(sync_root: &Path, src: &Path, dest: &Path) -> Result<(), DriveError> {
    if dest.exists() {
        return Err(DriveError::Conflict);
    }
    if let (Some(root), Some(target)) = (dunce_canonical(sync_root), canonical_target(dest))
        && target.starts_with(&root)
    {
        return Err(DriveError::Failed("inside the sync folder"));
    }
    let name = dest.file_name().ok_or(DriveError::Failed("path"))?;
    let mut tmp_name = name.to_os_string();
    tmp_name.push(".bb-tmp");
    let tmp = dest.with_file_name(tmp_name);
    let copied = std::fs::copy(src, &tmp).and_then(|_| std::fs::rename(&tmp, dest));
    if let Err(error) = copied {
        let _ = std::fs::remove_file(&tmp);
        return Err(DriveError::io(error));
    }
    Ok(())
}

/// `dest` as it will be on disk: canonical when it exists, else its canonical parent plus the name.
fn canonical_target(path: &Path) -> Option<PathBuf> {
    if let Some(found) = dunce_canonical(path) {
        return Some(found);
    }
    let parent = dunce_canonical(path.parent()?)?;
    Some(parent.join(path.file_name()?))
}
