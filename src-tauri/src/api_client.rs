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
use std::time::Duration;

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
            base_url,
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
            self.base_url, file_id, chunk_idx
        )
    }

    /// `GET /api/v1/files[?parent_id=...]` — returns the children of a
    /// folder (or the root if `parent_id` is `None`). Each entry is a
    /// raw JSON object; the daemon decodes the encrypted name with
    /// `master_key` locally.
    pub async fn list_files(
        &self,
        parent_id: Option<&str>,
    ) -> anyhow::Result<Vec<serde_json::Value>> {
        let url = match parent_id {
            Some(id) => format!("{}/api/v1/files?parent_id={}", self.base_url, id),
            None => format!("{}/api/v1/files", self.base_url),
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
        let url = format!("{}/api/v1/files/{}", self.base_url, file_id);
        let resp = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }

    /// `GET /api/v1/files/:id/chunks/:idx` — returns the raw encrypted
    /// chunk bytes. Decryption is the caller's job (`beebeeb_core`).
    pub async fn download_chunk(
        &self,
        file_id: &str,
        chunk_idx: u32,
    ) -> anyhow::Result<Vec<u8>> {
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
    pub async fn upload_chunk(
        &self,
        file_id: &str,
        chunk_idx: u32,
        data: &[u8],
    ) -> anyhow::Result<()> {
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
        let client = ApiClient::new(
            "https://api.beebeeb.io".into(),
            "tok".into(),
            [0u8; 32],
        );
        assert_eq!(
            client.chunk_url("file123", 0),
            "https://api.beebeeb.io/api/v1/files/file123/chunks/0"
        );
    }
}
