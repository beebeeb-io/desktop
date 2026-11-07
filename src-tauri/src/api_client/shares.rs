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
        let url = self.owned_folder_page_url(parent, cursor);
        Ok(self.send_with_retry(|| self.client.get(&url)).await?.json().await?)
    }

    fn owned_folder_page_url(&self, parent: &str, cursor: Option<&str>) -> String {
        self.list_files_url(ListFilesScope {
            parent_id: Some(FolderId::new(parent)),
            cursor,
            ..Default::default()
        })
    }

    /// [`Self::list_owned_folder_page`] with the clock of its request (LD-0123), for the backup records' rebuild:
    /// the response's `Date` header against this device's wall clock when the request went out, with the round trip
    /// timed on the monotonic clock ([`measured_clock`]). A 429 retry inside `send_with_retry` lengthens the round
    /// trip, which only widens the margin (more is uploaded again, never less).
    pub async fn list_owned_folder_page_clocked(
        &self,
        parent: &str,
        cursor: Option<&str>,
    ) -> anyhow::Result<(serde_json::Value, beebeeb_core::backup::RebuildClock)> {
        let url = self.owned_folder_page_url(parent, cursor);
        let sent_at_ms = chrono::Utc::now().timestamp_millis();
        let started = std::time::Instant::now();
        let response = self.send_with_retry(|| self.client.get(&url)).await?;
        let elapsed_ms = i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX);
        let received_wall_ms = chrono::Utc::now().timestamp_millis();
        let date = response
            .headers()
            .get(reqwest::header::DATE)
            .and_then(|v| v.to_str().ok());
        let clock = measured_clock(date, sent_at_ms, elapsed_ms, received_wall_ms);
        Ok((response.json().await?, clock))
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

/// How far a wall clock may drift from the monotonic clock over one request before the request counts as having
/// seen a clock step (NTP slews at most 0.5 ms a second; a step is seconds).
const CLOCK_STEP_TOLERANCE_MS: i64 = 250;

/// The clock of one listing request (LD-0123, review of LD-0139): core's `rebuild_clock_from_date` over the `Date`
/// header, with the round trip taken from the monotonic clock (`elapsed_ms`), so a wall-clock step cannot shrink the
/// margin. When the wall clock moved more than [`CLOCK_STEP_TOLERANCE_MS`] away from the monotonic one during the
/// request (`received_wall_ms`), the device's clock was stepped and nothing it measured holds: the clock is
/// `RebuildClock::UNKNOWN`, and core's 10-minute floor applies (more is uploaded again, never less).
pub(crate) fn measured_clock(
    date: Option<&str>,
    sent_at_ms: i64,
    elapsed_ms: i64,
    received_wall_ms: i64,
) -> beebeeb_core::backup::RebuildClock {
    let received_at_ms = sent_at_ms.saturating_add(elapsed_ms);
    if received_wall_ms.abs_diff(received_at_ms) > CLOCK_STEP_TOLERANCE_MS.unsigned_abs() {
        return beebeeb_core::backup::RebuildClock::UNKNOWN;
    }
    beebeeb_core::backup::rebuild_clock_from_date(date, sent_at_ms, received_at_ms)
}

#[cfg(test)]
mod measured_clock_tests {
    use super::measured_clock;
    use beebeeb_core::backup::RebuildClock;

    // Sun, 06 Nov 1994 08:49:37 GMT
    const DATE: &str = "Sun, 06 Nov 1994 08:49:37 GMT";
    const SERVER_MS: i64 = 784_111_777_000;

    #[test]
    fn a_steady_clock_measures_the_offset_with_the_monotonic_round_trip() {
        let sent = SERVER_MS - 60_000;
        let clock = measured_clock(Some(DATE), sent, 400, sent + 400);
        assert_eq!(
            clock,
            RebuildClock {
                offset_ms: Some(60_000 - 200),
                margin_ms: 2_000 + 400
            }
        );
    }

    #[test]
    fn a_clock_stepped_during_the_request_measures_nothing() {
        let sent = SERVER_MS - 60_000;
        // Stepped back 3 s during a 3.1 s round trip: the wall clock alone would see 0.1 s and a 2.1 s margin.
        assert_eq!(
            measured_clock(Some(DATE), sent, 3_100, sent + 100),
            RebuildClock::UNKNOWN
        );
        // Stepped forward 1 s.
        assert_eq!(
            measured_clock(Some(DATE), sent, 400, sent + 1_400),
            RebuildClock::UNKNOWN
        );
        // Within the slew tolerance it still measures.
        assert!(measured_clock(Some(DATE), sent, 400, sent + 600).offset_ms.is_some());
    }

    #[test]
    fn no_date_measures_nothing() {
        assert_eq!(measured_clock(None, 0, 100, 100), RebuildClock::UNKNOWN);
        assert_eq!(measured_clock(Some("yesterday"), 0, 100, 100), RebuildClock::UNKNOWN);
    }
}
