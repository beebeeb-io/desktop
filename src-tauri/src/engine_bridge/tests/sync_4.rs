//! `sync_4` engine tests, split out of `engine_bridge.rs`.

use super::*;
    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn base_pending_parks_after_ten_successful_snapshots_without_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [78u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_version_zero_row(&bridge, &server, "d6781532-6323-5217-b51a-bbd120c6b7a6", FileStatus::Local);
        seed_uploaded_row(&bridge, &server, "bfd11147-9d33-58c7-99ae-b5bcf9aee4b1");
        bridge.db.set_sync_cursor(0).unwrap();
        fp_save(&bridge, dir.path(), "d6781532-6323-5217-b51a-bbd120c6b7a6", "notes.txt", b"edit", "0");
        let listed = serde_json::json!({ "seq_id": 1, "nodes": [node(&master_key, "bfd11147-9d33-58c7-99ae-b5bcf9aee4b1", "notes.txt", 1, false)] });
        let op_id = bridge
            .db
            .list_operations_for_file("d6781532-6323-5217-b51a-bbd120c6b7a6")
            .unwrap()
            .remove(0)
            .op_id;
        for success in 1..=10 {
            {
                let mut s = server.state.lock().unwrap();
                s.snapshots
                    .push_back(("503 Service Unavailable".into(), serde_json::json!({ "error": "busy" })));
                s.snapshots.push_back(("200 OK".into(), listed.clone()));
            }
            assert!(
                sync_tick_outcome(&bridge, &sync_root).await.is_err(),
                "a failed snapshot costs nothing"
            );
            sync_tick_outcome(&bridge, &sync_root).await.unwrap();
            let op = bridge.db.get_operation(&op_id).unwrap().unwrap();
            if success < 10 {
                assert!(
                    op.attempts < op.max_attempts,
                    "not parked after {success} successful snapshots"
                );
            } else {
                assert_eq!(op.attempts, op.max_attempts, "parked base_unknown after the 10th");
            }
        }
        let state = server.finish();
        assert!(state.inits.is_empty());
    }

    /// Spec §9.1: the snapshot count's park changes how the item is presented, and the snapshot
    /// did not list the file, so the park puts the id in the tick's applied ids itself; the tick's
    /// one working-set signal carries them. Before the 10th snapshot nothing changed for it.
    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn a_snapshot_count_park_puts_its_item_in_the_ticks_signal() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [128u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_version_zero_row(&bridge, &server, "20571f96-2b78-5f39-ae82-e8e3c79a61f2", FileStatus::Local);
        seed_uploaded_row(&bridge, &server, "bfd11147-9d33-58c7-99ae-b5bcf9aee4b1");
        bridge.db.set_sync_cursor(0).unwrap();
        fp_save(&bridge, dir.path(), "20571f96-2b78-5f39-ae82-e8e3c79a61f2", "notes.txt", b"edit", "0");
        let listed = serde_json::json!({ "seq_id": 1, "nodes": [node(&master_key, "bfd11147-9d33-58c7-99ae-b5bcf9aee4b1", "notes.txt", 1, false)] });
        for success in 1..=10 {
            server
                .state
                .lock()
                .unwrap()
                .snapshots
                .push_back(("200 OK".into(), listed.clone()));
            let tick = sync_tick_outcome(&bridge, &sync_root).await.unwrap();
            let carried = tick.applied_item_ids.contains(&"20571f96-2b78-5f39-ae82-e8e3c79a61f2".to_string());
            if success < 10 {
                assert!(
                    !carried,
                    "nothing changed after {success} snapshots: {:?}",
                    tick.applied_item_ids
                );
            } else {
                let op = bridge
                    .db
                    .list_operations_for_file("20571f96-2b78-5f39-ae82-e8e3c79a61f2")
                    .unwrap()
                    .remove(0);
                assert_eq!(op.attempts, op.max_attempts, "precondition: parked after the 10th");
                assert_eq!(
                    bridge.db.get_file("20571f96-2b78-5f39-ae82-e8e3c79a61f2").unwrap().unwrap().status,
                    FileStatus::Error
                );
                assert!(carried, "the park is carried: {:?}", tick.applied_item_ids);
            }
        }
        let state = server.finish();
        assert!(state.inits.is_empty());
    }

    /// A write the snapshot count parked is parked like any other (spec §8.4): it asks for no
    /// more snapshots, and a save on its token takes its role at the claim and parks the
    /// same way, without a request on a base the client knows is unknown.
    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn a_save_after_a_snapshot_count_park_takes_over_and_parks_without_a_request() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [79u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_version_zero_row(&bridge, &server, "f54b1023-367a-54d0-9383-9b1fcefc40a9", FileStatus::Local);
        seed_uploaded_row(&bridge, &server, "bfd11147-9d33-58c7-99ae-b5bcf9aee4b1");
        bridge.db.set_sync_cursor(0).unwrap();
        let w = fp_save(&bridge, dir.path(), "f54b1023-367a-54d0-9383-9b1fcefc40a9", "notes.txt", b"first edit", "0");
        let w_op = bridge.db.list_operations_for_file("f54b1023-367a-54d0-9383-9b1fcefc40a9").unwrap().remove(0).op_id;
        let listed = serde_json::json!({ "seq_id": 1, "nodes": [node(&master_key, "bfd11147-9d33-58c7-99ae-b5bcf9aee4b1", "notes.txt", 1, false)] });
        for _ in 0..10 {
            server
                .state
                .lock()
                .unwrap()
                .snapshots
                .push_back(("200 OK".into(), listed.clone()));
            sync_tick_outcome(&bridge, &sync_root).await.unwrap();
        }
        let parked = bridge.db.get_operation(&w_op).unwrap().unwrap();
        assert_eq!(parked.attempts, parked.max_attempts, "parked after the 10th snapshot");
        assert_eq!(parked.last_error.as_deref(), Some("base_unknown"));
        // No snapshot is queued on the mock now: a tick that asked for one would fail. The
        // ops path, with nothing to apply, succeeds.
        assert!(
            sync_tick_outcome(&bridge, &sync_root).await.is_ok(),
            "a parked write asks for no more snapshots"
        );
        assert_eq!(bridge.db.peek_resnapshot_request().unwrap(), None);

        let n = fp_save(
            &bridge,
            dir.path(),
            "f54b1023-367a-54d0-9383-9b1fcefc40a9",
            "notes.txt",
            b"first edit, second edit",
            &w.token.unwrap(),
        );
        assert!(n.token.is_some());
        drain_upload_queue(&bridge, &sync_root).await;
        let state = server.finish();
        assert!(
            state.init_summary().is_empty(),
            "no request with a base the client knows is unknown: {:?}",
            state.init_summary()
        );
        let ops = bridge.db.list_operations_for_file("f54b1023-367a-54d0-9383-9b1fcefc40a9").unwrap();
        assert_eq!(ops.len(), 1, "the save took the parked write's role: {ops:?}");
        assert_ne!(ops[0].op_id, w_op);
        assert_eq!(ops[0].attempts, ops[0].max_attempts, "and parked the same way");
        assert_eq!(ops[0].last_error.as_deref(), Some("base_unknown"));
    }

    /// §6.1: `version_filled` is cleared by a landing, by a content op on the row and by an
    /// accepted Finder write.
    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn version_filled_is_cleared_by_a_landing_a_content_op_and_an_accepted_write() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [80u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        let filled = |file_id: &str| bridge.db.item_presentation(file_id).unwrap().unwrap().version_filled;
        let fill = |file_id: &str| {
            let snapshot = crate::api_client::SyncSnapshot {
                seq_id: 2,
                nodes: vec![node(&master_key, file_id, "notes.txt", 1, false)],
            };
            apply_snapshot(&bridge, &sync_root, &snapshot, now_secs(), now_secs(), &mut Vec::new()).unwrap();
        };

        // A landing: the fill reaches the waiting write's row, then the write lands.
        seed_version_zero_row(&bridge, &server, "3f58c864-efeb-5981-85b6-5eccd08beabc", FileStatus::Local);
        fp_save(&bridge, dir.path(), "3f58c864-efeb-5981-85b6-5eccd08beabc", "notes.txt", b"edit", "0");
        fill("3f58c864-efeb-5981-85b6-5eccd08beabc");
        assert!(filled("3f58c864-efeb-5981-85b6-5eccd08beabc"));
        drain_upload_queue(&bridge, &sync_root).await;
        assert!(!filled("3f58c864-efeb-5981-85b6-5eccd08beabc"), "a landing clears it");

        // A content op from another device.
        seed_version_zero_row(&bridge, &server, "b8fcdb66-cc54-59bd-bdf1-c29c597cb4f4", FileStatus::Local);
        fill("b8fcdb66-cc54-59bd-bdf1-c29c597cb4f4");
        assert!(filled("b8fcdb66-cc54-59bd-bdf1-c29c597cb4f4"));
        let update = crate::api_client::SyncOp {
            seq_id: 3,
            op_type: "file_update".into(),
            payload: serde_json::json!({ "id": "b8fcdb66-cc54-59bd-bdf1-c29c597cb4f4", "version_number": 2, "size_bytes": 11 }),
        };
        apply_sync_op(&bridge, &sync_root, &update, now_secs(), &mut Vec::new()).unwrap();
        assert!(!filled("b8fcdb66-cc54-59bd-bdf1-c29c597cb4f4"), "a content op clears it");

        // An accepted Finder write, before it lands.
        seed_version_zero_row(&bridge, &server, "9bb42630-e4eb-5425-be9e-f93415515ecc", FileStatus::Local);
        fill("9bb42630-e4eb-5425-be9e-f93415515ecc");
        assert!(filled("9bb42630-e4eb-5425-be9e-f93415515ecc"));
        fp_save(&bridge, dir.path(), "9bb42630-e4eb-5425-be9e-f93415515ecc", "notes.txt", b"edit", "1");
        assert!(!filled("9bb42630-e4eb-5425-be9e-f93415515ecc"), "an accepted write clears it");
        drop(server.finish());
    }

    /// §6.1 / §6.3.2: only a content op asks for a snapshot or clears `version_filled`. The
    /// server's thumbnail-flag ops (`{id, has_thumbnail}`) carry neither `version_number` nor
    /// `size_bytes`, and follow nearly every image upload; a versionless op that does carry
    /// content still does both.
    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn a_thumbnail_flag_op_is_not_a_content_op() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [81u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_version_zero_row(&bridge, &server, "5f9400c6-7b5f-58bf-ac3e-8b87fe4b037e", FileStatus::Local);
        let snapshot = crate::api_client::SyncSnapshot {
            seq_id: 2,
            nodes: vec![node(&master_key, "5f9400c6-7b5f-58bf-ac3e-8b87fe4b037e", "notes.txt", 1, false)],
        };
        apply_snapshot(&bridge, &sync_root, &snapshot, now_secs(), now_secs(), &mut Vec::new()).unwrap();
        assert_eq!(bridge.db.peek_resnapshot_request().unwrap(), None);
        let filled = || bridge.db.item_presentation("5f9400c6-7b5f-58bf-ac3e-8b87fe4b037e").unwrap().unwrap().version_filled;
        assert!(filled());

        for (seq_id, flag) in [(3, "has_thumbnail"), (4, "has_large_thumbnail")] {
            let thumbnail = crate::api_client::SyncOp {
                seq_id,
                op_type: "file_update".into(),
                payload: serde_json::json!({ "id": "5f9400c6-7b5f-58bf-ac3e-8b87fe4b037e", flag: true }),
            };
            apply_sync_op(&bridge, &sync_root, &thumbnail, now_secs(), &mut Vec::new()).unwrap();
            assert!(filled(), "a {flag} op is not a content op: the fill stands");
            assert_eq!(
                bridge.db.peek_resnapshot_request().unwrap(),
                None,
                "a {flag} op asks for no snapshot"
            );
        }

        // A versionless op that carries content (a legacy replace's size) still does both.
        let content = crate::api_client::SyncOp {
            seq_id: 5,
            op_type: "file_update".into(),
            payload: serde_json::json!({ "id": "5f9400c6-7b5f-58bf-ac3e-8b87fe4b037e", "size_bytes": 12 }),
        };
        apply_sync_op(&bridge, &sync_root, &content, now_secs(), &mut Vec::new()).unwrap();
        assert!(!filled(), "a versionless content op clears the fill");
        assert!(
            bridge.db.peek_resnapshot_request().unwrap().is_some(),
            "a versionless content op asks for a snapshot"
        );
        drop(server.finish());
    }

    // ── M1: the landing is one transaction after a recorded completion (spec §8.6) ──

    #[tokio::test]
    async fn m1_a_chain_failure_after_complete_never_duplicates() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [79u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        let created = fp_create(&bridge, dir.path(), "new.txt", b"created");
        let provisional = created.outcome_file_id();
        fp_save(
            &bridge,
            dir.path(),
            &provisional,
            "new.txt",
            b"created, edited",
            created.token.as_deref().unwrap(),
        );
        // SF-0001 (L13): the create lands under its own id, so there is no second server id to
        // re-key for. The successor's chain fails on the one thing the queue cannot cure: its
        // staged copy is gone, so it parks without a second init.
        let successor = bridge.db.list_operations_for_file(&provisional).unwrap().remove(1);
        std::fs::remove_file(successor.payload_path.as_deref().unwrap()).unwrap();
        let logs = capture_logs_async(async {
            drain_upload_queue(&bridge, &sync_root).await;
        })
        .await;
        let state = server.finish();
        assert_eq!(
            state.files_with_content().len(),
            1,
            "one server file: {:?}",
            state.init_summary()
        );
        assert_eq!(state.inits.len(), 1, "no second init");
        let server_id = state.files_with_content()[0].clone();
        assert_eq!(server_id, provisional, "the create landed under the id it chose");
        assert!(bridge.db.get_file(&server_id).unwrap().is_some(), "the landing was applied");
        let parked = bridge.db.get_operation(&successor.op_id).unwrap().unwrap();
        assert_eq!(
            parked.attempts, parked.max_attempts,
            "the successor parked at its claim"
        );
        assert!(!std::path::Path::new(parked.payload_path.as_deref().unwrap()).exists());
        assert!(logs.contains("reason=\"payload_missing\""), "{logs}");
    }

    // T30 (`i1_a_landing_with_a_later_write_queued_keeps_it_uploading`) is with rule 5's status
    // tests below: it needs a failed attempt to leave a Finder write's row `Uploading`.

    #[cfg(unix)] // asserts tokens through `held_content_version`
    #[tokio::test]
    async fn landing_vs_new_save_the_chain_step_sees_every_successor() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [81u8; 32];
        let server = VersionedServerMock::start();
        let bridge = Arc::new(test_bridge_with_api(
            &dir.path().join("state.db"),
            server.base_url.clone(),
            master_key,
        ));
        server
            .state
            .lock()
            .unwrap()
            .delay_complete
            .insert("session-1".into(), Duration::from_millis(400));
        seed_uploaded_row(&bridge, &server, "814a6c78-c515-516a-834f-41af230c2b93");
        let w = fp_save(&bridge, dir.path(), "814a6c78-c515-516a-834f-41af230c2b93", "notes.txt", b"W", "1")
            .token
            .unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let (saver, save_dir, w_token, seen_at_seam) = (
            Arc::clone(&bridge),
            dir.path().to_path_buf(),
            w.clone(),
            Arc::clone(&seen),
        );
        bridge.seams.arm("landing:after_complete", move || {
            run_competing(move || {
                seen_at_seam
                    .lock()
                    .unwrap()
                    .push(held_content_version(&saver, "814a6c78-c515-516a-834f-41af230c2b93"));
                fp_save(&saver, &save_dir, "814a6c78-c515-516a-834f-41af230c2b93", "notes.txt", b"W N", &w_token);
                seen_at_seam
                    .lock()
                    .unwrap()
                    .push(held_content_version(&saver, "814a6c78-c515-516a-834f-41af230c2b93"));
            });
        });
        drain_upload_queue(&bridge, &sync_root).await;
        let state = server.finish();
        assert_eq!(
            state.init_summary(),
            vec![
                (json!("814a6c78-c515-516a-834f-41af230c2b93"), json!(1), 201),
                (json!("814a6c78-c515-516a-834f-41af230c2b93"), json!(2), 201)
            ],
            "N lands on W's produced version"
        );
        let seen = seen.lock().unwrap().clone();
        assert_eq!(seen[0], w, "at the seam: W's token");
        let n = held_content_version(&bridge, "814a6c78-c515-516a-834f-41af230c2b93");
        assert_eq!(seen[1], n, "after N's accept: N's token, kept through the landing");
        assert!(
            seen.iter()
                .chain(std::iter::once(&n))
                .all(|v| crate::write_token::parse_token(v).is_some()),
            "never a numeric content version: {seen:?}"
        );
    }

