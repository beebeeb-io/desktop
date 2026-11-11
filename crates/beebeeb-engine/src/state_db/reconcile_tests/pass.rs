//! The reconcile pass against a real database (tasks 1888 + 2226).
use super::super::*;
use crate::held_queue::{Counts, Options};
use tempfile::{TempDir, tempdir};

const NOT_FOUND: &str = "HTTP status client error (404 Not Found) for url (https://api.example/api/v1/uploads/init)";
const SERVER_ERROR: &str = "HTTP status server error (500 Internal Server Error) for url (https://api.example/x)";
const NOW: i64 = 1_800_000_000;

fn opts() -> Options {
    Options {
        device_segment: Some("PC-1".to_string()),
        case_insensitive: true,
    }
}

/// Every directory holds a file: the pass never drops scaffolding (D5) in these tests.
fn holds_a_file(_: &str) -> bool {
    true
}

#[cfg(not(target_os = "macos"))]
/// The query plan of `sql`, one line per step.
fn query_plan(db: &StateDb, sql: &str) -> String {
    let conn = db.0.lock().unwrap();
    let mut stmt = conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
    let args = rusqlite::params_from_iter(std::iter::repeat("x").take(stmt.parameter_count()));
    let steps = stmt.query_map(args, |row| row.get::<_, String>(3)).unwrap();
    steps.map(Result::unwrap).collect::<Vec<_>>().join("\n")
}

#[cfg(not(target_os = "macos"))]
#[test]
fn a_remap_reads_the_files_table_through_an_index() {
    let pc = Pc::new();
    for sql in [REMAP_FILES_SQL, REMAP_OPS_SQL, GHOST_REFERENCES_SQL] {
        let plan = query_plan(&pc.db, sql);
        assert!(!plan.contains("SCAN files"), "a remap must not scan every row:\n{plan}");
        assert!(
            !plan.contains("SCAN operation_queue"),
            "a remap must not scan every op:\n{plan}"
        );
    }
}

#[test]
fn a_tree_holds_a_file_only_when_a_file_is_somewhere_under_it() {
    let root = tempdir().unwrap();
    assert!(
        !tree_holds_a_file(&root.path().join("missing")),
        "a missing directory holds no file"
    );
    std::fs::create_dir_all(root.path().join("a/b/c")).unwrap();
    assert!(
        !tree_holds_a_file(&root.path().join("a")),
        "empty nested directories hold no file"
    );
    std::fs::write(root.path().join("a/b/c/f.txt"), b"x").unwrap();
    assert!(
        tree_holds_a_file(&root.path().join("a")),
        "a file three levels down counts"
    );
    assert!(tree_holds_a_file(&root.path().join("a/b/c")));
}

type RowTuple = (String, Option<String>, String, i64);
type OpTuple = (
    String,
    Option<String>,
    Option<String>,
    i64,
    i64,
    Option<String>,
    Option<String>,
);

struct Pc {
    dir: TempDir,
    db: StateDb,
}

impl Pc {
    fn new() -> Self {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        Self { dir, db }
    }

    fn sql(&self, sql: &str, args: impl rusqlite::Params) {
        self.db.0.lock().unwrap().execute(sql, args).unwrap();
    }

    /// A row: `version` > 0 is server-known (the snapshot's shape: `remote_updated_at` stays 0).
    fn row(&self, id: &str, parent: Option<&str>, path: &str, folder: bool, version: i64) {
        self.sql(
                "INSERT INTO files (file_id, path, status, item_kind, current_version, parent_id, size_bytes, remote_updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, 0)",
                params![id, path, if version > 0 { "synced" } else { "local" }, if folder { "folder" } else { "file" }, version, parent],
            );
    }

    fn folder(&self, id: &str, parent: Option<&str>, path: &str, version: i64) {
        self.row(id, parent, path, true, version);
    }

    fn file(&self, id: &str, parent: Option<&str>, path: &str, version: i64) {
        self.row(id, parent, path, false, version);
    }

    /// A folder's create, paused for `auth` as on 4 Oct.
    fn create_folder(&self, op_id: &str, file_id: &str, parent: Option<&str>) {
        self.sql(
                "INSERT INTO operation_queue (op_id, kind, file_id, parent_id, attempts, max_attempts, paused_reason, created_at, updated_at)
                 VALUES (?1, 'create_folder', ?2, ?3, 0, 25, 'auth', 1, 1)",
                params![op_id, file_id, parent],
            );
    }

    /// A new file's upload as the scan queues it, with its staged payload on disk.
    fn upload(&self, op_id: &str, file_id: &str, parent: Option<&str>, created_at: i64) {
        let payload = self.dir.path().join(format!("staged-{op_id}"));
        std::fs::write(&payload, op_id.as_bytes()).unwrap();
        self.sql(
                "INSERT INTO operation_queue (op_id, kind, file_id, parent_id, metadata_json, payload_path, attempts, max_attempts,
                                              next_retry_at, created_at, updated_at)
                 VALUES (?1, 'upload_version', ?2, ?3, '{\"operation\":\"create_file\"}', ?4, 0, 25, 0, ?5, ?5)",
                params![op_id, file_id, parent, payload.to_string_lossy(), created_at],
            );
    }

    /// The op used its retries on `error`.
    fn gave_up(&self, op_id: &str, error: &str) {
        self.sql(
            "UPDATE operation_queue SET attempts = 25, last_error = ?2 WHERE op_id = ?1",
            params![op_id, error],
        );
    }

    fn pause(&self, op_id: &str, reason: &str) {
        self.sql(
            "UPDATE operation_queue SET paused_reason = ?2 WHERE op_id = ?1",
            params![op_id, reason],
        );
    }

    fn count(&self, sql: &str, args: impl rusqlite::Params) -> i64 {
        self.db
            .0
            .lock()
            .unwrap()
            .query_row(sql, args, |row| row.get(0))
            .unwrap()
    }

    /// How many rows and ops still name `id` (I1).
    fn references(&self, id: &str) -> i64 {
        self.count(
            "SELECT (SELECT COUNT(*) FROM files WHERE file_id = ?1 OR parent_id = ?1)
                      + (SELECT COUNT(*) FROM operation_queue WHERE file_id = ?1 OR parent_id = ?1)",
            params![id],
        )
    }

    fn parent_of_row(&self, id: &str) -> Option<String> {
        self.db
            .0
            .lock()
            .unwrap()
            .query_row("SELECT parent_id FROM files WHERE file_id = ?1", params![id], |row| {
                row.get(0)
            })
            .unwrap()
    }

    /// (parent, attempts, next_retry_at, last_error, paused_reason)
    fn op(&self, op_id: &str) -> (Option<String>, i64, i64, Option<String>, Option<String>) {
        self.db
                .0
                .lock()
                .unwrap()
                .query_row(
                    "SELECT parent_id, attempts, next_retry_at, last_error, paused_reason FROM operation_queue WHERE op_id = ?1",
                    params![op_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
                )
                .unwrap()
    }

    fn has_op(&self, op_id: &str) -> bool {
        self.count("SELECT COUNT(*) FROM operation_queue WHERE op_id = ?1", params![op_id]) == 1
    }

    fn due(&self) -> Vec<String> {
        self.db
            .list_due_operations(NOW)
            .unwrap()
            .into_iter()
            .map(|op| op.op_id)
            .collect()
    }

    /// Every row and op, sorted: what "unchanged" and "equal" compare.
    fn snapshot(&self) -> (Vec<RowTuple>, Vec<OpTuple>) {
        let conn = self.db.0.lock().unwrap();
        let rows = conn
            .prepare("SELECT file_id, parent_id, path, current_version FROM files ORDER BY file_id")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        let ops = conn
            .prepare(
                "SELECT op_id, file_id, parent_id, attempts, next_retry_at, last_error, paused_reason
                     FROM operation_queue ORDER BY op_id",
            )
            .unwrap()
            .query_map([], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                ))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        (rows, ops)
    }

    /// A server folder `t<n>`, the ghost `g<n>` made here at the same path (its create paused for `auth`), and a
    /// new file under the ghost whose upload gave up on the parent 404 the ghost caused.
    fn ghost_with_twin(&self, n: usize, name: &str) {
        let (t, g, f) = (format!("t{n}"), format!("g{n}"), format!("f{n}"));
        self.folder(&t, None, name, 3);
        self.folder(&g, None, name, 0);
        self.file(&f, Some(&g), &format!("{name}/{n}.jpg"), 0);
        self.create_folder(&format!("op-{g}"), &g, None);
        self.upload(&format!("op-{f}"), &f, Some(&g), 10 + n as i64);
        self.gave_up(&format!("op-{f}"), NOT_FOUND);
    }

    fn six_ghosts(&self) {
        for (n, name) in ["A", "B", "C", "D", "E", "F"].iter().enumerate() {
            self.ghost_with_twin(n + 1, name);
        }
    }

    /// A test-only trigger that makes deleting row `id` fail, so the transaction that deletes it rolls back.
    fn fail_deleting(&self, id: &str) {
        self.db
            .0
            .lock()
            .unwrap()
            .execute_batch(&format!(
                "CREATE TEMP TRIGGER stop_here BEFORE DELETE ON files WHEN OLD.file_id = '{id}'
                     BEGIN SELECT RAISE(ABORT, 'test'); END;"
            ))
            .unwrap();
    }

    fn stop_failing(&self) {
        self.db
            .0
            .lock()
            .unwrap()
            .execute_batch("DROP TRIGGER stop_here;")
            .unwrap();
    }
}

/// I1: after the pass no row and no op names the ghost; its child and its child's change moved to the twin, and the
/// change that gave up on the ghost's 404 is due again (D8: attempts 0, no error, due now).
#[test]
fn reconcile_remaps_a_ghost_and_leaves_no_reference_to_it() {
    let pc = Pc::new();
    pc.ghost_with_twin(1, "Photos");
    let counts = pc.db.reconcile_held_queue(&opts(), NOW, &holds_a_file).unwrap();
    assert_eq!(
        counts,
        Counts {
            remapped: 1,
            rearmed: 1,
            ..Counts::default()
        }
    );
    assert_eq!(pc.references("g1"), 0);
    assert!(!pc.has_op("op-g1"));
    assert_eq!(pc.parent_of_row("f1").as_deref(), Some("t1"));
    // Re-armed into the key check (cross-lane review P2-2), which resumes it under this session's key.
    assert_eq!(
        pc.op("op-f1"),
        (Some("t1".to_string()), 0, NOW, None, Some("auth".to_string()))
    );
    assert_eq!(pc.db.resume_operations_paused_for_auth(NOW).unwrap(), 1);
    assert_eq!(pc.due(), vec!["op-f1".to_string()]);
}

/// Goal 2, cross-lane review P2-2: a change D8 gives its retries back to was queued with its name sealed under the
/// key in force then, like every `auth` change. The pass puts it under `auth`, so the key check that follows the
/// pass decides it too: under a replaced key it is held as `key_replaced`, never sent.
#[test]
fn a_change_the_pass_rearms_goes_through_the_key_check() {
    let pc = Pc::new();
    pc.ghost_with_twin(1, "Photos");
    let counts = pc.db.reconcile_held_queue(&opts(), NOW, &holds_a_file).unwrap();
    assert_eq!(
        counts,
        Counts {
            remapped: 1,
            rearmed: 1,
            ..Counts::default()
        }
    );
    assert!(
        pc.due().is_empty(),
        "re-armed into the key check, never straight to due: {:?}",
        pc.due()
    );
    assert_eq!(pc.db.hold_operations_paused_for_auth_as_key_replaced(NOW).unwrap(), 1);
    assert_eq!(
        pc.op("op-f1"),
        (Some("t1".to_string()), 0, NOW, None, Some("key_replaced".to_string()))
    );
    assert!(pc.due().is_empty());
}

/// I3: the dry run's counts are the run's, and the dry run writes nothing.
#[test]
fn the_dry_run_counts_equal_the_run_and_change_nothing() {
    let pc = Pc::new();
    pc.ghost_with_twin(1, "Photos");
    pc.ghost_with_twin(2, "Music");
    pc.folder("new", None, "New", 0);
    pc.create_folder("op-new", "new", None);
    let before = pc.snapshot();
    let dry = pc.db.reconcile_dry_run(&opts(), &holds_a_file).unwrap();
    assert_eq!(pc.snapshot(), before, "a dry run writes nothing");
    assert_eq!(
        dry,
        Counts {
            remapped: 2,
            rearmed: 2,
            replay: 1,
            ..Counts::default()
        }
    );
    assert_eq!(pc.db.reconcile_held_queue(&opts(), NOW, &holds_a_file).unwrap(), dry);
}

/// I3: a second pass applies nothing; only the standing counts repeat (the ghost kept for replay).
#[test]
fn a_second_pass_changes_nothing() {
    let pc = Pc::new();
    pc.ghost_with_twin(1, "Photos");
    pc.folder("new", None, "New", 0);
    pc.create_folder("op-new", "new", None);
    pc.db.reconcile_held_queue(&opts(), NOW, &holds_a_file).unwrap();
    let after_first = pc.snapshot();
    assert_eq!(
        pc.db.reconcile_held_queue(&opts(), NOW + 60, &holds_a_file).unwrap(),
        Counts {
            replay: 1,
            ..Counts::default()
        }
    );
    assert_eq!(pc.snapshot(), after_first);
}

/// I4 (lead review of #243, fix 1): a failure inside a batch undoes that batch only; a rerun completes, and ends
/// where an uninterrupted run ends.
#[test]
fn a_failure_mid_batch_undoes_only_that_batch_and_a_rerun_completes() {
    let pc = Pc::new();
    pc.six_ghosts();
    let untouched = Pc::new();
    untouched.six_ghosts();
    let before = pc.snapshot();
    pc.fail_deleting("g4");
    assert!(
        pc.db
            .reconcile_in_batches(&opts(), NOW, 2, &holds_a_file, &mut |_, _| {})
            .is_err()
    );
    for g in ["g1", "g2"] {
        assert_eq!(pc.references(g), 0, "{g}: batch 1 committed");
    }
    let (rows, ops) = pc.snapshot();
    let of = |ids: &[&str]| {
        let rows: Vec<_> = rows.iter().filter(|r| ids.contains(&r.0.as_str())).cloned().collect();
        let ops: Vec<_> = ops
            .iter()
            .filter(|o| ids.iter().any(|id| o.0 == format!("op-{id}")))
            .cloned()
            .collect();
        (rows, ops)
    };
    let of_before = |ids: &[&str]| {
        let rows: Vec<_> = before
            .0
            .iter()
            .filter(|r| ids.contains(&r.0.as_str()))
            .cloned()
            .collect();
        let ops: Vec<_> = before
            .1
            .iter()
            .filter(|o| ids.iter().any(|id| o.0 == format!("op-{id}")))
            .cloned()
            .collect();
        (rows, ops)
    };
    let rest = ["g3", "f3", "g4", "f4", "g5", "f5", "g6", "f6"];
    assert_eq!(of(&rest), of_before(&rest), "batch 2 rolled back, batch 3 never ran");
    pc.stop_failing();
    pc.db
        .reconcile_in_batches(&opts(), NOW, 2, &holds_a_file, &mut |_, _| {})
        .unwrap();
    untouched
        .db
        .reconcile_in_batches(&opts(), NOW, 2, &holds_a_file, &mut |_, _| {})
        .unwrap();
    assert_eq!(pc.snapshot(), untouched.snapshot());
}

/// I4: a failure at the first unit of a batch keeps every batch committed before it.
#[test]
fn a_failure_at_a_batch_boundary_keeps_the_committed_batches_and_a_rerun_completes() {
    let pc = Pc::new();
    pc.six_ghosts();
    let untouched = Pc::new();
    untouched.six_ghosts();
    let before = pc.snapshot();
    pc.fail_deleting("g5");
    assert!(
        pc.db
            .reconcile_in_batches(&opts(), NOW, 2, &holds_a_file, &mut |_, _| {})
            .is_err()
    );
    for g in ["g1", "g2", "g3", "g4"] {
        assert_eq!(pc.references(g), 0, "{g}: batches 1 and 2 committed");
    }
    let (rows, ops) = pc.snapshot();
    let ids = ["g5", "f5", "g6", "f6"];
    let pick = |rows: &[RowTuple], ops: &[OpTuple]| {
        (
            rows.iter()
                .filter(|r| ids.contains(&r.0.as_str()))
                .cloned()
                .collect::<Vec<_>>(),
            ops.iter()
                .filter(|o| ids.iter().any(|id| o.0 == format!("op-{id}")))
                .cloned()
                .collect::<Vec<_>>(),
        )
    };
    assert_eq!(pick(&rows, &ops), pick(&before.0, &before.1));
    pc.stop_failing();
    pc.db
        .reconcile_in_batches(&opts(), NOW, 2, &holds_a_file, &mut |_, _| {})
        .unwrap();
    untouched
        .db
        .reconcile_in_batches(&opts(), NOW, 2, &holds_a_file, &mut |_, _| {})
        .unwrap();
    assert_eq!(pc.snapshot(), untouched.snapshot());
}

/// Spec §3 (corrected 2026-10-28): a twin the snapshot wrote (`current_version` 3, `remote_updated_at` 0) is found.
#[test]
fn a_twin_written_by_the_snapshot_is_found() {
    let pc = Pc::new();
    pc.ghost_with_twin(1, "Photos");
    assert_eq!(
        pc.count("SELECT remote_updated_at FROM files WHERE file_id = 't1'", []),
        0
    );
    assert_eq!(
        pc.db
            .reconcile_held_queue(&opts(), NOW, &holds_a_file)
            .unwrap()
            .remapped,
        1
    );
}

/// D8 order (lead review of #243, fix 2): a re-armed upload keeps its place, so it runs before a newer upload of the
/// same file, and the newer content lands last.
#[test]
fn a_rearmed_upload_runs_before_a_newer_upload_of_the_same_file() {
    let pc = Pc::new();
    pc.ghost_with_twin(1, "Photos");
    pc.upload("op-newer", "f1", Some("g1"), 500);
    pc.db.reconcile_held_queue(&opts(), NOW, &holds_a_file).unwrap();
    // Under `auth` until the key check (cross-lane review P2-2), it already holds its place: the newer one waits.
    assert!(matches!(
        pc.db.claim_operation("op-newer", NOW).unwrap(),
        ClaimOutcome::Wait
    ));
    assert_eq!(pc.db.resume_operations_paused_for_auth(NOW).unwrap(), 1);
    assert_eq!(pc.due(), vec!["op-f1".to_string(), "op-newer".to_string()]);
    assert!(matches!(
        pc.db.claim_operation("op-newer", NOW).unwrap(),
        ClaimOutcome::Wait
    ));
    let ClaimOutcome::Claimed(older) = pc.db.claim_operation("op-f1", NOW).unwrap() else {
        panic!("the re-armed upload is claimed first")
    };
    assert!(pc.db.finish_claimed("op-f1", &older.claim_id, None).unwrap());
    assert!(matches!(
        pc.db.claim_operation("op-newer", NOW).unwrap(),
        ClaimOutcome::Claimed(_)
    ));
}

/// A child written between two batches is re-pointed by its ghost's unit.
#[test]
fn a_child_added_between_batches_moves_with_its_ghost() {
    let pc = Pc::new();
    pc.ghost_with_twin(1, "A");
    pc.ghost_with_twin(2, "B");
    pc.db
            .reconcile_in_batches(&opts(), NOW, 1, &holds_a_file, &mut |db, batch| {
                if batch == 1 {
                    db.0.lock()
                        .unwrap()
                        .execute(
                            "INSERT INTO files (file_id, path, status, item_kind, current_version, parent_id, size_bytes, remote_updated_at)
                             VALUES ('late', 'B/late.txt', 'local', 'file', 0, 'g2', 0, 0)",
                            [],
                        )
                        .unwrap();
                }
            })
            .unwrap();
    assert_eq!(pc.parent_of_row("late").as_deref(), Some("t2"));
    assert_eq!(pc.references("g2"), 0);
}

/// D8 amendment (lead ruling 2026-10-29): a change held for the person that used its retries on the 404 its ghost
/// caused gets its budget back in the pass and keeps its pause; "Upload to my Beebeeb" then makes it due, and it runs.
#[test]
fn a_held_upload_exhausted_on_a_parent_404_under_a_remapped_ghost_uploads_after_upload() {
    let pc = Pc::new();
    pc.ghost_with_twin(1, "Photos");
    pc.pause("op-f1", "found_while_paused");
    let counts = pc.db.reconcile_held_queue(&opts(), NOW, &holds_a_file).unwrap();
    assert_eq!(
        counts,
        Counts {
            remapped: 1,
            rearmed_paused: 1,
            ..Counts::default()
        }
    );
    assert_eq!(
        pc.op("op-f1"),
        (
            Some("t1".to_string()),
            0,
            NOW,
            None,
            Some("found_while_paused".to_string())
        )
    );
    assert!(
        pc.due().is_empty(),
        "still held: the pass never releases a change held for the person"
    );
    assert_eq!(
        pc.db.release_found_while_paused(&["op-f1".to_string()], NOW).unwrap(),
        1
    );
    assert_eq!(pc.due(), vec!["op-f1".to_string()]);
    assert!(matches!(
        pc.db.claim_operation("op-f1", NOW).unwrap(),
        ClaimOutcome::Claimed(_)
    ));
}

/// D8 amendment: a held change that used its retries on another error keeps them; after Upload it stays stopped,
/// with its own error.
#[test]
fn a_held_upload_exhausted_on_another_error_stays_stopped_after_upload() {
    let pc = Pc::new();
    pc.ghost_with_twin(1, "Photos");
    pc.gave_up("op-f1", SERVER_ERROR);
    pc.pause("op-f1", "found_while_paused");
    let counts = pc.db.reconcile_held_queue(&opts(), NOW, &holds_a_file).unwrap();
    assert_eq!(
        counts,
        Counts {
            remapped: 1,
            ..Counts::default()
        }
    );
    assert_eq!(
        pc.op("op-f1"),
        (
            Some("t1".to_string()),
            25,
            0,
            Some(SERVER_ERROR.to_string()),
            Some("found_while_paused".to_string())
        )
    );
    assert_eq!(
        pc.db.release_found_while_paused(&["op-f1".to_string()], NOW).unwrap(),
        1
    );
    assert!(
        pc.due().is_empty(),
        "stopped after repeated errors: Upload only clears the pause"
    );
    assert_eq!(pc.op("op-f1").3.as_deref(), Some(SERVER_ERROR));
}

impl Pc {
    /// The op's sync-root-relative key, as every create queues it.
    fn queued_at(&self, op_id: &str, path: &str) {
        self.sql(
            "UPDATE operation_queue SET target_path = ?2 WHERE op_id = ?1",
            params![op_id, path],
        );
    }

    /// A new file `f` at `A/B/f.txt` whose upload was queued with no parent (D6).
    fn upload_without_a_parent(&self) {
        self.file("f", None, "A/B/f.txt", 0);
        self.upload("op-f", "f", None, 10);
        self.queued_at("op-f", "A/B/f.txt");
    }
}

/// D6: after the pass, the change and its row both name the folder found by path, and the change is due under it.
#[test]
fn reconcile_gives_a_root_level_upload_its_parent_by_path_in_both_columns() {
    let pc = Pc::new();
    pc.folder("a", None, "A", 3);
    pc.folder("b", Some("a"), "A/B", 3);
    pc.upload_without_a_parent();
    let counts = pc.db.reconcile_held_queue(&opts(), NOW, &holds_a_file).unwrap();
    assert_eq!(
        counts,
        Counts {
            parent_resolved: 1,
            ..Counts::default()
        }
    );
    assert_eq!(pc.op("op-f").0.as_deref(), Some("b"));
    assert_eq!(pc.parent_of_row("f").as_deref(), Some("b"));
    assert_eq!(pc.due(), vec!["op-f".to_string()]);
}

/// D6 then D2: the folder found by path is a ghost; its remap, in the same plan, points both columns at its twin.
/// The twin is found by parent and leaf (§3); its path here does not match, so the ghost is the only folder at the
/// directory. Split over two batches the result is the same.
#[test]
fn reconcile_points_both_columns_at_the_twin_when_the_parent_found_by_path_is_a_ghost() {
    let setup = |pc: &Pc| {
        pc.folder("a", None, "A", 3);
        pc.folder("g", Some("a"), "A/B", 0);
        pc.create_folder("op-g", "g", Some("a"));
        pc.folder("t", Some("a"), "Old/B", 3);
        pc.upload_without_a_parent();
    };
    let pc = Pc::new();
    setup(&pc);
    let mut batches = 0;
    let counts = pc
        .db
        .reconcile_in_batches(&opts(), NOW, RECONCILE_UNITS_PER_TX, &holds_a_file, &mut |_, n| {
            batches = n
        })
        .unwrap();
    assert_eq!(
        counts,
        Counts {
            parent_resolved: 1,
            remapped: 1,
            ..Counts::default()
        }
    );
    assert_eq!(batches, 1, "the resolve and the remap are in one plan");
    assert_eq!(pc.op("op-f").0.as_deref(), Some("t"));
    assert_eq!(pc.parent_of_row("f").as_deref(), Some("t"));
    assert_eq!(pc.references("g"), 0);

    let split = Pc::new();
    setup(&split);
    split
        .db
        .reconcile_in_batches(&opts(), NOW, 1, &holds_a_file, &mut |_, _| {})
        .unwrap();
    assert_eq!(split.snapshot(), pc.snapshot());
}

/// D10: a new file that meets a server file of the same name is held (`needs_review`, not due); once the server
/// file is gone, the next pass releases it to the key check (`auth`), and a key that opens its name resumes it.
#[test]
fn reconcile_holds_a_new_file_that_meets_a_server_file_and_releases_it_once_the_twin_is_gone() {
    let pc = Pc::new();
    pc.folder("p", None, "Docs", 3);
    pc.file("s", Some("p"), "Docs/report.txt", 2);
    pc.file("n", Some("p"), "Docs/Report.txt", 0);
    pc.upload("op-n", "n", Some("p"), 10);
    let counts = pc.db.reconcile_held_queue(&opts(), NOW, &holds_a_file).unwrap();
    assert_eq!(
        counts,
        Counts {
            file_twin: 1,
            ..Counts::default()
        }
    );
    assert_eq!(pc.op("op-n").4.as_deref(), Some(crate::held_queue::NEEDS_REVIEW));
    assert!(pc.due().is_empty());

    pc.sql("DELETE FROM files WHERE file_id = 's'", []);
    let counts = pc.db.reconcile_held_queue(&opts(), NOW, &holds_a_file).unwrap();
    assert_eq!(
        counts,
        Counts {
            released: 1,
            ..Counts::default()
        }
    );
    assert_eq!(pc.op("op-n").4.as_deref(), Some("auth"));
    assert!(pc.due().is_empty());
    assert_eq!(pc.db.resume_operations_paused_for_auth(NOW).unwrap(), 1);
    assert_eq!(pc.due(), vec!["op-n".to_string()]);
}

/// Goal 2, I6 (review finding on Task 3): the pass holds `auth` changes as `needs_review` (D4, D6, D10), out of
/// the key check's reach (D12 reads `auth` only). A later release must not make them due past that check: under a
/// replaced key they end up `key_replaced`, never sent with a name sealed under the old key.
#[test]
fn a_change_the_pass_held_is_never_released_past_the_key_check() {
    let pc = Pc::new();
    // D10: a new file that meets a server file of the same name.
    pc.folder("p", None, "Docs", 3);
    pc.file("s", Some("p"), "Docs/report.txt", 2);
    pc.file("n", Some("p"), "Docs/report.txt", 0);
    pc.upload("op-n", "n", Some("p"), 10);
    pc.pause("op-n", "auth");
    // D6: a new file queued with no parent, whose folder is not known yet.
    pc.upload_without_a_parent();
    pc.pause("op-f", "auth");
    // D4: a new folder with two twins (`≡`).
    pc.folder("t1", None, "Music", 3);
    pc.folder("t2", None, "music", 3);
    pc.folder("g", None, "Music", 0);
    pc.create_folder("op-g", "g", None);

    let counts = pc.db.reconcile_held_queue(&opts(), NOW, &holds_a_file).unwrap();
    assert_eq!(
        counts,
        Counts {
            held_ambiguous: 1,
            parent_unresolved: 1,
            file_twin: 1,
            ..Counts::default()
        }
    );
    // The key check fails: the vault key was replaced. The held changes are not `auth`, so it does not see them.
    pc.db.hold_operations_paused_for_auth_as_key_replaced(NOW).unwrap();

    // Every cause goes away.
    pc.sql("DELETE FROM files WHERE file_id IN ('s', 't1', 't2')", []);
    pc.folder("a", None, "A", 3);
    pc.folder("b", Some("a"), "A/B", 3);
    let counts = pc.db.reconcile_held_queue(&opts(), NOW, &holds_a_file).unwrap();
    assert_eq!(
        counts,
        Counts {
            replay: 1,
            released: 3,
            parent_resolved: 1,
            ..Counts::default()
        }
    );
    assert!(
        pc.due().is_empty(),
        "released into the key check, never straight to due: {:?}",
        pc.due()
    );
    // The key check after the pass fails again: all three are held as `key_replaced`.
    assert_eq!(pc.db.hold_operations_paused_for_auth_as_key_replaced(NOW).unwrap(), 3);
    for op_id in ["op-n", "op-f", "op-g"] {
        assert_eq!(pc.op(op_id).4.as_deref(), Some("key_replaced"), "{op_id}");
    }
    assert!(pc.due().is_empty());
}

/// D5: empty `Backups/unknown` scaffolding, under a `Backups` the pass folds into its twin, goes in the pass. No op
/// and no row is left under it, nothing names a dropped row (I1), and the pass returns its directories deepest first
/// for the caller to remove after the commit; the pass itself removes none. A second pass changes nothing (I3).
#[test]
fn the_pass_drops_empty_backups_unknown_scaffolding_and_returns_its_directories() {
    let pc = Pc::new();
    let sync_root = tempdir().unwrap();
    pc.folder("bt", None, "Backups", 3);
    pc.folder("bg", None, "Backups", 0);
    pc.folder("u", Some("bg"), "Backups/unknown", 0);
    pc.folder("d", Some("u"), "Backups/unknown/Documents", 0);
    pc.folder("o", Some("d"), "Backups/unknown/Documents/Old", 0);
    for (op_id, id, parent) in [
        ("op-bg", "bg", None),
        ("op-u", "u", Some("bg")),
        ("op-d", "d", Some("u")),
        ("op-o", "o", Some("d")),
    ] {
        pc.create_folder(op_id, id, parent);
    }
    std::fs::create_dir_all(sync_root.path().join("Backups/unknown/Documents/Old")).unwrap();
    let on_disk = |dir: &str| tree_holds_a_file(&sync_root.path().join(dir));
    let dry = pc.db.reconcile_dry_run(&opts(), &on_disk).unwrap();

    let counts = pc.db.reconcile_held_queue(&opts(), NOW, &on_disk);
    assert!(counts.is_ok(), "{counts:?}");
    let dirs = [
        "Backups/unknown/Documents/Old",
        "Backups/unknown/Documents",
        "Backups/unknown",
    ];
    let expected = Counts {
        remapped: 1,
        scaffold_dropped: 3,
        scaffold_dirs: dirs.iter().map(|dir| dir.to_string()).collect(),
        ..Counts::default()
    };
    assert_eq!(counts.unwrap(), expected);
    assert_eq!(dry, expected, "the dry run shows what the run does");
    assert_eq!(
        pc.count("SELECT COUNT(*) FROM files WHERE path LIKE 'Backups/unknown%'", []),
        0
    );
    assert_eq!(pc.count("SELECT COUNT(*) FROM operation_queue", []), 0);
    for id in ["bg", "u", "d", "o"] {
        assert_eq!(pc.references(id), 0, "{id}");
    }
    assert!(
        sync_root.path().join("Backups/unknown/Documents/Old").is_dir(),
        "the pass removes no directory"
    );
    assert_eq!(
        pc.db.reconcile_held_queue(&opts(), NOW + 60, &on_disk).unwrap(),
        Counts::default()
    );
}

#[cfg(not(target_os = "macos"))]
fn unpause(pc: &Pc, op_id: &str) {
    pc.sql(
        "UPDATE operation_queue SET paused_reason = NULL WHERE op_id = ?1",
        params![op_id],
    );
}

/// A folder made here, not on the server yet, whose create is due, and a new file in it whose upload is due too.
#[cfg(not(target_os = "macos"))]
fn new_folder_with_a_new_file(pc: &Pc) {
    pc.folder("new", None, "New", 0);
    pc.create_folder("op-new", "new", None);
    unpause(pc, "op-new");
    pc.file("f", Some("new"), "New/f.txt", 0);
    pc.upload("op-f", "f", Some("new"), 20);
}

/// D7, I5 (2226's verification line: "not retried"): a change under a folder whose create has not landed is not
/// due, its claim claims nothing, and it keeps its attempts and its retry time.
#[cfg(not(target_os = "macos"))]
#[test]
fn an_upload_under_a_folder_not_yet_on_the_server_is_not_due_and_keeps_its_attempts() {
    let pc = Pc::new();
    new_folder_with_a_new_file(&pc);
    assert_eq!(pc.due(), vec!["op-new".to_string()], "only the folder's create is due");
    assert!(
        matches!(pc.db.claim_operation("op-f", NOW).unwrap(), ClaimOutcome::Gone),
        "the change is not claimed"
    );
    assert_eq!(
        pc.op("op-f"),
        (Some("new".to_string()), 0, 0, None, None),
        "attempts and retry time untouched"
    );
    assert_eq!(
        pc.count(
            "SELECT COUNT(*) FROM operation_queue WHERE op_id = 'op-f' AND claim_id IS NULL",
            []
        ),
        1
    );
}

/// D7: once the folder's create lands (its op is gone), the change under it is due and is claimed.
#[cfg(not(target_os = "macos"))]
#[test]
fn it_becomes_due_once_its_folder_lands() {
    let pc = Pc::new();
    new_folder_with_a_new_file(&pc);
    assert!(
        !pc.due().contains(&"op-f".to_string()),
        "not due while its folder's create is queued"
    );
    let ClaimOutcome::Claimed(folder) = pc.db.claim_operation("op-new", NOW).unwrap() else {
        panic!("the folder's create is claimed")
    };
    assert!(pc.db.finish_claimed("op-new", &folder.claim_id, None).unwrap());
    assert_eq!(pc.due(), vec!["op-f".to_string()]);
    assert!(matches!(
        pc.db.claim_operation("op-f", NOW).unwrap(),
        ClaimOutcome::Claimed(_)
    ));
}

/// D7, D13: a change under a folder whose create is held (paused, or out of attempts) is counted as waiting for its
/// folder, not as waiting; under a folder whose create can still run it is waiting.
#[cfg(not(target_os = "macos"))]
#[test]
fn the_hold_summary_counts_children_of_a_held_folder_as_waiting_for_their_folder() {
    let pc = Pc::new();
    new_folder_with_a_new_file(&pc);
    let for_folder = HeldGroup {
        reason: "waiting_for_folder".to_string(),
        count: 1,
        oldest_at: 20,
        newest_at: 20,
    };

    pc.pause("op-new", "auth");
    let hold = pc.db.queue_hold_summary().unwrap();
    assert_eq!((hold.waiting, hold.gave_up), (0, 0));
    assert_eq!(
        hold.held,
        vec![
            HeldGroup {
                reason: "auth".to_string(),
                count: 1,
                oldest_at: 1,
                newest_at: 1
            },
            for_folder.clone()
        ]
    );

    unpause(&pc, "op-new");
    pc.gave_up("op-new", SERVER_ERROR);
    let hold = pc.db.queue_hold_summary().unwrap();
    assert_eq!((hold.waiting, hold.gave_up), (0, 1));
    assert_eq!(hold.held, vec![for_folder]);

    pc.sql("UPDATE operation_queue SET attempts = 0 WHERE op_id = 'op-new'", []);
    let hold = pc.db.queue_hold_summary().unwrap();
    assert_eq!(
        (hold.waiting, hold.gave_up),
        (2, 0),
        "the folder's create can run: both are waiting"
    );
    assert!(hold.held.is_empty(), "no group: {:?}", hold.held);
}

/// D7 below one level: a folder made here inside a held folder waits for it, and so does a change inside that
/// folder. Counted as waiting, it would show the queue busy for as long as the hold lasts.
#[cfg(not(target_os = "macos"))]
#[test]
fn a_change_in_a_folder_inside_a_held_folder_also_waits_for_its_folder() {
    let pc = Pc::new();
    pc.folder("new", None, "New", 0);
    pc.create_folder("op-new", "new", None);
    pc.folder("sub", Some("new"), "New/Sub", 0);
    pc.create_folder("op-sub", "sub", Some("new"));
    unpause(&pc, "op-sub");
    pc.file("f", Some("sub"), "New/Sub/f.txt", 0);
    pc.upload("op-f", "f", Some("sub"), 20);
    assert!(pc.due().is_empty(), "nothing runs before its folder");
    let hold = pc.db.queue_hold_summary().unwrap();
    assert_eq!((hold.waiting, hold.gave_up), (0, 0));
    assert_eq!(
        hold.held,
        vec![
            HeldGroup {
                reason: "auth".to_string(),
                count: 1,
                oldest_at: 1,
                newest_at: 1
            },
            HeldGroup {
                reason: "waiting_for_folder".to_string(),
                count: 2,
                oldest_at: 1,
                newest_at: 20
            },
        ]
    );
}

/// Task 2243's budget holds with the D7 group in it: 300 held folders and 2,700 changes waiting in them are
/// counted from the indexes, without reading the changes themselves (about 250 pages here).
///
/// Task 2409: both indexes carry `kind` now, so the downloads leave the count (`UNSENT_KINDS_SQL`) without a row
/// read; that took this read from 67 pages to 91, and the budget from 80 to 100. Reading the changes is 320.
#[cfg(not(target_os = "macos"))]
#[test]
fn the_waiting_for_folder_group_reads_indexes_not_the_changes() {
    let pc = Pc::new();
    {
        let mut conn = pc.db.0.lock().unwrap();
        let tx = conn.transaction().unwrap();
        let id = |n: i64| format!("0a000000-0000-4000-8000-{n:012}");
        for folder in 0..300_i64 {
            tx.execute(
                "INSERT INTO operation_queue (op_id, kind, file_id, target_path, paused_reason, created_at)
                     VALUES (?1, 'create_folder', ?2, ?3, 'auth', 1)",
                params![id(folder), id(folder), format!("Projects/Folder {folder:05}")],
            )
            .unwrap();
            for n in 0..9_i64 {
                let op = 1_000 + folder * 9 + n;
                tx.execute(
                        "INSERT INTO operation_queue (op_id, kind, file_id, parent_id, target_path, metadata_json, created_at)
                         VALUES (?1, 'upload_version', ?2, ?3, ?4, ?5, ?6)",
                        params![
                            id(op),
                            id(op),
                            id(folder),
                            format!("Projects/Folder {folder:05}/draft-{n}.txt"),
                            format!("{{\"operation\":\"create_file\",\"name_ct\":\"{}\"}}", "x".repeat(120)),
                            10 + op,
                        ],
                    )
                    .unwrap();
            }
        }
        tx.commit().unwrap();
    }
    let db = StateDb::open(pc.dir.path().join("state.db")).unwrap();
    let hold = db.queue_hold_summary().unwrap();
    let reads = db.page_reads();
    assert_eq!(hold.waiting, 0);
    assert_eq!(
        hold.held,
        vec![
            HeldGroup {
                reason: "auth".to_string(),
                count: 300,
                oldest_at: 1,
                newest_at: 1
            },
            HeldGroup {
                reason: "waiting_for_folder".to_string(),
                count: 2_700,
                oldest_at: 1_010,
                newest_at: 3_709
            },
        ]
    );
    assert!(
        reads <= 100,
        "the hold summary (the open and its reads) read {reads} pages from disk"
    );
}

/// D7 and the tray's activity: a change waiting for a held folder is not transfer work, or the engine would show
/// "syncing" for as long as the hold lasts. Under a folder whose create can still run it is.
#[cfg(not(target_os = "macos"))]
#[test]
fn a_change_waiting_for_a_held_folder_is_not_transfer_work() {
    let pc = Pc::new();
    new_folder_with_a_new_file(&pc);
    pc.pause("op-new", "auth");
    assert_eq!(pc.db.transfer_backlog(NOW).unwrap().upload_files, 0);
    unpause(&pc, "op-new");
    assert_eq!(
        pc.db.transfer_backlog(NOW).unwrap().upload_files,
        1,
        "it is sent once its folder lands"
    );
}

/// D7: the diagnostics' `due` is what the queue would send now, gate included.
#[cfg(not(target_os = "macos"))]
#[test]
fn the_diagnostics_due_count_is_what_the_queue_would_send() {
    let pc = Pc::new();
    new_folder_with_a_new_file(&pc);
    assert_eq!(pc.due().len(), 1);
    assert_eq!(pc.db.queue_diagnostics(NOW).unwrap().due, 1);
}
