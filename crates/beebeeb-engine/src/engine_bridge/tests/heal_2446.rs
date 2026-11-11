//! Task 2446: the one-time name heal at tick level, when its forced re-snapshot keeps failing.

use super::*;

/// Off macOS the heal asks for a snapshot on every tick until one succeeds, and a failed re-snapshot ends the tick
/// with an error (`after_failed_snapshot` lets the tick go on only on macOS). The runner runs its queue step only
/// after a tick that returned `Ok`, so a snapshot that keeps failing for a reason waiting does not cure (an
/// endpoint that times out on a large tree, a local database error) stops every upload, download and delete on
/// this PC for as long as a blocked or nameless row exists. The rule this pins: a pending heal must not keep the
/// tick from reaching the ops pull. Within five ticks whose snapshot fails, at least one returns `Ok` and has
/// pulled the ops from the kept cursor; the heal stays pending (it is marked done only after a good snapshot).
#[cfg(not(target_os = "macos"))]
#[ignore = "known red (task 2446 heal): a failing forced re-snapshot ends every tick off macOS. Lane H1c fixes it \
            on hotfix/0.8.25-prep (desktop PR #289); port that change and its tests here, then drop this ignore"]
#[tokio::test]
async fn a_heal_whose_snapshot_keeps_failing_does_not_keep_the_tick_from_the_ops_pull() {
    let dir = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let done = stop.clone();
    let handle = thread::spawn(move || {
        let mut paths = Vec::new();
        while !done.load(std::sync::atomic::Ordering::SeqCst) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    stream.set_nonblocking(false).unwrap();
                    stream
                        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                        .unwrap();
                    let req = read_http_request(&mut stream);
                    let response = if req.path.contains("/sync/ops") {
                        http_json("200 OK", serde_json::json!({"since": 8, "head_seq": 8, "ops": []}))
                    } else {
                        // The snapshot, every time: a failure that waiting does not cure.
                        http_json(
                            "500 Internal Server Error",
                            serde_json::json!({"error": "snapshot failed"}),
                        )
                    };
                    paths.push(req.path);
                    stream.write_all(response.as_bytes()).unwrap();
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(std::time::Duration::from_millis(2))
                }
                Err(e) => panic!("accept: {e}"),
            }
        }
        paths
    });
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), url, [3u8; 32]);
    bridge.db().set_sync_cursor(8).unwrap();
    // A row without a path: what the null-envelope bug left behind, and what makes the heal ask for a snapshot.
    bridge
        .db()
        .upsert_file(&FileEntry {
            file_id: "nameless-2446".into(),
            path: String::new(),
            status: FileStatus::CloudOnly,
            size_bytes: 1,
            modified_at: 0,
            content_hash: None,
            remote_updated_at: 0,
            parent_id: None,
            item_kind: ItemKind::File,
        })
        .unwrap();
    assert!(!bridge.db().envelope_heal_done().unwrap());
    assert!(bridge.db().has_blocked_or_nameless_rows().unwrap());

    let mut ok_ticks = 0;
    for _ in 0..5 {
        if sync_tick(&bridge, dir.path()).await.is_ok() {
            ok_ticks += 1;
        }
    }
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    let paths = handle.join().unwrap();
    let snapshots = paths.iter().filter(|p| p.contains("/sync/snapshot")).count();
    let ops_pulls = paths.iter().filter(|p| p.contains("/sync/ops?since=8")).count();
    assert!(snapshots >= 1, "the heal asked for its snapshot: {paths:?}");
    assert!(
        ok_ticks >= 1 && ops_pulls >= 1,
        "five ticks, the snapshot failed each time: {ok_ticks} returned Ok and {ops_pulls} pulled ops ({paths:?})"
    );
    assert_eq!(bridge.db().get_sync_cursor().unwrap(), Some(8), "the cursor is kept");
    assert!(
        !bridge.db().envelope_heal_done().unwrap(),
        "no snapshot succeeded, so the heal is still pending"
    );
}
