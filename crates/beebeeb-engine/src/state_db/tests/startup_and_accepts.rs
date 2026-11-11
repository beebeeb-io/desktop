//! Tests for startup repair, parks and finder accepts, split out of `state_db.rs`.

use super::*;

/// T31 (§9.2): the startup reconcile leaves a row whose Finder upload has not parked
/// `Uploading`. A row with no Finder upload is reset to `Error`, as before.
#[test]
fn startup_keeps_a_row_with_a_live_upload_uploading() {
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    seed_own_row(&db, "with-save", FileStatus::Local, 10);
    let mut contract = db.get_file_contract_state("with-save").unwrap().unwrap();
    contract.current_version = 1;
    db.set_file_contract_state(&contract).unwrap();
    db.track_staged_payload("/staged/with-save", None, false).unwrap();
    let accepted = db
        .accept_finder_write(
            &FinderAccept {
                retire_parked: None,
                op_id: "op-with-save",
                file_id: "with-save",
                kind: FinderAcceptKind::Modify {
                    incoming_base: Some("1"),
                },
                parent_id: None,
                target_path: Some("with-save.txt"),
                metadata_json: "{}",
                payload_path: "/staged/with-save",
                size_bytes: 3,
                modified_at: 100,
                backup_source_key: None,
                now: 100,
            },
            &|_, _| Vec::new(),
        )
        .unwrap();
    assert!(matches!(accepted, AcceptOutcome::Queued { .. }));
    seed_own_row(&db, "stale-upload", FileStatus::Uploading, 20);

    db.reconcile_stale_in_flight_on_startup().unwrap();

    assert_eq!(
        db.get_file("with-save").unwrap().unwrap().status,
        FileStatus::Uploading,
        "its save still uploads"
    );
    assert_eq!(
        db.get_file("stale-upload").unwrap().unwrap().status,
        FileStatus::Error,
        "unchanged for a row with no Finder upload"
    );
}

/// The startup reconcile returns the rows it reset, which is what its log
/// line reports. A row it keeps `Uploading` (its Finder upload resumes) is not counted.
#[test]
fn the_startup_reconcile_counts_only_the_rows_it_resets() {
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    seed_v1_row(&db, "with-save");
    assert!(matches!(
        accept_save(&db, "op-with-save", "with-save", Some("1")),
        AcceptOutcome::Queued { .. }
    ));
    db.set_status("with-save", FileStatus::Uploading).unwrap();
    seed_own_row(&db, "stale-upload", FileStatus::Uploading, 20);
    seed_own_row(&db, "stale-download", FileStatus::Downloading, 30);

    let reset = db.reconcile_stale_in_flight_on_startup().unwrap();

    assert_eq!(
        reset,
        vec!["stale-download".to_string(), "stale-upload".to_string()],
        "the stale upload and the stale download, not the kept row"
    );
    assert_eq!(db.get_file("with-save").unwrap().unwrap().status, FileStatus::Uploading);
    assert_eq!(db.get_file("stale-upload").unwrap().unwrap().status, FileStatus::Error);
    assert_eq!(
        db.get_file("stale-download").unwrap().unwrap().status,
        FileStatus::CloudOnly
    );
    assert!(
        db.reconcile_stale_in_flight_on_startup().unwrap().is_empty(),
        "nothing left to reset"
    );
}

/// Spec §9.1–§9.2: a row the startup reconcile resets changes how its item is presented
/// (an upload that will not resume presents `error`, read-only), so each reset is recorded
/// for the system like every other status change, in the reconcile's transaction. A row it
/// keeps records nothing.
#[test]
fn the_startup_reconcile_records_each_row_it_resets_for_the_system() {
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    seed_v1_row(&db, "with-save");
    assert!(matches!(
        accept_save(&db, "op-with-save", "with-save", Some("1")),
        AcceptOutcome::Queued { .. }
    ));
    seed_own_row(&db, "stale-upload", FileStatus::Uploading, 20);
    seed_own_row(&db, "stale-download", FileStatus::Downloading, 30);
    let tip = |db: &StateDb| -> i64 {
        db.0.lock()
            .unwrap()
            .query_row("SELECT COALESCE(MAX(seq), 0) FROM fp_changes", [], |row| row.get(0))
            .unwrap()
    };
    let before = tip(&db);

    db.reconcile_stale_in_flight_on_startup().unwrap();

    let recorded: Vec<(String, String)> = {
        let conn = db.0.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT file_id, kind FROM fp_changes WHERE seq > ?1 ORDER BY file_id")
            .unwrap();
        stmt.query_map(params![before], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<Result<Vec<_>>>()
            .unwrap()
    };
    assert_eq!(
        recorded,
        vec![
            (
                "stale-download".to_string(),
                FpChangeKind::Modified.as_str().to_string()
            ),
            ("stale-upload".to_string(), FpChangeKind::Modified.as_str().to_string()),
        ],
        "one change per reset row, none for the row whose save still uploads"
    );
}

/// §9.1, step 3.3: the runner's park of a claimed Finder upload sets `Error` in its own
/// transaction, and records the change for the system. A file whose later save is still
/// queued stays `Uploading`.
#[test]
fn a_claimed_park_settles_the_row_to_error() {
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    seed_v1_row(&db, "parked");
    queued_token(accept_save(&db, "w1", "parked", Some("1")));
    assert_eq!(status_of(&db, "parked"), FileStatus::Uploading, "precondition");
    let changes = changes_of(&db, "parked");
    let ClaimOutcome::Claimed(claimed) = db.claim_operation("w1", 1).unwrap() else {
        panic!("w1 is claimable")
    };
    assert!(
        db.park_claimed("w1", &claimed.claim_id, ParkReason::PayloadMissing, 2)
            .unwrap()
    );
    assert_eq!(
        status_of(&db, "parked"),
        FileStatus::Error,
        "a parked upload presents error"
    );
    assert_eq!(
        changes_of(&db, "parked"),
        changes + 1,
        "the flip is recorded for the system"
    );

    seed_v1_row(&db, "live");
    let first = queued_token(accept_save(&db, "l1", "live", Some("1")));
    queued_token(accept_save(&db, "l2", "live", Some(&first)));
    let ClaimOutcome::Claimed(claimed) = db.claim_operation("l1", 1).unwrap() else {
        panic!("l1 is claimable")
    };
    assert!(
        db.park_claimed("l1", &claimed.claim_id, ParkReason::PayloadMissing, 2)
            .unwrap()
    );
    assert_eq!(
        status_of(&db, "live"),
        FileStatus::Uploading,
        "its successor has not parked: still uploading"
    );
}

/// §9.1 (m-8): the settle moves only an `uploading` row. A park never
/// takes a `Trashing` row out of the trash, nor a `Conflict` row out of its conflict, and
/// records no change for either.
#[test]
fn a_park_never_moves_a_row_out_of_the_trash_or_a_conflict() {
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    for (file_id, op_id, status) in [
        ("in-trash", "t1", FileStatus::Trashing),
        ("in-conflict", "c1", FileStatus::Conflict),
    ] {
        seed_v1_row(&db, file_id);
        queued_token(accept_save(&db, op_id, file_id, Some("1")));
        db.set_status(file_id, status.clone()).unwrap();
        let changes = changes_of(&db, file_id);
        let ClaimOutcome::Claimed(claimed) = db.claim_operation(op_id, 1).unwrap() else {
            panic!("{op_id} is claimable")
        };
        assert!(
            db.park_claimed(op_id, &claimed.claim_id, ParkReason::PayloadMissing, 2)
                .unwrap(),
            "precondition: {op_id} parked"
        );
        assert_eq!(
            status_of(&db, file_id),
            status,
            "a park leaves a {status:?} row as it is"
        );
        assert_eq!(changes_of(&db, file_id), changes, "no change recorded for {file_id}");
    }
}

/// §9.1, step 3.3 (rule 6a′): a save parked at its accept sets `Error` in the accept's
/// transaction.
#[test]
fn a_park_at_accept_settles_the_row_to_error() {
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    seed_v1_row(&db, "unknown-base");
    let outcome = accept_save(&db, "w1", "unknown-base", None);
    assert!(
        matches!(outcome, AcceptOutcome::ParkedAtOnce { .. }),
        "precondition: {outcome:?}"
    );
    assert_eq!(status_of(&db, "unknown-base"), FileStatus::Error);
}

/// §9.1, step 3.3: a park inside the claim (here `predecessor_lost`) sets `Error` in the
/// claim's transaction.
#[test]
fn a_park_in_the_claim_settles_the_row_to_error() {
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    seed_v1_row(&db, "orphan");
    let first = queued_token(accept_save(&db, "w1", "orphan", Some("1")));
    queued_token(accept_save(&db, "w2", "orphan", Some(&first)));
    db.0.lock()
        .unwrap()
        .execute("DELETE FROM operation_queue WHERE op_id = 'w1'", [])
        .unwrap();
    let outcome = db.claim_operation("w2", 1).unwrap();
    assert!(
        matches!(
            outcome,
            ClaimOutcome::Parked {
                reason: ParkReason::PredecessorLost,
                ..
            }
        ),
        "precondition: parked in the claim"
    );
    assert_eq!(status_of(&db, "orphan"), FileStatus::Error);
}

/// §9.1, step 3.3: the snapshot count's park (`base_unknown`) sets `Error` for each file it
/// parked, in its transaction.
#[test]
fn the_snapshot_counts_park_settles_the_row_to_error() {
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    // Version 0 and no base: the save waits for a snapshot (6b).
    seed_own_row(&db, "no-version", FileStatus::Local, 10);
    queued_token(accept_save(&db, "w1", "no-version", None));
    assert_eq!(db.finder_write("w1").unwrap().unwrap().base_pending, 1, "precondition");
    let mut parked = Vec::new();
    for snapshot in 0..BASE_PENDING_PARK_AT {
        parked = db.note_snapshot_for_base_pending(10 + snapshot).unwrap();
        if !parked.is_empty() {
            break;
        }
    }
    assert_eq!(
        parked,
        vec![("w1".to_string(), Some("no-version".to_string()))],
        "precondition"
    );
    assert_eq!(status_of(&db, "no-version"), FileStatus::Error);
}

/// §9.1–§9.2: a Finder upload whose last attempt used up its attempts has parked by the
/// queue's own rule (`attempts < max_attempts` is "not parked"), so it presents `Error`.
/// An earlier failed attempt leaves `Uploading`. An upload without a write id (Windows, the
/// watcher) is not touched: its rollback keeps its status.
#[test]
fn a_finder_upload_that_uses_up_its_attempts_settles_the_row_to_error() {
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    for (file_id, op_id) in [("retried", "r1"), ("given-up", "g1")] {
        seed_v1_row(&db, file_id);
        queued_token(accept_save(&db, op_id, file_id, Some("1")));
    }
    let ClaimOutcome::Claimed(claimed) = db.claim_operation("r1", 1).unwrap() else {
        panic!("r1 is claimable")
    };
    assert!(
        db.record_attempt_claimed("r1", &claimed.claim_id, 1, 60, Some("HTTP 500"))
            .unwrap()
    );
    assert_eq!(status_of(&db, "retried"), FileStatus::Uploading, "it will retry");
    let ClaimOutcome::Claimed(claimed) = db.claim_operation("g1", 1).unwrap() else {
        panic!("g1 is claimable")
    };
    let last = claimed.op.max_attempts;
    assert!(
        db.record_attempt_claimed("g1", &claimed.claim_id, last, 60, Some("HTTP 500"))
            .unwrap()
    );
    assert_eq!(status_of(&db, "given-up"), FileStatus::Error, "it will not retry");

    seed_own_row(&db, "watcher", FileStatus::Uploading, 10);
    db.enqueue_operation(&queued(
        "u1",
        OperationKind::UploadVersion,
        "watcher",
        Some("/staged/u1"),
    ))
    .unwrap();
    let ClaimOutcome::Claimed(claimed) = db.claim_operation("u1", 1).unwrap() else {
        panic!("u1 is claimable")
    };
    assert!(
        db.record_attempt_claimed("u1", &claimed.claim_id, 25, 60, Some("HTTP 500"))
            .unwrap()
    );
    assert_eq!(
        status_of(&db, "watcher"),
        FileStatus::Uploading,
        "write-keyed: no write id, not touched here"
    );
}

/// Review M4 (macOS): a released copy (`completed = 1`) is one whose upload landed and that
/// waits only for its unlink, so the account-switch warning does not count it as a change not
/// uploaded. An unfinished copy still counts.
#[cfg(target_os = "macos")]
#[test]
fn a_released_copy_is_not_a_pending_change() {
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    db.track_staged_payload("/staged/released", None, true).unwrap();
    assert_eq!(
        db.pending_changes_count().unwrap(),
        0,
        "a landed copy waiting for its unlink"
    );
    db.track_staged_payload("/staged/unfinished", None, false).unwrap();
    assert_eq!(
        db.pending_changes_count().unwrap(),
        1,
        "an unfinished copy still counts"
    );
}

/// Spec §8.7 S6: the accept requires its copy's journal row, unfinished, so an engine-start sweep
/// that removed the copy first cannot leave an op naming a missing file. Without it,
/// nothing is written: no op, no held write, no status change.
#[test]
fn an_accept_whose_journal_row_is_gone_writes_nothing() {
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    seed_own_row(&db, "swept", FileStatus::Local, 10);
    let mut contract = db.get_file_contract_state("swept").unwrap().unwrap();
    contract.current_version = 1;
    db.set_file_contract_state(&contract).unwrap();
    let accept = |payload_path: &str| {
        db.accept_finder_write(
            &FinderAccept {
                retire_parked: None,
                op_id: "op-swept",
                file_id: "swept",
                kind: FinderAcceptKind::Modify {
                    incoming_base: Some("1"),
                },
                parent_id: None,
                target_path: Some("swept.txt"),
                metadata_json: "{}",
                payload_path,
                size_bytes: 4,
                modified_at: 20,
                backup_source_key: None,
                now: 30,
            },
            &|_, _| Vec::new(),
        )
        .unwrap()
    };
    // No row at all, and a released row: both are not an unfinished copy.
    db.track_staged_payload("/staged/released", None, true).unwrap();
    for path in ["/staged/never-journalled", "/staged/released"] {
        assert!(matches!(accept(path), AcceptOutcome::CopyGone), "{path}");
    }
    assert!(db.list_operations_for_file("swept").unwrap().is_empty(), "no op");
    assert_eq!(db.get_file("swept").unwrap().unwrap().status, FileStatus::Local);
    let (held, _) = held_write_conn(&db.hold_lock_for_test(), "swept").unwrap();
    assert!(held.is_none(), "no held write");
    // Journalled and unfinished: accepted.
    db.track_staged_payload("/staged/journalled", None, false).unwrap();
    assert!(matches!(accept("/staged/journalled"), AcceptOutcome::Queued { .. }));
}

/// U45 (spec 2026-10-10, §15 question 3, option A): once a sign-out's keep step has closed Finder accepts, an
/// accept that outlived the engine stop queues nothing, and its staged copy's journal row stays unfinished.
#[test]
fn a_finder_accept_after_the_engine_stop_queues_nothing() {
    let (_dir, db) = seeded_for_a_late_accept();
    db.close_finder_accepts().unwrap();
    assert!(matches!(
        accept_late_save(&db, "/staged/late"),
        AcceptOutcome::AcceptsClosed
    ));
    assert!(db.list_operations_for_file("late").unwrap().is_empty(), "no op");
    let unfinished: bool = db
        .hold_lock_for_test()
        .query_row(
            "SELECT completed = 0 FROM staged_payloads WHERE path = '/staged/late'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        unfinished,
        "the journal row is left for the purge, or for the next start's sweep"
    );
}

/// U45: an engine start, and a cleared account, take saves again.
#[test]
fn accepts_reopen_at_engine_start_and_with_the_cleared_account() {
    let (_dir, db) = seeded_for_a_late_accept();
    db.close_finder_accepts().unwrap();
    db.engine_start_repair().unwrap();
    assert!(!db.finder_accepts_closed().unwrap());
    assert!(matches!(
        accept_late_save(&db, "/staged/late"),
        AcceptOutcome::Queued { .. }
    ));
    db.close_finder_accepts().unwrap();
    db.clear_account_data(false).unwrap();
    assert!(!db.finder_accepts_closed().unwrap());
    db.close_finder_accepts().unwrap();
    db.reopen_finder_accepts().unwrap();
    assert!(
        !db.finder_accepts_closed().unwrap(),
        "a sign-out that stops reopens them at once"
    );
}

/// U45: reopening accepts deletes the key and nothing else, wherever it happens: a sign-out that stops and the
/// launch (`reopen_finder_accepts`), and an engine start (`engine_start_repair`). The rest of `sync_state` is the
/// R10 owner record, a Finder removal debt and the sync cursor. Without the owner, the next engine start would find
/// unowned local data and reset it (`account_binding::decide`), and the waiting saves with it.
#[test]
fn reopening_accepts_clears_the_key_and_nothing_else() {
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    let owner = crate::account_binding::Identity::new(Some("u-a"), Some("a@beebeeb.io"));
    db.set_owner(&owner).unwrap();
    db.set_finder_removal_owed(true).unwrap();
    db.set_sync_cursor(12).unwrap();
    let sync_state = |db: &StateDb| -> Vec<(String, String)> {
        let conn = db.hold_lock_for_test();
        let mut stmt = conn.prepare("SELECT key, value FROM sync_state ORDER BY key").unwrap();
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?))).unwrap();
        rows.collect::<Result<Vec<_>>>().unwrap()
    };
    let before = sync_state(&db);
    let reopened = |when: &str| {
        assert!(!db.finder_accepts_closed().unwrap(), "{when}: the key is gone");
        assert_eq!(db.owner().unwrap(), Some(owner.clone()), "{when}: the owner stays");
        assert!(db.finder_removal_owed().unwrap(), "{when}: a removal debt stays");
        assert_eq!(
            sync_state(&db),
            before,
            "{when}: every other sync_state row is as it was"
        );
    };
    db.close_finder_accepts().unwrap();
    db.reopen_finder_accepts().unwrap();
    reopened("a sign-out that stops, or the launch");
    db.close_finder_accepts().unwrap();
    db.engine_start_repair().unwrap();
    reopened("an engine start");
}

/// The closed-accepts key is a device key, as the Finder removal debt and the domain-gone mark are: it records what
/// a sign-out on this Mac closed, not anything of an account. Alone, or beside the owner record, it is not account
/// data, so it never makes the R10 binding reset a database or the upgrade adopt one.
#[test]
fn the_closed_accepts_key_is_not_account_data() {
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    db.close_finder_accepts().unwrap();
    assert!(
        !db.has_account_data().unwrap(),
        "the closed-accepts key is not account data"
    );
    db.set_owner(&crate::account_binding::Identity::new(
        Some("u-a"),
        Some("a@beebeeb.io"),
    ))
    .unwrap();
    assert!(!db.has_account_data().unwrap(), "nor is it beside the owner record");
    db.set_sync_cursor(12).unwrap();
    assert!(
        db.has_account_data().unwrap(),
        "a sync cursor beside them is account data"
    );
}

/// U45: the engine start's reopen does not ride the repair's transaction. A repair that fails rolls back only its
/// own work; the runner logs it and serves Finder writes anyway, so accepts are open by then.
#[test]
fn a_failed_engine_start_repair_still_reopens_accepts() {
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    db.close_finder_accepts().unwrap();
    // The repair's alias sweep is refused: an alias old enough to be swept, and a trigger that aborts its delete.
    db.hold_lock_for_test()
        .execute_batch(
            "INSERT INTO id_aliases (provisional_id, server_id, created_at) VALUES ('p', 's', 0);
                 CREATE TRIGGER no_sweep BEFORE DELETE ON id_aliases BEGIN SELECT RAISE(ABORT, 'refused'); END;",
        )
        .unwrap();
    assert!(db.engine_start_repair().is_err(), "the repair failed");
    assert!(
        !db.finder_accepts_closed().unwrap(),
        "a failed repair still reopens accepts"
    );
}

/// U45: an accept whose transaction began first commits first, and a read after the close sees it. Two
/// connections to one file, as the engine and the sign-out have.
#[test]
fn an_accept_already_running_commits_before_the_close() {
    for round in 0..50 {
        let dir = tempdir().unwrap();
        let path = dir.path().join("state.db");
        let engine = StateDb::open(&path).unwrap();
        let sign_out = StateDb::open(&path).unwrap();
        let (started, wait_started) = std::sync::mpsc::channel();
        let accept = std::thread::spawn(move || {
            let mut conn = engine.hold_lock_for_test();
            let tx = conn
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .unwrap();
            started.send(()).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(20));
            tx.execute(
                "INSERT INTO operation_queue (op_id, kind, file_id) VALUES ('late', 'upload_version', 'f')",
                [],
            )
            .unwrap();
            tx.commit().unwrap();
        });
        wait_started.recv().unwrap();
        sign_out.close_finder_accepts().unwrap();
        let rows = sign_out.waiting_rows().unwrap();
        accept.join().unwrap();
        assert!(rows.iter().any(|row| row.op_id == "late"), "round {round}");
    }
}

/// U45, the other half of the barrier: the close takes the write lock first. An accept that starts while the
/// close's transaction is open waits for it, then reads the key in its own transaction and queues nothing. The
/// close is held open by hand here (`close_finder_accepts` commits at once), so the accept starts inside it.
#[test]
fn an_accept_waiting_behind_the_close_queues_nothing() {
    for round in 0..20 {
        let (dir, engine) = seeded_for_a_late_accept();
        let sign_out = StateDb::open(dir.path().join("state.db")).unwrap();
        let mut conn = sign_out.hold_lock_for_test();
        let close = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .unwrap();
        let accept = std::thread::spawn(move || {
            let outcome = accept_late_save(&engine, "/staged/late");
            (outcome, engine.list_operations_for_file("late").unwrap().len())
        });
        std::thread::sleep(std::time::Duration::from_millis(100));
        close
            .execute(
                "INSERT INTO sync_state (key, value) VALUES (?1, '1')",
                params![FINDER_ACCEPTS_CLOSED_KEY],
            )
            .unwrap();
        close.commit().unwrap();
        drop(conn);
        let (outcome, ops) = accept.join().unwrap();
        assert!(
            matches!(outcome, AcceptOutcome::AcceptsClosed) && ops == 0,
            "round {round}: an accept that waited behind the close queued: {outcome:?}, {ops} op(s)"
        );
    }
}

/// U45, the same for every other Finder change: one that starts while the close holds the write lock waits for it,
/// then reads the key in `enqueue_finder_change`'s own transaction and queues nothing.
#[test]
fn a_finder_change_waiting_behind_the_close_queues_nothing() {
    for round in 0..20 {
        let dir = tempdir().unwrap();
        let path = dir.path().join("state.db");
        let engine = StateDb::open(&path).unwrap();
        let sign_out = StateDb::open(&path).unwrap();
        let mut conn = sign_out.hold_lock_for_test();
        let close = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .unwrap();
        let change = std::thread::spawn(move || {
            let outcome = engine
                .enqueue_finder_change(&queued_op("rename", OperationKind::RenameFile, "f"), None)
                .unwrap();
            (outcome, engine.list_operations_for_file("f").unwrap().len())
        });
        std::thread::sleep(std::time::Duration::from_millis(100));
        close
            .execute(
                "INSERT INTO sync_state (key, value) VALUES (?1, '1')",
                params![FINDER_ACCEPTS_CLOSED_KEY],
            )
            .unwrap();
        close.commit().unwrap();
        drop(conn);
        assert_eq!(
            change.join().unwrap(),
            (FinderEnqueue::AcceptsClosed, 0),
            "round {round}: a change that waited behind the close queued"
        );
    }
}

/// U45: in both fenced bodies the key is the first thing read inside the `BEGIN IMMEDIATE` transaction that queues,
/// never before it: a read taken before the write lock can see accepts open, wait behind the close, and then queue.
#[test]
fn the_fence_is_read_first_inside_the_transaction_that_queues() {
    let source: Vec<&str> = include_str!("../../state_db/landing.rs").lines().collect();
    for definition in ["pub fn accept_finder_write(", "pub fn enqueue_finder_change("] {
        let start = source
            .iter()
            .position(|line| line.trim_start().starts_with(definition))
            .unwrap_or_else(|| panic!("{definition} not found"));
        let end = start
            + source[start..]
                .iter()
                .position(|line| *line == "    }")
                .expect("the body's end");
        let body = &source[start..end];
        let begin = body
            .iter()
            .position(|line| line.contains(".transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;"))
            .unwrap_or_else(|| panic!("{definition}: no BEGIN IMMEDIATE"));
        let first = body[begin + 1..]
            .iter()
            .map(|line| line.trim())
            .find(|line| !line.is_empty() && !line.starts_with("//"))
            .unwrap();
        assert_eq!(
            first, "if finder_accepts_closed_conn(&tx)? {",
            "{definition}: the fence is the first read inside the transaction"
        );
        assert_eq!(
            body.iter()
                .filter(|line| line.contains("finder_accepts_closed_conn("))
                .count(),
            1,
            "{definition}: one read of the fence"
        );
    }
}

/// Spec §8.7 S6: the sweep deletes an unfinished row only while nothing names it, so an
/// accept that committed first keeps its row; a released row is never this step's.
#[test]
fn the_sweep_never_forgets_a_row_an_accept_committed() {
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    db.track_staged_payload("/staged/accepted", None, false).unwrap();
    db.hold_lock_for_test()
        .execute(
            "INSERT INTO operation_queue (op_id, kind, file_id, payload_path)
                 VALUES ('op-accepted', 'upload_version', 'f', '/staged/accepted')",
            [],
        )
        .unwrap();
    db.track_staged_payload("/staged/orphan", None, false).unwrap();
    db.track_staged_payload("/staged/released", None, true).unwrap();
    assert!(!db.forget_unfinished_unreferenced("/staged/accepted").unwrap());
    assert!(db.forget_unfinished_unreferenced("/staged/orphan").unwrap());
    assert!(!db.forget_unfinished_unreferenced("/staged/released").unwrap());
    let mut left: Vec<String> = db
        .staged_payloads_for_signout()
        .unwrap()
        .into_iter()
        .map(|(path, _, _)| path)
        .collect();
    left.sort();
    assert_eq!(
        left,
        vec!["/staged/accepted".to_string(), "/staged/released".to_string()]
    );
}

/// U17, the margin; U18.
#[test]
fn the_send_phase_makes_live_changes_due_with_a_margin() {
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    db.hold_lock_for_test()
            .execute_batch(
                "INSERT INTO operation_queue (op_id, kind, file_id, attempts, max_attempts, next_retry_at, paused_reason) VALUES
                   ('a0', 'upload_version', 'f', 0, 5, 9999, NULL),
                   ('a2', 'rename_file', 'f', 2, 5, 9999, NULL),
                   ('a3', 'upload_version', 'f', 3, 5, 9999, NULL),
                   ('a4', 'upload_version', 'f', 4, 5, 9999, NULL),
                   ('parked', 'upload_version', 'f', 5, 5, 9999, NULL),
                   ('paused', 'upload_version', 'f', 0, 5, 9999, 'auth'),
                   ('hydrate', 'hydrate_file', 'f', 0, 5, 9999, NULL);",
            )
            .unwrap();
    assert_eq!(db.make_due_for_send_phase(100).unwrap(), 2);
    let due: Vec<(String, i64, i64)> = {
        let conn = db.hold_lock_for_test();
        let mut stmt = conn
            .prepare("SELECT op_id, next_retry_at, attempts FROM operation_queue ORDER BY op_id")
            .unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    for (op, next, attempts) in due {
        let expected_next = if op == "a0" || op == "a2" { 100 } else { 9999 };
        assert_eq!(next, expected_next, "{op}");
        let inserted = match op.as_str() {
            "a2" => 2,
            "a3" => 3,
            "a4" => 4,
            "parked" => 5,
            _ => 0,
        };
        assert_eq!(attempts, inserted, "attempts never change: {op}");
    }
}

#[test]
fn the_send_phase_never_touches_parked_or_paused_changes() {
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    db.hold_lock_for_test()
            .execute_batch(
                "INSERT INTO operation_queue (op_id, kind, file_id, attempts, max_attempts, next_retry_at, paused_reason) VALUES
                   ('parked', 'upload_version', 'f', 25, 25, 9999, NULL), ('paused', 'create_folder', 'f', 0, 5, 9999, 'quota');",
            )
            .unwrap();
    assert_eq!(db.make_due_for_send_phase(100).unwrap(), 0);
    let untouched: i64 = db
        .hold_lock_for_test()
        .query_row(
            "SELECT COUNT(*) FROM operation_queue WHERE next_retry_at = 9999 AND attempts IN (0, 25)",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(untouched, 2);
}
