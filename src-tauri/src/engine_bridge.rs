//! Bridge between the Tauri runner, the SQLite state DB, and the
//! Beebeeb API client. Everything that "does sync work" goes through
//! this module:
//!
//! - **State machine** ([`FileSM`]) — explicit transition table for
//!   per-file lifecycle. The plan's value here is preventing nonsense
//!   transitions like Uploading → Downloading mid-flight.
//! - **`EngineBridge`** — owns the [`StateDb`] + [`ApiClient`] handles
//!   and exposes async operations: `hydrate_file` (cloud-only → local
//!   on demand) and the periodic `sync_tick` (metadata sweep, called
//!   from [`crate::runner`]).
//!
//! Spec: `docs/superpowers/plans/2026-05-07-desktop-sync-client.md`
//! Phase 1 Task 3.
//!
//! Content frames are opened only by core's pin-aware chunk_stream API. Transport
//! normalization accepts the existing raw and JSON envelopes without decrypting them.
//! ChunkDecryptor is the public facade over core StreamOpener; finish gates whole-file
//! publication. Indexed range reads authenticate frame zero before calling open_chunk.

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use zeroize::{Zeroize, Zeroizing};

use base64::Engine;
use beebeeb_types::EncryptedBlob;
use serde::{Deserialize, Serialize};

use crate::api_client::{ApiClient, DesktopUploadInitRequest};
use crate::conflict::is_text_file;
use crate::state_db::{
    ClaimOutcome, ClaimedOp, FileContractState, FileEntry, FileStatus, ItemKind, KeptReason, LocalActivityEventInput,
    LocalActivityKind, LocalFileId, Namespace, OperationKind, OperationPauseReason, PERMISSION_OWNER, PERMISSION_READ,
    PERMISSION_SHARE, PERMISSION_WRITE, ParkReason, PendingOperation, ServerFileId, StateDb, TookOver, UploadResume,
    VersionId,
};

pub struct EngineBridge {
    db: Arc<StateDb>,
    api: Arc<ApiClient>,
    /// Wire-byte counters shared with the heartbeat producer. Both are
    /// drained (swapped to 0) once per beat; incremented by the chunk loops.
    pub wire: Arc<WireCounters>,
    /// Bytes done and total for every transfer in flight (task 1683 slice 2). The
    /// runner hands in the board the popover snapshot reads; a one-shot bridge
    /// keeps a private one nobody reads.
    transfers: Arc<crate::transfer_progress::TransferBoard>,
    /// Cooperative stop signal (task 1538 Codex P1, PR #49 lib.rs:1087
    /// thread). `false` for the lifetime of a normal bridge. Flipped `true`
    /// by [`crate::runner::EngineRunner::abort`] BEFORE it even sends the
    /// tick-loop's cancel oneshot, so [`Self::process_due_operations`]
    /// (checked before every queued operation) and
    /// [`Self::queue_finder_create`]/[`Self::queue_finder_modify`]/
    /// [`Self::queue_finder_delete`] (checked before enqueuing) can refuse
    /// to start/queue further work immediately — instead of relying solely
    /// on `abort`'s tick-boundary cancel, which could otherwise let an
    /// entire in-progress due-operations batch, or a fresh watcher/File-
    /// Provider write landing mid-teardown, through unchecked.
    stopping: Arc<AtomicBool>,
    /// Files whose landed Finder write still waits for its new thumbnails (spec §5.6). The
    /// landing takes the op off the queue before they are uploaded, and until they are, the
    /// server holds the previous version's (one per file): [`Self::finder_thumbnail`] refuses.
    thumbnails_pending: std::sync::Mutex<HashSet<String>>,
    /// Ids whose listing carries a 2236 request envelope (L12). A cache only:
    /// the authoritative fact is persisted on the record (P3-3).
    request_rows: std::sync::Mutex<std::collections::HashSet<String>>,
    /// The owner's request keys, loaded once per account pass instead of once
    /// per download (P2-2).
    pub(crate) request_keys: std::sync::Mutex<Option<crate::owner_keys::RequestKeys>>,
    /// Re-snapshots that failed in a row (spec §6.3.2, macOS): from
    /// [`FAILED_SNAPSHOTS_BEFORE_THE_QUEUE_RUNS`] on, a failed one no longer ends the tick.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    failed_snapshots: std::sync::atomic::AtomicU32,
    /// Task 2244: edits newly held since the scan last logged its count ([`Self::take_newly_held`]), by
    /// reason: content unknown, then base unknown. A hold made between passes (a file's close) is counted
    /// in the next pass's line.
    newly_held: [std::sync::atomic::AtomicUsize; 2],
    /// Spec D11 (task 1888, review P2): the account's resume and pass requests, shared with the runner. A one-shot
    /// bridge keeps a private one nobody moves. Off macOS, [`Self::process_due_operations`] sends nothing while a
    /// release waits for a pass ([`Self::a_pass_is_owed`]).
    resume: crate::runner::ResumeTicket,
    /// LD-0113 (spec Amendment 8): `api` for the backup pass, built on first use; every request it makes carries
    /// `X-Beebeeb-Origin: backup` ([`ApiClient::for_backup_pass`]).
    backup_api: std::sync::OnceLock<Arc<ApiClient>>,
    #[cfg(test)]
    pub(crate) seams: Seams,
}

/// One concern of the `EngineBridge` inherent impl; the type, its fields and the re-exports
/// stay in this facade, and inherent impls may span modules.
mod lifecycle;
mod upload;
// LD-0113: the backup uploader (a file read in place from a backed-up folder).
mod backup_tree;
mod backup_upload;
// LD-0113: the Backups root, the merge's moves and the device manifest.
mod backup_root;
mod finder_create;
mod finder_edit;
mod hydrate;
mod local_classify;
mod local_layout;
mod operations;
mod shared_roots;
// LD-0105 W2: a Windows range session's preparation and its "whole" record.
mod range_fetch;
pub(crate) use backup_tree::{BackupChild, BackupFolderState, tighter_clock};
#[cfg_attr(not(target_os = "windows"), allow(unused_imports))]
pub(crate) use range_fetch::{PrepEnd, RangePrep};
mod conflicts;

// ── Wire-byte counters (P1 — live throughput) ────────────────────────────────
//
// Two `AtomicU64` counters track bytes actually sent/received on the wire in
// the chunk loops (upload + download).  The heartbeat producer drains them with
// `swap(0)` each beat so it gets the delta over the beat interval — the true
// wire speed, not a file-completion delta.
//
// Both are incremented from async task context (no blocking), so `Relaxed`
// ordering is sufficient: each counter is touched only from the engine thread,
// and the heartbeat producer races on a separate task.  The only requirement is
// that the swap and the add are individually atomic — a consistent view across
// both counters simultaneously is NOT required (the heartbeat is a telemetry
// estimate, not an accounting figure).

// ── Per-file state machine ────────────────────────────────────────────────────

// ── EngineBridge ──────────────────────────────────────────────────────────────

// Files `cached_sha256_file_hex` read on this thread (tests count them).
#[cfg(test)]
thread_local! {
    static HASH_READS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
thread_local! {
    /// Task 2190: the op whose decision check fails, for the feed's per-item test.
    static FAIL_DECIDABLE_READ: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
thread_local! { static LEGACY_CHILD_READS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

// ── Periodic sync tick ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    /// The production text of `src/<module>.rs` and every `src/<module>/` child, so a source guard
    /// scans every file the code lives in after a split. Test-only files are skipped and each
    /// file's `#[cfg(test)]` items are dropped.
    fn production_tree(module: &str) -> String {
        production_tree_parts(module)
            .into_iter()
            .map(|text| crate::finder_setup_command_tests::without_items(&text, &["#[cfg(test)]", "#[cfg(all(test"]))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The same files as [`production_tree`], with each file's `#[cfg(test)]` items kept: for a pin
    /// that asserts where a test-only item sits.
    fn production_tree_raw(module: &str) -> String {
        production_tree_parts(module).join("\n")
    }

    fn production_tree_parts(module: &str) -> Vec<String> {
        fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, out);
                } else if path.extension().is_some_and(|ext| ext == "rs") {
                    out.push(path);
                }
            }
        }
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = vec![src.join(format!("{module}.rs"))];
        walk(&src.join(module), &mut files);
        files.retain(|path| {
            let name = path.file_name().and_then(|name| name.to_str()).unwrap_or_default();
            name != "tests.rs"
                && !name.ends_with("_tests.rs")
                && !path.components().any(|part| part.as_os_str() == "tests")
        });
        files.sort();
        files
            .into_iter()
            .map(|path| {
                std::fs::read_to_string(&path)
                    .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
                    .replace("\r\n", "\n")
            })
            .collect()
    }

    fn test_open_content_frame(key: &beebeeb_core::kdf::FileKey, bytes: &[u8]) -> Result<Vec<u8>, EngineError> {
        let mut reader = beebeeb_core::chunk_stream::ChunkDecryptor::for_push_with_file_key(
            beebeeb_core::kdf::FileKey::from_bytes(*key.as_bytes()),
            "legacy",
            beebeeb_core::chunk_stream::ExpectedChunkFormat::Unrecorded,
        );
        let data = reader.push_frame(&content_frame(bytes)?)?.data;
        reader.finish()?;
        Ok(data)
    }
    mod sf2237d {
        use super::*;
        include!("engine_bridge/2237d_tests.rs");
    }
    use super::*;
    use base64::Engine;
    use serde_json::json;
    use std::io::Write as _;
    use std::net::TcpListener;
    use std::sync::Mutex;
    use std::thread;

    mod basics;
    mod conflicts;
    mod finder_queue;
    mod hydration;
    mod metadata;
    mod shared;
    mod trash_ops;
    mod uploads;
    mod uploads_2;
    mod uploads_3;
    mod uploads_4;
    // LD-0113 Task D3: the backup uploader leaves the source untouched.
    mod backup_host;
    mod backup_uploads;
    // LD-0113 PR 2: the Backups root, the merge and the device manifest.
    mod backup_layout;
    mod conflicts_2;
    mod downloads;
    mod errors_types;
    mod finder_queue_2;
    mod finder_queue_3;
    mod finder_queue_4;
    mod finder_queue_5;
    mod finder_queue_6;
    mod hydrate_files;
    mod ipc;
    mod ld0105_u1_rows;
    mod ld0113_protect;
    mod logging;
    mod metadata_2;
    mod refusals;
    mod refusals_2;
    mod refusals_3;
    mod reg_1640;
    mod reg_1640_2;
    mod remote_move;
    mod review_2190;
    mod review_2190_2;
    mod review_t3;
    mod scan;
    mod shared_2;
    mod sync;
    mod sync_2;
    mod sync_3;
    mod sync_4;
    mod sync_5;
    mod thumbnails;
    mod thumbnails_2;
    mod trash_markers;
    mod trash_ops_2;
    mod typed_ids;
    mod uploads_5;
    mod uploads_6;
    mod uploads_7;
    mod uploads_8;
    mod uploads_9;
    mod versions;
    mod versions_2;
    mod versions_3;
    // Task 2407 (core 2404): each sync-folder site takes its conflict path on an 8.3 alias (stubbed check; Windows: real names).
    mod name_alias_2407;
    #[cfg(windows)]
    mod name_alias_2407_windows;
    // Task 2409: Try again on a change that failed.
    mod unsent_retry;
    // Task 2413: a server row is stored only at a path of plain names; the one map onto the sync root.
    mod name_heal_2446;
    mod snapshot_names_2446;
    mod stored_path_2413;
    #[cfg(windows)]
    mod stored_path_2413_windows;

    use basics::{
        RecordedRequest, UploadMockServer, read_http_request_or_hangup, test_bridge, test_bridge_with_api,
        write_test_png,
    };
    use finder_queue_2::{capture_logs_async, fp_create};
    use finder_queue_5::{Refused, refuse, stop_refusing};
    #[cfg(unix)]
    use ipc::IpcHydrationMock;
    use metadata_2::patched_parents;
    use refusals_2::{reqwest_status_error, transient};
    use refusals_3::{PLANTED, assert_clean_line};
    #[cfg(target_os = "windows")]
    use reg_1640::{regression_1640_chunk, regression_1640_web_saves};
    use reg_1640::{
        regression_1640_edited_file_in_a_folder, regression_1640_edited_file_in_a_folder_in,
        regression_1640_synced_file,
    };
    use reg_1640_2::{regression_1640_new_placeholder, regression_1640_uploads_queued_for};
    use scan::{
        regression_2229_set_modified, task_2190_files_holding, task_2190_fu1_tick_after, task_2190_server_chunk,
        task_2190_web_saves,
    };
    use shared_2::{enqueue_test_operation, seed_bridge_entry, seed_bridge_row};
    use sync::{SyncMockServer, enc_name, snap_node};
    use sync_5::{landed_create, rule3_setup, thumbnail_blob};
    use trash_markers::{MockServerFile, VersionedServerMock, VersionedServerState};
    use uploads::{
        HydrationMockServer, TEST_FILE_ID, encrypt_chunk_wire, http_bytes, http_json, http_json_dated,
        hydration_test_key, read_http_request, upload_mock_response,
    };
    #[cfg(unix)]
    use uploads::{MockResponse, hydration_mock_response};
    use uploads_3::{ResumableUploadMock, ResumeMockMode, count_requests, init_item_id, resumable_create_op};
    use uploads_5::{fp_save, queue_save, seed_uploaded_db_row, seed_uploaded_row};
    #[cfg(unix)]
    use uploads_7::{land_one_save, remote_update};
    use uploads_8::{assert_kept_uploading, failing_passes};
    use versions::{
        drain_upload_queue, finder_file_target, held_content_version, run_competing, versioned_server_answer,
    };
    #[cfg(unix)]
    use versions_2::{FP_ROOT, fp_create_in, listed_under, reparented_changes};
    use versions_2::{fp_save_moved, name_parent_status, seed_folder_row, seed_row_in_folder};
    #[cfg(target_os = "macos")]
    use versions_3::*;
}

/// The attempt resumed its persisted upload session, found it reaped (the server answered 404 or
/// 410) and abandoned it: the next attempt starts a fresh upload, so for a File Provider write the
/// failure is transient, never a counted one (spec §8.4). A 400 at any session, and a 404 or 410 at a
/// session opened in the same attempt, abandon the session too, but stay counted. It shows, and has
/// as its source, the failure it wraps, so every reading of the error's text and status stays as it
/// was.
#[derive(Debug)]
struct SessionAbandoned(anyhow::Error);

impl std::fmt::Display for SessionAbandoned {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for SessionAbandoned {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        let inner: &(dyn std::error::Error + 'static) = self.0.as_ref();
        Some(inner)
    }
}

/// A Finder write could not be staged: the staging folder cannot be created or written to, and
/// on macOS nothing falls back to a folder the system may purge (spec §8.4). The reply is
/// `WriteRetryLater`, which the extension reports as a transient error, so the system keeps the
/// change and retries. Path-free on purpose: the text reaches the extension and the person.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
pub(crate) struct FinderStagingUnavailable(Option<std::io::ErrorKind>);

impl std::fmt::Display for FinderStagingUnavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            Some(kind) => write!(f, "the staging folder is unavailable: {kind}"),
            None => f.write_str("the staging folder is unavailable: no data directory"),
        }
    }
}

impl std::error::Error for FinderStagingUnavailable {}

/// The result of [`EngineBridge::hydrate_file_range`]: the decrypted plaintext
/// covering the requested range, plus enough context for the caller to splice
/// out the exact `[required_offset, required_offset + required_length)` span.
#[cfg(any(target_os = "windows", test))]
pub struct HydratedRange {
    /// Decrypted plaintext for chunks `[first_chunk, last_chunk]` — i.e. the
    /// smallest chunk-aligned span that covers the requested byte range. This
    /// is NOT necessarily aligned to the requested range itself; the caller
    /// must slice `data[required_offset - range_start_bytes ..]`.
    pub data: Zeroizing<Vec<u8>>,
    /// Absolute byte offset (into the full file) of `data[0]` — i.e.
    /// `first_chunk * chunk_size_bytes`.
    pub range_start_bytes: u64,
    /// `true` when `data` happens to cover the ENTIRE file (the covering-chunk
    /// range was `[0, chunk_count)`). Only then is a range hydration equivalent
    /// to a whole-file hydration for status-bookkeeping / resolve-or-error
    /// purposes.
    pub covers_whole_file: bool,
}

/// Test-only hooks at named points where the concurrency tests force an
/// interleaving (spec §13, "Concurrency tests"). Each hook fires once.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct Seams {
    hooks: std::sync::Mutex<HashMap<&'static str, SeamHook>>,
    /// Staging bases this bridge uses instead of the process's (`stage_under`, macOS tests).
    staging_bases: std::sync::Mutex<Option<FinderStagingBases>>,
    /// The download transport this bridge uses (task 2173). Unset: `Auto`, so a test never reads
    /// the developer's `BB_TRANSPORT` or `desktop.toml`.
    transport: std::sync::Mutex<Option<crate::direct_download::Transport>>,
    /// Every direct storage fetch fails with this status, without a request (task 2173).
    storage_fail: std::sync::Mutex<Option<u16>>,
}

#[cfg(test)]
type SeamHook = Box<dyn FnOnce() + Send>;

#[cfg(test)]
impl Seams {
    pub(crate) fn arm(&self, name: &'static str, hook: impl FnOnce() + Send + 'static) {
        self.hooks.lock().unwrap().insert(name, Box::new(hook));
    }

    fn fire(&self, name: &'static str) {
        // Taken out first: the hook runs without the map's lock held.
        let hook = self.hooks.lock().unwrap().remove(name);
        if let Some(hook) = hook {
            hook();
        }
    }

    /// From now on this bridge stages Finder writes under `bases`, not the test sandbox.
    /// macOS only: its one caller is the macOS staging-folder test.
    #[cfg(target_os = "macos")]
    pub(crate) fn stage_under(&self, bases: FinderStagingBases) {
        *self.staging_bases.lock().unwrap() = Some(bases);
    }

    fn staging_bases(&self) -> Option<FinderStagingBases> {
        self.staging_bases.lock().unwrap().clone()
    }

    /// From now on this bridge downloads with `transport` (task 2173).
    pub(crate) fn use_transport(&self, transport: crate::direct_download::Transport) {
        *self.transport.lock().unwrap() = Some(transport);
    }

    /// From now on every direct storage fetch of this bridge fails with `status` (task 2173).
    pub(crate) fn fail_storage(&self, status: u16) {
        *self.storage_fail.lock().unwrap() = Some(status);
    }
}

mod types;
#[cfg(windows)]
pub use types::FetchedBytes;
use types::*;
pub use types::{
    CacheCleanupOutcome, CachePolicy, ConflictContentPreview, ConflictContentSide, ConflictDetected, FetchTicket,
    FetchedVersion, FpWrite, ParkedChoice, ParkedResolution, PinUpdateOutcome, SyncTickOutcome, TransferLoopOutcome,
    WireCounters, name_opens,
};
pub(crate) use types::{
    LocalEntry, NON_UUID, OpDone, PrunedEntry, RENAME_SETTLE_SECS, SyncFolderContent, log_alias_resolved,
};
// LD-0113 Amendment 7: the Windows-native rung removes a read-only Backups file the way the engine does.
#[cfg(all(test, target_os = "windows"))]
pub(crate) use types::{RemovedKind, engine_remove};

mod errors;
use errors::*;
pub(crate) use errors::{
    AcceptsClosed, EngineError, ItemInTrash, ParkNow, QueueStateMoved, QueuedCopyMissing, StagedCopyGone,
    ThumbnailNotYet, UnknownItem, UnsentLocalChange, engine_cause, error_category, error_http_status, loggable_id,
};
pub use errors::{OperationFailureClass, classify_operation_error, retry_delay_seconds};

mod content;
use content::*;
pub(crate) use content::{ContentFingerprint, cached_sha256_file_hex, content_frame, sha256_file_hex, sha256_hex};

mod media;
use media::*;

mod fs;
use fs::*;
pub(crate) use fs::{LocalPathVerdict, move_no_replace, path_is_engine_internal, relative_db_path};

mod meta;
#[cfg(windows)]
pub(crate) use meta::promote_fetched_content;
use meta::*;
pub(crate) use meta::{
    HydrateProgressFn, hydrate_dest_is_allowed, hydrate_final_status, item_content_version, item_version_identifier,
    wire_id_fields, write_hydrated_from_reader, write_hydrated_plaintext,
};
pub use meta::{HydrateSource, SharedRootRefreshOutcome};

mod meta_2;
use meta_2::*;
pub(crate) use meta_2::{decrypt_shared_name_with_key, parse_base_version_number};

mod landing;
#[cfg(windows)]
pub(crate) use landing::retry_kept_originals;
use landing::*;
pub(crate) use landing::{FinderStagingBases, finder_staging_candidates, keep_both_name};
pub use landing::{
    FinderWriteItemKind, FinderWriteOutcome, FinderWriteTarget, VersionConflictEntry, is_ignored_finder_name,
    version_conflict_feed_from_db,
};

mod syncflow;
#[cfg(test)]
pub use syncflow::sync_tick;
pub use syncflow::sync_tick_outcome;
use syncflow::*;
pub(crate) use syncflow::{init_conflict_class, local_file_path_under_sync_root, sync_folder_content};
