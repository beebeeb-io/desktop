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
    pub file_name: String,
    pub file_size_bytes: i64,
    pub mime_type: Option<String>,
    pub parent_id: Option<String>,
    pub profile: String,
    pub is_media: bool,
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

    pub fn shares_mine_url(&self) -> String {
        self.url("/api/v1/shares/mine")
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
            .get(self.shares_mine_url())
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
        assert_eq!(client.shares_mine_url(), "https://api.beebeeb.io/api/v1/shares/mine");
    }
}
