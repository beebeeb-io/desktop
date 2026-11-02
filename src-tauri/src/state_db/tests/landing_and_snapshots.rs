//! Tests for claims, parks, landings and snapshots, split out of `state_db.rs`.

use super::*;

    /// The key-replaced re-mark of an auth-paused Finder write keeps the write
    /// held. Its token still names the bytes the system holds; its op and its staged copy stay, with the copy's
    /// journal row; engine start finds nothing to unlink; and the auth resume never makes it due.
    #[test]
    fn a_key_replaced_hold_keeps_a_held_finder_write_and_its_staged_copy() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        db.upsert_file(&FileEntry {
            file_id: "held".into(),
            path: "a.txt".into(),
            status: FileStatus::Local,
            size_bytes: 1,
            modified_at: 1,
            content_hash: None,
            remote_updated_at: 1,
            parent_id: None,
            item_kind: ItemKind::File,
        })
        .unwrap();
        let mut contract = db.get_file_contract_state("held").unwrap().unwrap();
        contract.current_version = 2;
        db.set_file_contract_state(&contract).unwrap();
        let staged = dir.path().join("staged-copy");
        std::fs::write(&staged, b"the only copy of the save").unwrap();
        let staged_path = staged.to_string_lossy().into_owned();
        db.track_staged_payload(&staged_path, Some("/source"), false).unwrap();
        let AcceptOutcome::Queued { token, .. } = db
            .accept_finder_write(
                &FinderAccept {
                    retire_parked: None,
                    op_id: "op-held",
                    file_id: "held",
                    kind: FinderAcceptKind::Modify {
                        incoming_base: Some("2"),
                    },
                    parent_id: None,
                    target_path: Some("a.txt"),
                    metadata_json: "{}",
                    payload_path: &staged_path,
                    size_bytes: 25,
                    modified_at: 2,
                    backup_source_key: None,
                    now: 2,
                },
                &|_, _| Vec::new(),
            )
            .unwrap()
        else {
            panic!("the save is queued");
        };
        db.record_operation_pause("op-held", OperationPauseReason::Auth, Some("HTTP 401 Unauthorized"), 3)
            .unwrap();

        assert_eq!(db.hold_operations_paused_for_auth_as_key_replaced(500).unwrap(), 1);

        let presentation = db.item_presentation("held").unwrap().unwrap();
        assert_eq!(
            presentation.held.map(|held| held.token()),
            Some(token),
            "the write stays held under its token"
        );
        assert!(presentation.held_write_queued, "its op still carries the write");
        let op = db.get_operation("op-held").unwrap().expect("the op is kept");
        assert_eq!(op.payload_path.as_deref(), Some(staged_path.as_str()), "with its copy");
        let reason: Option<String> =
            db.0.lock()
                .unwrap()
                .query_row(
                    "SELECT paused_reason FROM operation_queue WHERE op_id = 'op-held'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
        assert_eq!(reason.as_deref(), Some("key_replaced"));
        assert_eq!(
            db.staged_payloads_for_signout().unwrap(),
            vec![(staged_path.clone(), Some("/source".to_string()), false)],
            "the copy's journal row stays, not released"
        );
        assert!(
            !db.engine_start_repair()
                .unwrap()
                .released_payloads
                .contains(&staged_path),
            "engine start finds nothing to unlink"
        );
        assert!(staged.exists(), "the staged copy is on disk");
        assert_eq!(db.resume_operations_paused_for_auth(600).unwrap(), 0);
        assert!(
            db.list_due_operations(i64::MAX).unwrap().is_empty(),
            "never resumed as if auth-paused"
        );
    }

    /// S2: a park or a pause written under an old claim,
    /// on a row that still exists, changes nothing. The claim id, not the op id alone,
    /// is the key.
    #[test]
    fn a_park_or_a_pause_under_a_stale_claim_changes_nothing() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        db.enqueue_operation(&queued("u1", OperationKind::UploadVersion, "f", Some("/staged/u1")))
            .unwrap();
        let ClaimOutcome::Claimed(old) = db.claim_operation("u1", 1).unwrap() else {
            panic!("claimable")
        };
        // The attempt under `old` ended, and a newer attempt claimed the op.
        assert!(
            db.record_attempt_claimed("u1", &old.claim_id, 1, 0, Some("boom"))
                .unwrap()
        );
        let ClaimOutcome::Claimed(current) = db.claim_operation("u1", 2).unwrap() else {
            panic!("claimable again")
        };
        assert_ne!(old.claim_id, current.claim_id);
        let before = db.get_operation("u1").unwrap().unwrap();

        assert!(
            !db.park_claimed("u1", &old.claim_id, ParkReason::StaleBase, 3).unwrap(),
            "a park under an old claim matches no row"
        );
        assert!(
            !db.record_pause_claimed("u1", &old.claim_id, OperationPauseReason::Quota, Some("quota"), 4)
                .unwrap(),
            "a pause under an old claim matches no row"
        );
        assert_eq!(
            db.get_operation("u1").unwrap().unwrap(),
            before,
            "the row is as the current claim left it"
        );
        assert_eq!(db.queue_diagnostics(i64::MAX).unwrap().paused, 0, "nothing paused");
        // The current claim still holds the op: its own outcome writes.
        assert!(
            db.park_claimed("u1", &current.claim_id, ParkReason::StaleBase, 5)
                .unwrap(),
            "the current claim is intact"
        );
    }

    /// A pass lists its ops once, so the claim skips an op that paused after the listing, as
    /// the listing itself would. It is claimable again once resumed.
    #[test]
    fn the_claim_skips_an_op_that_paused_after_the_pass_listed_it() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        db.enqueue_operation(&queued("u1", OperationKind::UploadVersion, "f", Some("/staged/u1")))
            .unwrap();
        assert_eq!(db.list_due_operations(i64::MAX).unwrap().len(), 1, "listed");
        db.record_operation_pause("u1", OperationPauseReason::Auth, Some("HTTP 401 Unauthorized"), 3)
            .unwrap();
        assert!(
            matches!(db.claim_operation("u1", 4).unwrap(), ClaimOutcome::Gone),
            "a paused op is not claimed"
        );
        let claim_id: Option<String> =
            db.0.lock()
                .unwrap()
                .query_row("SELECT claim_id FROM operation_queue WHERE op_id = 'u1'", [], |row| {
                    row.get(0)
                })
                .unwrap();
        assert_eq!(claim_id, None, "and holds no claim");
        assert_eq!(db.resume_operations_paused_for_auth(5).unwrap(), 1);
        assert!(
            matches!(db.claim_operation("u1", 6).unwrap(), ClaimOutcome::Claimed(_)),
            "claimable again once resumed"
        );
    }

    /// §8.6 rule 6: no park path parks an op whose completion is
    /// recorded. That covers the runner's park (the immediate stale-base arm, a missing
    /// payload, an unknown base), the snapshot count's park and the claim's parks (both
    /// `park_unclaimed_conn`), and the accept's park, which only ever parks the op it inserts.
    #[test]
    fn a_completed_op_is_never_parked() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed_own_row(&db, "f", FileStatus::Local, 10);
        let mut contract = db.get_file_contract_state("f").unwrap().unwrap();
        contract.current_version = 1;
        db.set_file_contract_state(&contract).unwrap();
        db.track_staged_payload("/staged/w", None, false).unwrap();
        let accept = |op_id: &str, base: &str| {
            db.accept_finder_write(
                &FinderAccept {
                    retire_parked: None,
                    op_id,
                    file_id: "f",
                    kind: FinderAcceptKind::Modify {
                        incoming_base: Some(base),
                    },
                    parent_id: None,
                    target_path: Some("f.txt"),
                    metadata_json: "{}",
                    payload_path: "/staged/w",
                    size_bytes: 3,
                    modified_at: 20,
                    backup_source_key: None,
                    now: 20,
                },
                &|_, _| Vec::new(),
            )
        };
        assert!(matches!(accept("w", "1").unwrap(), AcceptOutcome::Queued { .. }));
        let ClaimOutcome::Claimed(claimed) = db.claim_operation("w", 21).unwrap() else {
            panic!("claimable")
        };
        assert!(
            db.put_upload_resume_claimed(
                &UploadResume {
                    op_id: "w".into(),
                    payload_path: "/staged/w".into(),
                    payload_size: 3,
                    payload_mtime_ns: 1,
                    upload_session_id: "session-w".into(),
                    server_file_id: "f".into(),
                    object_version_id: "object-w".into(),
                    chunk_size_bytes: 3,
                    chunk_count: 1,
                    acked_chunks: 1,
                    metadata_applied: true,
                    is_create: false,
                    self_minted: false,
                    completed_version: None,
                    completed_object_version_id: None,
                    completed_mime_type: None,
                },
                &claimed.claim_id,
            )
            .unwrap()
        );
        assert!(
            db.record_completion_claimed("w", &claimed.claim_id, 2, "object-2", None)
                .unwrap()
        );
        let unparked = |why: &str| {
            let op = db.get_operation("w").unwrap().expect(why);
            assert!(op.attempts < op.max_attempts, "{why}: parked");
        };

        // The runner's park.
        for reason in [
            ParkReason::StaleBase,
            ParkReason::PayloadMissing,
            ParkReason::BaseUnknown,
        ] {
            assert!(
                !db.park_claimed("w", &claimed.claim_id, reason, 22).unwrap(),
                "the runner parked a completed op: {}",
                reason.as_str()
            );
            unparked(reason.as_str());
        }
        // The snapshot count's park, at the 10th snapshot without a base.
        db.0.lock()
            .unwrap()
            .execute("UPDATE operation_queue SET base_pending = 10 WHERE op_id = 'w'", [])
            .unwrap();
        assert!(
            db.note_snapshot_for_base_pending(23).unwrap().is_empty(),
            "the snapshot count parked a completed op"
        );
        unparked("the snapshot count");
        // The claim's parks: W names a predecessor that no op carries (S5).
        db.0.lock()
            .unwrap()
            .execute(
                "UPDATE operation_queue SET base_pending = 0, after_write_id = 'gone', claim_id = NULL
                 WHERE op_id = 'w'",
                [],
            )
            .unwrap();
        assert!(
            matches!(db.claim_operation("w", 24).unwrap(), ClaimOutcome::Claimed(_)),
            "claimed for its landing, never parked"
        );
        unparked("the claim");
        // The accept's park (rule 6a′) only parks the op it inserts: one naming W's op id fails.
        assert!(accept("w", "0").is_err(), "the accept never touches an existing op");
        unparked("the accept");
        assert_eq!(
            db.get_upload_resume("w").unwrap().unwrap().completed_version,
            Some(2),
            "the completion is kept"
        );
    }

    /// Across a mixed pair (one op with a write id and
    /// one without) the landing re-keys but never rebases.
    /// - (a) A Finder create lands while an op without a write id is queued under its
    ///   provisional id: that op moves to the server id, and its base stays `None`.
    /// - (b) An upload without a write id lands while a Finder save on the same base is queued:
    ///   the save keeps its base (the server then refuses it and it parks with its bytes),
    ///   while another op without a write id is rebased, as in round 3.
    ///
    /// The spec states this behaviour (§8.1, the mixed pair).
    #[test]
    fn a_landing_rekeys_a_mixed_pair_but_never_rebases_across_it() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        let rekey = |op: &PendingOperation, _: &str| -> std::result::Result<Option<String>, StateDbError> {
            Ok(op.metadata_json.clone())
        };
        let landing = |op_id: &'static str, claim_id: &'static str, write_id: Option<&'static str>| LandingInput {
            op_id,
            claim_id: Some(claim_id),
            write_id,
            local_file_id: "",
            server_file_id: "",
            target_path: None,
            parent_id: None,
            landed_base: None,
            produced_version: 0,
            produced_object_version_id: "",
            size_bytes: 3,
            content_type: None,
            mime_type: None,
            completed_payload: None,
            completed_payload_hash: None,
            finalization: None,
            now: 20,
        };

        // (a)
        seed_own_row(&db, "provisional", FileStatus::Uploading, 10);
        db.0.lock()
            .unwrap()
            .execute_batch(
                r#"INSERT INTO operation_queue (op_id, kind, file_id, metadata_json, base_version, write_id,
                                               write_origin, claim_id, attempts, max_attempts)
                   VALUES ('create', 'upload_version', 'provisional', '{"operation":"create_file"}', NULL,
                           'w-create', 'minted', 'claim-c', 0, 25),
                          ('plain', 'upload_version', 'provisional', '{"operation":"upload_version"}', NULL,
                           NULL, NULL, NULL, 0, 25);"#,
            )
            .unwrap();
        let landed = db
            .apply_landing(
                &LandingInput {
                    local_file_id: "provisional",
                    server_file_id: "server",
                    produced_version: 1,
                    produced_object_version_id: "object-1",
                    ..landing("create", "claim-c", Some("w-create"))
                },
                &rekey,
            )
            .unwrap()
            .expect("the claim holds");
        assert_eq!(landed.resolved_successors, 0);
        let plain = db.get_operation("plain").unwrap().unwrap();
        assert_eq!(
            plain.file_id.as_deref(),
            Some("server"),
            "(a) re-keyed to the server id"
        );
        assert_eq!(plain.base_version, None, "(a) not rebased across the mixed pair");

        // (b)
        seed_own_row(&db, "f", FileStatus::Uploading, 10);
        let mut contract = db.get_file_contract_state("f").unwrap().unwrap();
        contract.current_version = 1;
        db.set_file_contract_state(&contract).unwrap();
        db.0.lock()
            .unwrap()
            .execute_batch(
                "INSERT INTO operation_queue (op_id, kind, file_id, metadata_json, base_version, write_id,
                                              write_origin, claim_id, attempts, max_attempts)
                 VALUES ('x', 'upload_version', 'f', '{}', 1, NULL, NULL, 'claim-x', 0, 25),
                        ('n', 'upload_version', 'f', '{}', 1, 'w-n', 'minted', NULL, 0, 25),
                        ('y', 'upload_version', 'f', '{}', 1, NULL, NULL, NULL, 0, 25);",
            )
            .unwrap();
        let landed = db
            .apply_landing(
                &LandingInput {
                    local_file_id: "f",
                    server_file_id: "f",
                    landed_base: Some(1),
                    produced_version: 2,
                    produced_object_version_id: "object-2",
                    ..landing("x", "claim-x", None)
                },
                &rekey,
            )
            .unwrap()
            .expect("the claim holds");
        assert_eq!(landed.resolved_successors, 1, "(b) only the op without a write id");
        assert_eq!(
            db.get_operation("n").unwrap().unwrap().base_version,
            Some(1),
            "(b) the Finder save keeps its base"
        );
        assert_eq!(
            db.get_operation("y").unwrap().unwrap().base_version,
            Some(2),
            "(b) round 3's equal-base rule"
        );
    }

    /// Task 2204 S0.3 (review-155): the pull read the row (at `a.txt`) before a Windows rename
    /// committed its re-key; its write, which still names `a.txt` and the old parent, keeps the
    /// row where the rename put it.
    #[test]
    fn task_2204_a_pull_write_after_a_rename_keeps_the_new_path() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed_own_row(&db, "f", FileStatus::Local, 10);
        let stale = db.get_file("f").unwrap().unwrap();
        db.0.lock()
            .unwrap()
            .execute_batch(
                r#"INSERT INTO operation_queue (op_id, kind, file_id, target_path, metadata_json, attempts, max_attempts)
                   VALUES ('rename', 'move_file', 'f', 'b.txt', '{"local_rename":true}', 0, 25);
                   UPDATE files SET path = 'docs/b.txt', parent_id = 'docs' WHERE file_id = 'f';"#,
            )
            .unwrap();
        let written = db
            .upsert_server_file_row(&stale, |contract| contract.parent_id = None)
            .unwrap();
        assert_eq!(written, "docs/b.txt");
        assert_eq!(db.get_file("f").unwrap().unwrap().path, "docs/b.txt", "the rename's path stands");
        assert_eq!(
            db.get_file_contract_state("f").unwrap().unwrap().parent_id.as_deref(),
            Some("docs"),
            "and its parent"
        );
        db.remove_operation("rename").unwrap();
        db.upsert_server_file_row(&stale, |contract| contract.parent_id = None).unwrap();
        assert_eq!(db.get_file("f").unwrap().unwrap().path, "f.txt", "nothing queued: the server's row is written");
    }

    /// Task 2204 S0.2 (review-128 F4): a sync-folder create renamed before it landed keeps the
    /// row's path and parent, and its finalization journal row names the row's path, not the
    /// op's queue-time one. Only an op `handle_rename` queued (`local_rename`) holds them.
    #[test]
    fn task_2204_a_landing_keeps_a_renamed_row_s_path_and_journal() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed_own_row(&db, "provisional", FileStatus::Uploading, 10);
        let mut row = db.get_file("provisional").unwrap().unwrap();
        row.path = "docs/b.txt".into();
        db.upsert_file(&row).unwrap();
        let mut contract = db
            .get_file_contract_state("provisional")
            .unwrap()
            .unwrap_or_else(|| default_contract("provisional"));
        contract.parent_id = Some("docs-folder".into());
        db.set_file_contract_state(&contract).unwrap();
        db.0.lock()
            .unwrap()
            .execute_batch(
                r#"INSERT INTO operation_queue (op_id, kind, file_id, target_path, metadata_json, payload_path,
                                               claim_id, attempts, max_attempts)
                   VALUES ('create', 'upload_version', 'provisional', 'a.txt', '{"operation":"create_file"}',
                           '/staged/create', 'claim-c', 0, 25),
                          ('rename', 'move_file', 'provisional', 'b.txt',
                           '{"operation":"metadata_update","local_rename":true}', NULL, NULL, 0, 25);"#,
            )
            .unwrap();
        let finalization = UploadFinalization {
            op_id: "create".into(),
            local_file_id: "provisional".into(),
            server_file_id: "server".into(),
            target_path: "a.txt".into(),
            payload_path: "/staged/create".into(),
            stamped: false,
            review: None,
        };
        let input = LandingInput {
            op_id: "create",
            claim_id: Some("claim-c"),
            write_id: None,
            local_file_id: "provisional",
            server_file_id: "server",
            target_path: Some("a.txt"),
            parent_id: None,
            landed_base: None,
            produced_version: 1,
            produced_object_version_id: "object-1",
            size_bytes: 3,
            content_type: None,
            mime_type: None,
            completed_payload: Some("/staged/create"),
            completed_payload_hash: None,
            finalization: Some(&finalization),
            now: 20,
        };
        let rekey = |op: &PendingOperation, _: &str| -> std::result::Result<Option<String>, StateDbError> {
            Ok(op.metadata_json.clone())
        };
        db.apply_landing(&input, &rekey).unwrap().expect("the claim holds");
        let landed = db.get_file("server").unwrap().unwrap();
        assert_eq!(landed.path, "docs/b.txt", "the row stays where the file is");
        assert_eq!(
            db.get_file_contract_state("server").unwrap().unwrap().parent_id.as_deref(),
            Some("docs-folder"),
            "and in its folder"
        );
        let journal = db.upload_finalizations().unwrap();
        assert_eq!(journal.len(), 1);
        assert_eq!(journal[0].target_path, "docs/b.txt", "the journal names the row's path");
        assert_eq!(
            db.get_operation("rename").unwrap().unwrap().file_id.as_deref(),
            Some("server"),
            "the rename now names the server id"
        );
    }

    /// Windows: a create's finalization journal row is written in the
    /// landing's own transaction. Either the landing commits with its journal row, or nothing
    /// of it does and the op stays for the retry: here the journal write fails (a row already
    /// holds its key, as a full disk would fail it), and the landing rolls back whole.
    #[test]
    fn a_landing_journals_its_finalization_in_its_own_transaction() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed_own_row(&db, "provisional", FileStatus::Uploading, 10);
        db.0.lock()
            .unwrap()
            .execute_batch(
                r#"INSERT INTO operation_queue (op_id, kind, file_id, target_path, metadata_json, payload_path,
                                               claim_id, attempts, max_attempts)
                   VALUES ('create', 'upload_version', 'provisional', 'p.txt', '{"operation":"create_file"}',
                           '/staged/create', 'claim-c', 0, 25);
                   INSERT INTO upload_finalizations (op_id, local_file_id, server_file_id, target_path, payload_path)
                   VALUES ('create', 'other', 'other', 'other.txt', '/staged/other');"#,
            )
            .unwrap();
        db.put_upload_resume(&UploadResume {
            op_id: "create".into(),
            payload_path: "/staged/create".into(),
            payload_size: 3,
            payload_mtime_ns: 1,
            upload_session_id: "session-c".into(),
            server_file_id: "server".into(),
            object_version_id: "object-1".into(),
            chunk_size_bytes: 3,
            chunk_count: 1,
            acked_chunks: 1,
            metadata_applied: true,
            is_create: true,
            self_minted: false,
            completed_version: None,
            completed_object_version_id: None,
            completed_mime_type: None,
        })
        .unwrap();
        let finalization = UploadFinalization {
            op_id: "create".into(),
            local_file_id: "provisional".into(),
            server_file_id: "server".into(),
            target_path: "p.txt".into(),
            payload_path: "/staged/create".into(),
            stamped: false,
            review: None,
        };
        let input = LandingInput {
            op_id: "create",
            claim_id: Some("claim-c"),
            write_id: None,
            local_file_id: "provisional",
            server_file_id: "server",
            target_path: Some("p.txt"),
            parent_id: None,
            landed_base: None,
            produced_version: 1,
            produced_object_version_id: "object-1",
            size_bytes: 3,
            content_type: None,
            mime_type: None,
            completed_payload: Some("/staged/create"),
            completed_payload_hash: None,
            finalization: Some(&finalization),
            now: 20,
        };
        let rekey = |op: &PendingOperation, _: &str| -> std::result::Result<Option<String>, StateDbError> {
            Ok(op.metadata_json.clone())
        };

        assert!(db.apply_landing(&input, &rekey).is_err(), "the journal write fails");
        assert!(
            db.get_operation("create").unwrap().is_some(),
            "nothing of the landing committed: the op stays for the retry"
        );
        assert!(db.get_upload_resume("create").unwrap().is_some(), "with its resume row");
        assert!(
            db.get_file("provisional").unwrap().is_some(),
            "the provisional row stays"
        );
        assert!(db.get_file("server").unwrap().is_none(), "no server row");

        db.0.lock()
            .unwrap()
            .execute("DELETE FROM upload_finalizations WHERE op_id = 'create'", [])
            .unwrap();
        assert!(db.apply_landing(&input, &rekey).unwrap().is_some(), "the retry lands");
        assert_eq!(
            db.upload_finalizations().unwrap(),
            vec![finalization.clone()],
            "the journal row is in the landing's commit"
        );
        assert!(db.get_operation("create").unwrap().is_none());
        assert!(db.get_upload_resume("create").unwrap().is_none());
        assert!(db.get_file("server").unwrap().is_some());
    }

    #[test]
    fn prune_absent_keeps_shared_with_me_row_absent_from_snapshot() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        // A shared-with-me row (namespace != my_files) absent from the OWN-tree
        // snapshot must survive — shared content has its own purge path.
        seed_contract_row(&db, "shared", "/Shared with me/x.txt", None, FileStatus::CloudOnly, 0);
        let mut c = db.get_file_contract_state("shared").unwrap().unwrap();
        c.namespace = Namespace::SharedWithMe;
        c.shared_root_id = Some("shared".into());
        db.set_file_contract_state(&c).unwrap();

        // Own-tree snapshot does not mention the shared row.
        let seen: HashSet<String> = ["my-own".to_string()].into_iter().collect();
        let pruned = db.prune_absent(&seen, FAR_FUTURE).unwrap();
        assert!(pruned.is_empty());
        assert!(
            db.get_file("shared").unwrap().is_some(),
            "shared-with-me row never pruned by own-tree snapshot"
        );
    }

    #[test]
    fn prune_absent_keeps_row_touched_at_or_after_snapshot_fetch() {
        // Issue 5: a just-completed upload re-keyed to the server id is `Local`
        // with `remote_updated_at = now` and NO operation_queue row. A snapshot
        // taken before the completion must NOT prune it.
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        let fetched_at = 1_000_000;
        // Row stamped AT the fetch instant (the boundary) — must be protected.
        seed_own_row(&db, "just-uploaded", FileStatus::Local, fetched_at);
        // A genuinely older settled row that the snapshot omits — must be pruned.
        seed_own_row(&db, "old-deleted", FileStatus::CloudOnly, fetched_at - 100);

        let seen: HashSet<String> = ["something-else".to_string()].into_iter().collect();
        let pruned = db.prune_absent(&seen, fetched_at).unwrap();

        assert_eq!(
            pruned.iter().map(|r| r.file_id.clone()).collect::<Vec<_>>(),
            vec!["old-deleted".to_string()]
        );
        assert!(
            db.get_file("just-uploaded").unwrap().is_some(),
            "row stamped at/after snapshot fetch survives (upload re-key window)"
        );
    }

    #[test]
    fn prune_absent_refuses_empty_snapshot_against_non_empty_tree() {
        // Issue 4: an empty `seen` set against a non-empty prunable own-tree is a
        // suspected degraded snapshot — refuse to prune anything.
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed_own_row(&db, "a", FileStatus::CloudOnly, 10);
        seed_own_row(&db, "b", FileStatus::CloudOnly, 10);

        let pruned = db.prune_absent(&HashSet::new(), FAR_FUTURE).unwrap();
        assert!(pruned.is_empty(), "empty snapshot must not prune (fail-closed)");
        assert!(db.get_file("a").unwrap().is_some());
        assert!(db.get_file("b").unwrap().is_some());
    }

    #[test]
    fn prune_absent_empty_snapshot_with_only_protected_rows_is_a_noop() {
        // Empty snapshot is allowed to proceed when there are NO prunable
        // candidates (only protected rows): nothing is deleted, no false refusal
        // signal needed. Here the only own-tree row is uploading (protected).
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed_own_row(&db, "uploading-only", FileStatus::Uploading, 10);

        let pruned = db.prune_absent(&HashSet::new(), FAR_FUTURE).unwrap();
        assert!(pruned.is_empty());
        assert!(db.get_file("uploading-only").unwrap().is_some());
    }

    #[test]
    fn prune_absent_removes_orphaned_descendants_of_a_trashed_folder() {
        // task 0806/0807: the server trash is NOT recursive, so trashing a FOLDER
        // drops only the folder from the snapshot; its CHILDREN remain in `seen`.
        // prune_absent must still remove those orphans by PATH-PREFIX, ordered
        // children-before-parent.
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed_own_at(&db, "folder", "docs", true);
        seed_own_at(&db, "child", "docs/a.txt", false);
        seed_own_at(&db, "subfolder", "docs/sub", true);
        seed_own_at(&db, "grandchild", "docs/sub/b.txt", false);
        // A sibling that is NOT under the folder must survive.
        seed_own_at(&db, "outside", "top.txt", false);

        // Snapshot still lists the children + the sibling but NOT the folder.
        let seen: HashSet<String> = ["child", "subfolder", "grandchild", "outside"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let pruned = db.prune_absent(&seen, FAR_FUTURE).unwrap();

        let ids: HashSet<String> = pruned.iter().map(|r| r.file_id.clone()).collect();
        assert!(ids.contains("folder"), "the trashed folder is pruned");
        assert!(ids.contains("child"), "orphaned child pruned by path-prefix");
        assert!(ids.contains("subfolder"), "orphaned subfolder pruned");
        assert!(ids.contains("grandchild"), "orphaned grandchild pruned");
        assert!(!ids.contains("outside"), "a non-descendant sibling is NOT pruned");

        // Children precede their parent folder in the returned order (so the
        // Windows caller removes leaf placeholders before the directory).
        let pos = |id: &str| pruned.iter().position(|r| r.file_id == id).unwrap();
        assert!(pos("grandchild") < pos("subfolder"), "grandchild before its subfolder");
        assert!(pos("child") < pos("folder"), "child before the root folder");
        assert!(pos("subfolder") < pos("folder"), "subfolder before the root folder");

        assert!(db.get_file("folder").unwrap().is_none());
        assert!(db.get_file("child").unwrap().is_none());
        assert!(db.get_file("grandchild").unwrap().is_none());
        assert!(db.get_file("outside").unwrap().is_some(), "sibling row survives");
    }

    #[test]
    fn prune_absent_prunes_descendants_when_folder_row_has_leading_slash() {
        // task 0806 review (high): on the degraded-decrypt path a ROOT-level folder
        // can be stored leading-slash-first (`/docs`) while its children are ALWAYS
        // stored leading-slash-free (`docs/a.txt`). Trimming only the trailing slash
        // would make the prefix `/docs/` miss `docs/a.txt` → orphan leak. The match
        // normalizes both ends, so the child is still pruned.
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed_own_at(&db, "folder", "/docs", true);
        seed_own_at(&db, "child", "docs/a.txt", false);

        // Snapshot lists the child but NOT the trashed folder.
        let seen: HashSet<String> = ["child"].iter().map(|s| s.to_string()).collect();
        let pruned = db.prune_absent(&seen, FAR_FUTURE).unwrap();

        let ids: HashSet<String> = pruned.iter().map(|r| r.file_id.clone()).collect();
        assert!(ids.contains("folder"), "the trashed leading-slash folder is pruned");
        assert!(
            ids.contains("child"),
            "leading-slash-free child of a leading-slash folder is still pruned"
        );
        assert!(db.get_file("child").unwrap().is_none(), "child row removed");
    }

    #[test]
    fn delete_file_subtree_prunes_descendants_when_folder_row_has_leading_slash() {
        // task 0806 review (high): same leading-slash mismatch on the OPS path
        // (`apply_sync_op` file_trash). A folder stored `/docs` must still sweep its
        // leading-slash-free child `docs/a.txt`.
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed_own_at(&db, "folder", "/docs", true);
        seed_own_at(&db, "child", "docs/a.txt", false);

        let removed = db.delete_file_subtree("folder").unwrap();
        let ids: HashSet<String> = removed.iter().map(|r| r.file_id.clone()).collect();
        assert!(ids.contains("folder"), "leading-slash folder removed");
        assert!(ids.contains("child"), "leading-slash-free child swept");
        // children-before-parent ordering still holds.
        let pos = |id: &str| removed.iter().position(|r| r.file_id == id).unwrap();
        assert!(pos("child") < pos("folder"), "child before its root folder");
        assert!(db.get_file("child").unwrap().is_none());
    }

    #[test]
    fn delete_file_subtree_removes_folder_and_descendants_children_first() {
        // The OPS reconcile path (`apply_sync_op` file_trash for a FOLDER) calls
        // this: the folder row + its path-prefix descendants are removed in one
        // transaction, children-before-parent.
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed_own_at(&db, "folder", "Projects", true);
        seed_own_at(&db, "f1", "Projects/report.txt", false);
        seed_own_at(&db, "sub", "Projects/deep", true);
        seed_own_at(&db, "f2", "Projects/deep/data.bin", false);
        // A path that merely SHARES a prefix string but is not a child
        // ("Projects2") must NOT be swept (prefix equality uses the `/` boundary).
        seed_own_at(&db, "decoy", "Projects2/x.txt", false);

        let removed = db.delete_file_subtree("folder").unwrap();
        let ids: HashSet<String> = removed.iter().map(|r| r.file_id.clone()).collect();
        assert_eq!(
            ids,
            ["folder", "f1", "sub", "f2"].iter().map(|s| s.to_string()).collect()
        );
        let pos = |id: &str| removed.iter().position(|r| r.file_id == id).unwrap();
        assert!(pos("f2") < pos("sub"), "deep file before its folder");
        assert!(pos("f1") < pos("folder"), "child file before the root folder");
        // Root folder is LAST (deepest-first ordering then the root appended).
        assert_eq!(removed.last().unwrap().file_id, "folder");

        assert!(db.get_file("folder").unwrap().is_none());
        assert!(db.get_file("f2").unwrap().is_none());
        assert!(
            db.get_file("decoy").unwrap().is_some(),
            "a sibling sharing only a prefix STRING (Projects2) must not be swept"
        );
    }

    #[test]
    fn delete_file_subtree_of_a_plain_file_removes_only_that_row() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed_own_at(&db, "lonefile", "a.txt", false);
        seed_own_at(&db, "other", "b.txt", false);

        let removed = db.delete_file_subtree("lonefile").unwrap();
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].file_id, "lonefile");
        assert!(!removed[0].is_dir);
        assert!(db.get_file("lonefile").unwrap().is_none());
        assert!(db.get_file("other").unwrap().is_some());

        // Unknown id → empty, no-op.
        assert!(db.delete_file_subtree("does-not-exist").unwrap().is_empty());
    }

    /// Spec §5.4 row 10: a restore reply without `version_number`
    /// still tells the system the content changed, and a snapshot fills the version.
    #[test]
    fn a_restore_reply_without_a_version_records_the_change_and_requests_a_snapshot() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed_restorable_row(&db, "restored", 2, "object-v2");
        #[cfg(target_os = "macos")]
        assert_eq!(
            db.peek_resnapshot_request().unwrap(),
            None,
            "no snapshot is pending before"
        );
        #[cfg(not(target_os = "macos"))]
        assert!(!db.take_needs_resnapshot().unwrap(), "no snapshot is pending before");
        let (_, anchor) = db.list_file_changes(None).unwrap().unwrap();

        db.apply_restore_response("restored", None, None).unwrap();

        let (changes, _) = db.list_file_changes(anchor.as_deref()).unwrap().unwrap();
        let modified = changes
            .iter()
            .filter(|change| change.file_id == "restored" && change.kind == FpChangeKind::Modified)
            .count();
        assert_eq!(modified, 1, "the system is told the content changed: {changes:?}");
        #[cfg(target_os = "macos")]
        assert!(
            db.peek_resnapshot_request().unwrap().is_some(),
            "a snapshot fills the version the reply left out"
        );
        #[cfg(not(target_os = "macos"))]
        assert!(
            db.take_needs_resnapshot().unwrap(),
            "a snapshot fills the version the reply left out"
        );
        assert_eq!(
            version_and_object(&db, "restored"),
            (2, Some("object-v2".to_string())),
            "nothing is guessed"
        );
    }

    /// The server's legacy restore branch answers no object version
    /// and keeps its own (VER:663-674), so the row keeps its own too.
    #[test]
    fn a_restore_reply_without_an_object_version_keeps_the_rows() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed_restorable_row(&db, "legacy", 2, "object-v2");

        db.apply_restore_response("legacy", Some(3), None).unwrap();
        assert_eq!(
            version_and_object(&db, "legacy"),
            (3, Some("object-v2".to_string())),
            "the legacy branch's reply keeps the row's object version"
        );

        db.apply_restore_response("legacy", Some(4), Some("object-v4")).unwrap();
        assert_eq!(
            version_and_object(&db, "legacy"),
            (4, Some("object-v4".to_string())),
            "a reply that names one replaces it"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_resnapshot_request_survives_a_request_made_while_it_runs() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        assert_eq!(db.peek_resnapshot_request().unwrap(), None);
        db.request_resnapshot().unwrap();
        let seen = db.peek_resnapshot_request().unwrap().expect("requested");
        db.request_resnapshot().unwrap(); // a request made during the bootstrap
        assert!(
            !db.clear_resnapshot_request(seen).unwrap(),
            "the newer request survives"
        );
        let again = db.peek_resnapshot_request().unwrap().expect("still requested");
        assert!(db.clear_resnapshot_request(again).unwrap());
        assert_eq!(db.peek_resnapshot_request().unwrap(), None);
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn needs_resnapshot_flag_is_take_once() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        // Unset → false.
        assert!(!db.take_needs_resnapshot().unwrap());
        // Set → next take returns true, then clears.
        db.request_resnapshot().unwrap();
        assert!(db.take_needs_resnapshot().unwrap(), "first take sees the request");
        assert!(!db.take_needs_resnapshot().unwrap(), "flag cleared after one take");
        // Idempotent set.
        db.request_resnapshot().unwrap();
        db.request_resnapshot().unwrap();
        assert!(db.take_needs_resnapshot().unwrap());
        assert!(!db.take_needs_resnapshot().unwrap());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn engine_start_requests_a_snapshot_for_version_zero_rows() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed_own_row(&db, "versioned", FileStatus::Local, 10);
        let mut contract = db.get_file_contract_state("versioned").unwrap().unwrap();
        contract.current_version = 4;
        db.set_file_contract_state(&contract).unwrap();
        assert!(!db.engine_start_repair().unwrap().resnapshot_requested);
        assert_eq!(db.peek_resnapshot_request().unwrap(), None);

        seed_own_row(&db, "version-zero", FileStatus::Local, 20); // learned from a legacy file_create op
        assert!(db.engine_start_repair().unwrap().resnapshot_requested);
        assert!(db.peek_resnapshot_request().unwrap().is_some());
    }

    /// The tick clears exactly the request value it read (spec §6.3.2). A shared helper, on
    /// every target: one request reads 1 from the counter (macOS) and from the flag (elsewhere).
    #[test]
    fn a_resnapshot_request_is_cleared_only_by_the_value_read() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        assert_eq!(db.peek_resnapshot_request().unwrap(), None);
        assert!(!db.clear_resnapshot_request(1).unwrap(), "nothing to clear");
        db.request_resnapshot().unwrap();
        assert_eq!(db.peek_resnapshot_request().unwrap(), Some(1));
        assert!(!db.clear_resnapshot_request(2).unwrap(), "another value clears nothing");
        assert_eq!(db.peek_resnapshot_request().unwrap(), Some(1));
        assert!(db.clear_resnapshot_request(1).unwrap());
        assert_eq!(db.peek_resnapshot_request().unwrap(), None);
    }

    /// §6.3.3: `base_pending` is 1 + the successful snapshots that did not give the op its
    /// base; the 10th parks it `base_unknown` through the claim's park statement, and it
    /// asks for no more. A shared helper: it runs on every target.
    #[test]
    fn base_pending_counts_snapshots_and_parks_at_the_tenth() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        assert!(!db.has_base_pending_uploads().unwrap());
        db.0.lock()
            .unwrap()
            .execute_batch(
                "INSERT INTO operation_queue (op_id, kind, file_id, write_id, write_origin, base_version,
                                              base_pending, attempts, max_attempts)
                 VALUES ('waiting', 'upload_version', 'f1', 'w1', 'minted', 0, 1, 0, 25),
                        ('known', 'upload_version', 'f2', 'w2', 'minted', 3, 0, 0, 25);",
            )
            .unwrap();
        assert!(db.has_base_pending_uploads().unwrap());
        for snapshot in 1..10 {
            assert!(
                db.note_snapshot_for_base_pending(100 + snapshot).unwrap().is_empty(),
                "not parked after {snapshot} snapshots"
            );
        }
        assert_eq!(
            db.finder_write("waiting").unwrap().unwrap().base_pending,
            10,
            "1 + nine misses"
        );
        assert_eq!(
            db.note_snapshot_for_base_pending(200).unwrap(),
            vec![("waiting".to_string(), Some("f1".to_string()))]
        );
        let parked = db.get_operation("waiting").unwrap().unwrap();
        assert_eq!(parked.attempts, parked.max_attempts, "parked at the 10th");
        assert_eq!(parked.last_error.as_deref(), Some("base_unknown"));
        let class: Option<String> =
            db.0.lock()
                .unwrap()
                .query_row(
                    "SELECT last_error_class FROM operation_queue WHERE op_id = 'waiting'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
        assert_eq!(
            class.as_deref(),
            Some("base_unknown"),
            "the claim's park statement: a save on its token takes over and parks the same way"
        );
        assert_eq!(db.finder_write("waiting").unwrap().unwrap().base_pending, 0);
        assert!(
            !db.has_base_pending_uploads().unwrap(),
            "a parked op asks for no more snapshots"
        );
        assert_eq!(
            db.get_operation("known").unwrap().unwrap().attempts,
            0,
            "an op with a known base is not counted"
        );
        assert!(db.note_snapshot_for_base_pending(300).unwrap().is_empty());
    }

    /// §6.3.2: the fill and the raise leave Conflict and Trashing rows alone, and on a row
    /// with a queued write the raise changes the version only (the size is that write's).
    /// A shared helper: it runs on every target.
    #[test]
    fn the_snapshot_version_skips_conflict_and_trashing_rows_and_keeps_a_queued_writes_size() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        let at = |file_id: &str, status: FileStatus, version: i64| {
            seed_own_row(&db, file_id, status, 10);
            let mut contract = db.get_file_contract_state(file_id).unwrap().unwrap();
            contract.current_version = version;
            contract.current_object_version_id = Some(format!("object-{file_id}"));
            db.set_file_contract_state(&contract).unwrap();
        };
        at("conflicted", FileStatus::Conflict, 1);
        at("trashing", FileStatus::Trashing, 0);
        at("queued", FileStatus::Uploading, 1);
        at("plain", FileStatus::Local, 1);
        db.enqueue_operation(&PendingOperation {
            op_id: "op-queued".into(),
            kind: OperationKind::UploadVersion,
            file_id: Some("queued".into()),
            parent_id: None,
            target_path: Some("queued.txt".into()),
            metadata_json: Some("{}".into()),
            payload_path: None,
            base_version: Some(1),
            base_object_version_id: None,
            attempts: 0,
            max_attempts: 25,
            next_retry_at: 0,
            last_error: None,
            backup_source_key: None,
            created_at: 0,
            updated_at: 0,
        })
        .unwrap();
        db.set_write_id_for_test("op-queued", "w-queued");

        let version = |file_id: &str| {
            let contract = db.get_file_contract_state(file_id).unwrap().unwrap();
            let size = db.get_file(file_id).unwrap().unwrap().size_bytes;
            (contract.current_version, size, contract.current_object_version_id)
        };
        assert_eq!(
            db.apply_snapshot_version("conflicted", 3, 99, false).unwrap(),
            SnapshotVersion::Unchanged
        );
        assert_eq!(
            version("conflicted"),
            (1, 1, Some("object-conflicted".into())),
            "a Conflict row keeps its own"
        );
        assert_eq!(
            db.apply_snapshot_version("trashing", 1, 99, false).unwrap(),
            SnapshotVersion::Unchanged
        );
        assert_eq!(version("trashing").0, 0, "a Trashing row is leaving");
        assert_eq!(
            db.apply_snapshot_version("queued", 3, 99, false).unwrap(),
            SnapshotVersion::Raised { old: 1, new: 3 }
        );
        assert_eq!(
            version("queued"),
            (3, 1, Some("object-queued".into())),
            "the queued write keeps its size and object id"
        );
        assert_eq!(
            db.apply_snapshot_version("plain", 3, 99, false).unwrap(),
            SnapshotVersion::Raised { old: 1, new: 3 }
        );
        assert_eq!(
            version("plain"),
            (3, 99, None),
            "the node's size; the old object id is not current"
        );
    }

    /// The engine-start check reads op metadata as JSON: one malformed row must not
    /// fail the start repair (its claims would stay set), as the accept's read does not.
    #[cfg(target_os = "macos")]
    #[test]
    fn engine_start_survives_an_op_with_malformed_metadata() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed_own_row(&db, "version-zero", FileStatus::Local, 20);
        db.enqueue_operation(&PendingOperation {
            op_id: "op-malformed".into(),
            kind: OperationKind::UploadVersion,
            file_id: Some("version-zero".into()),
            parent_id: None,
            target_path: Some("version-zero.txt".into()),
            metadata_json: Some("{not json".into()),
            payload_path: None,
            base_version: None,
            base_object_version_id: None,
            attempts: 0,
            max_attempts: 25,
            next_retry_at: 0,
            last_error: None,
            backup_source_key: None,
            created_at: 0,
            updated_at: 0,
        })
        .unwrap();
        let repair = db
            .engine_start_repair()
            .expect("a malformed op never fails engine start");
        assert!(
            repair.resnapshot_requested,
            "the version-0 row still asks for a snapshot"
        );
    }
