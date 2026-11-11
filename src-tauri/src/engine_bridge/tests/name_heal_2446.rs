//! Task 2446, the heal: a build up to 0.8.24 blocked every row of a snapshot that carried null request fields, and
//! stored it under the blocked label. Once per state database the tick asks for one snapshot when such rows exist;
//! the heal is done only after that snapshot succeeded, and a database with no such row takes no extra snapshot.
//! The blocked state is made by the real path (a malformed envelope through `prepare_owned_rows`), never written by
//! hand, except the `Local` row, which only a hand-written row can reach.

use super::snapshot_names_2446::prod_row;
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

const MK: [u8; 32] = [7u8; 32];
const FOLDER: &str = "f0000000-0000-4000-8000-00000002446a";
const FILE: &str = "a0000000-0000-4000-8000-00000002446a";

/// `/sync/snapshot` answers from `snapshots` in order (then 500); `/sync/ops` answers "nothing new" at `head`;
/// every other path is 404. Counts the snapshot requests.
struct HealServer {
    base_url: String,
    snapshots_asked: Arc<AtomicUsize>,
}

fn heal_server(head: i64, snapshots: Vec<(&'static str, serde_json::Value)>) -> HealServer {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let snapshots_asked = Arc::new(AtomicUsize::new(0));
    let asked = snapshots_asked.clone();
    thread::spawn(move || {
        let mut snapshots = snapshots.into_iter();
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let request = read_http_request(&mut stream);
            let response = if request.path.starts_with("/api/v1/sync/snapshot") {
                asked.fetch_add(1, AtomicOrdering::SeqCst);
                let (status, body) = snapshots
                    .next()
                    .unwrap_or(("500 Internal Server Error", serde_json::json!({})));
                http_json(status, body)
            } else if request.path.starts_with("/api/v1/sync/ops") {
                http_json(
                    "200 OK",
                    serde_json::json!({"since": head, "head_seq": head, "ops": []}),
                )
            } else {
                http_json("404 Not Found", serde_json::json!({}))
            };
            let _ = stream.write_all(response.as_bytes());
        }
    });
    HealServer {
        base_url,
        snapshots_asked,
    }
}

fn healthy_rows() -> Vec<serde_json::Value> {
    vec![
        prod_row(FOLDER, None, "Docs", true),
        prod_row(FILE, Some(FOLDER), "Report.pdf", false),
    ]
}

/// The two rows as a snapshot stored them while they were blocked: one request field set and two null is a
/// malformed envelope, which blocks the row exactly as the null fields did before the fix.
async fn store_blocked(bridge: &EngineBridge, root: &Path) {
    let mut rows = healthy_rows();
    for row in &mut rows {
        row["file_request_id"] = serde_json::json!("0f000000-0000-4000-8000-0000000000e2");
    }
    let prepared = bridge.prepare_owned_rows(&rows).await.unwrap();
    assert!(prepared.iter().all(|row| !row.readable), "both rows are blocked");
    let snapshot = crate::api_client::SyncSnapshot {
        seq_id: 5,
        nodes: prepared.into_iter().map(|p| p.row).collect(),
    };
    apply_snapshot(bridge, root, &snapshot, 100, 100, &mut Vec::new()).unwrap();
    let label = crate::owner_keys::blocked_name(FOLDER);
    assert_eq!(
        bridge.db().get_file(FOLDER).unwrap().unwrap().path,
        label,
        "stored under the blocked label"
    );
    assert!(
        bridge
            .db()
            .get_file(FILE)
            .unwrap()
            .unwrap()
            .path
            .starts_with(&format!("{label}/Unavailable file "))
    );
    assert_eq!(bridge.db().get_sync_cursor().unwrap(), Some(5));
    assert!(bridge.db().has_blocked_or_nameless_rows().unwrap());
    assert!(!bridge.db().name_heal_done().unwrap());
}

#[tokio::test]
async fn a_failed_snapshot_leaves_the_heal_pending_and_a_good_one_completes_it() {
    let dir = tempfile::tempdir().unwrap();
    let server = heal_server(
        6,
        vec![
            ("500 Internal Server Error", serde_json::json!({})),
            ("200 OK", serde_json::json!({"seq_id": 6, "nodes": healthy_rows()})),
        ],
    );
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), MK);
    store_blocked(&bridge, dir.path()).await;

    // Pass 1: the snapshot fails. Nothing is named, and the heal is still owed.
    let _ = sync_tick(&bridge, dir.path()).await;
    assert_eq!(
        server.snapshots_asked.load(AtomicOrdering::SeqCst),
        1,
        "the heal asked for one snapshot"
    );
    assert!(
        !bridge.db().name_heal_done().unwrap(),
        "a failed snapshot does not complete the heal"
    );
    assert_eq!(
        bridge.db().get_file(FOLDER).unwrap().unwrap().path,
        crate::owner_keys::blocked_name(FOLDER)
    );

    // Pass 2: asked for again, it succeeds: both rows have their names, the child under its folder.
    sync_tick(&bridge, dir.path()).await.unwrap();
    assert_eq!(
        server.snapshots_asked.load(AtomicOrdering::SeqCst),
        2,
        "asked for again after the failure"
    );
    assert_eq!(bridge.db().get_file(FOLDER).unwrap().unwrap().path, "Docs");
    assert_eq!(bridge.db().get_file(FILE).unwrap().unwrap().path, "Docs/Report.pdf");
    assert!(
        !bridge.db().has_blocked_or_nameless_rows().unwrap(),
        "no row is blocked any more"
    );
    assert!(
        bridge.db().name_heal_done().unwrap(),
        "done after the snapshot succeeded"
    );

    // Pass 3: nothing more is asked for.
    sync_tick(&bridge, dir.path()).await.unwrap();
    assert_eq!(
        server.snapshots_asked.load(AtomicOrdering::SeqCst),
        2,
        "the heal runs once"
    );
}

#[tokio::test]
async fn a_healthy_database_takes_no_extra_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let server = heal_server(5, Vec::new());
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), MK);
    let prepared = bridge.prepare_owned_rows(&healthy_rows()).await.unwrap();
    let snapshot = crate::api_client::SyncSnapshot {
        seq_id: 5,
        nodes: prepared.into_iter().map(|p| p.row).collect(),
    };
    apply_snapshot(&bridge, dir.path(), &snapshot, 100, 100, &mut Vec::new()).unwrap();
    assert!(!bridge.db().name_heal_done().unwrap());

    for _ in 0..2 {
        sync_tick(&bridge, dir.path()).await.unwrap();
    }
    assert_eq!(
        server.snapshots_asked.load(AtomicOrdering::SeqCst),
        0,
        "no row needs a name: no snapshot"
    );
    assert!(
        bridge.db().name_heal_done().unwrap(),
        "marked done on the first pass, so the check does not run again"
    );
    assert_eq!(bridge.db().get_file(FILE).unwrap().unwrap().path, "Docs/Report.pdf");
}

#[tokio::test]
async fn a_rightly_blocked_row_stays_blocked_and_the_heal_still_runs_once() {
    let dir = tempfile::tempdir().unwrap();
    let mut rows = healthy_rows();
    // The file still carries a malformed envelope in the healing snapshot: it is rightly blocked.
    rows[1]["file_request_id"] = serde_json::json!("0f000000-0000-4000-8000-0000000000e2");
    let server = heal_server(6, vec![("200 OK", serde_json::json!({"seq_id": 6, "nodes": rows}))]);
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), MK);
    store_blocked(&bridge, dir.path()).await;

    for _ in 0..3 {
        sync_tick(&bridge, dir.path()).await.unwrap();
    }
    assert_eq!(
        server.snapshots_asked.load(AtomicOrdering::SeqCst),
        1,
        "one healing snapshot, never a loop"
    );
    assert_eq!(
        bridge.db().get_file(FOLDER).unwrap().unwrap().path,
        "Docs",
        "the folder is named"
    );
    let file = bridge.db().get_file(FILE).unwrap().unwrap();
    assert_eq!(
        file.path,
        format!("Docs/{}", crate::owner_keys::blocked_name(FILE)),
        "the heal lifts no block"
    );
    assert!(
        bridge
            .db()
            .get_owned_key_record(FILE)
            .unwrap()
            .unwrap()
            .blocked_fingerprint
            .is_some()
    );
}

/// A `Local` row is skipped by a snapshot whose time is not newer than the stored one, and production's text
/// `updated_at` reads as 0 there. A row still at the blocked label is applied anyway.
#[tokio::test]
async fn a_local_row_left_at_the_blocked_label_is_named_again() {
    let dir = tempfile::tempdir().unwrap();
    let bridge = super::snapshot_names_2446::offline_bridge(dir.path());
    store_blocked(&bridge, dir.path()).await;
    let label = crate::owner_keys::blocked_name(FOLDER);
    seed_bridge_entry(&bridge, FOLDER, &label, None, FileStatus::Local, true, 0);
    assert!(awaits_its_name(&bridge.db().get_file(FOLDER).unwrap().unwrap()));

    let prepared = bridge.prepare_owned_rows(&healthy_rows()).await.unwrap();
    let snapshot = crate::api_client::SyncSnapshot {
        seq_id: 6,
        nodes: prepared.into_iter().map(|p| p.row).collect(),
    };
    apply_snapshot(&bridge, dir.path(), &snapshot, 200, 200, &mut Vec::new()).unwrap();
    let folder = bridge.db().get_file(FOLDER).unwrap().unwrap();
    assert_eq!((folder.path.as_str(), &folder.status), ("Docs", &FileStatus::Local));
    assert!(!awaits_its_name(&folder));
    assert_eq!(bridge.db().get_file(FILE).unwrap().unwrap().path, "Docs/Report.pdf");
}
