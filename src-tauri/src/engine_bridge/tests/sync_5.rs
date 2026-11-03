//! `sync_5` engine tests, split out of `engine_bridge.rs`.

use super::*;
    /// A temporary directory with a sync root, a versioned server and a bridge on `master_key`.
    pub(super) fn rule3_setup(master_key: [u8; 32]) -> (tempfile::TempDir, PathBuf, VersionedServerMock, EngineBridge) {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        (dir, sync_root, server, bridge)
    }

    /// A create landed: returns (P, S, the create's token).
    pub(super) async fn landed_create(
        bridge: &EngineBridge,
        server: &VersionedServerMock,
        dir: &Path,
        sync_root: &Path,
        name: &str,
    ) -> (String, String, String) {
        let created = fp_create(bridge, dir, name, b"created bytes");
        let provisional = created.outcome_file_id();
        drain_upload_queue(bridge, sync_root).await;
        let server_id = server.state.lock().unwrap().files_with_content()[0].clone();
        assert_eq!(
            provisional, server_id,
            "the create lands under the id this device chose, so P is the server id"
        );
        (provisional, server_id, created.token.unwrap())
    }

    /// The encrypted thumbnail blob the server would hold for `file_id`.
    pub(super) fn thumbnail_blob(master_key: [u8; 32], file_id: &str, plaintext: &[u8]) -> Vec<u8> {
        beebeeb_core::encrypt::encrypt_chunk_raw(
            &beebeeb_core::kdf::derive_file_key(
                &beebeeb_core::kdf::MasterKey::from_bytes(master_key),
                file_id.as_bytes(),
            ),
            plaintext,
        )
        .unwrap()
    }

    /// T15 (I3): a save under P after its create landed modifies S, replied under P.
    #[cfg(unix)] // names `crate::ipc_socket::write_outcome_response`
    #[tokio::test]
    async fn i3_create_lands_then_a_modify_under_the_provisional_id() {
        let master_key = [85u8; 32];
        let (dir, sync_root, server, bridge) = rule3_setup(master_key);
        let (p, s, create_token) = landed_create(&bridge, &server, dir.path(), &sync_root, "t.txt").await;
        let reply = fp_save(
            &bridge,
            dir.path(),
            &p,
            "t.txt",
            b"created bytes, edited",
            &create_token,
        );
        assert_eq!(
            reply.present_as, None,
            "P is now the server id itself, so nothing has to be presented under a provisional id"
        );
        let ipc = crate::ipc_socket::write_outcome_response("modify", &bridge.db, Ok(reply.clone()));
        let crate::ipc_socket::IpcResponse::WriteQueued { item: Some(item), .. } = ipc else {
            panic!("an item, never None: {ipc:?}")
        };
        assert_eq!(
            item.identifier, p,
            "presented under P until the system applies the swap"
        );
        assert_eq!(item.content_version, reply.token, "the new token");
        drain_upload_queue(&bridge, &sync_root).await;
        let state = server.finish();
        assert_eq!(state.files_with_content(), vec![s.clone()], "one server file");
        assert_eq!(state.files[&s].versions.len(), 2);
        assert_eq!(state.latest_plaintext(&s, master_key), b"created bytes, edited");
    }

    /// T16: a delete under P naming P's create token trashes S.
    #[tokio::test]
    async fn a_delete_of_the_provisional_id_after_landing_trashes_the_server_file() {
        let (dir, sync_root, server, bridge) = rule3_setup([86u8; 32]);
        let (p, s, create_token) = landed_create(&bridge, &server, dir.path(), &sync_root, "gone.txt").await;
        bridge.queue_finder_delete(&p, Some(create_token)).unwrap();
        drain_upload_queue(&bridge, &sync_root).await;
        assert_eq!(server.finish().trashes, vec![s]);
    }

    /// T58 (m-6): a delete under P that does not name S's held token keeps S. New creates no
    /// longer swap ids (SF-0001 L13), so the alias is seeded as an older client left it.
    #[tokio::test]
    async fn a_delete_through_the_alias_needs_the_held_token_as_base() {
        let (_dir, sync_root, server, bridge) = rule3_setup([87u8; 32]);
        let s = "3f2a9c1e-0000-4000-8000-0000000000c3".to_string();
        let p = "3f2a9c1e-0000-4000-8000-0000000000c4".to_string();
        seed_uploaded_row(&bridge, &server, &s);
        bridge.db.insert_alias_for_test(&p, &s, now_secs());
        let logs = capture_logs_async(async {
            bridge.queue_finder_delete(&p, Some("1".into())).unwrap();
            drain_upload_queue(&bridge, &sync_root).await;
        })
        .await;
        assert!(server.state.lock().unwrap().trashes.is_empty(), "S kept");
        assert!(
            logs.contains("delete of a provisional id not applied to the server file"),
            "{logs}"
        );
        assert!(bridge.db.get_file(&s).unwrap().is_some());
        assert!(bridge.db.get_file(&p).unwrap().is_none(), "P is only an alias");
        drop(server.finish());
    }

    /// T17 (§7.3): a modify of an id with no row and no alias is refused and queues nothing.
    #[cfg(unix)] // names `crate::ipc_socket::write_outcome_response`
    #[tokio::test]
    async fn an_unknown_id_is_refused_never_answered_without_an_item() {
        let (dir, _sync_root, server, bridge) = rule3_setup([88u8; 32]);
        let contents = dir.path().join("orphan.txt");
        std::fs::write(&contents, b"edit of an item nobody knows").unwrap();
        let unknown = "3f2a9c1e-0000-4000-8000-00000000dead";
        let logs = capture_logs_async(async {
            let result = bridge.queue_file_provider_modify(finder_file_target(
                Some(unknown),
                "orphan.txt",
                &contents,
                Some("1".into()),
            ));
            let reply = crate::ipc_socket::write_outcome_response("modify", &bridge.db, result);
            assert!(
                matches!(reply, crate::ipc_socket::IpcResponse::Error { .. }),
                "{reply:?}"
            );
            // A rename or move of the same unknown id is refused the same way.
            let rename = bridge.queue_file_provider_modify(FinderWriteTarget {
                file_id: Some(unknown.into()),
                parent_id: None,
                filename: "orphan renamed.txt".into(),
                rel_path: None,
                kind: FinderWriteItemKind::File,
                contents_path: None,
                content_type: None,
                base_version_identifier: Some("1".into()),
            });
            let reply = crate::ipc_socket::write_outcome_response("modify", &bridge.db, rename);
            assert!(
                matches!(reply, crate::ipc_socket::IpcResponse::Error { .. }),
                "{reply:?}"
            );
        })
        .await;
        assert!(
            bridge.db.list_due_operations(i64::MAX).unwrap().is_empty(),
            "nothing queued"
        );
        assert!(
            logs.matches("Finder write refused").count() == 2 && logs.matches("unknown_item").count() == 2,
            "{logs}"
        );
        drop(server.finish());
    }

    /// T18 (R2a): a fetch of an item whose create is queued is served from the staged bytes.
    #[tokio::test]
    async fn r2a_a_fetch_during_the_creates_queue_wait_is_served_locally() {
        // The mock answers 404 to anything unexpected.
        let (dir, _sync_root, server, bridge) = rule3_setup([89u8; 32]);
        let created = fp_create(&bridge, dir.path(), "r2.txt", b"eight million bytes, in spirit");
        let p = created.outcome_file_id();
        let dest = dir.path().join("fetch").join("r2.txt");
        // `hydrate_dest_is_allowed` canonicalizes the parent.
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        let logs = capture_logs_async(async {
            let source = bridge.serve_hydrate(&p, &dest, &[dir.path()], None).await.unwrap();
            assert_eq!(source, HydrateSource::Queue);
        })
        .await;
        assert_eq!(
            std::fs::read(&dest).unwrap(),
            b"eight million bytes, in spirit",
            "the staged bytes"
        );
        assert_eq!(
            bridge.db.get_file(&p).unwrap().unwrap().status,
            FileStatus::Uploading,
            "still writable"
        );
        assert!(
            logs.contains("Finder hydrate served") && logs.contains("source=\"queue\""),
            "{logs}"
        );
        assert!(server.finish().requests.is_empty(), "nothing reached the server");
    }

    /// T19 (§7.4.2): a failed hydrate from the server leaves a row with a live upload as it
    /// was. Called directly: `serve_hydrate` never sends a still-queued write to the server.
    #[tokio::test]
    async fn a_failed_hydrate_never_changes_a_row_with_a_live_upload() {
        // UUID-shaped: the hydrate parses the id before it asks the server.
        let live = "3f2a9c1e-0000-4000-8000-0000000000b2";
        let (dir, _sync_root, server, bridge) = rule3_setup([90u8; 32]);
        seed_uploaded_row(&bridge, &server, live);
        fp_save(&bridge, dir.path(), live, "notes.txt", b"queued edit", "1");
        let dest = dir.path().join("fetch").join("notes.txt");
        // Without it the guard fails first and the test passes for the wrong reason.
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        assert!(
            bridge
                .hydrate_file_with_progress(live, &dest, &[dir.path()], None)
                .await
                .is_err(),
            "the server has no metadata route"
        );
        assert_eq!(bridge.db.get_file(live).unwrap().unwrap().status, FileStatus::Uploading);
        let state = server.finish();
        assert!(
            state
                .requests
                .iter()
                .any(|(m, path)| m == "GET" && path == &format!("/api/v1/files/{live}")),
            "the hydrate failed at the server, not before it: {:?}",
            state.requests
        );
    }

    /// §7.4: a fetch of an item whose held write is still queued never reaches the server, even
    /// when the write's staged copy is missing: the server holds only the previous version, and
    /// the system would store it under the new token. The fetch fails instead, and the row is
    /// left as it was.
    #[tokio::test]
    async fn a_queued_write_whose_copy_is_missing_is_never_fetched_from_the_server() {
        let live = "3f2a9c1e-0000-4000-8000-0000000000b3";
        let (dir, _sync_root, server, bridge) = rule3_setup([95u8; 32]);
        seed_uploaded_row(&bridge, &server, live);
        fp_save(&bridge, dir.path(), live, "notes.txt", b"queued edit", "1");
        let op = bridge.db.list_operations_for_file(live).unwrap().remove(0);
        std::fs::remove_file(op.payload_path.as_deref().unwrap()).unwrap();
        let dest = dir.path().join("fetch").join("notes.txt");
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        let error = bridge
            .serve_hydrate(live, &dest, &[dir.path()], None)
            .await
            .expect_err("the bytes the token names are missing");
        assert!(engine_cause::<QueuedCopyMissing>(&error).is_some(), "{error:#}");
        assert!(!dest.exists(), "nothing was written");
        assert_eq!(bridge.db.get_file(live).unwrap().unwrap().status, FileStatus::Uploading);
        let state = server.finish();
        assert!(
            state.requests.is_empty(),
            "nothing reached the server: {:?}",
            state.requests
        );
    }

    /// §7.4 (4b): a landing that takes the write off the queue and unlinks its copy between
    /// the read and the open sends the fetch to the server, which now has those bytes.
    #[tokio::test]
    async fn a_copy_unlinked_by_the_landing_sends_the_fetch_to_the_server() {
        let live = "3f2a9c1e-0000-4000-8000-0000000000b4";
        let (dir, _sync_root, server, bridge) = rule3_setup([96u8; 32]);
        seed_uploaded_row(&bridge, &server, live);
        fp_save(&bridge, dir.path(), live, "notes.txt", b"queued edit", "1");
        let op = bridge.db.list_operations_for_file(live).unwrap().remove(0);
        let (db, op_id, copy) = (bridge.db.clone(), op.op_id.clone(), op.payload_path.clone().unwrap());
        // The landing, as it looks from here: the op leaves the queue, then its copy is unlinked.
        bridge.seams.arm("hydrate:before_open", move || {
            db.remove_operation(&op_id).unwrap();
            std::fs::remove_file(&copy).unwrap();
        });
        let dest = dir.path().join("fetch").join("notes.txt");
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        let error = bridge
            .serve_hydrate(live, &dest, &[dir.path()], None)
            .await
            .expect_err("the mock has no metadata route");
        assert!(engine_cause::<QueuedCopyMissing>(&error).is_none(), "no longer queued: {error:#}");
        let state = server.finish();
        assert!(
            state
                .requests
                .iter()
                .any(|(m, path)| m == "GET" && path == &format!("/api/v1/files/{live}")),
            "the fetch went to the server: {:?}",
            state.requests
        );
    }

