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
pub use constants::{POPOVER_SHOWN_EVENT, POPOVER_HIDDEN_EVENT, STORAGE_TTL_SECS, STORAGE_WAIT, STORAGE_FAILURE_BACKOFF_SECS, DEFAULT_ACTIVITY_LIMIT, CONFLICT_FILES_LIMIT};

mod runtime;
use runtime::*;
pub use runtime::{PopoverRuntime};

mod storage;
use storage::*;
pub use storage::{StorageUsage, StorageCache, clear_storage, StorageDto, cached_storage, cached_storage_with, fetch_storage_usage, storage_is_full};

mod snapshot;
use snapshot::*;
pub use snapshot::{finder_reason_line, AccountDto, EngineDto, ReasonDto, FinderDto, ConflictFile, ConflictsDto, ActivityState, ActivityRow, PopoverSnapshotDto, SnapshotInputs, build_activity_rows, DbView, gather_db_view, assemble};

mod review;
use review::*;
pub use review::{REVIEW_OPEN_EVENT, ReviewMode, ReviewOpen, emit_review_open};
