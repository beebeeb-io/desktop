//! Finder delete and trash interplay with cached writes, driven through the real
//! delete request and the real post-delete database states.

use super::*;

#[test]
fn a_real_finder_delete_then_the_same_create_uploads_the_new_file() {
    // Copy report.pdf in (Finder keeps the mtime, so the key is stable), delete
    // it, copy the same file back within the TTL: same key, but a NEW create.
    let fx = IpcFixture::start(|_| {});
    let src = tempfile::tempdir().unwrap();
    let path = source_file(&src, "report.pdf");
    fx.rt.block_on(async {
        let first = send_one(&fx, create_request("report.pdf", &path, Some("key-del-recreate"))).await;
        assert_write_queued(&first);
        let id = item_id(&first);
        let deleted = send_one(&fx, delete_request(&id)).await;
        assert!(
            deleted.get("WriteQueued").is_some(),
            "the delete must queue, got {deleted}"
        );
        let again = send_one(&fx, create_request("report.pdf", &path, Some("key-del-recreate"))).await;
        assert_write_queued(&again);
        assert_ne!(
            item_id(&again),
            id,
            "the re-created file must be a NEW item, not the trashed one"
        );
    });
    assert_eq!(
        operations_of_kind(&fx, OperationKind::UploadVersion).len(),
        2,
        "the re-created file must queue its own upload"
    );
    assert_eq!(operations_of_kind(&fx, OperationKind::TrashFile).len(), 1);
}

#[test]
fn a_create_after_the_trash_op_finished_still_uploads_the_new_file() {
    // After the server trash succeeds the op is REMOVED but the row is kept (see
    // the `TrashFile` arm in engine_bridge.rs), and the IPC delete path does not
    // mark it `Trashing`. Nothing in the row or the queue says "deleted" any
    // more, so only the delete itself forgetting the dedup entry can save this.
    let fx = IpcFixture::start(|_| {});
    let src = tempfile::tempdir().unwrap();
    let path = source_file(&src, "report.pdf");
    fx.rt.block_on(async {
        let first = send_one(&fx, create_request("report.pdf", &path, Some("key-after-trash"))).await;
        let id = item_id(&first);
        send_one(&fx, delete_request(&id)).await;
        for op in operations_of_kind(&fx, OperationKind::TrashFile) {
            fx.db.remove_operation(&op.op_id).unwrap(); // what a successful trash does
        }
        assert!(fx.db.get_file(&id).unwrap().is_some(), "the row outlives the trash op");
        let again = send_one(&fx, create_request("report.pdf", &path, Some("key-after-trash"))).await;
        assert_write_queued(&again);
        assert_ne!(item_id(&again), id);
    });
    assert_eq!(operations_of_kind(&fx, OperationKind::UploadVersion).len(), 2);
}

#[test]
fn a_cached_create_is_not_returned_while_a_trash_op_is_pending_for_its_row() {
    // A trash queued by any path other than the IPC delete (which also forgets
    // the entry) is caught by the row's pending `TrashFile` op.
    let fx = IpcFixture::start(|_| {});
    let src = tempfile::tempdir().unwrap();
    let path = source_file(&src, "report.pdf");
    fx.rt.block_on(async {
        let first = send_one(&fx, create_request("report.pdf", &path, Some("key-pending-trash"))).await;
        let id = item_id(&first);
        let now = 1_700_000_000;
        fx.db
            .enqueue_operation(&crate::state_db::PendingOperation {
                op_id: "trash-op".into(),
                kind: OperationKind::TrashFile,
                file_id: Some(id.clone()),
                parent_id: None,
                target_path: None,
                metadata_json: None,
                payload_path: None,
                base_version: None,
                base_object_version_id: None,
                attempts: 0,
                max_attempts: 25,
                next_retry_at: now,
                last_error: None,
                backup_source_key: None,
                created_at: now,
                updated_at: now,
            })
            .unwrap();
        let again = send_one(&fx, create_request("report.pdf", &path, Some("key-pending-trash"))).await;
        assert_write_queued(&again);
        assert_ne!(item_id(&again), id);
    });
    assert_eq!(operations_of_kind(&fx, OperationKind::UploadVersion).len(), 2);
}

#[test]
fn a_cached_create_is_not_returned_for_a_row_parked_trashing() {
    // `watcher::handle_delete` parks a locally-deleted row in `Trashing`.
    let fx = IpcFixture::start(|_| {});
    let src = tempfile::tempdir().unwrap();
    let path = source_file(&src, "report.pdf");
    fx.rt.block_on(async {
        let first = send_one(&fx, create_request("report.pdf", &path, Some("key-trashing"))).await;
        let id = item_id(&first);
        fx.db.set_status(&id, FileStatus::Trashing).unwrap();
        let again = send_one(&fx, create_request("report.pdf", &path, Some("key-trashing"))).await;
        assert_write_queued(&again);
        assert_ne!(item_id(&again), id);
    });
    assert_eq!(operations_of_kind(&fx, OperationKind::UploadVersion).len(), 2);
}

#[test]
fn a_cached_reply_reports_the_row_as_it_is_now_not_as_it_was() {
    // A >600 s copy times out, the upload then finalizes, and only then does the
    // system's backed-off retry arrive. The cached reply must carry the CURRENT
    // version and status, or the user's next edit sends a stale base version.
    let fx = IpcFixture::start(|_| {});
    let src = tempfile::tempdir().unwrap();
    let path = source_file(&src, "big.bin");
    fx.rt.block_on(async {
        let first = send_one(&fx, create_request("big.bin", &path, Some("key-fresh"))).await;
        let id = item_id(&first);
        assert_eq!(first["WriteQueued"]["item"]["status"], "uploading");
        // Upload finalization: the row becomes Local at a real server version,
        // stamped with the wall-clock second of the upload, as
        // the landing (`StateDb::apply_landing`) leaves it.
        fx.db
            .upsert_file(&FileEntry {
                file_id: id.clone(),
                path: "big.bin".into(),
                status: FileStatus::Local,
                size_bytes: 20,
                modified_at: 1_700_000_123,
                content_hash: None,
                remote_updated_at: 1_700_000_123,
                parent_id: None,
                item_kind: ItemKind::File,
            })
            .unwrap();
        let mut contract = fx.db.get_file_contract_state(&id).unwrap().unwrap();
        contract.current_version = 7;
        fx.db.set_file_contract_state(&contract).unwrap();
        let retry = send_one(&fx, create_request("big.bin", &path, Some("key-fresh"))).await;
        assert_eq!(item_id(&retry), id, "it is still the same item (a true retry)");
        // The create's op is still queued, so the item is reported as it is now: `uploading`,
        // the queue's status, not the row's (spec §5.4 row 1, §9.1).
        #[cfg(target_os = "macos")]
        assert_eq!(
            retry["WriteQueued"]["item"]["status"], "uploading",
            "status must be current: {retry}"
        );
        // The Linux arm does not mint, so no op carries a write id: the row's status.
        #[cfg(not(target_os = "macos"))]
        assert_eq!(
            retry["WriteQueued"]["item"]["status"], "local",
            "status must be current: {retry}"
        );
        assert_eq!(
            retry["WriteQueued"]["item"]["version_identifier"], "7:1700000123:20",
            "version must be current: {retry}"
        );
        // The create's op is still queued: under the create's token (spec §5.4 row 1).
        #[cfg(target_os = "macos")]
        assert_eq!(
            retry["WriteQueued"]["item"]["content_version"], first["WriteQueued"]["item"]["content_version"],
            "the queued create's bytes keep the name the first reply gave them: {retry}"
        );
        // The Linux arm does not mint: the reply keeps today's content version.
        #[cfg(not(target_os = "macos"))]
        assert_eq!(
            retry["WriteQueued"]["item"]["content_version"], "7",
            "the write base must be the current server version: {retry}"
        );
        assert_ne!(retry, first);
    });
    assert_eq!(operations_of_kind(&fx, OperationKind::UploadVersion).len(), 1);
}

/// Spec 2026-10-09 §7.2: a modify under a provisional id whose create has landed is replied
/// under that id. The system's retry of the same request (same `request_id`) is answered from
/// the remembered reply, brought up to date from the server file, and queues nothing again.
#[cfg(target_os = "macos")]
#[test]
fn a_repeated_modify_under_a_provisional_id_queues_one_upload() {
    const PROVISIONAL: &str = "3f2a9c1e-0000-4000-8000-0000000000d1";
    const SERVER: &str = "3f2a9c1e-0000-4000-8000-0000000000d2";
    let fx = IpcFixture::start(|db| {
        db.upsert_file(&FileEntry {
            file_id: SERVER.into(),
            path: "doc.txt".into(),
            status: FileStatus::Local,
            size_bytes: 3,
            modified_at: 1_700_000_100,
            content_hash: None,
            remote_updated_at: 1_700_000_100,
            parent_id: None,
            item_kind: ItemKind::File,
        })
        .unwrap();
        let mut contract = db.get_file_contract_state(SERVER).unwrap().unwrap();
        contract.current_version = 1;
        db.set_file_contract_state(&contract).unwrap();
        db.insert_alias_for_test(PROVISIONAL, SERVER, 1_700_000_100);
    });
    let src = tempfile::tempdir().unwrap();
    let path = source_file(&src, "doc.txt");
    let request = || {
        let line = modify_request(PROVISIONAL, "doc.txt", &path, Some("key-provisional-modify"));
        let mut request: serde_json::Value = serde_json::from_slice(&line).unwrap();
        request["QueueFinderModify"]["base_version_identifier"] = serde_json::json!("1");
        let mut line = serde_json::to_vec(&request).unwrap();
        line.push(b'\n');
        line
    };
    let first = fx.rt.block_on(send_one(&fx, request()));
    let repeat = fx.rt.block_on(send_one(&fx, request()));
    for reply in [&first, &repeat] {
        assert_eq!(
            reply["WriteQueued"]["item"]["identifier"],
            serde_json::json!(PROVISIONAL),
            "replied under the provisional id: {reply}"
        );
    }
    assert_eq!(
        repeat["WriteQueued"]["item"]["content_version"], first["WriteQueued"]["item"]["content_version"],
        "the same write"
    );
    let ops = operations_of_kind(&fx, OperationKind::UploadVersion);
    assert_eq!(ops.len(), 1, "one upload for one save: {ops:?}");
    assert_eq!(ops[0].file_id.as_deref(), Some(SERVER));
}

/// Spec 2026-10-09 §12 (4c): a create's repeat after its landing is unchanged, it runs
/// again. Only a modify's remembered reply is brought up to date through the alias: a create
/// reply must name the provider's identifier, never the provisional one.
#[cfg(target_os = "macos")]
#[test]
fn a_repeated_create_after_its_landing_runs_again() {
    const SERVER: &str = "3f2a9c1e-0000-4000-8000-0000000000d3";
    let fx = IpcFixture::start(|_| {});
    let src = tempfile::tempdir().unwrap();
    let path = source_file(&src, "new.txt");
    let first = fx.rt.block_on(send_one(
        &fx,
        create_request("new.txt", &path, Some("key-landed-create")),
    ));
    let provisional = first["WriteQueued"]["item"]["identifier"].as_str().unwrap().to_string();
    // The landing, as the queue leaves it: the provisional row is gone, the server row and
    // the alias exist.
    let mut row = fx.db.get_file(&provisional).unwrap().unwrap();
    fx.db.delete_file(&provisional).unwrap();
    row.file_id = SERVER.into();
    row.status = FileStatus::Local;
    fx.db.upsert_file(&row).unwrap();
    fx.db.insert_alias_for_test(&provisional, SERVER, 1_700_000_100);
    let repeat = fx.rt.block_on(send_one(
        &fx,
        create_request("new.txt", &path, Some("key-landed-create")),
    ));
    let identifier = repeat["WriteQueued"]["item"]["identifier"].as_str().unwrap();
    assert_ne!(
        identifier, provisional,
        "never answered under the provisional id: {repeat}"
    );
    assert_ne!(
        identifier, SERVER,
        "not answered from the alias either (spec §12): {repeat}"
    );
}

/// Spec 2026-10-09 §7.2 (I-1): the create arm hands a deletion-conflicted create's three
/// fields to the engine, which modifies the template's item instead of creating a second
/// file. macOS only: the other unix builds ignore the fields (an ordinary create).
#[cfg(target_os = "macos")]
#[test]
fn a_deletion_conflicted_create_over_the_socket_modifies_the_template_item() {
    const TEMPLATE: &str = "3f2a9c1e-0000-4000-8000-0000000000c1";
    let fx = IpcFixture::start(|db| {
        db.upsert_file(&FileEntry {
            file_id: TEMPLATE.into(),
            path: "t2.txt".into(),
            status: FileStatus::Local,
            size_bytes: 28,
            modified_at: 1_700_000_100,
            content_hash: None,
            remote_updated_at: 1_700_000_100,
            parent_id: None,
            item_kind: ItemKind::File,
        })
        .unwrap();
        let mut contract = db.get_file_contract_state(TEMPLATE).unwrap().unwrap();
        contract.current_version = 1;
        db.set_file_contract_state(&contract).unwrap();
    });
    let src = tempfile::tempdir().unwrap();
    let path = source_file(&src, "t2.txt");
    let mut request: serde_json::Value = serde_json::from_slice(&create_request("t2.txt", &path, None)).unwrap();
    request["QueueFinderCreate"]["deletion_conflicted"] = serde_json::json!(true);
    request["QueueFinderCreate"]["template_identifier"] = serde_json::json!(TEMPLATE);
    request["QueueFinderCreate"]["template_content_version"] = serde_json::json!("1");
    let mut line = serde_json::to_vec(&request).unwrap();
    line.push(b'\n');
    let reply = fx.rt.block_on(send_one(&fx, line));
    assert_eq!(
        reply["WriteQueued"]["item"]["identifier"],
        serde_json::json!(TEMPLATE),
        "the reply names the template's item: {reply}"
    );
    let ops = operations_of_kind(&fx, OperationKind::UploadVersion);
    assert_eq!(ops.len(), 1, "{reply}");
    assert_eq!(
        ops[0].file_id.as_deref(),
        Some(TEMPLATE),
        "a content modify of the template's item"
    );
    assert_eq!(fx.db.list_files().unwrap().len(), 1, "no second file");
}

#[test]
fn a_queued_modify_replies_with_the_size_of_the_bytes_it_was_handed() {
    // The system keeps the bytes it handed over and does not fetch them back,
    // so the item in the reply must describe THOSE bytes, not the row's
    // previous content (28 bytes here; the edit is 40).
    let fx = IpcFixture::start(|db| {
        db.upsert_file(&FileEntry {
            file_id: "7a000000-0000-4000-8000-000000000001".into(),
            path: "t.txt".into(),
            status: FileStatus::Local,
            size_bytes: 28,
            modified_at: 1_700_000_100,
            content_hash: None,
            remote_updated_at: 1_700_000_100,
            parent_id: None,
            item_kind: ItemKind::File,
        })
        .unwrap();
        let mut contract = db
            .get_file_contract_state("7a000000-0000-4000-8000-000000000001")
            .unwrap()
            .unwrap();
        contract.current_version = 1;
        db.set_file_contract_state(&contract).unwrap();
    });
    let src = tempfile::tempdir().unwrap();
    let path = src.path().join("t.txt");
    std::fs::write(&path, b"twenty-eight bytes of text.\nmore-bytes12").unwrap();
    let path = path.to_string_lossy().into_owned();
    let before = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let reply = fx.rt.block_on(send_one(
        &fx,
        modify_request_on_base("7a000000-0000-4000-8000-000000000001", "t.txt", &path, "1"),
    ));
    let item = &reply["WriteQueued"]["item"];
    let ops = operations_of_kind(&fx, OperationKind::UploadVersion);
    assert_eq!(ops.len(), 1, "{reply}");
    let staged = std::fs::metadata(ops[0].payload_path.as_deref().unwrap())
        .unwrap()
        .len() as i64;
    assert_eq!(staged, 40);
    assert_eq!(
        item["size_bytes"],
        serde_json::json!(staged),
        "the reply must describe the staged bytes: {reply}"
    );
    assert!(
        item["modified_at"].as_i64().unwrap() >= before,
        "the reply's modification time is the write's, not the previous content's: {reply}"
    );
    assert_eq!(item["status"], "uploading", "{reply}");
    let content_version = item["content_version"].as_str().unwrap();
    #[cfg(target_os = "macos")]
    assert!(
        crate::write_token::parse_token(content_version).is_some_and(|token| token.base == 1),
        "the reply names the bytes it accepted with a token led by their base: {reply}"
    );
    // The Linux arm does not mint: the reply keeps today's content version.
    #[cfg(not(target_os = "macos"))]
    assert_eq!(content_version, "1", "{reply}");
    let row = fx.db.get_file("7a000000-0000-4000-8000-000000000001").unwrap().unwrap();
    assert_eq!(row.size_bytes, 40, "the row records the staged size too");
}
