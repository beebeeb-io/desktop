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

fn flagged_root(bridge: &EngineBridge, status: FileStatus) {
    bridge
        .db
        .upsert_file(&FileEntry {
            file_id: ROOT_ID.into(),
            path: "Backups".into(),
            status,
            size_bytes: 0,
            modified_at: 0,
            content_hash: None,
            remote_updated_at: 0,
            parent_id: None,
            item_kind: ItemKind::Folder,
        })
        .unwrap();
    bridge
        .db
        .set_system_role(ROOT_ID, Some(beebeeb_types::backup::SystemRole::BackupsRoot))
        .unwrap();
}

fn trash_op(op_id: &str) -> PendingOperation {
    PendingOperation {
        op_id: op_id.into(),
        kind: OperationKind::TrashFile,
        file_id: Some(ROOT_ID.into()),
        parent_id: None,
        target_path: None,
        metadata_json: Some(serde_json::json!({ "operation": "trash" }).to_string()),
        payload_path: None,
        base_version: None,
        base_object_version_id: None,
        attempts: 0,
        max_attempts: 25,
        next_retry_at: 0,
        last_error: None,
        backup_source_key: None,
        created_at: 100,
        updated_at: 100,
    }
}

/// Spec §6.3: an op the server refuses with `protected_folder` (one an older client queued, or one
/// queued before this device knew the flag) is dropped at once, never retried or held, and the row
/// is live again. Any other 409 keeps today's retry.
#[tokio::test]
async fn a_protected_folder_refusal_drops_the_op_and_puts_the_row_back() {
    let dir = tempfile::tempdir().unwrap();
    let server = SyncMockServer::start(vec![
        (
            "409 Conflict".into(),
            serde_json::json!({ "error": "protected_folder", "message": beebeeb_core::backup::MSG_DELETE }),
        ),
        ("409 Conflict".into(), serde_json::json!({ "error": "conflict" })),
    ]);
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), [9u8; 32]);
    flagged_root(&bridge, FileStatus::Trashing);
    bridge.db.enqueue_operation(&trash_op("op-protected")).unwrap();

    let outcome = bridge.process_due_operations(dir.path(), 200).await.unwrap();
    assert!(outcome.completed_op_ids.is_empty(), "a refusal is not a completion");
    assert!(outcome.retried_op_ids.is_empty(), "a refusal is never retried");
    assert!(outcome.paused_op_ids.is_empty(), "a refusal is never held");
    assert!(bridge.db.get_operation("op-protected").unwrap().is_none(), "the op is dropped");
    assert_eq!(bridge.db.get_file(ROOT_ID).unwrap().unwrap().status, FileStatus::CloudOnly, "the row is live again");

    // The same 409 status with another body is not a protection refusal: the op stays and retries.
    bridge.db.set_status(ROOT_ID, FileStatus::Trashing).unwrap();
    bridge.db.enqueue_operation(&trash_op("op-other-conflict")).unwrap();
    let outcome = bridge.process_due_operations(dir.path(), 300).await.unwrap();
    assert_eq!(outcome.retried_op_ids, vec!["op-other-conflict".to_string()]);
    assert!(bridge.db.get_operation("op-other-conflict").unwrap().is_some());
    let requests = server.finish();
    assert_eq!(requests.len(), 2, "one trash request per op");
}

/// Spec §6.3 (refused after the fact): a local delete of the root that got through queues nothing for
/// the server and leaves the row live; an ordinary folder's delete still queues its trash.
#[tokio::test]
async fn a_delete_of_the_root_that_got_through_queues_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let sync_root = dir.path().join("root");
    std::fs::create_dir_all(&sync_root).unwrap();
    let bridge = test_bridge(&dir.path().join("state.db"));
    flagged_root(&bridge, FileStatus::CloudOnly);
    bridge
        .db
        .upsert_file(&FileEntry {
            file_id: "0b5ec0de-0000-4000-8000-0000000000a2".into(),
            path: "Documents".into(),
            status: FileStatus::CloudOnly,
            size_bytes: 0,
            modified_at: 0,
            content_hash: None,
            remote_updated_at: 0,
            parent_id: None,
            item_kind: ItemKind::Folder,
        })
        .unwrap();

    crate::watcher::handle_delete(&bridge, &sync_root, &sync_root.join("Backups")).await;
    assert!(bridge.db.list_operations_for_file(ROOT_ID).unwrap().is_empty(), "nothing is queued for the root");
    assert_eq!(bridge.db.get_file(ROOT_ID).unwrap().unwrap().status, FileStatus::CloudOnly, "the root stays live");

    crate::watcher::handle_rename(&bridge, &sync_root, &sync_root.join("Backups"), &sync_root.join("Old backups")).await;
    assert!(bridge.db.list_operations_for_file(ROOT_ID).unwrap().is_empty(), "nor for its rename");

    crate::watcher::handle_delete(&bridge, &sync_root, &sync_root.join("Documents")).await;
    assert_eq!(
        bridge.db.list_operations_for_file("0b5ec0de-0000-4000-8000-0000000000a2").unwrap().len(),
        1,
        "an ordinary folder's delete still queues its trash"
    );
}
