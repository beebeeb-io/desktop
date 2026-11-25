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

/// Red test: a start with a re-snapshot pending (the envelope heal, a name retry) fetches the list once; the next
/// tick goes to the ops path although the list's apply marked an unreadable name.
#[tokio::test]
async fn resnapshot_clean_start_with_a_good_snapshot_fetches_it_once() {
    let dir = tempfile::tempdir().unwrap();
    let (base_url, server) = serve(vec![snapshot(), idle_ops()], |_| {});
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), base_url, KEY);
    bridge.db().set_sync_cursor(5).unwrap();
    bridge.db().request_resnapshot().unwrap();

    sync_tick(&bridge, dir.path()).await.unwrap();
    assert_eq!(bridge.db().get_sync_cursor().unwrap(), Some(7));
    assert_eq!(bridge.db().get_file(FILE).unwrap().unwrap().path, "docs/notes.txt");
    assert!(
        bridge.db().name_unreadable(UNREADABLE).unwrap(),
        "the unreadable name is marked"
    );
    assert_eq!(
        bridge.db().peek_resnapshot_request().unwrap(),
        None,
        "the list just applied asks for no second list"
    );

    sync_tick(&bridge, dir.path()).await.unwrap();
    assert_eq!(
        server.join().unwrap(),
        ["/api/v1/sync/snapshot", "/api/v1/sync/ops?since=7"],
        "one snapshot, then the ops path"
    );
}

/// A request made while the list is on its way (the first reconcile pass's name retry after unlock) is served by
/// that list: its decrypt runs after the request.
#[tokio::test]
async fn resnapshot_asked_while_the_list_is_on_its_way_is_served_by_it() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("state.db");
    let bridge_db = db_path.clone();
    let (base_url, server) = serve(vec![snapshot(), idle_ops()], move |i| {
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
    assert_eq!(
        server.join().unwrap(),
        ["/api/v1/sync/snapshot", "/api/v1/sync/ops?since=7"]
    );
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
