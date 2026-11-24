//! Pinned folders: the account's `pinned_folders` preference (`GET/PUT /api/v1/preferences/pinned_folders`), the
//! same key and shape the web app keeps (`{ "folder_ids": [...] }`), so a pin made on either shows on the other.
//! Folders only; at most [`MAX_PINS`] (web's limit).

use super::{ApiClient, ApiResult};

const PREFERENCE_PATH: &str = "/api/v1/preferences/pinned_folders";

/// The most folders an account pins (the web app's own limit).
pub const MAX_PINS: usize = 10;

/// The folder ids a stored value holds, in order, once each. Reads `{ "folder_ids": [...] }` (what web writes) and
/// a bare array (the server's unset default is `[]`); anything else is no pins.
pub fn pinned_ids_of(value: &serde_json::Value) -> Vec<String> {
    let list = value.get("folder_ids").unwrap_or(value);
    let mut ids: Vec<String> = Vec::new();
    for id in list.as_array().into_iter().flatten().filter_map(|v| v.as_str()) {
        if !ids.iter().any(|seen| seen == id) {
            ids.push(id.to_string());
        }
    }
    ids
}

impl ApiClient {
    /// The account's pinned folder ids. A failed read is an `Err`, never "no pins" (a write after it would wipe them).
    pub async fn pinned_folders(&self) -> ApiResult<Vec<String>> {
        let body: serde_json::Value = self.get_typed(PREFERENCE_PATH).await?;
        Ok(pinned_ids_of(body.get("value").unwrap_or(&serde_json::Value::Null)))
    }

    /// Store the account's pinned folder ids (the server replaces the whole value).
    pub async fn set_pinned_folders(&self, ids: &[String]) -> ApiResult<()> {
        let url = self.url(PREFERENCE_PATH);
        let body = serde_json::json!({ "folder_ids": ids });
        self.send_with_retry(|| self.client.put(&url).json(&body)).await?;
        Ok(())
    }
}
