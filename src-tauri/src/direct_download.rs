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
    pub expected_format: beebeeb_core::chunk_stream::ExpectedChunkFormat,
    /// The user's download ceiling in KB/s (`0`: none), as on the API path.
    pub download_kbps_limit: u64,
    /// Sizes the plaintext buffer only.
    pub approx_size: usize,
    /// The version the metadata named, with its manifest (task 1643). The first window pins
    /// whatever is current, so landing on another version means the file was replaced after the
    /// metadata was read: a restart from fresh metadata, never the API path with the old geometry.
    /// `None`: the metadata names no version (a legacy file).
    pub pin: Option<&'a crate::download_pin::PinnedVersion>,
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
    let mut reader = beebeeb_core::chunk_stream::ChunkDecryptor::for_push_with_file_key(
        beebeeb_core::kdf::FileKey::from_bytes(*request.file_key.as_bytes()), request.file_id, request.expected_format);
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
                        reader.finish().map_err(|_| DirectError::Corrupt)?;
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
                        if let Some(pin) = request.pin {
                            let landed = planner
                                .pinned_version()
                                .and_then(|v| uuid::Uuid::parse_str(v).ok())
                                .map(|v| v.to_string());
                            if landed.as_deref() != Some(pin.object_version_id.as_str()) {
                                tracing::debug!("direct download: the grant names another version than the metadata");
                                return Err(DirectError::VersionChanged);
                            }
                            // The same version: the grant's sizes are its manifest's, or the API path
                            // (which checks every chunk against the manifest) serves the file.
                            let manifest = dt::Geometry {
                                chunk_count: pin.chunk_count,
                                chunk_size: pin.chunk_size_bytes,
                                encrypted_size: pin.ciphertext_size_bytes,
                                original_size: pin.plaintext_size_bytes,
                            };
                            if planner.geometry() != Some(manifest) {
                                tracing::debug!("direct download: the grant's sizes differ from the version's manifest");
                                return Ok(Outcome::UseApi);
                            }
                        }
                        if planner.geometry().map(|g| g.chunk_count) != Some(request.chunk_count) {
                            // The same version (or a legacy file) whose manifest and metadata
                            // disagree: the API path, which has its own checks, serves it.
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
            match crate::engine_bridge::content_frame(&frame).and_then(|frame| reader.push_frame(&frame).map(|opened| opened.data).map_err(Into::into)) {
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
    }

    impl Reply {
        pub(crate) fn json(status: u16, value: Value) -> Self {
            Reply {
                status,
                headers: vec![("Content-Type".into(), "application/json".into())],
                body: value.to_string().into_bytes(),
                hang_up: false,
                stall: None,
            }
        }
        pub(crate) fn bytes(body: Vec<u8>) -> Self {
            Reply {
                status: 200,
                headers: vec![("Content-Type".into(), "application/octet-stream".into())],
                body,
                hang_up: false,
                stall: None,
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
            }
        }
        pub(crate) fn redirect(location: String) -> Self {
            Reply {
                status: 302,
                headers: vec![("Location".into(), location)],
                body: Vec::new(),
                hang_up: false,
                stall: None,
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
            }
        }
        /// `self`, but after `sent` bytes of it the connection goes silent for `silence`.
        pub(crate) fn stalling(mut self, sent: usize, silence: std::time::Duration) -> Self {
            self.stall = Some((sent, silence));
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
