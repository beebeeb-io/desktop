//! Direct S3 download driver (task 2173).
//!
//! When the server offers `object-download-v1`, a whole-file hydrate fetches the file's encrypted
//! chunks straight from object storage through short-lived presigned links, and decrypts them
//! exactly as the API path does (`decrypt_downloaded_chunk`). The API only authorizes and signs.
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

/// Which transport a download may use. `Auto` (the default) uses direct storage transfers when the
/// server offers them and the handshake succeeds; `Api` forces today's API transport, for support
/// and the compatibility matrix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Transport {
    Auto,
    Api,
}

impl Transport {
    /// The environment variable that forces a transport (`api` or `auto`). It wins over
    /// `desktop.toml`'s `transport` key.
    #[cfg_attr(test, allow(dead_code))]
    pub(crate) const ENV: &'static str = "BB_TRANSPORT";

    fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "api" => Some(Transport::Api),
            "auto" => Some(Transport::Auto),
            _ => None,
        }
    }

    /// The environment's value wins over the config file's; an unrecognised value is ignored.
    pub(crate) fn resolve(env: Option<&str>, config: Option<&str>) -> Self {
        env.and_then(Self::parse)
            .or_else(|| config.and_then(Self::parse))
            .unwrap_or(Transport::Auto)
    }

    /// The transport this process selects, given `desktop.toml`'s `transport` value. (A test bridge
    /// takes its test's transport instead, so a test never reads the developer's environment.)
    #[cfg_attr(test, allow(dead_code))]
    pub(crate) fn selected(config: Option<&str>) -> Self {
        Self::resolve(std::env::var(Self::ENV).ok().as_deref(), config)
    }
}

/// Which transport a whole-file download ended up using.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FileTransport {
    /// The API's per-chunk route, as before task 2173.
    Api,
    /// Every chunk straight from object storage.
    Direct,
    /// Direct, with some chunks through the pinned per-chunk API route. Still one version.
    Mixed,
}

impl FileTransport {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            FileTransport::Api => "api",
            FileTransport::Direct => "direct",
            FileTransport::Mixed => "mixed",
        }
    }
}

/// What the caller asked for: one whole file.
pub(crate) struct Request<'a> {
    pub file_id: &'a str,
    pub file_key: &'a beebeeb_core::kdf::FileKey,
    /// The metadata's chunk count; the grant manifest must agree.
    pub chunk_count: u32,
    /// The user's download ceiling in KB/s (`0`: none), as on the API path.
    pub download_kbps_limit: u64,
    /// Sizes the plaintext buffer only.
    pub approx_size: usize,
}

/// Where the driver reports progress: the same two numbers the API path reports.
pub(crate) trait Progress {
    /// A chunk body arrived (storage or API): its wire length.
    fn received(&self, wire_bytes: u64);
    /// A chunk was decrypted and appended, strictly in order: the plaintext length so far. Called
    /// with 0 once, when the direct attempt starts (the API path's first report).
    fn decrypted(&self, plaintext_bytes: u64);
}

/// The result of an attempt to download over the direct transport.
pub(crate) enum Outcome {
    /// The file is complete and every chunk authenticated.
    Done {
        plaintext: Zeroizing<Vec<u8>>,
        transport: FileTransport,
    },
    /// Direct transport was not used, or was abandoned before the file completed: download this
    /// file through the API path. Nothing of the attempt is kept.
    UseApi,
}

/// A failure the API path would not fix. Nothing is returned on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DirectError {
    /// The pinned version went away mid-transfer (`version_unavailable`): restart from fresh
    /// metadata, never mix versions.
    VersionChanged,
    /// An authorization answer (401, 403, a JSON 404) on a grant or on the pinned route. Never
    /// routed around.
    Refused(u16),
    /// A chunk did not authenticate from storage, nor from the API.
    Corrupt,
}

impl std::fmt::Display for DirectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DirectError::VersionChanged => f.write_str("the file changed while it was downloading"),
            DirectError::Refused(status) => write!(f, "download refused by the server ({status})"),
            DirectError::Corrupt => f.write_str(
                "a chunk did not authenticate from storage or from the API: the file is damaged or was tampered with",
            ),
        }
    }
}

impl std::error::Error for DirectError {}

/// How a chunk is fetched from its presigned link.
#[derive(Clone, Copy)]
pub(crate) enum Storage<'a> {
    Client(&'a reqwest::Client),
    /// Tests only: every storage fetch fails with this status, without a request, so a test
    /// against a real server can force the renewals and the pinned-route fallback.
    #[cfg(test)]
    Fail(u16),
}

impl Storage<'_> {
    async fn fetch(self, grant: dt::ChunkGrant) -> Result<Vec<u8>, u16> {
        match self {
            Storage::Client(client) => crate::api_client::fetch_from_storage(client, &grant).await,
            #[cfg(test)]
            Storage::Fail(status) => Err(status),
        }
    }
}

enum Event {
    Window(GrantReply),
    Storage {
        index: u32,
        /// `Err(status)`: the HTTP status when there was one (0: none).
        result: Result<Vec<u8>, u16>,
    },
    Api {
        index: u32,
        result: Result<Vec<u8>, ChunkApiError>,
    },
}

type Task<'a> = Pin<Box<dyn Future<Output = Event> + Send + 'a>>;

/// When the user's download ceiling lets the next request start: `wire_bytes` received since
/// `start_ms` at `kbps` KB/s. The API path sleeps after each chunk; with up to four requests in
/// flight the direct path holds back new ones instead, so the average rate is the same.
pub(crate) fn paced_until_ms(start_ms: i64, wire_bytes: u64, kbps: u64) -> i64 {
    if kbps == 0 {
        return i64::MIN;
    }
    let ms = u128::from(wire_bytes) * 1000 / (u128::from(kbps) * 1024);
    start_ms.saturating_add(i64::try_from(ms).unwrap_or(i64::MAX))
}

/// Download `request.file_id` over the direct transport when the server offers it.
///
/// `Ok(UseApi)`: not used or abandoned; the caller downloads the file through the API.
/// `Err`: a failure the API path would not fix.
pub(crate) async fn download(
    api: &ApiClient,
    request: Request<'_>,
    transport: Transport,
    storage_override: Option<Storage<'_>>,
    progress: &(dyn Progress + Sync),
) -> Result<Outcome, DirectError> {
    if transport == Transport::Api || request.chunk_count == 0 {
        return Ok(Outcome::UseApi);
    }
    let Some(discovery) = api.direct_download_offer().await else {
        return Ok(Outcome::UseApi);
    };
    let storage = match storage_override {
        Some(storage) => storage,
        None => match api.storage_client() {
            Some(client) => Storage::Client(client),
            None => return Ok(Outcome::UseApi),
        },
    };
    let file_id = request.file_id;
    progress.decrypted(0);
    let config = dt::PlannerConfig::from_discovery(&discovery, rand::random());
    let mut planner = dt::DownloadPlanner::new(config, discovery, api.api_base(), file_id);
    let mut tasks: FuturesUnordered<Task<'_>> = FuturesUnordered::new();
    let mut ready: BTreeMap<u32, Vec<u8>> = BTreeMap::new();
    let mut plaintext: Zeroizing<Vec<u8>> = Zeroizing::new(Vec::with_capacity(request.approx_size));
    let mut geometry_checked = false;
    // Chunks that came through the pinned API route instead of storage.
    let mut api_chunks: u32 = 0;
    let started_ms = crate::api_client::now_ms();
    let mut wire_bytes: u64 = 0;

    loop {
        // Everything the planner has for us right now, unless the user's ceiling holds new
        // requests back (the ones in flight still finish).
        let paced = paced_until_ms(started_ms, wire_bytes, request.download_kbps_limit);
        let wake: Option<i64> = if crate::api_client::now_ms() < paced {
            Some(paced)
        } else {
            loop {
                match planner.poll(crate::api_client::now_ms()) {
                    dt::Poll::RequestWindow(window) => {
                        tracing::debug!(
                            start = window.start_index,
                            count = window.count,
                            "direct download: window request"
                        );
                        tasks.push(Box::pin(
                            async move { Event::Window(api.download_grants(file_id, &window).await) },
                        ));
                    }
                    dt::Poll::Fetch { index, grant } => {
                        tasks.push(Box::pin(async move {
                            Event::Storage {
                                index,
                                result: storage.fetch(grant).await,
                            }
                        }));
                    }
                    dt::Poll::FetchViaApi {
                        index,
                        object_version_id,
                    } => {
                        let Some(length) = planner.geometry().and_then(|g| g.chunk_length(index)) else {
                            return Ok(Outcome::UseApi);
                        };
                        tracing::debug!(chunk = index, "direct download: chunk through the pinned API route");
                        tasks.push(Box::pin(async move {
                            Event::Api {
                                index,
                                result: api
                                    .download_chunk_pinned(file_id, index, &object_version_id, length)
                                    .await,
                            }
                        }));
                    }
                    dt::Poll::Wait { until_ms } => break until_ms,
                    dt::Poll::Done => {
                        let transport = if api_chunks > 0 {
                            FileTransport::Mixed
                        } else {
                            FileTransport::Direct
                        };
                        return Ok(Outcome::Done { plaintext, transport });
                    }
                    dt::Poll::Stop(verdict) => {
                        return match verdict {
                            dt::Verdict::ApiForFile => {
                                tracing::debug!("direct download: falling back to the API for this file");
                                Ok(Outcome::UseApi)
                            }
                            dt::Verdict::Restart => Err(DirectError::VersionChanged),
                            dt::Verdict::Terminal(t) => Err(DirectError::Refused(t.status)),
                            dt::Verdict::Corrupt => Err(DirectError::Corrupt),
                        };
                    }
                }
            }
        };

        // Wait for something to finish, or for the next backoff (or the ceiling) to end.
        let event = if tasks.is_empty() {
            match wake {
                Some(at) => {
                    tokio::time::sleep(until(at)).await;
                    None
                }
                None => {
                    // Nothing outstanding and nothing scheduled: the planner is stuck. Never hang a
                    // download on that; the API path is always correct.
                    tracing::debug!("direct download: planner stalled; falling back to the API");
                    return Ok(Outcome::UseApi);
                }
            }
        } else if let Some(at) = wake {
            tokio::select! {
                ev = tasks.next() => ev,
                _ = tokio::time::sleep(until(at)) => None,
            }
        } else {
            tasks.next().await
        };

        let now = crate::api_client::now_ms();
        match event {
            None => {}
            Some(Event::Window(GrantReply::Window(response))) => match planner.window_answered(response, now) {
                Ok(()) => {
                    if !geometry_checked {
                        geometry_checked = true;
                        if planner.geometry().map(|g| g.chunk_count) != Some(request.chunk_count) {
                            // The manifest and the file's metadata disagree: the API path, which
                            // has its own checks, serves this file.
                            tracing::debug!("direct download: manifest chunk count differs from the metadata");
                            return Ok(Outcome::UseApi);
                        }
                    }
                }
                Err(violation) => tracing::debug!(%violation, "direct download: window refused"),
            },
            Some(Event::Window(GrantReply::Malformed)) => {
                tracing::debug!("direct download: window answer could not be decoded");
                planner.window_malformed();
            }
            Some(Event::Window(GrantReply::Failed(outcome))) => {
                tracing::debug!(?outcome, "direct download: window failed");
                planner.window_failed(outcome, now);
            }
            Some(Event::Storage { index, result }) => match result {
                Ok(bytes) => {
                    wire_bytes += bytes.len() as u64;
                    progress.received(bytes.len() as u64);
                    if let Err(e) = planner.fetch_ok(index) {
                        tracing::debug!(error = %e, "direct download: planner error");
                        return Ok(Outcome::UseApi);
                    }
                    ready.insert(index, bytes);
                }
                Err(status) => {
                    tracing::debug!(chunk = index, status, "direct download: chunk from storage failed");
                    if let Err(e) = planner.fetch_failed(index, now) {
                        tracing::debug!(error = %e, "direct download: planner error");
                        return Ok(Outcome::UseApi);
                    }
                }
            },
            Some(Event::Api { index, result }) => match result {
                Ok(bytes) => {
                    wire_bytes += bytes.len() as u64;
                    progress.received(bytes.len() as u64);
                    if let Err(e) = planner.fetch_ok(index) {
                        tracing::debug!(error = %e, "direct download: planner error");
                        return Ok(Outcome::UseApi);
                    }
                    api_chunks += 1;
                    ready.insert(index, bytes);
                }
                Err(ChunkApiError::Restart) => return Err(DirectError::VersionChanged),
                Err(ChunkApiError::Terminal(status)) => return Err(DirectError::Refused(status)),
                // The pinned route failed: nothing per chunk is left to try. The planner ends the
                // file and the API path takes it from the start.
                Err(ChunkApiError::Failed(message)) => {
                    tracing::debug!(chunk = index, %message, "direct download: pinned API route failed");
                    if let Err(e) = planner.api_fetch_failed(index) {
                        tracing::debug!(error = %e, "direct download: planner error");
                        return Ok(Outcome::UseApi);
                    }
                }
            },
        }

        // Decrypt and append what is ready, strictly in order.
        loop {
            let next = planner.consumed_count();
            let Some(frame) = ready.remove(&next) else { break };
            match crate::engine_bridge::decrypt_downloaded_chunk(request.file_key, &frame) {
                Ok(mut chunk) => {
                    plaintext.extend_from_slice(&chunk);
                    chunk.zeroize();
                    progress.decrypted(plaintext.len() as u64);
                    if let Err(e) = planner.consumed(next) {
                        tracing::debug!(error = %e, "direct download: planner error");
                        return Ok(Outcome::UseApi);
                    }
                }
                Err(_) => {
                    tracing::debug!(chunk = next, "direct download: chunk did not authenticate");
                    if let Err(e) = planner.chunk_unauthentic(next) {
                        tracing::debug!(error = %e, "direct download: planner error");
                        return Ok(Outcome::UseApi);
                    }
                    break;
                }
            }
        }
    }
}

fn until(at_ms: i64) -> Duration {
    Duration::from_millis(u64::try_from(at_ms.saturating_sub(crate::api_client::now_ms())).unwrap_or(0))
}

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
        /// Content-Length to state, when it must differ from the body (never, here: a mismatch is
        /// made by sending a different body).
        hang_up: bool,
    }

    impl Reply {
        pub(crate) fn json(status: u16, value: Value) -> Self {
            Reply {
                status,
                headers: vec![("Content-Type".into(), "application/json".into())],
                body: value.to_string().into_bytes(),
                hang_up: false,
            }
        }
        pub(crate) fn bytes(body: Vec<u8>) -> Self {
            Reply {
                status: 200,
                headers: vec![("Content-Type".into(), "application/octet-stream".into())],
                body,
                hang_up: false,
            }
        }
        /// An empty body with no content type: axum's unknown-route answer.
        pub(crate) fn bare(status: u16) -> Self {
            Reply {
                status,
                headers: Vec::new(),
                body: Vec::new(),
                hang_up: false,
            }
        }
        pub(crate) fn redirect(location: String) -> Self {
            Reply {
                status: 302,
                headers: vec![("Location".into(), location)],
                body: Vec::new(),
                hang_up: false,
            }
        }
        /// Close the connection without an answer.
        pub(crate) fn hang_up() -> Self {
            Reply {
                status: 0,
                headers: Vec::new(),
                body: Vec::new(),
                hang_up: true,
            }
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
    }

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
    }

    /// What one pinned API chunk GET gets (when the script is empty: served).
    #[derive(Debug, Clone)]
    pub(crate) enum ApiScript {
        Status(u16),
        Tamper,
        HangUp,
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
            let key = file_key(master_key, file_id);
            let version = Version::new(&key, "11111111-2222-4333-8444-555555555555", stride, content(size, 7));
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
                        return Reply::json(409, json!({"error": "version_unavailable"}));
                    }
                    let Some(frame) = k.version.frames.get(index as usize).cloned() else {
                        return Reply::json(404, json!({"error": "not found"}));
                    };
                    return match k.api_chunks.get_mut(&index).and_then(VecDeque::pop_front) {
                        None => Reply::bytes(frame),
                        Some(ApiScript::Status(s)) => Reply::json(s, json!({"error": "scripted"})),
                        Some(ApiScript::Tamper) => Reply::bytes(tamper(&frame)),
                        Some(ApiScript::HangUp) => Reply::hang_up(),
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
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicU64, Ordering};

    use serde_json::json;

    use super::test_support::*;
    use super::*;

    const FILE_ID: &str = "0b9f3c2e-7d41-4a8e-9c55-2173d0000001";
    const KEY: [u8; 32] = [21; 32];
    /// 11 chunks of 1 KiB (the last one 77 bytes): two grant windows of 8.
    const STRIDE: u64 = 1024;
    const SIZE: usize = 10 * 1024 + 77;

    #[derive(Default)]
    struct Recorder {
        wire: AtomicU64,
        decrypted: Mutex<Vec<u64>>,
    }

    impl Progress for Recorder {
        fn received(&self, wire_bytes: u64) {
            self.wire.fetch_add(wire_bytes, Ordering::Relaxed);
        }
        fn decrypted(&self, plaintext_bytes: u64) {
            self.decrypted.lock().unwrap().push(plaintext_bytes);
        }
    }

    fn new_pair() -> Pair {
        Pair::start(KEY, FILE_ID, STRIDE, SIZE)
    }

    fn client(pair: &Pair) -> ApiClient {
        ApiClient::new(pair.api.base.clone(), "token".into(), KEY)
    }

    async fn run_with(
        api: &ApiClient,
        pair: &Pair,
        transport: Transport,
        kbps: u64,
        progress: &Recorder,
    ) -> Result<Outcome, DirectError> {
        let key = file_key(KEY, FILE_ID);
        let chunk_count = pair.knobs().version.chunk_count();
        download(
            api,
            Request {
                file_id: FILE_ID,
                file_key: &key,
                chunk_count,
                download_kbps_limit: kbps,
                approx_size: SIZE,
            },
            transport,
            None,
            progress,
        )
        .await
    }

    async fn run(pair: &Pair) -> Result<Outcome, DirectError> {
        run_with(&client(pair), pair, Transport::Auto, 0, &Recorder::default()).await
    }

    fn done(outcome: Result<Outcome, DirectError>) -> (Vec<u8>, FileTransport) {
        match outcome {
            Ok(Outcome::Done { plaintext, transport }) => (plaintext.to_vec(), transport),
            Ok(Outcome::UseApi) => panic!("expected a direct download, got UseApi"),
            Err(e) => panic!("expected a direct download, got {e:?}"),
        }
    }

    fn uses_api(outcome: Result<Outcome, DirectError>) {
        match outcome {
            Ok(Outcome::UseApi) => {}
            Ok(Outcome::Done { transport, .. }) => panic!("expected UseApi, got a {transport:?} download"),
            Err(e) => panic!("expected UseApi, got {e:?}"),
        }
    }

    #[tokio::test]
    async fn direct_is_byte_identical_and_never_touches_the_api_chunk_route() {
        let pair = new_pair();
        let progress = Recorder::default();
        let (bytes, transport) = done(run_with(&client(&pair), &pair, Transport::Auto, 0, &progress).await);
        assert_eq!(bytes, pair.plain(), "byte-identical");
        assert_eq!(transport, FileTransport::Direct);
        assert_eq!(pair.discovery_requests(), 1);
        assert_eq!(pair.grant_requests().len(), 2, "11 chunks in windows of 8");
        assert_eq!(pair.storage_requests(), 11, "one storage GET per chunk");
        assert!(pair.api_chunk_requests().is_empty(), "the API chunk route is never touched");
        // The same numbers the API path reports: every wire byte, then the plaintext so far.
        let encrypted = pair.knobs().version.encrypted_size();
        assert_eq!(progress.wire.load(Ordering::Relaxed), encrypted);
        let decrypted = progress.decrypted.lock().unwrap().clone();
        assert_eq!(decrypted.first(), Some(&0), "the start is reported");
        assert_eq!(decrypted.last(), Some(&(SIZE as u64)));
        assert_eq!(decrypted.len(), 12, "the start, then once per chunk");
        // The first window pins nothing; every later one pins the version the first returned.
        let grants = pair.grant_requests();
        let first: serde_json::Value = serde_json::from_slice(&grants[0].body).unwrap();
        assert_eq!(first["object_version_id"], serde_json::Value::Null);
        assert_eq!(first["start_index"], 0);
        let second: serde_json::Value = serde_json::from_slice(&grants[1].body).unwrap();
        assert_eq!(second["object_version_id"], pair.knobs().version.id.as_str());
        assert_eq!(second["start_index"], 8);
    }

    #[tokio::test]
    async fn the_capability_header_rides_on_grant_requests_only_and_storage_sees_no_account_headers() {
        let pair = new_pair();
        // One chunk via the pinned route too, so that request is checked as well.
        pair.knobs().storage.insert(3, vec![StorageScript::Status(503); 4].into());
        let (bytes, transport) = done(run(&pair).await);
        assert_eq!((bytes, transport), (pair.plain(), FileTransport::Mixed));
        let api = pair.api.seen();
        assert!(api.len() >= 4, "discovery, two windows (+ renewals), one pinned chunk: {}", api.len());
        for request in &api {
            let capabilities = request.header("x-beebeeb-capabilities");
            if request.path().ends_with("/download-grants") {
                assert_eq!(capabilities, Some("object-download-v1"), "{}", request.path());
            } else {
                assert_eq!(capabilities, None, "only grant requests declare the token: {}", request.path());
            }
            assert_eq!(request.header("authorization"), Some("Bearer token"));
        }
        let storage = pair.storage.seen();
        assert_eq!(storage.len(), 10 + 4, "10 chunks once, chunk 3 once plus 3 renewals");
        for request in &storage {
            for (name, _) in &request.headers {
                let name = name.to_ascii_lowercase();
                assert!(
                    !(name == "authorization" || name == "cookie" || name.starts_with("x-beebeeb")),
                    "storage saw the header {name}"
                );
            }
        }
    }

    #[tokio::test]
    async fn discovery_is_asked_once_per_signed_in_client_and_a_new_client_asks_again() {
        let pair = new_pair();
        let api = client(&pair);
        for round in 0..3 {
            let (bytes, _) = done(run_with(&api, &pair, Transport::Auto, 0, &Recorder::default()).await);
            assert_eq!(bytes, pair.plain(), "round {round}");
        }
        assert_eq!(pair.discovery_requests(), 1, "one request, then the cache");
        // A sign-out or an account switch drops the client, and with it everything it learned.
        drop(api);
        done(run(&pair).await);
        assert_eq!(pair.discovery_requests(), 2, "a new session asks again");
    }

    #[tokio::test]
    async fn servers_without_direct_transport_are_served_over_the_api() {
        // (mode, discovery requests over three downloads with one client)
        let cases = [
            (DiscoveryMode::OfferNothing, 1),
            (DiscoveryMode::Json404, 1),
            (DiscoveryMode::Bare404, 2),
            (DiscoveryMode::Bare405, 2),
            (DiscoveryMode::Status500, 2),
        ];
        for (mode, asked) in cases {
            let pair = new_pair();
            pair.knobs().discovery = mode;
            let api = client(&pair);
            for _ in 0..3 {
                uses_api(run_with(&api, &pair, Transport::Auto, 0, &Recorder::default()).await);
            }
            assert_eq!(pair.discovery_requests(), asked, "{mode:?}");
            assert!(pair.grant_requests().is_empty(), "{mode:?}: no grant without an offer");
            assert_eq!(pair.storage_requests(), 0, "{mode:?}");
        }
    }

    #[tokio::test]
    async fn forcing_the_api_transport_asks_nothing() {
        let pair = new_pair();
        uses_api(run_with(&client(&pair), &pair, Transport::Api, 0, &Recorder::default()).await);
        assert!(pair.api.seen().is_empty(), "no discovery, no grant");
        assert_eq!(pair.storage_requests(), 0);
    }

    #[test]
    fn the_environment_beats_the_config_file_and_anything_else_means_auto() {
        use Transport::{Api, Auto};
        let cases = [
            (None, None, Auto),
            (None, Some("api"), Api),
            (Some("api"), None, Api),
            (Some(" API "), None, Api),
            (Some("auto"), Some("api"), Auto),
            (Some("api"), Some("auto"), Api),
            (Some("direct"), Some("api"), Api),
            (Some("nonsense"), None, Auto),
            (None, Some("nonsense"), Auto),
        ];
        for (env, config, want) in cases {
            assert_eq!(Transport::resolve(env, config), want, "env {env:?} config {config:?}");
        }
    }

    #[tokio::test]
    async fn a_refusal_to_serve_this_file_directly_falls_back_to_the_api() {
        for reason in ["disabled", "speed_cap", "pool_ineligible", "not_eligible"] {
            let pair = new_pair();
            pair.knobs().grants.push_back(GrantScript::Json(
                409,
                json!({"error": "direct_unavailable", "reason": reason}),
            ));
            uses_api(run(&pair).await);
            assert_eq!(pair.storage_requests(), 0, "{reason}");
            assert!(pair.api_chunk_requests().is_empty(), "{reason}: the caller's API path takes it");
        }
        // The server answers with another transport than the one asked for: not direct.
        let pair = new_pair();
        pair.knobs().transport_answer = Some("object-download-v2".into());
        uses_api(run(&pair).await);
        assert_eq!(pair.storage_requests(), 0);
        // A gateway page instead of the contract's JSON.
        let pair = new_pair();
        pair.knobs().grants.push_back(GrantScript::Garbage);
        uses_api(run(&pair).await);
        // A node without the route (mid-rollout).
        let pair = new_pair();
        pair.knobs().grants.push_back(GrantScript::Bare(404));
        uses_api(run(&pair).await);
    }

    #[tokio::test]
    async fn the_manifest_must_agree_with_the_metadata() {
        let pair = new_pair();
        let key = file_key(KEY, FILE_ID);
        let outcome = download(
            &client(&pair),
            Request {
                file_id: FILE_ID,
                file_key: &key,
                chunk_count: 12,
                download_kbps_limit: 0,
                approx_size: SIZE,
            },
            Transport::Auto,
            None,
            &Recorder::default(),
        )
        .await;
        uses_api(outcome);
        assert_eq!(pair.storage_requests(), 0, "nothing fetched for a file the metadata describes otherwise");
    }

    #[tokio::test]
    async fn an_authorization_refusal_is_terminal_and_never_falls_back() {
        let cases: [(GrantScript, u16); 3] = [
            (GrantScript::Json(401, json!({"error": "unauthorized"})), 401),
            (GrantScript::Json(403, json!({"error": "forbidden"})), 403),
            (GrantScript::Json(404, json!({"error": "not found"})), 404),
        ];
        for (script, status) in cases {
            let pair = new_pair();
            pair.knobs().grants.push_back(script);
            match run(&pair).await {
                Err(DirectError::Refused(got)) => assert_eq!(got, status),
                Ok(Outcome::UseApi) => panic!("{status}: fell back to the API"),
                Ok(Outcome::Done { .. }) => panic!("{status}: downloaded"),
                Err(e) => panic!("{status}: {e:?}"),
            }
            assert_eq!(pair.storage_requests(), 0);
            assert!(pair.api_chunk_requests().is_empty());
        }
        // The pinned route refuses too (a revoked invite between windows): terminal as well.
        let pair = new_pair();
        pair.knobs().storage.insert(0, vec![StorageScript::Status(503); 4].into());
        pair.knobs().api_chunks.insert(0, vec![ApiScript::Status(403)].into());
        assert_eq!(run(&pair).await.err(), Some(DirectError::Refused(403)));
    }

    #[tokio::test]
    async fn a_version_that_goes_away_mid_transfer_returns_nothing() {
        let pair = new_pair();
        {
            let mut k = pair.knobs();
            let replacement = Version::new(&file_key(KEY, FILE_ID), "99999999-2222-4333-8444-555555555555", STRIDE, content(SIZE, 8));
            k.replacement = Some(replacement);
            // The first window is served; the second finds the file replaced.
            k.grants.push_back(GrantScript::Serve);
            k.grants.push_back(GrantScript::Replace);
        }
        assert_eq!(run(&pair).await.err(), Some(DirectError::VersionChanged));
        assert!(pair.api_chunk_requests().is_empty(), "never the other version through the API");
    }

    #[tokio::test]
    async fn an_expired_link_is_renewed_and_the_file_stays_direct() {
        let pair = new_pair();
        pair.knobs().storage.insert(1, vec![StorageScript::Status(403)].into());
        let (bytes, transport) = done(run(&pair).await);
        assert_eq!((bytes, transport), (pair.plain(), FileTransport::Direct));
        let renewals: Vec<serde_json::Value> = pair
            .grant_requests()
            .iter()
            .map(|r| serde_json::from_slice(&r.body).unwrap())
            .filter(|b: &serde_json::Value| b["renewal"] == true)
            .collect();
        assert_eq!(renewals.len(), 1, "one renewal");
        assert_eq!((renewals[0]["start_index"].clone(), renewals[0]["count"].clone()), (json!(1), json!(1)));
        assert_eq!(pair.storage_requests(), 12);
        assert!(pair.api_chunk_requests().is_empty());
    }

    #[tokio::test]
    async fn a_chunk_storage_never_serves_goes_through_the_pinned_api_route() {
        let pair = new_pair();
        pair.knobs().storage.insert(2, vec![StorageScript::Status(503); 10].into());
        let (bytes, transport) = done(run(&pair).await);
        assert_eq!(bytes, pair.plain(), "byte-identical");
        assert_eq!(transport, FileTransport::Mixed);
        let via_api = pair.api_chunk_requests();
        assert_eq!(via_api.len(), 1, "only the chunk storage would not give");
        assert_eq!(via_api[0].path(), format!("/api/v1/files/{FILE_ID}/chunks/2"));
        let pinned = pair.knobs().version.id.clone();
        assert_eq!(via_api[0].query(), format!("object_version_id={pinned}"), "the pinned version");
        let to_storage_for_2 = pair.storage.count(|s| s.path().ends_with("/2.bin"));
        assert_eq!(to_storage_for_2, 4, "one try and three renewals, no more");
    }

    #[tokio::test]
    async fn a_window_refused_after_the_first_moves_the_rest_to_the_pinned_route() {
        let pair = new_pair();
        {
            let mut k = pair.knobs();
            k.grants.push_back(GrantScript::Serve);
            k.grants
                .push_back(GrantScript::Json(409, json!({"error": "direct_unavailable", "reason": "disabled"})));
        }
        let (bytes, transport) = done(run(&pair).await);
        assert_eq!((bytes, transport), (pair.plain(), FileTransport::Mixed));
        assert_eq!(pair.storage_requests(), 8, "the first window came from storage");
        assert_eq!(pair.api_chunk_requests().len(), 3, "the rest through the pinned route");
        assert!(pair.api_chunk_requests().iter().all(|r| r.query().starts_with("object_version_id=")));
    }

    #[tokio::test]
    async fn a_storage_redirect_is_never_followed() {
        let pair = new_pair();
        let elsewhere = Mock::start(std::sync::Arc::new(|_: &test_support::Seen| Reply::bytes(vec![0; 1052])));
        pair.knobs()
            .storage
            .insert(0, vec![StorageScript::Redirect(format!("{}/{BUCKET}/t/x/0.bin", elsewhere.base))].into());
        let (bytes, transport) = done(run(&pair).await);
        assert_eq!((bytes, transport), (pair.plain(), FileTransport::Direct));
        assert!(elsewhere.seen().is_empty(), "the redirect target was never asked");
    }

    #[tokio::test]
    async fn a_body_shorter_than_the_grant_is_a_failed_fetch() {
        let pair = new_pair();
        pair.knobs().storage.insert(4, vec![StorageScript::Short].into());
        let (bytes, transport) = done(run(&pair).await);
        assert_eq!((bytes, transport), (pair.plain(), FileTransport::Direct));
        assert_eq!(pair.storage.count(|s| s.path().ends_with("/4.bin")), 2, "renewed once");
    }

    #[tokio::test]
    async fn a_tampered_storage_chunk_is_refetched_through_the_api_and_a_double_tamper_is_fatal() {
        let pair = new_pair();
        pair.knobs().storage.insert(5, vec![StorageScript::Tamper].into());
        let (bytes, transport) = done(run(&pair).await);
        assert_eq!((bytes, transport), (pair.plain(), FileTransport::Mixed));
        assert_eq!(pair.api_chunk_requests().len(), 1, "one fresh copy through the API");

        let pair = new_pair();
        pair.knobs().storage.insert(5, vec![StorageScript::Tamper].into());
        pair.knobs().api_chunks.insert(5, vec![ApiScript::Tamper].into());
        assert_eq!(run(&pair).await.err(), Some(DirectError::Corrupt), "never partial plaintext");
    }

    #[tokio::test]
    async fn a_link_to_a_host_discovery_did_not_list_is_never_fetched() {
        let pair = new_pair();
        let unlisted = Mock::start(std::sync::Arc::new(|_: &test_support::Seen| Reply::bytes(vec![0; 1052])));
        pair.knobs().link_authority = Some(unlisted.authority.clone());
        uses_api(run(&pair).await);
        assert!(unlisted.seen().is_empty(), "the unlisted host was never asked");
        assert_eq!(pair.storage_requests(), 0);
    }

    #[tokio::test]
    async fn no_url_or_signature_reaches_a_log_line_or_an_error() {
        // Every storage failure there is, then the pinned route, then a terminal refusal.
        let (results, logs) = capture_logs(async {
            let pair = new_pair();
            {
                let mut k = pair.knobs();
                k.storage.insert(0, vec![StorageScript::HangUp, StorageScript::Status(403)].into());
                k.storage.insert(1, vec![StorageScript::Redirect("https://elsewhere.test/x".into())].into());
                k.storage.insert(2, vec![StorageScript::Short; 4].into());
                k.storage.insert(3, vec![StorageScript::Tamper].into());
            }
            let mixed = run(&pair).await.map(|o| matches!(o, Outcome::Done { .. }));
            // The pinned route itself drops the connection: reqwest's error names the API URL.
            let dropped = new_pair();
            dropped.knobs().storage.insert(6, vec![StorageScript::Status(503); 4].into());
            dropped.knobs().api_chunks.insert(6, vec![ApiScript::HangUp].into());
            uses_api(run(&dropped).await);
            assert_eq!(dropped.api_chunk_requests().len(), 1, "the pinned route was asked");
            let refused = new_pair();
            refused
                .knobs()
                .grants
                .push_back(GrantScript::Json(403, json!({"error": "forbidden"})));
            let terminal = run(&refused).await.map(|_| ()).map_err(|e| e.to_string());
            (mixed, terminal)
        })
        .await;
        assert_eq!(results.0, Ok(true), "the mixed download completed");
        let error = results.1.unwrap_err();
        assert!(logs.contains("direct download:"), "the capture saw the driver's lines:\n{logs}");
        for needle in [SIGNATURE, SECRET, "AKIDSECRET", BUCKET] {
            assert_eq!(logs.matches(needle).count(), 0, "{needle} in the log:\n{logs}");
            assert!(!error.contains(needle), "{needle} in the error: {error}");
        }
        assert!(!logs.contains("/t/"), "no object path in the log:\n{logs}");
        assert!(!logs.contains("object_version_id="), "no API chunk URL in the log either:\n{logs}");
        assert!(logs.contains("pinned API route failed"), "the dropped pinned request was logged:\n{logs}");
    }

    #[test]
    fn the_ceiling_holds_requests_back_by_the_bytes_received() {
        assert_eq!(paced_until_ms(1_000, 10_000, 0), i64::MIN, "no ceiling");
        assert_eq!(paced_until_ms(1_000, 0, 8), 1_000);
        assert_eq!(paced_until_ms(1_000, 8 * 1024, 8), 2_000, "8 KiB at 8 KB/s is one second");
        assert_eq!(paced_until_ms(0, u64::MAX, 1), i64::try_from(u128::from(u64::MAX) * 1000 / 1024).unwrap_or(i64::MAX));
    }

    #[tokio::test]
    async fn the_download_ceiling_applies_to_the_direct_path() {
        let pair = new_pair();
        let started = std::time::Instant::now();
        let (bytes, _) = done(run_with(&client(&pair), &pair, Transport::Auto, 8, &Recorder::default()).await);
        assert_eq!(bytes, pair.plain());
        // 11 chunks of about 1 KiB at 8 KB/s: the eleventh request cannot start before seven chunks
        // (the reorder window is four) have been received, i.e. about 0.9 s.
        let elapsed = started.elapsed();
        assert!(elapsed >= std::time::Duration::from_millis(800), "took {elapsed:?}");
    }
}
