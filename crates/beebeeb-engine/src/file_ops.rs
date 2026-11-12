//! File operations on a sync root: create, rename, move, duplicate and copy-in.
//!
//! Ported from `src-tauri/src/drive_v2/fsops.rs` as text: the same rules, the same checks, in the
//! engine crate so the native shell can reach them through `Engine` without Tauri. No network, no
//! key: the engine's watcher uploads changes from the sync root, and core encrypts.
//!
//! Every path is built from a state DB path, never from the UI, and checked twice: the string checks
//! of [`path_safety::reject_unsafe_rel_path`], then containment under the sync root. The Backups tree
//! keeps its protection (LD-0113): [`backup_protect`] answers before anything is changed.

use std::path::{Path, PathBuf};

use beebeeb_core::backup::Mutation;

use crate::backup_protect;
use crate::drive_v2::DriveError;
use crate::path_safety;
use crate::state_db::{FileEntry, FileStatus, Namespace, PERMISSION_WRITE, StateDb};
use crate::vault_tree;

// ---------------------------------------------------------------------------
// Name validation (same rules as `fsops::validate_name`)
// ---------------------------------------------------------------------------

const WINDOWS_RESERVED: [&str; 22] = [
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8", "com9", "lpt1", "lpt2",
    "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// A name the user typed: one path segment that Windows, macOS and Linux all accept.
pub fn validate_name(name: &str) -> Result<String, DriveError> {
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

// ---------------------------------------------------------------------------
// Path helpers
// ---------------------------------------------------------------------------

fn join_rel(parent_rel: &str, name: &str) -> String {
    let parent = parent_rel.trim_matches('/');
    if parent.is_empty() {
        name.to_string()
    } else {
        format!("{parent}/{name}")
    }
}

/// The state DB path of a folder (`""` for the root).
pub fn folder_rel(folder: Option<&FileEntry>) -> String {
    folder.map(|f| f.path.trim_matches('/').to_string()).unwrap_or_default()
}

/// `sync_root` + a state DB path, refused when it could leave the root.
pub fn full_path(sync_root: &Path, rel: &str) -> Result<PathBuf, DriveError> {
    let rel = rel.trim_matches('/');
    if rel.is_empty() {
        return Ok(sync_root.to_path_buf());
    }
    path_safety::reject_unsafe_rel_path(rel).map_err(|_| DriveError::NotFound)?;
    let full = sync_root.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
    // Not on disk yet (a new name, or a folder the engine has not placed): the nearest ancestor that
    // is must be inside the root; the segments below it passed the string checks.
    let anchor = full
        .ancestors()
        .find(|p| p.exists())
        .map(Path::to_path_buf)
        .unwrap_or_default();
    if !path_safety::is_contained(sync_root, &anchor) {
        return Err(DriveError::NotFound);
    }
    Ok(full)
}

/// Whether a child named `name` exists in `parent_rel`, as Windows and macOS compare names
/// (case-insensitively): on disk, or in the mirror (a cloud-only item has no bytes but has a name).
pub fn name_taken(db: &StateDb, sync_root: &Path, parent_id: Option<&str>, parent_rel: &str, name: &str) -> bool {
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

// ---------------------------------------------------------------------------
// Protection checks
// ---------------------------------------------------------------------------

fn refuse_if_protected(db: &StateDb, rel: &str, mutation: Mutation) -> Result<(), DriveError> {
    let own = backup_protect::own_backups();
    match backup_protect::protected_reason(db, &own, rel, mutation) {
        Some(_) => Err(DriveError::ReadOnly),
        None => Ok(()),
    }
}

fn refuse_if_shared_read_only(db: &StateDb, entry: Option<&FileEntry>) -> Result<(), DriveError> {
    let Some(entry) = entry else { return Ok(()) };
    let contract = db.get_file_contract_state(&entry.file_id).map_err(DriveError::db)?;
    let read_only =
        contract.is_some_and(|c| c.namespace == Namespace::SharedWithMe && c.permission_bits & PERMISSION_WRITE == 0);
    if read_only { Err(DriveError::ReadOnly) } else { Ok(()) }
}

// ---------------------------------------------------------------------------
// Operations
// ---------------------------------------------------------------------------

/// Create a new folder in `parent`. Returns its state DB path.
pub fn create_folder(
    db: &StateDb,
    sync_root: &Path,
    parent: Option<&FileEntry>,
    name: &str,
) -> Result<String, DriveError> {
    let name = validate_name(name)?;
    refuse_if_shared_read_only(db, parent)?;
    let parent_rel = folder_rel(parent);
    let rel = join_rel(&parent_rel, &name);
    if backup_protect::protected_destination(db, &rel).is_some() {
        return Err(DriveError::ReadOnly);
    }
    if name_taken(db, sync_root, parent.map(|p| p.file_id.as_str()), &parent_rel, &name) {
        return Err(DriveError::NameTaken);
    }
    std::fs::create_dir(full_path(sync_root, &rel)?).map_err(DriveError::io)?;
    Ok(rel)
}

/// Rename an item. Returns the new state DB path.
pub fn rename(db: &StateDb, sync_root: &Path, entry: &FileEntry, name: &str) -> Result<String, DriveError> {
    let name = validate_name(name)?;
    let old_rel = entry.path.trim_matches('/').to_string();
    let parent_rel = old_rel.rsplit_once('/').map_or("", |(p, _)| p).to_string();
    if vault_tree::folder_leaf_name(&old_rel).as_deref() == Some(name.as_str()) {
        return Ok(old_rel);
    }
    refuse_if_shared_read_only(db, Some(entry))?;
    refuse_if_protected(db, &old_rel, Mutation::Rename)?;
    // A change of case only is the same name to Windows: no clash with itself.
    let same_but_case =
        vault_tree::folder_leaf_name(&old_rel).is_some_and(|old| old.to_lowercase() == name.to_lowercase());
    if !same_but_case && name_taken(db, sync_root, entry.parent_id.as_deref(), &parent_rel, &name) {
        return Err(DriveError::NameTaken);
    }
    let new_rel = join_rel(&parent_rel, &name);
    std::fs::rename(full_path(sync_root, &old_rel)?, full_path(sync_root, &new_rel)?).map_err(DriveError::io)?;
    Ok(new_rel)
}

/// Move items into `target`. All are checked before the first one moves.
pub fn move_items(
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
        let name = vault_tree::folder_leaf_name(&old_rel).ok_or(DriveError::NotFound)?;
        let new_rel = join_rel(&target_rel, &name);
        let own = backup_protect::own_backups();
        if backup_protect::move_refusal(db, &own, &old_rel, Mutation::Move, Some(&new_rel)).is_some() {
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
pub fn copy_name(name: &str, attempt: u32) -> String {
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

/// Duplicate an item: a copy next to it. Returns the copy's state DB path.
pub fn duplicate(db: &StateDb, sync_root: &Path, entry: &FileEntry) -> Result<String, DriveError> {
    let old_rel = entry.path.trim_matches('/').to_string();
    let parent_rel = old_rel.rsplit_once('/').map_or("", |(p, _)| p).to_string();
    let name = vault_tree::folder_leaf_name(&old_rel).ok_or(DriveError::NotFound)?;
    let parent = match entry.parent_id.as_deref() {
        Some(p) => db.get_file(p).map_err(DriveError::db)?,
        None => None,
    };
    refuse_if_shared_read_only(db, parent.as_ref())?;
    if backup_protect::protected_destination(db, &join_rel(&parent_rel, &name)).is_some() {
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

/// Copy files and folders from this PC into `parent` (the UI's upload); the engine uploads them.
/// A clash gets a numeric suffix. Returns how many top-level items were copied in.
pub fn copy_in(
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
    let canonical_root = std::fs::canonicalize(sync_root).ok();
    let mut started = 0;
    for source in sources {
        let Ok(meta) = std::fs::symlink_metadata(source) else {
            continue;
        };
        if meta.file_type().is_symlink() {
            continue;
        }
        // A file already in the Beebeeb folder is not uploaded again from itself.
        if let Some(root) = &canonical_root {
            if std::fs::canonicalize(source).is_ok_and(|s| s.starts_with(root)) {
                continue;
            }
        }
        let Some(name) = source.file_name().map(|n| n.to_string_lossy().into_owned()) else {
            continue;
        };
        if validate_name(&name).is_err() {
            continue;
        }
        if backup_protect::protected_destination(db, &join_rel(&parent_rel, &name)).is_some() {
            return Err(DriveError::ReadOnly);
        }
        let destination = unused_child_path(&dir, &name)?;
        copy_recursive(source, &destination).map_err(DriveError::io)?;
        started += 1;
    }
    Ok(started)
}

/// A path inside `dir` for a child named `name`, with a numeric suffix if taken.
fn unused_child_path(dir: &Path, name: &str) -> Result<PathBuf, DriveError> {
    let direct = dir.join(name);
    if !direct.exists() {
        return Ok(direct);
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem, Some(ext)),
        _ => (name, None),
    };
    for n in 2..10_000 {
        let candidate = match ext {
            Some(ext) => format!("{stem} {n}.{ext}"),
            None => format!("{stem} {n}"),
        };
        let path = dir.join(&candidate);
        if !path.exists() {
            return Ok(path);
        }
    }
    Err(DriveError::NameTaken)
}

/// A file or folder that the server knows (has been synced), that is not mid-upload or in
/// conflict, outside the protected Backups tree, and not read-only shared.
pub fn check_trashable(db: &StateDb, entry: &FileEntry) -> Result<(), DriveError> {
    if entry.remote_updated_at == 0 || matches!(entry.status, FileStatus::Uploading | FileStatus::Conflict) {
        return Err(DriveError::Conflict);
    }
    let own = backup_protect::own_backups();
    if backup_protect::protected_reason(db, &own, entry.path.trim_matches('/'), Mutation::Trash).is_some() {
        return Err(DriveError::ReadOnly);
    }
    let contract = db.get_file_contract_state(&entry.file_id).map_err(DriveError::db)?;
    if contract.is_some_and(|c| c.namespace == Namespace::SharedWithMe && c.permission_bits & PERMISSION_WRITE == 0) {
        return Err(DriveError::ReadOnly);
    }
    Ok(())
}

/// Resolve a file or folder by id from the state DB (local mirror).
pub fn resolve(db: &StateDb, id: &str) -> Result<FileEntry, DriveError> {
    crate::drive_v2::model::resolve(db, id)
}

/// Resolve a folder by id (or `None` for the root) from the state DB.
pub fn resolve_folder(db: &StateDb, id: Option<&str>) -> Result<Option<FileEntry>, DriveError> {
    crate::drive_v2::model::resolve_folder(db, id)
}

/// The item's on-disk path inside the sync root (sync_root + state-DB relative path).
pub fn item_path(sync_root: &Path, entry: &FileEntry) -> Result<PathBuf, DriveError> {
    full_path(sync_root, &entry.path)
}

/// Whether the file has an extension the old shell refused to launch via "Open"
/// (`src-tauri/src/file_open_commands.rs::is_dangerous_executable`). Such files are revealed in
/// the folder instead of opened; the list matches the old shell exactly.
///
/// Windows strips trailing dots and spaces from file names before resolving them, so `x.exe.` and
/// `x.exe ` both execute as `x.exe`. The check strips them too, matching the name Windows sees.
pub fn is_dangerous_executable(path: &Path) -> bool {
    const DANGEROUS_EXTS: &[&str] = &[
        "exe", "bat", "cmd", "com", "scr", "lnk", "url", "hta", "ps1", "vbs", "vbe", "js", "jse", "wsf", "wsh", "msi",
        "reg", "cpl", "pif", "msc", "jar",
    ];
    // Strip trailing dots and spaces (Windows ignores them) before extracting the extension.
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().trim_end_matches(['.', ' ']).to_string())
        .unwrap_or_default();
    let ext = Path::new(&name)
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    DANGEROUS_EXTS.contains(&ext.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state_db::{FileEntry, FileStatus, ItemKind, StateDb};

    fn temp_db() -> (tempfile::TempDir, StateDb) {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("state.db");
        let db = StateDb::open(&db_path).unwrap();
        (dir, db)
    }

    fn seed(db: &StateDb, id: &str, path: &str, parent: Option<&str>, folder: bool) {
        let entry = FileEntry {
            file_id: id.to_string(),
            path: path.to_string(),
            status: FileStatus::CloudOnly,
            size_bytes: if folder { 0 } else { 1_200 },
            modified_at: 1_794_000_000,
            content_hash: None,
            remote_updated_at: 1_794_000_000,
            parent_id: parent.map(str::to_string),
            item_kind: if folder { ItemKind::Folder } else { ItemKind::File },
        };
        db.upsert_file_with_contract(&entry, |contract| {
            contract.parent_id = entry.parent_id.clone();
            contract.item_kind = entry.item_kind.clone();
        })
        .unwrap();
    }

    fn sync_root() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    // ── new_folder ──────────────────────────────────────────────────

    #[test]
    fn new_folder_creates_directory_and_engine_sees_it() {
        let root = sync_root();
        let (_d, db) = temp_db();
        let result = create_folder(&db, root.path(), None, "Plans");
        assert!(result.is_ok(), "create_folder succeeded");
        assert_eq!(result.unwrap(), "Plans");
        assert!(root.path().join("Plans").is_dir(), "the directory exists on disk");
    }

    #[test]
    fn new_folder_with_taken_name_fails() {
        let root = sync_root();
        let (_d, db) = temp_db();
        std::fs::create_dir(root.path().join("Work")).unwrap();
        let result = create_folder(&db, root.path(), None, "Work");
        assert_eq!(result.unwrap_err(), DriveError::NameTaken);
    }

    #[test]
    fn new_folder_with_invalid_name_fails() {
        let root = sync_root();
        let (_d, db) = temp_db();
        assert!(create_folder(&db, root.path(), None, "").is_err());
        assert!(create_folder(&db, root.path(), None, "..").is_err());
        assert!(create_folder(&db, root.path(), None, "a/b").is_err());
        assert!(create_folder(&db, root.path(), None, "CON").is_err());
    }

    // ── rename ──────────────────────────────────────────────────────

    #[test]
    fn rename_changes_the_file_on_disk() {
        let root = sync_root();
        let (_d, db) = temp_db();
        std::fs::write(root.path().join("notes.txt"), b"hello").unwrap();
        seed(&db, "f-1", "/notes.txt", None, false);
        let new_rel = rename(&db, root.path(), &resolve(&db, "f-1").unwrap(), "memo.txt").unwrap();
        assert_eq!(new_rel, "memo.txt");
        assert!(!root.path().join("notes.txt").exists());
        assert!(root.path().join("memo.txt").exists());
        assert_eq!(std::fs::read_to_string(root.path().join("memo.txt")).unwrap(), "hello");
    }

    #[test]
    fn rename_to_same_name_is_a_noop() {
        let root = sync_root();
        let (_d, db) = temp_db();
        std::fs::write(root.path().join("notes.txt"), b"hello").unwrap();
        seed(&db, "f-1", "/notes.txt", None, false);
        let result = rename(&db, root.path(), &resolve(&db, "f-1").unwrap(), "notes.txt").unwrap();
        assert_eq!(result, "notes.txt");
    }

    #[test]
    fn rename_to_taken_name_fails() {
        let root = sync_root();
        let (_d, db) = temp_db();
        std::fs::write(root.path().join("a.txt"), b"a").unwrap();
        std::fs::write(root.path().join("b.txt"), b"b").unwrap();
        seed(&db, "f-a", "/a.txt", None, false);
        seed(&db, "f-b", "/b.txt", None, false);
        assert_eq!(
            rename(&db, root.path(), &resolve(&db, "f-a").unwrap(), "b.txt").unwrap_err(),
            DriveError::NameTaken
        );
    }

    // ── move_to ─────────────────────────────────────────────────────

    #[test]
    fn move_to_moves_the_file_on_disk() {
        let root = sync_root();
        let (_d, db) = temp_db();
        std::fs::create_dir(root.path().join("Dest")).unwrap();
        std::fs::write(root.path().join("a.txt"), b"content").unwrap();
        seed(&db, "d-dest", "/Dest", None, true);
        seed(&db, "f-a", "/a.txt", None, false);
        let entries = vec![resolve(&db, "f-a").unwrap()];
        let target = resolve_folder(&db, Some("d-dest")).unwrap();
        move_items(&db, root.path(), &entries, target.as_ref()).unwrap();
        assert!(!root.path().join("a.txt").exists());
        assert!(root.path().join("Dest").join("a.txt").exists());
    }

    #[test]
    fn move_to_wrong_parent_is_detected() {
        let root = sync_root();
        let (_d, db) = temp_db();
        std::fs::create_dir(root.path().join("A")).unwrap();
        std::fs::create_dir(root.path().join("A").join("B")).unwrap();
        seed(&db, "d-a", "/A", None, true);
        seed(&db, "d-b", "/A/B", Some("d-a"), true);
        // Moving A into its child B should fail.
        let entries = vec![resolve(&db, "d-a").unwrap()];
        let target = resolve_folder(&db, Some("d-b")).unwrap();
        assert_eq!(
            move_items(&db, root.path(), &entries, target.as_ref()).unwrap_err(),
            DriveError::Conflict,
            "a folder cannot be moved into itself"
        );
    }

    // ── trash ───────────────────────────────────────────────────────

    #[test]
    fn trash_check_rejects_uploading_and_unsynced() {
        let (_d, db) = temp_db();
        // An uploading file: not trashable.
        let uploading = FileEntry {
            file_id: "f-up".to_string(),
            path: "/uploading.txt".to_string(),
            status: FileStatus::Uploading,
            size_bytes: 100,
            modified_at: 1_000_000,
            content_hash: None,
            remote_updated_at: 1_000_000,
            parent_id: None,
            item_kind: ItemKind::File,
        };
        db.upsert_file_with_contract(&uploading, |c| {
            c.item_kind = ItemKind::File;
        })
        .unwrap();
        assert_eq!(
            check_trashable(&db, &resolve(&db, "f-up").unwrap()).unwrap_err(),
            DriveError::Conflict,
            "uploading file cannot be trashed"
        );

        // A file the server has never seen: not trashable.
        let local_only = FileEntry {
            file_id: "f-local".to_string(),
            path: "/local.txt".to_string(),
            status: FileStatus::CloudOnly,
            size_bytes: 50,
            modified_at: 1_000_000,
            content_hash: None,
            remote_updated_at: 0,
            parent_id: None,
            item_kind: ItemKind::File,
        };
        db.upsert_file_with_contract(&local_only, |c| {
            c.item_kind = ItemKind::File;
        })
        .unwrap();
        assert_eq!(
            check_trashable(&db, &resolve(&db, "f-local").unwrap()).unwrap_err(),
            DriveError::Conflict,
            "unsynced file cannot be trashed"
        );
    }

    #[test]
    fn trash_check_accepts_synced_file() {
        let (_d, db) = temp_db();
        seed(&db, "f-ok", "/ok.txt", None, false);
        assert!(check_trashable(&db, &resolve(&db, "f-ok").unwrap()).is_ok());
    }

    // ── duplicate ───────────────────────────────────────────────────

    #[test]
    fn duplicate_creates_a_copy_with_a_suffix() {
        let root = sync_root();
        let (_d, db) = temp_db();
        std::fs::write(root.path().join("plan.pdf"), b"data").unwrap();
        seed(&db, "f-plan", "/plan.pdf", None, false);
        let copy_rel = duplicate(&db, root.path(), &resolve(&db, "f-plan").unwrap()).unwrap();
        assert_eq!(copy_rel, "plan - Copy.pdf");
        assert!(root.path().join("plan.pdf").exists(), "original still exists");
        assert!(root.path().join("plan - Copy.pdf").exists(), "copy exists");
        assert_eq!(
            std::fs::read(root.path().join("plan - Copy.pdf")).unwrap(),
            b"data",
            "copy has same content"
        );
    }

    // ── upload (copy_in) ────────────────────────────────────────────

    #[test]
    fn upload_copies_files_into_the_sync_root() {
        let root = sync_root();
        let source_dir = tempfile::tempdir().unwrap();
        let source_file = source_dir.path().join("report.pdf");
        std::fs::write(&source_file, b"pdf bytes").unwrap();
        let (_d, db) = temp_db();
        let count = copy_in(&db, root.path(), None, &[source_file]).unwrap();
        assert_eq!(count, 1);
        assert!(root.path().join("report.pdf").exists());
        assert_eq!(std::fs::read(root.path().join("report.pdf")).unwrap(), b"pdf bytes");
    }

    #[test]
    fn upload_skips_files_already_inside_sync_root() {
        let root = sync_root();
        let inside = root.path().join("already.txt");
        std::fs::write(&inside, b"inside").unwrap();
        let (_d, db) = temp_db();
        let count = copy_in(&db, root.path(), None, &[inside]).unwrap();
        assert_eq!(count, 0, "a file already in the sync root is not re-copied");
    }

    // ── open check ──────────────────────────────────────────────────

    #[test]
    fn dangerous_executable_blocks_open() {
        assert!(is_dangerous_executable(Path::new("virus.exe")));
        assert!(is_dangerous_executable(Path::new("script.bat")));
        assert!(is_dangerous_executable(Path::new("hack.ps1")));
        assert!(!is_dangerous_executable(Path::new("doc.pdf")));
        assert!(!is_dangerous_executable(Path::new("photo.jpg")));
    }

    // ── mutants ─────────────────────────────────────────────────────

    // Mutant: trash turned into hard delete.
    // The check_trashable function validates BEFORE the API call happens; the actual delete is
    // done by the API's `DELETE /api/v1/files/{id}` which is a soft delete (Trash). A hard delete
    // uses a DIFFERENT endpoint (`DELETE /api/v1/files/{id}/permanent` with a confirm token).
    // The Engine::trash_items method calls the delete endpoint without a confirm token, which the
    // server treats as Trash. To verify: the API call in app.rs uses `DELETE /api/v1/files/{id}`,
    // not the permanent endpoint.
    #[test]
    fn the_trash_api_call_is_not_a_hard_delete() {
        let source = include_str!("app.rs");
        let trash_fn = &source[source.find("pub fn trash_items").unwrap()..];
        let trash_fn = &trash_fn[..trash_fn.find("\n    }").unwrap()];
        assert!(trash_fn.contains("/api/v1/files/{id}"), "trash uses the file endpoint");
        assert!(
            !trash_fn.contains("permanent"),
            "trash does NOT use the permanent endpoint"
        );
        assert!(
            !trash_fn.contains("X-Confirm-Token"),
            "trash does NOT send a confirm token"
        );
    }

    // Mutant: rename dropping the encryption of the name.
    // The rename function works on the LOCAL filesystem. The engine's watcher detects the change
    // and the sync engine encrypts the name when it uploads. So no name encryption happens here;
    // the plaintext name is correct at the filesystem level.

    // Mutant: move to the wrong parent.
    #[test]
    fn move_to_puts_in_the_right_parent() {
        let root = sync_root();
        let (_d, db) = temp_db();
        std::fs::create_dir(root.path().join("Correct")).unwrap();
        std::fs::create_dir(root.path().join("Wrong")).unwrap();
        std::fs::write(root.path().join("file.txt"), b"content").unwrap();
        seed(&db, "d-correct", "/Correct", None, true);
        seed(&db, "d-wrong", "/Wrong", None, true);
        seed(&db, "f-1", "/file.txt", None, false);
        let entries = vec![resolve(&db, "f-1").unwrap()];
        let target = resolve_folder(&db, Some("d-correct")).unwrap();
        move_items(&db, root.path(), &entries, target.as_ref()).unwrap();
        assert!(
            root.path().join("Correct").join("file.txt").exists(),
            "file is in the correct folder"
        );
        assert!(
            !root.path().join("Wrong").join("file.txt").exists(),
            "file is not in the wrong folder"
        );
    }

    // Mutant: upload sending the plaintext (not encrypted).
    // The copy_in function copies files into the sync root as plaintext — this is correct.
    // The encryption is done by the sync engine when it picks up the change. The user's files
    // are plaintext on their own disk. This is the same as the old shell's upload path.

    // Mutant: an error mapped to Ok.
    #[test]
    fn error_on_nonexistent_folder_is_surfaced() {
        let root = sync_root();
        let (_d, db) = temp_db();
        // No folder "d-gone" in the DB.
        let result = create_folder(
            &db,
            root.path(),
            Some(&FileEntry {
                file_id: "d-gone".to_string(),
                path: "/gone".to_string(),
                status: FileStatus::CloudOnly,
                size_bytes: 0,
                modified_at: 0,
                content_hash: None,
                remote_updated_at: 0,
                parent_id: None,
                item_kind: ItemKind::Folder,
            }),
            "sub",
        );
        // The folder's parent path doesn't exist on disk, so create_dir fails.
        assert!(result.is_err(), "error is surfaced, never swallowed");
    }

    // ── injection safety ────────────────────────────────────────────

    #[test]
    fn dangerous_executable_matches_the_old_shells_list() {
        // Every extension from src-tauri/src/file_open_commands.rs::is_dangerous_executable.
        for ext in [
            "exe", "bat", "cmd", "com", "scr", "lnk", "url", "hta", "ps1", "vbs", "vbe", "js", "jse", "wsf", "wsh",
            "msi", "reg", "cpl", "pif", "msc", "jar",
        ] {
            assert!(
                is_dangerous_executable(Path::new(&format!("name.{ext}"))),
                "{ext} must be blocked"
            );
        }
        // Safe extensions.
        for ext in ["pdf", "jpg", "docx", "txt", "mp4", "zip"] {
            assert!(
                !is_dangerous_executable(Path::new(&format!("name.{ext}"))),
                "{ext} must not be blocked"
            );
        }
        // No extension.
        assert!(!is_dangerous_executable(Path::new("README")));
    }

    /// P2-1: case-folding and Windows name tricks. Each line goes red on the mutant "drop
    /// to_lowercase" or "drop trailing-dot/space stripping".
    #[test]
    fn denylist_catches_case_variants_and_windows_trailing_tricks() {
        // Case variants: red when to_lowercase is dropped.
        assert!(
            is_dangerous_executable(Path::new("report.EXE")),
            "uppercase EXE must be blocked"
        );
        assert!(
            is_dangerous_executable(Path::new("a.Bat")),
            "mixed-case Bat must be blocked"
        );
        // Trailing dot: Windows strips it, so x.exe. runs as x.exe.
        // Red when the trailing-dot stripping is dropped.
        assert!(
            is_dangerous_executable(Path::new("x.exe.")),
            "x.exe. (trailing dot) must be blocked: Windows strips the dot"
        );
        // Trailing space: Windows strips it, so x.exe<space> runs as x.exe.
        assert!(
            is_dangerous_executable(Path::new("x.exe ")),
            "x.exe<space> (trailing space) must be blocked: Windows strips the space"
        );
        // Double extension: only the last matters.
        assert!(
            is_dangerous_executable(Path::new("a.txt.exe")),
            "a.txt.exe must be blocked (last extension is exe)"
        );
        // Multiple trailing dots.
        assert!(
            is_dangerous_executable(Path::new("payload.scr...")),
            "payload.scr... must be blocked"
        );
        // Trailing dots on a safe extension: safe.
        assert!(
            !is_dangerous_executable(Path::new("notes.txt.")),
            "notes.txt. is safe (txt after stripping)"
        );
    }

    #[test]
    fn names_with_shell_metacharacters_are_valid_file_names() {
        // These names contain characters dangerous in cmd.exe but are valid filesystem names.
        // The validate_name function accepts them (they're valid on all three OSes).
        assert!(validate_name("a&calc.exe").is_ok());
        assert!(validate_name("x^y%PATH%.txt").is_ok());
        assert!(validate_name("file with spaces.txt").is_ok());
        assert!(validate_name("file\"quotes.txt").is_err(), "quotes are a bad char");
    }

    #[test]
    fn path_traversal_names_are_rejected() {
        // These names attempt path traversal and must be rejected.
        assert!(validate_name("..").is_err());
        assert!(validate_name("../../../etc/passwd").is_err(), "contains /");
        assert!(validate_name("..\\..\\evil.txt").is_err(), "contains \\");
    }

    #[test]
    fn reserved_windows_names_are_rejected() {
        assert!(validate_name("CON").is_err());
        assert!(validate_name("CON.txt").is_err());
        assert!(validate_name("NUL").is_err());
        assert!(validate_name("LPT1").is_err());
    }

    #[test]
    fn full_path_refuses_escape_from_sync_root() {
        let root = sync_root();
        assert!(full_path(root.path(), "..").is_err(), ".. is refused");
        assert!(full_path(root.path(), "../../etc/passwd").is_err());
    }
}
