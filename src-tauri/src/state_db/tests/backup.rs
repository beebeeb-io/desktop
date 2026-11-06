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
