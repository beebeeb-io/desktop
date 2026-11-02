//! The module's stable facts: the popover shown/hidden event names the window code
//! emits, the storage cache's TTL, wait and failure backoff (task 2179), and the
//! activity and conflict row limits.
//!
//! They live apart from the code that reads them so the UI's contract has one home:
//! the popover and the main window both use these names and numbers, and none of
//! them may drift into a second literal elsewhere.

use super::*;

/// Event Rust emits when the popover is shown; the UI refreshes its snapshot then.
/// Emitted by the window code (`macos_popover.rs`). The constant lives here so the
/// contract has one home.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub const POPOVER_SHOWN_EVENT: &str = "popover-shown";

/// Event Rust emits each time the popover is hidden; the UI stops reading its
/// snapshot until it is shown again (review P2-2 of #160: the window is hidden,
/// never destroyed).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub const POPOVER_HIDDEN_EVENT: &str = "popover-hidden";

/// How long a storage summary is trusted (spec section 9: "5 minutes, refetched
/// after unlock; never fetched on every open").
pub const STORAGE_TTL_SECS: i64 = 300;

/// The longest a snapshot waits for a storage fetch before it answers from the cache
/// (task 2179). The fetch's own timeout is 8 s and the main window reads a snapshot every
/// few seconds, so a slow API must not hold every read for 8 s.
pub const STORAGE_WAIT: std::time::Duration = std::time::Duration::from_secs(1);

/// After a failed fetch, snapshots answer from the cache for this long without asking the
/// API again (task 2179): an API that is down is not asked on every read.
pub const STORAGE_FAILURE_BACKOFF_SECS: i64 = 30;

/// How many activity rows a snapshot carries by default (the popover fits five).
pub const DEFAULT_ACTIVITY_LIMIT: usize = 8;

pub(super) const MAX_ACTIVITY_LIMIT: usize = 20;

/// How many conflicts the snapshot names (`Review` opens the first).
pub const CONFLICT_FILES_LIMIT: usize = 5;
