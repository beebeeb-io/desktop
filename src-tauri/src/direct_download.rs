//! Direct S3 download driver (task 2173).
//!
//! When the server offers `object-download-v1`, a whole-file hydrate fetches the file's encrypted
//! chunks straight from object storage through short-lived presigned links, and decrypts them
//! through the same core chunk_stream reader as the API path. The API only authorizes and signs.
//!
//! The rules (which links may be followed, when to renew, when to fall back) live in
//! `beebeeb_core::direct_transfer` and are shared by every client. This module is the desktop's
//! half: it performs the HTTP, keeps the planner's bookkeeping and decrypts chunks IN ORDER into
//! one [`Zeroizing`] buffer, which the caller publishes only after the whole file authenticated.
//! On any fallback the buffer is dropped (and wiped) before the API path starts, so two transports
//! never write into one result.
//!
//! What stays on the API: chunk RANGES (the Windows Cloud Files range read; the planner always
//! starts at chunk 0), thumbnails, version history, uploads (until the server's upload grants
//! ship). No URL is ever logged or put in an error: a storage failure is a status number, and the
//! decision lines name chunk numbers and statuses only.

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use beebeeb_core::direct_transfer as dt;
use futures_util::stream::{FuturesUnordered, StreamExt};
use zeroize::{Zeroize, Zeroizing};

use crate::api_client::{ApiClient, ChunkApiError, GrantReply};


















/// A loopback API and storage pair that speaks the direct-transfer contract, with scripted faults.
/// Shared by this module's tests and the engine-level tests in `engine_bridge.rs`.
#[cfg(test)]
pub(crate) mod test_support {
    use std::collections::{HashMap, VecDeque};
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    use serde_json::{Value, json};

    pub(crate) const BUCKET: &str = "bb-test";
    /// What every signed link carries; a test that greps for it proves no link got out.
    pub(crate) const SIGNATURE: &str = "X-Amz-Signature";
    pub(crate) const SECRET: &str = "SECRETSIG2173";

    /// One request a mock saw.
    #[derive(Debug, Clone)]
    pub(crate) struct Seen {
        pub method: String,
        /// Path and query, exactly as sent.
        pub target: String,
        pub headers: Vec<(String, String)>,
        pub body: Vec<u8>,
        /// When the request's head arrived.
        pub at: std::time::Instant,
    }

    impl Seen {
        pub(crate) fn path(&self) -> &str {
            self.target.split('?').next().unwrap_or("")
        }
        pub(crate) fn query(&self) -> &str {
            self.target.split_once('?').map(|(_, q)| q).unwrap_or("")
        }
        pub(crate) fn header(&self, name: &str) -> Option<&str> {
            self.headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(name))
                .map(|(_, v)| v.as_str())
        }
    }

    pub(crate) struct Reply {
        status: u16,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
        hang_up: bool,
        /// Send this many bytes of the response, then go silent for this long (a half-open
        /// connection), then close.
        stall: Option<(usize, std::time::Duration)>,
        /// Send the body this many bytes at a time, with this pause between pieces (a slow link).
        trickle: Option<(usize, std::time::Duration)>,
    }

    impl Reply {
        pub(crate) fn json(status: u16, value: Value) -> Self {
            Reply {
                status,
                headers: vec![("Content-Type".into(), "application/json".into())],
                body: value.to_string().into_bytes(),
                hang_up: false,
                stall: None,
                trickle: None,
            }
        }
        pub(crate) fn bytes(body: Vec<u8>) -> Self {
            Reply {
                status: 200,
                headers: vec![("Content-Type".into(), "application/octet-stream".into())],
                body,
                hang_up: false,
                stall: None,
                trickle: None,
            }
        }
        /// An empty body with no content type: axum's unknown-route answer.
        pub(crate) fn bare(status: u16) -> Self {
            Reply {
                status,
                headers: Vec::new(),
                body: Vec::new(),
                hang_up: false,
                stall: None,
                trickle: None,
            }
        }
        pub(crate) fn redirect(location: String) -> Self {
            Reply {
                status: 302,
                headers: vec![("Location".into(), location)],
                body: Vec::new(),
                hang_up: false,
                stall: None,
                trickle: None,
            }
        }
        /// Close the connection without an answer.
        pub(crate) fn hang_up() -> Self {
            Reply {
                status: 0,
                headers: Vec::new(),
                body: Vec::new(),
                hang_up: true,
                stall: None,
                trickle: None,
            }
        }
        /// `self`, but after `sent` bytes of it the connection goes silent for `silence`.
        pub(crate) fn stalling(mut self, sent: usize, silence: std::time::Duration) -> Self {
            self.stall = Some((sent, silence));
            self
        }
        /// `self`, but its body is sent `piece` bytes at a time with `pause` between pieces.
        pub(crate) fn trickling(mut self, piece: usize, pause: std::time::Duration) -> Self {
            self.trickle = Some((piece.max(1), pause));
            self
        }
        pub(crate) fn with_header(mut self, name: &str, value: &str) -> Self {
            self.headers.push((name.into(), value.into()));
            self
        }
    }

    type Handler = Arc<dyn Fn(&Seen) -> Reply + Send + Sync>;

    /// A threaded HTTP/1.1 server on 127.0.0.1 that answers every request with `handler` and
    /// records it. One thread per connection; every answer closes its connection.
    pub(crate) struct Mock {
        pub base: String,
        pub authority: String,
        seen: Arc<Mutex<Vec<Seen>>>,
        stop: Arc<AtomicBool>,
    }

    impl Mock {
        pub(crate) fn start(handler: Handler) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let authority = listener.local_addr().unwrap().to_string();
            let seen: Arc<Mutex<Vec<Seen>>> = Arc::default();
            let stop = Arc::new(AtomicBool::new(false));
            let (seen2, stop2) = (seen.clone(), stop.clone());
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    if stop2.load(Ordering::SeqCst) {
                        break;
                    }
                    let Ok(stream) = stream else { continue };
                    let (handler, seen) = (handler.clone(), seen2.clone());
                    std::thread::spawn(move || serve(stream, &handler, &seen));
                }
            });
            Mock {
                base: format!("http://{authority}"),
                authority,
                seen,
                stop,
            }
        }

        pub(crate) fn seen(&self) -> Vec<Seen> {
            self.seen.lock().unwrap().clone()
        }

        pub(crate) fn count(&self, pred: impl Fn(&Seen) -> bool) -> usize {
            self.seen().iter().filter(|s| pred(s)).count()
        }
    }

    impl Drop for Mock {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            let _ = TcpStream::connect(&self.authority);
        }
    }

    fn serve(mut stream: TcpStream, handler: &Handler, seen: &Mutex<Vec<Seen>>) {
        let mut buf = Vec::new();
        let mut tmp = [0u8; 8192];
        let head_end = loop {
            let Ok(n) = stream.read(&mut tmp) else { return };
            if n == 0 {
                return;
            }
            buf.extend_from_slice(&tmp[..n]);
            if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                break p + 4;
            }
        };
        let arrived = std::time::Instant::now();
        let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
        let mut lines = head.split("\r\n");
        let mut first = lines.next().unwrap_or("").split_whitespace();
        let method = first.next().unwrap_or("").to_string();
        let target = first.next().unwrap_or("").to_string();
        let headers: Vec<(String, String)> = lines
            .filter_map(|l| l.split_once(':'))
            .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
            .collect();
        let length: usize = headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
            .and_then(|(_, v)| v.parse().ok())
            .unwrap_or(0);
        let mut body = buf[head_end..].to_vec();
        while body.len() < length {
            let Ok(n) = stream.read(&mut tmp) else { return };
            if n == 0 {
                break;
            }
            body.extend_from_slice(&tmp[..n]);
        }
        let request = Seen {
            method,
            target,
            headers,
            body,
            at: arrived,
        };
        let reply = handler(&request);
        seen.lock().unwrap().push(request);
        if reply.hang_up {
            return;
        }
        let mut out = format!("HTTP/1.1 {} X\r\n", reply.status).into_bytes();
        for (k, v) in &reply.headers {
            out.extend_from_slice(format!("{k}: {v}\r\n").as_bytes());
        }
        out.extend_from_slice(format!("Content-Length: {}\r\nConnection: close\r\n\r\n", reply.body.len()).as_bytes());
        out.extend_from_slice(&reply.body);
        if let Some((sent, silence)) = reply.stall {
            let _ = stream.write_all(&out[..sent.min(out.len())]);
            let _ = stream.flush();
            std::thread::sleep(silence);
            return;
        }
        if let Some((piece, pause)) = reply.trickle {
            let head_len = out.len() - reply.body.len();
            if stream.write_all(&out[..head_len]).is_err() {
                return;
            }
            for part in reply.body.chunks(piece) {
                std::thread::sleep(pause);
                if stream.write_all(part).and_then(|()| stream.flush()).is_err() {
                    return;
                }
            }
            return;
        }
        let _ = stream.write_all(&out);
        let _ = stream.flush();
    }

    /// One version of one file, encrypted exactly as the API serves it.
    #[derive(Clone)]
    pub(crate) struct Version {
        pub id: String,
        pub stride: u64,
        pub plain: Vec<u8>,
        pub frames: Vec<Vec<u8>>,
    }

    /// Deterministic, incompressible content.
    pub(crate) fn content(len: usize, seed: u64) -> Vec<u8> {
        let mut x = seed | 1;
        (0..len)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                x as u8
            })
            .collect()
    }

    pub(crate) fn encrypt_frame(file_key: &beebeeb_core::kdf::FileKey, plain: &[u8]) -> Vec<u8> {
        beebeeb_core::encrypt::encrypt_chunk_raw(file_key, plain).unwrap()
    }

    impl Version {
        pub(crate) fn new(file_key: &beebeeb_core::kdf::FileKey, id: &str, stride: u64, plain: Vec<u8>) -> Self {
            let frames = plain
                .chunks(stride as usize)
                .map(|piece| encrypt_frame(file_key, piece))
                .collect();
            Version {
                id: id.to_string(),
                stride,
                plain,
                frames,
            }
        }
        pub(crate) fn chunk_count(&self) -> u32 {
            self.frames.len() as u32
        }
        pub(crate) fn encrypted_size(&self) -> u64 {
            self.frames.iter().map(|f| f.len() as u64).sum()
        }
        /// `download_version` exactly as the server builds it (task 1643 contract).
        pub(crate) fn manifest(&self) -> Value {
            let chunks: Vec<Value> = self
                .frames
                .iter()
                .enumerate()
                .map(|(i, f)| json!({"index": i, "size_bytes": f.len()}))
                .collect();
            json!({
                "object_version_id": self.id, "version_number": 1, "chunk_count": self.chunk_count(),
                "chunk_size_bytes": self.stride, "plaintext_size_bytes": self.plain.len(),
                "ciphertext_size_bytes": self.encrypted_size(), "chunks": chunks,
            })
        }
    }

    impl Version {
        /// The pin a download of this version takes (task 1643).
        pub(crate) fn pin(&self) -> crate::download_pin::PinnedVersion {
            crate::download_pin::PinnedVersion::from_metadata(&json!({"download_version": self.manifest()}))
                .unwrap()
                .unwrap()
        }
    }

    /// The pinned chunk route's real 409 (server `version_download::unavailable`).
    pub(crate) const PINNED_GONE: &str = "download version unavailable; restart download from fresh metadata";

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(crate) enum DiscoveryMode {
        Offer,
        OfferNothing,
        /// axum's unknown route.
        Bare404,
        Json404,
        Bare405,
        Status500,
    }

    /// The answer the next grant request gets (when the script is empty: served).
    #[derive(Debug, Clone)]
    pub(crate) enum GrantScript {
        Serve,
        Json(u16, Value),
        Bare(u16),
        /// A 200 whose body is not the contract's JSON.
        Garbage,
        /// The file gets its replacement version first, then the request is answered as usual
        /// (a pinned request then meets `version_unavailable`).
        Replace,
    }

    /// What one storage GET of one chunk gets (when the script is empty: served).
    #[derive(Debug, Clone)]
    pub(crate) enum StorageScript {
        Status(u16),
        /// 302 to this location.
        Redirect(String),
        /// One byte short (Content-Length says so too).
        Short,
        /// One bit flipped in the tag.
        Tamper,
        HangUp,
        /// The real frame, sent this many bytes at a time with this pause between pieces.
        Trickle(usize, std::time::Duration),
        /// About half the body arrives, then the connection closes (a hang-up part way).
        BreakOff,
    }

    /// What one pinned API chunk GET gets (when the script is empty: served).
    #[derive(Debug, Clone)]
    pub(crate) enum ApiScript {
        Status(u16),
        Tamper,
        HangUp,
        /// The pinned version is gone: the server's real 409.
        Gone,
        /// 429 with this `Retry-After` (seconds).
        RateLimited(u64),
        /// The real frame, sent this many bytes at a time with this pause between pieces.
        Trickle(usize, std::time::Duration),
    }

    pub(crate) struct Knobs {
        pub file_id: String,
        pub version: Version,
        pub replacement: Option<Version>,
        pub discovery: DiscoveryMode,
        pub grants: VecDeque<GrantScript>,
        pub storage: HashMap<u32, VecDeque<StorageScript>>,
        pub api_chunks: HashMap<u32, VecDeque<ApiScript>>,
        /// Grants name this authority instead of the storage mock's (an unlisted host).
        pub link_authority: Option<String>,
        /// Grants say this transport instead of `object-download-v1`.
        pub transport_answer: Option<String>,
        /// The metadata has no `download_version` (a legacy file).
        pub legacy: bool,
    }

    pub(crate) struct Pair {
        pub api: Mock,
        pub storage: Mock,
        pub knobs: Arc<Mutex<Knobs>>,
    }

    fn tamper(frame: &[u8]) -> Vec<u8> {
        let mut f = frame.to_vec();
        let last = f.len() - 1;
        f[last] ^= 0x01;
        f
    }

    fn grant_window(k: &Knobs, storage: &str, body: &[u8]) -> Reply {
        let Ok(request) = serde_json::from_slice::<Value>(body) else {
            return Reply::json(400, json!({"error": "bad request"}));
        };
        let pinned = request["object_version_id"].as_str();
        let start = request["start_index"].as_u64().unwrap_or(0) as u32;
        let count = request["count"].as_u64().unwrap_or(0) as u32;
        if !(1..=32).contains(&count) || (pinned.is_none() && start != 0) {
            return Reply::json(400, json!({"error": "bad window"}));
        }
        if pinned.is_some_and(|v| v != k.version.id) {
            return Reply::json(409, json!({"error": "version_unavailable"}));
        }
        let v = &k.version;
        let n = v.chunk_count();
        let end = (start + count).min(n);
        let authority = k.link_authority.clone().unwrap_or_else(|| storage.to_string());
        let expires = (chrono::Utc::now() + chrono::Duration::seconds(300))
            .format("%Y-%m-%dT%H:%M:%SZ")
            .to_string();
        let grants: Vec<Value> = (start..end)
            .map(|i| {
                json!({
                    "index": i,
                    "method": "GET",
                    "url": format!(
                        "http://{authority}/{BUCKET}/t/{}/{i}.bin?X-Amz-Credential=AKIDSECRET&{SIGNATURE}={SECRET}{i}",
                        v.id
                    ),
                    "length": v.frames[i as usize].len(),
                    "expires_at": expires,
                })
            })
            .collect();
        Reply::json(
            200,
            json!({
                "transport": k.transport_answer.clone().unwrap_or_else(|| "object-download-v1".into()),
                "file_id": k.file_id,
                "object_version_id": v.id,
                "chunk_count": n,
                "chunk_size": v.stride,
                "encrypted_size": v.encrypted_size(),
                "original_size": v.plain.len(),
                "grants": grants,
                "next_index": if end < n { json!(end) } else { Value::Null },
            }),
        )
    }

    impl Pair {
        /// A file of `size` bytes in chunks of `stride` plaintext bytes, offered over the direct
        /// transport.
        pub(crate) fn start(master_key: [u8; 32], file_id: &str, stride: u64, size: usize) -> Self {
            Self::start_with(master_key, file_id, stride, content(size, 7))
        }

        /// [`Self::start`] with this plaintext.
        pub(crate) fn start_with(master_key: [u8; 32], file_id: &str, stride: u64, plain: Vec<u8>) -> Self {
            let key = file_key(master_key, file_id);
            let version = Version::new(&key, "11111111-2222-4333-8444-555555555555", stride, plain);
            let knobs = Arc::new(Mutex::new(Knobs {
                file_id: file_id.to_string(),
                version,
                replacement: None,
                discovery: DiscoveryMode::Offer,
                grants: VecDeque::new(),
                storage: HashMap::new(),
                api_chunks: HashMap::new(),
                link_authority: None,
                transport_answer: None,
                legacy: false,
            }));
            let k = knobs.clone();
            let storage = Mock::start(Arc::new(move |req: &Seen| {
                let mut k = k.lock().unwrap();
                let prefix = format!("/{BUCKET}/t/{}/", k.version.id);
                let Some(index) = req
                    .path()
                    .strip_prefix(&prefix)
                    .and_then(|rest| rest.strip_suffix(".bin"))
                    .and_then(|i| i.parse::<u32>().ok())
                else {
                    return Reply::bare(404);
                };
                if req.method != "GET" || !req.query().contains(SIGNATURE) {
                    return Reply::bare(403);
                }
                let frame = k.version.frames[index as usize].clone();
                match k.storage.get_mut(&index).and_then(VecDeque::pop_front) {
                    None => Reply::bytes(frame),
                    Some(StorageScript::Status(s)) => Reply::bare(s),
                    Some(StorageScript::Redirect(to)) => Reply::redirect(to),
                    Some(StorageScript::Short) => Reply::bytes(frame[..frame.len() - 1].to_vec()),
                    Some(StorageScript::Tamper) => Reply::bytes(tamper(&frame)),
                    Some(StorageScript::HangUp) => Reply::hang_up(),
                    Some(StorageScript::Trickle(piece, pause)) => Reply::bytes(frame).trickling(piece, pause),
                    // The head is under 300 bytes: what is sent past it is part of the body, not all.
                    Some(StorageScript::BreakOff) => {
                        let half = frame.len() / 2;
                        Reply::bytes(frame).stalling(300 + half, std::time::Duration::from_millis(20))
                    }
                }
            }));
            let storage_authority = storage.authority.clone();
            let k = knobs.clone();
            let api = Mock::start(Arc::new(move |req: &Seen| {
                let mut k = k.lock().unwrap();
                let file = format!("/api/v1/files/{}", k.file_id);
                let path = req.path().to_string();
                if req.method == "GET" && path == beebeeb_core::direct_transfer::DISCOVERY_PATH {
                    return match k.discovery {
                        DiscoveryMode::Offer => Reply::json(
                            200,
                            json!({
                                "download": ["object-download-v1"], "upload": [],
                                "storage_hosts": [storage_authority],
                                "storage_buckets": [{"host": storage_authority, "bucket": BUCKET}],
                                "max_window": 32, "url_ttl_seconds": 300,
                            }),
                        ),
                        DiscoveryMode::OfferNothing => Reply::json(
                            200,
                            json!({"download": [], "upload": [], "storage_hosts": [], "storage_buckets": []}),
                        ),
                        DiscoveryMode::Bare404 => Reply::bare(404),
                        DiscoveryMode::Json404 => Reply::json(404, json!({"error": "not found"})),
                        DiscoveryMode::Bare405 => Reply::bare(405),
                        DiscoveryMode::Status500 => Reply::json(500, json!({"error": "internal"})),
                    };
                }
                if req.method == "POST" && path == format!("{file}/download-grants") {
                    if req.header("x-beebeeb-capabilities") != Some("object-download-v1") {
                        return Reply::json(
                            409,
                            json!({"error": "direct_unavailable", "reason": "capability_missing"}),
                        );
                    }
                    match k.grants.pop_front() {
                        None | Some(GrantScript::Serve) => {}
                        Some(GrantScript::Json(status, body)) => return Reply::json(status, body),
                        Some(GrantScript::Bare(status)) => return Reply::bare(status),
                        Some(GrantScript::Garbage) => {
                            return Reply {
                                status: 200,
                                headers: vec![("Content-Type".into(), "text/html".into())],
                                body: b"<html>gateway</html>".to_vec(),
                                hang_up: false,
                                stall: None,
                                trickle: None,
                            };
                        }
                        Some(GrantScript::Replace) => {
                            if let Some(next) = k.replacement.take() {
                                k.version = next;
                            }
                        }
                    }
                    return grant_window(&k, &storage_authority, &req.body);
                }
                if req.method == "GET" && path == file {
                    let v = &k.version;
                    return Reply::json(
                        200,
                        json!({
                            "id": k.file_id, "size_bytes": v.plain.len(), "chunk_count": v.chunk_count(),
                            "chunk_size_bytes": v.stride,
                            "name_encrypted": beebeeb_core::encrypt::encrypt_name(&beebeeb_core::kdf::MasterKey::from_bytes(master_key), &k.file_id, "direct.bin", None).unwrap(),
                            "download_version": if k.legacy { Value::Null } else { v.manifest() },
                        }),
                    );
                }
                if let Some(index) = path
                    .strip_prefix(&format!("{file}/chunks/"))
                    .and_then(|i| i.parse::<u32>().ok())
                {
                    let pinned = req
                        .query()
                        .split('&')
                        .find_map(|kv| kv.strip_prefix("object_version_id="));
                    if pinned.is_some_and(|v| v != k.version.id) {
                        return Reply::json(409, json!({"error": PINNED_GONE}));
                    }
                    let Some(frame) = k.version.frames.get(index as usize).cloned() else {
                        return Reply::json(404, json!({"error": "not found"}));
                    };
                    return match k.api_chunks.get_mut(&index).and_then(VecDeque::pop_front) {
                        None => Reply::bytes(frame),
                        Some(ApiScript::Status(s)) => Reply::json(s, json!({"error": "scripted"})),
                        Some(ApiScript::Tamper) => Reply::bytes(tamper(&frame)),
                        Some(ApiScript::Trickle(piece, pause)) => Reply::bytes(frame).trickling(piece, pause),
                        Some(ApiScript::HangUp) => Reply::hang_up(),
                        Some(ApiScript::Gone) => Reply::json(409, json!({"error": PINNED_GONE})),
                        Some(ApiScript::RateLimited(secs)) => {
                            Reply::json(429, json!({"error": "rate_limited"})).with_header("Retry-After", &secs.to_string())
                        }
                    };
                }
                Reply::json(404, json!({"error": "not found"}))
            }));
            Pair { api, storage, knobs }
        }

        pub(crate) fn knobs(&self) -> std::sync::MutexGuard<'_, Knobs> {
            self.knobs.lock().unwrap()
        }

        pub(crate) fn plain(&self) -> Vec<u8> {
            self.knobs().version.plain.clone()
        }

        pub(crate) fn discovery_requests(&self) -> usize {
            self.api
                .count(|s| s.path() == beebeeb_core::direct_transfer::DISCOVERY_PATH)
        }

        pub(crate) fn grant_requests(&self) -> Vec<Seen> {
            self.api
                .seen()
                .into_iter()
                .filter(|s| s.path().ends_with("/download-grants"))
                .collect()
        }

        pub(crate) fn api_chunk_requests(&self) -> Vec<Seen> {
            self.api
                .seen()
                .into_iter()
                .filter(|s| s.path().contains("/chunks/"))
                .collect()
        }

        pub(crate) fn storage_requests(&self) -> usize {
            self.storage.seen().len()
        }
    }

    pub(crate) fn file_key(master_key: [u8; 32], file_id: &str) -> beebeeb_core::kdf::FileKey {
        let mk = beebeeb_core::kdf::MasterKey::from_bytes(master_key);
        beebeeb_core::kdf::derive_file_key(&mk, file_id.as_bytes())
    }

    /// Everything `body` logs through `tracing`, at every level.
    pub(crate) async fn capture_logs<T>(body: impl std::future::Future<Output = T>) -> (T, String) {
        #[derive(Clone)]
        struct Capture(Arc<Mutex<Vec<u8>>>);
        impl Write for Capture {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let buffer: Arc<Mutex<Vec<u8>>> = Arc::default();
        let writer = Capture(buffer.clone());
        // tracing caches each callsite's interest for the whole process; a second live dispatcher
        // keeps registration on the all-dispatchers path, so a callsite another test thread hit
        // first is not cached as "never" (the pattern of engine_bridge's log tests).
        let _second_dispatcher = tracing::Dispatch::new(tracing_subscriber::registry());
        let subscriber = tracing_subscriber::fmt()
            .with_writer(move || writer.clone())
            .with_ansi(false)
            .with_max_level(tracing::Level::TRACE)
            .finish();
        let guard = tracing::subscriber::set_default(subscriber);
        let out = body.await;
        drop(guard);
        let text = String::from_utf8_lossy(&buffer.lock().unwrap()).into_owned();
        (out, text)
    }
}

#[cfg(test)]
mod tests;

mod transport;
pub(crate) use transport::{Transport, FileTransport, Storage};

mod protocol;
use protocol::*;
pub(crate) use protocol::{Request, Progress, Outcome, DirectError};

mod download;
pub(crate) use download::download;
#[cfg(test)]
pub(crate) use download::paced_until_ms;
