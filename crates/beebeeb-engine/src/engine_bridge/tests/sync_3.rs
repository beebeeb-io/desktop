//! `sync_3` engine tests, split out of `engine_bridge.rs`.

use super::*;
#[tokio::test]
async fn a_hand_over_whose_successor_then_runs_puts_its_item_in_the_ticks_signal() {
    // `ClaimOutcome::Claimed` with a hand-over: W parks `payload_missing`, N takes its role
    // at the claim and runs. N's `init` is answered "in progress", a plain retry, which
    // adds no id of its own.
    let (dir, sync_root, server, bridge) = rule3_setup([124u8; 32]);
    seed_uploaded_row(&bridge, &server, "d2aeddeb-8b87-53e4-8532-16505d07e9e9");
    let w = fp_save(
        &bridge,
        dir.path(),
        "d2aeddeb-8b87-53e4-8532-16505d07e9e9",
        "notes.txt",
        b"W",
        "1",
    );
    let w_op = bridge
        .db
        .list_operations_for_file("d2aeddeb-8b87-53e4-8532-16505d07e9e9")
        .unwrap()
        .remove(0);
    std::fs::remove_file(w_op.payload_path.as_deref().unwrap()).unwrap();
    bridge.process_due_operations(&sync_root, now_secs()).await.unwrap(); // W parks
    fp_save(
        &bridge,
        dir.path(),
        "d2aeddeb-8b87-53e4-8532-16505d07e9e9",
        "notes.txt",
        b"W N",
        w.token.as_deref().unwrap(),
    );
    server.state.lock().unwrap().conflict_init_once.insert(
        "d2aeddeb-8b87-53e4-8532-16505d07e9e9".into(),
        crate::api_client::IN_PROGRESS_MESSAGE.into(),
    );

    let mut outcome = None;
    let logs = capture_logs_async(async {
        outcome = Some(bridge.process_due_operations(&sync_root, now_secs()).await.unwrap());
    })
    .await;

    assert_eq!(logs.matches("queued write took over a parked one").count(), 1, "{logs}");
    let ops = bridge
        .db
        .list_operations_for_file("d2aeddeb-8b87-53e4-8532-16505d07e9e9")
        .unwrap();
    assert_eq!(ops.len(), 1, "precondition: W's op is gone");
    assert_eq!(ops[0].attempts, 1, "precondition: N ran once and will retry");
    assert_eq!(
        outcome.unwrap().invalidated_item_ids,
        vec!["d2aeddeb-8b87-53e4-8532-16505d07e9e9".to_string()]
    );
    let state = server.finish();
    assert_eq!(
        state.init_summary(),
        vec![(json!("d2aeddeb-8b87-53e4-8532-16505d07e9e9"), json!(1), 409)],
        "N ran on W's base"
    );
}

#[tokio::test]
async fn a_hand_over_whose_successor_parks_at_the_claim_puts_its_item_in_the_ticks_signal() {
    // `ClaimOutcome::Parked` with a hand-over, the fixture of
    // `a_base_unknown_predecessor_hands_over_and_the_successor_parks_without_a_request`.
    let (dir, sync_root, server, bridge) = rule3_setup([125u8; 32]);
    seed_uploaded_row(&bridge, &server, "82787d46-2fcf-5eec-836a-59aef6459f31"); // version 1, not filled
    let w = fp_save(
        &bridge,
        dir.path(),
        "82787d46-2fcf-5eec-836a-59aef6459f31",
        "notes.txt",
        b"W",
        "0",
    );
    fp_save(
        &bridge,
        dir.path(),
        "82787d46-2fcf-5eec-836a-59aef6459f31",
        "notes.txt",
        b"W N",
        w.token.as_deref().unwrap(),
    );

    let mut outcome = None;
    let logs = capture_logs_async(async {
        outcome = Some(bridge.process_due_operations(&sync_root, now_secs()).await.unwrap());
    })
    .await;

    assert_eq!(logs.matches("queued write took over a parked one").count(), 1, "{logs}");
    let ops = bridge
        .db
        .list_operations_for_file("82787d46-2fcf-5eec-836a-59aef6459f31")
        .unwrap();
    assert_eq!(ops.len(), 1, "precondition: W's op is gone");
    assert_eq!(
        ops[0].last_error.as_deref(),
        Some("base_unknown"),
        "precondition: N parked"
    );
    assert_eq!(
        outcome.unwrap().invalidated_item_ids,
        vec!["82787d46-2fcf-5eec-836a-59aef6459f31".to_string()]
    );
    drop(server.finish());
}

/// A transient failure (here a 500) never uses up a File Provider write's attempts (spec §8.4):
/// the attempt at the cap retries, leaves the item `uploading` and puts nothing in the signal.
/// A failure waiting does not cure still parks at the cap
/// (`a_finder_write_the_server_keeps_refusing_parks_when_its_attempts_are_used_up`).
#[tokio::test]
async fn the_attempt_at_the_cap_leaves_a_finder_write_uploading_and_out_of_the_signal() {
    let (dir, sync_root, server, bridge) = rule3_setup([126u8; 32]);
    seed_uploaded_row(&bridge, &server, "829468d7-4660-5a3d-a795-7074384ee957");
    server.state.lock().unwrap().refuse_init.insert(
        "829468d7-4660-5a3d-a795-7074384ee957".into(),
        "500 Internal Server Error",
    );
    fp_save(
        &bridge,
        dir.path(),
        "829468d7-4660-5a3d-a795-7074384ee957",
        "notes.txt",
        b"a save",
        "1",
    );
    let op_id = bridge
        .db
        .list_operations_for_file("829468d7-4660-5a3d-a795-7074384ee957")
        .unwrap()
        .remove(0)
        .op_id;
    let now = now_secs();

    // An attempt that leaves attempts to spare changes nothing the system shows.
    let first = bridge.process_due_operations(&sync_root, now).await.unwrap();
    assert_eq!(bridge.db.get_operation(&op_id).unwrap().unwrap().attempts, 1);
    assert!(
        first.invalidated_item_ids.is_empty(),
        "{:?}",
        first.invalidated_item_ids
    );

    bridge
        .db
        .hold_lock_for_test()
        .execute(
            "UPDATE operation_queue SET attempts = max_attempts - 1 WHERE op_id = ?1",
            rusqlite::params![op_id],
        )
        .unwrap();
    let last = bridge.process_due_operations(&sync_root, now + 10_000).await.unwrap();

    let op = bridge.db.get_operation(&op_id).unwrap().unwrap();
    assert_eq!(op.attempts, op.max_attempts - 1, "its attempts are never used up");
    assert_eq!(
        bridge
            .db
            .get_file("829468d7-4660-5a3d-a795-7074384ee957")
            .unwrap()
            .unwrap()
            .status,
        FileStatus::Uploading,
        "the row still presents uploading"
    );
    assert!(
        last.invalidated_item_ids.is_empty() && last.paused_op_ids.is_empty(),
        "{last:?}"
    );
    drop(server.finish());
}

#[tokio::test]
async fn a_park_whose_op_moved_under_its_claim_puts_no_item_in_the_signal() {
    // The stale-base 409 park, with the op purged between the attempt and the park (the
    // `outcome:before_tx` seam): `park_claimed` writes nothing, so nothing changed for the
    // system and no id is reported.
    let (dir, sync_root, server, bridge) = rule3_setup([127u8; 32]);
    seed_uploaded_row(&bridge, &server, "eef0840b-33a7-55ce-a234-5e789411720d");
    server.seed_file("eef0840b-33a7-55ce-a234-5e789411720d", 2);
    fp_save(
        &bridge,
        dir.path(),
        "eef0840b-33a7-55ce-a234-5e789411720d",
        "notes.txt",
        b"edit on v1",
        "1",
    );
    let op_id = bridge
        .db
        .list_operations_for_file("eef0840b-33a7-55ce-a234-5e789411720d")
        .unwrap()
        .remove(0)
        .op_id;
    let mut contract = bridge
        .db
        .get_file_contract_state("eef0840b-33a7-55ce-a234-5e789411720d")
        .unwrap()
        .unwrap();
    contract.namespace = Namespace::SharedWithMe;
    contract.shared_root_id = Some("moved-root".into());
    bridge.db.set_file_contract_state(&contract).unwrap();
    let db = bridge.db.clone();
    bridge.seams.arm("outcome:before_tx", move || {
        run_competing(move || {
            db.purge_revoked_shared_content(&[]).unwrap();
        })
    });

    let mut outcome = None;
    let logs = capture_logs_async(async {
        outcome = Some(bridge.process_due_operations(&sync_root, now_secs()).await.unwrap());
    })
    .await;

    assert!(
        bridge.db.get_operation(&op_id).unwrap().is_none(),
        "precondition: purged"
    );
    assert_eq!(
        logs.lines()
            .filter(|line| line.contains("queue state moved")
                && line.contains(&op_id)
                && line.contains("step=\"park\""))
            .count(),
        1,
        "precondition: the park found the op gone:\n{logs}"
    );
    assert!(
        outcome.unwrap().invalidated_item_ids.is_empty(),
        "a park that did not commit is not reported"
    );
    let state = server.finish();
    assert_eq!(
        state.init_summary(),
        vec![(json!("eef0840b-33a7-55ce-a234-5e789411720d"), json!(1), 409)]
    );
}

// ── The accept transaction and the base mapping (spec §6.1, §6.3.1, §8.7 S1.1, S3, S5) ──

#[tokio::test]
async fn i2_a_zero_base_on_a_versioned_row_parks() {
    let dir = tempfile::tempdir().unwrap();
    let sync_root = dir.path().join("sync-root");
    std::fs::create_dir_all(&sync_root).unwrap();
    let master_key = [44u8; 32];
    let server = VersionedServerMock::start();
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
    seed_uploaded_row(&bridge, &server, "264bb291-f121-5df9-8680-1a8ad36c57e1");
    fp_save(
        &bridge,
        dir.path(),
        "264bb291-f121-5df9-8680-1a8ad36c57e1",
        "notes.txt",
        b"edit on a stale 0",
        "0",
    );
    drain_upload_queue(&bridge, &sync_root).await;
    let state = server.finish();
    assert!(state.inits.is_empty(), "nothing is sent: {:?}", state.init_summary());
    let op = bridge
        .db
        .list_operations_for_file("264bb291-f121-5df9-8680-1a8ad36c57e1")
        .unwrap()
        .remove(0);
    assert_eq!(op.attempts, op.max_attempts, "parked at once");
    assert!(
        std::path::Path::new(op.payload_path.as_deref().unwrap()).is_file(),
        "the bytes are kept"
    );
}

#[tokio::test]
async fn the_init_guard_refuses_a_finder_replace_without_a_base() {
    let dir = tempfile::tempdir().unwrap();
    let sync_root = dir.path().join("sync-root");
    std::fs::create_dir_all(&sync_root).unwrap();
    let master_key = [45u8; 32];
    let server = VersionedServerMock::start();
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
    seed_uploaded_row(&bridge, &server, "cbe007c9-a763-53b7-a3f8-76aed6347b1b");
    fp_save(
        &bridge,
        dir.path(),
        "cbe007c9-a763-53b7-a3f8-76aed6347b1b",
        "notes.txt",
        b"a base beyond i32",
        "3000000000",
    );
    seed_uploaded_row(&bridge, &server, "2537b462-b8ec-5985-b74d-668d9a382eb3");
    fp_save(
        &bridge,
        dir.path(),
        "2537b462-b8ec-5985-b74d-668d9a382eb3",
        "notes.txt",
        b"a base that went missing",
        "1",
    );
    let op = bridge
        .db
        .list_operations_for_file("2537b462-b8ec-5985-b74d-668d9a382eb3")
        .unwrap()
        .remove(0);
    bridge.db.set_base_version_for_test(&op.op_id, None);
    drain_upload_queue(&bridge, &sync_root).await;
    let state = server.finish();
    assert!(
        state.inits.is_empty(),
        "no replace without a base: {:?}",
        state.init_summary()
    );
    for file in [
        "cbe007c9-a763-53b7-a3f8-76aed6347b1b",
        "2537b462-b8ec-5985-b74d-668d9a382eb3",
    ] {
        let op = bridge.db.list_operations_for_file(file).unwrap().remove(0);
        assert_eq!(op.attempts, op.max_attempts, "{file} parked");
    }
}

#[tokio::test]
async fn a_newer_save_queues_behind_a_write_without_a_session_and_both_land() {
    let dir = tempfile::tempdir().unwrap();
    let sync_root = dir.path().join("sync-root");
    std::fs::create_dir_all(&sync_root).unwrap();
    let master_key = [46u8; 32];
    let server = VersionedServerMock::start();
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
    seed_uploaded_row(&bridge, &server, "f05e9de2-3d3e-5e26-9562-5f163659cee3");
    let first = fp_save(
        &bridge,
        dir.path(),
        "f05e9de2-3d3e-5e26-9562-5f163659cee3",
        "notes.txt",
        b"first",
        "1",
    );
    fp_save(
        &bridge,
        dir.path(),
        "f05e9de2-3d3e-5e26-9562-5f163659cee3",
        "notes.txt",
        b"first, second",
        first.token.as_deref().unwrap_or("1"),
    );
    drain_upload_queue(&bridge, &sync_root).await;
    let state = server.finish();
    assert_eq!(
        state.init_summary(),
        vec![
            (json!("f05e9de2-3d3e-5e26-9562-5f163659cee3"), json!(1), 201),
            (json!("f05e9de2-3d3e-5e26-9562-5f163659cee3"), json!(2), 201)
        ],
        "one upload per save, in order"
    );
    assert_eq!(
        state.files["f05e9de2-3d3e-5e26-9562-5f163659cee3"].versions.len(),
        3,
        "both saves are versions in the history"
    );
    assert_eq!(
        state.latest_plaintext("f05e9de2-3d3e-5e26-9562-5f163659cee3", master_key),
        b"first, second"
    );
}
