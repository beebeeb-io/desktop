//! `finder_queue` engine tests, split out of `engine_bridge.rs`.

use super::*;
/// Spec 2026-10-10 §5.2 step 6: every change the Finder IPC queues reads the sign-out's fence in the transaction
/// that queues it (`accept_finder_write`, `enqueue_finder_change`). Only the engine-internal kinds the spec exempts
/// queue through the unfenced `enqueue_operation`: a version restore, a pin and a hydrate. The IPC's code, whose
/// write arms call the bridge, never calls it.
#[test]
fn only_engine_internal_enqueues_skip_the_sign_out_fence() {
    /// The function named on `line`, when it opens one.
    fn fn_name(line: &str) -> Option<&str> {
        let mut rest = line.trim_start();
        for qualifier in ["pub ", "pub(super) ", "pub ", "const ", "async ", "unsafe "] {
            rest = rest.strip_prefix(qualifier).unwrap_or(rest);
        }
        rest.strip_prefix("fn ")
            .map(|rest| rest.split(['(', '<']).next().unwrap_or(rest))
    }
    /// For each unfenced enqueue in the production part of `source` (before its tests module), the function it
    /// sits in. Comment lines do not count.
    fn unfenced(source: &str) -> Vec<&str> {
        let production = source.split("\nmod tests {").next().unwrap();
        let mut function = "";
        let mut found = Vec::new();
        for line in production.lines() {
            if let Some(name) = fn_name(line) {
                function = name;
            }
            if line.contains("enqueue_operation(") && !line.trim_start().starts_with("//") {
                found.push(function);
            }
        }
        found
    }
    assert_eq!(
        unfenced(&production_tree("engine_bridge")),
        [
            "queue_restore_version",
            "enqueue_pin_tree_operation",
            "enqueue_hydrate_operation"
        ],
        "only a version restore, a pin and a hydrate queue without the fence"
    );
    assert_eq!(
        unfenced(&production_tree("ipc_socket")),
        Vec::<&str>::new(),
        "the IPC never queues without the fence"
    );
}

#[tokio::test]
async fn test_process_due_operations_records_retry_for_upload_worker_handoff() {
    let dir = tempfile::tempdir().unwrap();
    let bridge = test_bridge(&dir.path().join("state.db"));
    seed_bridge_row(
        &bridge,
        "891ade83-2178-5932-91bd-4bea46844faa",
        "Draft.txt",
        None,
        FileStatus::Uploading,
        0,
    );
    bridge
        .db
        .enqueue_operation(&PendingOperation {
            op_id: "op-upload".into(),
            kind: OperationKind::UploadVersion,
            file_id: Some("891ade83-2178-5932-91bd-4bea46844faa".into()),
            parent_id: None,
            target_path: Some("Draft.txt".into()),
            metadata_json: Some(r#"{"operation":"upload_version"}"#.into()),
            payload_path: Some(dir.path().join("payload").to_string_lossy().into_owned()),
            base_version: Some(1),
            base_object_version_id: None,
            attempts: 0,
            max_attempts: 5,
            next_retry_at: 0,
            last_error: None,
            backup_source_key: None,
            created_at: 100,
            updated_at: 100,
        })
        .unwrap();

    let outcome = bridge.process_due_operations(dir.path(), 200).await.unwrap();
    assert_eq!(outcome.retried_op_ids, vec!["op-upload".to_string()]);
    assert!(outcome.paused_op_ids.is_empty());

    let due_too_early = bridge.db.list_due_operations(229).unwrap();
    assert!(due_too_early.is_empty());
    let due = bridge.db.list_due_operations(230).unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].attempts, 1);
    assert!(
        due[0]
            .last_error
            .as_deref()
            .unwrap_or("")
            .contains("staged upload payload is missing")
    );
    assert_eq!(
        bridge
            .db
            .get_file("891ade83-2178-5932-91bd-4bea46844faa")
            .unwrap()
            .unwrap()
            .status,
        FileStatus::Error,
        "a failed/deferred upload must not remain counted as active Uploading"
    );
}

/// A staged-copy read error other than a missing copy retries with a
/// fixed text. The copy's path never reaches `classify_operation_error`, so a path that
/// contains `403` is not taken for a refused request (the class of bug behind task 1252).
#[cfg(unix)]
#[tokio::test]
async fn an_unreadable_staged_copy_is_retried_whatever_its_path_contains() {
    use std::os::unix::fs::PermissionsExt;
    struct RestoreMode(PathBuf);
    impl Drop for RestoreMode {
        fn drop(&mut self) {
            let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o755));
        }
    }
    // Root reads through mode 000, so the precondition cannot be produced there (a CI
    // container runs as root). Say so and skip, rather than fail.
    // SAFETY: `geteuid` takes no arguments, cannot fail and touches no memory.
    if unsafe { libc::geteuid() } == 0 {
        eprintln!("skipped: running as root, which reads a mode-000 directory, so no permission error can be produced");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let bridge = test_bridge(&dir.path().join("state.db"));
    seed_bridge_row(
        &bridge,
        "891ade83-2178-5932-91bd-4bea46844faa",
        "Draft.txt",
        None,
        FileStatus::Uploading,
        0,
    );
    let locked = dir.path().join("staged-403");
    std::fs::create_dir(&locked).unwrap();
    let payload = locked.join("payload");
    std::fs::write(&payload, b"bytes").unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    let _restore = RestoreMode(locked.clone());
    match std::fs::metadata(&payload) {
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {}
        other => panic!("the setup must produce a permission error (not as root): {other:?}"),
    }
    bridge
        .db
        .enqueue_operation(&PendingOperation {
            op_id: "op-upload".into(),
            kind: OperationKind::UploadVersion,
            file_id: Some("891ade83-2178-5932-91bd-4bea46844faa".into()),
            parent_id: None,
            target_path: Some("Draft.txt".into()),
            metadata_json: Some(r#"{"operation":"upload_version"}"#.into()),
            payload_path: Some(payload.to_string_lossy().into_owned()),
            base_version: Some(1),
            base_object_version_id: None,
            attempts: 0,
            max_attempts: 5,
            next_retry_at: 0,
            last_error: None,
            backup_source_key: None,
            created_at: 100,
            updated_at: 100,
        })
        .unwrap();

    let outcome = bridge.process_due_operations(dir.path(), 200).await.unwrap();
    assert!(
        outcome.paused_op_ids.is_empty(),
        "a read error is not a refused request: {outcome:?}"
    );
    assert_eq!(outcome.retried_op_ids, vec!["op-upload".to_string()]);
    let op = bridge.db.get_operation("op-upload").unwrap().unwrap();
    assert_eq!(op.attempts, 1);
    assert_eq!(
        op.last_error.as_deref(),
        Some("staged upload payload could not be read"),
        "a fixed text, without the path"
    );
}

#[cfg(target_os = "windows")]
#[tokio::test]
async fn round7_busy_upload_retries_identity_without_reupload() {
    round7_completed_upload_retry(false).await;
}

#[cfg(target_os = "windows")]
#[tokio::test]
async fn round7_signout_retries_completed_identity_without_reupload() {
    round7_completed_upload_retry(true).await;
}

#[cfg(target_os = "windows")]
async fn round7_completed_upload_retry(signout_retry: bool) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("root");
    std::fs::create_dir_all(&root).unwrap();
    crate::windows_cf::register_test_sync_root(&root).unwrap();
    struct Registration(std::path::PathBuf);
    impl Drop for Registration {
        fn drop(&mut self) {
            crate::windows_cf::unregister_sync_root(&self.0).unwrap();
        }
    }
    let _registration = Registration(root.clone());
    let local = root.join("report.txt");
    std::fs::write(&local, b"live upload payload").unwrap();
    crate::windows_cf::placeholders::convert_to_unsynced_placeholder(&local, "6cc778db-0fc3-5a51-927a-b6e4a06d4864")
        .unwrap();
    use std::os::windows::fs::OpenOptionsExt;
    let held = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(7)
        .open(&local)
        .unwrap();
    let payload = dir.path().join("payload.txt");
    std::fs::write(&payload, b"live upload payload").unwrap();
    let server = UploadMockServer::start(false);
    let master_key = [11u8; 32];
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
    bridge
        .db
        .enqueue_operation(&PendingOperation {
            op_id: "op-upload-live".into(),
            kind: OperationKind::UploadVersion,
            file_id: Some("6cc778db-0fc3-5a51-927a-b6e4a06d4864".into()),
            parent_id: Some("846c7080-936d-5ffd-879c-39f630bfb30d".into()),
            target_path: Some("report.txt".into()),
            metadata_json: Some(
                serde_json::json!({
                    "operation": "create_file",
                    "name_encrypted": "{\"cipher_suite\":\"V1Aes256Gcm\"}",
                    "display_name": "report.txt",
                    "content_type": "text/plain"
                })
                .to_string(),
            ),
            payload_path: Some(payload.to_string_lossy().into_owned()),
            base_version: None,
            base_object_version_id: None,
            attempts: 0,
            max_attempts: 5,
            next_retry_at: 0,
            last_error: None,
            backup_source_key: None,
            created_at: 100,
            updated_at: 100,
        })
        .unwrap();

    let outcome = bridge.process_due_operations(&root, 200).await.unwrap();
    assert_eq!(outcome.completed_op_ids, vec!["op-upload-live".to_string()]);
    assert!(outcome.retried_op_ids.is_empty());
    assert!(
        payload.exists(),
        "completed payload proof must survive a busy identity stamp"
    );
    assert!(bridge.db.list_due_operations(999).unwrap().is_empty());
    assert!(
        bridge
            .db
            .get_file("6cc778db-0fc3-5a51-927a-b6e4a06d4864")
            .unwrap()
            .is_some()
    );
    // SF-0001 (L13): the create lands under the id it sent to init.
    let item_id = {
        let recorded = server.requests.lock().unwrap();
        init_item_id(&recorded)
    };

    let entry = bridge.db.get_file(&item_id).unwrap().unwrap();
    assert_eq!(entry.status, FileStatus::Local);
    assert_eq!(entry.path, "report.txt");
    assert_eq!(entry.size_bytes, 19);

    let contract = bridge.db.get_file_contract_state(&item_id).unwrap().unwrap();
    assert_eq!(contract.current_version, 1);
    assert_eq!(contract.local_base_version, 1);
    assert_eq!(contract.current_object_version_id.as_deref(), Some("object-complete-1"));
    assert_eq!(contract.content_type.as_deref(), Some("text/plain"));

    assert_eq!(bridge.db.upload_finalizations().unwrap().len(), 1);
    assert!(!bridge.db.upload_finalizations().unwrap()[0].stamped);
    drop(held);
    // Retry uses durable state after the bridge and SQLite connection close.
    drop(bridge);
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
    assert_eq!(bridge.db.upload_finalizations().unwrap().len(), 1);
    if !signout_retry {
        // Periodic operation pass has no upload op left to execute.
        bridge.process_due_operations(&root, 300).await.unwrap();
        assert!(!payload.exists(), "proof is removed after successful identity retry");
        assert_eq!(bridge.db.upload_finalizations().unwrap().len(), 0);
    }
    crate::windows_cf::signout::purge(
        &crate::identity::REAL,
        &bridge.db,
        Some(&root),
        &crate::windows_cf::signout::UnknownFiles::Delete,
    )
    .unwrap();
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
    assert!(!payload.exists());
    assert_eq!(bridge.db.upload_finalizations().unwrap().len(), 0);
    let requests = server.finish();
    assert_eq!(requests.len(), 4);
    assert_eq!(requests[0].method, "POST");
    assert_eq!(requests[0].path, "/api/v1/uploads/init");
    let init_body: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert!(
        init_body["file_id"].as_str().is_some_and(is_canonical_item_id),
        "a create sends the canonical id this device chose, never none"
    );
    assert_eq!(init_body["file_size_bytes"], 19);
    assert_eq!(init_body["parent_id"], "846c7080-936d-5ffd-879c-39f630bfb30d");
    assert_eq!(init_body["chunk_count"], 1);
    assert_eq!(requests[1].method, "PATCH");
    assert_eq!(requests[2].method, "PUT");
    assert_eq!(requests[2].path, "/api/v1/uploads/upload-session-1/chunks/0");
    assert_ne!(requests[2].body, b"live upload payload");

    let master_key = beebeeb_core::kdf::MasterKey::from_bytes(master_key);
    let file_key = beebeeb_core::kdf::derive_file_key(&master_key, item_id.as_bytes());
    assert_eq!(
        beebeeb_core::encrypt::decrypt_chunk_raw(&file_key, &requests[2].body).unwrap(),
        b"live upload payload"
    );
    assert_eq!(requests[3].method, "POST");
    assert_eq!(requests[3].path, "/api/v1/uploads/upload-session-1/complete");
}
