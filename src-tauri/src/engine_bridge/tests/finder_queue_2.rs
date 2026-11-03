//! `finder_queue_2` engine tests, split out of `engine_bridge.rs`.

use super::*;
    /// A File Provider create of a new file holding `bytes`.
    pub(super) fn fp_create(bridge: &EngineBridge, dir: &Path, filename: &str, bytes: &[u8]) -> FpWrite {
        let contents = dir.join(format!("create-{}.txt", uuid::Uuid::new_v4()));
        std::fs::write(&contents, bytes).unwrap();
        bridge
            .queue_file_provider_create(finder_file_target(None, filename, &contents, None))
            .unwrap()
    }

    #[tokio::test]
    async fn a_second_queued_save_rebases_after_the_first_lands() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [23u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_uploaded_row(&bridge, &server, "9690ce99-8b64-586d-be39-c3e39124022b");

        // Both saves are based on version 1: the second came before the
        // system learned of the version the first produces.
        queue_save(&bridge, dir.path(), "9690ce99-8b64-586d-be39-c3e39124022b", "notes.txt", b"first save", "1");
        queue_save(
            &bridge,
            dir.path(),
            "9690ce99-8b64-586d-be39-c3e39124022b",
            "notes.txt",
            b"first save, second save",
            "1",
        );
        drain_upload_queue(&bridge, &sync_root).await;

        let state = server.finish();
        assert_eq!(
            state.init_summary(),
            vec![
                (serde_json::json!("9690ce99-8b64-586d-be39-c3e39124022b"), serde_json::json!(1), 201),
                (serde_json::json!("9690ce99-8b64-586d-be39-c3e39124022b"), serde_json::json!(2), 201),
            ],
            "the second upload is based on the version the first produced"
        );
        assert_eq!(state.files["9690ce99-8b64-586d-be39-c3e39124022b"].versions.len(), 3);
        assert_eq!(
            state.latest_plaintext("9690ce99-8b64-586d-be39-c3e39124022b", master_key),
            b"first save, second save",
            "the newest bytes are the file's latest version"
        );
        assert!(bridge.db.list_due_operations(i64::MAX).unwrap().is_empty());
        assert_eq!(
            bridge.db.get_file("9690ce99-8b64-586d-be39-c3e39124022b").unwrap().unwrap().status,
            FileStatus::Local
        );
    }

    #[tokio::test]
    async fn a_later_save_waits_for_an_earlier_save_that_is_backing_off() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [24u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_uploaded_row(&bridge, &server, "a08315d5-a79a-5b65-9bd6-0c418cd1a14b");
        // The first save's upload fails once (a dropped chunk) and backs off.
        server
            .state
            .lock()
            .unwrap()
            .fail_first_chunk_once
            .insert("session-1".into());

        queue_save(&bridge, dir.path(), "a08315d5-a79a-5b65-9bd6-0c418cd1a14b", "notes.txt", b"older save", "1");
        queue_save(
            &bridge,
            dir.path(),
            "a08315d5-a79a-5b65-9bd6-0c418cd1a14b",
            "notes.txt",
            b"older save, newer save",
            "1",
        );
        drain_upload_queue(&bridge, &sync_root).await;

        let state = server.finish();
        assert_eq!(state.files["a08315d5-a79a-5b65-9bd6-0c418cd1a14b"].versions.len(), 3, "{:?}", state.init_summary());
        assert_eq!(
            state.latest_plaintext("a08315d5-a79a-5b65-9bd6-0c418cd1a14b", master_key),
            b"older save, newer save",
            "the older save must not land after the newer one: {:?}",
            state.init_summary()
        );
    }

    #[cfg(target_os = "macos")] // other platforms keep created_at order
    #[tokio::test]
    async fn m2_order_is_insertion_order_when_the_clock_steps_back() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [41u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_uploaded_row(&bridge, &server, "0e922871-34b2-58f3-99dc-2e99508f5a94");
        queue_save(&bridge, dir.path(), "0e922871-34b2-58f3-99dc-2e99508f5a94", "notes.txt", b"older save", "1");
        queue_save(
            &bridge,
            dir.path(),
            "0e922871-34b2-58f3-99dc-2e99508f5a94",
            "notes.txt",
            b"older save, newer save",
            "1",
        );
        // The wall clock stepped back between the two saves.
        let ops = bridge.db.list_operations_for_file("0e922871-34b2-58f3-99dc-2e99508f5a94").unwrap();
        assert_eq!(ops.len(), 2);
        bridge.db.set_created_at_for_test(&ops[0].op_id, 2_000_000_000);
        bridge.db.set_created_at_for_test(&ops[1].op_id, 1_000_000_000);

        drain_upload_queue(&bridge, &sync_root).await;

        let state = server.finish();
        assert_eq!(state.files["0e922871-34b2-58f3-99dc-2e99508f5a94"].versions.len(), 3, "{:?}", state.init_summary());
        assert_eq!(
            state.latest_plaintext("0e922871-34b2-58f3-99dc-2e99508f5a94", master_key),
            b"older save, newer save",
            "the newest bytes land last: {:?}",
            state.init_summary()
        );
    }

    #[tokio::test]
    async fn the_runners_copy_never_resurrects_or_drops_a_save() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [42u8; 32];
        let server = VersionedServerMock::start();
        server.state.lock().unwrap().delay_init = Some(Duration::from_millis(400));
        // The purged op's first chunk would fail (its init is the first: session-1). With the
        // guard its attempt ends at the resume write and sends no chunk. Without it, the
        // attempt writes a resume row for the deleted op, fails at the chunk and leaves
        // that row behind, because a failed attempt clears nothing.
        server
            .state
            .lock()
            .unwrap()
            .fail_first_chunk_once
            .insert("session-1".into());
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_uploaded_row(&bridge, &server, "43c0df67-e887-5a79-9f9a-772247f56239");
        queue_save(
            &bridge,
            dir.path(),
            "43c0df67-e887-5a79-9f9a-772247f56239",
            "notes.txt",
            b"save under a share that goes away",
            "1",
        );
        seed_uploaded_row(&bridge, &server, "06b9da0b-043d-50fc-acd4-87909064b924");
        queue_save(&bridge, dir.path(), "06b9da0b-043d-50fc-acd4-87909064b924", "other.txt", b"the second file's save", "1");
        let w = bridge.db.list_operations_for_file("43c0df67-e887-5a79-9f9a-772247f56239").unwrap().remove(0);
        // The first file is content of a share; the share is revoked while W's init is in flight.
        let mut contract = bridge.db.get_file_contract_state("43c0df67-e887-5a79-9f9a-772247f56239").unwrap().unwrap();
        contract.namespace = Namespace::SharedWithMe;
        contract.shared_root_id = Some("revoked-root".into());
        bridge.db.set_file_contract_state(&contract).unwrap();

        let logs = capture_logs_async(async {
            let ((), ()) = tokio::join!(
                async {
                    bridge.process_due_operations(&sync_root, now_secs()).await.unwrap();
                },
                async {
                    tokio::time::timeout(Duration::from_secs(10), async {
                        while server.state.lock().unwrap().inits.is_empty() {
                            tokio::time::sleep(Duration::from_millis(5)).await;
                        }
                    })
                    .await
                    .expect("init never reached the mock");
                    // The revoked-share purge path (SD:2587-2620): a bulk delete by file id.
                    bridge.db.purge_revoked_shared_content(&[]).unwrap();
                }
            );
        })
        .await;

        let state = server.finish();
        assert!(
            bridge.db.get_upload_resume(&w.op_id).unwrap().is_none(),
            "no resume row for a purged op"
        );
        assert!(
            bridge.db.get_operation(&w.op_id).unwrap().is_none(),
            "a purged op is never re-inserted"
        );
        let moved: Vec<&str> = logs.lines().filter(|line| line.contains("queue state moved")).collect();
        assert_eq!(moved.len(), 1, "one line for the purged attempt:\n{logs}");
        assert!(moved[0].contains(&w.op_id) && moved[0].contains("resume"), "{logs}");
        assert!(
            !state
                .requests
                .iter()
                .any(|(_, path)| path.starts_with("/api/v1/uploads/session-1/")),
            "the purged op sends no chunk and no complete: {:?}",
            state.requests
        );
        assert_eq!(
            state.latest_plaintext("06b9da0b-043d-50fc-acd4-87909064b924", master_key),
            b"the second file's save",
            "the other file's save is untouched and lands"
        );
    }

    /// What one refused attempt left behind when a purge removed its op between the
    /// failure and the runner's record of it.
    struct PurgedAttempt {
        op_id: String,
        outcome: TransferLoopOutcome,
        logs: String,
        init_statuses: Vec<u16>,
        op_left: bool,
    }

    /// One queued save whose `init` the server refuses with `refusal`. The share it
    /// belongs to is revoked after the attempt failed and before the runner records the
    /// outcome (the `outcome:before_tx` seam), from another thread.
    async fn refused_attempt_purged_before_its_outcome(refusal: &'static str) -> PurgedAttempt {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let server = VersionedServerMock::start();
        server
            .state
            .lock()
            .unwrap()
            .refuse_init
            .insert("f0b40592-4cab-5cd2-96b3-661bf4c703c2".into(), refusal);
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), [43u8; 32]);
        seed_uploaded_row(&bridge, &server, "f0b40592-4cab-5cd2-96b3-661bf4c703c2");
        queue_save(&bridge, dir.path(), "f0b40592-4cab-5cd2-96b3-661bf4c703c2", "notes.txt", b"a save in a share", "1");
        let op_id = bridge.db.list_operations_for_file("f0b40592-4cab-5cd2-96b3-661bf4c703c2").unwrap().remove(0).op_id;
        let mut contract = bridge.db.get_file_contract_state("f0b40592-4cab-5cd2-96b3-661bf4c703c2").unwrap().unwrap();
        contract.namespace = Namespace::SharedWithMe;
        contract.shared_root_id = Some("purged-root".into());
        bridge.db.set_file_contract_state(&contract).unwrap();
        let db = bridge.db.clone();
        bridge.seams.arm("outcome:before_tx", move || {
            run_competing(move || {
                db.purge_revoked_shared_content(&[]).unwrap();
            })
        });

        let mut outcome = None;
        let logs = capture_logs_async(async {
            outcome = Some(bridge.process_due_operations(&sync_root, now_secs()).await.unwrap());
        })
        .await;
        let op_left = bridge.db.get_operation(&op_id).unwrap().is_some();
        let state = server.finish();
        PurgedAttempt {
            op_id,
            outcome: outcome.unwrap(),
            logs,
            init_statuses: state.inits.iter().map(|(_, status)| *status).collect(),
            op_left,
        }
    }

    fn assert_moved_once(run: &PurgedAttempt, step: &str) {
        assert!(!run.op_left, "a purged op is never re-inserted");
        assert!(
            run.outcome.paused_op_ids.is_empty()
                && run.outcome.retried_op_ids.is_empty()
                && run.outcome.completed_op_ids.is_empty(),
            "an attempt whose op moved is not reported: paused {:?}, retried {:?}, completed {:?}",
            run.outcome.paused_op_ids,
            run.outcome.retried_op_ids,
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

    #[tokio::test]
    async fn a_pause_for_an_op_purged_under_its_claim_writes_nothing() {
        // Auth is the one class that still pauses a File Provider write (spec §8.4).
        let run = refused_attempt_purged_before_its_outcome("401 Unauthorized").await;
        assert_eq!(run.init_statuses, vec![401], "the attempt was refused as a pause");
        assert_moved_once(&run, "pause");
    }

    #[tokio::test]
    async fn an_attempt_for_an_op_purged_under_its_claim_writes_nothing() {
        let run = refused_attempt_purged_before_its_outcome("500 Internal Server Error").await;
        assert_eq!(run.init_statuses, vec![500], "the attempt failed as a retry");
        assert_moved_once(&run, "attempt");
    }

    #[tokio::test]
    async fn a_modify_queued_while_its_create_uploads_lands_on_the_created_file() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [25u8; 32];
        let server = VersionedServerMock::start();
        // The create's chunk takes a while, so the edit arrives mid-upload.
        server
            .state
            .lock()
            .unwrap()
            .delay_chunks
            .insert("session-1".into(), Duration::from_millis(400));
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);

        let created = dir.path().join("created.txt");
        std::fs::write(&created, b"created bytes").unwrap();
        bridge
            .queue_file_provider_create(finder_file_target(None, "t.txt", &created, None))
            .unwrap();
        let local_id = bridge.db.list_files().unwrap()[0].file_id.clone();

        let ((), ()) = tokio::join!(
            async {
                drain_upload_queue(&bridge, &sync_root).await;
            },
            async {
                while server.state.lock().unwrap().inits.is_empty() {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                // The system holds the item under its provisional id.
                let base = held_content_version(&bridge, &local_id);
                queue_save(&bridge, dir.path(), &local_id, "t.txt", b"created bytes, edited", &base);
                assert!(
                    server.state.lock().unwrap().files_with_content().is_empty(),
                    "the edit must be queued while the create is still uploading"
                );
            }
        );
        drain_upload_queue(&bridge, &sync_root).await;

        let state = server.finish();
        let server_id = {
            let files = state.files_with_content();
            assert_eq!(
                files.len(),
                1,
                "exactly one file on the server: {:?}",
                state.init_summary()
            );
            files[0].clone()
        };
        assert!(
            state.files.contains_key(&local_id),
            "the create's own id is the server id: the server never mints one"
        );
        assert_eq!(server_id, local_id, "the create's own id is the server id");
        assert_eq!(state.files[&server_id].versions.len(), 2, "{:?}", state.init_summary());
        assert_eq!(
            state.latest_plaintext(&server_id, master_key),
            b"created bytes, edited",
            "the edit is the created file's latest version"
        );
        assert_eq!(
            state.inits[1].0["base_version_number"],
            serde_json::json!(1),
            "the edit is based on the created version"
        );
        let mk = beebeeb_core::kdf::MasterKey::from_bytes(master_key);
        for (id, body) in state.patches.iter().filter(|(id, _)| id == &server_id) {
            let name = body["name_encrypted"].as_str().unwrap();
            assert_eq!(
                beebeeb_core::encrypt::decrypt_name(&mk, id, name).ok().as_deref(),
                Some("t.txt"),
                "every name sent for the server file is encrypted under its id"
            );
        }
        let rows = bridge.db.list_files().unwrap();
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0].file_id, server_id);
        assert_eq!(rows[0].status, FileStatus::Local);
    }

    // ── Observability: a refused upload leaves a trace ─────────────────────

    /// Everything `tracing` emits on this thread while `body` runs, as text.
    /// `#[tokio::test]` runs the future on the test's own thread, so a
    /// thread-local subscriber sees all of it.
    pub(super) async fn capture_logs_async(body: impl std::future::Future<Output = ()>) -> String {
        #[derive(Clone)]
        struct Capture(Arc<Mutex<Vec<u8>>>);
        impl std::io::Write for Capture {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let writer = Capture(buffer.clone());
        // tracing caches each callsite's interest for the whole process. While
        // only ONE dispatcher is alive, tracing-core takes that interest from
        // the registering thread's own default (`has_just_one`), so a
        // callsite first hit by another test thread, which has no subscriber,
        // is cached as "never" and this capture misses its events. A second
        // live dispatcher keeps registration on the all-dispatchers path.
        let _second_dispatcher = tracing::Dispatch::new(tracing_subscriber::registry());
        let subscriber = tracing_subscriber::fmt()
            .with_writer(move || writer.clone())
            .with_ansi(false)
            .with_max_level(tracing::Level::INFO)
            .finish();
        let guard = tracing::subscriber::set_default(subscriber);
        body.await;
        drop(guard);
        let bytes = buffer.lock().unwrap().clone();
        String::from_utf8(bytes).unwrap()
    }

    #[tokio::test]
    async fn an_upload_refused_with_a_stale_base_is_logged_once_and_parks() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), [26u8; 32]);
        // The server is at version 2; this File Provider save is based on 1 (stale).
        // Only a Finder write parks at once (spec §8.4).
        seed_uploaded_row(&bridge, &server, "9f036aec-d506-5a8b-a7a7-d10f1d9299db");
        server.seed_file("9f036aec-d506-5a8b-a7a7-d10f1d9299db", 2);
        fp_save(
            &bridge,
            dir.path(),
            "9f036aec-d506-5a8b-a7a7-d10f1d9299db",
            "private-notes.txt",
            b"stale save",
            "1",
        );
        let stale = bridge.db.list_operations_for_file("9f036aec-d506-5a8b-a7a7-d10f1d9299db").unwrap().remove(0);
        // Another upload fails once for a different reason (500): not a 409.
        seed_uploaded_row(&bridge, &server, "13da8fb7-10bd-5e55-9aeb-3f4406bed444");
        server
            .state
            .lock()
            .unwrap()
            .fail_first_chunk_once
            .insert("session-1".into());
        fp_save(&bridge, dir.path(), "13da8fb7-10bd-5e55-9aeb-3f4406bed444", "other-notes.txt", b"flaky save", "1");
        let flaky = bridge.db.list_operations_for_file("13da8fb7-10bd-5e55-9aeb-3f4406bed444").unwrap().remove(0);

        let logs = capture_logs_async(async {
            drain_upload_queue(&bridge, &sync_root).await;
        })
        .await;
        let state = server.finish();
        let stale_inits: Vec<_> = state
            .init_summary()
            .into_iter()
            .filter(|(file, _, _)| *file == json!("9f036aec-d506-5a8b-a7a7-d10f1d9299db"))
            .collect();
        assert_eq!(
            stale_inits,
            vec![(json!("9f036aec-d506-5a8b-a7a7-d10f1d9299db"), json!(1), 409)],
            "one refused attempt, then parked: {:?}",
            state.init_summary()
        );
        let refused: Vec<&str> = logs.lines().filter(|line| line.contains("upload refused")).collect();
        assert_eq!(refused.len(), 1, "one line for the one refused attempt, got:\n{logs}");
        assert!(refused[0].contains("WARN"), "{logs}");
        assert!(
            refused[0].contains("parked") && refused[0].contains("stale_base"),
            "the refusal says the op parked, and why:\n{logs}"
        );
        assert!(
            refused[0].contains(&stale.op_id) && refused[0].contains("9f036aec-d506-5a8b-a7a7-d10f1d9299db") && refused[0].contains("409"),
            "{logs}"
        );
        assert!(
            !refused.iter().any(|line| line.contains(&flaky.op_id)),
            "a failure other than 409 is not logged as one:\n{logs}"
        );
        assert!(
            logs.lines()
                .all(|line| !line.contains("notes.txt") && !line.contains("/api/v1") && !line.contains("127.0.0.1")),
            "no name, path or URL may reach any log line:\n{logs}"
        );
        assert!(
            !logs.contains(crate::api_client::STALE_BASE_MESSAGE),
            "the server's text never reaches the log, only the class:\n{logs}"
        );
        let parked = bridge.db.list_operations_for_file("9f036aec-d506-5a8b-a7a7-d10f1d9299db").unwrap().remove(0);
        assert_eq!(parked.attempts, parked.max_attempts, "parked after one attempt");
    }

    // ── Rule 4: the 409 classes, the immediate parks and the hand-over (spec §8.4) ──

