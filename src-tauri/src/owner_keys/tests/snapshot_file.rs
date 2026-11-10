//! A file the snapshot listed stays when its detail is missing, if its name opens.
//!
//! Owns: one owner folder sealed under the string id and one owner file sealed under the
//! uppercase id. The detail endpoint answers 404 for both.
//! Must not: rename the unavailable label or drop a row whose name does not open.
//! Tests: `snapshot_file_sealed_uppercase_stays_when_detail_is_missing`.

use super::*;
use beebeeb_core::encrypt::encrypt_name_for;
use beebeeb_core::kdf::{MasterKey, derive_file_key};

const FILE_ID: &str = "10000000-0000-4000-8000-0000000000b2";
const FOLDER_ID: &str = "20000000-0000-4000-8000-000000000001";

fn bridge_for(fixture: &Fixture) -> crate::engine_bridge::EngineBridge {
    let temp = tempfile::tempdir().unwrap();
    let bridge = crate::engine_bridge::EngineBridge::new(
        Arc::new(StateDb::open(temp.path().join("db")).unwrap()),
        Arc::new(fixture.api()),
    );
    std::mem::forget(temp);
    bridge
}

#[test]
fn snapshot_file_sealed_uppercase_stays_when_detail_is_missing() {
    let master = MasterKey::from_bytes([7; 32]);
    let file_key = derive_file_key(&master, FILE_ID.to_ascii_uppercase().as_bytes());
    let file_name = encrypt_name_for(
        &file_key,
        "Contract draft.pdf",
        None,
        beebeeb_core::chunk_stream::ExpectedChunkFormat::Unrecorded,
    )
    .unwrap();
    assert!(
        beebeeb_core::encrypt::decrypt_name_metadata(&master, FILE_ID, &file_name).is_err(),
        "this name must take the detail fetch, which answers 404"
    );
    let folder_key = derive_file_key(&master, FOLDER_ID.as_bytes());
    let folder_name = encrypt_name_for(
        &folder_key,
        "Budget 2026",
        None,
        beebeeb_core::chunk_stream::ExpectedChunkFormat::Unrecorded,
    )
    .unwrap();

    let fixture = Fixture::wire(move |req| {
        let status = if req.method == "GET" && req.path == format!("/api/v1/files/{FILE_ID}") {
            404
        } else {
            200
        };
        Some((status, br#"{"ok":true}"#.to_vec()))
    });
    let bridge = bridge_for(&fixture);
    let rows = [
        json!({
            "id": FOLDER_ID,
            "name_encrypted": folder_name,
            "parent_id": null,
            "is_folder": true,
            "version_number": 1,
            "size_bytes": 0,
            "updated_at": 1
        }),
        json!({
            "id": FILE_ID,
            "name_encrypted": file_name,
            "parent_id": FOLDER_ID,
            "is_folder": false,
            "version_number": 1,
            "size_bytes": 12,
            "updated_at": 1
        }),
    ];
    let prepared = run(bridge.prepare_owned_rows(&rows)).expect("prepare");
    let ids: Vec<_> = prepared.iter().filter_map(|row| row.row["id"].as_str()).collect();
    assert!(ids.contains(&FOLDER_ID), "folder dropped: {ids:?}");
    assert!(ids.contains(&FILE_ID), "the snapshot file was dropped: {ids:?}");
    let file = prepared
        .iter()
        .find(|row| row.row["id"].as_str() == Some(FILE_ID))
        .unwrap();
    assert!(file.readable, "the uppercase name did not open");
    let stored = crate::owner_keys::stored_name(bridge.db(), FILE_ID, &master.to_bytes()).unwrap();
    assert_eq!(stored.as_deref(), Some("Contract draft.pdf"));
}
