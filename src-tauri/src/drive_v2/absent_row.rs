//! Slice 58: a folder the server knows can go to Trash, and a row the server no longer has leaves
//! the local list when the snapshot is applied (instead of staying forever so a later trash fails).

use std::collections::HashSet;

use super::commands::check_trashable;
use super::model::{self, Sort};
use crate::state_db::{self, FileEntry, FileStatus, ItemKind, Namespace, StateDb};

struct Fixture {
    _dir: tempfile::TempDir,
    db: StateDb,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    Fixture { _dir: dir, db }
}

#[allow(clippy::too_many_arguments)]
fn row(db: &StateDb, id: &str, path: &str, parent: Option<&str>, folder: bool, modified: i64, status: FileStatus) {
    let entry = FileEntry {
        file_id: id.to_string(),
        path: path.to_string(),
        status,
        size_bytes: 0,
        modified_at: modified,
        content_hash: None,
        remote_updated_at: modified,
        parent_id: None,
        item_kind: if folder { ItemKind::Folder } else { ItemKind::File },
    };
    db.upsert_file_with_contract(&entry, |c| {
        c.parent_id = parent.map(str::to_string);
        c.item_kind = if folder { ItemKind::Folder } else { ItemKind::File };
        c.namespace = Namespace::MyFiles;
        c.permission_bits = state_db::PERMISSION_OWNER;
        c.current_version = 3;
    })
    .unwrap();
}

/// A folder the server knows (a remote time, not mid-upload, not in conflict) may go to Trash.
#[test]
fn known_folder_is_trashable() {
    let f = fixture();
    row(
        &f.db,
        "folder-1",
        "Budget 2026",
        None,
        true,
        1_700_000_000,
        FileStatus::Local,
    );
    let entry = model::resolve(&f.db, "folder-1").unwrap();
    assert_eq!(check_trashable(&f.db, &entry), Ok(()));
}

/// A row the server no longer has leaves the list when the snapshot is applied, even when its
/// stored name is the engine's blocked label.
#[test]
fn absent_row_leaves() {
    let f = fixture();
    row(
        &f.db,
        "keep",
        "keep.txt",
        None,
        false,
        1_700_000_000,
        FileStatus::CloudOnly,
    );
    row(
        &f.db,
        "stale",
        "Unavailable file 0123456789abcdef",
        None,
        false,
        0,
        FileStatus::Local,
    );
    f.db.enqueue_operation(&state_db::PendingOperation {
        op_id: "op-stale".to_string(),
        kind: state_db::OperationKind::UploadFile,
        file_id: Some("stale".to_string()),
        parent_id: None,
        target_path: None,
        metadata_json: None,
        payload_path: None,
        base_version: None,
        base_object_version_id: None,
        attempts: 5,
        max_attempts: 5,
        next_retry_at: 0,
        last_error: Some("failed".to_string()),
        backup_source_key: None,
        created_at: 0,
        updated_at: 0,
    })
    .unwrap();
    let seen: HashSet<String> = ["keep".to_string()].into_iter().collect();
    let pruned = model::drop_absent_rows(&f.db, &seen, 1_800_000_000).unwrap();
    assert!(
        pruned.iter().any(|r| r.file_id == "stale"),
        "the absent row must be pruned: {pruned:?}"
    );
    let listing = model::list(&f.db, None, Sort::default(), false).unwrap();
    assert!(
        !listing.items.iter().any(|item| item.id == "stale"),
        "the absent row must not be listed"
    );
}
