//! Photo previews: the account's preview setting (`GET/PUT /api/v1/preferences/photo_previews`), what the stored
//! previews take (`GET /api/v1/files/previews/summary`), each of the owner's media files' preview facts
//! (`GET /api/v1/files/media`: the account's OWN live media only), and the upload of one sealed preview
//! (`PUT /api/v1/files/{id}/thumbnail/preview?format=jpeg&px=N`).
//!
//! Must not: send a preview that is not sealed (the caller seals; this sends the bytes it is given as they are);
//! carry the server's words, a URL or an id in an error ([`PreviewPutError`] is a kind and a status only).

use super::*;
use crate::photo_previews::{PreviewFormat, PreviewSetting, PreviewState, PreviewsSummary};

/// Why a preview was not stored. A kind and a status only: never the server's words, a URL or an id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewPutError {
    /// 413 `quota_exceeded`: the account's storage is full.
    QuotaExceeded,
    /// Any other 413: the preview is over the server's cap.
    TooLarge,
    /// The server could not be reached, or did not answer.
    Offline,
    /// 429: Beebeeb asks to wait (`retry_after`: its hint in seconds) before this is sent again.
    RateLimited { retry_after: Option<u64> },
    /// Any other answer that is not a success (401: the session ended; 404: not the account's own file).
    Status(u16),
}

impl std::fmt::Display for PreviewPutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::QuotaExceeded => f.write_str("the account's storage is full"),
            Self::TooLarge => f.write_str("the preview is too large"),
            Self::Offline => f.write_str("offline"),
            Self::RateLimited { .. } => f.write_str("asked to wait"),
            Self::Status(status) => write!(f, "the server answered {status}"),
        }
    }
}

impl std::error::Error for PreviewPutError {}

#[derive(Deserialize)]
struct SummaryWire {
    count: i64,
    total_bytes: i64,
}

#[derive(Deserialize)]
struct MediaWire {
    /// `null` or absent is not "no files": the read is refused as unreadable.
    files: Option<Vec<MediaFileWire>>,
}

#[derive(Deserialize)]
struct MediaFileWire {
    id: String,
    #[serde(default)]
    is_folder: bool,
    #[serde(default)]
    has_preview: bool,
    preview_format: Option<String>,
    preview_px: Option<i64>,
    preview_bytes: Option<i64>,
    /// RFC 3339 text, or unix seconds.
    updated_at: Option<serde_json::Value>,
    size_bytes: Option<i64>,
}

/// Unix seconds of a timestamp field: a number, or RFC 3339 text.
fn seconds_of(value: &serde_json::Value) -> Option<i64> {
    value.as_i64().or_else(|| {
        value
            .as_str()
            .and_then(|text| chrono::DateTime::parse_from_rfc3339(text).ok())
            .map(|at| at.timestamp())
    })
}

const PREFERENCE_PATH: &str = "/api/v1/preferences/photo_previews";

impl ApiClient {
    /// The account's preview setting. An unset key reads as the server's default; each field that is missing or
    /// not one of its values falls back to its own default. A failed read is an `Err`, never the default.
    pub async fn photo_previews_setting(&self) -> ApiResult<PreviewSetting> {
        let body: serde_json::Value = self.get_typed(PREFERENCE_PATH).await?;
        let value = body.get("value").unwrap_or(&serde_json::Value::Null);
        Ok(PreviewSetting::from_value(value))
    }

    /// Store the account's preview setting (the server replaces the whole value).
    pub async fn set_photo_previews_setting(&self, setting: &PreviewSetting) -> ApiResult<()> {
        let url = self.url(PREFERENCE_PATH);
        let body = serde_json::json!({ "value": setting.to_value() });
        self.send_with_retry(|| self.client.put(&url).json(&body)).await?;
        Ok(())
    }

    /// What the account's stored previews take: how many, and their bytes.
    pub async fn previews_summary(&self) -> ApiResult<PreviewsSummary> {
        let wire: SummaryWire = self.get_typed("/api/v1/files/previews/summary").await?;
        let positive = |n: i64| u64::try_from(n).unwrap_or(0);
        Ok(PreviewsSummary {
            count: positive(wire.count),
            total_bytes: positive(wire.total_bytes),
        })
    }

    /// The preview facts of every one of the account's own live media files.
    pub async fn preview_states(&self) -> ApiResult<Vec<PreviewState>> {
        let wire: MediaWire = self.get_typed("/api/v1/files/media").await?;
        let files = wire.files.ok_or(DesktopApiError::InvalidCountResponse)?;
        Ok(files
            .into_iter()
            .filter(|file| !file.is_folder)
            .map(|file| PreviewState {
                id: file.id,
                has_preview: file.has_preview,
                format: file.preview_format.as_deref().and_then(PreviewFormat::from_wire),
                px: file.preview_px.and_then(|px| u32::try_from(px).ok()),
                bytes: file.preview_bytes.and_then(|bytes| u64::try_from(bytes).ok()),
                updated_at: file.updated_at.as_ref().and_then(seconds_of),
                size: file.size_bytes.and_then(|size| u64::try_from(size).ok()),
            })
            .collect())
    }

    /// Store one sealed JPEG preview of the file `file_id` at the long edge `px` (one PUT replaces the file's old
    /// preview). `sealed` goes as it is: the caller sealed it. ONE attempt: a 429 is handed back with the server's
    /// wait ([`PreviewPutError::RateLimited`]), so the caller asks its session again before it sends again. The PUT
    /// may take a minute plus the time the slowest allowed link needs for its body.
    pub async fn upload_preview(&self, file_id: &str, px: u32, sealed: &[u8]) -> Result<(), PreviewPutError> {
        let url = format!(
            "{}/api/v1/files/{}/thumbnail/preview?format=jpeg&px={px}",
            self.base_url,
            encode(file_id)
        );
        let response = self
            .client
            .put(url)
            .timeout(super::direct::storage_timeout(sealed.len() as u64))
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Content-Type", "application/octet-stream")
            .body(sealed.to_vec())
            .send()
            .await
            // The transport error names the URL (the file's id): only its kind leaves.
            .map_err(|_| PreviewPutError::Offline)?;
        let status = response.status();
        if status.is_success() {
            return Ok(());
        }
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(PreviewPutError::RateLimited {
                retry_after: header_secs(response.headers(), reqwest::header::RETRY_AFTER.as_str()),
            });
        }
        if status == reqwest::StatusCode::PAYLOAD_TOO_LARGE {
            // Only the body's code is read, never its words.
            let code = response
                .json::<serde_json::Value>()
                .await
                .ok()
                .and_then(|body| body.get("error").and_then(|code| code.as_str()).map(str::to_string));
            return Err(if code.as_deref() == Some("quota_exceeded") {
                PreviewPutError::QuotaExceeded
            } else {
                PreviewPutError::TooLarge
            });
        }
        Err(PreviewPutError::Status(status.as_u16()))
    }
}

/// How long to wait before sending a preview again that was answered 429 (`None`: give up on it): the server's hint
/// within the client's bounded budget, the rule every retried request follows. `attempt`: the retries already made;
/// `spent`: the seconds already waited for this preview.
pub fn preview_retry_wait(attempt: u32, spent: u64, retry_after: Option<u64>) -> Option<u64> {
    retry_decision(attempt, spent, retry_after, None)
}
