//! `types` items split out of `held_queue.rs`.

use super::*;

/// A `files` row, as the pass reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub file_id: String,
    pub parent_id: Option<String>,
    pub path: String,
    pub is_folder: bool,
    /// `current_version > 0 OR remote_updated_at > 0` (spec §3, corrected).
    pub server_known: bool,
    pub current_version: i64,
}

/// An `operation_queue` row, as the pass reads it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Op {
    pub op_id: String,
    pub kind: String,
    pub file_id: Option<String>,
    pub parent_id: Option<String>,
    pub target_path: Option<String>,
    /// An `upload_version` whose metadata says `operation: create_file`.
    pub create_file: bool,
    pub paused_reason: Option<String>,
    /// `attempts >= max_attempts AND paused_reason IS NULL AND last_error_class IS NULL` (spec §3).
    pub given_up: bool,
    /// `last_error_class IS NOT NULL`: parked. A park is never re-armed (spec D8).
    pub parked: bool,
    pub last_error: Option<String>,
    pub payload_path: Option<String>,
    pub base_version: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Options {
    /// This device's backup folder name (D9); `None` when the hostname could not be read.
    pub device_segment: Option<String>,
    /// `≡` ignores letter case (Windows: NTFS).
    pub case_insensitive: bool,
}

/// One unit of the plan. Units never split: each is applied whole, in one transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unit {
    /// D1: the create landed (its own row is server-known): delete only its op.
    DropLanded { op_id: String },
    /// D2: fold `ghost` into `twin`: re-point every child row and op, re-arm `rearm` and hold `hold` (D8: superseded),
    /// delete `ghost_op` and the ghost's row. The holds ride in the unit, so no batch separates them from the remap that
    /// takes their ghost away.
    Remap { ghost: String, twin: String, ghost_op: String, rearm: Vec<String>, hold: Vec<String> },
    /// D4 / D8 superseded: `needs_review`.
    Hold { op_id: String },
    /// A `needs_review` op whose cause is gone: back to `auth`, for the key check that follows the pass (D12).
    Release { op_id: String },
    /// D8 under a ghost kept for replay.
    Rearm { op_id: String },
    /// D6: the parent found by path, for the op and for its row (`file_id`, when the row exists).
    ResolveParent { op_id: String, file_id: Option<String>, parent: String },
    /// D5: empty `Backups/unknown` scaffolding.
    DropScaffold { op_ids: Vec<String>, file_ids: Vec<String>, dirs: Vec<String> },
}

/// What the plan does, by D row. The dry run (D14).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Counts {
    pub landed: usize,
    pub remapped: usize,
    pub replay: usize,
    /// A ghost kept for replay whose own create gave up on anything but a parent 404 (D8, builder check Q4).
    pub replay_given_up: usize,
    /// A ghost under a held (or person-paused) ghost: it waits with it, through the parent-first gate (D4, D7).
    pub waiting_under_held: usize,
    /// A ghost whose create waits for the person (2267's `found_while_paused`, quota, a replaced key): left alone.
    pub left_paused: usize,
    /// A `needs_review` op whose cause is gone, released to the key check (D4, D6, D10).
    pub released: usize,
    pub held_ambiguous: usize,
    pub held_acts_on_ghost: usize,
    pub held_parents_disagree: usize,
    pub held_twin_busy: usize,
    pub rearmed: usize,
    /// D8 amendment (lead ruling 2026-10-29): an op paused for the person (2267's `found_while_paused`, quota, a
    /// replaced key) whose retry budget is re-armed on a parent 404; its pause stays.
    pub rearmed_paused: usize,
    pub rearm_no_payload: usize,
    pub rearm_superseded: usize,
    pub parent_resolved: usize,
    pub parent_unresolved: usize,
    pub scaffold_dropped: usize,
    /// D5: the sync-root-relative directories of the dropped scaffolding, for the caller to remove bottom-up with
    /// `std::fs::remove_dir` after the commit. Paths: never logged.
    pub scaffold_dirs: Vec<String>,
    pub backup_unknown_kept: usize,
    pub file_twin: usize,
}

/// The pause reason of an op the pass held for the person to look at (D4).
pub const NEEDS_REVIEW: &str = "needs_review";

/// An upload reads its staged payload; without it a re-armed upload only fails again.
pub(super) const UPLOAD_KINDS: [&str; 2] = ["upload_version", "upload_file"];
