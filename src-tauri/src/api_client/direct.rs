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
/// A chunk can be 256 MiB; this bounds one storage GET, not the file.
const STORAGE_TIMEOUT: Duration = Duration::from_secs(600);
/// An API error body worth reading: the error objects are tiny.
const MAX_ERROR_BODY_BYTES: usize = 64 * 1024;

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
    /// 409 `version_unavailable`: the pinned version is gone.
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
    Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .https_only(!is_loopback_http(api_base))
        .connect_timeout(STORAGE_CONNECT_TIMEOUT)
        .timeout(STORAGE_TIMEOUT)
        .build()
        .ok()
}

/// Fetch one chunk from its presigned link: `200` with a `Content-Length` equal to the grant's
/// `length`, nothing else. The error is at most a status (0: no HTTP answer): reqwest's error
/// text holds the URL, so it never leaves this function.
pub(crate) async fn fetch_from_storage(client: &Client, grant: &dt::ChunkGrant) -> Result<Vec<u8>, u16> {
    let resp = client.get(&grant.url).send().await.map_err(|_| 0u16)?;
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
    /// states for this chunk.
    pub(crate) async fn download_chunk_pinned(
        &self,
        file_id: &str,
        index: u32,
        version: &str,
        expected_len: u64,
    ) -> Result<Vec<u8>, ChunkApiError> {
        let resp = self
            .client
            .get(self.chunk_url(file_id, index))
            .query(&[("object_version_id", version)])
            .header("Authorization", format!("Bearer {}", self.token))
            .send()
            .await
            .map_err(|e| ChunkApiError::Failed(dt::scrub(&e.without_url().to_string())))?;
        let status = resp.status();
        if !status.is_success() {
            let (answer, _) = answer_of(resp, MAX_ERROR_BODY_BYTES).await;
            return Err(match (status.as_u16(), answer.error_code()) {
                (401 | 403, _) => ChunkApiError::Terminal(status.as_u16()),
                (404, Some(_)) => ChunkApiError::Terminal(404),
                (409, Some("version_unavailable")) => ChunkApiError::Restart,
                (code, _) => ChunkApiError::Failed(format!("chunk {index} through the API failed ({code})")),
            });
        }
        if resp.content_length() != Some(expected_len) {
            return Err(ChunkApiError::Failed(format!(
                "chunk {index} through the API: a different length than the manifest"
            )));
        }
        let mut resp = resp;
        match read_capped(&mut resp, usize::try_from(expected_len).unwrap_or(usize::MAX)).await {
            Some(bytes) if bytes.len() as u64 == expected_len => Ok(bytes),
            _ => Err(ChunkApiError::Failed(format!(
                "chunk {index} through the API: the body ended early or ran long"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::build_storage_client;

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
