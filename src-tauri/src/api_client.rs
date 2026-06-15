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
    // `get_typed` / `send_typed`, which:
    //   * retry once on HTTP 429 (rate limit) after honouring `Retry-After`,
    //     mirroring the web `request()` client's backoff (capped, single
    //     retry — the daemon is not a hot loop),
    //   * surface a clean `<status>: <body-snippet>` error on any other
    //     non-2xx instead of reqwest's opaque default.

    /// Internal: GET a JSON endpoint and deserialize it, with a single 429
    /// backoff. `path` is the API path (leading slash); auth is attached.
    async fn get_typed<T: serde::de::DeserializeOwned>(&self, path: &str) -> anyhow::Result<T> {
        let url = self.url(path);
        let resp = self.send_with_retry(|| self.client.get(&url)).await?;
        Ok(resp.json::<T>().await?)
    }

    /// Internal: issue an arbitrary request (built by `build`) with a single
    /// 429 backoff, returning a 2xx response or a descriptive error.
    async fn send_with_retry(
        &self,
        build: impl Fn() -> reqwest::RequestBuilder,
    ) -> anyhow::Result<reqwest::Response> {
        // First attempt.
        let resp = build()
            .header("Authorization", format!("Bearer {}", self.token))
            .send()
            .await?;
        if resp.status() != reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Self::check_status(resp).await;
        }
        // 429 — honour Retry-After (seconds), capped at 10s, then retry once.
        let wait = resp
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.trim().parse::<u64>().ok())
            .unwrap_or(1)
            .min(10);
        tokio::time::sleep(Duration::from_secs(wait)).await;
        let resp = build()
            .header("Authorization", format!("Bearer {}", self.token))
            .send()
            .await?;
        Self::check_status(resp).await
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
