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

#[path = "ipc_socket_framing_tests/changes_and_thumbnail.rs"]
mod changes_and_thumbnail;
#[path = "ipc_socket_framing_tests/framing_protocol.rs"]
mod framing_protocol;
#[path = "ipc_socket_framing_tests/idempotent_writes.rs"]
mod idempotent_writes;
#[path = "ipc_socket_framing_tests/staged_writes.rs"]
mod staged_writes;
#[path = "ipc_socket_framing_tests/trash_and_cache.rs"]
mod trash_and_cache;

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
        let bridge = Arc::new(EngineBridge::new(&crate::identity::REAL, db.clone(), api));

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
        UnixStream::connect(&self.sock)
            .await
            .expect("connect to test IPC socket")
    }
}

impl Drop for IpcFixture {
    fn drop(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            let _ = cancel.send(());
        }
        if let Some(server) = self.server.take() {
            let _ = self
                .rt
                .block_on(async { tokio::time::timeout(Duration::from_secs(5), server).await });
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
            .unwrap_or_else(|_| {
                panic!(
                    "no delimited reply within {READ_DEADLINE:?} ({} bytes so far)",
                    line.len()
                )
            })
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
    held_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("database lock was not taken");
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

// ---------------------------------------------------------------------------
// Task 1699: FetchThumbnail RPC + retirement of the never-called smart-cache
// stubs (SetFileStatus / RecordOpenedFile / EnforceSmartCache, audit G13).
// ---------------------------------------------------------------------------

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
