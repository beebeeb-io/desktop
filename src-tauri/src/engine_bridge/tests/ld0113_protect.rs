//! LD-0113 (spec §5.4, §6.3): the Backups root's `system_role` reaches `state.db`, and the engine
//! never sends a change of a protected row (fixture rows only; nothing here talks to a real server).

use super::*;

const ROOT_ID: &str = "0b5ec0de-0000-4000-8000-0000000000a1";

// The fixture seals the name the way the other metadata tests do (a legacy sealer is fine for a fixture).
#[allow(deprecated)]
fn root_row(master_key: [u8; 32], extra: serde_json::Value) -> serde_json::Value {
    let mk = beebeeb_core::kdf::MasterKey::from_bytes(master_key);
    let name_encrypted = beebeeb_core::encrypt::encrypt_name(&mk, ROOT_ID, "Backups", None).unwrap();
    let mut row = serde_json::json!({
        "id": ROOT_ID,
        "name_encrypted": name_encrypted,
        "size_bytes": 0,
        "updated_at": 7,
        "is_folder": true,
    });
    for (key, value) in extra.as_object().unwrap() {
        row[key] = value.clone();
    }
    row
}

fn apply(db: &StateDb, row: &serde_json::Value, master_key: [u8; 32]) {
    apply_metadata_file_row(
        db,
        row,
        Namespace::MyFiles,
        None,
        None,
        PERMISSION_READ | PERMISSION_WRITE | PERMISSION_OWNER,
        100,
        &master_key,
        "",
    )
    .unwrap()
    .unwrap();
}

/// The listing's field sets and clears the role; a row without the field (an older server, or a row
/// this desktop builds from a sparse op) keeps it; the `folder_system_role` op sets and clears it.
#[test]
fn system_role_follows_the_listing_field_and_the_op() {
    let dir = tempfile::tempdir().unwrap();
    let bridge = test_bridge(&dir.path().join("state.db"));
    let master_key = [7u8; 32];
    let db = bridge.db();

    apply(db, &root_row(master_key, serde_json::json!({})), master_key);
    assert_eq!(db.system_role(ROOT_ID).unwrap(), None, "an older server's row has no role");
    assert!(db.backups_roots().unwrap().is_empty());

    apply(db, &root_row(master_key, serde_json::json!({ "system_role": "backups_root" })), master_key);
    assert_eq!(db.system_role(ROOT_ID).unwrap(), Some(beebeeb_types::backup::SystemRole::BackupsRoot));
    assert_eq!(db.backups_roots().unwrap(), vec![(ROOT_ID.to_string(), "Backups".to_string())]);

    apply(db, &root_row(master_key, serde_json::json!({})), master_key);
    assert_eq!(
        db.system_role(ROOT_ID).unwrap(),
        Some(beebeeb_types::backup::SystemRole::BackupsRoot),
        "a row without the field keeps the role"
    );

    apply(db, &root_row(master_key, serde_json::json!({ "system_role": null })), master_key);
    assert_eq!(db.system_role(ROOT_ID).unwrap(), None, "an explicit null clears it");

    apply(db, &root_row(master_key, serde_json::json!({ "system_role": "Backups" })), master_key);
    assert_eq!(db.system_role(ROOT_ID).unwrap(), None, "a folder NAME is never a role");

    let op = |role: serde_json::Value| crate::api_client::SyncOp {
        seq_id: 9,
        op_type: "folder_system_role".into(),
        payload: serde_json::json!({ "id": ROOT_ID, "system_role": role }),
    };
    let applied = apply_sync_op(&bridge, dir.path(), &op(serde_json::json!("backups_root")), 100, &mut Vec::new()).unwrap();
    assert_eq!(applied, vec![ROOT_ID.to_string()], "the op reports the row it changed");
    assert_eq!(db.system_role(ROOT_ID).unwrap(), Some(beebeeb_types::backup::SystemRole::BackupsRoot));
    let again = apply_sync_op(&bridge, dir.path(), &op(serde_json::json!("backups_root")), 100, &mut Vec::new()).unwrap();
    assert!(again.is_empty(), "the same role again changes nothing");
    apply_sync_op(&bridge, dir.path(), &op(serde_json::Value::Null), 100, &mut Vec::new()).unwrap();
    assert_eq!(db.system_role(ROOT_ID).unwrap(), None, "the op clears it");
}
