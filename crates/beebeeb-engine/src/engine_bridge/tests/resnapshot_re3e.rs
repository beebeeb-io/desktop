//! RE3-e: Guus's PC fetched `/sync/snapshot` twice right after start (42k rows, 64-71 s each): the first
//! snapshot's own apply, or a request made while it was on its way, asked for another. A good snapshot is
//! fetched once. Not on macOS: there a request raised during a snapshot is kept on purpose (spec §6.3.2).
#![cfg(not(target_os = "macos"))]

use super::*;
use std::net::TcpListener;

const KEY: [u8; 32] = [91u8; 32];
/// A key that opens no name here: a name sealed under it stays unreadable.
const OTHER_KEY: [u8; 32] = [19u8; 32];
const FOLDER: &str = "e3e00000-0000-4000-8000-000000000001";
const FILE: &str = "e3e00000-0000-4000-8000-000000000002";
const UNREADABLE: &str = "e3e00000-0000-4000-8000-000000000003";

/// A list with a folder, a file in it, and a file whose name no key here opens (its first listing marks it and
/// asks for the list again: dc-136b).
fn snapshot() -> serde_json::Value {
    serde_json::json!({
        "seq_id": 7,
        "nodes": [
            snap_node(&KEY, FOLDER, "docs", None, true, 100),
            snap_node(&KEY, FILE, "notes.txt", Some(FOLDER), false, 100),
            snap_node(&OTHER_KEY, UNREADABLE, "secret.txt", None, false, 100),
        ],
    })
}

/// The canonical row the decrypt pass asks for when a listed name does not open here.
fn unreadable_row() -> serde_json::Value {
    snap_node(&OTHER_KEY, UNREADABLE, "secret.txt", None, false, 100)
}

/// One snapshot (with the canonical fetch of its unreadable row), then one idle ops page.
fn once_then_ops() -> Vec<serde_json::Value> {
    vec![snapshot(), unreadable_row(), idle_ops()]
}

fn expected_paths() -> [String; 3] {
    [
        "/api/v1/sync/snapshot".to_string(),
        format!("/api/v1/files/{UNREADABLE}"),
        "/api/v1/sync/ops?since=7".to_string(),
    ]
}

fn idle_ops() -> serde_json::Value {
    serde_json::json!({ "since": 7, "head_seq": 7, "ops": [] })
}

/// Serves `responses` in order; `on_request(i)` runs before the i-th answer is written (the list "on its way").
fn serve(
    responses: Vec<serde_json::Value>,
    on_request: impl Fn(usize) + Send + 'static,
) -> (String, std::thread::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let handle = std::thread::spawn(move || {
        let mut paths = Vec::new();
        for (i, body) in responses.into_iter().enumerate() {
            let (mut stream, _) = listener.accept().unwrap();
            paths.push(read_http_request(&mut stream).path);
            on_request(i);
            std::io::Write::write_all(&mut stream, http_json("200 OK", body).as_bytes()).unwrap();
        }
        paths
    });
    (base_url, handle)
}

/// A start with a re-snapshot pending (the envelope heal, a name retry) fetches the list once; the next tick goes to
/// the ops path.
#[tokio::test]
async fn resnapshot_clean_start_with_a_good_snapshot_fetches_it_once() {
    let dir = tempfile::tempdir().unwrap();
    let (base_url, server) = serve(once_then_ops(), |_| {});
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), base_url, KEY);
    bridge.db().set_sync_cursor(5).unwrap();
    bridge.db().request_resnapshot().unwrap();

    sync_tick(&bridge, dir.path()).await.unwrap();
    assert_eq!(bridge.db().get_sync_cursor().unwrap(), Some(7));
    assert_eq!(bridge.db().get_file(FILE).unwrap().unwrap().path, "docs/notes.txt");
    assert_eq!(
        bridge.db().peek_resnapshot_request().unwrap(),
        None,
        "the list just applied asks for no second list"
    );

    sync_tick(&bridge, dir.path()).await.unwrap();
    assert_eq!(
        server.join().unwrap(),
        expected_paths(),
        "one snapshot, then the ops path"
    );
}

/// Red test: the apply of a list that marks a name unreadable (dc-136b asks for the list again) asks for no second
/// list: this list was the fresh one, and the same keys read it the same way again.
#[test]
fn resnapshot_not_asked_again_by_the_apply_of_a_fresh_list() {
    let dir = tempfile::tempdir().unwrap();
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), "http://127.0.0.1:9".into(), KEY);
    let snapshot: crate::api_client::SyncSnapshot = serde_json::from_value(snapshot()).unwrap();

    apply_snapshot(&bridge, dir.path(), &snapshot, 1_000, 1_000, &mut Vec::new()).unwrap();

    assert!(
        bridge.db().name_unreadable(UNREADABLE).unwrap(),
        "the unreadable name is marked"
    );
    assert_eq!(bridge.db().get_sync_cursor().unwrap(), Some(7));
    assert_eq!(bridge.db().peek_resnapshot_request().unwrap(), None, "no second list");
    // Outside a list's apply the same mark still asks for the list (an op's row is not the list).
    assert!(bridge.db().set_name_unreadable(UNREADABLE, false).unwrap());
    let row = snap_node(&OTHER_KEY, UNREADABLE, "secret.txt", None, false, 200);
    process_metadata_row(&bridge, &row, "", 1_000, RowSource::Op, &mut Vec::new()).unwrap();
    assert!(bridge.db().peek_resnapshot_request().unwrap().is_some());
}

/// Red test: a request made while the list is on its way (the first reconcile pass's name retry after unlock) is served by
/// that list: its decrypt runs after the request.
#[tokio::test]
async fn resnapshot_asked_while_the_list_is_on_its_way_is_served_by_it() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("state.db");
    let bridge_db = db_path.clone();
    let (base_url, server) = serve(once_then_ops(), move |i| {
        if i == 0 {
            StateDb::open(bridge_db.clone()).unwrap().request_resnapshot().unwrap();
        }
    });
    let bridge = test_bridge_with_api(&db_path, base_url, KEY);
    bridge.db().set_sync_cursor(5).unwrap();
    bridge.db().request_resnapshot().unwrap();

    sync_tick(&bridge, dir.path()).await.unwrap();
    assert_eq!(bridge.db().peek_resnapshot_request().unwrap(), None);
    sync_tick(&bridge, dir.path()).await.unwrap();
    assert_eq!(server.join().unwrap(), expected_paths());
}

/// Only the apply's own requests are served: one made outside it, or from another thread during it, lands.
#[test]
fn resnapshot_outside_a_snapshot_apply_is_still_requested() {
    let dir = tempfile::tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    StateDb::during_snapshot_apply(|| db.request_resnapshot().unwrap());
    assert_eq!(db.peek_resnapshot_request().unwrap(), None, "inside the apply: served");
    db.request_resnapshot().unwrap();
    assert!(db.peek_resnapshot_request().unwrap().is_some(), "outside it: requested");
    // The guard is per thread: another thread's request during an apply lands.
    let db = std::sync::Arc::new(db);
    StateDb::during_snapshot_apply(|| {
        let other = std::sync::Arc::clone(&db);
        std::thread::spawn(move || other.take_needs_resnapshot().unwrap())
            .join()
            .unwrap();
        let other = std::sync::Arc::clone(&db);
        std::thread::spawn(move || other.request_resnapshot().unwrap())
            .join()
            .unwrap();
    });
    assert!(
        db.peek_resnapshot_request().unwrap().is_some(),
        "another thread's request lands"
    );
}

/// A change newer than the list's `seq_id` that happens while the list downloads is not lost by serving the
/// request made then: the ops cursor starts at the list's `seq_id`, so the next tick's `/sync/ops?since=7` brings
/// the change (here a delete at seq 8).
#[tokio::test]
async fn resnapshot_a_change_made_during_the_download_arrives_through_the_ops() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("state.db");
    let during = db_path.clone();
    let list = serde_json::json!({
        "seq_id": 7,
        "nodes": [
            snap_node(&KEY, FOLDER, "docs", None, true, 100),
            snap_node(&KEY, FILE, "notes.txt", Some(FOLDER), false, 100),
        ],
    });
    let ops = serde_json::json!({
        "since": 7,
        "head_seq": 8,
        "ops": [{ "seq_id": 8, "op_type": "file_delete", "payload": { "id": FILE } }],
    });
    let (base_url, server) = serve(vec![list, ops], move |i| {
        if i == 0 {
            // The request made while the list is on its way (the server deleted the file after the list's seq).
            StateDb::open(during.clone()).unwrap().request_resnapshot().unwrap();
        }
    });
    let bridge = test_bridge_with_api(&db_path, base_url, KEY);
    bridge.db().set_sync_cursor(5).unwrap();
    bridge.db().request_resnapshot().unwrap();

    sync_tick(&bridge, dir.path()).await.unwrap();
    assert_eq!(
        bridge.db().get_sync_cursor().unwrap(),
        Some(7),
        "the cursor is the list's, not later"
    );
    assert_eq!(bridge.db().peek_resnapshot_request().unwrap(), None);
    assert!(bridge.db().get_file(FILE).unwrap().is_some());

    sync_tick(&bridge, dir.path()).await.unwrap();
    assert_eq!(bridge.db().get_sync_cursor().unwrap(), Some(8));
    assert!(
        bridge.db().get_file(FILE).unwrap().is_none(),
        "the change after the list is applied by the ops"
    );
    assert_eq!(
        server.join().unwrap(),
        ["/api/v1/sync/snapshot", "/api/v1/sync/ops?since=7"]
    );
}
