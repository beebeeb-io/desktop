//! `thumbnails_2` engine tests, split out of `engine_bridge.rs`.

use super::*;
/// Review Focus 3: an empty save lands, and a fetch while it is queued is an empty file.
#[tokio::test]
async fn an_empty_save_lands_and_is_served_from_the_queue() {
    let master_key = [94u8; 32];
    let (dir, sync_root, server, bridge) = rule3_setup(master_key);
    seed_uploaded_row(&bridge, &server, "bc87e59f-377e-59f3-ad18-492fe79c2ec5");
    fp_save(
        &bridge,
        dir.path(),
        "bc87e59f-377e-59f3-ad18-492fe79c2ec5",
        "notes.txt",
        b"",
        "1",
    );
    let dest = dir.path().join("fetch").join("notes.txt");
    std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
    assert_eq!(
        bridge
            .serve_hydrate("bc87e59f-377e-59f3-ad18-492fe79c2ec5", &dest, &[dir.path()], None)
            .await
            .unwrap(),
        HydrateSource::Queue
    );
    assert_eq!(
        std::fs::metadata(&dest).unwrap().len(),
        0,
        "an empty file, not an error"
    );
    drain_upload_queue(&bridge, &sync_root).await;
    let state = server.finish();
    assert_eq!(state.files["bc87e59f-377e-59f3-ad18-492fe79c2ec5"].versions.len(), 2);
    assert!(
        state
            .latest_plaintext("bc87e59f-377e-59f3-ad18-492fe79c2ec5", master_key)
            .is_empty()
    );
}

// ── Rule 5: the item's status follows the queue (spec §9.1–§9.2) ──────────

/// T33 (§9.2): a Finder upload that failed once and will retry keeps the row `Uploading`, so
/// the item stays writable and is never evicted.
#[cfg(unix)] // names `crate::ipc_socket::file_entry_payload_for_db`
#[tokio::test]
async fn a_retrying_upload_keeps_the_item_writable() {
    let (dir, sync_root, server, bridge) = rule3_setup([95u8; 32]);
    seed_uploaded_row(&bridge, &server, "ad4f26df-8274-576d-b6bf-9e0f85e94e59");
    fp_save(
        &bridge,
        dir.path(),
        "ad4f26df-8274-576d-b6bf-9e0f85e94e59",
        "notes.txt",
        b"edit",
        "1",
    );
    server
        .state
        .lock()
        .unwrap()
        .fail_first_chunk_once
        .insert("session-1".into());
    bridge.process_due_operations(&sync_root, now_secs()).await.unwrap(); // one transient failure
    let op = bridge
        .db
        .list_operations_for_file("ad4f26df-8274-576d-b6bf-9e0f85e94e59")
        .unwrap()
        .remove(0);
    assert_eq!(op.attempts, 1, "precondition: one failed attempt, to be retried");
    assert_eq!(
        bridge
            .db
            .get_file("ad4f26df-8274-576d-b6bf-9e0f85e94e59")
            .unwrap()
            .unwrap()
            .status,
        FileStatus::Uploading,
        "the row is not rolled back to Error"
    );
    let entry = bridge
        .db
        .get_file("ad4f26df-8274-576d-b6bf-9e0f85e94e59")
        .unwrap()
        .unwrap();
    let payload = crate::ipc_socket::file_entry_payload_for_db(&bridge.db, &entry, "root");
    assert_eq!(payload.status, "uploading");
    assert_ne!(payload.capabilities & crate::ipc_socket::CAP_WRITE, 0, "writable");
    drop(server.finish());
}

/// T60 (§9.1, m-8): a `Trashing` row keeps its trash presentation while its Finder upload is
/// queued: never `uploading`, no write capability, under the trash container.
#[cfg(unix)] // names `crate::ipc_socket::file_entry_payload_for_db`
#[tokio::test]
async fn a_trashing_row_is_never_presented_uploading() {
    let (dir, _sync_root, server, bridge) = rule3_setup([96u8; 32]);
    seed_uploaded_row(&bridge, &server, "faee2ad8-326e-568c-9127-c87604262d4a");
    let w = fp_save(
        &bridge,
        dir.path(),
        "faee2ad8-326e-568c-9127-c87604262d4a",
        "notes.txt",
        b"edit",
        "1",
    );
    bridge
        .queue_finder_delete("faee2ad8-326e-568c-9127-c87604262d4a", w.token)
        .unwrap();
    let entry = bridge
        .db
        .get_file("faee2ad8-326e-568c-9127-c87604262d4a")
        .unwrap()
        .unwrap();
    assert_eq!(entry.status, FileStatus::Trashing, "precondition: in the trash");
    assert!(
        bridge
            .db
            .item_presentation("faee2ad8-326e-568c-9127-c87604262d4a")
            .unwrap()
            .unwrap()
            .unparked_finder_upload,
        "precondition: its save is still queued"
    );
    let payload = crate::ipc_socket::file_entry_payload_for_db(&bridge.db, &entry, "root");
    assert_ne!(payload.status, "uploading");
    assert_eq!(
        payload.capabilities & crate::ipc_socket::CAP_WRITE,
        0,
        "no write capability in the trash"
    );
    assert_eq!(
        payload.parent_identifier,
        crate::ipc_socket::FP_TRASH_APPLE,
        "under the trash container"
    );
    drop(server.finish());
}

/// §9.1 (m-8): a save queued before a Finder trash lands while its row is
/// `Trashing`, before the trash op runs. The landing leaves the row in the trash: never `Local`,
/// so never writable or evictable, and still under the trash container.
#[cfg(unix)] // names `crate::ipc_socket::file_entry_payload_for_db`
#[tokio::test]
async fn a_save_that_lands_under_a_trashing_row_keeps_it_in_the_trash() {
    let master_key = [102u8; 32];
    let (dir, sync_root, server, bridge) = rule3_setup(master_key);
    seed_uploaded_row(&bridge, &server, "867458ce-f714-50f4-9bc4-3e52405092f6");
    let w = fp_save(
        &bridge,
        dir.path(),
        "867458ce-f714-50f4-9bc4-3e52405092f6",
        "notes.txt",
        b"edit",
        "1",
    );
    bridge
        .queue_finder_delete("867458ce-f714-50f4-9bc4-3e52405092f6", w.token)
        .unwrap();
    // The save is listed first, so it lands before the trash op runs. The seam fires once, at
    // the save's outcome: the trash op has not reached the server yet.
    let seen = Arc::new(Mutex::new(None));
    let (db, server_state, seen_in_seam) = (bridge.db.clone(), Arc::clone(&server.state), Arc::clone(&seen));
    bridge.seams.arm("outcome:before_tx", move || {
        let trashes = server_state.lock().unwrap().trashes.len();
        let entry = db.get_file("867458ce-f714-50f4-9bc4-3e52405092f6").unwrap().unwrap();
        let payload = crate::ipc_socket::file_entry_payload_for_db(&db, &entry, "root");
        *seen_in_seam.lock().unwrap() = Some((trashes, entry.status, payload));
    });
    bridge.process_due_operations(&sync_root, now_secs()).await.unwrap();
    let (trashes, status, payload) = seen.lock().unwrap().take().expect("the seam fired");
    assert_eq!(trashes, 0, "precondition: the save's outcome, before the trash op");
    assert_eq!(status, FileStatus::Trashing, "the landing leaves the row in the trash");
    assert!(
        payload.status != "local" && payload.status != "uploading",
        "never presented writable: {}",
        payload.status
    );
    assert_eq!(
        payload.capabilities & crate::ipc_socket::CAP_WRITE,
        0,
        "no write capability"
    );
    assert_eq!(payload.parent_identifier, crate::ipc_socket::FP_TRASH_APPLE);
    assert_eq!(
        bridge
            .db
            .get_file("867458ce-f714-50f4-9bc4-3e52405092f6")
            .unwrap()
            .unwrap()
            .status,
        FileStatus::Trashing,
        "still in the trash after the trash op"
    );
    let state = server.finish();
    assert_eq!(
        state.latest_plaintext("867458ce-f714-50f4-9bc4-3e52405092f6", master_key),
        b"edit",
        "precondition: the save landed"
    );
    assert_eq!(
        state.trashes,
        vec!["867458ce-f714-50f4-9bc4-3e52405092f6".to_string()],
        "precondition: the trash op ran"
    );
}

/// T31 / Review Focus 2: a restart in the middle of a save's upload. Every chunk was
/// acknowledged and the completion was lost when the process died holding its claim. After the
/// relaunch the item is still writable, it keeps the token it had (no re-download), and the
/// upload resumes its session.
#[cfg(unix)] // asserts a token through `held_content_version`
#[tokio::test]
async fn a_restart_mid_upload_resumes_and_keeps_the_token() {
    let master_key = [97u8; 32];
    let (dir, sync_root, server, bridge) = rule3_setup(master_key);
    let db_path = dir.path().join("state.db");
    seed_uploaded_row(&bridge, &server, "3eb48f67-982a-5177-b0d6-d836206ae36e");
    let token = fp_save(
        &bridge,
        dir.path(),
        "3eb48f67-982a-5177-b0d6-d836206ae36e",
        "notes.txt",
        b"saved before the restart",
        "1",
    )
    .token
    .unwrap();
    server
        .state
        .lock()
        .unwrap()
        .fail_complete_once
        .insert("session-1".into());
    bridge.process_due_operations(&sync_root, now_secs()).await.unwrap(); // every chunk acknowledged, no completion
    let op_id = bridge
        .db
        .list_operations_for_file("3eb48f67-982a-5177-b0d6-d836206ae36e")
        .unwrap()
        .remove(0)
        .op_id;
    let resume = bridge
        .db
        .get_upload_resume(&op_id)
        .unwrap()
        .expect("its session is kept");
    assert_eq!(
        resume.acked_chunks, resume.chunk_count,
        "precondition: every chunk acknowledged"
    );
    assert!(resume.completed_version.is_none(), "precondition: no completion");
    let _crashed_runner = bridge.db.claim_operation(&op_id, now_secs()).unwrap(); // the process dies holding a claim
    drop(bridge);

    // Relaunch: a new process opens the same state.db.
    let bridge = test_bridge_with_api(&db_path, server.base_url.clone(), master_key);
    assert_eq!(bridge.db.engine_start_repair().unwrap().claims_cleared, 1);
    bridge.db.reconcile_stale_in_flight_on_startup().unwrap();
    assert_eq!(
        bridge
            .db
            .get_file("3eb48f67-982a-5177-b0d6-d836206ae36e")
            .unwrap()
            .unwrap()
            .status,
        FileStatus::Uploading,
        "still writable"
    );
    assert_eq!(
        held_content_version(&bridge, "3eb48f67-982a-5177-b0d6-d836206ae36e"),
        token,
        "same name: no re-download"
    );
    drain_upload_queue(&bridge, &sync_root).await;
    let state = server.finish();
    assert_eq!(state.inits.len(), 1, "the session resumed: {:?}", state.init_summary());
    assert_eq!(
        state.latest_plaintext("3eb48f67-982a-5177-b0d6-d836206ae36e", master_key),
        b"saved before the restart"
    );
    assert_eq!(
        held_content_version(&bridge, "3eb48f67-982a-5177-b0d6-d836206ae36e"),
        token
    );
}

/// RF5 / Review Focus 5 (§9.1, m-8): a save of a file that was trashed on the web while the
/// save was queued. The server refuses its `init` with "file is in trash", class `other`: the
/// save retries with its bytes kept, and the item in the trash never looks writable.
#[cfg(unix)] // names `crate::ipc_socket::file_entry_payload_for_db`
#[tokio::test]
async fn a_save_to_a_file_trashed_elsewhere_keeps_its_bytes() {
    let (dir, sync_root, server, bridge) = rule3_setup([98u8; 32]);
    seed_uploaded_row(&bridge, &server, "6b80b427-4f4a-5820-9175-957ef1637092");
    fp_save(
        &bridge,
        dir.path(),
        "6b80b427-4f4a-5820-9175-957ef1637092",
        "notes.txt",
        b"an edit the person made",
        "1",
    );
    let trash = crate::api_client::SyncOp {
        seq_id: 7,
        op_type: "file_trash".into(),
        payload: serde_json::json!({ "id": "6b80b427-4f4a-5820-9175-957ef1637092" }),
    };
    apply_sync_op(&bridge, &sync_root, &trash, now_secs(), &mut Vec::new()).unwrap();
    server
        .state
        .lock()
        .unwrap()
        .trashed_files
        .insert("6b80b427-4f4a-5820-9175-957ef1637092".into());
    drain_upload_queue(&bridge, &sync_root).await;
    let op = bridge
        .db
        .list_operations_for_file("6b80b427-4f4a-5820-9175-957ef1637092")
        .unwrap()
        .remove(0);
    assert!(
        op.attempts < op.max_attempts,
        "class other: retried, not parked at once"
    );
    assert_eq!(
        std::fs::read(op.payload_path.as_deref().unwrap()).unwrap(),
        b"an edit the person made",
        "the bytes are kept"
    );
    let entry = bridge
        .db
        .get_file("6b80b427-4f4a-5820-9175-957ef1637092")
        .unwrap()
        .unwrap();
    let payload = crate::ipc_socket::file_entry_payload_for_db(&bridge.db, &entry, "root");
    assert_ne!(payload.status, "uploading", "an item in the trash never looks writable");
    assert_eq!(
        payload.capabilities & crate::ipc_socket::CAP_WRITE,
        0,
        "no write capability in the trash"
    );
    let state = server.finish();
    assert!(
        !state.inits.is_empty() && state.inits.iter().all(|(_, status)| *status == 409),
        "precondition: the server refused every attempt: {:?}",
        state.init_summary()
    );
}
