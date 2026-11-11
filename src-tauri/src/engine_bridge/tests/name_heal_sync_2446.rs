//! Task 2446, the heal, after the review of PR 289 (P1): a healing snapshot that keeps failing must not stop sync.
//! Each test runs whole ticks through `sync_tick_outcome_at` at a second the test chooses, then does what the runner
//! does with the answer (`runner.rs`: the queue step runs after an `Ok` tick, never after an `Err` one).
//!
//! The blocked state is made by the real path (a malformed envelope through `prepare_owned_rows`), as in
//! `name_heal_2446`.

use super::snapshot_names_2446::prod_row;
use super::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

const MK: [u8; 32] = [7u8; 32];
const FOLDER: &str = "f0000000-0000-4000-8000-00000002446a";
const FILE: &str = "a0000000-0000-4000-8000-00000002446a";
const OTHER: &str = "a0000000-0000-4000-8000-00000002446b";
const SUB: &str = "f0000000-0000-4000-8000-00000002446c";
const INSIDE: &str = "a0000000-0000-4000-8000-00000002446d";
const DOOMED: &str = "a0000000-0000-4000-8000-00000002446e";
/// The runner's tick interval, in seconds.
const TICK: i64 = 30;
/// The first tick of every test.
const T0: i64 = 1_000_000;

thread_local! {
    /// Rows whose write fails in `apply_snapshot_leaving` (these tests run on the thread that set it).
    static FAILING_ROWS: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
}

/// The fault `apply_snapshot_leaving` reads in a test build: the write of a row named in `FAILING_ROWS` fails.
pub(in crate::engine_bridge) fn snapshot_row_fault(file_id: &str) -> Option<EngineError> {
    FAILING_ROWS
        .with(|rows| rows.borrow().contains(file_id))
        .then(|| anyhow::anyhow!("test: this row is not stored").into())
}

fn fail_rows(ids: &[&str]) {
    FAILING_ROWS.with(|rows| *rows.borrow_mut() = ids.iter().map(|id| id.to_string()).collect());
}

/// `/sync/snapshot` answers from `snapshots` in order (then 500); `/sync/ops` answers "nothing new" at `head`;
/// `GET /files/<id>` (a row's own read) answers from `own_reads` (404 when not listed); `DELETE /files/<id>` (a
/// queued trash, the queue step's request) answers 200. Counts each kind.
struct Server {
    base_url: String,
    snapshots_asked: Arc<AtomicUsize>,
    ops_asked: Arc<AtomicUsize>,
    queue_sent: Arc<AtomicUsize>,
    own_reads: Arc<Mutex<HashMap<String, (&'static str, serde_json::Value)>>>,
}

impl Server {
    fn start(head: i64, snapshots: Vec<(&'static str, serde_json::Value)>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let server = Server {
            base_url: format!("http://{}", listener.local_addr().unwrap()),
            snapshots_asked: Arc::new(AtomicUsize::new(0)),
            ops_asked: Arc::new(AtomicUsize::new(0)),
            queue_sent: Arc::new(AtomicUsize::new(0)),
            own_reads: Arc::new(Mutex::new(HashMap::new())),
        };
        let (snapshots_asked, ops_asked, queue_sent, own_reads) = (
            server.snapshots_asked.clone(),
            server.ops_asked.clone(),
            server.queue_sent.clone(),
            server.own_reads.clone(),
        );
        thread::spawn(move || {
            let mut snapshots = snapshots.into_iter();
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let request = read_http_request(&mut stream);
                let response = if request.path.starts_with("/api/v1/sync/snapshot") {
                    snapshots_asked.fetch_add(1, AtomicOrdering::SeqCst);
                    let (status, body) = snapshots
                        .next()
                        .unwrap_or(("500 Internal Server Error", serde_json::json!({})));
                    http_json(status, body)
                } else if request.path.starts_with("/api/v1/sync/ops") {
                    ops_asked.fetch_add(1, AtomicOrdering::SeqCst);
                    http_json(
                        "200 OK",
                        serde_json::json!({"since": head, "head_seq": head, "ops": []}),
                    )
                } else if request.method == "DELETE" {
                    queue_sent.fetch_add(1, AtomicOrdering::SeqCst);
                    http_json("200 OK", serde_json::json!({}))
                } else if let Some((status, body)) = request
                    .path
                    .rsplit('/')
                    .next()
                    .and_then(|id| own_reads.lock().unwrap().get(id).cloned())
                {
                    http_json(status, body)
                } else {
                    http_json("404 Not Found", serde_json::json!({}))
                };
                let _ = stream.write_all(response.as_bytes());
            }
        });
        server
    }

    fn snapshots(&self) -> usize {
        self.snapshots_asked.load(AtomicOrdering::SeqCst)
    }
    fn ops(&self) -> usize {
        self.ops_asked.load(AtomicOrdering::SeqCst)
    }
    fn queue(&self) -> usize {
        self.queue_sent.load(AtomicOrdering::SeqCst)
    }
    fn own_read(&self, id: &str, status: &'static str, body: serde_json::Value) {
        self.own_reads.lock().unwrap().insert(id.to_string(), (status, body));
    }
}

fn healthy_rows() -> Vec<serde_json::Value> {
    vec![
        prod_row(FOLDER, None, "Docs", true),
        prod_row(FILE, Some(FOLDER), "Report.pdf", false),
        prod_row(OTHER, None, "Notes.txt", false),
    ]
}

/// `rows` as a snapshot stored them while they were blocked (one request field set and two null: a malformed
/// envelope, which blocks the row exactly as the null fields did before the fix).
async fn store_blocked(bridge: &EngineBridge, root: &Path, mut rows: Vec<serde_json::Value>) {
    for row in &mut rows {
        row["file_request_id"] = serde_json::json!("0f000000-0000-4000-8000-0000000000e2");
    }
    let prepared = bridge.prepare_owned_rows(&rows).await.unwrap();
    assert!(prepared.iter().all(|row| !row.readable), "every row is blocked");
    let snapshot = crate::api_client::SyncSnapshot {
        seq_id: 5,
        nodes: prepared.into_iter().map(|p| p.row).collect(),
    };
    apply_snapshot(bridge, root, &snapshot, 100, 100, &mut Vec::new()).unwrap();
    assert!(bridge.db().has_blocked_or_nameless_rows().unwrap());
    assert!(!bridge.db().name_heal_done().unwrap());
}

/// A change waiting in the queue: a file the person deleted, whose trash the queue step sends.
fn queue_a_change(bridge: &EngineBridge, op_id: &str, file_id: &str) {
    seed_bridge_entry(
        bridge,
        file_id,
        &format!("{op_id}.txt"),
        None,
        FileStatus::Trashing,
        false,
        0,
    );
    bridge
        .db()
        .enqueue_operation(&PendingOperation {
            op_id: op_id.into(),
            kind: OperationKind::TrashFile,
            file_id: Some(file_id.into()),
            parent_id: None,
            target_path: None,
            metadata_json: Some(serde_json::json!({ "operation": "trash" }).to_string()),
            payload_path: None,
            base_version: None,
            base_object_version_id: None,
            attempts: 0,
            max_attempts: 25,
            next_retry_at: 0,
            last_error: None,
            backup_source_key: None,
            created_at: 100,
            updated_at: 100,
        })
        .unwrap();
}

/// One tick as the runner runs it: the tick body, and the queue step only when the tick answered `Ok`.
async fn runner_tick(bridge: &EngineBridge, root: &Path, now: i64) -> bool {
    match sync_tick_outcome_at(bridge, root, now).await {
        Ok(_) => {
            bridge.process_due_operations(root, now).await.unwrap();
            true
        }
        Err(_) => false,
    }
}

/// The P1: with blocked rows and a snapshot that fails every time, every tick still pulls the ops and runs the
/// queue, the heal stays owed, and the snapshot is asked for after a wait that doubles up to an hour: 7 times in two
/// hours of ticks, not 240.
#[tokio::test]
async fn sync_goes_on_while_the_healing_snapshot_keeps_failing() {
    let dir = tempfile::tempdir().unwrap();
    let server = Server::start(5, Vec::new());
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), MK);
    store_blocked(&bridge, dir.path(), healthy_rows()).await;
    queue_a_change(&bridge, "op-first", DOOMED);

    assert!(
        runner_tick(&bridge, dir.path(), T0).await,
        "the tick whose healing snapshot failed still answers Ok"
    );
    assert_eq!(server.snapshots(), 1, "the heal asked for its snapshot");
    assert_eq!(server.ops(), 1, "the ops pull ran on the same tick");
    assert_eq!(
        server.queue(),
        1,
        "the queue step sent the waiting change on the same tick"
    );
    assert!(!bridge.db().name_heal_done().unwrap(), "the heal is still owed");
    assert_eq!(
        bridge.db().get_sync_cursor().unwrap(),
        Some(5),
        "the cursor did not move"
    );

    // Two hours of ticks, 30 s apart. A change queued halfway goes out on its own tick.
    let ticks = 240;
    for n in 1..ticks {
        if n == 120 {
            queue_a_change(&bridge, "op-later", "a0000000-0000-4000-8000-00000002446f");
        }
        assert!(
            runner_tick(&bridge, dir.path(), T0 + n * TICK).await,
            "tick {n} answers Ok"
        );
        if n == 120 {
            assert_eq!(server.queue(), 2, "a change queued while the heal fails is sent");
        }
    }
    assert_eq!(server.ops(), ticks as usize, "every tick pulled the ops");
    // Asked at +0, +60, +180, +420, +900, +1860 and +3780 s; the next is due at +7380 s, after these two hours.
    assert_eq!(
        server.snapshots(),
        7,
        "the snapshot is asked for after a doubling wait, not on every tick"
    );
    let retry = bridge.db().name_heal_retry().unwrap().unwrap();
    assert_eq!((retry.failures, retry.next_at, retry.rows_left), (7, T0 + 7380, 0));
    assert!(!bridge.db().name_heal_done().unwrap());
    assert_eq!(
        bridge.db().get_file(FOLDER).unwrap().unwrap().path,
        crate::owner_keys::blocked_name(FOLDER),
        "nothing was named"
    );
}

/// A pass that fails as a whole keeps the count of rows an earlier pass left, so the heal stays owed for them.
#[test]
fn a_whole_failure_keeps_the_rows_an_earlier_pass_left() {
    let dir = tempfile::tempdir().unwrap();
    let bridge = test_bridge(&dir.path().join("state.db"));
    let db = bridge.db();
    assert_eq!(db.name_heal_retry().unwrap(), None);
    let first = db.note_name_heal_failed(T0, Some(2)).unwrap();
    assert_eq!((first.failures, first.next_at, first.rows_left), (1, T0 + 60, 2));
    let second = db.note_name_heal_failed(T0 + 60, None).unwrap();
    assert_eq!((second.failures, second.next_at, second.rows_left), (2, T0 + 180, 2));
    assert_eq!(db.name_heal_retry().unwrap(), Some(second), "read back as written");
    assert!(!name_heal_is_due(db, T0 + 61).unwrap(), "owed, and waiting");
    assert!(
        !db.name_heal_done().unwrap(),
        "rows are left: not done, though no record is blocked"
    );
    assert!(name_heal_is_due(db, T0 + 180).unwrap());
}

/// A snapshot that succeeds after failures completes the heal once, and clears the wait.
#[tokio::test]
async fn a_good_snapshot_after_failures_completes_the_heal_once() {
    let dir = tempfile::tempdir().unwrap();
    let server = Server::start(
        6,
        vec![
            ("500 Internal Server Error", serde_json::json!({})),
            ("500 Internal Server Error", serde_json::json!({})),
            ("200 OK", serde_json::json!({"seq_id": 6, "nodes": healthy_rows()})),
        ],
    );
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), MK);
    store_blocked(&bridge, dir.path(), healthy_rows()).await;

    // Failures at +0 and +60; the third snapshot, at +180, is good.
    let asked_after: [(i64, usize); 8] = [
        (0, 1),
        (30, 1),
        (60, 2),
        (90, 2),
        (150, 2),
        (180, 3),
        (210, 3),
        (7200, 3),
    ];
    for (at, asked) in asked_after {
        assert!(runner_tick(&bridge, dir.path(), T0 + at).await);
        assert_eq!(server.snapshots(), asked, "snapshots asked for by +{at} s");
    }
    assert_eq!(bridge.db().get_file(FOLDER).unwrap().unwrap().path, "Docs");
    assert_eq!(bridge.db().get_file(FILE).unwrap().unwrap().path, "Docs/Report.pdf");
    assert!(bridge.db().name_heal_done().unwrap());
    assert_eq!(
        bridge.db().name_heal_retry().unwrap(),
        None,
        "the wait goes with the heal"
    );
    assert_eq!(bridge.db().get_sync_cursor().unwrap(), Some(6));
}

/// The wait is in the state database: an engine started again inside it asks for nothing.
#[tokio::test]
async fn a_restart_keeps_the_wait() {
    let dir = tempfile::tempdir().unwrap();
    let server = Server::start(5, Vec::new());
    let db_path = dir.path().join("state.db");
    {
        let bridge = test_bridge_with_api(&db_path, server.base_url.clone(), MK);
        store_blocked(&bridge, dir.path(), healthy_rows()).await;
        for at in [0, 60] {
            assert!(runner_tick(&bridge, dir.path(), T0 + at).await);
        }
        assert_eq!(server.snapshots(), 2);
    }
    let restarted = test_bridge_with_api(&db_path, server.base_url.clone(), MK);
    // The third is due at +180.
    assert!(runner_tick(&restarted, dir.path(), T0 + 90).await);
    assert!(runner_tick(&restarted, dir.path(), T0 + 150).await);
    assert_eq!(server.snapshots(), 2, "a restart inside the wait asks for no snapshot");
    assert!(runner_tick(&restarted, dir.path(), T0 + 180).await);
    assert_eq!(server.snapshots(), 3, "and asks when the wait is over");
}

/// A snapshot something else asked for too (gap recovery) keeps the rule it had: off macOS its failure ends that one
/// tick, and the request is gone with it. The heal's own wait still starts.
#[cfg(not(target_os = "macos"))]
#[tokio::test]
async fn a_failed_snapshot_gap_recovery_asked_for_still_ends_its_tick() {
    let dir = tempfile::tempdir().unwrap();
    let server = Server::start(5, Vec::new());
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), MK);
    store_blocked(&bridge, dir.path(), healthy_rows()).await;
    bridge.db().request_resnapshot().unwrap();

    assert!(
        !runner_tick(&bridge, dir.path(), T0).await,
        "as before the heal existed"
    );
    assert_eq!((server.snapshots(), server.ops()), (1, 0));
    assert_eq!(bridge.db().name_heal_retry().unwrap().unwrap().failures, 1);
    assert!(
        runner_tick(&bridge, dir.path(), T0 + TICK).await,
        "one tick, not every tick"
    );
    assert_eq!((server.snapshots(), server.ops()), (1, 1));
}

/// A row whose own read the server answers with 503 still fails the whole healing pass (the read lives in
/// `prepare_owned_rows`, which this fix does not change): nothing is applied, nothing is pruned, the tick goes on,
/// and the pass is taken again after the wait.
#[tokio::test]
async fn a_busy_own_read_fails_the_pass_and_sync_goes_on() {
    let dir = tempfile::tempdir().unwrap();
    let mut rows = healthy_rows();
    // Listed without its name: the row's own read is asked for.
    rows[2].as_object_mut().unwrap().remove("name_encrypted");
    let server = Server::start(
        7,
        vec![
            ("200 OK", serde_json::json!({"seq_id": 6, "nodes": rows.clone()})),
            ("200 OK", serde_json::json!({"seq_id": 7, "nodes": rows})),
        ],
    );
    server.own_read(OTHER, "503 Service Unavailable", serde_json::json!({}));
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), MK);
    store_blocked(&bridge, dir.path(), healthy_rows()).await;
    let before = bridge.db().get_file(OTHER).unwrap().unwrap().path;

    assert!(runner_tick(&bridge, dir.path(), T0).await, "the tick goes on");
    assert_eq!(server.ops(), 1);
    assert_eq!(
        bridge.db().get_file(FOLDER).unwrap().unwrap().path,
        crate::owner_keys::blocked_name(FOLDER),
        "nothing of the pass was applied"
    );
    assert_eq!(bridge.db().get_file(OTHER).unwrap().expect("not pruned").path, before);
    let retry = bridge.db().name_heal_retry().unwrap().unwrap();
    // The pass had begun (the snapshot was read), so it counts as unfinished: at least one row left.
    assert_eq!((retry.failures, retry.next_at, retry.rows_left), (1, T0 + 60, 1));
    assert_eq!(bridge.db().get_sync_cursor().unwrap(), Some(5));

    server.own_read(OTHER, "200 OK", prod_row(OTHER, None, "Notes.txt", false));
    assert!(runner_tick(&bridge, dir.path(), T0 + 60).await);
    assert_eq!(bridge.db().get_file(OTHER).unwrap().unwrap().path, "Notes.txt");
    assert_eq!(bridge.db().get_file(FOLDER).unwrap().unwrap().path, "Docs");
    assert!(bridge.db().name_heal_done().unwrap());
}

/// A row whose write fails is left with everything under it: the folder, the file in it and a row new to this
/// device are not placed anywhere else, none is pruned, and the heal stays owed even once no blocked record is left.
#[tokio::test]
async fn a_row_that_cannot_be_stored_is_left_with_the_rows_under_it() {
    let dir = tempfile::tempdir().unwrap();
    let stored = || {
        let mut rows = healthy_rows();
        rows.push(prod_row(SUB, Some(FOLDER), "Plans", true));
        rows
    };
    let listed = || {
        let mut rows = stored();
        // New to this device, under the folder whose write fails.
        rows.push(prod_row(INSIDE, Some(SUB), "Q4.txt", false));
        rows
    };
    let server = Server::start(
        7,
        vec![
            ("200 OK", serde_json::json!({"seq_id": 6, "nodes": listed()})),
            ("200 OK", serde_json::json!({"seq_id": 7, "nodes": listed()})),
        ],
    );
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), MK);
    store_blocked(&bridge, dir.path(), stored()).await;
    let sub_before = bridge.db().get_file(SUB).unwrap().unwrap().path;

    fail_rows(&[SUB]);
    assert!(runner_tick(&bridge, dir.path(), T0).await);
    assert_eq!(bridge.db().get_file(FILE).unwrap().unwrap().path, "Docs/Report.pdf");
    assert_eq!(
        bridge.db().get_file(SUB).unwrap().expect("left, not pruned").path,
        sub_before,
        "the folder is left as it was"
    );
    assert!(
        bridge.db().get_file(INSIDE).unwrap().is_none(),
        "a new row under it is not placed (never at the top level)"
    );
    let retry = bridge.db().name_heal_retry().unwrap().unwrap();
    assert_eq!(retry.rows_left, 2, "the folder and the row under it");
    assert!(!bridge.db().name_heal_done().unwrap());

    // The key records are no longer blocked (the pass read every row); the heal is owed for the rows left.
    assert!(!bridge.db().has_blocked_or_nameless_rows().unwrap());
    fail_rows(&[]);
    assert!(runner_tick(&bridge, dir.path(), T0 + TICK).await);
    assert!(!bridge.db().name_heal_done().unwrap(), "still owed inside the wait");
    assert!(runner_tick(&bridge, dir.path(), T0 + 60).await);
    assert_eq!(bridge.db().get_file(SUB).unwrap().unwrap().path, "Docs/Plans");
    assert_eq!(bridge.db().get_file(INSIDE).unwrap().unwrap().path, "Docs/Plans/Q4.txt");
    assert!(bridge.db().name_heal_done().unwrap());
    assert_eq!(server.snapshots(), 2);
}

/// Review of PR 289 (P2-3): a row at its blocked label that a change of this PC is still queued or parked for is
/// not moved by the heal. Its path, size, times and version stay, and so does the change with the base it was
/// queued on; the heal stays owed and names the row once the change is gone. A row in conflict waits the same way.
#[tokio::test]
async fn a_row_with_a_parked_change_keeps_its_place_until_the_change_is_gone() {
    let dir = tempfile::tempdir().unwrap();
    let mut listed = healthy_rows();
    for row in &mut listed {
        row["version_number"] = serde_json::json!(5);
        row["size_bytes"] = serde_json::json!(if row["is_folder"] == true { 0 } else { 9000 });
    }
    let server = Server::start(
        7,
        vec![
            ("200 OK", serde_json::json!({"seq_id": 6, "nodes": listed.clone()})),
            ("200 OK", serde_json::json!({"seq_id": 7, "nodes": listed})),
        ],
    );
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), MK);
    store_blocked(&bridge, dir.path(), healthy_rows()).await;
    let db = bridge.db();

    // OTHER: an edit made while the file showed under its label failed and is parked (the row is `Error`).
    let other_label = crate::owner_keys::blocked_name(OTHER);
    seed_bridge_entry(&bridge, OTHER, &other_label, None, FileStatus::Error, false, 50);
    db.enqueue_operation(&PendingOperation {
        op_id: "op-parked".into(),
        kind: OperationKind::UploadVersion,
        file_id: Some(OTHER.into()),
        parent_id: None,
        target_path: Some(other_label.clone()),
        metadata_json: None,
        payload_path: None,
        base_version: Some(3),
        base_object_version_id: None,
        attempts: 25,
        max_attempts: 25,
        next_retry_at: T0 + 10_000_000,
        last_error: Some("stale".into()),
        backup_source_key: None,
        created_at: 100,
        updated_at: 100,
    })
    .unwrap();
    // FILE: in conflict at its label.
    let file_before = db.get_file(FILE).unwrap().unwrap();
    seed_bridge_entry(
        &bridge,
        FILE,
        &file_before.path,
        Some(FOLDER),
        FileStatus::Conflict,
        false,
        50,
    );
    let other_before = db.get_file(OTHER).unwrap().unwrap();
    let contract_before = db.get_file_contract_state(OTHER).unwrap().unwrap();
    // macOS only (spec §6.3.2, `apply_snapshot_version`, since before the heal): EVERY snapshot raises a row's
    // version to the node's, and sets its size unless a Finder write is queued, in any status but `Trashing` and
    // `Conflict`; `local_base_version`, the stale marker a parked write is decided against, is never written there.
    // That rule runs before the heal's guard and is not the heal's: the test states it instead of hiding it.
    let (size_after, version_after) = if cfg!(target_os = "macos") {
        (9000, 5)
    } else {
        (other_before.size_bytes, contract_before.current_version)
    };

    assert!(runner_tick(&bridge, dir.path(), T0).await);
    assert_eq!(db.get_file(FOLDER).unwrap().unwrap().path, "Docs", "the rest is named");
    let other = db.get_file(OTHER).unwrap().unwrap();
    assert_eq!(
        (
            other.path.as_str(),
            &other.status,
            other.size_bytes,
            other.modified_at,
            other.remote_updated_at
        ),
        (
            other_label.as_str(),
            &FileStatus::Error,
            size_after,
            other_before.modified_at,
            other_before.remote_updated_at
        ),
        "the row with a parked change is not touched"
    );
    let contract = db.get_file_contract_state(OTHER).unwrap().unwrap();
    assert_eq!(
        contract.current_version, version_after,
        "its version as before (macOS: raised, see above)"
    );
    assert_eq!(
        contract.local_base_version, contract_before.local_base_version,
        "the base its content was agreed on is never touched, on any OS"
    );
    let op = db
        .get_operation("op-parked")
        .unwrap()
        .expect("the change is still queued");
    assert_eq!(
        (op.base_version, op.target_path.as_deref()),
        (Some(3), Some(other_label.as_str()))
    );
    assert!(
        db.get_file(FILE)
            .unwrap()
            .unwrap()
            .path
            .ends_with(&crate::owner_keys::blocked_name(FILE)),
        "the row in conflict keeps its label too"
    );
    assert!(!db.name_heal_done().unwrap(), "owed while a row waits for its change");
    assert_eq!(
        db.name_heal_retry().unwrap().unwrap().rows_left,
        2,
        "the parked row and the one in conflict"
    );

    // Both are settled (the change sent or dismissed, the conflict resolved): the next pass names them.
    db.remove_operation("op-parked").unwrap();
    db.set_status(OTHER, FileStatus::Local).unwrap();
    db.set_status(FILE, FileStatus::Local).unwrap();
    assert!(runner_tick(&bridge, dir.path(), T0 + 60).await);
    assert_eq!(db.get_file(OTHER).unwrap().unwrap().path, "Notes.txt");
    assert_eq!(db.get_file(FILE).unwrap().unwrap().path, "Docs/Report.pdf");
    assert!(db.name_heal_done().unwrap());
}

/// A rightly blocked row is not "waiting": it keeps its label and the heal still completes (as before).
#[tokio::test]
async fn a_rightly_blocked_row_does_not_keep_the_heal_owed() {
    let dir = tempfile::tempdir().unwrap();
    let mut rows = healthy_rows();
    rows[1]["file_request_id"] = serde_json::json!("0f000000-0000-4000-8000-0000000000e2");
    let server = Server::start(6, vec![("200 OK", serde_json::json!({"seq_id": 6, "nodes": rows}))]);
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), MK);
    store_blocked(&bridge, dir.path(), healthy_rows()).await;

    assert!(runner_tick(&bridge, dir.path(), T0).await);
    assert!(bridge.db().name_heal_done().unwrap());
    assert_eq!(
        bridge.db().get_file(FILE).unwrap().unwrap().path,
        format!("Docs/{}", crate::owner_keys::blocked_name(FILE))
    );
}

/// A row being trashed is never named again and is not waited for. A row with a download queued is held like any
/// other queued op (the op names the place it writes to), and a folder that waits keeps the rows under it with it.
#[tokio::test]
async fn a_row_being_trashed_is_not_waited_for_and_a_waiting_folder_keeps_its_rows() {
    let dir = tempfile::tempdir().unwrap();
    let listed = || serde_json::json!({"seq_id": 6, "nodes": healthy_rows()});
    let server = Server::start(
        6,
        vec![("200 OK", listed()), ("200 OK", listed()), ("200 OK", listed())],
    );
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), MK);
    store_blocked(&bridge, dir.path(), healthy_rows()).await;
    let db = bridge.db();
    let other_label = crate::owner_keys::blocked_name(OTHER);
    seed_bridge_entry(&bridge, OTHER, &other_label, None, FileStatus::Trashing, false, 50);
    // The folder has a failed change parked; the file in it is on this PC (`Local`).
    let folder_label = crate::owner_keys::blocked_name(FOLDER);
    seed_bridge_entry(&bridge, FOLDER, &folder_label, None, FileStatus::Error, true, 50);
    enqueue_test_operation(&bridge, "op-folder", OperationKind::HydrateFile, FOLDER, 100);
    let file_before = db.get_file(FILE).unwrap().unwrap().path;
    seed_bridge_entry(&bridge, FILE, &file_before, Some(FOLDER), FileStatus::Local, false, 50);

    sync_tick_outcome_at(&bridge, dir.path(), T0).await.unwrap();
    assert_eq!(
        db.get_file(FOLDER).unwrap().unwrap().path,
        folder_label,
        "held with its op queued"
    );
    assert_eq!(
        db.get_file(FILE).unwrap().unwrap().path,
        file_before,
        "the row under it waits with it"
    );
    assert_eq!(
        db.name_heal_retry().unwrap().unwrap().rows_left,
        2,
        "the folder and the file in it"
    );

    db.remove_operation("op-folder").unwrap();
    db.set_status(FOLDER, FileStatus::Local).unwrap();
    sync_tick_outcome_at(&bridge, dir.path(), T0 + 60).await.unwrap();
    assert_eq!(db.get_file(FOLDER).unwrap().unwrap().path, "Docs");
    assert_eq!(
        db.get_file(FILE).unwrap().unwrap().path,
        "Docs/Report.pdf",
        "named together with its folder"
    );
    assert_eq!(
        db.get_file(OTHER).unwrap().unwrap().path,
        other_label,
        "a row being trashed stays as it is"
    );
    assert!(
        db.name_heal_done().unwrap(),
        "the row being trashed does not keep the heal owed"
    );
}

/// Preparing the rows lifts their blocks before any row is named. A pass cut short after that (the app quit) is
/// still owed: it was recorded as unfinished first, so the next start does not find "nothing blocked" and stop.
#[tokio::test]
async fn a_pass_cut_short_after_the_blocks_were_lifted_is_still_owed() {
    let dir = tempfile::tempdir().unwrap();
    let server = Server::start(
        6,
        vec![("200 OK", serde_json::json!({"seq_id": 6, "nodes": healthy_rows()}))],
    );
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), MK);
    store_blocked(&bridge, dir.path(), healthy_rows()).await;
    let db = bridge.db();
    // What a pass does up to the point it is cut: the mark, then the blocks lifted, no row named.
    db.begin_name_heal_pass(T0).unwrap();
    bridge.prepare_owned_rows(&healthy_rows()).await.unwrap();
    assert!(
        !db.has_blocked_or_nameless_rows().unwrap(),
        "no blocked record is left to say a heal is owed"
    );
    assert_eq!(
        db.get_file(FOLDER).unwrap().unwrap().path,
        crate::owner_keys::blocked_name(FOLDER)
    );

    sync_tick_outcome_at(&bridge, dir.path(), T0 + 30).await.unwrap();
    assert!(!db.name_heal_done().unwrap(), "owed, and waiting");
    assert_eq!(server.snapshots(), 0);
    sync_tick_outcome_at(&bridge, dir.path(), T0 + 60).await.unwrap();
    assert_eq!(server.snapshots(), 1);
    assert_eq!(db.get_file(FOLDER).unwrap().unwrap().path, "Docs");
    assert!(db.name_heal_done().unwrap());
}

/// The macOS snapshot rule (spec §6.3.2), run here on every OS: on a row at its label with a parked change it
/// raises the version and the size and nothing else. The place, the status, the base the content was agreed on and
/// the base the parked change was queued on all stay, so the change is still sent on its own base.
#[tokio::test]
async fn the_macos_snapshot_version_rule_leaves_a_parked_change_its_base() {
    let dir = tempfile::tempdir().unwrap();
    let bridge = super::snapshot_names_2446::offline_bridge(dir.path());
    store_blocked(&bridge, dir.path(), healthy_rows()).await;
    let db = bridge.db();
    let label = crate::owner_keys::blocked_name(OTHER);
    seed_bridge_entry(&bridge, OTHER, &label, None, FileStatus::Error, false, 50);
    enqueue_test_operation(&bridge, "op-parked", OperationKind::UploadVersion, OTHER, 100);
    let op_before = db.get_operation("op-parked").unwrap().unwrap();
    let contract_before = db.get_file_contract_state(OTHER).unwrap().unwrap();

    db.apply_snapshot_version(OTHER, contract_before.current_version + 4, 9000, false)
        .unwrap();
    let row = db.get_file(OTHER).unwrap().unwrap();
    assert_eq!(
        (row.path.as_str(), &row.status, row.size_bytes),
        (label.as_str(), &FileStatus::Error, 9000)
    );
    let contract = db.get_file_contract_state(OTHER).unwrap().unwrap();
    assert_eq!(contract.current_version, contract_before.current_version + 4);
    assert_eq!(contract.local_base_version, contract_before.local_base_version);
    let op = db.get_operation("op-parked").unwrap().unwrap();
    assert_eq!(
        (op.base_version, op.target_path),
        (op_before.base_version, op_before.target_path)
    );
}
