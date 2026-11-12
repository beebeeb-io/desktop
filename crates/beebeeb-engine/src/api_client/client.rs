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
        }
    }

    /// This client with `timeout` for each request in place of the app's
    /// [`crate::link_health::API_REQUEST_TIMEOUT_SECS`]: for a listing that may move a whole tree on a slow link and
    /// that nobody waits on (task 2267's ownership proof, self-review F5).
    #[cfg(target_os = "windows")]
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
        if variant == "medium" {
            if let Some(blurhash) = blurhash.filter(|value| !value.is_empty()) {
                url.push_str("?blurhash=");
                url.push_str(&encode(blurhash));
            }
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
    //   * retry on HTTP 429 (rate limit) — up to `MAX_429_RETRIES` extra
    //     attempts — honouring `Retry-After` / `x-ratelimit-reset` for the
    //     per-attempt wait (capped at `RETRY_WAIT_CAP_SECS`) and bounding the
    //     CUMULATIVE wait at `RETRY_TOTAL_BUDGET_SECS` so a hammered endpoint
    //     can't stall a view indefinitely. This makes a transient 429 (the prod
    //     account-data endpoints currently 429 under load) retry gracefully
    //     instead of surfacing "Couldn't load" to the view.
    //   * surface a clean `<status>: <body-snippet>` error on any other
    //     non-2xx instead of reqwest's opaque default.

    /// Internal: GET a JSON endpoint and deserialize it, with bounded 429
    /// backoff + retry. `path` is the API path (leading slash); auth is attached.
    pub(super) async fn get_typed<T: serde::de::DeserializeOwned>(&self, path: &str) -> ApiResult<T> {
        let url = self.url(path);
        let resp = self.send_with_retry(|| self.client.get(&url)).await?;
        Ok(resp.json::<T>().await?)
    }

    /// Internal: issue an arbitrary request (built by `build`) with bounded 429
    /// backoff + retry, returning a 2xx response or a descriptive error.
    ///
    /// On a 429 we read the server's hint (`Retry-After` seconds or
    /// `x-ratelimit-reset` epoch-seconds), clamp it to a sane per-attempt cap,
    /// sleep, and retry — up to `MAX_429_RETRIES` extra attempts and a total
    /// sleep budget of `RETRY_TOTAL_BUDGET_SECS`. The final 429 (or a non-429
    /// non-2xx at any point) is surfaced via `check_status`.
    pub(super) async fn send_with_retry(
        &self,
        build: impl Fn() -> reqwest::RequestBuilder,
    ) -> ApiResult<reqwest::Response> {
        let mut spent: u64 = 0;
        let mut attempt: u32 = 0;
        loop {
            let resp = match build()
                .header("Authorization", format!("Bearer {}", self.token))
                .send()
                .await
            {
                Ok(resp) => resp,
                Err(e) => {
                    self.link.note_reqwest_error(&e, now_unix_secs() as i64);
                    return Err(e.into());
                }
            };
            self.link.note_status(resp.status(), now_unix_secs() as i64);
            if resp.status() != reqwest::StatusCode::TOO_MANY_REQUESTS {
                return Self::check_status(resp).await;
            }
            // 429: decide whether (and how long) to wait before retrying.
            let retry_after = header_secs(resp.headers(), reqwest::header::RETRY_AFTER.as_str());
            let reset_in =
                header_secs(resp.headers(), "x-ratelimit-reset").map(|epoch| epoch.saturating_sub(now_unix_secs()));
            match retry_decision(attempt, spent, retry_after, reset_in) {
                Some(wait) => {
                    if wait > 0 {
                        tokio::time::sleep(Duration::from_secs(wait)).await;
                    }
                    spent = spent.saturating_add(wait);
                    attempt += 1;
                    // Loop back and retry.
                }
                // Budget/attempt exhausted — surface the 429 as a clean error.
                None => return Self::check_status(resp).await,
            }
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
