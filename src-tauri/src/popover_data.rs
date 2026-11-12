//! The data the menu-bar popover reads (task 1683 slice 2; spec
//! `docs/specs/2026-09-30-macos-menubar-popover.md` section 9).
//!
//! One snapshot command returns everything the popover's first frame needs, so the
//! React side (slice 3) makes one `invoke` when it is shown instead of seven. This
//! module is the pure half: given what the command gathered (the engine's last
//! status, the state DB's queue and activity, the account, the Finder state, the
//! cached storage summary) it assembles the snapshot DTO, including the phase from
//! `surfaces::phase::popover_phase`. The gathering, which touches the keychain, the
//! network and the file-provider probe, is `popover_snapshot` in `lib.rs`.
//!
//! ## Who reads it, and how often
//!
//! Nothing here runs on a timer. The popover calls the snapshot when it is shown
//! (`popover-shown`) and after its own actions; the main window (task 1683 slice 1)
//! reads it a few seconds after each read while it is open. The storage summary is
//! fetched only when the cached one is older than [`STORAGE_TTL_SECS`] (or from
//! another session), and a snapshot never waits for that fetch longer than
//! [`STORAGE_WAIT`] (task 2179): a slow API answers from the cache while the fetch
//! finishes in the background, and a failed fetch is not tried again for
//! [`STORAGE_FAILURE_BACKOFF_SECS`].
//!
//! ## What is deliberately not here
//!
//! - The window. Nothing in this module shows, hides or creates one.
//! - Text. The snapshot carries facts and stable codes; the words are the UI's
//!   (spec section 4), except the two mono lines whose content is data
//!   (`reason.detail`, `finder.reason_line`).

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::engine_status::{EngineStatusView, SharedStatusView};
use crate::state_db::{FileEntry, TransferActivity, TransferBacklog};
use crate::surfaces::phase::{FinderFailureReason, FinderSetup, PopoverPhase, PopoverSnapshot, popover_phase};
use crate::transfer_progress::{Direction, Transfer, TransferBoard, display_name, parent_folder};

// ── Cross-platform flyout state (moved from macos_popover.rs) ──────────────

/// An available update, shown in the flyout's badge and the Beebeeb window's banner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct UpdateOffer {
    pub channel: String,
    pub version: String,
}

/// Progress while sign-out sends the last changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct Sending {
    pub changes_left: usize,
}

/// The count the popover shows while sign-out sends the last changes. A read that fails leaves the count out for that
/// tick (`progressLine` shows none for 0), never the whole snapshot of the popover and the Beebeeb window (review P3-5
/// of #160).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn changes_left(read: Result<crate::sign_out::waiting::WaitingChanges, String>) -> usize {
    match read {
        Ok(waiting) => {
            let counts = waiting.counts();
            counts.files.saturating_add(counts.others)
        }
        Err(_) => {
            tracing::warn!("could not count the changes sign-out is still sending");
            0
        }
    }
}

static UPDATE: Mutex<Option<(String, String)>> = Mutex::new(None);

pub(crate) fn note_update(channel: &str, version: Option<&str>) {
    *UPDATE.lock().unwrap_or_else(|p| p.into_inner()) = version.map(|v| (channel.to_owned(), v.to_owned()));
}

pub(crate) fn update_offer() -> Option<UpdateOffer> {
    let channel = crate::configured_release_channel();
    UPDATE
        .lock()
        .ok()?
        .as_ref()
        .filter(|(c, _)| c == channel.as_str())
        .map(|(channel, version)| UpdateOffer {
            channel: channel.clone(),
            version: version.clone(),
        })
}

// ── Shared runtime state ────────────────────────────────────────────────────

// ── Storage summary, cached ─────────────────────────────────────────────────

// ── Finder ──────────────────────────────────────────────────────────────────

// ── The snapshot ────────────────────────────────────────────────────────────

// ── The review window's retarget event ──────────────────────────────────────

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;

mod constants;
use constants::*;
pub use constants::{
    CONFLICT_FILES_LIMIT, DEFAULT_ACTIVITY_LIMIT, POPOVER_HIDDEN_EVENT, STORAGE_FAILURE_BACKOFF_SECS, STORAGE_TTL_SECS,
    STORAGE_WAIT,
};
// Emitted by the macOS popover window code (was in `macos_popover.rs`, deleted).
#[cfg(target_os = "macos")]
pub use constants::POPOVER_SHOWN_EVENT;

mod runtime;
pub use runtime::PopoverRuntime;

mod storage;
pub use storage::{
    StorageCache, StorageDto, StorageUsage, cached_storage, clear_storage, fetch_storage_usage, storage_is_full,
};
// The seam a snapshot test fetches through (`cached_storage` is the caller's form).
#[cfg(test)]
pub use storage::cached_storage_with;

mod snapshot;
pub use snapshot::{DbView, PopoverSnapshotDto, SnapshotInputs, assemble, gather_db_view};
// The row/conflict shapes and builders are exercised from this module's tests.
#[cfg(test)]
pub use snapshot::{ActivityState, ConflictFile, build_activity_rows, finder_reason_line};

mod review;
// The retarget payload is built and checked from this module's tests; the emitting window is slice 6's.
#[cfg(test)]
pub use review::{REVIEW_OPEN_EVENT, ReviewMode, ReviewOpen};
