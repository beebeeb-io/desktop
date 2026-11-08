//! The Backups root endpoints (LD-0113 spec §5.2, server `42a05fac`): `GET` and `PUT /api/v1/backups/root`, and the
//! root-level listing the root resolution reads (`GET /api/v1/files` with no parent).
//!
//! Owns: those three requests, and the pages of the notices read each backup pass starts with (LD-0139). Must not:
//! decide which folder is the root (core's `resolve_root`), seal a name, or decide what a notice means (core's
//! `backup_pass_gate_v2`).

use super::*;

/// What `PUT /api/v1/backups/root` answered: the account's root and how it was resolved (`existing`, `claimed`,
/// `created`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct BackupsRootAnswer {
    pub id: String,
    pub outcome: String,
}

impl ApiClient {
    /// `GET /api/v1/backups/root`: the account's flagged root, `None` when it has none yet (404).
    pub async fn get_backups_root(&self) -> anyhow::Result<Option<String>> {
        let resp = self
            .client
            .get(self.url("/api/v1/backups/root"))
            .header("Authorization", format!("Bearer {}", self.token))
            .send()
            .await?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let body: serde_json::Value = resp.error_for_status()?.json().await?;
        let id = body["id"].as_str().ok_or_else(|| anyhow::anyhow!("backups root answer has no id"))?;
        Ok(Some(id.to_string()))
    }

    /// `PUT /api/v1/backups/root`: use the flagged root, else claim `folder_id`, else (with `name_encrypted`) create
    /// the root with that id and sealed name, in one server transaction.
    pub async fn put_backups_root(
        &self,
        folder_id: &str,
        name_encrypted: Option<&str>,
    ) -> anyhow::Result<BackupsRootAnswer> {
        let mut body = serde_json::json!({ "folder_id": folder_id });
        if let Some(name) = name_encrypted {
            body["name_encrypted"] = serde_json::json!(name);
        }
        let resp = self
            .client
            .put(self.url("/api/v1/backups/root"))
            .header("Authorization", format!("Bearer {}", self.token))
            .json(&body)
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }

    /// One page of a backup pass's notice check (LD-0139, server LD-0141): `GET /api/v1/notifications` filtered to
    /// the two backup types, from `since_ms` (integer milliseconds, server clock, inclusive) on, newest first, at
    /// most 500 rows; `cursor` is the previous page's `next_cursor`. With the page: the clock of the request
    /// (`measured_clock`: no `Date`, or a clock that moved, is unknown) and the answer's `Date` in milliseconds
    /// (server clock).
    pub async fn list_backup_notices_page(
        &self,
        since_ms: i64,
        cursor: Option<&str>,
    ) -> anyhow::Result<(serde_json::Value, beebeeb_core::backup::RebuildClock, Option<i64>)> {
        use beebeeb_types::backup::{NOTICE_BACKUP_DELETED, NOTICE_FILES_REMOVED};
        let mut path =
            format!("/api/v1/notifications?type={NOTICE_FILES_REMOVED},{NOTICE_BACKUP_DELETED}&since={since_ms}");
        if let Some(cursor) = cursor {
            path.push_str(&format!("&cursor={}", encode(cursor)));
        }
        let url = self.url(&path);
        let sent_at_ms = chrono::Utc::now().timestamp_millis();
        let started = std::time::Instant::now();
        let response = self.send_with_retry(|| self.client.get(&url)).await?;
        let elapsed_ms = i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX);
        let received_wall_ms = chrono::Utc::now().timestamp_millis();
        let date = response
            .headers()
            .get(reqwest::header::DATE)
            .and_then(|v| v.to_str().ok());
        let clock = super::shares::measured_clock(date, sent_at_ms, elapsed_ms, received_wall_ms);
        let server_now_ms = date
            .and_then(|date| chrono::DateTime::parse_from_rfc2822(date.trim()).ok())
            .map(|date| date.timestamp_millis());
        Ok((response.json().await?, clock, server_now_ms))
    }

    /// One page of the account's root-level items (`GET /api/v1/files`, no parent), for the root resolution.
    pub async fn list_root_page(&self, cursor: Option<&str>) -> anyhow::Result<serde_json::Value> {
        let url = self.list_files_url(ListFilesScope { cursor, ..Default::default() });
        Ok(self.send_with_retry(|| self.client.get(&url)).await?.json().await?)
    }
}
