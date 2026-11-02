//! `metadata` engine tests, split out of `engine_bridge.rs`.

use super::*;
    #[test]
    fn test_metadata_file_row_composes_nested_path_under_parent() {
        // NESTED enumeration: a child discovered under a folder must be stored
        // at `<parent_rel_path>/<decrypted_leaf>` (slash-joined, no leading
        // slash) so the path round-trips with the upload watcher's
        // `relative_db_path` and the windows_cf placeholder seeder.
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();

        let master_key = [3u8; 32];
        let file_id = "aaaaaaaa-0000-4000-8000-000000000003";
        let mk = beebeeb_core::kdf::MasterKey::from_bytes(master_key);
        let name_encrypted =
            beebeeb_core::encrypt::encrypt_name(&mk, file_id, "notes.txt", Some("text/plain")).unwrap();

        let metadata = serde_json::json!({
            "id": file_id,
            "name_encrypted": name_encrypted,
            "size_bytes": 12,
            "updated_at": 42,
            "is_folder": false,
            "parent_id": "folder-docs",
        });

        // Parent folder resolved to "docs" earlier in the BFS walk.
        let entry = apply_metadata_file_row(
            &db,
            &metadata,
            Namespace::MyFiles,
            None,
            None,
            PERMISSION_READ | PERMISSION_WRITE | PERMISSION_OWNER,
            100,
            &master_key,
            "docs",
        )
        .unwrap()
        .unwrap();

        assert_eq!(entry.path, "docs/notes.txt");
        assert_eq!(entry.item_kind, ItemKind::File);
        assert_eq!(entry.parent_id.as_deref(), Some("folder-docs"));
        // The stored row (what list_by_status / the seeder reads) agrees, and
        // the contract carries the authoritative parent_id + item_kind.
        let stored = db.get_file(file_id).unwrap().unwrap();
        assert_eq!(stored.path, "docs/notes.txt");
        assert_eq!(stored.item_kind, ItemKind::File);
        assert_eq!(stored.parent_id.as_deref(), Some("folder-docs"));
        let contract = db.get_file_contract_state(file_id).unwrap().unwrap();
        assert_eq!(contract.parent_id.as_deref(), Some("folder-docs"));
        assert_eq!(contract.item_kind, ItemKind::File);
    }

    #[test]
    fn test_metadata_file_row_classifies_folder_rows() {
        // A row the server marks as a folder must surface item_kind == Folder
        // both on the returned FileEntry and the stored row, so the BFS walk
        // descends into it and the placeholder seeder mints a DIRECTORY.
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();

        let master_key = [5u8; 32];
        let file_id = "bbbbbbbb-0000-4000-8000-000000000004";
        let mk = beebeeb_core::kdf::MasterKey::from_bytes(master_key);
        let name_encrypted = beebeeb_core::encrypt::encrypt_name(&mk, file_id, "Photos", None).unwrap();

        let metadata = serde_json::json!({
            "id": file_id,
            "name_encrypted": name_encrypted,
            "size_bytes": 0,
            "updated_at": 7,
            "is_folder": true,
        });

        let entry = apply_metadata_file_row(
            &db,
            &metadata,
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

        assert_eq!(entry.path, "Photos");
        assert_eq!(entry.item_kind, ItemKind::Folder);
        assert!(entry.is_dir());
        let stored = db.get_file(file_id).unwrap().unwrap();
        assert_eq!(stored.item_kind, ItemKind::Folder);
        assert!(stored.is_dir());
    }

    #[tokio::test]
    async fn test_process_due_operations_completes_pin_marker_and_invalidates_file() {
        let dir = tempfile::tempdir().unwrap();
        let bridge = test_bridge(&dir.path().join("state.db"));
        bridge
            .db
            .enqueue_operation(&PendingOperation {
                op_id: "op-pin".into(),
                kind: OperationKind::PinTree,
                file_id: Some("folder-1".into()),
                parent_id: None,
                target_path: None,
                metadata_json: Some(r#"{"operation":"pin_tree","pinned":true}"#.into()),
                payload_path: None,
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
        assert_eq!(outcome.completed_op_ids, vec!["op-pin".to_string()]);
        assert_eq!(outcome.invalidated_item_ids, vec!["folder-1".to_string()]);
        assert!(bridge.db.list_due_operations(999).unwrap().is_empty());
    }

    /// Task 1538 Codex P1 (PR #49, lib.rs:1087 thread): once the bridge's
    /// stop flag is set, `process_due_operations` must not execute ANY due
    /// operation — not "finish the current batch, then stop next tick".
    /// Uses `PinTree` (the cheapest op kind: `execute_operation` returns
    /// `Ok(())` with no network call) so a failure here can only be the
    /// missing stop-check, never a flaky mock server.
    #[tokio::test]
    async fn test_process_due_operations_stops_immediately_once_engine_is_stopping() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("state.db");
        let db = Arc::new(StateDb::open(&db_path).unwrap());
        let api = Arc::new(ApiClient::new(
            "https://api.beebeeb.io".into(),
            "token".into(),
            [7u8; 32],
        ));
        let stopping = Arc::new(AtomicBool::new(false));
        let bridge = EngineBridge::new_with_stop_flag(db.clone(), api, stopping.clone());

        db.enqueue_operation(&PendingOperation {
            op_id: "op-pin-1".into(),
            kind: OperationKind::PinTree,
            file_id: Some("folder-1".into()),
            parent_id: None,
            target_path: None,
            metadata_json: Some(r#"{"operation":"pin_tree","pinned":true}"#.into()),
            payload_path: None,
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
        db.enqueue_operation(&PendingOperation {
            op_id: "op-pin-2".into(),
            kind: OperationKind::PinTree,
            file_id: Some("folder-2".into()),
            parent_id: None,
            target_path: None,
            metadata_json: Some(r#"{"operation":"pin_tree","pinned":true}"#.into()),
            payload_path: None,
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

        // Ask the engine to stop BEFORE draining the queue — simulates
        // `EngineRunner::abort` flipping the flag while a tick is already
        // about to process a batch of due operations.
        stopping.store(true, Ordering::SeqCst);

        let outcome = bridge.process_due_operations(dir.path(), 200).await.unwrap();

        assert!(
            outcome.completed_op_ids.is_empty(),
            "no operation may run once the engine has been asked to stop"
        );
        assert_eq!(
            db.list_due_operations(999).unwrap().len(),
            2,
            "both queued ops must still be in the queue, untouched, for the caller's purge to clear"
        );
    }

    /// Task 1538 Codex P1 — the watcher (Windows upload watcher AND the
    /// macOS/Linux File Provider IPC handler both call these directly) must
    /// not be able to slip a fresh write into the queue once the engine has
    /// been asked to stop, or sign-out's purge could run BEFORE the write
    /// lands and then miss it entirely.
    #[test]
    fn test_queue_finder_writes_refuse_to_enqueue_once_engine_is_stopping() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("state.db");
        let db = Arc::new(StateDb::open(&db_path).unwrap());
        let api = Arc::new(ApiClient::new(
            "https://api.beebeeb.io".into(),
            "token".into(),
            [7u8; 32],
        ));
        let stopping = Arc::new(AtomicBool::new(true));
        let bridge = EngineBridge::new_with_stop_flag(db.clone(), api, stopping);

        let create_result = bridge.queue_finder_create(FinderWriteTarget {
            file_id: None,
            parent_id: None,
            filename: "new-file.txt".into(),
            rel_path: None,
            kind: FinderWriteItemKind::File,
            contents_path: None,
            content_type: None,
            base_version_identifier: None,
        });
        assert!(create_result.is_err(), "queue_finder_create must refuse while stopping");

        let modify_result = bridge.queue_finder_modify(FinderWriteTarget {
            file_id: Some("file-1".into()),
            parent_id: None,
            filename: "renamed.txt".into(),
            rel_path: None,
            kind: FinderWriteItemKind::File,
            contents_path: None,
            content_type: None,
            base_version_identifier: None,
        });
        assert!(modify_result.is_err(), "queue_finder_modify must refuse while stopping");

        let delete_result = bridge.queue_finder_delete("file-1", None);
        assert!(delete_result.is_err(), "queue_finder_delete must refuse while stopping");

        assert!(
            db.list_due_operations(i64::MAX).unwrap().is_empty(),
            "not a single one of the refused writes may have reached the operation_queue"
        );
    }

    /// U45: every change the Finder IPC queues meets the sign-out's fence in the transaction that queues it. Once
    /// accepts are closed, a rename, a move to the top level, a folder create and a delete are each refused with
    /// `AcceptsClosed` (the extension hears `WriteRetryLater`): nothing is queued, and no row is written or changed.
    /// Reopened, the same rename queues.
    #[test]
    fn every_finder_change_after_the_close_queues_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let bridge = test_bridge(&dir.path().join("state.db"));
        seed_bridge_row(&bridge, "folder-a", "A", None, FileStatus::Local, 0);
        seed_bridge_row(&bridge, "in-a", "A/x.txt", Some("folder-a"), FileStatus::Local, 0);
        let target = |file_id: Option<&str>, parent_id: Option<&str>, filename: &str, kind: FinderWriteItemKind| {
            FinderWriteTarget {
                file_id: file_id.map(str::to_string),
                parent_id: parent_id.map(str::to_string),
                filename: filename.into(),
                rel_path: None,
                kind,
                contents_path: None,
                content_type: None,
                base_version_identifier: None,
            }
        };
        bridge.db.close_finder_accepts().unwrap();
        let changes: [(&str, anyhow::Result<FinderWriteOutcome>); 4] = [
            (
                "a rename",
                bridge.queue_finder_modify(target(
                    Some("in-a"),
                    Some("folder-a"),
                    "renamed.txt",
                    FinderWriteItemKind::File,
                )),
            ),
            (
                "a move to the top level",
                bridge.queue_finder_modify(target(Some("in-a"), None, "x.txt", FinderWriteItemKind::File)),
            ),
            (
                "a folder create",
                bridge.queue_finder_create(target(
                    None,
                    Some("folder-a"),
                    "New folder",
                    FinderWriteItemKind::Folder,
                )),
            ),
            ("a delete", bridge.queue_finder_delete("in-a", None)),
        ];
        for (what, result) in changes {
            let error = result.expect_err(what);
            assert!(error.is::<AcceptsClosed>(), "{what}: {error:#}");
        }
        // Every row, due or not: a paused, parked or backed-off op is still a queued change.
        let queued: i64 = bridge
            .db
            .hold_lock_for_test()
            .query_row("SELECT COUNT(*) FROM operation_queue", [], |row| row.get(0))
            .unwrap();
        assert_eq!(queued, 0, "nothing queued");
        assert!(
            bridge.db.get_file_by_path("A/New folder").unwrap().is_none(),
            "the refused folder create left no row"
        );
        let row = bridge.db.get_file("in-a").unwrap().unwrap();
        assert_eq!(
            (row.path.as_str(), row.status),
            ("A/x.txt", FileStatus::Local),
            "the refused delete left the row as it was"
        );
        bridge.db.reopen_finder_accepts().unwrap();
        bridge
            .queue_finder_modify(target(
                Some("in-a"),
                Some("folder-a"),
                "renamed.txt",
                FinderWriteItemKind::File,
            ))
            .expect("reopened, the rename queues: the refusals above were the fence");
        assert_eq!(bridge.db.list_due_operations(i64::MAX).unwrap().len(), 1);
        // Reopened, the folder create writes its row in the fenced transaction, as a folder under its parent, so a
        // nested child resolves it as its parent.
        bridge
            .queue_finder_create(target(
                None,
                Some("folder-a"),
                "New folder",
                FinderWriteItemKind::Folder,
            ))
            .expect("reopened, the folder create queues");
        let folder = bridge
            .db
            .get_file_by_path("A/New folder")
            .unwrap()
            .expect("the folder create's row");
        assert_eq!(
            (folder.item_kind, folder.parent_id.as_deref()),
            (ItemKind::Folder, Some("folder-a")),
            "the folder create's row is a folder, under its parent"
        );
        assert_eq!(bridge.db.list_due_operations(i64::MAX).unwrap().len(), 2);
    }

    /// U45: a person's save meets the fence too. Once accepts are closed, a File Provider create of a new file and a
    /// content save of an existing one are each refused with `AcceptsClosed`, and the IPC answers `WriteRetryLater`,
    /// which the extension reports as a transient error, so macOS keeps the change and hands it over again (an `Error`
    /// would reach it as the definitive `cannotSynchronize`). Nothing is queued, and the refused create leaves no row.
    #[test]
    fn a_file_provider_save_after_the_close_is_retried_later() {
        let dir = tempfile::tempdir().unwrap();
        let bridge = test_bridge(&dir.path().join("state.db"));
        seed_bridge_row(&bridge, "in-a", "x.txt", None, FileStatus::Local, 0);
        let contents = dir.path().join("new.txt");
        std::fs::write(&contents, b"new bytes").unwrap();
        bridge.db.close_finder_accepts().unwrap();
        let saves: [(&str, &'static str, anyhow::Result<FpWrite>); 2] = [
            (
                "a new file",
                "create",
                bridge.queue_file_provider_create(finder_file_target(None, "new.txt", &contents, None)),
            ),
            (
                "a save",
                "modify",
                bridge.queue_file_provider_modify(finder_file_target(Some("in-a"), "x.txt", &contents, None)),
            ),
        ];
        for (what, op, result) in saves {
            let error = result.err().unwrap_or_else(|| panic!("{what} ({op}): queued"));
            assert!(error.is::<AcceptsClosed>(), "{what} ({op}): {error:#}");
            #[cfg(unix)]
            {
                let response = crate::ipc_socket::write_outcome_response(op, &bridge.db, Err(error));
                assert!(
                    matches!(response, crate::ipc_socket::IpcResponse::WriteRetryLater { .. }),
                    "{what} ({op}): {response:?}"
                );
            }
        }
        // Every row, due or not.
        let queued: i64 = bridge
            .db
            .hold_lock_for_test()
            .query_row("SELECT COUNT(*) FROM operation_queue", [], |row| row.get(0))
            .unwrap();
        assert_eq!(queued, 0, "nothing queued");
        let rows: Vec<String> = bridge
            .db
            .list_files()
            .unwrap()
            .into_iter()
            .map(|row| row.file_id)
            .collect();
        assert_eq!(rows, ["in-a"], "the refused new file left no row");
    }

