//! The HTTP fixture and the shared world builders for the owner-key tests.
//!
//! Owns: a loopback HTTP server (`Fixture`) that records requests, the request parser, the
//! `run`/`record` helpers, and the shared-folder custody world (`prepare_world`, the copy builders,
//! `foreign_child`, and the request counters).
//! Must not: assert anything itself; every assertion lives in the area module that uses it.
//! Tests: none here; `migration`, `owner_copy`, `custody`, `prepare`, `invites` and `index_repair`.

use super::*;

#[derive(Clone)]
pub(super) struct Request {
    pub(super) path: String,
    pub(super) method: String,
    pub(super) bytes: Vec<u8>,
    pub(super) body: Value,
}
pub(super) struct Fixture {
    pub(super) base: String,
    pub(super) stop: Arc<AtomicBool>,
    pub(super) thread: Option<thread::JoinHandle<()>>,
}
impl Fixture {
    pub(super) fn new(mut answer: impl FnMut(Request) -> Option<Value> + Send + 'static) -> Self {
        Self::wire(move |request| answer(request).map(|body| (200, serde_json::to_vec(&body).unwrap())))
    }
    pub(super) fn wire(mut answer: impl FnMut(Request) -> Option<(u16, Vec<u8>)> + Send + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let signal = stop.clone();
        let thread = thread::spawn(move || {
            while !signal.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        // macOS inherits the listener's nonblocking mode.
                        stream.set_nonblocking(false).unwrap();
                        let request = read_request(&mut stream);
                        if let Some((status, bytes)) = answer(request) {
                            let header = format!(
                                "HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                                bytes.len()
                            );
                            stream.write_all(header.as_bytes()).unwrap();
                            stream.write_all(&bytes).unwrap();
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => thread::sleep(Duration::from_millis(2)),
                    Err(e) => panic!("fixture accept: {e}"),
                }
            }
        });
        Self {
            base,
            stop,
            thread: Some(thread),
        }
    }
    pub(super) fn api(&self) -> ApiClient {
        ApiClient::new(self.base.clone(), "fixture-session".into(), [7; 32])
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let result = self.thread.take().unwrap().join();
        if !thread::panicking() {
            result.expect("fixture server panicked");
        }
    }
}
fn read_request(stream: &mut TcpStream) -> Request {
    stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    let mut bytes = Vec::new();
    let end = loop {
        let mut byte = [0];
        assert_eq!(stream.read(&mut byte).unwrap(), 1);
        bytes.push(byte[0]);
        if bytes.ends_with(b"\r\n\r\n") {
            break bytes.len();
        }
    };
    let header = String::from_utf8(bytes.clone()).unwrap();
    let length = header
        .lines()
        .find_map(|line| {
            let (name, val) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| val.trim().parse::<usize>().unwrap())
        })
        .unwrap_or(0);
    bytes.resize(end + length, 0);
    stream.read_exact(&mut bytes[end..]).unwrap();
    let path = header
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .to_string();
    let body = serde_json::from_slice(&bytes[end..]).unwrap_or(Value::Null);
    let method = header.split_whitespace().next().unwrap().to_string();
    Request {
        path,
        method,
        bytes: bytes[end..].to_vec(),
        body,
    }
}
pub(super) fn run<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}
/// A canonical foreign (version 8) id for the stored-record fixtures.
pub(super) const RECORD_ID: &str = "30000000-0000-8000-8000-000000000001";

pub(super) fn record() -> OwnedKeyRecord {
    let master = beebeeb_core::kdf::MasterKey::from_bytes([7; 32]);
    let key = beebeeb_core::kdf::FileKey::from_bytes([0x9d; 32]);
    let envelope = beebeeb_sync::key_resolution::wrap_owner_file_key(&master, RECORD_ID, &key).unwrap();
    OwnedKeyRecord {
        file_id: RECORD_ID.into(),
        name_encrypted: "opaque-name".into(),
        pending_owner_wrap: Some(envelope),
        foreign_added: true,
        ..Default::default()
    }
}

pub(super) fn foreign_child(id: &str, root: &str) -> Value {
    json!({"id":id,"name_encrypted":"opaque-name","parent_id":root,"is_folder":false,
           "version_number":1,"size_bytes":4,"updated_at":1,
           "acted_via_share_id":"fixture-share","uploaded_by":null,
           "owner_wrapped_file_key":null})
}

/// The path the original bug used: `prepare_owned_rows`, not the custody
/// helper (review P2-3). The custody GET answers 404 (record, then winner),
/// the invite list and one invite's rows are served, and the custody PUT
/// answers `put`.
pub(super) fn prepare_world(
    root: &'static str,
    invites: Value,
    folder_keys: Value,
    inventory: Value,
    put: (u16, Value),
    winner: Option<Value>,
) -> (Fixture, Arc<Mutex<Vec<String>>>) {
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let seen_put = Arc::new(Mutex::new(false));
    let (log, put_seen) = (seen.clone(), seen_put.clone());
    let fixture = Fixture::wire(move |req| {
        log.lock().unwrap().push(format!("{} {}", req.method, req.path));
        let (status, body) = if req.path == "/api/v1/shares/invites/sent" {
            (200, invites.clone())
        } else if req.path.ends_with("/folder-keys") {
            (200, folder_keys.clone())
        } else if req.path == format!("/api/v1/shared-folders/{root}/key") && req.method == "PUT" {
            *put_seen.lock().unwrap() = true;
            put.clone()
        } else if req.path == format!("/api/v1/shared-folders/{root}") {
            // The record is absent until the write is attempted; a winner is
            // only visible on the re-read after a conflict.
            match &winner {
                Some(row) if *put_seen.lock().unwrap() => (200, row.clone()),
                _ => (404, json!({})),
            }
        } else if req.path.starts_with(&format!("/api/v1/shared-folders/{root}/keys")) {
            (200, inventory.clone())
        } else {
            (404, json!({}))
        };
        Some((status, serde_json::to_vec(&body).unwrap()))
    });
    (fixture, seen)
}

pub(super) fn count(seen: &Arc<Mutex<Vec<String>>>, needle: &str) -> usize {
    seen.lock().unwrap().iter().filter(|line| line.contains(needle)).count()
}

/// `contains("/shared-folders/{id}")` would also match `/keys`, so exact match
/// the request line when counting a specific endpoint.
pub(super) fn exact(seen: &Arc<Mutex<Vec<String>>>, line: &str) -> usize {
    seen.lock().unwrap().iter().filter(|seen| seen.as_str() == line).count()
}

pub(super) fn forged_name_blob(master: &beebeeb_core::kdf::MasterKey, root: &str) -> String {
    // The folder's own 32-byte name JSON, sealed under the legacy folder key:
    // a server can replay it as an "owner copy" after answering 404.
    let name_json = br#"{"mime_type":null,"name":"Docs"}"#;
    assert_eq!(name_json.len(), 32);
    let derived = beebeeb_core::kdf::derive_file_key(master, root.as_bytes());
    STANDARD.encode(beebeeb_core::encrypt::encrypt_chunk_raw(&derived, name_json).unwrap())
}

pub(super) fn legacy_copy(
    master: &beebeeb_core::kdf::MasterKey,
    root: &str,
    folder: &beebeeb_core::kdf::FileKey,
) -> String {
    let derived = beebeeb_core::kdf::derive_file_key(master, root.as_bytes());
    STANDARD.encode(beebeeb_core::encrypt::encrypt_chunk_raw(&derived, folder.as_bytes()).unwrap())
}

pub(super) fn purpose_bound_copy(
    master: &beebeeb_core::kdf::MasterKey,
    root: &str,
    folder: &beebeeb_core::kdf::FileKey,
) -> String {
    let blob = beebeeb_core::kdf::wrap_invite_owner_copy(master, root.as_bytes(), folder).unwrap();
    let mut copy = blob.nonce;
    copy.extend_from_slice(&blob.ciphertext);
    STANDARD.encode(&copy)
}
