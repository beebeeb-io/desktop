//! Task 2445 / night slice 65: a file row whose name cannot be decrypted with
//! the loaded master key survives `prepare_owned_rows` when the canonical fetch
//! answers 404, and appears in `model::list` (blocked, not gone).

use super::*;

/// The fixture serves 404 for every request (the canonical file fetch is gone).
fn lost_fixture() -> Fixture {
    Fixture::wire(|_req| Some((404, serde_json::to_vec(&serde_json::json!({})).unwrap())))
}

#[test]
fn a_file_row_with_an_unreadable_name_survives_the_key_walk() {
    let master = beebeeb_core::kdf::MasterKey::from_bytes([7; 32]);
    // Owner-class ids (version 4): the loaded master key derives the file key.
    let folder_id = "f0000000-0000-4000-8000-0000000000f1";
    let file_id = "a0000000-0000-4000-8000-0000000000a1";

    // The folder's name encrypts under the loaded key [7;32] so it resolves.
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

    // The file's name is sealed under a DIFFERENT key, so the loaded key cannot open it.
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

    // Drive the real path: prepare → snapshot → list.
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
    // The file should show with the blocked name label, not an empty name.
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

    // First pass: the file's name is sealed under the WRONG key.
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

    // First sync: wrong key, row blocked.
    let prepared = run(bridge.prepare_owned_rows(&[folder_row.clone(), file_row_bad])).unwrap();
    let snapshot = crate::api_client::SyncSnapshot {
        seq_id: 1,
        nodes: prepared.into_iter().map(|p| p.row).collect(),
    };
    crate::engine_bridge::syncflow::apply_snapshot(&bridge, temp.path(), &snapshot, 1, 1, &mut Vec::new()).unwrap();

    // Verify the row exists but has an empty path (blocked).
    let entry = bridge.db().get_file(file_id).unwrap().expect("row must exist");
    assert!(
        entry.path.is_empty(),
        "the row's path is empty when the name cannot decrypt"
    );

    // Second sync: the same file with a name sealed under the CORRECT key.
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

    // Now the row must have a real name.
    let listing = crate::drive_v2::model::list(
        bridge.db(),
        Some(folder_id),
        crate::drive_v2::model::Sort::default(),
        false,
    )
    .unwrap();
    let file_item = listing.items.iter().find(|i| i.id == file_id).unwrap();
    assert_eq!(
        file_item.name, "Report.pdf",
        "the healed row shows its real name, not the blocked label"
    );
}
