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
        let url = self.thumbnail_upload_url(FileId::new(file_id), variant, blurhash);
        let resp = self
            .send_paced(&self.client, || {
                self.client
                    .put(&url)
                    .header("Content-Type", "application/octet-stream")
                    .body(encrypted.to_vec())
            })
            .await?;
        Self::check_status(resp).await?;
        Ok(())
    }

    pub async fn init_upload(&self, body: &DesktopUploadInitRequest) -> anyhow::Result<DesktopUploadInitResponse> {
        // No idempotency key: sent once, after any running pause; a 429 is the error.
        let url = self.upload_init_url();
        let resp = self
            .send_paced(&self.client, || {
                self.client
                    .post(&url)
                    .header(CAPABILITIES_HEADER_NAME, WRITE_CAPABILITIES)
                    .json(body)
            })
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
        let url = self.upload_chunk_url(UploadSessionId::new(upload_session_id), chunk_idx);
        let resp = self
            .send_paced(&self.client, || {
                self.client
                    .put(&url)
                    .header(CAPABILITIES_HEADER_NAME, WRITE_CAPABILITIES)
                    .header("Content-Type", "application/octet-stream")
                    .body(data.to_vec())
            })
            .await?;
        Ok(upload_response(resp).await?.json().await?)
    }

    /// `POST /api/v1/uploads/{session}/complete`. `completion_name` is the upload's sealed name with its
    /// version commitment (chunk format v2): the server publishes it as the file's name and as the new
    /// version's metadata in the same transaction that makes the version current, as core's client sends
    /// it. `None` only for a session a build before chunk format v2 opened (its frames are v1).
    pub async fn complete_upload_session(
        &self,
        upload_session_id: &str,
        completion_name: Option<&str>,
    ) -> anyhow::Result<serde_json::Value> {
        let body = match completion_name {
            Some(name) => serde_json::json!({ "name_encrypted": name, "version_metadata_encrypted": name }),
            None => serde_json::json!({}),
        };
        // The commit carries no idempotency key, so it is sent exactly once: it waits for a running pause, and a 429
        // is the error (the caller decides what a refused commit means; a replay here could commit twice).
        let url = self.upload_complete_url(UploadSessionId::new(upload_session_id));
        let resp = self
            .send_paced(&self.client, || {
                self.client
                    .post(&url)
                    .header(CAPABILITIES_HEADER_NAME, WRITE_CAPABILITIES)
                    .json(&body)
            })
            .await?;
        Ok(upload_response(resp).await?.json().await?)
    }

    /// Whether the server still holds `upload_session_id` open under a live lease
    /// (`POST /api/v1/uploads/{session}/heartbeat`, which also renews it). `false` for every refusal
    /// (reaped, completed, not this account's): the caller asks only to decide whether the file's
    /// in-flight upload is its own. A request that did not arrive, and a server error, is an error.
    pub async fn upload_session_is_live(&self, upload_session_id: &str) -> anyhow::Result<bool> {
        let url = format!(
            "{}/heartbeat",
            self.upload_complete_url(UploadSessionId::new(upload_session_id))
                .trim_end_matches("/complete")
        );
        // A 429 is an error here (the session's state is unknown), never `false`.
        let resp = self
            .send_paced(&self.client, || {
                self.client
                    .post(&url)
                    .header(CAPABILITIES_HEADER_NAME, WRITE_CAPABILITIES)
            })
            .await?;
        if resp.status().is_client_error() {
            return Ok(false);
        }
        resp.error_for_status()?;
        Ok(true)
    }

    /// SF-0001 (L13): abandon an in-progress upload session's server row.
    ///
    /// `POST /api/v1/files/{id}/upload/abandon` with no body. This is the
    /// create-session abandonment path: unlike `trash_file` it never removes
    /// the item from the user's listing, it only discards the unfinished
    /// session. A `404` means the row (or its session) is already gone, which
    /// counts as done.
    pub async fn abandon_upload(&self, file_id: &str) -> anyhow::Result<()> {
        let url = self.upload_abandon_url(FileId::new(file_id));
        let resp = self.send_paced(&self.client, || self.client.post(&url)).await?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(());
        }
        resp.error_for_status()?;
        Ok(())
    }

    /// Reconciles an upload whose `complete` answer was lost and whose session is gone (404/410): did the
    /// server commit `object_version_id` for `file_id`? `Some(answer)` is shaped like `complete`'s answer
    /// (`version_number`, `current_object_version_id`), so the caller lands it once instead of uploading
    /// again. `None` means the version is not there (or the file is not): a fresh upload is then right.
    /// Never abandons or trashes anything.
    pub async fn committed_upload(
        &self,
        file_id: &str,
        object_version_id: &str,
    ) -> anyhow::Result<Option<serde_json::Value>> {
        let versions = match self.list_versions(file_id).await {
            Ok(versions) => versions,
            Err(error) if error_is_not_found(&error) => return Ok(None),
            Err(error) => return Err(error),
        };
        Ok(committed_answer(&versions, object_version_id))
    }
}

fn error_is_not_found(error: &anyhow::Error) -> bool {
    error
        .chain()
        .filter_map(|cause| cause.downcast_ref::<reqwest::Error>())
        .any(|e| e.status() == Some(reqwest::StatusCode::NOT_FOUND))
}

/// The `complete`-shaped answer for `object_version_id` out of the server's versions listing
/// (`GET /files/{id}/versions`, server `routes/versions/list.rs`: `{ "versions": [{ "id",
/// "object_version_id", "version_number", "size_bytes", .. }] }`), or `None` when the listing does not
/// hold it. The server lists only finished versions (`available`, `completed`), so one that is listed
/// was committed. A version of the older model has a null `object_version_id` and never matches.
pub(crate) fn committed_answer(listing: &serde_json::Value, object_version_id: &str) -> Option<serde_json::Value> {
    let list = listing["versions"].as_array()?;
    let found = list
        .iter()
        .find(|v| v["object_version_id"].as_str() == Some(object_version_id))?;
    let number = found["version_number"].as_i64();
    let mut answer = serde_json::json!({
        "current_object_version_id": object_version_id,
        "already_completed": true,
    });
    if let Some(number) = number {
        answer["version_number"] = number.into();
    }
    if let Some(size) = found["size_bytes"].as_u64() {
        answer["size_bytes"] = size.into();
    }
    Some(answer)
}

#[cfg(test)]
mod tests {
    use super::committed_answer;
    use serde_json::json;

    /// A lost `complete` answer plus a reaped session: the committed version is found, once, by its id.
    #[test]
    fn upload_ack_lost_after_commit_is_found_by_its_version_id() {
        // The server's shape (`routes/versions/list.rs`): an upload's version carries its id twice, a
        // version of the older model has no object version id.
        let listing = json!({ "file_id": "f", "current_version": 2, "versions": [
            { "id": "v2", "object_version_id": "v2", "file_version_id": null, "version_number": 2, "size_bytes": 9 },
            { "id": "v1", "object_version_id": "v1", "file_version_id": null, "version_number": 1, "size_bytes": 5 },
            { "id": "old", "object_version_id": null, "file_version_id": "old", "version_number": 1, "size_bytes": 3 },
        ]});
        let answer = committed_answer(&listing, "v2").expect("committed");
        assert_eq!(answer["version_number"], 2);
        assert_eq!(answer["size_bytes"], 9);
        assert_eq!(answer["current_object_version_id"], "v2");
        assert_eq!(answer["already_completed"], true);
        // An upload that never committed, a version of the older model, and a listing with no versions.
        assert!(committed_answer(&listing, "v3").is_none());
        assert!(committed_answer(&listing, "old").is_none());
        assert!(committed_answer(&json!({}), "v1").is_none());
    }
}
