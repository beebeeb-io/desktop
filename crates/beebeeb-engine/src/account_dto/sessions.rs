//! Account sessions, devices and client sync-session DTOs.
//!
//! The account-session shape is richer than the auth router's session list;
//! sync sessions also carry heartbeat health fields.

use super::*;

// ── 4. Account sessions — GET/DELETE /api/v1/account/sessions ──────────────────
//
// Server: routes/account_activity.rs ~236-273 (list), ~280-299 (delete),
// ~306-327 (revoke-all-others). This is the ACCOUNT sessions router (richer
// shape with device_name/country/last_active), distinct from the auth
// router's /api/v1/auth/sessions.

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AccountSession {
    pub id: String,
    pub device_name: String,
    pub device_kind: String,
    #[serde(default)]
    pub country_code: Option<String>,
    /// RFC3339 or null.
    #[serde(default)]
    pub last_active_at: Option<String>,
    /// RFC3339, when the server sends one. The account-sessions list (`list_sessions`) selects
    /// only `token AS id, device_name, device_kind, country_code, last_active_at,
    /// first_seen_recognized` and adds `is_current`; it carries no `created_at`. Requiring it made
    /// the whole list fail to decode ("missing field `created_at`") and the pane showed no devices.
    #[serde(default)]
    pub created_at: String,
    pub is_current: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AccountSessionList {
    pub sessions: Vec<AccountSession>,
}

/// `POST /api/v1/account/sessions/revoke-all-others` → `{ revoked: <count> }`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RevokeAllResult {
    pub revoked: i64,
}

// ── 5. Devices & client sync sessions — GET /api/v1/clients/* ──────────────────
//
// Server: routes/clients.rs ~75-103 (devices), ~282-326 + session_to_json
// ~598-617 (sessions). The session shape carries more than the brief
// listed (heartbeat_interval, alert_after_missed, device_id) — captured so
// the Devices view can show heartbeat health without a follow-up call.

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ClientDevice {
    pub id: String,
    pub hostname: String,
    pub platform: String,
    #[serde(default)]
    pub bb_version: Option<String>,
    /// RFC3339.
    pub last_seen: String,
    /// RFC3339.
    pub created_at: String,
    pub session_count: i64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ClientDeviceList {
    pub devices: Vec<ClientDevice>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ClientSyncSession {
    pub id: String,
    pub name: String,
    pub session_type: String,
    #[serde(default)]
    pub local_path: Option<String>,
    pub remote_path: String,
    pub status: String,
    #[serde(default)]
    pub heartbeat_interval_secs: Option<i32>,
    #[serde(default)]
    pub alert_after_missed: Option<i32>,
    /// RFC3339.
    pub created_at: String,
    /// RFC3339 or null.
    #[serde(default)]
    pub last_heartbeat: Option<String>,
    #[serde(default)]
    pub files_synced: Option<i64>,
    #[serde(default)]
    pub files_total: Option<i64>,
    #[serde(default)]
    pub bytes_synced: Option<i64>,
    #[serde(default)]
    pub bytes_total: Option<i64>,
    #[serde(default)]
    pub heartbeat_status: Option<String>,
    #[serde(default)]
    pub current_file: Option<String>,
    #[serde(default)]
    pub device_hostname: Option<String>,
    #[serde(default)]
    pub device_platform: Option<String>,
    #[serde(default)]
    pub device_id: Option<String>,
    /// Present only when the latest heartbeat row reported it.
    #[serde(default)]
    pub speed_bps: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ClientSyncSessionList {
    pub sessions: Vec<ClientSyncSession>,
}
