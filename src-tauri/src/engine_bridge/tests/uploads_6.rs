//! `uploads_6` engine tests, split out of `engine_bridge.rs`.

use super::*;
    #[test]
    fn the_409_messages_are_pinned() {
        use crate::api_client::{InitConflictClass, classify_init_conflict};
        assert_eq!(
            classify_init_conflict("stale base version for replacement upload"),
            InitConflictClass::StaleBase
        );
        assert_eq!(
            classify_init_conflict("upload is already in progress for this file"),
            InitConflictClass::InProgress
        );
        assert_eq!(classify_init_conflict("file is in trash"), InitConflictClass::Other);
        assert_eq!(
            classify_init_conflict(""),
            InitConflictClass::Other,
            "a changed message falls back to retry"
        );
    }

    #[tokio::test]
    async fn a_stale_base_409_parks_at_once() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [65u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_uploaded_row(&bridge, &server, "stale");
        server.seed_file("stale", 2); // the server moved on
        fp_save(&bridge, dir.path(), "stale", "notes.txt", b"edit on v1", "1");
        let logs = capture_logs_async(async {
            bridge.process_due_operations(&sync_root, now_secs()).await.unwrap();
        })
        .await;
        let op = bridge.db.list_operations_for_file("stale").unwrap().remove(0);
        assert_eq!(op.attempts, op.max_attempts, "parked after one attempt");
        let refused: Vec<&str> = logs.lines().filter(|l| l.contains("upload refused")).collect();
        assert_eq!(refused.len(), 1, "{logs}");
        assert!(
            refused[0].contains("stale_base") && refused[0].contains("parked"),
            "{logs}"
        );
        // The runner words this park by its reason and logs the attempt that was made, not the
        // cap: an immediate park never says its attempts were used up.
        assert!(
            refused[0].contains("its base is stale")
                && !refused[0].contains("attempts used up")
                && refused[0].contains("attempt=1 "),
            "{logs}"
        );
        drop(server.finish());
    }

    #[tokio::test]
    async fn an_in_progress_409_is_retried() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [66u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_uploaded_row(&bridge, &server, "busy");
        server
            .state
            .lock()
            .unwrap()
            .conflict_init_once
            .insert("busy".into(), crate::api_client::IN_PROGRESS_MESSAGE.into());
        fp_save(&bridge, dir.path(), "busy", "notes.txt", b"edit", "1");
        drain_upload_queue(&bridge, &sync_root).await;
        let state = server.finish();
        assert_eq!(
            state.init_summary(),
            vec![(json!("busy"), json!(1), 409), (json!("busy"), json!(1), 201)]
        );
        assert_eq!(state.latest_plaintext("busy", master_key), b"edit");
    }

    #[tokio::test]
    async fn i4_a_doomed_earlier_write_never_blocks_hours() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [67u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_uploaded_row(&bridge, &server, "doomed");
        let w = fp_save(&bridge, dir.path(), "doomed", "notes.txt", b"W", "1");
        fp_save(
            &bridge,
            dir.path(),
            "doomed",
            "notes.txt",
            b"W N",
            w.token.as_deref().unwrap(),
        );
        let w_op = bridge.db.list_operations_for_file("doomed").unwrap().remove(0);
        std::fs::remove_file(w_op.payload_path.as_deref().unwrap()).unwrap();
        let (passes, logs) = {
            let mut passes = 0;
            let logs = capture_logs_async(async {
                passes = drain_upload_queue(&bridge, &sync_root).await;
            })
            .await;
            (passes, logs)
        };
        let state = server.finish();
        assert!(
            passes <= 2,
            "the successor lands in the same or the next pass: {passes}"
        );
        assert_eq!(
            state.init_summary(),
            vec![(json!("doomed"), json!(1), 201)],
            "N took W's base"
        );
        assert_eq!(state.latest_plaintext("doomed", master_key), b"W N");
        assert_eq!(logs.matches("queued write took over a parked one").count(), 1, "{logs}");
        assert!(bridge.db.list_operations_for_file("doomed").unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_stale_base_predecessor_hands_over_and_the_successor_parks_with_the_newest_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [68u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_uploaded_row(&bridge, &server, "stale-chain");
        server.seed_file("stale-chain", 2);
        let w = fp_save(&bridge, dir.path(), "stale-chain", "notes.txt", b"W", "1");
        fp_save(
            &bridge,
            dir.path(),
            "stale-chain",
            "notes.txt",
            b"W N",
            w.token.as_deref().unwrap(),
        );
        let logs = capture_logs_async(async {
            drain_upload_queue(&bridge, &sync_root).await;
        })
        .await;
        let state = server.finish();
        assert_eq!(
            state.init_summary(),
            vec![
                (json!("stale-chain"), json!(1), 409),
                (json!("stale-chain"), json!(1), 409)
            ],
            "W and then N, on W's base"
        );
        let ops = bridge.db.list_operations_for_file("stale-chain").unwrap();
        assert_eq!(ops.len(), 1, "W's op is gone; one parked op remains");
        assert_eq!(ops[0].attempts, ops[0].max_attempts);
        assert_eq!(
            std::fs::read(ops[0].payload_path.as_deref().unwrap()).unwrap(),
            b"W N",
            "the newest bytes are kept"
        );
        assert_eq!(logs.matches("queued write took over a parked one").count(), 1, "{logs}");
    }

    #[tokio::test]
    async fn an_earlier_builds_op_is_never_handed_over() {
        // m-2: an earlier build's bytes may not be contained in a newer save's (spec §3).
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [69u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        // (a) an earlier-build op and a newer save on "1": both sets of bytes kept.
        seed_uploaded_row(&bridge, &server, "earlier");
        fp_save(
            &bridge,
            dir.path(),
            "earlier",
            "notes.txt",
            b"earlier build's bytes",
            "1",
        );
        let e = bridge.db.list_operations_for_file("earlier").unwrap().remove(0);
        bridge
            .db
            .set_write_origin_for_test(&e.op_id, crate::state_db::WriteOrigin::EarlierBuild);
        fp_save(&bridge, dir.path(), "earlier", "notes.txt", b"a newer save", "1");
        // (b) a provisional row whose earlier-build create parks with a save queued after it.
        let created = fp_create(&bridge, dir.path(), "made-earlier.txt", b"created");
        let provisional = created.outcome_file_id();
        fp_save(
            &bridge,
            dir.path(),
            &provisional,
            "made-earlier.txt",
            b"created, edited",
            created.token.as_deref().unwrap(),
        );
        let c = bridge.db.list_operations_for_file(&provisional).unwrap().remove(0);
        bridge
            .db
            .set_write_origin_for_test(&c.op_id, crate::state_db::WriteOrigin::EarlierBuild);
        bridge.db.park_for_test(&c.op_id);
        let logs = capture_logs_async(async {
            drain_upload_queue(&bridge, &sync_root).await;
        })
        .await;
        let state = server.finish();
        assert_eq!(
            state.latest_plaintext("earlier", master_key),
            b"earlier build's bytes",
            "(a) E landed"
        );
        let newer = bridge.db.list_operations_for_file("earlier").unwrap().remove(0);
        assert_eq!(
            newer.attempts, newer.max_attempts,
            "(a) the newer save parked, not dropped"
        );
        assert!(std::path::Path::new(newer.payload_path.as_deref().unwrap()).is_file());
        let ops = bridge.db.list_operations_for_file(&provisional).unwrap();
        assert_eq!(ops.len(), 2, "(b) no hand-over: C and N both remain");
        assert!(ops.iter().all(|op| op.attempts == op.max_attempts));
        assert!(logs.contains("predecessor_parked"), "{logs}");
        assert!(!logs.contains("took over"), "{logs}");
        assert!(
            state
                .inits
                .iter()
                .all(|(body, _)| body["file_id"] != json!(provisional)),
            "(b) nothing uploaded"
        );
    }

    #[tokio::test]
    async fn handover_vs_new_save_the_newest_bytes_land_last() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [70u8; 32];
        let server = VersionedServerMock::start();
        let bridge = Arc::new(test_bridge_with_api(
            &dir.path().join("state.db"),
            server.base_url.clone(),
            master_key,
        ));
        seed_uploaded_row(&bridge, &server, "handover");
        let w = fp_save(&bridge, dir.path(), "handover", "notes.txt", b"W", "1");
        let w_op = bridge.db.list_operations_for_file("handover").unwrap().remove(0);
        std::fs::remove_file(w_op.payload_path.as_deref().unwrap()).unwrap();
        bridge.process_due_operations(&sync_root, now_secs()).await.unwrap(); // W parks payload_missing
        let n = fp_save(
            &bridge,
            dir.path(),
            "handover",
            "notes.txt",
            b"W N",
            w.token.as_deref().unwrap(),
        );
        // At N's claim, a newer save N2 is accepted on N's token.
        let saver = Arc::clone(&bridge);
        let save_dir = dir.path().to_path_buf();
        let n_token = n.token.clone().unwrap();
        let fired = Arc::new(AtomicBool::new(false));
        let fired_in_seam = Arc::clone(&fired);
        bridge.seams.arm("claim:before_tx", move || {
            run_competing(move || {
                fp_save(&saver, &save_dir, "handover", "notes.txt", b"W N N2", &n_token);
            });
            fired_in_seam.store(true, Ordering::SeqCst);
        });
        let logs = capture_logs_async(async {
            drain_upload_queue(&bridge, &sync_root).await;
        })
        .await;
        let state = server.finish();
        assert!(fired.load(Ordering::SeqCst), "the seam fired and N2 was accepted");
        assert_eq!(
            state.init_summary(),
            vec![(json!("handover"), json!(1), 201), (json!("handover"), json!(2), 201)],
            "N on W's base, then N2"
        );
        assert_eq!(
            state.latest_plaintext("handover", master_key),
            b"W N N2",
            "the newest bytes land last"
        );
        assert_eq!(logs.matches("queued write took over a parked one").count(), 1, "{logs}");
    }

