//! `thumbnails` engine tests, split out of `engine_bridge.rs`.

use super::*;
/// Security P3-7: the landing's thumbnail failure logs a category, never the error's text
/// (it carries the request URL with the file id).
#[tokio::test]
async fn a_landings_thumbnail_failure_logs_a_category_not_its_text() {
    let photo = "3f2a9c1e-0000-4000-8000-0000000000c2";
    let (dir, sync_root, server, bridge) = rule3_setup([152u8; 32]);
    seed_uploaded_row(&bridge, &server, photo);
    server.state.lock().unwrap().fail_thumbnail_put = true;
    let png = dir.path().join("photo-save.png");
    write_test_png(&png);
    bridge
        .queue_file_provider_modify(FinderWriteTarget {
            file_id: Some(photo.into()),
            parent_id: None,
            filename: "photo.png".into(),
            rel_path: None,
            kind: FinderWriteItemKind::File,
            contents_path: Some(png.to_string_lossy().into_owned()),
            content_type: Some("image/png".into()),
            base_version_identifier: Some("1".into()),
        })
        .unwrap();
    let logs = capture_logs_async(async {
        drain_upload_queue(&bridge, &sync_root).await;
    })
    .await;
    let lines: Vec<&str> = logs
        .lines()
        .filter(|line| line.contains("upload-time thumbnail generation/upload skipped"))
        .collect();
    assert_eq!(lines.len(), 1, "{logs}");
    assert!(lines[0].contains("reason="), "a category: {}", lines[0]);
    assert!(
        !lines[0].contains("error=") && !lines[0].contains("http://") && !lines[0].contains("thumbnail:"),
        "no error text: {}",
        lines[0]
    );
    drop(server.finish());
}

/// Review M2: the thumbnail refusal reads the queue before the mark. A landing that sets the
/// mark and then commits between the two reads (the seam) is refused either way; read the
/// other way round, the request saw "no mark", then "not queued", and served the previous
/// version's thumbnail under the new token.
#[tokio::test]
async fn a_thumbnail_request_racing_a_landing_is_refused() {
    let photo = "3f2a9c1e-0000-4000-8000-0000000000c3";
    let (dir, _sync_root, server, bridge) = rule3_setup([153u8; 32]);
    seed_uploaded_row(&bridge, &server, photo);
    fp_save(&bridge, dir.path(), photo, "photo.png", b"a queued save", "1");
    let op_id = bridge.db.list_operations_for_file(photo).unwrap().remove(0).op_id;
    let bridge = Arc::new(bridge);
    let landing = Arc::clone(&bridge);
    // The landing's order: the mark first, then the commit that takes the op off the queue.
    bridge.seams.arm("thumbnail:between_reads", move || {
        landing.set_thumbnails_pending(photo, ThumbnailWait::Waiting);
        landing.db.remove_operation(&op_id).unwrap();
    });
    let error = bridge
        .finder_thumbnail(photo, "small")
        .await
        .expect_err("refused: the server holds the previous version's thumbnail");
    assert!(engine_cause::<ThumbnailNotYet>(&error).is_some(), "{error:#}");
    let bridge = Arc::try_unwrap(bridge).unwrap_or_else(|_| panic!("the seam ran and released its handle"));
    drop(bridge);
    assert!(
        server.finish().thumbnail_requests.is_empty(),
        "nothing asked of the server"
    );
}

/// Review M3: a save accepted while a server fetch of the same file runs leaves the row
/// `Uploading`: the fetch's final status is written only over its own `Downloading`.
#[tokio::test]
async fn a_save_accepted_during_a_fetch_leaves_the_row_uploading() {
    let file = "3f2a9c1e-0000-4000-8000-0000000000c4";
    let (dir, sync_root, server, bridge) = rule3_setup([154u8; 32]);
    seed_uploaded_row(&bridge, &server, file);
    fp_save(
        &bridge,
        dir.path(),
        file,
        "notes.txt",
        b"the version the server has",
        "1",
    );
    drain_upload_queue(&bridge, &sync_root).await;
    let landed = held_content_version(&bridge, file);
    server.state.lock().unwrap().serve_downloads = true;
    let bridge = Arc::new(bridge);
    let saver = Arc::clone(&bridge);
    let save_dir = dir.path().to_path_buf();
    bridge.seams.arm("hydrate:before_final_status", move || {
        fp_save(&saver, &save_dir, file, "notes.txt", b"saved during the fetch", &landed);
    });
    let dest = dir.path().join("fetch").join("notes.txt");
    std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
    assert_eq!(
        bridge.serve_hydrate(file, &dest, &[dir.path()], None).await.unwrap(),
        HydrateSource::Server
    );
    assert_eq!(
        bridge.db.list_operations_for_file(file).unwrap().len(),
        1,
        "precondition: the save was accepted during the fetch"
    );
    assert_eq!(
        bridge.db.get_file(file).unwrap().unwrap().status,
        FileStatus::Uploading,
        "the fetch left the save's status alone"
    );
    let bridge = Arc::try_unwrap(bridge).unwrap_or_else(|_| panic!("the seam ran and released its handle"));
    drop(bridge);
    drop(server.finish());
}

/// Review M6: a snapshot that keeps failing no longer holds the queue. The first two failures
/// end their ticks, as before; from the third on, the tick pulls ops and the queue runs, so a
/// write whose base is known lands, and the request is kept. A successful snapshot resets the
/// count: the next failure ends its tick again.
#[cfg(target_os = "macos")]
#[tokio::test]
async fn a_snapshot_that_keeps_failing_no_longer_holds_the_queue() {
    let master_key = [155u8; 32];
    let (dir, sync_root, server, bridge) = rule3_setup(master_key);
    seed_uploaded_row(&bridge, &server, "a7a64666-3270-5569-9c06-791f0846979a");
    bridge.db.set_sync_cursor(0).unwrap();
    fp_save(
        &bridge,
        dir.path(),
        "a7a64666-3270-5569-9c06-791f0846979a",
        "notes.txt",
        b"saved while the snapshot fails",
        "1",
    );
    bridge.db.request_resnapshot().unwrap();
    // The runner runs the queue only after a tick that returned `Ok`.
    async fn tick(bridge: &EngineBridge, sync_root: &Path) -> bool {
        let ok = sync_tick_outcome(bridge, sync_root).await.is_ok();
        if ok {
            bridge.process_due_operations(sync_root, now_secs()).await.unwrap();
        }
        ok
    }
    let mut ticks = Vec::new();
    for _ in 0..3 {
        ticks.push(tick(&bridge, &sync_root).await);
    }
    assert_eq!(
        ticks,
        vec![false, false, true],
        "the third failed snapshot no longer ends the tick"
    );
    assert!(
        bridge
            .db
            .list_operations_for_file("a7a64666-3270-5569-9c06-791f0846979a")
            .unwrap()
            .is_empty(),
        "the write whose base is known landed"
    );
    assert!(
        bridge.db.peek_resnapshot_request().unwrap().is_some(),
        "the request is kept: the next tick tries again"
    );
    server.state.lock().unwrap().snapshots.push_back((
        "200 OK".into(),
        serde_json::json!({
            "seq_id": 3,
            "nodes": [node(&master_key, "a7a64666-3270-5569-9c06-791f0846979a", "notes.txt", 2, false)]
        }),
    ));
    assert!(tick(&bridge, &sync_root).await, "the snapshot succeeded");
    assert_eq!(bridge.db.peek_resnapshot_request().unwrap(), None);
    bridge.db.request_resnapshot().unwrap();
    assert!(
        !tick(&bridge, &sync_root).await,
        "the count was reset: a first failure ends its tick again"
    );
    drop(server.finish());
}

/// Parked item 3b: a save with no content type of its own, whose only `complete` answer is
/// `already_completed` (no `mime_type`), lands keeping the row's content type.
#[tokio::test]
async fn a_landing_with_no_content_type_keeps_the_rows() {
    let (dir, sync_root, server, bridge) = rule3_setup([156u8; 32]);
    seed_uploaded_row(&bridge, &server, "a1f1e7d0-8c7b-57aa-bce6-12351453ef81");
    let mut contract = bridge
        .db
        .get_file_contract_state("a1f1e7d0-8c7b-57aa-bce6-12351453ef81")
        .unwrap()
        .unwrap();
    contract.content_type = Some("text/markdown".into());
    bridge.db.set_file_contract_state(&contract).unwrap();
    let contents = dir.path().join("save-typed");
    std::fs::write(&contents, b"edit").unwrap();
    let mut target = finder_file_target(
        Some("a1f1e7d0-8c7b-57aa-bce6-12351453ef81"),
        "notes",
        &contents,
        Some("1".into()),
    );
    target.content_type = None;
    bridge.queue_file_provider_modify(target).unwrap();
    server
        .state
        .lock()
        .unwrap()
        .complete_reply_lost_once
        .insert("session-1".into());
    drain_upload_queue(&bridge, &sync_root).await;
    drop(server.finish());
    assert!(
        bridge
            .db
            .list_operations_for_file("a1f1e7d0-8c7b-57aa-bce6-12351453ef81")
            .unwrap()
            .is_empty(),
        "landed"
    );
    let contract = bridge
        .db
        .get_file_contract_state("a1f1e7d0-8c7b-57aa-bce6-12351453ef81")
        .unwrap()
        .unwrap();
    assert_eq!(contract.current_version, 2, "precondition: the retried landing");
    assert_eq!(
        contract.content_type.as_deref(),
        Some("text/markdown"),
        "the row's content type is kept"
    );
}

/// Spec §8.7 S6: an engine start's sweep that runs between an accept's copy and its
/// transaction (a stopping engine's handler still inside an accept) removes the unfinished
/// copy and its row. The accept's transaction then finds the row gone, queues nothing, and the
/// reply asks the system to hand the write over again.
#[cfg(target_os = "macos")]
#[tokio::test]
async fn an_accept_whose_copy_the_start_sweep_removed_is_retried_later() {
    let (dir, _sync_root, server, bridge) = rule3_setup([168u8; 32]);
    seed_uploaded_row(&bridge, &server, "7051838c-b073-5630-90fc-7e9c7964ba46");
    let bases = FinderStagingBases {
        folder: "beebeeb",
        data: Some(dir.path().join("data")),
        cache: Some(dir.path().join("cache")),
        temp: dir.path().join("tmp"),
    };
    let root = bases.durable_root().unwrap();
    bridge.seams.stage_under(bases);
    let db = bridge.db.clone();
    let sweep_root = root.clone();
    let swept = Arc::new(Mutex::new(None));
    let swept_seen = Arc::clone(&swept);
    bridge.seams.arm("accept:before_tx", move || {
        let sweep = crate::staged_payload::sweep_unreferenced(&db, std::slice::from_ref(&sweep_root)).unwrap();
        *swept_seen.lock().unwrap() = Some(sweep);
    });
    let contents = dir.path().join("save.txt");
    std::fs::write(&contents, b"a save the sweep caught").unwrap();
    let result = bridge.queue_file_provider_modify(finder_file_target(
        Some("7051838c-b073-5630-90fc-7e9c7964ba46"),
        "notes.txt",
        &contents,
        Some("1".into()),
    ));
    let reply = crate::ipc_socket::write_outcome_response("modify", &bridge.db, result);
    let sweep = swept
        .lock()
        .unwrap()
        .take()
        .expect("the sweep ran between the copy and the accept");
    assert_eq!(
        sweep.unfinished, 1,
        "precondition: it removed the unfinished copy: {sweep:?}"
    );
    assert!(
        matches!(reply, crate::ipc_socket::IpcResponse::WriteRetryLater { .. }),
        "{reply:?}"
    );
    assert!(
        bridge
            .db
            .list_operations_for_file("7051838c-b073-5630-90fc-7e9c7964ba46")
            .unwrap()
            .is_empty(),
        "nothing was queued"
    );
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0, "no copy is left");
    assert!(
        bridge.db.staged_payloads_for_signout().unwrap().is_empty(),
        "and no journal row"
    );
    assert_eq!(
        bridge
            .db
            .get_file("7051838c-b073-5630-90fc-7e9c7964ba46")
            .unwrap()
            .unwrap()
            .status,
        FileStatus::Local,
        "the row is as it was"
    );
    drop(server.finish());
}

// ── Rule 3: provisional ids, fetches from the queue, thumbnails, unknown ids (spec §7, §5.6) ──
