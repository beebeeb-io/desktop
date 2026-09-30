//! Real Windows filesystem/CFAPI and SQLite tests; no provider registration or account.
use super::*;
use crate::state_db::{FileContractState, FileEntry, FileStatus, ItemKind, Namespace, PinState};

struct Fixture {
    db: StateDb,
    root: std::path::PathBuf,
    temp: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("root");
        std::fs::create_dir(&root).unwrap();
        let db = StateDb::open(temp.path().join("state.db")).unwrap();
        Self { db, root, temp }
    }

    fn track(&self, path: &str, kind: ItemKind, status: FileStatus) {
        self.db
            .upsert_file(&FileEntry {
                file_id: path.into(),
                path: path.into(),
                status,
                size_bytes: 0,
                modified_at: 0,
                content_hash: None,
                remote_updated_at: 1,
                parent_id: None,
                item_kind: kind.clone(),
            })
            .unwrap();
        // upsert_file deliberately does not write item_kind; the engine uses
        // the contract setter too when a local CreateFolder has completed.
        self.db
            .set_file_contract_state(&FileContractState {
                file_id: path.into(),
                namespace: Namespace::MyFiles,
                parent_id: None,
                shared_root_id: None,
                share_id: None,
                owner_email: None,
                permission_bits: 1,
                item_kind: kind,
                content_type: None,
                current_version: 1,
                current_object_version_id: None,
                local_base_version: 1,
                local_hash: None,
                cache_path: None,
                cache_bytes: 0,
                pin_state: PinState::Inherit,
                inherited_pin_state: PinState::Unpinned,
                last_sync_at: 1,
            })
            .unwrap();
    }

    fn folder(&self, path: &str) {
        std::fs::create_dir(self.root.join(path)).unwrap();
        self.track(path, ItemKind::Folder, FileStatus::Local);
    }
}

#[test]
fn synced_plain_directories_sign_out_and_clear_rows() {
    let f = Fixture::new();
    f.folder("local");
    f.folder("local/nested");
    assert_eq!(f.db.list_files().unwrap().len(), 2);
    let result = purge(&f.db, Some(&f.root));
    assert!(result.is_ok(), "synced plain directories must sign out: {result:?}");
    assert_eq!(std::fs::read_dir(&f.root).unwrap().count(), 0);
    assert_eq!(f.db.list_files().unwrap().len(), 0);
}

#[test]
fn plain_directory_unsynced_child_preserves_bytes_and_rows() {
    let f = Fixture::new();
    f.folder("local");
    let child = f.root.join("local/unsynced.txt");
    std::fs::write(&child, b"unsynced bytes").unwrap();
    let error = purge(&f.db, Some(&f.root)).unwrap_err();
    assert!(
        error.to_string().contains("Unsynced files remain"),
        "must inspect children: {error:?}"
    );
    assert_eq!(std::fs::read(child).unwrap(), b"unsynced bytes");
    assert_eq!(f.db.list_files().unwrap().len(), 1);
}

#[test]
fn tracked_plain_file_still_requires_cloud_identity() {
    let f = Fixture::new();
    f.folder("local");
    let child = f.root.join("local/tracked.txt");
    std::fs::write(&child, b"potentially unsynced bytes").unwrap();
    f.track("local/tracked.txt", ItemKind::File, FileStatus::Local);
    assert!(purge(&f.db, Some(&f.root)).is_err());
    assert_eq!(std::fs::read(child).unwrap(), b"potentially unsynced bytes");
    assert_eq!(f.db.list_files().unwrap().len(), 2);
}

#[test]
fn unsynced_plain_directory_preserves_rows() {
    let f = Fixture::new();
    f.folder("local");
    f.track("local", ItemKind::Folder, FileStatus::Uploading);
    let error = purge(&f.db, Some(&f.root)).unwrap_err();
    assert!(error.to_string().contains("Pending changes remain"));
    assert!(f.root.join("local").is_dir());
    assert_eq!(f.db.list_files().unwrap().len(), 1);
}

#[test]
fn directory_at_file_row_is_not_accepted_as_synced_folder() {
    let f = Fixture::new();
    f.folder("local");
    f.track("local", ItemKind::File, FileStatus::Local);
    let error = purge(&f.db, Some(&f.root)).unwrap_err();
    assert!(
        error.to_string().contains("Item type changed"),
        "must reject the DB/disk kind mismatch before traversal: {error:?}"
    );
    assert!(f.root.join("local").is_dir());
    assert_eq!(f.db.list_files().unwrap().len(), 1);
}

#[test]
fn directory_junction_is_not_traversed_or_removed() {
    let f = Fixture::new();
    let outside = f.temp.path().join("outside");
    std::fs::create_dir(&outside).unwrap();
    std::fs::write(outside.join("precious.txt"), b"outside bytes").unwrap();
    let link = f.root.join("link");
    let output = std::process::Command::new("cmd.exe")
        .args(["/C", "mklink", "/J"])
        .arg(&link)
        .arg(&outside)
        .output()
        .unwrap();
    assert!(output.status.success(), "junction fixture failed: {output:?}");
    f.track("link", ItemKind::Folder, FileStatus::Local);
    let error = purge(&f.db, Some(&f.root)).unwrap_err();
    assert!(
        !error.to_string().contains("Unsynced files remain"),
        "junction was traversed: {error:?}"
    );
    assert!(link.exists());
    assert_eq!(std::fs::read(outside.join("precious.txt")).unwrap(), b"outside bytes");
    assert_eq!(f.db.list_files().unwrap().len(), 1);
    std::fs::remove_dir(link).unwrap();
}

#[test]
fn prepared_directory_cannot_be_replaced_during_cleanup() {
    let f = Fixture::new();
    f.folder("local");
    let rows = f.db.list_files().unwrap();
    let known = rows.iter().map(|r| (r.path.clone(), r)).collect();
    let (mut files, mut directories) = (Vec::new(), Vec::new());
    prepare(&f.root, &f.root, &known, &mut files, &mut directories).unwrap();
    assert_eq!(files.len(), 0);
    assert_eq!(directories.len(), 1);
    assert!(
        std::fs::rename(f.root.join("local"), f.root.join("replaced")).is_err(),
        "prepared directory must remain bound to the validated path"
    );
    drop(directories);
    purge(&f.db, Some(&f.root)).unwrap();
    assert_eq!(f.db.list_files().unwrap().len(), 0);
}

#[test]
fn child_arriving_after_prepare_blocks_nonrecursive_deletion() {
    let f = Fixture::new();
    f.folder("local");
    let rows = f.db.list_files().unwrap();
    let known = rows.iter().map(|r| (r.path.clone(), r)).collect();
    let (mut files, mut directories) = (Vec::new(), Vec::new());
    prepare(&f.root, &f.root, &known, &mut files, &mut directories).unwrap();
    assert_eq!(directories.len(), 1);
    let child = f.root.join("local/late.txt");
    std::fs::write(&child, b"new unsynced bytes").unwrap();
    assert!(
        mark_for_deletion(&directories[0]).is_err(),
        "nonempty directory must survive"
    );
    drop(directories);
    assert_eq!(std::fs::read(&child).unwrap(), b"new unsynced bytes");
    assert_eq!(f.db.list_files().unwrap().len(), 1);
    assert!(purge(&f.db, Some(&f.root)).is_err());
    assert_eq!(std::fs::read(child).unwrap(), b"new unsynced bytes");
}

#[test]
fn regression_1640_r3_empty_upload_staging_signout() {
    let f = Fixture::new();
    let staging = f.root.join(".beebeeb/windows-writes");
    std::fs::create_dir_all(&staging).unwrap();
    // Upload completion removes its payload; these reserved directories remain.
    let payload = staging.join("completed");
    std::fs::write(&payload, b"uploaded bytes").unwrap();
    std::fs::remove_file(payload).unwrap();
    purge(&f.db, Some(&f.root)).expect("empty engine staging must not block sign-out");
    assert_eq!(std::fs::read_dir(&f.root).unwrap().count(), 0);
}

#[test]
fn regression_1640_r3_orphan_staging_refuses_signout() {
    let f = Fixture::new();
    let staging = f.root.join(".beebeeb/windows-writes");
    std::fs::create_dir_all(&staging).unwrap();
    let payload = staging.join("orphan");
    std::fs::write(&payload, b"only copy").unwrap();
    assert!(purge(&f.db, Some(&f.root)).is_err());
    assert_eq!(std::fs::read(payload).unwrap(), b"only copy");
}
