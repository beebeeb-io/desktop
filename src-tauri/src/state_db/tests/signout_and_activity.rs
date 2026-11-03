//! Tests for sign-out, activity feeds, queue diagnostics and parked work, split out of `state_db.rs`.

use super::*;

    #[test]
    fn round7_finalization_survives_restart_and_blocks_unfinished_signout() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        let db = StateDb::open(&path).unwrap();
        let row = UploadFinalization {
            op_id: "op".into(),
            local_file_id: "local".into(),
            server_file_id: "server".into(),
            target_path: "file".into(),
            payload_path: "proof".into(),
            stamped: false,
            review: None,
        };
        db.put_upload_finalization(&row).unwrap();
        assert_eq!(db.upload_finalizations().unwrap(), vec![row.clone()]);
        db.forget_upload_finalization("op").unwrap();
        assert_eq!(
            db.upload_finalizations().unwrap().len(),
            1,
            "unstamped proof must not be forgotten"
        );
        db.forget_staged_payload("proof").unwrap();
        assert!(
            db.finish_windows_signout().is_err(),
            "finalization owns its proof independently of staging"
        );
        db.mark_upload_finalization_stamped("op").unwrap();
        drop(db);
        let db = StateDb::open(&path).unwrap();
        assert!(db.upload_finalizations().unwrap()[0].stamped);
        db.forget_upload_finalization("op").unwrap();
        db.finish_windows_signout().unwrap();
        assert_eq!(db.upload_finalizations().unwrap().len(), 0);
    }

    /// Task 2270 (spec U1): a finalization held for review is one group of the tray's summary, dated by when it was
    /// held; a pending one is not counted, and the queue's own counts are untouched.
    #[test]
    fn queue_hold_summary_counts_review_rows_as_not_marked_synced() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        for op in ["a", "b", "c"] {
            db.put_upload_finalization(&finalization(op)).unwrap();
        }
        db.mark_upload_finalization_review("a", "other_file", 100).unwrap();
        db.mark_upload_finalization_review("b", "moved", 200).unwrap();

        let summary = db.queue_hold_summary().unwrap();

        assert_eq!((summary.waiting, summary.gave_up), (0, 0));
        assert_eq!(
            summary.held,
            vec![HeldGroup { reason: "not_marked_synced".into(), count: 2, oldest_at: 100, newest_at: 200 }]
        );
        let rows = db.upload_finalizations().unwrap();
        let review = |op: &str| rows.iter().find(|r| r.op_id == op).unwrap().review.clone();
        assert_eq!((review("a"), review("b"), review("c")), (Some("other_file".into()), Some("moved".into()), None));
    }

    /// Task 2270 (spec X1, I5; self-review of #195): a row held for review does not stop sign-out from finishing, and
    /// goes with the account data in the same transaction. A pending row still refuses it, and a refused sign-out
    /// keeps the row in review as well.
    #[test]
    fn finishing_sign_out_forgets_review_rows_and_a_pending_row_refuses_it() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        for op in ["held", "pending"] {
            db.put_upload_finalization(&finalization(op)).unwrap();
            // The purge unlinks every staged payload before it finishes.
            db.forget_staged_payload(&format!("proof-{op}")).unwrap();
        }
        db.mark_upload_finalization_review("held", "partly_on_disk", 7).unwrap();

        assert!(db.finish_windows_signout().is_err(), "a pending row refuses it");
        let ops: Vec<String> = db.upload_finalizations().unwrap().into_iter().map(|r| r.op_id).collect();
        assert_eq!(ops, ["held", "pending"], "a refused sign-out keeps both");

        db.mark_upload_finalization_review("pending", "moved", 8).unwrap();
        db.finish_windows_signout().expect("rows in review do not refuse it");
        assert!(db.upload_finalizations().unwrap().is_empty(), "and go with the account data");
    }

    /// Task 2270 (spec M1): a 0.8.19 database (no review columns) gains them on open, and its rows read as not held.
    #[test]
    fn a_state_db_without_the_review_columns_gains_them() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("state.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE upload_finalizations (
                    op_id TEXT PRIMARY KEY,
                    local_file_id TEXT NOT NULL,
                    server_file_id TEXT NOT NULL,
                    target_path TEXT NOT NULL,
                    payload_path TEXT NOT NULL,
                    stamped INTEGER NOT NULL DEFAULT 0
                );
                INSERT INTO upload_finalizations VALUES ('old', 'l', 's', 't', 'p', 0);",
            )
            .unwrap();
        }
        let db = StateDb::open(&path).unwrap();
        let rows = db.upload_finalizations().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].review, None);
        db.mark_upload_finalization_review("old", "unreadable", 5).unwrap();
        assert_eq!(db.upload_finalizations().unwrap()[0].review.as_deref(), Some("unreadable"));
    }

    // 2026-10-07 (task 1834, R10): renamed from `round5_legacy_resume_inventory_survives_operation_purge`
    // and its assertion changed. It used to require that `purge_all_local_state` KEEP the staged-payload
    // journal row. R10 (spec 2026-10-06 §5.6, lead ruling 4 "staged_payloads rows are included in the
    // purge") makes the purge forget the row in the same transaction and hand the file back for deletion
    // instead. What the test protects is unchanged: a legacy payload whose only durable owner is the
    // resume row is never silently lost. It now holds because the purge RETURNS that path, which the
    // caller (`purge_local_state_files`) deletes through the one disposable-path gate.
    #[test]
    fn round5_legacy_resume_inventory_is_handed_back_by_the_operation_purge() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("state.db");
        let db = StateDb::open(&path).unwrap();
        // Mimic an old database: no staging journal row, no queued owner.
        db.0.lock().unwrap().execute_batch("INSERT INTO upload_resume
            (op_id,payload_path,payload_size,payload_mtime_ns,upload_session_id,server_file_id,object_version_id,chunk_size_bytes,chunk_count)
            VALUES ('old','only-copy',1,0,'session','file','version',1,1)").unwrap();
        drop(db);
        let db = StateDb::open(&path).unwrap();
        assert_eq!(
            db.staged_payloads_for_signout().unwrap(),
            vec![("only-copy".into(), None, false)],
            "the journal owns it after the upgrade"
        );
        let purge = db.purge_all_local_state().unwrap();
        assert!(db.get_upload_resume("old").unwrap().is_none());
        assert!(
            purge.payload_paths.contains(&"only-copy".to_string()),
            "old payload lost its only durable owner on resume purge without being handed back: {:?}",
            purge.payload_paths
        );
        assert!(
            db.staged_payloads_for_signout().unwrap().is_empty(),
            "and the journal row goes with it"
        );
    }

    /// Desktop 0.8.15 (deliverable 3): changes held back (on Guus's PC, 2,832 folder creates and
    /// 92 versions paused for `auth` since 4 Oct) are counted as on hold. They are never transfer
    /// work, so they cannot keep the engine at "syncing"; they are not resumed here either.
    #[test]
    fn desktop_0815_changes_on_hold_are_counted_and_are_never_syncing() {
        let db = super::StateDb::open(":memory:").unwrap();
        db.0.lock()
            .unwrap()
            .execute_batch(
                "INSERT INTO operation_queue (op_id,kind,paused_reason,created_at,updated_at) VALUES
                   ('a','create_folder','auth',1,1), ('b','upload_version','auth',1,1), ('c','upload_version',NULL,1,1);",
            )
            .unwrap();
        assert_eq!(db.count_operations_on_hold().unwrap(), 2);
        assert_eq!(db.transfer_backlog(i64::MAX).unwrap().upload_files, 1, "only the change not on hold is transfer work");
        let still_paused: i64 = db
            .0
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM operation_queue WHERE paused_reason = 'auth'", [], |row| row.get(0))
            .unwrap();
        assert_eq!(still_paused, 2, "counting resumes nothing");
    }

    #[test]
    fn windows_signout_refuses_paused_pending_bytes_without_mutation() {
        let db = super::StateDb::open(":memory:").unwrap();
        db.0.lock().unwrap().execute_batch(
            "INSERT INTO operation_queue (op_id,kind,payload_path,paused_reason,created_at,updated_at) VALUES ('a','upload_file','only-copy','review',1,1);").unwrap();
        assert!(db.windows_signout_preflight().is_err());
        assert!(db.finish_windows_signout().is_err());
        let remaining: i64 =
            db.0.lock()
                .unwrap()
                .query_row(
                    "SELECT COUNT(*) FROM operation_queue WHERE payload_path='only-copy'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
        assert_eq!(remaining, 1);
    }

    #[test]
    fn windows_signout_clears_all_account_rows_before_relogin_twice() {
        let db = super::StateDb::open(":memory:").unwrap();
        for account in ["a", "b"] {
            db.0.lock()
                .unwrap()
                .execute(
                    "INSERT INTO files(file_id,path,status) VALUES (?1,?1,'cloud_only')",
                    [account],
                )
                .unwrap();
            db.0.lock()
                .unwrap()
                .execute("INSERT INTO sync_state(key,value) VALUES ('cursor','12')", [])
                .unwrap();
            db.windows_signout_preflight().unwrap();
            db.finish_windows_signout().unwrap();
            assert_eq!(db.list_files().unwrap().len(), 0);
            let cursors: i64 =
                db.0.lock()
                    .unwrap()
                    .query_row("SELECT COUNT(*) FROM sync_state", [], |r| r.get(0))
                    .unwrap();
            assert_eq!(cursors, 0);
        }
    }

    #[test]
    fn windows_signout_refuses_conflicted_content_even_without_queue() {
        let db = super::StateDb::open(":memory:").unwrap();
        db.0.lock()
            .unwrap()
            .execute("INSERT INTO files(file_id,path,status) VALUES ('a','a','conflict')", [])
            .unwrap();
        assert!(db.windows_signout_preflight().is_err());
        assert_eq!(db.list_files().unwrap().len(), 1);
    }

    /// Task 2190 (glegion's constraint 1): a held edit is written once; the same reason and
    /// fingerprint again write nothing (a scan pass that finds it again writes no page); a new
    /// fingerprint writes again, and the first detection's time is kept.
    #[test]
    fn task_2190_a_held_edit_is_written_once_per_fingerprint() {
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        db.upsert_file(&held_row("h1", "a.txt")).unwrap();
        assert!(db.hold_unsent_edit("h1", "base_unknown", "fp1", 10).unwrap(), "the first detection writes");
        assert!(!db.hold_unsent_edit("h1", "base_unknown", "fp1", 20).unwrap(), "unchanged: nothing written");
        assert!(db.hold_unsent_edit("h1", "base_unknown", "fp2", 30).unwrap(), "a new fingerprint writes");
        assert_eq!(
            db.list_held_edits().unwrap(),
            vec![("h1".to_string(), "a.txt".to_string(), "base_unknown".to_string(), 10)]
        );
        assert!(db.release_unsent_edit("h1").unwrap());
        assert!(!db.release_unsent_edit("h1").unwrap(), "nothing left to release");
    }

    /// Task 2190 (glegion's constraint 2): the paths that skip the classify clear the record, so no
    /// feed entry goes stale: the row's delete, its trash, a rename or move (its path), any status
    /// away from `local`, and a re-key of its id. A write that changes none of them keeps it.
    #[test]
    fn task_2190_a_held_edit_goes_with_its_row_path_status_and_id() {
        let held = |db: &StateDb| -> i64 {
            db.0.lock().unwrap().query_row("SELECT COUNT(*) FROM held_edits", [], |row| row.get(0)).unwrap()
        };
        for (case, sql, left) in [
            ("delete", "DELETE FROM files WHERE file_id = 'h1'", 0),
            ("trash", "UPDATE files SET status = 'trashing' WHERE file_id = 'h1'", 0),
            ("rename or move", "UPDATE files SET path = 'docs/b.txt' WHERE file_id = 'h1'", 0),
            ("uploading", "UPDATE files SET status = 'uploading' WHERE file_id = 'h1'", 0),
            ("re-key", "UPDATE files SET file_id = 'h2' WHERE file_id = 'h1'", 0),
            ("size only", "UPDATE files SET size_bytes = 5 WHERE file_id = 'h1'", 1),
            ("same path", "UPDATE files SET path = 'a.txt' WHERE file_id = 'h1'", 1),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let db = StateDb::open(dir.path().join("state.db")).unwrap();
            db.upsert_file(&held_row("h1", "a.txt")).unwrap();
            db.hold_unsent_edit("h1", "base_unknown", "fp", 1).unwrap();
            db.test_execute(sql).unwrap();
            assert_eq!(held(&db), left, "{case}");
        }
    }

    /// Task 2190 (glegion's constraint 3): the feed's read of held edits is an index read: the
    /// held rows by `idx_held_edits_detected`, each file by its key, and `files` never scanned.
    #[test]
    fn task_2190_the_feed_reads_held_edits_by_index() {
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        let conn = db.0.lock().unwrap();
        let mut stmt = conn.prepare(&format!("EXPLAIN QUERY PLAN {LIST_HELD_EDITS_SQL}")).unwrap();
        let plan: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(3))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert!(plan.iter().any(|step| step.contains("USING INDEX idx_held_edits_detected")), "{plan:?}");
        assert!(plan.iter().any(|step| step.starts_with("SEARCH file")), "each file by its key: {plan:?}");
        assert!(!plan.iter().any(|step| step.starts_with("SCAN file")), "files never scanned: {plan:?}");
    }

    #[test]
    fn test_upsert_and_get_file() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        let entry = FileEntry {
            file_id: "abc123".to_string(),
            path: "/test/file.txt".to_string(),
            status: FileStatus::CloudOnly,
            size_bytes: 1024,
            modified_at: 1700000000,
            content_hash: None,
            remote_updated_at: 1700000000,
            parent_id: None,
            item_kind: ItemKind::File,
        };
        db.upsert_file(&entry).unwrap();
        let got = db.get_file("abc123").unwrap().unwrap();
        assert_eq!(got.status, FileStatus::CloudOnly);
        assert_eq!(got.size_bytes, 1024);
        assert_eq!(got.remote_updated_at, 1700000000);
    }

    #[test]
    fn local_activity_round_trips_newest_first() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();

        db.record_local_activity(LocalActivityEventInput {
            event_type: LocalActivityKind::MovedToTrash,
            file_id: Some("file-1".into()),
            file_name: "report.pdf".into(),
            rel_path: Some("docs/report.pdf".into()),
            occurred_at: 10,
        })
        .unwrap();
        db.record_local_activity(LocalActivityEventInput {
            event_type: LocalActivityKind::Restored,
            file_id: Some("file-2".into()),
            file_name: "notes.txt".into(),
            rel_path: Some("notes.txt".into()),
            occurred_at: 20,
        })
        .unwrap();

        let events = db.list_recent_local_activity(10).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event_type, LocalActivityKind::Restored);
        assert_eq!(events[0].file_id.as_deref(), Some("file-2"));
        assert_eq!(events[0].file_name, "notes.txt");
        assert_eq!(events[0].rel_path.as_deref(), Some("notes.txt"));
        assert_eq!(events[0].occurred_at, 20);
        assert_eq!(events[1].event_type, LocalActivityKind::MovedToTrash);
        assert_eq!(events[1].file_name, "report.pdf");
    }

    #[test]
    fn local_activity_prunes_to_cap() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();

        for i in 0..(LOCAL_ACTIVITY_MAX_ROWS + 5) {
            db.record_local_activity(LocalActivityEventInput {
                event_type: LocalActivityKind::MovedToTrash,
                file_id: Some(format!("file-{i}")),
                file_name: format!("file-{i}.txt"),
                rel_path: Some(format!("file-{i}.txt")),
                occurred_at: i as i64,
            })
            .unwrap();
        }

        let events = db.list_recent_local_activity(LOCAL_ACTIVITY_MAX_ROWS + 10).unwrap();
        assert_eq!(events.len(), LOCAL_ACTIVITY_MAX_ROWS);
        assert_eq!(events[0].occurred_at, (LOCAL_ACTIVITY_MAX_ROWS + 4) as i64);
        assert_eq!(events.last().unwrap().occurred_at, 5);
    }

    #[test]
    fn transfer_activity_round_trips_newest_first_with_its_direction() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        db.record_transfer_activity(transfer("up", "a.txt", 10)).unwrap();
        db.record_transfer_activity(transfer("down", "b.txt", 20)).unwrap();
        let rows = db.list_recent_transfer_activity(10).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(
            (rows[0].file_name.as_str(), rows[0].direction.as_str()),
            ("b.txt", "down")
        );
        assert_eq!(
            (rows[1].file_name.as_str(), rows[1].direction.as_str()),
            ("a.txt", "up")
        );
        assert_eq!(rows[0].rel_path.as_deref(), Some("Work/b.txt"));
        assert_eq!(db.list_recent_transfer_activity(1).unwrap().len(), 1);
        assert!(db.list_recent_transfer_activity(0).unwrap().is_empty());
    }

    #[test]
    fn transfer_activity_prunes_to_its_own_cap_and_leaves_local_activity_alone() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        db.record_local_activity(LocalActivityEventInput {
            event_type: LocalActivityKind::MovedToTrash,
            file_id: Some("t".into()),
            file_name: "trashed.txt".into(),
            rel_path: None,
            occurred_at: 1,
        })
        .unwrap();
        for i in 0..(TRANSFER_ACTIVITY_MAX_ROWS + 7) {
            db.record_transfer_activity(transfer("up", &format!("f{i}.txt"), i as i64))
                .unwrap();
        }
        // Count the table itself: the read path also caps its own result, which would hide a
        // write path that forgot to prune.
        let stored: i64 =
            db.0.lock()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM transfer_activity", [], |row| row.get(0))
                .unwrap();
        assert_eq!(stored, TRANSFER_ACTIVITY_MAX_ROWS as i64, "107 written, 100 kept");
        let rows = db
            .list_recent_transfer_activity(TRANSFER_ACTIVITY_MAX_ROWS + 50)
            .unwrap();
        assert_eq!(rows.len(), TRANSFER_ACTIVITY_MAX_ROWS);
        assert_eq!(rows[0].occurred_at, (TRANSFER_ACTIVITY_MAX_ROWS + 6) as i64);
        assert_eq!(rows.last().unwrap().occurred_at, 7);
        // A large sync must not push the trash event out of its own table.
        assert_eq!(db.list_recent_local_activity(10).unwrap().len(), 1);
    }

    #[test]
    fn the_backlog_counts_due_uploads_downloads_and_quota_pauses() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed_contract_row(&db, "f1", "A/one.bin", None, FileStatus::Uploading, 100);
        seed_contract_row(&db, "f2", "A/two.bin", None, FileStatus::Local, 200);
        seed_contract_row(&db, "f3", "A/three.bin", None, FileStatus::Local, 300);
        seed_contract_row(&db, "f4", "A/four.bin", None, FileStatus::Downloading, 50);
        seed_contract_row(&db, "f5", "A/five.bin", None, FileStatus::Local, 7);
        db.enqueue_operation(&queued_op("u1", OperationKind::UploadFile, "f1"))
            .unwrap();
        db.enqueue_operation(&queued_op("u2", OperationKind::UploadVersion, "f2"))
            .unwrap();
        db.enqueue_operation(&queued_op("u3", OperationKind::UploadFile, "f3"))
            .unwrap();
        db.enqueue_operation(&queued_op("r1", OperationKind::RenameFile, "f5"))
            .unwrap();
        db.record_operation_pause("u3", OperationPauseReason::Quota, Some("quota exceeded"), 5)
            .unwrap();

        let backlog = db.transfer_backlog(100).unwrap();
        assert_eq!(
            backlog.upload_files, 2,
            "u1 and u2 are due; u3 is paused; r1 is a rename"
        );
        assert_eq!(backlog.upload_bytes, 300, "100 + 200");
        assert_eq!((backlog.download_files, backlog.download_bytes), (1, 50));
        assert_eq!(backlog.queued_ops, 4, "every queued operation, paused or not");
        assert_eq!(backlog.paused_for_quota, 1);
    }

    #[test]
    fn an_upload_waiting_out_a_retry_backoff_is_not_a_file_left() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed_contract_row(&db, "f1", "one.bin", None, FileStatus::Uploading, 100);
        seed_contract_row(&db, "f2", "two.bin", None, FileStatus::Local, 5_000);
        db.enqueue_operation(&queued_op("u1", OperationKind::UploadFile, "f1"))
            .unwrap();
        // Failed once and backed off for ten minutes (from t=1000 to t=1600).
        let mut backing_off = queued_op("u2", OperationKind::UploadFile, "f2");
        backing_off.attempts = 1;
        backing_off.next_retry_at = 1_600;
        db.enqueue_operation(&backing_off).unwrap();

        let during = db.transfer_backlog(1_000).unwrap();
        assert_eq!(during.upload_files, 1, "only u1 is due; u2 waits for t=1600");
        assert_eq!(
            during.upload_bytes, 100,
            "the backed-off file's 5000 bytes are not in the total"
        );
        assert_eq!(during.queued_ops, 2, "it is still a queued operation");

        let at_the_retry_time = db.transfer_backlog(1_600).unwrap();
        assert_eq!(at_the_retry_time.upload_files, 2, "due again exactly at next_retry_at");
        assert_eq!(at_the_retry_time.upload_bytes, 5_100);
    }

    #[test]
    fn an_exhausted_or_auth_paused_upload_is_not_work_left() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed_contract_row(&db, "f1", "one.bin", None, FileStatus::Local, 10);
        seed_contract_row(&db, "f2", "two.bin", None, FileStatus::Local, 20);
        let mut exhausted = queued_op("u1", OperationKind::UploadFile, "f1");
        exhausted.attempts = 5;
        db.enqueue_operation(&exhausted).unwrap();
        db.enqueue_operation(&queued_op("u2", OperationKind::UploadFile, "f2"))
            .unwrap();
        db.record_operation_pause("u2", OperationPauseReason::Auth, None, 5)
            .unwrap();
        let backlog = db.transfer_backlog(100).unwrap();
        assert_eq!(backlog.upload_files, 0);
        assert_eq!(backlog.paused_for_quota, 0);
        assert_eq!(backlog.queued_ops, 2);
    }

    /// Lead ruling F9: after a sign-in, the operations paused for `auth` are due again at once with their attempts kept,
    /// and the ones paused for quota, permission or a lock stay paused.
    #[test]
    fn only_the_operations_paused_for_auth_are_due_again_with_their_attempts_kept() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        let mut auth = queued_op("a1", OperationKind::CreateFolder, "f-a");
        auth.attempts = 2;
        auth.next_retry_at = 10_000;
        db.enqueue_operation(&auth).unwrap();
        for (id, reason) in [
            ("a1", OperationPauseReason::Auth),
            ("q1", OperationPauseReason::Quota),
            ("p1", OperationPauseReason::Permission),
            ("l1", OperationPauseReason::Locked),
        ] {
            if id != "a1" {
                db.enqueue_operation(&queued_op(id, OperationKind::UploadFile, id))
                    .unwrap();
            }
            db.record_operation_pause(id, reason, Some("HTTP 401 Unauthorized"), 5)
                .unwrap();
        }
        let mut unpaused = queued_op("d1", OperationKind::UploadFile, "f-d");
        unpaused.created_at = 2; // listed after a1, which was queued first
        db.enqueue_operation(&unpaused).unwrap();
        let due = |now| {
            db.list_due_operations(now)
                .unwrap()
                .into_iter()
                .map(|op| (op.op_id, op.attempts, op.next_retry_at))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            due(500),
            vec![("d1".to_string(), 0, 0)],
            "precondition: only the unpaused one is due"
        );

        assert_eq!(db.resume_operations_paused_for_auth(500).unwrap(), 1);

        assert_eq!(
            due(500),
            vec![("a1".to_string(), 2, 500), ("d1".to_string(), 0, 0)],
            "the auth-paused one is due now, its attempts kept"
        );
        let diagnostics = db.queue_diagnostics(500).unwrap();
        assert_eq!(
            diagnostics.paused_by_reason,
            BTreeMap::from([
                ("locked".to_string(), 1),
                ("permission".to_string(), 1),
                ("quota".to_string(), 1)
            ]),
            "every other pause stays"
        );
        assert_eq!(
            db.resume_operations_paused_for_auth(600).unwrap(),
            0,
            "nothing left to resume"
        );
        assert_eq!(due(500).len(), 2, "and a second call moves nothing");
    }

    /// 0.8.15: the tray's and Settings › Sync's queue summary. Waiting (due or backing off) and given-up operations are
    /// counted apart; paused ones are grouped by reason with the dates they were queued; a reason this build does not
    /// know is `other`; nothing in it is a name, a path or an error text. It changes nothing.
    #[test]
    fn queue_hold_summary_counts_waiting_given_up_and_held_by_reason_without_names() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        for (id, created_at) in [("a1", 1_759_579_200), ("a2", 1_759_586_400)] {
            let mut op = queued_op(id, OperationKind::CreateFolder, id);
            op.created_at = created_at;
            db.enqueue_operation(&op).unwrap();
            db.record_operation_pause(id, OperationPauseReason::Auth, Some("HTTP 401 for secret-folder"), 5)
                .unwrap();
        }
        let mut quota = queued_op("q1", OperationKind::UploadFile, "q1");
        quota.created_at = 1_760_000_000;
        db.enqueue_operation(&quota).unwrap();
        db.record_operation_pause("q1", OperationPauseReason::Quota, None, 5).unwrap();
        db.enqueue_operation(&queued_op("o1", OperationKind::UploadFile, "o1")).unwrap();
        db.0.lock()
            .unwrap()
            .execute("UPDATE operation_queue SET paused_reason = 'secret-folder' WHERE op_id = 'o1'", [])
            .unwrap();
        db.enqueue_operation(&queued_op("w1", OperationKind::UploadFile, "w1")).unwrap();
        let mut backing_off = queued_op("w2", OperationKind::UploadFile, "w2");
        backing_off.attempts = 3;
        backing_off.next_retry_at = 4_000_000_000;
        db.enqueue_operation(&backing_off).unwrap();
        let mut given_up = queued_op("g1", OperationKind::CreateFolder, "g1");
        given_up.attempts = 25;
        given_up.max_attempts = 25;
        db.enqueue_operation(&given_up).unwrap();
        let queued_before: i64 =
            db.0.lock().unwrap().query_row("SELECT COUNT(*) FROM operation_queue", [], |row| row.get(0)).unwrap();

        let summary = db.queue_hold_summary().unwrap();

        assert_eq!((summary.waiting, summary.gave_up), (2, 1));
        assert_eq!(
            summary.held,
            vec![
                HeldGroup { reason: "auth".into(), count: 2, oldest_at: 1_759_579_200, newest_at: 1_759_586_400 },
                HeldGroup { reason: "other".into(), count: 1, oldest_at: 1, newest_at: 1 },
                HeldGroup { reason: "quota".into(), count: 1, oldest_at: 1_760_000_000, newest_at: 1_760_000_000 },
            ]
        );
        let json = serde_json::to_string(&summary).unwrap();
        assert!(!json.contains("secret"), "{json}");
        let queued_after: i64 =
            db.0.lock().unwrap().query_row("SELECT COUNT(*) FROM operation_queue", [], |row| row.get(0)).unwrap();
        assert_eq!((queued_before, queued_after), (7, 7), "a read: nothing purged");
        assert_eq!(db.queue_diagnostics(500).unwrap().paused, 4, "and nothing resumed");
    }

    /// 0.8.15: the Selective sync tree reads `current_version` and `remote_updated_at` (whether the server knows the
    /// row); `list_files` does not carry the first.
    #[test]
    fn list_tree_rows_carries_the_server_version_of_every_row() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        {
            let conn = db.0.lock().unwrap();
            conn.execute(
                "INSERT INTO files (file_id, path, status, item_kind, current_version, parent_id, size_bytes, remote_updated_at)
                 VALUES ('ghost', 'Archive', 'local', 'folder', 0, NULL, 0, 0),
                        ('server', 'Archive', 'cloud_only', 'folder', 3, NULL, 0, 0),
                        ('f1', 'Archive/a.txt', 'local', 'file', 2, 'server', 12, 1761480000)",
                [],
            )
            .unwrap();
        }
        let rows = db.list_tree_rows().unwrap();
        let by_id = |id: &str| rows.iter().find(|row| row.file_id == id).unwrap().clone();
        assert_eq!(rows.len(), 3);
        assert_eq!(
            (by_id("ghost").current_version, by_id("ghost").remote_updated_at, by_id("ghost").status),
            (0, 0, FileStatus::Local)
        );
        assert_eq!(by_id("f1").remote_updated_at, 1_761_480_000);
        assert_eq!((by_id("server").current_version, by_id("server").item_kind), (3, ItemKind::Folder));
        assert_eq!(
            (by_id("f1").size_bytes, by_id("f1").parent_id.as_deref(), by_id("f1").item_kind),
            (12, Some("server"), ItemKind::File)
        );
    }

    /// Lead ruling, F9 review I-1: after a key replacement, the operations paused for `auth` are re-marked
    /// `key_replaced`. They stay queued and paused, with their attempts; the auth resume never makes them due again; the
    /// diagnostics count them under their own reason; every other pause, and an unpaused operation, is untouched.
    #[test]
    fn after_a_key_replacement_the_auth_paused_operations_are_kept_and_never_resumed() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        let mut auth = queued_op("a1", OperationKind::CreateFolder, "f-a");
        auth.attempts = 2;
        auth.next_retry_at = 10_000;
        db.enqueue_operation(&auth).unwrap();
        for (id, reason) in [
            ("a1", OperationPauseReason::Auth),
            ("a2", OperationPauseReason::Auth),
            ("q1", OperationPauseReason::Quota),
            ("p1", OperationPauseReason::Permission),
            ("l1", OperationPauseReason::Locked),
        ] {
            if id != "a1" {
                db.enqueue_operation(&queued_op(id, OperationKind::UploadFile, id))
                    .unwrap();
            }
            db.record_operation_pause(id, reason, Some("HTTP 401 Unauthorized"), 5)
                .unwrap();
        }
        let mut unpaused = queued_op("d1", OperationKind::UploadFile, "f-d");
        unpaused.created_at = 2;
        db.enqueue_operation(&unpaused).unwrap();
        let due = |now| {
            db.list_due_operations(now)
                .unwrap()
                .into_iter()
                .map(|op| op.op_id)
                .collect::<Vec<_>>()
        };

        assert_eq!(
            db.hold_operations_paused_for_auth_as_key_replaced(500).unwrap(),
            2,
            "both auth-paused operations"
        );
        let reasons = BTreeMap::from([
            ("key_replaced".to_string(), 2),
            ("locked".to_string(), 1),
            ("permission".to_string(), 1),
            ("quota".to_string(), 1),
        ]);
        let diagnostics = db.queue_diagnostics(500).unwrap();
        assert_eq!(
            diagnostics.paused_by_reason, reasons,
            "counted under their own reason; every other pause stays"
        );
        assert_eq!((diagnostics.queued, diagnostics.paused), (6, 5), "kept, not purged");

        assert_eq!(
            db.resume_operations_paused_for_auth(600).unwrap(),
            0,
            "the auth resume finds none"
        );
        assert_eq!(due(700), vec!["d1".to_string()], "never due again");
        assert_eq!(db.queue_diagnostics(700).unwrap().paused_by_reason, reasons);
        let attempts: i64 =
            db.0.lock()
                .unwrap()
                .query_row("SELECT attempts FROM operation_queue WHERE op_id = 'a1'", [], |row| {
                    row.get(0)
                })
                .unwrap();
        assert_eq!(attempts, 2, "its attempts are kept");
        assert_eq!(
            db.hold_operations_paused_for_auth_as_key_replaced(800).unwrap(),
            0,
            "a second call moves nothing"
        );
    }

    #[test]
    fn diagnostics_scrubs_names_known_only_from_recent_transfers() {
        // Task 1685 guard: a name that lives only in `transfer_activity` is a known
        // name, so the support bundle must not carry it. `2026` is a bare number the
        // allow-list keeps, so only the name scan can remove it.
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        db.record_transfer_activity(TransferActivityInput {
            direction: "up",
            file_id: None,
            file_name: "Board minutes 2026".into(),
            rel_path: None,
            bytes: 1,
            occurred_at: 1,
        })
        .unwrap();
        db.enqueue_operation(&queued_op("op-1", OperationKind::UploadFile, "x"))
            .unwrap();
        db.record_operation_attempt("op-1", 1, 10, Some("copy of Board minutes 2026 failed"))
            .unwrap();
        let exported = serde_json::to_string(&db.queue_diagnostics(200).unwrap()).unwrap();
        assert!(!exported.contains("2026"), "{exported}");
        assert!(!exported.contains("Board"), "{exported}");
        assert!(exported.contains("failed"), "{exported}");
    }

    #[test]
    fn known_names_include_synced_paths_for_the_lifecycle_log() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        db.upsert_file(&FileEntry {
            file_id: "f1".into(),
            path: "Tax 2025/aangifte.pdf".into(),
            status: FileStatus::Local,
            size_bytes: 1,
            modified_at: 0,
            content_hash: None,
            remote_updated_at: 0,
            parent_id: None,
            item_kind: ItemKind::File,
        })
        .unwrap();
        // `redact_for_export` turns any unknown word into `[name]` by itself, so `aangifte` going
        // missing proves nothing. `2025` is a bare number its allow-list keeps: only the known-name
        // scan removes it. The control proves it survives without the names, so this can go red.
        let message = "could not open Tax 2025";
        let control = crate::diagnostic_redaction::redact_for_export(message, &KnownNames::new());
        assert!(
            control.text.contains("2025"),
            "control: a bare number must survive without names: {}",
            control.text
        );
        let names = db.known_names(&[]).unwrap();
        let out = crate::diagnostic_redaction::redact_for_export(message, &names);
        assert!(
            !out.text.contains("2025"),
            "the synced path was not a known name: {}",
            out.text
        );
        // The extra paths (the sync root) are known names too.
        let with_root = db.known_names(&["Beebeeb Sync 4417".to_string()]).unwrap();
        let out = crate::diagnostic_redaction::redact_for_export("could not open Beebeeb Sync 4417", &with_root);
        assert!(
            !out.text.contains("4417"),
            "an extra path was not a known name: {}",
            out.text
        );
    }

    #[test]
    fn leaving_an_account_clears_its_recent_transfers() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        db.record_transfer_activity(transfer("up", "secret.txt", 1)).unwrap();
        db.purge_all_local_state().unwrap();
        assert!(db.list_recent_transfer_activity(10).unwrap().is_empty());
    }
