//! `sync` engine tests, split out of `engine_bridge.rs`.

use super::*;
#[test]
fn tick1697_apply_sync_op_collects_the_applied_create_id() {
    let dir = tempfile::tempdir().unwrap();
    let bridge = test_bridge(&dir.path().join("state.db"));
    let file = "rem00000-0000-4000-8000-000000000001";
    let op = crate::api_client::SyncOp {
        seq_id: 3,
        op_type: "file_create".into(),
        payload: serde_json::json!({ "id": file, "parent_id": serde_json::Value::Null, "name_encrypted": "new.txt", "size_bytes": 10 }),
    };
    let mut conflicts = Vec::new();
    let applied = apply_sync_op(&bridge, dir.path(), &op, 200, &mut conflicts).unwrap();
    assert!(
        bridge.db().get_file(file).unwrap().is_some(),
        "the create must really have been ingested"
    );
    assert_eq!(
        applied,
        vec![file.to_string()],
        "an applied create op must report its file id for the working-set signal"
    );
}

#[test]
fn tick1697_apply_sync_op_collects_remote_delete_subtree_ids() {
    // FLIPPED BY TASK 1698 (trash ruling — full sync): a server-side trash
    // of a folder no longer removes rows — it flips the folder AND its
    // descendants to `Trashing` (the macOS trash view mirrors the server
    // trash), reporting both ids so the replica moves the subtree into the
    // trash container. The hierarchy is PATH-based (the descendant sweep
    // is a path-prefix match), so the child must be seeded under the
    // folder's path.
    let dir = tempfile::tempdir().unwrap();
    let mk = [9u8; 32];
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), "http://placeholder".into(), mk);
    let folder = "fold0000-0000-4000-8000-000000000001";
    let child = "chil0000-0000-4000-8000-000000000002";
    seed_bridge_entry(&bridge, folder, "docs", None, FileStatus::CloudOnly, true, 10);
    seed_bridge_entry(&bridge, child, "docs/notes.txt", None, FileStatus::CloudOnly, false, 10);

    let op = crate::api_client::SyncOp {
        seq_id: 4,
        op_type: "file_trash".into(),
        payload: serde_json::json!({ "id": folder }),
    };
    let mut conflicts = Vec::new();
    let applied = apply_sync_op(&bridge, dir.path(), &op, 200, &mut conflicts).unwrap();
    assert!(
        bridge.db().get_file(folder).unwrap().is_some(),
        "the trash keeps the folder (trash view)"
    );
    assert!(
        bridge.db().get_file(child).unwrap().is_some(),
        "the trash keeps the descendant (trash view)"
    );
    for id in [folder, child] {
        assert_eq!(
            bridge.db().get_file(id).unwrap().unwrap().status,
            FileStatus::Trashing,
            "{id} must be Trashing"
        );
    }
    assert!(
        applied.contains(&folder.to_string()) && applied.contains(&child.to_string()),
        "remote delete must report the folder AND its descendants: {applied:?}"
    );
}

#[test]
fn tick1697_apply_sync_op_trash_echo_of_local_trashing_collects_nothing() {
    // The server echo of a LOCAL delete-in-flight must not report ids: the
    // local queue's own completion signals those via `operations_applied`
    // (double-signaling the same item would be redundant).
    let dir = tempfile::tempdir().unwrap();
    let bridge = test_bridge(&dir.path().join("state.db"));
    let file = "echo0000-0000-4000-8000-000000000001";
    seed_bridge_entry(&bridge, file, "doomed.txt", None, FileStatus::Trashing, false, 10);
    let op = crate::api_client::SyncOp {
        seq_id: 5,
        op_type: "file_trash".into(),
        payload: serde_json::json!({ "id": file }),
    };
    let mut conflicts = Vec::new();
    let applied = apply_sync_op(&bridge, dir.path(), &op, 200, &mut conflicts).unwrap();
    assert!(
        applied.is_empty(),
        "a Trashing echo applies nothing itself: {applied:?}"
    );
    assert!(
        bridge.db().get_file(file).unwrap().is_some(),
        "the echo must leave the row to the 0802 path"
    );
}

#[test]
fn tick1697_apply_snapshot_collects_ingested_and_pruned_ids() {
    // A snapshot re-bootstrap ingests nodes and prunes rows absent from it;
    // BOTH sides are remote changes Finder must see.
    let dir = tempfile::tempdir().unwrap();
    let mk = [9u8; 32];
    let bridge = test_bridge(&dir.path().join("state.db"));
    let kept = "keep0000-0000-4000-8000-000000000001";
    let absent = "abse0000-0000-4000-8000-000000000002";
    // `absent` exists locally but the snapshot below omits it → pruned.
    seed_bridge_entry(&bridge, absent, "stale.txt", None, FileStatus::CloudOnly, false, 10);
    let snapshot = crate::api_client::SyncSnapshot {
        seq_id: 9,
        nodes: vec![snap_node(&mk, kept, "fresh.txt", None, false, 50)],
    };
    let mut conflicts = Vec::new();
    let applied = apply_snapshot(&bridge, dir.path(), &snapshot, 200, 200, &mut conflicts).unwrap();
    assert!(
        bridge.db().get_file(kept).unwrap().is_some(),
        "the snapshot node must be ingested"
    );
    assert!(
        bridge.db().get_file(absent).unwrap().is_none(),
        "the row absent from the snapshot must be pruned"
    );
    assert!(
        applied.contains(&kept.to_string()),
        "ingested ids are collected: {applied:?}"
    );
    assert!(
        applied.contains(&absent.to_string()),
        "pruned (remote-deleted) ids are collected: {applied:?}"
    );
}

#[test]
fn audit_1244_apply_sync_op_trash_echo_preserves_local_trashing_owner() {
    // Production mutation caught: removing a Trashing row on a file_trash echo
    // would make the local-delete-in-flight path lose its durable owner state.
    let dir = tempfile::tempdir().unwrap();
    let bridge = test_bridge(&dir.path().join("state.db"));
    let file = "echo0000-0000-4000-8000-000000000001";
    seed_bridge_entry(&bridge, file, "doomed.txt", None, FileStatus::Trashing, false, 10);
    enqueue_test_operation(
        &bridge,
        "4ae33359-9e84-59f6-959d-2f62cb9d6a7e",
        OperationKind::TrashFile,
        file,
        100,
    );

    let op = crate::api_client::SyncOp {
        seq_id: 12,
        op_type: "file_trash".into(),
        payload: serde_json::json!({ "id": file }),
    };
    let mut conflicts = Vec::new();
    apply_sync_op(&bridge, dir.path(), &op, 200, &mut conflicts).unwrap();

    let row = bridge.db().get_file(file).unwrap().unwrap();
    assert_eq!(row.status, FileStatus::Trashing);
    let queued = bridge.db().list_due_operations(999).unwrap();
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].op_id, "4ae33359-9e84-59f6-959d-2f62cb9d6a7e");
    assert!(conflicts.is_empty());
}

#[test]
fn audit_1244_double_trash_restore_cycle_keeps_resnapshot_for_final_restore() {
    // Production mutation caught: treating file_restore as a no-op, or clearing
    // the resnapshot request during the same op batch, would leave the final
    // restored row invisible after two fast trash+restore cycles.
    let dir = tempfile::tempdir().unwrap();
    let mk = [8u8; 32];
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), "http://placeholder".into(), mk);
    let file = "cycle000-0000-4000-8000-000000000001";
    seed_bridge_entry(&bridge, file, "doc.txt", None, FileStatus::CloudOnly, false, 10);

    let mut conflicts = Vec::new();
    for (seq_id, op_type) in [
        (2, "file_trash"),
        (3, "file_restore"),
        (4, "file_trash"),
        (5, "file_restore"),
    ] {
        let op = crate::api_client::SyncOp {
            seq_id,
            op_type: op_type.into(),
            payload: serde_json::json!({ "id": file }),
        };
        apply_sync_op(&bridge, dir.path(), &op, 200, &mut conflicts).unwrap();
    }

    // Task 1698: the trash no longer removes the row — it flips it
    // `Trashing` (trash view) and the restore flips it straight back
    // (`CloudOnly`). Two fast trash+restore cycles converge to
    // CloudOnly with the resnapshot still requested.
    assert!(
        bridge.db().get_file(file).unwrap().is_some(),
        "the trash view keeps the row alive (1698); restore keeps it alive"
    );
    assert_eq!(
        bridge.db().get_file(file).unwrap().unwrap().status,
        FileStatus::CloudOnly,
        "the last restore in the batch un-trashed the row"
    );
    #[cfg(target_os = "macos")]
    assert!(
        bridge.db().peek_resnapshot_request().unwrap().is_some(),
        "at least one restore in the batch must force the next tick to snapshot"
    );
    #[cfg(not(target_os = "macos"))]
    assert!(
        bridge.db().take_needs_resnapshot().unwrap(),
        "at least one restore in the batch must force the next tick to snapshot"
    );

    let snapshot = crate::api_client::SyncSnapshot {
        seq_id: 5,
        nodes: vec![snap_node(&mk, file, "doc.txt", None, false, 50)],
    };
    apply_snapshot(&bridge, dir.path(), &snapshot, 250, 250, &mut conflicts).unwrap();

    let row = bridge.db().get_file(file).unwrap().unwrap();
    assert_eq!(row.path, "doc.txt");
    assert_eq!(row.status, FileStatus::CloudOnly);
    assert!(conflicts.is_empty());
}

#[test]
fn audit_1244_gap_rebootstrap_keeps_offline_local_upload_when_remote_delete_absent() {
    // Production mutation caught: pruning rows with pending UploadVersion work
    // during a gap-recovery snapshot would silently delete a local offline edit
    // just because another device remotely deleted the server row.
    let dir = tempfile::tempdir().unwrap();
    let mk = [4u8; 32];
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), "http://placeholder".into(), mk);
    let local = "local000-0000-4000-8000-000000000001";
    let other = "other000-0000-4000-8000-000000000002";
    seed_bridge_entry(&bridge, local, "report.txt", None, FileStatus::Error, false, 10);
    enqueue_test_operation(
        &bridge,
        "7c988b48-ad5b-59bf-abd7-757e7ca8bb05",
        OperationKind::UploadVersion,
        local,
        100,
    );

    let snapshot = crate::api_client::SyncSnapshot {
        seq_id: 77,
        nodes: vec![snap_node(&mk, other, "other.txt", None, false, 80)],
    };
    let mut conflicts = Vec::new();
    apply_snapshot(&bridge, dir.path(), &snapshot, 200, 200, &mut conflicts).unwrap();

    let row = bridge.db().get_file(local).unwrap().unwrap();
    assert_eq!(row.path, "report.txt");
    assert_eq!(row.status, FileStatus::Error);
    assert_eq!(
        bridge.db().list_due_operations(999).unwrap()[0].op_id,
        "7c988b48-ad5b-59bf-abd7-757e7ca8bb05",
        "pending local upload remains durable after rebootstrap"
    );
    assert!(bridge.db().get_file(other).unwrap().is_some());
    assert!(conflicts.is_empty());
}

#[test]
fn audit_1244_snapshot_partial_folder_trash_prunes_children_seen_without_parent() {
    // Production mutation caught: re-rooting children whose parent folder is
    // absent from the snapshot before prune_absent runs leaks ghost rows.
    let dir = tempfile::tempdir().unwrap();
    let mk = [6u8; 32];
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), "http://placeholder".into(), mk);
    let folder = "fold0000-0000-4000-8000-000000000001";
    let child = "fold0000-0000-4000-8000-000000000002";
    let subfolder = "fold0000-0000-4000-8000-000000000003";
    let grandchild = "fold0000-0000-4000-8000-000000000004";
    let sibling = "fold0000-0000-4000-8000-000000000005";

    seed_bridge_entry(&bridge, folder, "docs", None, FileStatus::CloudOnly, true, 10);
    seed_bridge_entry(
        &bridge,
        child,
        "docs/a.txt",
        Some(folder),
        FileStatus::CloudOnly,
        false,
        10,
    );
    seed_bridge_entry(
        &bridge,
        subfolder,
        "docs/sub",
        Some(folder),
        FileStatus::CloudOnly,
        true,
        10,
    );
    seed_bridge_entry(
        &bridge,
        grandchild,
        "docs/sub/b.txt",
        Some(subfolder),
        FileStatus::CloudOnly,
        false,
        10,
    );
    seed_bridge_entry(&bridge, sibling, "outside.txt", None, FileStatus::CloudOnly, false, 10);

    let snapshot = crate::api_client::SyncSnapshot {
        seq_id: 88,
        nodes: vec![
            // Concurrent partial sync: the server snapshot still contains the
            // folder's independent children, but the trashed folder node itself
            // is absent. The local mirror must remove the subtree, not re-root it.
            snap_node(&mk, child, "a.txt", Some(folder), false, 80),
            snap_node(&mk, subfolder, "sub", Some(folder), true, 80),
            snap_node(&mk, grandchild, "b.txt", Some(subfolder), false, 80),
            snap_node(&mk, sibling, "outside.txt", None, false, 80),
        ],
    };
    let mut conflicts = Vec::new();
    apply_snapshot(&bridge, dir.path(), &snapshot, 200, 200, &mut conflicts).unwrap();

    assert!(bridge.db().get_file(folder).unwrap().is_none());
    assert!(
        bridge.db().get_file(child).unwrap().is_none(),
        "child listed in a partial snapshot must still be pruned with its absent folder"
    );
    assert!(bridge.db().get_file(subfolder).unwrap().is_none());
    assert!(bridge.db().get_file(grandchild).unwrap().is_none());
    assert!(bridge.db().get_file(sibling).unwrap().is_some());
    assert!(conflicts.is_empty());
}

// ── /sync delta engine tests (task 0789) ──────────────────────────────────
//
// A scriptable mock that answers `/api/v1/sync/snapshot` and
// `/api/v1/sync/ops` from a FIFO of canned responses (status + JSON body)
// and records every request path. Mirrors `UploadMockServer`'s raw-TCP shape
// (the existing test harness) so it shares the request-capture pattern the
// 0789 plan calls out.

pub(super) struct SyncMockServer {
    pub(super) base_url: String,
    pub(super) requests: Arc<Mutex<Vec<RecordedRequest>>>,
    pub(super) handle: thread::JoinHandle<()>,
}

impl SyncMockServer {
    /// `responses`: ordered `(http_status, json_body)` answered one per
    /// incoming request. The server serves exactly `responses.len()`
    /// requests then exits, so the test must drive precisely that many.
    pub(super) fn start(responses: Vec<(String, serde_json::Value)>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let server_requests = Arc::clone(&requests);
        let handle = thread::spawn(move || {
            for (status, body) in responses {
                let (mut stream, _) = listener.accept().unwrap();
                let request = read_http_request(&mut stream);
                server_requests.lock().unwrap().push(request);
                let response = http_json(&status, body);
                stream.write_all(response.as_bytes()).unwrap();
            }
        });
        Self {
            base_url,
            requests,
            handle,
        }
    }

    pub(super) fn finish(self) -> Vec<RecordedRequest> {
        self.handle.join().unwrap();
        Arc::try_unwrap(self.requests).unwrap().into_inner().unwrap()
    }
}

#[tokio::test]
async fn test_1909_idle_head_avoids_snapshot() {
    for head in [0, 8, 12] {
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
                        // Darwin may inherit the listener's nonblocking flag.
                        stream.set_nonblocking(false).unwrap();
                        stream
                            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                            .unwrap();
                        let req = read_http_request(&mut stream);
                        let body = if req.path.contains("/sync/ops") {
                            serde_json::json!({"since": head, "head_seq": head, "ops": []})
                        } else {
                            serde_json::json!({"seq_id": head, "nodes": []})
                        };
                        paths.push(req.path);
                        stream.write_all(http_json("200 OK", body).as_bytes()).unwrap();
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
        bridge.db().set_sync_cursor(head).unwrap();
        for _ in 0..3 {
            sync_tick(&bridge, dir.path()).await.unwrap();
        }
        stop.store(true, std::sync::atomic::Ordering::SeqCst);
        let paths = handle.join().unwrap();
        assert_eq!(paths.len(), 3, "idle ticks must use only ops, even with cursor zero");
        assert!(paths.iter().all(|p| p.contains("/sync/ops?since=")));
        assert_eq!(bridge.db().get_sync_cursor().unwrap(), Some(head));
    }
}

#[tokio::test]
async fn test_1909_cursor_ahead_of_head_recovers_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let server = SyncMockServer::start(vec![
        (
            "200 OK".into(),
            serde_json::json!({"since": 50, "head_seq": 3, "ops": []}),
        ),
        ("200 OK".into(), serde_json::json!({"seq_id": 3, "nodes": []})),
    ]);
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), [3u8; 32]);
    bridge.db().set_sync_cursor(50).unwrap();
    sync_tick(&bridge, dir.path()).await.unwrap();
    let requests = server.finish();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1].path, "/api/v1/sync/snapshot");
    assert_eq!(bridge.db().get_sync_cursor().unwrap(), Some(3));
}

pub(super) fn enc_name(master_key: &[u8; 32], file_id: &str, name: &str) -> String {
    let mk = beebeeb_core::kdf::MasterKey::from_bytes(*master_key);
    beebeeb_core::encrypt::encrypt_name(&mk, file_id, name, None).unwrap()
}

/// Build a snapshot node the way the server's `/sync/snapshot` does.
pub(super) fn snap_node(
    master_key: &[u8; 32],
    id: &str,
    name: &str,
    parent_id: Option<&str>,
    is_folder: bool,
    updated_at: i64,
) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "parent_id": parent_id,
        "name_encrypted": enc_name(master_key, id, name),
        "size_bytes": 10,
        "is_folder": is_folder,
        "content_hash": serde_json::Value::Null,
        "version_number": 1,
        "is_trashed": false,
        "is_starred": false,
        "updated_at": updated_at,
    })
}

#[tokio::test]
async fn test_sync_tick_bootstrap_snapshot_ingests_same_rows_as_walk() {
    // cursor unset → ONE /sync/snapshot bootstrap. Every node lands as a
    // cloud_only row with the decrypted nested path, and the cursor is set
    // to the snapshot's seq_id.
    let dir = tempfile::tempdir().unwrap();
    let mk = [7u8; 32];
    let folder = "f0000000-0000-4000-8000-000000000001";
    let child = "c0000000-0000-4000-8000-000000000002";
    let root_file = "a0000000-0000-4000-8000-000000000003";
    let snapshot = serde_json::json!({
        "seq_id": 12,
        "nodes": [
            snap_node(&mk, folder, "docs", None, true, 100),
            snap_node(&mk, child, "notes.txt", Some(folder), false, 100),
            snap_node(&mk, root_file, "top.txt", None, false, 100),
        ],
    });
    let server = SyncMockServer::start(vec![("200 OK".into(), snapshot)]);
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), mk);

    let conflicts = sync_tick(&bridge, dir.path()).await.unwrap();
    assert!(conflicts.is_empty());

    let requests = server.finish();
    assert_eq!(requests.len(), 1, "bootstrap = exactly one request");
    assert_eq!(requests[0].path, "/api/v1/sync/snapshot");

    // Nested + root rows all present, paths nested correctly, all cloud_only.
    let f = bridge.db().get_file(folder).unwrap().unwrap();
    assert_eq!(f.path, "docs");
    assert_eq!(f.item_kind, ItemKind::Folder);
    let c = bridge.db().get_file(child).unwrap().unwrap();
    assert_eq!(c.path, "docs/notes.txt", "child nested under parent's resolved path");
    assert_eq!(c.status, FileStatus::CloudOnly);
    let r = bridge.db().get_file(root_file).unwrap().unwrap();
    assert_eq!(r.path, "top.txt");
    // Cursor advanced to the snapshot seq_id.
    assert_eq!(bridge.db().get_sync_cursor().unwrap(), Some(12));
}

#[tokio::test]
async fn test_sync_tick_snapshot_prunes_row_absent_from_snapshot() {
    // A known local row the fresh snapshot OMITS must be pruned (the silent
    // deletion-reconciliation fix).
    let dir = tempfile::tempdir().unwrap();
    let mk = [4u8; 32];
    let kept = "11111111-0000-4000-8000-000000000001";
    let stale = "22222222-0000-4000-8000-000000000002";
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), "http://placeholder".into(), mk);
    // Pre-seed a settled own-tree row that the snapshot will NOT mention.
    bridge
        .db()
        .upsert_file(&FileEntry {
            file_id: stale.into(),
            path: "deleted-server-side.txt".into(),
            status: FileStatus::CloudOnly,
            size_bytes: 1,
            modified_at: 0,
            content_hash: None,
            remote_updated_at: 0,
            parent_id: None,
            item_kind: ItemKind::File,
        })
        .unwrap();

    let snapshot = serde_json::json!({
        "seq_id": 5,
        "nodes": [ snap_node(&mk, kept, "kept.txt", None, false, 50) ],
    });
    let server = SyncMockServer::start(vec![("200 OK".into(), snapshot)]);
    // Re-point the bridge at the mock by rebuilding it on the same DB path.
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), mk);

    sync_tick(&bridge, dir.path()).await.unwrap();
    server.finish();

    assert!(bridge.db().get_file(kept).unwrap().is_some());
    assert!(
        bridge.db().get_file(stale).unwrap().is_none(),
        "snapshot-absent row pruned"
    );
}

#[tokio::test]
async fn test_sync_tick_snapshot_window_does_not_resurrect_trashing_row() {
    // task 0802 — THE WINDOW (the bug this fix closes). A file was just
    // locally deleted: its row is `Trashing`, its `TrashFile` op already
    // SUCCEEDED (so the op is gone). On the very next tick the server's
    // `/sync/snapshot` read is replica-lagged / same-second and STILL lists
    // the (now-trashed) file. The OLD code's None/insert arm re-inserted a
    // FRESH `CloudOnly` row here and re-minted the placeholder — the "deleted
    // file comes back" bug. The fix: `process_metadata_row`'s `Trashing` guard
    // preserves the row untouched, and `prune_absent` does NOT remove it while
    // it's still listed. Assert: row stays `Trashing`, never `CloudOnly`.
    let dir = tempfile::tempdir().unwrap();
    let mk = [7u8; 32];
    let trashing = "33333333-0000-4000-8000-000000000003";
    // First bridge only to create the DB file; rebuilt below against the mock.
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), "http://placeholder".into(), mk);
    // Pre-seed the post-local-delete state: Trashing row, NO pending op.
    bridge
        .db()
        .upsert_file(&FileEntry {
            file_id: trashing.into(),
            path: "doomed.txt".into(),
            status: FileStatus::Trashing,
            size_bytes: 10,
            modified_at: 0,
            content_hash: None,
            remote_updated_at: 10,
            parent_id: None,
            item_kind: ItemKind::File,
        })
        .unwrap();

    // Snapshot STILL lists the trashed file (propagation lag) — exactly the
    // race window. (is_trashed flag on the node is irrelevant: the lagged
    // snapshot is what the engine sees.)
    let snapshot = serde_json::json!({
        "seq_id": 9,
        "nodes": [ snap_node(&mk, trashing, "doomed.txt", None, false, 10) ],
    });
    let server = SyncMockServer::start(vec![("200 OK".into(), snapshot)]);
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), mk);

    sync_tick(&bridge, dir.path()).await.unwrap();
    server.finish();

    let row = bridge.db().get_file(trashing).unwrap();
    assert!(
        row.is_some(),
        "Trashing row must NOT be pruned while still listed in the snapshot"
    );
    assert_eq!(
        row.unwrap().status,
        FileStatus::Trashing,
        "a snapshot still listing a Trashing file must NOT resurrect it to CloudOnly (the bug)"
    );
}

#[tokio::test]
async fn test_sync_tick_snapshot_absent_keeps_trashing_row_trash_view() {
    // task 0802 — FLIPPED BY TASK 1698 (trash ruling, full sync): once the
    // trash propagates the file is ABSENT from the snapshot — and it STAYS
    // absent forever, because the snapshot never lists trashed files. The
    // op-less `Trashing` row is now the macOS TRASH VIEW: prune_absent
    // must keep it. Removal converges through the server's `file_delete`
    // (permanent) / `file_restore` ops instead (see the 1698 tests).
    let dir = tempfile::tempdir().unwrap();
    let mk = [7u8; 32];
    let trashing = "44444444-0000-4000-8000-000000000004";
    let kept = "55555555-0000-4000-8000-000000000005";
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), "http://placeholder".into(), mk);
    bridge
        .db()
        .upsert_file(&FileEntry {
            file_id: trashing.into(),
            path: "doomed.txt".into(),
            status: FileStatus::Trashing,
            size_bytes: 10,
            modified_at: 0,
            content_hash: None,
            remote_updated_at: 10,
            parent_id: None,
            item_kind: ItemKind::File,
        })
        .unwrap();

    // Snapshot no longer lists the trashed file (propagation complete); it
    // lists an unrelated row so the empty-snapshot guard doesn't trip.
    let snapshot = serde_json::json!({
        "seq_id": 9,
        "nodes": [ snap_node(&mk, kept, "kept.txt", None, false, 10) ],
    });
    let server = SyncMockServer::start(vec![("200 OK".into(), snapshot)]);
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), mk);

    sync_tick(&bridge, dir.path()).await.unwrap();
    server.finish();

    assert!(bridge.db().get_file(kept).unwrap().is_some());
    let row = bridge.db().get_file(trashing).unwrap().unwrap();
    assert_eq!(
        row.status,
        FileStatus::Trashing,
        "an op-less Trashing row absent from the snapshot is the trash view — it survives"
    );
}

#[tokio::test]
async fn test_sync_tick_ops_sequence_converges_to_fresh_snapshot() {
    // Bootstrap, then apply a stream of ops, and assert the resulting mirror
    // matches what a fresh snapshot of the post-op tree would produce.
    let dir = tempfile::tempdir().unwrap();
    let mk = [9u8; 32];
    let keep = "aaaa0000-0000-4000-8000-000000000001";
    let doomed = "bbbb0000-0000-4000-8000-000000000002";
    let created = "cccc0000-0000-4000-8000-000000000003";

    // Bootstrap snapshot: {keep, doomed}, seq 1.
    let boot = serde_json::json!({
        "seq_id": 1,
        "nodes": [
            snap_node(&mk, keep, "keep.txt", None, false, 10),
            snap_node(&mk, doomed, "doomed.txt", None, false, 10),
        ],
    });
    // Ops: create `created`, trash `doomed`. Highest seq = 3.
    let ops = serde_json::json!({
        "since": 1,
        "ops": [
            { "seq_id": 2, "op_type": "file_create",
              "payload": { "id": created, "name_encrypted": enc_name(&mk, created, "fresh.txt"),
                           "parent_id": serde_json::Value::Null, "size_bytes": 7,
                           "storage_pool_id": "pool" } },
            { "seq_id": 3, "op_type": "file_trash", "payload": { "id": doomed } },
        ],
    });

    let server = SyncMockServer::start(vec![("200 OK".into(), boot), ("200 OK".into(), ops)]);
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), mk);

    // Tick 1: bootstrap (cursor 0 → snapshot). Tick 2: ops?since=1.
    sync_tick(&bridge, dir.path()).await.unwrap();
    assert_eq!(bridge.db().get_sync_cursor().unwrap(), Some(1));
    sync_tick(&bridge, dir.path()).await.unwrap();

    let requests = server.finish();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].path, "/api/v1/sync/snapshot");
    assert_eq!(requests[1].path, "/api/v1/sync/ops?since=1");

    // Convergence: keep + created present, doomed is in the TRASH VIEW
    // (task 1698: the trash mirrors the server trash — the row survives
    // as `Trashing`, not deleted), cursor at 3.
    assert!(bridge.db().get_file(keep).unwrap().is_some());
    assert!(bridge.db().get_file(created).unwrap().is_some(), "file_create applied");
    assert_eq!(bridge.db().get_file(created).unwrap().unwrap().path, "fresh.txt");
    assert_eq!(
        bridge.db().get_file(doomed).unwrap().unwrap().status,
        FileStatus::Trashing,
        "file_trash put the row in the trash view (Trashing), it was NOT deleted"
    );
    assert_eq!(
        bridge.db().get_sync_cursor().unwrap(),
        Some(3),
        "cursor advanced to max seq_id"
    );
}

/// Flow-7 P0 regression: a desktop/web version upload emits `file_rename`
/// (name re-encrypt) + `file_update` together, and both land in ONE
/// `/sync/ops` response. `synthesize_op_row` used to stamp both rows with
/// the same wall-clock second, so on a hydrated (`Local`) row the rename
/// bumped `remote_updated_at` to `now` and the `file_update` with the very
/// same `now` hit the `remote_updated <= remote_updated_at` short-circuit —
/// then the cursor advanced past it. The device kept v1 forever while
/// claiming to be in sync.
async fn assert_rename_then_update_in_one_tick_applies_update(new_name: &str) {
    let dir = tempfile::tempdir().unwrap();
    let mk = [21u8; 32];
    let id = "dddd0000-0000-4000-8000-000000000001";
    let server_ops = serde_json::json!({
        "since": 5,
        "ops": [
            { "seq_id": 6, "op_type": "file_rename",
              "payload": { "id": id, "new_name_encrypted": enc_name(&mk, id, new_name) } },
            { "seq_id": 7, "op_type": "file_update",
              "payload": { "id": id, "name_encrypted": enc_name(&mk, id, new_name),
                           "parent_id": serde_json::Value::Null, "size_bytes": 32,
                           "storage_pool_id": "pool",
                           "current_object_version_id": "ov-2",
                           "version_number": 2 } },
        ],
    });
    let server = SyncMockServer::start(vec![("200 OK".into(), server_ops)]);
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), mk);

    // A bootstrapped device that has report.txt v1 (8 B) hydrated locally.
    bridge.db().set_sync_cursor(5).unwrap();
    bridge
        .db()
        .upsert_file(&FileEntry {
            file_id: id.into(),
            path: "report.txt".into(),
            status: FileStatus::Local,
            size_bytes: 8,
            modified_at: 100,
            content_hash: None,
            remote_updated_at: 100,
            parent_id: None,
            item_kind: ItemKind::File,
        })
        .unwrap();
    let mut contract = bridge.db().get_file_contract_state(id).unwrap().unwrap();
    contract.current_version = 1;
    contract.local_base_version = 1;
    contract.current_object_version_id = Some("ov-1".into());
    bridge.db().set_file_contract_state(&contract).unwrap();

    let conflicts = sync_tick(&bridge, dir.path()).await.unwrap();
    assert!(conflicts.is_empty());
    let requests = server.finish();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].path, "/api/v1/sync/ops?since=5");

    let entry = bridge.db().get_file(id).unwrap().unwrap();
    let contract = bridge.db().get_file_contract_state(id).unwrap().unwrap();
    assert_eq!(entry.path, new_name, "rename applied");
    assert_eq!(entry.size_bytes, 32, "file_update applied: size is v2's, not v1's 8 B");
    assert_eq!(contract.current_version, 2, "file_update applied: current_version is 2");
    assert_eq!(contract.current_object_version_id.as_deref(), Some("ov-2"));
    // The cached bytes are still v1: the row must read as stale (remote
    // version ahead of the local base) so the next open re-hydrates.
    assert!(
        contract.current_version > contract.local_base_version,
        "local copy marked stale: current_version {} must exceed local_base_version {}",
        contract.current_version,
        contract.local_base_version
    );
    assert_eq!(bridge.db().get_sync_cursor().unwrap(), Some(7));
}

#[tokio::test]
async fn test_sync_tick_rename_then_update_same_tick_applies_update() {
    assert_rename_then_update_in_one_tick_applies_update("report-final.txt").await;
}

#[tokio::test]
async fn test_sync_tick_name_reencrypt_then_update_same_tick_applies_update() {
    // The FLOW7_NO_RENAME shape: a plain content edit still emits a
    // same-name `file_rename` (name re-encrypt) before the `file_update`.
    assert_rename_then_update_in_one_tick_applies_update("report.txt").await;
}

#[tokio::test]
async fn test_sync_tick_ops_429_backs_off_and_preserves_cursor() {
    // A 429 on /sync/ops must NOT advance the cursor and must NOT lose data —
    // send_with_retry rides it out; if it ultimately surfaces, the tick
    // simply retries next time. Here the mock 429s every attempt; sync_tick
    // must return Ok with the cursor unchanged.
    let dir = tempfile::tempdir().unwrap();
    let mk = [3u8; 32];
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), "http://placeholder".into(), mk);
    // Pre-set a cursor so we take the ops path (not bootstrap).
    bridge.db().set_sync_cursor(8).unwrap();

    // send_with_retry does up to 1 + MAX_429_RETRIES (=3) attempts → 4 total.
    let err_body = serde_json::json!({ "error": "rate limit exceeded", "retry_after": 0 });
    let responses = vec![
        ("429 Too Many Requests".into(), err_body.clone()),
        ("429 Too Many Requests".into(), err_body.clone()),
        ("429 Too Many Requests".into(), err_body.clone()),
        ("429 Too Many Requests".into(), err_body),
    ];
    let server = SyncMockServer::start(responses);
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), mk);

    // Must not panic / error out the runner; cursor stays put for a retry.
    let conflicts = sync_tick(&bridge, dir.path()).await.unwrap();
    assert!(conflicts.is_empty());

    let requests = server.finish();
    assert_eq!(requests.len(), 4, "one initial + 3 retries (429 backoff)");
    for r in &requests {
        assert_eq!(r.path, "/api/v1/sync/ops?since=8");
    }
    assert_eq!(
        bridge.db().get_sync_cursor().unwrap(),
        Some(8),
        "cursor preserved across 429"
    );
}

/// Spec D11 (task 1888, review P1): a tick reaches the server only when its pull came back. A `/sync/ops` that
/// fails is swallowed (the tick is still `Ok`), so the outcome says so, and the reconcile pass and the resume wait.
#[tokio::test]
async fn a_tick_reaches_the_server_only_when_its_pull_came_back() {
    let dir = tempfile::tempdir().unwrap();
    let mk = [3u8; 32];
    let only = "eeee0000-0000-4000-8000-000000000001";
    let db_path = dir.path().join("state.db");

    // Cursor unset: the snapshot bootstrap came back.
    let boot = serde_json::json!({ "seq_id": 4, "nodes": [ snap_node(&mk, only, "only.txt", None, false, 10) ] });
    let server = SyncMockServer::start(vec![("200 OK".into(), boot)]);
    let bridge = test_bridge_with_api(&db_path, server.base_url.clone(), mk);
    let tick = sync_tick_outcome(&bridge, dir.path()).await.unwrap();
    server.finish();
    assert!(tick.reached_server, "a snapshot bootstrap reached the server");

    // Cursor set, the session refused: `/sync/ops` answers 401 and the tick is still Ok.
    let refused = serde_json::json!({ "error": "unauthorized" });
    let server = SyncMockServer::start(vec![("401 Unauthorized".into(), refused)]);
    let bridge = test_bridge_with_api(&db_path, server.base_url.clone(), mk);
    let tick = sync_tick_outcome(&bridge, dir.path()).await.unwrap();
    assert_eq!(server.finish().len(), 1);
    assert!(!tick.reached_server, "a refused pull did not reach the server");

    // A server error: the same.
    let failed = serde_json::json!({ "error": "unavailable" });
    let server = SyncMockServer::start(vec![("503 Service Unavailable".into(), failed)]);
    let bridge = test_bridge_with_api(&db_path, server.base_url.clone(), mk);
    assert!(
        !sync_tick_outcome(&bridge, dir.path()).await.unwrap().reached_server,
        "a 5xx pull"
    );
    server.finish();
    let failed = serde_json::json!({ "error": "unavailable" });

    // An empty delta came back (its freshness probe failing does not change that).
    let empty = serde_json::json!({ "since": 4, "ops": [] });
    let server = SyncMockServer::start(vec![
        ("200 OK".into(), empty),
        ("503 Service Unavailable".into(), failed),
    ]);
    let bridge = test_bridge_with_api(&db_path, server.base_url.clone(), mk);
    let tick = sync_tick_outcome(&bridge, dir.path()).await.unwrap();
    server.finish();
    assert!(tick.reached_server, "the delta came back, empty");

    // A delta with ops came back.
    let ops = serde_json::json!({ "since": 4, "ops": [ {
            "seq_id": 5, "op_type": "file_trash", "file_id": only, "payload": { "id": only }, "created_at": 20,
        } ] });
    let server = SyncMockServer::start(vec![("200 OK".into(), ops)]);
    let bridge = test_bridge_with_api(&db_path, server.base_url.clone(), mk);
    let tick = sync_tick_outcome(&bridge, dir.path()).await.unwrap();
    server.finish();
    assert!(tick.reached_server, "the delta came back");
    assert_eq!(bridge.db().get_sync_cursor().unwrap(), Some(5), "and was applied");
}

#[tokio::test]
async fn test_sync_tick_seq_id_zero_bootstrap_then_takes_ops_path_not_resnapshot() {
    // BLOCKER regression (issue 1): the server returns seq_id 0 for any vault
    // whose `sync_ops` log is empty (MAX(seq_id) → NULL → unwrap_or(0)). seq_id
    // 0 is a VALID bootstrapped cursor, NOT an "unset" sentinel. The 2nd tick
    // MUST issue /sync/ops?since=0 and MUST NOT re-snapshot.
    let dir = tempfile::tempdir().unwrap();
    let mk = [5u8; 32];
    let only = "dddd0000-0000-4000-8000-000000000001";

    // Bootstrap snapshot with seq_id 0 (empty op log) and one node.
    let boot = serde_json::json!({
        "seq_id": 0,
        "nodes": [ snap_node(&mk, only, "only.txt", None, false, 10) ],
    });
    // Empty ops list at since=0 (nothing happened yet). Because the ops list
    // is empty, the freshness-probe snapshot also fires; it returns seq_id 0,
    // which is NOT < cursor(0), so it does NOT re-bootstrap.
    let ops_empty = serde_json::json!({ "since": 0, "ops": [] });
    let probe = serde_json::json!({ "seq_id": 0, "nodes": [
            snap_node(&mk, only, "only.txt", None, false, 10)
        ] });

    let server = SyncMockServer::start(vec![
        ("200 OK".into(), boot),
        ("200 OK".into(), ops_empty),
        ("200 OK".into(), probe),
    ]);
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), mk);

    // Tick 1: cursor UNSET → bootstrap snapshot, stores cursor Some(0).
    sync_tick(&bridge, dir.path()).await.unwrap();
    assert_eq!(
        bridge.db().get_sync_cursor().unwrap(),
        Some(0),
        "bootstrap with seq_id 0 stores a real Some(0) cursor"
    );

    // Tick 2: cursor Some(0) → MUST take the ops path (since=0), NOT re-bootstrap.
    sync_tick(&bridge, dir.path()).await.unwrap();

    let requests = server.finish();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[0].path, "/api/v1/sync/snapshot", "tick1 = bootstrap");
    assert_eq!(
        requests[1].path, "/api/v1/sync/ops?since=0",
        "tick2 takes the ops delta path (NOT a re-bootstrap snapshot)"
    );
    // The 3rd request is the empty-delta freshness probe, not a re-bootstrap.
    assert_eq!(requests[2].path, "/api/v1/sync/snapshot");
    // The row from bootstrap survives (no spurious re-prune).
    assert!(bridge.db().get_file(only).unwrap().is_some());
}

#[tokio::test]
async fn test_sync_tick_nested_create_and_move_match_fresh_snapshot_path() {
    // HIGH regression (issue 2): a nested file_create and a move-into-folder
    // applied via ops must land at the SAME mirror path a fresh snapshot would
    // produce — under the parent's prefix, not at the sync root.
    let dir = tempfile::tempdir().unwrap();
    let mk = [6u8; 32];
    let folder = "f1110000-0000-4000-8000-000000000001";
    let nested = "f1110000-0000-4000-8000-000000000002";
    let mover = "f1110000-0000-4000-8000-000000000003";

    // Bootstrap: just the folder (parents-known-first), seq 1.
    let boot = serde_json::json!({
        "seq_id": 1,
        "nodes": [ snap_node(&mk, folder, "docs", None, true, 10) ],
    });
    // Ops: create a NESTED file under `folder`; create a root file then move
    // it INTO `folder`. Highest seq = 4.
    let ops = serde_json::json!({
        "since": 1,
        "ops": [
            { "seq_id": 2, "op_type": "file_create",
              "payload": { "id": nested, "name_encrypted": enc_name(&mk, nested, "notes.txt"),
                           "parent_id": folder, "size_bytes": 3, "storage_pool_id": "pool" } },
            { "seq_id": 3, "op_type": "file_create",
              "payload": { "id": mover, "name_encrypted": enc_name(&mk, mover, "moved.txt"),
                           "parent_id": serde_json::Value::Null, "size_bytes": 3,
                           "storage_pool_id": "pool" } },
            { "seq_id": 4, "op_type": "file_move",
              "payload": { "id": mover, "old_parent_id": serde_json::Value::Null,
                           "new_parent_id": folder } },
        ],
    });

    let server = SyncMockServer::start(vec![("200 OK".into(), boot), ("200 OK".into(), ops)]);
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), mk);

    sync_tick(&bridge, dir.path()).await.unwrap(); // bootstrap
    sync_tick(&bridge, dir.path()).await.unwrap(); // ops
    server.finish();

    // Both items nested under the folder prefix, exactly as a fresh snapshot
    // (which resolves nesting via parent_id) would place them.
    assert_eq!(
        bridge.db().get_file(nested).unwrap().unwrap().path,
        "docs/notes.txt",
        "nested file_create lands under the parent prefix, not at root"
    );
    assert_eq!(
        bridge.db().get_file(mover).unwrap().unwrap().path,
        "docs/moved.txt",
        "move-into-folder lands under the new parent prefix, not at root"
    );
}

#[tokio::test]
async fn test_sync_tick_trash_then_restore_keeps_row_via_resnapshot() {
    // HIGH regression (issue 3/7): a trash op deletes the local row; the
    // following restore op ({id}-only payload) cannot rebuild it, so it must
    // schedule a re-snapshot that re-materialises the row on the next tick.
    let dir = tempfile::tempdir().unwrap();
    let mk = [8u8; 32];
    let file = "e1110000-0000-4000-8000-000000000001";

    // Bootstrap: the file exists, seq 1.
    let boot = serde_json::json!({
        "seq_id": 1,
        "nodes": [ snap_node(&mk, file, "doc.txt", None, false, 10) ],
    });
    // Ops: trash then restore. Highest seq = 3.
    let ops = serde_json::json!({
        "since": 1,
        "ops": [
            { "seq_id": 2, "op_type": "file_trash", "payload": { "id": file } },
            { "seq_id": 3, "op_type": "file_restore", "payload": { "id": file } },
        ],
    });
    // The restore scheduled a re-snapshot, so the NEXT tick bootstraps from
    // this fresh snapshot (which includes the restored, no-longer-trashed row).
    let resnap = serde_json::json!({
        "seq_id": 3,
        "nodes": [ snap_node(&mk, file, "doc.txt", None, false, 10) ],
    });

    let server = SyncMockServer::start(vec![
        ("200 OK".into(), boot),
        ("200 OK".into(), ops),
        ("200 OK".into(), resnap),
    ]);
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), mk);

    sync_tick(&bridge, dir.path()).await.unwrap(); // bootstrap
    sync_tick(&bridge, dir.path()).await.unwrap(); // ops: trash + restore
    // Task 1698: the trash put the row in the TRASH VIEW (`Trashing`) and
    // the restore flipped it straight back out (`CloudOnly`) — the row
    // never disappeared; a re-snapshot is still pending to refresh its
    // authoritative metadata.
    assert!(
        bridge.db().get_file(file).unwrap().is_some(),
        "the trash-then-restore round trip keeps the row (1698 trash view)"
    );
    assert_eq!(
        bridge.db().get_file(file).unwrap().unwrap().status,
        FileStatus::CloudOnly,
        "restore un-trashes the row"
    );
    sync_tick(&bridge, dir.path()).await.unwrap(); // re-snapshot re-materialises the row

    let requests = server.finish();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[2].path, "/api/v1/sync/snapshot", "tick3 = forced re-snapshot");
    assert!(
        bridge.db().get_file(file).unwrap().is_some(),
        "re-snapshot re-materialised the restored row — it is NOT permanently invisible"
    );
}

#[tokio::test]
async fn test_sync_tick_empty_snapshot_does_not_prune_existing_tree() {
    // BLOCKER regression (issue 4): a degraded EMPTY snapshot (200 with no
    // nodes) must NOT prune the entire local own-tree.
    let dir = tempfile::tempdir().unwrap();
    let mk = [2u8; 32];
    let kept = "c2220000-0000-4000-8000-000000000001";

    // Pre-seed a settled own-tree row stamped in the past (so the freshness
    // cutoff would otherwise allow pruning it).
    {
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), "http://placeholder".into(), mk);
        bridge
            .db()
            .upsert_file(&FileEntry {
                file_id: kept.into(),
                path: "important.txt".into(),
                status: FileStatus::CloudOnly,
                size_bytes: 1,
                modified_at: 0,
                content_hash: None,
                remote_updated_at: 0,
                parent_id: None,
                item_kind: ItemKind::File,
            })
            .unwrap();
    }

    // Bootstrap with an EMPTY snapshot.
    let boot = serde_json::json!({ "seq_id": 7, "nodes": [] });
    let server = SyncMockServer::start(vec![("200 OK".into(), boot)]);
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), mk);

    sync_tick(&bridge, dir.path()).await.unwrap();
    server.finish();

    assert!(
        bridge.db().get_file(kept).unwrap().is_some(),
        "empty snapshot must NOT prune the existing own-tree (fail-closed)"
    );
}

// ── Task 1247: hydrate destination allow-list + real IPC entry point ──────
