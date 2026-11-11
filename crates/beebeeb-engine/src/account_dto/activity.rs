//! Account activity feed and security-score DTOs.
//!
//! `AccountActivitySummary.security_score` is the label string; the numeric
//! score lives on the dedicated security-score response.

use super::*;

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
