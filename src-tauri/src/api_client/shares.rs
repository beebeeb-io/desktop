//! Shared folders: invites, the owner's custody writes, the shared index and
//! the shared listing.
//!
//! Owns: the shared-listing hard cap that makes a looping cursor fail instead
//! of growing memory. Must not: hold or wrap key material (see `owner_keys`) or
//! decide what a refusal means (see `types`). Tests: `api_client/tests/`.

use super::*;

impl ApiClient {
    /// `GET /api/v1/file-requests` — the owner's upload requests, with their
    /// master-wrapped request private keys. Read-only: nothing here is ever
    /// sealed, adopted or published (task 2275, L12).
    pub async fn list_file_requests(&self) -> anyhow::Result<serde_json::Value> {
        Ok(self.get_typed("/api/v1/file-requests").await?)
    }

    pub async fn sent_folder_invites(&self) -> anyhow::Result<serde_json::Value> {
        Ok(self.get_typed("/api/v1/shares/invites/sent").await?)
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
        Ok(self.get_typed(&format!("/api/v1/shared-folders/{}/activity", encode(id))).await?)
    }

    pub async fn shared_index_manifest(&self, id: &str) -> anyhow::Result<serde_json::Value> {
        Ok(self.get_typed(&format!("/api/v1/shared-folders/{}/search-index/shards", encode(id))).await?)
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
        let url=self.list_files_url(ListFilesScope {parent_id:Some(FolderId::new(parent)),cursor,..Default::default()});
        Ok(self.send_with_retry(||self.client.get(&url)).await?.json().await?)
    }

    pub async fn get_shared_folder(&self, id: &str) -> anyhow::Result<serde_json::Value> {
        Ok(self.get_typed(&format!("/api/v1/shared-folders/{}", encode(id))).await?)
    }

    pub async fn get_shared_folder_keys(&self, id: &str, cursor: Option<&str>) -> anyhow::Result<serde_json::Value> {
        let mut path = format!("/api/v1/shared-folders/{}/keys?limit=500", encode(id));
        if let Some(cursor) = cursor {
            path.push_str(&format!("&cursor={}", encode(cursor)));
        }
        Ok(self.get_typed(&path).await?)
    }

    pub async fn adopt_owner_key(&self, id: &str, owner_wrap_b64: &str) -> anyhow::Result<bool> {
        let url = self.url(&format!("/api/v1/files/{}/owner-key", encode(id)));
        let response = self.send_with_retry(|| self.client.post(&url).json(&serde_json::json!({
            "owner_wrapped_file_key": owner_wrap_b64
        }))).await?;
        let result: serde_json::Value = response.json().await?;
        result["adopted"]
            .as_bool()
            .ok_or(DesktopApiError::InvalidAdoptionResponse)
            .map_err(Into::into)
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
                    parent_id: parent_id.map(FolderId::new),
                    shared_folder_id: Some(SharedFolderId::new(shared_folder_id)),
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
                return Err(DesktopApiError::ListingTooLarge {
                    listing: Listing::SharedFolder,
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

    pub async fn get_folder_keys(&self, invite_id: &str) -> anyhow::Result<serde_json::Value> {
        let resp = self
            .client
            .get(self.folder_keys_url(InviteId::new(invite_id)))
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
}
