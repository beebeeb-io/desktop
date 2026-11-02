//! `uploads_7` engine tests, split out of `engine_bridge.rs`.

use super::*;
    #[tokio::test]
    async fn a_newer_save_waits_behind_a_live_session() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [47u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        server
            .state
            .lock()
            .unwrap()
            .delay_chunks
            .insert("session-1".into(), Duration::from_millis(400));
        seed_uploaded_row(&bridge, &server, "live");
        let first = fp_save(&bridge, dir.path(), "live", "notes.txt", b"first", "1");
        let first_token = first.token.clone().unwrap();
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
                fp_save(&bridge, dir.path(), "live", "notes.txt", b"first, second", &first_token);
            }
        );
        drain_upload_queue(&bridge, &sync_root).await;
        let state = server.finish();
        assert_eq!(
            state.init_summary(),
            vec![(json!("live"), json!(1), 201), (json!("live"), json!(2), 201)],
            "the second save is based on what the first produced"
        );
        assert_eq!(state.latest_plaintext("live", master_key), b"first, second");
    }

    #[tokio::test]
    async fn enqueue_vs_runner_a_save_accepted_while_its_predecessor_lands_is_never_orphaned() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [48u8; 32];
        let server = VersionedServerMock::start();
        // The bridge is shared with the competing thread.
        let bridge = Arc::new(test_bridge_with_api(
            &dir.path().join("state.db"),
            server.base_url.clone(),
            master_key,
        ));
        seed_uploaded_row(&bridge, &server, "race");
        let w = fp_save(&bridge, dir.path(), "race", "notes.txt", b"W", "1");
        let w_token = w.token.clone().unwrap();
        // N's accept stops at its seam; W's landing commits meanwhile (review sequence C).
        let runner = Arc::clone(&bridge);
        let root = sync_root.clone();
        bridge.seams.arm("accept:before_tx", move || {
            run_competing(move || {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap()
                    .block_on(async {
                        runner.process_due_operations(&root, now_secs()).await.unwrap();
                    });
            });
        });
        let logs = capture_logs_async(async {
            fp_save(&bridge, dir.path(), "race", "notes.txt", b"W, then N", &w_token);
            drain_upload_queue(&bridge, &sync_root).await;
        })
        .await;
        let state = server.finish();
        assert_eq!(
            state.init_summary(),
            vec![(json!("race"), json!(1), 201), (json!("race"), json!(2), 201)],
            "N is based on W's produced version"
        );
        assert_eq!(state.latest_plaintext("race", master_key), b"W, then N");
        assert!(!logs.contains("predecessor_lost"), "{logs}");
        assert!(
            bridge.db.list_due_operations(i64::MAX).unwrap().is_empty(),
            "0 ops left"
        );
    }

    #[tokio::test]
    async fn a_numeric_base_while_a_minted_write_is_queued_follows_the_newest_write() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [49u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_uploaded_row(&bridge, &server, "chain");
        let w1 = fp_save(&bridge, dir.path(), "chain", "notes.txt", b"1", "1");
        fp_save(
            &bridge,
            dir.path(),
            "chain",
            "notes.txt",
            b"1 2",
            w1.token.as_deref().unwrap(),
        );
        // The system sent this save before it recorded either reply.
        fp_save(&bridge, dir.path(), "chain", "notes.txt", b"1 2 3", "1");
        drain_upload_queue(&bridge, &sync_root).await;
        let state = server.finish();
        assert_eq!(
            state.init_summary(),
            vec![
                (json!("chain"), json!(1), 201),
                (json!("chain"), json!(2), 201),
                (json!("chain"), json!(3), 201)
            ],
            "the third save follows the newest write of the chain"
        );
        assert_eq!(state.latest_plaintext("chain", master_key), b"1 2 3");
    }

    #[tokio::test]
    async fn an_unknown_token_is_sent_as_its_b() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [50u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_uploaded_row(&bridge, &server, "unknown-token");
        server.seed_file("unknown-token", 3);
        let mut contract = bridge.db.get_file_contract_state("unknown-token").unwrap().unwrap();
        contract.current_version = 3;
        bridge.db.set_file_contract_state(&contract).unwrap();
        fp_save(
            &bridge,
            dir.path(),
            "unknown-token",
            "notes.txt",
            b"edit",
            &format!("3:w{}", "c".repeat(32)),
        );
        drain_upload_queue(&bridge, &sync_root).await;
        let state = server.finish();
        assert_eq!(state.init_summary(), vec![(json!("unknown-token"), json!(3), 201)]);
    }

    #[tokio::test]
    async fn a_waiting_op_stores_its_b_as_base_version() {
        // m-4a: an older build would send this, and the server refuses it.
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [51u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_uploaded_row(&bridge, &server, "stores-b");
        let w1 = fp_save(&bridge, dir.path(), "stores-b", "notes.txt", b"1", "1");
        fp_save(
            &bridge,
            dir.path(),
            "stores-b",
            "notes.txt",
            b"1 2",
            w1.token.as_deref().unwrap(),
        );
        let ops = bridge.db.list_operations_for_file("stores-b").unwrap();
        assert_eq!(ops[1].base_version, Some(1), "after W1, stored as W1's b");
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
        let ops = bridge.db.list_operations_for_file(&provisional).unwrap();
        assert_eq!(ops[1].base_version, Some(0), "after a create, stored as 0");
        drop(server.finish());
    }

    // ── Rule 1: every surface reports the token (spec §5.3–§5.5, §5.4 rows 5–15) ──

    #[cfg(unix)] // asserts a token through `held_content_version`
    #[tokio::test]
    async fn a_landing_keeps_the_content_version_the_reply_named() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [52u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_uploaded_row(&bridge, &server, "named");
        let reply = fp_save(&bridge, dir.path(), "named", "notes.txt", b"edit", "1");
        let token = reply.token.clone().unwrap();
        assert!(token.starts_with("1:w"), "{token}");
        assert_eq!(held_content_version(&bridge, "named"), token, "while queued");
        drain_upload_queue(&bridge, &sync_root).await;
        assert_eq!(held_content_version(&bridge, "named"), token, "after our own landing");
        drop(server.finish());
    }

    /// W landed as v2 on `file_id`; returns W's token. Used by T2, T3, T4, T5, T6, T64.
    #[cfg(unix)] // asserts a token through `held_content_version`
    pub(super) async fn land_one_save(
        bridge: &EngineBridge,
        server: &VersionedServerMock,
        dir: &Path,
        sync_root: &Path,
        file_id: &str,
    ) -> String {
        seed_uploaded_row(bridge, server, file_id);
        let token = fp_save(bridge, dir, file_id, "notes.txt", b"landed edit", "1")
            .token
            .unwrap();
        drain_upload_queue(bridge, sync_root).await;
        assert_eq!(held_content_version(bridge, file_id), token);
        token
    }

    #[cfg(unix)] // only the unix token tests call it
    pub(super) fn remote_update(bridge: &EngineBridge, sync_root: &Path, file_id: &str, version: i64, object: &str) {
        let op = crate::api_client::SyncOp {
            seq_id: 100 + version,
            op_type: "file_update".into(),
            payload: serde_json::json!({
                "id": file_id,
                "version_number": version,
                "current_object_version_id": object,
                "size_bytes": 11
            }),
        };
        apply_sync_op(bridge, sync_root, &op, now_secs(), &mut Vec::new()).unwrap();
    }

    #[cfg(unix)] // asserts a token through `held_content_version`
    #[tokio::test]
    async fn a_remote_change_replaces_the_token() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [53u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        land_one_save(&bridge, &server, dir.path(), &sync_root, "remote").await;
        remote_update(&bridge, &sync_root, "remote", 3, "object-elsewhere-v3");
        assert_eq!(held_content_version(&bridge, "remote"), "3");
        assert!(
            bridge.db.item_presentation("remote").unwrap().unwrap().held.is_none(),
            "the builder cleared it"
        );

        // A new version under the SAME object id: the legacy one-shot `file_update` carries no
        // `current_object_version_id`, so the row keeps its own (EB:6169-6174).
        land_one_save(&bridge, &server, dir.path(), &sync_root, "remote-same-object").await;
        let replace = crate::api_client::SyncOp {
            seq_id: 200,
            op_type: "file_update".into(),
            payload: serde_json::json!({ "id": "remote-same-object", "version_number": 3, "size_bytes": 11 }),
        };
        apply_sync_op(&bridge, &sync_root, &replace, now_secs(), &mut Vec::new()).unwrap();
        assert_eq!(held_content_version(&bridge, "remote-same-object"), "3");
        drop(server.finish());
    }

    #[cfg(unix)] // asserts a token through `held_content_version`
    #[tokio::test]
    async fn our_own_echo_keeps_the_token() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [54u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        let token = land_one_save(&bridge, &server, dir.path(), &sync_root, "echo").await;
        remote_update(&bridge, &sync_root, "echo", 2, "object-echo-v2");
        assert_eq!(
            held_content_version(&bridge, "echo"),
            token,
            "the echo of our own landing changes nothing"
        );
        drop(server.finish());
    }

    #[cfg(unix)] // asserts a token through `held_content_version`
    #[cfg(target_os = "macos")] // tests the restore response, which only macOS applies
    #[tokio::test]
    async fn a_restore_from_this_mac_replaces_the_token() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [55u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        land_one_save(&bridge, &server, dir.path(), &sync_root, "restored").await;
        bridge
            .queue_restore_version("restored", "object-restored-v1", None)
            .unwrap();
        drain_upload_queue(&bridge, &sync_root).await;
        assert_eq!(
            held_content_version(&bridge, "restored"),
            "3",
            "the restore's version, so the system re-downloads"
        );
        drop(server.finish());
    }

