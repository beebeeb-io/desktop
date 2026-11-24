//! `types` items a surface reads (queue summary, selective-sync row, bandwidth point), split out of `types.rs` for
//! the file-size ratchet. Unchanged.

use super::*;

/// What the Windows tray and Settings › Sync say about the queue (0.8.15): counts and dates only, never a name, a path
/// or an error text.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct QueueHold {
    /// Not paused and not given up: due now, backing off, or being sent. On Windows and Linux, not one that waits for a
    /// held folder either (spec D7: those are `held`'s `waiting_for_folder`).
    pub waiting: i64,
    /// Not paused, but out of attempts: nothing retries them.
    pub gave_up: i64,
    /// Paused, one group per reason (`PAUSE_REASON_LABELS`, anything else as `other`), ordered by reason. On Windows
    /// and Linux it also holds the computed `waiting_for_folder` group (spec D7).
    pub held: Vec<HeldGroup>,
    /// Task 2414: downloads a pin queued (`hydrate_file`, `pin_tree`) that are paused or out of attempts: files meant to
    /// be on this computer that are not, and will not be by themselves. Never in the three counts above ("Not sent").
    pub not_downloaded: i64,
}

/// The operations paused for one reason.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HeldGroup {
    pub reason: String,
    pub count: i64,
    /// Unix seconds the oldest and newest of them were queued (0 = not recorded).
    pub oldest_at: i64,
    pub newest_at: i64,
}

/// One `files` row as the Selective sync tree reads it (0.8.15). A row the server never confirmed (a folder this PC
/// created whose `create_folder` never landed) has `current_version` 0 and `remote_updated_at` 0; a server row has one
/// of them set.
#[derive(Debug, Clone, PartialEq)]
pub struct TreeRow {
    pub file_id: String,
    pub path: String,
    pub status: FileStatus,
    pub size_bytes: i64,
    pub parent_id: Option<String>,
    pub item_kind: ItemKind,
    pub current_version: i64,
    pub remote_updated_at: i64,
}

/// A single bandwidth measurement point.
#[derive(Debug, Clone, serde::Serialize)]
pub struct BandwidthSample {
    /// Unix timestamp (seconds) when the sample was recorded.
    pub sampled_at: i64,
    /// Upload bytes transferred during `period_secs`.
    pub up_bytes: u64,
    /// Download bytes transferred during `period_secs`.
    pub down_bytes: u64,
    /// Duration of the measurement window (seconds).
    pub period_secs: u32,
}
