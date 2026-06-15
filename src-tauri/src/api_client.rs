//! HTTP client for the Beebeeb API, used by the desktop sync daemon.
//!
//! Wraps `reqwest::Client` with the bits the daemon actually needs:
//! `list_files` for the periodic sync sweep, `download_chunk` for the
//! cloud-only → local materialisation path, `upload_chunk` for local →
//! cloud propagation, plus the per-file URL helper.
//!
//! Master key + session token are owned by the client so call sites
//! don't have to thread them through every method. The token comes
//! from the WebView via the `set_session` IPC, the master key from
//! the same IPC.
//!
//! Spec: `docs/superpowers/plans/2026-05-07-desktop-sync-client.md`
//! Phase 1 Task 2.
//!
//! ## Why a fresh client (vs reusing the CLI crate)
//!
//! The CLI's `repos/cli/src/api.rs` is great prior art but lives in a
//! separate crate that pulls in clap, indicatif, dirs::config_dir, and
//! a config-file loader the daemon doesn't want. Easier to lift just
//! the HTTP primitives than to refactor the CLI into a library.

use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use urlencoding::encode;

// ── 429 backoff/retry tuning (account-data GETs) ───────────────────────────────
//
// The account-data endpoints (auth/me, billing/*, account/*, clients/*) can 429
// under load. The client must ride out a transient 429 rather than surfacing a
// "Couldn't load" to the view, but must stay BOUNDED — it is not a hot loop and
// must never hang a view waiting on a hammered endpoint.

/// Extra attempts after the first 429 (so up to 1 + this total requests).
const MAX_429_RETRIES: u32 = 3;
/// Hard cap on any SINGLE backoff wait, regardless of what the server asks for.
/// Protects against an absurd `Retry-After`/`x-ratelimit-reset` value pinning a
/// view. ~5s keeps the UI responsive while still letting a short window clear.
const RETRY_WAIT_CAP_SECS: u64 = 5;
/// Cap on the CUMULATIVE time spent sleeping across all retries for one call.
const RETRY_TOTAL_BUDGET_SECS: u64 = 8;
/// Backoff used when the server gives no usable hint (no Retry-After / reset).
const RETRY_DEFAULT_WAIT_SECS: u64 = 1;

/// Current UNIX time in seconds (for interpreting `x-ratelimit-reset`, which is
/// an absolute epoch). Falls back to 0 on the (impossible) pre-epoch clock so
/// the reset delta just reads as "0s" rather than panicking.
fn now_unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Parse a header value as whole seconds (`u64`). Tolerant of surrounding
/// whitespace; returns `None` if the header is absent or non-numeric (e.g. an
/// HTTP-date `Retry-After`, which we don't try to parse — the default wait
/// covers that case).
fn header_secs(headers: &reqwest::header::HeaderMap, name: &str) -> Option<u64> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
}

/// Pure backoff decision for a 429 response. Returns `Some(wait_secs)` to retry
/// after sleeping `wait_secs`, or `None` to give up (surface the error).
///
/// - `attempt` is the number of retries ALREADY made (0 on the first 429).
/// - `spent` is the cumulative seconds already slept for this call.
/// - `retry_after` / `reset_in` are the server's hints (seconds), if any.
///
/// The chosen wait is `max(retry_after, reset_in)` (the server may send either),
/// defaulting to `RETRY_DEFAULT_WAIT_SECS`, then clamped to `RETRY_WAIT_CAP_SECS`
/// and further trimmed so total sleep never exceeds `RETRY_TOTAL_BUDGET_SECS`.
/// We stop once `attempt` reaches `MAX_429_RETRIES` or the budget is exhausted.
fn retry_decision(
    attempt: u32,
    spent: u64,
    retry_after: Option<u64>,
    reset_in: Option<u64>,
) -> Option<u64> {
    if attempt >= MAX_429_RETRIES || spent >= RETRY_TOTAL_BUDGET_SECS {
        return None;
    }
    // Take the larger of the two server hints; fall back to the default.
    let hint = match (retry_after, reset_in) {
        (Some(a), Some(b)) => a.max(b),
        (Some(a), None) => a,
        (None, Some(b)) => b,
        (None, None) => RETRY_DEFAULT_WAIT_SECS,
    };
    // Per-attempt cap.
    let wait = hint.min(RETRY_WAIT_CAP_SECS);
    // Don't blow the cumulative budget: trim the wait to what's left.
    let remaining = RETRY_TOTAL_BUDGET_SECS.saturating_sub(spent);
    let wait = wait.min(remaining);
    Some(wait)
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ListFilesScope<'a> {
    pub parent_id: Option<&'a str>,
    pub shared_folder_id: Option<&'a str>,
    pub trashed: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DesktopUploadInitRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_id: Option<String>,
    pub file_name: String,
    pub file_size_bytes: u64,
    pub mime_type: Option<String>,
    pub parent_id: Option<String>,
    pub profile: String,
    pub is_media: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chunk_size_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chunk_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_version_number: Option<i32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DesktopUploadInitResponse {
    pub file_id: String,
    pub tenant_id: String,
    pub object_version_id: String,
    pub upload_session_id: String,
    pub chunk_size_bytes: i64,
    pub chunk_count: i64,
    pub storage_format_version: i64,
    pub storage_pool_id: String,
    pub region: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UploadChunkResponse {
    pub index: u32,
    pub size: i64,
    pub skipped: Option<bool>,
}

// ── Client device + sync-session telemetry (the WRITE/PRODUCE side) ────────────
//
// These power the desktop's half of the Bandwidth view: on login the runner
// registers THIS device and opens ONE sync session, then the heartbeat producer
// posts live telemetry every interval. The READ side (`client_devices` /
// `client_sync_sessions` GETs above) is what the BandwidthView polls. Server
// contract: `repos/server/.../routes/clients.rs` under `/api/v1/clients`.

/// Server response to `POST /api/v1/clients/devices`. We only need the `id`
/// (to open a session against it), but capture the rest so a caller could
/// surface the device row without a follow-up GET. Extra/unknown server fields
/// are ignored by serde, matching the read-side DTO tolerance.
#[derive(Debug, Clone, Deserialize)]
pub struct RegisteredDevice {
    pub id: String,
    #[serde(default)]
    pub hostname: Option<String>,
    #[serde(default)]
    pub platform: Option<String>,
    #[serde(default)]
    pub bb_version: Option<String>,
}

/// Server response to `POST /api/v1/clients/sessions`. The heartbeat producer
/// keys off `id` + the (possibly server-defaulted) `heartbeat_interval_secs`.
#[derive(Debug, Clone, Deserialize)]
pub struct CreatedSession {
    pub id: String,
    /// Server defaults this to 60 when the request omits it; we send our own.
    #[serde(default)]
    pub heartbeat_interval_secs: Option<i32>,
}

/// Body for `POST /api/v1/clients/sessions/{id}/heartbeat`.
///
/// Mirrors the server's `HeartbeatBody` exactly. `status` is required (a free-
/// form lowercase string the read side maps to a label: `watching` / `syncing`
/// / `paused` / `idle` / `error`); every other field is optional and skipped on
/// the wire when `None`, so a minimal heartbeat is just `{status}`.
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct HeartbeatBody {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub files_synced: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub files_total: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes_synced: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes_total: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speed_bps: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Owned HTTP client + credentials. One per running daemon process.
///
/// Cloning is intentionally not implemented — the master key has the
/// same security invariant as in the Tauri `Session` struct: exactly
/// one authoritative copy in memory at a time.
pub struct ApiClient {
    base_url: String,
    token: String,
    master_key: [u8; 32],
    client: Client,
}

impl ApiClient {
    /// Build a new client. `base_url` should NOT have a trailing slash
    /// (e.g. `https://api.beebeeb.io`). The 30-second timeout is the
    /// upper bound for any single chunk request — bigger files arrive
    /// chunk-at-a-time so a single slow chunk doesn't kill a large
    /// download.
    pub fn new(base_url: String, token: String, master_key: [u8; 32]) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            token,
            master_key,
            client: Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .expect("reqwest client"),
        }
    }

    /// Helper for tests + downloaders: full URL of a single chunk.
    pub fn chunk_url(&self, file_id: &str, chunk_idx: u32) -> String {
        format!(
            "{}/api/v1/files/{}/chunks/{}",
            self.base_url,
            encode(file_id),
            chunk_idx
        )
    }

    pub fn upload_init_url(&self) -> String {
        self.url("/api/v1/uploads/init")
    }

    pub fn upload_chunk_url(&self, upload_session_id: &str, chunk_idx: u32) -> String {
        self.url(&format!(
            "/api/v1/uploads/{}/chunks/{}",
            encode(upload_session_id),
            chunk_idx
        ))
    }

    pub fn upload_complete_url(&self, upload_session_id: &str) -> String {
        self.url(&format!("/api/v1/uploads/{}/complete", encode(upload_session_id)))
    }

    pub fn file_url(&self, file_id: &str) -> String {
        self.url(&format!("/api/v1/files/{}", encode(file_id)))
    }

    pub fn restore_file_url(&self, file_id: &str) -> String {
        self.url(&format!("/api/v1/files/{}/restore", encode(file_id)))
    }

    pub fn versions_url(&self, file_id: &str) -> String {
        self.url(&format!("/api/v1/files/{}/versions", encode(file_id)))
    }

    pub fn version_download_url(&self, file_id: &str, version_id: &str) -> String {
        self.url(&format!(
            "/api/v1/files/{}/versions/{}/download",
            encode(file_id),
            encode(version_id)
        ))
    }

    pub fn version_restore_url(&self, file_id: &str, version_id: &str) -> String {
        self.url(&format!(
            "/api/v1/files/{}/versions/{}/restore",
            encode(file_id),
            encode(version_id)
        ))
    }

    pub fn folder_members_url(&self, folder_id: &str) -> String {
        self.url(&format!("/api/v1/shares/invites/folder-members/{}", encode(folder_id)))
    }

    pub fn shares_incoming_url(&self) -> String {
        self.url("/api/v1/shares/invites/incoming")
    }

    pub fn list_files_url(&self, scope: ListFilesScope<'_>) -> String {
        let mut query = Vec::new();
        if let Some(parent_id) = scope.parent_id {
            query.push(format!("parent_id={}", encode(parent_id)));
        }
        if let Some(shared_folder_id) = scope.shared_folder_id {
            query.push(format!("shared_folder_id={}", encode(shared_folder_id)));
        }
        if let Some(trashed) = scope.trashed {
            query.push(format!("trashed={trashed}"));
        }
        if query.is_empty() {
            self.url("/api/v1/files")
        } else {
            self.url(&format!("/api/v1/files?{}", query.join("&")))
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    /// `GET /api/v1/files[?parent_id=...]` — returns the children of a
    /// folder (or the root if `parent_id` is `None`). Each entry is a
    /// raw JSON object; the daemon decodes the encrypted name with
    /// `master_key` locally.
    pub async fn list_files(&self, parent_id: Option<&str>) -> anyhow::Result<Vec<serde_json::Value>> {
        let url = match parent_id {
            Some(id) => self.list_files_url(ListFilesScope {
                parent_id: Some(id),
                shared_folder_id: None,
                trashed: None,
            }),
            None => self.list_files_url(ListFilesScope::default()),
        };
        let resp = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .send()
            .await?
            .error_for_status()?;
        let body: serde_json::Value = resp.json().await?;
        Ok(body["files"].as_array().cloned().unwrap_or_default())
    }

    /// `GET /api/v1/files/:id` — single-file metadata. Used by
    /// `EngineBridge::hydrate_file` to learn how many chunks to fetch
    /// and to recover the encrypted name when the local mirror entry
    /// is sparse (e.g. fresh placeholder).
    pub async fn get_file(&self, file_id: &str) -> anyhow::Result<serde_json::Value> {
        let resp = self
            .client
            .get(self.file_url(file_id))
            .header("Authorization", format!("Bearer {}", self.token))
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }

    /// `GET /api/v1/files/:id/chunks/:idx` — returns the raw encrypted
    /// chunk bytes. Decryption is the caller's job (`beebeeb_core`).
    pub async fn download_chunk(&self, file_id: &str, chunk_idx: u32) -> anyhow::Result<Vec<u8>> {
        let resp = self
            .client
            .get(&self.chunk_url(file_id, chunk_idx))
            .header("Authorization", format!("Bearer {}", self.token))
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.bytes().await?.to_vec())
    }

    /// `PUT /api/v1/files/:id/chunks/:idx` — uploads encrypted chunk
    /// bytes. The body is `application/octet-stream`; the encryption
    /// envelope is whatever the caller wrote (matches the CLI's
    /// `EncryptedBlob` JSON layout for parity).
    pub async fn upload_chunk(&self, file_id: &str, chunk_idx: u32, data: &[u8]) -> anyhow::Result<()> {
        self.client
            .put(&self.chunk_url(file_id, chunk_idx))
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Content-Type", "application/octet-stream")
            .body(data.to_vec())
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }

    pub async fn init_upload(&self, body: &DesktopUploadInitRequest) -> anyhow::Result<DesktopUploadInitResponse> {
        let resp = self
            .client
            .post(self.upload_init_url())
            .header("Authorization", format!("Bearer {}", self.token))
            .json(body)
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }

    pub async fn upload_session_chunk(
        &self,
        upload_session_id: &str,
        chunk_idx: u32,
        data: &[u8],
    ) -> anyhow::Result<UploadChunkResponse> {
        let resp = self
            .client
            .put(self.upload_chunk_url(upload_session_id, chunk_idx))
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Content-Type", "application/octet-stream")
            .body(data.to_vec())
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }

    pub async fn complete_upload_session(&self, upload_session_id: &str) -> anyhow::Result<serde_json::Value> {
        let resp = self
            .client
            .post(self.upload_complete_url(upload_session_id))
            .header("Authorization", format!("Bearer {}", self.token))
            .json(&serde_json::json!({}))
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }

    pub async fn list_shared_folder_files(
        &self,
        shared_folder_id: &str,
        parent_id: Option<&str>,
    ) -> anyhow::Result<Vec<serde_json::Value>> {
        let resp = self
            .client
            .get(self.list_files_url(ListFilesScope {
                parent_id,
                shared_folder_id: Some(shared_folder_id),
                trashed: None,
            }))
            .header("Authorization", format!("Bearer {}", self.token))
            .send()
            .await?
            .error_for_status()?;
        let body: serde_json::Value = resp.json().await?;
        Ok(body["files"].as_array().cloned().unwrap_or_default())
    }

    pub async fn list_versions(&self, file_id: &str) -> anyhow::Result<serde_json::Value> {
        let resp = self
            .client
            .get(self.versions_url(file_id))
            .header("Authorization", format!("Bearer {}", self.token))
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }

    pub async fn download_version(&self, file_id: &str, version_id: &str) -> anyhow::Result<Vec<u8>> {
        let resp = self
            .client
            .get(self.version_download_url(file_id, version_id))
            .header("Authorization", format!("Bearer {}", self.token))
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.bytes().await?.to_vec())
    }

    pub async fn restore_version(&self, file_id: &str, version_id: &str) -> anyhow::Result<serde_json::Value> {
        let resp = self
            .client
            .post(self.version_restore_url(file_id, version_id))
            .header("Authorization", format!("Bearer {}", self.token))
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }

    pub async fn trash_file(&self, file_id: &str) -> anyhow::Result<serde_json::Value> {
        let resp = self
            .client
            .delete(self.file_url(file_id))
            .header("Authorization", format!("Bearer {}", self.token))
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }

    pub async fn restore_file(&self, file_id: &str) -> anyhow::Result<serde_json::Value> {
        let resp = self
            .client
            .post(self.restore_file_url(file_id))
            .header("Authorization", format!("Bearer {}", self.token))
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }

    pub async fn update_metadata(
        &self,
        file_id: &str,
        name_encrypted: Option<&str>,
        parent_id: Option<&str>,
    ) -> anyhow::Result<serde_json::Value> {
        let mut body = serde_json::Map::new();
        if let Some(name) = name_encrypted {
            body.insert("name_encrypted".into(), serde_json::json!(name));
        }
        if let Some(parent_id) = parent_id {
            body.insert("parent_id".into(), serde_json::json!(parent_id));
        }
        let resp = self
            .client
            .patch(self.file_url(file_id))
            .header("Authorization", format!("Bearer {}", self.token))
            .json(&body)
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }

    pub async fn create_folder(
        &self,
        name_encrypted: &str,
        parent_id: Option<&str>,
        folder_id: Option<&str>,
    ) -> anyhow::Result<serde_json::Value> {
        let mut body = serde_json::json!({ "name_encrypted": name_encrypted });
        if let Some(parent_id) = parent_id {
            body["parent_id"] = serde_json::json!(parent_id);
        }
        if let Some(folder_id) = folder_id {
            body["folder_id"] = serde_json::json!(folder_id);
        }
        let resp = self
            .client
            .post(self.url("/api/v1/files/folder"))
            .header("Authorization", format!("Bearer {}", self.token))
            .json(&body)
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }

    pub async fn folder_members(&self, folder_id: &str) -> anyhow::Result<serde_json::Value> {
        let resp = self
            .client
            .get(self.folder_members_url(folder_id))
            .header("Authorization", format!("Bearer {}", self.token))
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }

    pub async fn list_shared_roots(&self) -> anyhow::Result<serde_json::Value> {
        let resp = self
            .client
            .get(self.shares_incoming_url())
            .header("Authorization", format!("Bearer {}", self.token))
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
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
    async fn get_typed<T: serde::de::DeserializeOwned>(&self, path: &str) -> anyhow::Result<T> {
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
    async fn send_with_retry(
        &self,
        build: impl Fn() -> reqwest::RequestBuilder,
    ) -> anyhow::Result<reqwest::Response> {
        let mut spent: u64 = 0;
        let mut attempt: u32 = 0;
        loop {
            let resp = build()
                .header("Authorization", format!("Bearer {}", self.token))
                .send()
                .await?;
            if resp.status() != reqwest::StatusCode::TOO_MANY_REQUESTS {
                return Self::check_status(resp).await;
            }
            // 429: decide whether (and how long) to wait before retrying.
            let retry_after = header_secs(resp.headers(), reqwest::header::RETRY_AFTER.as_str());
            let reset_in = header_secs(resp.headers(), "x-ratelimit-reset")
                .map(|epoch| epoch.saturating_sub(now_unix_secs()));
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
    async fn check_status(resp: reqwest::Response) -> anyhow::Result<reqwest::Response> {
        let status = resp.status();
        if status.is_success() {
            return Ok(resp);
        }
        let body = resp.text().await.unwrap_or_default();
        let snippet: String = body.chars().take(300).collect();
        anyhow::bail!("HTTP {status}: {snippet}")
    }

    /// `GET /api/v1/auth/me` — account profile.
    pub async fn account_profile(&self) -> anyhow::Result<crate::account_dto::AccountProfile> {
        self.get_typed("/api/v1/auth/me").await
    }

    /// `GET /api/v1/billing/subscription` — plan + quota + lifecycle.
    pub async fn subscription(&self) -> anyhow::Result<crate::account_dto::Subscription> {
        self.get_typed("/api/v1/billing/subscription").await
    }

    /// `GET /api/v1/billing/usage` — used/quota bytes + percentage.
    pub async fn billing_usage(&self) -> anyhow::Result<crate::account_dto::BillingUsage> {
        self.get_typed("/api/v1/billing/usage").await
    }

    /// `GET /api/v1/me/region` — the user's preferred region + available list.
    /// Mirrors the webapp `getUserRegion()` contract so the desktop resolves
    /// the effective region's CITY the same way (never the provider).
    pub async fn account_region(&self) -> anyhow::Result<crate::account_dto::UserRegionResponse> {
        self.get_typed("/api/v1/me/region").await
    }

    /// `GET /api/v1/account/activity` — events + summary.
    pub async fn account_activity(&self) -> anyhow::Result<crate::account_dto::AccountActivity> {
        self.get_typed("/api/v1/account/activity").await
    }

    /// `GET /api/v1/account/security-score` — numeric score + factors.
    pub async fn security_score(&self) -> anyhow::Result<crate::account_dto::SecurityScore> {
        self.get_typed("/api/v1/account/security-score").await
    }

    /// `GET /api/v1/account/sessions` — active account sessions.
    pub async fn account_sessions(&self) -> anyhow::Result<crate::account_dto::AccountSessionList> {
        self.get_typed("/api/v1/account/sessions").await
    }

    /// `DELETE /api/v1/account/sessions/{id}` — revoke one session.
    /// Server returns `{ revoked: true }`; we don't need the body.
    pub async fn revoke_account_session(&self, session_id: &str) -> anyhow::Result<()> {
        let url = self.url(&format!("/api/v1/account/sessions/{}", encode(session_id)));
        self.send_with_retry(|| self.client.delete(&url)).await?;
        Ok(())
    }

    /// `POST /api/v1/account/sessions/revoke-all-others` — revoke every
    /// session except the current one. Returns the count revoked.
    pub async fn revoke_other_sessions(&self) -> anyhow::Result<crate::account_dto::RevokeAllResult> {
        let url = self.url("/api/v1/account/sessions/revoke-all-others");
        let resp = self.send_with_retry(|| self.client.post(&url)).await?;
        Ok(resp.json().await?)
    }

    /// `GET /api/v1/clients/devices` — registered client devices.
    pub async fn client_devices(&self) -> anyhow::Result<crate::account_dto::ClientDeviceList> {
        self.get_typed("/api/v1/clients/devices").await
    }

    /// `GET /api/v1/clients/sessions` — per-device sync sessions.
    pub async fn client_sync_sessions(
        &self,
    ) -> anyhow::Result<crate::account_dto::ClientSyncSessionList> {
        self.get_typed("/api/v1/clients/sessions").await
    }

    // ── Telemetry producers (register device, open session, post heartbeat) ────

    /// `POST /api/v1/clients/devices` — register (UPSERT) THIS device.
    ///
    /// The server upserts on `(user_id, hostname, platform)`, so calling this
    /// repeatedly on every login/relaunch is safe and just refreshes
    /// `last_seen` + `bb_version`. Goes through `send_with_retry` so a transient
    /// 429 during a busy login doesn't drop the registration. Returns the device
    /// row (the caller needs `id` to open a sync session).
    pub async fn register_device(
        &self,
        hostname: &str,
        platform: &str,
        bb_version: &str,
    ) -> anyhow::Result<RegisteredDevice> {
        let url = self.url("/api/v1/clients/devices");
        let body = serde_json::json!({
            "hostname": hostname,
            "platform": platform,
            "bb_version": bb_version,
        });
        let resp = self
            .send_with_retry(|| self.client.post(&url).json(&body))
            .await?;
        Ok(resp.json().await?)
    }

    /// `POST /api/v1/clients/sessions` — open ONE sync session for `device_id`.
    ///
    /// Unlike device registration this is NOT idempotent server-side (each call
    /// inserts a new `client_sessions` row), so the runner calls it exactly once
    /// per engine spawn. `session_type` must be one of the server's accepted set
    /// (`sync`/`backup`/`mount`/`webdav`); the desktop uses `"sync"`.
    pub async fn create_session(
        &self,
        device_id: &str,
        name: &str,
        session_type: &str,
        local_path: Option<&str>,
        remote_path: &str,
        heartbeat_interval_secs: i32,
    ) -> anyhow::Result<CreatedSession> {
        let url = self.url("/api/v1/clients/sessions");
        let body = serde_json::json!({
            "device_id": device_id,
            "name": name,
            "session_type": session_type,
            "local_path": local_path,
            "remote_path": remote_path,
            "heartbeat_interval_secs": heartbeat_interval_secs,
        });
        let resp = self
            .send_with_retry(|| self.client.post(&url).json(&body))
            .await?;
        Ok(resp.json().await?)
    }

    /// `POST /api/v1/clients/sessions/{id}/heartbeat` — one telemetry beat.
    ///
    /// Fire-and-forget: the heartbeat is the lowest-stakes call in the engine
    /// (a missed beat just means one stale row until the next beat), so it does
    /// NOT retry on 429 — retrying would only pile the next beat on top of a
    /// rate-limited one. A non-2xx is surfaced as an error the caller logs and
    /// swallows; the producer keeps ticking.
    pub async fn post_heartbeat(&self, session_id: &str, body: &HeartbeatBody) -> anyhow::Result<()> {
        let url = self.url(&format!(
            "/api/v1/clients/sessions/{}/heartbeat",
            encode(session_id)
        ));
        let resp = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .json(body)
            .send()
            .await?;
        Self::check_status(resp).await?;
        Ok(())
    }

    /// `GET /api/v1/activity` — paginated audit-log feed. `page`/`limit`
    /// are optional; when `None` the server defaults apply.
    pub async fn activity_feed(
        &self,
        page: Option<u32>,
        limit: Option<u32>,
    ) -> anyhow::Result<crate::account_dto::ActivityFeed> {
        let mut query = Vec::new();
        if let Some(page) = page {
            query.push(format!("page={page}"));
        }
        if let Some(limit) = limit {
            query.push(format!("limit={limit}"));
        }
        let path = if query.is_empty() {
            "/api/v1/activity".to_string()
        } else {
            format!("/api/v1/activity?{}", query.join("&"))
        };
        self.get_typed(&path).await
    }

    /// `GET /api/v1/notifications` — notification list + unread count.
    pub async fn notifications(&self) -> anyhow::Result<crate::account_dto::NotificationList> {
        self.get_typed("/api/v1/notifications").await
    }

    /// `GET /api/v1/notifications/preferences` — push-preference toggles.
    pub async fn notification_preferences(
        &self,
    ) -> anyhow::Result<crate::account_dto::NotificationPreferences> {
        self.get_typed("/api/v1/notifications/preferences").await
    }

    /// `PUT /api/v1/notifications/preferences` — partial update; omitted
    /// fields keep their current server value.
    pub async fn update_notification_preferences(
        &self,
        update: &crate::account_dto::NotificationPreferencesUpdate,
    ) -> anyhow::Result<crate::account_dto::NotificationPreferences> {
        let url = self.url("/api/v1/notifications/preferences");
        let resp = self
            .send_with_retry(|| self.client.put(&url).json(update))
            .await?;
        Ok(resp.json().await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chunk_url() {
        let client = ApiClient::new("https://api.beebeeb.io".into(), "tok".into(), [0u8; 32]);
        assert_eq!(
            client.chunk_url("file123", 0),
            "https://api.beebeeb.io/api/v1/files/file123/chunks/0"
        );
    }

    #[test]
    fn test_desktop_contract_urls_trim_base_and_encode_queries() {
        let client = ApiClient::new("https://api.beebeeb.io/".into(), "tok".into(), [0u8; 32]);
        assert_eq!(client.upload_init_url(), "https://api.beebeeb.io/api/v1/uploads/init");
        assert_eq!(
            client.upload_chunk_url("session/with space", 2),
            "https://api.beebeeb.io/api/v1/uploads/session%2Fwith%20space/chunks/2"
        );
        assert_eq!(
            client.list_files_url(ListFilesScope {
                parent_id: Some("parent/one"),
                shared_folder_id: Some("shared one"),
                trashed: Some(false),
            }),
            "https://api.beebeeb.io/api/v1/files?parent_id=parent%2Fone&shared_folder_id=shared%20one&trashed=false"
        );
        assert_eq!(
            client.version_download_url("file/one", "version two"),
            "https://api.beebeeb.io/api/v1/files/file%2Fone/versions/version%20two/download"
        );
    }

    #[test]
    fn heartbeat_body_minimal_serializes_status_only() {
        // A status-only beat (no counts/bytes yet) must put `status` on the
        // wire and OMIT every optional field — the server's HeartbeatBody reads
        // them as `Option`, so absence is the same as null but smaller.
        let body = HeartbeatBody {
            status: "watching".into(),
            ..Default::default()
        };
        let json = serde_json::to_string(&body).unwrap();
        assert_eq!(json, r#"{"status":"watching"}"#);
    }

    #[test]
    fn heartbeat_body_full_serializes_all_fields() {
        let body = HeartbeatBody {
            status: "syncing".into(),
            files_synced: Some(10),
            files_total: Some(12),
            bytes_synced: Some(1000),
            bytes_total: Some(2000),
            current_file: Some("report.pdf".into()),
            speed_bps: Some(500_000),
            detail: Some("uploading".into()),
        };
        let v: serde_json::Value = serde_json::to_value(&body).unwrap();
        assert_eq!(v["status"], "syncing");
        assert_eq!(v["files_synced"], 10);
        assert_eq!(v["files_total"], 12);
        assert_eq!(v["bytes_synced"], 1000);
        assert_eq!(v["bytes_total"], 2000);
        assert_eq!(v["current_file"], "report.pdf");
        assert_eq!(v["speed_bps"], 500_000);
        assert_eq!(v["detail"], "uploading");
    }

    #[test]
    fn heartbeat_body_skips_none_optionals_but_keeps_zero() {
        // A zero count is a meaningful value (0 files synced) and MUST be sent;
        // only `None` is skipped. Guards against a `skip_serializing_if` that
        // accidentally drops legitimate zeros.
        let body = HeartbeatBody {
            status: "idle".into(),
            files_synced: Some(0),
            bytes_synced: Some(0),
            ..Default::default()
        };
        let v: serde_json::Value = serde_json::to_value(&body).unwrap();
        assert_eq!(v["files_synced"], 0);
        assert_eq!(v["bytes_synced"], 0);
        assert!(v.get("files_total").is_none());
        assert!(v.get("current_file").is_none());
        assert!(v.get("speed_bps").is_none());
    }

    #[test]
    fn registered_device_parses_and_tolerates_extra_fields() {
        let json = r#"{
            "id": "11111111-1111-1111-1111-111111111111",
            "hostname": "guus-pc",
            "platform": "windows",
            "bb_version": "0.1.0",
            "last_seen": "2026-06-15T00:00:00Z",
            "created_at": "2026-06-01T00:00:00Z"
        }"#;
        let d: RegisteredDevice = serde_json::from_str(json).unwrap();
        assert_eq!(d.id, "11111111-1111-1111-1111-111111111111");
        assert_eq!(d.platform.as_deref(), Some("windows"));
    }

    #[test]
    fn created_session_parses_minimal() {
        let json = r#"{ "id": "cs1", "heartbeat_interval_secs": 20 }"#;
        let s: CreatedSession = serde_json::from_str(json).unwrap();
        assert_eq!(s.id, "cs1");
        assert_eq!(s.heartbeat_interval_secs, Some(20));
    }

    // ── 429 backoff/retry decision (pure helper) ───────────────────────────────

    #[test]
    fn retry_decision_first_429_uses_default_when_no_hint() {
        // First 429 (attempt 0), nothing spent, no server hint → default wait.
        assert_eq!(retry_decision(0, 0, None, None), Some(RETRY_DEFAULT_WAIT_SECS));
    }

    #[test]
    fn retry_decision_honours_retry_after_capped() {
        // A small Retry-After is used verbatim …
        assert_eq!(retry_decision(0, 0, Some(2), None), Some(2));
        // … but an absurd one is clamped to the per-attempt cap.
        assert_eq!(retry_decision(0, 0, Some(120), None), Some(RETRY_WAIT_CAP_SECS));
    }

    #[test]
    fn retry_decision_uses_reset_when_retry_after_absent() {
        // `x-ratelimit-reset` delta (seconds-until-reset) drives the wait.
        assert_eq!(retry_decision(0, 0, None, Some(3)), Some(3));
    }

    #[test]
    fn retry_decision_takes_larger_of_two_hints() {
        // When BOTH hints are present we wait the longer of the two.
        assert_eq!(retry_decision(0, 0, Some(2), Some(4)), Some(4));
        assert_eq!(retry_decision(0, 0, Some(4), Some(2)), Some(4));
    }

    #[test]
    fn retry_decision_stops_after_max_attempts() {
        // Once we've already retried MAX_429_RETRIES times, give up.
        assert_eq!(retry_decision(MAX_429_RETRIES, 0, Some(1), None), None);
    }

    #[test]
    fn retry_decision_stops_when_budget_exhausted() {
        // Spent at/over the total budget → no more sleeping.
        assert_eq!(retry_decision(1, RETRY_TOTAL_BUDGET_SECS, Some(1), None), None);
    }

    #[test]
    fn retry_decision_trims_wait_to_remaining_budget() {
        // Cap says 5s, but only 2s of budget remain → wait is trimmed to 2s so
        // cumulative sleep never exceeds RETRY_TOTAL_BUDGET_SECS.
        let spent = RETRY_TOTAL_BUDGET_SECS - 2;
        assert_eq!(retry_decision(1, spent, Some(5), None), Some(2));
    }

    #[test]
    fn header_secs_parses_numeric_and_rejects_dates() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("retry-after", "  7 ".parse().unwrap());
        headers.insert("x-ratelimit-reset", "1718000000".parse().unwrap());
        headers.insert("x-http-date", "Wed, 21 Oct 2026 07:28:00 GMT".parse().unwrap());
        assert_eq!(header_secs(&headers, "retry-after"), Some(7));
        assert_eq!(header_secs(&headers, "x-ratelimit-reset"), Some(1718000000));
        // Non-numeric (HTTP-date) header → None (default wait covers it).
        assert_eq!(header_secs(&headers, "x-http-date"), None);
        // Absent header → None.
        assert_eq!(header_secs(&headers, "nope"), None);
    }

    #[test]
    fn test_metadata_operation_urls() {
        let client = ApiClient::new("https://api.beebeeb.io".into(), "tok".into(), [0u8; 32]);
        assert_eq!(
            client.file_url("file one"),
            "https://api.beebeeb.io/api/v1/files/file%20one"
        );
        assert_eq!(
            client.restore_file_url("file one"),
            "https://api.beebeeb.io/api/v1/files/file%20one/restore"
        );
        assert_eq!(
            client.folder_members_url("folder/one"),
            "https://api.beebeeb.io/api/v1/shares/invites/folder-members/folder%2Fone"
        );
        assert_eq!(
            client.shares_incoming_url(),
            "https://api.beebeeb.io/api/v1/shares/invites/incoming"
        );
    }
}
