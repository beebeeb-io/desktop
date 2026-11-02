//! `trash_ops` engine tests, split out of `engine_bridge.rs`.

use super::*;
    #[tokio::test]
    async fn test_process_due_operations_trash_retries_and_keeps_row_on_server_error() {
        // If the server trash fails, the op must be retried (not dropped) and the
        // row must be PRESERVED — recoverable rather than lost. (The row is left
        // in `Trashing`, so the placeholder still does not reappear meanwhile.)
        let dir = tempfile::tempdir().unwrap();
        let server = SyncMockServer::start(vec![(
            "500 Internal Server Error".into(),
            serde_json::json!({ "error": "boom" }),
        )]);
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), [9u8; 32]);

        bridge
            .db
            .upsert_file(&FileEntry {
                file_id: "server-file-2".into(),
                path: "doomed2.txt".into(),
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
                op_id: "op-trash-2".into(),
                kind: OperationKind::TrashFile,
                file_id: Some("server-file-2".into()),
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
        assert!(outcome.completed_op_ids.is_empty());
        assert_eq!(outcome.retried_op_ids, vec!["op-trash-2".to_string()]);
        // Row preserved (still Trashing) so a future retry can complete and the
        // file is recoverable from the server in the meantime.
        let row = bridge.db.get_file("server-file-2").unwrap().unwrap();
        assert_eq!(row.status, FileStatus::Trashing);

        let _ = server.finish();
    }

    /// Spec D7 (tasks 1888, 2226): a folder made here inside another new folder waits for that folder's create, and a
    /// pass reads its due work once. A pass that lands a folder create lists again, so a new tree two levels deep goes
    /// up in one pass, not one tick (`TICK_INTERVAL`, 30 s) per level.
    #[cfg(not(target_os = "macos"))]
    #[tokio::test]
    async fn a_new_folder_and_the_folder_inside_it_are_created_in_one_pass() {
        let dir = tempfile::tempdir().unwrap();
        let server = SyncMockServer::start(vec![
            ("200 OK".into(), serde_json::json!({ "id": "folder-new" })),
            ("200 OK".into(), serde_json::json!({ "id": "folder-sub" })),
        ]);
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), [9u8; 32]);
        for (op_id, file_id, parent_id, created_at) in
            [("op-new", "folder-new", None, 100), ("op-sub", "folder-sub", Some("folder-new"), 101)]
        {
            bridge
                .db
                .enqueue_operation(&PendingOperation {
                    op_id: op_id.into(),
                    kind: OperationKind::CreateFolder,
                    file_id: Some(file_id.into()),
                    parent_id: parent_id.map(str::to_string),
                    target_path: None,
                    metadata_json: Some(serde_json::json!({ "name_encrypted": "sealed" }).to_string()),
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
        assert_eq!(bridge.db.list_due_operations(200).unwrap().len(), 1, "the inner folder waits for its folder");

        let outcome = bridge.process_due_operations(dir.path(), 200).await.unwrap();
        assert_eq!(outcome.completed_op_ids, vec!["op-new".to_string(), "op-sub".to_string()], "both in one pass");
        assert!(outcome.retried_op_ids.is_empty());
        let requests = server.finish();
        assert_eq!(requests.len(), 2);
        let inner: serde_json::Value = serde_json::from_slice(&requests[1].body).unwrap();
        assert_eq!(inner["folder_id"], "folder-sub");
        assert_eq!(inner["parent_id"], "folder-new", "sent after its folder landed, under it");
    }

    #[test]
    fn test_recursive_pin_queues_hydration_for_cloud_only_descendants() {
        let dir = tempfile::tempdir().unwrap();
        let bridge = test_bridge(&dir.path().join("state.db"));
        seed_bridge_row(&bridge, "folder-a", "/Projects", None, FileStatus::CloudOnly, 0);
        seed_bridge_row(
            &bridge,
            "file-a",
            "/Projects/report.txt",
            Some("folder-a"),
            FileStatus::CloudOnly,
            128,
        );
        seed_bridge_row(
            &bridge,
            "file-local",
            "/Projects/local.txt",
            Some("folder-a"),
            FileStatus::Local,
            256,
        );

        // `sync_root` is only used on Windows (to resolve placeholder paths for
        // CfSetPinState). On the non-Windows test host the hydrate-enqueue path
        // runs and `sync_root` is unused, so any path is fine here.
        let queued = bridge.set_recursive_pin(dir.path(), "folder-a", true).unwrap();
        assert_eq!(queued.changed_item_ids.len(), 3);
        // hydrate_operations is the non-Windows materialise path; on Windows the
        // OS (CfSetPinState) hydrates pinned files, so this count is 0 there.
        #[cfg(not(target_os = "windows"))]
        assert_eq!(queued.hydrate_operations, 1);

        let due = bridge.db.list_due_operations(now_secs()).unwrap();
        assert!(due.iter().any(|op| {
            op.kind == OperationKind::PinTree
                && op.file_id.as_deref() == Some("folder-a")
                && op.metadata_json.as_deref().unwrap_or("").contains("\"pinned\":true")
        }));
        // On Windows the pin path calls CfSetPinState; the OS drives hydration
        // via FETCH_DATA rather than us enqueuing HydrateFile ops, so no
        // HydrateFile entries exist in the queue on that platform.
        #[cfg(not(target_os = "windows"))]
        {
            assert!(
                due.iter()
                    .any(|op| { op.kind == OperationKind::HydrateFile && op.file_id.as_deref() == Some("file-a") })
            );
            assert!(
                !due.iter()
                    .any(|op| { op.kind == OperationKind::HydrateFile && op.file_id.as_deref() == Some("file-local") })
            );
        }
    }

    #[test]
    fn test_smart_cache_cleanup_never_evicts_effectively_pinned_files() {
        let dir = tempfile::tempdir().unwrap();
        let bridge = test_bridge(&dir.path().join("state.db"));
        seed_bridge_row(&bridge, "pinned", "/Pinned.txt", None, FileStatus::Local, 700);
        seed_bridge_row(&bridge, "unpinned", "/Unpinned.txt", None, FileStatus::Local, 600);
        bridge.db.set_recursive_pin("pinned", true, 1).unwrap();
        bridge.db.mark_cached("pinned", "/cache/pinned", 700, 10).unwrap();
        bridge.db.mark_cached("unpinned", "/cache/unpinned", 600, 20).unwrap();

        let evicted = bridge
            .enforce_smart_cache(CachePolicy {
                max_unpinned_cache_bytes: 500,
                disk_pressure_min_free_bytes: 0,
            })
            .unwrap();

        assert_eq!(evicted.evicted_file_ids, vec!["unpinned".to_string()]);
        assert_eq!(bridge.db.get_file("pinned").unwrap().unwrap().status, FileStatus::Local);
    }

    #[test]
    fn test_local_cache_limit_counts_pinned_bytes_against_total_cap() {
        let dir = tempfile::tempdir().unwrap();
        let bridge = test_bridge(&dir.path().join("state.db"));
        seed_bridge_row(&bridge, "pinned", "/Pinned.txt", None, FileStatus::Local, 700);
        seed_bridge_row(&bridge, "old", "/Old.txt", None, FileStatus::Local, 400);
        seed_bridge_row(&bridge, "new", "/New.txt", None, FileStatus::Local, 300);

        bridge.db.set_recursive_pin("pinned", true, 1).unwrap();
        bridge.db.mark_cached("pinned", "/cache/pinned", 700, 10).unwrap();
        bridge.db.mark_cached("old", "/cache/old", 400, 20).unwrap();
        bridge.db.mark_cached("new", "/cache/new", 300, 30).unwrap();

        let evicted = bridge.enforce_local_cache_limit(Some(1_000)).unwrap();

        assert_eq!(evicted.evicted_file_ids, vec!["old".to_string()]);
        assert_eq!(bridge.db.get_file("pinned").unwrap().unwrap().status, FileStatus::Local);
        assert_eq!(bridge.db.get_file("new").unwrap().unwrap().status, FileStatus::Local);
        assert_eq!(
            bridge.db.get_file("old").unwrap().unwrap().status,
            FileStatus::CloudOnly
        );
    }

    /// Desktop 0.8.15 (deliverable 1): cache eviction is freeing work, so it waits for the
    /// start-up guard's pass, which now runs beside the engine instead of before it; and every
    /// caller goes through the process's one guard state.
    #[test]
    fn desktop_0815_cache_eviction_waits_for_the_start_up_pass() {
        let dir = tempfile::tempdir().unwrap();
        let bridge = test_bridge(&dir.path().join("state.db"));
        seed_bridge_row(&bridge, "old", "/Old.txt", None, FileStatus::Local, 400);
        seed_bridge_row(&bridge, "new", "/New.txt", None, FileStatus::Local, 300);
        bridge.db.mark_cached("old", "/cache/old", 400, 20).unwrap();
        bridge.db.mark_cached("new", "/cache/new", 300, 30).unwrap();
        let guard = crate::unknown_check::StartupGuard::new();
        let pass = guard.begin();
        let held = bridge.enforce_local_cache_limit_after_guard(Some(500), &guard).unwrap();
        assert!(held.evicted_file_ids.is_empty(), "nothing is evicted while the pass is owed");
        assert_eq!(bridge.db.get_file("old").unwrap().unwrap().status, FileStatus::Local);
        guard.finish(pass);
        let evicted = bridge.enforce_local_cache_limit_after_guard(Some(500), &guard).unwrap();
        assert_eq!(evicted.evicted_file_ids, vec!["old".to_string()], "evicted once the pass walked every row");
        // The checkout may carry CRLF line endings (the Windows gate clones with core.autocrlf=true).
        let source = include_str!("../../engine_bridge.rs").replace("\r\n", "\n");
        let production = source.split("\n#[cfg(test)]\nmod tests {").next().unwrap();
        let body = &production[production.find("pub fn enforce_local_cache_limit(").unwrap()..];
        let body = &body[..body.find("\n    }\n").unwrap()];
        assert!(
            crate::source_pin::squeeze(body).contains(&crate::source_pin::squeeze(
                "self.enforce_local_cache_limit_after_guard(max_local_cache_bytes, &crate::unknown_check::STARTUP_GUARD)"
            )),
            "{body}"
        );
    }

    #[test]
    fn test_local_cache_usage_bytes_sums_pinned_and_unpinned_cache() {
        let dir = tempfile::tempdir().unwrap();
        let bridge = test_bridge(&dir.path().join("state.db"));
        seed_bridge_row(&bridge, "pinned", "/Pinned.txt", None, FileStatus::Local, 700);
        seed_bridge_row(&bridge, "unpinned", "/Unpinned.txt", None, FileStatus::Local, 600);

        bridge.db.set_recursive_pin("pinned", true, 1).unwrap();
        bridge.db.mark_cached("pinned", "/cache/pinned", 700, 10).unwrap();
        bridge.db.mark_cached("unpinned", "/cache/unpinned", 600, 20).unwrap();

        assert_eq!(bridge.local_cache_usage_bytes().unwrap(), 1_300);
    }

    #[test]
    fn test_shared_invite_mapping_filters_approved_and_permissions() {
        let body = serde_json::json!({
            "invites": [
                {
                    "id": "invite-read",
                    "file_id": "root-read",
                    "status": "approved",
                    "display_name": "Client dropbox",
                    "is_folder_share": true,
                    "can_reshare": true
                },
                {
                    "id": "invite-write",
                    "file_id": "root-write",
                    "status": "approved",
                    "decrypted_name": "Editorial",
                    "is_folder": true,
                    "capability": "write"
                },
                {
                    "id": "invite-pending",
                    "file_id": "root-pending",
                    "status": "claimed"
                }
            ]
        });

        let roots = shared_roots_from_invite_response(&body);
        assert_eq!(roots.len(), 2);
        assert_eq!(roots[0].display_name, "Encrypted folder");
        assert_eq!(roots[0].permission_bits, PERMISSION_READ | PERMISSION_SHARE);
        assert_eq!(roots[1].permission_bits, PERMISSION_READ | PERMISSION_WRITE);
    }

    #[test]
    fn shared_invite_id_match_does_not_match_same_root_wrong_invite() {
        let wrong_invite_same_root = serde_json::json!({
            "id": "invite-wrong",
            "file_id": "root-folder",
            "status": "approved"
        });
        let right_invite_different_root = serde_json::json!({
            "invite_id": "invite-target",
            "file_id": "other-root",
            "status": "approved"
        });

        assert!(!shared_invite_id_matches(&wrong_invite_same_root, "invite-target"));
        assert!(shared_invite_id_matches(&right_invite_different_root, "invite-target"));
    }

