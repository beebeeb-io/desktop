//! Audit-log activity feed and notification DTOs.
//!
//! `NotificationPreferencesUpdate` serializes only the fields the caller set,
//! so a partial preference update never clears the others.

use super::*;

// ── 6. Activity feed — GET /api/v1/activity ───────────────────────────────────
//
// Server: routes/activity.rs ~38-147. Distinct from /account/activity: this
// is the paginated audit-log feed. The server splits the `target` column
// into `subject` + `details` and surfaces ip-or-device as `where`.

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ActivityFeedEvent {
    pub id: String,
    #[serde(rename = "type")]
    pub event_type: String,
    #[serde(default)]
    pub subject: Option<String>,
    #[serde(default)]
    pub details: Option<String>,
    /// ip_address if present, else device. Serialized as `where` on the wire.
    #[serde(rename = "where", default)]
    pub origin: Option<String>,
    /// RFC3339.
    pub created_at: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ActivityFeed {
    pub events: Vec<ActivityFeedEvent>,
    pub total: i64,
    #[serde(default)]
    pub page: Option<i64>,
}

// ── 7. Notifications — GET /api/v1/notifications + preferences ─────────────────
//
// Server: routes/notifications.rs ~193-271 (list), ~100-118 (get prefs),
// ~134-181 (put prefs).

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Notification {
    pub id: String,
    #[serde(rename = "type")]
    pub notification_type: String,
    pub title: String,
    #[serde(default)]
    pub body: Option<String>,
    /// Arbitrary JSON blob (JSONB column); may be null.
    #[serde(default)]
    pub data: Option<serde_json::Value>,
    pub read: bool,
    /// RFC3339.
    pub created_at: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct NotificationList {
    pub notifications: Vec<Notification>,
    pub unread_count: i64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct NotificationPreferenceValues {
    pub file_updated: bool,
    pub share_received: bool,
    pub storage_warning: bool,
    pub new_device_login: bool,
    pub backup_complete: bool,
}

/// Both GET and PUT /preferences wrap the values under a `preferences` key.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct NotificationPreferences {
    pub preferences: NotificationPreferenceValues,
}

/// Partial update body for PUT /notifications/preferences — omitted fields
/// keep their current server value. Serialized; `None` fields are skipped.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NotificationPreferencesUpdate {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_updated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub share_received: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub storage_warning: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_device_login: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup_complete: Option<bool>,
}
