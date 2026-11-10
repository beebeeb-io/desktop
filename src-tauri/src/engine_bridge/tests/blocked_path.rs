//! The unavailable label stays off the path the watcher uploads.

use super::*;
use crate::owner_keys::{OwnedKeyRecord, blocked_name};

#[test]
fn a_blocked_rows_label_is_not_stored_as_its_path() {
    let dir = tempfile::tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    let file_id = "10000000-0000-4000-8000-0000000000b2";
    let master_key = [7u8; 32];
    db.put_owned_key_record(&OwnedKeyRecord {
        file_id: file_id.into(),
        name_encrypted: "opaque".into(),
        blocked_fingerprint: Some("blocked".into()),
        ..Default::default()
    })
    .unwrap();
    let metadata = serde_json::json!({
        "id": file_id,
        "name_encrypted": "opaque",
        "size_bytes": 12,
        "updated_at": 42,
        "is_folder": false,
    });
    let entry = apply_metadata_file_row(
        &db,
        &metadata,
        Namespace::MyFiles,
        None,
        None,
        PERMISSION_READ | PERMISSION_WRITE | PERMISSION_OWNER,
        100,
        &master_key,
        "docs",
    )
    .unwrap()
    .unwrap();
    let label = blocked_name(file_id);
    assert!(
        !entry.path.contains(&label),
        "the label was stored as the path: {}",
        entry.path
    );
    assert!(entry.path.is_empty(), "path was {}", entry.path);
    assert_eq!(entry.item_kind, ItemKind::File);
}
