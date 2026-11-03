//! `finder_queue_5` engine tests, split out of `engine_bridge.rs`.

use super::*;
    /// A failure read as `Locked` (here a 423; SQLite's "database is locked" too), 30 times,
    /// then it clears: the save lands. Before, the first one paused it and nothing resumed it.
    #[tokio::test]
    async fn a_finder_write_refused_as_locked_thirty_times_lands_and_is_never_paused() {
        let master_key = [142u8; 32];
        let (dir, sync_root, server, bridge) = rule3_setup(master_key);
        seed_uploaded_row(&bridge, &server, "3c8d48c4-6dd0-5428-a3c0-4340524db744");
        server
            .state
            .lock()
            .unwrap()
            .refuse_init
            .insert("3c8d48c4-6dd0-5428-a3c0-4340524db744".into(), "423 Locked");
        fp_save(
            &bridge,
            dir.path(),
            "3c8d48c4-6dd0-5428-a3c0-4340524db744",
            "notes.txt",
            b"saved while locked",
            "1",
        );
        let mut now = now_secs();
        let outcomes = failing_passes(&bridge, &sync_root, "3c8d48c4-6dd0-5428-a3c0-4340524db744", 30, &mut now).await;
        assert_kept_uploading(&bridge, "3c8d48c4-6dd0-5428-a3c0-4340524db744", &outcomes, OperationFailureClass::Locked);
        server.state.lock().unwrap().refuse_init.remove("3c8d48c4-6dd0-5428-a3c0-4340524db744");
        bridge.process_due_operations(&sync_root, now).await.unwrap();
        assert!(
            bridge.db.list_operations_for_file("3c8d48c4-6dd0-5428-a3c0-4340524db744").unwrap().is_empty(),
            "landed once the lock cleared"
        );
        let state = server.finish();
        assert_eq!(state.inits.iter().filter(|(_, status)| *status == 423).count(), 30);
        assert_eq!(
            state.latest_plaintext("3c8d48c4-6dd0-5428-a3c0-4340524db744", master_key),
            b"saved while locked"
        );
    }

    /// Where the mock refuses a File Provider write.
    #[derive(Debug, Clone, Copy)]
    pub(super) enum Refused {
        Init,
        Chunk,
        Complete,
    }

    /// The server's real refusal bodies (server `error.rs`, `signup_plan.rs`): the status and the
    /// JSON the client reads its code from.
    fn quota_exceeded() -> (&'static str, serde_json::Value) {
        (
            "413 Payload Too Large",
            json!({ "error": "quota_exceeded", "limit_bytes": 5368709120_i64, "used_bytes": 5368709120_i64, "is_trial_cap": false }),
        )
    }

    fn object_budget_exceeded() -> (&'static str, serde_json::Value) {
        (
            "403 Forbidden",
            json!({
                "error": "object_budget_exceeded",
                "message": "This account has reached its stored-object limit. Delete files or old versions to free objects.",
                "used": 100000,
                "cap": 100000
            }),
        )
    }

    fn billing_read_only() -> (&'static str, serde_json::Value) {
        (
            "403 Forbidden",
            json!({
                "error": "billing_read_only",
                "message": "Your account is read-only due to an unpaid invoice. Update your payment method to restore access."
            }),
        )
    }

    /// The share gate's 409, which no upload route sends (server `signup_plan.rs`,
    /// `check_has_plan`): another 409 code, so it keeps the counted rule.
    fn trial_ended() -> (&'static str, serde_json::Value) {
        (
            "409 Conflict",
            json!({
                "error": "trial_ended",
                "message": "Your trial has ended and your account holds more than its included storage, so it is read-only. Subscribe, or free up space, to keep your files."
            }),
        )
    }

    /// The upload gate's 409s (server `signup_plan.rs`, `ensure_can_upload_for`, called at init,
    /// a chunk and complete): `{ "error": code, "message": message }`.
    fn account_lapsed() -> (&'static str, serde_json::Value) {
        (
            "409 Conflict",
            json!({
                "error": "account_lapsed",
                "message": "Your trial has ended and your account is read-only. Subscribe to keep using Beebeeb."
            }),
        )
    }

    fn plan_required() -> (&'static str, serde_json::Value) {
        (
            "409 Conflict",
            json!({ "error": "plan_required", "message": "Choose a plan to start using Beebeeb." }),
        )
    }

    fn trial_cancelled_read_only() -> (&'static str, serde_json::Value) {
        (
            "409 Conflict",
            json!({
                "error": "trial_cancelled_read_only",
                "message": "You cancelled your trial before it was charged, so uploads are paused. You can still view and download your files until the trial's end date."
            }),
        )
    }

    pub(super) fn refuse(server: &VersionedServerMock, at: Refused, file_id: &str, refusal: (&'static str, serde_json::Value)) {
        let mut s = server.state.lock().unwrap();
        match at {
            Refused::Init => s.refuse_init_with.insert(file_id.into(), refusal),
            Refused::Chunk => s.refuse_chunk_with.insert(file_id.into(), refusal),
            Refused::Complete => s.refuse_complete_with.insert(file_id.into(), refusal),
        };
    }

    pub(super) fn stop_refusing(server: &VersionedServerMock, file_id: &str) {
        let mut s = server.state.lock().unwrap();
        s.refuse_init_with.remove(file_id);
        s.refuse_chunk_with.remove(file_id);
        s.refuse_complete_with.remove(file_id);
    }

    /// U17, the arithmetic: a change made due at the phase's start, failing every attempt, is tried at most twice in
    /// 60 s and never parks there, for an other change (`UsesUp`) and for a Finder write's transient failure.
    #[test]
    fn a_phase_never_parks_what_it_made_due() {
        for (max_attempts, start) in [(5_i64, 0_i64), (5, 2), (25, 0), (25, 22)] {
            for count in [AttemptCount::UsesUp, AttemptCount::UpToTheBackoffCap] {
                let mut attempts = start;
                let mut at = 0_i64;
                let mut tries = 0;
                while at <= 60 {
                    attempts = next_attempt_count(attempts, max_attempts, count);
                    tries += 1;
                    at += retry_delay_seconds(attempts);
                }
                assert!(tries <= 2, "{max_attempts}/{start}/{count:?}: {tries} tries");
                assert!(
                    attempts < max_attempts,
                    "{max_attempts}/{start}/{count:?}: parked at {attempts}"
                );
            }
        }
    }

    /// U46 (spec §15 question 4): one attempt that meets a refusal the account cures says so to the send phase.
    #[tokio::test]
    async fn a_refusal_the_account_cures_ends_the_send_phase() {
        let refusals = [
            quota_exceeded(),
            object_budget_exceeded(),
            billing_read_only(),
            account_lapsed(),
            plan_required(),
            trial_cancelled_read_only(),
        ];
        for (index, refusal) in refusals.into_iter().enumerate() {
            let key = [190u8 + index as u8; 32];
            let file_id = format!("c2000000-0000-4000-8000-{index:012}");
            let (dir, sync_root, server, bridge) = rule3_setup(key);
            seed_uploaded_row(&bridge, &server, &file_id);
            refuse(&server, Refused::Init, &file_id, refusal);
            fp_save(&bridge, dir.path(), &file_id, "notes.txt", b"bytes", "1");
            let outcome = bridge.process_due_operations(&sync_root, now_secs()).await.unwrap();
            assert!(
                outcome.account_refused && !outcome.link_failed && !outcome.session_refused,
                "{file_id}: {outcome:?}"
            );
            assert!(
                bridge
                    .db
                    .list_operations_for_file(&file_id)
                    .unwrap()
                    .iter()
                    .all(|op| op.attempts < op.max_attempts),
                "still live"
            );
        }
        let (dir, sync_root, server, bridge) = rule3_setup([199u8; 32]);
        seed_uploaded_row(&bridge, &server, "bcbb5e6c-b7dc-5967-9f07-555d94a3a2b9");
        refuse(
            &server,
            Refused::Init,
            "bcbb5e6c-b7dc-5967-9f07-555d94a3a2b9",
            ("413 Payload Too Large", json!({ "error": "payload_too_large" })),
        );
        fp_save(&bridge, dir.path(), "bcbb5e6c-b7dc-5967-9f07-555d94a3a2b9", "notes.txt", b"bytes", "1");
        let outcome = bridge.process_due_operations(&sync_root, now_secs()).await.unwrap();
        assert!(
            !outcome.account_refused,
            "a plain 413 is counted, not an account refusal"
        );
    }

    /// Spec §8.4: a refusal the person cures (free space, subscribe, pay), in the server's real
    /// shape, 30 times: the File Provider write is never paused or parked and keeps presenting
    /// `uploading`; once the person has cured it, the save lands.
    async fn a_curable_refusal_lands_once_cured(
        master_key: [u8; 32],
        file_id: &str,
        at: Refused,
        refusal: (&'static str, serde_json::Value),
    ) {
        let (dir, sync_root, server, bridge) = rule3_setup(master_key);
        seed_uploaded_row(&bridge, &server, file_id);
        refuse(&server, at, file_id, refusal);
        fp_save(&bridge, dir.path(), file_id, "notes.txt", b"saved at the limit", "1");
        let mut now = now_secs();
        let outcomes = failing_passes(&bridge, &sync_root, file_id, 30, &mut now).await;
        for (pass, outcome) in outcomes.iter().enumerate() {
            assert!(
                outcome.paused_op_ids.is_empty() && outcome.invalidated_item_ids.is_empty(),
                "{at:?} pass {}: never paused, nothing changed for the system: {outcome:?}",
                pass + 1
            );
        }
        assert_eq!(
            bridge.db.get_file(file_id).unwrap().unwrap().status,
            FileStatus::Uploading,
            "{at:?}: still uploading"
        );
        stop_refusing(&server, file_id);
        bridge.process_due_operations(&sync_root, now).await.unwrap();
        assert!(
            bridge.db.list_operations_for_file(file_id).unwrap().is_empty(),
            "{at:?}: landed once cured"
        );
        let state = server.finish();
        assert_eq!(state.latest_plaintext(file_id, master_key), b"saved at the limit");
    }

    #[tokio::test]
    async fn a_finder_write_over_quota_at_init_lands_once_space_is_free() {
        a_curable_refusal_lands_once_cured([143u8; 32], "c1000000-0000-4000-8000-000000000001", Refused::Init, quota_exceeded()).await;
    }

    #[tokio::test]
    async fn a_finder_write_over_quota_at_a_chunk_lands_once_space_is_free() {
        a_curable_refusal_lands_once_cured([159u8; 32], "c1000000-0000-4000-8000-000000000002", Refused::Chunk, quota_exceeded()).await;
    }

    #[tokio::test]
    async fn a_finder_write_over_quota_at_complete_lands_once_space_is_free() {
        a_curable_refusal_lands_once_cured([160u8; 32], "c1000000-0000-4000-8000-000000000003", Refused::Complete, quota_exceeded()).await;
    }

    #[tokio::test]
    async fn a_finder_write_over_the_object_budget_lands_once_objects_are_freed() {
        a_curable_refusal_lands_once_cured([161u8; 32], "c1000000-0000-4000-8000-000000000004", Refused::Chunk, object_budget_exceeded()).await;
    }

    #[tokio::test]
    async fn a_finder_write_refused_while_billing_is_read_only_lands_once_paid() {
        a_curable_refusal_lands_once_cured([162u8; 32], "c1000000-0000-4000-8000-000000000005", Refused::Init, billing_read_only()).await;
    }

    #[tokio::test]
    async fn a_finder_write_refused_for_a_lapsed_account_lands_once_subscribed() {
        a_curable_refusal_lands_once_cured([163u8; 32], "c1000000-0000-4000-8000-000000000006", Refused::Init, account_lapsed()).await;
        a_curable_refusal_lands_once_cured([173u8; 32], "c1000000-0000-4000-8000-000000000007", Refused::Chunk, account_lapsed()).await;
        a_curable_refusal_lands_once_cured([174u8; 32], "c1000000-0000-4000-8000-000000000008", Refused::Complete, account_lapsed()).await;
    }

    #[tokio::test]
    async fn a_finder_write_refused_for_no_plan_lands_once_subscribed() {
        a_curable_refusal_lands_once_cured([175u8; 32], "c1000000-0000-4000-8000-000000000009", Refused::Init, plan_required()).await;
        a_curable_refusal_lands_once_cured([176u8; 32], "c1000000-0000-4000-8000-00000000000a", Refused::Chunk, plan_required()).await;
        a_curable_refusal_lands_once_cured([177u8; 32], "c1000000-0000-4000-8000-00000000000b", Refused::Complete, plan_required()).await;
    }

    #[tokio::test]
    async fn a_finder_write_refused_after_a_cancelled_trial_lands_once_subscribed() {
        a_curable_refusal_lands_once_cured(
            [178u8; 32],
            "c1000000-0000-4000-8000-00000000000c",
            Refused::Init,
            trial_cancelled_read_only(),
        )
        .await;
        a_curable_refusal_lands_once_cured(
            [179u8; 32],
            "c1000000-0000-4000-8000-00000000000d",
            Refused::Chunk,
            trial_cancelled_read_only(),
        )
        .await;
        a_curable_refusal_lands_once_cured(
            [180u8; 32],
            "c1000000-0000-4000-8000-00000000000e",
            Refused::Complete,
            trial_cancelled_read_only(),
        )
        .await;
    }

    /// Any other 413, 403 or 409 keeps its rule: the code decides, not the status. And an upload
    /// without a write id keeps its rules for the curable codes too.
    #[tokio::test]
    async fn other_refusals_and_uploads_without_a_write_id_keep_their_rules() {
        let (dir, sync_root, server, bridge) = rule3_setup([164u8; 32]);
        for (file_id, refusal) in [
            (
                "c1000000-0000-4000-8000-00000000000f",
                ("413 Payload Too Large", json!({ "error": "payload_too_large" })),
            ),
            ("c1000000-0000-4000-8000-000000000010", trial_ended()),
            ("c1000000-0000-4000-8000-000000000011", ("403 Forbidden", json!({ "error": "ba168ca1-ec2b-5945-8692-4a58b11c325a" }))),
        ] {
            seed_uploaded_row(&bridge, &server, file_id);
            refuse(&server, Refused::Init, file_id, refusal);
            fp_save(&bridge, dir.path(), file_id, "notes.txt", b"a save", "1");
        }
        // Past the backoff's cap, so a counted failure (one per pass) and a transient one (stops
        // at the cap) differ.
        let passes = backoff_cap_attempts() + 2;
        let mut now = now_secs();
        for _ in 0..passes {
            let outcome = bridge.process_due_operations(&sync_root, now).await.unwrap();
            now += 10_000;
            assert!(outcome.paused_op_ids.is_empty(), "{outcome:?}");
        }
        for file_id in ["c1000000-0000-4000-8000-00000000000f", "c1000000-0000-4000-8000-000000000010"] {
            let op = bridge.db.list_operations_for_file(file_id).unwrap().remove(0);
            assert_eq!(op.attempts, passes, "{file_id}: counted, once per pass");
        }
        let forbidden = bridge.db.list_operations_for_file("c1000000-0000-4000-8000-000000000011").unwrap().remove(0);
        assert_eq!(
            forbidden.last_error.as_deref(),
            Some("permission_denied"),
            "a 403 with another code parks at once"
        );
        drop(server.finish());

        // Without a write id: a quota refusal uses up an attempt and a billing refusal pauses,
        // as before.
        for (file_id, refusal, paused) in [
            ("c1000000-0000-4000-8000-000000000012", quota_exceeded(), false),
            ("c1000000-0000-4000-8000-000000000013", billing_read_only(), true),
        ] {
            let (dir, sync_root, server, bridge) = rule3_setup([165u8; 32]);
            seed_uploaded_row(&bridge, &server, file_id);
            refuse(&server, Refused::Init, file_id, refusal);
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
            let outcome = bridge.process_due_operations(&sync_root, now_secs()).await.unwrap();
            let op = bridge.db.list_operations_for_file(file_id).unwrap().remove(0);
            assert_eq!(
                outcome.paused_op_ids.len(),
                usize::from(paused),
                "{file_id}: {outcome:?}"
            );
            assert_eq!(op.attempts, i64::from(!paused), "{file_id}");
            drop(server.finish());
        }
    }

    /// A 403 parks at once as `permission_denied`: read-only with its bytes kept, its row
    /// `Error`, and its id in the tick's signal. Before, it paused and showed `uploading`
    /// for good.
    #[tokio::test]
    async fn a_finder_write_the_server_forbids_parks_permission_denied() {
        let (dir, sync_root, server, bridge) = rule3_setup([144u8; 32]);
        seed_uploaded_row(&bridge, &server, "ba168ca1-ec2b-5945-8692-4a58b11c325a");
        server
            .state
            .lock()
            .unwrap()
            .refuse_init
            .insert("ba168ca1-ec2b-5945-8692-4a58b11c325a".into(), "403 Forbidden");
        fp_save(
            &bridge,
            dir.path(),
            "ba168ca1-ec2b-5945-8692-4a58b11c325a",
            "notes.txt",
            b"a save the server refuses",
            "1",
        );
        let now = now_secs();
        let mut outcome = None;
        let logs = capture_logs_async(async {
            outcome = Some(bridge.process_due_operations(&sync_root, now).await.unwrap());
        })
        .await;
        let outcome = outcome.unwrap();
        assert!(outcome.paused_op_ids.is_empty(), "never paused: {outcome:?}");
        assert_eq!(outcome.invalidated_item_ids, vec!["ba168ca1-ec2b-5945-8692-4a58b11c325a".to_string()]);
        let op = bridge.db.list_operations_for_file("ba168ca1-ec2b-5945-8692-4a58b11c325a").unwrap().remove(0);
        assert_eq!(op.attempts, op.max_attempts, "parked");
        assert_eq!(op.last_error.as_deref(), Some("permission_denied"));
        assert_eq!(
            std::fs::read(op.payload_path.as_deref().unwrap()).unwrap(),
            b"a save the server refuses",
            "its bytes are kept"
        );
        assert_eq!(
            bridge.db.get_file("ba168ca1-ec2b-5945-8692-4a58b11c325a").unwrap().unwrap().status,
            FileStatus::Error
        );
        let parked: Vec<&str> = logs
            .lines()
            .filter(|line| line.contains("upload parked with its bytes kept in the queue"))
            .collect();
        assert_eq!(parked.len(), 1, "{logs}");
        assert!(
            parked[0].contains("reason=\"permission_denied\"") && parked[0].contains(&op.op_id),
            "{logs}"
        );
        // Parked: the next pass sends nothing.
        bridge.process_due_operations(&sync_root, now + 10_000).await.unwrap();
        assert_eq!(server.finish().inits.len(), 1);
    }

