//! Account profile, subscription and billing-usage DTOs.
//!
//! `AccountProfile` mirrors `GET /api/v1/auth/me`; `Subscription` and
//! `BillingUsage` mirror `/api/v1/billing/*`. Both subscription branches
//! (paid row, free-user default) must decode, so dates are optional.

use super::*;

// ── 1. Account profile — GET /api/v1/auth/me ──────────────────────────────────
//
// Server: routes/auth.rs ~804-851 (built with `json!()`, no named struct).
// Fields beyond the four the UI needs (role, totp_enabled, frozen_at,
// is_impersonation, admin_user_id) are captured so the Account page can
// surface 2FA / freeze state without a second round-trip.

/// Who a profile describes. The wire spells impersonation as two optional
/// fields (`is_impersonation`, `admin_user_id`); they only mean one thing
/// together, so a caller matches this union once instead of checking two flags
/// in the right order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountAccess {
    /// The signed-in account itself.
    Own,
    /// An admin acting as another account. `admin_user_id` names the target; it
    /// is `None` only from a server that said "impersonation" without naming the
    /// account, and a caller then shows no target rather than the admin's own.
    Impersonated { admin_user_id: Option<String> },
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
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

impl AccountProfile {
    /// The one value the two impersonation optionals mean.
    pub fn access(&self) -> AccountAccess {
        match self.is_impersonation {
            Some(true) => AccountAccess::Impersonated {
                admin_user_id: self.admin_user_id.clone(),
            },
            _ => AccountAccess::Own,
        }
    }
}

// ── 2. Billing — GET /api/v1/billing/subscription + /usage ────────────────────
//
// Server: routes/billing.rs ~214-292 (subscription), ~192-203 (usage).
// The subscription endpoint has two branches: a paid/mock row and a
// free-user default. The free branch sets created_at / current_period_end
// to null, so both are Option. quota_bytes/used_bytes are i64; percentage
// is an f64 ratio (0.0..=1.0).
//
// The billing-provider configuration flag the server used to send alongside
// the plan is gone (task 2285): the server no longer emits it and this DTO
// carries no field for it.

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
    pub billing_state: Option<String>,
    /// RFC3339 timestamp the account first went past-due, or null. Lets the
    /// billing/account view surface an overdue-payment state.
    #[serde(default)]
    pub past_due_since: Option<String>,
    #[serde(default)]
    pub pending_downgrade_plan: Option<String>,
    #[serde(default)]
    pub downgrade_cooldown_until: Option<String>,
    #[serde(default)]
    pub storage_grace_deadline: Option<String>,
    #[serde(default)]
    pub pause_until: Option<String>,
    /// Server #422: `none` | `trial` | `free` | `active` | `lapsed`. Absent from an older server (the gate then reads
    /// the plan and quota as before).
    #[serde(default)]
    pub plan_state: Option<String>,
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
