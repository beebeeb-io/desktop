//! `finder_queue_6` engine tests, split out of `engine_bridge.rs`.

use super::*;
    /// Spec §8.4: which failures of a File Provider write waiting cures. Decided by the error's
    /// type and kind, never by its text.
    #[tokio::test]
    async fn a_finder_write_failure_is_transient_only_when_waiting_cures_it() {
        // Transient: nothing listens; a server that never answers; 503; 429; 408; 425.
        let closed = {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            format!("http://{}/", listener.local_addr().unwrap())
        };
        let refused = anyhow::Error::new(reqwest::Client::new().get(&closed).send().await.unwrap_err());
        assert!(
            refused.chain().any(|cause| cause
                .downcast_ref::<reqwest::Error>()
                .is_some_and(reqwest::Error::is_connect)),
            "precondition: a connect error"
        );
        assert!(transient(&refused), "connection refused: {refused:#}");
        let silent = TcpListener::bind("127.0.0.1:0").unwrap();
        let silent_url = format!("http://{}/", silent.local_addr().unwrap());
        let timed_out = anyhow::Error::new(
            reqwest::Client::builder()
                .timeout(Duration::from_millis(200))
                .build()
                .unwrap()
                .get(&silent_url)
                .send()
                .await
                .unwrap_err(),
        );
        drop(silent);
        assert!(
            timed_out.chain().any(|cause| cause
                .downcast_ref::<reqwest::Error>()
                .is_some_and(reqwest::Error::is_timeout)),
            "precondition: a timeout"
        );
        assert!(transient(&timed_out), "timeout: {timed_out:#}");
        for status in [
            "503 Service Unavailable",
            "500 Internal Server Error",
            "429 Too Many Requests",
            "408 Request Timeout",
            "425 Too Early",
        ] {
            let error = reqwest_status_error(status).await;
            assert_eq!(
                classify_operation_error(&error.to_string()),
                OperationFailureClass::Retryable
            );
            assert!(transient(&error), "{status}: {error:#}");
        }
        // Locked and Quota are transient whatever carries them.
        for class in [OperationFailureClass::Locked, OperationFailureClass::Quota] {
            assert!(
                finder_write_failure_is_transient(class, &anyhow::anyhow!("x")),
                "{class:?}"
            );
        }

        // Counted: any other status; a local error; a network-looking I/O error with no request
        // in its chain (a local file operation, not the network path); and text that only looks
        // like a transient status.
        for status in [
            "400 Bad Request",
            "404 Not Found",
            "410 Gone",
            "413 Payload Too Large",
            "422 Unprocessable Entity",
        ] {
            let error = reqwest_status_error(status).await;
            assert_eq!(
                classify_operation_error(&error.to_string()),
                OperationFailureClass::Retryable
            );
            assert!(!transient(&error), "{status}: {error:#}");
        }
        assert!(!transient(&anyhow::anyhow!("staged upload payload could not be read")));
        assert!(!transient(&anyhow::Error::new(std::io::Error::from(
            std::io::ErrorKind::ConnectionReset
        ))));
        assert!(!transient(&anyhow::anyhow!("HTTP 503 Service Unavailable: timed out")));
        // Auth and Permission never reach the rule (they pause and park), and are not transient.
        for class in [OperationFailureClass::Auth, OperationFailureClass::Permission] {
            assert!(!finder_write_failure_is_transient(class, &refused), "{class:?}");
        }
    }

    /// Spec §8.4: a failure waiting does not cure (here 30 × 400) still uses up a File Provider
    /// write's attempts. It parks at the 25th, presented read-only, with its id in that tick's
    /// signal, and nothing more is sent.
    #[tokio::test]
    async fn a_finder_write_the_server_keeps_refusing_parks_when_its_attempts_are_used_up() {
        let (dir, sync_root, server, bridge) = rule3_setup([157u8; 32]);
        seed_uploaded_row(&bridge, &server, "6e3e2a13-624a-5cd7-b46d-d9ff5fa6ee68");
        server
            .state
            .lock()
            .unwrap()
            .refuse_init
            .insert("6e3e2a13-624a-5cd7-b46d-d9ff5fa6ee68".into(), "400 Bad Request");
        fp_save(
            &bridge,
            dir.path(),
            "6e3e2a13-624a-5cd7-b46d-d9ff5fa6ee68",
            "notes.txt",
            b"a save the server refuses",
            "1",
        );
        let op_id = bridge
            .db
            .list_operations_for_file("6e3e2a13-624a-5cd7-b46d-d9ff5fa6ee68")
            .unwrap()
            .remove(0)
            .op_id;
        let mut now = now_secs();
        let mut parked_at = None;
        for pass in 1..=30 {
            let outcome = bridge.process_due_operations(&sync_root, now).await.unwrap();
            now += 10_000;
            let op = bridge.db.get_operation(&op_id).unwrap().expect("kept in the queue");
            if parked_at.is_none() && op.attempts >= op.max_attempts {
                parked_at = Some(pass);
                assert_eq!(
                    outcome.invalidated_item_ids,
                    vec!["6e3e2a13-624a-5cd7-b46d-d9ff5fa6ee68".to_string()],
                    "the attempt that used them up is in the tick's signal"
                );
            }
        }
        assert_eq!(parked_at, Some(25), "parked when its 25 attempts were used up");
        let op = bridge.db.get_operation(&op_id).unwrap().unwrap();
        assert_eq!(op.attempts, op.max_attempts);
        assert_eq!(
            bridge.db.get_file("6e3e2a13-624a-5cd7-b46d-d9ff5fa6ee68").unwrap().unwrap().status,
            FileStatus::Error,
            "presented read-only"
        );
        assert_eq!(
            std::fs::read(op.payload_path.as_deref().unwrap()).unwrap(),
            b"a save the server refuses",
            "its bytes are kept"
        );
        let state = server.finish();
        assert_eq!(state.inits.len(), 25, "nothing is sent once it parked");
        assert!(state.inits.iter().all(|(_, status)| *status == 400));
    }

    /// Spec §11: a File Provider write whose attempts a failure waiting does not cure used up
    /// (here 422, not a 409) logs the park line once, at the attempt that used them up, with
    /// `reason="attempts_used_up"`, and never before.
    #[tokio::test]
    async fn an_attempts_used_up_park_logs_its_line_once_at_the_cap() {
        let file = "3f2a9c1e-0000-4000-8000-0000000000c5";
        let (dir, sync_root, server, bridge) = rule3_setup([158u8; 32]);
        seed_uploaded_row(&bridge, &server, file);
        server
            .state
            .lock()
            .unwrap()
            .refuse_init
            .insert(file.into(), "422 Unprocessable Entity");
        fp_save(
            &bridge,
            dir.path(),
            file,
            "notes.txt",
            b"a save the server cannot take",
            "1",
        );
        let op_id = bridge.db.list_operations_for_file(file).unwrap().remove(0).op_id;
        let mut now = now_secs();
        let mut lines_per_pass = Vec::new();
        for _ in 1..=30 {
            let logs = capture_logs_async(async {
                bridge.process_due_operations(&sync_root, now).await.unwrap();
            })
            .await;
            now += 10_000;
            let lines: Vec<String> = logs
                .lines()
                .filter(|line| line.contains("upload parked with its bytes kept in the queue"))
                .map(str::to_string)
                .collect();
            lines_per_pass.push(lines);
        }
        let passes_with_a_line: Vec<usize> = lines_per_pass
            .iter()
            .enumerate()
            .filter(|(_, lines)| !lines.is_empty())
            .map(|(index, _)| index + 1)
            .collect();
        assert_eq!(
            passes_with_a_line,
            vec![25],
            "only the attempt that used them up logs it"
        );
        assert_eq!(lines_per_pass[24].len(), 1, "once: {:?}", lines_per_pass[24]);
        let line = &lines_per_pass[24][0];
        assert!(
            line.contains("reason=\"attempts_used_up\"")
                && line.contains(&op_id)
                && line.contains(&format!("file_id={file}")),
            "{line}"
        );
        assert!(
            !line.contains("Unprocessable") && !line.contains("http://"),
            "no error text: {line}"
        );
        let op = bridge.db.get_operation(&op_id).unwrap().unwrap();
        assert_eq!(op.attempts, op.max_attempts, "precondition: parked");
        drop(server.finish());
    }

    /// One refusal of an upload WITHOUT a write id (the watcher, Windows), queued at `attempts`:
    /// the outcome and the op after it.
    async fn refused_upload_without_a_write_id(
        master_key: [u8; 32],
        file_id: &str,
        refusal: &'static str,
        attempts: i64,
    ) -> (TransferLoopOutcome, PendingOperation) {
        let (dir, sync_root, server, bridge) = rule3_setup(master_key);
        seed_uploaded_row(&bridge, &server, file_id);
        server.state.lock().unwrap().refuse_init.insert(file_id.into(), refusal);
        let contents = dir.path().join("watched.txt");
        std::fs::write(&contents, b"a watched edit").unwrap();
        bridge
            .queue_finder_modify(finder_file_target(
                Some(file_id),
                "notes.txt",
                &contents,
                Some("1".into()),
            ))
            .unwrap();
        let op_id = bridge.db.list_operations_for_file(file_id).unwrap().remove(0).op_id;
        let no_write_id: bool = bridge
            .db
            .hold_lock_for_test()
            .query_row(
                "SELECT write_id IS NULL FROM operation_queue WHERE op_id = ?1 AND attempts = 0",
                rusqlite::params![op_id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(no_write_id, "precondition: an upload without a write id");
        bridge
            .db
            .hold_lock_for_test()
            .execute(
                "UPDATE operation_queue SET attempts = ?2 WHERE op_id = ?1",
                rusqlite::params![op_id, attempts],
            )
            .unwrap();
        let outcome = bridge.process_due_operations(&sync_root, now_secs()).await.unwrap();
        let op = bridge.db.get_operation(&op_id).unwrap().expect("still queued");
        assert_eq!(server.finish().inits.len(), 1, "one attempt reached the server");
        (outcome, op)
    }

    /// Today's pause, for each class that pauses an upload without a write id.
    async fn assert_upload_without_a_write_id_pauses(master_key: [u8; 32], file_id: &str, refusal: &'static str) {
        let (outcome, op) = refused_upload_without_a_write_id(master_key, file_id, refusal, 0).await;
        assert_eq!(outcome.paused_op_ids, vec![op.op_id.clone()], "{refusal}: paused");
        assert_eq!(op.attempts, 0, "{refusal}: a pause is not an attempt");
    }

    #[tokio::test]
    async fn an_upload_without_a_write_id_still_pauses_for_auth() {
        assert_upload_without_a_write_id_pauses([145u8; 32], "plain-auth", "401 Unauthorized").await;
    }

    #[tokio::test]
    async fn an_upload_without_a_write_id_still_pauses_for_quota() {
        assert_upload_without_a_write_id_pauses([146u8; 32], "plain-quota", "507 Insufficient Storage").await;
    }

    #[tokio::test]
    async fn an_upload_without_a_write_id_still_pauses_for_permission() {
        assert_upload_without_a_write_id_pauses([147u8; 32], "plain-permission", "403 Forbidden").await;
    }

    #[tokio::test]
    async fn an_upload_without_a_write_id_still_pauses_when_locked() {
        assert_upload_without_a_write_id_pauses([148u8; 32], "plain-locked", "423 Locked").await;
    }

    /// Today's give-up: a retryable failure at the last attempt uses the attempts up.
    #[tokio::test]
    async fn an_upload_without_a_write_id_still_gives_up_after_its_attempts() {
        let (outcome, op) =
            refused_upload_without_a_write_id([149u8; 32], "plain-retry", "503 Service Unavailable", 24).await;
        assert!(outcome.paused_op_ids.is_empty(), "{outcome:?}");
        assert_eq!(op.attempts, op.max_attempts, "its attempts are used up");
        assert_eq!(op.max_attempts, 25);
        assert_eq!(outcome.invalidated_item_ids, vec!["plain-retry".to_string()]);
    }

    // ── The final review's small items ──

    /// Security P3-2: a fetch served from the queue records its destination as the server path
    /// does, so the sign-out sweep finds a copy written outside the hydrate cache. The row keeps
    /// its upload's status.
    #[tokio::test]
    async fn a_fetch_served_from_the_queue_records_its_destination() {
        let (dir, _sync_root, server, bridge) = rule3_setup([150u8; 32]);
        seed_uploaded_row(&bridge, &server, "8d8e18bd-71a1-5b6f-8e2a-afca0865d12b");
        fp_save(&bridge, dir.path(), "8d8e18bd-71a1-5b6f-8e2a-afca0865d12b", "notes.txt", b"queued bytes", "1");
        let dest = dir.path().join("fetch").join("notes.txt");
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        assert_eq!(
            bridge
                .serve_hydrate("8d8e18bd-71a1-5b6f-8e2a-afca0865d12b", &dest, &[dir.path()], None)
                .await
                .unwrap(),
            HydrateSource::Queue
        );
        let contract = bridge.db.get_file_contract_state("8d8e18bd-71a1-5b6f-8e2a-afca0865d12b").unwrap().unwrap();
        assert_eq!(
            contract.cache_path.as_deref(),
            Some(dest.to_string_lossy().as_ref()),
            "recorded, as the server path records it"
        );
        assert_eq!(contract.cache_bytes, 12);
        assert_eq!(
            bridge.db.get_file("8d8e18bd-71a1-5b6f-8e2a-afca0865d12b").unwrap().unwrap().status,
            FileStatus::Uploading,
            "the upload's status stays"
        );
        drop(server.finish());
    }

