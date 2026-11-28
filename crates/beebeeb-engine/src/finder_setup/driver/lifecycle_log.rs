//! The lifecycle log's event type and token filter, as the driver uses them. The file-backed log
//! (rotation, redaction, the global `tail`) is dm-23's; the driver reaches it through `Ports::log`
//! and `copy_details_from`'s `tail` argument.

use super::super::core::Trigger;
use super::super::error::FpError;
use super::super::launch_location::LaunchLocation;
use crate::surfaces::phase::{FinderFailureReason, FinderSetup};

pub const TAIL_LINES: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LifecycleEvent {
    Launch {
        app_version: String,
        macos_version: String,
        launch_location: LaunchLocation,
    },
    Trigger(Trigger),
    Transition {
        from: FinderSetup,
        to: FinderSetup,
        reason: Option<FinderFailureReason>,
        error: Option<FpError>,
        attempt: u8,
        max_attempts: u8,
    },
    SignedIn,
    SignedOut,
    UpdateDownloaded { from: String, to: String },
    UpdateInstalled { from: String, to: String },
}

/// Versions and NSError domains pass a whitelist, so no free text slips in: `[A-Za-z0-9._-]`, at most 64 characters.
pub fn token(value: &str) -> String {
    value
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        .take(64)
        .collect()
}

/// The last `n` lines of the lifecycle log. The file-backed log is dm-23's; until it is wired this reads empty.
pub fn tail(_n: usize) -> Vec<String> {
    Vec::new()
}
