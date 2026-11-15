//! The sign-in activity log of the account (`GET /api/v1/me/activity`): whether the account records it, and the
//! events when it does. One request answers both, so a log that is off is never read as "nothing happened".

use super::*;

/// One event of the account's log, as Beebeeb sends it. The description is Beebeeb's own words for the event
/// type; no file or item name is part of it.
#[derive(Debug, Clone, Deserialize)]
pub struct MyActivityEvent {
    #[serde(rename = "type")]
    pub event_type: String,
    #[serde(default)]
    pub outcome: Option<String>,
    #[serde(default)]
    pub device: Option<String>,
    #[serde(default)]
    pub country_code: Option<String>,
    /// RFC 3339.
    pub created_at: String,
}

/// The log: `opted_in` false means the account records nothing (and `events` is empty).
#[derive(Debug, Clone, Deserialize)]
pub struct MyActivity {
    pub opted_in: bool,
    #[serde(default)]
    pub events: Vec<MyActivityEvent>,
}

impl ApiClient {
    /// `GET /api/v1/me/activity?limit=<limit>`: the newest events first, when the account records them.
    pub async fn my_activity(&self, limit: u32) -> anyhow::Result<MyActivity> {
        Ok(self.get_typed(&format!("/api/v1/me/activity?limit={limit}")).await?)
    }
}
