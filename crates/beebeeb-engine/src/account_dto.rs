//! Typed DTOs for the desktop **data layer** — the read-only account,
//! billing, devices, sessions, activity, and notifications views the
//! frontend renders.
//!
//! ## Why these exist
//!
//! The desktop UI ("all pages empty") was calling almost none of the
//! server endpoints that already exist. These structs mirror the exact
//! JSON shapes returned by the public API so the
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
//!
//! ## Layout
//!
//! - `profile` — account profile, subscription, billing usage
//! - `region` — region choices and storage location
//! - `activity` — account activity and security score
//! - `sessions` — account sessions, devices, client sync sessions
//! - `notifications` — audit-log feed and notification preferences
//! - `storage` — local storage breakdown by extension
//! - `overview` — per-device file overview computed from the mirror
//!
//! The DTO tests live in `tests/`, one module per area above.

use serde::{Deserialize, Serialize};

mod profile;
/// Tests only: the profile tests pin the impersonation enum's variants.
#[cfg(any(test, feature = "test-support"))]
pub use profile::AccountAccess;
pub use profile::{AccountProfile, BillingUsage, Subscription};

mod region;
pub use region::{SetPreferredRegionResponse, UserRegionResponse};

mod activity;
pub use activity::{AccountActivity, SecurityScore};

mod sessions;
pub use sessions::{AccountSession, AccountSessionList, ClientDeviceList, ClientSyncSessionList, RevokeAllResult};

mod notifications;
pub use notifications::{
    ActivityFeed, ActivityFeedEvent, Notification, NotificationList, NotificationPreferenceValues,
    NotificationPreferences, NotificationPreferencesUpdate,
};

mod storage;
/// Tests only: the storage tests classify extensions against the category enum.
#[cfg(any(test, feature = "test-support"))]
pub use storage::StorageCategory;
pub use storage::{BreakdownInput, StorageBreakdown, compute_storage_breakdown};

mod overview;
pub use overview::{
    ConfirmActionResult, DesktopTrashItem, FileOverview, FileOverviewActivityInput, FileOverviewInput, RecentFile,
    compute_file_overview,
};

#[cfg(test)]
mod tests {
    use super::*;

    mod profile_tests;

    mod region_tests;

    mod account_tests;

    mod notifications_tests;

    mod storage_tests;

    mod overview_tests;
}
