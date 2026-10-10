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
use crate::state_db::{FileEntry, FileStatus, ItemKind, Namespace, OperationKind, PERMISSION_READ, StateDb};

const READ_DEADLINE: Duration = Duration::from_secs(5);

struct IpcFixture {
    rt: tokio::runtime::Runtime,
    sock: std::path::PathBuf,
    db: Arc<StateDb>,
    cancel: Option<tokio::sync::oneshot::Sender<()>>,
    server: Option<tokio::task::JoinHandle<std::io::Result<()>>>,
    _state_dir: tempfile::TempDir,
    _sock_dir: tempfile::TempDir,
    /// Set for a daemon started with `start_staged`.
    staging: Option<tempfile::TempDir>,
    /// The daemon's engine bridge (a test can point its staging folder elsewhere). Only the
    /// macOS staging-folder test reads it.
    #[cfg(target_os = "macos")]
    bridge: Arc<EngineBridge>,
}

impl IpcFixture {
    /// A daemon that takes write contents from any path (the Linux
    /// behaviour, and the shape before the upload-staging directory).
    fn start(seed: impl FnOnce(&StateDb)) -> Self {
        Self::start_with(seed, None)
    }

    /// A daemon that takes write contents ONLY from a throwaway
    /// upload-staging directory and deletes each copy once answered, as the
    /// macOS daemon does with the App Group directory.
    fn start_staged(seed: impl FnOnce(&StateDb)) -> Self {
        Self::start_with(seed, Some(tempfile::tempdir().unwrap()))
    }

    fn start_with(seed: impl FnOnce(&StateDb), staging: Option<tempfile::TempDir>) -> Self {
        // The staging directory is a CHILD of the temp dir, so a test can put a
        // file right next to it (outside it) and reach it with `..`.
        let contents = match &staging {
            Some(root) => {
                let dir = root.path().join("upload-staging");
                std::fs::create_dir(&dir).unwrap();
                crate::ipc_socket::WriteContentsPolicy::StagingDir(dir)
            }
            None => crate::ipc_socket::WriteContentsPolicy::AnyPath,
        };
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
            bridge.clone(),
            cancel_rx,
            Some(ready_tx),
            contents,
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
            staging,
            #[cfg(target_os = "macos")]
            bridge,
        }
    }

    fn staging_dir(&self) -> std::path::PathBuf {
        self.staging
            .as_ref()
            .expect("started with start_staged")
            .path()
            .join("upload-staging")
    }

    /// The temp dir that CONTAINS the staging directory (outside it).
    fn staging_parent(&self) -> &std::path::Path {
        self.staging.as_ref().expect("started with start_staged").path()
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
            .write_all(b"{\"ListFileProviderItems\":{\"container_id\":\"__fp_root__\"}}\n")
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
        assert_eq!(
            items.len(),
            0,
            "the root container enumerates the REAL vault tree (1701): an empty vault lists nothing — the four synthetic namespaces are gone"
        );
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
    // JSON". Pin the shape both in isolation and over the socket. (The probe
    // was `SetFileStatus` until task 1699 retired it; `ReportMaterialized`
    // with an empty list is the remaining `Ok {}` RPC.)
    assert_eq!(
        serde_json::to_string(&crate::ipc_socket::IpcResponse::Ok {}).unwrap(),
        r#"{"Ok":{}}"#
    );
    let fx = IpcFixture::start(|_| {});
    fx.rt.block_on(async {
        let mut client = fx.connect().await;
        client.write_all(b"{\"ReportMaterialized\":{\"container_ids\":[]}}\n").await.unwrap();
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

/// [`modify_request`] carrying a base, as the extension always sends one: the content
/// version it holds for the item (`contentVersion`).
fn modify_request_on_base(file_id: &str, filename: &str, contents_path: &str, base: &str) -> Vec<u8> {
    let line = modify_request(file_id, filename, contents_path, None);
    let mut request: serde_json::Value = serde_json::from_slice(&line).unwrap();
    request["QueueFinderModify"]["base_version_identifier"] = serde_json::json!(base);
    let mut line = serde_json::to_vec(&request).unwrap();
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

/// Park every database writer behind a held lock until the returned sender is
/// dropped or used. While it is held the leader's copy cannot reach the queue,
/// so every request sent meanwhile is GUARANTEED to overlap the in-flight
/// leader: the wait-for-the-first-result path is exercised over the real
/// socket, not left to scheduling (with a tiny source file the leader can finish
/// before the others claim the key, and they then take the cached path instead).
fn hold_database(fx: &IpcFixture) -> (std::sync::mpsc::Sender<()>, std::thread::JoinHandle<()>) {
    let db = fx.db.clone();
    let (held_tx, held_rx) = std::sync::mpsc::channel::<()>();
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    let handle = std::thread::spawn(move || {
        let _guard = db.hold_lock_for_test();
        held_tx.send(()).unwrap();
        let _ = release_rx.recv(); // a message or the sender being dropped
    });
    held_rx.recv_timeout(Duration::from_secs(5)).expect("database lock was not taken");
    (release_tx, handle)
}

/// Send `count` copies of `request` on separate connections while the database
/// is held, prove none has answered (they are all parked behind the leader, or
/// behind the lock), then release and collect the replies.
fn send_overlapping(fx: &IpcFixture, count: usize, request: impl Fn() -> Vec<u8>) -> Vec<serde_json::Value> {
    let (release, holder) = hold_database(fx);
    let replies = fx.rt.block_on(async {
        // Connect everything FIRST: a mutated daemon that runs the work inline
        // parks its workers behind the lock, and a later `connect().await` would
        // then never get polled (a hang instead of a clean red).
        let mut clients = Vec::new();
        for _ in 0..count {
            clients.push(fx.connect().await);
        }
        let mut tasks = Vec::new();
        for mut client in clients {
            let request = request();
            tasks.push(tokio::spawn(async move {
                client.write_all(&request).await.unwrap();
                parse(&read_line(&mut client).await)
            }));
        }
        // A blocking sleep on purpose: if the daemon wrongly runs the work inline
        // on its workers, they are all parked behind the lock and nothing would
        // drive a tokio timer, turning a clean red into a hang.
        std::thread::sleep(Duration::from_millis(400));
        assert!(
            tasks.iter().all(|t| !t.is_finished()),
            "no request may be answered while the first is still copying"
        );
        release.send(()).unwrap();
        let mut out = Vec::new();
        for t in tasks {
            out.push(t.await.unwrap());
        }
        out
    });
    holder.join().unwrap();
    replies
}

#[test]
fn concurrent_creates_with_one_request_id_queue_exactly_one_upload() {
    let fx = IpcFixture::start(|_| {});
    let src = tempfile::tempdir().unwrap();
    let path = source_file(&src, "big.bin");
    let replies = send_overlapping(&fx, 8, || create_request("big.bin", &path, Some("key-concurrent")));
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
    let replies = send_overlapping(&fx, 6, || {
        modify_request("00000000-0000-0000-0000-00000000aaaa", "doc.txt", &path, Some("key-modify"))
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

// ---------------------------------------------------------------------------
// Task 1684 fix round: a Finder delete must never be undone by a cached create.
//
// The REAL delete path (`QueueFinderDelete`) enqueues a `TrashFile` op and leaves
// the `files` row in place (it stays until the snapshot prune or the op echo
// removes it). So `delete_file` — what `a_cached_create_is_not_returned_once_its_
// row_is_gone` uses — is a state the daemon never produces after a user delete.
// These tests drive the real request and the real post-delete database states.
// ---------------------------------------------------------------------------

fn delete_request(file_id: &str) -> Vec<u8> {
    let mut line = serde_json::to_vec(&serde_json::json!({
        "QueueFinderDelete": { "file_id": file_id, "base_version_identifier": null }
    }))
    .unwrap();
    line.push(b'\n');
    line
}

fn operations_of_kind(fx: &IpcFixture, kind: OperationKind) -> Vec<crate::state_db::PendingOperation> {
    fx.db
        .list_due_operations(i64::MAX)
        .unwrap()
        .into_iter()
        .filter(|op| op.kind == kind)
        .collect()
}

fn item_id(reply: &serde_json::Value) -> String {
    reply["WriteQueued"]["item"]["identifier"].as_str().unwrap().to_string()
}

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
        assert!(deleted.get("WriteQueued").is_some(), "the delete must queue, got {deleted}");
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
        // `apply_completed_upload` leaves it.
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
        assert_eq!(retry["WriteQueued"]["item"]["status"], "local", "status must be current: {retry}");
        assert_eq!(
            retry["WriteQueued"]["item"]["version_identifier"], "7:1700000123:20",
            "version must be current: {retry}"
        );
        // The create's op is still queued, so the item is reported as it is now:
        // under the create's token (spec §5.4 row 1). Task 9 turns the status to
        // `uploading`; until then it stays `local` (lead ruling E4.2).
        #[cfg(target_os = "macos")]
        assert_eq!(
            retry["WriteQueued"]["item"]["content_version"], first["WriteQueued"]["item"]["content_version"],
            "the queued create's bytes keep the name the first reply gave them: {retry}"
        );
        // The Linux arm does not mint (Task 3 step 5.7): the reply keeps today's content version.
        #[cfg(not(target_os = "macos"))]
        assert_eq!(
            retry["WriteQueued"]["item"]["content_version"], "7",
            "the write base must be the current server version: {retry}"
        );
        assert_ne!(retry, first);
    });
    assert_eq!(operations_of_kind(&fx, OperationKind::UploadVersion).len(), 1);
}

#[test]
fn a_queued_modify_replies_with_the_size_of_the_bytes_it_was_handed() {
    // The system keeps the bytes it handed over and does not fetch them back,
    // so the item in the reply must describe THOSE bytes, not the row's
    // previous content (28 bytes here; the edit is 40).
    let fx = IpcFixture::start(|db| {
        db.upsert_file(&FileEntry {
            file_id: "edited-item".into(),
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
        let mut contract = db.get_file_contract_state("edited-item").unwrap().unwrap();
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
        modify_request_on_base("edited-item", "t.txt", &path, "1"),
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
    // The Linux arm does not mint (Task 3 step 5.7): the reply keeps today's content version.
    #[cfg(not(target_os = "macos"))]
    assert_eq!(content_version, "1", "{reply}");
    let row = fx.db.get_file("edited-item").unwrap().unwrap();
    assert_eq!(row.size_bytes, 40, "the row records the staged size too");
}

// ---------------------------------------------------------------------------
// Task 1697: ListChanges — the daemon-side change log the replica's
// enumerator pages through, over the real socket.
// ---------------------------------------------------------------------------

fn fp_item(file_id: &str, path: &str, kind: ItemKind) -> FileEntry {
    FileEntry {
        file_id: file_id.into(),
        path: path.into(),
        status: FileStatus::Local,
        size_bytes: 3,
        modified_at: 1,
        content_hash: None,
        remote_updated_at: 1,
        parent_id: None,
        item_kind: kind,
    }
}

fn list_changes_request(since_anchor: Option<&str>) -> Vec<u8> {
    let mut body = serde_json::json!({});
    if let Some(anchor) = since_anchor {
        body["since_anchor"] = serde_json::json!(anchor);
    }
    let mut line = serde_json::to_vec(&serde_json::json!({ "ListChanges": body })).unwrap();
    line.push(b'\n');
    line
}

async fn seed_two_changes(fx: &IpcFixture) {
    // The upserts record their own Created changes now (task 1697 wiring).
    fx.db.upsert_file(&fp_item("fp-a", "/a.txt", ItemKind::File)).unwrap();
    fx.db.upsert_file(&fp_item("fp-b", "/b.txt", ItemKind::File)).unwrap();
}

#[test]
fn list_changes_returns_created_items_and_a_nonempty_anchor() {
    let fx = IpcFixture::start(|_| {});
    fx.rt.block_on(seed_two_changes(&fx));
    fx.rt.block_on(async {
        let reply = send_one(&fx, list_changes_request(None)).await;
        let payload = reply
            .get("FileProviderChanges")
            .unwrap_or_else(|| panic!("expected FileProviderChanges, got {reply}"));
        let changes = payload["changes"].as_array().expect("changes array");
        assert_eq!(changes.len(), 2, "both created items: {payload}");
        assert_eq!(changes[0]["file_id"], "fp-a");
        assert_eq!(changes[0]["kind"], "created");
        let anchor = payload["next_anchor"].as_str().expect("next_anchor string");
        assert!(!anchor.is_empty(), "the anchor after real changes is never empty");
        assert!(anchor.len() <= 500, "Apple caps the anchor at 500 bytes");
    });
}

#[test]
fn list_changes_from_the_fresh_anchor_delivers_nothing_and_keeps_the_anchor() {
    let fx = IpcFixture::start(|_| {});
    fx.rt.block_on(seed_two_changes(&fx));
    fx.rt.block_on(async {
        let first = send_one(&fx, list_changes_request(None)).await;
        let anchor = first["FileProviderChanges"]["next_anchor"].as_str().unwrap().to_string();
        let second = send_one(&fx, list_changes_request(Some(&anchor))).await;
        let payload = &second["FileProviderChanges"];
        assert_eq!(payload["changes"].as_array().map(Vec::len), Some(0), "up to date: {second}");
        assert_eq!(payload["next_anchor"].as_str(), Some(anchor.as_str()), "a no-op poll must not move the anchor");
    });
}

#[test]
fn list_changes_pages_with_resume_tokens_without_losing_or_repeating() {
    let fx = IpcFixture::start(|_| {});
    fx.rt.block_on(async {
        for i in 0..250 {
            let id = format!("fp-{i:03}");
            fx.db.upsert_file(&fp_item(&id, &format!("/{id}"), ItemKind::File)).unwrap();
        }
    });
    fx.rt.block_on(async {
        let mut seen = std::collections::HashSet::new();
        let mut pages = 0;
        let mut cursor: Option<String> = None;
        loop {
            let reply = send_one(&fx, list_changes_request(cursor.as_deref())).await;
            let payload = &reply["FileProviderChanges"];
            let changes = payload["changes"].as_array().expect("changes array");
            assert!(changes.len() <= 100, "page size must be honored, got {}", changes.len());
            for change in changes {
                let key = (
                    change["file_id"].as_str().unwrap().to_string(),
                    change["kind"].as_str().unwrap().to_string(),
                );
                assert!(seen.insert(key), "a change repeated across pages: {change}");
            }
            pages += 1;
            let next = payload["next_anchor"].as_str().map(str::to_string);
            match next {
                Some(a) if Some(&a) == cursor.as_ref() => break, // up to date
                Some(a) => {
                    assert!(a > cursor.clone().unwrap_or_default(), "resume tokens strictly increase");
                    cursor = Some(a);
                }
                None => break,
            }
            assert!(pages < 10, "250 changes at 100/page must take 3 pages");
        }
        assert_eq!(seen.len(), 250, "every change delivered exactly once");
        // 3 delivery pages + 1 final round trip that reports "current" (the
        // echoed anchor breaks the loop).
        assert_eq!(pages, 4, "3 delivery pages + 1 up-to-date round trip, got {pages}");
    });
}

#[test]
fn list_changes_reports_deletes_with_the_old_parent() {
    let fx = IpcFixture::start(|_| {});
    fx.rt.block_on(async {
        fx.db.upsert_file(&fp_item("fp-gone", "/gone.txt", ItemKind::File)).unwrap();
        // delete_file records the Deleted change itself, capturing the row's
        // parent BEFORE the delete (task 1697 wiring). Put the row under
        // parent-9 first so the change carries it.
        fx.db.set_file_contract_state(&crate::state_db::FileContractState {
            file_id: "fp-gone".into(),
            parent_id: Some("parent-9".into()),
            ..crate::state_db::FileContractState {
                file_id: String::new(),
                namespace: crate::state_db::Namespace::MyFiles,
                parent_id: None,
                shared_root_id: None,
                share_id: None,
                owner_email: None,
                permission_bits: 0,
                item_kind: ItemKind::File,
                content_type: None,
                current_version: 0,
                current_object_version_id: None,
                local_base_version: 0,
                local_hash: None,
                cache_path: None,
                cache_bytes: 0,
                pin_state: crate::state_db::PinState::Inherit,
                inherited_pin_state: crate::state_db::PinState::Unpinned,
                last_sync_at: 0,
            }
        })
        .unwrap();
        fx.db.delete_file("fp-gone").unwrap();
        let reply = send_one(&fx, list_changes_request(None)).await;
        let changes = reply["FileProviderChanges"]["changes"].as_array().unwrap();
        let deletion = changes.iter().find(|c| c["kind"] == "deleted").expect("the delete");
        assert_eq!(deletion["file_id"], "fp-gone");
        assert_eq!(deletion["new_parent_id"], "parent-9", "the materialized filter needs the old parent");
    });
}

#[test]
fn list_changes_reports_both_parents_for_reparents() {
    let fx = IpcFixture::start(|_| {});
    fx.rt.block_on(async {
        fx.db.upsert_file(&fp_item("fp-moved", "/old/moved.txt", ItemKind::File)).unwrap();
        fx.db.record_file_change("fp-moved", crate::state_db::FpChangeKind::Created, None).unwrap();
        fx.db
            .record_file_change("fp-moved", crate::state_db::FpChangeKind::Reparented, Some("old-parent".into()))
            .unwrap();
        let reply = send_one(&fx, list_changes_request(None)).await;
        let changes = reply["FileProviderChanges"]["changes"].as_array().unwrap();
        let moved = changes.iter().find(|c| c["kind"] == "reparented").expect("the reparent");
        assert_eq!(moved["old_parent_id"], "old-parent");
    });
}

#[test]
fn an_unparsable_anchor_gets_an_error_not_a_crash() {
    let fx = IpcFixture::start(|_| {});
    fx.rt.block_on(async {
        let reply = send_one(&fx, list_changes_request(Some("garbage-anchor"))).await;
        assert!(reply.get("Error").is_some(), "an expired/garbage anchor is an error: {reply}");
    });
}

// ---------------------------------------------------------------------------
// Task 1699: FetchThumbnail RPC + retirement of the never-called smart-cache
// stubs (SetFileStatus / RecordOpenedFile / EnforceSmartCache, audit G13).
// ---------------------------------------------------------------------------

#[test]
fn retired_rpcs_are_refused_and_the_connection_survives() {
    // Task 1699 audit G13: SetFileStatus (a no-op stub the extension never
    // sent), RecordOpenedFile and EnforceSmartCache (smart-cache bookkeeping
    // with no File Provider caller — the daemon enforces the cache limit
    // internally) are RETIRED from the wire. An unknown variant must be
    // refused per-request (Error reply) without taking the connection down,
    // exactly like any other deserialization failure.
    let fx = IpcFixture::start(|_| {});
    fx.rt.block_on(async {
        let client = fx.connect().await;
        let mut lines = LineReader::new(client);
        for request in [
            r#"{"SetFileStatus":{"file_id":"x","status":"local"}}"#,
            r#"{"RecordOpenedFile":{"file_id":"x","cache_path":"/tmp/x","cache_bytes":1}}"#,
            r#"{"EnforceSmartCache":{"max_unpinned_cache_bytes":null,"disk_pressure_min_free_bytes":null}}"#,
        ] {
            lines.client.write_all(format!("{request}\n").as_bytes()).await.unwrap();
            let reply = parse(&lines.next_line().await);
            let message = reply["Error"]["message"].as_str().unwrap_or_default().to_string();
            assert!(message.contains("unknown variant"), "retired RPC must be refused, got {reply}");
        }
        lines.client.write_all(b"\"GetSyncSummary\"\n").await.unwrap();
        let next = parse(&lines.next_line().await);
        assert!(next.get("SyncSummary").is_some(), "connection must still be usable: {next}");
    });
}

#[test]
fn thumbnail_variant_buckets_match_the_windows_picker() {
    // The variant picker mirrors windows_cf/thumbnail_provider.rs: a
    // requested max dimension of <=96 px gets the "small" server variant,
    // <=256 px "medium", anything larger "large".
    assert_eq!(crate::ipc_socket::thumbnail_variant(1), "small");
    assert_eq!(crate::ipc_socket::thumbnail_variant(96), "small");
    assert_eq!(crate::ipc_socket::thumbnail_variant(97), "medium");
    assert_eq!(crate::ipc_socket::thumbnail_variant(256), "medium");
    assert_eq!(crate::ipc_socket::thumbnail_variant(257), "large");
    assert_eq!(crate::ipc_socket::thumbnail_variant(u32::MAX), "large");
}

#[test]
fn persist_thumbnail_plaintext_writes_atomically_inside_allowed_roots() {
    let dir = tempfile::tempdir().unwrap();
    let roots = vec![dir.path().to_path_buf()];
    let dest = dir.path().join("thumb.bin");
    let bytes = b"thumbnail-plaintext";
    let written = crate::ipc_socket::persist_thumbnail_plaintext(&dest, &roots, bytes).unwrap();
    assert_eq!(written, bytes.len() as u64);
    assert_eq!(std::fs::read(&dest).unwrap(), bytes, "the renamed file must carry the plaintext");
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(&dest).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600, "thumbnail staging plaintext must be owner-only");
    let entries: Vec<_> = std::fs::read_dir(dir.path()).unwrap().filter_map(|e| e.ok()).collect();
    assert_eq!(entries.len(), 1, "no `.part` staging leftovers may remain");
}

#[test]
fn persist_thumbnail_plaintext_rejects_destinations_outside_allowed_roots() {
    let dir = tempfile::tempdir().unwrap();
    let outside = std::env::temp_dir().join("1699-persist-outside-thumb.bin");
    let roots = vec![dir.path().to_path_buf()];
    let err = crate::ipc_socket::persist_thumbnail_plaintext(&outside, &roots, b"x").unwrap_err();
    assert!(err.to_string().contains("allowed root"), "got {err}");
    assert!(!outside.exists(), "nothing may be written outside an allowed root");
}

// Task 1699 review (PR #103, thread PRRT_kwDOSLX6Xs6oeNfW, P1): the staging
// write must never FOLLOW a pre-created `<dest>.part` symlink. The old plain
// `File::create(&staging)` did: a caller that can reach the IPC socket plants
// `<dest>.part` as a symlink to an arbitrary daemon-accessible file, the
// parent-only containment check passes, and the plaintext truncates/overwrites
// the target outside every allowed root. The anchored staging writer (task
// 1247 / 1670-round-3 pattern: O_NOFOLLOW descent + fresh temp leaf +
// anchored renameat) can only REPLACE a planted path, never write through it.
#[test]
fn persist_thumbnail_plaintext_never_writes_through_a_planted_staging_symlink() {
    let dir = tempfile::tempdir().unwrap();
    let outside_dir = tempfile::tempdir().unwrap();
    let outside = outside_dir.path().join("victim.txt");
    std::fs::write(&outside, b"DO-NOT-TOUCH").unwrap();
    let dest = dir.path().join("thumb.bin");
    let staging = dir.path().join("thumb.bin.part");
    std::os::unix::fs::symlink(&outside, &staging).unwrap();
    let roots = vec![dir.path().to_path_buf()];
    let written = crate::ipc_socket::persist_thumbnail_plaintext(&dest, &roots, b"payload")
        .expect("a normal dest inside an allowed root must still succeed");
    assert_eq!(written, b"payload".len() as u64);
    assert_eq!(
        std::fs::read(&outside).unwrap(),
        b"DO-NOT-TOUCH",
        "the pre-created staging symlink must never divert the write outside the allowed roots"
    );
    let meta = std::fs::symlink_metadata(&dest).unwrap();
    assert!(
        meta.file_type().is_file(),
        "the published dest must be a REAL file, not the planted symlink moved into place"
    );
    assert_eq!(std::fs::read(&dest).unwrap(), b"payload");
}

#[test]
fn fetch_thumbnail_rejects_destination_outside_allowed_roots() {
    // The dest_path arrives straight off the wire (untrusted), exactly like
    // HydrateFile's: it must be bounded to the allowed roots BEFORE the
    // daemon does anything else with the request.
    let fx = IpcFixture::start(|_| {});
    fx.rt.block_on(async {
        let mut client = fx.connect().await;
        client
            .write_all(
                b"{\"FetchThumbnail\":{\"file_id\":\"00000000-0000-0000-0000-000000000001\",\"dest_path\":\"/tmp/1699-fpfs-outside/thumb.bin\",\"max_dimension\":256}}\n",
            )
            .await
            .unwrap();
        let reply = parse(&read_line(&mut client).await);
        let message = reply["Error"]["message"].as_str().unwrap_or_default();
        assert!(message.contains("allowed root"), "got {reply}");
    });
}

#[test]
fn fetch_thumbnail_reports_daemon_side_failures_without_writing_the_dest() {
    // The fixture's ApiClient points at an unreachable port, so a request
    // that survives identifier + destination validation must surface the
    // daemon-side failure as an Error reply and leave NO file at dest —
    // the extension must never read a stale or partial staging file.
    let fx = IpcFixture::start(|_| {});
    fx.rt.block_on(async {
        let dest = std::env::temp_dir().join("1699-thumb-should-not-exist.bin");
        let _ = std::fs::remove_file(&dest);
        let request = format!(
            "{{\"FetchThumbnail\":{{\"file_id\":\"00000000-0000-0000-0000-000000000001\",\"dest_path\":\"{}\",\"max_dimension\":256}}}}\n",
            dest.display()
        );
        let mut client = fx.connect().await;
        client.write_all(request.as_bytes()).await.unwrap();
        let reply = parse(&read_line(&mut client).await);
        let message = reply["Error"]["message"].as_str().unwrap_or_default();
        assert!(
            message.contains("thumbnail") && !message.contains("unknown variant"),
            "the arm must reach the daemon-side fetch, got {reply}"
        );
        assert!(!dest.exists(), "a failed fetch must not leave a staging file");
        let _ = std::fs::remove_file(&dest);
    });
}

#[test]
fn list_changes_ride_along_full_item_payloads_for_updates() {
    // The replica's didUpdateItems needs the FULL item; a second GetFileStatus
    // round-trip per change would triple the polling cost. The change payload
    // carries it for created/modified/reparented rows while the row still
    // exists, and nothing for deletions (or for rows that vanished between
    // the change and the poll — the Swift enumerator skips those).
    let fx = IpcFixture::start(|_| {});
    fx.rt.block_on(async {
        fx.db.upsert_file(&fp_item("fp-ride", "/ride.txt", ItemKind::File)).unwrap();
        fx.db.upsert_file(&fp_item("fp-live", "/live.txt", ItemKind::File)).unwrap();
        fx.db.delete_file("fp-ride").unwrap();
        let reply = send_one(&fx, list_changes_request(None)).await;
        let changes = reply["FileProviderChanges"]["changes"].as_array().unwrap();
        let updated = changes.iter().find(|c| c["kind"] == "created" && c["file_id"] == "fp-live").expect("the live created change");
        let item = updated["item"].as_object().expect("created rows carry the full item payload");
        assert_eq!(item["identifier"], "fp-live");
        assert_eq!(item["status"], "local");
        // A created change whose row has since been deleted rides no item.
        let vanished = changes.iter().find(|c| c["kind"] == "created" && c["file_id"] == "fp-ride").expect("the vanished created change");
        assert!(vanished["item"].is_null(), "a row gone since the change yields no item");
        let deleted = changes.iter().find(|c| c["kind"] == "deleted").expect("the deleted change");
        assert!(deleted["item"].is_null(), "deletions carry no item payload");
    });
}

// ---------------------------------------------------------------------------
// Upload staging: on macOS the extension hands write contents over as a copy in the
// App Group upload-staging directory (the daemon cannot read the system's own
// contents URL). The daemon must take contents from there ONLY, upload from
// its OWN copy, and delete the handed-over copy once the request is answered,
// including when the request is refused after the contents were accepted.
// ---------------------------------------------------------------------------

/// A handed-over copy, named the way the extension names it (a random UUID,
/// never the user's file name).
fn staged_copy(fx: &IpcFixture, bytes: &[u8]) -> std::path::PathBuf {
    let path = fx.staging_dir().join(uuid::Uuid::new_v4().to_string());
    std::fs::write(&path, bytes).unwrap();
    path
}

fn staging_entries(fx: &IpcFixture) -> Vec<std::path::PathBuf> {
    std::fs::read_dir(fx.staging_dir())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect()
}

fn create_request_in(parent_id: &str, filename: &str, contents_path: &str, request_id: Option<&str>) -> Vec<u8> {
    let mut body = serde_json::json!({
        "parent_id": parent_id,
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

/// The daemon's own copy behind the one queued upload: it must exist, hold the
/// handed-over bytes, and live outside the staging directory.
fn assert_uploads_from_own_copy(fx: &IpcFixture, expected: &[u8]) {
    let uploads = operations_of_kind(fx, OperationKind::UploadVersion);
    assert_eq!(uploads.len(), 1, "exactly one upload must be queued");
    let own = uploads[0]
        .payload_path
        .as_deref()
        .expect("the upload carries the daemon's own copy");
    let own_parent = std::fs::canonicalize(std::path::Path::new(own).parent().unwrap()).unwrap();
    assert_ne!(
        own_parent,
        std::fs::canonicalize(fx.staging_dir()).unwrap(),
        "the upload must read the daemon's own copy, not the handed-over one"
    );
    assert_eq!(
        std::fs::read(own).unwrap(),
        expected,
        "the daemon's copy holds the handed-over bytes"
    );
}

fn seed_read_only_shared_folder(db: &StateDb, file_id: &str) {
    db.upsert_file(&FileEntry {
        file_id: file_id.into(),
        path: "Read-only share".into(),
        status: FileStatus::Local,
        size_bytes: 0,
        modified_at: 1,
        content_hash: None,
        remote_updated_at: 1,
        parent_id: None,
        item_kind: ItemKind::Folder,
    })
    .unwrap();
    let mut contract = db.get_file_contract_state(file_id).unwrap().unwrap();
    contract.namespace = Namespace::SharedWithMe;
    contract.shared_root_id = Some(file_id.into());
    contract.share_id = Some(format!("invite-{file_id}"));
    contract.permission_bits = PERMISSION_READ;
    contract.item_kind = ItemKind::Folder;
    db.set_file_contract_state(&contract).unwrap();
}

#[test]
fn staged_create_uploads_from_the_daemons_own_copy_and_deletes_the_handed_over_one() {
    let fx = IpcFixture::start_staged(|_| {});
    let copy = staged_copy(&fx, b"finder file contents");
    fx.rt.block_on(async {
        let reply = send_one(
            &fx,
            create_request("notes.txt", &copy.to_string_lossy(), Some("key-staged")),
        )
        .await;
        assert_write_queued(&reply);
    });
    assert!(
        !copy.exists(),
        "the handed-over copy must be deleted once the daemon has its own"
    );
    assert!(
        staging_entries(&fx).is_empty(),
        "nothing may be left in the staging dir"
    );
    assert_uploads_from_own_copy(&fx, b"finder file contents");
}

/// Spec 2026-10-09 §8.4 (the staging folder, macOS): when the app's own staging folder cannot be
/// created or written to, a create and a modify are answered `WriteRetryLater` with a fixed reason and
/// no path. The extension reports that as a transient error, so the system keeps the change and
/// retries the write. Nothing is staged anywhere, nothing is queued, no row is added, and the
/// handed-over copy is deleted as after any answer. Once the folder can take a copy again, the
/// system's retry with the same request id is queued, exactly once.
#[cfg(target_os = "macos")]
#[test]
fn an_unwritable_staging_folder_answers_write_retry_later_and_stages_and_queues_nothing() {
    const FILE_ID: &str = "00000000-0000-0000-0000-00000000cccc";
    let fx = IpcFixture::start_staged(|db| {
        db.upsert_file(&FileEntry {
            file_id: FILE_ID.into(),
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
    let bases_dir = tempfile::tempdir().unwrap();
    let data = bases_dir.path().join("data");
    std::fs::create_dir(&data).unwrap();
    // A file where the staging folder's parent would be: `beebeeb/finder-writes` cannot be created.
    std::fs::write(data.join("beebeeb"), b"in the way").unwrap();
    fx.bridge.seams.stage_under(crate::engine_bridge::FinderStagingBases {
        data: Some(data.clone()),
        cache: Some(bases_dir.path().join("cache")),
        temp: bases_dir.path().join("temp"),
    });
    let rows_before = fx.db.list_files().unwrap().len();

    for kind in ["create", "modify"] {
        let copy = staged_copy(&fx, b"a save the app cannot stage");
        let contents = copy.to_string_lossy().into_owned();
        let request = match kind {
            "create" => create_request("new.txt", &contents, Some("key-retry-create")),
            _ => modify_request_on_base(FILE_ID, "doc.txt", &contents, "1"),
        };
        let reply = fx.rt.block_on(send_one(&fx, request));
        let message = reply["WriteRetryLater"]["message"]
            .as_str()
            .unwrap_or_else(|| panic!("{kind}: expected WriteRetryLater, got {reply}"));
        assert_eq!(
            reply,
            serde_json::json!({ "WriteRetryLater": { "message": message } }),
            "{kind}: the wire shape"
        );
        assert!(
            message.starts_with("the staging folder is unavailable") && !message.contains('/'),
            "{kind}: a fixed reason, no path: {message}"
        );
        assert!(
            !copy.exists(),
            "{kind}: the handed-over copy is deleted after the answer"
        );
    }

    assert_eq!(queued_operation_count(&fx), 0, "nothing is queued");
    assert!(
        fx.db.staged_payloads_for_signout().unwrap().is_empty(),
        "nothing is journalled"
    );
    assert_eq!(
        fx.db.list_files().unwrap().len(),
        rows_before,
        "the create added no row"
    );
    let mut left = Vec::new();
    let mut stack = vec![bases_dir.path().to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                left.push(path);
            }
        }
    }
    assert_eq!(left, vec![data.join("beebeeb")], "no copy was written anywhere");
    assert!(staging_entries(&fx).is_empty(), "nothing is left in upload staging");

    // The folder can take a copy again. The system's retry carries the SAME request id: the
    // refusal was not remembered, so the retry is accepted and queued, and a repeat of it is
    // answered from that result, so the save is queued exactly once.
    std::fs::remove_file(data.join("beebeeb")).unwrap();
    for attempt in ["the retry", "a repeat of the retry"] {
        let copy = staged_copy(&fx, b"a save the app could not stage before");
        let reply = fx.rt.block_on(send_one(
            &fx,
            create_request("new.txt", &copy.to_string_lossy(), Some("key-retry-create")),
        ));
        assert!(
            reply.get("WriteQueued").is_some(),
            "{attempt}: expected WriteQueued, got {reply}"
        );
        assert_eq!(
            queued_operation_count(&fx),
            1,
            "{attempt}: the save is queued exactly once"
        );
    }
    let ops = fx.db.list_due_operations(i64::MAX).unwrap();
    let own = ops[0].payload_path.as_deref().expect("the upload carries its copy");
    assert!(
        std::path::Path::new(own).starts_with(data.join("beebeeb").join("finder-writes")),
        "the copy is staged in the data root: {own}"
    );
}

#[test]
fn staged_modify_uploads_from_the_daemons_own_copy_and_deletes_the_handed_over_one() {
    let fx = IpcFixture::start_staged(|db| {
        db.upsert_file(&FileEntry {
            file_id: "00000000-0000-0000-0000-00000000bbbb".into(),
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
    let copy = staged_copy(&fx, b"edited contents");
    fx.rt.block_on(async {
        let reply = send_one(
            &fx,
            modify_request(
                "00000000-0000-0000-0000-00000000bbbb",
                "doc.txt",
                &copy.to_string_lossy(),
                Some("key-staged-modify"),
            ),
        )
        .await;
        assert!(reply.get("WriteQueued").is_some(), "expected WriteQueued, got {reply}");
    });
    assert!(
        !copy.exists(),
        "the handed-over copy must be deleted once the daemon has its own"
    );
    assert!(staging_entries(&fx).is_empty());
    assert_uploads_from_own_copy(&fx, b"edited contents");
}

#[test]
fn contents_not_directly_in_the_staging_dir_are_refused_with_their_category_and_never_deleted() {
    let fx = IpcFixture::start_staged(|_| {});
    // A real file right next to the staging dir: outside it, reachable by `..`.
    let outside = fx.staging_parent().join("outside.txt");
    std::fs::write(&outside, b"not handed over").unwrap();
    let traversal = fx.staging_dir().join("..").join("outside.txt");
    let link = fx.staging_dir().join("link");
    std::os::unix::fs::symlink(&outside, &link).unwrap();
    let missing = fx.staging_dir().join("never-staged");
    let cases = [
        (outside.clone(), "outside_staging"),
        (traversal, "traversal"),
        (link.clone(), "symlink"),
        (missing, "missing"),
    ];
    fx.rt.block_on(async {
        for (path, category) in &cases {
            let reply = send_one(&fx, create_request("a.txt", &path.to_string_lossy(), None)).await;
            let message = reply["Error"]["message"].as_str().unwrap_or_default();
            assert!(
                message.contains(category),
                "{category}: the refusal must name its category, got {reply}"
            );
            assert!(
                !message.contains("outside.txt"),
                "a refusal must not echo the path, got {reply}"
            );
        }
    });
    assert_eq!(
        std::fs::read(&outside).unwrap(),
        b"not handed over",
        "a refused path is never deleted"
    );
    assert!(
        std::fs::symlink_metadata(&link).is_ok(),
        "a refused entry is left for the age-bound purge, not deleted on the request path"
    );
    assert_eq!(queued_operation_count(&fx), 0, "a refused request must queue nothing");
}

#[test]
fn a_write_refused_after_its_contents_were_accepted_still_deletes_the_handed_over_copy() {
    let fx = IpcFixture::start_staged(|db| seed_read_only_shared_folder(db, "shared-read-only"));
    // 1. The engine refuses: a new file in a read-only shared folder.
    let engine_refused = staged_copy(&fx, b"a");
    // 2. The socket refuses: the "Shared with me" namespace root.
    let namespace_refused = staged_copy(&fx, b"b");
    // 3. The idempotency key is unusable.
    let key_refused = staged_copy(&fx, b"c");
    fx.rt.block_on(async {
        let reply = send_one(
            &fx,
            create_request_in(
                "shared-read-only",
                "a.txt",
                &engine_refused.to_string_lossy(),
                Some("key-ro"),
            ),
        )
        .await;
        assert!(reply.get("Error").is_some(), "the engine must refuse, got {reply}");
        let reply = send_one(
            &fx,
            create_request_in(
                "namespace:shared_with_me",
                "b.txt",
                &namespace_refused.to_string_lossy(),
                None,
            ),
        )
        .await;
        assert!(
            reply.get("Error").is_some(),
            "the namespace root must refuse, got {reply}"
        );
        let reply = send_one(&fx, create_request("c.txt", &key_refused.to_string_lossy(), Some(""))).await;
        assert!(
            reply.get("Error").is_some(),
            "an empty key must be refused, got {reply}"
        );
    });
    for copy in [&engine_refused, &namespace_refused, &key_refused] {
        assert!(
            !copy.exists(),
            "a refused request's handed-over copy must be deleted: {}",
            copy.display()
        );
    }
    assert_eq!(queued_operation_count(&fx), 0);
}

#[test]
fn concurrent_staged_creates_with_one_request_id_queue_one_upload_and_delete_every_copy() {
    // Each retry of a timed-out create hands over its OWN copy. Only the first
    // is read; every copy, read or not, must be gone once its request is
    // answered.
    let fx = IpcFixture::start_staged(|_| {});
    let staging = fx.staging_dir();
    let replies = send_overlapping(&fx, 6, || {
        let copy = staging.join(uuid::Uuid::new_v4().to_string());
        std::fs::write(&copy, b"big file").unwrap();
        create_request("big.bin", &copy.to_string_lossy(), Some("key-staged-concurrent"))
    });
    assert_eq!(replies.len(), 6);
    for r in &replies {
        assert_write_queued(r);
        assert_eq!(r, &replies[0], "every repeat must get the SAME WriteQueued reply");
    }
    assert!(
        staging_entries(&fx).is_empty(),
        "every handed-over copy must be deleted"
    );
    assert_uploads_from_own_copy(&fx, b"big file");
}

#[test]
fn a_create_still_in_flight_when_the_socket_stops_keeps_its_copy_until_the_engine_has_read_it() {
    // The first attempt of a keyed write runs detached from its connection (the
    // engine finishes it even if the client is gone). When the socket stops,
    // every connection task is aborted; the handed-over copy must stay until
    // that detached work has read it, or the create fails after the fact.
    let mut fx = IpcFixture::start_staged(|_| {});
    let copy = staged_copy(&fx, b"in flight");
    let (release, holder) = hold_database(&fx);
    fx.rt.block_on(async {
        let mut client = fx.connect().await;
        client
            .write_all(&create_request(
                "late.txt",
                &copy.to_string_lossy(),
                Some("key-in-flight"),
            ))
            .await
            .unwrap();
        // Blocking on purpose (see `send_overlapping`): let the work reach the held lock.
        std::thread::sleep(Duration::from_millis(300));
    });
    let _ = fx.cancel.take().expect("server running").send(());
    let server = fx.server.take().expect("server running");
    fx.rt.block_on(async {
        tokio::time::timeout(Duration::from_secs(5), server)
            .await
            .expect("the socket stops")
            .expect("server task")
            .expect("server result");
        // Let the aborted connection tasks be dropped.
        tokio::time::sleep(Duration::from_millis(200)).await;
    });
    assert!(
        copy.exists(),
        "the copy must outlive the stopped connection while the engine still needs it"
    );
    release.send(()).unwrap();
    holder.join().unwrap();
    let deadline = std::time::Instant::now() + READ_DEADLINE;
    while (copy.exists() || queued_operation_count(&fx) == 0) && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_uploads_from_own_copy(&fx, b"in flight");
    assert!(
        !copy.exists(),
        "once the engine has its own copy the handed-over one is deleted"
    );
}

#[test]
fn a_staged_copy_swapped_for_a_symlink_after_validation_uploads_what_was_handed_over() {
    // The daemon validates the handed-over copy, then the engine reads it,
    // later and on another thread. Whatever happens to the entry in between
    // (here: renamed away and replaced by a symlink to a file the requester
    // never handed over), the engine must read the file that was validated.
    let fx = IpcFixture::start_staged(|_| {});
    let copy = staged_copy(&fx, b"handed over");
    let secret = fx.staging_parent().join("not-handed-over.txt");
    std::fs::write(&secret, b"never handed over").unwrap();
    let (release, holder) = hold_database(&fx);
    let reply = fx.rt.block_on(async {
        let mut client = fx.connect().await;
        client
            .write_all(&create_request("swapped.txt", &copy.to_string_lossy(), Some("key-swapped")))
            .await
            .unwrap();
        // Blocking on purpose (see `send_overlapping`): the request is
        // validated, and its work then waits behind the held database.
        std::thread::sleep(Duration::from_millis(300));
        std::fs::rename(&copy, fx.staging_dir().join("moved-away")).unwrap();
        std::os::unix::fs::symlink(&secret, &copy).unwrap();
        release.send(()).unwrap();
        parse(&read_line(&mut client).await)
    });
    holder.join().unwrap();
    assert_write_queued(&reply);
    assert_uploads_from_own_copy(&fx, b"handed over");
    assert_eq!(
        std::fs::read(&secret).unwrap(),
        b"never handed over",
        "the symlink's target is never touched"
    );
}
