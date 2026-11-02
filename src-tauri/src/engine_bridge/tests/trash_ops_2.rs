//! `trash_ops_2` engine tests, split out of `engine_bridge.rs`.

use super::*;
    /// Spec §7.2 (final review): a save of a file whose folder is deleted elsewhere is re-homed
    /// at the top level with its row, and its upload sends that move (`"parent_id": null`), so
    /// the server file follows the row.
    #[tokio::test]
    async fn a_save_re_homed_by_a_sweep_sends_the_move() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [110u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_row_in_folder(&bridge, &server, "in-deleted-folder");
        fp_save_moved(
            &bridge,
            dir.path(),
            "in-deleted-folder",
            "notes.txt",
            "folder-a",
            b"saved",
            "1",
        );
        let delete = crate::api_client::SyncOp {
            seq_id: 703,
            op_type: "file_delete".into(),
            payload: serde_json::json!({ "id": "folder-a" }),
        };
        apply_sync_op(&bridge, &sync_root, &delete, now_secs(), &mut Vec::new()).unwrap();
        assert_eq!(
            name_parent_status(&bridge, "in-deleted-folder"),
            ("notes.txt".into(), None, FileStatus::Uploading)
        );
        drain_upload_queue(&bridge, &sync_root).await;
        let state = server.finish();
        assert_eq!(state.latest_plaintext("in-deleted-folder", master_key), b"saved");
        assert_eq!(
            patched_parents(&state, "in-deleted-folder"),
            vec![Some(serde_json::Value::Null)],
            "the save sent the move to the top level"
        );
    }

    /// Spec §7.2 (final review): a file deleted for good elsewhere while a save of it is queued.
    /// The edit wins: the row stays, visible, and the save re-creates the file under its id. The
    /// person can delete it again.
    #[cfg(unix)] // names `crate::ipc_socket::list_file_provider_items`
    #[tokio::test]
    async fn a_save_of_a_file_deleted_elsewhere_re_creates_it() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [107u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_row_in_folder(&bridge, &server, "deleted-elsewhere");
        fp_save_moved(
            &bridge,
            dir.path(),
            "deleted-elsewhere",
            "notes.txt",
            "folder-a",
            b"the edit",
            "1",
        );
        // The server deleted it for good.
        server.state.lock().unwrap().files.remove("deleted-elsewhere");
        let delete = crate::api_client::SyncOp {
            seq_id: 702,
            op_type: "file_delete".into(),
            payload: serde_json::json!({ "id": "deleted-elsewhere" }),
        };
        apply_sync_op(&bridge, &sync_root, &delete, now_secs(), &mut Vec::new()).unwrap();
        assert!(
            bridge.db.get_file("deleted-elsewhere").unwrap().is_some(),
            "the row with a queued save is kept"
        );
        assert!(listed_under(&bridge, "folder-a").contains(&"deleted-elsewhere".to_string()));

        drain_upload_queue(&bridge, &sync_root).await;
        assert_eq!(
            name_parent_status(&bridge, "deleted-elsewhere"),
            ("Docs/notes.txt".into(), Some("folder-a".into()), FileStatus::Local),
            "re-created, in its folder"
        );
        assert!(listed_under(&bridge, "folder-a").contains(&"deleted-elsewhere".to_string()));
        let state = server.finish();
        assert_eq!(state.latest_plaintext("deleted-elsewhere", master_key), b"the edit");
    }

    /// Spec §7.2 (final review): a move to the top level made elsewhere (`file_move` or
    /// `folder_move` with `"new_parent_id": null`, as the server's PATCH emits it, and the bare
    /// `file_move` of its restore re-home) leaves the row with no parent and its name alone as the
    /// path: a `Local` file, a `CloudOnly` file and folders. The root lists each; the old folder
    /// lists none.
    #[cfg(unix)] // names `crate::ipc_socket::list_file_provider_items`
    #[test]
    fn a_remote_move_to_the_top_level_clears_the_parent() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let bridge = test_bridge(&dir.path().join("state.db"));
        seed_folder_row(&bridge, "folder-a", "Docs");
        let rows = [
            (
                "local-file",
                "Docs/local.txt",
                FileStatus::Local,
                ItemKind::File,
                "local.txt",
            ),
            (
                "cloud-file",
                "Docs/cloud.txt",
                FileStatus::CloudOnly,
                ItemKind::File,
                "cloud.txt",
            ),
            ("sub-folder", "Docs/Sub", FileStatus::Local, ItemKind::Folder, "Sub"),
            (
                "rehomed-folder",
                "Docs/Restored",
                FileStatus::CloudOnly,
                ItemKind::Folder,
                "Restored",
            ),
        ];
        for (id, path, status, kind, _) in &rows {
            bridge
                .db
                .upsert_file_with_contract(
                    &FileEntry {
                        file_id: (*id).into(),
                        path: (*path).into(),
                        status: status.clone(),
                        size_bytes: 3,
                        modified_at: 1_700_000_000,
                        content_hash: None,
                        remote_updated_at: 1_700_000_000,
                        parent_id: Some("folder-a".into()),
                        item_kind: kind.clone(),
                    },
                    |contract| {
                        contract.parent_id = Some("folder-a".into());
                        contract.item_kind = kind.clone();
                    },
                )
                .unwrap();
        }
        let ops = [
            (
                "file_move",
                serde_json::json!({ "id": "local-file", "old_parent_id": "folder-a", "new_parent_id": null }),
            ),
            (
                "file_move",
                serde_json::json!({ "id": "cloud-file", "old_parent_id": "folder-a", "new_parent_id": null }),
            ),
            (
                "folder_move",
                serde_json::json!({ "id": "sub-folder", "old_parent_id": "folder-a", "new_parent_id": null }),
            ),
            // The restore re-home: `file_move` for a folder too, with the id and a null parent only.
            (
                "file_move",
                serde_json::json!({ "id": "rehomed-folder", "new_parent_id": null }),
            ),
        ];
        for (seq_id, (op_type, payload)) in (601..).zip(ops) {
            let op = crate::api_client::SyncOp {
                seq_id,
                op_type: op_type.into(),
                payload,
            };
            apply_sync_op(&bridge, &sync_root, &op, now_secs(), &mut Vec::new()).unwrap();
        }
        let root = listed_under(&bridge, FP_ROOT);
        let old_folder = listed_under(&bridge, "folder-a");
        for (id, _, _, _, name) in &rows {
            let row = bridge.db.get_file(id).unwrap().unwrap();
            assert_eq!((row.path.as_str(), row.parent_id.as_deref()), (*name, None), "{id}");
            assert!(root.contains(&id.to_string()), "{id} is listed at the root: {root:?}");
            assert!(!old_folder.contains(&id.to_string()), "{id} is not in its old folder");
        }
    }

    /// Spec §7.2 (final review): ingestion writes a new row with its parent, namespace and kind
    /// in ONE call, `upsert_file_with_contract`, so no listing can see a new nested row without
    /// its parent, or a new shared row in the person's own namespace. After the single call the
    /// row carries them, and the root lists neither.
    #[cfg(unix)] // names `crate::ipc_socket::list_file_provider_items`
    #[test]
    fn ingestion_writes_a_new_row_with_its_parent_and_namespace() {
        let dir = tempfile::tempdir().unwrap();
        let bridge = test_bridge(&dir.path().join("state.db"));
        seed_folder_row(&bridge, "folder-a", "Docs");
        // The person's own tree, nested.
        apply_metadata_file_row(
            &bridge.db,
            &serde_json::json!({ "id": "nested", "name": "n.txt", "parent_id": "folder-a", "size_bytes": 3 }),
            Namespace::MyFiles,
            None,
            None,
            PERMISSION_READ | PERMISSION_WRITE | PERMISSION_OWNER,
            1_700_000_000,
            &[0u8; 32],
            "Docs",
        )
        .unwrap();
        // A shared folder's child.
        let root = SharedRootMapping {
            invite_id: "invite-1".into(),
            file_id: "shared-root".into(),
            display_name: "Team".into(),
            is_folder: true,
            size_bytes: 0,
            content_type: None,
            owner_email: None,
            sender_public_key: None,
            encrypted_file_key: None,
            encrypted_folder_key: None,
            file_name_encrypted: None,
            permission_bits: PERMISSION_READ,
            approved_at: None,
        };
        apply_shared_metadata_file_row(
            &bridge.db,
            &serde_json::json!({ "id": "shared-child", "size_bytes": 3, "is_folder": false }),
            &root,
            1_700_000_000,
            Some(root.file_id.clone()),
            "Shared with me/Team",
            None,
        )
        .unwrap();
        // A create through the shared entry point (the watcher's), in a folder.
        let contents = dir.path().join("w.txt");
        std::fs::write(&contents, b"watched").unwrap();
        let FinderWriteOutcome::Queued {
            file_id: Some(watched), ..
        } = bridge
            .queue_finder_create(FinderWriteTarget {
                file_id: None,
                parent_id: Some("folder-a".into()),
                filename: "w.txt".into(),
                rel_path: Some("Docs/w.txt".into()),
                kind: FinderWriteItemKind::File,
                contents_path: Some(contents.to_string_lossy().into_owned()),
                content_type: Some("text/plain".into()),
                base_version_identifier: None,
            })
            .unwrap()
        else {
            panic!("a queued create")
        };

        let contract = |id: &str| bridge.db.get_file_contract_state(id).unwrap().unwrap();
        assert_eq!(
            (contract("nested").parent_id, contract("nested").namespace),
            (Some("folder-a".into()), Namespace::MyFiles)
        );
        assert_eq!(
            (contract("shared-child").parent_id, contract("shared-child").namespace),
            (Some("shared-root".into()), Namespace::SharedWithMe)
        );
        assert_eq!(
            (contract(&watched).parent_id, contract(&watched).item_kind),
            (Some("folder-a".into()), ItemKind::File)
        );
        let at_root = listed_under(&bridge, FP_ROOT);
        for id in ["nested", "shared-child", watched.as_str()] {
            assert!(
                !at_root.contains(&id.to_string()),
                "{id} is not at the top level: {at_root:?}"
            );
        }
    }

    /// Spec §7.2 (final review): the shared-root refresh writes a new shared root with its
    /// namespace in the same call. After `refresh_shared_roots`, the row is `SharedWithMe` with
    /// no parent, and the root does not list it.
    #[cfg(unix)] // names `crate::ipc_socket::list_file_provider_items`
    #[tokio::test]
    async fn the_shared_root_refresh_writes_its_row_with_its_namespace() {
        let dir = tempfile::tempdir().unwrap();
        let server = VersionedServerMock::start();
        server.state.lock().unwrap().incoming_invites = Some(serde_json::json!({
            "invites": [{
                "status": "approved",
                "id": "invite-1",
                "file_id": "shared-file",
                "is_folder_share": false,
                "size_bytes": 3
            }]
        }));
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), [111u8; 32]);
        bridge.refresh_shared_roots().await.unwrap();
        let contract = bridge.db.get_file_contract_state("shared-file").unwrap().unwrap();
        assert_eq!(
            (
                contract.namespace,
                contract.parent_id,
                contract.shared_root_id.as_deref(),
                contract.share_id.as_deref()
            ),
            (Namespace::SharedWithMe, None, Some("shared-file"), Some("invite-1"))
        );
        assert!(!listed_under(&bridge, FP_ROOT).contains(&"shared-file".to_string()));
        drop(server.finish());
    }

    /// Spec §7.2 (final review): a `*_move` echo with no `new_parent_id` key names no new parent:
    /// the row keeps both its parent and its path. (The server's own op push reads an absent key
    /// as the top level; no client pushes a move today, §12.)
    #[cfg(unix)] // names `crate::ipc_socket::list_file_provider_items`
    #[test]
    fn a_move_echo_without_a_parent_key_keeps_the_parent_and_the_path() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let bridge = test_bridge(&dir.path().join("state.db"));
        seed_folder_row(&bridge, "folder-a", "Docs");
        bridge
            .db
            .upsert_file_with_contract(
                &FileEntry {
                    file_id: "kept-file".into(),
                    path: "Docs/kept.txt".into(),
                    status: FileStatus::Local,
                    size_bytes: 3,
                    modified_at: 1_700_000_000,
                    content_hash: None,
                    remote_updated_at: 1_700_000_000,
                    parent_id: Some("folder-a".into()),
                    item_kind: ItemKind::File,
                },
                |contract| contract.parent_id = Some("folder-a".into()),
            )
            .unwrap();
        let op = crate::api_client::SyncOp {
            seq_id: 801,
            op_type: "file_move".into(),
            payload: serde_json::json!({ "id": "kept-file" }),
        };
        apply_sync_op(&bridge, &sync_root, &op, now_secs(), &mut Vec::new()).unwrap();
        let row = bridge.db.get_file("kept-file").unwrap().unwrap();
        assert_eq!(
            (row.path.as_str(), row.parent_id.as_deref()),
            ("Docs/kept.txt", Some("folder-a")),
            "the parent and the path are kept together"
        );
        assert!(listed_under(&bridge, "folder-a").contains(&"kept-file".to_string()));
        assert!(!listed_under(&bridge, FP_ROOT).contains(&"kept-file".to_string()));
    }

    /// Spec §7.2 (final review): the writers that insert a row from the server or a create write
    /// its contract in the same call. The two-call shape (`upsert_file`, then
    /// `set_file_contract_state`) is gone from production code: `set_file_contract_state` exists
    /// for tests only, and each writer calls `upsert_file_with_contract`. A Finder folder create
    /// writes its row and contract through `enqueue_finder_change`, with its op.
    #[test]
    fn ingestion_writers_use_the_single_transaction_api() {
        let source = include_str!("engine_bridge.rs");
        let production = source.split("\nmod tests {").next().unwrap();
        assert!(
            !production.contains(".set_file_contract_state("),
            "production code writes a contract in a second call"
        );
        let state_db = include_str!("state_db.rs");
        let at = state_db
            .find("    pub fn set_file_contract_state(")
            .expect("the test-only writer");
        assert!(
            state_db[..at].trim_end().ends_with("#[cfg(test)]"),
            "set_file_contract_state is for tests only"
        );
        // Each writer's body, from its signature to the brace that closes it, so the window does
        // not depend on what follows the function. One call per row it inserts: the folder and
        // the file branch of `queue_finder_create_from` count apart.
        fn body<'a>(source: &'a str, signature: &str) -> &'a str {
            let start = source
                .find(signature)
                .unwrap_or_else(|| panic!("{signature} not found"));
            let open = start + source[start..].find('{').unwrap();
            let mut depth = 0usize;
            for (i, c) in source[open..].char_indices() {
                match c {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            return &source[start..=open + i];
                        }
                    }
                    _ => {}
                }
            }
            panic!("{signature} has no closing brace")
        }
        for (writer, calls) in [
            ("pub async fn refresh_shared_roots(", 1),
            ("pub fn queue_finder_create_from(", 1),
            ("fn apply_shared_metadata_file_row(", 1),
            ("fn apply_metadata_file_row(", 1),
        ] {
            // Task 2204 S0.3: the pull's writer is `upsert_server_file_row`, the same one-transaction
            // write with the Windows-rename hold decided inside it.
            let body = body(production, writer);
            assert_eq!(
                body.matches("upsert_file_with_contract(").count() + body.matches("upsert_server_file_row(").count(),
                calls,
                "{writer} writes each row and its contract in one call"
            );
        }
        let held = body(state_db, "    pub fn upsert_server_file_row(");
        assert!(
            held.contains("upsert_file_conn(") && held.contains("set_file_contract_state_conn(") && held.contains("tx.commit()"),
            "{held}"
        );
        // Spec 2026-10-10 §5.2 step 6: a Finder folder create writes its row and its contract in the same
        // transaction as its op, behind the sign-out's fence, so a refused create leaves no row.
        assert!(
            body(production, "pub fn queue_finder_create_from(").contains("Some(crate::state_db::FinderFolderRow {"),
            "the folder branch hands its row to the fenced enqueue"
        );
        let fenced = body(state_db, "    pub fn enqueue_finder_change(");
        assert!(
            fenced.contains("upsert_file_conn(") && fenced.contains("set_file_contract_state_conn("),
            "{fenced}"
        );
    }

