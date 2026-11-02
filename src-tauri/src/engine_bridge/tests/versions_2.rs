//! `versions_2` engine tests, split out of `engine_bridge.rs`.

use super::*;
    #[cfg(unix)] // asserts a token through `held_content_version`
    #[tokio::test]
    async fn the_token_survives_rename_move_and_trash() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [56u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        let token = land_one_save(&bridge, &server, dir.path(), &sync_root, "moved").await;
        let rename = FinderWriteTarget {
            file_id: Some("moved".into()),
            parent_id: None,
            filename: "renamed.txt".into(),
            rel_path: None,
            kind: FinderWriteItemKind::File,
            contents_path: None,
            content_type: Some("text/plain".into()),
            base_version_identifier: Some(token.clone()),
        };
        bridge.queue_file_provider_modify(rename).unwrap();
        assert_eq!(held_content_version(&bridge, "moved"), token, "rename");
        bridge.queue_finder_delete("moved", Some(token.clone())).unwrap();
        assert_eq!(held_content_version(&bridge, "moved"), token, "trash");
        drop(server.finish());
    }

    /// A top-level folder row, as a snapshot writes it.
    pub(super) fn seed_folder_row(bridge: &EngineBridge, folder_id: &str, path: &str) {
        bridge
            .db
            .upsert_file(&FileEntry {
                file_id: folder_id.into(),
                path: path.into(),
                status: FileStatus::Local,
                size_bytes: 0,
                modified_at: 1_700_000_000,
                content_hash: None,
                remote_updated_at: 1_700_000_000,
                parent_id: None,
                item_kind: ItemKind::Folder,
            })
            .unwrap();
        let mut contract = bridge.db.get_file_contract_state(folder_id).unwrap().unwrap();
        contract.item_kind = ItemKind::Folder;
        contract.current_version = 1;
        bridge.db.set_file_contract_state(&contract).unwrap();
    }

    /// [`seed_uploaded_row`], nested as a snapshot writes it: `Docs/notes.txt` in `folder-a`
    /// (`Docs`). The folders `folder-b` (`Other`) and `folder-c` (`Third`) exist too.
    pub(super) fn seed_row_in_folder(bridge: &EngineBridge, server: &VersionedServerMock, file_id: &str) {
        seed_folder_row(bridge, "folder-a", "Docs");
        seed_folder_row(bridge, "folder-b", "Other");
        seed_folder_row(bridge, "folder-c", "Third");
        seed_uploaded_row(bridge, server, file_id);
        let mut entry = bridge.db.get_file(file_id).unwrap().unwrap();
        entry.path = "Docs/notes.txt".into();
        bridge.db.upsert_file(&entry).unwrap();
        let mut contract = bridge.db.get_file_contract_state(file_id).unwrap().unwrap();
        contract.parent_id = Some("folder-a".into());
        bridge.db.set_file_contract_state(&contract).unwrap();
    }

    /// A File Provider save whose item the person also renamed and moved before the system
    /// synced it: the modify carries the contents with the new name and parent.
    pub(super) fn fp_save_moved(
        bridge: &EngineBridge,
        dir: &Path,
        file_id: &str,
        filename: &str,
        parent_id: &str,
        bytes: &[u8],
        base: &str,
    ) -> FpWrite {
        let contents = dir.join(format!("save-{}.txt", uuid::Uuid::new_v4()));
        std::fs::write(&contents, bytes).unwrap();
        let mut target = finder_file_target(Some(file_id), filename, &contents, Some(base.to_string()));
        target.parent_id = Some(parent_id.to_string());
        bridge.queue_file_provider_modify(target).unwrap()
    }

    pub(super) fn name_parent_status(bridge: &EngineBridge, file_id: &str) -> (String, Option<String>, FileStatus) {
        let row = bridge.db.get_file(file_id).unwrap().unwrap();
        (row.path, row.parent_id, row.status)
    }

    /// Spec §7.2 (final review): a save that also renames and moves its file is described by
    /// the new name and parent from its reply on, and keeps them through its landing. The
    /// server gets both with the write.
    #[cfg(unix)] // names `crate::ipc_socket::write_outcome_response`
    #[tokio::test]
    async fn a_save_that_renames_and_moves_is_described_by_its_new_name_and_parent() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [96u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_row_in_folder(&bridge, &server, "moved-save");
        let reply = fp_save_moved(
            &bridge,
            dir.path(),
            "moved-save",
            "renamed.txt",
            "folder-b",
            b"saved under a new name",
            "1",
        );
        let ipc = crate::ipc_socket::write_outcome_response("modify", &bridge.db, Ok(reply));
        let crate::ipc_socket::IpcResponse::WriteQueued { item: Some(item), .. } = ipc else {
            panic!("an item, never None: {ipc:?}")
        };
        assert_eq!(
            (item.filename.as_str(), item.parent_identifier.as_str()),
            ("renamed.txt", "folder-b"),
            "the reply describes the item as the person left it"
        );
        assert_eq!(
            name_parent_status(&bridge, "moved-save"),
            (
                "Other/renamed.txt".into(),
                Some("folder-b".into()),
                FileStatus::Uploading
            ),
            "the accept applied the name and the parent, with the full relative path"
        );

        drain_upload_queue(&bridge, &sync_root).await;
        assert_eq!(
            name_parent_status(&bridge, "moved-save"),
            ("Other/renamed.txt".into(), Some("folder-b".into()), FileStatus::Local),
            "the landing keeps them"
        );
        let state = server.finish();
        assert_eq!(
            state.latest_plaintext("moved-save", master_key),
            b"saved under a new name"
        );
        let parents: Vec<_> = state
            .patches
            .iter()
            .filter(|(id, _)| id == "moved-save")
            .map(|(_, body)| body["parent_id"].clone())
            .collect();
        assert_eq!(
            parents,
            vec![serde_json::json!("folder-b")],
            "the move reached the server"
        );
    }

    /// Spec §7.2 (final review): a landing keeps the name and parent of a later save of the
    /// file, which that save's accept applied. The server gets each save's in the order of the
    /// saves.
    #[tokio::test]
    async fn a_landing_keeps_the_name_and_parent_of_a_later_queued_save() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [97u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_row_in_folder(&bridge, &server, "two-moves");
        // The second save's first chunk fails once, so the first save lands alone.
        server
            .state
            .lock()
            .unwrap()
            .fail_first_chunk_once
            .insert("session-2".into());
        let first = fp_save_moved(&bridge, dir.path(), "two-moves", "first.txt", "folder-b", b"first", "1");
        fp_save_moved(
            &bridge,
            dir.path(),
            "two-moves",
            "second.txt",
            "folder-c",
            b"second",
            &first.token.unwrap(),
        );

        bridge.process_due_operations(&sync_root, now_secs()).await.unwrap();
        assert_eq!(
            server.state.lock().unwrap().files["two-moves"].versions.len(),
            2,
            "the first save landed, the second did not"
        );
        assert_eq!(
            name_parent_status(&bridge, "two-moves"),
            (
                "Third/second.txt".into(),
                Some("folder-c".into()),
                FileStatus::Uploading
            ),
            "the first landing kept the later save's name and parent"
        );

        drain_upload_queue(&bridge, &sync_root).await;
        assert_eq!(
            name_parent_status(&bridge, "two-moves"),
            ("Third/second.txt".into(), Some("folder-c".into()), FileStatus::Local)
        );
        let state = server.finish();
        assert_eq!(state.latest_plaintext("two-moves", master_key), b"second");
        let parents: Vec<_> = state
            .patches
            .iter()
            .filter(|(id, _)| id == "two-moves")
            .map(|(_, body)| body["parent_id"].clone())
            .collect();
        assert_eq!(
            parents,
            vec![serde_json::json!("folder-b"), serde_json::json!("folder-c")],
            "each save's parent, in order"
        );
    }

    /// Spec §7.2 (final review): a save that parks for good keeps the name and parent it was
    /// saved with. The row is never left describing the old ones.
    #[tokio::test]
    async fn a_parked_save_keeps_the_name_and_parent_it_was_saved_with() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [98u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_row_in_folder(&bridge, &server, "parked-move");
        server
            .state
            .lock()
            .unwrap()
            .refuse_init
            .insert("parked-move".into(), "403 Forbidden");
        fp_save_moved(&bridge, dir.path(), "parked-move", "kept.txt", "folder-b", b"kept", "1");

        drain_upload_queue(&bridge, &sync_root).await;
        let op = bridge.db.list_operations_for_file("parked-move").unwrap();
        assert_eq!(op.len(), 1, "the save is kept, parked");
        assert_eq!(op[0].attempts, op[0].max_attempts, "parked");
        assert_eq!(
            name_parent_status(&bridge, "parked-move"),
            ("Other/kept.txt".into(), Some("folder-b".into()), FileStatus::Error)
        );
        drop(server.finish());
    }

    /// The File Provider root container's identifier, as the system names it.
    #[cfg(unix)] // only the listing tests use it
    pub(super) const FP_ROOT: &str = "NSFileProviderRootContainerItemIdentifier";

    /// The ids the File Provider enumeration lists under `container`.
    #[cfg(unix)] // names `crate::ipc_socket::list_file_provider_items`
    pub(super) fn listed_under(bridge: &EngineBridge, container: &str) -> Vec<String> {
        crate::ipc_socket::list_file_provider_items(&bridge.db, container)
            .into_iter()
            .map(|item| item.identifier)
            .collect()
    }

    /// How many `reparented` change rows `file_id` has.
    pub(super) fn reparented_changes(bridge: &EngineBridge, file_id: &str) -> usize {
        let (changes, _) = bridge.db.list_file_changes_paged(None, 1000).unwrap().unwrap();
        changes
            .iter()
            .filter(|change| change.file_id == file_id && change.kind == crate::state_db::FpChangeKind::Reparented)
            .count()
    }

    /// A File Provider create of `filename` in the folder `parent_id`, as the extension sends it.
    #[cfg(unix)] // only the unix listing tests use it
    pub(super) fn fp_create_in(bridge: &EngineBridge, dir: &Path, parent_id: &str, filename: &str, bytes: &[u8]) -> FpWrite {
        let contents = dir.join(format!("create-{}.txt", uuid::Uuid::new_v4()));
        std::fs::write(&contents, bytes).unwrap();
        let mut target = finder_file_target(None, filename, &contents, None);
        target.parent_id = Some(parent_id.to_string());
        bridge.queue_file_provider_create(target).unwrap()
    }

    /// Spec §7.2 (final review): a create inside a folder is placed in that folder from its
    /// accept on. The row's parent and full relative path, the reply's parent and the
    /// listings say so while the create is queued, and after it lands as the server file.
    #[cfg(unix)] // names `crate::ipc_socket`
    #[tokio::test]
    async fn a_nested_create_stays_in_its_folder_while_queued_and_after_it_lands() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [99u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_folder_row(&bridge, "r9-folder", "r9-folder");
        let created = fp_create_in(&bridge, dir.path(), "r9-folder", "n1.txt", b"nested");
        let p = created.outcome_file_id();
        let ipc = crate::ipc_socket::write_outcome_response("create", &bridge.db, Ok(created));
        let crate::ipc_socket::IpcResponse::WriteQueued { item: Some(item), .. } = ipc else {
            panic!("an item, never None: {ipc:?}")
        };
        assert_eq!(
            (item.filename.as_str(), item.parent_identifier.as_str()),
            ("n1.txt", "r9-folder"),
            "the reply places the new file in its folder, never at the top level"
        );
        assert_eq!(
            name_parent_status(&bridge, &p),
            (
                "r9-folder/n1.txt".into(),
                Some("r9-folder".into()),
                FileStatus::Uploading
            )
        );
        assert!(
            !listed_under(&bridge, FP_ROOT).contains(&p),
            "not listed at the top level while queued"
        );
        assert_eq!(
            listed_under(&bridge, "r9-folder"),
            vec![p.clone()],
            "listed in its folder"
        );

        drain_upload_queue(&bridge, &sync_root).await;
        // SF-0001 (L13): the create proposed its own id, so it lands under it — no swap.
        let s = p.clone();
        assert!(
            bridge.db.get_file(&p).unwrap().is_some(),
            "the create lands under the id this device chose"
        );
        assert_eq!(
            name_parent_status(&bridge, &s),
            ("r9-folder/n1.txt".into(), Some("r9-folder".into()), FileStatus::Local),
            "the landing keeps the full relative path and the parent"
        );
        assert!(
            !listed_under(&bridge, FP_ROOT).contains(&s),
            "not at the top level once landed"
        );
        assert_eq!(listed_under(&bridge, "r9-folder"), vec![s.clone()]);
        let state = server.finish();
        assert_eq!(state.latest_plaintext(&s, master_key), b"nested");
        let parents: Vec<_> = state
            .patches
            .iter()
            .filter(|(id, _)| id == &s)
            .map(|(_, body)| body["parent_id"].clone())
            .collect();
        assert_eq!(
            parents,
            vec![serde_json::json!("r9-folder")],
            "the server file is in the folder"
        );
    }

