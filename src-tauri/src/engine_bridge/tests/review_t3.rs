//! T3 review pins (task 2275): the request-row version upload and the foreign
//! item's missing key.
//!
//! Owns: P1-1 (a version upload of a request row becomes a new item) and P2-8
//! (an unknown foreign item never falls back to a derived key).
//! Must not: call `adopt_foreign_subtree` directly or change a stored format.
//! Tests: `review_p1_1_`, `review_p2_8_`.

use super::*;
use crate::owner_keys::OwnedKeyRecord;

const REQUEST: &str = "0f000000-0000-4000-8000-000000000011";
const FOLDER: &str = "40000000-0000-4000-8000-0000000000d1";
const FOREIGN: &str = "20000000-0000-8000-8000-0000000000d2";

#[tokio::test]
async fn review_p1_1_a_version_upload_of_a_request_row_creates_a_new_item() {
    use beebeeb_core::kdf::{MasterKey, derive_file_key};
    let dir = tempfile::tempdir().unwrap();
    let sync_root = dir.path().join("sync");
    std::fs::create_dir_all(&sync_root).unwrap();
    let bridge = test_bridge(&dir.path().join("state.db"));
    seed_folder_row(&bridge, FOLDER, "Inbox");
    seed_bridge_row(
        &bridge,
        REQUEST,
        "Inbox/report.txt",
        Some(FOLDER),
        FileStatus::Local,
        4,
    );

    // The row's name is sealed under the derived string convention, and the row
    // is a 2236 request row (the durable L12 fact P3-3 persists).
    let master = MasterKey::from_bytes([7; 32]);
    let derived = derive_file_key(&master, REQUEST.as_bytes());
    let name = beebeeb_core::encrypt::encrypt_name_for(
        &derived,
        "report.txt",
        None,
        beebeeb_core::chunk_stream::ExpectedChunkFormat::Unrecorded,
    )
    .unwrap();
    bridge
        .db()
        .put_owned_key_record(&OwnedKeyRecord {
            file_id: REQUEST.into(),
            name_encrypted: name.clone(),
            request_row: true,
            ..Default::default()
        })
        .unwrap();

    let payload = dir.path().join("payload.bin");
    std::fs::write(&payload, b"edit").unwrap();
    let op = PendingOperation {
        op_id: "op-request-edit".into(),
        kind: OperationKind::UploadVersion,
        file_id: Some(REQUEST.into()),
        parent_id: Some(FOLDER.into()),
        target_path: Some("Inbox/report.txt".into()),
        metadata_json: Some(
            serde_json::json!({"name_encrypted": name, "content_type": "text/plain"}).to_string(),
        ),
        payload_path: Some(payload.to_string_lossy().into_owned()),
        base_version: None,
        base_object_version_id: None,
        attempts: 0,
        max_attempts: 25,
        next_retry_at: 0,
        last_error: None,
        backup_source_key: None,
        created_at: 0,
        updated_at: 0,
    };
    let mut errors = Vec::new();
    let done = bridge
        .upload_version(&op, None, &sync_root, &mut errors)
        .await
        .expect("a request-row edit is re-planned, not refused");

    assert!(matches!(done, OpDone::Remove { .. }), "the request op is done, not sent");
    let rows = bridge.db().list_files().unwrap();
    let created: Vec<_> = rows
        .iter()
        .filter(|row| row.path == "Inbox/report (edited).txt")
        .collect();
    assert_eq!(created.len(), 1, "a NEW item under a fresh id: {rows:?}");
    assert_ne!(created[0].file_id, REQUEST, "the request row is never the upload target");
    let request = bridge
        .db()
        .get_file(REQUEST)
        .unwrap()
        .expect("the request row still exists");
    assert_eq!(request.path, "Inbox/report.txt", "the original row is unchanged");
    assert!(
        bridge.db().list_operations_for_file(REQUEST).unwrap().is_empty(),
        "no version is uploaded under the request row"
    );
    let ops = bridge.db().list_operations_for_file(&created[0].file_id).unwrap();
    assert!(
        ops.iter().any(|queued| queued.kind == OperationKind::UploadVersion),
        "the new item carries the version upload: {ops:?}"
    );
}

#[test]
fn review_p2_8_an_unknown_foreign_item_has_no_derived_key() {
    let dir = tempfile::tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    let err = match owned_key_and_pin(&db, &[7u8; 32], FOREIGN) {
        Ok(_) => panic!("a foreign item with no record must be unavailable"),
        Err(err) => err,
    };
    assert!(
        err.to_string().contains("item key is unavailable"),
        "a foreign item with no record is unavailable, never derived: {err}"
    );
    let (key, format) = owned_key_and_pin(&db, &[7u8; 32], "40000000-0000-4000-8000-0000000000d3")
        .expect("an owner id still derives its key");
    assert_eq!(format, beebeeb_core::chunk_stream::ExpectedChunkFormat::Unrecorded);
    let _ = key;
}
