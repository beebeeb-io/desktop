//! `conflicts` engine tests, split out of `engine_bridge.rs`.

use super::*;
#[tokio::test]
async fn test_conflict_content_preview_local_bounded_read_reports_too_large_with_accurate_size() {
    // Codex round 2, finding 1's local half: a local file over the cap
    // must report "too large" with the REAL size (from `fs::metadata`,
    // not from reading the whole file — `local_content_preview` never
    // calls `std::fs::read` on an oversized file).
    let dir = tempfile::tempdir().unwrap();
    let sync_root = dir.path().join("sync-root");
    std::fs::create_dir_all(&sync_root).unwrap();
    let oversized = vec![b'x'; CONFLICT_PREVIEW_TEXT_MAX_BYTES + 1];
    let expected_size = oversized.len() as u64;
    std::fs::write(sync_root.join("notes.txt"), &oversized).unwrap();

    let master_key = [24u8; 32];
    let file_key = hydration_test_key(master_key, TEST_FILE_ID);
    // 1 metadata GET + 1 chunk GET
    // + 1 discovery GET (task 2173): this server has no direct route (a JSON 404), so the API path serves it.
    let server = HydrationMockServer::start(file_key, vec![b"remote version text".to_vec()], 3);

    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
    bridge
        .db
        .upsert_file(&FileEntry {
            file_id: TEST_FILE_ID.into(),
            path: "notes.txt".into(),
            status: FileStatus::Conflict,
            size_bytes: expected_size as i64,
            modified_at: 100,
            content_hash: Some("local-hash".into()),
            remote_updated_at: 90,
            parent_id: None,
            item_kind: ItemKind::File,
        })
        .unwrap();

    let preview = bridge.conflict_content_preview(TEST_FILE_ID, &sync_root).await.unwrap();

    assert!(preview.local.text.is_none());
    assert_eq!(preview.local.size_bytes, Some(expected_size));
    let reason = preview
        .local
        .unavailable_reason
        .expect("must explain why text is absent");
    assert!(reason.contains("too large"), "reason was: {reason}");
    assert!(reason.contains(&expected_size.to_string()), "reason was: {reason}");

    // The remote side (small, real content) is untouched by this finding.
    assert_eq!(preview.remote.text.as_deref(), Some("remote version text"));

    server.finish();
}

#[tokio::test]
async fn test_conflict_content_preview_determines_textness_from_the_file_path_not_a_caller_flag() {
    // Codex round 2, finding 3: `conflict_content_preview` no longer
    // takes an `is_text` parameter at all — the daemon decides from the
    // row's own `path` via `is_text_file`. This is exactly the bug:
    // VersionCenter's `open_conflict_window` call always hardcoded
    // `isText: false`, which (before this fix) discarded valid text
    // content for every conflict opened that way. Here the path has a
    // binary extension (`.jpg`) even though the bytes on both sides
    // happen to be valid UTF-8 — the response must still classify as
    // binary and never surface `text`, proving the decision comes from
    // the path, not from any UTF-8-validity heuristic on the bytes.
    let dir = tempfile::tempdir().unwrap();
    let sync_root = dir.path().join("sync-root");
    std::fs::create_dir_all(&sync_root).unwrap();
    std::fs::write(sync_root.join("photo.jpg"), b"not really jpeg bytes").unwrap();

    let master_key = [25u8; 32];
    let file_key = hydration_test_key(master_key, TEST_FILE_ID);
    // requests: 1 — a binary preview must fetch ONLY the metadata GET
    // (to learn `size_bytes`) and never a chunk GET, so the mock server
    // must never need to serve a 2nd request. If the implementation
    // regresses to always downloading, that 2nd request fails to
    // connect (the server thread already exited after 1) rather than
    // hanging, so this stays a fast, deterministic red, not a hang.
    let server = HydrationMockServer::start(file_key, vec![b"also not really jpeg bytes".to_vec()], 1);

    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
    bridge
        .db
        .upsert_file(&FileEntry {
            file_id: TEST_FILE_ID.into(),
            path: "photo.jpg".into(),
            status: FileStatus::Conflict,
            size_bytes: 21,
            modified_at: 100,
            content_hash: Some("local-hash".into()),
            remote_updated_at: 90,
            parent_id: None,
            item_kind: ItemKind::File,
        })
        .unwrap();

    let preview = bridge.conflict_content_preview(TEST_FILE_ID, &sync_root).await.unwrap();

    assert!(!preview.is_text, "a .jpg path must classify as binary");
    assert!(preview.local.text.is_none());
    assert!(preview.remote.text.is_none());
    assert!(
        preview.local.unavailable_reason.is_none(),
        "binary is an expected, not an error, state"
    );
    assert!(
        preview.remote.unavailable_reason.is_none(),
        "binary is an expected, not an error, state"
    );
    assert_eq!(preview.local.size_bytes, Some(21));
    assert_eq!(preview.remote.size_bytes, Some(26));

    server.finish();
}

#[tokio::test]
async fn test_resolve_keep_mine_uploads_local_payload_and_commits_state() {
    let dir = tempfile::tempdir().unwrap();
    let sync_root = dir.path().join("sync-root");
    std::fs::create_dir_all(sync_root.join("Docs")).unwrap();
    let local_path = sync_root.join("Docs/conflict.txt");
    std::fs::write(&local_path, b"mine upload payload").unwrap();

    let server = UploadMockServer::start(false);
    let master_key = [13u8; 32];
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
    bridge
        .db
        .upsert_file(&FileEntry {
            file_id: "6d2d20bc-321b-533d-931c-778b0b34a9fc".into(),
            path: "Docs/conflict.txt".into(),
            status: FileStatus::Conflict,
            size_bytes: 99,
            modified_at: 100,
            content_hash: Some("local-conflict-hash".into()),
            remote_updated_at: 90,
            parent_id: None,
            item_kind: ItemKind::File,
        })
        .unwrap();
    let mut contract = bridge
        .db
        .get_file_contract_state("6d2d20bc-321b-533d-931c-778b0b34a9fc")
        .unwrap()
        .unwrap();
    contract.parent_id = Some("folder-1".into());
    contract.content_type = Some("text/plain".into());
    contract.current_version = 3;
    contract.local_base_version = 2;
    contract.current_object_version_id = Some("object-before".into());
    bridge.db.set_file_contract_state(&contract).unwrap();

    let before = now_secs();
    bridge
        .resolve_keep_mine("6d2d20bc-321b-533d-931c-778b0b34a9fc", &sync_root)
        .await
        .unwrap();

    let entry = bridge
        .db
        .get_file("6d2d20bc-321b-533d-931c-778b0b34a9fc")
        .unwrap()
        .unwrap();
    assert_eq!(entry.status, FileStatus::Local);
    assert_eq!(entry.path, "Docs/conflict.txt");
    assert_eq!(entry.size_bytes, 19);
    assert_eq!(entry.content_hash.as_deref(), Some("local-conflict-hash"));
    assert!(entry.remote_updated_at >= before);

    let contract = bridge
        .db
        .get_file_contract_state("6d2d20bc-321b-533d-931c-778b0b34a9fc")
        .unwrap()
        .unwrap();
    assert_eq!(contract.current_version, 1);
    assert_eq!(contract.local_base_version, 1);
    assert_eq!(contract.current_object_version_id.as_deref(), Some("object-complete-1"));
    assert_eq!(contract.parent_id.as_deref(), Some("folder-1"));
    assert_eq!(contract.content_type.as_deref(), Some("text/plain"));

    let requests = server.finish();
    // A new version of an existing file reads the row first: its record as the server holds it.
    assert_eq!(requests.len(), 5);
    assert_eq!(
        (requests[0].method.as_str(), requests[0].path.as_str()),
        ("GET", "/api/v1/files/6d2d20bc-321b-533d-931c-778b0b34a9fc")
    );
    let requests = &requests[1..];
    assert_eq!(requests[0].method, "POST");
    assert_eq!(requests[0].path, "/api/v1/uploads/init");
    let init_body: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(init_body["file_id"], "6d2d20bc-321b-533d-931c-778b0b34a9fc");
    assert_eq!(init_body["file_size_bytes"], 19);
    assert_eq!(init_body["parent_id"], "folder-1");
    assert_eq!(init_body["chunk_count"], 1);
    assert!(
        init_body.get("base_version_number").is_none(),
        "Keep Mine is an explicit conflict override, not a stale-base retry"
    );
    assert_eq!(requests[1].method, "PATCH");
    assert_eq!(requests[1].path, "/api/v1/files/6d2d20bc-321b-533d-931c-778b0b34a9fc");
    assert_eq!(requests[2].method, "PUT");
    assert_eq!(requests[2].path, "/api/v1/uploads/upload-session-1/chunks/0");
    assert_ne!(requests[2].body, b"mine upload payload");

    let master_key = beebeeb_core::kdf::MasterKey::from_bytes(master_key);
    let file_key = beebeeb_core::kdf::derive_file_key(&master_key, b"6d2d20bc-321b-533d-931c-778b0b34a9fc");
    assert_eq!(
        uploaded_plaintext(requests, &file_key, "6d2d20bc-321b-533d-931c-778b0b34a9fc"),
        b"mine upload payload"
    );
    assert_eq!(requests[3].method, "POST");
    assert_eq!(requests[3].path, "/api/v1/uploads/upload-session-1/complete");
}

#[tokio::test]
async fn test_resolve_keep_mine_returns_error_when_upload_fails() {
    let dir = tempfile::tempdir().unwrap();
    let sync_root = dir.path().join("sync-root");
    std::fs::create_dir_all(&sync_root).unwrap();
    std::fs::write(sync_root.join("conflict.txt"), b"retry me").unwrap();

    let server = UploadMockServer::start(true);
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), [14u8; 32]);
    bridge
        .db
        .upsert_file(&FileEntry {
            file_id: "6d2d20bc-321b-533d-931c-778b0b34a9fc".into(),
            path: "conflict.txt".into(),
            status: FileStatus::Conflict,
            size_bytes: 8,
            modified_at: 100,
            content_hash: Some("local-conflict-hash".into()),
            remote_updated_at: 90,
            parent_id: None,
            item_kind: ItemKind::File,
        })
        .unwrap();
    let mut contract = bridge
        .db
        .get_file_contract_state("6d2d20bc-321b-533d-931c-778b0b34a9fc")
        .unwrap()
        .unwrap();
    contract.content_type = Some("text/plain".into());
    contract.current_version = 3;
    contract.local_base_version = 2;
    bridge.db.set_file_contract_state(&contract).unwrap();

    let err = bridge
        .resolve_keep_mine("6d2d20bc-321b-533d-931c-778b0b34a9fc", &sync_root)
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("500 Internal Server Error"));

    let entry = bridge
        .db
        .get_file("6d2d20bc-321b-533d-931c-778b0b34a9fc")
        .unwrap()
        .unwrap();
    assert_eq!(entry.status, FileStatus::Conflict);
    assert_eq!(entry.remote_updated_at, 90);

    let requests = server.finish();
    assert_eq!(
        requests
            .iter()
            .map(|r| (r.method.as_str(), r.path.as_str()))
            .collect::<Vec<_>>(),
        vec![
            // The row as the server holds it, then the session and the refused chunk. No metadata
            // update: the file stays where it is, and its name travels with `complete`.
            ("GET", "/api/v1/files/6d2d20bc-321b-533d-931c-778b0b34a9fc"),
            ("POST", "/api/v1/uploads/init"),
            ("PUT", "/api/v1/uploads/upload-session-1/chunks/0"),
        ]
    );
}

#[tokio::test]
async fn audit_1244_auto_resolve_keep_both_preserves_local_copy_when_remote_hydrate_fails() {
    // Production mutation caught: hydrating before renaming, or leaving the row
    // in Conflict after a hydrate failure, would either risk the local bytes or
    // hide the retry state from the normal Error flow.
    let dir = tempfile::tempdir().unwrap();
    let sync_root = dir.path().join("sync-root");
    std::fs::create_dir_all(&sync_root).unwrap();
    let original = sync_root.join("conflict.txt");
    std::fs::write(&original, b"local conflict bytes").unwrap();

    let file_id = "bbbb0000-0000-4000-8000-000000000010";
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), "http://127.0.0.1:9".into(), [15u8; 32]);
    bridge
        .db
        .upsert_file(&FileEntry {
            file_id: file_id.into(),
            path: "conflict.txt".into(),
            status: FileStatus::Conflict,
            size_bytes: 20,
            modified_at: 100,
            content_hash: Some("local-conflict-hash".into()),
            remote_updated_at: 90,
            parent_id: None,
            item_kind: ItemKind::File,
        })
        .unwrap();
    let entry = bridge.db.get_file(file_id).unwrap().unwrap();

    let err = bridge
        .auto_resolve_keep_both(&sync_root, &entry)
        .await
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("error sending request") || err.contains("Connection refused"),
        "actual hydration error: {err}"
    );

    let row = bridge.db.get_file(file_id).unwrap().unwrap();
    assert_eq!(row.status, FileStatus::Error);
    assert!(!original.exists(), "original path is left vacant for the remote retry");
    let conflict_copy = std::fs::read_dir(&sync_root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("conflict (conflict - ") && name.ends_with(".txt"))
        })
        .expect("local conflict copy should be renamed with a device/date suffix");
    assert_eq!(std::fs::read(conflict_copy).unwrap(), b"local conflict bytes");
}

#[tokio::test]
async fn test_process_due_operations_uploads_encrypted_image_thumbnails_after_complete() {
    let dir = tempfile::tempdir().unwrap();
    let payload = dir.path().join("photo.png");
    write_test_png(&payload);

    let server = UploadMockServer::start(false);
    let master_key = [31u8; 32];
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
    bridge
        .db
        .enqueue_operation(&PendingOperation {
            op_id: "op-upload-photo".into(),
            kind: OperationKind::UploadVersion,
            file_id: Some("c5aa9a16-7bdc-5b79-a7ac-9469701c135c".into()),
            parent_id: None,
            target_path: Some("Photos/photo.png".into()),
            metadata_json: Some(
                serde_json::json!({
                    "operation": "create_file",
                    "name_encrypted": "{\"cipher_suite\":\"V1Aes256Gcm\"}",
                    "display_name": "photo.png",
                    "content_type": "image/png"
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

    let outcome = bridge.process_due_operations(dir.path(), 200).await.unwrap();
    assert_eq!(outcome.completed_op_ids, vec!["op-upload-photo".to_string()]);
    // Task 1700: if post-complete thumbnail work failed, say WHY here
    // instead of letting the expects below fire blind.
    assert!(
        outcome.post_complete_errors.is_empty(),
        "post-complete thumbnail work failed: {:?}",
        outcome.post_complete_errors
    );

    let requests = server.finish();
    let item_id = init_item_id(&requests);
    let recorded: Vec<String> = requests.iter().map(|r| format!("{} {}", r.method, r.path)).collect();
    let medium = requests
        .iter()
        .find(|r| {
            r.method == "PUT"
                && r.path
                    .starts_with(&format!("/api/v1/files/{item_id}/thumbnail?blurhash="))
        })
        .unwrap_or_else(|| panic!("medium thumbnail upload with blurhash query — recorded requests: {recorded:?}"));
    let large = requests
        .iter()
        .find(|r| r.method == "PUT" && r.path == format!("/api/v1/files/{item_id}/thumbnail/large"))
        .unwrap_or_else(|| panic!("large thumbnail upload — recorded requests: {recorded:?}"));

    let master_key = beebeeb_core::kdf::MasterKey::from_bytes(master_key);
    let file_key = beebeeb_core::kdf::derive_file_key(&master_key, item_id.as_bytes());
    let medium_plain = beebeeb_core::encrypt::decrypt_thumbnail(&file_key, &medium.body).unwrap();
    let large_plain = beebeeb_core::encrypt::decrypt_thumbnail(&file_key, &large.body).unwrap();

    assert_ne!(medium.body, medium_plain);
    assert_ne!(large.body, large_plain);
    assert!(medium_plain.starts_with(b"RIFF"), "medium thumbnail must be WebP");
    assert!(large_plain.starts_with(b"RIFF"), "large thumbnail must be WebP");

    let complete_index = requests
        .iter()
        .position(|r| r.method == "POST" && r.path == "/api/v1/uploads/upload-session-1/complete")
        .unwrap();
    let medium_index = requests.iter().position(|r| std::ptr::eq(r, medium)).unwrap();
    let large_index = requests.iter().position(|r| std::ptr::eq(r, large)).unwrap();
    assert!(medium_index > complete_index);
    assert!(large_index > complete_index);
}

/// Task 1700 guard: when post-complete thumbnail upload work fails, the
/// failure must be VISIBLE on `TransferLoopOutcome::post_complete_errors`
/// — never only in a `tracing::warn!` that no test subscriber captures.
/// Product law is unchanged: the failed thumbnail still never fails the
/// upload itself. Red-if-broken: removing the outcome plumbing (swallow
/// silently again) fails the `post_complete_errors` assertions below.
#[tokio::test]
async fn test_process_due_operations_reports_failed_thumbnail_upload_in_outcome() {
    let dir = tempfile::tempdir().unwrap();
    let payload = dir.path().join("photo.png");
    write_test_png(&payload);

    let server = UploadMockServer::start_failing_thumbnail_uploads(false);
    let master_key = [31u8; 32];
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
    bridge
        .db
        .enqueue_operation(&PendingOperation {
            op_id: "op-upload-photo-thumb-fail".into(),
            kind: OperationKind::UploadVersion,
            file_id: Some("c5aa9a16-7bdc-5b79-a7ac-9469701c135c".into()),
            parent_id: None,
            target_path: Some("Photos/photo.png".into()),
            metadata_json: Some(
                serde_json::json!({
                    "operation": "create_file",
                    "name_encrypted": "{\"cipher_suite\":\"V1Aes256Gcm\"}",
                    "display_name": "photo.png",
                    "content_type": "image/png"
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

    let outcome = bridge.process_due_operations(dir.path(), 200).await.unwrap();

    // The upload itself still completes — post-complete work is
    // best-effort by contract.
    assert_eq!(
        outcome.completed_op_ids,
        vec!["op-upload-photo-thumb-fail".to_string()],
        "a failed thumbnail must never fail the upload"
    );
    // But the failure is no longer swallowed: the outcome names it.
    assert_eq!(
        outcome.post_complete_errors.len(),
        1,
        "the failed thumbnail upload must be reported exactly once: {outcome:?}"
    );
    assert!(
        outcome.post_complete_errors[0].contains("op-upload-photo-thumb-fail"),
        "the report carries the op id: {:?}",
        outcome.post_complete_errors
    );
    assert!(
        outcome.post_complete_errors[0].contains("upload medium thumbnail"),
        "the report names the variant that failed: {:?}",
        outcome.post_complete_errors
    );
    assert!(
        outcome.post_complete_errors[0].contains("HTTP 500"),
        "the report carries the server's status: {:?}",
        outcome.post_complete_errors
    );

    // The mock recorded the medium PUT before answering 500 — the
    // failure is server-side, not a lost request. Medium is attempted
    // first and its failure short-circuits, so large is never sent.
    let requests = server.finish();
    let item_id = init_item_id(&requests);
    assert!(
        requests.iter().any(|r| r.method == "PUT"
            && r.path
                .starts_with(&format!("/api/v1/files/{item_id}/thumbnail?blurhash="))),
        "the medium thumbnail PUT must reach the server: {:?}",
        requests
            .iter()
            .map(|r| format!("{} {}", r.method, r.path))
            .collect::<Vec<_>>()
    );
    assert!(
        !requests
            .iter()
            .any(|r| r.method == "PUT" && r.path == format!("/api/v1/files/{item_id}/thumbnail/large")),
        "the medium failure short-circuits before the large variant"
    );
}
