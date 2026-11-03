//! `refusals_2` engine tests, split out of `engine_bridge.rs`.

use super::*;
    /// Spec §8.4: the backoff stops growing at `backoff_cap_attempts`, derived from
    /// `retry_delay_seconds`: the delay there is the largest it gets, and the one before is smaller.
    #[test]
    fn the_backoff_cap_is_where_the_delay_stops_growing() {
        let cap = backoff_cap_attempts();
        assert!(cap >= 1);
        assert_eq!(retry_delay_seconds(cap), retry_delay_seconds(i64::MAX));
        assert_eq!(retry_delay_seconds(cap), retry_delay_seconds(cap + 1));
        if cap > 1 {
            assert!(retry_delay_seconds(cap - 1) < retry_delay_seconds(cap));
        }
    }

    /// Spec §8.4: how each kind of retried attempt moves the count.
    #[test]
    fn a_transient_failure_counts_only_up_to_the_backoff_cap() {
        let cap = backoff_cap_attempts();
        let mut attempts = 0;
        for _ in 0..30 {
            attempts = next_attempt_count(attempts, 25, AttemptCount::UpToTheBackoffCap);
        }
        assert_eq!(attempts, cap, "climbs to the backoff's cap and stays");
        assert_eq!(
            next_attempt_count(20, 25, AttemptCount::UpToTheBackoffCap),
            20,
            "a count already above it stays as it is"
        );
        assert_eq!(
            next_attempt_count(0, 3, AttemptCount::UpToTheBackoffCap),
            1,
            "a small budget: still below max - 1"
        );
        assert_eq!(next_attempt_count(2, 3, AttemptCount::UpToTheBackoffCap), 2);
        assert_eq!(next_attempt_count(23, 25, AttemptCount::ShortOfTheCap), 24);
        assert_eq!(next_attempt_count(24, 25, AttemptCount::ShortOfTheCap), 24);
        assert_eq!(next_attempt_count(24, 25, AttemptCount::UsesUp), 25);
    }

    /// Spec §8.4: a transient failure uses no counted budget past the backoff's cap. After
    /// an outage of 30 passes the count sits where the backoff stops growing, so a failure waiting
    /// does not cure still has its remaining attempts: the write parks only after
    /// `max_attempts - backoff_cap_attempts()` of them (19 today), not at the first.
    #[tokio::test]
    async fn after_an_outage_a_counted_failure_still_has_its_remaining_attempts() {
        let (dir, sync_root, server, bridge) = rule3_setup([166u8; 32]);
        seed_uploaded_row(&bridge, &server, "375b241b-64ae-5601-951b-21f91a915416");
        server
            .state
            .lock()
            .unwrap()
            .refuse_init
            .insert("375b241b-64ae-5601-951b-21f91a915416".into(), "503 Service Unavailable");
        fp_save(&bridge, dir.path(), "375b241b-64ae-5601-951b-21f91a915416", "notes.txt", b"a save", "1");
        let mut now = now_secs();
        failing_passes(&bridge, &sync_root, "375b241b-64ae-5601-951b-21f91a915416", 30, &mut now).await;
        let op = bridge.db.list_operations_for_file("375b241b-64ae-5601-951b-21f91a915416").unwrap().remove(0);
        assert_eq!(op.attempts, backoff_cap_attempts(), "the outage used no more than that");
        let remaining = op.max_attempts - op.attempts;
        assert!(remaining >= 19, "{remaining} attempts left");
        server
            .state
            .lock()
            .unwrap()
            .refuse_init
            .insert("375b241b-64ae-5601-951b-21f91a915416".into(), "400 Bad Request");
        let mut counted = 0;
        loop {
            bridge.process_due_operations(&sync_root, now).await.unwrap();
            now += 10_000;
            counted += 1;
            let op = bridge.db.get_operation(&op.op_id).unwrap().unwrap();
            if op.attempts >= op.max_attempts {
                break;
            }
            assert!(counted < 30, "never parked");
        }
        assert_eq!(
            counted, remaining,
            "parked after its remaining attempts, not at the first"
        );
        drop(server.finish());
    }

    /// Spec §8.4: a save whose upload session was open through an outage. 24 transient
    /// failures at the resumed chunk; then the server has reaped the session, so the resumed chunk
    /// answers 404. The attempt abandons the session, which is not a counted failure, and the next
    /// attempt starts a fresh upload that lands.
    #[tokio::test]
    async fn a_session_reaped_during_an_outage_lands_on_a_fresh_upload() {
        let master_key = [167u8; 32];
        let (dir, sync_root, server, bridge) = rule3_setup(master_key);
        seed_uploaded_row(&bridge, &server, "abb2ac63-26f4-506d-a275-7ef18b27841e");
        refuse(
            &server,
            Refused::Chunk,
            "abb2ac63-26f4-506d-a275-7ef18b27841e",
            ("503 Service Unavailable", json!({ "error": "unavailable" })),
        );
        fp_save(
            &bridge,
            dir.path(),
            "abb2ac63-26f4-506d-a275-7ef18b27841e",
            "notes.txt",
            b"saved through the outage",
            "1",
        );
        let mut now = now_secs();
        failing_passes(&bridge, &sync_root, "abb2ac63-26f4-506d-a275-7ef18b27841e", 24, &mut now).await;
        {
            let mut s = server.state.lock().unwrap();
            assert_eq!(
                s.inits.len(),
                1,
                "precondition: one session, resumed after the first attempt"
            );
            s.refuse_chunk_with.remove("abb2ac63-26f4-506d-a275-7ef18b27841e");
            s.sessions.clear(); // the server reaped it
        }
        let reaped = bridge.process_due_operations(&sync_root, now).await.unwrap();
        now += 10_000;
        assert!(reaped.invalidated_item_ids.is_empty(), "nothing parked: {reaped:?}");
        let op = bridge.db.list_operations_for_file("abb2ac63-26f4-506d-a275-7ef18b27841e").unwrap().remove(0);
        assert!(
            op.attempts < op.max_attempts,
            "not parked: {} of {}",
            op.attempts,
            op.max_attempts
        );
        assert_eq!(op.attempts, backoff_cap_attempts(), "the gone session is not counted");
        assert!(
            bridge.db.get_upload_resume(&op.op_id).unwrap().is_none(),
            "precondition: the session was abandoned"
        );
        bridge.process_due_operations(&sync_root, now).await.unwrap();
        assert!(
            bridge.db.list_operations_for_file("abb2ac63-26f4-506d-a275-7ef18b27841e").unwrap().is_empty(),
            "landed on a fresh upload"
        );
        let state = server.finish();
        assert_eq!(state.inits.len(), 2, "a fresh init after the gone session");
        assert_eq!(
            state.latest_plaintext("abb2ac63-26f4-506d-a275-7ef18b27841e", master_key),
            b"saved through the outage"
        );
    }

    /// Spec §8.4 (fix round 5): a resumed session the server answers with 400 is abandoned, so the
    /// next attempt starts a fresh upload, but the 400 is counted: one attempt. Only a reaped
    /// session (404, 410) is not.
    #[tokio::test]
    async fn a_resumed_sessions_400_is_counted_and_the_next_attempt_starts_fresh() {
        let master_key = [169u8; 32];
        let (dir, sync_root, server, bridge) = rule3_setup(master_key);
        seed_uploaded_row(&bridge, &server, "f39d78f8-e6a7-5b2e-968e-a1fc7d9a447a");
        refuse(
            &server,
            Refused::Chunk,
            "f39d78f8-e6a7-5b2e-968e-a1fc7d9a447a",
            ("503 Service Unavailable", json!({ "error": "unavailable" })),
        );
        fp_save(&bridge, dir.path(), "f39d78f8-e6a7-5b2e-968e-a1fc7d9a447a", "notes.txt", b"a save", "1");
        let op_id = bridge
            .db
            .list_operations_for_file("f39d78f8-e6a7-5b2e-968e-a1fc7d9a447a")
            .unwrap()
            .remove(0)
            .op_id;
        let mut now = now_secs();
        // An outage first: the count sits at the backoff's cap, where a transient failure no
        // longer moves it, so only a counted failure can.
        failing_passes(&bridge, &sync_root, "f39d78f8-e6a7-5b2e-968e-a1fc7d9a447a", 8, &mut now).await;
        let before = bridge.db.get_operation(&op_id).unwrap().unwrap().attempts;
        assert_eq!(before, backoff_cap_attempts(), "precondition: at the backoff's cap");
        assert!(
            bridge.db.get_upload_resume(&op_id).unwrap().is_some(),
            "precondition: a session to resume"
        );
        refuse(
            &server,
            Refused::Chunk,
            "f39d78f8-e6a7-5b2e-968e-a1fc7d9a447a",
            ("400 Bad Request", json!({ "error": "bad_request" })),
        );
        bridge.process_due_operations(&sync_root, now + 10_000).await.unwrap();
        let op = bridge.db.get_operation(&op_id).unwrap().unwrap();
        assert_eq!(op.attempts, before + 1, "the 400 is counted");
        assert!(
            bridge.db.get_upload_resume(&op_id).unwrap().is_none(),
            "the session was abandoned"
        );
        stop_refusing(&server, "f39d78f8-e6a7-5b2e-968e-a1fc7d9a447a");
        bridge.process_due_operations(&sync_root, now + 20_000).await.unwrap();
        assert!(
            bridge.db.list_operations_for_file("f39d78f8-e6a7-5b2e-968e-a1fc7d9a447a").unwrap().is_empty(),
            "landed on a fresh upload"
        );
        let state = server.finish();
        assert_eq!(state.inits.len(), 2, "the next attempt started fresh");
        assert_eq!(state.latest_plaintext("f39d78f8-e6a7-5b2e-968e-a1fc7d9a447a", master_key), b"a save");
    }

    /// Spec §8.4 (fix round 5): a server that answers 400 to every fresh session still parks the
    /// write when its attempts are used up.
    #[tokio::test]
    async fn a_server_that_refuses_every_session_with_400_parks_the_write() {
        let (dir, sync_root, server, bridge) = rule3_setup([170u8; 32]);
        seed_uploaded_row(&bridge, &server, "c276b1a0-4526-5eb0-8238-e1fbd4a9a194");
        refuse(
            &server,
            Refused::Chunk,
            "c276b1a0-4526-5eb0-8238-e1fbd4a9a194",
            ("400 Bad Request", json!({ "error": "bad_request" })),
        );
        fp_save(&bridge, dir.path(), "c276b1a0-4526-5eb0-8238-e1fbd4a9a194", "notes.txt", b"a save", "1");
        let op_id = bridge.db.list_operations_for_file("c276b1a0-4526-5eb0-8238-e1fbd4a9a194").unwrap().remove(0).op_id;
        let mut now = now_secs();
        let mut parked_at = None;
        for pass in 1..=30 {
            bridge.process_due_operations(&sync_root, now).await.unwrap();
            now += 10_000;
            let op = bridge.db.get_operation(&op_id).unwrap().unwrap();
            if parked_at.is_none() && op.attempts >= op.max_attempts {
                parked_at = Some(pass);
            }
        }
        assert_eq!(parked_at, Some(25), "parked when its attempts were used up");
        assert_eq!(
            bridge.db.get_file("c276b1a0-4526-5eb0-8238-e1fbd4a9a194").unwrap().unwrap().status,
            FileStatus::Error
        );
        assert_eq!(
            server.finish().inits.len(),
            25,
            "a fresh session per attempt, then nothing"
        );
    }

    /// Spec §8.4 (fix round 6): only a session resumed from its persisted row can be reaped. A
    /// 404 at the chunk of a session opened in the same attempt is counted (the session is still
    /// abandoned, so the next attempt opens a new one), so a server that answers 404 to every new
    /// session parks the write when its attempts are used up.
    #[tokio::test]
    async fn a_404_on_every_new_session_parks_the_write() {
        let (dir, sync_root, server, bridge) = rule3_setup([172u8; 32]);
        seed_uploaded_row(&bridge, &server, "800250eb-b5ce-5a2b-8653-67e110b79347");
        refuse(
            &server,
            Refused::Chunk,
            "800250eb-b5ce-5a2b-8653-67e110b79347",
            ("404 Not Found", json!({ "error": "not_found" })),
        );
        fp_save(&bridge, dir.path(), "800250eb-b5ce-5a2b-8653-67e110b79347", "notes.txt", b"a save", "1");
        let op_id = bridge.db.list_operations_for_file("800250eb-b5ce-5a2b-8653-67e110b79347").unwrap().remove(0).op_id;
        let mut now = now_secs();
        let mut parked_at = None;
        for pass in 1..=30 {
            bridge.process_due_operations(&sync_root, now).await.unwrap();
            now += 10_000;
            let op = bridge.db.get_operation(&op_id).unwrap().unwrap();
            if parked_at.is_none() && op.attempts >= op.max_attempts {
                parked_at = Some(pass);
            }
        }
        assert_eq!(parked_at, Some(25), "parked when its attempts were used up");
        assert_eq!(
            bridge.db.get_file("800250eb-b5ce-5a2b-8653-67e110b79347").unwrap().unwrap().status,
            FileStatus::Error,
            "presented read-only"
        );
        assert_eq!(
            server.finish().inits.len(),
            25,
            "a new session per attempt, then nothing"
        );
    }

    /// Spec §11 (fix round 5): the gone-session line logs ids and a category, never the error's
    /// text (it carries the request URL with the session id).
    #[tokio::test]
    async fn the_gone_session_line_logs_a_category_not_its_text() {
        let (dir, sync_root, server, bridge) = rule3_setup([171u8; 32]);
        seed_uploaded_row(&bridge, &server, "aed0a62d-d7d2-5e26-9d3d-42554cf4da99");
        refuse(
            &server,
            Refused::Chunk,
            "aed0a62d-d7d2-5e26-9d3d-42554cf4da99",
            ("503 Service Unavailable", json!({ "error": "unavailable" })),
        );
        fp_save(&bridge, dir.path(), "aed0a62d-d7d2-5e26-9d3d-42554cf4da99", "notes.txt", b"a save", "1");
        let now = now_secs();
        bridge.process_due_operations(&sync_root, now).await.unwrap();
        {
            let mut s = server.state.lock().unwrap();
            s.refuse_chunk_with.remove("aed0a62d-d7d2-5e26-9d3d-42554cf4da99");
            s.sessions.clear();
        }
        let logs = capture_logs_async(async {
            bridge.process_due_operations(&sync_root, now + 10_000).await.unwrap();
        })
        .await;
        let line = assert_clean_line(
            &logs,
            "server no longer accepts the upload session",
            &["op_id", "file_id", "reason", "status", "resumed"],
            dir.path(),
        );
        assert!(line.contains("status=404") && line.contains("resumed=true"), "{line}");
        assert!(!line.contains("error="), "{line}");
        drop(server.finish());
    }

    /// The error `reqwest` gives for `status`, from a one-request server: what the upload calls
    /// return through `error_for_status`.
    pub(super) async fn reqwest_status_error(status: &'static str) -> anyhow::Error {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let _ = read_http_request(&mut stream);
            stream
                .write_all(http_json(status, json!({ "error": "refused" })).as_bytes())
                .unwrap();
        });
        let response = reqwest::Client::new().get(&url).send().await.unwrap();
        let error = response.error_for_status().unwrap_err();
        server.join().unwrap();
        anyhow::Error::new(error)
    }

    /// `finder_write_failure_is_transient` for `error`, with the class the runner gives it.
    pub(super) fn transient(error: &anyhow::Error) -> bool {
        finder_write_failure_is_transient(classify_operation_error(&error.to_string()), error)
    }

