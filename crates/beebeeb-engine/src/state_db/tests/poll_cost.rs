//! Task 2243's poll-cost guards, ported from the old shell crate's `task_2243_poll_cost_tests.rs`: what the tray's
//! polls cost the disk, counted in pages the connection read from the database file (`StateDb::page_reads`).
//! (The old session-poll source guard and the shell's `sync_status_counts` test went with the shell.)

use super::*;

/// A state.db at `path` with `rows` files of each status, written through another connection as the engine writes
/// them. The paths are long, as real ones are: reading the rows reads many pages.
fn seeded_state_db(path: &Path, rows: &[(&str, usize)]) {
    StateDb::open(path).unwrap();
    let mut conn = Connection::open(path).unwrap();
    let tx = conn.transaction().unwrap();
    for (status, count) in rows {
        for i in 0..*count {
            tx.execute(
                "INSERT INTO files (file_id, path, status, size_bytes, modified_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    format!("{status}-{i}"),
                    format!("Documents/Projects/Customer archive {i:05}/Correspondence and drafts/{status}-{i}.docx"),
                    status,
                    1_000 + i as i64,
                    1_700_000_000 + i as i64,
                ],
            )
            .unwrap();
        }
    }
    tx.commit().unwrap();
}

/// Ported from `task_2243_the_tray_recent_list_reads_only_the_newest_rows`.
///
/// The tray's recent list is the overview's `recent`, read from the newest rows alone. The overview loads every row
/// for its totals: 5,461 page reads and 117 ms on a 43k-row state.db, every 5 s while the tray was loaded.
#[test]
fn task_2243_the_tray_recent_list_reads_only_the_newest_rows() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    // Equal modification times across the two statuses, so ties are broken by path.
    seeded_state_db(&path, &[("local", 3000), ("cloud_only", 3000)]);
    {
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "INSERT INTO files (file_id, path, status, item_kind, modified_at)
                 VALUES ('folder-new', 'Documents/A newer folder', 'local', 'folder', 1800000000);
             UPDATE files SET pin_state = 'pinned' WHERE file_id IN ('local-2999', 'cloud_only-2990');
             UPDATE files SET pin_state = 'inherit', inherited_pin_state = 'pinned' WHERE file_id = 'local-2995';
             INSERT INTO local_activity (event_type, file_id, file_name, rel_path, occurred_at)
                 VALUES ('moved_to_trash', 'gone-1', 'old plan.docx', 'Documents/old plan.docx', 1700002990),
                        ('moved_to_trash', 'gone-2', 'older.docx', NULL, 1600000000);",
        )
        .unwrap();
    }
    let expected = {
        let db = StateDb::open(&path).unwrap();
        crate::account_dto::compute_file_overview(
            &crate::status_views::file_overview_inputs(db.file_overview_rows().unwrap()),
            &crate::status_views::file_overview_activity(&db, 50).unwrap(),
            50,
        )
        .recent
    };
    assert_eq!(expected.len(), 50);
    assert!(
        expected.iter().any(|f| f.activity_type.is_some()),
        "precondition: a trash event is in the list"
    );
    assert!(
        expected.iter().filter(|f| f.pinned).count() >= 2,
        "precondition: pinned rows are in the list"
    );

    let db = StateDb::open(&path).unwrap();
    let recent = crate::status_views::recent_files(&db, 50).unwrap();
    let reads = db.page_reads();
    assert_eq!(
        serde_json::to_value(&recent).unwrap(),
        serde_json::to_value(&expected).unwrap()
    );
    assert!(
        reads <= 60,
        "the tray's recent list (the open and its reads) read {reads} pages from disk"
    );
}

/// Ported from `task_2243_the_hold_summary_reads_an_index_not_the_held_operations`.
///
/// The tray's activity line asks for the queue's hold summary every 3 s. Its oldest and newest dates per reason come
/// from an index alone; looking each held operation up read the whole queue (627 pages on one PC, with 2,972 changes
/// held since a sign-in expired).
#[test]
fn task_2243_the_hold_summary_reads_an_index_not_the_held_operations() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    StateDb::open(&path).unwrap();
    {
        let mut conn = Connection::open(&path).unwrap();
        let tx = conn.transaction().unwrap();
        for i in 0..3000_i64 {
            // Held ops carry their metadata, as real ones do: reading them reads many pages.
            let (reason, attempts) = match i % 3 {
                0 => (Some("auth"), 0),
                1 => (Some("quota"), 0),
                _ => (None, if i % 2 == 0 { 5 } else { 1 }),
            };
            tx.execute(
                "INSERT INTO operation_queue (op_id, kind, file_id, target_path, metadata_json, attempts, max_attempts,
                                              paused_reason, created_at)
                 VALUES (?1, 'upload_file', ?2, ?3, ?4, ?5, 5, ?6, ?7)",
                params![
                    format!("op-{i}"),
                    format!("file-{i}"),
                    format!("Documents/Projects/Customer archive {i:05}/Correspondence and drafts/letter-{i}.docx"),
                    format!("{{\"name_ct\":\"{}\",\"size\":{i}}}", "x".repeat(120)),
                    attempts,
                    reason,
                    1_700_000_000 + i,
                ],
            )
            .unwrap();
        }
        tx.commit().unwrap();
    }
    let db = StateDb::open(&path).unwrap();
    let hold = db.queue_hold_summary().unwrap();
    let reads = db.page_reads();
    assert_eq!((hold.waiting, hold.gave_up), (500, 500));
    let groups: Vec<_> = hold
        .held
        .iter()
        .map(|g| (g.reason.as_str(), g.count, g.oldest_at, g.newest_at))
        .collect();
    assert_eq!(
        groups,
        [
            ("auth", 1000, 1_700_000_000, 1_700_002_997),
            ("quota", 1000, 1_700_000_001, 1_700_002_998)
        ]
    );
    assert!(
        reads <= 40,
        "the hold summary (the open and its reads) read {reads} pages from disk"
    );
}
