//! `versions_3` engine tests, split out of `engine_bridge.rs`.

#[cfg(unix)]
use super::*;
#[cfg(unix)] // names `arm_builder_seam`
#[tokio::test]
async fn the_predicate_reads_row_and_queue_together() {
    let dir = tempfile::tempdir().unwrap();
    let sync_root = dir.path().join("sync-root");
    std::fs::create_dir_all(&sync_root).unwrap();
    let master_key = [63u8; 32];
    let server = VersionedServerMock::start();
    let bridge = Arc::new(test_bridge_with_api(
        &dir.path().join("state.db"),
        server.base_url.clone(),
        master_key,
    ));
    seed_uploaded_row(&bridge, &server, "aca4fc59-8292-5c6a-a5e4-634087eddba6");
    let w = fp_save(
        &bridge,
        dir.path(),
        "aca4fc59-8292-5c6a-a5e4-634087eddba6",
        "notes.txt",
        b"W",
        "1",
    )
    .token
    .unwrap();
    let runner = Arc::clone(&bridge);
    let root = sync_root.clone();
    crate::ipc_socket::arm_builder_seam(move || {
        run_competing(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(async {
                    runner.process_due_operations(&root, now_secs()).await.unwrap();
                });
        });
    });
    assert_eq!(
        held_content_version(&bridge, "aca4fc59-8292-5c6a-a5e4-634087eddba6"),
        w,
        "never the old {{cv}} while W lands"
    );
    let state = server.finish();
    assert_eq!(
        state.init_summary(),
        vec![(json!("aca4fc59-8292-5c6a-a5e4-634087eddba6"), json!(1), 201)],
        "W landed inside the seam"
    );
}

#[cfg(unix)] // asserts a token through `held_content_version`
#[tokio::test]
async fn twenty_rapid_saves_land_in_order_and_the_last_wins() {
    // Review Focus 1
    let dir = tempfile::tempdir().unwrap();
    let sync_root = dir.path().join("sync-root");
    std::fs::create_dir_all(&sync_root).unwrap();
    let master_key = [64u8; 32];
    let server = VersionedServerMock::start();
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
    server
        .state
        .lock()
        .unwrap()
        .delay_chunks
        .insert("session-1".into(), Duration::from_millis(400));
    seed_uploaded_row(&bridge, &server, "ded8e3ab-0bf7-5db9-9426-808ead3daa73");
    let first = fp_save(
        &bridge,
        dir.path(),
        "ded8e3ab-0bf7-5db9-9426-808ead3daa73",
        "notes.txt",
        b"save 1",
        "1",
    );
    let mut base = first.token.unwrap();
    let mut last = b"save 1".to_vec();
    let ((), ()) = tokio::join!(
        async {
            drain_upload_queue(&bridge, &sync_root).await;
        },
        async {
            tokio::time::timeout(Duration::from_secs(10), async {
                while server.state.lock().unwrap().inits.is_empty() {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            })
            .await
            .expect("init never reached the mock");
            for n in 2..=20 {
                let bytes = format!("save {n}").into_bytes();
                let reply = fp_save(
                    &bridge,
                    dir.path(),
                    "ded8e3ab-0bf7-5db9-9426-808ead3daa73",
                    "notes.txt",
                    &bytes,
                    &base,
                );
                base = reply.token.unwrap();
                assert_eq!(
                    held_content_version(&bridge, "ded8e3ab-0bf7-5db9-9426-808ead3daa73"),
                    base,
                    "save {n}: one name for its bytes"
                );
                last = bytes;
            }
        }
    );
    drain_upload_queue(&bridge, &sync_root).await;
    let state = server.finish();
    let bases: Vec<i64> = state
        .inits
        .iter()
        .map(|(body, status)| {
            assert_eq!(*status, 201, "{:?}", state.init_summary());
            body["base_version_number"].as_i64().unwrap()
        })
        .collect();
    assert_eq!(bases, (1..=20).collect::<Vec<i64>>(), "one upload per save, in order");
    assert_eq!(
        state.latest_plaintext("ded8e3ab-0bf7-5db9-9426-808ead3daa73", master_key),
        last,
        "the last save wins"
    );
    assert!(bridge.db.list_due_operations(i64::MAX).unwrap().is_empty());
    assert_eq!(
        held_content_version(&bridge, "ded8e3ab-0bf7-5db9-9426-808ead3daa73"),
        base,
        "the last save's name stays after it lands"
    );
}

/// Spec §8.4, the old folders: a save staged before the move names its copy by an absolute path in the old
/// cache root (the op and the journal). It still uploads from that copy, and the copy is released and
/// unlinked after the landing, with its journal row, as before.
#[cfg(target_os = "macos")]
#[tokio::test]
async fn a_save_staged_in_the_old_cache_root_still_uploads_and_its_copy_is_released() {
    let dir = tempfile::tempdir().unwrap();
    let sync_root = dir.path().join("sync-root");
    std::fs::create_dir_all(&sync_root).unwrap();
    let master_key = [91u8; 32];
    let server = VersionedServerMock::start();
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
    seed_uploaded_row(&bridge, &server, "da61c658-d6d9-578a-931c-a3aaf1ea7388");
    fp_save(
        &bridge,
        dir.path(),
        "da61c658-d6d9-578a-931c-a3aaf1ea7388",
        "notes.txt",
        b"saved before the move",
        "1",
    );
    let ops = bridge.db.list_due_operations(i64::MAX).unwrap();
    assert_eq!(ops.len(), 1);
    let staged = PathBuf::from(ops[0].payload_path.clone().expect("a save carries its copy"));
    let bases = FinderStagingBases::current();
    assert!(
        staged.starts_with(bases.durable_root().unwrap()),
        "a new save stages in the data root: {staged:?}"
    );
    // Where a build before the move staged it: the old cache root.
    let old_root = bases.cache_root();
    std::fs::create_dir_all(&old_root).unwrap();
    let old_copy = old_root.join(uuid::Uuid::new_v4().to_string());
    std::fs::rename(&staged, &old_copy).unwrap();
    {
        let conn = bridge.db.hold_lock_for_test();
        let (old, new) = (
            old_copy.to_string_lossy().into_owned(),
            staged.to_string_lossy().into_owned(),
        );
        for sql in [
            "UPDATE operation_queue SET payload_path = ?1 WHERE payload_path = ?2",
            "UPDATE staged_payloads SET path = ?1 WHERE path = ?2",
        ] {
            assert_eq!(conn.execute(sql, rusqlite::params![old, new]).unwrap(), 1, "{sql}");
        }
    }

    drain_upload_queue(&bridge, &sync_root).await;

    let state = server.finish();
    assert_eq!(
        state.init_summary(),
        vec![(json!("da61c658-d6d9-578a-931c-a3aaf1ea7388"), json!(1), 201)],
        "the save uploads once, on its base"
    );
    assert_eq!(
        state.latest_plaintext("da61c658-d6d9-578a-931c-a3aaf1ea7388", master_key),
        b"saved before the move",
        "from the copy in the old root"
    );
    assert!(bridge.db.list_due_operations(i64::MAX).unwrap().is_empty());
    assert!(
        !old_copy.exists(),
        "the copy in the old root is unlinked after the landing"
    );
    let journalled: i64 = bridge
        .db
        .hold_lock_for_test()
        .query_row(
            "SELECT COUNT(*) FROM staged_payloads WHERE path = ?1",
            [old_copy.to_string_lossy()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(journalled, 0, "and its journal row is gone");
}

// ── Rule 2, the snapshot side: version 0 and versionless replaces (spec §6.3.2–§6.3.4) ──

/// A row this Mac learned from a legacy `file_create` op: at version 0 (FILES:2286-2297).
/// (Both helpers carry `#[cfg(target_os = "macos")]`: only the snapshot-side tests call them.)
#[cfg(target_os = "macos")]
pub(super) fn seed_version_zero_row(
    bridge: &EngineBridge,
    server: &VersionedServerMock,
    file_id: &str,
    status: FileStatus,
) {
    seed_uploaded_row(bridge, server, file_id);
    let mut contract = bridge.db.get_file_contract_state(file_id).unwrap().unwrap();
    contract.current_version = 0;
    bridge.db.set_file_contract_state(&contract).unwrap();
    bridge.db.set_status(file_id, status).unwrap();
}

/// `name` is the file's display name: the log-line tests pass `PLANTED` here, so a line
/// that logged the name would leak it. Every other caller passes `"notes.txt"`.
#[cfg(target_os = "macos")]
pub(super) fn node(master_key: &[u8; 32], id: &str, name: &str, version: i64, is_uploading: bool) -> serde_json::Value {
    let mut node = snap_node(master_key, id, name, None, false, 0);
    node["version_number"] = serde_json::json!(version);
    node["is_uploading"] = serde_json::json!(is_uploading);
    node
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn i2_a_row_without_a_version_never_uploads_without_a_base() {
    let dir = tempfile::tempdir().unwrap();
    let sync_root = dir.path().join("sync-root");
    std::fs::create_dir_all(&sync_root).unwrap();
    let master_key = [72u8; 32];
    let server = VersionedServerMock::start();
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
    seed_version_zero_row(
        &bridge,
        &server,
        "e1b3818d-a25f-5dbf-9781-abd804401fff",
        FileStatus::Local,
    );
    bridge.db.set_sync_cursor(0).unwrap();
    fp_save(
        &bridge,
        dir.path(),
        "e1b3818d-a25f-5dbf-9781-abd804401fff",
        "notes.txt",
        b"edit",
        "0",
    );
    drain_upload_queue(&bridge, &sync_root).await;
    assert!(
        server.state.lock().unwrap().inits.is_empty(),
        "no init until the version is known"
    );
    server.state.lock().unwrap().snapshots.push_back((
            "200 OK".into(),
            serde_json::json!({ "seq_id": 1, "nodes": [node(&master_key, "e1b3818d-a25f-5dbf-9781-abd804401fff", "notes.txt", 1, false)] }),
        ));
    let logs = capture_logs_async(async {
        sync_tick_outcome(&bridge, &sync_root).await.unwrap();
        drain_upload_queue(&bridge, &sync_root).await;
    })
    .await;
    let state = server.finish();
    assert_eq!(
        state.init_summary(),
        vec![(json!("e1b3818d-a25f-5dbf-9781-abd804401fff"), json!(1), 201)]
    );
    assert!(
        logs.contains("queued write based on the version the snapshot reported"),
        "{logs}"
    );
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn a_versionless_create_op_requests_a_snapshot_that_fills_local_rows() {
    let dir = tempfile::tempdir().unwrap();
    let sync_root = dir.path().join("sync-root");
    std::fs::create_dir_all(&sync_root).unwrap();
    let master_key = [73u8; 32];
    let server = VersionedServerMock::start();
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
    let create = crate::api_client::SyncOp {
        seq_id: 5,
        op_type: "file_create".into(),
        payload: serde_json::json!({
            "id": "b1000000-0000-4000-8000-000000000005",
            "name_encrypted": enc_name(&master_key, "b1000000-0000-4000-8000-000000000005", "notes.txt"),
            "parent_id": null,
            "size_bytes": 10
        }),
    };
    apply_sync_op(&bridge, &sync_root, &create, now_secs(), &mut Vec::new()).unwrap();
    assert!(
        bridge.db.peek_resnapshot_request().unwrap().is_some(),
        "a versionless op asks for a snapshot"
    );
    bridge
        .db
        .set_status("b1000000-0000-4000-8000-000000000005", FileStatus::Local)
        .unwrap();
    let snapshot = crate::api_client::SyncSnapshot {
        seq_id: 6,
        nodes: vec![node(
            &master_key,
            "b1000000-0000-4000-8000-000000000005",
            "notes.txt",
            1,
            false,
        )],
    };
    apply_snapshot(&bridge, &sync_root, &snapshot, now_secs(), now_secs(), &mut Vec::new()).unwrap();
    let presentation = bridge
        .db
        .item_presentation("b1000000-0000-4000-8000-000000000005")
        .unwrap()
        .unwrap();
    assert_eq!(
        presentation.contract.current_version, 1,
        "filled although the row is Local"
    );
    assert!(presentation.version_filled);
    assert_eq!(
        bridge
            .db
            .get_file("b1000000-0000-4000-8000-000000000005")
            .unwrap()
            .unwrap()
            .status,
        FileStatus::Local,
        "content untouched"
    );
    drop(server.finish());
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn i2_a_zero_base_save_after_the_fill_lands_on_the_filled_version() {
    // Rule 6a.
    let dir = tempfile::tempdir().unwrap();
    let sync_root = dir.path().join("sync-root");
    std::fs::create_dir_all(&sync_root).unwrap();
    let master_key = [74u8; 32];
    let server = VersionedServerMock::start();
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
    seed_version_zero_row(
        &bridge,
        &server,
        "9d4a9714-48bd-5116-a268-a83f2cb2ac0e",
        FileStatus::Local,
    );
    let snapshot = crate::api_client::SyncSnapshot {
        seq_id: 2,
        nodes: vec![node(
            &master_key,
            "9d4a9714-48bd-5116-a268-a83f2cb2ac0e",
            "notes.txt",
            1,
            false,
        )],
    };
    apply_snapshot(&bridge, &sync_root, &snapshot, now_secs(), now_secs(), &mut Vec::new()).unwrap();
    fp_save(
        &bridge,
        dir.path(),
        "9d4a9714-48bd-5116-a268-a83f2cb2ac0e",
        "notes.txt",
        b"saved before the re-read",
        "0",
    );
    drain_upload_queue(&bridge, &sync_root).await;
    let state = server.finish();
    assert_eq!(
        state.init_summary(),
        vec![(json!("9d4a9714-48bd-5116-a268-a83f2cb2ac0e"), json!(1), 201)]
    );
    assert!(
        bridge
            .db
            .list_operations_for_file("9d4a9714-48bd-5116-a268-a83f2cb2ac0e")
            .unwrap()
            .is_empty(),
        "nothing parks"
    );
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn i2_a_failed_snapshot_keeps_the_request_and_base_pending_resolves_on_the_next_success() {
    let dir = tempfile::tempdir().unwrap();
    let sync_root = dir.path().join("sync-root");
    std::fs::create_dir_all(&sync_root).unwrap();
    let master_key = [75u8; 32];
    let server = VersionedServerMock::start();
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
    seed_version_zero_row(
        &bridge,
        &server,
        "b57f39c5-e8fa-51ee-9c2a-7cfbf0a06b5b",
        FileStatus::Local,
    );
    bridge.db.set_sync_cursor(0).unwrap();
    fp_save(
        &bridge,
        dir.path(),
        "b57f39c5-e8fa-51ee-9c2a-7cfbf0a06b5b",
        "notes.txt",
        b"edit",
        "0",
    );
    {
        let mut s = server.state.lock().unwrap();
        s.snapshots
            .push_back(("503 Service Unavailable".into(), serde_json::json!({ "error": "busy" })));
        s.snapshots.push_back((
            "200 OK".into(),
            serde_json::json!({
                "seq_id": 3,
                "nodes": [node(&master_key, "b57f39c5-e8fa-51ee-9c2a-7cfbf0a06b5b", "notes.txt", 1, false)]
            }),
        ));
    }
    // The runner runs the queue only after an Ok tick (RUN:1293-1315).
    assert!(
        sync_tick_outcome(&bridge, &sync_root).await.is_err(),
        "the failed snapshot fails the tick"
    );
    assert!(
        bridge.db.peek_resnapshot_request().unwrap().is_some(),
        "the request survives a failed snapshot"
    );
    sync_tick_outcome(&bridge, &sync_root).await.unwrap();
    drain_upload_queue(&bridge, &sync_root).await;
    let state = server.finish();
    assert_eq!(
        state.init_summary(),
        vec![(json!("b57f39c5-e8fa-51ee-9c2a-7cfbf0a06b5b"), json!(1), 201)]
    );
    assert_eq!(bridge.db.peek_resnapshot_request().unwrap(), None);
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn i2_the_fill_reaches_an_uploading_row() {
    let dir = tempfile::tempdir().unwrap();
    let sync_root = dir.path().join("sync-root");
    std::fs::create_dir_all(&sync_root).unwrap();
    let master_key = [76u8; 32];
    let server = VersionedServerMock::start();
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
    seed_version_zero_row(
        &bridge,
        &server,
        "d72e0057-5771-5026-ad96-042bc57d9168",
        FileStatus::Local,
    );
    fp_save(
        &bridge,
        dir.path(),
        "d72e0057-5771-5026-ad96-042bc57d9168",
        "notes.txt",
        b"edit",
        "0",
    );
    assert_eq!(
        bridge
            .db
            .get_file("d72e0057-5771-5026-ad96-042bc57d9168")
            .unwrap()
            .unwrap()
            .status,
        FileStatus::Uploading
    );
    let snapshot = crate::api_client::SyncSnapshot {
        seq_id: 2,
        nodes: vec![node(
            &master_key,
            "d72e0057-5771-5026-ad96-042bc57d9168",
            "notes.txt",
            1,
            false,
        )],
    };
    apply_snapshot(&bridge, &sync_root, &snapshot, now_secs(), now_secs(), &mut Vec::new()).unwrap();
    assert_eq!(
        bridge
            .db
            .get_file_contract_state("d72e0057-5771-5026-ad96-042bc57d9168")
            .unwrap()
            .unwrap()
            .current_version,
        1
    );
    let op = bridge
        .db
        .list_operations_for_file("d72e0057-5771-5026-ad96-042bc57d9168")
        .unwrap()
        .remove(0);
    assert_eq!(op.base_version, Some(1), "the op is based on the filled version");
    assert_eq!(bridge.db.finder_write(&op.op_id).unwrap().unwrap().base_pending, 0);
    drop(server.finish());
}

#[cfg(target_os = "macos")]
#[cfg(unix)] // asserts a token through `held_content_version` and `land_one_save`
#[tokio::test]
async fn i3_a_versionless_replace_from_another_device_reaches_the_disk() {
    let dir = tempfile::tempdir().unwrap();
    let sync_root = dir.path().join("sync-root");
    std::fs::create_dir_all(&sync_root).unwrap();
    let master_key = [77u8; 32];
    let server = VersionedServerMock::start();
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
    let token = land_one_save(
        &bridge,
        &server,
        dir.path(),
        &sync_root,
        "b1000000-0000-4000-8000-000000000003",
    )
    .await;
    let replace = crate::api_client::SyncOp {
        seq_id: 9,
        op_type: "file_create".into(),
        payload: serde_json::json!({
            "id": "b1000000-0000-4000-8000-000000000003",
            "name_encrypted": enc_name(&master_key, "b1000000-0000-4000-8000-000000000003", "notes.txt"),
            "parent_id": null,
            "size_bytes": 12
        }),
    };
    apply_sync_op(&bridge, &sync_root, &replace, now_secs(), &mut Vec::new()).unwrap();
    let seen = bridge.db.peek_resnapshot_request().unwrap().expect("requested");
    assert_eq!(
        held_content_version(&bridge, "b1000000-0000-4000-8000-000000000003"),
        token,
        "the op alone cannot tell a replace"
    );
    // The legacy init bumped the version before the bytes exist: skipped, the request stays.
    let mid = crate::api_client::SyncSnapshot {
        seq_id: 10,
        nodes: vec![node(
            &master_key,
            "b1000000-0000-4000-8000-000000000003",
            "notes.txt",
            3,
            true,
        )],
    };
    apply_snapshot(&bridge, &sync_root, &mid, now_secs(), now_secs(), &mut Vec::new()).unwrap();
    assert_eq!(
        held_content_version(&bridge, "b1000000-0000-4000-8000-000000000003"),
        token
    );
    assert!(
        !bridge.db.clear_resnapshot_request(seen).unwrap(),
        "the skip kept the request"
    );
    let done = crate::api_client::SyncSnapshot {
        seq_id: 11,
        nodes: vec![node(
            &master_key,
            "b1000000-0000-4000-8000-000000000003",
            "notes.txt",
            3,
            false,
        )],
    };
    let logs = capture_logs_async(async {
        apply_snapshot(&bridge, &sync_root, &done, now_secs(), now_secs(), &mut Vec::new()).unwrap();
    })
    .await;
    assert_eq!(
        held_content_version(&bridge, "b1000000-0000-4000-8000-000000000003"),
        "3",
        "the system re-downloads the phone's bytes"
    );
    assert!(logs.contains("file version raised from the snapshot"), "{logs}");
    drop(server.finish());
}

/// The phone-replace path of the device run (D8b): the node is newer than the row, so the
/// `Local` row continues past the short-circuit into the metadata refresh after the raise.
/// The raise's facts hold after it.
#[cfg(target_os = "macos")]
#[cfg(unix)] // asserts a token through `held_content_version` and `land_one_save`
#[tokio::test]
async fn i3_a_newer_versionless_replace_reaches_the_disk_through_the_metadata_refresh() {
    let dir = tempfile::tempdir().unwrap();
    let sync_root = dir.path().join("sync-root");
    std::fs::create_dir_all(&sync_root).unwrap();
    let master_key = [82u8; 32];
    let server = VersionedServerMock::start();
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
    let token = land_one_save(
        &bridge,
        &server,
        dir.path(),
        &sync_root,
        "b1000000-0000-4000-8000-000000000003",
    )
    .await;
    let landed = bridge
        .db
        .get_file_contract_state("b1000000-0000-4000-8000-000000000003")
        .unwrap()
        .unwrap();
    assert_eq!((landed.current_version, landed.local_base_version), (2, 2));
    assert!(landed.current_object_version_id.is_some());
    let replace = crate::api_client::SyncOp {
        seq_id: 9,
        op_type: "file_create".into(),
        payload: serde_json::json!({
            "id": "b1000000-0000-4000-8000-000000000003",
            "name_encrypted": enc_name(&master_key, "b1000000-0000-4000-8000-000000000003", "notes.txt"),
            "parent_id": null,
            "size_bytes": 12
        }),
    };
    apply_sync_op(&bridge, &sync_root, &replace, now_secs(), &mut Vec::new()).unwrap();
    assert_eq!(
        held_content_version(&bridge, "b1000000-0000-4000-8000-000000000003"),
        token
    );
    let mut newer = node(
        &master_key,
        "b1000000-0000-4000-8000-000000000003",
        "notes.txt",
        3,
        false,
    );
    newer["updated_at"] = serde_json::json!(now_secs() + 60);
    newer["size_bytes"] = serde_json::json!(12);
    let done = crate::api_client::SyncSnapshot {
        seq_id: 11,
        nodes: vec![newer],
    };
    apply_snapshot(&bridge, &sync_root, &done, now_secs(), now_secs(), &mut Vec::new()).unwrap();
    assert_eq!(
        held_content_version(&bridge, "b1000000-0000-4000-8000-000000000003"),
        "3",
        "the system re-downloads the phone's bytes"
    );
    let contract = bridge
        .db
        .get_file_contract_state("b1000000-0000-4000-8000-000000000003")
        .unwrap()
        .unwrap();
    assert_eq!(contract.current_version, 3);
    assert_eq!(
        contract.local_base_version, 2,
        "the bytes on disk are still version 2's"
    );
    assert_eq!(
        contract.current_object_version_id, None,
        "the snapshot names no object version, and version 2's is not current"
    );
    assert!(
        !bridge
            .db
            .item_presentation("b1000000-0000-4000-8000-000000000003")
            .unwrap()
            .unwrap()
            .version_filled,
        "a raise is not a fill"
    );
    assert_eq!(
        bridge
            .db
            .get_file("b1000000-0000-4000-8000-000000000003")
            .unwrap()
            .unwrap()
            .size_bytes,
        12
    );
    drop(server.finish());
}
