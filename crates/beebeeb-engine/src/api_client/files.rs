//! The file half of the API: listings, the sync read endpoints, metadata and
//! the file lifecycle (versions, trash, restore, permanent delete), plus the
//! per-file blob reads.
//!
//! Owns: the requests that name a file id, and the paging cap that turns a bad
//! cursor loop into an error instead of unbounded memory. Must not: build a URL
//! (see `client`), define a wire type (see `types`), or decide what a refusal
//! means (see `error`). Tests: `api_client/tests/`.

use super::*;

impl ApiClient {
    /// `GET /api/v1/files[?parent_id=...]` — returns the children of a
    /// folder (or the root if `parent_id` is `None`). Each entry is a
    /// raw JSON object; the daemon decodes the encrypted name with
    /// `master_key` locally.
    pub async fn list_files(&self, parent_id: Option<&str>) -> anyhow::Result<Vec<serde_json::Value>> {
        let url = match parent_id {
            Some(id) => self.list_files_url(ListFilesScope {
                parent_id: Some(FolderId::new(id)),
                shared_folder_id: None,
                trashed: None,
                cursor: None,
            }),
            None => self.list_files_url(ListFilesScope::default()),
        };
        // Route through `send_with_retry` so a transient 429 backs off and
        // retries (honouring Retry-After / x-ratelimit-reset) instead of
        // erroring immediately — the hot-path full-tree re-walk would
        // otherwise self-saturate the per-IP rate limit. The closure rebuilds
        // the full request (with the same query params baked into `url`) each
        // attempt so retries re-send identically; the auth header is added
        // inside `send_with_retry`, so it must NOT be added here.
        let resp = self.send_with_retry(|| self.client.get(&url)).await?;
        let body: serde_json::Value = resp.json().await?;
        Ok(body["files"].as_array().cloned().unwrap_or_default())
    }

    /// `GET /api/v1/sync/snapshot` — the ENTIRE non-trashed tree + the latest
    /// op `seq_id`, in ONE request. The bootstrap for the delta engine (task
    /// 0789): the runner calls this once when the local cursor is unset/0, then
    /// switches to [`Self::sync_ops`] for incremental catch-up. Routes through
    /// `get_typed`, so a transient 429 backs off + retries (honouring
    /// Retry-After / x-ratelimit-reset) instead of erroring immediately.
    pub async fn sync_snapshot(&self) -> anyhow::Result<crate::api_client::SyncSnapshot> {
        Ok(self.get_typed("/api/v1/sync/snapshot").await?)
    }

    /// `GET /api/v1/sync/ops?since={seq_id}` — every op with `seq_id > since`,
    /// oldest-first. The steady-state delta call: ONE request/tick replaces the
    /// old `1 + folders` full-tree re-walk. Same 429 backoff as `sync_snapshot`.
    pub async fn sync_ops(&self, since: i64) -> anyhow::Result<crate::api_client::SyncOps> {
        Ok(self.get_typed(&format!("/api/v1/sync/ops?since={since}")).await?)
    }

    /// `GET /api/v1/files/:id` — single-file metadata. Used by
    /// `EngineBridge::hydrate_file` to learn how many chunks to fetch
    /// and to recover the encrypted name when the local mirror entry
    /// is sparse (e.g. fresh placeholder).
    pub async fn get_file(&self, file_id: &str) -> anyhow::Result<serde_json::Value> {
        let resp = self
            .client
            .get(self.file_url(FileId::new(file_id)))
            .header("Authorization", format!("Bearer {}", self.token))
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }

    /// `GET /api/v1/files/:id/chunks/:idx` — returns the raw encrypted
    /// chunk bytes. Decryption is the caller's job (`beebeeb_core`). Each piece of the body is
    /// reported to `on_body` as it arrives (LD-0105 S0), not once the chunk is whole.
    pub async fn download_chunk(
        &self,
        file_id: &str,
        chunk_idx: u32,
        on_body: super::OnBody<'_>,
    ) -> anyhow::Result<Vec<u8>> {
        self.download_chunk_within(file_id, chunk_idx, None, on_body).await
    }

    /// [`Self::download_chunk`], holding at most `max_len` bytes of the body: a body that passes it is given up
    /// while it streams ([`super::ChunkOverLimit`]; what arrived is reported as discarded), never read whole
    /// first. For a caller that holds the file in memory and has no manifest to bound a chunk by (a file whose
    /// metadata names no version).
    pub async fn download_chunk_capped(
        &self,
        file_id: &str,
        chunk_idx: u32,
        max_len: u64,
        on_body: super::OnBody<'_>,
    ) -> anyhow::Result<Vec<u8>> {
        self.download_chunk_within(file_id, chunk_idx, Some(max_len), on_body)
            .await
    }

    async fn download_chunk_within(
        &self,
        file_id: &str,
        chunk_idx: u32,
        max_len: Option<u64>,
        on_body: super::OnBody<'_>,
    ) -> anyhow::Result<Vec<u8>> {
        let mut resp = self
            .client
            .get(&self.chunk_url(FileId::new(file_id), chunk_idx))
            .header("Authorization", format!("Bearer {}", self.token))
            .send()
            .await?
            .error_for_status()?;
        // Sized from the length the server states, capped: a wrong header costs a reallocation, never
        // a large allocation up front.
        const RESERVE_AT_MOST: u64 = 64 << 20;
        let mut body = Vec::with_capacity(resp.content_length().unwrap_or(0).min(RESERVE_AT_MOST) as usize);
        loop {
            match resp.chunk().await {
                Ok(Some(piece)) => {
                    if max_len.is_some_and(|max| body.len() as u64 + piece.len() as u64 > max) {
                        if !body.is_empty() {
                            on_body(super::BodyBytes::Discarded(body.len() as u64));
                        }
                        return Err(anyhow::Error::new(super::ChunkOverLimit));
                    }
                    on_body(super::BodyBytes::Read(piece.len() as u64));
                    body.extend_from_slice(&piece);
                }
                Ok(None) => return Ok(body),
                Err(e) => {
                    // The bytes that arrived kept the fetch alive; the chunk is not used.
                    if !body.is_empty() {
                        on_body(super::BodyBytes::Discarded(body.len() as u64));
                    }
                    return Err(e.into());
                }
            }
        }
    }

    /// `GET /api/v1/files/:id/thumbnail/{variant}` — returns the raw *encrypted*
    /// thumbnail blob for the given variant (`small`/`medium`/`large`).
    ///
    /// The body is the same `nonce(12) || AES-256-GCM(ciphertext+tag)` envelope
    /// the web/mobile/CLI clients uploaded at file-upload time (a poster frame
    /// for video, a downscaled still for images). Decryption with the per-file
    /// key is the caller's job (see `EngineBridge::fetch_thumbnail_to_memory`).
    ///
    /// A `404` (no thumbnail for this file / variant) surfaces as an `Err` via
    /// `error_for_status`; the caller treats any error as "no thumbnail" and lets
    /// Explorer fall back to the file-type icon.
    pub async fn download_thumbnail(&self, file_id: &str, variant: &str) -> anyhow::Result<Vec<u8>> {
        let resp = self
            .client
            .get(self.thumbnail_url(FileId::new(file_id), variant))
            .header("Authorization", format!("Bearer {}", self.token))
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.bytes().await?.to_vec())
    }

    /// Enumerate every trashed row by following the `/files?trashed=true`
    /// `next_cursor`, mirroring the web app's `listAllFiles(..., { trashed:
    /// true })`. The hard cap prevents a bad cursor loop from growing memory
    /// without bound.
    pub async fn list_trashed_files_all(&self) -> anyhow::Result<Vec<serde_json::Value>> {
        const HARD_CAP: usize = 50_000;
        let mut out = Vec::new();
        let mut cursor: Option<String> = None;

        loop {
            let url = self.list_files_url(ListFilesScope {
                parent_id: None,
                shared_folder_id: None,
                trashed: Some(true),
                cursor: cursor.as_deref(),
            });
            let resp = self.send_with_retry(|| self.client.get(&url)).await?;
            let body: serde_json::Value = resp.json().await?;
            let files = body["files"].as_array().cloned().unwrap_or_default();
            out.extend(files);
            if out.len() > HARD_CAP {
                return Err(DesktopApiError::ListingTooLarge {
                    listing: Listing::Trashed,
                    cap: HARD_CAP,
                }
                .into());
            }
            cursor = body["next_cursor"].as_str().map(str::to_string);
            if cursor.is_none() {
                break;
            }
        }

        Ok(out)
    }

    pub async fn list_versions(&self, file_id: &str) -> anyhow::Result<serde_json::Value> {
        let resp = self
            .client
            .get(self.versions_url(FileId::new(file_id)))
            .header("Authorization", format!("Bearer {}", self.token))
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }

    pub async fn restore_version(&self, file_id: &str, version_id: &str) -> anyhow::Result<serde_json::Value> {
        let resp = self
            .client
            .post(self.version_restore_url(FileId::new(file_id), VersionId::new(version_id)))
            .header("Authorization", format!("Bearer {}", self.token))
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }

    /// LD-0113: a 409 `protected_folder` is a [`ProtectedFolderRefused`] (the Backups root).
    pub async fn trash_file(&self, file_id: &str) -> anyhow::Result<serde_json::Value> {
        let resp = self
            .client
            .delete(self.file_url(FileId::new(file_id)))
            .header("Authorization", format!("Bearer {}", self.token))
            .send()
            .await?;
        let resp = protected_folder_response(resp).await?;
        Ok(resp.json().await?)
    }

    pub async fn restore_file(&self, file_id: &str) -> anyhow::Result<serde_json::Value> {
        let resp = self
            .client
            .post(self.restore_file_url(FileId::new(file_id)))
            .header("Authorization", format!("Bearer {}", self.token))
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }

    /// [`ApiClient::restore_file`] without reading the answer: `Ok` is any 2xx (200, or 202 for a group the server
    /// restores in batches), whatever its body says. For a caller that only needs to know the restore was taken.
    pub async fn restore_file_sent(&self, file_id: &str) -> anyhow::Result<()> {
        self.client
            .post(self.restore_file_url(FileId::new(file_id)))
            .header("Authorization", format!("Bearer {}", self.token))
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }

    pub async fn permanent_delete_file(&self, file_id: &str, confirm_token: &str) -> anyhow::Result<()> {
        self.client
            .delete(self.permanent_file_url(FileId::new(file_id)))
            .header("Authorization", format!("Bearer {}", self.token))
            .header("X-Confirm-Token", confirm_token)
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }

    pub async fn update_metadata(
        &self,
        file_id: &str,
        name_encrypted: Option<&str>,
        parent: ParentChange<'_>,
    ) -> anyhow::Result<serde_json::Value> {
        let body = metadata_update_body(name_encrypted, parent);
        let resp = self
            .client
            .patch(self.file_url(FileId::new(file_id)))
            .header("Authorization", format!("Bearer {}", self.token))
            .json(&body)
            .send()
            .await?;
        // LD-0113: a rename or move of the Backups root is a [`ProtectedFolderRefused`].
        let resp = protected_folder_response(resp).await?;
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
}
