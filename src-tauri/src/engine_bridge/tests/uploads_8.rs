//! `uploads_8` engine tests, split out of `engine_bridge.rs`.

use super::*;
    #[tokio::test]
    async fn a_completed_session_is_never_abandoned_at_give_up() {
        // m-11
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [82u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        let created = fp_create(&bridge, dir.path(), "completed.txt", b"bytes the server has");
        let provisional = created.outcome_file_id();
        crate::state_db::fail_next_landings_for_test(30);
        let mut now = now_secs();
        let logs = capture_logs_async(async {
            for _ in 0..26 {
                bridge.process_due_operations(&sync_root, now).await.unwrap();
                now += 10_000;
            }
        })
        .await;
        // Never trashed first: with the attempts cap gone, the give-up guard alone keeps it.
        assert!(
            server.state.lock().unwrap().trashes.is_empty(),
            "the completed server file is never trashed"
        );
        assert_eq!(server.state.lock().unwrap().inits.len(), 1, "no second upload");
        let op = bridge.db.list_operations_for_file(&provisional).unwrap().remove(0);
        assert!(op.attempts < op.max_attempts, "never parked");
        assert!(
            bridge
                .db
                .get_upload_resume(&op.op_id)
                .unwrap()
                .unwrap()
                .completed_version
                .is_some(),
            "the resume row is kept"
        );
        assert!(logs.matches("local landing will be retried").count() >= 25, "{logs}");
        crate::state_db::fail_next_landings_for_test(0);
        bridge.process_due_operations(&sync_root, now).await.unwrap();
        assert!(
            bridge.db.get_file(&provisional).unwrap().is_some(),
            "landed once the disk recovers, under the id the create chose"
        );
        drop(server.finish());
    }

    #[tokio::test]
    async fn a_recorded_completion_is_applied_without_the_network() {
        // §8.6 rule 4
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [83u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_uploaded_row(&bridge, &server, "recorded");
        fp_save(&bridge, dir.path(), "recorded", "notes.txt", b"edit", "1");
        crate::state_db::fail_next_landings_for_test(1);
        let now = now_secs();
        bridge.process_due_operations(&sync_root, now).await.unwrap();
        let before = server.state.lock().unwrap().requests.len();
        bridge.process_due_operations(&sync_root, now + 10_000).await.unwrap();
        let state = server.finish();
        assert_eq!(
            state.requests.len(),
            before,
            "the retry asked the server nothing: {:?}",
            &state.requests[before..]
        );
        assert!(
            bridge.db.list_operations_for_file("recorded").unwrap().is_empty(),
            "landed"
        );
        let contract = bridge.db.get_file_contract_state("recorded").unwrap().unwrap();
        assert_eq!(contract.current_version, 2);
        assert_eq!(
            contract.content_type.as_deref(),
            Some("text/plain"),
            "the retried landing keeps the save's content type"
        );
    }

    #[cfg(unix)] // asserts a token through `held_content_version`
    #[tokio::test]
    async fn the_create_landing_writes_the_alias_in_its_transaction() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [84u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        let created = fp_create(&bridge, dir.path(), "aliased.txt", b"created");
        let provisional = created.outcome_file_id();
        drain_upload_queue(&bridge, &sync_root).await;
        let state = server.finish();
        let server_id = state.files_with_content()[0].clone();
        // SF-0001 (L13): the create sent its own id, so the server file is that id and the
        // landing needs no alias to reach it.
        assert_eq!(server_id, provisional, "the server file carries the id this device chose");
        assert_eq!(bridge.db.alias_target_for_test(&provisional), None, "no swap, no alias");
        assert!(bridge.db.get_file(&provisional).unwrap().is_some());
        assert_eq!(
            held_content_version(&bridge, &server_id),
            created.token.unwrap(),
            "the landed row carries P's create token (§5.4 row 14)"
        );
    }

    /// A save N is accepted on W's token after W's upload landed and
    /// before the runner records W's outcome (`outcome:before_tx`, after the thumbnail
    /// work). The landing transaction has already removed W's op, so N decides against
    /// the landed W (rule 1b) and never waits on a write that no op carries.
    #[tokio::test]
    async fn a_save_accepted_before_the_landed_outcome_is_recorded_lands_on_the_produced_version() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [85u8; 32];
        let server = VersionedServerMock::start();
        let bridge = Arc::new(test_bridge_with_api(
            &dir.path().join("state.db"),
            server.base_url.clone(),
            master_key,
        ));
        seed_uploaded_row(&bridge, &server, "window");
        let w = fp_save(&bridge, dir.path(), "window", "notes.txt", b"W", "1")
            .token
            .unwrap();
        let (saver, save_dir) = (Arc::clone(&bridge), dir.path().to_path_buf());
        let fired = Arc::new(AtomicBool::new(false));
        let fired_in_seam = Arc::clone(&fired);
        bridge.seams.arm("outcome:before_tx", move || {
            run_competing(move || {
                fp_save(&saver, &save_dir, "window", "notes.txt", b"W N", &w);
            });
            fired_in_seam.store(true, Ordering::SeqCst);
        });
        let logs = capture_logs_async(async {
            drain_upload_queue(&bridge, &sync_root).await;
        })
        .await;
        let state = server.finish();
        assert!(fired.load(Ordering::SeqCst), "the seam fired and N was accepted");
        assert_eq!(
            state.init_summary(),
            vec![(json!("window"), json!(1), 201), (json!("window"), json!(2), 201)],
            "N lands on W's produced version: {logs}"
        );
        assert_eq!(state.latest_plaintext("window", master_key), b"W N");
        assert!(!logs.contains("predecessor_lost"), "{logs}");
        assert!(
            bridge.db.list_operations_for_file("window").unwrap().is_empty(),
            "nothing parked"
        );
    }

    /// The next base is the version the server's reply produced. An
    /// idempotent repeat of `complete` answers with neither the version nor the object id
    /// (`already_completed`); the produced version is then W's base + 1 (§8.6.1), never the
    /// stored record + 1. Here a snapshot raised the stored record to W's own version while
    /// W's first reply was lost.
    #[tokio::test]
    async fn a_repeated_completion_bases_the_next_save_on_the_version_w_produced() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [86u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_uploaded_row(&bridge, &server, "reply");
        let w = fp_save(&bridge, dir.path(), "reply", "notes.txt", b"W", "1");
        fp_save(
            &bridge,
            dir.path(),
            "reply",
            "notes.txt",
            b"W N",
            w.token.as_deref().unwrap(),
        );
        server
            .state
            .lock()
            .unwrap()
            .complete_reply_lost_once
            .insert("session-1".into());
        bridge.process_due_operations(&sync_root, now_secs()).await.unwrap();
        assert_eq!(
            server.state.lock().unwrap().files["reply"].versions.len(),
            2,
            "precondition: W is version 2 on the server, and its reply was lost"
        );
        assert!(matches!(
            bridge.db.apply_snapshot_version("reply", 2, 1, false).unwrap(),
            crate::state_db::SnapshotVersion::Raised { old: 1, new: 2 }
        ));
        drain_upload_queue(&bridge, &sync_root).await;
        let state = server.finish();
        assert_eq!(
            state.init_summary(),
            vec![(json!("reply"), json!(1), 201), (json!("reply"), json!(2), 201)],
            "N is based on the version W produced"
        );
        assert_eq!(state.latest_plaintext("reply", master_key), b"W N");
        assert!(bridge.db.list_operations_for_file("reply").unwrap().is_empty());
        assert_eq!(
            bridge
                .db
                .get_file_contract_state("reply")
                .unwrap()
                .unwrap()
                .current_version,
            3
        );
    }

    /// What an upload left behind when a purge removed its op after the server completed
    /// it: during `complete` (`during_complete`), or at the landing.
    struct PurgedLanding {
        op_id: String,
        outcome: TransferLoopOutcome,
        logs: String,
        server_versions: usize,
        op_left: bool,
        row_left: bool,
        resume_left: bool,
    }

    async fn upload_purged_after_the_server_completed(during_complete: bool, master_key: [u8; 32]) -> PurgedLanding {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        const PURGED: &str = "40000000-0000-4000-8000-0000000000f4";
        seed_uploaded_row(&bridge, &server, PURGED);
        fp_save(&bridge, dir.path(), PURGED, "notes.txt", b"a save in a share", "1");
        let op_id = bridge.db.list_operations_for_file(PURGED).unwrap().remove(0).op_id;
        let mut contract = bridge.db.get_file_contract_state(PURGED).unwrap().unwrap();
        contract.namespace = Namespace::SharedWithMe;
        contract.shared_root_id = Some("purged-root".into());
        bridge.db.set_file_contract_state(&contract).unwrap();
        let mut outcome = None;
        let logs = if during_complete {
            // The server completes at once and answers 400 ms later; the share goes away
            // while the reply is in flight.
            server
                .state
                .lock()
                .unwrap()
                .delay_complete
                .insert("session-1".into(), Duration::from_millis(400));
            capture_logs_async(async {
                let ((), ()) = tokio::join!(
                    async {
                        outcome = Some(bridge.process_due_operations(&sync_root, now_secs()).await.unwrap());
                    },
                    async {
                        tokio::time::timeout(Duration::from_secs(10), async {
                            while !server
                                .state
                                .lock()
                                .unwrap()
                                .requests
                                .iter()
                                .any(|(_, path)| path.ends_with("/complete"))
                            {
                                tokio::time::sleep(Duration::from_millis(5)).await;
                            }
                        })
                        .await
                        .expect("complete never reached the mock");
                        bridge.db.purge_revoked_shared_content(&[]).unwrap();
                    }
                );
            })
            .await
        } else {
            let db = bridge.db.clone();
            bridge.seams.arm("landing:after_complete", move || {
                run_competing(move || {
                    db.purge_revoked_shared_content(&[]).unwrap();
                })
            });
            capture_logs_async(async {
                outcome = Some(bridge.process_due_operations(&sync_root, now_secs()).await.unwrap());
            })
            .await
        };
        let state = server.finish();
        PurgedLanding {
            outcome: outcome.unwrap(),
            logs,
            server_versions: state.files[PURGED].versions.len(),
            op_left: bridge.db.get_operation(&op_id).unwrap().is_some(),
            row_left: bridge.db.get_file(PURGED).unwrap().is_some(),
            resume_left: bridge.db.get_upload_resume(&op_id).unwrap().is_some(),
            op_id,
        }
    }

    fn assert_purged_landing_wrote_nothing(run: &PurgedLanding, step: &str) {
        assert_eq!(run.server_versions, 2, "precondition: the server completed the upload");
        assert!(!run.op_left, "a purged op is never re-inserted");
        assert!(!run.row_left, "the landing never brings back a purged row");
        assert!(!run.resume_left, "nor its resume row");
        assert!(
            run.outcome.completed_op_ids.is_empty()
                && run.outcome.retried_op_ids.is_empty()
                && run.outcome.paused_op_ids.is_empty(),
            "an attempt whose op moved is not reported: {:?}",
            run.outcome.completed_op_ids
        );
        let moved: Vec<&str> = run
            .logs
            .lines()
            .filter(|line| line.contains("queue state moved"))
            .collect();
        assert_eq!(moved.len(), 1, "one line for the purged attempt:\n{}", run.logs);
        assert!(
            moved[0].contains(&run.op_id) && moved[0].contains(&format!("step=\"{step}\"")),
            "{}",
            run.logs
        );
    }

    /// The completion record is guarded by the claim (S4).
    #[tokio::test]
    async fn a_completion_for_an_op_purged_under_its_claim_writes_nothing() {
        let run = upload_purged_after_the_server_completed(true, [87u8; 32]).await;
        assert_purged_landing_wrote_nothing(&run, "completion");
    }

    /// The landing transaction is guarded by the
    /// claim (S2, S4) and writes nothing for an op that moved while its upload landed.
    #[tokio::test]
    async fn a_landing_for_an_op_purged_under_its_claim_writes_nothing() {
        let run = upload_purged_after_the_server_completed(false, [88u8; 32]).await;
        assert_purged_landing_wrote_nothing(&run, "landing");
    }

    /// §8.6 rules 4 and 6: a recorded completion lands before anything else is checked, so a
    /// staged copy that is gone by the retry never parks it.
    #[tokio::test]
    async fn a_recorded_completion_lands_even_when_its_staged_copy_is_gone() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [89u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_uploaded_row(&bridge, &server, "copy-gone");
        fp_save(&bridge, dir.path(), "copy-gone", "notes.txt", b"edit", "1");
        let op = bridge.db.list_operations_for_file("copy-gone").unwrap().remove(0);
        crate::state_db::fail_next_landings_for_test(1);
        let now = now_secs();
        bridge.process_due_operations(&sync_root, now).await.unwrap();
        assert!(
            bridge
                .db
                .get_upload_resume(&op.op_id)
                .unwrap()
                .is_some_and(|resume| resume.completed_version == Some(2)),
            "precondition: the completion is recorded"
        );
        std::fs::remove_file(op.payload_path.as_deref().unwrap()).unwrap();
        let logs = capture_logs_async(async {
            bridge.process_due_operations(&sync_root, now + 10_000).await.unwrap();
        })
        .await;
        let state = server.finish();
        assert!(
            bridge.db.list_operations_for_file("copy-gone").unwrap().is_empty(),
            "landed: {logs}"
        );
        assert!(!logs.contains("upload parked"), "{logs}");
        assert_eq!(state.inits.len(), 1);
        assert_eq!(
            bridge
                .db
                .get_file_contract_state("copy-gone")
                .unwrap()
                .unwrap()
                .current_version,
            2
        );
    }

    /// A save with no content type of its own lands with the server's
    /// `mime_type`, as its first landing would, also when that landing is retried from the
    /// recorded completion.
    #[tokio::test]
    async fn a_retried_landing_keeps_the_servers_mime_type() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [92u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_uploaded_row(&bridge, &server, "untyped");
        let contents = dir.path().join("save-untyped");
        std::fs::write(&contents, b"edit").unwrap();
        let mut target = finder_file_target(Some("untyped"), "notes", &contents, Some("1".into()));
        target.content_type = None;
        bridge.queue_file_provider_modify(target).unwrap();
        crate::state_db::fail_next_landings_for_test(1);
        let now = now_secs();
        bridge.process_due_operations(&sync_root, now).await.unwrap();
        bridge.process_due_operations(&sync_root, now + 10_000).await.unwrap();
        drop(server.finish());
        assert!(
            bridge.db.list_operations_for_file("untyped").unwrap().is_empty(),
            "landed on the retry"
        );
        assert_eq!(
            bridge
                .db
                .get_file_contract_state("untyped")
                .unwrap()
                .unwrap()
                .content_type
                .as_deref(),
            Some("text/plain"),
            "the server's mime_type: the save has no content type of its own"
        );
    }

    /// m-11: a landing that fails with an error `classify_operation_error` reads as a pause
    /// (SQLite's "database is locked" is `Locked`) is retried, never paused: a paused op is
    /// not listed again.
    #[tokio::test]
    async fn a_recorded_completion_is_never_paused() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [90u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_uploaded_row(&bridge, &server, "busy");
        fp_save(&bridge, dir.path(), "busy", "notes.txt", b"edit", "1");
        crate::state_db::fail_next_landings_as_locked_for_test(1);
        let now = now_secs();
        let outcome = bridge.process_due_operations(&sync_root, now).await.unwrap();
        assert!(outcome.paused_op_ids.is_empty(), "never paused: {outcome:?}");
        let op = bridge.db.list_operations_for_file("busy").unwrap().remove(0);
        assert_eq!(op.attempts, 1, "retried on the normal backoff");
        bridge.process_due_operations(&sync_root, now + 10_000).await.unwrap();
        let state = server.finish();
        assert!(
            bridge.db.list_operations_for_file("busy").unwrap().is_empty(),
            "landed on the retry"
        );
        assert_eq!(state.inits.len(), 1);
    }

    // ── A File Provider write never stops uploading for a reason that waiting or the person can
    // cure (spec §8.4) ──

    /// `passes` passes of the runner, `10_000` s apart from `now`, while every attempt of
    /// `file_id`'s one queued save fails. After each pass the save is still queued and has
    /// attempts to spare.
    pub(super) async fn failing_passes(
        bridge: &EngineBridge,
        sync_root: &Path,
        file_id: &str,
        passes: usize,
        now: &mut i64,
    ) -> Vec<TransferLoopOutcome> {
        let mut outcomes = Vec::new();
        for pass in 1..=passes {
            outcomes.push(bridge.process_due_operations(sync_root, *now).await.unwrap());
            *now += 10_000;
            let ops = bridge.db.list_operations_for_file(file_id).unwrap();
            assert_eq!(ops.len(), 1, "pass {pass}: the save is still queued");
            assert!(
                ops[0].attempts < ops[0].max_attempts,
                "pass {pass}: never parked, attempts {} of {}",
                ops[0].attempts,
                ops[0].max_attempts
            );
        }
        outcomes
    }

    /// After [`failing_passes`]: no pass paused the save or changed what the system shows, its
    /// count stopped one short of the cap, its row still presents `uploading`, and its last
    /// failure is of `class` (the instrument made the failure it names).
    pub(super) fn assert_kept_uploading(
        bridge: &EngineBridge,
        file_id: &str,
        outcomes: &[TransferLoopOutcome],
        class: OperationFailureClass,
    ) {
        for (pass, outcome) in outcomes.iter().enumerate() {
            assert!(
                outcome.paused_op_ids.is_empty() && outcome.invalidated_item_ids.is_empty(),
                "pass {}: never paused, nothing changed for the system: {outcome:?}",
                pass + 1
            );
        }
        let op = bridge.db.list_operations_for_file(file_id).unwrap().remove(0);
        assert_eq!(
            op.attempts,
            backoff_cap_attempts(),
            "the count stops where the backoff stops growing"
        );
        assert_eq!(
            classify_operation_error(op.last_error.as_deref().unwrap()),
            class,
            "{:?}",
            op.last_error
        );
        assert_eq!(
            bridge.db.get_file(file_id).unwrap().unwrap().status,
            FileStatus::Uploading
        );
    }

    /// An outage longer than the 25 attempts: 30 refusals with 503, then the server answers
    /// and the save lands. Before, the 25th attempt parked it read-only for good.
    #[tokio::test]
    async fn a_finder_write_outlives_thirty_server_errors_and_lands() {
        let master_key = [140u8; 32];
        let (dir, sync_root, server, bridge) = rule3_setup(master_key);
        seed_uploaded_row(&bridge, &server, "outage-503");
        server
            .state
            .lock()
            .unwrap()
            .refuse_init
            .insert("outage-503".into(), "503 Service Unavailable");
        fp_save(
            &bridge,
            dir.path(),
            "outage-503",
            "notes.txt",
            b"saved during the outage",
            "1",
        );
        let mut now = now_secs();
        let logs = capture_logs_async(async {
            let outcomes = failing_passes(&bridge, &sync_root, "outage-503", 30, &mut now).await;
            assert_kept_uploading(&bridge, "outage-503", &outcomes, OperationFailureClass::Retryable);
            server.state.lock().unwrap().refuse_init.remove("outage-503");
            bridge.process_due_operations(&sync_root, now).await.unwrap();
        })
        .await;
        assert!(
            bridge.db.list_operations_for_file("outage-503").unwrap().is_empty(),
            "landed once the server answered"
        );
        assert!(!logs.contains("parked"), "{logs}");
        let state = server.finish();
        assert_eq!(
            state.inits.iter().filter(|(_, status)| *status == 503).count(),
            30,
            "30 refusals reached the server"
        );
        assert_eq!(
            state.latest_plaintext("outage-503", master_key),
            b"saved during the outage"
        );
    }

    /// The same with nothing listening: 30 connection errors, then the network comes back.
    ///
    /// The server the save lands on is started only after the outage, as in the scenario. A
    /// refused connection to a closed port returns at once on macOS and Linux, but on Windows each
    /// takes about 2 s (the stack retries the refused connection), so the 30 passes outlast the
    /// mock's 30 s serving window if it is started first, and the landing found no server.
    #[tokio::test]
    async fn a_finder_write_outlives_thirty_connection_errors_and_lands() {
        let master_key = [141u8; 32];
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        // An API whose address nothing listens on.
        let closed = {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            format!("http://{}", listener.local_addr().unwrap())
        };
        let offline = test_bridge_with_api(&dir.path().join("state.db"), closed, master_key);
        seed_uploaded_db_row(&offline, "outage-offline");
        fp_save(
            &offline,
            dir.path(),
            "outage-offline",
            "notes.txt",
            b"saved offline",
            "1",
        );
        let mut now = now_secs();
        let outcomes = failing_passes(&offline, &sync_root, "outage-offline", 30, &mut now).await;
        assert_kept_uploading(&offline, "outage-offline", &outcomes, OperationFailureClass::Retryable);
        // The network comes back: the same queue, with an API that answers.
        let server = VersionedServerMock::start();
        server.seed_file("outage-offline", 1);
        let bridge = EngineBridge::new(
            offline.db.clone(),
            Arc::new(ApiClient::new(server.base_url.clone(), "token".into(), master_key)),
        );
        bridge.process_due_operations(&sync_root, now).await.unwrap();
        assert!(
            bridge.db.list_operations_for_file("outage-offline").unwrap().is_empty(),
            "landed once the network came back"
        );
        let state = server.finish();
        assert_eq!(state.latest_plaintext("outage-offline", master_key), b"saved offline");
    }

