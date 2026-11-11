//! `uploads_9` engine tests, split out of `engine_bridge.rs`.

use super::*;
/// T30 (§9.1–§9.2): A lands while B is queued and
/// B's first attempt fails: the item is `uploading`, with B's size and B's token.
#[cfg(unix)] // names `crate::ipc_socket::file_entry_payload_for_db`
#[tokio::test]
async fn i1_a_landing_with_a_later_write_queued_keeps_it_uploading() {
    let (dir, sync_root, server, bridge) = rule3_setup([80u8; 32]);
    seed_uploaded_row(&bridge, &server, "e3ab802e-f604-5e5c-a66b-334950ef0e77");
    let a = fp_save(
        &bridge,
        dir.path(),
        "e3ab802e-f604-5e5c-a66b-334950ef0e77",
        "notes.txt",
        b"A",
        "1",
    );
    let b = fp_save(
        &bridge,
        dir.path(),
        "e3ab802e-f604-5e5c-a66b-334950ef0e77",
        "notes.txt",
        b"A, and B's longer bytes",
        a.token.as_deref().unwrap(),
    );
    server
        .state
        .lock()
        .unwrap()
        .fail_first_chunk_once
        .insert("session-2".into()); // B backs off
    bridge.process_due_operations(&sync_root, now_secs()).await.unwrap();
    let entry = bridge
        .db
        .get_file("e3ab802e-f604-5e5c-a66b-334950ef0e77")
        .unwrap()
        .unwrap();
    let payload = crate::ipc_socket::file_entry_payload_for_db(&bridge.db, &entry, "root");
    assert_eq!(payload.status, "uploading");
    assert_eq!(
        payload.size_bytes,
        b"A, and B's longer bytes".len() as i64,
        "the newer write's size"
    );
    assert_eq!(payload.content_version, b.token, "the newer write's token");
    let state = server.finish();
    assert_eq!(
        state.init_summary(),
        vec![
            (json!("e3ab802e-f604-5e5c-a66b-334950ef0e77"), json!(1), 201),
            (json!("e3ab802e-f604-5e5c-a66b-334950ef0e77"), json!(2), 201)
        ],
        "precondition: A landed and B's attempt began"
    );
}

/// A lands while B is queued. When the runner records A's outcome
/// (`outcome:before_tx`), the row is `Uploading`: the landing left it so for B (§9.2), and the
/// attempt no longer reads that as a failure and rolls it back to `Error`.
#[tokio::test]
async fn a_landing_with_a_later_write_queued_is_uploading_when_its_outcome_is_recorded() {
    let (dir, sync_root, server, bridge) = rule3_setup([99u8; 32]);
    seed_uploaded_row(&bridge, &server, "1f0cf4ce-d5e7-526f-98b9-3696300fe8d2");
    let a = fp_save(
        &bridge,
        dir.path(),
        "1f0cf4ce-d5e7-526f-98b9-3696300fe8d2",
        "notes.txt",
        b"A",
        "1",
    );
    fp_save(
        &bridge,
        dir.path(),
        "1f0cf4ce-d5e7-526f-98b9-3696300fe8d2",
        "notes.txt",
        b"A B",
        a.token.as_deref().unwrap(),
    );
    let seen = Arc::new(Mutex::new(None));
    let (db, seen_in_seam) = (bridge.db.clone(), Arc::clone(&seen));
    bridge.seams.arm("outcome:before_tx", move || {
        let ops_left = db
            .list_operations_for_file("1f0cf4ce-d5e7-526f-98b9-3696300fe8d2")
            .unwrap()
            .len();
        let status = db
            .get_file("1f0cf4ce-d5e7-526f-98b9-3696300fe8d2")
            .unwrap()
            .unwrap()
            .status;
        *seen_in_seam.lock().unwrap() = Some((ops_left, status));
    });
    bridge.process_due_operations(&sync_root, now_secs()).await.unwrap();
    let (ops_left, status) = seen.lock().unwrap().take().expect("the seam fired");
    assert_eq!(
        ops_left, 1,
        "precondition: A's outcome, after its landing, with B queued"
    );
    assert_eq!(
        status,
        FileStatus::Uploading,
        "not rolled back to Error after A's landing"
    );
    drop(server.finish());
}

/// §7.4, §9.1: a write that parked because its staged copy is gone never
/// leaves the queue, and nothing local can serve it. Its fetch goes to the server for the version
/// the server has, and the item keeps the parked status: read-only, not evictable. A write still
/// queued with its copy missing keeps its error
/// (`a_queued_write_whose_copy_is_missing_is_never_fetched_from_the_server`).
#[cfg(unix)] // names `crate::ipc_socket::file_entry_payload_for_db`
#[tokio::test]
async fn a_parked_write_whose_copy_is_gone_is_fetched_from_the_server() {
    // UUID-shaped: the hydrate parses the id before it asks the server.
    let live = "3f2a9c1e-0000-4000-8000-0000000000b5";
    let (dir, sync_root, server, bridge) = rule3_setup([100u8; 32]);
    seed_uploaded_row(&bridge, &server, live);
    fp_save(
        &bridge,
        dir.path(),
        live,
        "notes.txt",
        b"the version the server has",
        "1",
    );
    drain_upload_queue(&bridge, &sync_root).await;
    let landed = held_content_version(&bridge, live);
    fp_save(
        &bridge,
        dir.path(),
        live,
        "notes.txt",
        b"a save whose copy is gone",
        &landed,
    );
    let op = bridge.db.list_operations_for_file(live).unwrap().remove(0);
    std::fs::remove_file(op.payload_path.as_deref().unwrap()).unwrap();
    bridge.process_due_operations(&sync_root, now_secs()).await.unwrap();
    let parked = bridge
        .db
        .get_operation(&op.op_id)
        .unwrap()
        .expect("a parked write stays queued");
    assert_eq!(parked.attempts, parked.max_attempts, "precondition: parked");
    assert_eq!(parked.last_error.as_deref(), Some("payload_missing"));
    let before = {
        let mut state = server.state.lock().unwrap();
        state.serve_downloads = true;
        state.requests.len()
    };
    let dest = dir.path().join("fetch").join("notes.txt");
    // `hydrate_dest_is_allowed` canonicalizes the parent.
    std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
    let source = bridge
        .serve_hydrate(live, &dest, &[dir.path()], None)
        .await
        .expect("nothing local can serve it: the server's version");
    assert_eq!(source, HydrateSource::Server);
    assert_eq!(std::fs::read(&dest).unwrap(), b"the version the server has");
    let entry = bridge.db.get_file(live).unwrap().unwrap();
    assert_eq!(entry.status, FileStatus::Error, "the parked status");
    let payload = crate::ipc_socket::file_entry_payload_for_db(&bridge.db, &entry, "root");
    assert_eq!(payload.status, "error", "presented parked: read-only, not evictable");
    assert_eq!(payload.capabilities & crate::ipc_socket::CAP_WRITE, 0, "read-only");
    let state = server.finish();
    let fetch = &state.requests[before..];
    let metadata = format!("/api/v1/files/{live}");
    assert_eq!(
        fetch.iter().filter(|(m, path)| m == "GET" && path == &metadata).count(),
        1,
        "one fetch reached the server: {fetch:?}"
    );
    // Task 2173: plus at most one discovery request (this server has no direct route).
    assert!(
        fetch
            .iter()
            .all(|(m, path)| m == "GET" && (path.starts_with(&metadata) || path == "/api/v1/transfer/capabilities")),
        "nothing but that fetch: {fetch:?}"
    );
    assert!(
        fetch
            .iter()
            .filter(|(_, path)| path == "/api/v1/transfer/capabilities")
            .count()
            <= 1,
        "discovery at most once: {fetch:?}"
    );
}

/// §9.1 (step 3.3): a landing whose only later write parked leaves the parked status, through
/// the same settle as every park: a create's successor it could not re-key, and a modify's
/// successor parked at its accept. The modify's own op is keyed on the same file, so the settle
/// runs once that op is gone.
#[tokio::test]
async fn a_landing_whose_only_later_write_parked_leaves_the_parked_status() {
    let (dir, sync_root, server, bridge) = rule3_setup([101u8; 32]);
    seed_uploaded_row(&bridge, &server, "cb1e6a6c-0825-5070-8a3f-3a919b21e080");
    fp_save(
        &bridge,
        dir.path(),
        "cb1e6a6c-0825-5070-8a3f-3a919b21e080",
        "notes.txt",
        b"A",
        "1",
    );
    let no_base = dir.path().join("no-base.txt");
    std::fs::write(&no_base, b"A, and B").unwrap();
    bridge
        .queue_file_provider_modify(finder_file_target(
            Some("cb1e6a6c-0825-5070-8a3f-3a919b21e080"),
            "notes.txt",
            &no_base,
            None,
        ))
        .unwrap();
    let ops = bridge
        .db
        .list_operations_for_file("cb1e6a6c-0825-5070-8a3f-3a919b21e080")
        .unwrap();
    assert_eq!(
        ops[1].attempts, ops[1].max_attempts,
        "precondition: B parked at its accept"
    );
    assert_eq!(
        bridge
            .db
            .get_file("cb1e6a6c-0825-5070-8a3f-3a919b21e080")
            .unwrap()
            .unwrap()
            .status,
        FileStatus::Uploading,
        "precondition: A still uploads"
    );
    let created = fp_create(&bridge, dir.path(), "new.txt", b"created");
    let provisional = created.outcome_file_id();
    fp_save(
        &bridge,
        dir.path(),
        &provisional,
        "new.txt",
        b"created, edited",
        created.token.as_deref().unwrap(),
    );
    // SF-0001 (L13): the create lands under its own id, so the successor parks on the one
    // thing the queue cannot cure — its staged copy is gone — not on a re-key failure.
    let successor = bridge.db.list_operations_for_file(&provisional).unwrap().remove(1);
    std::fs::remove_file(successor.payload_path.as_deref().unwrap()).unwrap();
    drain_upload_queue(&bridge, &sync_root).await;
    let state = server.finish();
    let server_id = state
        .files_with_content()
        .into_iter()
        .find(|id| id != "cb1e6a6c-0825-5070-8a3f-3a919b21e080")
        .expect("the create landed");
    assert_eq!(server_id, provisional, "the create landed under the id it chose");
    let parked = bridge.db.get_operation(&successor.op_id).unwrap().unwrap();
    assert_eq!(
        parked.attempts, parked.max_attempts,
        "precondition: the successor parked"
    );
    assert_eq!(
        bridge.db.get_file(&server_id).unwrap().unwrap().status,
        FileStatus::Error,
        "a parked upload presents error"
    );
    assert_eq!(
        state.latest_plaintext("cb1e6a6c-0825-5070-8a3f-3a919b21e080", [101u8; 32]),
        b"A",
        "precondition: A landed"
    );
    assert_eq!(
        bridge
            .db
            .get_file("cb1e6a6c-0825-5070-8a3f-3a919b21e080")
            .unwrap()
            .unwrap()
            .status,
        FileStatus::Error,
        "after a modify's landing, its parked successor presents error"
    );
}

// ── Rule 6: the upgrade path (spec §10.2–§10.3, m-4c) ──

/// T34, spec §10.3 (ruling 2): an upload an earlier build queued on a wall-clock second is
/// marked at engine start and keeps that base. It is sent once, the server refuses it as a
/// stale base, and it parks at its first attempt with its bytes. It never holds a token.
#[tokio::test]
async fn an_old_timestamp_base_parks_at_its_first_attempt() {
    // The two QA shapes (spec §2 item 11); this is the first.
    let (dir, sync_root, server, bridge) = rule3_setup([99u8; 32]);
    seed_uploaded_row(&bridge, &server, "1db7b53d-bbcf-5bd0-ae6d-c144cbfaaaab");
    // The way the round-3 QA build queued it: through the shared entry point, no write id,
    // a base that is a wall-clock second (QA/inv2-statedb-copy-queries.log).
    let contents = dir.path().join("old.txt");
    std::fs::write(&contents, b"forty bytes the old build queued, here.").unwrap();
    bridge
        .queue_finder_modify(finder_file_target(
            Some("1db7b53d-bbcf-5bd0-ae6d-c144cbfaaaab"),
            "notes.txt",
            &contents,
            Some("1791550250".into()),
        ))
        .unwrap();
    assert_eq!(bridge.db.mark_earlier_build_uploads().unwrap(), 1);
    let op = bridge
        .db
        .list_operations_for_file("1db7b53d-bbcf-5bd0-ae6d-c144cbfaaaab")
        .unwrap()
        .remove(0);
    assert_eq!(op.base_version, Some(1_791_550_250), "no re-base");
    assert_eq!(
        bridge.db.finder_write(&op.op_id).unwrap().unwrap().origin,
        crate::state_db::WriteOrigin::EarlierBuild
    );
    drain_upload_queue(&bridge, &sync_root).await;
    let state = server.finish();
    assert_eq!(
        state.init_summary(),
        vec![(
            json!("1db7b53d-bbcf-5bd0-ae6d-c144cbfaaaab"),
            json!(1_791_550_250_i64),
            409
        )],
        "one init with the timestamp base"
    );
    let op = bridge.db.get_operation(&op.op_id).unwrap().unwrap();
    assert_eq!(op.attempts, op.max_attempts, "parked at its first attempt");
    assert!(
        std::path::Path::new(op.payload_path.as_deref().unwrap()).is_file(),
        "its bytes are kept"
    );
    assert!(
        bridge
            .db
            .item_presentation("1db7b53d-bbcf-5bd0-ae6d-c144cbfaaaab")
            .unwrap()
            .unwrap()
            .held
            .is_none(),
        "no held token"
    );
}

/// Spec §10.2 with §8.4's old folders: an upload an earlier build queued names its staged copy
/// by an absolute path in the old cache root, and the marking leaves that path as it is.
/// (a) Its base is version 1, but the server has moved on to 2: it parks. A newer save on
/// version 1, queued behind it, is never made its successor (the marking gives (a) no token,
/// and rule 3 excludes earlier builds), so it never takes it over: both park, and (a)'s copy
/// stays in the old root, journalled and unreleased. (b) On the server's version it uploads
/// from that copy, which is then released and unlinked, with its journal row. The data root
/// and the old cache root differ only on macOS, so the test is macOS-only.
#[cfg(target_os = "macos")]
#[tokio::test]
async fn an_earlier_builds_copy_in_the_old_cache_root_is_never_handed_over_and_never_lost() {
    let master_key = [102u8; 32];
    let (dir, sync_root, server, bridge) = rule3_setup(master_key);
    let bases = FinderStagingBases::current();
    let old_root = bases.cache_root();
    std::fs::create_dir_all(&old_root).unwrap();
    let mut queued = Vec::new();
    for (file_id, bytes, base) in [
        (
            "a3c0af61-de27-59e9-9c0b-6a9c538216d6",
            &b"an earlier build's edit of version 1"[..],
            "1",
        ),
        (
            "b1000000-0000-4000-8000-000000000006",
            &b"an earlier build's save"[..],
            "1",
        ),
    ] {
        seed_uploaded_row(&bridge, &server, file_id);
        let contents = dir.path().join(format!("{file_id}.txt"));
        std::fs::write(&contents, bytes).unwrap();
        bridge
            .queue_finder_modify(finder_file_target(
                Some(file_id),
                "notes.txt",
                &contents,
                Some(base.into()),
            ))
            .unwrap();
        let op = bridge.db.list_operations_for_file(file_id).unwrap().remove(0);
        // Where a build before the move staged it: the old cache root, by absolute path in
        // the op and the journal.
        let staged = PathBuf::from(op.payload_path.clone().expect("a save carries its copy"));
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
        queued.push((op.op_id, old_copy, bytes));
    }
    let (a_op, a_copy, a_bytes) = queued[0].clone();
    let (b_op, b_copy, b_bytes) = queued[1].clone();
    // (a)'s file changed elsewhere: the server is at version 2, which this row has not seen.
    server.seed_file("a3c0af61-de27-59e9-9c0b-6a9c538216d6", 2);

    assert_eq!(bridge.db.mark_earlier_build_uploads().unwrap(), 2);
    for (op_id, copy) in [(&a_op, &a_copy), (&b_op, &b_copy)] {
        let op = bridge.db.get_operation(op_id).unwrap().unwrap();
        assert_eq!(
            op.payload_path.as_deref(),
            Some(copy.to_string_lossy().as_ref()),
            "the marking leaves the copy's path as it is"
        );
    }
    // A newer save of (a)'s file, on the version the system holds, queued behind it. Had the
    // marking minted (a) as the accept does (a held token and the minted origin), rule 3 would
    // make this save (a)'s successor, and its claim would hand (a) over.
    fp_save(
        &bridge,
        dir.path(),
        "a3c0af61-de27-59e9-9c0b-6a9c538216d6",
        "notes.txt",
        b"a newer save",
        "1",
    );

    let logs = capture_logs_async(async {
        drain_upload_queue(&bridge, &sync_root).await;
    })
    .await;
    let state = server.finish();

    assert_eq!(
        state.init_summary(),
        vec![
            (json!("a3c0af61-de27-59e9-9c0b-6a9c538216d6"), json!(1), 409),
            (json!("b1000000-0000-4000-8000-000000000006"), json!(1), 201),
            (json!("a3c0af61-de27-59e9-9c0b-6a9c538216d6"), json!(1), 409)
        ],
        "(a) refused once, (b) uploaded once, and the newer save is sent on its own base"
    );
    // (a) never handed over, and its copy is not lost.
    let a = bridge
        .db
        .get_operation(&a_op)
        .unwrap()
        .expect("(a) is kept in the queue");
    assert_eq!(a.attempts, a.max_attempts, "(a) parked");
    assert_eq!(a.payload_path.as_deref(), Some(a_copy.to_string_lossy().as_ref()));
    assert_eq!(
        std::fs::read(&a_copy).unwrap(),
        a_bytes,
        "(a)'s copy stays in the old root, with its bytes"
    );
    assert!(
        bridge
            .db
            .staged_payloads_for_signout()
            .unwrap()
            .iter()
            .any(|(path, _, completed)| path == a_copy.to_string_lossy().as_ref() && !completed),
        "(a)'s journal row stays, not released"
    );
    let ops = bridge
        .db
        .list_operations_for_file("a3c0af61-de27-59e9-9c0b-6a9c538216d6")
        .unwrap();
    assert_eq!(ops.len(), 2, "(a) and the newer save both remain: no hand-over");
    assert!(ops.iter().all(|op| op.attempts == op.max_attempts), "both parked");
    assert_eq!(
        std::fs::read(ops[1].payload_path.as_deref().unwrap()).unwrap(),
        b"a newer save",
        "the newer save keeps its bytes too"
    );
    assert!(!logs.contains("took over"), "{logs}");
    assert_eq!(
        bridge.db.finder_write(&a_op).unwrap().unwrap().origin,
        crate::state_db::WriteOrigin::EarlierBuild,
        "(a) is still marked as from an earlier build"
    );
    // (b) read from the old root, landed, and released as any landing is.
    assert_eq!(
        state.latest_plaintext("b1000000-0000-4000-8000-000000000006", master_key),
        b_bytes
    );
    assert!(bridge.db.get_operation(&b_op).unwrap().is_none(), "(b) landed");
    assert!(
        !b_copy.exists(),
        "(b)'s copy in the old root is unlinked after the landing"
    );
    assert!(
        bridge
            .db
            .staged_payloads_for_signout()
            .unwrap()
            .iter()
            .all(|(path, _, _)| path != b_copy.to_string_lossy().as_ref()),
        "and its journal row is gone"
    );
}
