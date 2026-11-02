//! `refusals` engine tests, split out of `engine_bridge.rs`.

use super::*;
    #[tokio::test]
    async fn a_lost_reply_base_parks_with_its_bytes() {
        // Review Focus 4: the shipped fallback of the split rule 1c.
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [71u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_uploaded_row(&bridge, &server, "lost-reply");
        let a = fp_save(&bridge, dir.path(), "lost-reply", "notes.txt", b"A", "1");
        drain_upload_queue(&bridge, &sync_root).await;
        let a_token = a.token.unwrap();
        let _b_reply_lost = fp_save(&bridge, dir.path(), "lost-reply", "notes.txt", b"A B", &a_token);
        fp_save(&bridge, dir.path(), "lost-reply", "notes.txt", b"A B C", &a_token); // still A's token
        let logs = capture_logs_async(async {
            drain_upload_queue(&bridge, &sync_root).await;
        })
        .await;
        let state = server.finish();
        assert_eq!(
            state.init_summary(),
            vec![
                (json!("lost-reply"), json!(1), 201),
                (json!("lost-reply"), json!(2), 201),
                (json!("lost-reply"), json!(1), 409)
            ]
        );
        assert_eq!(state.latest_plaintext("lost-reply", master_key), b"A B");
        let parked = bridge.db.list_operations_for_file("lost-reply").unwrap().remove(0);
        assert_eq!(parked.attempts, parked.max_attempts, "a visible park");
        assert_eq!(
            std::fs::read(parked.payload_path.as_deref().unwrap()).unwrap(),
            b"A B C",
            "its bytes are kept"
        );
        assert!(
            logs.lines()
                .any(|l| l.contains("upload refused") && l.contains("parked")),
            "{logs}"
        );
    }

    #[tokio::test]
    async fn a_parked_predecessor_with_a_recorded_completion_is_waited_for_never_taken_over() {
        // Spec §8.4: "W has a recorded completion: W never parks (§8.6 rule 6). N waits,
        // and the chain step resolves it when W's local landing succeeds." W is set up parked
        // with a recorded completion, a state no park path produces, so the claim's own guard is tested.
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [72u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_uploaded_row(&bridge, &server, "completed");
        let w = fp_save(&bridge, dir.path(), "completed", "notes.txt", b"W", "1");
        fp_save(
            &bridge,
            dir.path(),
            "completed",
            "notes.txt",
            b"W N",
            w.token.as_deref().unwrap(),
        );
        let ops = bridge.db.list_operations_for_file("completed").unwrap();
        let (w_op, n_op) = (ops[0].clone(), ops[1].clone());
        bridge
            .db
            .put_upload_resume(&UploadResume {
                op_id: w_op.op_id.clone(),
                payload_path: w_op.payload_path.clone().unwrap(),
                payload_size: 1,
                payload_mtime_ns: 1,
                upload_session_id: "session-w".into(),
                server_file_id: "completed".into(),
                object_version_id: "object-w".into(),
                chunk_size_bytes: 1,
                chunk_count: 1,
                acked_chunks: 1,
                metadata_applied: true,
                is_create: false,
                self_minted: false,
                completed_version: None,
                completed_object_version_id: None,
                completed_mime_type: None,
            })
            .unwrap();
        bridge.db.set_completed_for_test(&w_op.op_id, 2);
        bridge.db.park_for_test(&w_op.op_id);
        let logs = capture_logs_async(async {
            bridge.process_due_operations(&sync_root, now_secs()).await.unwrap();
        })
        .await;
        let state = server.finish();
        assert!(
            state.inits.is_empty(),
            "nothing is sent while W's landing is pending: {:?}",
            state.init_summary()
        );
        let ops = bridge.db.list_operations_for_file("completed").unwrap();
        assert_eq!(ops.len(), 2, "W is never taken over: both ops remain");
        assert_eq!(ops[1].op_id, n_op.op_id);
        assert_eq!(ops[1].attempts, 0, "N waits, and waiting is not an attempt");
        assert_eq!(
            bridge.db.finder_write(&n_op.op_id).unwrap().unwrap().after_write_id,
            bridge.db.finder_write(&w_op.op_id).unwrap().map(|write| write.write_id),
            "N still waits after W"
        );
        assert!(!logs.contains("took over"), "{logs}");
        assert!(std::path::Path::new(w_op.payload_path.as_deref().unwrap()).is_file());
    }

    #[tokio::test]
    async fn a_hand_over_that_inherits_an_earlier_builds_predecessor_parks_and_releases_the_copy() {
        // X (an earlier build's op, parked) <- W <- N. W parks behind X at its claim (m-2); at
        // N's claim N takes W's role, inherits X, and parks too. The hand-over is still
        // logged, and W's copy is unlinked after the commit (S6).
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [73u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_uploaded_row(&bridge, &server, "chain-park");
        let x = fp_save(&bridge, dir.path(), "chain-park", "notes.txt", b"X", "1");
        let x_op = bridge.db.list_operations_for_file("chain-park").unwrap().remove(0);
        bridge
            .db
            .set_write_origin_for_test(&x_op.op_id, crate::state_db::WriteOrigin::EarlierBuild);
        bridge.db.park_for_test(&x_op.op_id);
        let w = fp_save(
            &bridge,
            dir.path(),
            "chain-park",
            "notes.txt",
            b"X W",
            x.token.as_deref().unwrap(),
        );
        fp_save(
            &bridge,
            dir.path(),
            "chain-park",
            "notes.txt",
            b"X W N",
            w.token.as_deref().unwrap(),
        );
        let ops = bridge.db.list_operations_for_file("chain-park").unwrap();
        let (w_op, n_op) = (ops[1].clone(), ops[2].clone());
        let logs = capture_logs_async(async {
            bridge.process_due_operations(&sync_root, now_secs()).await.unwrap();
        })
        .await;
        let state = server.finish();
        assert!(state.inits.is_empty(), "nothing is sent: {:?}", state.init_summary());
        let ops = bridge.db.list_operations_for_file("chain-park").unwrap();
        assert_eq!(
            ops.iter().map(|op| op.op_id.clone()).collect::<Vec<_>>(),
            vec![x_op.op_id.clone(), n_op.op_id.clone()],
            "W's op is gone; X and N remain"
        );
        assert!(
            ops.iter().all(|op| op.attempts == op.max_attempts),
            "both parked with their bytes"
        );
        assert_eq!(
            std::fs::read(ops[1].payload_path.as_deref().unwrap()).unwrap(),
            b"X W N"
        );
        assert_eq!(logs.matches("queued write took over a parked one").count(), 1, "{logs}");
        assert!(
            !std::path::Path::new(w_op.payload_path.as_deref().unwrap()).exists(),
            "W's copy is released after the commit"
        );
        assert!(
            logs.lines()
                .any(|line| line.contains("predecessor_parked") && line.contains(&n_op.op_id)),
            "{logs}"
        );
    }

    #[tokio::test]
    async fn a_base_unknown_predecessor_hands_over_and_the_successor_parks_without_a_request() {
        // W parked at accept under rule 6a′, because its base cannot be known.
        // N, saved on W's token, takes W's role at its claim and parks `base_unknown` there:
        // no request goes out with a base the client knows is unknown.
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [74u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_uploaded_row(&bridge, &server, "unknown-base"); // version 1, its version not filled
        let w = fp_save(&bridge, dir.path(), "unknown-base", "notes.txt", b"W", "0");
        let w_op = bridge.db.list_operations_for_file("unknown-base").unwrap().remove(0);
        assert_eq!(w_op.attempts, w_op.max_attempts, "rule 6a′ parked W at accept");
        fp_save(
            &bridge,
            dir.path(),
            "unknown-base",
            "notes.txt",
            b"W N",
            w.token.as_deref().unwrap(),
        );
        let n_op = bridge.db.list_operations_for_file("unknown-base").unwrap().remove(1);
        let logs = capture_logs_async(async {
            bridge.process_due_operations(&sync_root, now_secs()).await.unwrap();
        })
        .await;
        let state = server.finish();
        assert_eq!(
            state.requests.len(),
            0,
            "no request with a base the client knows is unknown: {:?}",
            state.init_summary()
        );
        let ops = bridge.db.list_operations_for_file("unknown-base").unwrap();
        assert_eq!(ops.len(), 1, "W's op is gone; N remains");
        assert_eq!(ops[0].op_id, n_op.op_id);
        assert_eq!(ops[0].attempts, ops[0].max_attempts, "N parked at its claim");
        assert_eq!(ops[0].last_error.as_deref(), Some("base_unknown"));
        assert_eq!(
            std::fs::read(ops[0].payload_path.as_deref().unwrap()).unwrap(),
            b"W N",
            "its bytes are kept"
        );
        assert!(
            !std::path::Path::new(w_op.payload_path.as_deref().unwrap()).exists(),
            "W's copy is released after the commit"
        );
        assert_eq!(logs.matches("queued write took over a parked one").count(), 1, "{logs}");
        assert_eq!(
            logs.lines()
                .filter(|line| line.contains("upload parked")
                    && line.contains(&n_op.op_id)
                    && line.contains("base_unknown"))
                .count(),
            1,
            "{logs}"
        );
    }

    #[tokio::test]
    async fn a_parked_create_hands_over_and_the_save_lands_as_the_create() {
        // The create branch of the hand-over (spec §8.4): a Finder create W whose staged copy
        // is gone parks; the save N on W's token takes W's create at its claim and lands as
        // the new file's first version, with N's bytes (they contain W's).
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [75u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        let created = fp_create(&bridge, dir.path(), "made.txt", b"created");
        let provisional = created.outcome_file_id();
        let w_op = bridge.db.list_operations_for_file(&provisional).unwrap().remove(0);
        std::fs::remove_file(w_op.payload_path.as_deref().unwrap()).unwrap();
        fp_save(
            &bridge,
            dir.path(),
            &provisional,
            "made.txt",
            b"created, edited",
            created.token.as_deref().unwrap(),
        );
        let logs = capture_logs_async(async {
            drain_upload_queue(&bridge, &sync_root).await;
        })
        .await;
        let state = server.finish();
        assert_eq!(
            state.init_summary(),
            vec![(json!(provisional), json!(null), 201)],
            "one create, under the id this device chose, without a base"
        );
        let server_id = {
            let files = state.files_with_content();
            assert_eq!(files.len(), 1, "exactly one file on the server: {files:?}");
            files[0].clone()
        };
        assert_eq!(
            server_id, provisional,
            "the create's own id is the server id: the server never mints one"
        );
        assert_eq!(state.files[&server_id].versions.len(), 1, "N's bytes are version 1");
        assert_eq!(state.latest_plaintext(&server_id, master_key), b"created, edited");
        let rows = bridge.db.list_files().unwrap();
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0].file_id, server_id, "the create's own id");
        assert_eq!(rows[0].status, FileStatus::Local);
        assert!(bridge.db.list_operations_for_file(&provisional).unwrap().is_empty());
        assert!(bridge.db.list_operations_for_file(&server_id).unwrap().is_empty());
        assert_eq!(logs.matches("queued write took over a parked one").count(), 1, "{logs}");
    }

    #[tokio::test]
    async fn a_hand_over_that_inherits_a_pending_base_waits_and_releases_the_copy() {
        // `ClaimOutcome::WaitAfterHandOver`. Not reachable in production today: nothing parks an
        // op whose base is pending. W is parked by hand here. N takes W's role, inherits the
        // pending base, and waits: the hand-over is still logged once and W's copy unlinked.
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [76u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        bridge
            .db
            .upsert_file(&FileEntry {
                file_id: "versionless".into(),
                path: "notes.txt".into(),
                status: FileStatus::Local,
                size_bytes: 10,
                modified_at: 1_700_000_000,
                content_hash: None,
                remote_updated_at: 1_700_000_000,
                parent_id: None,
                item_kind: ItemKind::File,
            })
            .unwrap();
        assert_eq!(
            bridge
                .db
                .get_file_contract_state("versionless")
                .unwrap()
                .unwrap()
                .current_version,
            0
        );
        let w = fp_save(&bridge, dir.path(), "versionless", "notes.txt", b"W", "0"); // rule 6b
        let w_op = bridge.db.list_operations_for_file("versionless").unwrap().remove(0);
        assert_eq!(bridge.db.finder_write(&w_op.op_id).unwrap().unwrap().base_pending, 1);
        fp_save(
            &bridge,
            dir.path(),
            "versionless",
            "notes.txt",
            b"W N",
            w.token.as_deref().unwrap(),
        );
        let n_op = bridge.db.list_operations_for_file("versionless").unwrap().remove(1);
        bridge.db.park_for_test(&w_op.op_id);
        let logs = capture_logs_async(async {
            bridge.process_due_operations(&sync_root, now_secs()).await.unwrap();
        })
        .await;
        let state = server.finish();
        assert!(state.requests.is_empty(), "nothing is sent: {:?}", state.requests);
        let ops = bridge.db.list_operations_for_file("versionless").unwrap();
        assert_eq!(ops.len(), 1, "W's op is gone; N remains");
        assert_eq!(ops[0].op_id, n_op.op_id);
        assert_eq!(ops[0].attempts, 0, "N waits, and waiting is not an attempt");
        let n_write = bridge.db.finder_write(&n_op.op_id).unwrap().unwrap();
        assert_eq!(
            (n_write.base_pending, n_write.after_write_id),
            (1, None),
            "N inherited W's pending base"
        );
        assert_eq!(logs.matches("queued write took over a parked one").count(), 1, "{logs}");
        assert!(
            !std::path::Path::new(w_op.payload_path.as_deref().unwrap()).exists(),
            "W's copy is released after the commit"
        );
        assert!(!logs.contains("upload parked"), "{logs}");
    }

    // ── Every committed park and hand-over reaches the tick's working-set signal (spec §9.1) ──
    //
    // A parked upload presents `error`, read-only. The runner's one signal per tick is built
    // from `invalidated_item_ids` (and the tick's remote applies), so a park missing from it
    // reached Finder only at the next unrelated signal: on a Mac, minutes during which the
    // person could keep editing a file whose saves would not upload.

    #[tokio::test]
    async fn a_stale_base_park_puts_its_item_in_the_ticks_signal() {
        let (dir, sync_root, server, bridge) = rule3_setup([120u8; 32]);
        seed_uploaded_row(&bridge, &server, "stale-signal");
        server.seed_file("stale-signal", 2); // the server moved on
        fp_save(&bridge, dir.path(), "stale-signal", "notes.txt", b"edit on v1", "1");

        let outcome = bridge.process_due_operations(&sync_root, now_secs()).await.unwrap();

        let op = bridge.db.list_operations_for_file("stale-signal").unwrap().remove(0);
        assert_eq!(op.attempts, op.max_attempts, "precondition: parked");
        assert_eq!(
            op.last_error.as_deref(),
            Some("stale_base"),
            "precondition: the 409 park"
        );
        assert_eq!(
            bridge.db.get_file("stale-signal").unwrap().unwrap().status,
            FileStatus::Error,
            "the park settled the row"
        );
        assert_eq!(outcome.invalidated_item_ids, vec!["stale-signal".to_string()]);
        drop(server.finish());
    }

    #[tokio::test]
    async fn a_park_before_any_request_puts_its_item_in_the_ticks_signal() {
        // `ParkNow`: the `init` guard (§6.3.1) never sends a replace of a known file without a base.
        let (dir, sync_root, server, bridge) = rule3_setup([121u8; 32]);
        seed_uploaded_row(&bridge, &server, "no-base-signal");
        fp_save(&bridge, dir.path(), "no-base-signal", "notes.txt", b"a save", "1");
        let op_id = bridge
            .db
            .list_operations_for_file("no-base-signal")
            .unwrap()
            .remove(0)
            .op_id;
        bridge.db.set_base_version_for_test(&op_id, None);

        let outcome = bridge.process_due_operations(&sync_root, now_secs()).await.unwrap();

        let op = bridge.db.get_operation(&op_id).unwrap().unwrap();
        assert_eq!(op.attempts, op.max_attempts, "precondition: parked");
        assert_eq!(op.last_error.as_deref(), Some("base_unknown"));
        assert_eq!(
            bridge.db.get_file("no-base-signal").unwrap().unwrap().status,
            FileStatus::Error
        );
        assert_eq!(outcome.invalidated_item_ids, vec!["no-base-signal".to_string()]);
        let state = server.finish();
        assert!(state.inits.is_empty(), "nothing is sent: {:?}", state.init_summary());
    }

    #[tokio::test]
    async fn a_claim_time_park_puts_its_item_in_the_ticks_signal() {
        // X, an earlier build's op, is parked; N, saved on X's token, parks at its claim without
        // a hand-over, because an earlier build's bytes may not be contained in a later save's.
        let (dir, sync_root, server, bridge) = rule3_setup([122u8; 32]);
        seed_uploaded_row(&bridge, &server, "claim-park-signal");
        let x = fp_save(&bridge, dir.path(), "claim-park-signal", "notes.txt", b"X", "1");
        let x_op = bridge
            .db
            .list_operations_for_file("claim-park-signal")
            .unwrap()
            .remove(0);
        bridge
            .db
            .set_write_origin_for_test(&x_op.op_id, crate::state_db::WriteOrigin::EarlierBuild);
        bridge.db.park_for_test(&x_op.op_id);
        fp_save(
            &bridge,
            dir.path(),
            "claim-park-signal",
            "notes.txt",
            b"X N",
            x.token.as_deref().unwrap(),
        );

        let outcome = bridge.process_due_operations(&sync_root, now_secs()).await.unwrap();

        let n = bridge
            .db
            .list_operations_for_file("claim-park-signal")
            .unwrap()
            .remove(1);
        assert_eq!(n.attempts, n.max_attempts, "precondition: N parked at its claim");
        assert_eq!(n.last_error.as_deref(), Some("predecessor_parked"));
        assert_eq!(
            bridge.db.get_file("claim-park-signal").unwrap().unwrap().status,
            FileStatus::Error
        );
        assert_eq!(outcome.invalidated_item_ids, vec!["claim-park-signal".to_string()]);
        let state = server.finish();
        assert!(state.inits.is_empty(), "nothing is sent: {:?}", state.init_summary());
    }

    #[tokio::test]
    async fn a_hand_over_whose_successor_waits_puts_its_item_in_the_ticks_signal() {
        // `ClaimOutcome::WaitAfterHandOver`, the fixture of
        // `a_hand_over_that_inherits_a_pending_base_waits_and_releases_the_copy`.
        let (dir, sync_root, server, bridge) = rule3_setup([123u8; 32]);
        bridge
            .db
            .upsert_file(&FileEntry {
                file_id: "versionless-signal".into(),
                path: "notes.txt".into(),
                status: FileStatus::Local,
                size_bytes: 10,
                modified_at: 1_700_000_000,
                content_hash: None,
                remote_updated_at: 1_700_000_000,
                parent_id: None,
                item_kind: ItemKind::File,
            })
            .unwrap();
        let w = fp_save(&bridge, dir.path(), "versionless-signal", "notes.txt", b"W", "0"); // rule 6b
        fp_save(
            &bridge,
            dir.path(),
            "versionless-signal",
            "notes.txt",
            b"W N",
            w.token.as_deref().unwrap(),
        );
        let w_op = bridge
            .db
            .list_operations_for_file("versionless-signal")
            .unwrap()
            .remove(0);
        bridge.db.park_for_test(&w_op.op_id);

        let mut outcome = None;
        let logs = capture_logs_async(async {
            outcome = Some(bridge.process_due_operations(&sync_root, now_secs()).await.unwrap());
        })
        .await;

        assert_eq!(logs.matches("queued write took over a parked one").count(), 1, "{logs}");
        let ops = bridge.db.list_operations_for_file("versionless-signal").unwrap();
        assert_eq!(ops.len(), 1, "precondition: W's op is gone");
        assert_eq!(ops[0].attempts, 0, "precondition: N waits");
        assert_eq!(
            outcome.unwrap().invalidated_item_ids,
            vec!["versionless-signal".to_string()]
        );
        let state = server.finish();
        assert!(state.requests.is_empty(), "nothing is sent: {:?}", state.requests);
    }

