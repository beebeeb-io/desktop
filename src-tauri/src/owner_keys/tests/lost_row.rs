//! Task 2445: prod-shaped tests for the files data path.
//!
//! The server snapshot emits `"file_request_id": null, "sender_ephemeral_pubkey": null,
//! "wrapped_content_key": null` for ordinary rows. The old `request_envelope` treated JSON null
//! as "carries an envelope" (`row.get(field).is_some()` is true for null), blocking every row.
//! `updated_at` is an RFC 3339 string, not an integer.

use super::*;

/// A fixture that serves 404 for every canonical file fetch.
fn lost_fixture() -> Fixture {
    Fixture::wire(|_req| Some((404, serde_json::to_vec(&serde_json::json!({})).unwrap())))
}

/// Build a prod-shaped snapshot row: null envelope fields, RFC 3339 updated_at.
fn prod_row(id: &str, parent_id: Option<&str>, name_encrypted: &str, is_folder: bool, size: i64) -> Value {
    json!({
        "id": id,
        "parent_id": parent_id,
        "name_encrypted": name_encrypted,
        "file_request_id": null,
        "sender_ephemeral_pubkey": null,
        "wrapped_content_key": null,
        "acted_via_share_id": null,
        "uploaded_by": null,
        "owner_wrapped_file_key": null,
        "chunk_count": 1,
        "size_bytes": size,
        "is_folder": is_folder,
        "is_uploading": false,
        "content_hash": null,
        "has_thumbnail": false,
        "version_number": 1,
        "storage_pool_id": null,
        "is_trashed": false,
        "updated_at": "2026-11-10T12:00:00Z"
    })
}

#[test]
fn prod_shaped_row_is_readable_and_nested() {
    let master = beebeeb_core::kdf::MasterKey::from_bytes([7; 32]);
    let folder_id = "f0000000-0000-4000-8000-000000000001";
    let file_id = "a0000000-0000-4000-8000-000000000001";

    let folder_name = beebeeb_core::encrypt::encrypt_name(&master, folder_id, "Docs", None).unwrap();
    let file_name = beebeeb_core::encrypt::encrypt_name(&master, file_id, "Report.pdf", None).unwrap();

    let folder = prod_row(folder_id, None, &folder_name, true, 0);
    let file = prod_row(file_id, Some(folder_id), &file_name, false, 2000);

    let fixture = lost_fixture();
    let temp = tempfile::tempdir().unwrap();
    let bridge = crate::engine_bridge::EngineBridge::new(
        Arc::new(StateDb::open(temp.path().join("db")).unwrap()),
        Arc::new(fixture.api()),
    );

    let prepared = run(bridge.prepare_owned_rows(&[folder, file])).unwrap();
    assert_eq!(prepared.len(), 2, "both rows must survive prepare");
    let file_prep = prepared.iter().find(|p| p.row["id"] == file_id).unwrap();
    assert!(file_prep.readable, "the file must be readable with the correct key");
    assert!(
        file_prep.record.blocked_fingerprint.is_none(),
        "the file must not be blocked"
    );

    let snapshot = crate::api_client::SyncSnapshot {
        seq_id: 1,
        nodes: prepared.into_iter().map(|p| p.row).collect(),
    };
    crate::engine_bridge::syncflow::apply_snapshot(&bridge, temp.path(), &snapshot, 1, 1, &mut Vec::new()).unwrap();

    // The file must be listed in the folder with its real name and nested path.
    let listing = crate::drive_v2::model::list(
        bridge.db(),
        Some(folder_id),
        crate::drive_v2::model::Sort::default(),
        false,
    )
    .unwrap();
    let file_item = listing.items.iter().find(|i| i.id == file_id);
    assert!(file_item.is_some(), "the file must be listed in the folder");
    assert_eq!(file_item.unwrap().name, "Report.pdf");

    // The file's stored path must be nested.
    let entry = bridge.db().get_file(file_id).unwrap().expect("file must exist in DB");
    assert_eq!(entry.path, "Docs/Report.pdf", "path must be nested under the folder");

    // updated_at from RFC 3339 must parse (not 0).
    assert!(
        entry.remote_updated_at > 0,
        "RFC 3339 updated_at must parse to a real timestamp, got {}",
        entry.remote_updated_at
    );
}

#[test]
fn request_envelope_null_fields_are_not_carried() {
    // All three fields null → no envelope, Ok(None).
    let row = json!({"file_request_id": null, "sender_ephemeral_pubkey": null, "wrapped_content_key": null});
    assert!(
        crate::owner_keys::requests::request_envelope(&row).unwrap().is_none(),
        "all-null fields must not be treated as carrying an envelope"
    );
    // All three absent → no envelope, Ok(None).
    let row = json!({});
    assert!(crate::owner_keys::requests::request_envelope(&row).unwrap().is_none());
    // A mix of null and non-null → malformed, Err.
    let row = json!({"file_request_id": "some-uuid", "sender_ephemeral_pubkey": null, "wrapped_content_key": null});
    assert!(
        crate::owner_keys::requests::request_envelope(&row).is_err(),
        "a mix of null and non-null is malformed"
    );
}

#[test]
fn never_overwrite_nonempty_path_with_empty() {
    let master = beebeeb_core::kdf::MasterKey::from_bytes([7; 32]);
    let folder_id = "f0000000-0000-4000-8000-000000000002";
    let file_id = "a0000000-0000-4000-8000-000000000002";

    let folder_name = beebeeb_core::encrypt::encrypt_name(&master, folder_id, "Docs", None).unwrap();
    let file_name = beebeeb_core::encrypt::encrypt_name(&master, file_id, "Report.pdf", None).unwrap();

    let fixture = lost_fixture();
    let temp = tempfile::tempdir().unwrap();
    let bridge = crate::engine_bridge::EngineBridge::new(
        Arc::new(StateDb::open(temp.path().join("db")).unwrap()),
        Arc::new(fixture.api()),
    );

    // First sync: correct name.
    let folder = prod_row(folder_id, None, &folder_name, true, 0);
    let file = prod_row(file_id, Some(folder_id), &file_name, false, 2000);
    let prepared = run(bridge.prepare_owned_rows(&[folder.clone(), file])).unwrap();
    let snapshot = crate::api_client::SyncSnapshot {
        seq_id: 1,
        nodes: prepared.into_iter().map(|p| p.row).collect(),
    };
    crate::engine_bridge::syncflow::apply_snapshot(&bridge, temp.path(), &snapshot, 1, 1, &mut Vec::new()).unwrap();
    let entry = bridge.db().get_file(file_id).unwrap().unwrap();
    assert_eq!(entry.path, "Docs/Report.pdf");

    // Second sync: the file's name is now sealed under a WRONG key (simulating vault lock).
    let wrong_master = beebeeb_core::kdf::MasterKey::from_bytes([9; 32]);
    let bad_name = beebeeb_core::encrypt::encrypt_name(&wrong_master, file_id, "Report.pdf", None).unwrap();
    let file_bad = prod_row(file_id, Some(folder_id), &bad_name, false, 2000);
    let prepared2 = run(bridge.prepare_owned_rows(&[folder, file_bad])).unwrap();
    let snapshot2 = crate::api_client::SyncSnapshot {
        seq_id: 2,
        nodes: prepared2.into_iter().map(|p| p.row).collect(),
    };
    crate::engine_bridge::syncflow::apply_snapshot(&bridge, temp.path(), &snapshot2, 2, 2, &mut Vec::new()).unwrap();

    // The path must NOT have been overwritten with an empty string.
    let entry = bridge.db().get_file(file_id).unwrap().unwrap();
    assert_eq!(
        entry.path, "Docs/Report.pdf",
        "a non-empty path must never be overwritten with an empty one"
    );
}

#[test]
fn heal_detects_blocked_rows_and_prepareownedrows_resets_them() {
    let master = beebeeb_core::kdf::MasterKey::from_bytes([7; 32]);
    let file_id = "a0000000-0000-4000-8000-000000000003";
    let file_name = beebeeb_core::encrypt::encrypt_name(&master, file_id, "notes.txt", None).unwrap();

    let fixture = lost_fixture();
    let temp = tempfile::tempdir().unwrap();
    let db = StateDb::open(temp.path().join("db")).unwrap();

    // Simulate a DB left by the old bug: a blocked record + empty path + a set cursor.
    let mut record = crate::owner_keys::OwnedKeyRecord {
        file_id: file_id.into(),
        name_encrypted: file_name.clone(),
        ..Default::default()
    };
    record.blocked_fingerprint = Some(record.fingerprint());
    db.put_owned_key_record(&record).unwrap();
    let entry = FileEntry {
        file_id: file_id.into(),
        path: String::new(),
        status: FileStatus::CloudOnly,
        size_bytes: 100,
        modified_at: 0,
        content_hash: None,
        remote_updated_at: 0,
        parent_id: None,
        item_kind: ItemKind::File,
    };
    db.upsert_file(&entry).unwrap();
    db.set_sync_cursor(42).unwrap();

    // The heal mechanism must detect blocked/nameless rows.
    assert!(!db.envelope_heal_done().unwrap());
    assert!(
        db.has_blocked_or_nameless_rows().unwrap(),
        "must detect blocked records or empty-path rows"
    );

    // Requesting a resnapshot is the heal action.
    db.request_resnapshot().unwrap();
    assert!(
        db.peek_resnapshot_request().unwrap().is_some(),
        "the heal must request a resnapshot"
    );

    // prepare_owned_rows resets blocked_fingerprint on every pass (line 747:
    // `record.blocked_fingerprint = None;`). With the fixed request_envelope,
    // the prod-shaped row resolves and the record is no longer blocked.
    let bridge = crate::engine_bridge::EngineBridge::new(Arc::new(db), Arc::new(fixture.api()));
    let row = prod_row(file_id, None, &file_name, false, 100);
    let prepared = run(bridge.prepare_owned_rows(&[row])).unwrap();
    assert!(
        prepared[0].readable,
        "prepare_owned_rows resets blocked_fingerprint; the row is readable with the correct key"
    );
    assert!(
        prepared[0].record.blocked_fingerprint.is_none(),
        "blocked_fingerprint is cleared by prepare_owned_rows"
    );

    let snapshot = crate::api_client::SyncSnapshot {
        seq_id: 43,
        nodes: prepared.into_iter().map(|p| p.row).collect(),
    };
    crate::engine_bridge::syncflow::apply_snapshot(&bridge, temp.path(), &snapshot, 43, 43, &mut Vec::new()).unwrap();

    let entry = bridge.db().get_file(file_id).unwrap().unwrap();
    assert_eq!(entry.path, "notes.txt", "the healed row must have its real name");

    // Mark heal done; a DB without blocked rows does not request another snapshot.
    bridge.db().mark_envelope_heal_done().unwrap();
    assert!(bridge.db().envelope_heal_done().unwrap());
}

#[test]
fn heal_not_needed_when_no_blocked_rows() {
    let temp = tempfile::tempdir().unwrap();
    let db = StateDb::open(temp.path().join("db")).unwrap();
    assert!(!db.envelope_heal_done().unwrap());
    assert!(
        !db.has_blocked_or_nameless_rows().unwrap(),
        "an empty DB has no blocked rows"
    );
    // The heal logic would skip the resnapshot and just mark done.
    db.mark_envelope_heal_done().unwrap();
    assert!(db.envelope_heal_done().unwrap());
}

#[test]
fn a_file_row_with_an_unreadable_name_survives_the_key_walk() {
    let master = beebeeb_core::kdf::MasterKey::from_bytes([7; 32]);
    let folder_id = "f0000000-0000-4000-8000-0000000000f1";
    let file_id = "a0000000-0000-4000-8000-0000000000a1";

    let folder_name = beebeeb_core::encrypt::encrypt_name(&master, folder_id, "Docs", None).unwrap();
    let folder_row = json!({
        "id": folder_id,
        "name_encrypted": folder_name,
        "parent_id": null,
        "is_folder": true,
        "size_bytes": 0,
        "updated_at": 100,
        "version_number": 1
    });

    let wrong_master = beebeeb_core::kdf::MasterKey::from_bytes([9; 32]);
    let file_name = beebeeb_core::encrypt::encrypt_name(&wrong_master, file_id, "Report.pdf", None).unwrap();
    let file_row = json!({
        "id": file_id,
        "name_encrypted": file_name,
        "parent_id": folder_id,
        "is_folder": false,
        "size_bytes": 2000,
        "updated_at": 200,
        "version_number": 1
    });

    let fixture = lost_fixture();
    let temp = tempfile::tempdir().unwrap();
    let bridge = crate::engine_bridge::EngineBridge::new(
        Arc::new(StateDb::open(temp.path().join("db")).unwrap()),
        Arc::new(fixture.api()),
    );

    let prepared = run(bridge.prepare_owned_rows(&[folder_row, file_row])).unwrap();
    let snapshot = crate::api_client::SyncSnapshot {
        seq_id: 1,
        nodes: prepared.into_iter().map(|p| p.row).collect(),
    };
    crate::engine_bridge::syncflow::apply_snapshot(&bridge, temp.path(), &snapshot, 1, 1, &mut Vec::new()).unwrap();

    let listing = crate::drive_v2::model::list(
        bridge.db(),
        Some(folder_id),
        crate::drive_v2::model::Sort::default(),
        false,
    )
    .unwrap();
    assert!(
        listing.items.iter().any(|item| item.id == file_id),
        "the file row must survive the key walk, not vanish: {:?}",
        listing.items.iter().map(|i| &i.id).collect::<Vec<_>>()
    );
    let file_item = listing.items.iter().find(|i| i.id == file_id).unwrap();
    assert!(
        file_item.name.starts_with("Unavailable file "),
        "a file whose name cannot be decrypted shows the blocked label: {}",
        file_item.name
    );
}

#[test]
fn a_healed_row_gets_its_real_name_on_the_next_sync() {
    let master = beebeeb_core::kdf::MasterKey::from_bytes([7; 32]);
    let folder_id = "f0000000-0000-4000-8000-0000000000f2";
    let file_id = "a0000000-0000-4000-8000-0000000000a2";

    let folder_name = beebeeb_core::encrypt::encrypt_name(&master, folder_id, "Docs", None).unwrap();
    let folder_row = json!({
        "id": folder_id,
        "name_encrypted": folder_name,
        "parent_id": null,
        "is_folder": true,
        "size_bytes": 0,
        "updated_at": 100,
        "version_number": 1
    });

    let wrong_master = beebeeb_core::kdf::MasterKey::from_bytes([9; 32]);
    let bad_name = beebeeb_core::encrypt::encrypt_name(&wrong_master, file_id, "Report.pdf", None).unwrap();
    let file_row_bad = json!({
        "id": file_id,
        "name_encrypted": bad_name,
        "parent_id": folder_id,
        "is_folder": false,
        "size_bytes": 2000,
        "updated_at": 200,
        "version_number": 1
    });

    let fixture = lost_fixture();
    let temp = tempfile::tempdir().unwrap();
    let bridge = crate::engine_bridge::EngineBridge::new(
        Arc::new(StateDb::open(temp.path().join("db")).unwrap()),
        Arc::new(fixture.api()),
    );

    let prepared = run(bridge.prepare_owned_rows(&[folder_row.clone(), file_row_bad])).unwrap();
    let snapshot = crate::api_client::SyncSnapshot {
        seq_id: 1,
        nodes: prepared.into_iter().map(|p| p.row).collect(),
    };
    crate::engine_bridge::syncflow::apply_snapshot(&bridge, temp.path(), &snapshot, 1, 1, &mut Vec::new()).unwrap();

    let entry = bridge.db().get_file(file_id).unwrap().expect("row must exist");
    assert!(
        entry.path.is_empty(),
        "the row's path is empty when the name cannot decrypt"
    );

    let good_name = beebeeb_core::encrypt::encrypt_name(&master, file_id, "Report.pdf", None).unwrap();
    let file_row_good = json!({
        "id": file_id,
        "name_encrypted": good_name,
        "parent_id": folder_id,
        "is_folder": false,
        "size_bytes": 2000,
        "updated_at": 200,
        "version_number": 1
    });

    let prepared2 = run(bridge.prepare_owned_rows(&[folder_row, file_row_good])).unwrap();
    let snapshot2 = crate::api_client::SyncSnapshot {
        seq_id: 2,
        nodes: prepared2.into_iter().map(|p| p.row).collect(),
    };
    crate::engine_bridge::syncflow::apply_snapshot(&bridge, temp.path(), &snapshot2, 2, 2, &mut Vec::new()).unwrap();

    let listing = crate::drive_v2::model::list(
        bridge.db(),
        Some(folder_id),
        crate::drive_v2::model::Sort::default(),
        false,
    )
    .unwrap();
    let file_item = listing.items.iter().find(|i| i.id == file_id).unwrap();
    assert_eq!(file_item.name, "Report.pdf", "the healed row shows its real name");
}
