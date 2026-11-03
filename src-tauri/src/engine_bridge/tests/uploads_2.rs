//! `uploads_2` engine tests, split out of `engine_bridge.rs`.

use super::*;
    #[tokio::test]
    async fn test_process_due_operations_uploads_encrypted_payload_and_commits_state() {
        let dir = tempfile::tempdir().unwrap();
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
                target_path: Some("Reports/report.txt".into()),
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

        let outcome = bridge.process_due_operations(dir.path(), 200).await.unwrap();
        assert_eq!(outcome.completed_op_ids, vec!["op-upload-live".to_string()]);
        assert!(outcome.retried_op_ids.is_empty());
        assert!(
            !payload.exists(),
            "staged payload should be removed only after complete succeeds"
        );
        let requests = server.finish();
        let item_id = init_item_id(&requests);
        assert!(bridge.db.list_due_operations(999).unwrap().is_empty());
        assert!(bridge.db.get_file("6cc778db-0fc3-5a51-927a-b6e4a06d4864").unwrap().is_some());

        let entry = bridge.db.get_file(&item_id).unwrap().unwrap();
        assert_eq!(entry.status, FileStatus::Local);
        assert_eq!(entry.path, "Reports/report.txt");
        assert_eq!(entry.size_bytes, 19);

        let contract = bridge.db.get_file_contract_state(&item_id).unwrap().unwrap();
        assert_eq!(contract.current_version, 1);
        assert_eq!(contract.local_base_version, 1);
        assert_eq!(contract.current_object_version_id.as_deref(), Some("object-complete-1"));
        assert_eq!(contract.content_type.as_deref(), Some("text/plain"));

        assert_eq!(requests.len(), 4);
        assert_eq!(requests[0].method, "POST");
        assert_eq!(requests[0].path, "/api/v1/uploads/init");
        let init_body: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(
            init_body["file_id"],
            item_id.as_str(),
            "a create must send the id this device chose, never let the server mint one"
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

    /// Empty (0-byte) files — `.gitkeep`, `__init__.py`, `touch` placeholders —
    /// must sync like any other file. The canonical empty-file plan is ONE chunk
    /// carrying the AEAD of zero bytes (`plan_chunks(0, _)` → `chunk_count == 1`),
    /// so the upload is init(size 0) → one 28-byte chunk PUT → complete, and the
    /// op completes instead of landing in `Error` and being retried.
    #[tokio::test]
    async fn test_process_due_operations_uploads_empty_file_as_one_encrypted_empty_chunk() {
        let dir = tempfile::tempdir().unwrap();
        let payload = dir.path().join("empty.txt");
        std::fs::write(&payload, b"").unwrap();
        let server = UploadMockServer::start(false);
        let master_key = [12u8; 32];
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        bridge
            .db
            .enqueue_operation(&PendingOperation {
                op_id: "op-upload-empty".into(),
                kind: OperationKind::UploadVersion,
                file_id: Some("67f5e5ca-6ef8-5ef9-9264-c618d1f5308a".into()),
                parent_id: Some("846c7080-936d-5ffd-879c-39f630bfb30d".into()),
                target_path: Some("Project/empty.txt".into()),
                metadata_json: Some(
                    serde_json::json!({
                        "operation": "create_file",
                        "name_encrypted": "{\"cipher_suite\":\"V1Aes256Gcm\"}",
                        "display_name": "empty.txt",
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

        let outcome = bridge.process_due_operations(dir.path(), 200).await.unwrap();
        assert!(
            outcome.retried_op_ids.is_empty(),
            "an empty file must not be retried: {:?}",
            bridge.db.queue_diagnostics(200).unwrap().last_error
        );
        assert_eq!(outcome.completed_op_ids, vec!["op-upload-empty".to_string()]);
        assert!(bridge.db.list_due_operations(999).unwrap().is_empty());
        let requests = server.finish();
        let item_id = init_item_id(&requests);
        let entry = bridge.db.get_file(&item_id).unwrap().unwrap();
        assert_eq!(entry.status, FileStatus::Local);
        assert_eq!(entry.path, "Project/empty.txt");

        assert_eq!(requests.len(), 4);
        assert_eq!(requests[0].path, "/api/v1/uploads/init");
        let init_body: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(init_body["file_size_bytes"], 0);
        assert_eq!(init_body["chunk_count"], 1);
        assert_eq!(requests[2].method, "PUT");
        assert_eq!(requests[2].path, "/api/v1/uploads/upload-session-1/chunks/0");
        assert_eq!(requests[2].body.len(), 28, "nonce + tag, no payload");
        let master_key = beebeeb_core::kdf::MasterKey::from_bytes(master_key);
        let file_key = beebeeb_core::kdf::derive_file_key(&master_key, item_id.as_bytes());
        assert!(
            beebeeb_core::encrypt::decrypt_chunk_raw(&file_key, &requests[2].body)
                .unwrap()
                .is_empty()
        );
        assert_eq!(requests[3].path, "/api/v1/uploads/upload-session-1/complete");
    }

    /// Device B side of the empty-file round trip: a server file with
    /// `size_bytes: 0` and one encrypted empty chunk hydrates to a 0-byte file.
    #[test]
    fn hydrate_file_writes_empty_file_from_single_empty_chunk() {
        let dir = tempfile::tempdir().unwrap();
        let master_key = [14u8; 32];
        let file_key = hydration_test_key(master_key, TEST_FILE_ID);
        // 1 metadata GET + 1 chunk GET.
        // + 1 discovery GET (task 2173): this server has no direct route (a JSON 404), so the API path serves it.
        let server = HydrationMockServer::start(file_key, vec![Vec::new()], 3);
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_bridge_row(&bridge, TEST_FILE_ID, "/empty.txt", None, FileStatus::CloudOnly, 0);

        let dest = dir.path().join("empty.txt");
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async { bridge.hydrate_file(TEST_FILE_ID, &dest, &[dir.path()]).await })
            .unwrap();
        server.finish();

        assert_eq!(std::fs::metadata(&dest).unwrap().len(), 0);
        let entry = bridge.db.get_file(TEST_FILE_ID).unwrap().unwrap();
        assert_eq!(entry.status, FileStatus::Local);
    }

    #[tokio::test]
    async fn test_conflict_content_preview_reads_local_disk_and_downloads_real_remote_content() {
        // Task 1546 finding 2: ConflictWindow.tsx's own doc-comment admitted the
        // diff body was a hardcoded placeholder for EVERY conflict ("Content
        // from this device…" / "Content from other device…"), never the
        // file's real content, because "actual diffing needs the daemon to
        // expose both blob bytes". This is that daemon-side exposure.
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        std::fs::write(sync_root.join("notes.txt"), b"local version text").unwrap();

        let master_key = [21u8; 32];
        let file_key = hydration_test_key(master_key, TEST_FILE_ID);
        let remote_chunks = vec![b"remote version text".to_vec()];
        // 1 metadata GET + 1 chunk GET
        // + 1 discovery GET (task 2173): this server has no direct route (a JSON 404), so the API path serves it.
        let server = HydrationMockServer::start(file_key, remote_chunks, 3);

        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        bridge
            .db
            .upsert_file(&FileEntry {
                file_id: TEST_FILE_ID.into(),
                path: "notes.txt".into(),
                status: FileStatus::Conflict,
                size_bytes: 18,
                modified_at: 100,
                content_hash: Some("local-hash".into()),
                remote_updated_at: 90,
                parent_id: None,
                item_kind: ItemKind::File,
            })
            .unwrap();

        let preview = bridge.conflict_content_preview(TEST_FILE_ID, &sync_root).await.unwrap();

        // The two sides must be the REAL, DIFFERENT content, not the old
        // "Content from this device…" / "Content from other device…" pair.
        assert_eq!(preview.local.text.as_deref(), Some("local version text"));
        assert_eq!(preview.remote.text.as_deref(), Some("remote version text"));
        assert!(preview.local.unavailable_reason.is_none());
        assert!(preview.remote.unavailable_reason.is_none());

        // A preview must be read-only: the row is mid-conflict and neither
        // side has been chosen, so daemon bookkeeping (status, cache) must
        // not move just because the user opened the window. This is exactly
        // why the implementation calls `do_hydrate` directly instead of
        // `hydrate_file`/`hydrate_file_to_memory`, which both flip status.
        let entry = bridge.db.get_file(TEST_FILE_ID).unwrap().unwrap();
        assert_eq!(entry.status, FileStatus::Conflict);

        server.finish();
    }

    #[tokio::test]
    async fn test_conflict_content_preview_reports_a_local_read_failure_without_failing_the_whole_call() {
        // The local file may be missing (e.g. deleted outside the daemon) even
        // though the row is Conflict; the remote side must still load so the
        // user isn't left with neither pane.
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        // Deliberately do NOT write sync_root/notes.txt.

        let master_key = [22u8; 32];
        let file_key = hydration_test_key(master_key, TEST_FILE_ID);
        // 1 metadata GET + 1 chunk GET
        // + 1 discovery GET (task 2173): this server has no direct route (a JSON 404), so the API path serves it.
        let server = HydrationMockServer::start(file_key, vec![b"remote only".to_vec()], 3);

        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        bridge
            .db
            .upsert_file(&FileEntry {
                file_id: TEST_FILE_ID.into(),
                path: "notes.txt".into(),
                status: FileStatus::Conflict,
                size_bytes: 0,
                modified_at: 100,
                content_hash: None,
                remote_updated_at: 90,
                parent_id: None,
                item_kind: ItemKind::File,
            })
            .unwrap();

        let preview = bridge.conflict_content_preview(TEST_FILE_ID, &sync_root).await.unwrap();

        assert!(preview.local.text.is_none());
        assert!(preview.local.unavailable_reason.is_some());
        assert_eq!(preview.remote.text.as_deref(), Some("remote only"));

        server.finish();
    }

    #[tokio::test]
    async fn test_conflict_content_preview_skips_remote_download_when_metadata_reports_oversized_file() {
        // Codex round 2, finding 1: the remote size must be checked from
        // metadata BEFORE any chunk is downloaded. `requests: 1` means the
        // mock server serves ONLY the metadata GET and then stops — if the
        // implementation regresses to downloading anyway, the resulting
        // chunk GET either fails to connect (server already exited) or hits
        // a closed listener, so `remote.unavailable_reason` would report a
        // download/connection failure instead of "too large," and this test
        // would fail (not hang: the listener is dropped, not left open).
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        std::fs::write(sync_root.join("notes.txt"), b"small local text").unwrap();

        let master_key = [23u8; 32];
        let file_key = hydration_test_key(master_key, TEST_FILE_ID);
        // The chunk bytes are real (so `size_bytes` in the mocked metadata
        // response is real and over the cap) but must NEVER be fetched.
        let oversized_chunk = vec![b'z'; CONFLICT_PREVIEW_TEXT_MAX_BYTES + 100];
        let expected_size = oversized_chunk.len() as u64;
        let server = HydrationMockServer::start(file_key, vec![oversized_chunk], 1);

        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        bridge
            .db
            .upsert_file(&FileEntry {
                file_id: TEST_FILE_ID.into(),
                path: "notes.txt".into(),
                status: FileStatus::Conflict,
                size_bytes: 16,
                modified_at: 100,
                content_hash: Some("local-hash".into()),
                remote_updated_at: 90,
                parent_id: None,
                item_kind: ItemKind::File,
            })
            .unwrap();

        let preview = bridge.conflict_content_preview(TEST_FILE_ID, &sync_root).await.unwrap();

        assert!(preview.is_text, "notes.txt must classify as text");
        assert!(preview.remote.text.is_none());
        assert_eq!(preview.remote.size_bytes, Some(expected_size));
        let reason = preview
            .remote
            .unavailable_reason
            .expect("must explain why text is absent");
        assert!(reason.contains("too large"), "reason was: {reason}");
        assert!(reason.contains(&expected_size.to_string()), "reason was: {reason}");

        // The local side is untouched by this finding and must still work.
        assert_eq!(preview.local.text.as_deref(), Some("small local text"));

        server.finish();
    }

