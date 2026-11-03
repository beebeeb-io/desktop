//! `hydration` engine tests, split out of `engine_bridge.rs`.

use super::*;
    /// Metadata missing `chunk_count` (or it's `0`) must fall back cleanly —
    /// `Ok(None)`, never an error — so the caller can retry via the whole-file
    /// `hydrate_file_to_memory` path.
    #[test]
    fn hydrate_file_range_falls_back_when_chunk_count_absent() {
        let dir = tempfile::tempdir().unwrap();
        let master_key = [3u8; 32];
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let _ = read_http_request(&mut stream);
            let body = http_json(
                "200 OK",
                serde_json::json!({ "id": TEST_FILE_ID, "size_bytes": 100 }), // no chunk_count
            );
            stream.write_all(body.as_bytes()).unwrap();
        });

        let bridge = test_bridge_with_api(&dir.path().join("state.db"), base_url, master_key);
        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async { bridge.hydrate_file_range(TEST_FILE_ID, 0, 10).await })
            .unwrap();
        assert!(result.is_none());

        handle.join().unwrap();
    }

    #[test]
    fn hydrate_file_range_falls_back_when_chunk_size_absent_even_if_size_and_count_present() {
        let dir = tempfile::tempdir().unwrap();
        let master_key = [4u8; 32];
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let _ = read_http_request(&mut stream);
            let body = http_json(
                "200 OK",
                serde_json::json!({
                    "id": TEST_FILE_ID,
                    "size_bytes": 24,
                    "chunk_count": 3,
                }),
            );
            stream.write_all(body.as_bytes()).unwrap();
        });

        let bridge = test_bridge_with_api(&dir.path().join("state.db"), base_url, master_key);
        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async { bridge.hydrate_file_range(TEST_FILE_ID, 0, 10).await })
            .unwrap();
        assert!(result.is_none());

        handle.join().unwrap();
    }

    /// R4 (task 1834 fix round 2), and the move of the macOS staging root (spec §8.4): the directories the
    /// account reset and the sign-out purge sweep are exactly the places the engine stages into now and staged
    /// into before. On macOS: the data root, then the cache root and the temp root that earlier builds used.
    /// Elsewhere: the cache root and its temp fallback. The one `default_finder_staging_root()` picks is always
    /// among them.
    #[test]
    fn the_swept_staging_directories_are_the_ones_the_engine_stages_into() {
        let bases = FinderStagingBases::current();
        let candidates = finder_staging_candidates();
        #[cfg(target_os = "macos")]
        let expected = vec![
            bases.durable_root().expect("the test sandbox has a data dir"),
            bases.cache_root(),
            bases.temp_root(),
        ];
        #[cfg(not(target_os = "macos"))]
        let expected = vec![bases.cache_root(), bases.temp_root()];
        assert_eq!(candidates, expected);
        assert!(
            candidates.contains(&default_finder_staging_root().expect("the sandbox root is writable")),
            "the engine stages somewhere the sweep covers"
        );
    }

    /// Spec §8.4, the staging root (macOS): `<data dir>/beebeeb/finder-writes`, never under the cache dir or the
    /// temp dir, the system may purge either. The folder is private: mode 0700, and excluded from backups on
    /// macOS (it holds plaintext). Without a data dir, or with a root that cannot be created or written to, it is
    /// the typed refusal, and nothing is created in the cache or temp dir instead.
    #[test]
    fn the_staging_root_is_under_the_data_dir_never_the_cache_or_temp_dir() {
        let base = tempfile::tempdir().unwrap();
        let (data, cache, temp) = (
            base.path().join("data"),
            base.path().join("cache"),
            base.path().join("temp"),
        );
        let bases = FinderStagingBases {
            data: Some(data.clone()),
            cache: Some(cache.clone()),
            temp: temp.clone(),
        };
        let root = durable_finder_staging_root(&bases).expect("a writable data dir takes the copy");
        assert_eq!(root, data.join("beebeeb").join("finder-writes"), "under the data dir");
        assert!(
            !root.starts_with(&cache) && !root.starts_with(&temp),
            "never under the cache or temp dir: {root:?}"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&root).unwrap().permissions().mode() & 0o777,
                0o700,
                "owner-only: it holds plaintext"
            );
        }
        #[cfg(target_os = "macos")]
        {
            use std::os::unix::ffi::OsStrExt;
            let attr = std::ffi::CString::new("com.apple.metadata:com_apple_backup_excludeItem").unwrap();
            let path = std::ffi::CString::new(root.as_os_str().as_bytes()).unwrap();
            let mut buf = vec![0u8; 64];
            // SAFETY: both strings are NUL-terminated and live for the call; `buf` is ours, with its length.
            let n = unsafe {
                libc::getxattr(
                    path.as_ptr(),
                    attr.as_ptr(),
                    buf.as_mut_ptr() as *mut libc::c_void,
                    buf.len(),
                    0,
                    0,
                )
            };
            assert!(n > 0, "excluded from backups (getxattr returned {n})");
            assert_eq!(&buf[..n as usize], b"com.apple.backupd");
        }
        assert_eq!(
            std::fs::read_dir(&root).unwrap().count(),
            0,
            "the write check leaves nothing behind"
        );

        // No data dir: refused, with no fallback.
        let no_data = FinderStagingBases {
            data: None,
            ..bases.clone()
        };
        assert_eq!(
            durable_finder_staging_root(&no_data),
            Err(FinderStagingUnavailable(None))
        );

        // A root that cannot be created: a file where its parent folder would be.
        let blocked = base.path().join("blocked");
        std::fs::create_dir(&blocked).unwrap();
        std::fs::write(blocked.join("beebeeb"), b"in the way").unwrap();
        let refused = durable_finder_staging_root(&FinderStagingBases {
            data: Some(blocked),
            ..bases.clone()
        })
        .expect_err("a root that cannot be created is refused");
        assert!(refused.0.is_some(), "the refusal names the error kind: {refused:?}");
        assert!(
            refused.to_string().starts_with("the staging folder is unavailable") && !refused.to_string().contains('/'),
            "a fixed reason, no path: {refused}"
        );

        assert!(
            !cache.exists() && !temp.exists(),
            "nothing was created in the cache or temp dir"
        );
    }

    #[test]
    fn unit_tests_stage_finder_writes_in_a_sandbox_never_the_real_cache() {
        // `default_finder_staging_root()` is where Finder create/modify/keep-mine copy the plaintext
        // they are about to upload. The INSTALLED app uses the real `<cache dir>/beebeeb/finder-writes`
        // on this machine, so a test build must resolve to a per-process sandbox next to the test
        // binary: otherwise every test that queues a Finder write drops plaintext files there.
        // Only paths are computed (and the sandbox dir is created); the real dir is not touched.
        let root = default_finder_staging_root().expect("the sandbox root is writable");
        // The specific real directories, not the whole cache or data dir: a `CARGO_TARGET_DIR` under
        // `~/Library/Caches` (or `~/.cache`) puts the sandbox inside the cache dir legitimately.
        let real_staging = dirs::cache_dir()
            .expect("a user cache dir exists on a dev machine or CI runner")
            .join("beebeeb/finder-writes");
        assert!(
            !root.starts_with(&real_staging),
            "{root:?} is inside the real staging dir {real_staging:?}"
        );
        if let Some(data) = dirs::data_dir() {
            let real_durable = data.join("beebeeb/finder-writes");
            assert!(
                !root.starts_with(&real_durable),
                "{root:?} is inside the real staging dir {real_durable:?}"
            );
        }
        let exe_dir = std::env::current_exe()
            .expect("test binary path")
            .parent()
            .expect("exe dir")
            .to_path_buf();
        assert!(
            root.starts_with(&exe_dir),
            "{root:?} is not inside the test binary's directory {exe_dir:?}"
        );
        assert!(
            root.ends_with("beebeeb/finder-writes"),
            "the tail of the staging path is unchanged: {root:?}"
        );
    }

    #[test]
    fn test_create_file_queues_upload_version_and_preserves_payload() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("report.txt");
        std::fs::write(&source, b"queued payload").unwrap();
        let bridge = test_bridge(&dir.path().join("state.db"));

        let outcome = bridge
            .queue_finder_create(FinderWriteTarget {
                file_id: None,
                parent_id: None,
                filename: "report.txt".into(),
                rel_path: None,
                kind: FinderWriteItemKind::File,
                contents_path: Some(source.to_string_lossy().into_owned()),
                content_type: Some("text/plain".into()),
                base_version_identifier: None,
            })
            .unwrap();

        let FinderWriteOutcome::Queued {
            file_id: Some(file_id),
            kind,
            ..
        } = outcome
        else {
            panic!("expected queued file upload");
        };
        assert_eq!(kind, OperationKind::UploadVersion);

        let queued = bridge.db.list_due_operations(now_secs()).unwrap();
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].kind, OperationKind::UploadVersion);
        assert_eq!(queued[0].file_id.as_deref(), Some(file_id.as_str()));
        let payload_path = queued[0].payload_path.as_deref().unwrap();
        assert_eq!(std::fs::read(payload_path).unwrap(), b"queued payload");

        let metadata: serde_json::Value = serde_json::from_str(queued[0].metadata_json.as_deref().unwrap()).unwrap();
        assert_eq!(metadata["operation"], "create_file");
        assert!(metadata["name_encrypted"].as_str().unwrap().starts_with('{'));
        assert_eq!(
            bridge.db.get_file(&file_id).unwrap().unwrap().status,
            FileStatus::Uploading
        );
    }

    #[test]
    fn test_metadata_modify_does_not_queue_content_version() {
        let dir = tempfile::tempdir().unwrap();
        let bridge = test_bridge(&dir.path().join("state.db"));

        let outcome = bridge
            .queue_finder_modify(FinderWriteTarget {
                file_id: Some("40000000-0000-4000-8000-0000000000f6".into()),
                parent_id: Some("40000000-0000-4000-8000-0000000000f7".into()),
                filename: "renamed.txt".into(),
                rel_path: None,
                kind: FinderWriteItemKind::File,
                contents_path: None,
                content_type: Some("text/plain".into()),
                base_version_identifier: Some("4:1700000000:12".into()),
            })
            .unwrap();
        let FinderWriteOutcome::Queued { kind, .. } = outcome else {
            panic!("expected queued metadata op");
        };
        assert_eq!(kind, OperationKind::MoveFile);

        let queued = bridge.db.list_due_operations(now_secs()).unwrap();
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].kind, OperationKind::MoveFile);
        assert!(queued[0].payload_path.is_none());
        let metadata: serde_json::Value = serde_json::from_str(queued[0].metadata_json.as_deref().unwrap()).unwrap();
        assert_eq!(metadata["operation"], "metadata_update");
        assert!(metadata["name_encrypted"].as_str().unwrap().starts_with('{'));
    }

    #[test]
    fn test_delete_maps_to_trash_operation() {
        let dir = tempfile::tempdir().unwrap();
        let bridge = test_bridge(&dir.path().join("state.db"));
        // Task 1698: an UNKNOWN item is now an idempotent Ignored (ruling:
        // "unknown items report success") — a delete needs a real row.
        seed_bridge_entry(&bridge, "891ade83-2178-5932-91bd-4bea46844faa", "file-1.txt", None, FileStatus::Local, false, 10);

        let outcome = bridge
            .queue_finder_delete("891ade83-2178-5932-91bd-4bea46844faa", Some("9:1700000000:100".into()))
            .unwrap();
        let FinderWriteOutcome::Queued { kind, .. } = outcome else {
            panic!("expected queued trash op");
        };
        assert_eq!(kind, OperationKind::TrashFile);

        let queued = bridge.db.list_due_operations(now_secs()).unwrap();
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].kind, OperationKind::TrashFile);
        assert_eq!(queued[0].base_version, Some(9));
        assert!(queued[0].payload_path.is_none());
    }

    #[test]
    fn record_moved_to_trash_activity_writes_deletion_event() {
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();

        record_moved_to_trash_activity(&db, "6d2d20bc-321b-533d-931c-778b0b34a9fc", "docs/doomed.txt", 200).unwrap();

        let events = db.list_recent_local_activity(5).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, LocalActivityKind::MovedToTrash);
        assert_eq!(events[0].file_id.as_deref(), Some("6d2d20bc-321b-533d-931c-778b0b34a9fc"));
        assert_eq!(events[0].file_name, "doomed.txt");
        assert_eq!(events[0].rel_path.as_deref(), Some("docs/doomed.txt"));
        assert_eq!(events[0].occurred_at, 200);
    }

    #[tokio::test]
    async fn test_process_due_operations_trash_calls_api_and_keeps_row_trashing() {
        // task 0802 (server-authoritative deletion): a locally-deleted file is
        // parked in `Trashing` and its `TrashFile` op enqueued. Processing that op
        // must (a) issue the server trash DELETE and (b) on success DROP THE OP but
        // KEEP the row in `Trashing`. The `Trashing` status — not the op — is now
        // the durable hidden-locally marker; convergence (row removal) happens
        // authoritatively once the trash propagates out of `/sync/snapshot`
        // (`prune_absent`) or the `file_trash` op echo arrives (`apply_sync_op`).
        // Deleting the row HERE is what caused the "deleted file comes back" race
        // (a same-tick lagged snapshot would re-insert it as CloudOnly).
        let dir = tempfile::tempdir().unwrap();
        let server = SyncMockServer::start(vec![("200 OK".into(), serde_json::json!({ "ok": true }))]);
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), [9u8; 32]);

        // Row in the post-local-delete `Trashing` state + a queued trash op.
        bridge
            .db
            .upsert_file(&FileEntry {
                file_id: "6d2d20bc-321b-533d-931c-778b0b34a9fc".into(),
                path: "doomed.txt".into(),
                status: FileStatus::Trashing,
                size_bytes: 5,
                modified_at: 0,
                content_hash: None,
                remote_updated_at: 0,
                parent_id: None,
                item_kind: ItemKind::File,
            })
            .unwrap();
        bridge
            .db
            .enqueue_operation(&PendingOperation {
                op_id: "op-trash-1".into(),
                kind: OperationKind::TrashFile,
                file_id: Some("6d2d20bc-321b-533d-931c-778b0b34a9fc".into()),
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
            })
            .unwrap();

        let outcome = bridge.process_due_operations(dir.path(), 200).await.unwrap();
        assert_eq!(outcome.completed_op_ids, vec!["op-trash-1".to_string()]);
        assert!(outcome.retried_op_ids.is_empty());

        // The op is gone (success removes it) but the row REMAINS in `Trashing` —
        // the durable marker. The seeder only re-mints `CloudOnly`, so a `Trashing`
        // row never re-creates the placeholder; the row itself is removed later by
        // prune/op-echo once the server trash propagates.
        assert!(bridge.db.list_due_operations(999).unwrap().is_empty());
        let row = bridge.db.get_file("6d2d20bc-321b-533d-931c-778b0b34a9fc").unwrap();
        assert!(row.is_some(), "trash op must KEEP the row on success (not delete it)");
        assert_eq!(
            row.unwrap().status,
            FileStatus::Trashing,
            "row stays Trashing after a successful trash — the durable hidden-locally marker"
        );
        let events = bridge.db.list_recent_local_activity(5).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, crate::state_db::LocalActivityKind::MovedToTrash);
        assert_eq!(events[0].file_id.as_deref(), Some("6d2d20bc-321b-533d-931c-778b0b34a9fc"));
        assert_eq!(events[0].file_name, "doomed.txt");
        assert_eq!(events[0].rel_path.as_deref(), Some("doomed.txt"));
        assert_eq!(events[0].occurred_at, 200);

        let requests = server.finish();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method, "DELETE");
        assert_eq!(requests[0].path, "/api/v1/files/server-file-1");
    }

