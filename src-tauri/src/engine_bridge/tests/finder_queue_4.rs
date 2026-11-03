//! `finder_queue_4` engine tests, split out of `engine_bridge.rs`.

use super::*;
    #[cfg(unix)] // asserts a token through `held_content_version`
    #[tokio::test]
    async fn sign_out_clears_tokens_and_aliases() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [57u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        land_one_save(&bridge, &server, dir.path(), &sync_root, "9b1c3d5e-7f20-4a6b-8c9d-0e1f2a3b4c5d").await;
        bridge
            .db
            .insert_alias_for_test("provisional-1", "9b1c3d5e-7f20-4a6b-8c9d-0e1f2a3b4c5d", now_secs());
        bridge.db.purge_all_local_state().unwrap();
        assert!(
            bridge
                .db
                .item_presentation("9b1c3d5e-7f20-4a6b-8c9d-0e1f2a3b4c5d")
                .unwrap()
                .unwrap()
                .held
                .is_none()
        );
        assert_eq!(bridge.db.alias_count_for_test(), 0);
        drop(server.finish());
    }

    #[tokio::test]
    async fn sign_out_purges_provisional_rows_without_a_contract() {
        let dir = tempfile::tempdir().unwrap();
        let master_key = [58u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_uploaded_row(&bridge, &server, "6a4d439a-a6ea-5a25-bfee-3497c85bfcf2");
        let created = fp_create(&bridge, dir.path(), "never-uploaded.txt", b"local only");
        let provisional = created.outcome_file_id();
        bridge
            .db
            .insert_alias_for_test("older-provisional", "6a4d439a-a6ea-5a25-bfee-3497c85bfcf2", now_secs());
        bridge.db.purge_all_local_state().unwrap();
        assert!(
            bridge.db.get_file(&provisional).unwrap().is_none(),
            "no ghost row after sign-out"
        );
        assert!(
            bridge.db.get_file("6a4d439a-a6ea-5a25-bfee-3497c85bfcf2").unwrap().is_some(),
            "server-known rows are kept"
        );
        assert_eq!(bridge.db.alias_count_for_test(), 0);
        drop(server.finish());
    }

    #[cfg(unix)] // asserts a token through `held_content_version`
    #[tokio::test]
    async fn c1_a_save_after_the_first_landed_is_based_on_what_it_produced() {
        // The review's two passes.
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [59u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_uploaded_row(&bridge, &server, "56e3b263-b09d-5252-9784-71a28c38a13d");
        fp_save(&bridge, dir.path(), "56e3b263-b09d-5252-9784-71a28c38a13d", "notes.txt", b"A", "1");
        drain_upload_queue(&bridge, &sync_root).await;
        let base = held_content_version(&bridge, "56e3b263-b09d-5252-9784-71a28c38a13d"); // what the system holds after re-reading
        fp_save(&bridge, dir.path(), "56e3b263-b09d-5252-9784-71a28c38a13d", "notes.txt", b"A B", &base);
        drain_upload_queue(&bridge, &sync_root).await;
        let state = server.finish();
        assert_eq!(
            state.init_summary(),
            vec![(json!("56e3b263-b09d-5252-9784-71a28c38a13d"), json!(1), 201), (json!("56e3b263-b09d-5252-9784-71a28c38a13d"), json!(2), 201)]
        );
        assert_eq!(state.latest_plaintext("56e3b263-b09d-5252-9784-71a28c38a13d", master_key), b"A B");
    }

    #[tokio::test]
    async fn c1_three_saves_in_one_pass() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [60u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        server
            .state
            .lock()
            .unwrap()
            .delay_chunks
            .insert("session-1".into(), Duration::from_millis(400));
        seed_uploaded_row(&bridge, &server, "272216e1-099f-5c7a-832e-f8cdd363c9e0");
        let a = fp_save(&bridge, dir.path(), "272216e1-099f-5c7a-832e-f8cdd363c9e0", "notes.txt", b"A", "1");
        let ((), ()) = tokio::join!(
            async {
                drain_upload_queue(&bridge, &sync_root).await;
            },
            async {
                tokio::time::timeout(Duration::from_secs(10), async {
                    while server.state.lock().unwrap().inits.is_empty() {
                        tokio::time::sleep(Duration::from_millis(5)).await;
                    }
                })
                .await
                .expect("init never reached the mock");
                let b = fp_save(
                    &bridge,
                    dir.path(),
                    "272216e1-099f-5c7a-832e-f8cdd363c9e0",
                    "notes.txt",
                    b"A B",
                    a.token.as_deref().unwrap(),
                );
                fp_save(
                    &bridge,
                    dir.path(),
                    "272216e1-099f-5c7a-832e-f8cdd363c9e0",
                    "notes.txt",
                    b"A B C",
                    b.token.as_deref().unwrap(),
                );
            }
        );
        drain_upload_queue(&bridge, &sync_root).await;
        let state = server.finish();
        assert_eq!(
            state.init_summary(),
            vec![
                (json!("272216e1-099f-5c7a-832e-f8cdd363c9e0"), json!(1), 201),
                (json!("272216e1-099f-5c7a-832e-f8cdd363c9e0"), json!(2), 201),
                (json!("272216e1-099f-5c7a-832e-f8cdd363c9e0"), json!(3), 201)
            ]
        );
        assert_eq!(state.latest_plaintext("272216e1-099f-5c7a-832e-f8cdd363c9e0", master_key), b"A B C");
    }

    #[cfg(unix)] // asserts a token through `held_content_version`
    #[cfg(target_os = "macos")] // tests the restore response, which only macOS applies
    #[tokio::test]
    async fn a_restore_with_a_queued_write_runs_after_it_and_keeps_both_versions() {
        // m-9
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [61u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        // W's first attempt fails, so the runner reaches the restore while W is still
        // queued: only the content order keeps the restore behind it. Without this,
        // the runner's insertion order alone would land W first.
        server
            .state
            .lock()
            .unwrap()
            .fail_first_chunk_once
            .insert("session-1".into());
        seed_uploaded_row(&bridge, &server, "1672eade-e439-5ad0-972f-53bb8688c354");
        let w = fp_save(&bridge, dir.path(), "1672eade-e439-5ad0-972f-53bb8688c354", "notes.txt", b"W", "1");
        bridge
            .queue_restore_version("1672eade-e439-5ad0-972f-53bb8688c354", "object-ordered-v1", None)
            .unwrap();
        assert_eq!(
            held_content_version(&bridge, "1672eade-e439-5ad0-972f-53bb8688c354"),
            w.token.clone().unwrap(),
            "kept while W is queued"
        );
        drain_upload_queue(&bridge, &sync_root).await;
        let state = server.finish();
        assert_eq!(
            state.files["1672eade-e439-5ad0-972f-53bb8688c354"].versions.len(),
            3,
            "v1, W as v2, the restore as v3"
        );
        let complete_at = state
            .requests
            .iter()
            .position(|(m, p)| m == "POST" && p.ends_with("/complete"))
            .unwrap();
        let restore_at = state
            .requests
            .iter()
            .position(|(m, p)| m == "POST" && p.ends_with("/restore"))
            .unwrap();
        assert!(complete_at < restore_at, "W lands first: {:?}", state.requests);
        assert_eq!(
            state.restores,
            vec![("1672eade-e439-5ad0-972f-53bb8688c354".to_string(), "object-ordered-v1".to_string())]
        );
        assert_eq!(held_content_version(&bridge, "1672eade-e439-5ad0-972f-53bb8688c354"), "3");
    }

    /// The server's restore is not idempotent (each call mints a
    /// version, VER:534-535). A local failure after it must not send it again.
    #[cfg(target_os = "macos")] // tests the restore response, which only macOS applies
    #[tokio::test]
    async fn a_restore_the_server_made_is_never_sent_again_after_a_local_failure() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [65u8; 32];
        let server = VersionedServerMock::start();
        let db_path = dir.path().join("state.db");
        let bridge = test_bridge_with_api(&db_path, server.base_url.clone(), master_key);
        seed_uploaded_row(&bridge, &server, "421ef12f-65e1-55da-bcfa-ab09bed0b382");
        bridge
            .queue_restore_version("421ef12f-65e1-55da-bcfa-ab09bed0b382", "object-restore-once-v1", None)
            .unwrap();
        assert_eq!(
            bridge.db.peek_resnapshot_request().unwrap(),
            None,
            "no snapshot is pending before"
        );
        // The server restores; then the local record fails: the change log is gone.
        let path = db_path.clone();
        bridge.seams.arm("restore:after_server", move || {
            rusqlite::Connection::open(&path)
                .unwrap()
                .execute("ALTER TABLE fp_changes RENAME TO fp_changes_away", [])
                .unwrap();
        });
        let logs = capture_logs_async(async {
            drain_upload_queue(&bridge, &sync_root).await;
        })
        .await;
        let state = server.finish();
        assert_eq!(
            state.restores,
            vec![("421ef12f-65e1-55da-bcfa-ab09bed0b382".to_string(), "object-restore-once-v1".to_string())],
            "the restore is sent once"
        );
        assert!(
            bridge.db.list_due_operations(i64::MAX).unwrap().is_empty(),
            "the op is done, not retried"
        );
        assert!(
            bridge.db.peek_resnapshot_request().unwrap().is_some(),
            "a snapshot repairs what the reply could not record"
        );
        let lines: Vec<&str> = logs
            .lines()
            .filter(|line| line.contains("restore_bookkeeping_failed"))
            .collect();
        assert_eq!(lines.len(), 1, "one line for the failed record:\n{logs}");
        assert!(lines[0].contains("WARN") && lines[0].contains("421ef12f-65e1-55da-bcfa-ab09bed0b382"), "{logs}");
        assert!(
            !logs.contains("fp_changes"),
            "the raw error never reaches the log:\n{logs}"
        );
    }

    #[cfg(unix)] // names `arm_builder_seam`
    #[tokio::test]
    async fn a_builder_clears_held_columns_only_for_the_write_it_read() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [62u8; 32];
        let server = VersionedServerMock::start();
        let bridge = Arc::new(test_bridge_with_api(
            &dir.path().join("state.db"),
            server.base_url.clone(),
            master_key,
        ));
        land_one_save(&bridge, &server, dir.path(), &sync_root, "7c0e2f5a-1b3d-4c6e-9a80-2f4b6d8e1c37").await;
        remote_update(&bridge, &sync_root, "7c0e2f5a-1b3d-4c6e-9a80-2f4b6d8e1c37", 3, "object-elsewhere-v3");
        // Between the builder's read (W, no longer current) and its clear, a save sets held = N.
        let saver = Arc::clone(&bridge);
        let save_dir = dir.path().to_path_buf();
        let n_token = Arc::new(Mutex::new(None));
        let n_slot = Arc::clone(&n_token);
        crate::ipc_socket::arm_builder_seam(move || {
            let n = fp_save(&saver, &save_dir, "7c0e2f5a-1b3d-4c6e-9a80-2f4b6d8e1c37", "notes.txt", b"N", "3");
            *n_slot.lock().unwrap() = n.token;
        });
        let _ = held_content_version(&bridge, "7c0e2f5a-1b3d-4c6e-9a80-2f4b6d8e1c37");
        let n = n_token
            .lock()
            .unwrap()
            .clone()
            .expect("the seam fired and N was accepted");
        let held = bridge.db.item_presentation("7c0e2f5a-1b3d-4c6e-9a80-2f4b6d8e1c37").unwrap().unwrap().held;
        assert_eq!(
            held.map(|held| held.token()),
            Some(n),
            "the clear matched no row; N's token is kept"
        );
        drop(server.finish());
    }

