//! `uploads_4` engine tests, split out of `engine_bridge.rs`.

use super::*;
    /// Poll the board until `file_id` shows at least `done` bytes, or give up.
    async fn wait_for_upload_bytes(
        board: &Arc<crate::transfer_progress::TransferBoard>,
        file_id: &str,
        done: u64,
        within: Duration,
    ) -> Option<crate::transfer_progress::Transfer> {
        let started = std::time::Instant::now();
        while started.elapsed() < within {
            if let Some(t) = board.get(file_id)
                && t.done >= done
            {
                return Some(t);
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        None
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_upload_reports_its_bytes_chunk_by_chunk_and_a_cut_one_leaves_nothing() {
        // 20 bytes in chunks of 8 (8 + 8 + 4). Chunk 1 hangs, so after chunk 0 is
        // acknowledged the upload sits at 8 of 20: the moment a popover would look.
        let dir = tempfile::tempdir().unwrap();
        let payload = dir.path().join("resume.bin");
        std::fs::write(&payload, b"0123456789abcdefghij").unwrap();
        let server = ResumableUploadMock::start(ResumeMockMode::HangOnce);
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), [21u8; 32]);
        bridge.db.enqueue_operation(&resumable_create_op(&payload)).unwrap();
        let board = bridge.transfers().clone();

        let (cut, seen) = tokio::join!(
            tokio::time::timeout(
                Duration::from_millis(1500),
                bridge.process_due_operations(dir.path(), 200)
            ),
            wait_for_upload_bytes(&board, "local-file-resume", 8, Duration::from_millis(1400)),
        );
        assert!(cut.is_err(), "the first pass is cut mid-upload");
        let seen = seen.expect("the board showed the upload after chunk 0 was acknowledged");
        assert_eq!(
            seen,
            crate::transfer_progress::Transfer {
                direction: crate::transfer_progress::Direction::Up,
                done: 8,
                total: 20
            }
        );
        assert!(board.active().is_empty(), "a cut upload must not linger as in flight");
        assert_eq!(board.finished_bytes(), 0, "a cut upload finishes no bytes");

        let outcome = bridge.process_due_operations(dir.path(), 200).await.unwrap();
        assert_eq!(outcome.completed_op_ids, vec!["op-upload-resume".to_string()]);
        server.finish();
        assert_eq!(
            board.finished_bytes(),
            20,
            "the resumed upload finishes the whole 20 bytes"
        );
        assert!(board.active().is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_resumed_upload_starts_its_bar_at_the_acknowledged_watermark() {
        let dir = tempfile::tempdir().unwrap();
        let payload = dir.path().join("resume.bin");
        std::fs::write(&payload, b"0123456789abcdefghij").unwrap();
        let server = ResumableUploadMock::start(ResumeMockMode::HangOnce);
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), [21u8; 32]);
        let op = resumable_create_op(&payload);
        bridge.db.enqueue_operation(&op).unwrap();
        // A previous run acknowledged chunk 0 (8 bytes) of session-1 and was killed.
        bridge
            .db
            .put_upload_resume(&UploadResume {
                op_id: op.op_id.clone(),
                payload_path: payload.to_string_lossy().into_owned(),
                payload_size: 20,
                payload_mtime_ns: payload_mtime_ns(&payload),
                upload_session_id: "session-1".into(),
                server_file_id: "40000000-0000-4000-8000-0000000000f5".into(),
                object_version_id: "object-init-1".into(),
                chunk_size_bytes: 8,
                chunk_count: 3,
                acked_chunks: 1,
                metadata_applied: true,
                is_create: true,
                // A session the new code opened always chose its own id.
                self_minted: true,
                completed_version: None,
                completed_object_version_id: None,
                completed_mime_type: None,
            })
            .unwrap();
        let board = bridge.transfers().clone();
        let (cut, seen) = tokio::join!(
            tokio::time::timeout(
                Duration::from_millis(1200),
                bridge.process_due_operations(dir.path(), 200)
            ),
            wait_for_upload_bytes(&board, "local-file-resume", 8, Duration::from_millis(1000)),
        );
        assert!(cut.is_err(), "chunk 1 hangs, so the pass is cut");
        let seen = seen.expect("the resumed upload shows on the board");
        assert_eq!((seen.done, seen.total), (8, 20), "it resumes at 8 of 20, not at 0");
        let requests = server.finish();
        assert_eq!(
            count_requests(&requests, "PUT", "/api/v1/uploads/session-1/chunks/0"),
            0,
            "chunk 0 is not re-sent, so 8 bytes can only come from the watermark"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn flow7_gone_session_abandons_orphan_then_reuploads_once() {
        let dir = tempfile::tempdir().unwrap();
        let payload = dir.path().join("resume.bin");
        std::fs::write(&payload, b"0123456789abcdefghij").unwrap();
        let server = ResumableUploadMock::start(ResumeMockMode::HangOnceThenSessionGone);
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), [22u8; 32]);
        bridge.db.enqueue_operation(&resumable_create_op(&payload)).unwrap();

        let cut = tokio::time::timeout(
            Duration::from_millis(1500),
            bridge.process_due_operations(dir.path(), 200),
        )
        .await;
        assert!(cut.is_err(), "the first pass must be cut mid-upload");
        assert!(bridge.db.get_upload_resume("op-upload-resume").unwrap().is_some());

        // Second pass: the persisted session is gone → the orphan create row is
        // abandoned, resume state dropped, and the op scheduled for retry.
        let outcome = bridge.process_due_operations(dir.path(), 200).await.unwrap();
        assert_eq!(outcome.retried_op_ids, vec!["op-upload-resume".to_string()]);
        assert!(bridge.db.get_upload_resume("op-upload-resume").unwrap().is_none());

        // Third pass: a fresh session uploads the file exactly once.
        let outcome = bridge.process_due_operations(dir.path(), 10_000).await.unwrap();
        assert_eq!(outcome.completed_op_ids, vec!["op-upload-resume".to_string()]);

        let requests = server.finish();
        assert_eq!(count_requests(&requests, "POST", "/api/v1/uploads/init"), 2);
        let init_ids: Vec<String> = requests
            .iter()
            .filter(|r| r.method == "POST" && r.path == "/api/v1/uploads/init")
            .map(|r| {
                serde_json::from_slice::<serde_json::Value>(&r.body).unwrap()["file_id"]
                    .as_str()
                    .expect("every init carries the id this device chose")
                    .to_string()
            })
            .collect();
        let first_id = &init_ids[0];
        let second_id = &init_ids[1];
        assert_ne!(first_id, second_id, "each fresh create attempt proposes its own id");
        assert_eq!(
            count_requests(&requests, "POST", &format!("/api/v1/files/{first_id}/upload/abandon")),
            1,
            "the orphaned is_uploading row from the dead session must be abandoned"
        );
        assert_eq!(
            count_requests(&requests, "DELETE", &format!("/api/v1/files/{first_id}")),
            0,
            "abandoning a create session must never trash the item"
        );
        assert_eq!(
            count_requests(&requests, "POST", "/api/v1/uploads/session-2/complete"),
            1
        );
        assert!(bridge.db.get_file(second_id).unwrap().is_some());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn flow7_changed_payload_does_not_resume_stale_session() {
        let dir = tempfile::tempdir().unwrap();
        let payload = dir.path().join("resume.bin");
        std::fs::write(&payload, b"0123456789abcdefghij").unwrap();
        let server = ResumableUploadMock::start(ResumeMockMode::HangOnce);
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), [23u8; 32]);
        bridge.db.enqueue_operation(&resumable_create_op(&payload)).unwrap();

        let cut = tokio::time::timeout(
            Duration::from_millis(1500),
            bridge.process_due_operations(dir.path(), 200),
        )
        .await;
        assert!(cut.is_err());

        // The staged payload changes size before the retry: the persisted
        // chunks belong to other bytes, so the session must NOT be resumed.
        std::fs::write(&payload, b"0123456789abcdefghijKLMN").unwrap();
        let outcome = bridge.process_due_operations(dir.path(), 200).await.unwrap();
        assert_eq!(outcome.completed_op_ids, vec!["op-upload-resume".to_string()]);

        let requests = server.finish();
        assert_eq!(count_requests(&requests, "POST", "/api/v1/uploads/init"), 2);
        let first_id = init_item_id(&requests);
        assert_eq!(
            count_requests(&requests, "POST", &format!("/api/v1/files/{first_id}/upload/abandon")),
            1,
            "the stale create session is abandoned, not resumed"
        );
        assert_eq!(
            count_requests(&requests, "DELETE", &format!("/api/v1/files/{first_id}")),
            0,
            "abandoning a create session must never trash the item"
        );
        assert_eq!(
            count_requests(&requests, "POST", "/api/v1/uploads/session-1/complete"),
            0
        );
        assert_eq!(
            count_requests(&requests, "POST", "/api/v1/uploads/session-2/complete"),
            1
        );
    }

    #[tokio::test]
    async fn flow7_given_up_create_upload_abandons_its_orphan_row() {
        let dir = tempfile::tempdir().unwrap();
        let payload = dir.path().join("payload.txt");
        std::fs::write(&payload, b"retry me").unwrap();
        let server = UploadMockServer::start(true);
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), [24u8; 32]);
        let mut op = resumable_create_op(&payload);
        op.max_attempts = 1;
        bridge.db.enqueue_operation(&op).unwrap();

        let outcome = bridge.process_due_operations(dir.path(), 200).await.unwrap();
        assert_eq!(outcome.retried_op_ids, vec!["op-upload-resume".to_string()]);
        assert!(
            bridge.db.list_due_operations(i64::MAX).unwrap().is_empty(),
            "op exhausted its retries"
        );
        assert!(bridge.db.get_upload_resume("op-upload-resume").unwrap().is_none());

        let requests = server.finish();
        let item_id = init_item_id(&requests);
        assert_eq!(requests.len(), 4);
        assert_eq!(requests[2].path, "/api/v1/uploads/upload-session-1/chunks/0");
        assert_eq!(requests[3].method, "POST");
        assert_eq!(requests[3].path, format!("/api/v1/files/{item_id}/upload/abandon"));
        assert_eq!(
            count_requests(&requests, "DELETE", &format!("/api/v1/files/{item_id}")),
            0,
            "abandoning a create session must never trash the item"
        );
    }

    #[test]
    fn flow7_snapshot_skips_new_is_uploading_rows() {
        let dir = tempfile::tempdir().unwrap();
        let bridge = test_bridge(&dir.path().join("state.db"));
        let mut conflicts = Vec::new();
        let row = serde_json::json!({
            "id": "partial-upload-1",
            "parent_id": null,
            "name": "resume.bin",
            "size_bytes": 50331655,
            "is_folder": false,
            "is_uploading": true,
            "updated_at": 100
        });
        let resolved = process_metadata_row(&bridge, &row, "", 200, RowSource::Snapshot, &mut conflicts).unwrap();
        assert!(resolved.is_none());
        assert!(
            bridge.db.get_file("partial-upload-1").unwrap().is_none(),
            "an in-progress upload has no content yet and must not become a placeholder"
        );
    }

