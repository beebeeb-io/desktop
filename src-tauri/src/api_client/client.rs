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
    #[cfg(any(target_os = "windows", test))]
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

/// The `PUT /api/v1/me/region` body (task 2050): the chosen id, and `region_strict` only when the
/// caller states it.
pub(super) fn set_region_body(preferred_region: Option<&str>, region_strict: Option<bool>) -> serde_json::Value {
    let mut body = serde_json::json!({ "preferred_region": preferred_region });
    if let Some(strict) = region_strict {
        body["region_strict"] = serde_json::json!(strict);
    }
    body
}

impl ApiClient {
    #[cfg(any(target_os = "windows", test))]
    pub fn credential_lifetime(&self) -> std::sync::Weak<()> {
        std::sync::Arc::downgrade(&self.credential_lifetime)
    }

    /// Build a new client. `base_url` should NOT have a trailing slash
    /// (e.g. `https://api.beebeeb.io`). The 30-second timeout is the
    /// upper bound for any single chunk request — bigger files arrive
    /// chunk-at-a-time so a single slow chunk doesn't kill a large
    /// download.
    pub fn new(base_url: String, token: String, master_key: [u8; 32]) -> Self {
        Self {
            #[cfg(any(target_os = "windows", test))]
            credential_lifetime: std::sync::Arc::new(()),
            base_url: base_url.trim_end_matches('/').to_string(),
            token,
            master_key,
            client: Client::builder()
                .timeout(Duration::from_secs(crate::link_health::API_REQUEST_TIMEOUT_SECS))
                .default_headers(provenance_headers())
                .build()
                .expect("reqwest client"),
            link: std::sync::Arc::new(crate::link_health::LinkMonitor::new()),
            direct_discovery: tokio::sync::Mutex::new(beebeeb_core::direct_transfer::DiscoveryCache::new()),
            storage_client: std::sync::OnceLock::new(),
        }
    }

    /// This client with `timeout` for each request in place of the app's
    /// [`crate::link_health::API_REQUEST_TIMEOUT_SECS`]: for a listing that may move a whole tree on a slow link and
    /// that nobody waits on (task 2267's ownership proof, self-review F5).
    #[cfg(any(target_os = "windows", test))]
    pub fn with_request_timeout(mut self, timeout: Duration) -> Self {
        self.client = Client::builder()
            .timeout(timeout)
            .default_headers(provenance_headers())
            .build()
            .expect("reqwest client");
        self
    }

    /// The shared record of whether the last requests reached the server.
    pub fn link(&self) -> std::sync::Arc<crate::link_health::LinkMonitor> {
        self.link.clone()
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

    /// Full URL of an encrypted thumbnail variant
    /// (`GET /api/v1/files/:id/thumbnail/{variant}`). `variant` is one of
    /// `small` / `medium` / `large`; the server maps each to a reserved sentinel
    /// chunk index. The returned body is the *encrypted* thumbnail blob in the
    /// raw `nonce(12) || AES-256-GCM(ciphertext+tag)` envelope the web/mobile
    /// clients uploaded — decryption is the caller's job.
    pub fn thumbnail_url(&self, file_id: &str, variant: &str) -> String {
        format!(
            "{}/api/v1/files/{}/thumbnail/{}",
            self.base_url,
            encode(file_id),
            variant
        )
    }

    /// Full URL for uploading an encrypted thumbnail variant. The medium
    /// variant intentionally uses `/thumbnail` (no `/medium` suffix), matching
    /// web/mobile. Other variants use `/thumbnail/{variant}`.
    pub fn thumbnail_upload_url(&self, file_id: &str, variant: &str, blurhash: Option<&str>) -> String {
        let mut url = if variant == "medium" {
            format!("{}/api/v1/files/{}/thumbnail", self.base_url, encode(file_id))
        } else {
            format!(
                "{}/api/v1/files/{}/thumbnail/{}",
                self.base_url,
                encode(file_id),
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

    pub fn upload_abandon_url(&self, file_id: &str) -> String {
        self.url(&format!("/api/v1/files/{}/upload/abandon", encode(file_id)))
    }

    pub fn restore_file_url(&self, file_id: &str) -> String {
        self.url(&format!("/api/v1/files/{}/restore", encode(file_id)))
    }

    pub fn permanent_file_url(&self, file_id: &str) -> String {
        self.url(&format!("/api/v1/files/{}/permanent", encode(file_id)))
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

    pub fn folder_keys_url(&self, invite_id: &str) -> String {
        self.url(&format!("/api/v1/shares/invites/{}/folder-keys", encode(invite_id)))
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

    /// `GET /api/v1/search-index/shards` — web-compatible shard manifest.
    /// Returns `[]` when the server has no shards or omits the optional wrapper.
    pub async fn fetch_search_shard_manifest(&self) -> anyhow::Result<Vec<SearchShardRef>> {
        let url = self.search_shard_manifest_url();
        let resp = self.send_with_retry(|| self.client.get(&url)).await?;
        Ok(resp.json::<SearchShardManifest>().await?.shards)
    }

    /// `GET /api/v1/search-index/shards/{bucket}/{page}` — raw encrypted blob.
    /// A 404 races a concurrent delete and is treated the same way web does:
    /// skip the missing shard and let a later rebuild repair the cache.
    pub async fn get_search_shard(&self, bucket: u32, page: u32) -> anyhow::Result<Option<Vec<u8>>> {
        let url = self.search_shard_url(bucket, page);
        let resp = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .send()
            .await?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let resp = Self::check_status(resp).await?;
        Ok(Some(resp.bytes().await?.to_vec()))
    }

    /// `PUT /api/v1/search-index/shards/{bucket}/{page}` — raw octet-stream
    /// encrypted shard upload. The body is never JSON.
    pub async fn put_search_shard(
        &self,
        shard: &beebeeb_core::search_index::EncryptedShard,
    ) -> anyhow::Result<SearchShardRef> {
        anyhow::ensure!(!shard.blob.is_empty(), "search shard blob must not be empty");
        anyhow::ensure!(
            shard.blob.len() <= MAX_SEARCH_SHARD_BLOB_BYTES,
            "search shard {}/{} blob {}B exceeds cap",
            shard.bucket,
            shard.page,
            shard.blob.len()
        );
        let resp = self
            .client
            .put(self.search_shard_url(shard.bucket, shard.page))
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Content-Type", "application/octet-stream")
            .body(shard.blob.clone())
            .send()
            .await?;
        let resp = Self::check_status(resp).await?;
        Ok(resp.json::<SearchShardRef>().await?)
    }

    /// `DELETE /api/v1/search-index/shards/{bucket}/{page}` — idempotent stale
    /// shard removal. 204 and 404 both mean the coordinate is gone.
    pub async fn delete_search_shard(&self, bucket: u32, page: u32) -> anyhow::Result<()> {
        let resp = self
            .client
            .delete(self.search_shard_url(bucket, page))
            .header("Authorization", format!("Bearer {}", self.token))
            .send()
            .await?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(());
        }
        Self::check_status(resp).await?;
        Ok(())
    }

    /// `GET /api/v1/sync/snapshot` — the ENTIRE non-trashed tree + the latest
    /// op `seq_id`, in ONE request. The bootstrap for the delta engine (task
    /// 0789): the runner calls this once when the local cursor is unset/0, then
    /// switches to [`Self::sync_ops`] for incremental catch-up. Routes through
    /// `get_typed`, so a transient 429 backs off + retries (honouring
    /// Retry-After / x-ratelimit-reset) instead of erroring immediately.
    pub async fn sync_snapshot(&self) -> anyhow::Result<crate::api_client::SyncSnapshot> {
        self.get_typed("/api/v1/sync/snapshot").await
    }

    /// `GET /api/v1/sync/ops?since={seq_id}` — every op with `seq_id > since`,
    /// oldest-first. The steady-state delta call: ONE request/tick replaces the
    /// old `1 + folders` full-tree re-walk. Same 429 backoff as `sync_snapshot`.
    pub async fn sync_ops(&self, since: i64) -> anyhow::Result<crate::api_client::SyncOps> {
        self.get_typed(&format!("/api/v1/sync/ops?since={since}")).await
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

    pub async fn sent_folder_invites(&self) -> anyhow::Result<serde_json::Value> {
        self.get_typed("/api/v1/shares/invites/sent").await
    }

    /// `PUT /api/v1/shared-folders/{id}/key` — the folder's custody record.
    ///
    /// `format` declares the key format of the write (task 2258, L10):
    /// `Some("l7")` for a custody folder, `None` for a no-custody (legacy)
    /// write, where the field must be absent rather than null.
    pub async fn register_shared_folder_key(
        &self,
        id: &str,
        wrapped: &str,
        version: i32,
        format: Option<&str>,
    ) -> anyhow::Result<serde_json::Value> {
        let url = self.url(&format!("/api/v1/shared-folders/{}/key", encode(id)));
        let mut body = serde_json::json!({"owner_wrapped_folder_key":wrapped,"key_version":version});
        if let Some(format) = format {
            body["format"] = serde_json::Value::String(format.to_string());
        }
        let response = self.send_with_retry(|| self.client.put(&url).json(&body)).await?;
        Ok(response.json().await?)
    }

    pub async fn shared_folder_activity(&self, id: &str) -> anyhow::Result<serde_json::Value> {
        self.get_typed(&format!("/api/v1/shared-folders/{}/activity", encode(id))).await
    }

    pub async fn shared_index_manifest(&self, id: &str) -> anyhow::Result<serde_json::Value> {
        self.get_typed(&format!("/api/v1/shared-folders/{}/search-index/shards", encode(id))).await
    }

    pub async fn shared_index_get(&self, id: &str, bucket: u32, page: u32) -> anyhow::Result<Option<Vec<u8>>> {
        let url=self.url(&format!("/api/v1/shared-folders/{}/search-index/shards/{bucket}/{page}",encode(id)));
        let response=self.client.get(url).header("Authorization",format!("Bearer {}",self.token)).send().await?;
        if response.status()==reqwest::StatusCode::NOT_FOUND{return Ok(None);}
        Ok(Self::check_status(response).await?.bytes().await?.to_vec().into())
    }

    pub async fn shared_index_write(&self, id: &str, bucket: u32, page: u32, version: i64, blob: Option<&[u8]>) -> anyhow::Result<()> {
        let url=self.url(&format!("/api/v1/shared-folders/{}/search-index/shards/{bucket}/{page}?if_version={version}",encode(id)));
        self.send_with_retry(|| match blob {
            Some(blob)=>self.client.put(&url).header("Content-Type","application/octet-stream").body(blob.to_vec()),
            None=>self.client.delete(&url),
        }).await?;
        Ok(())
    }

    /// Paginated listing for a bounded shared-index rebuild; ordinary sync
    /// retains its snapshot/delta transport.
    pub async fn list_owned_folder_page(&self, parent: &str, cursor: Option<&str>) -> anyhow::Result<serde_json::Value> {
        let url=self.list_files_url(ListFilesScope {parent_id:Some(parent),cursor,..Default::default()});
        Ok(self.send_with_retry(||self.client.get(&url)).await?.json().await?)
    }

    pub async fn get_shared_folder(&self, id: &str) -> anyhow::Result<serde_json::Value> {
        self.get_typed(&format!("/api/v1/shared-folders/{}", encode(id))).await
    }

    pub async fn get_shared_folder_keys(&self, id: &str, cursor: Option<&str>) -> anyhow::Result<serde_json::Value> {
        let mut path = format!("/api/v1/shared-folders/{}/keys?limit=500", encode(id));
        if let Some(cursor) = cursor {
            path.push_str(&format!("&cursor={}", encode(cursor)));
        }
        self.get_typed(&path).await
    }

    pub async fn adopt_owner_key(&self, id: &str, owner_wrap_b64: &str) -> anyhow::Result<bool> {
        let url = self.url(&format!("/api/v1/files/{}/owner-key", encode(id)));
        let response = self.send_with_retry(|| self.client.post(&url).json(&serde_json::json!({
            "owner_wrapped_file_key": owner_wrap_b64
        }))).await?;
        let result: serde_json::Value = response.json().await?;
        result["adopted"].as_bool().ok_or_else(|| anyhow::anyhow!("invalid adoption response"))
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
            .get(self.thumbnail_url(file_id, variant))
            .header("Authorization", format!("Bearer {}", self.token))
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.bytes().await?.to_vec())
    }

    /// `PUT /api/v1/files/:id/thumbnail[/variant]` — upload a raw encrypted
    /// thumbnail blob in the web/mobile envelope (`nonce(12) || ciphertext+tag`).
    /// The server stores it opaquely; plaintext thumbnail bytes must never reach
    /// this method.
    pub async fn upload_thumbnail(
        &self,
        file_id: &str,
        variant: &str,
        encrypted: &[u8],
        blurhash: Option<&str>,
    ) -> anyhow::Result<()> {
        let resp = self
            .client
            .put(self.thumbnail_upload_url(file_id, variant, blurhash))
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Content-Type", "application/octet-stream")
            .body(encrypted.to_vec())
            .send()
            .await?;
        Self::check_status(resp).await?;
        Ok(())
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
            .await?;
        if resp.status() == reqwest::StatusCode::CONFLICT {
            // `error_for_status` would drop the message the class is read from (spec §8.4).
            let body: serde_json::Value = resp.json().await.unwrap_or(serde_json::Value::Null);
            let message = body.get("error").and_then(|value| value.as_str()).unwrap_or_default();
            return Err(anyhow::Error::new(InitConflict {
                class: classify_init_conflict(message),
                refusal: UploadRefusalCode::from_code(message),
            }));
        }
        Ok(upload_response(resp).await?.json().await?)
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
            .await?;
        Ok(upload_response(resp).await?.json().await?)
    }

    pub async fn complete_upload_session(&self, upload_session_id: &str) -> anyhow::Result<serde_json::Value> {
        let resp = self
            .client
            .post(self.upload_complete_url(upload_session_id))
            .header("Authorization", format!("Bearer {}", self.token))
            .json(&serde_json::json!({}))
            .send()
            .await?;
        Ok(upload_response(resp).await?.json().await?)
    }

    pub async fn list_shared_folder_files(
        &self,
        shared_folder_id: &str,
        parent_id: Option<&str>,
    ) -> anyhow::Result<Vec<serde_json::Value>> {
        const HARD_CAP: usize = 50_000;
        let mut out = Vec::new();
        let mut cursor: Option<String> = None;

        loop {
            let resp = self
                .client
                .get(self.list_files_url(ListFilesScope {
                    parent_id,
                    shared_folder_id: Some(shared_folder_id),
                    trashed: None,
                    cursor: cursor.as_deref(),
                }))
                .header("Authorization", format!("Bearer {}", self.token))
                .send()
                .await?
                .error_for_status()?;
            let body: serde_json::Value = resp.json().await?;
            let files = body["files"].as_array().cloned().unwrap_or_default();
            out.extend(files);
            if out.len() > HARD_CAP {
                anyhow::bail!("shared folder listing exceeded hard cap of {HARD_CAP}");
            }
            cursor = body["next_cursor"].as_str().map(str::to_string);
            if cursor.is_none() {
                break;
            }
        }

        Ok(out)
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
                anyhow::bail!("trashed file listing exceeded hard cap of {HARD_CAP}");
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

    /// SF-0001 (L13): abandon an in-progress upload session's server row.
    ///
    /// `POST /api/v1/files/{id}/upload/abandon` with no body. This is the
    /// create-session abandonment path: unlike `trash_file` it never removes
    /// the item from the user's listing, it only discards the unfinished
    /// session. A `404` means the row (or its session) is already gone, which
    /// counts as done.
    pub async fn abandon_upload(&self, file_id: &str) -> anyhow::Result<()> {
        let resp = self
            .client
            .post(self.upload_abandon_url(file_id))
            .header("Authorization", format!("Bearer {}", self.token))
            .send()
            .await?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(());
        }
        resp.error_for_status()?;
        Ok(())
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

    pub async fn permanent_delete_file(&self, file_id: &str, confirm_token: &str) -> anyhow::Result<()> {
        self.client
            .delete(self.permanent_file_url(file_id))
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

    pub async fn get_folder_keys(&self, invite_id: &str) -> anyhow::Result<serde_json::Value> {
        let resp = self
            .client
            .get(self.folder_keys_url(invite_id))
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
    pub(super) async fn get_typed<T: serde::de::DeserializeOwned>(&self, path: &str) -> anyhow::Result<T> {
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
    pub(super) async fn send_with_retry(&self, build: impl Fn() -> reqwest::RequestBuilder) -> anyhow::Result<reqwest::Response> {
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
    pub(super) async fn check_status(resp: reqwest::Response) -> anyhow::Result<reqwest::Response> {
        let status = resp.status();
        if status.is_success() {
            return Ok(resp);
        }
        let body = resp.text().await.unwrap_or_default();
        // L11: a custody write over a folder that still has legacy key rows is
        // refused until the owner upgrades it. It is its own state — never a
        // generic error and never a reason to retry as a legacy write.
        if status == reqwest::StatusCode::CONFLICT
            && serde_json::from_str::<serde_json::Value>(&body)
                .is_ok_and(|value| value["error"] == "upgrade_required")
        {
            return Err(anyhow::Error::new(UpgradeRequired));
        }
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
    /// Mirrors the webapp `getUserRegion()` contract: the list is by COUNTRY
    /// (task 2050), never a city and never the provider.
    pub async fn account_region(&self) -> anyhow::Result<crate::account_dto::UserRegionResponse> {
        self.get_typed("/api/v1/me/region").await
    }

    /// `PUT /api/v1/me/region` — set or clear the user's preferred region (`europe` or a country
    /// id). `region_strict` is "only store in <country>"; the server replaces the whole choice, so
    /// an omitted value means false there. `None` leaves it out of the body.
    pub async fn set_preferred_region(
        &self,
        preferred_region: Option<&str>,
        region_strict: Option<bool>,
    ) -> anyhow::Result<crate::account_dto::SetPreferredRegionResponse> {
        let body = set_region_body(preferred_region, region_strict);
        #[cfg(test)]
        if let Some(response) = region_set_fixture::try_handle(&self.base_url, &self.token, &body)? {
            return Ok(response);
        }

        let url = self.url("/api/v1/me/region");
        let resp = self.send_with_retry(|| self.client.put(&url).json(&body)).await?;
        Ok(resp.json().await?)
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
    pub async fn client_sync_sessions(&self) -> anyhow::Result<crate::account_dto::ClientSyncSessionList> {
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
        let resp = self.send_with_retry(|| self.client.post(&url).json(&body)).await?;
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
        let resp = self.send_with_retry(|| self.client.post(&url).json(&body)).await?;
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
        let url = self.url(&format!("/api/v1/clients/sessions/{}/heartbeat", encode(session_id)));
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
    pub async fn notification_preferences(&self) -> anyhow::Result<crate::account_dto::NotificationPreferences> {
        self.get_typed("/api/v1/notifications/preferences").await
    }

    /// `PUT /api/v1/notifications/preferences` — partial update; omitted
    /// fields keep their current server value.
    pub async fn update_notification_preferences(
        &self,
        update: &crate::account_dto::NotificationPreferencesUpdate,
    ) -> anyhow::Result<crate::account_dto::NotificationPreferences> {
        let url = self.url("/api/v1/notifications/preferences");
        let resp = self.send_with_retry(|| self.client.put(&url).json(update)).await?;
        Ok(resp.json().await?)
    }

    // ── Speedtest endpoints (P2 — task 0810) ─────────────────────────────────
    //
    // Mirrors the CLI (`repos/cli/src/commands/speedtest.rs`) and iOS
    // (`SpeedtestScreen`) implementations exactly so results are comparable
    // across clients.  All three use the same parameter set:
    //   latency  — 5× GET /api/v1/health
    //   download — 3× GET /api/v1/speedtest?size=10000000 (10 MB)
    //   upload   — 3× POST /api/v1/speedtest with a 10 MB octet-stream body

    /// `GET /api/v1/health` — used for latency measurement.  Returns `Ok(())`
    /// when the server responds (even a non-200 counts; we only care about RTT).
    pub async fn ping_health(&self) -> anyhow::Result<()> {
        let url = self.url("/api/v1/health");
        let _ = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .timeout(Duration::from_secs(10))
            .send()
            .await?;
        Ok(())
    }

    /// `GET /api/v1/speedtest?size=N` — download `size` bytes of random data.
    /// Returns the raw bytes (length is the measurement input).
    pub async fn speedtest_download(&self, size: u64) -> anyhow::Result<Vec<u8>> {
        let url = format!("{}/api/v1/speedtest?size={size}", self.base_url);
        let resp = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .timeout(Duration::from_secs(120))
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.bytes().await?.to_vec())
    }

    /// `POST /api/v1/speedtest` — upload `data` as a raw octet-stream and
    /// discard the server response.  The body is random zeros; the server
    /// sinks them and responds 204.
    pub async fn speedtest_upload(&self, data: &[u8]) -> anyhow::Result<()> {
        let url = self.url("/api/v1/speedtest");
        self.client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Content-Type", "application/octet-stream")
            .body(data.to_vec())
            .timeout(Duration::from_secs(120))
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }
}
