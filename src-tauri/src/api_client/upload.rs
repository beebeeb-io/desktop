//! The upload session: init, the chunks, the thumbnail, complete and abandon.
//!
//! Owns: the refusal mapping — init's 409 becomes a typed [`InitConflict`],
//! every other non-2xx a typed [`UploadRefused`] that keeps the server's code
//! and never its body. Must not: build the session URLs (see `client`) or
//! choose to retry (see `engine_bridge`). Tests: `api_client/tests/`.

use super::*;

impl ApiClient {
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
            .put(self.thumbnail_upload_url(FileId::new(file_id), variant, blurhash))
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Content-Type", "application/octet-stream")
            .body(encrypted.to_vec())
            .send()
            .await?;
        Self::check_status(resp).await?;
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
            return Err(DesktopApiError::InitConflict(InitConflict {
                class: classify_init_conflict(message),
                refusal: UploadRefusalCode::from_code(message),
            })
            .into());
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
            .put(self.upload_chunk_url(UploadSessionId::new(upload_session_id), chunk_idx))
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
            .post(self.upload_complete_url(UploadSessionId::new(upload_session_id)))
            .header("Authorization", format!("Bearer {}", self.token))
            .json(&serde_json::json!({}))
            .send()
            .await?;
        Ok(upload_response(resp).await?.json().await?)
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
            .post(self.upload_abandon_url(FileId::new(file_id)))
            .header("Authorization", format!("Bearer {}", self.token))
            .send()
            .await?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(());
        }
        resp.error_for_status()?;
        Ok(())
    }
}
