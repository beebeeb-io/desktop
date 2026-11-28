//! The `ApiClient` value: construction, the URLs it owns, and the request
//! methods the daemon's sync engine and the Tauri commands call.
//!
//! Owns: the master key and session token, per-request timeouts, and the link
//! bookkeeping every request records. Must not: define wire DTOs (see `types`)
//! or the provenance/retry rules (see `headers`). Tests: `api_client/tests/`.

use super::*;

pub struct ApiClient {
    // Dropped AFTER Drop has zeroized credentials, unlike Weak<ApiClient>'s
    // strong count (which reaches zero before its destructor runs).
    #[cfg(target_os = "windows")]
    pub(super) credential_lifetime: std::sync::Arc<()>,
    pub(super) base_url: String,
    pub(super) token: String,
    pub(super) master_key: [u8; 32],
    pub(super) client: Client,
    /// Outcome of the most recent request, read by the runner after each tick
    /// (see `link_health`). Not secret.
    pub(super) link: std::sync::Arc<crate::link_health::LinkMonitor>,
    /// Task 2173: what the server's direct-transfer discovery said, asked lazily and trusted for
    /// at most 10 minutes (`beebeeb_core::direct_transfer::DiscoveryCache`). It lives and dies
    /// with this client, i.e. with the signed-in session: nothing of it outlives a sign-out or
    /// an account switch.
    pub(super) direct_discovery: tokio::sync::Mutex<beebeeb_core::direct_transfer::DiscoveryCache>,
    /// Task 2173: the client for presigned storage links, built on first use. Never `client`:
    /// no default headers (so no `X-Beebeeb-*`), no cookies, no `Authorization`, redirects
    /// refused, https only (loopback http when the API itself is on loopback http).
    pub(super) storage_client: std::sync::OnceLock<Option<Client>>,
    /// The pause after a 429, shared by every request of this session (and its backup-pass client).
    pub(super) rate_pause: std::sync::Arc<RatePause>,
}

impl Drop for ApiClient {
    fn drop(&mut self) {
        self.master_key.zeroize();
        self.token.zeroize();
    }
}

// Hand-implemented (can't `#[derive(ZeroizeOnDrop)]` — `client: reqwest::Client`
// isn't `Zeroize` — see the struct's doc comment). `ZeroizeOnDrop` is a plain,
// safe marker trait with no required methods (zeroize 1.x); implementing it
// here is what lets a test assert, at compile time, that dropping an
// `ApiClient` is guaranteed to wipe its key material — the same guarantee
// `Session::ZeroizeOnDrop` gives, verified the same way in `lib.rs`'s
// `session_zeroizes_master_key_on_drop` test.
impl zeroize::ZeroizeOnDrop for ApiClient {}

/// The longest a bulk GET may run in all, however steadily its body arrives.
const BULK_GET_CEILING: Duration = Duration::from_secs(15 * 60);

/// The longest the body of a 429 is waited for before the pause starts from the header alone.
const RATE_BODY_WAIT: Duration = Duration::from_secs(2);

/// The `retry_after` (whole seconds) a 429's JSON body names, if it names one.
pub(super) fn body_retry_after(body: &str) -> Option<u64> {
    let value = serde_json::from_str::<serde_json::Value>(body).ok()?;
    let hint = value.get("retry_after")?;
    hint.as_u64()
        .or_else(|| hint.as_f64().filter(|secs| *secs >= 0.0).map(|secs| secs.ceil() as u64))
}

impl ApiClient {
    #[cfg(target_os = "windows")]
    pub fn credential_lifetime(&self) -> std::sync::Weak<()> {
        std::sync::Arc::downgrade(&self.credential_lifetime)
    }

    /// Build a new client. `base_url` should NOT have a trailing slash
    /// (e.g. `https://api.beebeeb.io`). The 30-second timeout is the
    /// upper bound for any single chunk request — bigger files arrive
    /// chunk-at-a-time so a single slow chunk doesn't kill a large
    /// download.
    pub fn new(base_url: String, token: String, master_key: [u8; 32]) -> Self {
        let base_url = base_url.trim_end_matches('/').to_string();
        // The proxy rule of the API base (`api_client::proxy`), like every client built here.
        let client = api_base_client_builder(&base_url)
            .timeout(Duration::from_secs(crate::link_health::API_REQUEST_TIMEOUT_SECS))
            .default_headers(provenance_headers())
            .build()
            .expect("reqwest client");
        Self {
            #[cfg(target_os = "windows")]
            credential_lifetime: std::sync::Arc::new(()),
            base_url,
            token,
            master_key,
            client,
            link: std::sync::Arc::new(crate::link_health::LinkMonitor::new()),
            direct_discovery: tokio::sync::Mutex::new(beebeeb_core::direct_transfer::DiscoveryCache::new()),
            storage_client: std::sync::OnceLock::new(),
            rate_pause: std::sync::Arc::new(RatePause::new()),
        }
    }

    /// Tests: this client with a shorter 429 pause (`floor` doubling up to `cap`) so a refusal does not cost seconds.
    #[cfg(test)]
    pub(crate) fn with_rate_limits(mut self, floor: Duration, cap: Duration) -> Self {
        self.rate_pause = std::sync::Arc::new(RatePause::with_limits(floor, cap));
        self
    }

    /// This client with `timeout` for each request in place of the app's
    /// [`crate::link_health::API_REQUEST_TIMEOUT_SECS`]: for a listing that may move a whole tree on a slow link and
    /// that nobody waits on (task 2267's ownership proof, self-review F5).
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    pub fn with_request_timeout(mut self, timeout: Duration) -> Self {
        self.client = api_base_client_builder(&self.base_url)
            .timeout(timeout)
            .default_headers(provenance_headers())
            .build()
            .expect("reqwest client");
        self
    }

    /// LD-0113 (spec Amendment 8): this session's client for the backup pass. Every request it makes carries
    /// `X-Beebeeb-Origin: backup` (`beebeeb_types::backup::{ORIGIN_HEADER, ORIGIN_BACKUP}`), on top of the provenance
    /// headers, so the server can tell the one writer of a device's backup from everything else (it ignores the
    /// header until phase 2). Same session, keys, link monitor and timeout; its own HTTP client.
    pub fn for_backup_pass(&self) -> Self {
        let mut headers = provenance_headers();
        headers.insert(
            beebeeb_types::backup::ORIGIN_HEADER,
            reqwest::header::HeaderValue::from_static(beebeeb_types::backup::ORIGIN_BACKUP),
        );
        Self {
            #[cfg(target_os = "windows")]
            credential_lifetime: self.credential_lifetime.clone(),
            base_url: self.base_url.clone(),
            token: self.token.clone(),
            master_key: self.master_key,
            client: api_base_client_builder(&self.base_url)
                .timeout(Duration::from_secs(crate::link_health::API_REQUEST_TIMEOUT_SECS))
                .default_headers(headers)
                .build()
                .expect("reqwest client"),
            link: self.link.clone(),
            direct_discovery: tokio::sync::Mutex::new(beebeeb_core::direct_transfer::DiscoveryCache::new()),
            storage_client: std::sync::OnceLock::new(),
            rate_pause: self.rate_pause.clone(),
        }
    }

    /// The shared record of whether the last requests reached the server.
    pub fn link(&self) -> std::sync::Arc<crate::link_health::LinkMonitor> {
        self.link.clone()
    }

    /// Helper for tests + downloaders: full URL of a single chunk.
    pub fn chunk_url(&self, file_id: FileId<'_>, chunk_idx: u32) -> String {
        format!(
            "{}/api/v1/files/{}/chunks/{}",
            self.base_url,
            encode(file_id.as_str()),
            chunk_idx
        )
    }

    /// Full URL of an encrypted thumbnail variant
    /// (`GET /api/v1/files/:id/thumbnail/{variant}`). `variant` is one of
    /// `small` / `medium` / `large`; the server maps each to a reserved sentinel
    /// chunk index. The returned body is the *encrypted* thumbnail blob in the
    /// raw `nonce(12) || AES-256-GCM(ciphertext+tag)` envelope the web/mobile
    /// clients uploaded — decryption is the caller's job.
    pub fn thumbnail_url(&self, file_id: FileId<'_>, variant: &str) -> String {
        format!(
            "{}/api/v1/files/{}/thumbnail/{}",
            self.base_url,
            encode(file_id.as_str()),
            variant
        )
    }

    /// Full URL for uploading an encrypted thumbnail variant. The medium
    /// variant intentionally uses `/thumbnail` (no `/medium` suffix), matching
    /// web/mobile. Other variants use `/thumbnail/{variant}`.
    pub fn thumbnail_upload_url(&self, file_id: FileId<'_>, variant: &str, blurhash: Option<&str>) -> String {
        let mut url = if variant == "medium" {
            format!("{}/api/v1/files/{}/thumbnail", self.base_url, encode(file_id.as_str()))
        } else {
            format!(
                "{}/api/v1/files/{}/thumbnail/{}",
                self.base_url,
                encode(file_id.as_str()),
                encode(variant)
            )
        };
        if variant == "medium"
            && let Some(blurhash) = blurhash.filter(|value| !value.is_empty())
        {
            url.push_str("?blurhash=");
            url.push_str(&encode(blurhash));
        }
        url
    }

    pub fn upload_init_url(&self) -> String {
        self.url("/api/v1/uploads/init")
    }

    pub fn upload_chunk_url(&self, upload_session_id: UploadSessionId<'_>, chunk_idx: u32) -> String {
        self.url(&format!(
            "/api/v1/uploads/{}/chunks/{}",
            encode(upload_session_id.as_str()),
            chunk_idx
        ))
    }

    pub fn upload_complete_url(&self, upload_session_id: UploadSessionId<'_>) -> String {
        self.url(&format!(
            "/api/v1/uploads/{}/complete",
            encode(upload_session_id.as_str())
        ))
    }

    pub fn file_url(&self, file_id: FileId<'_>) -> String {
        self.url(&format!("/api/v1/files/{}", encode(file_id.as_str())))
    }

    pub fn upload_abandon_url(&self, file_id: FileId<'_>) -> String {
        self.url(&format!("/api/v1/files/{}/upload/abandon", encode(file_id.as_str())))
    }

    pub fn restore_file_url(&self, file_id: FileId<'_>) -> String {
        self.url(&format!("/api/v1/files/{}/restore", encode(file_id.as_str())))
    }

    pub fn permanent_file_url(&self, file_id: FileId<'_>) -> String {
        self.url(&format!("/api/v1/files/{}/permanent", encode(file_id.as_str())))
    }

    pub fn versions_url(&self, file_id: FileId<'_>) -> String {
        self.url(&format!("/api/v1/files/{}/versions", encode(file_id.as_str())))
    }

    /// Tests only: the download path is bound to a pinned version (`download_pin`), so no production caller builds this URL.
    #[cfg(any(test, feature = "test-support"))]
    pub fn version_download_url(&self, file_id: FileId<'_>, version_id: VersionId<'_>) -> String {
        self.url(&format!(
            "/api/v1/files/{}/versions/{}/download",
            encode(file_id.as_str()),
            encode(version_id.as_str())
        ))
    }

    pub fn version_restore_url(&self, file_id: FileId<'_>, version_id: VersionId<'_>) -> String {
        self.url(&format!(
            "/api/v1/files/{}/versions/{}/restore",
            encode(file_id.as_str()),
            encode(version_id.as_str())
        ))
    }

    /// Tests only: no desktop caller lists folder members; the URL contract is still pinned.
    #[cfg(any(test, feature = "test-support"))]
    pub fn folder_members_url(&self, folder_id: FolderId<'_>) -> String {
        self.url(&format!(
            "/api/v1/shares/invites/folder-members/{}",
            encode(folder_id.as_str())
        ))
    }

    pub fn folder_keys_url(&self, invite_id: InviteId<'_>) -> String {
        self.url(&format!(
            "/api/v1/shares/invites/{}/folder-keys",
            encode(invite_id.as_str())
        ))
    }

    pub fn shares_incoming_url(&self) -> String {
        self.url("/api/v1/shares/invites/incoming")
    }

    pub fn list_files_url(&self, scope: ListFilesScope<'_>) -> String {
        let mut query = Vec::new();
        if let Some(parent_id) = scope.parent_id {
            query.push(format!("parent_id={}", encode(parent_id.as_str())));
        }
        if let Some(shared_folder_id) = scope.shared_folder_id {
            query.push(format!("shared_folder_id={}", encode(shared_folder_id.as_str())));
        }
        if let Some(trashed) = scope.trashed {
            query.push(format!("trashed={trashed}"));
        }
        if let Some(cursor) = scope.cursor {
            query.push(format!("cursor={}", encode(cursor)));
        }
        if query.is_empty() {
            self.url("/api/v1/files")
        } else {
            self.url(&format!("/api/v1/files?{}", query.join("&")))
        }
    }

    pub fn search_shard_manifest_url(&self) -> String {
        self.url("/api/v1/search-index/shards")
    }

    pub fn search_shard_url(&self, bucket: u32, page: u32) -> String {
        self.url(&format!("/api/v1/search-index/shards/{bucket}/{page}"))
    }

    pub(super) fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    /// Borrow the master key for callers doing local crypto. Borrow,
    /// not clone — same one-authoritative-copy invariant.
    pub fn master_key(&self) -> &[u8; 32] {
        &self.master_key
    }

    // ── Read-only data-layer wrappers (account / billing / devices / etc.) ────
    //
    // These power the desktop's data-backed views. They are plain
    // `GET`/`DELETE`/`POST` calls returning typed DTOs from `account_dto`.
    // Unlike the sync-engine binary paths above, they go through
    // `get_typed` / `send_with_retry`, which:
    //   * on HTTP 429 (rate limit) start one pause every request of this client
    //     waits out (`RatePause`: `Retry-After`, or a 2 s floor that doubles, with
    //     jitter, capped at 60 s), and send a GET again (or a write that carries
    //     an idempotency key) up to `MAX_429_RETRIES` times. Other writes surface
    //     the 429: they are never replayed.
    //   * surface a clean `<status>: <body-snippet>` error on any other
    //     non-2xx instead of reqwest's opaque default.

    /// Internal: GET a JSON endpoint and deserialize it, with bounded 429
    /// backoff + retry. `path` is the API path (leading slash); auth is attached.
    pub(super) async fn get_typed<T: serde::de::DeserializeOwned>(&self, path: &str) -> ApiResult<T> {
        let url = self.url(path);
        let resp = self.send_with_retry(|| self.client.get(&url)).await?;
        Ok(resp.json::<T>().await?)
    }

    /// GET a JSON body that can be tens of MB (the whole-tree snapshot of a large account). The app's 30 s
    /// request timeout is a TOTAL cap, headers and body together, so a body that needs longer than 30 s to
    /// arrive was cut off and surfaced as "error decoding response body" (0.9.0). Here no total cap applies:
    /// a connect timeout, then `read_timeout` of silence on the connection (a stalled link still fails
    /// in bounded time) and a [`BULK_GET_CEILING`] on the whole transfer.
    pub(super) async fn get_typed_bulk<T: serde::de::DeserializeOwned>(&self, path: &str) -> ApiResult<T> {
        self.get_typed_bulk_with(path, Duration::from_secs(crate::link_health::API_REQUEST_TIMEOUT_SECS))
            .await
    }

    pub(super) async fn get_typed_bulk_with<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        read_timeout: Duration,
    ) -> ApiResult<T> {
        self.get_typed_bulk_headers(path, read_timeout, false).await
    }

    /// [`Self::get_typed_bulk_with`]; `backup` adds `X-Beebeeb-Origin: backup` (RE3-a: the backup pass's own reads).
    pub(super) async fn get_typed_bulk_headers<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        read_timeout: Duration,
        backup: bool,
    ) -> ApiResult<T> {
        let url = self.url(path);
        let mut headers = provenance_headers();
        if backup {
            headers.insert(
                beebeeb_types::backup::ORIGIN_HEADER,
                reqwest::header::HeaderValue::from_static(beebeeb_types::backup::ORIGIN_BACKUP),
            );
        }
        let http = api_base_client_builder(&self.base_url)
            .connect_timeout(Duration::from_secs(10))
            .read_timeout(read_timeout)
            .default_headers(headers)
            .build()
            .expect("reqwest client");
        let resp = self
            .send_with_retry_on(&http, || http.get(&url).timeout(BULK_GET_CEILING))
            .await?;
        Ok(resp.json::<T>().await?)
    }

    /// Internal: issue an arbitrary request (built by `build`) with the shared 429 pause,
    /// returning a 2xx response or a descriptive error.
    ///
    /// On a 429 the client's [`RatePause`] starts (the server's `Retry-After`, or a doubling
    /// floor with jitter, capped at 60 s); every request waits for it before it is sent. A GET
    /// (or a write with an idempotency key) is then sent again, up to `MAX_429_RETRIES` times;
    /// any other request is not replayed and gets the 429 as its error.
    pub(super) async fn send_with_retry(
        &self,
        build: impl Fn() -> reqwest::RequestBuilder,
    ) -> ApiResult<reqwest::Response> {
        self.send_with_retry_on(&self.client, build).await
    }

    /// [`Self::send_with_retry`] executed on `http` (a client with its own timeouts).
    pub(super) async fn send_with_retry_on(
        &self,
        http: &reqwest::Client,
        build: impl Fn() -> reqwest::RequestBuilder,
    ) -> ApiResult<reqwest::Response> {
        Self::check_status(self.send_under_pause(http, build, true).await?).await
    }

    /// One API request under the shared 429 pause, for a caller with status branches of its own (404 = absent, a
    /// typed refusal): every answer that is not a 429 comes back unchecked. `build` must not add `Authorization`;
    /// it is added here.
    ///
    /// The request waits for a running pause before it goes out. A 429 starts the pause from the larger of the
    /// `Retry-After` header and the body's `retry_after`. A GET (or a write with an idempotency key) is then sent
    /// again, up to `MAX_429_RETRIES` times; any other request is sent exactly once and its 429 is the error
    /// (`DesktopApiError::HttpStatus`), as is the last 429 of a request that was sent again.
    ///
    /// Only the 429 is handled here. A connection that fails, a timeout and a 502/503/504 come back at once and are
    /// not sent again: these callers were single sends before they waited on the pause, and the layers above them
    /// (the sync and transfer retries, the adapter's own read-again) already decide what a failure means.
    pub(super) async fn send_paced(
        &self,
        http: &reqwest::Client,
        build: impl Fn() -> reqwest::RequestBuilder,
    ) -> ApiResult<reqwest::Response> {
        self.send_under_pause(http, build, false).await
    }

    /// The send loop behind [`Self::send_paced`] and [`Self::send_with_retry_on`]. `transport_retries` adds the
    /// bounded resend after a connect failure, a timeout or a 502/503/504 (`transport_retry_decision`): the typed
    /// data-layer wrappers have it, the engine's own calls do not.
    async fn send_under_pause(
        &self,
        http: &reqwest::Client,
        build: impl Fn() -> reqwest::RequestBuilder,
        transport_retries: bool,
    ) -> ApiResult<reqwest::Response> {
        let mut attempt: u32 = 0;
        // Transport/5xx retries are counted apart from 429 retries; each class is bounded on its own.
        let mut transport_attempt: u32 = 0;
        // The last 502/503/504 seen: surfaced if the retry that followed it fails to connect at all.
        let mut held: Option<reqwest::Response> = None;
        loop {
            self.rate_pause.wait().await;
            let request = build()
                .header("Authorization", format!("Bearer {}", self.token))
                .build()?;
            let idempotent = is_idempotent(request.method());
            let again_after_429 = retries_after_429(&request);
            let resp = match http.execute(request).await {
                Ok(resp) => resp,
                Err(e) => {
                    if let Some(old) = held.take() {
                        return Ok(old);
                    }
                    if transport_retries
                        && let Some(wait) = transport_retry_decision(
                            transport_attempt,
                            e.is_connect(),
                            e.is_timeout() || e.is_request(),
                            idempotent,
                        )
                    {
                        tokio::time::sleep(wait).await;
                        transport_attempt += 1;
                        continue;
                    }
                    self.link.note_reqwest_error(&e, now_unix_secs() as i64);
                    return Err(e.into());
                }
            };
            self.link.note_status(resp.status(), now_unix_secs() as i64);
            if transport_retries
                && matches!(resp.status().as_u16(), 502..=504)
                && let Some(wait) = transport_retry_decision(transport_attempt, false, true, idempotent)
            {
                tokio::time::sleep(wait).await;
                transport_attempt += 1;
                held = Some(resp);
                continue;
            }
            let status = resp.status();
            if status != reqwest::StatusCode::TOO_MANY_REQUESTS {
                self.rate_pause.answered();
                return Ok(resp);
            }
            // 429: everyone waits; only a request that is safe to repeat is sent again. The body is a few bytes of
            // JSON; one that does not arrive must not hold the pause back.
            let header = header_secs(resp.headers(), reqwest::header::RETRY_AFTER.as_str());
            let body = tokio::time::timeout(RATE_BODY_WAIT, resp.text())
                .await
                .ok()
                .and_then(Result::ok)
                .unwrap_or_default();
            self.rate_pause.refused(header.max(body_retry_after(&body)));
            if !again_after_429 || attempt >= MAX_429_RETRIES {
                let snippet: String = body.chars().take(300).collect();
                return Err(DesktopApiError::HttpStatus { status, snippet });
            }
            attempt += 1;
        }
    }

    /// Map a non-2xx response to a descriptive error (status + a short body
    /// snippet); pass a 2xx response through unchanged.
    pub(super) async fn check_status(resp: reqwest::Response) -> ApiResult<reqwest::Response> {
        let status = resp.status();
        if status.is_success() {
            return Ok(resp);
        }
        let body = resp.text().await.unwrap_or_default();
        // L11: a custody write over a folder that still has legacy key rows is
        // refused until the owner upgrades it. It is its own state — never a
        // generic error and never a reason to retry as a legacy write.
        if status == reqwest::StatusCode::CONFLICT
            && serde_json::from_str::<serde_json::Value>(&body).is_ok_and(|value| value["error"] == "upgrade_required")
        {
            return Err(DesktopApiError::UpgradeRequired(UpgradeRequired));
        }
        let snippet: String = body.chars().take(300).collect();
        Err(DesktopApiError::HttpStatus { status, snippet })
    }
}
