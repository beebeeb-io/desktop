//! `conflicts_2` engine tests, split out of `engine_bridge.rs`.

use super::*;
    /// T43 (I-1): a deletion-conflicted create of P is a content modify of S, replied under S.
    #[tokio::test]
    async fn a_deletion_conflicted_create_of_the_provisional_id_modifies_the_server_file() {
        let master_key = [91u8; 32];
        let (dir, sync_root, server, bridge) = rule3_setup(master_key);
        let (p, s, create_token) = landed_create(&bridge, &server, dir.path(), &sync_root, "t2.txt").await;
        let edited = dir.path().join("t2-edited.txt");
        std::fs::write(&edited, b"created bytes, edited before the swap").unwrap();
        let reply = bridge
            .queue_file_provider_deletion_conflicted_create(
                finder_file_target(None, "t2.txt", &edited, None),
                Some(p.clone()),
                Some(create_token),
                None,
            )
            .unwrap();
        let crate::engine_bridge::FinderWriteOutcome::Queued { file_id: Some(id), .. } = &reply.outcome else {
            panic!("{reply:?}")
        };
        assert_eq!(id, &s, "a content modify of S");
        assert!(
            reply.present_as.is_none(),
            "a create reply names the provider's identifier, S (REPL.h:438-443)"
        );
        // Guard: without an alias it is an ordinary create.
        let other = dir.path().join("t3.txt");
        std::fs::write(&other, b"a file deleted elsewhere, edited here").unwrap();
        bridge
            .queue_file_provider_deletion_conflicted_create(
                finder_file_target(None, "t3.txt", &other, None),
                Some("3f2a9c1e-0000-4000-8000-0000000000aa".into()),
                Some("0".into()),
                None,
            )
            .unwrap();
        drain_upload_queue(&bridge, &sync_root).await;
        let state = server.finish();
        assert_eq!(state.files[&s].versions.len(), 2, "one new version of S");
        assert_eq!(
            state.latest_plaintext(&s, master_key),
            b"created bytes, edited before the swap"
        );
        assert_eq!(
            state.files_with_content().len(),
            2,
            "S and the guard's new file; no second t2"
        );
    }

    /// T54: every request kind under P reaches S (one case per kind).
    #[cfg(unix)] // names `crate::ipc_socket::file_status_response`
    #[tokio::test]
    async fn the_alias_resolves_rename_move_item_thumbnail_and_hydrate() {
        let master_key = [92u8; 32];
        let (dir, sync_root, server, bridge) = rule3_setup(master_key);
        // The thumbnail and hydrate paths parse a UUID before they ask the server.
        server.state.lock().unwrap().uuid_ids = true;
        let (p, s, create_token) = landed_create(&bridge, &server, dir.path(), &sync_root, "aliased.txt").await;
        let blob = thumbnail_blob(master_key, &s, b"thumbnail bytes");
        server.state.lock().unwrap().thumbnails.insert(s.clone(), blob);
        let rename = |parent: Option<&str>, name: &str| FinderWriteTarget {
            file_id: Some(p.clone()),
            parent_id: parent.map(str::to_string),
            filename: name.into(),
            rel_path: None,
            kind: FinderWriteItemKind::File,
            contents_path: None,
            content_type: None,
            base_version_identifier: Some(create_token.clone()),
        };
        // rename
        bridge.queue_file_provider_modify(rename(None, "renamed.txt")).unwrap();
        // move
        bridge
            .queue_file_provider_modify(rename(Some("3f2a9c1e-0000-4000-8000-0000000000f0"), "renamed.txt"))
            .unwrap();
        let ops = bridge.db.list_operations_for_file(&s).unwrap();
        assert_eq!(
            ops.iter().map(|op| op.kind.clone()).collect::<Vec<_>>(),
            vec![OperationKind::RenameFile, OperationKind::MoveFile],
            "rename and move reach S"
        );
        // item
        let crate::ipc_socket::IpcResponse::FileStatus(item) = crate::ipc_socket::file_status_response(&bridge.db, &p)
        else {
            panic!("item under P")
        };
        assert_eq!(item.identifier, p);
        // thumbnail
        assert_eq!(
            &bridge.finder_thumbnail(&p, "small").await.unwrap()[..],
            b"thumbnail bytes"
        );
        // hydrate (no write queued: the server is asked for S)
        let dest = dir.path().join("fetch").join("aliased.txt");
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        let _ = bridge.serve_hydrate(&p, &dest, &[dir.path()], None).await;
        let state = server.finish();
        assert_eq!(state.thumbnail_requests, vec![s.clone()]);
        assert!(
            state
                .requests
                .iter()
                .any(|(m, path)| m == "GET" && path == &format!("/api/v1/files/{s}")),
            "the hydrate asked for S: {:?}",
            state.requests
        );
    }

    /// T55: an alias is kept 30 days; engine start and the daily tick sweep older ones.
    #[test]
    fn aliases_older_than_30_days_are_swept() {
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        let now = now_secs();
        db.insert_alias_for_test("young", "s1", now - 29 * 86_400);
        db.insert_alias_for_test("old", "s2", now - 31 * 86_400);
        db.engine_start_repair().unwrap(); // at start
        assert_eq!(db.resolve_alias("young").unwrap(), Some("s1".into()));
        assert_eq!(db.resolve_alias("old").unwrap(), None);
        db.insert_alias_for_test("old-again", "s3", now - 31 * 86_400);
        assert_eq!(db.sweep_aliases(now, crate::state_db::ALIAS_MAX_AGE_SECS).unwrap(), 1); // daily
        assert_eq!(
            db.resolve_alias("young").unwrap(),
            Some("s1".into()),
            "the young one stays"
        );
    }

    /// T56′ (§5.6, the safe default): no thumbnail is fetched while the write is queued.
    #[tokio::test]
    async fn the_thumbnail_of_a_queued_write_is_an_error_without_a_server_request() {
        // UUID-shaped: the thumbnail fetch parses the id before it asks the server.
        let photo = "3f2a9c1e-0000-4000-8000-0000000000b1";
        let master_key = [93u8; 32];
        let (dir, sync_root, server, bridge) = rule3_setup(master_key);
        seed_uploaded_row(&bridge, &server, photo);
        let blob = thumbnail_blob(master_key, photo, b"new thumbnail");
        server.state.lock().unwrap().thumbnails.insert(photo.into(), blob);
        fp_save(&bridge, dir.path(), photo, "photo.png", b"new image bytes", "1");
        assert!(
            bridge.finder_thumbnail(photo, "small").await.is_err(),
            "an error while the write is queued"
        );
        assert!(
            server.state.lock().unwrap().thumbnail_requests.is_empty(),
            "0 requests: never the old version's thumbnail"
        );
        drain_upload_queue(&bridge, &sync_root).await;
        assert_eq!(
            &bridge.finder_thumbnail(photo, "small").await.unwrap()[..],
            b"new thumbnail",
            "served after the landing"
        );
        assert_eq!(server.finish().thumbnail_requests, vec![photo.to_string()]);
    }

    /// §5.6: the landing takes the op off the queue before the new version's thumbnails are
    /// uploaded. Until they are, the server holds the previous version's (one per file), and the
    /// system would cache it under the token: the thumbnail is refused, and nothing is asked of
    /// the server.
    #[tokio::test]
    async fn the_thumbnail_waits_for_the_landed_versions_thumbnails() {
        let photo = "3f2a9c1e-0000-4000-8000-0000000000b5";
        let master_key = [97u8; 32];
        let (dir, sync_root, server, bridge) = rule3_setup(master_key);
        seed_uploaded_row(&bridge, &server, photo);
        {
            let mut state = server.state.lock().unwrap();
            let previous = thumbnail_blob(master_key, photo, b"the previous version's thumbnail");
            state.thumbnails.insert(photo.into(), previous);
            state.delay_thumbnail_put = Some(Duration::from_millis(400));
        }
        let png = dir.path().join("photo-save.png");
        write_test_png(&png);
        bridge
            .queue_file_provider_modify(FinderWriteTarget {
                file_id: Some(photo.into()),
                parent_id: None,
                filename: "photo.png".into(),
                rel_path: None,
                kind: FinderWriteItemKind::File,
                contents_path: Some(png.to_string_lossy().into_owned()),
                content_type: Some("image/png".into()),
                base_version_identifier: Some("1".into()),
            })
            .unwrap();
        let in_the_window = async {
            // The first thumbnail upload has reached the server: the landing has committed and
            // the new thumbnails are not all uploaded yet.
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            while server.state.lock().unwrap().thumbnail_puts.is_empty() {
                assert!(
                    std::time::Instant::now() < deadline,
                    "no thumbnail upload reached the server: {:?}",
                    server.state.lock().unwrap().requests
                );
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            assert!(
                bridge.db.list_operations_for_file(photo).unwrap().is_empty(),
                "the op left the queue at the landing"
            );
            bridge.finder_thumbnail(photo, "small").await
        };
        let (_, during) = tokio::join!(drain_upload_queue(&bridge, &sync_root), in_the_window);
        let error = during.expect_err("refused while the landed version's thumbnails upload");
        assert!(engine_cause::<ThumbnailNotYet>(&error).is_some(), "{error:#}");
        let served = bridge.finder_thumbnail(photo, "small").await.unwrap();
        assert_ne!(
            &served[..],
            b"the previous version's thumbnail",
            "served once the landed version's thumbnails are uploaded"
        );
        let state = server.finish();
        assert_eq!(state.thumbnail_puts.len(), 2, "two variants uploaded");
        assert_eq!(
            state.thumbnail_requests,
            vec![photo.to_string()],
            "one GET, after the upload"
        );
    }

    /// §5.6: when the landed version's thumbnails fail to upload, the server still holds the
    /// previous version's, so the thumbnail stays refused and nothing is asked of the server.
    #[tokio::test]
    async fn a_failed_thumbnail_upload_keeps_the_thumbnail_refused() {
        let photo = "3f2a9c1e-0000-4000-8000-0000000000b6";
        let master_key = [98u8; 32];
        let (dir, sync_root, server, bridge) = rule3_setup(master_key);
        seed_uploaded_row(&bridge, &server, photo);
        {
            let mut state = server.state.lock().unwrap();
            let previous = thumbnail_blob(master_key, photo, b"the previous version's thumbnail");
            state.thumbnails.insert(photo.into(), previous);
            state.fail_thumbnail_put = true;
        }
        let png = dir.path().join("photo-save.png");
        write_test_png(&png);
        bridge
            .queue_file_provider_modify(FinderWriteTarget {
                file_id: Some(photo.into()),
                parent_id: None,
                filename: "photo.png".into(),
                rel_path: None,
                kind: FinderWriteItemKind::File,
                contents_path: Some(png.to_string_lossy().into_owned()),
                content_type: Some("image/png".into()),
                base_version_identifier: Some("1".into()),
            })
            .unwrap();
        drain_upload_queue(&bridge, &sync_root).await;
        assert!(bridge.db.list_operations_for_file(photo).unwrap().is_empty(), "landed");
        let error = bridge
            .finder_thumbnail(photo, "small")
            .await
            .expect_err("the server still holds the previous version's thumbnail");
        assert!(engine_cause::<ThumbnailNotYet>(&error).is_some(), "{error:#}");
        let state = server.finish();
        assert!(!state.thumbnail_puts.is_empty(), "the upload was attempted");
        assert!(state.thumbnail_requests.is_empty(), "nothing asked of the server");
    }

