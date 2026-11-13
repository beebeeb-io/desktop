//! `uploads_3` engine tests, split out of `engine_bridge.rs`.

use super::*;
#[tokio::test]
async fn test_process_due_operations_preserves_payload_when_upload_chunk_fails() {
    let dir = tempfile::tempdir().unwrap();
    let payload = dir.path().join("payload.txt");
    std::fs::write(&payload, b"retry me").unwrap();
    let server = UploadMockServer::start(true);
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), [12u8; 32]);
    bridge
        .db
        .enqueue_operation(&PendingOperation {
            op_id: "op-upload-retry".into(),
            kind: OperationKind::UploadVersion,
            file_id: Some("78cd3eb2-6aee-527c-ad1c-9130e4ed62ad".into()),
            parent_id: None,
            target_path: Some("retry.txt".into()),
            metadata_json: Some(
                serde_json::json!({
                    "operation": "create_file",
                    "name_encrypted": "{\"cipher_suite\":\"V1Aes256Gcm\"}",
                    "display_name": "retry.txt",
                    "content_type": "text/plain"
                })
                .to_string(),
            ),
            payload_path: Some(payload.to_string_lossy().into_owned()),
            base_version: None,
            base_object_version_id: None,
            attempts: 0,
            max_attempts: 5,
            next_retry_at: 0,
            last_error: None,
            backup_source_key: None,
            created_at: 100,
            updated_at: 100,
        })
        .unwrap();

    let outcome = bridge.process_due_operations(dir.path(), 200).await.unwrap();
    assert_eq!(outcome.retried_op_ids, vec!["op-upload-retry".to_string()]);
    assert!(outcome.completed_op_ids.is_empty());
    assert!(payload.exists(), "failed uploads must preserve the staged payload");

    let due_too_early = bridge.db.list_due_operations(229).unwrap();
    assert!(due_too_early.is_empty());
    let due = bridge.db.list_due_operations(230).unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].attempts, 1);
    assert!(
        due[0]
            .last_error
            .as_deref()
            .unwrap_or("")
            .contains("500 Internal Server Error")
    );

    let requests = server.finish();
    let item_id = init_item_id(&requests);
    // No metadata update: this create moves nothing, and its name travels with `complete`.
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].path, "/api/v1/uploads/init");
    assert_eq!(
        count_requests(&requests, "PATCH", &format!("/api/v1/files/{item_id}")),
        0
    );
    assert_eq!(requests[1].path, "/api/v1/uploads/upload-session-1/chunks/0");
}

// ── Transfer progress + activity (task 1683 slice 2) ─────────────────────

fn slice2_upload_op(op_id: &str, file_id: &str, target: &str, payload: &Path) -> PendingOperation {
    PendingOperation {
        op_id: op_id.into(),
        kind: OperationKind::UploadVersion,
        file_id: Some(file_id.into()),
        parent_id: None,
        target_path: Some(target.into()),
        metadata_json: Some(
            serde_json::json!({
                "operation": "create_file",
                "name_encrypted": "{\"cipher_suite\":\"V1Aes256Gcm\"}",
                "display_name": "report.txt",
                "content_type": "text/plain"
            })
            .to_string(),
        ),
        payload_path: Some(payload.to_string_lossy().into_owned()),
        base_version: None,
        base_object_version_id: None,
        attempts: 0,
        max_attempts: 5,
        next_retry_at: 0,
        last_error: None,
        backup_source_key: None,
        created_at: 100,
        updated_at: 100,
    }
}

#[tokio::test]
async fn a_finished_upload_adds_its_bytes_to_the_batch_and_records_an_up_row() {
    let dir = tempfile::tempdir().unwrap();
    let payload = dir.path().join("payload.txt");
    std::fs::write(&payload, b"live upload payload").unwrap();
    let server = UploadMockServer::start(false);
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), [11u8; 32]);
    bridge
        .db
        .enqueue_operation(&slice2_upload_op(
            "op-1",
            "local-file-1",
            "Reports/report.txt",
            &payload,
        ))
        .unwrap();

    let outcome = bridge.process_due_operations(dir.path(), 200).await.unwrap();
    assert_eq!(outcome.completed_op_ids, vec!["op-1".to_string()]);
    let requests = server.finish();
    let item_id = init_item_id(&requests);

    assert_eq!(bridge.transfers().finished_bytes(), 19, "the payload is 19 bytes");
    assert!(
        bridge.transfers().active().is_empty(),
        "nothing is in flight after the upload"
    );
    let rows = bridge.db.list_recent_transfer_activity(5).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].direction, "up");
    assert_eq!(rows[0].file_name, "report.txt");
    assert_eq!(rows[0].rel_path.as_deref(), Some("Reports/report.txt"));
    assert_eq!(rows[0].bytes, 19);
    assert_eq!(rows[0].file_id.as_deref(), Some(item_id.as_str()));
}

#[tokio::test]
async fn a_failed_upload_leaves_nothing_on_the_board_and_no_activity_row() {
    let dir = tempfile::tempdir().unwrap();
    let payload = dir.path().join("payload.txt");
    std::fs::write(&payload, b"retry me").unwrap();
    let server = UploadMockServer::start(true);
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), [12u8; 32]);
    bridge
        .db
        .enqueue_operation(&slice2_upload_op(
            "op-2",
            "78cd3eb2-6aee-527c-ad1c-9130e4ed62ad",
            "retry.txt",
            &payload,
        ))
        .unwrap();

    let outcome = bridge.process_due_operations(dir.path(), 200).await.unwrap();
    assert_eq!(outcome.retried_op_ids, vec!["op-2".to_string()]);
    server.finish();

    assert!(
        bridge.transfers().active().is_empty(),
        "a failed upload must not linger as in flight"
    );
    assert_eq!(
        bridge.transfers().finished_bytes(),
        0,
        "a failed upload finishes no bytes"
    );
    assert!(bridge.db.list_recent_transfer_activity(5).unwrap().is_empty());
}

#[test]
fn a_hydrate_reports_per_file_bytes_and_records_a_down_row() {
    let dir = tempfile::tempdir().unwrap();
    let master_key = [9u8; 32];
    let file_key = hydration_test_key(master_key, TEST_FILE_ID);
    let chunks: Vec<Vec<u8>> = vec![vec![b'a'; 10], vec![b'b'; 10], vec![b'c'; 10]];
    // 1 metadata GET + 3 chunk GETs.
    // + 1 discovery GET (task 2173): this server has no direct route (a JSON 404), so the API path serves it.
    let server = HydrationMockServer::start(file_key, chunks, 5);
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
    seed_bridge_row(
        &bridge,
        TEST_FILE_ID,
        "Photos/three-chunks.bin",
        None,
        FileStatus::CloudOnly,
        30,
    );
    let dest = dir.path().join("three-chunks.bin");

    // Read the board from inside the caller's own progress callback: that is the
    // moment a popover snapshot would see mid-transfer.
    let seen: Arc<Mutex<Vec<(u64, u64, u64, u64)>>> = Arc::new(Mutex::new(Vec::new()));
    let board = bridge.transfers().clone();
    let seen_in = seen.clone();
    let progress = move |done: u64, total: u64| {
        let on_board = board.get(TEST_FILE_ID).expect("in flight while reporting");
        seen_in
            .lock()
            .unwrap()
            .push((done, total, on_board.done, on_board.total));
    };
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(async {
            bridge
                .hydrate_file_with_progress(TEST_FILE_ID, &dest, &[dir.path()], Some(&progress))
                .await
        })
        .unwrap();
    server.finish();

    assert_eq!(
        *seen.lock().unwrap(),
        vec![(0, 30, 0, 30), (10, 30, 10, 30), (20, 30, 20, 30), (30, 30, 30, 30)],
        "the caller still gets every report, and the board already holds the same bytes and the same total"
    );
    assert!(bridge.transfers().active().is_empty());
    assert_eq!(bridge.transfers().finished_bytes(), 30);
    let rows = bridge.db.list_recent_transfer_activity(5).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(
        (rows[0].direction.as_str(), rows[0].file_name.as_str()),
        ("down", "three-chunks.bin")
    );
    assert_eq!(rows[0].bytes, 30);
}

#[test]
fn a_hydrate_without_a_caller_callback_still_fills_the_board() {
    let dir = tempfile::tempdir().unwrap();
    let master_key = [9u8; 32];
    let file_key = hydration_test_key(master_key, TEST_FILE_ID);
    // 1 metadata GET + 1 chunk GET
    // + 1 discovery GET (task 2173): this server has no direct route (a JSON 404), so the API path serves it.
    let server = HydrationMockServer::start(file_key, vec![vec![b'x'; 12]], 3);
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
    seed_bridge_row(&bridge, TEST_FILE_ID, "one.bin", None, FileStatus::CloudOnly, 12);
    let dest = dir.path().join("one.bin");
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(async { bridge.hydrate_file(TEST_FILE_ID, &dest, &[dir.path()]).await })
        .unwrap();
    server.finish();
    assert_eq!(bridge.transfers().finished_bytes(), 12);
}

// ── Interrupted upload resume (flow 7, harness STEP 9) ──────────────────
//
// A desktop upload cut mid-transfer (tokio timeout on the transfer loop,
// app quit, network drop) used to re-run `POST /uploads/init` on retry:
// every retry minted a NEW server file row + session, re-sent every chunk
// from zero, and left the first attempt's row behind as a visible broken
// `is_uploading` duplicate (GET → 409) until the server's 7-day
// stale-upload sweep. The retry must resume the persisted session instead.

/// Stateful upload mock: 3-chunk plan (8+8+4 bytes). Each connection is
/// served on its own thread so a deliberately hung chunk response cannot
/// block the retry's requests.
pub(super) struct ResumableUploadMock {
    pub(super) base_url: String,
    pub(super) requests: Arc<Mutex<Vec<RecordedRequest>>>,
    pub(super) stop: Arc<std::sync::atomic::AtomicBool>,
    pub(super) handle: thread::JoinHandle<()>,
}

#[derive(Clone, Copy, PartialEq)]
pub(super) enum ResumeMockMode {
    /// Chunk 1 of the first session hangs once (the cut), then the
    /// session keeps accepting chunks.
    HangOnce,
    /// Chunk 1 hangs once, and afterwards session 1 is gone server-side
    /// (404 on every further chunk/complete) — e.g. reaped.
    HangOnceThenSessionGone,
    /// Nothing hangs: every session accepts every chunk.
    Plain,
}

struct ResumeMockState {
    inits: usize,
    hung: bool,
}

impl ResumableUploadMock {
    pub(super) fn start(mode: ResumeMockMode) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let state = Arc::new(Mutex::new(ResumeMockState { inits: 0, hung: false }));
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let server_requests = Arc::clone(&requests);
        let server_stop = Arc::clone(&stop);
        let handle = thread::spawn(move || {
            let started = std::time::Instant::now();
            while !server_stop.load(Ordering::SeqCst) && started.elapsed() < Duration::from_secs(20) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream.set_nonblocking(false).unwrap();
                        let requests = Arc::clone(&server_requests);
                        let state = Arc::clone(&state);
                        thread::spawn(move || {
                            let request = read_http_request(&mut stream);
                            requests.lock().unwrap().push(request.clone());
                            let (delay, response) = resumable_mock_response(&request, &state, mode);
                            if let Some(delay) = delay {
                                std::thread::sleep(delay);
                            }
                            // The client may have been cancelled meanwhile.
                            let _ = stream.write_all(response.as_bytes());
                        });
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(e) => panic!("resumable upload mock accept failed: {e}"),
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

    pub(super) fn finish(self) -> Vec<RecordedRequest> {
        self.stop.store(true, Ordering::SeqCst);
        self.handle.join().unwrap();
        self.requests.lock().unwrap().clone()
    }
}

fn resumable_mock_response(
    request: &RecordedRequest,
    state: &Arc<Mutex<ResumeMockState>>,
    mode: ResumeMockMode,
) -> (Option<Duration>, String) {
    let method = request.method.as_str();
    let path = request.path.as_str();
    if method == "POST" && path == "/api/v1/uploads/init" {
        let n = {
            let mut s = state.lock().unwrap();
            s.inits += 1;
            s.inits
        };
        // SF-0001 (L13): the client chooses the item id, so an honest server
        // answers with the `file_id` the request carried.
        let file_id = serde_json::from_slice::<serde_json::Value>(&request.body)
            .ok()
            .and_then(|body| body["file_id"].as_str().map(str::to_string))
            .unwrap_or_else(|| format!("server-file-{n}"));
        return (
            None,
            http_json(
                "201 Created",
                serde_json::json!({
                    "file_id": file_id,
                    "tenant_id": "tenant-1",
                    "object_version_id": format!("object-init-{n}"),
                    "upload_session_id": format!("session-{n}"),
                    "chunk_size_bytes": 8,
                    "chunk_count": 3,
                    "storage_format_version": 2,
                    "storage_pool_id": "pool-1",
                    "region": "local"
                }),
            ),
        );
    }
    if method == "PATCH" && path.starts_with("/api/v1/files/") {
        return (None, http_json("200 OK", serde_json::json!({ "ok": true })));
    }
    if method == "POST" && path.ends_with("/upload/abandon") {
        return (None, http_json("200 OK", serde_json::json!({ "abandoned": true })));
    }
    if method == "DELETE" && path.starts_with("/api/v1/files/") {
        return (None, http_json("200 OK", serde_json::json!({ "trashed": true })));
    }
    if let Some(rest) = path.strip_prefix("/api/v1/uploads/") {
        let mut parts = rest.split('/');
        let session = parts.next().unwrap_or_default().to_string();
        let action = parts.next().unwrap_or_default();
        let hung_before = state.lock().unwrap().hung;
        if mode == ResumeMockMode::HangOnceThenSessionGone && session == "session-1" && hung_before {
            return (
                None,
                http_json("404 Not Found", serde_json::json!({ "error": "not found" })),
            );
        }
        if method == "PUT" && action == "chunks" {
            let index: u32 = parts.next().unwrap_or("0").parse().unwrap();
            if index == 1 && session == "session-1" && mode != ResumeMockMode::Plain {
                let mut s = state.lock().unwrap();
                if !s.hung {
                    s.hung = true;
                    return (
                        Some(Duration::from_secs(3)),
                        http_json("200 OK", serde_json::json!({ "index": 1, "size": 0, "skipped": false })),
                    );
                }
            }
            return (
                None,
                http_json(
                    "200 OK",
                    serde_json::json!({ "index": index, "size": request.body.len() as i64, "skipped": false }),
                ),
            );
        }
        if method == "POST" && action == "complete" {
            let n = session.trim_start_matches("session-");
            let mut answer = serde_json::json!({
                "file_id": format!("server-file-{n}"),
                "version_number": 1,
                "current_object_version_id": format!("object-complete-{n}"),
                "size_bytes": 20,
                "mime_type": "text/plain"
            });
            // As the server does with version metadata on: the answer carries the name the file now has.
            if let Some(name) = completion_name(request) {
                answer["name_encrypted"] = serde_json::json!(name);
            }
            return (None, http_json("200 OK", answer));
        }
    }
    (
        None,
        http_json(
            "404 Not Found",
            serde_json::json!({ "error": format!("unexpected {method} {path}") }),
        ),
    )
}

pub(super) fn resumable_create_op(payload: &Path) -> PendingOperation {
    PendingOperation {
        op_id: "op-upload-resume".into(),
        kind: OperationKind::UploadVersion,
        file_id: Some("local-file-resume".into()),
        parent_id: None,
        target_path: Some("resume.bin".into()),
        metadata_json: Some(
            serde_json::json!({
                "operation": "create_file",
                "name_encrypted": "{\"cipher_suite\":\"V1Aes256Gcm\"}",
                "display_name": "resume.bin",
                "content_type": "text/plain"
            })
            .to_string(),
        ),
        payload_path: Some(payload.to_string_lossy().into_owned()),
        base_version: None,
        base_object_version_id: None,
        attempts: 0,
        max_attempts: 5,
        next_retry_at: 0,
        last_error: None,
        backup_source_key: None,
        created_at: 100,
        updated_at: 100,
    }
}

pub(super) fn count_requests(requests: &[RecordedRequest], method: &str, path: &str) -> usize {
    requests.iter().filter(|r| r.method == method && r.path == path).count()
}

/// SF-0001 (L13): the item id the client sent to `init` — the only id an
/// honest server may use for the rest of that upload. Reads the first init's
/// recorded body.
pub(super) fn init_item_id(requests: &[RecordedRequest]) -> String {
    let init = requests
        .iter()
        .find(|r| r.method == "POST" && r.path == "/api/v1/uploads/init")
        .unwrap_or_else(|| {
            panic!(
                "no init request recorded: {:?}",
                requests
                    .iter()
                    .map(|r| format!("{} {}", r.method, r.path))
                    .collect::<Vec<_>>()
            )
        });
    serde_json::from_slice::<serde_json::Value>(&init.body).unwrap()["file_id"]
        .as_str()
        .expect("every init now carries the id this device chose")
        .to_string()
}

// ── SF-0001 (L13): owner-chosen item id upload mock ─────────────────────
// The older mocks answer init with a fixed `server-file-N`, which cannot
// express "the server echoes the id the client sent". These tests need the
// echo, so this mock records every request (like the others) and answers
// init with either the request's own `file_id` or a configured stranger id.

/// SF-0001 (L13): how the mock answers init's `file_id`.
#[derive(Clone, Copy, PartialEq)]
enum L13InitEcho {
    /// Echo the `file_id` the client sent; fall back to a server id.
    Same,
    /// Ignore what was sent and answer this id (a server that changed it).
    Fixed(&'static str),
    /// Echo the `file_id` the client sent, re-spelled in upper case. Not the
    /// same id: an echo must be byte-identical to count.
    Uppercase,
}

struct L13MockState {
    inits: usize,
    /// upload session id → the file id the init answered with.
    sessions: std::collections::HashMap<String, String>,
    /// upload session id → the object version id the init answered with.
    objects: std::collections::HashMap<String, String>,
}

struct L13UploadMock {
    base_url: String,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    stop: Arc<AtomicBool>,
    handle: thread::JoinHandle<()>,
}

impl L13UploadMock {
    fn start(echo: L13InitEcho, fail_chunks: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let state = Arc::new(Mutex::new(L13MockState {
            inits: 0,
            sessions: std::collections::HashMap::new(),
            objects: std::collections::HashMap::new(),
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let server_requests = Arc::clone(&requests);
        let server_state = Arc::clone(&state);
        let server_stop = Arc::clone(&stop);
        let handle = thread::spawn(move || {
            let started = std::time::Instant::now();
            while !server_stop.load(Ordering::SeqCst) && started.elapsed() < Duration::from_secs(20) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream.set_nonblocking(false).unwrap();
                        let requests = Arc::clone(&server_requests);
                        let state = Arc::clone(&server_state);
                        thread::spawn(move || {
                            let request = read_http_request(&mut stream);
                            requests.lock().unwrap().push(request.clone());
                            let response = l13_mock_response(&request, &state, echo, fail_chunks);
                            let _ = stream.write_all(response.as_bytes());
                        });
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(e) => panic!("l13 upload mock accept failed: {e}"),
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

    fn finish(self) -> Vec<RecordedRequest> {
        self.stop.store(true, Ordering::SeqCst);
        self.handle.join().unwrap();
        self.requests.lock().unwrap().clone()
    }
}

fn l13_mock_response(
    request: &RecordedRequest,
    state: &Arc<Mutex<L13MockState>>,
    echo: L13InitEcho,
    fail_chunks: bool,
) -> String {
    let method = request.method.as_str();
    let path = request.path.as_str();
    if method == "POST" && path == "/api/v1/uploads/init" {
        let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap_or(serde_json::Value::Null);
        let sent = body.get("file_id").and_then(serde_json::Value::as_str);
        let (session, object, file_id) = {
            let mut s = state.lock().unwrap();
            s.inits += 1;
            let n = s.inits;
            let session = format!("l13-session-{n}");
            let object = format!("l13-object-{n}");
            let file_id = match echo {
                L13InitEcho::Same => sent.map(str::to_string).unwrap_or_else(|| format!("server-file-{n}")),
                L13InitEcho::Fixed(id) => id.to_string(),
                L13InitEcho::Uppercase => sent
                    .map(|id| id.to_uppercase())
                    .unwrap_or_else(|| format!("SERVER-FILE-{n}")),
            };
            s.sessions.insert(session.clone(), file_id.clone());
            s.objects.insert(session.clone(), object.clone());
            (session, object, file_id)
        };
        return http_json(
            "201 Created",
            serde_json::json!({
                "file_id": file_id,
                "tenant_id": "tenant-1",
                "object_version_id": object,
                "upload_session_id": session,
                "chunk_size_bytes": 8,
                "chunk_count": 3,
                "storage_format_version": 2,
                "storage_pool_id": "pool-1",
                "region": "local"
            }),
        );
    }
    if method == "PATCH" && path.starts_with("/api/v1/files/") {
        return http_json("200 OK", serde_json::json!({ "ok": true }));
    }
    if method == "POST" && path.ends_with("/upload/abandon") {
        return http_json("200 OK", serde_json::json!({ "abandoned": true }));
    }
    if method == "DELETE" && path.starts_with("/api/v1/files/") {
        // Present so a run of the STUB (which still trashes) does not turn
        // into an unrelated transport failure; the tests assert it is never hit.
        return http_json("200 OK", serde_json::json!({ "trashed": true }));
    }
    if let Some(rest) = path.strip_prefix("/api/v1/uploads/") {
        let mut parts = rest.split('/');
        let session = parts.next().unwrap_or_default().to_string();
        let action = parts.next().unwrap_or_default();
        if method == "PUT" && action == "chunks" {
            let index: u32 = parts.next().unwrap_or("0").parse().unwrap();
            if fail_chunks {
                return http_json("500 Internal Server Error", serde_json::json!({ "error": "boom" }));
            }
            return http_json(
                "200 OK",
                serde_json::json!({ "index": index, "size": request.body.len() as i64, "skipped": false }),
            );
        }
        if method == "POST" && action == "complete" {
            let s = state.lock().unwrap();
            let file_id = s.sessions.get(&session).cloned().unwrap_or_default();
            let object = s.objects.get(&session).cloned().unwrap_or_default();
            return http_json(
                "200 OK",
                serde_json::json!({
                    "file_id": file_id,
                    "version_number": 1,
                    "current_object_version_id": object,
                    "size_bytes": 20,
                    "mime_type": "text/plain"
                }),
            );
        }
    }
    http_json(
        "404 Not Found",
        serde_json::json!({ "error": format!("unexpected {method} {path}") }),
    )
}

/// SF-0001 (L13): a create op for `local_id`, 20 bytes of payload.
fn l13_create_op(op_id: &str, local_id: &str, payload: &Path) -> PendingOperation {
    let mut op = resumable_create_op(payload);
    op.op_id = op_id.into();
    op.file_id = Some(local_id.into());
    op
}

/// SF-0001 (L13): a replace/version op updating the existing row `local_id`.
fn l13_replace_op(op_id: &str, local_id: &str, payload: &Path) -> PendingOperation {
    let mut op = resumable_create_op(payload);
    op.op_id = op_id.into();
    op.file_id = Some(local_id.into());
    op.base_version = Some(1);
    op.metadata_json = Some(
        serde_json::json!({
            "name_encrypted": "{\"cipher_suite\":\"V1Aes256Gcm\"}",
            "display_name": "resume.bin",
            "content_type": "text/plain"
        })
        .to_string(),
    );
    op
}

pub(super) fn l13_resume(is_create: bool, self_minted: bool, server_file_id: &str) -> UploadResume {
    UploadResume {
        op_id: "op-l13".into(),
        payload_path: "/tmp/l13.bin".into(),
        payload_size: 20,
        payload_mtime_ns: 0,
        upload_session_id: "l13-session-existing".into(),
        server_file_id: server_file_id.into(),
        object_version_id: "l13-object-existing".into(),
        chunk_size_bytes: 8,
        chunk_count: 3,
        acked_chunks: 0,
        metadata_applied: false,
        is_create,
        self_minted,
        completed_version: None,
        completed_object_version_id: None,
        completed_mime_type: None,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn l13_a_create_init_sends_the_desktop_own_id() {
    // SF-0001 (L13): a create whose local id is canonical proposes that id
    // to init instead of letting the server mint one.
    let dir = tempfile::tempdir().unwrap();
    let payload = dir.path().join("payload.bin");
    std::fs::write(&payload, b"0123456789abcdefghij").unwrap();
    let local_id = "0f6c1d2e-3a4b-4c5d-8e9f-a0b1c2d3e4f5";
    let server = L13UploadMock::start(L13InitEcho::Same, false);
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), [31u8; 32]);
    bridge
        .db
        .enqueue_operation(&l13_create_op("op-l13", local_id, &payload))
        .unwrap();

    bridge.process_due_operations(dir.path(), 200).await.unwrap();

    let requests = server.finish();
    assert_eq!(requests[0].method, "POST");
    assert_eq!(requests[0].path, "/api/v1/uploads/init");
    let init_body: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(
        init_body["file_id"], local_id,
        "a create must send the desktop's own canonical id to init"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn l13_a_create_whose_init_echoes_another_id_aborts_before_anything_is_sealed() {
    // SF-0001 (L13): a server that answers with a different id must abort
    // the op before the session is persisted or any chunk/name is sent.
    let dir = tempfile::tempdir().unwrap();
    let payload = dir.path().join("payload.bin");
    std::fs::write(&payload, b"0123456789abcdefghij").unwrap();
    let local_id = "1a2b3c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d";
    let server = L13UploadMock::start(L13InitEcho::Fixed("stranger-file-id"), false);
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), [32u8; 32]);
    bridge
        .db
        .enqueue_operation(&l13_create_op("op-l13", local_id, &payload))
        .unwrap();

    let outcome = bridge.process_due_operations(dir.path(), 200).await.unwrap();
    let recorded = bridge
        .db
        .get_operation("op-l13")
        .unwrap()
        .and_then(|op| op.last_error)
        .unwrap_or_default();
    assert!(
        outcome.retried_op_ids.contains(&"op-l13".to_string()) || outcome.paused_op_ids.contains(&"op-l13".to_string()),
        "the op must not complete"
    );
    assert!(
        recorded.contains("server_changed_item_id"),
        "error must name server_changed_item_id, got {recorded:?}"
    );
    assert!(
        bridge.db.get_upload_resume("op-l13").unwrap().is_none(),
        "nothing may be persisted after a changed id"
    );
    let requests = server.finish();
    assert_eq!(requests.len(), 1, "only the init may be sent: {:?}", requests);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn l13_an_uppercased_echo_of_the_own_id_aborts() {
    // SF-0001 (L13): the comparison is exact string equality. A server that
    // echoes the id the client sent but spells it in upper case answered
    // with a different id, so the op must abort.
    let dir = tempfile::tempdir().unwrap();
    let payload = dir.path().join("payload.bin");
    std::fs::write(&payload, b"0123456789abcdefghij").unwrap();
    let local_id = "6f7a8b9c-0d1e-4f2a-3b4c-5d6e7f8091a2";
    let server = L13UploadMock::start(L13InitEcho::Uppercase, false);
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), [37u8; 32]);
    bridge
        .db
        .enqueue_operation(&l13_create_op("op-l13", local_id, &payload))
        .unwrap();

    let outcome = bridge.process_due_operations(dir.path(), 200).await.unwrap();
    let recorded = bridge
        .db
        .get_operation("op-l13")
        .unwrap()
        .and_then(|op| op.last_error)
        .unwrap_or_default();
    assert!(
        outcome.retried_op_ids.contains(&"op-l13".to_string()) || outcome.paused_op_ids.contains(&"op-l13".to_string()),
        "the op must not complete"
    );
    assert!(
        recorded.contains("server_changed_item_id"),
        "error must name server_changed_item_id, got {recorded:?}"
    );
    assert!(
        bridge.db.get_upload_resume("op-l13").unwrap().is_none(),
        "nothing may be persisted for an id this device did not choose"
    );
    let requests = server.finish();
    assert_eq!(requests.len(), 1, "only the init may be sent: {:?}", requests);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn l13_a_replace_whose_init_echoes_another_id_aborts() {
    // SF-0001 (L13): a replace sends the row's id and aborts if the echo differs.
    let dir = tempfile::tempdir().unwrap();
    let payload = dir.path().join("payload.bin");
    std::fs::write(&payload, b"0123456789abcdefghij").unwrap();
    let local_id = "2b3c4d5e-6f7a-4b8c-9d0e-1f2a3b4c5d6e";
    let server = L13UploadMock::start(L13InitEcho::Fixed("stranger-file-id"), false);
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), [33u8; 32]);
    bridge
        .db
        .enqueue_operation(&l13_replace_op("op-l13", local_id, &payload))
        .unwrap();

    let outcome = bridge.process_due_operations(dir.path(), 200).await.unwrap();
    let recorded = bridge
        .db
        .get_operation("op-l13")
        .unwrap()
        .and_then(|op| op.last_error)
        .unwrap_or_default();
    assert!(
        outcome.retried_op_ids.contains(&"op-l13".to_string()) || outcome.paused_op_ids.contains(&"op-l13".to_string()),
        "the op must not complete"
    );
    assert!(
        recorded.contains("server_changed_item_id"),
        "error must name server_changed_item_id, got {recorded:?}"
    );
    assert!(bridge.db.get_upload_resume("op-l13").unwrap().is_none());
    let requests = server.finish();
    assert_eq!(requests.len(), 1, "only the init may be sent: {:?}", requests);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn l13_is_create_comes_from_the_op_kind() {
    // SF-0001 (L13): `is_create` must be derived from the op metadata, not
    // from whether init sent a `file_id` (which creates now always do).
    let dir = tempfile::tempdir().unwrap();
    let payload = dir.path().join("payload.bin");
    std::fs::write(&payload, b"0123456789abcdefghij").unwrap();
    let local_id = "3c4d5e6f-7a8b-4c9d-0e1f-2a3b4c5d6e7f";
    let server = L13UploadMock::start(L13InitEcho::Same, true);
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), [34u8; 32]);
    bridge
        .db
        .enqueue_operation(&l13_create_op("op-l13", local_id, &payload))
        .unwrap();

    bridge.process_due_operations(dir.path(), 200).await.unwrap();

    let session = bridge
        .db
        .get_upload_resume("op-l13")
        .unwrap()
        .expect("a failed chunk keeps the session persisted");
    assert!(session.is_create, "the op kind says create_file");
    assert!(session.self_minted, "a session opened by the new code is self-minted");
    let _ = server.finish();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn l13_a_non_canonical_local_id_proposes_a_fresh_own_id() {
    // SF-0001 (L13): a non-canonical local id cannot be the server id, so
    // the desktop mints a fresh canonical v4 id for this init.
    let dir = tempfile::tempdir().unwrap();
    let payload = dir.path().join("payload.bin");
    std::fs::write(&payload, b"0123456789abcdefghij").unwrap();
    let server = L13UploadMock::start(L13InitEcho::Same, true);
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), [35u8; 32]);
    bridge
        .db
        .enqueue_operation(&l13_create_op("op-l13", "local-file-1", &payload))
        .unwrap();

    bridge.process_due_operations(dir.path(), 200).await.unwrap();

    let requests_snapshot = server.requests.lock().unwrap().clone();
    assert_eq!(requests_snapshot[0].path, "/api/v1/uploads/init");
    let init_body: serde_json::Value = serde_json::from_slice(&requests_snapshot[0].body).unwrap();
    let proposed = init_body["file_id"]
        .as_str()
        .expect("a non-canonical local id must not be sent; a fresh one is")
        .to_string();
    assert_ne!(proposed, "local-file-1");
    assert!(
        is_canonical_item_id(&proposed),
        "the proposed id must be canonical, got {proposed:?}"
    );
    let session = bridge
        .db
        .get_upload_resume("op-l13")
        .unwrap()
        .expect("a failed chunk keeps the session persisted");
    assert!(session.self_minted);
    assert_eq!(session.server_file_id, proposed);
    let requests = server.finish();
    assert_eq!(
        count_requests(&requests, "POST", "/api/v1/uploads/init"),
        1,
        "exactly one init"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn l13_a_pre_update_create_session_is_abandoned_and_restarted() {
    // SF-0001 (L13): a CREATE session persisted by an older version (server-
    // chosen id, self_minted=false) must never resume; it is abandoned and a
    // fresh init runs under the desktop's own id.
    let dir = tempfile::tempdir().unwrap();
    let payload = dir.path().join("payload.bin");
    std::fs::write(&payload, b"0123456789abcdefghij").unwrap();
    let local_id = "4d5e6f7a-8b9c-4d0e-1f2a-3b4c5d6e7f80";
    let old_server_id = "old-server-chosen-id";
    let server = L13UploadMock::start(L13InitEcho::Same, true);
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), [36u8; 32]);
    let op = l13_create_op("op-l13", local_id, &payload);
    bridge.db.enqueue_operation(&op).unwrap();
    // Same path/size/mtime as the op, so the old code would resume it.
    let mut previous = l13_resume(true, false, old_server_id);
    previous.op_id = "op-l13".into();
    previous.payload_path = payload.to_string_lossy().into_owned();
    previous.payload_size = 20;
    previous.payload_mtime_ns = payload_mtime_ns(&payload);
    bridge.db.put_upload_resume(&previous).unwrap();

    bridge.process_due_operations(dir.path(), 200).await.unwrap();

    let requests = server.finish();
    assert_eq!(
        count_requests(
            &requests,
            "POST",
            &format!("/api/v1/files/{old_server_id}/upload/abandon")
        ),
        1,
        "the pre-update create session must be abandoned: {:?}",
        requests
    );
    assert_eq!(
        count_requests(&requests, "DELETE", &format!("/api/v1/files/{old_server_id}")),
        0,
        "abandoning a create must never trash the item"
    );
    let init = requests
        .iter()
        .find(|r| r.method == "POST" && r.path == "/api/v1/uploads/init")
        .expect("a fresh init must follow the abandon");
    let init_body: serde_json::Value = serde_json::from_slice(&init.body).unwrap();
    assert_eq!(init_body["file_id"], local_id);
    if let Some(session) = bridge.db.get_upload_resume("op-l13").unwrap() {
        assert!(session.self_minted, "the restarted session is self-minted");
    }
}

#[test]
fn l13_canonical_item_id() {
    // SF-0001 (L13): only the exact lowercase hyphenated UUID spelling is canonical.
    let canonical = "0f6c1d2e-3a4b-4c5d-8e9f-a0b1c2d3e4f5";
    assert!(is_canonical_item_id(canonical));
    assert!(!is_canonical_item_id(&canonical.to_uppercase()));
    assert!(!is_canonical_item_id(&format!("{{{canonical}}}")));
    assert!(!is_canonical_item_id(&canonical.replace('-', "")));
    assert!(!is_canonical_item_id(&format!(" {canonical}")));
    assert!(!is_canonical_item_id(&format!("{canonical} ")));
    assert!(!is_canonical_item_id(""));
    assert!(!is_canonical_item_id("local-file-1"));
}

#[test]
fn l13_session_item_id_must_be_ours() {
    // SF-0001 (L13): a create needs self_minted; a replace needs the row's id.
    let local_id = "5e6f7a8b-9c0d-4e1f-2a3b-4c5d6e7f8091";
    let local = LocalFileId::new(local_id);
    let err = ensure_session_item_id(&l13_resume(true, false, "server-chosen"), local)
        .unwrap_err()
        .to_string();
    assert!(err.contains("server_changed_item_id"), "got {err:?}");
    assert!(ensure_session_item_id(&l13_resume(true, true, "whatever-the-server-says"), local).is_ok());
    let err = ensure_session_item_id(&l13_resume(false, true, "another-row"), local)
        .unwrap_err()
        .to_string();
    assert!(err.contains("server_changed_item_id"), "got {err:?}");
    assert!(ensure_session_item_id(&l13_resume(false, true, local_id), local).is_ok());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn round5_lock_during_keep_mine_restores_conflict_and_removes_staging() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("sync");
    std::fs::create_dir_all(&root).unwrap();
    let original = root.join("conflict.bin");
    std::fs::write(&original, b"0123456789abcdefghij").unwrap();
    let server = ResumableUploadMock::start(ResumeMockMode::HangOnce);
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), [21; 32]);
    bridge
        .db
        .upsert_file(&FileEntry {
            file_id: "6d2d20bc-321b-533d-931c-778b0b34a9fc".into(),
            path: "conflict.bin".into(),
            status: FileStatus::Conflict,
            size_bytes: 20,
            modified_at: 100,
            content_hash: None,
            remote_updated_at: 90,
            parent_id: None,
            item_kind: ItemKind::File,
        })
        .unwrap();
    let commands = crate::session_commands::SessionCommands::new();
    commands.open().unwrap();
    let held = async {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if server
                    .requests
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|r| r.path.ends_with("/chunks/1"))
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("positive control: upload reached held second chunk");
    };
    let work = commands.run(async {
        bridge
            .resolve_keep_mine("6d2d20bc-321b-533d-931c-778b0b34a9fc", &root)
            .await
            .map_err(|e| e.to_string())
    });
    let mut work = Box::pin(work);
    tokio::select! { biased; _ = held => (), result = &mut work => panic!("upload completed before lock: {result:?}") }
    let conn = rusqlite::Connection::open(dir.path().join("state.db")).unwrap();
    let staged: Vec<String> = conn
        .prepare("SELECT payload_path FROM upload_resume")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(staged.len(), 1, "positive control: resume owns staged upload");
    assert!(std::path::Path::new(&staged[0]).is_file());
    let revoked = commands.close();
    assert!(work.await.is_err());
    commands.drain(revoked, Duration::from_secs(1)).unwrap();
    let requests = server.finish();
    assert_eq!(
        count_requests(&requests, "PUT", "/api/v1/uploads/session-1/chunks/1"),
        1
    );
    assert_eq!(
        bridge
            .db
            .get_file("6d2d20bc-321b-533d-931c-778b0b34a9fc")
            .unwrap()
            .unwrap()
            .status,
        FileStatus::Conflict,
        "cancelled Keep Mine must restore the conflict state"
    );
    assert_eq!(
        staged.iter().filter(|p| std::path::Path::new(p).exists()).count(),
        0,
        "cancelled Keep Mine left orphaned plaintext"
    );
    assert_eq!(std::fs::read(&original).unwrap(), b"0123456789abcdefghij");
    assert!(
        bridge.db.windows_signout_preflight().is_err(),
        "sign-out must preserve the original unsynced conflict"
    );
    // User resolves/moves the original; subsequent account purge leaves zero staged plaintext.
    bridge
        .db
        .set_status("6d2d20bc-321b-533d-931c-778b0b34a9fc", FileStatus::Local)
        .unwrap();
    bridge.db.windows_signout_preflight().unwrap();
    bridge.db.finish_windows_signout().unwrap();
    assert_eq!(staged.iter().filter(|p| std::path::Path::new(p).exists()).count(), 0);
}

/// Chunk format v2: a cut upload is never continued. The retry abandons the session on the server
/// and uploads from the first chunk in a fresh one: a new writer has a new version id, and its
/// frames next to the old session's would make a file that completes and never opens (core's
/// `resume_policy`). Mutant: a retry that goes on in the old session sends its chunk 2 there.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn flow7_a_cut_upload_restarts_in_a_fresh_session_and_never_continues_the_old_one() {
    let dir = tempfile::tempdir().unwrap();
    let payload = dir.path().join("resume.bin");
    std::fs::write(&payload, b"0123456789abcdefghij").unwrap();
    let server = ResumableUploadMock::start(ResumeMockMode::HangOnce);
    let master_key = [21u8; 32];
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
    bridge.db.enqueue_operation(&resumable_create_op(&payload)).unwrap();

    // The cut: chunk 1's response hangs; the transfer loop is cancelled
    // exactly like the harness's tokio timeout (the future is dropped).
    let cut = tokio::time::timeout(
        Duration::from_millis(1500),
        bridge.process_due_operations(dir.path(), 200),
    )
    .await;
    assert!(cut.is_err(), "the first pass must be cut mid-upload");
    assert!(
        bridge.db.upload_completion_name("op-upload-resume").unwrap().is_some(),
        "the completion name is stored before the first chunk is sent"
    );

    let outcome = bridge.process_due_operations(dir.path(), 200).await.unwrap();
    assert_eq!(outcome.completed_op_ids, vec!["op-upload-resume".to_string()]);

    let requests = server.finish();
    let first_id = init_item_id(&requests);
    assert_eq!(
        count_requests(&requests, "POST", "/api/v1/uploads/init"),
        2,
        "the retry opens a fresh session"
    );
    assert_eq!(
        count_requests(&requests, "POST", &format!("/api/v1/files/{first_id}/upload/abandon")),
        1,
        "the cut session is abandoned on the server before the fresh init"
    );
    for (path, sent) in [
        ("/api/v1/uploads/session-1/chunks/0", 1),
        ("/api/v1/uploads/session-1/chunks/1", 1),
        ("/api/v1/uploads/session-1/chunks/2", 0),
        ("/api/v1/uploads/session-2/chunks/0", 1),
        ("/api/v1/uploads/session-2/chunks/1", 1),
        ("/api/v1/uploads/session-2/chunks/2", 1),
    ] {
        assert_eq!(count_requests(&requests, "PUT", path), sent, "{path}");
    }
    assert_eq!(
        count_requests(&requests, "POST", "/api/v1/uploads/session-1/complete"),
        0,
        "the old session is never completed"
    );
    assert_eq!(
        count_requests(&requests, "POST", "/api/v1/uploads/session-2/complete"),
        1
    );
    assert_eq!(
        requests.iter().filter(|r| r.method == "DELETE").count(),
        0,
        "an abandoned session is never a trashed item"
    );
    // The fresh session's frames are one whole upload: they open, in order, against the
    // commitment its `complete` published.
    let second_id = requests
        .iter()
        .filter(|r| r.method == "POST" && r.path == "/api/v1/uploads/init")
        .nth(1)
        .map(|r| {
            serde_json::from_slice::<serde_json::Value>(&r.body).unwrap()["file_id"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .unwrap();
    let fresh: Vec<RecordedRequest> = requests
        .iter()
        .filter(|r| r.path.contains("/session-2/"))
        .cloned()
        .collect();
    let file_key = hydration_test_key(master_key, &second_id);
    assert_eq!(
        uploaded_plaintext(&fresh, &file_key, &second_id),
        b"0123456789abcdefghij"
    );
    let item_id = second_id;

    let entry = bridge.db.get_file(&item_id).unwrap().unwrap();
    assert_eq!(entry.status, FileStatus::Local);
    assert!(
        bridge.db.get_upload_resume("op-upload-resume").unwrap().is_none(),
        "resume state must be cleared once the upload completes"
    );
}
