//! Account, billing, region, sessions, devices, telemetry, notifications and
//! the speedtest endpoints: the data behind the desktop's views.
//!
//! Every call here goes through [`ApiClient::get_typed`] or
//! [`ApiClient::send_with_retry`], which ride out a bounded 429 and turn any
//! other non-2xx into a typed [`DesktopApiError`] with the status and a short
//! body snippet. Must not: retry by hand, or read a body a view will show.
//! Tests: `api_client/tests/`.

use super::*;

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
    /// `GET /api/v1/auth/me` — account profile.
    pub async fn account_profile(&self) -> anyhow::Result<crate::account_dto::AccountProfile> {
        Ok(self.get_typed("/api/v1/auth/me").await?)
    }

    /// GET /api/v1/me/profile: the name the account shows. `None`: none was set (or only spaces).
    pub async fn account_display_name(&self) -> anyhow::Result<Option<String>> {
        #[derive(serde::Deserialize)]
        struct Profile {
            #[serde(default)]
            display_name: Option<String>,
        }
        let profile: Profile = self.get_typed("/api/v1/me/profile").await?;
        Ok(profile
            .display_name
            .map(|name| name.trim().to_string())
            .filter(|name| !name.is_empty()))
    }

    /// `GET /api/v1/billing/subscription` — plan + quota + lifecycle.
    pub async fn subscription(&self) -> anyhow::Result<crate::account_dto::Subscription> {
        Ok(self.get_typed("/api/v1/billing/subscription").await?)
    }

    /// `GET /api/v1/billing/usage` — used/quota bytes + percentage.
    pub async fn billing_usage(&self) -> anyhow::Result<crate::account_dto::BillingUsage> {
        Ok(self.get_typed("/api/v1/billing/usage").await?)
    }

    /// `GET /api/v1/me/region` — the user's preferred region + available list.
    /// Mirrors the webapp `getUserRegion()` contract: the list is by COUNTRY
    /// (task 2050), never a city and never the provider.
    pub async fn account_region(&self) -> anyhow::Result<crate::account_dto::UserRegionResponse> {
        Ok(self.get_typed("/api/v1/me/region").await?)
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
        #[cfg(any(test, feature = "test-support"))]
        if let Some(response) = region_set_fixture::try_handle(&self.base_url, &self.token, &body)? {
            return Ok(response);
        }

        let url = self.url("/api/v1/me/region");
        let resp = self.send_with_retry(|| self.client.put(&url).json(&body)).await?;
        Ok(resp.json().await?)
    }

    /// `GET /api/v1/account/activity` — events + summary.
    pub async fn account_activity(&self) -> anyhow::Result<crate::account_dto::AccountActivity> {
        Ok(self.get_typed("/api/v1/account/activity").await?)
    }

    /// `GET /api/v1/account/security-score` — numeric score + factors.
    pub async fn security_score(&self) -> anyhow::Result<crate::account_dto::SecurityScore> {
        Ok(self.get_typed("/api/v1/account/security-score").await?)
    }

    /// `GET /api/v1/account/sessions` — active account sessions.
    pub async fn account_sessions(&self) -> anyhow::Result<crate::account_dto::AccountSessionList> {
        Ok(self.get_typed("/api/v1/account/sessions").await?)
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
        Ok(self.get_typed("/api/v1/clients/devices").await?)
    }

    /// `GET /api/v1/clients/sessions` — per-device sync sessions.
    pub async fn client_sync_sessions(&self) -> anyhow::Result<crate::account_dto::ClientSyncSessionList> {
        Ok(self.get_typed("/api/v1/clients/sessions").await?)
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
        Ok(self.get_typed(&path).await?)
    }

    /// `GET /api/v1/notifications` — notification list + unread count.
    pub async fn notifications(&self) -> anyhow::Result<crate::account_dto::NotificationList> {
        Ok(self.get_typed("/api/v1/notifications").await?)
    }

    /// `GET /api/v1/notifications/preferences` — push-preference toggles.
    pub async fn notification_preferences(&self) -> anyhow::Result<crate::account_dto::NotificationPreferences> {
        Ok(self.get_typed("/api/v1/notifications/preferences").await?)
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

    /// `PATCH /api/v1/notifications/{id}` with `{ "read": true }`: one notification seen. One the server no longer
    /// has (404: removed meanwhile) counts as seen.
    pub async fn mark_notification_read(&self, id: &str) -> anyhow::Result<()> {
        let url = self.url(&format!("/api/v1/notifications/{}", encode(id)));
        let body = serde_json::json!({ "read": true });
        match self.send_with_retry(|| self.client.patch(&url).json(&body)).await {
            Ok(_) => Ok(()),
            Err(DesktopApiError::HttpStatus { status, .. }) if status == reqwest::StatusCode::NOT_FOUND => Ok(()),
            Err(error) => Err(error.into()),
        }
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
