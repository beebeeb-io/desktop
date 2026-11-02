//! `shared_2` engine tests, split out of `engine_bridge.rs`.

use super::*;
    #[test]
    fn test_read_only_shared_item_rejects_finder_writes() {
        let dir = tempfile::tempdir().unwrap();
        let bridge = test_bridge(&dir.path().join("state.db"));
        seed_bridge_row(
            &bridge,
            "shared-readonly",
            "/Shared with me/Readonly.txt",
            None,
            FileStatus::Local,
            100,
        );
        let mut contract = bridge.db.get_file_contract_state("shared-readonly").unwrap().unwrap();
        contract.namespace = Namespace::SharedWithMe;
        contract.shared_root_id = Some("shared-readonly".into());
        contract.share_id = Some("invite-1".into());
        contract.permission_bits = PERMISSION_READ;
        bridge.db.set_file_contract_state(&contract).unwrap();

        let err = bridge
            .queue_finder_delete("shared-readonly", Some("1:1:100".into()))
            .unwrap_err()
            .to_string();
        assert!(err.contains("read-only shared item"));
        assert!(bridge.db.list_due_operations(now_secs()).unwrap().is_empty());
    }

    #[test]
    fn test_editable_shared_write_queues_actor_and_share_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("shared.txt");
        std::fs::write(&source, b"shared payload").unwrap();
        let bridge = test_bridge(&dir.path().join("state.db"));
        seed_bridge_row(
            &bridge,
            "shared-editable",
            "/Shared with me/Editable.txt",
            None,
            FileStatus::Local,
            100,
        );
        let mut contract = bridge.db.get_file_contract_state("shared-editable").unwrap().unwrap();
        contract.namespace = Namespace::SharedWithMe;
        contract.shared_root_id = Some("shared-root".into());
        contract.share_id = Some("invite-2".into());
        contract.permission_bits = PERMISSION_READ | PERMISSION_WRITE;
        contract.current_object_version_id = Some("object-1".into());
        bridge.db.set_file_contract_state(&contract).unwrap();

        bridge
            .queue_finder_modify(FinderWriteTarget {
                file_id: Some("shared-editable".into()),
                parent_id: None,
                filename: "Editable.txt".into(),
                rel_path: None,
                kind: FinderWriteItemKind::File,
                contents_path: Some(source.to_string_lossy().into_owned()),
                content_type: Some("text/plain".into()),
                base_version_identifier: Some("3:2:100".into()),
            })
            .unwrap();

        let queued = bridge.db.list_due_operations(now_secs()).unwrap();
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].kind, OperationKind::UploadVersion);
        assert_eq!(queued[0].base_version, Some(3));
        assert_eq!(queued[0].base_object_version_id.as_deref(), Some("object-1"));
        let metadata: serde_json::Value = serde_json::from_str(queued[0].metadata_json.as_deref().unwrap()).unwrap();
        assert_eq!(metadata["shared_root_id"], "shared-root");
        assert_eq!(metadata["share_id"], "invite-2");
        assert_eq!(metadata["uploaded_by"], "authenticated_desktop_user");
    }

    #[test]
    fn test_version_center_feed_maps_conflicts_and_review_operations() {
        let dir = tempfile::tempdir().unwrap();
        let bridge = test_bridge(&dir.path().join("state.db"));
        bridge
            .db
            .upsert_file(&FileEntry {
                file_id: "conflict-file".into(),
                path: "/Work/conflict.md".into(),
                status: FileStatus::Conflict,
                size_bytes: 64,
                modified_at: 100,
                content_hash: Some("local".into()),
                remote_updated_at: 90,
                parent_id: None,
                item_kind: ItemKind::File,
            })
            .unwrap();
        bridge
            .db
            .enqueue_operation(&PendingOperation {
                op_id: "op-quota".into(),
                kind: OperationKind::UploadVersion,
                file_id: Some("quota-file".into()),
                parent_id: None,
                target_path: Some("/Work/quota.png".into()),
                metadata_json: Some(r#"{"operation":"upload_version"}"#.into()),
                payload_path: Some("/tmp/quota".into()),
                base_version: None,
                base_object_version_id: None,
                attempts: 3,
                max_attempts: 3,
                next_retry_at: 100,
                last_error: Some("quota exceeded".into()),
                backup_source_key: None,
                created_at: 101,
                updated_at: 101,
            })
            .unwrap();
        bridge
            .db
            .enqueue_operation(&PendingOperation {
                op_id: "op-permission".into(),
                kind: OperationKind::MoveFile,
                file_id: Some("shared-file".into()),
                parent_id: Some("shared-folder".into()),
                target_path: Some("/Shared/blocked.txt".into()),
                metadata_json: Some(r#"{"operation":"metadata_update"}"#.into()),
                payload_path: None,
                base_version: None,
                base_object_version_id: None,
                attempts: 1,
                max_attempts: 5,
                next_retry_at: 100,
                last_error: Some("403 forbidden: permission denied".into()),
                backup_source_key: None,
                created_at: 102,
                updated_at: 102,
            })
            .unwrap();
        bridge
            .db
            .enqueue_operation(&PendingOperation {
                op_id: "op-stale".into(),
                kind: OperationKind::UploadVersion,
                file_id: Some("stale-file".into()),
                parent_id: None,
                target_path: Some("/Work/stale.txt".into()),
                metadata_json: Some(r#"{"operation":"upload_version","base_version_identifier":"7:1700:12"}"#.into()),
                payload_path: Some("/tmp/stale".into()),
                base_version: Some(7),
                base_object_version_id: None,
                attempts: 0,
                max_attempts: 25,
                next_retry_at: 100,
                last_error: Some("stale base version rejected".into()),
                backup_source_key: None,
                created_at: 103,
                updated_at: 103,
            })
            .unwrap();

        let feed = bridge.version_conflict_feed().unwrap();
        assert!(feed.iter().any(|entry| {
            entry.kind == "conflict"
                && entry.file_id == "conflict-file"
                && entry.file_name == "conflict.md"
                && entry.action == "open_conflict"
        }));
        assert!(
            feed.iter()
                .any(|entry| entry.kind == "quota_failure" && entry.file_name == "quota.png")
        );
        assert!(
            feed.iter()
                .any(|entry| entry.kind == "permission_failure" && entry.file_name == "blocked.txt")
        );
        assert!(feed.iter().any(|entry| {
            entry.kind == "stale_base"
                && entry.file_name == "stale.txt"
                && entry.base_version == Some(7)
                && entry.detail.contains("preserved")
        }));
    }

    #[test]
    fn test_restore_version_routes_to_durable_operation_for_review() {
        let dir = tempfile::tempdir().unwrap();
        let bridge = test_bridge(&dir.path().join("state.db"));

        let queued = bridge
            .queue_restore_version(
                "file-restore",
                "version-2",
                Some("direct restore failed: network offline".into()),
            )
            .unwrap();

        assert_eq!(queued.kind, OperationKind::RestoreVersion);
        assert_eq!(queued.file_id.as_deref(), Some("file-restore"));
        assert_eq!(queued.base_object_version_id.as_deref(), Some("version-2"));
        let metadata: serde_json::Value = serde_json::from_str(queued.metadata_json.as_deref().unwrap()).unwrap();
        assert_eq!(metadata["operation"], "restore_version");
        assert_eq!(metadata["version_id"], "version-2");

        let feed = bridge.version_conflict_feed().unwrap();
        assert!(feed.iter().any(|entry| {
            entry.kind == "restore"
                && entry.file_id == "file-restore"
                && entry.version_id.as_deref() == Some("version-2")
                && entry.last_error.as_deref() == Some("direct restore failed: network offline")
        }));
    }

    pub(super) fn seed_bridge_row(
        bridge: &EngineBridge,
        file_id: &str,
        path: &str,
        parent_id: Option<&str>,
        status: FileStatus,
        cache_bytes: i64,
    ) {
        bridge
            .db
            .upsert_file(&FileEntry {
                file_id: file_id.into(),
                path: path.into(),
                status,
                size_bytes: cache_bytes,
                modified_at: 0,
                content_hash: None,
                remote_updated_at: 0,
                parent_id: parent_id.map(str::to_string),
                item_kind: ItemKind::File,
            })
            .unwrap();
        let mut contract = bridge.db.get_file_contract_state(file_id).unwrap().unwrap();
        contract.parent_id = parent_id.map(str::to_string);
        contract.cache_bytes = cache_bytes;
        bridge.db.set_file_contract_state(&contract).unwrap();
    }

    pub(super) fn seed_bridge_entry(
        bridge: &EngineBridge,
        file_id: &str,
        path: &str,
        parent_id: Option<&str>,
        status: FileStatus,
        is_folder: bool,
        remote_updated_at: i64,
    ) {
        let item_kind = if is_folder { ItemKind::Folder } else { ItemKind::File };
        bridge
            .db
            .upsert_file(&FileEntry {
                file_id: file_id.into(),
                path: path.into(),
                status,
                size_bytes: if is_folder { 0 } else { 10 },
                modified_at: remote_updated_at,
                content_hash: None,
                remote_updated_at,
                parent_id: parent_id.map(str::to_string),
                item_kind: item_kind.clone(),
            })
            .unwrap();
        let mut contract = bridge.db.get_file_contract_state(file_id).unwrap().unwrap();
        contract.namespace = Namespace::MyFiles;
        contract.parent_id = parent_id.map(str::to_string);
        contract.item_kind = item_kind;
        contract.permission_bits = PERMISSION_READ | PERMISSION_WRITE | PERMISSION_OWNER;
        bridge.db.set_file_contract_state(&contract).unwrap();
    }

    pub(super) fn enqueue_test_operation(bridge: &EngineBridge, op_id: &str, kind: OperationKind, file_id: &str, created_at: i64) {
        bridge
            .db
            .enqueue_operation(&PendingOperation {
                op_id: op_id.into(),
                kind,
                file_id: Some(file_id.into()),
                parent_id: None,
                target_path: Some(format!("{file_id}.txt")),
                metadata_json: None,
                payload_path: None,
                base_version: None,
                base_object_version_id: None,
                attempts: 0,
                max_attempts: 25,
                next_retry_at: 0,
                last_error: None,
                backup_source_key: None,
                created_at,
                updated_at: created_at,
            })
            .unwrap();
    }

    // ------------------------------------------------------------------
    // Task 1697 review fix (T3): the remote ingestion path must collect the
    // item ids it APPLIED so the runner can signal the File Provider working
    // set about REMOTE changes — before this, only local ops signaled
    // (`operations_applied`), so Finder stayed blind to server-side
    // creates/modifies/moves/deletes until an unrelated signal. RED-first:
    // written against the plumbed-but-empty collectors and seen failing (an
    // applied op returned an empty id list) before the collection landed.
    // ------------------------------------------------------------------

