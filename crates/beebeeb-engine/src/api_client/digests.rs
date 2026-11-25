//! `GET /api/v1/folder-digests`: one page of a folder's children with their digests (folder digest v1).
//!
//! Owns: reading the endpoint's answers into [`DigestsAnswer`], the one place that tells "this server has no
//! digests" (any other 404, a 405) from "this folder is gone" (404 `folder_not_found`) and "not yet" (503
//! `digests_not_ready` with `Retry-After`). Must not: walk a tree (see `crate::folder_digest`) or log an id.
//! Retries: the client's usual policy (`headers`: bounded 429 backoff, bounded transport/502/504 retries); a 503
//! `digests_not_ready` is an answer, not a retry, so its `Retry-After` reaches the caller.

use super::*;
use beebeeb_types::folder_digest::FolderDigestPage;

/// What the server answered to one digests request.
#[derive(Debug)]
pub enum DigestsAnswer {
    /// 200: one page.
    Page(FolderDigestPage),
    /// 404 `folder_not_found`: an unknown id, a file, a trashed folder or one the caller may not read.
    FolderGone,
    /// 503 `digests_not_ready`: the account's digests are not computed yet; ask again after this many seconds.
    NotReady { retry_after_secs: u64 },
    /// Any other 404, or a 405: a server without digests. The caller lists the old way.
    Unsupported,
}

/// The wait a `digests_not_ready` answer without a usable `Retry-After` gets.
pub const NOT_READY_DEFAULT_RETRY_SECS: u64 = 30;

/// The `error` code of a JSON error body, if it has one.
fn error_code(body: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()?
        .get("error")?
        .as_str()
        .map(str::to_string)
}

impl ApiClient {
    /// A non-2xx answer as the client's usual error (`HTTP <status>: <snippet>`).
    async fn status_error(resp: reqwest::Response) -> anyhow::Error {
        match Self::check_status(resp).await {
            Err(e) => e.into(),
            Ok(resp) => anyhow::anyhow!("HTTP {}", resp.status()),
        }
    }

    pub(super) fn folder_digests_url(&self, folder_id: Option<&str>, cursor: Option<&str>, limit: u32) -> String {
        let mut query = Vec::new();
        if let Some(folder_id) = folder_id {
            query.push(format!("folder_id={}", encode(folder_id)));
        }
        if let Some(cursor) = cursor {
            query.push(format!("cursor={}", encode(cursor)));
        }
        query.push(format!("limit={limit}"));
        self.url(&format!("/api/v1/folder-digests?{}", query.join("&")))
    }

    /// `GET /api/v1/folder-digests?folder_id=&cursor=&limit=`. `folder_id` None = the account's root; `limit` 0 =
    /// the digest only (0..=1000). Transport failures, a 429 or 5xx the retries did not clear, and any other non-2xx
    /// are an `Err`.
    pub async fn folder_digests(
        &self,
        folder_id: Option<&str>,
        cursor: Option<&str>,
        limit: u32,
    ) -> anyhow::Result<DigestsAnswer> {
        let url = self.folder_digests_url(folder_id, cursor, limit);
        let mut spent: u64 = 0;
        let mut attempt: u32 = 0;
        let mut transport_attempt: u32 = 0;
        let mut held: Option<reqwest::Response> = None;
        loop {
            let request = self
                .client
                .get(&url)
                .header("Authorization", format!("Bearer {}", self.token))
                .build()?;
            let resp = match self.client.execute(request).await {
                Ok(resp) => resp,
                Err(e) => {
                    if let Some(old) = held.take() {
                        return Err(Self::status_error(old).await);
                    }
                    if let Some(wait) = transport_retry_decision(
                        transport_attempt,
                        e.is_connect(),
                        e.is_timeout() || e.is_request(),
                        true,
                    ) {
                        tokio::time::sleep(wait).await;
                        transport_attempt += 1;
                        continue;
                    }
                    self.link.note_reqwest_error(&e, now_unix_secs() as i64);
                    return Err(DesktopApiError::from(e).into());
                }
            };
            self.link.note_status(resp.status(), now_unix_secs() as i64);
            let status = resp.status().as_u16();
            match status {
                200..=299 => return Ok(DigestsAnswer::Page(resp.json::<FolderDigestPage>().await?)),
                405 => return Ok(DigestsAnswer::Unsupported),
                404 => {
                    let body = resp.text().await.unwrap_or_default();
                    return Ok(match error_code(&body).as_deref() {
                        Some("folder_not_found") => DigestsAnswer::FolderGone,
                        _ => DigestsAnswer::Unsupported,
                    });
                }
                503 | 502 | 504 => {
                    let retry_after = header_secs(resp.headers(), reqwest::header::RETRY_AFTER.as_str());
                    if status == 503 {
                        let body = resp.text().await.unwrap_or_default();
                        if error_code(&body).as_deref() == Some("digests_not_ready") {
                            return Ok(DigestsAnswer::NotReady {
                                retry_after_secs: retry_after.unwrap_or(NOT_READY_DEFAULT_RETRY_SECS),
                            });
                        }
                        let snippet: String = body.chars().take(300).collect();
                        if let Some(wait) = transport_retry_decision(transport_attempt, false, true, true) {
                            tokio::time::sleep(wait).await;
                            transport_attempt += 1;
                            continue;
                        }
                        return Err(DesktopApiError::HttpStatus {
                            status: reqwest::StatusCode::SERVICE_UNAVAILABLE,
                            snippet,
                        }
                        .into());
                    }
                    if let Some(wait) = transport_retry_decision(transport_attempt, false, true, true) {
                        tokio::time::sleep(wait).await;
                        transport_attempt += 1;
                        held = Some(resp);
                        continue;
                    }
                    return Err(Self::status_error(resp).await);
                }
                429 => {
                    let retry_after = header_secs(resp.headers(), reqwest::header::RETRY_AFTER.as_str());
                    let reset_in = header_secs(resp.headers(), "x-ratelimit-reset")
                        .map(|epoch| epoch.saturating_sub(now_unix_secs()));
                    match retry_decision(attempt, spent, retry_after, reset_in) {
                        Some(wait) => {
                            if wait > 0 {
                                tokio::time::sleep(Duration::from_secs(wait)).await;
                            }
                            spent = spent.saturating_add(wait);
                            attempt += 1;
                        }
                        None => {
                            return Err(Self::status_error(resp).await);
                        }
                    }
                }
                _ => {
                    return Err(Self::status_error(resp).await);
                }
            }
        }
    }
}
