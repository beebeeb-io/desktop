//! LD-0113: the backup's local state (`state_db/backup.rs`) and the schema rules that keep a backup source out of
//! every table desktop deletes paths from.

use super::*;

    fn backup_db() -> (tempfile::TempDir, StateDb) {
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        db.set_owner(&crate::account_binding::Identity::new(Some("u-a"), Some("a@beebeeb.io")))
            .unwrap();
        (dir, db)
    }

    fn raw(db: &StateDb, sql: &str) -> rusqlite::Result<usize> {
        db.0.lock().unwrap().execute(sql, [])
    }

    /// Spec §6.1 point 4: a backup op keeps its source in `source_path` and never in `payload_path`, which desktop
    /// deletes after an upload (`staged_payload::remove`, no gate). The table refuses it, whoever writes the row: an
    /// insert, a later update, a sync op with a source, a sync-path session or finalization for a backup op, and a
    /// staged-copy journal row naming a queued backup source. Each refusal is counted; an allowed row is too.
    #[test]
    fn a_backup_op_never_holds_its_source_where_desktop_deletes_paths() {
        let (_dir, db) = backup_db();
        let refused = |result: rusqlite::Result<usize>| match result {
            Err(error) => error.to_string().contains("ld0113"),
            Ok(_) => false,
        };
        let mut refusals = 0;
        refusals += usize::from(refused(raw(
            &db,
            "INSERT INTO operation_queue (op_id, kind, origin, source_path, payload_path)
             VALUES ('op-mutant', 'upload_file', 'backup', '/home/p/Pictures/a.jpg', '/home/p/Pictures/a.jpg')",
        )));
        refusals += usize::from(refused(raw(
            &db,
            "INSERT INTO operation_queue (op_id, kind, origin, payload_path)
             VALUES ('op-no-source', 'upload_file', 'backup', NULL)",
        )));
        refusals += usize::from(refused(raw(
            &db,
            "INSERT INTO operation_queue (op_id, kind, origin, source_path)
             VALUES ('op-sync-with-source', 'upload_file', 'sync', '/home/p/Pictures/a.jpg')",
        )));
        refusals += usize::from(refused(raw(
            &db,
            "INSERT INTO operation_queue (op_id, kind, origin, source_path)
             VALUES ('op-other-origin', 'upload_file', 'mirror', NULL)",
        )));
        // The one allowed shape: origin backup, a source, no payload.
        assert_eq!(
            raw(
                &db,
                "INSERT INTO operation_queue (op_id, kind, origin, source_path)
                 VALUES ('op-backup', 'upload_file', 'backup', '/home/p/Pictures/a.jpg')",
            )
            .unwrap(),
            1
        );
        refusals += usize::from(refused(raw(
            &db,
            "UPDATE operation_queue SET payload_path = source_path WHERE op_id = 'op-backup'",
        )));
        refusals += usize::from(refused(raw(
            &db,
            "UPDATE operation_queue SET origin = 'sync' WHERE op_id = 'op-backup'",
        )));
        refusals += usize::from(refused(raw(
            &db,
            "INSERT INTO upload_resume (op_id, payload_path, payload_size, payload_mtime_ns, upload_session_id,
                 server_file_id, object_version_id, chunk_size_bytes, chunk_count)
             VALUES ('op-backup', '/home/p/Pictures/a.jpg', 1, 1, 's', 'f', 'o', 1, 1)",
        )));
        refusals += usize::from(refused(raw(
            &db,
            "INSERT INTO upload_finalizations (op_id, local_file_id, server_file_id, target_path, payload_path)
             VALUES ('op-backup', 'f', 'f', 'a.jpg', '/home/p/Pictures/a.jpg')",
        )));
        refusals += usize::from(refused(raw(
            &db,
            "INSERT INTO staged_payloads (path, completed) VALUES ('/home/p/Pictures/a.jpg', 0)",
        )));
        assert_eq!(refusals, 9, "every one of the 9 shapes that would let desktop delete a source is refused");
        // A sync op and its staged copy are untouched by the rule.
        assert_eq!(
            raw(
                &db,
                "INSERT INTO operation_queue (op_id, kind, payload_path) VALUES ('op-sync', 'upload_file', '/staged/1')",
            )
            .unwrap(),
            1
        );
        assert_eq!(
            raw(&db, "INSERT INTO staged_payloads (path, completed) VALUES ('/staged/1', 0)").unwrap(),
            1
        );
        let backup = db.backup_upload_op("op-backup").unwrap().expect("the backup op reads back");
        assert_eq!(backup.source_path, std::path::PathBuf::from("/home/p/Pictures/a.jpg"));
        assert_eq!(
            db.get_operation("op-backup").unwrap().expect("queued").payload_path,
            None,
            "the generic reader sees no payload for a backup op"
        );
        assert!(db.backup_upload_op("op-sync").unwrap().is_none(), "a sync op is never read as a backup op");
    }

    /// The open-time inventory (`staged_payloads` from every `upload_resume` row, `schema.rs`) never names a queued
    /// backup source: the `staged_payloads` trigger would abort the open itself, and a database that cannot open stops
    /// the app. Here a sync session's payload and a backup op's source are one path, a state no writer makes, so only
    /// the open is under test.
    #[test]
    fn an_open_never_fails_on_a_session_naming_a_backup_source() {
        let (dir, db) = backup_db();
        for sql in [
            "INSERT INTO operation_queue (op_id, kind, payload_path) VALUES ('op-sync', 'upload_file', '/shared/a.jpg')",
            "INSERT INTO upload_resume (op_id, payload_path, payload_size, payload_mtime_ns, upload_session_id,
                 server_file_id, object_version_id, chunk_size_bytes, chunk_count)
             VALUES ('op-sync', '/shared/a.jpg', 1, 1, 's', 'f', 'o', 1, 1)",
            "INSERT INTO operation_queue (op_id, kind, origin, source_path)
             VALUES ('op-backup', 'upload_file', 'backup', '/shared/a.jpg')",
        ] {
            assert_eq!(raw(&db, sql).unwrap(), 1, "{sql}");
        }
        drop(db);
        let reopened = StateDb::open(dir.path().join("state.db"));
        let reopened = match reopened {
            Ok(db) => db,
            Err(error) => panic!("the open failed: {error}"),
        };
        let journalled: i64 = reopened
            .0
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM staged_payloads", [], |row| row.get(0))
            .unwrap();
        assert_eq!(journalled, 0, "the backup source was not journalled as a staged copy");
    }

    /// The trigger order fix behind #231's macOS red (`reauth_tests` … `an_unreadable_count_settles_nothing`), on every
    /// platform: a database whose queue lost `payload_path` (the damage that test simulates) can be made, opened again
    /// (the open re-creates the LD-0113 triggers on a queue without the column) and read: the owner reads, and the
    /// count it is counted by does not, so nothing says "0 changes".
    #[test]
    fn a_queue_without_payload_path_opens_and_its_count_is_unreadable() {
        let (dir, db) = backup_db();
        let path = dir.path().join("state.db");
        drop(db);
        {
            let conn = rusqlite::Connection::open(&path).unwrap();
            let named: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type = 'trigger' AND tbl_name = 'operation_queue'
                       AND sql LIKE '%payload_path%'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(named, 2, "the premise: two LD-0113 triggers name payload_path");
            StateDb::drop_column_for_test(&conn, "operation_queue", "payload_path")
                .expect("the column is dropped with the triggers that name it");
        }
        let reopened = StateDb::open(&path).expect("the open re-creates the triggers on a queue without the column");
        assert!(reopened.backup_owner_key().unwrap().is_some(), "the owner still reads");
        assert!(reopened.pending_changes_count().is_err(), "the count is unreadable, never 0");
    }
