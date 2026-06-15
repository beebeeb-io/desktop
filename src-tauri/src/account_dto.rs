//! Typed DTOs for the desktop **data layer** — the read-only account,
//! billing, devices, sessions, activity, and notifications views the
//! frontend renders.
//!
//! ## Why these exist
//!
//! The desktop UI ("all pages empty") was calling almost none of the
//! server endpoints that already exist. These structs mirror the exact
//! JSON shapes returned by `repos/server/beebeeb-api/src/routes/*` so the
//! frontend can `invoke()` a Tauri command and get back a typed payload
//! instead of an empty placeholder.
//!
//! ## Shape rules (verified against the server, NOT guessed)
//!
//! - Every timestamp is an **RFC3339 string** on the wire (the server
//!   calls `.to_rfc3339()` or lets serde_json's chrono integration emit
//!   RFC3339). We keep them as `String` here — the frontend formats them.
//! - Nullable columns are `Option<T>`. The free-plan branch of the
//!   subscription endpoint, in particular, returns `created_at: null` and
//!   `current_period_end: null`, so those MUST be `Option`.
//! - Unknown/extra server fields are tolerated: serde's default behaviour
//!   ignores fields we don't name, so the server adding a column never
//!   breaks deserialization here.
//!
//! Each DTO is `Deserialize` (parse the server response) + `Serialize`
//! (hand it to the WebView across the Tauri IPC boundary) + `Clone`.

use serde::{Deserialize, Serialize};

// ── 1. Account profile — GET /api/v1/auth/me ──────────────────────────────────
//
// Server: routes/auth.rs ~804-851 (built with `json!()`, no named struct).
// Fields beyond the four the UI needs (role, totp_enabled, frozen_at,
// is_impersonation, admin_user_id) are captured so the Account page can
// surface 2FA / freeze state without a second round-trip.

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AccountProfile {
    pub user_id: String,
    pub email: String,
    pub email_verified: bool,
    /// RFC3339.
    pub created_at: String,
    /// RFC3339 or null — non-null only for accounts pending deletion / frozen.
    #[serde(default)]
    pub frozen_at: Option<String>,
    /// "user" | "admin" | "superadmin". Optional defensively in case an
    /// older server build omits it.
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub totp_enabled: Option<bool>,
    #[serde(default)]
    pub is_impersonation: Option<bool>,
    /// Set only inside an admin impersonation session.
    #[serde(default)]
    pub admin_user_id: Option<String>,
}

// ── 2. Billing — GET /api/v1/billing/subscription + /usage ────────────────────
//
// Server: routes/billing.rs ~214-292 (subscription), ~192-203 (usage).
// The subscription endpoint has two branches: a paid/mock row and a
// free-user default. The free branch sets created_at / current_period_end
// to null, so both are Option. quota_bytes/used_bytes are i64; percentage
// is an f64 ratio (0.0..=1.0).

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Subscription {
    pub plan: String,
    pub billing_cycle: String,
    pub status: String,
    pub seats: i32,
    pub region: String,
    /// RFC3339; null on the free-plan default branch.
    #[serde(default)]
    pub created_at: Option<String>,
    /// RFC3339; null on the free-plan default branch.
    #[serde(default)]
    pub current_period_end: Option<String>,
    pub quota_bytes: i64,
    pub used_bytes: i64,
    #[serde(default)]
    pub extra_storage_tb: i32,
    #[serde(default)]
    pub extra_users: i32,
    #[serde(default)]
    pub is_mock: Option<bool>,
    #[serde(default)]
    pub stripe_configured: Option<bool>,
    #[serde(default)]
    pub billing_state: Option<String>,
    #[serde(default)]
    pub pending_downgrade_plan: Option<String>,
    #[serde(default)]
    pub downgrade_cooldown_until: Option<String>,
    #[serde(default)]
    pub storage_grace_deadline: Option<String>,
    #[serde(default)]
    pub pause_until: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BillingUsage {
    pub used_bytes: i64,
    pub quota_bytes: i64,
    /// Ratio 0.0..=1.0 (0.0 for an unlimited plan). The desktop previously
    /// ignored this; now exposed so the UI can render a usage bar without
    /// recomputing.
    pub percentage: f64,
}

// ── 3. Account activity & security score ──────────────────────────────────────
//
// Server: routes/account_activity.rs ~39-138 (activity), ~144-229 (score).
// NOTE: the activity summary's `security_score` is the LABEL STRING
// ("Excellent" | "Strong" | ...), not the numeric score. The numeric
// 0..=5 score lives on the dedicated security-score endpoint.

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AccountActivityEvent {
    pub id: String,
    /// Audit event name, e.g. "auth.login.success".
    #[serde(rename = "type")]
    pub event_type: String,
    pub description: String,
    pub category: String,
    pub outcome: String,
    #[serde(default)]
    pub device: Option<String>,
    #[serde(default)]
    pub country_code: Option<String>,
    /// RFC3339.
    pub created_at: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AccountActivitySummary {
    /// RFC3339 or null.
    #[serde(default)]
    pub last_login_at: Option<String>,
    #[serde(default)]
    pub last_login_device: Option<String>,
    pub active_sessions: i64,
    pub active_shares: i64,
    /// Label string ("Excellent" | "Strong" | "Good" | "Basic" |
    /// "Vulnerable" | "unknown"), NOT the numeric score.
    pub security_score: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AccountActivity {
    pub events: Vec<AccountActivityEvent>,
    pub summary: AccountActivitySummary,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SecurityFactor {
    /// One of: "email_verified" | "phrase_saved" | "two_factor_enabled" |
    /// "phrase_recently_tested" | "all_devices_recognized".
    pub key: String,
    pub satisfied: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SecurityScore {
    /// 0..=5.
    pub score: u8,
    /// Always 5; optional defensively (the DB-error fallback omits it).
    #[serde(default)]
    pub max: Option<u8>,
    /// "Excellent" | "Strong" | "Good" | "Basic" | "Vulnerable" |
    /// "Unknown" (the last only on the DB-error fallback).
    pub label: String,
    pub factors: Vec<SecurityFactor>,
}

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
    /// RFC3339.
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

// ── 8. Storage breakdown — computed LOCALLY (no server endpoint) ───────────────
//
// The server has no user-facing storage-by-type endpoint. We compute the
// breakdown from the desktop SQLite mirror (`state_db::list_files`): bucket
// each file by extension into Media / Documents / Other, sum sizes, and
// surface the N largest files.

/// A coarse content category derived from a file's extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum StorageCategory {
    Media,
    Documents,
    Other,
}

impl StorageCategory {
    /// Classify by lowercase extension (no dot). Unknown / extensionless →
    /// `Other`. The lists are intentionally conservative — the goal is a
    /// useful breakdown, not MIME-perfect classification.
    pub fn from_extension(ext: &str) -> Self {
        const MEDIA: &[&str] = &[
            // images
            "jpg", "jpeg", "png", "gif", "bmp", "tiff", "tif", "webp", "heic", "heif", "svg", "ico",
            "raw", "cr2", "nef", "arw", "dng", "avif",
            // video
            "mp4", "mov", "avi", "mkv", "webm", "flv", "wmv", "m4v", "mpg", "mpeg", "3gp",
            // audio
            "mp3", "wav", "flac", "aac", "ogg", "m4a", "wma", "opus", "aiff", "alac",
        ];
        const DOCUMENTS: &[&str] = &[
            "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "odt", "ods", "odp", "rtf", "txt",
            "md", "csv", "tsv", "epub", "pages", "numbers", "key", "tex", "json", "xml", "yaml",
            "yml", "html", "htm",
        ];
        let ext = ext.to_ascii_lowercase();
        if MEDIA.contains(&ext.as_str()) {
            StorageCategory::Media
        } else if DOCUMENTS.contains(&ext.as_str()) {
            StorageCategory::Documents
        } else {
            StorageCategory::Other
        }
    }
}

/// Per-category roll-up.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StorageCategoryTotal {
    pub category: StorageCategory,
    pub bytes: i64,
    pub file_count: i64,
}

/// One of the N largest files in the local mirror.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LargestFile {
    pub file_id: String,
    pub path: String,
    pub size_bytes: i64,
    pub category: StorageCategory,
}

/// Result of [`compute_storage_breakdown`].
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StorageBreakdown {
    /// One entry per category (Media, Documents, Other), always all three
    /// present (a category with no files reports zero) so the UI can render
    /// a stable legend.
    pub by_category: Vec<StorageCategoryTotal>,
    /// Largest files, descending by size, capped at the requested limit.
    pub largest_files: Vec<LargestFile>,
    /// Total file count (folders excluded).
    pub total_files: i64,
    /// Sum of all file sizes (folders excluded).
    pub total_bytes: i64,
}

/// A minimal view of a mirrored file for the breakdown computation. Decoupled
/// from `state_db::FileEntry` so the pure computation is unit-testable
/// without a SQLite handle (the caller maps `FileEntry` → this, skipping
/// folders).
#[derive(Debug, Clone)]
pub struct BreakdownInput {
    pub file_id: String,
    pub path: String,
    pub size_bytes: i64,
}

/// Pure storage-breakdown computation. `files` should already exclude
/// folders. `largest_limit` caps the `largest_files` list. Negative sizes
/// are clamped to 0.
pub fn compute_storage_breakdown(files: &[BreakdownInput], largest_limit: usize) -> StorageBreakdown {
    let mut media = StorageCategoryTotal {
        category: StorageCategory::Media,
        bytes: 0,
        file_count: 0,
    };
    let mut documents = StorageCategoryTotal {
        category: StorageCategory::Documents,
        bytes: 0,
        file_count: 0,
    };
    let mut other = StorageCategoryTotal {
        category: StorageCategory::Other,
        bytes: 0,
        file_count: 0,
    };

    let mut total_bytes: i64 = 0;
    let mut total_files: i64 = 0;
    let mut ranked: Vec<LargestFile> = Vec::with_capacity(files.len());

    for f in files {
        let size = f.size_bytes.max(0);
        let category = StorageCategory::from_extension(&extension_of(&f.path));
        let bucket = match category {
            StorageCategory::Media => &mut media,
            StorageCategory::Documents => &mut documents,
            StorageCategory::Other => &mut other,
        };
        bucket.bytes = bucket.bytes.saturating_add(size);
        bucket.file_count += 1;

        total_bytes = total_bytes.saturating_add(size);
        total_files += 1;

        ranked.push(LargestFile {
            file_id: f.file_id.clone(),
            path: f.path.clone(),
            size_bytes: size,
            category,
        });
    }

    ranked.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes).then_with(|| a.path.cmp(&b.path)));
    ranked.truncate(largest_limit);

    StorageBreakdown {
        by_category: vec![media, documents, other],
        largest_files: ranked,
        total_files,
        total_bytes,
    }
}

/// Lowercase extension (no dot) of a path, or "" if none. Handles paths with
/// no extension and dotfiles (".gitignore" → "" — a leading-dot-only name has
/// no extension).
fn extension_of(path: &str) -> String {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    match name.rsplit_once('.') {
        // Leading-dot file like ".gitignore": the part before the dot is empty
        // → treat as no extension.
        Some((stem, ext)) if !stem.is_empty() => ext.to_ascii_lowercase(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_profile_parses_full_shape() {
        let json = r#"{
            "user_id": "11111111-1111-1111-1111-111111111111",
            "email": "guus@example.com",
            "email_verified": true,
            "created_at": "2026-01-02T03:04:05Z",
            "frozen_at": null,
            "role": "user",
            "totp_enabled": false,
            "is_impersonation": false,
            "admin_user_id": null
        }"#;
        let p: AccountProfile = serde_json::from_str(json).unwrap();
        assert_eq!(p.email, "guus@example.com");
        assert!(p.email_verified);
        assert_eq!(p.created_at, "2026-01-02T03:04:05Z");
        assert_eq!(p.frozen_at, None);
        assert_eq!(p.role.as_deref(), Some("user"));
        // Round-trip back out to the WebView.
        let back = serde_json::to_string(&p).unwrap();
        assert!(back.contains("\"email_verified\":true"));
    }

    #[test]
    fn account_profile_parses_minimal_shape() {
        // An older server build that only emits the four core fields.
        let json = r#"{
            "user_id": "u",
            "email": "a@b.c",
            "email_verified": false,
            "created_at": "2026-01-01T00:00:00Z"
        }"#;
        let p: AccountProfile = serde_json::from_str(json).unwrap();
        assert_eq!(p.role, None);
        assert_eq!(p.totp_enabled, None);
    }

    #[test]
    fn subscription_paid_branch_parses() {
        let json = r#"{
            "plan": "pro",
            "billing_cycle": "yearly",
            "status": "active",
            "seats": 1,
            "region": "falkenstein",
            "is_mock": false,
            "stripe_configured": true,
            "created_at": "2026-01-01T00:00:00Z",
            "current_period_end": "2027-01-01T00:00:00Z",
            "quota_bytes": 2199023255552,
            "used_bytes": 12345,
            "extra_storage_tb": 1,
            "extra_users": 0,
            "billing_state": "active",
            "past_due_since": null,
            "pending_downgrade_plan": null,
            "downgrade_cooldown_until": null,
            "storage_grace_deadline": null,
            "pause_until": null
        }"#;
        let s: Subscription = serde_json::from_str(json).unwrap();
        assert_eq!(s.plan, "pro");
        assert_eq!(s.current_period_end.as_deref(), Some("2027-01-01T00:00:00Z"));
        assert_eq!(s.quota_bytes, 2199023255552);
        assert_eq!(s.extra_storage_tb, 1);
    }

    #[test]
    fn subscription_free_branch_has_null_dates() {
        let json = r#"{
            "plan": "free",
            "billing_cycle": "monthly",
            "seats": 1,
            "region": "falkenstein",
            "status": "active",
            "is_mock": true,
            "stripe_configured": false,
            "created_at": null,
            "current_period_end": null,
            "quota_bytes": 10737418240,
            "used_bytes": 0,
            "extra_storage_tb": 0,
            "extra_users": 0
        }"#;
        let s: Subscription = serde_json::from_str(json).unwrap();
        assert_eq!(s.plan, "free");
        assert_eq!(s.created_at, None);
        assert_eq!(s.current_period_end, None);
        assert_eq!(s.quota_bytes, 10737418240);
    }

    #[test]
    fn billing_usage_parses_percentage() {
        let json = r#"{ "used_bytes": 500, "quota_bytes": 1000, "percentage": 0.5 }"#;
        let u: BillingUsage = serde_json::from_str(json).unwrap();
        assert_eq!(u.used_bytes, 500);
        assert!((u.percentage - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn account_activity_parses_events_and_summary() {
        let json = r#"{
            "events": [
                {
                    "id": "e1",
                    "type": "auth.login.success",
                    "description": "Signed in",
                    "category": "auth",
                    "outcome": "success",
                    "device": "Chrome on macOS",
                    "country_code": "NL",
                    "created_at": "2026-06-01T10:00:00Z"
                },
                {
                    "id": "e2",
                    "type": "file.upload",
                    "description": "Uploaded a file",
                    "category": "general",
                    "outcome": "success",
                    "device": null,
                    "country_code": null,
                    "created_at": "2026-06-01T11:00:00Z"
                }
            ],
            "summary": {
                "last_login_at": "2026-06-01T10:00:00Z",
                "last_login_device": "Chrome on macOS",
                "active_sessions": 2,
                "active_shares": 1,
                "security_score": "Strong"
            }
        }"#;
        let a: AccountActivity = serde_json::from_str(json).unwrap();
        assert_eq!(a.events.len(), 2);
        assert_eq!(a.events[0].event_type, "auth.login.success");
        assert_eq!(a.events[1].device, None);
        assert_eq!(a.summary.active_sessions, 2);
        // security_score is the LABEL string, not numeric.
        assert_eq!(a.summary.security_score, "Strong");
    }

    #[test]
    fn security_score_parses() {
        let json = r#"{
            "score": 4,
            "max": 5,
            "label": "Strong",
            "factors": [
                { "key": "email_verified", "satisfied": true },
                { "key": "two_factor_enabled", "satisfied": false }
            ]
        }"#;
        let s: SecurityScore = serde_json::from_str(json).unwrap();
        assert_eq!(s.score, 4);
        assert_eq!(s.max, Some(5));
        assert_eq!(s.label, "Strong");
        assert_eq!(s.factors.len(), 2);
        assert!(s.factors[0].satisfied);
        assert!(!s.factors[1].satisfied);
    }

    #[test]
    fn security_score_db_error_fallback_parses() {
        // The server's DB-error path omits `max` and uses label "Unknown".
        let json = r#"{ "score": 0, "label": "Unknown", "factors": [] }"#;
        let s: SecurityScore = serde_json::from_str(json).unwrap();
        assert_eq!(s.score, 0);
        assert_eq!(s.max, None);
        assert!(s.factors.is_empty());
    }

    #[test]
    fn account_sessions_parse() {
        let json = r#"{
            "sessions": [
                {
                    "id": "s1",
                    "device_name": "MacBook Pro",
                    "device_kind": "desktop",
                    "country_code": "NL",
                    "last_active_at": "2026-06-01T12:00:00Z",
                    "created_at": "2026-05-01T09:00:00Z",
                    "is_current": true
                },
                {
                    "id": "s2",
                    "device_name": "Unknown device",
                    "device_kind": "web",
                    "country_code": null,
                    "last_active_at": null,
                    "created_at": "2026-05-02T09:00:00Z",
                    "is_current": false
                }
            ]
        }"#;
        let list: AccountSessionList = serde_json::from_str(json).unwrap();
        assert_eq!(list.sessions.len(), 2);
        assert!(list.sessions[0].is_current);
        assert_eq!(list.sessions[1].country_code, None);
        assert_eq!(list.sessions[1].last_active_at, None);
    }

    #[test]
    fn revoke_all_result_parses() {
        let json = r#"{ "revoked": 3 }"#;
        let r: RevokeAllResult = serde_json::from_str(json).unwrap();
        assert_eq!(r.revoked, 3);
    }

    #[test]
    fn devices_parse() {
        let json = r#"{
            "devices": [
                {
                    "id": "d1",
                    "hostname": "guus-mbp",
                    "platform": "macos",
                    "bb_version": "0.3.1",
                    "last_seen": "2026-06-01T12:00:00Z",
                    "created_at": "2026-05-01T09:00:00Z",
                    "session_count": 2
                },
                {
                    "id": "d2",
                    "hostname": "guus-pc",
                    "platform": "windows",
                    "bb_version": null,
                    "last_seen": "2026-06-02T12:00:00Z",
                    "created_at": "2026-05-10T09:00:00Z",
                    "session_count": 0
                }
            ]
        }"#;
        let list: ClientDeviceList = serde_json::from_str(json).unwrap();
        assert_eq!(list.devices.len(), 2);
        assert_eq!(list.devices[0].session_count, 2);
        assert_eq!(list.devices[1].bb_version, None);
    }

    #[test]
    fn client_sync_sessions_parse() {
        let json = r#"{
            "sessions": [
                {
                    "id": "cs1",
                    "name": "Documents sync",
                    "session_type": "sync",
                    "local_path": "/Users/guus/Beebeeb",
                    "remote_path": "/",
                    "status": "active",
                    "heartbeat_interval_secs": 60,
                    "alert_after_missed": 5,
                    "created_at": "2026-05-01T09:00:00Z",
                    "last_heartbeat": "2026-06-01T12:00:00Z",
                    "files_synced": 100,
                    "files_total": 120,
                    "bytes_synced": 1000,
                    "bytes_total": 2000,
                    "heartbeat_status": "healthy",
                    "current_file": "report.pdf",
                    "device_hostname": "guus-mbp",
                    "device_platform": "macos",
                    "device_id": "d1",
                    "speed_bps": 5000000
                }
            ]
        }"#;
        let list: ClientSyncSessionList = serde_json::from_str(json).unwrap();
        assert_eq!(list.sessions.len(), 1);
        let s = &list.sessions[0];
        assert_eq!(s.local_path.as_deref(), Some("/Users/guus/Beebeeb"));
        assert_eq!(s.files_synced, Some(100));
        assert_eq!(s.speed_bps, Some(5000000));
    }

    #[test]
    fn client_sync_session_tolerates_missing_speed_bps() {
        // speed_bps is absent if the heartbeat try_get fails server-side.
        let json = r#"{
            "sessions": [
                {
                    "id": "cs1",
                    "name": "s",
                    "session_type": "sync",
                    "remote_path": "/",
                    "status": "paused",
                    "created_at": "2026-05-01T09:00:00Z",
                    "device_hostname": "h",
                    "device_platform": "linux",
                    "device_id": "d"
                }
            ]
        }"#;
        let list: ClientSyncSessionList = serde_json::from_str(json).unwrap();
        assert_eq!(list.sessions[0].speed_bps, None);
        assert_eq!(list.sessions[0].local_path, None);
    }

    #[test]
    fn activity_feed_parses() {
        let json = r#"{
            "events": [
                {
                    "id": "a1",
                    "type": "file.upload",
                    "subject": "report.pdf",
                    "details": "12 MB",
                    "where": "203.0.113.7",
                    "created_at": "2026-06-01T10:00:00Z"
                },
                {
                    "id": "a2",
                    "type": "auth.login.success",
                    "subject": null,
                    "details": null,
                    "where": null,
                    "created_at": "2026-06-01T09:00:00Z"
                }
            ],
            "total": 42,
            "page": 1
        }"#;
        let feed: ActivityFeed = serde_json::from_str(json).unwrap();
        assert_eq!(feed.total, 42);
        assert_eq!(feed.page, Some(1));
        assert_eq!(feed.events[0].event_type, "file.upload");
        assert_eq!(feed.events[0].origin.as_deref(), Some("203.0.113.7"));
        assert_eq!(feed.events[1].subject, None);
    }

    #[test]
    fn notifications_parse() {
        let json = r#"{
            "notifications": [
                {
                    "id": "n1",
                    "type": "welcome",
                    "title": "Welcome to Beebeeb",
                    "body": "Your vault is ready",
                    "data": { "k": "v" },
                    "read": false,
                    "created_at": "2026-06-01T10:00:00Z"
                }
            ],
            "unread_count": 1
        }"#;
        let list: NotificationList = serde_json::from_str(json).unwrap();
        assert_eq!(list.unread_count, 1);
        assert_eq!(list.notifications[0].notification_type, "welcome");
        assert_eq!(list.notifications[0].body.as_deref(), Some("Your vault is ready"));
        assert!(list.notifications[0].data.is_some());
    }

    #[test]
    fn notification_preferences_parse_and_serialize_partial() {
        let json = r#"{
            "preferences": {
                "file_updated": true,
                "share_received": true,
                "storage_warning": true,
                "new_device_login": true,
                "backup_complete": false
            }
        }"#;
        let prefs: NotificationPreferences = serde_json::from_str(json).unwrap();
        assert!(prefs.preferences.file_updated);
        assert!(!prefs.preferences.backup_complete);

        // Partial update only serializes the fields that are Some.
        let update = NotificationPreferencesUpdate {
            backup_complete: Some(true),
            ..Default::default()
        };
        let body = serde_json::to_string(&update).unwrap();
        assert_eq!(body, r#"{"backup_complete":true}"#);
    }

    #[test]
    fn storage_category_classifies_by_extension() {
        assert_eq!(StorageCategory::from_extension("JPG"), StorageCategory::Media);
        assert_eq!(StorageCategory::from_extension("mp4"), StorageCategory::Media);
        assert_eq!(StorageCategory::from_extension("pdf"), StorageCategory::Documents);
        assert_eq!(StorageCategory::from_extension("docx"), StorageCategory::Documents);
        assert_eq!(StorageCategory::from_extension("bin"), StorageCategory::Other);
        assert_eq!(StorageCategory::from_extension(""), StorageCategory::Other);
    }

    #[test]
    fn extension_of_handles_edge_cases() {
        assert_eq!(extension_of("/a/b/c.PDF"), "pdf");
        assert_eq!(extension_of("photo.jpeg"), "jpeg");
        assert_eq!(extension_of("noext"), "");
        assert_eq!(extension_of("/path/.gitignore"), "");
        assert_eq!(extension_of("archive.tar.gz"), "gz");
        assert_eq!(extension_of("C:\\Users\\g\\file.txt"), "txt");
    }

    #[test]
    fn storage_breakdown_computes_buckets_and_largest() {
        let files = vec![
            BreakdownInput {
                file_id: "1".into(),
                path: "/photos/a.jpg".into(),
                size_bytes: 3000,
            },
            BreakdownInput {
                file_id: "2".into(),
                path: "/docs/report.pdf".into(),
                size_bytes: 5000,
            },
            BreakdownInput {
                file_id: "3".into(),
                path: "/misc/blob.bin".into(),
                size_bytes: 1000,
            },
            BreakdownInput {
                file_id: "4".into(),
                path: "/photos/b.png".into(),
                size_bytes: 2000,
            },
            // Negative size is clamped to 0.
            BreakdownInput {
                file_id: "5".into(),
                path: "/weird/neg.dat".into(),
                size_bytes: -10,
            },
        ];
        let b = compute_storage_breakdown(&files, 3);

        assert_eq!(b.total_files, 5);
        assert_eq!(b.total_bytes, 3000 + 5000 + 1000 + 2000);

        let media = b
            .by_category
            .iter()
            .find(|c| c.category == StorageCategory::Media)
            .unwrap();
        assert_eq!(media.bytes, 5000); // 3000 + 2000
        assert_eq!(media.file_count, 2);

        let docs = b
            .by_category
            .iter()
            .find(|c| c.category == StorageCategory::Documents)
            .unwrap();
        assert_eq!(docs.bytes, 5000);
        assert_eq!(docs.file_count, 1);

        let other = b
            .by_category
            .iter()
            .find(|c| c.category == StorageCategory::Other)
            .unwrap();
        assert_eq!(other.bytes, 1000); // blob.bin + neg.dat(clamped 0)
        assert_eq!(other.file_count, 2);

        // Largest three, descending: report.pdf(5000), a.jpg(3000), b.png(2000).
        assert_eq!(b.largest_files.len(), 3);
        assert_eq!(b.largest_files[0].path, "/docs/report.pdf");
        assert_eq!(b.largest_files[0].size_bytes, 5000);
        assert_eq!(b.largest_files[1].size_bytes, 3000);
        assert_eq!(b.largest_files[2].size_bytes, 2000);
    }

    #[test]
    fn storage_breakdown_all_categories_present_when_empty() {
        let b = compute_storage_breakdown(&[], 5);
        assert_eq!(b.total_files, 0);
        assert_eq!(b.total_bytes, 0);
        assert_eq!(b.by_category.len(), 3);
        assert!(b.largest_files.is_empty());
    }
}
