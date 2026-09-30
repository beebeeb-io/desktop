//! End-to-end framing tests for the daemon IPC socket (task 1670 issue 3).
//!
//! These drive the REAL `serve_ipc_at_with_ready` accept loop over a real Unix
//! socket with hand-rolled clients, so they pin the wire behaviour a Swift
//! client sees — not an internal function. Requests are built from raw JSON
//! on purpose: nothing here depends on the new `progress` field, so the same
//! file compiles against the pre-fix daemon, which is how the RED runs in the
//! task notes were captured (see `docs/IPC_PROTOCOL.md`).
//!
//! Hydrate progress + cancellation need a fake HTTP backend and live next to
//! that fixture in `engine_bridge.rs`'s test module.

use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

use crate::api_client::ApiClient;
use crate::engine_bridge::EngineBridge;
use crate::state_db::{FileEntry, FileStatus, ItemKind, StateDb};

const READ_DEADLINE: Duration = Duration::from_secs(5);

struct IpcFixture {
    rt: tokio::runtime::Runtime,
    sock: std::path::PathBuf,
    db: Arc<StateDb>,
    cancel: Option<tokio::sync::oneshot::Sender<()>>,
    server: Option<tokio::task::JoinHandle<std::io::Result<()>>>,
    _state_dir: tempfile::TempDir,
    _sock_dir: tempfile::TempDir,
}

impl IpcFixture {
    fn start(seed: impl FnOnce(&StateDb)) -> Self {
        let state_dir = tempfile::tempdir().unwrap();
        let sock_dir = tempfile::tempdir().unwrap();
        let sock = sock_dir.path().join("ipc.sock");
        let db = Arc::new(StateDb::open(state_dir.path().join("state.db")).unwrap());
        seed(&db);
        let api = Arc::new(ApiClient::new("http://127.0.0.1:9".into(), "token".into(), [7u8; 32]));
        let bridge = Arc::new(EngineBridge::new(db.clone(), api));

        let rt = tokio::runtime::Runtime::new().unwrap();
        let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let server = rt.spawn(crate::ipc_socket::serve_ipc_at_with_ready(
            sock.clone(),
            db.clone(),
            bridge,
            cancel_rx,
            Some(ready_tx),
        ));
        rt.block_on(async {
            tokio::time::timeout(Duration::from_secs(5), ready_rx)
                .await
                .expect("IPC server did not become ready")
                .expect("IPC server failed to start");
        });
        Self {
            rt,
            sock,
            db,
            cancel: Some(cancel_tx),
            server: Some(server),
            _state_dir: state_dir,
            _sock_dir: sock_dir,
        }
    }

    async fn connect(&self) -> UnixStream {
        UnixStream::connect(&self.sock).await.expect("connect to test IPC socket")
    }
}

impl Drop for IpcFixture {
    fn drop(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            let _ = cancel.send(());
        }
        if let Some(server) = self.server.take() {
            let _ = self.rt.block_on(async { tokio::time::timeout(Duration::from_secs(5), server).await });
        }
    }
}

/// Read one `\n`-terminated reply line, one byte at a time so nothing past the
/// delimiter is consumed. Panics (test failure) if no delimiter arrives within
/// the deadline — which is exactly how the pre-fix daemon, that wrote replies
/// with no delimiter, fails these tests.
async fn read_line(client: &mut UnixStream) -> Vec<u8> {
    let deadline = tokio::time::Instant::now() + READ_DEADLINE;
    let mut line = Vec::new();
    loop {
        let mut byte = [0u8; 1];
        let n = tokio::time::timeout_at(deadline, client.read(&mut byte))
            .await
            .unwrap_or_else(|_| panic!("no delimited reply within {READ_DEADLINE:?} ({} bytes so far)", line.len()))
            .expect("read from daemon");
        assert!(
            n > 0,
            "daemon closed the connection before a full line ({} bytes read)",
            line.len()
        );
        if byte[0] == b'\n' {
            return line;
        }
        line.push(byte[0]);
    }
}

/// Buffered line reader for tests where several replies may already be queued.
struct LineReader {
    client: UnixStream,
    pending: Vec<u8>,
}

impl LineReader {
    fn new(client: UnixStream) -> Self {
        Self {
            client,
            pending: Vec::new(),
        }
    }

    async fn next_line(&mut self) -> Vec<u8> {
        let deadline = tokio::time::Instant::now() + READ_DEADLINE;
        loop {
            if let Some(pos) = self.pending.iter().position(|b| *b == b'\n') {
                let mut line: Vec<u8> = self.pending.drain(..=pos).collect();
                line.pop();
                return line;
            }
            let mut chunk = [0u8; 8192];
            let n = tokio::time::timeout_at(deadline, self.client.read(&mut chunk))
                .await
                .unwrap_or_else(|_| panic!("no delimited reply within {READ_DEADLINE:?}"))
                .expect("read from daemon");
            assert!(n > 0, "daemon closed the connection before the expected reply");
            self.pending.extend_from_slice(&chunk[..n]);
        }
    }

    /// `true` if the daemon sends nothing more for `quiet`.
    async fn stays_quiet_for(&mut self, quiet: Duration) -> bool {
        if !self.pending.is_empty() {
            return false;
        }
        let mut chunk = [0u8; 1024];
        match tokio::time::timeout(quiet, self.client.read(&mut chunk)).await {
            Err(_) => true,
            Ok(Ok(0)) => true, // closed is also "nothing more"
            Ok(_) => false,
        }
    }
}

fn parse(line: &[u8]) -> serde_json::Value {
    serde_json::from_slice(line).unwrap_or_else(|e| panic!("reply line was not valid JSON ({e}): {} bytes", line.len()))
}

fn seed_many_top_level_files(db: &StateDb, count: usize) {
    for i in 0..count {
        db.upsert_file(&FileEntry {
            file_id: format!("00000000-0000-0000-0000-{i:012}"),
            // Long names make each item ~350 bytes on the wire, so a few
            // hundred of them are well past one 64 KiB read.
            path: format!("/{i:05}-{}.bin", "long-encrypted-file-name-".repeat(6)),
            status: FileStatus::CloudOnly,
            size_bytes: 1234,
            modified_at: 1,
            content_hash: None,
            remote_updated_at: 1,
            parent_id: None,
            item_kind: ItemKind::File,
        })
        .unwrap();
    }
}

#[test]
fn a_reply_larger_than_64_kib_arrives_intact_and_delimited() {
    let fx = IpcFixture::start(|db| seed_many_top_level_files(db, 400));
    fx.rt.block_on(async {
        let mut client = fx.connect().await;
        client
            .write_all(b"{\"ListFileProviderItems\":{\"container_id\":\"namespace:my_files\"}}\n")
            .await
            .unwrap();
        let line = read_line(&mut client).await;
        assert!(
            line.len() > 65536,
            "fixture must exceed one 64 KiB read to prove anything (got {} bytes)",
            line.len()
        );
        let reply = parse(&line);
        let items = reply["FileProviderItems"]["items"].as_array().expect("items array");
        assert_eq!(items.len(), 400, "every seeded item must be in the reply");
    });
}

#[test]
fn a_request_split_across_two_writes_is_parsed_as_one_request() {
    let fx = IpcFixture::start(|_| {});
    fx.rt.block_on(async {
        let mut client = fx.connect().await;
        client.write_all(b"{\"ListFileProviderItems\":{\"contai").await.unwrap();
        client.flush().await.unwrap();
        tokio::time::sleep(Duration::from_millis(150)).await;
        client.write_all(b"ner_id\":\"__fp_root__\"}}\n").await.unwrap();
        let reply = parse(&read_line(&mut client).await);
        let items = reply["FileProviderItems"]["items"]
            .as_array()
            .unwrap_or_else(|| panic!("expected FileProviderItems, got {reply}"));
        assert_eq!(items.len(), 4, "the four namespaces");
    });
}

#[test]
fn several_requests_on_one_connection_each_get_exactly_one_delimited_reply() {
    let fx = IpcFixture::start(|_| {});
    fx.rt.block_on(async {
        let client = fx.connect().await;
        let mut lines = LineReader::new(client);
        // Three requests in ONE write: the pre-fix daemon parsed the whole
        // buffer as a single request and failed.
        let batch = b"\"GetSyncSummary\"\n\
            {\"ListFileProviderItems\":{\"container_id\":\"__fp_root__\"}}\n\
            {\"GetFileStatus\":{\"file_id\":\"does-not-exist\"}}\n";
        lines.client.write_all(batch).await.unwrap();

        let first = parse(&lines.next_line().await);
        assert!(first.get("SyncSummary").is_some(), "reply 1 must answer GetSyncSummary: {first}");
        let second = parse(&lines.next_line().await);
        assert!(
            second.get("FileProviderItems").is_some(),
            "reply 2 must answer ListFileProviderItems: {second}"
        );
        let third = parse(&lines.next_line().await);
        assert_eq!(third["Error"]["message"], "not found", "reply 3 must answer GetFileStatus: {third}");
        assert!(
            lines.stays_quiet_for(Duration::from_millis(300)).await,
            "exactly one reply per request: nothing more may follow"
        );
    });
}

#[test]
fn an_unframed_request_from_an_old_extension_is_still_answered() {
    // The 0.8.6 extension wrote its request in one write() with no delimiter
    // and then waited for the reply without closing. The daemon must answer
    // without waiting for a newline or EOF.
    let fx = IpcFixture::start(|_| {});
    fx.rt.block_on(async {
        let mut client = fx.connect().await;
        client
            .write_all(br#"{"ListFileProviderItems":{"container_id":"__fp_root__"}}"#)
            .await
            .unwrap();
        let reply = parse(&read_line(&mut client).await);
        assert!(reply.get("FileProviderItems").is_some(), "got {reply}");
    });
}

#[test]
fn a_malformed_line_gets_an_error_and_the_connection_keeps_working() {
    let fx = IpcFixture::start(|_| {});
    fx.rt.block_on(async {
        let client = fx.connect().await;
        let mut lines = LineReader::new(client);
        lines.client.write_all(b"{this is not json}\n").await.unwrap();
        let err = parse(&lines.next_line().await);
        assert!(err["Error"]["message"].is_string(), "got {err}");
        lines.client.write_all(b"\"GetSyncSummary\"\n").await.unwrap();
        let ok = parse(&lines.next_line().await);
        assert!(ok.get("SyncSummary").is_some(), "connection must survive a bad line: {ok}");
    });
}

#[test]
fn an_oversized_request_is_refused_with_an_error_then_the_connection_closes() {
    let fx = IpcFixture::start(|_| {});
    fx.rt.block_on(async {
        let mut client = fx.connect().await;
        // 2 MiB with no newline and never valid JSON.
        let junk = vec![b'x'; 2 * 1024 * 1024];
        // The daemon may close mid-write once it has seen enough; ignore that.
        let _ = client.write_all(&junk).await;
        let reply = parse(&read_line(&mut client).await);
        let message = reply["Error"]["message"].as_str().unwrap_or_default().to_string();
        assert!(message.contains("limit"), "expected a size-limit error, got {reply}");
    });
}

#[test]
fn success_is_an_object_on_the_wire_not_a_bare_string() {
    // JSONSerialization on the Swift side rejects a top-level string, which is
    // what turned every SUCCESSFUL hydrate into "daemon response was not valid
    // JSON". Pin the shape both in isolation and over the socket.
    assert_eq!(
        serde_json::to_string(&crate::ipc_socket::IpcResponse::Ok {}).unwrap(),
        r#"{"Ok":{}}"#
    );
    let fx = IpcFixture::start(|_| {});
    fx.rt.block_on(async {
        let mut client = fx.connect().await;
        client
            .write_all(b"{\"SetFileStatus\":{\"file_id\":\"x\",\"status\":\"local\"}}\n")
            .await
            .unwrap();
        let line = read_line(&mut client).await;
        assert_eq!(String::from_utf8(line).unwrap(), r#"{"Ok":{}}"#);
    });
}

#[test]
fn a_rejected_write_keeps_the_connection_open_for_the_next_request() {
    // `QueueFinderCreate` into "Shared with me" used to `return` from the
    // connection handler after writing its error, silently dropping the peer.
    let fx = IpcFixture::start(|_| {});
    fx.rt.block_on(async {
        let client = fx.connect().await;
        let mut lines = LineReader::new(client);
        lines
            .client
            .write_all(
                b"{\"QueueFinderCreate\":{\"parent_id\":\"namespace:shared_with_me\",\"filename\":\"a.txt\",\"kind\":\"file\",\"contents_path\":null,\"content_type\":null}}\n",
            )
            .await
            .unwrap();
        let err = parse(&lines.next_line().await);
        assert!(err["Error"]["message"].as_str().unwrap_or_default().contains("read-only"), "got {err}");
        lines.client.write_all(b"\"GetSyncSummary\"\n").await.unwrap();
        let next = parse(&lines.next_line().await);
        assert!(next.get("SyncSummary").is_some(), "connection must still be usable: {next}");
    });
}

// ---------------------------------------------------------------------------
// Task 1684: write-queue request-id dedup, over the real socket, counting the
// REAL queued operations in the state DB (not a mock).
// ---------------------------------------------------------------------------

fn queued_operation_count(fx: &IpcFixture) -> usize {
    fx.db.list_due_operations(i64::MAX).unwrap().len()
}

fn source_file(dir: &tempfile::TempDir, name: &str) -> String {
    let path = dir.path().join(name);
    std::fs::write(&path, b"finder file contents").unwrap();
    path.to_string_lossy().into_owned()
}

fn create_request(filename: &str, contents_path: &str, request_id: Option<&str>) -> Vec<u8> {
    let mut body = serde_json::json!({
        "parent_id": null,
        "filename": filename,
        "kind": "file",
        "contents_path": contents_path,
        "content_type": null,
    });
    if let Some(id) = request_id {
        body["request_id"] = serde_json::json!(id);
    }
    let mut line = serde_json::to_vec(&serde_json::json!({ "QueueFinderCreate": body })).unwrap();
    line.push(b'\n');
    line
}

fn modify_request(file_id: &str, filename: &str, contents_path: &str, request_id: Option<&str>) -> Vec<u8> {
    let mut body = serde_json::json!({
        "file_id": file_id,
        "parent_id": null,
        "filename": filename,
        "kind": "file",
        "contents_path": contents_path,
        "content_type": null,
        "base_version_identifier": null,
    });
    if let Some(id) = request_id {
        body["request_id"] = serde_json::json!(id);
    }
    let mut line = serde_json::to_vec(&serde_json::json!({ "QueueFinderModify": body })).unwrap();
    line.push(b'\n');
    line
}

async fn send_one(fx: &IpcFixture, request: Vec<u8>) -> serde_json::Value {
    let mut client = fx.connect().await;
    client.write_all(&request).await.unwrap();
    parse(&read_line(&mut client).await)
}

fn assert_write_queued(reply: &serde_json::Value) {
    assert!(reply.get("WriteQueued").is_some(), "expected WriteQueued, got {reply}");
    assert!(
        reply["WriteQueued"]["item"]["identifier"].as_str().is_some(),
        "a queued create must report the item it created: {reply}"
    );
}

#[test]
fn concurrent_creates_with_one_request_id_queue_exactly_one_upload() {
    let fx = IpcFixture::start(|_| {});
    let src = tempfile::tempdir().unwrap();
    let path = source_file(&src, "big.bin");
    let replies: Vec<serde_json::Value> = fx.rt.block_on(async {
        let mut tasks = Vec::new();
        for _ in 0..8 {
            let request = create_request("big.bin", &path, Some("key-concurrent"));
            let mut client = fx.connect().await;
            tasks.push(tokio::spawn(async move {
                client.write_all(&request).await.unwrap();
                parse(&read_line(&mut client).await)
            }));
        }
        let mut out = Vec::new();
        for t in tasks {
            out.push(t.await.unwrap());
        }
        out
    });
    assert_eq!(replies.len(), 8);
    for r in &replies {
        assert_write_queued(r);
        assert_eq!(r, &replies[0], "every repeat must get the SAME WriteQueued reply");
    }
    assert_eq!(queued_operation_count(&fx), 1, "one logical create must queue exactly one upload");
}

#[test]
fn a_repeat_after_completion_returns_the_cached_reply_without_queueing() {
    let fx = IpcFixture::start(|_| {});
    let src = tempfile::tempdir().unwrap();
    let path = source_file(&src, "a.bin");
    fx.rt.block_on(async {
        let first = send_one(&fx, create_request("a.bin", &path, Some("key-after"))).await;
        assert_write_queued(&first);
        assert_eq!(queued_operation_count(&fx), 1);
        // The retry comes on a NEW connection, well after the first finished.
        let retry = send_one(&fx, create_request("a.bin", &path, Some("key-after"))).await;
        assert_eq!(retry, first);
    });
    assert_eq!(queued_operation_count(&fx), 1, "the retry must not queue a second upload");
}

#[test]
fn different_request_ids_each_queue_their_own_upload() {
    let fx = IpcFixture::start(|_| {});
    let src = tempfile::tempdir().unwrap();
    let path = source_file(&src, "a.bin");
    fx.rt.block_on(async {
        let one = send_one(&fx, create_request("a.bin", &path, Some("key-one"))).await;
        let two = send_one(&fx, create_request("a.bin", &path, Some("key-two"))).await;
        assert_write_queued(&one);
        assert_write_queued(&two);
        assert_ne!(
            one["WriteQueued"]["item"]["identifier"], two["WriteQueued"]["item"]["identifier"],
            "two different keys are two different creates"
        );
    });
    assert_eq!(queued_operation_count(&fx), 2);
}

#[test]
fn a_request_without_a_request_id_behaves_exactly_as_before() {
    // Version skew: the 0.8.6 extension sends no key. Two identical key-less
    // requests are two creates, as they always were.
    let fx = IpcFixture::start(|_| {});
    let src = tempfile::tempdir().unwrap();
    let path = source_file(&src, "a.bin");
    fx.rt.block_on(async {
        let one = send_one(&fx, create_request("a.bin", &path, None)).await;
        let two = send_one(&fx, create_request("a.bin", &path, None)).await;
        assert_write_queued(&one);
        assert_write_queued(&two);
        assert_ne!(one, two);
    });
    assert_eq!(queued_operation_count(&fx), 2);
}

#[test]
fn a_cached_create_is_not_returned_once_its_row_is_gone() {
    // The false-dedup guard: the same key arriving after the user deleted the
    // file is a NEW create, not a retry.
    let fx = IpcFixture::start(|_| {});
    let src = tempfile::tempdir().unwrap();
    let path = source_file(&src, "a.bin");
    fx.rt.block_on(async {
        let first = send_one(&fx, create_request("a.bin", &path, Some("key-gone"))).await;
        assert_write_queued(&first);
        let id = first["WriteQueued"]["item"]["identifier"].as_str().unwrap().to_string();
        fx.db.delete_file(&id).unwrap();
        let again = send_one(&fx, create_request("a.bin", &path, Some("key-gone"))).await;
        assert_write_queued(&again);
        assert_ne!(again["WriteQueued"]["item"]["identifier"], first["WriteQueued"]["item"]["identifier"]);
    });
    assert_eq!(queued_operation_count(&fx), 2);
}

#[test]
fn a_request_id_reused_for_another_file_name_is_not_merged() {
    let fx = IpcFixture::start(|_| {});
    let src = tempfile::tempdir().unwrap();
    let path = source_file(&src, "a.bin");
    fx.rt.block_on(async {
        let a = send_one(&fx, create_request("a.bin", &path, Some("key-collide"))).await;
        let b = send_one(&fx, create_request("b.bin", &path, Some("key-collide"))).await;
        assert_write_queued(&a);
        assert_write_queued(&b);
        assert_ne!(a["WriteQueued"]["item"]["identifier"], b["WriteQueued"]["item"]["identifier"]);
    });
    assert_eq!(queued_operation_count(&fx), 2);
}

#[test]
fn concurrent_modifies_with_one_request_id_queue_exactly_one_version() {
    let fx = IpcFixture::start(|db| {
        db.upsert_file(&FileEntry {
            file_id: "00000000-0000-0000-0000-00000000aaaa".into(),
            path: "doc.txt".into(),
            status: FileStatus::Local,
            size_bytes: 3,
            modified_at: 1,
            content_hash: None,
            remote_updated_at: 1,
            parent_id: None,
            item_kind: ItemKind::File,
        })
        .unwrap();
    });
    let src = tempfile::tempdir().unwrap();
    let path = source_file(&src, "doc.txt");
    let replies: Vec<serde_json::Value> = fx.rt.block_on(async {
        let mut tasks = Vec::new();
        for _ in 0..6 {
            let request = modify_request("00000000-0000-0000-0000-00000000aaaa", "doc.txt", &path, Some("key-modify"));
            let mut client = fx.connect().await;
            tasks.push(tokio::spawn(async move {
                client.write_all(&request).await.unwrap();
                parse(&read_line(&mut client).await)
            }));
        }
        let mut out = Vec::new();
        for t in tasks {
            out.push(t.await.unwrap());
        }
        out
    });
    for r in &replies {
        assert!(r.get("WriteQueued").is_some(), "expected WriteQueued, got {r}");
        assert_eq!(r, &replies[0]);
    }
    assert_eq!(queued_operation_count(&fx), 1, "one logical modify must queue exactly one version");
}

#[test]
fn an_unusable_request_id_is_refused_not_ignored() {
    let fx = IpcFixture::start(|_| {});
    let src = tempfile::tempdir().unwrap();
    let path = source_file(&src, "a.bin");
    fx.rt.block_on(async {
        for bad in ["".to_string(), "x".repeat(129), "line\nbreak".to_string()] {
            let reply = send_one(&fx, create_request("a.bin", &path, Some(&bad))).await;
            let message = reply["Error"]["message"].as_str().unwrap_or_default();
            assert!(message.contains("request_id"), "a bad key must be refused, got {reply}");
        }
    });
    assert_eq!(queued_operation_count(&fx), 0, "a refused request must queue nothing");
}

#[test]
fn a_failed_create_is_not_remembered_so_the_retry_actually_runs() {
    // The first attempt fails (its source file is unreadable), which must not
    // poison the key: the system's retry, now with a readable file, has to queue.
    let fx = IpcFixture::start(|_| {});
    let src = tempfile::tempdir().unwrap();
    let good = source_file(&src, "a.bin");
    let missing = src.path().join("does-not-exist.bin").to_string_lossy().into_owned();
    fx.rt.block_on(async {
        let failed = send_one(&fx, create_request("a.bin", &missing, Some("key-fail"))).await;
        assert!(failed.get("Error").is_some(), "the first attempt must fail, got {failed}");
        assert_eq!(queued_operation_count(&fx), 0);
        let retried = send_one(&fx, create_request("a.bin", &good, Some("key-fail"))).await;
        assert_write_queued(&retried);
    });
    assert_eq!(queued_operation_count(&fx), 1);
}
