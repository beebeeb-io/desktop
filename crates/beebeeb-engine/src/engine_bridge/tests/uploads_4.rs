//! `uploads_4` engine tests, split out of `engine_bridge.rs`.

use super::*;
/// Poll the board until `file_id` shows at least `done` bytes, or give up.
async fn wait_for_upload_bytes(
    board: &Arc<crate::transfer_progress::TransferBoard>,
    file_id: &str,
    done: u64,
    within: Duration,
) -> Option<crate::transfer_progress::Transfer> {
    let started = std::time::Instant::now();
    while started.elapsed() < within {
        if let Some(t) = board.get(file_id)
            && t.done >= done
        {
            return Some(t);
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    None
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_upload_reports_its_bytes_chunk_by_chunk_and_a_cut_one_leaves_nothing() {
    // 20 bytes in chunks of 8 (8 + 8 + 4). Chunk 1 hangs, so after chunk 0 is
    // acknowledged the upload sits at 8 of 20: the moment a popover would look.
    let dir = tempfile::tempdir().unwrap();
    let payload = dir.path().join("resume.bin");
    std::fs::write(&payload, b"0123456789abcdefghij").unwrap();
    let server = ResumableUploadMock::start(ResumeMockMode::HangOnce);
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), [21u8; 32]);
    bridge.db.enqueue_operation(&resumable_create_op(&payload)).unwrap();
    let board = bridge.transfers().clone();

    let (cut, seen) = tokio::join!(
        tokio::time::timeout(
            Duration::from_millis(1500),
            bridge.process_due_operations(dir.path(), 200)
        ),
        wait_for_upload_bytes(&board, "local-file-resume", 8, Duration::from_millis(1400)),
    );
    assert!(cut.is_err(), "the first pass is cut mid-upload");
    let seen = seen.expect("the board showed the upload after chunk 0 was acknowledged");
    assert_eq!(
        seen,
        crate::transfer_progress::Transfer {
            direction: crate::transfer_progress::Direction::Up,
            done: 8,
            total: 20,
            ..crate::transfer_progress::Transfer::default()
        }
    );
    assert!(board.active().is_empty(), "a cut upload must not linger as in flight");
    assert_eq!(board.finished_bytes(), 0, "a cut upload finishes no bytes");

    let outcome = bridge.process_due_operations(dir.path(), 200).await.unwrap();
    assert_eq!(outcome.completed_op_ids, vec!["op-upload-resume".to_string()]);
    server.finish();
    assert_eq!(
        board.finished_bytes(),
        20,
        "the resumed upload finishes the whole 20 bytes"
    );
    assert!(board.active().is_empty());
}

/// The 0.8.x upgrade case: a session an older build opened (v1 frames on the server, no stored
/// completion name) with some of its chunks acknowledged. This build writes v2, and a v2 writer never
/// continues a v1 stream: frames of two formats in one session are a file that completes and never
/// opens. The session is abandoned on the server and the file is uploaded whole, as v2, in a fresh
/// one. Mutant: `resume_step` taking "some chunks acknowledged" for "all of them" (the op then fails
/// on the body's guard and never completes); a body without that guard PUTs into `session-old`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_persisted_v1_session_with_acknowledged_chunks_is_never_continued() {
    const OLD_FILE: &str = "40000000-0000-4000-8000-0000000000f5";
    let dir = tempfile::tempdir().unwrap();
    let payload = dir.path().join("resume.bin");
    std::fs::write(&payload, b"0123456789abcdefghij").unwrap();
    let server = ResumableUploadMock::start(ResumeMockMode::Plain);
    let master_key = [21u8; 32];
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
    let op = resumable_create_op(&payload);
    bridge.db.enqueue_operation(&op).unwrap();
    // A previous build acknowledged chunk 0 (8 bytes, a v1 frame) of its session and was replaced.
    bridge
        .db
        .put_upload_resume(&UploadResume {
            op_id: op.op_id.clone(),
            payload_path: payload.to_string_lossy().into_owned(),
            payload_size: 20,
            payload_mtime_ns: payload_mtime_ns(&payload),
            upload_session_id: "session-old".into(),
            server_file_id: OLD_FILE.into(),
            object_version_id: "object-init-old".into(),
            chunk_size_bytes: 8,
            chunk_count: 3,
            acked_chunks: 1,
            metadata_applied: true,
            is_create: true,
            self_minted: true,
            completed_version: None,
            completed_object_version_id: None,
            completed_mime_type: None,
        })
        .unwrap();
    assert!(bridge.db.upload_completion_name(&op.op_id).unwrap().is_none());

    let outcome = bridge.process_due_operations(dir.path(), 200).await.unwrap();
    assert_eq!(outcome.completed_op_ids, vec![op.op_id.clone()]);

    let requests = server.finish();
    assert_eq!(
        requests.iter().filter(|r| r.path.contains("session-old")).count(),
        0,
        "nothing is ever sent into the old session: no chunk, no complete"
    );
    assert_eq!(
        count_requests(&requests, "POST", &format!("/api/v1/files/{OLD_FILE}/upload/abandon")),
        1,
        "the old session is abandoned on the server"
    );
    assert_eq!(count_requests(&requests, "POST", "/api/v1/uploads/init"), 1);
    for index in 0..3 {
        assert_eq!(
            count_requests(&requests, "PUT", &format!("/api/v1/uploads/session-1/chunks/{index}")),
            1,
            "chunk {index} is sent once into the fresh session, chunk 0 included"
        );
    }
    assert_eq!(
        count_requests(&requests, "POST", "/api/v1/uploads/session-1/complete"),
        1
    );
    // The whole file is there as v2: every frame opens against the commitment `complete` published.
    let item_id = init_item_id(&requests);
    let file_key = hydration_test_key(master_key, &item_id);
    let (record, _) = uploaded_record(&requests, &file_key);
    assert_eq!(record.format, beebeeb_core::chunk_stream::ExpectedChunkFormat::V2);
    assert_eq!(
        uploaded_plaintext(&requests, &file_key, &item_id),
        b"0123456789abcdefghij"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn flow7_gone_session_abandons_orphan_then_reuploads_once() {
    let dir = tempfile::tempdir().unwrap();
    let payload = dir.path().join("resume.bin");
    std::fs::write(&payload, b"0123456789abcdefghij").unwrap();
    let server = ResumableUploadMock::start(ResumeMockMode::HangOnceThenSessionGone);
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), [22u8; 32]);
    bridge.db.enqueue_operation(&resumable_create_op(&payload)).unwrap();

    let cut = tokio::time::timeout(
        Duration::from_millis(1500),
        bridge.process_due_operations(dir.path(), 200),
    )
    .await;
    assert!(cut.is_err(), "the first pass must be cut mid-upload");
    assert!(bridge.db.get_upload_resume("op-upload-resume").unwrap().is_some());

    // Second pass: a cut session is never continued (chunk format v2), so whether the server
    // still has it does not matter: the orphan create row is abandoned and a fresh session
    // uploads the file exactly once.
    let outcome = bridge.process_due_operations(dir.path(), 200).await.unwrap();
    assert_eq!(outcome.completed_op_ids, vec!["op-upload-resume".to_string()]);
    assert!(bridge.db.get_upload_resume("op-upload-resume").unwrap().is_none());

    let requests = server.finish();
    assert_eq!(count_requests(&requests, "POST", "/api/v1/uploads/init"), 2);
    let init_ids: Vec<String> = requests
        .iter()
        .filter(|r| r.method == "POST" && r.path == "/api/v1/uploads/init")
        .map(|r| {
            serde_json::from_slice::<serde_json::Value>(&r.body).unwrap()["file_id"]
                .as_str()
                .expect("every init carries the id this device chose")
                .to_string()
        })
        .collect();
    let first_id = &init_ids[0];
    let second_id = &init_ids[1];
    assert_ne!(first_id, second_id, "each fresh create attempt proposes its own id");
    assert_eq!(
        count_requests(&requests, "POST", &format!("/api/v1/files/{first_id}/upload/abandon")),
        1,
        "the orphaned is_uploading row from the dead session must be abandoned"
    );
    assert_eq!(
        count_requests(&requests, "DELETE", &format!("/api/v1/files/{first_id}")),
        0,
        "abandoning a create session must never trash the item"
    );
    assert_eq!(
        count_requests(&requests, "POST", "/api/v1/uploads/session-2/complete"),
        1
    );
    assert!(bridge.db.get_file(second_id).unwrap().is_some());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn flow7_changed_payload_does_not_resume_stale_session() {
    let dir = tempfile::tempdir().unwrap();
    let payload = dir.path().join("resume.bin");
    std::fs::write(&payload, b"0123456789abcdefghij").unwrap();
    let server = ResumableUploadMock::start(ResumeMockMode::HangOnce);
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), [23u8; 32]);
    bridge.db.enqueue_operation(&resumable_create_op(&payload)).unwrap();

    let cut = tokio::time::timeout(
        Duration::from_millis(1500),
        bridge.process_due_operations(dir.path(), 200),
    )
    .await;
    assert!(cut.is_err());

    // The staged payload changes size before the retry: the persisted
    // chunks belong to other bytes, so the session must NOT be resumed.
    std::fs::write(&payload, b"0123456789abcdefghijKLMN").unwrap();
    let outcome = bridge.process_due_operations(dir.path(), 200).await.unwrap();
    assert_eq!(outcome.completed_op_ids, vec!["op-upload-resume".to_string()]);

    let requests = server.finish();
    assert_eq!(count_requests(&requests, "POST", "/api/v1/uploads/init"), 2);
    let first_id = init_item_id(&requests);
    assert_eq!(
        count_requests(&requests, "POST", &format!("/api/v1/files/{first_id}/upload/abandon")),
        1,
        "the stale create session is abandoned, not resumed"
    );
    assert_eq!(
        count_requests(&requests, "DELETE", &format!("/api/v1/files/{first_id}")),
        0,
        "abandoning a create session must never trash the item"
    );
    assert_eq!(
        count_requests(&requests, "POST", "/api/v1/uploads/session-1/complete"),
        0
    );
    assert_eq!(
        count_requests(&requests, "POST", "/api/v1/uploads/session-2/complete"),
        1
    );
}

#[tokio::test]
async fn flow7_given_up_create_upload_abandons_its_orphan_row() {
    let dir = tempfile::tempdir().unwrap();
    let payload = dir.path().join("payload.txt");
    std::fs::write(&payload, b"retry me").unwrap();
    let server = UploadMockServer::start(true);
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), [24u8; 32]);
    let mut op = resumable_create_op(&payload);
    op.max_attempts = 1;
    bridge.db.enqueue_operation(&op).unwrap();

    let outcome = bridge.process_due_operations(dir.path(), 200).await.unwrap();
    assert_eq!(outcome.retried_op_ids, vec!["op-upload-resume".to_string()]);
    assert!(
        bridge.db.list_due_operations(i64::MAX).unwrap().is_empty(),
        "op exhausted its retries"
    );
    assert!(bridge.db.get_upload_resume("op-upload-resume").unwrap().is_none());

    let requests = server.finish();
    let item_id = init_item_id(&requests);
    // init, the refused chunk, the abandon (no metadata update: the name travels with `complete`).
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[1].path, "/api/v1/uploads/upload-session-1/chunks/0");
    assert_eq!(requests[2].method, "POST");
    assert_eq!(requests[2].path, format!("/api/v1/files/{item_id}/upload/abandon"));
    assert_eq!(
        count_requests(&requests, "DELETE", &format!("/api/v1/files/{item_id}")),
        0,
        "abandoning a create session must never trash the item"
    );
}

#[test]
fn flow7_snapshot_skips_new_is_uploading_rows() {
    let dir = tempfile::tempdir().unwrap();
    let bridge = test_bridge(&dir.path().join("state.db"));
    let mut conflicts = Vec::new();
    let row = serde_json::json!({
        "id": "partial-upload-1",
        "parent_id": null,
        "name": "resume.bin",
        "size_bytes": 50331655,
        "is_folder": false,
        "is_uploading": true,
        "updated_at": 100
    });
    let resolved = process_metadata_row(&bridge, &row, "", 200, RowSource::Snapshot, &mut conflicts).unwrap();
    assert!(resolved.is_none());
    assert!(
        bridge.db.get_file("partial-upload-1").unwrap().is_none(),
        "an in-progress upload has no content yet and must not become a placeholder"
    );
}

// ── #59 (flow 7): a remote rename/move on a row this device holds (`Local`) ──
// Ported from desktop #59: a rename/move is metadata-only, so on a Local row it
// always applies the new path and parent, never re-anchors `remote_updated_at`,
// never raises a content conflict and never blanks the path.

fn flow7_seed_local_with_hash(bridge: &EngineBridge, file_id: &str, path: &str) {
    seed_bridge_entry(bridge, file_id, path, None, FileStatus::Local, false, 10);
    let mut row = bridge.db().get_file(file_id).unwrap().unwrap();
    row.content_hash = Some("local-hash-abc".into());
    bridge.db().upsert_file(&row).unwrap();
}

fn flow7_apply_batch(
    bridge: &EngineBridge,
    root: &Path,
    ops: &[(i64, &str, serde_json::Value)],
) -> Vec<ConflictDetected> {
    let mut conflicts = Vec::new();
    for (seq_id, op_type, payload) in ops {
        let op = crate::api_client::SyncOp {
            seq_id: *seq_id,
            op_type: (*op_type).into(),
            payload: payload.clone(),
        };
        apply_sync_op(bridge, root, &op, 200, &mut conflicts).unwrap();
    }
    conflicts
}

#[test]
fn flow7_two_renames_in_one_batch_land_final_name_on_local_row() {
    let dir = tempfile::tempdir().unwrap();
    let mk = [9u8; 32];
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), "http://placeholder".into(), mk);
    let file = "dbl00000-0000-4000-8000-000000000001";
    flow7_seed_local_with_hash(&bridge, file, "draft.txt");

    let conflicts = flow7_apply_batch(
        &bridge,
        dir.path(),
        &[
            (
                21,
                "file_rename",
                serde_json::json!({ "id": file, "new_name_encrypted": enc_name(&mk, file, "draft-v2.txt") }),
            ),
            (
                22,
                "file_rename",
                serde_json::json!({ "id": file, "new_name_encrypted": enc_name(&mk, file, "final.txt") }),
            ),
        ],
    );

    let row = bridge.db().get_file(file).unwrap().unwrap();
    assert_eq!(
        row.path, "final.txt",
        "second rename in the same batch must not be dropped"
    );
    assert_eq!(row.status, FileStatus::Local, "a rename is metadata-only");
    assert_eq!(row.content_hash.as_deref(), Some("local-hash-abc"));
    assert!(
        conflicts.is_empty(),
        "a metadata-only op never raises a content conflict"
    );
}

#[test]
fn flow7_move_then_rename_in_one_batch_lands_under_dest_on_local_row() {
    let dir = tempfile::tempdir().unwrap();
    let mk = [9u8; 32];
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), "http://placeholder".into(), mk);
    let dest = "dst00000-0000-4000-8000-000000000001";
    let file = "mvr00000-0000-4000-8000-000000000002";
    seed_bridge_entry(&bridge, dest, "Dest", None, FileStatus::CloudOnly, true, 10);
    flow7_seed_local_with_hash(&bridge, file, "draft.txt");

    let conflicts = flow7_apply_batch(
        &bridge,
        dir.path(),
        &[
            (
                31,
                "file_move",
                serde_json::json!({ "id": file, "new_parent_id": dest }),
            ),
            (
                32,
                "file_rename",
                serde_json::json!({ "id": file, "new_name_encrypted": enc_name(&mk, file, "n2.txt") }),
            ),
        ],
    );

    let row = bridge.db().get_file(file).unwrap().unwrap();
    assert_eq!(row.path, "Dest/n2.txt", "move then rename in one batch must both apply");
    assert_eq!(row.status, FileStatus::Local);
    assert_eq!(row.content_hash.as_deref(), Some("local-hash-abc"));
    let contract = bridge.db().get_file_contract_state(file).unwrap().unwrap();
    assert_eq!(contract.parent_id.as_deref(), Some(dest));
    assert!(conflicts.is_empty());
}

#[test]
fn flow7_rename_on_locally_edited_local_row_moves_path_without_conflict() {
    let dir = tempfile::tempdir().unwrap();
    let mk = [9u8; 32];
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), "http://placeholder".into(), mk);
    let file = "edt00000-0000-4000-8000-000000000003";
    flow7_seed_local_with_hash(&bridge, file, "notes.txt");

    let conflicts = flow7_apply_batch(
        &bridge,
        dir.path(),
        &[(
            41,
            "file_rename",
            serde_json::json!({ "id": file, "new_name_encrypted": enc_name(&mk, file, "notes-renamed.txt") }),
        )],
    );

    let row = bridge.db().get_file(file).unwrap().unwrap();
    assert_eq!(row.path, "notes-renamed.txt");
    assert_eq!(row.status, FileStatus::Local);
    assert_eq!(
        row.remote_updated_at, 10,
        "a rename must not re-anchor the content freshness clock"
    );
    assert!(conflicts.is_empty());
}

#[test]
fn flow7_rename_with_undecryptable_name_keeps_path_and_schedules_resnapshot() {
    let dir = tempfile::tempdir().unwrap();
    let mk = [9u8; 32];
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), "http://placeholder".into(), mk);
    let file = "bad00000-0000-4000-8000-000000000004";
    flow7_seed_local_with_hash(&bridge, file, "notes.txt");
    // Drain any flag left by seeding so the assertion below is caused by this op.
    bridge.db().take_needs_resnapshot().unwrap();

    let conflicts = flow7_apply_batch(
        &bridge,
        dir.path(),
        &[(
            51,
            "file_rename",
            serde_json::json!({ "id": file, "new_name_encrypted": "not-a-valid-encrypted-blob" }),
        )],
    );

    let row = bridge.db().get_file(file).unwrap().unwrap();
    assert_eq!(row.path, "notes.txt", "an undecryptable name must never blank the path");
    assert_eq!(row.status, FileStatus::Local);
    assert_eq!(row.content_hash.as_deref(), Some("local-hash-abc"));
    assert_eq!(row.remote_updated_at, 10);
    assert!(
        bridge.db().take_needs_resnapshot().unwrap(),
        "an unresolvable name schedules a re-snapshot so the real name is recovered"
    );
    assert!(conflicts.is_empty());
    // dc-136b: the row says its name could not be read, and the window shows it with the kept name.
    assert!(bridge.db().name_unreadable(file).unwrap());
    let item = crate::drive_v2::model::item_for_entry(bridge.db(), &row).unwrap();
    assert_eq!(item.name, "notes.txt");
    assert_eq!(item.name_unreadable, Some(true));

    // The same unreadable name again (a re-snapshot still lists it) asks for no further re-snapshot.
    flow7_apply_batch(
        &bridge,
        dir.path(),
        &[(
            52,
            "file_rename",
            serde_json::json!({ "id": file, "new_name_encrypted": "not-a-valid-encrypted-blob" }),
        )],
    );
    assert!(
        !bridge.db().take_needs_resnapshot().unwrap(),
        "a name that stays unreadable is no loop"
    );

    // A name that decrypts (the keys changed, another client fixed it) is applied and clears the mark.
    flow7_apply_batch(
        &bridge,
        dir.path(),
        &[(
            53,
            "file_rename",
            serde_json::json!({ "id": file, "new_name_encrypted": enc_name(&mk, file, "notes-fixed.txt") }),
        )],
    );
    let row = bridge.db().get_file(file).unwrap().unwrap();
    assert_eq!(row.path, "notes-fixed.txt");
    assert!(!bridge.db().name_unreadable(file).unwrap());
}

// ── Backlog 2351: no uploaded chunk is larger than 32 MiB ──

/// Mock whose `init` announces `chunk_size` and a chunk count for `total` bytes; every chunk PUT is
/// recorded and acknowledged. Serves `complete` the way the resumable mock does.
struct CapMock {
    base_url: String,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    stop: Arc<std::sync::atomic::AtomicBool>,
    handle: thread::JoinHandle<()>,
}

impl CapMock {
    fn start(chunk_size: u64, total: u64) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (server_requests, server_stop) = (Arc::clone(&requests), Arc::clone(&stop));
        let handle = thread::spawn(move || {
            let started = std::time::Instant::now();
            while !server_stop.load(Ordering::SeqCst) && started.elapsed() < Duration::from_secs(90) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream.set_nonblocking(false).unwrap();
                        let request = read_http_request(&mut stream);
                        let response = Self::answer(&request, chunk_size, total);
                        server_requests.lock().unwrap().push(request);
                        let _ = stream.write_all(response.as_bytes());
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5))
                    }
                    Err(e) => panic!("cap mock accept failed: {e}"),
                }
            }
        });
        Self {
            base_url,
            requests,
            stop,
            handle,
        }
    }

    fn answer(request: &RecordedRequest, chunk_size: u64, total: u64) -> String {
        let path = request.path.as_str();
        if request.method == "POST" && path == "/api/v1/uploads/init" {
            let file_id = serde_json::from_slice::<serde_json::Value>(&request.body).unwrap()["file_id"]
                .as_str()
                .unwrap()
                .to_string();
            return http_json(
                "201 Created",
                serde_json::json!({
                    "file_id": file_id, "tenant_id": "tenant-1", "object_version_id": "object-init-1",
                    "upload_session_id": "session-1", "chunk_size_bytes": chunk_size as i64,
                    "chunk_count": total.div_ceil(chunk_size) as i64, "storage_format_version": 2,
                    "storage_pool_id": "pool-1", "region": "local"
                }),
            );
        }
        if request.method == "PUT" && path.starts_with("/api/v1/uploads/session-1/chunks/") {
            let index: u32 = path.rsplit('/').next().unwrap().parse().unwrap();
            return http_json(
                "200 OK",
                serde_json::json!({ "index": index, "size": request.body.len() as i64, "skipped": false }),
            );
        }
        if request.method == "POST" && path == "/api/v1/uploads/session-1/complete" {
            let mut answer = serde_json::json!({
                "file_id": "server-file-1", "version_number": 1,
                "current_object_version_id": "object-complete-1",
                "size_bytes": total as i64, "mime_type": "application/octet-stream"
            });
            if let Some(name) = completion_name(request) {
                answer["name_encrypted"] = serde_json::json!(name);
            }
            return http_json("200 OK", answer);
        }
        http_json("200 OK", serde_json::json!({ "ok": true }))
    }

    fn finish(self) -> Vec<RecordedRequest> {
        self.stop.store(true, Ordering::SeqCst);
        self.handle.join().unwrap();
        self.requests.lock().unwrap().clone()
    }
}

/// A 70 MiB file whose server plan is 32 MiB chunks (32 + 32 + 6): every PUT body carries at most
/// 32 MiB of plaintext (plus the frame overhead), and the frames open back to the exact bytes.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_seeded_upload_writes_chunks_of_at_most_32_mib() {
    const MIB: u64 = 1024 * 1024;
    let total = 70 * MIB;
    let dir = tempfile::tempdir().unwrap();
    let payload = dir.path().join("resume.bin");
    let bytes: Vec<u8> = (0..total).map(|i| (i % 251) as u8).collect();
    std::fs::write(&payload, &bytes).unwrap();
    let server = CapMock::start(32 * MIB, total);
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), [25u8; 32]);
    bridge.db.enqueue_operation(&resumable_create_op(&payload)).unwrap();

    let outcome = bridge.process_due_operations(dir.path(), 200).await.unwrap();
    assert_eq!(outcome.completed_op_ids, vec!["op-upload-resume".to_string()]);

    let requests = server.finish();
    let sizes: Vec<u64> = requests
        .iter()
        .filter(|r| r.method == "PUT" && r.path.contains("/chunks/"))
        .map(|r| r.body.len() as u64)
        .collect();
    assert!(
        sizes.len() >= 3,
        "70 MiB cannot fit in fewer than three 32 MiB chunks: {sizes:?}"
    );
    let overhead = beebeeb_core::chunk_stream::CHUNK_OVERHEAD;
    for size in &sizes {
        assert!(
            *size <= 32 * MIB + overhead,
            "a chunk of {size} bytes exceeds 32 MiB of plaintext plus its {overhead}-byte frame"
        );
    }
}
