//! The API calls of direct S3 transfers v1 (task 2173): discovery, download grants and the pinned
//! per-chunk fallback route, plus the HTTP client that fetches presigned storage links.
//!
//! The DECISIONS (what an answer means, which links may be followed, when to renew or fall back)
//! live in `beebeeb_core::direct_transfer`, shared with every other client. This file only makes
//! the requests and reports what it saw. Nothing here carries a URL out: a storage error is a
//! status number, and API errors are run through `dt::scrub`.

use std::time::Duration;

use beebeeb_core::direct_transfer as dt;
use reqwest::Client;
use urlencoding::encode;

use super::ApiClient;

const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(15);
const STORAGE_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// A storage GET that receives nothing for this long has stalled (a half-open connection): it fails,
/// and the planner renews or falls back, instead of the head chunk holding up the file and with it the
/// queue. The API's per-chunk route has the same 30 s.
const STORAGE_READ_TIMEOUT: Duration = Duration::from_secs(crate::link_health::API_REQUEST_TIMEOUT_SECS);
/// The slowest link a storage GET's total time allows for: 128 KiB/s (about 1 Mbit/s).
const MIN_STORAGE_BYTES_PER_SEC: u64 = 128 * 1024;
/// An API error body worth reading: the error objects are tiny.
const MAX_ERROR_BODY_BYTES: usize = 64 * 1024;

/// The total time one storage GET of `length` bytes may take: a minute, plus the time the slowest
/// allowed link needs for the body (chunks are up to 256 MiB, so no single constant fits).
pub(crate) fn storage_timeout(length: u64) -> Duration {
    Duration::from_secs(60).saturating_add(Duration::from_secs(length / MIN_STORAGE_BYTES_PER_SEC))
}

/// How hard the pinned per-chunk route is tried before the chunk, and with it the file, goes to the
/// API path (the CLI's rule). Grants and this route share the per-user rate limiter server-side, and a
/// download that fell back to it fetches four chunks at a time, so one 429 must not cost the transfer.
#[derive(Clone, Copy)]
pub(crate) struct PinnedRetry {
    /// Attempts in all, the first one included.
    pub attempts: u32,
    /// The most the retries may wait in total; a wait that would pass it ends them.
    pub budget: Duration,
    /// The wait before the second attempt when the server named none; it doubles each time.
    pub base: Duration,
}

const PINNED_RETRY: PinnedRetry = PinnedRetry {
    attempts: 4,
    budget: Duration::from_secs(60),
    base: Duration::from_secs(1),
};

/// One attempt of the pinned route that did not give the chunk.
enum PinnedFailure {
    /// Not worth another try: an authorization answer, a gone version, a refusal that will repeat.
    Final(ChunkApiError),
    /// Could be gone in a moment: a 429, a 5xx, a held file, a network blip. `wait` is what the
    /// server asked for, if it did.
    Again { wait: Option<Duration>, message: String },
}

/// Unix milliseconds: the clock the planner and the discovery cache are given.
pub(crate) fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// The answer to a grant request.
pub(crate) enum GrantReply {
    Window(dt::DownloadGrantResponse),
    /// A 2xx whose body is not the contract's JSON.
    Malformed,
    /// Anything but a window: the classifier's verdict on the answer.
    Failed(dt::Outcome),
}

/// Why the pinned per-chunk API route did not give a chunk.
#[derive(Debug)]
pub(crate) enum ChunkApiError {
    /// 409: the pinned version is gone.
    Restart,
    /// 401/403 or a real (JSON) not-found: do not retry, do not fall back.
    Terminal(u16),
    /// Anything else (network, 5xx, wrong length). The text never holds a URL.
    Failed(String),
}

/// Read a body incrementally, giving up (`None`) when it passes `max` bytes.
async fn read_capped(resp: &mut reqwest::Response, max: usize) -> Option<Vec<u8>> {
    let mut body = Vec::new();
    while let Some(piece) = resp.chunk().await.ok()? {
        if body.len() + piece.len() > max {
            return None;
        }
        body.extend_from_slice(&piece);
    }
    Some(body)
}

/// Turn a finished response into what the classifier needs, reading at most `max` body bytes.
async fn answer_of(mut resp: reqwest::Response, max: usize) -> (dt::ApiAnswer, Vec<u8>) {
    let status = resp.status().as_u16();
    let header = |name: &str| {
        resp.headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
    };
    let content_type = header("content-type");
    let retry_after = header("retry-after");
    let body = read_capped(&mut resp, max).await.unwrap_or_default();
    let answer = dt::ApiAnswer::from_parts(status, content_type.as_deref(), &body, retry_after.as_deref());
    (answer, body)
}

/// Is `api_base` loopback http (local development)? The storage client then allows plain http,
/// the same switch `dt::check_storage_url` uses.
fn is_loopback_http(api_base: &str) -> bool {
    reqwest::Url::parse(api_base)
        .map(|u| u.scheme() == "http" && matches!(u.host_str(), Some("127.0.0.1" | "localhost")))
        .unwrap_or(false)
}

/// The client for presigned storage links (R1.11). It is NOT [`ApiClient`]'s `client`: that one
/// sends `X-Beebeeb-*` headers on every request, and a storage host must learn nothing about the
/// account or the client. This one has no default headers, no cookie store, never follows a
/// redirect, and refuses plain http unless the API itself is on loopback http. `None` if it cannot
/// be built: the caller then stays on the API rather than use a less restricted client.
pub(super) fn build_storage_client(api_base: &str) -> Option<Client> {
    build_storage_client_with(api_base, STORAGE_READ_TIMEOUT)
}

/// [`build_storage_client`] with another idle timeout (tests). The total time of a GET is set per
/// request by [`storage_timeout`], from the chunk's length.
fn build_storage_client_with(api_base: &str, read_timeout: Duration) -> Option<Client> {
    Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .https_only(!is_loopback_http(api_base))
        .connect_timeout(STORAGE_CONNECT_TIMEOUT)
        .read_timeout(read_timeout)
        .build()
        .ok()
}

/// Fetch one chunk from its presigned link: `200` with a `Content-Length` equal to the grant's
/// `length`, nothing else. The error is at most a status (0: no HTTP answer): reqwest's error
/// text holds the URL, so it never leaves this function.
pub(crate) async fn fetch_from_storage(client: &Client, grant: &dt::ChunkGrant) -> Result<Vec<u8>, u16> {
    let resp = client
        .get(&grant.url)
        .timeout(storage_timeout(grant.length))
        .send()
        .await
        .map_err(|_| 0u16)?;
    let status = resp.status().as_u16();
    if status != 200 || resp.content_length() != Some(grant.length) {
        return Err(status);
    }
    // The HTTP stack frames the body by that Content-Length: a short body is a read error, a longer
    // one never reaches us. The grant's length was checked against the version's geometry by the
    // planner, so memory is bounded by one chunk.
    let body = resp.bytes().await.map_err(|_| 0u16)?;
    Ok(body.to_vec())
}

impl ApiClient {
    /// The API origin this client talks to (the authority check needs it).
    pub(crate) fn api_base(&self) -> &str {
        &self.base_url
    }

    /// The storage-link client, or `None` when it could not be built.
    pub(crate) fn storage_client(&self) -> Option<&Client> {
        self.storage_client
            .get_or_init(|| build_storage_client(&self.base_url))
            .as_ref()
    }

    /// Does this server offer direct downloads right now? Asks the discovery route lazily and
    /// remembers the answer (R1.6). `None` means "use the API for this file", for any reason: an
    /// old server, a node without the route, a network failure, a server that offers nothing.
    /// An authorization answer (401/403) is not handled here: the API path that follows meets it
    /// on its own request.
    pub(crate) async fn direct_download_offer(&self) -> Option<dt::Discovery> {
        if self.token.is_empty() {
            return None;
        }
        let mut cache = self.direct_discovery.lock().await;
        match cache.lookup(now_ms()) {
            dt::Lookup::Cached(discovery) => discovery.offers_download().then_some(discovery),
            dt::Lookup::UseApi => None,
            dt::Lookup::Fetch => {
                let (outcome, discovery) = self.fetch_discovery().await;
                cache.record(&outcome, discovery.clone(), now_ms());
                discovery.filter(dt::Discovery::offers_download)
            }
        }
    }

    async fn fetch_discovery(&self) -> (dt::Outcome, Option<dt::Discovery>) {
        // The capabilities header is NOT sent here: only grant requests carry it.
        let sent = self
            .client
            .get(self.url(dt::DISCOVERY_PATH))
            .header("Authorization", format!("Bearer {}", self.token))
            .timeout(DISCOVERY_TIMEOUT)
            .send()
            .await;
        let (answer, body) = match sent {
            Ok(resp) => answer_of(resp, dt::wire::MAX_DISCOVERY_BYTES).await,
            Err(_) => (dt::ApiAnswer::no_answer(), Vec::new()),
        };
        match dt::classify(dt::Call::Discovery, &answer) {
            dt::Outcome::Proceed => match dt::Discovery::from_json(&body) {
                Ok(discovery) => (dt::Outcome::Proceed, Some(discovery)),
                Err(_) => (dt::Outcome::UseApiNow, None),
            },
            other => (other, None),
        }
    }

    /// `POST /api/v1/files/{id}/download-grants` with the capability header (this call only).
    pub(crate) async fn download_grants(&self, file_id: &str, request: &dt::DownloadGrantRequest) -> GrantReply {
        let sent = self
            .client
            .post(self.url(&dt::download_grants_path(&encode(file_id))))
            .header("Authorization", format!("Bearer {}", self.token))
            .header(dt::CAPABILITIES_HEADER, dt::CAP_OBJECT_DOWNLOAD_V1)
            .json(request)
            .send()
            .await;
        let (answer, body) = match sent {
            Ok(resp) => answer_of(resp, dt::wire::MAX_GRANT_BODY_BYTES).await,
            Err(_) => (dt::ApiAnswer::no_answer(), Vec::new()),
        };
        match dt::classify(dt::Call::DownloadGrant, &answer) {
            dt::Outcome::Proceed => match dt::DownloadGrantResponse::from_json(&body) {
                Ok(window) => GrantReply::Window(window),
                Err(_) => GrantReply::Malformed,
            },
            other => GrantReply::Failed(other),
        }
    }

    /// `GET /api/v1/files/{id}/chunks/{index}?object_version_id=<pinned>`: one chunk of the pinned
    /// version through the API, the fallback for a chunk storage could not give and the re-fetch
    /// of one that did not authenticate. `expected_len` is the stored length the grant manifest
    /// states for this chunk. A transient failure (429, 5xx, a held file, a network blip) is retried
    /// a few times, honouring `Retry-After`; only when that is spent does the caller give the file
    /// to the API path.
    pub(crate) async fn download_chunk_pinned(
        &self,
        file_id: &str,
        index: u32,
        version: &str,
        expected_len: u64,
    ) -> Result<Vec<u8>, ChunkApiError> {
        self.download_chunk_pinned_with(PINNED_RETRY, file_id, index, version, expected_len)
            .await
    }

    pub(crate) async fn download_chunk_pinned_with(
        &self,
        retry: PinnedRetry,
        file_id: &str,
        index: u32,
        version: &str,
        expected_len: u64,
    ) -> Result<Vec<u8>, ChunkApiError> {
        let mut waited = Duration::ZERO;
        let mut attempt: u32 = 1;
        loop {
            let (wait, message) = match self.pinned_attempt(file_id, index, version, expected_len).await {
                Ok(bytes) => return Ok(bytes),
                Err(PinnedFailure::Final(e)) => return Err(e),
                Err(PinnedFailure::Again { wait, message }) => {
                    let doubling = 2u32.saturating_pow(attempt - 1);
                    (wait.unwrap_or_else(|| retry.base.saturating_mul(doubling)), message)
                }
            };
            if attempt >= retry.attempts || waited.saturating_add(wait) > retry.budget {
                return Err(ChunkApiError::Failed(message));
            }
            tracing::debug!(chunk = index, attempt, %message, "pinned API route: trying again");
            tokio::time::sleep(wait).await;
            waited += wait;
            attempt += 1;
        }
    }

    async fn pinned_attempt(
        &self,
        file_id: &str,
        index: u32,
        version: &str,
        expected_len: u64,
    ) -> Result<Vec<u8>, PinnedFailure> {
        let resp = self
            .client
            .get(self.chunk_url(file_id, index))
            .query(&[("object_version_id", version)])
            .header("Authorization", format!("Bearer {}", self.token))
            .send()
            .await
            .map_err(|e| PinnedFailure::Again {
                wait: None,
                message: dt::scrub(&e.without_url().to_string()),
            })?;
        let status = resp.status();
        if !status.is_success() {
            let (answer, _) = answer_of(resp, MAX_ERROR_BODY_BYTES).await;
            let failed = format!("chunk {index} through the API failed ({})", status.as_u16());
            // On this route a 409 means the pinned version is gone. The server's body is the sentence
            // "download version unavailable; restart download from fresh metadata" (task 1643), not
            // the grant route's `version_unavailable`, so the status decides. Only "another operation
            // holds the file" is a wait.
            if answer.status == 409 && !matches!(answer.error_code(), Some("replace_in_progress" | "commit_in_progress")) {
                return Err(PinnedFailure::Final(ChunkApiError::Restart));
            }
            // Otherwise the grant's questions: authorization is final, "held" and rate limits wait, a
            // server hiccup is tried again.
            return Err(match dt::classify(dt::Call::DownloadGrant, &answer) {
                dt::Outcome::Terminal(t) => PinnedFailure::Final(ChunkApiError::Terminal(t.status)),
                dt::Outcome::Restart => PinnedFailure::Final(ChunkApiError::Restart),
                dt::Outcome::Backoff(wait) => PinnedFailure::Again {
                    wait: Some(wait),
                    message: failed,
                },
                dt::Outcome::UseApiNow => PinnedFailure::Again {
                    wait: None,
                    message: failed,
                },
                _ => PinnedFailure::Final(ChunkApiError::Failed(failed)),
            });
        }
        if resp.content_length() != Some(expected_len) {
            return Err(PinnedFailure::Final(ChunkApiError::Failed(format!(
                "chunk {index} through the API: a different length than the manifest"
            ))));
        }
        let mut resp = resp;
        match read_capped(&mut resp, usize::try_from(expected_len).unwrap_or(usize::MAX)).await {
            Some(bytes) if bytes.len() as u64 == expected_len => Ok(bytes),
            _ => Err(PinnedFailure::Again {
                wait: None,
                message: format!("chunk {index} through the API: the connection broke while reading"),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use super::{PinnedRetry, build_storage_client, build_storage_client_with, fetch_from_storage, storage_timeout};
    use crate::api_client::ApiClient;
    use crate::direct_download::test_support::{Mock, Reply, Seen};

    #[test]
    fn a_storage_get_may_take_a_minute_plus_the_slowest_link_for_its_length() {
        assert_eq!(storage_timeout(0), Duration::from_secs(60));
        assert_eq!(storage_timeout(4 * 1024 * 1024), Duration::from_secs(60 + 32), "a 4 MiB chunk");
        assert_eq!(storage_timeout(256 * 1024 * 1024), Duration::from_secs(60 + 2048), "a 256 MiB chunk");
        assert_eq!(storage_timeout(u64::MAX), Duration::from_secs(60 + u64::MAX / (128 * 1024)));
    }

    #[tokio::test]
    async fn a_storage_connection_that_goes_silent_fails_at_the_idle_timeout() {
        // Headers and part of the body, then nothing: a half-open connection.
        let storage = Mock::start(Arc::new(|_: &Seen| Reply::bytes(vec![7; 64 * 1024]).stalling(1024, Duration::from_secs(8))));
        let client = build_storage_client_with(&storage.base, Duration::from_millis(300)).expect("client");
        let grant = beebeeb_core::direct_transfer::ChunkGrant {
            index: 0,
            method: "GET".into(),
            url: format!("{}/b/k", storage.base),
            length: 64 * 1024,
            expires_at: "2026-10-25T00:00:00Z".into(),
        };
        let started = Instant::now();
        assert_eq!(fetch_from_storage(&client, &grant).await, Err(0), "a failed fetch, no status");
        let took = started.elapsed();
        assert!(took < Duration::from_secs(4), "the idle timeout ended it, not the server's 8 s silence: {took:?}");
    }

    #[tokio::test]
    async fn the_pinned_route_gives_up_after_its_attempts() {
        let api = Mock::start(Arc::new(|_: &Seen| Reply::json(503, serde_json::json!({"error": "busy"}))));
        let client = ApiClient::new(api.base.clone(), "token".into(), [0; 32]);
        let fast = PinnedRetry {
            attempts: 4,
            budget: Duration::from_secs(60),
            base: Duration::from_millis(10),
        };
        let got = client
            .download_chunk_pinned_with(fast, "f", 0, "11111111-2222-4333-8444-555555555555", 38)
            .await;
        assert!(matches!(got, Err(super::ChunkApiError::Failed(_))), "{got:?}");
        assert_eq!(api.seen().len(), 4, "four attempts, then the caller's fallback");
        // A budget too small for the next wait ends the retries at once.
        let api = Mock::start(Arc::new(|_: &Seen| Reply::json(503, serde_json::json!({"error": "busy"}))));
        let client = ApiClient::new(api.base.clone(), "token".into(), [0; 32]);
        let tight = PinnedRetry {
            budget: Duration::from_millis(5),
            ..fast
        };
        let _ = client
            .download_chunk_pinned_with(tight, "f", 0, "11111111-2222-4333-8444-555555555555", 38)
            .await;
        assert_eq!(api.seen().len(), 1);
    }

    #[tokio::test]
    async fn the_storage_client_is_https_only_unless_the_api_is_loopback_http() {
        // A production API: a plain-http storage link is refused before any connection.
        let client = build_storage_client("https://api.beebeeb.test").expect("client");
        let err = client.get("http://storage.test/b/k").send().await.unwrap_err();
        assert!(err.is_builder(), "plain http must be refused up front, got: {err}");
        // Development against a loopback http API: http reaches the connector (port 9 refuses it).
        for api in ["http://127.0.0.1:3001", "http://localhost:3001"] {
            let dev = build_storage_client(api).expect("client");
            let err = dev.get("http://127.0.0.1:9/b/k").send().await.unwrap_err();
            assert!(!err.is_builder(), "{api}: http must reach the connector, got: {err}");
        }
        // A look-alike is not loopback.
        let fake = build_storage_client("http://localhost.evil.test:3001").expect("client");
        let err = fake.get("http://127.0.0.1:9/b/k").send().await.unwrap_err();
        assert!(err.is_builder(), "a look-alike API base must not enable plain http");
    }
}
