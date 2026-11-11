//! Tests for pruning absent rows, finder writes and held edits, split out of `state_db.rs`.

use super::*;

#[test]
fn prune_absent_deletes_settled_cloud_only_row_absent_from_snapshot() {
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    seed_own_row(&db, "stale", FileStatus::CloudOnly, 10);

    let seen: HashSet<String> = ["kept".to_string()].into_iter().collect();
    let pruned = db.prune_absent(&seen, FAR_FUTURE).unwrap();

    assert_eq!(
        pruned.iter().map(|r| r.file_id.clone()).collect::<Vec<_>>(),
        vec!["stale".to_string()]
    );
    assert!(db.get_file("stale").unwrap().is_none());
}

#[test]
fn prune_absent_keeps_row_with_pending_operation() {
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    // A locally-created-but-not-yet-uploaded file under its client UUID.
    seed_own_row(&db, "in-flight", FileStatus::CloudOnly, 10);
    db.enqueue_operation(&PendingOperation {
        op_id: "op-1".into(),
        kind: OperationKind::UploadVersion,
        file_id: Some("in-flight".into()),
        parent_id: None,
        target_path: Some("/in-flight.txt".into()),
        metadata_json: None,
        payload_path: None,
        base_version: None,
        base_object_version_id: None,
        attempts: 0,
        max_attempts: 5,
        next_retry_at: 0,
        last_error: None,
        backup_source_key: None,
        created_at: 0,
        updated_at: 0,
    })
    .unwrap();

    // Empty snapshot can't see the client UUID; the operation_queue join must
    // keep it (and the empty-snapshot guard also protects it — assert it
    // survives regardless).
    let pruned = db.prune_absent(&HashSet::new(), FAR_FUTURE).unwrap();
    assert!(pruned.is_empty());
    assert!(
        db.get_file("in-flight").unwrap().is_some(),
        "pending-op row never pruned"
    );
}

#[test]
fn prune_absent_keeps_trashing_row_with_pending_trash_op_present_in_snapshot() {
    // task 0802: a locally-deleted file is `Trashing` with its TrashFile op
    // still IN FLIGHT (not yet succeeded), and is STILL PRESENT in the
    // snapshot. prune must not remove it — the `operation_queue` join protects
    // a row with a pending op — so the trash op is left to complete. (Once it
    // succeeds the op is dropped and the row stays `Trashing`; convergence then
    // happens via snapshot-absence — see the `..._absent_from_snapshot` test.)
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    seed_own_row(&db, "trashing", FileStatus::Trashing, 10);
    db.enqueue_operation(&PendingOperation {
        op_id: "op-trash".into(),
        kind: OperationKind::TrashFile,
        file_id: Some("trashing".into()),
        parent_id: None,
        target_path: None,
        metadata_json: None,
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

    // Snapshot still lists the file (server-side trash not yet applied).
    let seen: HashSet<String> = ["trashing".to_string()].into_iter().collect();
    let pruned = db.prune_absent(&seen, FAR_FUTURE).unwrap();
    assert!(pruned.is_empty());
    assert!(
        db.get_file("trashing").unwrap().is_some(),
        "a Trashing row with a pending trash op must survive prune"
    );
}

#[test]
fn prune_absent_keeps_trashing_row_absent_from_snapshot_trash_view() {
    // task 0802 — FLIPPED BY TASK 1698 (trash ruling, RED-first): after
    // `api.trash_file` succeeds the TrashFile op is REMOVED (the
    // `Trashing` status, not the op, is the durable marker). The file is
    // ABSENT from `/sync/snapshot` — and under the 1698 ruling it stays
    // ABSENT forever (the snapshot never lists trashed files), but the
    // row IS the macOS Trash view now. prune_absent must NOT delete it:
    // convergence to removal flows through the server's `file_delete` op
    // (permanent delete), `file_restore`, or the retention janitor — the
    // same server trash model, no second one invented.
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    // Trashing row, NO pending op (op was dropped on a successful trash).
    seed_own_row(&db, "trashed-gone", FileStatus::Trashing, 10);

    // Snapshot no longer lists the file (server trash has propagated).
    let seen: HashSet<String> = ["still-here".to_string()].into_iter().collect();
    let pruned = db.prune_absent(&seen, FAR_FUTURE).unwrap();

    assert!(
        pruned.is_empty(),
        "a Trashing row absent from the snapshot must NOT be pruned (it is the trash view): {pruned:?}"
    );
    assert!(
        db.get_file("trashed-gone").unwrap().is_some(),
        "a Trashing row absent from the snapshot is the TRASH VIEW (task 1698): the row survives. \
             Convergence to removal now flows through the server's file_delete op (permanent delete), \
             the file_restore op, or the retention janitor — NOT prune_absent."
    );
}

#[test]
fn prune_absent_keeps_children_of_trashing_rows_trash_view_content() {
    // Task 1698: a child inside a trashed folder is the trash view's
    // folder content — the server cascade_trash trashed it too, so the
    // row must survive a snapshot that no longer lists either of them.
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    db.upsert_file(&FileEntry {
        file_id: "trashed-dir".into(),
        path: "trashed-dir".into(),
        status: FileStatus::Trashing,
        size_bytes: 0,
        modified_at: 0,
        content_hash: None,
        remote_updated_at: 10,
        parent_id: None,
        item_kind: ItemKind::Folder,
    })
    .unwrap();
    db.upsert_file(&FileEntry {
        file_id: "trashed-child".into(),
        path: "trashed-dir/leaf.txt".into(),
        status: FileStatus::CloudOnly,
        size_bytes: 1,
        modified_at: 0,
        content_hash: None,
        remote_updated_at: 10,
        parent_id: Some("trashed-dir".into()),
        item_kind: ItemKind::File,
    })
    .unwrap();
    // `upsert_file` does NOT write the parent linkage — the contract does.
    let mut contract = db.get_file_contract_state("trashed-child").unwrap().unwrap();
    contract.parent_id = Some("trashed-dir".into());
    db.set_file_contract_state(&contract).unwrap();

    let seen: HashSet<String> = ["still-here".to_string()].into_iter().collect();
    let pruned = db.prune_absent(&seen, FAR_FUTURE).unwrap();
    assert!(
        pruned.is_empty(),
        "trash-view rows (parent + content) must survive the prune: {pruned:?}"
    );
    assert!(db.get_file("trashed-dir").unwrap().is_some());
    assert!(db.get_file("trashed-child").unwrap().is_some());
}

#[test]
fn prune_absent_keeps_uploading_row() {
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    seed_own_row(&db, "uploading-row", FileStatus::Uploading, 10);

    let seen: HashSet<String> = ["other".to_string()].into_iter().collect();
    let pruned = db.prune_absent(&seen, FAR_FUTURE).unwrap();
    assert!(pruned.is_empty());
    assert!(
        db.get_file("uploading-row").unwrap().is_some(),
        "uploading row never pruned"
    );
}

#[test]
fn reconcile_stale_in_flight_on_startup_resets_transient_rows() {
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    seed_own_row(&db, "stale-upload", FileStatus::Uploading, 10);
    seed_own_row(&db, "stale-download", FileStatus::Downloading, 20);
    seed_own_row(&db, "local", FileStatus::Local, 30);
    seed_own_row(&db, "conflict", FileStatus::Conflict, 40);

    let touched = db.reconcile_stale_in_flight_on_startup().unwrap();

    assert_eq!(touched.len(), 2);
    assert_eq!(db.get_file("stale-upload").unwrap().unwrap().status, FileStatus::Error);
    assert_eq!(
        db.get_file("stale-download").unwrap().unwrap().status,
        FileStatus::CloudOnly
    );
    assert_eq!(db.get_file("local").unwrap().unwrap().status, FileStatus::Local);
    assert_eq!(db.get_file("conflict").unwrap().unwrap().status, FileStatus::Conflict);
    assert!(db.list_by_status(FileStatus::Uploading).unwrap().is_empty());
    assert!(db.list_by_status(FileStatus::Downloading).unwrap().is_empty());
}

#[test]
fn the_migration_is_additive_and_idempotent() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("state.db");
    // Build a round-3 database: today's schema without the round-4 columns.
    drop(StateDb::open(&path).unwrap());
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        for (table, column) in [
            ("files", "held_write_id"),
            ("files", "held_base"),
            ("files", "held_version"),
            ("files", "held_object_version_id"),
            ("files", "version_filled"),
            ("operation_queue", "write_id"),
            ("operation_queue", "write_origin"),
            ("operation_queue", "after_write_id"),
            ("operation_queue", "base_pending"),
            ("operation_queue", "claim_id"),
            ("operation_queue", "claimed_at"),
            ("upload_resume", "completed_version"),
            ("upload_resume", "completed_object_version_id"),
            ("upload_resume", "completed_mime_type"),
            ("upload_resume", "self_minted"),
        ] {
            let _ = conn.execute(&format!("DROP INDEX IF EXISTS idx_{table}_{column}"), []);
            conn.execute(&format!("ALTER TABLE {table} DROP COLUMN {column}"), [])
                .unwrap();
        }
        conn.execute("DROP TABLE id_aliases", []).unwrap();
        // The earlier schema had no index on operation_queue.file_id either.
        conn.execute("DROP INDEX IF EXISTS idx_operation_queue_file_id", [])
            .unwrap();
        conn.execute(
                "INSERT INTO files (file_id, path, status, size_bytes, current_version) VALUES ('f1', 'a.txt', 'local', 7, 3)",
                [],
            )
            .unwrap();
        conn.execute(
                "INSERT INTO operation_queue (op_id, kind, file_id, base_version) VALUES ('op1', 'upload_version', 'f1', 3)",
                [],
            )
            .unwrap();
    }
    // Opened twice: the second open must not fail on an existing column.
    drop(StateDb::open(&path).unwrap());
    let db = StateDb::open(&path).unwrap();

    let conn = rusqlite::Connection::open(&path).unwrap();
    let columns = |table: &str| -> Vec<String> {
        let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})")).unwrap();
        stmt.query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    for column in [
        "held_write_id",
        "held_base",
        "held_version",
        "held_object_version_id",
        "version_filled",
    ] {
        assert!(columns("files").contains(&column.to_string()), "files.{column}");
    }
    for column in [
        "write_id",
        "write_origin",
        "after_write_id",
        "base_pending",
        "claim_id",
        "claimed_at",
    ] {
        assert!(
            columns("operation_queue").contains(&column.to_string()),
            "operation_queue.{column}"
        );
    }
    for column in [
        "completed_version",
        "completed_object_version_id",
        "completed_mime_type",
        "self_minted",
    ] {
        assert!(
            columns("upload_resume").contains(&column.to_string()),
            "upload_resume.{column}"
        );
    }
    assert!(columns("id_aliases").contains(&"provisional_id".to_string()));
    let indexes: Vec<String> = {
        let mut stmt = conn.prepare("PRAGMA index_list(operation_queue)").unwrap();
        stmt.query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    let index_columns = |index: &str| -> Vec<String> {
        let mut stmt = conn.prepare(&format!("PRAGMA index_info({index})")).unwrap();
        stmt.query_map([], |row| row.get::<_, String>(2))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    for (index, column) in [
        ("idx_operation_queue_write_id", "write_id"),
        ("idx_operation_queue_after_write_id", "after_write_id"),
        ("idx_operation_queue_file_id", "file_id"),
    ] {
        assert!(indexes.contains(&index.to_string()), "{index} missing from {indexes:?}");
        assert_eq!(index_columns(index), vec![column.to_string()], "{index}");
    }

    // Rows are unchanged; the new columns read as their defaults.
    let row = db.get_file("f1").unwrap().unwrap();
    assert_eq!((row.path.as_str(), row.size_bytes), ("a.txt", 7));
    let presentation = db.item_presentation("f1").unwrap().unwrap();
    assert_eq!(presentation.contract.current_version, 3);
    assert!(presentation.held.is_none());
    assert!(!presentation.version_filled);
    assert_eq!(db.get_operation("op1").unwrap().unwrap().base_version, Some(3));
    let (base_pending, write_id): (i64, Option<String>) = conn
        .query_row(
            "SELECT base_pending, write_id FROM operation_queue WHERE op_id = 'op1'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        (base_pending, write_id),
        (0, None),
        "the earlier op's base is known and it carries no write"
    );
    assert!(
        db.finder_write("op1").unwrap().is_none(),
        "an op without a write id is not a Finder write"
    );
}

/// The held columns are written together: the accept sets
/// `held_write_id` and `held_base` in one statement, and a row with a write id but no
/// base is an error, never read as a base of 0.
#[test]
fn held_columns_are_written_together_and_a_half_set_row_is_an_error() {
    fn accept(db: &StateDb, op_id: &str, file_id: &str, kind: FinderAcceptKind<'_>) -> Result<AcceptOutcome> {
        // The accept requires its copy's journal row (spec §8.7 S6), as production writes it.
        db.track_staged_payload("/staged/x", None, false)?;
        db.accept_finder_write(
            &FinderAccept {
                retire_parked: None,
                op_id,
                file_id,
                kind,
                parent_id: None,
                target_path: Some("a.txt"),
                metadata_json: "{}",
                payload_path: "/staged/x",
                size_bytes: 1,
                modified_at: 1,
                backup_source_key: None,
                now: 1,
            },
            &|_, _| Vec::new(),
        )
    }
    fn held_pair(db: &StateDb, file_id: &str) -> (Option<String>, Option<i64>) {
        db.0.lock()
            .unwrap()
            .query_row(
                "SELECT held_write_id, held_base FROM files WHERE file_id = ?1",
                params![file_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap()
    }
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    let row = |file_id: &str| FileEntry {
        file_id: file_id.into(),
        path: "a.txt".into(),
        status: FileStatus::Local,
        size_bytes: 1,
        modified_at: 1,
        content_hash: None,
        remote_updated_at: 0,
        parent_id: None,
        item_kind: ItemKind::File,
    };

    let created = row("created");
    let AcceptOutcome::Queued { token, .. } =
        accept(&db, "op-create", "created", FinderAcceptKind::Create { row: &created }).unwrap()
    else {
        panic!("a create is queued");
    };
    let (write_id, base) = held_pair(&db, "created");
    assert_eq!(base, Some(0), "a create holds b = 0, set with its write id");
    assert_eq!(format!("0:w{}", write_id.unwrap()), token);

    db.upsert_file(&row("versioned")).unwrap();
    let mut contract = db.get_file_contract_state("versioned").unwrap().unwrap();
    contract.current_version = 2;
    db.set_file_contract_state(&contract).unwrap();
    let AcceptOutcome::Queued { token, .. } = accept(
        &db,
        "op-modify",
        "versioned",
        FinderAcceptKind::Modify {
            incoming_base: Some("2"),
        },
    )
    .unwrap() else {
        panic!("a modify on a known base is queued");
    };
    let (write_id, base) = held_pair(&db, "versioned");
    assert_eq!(base, Some(2), "a modify holds its decided base, set with its write id");
    assert_eq!(format!("2:w{}", write_id.unwrap()), token);

    // A write id without its base can only come from a bug: reading it fails.
    db.0.lock()
        .unwrap()
        .execute("UPDATE files SET held_base = NULL WHERE file_id = 'versioned'", [])
        .unwrap();
    assert!(
        db.item_presentation("versioned").is_err(),
        "a NULL held_base is never read as a base of 0"
    );
    assert!(
        accept(
            &db,
            "op-after-bug",
            "versioned",
            FinderAcceptKind::Modify {
                incoming_base: Some("2"),
            },
        )
        .is_err(),
        "no save is mapped through a half-set row"
    );
    assert_eq!(
        db.list_operations_for_file("versioned").unwrap().len(),
        1,
        "the refused accept queued nothing"
    );
}

#[test]
fn item_presentation_reads_the_held_write_and_the_queue_facts() {
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    let held = "a".repeat(32);
    let other = "b".repeat(32);
    db.0.lock()
        .unwrap()
        .execute(
            "INSERT INTO files (file_id, path, status, current_version, current_object_version_id,
                                    held_write_id, held_base, held_version, held_object_version_id, version_filled)
                 VALUES ('f1', 'a.txt', 'local', 4, 'o4', ?1, 3, 4, 'o4', 1)",
            params![held],
        )
        .unwrap();
    assert!(
        db.item_presentation("missing").unwrap().is_none(),
        "no row, no presentation"
    );

    let presentation = db.item_presentation("f1").unwrap().unwrap();
    assert_eq!(presentation.contract.current_version, 4);
    assert_eq!(
        presentation.held,
        Some(crate::write_token::HeldWrite {
            write_id: held.clone(),
            base: 3,
            version: Some(4),
            object_version_id: Some("o4".into()),
        }),
        "the held columns map one to one"
    );
    assert!(presentation.version_filled);

    // (op_id, kind, file_id, write_id, attempts, max_attempts)
    type Op<'a> = (&'a str, &'a str, &'a str, Option<&'a str>, i64, i64);
    let check = |ops: &[Op<'_>], queued: bool, unparked: bool, why: &str| {
        {
            let conn = db.0.lock().unwrap();
            conn.execute("DELETE FROM operation_queue", []).unwrap();
            for (op_id, kind, file_id, write_id, attempts, max_attempts) in ops {
                conn.execute(
                    "INSERT INTO operation_queue (op_id, kind, file_id, write_id, attempts, max_attempts)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![op_id, kind, file_id, write_id, attempts, max_attempts],
                )
                .unwrap();
            }
        }
        let presentation = db.item_presentation("f1").unwrap().unwrap();
        assert_eq!(presentation.held_write_queued, queued, "held_write_queued: {why}");
        assert_eq!(
            presentation.unparked_finder_upload, unparked,
            "unparked_finder_upload: {why}"
        );
    };
    let h = Some(held.as_str());
    let o = Some(other.as_str());

    check(&[], false, false, "an empty queue");
    check(
        &[("op", "upload_version", "f1", h, 0, 5)],
        true,
        true,
        "a queued Finder upload of the held write",
    );
    check(
        &[("op", "upload_version", "f1", h, 4, 5)],
        true,
        true,
        "one attempt left is not parked",
    );
    check(
        &[("op", "upload_version", "f1", h, 5, 5)],
        true,
        false,
        "attempts = max_attempts is parked, and still queued",
    );
    check(
        &[("op", "upload_version", "f1", h, 6, 5)],
        true,
        false,
        "attempts past max_attempts is parked",
    );
    check(
        &[("op", "upload_file", "f1", o, 0, 5)],
        false,
        true,
        "another write's create counts as a Finder upload",
    );
    check(
        &[("op", "rename_file", "f1", o, 0, 5)],
        false,
        false,
        "only upload kinds are Finder uploads",
    );
    check(
        &[("op", "upload_version", "f1", None, 0, 5)],
        false,
        false,
        "an upload without a write id is not a Finder upload",
    );
    check(
        &[("op", "upload_version", "f2", o, 0, 5)],
        false,
        false,
        "another file's upload",
    );

    db.0.lock()
        .unwrap()
        .execute("UPDATE files SET held_write_id = NULL WHERE file_id = 'f1'", [])
        .unwrap();
    check(
        &[("op", "upload_version", "f1", h, 0, 5)],
        false,
        true,
        "no held write, nothing to find",
    );
    assert!(db.item_presentation("f1").unwrap().unwrap().held.is_none());
}

#[test]
fn finder_write_maps_its_columns_and_reads_an_unknown_origin_as_earlier_build() {
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    db.0.lock()
        .unwrap()
        .execute_batch(
            "INSERT INTO operation_queue (op_id, kind, file_id, write_id, write_origin, after_write_id, base_pending)
                 VALUES ('minted', 'upload_version', 'f1', 'w1', 'minted', 'w0', 2),
                        ('earlier', 'upload_file', 'f1', 'w2', 'earlier_build', NULL, 0),
                        ('no-origin', 'upload_version', 'f1', 'w3', NULL, NULL, 0),
                        ('odd-origin', 'upload_version', 'f1', 'w4', 'Minted', NULL, 0),
                        ('plain', 'upload_version', 'f1', NULL, 'minted', 'w0', 1);",
        )
        .unwrap();

    assert_eq!(
        db.finder_write("minted").unwrap(),
        Some(FinderWrite {
            write_id: "w1".into(),
            origin: WriteOrigin::Minted,
            after_write_id: Some("w0".into()),
            base_pending: 2,
        })
    );
    assert_eq!(
        db.finder_write("earlier").unwrap(),
        Some(FinderWrite {
            write_id: "w2".into(),
            origin: WriteOrigin::EarlierBuild,
            after_write_id: None,
            base_pending: 0,
        })
    );
    assert_eq!(
        db.finder_write("no-origin").unwrap().unwrap().origin,
        WriteOrigin::EarlierBuild,
        "a write id without an origin is never treated as minted"
    );
    assert_eq!(
        db.finder_write("odd-origin").unwrap().unwrap().origin,
        WriteOrigin::EarlierBuild,
        "an origin this build does not know reads as earlier-build"
    );
    assert_eq!(
        db.finder_write("plain").unwrap(),
        None,
        "no write id, not a Finder write"
    );
    assert_eq!(db.finder_write("missing").unwrap(), None, "no op");
}

#[test]
fn write_origin_round_trips_through_its_stored_form() {
    for origin in [WriteOrigin::Minted, WriteOrigin::EarlierBuild] {
        assert_eq!(WriteOrigin::from_db(Some(origin.as_str())), Some(origin), "{origin:?}");
    }
    // The stored strings are schema: a row one build writes, the next build reads.
    assert_eq!(WriteOrigin::Minted.as_str(), "minted");
    assert_eq!(WriteOrigin::EarlierBuild.as_str(), "earlier_build");
    assert_eq!(WriteOrigin::from_db(None), None);
    assert_eq!(WriteOrigin::from_db(Some("")), None);
    assert_eq!(WriteOrigin::from_db(Some("Minted")), None);
}

/// T66, spec §10.2 (m-4c) and m-4a: on a re-upgrade after a downgrade, the held columns of a
/// file with an upload queued without a write id are cleared before that upload is marked, so
/// no token the system no longer holds is revived. A waiting op stores its token's `b`.
#[test]
fn a_re_upgrade_clears_stale_held_columns_and_waiting_ops_store_b() {
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    seed_own_row(&db, "downgraded", FileStatus::Local, 10);
    let mut contract = db.get_file_contract_state("downgraded").unwrap().unwrap();
    contract.current_version = 2;
    db.set_file_contract_state(&contract).unwrap();
    db.track_staged_payload("/staged/d", None, false).unwrap();
    // Borrowed parameters, not `&'static str`: the token below is an owned `String`.
    let accept = |op_id: &str, base: &str| {
        db.accept_finder_write(
            &FinderAccept {
                retire_parked: None,
                op_id,
                file_id: "downgraded",
                kind: FinderAcceptKind::Modify {
                    incoming_base: Some(base),
                },
                parent_id: None,
                target_path: Some("d.txt"),
                metadata_json: "{}",
                payload_path: "/staged/d",
                size_bytes: 1,
                modified_at: 1,
                backup_source_key: None,
                now: 1,
            },
            &|_, _| Vec::new(),
        )
        .unwrap()
    };
    let AcceptOutcome::Queued { token, .. } = accept("w1", "2") else {
        panic!("queued")
    };
    let AcceptOutcome::Queued { .. } = accept("w2", &token) else {
        panic!("queued")
    };
    assert_eq!(
        db.get_operation("w2").unwrap().unwrap().base_version,
        Some(2),
        "m-4a: the waiting op stores its b"
    );
    let this_build = ["w1", "w2"].map(|op_id| db.finder_write(op_id).unwrap().unwrap());
    assert!(
        this_build.iter().all(|write| write.origin == WriteOrigin::Minted),
        "precondition: this build minted both"
    );
    // A downgraded build then queued an upload of the same file without a write id.
    db.enqueue_operation(&queued("older-build", OperationKind::UploadVersion, "downgraded", None))
        .unwrap();
    assert_eq!(
        db.mark_earlier_build_uploads().unwrap(),
        1,
        "only the upload without a write id is marked"
    );
    assert!(
        db.item_presentation("downgraded").unwrap().unwrap().held.is_none(),
        "m-4c: the held columns are cleared"
    );
    let marked = db.finder_write("older-build").unwrap().unwrap();
    assert_eq!(marked.origin, WriteOrigin::EarlierBuild);
    for (op_id, before) in ["w1", "w2"].iter().zip(&this_build) {
        assert_eq!(
            &db.finder_write(op_id).unwrap().unwrap(),
            before,
            "{op_id}: this build's write keeps its write id and stays minted"
        );
    }
    // The next engine start marks nothing: an op is marked once.
    assert_eq!(
        db.mark_earlier_build_uploads().unwrap(),
        0,
        "a second start marks nothing"
    );
    assert_eq!(
        db.finder_write("older-build").unwrap().unwrap(),
        marked,
        "the marked op keeps its write id"
    );
}

/// Spec §10.2 with the auth pauses (spec 2026-10-06, F9): an upload an earlier build queued that is
/// paused for `auth` or `key_replaced` is marked like any other and keeps its pause. The marking
/// adds a write id and the earlier-build origin; it never makes a paused op due and keeps its
/// attempts and base. A same-account sign-in then resumes only the `auth` one, still marked;
/// the `key_replaced` one stays kept and never sent.
#[test]
fn an_earlier_builds_paused_upload_keeps_its_pause_through_the_marking() {
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    let enqueue_paused = |op_id: &str, file_id: &str| {
        seed_own_row(&db, file_id, FileStatus::Uploading, 10);
        let mut op = queued(
            op_id,
            OperationKind::UploadVersion,
            file_id,
            Some(&format!("/staged/{op_id}")),
        );
        op.attempts = 2;
        op.base_version = Some(1);
        db.enqueue_operation(&op).unwrap();
        db.record_operation_pause(op_id, OperationPauseReason::Auth, Some("HTTP 401 Unauthorized"), 5)
            .unwrap();
    };
    // The key-replaced path re-marks every auth pause, so that one is paused first.
    enqueue_paused("key-replaced", "f-key");
    assert_eq!(db.hold_operations_paused_for_auth_as_key_replaced(6).unwrap(), 1);
    enqueue_paused("auth-paused", "f-auth");
    let paused = |op_id: &str| -> Option<String> {
        db.0.lock()
            .unwrap()
            .query_row(
                "SELECT paused_reason FROM operation_queue WHERE op_id = ?1",
                [op_id],
                |row| row.get(0),
            )
            .unwrap()
    };

    assert_eq!(db.mark_earlier_build_uploads().unwrap(), 2, "both are marked");

    for op_id in ["auth-paused", "key-replaced"] {
        assert_eq!(
            db.finder_write(op_id).unwrap().expect("a write id").origin,
            WriteOrigin::EarlierBuild
        );
        let op = db.get_operation(op_id).unwrap().unwrap();
        assert_eq!(
            (op.attempts, op.base_version),
            (2, Some(1)),
            "{op_id}: attempts and base kept"
        );
    }
    assert_eq!(paused("auth-paused").as_deref(), Some("auth"), "the auth pause is kept");
    assert_eq!(
        paused("key-replaced").as_deref(),
        Some("key_replaced"),
        "the key-replaced pause is kept"
    );
    assert!(db.list_due_operations(i64::MAX).unwrap().is_empty(), "neither is due");

    assert_eq!(db.resume_operations_paused_for_auth(600).unwrap(), 1);
    let due: Vec<String> = db
        .list_due_operations(600)
        .unwrap()
        .into_iter()
        .map(|op| op.op_id)
        .collect();
    assert_eq!(due, vec!["auth-paused".to_string()], "only the auth one resumes");
    assert_eq!(
        db.finder_write("auth-paused").unwrap().unwrap().origin,
        WriteOrigin::EarlierBuild,
        "still marked after the resume"
    );
    assert_eq!(
        paused("key-replaced").as_deref(),
        Some("key_replaced"),
        "kept, never sent"
    );
}

#[test]
fn a_restore_and_an_upload_of_one_file_wait_for_each_other_in_queue_order() {
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    db.enqueue_operation(&queued("u1", OperationKind::UploadVersion, "f", None))
        .unwrap();
    db.enqueue_operation(&queued("r", OperationKind::RestoreVersion, "f", None))
        .unwrap();
    db.enqueue_operation(&queued("u2", OperationKind::UploadVersion, "f", None))
        .unwrap();
    // A restore and an upload wait for each other only when the upload is a Finder write.
    db.set_write_id_for_test("u1", &"1".repeat(32));
    db.set_write_id_for_test("u2", &"2".repeat(32));

    assert!(
        matches!(db.claim_operation("r", 1).unwrap(), ClaimOutcome::Wait),
        "a restore waits for an earlier upload"
    );
    let ClaimOutcome::Claimed(u1) = db.claim_operation("u1", 1).unwrap() else {
        panic!("u1 is first")
    };
    assert!(db.finish_claimed("u1", &u1.claim_id, None).unwrap());
    assert!(
        matches!(db.claim_operation("u2", 1).unwrap(), ClaimOutcome::Wait),
        "an upload waits for an earlier restore"
    );
    assert!(matches!(db.claim_operation("r", 1).unwrap(), ClaimOutcome::Claimed(_)));

    // The watcher's and Windows' uploads carry no write id. A restore does not wait
    // for them, and they do not wait for a restore: round 3's upload-only order.
    db.enqueue_operation(&queued("g-u0", OperationKind::UploadVersion, "g", None))
        .unwrap();
    db.enqueue_operation(&queued("g-r", OperationKind::RestoreVersion, "g", None))
        .unwrap();
    db.enqueue_operation(&queued("g-u1", OperationKind::UploadVersion, "g", None))
        .unwrap();
    assert!(
        matches!(db.claim_operation("g-r", 1).unwrap(), ClaimOutcome::Claimed(_)),
        "a restore does not wait for an earlier upload without a write id"
    );
    let ClaimOutcome::Claimed(g_u0) = db.claim_operation("g-u0", 1).unwrap() else {
        panic!("g-u0 is first")
    };
    assert!(db.finish_claimed("g-u0", &g_u0.claim_id, None).unwrap());
    assert!(
        matches!(db.claim_operation("g-u1", 1).unwrap(), ClaimOutcome::Claimed(_)),
        "an upload without a write id does not wait for an earlier restore, still queued"
    );
    assert!(db.get_operation("g-r").unwrap().is_some());

    assert!(matches!(db.claim_operation("gone", 1).unwrap(), ClaimOutcome::Gone));
}

#[test]
fn engine_start_clears_claims_and_releases_only_unreferenced_completed_payloads() {
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    db.enqueue_operation(&queued(
        "u1",
        OperationKind::UploadVersion,
        "f",
        Some("/staged/referenced"),
    ))
    .unwrap();
    let ClaimOutcome::Claimed(stale) = db.claim_operation("u1", 1).unwrap() else {
        panic!("claimable")
    };
    db.track_staged_payload("/staged/free", None, true).unwrap();
    db.track_staged_payload("/staged/referenced", None, true).unwrap();
    db.track_staged_payload("/staged/unfinished", None, false).unwrap();
    // A copy only a resume row references (Keep Mine's one-shot op has no queue row),
    // and one only a Windows finalization references.
    db.put_upload_resume(&UploadResume {
        op_id: "inline".into(),
        payload_path: "/staged/resumed".into(),
        payload_size: 1,
        payload_mtime_ns: 1,
        upload_session_id: "session".into(),
        server_file_id: "f".into(),
        object_version_id: "object".into(),
        chunk_size_bytes: 1,
        chunk_count: 1,
        acked_chunks: 0,
        metadata_applied: false,
        is_create: false,
        self_minted: false,
        completed_version: None,
        completed_object_version_id: None,
        completed_mime_type: None,
    })
    .unwrap();
    db.track_staged_payload("/staged/resumed", None, true).unwrap();
    db.put_upload_finalization(&UploadFinalization {
        op_id: "finalizing".into(),
        local_file_id: "f".into(),
        server_file_id: "f".into(),
        target_path: "a.txt".into(),
        payload_path: "/staged/finalizing".into(),
        stamped: false,
        review: None,
    })
    .unwrap();

    let repair = db.engine_start_repair().unwrap();

    assert_eq!(repair.claims_cleared, 1);
    // The release journal is macOS-only: elsewhere nothing is released at engine start.
    #[cfg(target_os = "macos")]
    assert_eq!(repair.released_payloads, vec!["/staged/free".to_string()]);
    #[cfg(not(target_os = "macos"))]
    assert!(repair.released_payloads.is_empty(), "{:?}", repair.released_payloads);
    assert!(
        !db.record_attempt_claimed("u1", &stale.claim_id, 1, 0, None).unwrap(),
        "a claim from before the restart guards nothing"
    );
}

/// Spec §8.4 (the staging folder) and §8.7 S6: engine start unlinks a released copy wherever it was staged,
/// by its journalled absolute path: in the data root, and in the old cache root that builds before the move
/// used. A copy a queued op still owns stays in each root, with its journal row. (The release journal is
/// macOS-only, so the test is too.)
#[cfg(target_os = "macos")]
#[test]
fn engine_start_unlinks_released_copies_in_the_new_and_the_old_root_and_keeps_live_ones() {
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    let bases = crate::engine_bridge::FinderStagingBases::current();
    let mut released = Vec::new();
    let mut live = Vec::new();
    for (label, root) in [
        ("new", bases.durable_root().expect("the test sandbox has a data dir")),
        ("old", bases.cache_root()),
    ] {
        std::fs::create_dir_all(&root).unwrap();
        // Landed: marked released in the journal, and no op, resume row or finalization names it.
        let done = root.join(uuid::Uuid::new_v4().to_string());
        std::fs::write(&done, b"landed").unwrap();
        db.track_staged_payload(&done.to_string_lossy(), None, true).unwrap();
        // Still queued: an upload reads it.
        let queued_copy = root.join(uuid::Uuid::new_v4().to_string());
        std::fs::write(&queued_copy, b"waiting").unwrap();
        db.track_staged_payload(&queued_copy.to_string_lossy(), Some("/source"), false)
            .unwrap();
        db.enqueue_operation(&queued(
            &format!("live-{label}"),
            OperationKind::UploadVersion,
            &format!("f-{label}"),
            Some(&queued_copy.to_string_lossy()),
        ))
        .unwrap();
        released.push(done);
        live.push(queued_copy);
    }

    let repair = db.engine_start_repair().unwrap();
    let removed = crate::staged_payload::remove_released(&db, &repair.released_payloads);

    assert_eq!(removed, 2, "one released copy in each root");
    for path in &released {
        assert!(!path.exists(), "a released copy is unlinked: {path:?}");
    }
    for path in &live {
        assert!(path.exists(), "a copy a queued op owns stays: {path:?}");
    }
    let mut journalled: Vec<String> = db
        .staged_payloads_for_signout()
        .unwrap()
        .into_iter()
        .map(|(path, _, _)| path)
        .collect();
    journalled.sort();
    let mut expected: Vec<String> = live.iter().map(|path| path.to_string_lossy().into_owned()).collect();
    expected.sort();
    assert_eq!(journalled, expected, "only the live copies keep their journal rows");
    for path in &live {
        std::fs::remove_file(path).unwrap();
    }
}

/// The sign-in's auth resume (spec 2026-10-06, F9) never makes a parked Finder write due again. That holds
/// for one its own attempt parked (`stale_base`, `payload_missing`) and for one the snapshot count parked
/// (`base_unknown`) while it was paused for `auth`, so a parked write is never resumed as if it were only
/// auth-paused. Each keeps its park: attempts used up, and its reason. Only the auth-paused write that never
/// parked is due again.
#[test]
fn the_auth_resume_never_makes_a_parked_finder_write_due() {
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    for (op_id, reason) in [
        ("w-stale", ParkReason::StaleBase),
        ("w-missing", ParkReason::PayloadMissing),
    ] {
        db.enqueue_operation(&queued(
            op_id,
            OperationKind::UploadVersion,
            op_id,
            Some(&format!("/staged/{op_id}")),
        ))
        .unwrap();
        let ClaimOutcome::Claimed(claimed) = db.claim_operation(op_id, 1).unwrap() else {
            panic!("{op_id} is claimable")
        };
        assert!(db.park_claimed(op_id, &claimed.claim_id, reason, 2).unwrap());
        db.set_write_id_for_test(op_id, &format!("{op_id}-write"));
    }
    // Waiting for its base and paused for `auth`; the 10th snapshot without its base parks it.
    db.enqueue_operation(&queued(
        "w-base",
        OperationKind::UploadVersion,
        "w-base",
        Some("/staged/w-base"),
    ))
    .unwrap();
    db.0.lock()
        .unwrap()
        .execute(
            "UPDATE operation_queue SET base_pending = 1, write_id = 'w-base-write' WHERE op_id = 'w-base'",
            [],
        )
        .unwrap();
    db.record_operation_pause("w-base", OperationPauseReason::Auth, Some("HTTP 401 Unauthorized"), 3)
        .unwrap();
    for snapshot in 1..10 {
        assert!(db.note_snapshot_for_base_pending(10 + snapshot).unwrap().is_empty());
    }
    assert_eq!(
        db.note_snapshot_for_base_pending(20).unwrap(),
        vec![("w-base".to_string(), Some("w-base".to_string()))],
        "precondition: parked while paused for auth"
    );
    // Paused for `auth`, never parked.
    db.enqueue_operation(&queued(
        "w-auth",
        OperationKind::UploadVersion,
        "w-auth",
        Some("/staged/w-auth"),
    ))
    .unwrap();
    db.set_write_id_for_test("w-auth", "w-auth-write");
    db.record_operation_pause("w-auth", OperationPauseReason::Auth, Some("HTTP 401 Unauthorized"), 4)
        .unwrap();
    let due = || {
        db.list_due_operations(i64::MAX)
            .unwrap()
            .into_iter()
            .map(|op| op.op_id)
            .collect::<Vec<_>>()
    };
    assert!(due().is_empty(), "precondition: nothing is due");

    db.resume_operations_paused_for_auth(500).unwrap();

    let still_paused_for_auth: i64 =
        db.0.lock()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM operation_queue WHERE paused_reason = 'auth'",
                [],
                |row| row.get(0),
            )
            .unwrap();
    assert_eq!(still_paused_for_auth, 0, "nothing paused for auth remains");
    assert_eq!(due(), vec!["w-auth".to_string()], "only the write that never parked");
    for (op_id, reason) in [
        ("w-stale", ParkReason::StaleBase),
        ("w-missing", ParkReason::PayloadMissing),
        ("w-base", ParkReason::BaseUnknown),
    ] {
        assert!(!due().contains(&op_id.to_string()), "{op_id} is not due");
        let op = db.get_operation(op_id).unwrap().expect("a parked write keeps its op");
        assert_eq!(op.attempts, op.max_attempts, "{op_id} stays parked");
        assert_eq!(
            op.last_error.as_deref(),
            Some(reason.as_str()),
            "{op_id} keeps its reason"
        );
        assert_eq!(
            op.payload_path.as_deref(),
            Some(format!("/staged/{op_id}").as_str()),
            "{op_id} keeps its bytes"
        );
    }
}
