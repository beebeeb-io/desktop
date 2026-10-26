//! Conditional match promotion and durable copy-before-delete recovery (2191 U1/U2/U7).
use crate::state_db::{StateDb, UnknownRow};
use std::path::Path;

pub(super) fn exclusive(path: &Path, write: bool, delete: bool) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true).write(write);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows::Win32::Storage::FileSystem::*;
        options.share_mode(0).custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0);
        if delete {
            options.access_mode(FILE_READ_DATA.0 | FILE_READ_ATTRIBUTES.0 | DELETE.0);
        }
    }
    #[cfg(not(target_os = "windows"))]
    let _ = delete;
    options.open(path)
}
fn hash(file: &mut std::fs::File) -> std::io::Result<String> {
    use sha2::{Digest, Sha256};
    use std::io::{Seek, SeekFrom};
    file.seek(SeekFrom::Start(0))?;
    let mut h = Sha256::new();
    std::io::copy(file, &mut h)?;
    Ok(format!("{:x}", h.finalize()))
}
pub(super) fn owns(file: &std::fs::File, id: &str) -> bool {
    #[cfg(target_os = "windows")]
    {
        let (placeholder, identity) = crate::windows_cf::placeholders::handle_identity(file);
        (!placeholder || identity.as_deref() == Some(id)) && crate::windows_cf::placeholders::held_fully_on_disk(file)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (file, id);
        true
    }
}
#[cfg(test)]
pub(super) fn matched(
    db: &StateDb,
    row: &UnknownRow,
    path: &Path,
    version: i64,
    expected: &str,
    hash_mode: bool,
) -> anyhow::Result<bool> {
    matched_checked(db, row, path, version, expected, hash_mode, None)
}
pub(super) fn matched_checked(
    db: &StateDb,
    row: &UnknownRow,
    path: &Path,
    version: i64,
    expected: &str,
    hash_mode: bool,
    scope: Option<&super::job::Scope>,
) -> anyhow::Result<bool> {
    let mut file = exclusive(path, false, false)?;
    if !owns(&file, &row.file_id) {
        return Ok(false);
    }
    if !hash_mode && hash(&mut file)? != expected {
        return Ok(false);
    }
    gate(scope, || {
        Ok(db.record_baseline_if_unknown(&row.file_id, &row.path, version, expected)?)
    })
}
#[cfg(test)]
pub(super) fn diverged(db: &StateDb, root: &Path, row: &UnknownRow, now: i64) -> anyhow::Result<bool> {
    diverged_checked(db, root, row, now, None)
}
pub(super) fn diverged_checked(
    db: &StateDb,
    root: &Path,
    row: &UnknownRow,
    now: i64,
    scope: Option<&super::job::Scope>,
) -> anyhow::Result<bool> {
    use crate::state_db::{KeptCopyInput, KeptReason};
    let path = crate::engine_bridge::local_file_path_under_sync_root(root, &row.path)?;
    if db.pending_kept_original(&row.path)?.is_none() {
        // Read the identity before copying; a different placeholder belongs to S3, never S2.
        let held = exclusive(&path, false, false)?;
        if !owns(&held, &row.file_id) {
            return Ok(false);
        }
        drop(held);
        let dir = crate::engine_bridge::kept_copy_dir(db, root, &path, &Default::default());
        let (copy, copied_hash) = crate::engine_bridge::copy_as_this_pc(&path, &dir)?;
        let relative =
            crate::engine_bridge::relative_db_path(root, &copy).ok_or_else(|| anyhow::anyhow!("copy outside root"))?;
        gate(scope, || {
            Ok(db.record_kept_copy(&KeptCopyInput {
                original_file_id: &row.file_id,
                original_path: &row.path,
                copy_path: &relative,
                copied_hash: &copied_hash,
                original_removed: false,
                reason: KeptReason::PreFixChange,
                kept_at: now,
            })?)
        })?;
    }
    let Some(copy) = db.pending_kept_original(&row.path)? else {
        return Ok(false);
    };
    finish(db, root, &copy, now, scope)
}
fn finish(
    db: &StateDb,
    root: &Path,
    copy: &crate::state_db::KeptCopy,
    now: i64,
    scope: Option<&super::job::Scope>,
) -> anyhow::Result<bool> {
    use crate::state_db::{KeptCopyInput, KeptReason};
    use std::io::{Seek, SeekFrom};
    let original = crate::engine_bridge::local_file_path_under_sync_root(root, &copy.original_path)?;
    let kept = crate::engine_bridge::local_file_path_under_sync_root(root, &copy.copy_path)?;
    // A removed/edited kept copy is not evidence that the original can be deleted.
    let mut kept_file = match exclusive(&kept, true, false) {
        Ok(f) => f,
        Err(_) => return Ok(false),
    };
    if hash(&mut kept_file)? != copy.copied_hash {
        return Ok(false);
    }
    let mut file = match exclusive(&original, false, true) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return gate(scope, || {
                db.reset_after_divergence(&copy.original_file_id, &copy.original_path)?;
                db.mark_kept_original_removed(copy.id)?;
                Ok(true)
            });
        }
        Err(_) => return Ok(false),
    };
    if !owns(&file, &copy.original_file_id) {
        return Ok(false);
    }
    let current = hash(&mut file)?;
    if current != copy.copied_hash {
        let (_, mtime) = super::record::stamp(&original)?;
        if now.saturating_sub(mtime / 1_000_000_000) < 60 {
            return Ok(false);
        }
        // One refresh, through held handles, then wait until the next quantum. No second copy.
        return gate(scope, || {
            file.seek(SeekFrom::Start(0))?;
            kept_file.seek(SeekFrom::Start(0))?;
            kept_file.set_len(0)?;
            std::io::copy(&mut file, &mut kept_file)?;
            kept_file.sync_all()?;
            let copied_hash = hash(&mut kept_file)?;
            db.update_kept_copy(
                copy.id,
                &KeptCopyInput {
                    original_file_id: &copy.original_file_id,
                    original_path: &copy.original_path,
                    copy_path: &copy.copy_path,
                    copied_hash: &copied_hash,
                    original_removed: false,
                    reason: KeptReason::PreFixChange,
                    kept_at: now,
                },
            )?;
            Ok(false)
        });
    }
    gate(scope, || {
        db.set_unknown_removing(&copy.original_file_id, true)?;
        crate::watcher::suppress_engine_delete(&original);
        let removed = delete_held(&file, &original);
        if removed.is_err() {
            crate::watcher::cancel_engine_delete_suppression(&original);
            db.set_unknown_removing(&copy.original_file_id, false)?;
            return Ok(false);
        }
        drop(file);
        db.reset_after_divergence(&copy.original_file_id, &copy.original_path)?;
        db.mark_kept_original_removed(copy.id)?;
        Ok(true)
    })
}
fn gate<T>(scope: Option<&super::job::Scope>, f: impl FnOnce() -> anyhow::Result<T>) -> anyhow::Result<T> {
    match scope {
        Some(s) => s.commit(f),
        None => f(),
    }
}
fn delete_held(file: &std::fs::File, path: &Path) -> anyhow::Result<()> {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::{
            Foundation::{BOOLEAN, HANDLE},
            Storage::FileSystem::*,
        };
        let info = FILE_DISPOSITION_INFO { DeleteFile: BOOLEAN(1) };
        // SAFETY: the exclusive read/delete handle remains alive through deletion on close.
        unsafe {
            SetFileInformationByHandle(
                HANDLE(file.as_raw_handle()),
                FileDispositionInfo,
                &info as *const _ as *const std::ffi::c_void,
                std::mem::size_of_val(&info) as u32,
            )?;
        }
        let _ = path;
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = file;
        std::fs::remove_file(path)?;
    }
    Ok(())
}
#[cfg(test)]
pub(super) fn retry(db: &StateDb, root: &Path, now: i64) -> anyhow::Result<usize> {
    retry_checked(db, root, now, None, 0)
}
pub(super) fn retry_checked(
    db: &StateDb,
    root: &Path,
    now: i64,
    scope: Option<&super::job::Scope>,
    offset: usize,
) -> anyhow::Result<usize> {
    let mut count = 0;
    for copy in db
        .list_kept_copies()?
        .into_iter()
        .filter(|c| !c.original_removed && c.reason == crate::state_db::KeptReason::PreFixChange)
        .skip(offset)
        .take(16)
    {
        if finish(db, root, &copy, now, scope)? {
            count += 1;
        }
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state_db::{FileEntry, FileStatus, ItemKind};
    use sha2::{Digest, Sha256};
    fn setup() -> (tempfile::TempDir, StateDb, UnknownRow, String) {
        let d = tempfile::tempdir().unwrap();
        let db = StateDb::open(d.path().join("state.db")).unwrap();
        std::fs::write(d.path().join("a.txt"), b"local edit").unwrap();
        db.upsert_file(&FileEntry {
            file_id: "u".into(),
            path: "a.txt".into(),
            status: FileStatus::Local,
            size_bytes: 10,
            modified_at: 0,
            content_hash: None,
            remote_updated_at: 0,
            parent_id: None,
            item_kind: ItemKind::File,
        })
        .unwrap();
        let row = db.list_unknown_to_record(1).unwrap().remove(0);
        (d, db, row, format!("{:x}", Sha256::digest(b"local edit")))
    }
    #[test]
    fn task_2191_s2_match_rehashes_and_preserves_hash_mode_edits() {
        let (d, db, row, hash) = setup();
        let path = d.path().join("a.txt");
        std::fs::write(&path, b"edited again").unwrap();
        assert!(!matched(&db, &row, &path, 3, &hash, false).unwrap());
        assert!(matched(&db, &row, &path, 3, &hash, true).unwrap());
        assert_eq!(std::fs::read(&path).unwrap(), b"edited again");
        let contract = db.get_file_contract_state("u").unwrap().unwrap();
        assert_eq!((contract.local_base_version, contract.local_hash), (3, Some(hash)));
    }
    #[test]
    fn task_2191_s2_divergence_keeps_copy_before_removal_and_never_replaces_original() {
        let (d, db, row, _) = setup();
        assert!(diverged(&db, d.path(), &row, 1000).unwrap());
        let kept = db.list_kept_copies().unwrap();
        assert_eq!(kept.len(), 1);
        assert!(!kept[0].content_known);
        assert_eq!(std::fs::read(d.path().join(&kept[0].copy_path)).unwrap(), b"local edit");
        assert!(!d.path().join("a.txt").exists());
        assert!(kept[0].original_removed);
        assert_eq!(db.get_file("u").unwrap().unwrap().status, FileStatus::CloudOnly);
        assert!(db.mark_no_fetched_content("u").unwrap());
        assert!(db.take_removing("u").unwrap(), "guard survives until watcher event");
    }
    #[test]
    fn task_2191_s2_recovery_is_entry_driven_when_original_and_row_are_gone() {
        let (d, db, row, hash) = setup();
        std::fs::write(d.path().join("kept.txt"), b"local edit").unwrap();
        db.test_execute(&format!("INSERT INTO kept_copies(original_file_id,original_path,copy_path,copied_hash,original_removed,reason,kept_at) VALUES ('u','a.txt','kept.txt','{hash}',0,'pre_fix_change',0)")).unwrap();
        std::fs::remove_file(d.path().join("a.txt")).unwrap();
        assert_eq!(retry(&db, d.path(), 1000).unwrap(), 1);
        assert_eq!(db.get_file("u").unwrap().unwrap().status, FileStatus::CloudOnly);
        db.test_execute("UPDATE kept_copies SET original_removed=0").unwrap();
        db.test_execute("DELETE FROM files").unwrap();
        assert_eq!(retry(&db, d.path(), 1001).unwrap(), 1);
        assert_eq!(std::fs::read(d.path().join("kept.txt")).unwrap(), b"local edit");
        let _ = row;
    }
    #[tokio::test]
    async fn task_2191_s2_missed_delete_suppression_never_queues_server_trash() {
        let (d, db, row, _) = setup();
        let db = std::sync::Arc::new(db);
        db.set_unknown_removing(&row.file_id, true).unwrap();
        let api = std::sync::Arc::new(crate::api_client::ApiClient::new(
            "http://127.0.0.1:9".into(),
            "test".into(),
            [0; 32],
        ));
        let bridge = crate::engine_bridge::EngineBridge::new(db.clone(), api);
        std::fs::remove_file(d.path().join("a.txt")).unwrap();
        crate::watcher::handle_delete(&bridge, d.path(), &d.path().join("a.txt")).await;
        assert!(!db.has_pending_trash(&row.file_id).unwrap());
        assert!(!db.take_removing(&row.file_id).unwrap());
        assert_eq!(db.get_file(&row.file_id).unwrap().unwrap().status, FileStatus::Local);
    }
    #[test]
    fn task_2191_s2_changed_original_refreshes_one_copy_only_after_quiet() {
        let (d, db, row, _) = setup();
        let copy = d.path().join("a (this PC).txt");
        std::fs::write(&copy, b"old").unwrap();
        let old = format!("{:x}", Sha256::digest(b"old"));
        db.record_kept_copy(&crate::state_db::KeptCopyInput {
            original_file_id: &row.file_id,
            original_path: &row.path,
            copy_path: "a (this PC).txt",
            copied_hash: &old,
            original_removed: false,
            reason: crate::state_db::KeptReason::PreFixChange,
            kept_at: 0,
        })
        .unwrap();
        let original = d.path().join("a.txt");
        std::fs::File::options()
            .write(true)
            .open(&original)
            .unwrap()
            .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(100))
            .unwrap();
        assert_eq!(retry(&db, d.path(), 159).unwrap(), 0);
        assert_eq!(std::fs::read(&copy).unwrap(), b"old");
        assert_eq!(retry(&db, d.path(), 160).unwrap(), 0);
        assert_eq!(std::fs::read(&copy).unwrap(), b"local edit");
        assert!(original.exists());
        assert_eq!(db.list_kept_copies().unwrap().len(), 1);
        assert_eq!(retry(&db, d.path(), 161).unwrap(), 1);
        assert!(!original.exists());
    }
    #[test]
    fn task_2191_s2_edited_copy_never_authorizes_original_deletion() {
        let (d, db, row, hash) = setup();
        std::fs::write(d.path().join("kept.txt"), b"user edited the copy").unwrap();
        db.record_kept_copy(&crate::state_db::KeptCopyInput {
            original_file_id: &row.file_id,
            original_path: &row.path,
            copy_path: "kept.txt",
            copied_hash: &hash,
            original_removed: false,
            reason: crate::state_db::KeptReason::PreFixChange,
            kept_at: 0,
        })
        .unwrap();
        assert_eq!(retry(&db, d.path(), 1000).unwrap(), 0);
        assert!(d.path().join("a.txt").exists());
        assert_eq!(
            std::fs::read(d.path().join("kept.txt")).unwrap(),
            b"user edited the copy"
        );
    }
    #[cfg(target_os = "windows")]
    #[test]
    fn task_2191_s2_native_exclusive_handle_blocks_write_and_rename_until_delete() {
        let (d, _db, _row, expected) = setup();
        let path = d.path().join("a.txt");
        let mut file = exclusive(&path, false, true).unwrap();
        assert_eq!(hash(&mut file).unwrap(), expected);
        assert!(std::fs::OpenOptions::new().write(true).open(&path).is_err());
        assert!(std::fs::rename(&path, d.path().join("renamed")).is_err());
        delete_held(&file, &path).unwrap();
        drop(file);
        assert!(!path.exists());
    }
    #[test]
    fn task_2191_s2_read_only_shared_folder_copy_uses_writable_parent() {
        let (d, db, _, _) = setup();
        std::fs::create_dir(d.path().join("shared")).unwrap();
        db.upsert_file(&FileEntry {
            file_id: "folder".into(),
            path: "shared".into(),
            status: FileStatus::Local,
            size_bytes: 0,
            modified_at: 0,
            content_hash: None,
            remote_updated_at: 0,
            parent_id: None,
            item_kind: ItemKind::Folder,
        })
        .unwrap();
        db.test_execute("UPDATE files SET permission_bits=1 WHERE file_id='folder'")
            .unwrap();
        assert_eq!(
            crate::engine_bridge::kept_copy_dir(&db, d.path(), &d.path().join("shared/a.txt"), &Default::default()),
            d.path()
        );
    }
}
