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
//! ## Divergence from the plan
//!
//! The plan's `hydrate_file` body referenced
//! `beebeeb_core::crypto::derive_file_key` and a raw `nonce || ciphertext`
//! chunk format. The real API surface is
//! `beebeeb_core::kdf::derive_file_key(&MasterKey, &[u8]) -> FileKey` and
//! `beebeeb_core::encrypt::decrypt_chunk(&FileKey, &EncryptedBlob)`, with
//! chunks stored on the wire as JSON-serialised `EncryptedBlob`s (matches
//! `repos/cli/src/commands/push.rs`'s `serde_json::to_vec(&blob)` upload
//! path and `pull.rs`'s `serde_json::from_slice` decode path). The
//! implementation below uses the real API.
//!
//! The per-chunk GET endpoint (`GET /api/v1/files/:id/chunks/:idx`) was
//! added in `server` commit `d3cf0e2`. Before that landed, `hydrate_file`
//! was a stub returning `anyhow::Err`.

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use zeroize::{Zeroize, Zeroizing};

use base64::Engine;
use beebeeb_types::{CipherSuite, EncryptedBlob};
use serde::{Deserialize, Serialize};

use crate::api_client::{ApiClient, DesktopUploadInitRequest};
use crate::conflict::{VersionInfo, is_conflict, is_text_file};
use crate::state_db::{
    ClaimOutcome, ClaimedOp, FileContractState, FileEntry, FileStatus, ItemKind, LocalActivityEventInput,
    LocalActivityKind, Namespace, OperationKind, OperationPauseReason, PERMISSION_OWNER, PERMISSION_READ,
    PERMISSION_SHARE, PERMISSION_WRITE, ParkReason, PendingOperation, QueueDiagnostics, StateDb, TookOver,
    UploadResume,
};

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

/// Shared wire-byte counters threaded from `EngineBridge` to `runner`
/// to the heartbeat producer. Wrapped in `Arc` so it can be cloned
/// cheaply into the spawned heartbeat task.
pub struct WireCounters {
    /// Bytes written to the server in upload chunk loops (plaintext length
    /// of each chunk before encryption — the user's data rate, not wire overhead).
    pub upload_bytes: AtomicU64,
    /// Bytes received from the server in download chunk loops (raw wire bytes
    /// including the encryption envelope, which is all the client can measure
    /// without re-decrypting — close enough for throughput display).
    pub download_bytes: AtomicU64,
}

impl WireCounters {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            upload_bytes: AtomicU64::new(0),
            download_bytes: AtomicU64::new(0),
        })
    }

    /// Drain and return `(upload_bytes, download_bytes)` since the last drain.
    /// Both counters are atomically reset to 0. Called once per heartbeat beat.
    pub fn drain(&self) -> (u64, u64) {
        let up = self.upload_bytes.swap(0, Ordering::Relaxed);
        let dn = self.download_bytes.swap(0, Ordering::Relaxed);
        (up, dn)
    }
}

const THUMBNAIL_VARIANTS_FOR_UPLOAD: [ThumbnailUploadVariant; 2] =
    [ThumbnailUploadVariant::Medium, ThumbnailUploadVariant::Large];
const MEDIUM_ENCRYPTED_THUMBNAIL_MAX_BYTES: usize = 128 * 1024;
const LARGE_ENCRYPTED_THUMBNAIL_MAX_BYTES: usize = 512 * 1024;
const BLURHASH_COMPONENTS_X: usize = 4;
const BLURHASH_COMPONENTS_Y: usize = 3;
const BLURHASH_SOURCE_MAX_DIMENSION: u32 = 64;
const BLURHASH_BASE83: &[u8; 83] =
    b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz#$%*+,-.:;=?@[]^_{|}~";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ThumbnailUploadVariant {
    Medium,
    Large,
}

impl ThumbnailUploadVariant {
    fn label(self) -> &'static str {
        match self {
            Self::Medium => "medium",
            Self::Large => "large",
        }
    }

    fn config(self) -> beebeeb_core::thumbnail::ThumbnailConfig {
        match self {
            Self::Medium => beebeeb_core::thumbnail::ThumbnailConfig::medium(),
            Self::Large => beebeeb_core::thumbnail::ThumbnailConfig::large(),
        }
    }

    fn encrypted_max_bytes(self) -> usize {
        match self {
            Self::Medium => MEDIUM_ENCRYPTED_THUMBNAIL_MAX_BYTES,
            Self::Large => LARGE_ENCRYPTED_THUMBNAIL_MAX_BYTES,
        }
    }
}

#[derive(Debug)]
struct ThumbnailSource {
    rgba: Zeroizing<Vec<u8>>,
    width: u32,
    height: u32,
    is_video: bool,
}

#[derive(Debug)]
struct PreparedThumbnailUpload {
    variant: ThumbnailUploadVariant,
    encrypted: Zeroizing<Vec<u8>>,
    blurhash: Option<String>,
}

// ── Per-file state machine ────────────────────────────────────────────────────

/// Events that drive a file through its sync lifecycle.
pub enum FileEvent {
    DownloadStart,
    DownloadComplete,
    DownloadFail,
    UploadStart,
    UploadComplete,
    UploadFail,
    ConflictDetected,
    ConflictResolved,
    Evict,
}

/// In-memory state tracker for a single file. The persisted state is
/// in [`StateDb`]; this struct exists so callers can validate the
/// _next_ state before writing it.
pub struct FileSM {
    state: FileStatus,
}

impl FileSM {
    pub fn new(state: FileStatus) -> Self {
        Self { state }
    }
    pub fn state(&self) -> FileStatus {
        self.state.clone()
    }

    /// Apply `event`. Returns `Err` if the event isn't legal in the
    /// current state — callers should treat that as a programming bug,
    /// not a runtime error to recover from. The whitelist below is the
    /// canonical sync state machine; deny by default.
    pub fn transition(&mut self, event: FileEvent) -> Result<(), &'static str> {
        use FileEvent::*;
        use FileStatus::*;
        self.state = match (&self.state, event) {
            (CloudOnly, DownloadStart) => Downloading,
            (Downloading, DownloadComplete) => Local,
            (Downloading, DownloadFail) => Error,
            (Local, UploadStart) => Uploading,
            (Uploading, UploadComplete) => Local,
            (Uploading, UploadFail) => Error,
            (Local, ConflictDetected) => Conflict,
            (Conflict, ConflictResolved) => Local,
            (Local, Evict) => CloudOnly,
            _ => return Err("invalid state transition"),
        };
        Ok(())
    }
}

// ── EngineBridge ──────────────────────────────────────────────────────────────

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
    #[cfg(test)]
    pub(crate) seams: Seams,
}

/// Test-only hooks at named points where the concurrency tests force an
/// interleaving (spec §13, "Concurrency tests"). Each hook fires once.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct Seams {
    hooks: std::sync::Mutex<HashMap<&'static str, SeamHook>>,
    /// Staging bases this bridge uses instead of the process's (`stage_under`, macOS tests).
    staging_bases: std::sync::Mutex<Option<FinderStagingBases>>,
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FinderWriteItemKind {
    File,
    Folder,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FinderWriteTarget {
    pub file_id: Option<String>,
    pub parent_id: Option<String>,
    pub filename: String,
    /// Full server-relative, '/'-joined path of this item under the sync root
    /// (e.g. `docs/a.txt` for a nested file, `a.txt` at the root). When `None`,
    /// the path defaults to the leaf `filename` — the legacy top-level behaviour.
    ///
    /// **Why this exists (task 0780 nested correctness):** the local-create path
    /// stores this as the row's `path` and threads it through as the upload's
    /// `target_path`, so (a) `classify_local_path`'s "already a known server
    /// file?" filter (which queries the FULL relative key) matches a nested
    /// file's row immediately, (b) `defer_local_upload_finalization` joins
    /// `sync_root + rel_path` to find the file on disk and re-stamp it in-sync,
    /// and (c) a local delete looks up the full key and trashes it on the server.
    /// Without it, nested rows were keyed by the leaf only and all three broke
    /// until the next `sync_tick` re-keyed them. Top-level is unaffected (leaf ==
    /// full key). The OS-extension IPC paths that don't supply it keep the leaf
    /// fallback.
    #[serde(default)]
    pub rel_path: Option<String>,
    pub kind: FinderWriteItemKind,
    pub contents_path: Option<String>,
    pub content_type: Option<String>,
    pub base_version_identifier: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FinderWriteOutcome {
    Queued {
        op_id: String,
        file_id: Option<String>,
        kind: OperationKind,
        ignored: bool,
        message: String,
    },
    Ignored {
        message: String,
    },
}

/// A File Provider write's outcome, with the token its reply names (spec §5) and, for a
/// write under a provisional id whose create has landed, the identifier the reply is
/// presented under: the system's own id for the item until it applies the swap (spec §7.2).
#[derive(Debug, Clone, PartialEq)]
pub struct FpWrite {
    pub outcome: FinderWriteOutcome,
    pub token: Option<String>,
    pub present_as: Option<String>,
}

impl FpWrite {
    pub fn plain(outcome: FinderWriteOutcome) -> Self {
        Self {
            outcome,
            token: None,
            present_as: None,
        }
    }
}

/// Where a Finder fetch was served from (spec §7.4, §11: the device plan counts these).
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HydrateSource {
    /// The staged copy of the held write, still queued: the bytes its token names.
    Queue,
    /// The server, as every fetch did before.
    Server,
}

#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
impl HydrateSource {
    pub fn as_str(self) -> &'static str {
        match self {
            HydrateSource::Queue => "queue",
            HydrateSource::Server => "server",
        }
    }
}

/// A Finder write for an id this Mac never knew and no alias resolves (spec §7.3). The
/// reply is an error the extension reports as `cannotSynchronize`, so the edit stays on
/// disk; never "no such item", which would make the system delete it.
#[derive(Debug)]
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
pub(crate) struct UnknownItem;

impl std::fmt::Display for UnknownItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("this item is not known to Beebeeb")
    }
}

impl std::error::Error for UnknownItem {}

/// §5.6 safe default: no thumbnail is fetched while the item's held write is queued. The
/// server holds only the previous version's, and the system would cache it under the token.
#[derive(Debug)]
#[cfg_attr(not(any(unix, test)), allow(dead_code))]
pub(crate) struct ThumbnailNotYet;

impl std::fmt::Display for ThumbnailNotYet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the thumbnail is available once the save has uploaded")
    }
}

impl std::error::Error for ThumbnailNotYet {}

/// One line per request that reached the server id through an alias (spec §7.2, §11).
pub(crate) fn log_alias_resolved(request: &'static str, provisional_id: &str, file_id: &str) {
    tracing::warn!(
        request,
        provisional_id = %provisional_id,
        file_id = %file_id,
        "provisional id resolved to the server id"
    );
}

/// A delete under a provisional id that does not name the server file's held token (m-6).
fn log_alias_delete_kept(provisional_id: &str, file_id: &str) {
    tracing::warn!(
        provisional_id = %provisional_id,
        file_id = %file_id,
        "delete of a provisional id not applied to the server file"
    );
}

/// The device plan's count of fetches that reached the daemon (spec §11, §14 instrument 2).
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
fn log_hydrate_served(file_id: &str, source: HydrateSource) {
    tracing::info!(file_id = %file_id, source = source.as_str(), "Finder hydrate served");
}

#[cfg(test)]
impl FpWrite {
    /// The file id of a queued write.
    fn outcome_file_id(&self) -> String {
        match &self.outcome {
            FinderWriteOutcome::Queued {
                file_id: Some(file_id), ..
            } => file_id.clone(),
            other => panic!("not a queued write with a file id: {other:?}"),
        }
    }
}

/// What every Finder create checks first, and the ids it gets (`start_finder_create`).
struct FinderCreateStart {
    parent_contract: Option<FileContractState>,
    file_id: String,
    rel_path: String,
}

/// A Finder file create, staged and described, before it is queued.
struct PreparedFinderCreate {
    file_id: String,
    rel_path: String,
    staged: crate::staged_payload::StagedPayload,
    staged_path: String,
    size_bytes: i64,
    row: FileEntry,
    payload: serde_json::Value,
}

/// A Finder content modify, staged and described, before it is queued.
struct PreparedFinderModify {
    staged: crate::staged_payload::StagedPayload,
    staged_path: String,
    size_bytes: i64,
    modified_at: i64,
    payload: serde_json::Value,
}

/// The reply to a write of a name Finder writes for itself.
fn ignored_finder_item(filename: &str) -> FinderWriteOutcome {
    FinderWriteOutcome::Ignored {
        message: format!("ignored temporary Finder item {filename}"),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachePolicy {
    pub max_unpinned_cache_bytes: i64,
    pub disk_pressure_min_free_bytes: u64,
}

impl Default for CachePolicy {
    fn default() -> Self {
        Self {
            max_unpinned_cache_bytes: crate::config::LOCAL_CACHE_LIMIT_100_GB_BYTES,
            disk_pressure_min_free_bytes: 5 * 1024 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PinUpdateOutcome {
    pub changed_item_ids: Vec<String>,
    pub hydrate_operations: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheCleanupOutcome {
    pub evicted_file_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VersionConflictEntry {
    pub id: String,
    pub file_id: String,
    pub file_name: String,
    pub kind: String,
    pub status: String,
    pub updated_at: Option<i64>,
    pub detail: String,
    pub action: String,
    pub op_id: Option<String>,
    pub version_id: Option<String>,
    pub base_version: Option<i64>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SharedRootRefreshOutcome {
    pub active_shared_root_ids: Vec<String>,
    pub removed_shared_file_ids: Vec<String>,
    pub removed_cache_paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TransferLoopOutcome {
    pub completed_op_ids: Vec<String>,
    pub retried_op_ids: Vec<String>,
    pub paused_op_ids: Vec<String>,
    pub invalidated_item_ids: Vec<String>,
    /// Task 1700: errors from best-effort post-complete upload work
    /// (thumbnail generation/upload). These never fail the op — the upload
    /// itself is committed — but they used to vanish into a
    /// `tracing::warn!` that no test subscriber captures, so a red CI run
    /// said only "medium thumbnail upload with blurhash query" with no
    /// trail. Each entry is `"<op_id>: <error>"`.
    pub post_complete_errors: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationFailureClass {
    Retryable,
    Auth,
    Quota,
    Permission,
    Locked,
}

impl OperationFailureClass {
    fn pause_reason(self) -> Option<OperationPauseReason> {
        match self {
            OperationFailureClass::Retryable => None,
            OperationFailureClass::Auth => Some(OperationPauseReason::Auth),
            OperationFailureClass::Quota => Some(OperationPauseReason::Quota),
            OperationFailureClass::Permission => Some(OperationPauseReason::Permission),
            OperationFailureClass::Locked => Some(OperationPauseReason::Locked),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SharedRootMapping {
    invite_id: String,
    file_id: String,
    display_name: String,
    is_folder: bool,
    size_bytes: i64,
    content_type: Option<String>,
    owner_email: Option<String>,
    sender_public_key: Option<String>,
    encrypted_file_key: Option<String>,
    encrypted_folder_key: Option<String>,
    file_name_encrypted: Option<String>,
    permission_bits: i64,
    approved_at: Option<i64>,
}

impl EngineBridge {
    /// Build a bridge with its own private, never-flipped stop flag. Correct
    /// for the one-shot bridges Tauri IPC commands build over the app-local
    /// state DB (restore version, set pin, resolve conflict, …) — those are
    /// each a fresh, independent unit of work, not the long-running engine
    /// loop `EngineRunner` owns, so there is nothing external that should
    /// ever ask THIS instance to stop mid-call.
    pub fn new(db: Arc<StateDb>, api: Arc<ApiClient>) -> Self {
        Self::new_with_stop_flag(db, api, Arc::new(AtomicBool::new(false)))
    }

    /// Like [`Self::new`], but shares an externally-owned stop flag —
    /// [`crate::runner::run`] passes the SAME `Arc<AtomicBool>` its
    /// `EngineRunner` flips on `abort()`, so this bridge (and every clone of
    /// it handed to the IPC socket server / Windows upload watcher) observes
    /// the stop request the instant it's set, not just at the next tick
    /// boundary (task 1538 Codex P1).
    pub fn new_with_stop_flag(db: Arc<StateDb>, api: Arc<ApiClient>, stopping: Arc<AtomicBool>) -> Self {
        Self {
            db,
            api,
            wire: WireCounters::new(),
            transfers: crate::transfer_progress::TransferBoard::new(),
            stopping,
            #[cfg(test)]
            seams: Seams::default(),
        }
    }

    /// A named point for the concurrency tests; nothing outside tests.
    fn seam(&self, name: &'static str) {
        #[cfg(test)]
        self.seams.fire(name);
        #[cfg(not(test))]
        let _ = name;
    }

    /// Where this bridge stages a Finder write's plaintext copy ([`default_finder_staging_root`]). A test may
    /// point one bridge at bases of its own.
    fn finder_staging_root(&self) -> Result<PathBuf, FinderStagingUnavailable> {
        #[cfg(test)]
        if let Some(bases) = self.seams.staging_bases() {
            return finder_staging_root_from(&bases);
        }
        default_finder_staging_root()
    }

    /// Report transfer progress to `board` (task 1683 slice 2).
    pub fn with_transfers(mut self, board: Arc<crate::transfer_progress::TransferBoard>) -> Self {
        self.transfers = board;
        self
    }

    pub fn transfers(&self) -> &Arc<crate::transfer_progress::TransferBoard> {
        &self.transfers
    }

    /// Remember that a transfer finished, for the popover's recent-activity arrow.
    /// Best-effort: a failed write costs a row in a list, never the transfer.
    fn record_transfer_done(&self, direction: crate::transfer_progress::Direction, file_id: &str, bytes: u64) {
        let (name, rel_path) = match self.db.get_file(file_id) {
            Ok(Some(entry)) => (crate::transfer_progress::display_name(&entry.path), Some(entry.path)),
            _ => return,
        };
        let input = crate::state_db::TransferActivityInput {
            direction: direction.as_str(),
            file_id: Some(file_id.to_string()),
            file_name: name,
            rel_path,
            bytes: i64::try_from(bytes).unwrap_or(i64::MAX),
            occurred_at: now_secs() as i64,
        };
        if let Err(e) = self.db.record_transfer_activity(input) {
            tracing::debug!(error = %e, "could not record transfer activity (best-effort)");
        }
    }

    /// `true` once a caller has asked this engine to stop (task 1538).
    /// `SeqCst` — this flag is the FIRST thing `EngineRunner::abort` sets,
    /// before it even sends the tick loop's cancel oneshot, specifically so
    /// this read is guaranteed to observe it promptly from any thread.
    pub fn is_stopping(&self) -> bool {
        self.stopping.load(Ordering::SeqCst)
    }

    /// Borrow the underlying state DB so callers (e.g. [`sync_tick`])
    /// can do their own writes without piping every operation through
    /// the bridge.
    pub fn db(&self) -> &StateDb {
        &self.db
    }

    /// Borrow the API client for the same reason.
    pub fn api(&self) -> &ApiClient {
        &self.api
    }

    pub fn queue_diagnostics(&self, now: i64) -> anyhow::Result<QueueDiagnostics> {
        Ok(self.db.queue_diagnostics(now)?)
    }

    pub async fn process_due_operations(&self, sync_root: &Path, now: i64) -> anyhow::Result<TransferLoopOutcome> {
        let mut outcome = TransferLoopOutcome {
            completed_op_ids: Vec::new(),
            retried_op_ids: Vec::new(),
            paused_op_ids: Vec::new(),
            invalidated_item_ids: Vec::new(),
            post_complete_errors: Vec::new(),
        };
        #[cfg(target_os = "windows")]
        if !self.is_stopping() {
            crate::windows_cf::upload_finalization::retry(&self.db, sync_root);
        }
        let operations = self.db.list_due_operations(now)?;

        for op in operations {
            // Task 1538 Codex P1 (PR #49, lib.rs:1087 thread): stop draining
            // the queue the instant a caller asks this engine to stop,
            // rather than finishing every due operation first. `abort()`'s
            // graceful window is bounded (3s) before it force-terminates the
            // whole task — a caller waiting on that to purge the queue on
            // sign-out needs this loop to actually stop promptly on its own,
            // not "eventually, once the batch happens to finish".
            if self.is_stopping() {
                break;
            }
            // The list is read once per pass, and an upload that lands moves
            // the later ops of its file along (`StateDb::apply_landing`),
            // so run each op as it is NOW: the claim re-reads it, enforces its
            // file's content order and marks this attempt, in one transaction.
            // Every write that records the attempt's outcome names the claim.
            self.seam("claim:before_tx");
            let claimed = match self.db.claim_operation(&op.op_id, now)? {
                // Waiting for an earlier content op of its file is not an attempt.
                ClaimOutcome::Gone | ClaimOutcome::Wait => continue,
                // The hand-over committed (spec §8.4); the successor itself still waits.
                ClaimOutcome::WaitAfterHandOver(took_over) => {
                    self.after_hand_over(&op, &took_over);
                    continue;
                }
                ClaimOutcome::Claimed(claimed) => {
                    if let Some(took_over) = &claimed.took_over {
                        self.after_hand_over(&claimed.op, took_over);
                    }
                    *claimed
                }
                ClaimOutcome::Parked {
                    op_id,
                    file_id,
                    reason,
                    took_over,
                } => {
                    if let Some(took_over) = &took_over {
                        self.after_hand_over(&op, took_over);
                    }
                    log_parked(&op_id, file_id.as_deref(), reason);
                    outcome.retried_op_ids.push(op_id);
                    continue;
                }
            };
            let op = claimed.op.clone();
            let result = self
                .execute_operation(&claimed, sync_root, now, &mut outcome.post_complete_errors)
                .await;
            self.seam("outcome:before_tx");
            // §8.6 rule 6: an upload whose completion is recorded never parks, is never paused
            // and is never abandoned: whatever failed, its retry repeats the landing (rule 4).
            // Write-keyed: only a File Provider write records a completion.
            let completed = matches!(&result, Err(error) if error.downcast_ref::<QueueStateMoved>().is_none())
                && claimed.write.is_some()
                && matches!(op.kind, OperationKind::UploadVersion | OperationKind::UploadFile)
                && self
                    .db
                    .get_upload_resume(&op.op_id)?
                    .and_then(|resume| resume.completed_version)
                    .is_some();
            match result {
                Ok(done) => {
                    let release = match done {
                        OpDone::Remove { release } => {
                            if !self
                                .db
                                .finish_claimed(&op.op_id, &claimed.claim_id, release.as_deref())?
                            {
                                log_queue_state_moved(&op.op_id, "landing");
                                continue;
                            }
                            release
                        }
                        // The landing transaction removed the op.
                        OpDone::Removed { release } => release,
                    };
                    if let Some(path) = release.as_deref()
                        && let Err(e) = crate::staged_payload::remove(&self.db, Path::new(path))
                    {
                        tracing::warn!(error = %e, "staged upload cleanup deferred; journal retained");
                    }
                    if let Some(file_id) = &op.file_id {
                        outcome.invalidated_item_ids.push(file_id.clone());
                    }
                    outcome.completed_op_ids.push(op.op_id);
                }
                // The attempt already logged the step that found the op gone.
                Err(error) if error.downcast_ref::<QueueStateMoved>().is_some() => continue,
                Err(error) if !completed && error.downcast_ref::<ParkNow>().is_some() => {
                    let reason = error
                        .downcast_ref::<ParkNow>()
                        .map(|park| park.0)
                        .unwrap_or(ParkReason::BaseUnknown);
                    if !self.db.park_claimed(&op.op_id, &claimed.claim_id, reason, now)? {
                        log_queue_state_moved(&op.op_id, "park");
                        continue;
                    }
                    log_parked(&op.op_id, op.file_id.as_deref(), reason);
                    outcome.retried_op_ids.push(op.op_id);
                }
                // A stale base never becomes valid again: the server's version only grows
                // (spec §8.4). Only a File Provider write parks at once; every other upload
                // keeps today's retries.
                Err(error)
                    if !completed
                        && init_conflict_class(&error) == Some(crate::api_client::InitConflictClass::StaleBase)
                        && claimed.write.is_some() =>
                {
                    if !self
                        .db
                        .park_claimed(&op.op_id, &claimed.claim_id, ParkReason::StaleBase, now)?
                    {
                        log_queue_state_moved(&op.op_id, "park");
                        continue;
                    }
                    log_refused_upload(&op, op.max_attempts, crate::api_client::InitConflictClass::StaleBase);
                    outcome.retried_op_ids.push(op.op_id);
                }
                Err(error) => {
                    let class = classify_operation_error(&error.to_string());
                    // m-11: a recorded completion is never paused (a busy database reads as
                    // `Locked`), because a paused op is not listed again.
                    if let Some(reason) = class.pause_reason().filter(|_| !completed) {
                        if !self.db.record_pause_claimed(
                            &op.op_id,
                            &claimed.claim_id,
                            reason,
                            Some(&error.to_string()),
                            now,
                        )? {
                            log_queue_state_moved(&op.op_id, "pause");
                            continue;
                        }
                        outcome.paused_op_ids.push(op.op_id);
                    } else {
                        // §8.6 rule 6: a recorded completion keeps retrying its landing on the
                        // normal backoff and never uses up its attempts.
                        let attempts = if completed {
                            op.attempts.saturating_add(1).min(op.max_attempts.saturating_sub(1))
                        } else {
                            op.attempts.saturating_add(1)
                        };
                        if completed {
                            log_completed_landing_retried(&op.op_id, op.file_id.as_deref(), attempts);
                        }
                        if matches!(op.kind, OperationKind::UploadVersion | OperationKind::UploadFile)
                            && error_http_status(&error) == Some(409)
                        {
                            let class =
                                init_conflict_class(&error).unwrap_or(crate::api_client::InitConflictClass::Other);
                            log_refused_upload(&op, attempts, class);
                        }
                        let next_retry_at = now.saturating_add(retry_delay_seconds(attempts));
                        if !self.db.record_attempt_claimed(
                            &op.op_id,
                            &claimed.claim_id,
                            attempts,
                            next_retry_at,
                            Some(&error.to_string()),
                        )? {
                            log_queue_state_moved(&op.op_id, "attempt");
                            continue;
                        }
                        if attempts >= op.max_attempts && !completed {
                            self.abandon_upload_after_give_up(&op).await;
                        }
                        outcome.retried_op_ids.push(op.op_id);
                    }
                }
            }
        }

        Ok(outcome)
    }

    /// The hand-over committed (spec §8.4): one line, then the retired copy is unlinked,
    /// after the commit (S6).
    fn after_hand_over(&self, successor: &PendingOperation, took_over: &TookOver) {
        log_took_over(&successor.op_id, successor.file_id.as_deref(), &took_over.parked_op_id);
        if let Some(path) = took_over.released_payload.as_deref()
            && let Err(e) = crate::staged_payload::remove(&self.db, Path::new(path))
        {
            tracing::warn!(error = %e, "staged upload cleanup deferred; journal retained");
        }
    }

    /// Run one claimed op, and say what the runner still does after it (`OpDone`).
    async fn execute_operation(
        &self,
        claimed: &ClaimedOp,
        sync_root: &Path,
        now: i64,
        post_complete_errors: &mut Vec<String>,
    ) -> anyhow::Result<OpDone> {
        let op = &claimed.op;
        match op.kind {
            OperationKind::PinTree => Ok(OpDone::Remove { release: None }),
            OperationKind::HydrateFile => {
                let file_id = op
                    .file_id
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("hydrate operation missing file_id"))?;
                let dest = if let Some(target_path) = op.target_path.as_deref() {
                    local_file_path_under_sync_root(sync_root, target_path)?
                } else if let Some(entry) = self.db.get_file(file_id)? {
                    local_file_path_under_sync_root(sync_root, &entry.path)?
                } else {
                    return Err(anyhow::anyhow!("hydrate operation target missing from state"));
                };
                self.hydrate_file(file_id, &dest, &[sync_root]).await?;
                Ok(OpDone::Remove { release: None })
            }
            OperationKind::CreateFolder => {
                let metadata = operation_metadata(op)?;
                let name = metadata["name_encrypted"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("create folder operation missing encrypted name"))?;
                self.api
                    .create_folder(name, op.parent_id.as_deref(), op.file_id.as_deref())
                    .await?;
                Ok(OpDone::Remove { release: None })
            }
            OperationKind::MoveFile | OperationKind::RenameFile => {
                let file_id = op
                    .file_id
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("metadata operation missing file_id"))?;
                let metadata = operation_metadata(op)?;
                let name = metadata["name_encrypted"].as_str();
                self.api.update_metadata(file_id, name, op.parent_id.as_deref()).await?;
                Ok(OpDone::Remove { release: None })
            }
            OperationKind::TrashFile => {
                let file_id = op
                    .file_id
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("trash operation missing file_id"))?;
                let activity_entry = self.db.get_file(file_id)?;
                // Server-authoritative deletion (task 0802). `trash_file` calls
                // `.error_for_status()`, so an `Ok` here means the server returned
                // HTTP 2xx for `DELETE /files/{id}` (it set `is_trashed=TRUE` and
                // emitted a `file_trash` sync op).
                //
                // We deliberately DO NOT delete the local row here. The row was
                // parked in `Trashing` by `watcher::handle_delete`; we KEEP it in
                // `Trashing` as the durable hidden-locally marker. Returning `Ok`
                // makes `process_due_operations` REMOVE this op — so the
                // `Trashing` status (not the op) is now what protects the row.
                //
                // Why not delete now: deleting + a same-tick / replica-lagged
                // `/sync/snapshot` that still lists the (just-trashed) file would
                // re-insert a FRESH `CloudOnly` row and re-mint the placeholder —
                // the "deleted file comes back" race. Instead we let convergence
                // happen authoritatively: while the snapshot still lists the file
                // the `Trashing` guard in `process_metadata_row` preserves it; once
                // the trash propagates the file is ABSENT from the snapshot and
                // `prune_absent` removes the (now op-less) `Trashing` row. If the
                // `file_trash` op echo arrives first, `apply_sync_op` deletes the
                // row directly — either path converges.
                self.api.trash_file(file_id).await?;
                if let Some(entry) = activity_entry {
                    record_moved_to_trash_activity(self.db.as_ref(), file_id, &entry.path, now)?;
                }
                // Trace logs keep only the opaque id; plaintext names stay in the
                // local state DB, where file paths already live.
                tracing::info!(
                    file_id = %file_id,
                    "trash op: server DELETE /files/{{id}} returned 2xx — keeping row Trashing, dropping op"
                );
                Ok(OpDone::Remove { release: None })
            }
            OperationKind::RestoreFile => {
                let file_id = op
                    .file_id
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("restore file operation missing file_id"))?;
                self.api.restore_file(file_id).await?;
                Ok(OpDone::Remove { release: None })
            }
            OperationKind::RestoreVersion => {
                let file_id = op
                    .file_id
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("restore operation missing file_id"))?;
                let metadata = operation_metadata(op)?;
                let version_id = metadata["version_id"]
                    .as_str()
                    .or(op.base_object_version_id.as_deref())
                    .ok_or_else(|| anyhow::anyhow!("restore operation missing version id"))?;
                // Spec §5.4 row 10: the restore's version becomes current, so the
                // system re-downloads the restored bytes instead of keeping a token.
                #[cfg(target_os = "macos")]
                {
                    let response = self.api.restore_version(file_id, version_id).await?;
                    self.seam("restore:after_server");
                    // The server has restored, and each restore mints a version: a
                    // retry would make another. A local failure is left to a snapshot
                    // to repair, and the op completes. A fixed category only: the
                    // error can carry a path (spec §11).
                    if self
                        .db
                        .apply_restore_response(
                            file_id,
                            response["version_number"].as_i64(),
                            response["current_object_version_id"].as_str(),
                        )
                        .is_err()
                    {
                        tracing::warn!(
                            file_id = %file_id,
                            category = "restore_bookkeeping_failed",
                            "restore done on the server; its local record failed, a snapshot will repair it"
                        );
                        if self.db.request_resnapshot().is_err() {
                            tracing::warn!(
                                file_id = %file_id,
                                category = "resnapshot_request_failed",
                                "restore done on the server; a snapshot could not be requested"
                            );
                        }
                    }
                }
                #[cfg(not(target_os = "macos"))]
                {
                    self.api.restore_version(file_id, version_id).await?;
                }
                Ok(OpDone::Remove { release: None })
            }
            OperationKind::UploadVersion | OperationKind::UploadFile => {
                self.upload_version(op, Some(claimed), sync_root, post_complete_errors)
                    .await
            }
        }
    }

    /// Upload one op's staged bytes. `claim` is the runner's claim of the op; every
    /// queue write the upload makes is guarded by it (spec §8.7 S4). `None` is Keep
    /// Mine's inline run, outside the queue: its writes stay unguarded. The `release` of
    /// the returned `OpDone` is, on macOS, the staged payload, which the caller unlinks
    /// once the op is gone.
    async fn upload_version(
        &self,
        op: &PendingOperation,
        claim: Option<&ClaimedOp>,
        #[cfg_attr(not(target_os = "windows"), allow(unused_variables))] sync_root: &Path,
        post_complete_errors: &mut Vec<String>,
    ) -> anyhow::Result<OpDone> {
        let local_file_id = op
            .file_id
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("upload operation missing file_id"))?;
        let previous_status = self.db.get_file(local_file_id)?.map(|entry| entry.status);
        self.db.set_status(local_file_id, FileStatus::Uploading)?;

        struct Rollback<'a> {
            db: &'a StateDb,
            id: &'a str,
            previous: Option<FileStatus>,
        }
        impl Drop for Rollback<'_> {
            fn drop(&mut self) {
                // A server-completed upload has already committed Local; don't
                // undo that if cancellation happens during thumbnail work.
                if self
                    .db
                    .get_file(self.id)
                    .ok()
                    .flatten()
                    .is_some_and(|r| r.status == FileStatus::Uploading)
                {
                    let status = if self.previous == Some(FileStatus::Conflict) {
                        FileStatus::Conflict
                    } else {
                        FileStatus::Error
                    };
                    let _ = self.db.set_status(self.id, status);
                }
            }
        }
        let _rollback = Rollback {
            db: &self.db,
            id: local_file_id,
            previous: previous_status,
        };
        self.do_upload_version(local_file_id, op, claim, sync_root, post_complete_errors)
            .await
    }

    async fn do_upload_version(
        &self,
        local_file_id: &str,
        op: &PendingOperation,
        claim: Option<&ClaimedOp>,
        #[cfg_attr(not(target_os = "windows"), allow(unused_variables))] sync_root: &Path,
        post_complete_errors: &mut Vec<String>,
    ) -> anyhow::Result<OpDone> {
        // §8.6 rules 4 and 6: a recorded completion lands without the network, before anything
        // else is checked, because it never parks, not even on a missing payload. Write-keyed:
        // a completion is recorded only for a File Provider write, so only one can be found
        // here; Windows and watcher uploads never take this shortcut.
        if claim.is_some_and(|claim| claim.write.is_some())
            && let Some(previous) = self.db.get_upload_resume(&op.op_id)?
            && let (Some(version), Some(object)) =
                (previous.completed_version, previous.completed_object_version_id.clone())
        {
            // The save's own content type, as the first attempt had it, and the server's
            // `mime_type` recorded with the completion, its fallback.
            let content_type = op
                .metadata_json
                .as_deref()
                .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
                .and_then(|metadata| metadata["content_type"].as_str().map(str::to_string));
            return self
                .land(
                    local_file_id,
                    op,
                    claim,
                    &previous.server_file_id,
                    version,
                    &object,
                    previous.payload_size as u64,
                    content_type,
                    previous.completed_mime_type.clone(),
                    post_complete_errors,
                    sync_root,
                )
                .await;
        }
        let payload_path = op
            .payload_path
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("upload operation missing staged payload"))?;
        let payload_path = Path::new(payload_path);
        match std::fs::metadata(payload_path) {
            Ok(meta) if meta.is_file() => {}
            // The staged copy lives in the app's own container and cannot come back
            // (spec §8.4). Only a File Provider write parks on it.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && claim.and_then(|c| c.write.as_ref()).is_some() => {
                return Err(anyhow::Error::new(ParkNow(ParkReason::PayloadMissing)));
            }
            // An op without a write id keeps today's retry and today's text
            // (`test_process_due_operations_records_retry_for_upload_worker_handoff`).
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(anyhow::anyhow!(
                    "staged upload payload is missing: {}",
                    payload_path.display()
                ));
            }
            // Every other read error (permission, I/O, not a regular file) retries as
            // today (spec §8.4), with a fixed text. Neither the path nor the I/O error
            // reaches `classify_operation_error`: it matches bare substrings such as
            // `403` and `permission`, and would pause the op instead.
            _ => return Err(anyhow::anyhow!("staged upload payload could not be read")),
        }

        let metadata = operation_metadata(op)?;
        let name_encrypted = metadata["name_encrypted"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("upload operation missing encrypted name"))?;
        let content_type = metadata["content_type"].as_str().map(str::to_string);
        // An empty (0-byte) file is a normal upload: the canonical plan for it is
        // ONE chunk carrying the AEAD of zero bytes (`plan_chunks(0, _)` →
        // `chunk_count == 1`), which the chunk loop below sends as a 28-byte
        // nonce + tag. Requires a server whose `/uploads/init` accepts
        // `file_size_bytes: 0`.
        let plaintext_size = std::fs::metadata(payload_path)?.len();

        let is_create = is_create_file_operation(&metadata);
        let payload_path_str = payload_path.to_string_lossy().into_owned();
        let payload_mtime_ns = payload_mtime_ns(payload_path);

        // Flow 7: resume the persisted upload session when the retry is for the
        // SAME staged bytes. Re-running `init` would mint a second server file
        // row (the first left behind as a broken `is_uploading` duplicate) and
        // re-send every chunk from zero.
        let mut session: Option<UploadResume> = None;
        if let Some(previous) = self.db.get_upload_resume(&op.op_id)? {
            let same_payload = previous.payload_path == payload_path_str
                && previous.payload_size == plaintext_size as i64
                && previous.payload_mtime_ns == payload_mtime_ns
                && previous.chunk_size_bytes > 0
                && previous.chunk_count > 0
                && previous.acked_chunks >= 0
                && previous.acked_chunks <= previous.chunk_count;
            if same_payload {
                tracing::info!(
                    op_id = %op.op_id,
                    file_id = %previous.server_file_id,
                    acked_chunks = previous.acked_chunks,
                    chunk_count = previous.chunk_count,
                    "upload: resuming persisted upload session"
                );
                session = Some(previous);
            } else {
                tracing::info!(
                    op_id = %op.op_id,
                    file_id = %previous.server_file_id,
                    "upload: staged payload changed since the persisted session — abandoning it"
                );
                self.abandon_upload_session(&previous).await?;
            }
        }

        let session = match session {
            Some(session) => session,
            None => {
                guard_init_base(
                    op.base_version,
                    is_create,
                    claim.and_then(|claim| claim.write.as_ref()).is_some(),
                )?;
                let init_request = upload_init_request_for_operation(
                    local_file_id,
                    name_encrypted,
                    content_type.clone(),
                    op.parent_id.clone(),
                    plaintext_size,
                    op.base_version,
                    is_create,
                );
                let upload = self.api.init_upload(&init_request).await?;
                if upload.chunk_size_bytes <= 0 || upload.chunk_count <= 0 {
                    return Err(anyhow::anyhow!("upload init returned invalid chunk plan"));
                }
                let session = UploadResume {
                    op_id: op.op_id.clone(),
                    payload_path: payload_path_str.clone(),
                    payload_size: plaintext_size as i64,
                    payload_mtime_ns,
                    upload_session_id: upload.upload_session_id,
                    server_file_id: upload.file_id,
                    object_version_id: upload.object_version_id,
                    chunk_size_bytes: upload.chunk_size_bytes,
                    chunk_count: upload.chunk_count,
                    acked_chunks: 0,
                    metadata_applied: false,
                    // Only a create (no `file_id` sent to init) owns the server
                    // row outright; a replace targets the user's existing file.
                    is_create: init_request.file_id.is_none(),
                    completed_version: None,
                    completed_object_version_id: None,
                    completed_mime_type: None,
                };
                // Persist BEFORE the next await: a cut anywhere after init must
                // leave the session discoverable by the retry.
                match claim {
                    Some(claim) => {
                        if !self.db.put_upload_resume_claimed(&session, &claim.claim_id)? {
                            log_queue_state_moved(&op.op_id, "resume");
                            return Err(anyhow::Error::new(QueueStateMoved));
                        }
                    }
                    // Keep Mine runs inline, outside the queue: no claim guards it.
                    None => self.db.put_upload_resume(&session)?,
                }
                session
            }
        };

        let body = self
            .upload_session_body(
                local_file_id,
                op,
                &metadata,
                name_encrypted,
                content_type.clone(),
                payload_path,
                &session,
            )
            .await;
        match body {
            Ok(completed) => {
                // §8.6.1: the version this upload produced, from the server's reply. An
                // idempotent repeat answers `already_completed` with neither field; its version
                // is then base + 1 (1 for a create), its object id the session's. An upload
                // without a base (Windows and the watcher) keeps the stored version + 1.
                let produced_version = match completed["version_number"].as_i64() {
                    Some(version) => version,
                    None if is_create => 1,
                    None => match op.base_version {
                        Some(base) => base.saturating_add(1),
                        None => self
                            .db
                            .get_file_contract_state(local_file_id)?
                            .map_or(0, |contract| contract.current_version)
                            .saturating_add(1),
                    },
                };
                let produced_object = completed["current_object_version_id"]
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| session.object_version_id.clone());
                let mime_type = completed["mime_type"].as_str().map(str::to_string);
                if let Some(claim) = claim
                    && claim.write.is_some() // write-keyed
                    && !self.db.record_completion_claimed(
                        &op.op_id,
                        &claim.claim_id,
                        produced_version,
                        &produced_object,
                        mime_type.as_deref(),
                    )?
                {
                    log_queue_state_moved(&op.op_id, "completion");
                    return Err(anyhow::Error::new(QueueStateMoved));
                }
                let size = completed["size_bytes"].as_u64().unwrap_or(plaintext_size);
                self.land(
                    local_file_id,
                    op,
                    claim,
                    &session.server_file_id,
                    produced_version,
                    &produced_object,
                    size,
                    content_type,
                    mime_type,
                    post_complete_errors,
                    sync_root,
                )
                .await
            }
            Err(error) => {
                if upload_session_is_gone(&error) {
                    tracing::warn!(
                        op_id = %op.op_id,
                        file_id = %session.server_file_id,
                        error = %error,
                        "upload: server no longer accepts the persisted session — abandoning it; the retry starts a fresh upload"
                    );
                    self.abandon_upload_session(&session).await?;
                }
                Err(error)
            }
        }
    }

    /// The landing (spec §8.6.2): one transaction (`StateDb::apply_landing`), then the work
    /// that follows its commit. Keep Mine passes `claim: None`: the landing removes nothing
    /// it does not own, and the caller unlinks the released copy.
    #[allow(clippy::too_many_arguments)]
    async fn land(
        &self,
        local_file_id: &str,
        op: &PendingOperation,
        claim: Option<&ClaimedOp>,
        server_file_id: &str,
        produced_version: i64,
        produced_object: &str,
        size: u64,
        content_type: Option<String>,
        mime_type: Option<String>,
        post_complete_errors: &mut Vec<String>,
        #[cfg_attr(not(target_os = "windows"), allow(unused_variables))] sync_root: &Path,
    ) -> anyhow::Result<OpDone> {
        self.seam("landing:after_complete");
        let payload_path = op.payload_path.clone();
        // macOS only: `release` says only what the caller unlinks after the commit (S6).
        // Elsewhere `finish_completed_upload` unlinks the copy, as before.
        let release = if cfg!(target_os = "macos") {
            payload_path.clone()
        } else {
            None
        };
        // Windows: a create's finalization journal row, read from local state only, and
        // written in the landing's own transaction (the journal then owns the retry).
        #[cfg(target_os = "windows")]
        let finalization = match payload_path.as_deref() {
            Some(path) => self.defer_local_upload_finalization(op, server_file_id, sync_root, Path::new(path))?,
            None => None,
        };
        #[cfg(not(target_os = "windows"))]
        let finalization: Option<crate::state_db::UploadFinalization> = None;
        let master_key = self.api.master_key();
        let landed = self.db.apply_landing(
            &crate::state_db::LandingInput {
                op_id: &op.op_id,
                claim_id: claim.map(|c| c.claim_id.as_str()),
                write_id: claim.and_then(|c| c.write.as_ref()).map(|w| w.write_id.as_str()),
                local_file_id,
                server_file_id,
                target_path: op.target_path.as_deref(),
                parent_id: op.parent_id.as_deref(),
                landed_base: op.base_version,
                produced_version,
                produced_object_version_id: produced_object,
                size_bytes: i64::try_from(size).unwrap_or(i64::MAX),
                content_type: content_type.as_deref(),
                mime_type: mime_type.as_deref(),
                // Always the op's payload path, whatever `release` is.
                completed_payload: payload_path.as_deref(),
                finalization: finalization.as_ref(),
                now: now_secs(),
            },
            &|queued, server_id| metadata_rekeyed_to(master_key, queued, server_id),
        )?;
        let Some(landed) = landed else {
            log_queue_state_moved(&op.op_id, "landing");
            return Err(anyhow::Error::new(QueueStateMoved));
        };
        for (successor, reason) in &landed.parked_successors {
            log_parked(successor, Some(server_file_id), *reason);
        }
        self.record_transfer_done(crate::transfer_progress::Direction::Up, server_file_id, size);
        if let Some(path) = payload_path.as_deref() {
            // Task 1700: post-complete thumbnail work never fails the upload, but its
            // failure is surfaced on the outcome so a red run names the real error.
            let file_key = file_key_for(self.api.master_key(), server_file_id);
            if let Err(e) = self
                .finish_completed_upload(op, server_file_id, Path::new(path), content_type, &file_key, sync_root)
                .await
            {
                tracing::warn!(
                    file_id = %server_file_id,
                    error = %e,
                    "upload-time thumbnail generation/upload skipped"
                );
                post_complete_errors.push(format!("{}: {e}", op.op_id));
            }
        }
        Ok(if claim.is_some() {
            OpDone::Removed { release }
        } else {
            OpDone::Remove { release }
        })
    }

    /// Everything between init and a successful `complete`: the post-init
    /// metadata PATCH and the chunk PUTs from the acknowledged watermark on.
    /// Every step's success is persisted before the next await so a cut
    /// resumes exactly where the server's acknowledgements stop.
    #[allow(clippy::too_many_arguments)]
    async fn upload_session_body(
        &self,
        local_file_id: &str,
        op: &PendingOperation,
        metadata: &serde_json::Value,
        name_encrypted: &str,
        content_type: Option<String>,
        payload_path: &Path,
        session: &UploadResume,
    ) -> anyhow::Result<serde_json::Value> {
        let server_file_id = session.server_file_id.clone();
        if !session.metadata_applied {
            let effective_name_encrypted = if server_file_id != local_file_id {
                encrypted_metadata_for_name(
                    self.api.master_key(),
                    &server_file_id,
                    metadata_display_name(metadata, op)
                        .as_deref()
                        .unwrap_or(&server_file_id),
                    content_type.as_deref(),
                )?
            } else {
                name_encrypted.to_string()
            };
            self.api
                .update_metadata(
                    &server_file_id,
                    Some(&effective_name_encrypted),
                    op.parent_id.as_deref(),
                )
                .await?;
            self.db.set_upload_resume_metadata_applied(&op.op_id)?;
        }

        let file_key = file_key_for(self.api.master_key(), &server_file_id);

        let mut file = std::fs::File::open(payload_path)?;
        let chunk_size = session.chunk_size_bytes as usize;
        let chunk_count = session.chunk_count as u64;
        if chunk_size == 0 || chunk_count == 0 {
            return Err(anyhow::anyhow!("upload init returned invalid chunk plan"));
        }
        let first_chunk = session.acked_chunks.max(0) as u64;
        if first_chunk > 0 {
            use std::io::Seek;
            file.seek(std::io::SeekFrom::Start(first_chunk * chunk_size as u64))?;
        }
        let mut buffer = vec![0u8; chunk_size];
        // Task 1683 slice 2: per-file bytes for the popover. Keyed by the LOCAL file
        // id (the `files` row the activity list reads); a resumed session starts at
        // its acknowledged watermark, not at 0.
        let payload_total = u64::try_from(session.payload_size).unwrap_or(0);
        let progress = self
            .transfers
            .begin(local_file_id, crate::transfer_progress::Direction::Up, payload_total);
        progress.update(crate::transfer_progress::uploaded_bytes(
            first_chunk,
            chunk_size as u64,
            payload_total,
        ));
        // Rate-limit ceiling: read once per file, not per chunk (config is on
        // disk but the file is fast to parse; at most one read per upload op).
        let upload_kbps_limit = crate::config::DesktopConfig::load()
            .map(|c| c.upload_kbps_limit)
            .unwrap_or(0);

        for chunk_index in first_chunk..chunk_count {
            let read = read_full_chunk(&mut file, &mut buffer)?;
            // A zero-length read is only legitimate for the single chunk of an
            // empty file; anywhere else the staged payload is truncated.
            if read == 0 && session.payload_size > 0 {
                return Err(anyhow::anyhow!(
                    "staged upload ended before expected chunk {} of {}",
                    chunk_index + 1,
                    chunk_count
                ));
            }
            let encrypted = beebeeb_core::encrypt::encrypt_chunk_raw(&file_key, &buffer[..read])
                .map_err(|e| anyhow::anyhow!("encrypt upload chunk {chunk_index}: {e}"))?;
            let chunk_start = std::time::Instant::now();
            self.api
                .upload_session_chunk(&session.upload_session_id, chunk_index as u32, &encrypted)
                .await?;
            self.db.set_upload_resume_acked(&op.op_id, chunk_index as i64 + 1)?;
            progress.update(crate::transfer_progress::uploaded_bytes(
                chunk_index + 1,
                chunk_size as u64,
                payload_total,
            ));

            // P1 — wire-byte counter: count plaintext bytes (the user-data rate).
            self.wire.upload_bytes.fetch_add(read as u64, Ordering::Relaxed);

            // E — token-bucket pacing: if a limit is set and the chunk transferred
            // faster than the budget allows, sleep the remainder.
            if upload_kbps_limit > 0 {
                let budget_secs = read as f64 / (upload_kbps_limit as f64 * 1024.0);
                let elapsed_secs = chunk_start.elapsed().as_secs_f64();
                if budget_secs > elapsed_secs {
                    let sleep_ms = ((budget_secs - elapsed_secs) * 1000.0) as u64;
                    if sleep_ms > 0 {
                        tokio::time::sleep(Duration::from_millis(sleep_ms)).await;
                    }
                }
            }
        }

        let completed = self.api.complete_upload_session(&session.upload_session_id).await;
        if completed.is_ok() {
            progress.finish();
        }
        completed
    }

    /// Post-`complete` best-effort work: thumbnails, staged-payload cleanup and
    /// (Windows) placeholder conversion. Never fails the upload — but the
    /// thumbnail outcome is RETURNED (task 1700) instead of being swallowed
    /// into a `tracing::warn!` that no test subscriber captures, so callers
    /// can surface it for diagnostics. Cleanup and Windows finalization stay
    /// warn-only inside.
    async fn finish_completed_upload(
        &self,
        #[cfg_attr(not(target_os = "windows"), allow(unused_variables))] op: &PendingOperation,
        server_file_id: &str,
        payload_path: &Path,
        thumbnail_content_type: Option<String>,
        file_key: &beebeeb_core::kdf::FileKey,
        #[cfg_attr(not(target_os = "windows"), allow(unused_variables))] sync_root: &Path,
    ) -> anyhow::Result<()> {
        let server_file_id = server_file_id.to_string();
        let thumbnails = self
            .upload_thumbnails_for_plaintext_media(
                &server_file_id,
                payload_path,
                thumbnail_content_type.as_deref(),
                file_key,
            )
            .await;

        #[cfg(target_os = "windows")]
        {
            crate::windows_cf::upload_finalization::retry(&self.db, sync_root);
            // A failed stamp or unlink retains its durable completed proof.
            match self.db.upload_finalizations() {
                Ok(rows) if rows.iter().any(|row| row.op_id == op.op_id) => return thumbnails,
                Err(e) => {
                    tracing::warn!(error = %e, "cannot inspect finalization journal");
                    return thumbnails;
                }
                _ => {}
            }
        }
        // On macOS the caller releases the payload once the op's removal has
        // committed (spec §8.7 S6).
        #[cfg(not(target_os = "macos"))]
        {
            if let Err(e) = crate::staged_payload::remove(&self.db, payload_path) {
                tracing::warn!(error = %e, "staged upload cleanup deferred; journal retained");
            }
        }
        thumbnails
    }

    /// Give up on a persisted upload session (payload changed, session gone,
    /// or the op exhausted its retries). For a CREATE the server row minted by
    /// `init` holds no completed content and is only a broken `is_uploading`
    /// duplicate, so it is trashed (best-effort: the server's stale-upload
    /// sweep still hard-deletes it after 7 days if this fails). A REPLACE
    /// targets the user's existing file and is never trashed. The resume row
    /// is always dropped so the next attempt starts a fresh session.
    async fn abandon_upload_session(&self, session: &UploadResume) -> anyhow::Result<()> {
        if session.is_create {
            match self.api.trash_file(&session.server_file_id).await {
                Ok(_) => tracing::info!(
                    op_id = %session.op_id,
                    file_id = %session.server_file_id,
                    "upload: trashed orphaned in-progress server row of an abandoned create"
                ),
                Err(e) => tracing::warn!(
                    op_id = %session.op_id,
                    file_id = %session.server_file_id,
                    error = %e,
                    "upload: could not trash orphaned in-progress server row; the server's stale-upload sweep reaps it"
                ),
            }
        }
        self.db.clear_upload_resume(&session.op_id)?;
        Ok(())
    }

    /// Called when an upload op has exhausted its retries: nothing will ever
    /// resume its session, so abandon it now instead of leaving the orphan
    /// visible for the server's 7-day stale-upload window.
    async fn abandon_upload_after_give_up(&self, op: &PendingOperation) {
        if !matches!(op.kind, OperationKind::UploadVersion | OperationKind::UploadFile) {
            return;
        }
        match self.db.get_upload_resume(&op.op_id) {
            Ok(Some(session)) => {
                if let Err(e) = self.abandon_upload_session(&session).await {
                    tracing::warn!(op_id = %op.op_id, error = %e, "upload: failed to abandon given-up session");
                }
            }
            Ok(None) => {}
            Err(e) => tracing::warn!(op_id = %op.op_id, error = %e, "upload: failed to read resume state"),
        }
    }

    async fn upload_thumbnails_for_plaintext_media(
        &self,
        server_file_id: &str,
        payload_path: &Path,
        mime_type: Option<&str>,
        file_key: &beebeeb_core::kdf::FileKey,
    ) -> anyhow::Result<()> {
        let uploads = prepare_thumbnail_uploads_for_plaintext_media(payload_path, mime_type, file_key)?;
        for upload in uploads {
            self.api
                .upload_thumbnail(
                    server_file_id,
                    upload.variant.label(),
                    &upload.encrypted,
                    upload.blurhash.as_deref(),
                )
                .await
                .map_err(|e| anyhow::anyhow!("upload {} thumbnail: {e}", upload.variant.label()))?;
        }
        Ok(())
    }

    /// The journal row that defers a create's native stamping past the landing, or `None`
    /// when there is nothing on disk to stamp. The landing writes it in its own transaction,
    /// before any cancellable post-completion work, so the op is never gone without it.
    #[cfg(target_os = "windows")]
    fn defer_local_upload_finalization(
        &self,
        op: &PendingOperation,
        server_file_id: &str,
        sync_root: &Path,
        payload_path: &Path,
    ) -> anyhow::Result<Option<crate::state_db::UploadFinalization>> {
        let is_create = op
            .metadata_json
            .as_deref()
            .and_then(|m| serde_json::from_str::<serde_json::Value>(m).ok())
            .is_some_and(|m| m["operation"].as_str() == Some("create_file"));
        if !is_create {
            return Ok(None);
        }
        let Some(target_path) = op.target_path.as_deref() else {
            return Ok(None);
        };
        let on_disk = local_file_path_under_sync_root(sync_root, target_path)?;
        if !on_disk.is_file() {
            return Ok(None);
        }
        Ok(Some(crate::state_db::UploadFinalization {
            op_id: op.op_id.clone(),
            local_file_id: op.file_id.as_deref().unwrap_or(server_file_id).to_string(),
            server_file_id: server_file_id.to_string(),
            target_path: target_path.to_string(),
            payload_path: payload_path.to_string_lossy().into_owned(),
            stamped: false,
        }))
    }

    fn decrypted_direct_shared_root_name(&self, root: &SharedRootMapping) -> Option<String> {
        let sender_public_key = root.sender_public_key.as_deref()?;
        let encrypted_file_key = root.encrypted_file_key.as_deref()?;
        let name_encrypted = root.file_name_encrypted.as_deref()?;
        let file_key = unwrap_direct_shared_file_key(
            self.api.master_key(),
            sender_public_key,
            &root.file_id,
            encrypted_file_key,
        )
        .ok()?;
        decrypt_shared_name_with_key(&file_key, name_encrypted)
    }

    async fn shared_folder_material(
        &self,
        root: &SharedRootMapping,
    ) -> anyhow::Result<(Zeroizing<[u8; 32]>, serde_json::Value)> {
        let sender_public_key = root
            .sender_public_key
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("folder share {} is missing sender_public_key", root.invite_id))?;
        let encrypted_folder_key = root
            .encrypted_folder_key
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("folder share {} is missing encrypted_folder_key", root.invite_id))?;
        let folder_key = unwrap_folder_share_key(
            self.api.master_key(),
            sender_public_key,
            &root.file_id,
            encrypted_folder_key,
        )?;
        let folder_keys_response = self.api.get_folder_keys(&root.invite_id).await?;
        Ok((folder_key, folder_keys_response))
    }

    fn decrypted_folder_root_name(
        &self,
        root: &SharedRootMapping,
        folder_key: &[u8; 32],
        folder_keys_response: &serde_json::Value,
    ) -> Option<String> {
        let name_encrypted = root.file_name_encrypted.as_deref()?;
        let encrypted_file_key = encrypted_file_key_from_folder_keys_response(folder_keys_response, &root.file_id)?;
        let file_key = unwrap_child_file_key(folder_key, encrypted_file_key).ok()?;
        decrypt_shared_name_with_key(&file_key, name_encrypted)
    }

    fn shared_root_display_name(
        &self,
        root: &SharedRootMapping,
        folder_key: Option<&[u8; 32]>,
        folder_keys_response: Option<&serde_json::Value>,
    ) -> String {
        let item_kind = if root.is_folder {
            ItemKind::Folder
        } else {
            ItemKind::File
        };
        let decrypted = if root.is_folder {
            folder_key
                .zip(folder_keys_response)
                .and_then(|(key, keys_response)| self.decrypted_folder_root_name(root, key, keys_response))
        } else {
            self.decrypted_direct_shared_root_name(root)
        };

        decrypted
            .as_deref()
            .and_then(safe_shared_leaf_name)
            .unwrap_or_else(|| shared_placeholder_name(&item_kind).to_string())
    }

    async fn refresh_shared_folder_children(
        &self,
        root: &SharedRootMapping,
        root_display_name: &str,
        folder_key: &[u8; 32],
        folder_keys_response: &serde_json::Value,
        now: i64,
    ) -> anyhow::Result<()> {
        let key_map = folder_keys_map(folder_keys_response);
        let root_path = format!("Shared with me/{root_display_name}");
        let mut queue: VecDeque<(Option<String>, String, String)> =
            VecDeque::from([(None, root.file_id.clone(), root_path)]);

        while let Some((api_parent_id, db_parent_id, parent_path)) = queue.pop_front() {
            let children = self
                .api
                .list_shared_folder_files(&root.file_id, api_parent_id.as_deref())
                .await?;

            for child in children {
                let Some(child_id) = child["id"].as_str().map(str::to_string) else {
                    continue;
                };
                let child_file_key = key_map
                    .get(&child_id)
                    .and_then(|encrypted| unwrap_child_file_key(folder_key, encrypted).ok());
                let Some(entry) = apply_shared_metadata_file_row(
                    &self.db,
                    &child,
                    root,
                    now,
                    Some(db_parent_id.clone()),
                    &parent_path,
                    child_file_key.as_ref(),
                )?
                else {
                    continue;
                };

                if entry.item_kind == ItemKind::Folder {
                    queue.push_back((Some(child_id.clone()), child_id, entry.path));
                }
            }
        }

        Ok(())
    }

    pub async fn refresh_shared_roots(&self) -> anyhow::Result<SharedRootRefreshOutcome> {
        let body = self.api.list_shared_roots().await?;
        let roots = shared_roots_from_invite_response(&body);
        let active_shared_root_ids: Vec<String> = roots.iter().map(|root| root.file_id.clone()).collect();
        let now = now_secs();

        for root in &roots {
            let mut folder_material = None;
            if root.is_folder {
                match self.shared_folder_material(root).await {
                    Ok((folder_key, folder_keys_response)) => {
                        folder_material = Some((folder_key, folder_keys_response));
                    }
                    Err(e) => {
                        tracing::warn!(
                            invite_id = %root.invite_id,
                            error = %e,
                            "shared folder key unwrap failed during refresh"
                        );
                    }
                }
            }
            let display_name = self.shared_root_display_name(
                root,
                folder_material.as_ref().map(|(folder_key, _)| &**folder_key),
                folder_material.as_ref().map(|(_, keys_response)| keys_response),
            );
            let root_path = format!("Shared with me/{display_name}");
            crate::reject_unsafe_rel_path(&root_path)
                .map_err(|e| anyhow::anyhow!("unsafe shared root path for {}: {e}", root.file_id))?;

            self.db.upsert_file(&FileEntry {
                file_id: root.file_id.clone(),
                path: root_path,
                status: FileStatus::CloudOnly,
                size_bytes: root.size_bytes,
                modified_at: root.approved_at.unwrap_or(now),
                content_hash: None,
                remote_updated_at: root.approved_at.unwrap_or(0),
                // upsert_file does not persist these; the contract update
                // just below sets the authoritative parent_id/item_kind.
                parent_id: None,
                item_kind: if root.is_folder {
                    ItemKind::Folder
                } else {
                    ItemKind::File
                },
            })?;

            let mut contract = self
                .db
                .get_file_contract_state(&root.file_id)?
                .ok_or_else(|| anyhow::anyhow!("missing state row for shared root {}", root.file_id))?;
            contract.namespace = Namespace::SharedWithMe;
            contract.parent_id = None;
            contract.shared_root_id = Some(root.file_id.clone());
            contract.share_id = Some(root.invite_id.clone());
            contract.owner_email = root.owner_email.clone();
            contract.permission_bits = root.permission_bits;
            contract.item_kind = if root.is_folder {
                ItemKind::Folder
            } else {
                ItemKind::File
            };
            contract.content_type = root.content_type.clone();
            contract.last_sync_at = now;
            self.db.set_file_contract_state(&contract)?;

            if let Some((folder_key, folder_keys_response)) = folder_material
                && let Err(e) = self
                    .refresh_shared_folder_children(root, &display_name, &folder_key, &folder_keys_response, now)
                    .await
            {
                tracing::warn!(
                    invite_id = %root.invite_id,
                    error = %e,
                    "shared folder child refresh failed"
                );
            }
        }

        let removed = self.db.purge_revoked_shared_content(&active_shared_root_ids)?;
        let mut removed_shared_file_ids = Vec::new();
        let mut removed_cache_paths = Vec::new();
        for cache in removed {
            removed_shared_file_ids.push(cache.file_id);
            if let Some(path) = cache.cache_path {
                if let Err(e) = std::fs::remove_file(&path) {
                    if e.kind() != std::io::ErrorKind::NotFound {
                        tracing::warn!(path = %path, error = %e, "failed to remove revoked shared cache file");
                    }
                }
                removed_cache_paths.push(path);
            }
        }

        Ok(SharedRootRefreshOutcome {
            active_shared_root_ids,
            removed_shared_file_ids,
            removed_cache_paths,
        })
    }

    pub fn queue_finder_create(&self, target: FinderWriteTarget) -> anyhow::Result<FinderWriteOutcome> {
        self.queue_finder_create_from(target, None)
    }

    /// [`Self::queue_finder_create`], reading a file's contents from
    /// `contents` when given: a file the caller already opened and checked
    /// (the macOS handed-over copy, see `ipc_socket::open_staged_contents`).
    /// `target.contents_path` is then only the journal's label for it, and is
    /// never opened.
    pub fn queue_finder_create_from(
        &self,
        target: FinderWriteTarget,
        contents: Option<&std::fs::File>,
    ) -> anyhow::Result<FinderWriteOutcome> {
        let Some(start) = self.start_finder_create(&target)? else {
            return Ok(ignored_finder_item(&target.filename));
        };
        match target.kind {
            FinderWriteItemKind::Folder => {
                let FinderCreateStart {
                    parent_contract,
                    file_id,
                    rel_path,
                } = start;
                let metadata = encrypted_metadata_for_name(self.api.master_key(), &file_id, &target.filename, None)?;
                // Write the local folder row + contract synchronously, the same
                // way the File branch writes its `Uploading` row, so a nested
                // child dispatched later in the SAME scan resolves this folder as
                // its parent (`resolve_parent_id_for` reads `files.item_kind`)
                // without waiting for a server `/sync` round-trip. `Local` =
                // present on disk, server CreateFolder op pending in the queue.
                self.db.upsert_file(&FileEntry {
                    file_id: file_id.clone(),
                    path: rel_path.clone(),
                    status: FileStatus::Local,
                    size_bytes: 0,
                    modified_at: now_secs(),
                    content_hash: None,
                    remote_updated_at: 0,
                    // Not persisted by upsert_file; the contract update owns these.
                    parent_id: None,
                    item_kind: ItemKind::Folder,
                })?;
                if let Some(mut contract) = self.db.get_file_contract_state(&file_id)? {
                    contract.item_kind = ItemKind::Folder;
                    contract.parent_id = target.parent_id.clone();
                    self.db.set_file_contract_state(&contract)?;
                }
                let mut payload = serde_json::json!({
                    "operation": "create_folder",
                    "name_encrypted": metadata,
                });
                apply_shared_context(&mut payload, parent_contract.as_ref());
                self.enqueue_finder_operation(
                    OperationKind::CreateFolder,
                    Some(file_id),
                    target.parent_id,
                    Some(rel_path),
                    payload,
                    None,
                    None,
                    None,
                )
            }
            FinderWriteItemKind::File => {
                let PreparedFinderCreate {
                    file_id,
                    rel_path,
                    staged,
                    staged_path,
                    row,
                    payload,
                    ..
                } = self.prepare_finder_create(&target, contents, start)?;
                self.db.upsert_file(&row)?;
                let outcome = self.enqueue_finder_operation(
                    OperationKind::UploadVersion,
                    Some(file_id),
                    target.parent_id,
                    // target_path = the FULL relative key, so
                    // defer_local_upload_finalization joins sync_root + this
                    // and finds the nested file on disk to re-stamp it in-sync.
                    Some(rel_path),
                    payload,
                    Some(staged_path),
                    None,
                    None,
                )?;
                staged.retain();
                Ok(outcome)
            }
        }
    }

    /// What every Finder create checks first, and the ids it gets. `None`: the name is
    /// one Finder writes for itself, which is ignored.
    fn start_finder_create(&self, target: &FinderWriteTarget) -> anyhow::Result<Option<FinderCreateStart>> {
        // Task 1538 Codex P1 (PR #49, lib.rs:1087 thread): refuse a brand-new
        // enqueue once this engine has been asked to stop. Both the Windows
        // upload watcher (`watcher::spawn`'s debounce/scan loops) and the
        // macOS/Linux File Provider extension (via `ipc_socket::handle_connection`'s
        // `QueueFinderCreate` dispatch) call this directly, so this single
        // check covers "the watcher" on every platform without needing a
        // separate flag check duplicated in each caller.
        if self.is_stopping() {
            anyhow::bail!("engine is stopping; refusing to enqueue a new local write");
        }
        if is_ignored_finder_name(&target.filename) {
            return Ok(None);
        }

        let parent_contract = self.ensure_shared_parent_allows_write(target.parent_id.as_deref())?;
        // Honour a caller-supplied id (task 0811 folder scaffolding writes the
        // local row under a known id and needs the server CreateFolder to reuse
        // it — the server accepts a client `folder_id`). The file path and the
        // legacy IPC always pass `None`, getting a fresh uuid as before.
        let file_id = target
            .file_id
            .clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        // Full server-relative key for this item: the FULL nested path when the
        // caller supplied one (`docs/a.txt`), else the leaf filename (top-level /
        // legacy IPC). This is stored as the row's `path` AND threaded as the
        // upload's `target_path`, so filter-3, finalize, and delete all key off
        // the same path. `clone` because `filename` is still needed for the
        // server `name` (always the leaf).
        let rel_path = target.rel_path.clone().unwrap_or_else(|| target.filename.clone());
        Ok(Some(FinderCreateStart {
            parent_contract,
            file_id,
            rel_path,
        }))
    }

    /// A Finder file create, staged and described: the daemon's own copy of the bytes,
    /// the row it inserts and the upload's metadata. Shared by both create entry points.
    fn prepare_finder_create(
        &self,
        target: &FinderWriteTarget,
        contents: Option<&std::fs::File>,
        start: FinderCreateStart,
    ) -> anyhow::Result<PreparedFinderCreate> {
        let FinderCreateStart {
            parent_contract,
            file_id,
            rel_path,
        } = start;
        let contents_path = target
            .contents_path
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("Finder create file callback did not include contents"))?;
        let staged = stage_finder_contents(
            &self.db,
            self.finder_staging_root()?,
            Path::new(contents_path),
            contents,
        )?;
        let staged_path = staged.path().to_string();
        let size_bytes = std::fs::metadata(&staged_path).map(|m| m.len() as i64).unwrap_or(0);
        let mime = target
            .content_type
            .as_deref()
            .or_else(|| beebeeb_core::media::guess_mime_type(&target.filename));
        let name_encrypted = encrypted_metadata_for_name(self.api.master_key(), &file_id, &target.filename, mime)?;
        let row = FileEntry {
            file_id: file_id.clone(),
            // FULL relative key (e.g. `docs/a.txt`), so a nested file's
            // row is found by `get_file_by_path(full_key)` immediately —
            // not keyed by the leaf, which classify_local_path's filter-3
            // would miss for a nested file → spurious re-upload.
            path: rel_path.clone(),
            status: FileStatus::Uploading,
            size_bytes,
            modified_at: now_secs(),
            content_hash: None,
            remote_updated_at: 0,
            // Not persisted by upsert_file; contract owns these.
            parent_id: None,
            item_kind: ItemKind::File,
        };
        let mut payload = serde_json::json!({
            "operation": "create_file",
            "name_encrypted": name_encrypted,
            "content_type": target.content_type,
            "uploaded_by": "authenticated_desktop_user",
        });
        apply_shared_context(&mut payload, parent_contract.as_ref());
        Ok(PreparedFinderCreate {
            file_id,
            rel_path,
            staged,
            staged_path,
            size_bytes,
            row,
            payload,
        })
    }

    /// The File Provider create (spec §8.8: minting is keyed on the IPC entries, never on
    /// the shared `queue_finder_*`, which the watcher and the upload driver call too).
    /// One accept transaction inserts the row, mints the write token and queues the
    /// upload (spec §8.7 S1.1). A folder carries no content and gets no token.
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    pub fn queue_file_provider_create_from(
        &self,
        target: FinderWriteTarget,
        contents: Option<&std::fs::File>,
    ) -> anyhow::Result<FpWrite> {
        if target.kind == FinderWriteItemKind::Folder {
            return self.queue_finder_create_from(target, contents).map(FpWrite::plain);
        }
        let Some(start) = self.start_finder_create(&target)? else {
            return Ok(FpWrite::plain(ignored_finder_item(&target.filename)));
        };
        let prepared = self.prepare_finder_create(&target, contents, start)?;
        let op_id = uuid::Uuid::new_v4().to_string();
        let metadata_json = serde_json::to_string(&prepared.payload)?;
        let backup_source_key = crate::known_folder::backup_source_key_for_this_device(&prepared.rel_path);
        self.seam("accept:before_tx");
        let accepted = self.db.accept_finder_write(
            &crate::state_db::FinderAccept {
                op_id: &op_id,
                file_id: &prepared.file_id,
                kind: crate::state_db::FinderAcceptKind::Create { row: &prepared.row },
                parent_id: target.parent_id.as_deref(),
                // The FULL relative key, as the Finder create queues it.
                target_path: Some(&prepared.rel_path),
                metadata_json: &metadata_json,
                payload_path: &prepared.staged_path,
                size_bytes: prepared.size_bytes,
                modified_at: prepared.row.modified_at,
                backup_source_key: backup_source_key.as_deref(),
                now: now_secs(),
            },
            &|entry, contract| legacy_item_identifiers(entry, contract).to_vec(),
        )?;
        let token = match accepted {
            crate::state_db::AcceptOutcome::Queued { token, .. } => token,
            crate::state_db::AcceptOutcome::ParkedAtOnce { token, reason } => {
                log_parked(&op_id, Some(&prepared.file_id), reason);
                token
            }
            crate::state_db::AcceptOutcome::UnknownItem => anyhow::bail!("a create always inserts its row"),
        };
        prepared.staged.retain();
        Ok(FpWrite {
            outcome: FinderWriteOutcome::Queued {
                op_id,
                file_id: Some(prepared.file_id),
                kind: OperationKind::UploadVersion,
                ignored: false,
                message: "queued for encrypted sync".to_string(),
            },
            token: Some(token),
            present_as: None,
        })
    }

    /// A create with `.deletionConflicted` (spec §7.2, I-1): the system could not apply our
    /// delete of the template's item because the person edited it. Resolved through the alias
    /// (P→S), or a live row with the template's id, it is a content modify of that row,
    /// replied under the provider's identifier, S (`REPL.h:438-443`), so the system's reuse
    /// rule replaces item S on disk with the edited file. Otherwise an ordinary create.
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    pub fn queue_file_provider_deletion_conflicted_create(
        &self,
        target: FinderWriteTarget,
        template_identifier: Option<String>,
        template_content_version: Option<String>,
        contents: Option<&std::fs::File>,
    ) -> anyhow::Result<FpWrite> {
        let resolved = match template_identifier.as_deref() {
            Some(id) => match self.db.get_file(id)? {
                Some(row) => Some(row),
                None => match self.db.resolve_alias(id)? {
                    Some(server_id) => {
                        log_alias_resolved("deletion_conflicted_create", id, &server_id);
                        self.db.get_file(&server_id)?
                    }
                    None => None,
                },
            },
            None => None,
        };
        match resolved {
            // A content modify of S. Only a file has content to modify.
            Some(row)
                if row.status != FileStatus::Trashing
                    && row.item_kind == ItemKind::File
                    && target.kind == FinderWriteItemKind::File =>
            {
                self.queue_file_provider_modify_from(
                    FinderWriteTarget {
                        file_id: Some(row.file_id),
                        parent_id: target.parent_id,
                        filename: target.filename,
                        rel_path: None,
                        kind: FinderWriteItemKind::File,
                        contents_path: target.contents_path,
                        content_type: target.content_type,
                        base_version_identifier: template_content_version,
                    },
                    contents,
                )
            }
            // No alias and no row (a file deleted elsewhere: the edit becomes a new file), or a
            // row going to the trash (the edit must not follow it there): an ordinary create.
            _ => self.queue_file_provider_create_from(target, contents),
        }
    }

    /// §7.2: the id a request under `id` acts on, and the id its reply is presented under. A
    /// live row with `id` is acted on as is; otherwise an alias sends a provisional id to the
    /// server id its create landed as, presented under `id`. An id with neither is returned
    /// as is, for the caller to refuse or to let fail.
    #[cfg_attr(not(any(unix, test)), allow(dead_code))]
    pub fn resolve_provisional(&self, id: &str, request: &'static str) -> anyhow::Result<(String, Option<String>)> {
        if self.db.get_file(id)?.is_some() {
            return Ok((id.to_string(), None));
        }
        match self.db.resolve_alias(id)? {
            Some(server_id) => {
                log_alias_resolved(request, id, &server_id);
                Ok((server_id, Some(id.to_string())))
            }
            None => Ok((id.to_string(), None)),
        }
    }

    /// §7.4: a fetch of an item whose held write is queued is served from that write's
    /// staged bytes, the bytes its token names, and never reaches the server; otherwise
    /// from the server, as before.
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    pub async fn serve_hydrate(
        &self,
        id: &str,
        dest: &Path,
        allowed_roots: &[&Path],
        progress: Option<&HydrateProgressFn>,
    ) -> anyhow::Result<HydrateSource> {
        if !hydrate_dest_is_allowed(dest, allowed_roots) {
            anyhow::bail!("hydrate destination is not within an allowed root");
        }
        let (file_id, _) = self.resolve_provisional(id, "hydrate")?;
        // Decide once more when the landing unlinked the copy between the read and the open:
        // the write is then no longer queued, and the server has its bytes (§7.4, 4b).
        for _ in 0..2 {
            let Some(path) = self.db.queue_fetch_source(&file_id)? else {
                break;
            };
            match std::fs::File::open(&path) {
                Ok(mut staged) => {
                    if let Some(parent) = dest.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    let bytes = write_hydrated_from_reader(dest, allowed_roots, &mut staged)?;
                    if let Some(progress) = progress {
                        progress(bytes, bytes);
                    }
                    log_hydrate_served(&file_id, HydrateSource::Queue);
                    return Ok(HydrateSource::Queue);
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e.into()),
            }
        }
        self.hydrate_file_with_progress(&file_id, dest, allowed_roots, progress)
            .await?;
        log_hydrate_served(&file_id, HydrateSource::Server);
        Ok(HydrateSource::Server)
    }

    /// A Finder thumbnail (spec §5.6, §7.2): through the alias, and never while the item's
    /// held write is queued (the safe default: a per-item error, and nothing asked of the
    /// server).
    #[cfg_attr(not(any(unix, test)), allow(dead_code))]
    pub async fn finder_thumbnail(&self, id: &str, variant: &str) -> anyhow::Result<Zeroizing<Vec<u8>>> {
        let (file_id, _) = self.resolve_provisional(id, "thumbnail")?;
        if self
            .db
            .item_presentation(&file_id)?
            .is_some_and(|presentation| presentation.held_write_queued)
        {
            return Err(anyhow::Error::new(ThumbnailNotYet));
        }
        self.fetch_thumbnail_to_memory(&file_id, variant).await
    }

    /// [`Self::queue_file_provider_create_from`] reading the contents by path.
    #[cfg(test)]
    pub fn queue_file_provider_create(&self, target: FinderWriteTarget) -> anyhow::Result<FpWrite> {
        self.queue_file_provider_create_from(target, None)
    }

    pub fn queue_finder_modify(&self, target: FinderWriteTarget) -> anyhow::Result<FinderWriteOutcome> {
        self.queue_finder_modify_from(target, None)
    }

    /// [`Self::queue_finder_modify`], reading new contents from `contents`
    /// when given; see [`Self::queue_finder_create_from`].
    pub fn queue_finder_modify_from(
        &self,
        target: FinderWriteTarget,
        contents: Option<&std::fs::File>,
    ) -> anyhow::Result<FinderWriteOutcome> {
        let Some((file_id, item_contract)) = self.start_finder_modify(&target)? else {
            return Ok(ignored_finder_item(&target.filename));
        };

        if let Some(contents_path) = target.contents_path.as_deref() {
            let PreparedFinderModify {
                staged,
                staged_path,
                size_bytes,
                modified_at,
                payload,
            } = self.prepare_finder_modify(&target, contents_path, contents, &file_id, item_contract.as_ref())?;
            // Decided from the row BEFORE this write changes it.
            let current_row = self.db.get_file(&file_id)?;
            let current_contract = self.db.get_file_contract_state(&file_id)?;
            let base_version = modify_base_version(
                target.base_version_identifier.as_deref(),
                current_row.as_ref().zip(current_contract.as_ref()),
            );
            // The system keeps the bytes it handed over and is not asked to
            // fetch them back, so the item in the reply must describe them:
            // the row takes the staged copy's size and modification time. The
            // content version is untouched until the upload lands.
            self.db.record_local_write(&file_id, size_bytes, modified_at)?;
            let outcome = self.enqueue_finder_operation(
                OperationKind::UploadVersion,
                Some(file_id),
                target.parent_id,
                Some(target.filename),
                payload,
                Some(staged_path),
                base_version,
                item_contract
                    .as_ref()
                    .and_then(|contract| contract.current_object_version_id.clone()),
            )?;
            staged.retain();
            Ok(outcome)
        } else {
            let name_encrypted = encrypted_metadata_for_name(
                self.api.master_key(),
                &file_id,
                &target.filename,
                target.content_type.as_deref(),
            )?;
            let kind = if target.parent_id.is_some() {
                OperationKind::MoveFile
            } else {
                OperationKind::RenameFile
            };
            let mut payload = serde_json::json!({
                "operation": "metadata_update",
                "name_encrypted": name_encrypted,
                "base_version_identifier": target.base_version_identifier,
            });
            apply_shared_context(&mut payload, item_contract.as_ref());
            self.enqueue_finder_operation(
                kind,
                Some(file_id),
                target.parent_id,
                Some(target.filename),
                payload,
                None,
                None,
                item_contract
                    .as_ref()
                    .and_then(|contract| contract.current_object_version_id.clone()),
            )
        }
    }

    /// What every Finder modify checks first: the file id, and the item's contract when it
    /// is shared. `None`: the name is one Finder writes for itself, which is ignored.
    fn start_finder_modify(
        &self,
        target: &FinderWriteTarget,
    ) -> anyhow::Result<Option<(String, Option<FileContractState>)>> {
        // Task 1538 Codex P1 — see `queue_finder_create`'s identical guard.
        if self.is_stopping() {
            anyhow::bail!("engine is stopping; refusing to enqueue a new local write");
        }
        if is_ignored_finder_name(&target.filename) {
            return Ok(None);
        }

        let file_id = target
            .file_id
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Finder modify callback did not include a file id"))?;
        let item_contract = self.ensure_item_allows_shared_write(&file_id, "modify")?;
        Ok(Some((file_id, item_contract)))
    }

    /// A Finder content modify, staged and described: the daemon's own copy of the new
    /// bytes, its size and time, and the upload's metadata. It reads and writes no row:
    /// each entry point decides the base itself. Shared by both modify entry points.
    fn prepare_finder_modify(
        &self,
        target: &FinderWriteTarget,
        contents_path: &str,
        contents: Option<&std::fs::File>,
        file_id: &str,
        item_contract: Option<&FileContractState>,
    ) -> anyhow::Result<PreparedFinderModify> {
        let staged = stage_finder_contents(
            &self.db,
            self.finder_staging_root()?,
            Path::new(contents_path),
            contents,
        )?;
        let staged_path = staged.path().to_string();
        let staged_metadata = std::fs::metadata(&staged_path).ok();
        let size_bytes = staged_metadata.as_ref().map(|m| m.len() as i64).unwrap_or(0);
        let modified_at = staged_metadata
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or_else(now_secs);
        let mime = target
            .content_type
            .as_deref()
            .or_else(|| beebeeb_core::media::guess_mime_type(&target.filename));
        let name_encrypted = encrypted_metadata_for_name(self.api.master_key(), file_id, &target.filename, mime)?;
        let mut payload = serde_json::json!({
            "operation": "upload_version",
            "name_encrypted": name_encrypted,
            "content_type": target.content_type,
            "size_bytes": size_bytes,
            "base_version_identifier": target.base_version_identifier,
            "uploaded_by": "authenticated_desktop_user",
        });
        apply_shared_context(&mut payload, item_contract);
        Ok(PreparedFinderModify {
            staged,
            staged_path,
            size_bytes,
            modified_at,
            payload,
        })
    }

    /// The File Provider modify (spec §8.8; see [`Self::queue_file_provider_create_from`]).
    /// New contents go through one accept transaction that maps the system's base
    /// through the write token (spec §6.1) and queues the upload. A rename or move
    /// carries no content and gets no token.
    ///
    /// The id is resolved first (spec §7.2): a provisional id whose create has landed acts
    /// on the server file, and the reply is presented under the provisional id. An id with
    /// no row and no alias is refused with [`UnknownItem`] before anything is staged, for a
    /// content and a metadata modify alike (§7.3). A name Finder writes for itself stays
    /// ignored, as before, whatever its id.
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    pub fn queue_file_provider_modify_from(
        &self,
        mut target: FinderWriteTarget,
        contents: Option<&std::fs::File>,
    ) -> anyhow::Result<FpWrite> {
        let mut present_as = None;
        if let Some(requested_id) = target.file_id.clone()
            && !is_ignored_finder_name(&target.filename)
        {
            let (file_id, presented) = self.resolve_provisional(&requested_id, "modify")?;
            if self.db.get_file(&file_id)?.is_none() {
                return Err(anyhow::Error::new(UnknownItem));
            }
            target.file_id = Some(file_id);
            present_as = presented;
        }
        let Some(contents_path) = target.contents_path.clone() else {
            return self.queue_finder_modify_from(target, contents).map(|outcome| FpWrite {
                outcome,
                token: None,
                present_as,
            });
        };
        let Some((file_id, item_contract)) = self.start_finder_modify(&target)? else {
            return Ok(FpWrite::plain(ignored_finder_item(&target.filename)));
        };
        let prepared =
            self.prepare_finder_modify(&target, &contents_path, contents, &file_id, item_contract.as_ref())?;
        let op_id = uuid::Uuid::new_v4().to_string();
        let metadata_json = serde_json::to_string(&prepared.payload)?;
        let backup_source_key = crate::known_folder::backup_source_key_for_this_device(&target.filename);
        self.seam("accept:before_tx");
        let accepted = self.db.accept_finder_write(
            &crate::state_db::FinderAccept {
                op_id: &op_id,
                file_id: &file_id,
                kind: crate::state_db::FinderAcceptKind::Modify {
                    incoming_base: target.base_version_identifier.as_deref(),
                },
                parent_id: target.parent_id.as_deref(),
                target_path: Some(&target.filename),
                metadata_json: &metadata_json,
                payload_path: &prepared.staged_path,
                size_bytes: prepared.size_bytes,
                modified_at: prepared.modified_at,
                backup_source_key: backup_source_key.as_deref(),
                now: now_secs(),
            },
            &|entry, contract| legacy_item_identifiers(entry, contract).to_vec(),
        )?;
        let token = match accepted {
            crate::state_db::AcceptOutcome::Queued { token, .. } => token,
            crate::state_db::AcceptOutcome::ParkedAtOnce { token, reason } => {
                log_parked(&op_id, Some(&file_id), reason);
                token
            }
            // The row went between the check above and the accept: refused the same way
            // (§7.3). The staged copy is dropped with `prepared`; nothing was queued.
            crate::state_db::AcceptOutcome::UnknownItem => return Err(anyhow::Error::new(UnknownItem)),
        };
        prepared.staged.retain();
        Ok(FpWrite {
            outcome: FinderWriteOutcome::Queued {
                op_id,
                file_id: Some(file_id),
                kind: OperationKind::UploadVersion,
                ignored: false,
                message: "queued for encrypted sync".to_string(),
            },
            token: Some(token),
            present_as,
        })
    }

    /// [`Self::queue_file_provider_modify_from`] reading the contents by path.
    #[cfg(test)]
    pub fn queue_file_provider_modify(&self, target: FinderWriteTarget) -> anyhow::Result<FpWrite> {
        self.queue_file_provider_modify_from(target, None)
    }

    pub fn queue_finder_delete(
        &self,
        file_id: &str,
        base_version_identifier: Option<String>,
    ) -> anyhow::Result<FinderWriteOutcome> {
        // Task 1538 Codex P1 — see `queue_finder_create`'s identical guard.
        if self.is_stopping() {
            anyhow::bail!("engine is stopping; refusing to enqueue a new local write");
        }
        // §7.2 (m-6): a provisional id whose create landed reaches the server file only when
        // the delete's base is that file's held token, the token the create's reply named.
        // Any other base keeps the unknown-item answer below: the system's own resolution of
        // the swap is undocumented, and a delete that does not name our bytes must never
        // trash the person's file.
        if let Some(server_id) = self.db.resolve_alias(file_id)? {
            let held = self
                .db
                .item_presentation(&server_id)?
                .and_then(|presentation| presentation.held)
                .map(|held| held.token());
            if held.is_some() && held == base_version_identifier {
                log_alias_resolved("delete", file_id, &server_id);
                return self.queue_finder_delete(&server_id, base_version_identifier);
            }
            log_alias_delete_kept(file_id, &server_id);
            return Ok(FinderWriteOutcome::Ignored {
                message: "the item is already gone from Beebeeb".to_string(),
            });
        }
        // Task 1698 (trash ruling): an UNKNOWN item is an idempotent success
        // ("unknown items report success") — the item may have been deleted
        // remotely already, or the replica converged past it. Queueing a
        // doomed server call would retry a guaranteed 404 for hours.
        let item_contract = match self.ensure_item_allows_shared_write(file_id, "delete")? {
            contract if contract.is_none() && self.db.get_file(file_id)?.is_none() => {
                tracing::info!(file_id = %file_id, "finder delete: unknown item — idempotent success");
                return Ok(FinderWriteOutcome::Ignored {
                    message: "the item is already gone from Beebeeb".to_string(),
                });
            }
            contract => contract,
        };
        let mut payload = serde_json::json!({
            "operation": "trash",
            "base_version_identifier": base_version_identifier,
        });
        apply_shared_context(&mut payload, item_contract.as_ref());
        let outcome = self.enqueue_finder_operation(
            OperationKind::TrashFile,
            Some(file_id.to_string()),
            None,
            None,
            payload,
            None,
            parse_base_version_number(base_version_identifier.as_deref()),
            item_contract
                .as_ref()
                .and_then(|contract| contract.current_object_version_id.clone()),
        )?;
        // Task 1698: park the row `Trashing` AFTER a successful enqueue, so
        // the change feed immediately presents the item under the trash
        // container (ruling step 2) while the server trash converges. A
        // failed enqueue leaves the row untouched (nothing hidden without a
        // queued op). Idempotent for the watcher path, which parks first.
        if let Some(entry) = self.db.get_file(file_id)?
            && entry.status != crate::state_db::FileStatus::Trashing
        {
            self.db.set_status(file_id, crate::state_db::FileStatus::Trashing)?;
        }
        Ok(outcome)
    }

    /// Single, parent-aware classifier shared by every local-write trigger
    /// (the retired `notify` watcher *and* the Windows Cloud Files NOTIFY
    /// callbacks). Given an absolute on-disk `path` under `sync_root`, it
    /// decides whether `path` is a *genuinely-new* user file that should be
    /// uploaded, and if so returns a ready-to-queue [`FinderWriteTarget`] with
    /// `parent_id` resolved from the path's parent directory.
    ///
    /// It runs the three feedback-loop filters, in order, so a download / a
    /// hydration / an engine-internal write is NEVER mistaken for a new upload:
    ///
    /// 1. **Engine-internal paths + OS junk** — anything under `<root>/.beebeeb/`,
    ///    the `<root>/.beebeeb-sync.lock`, any `.beebeeb` path component, and the
    ///    ignored-name set ([`is_ignored_finder_name`]: `.tmp`, `~$…`, `.DS_Store`,
    ///    …). These are writes the engine itself (or the OS) makes constantly.
    /// 2. **Cloud Files placeholders** (Windows only) — a reparse point under the
    ///    sync root is something WE minted (placeholder seed or post-upload
    ///    convert). Hydration fills a placeholder's data stream WITHOUT clearing
    ///    its reparse-point attribute, so a hydration-write still trips this guard
    ///    and never re-uploads. Checked via
    ///    [`crate::windows_cf::placeholders::is_cloud_placeholder`].
    /// 3. **Already a known server file** (authoritative) — look the path up in
    ///    the state DB. A row means the file already lives on the server
    ///    (cloud-only / downloading / local / uploading), so a write to it is a
    ///    hydration or a re-download, NOT a new creation. Only a path with NO DB
    ///    row survives.
    ///
    /// `parent_id` resolution (nested uploads): for a survivor at
    /// `<parent_dir>/<name>`, the parent directory's server folder id is looked
    /// up by its server-relative path. A root-level file resolves to `None`; a
    /// nested file resolves to `Some(<parent folder file_id>)` so the upload
    /// lands in the right server folder. If the parent directory has no DB row
    /// yet (its folder placeholder hasn't reconciled), `parent_id` falls back to
    /// `None` rather than guessing — the file uploads to the root and a later
    /// tick can reconcile, which is strictly safer than attaching it to the
    /// wrong parent.
    ///
    /// Returns `None` when `path` is not a regular file, is filtered out, or the
    /// DB lookup fails (fail-closed: a lookup error skips the upload rather than
    /// risk a spurious one). Cross-platform so it type-checks everywhere; only
    /// the Windows triggers call it.
    pub fn classify_local_path(&self, sync_root: &Path, path: &Path) -> Option<FinderWriteTarget> {
        // The file may have been deleted/renamed during a debounce window — if
        // it is no longer a regular file there is nothing to upload.
        match std::fs::symlink_metadata(path) {
            Ok(m) if m.is_file() => {}
            Ok(m) => {
                let file_type = m.file_type();
                tracing::debug!(
                    path = %path.display(),
                    is_dir = file_type.is_dir(),
                    is_symlink = file_type.is_symlink(),
                    "classify_local_path: rejected non-regular file"
                );
                return None;
            }
            Err(e) => {
                tracing::debug!(
                    path = %path.display(),
                    error = %e,
                    "classify_local_path: stat failed; skipping"
                );
                return None;
            }
        }

        // Filter 1 — engine-internal paths + OS junk.
        if path_is_engine_internal(sync_root, path) {
            tracing::debug!(
                path = %path.display(),
                "classify_local_path: rejected engine-internal path"
            );
            return None;
        }
        let Some(file_name) = path.file_name().and_then(|n| n.to_str()).map(str::to_string) else {
            tracing::debug!(
                path = %path.display(),
                "classify_local_path: rejected path with non-utf8 filename"
            );
            return None;
        };
        if is_ignored_finder_name(&file_name) {
            tracing::debug!(
                path = %path.display(),
                filename = %file_name,
                "classify_local_path: rejected ignored/temp filename"
            );
            return None;
        }

        // Filter 2 — Cloud Files placeholders are engine-owned (Windows only). A
        // reparse point under the sync root is a placeholder we minted or
        // converted; the hydration write that fills it does NOT turn it back into
        // a plain file, so we must never treat a placeholder write as a new
        // upload.
        #[cfg(target_os = "windows")]
        if crate::windows_cf::placeholders::is_cloud_placeholder(path) {
            tracing::debug!(
                path = %path.display(),
                "classify_local_path: rejected Cloud Files placeholder"
            );
            return None;
        }

        // Filter 3 (authoritative) — already a known server file?
        let Some(rel) = relative_db_path(sync_root, path) else {
            tracing::debug!(
                path = %path.display(),
                sync_root = %sync_root.display(),
                "classify_local_path: rejected path outside sync root"
            );
            return None;
        };
        match self.db.get_file_by_path(&rel) {
            Ok(Some(_existing)) => {
                tracing::debug!(
                    path = %path.display(),
                    rel_path = %rel,
                    "classify_local_path: rejected already-tracked server file"
                );
                return None;
            }
            Ok(None) => { /* genuinely new — fall through */ }
            Err(e) => {
                tracing::warn!(
                    path = %path.display(),
                    rel_path = %rel,
                    error = %e,
                    "classify_local_path: state DB lookup failed; skipping to be safe"
                );
                return None;
            }
        }

        // parent_id resolution: a survivor at `<parent_dir>/<name>`. If the parent
        // directory maps to a known server FOLDER row, attach the new file to it;
        // otherwise upload at the root (None). We never attach to a row that isn't
        // a folder.
        let parent_id = self.resolve_parent_id_for(sync_root, path);
        tracing::debug!(
            path = %path.display(),
            rel_path = %rel,
            nested = parent_id.is_some(),
            "classify_local_path: accepted new local file"
        );

        Some(FinderWriteTarget {
            file_id: None,
            parent_id,
            filename: file_name.clone(),
            // The FULL '/'-joined relative key (e.g. `docs/a.txt`), so the row is
            // stored + the upload's target_path is threaded under the same key
            // that filter-3, finalize, and delete all query. `rel` was computed
            // above for the filter-3 lookup; reuse it verbatim.
            rel_path: Some(rel),
            kind: FinderWriteItemKind::File,
            contents_path: Some(path.to_string_lossy().into_owned()),
            content_type: beebeeb_core::media::guess_mime_type(&file_name).map(str::to_string),
            base_version_identifier: None,
        })
    }

    /// Resolve the server folder id of `path`'s immediate parent directory, for
    /// nested uploads. Returns `None` for a root-level file (parent == sync root)
    /// or when the parent directory has no known server FOLDER row yet. The
    /// parent's server-relative key is built in the same '/'-joined shape the
    /// state DB stores, then looked up via [`StateDb::get_file_by_path`]; a hit
    /// that is a folder yields its `file_id`.
    ///
    /// `pub(crate)` so the upload driver's rename handler can resolve the NEW
    /// parent of a moved file through the exact same logic.
    pub(crate) fn resolve_parent_id_for(&self, sync_root: &Path, path: &Path) -> Option<String> {
        let parent_dir = path.parent()?;
        // Root-level file: parent IS the sync root → no server parent.
        if parent_dir == sync_root {
            return None;
        }
        let parent_rel = relative_db_path(sync_root, parent_dir)?;
        match self.db.get_file_by_path(&parent_rel) {
            Ok(Some(entry)) if entry.is_dir() => Some(entry.file_id),
            // No row, or a row that is somehow a file (shouldn't happen for a
            // directory path) → upload at the root rather than mis-parent.
            _ => None,
        }
    }

    /// Ensure the on-disk DIRECTORY at `path` (under `sync_root`) has a server
    /// vault folder, returning its `file_id`. This is the folder-hierarchy half
    /// of the local-create pipeline (task 0811): the enumeration scan only
    /// uploaded *files*, so a freshly-mirrored nested tree
    /// (`Backup/<device>/<folder>/<sub>/file`) had no folder rows — every nested
    /// file's `resolve_parent_id_for` missed and the file uploaded FLAT to the
    /// vault root. Scaffolding the folder first (parents before children, which
    /// the top-down walk already guarantees) gives each child a real parent to
    /// attach to, so the server vault mirrors the on-disk hierarchy.
    ///
    /// Idempotent + parent-aware:
    /// - If a DB row already exists for this directory's relative key, returns its
    ///   `file_id` (a folder row, or `None` if it is somehow a file row).
    /// - Otherwise mints a client folder id, writes a local `Folder` row + contract
    ///   IMMEDIATELY (so a child file dispatched later in the SAME scan resolves
    ///   this parent without waiting for a server round-trip), and enqueues a
    ///   `CreateFolder` op carrying that same id — the server honours the
    ///   client-supplied `folder_id`, so the local row and the server folder share
    ///   one id and the parent linkage is consistent.
    ///
    /// The op's `target_path` is the folder's relative key, so a backup folder is
    /// auto-tagged with its origin key (Part C) and purged on disable. Returns
    /// `None` for the sync root itself, a non-directory, or on a DB error
    /// (fail-closed: the file then uploads at the root rather than mis-parenting).
    pub fn ensure_local_folder(&self, sync_root: &Path, path: &Path) -> Option<String> {
        // Root itself has no server folder.
        if path == sync_root {
            return None;
        }
        match std::fs::symlink_metadata(path) {
            Ok(m) if m.is_dir() => {}
            _ => return None,
        }
        if path_is_engine_internal(sync_root, path) {
            return None;
        }
        let name = path.file_name().and_then(|n| n.to_str())?.to_string();
        if is_ignored_finder_name(&name) {
            return None;
        }
        let rel = relative_db_path(sync_root, path)?;

        // Already known? Return the existing folder id (idempotent across scans).
        match self.db.get_file_by_path(&rel) {
            Ok(Some(entry)) if entry.is_dir() => return Some(entry.file_id),
            Ok(Some(_)) => return None, // a file row at a dir path — never mis-parent
            Ok(None) => { /* mint below */ }
            Err(e) => {
                tracing::warn!(error = %e, "ensure_local_folder: DB lookup failed; skipping");
                return None;
            }
        }

        // Parent linkage: resolve THIS directory's parent dir to its folder id
        // (None at the Backup root level — a top-level folder under the vault).
        let parent_id = self.resolve_parent_id_for(sync_root, path);
        let folder_id = uuid::Uuid::new_v4().to_string();

        // Delegate the row write + contract + CreateFolder enqueue to the one
        // shared create path, carrying our pre-minted id (which the server
        // honours) so the local row and the server folder share an id. The
        // `rel`-path `target_path` auto-tags a backup folder with its origin key.
        let target = FinderWriteTarget {
            file_id: Some(folder_id.clone()),
            parent_id,
            filename: name,
            rel_path: Some(rel),
            kind: FinderWriteItemKind::Folder,
            contents_path: None,
            content_type: None,
            base_version_identifier: None,
        };
        match self.queue_finder_create(target) {
            Ok(FinderWriteOutcome::Queued { op_id, .. }) => {
                tracing::info!(op_id = %op_id, folder_id = %folder_id, "upload driver: scaffolded vault folder for nested upload");
                Some(folder_id)
            }
            Ok(FinderWriteOutcome::Ignored { .. }) => None,
            Err(e) => {
                tracing::warn!(error = %e, "ensure_local_folder: failed to scaffold vault folder");
                None
            }
        }
    }

    /// Apply a pin ("available offline") toggle to `file_id` and its subtree.
    ///
    /// `sync_root` is the on-disk root of the vault — needed on Windows to
    /// resolve each affected DB row to its placeholder path and set the OS-level
    /// pin state. It is unused on macOS/Linux (the `#[cfg]` below silences the
    /// warning) but threaded in unconditionally to keep one signature.
    ///
    /// Three things happen on a pin change:
    /// 1. **DB** — `db.set_recursive_pin` flips the `pin_state` column for the
    ///    whole subtree and returns the ids whose state actually changed.
    /// 2. **Server** — `enqueue_pin_tree_operation` propagates the pin so OTHER
    ///    devices learn about it. Always runs (every platform).
    /// 3. **OS hydration** —
    ///    - **Windows**: `CfSetPinState` is the OS contract. A pinned placeholder
    ///      is hydrated by Windows via the normal FETCH_DATA path AND is exempt
    ///      from auto-dehydration, so it stays resident. We therefore do NOT
    ///      enqueue our own out-of-band hydrate ops (the old DB-only model did,
    ///      because Windows was never told about the pin — that loop is dropped on
    ///      Windows). New children inherit the parent pin via CF_PIN_STATE_INHERIT.
    ///    - **macOS/Linux**: there is no CF pin primitive, so we keep enqueuing an
    ///      explicit hydrate op per newly-pinned cloud-only file to materialise it.
    pub fn set_recursive_pin(
        &self,
        #[cfg_attr(not(target_os = "windows"), allow(unused_variables))] sync_root: &Path,
        file_id: &str,
        pinned: bool,
    ) -> anyhow::Result<PinUpdateOutcome> {
        let now = now_secs();
        let changed_item_ids = self.db.set_recursive_pin(file_id, pinned, now)?;
        // Mutated only on the non-Windows hydrate-enqueue path below; on Windows
        // the OS (CfSetPinState) drives hydration, so this stays 0.
        #[cfg_attr(target_os = "windows", allow(unused_mut))]
        let mut hydrate_operations = 0usize;

        self.enqueue_pin_tree_operation(file_id, pinned, now)?;

        #[cfg(target_os = "windows")]
        {
            // OS-level pin: tell Windows the actual "available offline" state so a
            // pinned file is kept resident (no auto-dehydration) and an unpinned
            // one becomes reclaim-eligible again. We pin/unpin only the TOP item
            // the user toggled, with RECURSE when it is a directory — the
            // recursive flag stamps existing descendants in one call and
            // CF_PIN_STATE_INHERIT covers descendants created later — rather than
            // issuing a per-row CfSetPinState for every changed id.
            if let Some(top) = self.db.get_file(file_id)? {
                if let Some(path) = Self::placeholder_path_under(sync_root, &top.path) {
                    let recurse = top.is_dir();
                    if let Err(e) = crate::windows_cf::placeholders::set_pin_state(&path, pinned, recurse) {
                        // Zero-knowledge: never log the path. A pin-state failure
                        // must not abort the DB+server pin that already succeeded;
                        // surface it as a log line and continue.
                        tracing::warn!(file_id = %file_id, pinned, recurse, error = %e, "CfSetPinState failed for pin toggle");
                    }
                }

                // Proactive hydrate. CfSetPinState(PINNED) marks the subtree
                // pinned but Windows does NOT eagerly download a not-yet-opened
                // placeholder — a pinned-but-unopened file stays cloud-only
                // (RECALL_ON_DATA_ACCESS) until something reads it. So "available
                // offline" is only real once we force the download. Collect the
                // cloud-only file descendants (the toggled item itself if it is a
                // cloud-only file, or every cloud-only file under it if a folder)
                // and CfHydratePlaceholder each. We only ever hydrate on pin, not
                // unpin (unpinning just makes files reclaim-eligible again).
                if pinned {
                    match self.db.cloud_only_file_descendants(file_id) {
                        Ok(entries) if !entries.is_empty() => {
                            // Resolve placeholder paths the SAME way the pin does,
                            // dropping any that fail the containment guard.
                            let paths: Vec<PathBuf> = entries
                                .iter()
                                .filter_map(|e| Self::placeholder_path_under(sync_root, &e.path))
                                .collect();
                            // CfHydratePlaceholder BLOCKS until each file's bytes
                            // are on disk, and a folder may hold many/large files,
                            // so run the whole sweep OFF the IPC thread — set_recursive_pin
                            // returns immediately while hydration proceeds in the
                            // background. A per-file failure is logged and never
                            // aborts the rest of the sweep.
                            let total = paths.len();
                            std::thread::spawn(move || {
                                tracing::debug!(count = total, "proactive pin hydrate: starting");
                                let mut ok = 0usize;
                                for path in &paths {
                                    match crate::windows_cf::placeholders::hydrate_placeholder(path) {
                                        Ok(()) => ok += 1,
                                        // Zero-knowledge: never log the path.
                                        Err(e) => tracing::warn!(error = %e, "proactive pin hydrate: file failed"),
                                    }
                                }
                                tracing::debug!(hydrated = ok, total, "proactive pin hydrate: finished");
                            });
                        }
                        Ok(_) => {} // nothing cloud-only to hydrate
                        Err(e) => {
                            // A query failure must not abort the DB+server pin that
                            // already succeeded; log and continue.
                            tracing::warn!(file_id = %file_id, error = %e, "proactive pin hydrate: descendant query failed");
                        }
                    }
                }
            }
        }

        #[cfg(not(target_os = "windows"))]
        if pinned {
            for changed_id in &changed_item_ids {
                let Some(entry) = self.db.get_file(changed_id)? else {
                    continue;
                };
                let Some(contract) = self.db.get_file_contract_state(changed_id)? else {
                    continue;
                };
                if contract.effective_pin_state() == crate::state_db::PinState::Pinned
                    && entry.status == FileStatus::CloudOnly
                    && entry.size_bytes > 0
                {
                    self.enqueue_hydrate_operation(&entry, now)?;
                    hydrate_operations += 1;
                }
            }
        }

        Ok(PinUpdateOutcome {
            changed_item_ids,
            hydrate_operations,
        })
    }

    /// Map a server-relative, '/'-joined DB key (`FileEntry::path`) to its
    /// on-disk placeholder path under `sync_root`, rejecting empty/traversal
    /// keys. Mirrors `windows_cf::callbacks::safe_join_under_root` (kept private
    /// there) so the pin path resolution uses the same containment guard as the
    /// fetch fallback. Returns `None` for an empty key or any `.`/`..`/empty
    /// segment, or if the join escapes the root.
    #[cfg(target_os = "windows")]
    fn placeholder_path_under(sync_root: &Path, rel_path: &str) -> Option<PathBuf> {
        let rel = rel_path.trim_matches('/');
        if rel.is_empty() {
            return None;
        }
        if rel.split('/').any(|seg| seg.is_empty() || seg == "." || seg == "..") {
            return None;
        }
        let native_rel = rel.replace('/', std::path::MAIN_SEPARATOR_STR);
        let candidate = sync_root.join(&native_rel);
        if !candidate.starts_with(sync_root) {
            return None;
        }
        Some(candidate)
    }

    pub fn record_smart_cache_open(
        &self,
        file_id: &str,
        cache_path: &Path,
        cache_bytes: i64,
    ) -> anyhow::Result<CacheCleanupOutcome> {
        self.db
            .mark_cached(file_id, &cache_path.to_string_lossy(), cache_bytes, now_secs())?;
        self.enforce_configured_cache_limit()
    }

    pub fn enforce_smart_cache(&self, policy: CachePolicy) -> anyhow::Result<CacheCleanupOutcome> {
        let evicted = self
            .db
            .evict_unpinned_cache_until_under(policy.max_unpinned_cache_bytes, now_secs())?;
        #[cfg(target_os = "linux")]
        self.remove_linux_freedesktop_thumbnails_for_file_ids(&evicted);
        Ok(CacheCleanupOutcome {
            evicted_file_ids: evicted,
        })
    }

    pub fn enforce_local_cache_limit(&self, max_local_cache_bytes: Option<i64>) -> anyhow::Result<CacheCleanupOutcome> {
        let Some(max_local_cache_bytes) = max_local_cache_bytes else {
            return Ok(CacheCleanupOutcome {
                evicted_file_ids: Vec::new(),
            });
        };
        if max_local_cache_bytes <= 0 {
            return Ok(CacheCleanupOutcome {
                evicted_file_ids: Vec::new(),
            });
        }

        let pinned_bytes = self.db.cache_bytes_by_effective_pin(true)?.max(0);
        let max_unpinned_cache_bytes = max_local_cache_bytes.saturating_sub(pinned_bytes);
        self.enforce_smart_cache(CachePolicy {
            max_unpinned_cache_bytes,
            ..CachePolicy::default()
        })
    }

    pub fn local_cache_usage_bytes(&self) -> anyhow::Result<i64> {
        let pinned_bytes = self.db.cache_bytes_by_effective_pin(true)?.max(0);
        let unpinned_bytes = self.db.cache_bytes_by_effective_pin(false)?.max(0);
        Ok(pinned_bytes.saturating_add(unpinned_bytes))
    }

    pub fn enforce_configured_cache_limit(&self) -> anyhow::Result<CacheCleanupOutcome> {
        let cfg = crate::config::DesktopConfig::load().unwrap_or_default();
        self.enforce_local_cache_limit(cfg.local_cache_limit_for_eviction())
    }

    #[cfg(target_os = "linux")]
    async fn write_linux_freedesktop_thumbnails(&self, file_id: &str, dest_path: &Path) {
        let entry = match self.db.get_file(file_id) {
            Ok(Some(entry)) if !entry.is_dir() => entry,
            Ok(_) => return,
            Err(error) => {
                tracing::debug!(file_id = %file_id, error = %error, "linux thumbnail skipped: state row unavailable");
                return;
            }
        };

        let source_path = linux_thumbnail_source_path_for_entry(&entry).unwrap_or_else(|| dest_path.to_path_buf());
        let thumbnail = match tokio::time::timeout(
            Duration::from_secs(8),
            self.fetch_thumbnail_to_memory(file_id, "medium"),
        )
        .await
        {
            Ok(Ok(bytes)) => bytes,
            Ok(Err(error)) => {
                tracing::debug!(file_id = %file_id, error = %error, "linux thumbnail skipped: server thumbnail unavailable");
                return;
            }
            Err(_) => {
                tracing::debug!(file_id = %file_id, "linux thumbnail skipped: server thumbnail fetch timed out");
                return;
            }
        };

        match crate::linux_thumbnail::write_freedesktop_thumbnails(&source_path, entry.modified_at, &thumbnail) {
            Ok(paths) => {
                tracing::debug!(
                    file_id = %file_id,
                    count = paths.len(),
                    "linux freedesktop thumbnails written"
                );
            }
            Err(error) => {
                tracing::warn!(file_id = %file_id, error = %error, "linux freedesktop thumbnail write failed");
            }
        }
    }

    #[cfg(target_os = "linux")]
    fn remove_linux_freedesktop_thumbnails_for_file_ids(&self, file_ids: &[String]) {
        for file_id in file_ids {
            let entry = match self.db.get_file(file_id) {
                Ok(Some(entry)) if !entry.is_dir() => entry,
                Ok(_) => continue,
                Err(error) => {
                    tracing::debug!(file_id = %file_id, error = %error, "linux thumbnail cleanup skipped: state row unavailable");
                    continue;
                }
            };
            let Some(source_path) = linux_thumbnail_source_path_for_entry(&entry) else {
                continue;
            };
            if let Err(error) = crate::linux_thumbnail::remove_freedesktop_thumbnails(&source_path) {
                tracing::warn!(file_id = %file_id, error = %error, "linux freedesktop thumbnail cleanup failed");
            }
        }
    }

    pub fn version_conflict_feed(&self) -> anyhow::Result<Vec<VersionConflictEntry>> {
        version_conflict_feed_from_db(self.db.as_ref())
    }

    pub fn queue_restore_version(
        &self,
        file_id: &str,
        version_id: &str,
        last_error: Option<String>,
    ) -> anyhow::Result<PendingOperation> {
        let now = now_secs();
        let op = PendingOperation {
            op_id: uuid::Uuid::new_v4().to_string(),
            kind: OperationKind::RestoreVersion,
            file_id: Some(file_id.to_string()),
            parent_id: None,
            target_path: None,
            metadata_json: Some(
                serde_json::json!({
                    "operation": "restore_version",
                    "version_id": version_id,
                })
                .to_string(),
            ),
            payload_path: None,
            base_version: None,
            base_object_version_id: Some(version_id.to_string()),
            attempts: 0,
            max_attempts: 25,
            next_retry_at: now,
            last_error,
            backup_source_key: None,
            created_at: now,
            updated_at: now,
        };
        self.db.enqueue_operation(&op)?;
        Ok(op)
    }

    fn enqueue_finder_operation(
        &self,
        kind: OperationKind,
        file_id: Option<String>,
        parent_id: Option<String>,
        target_path: Option<String>,
        metadata: serde_json::Value,
        payload_path: Option<String>,
        base_version: Option<i64>,
        base_object_version_id: Option<String>,
    ) -> anyhow::Result<FinderWriteOutcome> {
        let op_id = uuid::Uuid::new_v4().to_string();
        let now = now_secs();
        // Task 0811: tag known-folder backup ops with their origin key (derived
        // from the server-relative path the upload threads as `target_path`), so
        // disabling that folder can purge exactly its queued ops. `None` for any
        // normal user upload — those are never purged by a folder disable. The
        // `_this_device` form pins segment 2 to THIS machine's sanitized device
        // name (the only value the mirror writes), so a user's own
        // `Backup/<other>/<CatalogName>/…` file is never mis-tagged + wrongly
        // purged (review fix).
        let backup_source_key = target_path
            .as_deref()
            .and_then(crate::known_folder::backup_source_key_for_this_device);
        let op = PendingOperation {
            op_id: op_id.clone(),
            kind: kind.clone(),
            file_id: file_id.clone(),
            parent_id,
            target_path,
            metadata_json: Some(serde_json::to_string(&metadata)?),
            payload_path,
            base_version,
            base_object_version_id,
            attempts: 0,
            max_attempts: 25,
            next_retry_at: now,
            last_error: Some("queued from Finder; upload worker not yet attached".to_string()),
            backup_source_key,
            created_at: now,
            updated_at: now,
        };
        self.db.enqueue_operation(&op)?;
        Ok(FinderWriteOutcome::Queued {
            op_id,
            file_id,
            kind,
            ignored: false,
            message: "queued for encrypted sync".to_string(),
        })
    }

    fn enqueue_pin_tree_operation(&self, file_id: &str, pinned: bool, now: i64) -> anyhow::Result<()> {
        let op = PendingOperation {
            op_id: uuid::Uuid::new_v4().to_string(),
            kind: OperationKind::PinTree,
            file_id: Some(file_id.to_string()),
            parent_id: None,
            target_path: None,
            metadata_json: Some(
                serde_json::json!({
                    "operation": "pin_tree",
                    "pinned": pinned,
                })
                .to_string(),
            ),
            payload_path: None,
            base_version: None,
            base_object_version_id: None,
            attempts: 0,
            max_attempts: 25,
            next_retry_at: now,
            last_error: Some("queued recursive pin state; hydration worker not yet attached".to_string()),
            backup_source_key: None,
            created_at: now,
            updated_at: now,
        };
        self.db.enqueue_operation(&op)?;
        Ok(())
    }

    fn enqueue_hydrate_operation(&self, entry: &FileEntry, now: i64) -> anyhow::Result<()> {
        let op = PendingOperation {
            op_id: uuid::Uuid::new_v4().to_string(),
            kind: OperationKind::HydrateFile,
            file_id: Some(entry.file_id.clone()),
            parent_id: None,
            target_path: Some(entry.path.clone()),
            metadata_json: Some(
                serde_json::json!({
                    "operation": "hydrate_file",
                    "reason": "recursive_pin",
                })
                .to_string(),
            ),
            payload_path: None,
            base_version: None,
            base_object_version_id: None,
            attempts: 0,
            max_attempts: 25,
            next_retry_at: now,
            last_error: Some("queued pinned content hydration; transfer worker not yet attached".to_string()),
            backup_source_key: None,
            created_at: now,
            updated_at: now,
        };
        self.db.enqueue_operation(&op)?;
        Ok(())
    }

    fn ensure_shared_parent_allows_write(&self, parent_id: Option<&str>) -> anyhow::Result<Option<FileContractState>> {
        let Some(parent_id) = parent_id else {
            return Ok(None);
        };
        if parent_id == "namespace:shared_with_me" {
            return Err(anyhow::anyhow!(
                "Shared with me is read-only at the namespace root; open an editable shared folder first"
            ));
        }
        let Some(contract) = self.db.get_file_contract_state(parent_id)? else {
            return Ok(None);
        };
        if contract.is_shared() && !contract.can_write() {
            return Err(anyhow::anyhow!(
                "read-only shared folder cannot accept new Finder items"
            ));
        }
        Ok(Some(contract).filter(|contract| contract.is_shared()))
    }

    fn ensure_item_allows_shared_write(
        &self,
        file_id: &str,
        operation: &str,
    ) -> anyhow::Result<Option<FileContractState>> {
        let Some(contract) = self.db.get_file_contract_state(file_id)? else {
            return Ok(None);
        };
        if contract.is_shared() && !contract.can_write() {
            return Err(anyhow::anyhow!(
                "read-only shared item cannot be changed from Finder during {operation}"
            ));
        }
        Ok(Some(contract).filter(|contract| contract.is_shared()))
    }

    /// Download `file_id` from the vault, decrypt, write to `dest_path`.
    /// Called by the Linux FUSE driver, the IPC socket, and conflict-resolution
    /// paths where the decrypted file must land on disk (sync root or a caller-
    /// chosen path). Status transitions: `Downloading` → `Local` on success,
    /// `Error` on failure.
    ///
    /// Steps mirror `repos/cli/src/commands/pull.rs::pull_single_file`:
    ///
    /// 1. Flip status to `Downloading` so the Finder/Explorer overlay
    ///    shows a spinner immediately.
    /// 2. Fetch fresh metadata to learn `chunk_count` (the local
    ///    placeholder may not have it).
    /// 3. Resolve the per-file key. Owned files derive it from the master key;
    ///    shared-with-me files unwrap the sender-provided key envelope using
    ///    the share invite's X25519/HKDF contract.
    /// 4. For each chunk index: GET the bytes, parse as
    ///    `EncryptedBlob` JSON, decrypt with `decrypt_chunk`,
    ///    accumulate plaintext.
    /// 5. Write the whole plaintext to `dest_path` (caller chose the
    ///    layout — `dest_path` is usually `<sync_root>/<decrypted_name>`
    ///    or a per-extension cache path).
    /// 6. Flip status to `Local`.
    ///
    /// On any error the status is flipped to `Error` so the overlay can
    /// render a problem indicator, then the error is bubbled.
    ///
    /// **Windows Cloud Files callers must use [`Self::hydrate_file_to_memory`]
    /// instead** — that variant never writes plaintext to disk, satisfying the
    /// zero-knowledge requirement for on-demand CF hydration.
    pub async fn hydrate_file(&self, file_id: &str, dest_path: &Path, allowed_roots: &[&Path]) -> anyhow::Result<()> {
        self.hydrate_file_with_progress(file_id, dest_path, allowed_roots, None)
            .await
    }

    /// [`Self::hydrate_file`] plus an optional `progress(done_bytes,
    /// total_bytes)` callback, invoked synchronously from the download loop
    /// (task 1670 issue 3 — the macOS IPC handler forwards it to Finder).
    ///
    /// Cancel-safe: if this future is dropped while the download is in
    /// flight (the IPC client hung up), the row's status is restored instead
    /// of being left on `Downloading` forever.
    pub async fn hydrate_file_with_progress(
        &self,
        file_id: &str,
        dest_path: &Path,
        allowed_roots: &[&Path],
        progress: Option<&HydrateProgressFn>,
    ) -> anyhow::Result<()> {
        // Task 1247 self-defense: validate `dest_path` against the caller's
        // trusted root(s) BEFORE any directory creation or plaintext write.
        // `dest_path` reaches the one untrusted caller (the IPC socket handler)
        // straight off the wire, and the `file_id`-keyed guard below
        // (`ensure_shared_hydrate_path_safe`) validates an entirely SEPARATE
        // value (`entry.path`), so it can never vouch for `dest_path`. Without
        // this check any local process could turn the daemon into a
        // decrypt-oracle + arbitrary-write primitive (e.g. writing decrypted
        // vault plaintext over ~/.ssh/authorized_keys).
        if !hydrate_dest_is_allowed(dest_path, allowed_roots) {
            return Err(anyhow::anyhow!("hydrate destination is not within an allowed root"));
        }
        self.ensure_shared_hydrate_path_safe(file_id)?;
        // Spec §7.4.2: a hydrate never changes the status of a row with a live upload of its
        // own (a Finder upload that has not parked): no `Downloading`, `Local` or `Error`.
        // That status is the upload's, and `Error` would make the item read-only.
        let touch_status = !self
            .db
            .item_presentation(file_id)?
            .is_some_and(|presentation| presentation.unparked_finder_upload);
        let set_failed = || {
            if touch_status {
                // Best-effort status flip; if the DB is broken we still
                // return the original error.
                let _ = self.db.set_status(file_id, FileStatus::Error);
            }
        };
        // RAII-style: any early return below the status flip should
        // leave the file in `Error`, not `Downloading`. We do that by
        // wrapping the body in an inner async fn whose Err branch we
        // catch.
        let mut downloading_guard = touch_status.then(|| DownloadingStatusGuard::arm(&self.db, file_id));
        if touch_status {
            self.db.set_status(file_id, FileStatus::Downloading)?;
        }
        // Task 1683 slice 2: the popover's per-file bytes. The caller's own progress
        // callback (the IPC handler forwarding to Finder) still gets every report.
        let transfer = self
            .transfers
            .begin(file_id, crate::transfer_progress::Direction::Down, 0);
        let hydrated = self.do_hydrate(file_id, progress).await;
        // Past the only cancellation point (the download await): every path
        // below sets the final status itself.
        if let Some(guard) = downloading_guard.as_mut() {
            guard.disarm();
        }
        match hydrated {
            Ok(mut buf) => {
                // Write the decrypted bytes to disk (this is the intentional
                // disk-writing path — sync root / conflict resolution / FUSE).
                // Make the destination directory if needed.
                if let Some(parent) = dest_path.parent()
                    && let Err(e) = std::fs::create_dir_all(parent)
                {
                    buf.zeroize();
                    // Task 1670 round 2: this used to `?` straight out of the
                    // match arm below, which skipped the `Err(e) =>` arm's
                    // `FileStatus::Error` flip entirely — a directory-create
                    // failure left the row stuck on `Downloading` forever.
                    // Route it through the same status flip as every other
                    // failure in this function.
                    set_failed();
                    return Err(anyhow::anyhow!("create dest dir {}: {e}", parent.display()));
                }
                // Write via an O_NOFOLLOW handle (task 1247): the containment
                // guard at the top of this fn is a check-then-use, and there is
                // a real time window here (`do_hydrate` did a network download +
                // decrypt). A same-UID attacker could plant a symlink at
                // `dest_path` during that window pointing at, say,
                // ~/.ssh/authorized_keys; a plain `fs::write` would follow it
                // and overwrite the real target with decrypted plaintext.
                // `write_hydrated_plaintext` fails closed if the final component
                // is (or race-becomes) a symlink, closing the race atomically at
                // open() time rather than re-checking-then-hoping. Since task
                // 1670 round 2 it also stages the write under a temp name and
                // publishes with one atomic rename, so a failure anywhere in
                // that sequence is guaranteed to leave NOTHING new at
                // `dest_path` — there is no separate "delete the half-written
                // file" step needed here because the primitive itself never
                // makes one externally visible.
                let write_result = write_hydrated_plaintext(dest_path, allowed_roots, &buf);
                // Zeroize the in-memory copy now that it is on disk (or on
                // error) so the allocation does not linger with plaintext.
                buf.zeroize();
                if let Err(e) = write_result {
                    set_failed();
                    return Err(anyhow::anyhow!("write {}: {e}", dest_path.display()));
                }

                // Task 1698: a Trashing row keeps its marker (viewing a file
                // from the macOS Trash view must not un-trash it in the
                // replica); everything else completes as `Local`.
                // PR #100 review (Codex P1): the pre-hydrate status is the
                // row's status BEFORE the flip to `Downloading` — re-reading
                // the DB here always saw `Downloading`, so a Trashing row
                // completed as `Local` and reparented itself out of the trash
                // view. `DownloadingStatusGuard::arm` captured exactly that
                // pre-flip status when it armed above — reuse it.
                if let Some(guard) = &downloading_guard {
                    self.db
                        .set_status(file_id, hydrate_final_status(guard.restore.clone()))?;
                }
                let downloaded_bytes = self.transfers.get(file_id).map(|t| t.total).unwrap_or(0);
                transfer.finish();
                self.record_transfer_done(crate::transfer_progress::Direction::Down, file_id, downloaded_bytes);

                // Task 1670 round 4 (Codex P2 on PR #75, review thread on
                // `runner.rs:1044`): see [`record_hydration_cache_state`]'s
                // doc comment for the full "why" — in short, a macOS Finder
                // `fetchContents` hydration writes here as a HANDOFF, not a
                // durable local cache copy, and registering it via
                // `mark_cached` anyway left a phantom cache-usage entry in
                // `desktop_storage_summary` / smart-cache eviction after the
                // system (not us) took over the materialized content.
                #[cfg(target_os = "macos")]
                record_hydration_cache_state(
                    &self.db,
                    file_id,
                    dest_path,
                    &crate::ipc_socket::macos_hydrate_cache_dir(),
                )?;
                #[cfg(not(target_os = "macos"))]
                {
                    let cache_bytes = std::fs::metadata(dest_path).map(|m| m.len() as i64).unwrap_or(0);
                    self.db
                        .mark_cached(file_id, &dest_path.to_string_lossy(), cache_bytes, now_secs())?;
                }
                #[cfg(target_os = "linux")]
                self.write_linux_freedesktop_thumbnails(file_id, dest_path).await;
                let _ = self.enforce_configured_cache_limit();
                Ok(())
            }
            Err(e) => {
                set_failed();
                Err(e)
            }
        }
    }

    fn ensure_shared_hydrate_path_safe(&self, file_id: &str) -> anyhow::Result<()> {
        let Some(contract) = self.db.get_file_contract_state(file_id)? else {
            return Ok(());
        };
        if contract.namespace != Namespace::SharedWithMe {
            return Ok(());
        }

        let entry = self
            .db
            .get_file(file_id)?
            .ok_or_else(|| anyhow::anyhow!("shared hydrate target missing state row for {file_id}"))?;
        crate::reject_unsafe_rel_path(&entry.path)
            .map_err(|e| anyhow::anyhow!("unsafe shared hydrate path for {file_id}: {e}"))?;
        Ok(())
    }

    /// Download `file_id` from the vault, decrypt, and return the plaintext
    /// **in memory only** — it is NEVER written to disk.
    ///
    /// This is the Windows Cloud Files hydration path. The Cloud Files runtime
    /// calls `fetch_data_callback` on a filter-driver thread and expects us to
    /// deliver bytes via `CfExecute(TRANSFER_DATA)`; there is no requirement to
    /// materialise the plaintext as a file. Writing to `%TEMP%` would expose
    /// decrypted user data on disk, violating the zero-knowledge contract.
    ///
    /// The returned [`Zeroizing`] wrapper overwrites the buffer with zeros on
    /// drop, so plaintext is wiped from memory as soon as the caller is done
    /// with it — even if an early `return` or `?` is taken. Intermediate
    /// per-chunk decrypted buffers are also explicitly zeroized after being
    /// copied into the accumulator (see [`Self::do_hydrate`]).
    ///
    /// Status transitions: `Downloading` → `Local` on success, `Error` on
    /// failure. The `cache_path` recorded in state_db is `""` (empty) because
    /// the bytes live inside the CF placeholder in the sync root, not at a
    /// separate cache file — `unpinned_local_files_for_dehydration` reconstructs
    /// the real on-disk path from `path` + sync root (see state_db comment).
    #[cfg(target_os = "windows")]
    pub async fn hydrate_file_to_memory(&self, file_id: &str) -> anyhow::Result<Zeroizing<Vec<u8>>> {
        self.db.set_status(file_id, FileStatus::Downloading)?;
        match self.do_hydrate(file_id, None).await {
            Ok(buf) => {
                self.db.set_status(file_id, FileStatus::Local)?;
                // cache_path is empty: on Windows CF the hydrated bytes live
                // INSIDE the placeholder (the CF runtime writes them there after
                // our CfExecute(TRANSFER_DATA) calls). There is no separate temp
                // file. Byte count is still recorded for smart-cache accounting.
                self.db.mark_cached(file_id, "", buf.len() as i64, now_secs())?;
                let _ = self.enforce_configured_cache_limit();
                Ok(buf)
            }
            Err(e) => {
                let _ = self.db.set_status(file_id, FileStatus::Error);
                Err(e)
            }
        }
    }

    /// Core download + decrypt loop. Returns the full plaintext in a
    /// [`Zeroizing`] wrapper so the allocation is wiped on drop regardless of
    /// which exit path is taken (normal return, early `?`, or a panic unwind).
    ///
    /// Intermediate per-chunk buffers from `decrypt_downloaded_chunk` are
    /// explicitly zeroized after being copied into the accumulator, so at most
    /// one extra chunk's worth of plaintext is live in memory at any time.
    ///
    /// This function deliberately does NOT write anything to disk. Callers that
    /// need the bytes on disk (`hydrate_file`) or in memory (`hydrate_file_to_memory`)
    /// handle that themselves so the disk-write decision stays at the call site,
    /// not buried inside the crypto loop.
    fn owned_file_key(&self, file_id: &str) -> beebeeb_core::kdf::FileKey {
        let mk_bytes: [u8; 32] = *self.api.master_key();
        let master_key = beebeeb_core::kdf::MasterKey::from_bytes(mk_bytes);
        beebeeb_core::kdf::derive_file_key(&master_key, file_id.as_bytes())
    }

    async fn file_key_for_download(&self, file_id: &str) -> anyhow::Result<beebeeb_core::kdf::FileKey> {
        let Some(contract) = self.db.get_file_contract_state(file_id)? else {
            return Ok(self.owned_file_key(file_id));
        };
        if contract.namespace != Namespace::SharedWithMe {
            return Ok(self.owned_file_key(file_id));
        }
        if !contract.can_read() {
            anyhow::bail!("shared item is not readable");
        }

        let share_id = contract
            .share_id
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("shared item is missing invite id"))?;
        let body = self.api.list_shared_roots().await?;
        let invite = body
            .get("invites")
            .and_then(|value| value.as_array())
            .into_iter()
            .flatten()
            .find(|invite| shared_invite_id_matches(invite, share_id))
            .ok_or_else(|| anyhow::anyhow!("shared invite {share_id} was not returned by the server"))?;
        let mapping = shared_root_from_invite(invite)
            .ok_or_else(|| anyhow::anyhow!("shared invite {share_id} is not approved or is malformed"))?;
        let sender_public_key = mapping
            .sender_public_key
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("shared invite {share_id} is missing sender_public_key"))?;

        if mapping.is_folder {
            let encrypted_folder_key = mapping
                .encrypted_folder_key
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("folder share {share_id} is missing encrypted_folder_key"))?;
            let folder_keys = self.api.get_folder_keys(share_id).await?;
            unwrap_folder_share_file_key(
                self.api.master_key(),
                sender_public_key,
                &mapping.file_id,
                encrypted_folder_key,
                file_id,
                &folder_keys,
            )
        } else {
            let encrypted_file_key = mapping
                .encrypted_file_key
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("share invite {share_id} is missing encrypted_file_key"))?;
            unwrap_direct_shared_file_key(self.api.master_key(), sender_public_key, file_id, encrypted_file_key)
        }
    }

    async fn do_hydrate(
        &self,
        file_id: &str,
        progress: Option<&HydrateProgressFn>,
    ) -> anyhow::Result<Zeroizing<Vec<u8>>> {
        let _file_uuid: uuid::Uuid = file_id
            .parse()
            .map_err(|e| anyhow::anyhow!("invalid file_id (not a UUID): {e}"))?;

        // Per-file metadata. We trust the server's chunk_count rather
        // than the local entry's because a file we know about as
        // `cloud_only` may have been re-uploaded with a new chunk
        // layout since we last saw it.
        let meta = self.api.get_file(file_id).await?;
        self.do_hydrate_with_meta(file_id, &meta, progress).await
    }

    /// Shared core of [`Self::do_hydrate`] and [`Self::remote_content_preview`]
    /// (task 1546 Codex round 2, finding 1): downloads + decrypts every chunk
    /// for `file_id` given ALREADY-FETCHED metadata. `remote_content_preview`
    /// needs the metadata anyway to check the remote size before deciding
    /// whether to download at all — routing through this shared helper
    /// instead of `do_hydrate` means that check doesn't cost a second
    /// `GET /files/{id}` round trip for files it does end up downloading.
    ///
    /// Internal helper only — callers are responsible for their own
    /// `file_id` UUID validation (both current callers already do theirs
    /// before this is reached).
    async fn do_hydrate_with_meta(
        &self,
        file_id: &str,
        meta: &serde_json::Value,
        progress: Option<&HydrateProgressFn>,
    ) -> anyhow::Result<Zeroizing<Vec<u8>>> {
        let chunk_count = meta
            .get("chunk_count")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| anyhow::anyhow!("server response missing chunk_count"))? as u32;

        let file_key = self.file_key_for_download(file_id).await?;

        // Rate-limit ceiling for downloads.
        let download_kbps_limit = crate::config::DesktopConfig::load()
            .map(|c| c.download_kbps_limit)
            .unwrap_or(0);

        // Walk chunks. Pre-allocate roughly the file size if known,
        // but fall back to defaults — chunks are encrypted so the
        // ciphertext is always larger than plaintext anyway.
        // Use Zeroizing so that if we bail mid-loop (network error,
        // decrypt error) the partial plaintext is still wiped.
        let approx_size = meta.get("size_bytes").and_then(|v| v.as_u64()).unwrap_or(0) as usize;

        self.download_and_decrypt_chunk_range(
            file_id,
            &file_key,
            0..chunk_count,
            download_kbps_limit,
            approx_size,
            progress,
        )
        .await
    }

    /// Download + decrypt chunk indices `[range.start, range.end)` for
    /// `file_id`, concatenating the plaintext into one [`Zeroizing`] buffer.
    /// Shared by [`Self::do_hydrate`] (the whole-file range `0..chunk_count`)
    /// and [`Self::hydrate_file_range`] (a covering sub-range) so the
    /// pacing/wire-counting/zeroize discipline lives in exactly one place.
    ///
    /// `approx_size_hint` only sizes the initial allocation (`Vec::with_capacity`)
    /// — it never bounds or truncates the actual read, so a wrong hint costs at
    /// most a reallocation, never a silent short read.
    async fn download_and_decrypt_chunk_range(
        &self,
        file_id: &str,
        file_key: &beebeeb_core::kdf::FileKey,
        range: std::ops::Range<u32>,
        download_kbps_limit: u64,
        approx_size_hint: usize,
        progress: Option<&HydrateProgressFn>,
    ) -> anyhow::Result<Zeroizing<Vec<u8>>> {
        let mut acc: Zeroizing<Vec<u8>> = Zeroizing::new(Vec::with_capacity(approx_size_hint));
        // Task 1670 issue 3: tell the caller (the macOS IPC handler, which
        // forwards it to Finder's progress bar) how far along we are. `total`
        // is the size hint (server's plaintext size); the initial 0/total
        // report lets the UI switch from indeterminate to determinate before
        // the first chunk lands.
        let total = approx_size_hint as u64;
        // Task 1683 slice 2: the board first, so a caller that reads it from inside its
        // own callback sees the same bytes it was just told about. A no-op for a file
        // that is not registered (a partial range read).
        self.transfers.report(file_id, 0, total);
        if let Some(report) = progress {
            report(0, total);
        }

        for i in range {
            let chunk_start = std::time::Instant::now();
            let chunk_bytes = self.api.download_chunk(file_id, i).await?;
            let wire_len = chunk_bytes.len() as u64;
            // Decrypt into a temporary buffer, copy into the accumulator,
            // then zeroize the temporary. This limits live plaintext to
            // the accumulator + one chunk at any moment.
            let mut decrypted = decrypt_downloaded_chunk(file_key, &chunk_bytes)
                .map_err(|e| anyhow::anyhow!("decrypt chunk {i}: {e}"))?;
            acc.extend_from_slice(&decrypted);
            decrypted.zeroize();
            self.transfers.report(file_id, acc.len() as u64, total);
            if let Some(report) = progress {
                report(acc.len() as u64, total);
            }

            // P1 — wire-byte counter: count raw wire bytes received.
            self.wire.download_bytes.fetch_add(wire_len, Ordering::Relaxed);

            // E — token-bucket pacing for downloads.
            if download_kbps_limit > 0 {
                let budget_secs = wire_len as f64 / (download_kbps_limit as f64 * 1024.0);
                let elapsed_secs = chunk_start.elapsed().as_secs_f64();
                if budget_secs > elapsed_secs {
                    let sleep_ms = ((budget_secs - elapsed_secs) * 1000.0) as u64;
                    if sleep_ms > 0 {
                        tokio::time::sleep(Duration::from_millis(sleep_ms)).await;
                    }
                }
            }
        }

        Ok(acc)
    }

    /// Download `file_id` from the vault, decrypt, and return ONLY the plaintext
    /// bytes covering `[required_offset, required_offset + required_length)` —
    /// **in memory only**, never written to disk. This is the range-targeted
    /// counterpart to [`Self::hydrate_file_to_memory`] (task 1024, follow-up to
    /// 0769): instead of decrypting the whole file on every CF fetch callback, it
    /// downloads + decrypts only the chunks that cover the requested range, so
    /// peak memory is bounded by one covering-range buffer (typically a handful
    /// of chunks) rather than the entire file.
    ///
    /// Requires an authoritative, uniform `chunk_size_bytes` for the file. This
    /// value must come from the server's stored `object_versions` row; it cannot
    /// be recovered from `size_bytes / chunk_count`, because real files use a
    /// fixed chunk size with only the final chunk shortened.
    ///
    /// Returns `Ok(None)` (never an error) when the covering-chunk math can't be
    /// established from the metadata (`chunk_count` is `0`, `size_bytes` is
    /// missing, or `chunk_size_bytes` is missing/invalid) — the caller falls
    /// back to
    /// [`Self::hydrate_file_to_memory`] for the whole-file path in that case.
    ///
    /// Status bookkeeping: unlike [`Self::hydrate_file_to_memory`], this method
    /// does **not** flip the row to `FileStatus::Local` — a single range fetch is
    /// not evidence the whole file is now cached, so marking it fully `Local`
    /// here would be a stronger claim than the bytes we actually have. See the
    /// caller (`windows_cf::callbacks::fetch_data_callback`) for how "fully
    /// in-sync" is decided instead.
    #[cfg(any(target_os = "windows", test))]
    pub async fn hydrate_file_range(
        &self,
        file_id: &str,
        required_offset: u64,
        required_length: u64,
    ) -> anyhow::Result<Option<HydratedRange>> {
        let _file_uuid: uuid::Uuid = file_id
            .parse()
            .map_err(|e| anyhow::anyhow!("invalid file_id (not a UUID): {e}"))?;

        let meta = self.api.get_file(file_id).await?;
        let chunk_count = meta.get("chunk_count").and_then(|v| v.as_i64()).unwrap_or(0) as u64;
        let size_bytes = meta.get("size_bytes").and_then(|v| v.as_u64());
        let chunk_size_bytes = meta.get("chunk_size_bytes").and_then(|v| v.as_u64());

        let (Some(size_bytes), Some(chunk_size_bytes), true) = (size_bytes, chunk_size_bytes, chunk_count > 0) else {
            // Missing metadata — caller falls back to the whole-file path.
            return Ok(None);
        };
        if chunk_size_bytes == 0 {
            return Ok(None);
        }

        // Clamp the requested range to the file's real extent before computing
        // covering chunks, so a stale/over-wide CF request never derives an
        // out-of-bounds chunk index.
        let range_end = required_offset.saturating_add(required_length).min(size_bytes);
        if range_end <= required_offset {
            // Degenerate/empty range — nothing to hydrate.
            return Ok(Some(HydratedRange {
                data: Zeroizing::new(Vec::new()),
                range_start_bytes: required_offset,
                covers_whole_file: size_bytes == 0,
            }));
        }

        let Some(plan) = plan_hydration_chunk_range(
            size_bytes,
            chunk_count,
            chunk_size_bytes,
            required_offset,
            required_length,
        )?
        else {
            return Ok(None);
        };

        self.db.set_status(file_id, FileStatus::Downloading)?;

        let file_key = self.file_key_for_download(file_id).await?;
        let download_kbps_limit = crate::config::DesktopConfig::load()
            .map(|c| c.download_kbps_limit)
            .unwrap_or(0);

        match self
            .download_and_decrypt_chunk_range(
                file_id,
                &file_key,
                plan.first_chunk..plan.last_chunk_exclusive,
                download_kbps_limit,
                plan.covering_span_hint,
                None,
            )
            .await
        {
            Ok(data) => {
                // Only a whole-file covering range is evidence the file is fully
                // cached; a partial range leaves the row `Downloading` for the
                // caller (the CF callback) to resolve based on what CF itself
                // reports (see `windows_cf::callbacks`).
                if plan.covers_whole_file {
                    self.db.set_status(file_id, FileStatus::Local)?;
                    self.db.mark_cached(file_id, "", data.len() as i64, now_secs())?;
                    let _ = self.enforce_configured_cache_limit();
                }
                Ok(Some(HydratedRange {
                    data,
                    range_start_bytes: plan.range_start_bytes,
                    covers_whole_file: plan.covers_whole_file,
                }))
            }
            Err(e) => {
                let _ = self.db.set_status(file_id, FileStatus::Error);
                Err(e)
            }
        }
    }

    /// Fetch the encrypted server thumbnail for `file_id`, decrypt it **in
    /// memory only**, and return the decoded image bytes (WebP / JPEG / PNG)
    /// inside a [`Zeroizing`] wrapper.
    ///
    /// This is the shared read path for OS thumbnail consumers. It mirrors
    /// [`Self::do_hydrate`]'s per-file-key derivation but hits
    /// `GET /files/:id/thumbnail/{variant}` instead of the chunk range and reuses
    /// the **existing** encrypted thumbnail the original uploading client
    /// generated (a downscaled still for images, a poster frame for video). OS
    /// integrations never regenerate a thumbnail and never decode the original
    /// media just to preview it — they decrypt this small blob.
    ///
    /// ## Wire envelope (NOT the chunk envelope)
    ///
    /// Thumbnails are stored in the **raw** `nonce(12) || AES-256-GCM(ct+tag)`
    /// envelope the web/mobile clients write (`thumbnail.ts encryptThumbnailBlob`):
    /// a 12-byte random nonce followed by the AES-256-GCM ciphertext+tag, keyed
    /// directly by the per-file key, with no AAD and no JSON. This differs from
    /// file *chunks* (which are the `EncryptedBlob` JSON / `decrypt_chunk_raw`
    /// frame), so this path decrypts the blob directly rather than going through
    /// [`decrypt_downloaded_chunk`].
    ///
    /// ## Zero-knowledge
    ///
    /// The decrypted bytes live ONLY in the returned `Zeroizing<Vec<u8>>`, which
    /// is wiped on drop (normal return, `?`, or panic unwind). Callers decide how
    /// to consume the decoded image bytes: Windows decodes straight to an
    /// in-memory `HBITMAP`; Linux writes the freedesktop.org cache PNG derivative
    /// required by file managers.
    ///
    /// `variant` is `small` / `medium` / `large`. Any error (no thumbnail for
    /// this file, network failure, decrypt failure) bubbles as `Err` so the
    /// caller can fall back to the file-type icon — it is never reported as a
    /// success with empty bytes.
    pub async fn fetch_thumbnail_to_memory(&self, file_id: &str, variant: &str) -> anyhow::Result<Zeroizing<Vec<u8>>> {
        use aes_gcm::aead::Aead;
        use aes_gcm::{Aes256Gcm, KeyInit, Nonce};

        // Validate the id shape up front (same guard as do_hydrate).
        let _file_uuid: uuid::Uuid = file_id
            .parse()
            .map_err(|e| anyhow::anyhow!("invalid file_id (not a UUID): {e}"))?;

        // Pull the encrypted thumbnail blob. A 404 (no thumbnail) is an Err here
        // and the COM caller maps it to the type-icon fallback.
        let blob = self.api.download_thumbnail(file_id, variant).await?;
        if blob.len() < 12 + 16 {
            // Must hold at least a 12-byte nonce + 16-byte GCM tag.
            anyhow::bail!("thumbnail blob too short ({} bytes)", blob.len());
        }

        let file_key = self.file_key_for_download(file_id).await?;

        // Split nonce || ciphertext+tag and AES-256-GCM decrypt with no AAD.
        let (nonce_bytes, ciphertext) = blob.split_at(12);
        let cipher = Aes256Gcm::new_from_slice(file_key.as_bytes())
            .map_err(|e| anyhow::anyhow!("init thumbnail cipher: {e}"))?;
        let nonce = Nonce::from_slice(nonce_bytes);
        let plaintext = cipher
            .decrypt(nonce, ciphertext)
            // The error type carries no plaintext; a generic message avoids
            // leaking anything about the ciphertext.
            .map_err(|_| anyhow::anyhow!("thumbnail decrypt failed (bad key or corrupt blob)"))?;

        // Hand the decoded image bytes back wrapped so they are wiped on drop.
        Ok(Zeroizing::new(plaintext))
    }

    /// Apply a "Keep Mine" resolution: stage the current local bytes and upload
    /// them as a new server version using the same chunked upload path as normal
    /// Finder writes. On success, [`Self::upload_version`] applies the regular
    /// upload completion bookkeeping (status, version, object version, size, and
    /// remote timestamp). On failure, the row is left conflicted and the error is
    /// returned to the caller so the UI does not report a false resolution.
    ///
    /// `sync_root` is supplied by the caller because the bridge itself doesn't
    /// know it (the runner owns that).
    pub async fn resolve_keep_mine(&self, file_id: &str, sync_root: &Path) -> anyhow::Result<()> {
        let entry = self
            .db
            .get_file(file_id)?
            .ok_or_else(|| anyhow::anyhow!("no state.db row for {file_id}"))?;
        if entry.is_dir() {
            return Err(anyhow::anyhow!("Keep Mine upload requires a file row: {file_id}"));
        }

        let shared_contract = self.ensure_item_allows_shared_write(file_id, "keep mine")?;
        let contract = self.db.get_file_contract_state(file_id)?;
        let parent_id = contract.as_ref().and_then(|contract| contract.parent_id.clone());
        let file_name = display_name_for_path(&entry.path);
        let content_type = contract
            .as_ref()
            .and_then(|contract| contract.content_type.clone())
            .or_else(|| beebeeb_core::media::guess_mime_type(&file_name).map(str::to_string));
        let name_encrypted =
            encrypted_metadata_for_name(self.api.master_key(), file_id, &file_name, content_type.as_deref())?;

        let local_path = local_file_path_under_sync_root(sync_root, &entry.path)?;
        let staged =
            crate::staged_payload::StagedPayload::copy(self.db.clone(), &local_path, self.finder_staging_root()?)?;
        let staged_path = staged.path().to_string();
        let staged_size = std::fs::metadata(&staged_path).map(|m| m.len()).unwrap_or(0);

        let mut metadata = serde_json::json!({
            "operation": "upload_version",
            "name_encrypted": name_encrypted,
            "content_type": content_type,
            "size_bytes": staged_size,
            "uploaded_by": "authenticated_desktop_user",
        });
        apply_shared_context(&mut metadata, shared_contract.as_ref());

        let now = now_secs();
        let op = PendingOperation {
            op_id: uuid::Uuid::new_v4().to_string(),
            kind: OperationKind::UploadVersion,
            file_id: Some(file_id.to_string()),
            parent_id,
            target_path: Some(entry.path.clone()),
            metadata_json: Some(serde_json::to_string(&metadata)?),
            payload_path: Some(staged_path.clone()),
            // Keep Mine is an explicit conflict override. Passing the stale base
            // that caused the conflict would make the server reject the upload.
            base_version: None,
            base_object_version_id: None,
            attempts: 0,
            max_attempts: 1,
            next_retry_at: now,
            last_error: None,
            backup_source_key: None,
            created_at: now,
            updated_at: now,
        };

        let mut post_complete_errors = Vec::new();
        let released = match self
            .upload_version(&op, None, sync_root, &mut post_complete_errors)
            .await
        {
            Ok(OpDone::Remove { release } | OpDone::Removed { release }) => release,
            Err(e) => {
                // One-shot op (never queued): nothing will resume its session.
                self.abandon_upload_after_give_up(&op).await;
                return Err(anyhow::anyhow!("Keep Mine upload failed: {e}"));
            }
        };
        // macOS: the upload leaves its staged copy for the caller to release. This op
        // was never queued, so there is no removal to wait for.
        if let Some(path) = released.as_deref()
            && let Err(e) = crate::staged_payload::remove(&self.db, Path::new(path))
        {
            tracing::warn!(error = %e, "staged upload cleanup deferred; journal retained");
        }
        // Task 1700: thumbnail failures never fail the resolution, but they
        // must not vanish silently either.
        for error in &post_complete_errors {
            tracing::warn!(file_id = %file_id, error = %error, "conflict keep-mine upload: post-complete thumbnail work skipped");
        }

        tracing::info!(file_id = %file_id, "conflict resolved: keep mine uploaded local version");
        Ok(())
    }

    /// Apply a "Keep Theirs" resolution: download the remote version
    /// and overwrite the local file at `<sync_root>/<entry.path>`.
    /// Status ends in `Local`; `remote_updated_at` is anchored to
    /// "now" so the next tick treats the file as freshly synced.
    ///
    /// Reuses [`Self::hydrate_file`], which already handles status
    /// transitions and Error-on-failure rollback. The extra
    /// `remote_updated_at` bump after a successful hydrate prevents
    /// the next tick from re-flagging a conflict if the user's old
    /// `content_hash` is still on the row (it isn't anymore — the row
    /// was already in Conflict — but we belt-and-brace anchor anyway).
    pub async fn resolve_keep_theirs(&self, file_id: &str, sync_root: &Path) -> anyhow::Result<()> {
        let mut entry = self
            .db
            .get_file(file_id)?
            .ok_or_else(|| anyhow::anyhow!("no state.db row for {file_id}"))?;
        let dest = local_file_path_under_sync_root(sync_root, &entry.path)?;
        // hydrate_file flips Conflict → Downloading → Local on success
        // (or Error on failure). We don't care about the intermediate
        // state for this path.
        self.hydrate_file(file_id, &dest, &[sync_root]).await?;
        let now_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        entry.status = FileStatus::Local;
        entry.remote_updated_at = now_secs;
        entry.modified_at = now_secs;
        self.db.upsert_file(&entry)?;
        tracing::info!(file_id = %file_id, dest = %dest.display(), "conflict resolved: keep theirs");
        Ok(())
    }

    /// Apply a "Keep Both" resolution: rename the local copy to a
    /// device-suffixed conflict filename, hydrate the remote into the
    /// original path, flip status back to `Local`. Called by Task 13's
    /// auto-resolution timer in [`crate::runner`] and (eventually) by
    /// the user-driven `resolve_conflict` IPC when the user picks
    /// "Keep Both" from the conflict window.
    ///
    /// Filesystem ops are done in this order so a partial failure
    /// leaves recoverable state:
    ///
    ///   1. Rename local → conflict copy (cheap, instantaneous)
    ///   2. Hydrate remote into the original path (network — may
    ///      fail; if it does, the user still has both their original
    ///      *and* the conflict copy on disk, so no data is lost; we
    ///      flip status to `Error` so the next tick retries).
    ///
    /// `sync_root` is supplied by the caller because the bridge
    /// itself doesn't know it (the runner owns that).
    pub async fn auto_resolve_keep_both(&self, sync_root: &Path, entry: &FileEntry) -> anyhow::Result<String> {
        let original = local_file_path_under_sync_root(sync_root, &entry.path)?;

        // If the local file no longer exists on disk (user deleted it
        // outside the daemon), Keep Both collapses to Keep Remote.
        if !original.exists() {
            self.hydrate_file(&entry.file_id, &original, &[sync_root]).await?;
            self.db.set_status(&entry.file_id, FileStatus::Local)?;
            return Ok(entry.path.clone());
        }

        let host = hostname::get()
            .map(|h| h.to_string_lossy().to_string())
            .unwrap_or_else(|_| "device".into());
        let date = chrono::Utc::now().format("%Y-%m-%d");
        let stem = original.file_stem().and_then(|s| s.to_str()).unwrap_or("file");
        let ext = original
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| format!(".{e}"))
            .unwrap_or_default();
        let conflict_name = format!("{stem} (conflict - {host} - {date}){ext}");
        let conflict_path = original.with_file_name(&conflict_name);

        std::fs::rename(&original, &conflict_path)
            .map_err(|e| anyhow::anyhow!("rename {} -> {}: {e}", original.display(), conflict_path.display()))?;

        // Hydrate remote into the now-vacant original path. On failure
        // we mark Error rather than Conflict — the conflict copy is
        // safe on disk, the remote download just needs a retry.
        if let Err(e) = self.hydrate_file(&entry.file_id, &original, &[sync_root]).await {
            self.db.set_status(&entry.file_id, FileStatus::Error)?;
            return Err(e);
        }
        self.db.set_status(&entry.file_id, FileStatus::Local)?;
        Ok(conflict_name)
    }

    /// Read-only content preview for the conflict-resolution window (task
    /// 1546 finding 2): reads the LOCAL file straight off disk and
    /// downloads+decrypts the CURRENT REMOTE version, without touching
    /// state.db — unlike [`Self::hydrate_file`] / [`Self::hydrate_file_to_memory`],
    /// which both flip the row's status to `Downloading`/`Local` and record a
    /// cache entry. The row is mid-conflict and neither side has been chosen
    /// yet, so daemon bookkeeping must not move just because the user opened
    /// the window — that's why this calls the private `do_hydrate` directly
    /// instead of either public hydrate wrapper.
    ///
    /// Replaces `ConflictWindow.tsx`'s previous hardcoded placeholder text
    /// ("Content from this device…" / "Content from other device…"), which
    /// its own doc-comment admitted was fake for every conflict.
    ///
    /// Task 1546 Codex round 2, finding 3: textness is decided HERE, from
    /// the file's own path via [`is_text_file`], never taken from a
    /// caller-supplied flag — the VersionCenter-initiated open always passed
    /// a hardcoded `isText: false` (only the daemon's auto-open path derived
    /// it correctly from the filename), which made every manually-reviewed
    /// text conflict render as binary and silently discard both text bodies.
    pub async fn conflict_content_preview(
        &self,
        file_id: &str,
        sync_root: &Path,
    ) -> anyhow::Result<ConflictContentPreview> {
        let entry = self
            .db
            .get_file(file_id)?
            .ok_or_else(|| anyhow::anyhow!("no state.db row for {file_id}"))?;
        let is_text = is_text_file(&entry.path);

        let local = self.local_content_preview(sync_root, &entry.path, is_text);
        let remote = self.remote_content_preview(file_id, is_text).await;

        Ok(ConflictContentPreview { is_text, local, remote })
    }

    /// Local half of [`Self::conflict_content_preview`] (task 1546 Codex
    /// round 2, finding 1): stats the file to learn its size WITHOUT reading
    /// it, and only reads bytes at all when a text preview applies AND the
    /// file is within [`CONFLICT_PREVIEW_TEXT_MAX_BYTES`] — bounded to one
    /// byte past the cap via [`std::io::Read::take`] so a race where the
    /// file grows between the stat and the read can only ever push the
    /// result to "too large," never load an oversized buffer. Conflict
    /// windows open automatically, so a multi-gigabyte local file must never
    /// be pulled fully into memory just to report its size.
    fn local_content_preview(&self, sync_root: &Path, entry_path: &str, is_text: bool) -> ConflictContentSide {
        let local_path = match local_file_path_under_sync_root(sync_root, entry_path) {
            Ok(p) => p,
            Err(e) => {
                return ConflictContentSide {
                    size_bytes: None,
                    text: None,
                    unavailable_reason: Some(format!("Couldn't locate the local file: {e}")),
                };
            }
        };

        let size_bytes = match std::fs::metadata(&local_path) {
            Ok(m) => m.len(),
            Err(e) => {
                return ConflictContentSide {
                    size_bytes: None,
                    text: None,
                    unavailable_reason: Some(format!("Couldn't read the local file: {e}")),
                };
            }
        };

        if !is_text {
            // Binary preview only ever shows size — the bytes are never read.
            return ConflictContentSide {
                size_bytes: Some(size_bytes),
                text: None,
                unavailable_reason: None,
            };
        }
        if size_bytes > CONFLICT_PREVIEW_TEXT_MAX_BYTES as u64 {
            return ConflictContentSide {
                size_bytes: Some(size_bytes),
                text: None,
                unavailable_reason: Some(format!("File is too large to preview, {size_bytes} bytes")),
            };
        }

        let file = match std::fs::File::open(&local_path) {
            Ok(f) => f,
            Err(e) => {
                return ConflictContentSide {
                    size_bytes: Some(size_bytes),
                    text: None,
                    unavailable_reason: Some(format!("Couldn't read the local file: {e}")),
                };
            }
        };
        let mut bytes = Vec::with_capacity((size_bytes as usize).min(CONFLICT_PREVIEW_TEXT_MAX_BYTES) + 1);
        // `take(LIMIT + 1)` bounds the read itself — never `std::fs::read`
        // (unbounded) — so even a TOCTOU race where the file grows after the
        // `metadata()` call above can produce at most LIMIT+1 bytes.
        match file
            .take(CONFLICT_PREVIEW_TEXT_MAX_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
        {
            Ok(_) => content_side_from_bytes(bytes, is_text),
            Err(e) => ConflictContentSide {
                size_bytes: Some(size_bytes),
                text: None,
                unavailable_reason: Some(format!("Couldn't read the local file: {e}")),
            },
        }
    }

    /// Remote half of [`Self::conflict_content_preview`] (task 1546 Codex
    /// round 2, finding 1): fetches file metadata — one small JSON response —
    /// to learn the remote size BEFORE deciding whether to download
    /// anything. [`Self::do_hydrate_with_meta`] (which downloads and
    /// decrypts every chunk) is called only when BOTH a text preview applies
    /// AND the metadata size is within [`CONFLICT_PREVIEW_TEXT_MAX_BYTES`] —
    /// a multi-gigabyte conflict, text or binary, therefore never costs
    /// bandwidth or a full decrypt just to be previewed. Reuses the SAME
    /// metadata fetch `do_hydrate_with_meta` needs instead of letting
    /// `do_hydrate` re-fetch it, so the small-file path costs exactly the
    /// metadata GET + the chunk GETs it always cost.
    async fn remote_content_preview(&self, file_id: &str, is_text: bool) -> ConflictContentSide {
        if let Err(e) = file_id.parse::<uuid::Uuid>() {
            return ConflictContentSide {
                size_bytes: None,
                text: None,
                unavailable_reason: Some(format!("invalid file_id (not a UUID): {e}")),
            };
        }

        let meta = match self.api.get_file(file_id).await {
            Ok(m) => m,
            Err(e) => {
                return ConflictContentSide {
                    size_bytes: None,
                    text: None,
                    unavailable_reason: Some(format!("Couldn't download the other device's version: {e}")),
                };
            }
        };
        let size_bytes = meta.get("size_bytes").and_then(|v| v.as_u64());

        if !is_text {
            // Binary preview only ever shows size — never download the bytes.
            return ConflictContentSide {
                size_bytes,
                text: None,
                unavailable_reason: None,
            };
        }
        let within_limit = matches!(size_bytes, Some(s) if s <= CONFLICT_PREVIEW_TEXT_MAX_BYTES as u64);
        if !within_limit {
            return ConflictContentSide {
                size_bytes,
                text: None,
                unavailable_reason: Some(match size_bytes {
                    Some(s) => format!("File is too large to preview, {s} bytes"),
                    None => "Couldn't determine the other device's file size".to_string(),
                }),
            };
        }

        match self.do_hydrate_with_meta(file_id, &meta, None).await {
            Ok(mut bytes) => {
                // Move the plaintext out instead of `bytes.to_vec()` — a
                // clone would briefly hold two live copies of the remote
                // plaintext in memory. `content_side_from_bytes` consumes
                // the Vec by value (no further copy), and the now-empty
                // `bytes` is zeroized below for defense in depth.
                let side = content_side_from_bytes(std::mem::take(&mut *bytes), is_text);
                bytes.zeroize();
                side
            }
            Err(e) => ConflictContentSide {
                size_bytes,
                text: None,
                unavailable_reason: Some(format!("Couldn't download the other device's version: {e}")),
            },
        }
    }
}

/// One side (local or remote) of a conflict-resolution content preview.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct ConflictContentSide {
    pub size_bytes: Option<u64>,
    /// UTF-8 text content — present only when the caller asked for a text
    /// preview AND the bytes are valid UTF-8 AND within
    /// [`CONFLICT_PREVIEW_TEXT_MAX_BYTES`]. `None` always means "see
    /// `unavailable_reason`", never a silent truncation.
    pub text: Option<String>,
    /// Human-readable reason `text` is absent (too large, not UTF-8, local
    /// read failed, remote download failed) — `None` when `text` is present
    /// or this is the (expected-textless) binary branch.
    pub unavailable_reason: Option<String>,
}

/// Result of [`EngineBridge::conflict_content_preview`] — the real content
/// (or an honest reason it's unavailable) for both sides of a conflict.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ConflictContentPreview {
    pub is_text: bool,
    pub local: ConflictContentSide,
    pub remote: ConflictContentSide,
}

/// Cap on how large a text file's content preview may be. Deliberately
/// small: the whole file round-trips over Tauri's IPC as a JSON string and
/// is diffed synchronously in the webview, so this bounds both the IPC
/// payload and the diff algorithm's input size.
const CONFLICT_PREVIEW_TEXT_MAX_BYTES: usize = 256 * 1024;

/// Pure classifier: turns real file bytes into what the conflict window can
/// safely show. Never fabricates content — a non-text file, an oversized
/// file, or invalid UTF-8 all report `text: None` plus an honest
/// `unavailable_reason`, never a placeholder string.
fn content_side_from_bytes(bytes: Vec<u8>, is_text: bool) -> ConflictContentSide {
    let size_bytes = Some(bytes.len() as u64);
    if !is_text {
        return ConflictContentSide {
            size_bytes,
            text: None,
            unavailable_reason: None,
        };
    }
    if bytes.len() > CONFLICT_PREVIEW_TEXT_MAX_BYTES {
        return ConflictContentSide {
            size_bytes,
            text: None,
            unavailable_reason: Some("File is too large to preview inline".to_string()),
        };
    }
    match String::from_utf8(bytes) {
        Ok(text) => ConflictContentSide {
            size_bytes,
            text: Some(text),
            unavailable_reason: None,
        },
        Err(_) => ConflictContentSide {
            size_bytes,
            text: None,
            unavailable_reason: Some("File isn't valid UTF-8 text".to_string()),
        },
    }
}

/// Name of the per-sync-root state directory (mirrors `runner::STATE_DIR`).
const SYNC_STATE_DIR: &str = ".beebeeb";
/// Name of the cross-process lock file the engine writes at the sync root.
const SYNC_LOCK_FILE: &str = ".beebeeb-sync.lock";

/// True if `path` is something the engine itself writes (so it must never be
/// fed back as a user upload): the `<sync_root>/.beebeeb-sync.lock`, anything
/// inside `<sync_root>/.beebeeb/`, or any path component named `.beebeeb`
/// (defense in depth for a nested-root layout).
///
/// Shared by [`EngineBridge::classify_local_path`] (filter 1) so the retired
/// `notify` path and the Windows Cloud Files NOTIFY callbacks run ONE identical
/// engine-internal check.
pub(crate) fn path_is_engine_internal(sync_root: &Path, path: &Path) -> bool {
    let state_dir = sync_root.join(SYNC_STATE_DIR);
    let lock = sync_root.join(SYNC_LOCK_FILE);
    if path == lock || path.starts_with(&state_dir) {
        return true;
    }
    path.components()
        .any(|c| matches!(c.as_os_str().to_str(), Some(SYNC_STATE_DIR)))
}

/// Map an absolute on-disk path under `sync_root` to the server-relative,
/// '/'-separated, leading-slash-free key the state DB stores in `files.path`
/// (the same shape [`crate::state_db::StateDb::get_file_by_path`] expects, and
/// the same the nested-enumeration sweep writes). Returns `None` if `path` is
/// not under `sync_root` or resolves to the empty (root) key.
///
/// Shared by [`EngineBridge::classify_local_path`] and
/// [`EngineBridge::resolve_parent_id_for`] so the DB lookup key is computed in
/// exactly one place.
pub(crate) fn relative_db_path(sync_root: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(sync_root).ok()?;
    let mut out = String::new();
    for (i, comp) in rel.components().enumerate() {
        let part = comp.as_os_str().to_str()?;
        if i > 0 {
            out.push('/');
        }
        out.push_str(part);
    }
    if out.is_empty() { None } else { Some(out) }
}

pub fn is_ignored_finder_name(name: &str) -> bool {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return true;
    }
    let lower = trimmed.to_ascii_lowercase();
    matches!(
        trimmed,
        ".DS_Store" | ".DocumentRevisions-V100" | ".Spotlight-V100" | ".TemporaryItems" | ".Trashes" | "TemporaryItems"
    ) || trimmed.starts_with("._")
        || trimmed.starts_with("~$")
        || trimmed.ends_with('~')
        || lower.ends_with(".tmp")
        || lower.ends_with(".temp")
        || lower.ends_with(".swp")
        || lower.ends_with(".swo")
        || lower.ends_with(".part")
        || lower.ends_with(".crdownload")
}

pub fn version_conflict_feed_from_db(db: &StateDb) -> anyhow::Result<Vec<VersionConflictEntry>> {
    let mut entries = Vec::new();

    for file in db.list_by_status(FileStatus::Conflict)? {
        entries.push(VersionConflictEntry {
            id: format!("conflict:{}", file.file_id),
            file_id: file.file_id.clone(),
            file_name: display_name_for_path(&file.path),
            kind: "conflict".to_string(),
            status: "needs review".to_string(),
            updated_at: Some(file.modified_at),
            detail: "Local and remote both changed from the last synced base.".to_string(),
            action: "open_conflict".to_string(),
            op_id: None,
            version_id: None,
            base_version: None,
            last_error: None,
        });
    }

    for op in db.list_review_operations()? {
        entries.push(review_entry_for_operation(&op, db)?);
    }

    entries.sort_by(|a, b| {
        b.updated_at
            .unwrap_or_default()
            .cmp(&a.updated_at.unwrap_or_default())
            .then_with(|| a.id.cmp(&b.id))
    });
    Ok(entries)
}

fn encrypted_metadata_for_name(
    master_key_bytes: &[u8; 32],
    file_id: &str,
    filename: &str,
    mime_type: Option<&str>,
) -> anyhow::Result<String> {
    let master_key = beebeeb_core::kdf::MasterKey::from_bytes(*master_key_bytes);
    beebeeb_core::encrypt::encrypt_name(&master_key, file_id, filename, mime_type)
        .map_err(|e| anyhow::anyhow!("encrypt Finder metadata: {e}"))
}

/// The `init` guard (spec §6.3.1), checked where the request is built: a File Provider
/// write (`has_write_id`) that replaces a file is never sent without a base. A missing
/// base, or one that does not fit the server's `i32` (which
/// [`upload_init_request_for_operation`] would drop as "no base"), parks it as
/// `base_unknown`. A create sends no base, and uploads without a write id (Windows, the
/// watcher) keep today's bases.
fn guard_init_base(base_version: Option<i64>, is_new_file: bool, has_write_id: bool) -> anyhow::Result<()> {
    if !has_write_id || is_new_file {
        return Ok(());
    }
    match base_version.map(i32::try_from) {
        Some(Ok(_)) => Ok(()),
        _ => Err(anyhow::Error::new(ParkNow(ParkReason::BaseUnknown))),
    }
}

fn upload_init_request_for_operation(
    file_id: &str,
    name_encrypted: &str,
    content_type: Option<String>,
    parent_id: Option<String>,
    plaintext_size: u64,
    base_version: Option<i64>,
    is_new_file: bool,
) -> DesktopUploadInitRequest {
    let plan = beebeeb_types::plan_chunks(plaintext_size, beebeeb_types::ChunkProfile::Desktop);
    DesktopUploadInitRequest {
        file_id: if is_new_file { None } else { Some(file_id.to_string()) },
        file_name: name_encrypted.to_string(),
        file_size_bytes: plaintext_size,
        mime_type: None,
        parent_id,
        profile: "desktop".to_string(),
        is_media: beebeeb_core::media::is_media(content_type.as_deref()),
        chunk_size_bytes: Some(plan.chunk_size_bytes),
        chunk_count: Some(plan.chunk_count),
        base_version_number: base_version.and_then(|version| i32::try_from(version).ok()),
    }
}

fn prepare_thumbnail_uploads_for_plaintext_media(
    payload_path: &Path,
    mime_type: Option<&str>,
    file_key: &beebeeb_core::kdf::FileKey,
) -> anyhow::Result<Vec<PreparedThumbnailUpload>> {
    if !beebeeb_core::media::is_media(mime_type) {
        return Ok(Vec::new());
    }

    let source = decode_thumbnail_source(payload_path, mime_type)?;
    let blurhash = if source.is_video {
        None
    } else {
        blurhash_for_source(&source).ok().flatten()
    };

    let mut uploads = Vec::new();
    for variant in THUMBNAIL_VARIANTS_FOR_UPLOAD {
        let config = variant.config();
        let output =
            match beebeeb_core::thumbnail::generate_thumbnail(&source.rgba, source.width, source.height, &config) {
                Ok(output) => output,
                Err(e) => {
                    tracing::warn!(
                        variant = variant.label(),
                        error = %e,
                        "thumbnail generation skipped for variant"
                    );
                    continue;
                }
            };
        let plaintext = Zeroizing::new(output.data);
        let encrypted = Zeroizing::new(
            beebeeb_core::encrypt::encrypt_chunk_raw(file_key, &plaintext)
                .map_err(|e| anyhow::anyhow!("encrypt thumbnail {}: {e}", variant.label()))?,
        );
        if encrypted.len() > variant.encrypted_max_bytes() {
            tracing::warn!(
                variant = variant.label(),
                bytes = encrypted.len(),
                max_bytes = variant.encrypted_max_bytes(),
                "thumbnail generation skipped because encrypted payload is too large"
            );
            continue;
        }
        uploads.push(PreparedThumbnailUpload {
            variant,
            encrypted,
            blurhash: if variant == ThumbnailUploadVariant::Medium {
                blurhash.clone()
            } else {
                None
            },
        });
    }

    Ok(uploads)
}

fn decode_thumbnail_source(payload_path: &Path, mime_type: Option<&str>) -> anyhow::Result<ThumbnailSource> {
    match mime_type {
        Some(mime) if mime.starts_with("video/") => decode_video_thumbnail_source(payload_path),
        _ => decode_image_thumbnail_source(payload_path),
    }
}

fn decode_image_thumbnail_source(payload_path: &Path) -> anyhow::Result<ThumbnailSource> {
    let image = image::ImageReader::open(payload_path)?
        .with_guessed_format()?
        .decode()
        .map_err(|e| anyhow::anyhow!("decode image for thumbnail: {e}"))?;
    dynamic_image_to_thumbnail_source(image, false)
}

fn decode_video_thumbnail_source(payload_path: &Path) -> anyhow::Result<ThumbnailSource> {
    let mut last_error: Option<anyhow::Error> = None;
    for seek in ["1", "0"] {
        match decode_video_frame_with_ffmpeg(payload_path, seek) {
            Ok(source) => return Ok(source),
            Err(e) => last_error = Some(e),
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("video thumbnail extraction failed")))
}

fn decode_video_frame_with_ffmpeg(payload_path: &Path, seek: &str) -> anyhow::Result<ThumbnailSource> {
    let ffmpeg = std::env::var_os("BEEBEEB_FFMPEG_PATH").unwrap_or_else(|| "ffmpeg".into());
    let output = Command::new(ffmpeg)
        .arg("-nostdin")
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-ss")
        .arg(seek)
        .arg("-i")
        .arg(payload_path)
        .arg("-frames:v")
        .arg("1")
        .arg("-f")
        .arg("image2pipe")
        .arg("-vcodec")
        .arg("png")
        .arg("-")
        .output()
        .map_err(|e| anyhow::anyhow!("spawn ffmpeg for video thumbnail: {e}"))?;

    if !output.status.success() || output.stdout.is_empty() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let snippet: String = stderr.chars().take(240).collect();
        anyhow::bail!("ffmpeg video thumbnail failed: {snippet}");
    }

    let frame_png = Zeroizing::new(output.stdout);
    let image =
        image::load_from_memory(&frame_png).map_err(|e| anyhow::anyhow!("decode ffmpeg thumbnail frame: {e}"))?;
    dynamic_image_to_thumbnail_source(image, true)
}

fn dynamic_image_to_thumbnail_source(image: image::DynamicImage, is_video: bool) -> anyhow::Result<ThumbnailSource> {
    let rgba = image.into_rgba8();
    let width = rgba.width();
    let height = rgba.height();
    if width == 0 || height == 0 {
        anyhow::bail!("thumbnail source has zero dimensions");
    }
    Ok(ThumbnailSource {
        rgba: Zeroizing::new(rgba.into_raw()),
        width,
        height,
        is_video,
    })
}

fn blurhash_for_source(source: &ThumbnailSource) -> anyhow::Result<Option<String>> {
    let (small_rgba, width, height) = beebeeb_core::thumbnail::resize_for_thumbnail(
        &source.rgba,
        source.width,
        source.height,
        BLURHASH_SOURCE_MAX_DIMENSION,
    )
    .map_err(|e| anyhow::anyhow!("resize blurhash source: {e}"))?;
    let small_rgba = Zeroizing::new(small_rgba);
    Ok(encode_blurhash_rgba(
        &small_rgba,
        width,
        height,
        BLURHASH_COMPONENTS_X,
        BLURHASH_COMPONENTS_Y,
    ))
}

fn encode_blurhash_rgba(
    rgba: &[u8],
    width: u32,
    height: u32,
    components_x: usize,
    components_y: usize,
) -> Option<String> {
    if width == 0
        || height == 0
        || components_x == 0
        || components_x > 9
        || components_y == 0
        || components_y > 9
        || rgba.len() != width as usize * height as usize * 4
    {
        return None;
    }

    let mut factors = Vec::with_capacity(components_x * components_y);
    for y in 0..components_y {
        for x in 0..components_x {
            factors.push(multiply_blurhash_basis(rgba, width, height, x, y));
        }
    }

    let size_flag = (components_x - 1) + (components_y - 1) * 9;
    let mut encoded = String::with_capacity(4 + 2 * factors.len());
    encoded.push_str(&encode_base83(size_flag as u32, 1));

    let maximum_value = if factors.len() > 1 {
        let actual_max = factors[1..]
            .iter()
            .flat_map(|factor| factor.iter())
            .fold(0.0_f64, |max, value| max.max(value.abs()));
        let quantized = ((actual_max * 166.0 - 0.5).floor() as i32).clamp(0, 82) as u32;
        encoded.push_str(&encode_base83(quantized, 1));
        (quantized + 1) as f64 / 166.0
    } else {
        encoded.push_str(&encode_base83(0, 1));
        1.0
    };

    encoded.push_str(&encode_base83(encode_blurhash_dc(factors[0]), 4));
    for factor in factors.iter().skip(1) {
        encoded.push_str(&encode_base83(encode_blurhash_ac(*factor, maximum_value), 2));
    }

    if encoded.len() <= 64 { Some(encoded) } else { None }
}

fn multiply_blurhash_basis(rgba: &[u8], width: u32, height: u32, component_x: usize, component_y: usize) -> [f64; 3] {
    let normalisation = if component_x == 0 && component_y == 0 { 1.0 } else { 2.0 };
    let width_f = width as f64;
    let height_f = height as f64;
    let mut r = 0.0;
    let mut g = 0.0;
    let mut b = 0.0;

    for y in 0..height {
        for x in 0..width {
            let basis = (std::f64::consts::PI * component_x as f64 * x as f64 / width_f).cos()
                * (std::f64::consts::PI * component_y as f64 * y as f64 / height_f).cos();
            let idx = ((y * width + x) as usize) * 4;
            let alpha = rgba[idx + 3] as f64 / 255.0;
            let sr = (rgba[idx] as f64 / 255.0) * alpha + (1.0 - alpha);
            let sg = (rgba[idx + 1] as f64 / 255.0) * alpha + (1.0 - alpha);
            let sb = (rgba[idx + 2] as f64 / 255.0) * alpha + (1.0 - alpha);
            r += basis * srgb_to_linear(sr);
            g += basis * srgb_to_linear(sg);
            b += basis * srgb_to_linear(sb);
        }
    }

    let scale = normalisation / (width_f * height_f);
    [r * scale, g * scale, b * scale]
}

fn srgb_to_linear(value: f64) -> f64 {
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(value: f64) -> u32 {
    let value = value.clamp(0.0, 1.0);
    let srgb = if value <= 0.0031308 {
        value * 12.92
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    };
    (srgb * 255.0 + 0.5).floor().clamp(0.0, 255.0) as u32
}

fn encode_blurhash_dc(value: [f64; 3]) -> u32 {
    (linear_to_srgb(value[0]) << 16) + (linear_to_srgb(value[1]) << 8) + linear_to_srgb(value[2])
}

fn encode_blurhash_ac(value: [f64; 3], maximum_value: f64) -> u32 {
    let quant_r = quantize_blurhash_ac(value[0], maximum_value);
    let quant_g = quantize_blurhash_ac(value[1], maximum_value);
    let quant_b = quantize_blurhash_ac(value[2], maximum_value);
    quant_r * 19 * 19 + quant_g * 19 + quant_b
}

fn quantize_blurhash_ac(value: f64, maximum_value: f64) -> u32 {
    let normalized = if maximum_value > 0.0 {
        value / maximum_value
    } else {
        0.0
    };
    (sign_pow(normalized.clamp(-1.0, 1.0), 0.5) * 9.0 + 9.5)
        .floor()
        .clamp(0.0, 18.0) as u32
}

fn sign_pow(value: f64, exp: f64) -> f64 {
    value.abs().powf(exp).copysign(value)
}

fn encode_base83(mut value: u32, length: usize) -> String {
    let mut chars = vec![0u8; length];
    for i in (0..length).rev() {
        chars[i] = BLURHASH_BASE83[(value % 83) as usize];
        value /= 83;
    }
    String::from_utf8(chars).expect("base83 alphabet is ASCII")
}

fn file_key_for(master_key: &[u8; 32], server_file_id: &str) -> beebeeb_core::kdf::FileKey {
    let master_key = beebeeb_core::kdf::MasterKey::from_bytes(*master_key);
    beebeeb_core::kdf::derive_file_key(&master_key, server_file_id.as_bytes())
}

/// Staged-payload modification time in nanoseconds (0 when unavailable). Part
/// of the resume fingerprint: a session is resumed only onto the same bytes.
fn payload_mtime_ns(path: &Path) -> i64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| i64::try_from(d.as_nanos()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// Fill `buffer` from `file` (short only at EOF). Chunk `k` must be exactly the
/// bytes at `k * chunk_size`, so a resumed upload that seeks to the
/// acknowledged watermark lines up with the chunks the server already holds.
fn read_full_chunk(file: &mut std::fs::File, buffer: &mut [u8]) -> std::io::Result<usize> {
    let mut filled = 0;
    while filled < buffer.len() {
        match file.read(&mut buffer[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(filled)
}

/// The server no longer accepts writes to this upload session: 404 (session
/// or file row gone — e.g. reaped by the stale-upload sweep), 410, or 400
/// (session not writable / chunk plan no longer matches / a chunk the client
/// believed acknowledged is missing at `complete`). Resuming cannot succeed;
/// the op must start a fresh session.
fn upload_session_is_gone(error: &anyhow::Error) -> bool {
    error
        .chain()
        .filter_map(|cause| cause.downcast_ref::<reqwest::Error>())
        .filter_map(reqwest::Error::status)
        .any(|status| matches!(status.as_u16(), 400 | 404 | 410))
}

/// The HTTP status of the request that failed somewhere in `error`'s chain,
/// if it was an HTTP error. An `init` refused with 409 carries its class instead
/// of reqwest's error (spec §8.4), and still answers 409 here.
pub(crate) fn error_http_status(error: &anyhow::Error) -> Option<u16> {
    if init_conflict_class(error).is_some() {
        return Some(409);
    }
    error
        .chain()
        .find_map(|cause| cause.downcast_ref::<reqwest::Error>())
        .and_then(reqwest::Error::status)
        .map(|status| status.as_u16())
}

/// The class of an `init` 409 somewhere in `error`'s chain (spec §8.4).
pub(crate) fn init_conflict_class(error: &anyhow::Error) -> Option<crate::api_client::InitConflictClass> {
    error
        .chain()
        .find_map(|cause| cause.downcast_ref::<crate::api_client::InitConflict>())
        .map(|conflict| conflict.class)
}

/// One warning per upload attempt the server refused with 409, with its class
/// (a stale base, another upload of the file in progress, or other), and a
/// distinct one when that attempt parks the op: its attempts are used up, or a
/// stale base parked it at once (spec §8.4). Ids, counts and the class only: the
/// file's name, the request URL and the server's text never reach the log.
fn log_refused_upload(op: &PendingOperation, attempt: i64, class: crate::api_client::InitConflictClass) {
    let file_id = op.file_id.as_deref().unwrap_or_default();
    if attempt >= op.max_attempts {
        let reason = (class == crate::api_client::InitConflictClass::StaleBase).then(|| ParkReason::StaleBase.as_str());
        tracing::warn!(
            op_id = %op.op_id,
            file_id,
            attempt,
            max_attempts = op.max_attempts,
            base_version = ?op.base_version,
            class = class.as_str(),
            reason,
            "upload refused by the server (409 Conflict); attempts used up, parked with its bytes kept in the queue"
        );
    } else {
        tracing::warn!(
            op_id = %op.op_id,
            file_id,
            attempt,
            max_attempts = op.max_attempts,
            base_version = ?op.base_version,
            class = class.as_str(),
            "upload refused by the server (409 Conflict); will retry"
        );
    }
}

/// A guarded queue write matched no row: the op moved since its claim (spec §8.7 S2, S4).
#[derive(Debug)]
pub(crate) struct QueueStateMoved;

impl std::fmt::Display for QueueStateMoved {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("queue state moved")
    }
}

impl std::error::Error for QueueStateMoved {}

/// What the runner still does after an op succeeded.
#[derive(Debug)]
pub(crate) enum OpDone {
    /// Remove the op now (`finish_claimed`).
    Remove { release: Option<String> },
    /// The landing transaction already removed it; only unlink the released copy.
    Removed { release: Option<String> },
}

/// One line per retried landing of an upload the server completed (spec §8.6 rule 6):
/// ids and the attempt only (spec §11).
fn log_completed_landing_retried(op_id: &str, file_id: Option<&str>, attempt: i64) {
    tracing::warn!(
        op_id = %op_id,
        file_id = file_id.unwrap_or_default(),
        attempt,
        "upload completed on the server; local landing will be retried"
    );
}

/// One line for an attempt whose op moved since its claim: the op id and the
/// step only (spec §11).
fn log_queue_state_moved(op_id: &str, step: &'static str) {
    tracing::warn!(op_id = %op_id, step, "queue state moved");
}

/// Park the claimed op now, with its bytes kept (spec §8.4, §6.3.1).
#[derive(Debug)]
pub(crate) struct ParkNow(pub ParkReason);

impl std::fmt::Display for ParkNow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "upload parked: {}", self.0.as_str())
    }
}

impl std::error::Error for ParkNow {}

/// One line when a queued write takes over a parked predecessor at its claim
/// (spec §8.4): ids only.
fn log_took_over(op_id: &str, file_id: Option<&str>, parked_op_id: &str) {
    tracing::warn!(
        op_id = %op_id,
        file_id = file_id.unwrap_or_default(),
        parked_op_id = %parked_op_id,
        "queued write took over a parked one"
    );
}

/// One line when an upload parks: ids and the reason only (spec §11).
fn log_parked(op_id: &str, file_id: Option<&str>, reason: ParkReason) {
    tracing::warn!(
        op_id = %op_id,
        file_id = file_id.unwrap_or_default(),
        reason = reason.as_str(),
        "upload parked with its bytes kept in the queue"
    );
}

/// `op`'s metadata with its encrypted name (if it carries one) re-encrypted
/// for `file_id`. The plaintext name is the op's display name; the MIME hint
/// is the op's content type, else the one guessed from the name, as the
/// Finder write path does.
fn metadata_rekeyed_to(master_key: &[u8; 32], op: &PendingOperation, file_id: &str) -> anyhow::Result<Option<String>> {
    let Some(raw) = op.metadata_json.as_deref() else {
        return Ok(None);
    };
    let mut metadata: serde_json::Value = serde_json::from_str(raw)?;
    if metadata.get("name_encrypted").and_then(|v| v.as_str()).is_none() {
        return Ok(Some(raw.to_string()));
    }
    let Some(name) = metadata_display_name(&metadata, op) else {
        anyhow::bail!(
            "queued operation {} has an encrypted name but no display name",
            op.op_id
        );
    };
    let mime = metadata["content_type"]
        .as_str()
        .map(str::to_string)
        .or_else(|| beebeeb_core::media::guess_mime_type(&name).map(str::to_string));
    metadata["name_encrypted"] = serde_json::json!(encrypted_metadata_for_name(
        master_key,
        file_id,
        &name,
        mime.as_deref()
    )?);
    Ok(Some(serde_json::to_string(&metadata)?))
}

fn is_create_file_operation(metadata: &serde_json::Value) -> bool {
    metadata["operation"].as_str() == Some("create_file")
}

fn metadata_display_name(metadata: &serde_json::Value, op: &PendingOperation) -> Option<String> {
    metadata["display_name"]
        .as_str()
        .map(str::to_string)
        .or_else(|| op.target_path.as_deref().map(display_name_for_path))
}

fn decrypt_downloaded_chunk(
    file_key: &beebeeb_core::kdf::FileKey,
    chunk_bytes: &[u8],
) -> Result<Vec<u8>, beebeeb_core::CoreError> {
    match beebeeb_core::encrypt::decrypt_chunk_raw(file_key, chunk_bytes) {
        Ok(bytes) => Ok(bytes),
        Err(raw_error) => {
            let blob: EncryptedBlob = serde_json::from_slice(chunk_bytes).map_err(|_| raw_error)?;
            beebeeb_core::encrypt::decrypt_chunk(file_key, &blob)
        }
    }
}

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

#[cfg(any(target_os = "windows", test))]
struct HydrationChunkRangePlan {
    first_chunk: u32,
    last_chunk_exclusive: u32,
    range_start_bytes: u64,
    covering_span_hint: usize,
    covers_whole_file: bool,
}

#[cfg(any(target_os = "windows", test))]
fn chunk_layout_matches_metadata(size_bytes: u64, chunk_count: u64, chunk_size_bytes: u64) -> bool {
    if chunk_count == 0 || chunk_size_bytes == 0 {
        return false;
    }

    let size = size_bytes as u128;
    let count = chunk_count as u128;
    let chunk = chunk_size_bytes as u128;
    if count == 1 {
        return size <= chunk;
    }

    let full_prefix = (count - 1) * chunk;
    let full_span = count * chunk;
    size > full_prefix && size <= full_span
}

#[cfg(any(target_os = "windows", test))]
fn plan_hydration_chunk_range(
    size_bytes: u64,
    chunk_count: u64,
    chunk_size_bytes: u64,
    required_offset: u64,
    required_length: u64,
) -> anyhow::Result<Option<HydrationChunkRangePlan>> {
    if !chunk_layout_matches_metadata(size_bytes, chunk_count, chunk_size_bytes) {
        return Ok(None);
    }

    let range_end = required_offset.saturating_add(required_length).min(size_bytes);
    if range_end <= required_offset {
        return Ok(None);
    }

    let first_chunk = required_offset / chunk_size_bytes;
    let last_chunk = (range_end - 1) / chunk_size_bytes;
    if last_chunk >= chunk_count {
        return Ok(None);
    }

    let first_chunk_u32 =
        u32::try_from(first_chunk).map_err(|_| anyhow::anyhow!("chunk index {first_chunk} exceeds u32 range"))?;
    let last_chunk_u32 =
        u32::try_from(last_chunk).map_err(|_| anyhow::anyhow!("chunk index {last_chunk} exceeds u32 range"))?;
    let last_chunk_exclusive = last_chunk_u32
        .checked_add(1)
        .ok_or_else(|| anyhow::anyhow!("chunk range end exceeds u32 range"))?;
    let chunk_size_usize = usize::try_from(chunk_size_bytes)
        .map_err(|_| anyhow::anyhow!("chunk_size_bytes {chunk_size_bytes} exceeds usize range"))?;
    let chunk_count_in_range = usize::try_from(last_chunk_u32 - first_chunk_u32 + 1)
        .map_err(|_| anyhow::anyhow!("covering chunk count exceeds usize range"))?;
    let covering_span_hint = chunk_count_in_range
        .checked_mul(chunk_size_usize)
        .ok_or_else(|| anyhow::anyhow!("covering range size exceeds usize range"))?;

    Ok(Some(HydrationChunkRangePlan {
        first_chunk: first_chunk_u32,
        last_chunk_exclusive,
        range_start_bytes: first_chunk * chunk_size_bytes,
        covering_span_hint,
        covers_whole_file: first_chunk == 0 && last_chunk == chunk_count - 1,
    }))
}

pub(crate) fn local_file_path_under_sync_root(sync_root: &Path, rel_path: &str) -> anyhow::Result<PathBuf> {
    crate::reject_unsafe_rel_path(rel_path)
        .map_err(|e| anyhow::anyhow!("local path must stay under the sync root: {e}"))?;
    Ok(sync_root.join(rel_path.replace('/', std::path::MAIN_SEPARATOR_STR)))
}

/// `progress(done_bytes, total_bytes)` callback for [`EngineBridge::hydrate_file_with_progress`].
/// Called synchronously from the download loop, so it must be cheap and must
/// not block (the IPC handler just pushes onto a channel).
pub(crate) type HydrateProgressFn = dyn Fn(u64, u64) + Send + Sync;

/// Restores a row's status if [`EngineBridge::hydrate_file_with_progress`] is
/// dropped mid-download (the IPC client hung up and the future was cancelled).
/// Without it a cancelled hydrate leaves the row on `Downloading` forever,
/// which the overlay renders as a permanent spinner. Restores the status the
/// row had before hydration started; a stale `Downloading` (or a missing row)
/// falls back to `CloudOnly`, the state hydration is meant to leave.
struct DownloadingStatusGuard<'a> {
    db: &'a StateDb,
    file_id: &'a str,
    restore: FileStatus,
    armed: bool,
}

impl<'a> DownloadingStatusGuard<'a> {
    fn arm(db: &'a StateDb, file_id: &'a str) -> Self {
        let restore = match db.get_file(file_id).ok().flatten().map(|entry| entry.status) {
            Some(FileStatus::Downloading) | None => FileStatus::CloudOnly,
            Some(previous) => previous,
        };
        Self {
            db,
            file_id,
            restore,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for DownloadingStatusGuard<'_> {
    fn drop(&mut self) {
        if self.armed {
            let _ = self.db.set_status(self.file_id, self.restore.clone());
        }
    }
}

/// Task 1698: the status a SUCCESSFUL hydrate leaves the row in. A `Trashing`
/// row (the user is viewing a file from the macOS Trash view) must KEEP its
/// marker — flipping it to `Local` would reparent the item back under its
/// folder in the replica's working set (the trash view would lose it). Any
/// other row completes normally as `Local` (the content is now on disk).
pub(crate) fn hydrate_final_status(previous: FileStatus) -> FileStatus {
    match previous {
        FileStatus::Trashing => FileStatus::Trashing,
        _ => FileStatus::Local,
    }
}

/// Task 1247: is `dest_path` a safe hydration target — i.e. inside (or about to
/// be created inside) at least one of the caller's trusted `allowed_roots`?
///
/// Pure and free-standing so the containment decision is directly unit-testable
/// without a live `EngineBridge`. Mirrors the two-layer containment pattern
/// already used by `lib.rs::reveal_and_open_file` (see its documented 3-layer
/// comment block):
///   - If `dest_path` already exists, canonicalize both and require containment.
///   - If it does not exist yet (the normal hydrate case — we are about to
///     create it), require the PARENT directory to be contained AND the file
///     name to be a single normal component with no embedded separator, so a
///     traversal like `<root>/../evil` cannot slip past the parent check.
///
/// Returns true if ANY root passes; false if none do. An empty `allowed_roots`
/// slice returns false (fail-closed), never vacuously true.
pub(crate) fn hydrate_dest_is_allowed(dest_path: &Path, allowed_roots: &[&Path]) -> bool {
    for &root in allowed_roots {
        let ok = if dest_path.exists() {
            crate::is_contained(root, dest_path)
        } else {
            let parent_ok = dest_path
                .parent()
                .map(|p| crate::is_contained(root, p))
                .unwrap_or(false);
            let name_ok = dest_path
                .file_name()
                .map(|n| {
                    let s = n.to_string_lossy();
                    !s.contains('/') && !s.contains('\\')
                })
                .unwrap_or(false);
            parent_ok && name_ok
        };
        if ok {
            return true;
        }
    }
    false
}

/// Task 1670 round 4 (Codex P2 on PR #75, review thread on `runner.rs:1044`):
/// `true` when `dest_path` is a macOS Finder `fetchContents` HANDOFF staging
/// file — i.e. it lives inside `hydrate_dir`, never a durable local cache
/// copy. `hydrate_dir` is a parameter (never
/// `crate::ipc_socket::macos_hydrate_cache_dir()` read inline) so this is
/// unit-testable with a throwaway directory, never the real, singleton
/// App-Group container a live Beebeeb.app/Finder extension share on this
/// same machine (`macos_hydrate_cache_dir`'s own doc comment covers why that
/// directory must never be written to by a test process here — round 2's
/// `disposable_cache_roots_includes_the_macos_hydrate_cache_dir` test follows
/// the identical "real path, read-only, no write into it" precedent).
///
/// See [`record_hydration_cache_state`] for the full "why this matters"
/// writeup: registering this file's path via `mark_cached` would point
/// `files.cache_path`/`cache_bytes` at a file the SYSTEM (via
/// `FileProviderExtension.copyToSystemTemporaryDirectory(stagedAt:)`), not
/// this daemon, now owns and that our own copy stops existing moments after
/// this check runs — `desktop_storage_summary` and smart-cache eviction both
/// sum/act on any row with a non-null `cache_path` and no existence check.
#[cfg(target_os = "macos")]
fn is_macos_finder_handoff_staging_path(dest_path: &Path, hydrate_dir: &Path) -> bool {
    crate::is_contained(hydrate_dir, dest_path)
}

/// Task 1670 round 4 (Codex P2 on PR #75, review thread on `runner.rs:1044`):
/// register `dest_path` as this file's durable cache entry (`mark_cached`,
/// same as before this round) — UNLESS it is a macOS Finder `fetchContents`
/// HANDOFF staging file (i.e. [`is_macos_finder_handoff_staging_path`] against
/// `macos_hydrate_dir`), in which case this deliberately does nothing.
///
/// **Why:** `FileProviderExtension.fetchContents` (round 4,
/// `BeebeebFileProvider/FileProviderExtension.swift`) copies a staged file
/// like this one into the SYSTEM's own
/// `NSFileProviderManager.temporaryDirectoryURL()` and deletes OUR copy at
/// `dest_path` immediately after — so `dest_path` can already be gone by the
/// time anything reads `files.cache_path` again. Registering it anyway
/// pointed `desktop_storage_summary` (`cache_bytes_by_effective_pin`,
/// `state_db.rs`) and smart-cache eviction
/// (`evict_unpinned_cache_until_under`, `enforce_configured_cache_limit`) at
/// a file that stops existing almost immediately — both sum/act on ANY row
/// with `cache_path IS NOT NULL AND cache_bytes > 0` with no existence
/// check — so every Finder peek left a phantom cache-usage entry, and a
/// target for a pointless eviction of a file that was never really
/// "cached" at all. [`Self::hydrate_file`]'s `set_status(..., Local)` call
/// (just before this one runs) still flips Finder's own downloaded badge
/// (`FileProviderItem.isDownloaded` reads `status == "local"`,
/// `BeebeebFileProvider/FileProviderItem.swift`) — that is the one real UI
/// signal a momentary Finder peek should produce; it should not also claim
/// a durable cache footprint the system, not us, now owns.
///
/// `macos_hydrate_dir` is a parameter — production passes the REAL
/// `crate::ipc_socket::macos_hydrate_cache_dir()` (see the call site in
/// [`Self::hydrate_file`]); tests pass a throwaway tempdir, so this whole
/// decision is exercised end to end without ever touching the real,
/// singleton App-Group directory a live Beebeeb.app/Finder extension share
/// on this same machine.
#[cfg(target_os = "macos")]
fn record_hydration_cache_state(
    db: &StateDb,
    file_id: &str,
    dest_path: &Path,
    macos_hydrate_dir: &Path,
) -> anyhow::Result<()> {
    if is_macos_finder_handoff_staging_path(dest_path, macos_hydrate_dir) {
        return Ok(());
    }
    let cache_bytes = std::fs::metadata(dest_path).map(|m| m.len() as i64).unwrap_or(0);
    db.mark_cached(file_id, &dest_path.to_string_lossy(), cache_bytes, now_secs())?;
    Ok(())
}

/// Task 1247: write hydrated plaintext to `dest_path`, enforcing that it lands
/// inside an allowed root even against an actively-racing same-UID attacker.
///
/// The threat: `hydrate_file` validates the destination *before* the network
/// download/decrypt (`do_hydrate`), but the write happens *after* — a real
/// wall-clock window. Earlier rounds re-checked containment and then re-opened
/// the parent by PATH; those are two independent, non-atomic path resolutions,
/// so a plain directory `rename()` swap between them (no symlink needed) still
/// escaped: `O_NOFOLLOW` on the second open only refuses a *symlink* at that
/// name, it says nothing about whether the name still resolves to the SAME
/// inode that was validated.
///
/// The fix eliminates the second path resolution entirely. We descend from a
/// trusted allowed root to the parent directory ONE COMPONENT AT A TIME, purely
/// via `openat` relative to already-open directory fds, never touching an
/// absolute path string again after the first (root) open. Each component's
/// `openat` IS its own validation — it happens exactly once, atomically, and
/// once a directory fd is open a rename of its name elsewhere in the tree can no
/// longer affect that fd. `O_NOFOLLOW` on every hop refuses symlinks, and
/// because each inode is reached only as a direct child entry of an
/// already-in-root directory fd, nothing can escape the root subtree.
///
/// Also (unchanged from the prior round): the leaf is created with `O_NOFOLLOW`
/// and then `fchmod`'d to `0o600` unconditionally — POSIX applies the `O_CREAT`
/// mode only to a newly created inode, so an attacker-planted pre-existing
/// `0o644` file would otherwise keep its perms and leak the plaintext.
///
/// Non-unix writes with `File::create` (`openat`/`fchmod`/`O_NOFOLLOW` are Unix-only,
/// and the Windows Cloud Files path never writes plaintext to disk via this fn —
/// it uses `hydrate_file_to_memory`).
///
/// **Task 1670 round 2 addition:** the plaintext is staged under a per-call,
/// randomly-suffixed temp name in the SAME directory, then published onto the
/// real `leaf` name with one atomic `renameat` — anchored to the SAME
/// already-validated `dir_fd` for the temp create, the write, AND the rename,
/// never re-resolving by path (this keeps the anchored-descent invariant
/// `create_leaf_relative_is_anchored_to_original_dir_fd_across_rename_swap`
/// pins). Two consequences:
/// - Any reader racing this write (another same-UID process, or the macOS
///   hydrate-cache TTL sweep running concurrently in a different IPC
///   connection) can never observe a file at `dest_path` that exists but is
///   only partially written — it either isn't there yet, or it's complete.
/// - On ANY failure after the temp file is created, that temp file is removed
///   before the error is returned, so nothing new is ever left behind at
///   `dest_path` — there is no separate "clean up the half-written file" step
///   for a caller to remember.
///
/// The leaf-is-currently-a-symlink refusal is preserved with the EXACT same
/// observable behavior as before (a probe `open(O_NOFOLLOW)`, not
/// `fstat`/`lstat`, so a symlink leaf still fails closed with `ELOOP`) even
/// though the final publish step is now a `renameat`, which — unlike
/// `open`+`O_TRUNC` — does not dereference a symlink destination at all and so
/// could never be tricked into writing THROUGH one to an outside target on its
/// own. Keeping the probe is about not silently replacing a foreign symlink
/// with a real file, not about a plaintext-leak risk `renameat` doesn't have.
///
/// Known residual (documented follow-up, not closed here): a same-filesystem
/// HARD link to a file outside all allowed roots bypasses containment (the walk
/// sees a regular in-root leaf), and `O_TRUNC` would overwrite the linked inode.
/// Noted in the task file.
pub(crate) fn write_hydrated_plaintext(dest_path: &Path, allowed_roots: &[&Path], buf: &[u8]) -> std::io::Result<()> {
    write_hydrated_from_reader(dest_path, allowed_roots, &mut &buf[..]).map(|_| ())
}

/// [`write_hydrated_plaintext`]'s core, reading the plaintext from `reader` (spec §7.4: a
/// fetch served from a queued write's staged copy streams it). Every step is the same:
/// the anchored descent, the `O_NOFOLLOW` temp file, the atomic rename. Returns the number
/// of bytes written.
#[cfg(unix)]
pub(crate) fn write_hydrated_from_reader(
    dest_path: &Path,
    allowed_roots: &[&Path],
    reader: &mut dyn std::io::Read,
) -> std::io::Result<u64> {
    use std::os::unix::io::AsRawFd;
    use std::path::Component;

    let deny = |m: &'static str| std::io::Error::new(std::io::ErrorKind::PermissionDenied, m);

    let parent = match dest_path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => return Err(deny("hydrate destination has no parent directory")),
    };
    let leaf = dest_path
        .file_name()
        .ok_or_else(|| deny("hydrate destination has no file name"))?;

    // Pick the allowed root that is a lexical prefix of the parent, and the
    // component chain from that root to the parent. This is a pure string
    // operation (no filesystem access), so there is nothing to race here — the
    // openat descent below is the real, atomic enforcement.
    let (root, rel) = allowed_roots
        .iter()
        .find_map(|&root| parent.strip_prefix(root).ok().map(|rel| (root, rel)))
        .ok_or_else(|| deny("hydrate destination is not within an allowed root"))?;

    // Every relative component must be a plain name — reject `.`/`..`/root/prefix
    // (defense in depth; a legit dest built by root.join(rel) never has these).
    let mut components: Vec<&std::ffi::OsStr> = Vec::new();
    for comp in rel.components() {
        match comp {
            Component::Normal(c) => components.push(c),
            _ => return Err(deny("hydrate destination has a non-normal path component")),
        }
    }

    // Descend from the trusted root to the immediate parent, one component at a
    // time, entirely via fd-relative opens. Each `OwnedFd` reassignment drops the
    // previous one.
    let mut dir_fd = open_dir_no_follow(root)?;
    for comp in components {
        dir_fd = open_dir_relative_no_follow(dir_fd.as_raw_fd(), comp)?;
    }

    // Refuse (without creating or writing anything) if the leaf currently
    // exists as a symlink — same ELOOP-producing O_NOFOLLOW semantics the
    // previous direct-open implementation used, reproduced here via a
    // read-only probe open so the observable failure mode for
    // `write_hydrated_plaintext_refuses_symlink_and_sets_0600` is unchanged.
    // ENOENT (no existing leaf — the common, fresh-hydrate case) is fine; the
    // rename below creates it. Any other pre-existing entry (a regular file)
    // is also fine — it gets atomically replaced below, same as before.
    let leaf_c = path_to_cstring(leaf)?;
    // SAFETY: `dir_fd` is a live, already-validated directory fd; `leaf_c` is
    // NUL-terminated. We only inspect the syscall's return value / errno.
    let probe = unsafe {
        libc::openat(
            dir_fd.as_raw_fd(),
            leaf_c.as_ptr(),
            libc::O_NOFOLLOW | libc::O_RDONLY | libc::O_CLOEXEC,
        )
    };
    if probe < 0 {
        let err = std::io::Error::last_os_error();
        if err.raw_os_error() != Some(libc::ENOENT) {
            return Err(err);
        }
    } else {
        // SAFETY: `probe` is a valid fd just returned by the openat above.
        unsafe { libc::close(probe) };
    }

    // Stage under a per-call, randomly-suffixed temp name in the same
    // directory (task 1670 round 2 — see the fn doc comment).
    let temp_leaf_name = format!(".{}.{}.part", leaf.to_string_lossy(), uuid::Uuid::new_v4());
    let temp_leaf = std::ffi::OsStr::new(&temp_leaf_name);
    let temp_leaf_c = path_to_cstring(temp_leaf)?;
    let remove_temp = || {
        // SAFETY: `dir_fd` is still live (we only ever drop it by falling out
        // of this function); `temp_leaf_c` is NUL-terminated and owned for the
        // duration of this closure's use. Best-effort — errors are ignored,
        // matching every other cleanup-on-failure path in this module.
        let _ = unsafe { libc::unlinkat(dir_fd.as_raw_fd(), temp_leaf_c.as_ptr(), 0) };
    };

    // Create/open the temp leaf relative to the anchored parent fd, refusing a
    // symlink at the leaf itself (it never pre-exists — the name is fresh —
    // but O_NOFOLLOW costs nothing and matches the invariant every other leaf
    // open in this function keeps), then force owner-only perms on the fd
    // unconditionally (before any plaintext is written).
    let file_fd = create_leaf_relative(dir_fd.as_raw_fd(), temp_leaf)?;
    if unsafe { libc::fchmod(file_fd.as_raw_fd(), 0o600 as libc::mode_t) } != 0 {
        let err = std::io::Error::last_os_error();
        remove_temp();
        return Err(err);
    }

    let written = match std::io::copy(reader, &mut std::fs::File::from(file_fd)) {
        Ok(written) => written,
        Err(e) => {
            remove_temp();
            return Err(e);
        }
    };

    // Atomically publish: same `dir_fd` for both sides, so this is anchored to
    // the already-validated directory inode, not a fresh path resolution.
    // SAFETY: both name arguments are NUL-terminated `CString`s alive for this
    // call; `dir_fd` is a live, already-validated directory fd.
    let rc = unsafe {
        libc::renameat(
            dir_fd.as_raw_fd(),
            temp_leaf_c.as_ptr(),
            dir_fd.as_raw_fd(),
            leaf_c.as_ptr(),
        )
    };
    if rc != 0 {
        let err = std::io::Error::last_os_error();
        remove_temp();
        return Err(err);
    }

    Ok(written)
}

#[cfg(not(unix))]
pub(crate) fn write_hydrated_from_reader(
    dest_path: &Path,
    _allowed_roots: &[&Path],
    reader: &mut dyn std::io::Read,
) -> std::io::Result<u64> {
    std::io::copy(reader, &mut std::fs::File::create(dest_path)?)
}

/// Open `dir` as a directory fd, refusing to follow a symlink at its final
/// component. Used only for the trusted allowed-root itself.
#[cfg(unix)]
fn open_dir_no_follow(dir: &Path) -> std::io::Result<std::os::unix::io::OwnedFd> {
    use std::os::unix::io::FromRawFd;
    let c = path_to_cstring(dir.as_os_str())?;
    let raw = unsafe { libc::open(c.as_ptr(), libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC) };
    if raw < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { std::os::unix::io::OwnedFd::from_raw_fd(raw) })
}

/// Open a single child directory `component` relative to `dir_fd`, refusing to
/// follow a symlink. This is the anchored descent step: because the open is
/// relative to an already-open dir fd, a rename of `component` racing this call
/// cannot redirect it outside the subtree, and once returned the fd tracks that
/// exact inode regardless of later renames. `component` must be a single normal
/// name (callers pass `Component::Normal` only); reject the obvious escapes as
/// belt-and-braces.
#[cfg(unix)]
fn open_dir_relative_no_follow(
    dir_fd: std::os::unix::io::RawFd,
    component: &std::ffi::OsStr,
) -> std::io::Result<std::os::unix::io::OwnedFd> {
    use std::os::unix::io::FromRawFd;
    if component.is_empty() || component == std::ffi::OsStr::new(".") || component == std::ffi::OsStr::new("..") {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "unsafe path component in hydrate destination",
        ));
    }
    let c = path_to_cstring(component)?;
    let raw = unsafe {
        libc::openat(
            dir_fd,
            c.as_ptr(),
            libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if raw < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { std::os::unix::io::OwnedFd::from_raw_fd(raw) })
}

/// Create/open the leaf file `leaf` relative to the anchored parent `dir_fd`
/// (O_NOFOLLOW|O_CREAT|O_WRONLY|O_TRUNC). Refuses a symlink at the leaf.
#[cfg(unix)]
fn create_leaf_relative(
    dir_fd: std::os::unix::io::RawFd,
    leaf: &std::ffi::OsStr,
) -> std::io::Result<std::os::unix::io::OwnedFd> {
    use std::os::unix::io::FromRawFd;
    let c = path_to_cstring(leaf)?;
    let raw = unsafe {
        libc::openat(
            dir_fd,
            c.as_ptr(),
            libc::O_NOFOLLOW | libc::O_CREAT | libc::O_WRONLY | libc::O_TRUNC | libc::O_CLOEXEC,
            0o600 as libc::c_int,
        )
    };
    if raw < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { std::os::unix::io::OwnedFd::from_raw_fd(raw) })
}

/// Convert an `OsStr` path component to a NUL-terminated `CString` for the raw
/// `libc::open`/`openat` calls in `write_hydrated_plaintext`.
#[cfg(unix)]
fn path_to_cstring(s: &std::ffi::OsStr) -> std::io::Result<std::ffi::CString> {
    use std::os::unix::ffi::OsStrExt;
    std::ffi::CString::new(s.as_bytes())
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "path contains an interior NUL byte"))
}

#[cfg(target_os = "linux")]
fn linux_thumbnail_source_path_for_entry(entry: &FileEntry) -> Option<PathBuf> {
    let sync_root = crate::config::DesktopConfig::load().ok()?.sync_root?;
    crate::linux_thumbnail::source_path_under_sync_root(&sync_root, &entry.path)
}

/// The daemon's own copy of a Finder write's contents, in `root`: from the
/// opened file when the caller holds one (only `source` is journaled then, as
/// a label), else from `source` by path.
fn stage_finder_contents(
    db: &Arc<StateDb>,
    root: PathBuf,
    source: &Path,
    opened: Option<&std::fs::File>,
) -> anyhow::Result<crate::staged_payload::StagedPayload> {
    match opened {
        Some(file) => crate::staged_payload::StagedPayload::copy_from_file(db.clone(), file, source, root),
        None => crate::staged_payload::StagedPayload::copy(db.clone(), source, root),
    }
}

/// The folders a Finder write's plaintext copy can be staged in, from the bases this process
/// resolves: the app's data dir, its cache dir and the temp dir. Paths only: nothing is created
/// or touched until a root is chosen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FinderStagingBases {
    pub(crate) data: Option<PathBuf>,
    pub(crate) cache: Option<PathBuf>,
    pub(crate) temp: PathBuf,
}

impl FinderStagingBases {
    /// This process's bases: the OS's data, cache and temp dirs.
    #[cfg(not(test))]
    pub(crate) fn current() -> Self {
        Self {
            data: dirs::data_dir(),
            cache: dirs::cache_dir(),
            temp: std::env::temp_dir(),
        }
    }

    /// A test build stages under a per-process sandbox, never the person's real data or cache
    /// dir: the installed app stages there too, and every test that queues a Finder write
    /// (engine bridge, IPC socket, watcher) would otherwise add plaintext files to it. A test
    /// points one bridge at bases of its own with `Seams::stage_under`. Pinned by
    /// `unit_tests_stage_finder_writes_in_a_sandbox_never_the_real_cache`.
    #[cfg(test)]
    pub(crate) fn current() -> Self {
        Self {
            data: Some(crate::test_sandbox::dir("data").expect("create the unit-test sandbox")),
            cache: Some(crate::test_sandbox::dir("cache").expect("create the unit-test sandbox")),
            temp: std::env::temp_dir(),
        }
    }

    /// `<data dir>/beebeeb/finder-writes`, where macOS stages: a folder the system does not
    /// purge. `None` when the OS gives no data dir.
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    pub(crate) fn durable_root(&self) -> Option<PathBuf> {
        self.data
            .as_ref()
            .map(|data| data.join("beebeeb").join("finder-writes"))
    }

    /// `<cache dir>/beebeeb/finder-writes` (the temp dir when the OS gives no cache dir): where
    /// every platform staged before, and where Windows and Linux still stage.
    pub(crate) fn cache_root(&self) -> PathBuf {
        self.cache
            .as_ref()
            .unwrap_or(&self.temp)
            .join("beebeeb")
            .join("finder-writes")
    }

    /// `<temp dir>/beebeeb/finder-writes`: the fallback when the cache root is not writable
    /// (Windows and Linux; macOS used it before as well).
    pub(crate) fn temp_root(&self) -> PathBuf {
        self.temp.join("beebeeb").join("finder-writes")
    }

    /// Every folder a staged copy can be in. On macOS: the data root, then the cache root and the
    /// temp root, which builds before the move staged into, so their copies are still found. Elsewhere:
    /// the cache root and the temp root.
    pub(crate) fn candidates(&self) -> Vec<PathBuf> {
        let mut candidates = Vec::with_capacity(3);
        #[cfg(target_os = "macos")]
        candidates.extend(self.durable_root());
        candidates.extend([self.cache_root(), self.temp_root()]);
        candidates
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

/// Where this process stages a Finder write's plaintext copy now.
fn default_finder_staging_root() -> Result<PathBuf, FinderStagingUnavailable> {
    finder_staging_root_from(&FinderStagingBases::current())
}

/// The staging root `bases` give on this platform.
fn finder_staging_root_from(bases: &FinderStagingBases) -> Result<PathBuf, FinderStagingUnavailable> {
    #[cfg(target_os = "macos")]
    {
        durable_finder_staging_root(bases)
    }
    #[cfg(not(target_os = "macos"))]
    {
        Ok(cache_finder_staging_root(bases))
    }
}

/// macOS: the staged copy can be the only copy of a save until it uploads (once the extension
/// has handed the write over, the system counts the item as synced and may evict its bytes), so
/// it lives in the app's data dir (`Library/Application Support`), never in the cache or temp
/// dir, which the system may purge. The folder is private (`0700`, excluded from backups: it
/// holds plaintext) and must take a file. When it cannot be created or written to, the accept
/// fails with [`FinderStagingUnavailable`], and nothing falls back.
#[cfg(any(target_os = "macos", test))]
fn durable_finder_staging_root(bases: &FinderStagingBases) -> Result<PathBuf, FinderStagingUnavailable> {
    let root = bases.durable_root().ok_or(FinderStagingUnavailable(None))?;
    let unavailable = |e: std::io::Error| FinderStagingUnavailable(Some(e.kind()));
    prepare_private_staging_root(&root).map_err(unavailable)?;
    verify_staging_root_writable(&root).map_err(unavailable)?;
    Ok(root)
}

/// Create (if needed) and harden the staging root: owner-only `0700`, excluded from backups,
/// a real directory this user owns, never a symlink (the App Group staging folders' hardening).
#[cfg(target_os = "macos")]
fn prepare_private_staging_root(root: &Path) -> std::io::Result<()> {
    crate::ipc_socket::macos_ensure_private_staging_dir(root)
}

/// Test builds off macOS: the same `0700` folder (there is no backup exclusion to set).
#[cfg(all(unix, test, not(target_os = "macos")))]
fn prepare_private_staging_root(root: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(root)?;
    std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o700))
}

/// Test builds on Windows: the folder only.
#[cfg(all(windows, test))]
fn prepare_private_staging_root(root: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(root)
}

/// Windows and Linux: the cache root, or the temp root when the cache root is not writable.
#[cfg(not(target_os = "macos"))]
fn cache_finder_staging_root(bases: &FinderStagingBases) -> PathBuf {
    let primary = bases.cache_root();
    match verify_staging_root_writable(&primary) {
        Ok(()) => primary,
        Err(e) => {
            let fallback = bases.temp_root();
            tracing::warn!(
                path = %primary.display(),
                fallback = %fallback.display(),
                error = %e,
                "Finder staging cache root is unavailable; falling back to temp directory"
            );
            fallback
        }
    }
}

/// Where Finder-write plaintext copies are staged, and were staged by earlier builds
/// ([`FinderStagingBases::candidates`]). The account reset and the sign-out purge sweep every one, so a copy no
/// row points at any more does not outlive the account, and the sign-out purge's allow-list accepts every one.
/// Paths only: nothing is created or touched here.
pub(crate) fn finder_staging_candidates() -> Vec<PathBuf> {
    FinderStagingBases::current().candidates()
}

fn verify_staging_root_writable(root: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(root)?;
    let probe = root.join(format!(".beebeeb-write-test-{}", uuid::Uuid::new_v4()));
    std::fs::write(&probe, b"")?;
    let _ = std::fs::remove_file(probe);
    Ok(())
}

/// The content version the File Provider extension holds for an item
/// (`docs/IPC_PROTOCOL.md`, "Version identifiers"): the server's version
/// number, then the content hash when the row has one.
///
/// The extension hands it back as the base of its next write, and
/// [`parse_base_version_number`] reads the first segment as the
/// `base_version_number` the server compares with the file's current version.
/// It never carries a wall-clock value: a re-stamp of the row without a new
/// server version (the `/sync/ops` echo of an upload, a finished upload's
/// `remote_updated_at`) is not a content change, and must neither force a
/// re-download nor turn into a base the server refuses.
pub(crate) fn item_content_version(current_version: i64, content_hash: Option<&str>) -> String {
    match content_hash {
        Some(hash) => format!("{current_version}:{hash}"),
        None => current_version.to_string(),
    }
}

/// The item's full version identifier: server version, then modification
/// time and size. Older extensions fall back to it when the payload has no
/// content version, so it leads with the server version too.
pub(crate) fn item_version_identifier(current_version: i64, modified_at: i64, size_bytes: i64) -> String {
    format!("{current_version}:{modified_at}:{size_bytes}")
}

/// The server version a Finder content modify is based on, from the content
/// version the system held when the user saved.
///
/// Builds before the server-version-led format told the system identifiers
/// led by `max(current_version, remote_updated_at)`: a wall-clock second once
/// the row had been re-stamped. The system keeps such an identifier for an
/// item until it reads the item again, and sends it as the base of the next
/// write (also when it re-sends a write that failed earlier). Parsed as is,
/// that base is a timestamp the server refuses as stale.
///
/// Rule: an identifier that is EXACTLY what the old formula gives for the row
/// as it is now ([`legacy_item_identifiers`]: the old content version, with
/// the row's hash when it has one, or the old full identifier) describes the
/// row's current content, so it is based on `current_version`. Every other
/// identifier is parsed as before: its first segment is the base, and a base
/// the server does not hold is refused as stale. An old identifier for older
/// content never equals the row's current one: content changes move
/// `current_version`, and every re-stamp moves `remote_updated_at`.
fn modify_base_version(
    version_identifier: Option<&str>,
    current: Option<(&FileEntry, &FileContractState)>,
) -> Option<i64> {
    if let (Some(identifier), Some((entry, contract))) = (version_identifier, current)
        && legacy_item_identifiers(entry, contract)
            .iter()
            .any(|legacy| legacy == identifier)
    {
        return Some(contract.current_version).filter(|version| *version > 0);
    }
    parse_base_version_number(version_identifier)
}

/// The identifiers a build before the server-version-led format reported for
/// this row: the content version and the full version identifier, both led by
/// `max(current_version, remote_updated_at)`. Used only to recognise them.
fn legacy_item_identifiers(entry: &FileEntry, contract: &FileContractState) -> [String; 2] {
    let leading = contract.current_version.max(entry.remote_updated_at);
    [
        item_content_version(leading, entry.content_hash.as_deref()),
        item_version_identifier(leading, entry.modified_at, entry.size_bytes),
    ]
}

pub(crate) fn parse_base_version_number(version_identifier: Option<&str>) -> Option<i64> {
    version_identifier.and_then(|value| {
        value
            .split(':')
            .next()
            .and_then(|part| part.parse::<i64>().ok())
            .filter(|version| *version > 0)
    })
}

fn display_name_for_path(path: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(path)
        .to_string()
}

fn record_moved_to_trash_activity(db: &StateDb, file_id: &str, rel_path: &str, occurred_at: i64) -> anyhow::Result<()> {
    db.record_local_activity(LocalActivityEventInput {
        event_type: LocalActivityKind::MovedToTrash,
        file_id: Some(file_id.to_string()),
        file_name: display_name_for_path(rel_path),
        rel_path: Some(rel_path.to_string()),
        occurred_at,
    })?;
    Ok(())
}

fn review_entry_for_operation(op: &PendingOperation, db: &StateDb) -> anyhow::Result<VersionConflictEntry> {
    let file_id = op.file_id.clone().unwrap_or_else(|| op.op_id.clone());
    let file_name = op
        .target_path
        .as_deref()
        .map(display_name_for_path)
        .or_else(|| {
            op.file_id
                .as_deref()
                .and_then(|id| db.get_file(id).ok().flatten())
                .map(|entry| display_name_for_path(&entry.path))
        })
        .unwrap_or_else(|| file_id.clone());
    let (kind, status, detail, action) = classify_review_operation(op);

    Ok(VersionConflictEntry {
        id: format!("op:{}", op.op_id),
        file_id,
        file_name,
        kind: kind.to_string(),
        status: status.to_string(),
        updated_at: Some(op.updated_at),
        detail,
        action: action.to_string(),
        op_id: Some(op.op_id.clone()),
        version_id: operation_version_id(op),
        base_version: op.base_version,
        last_error: op.last_error.clone(),
    })
}

fn classify_review_operation(op: &PendingOperation) -> (&'static str, &'static str, String, &'static str) {
    let raw_error = op.last_error.as_deref().unwrap_or("");
    let error = raw_error.to_ascii_lowercase();
    let metadata = op.metadata_json.as_deref().unwrap_or("").to_ascii_lowercase();
    let stale_base = error.contains("stale")
        || error.contains("base version")
        || metadata.contains("base_version_identifier")
        || op.base_version.is_some();

    if matches!(op.kind, OperationKind::RestoreVersion) {
        return (
            "restore",
            "restore review",
            "Restore is queued or failed; the server restore endpoint creates a new current version when it succeeds."
                .to_string(),
            "restore_review",
        );
    }

    // Auth failures take priority over quota/permission/stale-base: an
    // expired session's 401 unblocks nothing until the user signs in again,
    // and "sign in again" is a strictly more actionable message than "your
    // storage is full" or "permission denied" would be for the same root
    // cause. Reuses `classify_operation_error`'s stripped-URL substring
    // matching (task 1252 — a reqwest error's `" for url (…)"` suffix can
    // embed a port number containing "401") instead of re-deriving a second,
    // looser copy of the same check here (task 1546 finding 3).
    if matches!(classify_operation_error(raw_error), OperationFailureClass::Auth) {
        return (
            "auth_failure",
            "sign-in needed",
            "Your session has expired. Sign in again to resume syncing.".to_string(),
            "sign_in_again",
        );
    }
    if error.contains("quota") || error.contains("insufficient storage") {
        return (
            "quota_failure",
            "quota blocked",
            op.last_error
                .clone()
                .unwrap_or_else(|| "Upload is blocked by account storage quota.".to_string()),
            "review_upload",
        );
    }
    if error.contains("permission")
        || error.contains("forbidden")
        || error.contains("read-only")
        || error.contains("403")
    {
        return (
            "permission_failure",
            "permission blocked",
            op.last_error
                .clone()
                .unwrap_or_else(|| "Write is blocked by folder permissions.".to_string()),
            "review_upload",
        );
    }
    if stale_base && matches!(op.kind, OperationKind::UploadVersion) {
        return (
            "stale_base",
            "stale base kept local",
            "Local bytes are preserved in the durable queue and need version review before retry.".to_string(),
            "review_upload",
        );
    }

    match op.kind {
        OperationKind::UploadVersion | OperationKind::UploadFile => (
            "failed_upload",
            "upload review",
            op.last_error
                .clone()
                .unwrap_or_else(|| "Upload is queued for the encrypted transfer worker.".to_string()),
            "review_upload",
        ),
        OperationKind::RenameFile | OperationKind::MoveFile => (
            "metadata",
            "metadata review",
            op.last_error
                .clone()
                .unwrap_or_else(|| "Rename or move is queued for metadata sync.".to_string()),
            "review_upload",
        ),
        OperationKind::TrashFile => (
            "delete",
            "delete review",
            op.last_error
                .clone()
                .unwrap_or_else(|| "Delete is queued as a trash/soft-delete operation.".to_string()),
            "review_upload",
        ),
        _ => (
            "failed_upload",
            "queued review",
            op.last_error
                .clone()
                .unwrap_or_else(|| "Operation is queued for a worker that is not attached yet.".to_string()),
            "review_upload",
        ),
    }
}

fn operation_version_id(op: &PendingOperation) -> Option<String> {
    op.base_object_version_id.clone().or_else(|| {
        op.metadata_json
            .as_deref()
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
            .and_then(|value| value.get("version_id").and_then(|v| v.as_str()).map(str::to_string))
    })
}

fn operation_metadata(op: &PendingOperation) -> anyhow::Result<serde_json::Value> {
    Ok(op
        .metadata_json
        .as_deref()
        .map(serde_json::from_str::<serde_json::Value>)
        .transpose()?
        .unwrap_or_else(|| serde_json::json!({})))
}

pub fn classify_operation_error(error: &str) -> OperationFailureClass {
    // Classify on the failure MESSAGE only, never on the request URL. reqwest's
    // Display appends `" for url (…)"` (for both HTTP-status and connection
    // errors), and that URL carries the random ephemeral port plus opaque path
    // ids. Those digits are not status codes, but the bare-code substring checks
    // below ("401", "403") would happily match them — so a plain HTTP 500 sent
    // to, say, `127.0.0.1:45401` was being misread as an Auth failure and
    // *paused* (never retried). That is the entire flake in task 1252: it fired
    // only when the OS handed the mock server a port whose digits contained
    // "401"/"403", which is why it reproduced in CI but ~never locally. Dropping
    // the URL suffix keeps classification keyed on reqwest's status reason phrase
    // ("401 Unauthorized", "403 Forbidden", "507 Insufficient Storage", …) and
    // on our own descriptive errors, none of which live past `" for url ("`.
    let message = error.split(" for url (").next().unwrap_or(error);
    let lower = message.to_ascii_lowercase();
    if lower.contains("401") || lower.contains("unauthorized") || lower.contains("invalid token") {
        OperationFailureClass::Auth
    } else if lower.contains("quota") || lower.contains("insufficient storage") || lower.contains("storage limit") {
        OperationFailureClass::Quota
    } else if lower.contains("403")
        || lower.contains("forbidden")
        || lower.contains("permission")
        || lower.contains("read-only")
    {
        OperationFailureClass::Permission
    } else if lower.contains("vault locked") || lower.contains("locked") || lower.contains("unlock") {
        OperationFailureClass::Locked
    } else {
        OperationFailureClass::Retryable
    }
}

pub fn retry_delay_seconds(attempts: i64) -> i64 {
    let exponent = attempts.clamp(1, 6) as u32;
    30_i64.saturating_mul(2_i64.saturating_pow(exponent - 1))
}

fn shared_roots_from_invite_response(body: &serde_json::Value) -> Vec<SharedRootMapping> {
    body.get("invites")
        .and_then(|value| value.as_array())
        .into_iter()
        .flatten()
        .filter_map(shared_root_from_invite)
        .collect()
}

fn shared_invite_id_matches(invite: &serde_json::Value, share_id: &str) -> bool {
    string_field(invite, &["id", "invite_id"]).as_deref() == Some(share_id)
}

fn shared_root_from_invite(invite: &serde_json::Value) -> Option<SharedRootMapping> {
    if invite.get("status").and_then(|value| value.as_str()) != Some("approved") {
        return None;
    }

    let invite_id = string_field(invite, &["id", "invite_id"])?;
    let file_id = string_field(invite, &["file_id"])?;
    let is_folder = invite
        .get("is_folder_share")
        .and_then(|value| value.as_bool())
        .or_else(|| invite.get("is_folder").and_then(|value| value.as_bool()))
        .unwrap_or(false);
    let item_kind = if is_folder { ItemKind::Folder } else { ItemKind::File };
    let display_name = shared_placeholder_name(&item_kind).to_string();
    let content_type = string_field(invite, &["mime_type", "content_type"]);
    let size_bytes = invite.get("size_bytes").and_then(|value| value.as_i64()).unwrap_or(0);
    let mut permission_bits = PERMISSION_READ;
    if invite
        .get("can_reshare")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
    {
        permission_bits |= PERMISSION_SHARE;
    }
    if shared_invite_allows_write(invite) {
        permission_bits |= PERMISSION_WRITE;
    }

    Some(SharedRootMapping {
        invite_id,
        file_id,
        display_name,
        is_folder,
        size_bytes,
        content_type,
        owner_email: string_field(invite, &["sender_email", "owner_email", "shared_by"]),
        sender_public_key: string_field(invite, &["sender_public_key", "owner_public_key"]),
        encrypted_file_key: string_field(invite, &["encrypted_file_key", "wrapped_file_key"]),
        encrypted_folder_key: string_field(invite, &["encrypted_folder_key"]),
        file_name_encrypted: string_field(invite, &["file_name_encrypted", "name_encrypted"]),
        permission_bits,
        approved_at: invite
            .get("approved_at")
            .and_then(|value| value.as_str())
            .and_then(parse_rfc3339_secs),
    })
}

fn shared_invite_allows_write(invite: &serde_json::Value) -> bool {
    if ["can_write", "can_edit", "editable"]
        .iter()
        .any(|key| invite.get(*key).and_then(|value| value.as_bool()).unwrap_or(false))
    {
        return true;
    }
    ["permission", "capability", "permissions", "role", "access"]
        .iter()
        .filter_map(|key| invite.get(*key).and_then(|value| value.as_str()))
        .any(|value| {
            matches!(
                value.to_ascii_lowercase().as_str(),
                "write" | "edit" | "editable" | "editor" | "admin" | "owner"
            )
        })
}

fn decode_standard_b64(label: &str, value: &str) -> anyhow::Result<Vec<u8>> {
    base64::engine::general_purpose::STANDARD
        .decode(value)
        .map_err(|e| anyhow::anyhow!("{label} is not valid standard base64: {e}"))
}

fn decode_x25519_public_key(label: &str, value: &str) -> anyhow::Result<[u8; 32]> {
    let bytes = decode_standard_b64(label, value)?;
    if bytes.len() != 32 {
        anyhow::bail!("{label} must be 32 bytes, got {}", bytes.len());
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&bytes);
    Ok(out)
}

fn derive_recipient_share_key(
    recipient_master_key: &[u8; 32],
    sender_public_key_b64: &str,
    file_id: &str,
) -> anyhow::Result<Zeroizing<[u8; 32]>> {
    let master_key = beebeeb_core::kdf::MasterKey::from_bytes(*recipient_master_key);
    let recipient_private = beebeeb_core::opaque::derive_x25519_private(&master_key);
    let sender_public = decode_x25519_public_key("sender_public_key", sender_public_key_b64)?;
    let shared_secret = beebeeb_core::opaque::x25519_shared_secret(&recipient_private, &sender_public)
        .map_err(|e| anyhow::anyhow!("derive X25519 shared secret: {e}"))?;
    Ok(beebeeb_core::opaque::derive_share_key(
        &shared_secret,
        file_id.as_bytes(),
    ))
}

fn unwrap_key_frame(wrap_key: &[u8; 32], encrypted_key_b64: &str, label: &str) -> anyhow::Result<Zeroizing<[u8; 32]>> {
    let raw = decode_standard_b64(label, encrypted_key_b64)?;
    let frame_key = beebeeb_core::kdf::FileKey::from_bytes(*wrap_key);
    let plaintext = Zeroizing::new(
        beebeeb_core::encrypt::decrypt_chunk_raw(&frame_key, &raw)
            .map_err(|_| anyhow::anyhow!("{label} decrypt failed"))?,
    );
    if plaintext.len() != 32 {
        let len = plaintext.len();
        anyhow::bail!("{label} decrypted key must be 32 bytes, got {len}");
    }
    let mut out = Zeroizing::new([0u8; 32]);
    out.copy_from_slice(&plaintext);
    Ok(out)
}

fn unwrap_direct_shared_file_key(
    recipient_master_key: &[u8; 32],
    sender_public_key_b64: &str,
    file_id: &str,
    encrypted_file_key_b64: &str,
) -> anyhow::Result<beebeeb_core::kdf::FileKey> {
    let share_key = derive_recipient_share_key(recipient_master_key, sender_public_key_b64, file_id)?;
    let file_key = unwrap_key_frame(&share_key, encrypted_file_key_b64, "encrypted_file_key")?;
    Ok(beebeeb_core::kdf::FileKey::from_bytes(*file_key))
}

fn unwrap_folder_share_key(
    recipient_master_key: &[u8; 32],
    sender_public_key_b64: &str,
    folder_id: &str,
    encrypted_folder_key_b64: &str,
) -> anyhow::Result<Zeroizing<[u8; 32]>> {
    let share_key = derive_recipient_share_key(recipient_master_key, sender_public_key_b64, folder_id)?;
    unwrap_key_frame(&share_key, encrypted_folder_key_b64, "encrypted_folder_key")
}

fn encrypted_file_key_from_folder_keys_response<'a>(
    folder_keys_response: &'a serde_json::Value,
    file_id: &str,
) -> Option<&'a str> {
    folder_keys_response
        .get("keys")
        .and_then(|value| value.as_array())
        .into_iter()
        .flatten()
        .find(|entry| entry.get("file_id").and_then(|value| value.as_str()) == Some(file_id))
        .and_then(|entry| entry.get("encrypted_file_key").and_then(|value| value.as_str()))
}

fn unwrap_child_file_key(
    folder_key: &[u8; 32],
    encrypted_file_key_b64: &str,
) -> anyhow::Result<beebeeb_core::kdf::FileKey> {
    let file_key = unwrap_key_frame(folder_key, encrypted_file_key_b64, "encrypted_file_key")?;
    Ok(beebeeb_core::kdf::FileKey::from_bytes(*file_key))
}

fn unwrap_folder_share_file_key(
    recipient_master_key: &[u8; 32],
    sender_public_key_b64: &str,
    folder_id: &str,
    encrypted_folder_key_b64: &str,
    file_id: &str,
    folder_keys_response: &serde_json::Value,
) -> anyhow::Result<beebeeb_core::kdf::FileKey> {
    let folder_key = unwrap_folder_share_key(
        recipient_master_key,
        sender_public_key_b64,
        folder_id,
        encrypted_folder_key_b64,
    )?;
    let encrypted_file_key = encrypted_file_key_from_folder_keys_response(folder_keys_response, file_id)
        .ok_or_else(|| anyhow::anyhow!("folder share is missing encrypted_file_key for {file_id}"))?;
    unwrap_child_file_key(&folder_key, encrypted_file_key)
}

fn folder_keys_map(folder_keys_response: &serde_json::Value) -> HashMap<String, String> {
    folder_keys_response
        .get("keys")
        .and_then(|value| value.as_array())
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            Some((
                entry.get("file_id")?.as_str()?.to_string(),
                entry.get("encrypted_file_key")?.as_str()?.to_string(),
            ))
        })
        .collect()
}

fn json_timestamp_secs(value: &serde_json::Value, keys: &[&str]) -> Option<i64> {
    keys.iter().find_map(|key| {
        let field = value.get(*key)?;
        field.as_i64().or_else(|| field.as_str().and_then(parse_rfc3339_secs))
    })
}

fn item_kind_from_metadata(f: &serde_json::Value) -> ItemKind {
    if f["is_folder"].as_bool().unwrap_or(false)
        || f["kind"].as_str() == Some("folder")
        || f["type"].as_str() == Some("folder")
    {
        ItemKind::Folder
    } else {
        ItemKind::File
    }
}

fn shared_placeholder_name(item_kind: &ItemKind) -> &'static str {
    match item_kind {
        ItemKind::Folder => "Encrypted folder",
        ItemKind::File => "Encrypted file",
    }
}

fn safe_shared_leaf_name(name: &str) -> Option<String> {
    let leaf = name.trim();
    if crate::reject_unsafe_rel_path(leaf).is_err() || leaf.contains('/') || leaf.contains('\\') {
        return None;
    }
    if leaf.is_empty() {
        return None;
    }
    Some(leaf.to_string())
}

fn metadata_name_from_plaintext(plaintext: &str) -> String {
    serde_json::from_str::<serde_json::Value>(plaintext)
        .ok()
        .and_then(|value| value.get("name").and_then(|name| name.as_str()).map(str::to_string))
        .unwrap_or_else(|| plaintext.to_string())
}

fn decrypt_shared_name_with_key(file_key: &beebeeb_core::kdf::FileKey, name_encrypted: &str) -> Option<String> {
    if let Ok(blob) = serde_json::from_str::<EncryptedBlob>(name_encrypted)
        && let Ok(plaintext) = beebeeb_core::encrypt::decrypt_metadata(file_key, &blob)
    {
        return Some(metadata_name_from_plaintext(&plaintext));
    }

    if let Ok((nonce, ciphertext)) = beebeeb_core::metadata_wire::parse_encrypted_metadata(name_encrypted) {
        let blob = EncryptedBlob {
            cipher_suite: CipherSuite::V1Aes256Gcm,
            nonce,
            ciphertext,
        };
        if let Ok(plaintext) = beebeeb_core::encrypt::decrypt_metadata(file_key, &blob) {
            return Some(metadata_name_from_plaintext(&plaintext));
        }
    }

    None
}

fn shared_name_from_metadata(
    f: &serde_json::Value,
    item_kind: &ItemKind,
    file_key: Option<&beebeeb_core::kdf::FileKey>,
) -> String {
    if let (Some(key), Some(name_encrypted)) = (file_key, f["name_encrypted"].as_str()) {
        if let Some(name) = decrypt_shared_name_with_key(key, name_encrypted) {
            if let Some(leaf) = safe_shared_leaf_name(&name) {
                return leaf;
            }
        }
    }

    shared_placeholder_name(item_kind).to_string()
}

fn apply_shared_metadata_file_row(
    db: &StateDb,
    f: &serde_json::Value,
    root: &SharedRootMapping,
    now: i64,
    parent_id: Option<String>,
    parent_rel_path: &str,
    file_key: Option<&beebeeb_core::kdf::FileKey>,
) -> anyhow::Result<Option<FileEntry>> {
    let file_id = f["id"].as_str().unwrap_or_default();
    if file_id.is_empty() {
        return Ok(None);
    }

    let existing = db.get_file(file_id)?;
    let size = f["size_bytes"].as_i64().or_else(|| f["size"].as_i64()).unwrap_or(0);
    let remote_updated = json_timestamp_secs(f, &["updated_at", "uploaded_at", "created_at"]).unwrap_or(now);
    let item_kind = item_kind_from_metadata(f);
    let leaf = shared_name_from_metadata(f, &item_kind, file_key);
    let path = if parent_rel_path.is_empty() {
        leaf
    } else {
        format!("{}/{}", parent_rel_path.trim_end_matches('/'), leaf)
    };
    crate::reject_unsafe_rel_path(&path).map_err(|e| anyhow::anyhow!("unsafe shared item path for {file_id}: {e}"))?;
    let status = existing
        .as_ref()
        .map(|entry| entry.status.clone())
        .unwrap_or(FileStatus::CloudOnly);

    let entry = FileEntry {
        file_id: file_id.to_string(),
        path,
        status,
        size_bytes: size,
        modified_at: remote_updated,
        content_hash: existing.as_ref().and_then(|entry| entry.content_hash.clone()),
        remote_updated_at: remote_updated,
        parent_id: parent_id.clone(),
        item_kind: item_kind.clone(),
    };
    db.upsert_file(&entry)?;

    let mut contract = db
        .get_file_contract_state(file_id)?
        .ok_or_else(|| anyhow::anyhow!("missing state row for shared item {file_id}"))?;
    contract.namespace = Namespace::SharedWithMe;
    contract.parent_id = parent_id;
    contract.shared_root_id = Some(root.file_id.clone());
    contract.share_id = Some(root.invite_id.clone());
    contract.owner_email = root.owner_email.clone();
    contract.permission_bits = root.permission_bits;
    contract.item_kind = item_kind;
    contract.content_type = f["content_type"]
        .as_str()
        .or_else(|| f["mime_type"].as_str())
        .map(str::to_string);
    contract.current_version = f["current_version"]
        .as_i64()
        .or_else(|| f["version"].as_i64())
        .or_else(|| f["version_number"].as_i64())
        .unwrap_or(contract.current_version);
    contract.current_object_version_id = f["current_object_version_id"]
        .as_str()
        .or_else(|| f["object_version_id"].as_str())
        .map(str::to_string)
        .or(contract.current_object_version_id);
    contract.last_sync_at = now;
    db.set_file_contract_state(&contract)?;
    Ok(Some(entry))
}

fn string_field(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .filter_map(|key| value.get(*key).and_then(|field| field.as_str()))
        .map(str::trim)
        .find(|field| !field.is_empty())
        .map(str::to_string)
}

fn parse_rfc3339_secs(value: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|dt| dt.timestamp())
}

fn apply_shared_context(metadata: &mut serde_json::Value, contract: Option<&FileContractState>) {
    let Some(contract) = contract else {
        return;
    };
    metadata["shared_root_id"] = serde_json::json!(contract.shared_root_id);
    metadata["share_id"] = serde_json::json!(contract.share_id);
    metadata["permission_bits"] = serde_json::json!(contract.permission_bits);
    metadata["uploaded_by"] = serde_json::json!("authenticated_desktop_user");
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Resolve the plaintext relative path for a server file row.
///
/// This is an **E2EE** product: the `GET /api/v1/files` listing returns the
/// filename only as the encrypted `name_encrypted` blob — there is NO
/// plaintext `path`/`display_path`/`name` field on the wire (confirmed in
/// `server/.../routes/files.rs::list_files`). The desktop must therefore
/// decrypt the name itself, with the unlocked master key, before it can
/// place a placeholder or compute a hydration destination.
///
/// Decryption goes through the **shared core primitive**
/// [`beebeeb_core::encrypt::decrypt_name`] — the exact same path the CLI
/// (`repos/cli/src/crypto::decrypt_name`) and the desktop SelectiveSync page
/// ([`crate::try_decrypt_name`]) use, so every client decrypts identically.
/// The per-file key is HKDF-derived in core (`derive_file_key(master_key,
/// file_id)`); the `MasterKey` zeroizes on drop. We never log the plaintext
/// name here.
///
/// `sync_tick` lists only the vault root (`list_files(None)` is one level —
/// the server endpoint is non-recursive), so a decrypted name IS the file's
/// relative path under the sync root. When nested traversal is added, this is
/// the single place that must compose `<parent rel path>/<name>`.
///
/// Fallback order (keeps legacy/test rows that DO carry a plaintext path
/// working, and degrades safely if a blob is missing/garbled):
///   1. decrypt `name_encrypted` (the canonical zero-knowledge path)
///   2. a plaintext `path`/`display_path`/`name` field if the server ever
///      surfaces one (older rows / test fixtures)
///   3. empty string — caller skips placeholder seeding for the row
fn resolve_relative_path(f: &serde_json::Value, file_id: &str, master_key: &[u8; 32]) -> String {
    if let Some(name_enc) = f["name_encrypted"].as_str() {
        let mk_bytes: [u8; 32] = *master_key;
        let mk = beebeeb_core::kdf::MasterKey::from_bytes(mk_bytes);
        if let Ok(name) = beebeeb_core::encrypt::decrypt_name(&mk, file_id, name_enc) {
            let name = name.trim();
            if !name.is_empty() {
                return name.to_string();
            }
        }
        // Decryption failed (wrong key, garbled blob, legacy envelope) —
        // fall through to any plaintext field rather than abort the sweep.
    }
    f["path"]
        .as_str()
        .or_else(|| f["display_path"].as_str())
        .or_else(|| f["name"].as_str())
        .unwrap_or("")
        .to_string()
}

fn apply_metadata_file_row(
    db: &StateDb,
    f: &serde_json::Value,
    namespace: Namespace,
    shared_root_id: Option<String>,
    share_id: Option<String>,
    permission_bits: i64,
    now: i64,
    master_key: &[u8; 32],
    // Server-relative path of the PARENT directory ("" at the vault root).
    // The decrypted leaf name is composed under it so nested files/folders
    // get a full `<parent>/<leaf>` path that round-trips with the upload
    // watcher's `relative_db_path` (also '/'-joined, no leading slash).
    parent_rel_path: &str,
) -> anyhow::Result<Option<FileEntry>> {
    let file_id = f["id"].as_str().unwrap_or_default();
    if file_id.is_empty() {
        return Ok(None);
    }

    let existing = db.get_file(file_id)?;
    let size = f["size_bytes"].as_i64().or_else(|| f["size"].as_i64()).unwrap_or(0);
    let remote_updated = f["updated_at"].as_i64().unwrap_or(0);
    let leaf = resolve_relative_path(f, file_id, master_key);
    // Compose the nested path. An empty leaf means the name didn't decrypt;
    // keep it empty so the placeholder seeder skips the row (and retries next
    // tick) rather than seeding a bare parent dir. A leaf that already carries
    // a leading slash (legacy plaintext-`path` fallback) is normalised.
    let path = if parent_rel_path.is_empty() || leaf.is_empty() {
        leaf
    } else {
        format!(
            "{}/{}",
            parent_rel_path.trim_end_matches('/'),
            leaf.trim_start_matches('/')
        )
    };
    let status = existing
        .as_ref()
        .map(|entry| entry.status.clone())
        .unwrap_or(FileStatus::CloudOnly);

    let item_kind = if f["is_folder"].as_bool().unwrap_or(false)
        || f["kind"].as_str() == Some("folder")
        || f["type"].as_str() == Some("folder")
    {
        ItemKind::Folder
    } else {
        ItemKind::File
    };
    let server_parent_id = f["parent_id"].as_str().map(str::to_string);

    let entry = FileEntry {
        file_id: file_id.to_string(),
        path,
        status,
        size_bytes: size,
        modified_at: remote_updated,
        content_hash: existing.as_ref().and_then(|entry| entry.content_hash.clone()),
        remote_updated_at: remote_updated,
        // NOTE: parent_id/item_kind are NOT persisted by upsert_file — the
        // `set_file_contract_state` call at the end of this fn writes the
        // authoritative `files.parent_id` / `files.item_kind` columns. These
        // struct fields exist so the in-memory `FileEntry` returned to callers
        // (notably the windows_cf placeholder seeder reading via list_by_status)
        // carries the correct folder/parent classification.
        parent_id: server_parent_id.clone(),
        item_kind: item_kind.clone(),
    };
    db.upsert_file(&entry)?;
    let mut contract = db
        .get_file_contract_state(file_id)?
        .unwrap_or_else(|| FileContractState {
            file_id: file_id.to_string(),
            namespace: namespace.clone(),
            parent_id: None,
            shared_root_id: shared_root_id.clone(),
            share_id: share_id.clone(),
            owner_email: None,
            permission_bits,
            item_kind: item_kind.clone(),
            content_type: None,
            current_version: 0,
            current_object_version_id: None,
            local_base_version: 0,
            local_hash: None,
            cache_path: None,
            cache_bytes: 0,
            pin_state: crate::state_db::PinState::Inherit,
            inherited_pin_state: crate::state_db::PinState::Unpinned,
            last_sync_at: now,
        });
    contract.namespace = namespace;
    contract.parent_id = server_parent_id;
    contract.shared_root_id = shared_root_id;
    contract.share_id = share_id;
    contract.permission_bits = permission_bits;
    contract.item_kind = item_kind;
    contract.content_type = f["content_type"]
        .as_str()
        .or_else(|| f["mime_type"].as_str())
        .map(str::to_string);
    contract.current_version = f["current_version"]
        .as_i64()
        .or_else(|| f["version"].as_i64())
        .or_else(|| f["version_number"].as_i64())
        .unwrap_or(contract.current_version);
    contract.current_object_version_id = f["current_object_version_id"]
        .as_str()
        .or_else(|| f["object_version_id"].as_str())
        .map(str::to_string)
        .or(contract.current_object_version_id);
    if entry.status == FileStatus::Local && contract.local_base_version == 0 {
        contract.local_base_version = contract.current_version;
    }
    contract.last_sync_at = now;
    db.set_file_contract_state(&contract)?;
    Ok(Some(entry))
}

// ── Periodic sync tick ────────────────────────────────────────────────────────

/// One file the latest tick noticed has diverged on both sides.
/// Returned from [`sync_tick`] so [`crate::runner`] can fan it out
/// to UI: open a conflict window + fire a notification.
#[derive(Debug, Clone)]
pub struct ConflictDetected {
    pub file_id: String,
    pub file_name: String,
    pub is_text: bool,
}

/// Pull the user's file list from the API, refresh the state DB, and
/// flag any newly-divergent files. Called from [`crate::runner`]'s
/// tick loop.
///
/// Three-way decision per remote file:
///
/// 1. **New to us** — no row exists. Insert as `cloud_only`. The OS
///    extension surfaces it as a placeholder; the user gets it on
///    demand.
///
/// 2. **Known and locally `Local`, remote moved** — the row exists,
///    its status is `Local`, and the server's `updated_at` is past
///    `remote_updated_at`. We use the
///    [`crate::conflict::is_conflict`] predicate to decide whether
///    that's a one-sided update we can quietly accept (no local edit
///    since base) or a divergent edit that needs the user. The
///    "hashes" we feed are synthetic right now: the server doesn't
///    expose a content hash on file metadata yet, so we compose
///    `<size>-<updated_at>` for remote and reuse the local
///    `content_hash` for local. False negatives (different bytes,
///    same size + mtime) are theoretically possible but rare; the
///    engine bridge will catch them on the next chunk diff once that
///    code lands.
///
/// 3. **Known and not in `Local`** — pending download/upload, etc.
///    Leave alone; the upload/download path owns those transitions.
/// Task 1697 review fix (T3): what one sync tick did. The remote ingestion
/// path (delta ops + snapshot bootstrap) records every item id it APPLIED so
/// the runner can signal the File Provider working set about REMOTE changes
/// — previously only local ops (`operations_applied`) signaled, so Finder
/// stayed blind to server-side creates/modifies/moves/deletes until an
/// unrelated signal.
#[derive(Debug, Default)]
pub struct SyncTickOutcome {
    pub conflicts: Vec<ConflictDetected>,
    pub applied_item_ids: Vec<String>,
}

pub async fn sync_tick(
    bridge: &EngineBridge,
    // See `sync_tick_outcome` for the `sync_root` contract.
    #[cfg_attr(not(target_os = "windows"), allow(unused_variables))] sync_root: &Path,
) -> anyhow::Result<Vec<ConflictDetected>> {
    Ok(sync_tick_outcome(bridge, sync_root).await?.conflicts)
}

/// The real tick body — same behavior as [`sync_tick`], plus the applied item
/// ids for the working-set signal (T3).
pub async fn sync_tick_outcome(
    bridge: &EngineBridge,
    // The on-disk vault root. Needed on Windows so the deletion-reconcile paths
    // (`apply_sync_op` / `apply_snapshot`) can locate and remove the on-disk Cloud
    // Files placeholder of a remotely-deleted row — not just its DB row (task
    // 0806). Unused on macOS/Linux (the OS extension owns the namespace there);
    // `#[cfg_attr]` silences the unused warning on those builds.
    #[cfg_attr(not(target_os = "windows"), allow(unused_variables))] sync_root: &Path,
) -> anyhow::Result<SyncTickOutcome> {
    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let mut conflicts: Vec<ConflictDetected> = Vec::new();
    let mut applied_item_ids: Vec<String> = Vec::new();

    // ── /sync delta engine (task 0789) ────────────────────────────────────────
    //
    // Replaces the old per-folder full-tree `/files` BFS re-walk (`1 + folders`
    // requests/tick, never pruned server deletions, self-saturated the per-IP
    // 429 limit). Two cheap paths, gated on whether the op cursor has EVER been
    // persisted (`Option`, NOT a 0 sentinel):
    //
    //   cursor UNSET (never bootstrapped) → BOOTSTRAP via ONE `GET /sync/snapshot`:
    //     the snapshot is the authoritative non-trashed tree, so we (a) apply
    //     every node through the SAME ingest path the BFS used
    //     (`process_metadata_row`, which keeps conflict detection identical +
    //     flows rows through `upsert_file` for Windows `refresh_placeholders`),
    //     (b) PRUNE every own-tree row the snapshot omits (the
    //     deletion-reconciliation fix), and (c) store the snapshot's `seq_id` as
    //     the cursor — which may legitimately be 0. The server returns
    //     `MAX(seq_id) … unwrap_or(0)` and `sync_ops` is NOT backfilled, so a
    //     pre-existing vault (or a brand-new one before its first create-op
    //     lands) snapshots at seq_id 0. seq_id 0 is therefore a VALID
    //     bootstrapped cursor, NOT an "unset" sentinel: gating on `== 0` would
    //     re-snapshot + re-prune the WHOLE tree every tick forever and never
    //     reach the cheap ops path (defeating the 429 fix).
    //
    //   cursor SET (incl. Some(0)) → CATCH-UP via ONE `GET /sync/ops?since={cursor}`:
    //     apply each op by `op_type` and advance the cursor to the max applied
    //     `seq_id`. With `since=0` the server returns every op `> 0`, which is
    //     exactly right for a just-bootstrapped empty/low op log. ~1
    //     request/tick in steady state.
    //
    // Gap / since-too-old fallback: if applying ops can't proceed coherently
    // (the cursor is ahead of the server, or an op references a row we can't
    // place), we re-bootstrap from a fresh snapshot — the snapshot is always
    // authoritative, so a re-bootstrap can never lose data.

    // A pending re-snapshot request (e.g. a `file_restore` op the previous tick
    // couldn't materialise from its `{id}`-only payload) forces a bootstrap this
    // tick regardless of the cursor. On macOS the request is read here and cleared
    // only after the bootstrap succeeded (spec §6.3.2, I-2(b)): a failed snapshot
    // keeps it, and a request made while the snapshot ran survives the clear.
    // Elsewhere the flag is taken here, once.
    #[cfg(target_os = "macos")]
    let resnapshot_request = {
        // §6.3.2: while any upload waits for its base, every pass asks for a snapshot.
        if bridge.db().has_base_pending_uploads()? {
            bridge.db().request_resnapshot()?;
        }
        bridge.db().peek_resnapshot_request()?
    };
    #[cfg(target_os = "macos")]
    let needs_resnapshot = resnapshot_request.is_some();
    #[cfg(not(target_os = "macos"))]
    let needs_resnapshot = bridge.db().take_needs_resnapshot()?;

    let cursor = match bridge.db().get_sync_cursor()? {
        // Never bootstrapped, or a re-snapshot was explicitly requested → full
        // snapshot. (Some(0) is a real cursor and does NOT bootstrap here.)
        None => {
            let applied = bootstrap_from_snapshot(bridge, sync_root, now_secs, &mut conflicts).await?;
            #[cfg(target_os = "macos")]
            if let Some(seen) = resnapshot_request {
                bridge.db().clear_resnapshot_request(seen)?;
            }
            applied_item_ids.extend(applied);
            return Ok(SyncTickOutcome {
                conflicts,
                applied_item_ids: dedupe(applied_item_ids),
            });
        }
        Some(_) if needs_resnapshot => {
            tracing::info!("sync_tick: re-snapshot requested (gap recovery); bootstrapping");
            let applied = bootstrap_from_snapshot(bridge, sync_root, now_secs, &mut conflicts).await?;
            #[cfg(target_os = "macos")]
            if let Some(seen) = resnapshot_request {
                bridge.db().clear_resnapshot_request(seen)?;
            }
            applied_item_ids.extend(applied);
            return Ok(SyncTickOutcome {
                conflicts,
                applied_item_ids: dedupe(applied_item_ids),
            });
        }
        Some(c) => c,
    };

    let ops = match bridge.api().sync_ops(cursor).await {
        Ok(ops) => ops,
        Err(e) => {
            // Transient list failure (network / lingering 429 after backoff): do
            // NOT advance the cursor and do NOT fall back to a (heavier)
            // snapshot — just skip this tick and retry next time. The cursor is
            // unchanged, so no op is missed.
            tracing::warn!(error = %e, cursor, "sync_tick: /sync/ops failed; retrying next tick");
            return Ok(SyncTickOutcome {
                conflicts,
                applied_item_ids: dedupe(applied_item_ids),
            });
        }
    };

    // Detect a cursor that is somehow AHEAD of the server's op log (e.g. a server
    // history reset / op-log truncation): the server clamps `since` to whatever
    // we sent and returns only `seq_id > since`, so a stale/over-large cursor
    // yields an empty op list forever and we'd never reconcile. We can't
    // distinguish "nothing changed" from "cursor too old" from the ops response
    // alone, so we re-anchor against the snapshot's authoritative `seq_id`: only
    // when the ops list is empty do we cheaply confirm the cursor still tracks
    // the server (a single extra call ONLY on the empty-delta path is acceptable;
    // any non-empty delta means the cursor is valid and we skip the check).
    if ops.ops.is_empty() {
        // No new ops. Verify the cursor hasn't drifted past the server head; if
        // it has (server reset its op log), re-bootstrap. The snapshot call here
        // is the single concession — it runs only when there is genuinely
        // nothing to apply, so steady state stays at ~1 request/tick.
        // `now_secs` (captured at tick start) is the prune freshness cutoff: it
        // predates this snapshot fetch, so it can't be later than any row a
        // concurrent completion stamps during the fetch.
        let fetched_at = now_secs;
        match bridge.api().sync_snapshot().await {
            Ok(snap) if snap.seq_id < cursor => {
                tracing::warn!(
                    cursor,
                    server_seq = snap.seq_id,
                    "sync_tick: cursor ahead of server op-log head; re-bootstrapping from snapshot"
                );
                let applied = apply_snapshot(bridge, sync_root, &snap, now_secs, fetched_at, &mut conflicts)?;
                applied_item_ids.extend(applied);
            }
            Ok(_) => { /* cursor still valid, nothing to do */ }
            Err(e) => {
                tracing::warn!(error = %e, "sync_tick: snapshot freshness probe failed; retrying next tick");
            }
        }
        return Ok(SyncTickOutcome {
            conflicts,
            applied_item_ids: dedupe(applied_item_ids),
        });
    }

    // Apply the delta ops in order, advancing the cursor to the max seq_id.
    let mut max_seq = cursor;
    for op in &ops.ops {
        match apply_sync_op(bridge, sync_root, op, now_secs, &mut conflicts) {
            Ok(applied) => applied_item_ids.extend(applied),
            Err(e) => {
                // A failed op applied nothing: it contributes no ids.
                tracing::warn!(error = %e, op_type = %op.op_type, seq_id = op.seq_id, "sync_tick: op apply error; skipping op");
            }
        }
        max_seq = max_seq.max(op.seq_id);
    }
    if max_seq > cursor {
        bridge.db().set_sync_cursor(max_seq)?;
    }
    Ok(SyncTickOutcome {
        conflicts,
        applied_item_ids: dedupe(applied_item_ids),
    })
}

/// Drop duplicate ids from one tick's applied batch (an item can be applied
/// by two ops in one delta — the signal cares that it changed, not how often).
fn dedupe(mut ids: Vec<String>) -> Vec<String> {
    ids.sort();
    ids.dedup();
    ids
}

/// Pull a fresh `/sync/snapshot` and reconcile the whole mirror against it, then
/// store its `seq_id` as the new cursor. The bootstrap path (cursor unset) AND
/// the gap-recovery path both funnel through here so the behaviour is identical.
async fn bootstrap_from_snapshot(
    bridge: &EngineBridge,
    #[cfg_attr(not(target_os = "windows"), allow(unused_variables))] sync_root: &Path,
    now_secs: i64,
    conflicts: &mut Vec<ConflictDetected>,
) -> anyhow::Result<Vec<String>> {
    // `now_secs` is captured at tick start, BEFORE this request — so it is the
    // prune freshness cutoff that never prunes a row a concurrent local
    // completion stamps while this snapshot is in flight (see `prune_absent`).
    let fetched_at = now_secs;
    let snapshot = bridge.api().sync_snapshot().await?;
    let applied = apply_snapshot(bridge, sync_root, &snapshot, now_secs, fetched_at, conflicts)?;
    // §6.3.3: an upload still waiting for its base after this successful snapshot counts
    // one more; at the 10th it parks with its bytes.
    #[cfg(target_os = "macos")]
    for (op_id, file_id) in bridge.db().note_snapshot_for_base_pending(now_secs)? {
        log_parked(&op_id, file_id.as_deref(), ParkReason::BaseUnknown);
    }
    Ok(applied)
}

/// Reconcile the local mirror against an already-fetched snapshot:
///   1. order nodes parent-before-child and compute each node's parent rel path,
///   2. ingest every node through `process_metadata_row` (same conflict logic +
///      `upsert_file` placeholder-freshness flow the old BFS used),
///   3. PRUNE every own-tree row the snapshot omits (deletion reconciliation),
///      EXCEPT rows touched at/after `snapshot_fetched_at` (a just-completed
///      upload) and EXCEPT the whole tree when the snapshot is suspiciously empty
///      (both handled inside `prune_absent`),
///   4. store the snapshot's `seq_id` as the cursor.
///
/// `snapshot_fetched_at` is the wall-clock second the snapshot HTTP fetch began;
/// it is the prune freshness cutoff so an upload re-keyed mid-flight survives.
///
/// Split out from `bootstrap_from_snapshot` so it's directly unit-testable with
/// a synthetic `SyncSnapshot` (no HTTP).
fn apply_snapshot(
    bridge: &EngineBridge,
    #[cfg_attr(not(target_os = "windows"), allow(unused_variables))] sync_root: &Path,
    snapshot: &crate::api_client::SyncSnapshot,
    now_secs: i64,
    snapshot_fetched_at: i64,
    conflicts: &mut Vec<ConflictDetected>,
) -> anyhow::Result<Vec<String>> {
    // Task 1697 review fix (T3): every ingested and pruned item id flows back
    // to the tick, which signals the File Provider working set with the union.
    let mut applied: Vec<String> = Vec::new();
    // Resolve each node's PARENT relative path. The snapshot is a flat node list
    // (each with `parent_id`); the old BFS got nesting for free by listing
    // folder-by-folder. Here we topologically order so a parent's resolved path
    // is known before its children, then thread it into `process_metadata_row`
    // exactly as the BFS threaded the folder's path.
    let order = order_snapshot_nodes(&snapshot.nodes);

    let mut seen: HashSet<String> = HashSet::new();
    // file_id → resolved FULL relative path (so children can prefix their leaf).
    let mut resolved_paths: std::collections::HashMap<String, String> = std::collections::HashMap::new();

    for f in order {
        let file_id = f["id"].as_str().unwrap_or_default();
        if file_id.is_empty() {
            continue;
        }
        if !seen.insert(file_id.to_string()) {
            continue;
        }
        // Parent's resolved path ("" at the vault root, or when the parent
        // didn't resolve — the leaf then sits at root, matching BFS behaviour
        // where an unresolved parent frame simply wasn't descended).
        let parent_rel_path = match f["parent_id"].as_str().filter(|pid| !pid.is_empty()) {
            Some(pid) => resolved_paths.get(pid).cloned().unwrap_or_else(|| {
                // If the parent is missing from this snapshot but the child already
                // exists locally, keep its current parent prefix for this ingest.
                // A concurrent folder trash can legitimately produce a partial
                // snapshot that still lists the folder's children but not the folder;
                // re-rooting those children before `prune_absent` runs would hide
                // them from the absent-folder subtree sweep.
                let existing_parent = existing_parent_rel_path(bridge, file_id);
                if !existing_parent.is_empty() {
                    tracing::debug!(
                        file_id = %file_id,
                        parent_id = %pid,
                        parent_rel_path = %existing_parent,
                        "sync_tick: preserving existing parent path for snapshot node whose parent is absent"
                    );
                }
                existing_parent
            }),
            None => String::new(),
        };

        if let Some((rel_path, _kind)) =
            process_metadata_row(bridge, f, &parent_rel_path, now_secs, RowSource::Snapshot, conflicts)?
        {
            if !rel_path.is_empty() {
                resolved_paths.insert(file_id.to_string(), rel_path);
            }
            applied.push(file_id.to_string());
        }
    }

    // Deletion reconciliation: anything in the local own-tree the snapshot did
    // NOT mention was deleted/trashed server-side. `prune_absent` excludes shared
    // rows, rows with a pending upload/op, rows touched at/after the snapshot
    // fetch (a just-completed upload the snapshot predates), and refuses to prune
    // the whole tree on a suspicious EMPTY snapshot (see its doc) — so neither an
    // in-flight local create nor a just-uploaded file is ever collateral.
    let pruned = bridge.db().prune_absent(&seen, snapshot_fetched_at)?;
    if !pruned.is_empty() {
        tracing::info!(count = pruned.len(), "sync_tick: pruned rows absent from snapshot");
        // task 0806: removing only the DB row left the on-disk Cloud Files
        // placeholder ghosting in Explorer. Now ALSO remove each pruned row's
        // placeholder. `prune_absent` returns rows CHILDREN-BEFORE-PARENTS (orphan
        // descendants of a pruned folder precede the folder), which is exactly the
        // order placeholder removal needs (leaf files before their directory).
        remove_pruned_placeholders(sync_root, &pruned);
        // A pruned row is a REMOTE deletion Finder must see (T3).
        applied.extend(pruned.iter().map(|row| row.file_id.clone()));
    }

    bridge.db().set_sync_cursor(snapshot.seq_id)?;
    Ok(applied)
}

/// Windows: remove the on-disk Cloud Files placeholder for each row the deletion
/// reconcile just dropped from the DB (task 0806). The DB row is ALREADY gone (the
/// caller deleted it before calling this), so the watcher's `handle_delete` finds
/// no row and queues no server trash; we ALSO register each path in the watcher's
/// engine-delete suppression set so the `NOTIFY_DELETE_COMPLETION` our own remove
/// fires is dropped deterministically rather than echoing into a redundant trash.
///
/// `rows` MUST be ordered children-before-parents (both `prune_absent` and
/// `delete_file_subtree` return that order) so a directory's leaf placeholders are
/// removed before the directory placeholder itself.
///
/// A row whose path can't be safely resolved under the root (empty/traversal) is
/// skipped. Per-row failures are logged (file_id only — zero-knowledge) and never
/// abort the sweep. No-op on macOS/Linux.
#[cfg(target_os = "windows")]
fn remove_pruned_placeholders(sync_root: &Path, rows: &[crate::state_db::PrunedRow]) {
    for row in rows {
        let Some(path) = EngineBridge::placeholder_path_under(sync_root, &row.path) else {
            continue;
        };
        // Register BEFORE the remove so the NOTIFY_DELETE_COMPLETION the remove
        // fires is already suppressed when it lands in the watcher.
        crate::watcher::suppress_engine_delete(&path);
        if let Err(e) = crate::windows_cf::placeholders::delete_placeholder(&path, row.is_dir) {
            // Zero-knowledge: never log the path — only the file_id + kind.
            tracing::warn!(file_id = %row.file_id, is_dir = row.is_dir, error = %e, "remote-deletion reconcile: placeholder removal failed");
        }
    }
}

/// No-op stand-in on non-Windows builds so the call sites stay platform-agnostic.
#[cfg(not(target_os = "windows"))]
fn remove_pruned_placeholders(_sync_root: &Path, _rows: &[crate::state_db::PrunedRow]) {}

/// Order snapshot nodes so every node appears AFTER its parent (parents-first),
/// which lets `apply_snapshot` resolve a parent's path before its children.
///
/// Roots (no `parent_id`, or a `parent_id` not present in the node set — a
/// shared-into / cross-tree parent) come first; remaining nodes are emitted as
/// their parents become available. Any nodes left in a `parent_id` cycle the
/// server should never produce are appended at the end so they're still ingested
/// (just possibly with an empty parent path), never dropped.
fn order_snapshot_nodes(nodes: &[serde_json::Value]) -> Vec<&serde_json::Value> {
    use std::collections::HashMap;
    let present: HashSet<&str> = nodes.iter().filter_map(|n| n["id"].as_str()).collect();
    // children[parent_id] = [node, …]
    let mut children: HashMap<&str, Vec<&serde_json::Value>> = HashMap::new();
    let mut roots: Vec<&serde_json::Value> = Vec::new();
    for n in nodes {
        match n["parent_id"].as_str() {
            Some(pid) if present.contains(pid) => children.entry(pid).or_default().push(n),
            _ => roots.push(n), // root, or parent outside the snapshot set
        }
    }
    let mut ordered: Vec<&serde_json::Value> = Vec::with_capacity(nodes.len());
    let mut queue: VecDeque<&serde_json::Value> = roots.into_iter().collect();
    let mut emitted: HashSet<&str> = HashSet::new();
    while let Some(n) = queue.pop_front() {
        let Some(id) = n["id"].as_str() else { continue };
        if !emitted.insert(id) {
            continue;
        }
        ordered.push(n);
        if let Some(kids) = children.get(id) {
            for k in kids {
                queue.push_back(k);
            }
        }
    }
    // Defensive: emit any node not reached (a parent_id cycle) so nothing is lost.
    for n in nodes {
        if let Some(id) = n["id"].as_str() {
            if emitted.insert(id) {
                ordered.push(n);
            }
        }
    }
    ordered
}

/// Apply ONE `/sync/ops` delta to the local mirror by `op_type`. Mirrors the
/// server's op vocabulary (`server/.../routes/sync.rs` + the `emit_sync_op`
/// call sites in `routes/files.rs` / `routes/uploads.rs`):
///   * `file_create` / `folder_create` → ingest the new row (cloud_only),
///   * `file_update`                   → refresh the existing row's metadata,
///   * `file_rename` / `folder_rename` → update the leaf name (re-derives path),
///   * `file_move`   / `folder_move`   → re-parent the row,
///   * `file_trash`  / `file_delete`   → remove the local mirror row,
///   * `file_restore`                  → request a re-snapshot ({id}-only payload
///     can't rebuild the row; the authoritative snapshot re-materialises it).
///
/// Each op payload is `{ id, … }`. For create/update/rename/move we synthesise a
/// `serde_json::Value` row and feed it through the SAME `process_metadata_row`
/// ingest path the snapshot uses, so conflict detection + placeholder freshness
/// are identical across both paths. Nested paths ARE resolved from the op's
/// `parent_id`/`new_parent_id` against the local mirror (`parent_rel_path_by_id`),
/// matching how `apply_snapshot` threads parent paths — because the Windows
/// placeholder seeder (`populate_placeholders`) derives the on-disk location
/// purely from the row's stored `path`, NOT from `parent_id`, and there is no
/// periodic snapshot to fix a mis-prefixed row later. When a parent isn't locally
/// known yet, the op requests a re-snapshot so nesting converges next tick.
fn apply_sync_op(
    bridge: &EngineBridge,
    #[cfg_attr(not(target_os = "windows"), allow(unused_variables))] sync_root: &Path,
    op: &crate::api_client::SyncOp,
    now_secs: i64,
    conflicts: &mut Vec<ConflictDetected>,
) -> anyhow::Result<Vec<String>> {
    // Task 1697 review fix (T3): the ids of items this op actually changed on
    // the local mirror — what the runner's working-set signal must carry.
    let mut applied: Vec<String> = Vec::new();
    let payload = &op.payload;
    let id = payload["id"].as_str().unwrap_or_default();
    if id.is_empty() {
        return Ok(applied);
    }

    match op.op_type.as_str() {
        // Task 1698 (trash ruling — full sync): the two server-side deletion
        // kinds now mean different things locally.
        //
        // `file_trash` — the server TRASH (is_trashed=TRUE, recoverable): the
        // mirror flips the subtree to `Trashing` (the macOS Trash view) and
        // keeps the rows; each flipped row records a `Modified` change so the
        // replica moves it into the trash container. A `Trashing` row is the
        // server echo of the LOCAL delete-in-flight (0802) — the row already
        // IS the trash view, nothing to flip.
        //
        // `file_delete` — the server PERMANENT delete (password-confirmed in
        // the app/web, or the retention janitor): the mirror row leaves the
        // trash view for good — the row is deleted and a `deleted` change is
        // recorded. This is the convergence path that replaced prune_absent's
        // 0802 sweep (flipped by task 1698).
        "file_trash" => match bridge.db().get_file(id)? {
            Some(entry) if entry.status == crate::state_db::FileStatus::Trashing => {
                // PR #100 review (Codex P1): the root may already be parked
                // `Trashing` by the LOCAL trash flow — `queue_finder_delete`
                // parks ONLY the folder row, its descendants stay unmarked
                // until this server echo arrives. This echo IS that arrival:
                // mark the still-unmarked subtree (mark_subtree_trashing
                // no-ops the parked root and every already-Trashing row, so
                // no duplicate change rows) — otherwise prune_absent's
                // immediate-parent protection cannot cover grandchildren and
                // they drop out of the trash view.
                let marked = bridge.db().mark_subtree_trashing(id)?;
                remove_pruned_placeholders(sync_root, &marked);
                applied.extend(marked.iter().map(|row| row.file_id.clone()));
            }
            Some(_) => {
                // Mark the subtree Trashing (the trash view), remove the
                // on-disk placeholders exactly as the delete path did (the
                // item is hidden locally — that part of 0802 stands), and
                // report the ids so the working-set signal fires.
                let marked = bridge.db().mark_subtree_trashing(id)?;
                remove_pruned_placeholders(sync_root, &marked);
                applied.push(id.to_string());
                applied.extend(marked.iter().map(|row| row.file_id.clone()));
            }
            None => {
                let orphans = bridge.db().delete_orphaned_children_of_absent_folder(id)?;
                if !orphans.is_empty() {
                    tracing::info!(
                        folder_id = %id,
                        count = orphans.len(),
                        "sync_tick: trashed folder had no local row but \
                         {} orphaned child(ren) — pruned (task 0828)",
                        orphans.len()
                    );
                    remove_pruned_placeholders(sync_root, &orphans);
                    // The orphaned children left the mirror (T3); the folder
                    // itself never existed locally, so it contributes nothing.
                    applied.extend(orphans.iter().map(|row| row.file_id.clone()));
                }
            }
        },
        "file_delete" => match bridge.db().get_file(id)? {
            Some(_) => {
                // The item is GONE server-side (irreversible): drop the row —
                // Trashing rows leave the trash view, live rows leave the tree
                // (unchanged 0806 behavior for a live row).
                let removed = bridge.db().delete_file_subtree(id)?;
                remove_pruned_placeholders(sync_root, &removed);
                applied.push(id.to_string());
                applied.extend(removed.iter().map(|row| row.file_id.clone()));
            }
            None => {
                let orphans = bridge.db().delete_orphaned_children_of_absent_folder(id)?;
                if !orphans.is_empty() {
                    remove_pruned_placeholders(sync_root, &orphans);
                    applied.extend(orphans.iter().map(|row| row.file_id.clone()));
                }
            }
        },
        "file_restore" => {
            // The restore payload is only `{ id }` (server `routes/files.rs`), so
            // the un-trashed row's full metadata is NOT recoverable from the op.
            // We already DELETED this row on the preceding `file_trash`/
            // `file_delete` op, and there is NO periodic snapshot in steady state
            // (the cursor only advances via ops once bootstrapped) — so a no-op
            // here would make a trash-then-restore on another device leave the
            // file permanently invisible on THIS device. Request a re-bootstrap
            // on the next tick: the authoritative snapshot re-materialises the
            // restored row. Cheap and matches the existing gap-recovery design.
            //
            // Task 1698 (trash ruling): when we still HOLD the row — it stayed
            // in the mirror as `Trashing` (the trash view) instead of being
            // pruned — the restore flips it back OUT of the trash view
            // directly: `CloudOnly` (placeholder re-mints; content
            // re-downloads on open), and the flip is reported so the replica
            // reparents the item out of the trash container.
            if let Some(row) = bridge.db().get_file(id)?
                && row.status == crate::state_db::FileStatus::Trashing
            {
                // PR #100 review (Codex P1): the restore must take the ENTIRE
                // held subtree out of the trash view, not just the root row —
                // the authoritative re-snapshot preserves `Trashing` rows by
                // design, so it can never repair the descendants (they would
                // linger in the Trash view; direct children even become
                // top-level trash entries after the parent leaves).
                let flipped = bridge.db().untrash_subtree(id)?;
                remove_pruned_placeholders(sync_root, &flipped);
                applied.extend(flipped.iter().map(|r| r.file_id.clone()));
            }
            bridge.db().request_resnapshot()?;
            tracing::info!(
                file_id = %id,
                "sync_tick: file_restore op — scheduling re-snapshot to re-materialise the row"
            );
        }
        "file_rename" | "folder_rename" => {
            // Re-key the leaf name. Build a row carrying the NEW name_encrypted so
            // `resolve_relative_path` re-derives the plaintext leaf. We keep the
            // existing row's parent path by reading its current path's parent.
            let new_name = payload["new_name_encrypted"].as_str();
            let row = synthesize_op_row(bridge, id, op, new_name);
            let parent_rel = existing_parent_rel_path(bridge, id);
            if process_metadata_row(bridge, &row, &parent_rel, now_secs, RowSource::Op, conflicts)?.is_some() {
                applied.push(id.to_string());
            }
        }
        "file_move" | "folder_move" => {
            // Re-parent. The op gives `new_parent_id`; the leaf name is unchanged.
            // The move op carries no name blob, so `synthesize_op_row` supplies the
            // unchanged leaf as a plaintext `"name"` fallback from the existing
            // row's stored path (the `files` table doesn't persist name_encrypted).
            let row = synthesize_op_row(bridge, id, op, None);
            // Resolve the NEW parent's full relative path from the local mirror by
            // `new_parent_id` (the same way `apply_snapshot` resolves nesting), so
            // a move-into-folder lands the row under its folder prefix instead of
            // at the sync root. If the new parent isn't locally known yet, fall
            // back to a re-snapshot rather than mis-placing the row at root.
            let parent_rel = parent_rel_path_by_id(bridge, payload["new_parent_id"].as_str());
            if process_metadata_row(bridge, &row, &parent_rel, now_secs, RowSource::Op, conflicts)?.is_some() {
                applied.push(id.to_string());
            }
        }
        "file_create" | "folder_create" | "file_update" => {
            // §6.3.2 (I-3): a content op without `version_number` cannot say which version it
            // made (a legacy chunked replace looks like a create), so the snapshot settles it.
            // A content op on a row this Mac already holds means the row's version is no
            // longer a snapshot fill (§6.1). A file op is a content op when it is a create, or
            // carries a version or a size; the server's thumbnail-flag ops (`has_thumbnail`,
            // `has_large_thumbnail`) carry neither, and are not.
            #[cfg(target_os = "macos")]
            let content = op.op_type == "file_create"
                || (op.op_type == "file_update"
                    && (payload["version_number"].as_i64().is_some() || payload["size_bytes"].as_i64().is_some()));
            #[cfg(target_os = "macos")]
            let versionless = content && payload["version_number"].as_i64().is_none();
            #[cfg(target_os = "macos")]
            let touches_existing_row = content && bridge.db().get_file(id)?.is_some();
            let row = synthesize_op_row(bridge, id, op, payload["name_encrypted"].as_str());
            // For a CREATE there is no existing local row, so the parent path must
            // come from the op's `parent_id` resolved against the local mirror —
            // NOT from the (nonexistent) row's own path, which would yield "" and
            // mis-place a nested create at the sync root. `populate_placeholders`
            // derives the on-disk location purely from the row's `path`, so the
            // path prefix MUST be correct here; there is no periodic snapshot to
            // fix it later. For an UPDATE the row already exists, so keep its
            // current parent prefix (`existing_parent_rel_path`).
            let parent_rel = if op.op_type == "file_update" {
                existing_parent_rel_path(bridge, id)
            } else {
                parent_rel_path_by_id(bridge, payload["parent_id"].as_str())
            };
            if process_metadata_row(bridge, &row, &parent_rel, now_secs, RowSource::Op, conflicts)?.is_some() {
                applied.push(id.to_string());
            }
            #[cfg(target_os = "macos")]
            {
                if touches_existing_row {
                    bridge.db().clear_version_filled(id)?;
                }
                if versionless {
                    bridge.db().request_resnapshot()?;
                }
            }
        }
        other => {
            tracing::debug!(op_type = other, "sync_tick: ignoring unknown op_type");
        }
    }
    Ok(applied)
}

/// Build a `/files`-shaped `serde_json::Value` from a sync op + the existing
/// local row, so `process_metadata_row` ingests it identically to a snapshot
/// node. Carries the op's `id` plus any of `name_encrypted` / `parent_id` /
/// `size_bytes` / `version_number` / `is_folder` it can determine, falling back
/// to the existing row's values where the op is partial (e.g. a `file_update`
/// that only changed `has_thumbnail`).
fn synthesize_op_row(
    bridge: &EngineBridge,
    id: &str,
    op: &crate::api_client::SyncOp,
    name_encrypted: Option<&str>,
) -> serde_json::Value {
    let payload = &op.payload;
    let existing = bridge.db().get_file(id).ok().flatten();
    let existing_contract = bridge.db().get_file_contract_state(id).ok().flatten();

    let is_folder = op.op_type.starts_with("folder_")
        || existing
            .as_ref()
            .map(|e| e.item_kind == ItemKind::Folder)
            .unwrap_or(false);

    let mut row = serde_json::json!({ "id": id, "is_folder": is_folder });

    // name_encrypted: op-provided wins (create/rename carry it). A MOVE op carries
    // NO name blob, and the `files` table does NOT persist `name_encrypted`, so it
    // is genuinely unrecoverable from the op for a move. To still compose the
    // correct path, fall back to the EXISTING row's plaintext leaf (the last
    // segment of its already-decrypted stored `path`) as the `"name"` field —
    // `resolve_relative_path` prefers `name_encrypted` but falls through to
    // `"name"`, so the moved row keeps its leaf under the new parent prefix
    // instead of resolving to an empty path (which would mis-seed at the root or
    // skip the row entirely).
    if let Some(name) = name_encrypted.or_else(|| payload["name_encrypted"].as_str()) {
        row["name_encrypted"] = serde_json::json!(name);
    } else if let Some(leaf) = existing.as_ref().and_then(|e| {
        e.path
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .map(str::to_string)
            .filter(|l| !l.is_empty())
    }) {
        row["name"] = serde_json::json!(leaf);
    }
    // parent_id: a move sets new_parent_id; create/update set parent_id; else keep
    // the existing parent.
    let parent_id = payload["new_parent_id"]
        .as_str()
        .or_else(|| payload["parent_id"].as_str())
        .map(str::to_string)
        .or_else(|| existing.as_ref().and_then(|e| e.parent_id.clone()));
    if let Some(pid) = parent_id {
        row["parent_id"] = serde_json::json!(pid);
    }
    // size_bytes: op-provided wins, else existing.
    let size = payload["size_bytes"]
        .as_i64()
        .or_else(|| existing.as_ref().map(|e| e.size_bytes));
    if let Some(sz) = size {
        row["size_bytes"] = serde_json::json!(sz);
    }
    // version_number: op-provided wins, else the existing contract's.
    let version = payload["version_number"]
        .as_i64()
        .or_else(|| existing_contract.as_ref().map(|c| c.current_version));
    if let Some(v) = version {
        row["version_number"] = serde_json::json!(v);
    }
    // current_object_version_id: a `file_update` carries the new version's
    // object id; without it the contract keeps pointing at the old version.
    // `apply_metadata_file_row` keeps the existing id when the op omits it.
    if let Some(ov) = payload["current_object_version_id"].as_str() {
        row["current_object_version_id"] = serde_json::json!(ov);
    }
    // updated_at: the op log carries no file timestamp the client can read, so
    // stamp "now" as the row's `remote_updated_at` / `modified_at` value. This
    // is NOT a freshness token: several ops routinely land in the same second
    // (a version upload emits `file_rename` + `file_update` together), so
    // `process_metadata_row` never gates an op-derived row on it — see
    // [`RowSource::Op`].
    row["updated_at"] = serde_json::json!(now_secs());
    row
}

/// Resolve a parent's FULL relative path from the local mirror by its `file_id`
/// — the parent path a freshly-created/moved child must be prefixed with. This
/// mirrors how [`apply_snapshot`] resolves nesting (it threads each parent's
/// resolved full path into its children), so an op-driven create/move lands at
/// the SAME path a fresh snapshot would produce.
///
///   * `parent_id == None`            → "" (root child),
///   * parent row present locally     → the parent's stored full `path` (its
///     leading slash trimmed so it composes cleanly as a prefix),
///   * `parent_id == Some` but the parent row is NOT in the local mirror yet
///     (the create/move arrived before its parent's create op landed) → "" for
///     THIS tick AND a re-snapshot is requested, so the authoritative snapshot
///     re-places the row under its correct prefix on the next tick rather than
///     leaving it permanently mis-placed at the sync root.
fn parent_rel_path_by_id(bridge: &EngineBridge, parent_id: Option<&str>) -> String {
    let Some(pid) = parent_id.filter(|p| !p.is_empty()) else {
        return String::new(); // root child — no prefix
    };
    match bridge.db().get_file(pid).ok().flatten() {
        Some(parent) => parent.path.trim_start_matches('/').to_string(),
        None => {
            // Parent not locally known — can't compute the prefix from the op
            // alone. Schedule a re-snapshot so the next tick reconciles nesting
            // authoritatively; a best-effort log keeps this observable.
            if let Err(e) = bridge.db().request_resnapshot() {
                tracing::warn!(error = %e, "sync_tick: could not request re-snapshot for unknown parent");
            }
            tracing::info!(
                parent_id = %pid,
                "sync_tick: op references a parent not in the local mirror; scheduling re-snapshot"
            );
            String::new()
        }
    }
}

/// The PARENT relative path of an existing local row (everything before the last
/// `/` in its stored `path`), or "" when the row is unknown / at the vault root.
/// Used so an op-driven re-ingest keeps a nested row under its folder prefix
/// without needing the parent's plaintext name in the op.
fn existing_parent_rel_path(bridge: &EngineBridge, id: &str) -> String {
    bridge
        .db()
        .get_file(id)
        .ok()
        .flatten()
        .and_then(|e| {
            let p = e.path.trim_start_matches('/');
            p.rsplit_once('/').map(|(parent, _leaf)| parent.to_string())
        })
        .unwrap_or_default()
}

/// Where a row fed to [`process_metadata_row`] came from. Decides whether the
/// `updated_at` short-circuit for an already-`Local` row may fire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RowSource {
    /// A `/sync/snapshot` node. The whole tree is re-ingested, so an unchanged
    /// row must be skipped; `updated_at` is the server's own timestamp.
    Snapshot,
    /// A row synthesised from ONE `/sync/ops` delta. Every op is a real remote
    /// change delivered exactly once (the cursor advances past it), and its
    /// `updated_at` is only the local wall-clock second `synthesize_op_row`
    /// stamped — two ops in one tick share it. Gating on it dropped the
    /// `file_update` that follows a same-second `file_rename` (flow-7 P0), so
    /// op rows are always applied. Re-applying an op after a crash before the
    /// cursor persisted is idempotent: ops replay in order to the same state.
    Op,
}

/// Apply one server metadata row during [`sync_tick`]'s recursive walk,
/// running the same three-way decision the flat sweep used (new → cloud_only;
/// local + remote-moved → conflict check; otherwise refresh metadata). On
/// success returns `Some((resolved_rel_path, item_kind))` so the BFS can
/// recurse into folders using the row's full nested path; returns `None` only
/// when the row is unusable (empty id / undecryptable name).
fn process_metadata_row(
    bridge: &EngineBridge,
    f: &serde_json::Value,
    parent_rel_path: &str,
    now_secs: i64,
    source: RowSource,
    conflicts: &mut Vec<ConflictDetected>,
) -> anyhow::Result<Option<(String, ItemKind)>> {
    let file_id = f["id"].as_str().unwrap_or_default();
    if file_id.is_empty() {
        return Ok(None);
    }
    let size = f["size_bytes"].as_i64().unwrap_or(0);
    let remote_updated = f["updated_at"].as_i64().unwrap_or(0);

    // §6.3.2: a snapshot node fills a row at version 0, or raises an older one, in any
    // status but Trashing and Conflict. Only the version (and the size) changes here; the
    // short-circuit below still holds for every other field.
    #[cfg(target_os = "macos")]
    if source == RowSource::Snapshot
        && let Some(node_version) = f["version_number"].as_i64()
    {
        let is_uploading = f["is_uploading"].as_bool() == Some(true);
        match bridge
            .db()
            .apply_snapshot_version(file_id, node_version, size, is_uploading)?
        {
            crate::state_db::SnapshotVersion::Raised { old, new } => tracing::warn!(
                file_id = %file_id,
                old_version = old,
                new_version = new,
                "file version raised from the snapshot"
            ),
            crate::state_db::SnapshotVersion::Filled { resolved_ops } => {
                for op_id in resolved_ops {
                    tracing::warn!(
                        op_id = %op_id,
                        file_id = %file_id,
                        base_version = node_version,
                        "queued write based on the version the snapshot reported"
                    );
                }
            }
            crate::state_db::SnapshotVersion::Unchanged | crate::state_db::SnapshotVersion::SkippedUploading => {}
        }
    }

    // Helper to refresh metadata + return the resolved path/kind. The single
    // place that writes the row's nested path and folder/file classification.
    let apply = |bridge: &EngineBridge| -> anyhow::Result<Option<(String, ItemKind)>> {
        let entry = apply_metadata_file_row(
            bridge.db(),
            f,
            Namespace::MyFiles,
            None,
            None,
            PERMISSION_READ | PERMISSION_WRITE | PERMISSION_OWNER,
            now_secs,
            bridge.api().master_key(),
            parent_rel_path,
        )?;
        Ok(entry.map(|e| (e.path, e.item_kind)))
    };

    match bridge.db().get_file(file_id)? {
        // (0) Locally-deleted, server-trash pending (task 0802). The row is in
        // `Trashing` because the user deleted the file and the `TrashFile` op has
        // already succeeded (op removed) OR is still in flight. A `/sync/snapshot`
        // that STILL lists this file is just propagation lag — the server has set
        // `is_trashed=TRUE` but the snapshot read hasn't caught up. We MUST NOT
        // re-materialise the row: neither re-insert it nor flip it back to
        // `CloudOnly`, which would re-mint the placeholder (the "deleted file comes
        // back" race). Keep it EXACTLY as-is (`Trashing`, no placeholder). When the
        // trash finally propagates the file drops out of the snapshot and
        // `prune_absent` removes the row; if the `file_trash` op echo arrives first
        // `apply_sync_op` deletes it. Either way it converges WITHOUT resurrecting.
        Some(entry) if entry.status == FileStatus::Trashing => {
            // Still report path/kind so a (rare) trashing folder's independent
            // children continue to enumerate; do NOT touch this row's status.
            Ok(Some((entry.path, entry.item_kind)))
        }
        // (1a) New to us but still an in-progress upload (`is_uploading`): it
        // has no completed content yet — hydrating it would 409 — so minting a
        // placeholder shows a broken duplicate in Finder/Explorer (flow 7: an
        // interrupted upload's orphan row). Skip it; once `complete` lands the
        // row is listed with `is_uploading = false` and materialises normally.
        None if f["is_uploading"].as_bool() == Some(true) => Ok(None),
        None => {
            // (1) New to us — insert as cloud_only. base = remote.
            apply(bridge)
        }
        Some(entry) if entry.status == FileStatus::Local => {
            // (2) Local copy + remote moved? Nothing to check if the
            // timestamps haven't drifted past base. Still report the row's
            // path/kind so a folder we already have locally is still descended.
            // Snapshot rows only: an op row's `updated_at` is a local
            // same-second stamp, not a freshness token (see `RowSource::Op`).
            if source == RowSource::Snapshot && remote_updated <= entry.remote_updated_at {
                return Ok(Some((entry.path, entry.item_kind)));
            }

            // Synthesise version triplet (no server content hash on metadata;
            // `size-mtime` is an opaque-but-stable surrogate — conservative,
            // a same-size+mtime edit won't flag, but those collisions are rare).
            let local = VersionInfo {
                hash: entry.content_hash.clone().unwrap_or_default(),
                modified_at: entry.modified_at as u64,
            };
            let remote = VersionInfo {
                hash: format!("{size}-{remote_updated}"),
                modified_at: remote_updated as u64,
            };
            let base = VersionInfo {
                hash: entry.content_hash.clone().unwrap_or_default(),
                modified_at: entry.remote_updated_at as u64,
            };
            // Local matches base → only remote changed. Quietly re-anchor and
            // let a future hydrate replace bytes when the user opens it.
            if !is_conflict(&local, &remote, &base) {
                let mut updated = entry.clone();
                updated.remote_updated_at = remote_updated;
                updated.size_bytes = size;
                updated.modified_at = remote_updated;
                bridge.db().upsert_file(&updated)?;
                return apply(bridge);
            }

            // True conflict. Flip status, anchor `modified_at` to "now" so the
            // auto-resolution clock starts from detection.
            let mut updated = entry.clone();
            updated.status = FileStatus::Conflict;
            updated.modified_at = now_secs;
            bridge.db().upsert_file(&updated)?;

            let file_name = std::path::Path::new(&entry.path)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or(&entry.path)
                .to_string();
            let is_text = is_text_file(&file_name);
            tracing::warn!(
                file_id = %entry.file_id,
                name = %file_name,
                "conflict detected on tick — both sides moved past base"
            );
            conflicts.push(ConflictDetected {
                file_id: entry.file_id.clone(),
                file_name,
                is_text,
            });
            // A conflicted folder is unusual, but still report its path/kind so
            // enumeration of its (independent) children continues.
            Ok(Some((entry.path, entry.item_kind)))
        }
        Some(entry) => {
            // (3) Pending download/upload/conflict/error — let the dedicated
            // path own its transitions, but still refresh cloud_only/downloading
            // metadata (and always report path/kind so folders are descended).
            if matches!(entry.status, FileStatus::CloudOnly | FileStatus::Downloading) {
                return apply(bridge);
            }
            Ok(Some((entry.path, entry.item_kind)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use serde_json::json;
    use std::io::Write as _;
    use std::net::TcpListener;
    use std::sync::Mutex;
    use std::thread;

    #[test]
    fn test_file_state_machine_transitions() {
        // cloud_only -> downloading -> local
        let mut sm = FileSM::new(FileStatus::CloudOnly);
        sm.transition(FileEvent::DownloadStart).unwrap();
        assert_eq!(sm.state(), FileStatus::Downloading);
        sm.transition(FileEvent::DownloadComplete).unwrap();
        assert_eq!(sm.state(), FileStatus::Local);
    }

    #[test]
    fn test_invalid_transition_rejected() {
        let mut sm = FileSM::new(FileStatus::CloudOnly);
        assert!(sm.transition(FileEvent::UploadStart).is_err());
    }

    // ── Shared upload-trigger helpers (task 0780) ────────────────────────────
    // Relocated from the retired `watcher`-local copies so the one shared
    // engine-internal + relative-path logic stays covered.

    #[test]
    fn engine_internal_filters_state_dir_and_lock() {
        let root = std::path::PathBuf::from("/sync");
        assert!(path_is_engine_internal(&root, &root.join(".beebeeb").join("state.db")));
        assert!(path_is_engine_internal(&root, &root.join(".beebeeb-sync.lock")));
        assert!(path_is_engine_internal(
            &root,
            &root.join("sub").join(".beebeeb").join("x")
        ));
        assert!(!path_is_engine_internal(&root, &root.join("photo.jpg")));
        assert!(!path_is_engine_internal(&root, &root.join("docs").join("a.txt")));
    }

    #[test]
    fn relative_db_path_is_slash_joined_without_leading_slash() {
        let root = std::path::PathBuf::from("/sync");
        assert_eq!(relative_db_path(&root, &root.join("a.txt")).as_deref(), Some("a.txt"));
        assert_eq!(
            relative_db_path(&root, &root.join("docs").join("b.md")).as_deref(),
            Some("docs/b.md")
        );
        assert_eq!(relative_db_path(&root, &std::path::PathBuf::from("/other/c.txt")), None);
    }

    #[test]
    fn test_temp_file_filter_covers_editor_and_system_names() {
        for name in [
            ".DS_Store",
            "._report.pdf",
            "~$budget.xlsx",
            "draft.txt~",
            ".report.swp",
            "upload.tmp",
            "video.crdownload",
        ] {
            assert!(is_ignored_finder_name(name), "{name} should be ignored");
        }
        assert!(!is_ignored_finder_name("report.pdf"));
        assert!(!is_ignored_finder_name(".env.sample"));
    }

    #[test]
    fn test_content_side_from_bytes_covers_text_binary_and_size_cap() {
        // Task 1546 finding 2: the conflict window's diff body was a
        // hardcoded placeholder for EVERY conflict, text or binary,
        // regardless of actual file content. `content_side_from_bytes` is
        // the pure classifier `EngineBridge::conflict_content_preview` uses
        // to turn real bytes into what the window can safely show.
        let small_text = content_side_from_bytes(b"hello world".to_vec(), true);
        assert_eq!(small_text.text.as_deref(), Some("hello world"));
        assert_eq!(small_text.size_bytes, Some(11));
        assert!(small_text.unavailable_reason.is_none());

        // Binary side never carries text, even for tiny content — only size.
        let binary = content_side_from_bytes(b"hello world".to_vec(), false);
        assert!(binary.text.is_none());
        assert_eq!(binary.size_bytes, Some(11));
        assert!(binary.unavailable_reason.is_none());

        // Invalid UTF-8 for a file the caller marked as text: honest
        // "isn't valid UTF-8" reason, never fabricated text.
        let invalid_utf8 = content_side_from_bytes(vec![0xFF, 0xFE, 0xFD], true);
        assert!(invalid_utf8.text.is_none());
        assert_eq!(invalid_utf8.size_bytes, Some(3));
        assert!(invalid_utf8.unavailable_reason.unwrap().contains("UTF-8"));

        // Oversized text: the whole file round-trips over Tauri's IPC as a
        // JSON string and is diffed synchronously, so there is a real cap —
        // over it, size is still reported honestly but text is withheld
        // rather than silently truncated (which would corrupt the diff).
        let oversized = vec![b'a'; CONFLICT_PREVIEW_TEXT_MAX_BYTES + 1];
        let too_big = content_side_from_bytes(oversized, true);
        assert!(too_big.text.is_none());
        assert_eq!(too_big.size_bytes, Some((CONFLICT_PREVIEW_TEXT_MAX_BYTES + 1) as u64));
        assert!(too_big.unavailable_reason.unwrap().contains("too large"));
    }

    #[test]
    fn test_base_version_parser_reads_current_version_prefix() {
        assert_eq!(parse_base_version_number(Some("7:1700000000:1024")), Some(7));
        assert_eq!(parse_base_version_number(Some("0:1700000000:1024")), None);
        assert_eq!(parse_base_version_number(Some("local:1700000000")), None);
        assert_eq!(parse_base_version_number(None), None);
    }

    #[test]
    fn test_upload_init_request_omits_new_file_id_and_preserves_existing_base() {
        let req = upload_init_request_for_operation(
            "file-new",
            "{\"cipher_suite\":\"V1Aes256Gcm\"}",
            Some("image/png".into()),
            Some("folder-1".into()),
            11,
            None,
            true,
        );
        assert_eq!(req.file_id, None);
        assert_eq!(req.file_size_bytes, 11);
        assert_eq!(req.profile, "desktop");
        assert!(req.is_media);
        assert_eq!(req.chunk_count, Some(1));

        let replace = upload_init_request_for_operation(
            "file-existing",
            "{\"cipher_suite\":\"V1Aes256Gcm\"}",
            Some("text/plain".into()),
            None,
            42,
            Some(7),
            false,
        );
        assert_eq!(replace.file_id.as_deref(), Some("file-existing"));
        assert_eq!(replace.base_version_number, Some(7));
        assert!(!replace.is_media);
    }

    fn write_test_png(path: &Path) {
        let image = image::RgbaImage::from_fn(8, 6, |x, y| {
            image::Rgba([(x * 31) as u8, (y * 41) as u8, ((x + y) * 17) as u8, 255])
        });
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image)
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        std::fs::write(path, png.into_inner()).unwrap();
    }

    #[test]
    fn test_prepare_thumbnail_uploads_encrypts_medium_and_large_with_blurhash() {
        let dir = tempfile::tempdir().unwrap();
        let payload = dir.path().join("photo.png");
        write_test_png(&payload);

        let master_key = beebeeb_core::kdf::MasterKey::from_bytes([31u8; 32]);
        let file_key = beebeeb_core::kdf::derive_file_key(&master_key, b"server-file-1");
        let uploads = prepare_thumbnail_uploads_for_plaintext_media(&payload, Some("image/png"), &file_key).unwrap();

        assert_eq!(uploads.len(), 2);
        assert_eq!(uploads[0].variant, ThumbnailUploadVariant::Medium);
        assert_eq!(uploads[1].variant, ThumbnailUploadVariant::Large);
        assert!(uploads[0].blurhash.as_ref().is_some_and(|hash| hash.len() <= 64));
        assert!(uploads[1].blurhash.is_none());

        for upload in &uploads {
            assert!(upload.encrypted.len() <= upload.variant.encrypted_max_bytes());
            let plaintext = beebeeb_core::encrypt::decrypt_chunk_raw(&file_key, &upload.encrypted).unwrap();
            assert!(plaintext.starts_with(b"RIFF"), "thumbnail plaintext must be WebP");
            assert_ne!(&*upload.encrypted, plaintext.as_slice());
        }
    }

    #[test]
    fn test_prepare_thumbnail_uploads_extracts_video_frame_when_ffmpeg_available() {
        if std::process::Command::new("ffmpeg").arg("-version").output().is_err() {
            return;
        }

        let dir = tempfile::tempdir().unwrap();
        let payload = dir.path().join("clip.mp4");
        let status = std::process::Command::new("ffmpeg")
            .arg("-nostdin")
            .arg("-hide_banner")
            .arg("-loglevel")
            .arg("error")
            .arg("-y")
            .arg("-f")
            .arg("lavfi")
            .arg("-i")
            .arg("testsrc=size=16x16:rate=1")
            .arg("-t")
            .arg("1")
            .arg("-pix_fmt")
            .arg("yuv420p")
            .arg(&payload)
            .status()
            .unwrap();
        if !status.success() {
            return;
        }

        let master_key = beebeeb_core::kdf::MasterKey::from_bytes([32u8; 32]);
        let file_key = beebeeb_core::kdf::derive_file_key(&master_key, b"server-video-1");
        let uploads = prepare_thumbnail_uploads_for_plaintext_media(&payload, Some("video/mp4"), &file_key).unwrap();

        assert_eq!(uploads.len(), 2);
        assert_eq!(uploads[0].variant, ThumbnailUploadVariant::Medium);
        assert_eq!(uploads[1].variant, ThumbnailUploadVariant::Large);
        assert!(uploads[0].blurhash.is_none(), "video thumbnails do not carry blurhash");
        assert!(uploads[1].blurhash.is_none());

        for upload in &uploads {
            let plaintext = beebeeb_core::encrypt::decrypt_chunk_raw(&file_key, &upload.encrypted).unwrap();
            assert!(plaintext.starts_with(b"RIFF"), "video thumbnail plaintext must be WebP");
        }
    }

    #[test]
    fn test_download_chunk_decrypts_raw_and_json_formats() {
        let file_id = uuid::Uuid::new_v4().to_string();
        let master_key = beebeeb_core::kdf::MasterKey::from_bytes([9u8; 32]);
        let file_key = beebeeb_core::kdf::derive_file_key(&master_key, file_id.as_bytes());

        let raw = beebeeb_core::encrypt::encrypt_chunk_raw(&file_key, b"raw bytes").unwrap();
        assert_eq!(decrypt_downloaded_chunk(&file_key, &raw).unwrap(), b"raw bytes");

        let blob = beebeeb_core::encrypt::encrypt_chunk(&file_key, b"json bytes").unwrap();
        let json = serde_json::to_vec(&blob).unwrap();
        assert_eq!(decrypt_downloaded_chunk(&file_key, &json).unwrap(), b"json bytes");
    }

    #[test]
    fn test_staging_payload_copies_to_durable_location() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("report.txt");
        std::fs::write(&source, b"finder data").unwrap();
        let db = Arc::new(StateDb::open(dir.path().join("state.db")).unwrap());
        let staged =
            crate::staged_payload::StagedPayload::copy(db.clone(), &source, dir.path().join("staging")).unwrap();
        assert_eq!(std::fs::read(staged.path()).unwrap(), b"finder data");
        assert_eq!(db.staged_payloads_for_signout().unwrap().len(), 1);
    }

    fn test_bridge(db_path: &Path) -> EngineBridge {
        let db = Arc::new(StateDb::open(db_path).unwrap());
        let api = Arc::new(ApiClient::new(
            "https://api.beebeeb.io".into(),
            "token".into(),
            [7u8; 32],
        ));
        EngineBridge::new(db, api)
    }

    fn test_bridge_with_api(db_path: &Path, base_url: String, master_key: [u8; 32]) -> EngineBridge {
        let db = Arc::new(StateDb::open(db_path).unwrap());
        let api = Arc::new(ApiClient::new(base_url, "token".into(), master_key));
        EngineBridge::new(db, api)
    }

    #[derive(Debug, Clone)]
    struct RecordedRequest {
        method: String,
        path: String,
        body: Vec<u8>,
    }

    struct UploadMockServer {
        base_url: String,
        requests: Arc<Mutex<Vec<RecordedRequest>>>,
        handle: thread::JoinHandle<()>,
        shutdown: Arc<AtomicBool>,
    }

    impl UploadMockServer {
        fn start(fail_chunk: bool) -> Self {
            Self::start_inner(fail_chunk, false)
        }

        /// Variant whose `PUT …/thumbnail` responses are 500s — drives the
        /// task-1700 diagnosability test (failed post-complete work must be
        /// visible on the outcome, never silent).
        fn start_failing_thumbnail_uploads(fail_chunk: bool) -> Self {
            Self::start_inner(fail_chunk, true)
        }

        fn start_inner(fail_chunk: bool, fail_thumbnails: bool) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let base_url = format!("http://{}", listener.local_addr().unwrap());
            let requests = Arc::new(Mutex::new(Vec::new()));
            let server_requests = Arc::clone(&requests);
            let shutdown = Arc::new(AtomicBool::new(false));
            let server_shutdown = Arc::clone(&shutdown);
            let handle = thread::spawn(move || {
                let started = std::time::Instant::now();
                loop {
                    // Task 1700: shutdown is owned by `finish()`, not by an
                    // idle timer. The old heuristic — stop 300ms after the
                    // request count reached a minimum — raced the client's
                    // post-complete thumbnail work: every mock response says
                    // `Connection: close`, so the thumbnail PUTs need a fresh
                    // accept, and under enough runner load the decode + 2×
                    // WebP-encode + blurhash + encrypt gap exceeds the window.
                    // The listener then dies first, the upload fails with a
                    // connection error, the product's warn swallows it (no
                    // subscriber in tests), and the test panicked at the
                    // thumbnail expect with no trail — exactly the Windows CI
                    // flake of task 1700. Recording happens-before the
                    // response is written, and the response happens-before
                    // the client's next step, so by the time `finish()` runs
                    // every recorded request is already in the list.
                    if server_shutdown.load(Ordering::SeqCst) {
                        break;
                    }
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            // The listener above is non-blocking so this accept
                            // loop can poll it (see the WouldBlock arm below),
                            // but the *accepted* stream must be put back into
                            // blocking mode before handing it to
                            // `read_http_request`, which does a raw, un-retried
                            // `.read().unwrap()`. Without this, under enough
                            // scheduler contention the request bytes can still
                            // be in flight when `read` is called, and a
                            // non-blocking read returns `WouldBlock` instead of
                            // waiting — panicking the mock server's accept
                            // thread. `IpcHydrationMock::start` (below) already
                            // does this; this server predates that fix and was
                            // missing it, causing an intermittent
                            // `Os { code: 35, kind: WouldBlock }` panic under
                            // shared-machine load (confirmed via task 1546's
                            // and task 1538's independent gate runs: 3 distinct
                            // tests using this helper each panicked here under
                            // load and passed 3/3 when rerun in isolation).
                            stream.set_nonblocking(false).unwrap();
                            let request = read_http_request(&mut stream);
                            let response = upload_mock_response(&request, fail_chunk, fail_thumbnails);
                            server_requests.lock().unwrap().push(request);
                            stream.write_all(response.as_bytes()).unwrap();
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            // Hang backstop only; the normal exit is the
                            // shutdown flag above.
                            if started.elapsed() >= Duration::from_secs(30) {
                                break;
                            }
                            std::thread::sleep(Duration::from_millis(10));
                        }
                        Err(e) => panic!("upload mock accept failed: {e}"),
                    }
                }
            });
            Self {
                base_url,
                requests,
                handle,
                shutdown,
            }
        }

        fn finish(self) -> Vec<RecordedRequest> {
            self.shutdown.store(true, Ordering::SeqCst);
            self.handle.join().unwrap();
            Arc::try_unwrap(self.requests).unwrap().into_inner().unwrap()
        }
    }

    /// Headers-only variant for bodiless GETs that treats a peer hang-up before
    /// the headers arrive as `None` instead of a panic (task 1670 issue 3: the
    /// cancelled-hydrate test hangs the daemon up while it is connecting, and on
    /// a busy CI runner that can land before the request line is sent).
    #[cfg(unix)]
    fn read_http_request_or_hangup(stream: &mut std::net::TcpStream) -> Option<RecordedRequest> {
        let mut buffer = Vec::new();
        let mut temp = [0u8; 4096];
        loop {
            let read = std::io::Read::read(stream, &mut temp).ok()?;
            if read == 0 {
                return None;
            }
            buffer.extend_from_slice(&temp[..read]);
            if buffer.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
        }
        let head = String::from_utf8_lossy(&buffer).to_string();
        let mut parts = head.lines().next()?.split_whitespace();
        let method = parts.next()?.to_string();
        let path = parts.next()?.to_string();
        Some(RecordedRequest {
            method,
            path,
            body: Vec::new(),
        })
    }

    fn read_http_request(stream: &mut std::net::TcpStream) -> RecordedRequest {
        let mut buffer = Vec::new();
        let mut temp = [0u8; 4096];
        let header_end;
        loop {
            let read = std::io::Read::read(stream, &mut temp).unwrap();
            assert!(read > 0, "mock server connection closed before headers");
            buffer.extend_from_slice(&temp[..read]);
            if let Some(pos) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
                header_end = pos + 4;
                break;
            }
        }

        let headers = String::from_utf8_lossy(&buffer[..header_end]);
        let mut lines = headers.lines();
        let request_line = lines.next().unwrap();
        let mut request_parts = request_line.split_whitespace();
        let method = request_parts.next().unwrap().to_string();
        let path = request_parts.next().unwrap().to_string();
        let content_length = headers
            .lines()
            .find_map(|line| {
                line.strip_prefix("Content-Length:")
                    .or_else(|| line.strip_prefix("content-length:"))
                    .and_then(|value| value.trim().parse::<usize>().ok())
            })
            .unwrap_or(0);

        let mut body = buffer[header_end..].to_vec();
        while body.len() < content_length {
            let read = std::io::Read::read(stream, &mut temp).unwrap();
            assert!(read > 0, "mock server connection closed before body");
            body.extend_from_slice(&temp[..read]);
        }
        body.truncate(content_length);

        RecordedRequest { method, path, body }
    }

    fn http_json(status: &str, body: serde_json::Value) -> String {
        let body = body.to_string();
        format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    /// [`http_json`] for a binary body (`application/octet-stream`).
    fn http_bytes(status: &str, body: &[u8]) -> Vec<u8> {
        let mut response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .into_bytes();
        response.extend_from_slice(body);
        response
    }

    fn upload_mock_response(request: &RecordedRequest, fail_chunk: bool, fail_thumbnails: bool) -> String {
        match (request.method.as_str(), request.path.as_str()) {
            ("POST", "/api/v1/uploads/init") => http_json(
                "200 OK",
                serde_json::json!({
                    "file_id": "server-file-1",
                    "tenant_id": "tenant-1",
                    "object_version_id": "object-init-1",
                    "upload_session_id": "upload-session-1",
                    "chunk_size_bytes": 4 * 1024 * 1024,
                    "chunk_count": 1,
                    "storage_format_version": 1,
                    "storage_pool_id": "pool-1",
                    "region": "local"
                }),
            ),
            ("PATCH", "/api/v1/files/server-file-1") => {
                http_json("200 OK", serde_json::json!({ "id": "server-file-1" }))
            }
            ("PUT", "/api/v1/uploads/upload-session-1/chunks/0") if fail_chunk => {
                http_json("500 Internal Server Error", serde_json::json!({ "error": "boom" }))
            }
            ("PUT", "/api/v1/uploads/upload-session-1/chunks/0") => http_json(
                "200 OK",
                serde_json::json!({ "index": 0, "size": request.body.len() as i64, "skipped": false }),
            ),
            ("POST", "/api/v1/uploads/upload-session-1/complete") => http_json(
                "200 OK",
                serde_json::json!({
                    "file_id": "server-file-1",
                    "version_number": 1,
                    "current_object_version_id": "object-complete-1",
                    "size_bytes": 19,
                    "mime_type": "text/plain"
                }),
            ),
            ("PUT", path) if path.starts_with("/api/v1/files/server-file-1/thumbnail") => {
                if fail_thumbnails {
                    http_json(
                        "500 Internal Server Error",
                        serde_json::json!({ "error": "thumbnail boom" }),
                    )
                } else {
                    http_json("200 OK", serde_json::json!({ "message": "thumbnail uploaded" }))
                }
            }
            _ => http_json(
                "404 Not Found",
                serde_json::json!({ "error": format!("unexpected {} {}", request.method, request.path) }),
            ),
        }
    }

    // ── hydrate_file_range (task 1024) ───────────────────────────────────────

    /// `hydrate_file_range` parses `file_id` as a UUID up front, so every
    /// hydration test needs a real one — this is an arbitrary fixed value, not
    /// tied to any real file.
    const TEST_FILE_ID: &str = "d6c090b4-69c8-437e-a61b-2023ea99ef89";

    /// Encrypt `plaintext` under `file_key` the same wire format
    /// `decrypt_downloaded_chunk` accepts (`decrypt_chunk_raw`'s
    /// `nonce(12) || ciphertext || tag(16)`), returning the raw bytes a
    /// `download_chunk` response body would carry.
    fn encrypt_chunk_wire(file_key: &beebeeb_core::kdf::FileKey, plaintext: &[u8]) -> Vec<u8> {
        let blob = beebeeb_core::encrypt::encrypt_chunk(file_key, plaintext).unwrap();
        let mut wire = blob.nonce.clone();
        wire.extend_from_slice(&blob.ciphertext);
        wire
    }

    /// Minimal mock server for hydration tests: serves `GET /api/v1/files/:id`
    /// (metadata) and `GET /api/v1/files/:id/chunks/:idx` (one encrypted chunk
    /// each) from a fixed plan, so `hydrate_file_range` can be exercised against
    /// real encrypted bytes without a live server.
    struct HydrationMockServer {
        base_url: String,
        handle: thread::JoinHandle<()>,
    }

    impl HydrationMockServer {
        /// `chunks` are PLAINTEXT chunk bodies (already split by the caller to
        /// the desired chunk_size_bytes); `size_bytes`/`chunk_count` reported by
        /// the metadata endpoint are derived from `chunks` so the test plan is
        /// internally consistent (mirrors how `plan_with_cap` lays out chunks:
        /// uniform except the last).
        fn start(file_key: beebeeb_core::kdf::FileKey, chunks: Vec<Vec<u8>>, requests: usize) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let base_url = format!("http://{}", listener.local_addr().unwrap());
            let size_bytes: usize = chunks.iter().map(|c| c.len()).sum();
            let chunk_count = chunks.len();
            let chunk_size_bytes = chunks.first().map(|c| c.len()).unwrap_or(0);
            let handle = thread::spawn(move || {
                for _ in 0..requests {
                    let (mut stream, _) = listener.accept().unwrap();
                    let request = read_http_request(&mut stream);
                    let response = hydration_mock_response(
                        &request,
                        &file_key,
                        &chunks,
                        size_bytes,
                        chunk_count,
                        chunk_size_bytes,
                    );
                    match response {
                        MockResponse::Text(body) => stream.write_all(body.as_bytes()).unwrap(),
                        MockResponse::Binary(body) => {
                            let header = format!(
                                "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                                body.len()
                            );
                            stream.write_all(header.as_bytes()).unwrap();
                            stream.write_all(&body).unwrap();
                        }
                    }
                }
            });
            Self { base_url, handle }
        }

        fn finish(self) {
            self.handle.join().unwrap();
        }
    }

    enum MockResponse {
        Text(String),
        Binary(Vec<u8>),
    }

    fn hydration_mock_response(
        request: &RecordedRequest,
        file_key: &beebeeb_core::kdf::FileKey,
        chunks: &[Vec<u8>],
        size_bytes: usize,
        chunk_count: usize,
        chunk_size_bytes: usize,
    ) -> MockResponse {
        if request.method == "GET" && request.path == format!("/api/v1/files/{TEST_FILE_ID}") {
            return MockResponse::Text(http_json(
                "200 OK",
                serde_json::json!({
                    "id": TEST_FILE_ID,
                    "size_bytes": size_bytes,
                    "chunk_count": chunk_count,
                    "chunk_size_bytes": chunk_size_bytes,
                }),
            ));
        }
        if let Some(idx_str) = request
            .path
            .strip_prefix(&format!("/api/v1/files/{TEST_FILE_ID}/chunks/"))
        {
            let idx: usize = idx_str.parse().expect("chunk index");
            let wire = encrypt_chunk_wire(file_key, &chunks[idx]);
            return MockResponse::Binary(wire);
        }
        MockResponse::Text(http_json(
            "404 Not Found",
            serde_json::json!({ "error": format!("unexpected {} {}", request.method, request.path) }),
        ))
    }

    fn hydration_test_key(master_key: [u8; 32], file_id: &str) -> beebeeb_core::kdf::FileKey {
        let mk = beebeeb_core::kdf::MasterKey::from_bytes(master_key);
        beebeeb_core::kdf::derive_file_key(&mk, file_id.as_bytes())
    }

    /// A multi-chunk file: request a range that spans exactly ONE interior
    /// chunk. `hydrate_file_range` must download+decrypt ONLY that chunk — the
    /// mock server accepts exactly 2 requests (the metadata GET + the one
    /// covering chunk GET) and would hang/fail on a third, so this test fails
    /// loudly if the whole file were downloaded instead.
    #[test]
    fn hydrate_file_range_downloads_only_covering_chunk() {
        let dir = tempfile::tempdir().unwrap();
        let master_key = [9u8; 32];
        let file_key = hydration_test_key(master_key, TEST_FILE_ID);

        // 3 chunks of 10 bytes each (uniform, no remainder) — chunk 1 is the
        // interior chunk under test.
        let chunks: Vec<Vec<u8>> = vec![b"AAAAAAAAAA".to_vec(), b"BBBBBBBBBB".to_vec(), b"CCCCCCCCCC".to_vec()];
        // 1 metadata GET + 1 chunk GET (only chunk index 1).
        let server = HydrationMockServer::start(file_key, chunks, 2);

        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_bridge_row(&bridge, TEST_FILE_ID, "/range.bin", None, FileStatus::CloudOnly, 30);

        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async { bridge.hydrate_file_range(TEST_FILE_ID, 10, 10).await })
            .unwrap()
            .expect("server-provided chunk_size_bytes should enable range hydration");

        assert_eq!(result.range_start_bytes, 10);
        assert!(!result.covers_whole_file);
        assert_eq!(&*result.data, b"BBBBBBBBBB");

        server.finish();

        // The row must NOT be marked Local from a single interior-range fetch —
        // only a whole-file covering range is evidence of full hydration.
        let entry = bridge.db.get_file(TEST_FILE_ID).unwrap().unwrap();
        assert_eq!(entry.status, FileStatus::Downloading);
    }

    /// Real desktop chunk planning is fixed-size per tier with only the last
    /// chunk short. For 9 MiB at the desktop tier the layout is 4 MiB + 4 MiB +
    /// 1 MiB, so a range inside the short final chunk must resolve to byte
    /// offset 8 MiB. The old `ceil(size_bytes / chunk_count)` derivation would
    /// incorrectly use 3 MiB and report offset 6 MiB.
    #[test]
    fn hydrate_file_range_uses_server_chunk_size_for_short_last_chunk_boundary() {
        const MIB: usize = 1024 * 1024;

        let dir = tempfile::tempdir().unwrap();
        let master_key = [12u8; 32];
        let file_key = hydration_test_key(master_key, TEST_FILE_ID);

        let mut last = vec![b'C'; MIB];
        last[123..127].copy_from_slice(b"LAST");
        let chunks: Vec<Vec<u8>> = vec![vec![b'A'; 4 * MIB], vec![b'B'; 4 * MIB], last];
        let server = HydrationMockServer::start(file_key, chunks, 2);

        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_bridge_row(
            &bridge,
            TEST_FILE_ID,
            "/short-last.bin",
            None,
            FileStatus::CloudOnly,
            9 * MIB as i64,
        );

        let required_offset = (8 * MIB + 123) as u64;
        let required_length = 4u64;
        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async {
                bridge
                    .hydrate_file_range(TEST_FILE_ID, required_offset, required_length)
                    .await
            })
            .unwrap()
            .expect("server-provided chunk_size_bytes should enable range hydration");

        assert_eq!(result.range_start_bytes, (8 * MIB) as u64);
        assert!(!result.covers_whole_file);
        let relative_start = (required_offset - result.range_start_bytes) as usize;
        let relative_end = relative_start + required_length as usize;
        assert_eq!(&result.data[relative_start..relative_end], b"LAST");

        server.finish();
    }

    #[test]
    fn hydration_range_plan_uses_authoritative_chunk_size_for_short_last_chunk() {
        const MIB: u64 = 1024 * 1024;

        let plan = plan_hydration_chunk_range(9 * MIB, 3, 4 * MIB, 8 * MIB + 123, 4)
            .unwrap()
            .expect("valid server chunk size should produce a range plan");

        assert_eq!(plan.first_chunk, 2);
        assert_eq!(plan.last_chunk_exclusive, 3);
        assert_eq!(plan.range_start_bytes, 8 * MIB);
        assert_eq!(plan.covering_span_hint, 4 * MIB as usize);
        assert!(!plan.covers_whole_file);
        assert_ne!(
            plan.range_start_bytes,
            2 * (9 * MIB).div_ceil(3),
            "regression guard: ceil(size/chunk_count) would pick the wrong base offset"
        );
    }

    /// A range request that happens to cover the WHOLE file (offset 0, length
    /// >= size) — this is the `covers_whole_file` branch, which mirrors
    /// `hydrate_file_to_memory`'s bookkeeping: mark `Local` + `mark_cached`.
    #[test]
    fn hydrate_file_range_covering_whole_file_marks_local() {
        let dir = tempfile::tempdir().unwrap();
        let master_key = [11u8; 32];
        let file_key = hydration_test_key(master_key, TEST_FILE_ID);

        let chunks: Vec<Vec<u8>> = vec![b"HELLOWORLD".to_vec()]; // 1 chunk, 10 bytes
        let server = HydrationMockServer::start(file_key, chunks, 2);

        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_bridge_row(&bridge, TEST_FILE_ID, "/whole.bin", None, FileStatus::CloudOnly, 10);

        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async { bridge.hydrate_file_range(TEST_FILE_ID, 0, 10).await })
            .unwrap()
            .unwrap();

        assert!(result.covers_whole_file);
        assert_eq!(&*result.data, b"HELLOWORLD");

        server.finish();

        let entry = bridge.db.get_file(TEST_FILE_ID).unwrap().unwrap();
        assert_eq!(entry.status, FileStatus::Local);
    }

    /// Metadata missing `chunk_count` (or it's `0`) must fall back cleanly —
    /// `Ok(None)`, never an error — so the caller can retry via the whole-file
    /// `hydrate_file_to_memory` path.
    #[test]
    fn hydrate_file_range_falls_back_when_chunk_count_absent() {
        let dir = tempfile::tempdir().unwrap();
        let master_key = [3u8; 32];
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let _ = read_http_request(&mut stream);
            let body = http_json(
                "200 OK",
                serde_json::json!({ "id": TEST_FILE_ID, "size_bytes": 100 }), // no chunk_count
            );
            stream.write_all(body.as_bytes()).unwrap();
        });

        let bridge = test_bridge_with_api(&dir.path().join("state.db"), base_url, master_key);
        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async { bridge.hydrate_file_range(TEST_FILE_ID, 0, 10).await })
            .unwrap();
        assert!(result.is_none());

        handle.join().unwrap();
    }

    #[test]
    fn hydrate_file_range_falls_back_when_chunk_size_absent_even_if_size_and_count_present() {
        let dir = tempfile::tempdir().unwrap();
        let master_key = [4u8; 32];
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let _ = read_http_request(&mut stream);
            let body = http_json(
                "200 OK",
                serde_json::json!({
                    "id": TEST_FILE_ID,
                    "size_bytes": 24,
                    "chunk_count": 3,
                }),
            );
            stream.write_all(body.as_bytes()).unwrap();
        });

        let bridge = test_bridge_with_api(&dir.path().join("state.db"), base_url, master_key);
        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async { bridge.hydrate_file_range(TEST_FILE_ID, 0, 10).await })
            .unwrap();
        assert!(result.is_none());

        handle.join().unwrap();
    }

    /// R4 (task 1834 fix round 2), and the move of the macOS staging root (spec §8.4): the directories the
    /// account reset and the sign-out purge sweep are exactly the places the engine stages into now and staged
    /// into before. On macOS: the data root, then the cache root and the temp root that earlier builds used.
    /// Elsewhere: the cache root and its temp fallback. The one `default_finder_staging_root()` picks is always
    /// among them.
    #[test]
    fn the_swept_staging_directories_are_the_ones_the_engine_stages_into() {
        let bases = FinderStagingBases::current();
        let candidates = finder_staging_candidates();
        #[cfg(target_os = "macos")]
        let expected = vec![
            bases.durable_root().expect("the test sandbox has a data dir"),
            bases.cache_root(),
            bases.temp_root(),
        ];
        #[cfg(not(target_os = "macos"))]
        let expected = vec![bases.cache_root(), bases.temp_root()];
        assert_eq!(candidates, expected);
        assert!(
            candidates.contains(&default_finder_staging_root().expect("the sandbox root is writable")),
            "the engine stages somewhere the sweep covers"
        );
    }

    /// Spec §8.4, the staging root (macOS): `<data dir>/beebeeb/finder-writes`, never under the cache dir or the
    /// temp dir, the system may purge either. The folder is private: mode 0700, and excluded from backups on
    /// macOS (it holds plaintext). Without a data dir, or with a root that cannot be created or written to, it is
    /// the typed refusal, and nothing is created in the cache or temp dir instead.
    #[test]
    fn the_staging_root_is_under_the_data_dir_never_the_cache_or_temp_dir() {
        let base = tempfile::tempdir().unwrap();
        let (data, cache, temp) = (
            base.path().join("data"),
            base.path().join("cache"),
            base.path().join("temp"),
        );
        let bases = FinderStagingBases {
            data: Some(data.clone()),
            cache: Some(cache.clone()),
            temp: temp.clone(),
        };
        let root = durable_finder_staging_root(&bases).expect("a writable data dir takes the copy");
        assert_eq!(root, data.join("beebeeb").join("finder-writes"), "under the data dir");
        assert!(
            !root.starts_with(&cache) && !root.starts_with(&temp),
            "never under the cache or temp dir: {root:?}"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&root).unwrap().permissions().mode() & 0o777,
                0o700,
                "owner-only: it holds plaintext"
            );
        }
        #[cfg(target_os = "macos")]
        {
            use std::os::unix::ffi::OsStrExt;
            let attr = std::ffi::CString::new("com.apple.metadata:com_apple_backup_excludeItem").unwrap();
            let path = std::ffi::CString::new(root.as_os_str().as_bytes()).unwrap();
            let mut buf = vec![0u8; 64];
            // SAFETY: both strings are NUL-terminated and live for the call; `buf` is ours, with its length.
            let n = unsafe {
                libc::getxattr(
                    path.as_ptr(),
                    attr.as_ptr(),
                    buf.as_mut_ptr() as *mut libc::c_void,
                    buf.len(),
                    0,
                    0,
                )
            };
            assert!(n > 0, "excluded from backups (getxattr returned {n})");
            assert_eq!(&buf[..n as usize], b"com.apple.backupd");
        }
        assert_eq!(
            std::fs::read_dir(&root).unwrap().count(),
            0,
            "the write check leaves nothing behind"
        );

        // No data dir: refused, with no fallback.
        let no_data = FinderStagingBases {
            data: None,
            ..bases.clone()
        };
        assert_eq!(
            durable_finder_staging_root(&no_data),
            Err(FinderStagingUnavailable(None))
        );

        // A root that cannot be created: a file where its parent folder would be.
        let blocked = base.path().join("blocked");
        std::fs::create_dir(&blocked).unwrap();
        std::fs::write(blocked.join("beebeeb"), b"in the way").unwrap();
        let refused = durable_finder_staging_root(&FinderStagingBases {
            data: Some(blocked),
            ..bases.clone()
        })
        .expect_err("a root that cannot be created is refused");
        assert!(refused.0.is_some(), "the refusal names the error kind: {refused:?}");
        assert!(
            refused.to_string().starts_with("the staging folder is unavailable") && !refused.to_string().contains('/'),
            "a fixed reason, no path: {refused}"
        );

        assert!(
            !cache.exists() && !temp.exists(),
            "nothing was created in the cache or temp dir"
        );
    }

    #[test]
    fn unit_tests_stage_finder_writes_in_a_sandbox_never_the_real_cache() {
        // `default_finder_staging_root()` is where Finder create/modify/keep-mine copy the plaintext
        // they are about to upload. The INSTALLED app uses the real `<cache dir>/beebeeb/finder-writes`
        // on this machine, so a test build must resolve to a per-process sandbox next to the test
        // binary: otherwise every test that queues a Finder write drops plaintext files there.
        // Only paths are computed (and the sandbox dir is created); the real dir is not touched.
        let root = default_finder_staging_root().expect("the sandbox root is writable");
        // The specific real directories, not the whole cache or data dir: a `CARGO_TARGET_DIR` under
        // `~/Library/Caches` (or `~/.cache`) puts the sandbox inside the cache dir legitimately.
        let real_staging = dirs::cache_dir()
            .expect("a user cache dir exists on a dev machine or CI runner")
            .join("beebeeb/finder-writes");
        assert!(
            !root.starts_with(&real_staging),
            "{root:?} is inside the real staging dir {real_staging:?}"
        );
        if let Some(data) = dirs::data_dir() {
            let real_durable = data.join("beebeeb/finder-writes");
            assert!(
                !root.starts_with(&real_durable),
                "{root:?} is inside the real staging dir {real_durable:?}"
            );
        }
        let exe_dir = std::env::current_exe()
            .expect("test binary path")
            .parent()
            .expect("exe dir")
            .to_path_buf();
        assert!(
            root.starts_with(&exe_dir),
            "{root:?} is not inside the test binary's directory {exe_dir:?}"
        );
        assert!(
            root.ends_with("beebeeb/finder-writes"),
            "the tail of the staging path is unchanged: {root:?}"
        );
    }

    #[test]
    fn test_create_file_queues_upload_version_and_preserves_payload() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("report.txt");
        std::fs::write(&source, b"queued payload").unwrap();
        let bridge = test_bridge(&dir.path().join("state.db"));

        let outcome = bridge
            .queue_finder_create(FinderWriteTarget {
                file_id: None,
                parent_id: None,
                filename: "report.txt".into(),
                rel_path: None,
                kind: FinderWriteItemKind::File,
                contents_path: Some(source.to_string_lossy().into_owned()),
                content_type: Some("text/plain".into()),
                base_version_identifier: None,
            })
            .unwrap();

        let FinderWriteOutcome::Queued {
            file_id: Some(file_id),
            kind,
            ..
        } = outcome
        else {
            panic!("expected queued file upload");
        };
        assert_eq!(kind, OperationKind::UploadVersion);

        let queued = bridge.db.list_due_operations(now_secs()).unwrap();
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].kind, OperationKind::UploadVersion);
        assert_eq!(queued[0].file_id.as_deref(), Some(file_id.as_str()));
        let payload_path = queued[0].payload_path.as_deref().unwrap();
        assert_eq!(std::fs::read(payload_path).unwrap(), b"queued payload");

        let metadata: serde_json::Value = serde_json::from_str(queued[0].metadata_json.as_deref().unwrap()).unwrap();
        assert_eq!(metadata["operation"], "create_file");
        assert!(metadata["name_encrypted"].as_str().unwrap().starts_with('{'));
        assert_eq!(
            bridge.db.get_file(&file_id).unwrap().unwrap().status,
            FileStatus::Uploading
        );
    }

    #[test]
    fn test_metadata_modify_does_not_queue_content_version() {
        let dir = tempfile::tempdir().unwrap();
        let bridge = test_bridge(&dir.path().join("state.db"));

        let outcome = bridge
            .queue_finder_modify(FinderWriteTarget {
                file_id: Some("file-1".into()),
                parent_id: Some("folder-1".into()),
                filename: "renamed.txt".into(),
                rel_path: None,
                kind: FinderWriteItemKind::File,
                contents_path: None,
                content_type: Some("text/plain".into()),
                base_version_identifier: Some("4:1700000000:12".into()),
            })
            .unwrap();
        let FinderWriteOutcome::Queued { kind, .. } = outcome else {
            panic!("expected queued metadata op");
        };
        assert_eq!(kind, OperationKind::MoveFile);

        let queued = bridge.db.list_due_operations(now_secs()).unwrap();
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].kind, OperationKind::MoveFile);
        assert!(queued[0].payload_path.is_none());
        let metadata: serde_json::Value = serde_json::from_str(queued[0].metadata_json.as_deref().unwrap()).unwrap();
        assert_eq!(metadata["operation"], "metadata_update");
        assert!(metadata["name_encrypted"].as_str().unwrap().starts_with('{'));
    }

    #[test]
    fn test_delete_maps_to_trash_operation() {
        let dir = tempfile::tempdir().unwrap();
        let bridge = test_bridge(&dir.path().join("state.db"));
        // Task 1698: an UNKNOWN item is now an idempotent Ignored (ruling:
        // "unknown items report success") — a delete needs a real row.
        seed_bridge_entry(&bridge, "file-1", "file-1.txt", None, FileStatus::Local, false, 10);

        let outcome = bridge
            .queue_finder_delete("file-1", Some("9:1700000000:100".into()))
            .unwrap();
        let FinderWriteOutcome::Queued { kind, .. } = outcome else {
            panic!("expected queued trash op");
        };
        assert_eq!(kind, OperationKind::TrashFile);

        let queued = bridge.db.list_due_operations(now_secs()).unwrap();
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].kind, OperationKind::TrashFile);
        assert_eq!(queued[0].base_version, Some(9));
        assert!(queued[0].payload_path.is_none());
    }

    #[test]
    fn record_moved_to_trash_activity_writes_deletion_event() {
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();

        record_moved_to_trash_activity(&db, "server-file-1", "docs/doomed.txt", 200).unwrap();

        let events = db.list_recent_local_activity(5).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, LocalActivityKind::MovedToTrash);
        assert_eq!(events[0].file_id.as_deref(), Some("server-file-1"));
        assert_eq!(events[0].file_name, "doomed.txt");
        assert_eq!(events[0].rel_path.as_deref(), Some("docs/doomed.txt"));
        assert_eq!(events[0].occurred_at, 200);
    }

    #[tokio::test]
    async fn test_process_due_operations_trash_calls_api_and_keeps_row_trashing() {
        // task 0802 (server-authoritative deletion): a locally-deleted file is
        // parked in `Trashing` and its `TrashFile` op enqueued. Processing that op
        // must (a) issue the server trash DELETE and (b) on success DROP THE OP but
        // KEEP the row in `Trashing`. The `Trashing` status — not the op — is now
        // the durable hidden-locally marker; convergence (row removal) happens
        // authoritatively once the trash propagates out of `/sync/snapshot`
        // (`prune_absent`) or the `file_trash` op echo arrives (`apply_sync_op`).
        // Deleting the row HERE is what caused the "deleted file comes back" race
        // (a same-tick lagged snapshot would re-insert it as CloudOnly).
        let dir = tempfile::tempdir().unwrap();
        let server = SyncMockServer::start(vec![("200 OK".into(), serde_json::json!({ "ok": true }))]);
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), [9u8; 32]);

        // Row in the post-local-delete `Trashing` state + a queued trash op.
        bridge
            .db
            .upsert_file(&FileEntry {
                file_id: "server-file-1".into(),
                path: "doomed.txt".into(),
                status: FileStatus::Trashing,
                size_bytes: 5,
                modified_at: 0,
                content_hash: None,
                remote_updated_at: 0,
                parent_id: None,
                item_kind: ItemKind::File,
            })
            .unwrap();
        bridge
            .db
            .enqueue_operation(&PendingOperation {
                op_id: "op-trash-1".into(),
                kind: OperationKind::TrashFile,
                file_id: Some("server-file-1".into()),
                parent_id: None,
                target_path: None,
                metadata_json: Some(serde_json::json!({ "operation": "trash" }).to_string()),
                payload_path: None,
                base_version: None,
                base_object_version_id: None,
                attempts: 0,
                max_attempts: 25,
                next_retry_at: 0,
                last_error: None,
                backup_source_key: None,
                created_at: 100,
                updated_at: 100,
            })
            .unwrap();

        let outcome = bridge.process_due_operations(dir.path(), 200).await.unwrap();
        assert_eq!(outcome.completed_op_ids, vec!["op-trash-1".to_string()]);
        assert!(outcome.retried_op_ids.is_empty());

        // The op is gone (success removes it) but the row REMAINS in `Trashing` —
        // the durable marker. The seeder only re-mints `CloudOnly`, so a `Trashing`
        // row never re-creates the placeholder; the row itself is removed later by
        // prune/op-echo once the server trash propagates.
        assert!(bridge.db.list_due_operations(999).unwrap().is_empty());
        let row = bridge.db.get_file("server-file-1").unwrap();
        assert!(row.is_some(), "trash op must KEEP the row on success (not delete it)");
        assert_eq!(
            row.unwrap().status,
            FileStatus::Trashing,
            "row stays Trashing after a successful trash — the durable hidden-locally marker"
        );
        let events = bridge.db.list_recent_local_activity(5).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, crate::state_db::LocalActivityKind::MovedToTrash);
        assert_eq!(events[0].file_id.as_deref(), Some("server-file-1"));
        assert_eq!(events[0].file_name, "doomed.txt");
        assert_eq!(events[0].rel_path.as_deref(), Some("doomed.txt"));
        assert_eq!(events[0].occurred_at, 200);

        let requests = server.finish();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method, "DELETE");
        assert_eq!(requests[0].path, "/api/v1/files/server-file-1");
    }

    #[tokio::test]
    async fn test_process_due_operations_trash_retries_and_keeps_row_on_server_error() {
        // If the server trash fails, the op must be retried (not dropped) and the
        // row must be PRESERVED — recoverable rather than lost. (The row is left
        // in `Trashing`, so the placeholder still does not reappear meanwhile.)
        let dir = tempfile::tempdir().unwrap();
        let server = SyncMockServer::start(vec![(
            "500 Internal Server Error".into(),
            serde_json::json!({ "error": "boom" }),
        )]);
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), [9u8; 32]);

        bridge
            .db
            .upsert_file(&FileEntry {
                file_id: "server-file-2".into(),
                path: "doomed2.txt".into(),
                status: FileStatus::Trashing,
                size_bytes: 5,
                modified_at: 0,
                content_hash: None,
                remote_updated_at: 0,
                parent_id: None,
                item_kind: ItemKind::File,
            })
            .unwrap();
        bridge
            .db
            .enqueue_operation(&PendingOperation {
                op_id: "op-trash-2".into(),
                kind: OperationKind::TrashFile,
                file_id: Some("server-file-2".into()),
                parent_id: None,
                target_path: None,
                metadata_json: Some(serde_json::json!({ "operation": "trash" }).to_string()),
                payload_path: None,
                base_version: None,
                base_object_version_id: None,
                attempts: 0,
                max_attempts: 25,
                next_retry_at: 0,
                last_error: None,
                backup_source_key: None,
                created_at: 100,
                updated_at: 100,
            })
            .unwrap();

        let outcome = bridge.process_due_operations(dir.path(), 200).await.unwrap();
        assert!(outcome.completed_op_ids.is_empty());
        assert_eq!(outcome.retried_op_ids, vec!["op-trash-2".to_string()]);
        // Row preserved (still Trashing) so a future retry can complete and the
        // file is recoverable from the server in the meantime.
        let row = bridge.db.get_file("server-file-2").unwrap().unwrap();
        assert_eq!(row.status, FileStatus::Trashing);

        let _ = server.finish();
    }

    #[test]
    fn test_recursive_pin_queues_hydration_for_cloud_only_descendants() {
        let dir = tempfile::tempdir().unwrap();
        let bridge = test_bridge(&dir.path().join("state.db"));
        seed_bridge_row(&bridge, "folder-a", "/Projects", None, FileStatus::CloudOnly, 0);
        seed_bridge_row(
            &bridge,
            "file-a",
            "/Projects/report.txt",
            Some("folder-a"),
            FileStatus::CloudOnly,
            128,
        );
        seed_bridge_row(
            &bridge,
            "file-local",
            "/Projects/local.txt",
            Some("folder-a"),
            FileStatus::Local,
            256,
        );

        // `sync_root` is only used on Windows (to resolve placeholder paths for
        // CfSetPinState). On the non-Windows test host the hydrate-enqueue path
        // runs and `sync_root` is unused, so any path is fine here.
        let queued = bridge.set_recursive_pin(dir.path(), "folder-a", true).unwrap();
        assert_eq!(queued.changed_item_ids.len(), 3);
        // hydrate_operations is the non-Windows materialise path; on Windows the
        // OS (CfSetPinState) hydrates pinned files, so this count is 0 there.
        #[cfg(not(target_os = "windows"))]
        assert_eq!(queued.hydrate_operations, 1);

        let due = bridge.db.list_due_operations(now_secs()).unwrap();
        assert!(due.iter().any(|op| {
            op.kind == OperationKind::PinTree
                && op.file_id.as_deref() == Some("folder-a")
                && op.metadata_json.as_deref().unwrap_or("").contains("\"pinned\":true")
        }));
        // On Windows the pin path calls CfSetPinState; the OS drives hydration
        // via FETCH_DATA rather than us enqueuing HydrateFile ops, so no
        // HydrateFile entries exist in the queue on that platform.
        #[cfg(not(target_os = "windows"))]
        {
            assert!(
                due.iter()
                    .any(|op| { op.kind == OperationKind::HydrateFile && op.file_id.as_deref() == Some("file-a") })
            );
            assert!(
                !due.iter()
                    .any(|op| { op.kind == OperationKind::HydrateFile && op.file_id.as_deref() == Some("file-local") })
            );
        }
    }

    #[test]
    fn test_smart_cache_cleanup_never_evicts_effectively_pinned_files() {
        let dir = tempfile::tempdir().unwrap();
        let bridge = test_bridge(&dir.path().join("state.db"));
        seed_bridge_row(&bridge, "pinned", "/Pinned.txt", None, FileStatus::Local, 700);
        seed_bridge_row(&bridge, "unpinned", "/Unpinned.txt", None, FileStatus::Local, 600);
        bridge.db.set_recursive_pin("pinned", true, 1).unwrap();
        bridge.db.mark_cached("pinned", "/cache/pinned", 700, 10).unwrap();
        bridge.db.mark_cached("unpinned", "/cache/unpinned", 600, 20).unwrap();

        let evicted = bridge
            .enforce_smart_cache(CachePolicy {
                max_unpinned_cache_bytes: 500,
                disk_pressure_min_free_bytes: 0,
            })
            .unwrap();

        assert_eq!(evicted.evicted_file_ids, vec!["unpinned".to_string()]);
        assert_eq!(bridge.db.get_file("pinned").unwrap().unwrap().status, FileStatus::Local);
    }

    #[test]
    fn test_local_cache_limit_counts_pinned_bytes_against_total_cap() {
        let dir = tempfile::tempdir().unwrap();
        let bridge = test_bridge(&dir.path().join("state.db"));
        seed_bridge_row(&bridge, "pinned", "/Pinned.txt", None, FileStatus::Local, 700);
        seed_bridge_row(&bridge, "old", "/Old.txt", None, FileStatus::Local, 400);
        seed_bridge_row(&bridge, "new", "/New.txt", None, FileStatus::Local, 300);

        bridge.db.set_recursive_pin("pinned", true, 1).unwrap();
        bridge.db.mark_cached("pinned", "/cache/pinned", 700, 10).unwrap();
        bridge.db.mark_cached("old", "/cache/old", 400, 20).unwrap();
        bridge.db.mark_cached("new", "/cache/new", 300, 30).unwrap();

        let evicted = bridge.enforce_local_cache_limit(Some(1_000)).unwrap();

        assert_eq!(evicted.evicted_file_ids, vec!["old".to_string()]);
        assert_eq!(bridge.db.get_file("pinned").unwrap().unwrap().status, FileStatus::Local);
        assert_eq!(bridge.db.get_file("new").unwrap().unwrap().status, FileStatus::Local);
        assert_eq!(
            bridge.db.get_file("old").unwrap().unwrap().status,
            FileStatus::CloudOnly
        );
    }

    #[test]
    fn test_local_cache_usage_bytes_sums_pinned_and_unpinned_cache() {
        let dir = tempfile::tempdir().unwrap();
        let bridge = test_bridge(&dir.path().join("state.db"));
        seed_bridge_row(&bridge, "pinned", "/Pinned.txt", None, FileStatus::Local, 700);
        seed_bridge_row(&bridge, "unpinned", "/Unpinned.txt", None, FileStatus::Local, 600);

        bridge.db.set_recursive_pin("pinned", true, 1).unwrap();
        bridge.db.mark_cached("pinned", "/cache/pinned", 700, 10).unwrap();
        bridge.db.mark_cached("unpinned", "/cache/unpinned", 600, 20).unwrap();

        assert_eq!(bridge.local_cache_usage_bytes().unwrap(), 1_300);
    }

    #[test]
    fn test_shared_invite_mapping_filters_approved_and_permissions() {
        let body = serde_json::json!({
            "invites": [
                {
                    "id": "invite-read",
                    "file_id": "root-read",
                    "status": "approved",
                    "display_name": "Client dropbox",
                    "is_folder_share": true,
                    "can_reshare": true
                },
                {
                    "id": "invite-write",
                    "file_id": "root-write",
                    "status": "approved",
                    "decrypted_name": "Editorial",
                    "is_folder": true,
                    "capability": "write"
                },
                {
                    "id": "invite-pending",
                    "file_id": "root-pending",
                    "status": "claimed"
                }
            ]
        });

        let roots = shared_roots_from_invite_response(&body);
        assert_eq!(roots.len(), 2);
        assert_eq!(roots[0].display_name, "Encrypted folder");
        assert_eq!(roots[0].permission_bits, PERMISSION_READ | PERMISSION_SHARE);
        assert_eq!(roots[1].permission_bits, PERMISSION_READ | PERMISSION_WRITE);
    }

    #[test]
    fn shared_invite_id_match_does_not_match_same_root_wrong_invite() {
        let wrong_invite_same_root = serde_json::json!({
            "id": "invite-wrong",
            "file_id": "root-folder",
            "status": "approved"
        });
        let right_invite_different_root = serde_json::json!({
            "invite_id": "invite-target",
            "file_id": "other-root",
            "status": "approved"
        });

        assert!(!shared_invite_id_matches(&wrong_invite_same_root, "invite-target"));
        assert!(shared_invite_id_matches(&right_invite_different_root, "invite-target"));
    }

    fn fixed_aes_gcm_frame(key: &[u8; 32], nonce_byte: u8, plaintext: &[u8]) -> Vec<u8> {
        use aes_gcm::aead::Aead;
        use aes_gcm::{Aes256Gcm, KeyInit, Nonce};

        let cipher = Aes256Gcm::new_from_slice(key).unwrap();
        let nonce_bytes = [nonce_byte; 12];
        let mut out = nonce_bytes.to_vec();
        let ciphertext = cipher.encrypt(Nonce::from_slice(&nonce_bytes), plaintext).unwrap();
        out.extend_from_slice(&ciphertext);
        out
    }

    #[test]
    fn unwrap_folder_share_file_key_fixture_decrypts_content_chunk() {
        let owner_master = [0x11u8; 32];
        let recipient_master = [0x22u8; 32];
        let folder_id = "0bb0f451-986d-43e0-bbef-f1e8acb27a01";
        let child_id = "4a823d35-3ae2-4fc7-8266-762096405bc7";
        let folder_key = [0x33u8; 32];
        let child_file_key = [0x44u8; 32];
        let plaintext = b"shared plaintext fixture";

        let owner_mk = beebeeb_core::kdf::MasterKey::from_bytes(owner_master);
        let owner_private = beebeeb_core::opaque::derive_x25519_private(&owner_mk);
        let owner_public = beebeeb_core::opaque::derive_x25519_public(&owner_private);
        let recipient_mk = beebeeb_core::kdf::MasterKey::from_bytes(recipient_master);
        let recipient_private = beebeeb_core::opaque::derive_x25519_private(&recipient_mk);
        let recipient_public = beebeeb_core::opaque::derive_x25519_public(&recipient_private);
        let shared_secret = beebeeb_core::opaque::x25519_shared_secret(&owner_private, &recipient_public).unwrap();
        let share_key = beebeeb_core::opaque::derive_share_key(&shared_secret, folder_id.as_bytes());

        let encrypted_folder_key =
            base64::engine::general_purpose::STANDARD.encode(fixed_aes_gcm_frame(&share_key, 0xa1, &folder_key));
        let encrypted_child_file_key =
            base64::engine::general_purpose::STANDARD.encode(fixed_aes_gcm_frame(&folder_key, 0xb2, &child_file_key));
        let content_wire = fixed_aes_gcm_frame(&child_file_key, 0xc3, plaintext);

        let unwrapped = unwrap_folder_share_file_key(
            &recipient_master,
            &base64::engine::general_purpose::STANDARD.encode(owner_public),
            folder_id,
            &encrypted_folder_key,
            child_id,
            &serde_json::json!({
                "keys": [{ "file_id": child_id, "encrypted_file_key": encrypted_child_file_key }]
            }),
        )
        .unwrap();

        assert_eq!(unwrapped.as_bytes(), &child_file_key);
        assert_eq!(decrypt_downloaded_chunk(&unwrapped, &content_wire).unwrap(), plaintext);
    }

    #[test]
    fn unwrap_folder_share_file_key_web_generated_vector_decrypts_child_key() {
        // Generated from repos/web/src/lib/folder-share-crypto.ts using the real
        // encryptFolderKeyForRecipient/encryptChildFileKey functions with
        // fixed inputs and beebeeb-wasm backing their crypto worker proxy.
        let recipient_master = [0x22u8; 32];
        let folder_id = "0bb0f451-986d-43e0-bbef-f1e8acb27a01";
        let child_id = "4a823d35-3ae2-4fc7-8266-762096405bc7";
        let owner_public_key = "zr4yc0fCIWlQgKPrOGWQG25jB3Kbm13rDDezBBz61Uc=";
        let encrypted_folder_key = "PVZ8a7JfzaV/m2p53h33xsIHsXtwaFmfcttbly6GoKnRVlnh8V5LbXiwCGFLbZhNLskOuwJX0NJQ3UzJ";
        let encrypted_child_file_key =
            "xbKO8C1APBQytc8VqT0mfT5oSaP+8h02CWObRL2kegAOsBbzFE6N4ZLfRQy2m63bLsPBjmNXLEUaHlPu";

        let folder_key =
            unwrap_folder_share_key(&recipient_master, owner_public_key, folder_id, encrypted_folder_key).unwrap();
        assert_eq!(&*folder_key, &[0x33u8; 32]);

        let child_key = unwrap_folder_share_file_key(
            &recipient_master,
            owner_public_key,
            folder_id,
            encrypted_folder_key,
            child_id,
            &serde_json::json!({
                "keys": [{ "file_id": child_id, "encrypted_file_key": encrypted_child_file_key }]
            }),
        )
        .unwrap();

        assert_eq!(child_key.as_bytes(), &[0x44u8; 32]);
    }

    #[test]
    fn decrypt_shared_name_with_key_rejects_unauthenticated_plaintext_name() {
        let file_key = beebeeb_core::kdf::FileKey::from_bytes([0x55u8; 32]);

        assert_eq!(
            decrypt_shared_name_with_key(&file_key, "../../../../.ssh/authorized_keys"),
            None
        );
    }

    #[test]
    fn decrypt_shared_name_with_key_accepts_authenticated_metadata_name() {
        let file_key = beebeeb_core::kdf::FileKey::from_bytes([0x55u8; 32]);
        let plaintext = serde_json::json!({
            "name": "Quarterly report.txt",
            "mime_type": "text/plain"
        })
        .to_string();
        let name_encrypted =
            serde_json::to_string(&beebeeb_core::encrypt::encrypt_metadata(&file_key, &plaintext).unwrap()).unwrap();

        assert_eq!(
            decrypt_shared_name_with_key(&file_key, &name_encrypted),
            Some("Quarterly report.txt".to_string())
        );
    }

    #[test]
    fn shared_metadata_ignores_raw_server_name_fields_when_decrypt_fails() {
        let file_key = beebeeb_core::kdf::FileKey::from_bytes([0x55u8; 32]);
        let metadata = serde_json::json!({
            "id": "child-file",
            "name_encrypted": "not authenticated ciphertext",
            "display_name": "../../../../.ssh/authorized_keys",
            "decrypted_name": "server-claimed.txt",
            "path": "nested/server-claimed.txt"
        });

        assert_eq!(
            shared_name_from_metadata(&metadata, &ItemKind::File, Some(&file_key)),
            "Encrypted file"
        );
    }

    #[test]
    fn shared_metadata_uses_placeholder_for_authenticated_traversal_name() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = StateDb::open(dir.path().join("state.db")).expect("state db");
        let root = SharedRootMapping {
            invite_id: "invite-folder".into(),
            file_id: "root-folder".into(),
            display_name: "Shared folder".into(),
            is_folder: true,
            size_bytes: 0,
            content_type: None,
            owner_email: Some("owner@example.com".into()),
            sender_public_key: None,
            encrypted_file_key: None,
            encrypted_folder_key: None,
            file_name_encrypted: None,
            permission_bits: PERMISSION_READ,
            approved_at: None,
        };
        let file_key = beebeeb_core::kdf::FileKey::from_bytes([0x66u8; 32]);
        let malicious_name = serde_json::json!({
            "name": "../../../../.ssh/authorized_keys",
            "mime_type": "text/plain"
        })
        .to_string();
        let name_encrypted =
            serde_json::to_string(&beebeeb_core::encrypt::encrypt_metadata(&file_key, &malicious_name).unwrap())
                .unwrap();

        let entry = apply_shared_metadata_file_row(
            &db,
            &serde_json::json!({
                "id": "child-file",
                "name_encrypted": name_encrypted,
                "size_bytes": 12,
                "is_folder": false,
            }),
            &root,
            1_700_000_000,
            Some(root.file_id.clone()),
            "Shared with me/Shared folder",
            Some(&file_key),
        )
        .expect("apply shared metadata")
        .expect("entry");

        assert_eq!(entry.path, "Shared with me/Shared folder/Encrypted file");
        crate::reject_unsafe_rel_path(&entry.path).expect("shared path must stay safe");
    }

    #[test]
    fn shared_hydrate_write_guard_rejects_persisted_traversal_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        let bridge = test_bridge(&dir.path().join("state.db"));
        bridge
            .db
            .upsert_file(&FileEntry {
                file_id: "malicious-shared-file".into(),
                path: "Shared with me/folder/../../outside.txt".into(),
                status: FileStatus::CloudOnly,
                size_bytes: 12,
                modified_at: 0,
                content_hash: None,
                remote_updated_at: 0,
                parent_id: None,
                item_kind: ItemKind::File,
            })
            .unwrap();
        let mut contract = bridge
            .db
            .get_file_contract_state("malicious-shared-file")
            .unwrap()
            .unwrap();
        contract.namespace = Namespace::SharedWithMe;
        contract.shared_root_id = Some("root-folder".into());
        contract.share_id = Some("invite-folder".into());
        contract.permission_bits = PERMISSION_READ;
        bridge.db.set_file_contract_state(&contract).unwrap();

        let err = bridge
            .ensure_shared_hydrate_path_safe("malicious-shared-file")
            .unwrap_err()
            .to_string();

        assert!(err.contains("unsafe shared hydrate path"));
        assert!(err.contains(".."));
    }

    #[test]
    fn test_operation_error_classification_and_backoff() {
        assert_eq!(
            classify_operation_error("401 unauthorized Bearer token"),
            OperationFailureClass::Auth
        );
        assert_eq!(classify_operation_error("quota exceeded"), OperationFailureClass::Quota);
        assert_eq!(
            classify_operation_error("403 forbidden permission denied"),
            OperationFailureClass::Permission
        );
        assert_eq!(
            classify_operation_error("vault locked, unlock required"),
            OperationFailureClass::Locked
        );
        assert_eq!(
            classify_operation_error("connection reset by peer"),
            OperationFailureClass::Retryable
        );
        assert_eq!(retry_delay_seconds(1), 30);
        assert_eq!(retry_delay_seconds(2), 60);
        assert_eq!(retry_delay_seconds(10), 960);
    }

    #[test]
    fn test_classify_review_operation_recognizes_expired_session_as_auth_failure() {
        // Task 1546 finding 3: an expired-session (401) upload failure fell
        // through to the generic `failed_upload` bucket with the raw HTTP
        // error text and no "sign in again" path. `classify_review_operation`
        // is the function VersionCenter's list actually reads (NOT
        // `classify_operation_error`, which already classified this
        // correctly for backoff purposes but was never consulted here).
        let op = PendingOperation {
            op_id: "op-auth-1".into(),
            kind: OperationKind::UploadVersion,
            file_id: Some("server-file-1".into()),
            parent_id: None,
            target_path: Some("Docs/notes.txt".into()),
            metadata_json: None,
            payload_path: None,
            base_version: None,
            base_object_version_id: None,
            attempts: 1,
            max_attempts: 25,
            next_retry_at: 0,
            last_error: Some(
                "HTTP status client error (401 Unauthorized) for url (http://127.0.0.1:8080/api/v1/uploads/init)"
                    .to_string(),
            ),
            backup_source_key: None,
            created_at: 100,
            updated_at: 100,
        };

        let (kind, _status, detail, action) = classify_review_operation(&op);
        assert_eq!(kind, "auth_failure");
        assert_eq!(action, "sign_in_again");
        assert!(
            detail.to_ascii_lowercase().contains("sign in"),
            "detail should tell the user to sign in again, got: {detail}"
        );

        // A completely unrelated failure (no 401/unauthorized/invalid-token
        // substring) must still classify as a plain upload review, not auth —
        // this guards against the new check being too broad.
        let unrelated = PendingOperation {
            last_error: Some("500 Internal Server Error".to_string()),
            ..op
        };
        let (kind, _status, _detail, _action) = classify_review_operation(&unrelated);
        assert_eq!(kind, "failed_upload");
    }

    #[test]
    fn classify_ignores_status_like_digits_in_the_request_url() {
        // Task 1252: reqwest's Display suffixes `" for url (…)"`, and the URL's
        // ephemeral port / path ids can contain "401", "403", etc. Those are not
        // status codes and must never flip a retryable failure into a pausable
        // one. A real 5xx to such a port must stay Retryable.
        for port in ["45401", "40113", "34012", "50403", "44039"] {
            let err = format!(
                "HTTP status server error (500 Internal Server Error) for url (http://127.0.0.1:{port}/api/v1/uploads/upload-session-1/chunks/0)"
            );
            assert_eq!(
                classify_operation_error(&err),
                OperationFailureClass::Retryable,
                "500 to port {port} must stay Retryable, not be misread from the URL"
            );
        }
        // A connection error to the same kind of port is likewise Retryable.
        assert_eq!(
            classify_operation_error(
                "error sending request for url (http://127.0.0.1:45401/api/v1/uploads/upload-session-1/chunks/0)"
            ),
            OperationFailureClass::Retryable
        );
        // Genuine auth/permission failures still classify from reqwest's status
        // reason phrase (which sits BEFORE the stripped URL), even when the URL
        // also carries unrelated digits.
        assert_eq!(
            classify_operation_error(
                "HTTP status client error (401 Unauthorized) for url (http://127.0.0.1:8080/api/v1/uploads/init)"
            ),
            OperationFailureClass::Auth
        );
        assert_eq!(
            classify_operation_error(
                "HTTP status client error (403 Forbidden) for url (http://127.0.0.1:8080/api/v1/files/abc)"
            ),
            OperationFailureClass::Permission
        );
    }

    #[test]
    fn test_metadata_file_row_persists_contract_for_own_file() {
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        let metadata = serde_json::json!({
            "id": "file-1",
            "path": "Projects/spec.docx",
            "parent_id": "folder-1",
            "size_bytes": 4096,
            "updated_at": 1234,
            "content_type": "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            "version_number": 7,
            "object_version_id": "object-7"
        });

        // This fixture carries a plaintext `path` and no `name_encrypted`, so
        // it exercises the plaintext fallback in `resolve_relative_path`; the
        // master key is unused on that branch.
        let entry = apply_metadata_file_row(
            &db,
            &metadata,
            Namespace::MyFiles,
            None,
            None,
            PERMISSION_READ | PERMISSION_WRITE | PERMISSION_OWNER,
            2000,
            &[0u8; 32],
            "",
        )
        .unwrap()
        .unwrap();

        assert_eq!(entry.status, FileStatus::CloudOnly);
        assert_eq!(entry.remote_updated_at, 1234);
        let contract = db.get_file_contract_state("file-1").unwrap().unwrap();
        assert_eq!(contract.namespace, Namespace::MyFiles);
        assert_eq!(contract.parent_id.as_deref(), Some("folder-1"));
        assert_eq!(
            contract.permission_bits,
            PERMISSION_READ | PERMISSION_WRITE | PERMISSION_OWNER
        );
        assert_eq!(
            contract.content_type.as_deref(),
            Some("application/vnd.openxmlformats-officedocument.wordprocessingml.document")
        );
        assert_eq!(contract.current_version, 7);
        assert_eq!(contract.current_object_version_id.as_deref(), Some("object-7"));
        assert_eq!(contract.last_sync_at, 2000);
    }

    #[test]
    fn test_metadata_file_row_decrypts_encrypted_name_into_path() {
        // E2EE invariant: the server returns the filename only as the
        // encrypted `name_encrypted` blob (no plaintext path). The ingest must
        // decrypt it with the master key and store the plaintext relative path
        // so placeholder seeding + hydration can resolve a real destination.
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();

        let master_key = [7u8; 32];
        let file_id = "f1e2d3c4-0000-4000-8000-000000000001";
        // Encrypt the name exactly as every client does (shared core path).
        let mk = beebeeb_core::kdf::MasterKey::from_bytes(master_key);
        let name_encrypted =
            beebeeb_core::encrypt::encrypt_name(&mk, file_id, "Quarterly Report.pdf", Some("application/pdf")).unwrap();

        let metadata = serde_json::json!({
            "id": file_id,
            "name_encrypted": name_encrypted,
            "size_bytes": 2048,
            "updated_at": 5555,
            "is_folder": false,
        });

        let entry = apply_metadata_file_row(
            &db,
            &metadata,
            Namespace::MyFiles,
            None,
            None,
            PERMISSION_READ | PERMISSION_WRITE | PERMISSION_OWNER,
            6000,
            &master_key,
            "",
        )
        .unwrap()
        .unwrap();

        // The decrypted name lands in `path` (a root-level item's relative
        // path under the sync root IS its name), not an empty string.
        assert_eq!(entry.path, "Quarterly Report.pdf");
        assert_eq!(entry.status, FileStatus::CloudOnly);
        let stored = db.get_file(file_id).unwrap().unwrap();
        assert_eq!(stored.path, "Quarterly Report.pdf");
    }

    #[test]
    fn test_metadata_file_row_falls_back_when_name_undecryptable() {
        // A garbled/foreign blob must not abort the sweep nor poison the row:
        // resolve_relative_path falls through to any plaintext field, else "".
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();

        let metadata = serde_json::json!({
            "id": "f1e2d3c4-0000-4000-8000-000000000002",
            "name_encrypted": "{\"cipher_suite\":\"V1Aes256Gcm\",\"nonce\":[],\"ciphertext\":[]}",
            "size_bytes": 10,
            "updated_at": 1,
            "is_folder": false,
        });

        let entry = apply_metadata_file_row(
            &db,
            &metadata,
            Namespace::MyFiles,
            None,
            None,
            PERMISSION_READ | PERMISSION_WRITE | PERMISSION_OWNER,
            1,
            &[9u8; 32],
            "",
        )
        .unwrap()
        .unwrap();

        // No plaintext field present → empty path (caller skips seeding),
        // but the row still upserts so the rest of the sweep proceeds.
        assert_eq!(entry.path, "");
    }

    #[test]
    fn test_metadata_file_row_composes_nested_path_under_parent() {
        // NESTED enumeration: a child discovered under a folder must be stored
        // at `<parent_rel_path>/<decrypted_leaf>` (slash-joined, no leading
        // slash) so the path round-trips with the upload watcher's
        // `relative_db_path` and the windows_cf placeholder seeder.
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();

        let master_key = [3u8; 32];
        let file_id = "aaaaaaaa-0000-4000-8000-000000000003";
        let mk = beebeeb_core::kdf::MasterKey::from_bytes(master_key);
        let name_encrypted =
            beebeeb_core::encrypt::encrypt_name(&mk, file_id, "notes.txt", Some("text/plain")).unwrap();

        let metadata = serde_json::json!({
            "id": file_id,
            "name_encrypted": name_encrypted,
            "size_bytes": 12,
            "updated_at": 42,
            "is_folder": false,
            "parent_id": "folder-docs",
        });

        // Parent folder resolved to "docs" earlier in the BFS walk.
        let entry = apply_metadata_file_row(
            &db,
            &metadata,
            Namespace::MyFiles,
            None,
            None,
            PERMISSION_READ | PERMISSION_WRITE | PERMISSION_OWNER,
            100,
            &master_key,
            "docs",
        )
        .unwrap()
        .unwrap();

        assert_eq!(entry.path, "docs/notes.txt");
        assert_eq!(entry.item_kind, ItemKind::File);
        assert_eq!(entry.parent_id.as_deref(), Some("folder-docs"));
        // The stored row (what list_by_status / the seeder reads) agrees, and
        // the contract carries the authoritative parent_id + item_kind.
        let stored = db.get_file(file_id).unwrap().unwrap();
        assert_eq!(stored.path, "docs/notes.txt");
        assert_eq!(stored.item_kind, ItemKind::File);
        assert_eq!(stored.parent_id.as_deref(), Some("folder-docs"));
        let contract = db.get_file_contract_state(file_id).unwrap().unwrap();
        assert_eq!(contract.parent_id.as_deref(), Some("folder-docs"));
        assert_eq!(contract.item_kind, ItemKind::File);
    }

    #[test]
    fn test_metadata_file_row_classifies_folder_rows() {
        // A row the server marks as a folder must surface item_kind == Folder
        // both on the returned FileEntry and the stored row, so the BFS walk
        // descends into it and the placeholder seeder mints a DIRECTORY.
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();

        let master_key = [5u8; 32];
        let file_id = "bbbbbbbb-0000-4000-8000-000000000004";
        let mk = beebeeb_core::kdf::MasterKey::from_bytes(master_key);
        let name_encrypted = beebeeb_core::encrypt::encrypt_name(&mk, file_id, "Photos", None).unwrap();

        let metadata = serde_json::json!({
            "id": file_id,
            "name_encrypted": name_encrypted,
            "size_bytes": 0,
            "updated_at": 7,
            "is_folder": true,
        });

        let entry = apply_metadata_file_row(
            &db,
            &metadata,
            Namespace::MyFiles,
            None,
            None,
            PERMISSION_READ | PERMISSION_WRITE | PERMISSION_OWNER,
            100,
            &master_key,
            "",
        )
        .unwrap()
        .unwrap();

        assert_eq!(entry.path, "Photos");
        assert_eq!(entry.item_kind, ItemKind::Folder);
        assert!(entry.is_dir());
        let stored = db.get_file(file_id).unwrap().unwrap();
        assert_eq!(stored.item_kind, ItemKind::Folder);
        assert!(stored.is_dir());
    }

    #[tokio::test]
    async fn test_process_due_operations_completes_pin_marker_and_invalidates_file() {
        let dir = tempfile::tempdir().unwrap();
        let bridge = test_bridge(&dir.path().join("state.db"));
        bridge
            .db
            .enqueue_operation(&PendingOperation {
                op_id: "op-pin".into(),
                kind: OperationKind::PinTree,
                file_id: Some("folder-1".into()),
                parent_id: None,
                target_path: None,
                metadata_json: Some(r#"{"operation":"pin_tree","pinned":true}"#.into()),
                payload_path: None,
                base_version: None,
                base_object_version_id: None,
                attempts: 0,
                max_attempts: 5,
                next_retry_at: 0,
                last_error: None,
                backup_source_key: None,
                created_at: 100,
                updated_at: 100,
            })
            .unwrap();

        let outcome = bridge.process_due_operations(dir.path(), 200).await.unwrap();
        assert_eq!(outcome.completed_op_ids, vec!["op-pin".to_string()]);
        assert_eq!(outcome.invalidated_item_ids, vec!["folder-1".to_string()]);
        assert!(bridge.db.list_due_operations(999).unwrap().is_empty());
    }

    /// Task 1538 Codex P1 (PR #49, lib.rs:1087 thread): once the bridge's
    /// stop flag is set, `process_due_operations` must not execute ANY due
    /// operation — not "finish the current batch, then stop next tick".
    /// Uses `PinTree` (the cheapest op kind: `execute_operation` returns
    /// `Ok(())` with no network call) so a failure here can only be the
    /// missing stop-check, never a flaky mock server.
    #[tokio::test]
    async fn test_process_due_operations_stops_immediately_once_engine_is_stopping() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("state.db");
        let db = Arc::new(StateDb::open(&db_path).unwrap());
        let api = Arc::new(ApiClient::new(
            "https://api.beebeeb.io".into(),
            "token".into(),
            [7u8; 32],
        ));
        let stopping = Arc::new(AtomicBool::new(false));
        let bridge = EngineBridge::new_with_stop_flag(db.clone(), api, stopping.clone());

        db.enqueue_operation(&PendingOperation {
            op_id: "op-pin-1".into(),
            kind: OperationKind::PinTree,
            file_id: Some("folder-1".into()),
            parent_id: None,
            target_path: None,
            metadata_json: Some(r#"{"operation":"pin_tree","pinned":true}"#.into()),
            payload_path: None,
            base_version: None,
            base_object_version_id: None,
            attempts: 0,
            max_attempts: 5,
            next_retry_at: 0,
            last_error: None,
            backup_source_key: None,
            created_at: 100,
            updated_at: 100,
        })
        .unwrap();
        db.enqueue_operation(&PendingOperation {
            op_id: "op-pin-2".into(),
            kind: OperationKind::PinTree,
            file_id: Some("folder-2".into()),
            parent_id: None,
            target_path: None,
            metadata_json: Some(r#"{"operation":"pin_tree","pinned":true}"#.into()),
            payload_path: None,
            base_version: None,
            base_object_version_id: None,
            attempts: 0,
            max_attempts: 5,
            next_retry_at: 0,
            last_error: None,
            backup_source_key: None,
            created_at: 100,
            updated_at: 100,
        })
        .unwrap();

        // Ask the engine to stop BEFORE draining the queue — simulates
        // `EngineRunner::abort` flipping the flag while a tick is already
        // about to process a batch of due operations.
        stopping.store(true, Ordering::SeqCst);

        let outcome = bridge.process_due_operations(dir.path(), 200).await.unwrap();

        assert!(
            outcome.completed_op_ids.is_empty(),
            "no operation may run once the engine has been asked to stop"
        );
        assert_eq!(
            db.list_due_operations(999).unwrap().len(),
            2,
            "both queued ops must still be in the queue, untouched, for the caller's purge to clear"
        );
    }

    /// Task 1538 Codex P1 — the watcher (Windows upload watcher AND the
    /// macOS/Linux File Provider IPC handler both call these directly) must
    /// not be able to slip a fresh write into the queue once the engine has
    /// been asked to stop, or sign-out's purge could run BEFORE the write
    /// lands and then miss it entirely.
    #[test]
    fn test_queue_finder_writes_refuse_to_enqueue_once_engine_is_stopping() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("state.db");
        let db = Arc::new(StateDb::open(&db_path).unwrap());
        let api = Arc::new(ApiClient::new(
            "https://api.beebeeb.io".into(),
            "token".into(),
            [7u8; 32],
        ));
        let stopping = Arc::new(AtomicBool::new(true));
        let bridge = EngineBridge::new_with_stop_flag(db.clone(), api, stopping);

        let create_result = bridge.queue_finder_create(FinderWriteTarget {
            file_id: None,
            parent_id: None,
            filename: "new-file.txt".into(),
            rel_path: None,
            kind: FinderWriteItemKind::File,
            contents_path: None,
            content_type: None,
            base_version_identifier: None,
        });
        assert!(create_result.is_err(), "queue_finder_create must refuse while stopping");

        let modify_result = bridge.queue_finder_modify(FinderWriteTarget {
            file_id: Some("file-1".into()),
            parent_id: None,
            filename: "renamed.txt".into(),
            rel_path: None,
            kind: FinderWriteItemKind::File,
            contents_path: None,
            content_type: None,
            base_version_identifier: None,
        });
        assert!(modify_result.is_err(), "queue_finder_modify must refuse while stopping");

        let delete_result = bridge.queue_finder_delete("file-1", None);
        assert!(delete_result.is_err(), "queue_finder_delete must refuse while stopping");

        assert!(
            db.list_due_operations(i64::MAX).unwrap().is_empty(),
            "not a single one of the refused writes may have reached the operation_queue"
        );
    }

    #[tokio::test]
    async fn test_process_due_operations_records_retry_for_upload_worker_handoff() {
        let dir = tempfile::tempdir().unwrap();
        let bridge = test_bridge(&dir.path().join("state.db"));
        seed_bridge_row(&bridge, "file-1", "Draft.txt", None, FileStatus::Uploading, 0);
        bridge
            .db
            .enqueue_operation(&PendingOperation {
                op_id: "op-upload".into(),
                kind: OperationKind::UploadVersion,
                file_id: Some("file-1".into()),
                parent_id: None,
                target_path: Some("Draft.txt".into()),
                metadata_json: Some(r#"{"operation":"upload_version"}"#.into()),
                payload_path: Some(dir.path().join("payload").to_string_lossy().into_owned()),
                base_version: Some(1),
                base_object_version_id: None,
                attempts: 0,
                max_attempts: 5,
                next_retry_at: 0,
                last_error: None,
                backup_source_key: None,
                created_at: 100,
                updated_at: 100,
            })
            .unwrap();

        let outcome = bridge.process_due_operations(dir.path(), 200).await.unwrap();
        assert_eq!(outcome.retried_op_ids, vec!["op-upload".to_string()]);
        assert!(outcome.paused_op_ids.is_empty());

        let due_too_early = bridge.db.list_due_operations(229).unwrap();
        assert!(due_too_early.is_empty());
        let due = bridge.db.list_due_operations(230).unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].attempts, 1);
        assert!(
            due[0]
                .last_error
                .as_deref()
                .unwrap_or("")
                .contains("staged upload payload is missing")
        );
        assert_eq!(
            bridge.db.get_file("file-1").unwrap().unwrap().status,
            FileStatus::Error,
            "a failed/deferred upload must not remain counted as active Uploading"
        );
    }

    /// Review Minor 3: a staged-copy read error other than a missing copy retries with a
    /// fixed text. The copy's path never reaches `classify_operation_error`, so a path that
    /// contains `403` is not taken for a refused request (the class of bug behind task 1252).
    #[cfg(unix)]
    #[tokio::test]
    async fn an_unreadable_staged_copy_is_retried_whatever_its_path_contains() {
        use std::os::unix::fs::PermissionsExt;
        struct RestoreMode(PathBuf);
        impl Drop for RestoreMode {
            fn drop(&mut self) {
                let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o755));
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let bridge = test_bridge(&dir.path().join("state.db"));
        seed_bridge_row(&bridge, "file-1", "Draft.txt", None, FileStatus::Uploading, 0);
        let locked = dir.path().join("staged-403");
        std::fs::create_dir(&locked).unwrap();
        let payload = locked.join("payload");
        std::fs::write(&payload, b"bytes").unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        let _restore = RestoreMode(locked.clone());
        match std::fs::metadata(&payload) {
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {}
            other => panic!("the setup must produce a permission error (not as root): {other:?}"),
        }
        bridge
            .db
            .enqueue_operation(&PendingOperation {
                op_id: "op-upload".into(),
                kind: OperationKind::UploadVersion,
                file_id: Some("file-1".into()),
                parent_id: None,
                target_path: Some("Draft.txt".into()),
                metadata_json: Some(r#"{"operation":"upload_version"}"#.into()),
                payload_path: Some(payload.to_string_lossy().into_owned()),
                base_version: Some(1),
                base_object_version_id: None,
                attempts: 0,
                max_attempts: 5,
                next_retry_at: 0,
                last_error: None,
                backup_source_key: None,
                created_at: 100,
                updated_at: 100,
            })
            .unwrap();

        let outcome = bridge.process_due_operations(dir.path(), 200).await.unwrap();
        assert!(
            outcome.paused_op_ids.is_empty(),
            "a read error is not a refused request: {outcome:?}"
        );
        assert_eq!(outcome.retried_op_ids, vec!["op-upload".to_string()]);
        let op = bridge.db.get_operation("op-upload").unwrap().unwrap();
        assert_eq!(op.attempts, 1);
        assert_eq!(
            op.last_error.as_deref(),
            Some("staged upload payload could not be read"),
            "a fixed text, without the path"
        );
    }

    #[cfg(target_os = "windows")]
    #[tokio::test]
    async fn round7_busy_upload_retries_identity_without_reupload() {
        round7_completed_upload_retry(false).await;
    }

    #[cfg(target_os = "windows")]
    #[tokio::test]
    async fn round7_signout_retries_completed_identity_without_reupload() {
        round7_completed_upload_retry(true).await;
    }

    #[cfg(target_os = "windows")]
    async fn round7_completed_upload_retry(signout_retry: bool) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        std::fs::create_dir_all(&root).unwrap();
        crate::windows_cf::register_sync_root(&root).unwrap();
        struct Registration(std::path::PathBuf);
        impl Drop for Registration {
            fn drop(&mut self) {
                crate::windows_cf::unregister_sync_root(&self.0).unwrap();
            }
        }
        let _registration = Registration(root.clone());
        let local = root.join("report.txt");
        std::fs::write(&local, b"live upload payload").unwrap();
        crate::windows_cf::placeholders::convert_to_unsynced_placeholder(&local, "local-file-1").unwrap();
        use std::os::windows::fs::OpenOptionsExt;
        let held = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(7)
            .open(&local)
            .unwrap();
        let payload = dir.path().join("payload.txt");
        std::fs::write(&payload, b"live upload payload").unwrap();
        let server = UploadMockServer::start(false);
        let master_key = [11u8; 32];
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        bridge
            .db
            .enqueue_operation(&PendingOperation {
                op_id: "op-upload-live".into(),
                kind: OperationKind::UploadVersion,
                file_id: Some("local-file-1".into()),
                parent_id: Some("folder-1".into()),
                target_path: Some("report.txt".into()),
                metadata_json: Some(
                    serde_json::json!({
                        "operation": "create_file",
                        "name_encrypted": "{\"cipher_suite\":\"V1Aes256Gcm\"}",
                        "display_name": "report.txt",
                        "content_type": "text/plain"
                    })
                    .to_string(),
                ),
                payload_path: Some(payload.to_string_lossy().into_owned()),
                base_version: None,
                base_object_version_id: None,
                attempts: 0,
                max_attempts: 5,
                next_retry_at: 0,
                last_error: None,
                backup_source_key: None,
                created_at: 100,
                updated_at: 100,
            })
            .unwrap();

        let outcome = bridge.process_due_operations(&root, 200).await.unwrap();
        assert_eq!(outcome.completed_op_ids, vec!["op-upload-live".to_string()]);
        assert!(outcome.retried_op_ids.is_empty());
        assert!(
            payload.exists(),
            "completed payload proof must survive a busy identity stamp"
        );
        assert!(bridge.db.list_due_operations(999).unwrap().is_empty());
        assert!(bridge.db.get_file("local-file-1").unwrap().is_none());

        let entry = bridge.db.get_file("server-file-1").unwrap().unwrap();
        assert_eq!(entry.status, FileStatus::Local);
        assert_eq!(entry.path, "report.txt");
        assert_eq!(entry.size_bytes, 19);

        let contract = bridge.db.get_file_contract_state("server-file-1").unwrap().unwrap();
        assert_eq!(contract.current_version, 1);
        assert_eq!(contract.local_base_version, 1);
        assert_eq!(contract.current_object_version_id.as_deref(), Some("object-complete-1"));
        assert_eq!(contract.content_type.as_deref(), Some("text/plain"));

        assert_eq!(bridge.db.upload_finalizations().unwrap().len(), 1);
        assert!(!bridge.db.upload_finalizations().unwrap()[0].stamped);
        drop(held);
        // Retry uses durable state after the bridge and SQLite connection close.
        drop(bridge);
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        assert_eq!(bridge.db.upload_finalizations().unwrap().len(), 1);
        if !signout_retry {
            // Periodic operation pass has no upload op left to execute.
            bridge.process_due_operations(&root, 300).await.unwrap();
            assert!(!payload.exists(), "proof is removed after successful identity retry");
            assert_eq!(bridge.db.upload_finalizations().unwrap().len(), 0);
        }
        crate::windows_cf::signout::purge(&bridge.db, Some(&root)).unwrap();
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
        assert!(!payload.exists());
        assert_eq!(bridge.db.upload_finalizations().unwrap().len(), 0);
        let requests = server.finish();
        assert_eq!(requests.len(), 4);
        assert_eq!(requests[0].method, "POST");
        assert_eq!(requests[0].path, "/api/v1/uploads/init");
        let init_body: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert!(
            init_body.get("file_id").is_none(),
            "new-file uploads must let the server mint the id"
        );
        assert_eq!(init_body["file_size_bytes"], 19);
        assert_eq!(init_body["parent_id"], "folder-1");
        assert_eq!(init_body["chunk_count"], 1);
        assert_eq!(requests[1].method, "PATCH");
        assert_eq!(requests[2].method, "PUT");
        assert_eq!(requests[2].path, "/api/v1/uploads/upload-session-1/chunks/0");
        assert_ne!(requests[2].body, b"live upload payload");

        let master_key = beebeeb_core::kdf::MasterKey::from_bytes(master_key);
        let file_key = beebeeb_core::kdf::derive_file_key(&master_key, b"server-file-1");
        assert_eq!(
            beebeeb_core::encrypt::decrypt_chunk_raw(&file_key, &requests[2].body).unwrap(),
            b"live upload payload"
        );
        assert_eq!(requests[3].method, "POST");
        assert_eq!(requests[3].path, "/api/v1/uploads/upload-session-1/complete");
    }

    #[tokio::test]
    async fn test_process_due_operations_uploads_encrypted_payload_and_commits_state() {
        let dir = tempfile::tempdir().unwrap();
        let payload = dir.path().join("payload.txt");
        std::fs::write(&payload, b"live upload payload").unwrap();
        let server = UploadMockServer::start(false);
        let master_key = [11u8; 32];
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        bridge
            .db
            .enqueue_operation(&PendingOperation {
                op_id: "op-upload-live".into(),
                kind: OperationKind::UploadVersion,
                file_id: Some("local-file-1".into()),
                parent_id: Some("folder-1".into()),
                target_path: Some("Reports/report.txt".into()),
                metadata_json: Some(
                    serde_json::json!({
                        "operation": "create_file",
                        "name_encrypted": "{\"cipher_suite\":\"V1Aes256Gcm\"}",
                        "display_name": "report.txt",
                        "content_type": "text/plain"
                    })
                    .to_string(),
                ),
                payload_path: Some(payload.to_string_lossy().into_owned()),
                base_version: None,
                base_object_version_id: None,
                attempts: 0,
                max_attempts: 5,
                next_retry_at: 0,
                last_error: None,
                backup_source_key: None,
                created_at: 100,
                updated_at: 100,
            })
            .unwrap();

        let outcome = bridge.process_due_operations(dir.path(), 200).await.unwrap();
        assert_eq!(outcome.completed_op_ids, vec!["op-upload-live".to_string()]);
        assert!(outcome.retried_op_ids.is_empty());
        assert!(
            !payload.exists(),
            "staged payload should be removed only after complete succeeds"
        );
        assert!(bridge.db.list_due_operations(999).unwrap().is_empty());
        assert!(bridge.db.get_file("local-file-1").unwrap().is_none());

        let entry = bridge.db.get_file("server-file-1").unwrap().unwrap();
        assert_eq!(entry.status, FileStatus::Local);
        assert_eq!(entry.path, "Reports/report.txt");
        assert_eq!(entry.size_bytes, 19);

        let contract = bridge.db.get_file_contract_state("server-file-1").unwrap().unwrap();
        assert_eq!(contract.current_version, 1);
        assert_eq!(contract.local_base_version, 1);
        assert_eq!(contract.current_object_version_id.as_deref(), Some("object-complete-1"));
        assert_eq!(contract.content_type.as_deref(), Some("text/plain"));

        let requests = server.finish();
        assert_eq!(requests.len(), 4);
        assert_eq!(requests[0].method, "POST");
        assert_eq!(requests[0].path, "/api/v1/uploads/init");
        let init_body: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert!(
            init_body.get("file_id").is_none(),
            "new-file uploads must let the server mint the id"
        );
        assert_eq!(init_body["file_size_bytes"], 19);
        assert_eq!(init_body["parent_id"], "folder-1");
        assert_eq!(init_body["chunk_count"], 1);
        assert_eq!(requests[1].method, "PATCH");
        assert_eq!(requests[2].method, "PUT");
        assert_eq!(requests[2].path, "/api/v1/uploads/upload-session-1/chunks/0");
        assert_ne!(requests[2].body, b"live upload payload");

        let master_key = beebeeb_core::kdf::MasterKey::from_bytes(master_key);
        let file_key = beebeeb_core::kdf::derive_file_key(&master_key, b"server-file-1");
        assert_eq!(
            beebeeb_core::encrypt::decrypt_chunk_raw(&file_key, &requests[2].body).unwrap(),
            b"live upload payload"
        );
        assert_eq!(requests[3].method, "POST");
        assert_eq!(requests[3].path, "/api/v1/uploads/upload-session-1/complete");
    }

    /// Empty (0-byte) files — `.gitkeep`, `__init__.py`, `touch` placeholders —
    /// must sync like any other file. The canonical empty-file plan is ONE chunk
    /// carrying the AEAD of zero bytes (`plan_chunks(0, _)` → `chunk_count == 1`),
    /// so the upload is init(size 0) → one 28-byte chunk PUT → complete, and the
    /// op completes instead of landing in `Error` and being retried.
    #[tokio::test]
    async fn test_process_due_operations_uploads_empty_file_as_one_encrypted_empty_chunk() {
        let dir = tempfile::tempdir().unwrap();
        let payload = dir.path().join("empty.txt");
        std::fs::write(&payload, b"").unwrap();
        let server = UploadMockServer::start(false);
        let master_key = [12u8; 32];
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        bridge
            .db
            .enqueue_operation(&PendingOperation {
                op_id: "op-upload-empty".into(),
                kind: OperationKind::UploadVersion,
                file_id: Some("local-file-empty".into()),
                parent_id: Some("folder-1".into()),
                target_path: Some("Project/empty.txt".into()),
                metadata_json: Some(
                    serde_json::json!({
                        "operation": "create_file",
                        "name_encrypted": "{\"cipher_suite\":\"V1Aes256Gcm\"}",
                        "display_name": "empty.txt",
                        "content_type": "text/plain"
                    })
                    .to_string(),
                ),
                payload_path: Some(payload.to_string_lossy().into_owned()),
                base_version: None,
                base_object_version_id: None,
                attempts: 0,
                max_attempts: 5,
                next_retry_at: 0,
                last_error: None,
                backup_source_key: None,
                created_at: 100,
                updated_at: 100,
            })
            .unwrap();

        let outcome = bridge.process_due_operations(dir.path(), 200).await.unwrap();
        assert!(
            outcome.retried_op_ids.is_empty(),
            "an empty file must not be retried: {:?}",
            bridge.db.queue_diagnostics(200).unwrap().last_error
        );
        assert_eq!(outcome.completed_op_ids, vec!["op-upload-empty".to_string()]);
        assert!(bridge.db.list_due_operations(999).unwrap().is_empty());
        let entry = bridge.db.get_file("server-file-1").unwrap().unwrap();
        assert_eq!(entry.status, FileStatus::Local);
        assert_eq!(entry.path, "Project/empty.txt");

        let requests = server.finish();
        assert_eq!(requests.len(), 4);
        assert_eq!(requests[0].path, "/api/v1/uploads/init");
        let init_body: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(init_body["file_size_bytes"], 0);
        assert_eq!(init_body["chunk_count"], 1);
        assert_eq!(requests[2].method, "PUT");
        assert_eq!(requests[2].path, "/api/v1/uploads/upload-session-1/chunks/0");
        assert_eq!(requests[2].body.len(), 28, "nonce + tag, no payload");
        let master_key = beebeeb_core::kdf::MasterKey::from_bytes(master_key);
        let file_key = beebeeb_core::kdf::derive_file_key(&master_key, b"server-file-1");
        assert!(
            beebeeb_core::encrypt::decrypt_chunk_raw(&file_key, &requests[2].body)
                .unwrap()
                .is_empty()
        );
        assert_eq!(requests[3].path, "/api/v1/uploads/upload-session-1/complete");
    }

    /// Device B side of the empty-file round trip: a server file with
    /// `size_bytes: 0` and one encrypted empty chunk hydrates to a 0-byte file.
    #[test]
    fn hydrate_file_writes_empty_file_from_single_empty_chunk() {
        let dir = tempfile::tempdir().unwrap();
        let master_key = [14u8; 32];
        let file_key = hydration_test_key(master_key, TEST_FILE_ID);
        // 1 metadata GET + 1 chunk GET.
        let server = HydrationMockServer::start(file_key, vec![Vec::new()], 2);
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_bridge_row(&bridge, TEST_FILE_ID, "/empty.txt", None, FileStatus::CloudOnly, 0);

        let dest = dir.path().join("empty.txt");
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async { bridge.hydrate_file(TEST_FILE_ID, &dest, &[dir.path()]).await })
            .unwrap();
        server.finish();

        assert_eq!(std::fs::metadata(&dest).unwrap().len(), 0);
        let entry = bridge.db.get_file(TEST_FILE_ID).unwrap().unwrap();
        assert_eq!(entry.status, FileStatus::Local);
    }

    #[tokio::test]
    async fn test_conflict_content_preview_reads_local_disk_and_downloads_real_remote_content() {
        // Task 1546 finding 2: ConflictWindow.tsx's own doc-comment admitted the
        // diff body was a hardcoded placeholder for EVERY conflict ("Content
        // from this device…" / "Content from other device…"), never the
        // file's real content, because "actual diffing needs the daemon to
        // expose both blob bytes". This is that daemon-side exposure.
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        std::fs::write(sync_root.join("notes.txt"), b"local version text").unwrap();

        let master_key = [21u8; 32];
        let file_key = hydration_test_key(master_key, TEST_FILE_ID);
        let remote_chunks = vec![b"remote version text".to_vec()];
        let server = HydrationMockServer::start(file_key, remote_chunks, 2);

        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        bridge
            .db
            .upsert_file(&FileEntry {
                file_id: TEST_FILE_ID.into(),
                path: "notes.txt".into(),
                status: FileStatus::Conflict,
                size_bytes: 18,
                modified_at: 100,
                content_hash: Some("local-hash".into()),
                remote_updated_at: 90,
                parent_id: None,
                item_kind: ItemKind::File,
            })
            .unwrap();

        let preview = bridge.conflict_content_preview(TEST_FILE_ID, &sync_root).await.unwrap();

        // The two sides must be the REAL, DIFFERENT content, not the old
        // "Content from this device…" / "Content from other device…" pair.
        assert_eq!(preview.local.text.as_deref(), Some("local version text"));
        assert_eq!(preview.remote.text.as_deref(), Some("remote version text"));
        assert!(preview.local.unavailable_reason.is_none());
        assert!(preview.remote.unavailable_reason.is_none());

        // A preview must be read-only: the row is mid-conflict and neither
        // side has been chosen, so daemon bookkeeping (status, cache) must
        // not move just because the user opened the window. This is exactly
        // why the implementation calls `do_hydrate` directly instead of
        // `hydrate_file`/`hydrate_file_to_memory`, which both flip status.
        let entry = bridge.db.get_file(TEST_FILE_ID).unwrap().unwrap();
        assert_eq!(entry.status, FileStatus::Conflict);

        server.finish();
    }

    #[tokio::test]
    async fn test_conflict_content_preview_reports_a_local_read_failure_without_failing_the_whole_call() {
        // The local file may be missing (e.g. deleted outside the daemon) even
        // though the row is Conflict; the remote side must still load so the
        // user isn't left with neither pane.
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        // Deliberately do NOT write sync_root/notes.txt.

        let master_key = [22u8; 32];
        let file_key = hydration_test_key(master_key, TEST_FILE_ID);
        let server = HydrationMockServer::start(file_key, vec![b"remote only".to_vec()], 2);

        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        bridge
            .db
            .upsert_file(&FileEntry {
                file_id: TEST_FILE_ID.into(),
                path: "notes.txt".into(),
                status: FileStatus::Conflict,
                size_bytes: 0,
                modified_at: 100,
                content_hash: None,
                remote_updated_at: 90,
                parent_id: None,
                item_kind: ItemKind::File,
            })
            .unwrap();

        let preview = bridge.conflict_content_preview(TEST_FILE_ID, &sync_root).await.unwrap();

        assert!(preview.local.text.is_none());
        assert!(preview.local.unavailable_reason.is_some());
        assert_eq!(preview.remote.text.as_deref(), Some("remote only"));

        server.finish();
    }

    #[tokio::test]
    async fn test_conflict_content_preview_skips_remote_download_when_metadata_reports_oversized_file() {
        // Codex round 2, finding 1: the remote size must be checked from
        // metadata BEFORE any chunk is downloaded. `requests: 1` means the
        // mock server serves ONLY the metadata GET and then stops — if the
        // implementation regresses to downloading anyway, the resulting
        // chunk GET either fails to connect (server already exited) or hits
        // a closed listener, so `remote.unavailable_reason` would report a
        // download/connection failure instead of "too large," and this test
        // would fail (not hang: the listener is dropped, not left open).
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        std::fs::write(sync_root.join("notes.txt"), b"small local text").unwrap();

        let master_key = [23u8; 32];
        let file_key = hydration_test_key(master_key, TEST_FILE_ID);
        // The chunk bytes are real (so `size_bytes` in the mocked metadata
        // response is real and over the cap) but must NEVER be fetched.
        let oversized_chunk = vec![b'z'; CONFLICT_PREVIEW_TEXT_MAX_BYTES + 100];
        let expected_size = oversized_chunk.len() as u64;
        let server = HydrationMockServer::start(file_key, vec![oversized_chunk], 1);

        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        bridge
            .db
            .upsert_file(&FileEntry {
                file_id: TEST_FILE_ID.into(),
                path: "notes.txt".into(),
                status: FileStatus::Conflict,
                size_bytes: 16,
                modified_at: 100,
                content_hash: Some("local-hash".into()),
                remote_updated_at: 90,
                parent_id: None,
                item_kind: ItemKind::File,
            })
            .unwrap();

        let preview = bridge.conflict_content_preview(TEST_FILE_ID, &sync_root).await.unwrap();

        assert!(preview.is_text, "notes.txt must classify as text");
        assert!(preview.remote.text.is_none());
        assert_eq!(preview.remote.size_bytes, Some(expected_size));
        let reason = preview
            .remote
            .unavailable_reason
            .expect("must explain why text is absent");
        assert!(reason.contains("too large"), "reason was: {reason}");
        assert!(reason.contains(&expected_size.to_string()), "reason was: {reason}");

        // The local side is untouched by this finding and must still work.
        assert_eq!(preview.local.text.as_deref(), Some("small local text"));

        server.finish();
    }

    #[tokio::test]
    async fn test_conflict_content_preview_local_bounded_read_reports_too_large_with_accurate_size() {
        // Codex round 2, finding 1's local half: a local file over the cap
        // must report "too large" with the REAL size (from `fs::metadata`,
        // not from reading the whole file — `local_content_preview` never
        // calls `std::fs::read` on an oversized file).
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let oversized = vec![b'x'; CONFLICT_PREVIEW_TEXT_MAX_BYTES + 1];
        let expected_size = oversized.len() as u64;
        std::fs::write(sync_root.join("notes.txt"), &oversized).unwrap();

        let master_key = [24u8; 32];
        let file_key = hydration_test_key(master_key, TEST_FILE_ID);
        let server = HydrationMockServer::start(file_key, vec![b"remote version text".to_vec()], 2);

        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        bridge
            .db
            .upsert_file(&FileEntry {
                file_id: TEST_FILE_ID.into(),
                path: "notes.txt".into(),
                status: FileStatus::Conflict,
                size_bytes: expected_size as i64,
                modified_at: 100,
                content_hash: Some("local-hash".into()),
                remote_updated_at: 90,
                parent_id: None,
                item_kind: ItemKind::File,
            })
            .unwrap();

        let preview = bridge.conflict_content_preview(TEST_FILE_ID, &sync_root).await.unwrap();

        assert!(preview.local.text.is_none());
        assert_eq!(preview.local.size_bytes, Some(expected_size));
        let reason = preview
            .local
            .unavailable_reason
            .expect("must explain why text is absent");
        assert!(reason.contains("too large"), "reason was: {reason}");
        assert!(reason.contains(&expected_size.to_string()), "reason was: {reason}");

        // The remote side (small, real content) is untouched by this finding.
        assert_eq!(preview.remote.text.as_deref(), Some("remote version text"));

        server.finish();
    }

    #[tokio::test]
    async fn test_conflict_content_preview_determines_textness_from_the_file_path_not_a_caller_flag() {
        // Codex round 2, finding 3: `conflict_content_preview` no longer
        // takes an `is_text` parameter at all — the daemon decides from the
        // row's own `path` via `is_text_file`. This is exactly the bug:
        // VersionCenter's `open_conflict_window` call always hardcoded
        // `isText: false`, which (before this fix) discarded valid text
        // content for every conflict opened that way. Here the path has a
        // binary extension (`.jpg`) even though the bytes on both sides
        // happen to be valid UTF-8 — the response must still classify as
        // binary and never surface `text`, proving the decision comes from
        // the path, not from any UTF-8-validity heuristic on the bytes.
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        std::fs::write(sync_root.join("photo.jpg"), b"not really jpeg bytes").unwrap();

        let master_key = [25u8; 32];
        let file_key = hydration_test_key(master_key, TEST_FILE_ID);
        // requests: 1 — a binary preview must fetch ONLY the metadata GET
        // (to learn `size_bytes`) and never a chunk GET, so the mock server
        // must never need to serve a 2nd request. If the implementation
        // regresses to always downloading, that 2nd request fails to
        // connect (the server thread already exited after 1) rather than
        // hanging, so this stays a fast, deterministic red, not a hang.
        let server = HydrationMockServer::start(file_key, vec![b"also not really jpeg bytes".to_vec()], 1);

        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        bridge
            .db
            .upsert_file(&FileEntry {
                file_id: TEST_FILE_ID.into(),
                path: "photo.jpg".into(),
                status: FileStatus::Conflict,
                size_bytes: 21,
                modified_at: 100,
                content_hash: Some("local-hash".into()),
                remote_updated_at: 90,
                parent_id: None,
                item_kind: ItemKind::File,
            })
            .unwrap();

        let preview = bridge.conflict_content_preview(TEST_FILE_ID, &sync_root).await.unwrap();

        assert!(!preview.is_text, "a .jpg path must classify as binary");
        assert!(preview.local.text.is_none());
        assert!(preview.remote.text.is_none());
        assert!(
            preview.local.unavailable_reason.is_none(),
            "binary is an expected, not an error, state"
        );
        assert!(
            preview.remote.unavailable_reason.is_none(),
            "binary is an expected, not an error, state"
        );
        assert_eq!(preview.local.size_bytes, Some(21));
        assert_eq!(preview.remote.size_bytes, Some(26));

        server.finish();
    }

    #[tokio::test]
    async fn test_resolve_keep_mine_uploads_local_payload_and_commits_state() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(sync_root.join("Docs")).unwrap();
        let local_path = sync_root.join("Docs/conflict.txt");
        std::fs::write(&local_path, b"mine upload payload").unwrap();

        let server = UploadMockServer::start(false);
        let master_key = [13u8; 32];
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        bridge
            .db
            .upsert_file(&FileEntry {
                file_id: "server-file-1".into(),
                path: "Docs/conflict.txt".into(),
                status: FileStatus::Conflict,
                size_bytes: 99,
                modified_at: 100,
                content_hash: Some("local-conflict-hash".into()),
                remote_updated_at: 90,
                parent_id: None,
                item_kind: ItemKind::File,
            })
            .unwrap();
        let mut contract = bridge.db.get_file_contract_state("server-file-1").unwrap().unwrap();
        contract.parent_id = Some("folder-1".into());
        contract.content_type = Some("text/plain".into());
        contract.current_version = 3;
        contract.local_base_version = 2;
        contract.current_object_version_id = Some("object-before".into());
        bridge.db.set_file_contract_state(&contract).unwrap();

        let before = now_secs();
        bridge.resolve_keep_mine("server-file-1", &sync_root).await.unwrap();

        let entry = bridge.db.get_file("server-file-1").unwrap().unwrap();
        assert_eq!(entry.status, FileStatus::Local);
        assert_eq!(entry.path, "Docs/conflict.txt");
        assert_eq!(entry.size_bytes, 19);
        assert_eq!(entry.content_hash.as_deref(), Some("local-conflict-hash"));
        assert!(entry.remote_updated_at >= before);

        let contract = bridge.db.get_file_contract_state("server-file-1").unwrap().unwrap();
        assert_eq!(contract.current_version, 1);
        assert_eq!(contract.local_base_version, 1);
        assert_eq!(contract.current_object_version_id.as_deref(), Some("object-complete-1"));
        assert_eq!(contract.parent_id.as_deref(), Some("folder-1"));
        assert_eq!(contract.content_type.as_deref(), Some("text/plain"));

        let requests = server.finish();
        assert_eq!(requests.len(), 4);
        assert_eq!(requests[0].method, "POST");
        assert_eq!(requests[0].path, "/api/v1/uploads/init");
        let init_body: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(init_body["file_id"], "server-file-1");
        assert_eq!(init_body["file_size_bytes"], 19);
        assert_eq!(init_body["parent_id"], "folder-1");
        assert_eq!(init_body["chunk_count"], 1);
        assert!(
            init_body.get("base_version_number").is_none(),
            "Keep Mine is an explicit conflict override, not a stale-base retry"
        );
        assert_eq!(requests[1].method, "PATCH");
        assert_eq!(requests[1].path, "/api/v1/files/server-file-1");
        assert_eq!(requests[2].method, "PUT");
        assert_eq!(requests[2].path, "/api/v1/uploads/upload-session-1/chunks/0");
        assert_ne!(requests[2].body, b"mine upload payload");

        let master_key = beebeeb_core::kdf::MasterKey::from_bytes(master_key);
        let file_key = beebeeb_core::kdf::derive_file_key(&master_key, b"server-file-1");
        assert_eq!(
            beebeeb_core::encrypt::decrypt_chunk_raw(&file_key, &requests[2].body).unwrap(),
            b"mine upload payload"
        );
        assert_eq!(requests[3].method, "POST");
        assert_eq!(requests[3].path, "/api/v1/uploads/upload-session-1/complete");
    }

    #[tokio::test]
    async fn test_resolve_keep_mine_returns_error_when_upload_fails() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        std::fs::write(sync_root.join("conflict.txt"), b"retry me").unwrap();

        let server = UploadMockServer::start(true);
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), [14u8; 32]);
        bridge
            .db
            .upsert_file(&FileEntry {
                file_id: "server-file-1".into(),
                path: "conflict.txt".into(),
                status: FileStatus::Conflict,
                size_bytes: 8,
                modified_at: 100,
                content_hash: Some("local-conflict-hash".into()),
                remote_updated_at: 90,
                parent_id: None,
                item_kind: ItemKind::File,
            })
            .unwrap();
        let mut contract = bridge.db.get_file_contract_state("server-file-1").unwrap().unwrap();
        contract.content_type = Some("text/plain".into());
        contract.current_version = 3;
        contract.local_base_version = 2;
        bridge.db.set_file_contract_state(&contract).unwrap();

        let err = bridge
            .resolve_keep_mine("server-file-1", &sync_root)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("500 Internal Server Error"));

        let entry = bridge.db.get_file("server-file-1").unwrap().unwrap();
        assert_eq!(entry.status, FileStatus::Conflict);
        assert_eq!(entry.remote_updated_at, 90);

        let requests = server.finish();
        assert_eq!(requests.len(), 3);
        assert_eq!(requests[0].path, "/api/v1/uploads/init");
        assert_eq!(requests[1].path, "/api/v1/files/server-file-1");
        assert_eq!(requests[2].path, "/api/v1/uploads/upload-session-1/chunks/0");
    }

    #[tokio::test]
    async fn audit_1244_auto_resolve_keep_both_preserves_local_copy_when_remote_hydrate_fails() {
        // Production mutation caught: hydrating before renaming, or leaving the row
        // in Conflict after a hydrate failure, would either risk the local bytes or
        // hide the retry state from the normal Error flow.
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let original = sync_root.join("conflict.txt");
        std::fs::write(&original, b"local conflict bytes").unwrap();

        let file_id = "bbbb0000-0000-4000-8000-000000000010";
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), "http://127.0.0.1:9".into(), [15u8; 32]);
        bridge
            .db
            .upsert_file(&FileEntry {
                file_id: file_id.into(),
                path: "conflict.txt".into(),
                status: FileStatus::Conflict,
                size_bytes: 20,
                modified_at: 100,
                content_hash: Some("local-conflict-hash".into()),
                remote_updated_at: 90,
                parent_id: None,
                item_kind: ItemKind::File,
            })
            .unwrap();
        let entry = bridge.db.get_file(file_id).unwrap().unwrap();

        let err = bridge
            .auto_resolve_keep_both(&sync_root, &entry)
            .await
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("error sending request") || err.contains("Connection refused"),
            "actual hydration error: {err}"
        );

        let row = bridge.db.get_file(file_id).unwrap().unwrap();
        assert_eq!(row.status, FileStatus::Error);
        assert!(!original.exists(), "original path is left vacant for the remote retry");
        let conflict_copy = std::fs::read_dir(&sync_root)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("conflict (conflict - ") && name.ends_with(".txt"))
            })
            .expect("local conflict copy should be renamed with a device/date suffix");
        assert_eq!(std::fs::read(conflict_copy).unwrap(), b"local conflict bytes");
    }

    #[tokio::test]
    async fn test_process_due_operations_uploads_encrypted_image_thumbnails_after_complete() {
        let dir = tempfile::tempdir().unwrap();
        let payload = dir.path().join("photo.png");
        write_test_png(&payload);

        let server = UploadMockServer::start(false);
        let master_key = [31u8; 32];
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        bridge
            .db
            .enqueue_operation(&PendingOperation {
                op_id: "op-upload-photo".into(),
                kind: OperationKind::UploadVersion,
                file_id: Some("local-photo-1".into()),
                parent_id: None,
                target_path: Some("Photos/photo.png".into()),
                metadata_json: Some(
                    serde_json::json!({
                        "operation": "create_file",
                        "name_encrypted": "{\"cipher_suite\":\"V1Aes256Gcm\"}",
                        "display_name": "photo.png",
                        "content_type": "image/png"
                    })
                    .to_string(),
                ),
                payload_path: Some(payload.to_string_lossy().into_owned()),
                base_version: None,
                base_object_version_id: None,
                attempts: 0,
                max_attempts: 5,
                next_retry_at: 0,
                last_error: None,
                backup_source_key: None,
                created_at: 100,
                updated_at: 100,
            })
            .unwrap();

        let outcome = bridge.process_due_operations(dir.path(), 200).await.unwrap();
        assert_eq!(outcome.completed_op_ids, vec!["op-upload-photo".to_string()]);
        // Task 1700: if post-complete thumbnail work failed, say WHY here
        // instead of letting the expects below fire blind.
        assert!(
            outcome.post_complete_errors.is_empty(),
            "post-complete thumbnail work failed: {:?}",
            outcome.post_complete_errors
        );

        let requests = server.finish();
        let recorded: Vec<String> = requests.iter().map(|r| format!("{} {}", r.method, r.path)).collect();
        let medium = requests
            .iter()
            .find(|r| r.method == "PUT" && r.path.starts_with("/api/v1/files/server-file-1/thumbnail?blurhash="))
            .unwrap_or_else(|| panic!("medium thumbnail upload with blurhash query — recorded requests: {recorded:?}"));
        let large = requests
            .iter()
            .find(|r| r.method == "PUT" && r.path == "/api/v1/files/server-file-1/thumbnail/large")
            .unwrap_or_else(|| panic!("large thumbnail upload — recorded requests: {recorded:?}"));

        let master_key = beebeeb_core::kdf::MasterKey::from_bytes(master_key);
        let file_key = beebeeb_core::kdf::derive_file_key(&master_key, b"server-file-1");
        let medium_plain = beebeeb_core::encrypt::decrypt_chunk_raw(&file_key, &medium.body).unwrap();
        let large_plain = beebeeb_core::encrypt::decrypt_chunk_raw(&file_key, &large.body).unwrap();

        assert_ne!(medium.body, medium_plain);
        assert_ne!(large.body, large_plain);
        assert!(medium_plain.starts_with(b"RIFF"), "medium thumbnail must be WebP");
        assert!(large_plain.starts_with(b"RIFF"), "large thumbnail must be WebP");

        let complete_index = requests
            .iter()
            .position(|r| r.method == "POST" && r.path == "/api/v1/uploads/upload-session-1/complete")
            .unwrap();
        let medium_index = requests.iter().position(|r| std::ptr::eq(r, medium)).unwrap();
        let large_index = requests.iter().position(|r| std::ptr::eq(r, large)).unwrap();
        assert!(medium_index > complete_index);
        assert!(large_index > complete_index);
    }

    /// Task 1700 guard: when post-complete thumbnail upload work fails, the
    /// failure must be VISIBLE on `TransferLoopOutcome::post_complete_errors`
    /// — never only in a `tracing::warn!` that no test subscriber captures.
    /// Product law is unchanged: the failed thumbnail still never fails the
    /// upload itself. Red-if-broken: removing the outcome plumbing (swallow
    /// silently again) fails the `post_complete_errors` assertions below.
    #[tokio::test]
    async fn test_process_due_operations_reports_failed_thumbnail_upload_in_outcome() {
        let dir = tempfile::tempdir().unwrap();
        let payload = dir.path().join("photo.png");
        write_test_png(&payload);

        let server = UploadMockServer::start_failing_thumbnail_uploads(false);
        let master_key = [31u8; 32];
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        bridge
            .db
            .enqueue_operation(&PendingOperation {
                op_id: "op-upload-photo-thumb-fail".into(),
                kind: OperationKind::UploadVersion,
                file_id: Some("local-photo-1".into()),
                parent_id: None,
                target_path: Some("Photos/photo.png".into()),
                metadata_json: Some(
                    serde_json::json!({
                        "operation": "create_file",
                        "name_encrypted": "{\"cipher_suite\":\"V1Aes256Gcm\"}",
                        "display_name": "photo.png",
                        "content_type": "image/png"
                    })
                    .to_string(),
                ),
                payload_path: Some(payload.to_string_lossy().into_owned()),
                base_version: None,
                base_object_version_id: None,
                attempts: 0,
                max_attempts: 5,
                next_retry_at: 0,
                last_error: None,
                backup_source_key: None,
                created_at: 100,
                updated_at: 100,
            })
            .unwrap();

        let outcome = bridge.process_due_operations(dir.path(), 200).await.unwrap();

        // The upload itself still completes — post-complete work is
        // best-effort by contract.
        assert_eq!(
            outcome.completed_op_ids,
            vec!["op-upload-photo-thumb-fail".to_string()],
            "a failed thumbnail must never fail the upload"
        );
        // But the failure is no longer swallowed: the outcome names it.
        assert_eq!(
            outcome.post_complete_errors.len(),
            1,
            "the failed thumbnail upload must be reported exactly once: {outcome:?}"
        );
        assert!(
            outcome.post_complete_errors[0].contains("op-upload-photo-thumb-fail"),
            "the report carries the op id: {:?}",
            outcome.post_complete_errors
        );
        assert!(
            outcome.post_complete_errors[0].contains("upload medium thumbnail"),
            "the report names the variant that failed: {:?}",
            outcome.post_complete_errors
        );
        assert!(
            outcome.post_complete_errors[0].contains("HTTP 500"),
            "the report carries the server's status: {:?}",
            outcome.post_complete_errors
        );

        // The mock recorded the medium PUT before answering 500 — the
        // failure is server-side, not a lost request. Medium is attempted
        // first and its failure short-circuits, so large is never sent.
        let requests = server.finish();
        assert!(
            requests
                .iter()
                .any(|r| r.method == "PUT" && r.path.starts_with("/api/v1/files/server-file-1/thumbnail?blurhash=")),
            "the medium thumbnail PUT must reach the server: {:?}",
            requests
                .iter()
                .map(|r| format!("{} {}", r.method, r.path))
                .collect::<Vec<_>>()
        );
        assert!(
            !requests
                .iter()
                .any(|r| r.method == "PUT" && r.path == "/api/v1/files/server-file-1/thumbnail/large"),
            "the medium failure short-circuits before the large variant"
        );
    }

    #[tokio::test]
    async fn test_process_due_operations_preserves_payload_when_upload_chunk_fails() {
        let dir = tempfile::tempdir().unwrap();
        let payload = dir.path().join("payload.txt");
        std::fs::write(&payload, b"retry me").unwrap();
        let server = UploadMockServer::start(true);
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), [12u8; 32]);
        bridge
            .db
            .enqueue_operation(&PendingOperation {
                op_id: "op-upload-retry".into(),
                kind: OperationKind::UploadVersion,
                file_id: Some("local-file-2".into()),
                parent_id: None,
                target_path: Some("retry.txt".into()),
                metadata_json: Some(
                    serde_json::json!({
                        "operation": "create_file",
                        "name_encrypted": "{\"cipher_suite\":\"V1Aes256Gcm\"}",
                        "display_name": "retry.txt",
                        "content_type": "text/plain"
                    })
                    .to_string(),
                ),
                payload_path: Some(payload.to_string_lossy().into_owned()),
                base_version: None,
                base_object_version_id: None,
                attempts: 0,
                max_attempts: 5,
                next_retry_at: 0,
                last_error: None,
                backup_source_key: None,
                created_at: 100,
                updated_at: 100,
            })
            .unwrap();

        let outcome = bridge.process_due_operations(dir.path(), 200).await.unwrap();
        assert_eq!(outcome.retried_op_ids, vec!["op-upload-retry".to_string()]);
        assert!(outcome.completed_op_ids.is_empty());
        assert!(payload.exists(), "failed uploads must preserve the staged payload");

        let due_too_early = bridge.db.list_due_operations(229).unwrap();
        assert!(due_too_early.is_empty());
        let due = bridge.db.list_due_operations(230).unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].attempts, 1);
        assert!(
            due[0]
                .last_error
                .as_deref()
                .unwrap_or("")
                .contains("500 Internal Server Error")
        );

        let requests = server.finish();
        assert_eq!(requests.len(), 3);
        assert_eq!(requests[0].path, "/api/v1/uploads/init");
        assert_eq!(requests[1].path, "/api/v1/files/server-file-1");
        assert_eq!(requests[2].path, "/api/v1/uploads/upload-session-1/chunks/0");
    }

    // ── Transfer progress + activity (task 1683 slice 2) ─────────────────────

    fn slice2_upload_op(op_id: &str, file_id: &str, target: &str, payload: &Path) -> PendingOperation {
        PendingOperation {
            op_id: op_id.into(),
            kind: OperationKind::UploadVersion,
            file_id: Some(file_id.into()),
            parent_id: None,
            target_path: Some(target.into()),
            metadata_json: Some(
                serde_json::json!({
                    "operation": "create_file",
                    "name_encrypted": "{\"cipher_suite\":\"V1Aes256Gcm\"}",
                    "display_name": "report.txt",
                    "content_type": "text/plain"
                })
                .to_string(),
            ),
            payload_path: Some(payload.to_string_lossy().into_owned()),
            base_version: None,
            base_object_version_id: None,
            attempts: 0,
            max_attempts: 5,
            next_retry_at: 0,
            last_error: None,
            backup_source_key: None,
            created_at: 100,
            updated_at: 100,
        }
    }

    #[tokio::test]
    async fn a_finished_upload_adds_its_bytes_to_the_batch_and_records_an_up_row() {
        let dir = tempfile::tempdir().unwrap();
        let payload = dir.path().join("payload.txt");
        std::fs::write(&payload, b"live upload payload").unwrap();
        let server = UploadMockServer::start(false);
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), [11u8; 32]);
        bridge
            .db
            .enqueue_operation(&slice2_upload_op(
                "op-1",
                "local-file-1",
                "Reports/report.txt",
                &payload,
            ))
            .unwrap();

        let outcome = bridge.process_due_operations(dir.path(), 200).await.unwrap();
        assert_eq!(outcome.completed_op_ids, vec!["op-1".to_string()]);
        server.finish();

        assert_eq!(bridge.transfers().finished_bytes(), 19, "the payload is 19 bytes");
        assert!(
            bridge.transfers().active().is_empty(),
            "nothing is in flight after the upload"
        );
        let rows = bridge.db.list_recent_transfer_activity(5).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].direction, "up");
        assert_eq!(rows[0].file_name, "report.txt");
        assert_eq!(rows[0].rel_path.as_deref(), Some("Reports/report.txt"));
        assert_eq!(rows[0].bytes, 19);
        assert_eq!(rows[0].file_id.as_deref(), Some("server-file-1"));
    }

    #[tokio::test]
    async fn a_failed_upload_leaves_nothing_on_the_board_and_no_activity_row() {
        let dir = tempfile::tempdir().unwrap();
        let payload = dir.path().join("payload.txt");
        std::fs::write(&payload, b"retry me").unwrap();
        let server = UploadMockServer::start(true);
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), [12u8; 32]);
        bridge
            .db
            .enqueue_operation(&slice2_upload_op("op-2", "local-file-2", "retry.txt", &payload))
            .unwrap();

        let outcome = bridge.process_due_operations(dir.path(), 200).await.unwrap();
        assert_eq!(outcome.retried_op_ids, vec!["op-2".to_string()]);
        server.finish();

        assert!(
            bridge.transfers().active().is_empty(),
            "a failed upload must not linger as in flight"
        );
        assert_eq!(
            bridge.transfers().finished_bytes(),
            0,
            "a failed upload finishes no bytes"
        );
        assert!(bridge.db.list_recent_transfer_activity(5).unwrap().is_empty());
    }

    #[test]
    fn a_hydrate_reports_per_file_bytes_and_records_a_down_row() {
        let dir = tempfile::tempdir().unwrap();
        let master_key = [9u8; 32];
        let file_key = hydration_test_key(master_key, TEST_FILE_ID);
        let chunks: Vec<Vec<u8>> = vec![vec![b'a'; 10], vec![b'b'; 10], vec![b'c'; 10]];
        // 1 metadata GET + 3 chunk GETs.
        let server = HydrationMockServer::start(file_key, chunks, 4);
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_bridge_row(
            &bridge,
            TEST_FILE_ID,
            "Photos/three-chunks.bin",
            None,
            FileStatus::CloudOnly,
            30,
        );
        let dest = dir.path().join("three-chunks.bin");

        // Read the board from inside the caller's own progress callback: that is the
        // moment a popover snapshot would see mid-transfer.
        let seen: Arc<Mutex<Vec<(u64, u64, u64, u64)>>> = Arc::new(Mutex::new(Vec::new()));
        let board = bridge.transfers().clone();
        let seen_in = seen.clone();
        let progress = move |done: u64, total: u64| {
            let on_board = board.get(TEST_FILE_ID).expect("in flight while reporting");
            seen_in
                .lock()
                .unwrap()
                .push((done, total, on_board.done, on_board.total));
        };
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async {
                bridge
                    .hydrate_file_with_progress(TEST_FILE_ID, &dest, &[dir.path()], Some(&progress))
                    .await
            })
            .unwrap();
        server.finish();

        assert_eq!(
            *seen.lock().unwrap(),
            vec![(0, 30, 0, 30), (10, 30, 10, 30), (20, 30, 20, 30), (30, 30, 30, 30)],
            "the caller still gets every report, and the board already holds the same bytes and the same total"
        );
        assert!(bridge.transfers().active().is_empty());
        assert_eq!(bridge.transfers().finished_bytes(), 30);
        let rows = bridge.db.list_recent_transfer_activity(5).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(
            (rows[0].direction.as_str(), rows[0].file_name.as_str()),
            ("down", "three-chunks.bin")
        );
        assert_eq!(rows[0].bytes, 30);
    }

    #[test]
    fn a_hydrate_without_a_caller_callback_still_fills_the_board() {
        let dir = tempfile::tempdir().unwrap();
        let master_key = [9u8; 32];
        let file_key = hydration_test_key(master_key, TEST_FILE_ID);
        let server = HydrationMockServer::start(file_key, vec![vec![b'x'; 12]], 2);
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_bridge_row(&bridge, TEST_FILE_ID, "one.bin", None, FileStatus::CloudOnly, 12);
        let dest = dir.path().join("one.bin");
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async { bridge.hydrate_file(TEST_FILE_ID, &dest, &[dir.path()]).await })
            .unwrap();
        server.finish();
        assert_eq!(bridge.transfers().finished_bytes(), 12);
    }

    // ── Interrupted upload resume (flow 7, harness STEP 9) ──────────────────
    //
    // A desktop upload cut mid-transfer (tokio timeout on the transfer loop,
    // app quit, network drop) used to re-run `POST /uploads/init` on retry:
    // every retry minted a NEW server file row + session, re-sent every chunk
    // from zero, and left the first attempt's row behind as a visible broken
    // `is_uploading` duplicate (GET → 409) until the server's 7-day
    // stale-upload sweep. The retry must resume the persisted session instead.

    /// Stateful upload mock: 3-chunk plan (8+8+4 bytes). Each connection is
    /// served on its own thread so a deliberately hung chunk response cannot
    /// block the retry's requests.
    struct ResumableUploadMock {
        base_url: String,
        requests: Arc<Mutex<Vec<RecordedRequest>>>,
        stop: Arc<std::sync::atomic::AtomicBool>,
        handle: thread::JoinHandle<()>,
    }

    #[derive(Clone, Copy, PartialEq)]
    enum ResumeMockMode {
        /// Chunk 1 of the first session hangs once (the cut), then the
        /// session keeps accepting chunks.
        HangOnce,
        /// Chunk 1 hangs once, and afterwards session 1 is gone server-side
        /// (404 on every further chunk/complete) — e.g. reaped.
        HangOnceThenSessionGone,
    }

    struct ResumeMockState {
        inits: usize,
        hung: bool,
    }

    impl ResumableUploadMock {
        fn start(mode: ResumeMockMode) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let base_url = format!("http://{}", listener.local_addr().unwrap());
            let requests = Arc::new(Mutex::new(Vec::new()));
            let state = Arc::new(Mutex::new(ResumeMockState { inits: 0, hung: false }));
            let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let server_requests = Arc::clone(&requests);
            let server_stop = Arc::clone(&stop);
            let handle = thread::spawn(move || {
                let started = std::time::Instant::now();
                while !server_stop.load(Ordering::SeqCst) && started.elapsed() < Duration::from_secs(20) {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            stream.set_nonblocking(false).unwrap();
                            let requests = Arc::clone(&server_requests);
                            let state = Arc::clone(&state);
                            thread::spawn(move || {
                                let request = read_http_request(&mut stream);
                                requests.lock().unwrap().push(request.clone());
                                let (delay, response) = resumable_mock_response(&request, &state, mode);
                                if let Some(delay) = delay {
                                    std::thread::sleep(delay);
                                }
                                // The client may have been cancelled meanwhile.
                                let _ = stream.write_all(response.as_bytes());
                            });
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Err(e) => panic!("resumable upload mock accept failed: {e}"),
                    }
                }
            });
            Self {
                base_url,
                requests,
                stop,
                handle,
            }
        }

        fn finish(self) -> Vec<RecordedRequest> {
            self.stop.store(true, Ordering::SeqCst);
            self.handle.join().unwrap();
            self.requests.lock().unwrap().clone()
        }
    }

    fn resumable_mock_response(
        request: &RecordedRequest,
        state: &Arc<Mutex<ResumeMockState>>,
        mode: ResumeMockMode,
    ) -> (Option<Duration>, String) {
        let method = request.method.as_str();
        let path = request.path.as_str();
        if method == "POST" && path == "/api/v1/uploads/init" {
            let n = {
                let mut s = state.lock().unwrap();
                s.inits += 1;
                s.inits
            };
            return (
                None,
                http_json(
                    "201 Created",
                    serde_json::json!({
                        "file_id": format!("server-file-{n}"),
                        "tenant_id": "tenant-1",
                        "object_version_id": format!("object-init-{n}"),
                        "upload_session_id": format!("session-{n}"),
                        "chunk_size_bytes": 8,
                        "chunk_count": 3,
                        "storage_format_version": 2,
                        "storage_pool_id": "pool-1",
                        "region": "local"
                    }),
                ),
            );
        }
        if method == "PATCH" && path.starts_with("/api/v1/files/server-file-") {
            return (None, http_json("200 OK", serde_json::json!({ "ok": true })));
        }
        if method == "DELETE" && path.starts_with("/api/v1/files/server-file-") {
            return (None, http_json("200 OK", serde_json::json!({ "trashed": true })));
        }
        if let Some(rest) = path.strip_prefix("/api/v1/uploads/") {
            let mut parts = rest.split('/');
            let session = parts.next().unwrap_or_default().to_string();
            let action = parts.next().unwrap_or_default();
            let hung_before = state.lock().unwrap().hung;
            if mode == ResumeMockMode::HangOnceThenSessionGone && session == "session-1" && hung_before {
                return (
                    None,
                    http_json("404 Not Found", serde_json::json!({ "error": "not found" })),
                );
            }
            if method == "PUT" && action == "chunks" {
                let index: u32 = parts.next().unwrap_or("0").parse().unwrap();
                if index == 1 && session == "session-1" {
                    let mut s = state.lock().unwrap();
                    if !s.hung {
                        s.hung = true;
                        return (
                            Some(Duration::from_secs(3)),
                            http_json("200 OK", serde_json::json!({ "index": 1, "size": 0, "skipped": false })),
                        );
                    }
                }
                return (
                    None,
                    http_json(
                        "200 OK",
                        serde_json::json!({ "index": index, "size": request.body.len() as i64, "skipped": false }),
                    ),
                );
            }
            if method == "POST" && action == "complete" {
                let n = session.trim_start_matches("session-");
                return (
                    None,
                    http_json(
                        "200 OK",
                        serde_json::json!({
                            "file_id": format!("server-file-{n}"),
                            "version_number": 1,
                            "current_object_version_id": format!("object-complete-{n}"),
                            "size_bytes": 20,
                            "mime_type": "text/plain"
                        }),
                    ),
                );
            }
        }
        (
            None,
            http_json(
                "404 Not Found",
                serde_json::json!({ "error": format!("unexpected {method} {path}") }),
            ),
        )
    }

    fn resumable_create_op(payload: &Path) -> PendingOperation {
        PendingOperation {
            op_id: "op-upload-resume".into(),
            kind: OperationKind::UploadVersion,
            file_id: Some("local-file-resume".into()),
            parent_id: None,
            target_path: Some("resume.bin".into()),
            metadata_json: Some(
                serde_json::json!({
                    "operation": "create_file",
                    "name_encrypted": "{\"cipher_suite\":\"V1Aes256Gcm\"}",
                    "display_name": "resume.bin",
                    "content_type": "text/plain"
                })
                .to_string(),
            ),
            payload_path: Some(payload.to_string_lossy().into_owned()),
            base_version: None,
            base_object_version_id: None,
            attempts: 0,
            max_attempts: 5,
            next_retry_at: 0,
            last_error: None,
            backup_source_key: None,
            created_at: 100,
            updated_at: 100,
        }
    }

    fn count_requests(requests: &[RecordedRequest], method: &str, path: &str) -> usize {
        requests.iter().filter(|r| r.method == method && r.path == path).count()
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn round5_lock_during_keep_mine_restores_conflict_and_removes_staging() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("sync");
        std::fs::create_dir_all(&root).unwrap();
        let original = root.join("conflict.bin");
        std::fs::write(&original, b"0123456789abcdefghij").unwrap();
        let server = ResumableUploadMock::start(ResumeMockMode::HangOnce);
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), [21; 32]);
        bridge
            .db
            .upsert_file(&FileEntry {
                file_id: "server-file-1".into(),
                path: "conflict.bin".into(),
                status: FileStatus::Conflict,
                size_bytes: 20,
                modified_at: 100,
                content_hash: None,
                remote_updated_at: 90,
                parent_id: None,
                item_kind: ItemKind::File,
            })
            .unwrap();
        let commands = crate::session_commands::SessionCommands::new();
        commands.open().unwrap();
        let held = async {
            tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    if server
                        .requests
                        .lock()
                        .unwrap()
                        .iter()
                        .any(|r| r.path.ends_with("/chunks/1"))
                    {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            })
            .await
            .expect("positive control: upload reached held second chunk");
        };
        let work = commands.run(async {
            bridge
                .resolve_keep_mine("server-file-1", &root)
                .await
                .map_err(|e| e.to_string())
        });
        let mut work = Box::pin(work);
        tokio::select! { biased; _ = held => (), result = &mut work => panic!("upload completed before lock: {result:?}") }
        let conn = rusqlite::Connection::open(dir.path().join("state.db")).unwrap();
        let staged: Vec<String> = conn
            .prepare("SELECT payload_path FROM upload_resume")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(staged.len(), 1, "positive control: resume owns staged upload");
        assert!(std::path::Path::new(&staged[0]).is_file());
        let revoked = commands.close();
        assert!(work.await.is_err());
        commands.drain(revoked, Duration::from_secs(1)).unwrap();
        let requests = server.finish();
        assert_eq!(
            count_requests(&requests, "PUT", "/api/v1/uploads/session-1/chunks/1"),
            1
        );
        assert_eq!(
            bridge.db.get_file("server-file-1").unwrap().unwrap().status,
            FileStatus::Conflict,
            "cancelled Keep Mine must restore the conflict state"
        );
        assert_eq!(
            staged.iter().filter(|p| std::path::Path::new(p).exists()).count(),
            0,
            "cancelled Keep Mine left orphaned plaintext"
        );
        assert_eq!(std::fs::read(&original).unwrap(), b"0123456789abcdefghij");
        assert!(
            bridge.db.windows_signout_preflight().is_err(),
            "sign-out must preserve the original unsynced conflict"
        );
        // User resolves/moves the original; subsequent account purge leaves zero staged plaintext.
        bridge.db.set_status("server-file-1", FileStatus::Local).unwrap();
        bridge.db.windows_signout_preflight().unwrap();
        bridge.db.finish_windows_signout().unwrap();
        assert_eq!(staged.iter().filter(|p| std::path::Path::new(p).exists()).count(), 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn flow7_cancelled_upload_resumes_session_instead_of_reinit() {
        let dir = tempfile::tempdir().unwrap();
        let payload = dir.path().join("resume.bin");
        std::fs::write(&payload, b"0123456789abcdefghij").unwrap();
        let server = ResumableUploadMock::start(ResumeMockMode::HangOnce);
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), [21u8; 32]);
        bridge.db.enqueue_operation(&resumable_create_op(&payload)).unwrap();

        // The cut: chunk 1's response hangs; the transfer loop is cancelled
        // exactly like the harness's tokio timeout (the future is dropped).
        let cut = tokio::time::timeout(
            Duration::from_millis(1500),
            bridge.process_due_operations(dir.path(), 200),
        )
        .await;
        assert!(cut.is_err(), "the first pass must be cut mid-upload");

        let outcome = bridge.process_due_operations(dir.path(), 200).await.unwrap();
        assert_eq!(outcome.completed_op_ids, vec!["op-upload-resume".to_string()]);

        let requests = server.finish();
        let inits = count_requests(&requests, "POST", "/api/v1/uploads/init");
        assert_eq!(
            inits, 1,
            "retry must resume the persisted session, not init a second file row"
        );
        assert_eq!(
            count_requests(&requests, "PUT", "/api/v1/uploads/session-1/chunks/0"),
            1,
            "chunk 0 was acknowledged before the cut and must not be re-sent"
        );
        assert_eq!(
            count_requests(&requests, "PUT", "/api/v1/uploads/session-1/chunks/1"),
            2
        );
        assert_eq!(
            count_requests(&requests, "PUT", "/api/v1/uploads/session-1/chunks/2"),
            1
        );
        assert_eq!(
            count_requests(&requests, "POST", "/api/v1/uploads/session-1/complete"),
            1
        );
        assert_eq!(
            requests.iter().filter(|r| r.method == "DELETE").count(),
            0,
            "a resumable session must not be trashed"
        );

        let entry = bridge.db.get_file("server-file-1").unwrap().unwrap();
        assert_eq!(entry.status, FileStatus::Local);
        assert!(
            bridge.db.get_upload_resume("op-upload-resume").unwrap().is_none(),
            "resume state must be cleared once the upload completes"
        );
    }

    /// Poll the board until `file_id` shows at least `done` bytes, or give up.
    async fn wait_for_upload_bytes(
        board: &Arc<crate::transfer_progress::TransferBoard>,
        file_id: &str,
        done: u64,
        within: Duration,
    ) -> Option<crate::transfer_progress::Transfer> {
        let started = std::time::Instant::now();
        while started.elapsed() < within {
            if let Some(t) = board.get(file_id)
                && t.done >= done
            {
                return Some(t);
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        None
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_upload_reports_its_bytes_chunk_by_chunk_and_a_cut_one_leaves_nothing() {
        // 20 bytes in chunks of 8 (8 + 8 + 4). Chunk 1 hangs, so after chunk 0 is
        // acknowledged the upload sits at 8 of 20: the moment a popover would look.
        let dir = tempfile::tempdir().unwrap();
        let payload = dir.path().join("resume.bin");
        std::fs::write(&payload, b"0123456789abcdefghij").unwrap();
        let server = ResumableUploadMock::start(ResumeMockMode::HangOnce);
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), [21u8; 32]);
        bridge.db.enqueue_operation(&resumable_create_op(&payload)).unwrap();
        let board = bridge.transfers().clone();

        let (cut, seen) = tokio::join!(
            tokio::time::timeout(
                Duration::from_millis(1500),
                bridge.process_due_operations(dir.path(), 200)
            ),
            wait_for_upload_bytes(&board, "local-file-resume", 8, Duration::from_millis(1400)),
        );
        assert!(cut.is_err(), "the first pass is cut mid-upload");
        let seen = seen.expect("the board showed the upload after chunk 0 was acknowledged");
        assert_eq!(
            seen,
            crate::transfer_progress::Transfer {
                direction: crate::transfer_progress::Direction::Up,
                done: 8,
                total: 20
            }
        );
        assert!(board.active().is_empty(), "a cut upload must not linger as in flight");
        assert_eq!(board.finished_bytes(), 0, "a cut upload finishes no bytes");

        let outcome = bridge.process_due_operations(dir.path(), 200).await.unwrap();
        assert_eq!(outcome.completed_op_ids, vec!["op-upload-resume".to_string()]);
        server.finish();
        assert_eq!(
            board.finished_bytes(),
            20,
            "the resumed upload finishes the whole 20 bytes"
        );
        assert!(board.active().is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_resumed_upload_starts_its_bar_at_the_acknowledged_watermark() {
        let dir = tempfile::tempdir().unwrap();
        let payload = dir.path().join("resume.bin");
        std::fs::write(&payload, b"0123456789abcdefghij").unwrap();
        let server = ResumableUploadMock::start(ResumeMockMode::HangOnce);
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), [21u8; 32]);
        let op = resumable_create_op(&payload);
        bridge.db.enqueue_operation(&op).unwrap();
        // A previous run acknowledged chunk 0 (8 bytes) of session-1 and was killed.
        bridge
            .db
            .put_upload_resume(&UploadResume {
                op_id: op.op_id.clone(),
                payload_path: payload.to_string_lossy().into_owned(),
                payload_size: 20,
                payload_mtime_ns: payload_mtime_ns(&payload),
                upload_session_id: "session-1".into(),
                server_file_id: "server-file-1".into(),
                object_version_id: "object-init-1".into(),
                chunk_size_bytes: 8,
                chunk_count: 3,
                acked_chunks: 1,
                metadata_applied: true,
                is_create: true,
                completed_version: None,
                completed_object_version_id: None,
                completed_mime_type: None,
            })
            .unwrap();
        let board = bridge.transfers().clone();
        let (cut, seen) = tokio::join!(
            tokio::time::timeout(
                Duration::from_millis(1200),
                bridge.process_due_operations(dir.path(), 200)
            ),
            wait_for_upload_bytes(&board, "local-file-resume", 8, Duration::from_millis(1000)),
        );
        assert!(cut.is_err(), "chunk 1 hangs, so the pass is cut");
        let seen = seen.expect("the resumed upload shows on the board");
        assert_eq!((seen.done, seen.total), (8, 20), "it resumes at 8 of 20, not at 0");
        let requests = server.finish();
        assert_eq!(
            count_requests(&requests, "PUT", "/api/v1/uploads/session-1/chunks/0"),
            0,
            "chunk 0 is not re-sent, so 8 bytes can only come from the watermark"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn flow7_gone_session_trashes_orphan_then_reuploads_once() {
        let dir = tempfile::tempdir().unwrap();
        let payload = dir.path().join("resume.bin");
        std::fs::write(&payload, b"0123456789abcdefghij").unwrap();
        let server = ResumableUploadMock::start(ResumeMockMode::HangOnceThenSessionGone);
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), [22u8; 32]);
        bridge.db.enqueue_operation(&resumable_create_op(&payload)).unwrap();

        let cut = tokio::time::timeout(
            Duration::from_millis(1500),
            bridge.process_due_operations(dir.path(), 200),
        )
        .await;
        assert!(cut.is_err(), "the first pass must be cut mid-upload");
        assert!(bridge.db.get_upload_resume("op-upload-resume").unwrap().is_some());

        // Second pass: the persisted session is gone → the orphan create row is
        // trashed, resume state dropped, and the op scheduled for retry.
        let outcome = bridge.process_due_operations(dir.path(), 200).await.unwrap();
        assert_eq!(outcome.retried_op_ids, vec!["op-upload-resume".to_string()]);
        assert!(bridge.db.get_upload_resume("op-upload-resume").unwrap().is_none());

        // Third pass: a fresh session uploads the file exactly once.
        let outcome = bridge.process_due_operations(dir.path(), 10_000).await.unwrap();
        assert_eq!(outcome.completed_op_ids, vec!["op-upload-resume".to_string()]);

        let requests = server.finish();
        assert_eq!(count_requests(&requests, "POST", "/api/v1/uploads/init"), 2);
        assert_eq!(
            count_requests(&requests, "DELETE", "/api/v1/files/server-file-1"),
            1,
            "the orphaned is_uploading row from the dead session must be trashed"
        );
        assert_eq!(
            count_requests(&requests, "POST", "/api/v1/uploads/session-2/complete"),
            1
        );
        assert!(bridge.db.get_file("server-file-2").unwrap().is_some());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn flow7_changed_payload_does_not_resume_stale_session() {
        let dir = tempfile::tempdir().unwrap();
        let payload = dir.path().join("resume.bin");
        std::fs::write(&payload, b"0123456789abcdefghij").unwrap();
        let server = ResumableUploadMock::start(ResumeMockMode::HangOnce);
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), [23u8; 32]);
        bridge.db.enqueue_operation(&resumable_create_op(&payload)).unwrap();

        let cut = tokio::time::timeout(
            Duration::from_millis(1500),
            bridge.process_due_operations(dir.path(), 200),
        )
        .await;
        assert!(cut.is_err());

        // The staged payload changes size before the retry: the persisted
        // chunks belong to other bytes, so the session must NOT be resumed.
        std::fs::write(&payload, b"0123456789abcdefghijKLMN").unwrap();
        let outcome = bridge.process_due_operations(dir.path(), 200).await.unwrap();
        assert_eq!(outcome.completed_op_ids, vec!["op-upload-resume".to_string()]);

        let requests = server.finish();
        assert_eq!(count_requests(&requests, "POST", "/api/v1/uploads/init"), 2);
        assert_eq!(count_requests(&requests, "DELETE", "/api/v1/files/server-file-1"), 1);
        assert_eq!(
            count_requests(&requests, "POST", "/api/v1/uploads/session-1/complete"),
            0
        );
        assert_eq!(
            count_requests(&requests, "POST", "/api/v1/uploads/session-2/complete"),
            1
        );
    }

    #[tokio::test]
    async fn flow7_given_up_create_upload_trashes_its_orphan_row() {
        let dir = tempfile::tempdir().unwrap();
        let payload = dir.path().join("payload.txt");
        std::fs::write(&payload, b"retry me").unwrap();
        let server = UploadMockServer::start(true);
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), [24u8; 32]);
        let mut op = resumable_create_op(&payload);
        op.max_attempts = 1;
        bridge.db.enqueue_operation(&op).unwrap();

        let outcome = bridge.process_due_operations(dir.path(), 200).await.unwrap();
        assert_eq!(outcome.retried_op_ids, vec!["op-upload-resume".to_string()]);
        assert!(
            bridge.db.list_due_operations(i64::MAX).unwrap().is_empty(),
            "op exhausted its retries"
        );
        assert!(bridge.db.get_upload_resume("op-upload-resume").unwrap().is_none());

        let requests = server.finish();
        assert_eq!(requests.len(), 4);
        assert_eq!(requests[2].path, "/api/v1/uploads/upload-session-1/chunks/0");
        assert_eq!(requests[3].method, "DELETE");
        assert_eq!(requests[3].path, "/api/v1/files/server-file-1");
    }

    #[test]
    fn flow7_snapshot_skips_new_is_uploading_rows() {
        let dir = tempfile::tempdir().unwrap();
        let bridge = test_bridge(&dir.path().join("state.db"));
        let mut conflicts = Vec::new();
        let row = serde_json::json!({
            "id": "partial-upload-1",
            "parent_id": null,
            "name": "resume.bin",
            "size_bytes": 50331655,
            "is_folder": false,
            "is_uploading": true,
            "updated_at": 100
        });
        let resolved = process_metadata_row(&bridge, &row, "", 200, RowSource::Snapshot, &mut conflicts).unwrap();
        assert!(resolved.is_none());
        assert!(
            bridge.db.get_file("partial-upload-1").unwrap().is_none(),
            "an in-progress upload has no content yet and must not become a placeholder"
        );
    }

    #[test]
    fn test_read_only_shared_item_rejects_finder_writes() {
        let dir = tempfile::tempdir().unwrap();
        let bridge = test_bridge(&dir.path().join("state.db"));
        seed_bridge_row(
            &bridge,
            "shared-readonly",
            "/Shared with me/Readonly.txt",
            None,
            FileStatus::Local,
            100,
        );
        let mut contract = bridge.db.get_file_contract_state("shared-readonly").unwrap().unwrap();
        contract.namespace = Namespace::SharedWithMe;
        contract.shared_root_id = Some("shared-readonly".into());
        contract.share_id = Some("invite-1".into());
        contract.permission_bits = PERMISSION_READ;
        bridge.db.set_file_contract_state(&contract).unwrap();

        let err = bridge
            .queue_finder_delete("shared-readonly", Some("1:1:100".into()))
            .unwrap_err()
            .to_string();
        assert!(err.contains("read-only shared item"));
        assert!(bridge.db.list_due_operations(now_secs()).unwrap().is_empty());
    }

    #[test]
    fn test_editable_shared_write_queues_actor_and_share_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("shared.txt");
        std::fs::write(&source, b"shared payload").unwrap();
        let bridge = test_bridge(&dir.path().join("state.db"));
        seed_bridge_row(
            &bridge,
            "shared-editable",
            "/Shared with me/Editable.txt",
            None,
            FileStatus::Local,
            100,
        );
        let mut contract = bridge.db.get_file_contract_state("shared-editable").unwrap().unwrap();
        contract.namespace = Namespace::SharedWithMe;
        contract.shared_root_id = Some("shared-root".into());
        contract.share_id = Some("invite-2".into());
        contract.permission_bits = PERMISSION_READ | PERMISSION_WRITE;
        contract.current_object_version_id = Some("object-1".into());
        bridge.db.set_file_contract_state(&contract).unwrap();

        bridge
            .queue_finder_modify(FinderWriteTarget {
                file_id: Some("shared-editable".into()),
                parent_id: None,
                filename: "Editable.txt".into(),
                rel_path: None,
                kind: FinderWriteItemKind::File,
                contents_path: Some(source.to_string_lossy().into_owned()),
                content_type: Some("text/plain".into()),
                base_version_identifier: Some("3:2:100".into()),
            })
            .unwrap();

        let queued = bridge.db.list_due_operations(now_secs()).unwrap();
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].kind, OperationKind::UploadVersion);
        assert_eq!(queued[0].base_version, Some(3));
        assert_eq!(queued[0].base_object_version_id.as_deref(), Some("object-1"));
        let metadata: serde_json::Value = serde_json::from_str(queued[0].metadata_json.as_deref().unwrap()).unwrap();
        assert_eq!(metadata["shared_root_id"], "shared-root");
        assert_eq!(metadata["share_id"], "invite-2");
        assert_eq!(metadata["uploaded_by"], "authenticated_desktop_user");
    }

    #[test]
    fn test_version_center_feed_maps_conflicts_and_review_operations() {
        let dir = tempfile::tempdir().unwrap();
        let bridge = test_bridge(&dir.path().join("state.db"));
        bridge
            .db
            .upsert_file(&FileEntry {
                file_id: "conflict-file".into(),
                path: "/Work/conflict.md".into(),
                status: FileStatus::Conflict,
                size_bytes: 64,
                modified_at: 100,
                content_hash: Some("local".into()),
                remote_updated_at: 90,
                parent_id: None,
                item_kind: ItemKind::File,
            })
            .unwrap();
        bridge
            .db
            .enqueue_operation(&PendingOperation {
                op_id: "op-quota".into(),
                kind: OperationKind::UploadVersion,
                file_id: Some("quota-file".into()),
                parent_id: None,
                target_path: Some("/Work/quota.png".into()),
                metadata_json: Some(r#"{"operation":"upload_version"}"#.into()),
                payload_path: Some("/tmp/quota".into()),
                base_version: None,
                base_object_version_id: None,
                attempts: 3,
                max_attempts: 3,
                next_retry_at: 100,
                last_error: Some("quota exceeded".into()),
                backup_source_key: None,
                created_at: 101,
                updated_at: 101,
            })
            .unwrap();
        bridge
            .db
            .enqueue_operation(&PendingOperation {
                op_id: "op-permission".into(),
                kind: OperationKind::MoveFile,
                file_id: Some("shared-file".into()),
                parent_id: Some("shared-folder".into()),
                target_path: Some("/Shared/blocked.txt".into()),
                metadata_json: Some(r#"{"operation":"metadata_update"}"#.into()),
                payload_path: None,
                base_version: None,
                base_object_version_id: None,
                attempts: 1,
                max_attempts: 5,
                next_retry_at: 100,
                last_error: Some("403 forbidden: permission denied".into()),
                backup_source_key: None,
                created_at: 102,
                updated_at: 102,
            })
            .unwrap();
        bridge
            .db
            .enqueue_operation(&PendingOperation {
                op_id: "op-stale".into(),
                kind: OperationKind::UploadVersion,
                file_id: Some("stale-file".into()),
                parent_id: None,
                target_path: Some("/Work/stale.txt".into()),
                metadata_json: Some(r#"{"operation":"upload_version","base_version_identifier":"7:1700:12"}"#.into()),
                payload_path: Some("/tmp/stale".into()),
                base_version: Some(7),
                base_object_version_id: None,
                attempts: 0,
                max_attempts: 25,
                next_retry_at: 100,
                last_error: Some("stale base version rejected".into()),
                backup_source_key: None,
                created_at: 103,
                updated_at: 103,
            })
            .unwrap();

        let feed = bridge.version_conflict_feed().unwrap();
        assert!(feed.iter().any(|entry| {
            entry.kind == "conflict"
                && entry.file_id == "conflict-file"
                && entry.file_name == "conflict.md"
                && entry.action == "open_conflict"
        }));
        assert!(
            feed.iter()
                .any(|entry| entry.kind == "quota_failure" && entry.file_name == "quota.png")
        );
        assert!(
            feed.iter()
                .any(|entry| entry.kind == "permission_failure" && entry.file_name == "blocked.txt")
        );
        assert!(feed.iter().any(|entry| {
            entry.kind == "stale_base"
                && entry.file_name == "stale.txt"
                && entry.base_version == Some(7)
                && entry.detail.contains("preserved")
        }));
    }

    #[test]
    fn test_restore_version_routes_to_durable_operation_for_review() {
        let dir = tempfile::tempdir().unwrap();
        let bridge = test_bridge(&dir.path().join("state.db"));

        let queued = bridge
            .queue_restore_version(
                "file-restore",
                "version-2",
                Some("direct restore failed: network offline".into()),
            )
            .unwrap();

        assert_eq!(queued.kind, OperationKind::RestoreVersion);
        assert_eq!(queued.file_id.as_deref(), Some("file-restore"));
        assert_eq!(queued.base_object_version_id.as_deref(), Some("version-2"));
        let metadata: serde_json::Value = serde_json::from_str(queued.metadata_json.as_deref().unwrap()).unwrap();
        assert_eq!(metadata["operation"], "restore_version");
        assert_eq!(metadata["version_id"], "version-2");

        let feed = bridge.version_conflict_feed().unwrap();
        assert!(feed.iter().any(|entry| {
            entry.kind == "restore"
                && entry.file_id == "file-restore"
                && entry.version_id.as_deref() == Some("version-2")
                && entry.last_error.as_deref() == Some("direct restore failed: network offline")
        }));
    }

    fn seed_bridge_row(
        bridge: &EngineBridge,
        file_id: &str,
        path: &str,
        parent_id: Option<&str>,
        status: FileStatus,
        cache_bytes: i64,
    ) {
        bridge
            .db
            .upsert_file(&FileEntry {
                file_id: file_id.into(),
                path: path.into(),
                status,
                size_bytes: cache_bytes,
                modified_at: 0,
                content_hash: None,
                remote_updated_at: 0,
                parent_id: parent_id.map(str::to_string),
                item_kind: ItemKind::File,
            })
            .unwrap();
        let mut contract = bridge.db.get_file_contract_state(file_id).unwrap().unwrap();
        contract.parent_id = parent_id.map(str::to_string);
        contract.cache_bytes = cache_bytes;
        bridge.db.set_file_contract_state(&contract).unwrap();
    }

    fn seed_bridge_entry(
        bridge: &EngineBridge,
        file_id: &str,
        path: &str,
        parent_id: Option<&str>,
        status: FileStatus,
        is_folder: bool,
        remote_updated_at: i64,
    ) {
        let item_kind = if is_folder { ItemKind::Folder } else { ItemKind::File };
        bridge
            .db
            .upsert_file(&FileEntry {
                file_id: file_id.into(),
                path: path.into(),
                status,
                size_bytes: if is_folder { 0 } else { 10 },
                modified_at: remote_updated_at,
                content_hash: None,
                remote_updated_at,
                parent_id: parent_id.map(str::to_string),
                item_kind: item_kind.clone(),
            })
            .unwrap();
        let mut contract = bridge.db.get_file_contract_state(file_id).unwrap().unwrap();
        contract.namespace = Namespace::MyFiles;
        contract.parent_id = parent_id.map(str::to_string);
        contract.item_kind = item_kind;
        contract.permission_bits = PERMISSION_READ | PERMISSION_WRITE | PERMISSION_OWNER;
        bridge.db.set_file_contract_state(&contract).unwrap();
    }

    fn enqueue_test_operation(bridge: &EngineBridge, op_id: &str, kind: OperationKind, file_id: &str, created_at: i64) {
        bridge
            .db
            .enqueue_operation(&PendingOperation {
                op_id: op_id.into(),
                kind,
                file_id: Some(file_id.into()),
                parent_id: None,
                target_path: Some(format!("{file_id}.txt")),
                metadata_json: None,
                payload_path: None,
                base_version: None,
                base_object_version_id: None,
                attempts: 0,
                max_attempts: 25,
                next_retry_at: 0,
                last_error: None,
                backup_source_key: None,
                created_at,
                updated_at: created_at,
            })
            .unwrap();
    }

    // ------------------------------------------------------------------
    // Task 1697 review fix (T3): the remote ingestion path must collect the
    // item ids it APPLIED so the runner can signal the File Provider working
    // set about REMOTE changes — before this, only local ops signaled
    // (`operations_applied`), so Finder stayed blind to server-side
    // creates/modifies/moves/deletes until an unrelated signal. RED-first:
    // written against the plumbed-but-empty collectors and seen failing (an
    // applied op returned an empty id list) before the collection landed.
    // ------------------------------------------------------------------

    #[test]
    fn tick1697_apply_sync_op_collects_the_applied_create_id() {
        let dir = tempfile::tempdir().unwrap();
        let bridge = test_bridge(&dir.path().join("state.db"));
        let file = "rem00000-0000-4000-8000-000000000001";
        let op = crate::api_client::SyncOp {
            seq_id: 3,
            op_type: "file_create".into(),
            payload: serde_json::json!({ "id": file, "parent_id": serde_json::Value::Null, "name_encrypted": "new.txt", "size_bytes": 10 }),
        };
        let mut conflicts = Vec::new();
        let applied = apply_sync_op(&bridge, dir.path(), &op, 200, &mut conflicts).unwrap();
        assert!(
            bridge.db().get_file(file).unwrap().is_some(),
            "the create must really have been ingested"
        );
        assert_eq!(
            applied,
            vec![file.to_string()],
            "an applied create op must report its file id for the working-set signal"
        );
    }

    #[test]
    fn tick1697_apply_sync_op_collects_remote_delete_subtree_ids() {
        // FLIPPED BY TASK 1698 (trash ruling — full sync): a server-side trash
        // of a folder no longer removes rows — it flips the folder AND its
        // descendants to `Trashing` (the macOS trash view mirrors the server
        // trash), reporting both ids so the replica moves the subtree into the
        // trash container. The hierarchy is PATH-based (the descendant sweep
        // is a path-prefix match), so the child must be seeded under the
        // folder's path.
        let dir = tempfile::tempdir().unwrap();
        let mk = [9u8; 32];
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), "http://placeholder".into(), mk);
        let folder = "fold0000-0000-4000-8000-000000000001";
        let child = "chil0000-0000-4000-8000-000000000002";
        seed_bridge_entry(&bridge, folder, "docs", None, FileStatus::CloudOnly, true, 10);
        seed_bridge_entry(&bridge, child, "docs/notes.txt", None, FileStatus::CloudOnly, false, 10);

        let op = crate::api_client::SyncOp {
            seq_id: 4,
            op_type: "file_trash".into(),
            payload: serde_json::json!({ "id": folder }),
        };
        let mut conflicts = Vec::new();
        let applied = apply_sync_op(&bridge, dir.path(), &op, 200, &mut conflicts).unwrap();
        assert!(
            bridge.db().get_file(folder).unwrap().is_some(),
            "the trash keeps the folder (trash view)"
        );
        assert!(
            bridge.db().get_file(child).unwrap().is_some(),
            "the trash keeps the descendant (trash view)"
        );
        for id in [folder, child] {
            assert_eq!(
                bridge.db().get_file(id).unwrap().unwrap().status,
                FileStatus::Trashing,
                "{id} must be Trashing"
            );
        }
        assert!(
            applied.contains(&folder.to_string()) && applied.contains(&child.to_string()),
            "remote delete must report the folder AND its descendants: {applied:?}"
        );
    }

    #[test]
    fn tick1697_apply_sync_op_trash_echo_of_local_trashing_collects_nothing() {
        // The server echo of a LOCAL delete-in-flight must not report ids: the
        // local queue's own completion signals those via `operations_applied`
        // (double-signaling the same item would be redundant).
        let dir = tempfile::tempdir().unwrap();
        let bridge = test_bridge(&dir.path().join("state.db"));
        let file = "echo0000-0000-4000-8000-000000000001";
        seed_bridge_entry(&bridge, file, "doomed.txt", None, FileStatus::Trashing, false, 10);
        let op = crate::api_client::SyncOp {
            seq_id: 5,
            op_type: "file_trash".into(),
            payload: serde_json::json!({ "id": file }),
        };
        let mut conflicts = Vec::new();
        let applied = apply_sync_op(&bridge, dir.path(), &op, 200, &mut conflicts).unwrap();
        assert!(
            applied.is_empty(),
            "a Trashing echo applies nothing itself: {applied:?}"
        );
        assert!(
            bridge.db().get_file(file).unwrap().is_some(),
            "the echo must leave the row to the 0802 path"
        );
    }

    #[test]
    fn tick1697_apply_snapshot_collects_ingested_and_pruned_ids() {
        // A snapshot re-bootstrap ingests nodes and prunes rows absent from it;
        // BOTH sides are remote changes Finder must see.
        let dir = tempfile::tempdir().unwrap();
        let mk = [9u8; 32];
        let bridge = test_bridge(&dir.path().join("state.db"));
        let kept = "keep0000-0000-4000-8000-000000000001";
        let absent = "abse0000-0000-4000-8000-000000000002";
        // `absent` exists locally but the snapshot below omits it → pruned.
        seed_bridge_entry(&bridge, absent, "stale.txt", None, FileStatus::CloudOnly, false, 10);
        let snapshot = crate::api_client::SyncSnapshot {
            seq_id: 9,
            nodes: vec![snap_node(&mk, kept, "fresh.txt", None, false, 50)],
        };
        let mut conflicts = Vec::new();
        let applied = apply_snapshot(&bridge, dir.path(), &snapshot, 200, 200, &mut conflicts).unwrap();
        assert!(
            bridge.db().get_file(kept).unwrap().is_some(),
            "the snapshot node must be ingested"
        );
        assert!(
            bridge.db().get_file(absent).unwrap().is_none(),
            "the row absent from the snapshot must be pruned"
        );
        assert!(
            applied.contains(&kept.to_string()),
            "ingested ids are collected: {applied:?}"
        );
        assert!(
            applied.contains(&absent.to_string()),
            "pruned (remote-deleted) ids are collected: {applied:?}"
        );
    }

    #[test]
    fn audit_1244_apply_sync_op_trash_echo_preserves_local_trashing_owner() {
        // Production mutation caught: removing a Trashing row on a file_trash echo
        // would make the local-delete-in-flight path lose its durable owner state.
        let dir = tempfile::tempdir().unwrap();
        let bridge = test_bridge(&dir.path().join("state.db"));
        let file = "echo0000-0000-4000-8000-000000000001";
        seed_bridge_entry(&bridge, file, "doomed.txt", None, FileStatus::Trashing, false, 10);
        enqueue_test_operation(&bridge, "op-trash-echo", OperationKind::TrashFile, file, 100);

        let op = crate::api_client::SyncOp {
            seq_id: 12,
            op_type: "file_trash".into(),
            payload: serde_json::json!({ "id": file }),
        };
        let mut conflicts = Vec::new();
        apply_sync_op(&bridge, dir.path(), &op, 200, &mut conflicts).unwrap();

        let row = bridge.db().get_file(file).unwrap().unwrap();
        assert_eq!(row.status, FileStatus::Trashing);
        let queued = bridge.db().list_due_operations(999).unwrap();
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].op_id, "op-trash-echo");
        assert!(conflicts.is_empty());
    }

    #[test]
    fn audit_1244_double_trash_restore_cycle_keeps_resnapshot_for_final_restore() {
        // Production mutation caught: treating file_restore as a no-op, or clearing
        // the resnapshot request during the same op batch, would leave the final
        // restored row invisible after two fast trash+restore cycles.
        let dir = tempfile::tempdir().unwrap();
        let mk = [8u8; 32];
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), "http://placeholder".into(), mk);
        let file = "cycle000-0000-4000-8000-000000000001";
        seed_bridge_entry(&bridge, file, "doc.txt", None, FileStatus::CloudOnly, false, 10);

        let mut conflicts = Vec::new();
        for (seq_id, op_type) in [
            (2, "file_trash"),
            (3, "file_restore"),
            (4, "file_trash"),
            (5, "file_restore"),
        ] {
            let op = crate::api_client::SyncOp {
                seq_id,
                op_type: op_type.into(),
                payload: serde_json::json!({ "id": file }),
            };
            apply_sync_op(&bridge, dir.path(), &op, 200, &mut conflicts).unwrap();
        }

        // Task 1698: the trash no longer removes the row — it flips it
        // `Trashing` (trash view) and the restore flips it straight back
        // (`CloudOnly`). Two fast trash+restore cycles converge to
        // CloudOnly with the resnapshot still requested.
        assert!(
            bridge.db().get_file(file).unwrap().is_some(),
            "the trash view keeps the row alive (1698); restore keeps it alive"
        );
        assert_eq!(
            bridge.db().get_file(file).unwrap().unwrap().status,
            FileStatus::CloudOnly,
            "the last restore in the batch un-trashed the row"
        );
        #[cfg(target_os = "macos")]
        assert!(
            bridge.db().peek_resnapshot_request().unwrap().is_some(),
            "at least one restore in the batch must force the next tick to snapshot"
        );
        #[cfg(not(target_os = "macos"))]
        assert!(
            bridge.db().take_needs_resnapshot().unwrap(),
            "at least one restore in the batch must force the next tick to snapshot"
        );

        let snapshot = crate::api_client::SyncSnapshot {
            seq_id: 5,
            nodes: vec![snap_node(&mk, file, "doc.txt", None, false, 50)],
        };
        apply_snapshot(&bridge, dir.path(), &snapshot, 250, 250, &mut conflicts).unwrap();

        let row = bridge.db().get_file(file).unwrap().unwrap();
        assert_eq!(row.path, "doc.txt");
        assert_eq!(row.status, FileStatus::CloudOnly);
        assert!(conflicts.is_empty());
    }

    #[test]
    fn audit_1244_gap_rebootstrap_keeps_offline_local_upload_when_remote_delete_absent() {
        // Production mutation caught: pruning rows with pending UploadVersion work
        // during a gap-recovery snapshot would silently delete a local offline edit
        // just because another device remotely deleted the server row.
        let dir = tempfile::tempdir().unwrap();
        let mk = [4u8; 32];
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), "http://placeholder".into(), mk);
        let local = "local000-0000-4000-8000-000000000001";
        let other = "other000-0000-4000-8000-000000000002";
        seed_bridge_entry(&bridge, local, "report.txt", None, FileStatus::Error, false, 10);
        enqueue_test_operation(&bridge, "op-upload-offline", OperationKind::UploadVersion, local, 100);

        let snapshot = crate::api_client::SyncSnapshot {
            seq_id: 77,
            nodes: vec![snap_node(&mk, other, "other.txt", None, false, 80)],
        };
        let mut conflicts = Vec::new();
        apply_snapshot(&bridge, dir.path(), &snapshot, 200, 200, &mut conflicts).unwrap();

        let row = bridge.db().get_file(local).unwrap().unwrap();
        assert_eq!(row.path, "report.txt");
        assert_eq!(row.status, FileStatus::Error);
        assert_eq!(
            bridge.db().list_due_operations(999).unwrap()[0].op_id,
            "op-upload-offline",
            "pending local upload remains durable after rebootstrap"
        );
        assert!(bridge.db().get_file(other).unwrap().is_some());
        assert!(conflicts.is_empty());
    }

    #[test]
    fn audit_1244_snapshot_partial_folder_trash_prunes_children_seen_without_parent() {
        // Production mutation caught: re-rooting children whose parent folder is
        // absent from the snapshot before prune_absent runs leaks ghost rows.
        let dir = tempfile::tempdir().unwrap();
        let mk = [6u8; 32];
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), "http://placeholder".into(), mk);
        let folder = "fold0000-0000-4000-8000-000000000001";
        let child = "fold0000-0000-4000-8000-000000000002";
        let subfolder = "fold0000-0000-4000-8000-000000000003";
        let grandchild = "fold0000-0000-4000-8000-000000000004";
        let sibling = "fold0000-0000-4000-8000-000000000005";

        seed_bridge_entry(&bridge, folder, "docs", None, FileStatus::CloudOnly, true, 10);
        seed_bridge_entry(
            &bridge,
            child,
            "docs/a.txt",
            Some(folder),
            FileStatus::CloudOnly,
            false,
            10,
        );
        seed_bridge_entry(
            &bridge,
            subfolder,
            "docs/sub",
            Some(folder),
            FileStatus::CloudOnly,
            true,
            10,
        );
        seed_bridge_entry(
            &bridge,
            grandchild,
            "docs/sub/b.txt",
            Some(subfolder),
            FileStatus::CloudOnly,
            false,
            10,
        );
        seed_bridge_entry(&bridge, sibling, "outside.txt", None, FileStatus::CloudOnly, false, 10);

        let snapshot = crate::api_client::SyncSnapshot {
            seq_id: 88,
            nodes: vec![
                // Concurrent partial sync: the server snapshot still contains the
                // folder's independent children, but the trashed folder node itself
                // is absent. The local mirror must remove the subtree, not re-root it.
                snap_node(&mk, child, "a.txt", Some(folder), false, 80),
                snap_node(&mk, subfolder, "sub", Some(folder), true, 80),
                snap_node(&mk, grandchild, "b.txt", Some(subfolder), false, 80),
                snap_node(&mk, sibling, "outside.txt", None, false, 80),
            ],
        };
        let mut conflicts = Vec::new();
        apply_snapshot(&bridge, dir.path(), &snapshot, 200, 200, &mut conflicts).unwrap();

        assert!(bridge.db().get_file(folder).unwrap().is_none());
        assert!(
            bridge.db().get_file(child).unwrap().is_none(),
            "child listed in a partial snapshot must still be pruned with its absent folder"
        );
        assert!(bridge.db().get_file(subfolder).unwrap().is_none());
        assert!(bridge.db().get_file(grandchild).unwrap().is_none());
        assert!(bridge.db().get_file(sibling).unwrap().is_some());
        assert!(conflicts.is_empty());
    }

    // ── /sync delta engine tests (task 0789) ──────────────────────────────────
    //
    // A scriptable mock that answers `/api/v1/sync/snapshot` and
    // `/api/v1/sync/ops` from a FIFO of canned responses (status + JSON body)
    // and records every request path. Mirrors `UploadMockServer`'s raw-TCP shape
    // (the existing test harness) so it shares the request-capture pattern the
    // 0789 plan calls out.

    struct SyncMockServer {
        base_url: String,
        requests: Arc<Mutex<Vec<RecordedRequest>>>,
        handle: thread::JoinHandle<()>,
    }

    impl SyncMockServer {
        /// `responses`: ordered `(http_status, json_body)` answered one per
        /// incoming request. The server serves exactly `responses.len()`
        /// requests then exits, so the test must drive precisely that many.
        fn start(responses: Vec<(String, serde_json::Value)>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let base_url = format!("http://{}", listener.local_addr().unwrap());
            let requests = Arc::new(Mutex::new(Vec::new()));
            let server_requests = Arc::clone(&requests);
            let handle = thread::spawn(move || {
                for (status, body) in responses {
                    let (mut stream, _) = listener.accept().unwrap();
                    let request = read_http_request(&mut stream);
                    server_requests.lock().unwrap().push(request);
                    let response = http_json(&status, body);
                    stream.write_all(response.as_bytes()).unwrap();
                }
            });
            Self {
                base_url,
                requests,
                handle,
            }
        }

        fn finish(self) -> Vec<RecordedRequest> {
            self.handle.join().unwrap();
            Arc::try_unwrap(self.requests).unwrap().into_inner().unwrap()
        }
    }

    fn enc_name(master_key: &[u8; 32], file_id: &str, name: &str) -> String {
        let mk = beebeeb_core::kdf::MasterKey::from_bytes(*master_key);
        beebeeb_core::encrypt::encrypt_name(&mk, file_id, name, None).unwrap()
    }

    /// Build a snapshot node the way the server's `/sync/snapshot` does.
    fn snap_node(
        master_key: &[u8; 32],
        id: &str,
        name: &str,
        parent_id: Option<&str>,
        is_folder: bool,
        updated_at: i64,
    ) -> serde_json::Value {
        serde_json::json!({
            "id": id,
            "parent_id": parent_id,
            "name_encrypted": enc_name(master_key, id, name),
            "size_bytes": 10,
            "is_folder": is_folder,
            "content_hash": serde_json::Value::Null,
            "version_number": 1,
            "is_trashed": false,
            "is_starred": false,
            "updated_at": updated_at,
        })
    }

    #[tokio::test]
    async fn test_sync_tick_bootstrap_snapshot_ingests_same_rows_as_walk() {
        // cursor unset → ONE /sync/snapshot bootstrap. Every node lands as a
        // cloud_only row with the decrypted nested path, and the cursor is set
        // to the snapshot's seq_id.
        let dir = tempfile::tempdir().unwrap();
        let mk = [7u8; 32];
        let folder = "f0000000-0000-4000-8000-000000000001";
        let child = "c0000000-0000-4000-8000-000000000002";
        let root_file = "a0000000-0000-4000-8000-000000000003";
        let snapshot = serde_json::json!({
            "seq_id": 12,
            "nodes": [
                snap_node(&mk, folder, "docs", None, true, 100),
                snap_node(&mk, child, "notes.txt", Some(folder), false, 100),
                snap_node(&mk, root_file, "top.txt", None, false, 100),
            ],
        });
        let server = SyncMockServer::start(vec![("200 OK".into(), snapshot)]);
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), mk);

        let conflicts = sync_tick(&bridge, dir.path()).await.unwrap();
        assert!(conflicts.is_empty());

        let requests = server.finish();
        assert_eq!(requests.len(), 1, "bootstrap = exactly one request");
        assert_eq!(requests[0].path, "/api/v1/sync/snapshot");

        // Nested + root rows all present, paths nested correctly, all cloud_only.
        let f = bridge.db().get_file(folder).unwrap().unwrap();
        assert_eq!(f.path, "docs");
        assert_eq!(f.item_kind, ItemKind::Folder);
        let c = bridge.db().get_file(child).unwrap().unwrap();
        assert_eq!(c.path, "docs/notes.txt", "child nested under parent's resolved path");
        assert_eq!(c.status, FileStatus::CloudOnly);
        let r = bridge.db().get_file(root_file).unwrap().unwrap();
        assert_eq!(r.path, "top.txt");
        // Cursor advanced to the snapshot seq_id.
        assert_eq!(bridge.db().get_sync_cursor().unwrap(), Some(12));
    }

    #[tokio::test]
    async fn test_sync_tick_snapshot_prunes_row_absent_from_snapshot() {
        // A known local row the fresh snapshot OMITS must be pruned (the silent
        // deletion-reconciliation fix).
        let dir = tempfile::tempdir().unwrap();
        let mk = [4u8; 32];
        let kept = "11111111-0000-4000-8000-000000000001";
        let stale = "22222222-0000-4000-8000-000000000002";
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), "http://placeholder".into(), mk);
        // Pre-seed a settled own-tree row that the snapshot will NOT mention.
        bridge
            .db()
            .upsert_file(&FileEntry {
                file_id: stale.into(),
                path: "deleted-server-side.txt".into(),
                status: FileStatus::CloudOnly,
                size_bytes: 1,
                modified_at: 0,
                content_hash: None,
                remote_updated_at: 0,
                parent_id: None,
                item_kind: ItemKind::File,
            })
            .unwrap();

        let snapshot = serde_json::json!({
            "seq_id": 5,
            "nodes": [ snap_node(&mk, kept, "kept.txt", None, false, 50) ],
        });
        let server = SyncMockServer::start(vec![("200 OK".into(), snapshot)]);
        // Re-point the bridge at the mock by rebuilding it on the same DB path.
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), mk);

        sync_tick(&bridge, dir.path()).await.unwrap();
        server.finish();

        assert!(bridge.db().get_file(kept).unwrap().is_some());
        assert!(
            bridge.db().get_file(stale).unwrap().is_none(),
            "snapshot-absent row pruned"
        );
    }

    #[tokio::test]
    async fn test_sync_tick_snapshot_window_does_not_resurrect_trashing_row() {
        // task 0802 — THE WINDOW (the bug this fix closes). A file was just
        // locally deleted: its row is `Trashing`, its `TrashFile` op already
        // SUCCEEDED (so the op is gone). On the very next tick the server's
        // `/sync/snapshot` read is replica-lagged / same-second and STILL lists
        // the (now-trashed) file. The OLD code's None/insert arm re-inserted a
        // FRESH `CloudOnly` row here and re-minted the placeholder — the "deleted
        // file comes back" bug. The fix: `process_metadata_row`'s `Trashing` guard
        // preserves the row untouched, and `prune_absent` does NOT remove it while
        // it's still listed. Assert: row stays `Trashing`, never `CloudOnly`.
        let dir = tempfile::tempdir().unwrap();
        let mk = [7u8; 32];
        let trashing = "33333333-0000-4000-8000-000000000003";
        // First bridge only to create the DB file; rebuilt below against the mock.
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), "http://placeholder".into(), mk);
        // Pre-seed the post-local-delete state: Trashing row, NO pending op.
        bridge
            .db()
            .upsert_file(&FileEntry {
                file_id: trashing.into(),
                path: "doomed.txt".into(),
                status: FileStatus::Trashing,
                size_bytes: 10,
                modified_at: 0,
                content_hash: None,
                remote_updated_at: 10,
                parent_id: None,
                item_kind: ItemKind::File,
            })
            .unwrap();

        // Snapshot STILL lists the trashed file (propagation lag) — exactly the
        // race window. (is_trashed flag on the node is irrelevant: the lagged
        // snapshot is what the engine sees.)
        let snapshot = serde_json::json!({
            "seq_id": 9,
            "nodes": [ snap_node(&mk, trashing, "doomed.txt", None, false, 10) ],
        });
        let server = SyncMockServer::start(vec![("200 OK".into(), snapshot)]);
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), mk);

        sync_tick(&bridge, dir.path()).await.unwrap();
        server.finish();

        let row = bridge.db().get_file(trashing).unwrap();
        assert!(
            row.is_some(),
            "Trashing row must NOT be pruned while still listed in the snapshot"
        );
        assert_eq!(
            row.unwrap().status,
            FileStatus::Trashing,
            "a snapshot still listing a Trashing file must NOT resurrect it to CloudOnly (the bug)"
        );
    }

    #[tokio::test]
    async fn test_sync_tick_snapshot_absent_keeps_trashing_row_trash_view() {
        // task 0802 — FLIPPED BY TASK 1698 (trash ruling, full sync): once the
        // trash propagates the file is ABSENT from the snapshot — and it STAYS
        // absent forever, because the snapshot never lists trashed files. The
        // op-less `Trashing` row is now the macOS TRASH VIEW: prune_absent
        // must keep it. Removal converges through the server's `file_delete`
        // (permanent) / `file_restore` ops instead (see the 1698 tests).
        let dir = tempfile::tempdir().unwrap();
        let mk = [7u8; 32];
        let trashing = "44444444-0000-4000-8000-000000000004";
        let kept = "55555555-0000-4000-8000-000000000005";
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), "http://placeholder".into(), mk);
        bridge
            .db()
            .upsert_file(&FileEntry {
                file_id: trashing.into(),
                path: "doomed.txt".into(),
                status: FileStatus::Trashing,
                size_bytes: 10,
                modified_at: 0,
                content_hash: None,
                remote_updated_at: 10,
                parent_id: None,
                item_kind: ItemKind::File,
            })
            .unwrap();

        // Snapshot no longer lists the trashed file (propagation complete); it
        // lists an unrelated row so the empty-snapshot guard doesn't trip.
        let snapshot = serde_json::json!({
            "seq_id": 9,
            "nodes": [ snap_node(&mk, kept, "kept.txt", None, false, 10) ],
        });
        let server = SyncMockServer::start(vec![("200 OK".into(), snapshot)]);
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), mk);

        sync_tick(&bridge, dir.path()).await.unwrap();
        server.finish();

        assert!(bridge.db().get_file(kept).unwrap().is_some());
        let row = bridge.db().get_file(trashing).unwrap().unwrap();
        assert_eq!(
            row.status,
            FileStatus::Trashing,
            "an op-less Trashing row absent from the snapshot is the trash view — it survives"
        );
    }

    #[tokio::test]
    async fn test_sync_tick_ops_sequence_converges_to_fresh_snapshot() {
        // Bootstrap, then apply a stream of ops, and assert the resulting mirror
        // matches what a fresh snapshot of the post-op tree would produce.
        let dir = tempfile::tempdir().unwrap();
        let mk = [9u8; 32];
        let keep = "aaaa0000-0000-4000-8000-000000000001";
        let doomed = "bbbb0000-0000-4000-8000-000000000002";
        let created = "cccc0000-0000-4000-8000-000000000003";

        // Bootstrap snapshot: {keep, doomed}, seq 1.
        let boot = serde_json::json!({
            "seq_id": 1,
            "nodes": [
                snap_node(&mk, keep, "keep.txt", None, false, 10),
                snap_node(&mk, doomed, "doomed.txt", None, false, 10),
            ],
        });
        // Ops: create `created`, trash `doomed`. Highest seq = 3.
        let ops = serde_json::json!({
            "since": 1,
            "ops": [
                { "seq_id": 2, "op_type": "file_create",
                  "payload": { "id": created, "name_encrypted": enc_name(&mk, created, "fresh.txt"),
                               "parent_id": serde_json::Value::Null, "size_bytes": 7,
                               "storage_pool_id": "pool" } },
                { "seq_id": 3, "op_type": "file_trash", "payload": { "id": doomed } },
            ],
        });

        let server = SyncMockServer::start(vec![("200 OK".into(), boot), ("200 OK".into(), ops)]);
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), mk);

        // Tick 1: bootstrap (cursor 0 → snapshot). Tick 2: ops?since=1.
        sync_tick(&bridge, dir.path()).await.unwrap();
        assert_eq!(bridge.db().get_sync_cursor().unwrap(), Some(1));
        sync_tick(&bridge, dir.path()).await.unwrap();

        let requests = server.finish();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].path, "/api/v1/sync/snapshot");
        assert_eq!(requests[1].path, "/api/v1/sync/ops?since=1");

        // Convergence: keep + created present, doomed is in the TRASH VIEW
        // (task 1698: the trash mirrors the server trash — the row survives
        // as `Trashing`, not deleted), cursor at 3.
        assert!(bridge.db().get_file(keep).unwrap().is_some());
        assert!(bridge.db().get_file(created).unwrap().is_some(), "file_create applied");
        assert_eq!(bridge.db().get_file(created).unwrap().unwrap().path, "fresh.txt");
        assert_eq!(
            bridge.db().get_file(doomed).unwrap().unwrap().status,
            FileStatus::Trashing,
            "file_trash put the row in the trash view (Trashing), it was NOT deleted"
        );
        assert_eq!(
            bridge.db().get_sync_cursor().unwrap(),
            Some(3),
            "cursor advanced to max seq_id"
        );
    }

    /// Flow-7 P0 regression: a desktop/web version upload emits `file_rename`
    /// (name re-encrypt) + `file_update` together, and both land in ONE
    /// `/sync/ops` response. `synthesize_op_row` used to stamp both rows with
    /// the same wall-clock second, so on a hydrated (`Local`) row the rename
    /// bumped `remote_updated_at` to `now` and the `file_update` with the very
    /// same `now` hit the `remote_updated <= remote_updated_at` short-circuit —
    /// then the cursor advanced past it. The device kept v1 forever while
    /// claiming to be in sync.
    async fn assert_rename_then_update_in_one_tick_applies_update(new_name: &str) {
        let dir = tempfile::tempdir().unwrap();
        let mk = [21u8; 32];
        let id = "dddd0000-0000-4000-8000-000000000001";
        let server_ops = serde_json::json!({
            "since": 5,
            "ops": [
                { "seq_id": 6, "op_type": "file_rename",
                  "payload": { "id": id, "new_name_encrypted": enc_name(&mk, id, new_name) } },
                { "seq_id": 7, "op_type": "file_update",
                  "payload": { "id": id, "name_encrypted": enc_name(&mk, id, new_name),
                               "parent_id": serde_json::Value::Null, "size_bytes": 32,
                               "storage_pool_id": "pool",
                               "current_object_version_id": "ov-2",
                               "version_number": 2 } },
            ],
        });
        let server = SyncMockServer::start(vec![("200 OK".into(), server_ops)]);
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), mk);

        // A bootstrapped device that has report.txt v1 (8 B) hydrated locally.
        bridge.db().set_sync_cursor(5).unwrap();
        bridge
            .db()
            .upsert_file(&FileEntry {
                file_id: id.into(),
                path: "report.txt".into(),
                status: FileStatus::Local,
                size_bytes: 8,
                modified_at: 100,
                content_hash: None,
                remote_updated_at: 100,
                parent_id: None,
                item_kind: ItemKind::File,
            })
            .unwrap();
        let mut contract = bridge.db().get_file_contract_state(id).unwrap().unwrap();
        contract.current_version = 1;
        contract.local_base_version = 1;
        contract.current_object_version_id = Some("ov-1".into());
        bridge.db().set_file_contract_state(&contract).unwrap();

        let conflicts = sync_tick(&bridge, dir.path()).await.unwrap();
        assert!(conflicts.is_empty());
        let requests = server.finish();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].path, "/api/v1/sync/ops?since=5");

        let entry = bridge.db().get_file(id).unwrap().unwrap();
        let contract = bridge.db().get_file_contract_state(id).unwrap().unwrap();
        assert_eq!(entry.path, new_name, "rename applied");
        assert_eq!(entry.size_bytes, 32, "file_update applied: size is v2's, not v1's 8 B");
        assert_eq!(contract.current_version, 2, "file_update applied: current_version is 2");
        assert_eq!(contract.current_object_version_id.as_deref(), Some("ov-2"));
        // The cached bytes are still v1: the row must read as stale (remote
        // version ahead of the local base) so the next open re-hydrates.
        assert!(
            contract.current_version > contract.local_base_version,
            "local copy marked stale: current_version {} must exceed local_base_version {}",
            contract.current_version,
            contract.local_base_version
        );
        assert_eq!(bridge.db().get_sync_cursor().unwrap(), Some(7));
    }

    #[tokio::test]
    async fn test_sync_tick_rename_then_update_same_tick_applies_update() {
        assert_rename_then_update_in_one_tick_applies_update("report-final.txt").await;
    }

    #[tokio::test]
    async fn test_sync_tick_name_reencrypt_then_update_same_tick_applies_update() {
        // The FLOW7_NO_RENAME shape: a plain content edit still emits a
        // same-name `file_rename` (name re-encrypt) before the `file_update`.
        assert_rename_then_update_in_one_tick_applies_update("report.txt").await;
    }

    #[tokio::test]
    async fn test_sync_tick_ops_429_backs_off_and_preserves_cursor() {
        // A 429 on /sync/ops must NOT advance the cursor and must NOT lose data —
        // send_with_retry rides it out; if it ultimately surfaces, the tick
        // simply retries next time. Here the mock 429s every attempt; sync_tick
        // must return Ok with the cursor unchanged.
        let dir = tempfile::tempdir().unwrap();
        let mk = [3u8; 32];
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), "http://placeholder".into(), mk);
        // Pre-set a cursor so we take the ops path (not bootstrap).
        bridge.db().set_sync_cursor(8).unwrap();

        // send_with_retry does up to 1 + MAX_429_RETRIES (=3) attempts → 4 total.
        let err_body = serde_json::json!({ "error": "rate limit exceeded", "retry_after": 0 });
        let responses = vec![
            ("429 Too Many Requests".into(), err_body.clone()),
            ("429 Too Many Requests".into(), err_body.clone()),
            ("429 Too Many Requests".into(), err_body.clone()),
            ("429 Too Many Requests".into(), err_body),
        ];
        let server = SyncMockServer::start(responses);
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), mk);

        // Must not panic / error out the runner; cursor stays put for a retry.
        let conflicts = sync_tick(&bridge, dir.path()).await.unwrap();
        assert!(conflicts.is_empty());

        let requests = server.finish();
        assert_eq!(requests.len(), 4, "one initial + 3 retries (429 backoff)");
        for r in &requests {
            assert_eq!(r.path, "/api/v1/sync/ops?since=8");
        }
        assert_eq!(
            bridge.db().get_sync_cursor().unwrap(),
            Some(8),
            "cursor preserved across 429"
        );
    }

    #[tokio::test]
    async fn test_sync_tick_seq_id_zero_bootstrap_then_takes_ops_path_not_resnapshot() {
        // BLOCKER regression (issue 1): the server returns seq_id 0 for any vault
        // whose `sync_ops` log is empty (MAX(seq_id) → NULL → unwrap_or(0)). seq_id
        // 0 is a VALID bootstrapped cursor, NOT an "unset" sentinel. The 2nd tick
        // MUST issue /sync/ops?since=0 and MUST NOT re-snapshot.
        let dir = tempfile::tempdir().unwrap();
        let mk = [5u8; 32];
        let only = "dddd0000-0000-4000-8000-000000000001";

        // Bootstrap snapshot with seq_id 0 (empty op log) and one node.
        let boot = serde_json::json!({
            "seq_id": 0,
            "nodes": [ snap_node(&mk, only, "only.txt", None, false, 10) ],
        });
        // Empty ops list at since=0 (nothing happened yet). Because the ops list
        // is empty, the freshness-probe snapshot also fires; it returns seq_id 0,
        // which is NOT < cursor(0), so it does NOT re-bootstrap.
        let ops_empty = serde_json::json!({ "since": 0, "ops": [] });
        let probe = serde_json::json!({ "seq_id": 0, "nodes": [
            snap_node(&mk, only, "only.txt", None, false, 10)
        ] });

        let server = SyncMockServer::start(vec![
            ("200 OK".into(), boot),
            ("200 OK".into(), ops_empty),
            ("200 OK".into(), probe),
        ]);
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), mk);

        // Tick 1: cursor UNSET → bootstrap snapshot, stores cursor Some(0).
        sync_tick(&bridge, dir.path()).await.unwrap();
        assert_eq!(
            bridge.db().get_sync_cursor().unwrap(),
            Some(0),
            "bootstrap with seq_id 0 stores a real Some(0) cursor"
        );

        // Tick 2: cursor Some(0) → MUST take the ops path (since=0), NOT re-bootstrap.
        sync_tick(&bridge, dir.path()).await.unwrap();

        let requests = server.finish();
        assert_eq!(requests.len(), 3);
        assert_eq!(requests[0].path, "/api/v1/sync/snapshot", "tick1 = bootstrap");
        assert_eq!(
            requests[1].path, "/api/v1/sync/ops?since=0",
            "tick2 takes the ops delta path (NOT a re-bootstrap snapshot)"
        );
        // The 3rd request is the empty-delta freshness probe, not a re-bootstrap.
        assert_eq!(requests[2].path, "/api/v1/sync/snapshot");
        // The row from bootstrap survives (no spurious re-prune).
        assert!(bridge.db().get_file(only).unwrap().is_some());
    }

    #[tokio::test]
    async fn test_sync_tick_nested_create_and_move_match_fresh_snapshot_path() {
        // HIGH regression (issue 2): a nested file_create and a move-into-folder
        // applied via ops must land at the SAME mirror path a fresh snapshot would
        // produce — under the parent's prefix, not at the sync root.
        let dir = tempfile::tempdir().unwrap();
        let mk = [6u8; 32];
        let folder = "f1110000-0000-4000-8000-000000000001";
        let nested = "f1110000-0000-4000-8000-000000000002";
        let mover = "f1110000-0000-4000-8000-000000000003";

        // Bootstrap: just the folder (parents-known-first), seq 1.
        let boot = serde_json::json!({
            "seq_id": 1,
            "nodes": [ snap_node(&mk, folder, "docs", None, true, 10) ],
        });
        // Ops: create a NESTED file under `folder`; create a root file then move
        // it INTO `folder`. Highest seq = 4.
        let ops = serde_json::json!({
            "since": 1,
            "ops": [
                { "seq_id": 2, "op_type": "file_create",
                  "payload": { "id": nested, "name_encrypted": enc_name(&mk, nested, "notes.txt"),
                               "parent_id": folder, "size_bytes": 3, "storage_pool_id": "pool" } },
                { "seq_id": 3, "op_type": "file_create",
                  "payload": { "id": mover, "name_encrypted": enc_name(&mk, mover, "moved.txt"),
                               "parent_id": serde_json::Value::Null, "size_bytes": 3,
                               "storage_pool_id": "pool" } },
                { "seq_id": 4, "op_type": "file_move",
                  "payload": { "id": mover, "old_parent_id": serde_json::Value::Null,
                               "new_parent_id": folder } },
            ],
        });

        let server = SyncMockServer::start(vec![("200 OK".into(), boot), ("200 OK".into(), ops)]);
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), mk);

        sync_tick(&bridge, dir.path()).await.unwrap(); // bootstrap
        sync_tick(&bridge, dir.path()).await.unwrap(); // ops
        server.finish();

        // Both items nested under the folder prefix, exactly as a fresh snapshot
        // (which resolves nesting via parent_id) would place them.
        assert_eq!(
            bridge.db().get_file(nested).unwrap().unwrap().path,
            "docs/notes.txt",
            "nested file_create lands under the parent prefix, not at root"
        );
        assert_eq!(
            bridge.db().get_file(mover).unwrap().unwrap().path,
            "docs/moved.txt",
            "move-into-folder lands under the new parent prefix, not at root"
        );
    }

    #[tokio::test]
    async fn test_sync_tick_trash_then_restore_keeps_row_via_resnapshot() {
        // HIGH regression (issue 3/7): a trash op deletes the local row; the
        // following restore op ({id}-only payload) cannot rebuild it, so it must
        // schedule a re-snapshot that re-materialises the row on the next tick.
        let dir = tempfile::tempdir().unwrap();
        let mk = [8u8; 32];
        let file = "e1110000-0000-4000-8000-000000000001";

        // Bootstrap: the file exists, seq 1.
        let boot = serde_json::json!({
            "seq_id": 1,
            "nodes": [ snap_node(&mk, file, "doc.txt", None, false, 10) ],
        });
        // Ops: trash then restore. Highest seq = 3.
        let ops = serde_json::json!({
            "since": 1,
            "ops": [
                { "seq_id": 2, "op_type": "file_trash", "payload": { "id": file } },
                { "seq_id": 3, "op_type": "file_restore", "payload": { "id": file } },
            ],
        });
        // The restore scheduled a re-snapshot, so the NEXT tick bootstraps from
        // this fresh snapshot (which includes the restored, no-longer-trashed row).
        let resnap = serde_json::json!({
            "seq_id": 3,
            "nodes": [ snap_node(&mk, file, "doc.txt", None, false, 10) ],
        });

        let server = SyncMockServer::start(vec![
            ("200 OK".into(), boot),
            ("200 OK".into(), ops),
            ("200 OK".into(), resnap),
        ]);
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), mk);

        sync_tick(&bridge, dir.path()).await.unwrap(); // bootstrap
        sync_tick(&bridge, dir.path()).await.unwrap(); // ops: trash + restore
        // Task 1698: the trash put the row in the TRASH VIEW (`Trashing`) and
        // the restore flipped it straight back out (`CloudOnly`) — the row
        // never disappeared; a re-snapshot is still pending to refresh its
        // authoritative metadata.
        assert!(
            bridge.db().get_file(file).unwrap().is_some(),
            "the trash-then-restore round trip keeps the row (1698 trash view)"
        );
        assert_eq!(
            bridge.db().get_file(file).unwrap().unwrap().status,
            FileStatus::CloudOnly,
            "restore un-trashes the row"
        );
        sync_tick(&bridge, dir.path()).await.unwrap(); // re-snapshot re-materialises the row

        let requests = server.finish();
        assert_eq!(requests.len(), 3);
        assert_eq!(requests[2].path, "/api/v1/sync/snapshot", "tick3 = forced re-snapshot");
        assert!(
            bridge.db().get_file(file).unwrap().is_some(),
            "re-snapshot re-materialised the restored row — it is NOT permanently invisible"
        );
    }

    #[tokio::test]
    async fn test_sync_tick_empty_snapshot_does_not_prune_existing_tree() {
        // BLOCKER regression (issue 4): a degraded EMPTY snapshot (200 with no
        // nodes) must NOT prune the entire local own-tree.
        let dir = tempfile::tempdir().unwrap();
        let mk = [2u8; 32];
        let kept = "c2220000-0000-4000-8000-000000000001";

        // Pre-seed a settled own-tree row stamped in the past (so the freshness
        // cutoff would otherwise allow pruning it).
        {
            let bridge = test_bridge_with_api(&dir.path().join("state.db"), "http://placeholder".into(), mk);
            bridge
                .db()
                .upsert_file(&FileEntry {
                    file_id: kept.into(),
                    path: "important.txt".into(),
                    status: FileStatus::CloudOnly,
                    size_bytes: 1,
                    modified_at: 0,
                    content_hash: None,
                    remote_updated_at: 0,
                    parent_id: None,
                    item_kind: ItemKind::File,
                })
                .unwrap();
        }

        // Bootstrap with an EMPTY snapshot.
        let boot = serde_json::json!({ "seq_id": 7, "nodes": [] });
        let server = SyncMockServer::start(vec![("200 OK".into(), boot)]);
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), mk);

        sync_tick(&bridge, dir.path()).await.unwrap();
        server.finish();

        assert!(
            bridge.db().get_file(kept).unwrap().is_some(),
            "empty snapshot must NOT prune the existing own-tree (fail-closed)"
        );
    }

    // ── Task 1247: hydrate destination allow-list + real IPC entry point ──────

    /// Item 1: the pure containment predicate `hydrate_file` uses to reject an
    /// out-of-root destination before writing plaintext.
    #[test]
    fn hydrate_dest_is_allowed_covers_the_containment_matrix() {
        let root = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let root_path = root.path();
        let other_path = other.path();

        // An absolute path outside all allowed roots (its parent exists but is
        // NOT under `root`) — the ~/.ssh/authorized_keys style attack — is
        // rejected. `other` exists, so this hits the parent-containment branch.
        let outside = other_path.join("evil.txt");
        assert!(
            !hydrate_dest_is_allowed(&outside, &[root_path]),
            "an existing-parent path outside every root must be rejected"
        );

        // A path with `..` that resolves outside the root is rejected: the
        // parent (`root/..`) is not contained even though the file name is a
        // single component.
        let traversal = root_path.join("../evil.txt");
        assert!(
            !hydrate_dest_is_allowed(&traversal, &[root_path]),
            "a `..` traversal escaping the root must be rejected"
        );

        // The normal hydrate case: a not-yet-existing file directly under the
        // allowed root is accepted (parent contained + single-component name).
        let legit = root_path.join("newfile.bin");
        assert!(
            !legit.exists(),
            "precondition: destination must not exist yet for the common case"
        );
        assert!(
            hydrate_dest_is_allowed(&legit, &[root_path]),
            "a not-yet-existing file under the allowed root must be accepted"
        );

        // Accepted when the destination is under a SECOND allowed root even
        // though it is outside the first (ANY root passing is enough).
        let legit_second = other_path.join("under-second.bin");
        assert!(
            hydrate_dest_is_allowed(&legit_second, &[root_path, other_path]),
            "a destination under any allowed root must be accepted"
        );

        // An empty allow-list fails closed — never vacuously true.
        assert!(
            !hydrate_dest_is_allowed(&legit, &[]),
            "an empty allowed_roots slice must reject everything"
        );

        // A destination whose parent directory does not exist is rejected too
        // (canonicalization of a missing parent fails) — this is exactly the
        // construction the malicious IPC test below relies on.
        let missing_parent = root_path.join("does-not-exist-dir").join("x.txt");
        assert!(
            !hydrate_dest_is_allowed(&missing_parent, &[root_path]),
            "a path under a non-existent subdirectory must be rejected"
        );
    }

    /// Task 1670 round 4 (Codex P2 on PR #75): the pure predicate
    /// `hydrate_file` uses to decide whether a just-written destination is a
    /// macOS Finder handoff staging file (never registered as durable cache)
    /// versus a real cache copy (registered, as always). Uses a throwaway
    /// tempdir as the "hydrate dir" — never the real, singleton App-Group
    /// container a live Beebeeb.app/Finder extension share on this same
    /// machine.
    #[test]
    #[cfg(target_os = "macos")]
    fn is_macos_finder_handoff_staging_path_detects_dest_under_hydrate_dir() {
        let hydrate_dir = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();

        let staged = hydrate_dir.path().join("file-id.abcd1234");
        std::fs::write(&staged, b"decrypted plaintext").unwrap();
        assert!(
            is_macos_finder_handoff_staging_path(&staged, hydrate_dir.path()),
            "a destination inside the hydrate dir must be recognized as Finder handoff staging"
        );

        let cached = elsewhere.path().join("file-id");
        std::fs::write(&cached, b"a real cached copy").unwrap();
        assert!(
            !is_macos_finder_handoff_staging_path(&cached, hydrate_dir.path()),
            "a destination outside the hydrate dir must NOT be treated as Finder handoff staging"
        );
    }

    /// Task 1670 round 4: exercises the REAL [`record_hydration_cache_state`]
    /// — the exact function [`EngineBridge::hydrate_file`] calls — end to
    /// end against a real `StateDb`: a macOS-hydrate-dir-shaped destination
    /// gets NO `cache_path`/`cache_bytes` after "hydration" (a plain on-disk
    /// write here, no network involved — the decrypt/download path is
    /// already covered by the pre-existing hydration tests, this one is
    /// purely about the cache-registration DECISION), so the storage summary
    /// (`cache_bytes_by_effective_pin`, `state_db.rs`) does not count it.
    /// `macos_hydrate_dir` is a throwaway tempdir passed in directly (the
    /// same injection seam production uses to pass the REAL
    /// `macos_hydrate_cache_dir()` — see that function's call site in
    /// `hydrate_file`) — this test never touches the real, singleton
    /// App-Group container.
    #[test]
    #[cfg(target_os = "macos")]
    fn record_hydration_cache_state_skips_mark_cached_under_the_hydrate_dir() {
        let dir = tempfile::tempdir().unwrap();
        let bridge = test_bridge(&dir.path().join("state.db"));
        seed_bridge_row(
            &bridge,
            TEST_FILE_ID,
            "/finder-peek.bin",
            None,
            FileStatus::CloudOnly,
            0,
        );

        let hydrate_dir = tempfile::tempdir().unwrap();
        let finder_dest = hydrate_dir.path().join("finder-peek.abcd1234");
        std::fs::write(&finder_dest, b"decrypted plaintext for a Finder peek").unwrap();

        record_hydration_cache_state(&bridge.db, TEST_FILE_ID, &finder_dest, hydrate_dir.path()).unwrap();

        let row = bridge
            .db
            .get_file_contract_state(TEST_FILE_ID)
            .unwrap()
            .expect("row must still exist");
        assert_eq!(
            row.cache_path, None,
            "a macOS Finder handoff staging destination must NOT be registered as cache_path"
        );
        assert_eq!(
            bridge.db.cache_bytes_by_effective_pin(false).unwrap(),
            0,
            "the storage summary must not count a Finder handoff staging file as cache usage"
        );
    }

    /// Task 1670 round 4: the same function, but `dest_path` is OUTSIDE the
    /// hydrate dir — `mark_cached` must still run exactly as before this
    /// round, and the file must still be counted in the storage summary.
    /// Regression guard for the ordinary (non-macOS-Finder) hydration path,
    /// which this round must not have touched.
    #[test]
    #[cfg(target_os = "macos")]
    fn record_hydration_cache_state_marks_cached_outside_the_hydrate_dir() {
        let dir = tempfile::tempdir().unwrap();
        let bridge = test_bridge(&dir.path().join("state.db"));
        seed_bridge_row(&bridge, TEST_FILE_ID, "/ordinary.bin", None, FileStatus::CloudOnly, 0);

        let hydrate_dir = tempfile::tempdir().unwrap();
        let ordinary_root = tempfile::tempdir().unwrap();
        let ordinary_dest = ordinary_root.path().join("ordinary.bin");
        let plaintext = b"an ordinary cached copy, not a Finder handoff";
        std::fs::write(&ordinary_dest, plaintext).unwrap();

        record_hydration_cache_state(&bridge.db, TEST_FILE_ID, &ordinary_dest, hydrate_dir.path()).unwrap();

        let row = bridge
            .db
            .get_file_contract_state(TEST_FILE_ID)
            .unwrap()
            .expect("row must still exist");
        assert_eq!(
            row.cache_path.as_deref(),
            Some(ordinary_dest.to_string_lossy().as_ref()),
            "an ordinary (non-hydrate-dir) destination must still be registered as cache_path"
        );
        assert_eq!(
            bridge.db.cache_bytes_by_effective_pin(false).unwrap(),
            plaintext.len() as i64,
            "an ordinary cached file must still be counted in the storage summary"
        );
    }

    /// Self-terminating HTTP mock for the IPC end-to-end hydration tests. Unlike
    /// `HydrationMockServer` (blocking accept, fixed request count), this one is
    /// non-blocking with a stop flag + deadline, so the malicious test — where
    /// the destination guard fires BEFORE any HTTP call and the server therefore
    /// sees zero requests — can never hang. It also reports how many requests it
    /// actually served, which is what proves `do_hydrate` never ran on rejection.
    #[cfg(unix)]
    struct IpcHydrationMock {
        base_url: String,
        requests: Arc<std::sync::atomic::AtomicUsize>,
        stop: Arc<std::sync::atomic::AtomicBool>,
        handle: thread::JoinHandle<()>,
    }

    #[cfg(unix)]
    impl IpcHydrationMock {
        fn start(file_key: beebeeb_core::kdf::FileKey, chunks: Vec<Vec<u8>>) -> Self {
            Self::start_with_chunk_delay(file_key, chunks, Duration::ZERO)
        }

        /// Like [`Self::start`], but every chunk response is held back for
        /// `chunk_delay` (task 1670 issue 3: lets a test hang the IPC client
        /// up while the daemon is provably mid-download). With a non-zero delay
        /// a failed write to the (hung-up) daemon is tolerated instead of
        /// panicking the mock thread.
        fn start_with_chunk_delay(
            file_key: beebeeb_core::kdf::FileKey,
            chunks: Vec<Vec<u8>>,
            chunk_delay: Duration,
        ) -> Self {
            use std::sync::atomic::{AtomicBool, AtomicUsize};
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let base_url = format!("http://{}", listener.local_addr().unwrap());
            let size_bytes: usize = chunks.iter().map(|c| c.len()).sum();
            let chunk_count = chunks.len();
            let chunk_size_bytes = chunks.first().map(|c| c.len()).unwrap_or(0);
            let requests = Arc::new(AtomicUsize::new(0));
            let stop = Arc::new(AtomicBool::new(false));
            let (rq, st) = (requests.clone(), stop.clone());
            let handle = thread::spawn(move || {
                let started = std::time::Instant::now();
                loop {
                    if st.load(Ordering::Relaxed) || started.elapsed() > Duration::from_secs(15) {
                        break;
                    }
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            stream.set_nonblocking(false).unwrap();
                            let request = if chunk_delay.is_zero() {
                                read_http_request(&mut stream)
                            } else {
                                // Delayed mode is only used by the cancellation test.
                                match read_http_request_or_hangup(&mut stream) {
                                    Some(request) => request,
                                    None => continue,
                                }
                            };
                            rq.fetch_add(1, Ordering::Relaxed);
                            let response = hydration_mock_response(
                                &request,
                                &file_key,
                                &chunks,
                                size_bytes,
                                chunk_count,
                                chunk_size_bytes,
                            );
                            if !chunk_delay.is_zero() && request.path.contains("/chunks/") {
                                std::thread::sleep(chunk_delay);
                            }
                            let written: std::io::Result<()> = match response {
                                MockResponse::Text(body) => stream.write_all(body.as_bytes()),
                                MockResponse::Binary(body) => {
                                    let header = format!(
                                        "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                                        body.len()
                                    );
                                    stream
                                        .write_all(header.as_bytes())
                                        .and_then(|()| stream.write_all(&body))
                                }
                            };
                            if chunk_delay.is_zero() {
                                written.unwrap();
                            }
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Err(e) => panic!("ipc hydration mock accept failed: {e}"),
                    }
                }
            });
            Self {
                base_url,
                requests,
                stop,
                handle,
            }
        }

        fn stop_and_count(self) -> usize {
            self.stop.store(true, Ordering::Relaxed);
            self.handle.join().unwrap();
            self.requests.load(Ordering::Relaxed)
        }
    }

    /// Drive one real `HydrateFile` request over a real Unix socket against a
    /// real `serve_ipc_at` server, returning the daemon's response. This is the
    /// actual IPC entry point an attacker would use — not an internal function.
    #[cfg(unix)]
    fn ipc_hydrate_roundtrip(
        db: Arc<StateDb>,
        bridge: Arc<EngineBridge>,
        file_id: &str,
        dest_path: &Path,
    ) -> crate::ipc_socket::IpcResponse {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let sock_dir = tempfile::tempdir().unwrap();
        let sock_path = sock_dir.path().join("ipc.sock");
        let file_id = file_id.to_string();
        let dest_string = dest_path.to_string_lossy().to_string();
        let sp = sock_path.clone();

        tokio::runtime::Runtime::new().unwrap().block_on(async move {
            let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
            let server = tokio::spawn(crate::ipc_socket::serve_ipc_at(sp.clone(), db, bridge, cancel_rx));

            // Wait for the listener to bind, then connect a real client.
            let mut client = {
                let mut attempt = 0;
                loop {
                    match tokio::net::UnixStream::connect(&sp).await {
                        Ok(s) => break s,
                        Err(_) if attempt < 300 => {
                            attempt += 1;
                            tokio::time::sleep(Duration::from_millis(10)).await;
                        }
                        Err(e) => panic!("could not connect to test IPC socket: {e}"),
                    }
                }
            };

            // Exact wire shape the server deserializes off the socket.
            let mut req = serde_json::to_vec(&crate::ipc_socket::IpcRequest::HydrateFile {
                file_id: file_id.clone(),
                dest_path: dest_string.clone(),
                progress: false,
            })
            .unwrap();
            req.push(b'\n');
            client.write_all(&req).await.unwrap();

            // One delimited reply line (task 1670 issue 3 framing).
            let mut reply = Vec::new();
            loop {
                let mut byte = [0u8; 1];
                client.read_exact(&mut byte).await.unwrap();
                if byte[0] == b'\n' {
                    break;
                }
                reply.push(byte[0]);
            }
            let resp: crate::ipc_socket::IpcResponse =
                serde_json::from_slice(&reply).expect("daemon must return a valid IpcResponse");

            drop(client);
            let _ = cancel_tx.send(());
            let _ = server.await;
            resp
        })
    }

    /// Item 3 (load-bearing): a malicious `HydrateFile` whose `dest_path` is
    /// outside every allowed root, sent over the REAL IPC socket, is rejected —
    /// the daemon returns `Error`, the target file is NOT created, and the
    /// hydration backend is never even contacted (0 requests), proving the
    /// destination guard short-circuits before any decrypt/download happens.
    #[cfg(unix)]
    #[test]
    fn ipc_rejects_hydrate_write_outside_allowed_roots() {
        let dir = tempfile::tempdir().unwrap();
        let master_key = [9u8; 32];
        let file_key = hydration_test_key(master_key, TEST_FILE_ID);

        // A working backend, so the ONLY thing that can stop the out-of-root
        // write is the guard. If the guard were removed, do_hydrate would
        // succeed and the file would land on disk — failing this test.
        let server = IpcHydrationMock::start(file_key, vec![b"attacker-would-read-this".to_vec()]);

        let db = Arc::new(StateDb::open(dir.path().join("state.db")).unwrap());
        let api = Arc::new(ApiClient::new(server.base_url.clone(), "token".into(), master_key));
        let bridge = Arc::new(EngineBridge::new(db.clone(), api));
        seed_bridge_row(&bridge, TEST_FILE_ID, "/secret.bin", None, FileStatus::CloudOnly, 24);

        // Destination under a non-existent subdirectory of a real temp dir: it
        // is outside the sync root AND its parent cannot be canonicalized, so it
        // is outside every allowed root the IPC handler passes (sync_root? +
        // temp_dir) regardless of this machine's config.
        let malicious_dest = dir.path().join("attacker-controlled-dir").join("authorized_keys");

        let resp = ipc_hydrate_roundtrip(db, bridge, TEST_FILE_ID, &malicious_dest);

        match resp {
            crate::ipc_socket::IpcResponse::Error { message } => {
                assert!(
                    message.contains("allowed root"),
                    "rejection must come from the destination guard, got: {message}"
                );
            }
            other => panic!("expected the malicious write to be rejected, got {other:?}"),
        }

        assert!(
            !malicious_dest.exists(),
            "the malicious destination file must NOT have been created"
        );
        assert!(
            !malicious_dest.parent().unwrap().exists(),
            "the guard must run before create_dir_all — the parent dir must not exist either"
        );

        let served = server.stop_and_count();
        assert_eq!(
            served, 0,
            "the hydration backend must never be contacted when the destination is rejected"
        );
    }

    /// Item 4: the fix does NOT break the legitimate case. The same real IPC
    /// server, given a `dest_path` genuinely under an allowed root (the temp
    /// dir, always an allowed root in the handler), decrypts the real content
    /// and writes it to disk, returning `Ok`.
    #[cfg(unix)]
    #[test]
    fn ipc_allows_hydrate_write_under_allowed_root() {
        let dir = tempfile::tempdir().unwrap();
        let dest_dir = tempfile::tempdir().unwrap(); // under std::env::temp_dir()
        let master_key = [9u8; 32];
        let file_key = hydration_test_key(master_key, TEST_FILE_ID);

        let plaintext = b"beebeeb-1247-real-decrypted-plaintext".to_vec();
        let server = IpcHydrationMock::start(file_key, vec![plaintext.clone()]);

        let db = Arc::new(StateDb::open(dir.path().join("state.db")).unwrap());
        let api = Arc::new(ApiClient::new(server.base_url.clone(), "token".into(), master_key));
        let bridge = Arc::new(EngineBridge::new(db.clone(), api));
        seed_bridge_row(
            &bridge,
            TEST_FILE_ID,
            "/legit.bin",
            None,
            FileStatus::CloudOnly,
            plaintext.len() as i64,
        );

        // dest_dir is created by tempfile under std::env::temp_dir(), which the
        // IPC handler always includes as an allowed root, so this is accepted.
        let legit_dest = dest_dir.path().join("legit-out.bin");

        let resp = ipc_hydrate_roundtrip(db, bridge, TEST_FILE_ID, &legit_dest);

        assert!(
            matches!(resp, crate::ipc_socket::IpcResponse::Ok {}),
            "a legitimate in-root hydrate must succeed, got {resp:?}"
        );
        assert_eq!(
            std::fs::read(&legit_dest).expect("the decrypted file must exist on disk"),
            plaintext,
            "the real decrypted plaintext must land at the requested path"
        );

        let served = server.stop_and_count();
        assert!(
            served >= 2,
            "the legitimate path must contact the backend for metadata + chunk (got {served})"
        );
    }

    /// Start a real `serve_ipc_at` server on a throwaway socket and hand `body`
    /// a connected client (task 1670 issue 3 hydrate-streaming tests).
    #[cfg(unix)]
    fn with_ipc_client<T>(
        db: Arc<StateDb>,
        bridge: Arc<EngineBridge>,
        body: impl FnOnce(tokio::net::UnixStream) -> std::pin::Pin<Box<dyn std::future::Future<Output = T>>>,
    ) -> T {
        let sock_dir = tempfile::tempdir().unwrap();
        let sp = sock_dir.path().join("ipc.sock");
        tokio::runtime::Runtime::new().unwrap().block_on(async move {
            let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
            let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
            let server = tokio::spawn(crate::ipc_socket::serve_ipc_at_with_ready(
                sp.clone(),
                db,
                bridge,
                cancel_rx,
                Some(ready_tx),
                crate::ipc_socket::WriteContentsPolicy::AnyPath,
            ));
            tokio::time::timeout(Duration::from_secs(5), ready_rx)
                .await
                .expect("IPC readiness timed out")
                .expect("IPC startup failed");
            let client = tokio::net::UnixStream::connect(&sp).await.unwrap();
            let out = body(client).await;
            let _ = cancel_tx.send(());
            let _ = server.await;
            out
        })
    }

    /// Read one `\n`-terminated JSON line, byte at a time, with a deadline.
    #[cfg(unix)]
    async fn read_json_line(client: &mut tokio::net::UnixStream) -> serde_json::Value {
        use tokio::io::AsyncReadExt;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        let mut line = Vec::new();
        loop {
            let mut byte = [0u8; 1];
            let n = tokio::time::timeout_at(deadline, client.read(&mut byte))
                .await
                .expect("no delimited line from the daemon within 10s")
                .unwrap();
            assert!(n > 0, "daemon closed the connection mid-reply");
            if byte[0] == b'\n' {
                return serde_json::from_slice(&line).expect("reply line must be valid JSON");
            }
            line.push(byte[0]);
        }
    }

    #[cfg(unix)]
    fn hydrate_request_line(dest: &Path, progress: bool) -> Vec<u8> {
        let mut req = serde_json::to_vec(&serde_json::json!({
            "HydrateFile": {
                "file_id": TEST_FILE_ID,
                "dest_path": dest.to_string_lossy(),
                "progress": progress,
            }
        }))
        .unwrap();
        req.push(b'\n');
        req
    }

    /// Task 1670 issue 3: a hydrate that opts in streams `HydrateProgress`
    /// frames (initial 0/total, then one per decrypted chunk) BEFORE the single
    /// final `{"Ok":{}}`, and the byte counts are the real plaintext sizes.
    #[cfg(unix)]
    #[test]
    fn ipc_hydrate_streams_progress_frames_before_the_final_reply() {
        use tokio::io::AsyncWriteExt;
        let dir = tempfile::tempdir().unwrap();
        let dest_dir = tempfile::tempdir().unwrap();
        let master_key = [9u8; 32];
        let file_key = hydration_test_key(master_key, TEST_FILE_ID);
        let chunks: Vec<Vec<u8>> = vec![vec![b'a'; 10], vec![b'b'; 10], vec![b'c'; 10]];
        let server = IpcHydrationMock::start(file_key, chunks);
        let db = Arc::new(StateDb::open(dir.path().join("state.db")).unwrap());
        let api = Arc::new(ApiClient::new(server.base_url.clone(), "token".into(), master_key));
        let bridge = Arc::new(EngineBridge::new(db.clone(), api));
        seed_bridge_row(
            &bridge,
            TEST_FILE_ID,
            "/three-chunks.bin",
            None,
            FileStatus::CloudOnly,
            30,
        );
        let dest = dest_dir.path().join("three-chunks.bin");
        let request = hydrate_request_line(&dest, true);

        let lines = with_ipc_client(db, bridge, |mut client| {
            Box::pin(async move {
                client.write_all(&request).await.unwrap();
                let mut lines = Vec::new();
                loop {
                    let line = read_json_line(&mut client).await;
                    let is_progress = line.get("HydrateProgress").is_some();
                    lines.push(line);
                    if !is_progress {
                        return lines;
                    }
                }
            })
        });

        let progress: Vec<(u64, u64)> = lines
            .iter()
            .filter_map(|l| l.get("HydrateProgress"))
            .map(|p| (p["done"].as_u64().unwrap(), p["total"].as_u64().unwrap()))
            .collect();
        assert_eq!(
            progress,
            vec![(0, 30), (10, 30), (20, 30), (30, 30)],
            "progress frames must be the initial 0/total then cumulative plaintext bytes per chunk"
        );
        assert_eq!(
            lines.last().unwrap(),
            &serde_json::json!({"Ok": {}}),
            "the final reply must come AFTER every progress frame"
        );
        assert_eq!(lines.len(), 5, "4 progress frames + 1 final reply, nothing else");
        assert_eq!(std::fs::read(&dest).unwrap().len(), 30);
        server.stop_and_count();
    }

    /// Task 1670 issue 3: a client that does not ask for progress (the 0.8.6
    /// extension) must get exactly ONE reply line and no progress frames — it
    /// reads a single reply and would take the first progress frame for it.
    #[cfg(unix)]
    #[test]
    fn ipc_hydrate_without_opt_in_sends_no_progress_frames() {
        use tokio::io::AsyncWriteExt;
        let dir = tempfile::tempdir().unwrap();
        let dest_dir = tempfile::tempdir().unwrap();
        let master_key = [9u8; 32];
        let file_key = hydration_test_key(master_key, TEST_FILE_ID);
        let chunks: Vec<Vec<u8>> = vec![vec![b'a'; 10], vec![b'b'; 10]];
        let server = IpcHydrationMock::start(file_key, chunks);
        let db = Arc::new(StateDb::open(dir.path().join("state.db")).unwrap());
        let api = Arc::new(ApiClient::new(server.base_url.clone(), "token".into(), master_key));
        let bridge = Arc::new(EngineBridge::new(db.clone(), api));
        seed_bridge_row(
            &bridge,
            TEST_FILE_ID,
            "/two-chunks.bin",
            None,
            FileStatus::CloudOnly,
            20,
        );
        let request = hydrate_request_line(&dest_dir.path().join("two-chunks.bin"), false);

        let (first, quiet) = with_ipc_client(db, bridge, |mut client| {
            Box::pin(async move {
                use tokio::io::AsyncReadExt;
                client.write_all(&request).await.unwrap();
                let first = read_json_line(&mut client).await;
                let mut extra = [0u8; 16];
                let quiet = tokio::time::timeout(Duration::from_millis(300), client.read(&mut extra))
                    .await
                    .is_err();
                (first, quiet)
            })
        });
        assert_eq!(
            first,
            serde_json::json!({"Ok": {}}),
            "the first and only line is the final reply"
        );
        assert!(quiet, "nothing may follow the single reply");
        server.stop_and_count();
    }

    /// Task 1670 issue 3: when the client hangs up mid-download (Finder cancelled
    /// the Progress), the daemon stops downloading, never writes the plaintext,
    /// and puts the row's status back instead of leaving it on `Downloading`.
    #[cfg(unix)]
    #[test]
    fn ipc_hydrate_is_cancelled_and_status_restored_when_the_client_hangs_up() {
        use tokio::io::AsyncWriteExt;
        let dir = tempfile::tempdir().unwrap();
        let dest_dir = tempfile::tempdir().unwrap();
        let master_key = [9u8; 32];
        let file_key = hydration_test_key(master_key, TEST_FILE_ID);
        let chunks: Vec<Vec<u8>> = vec![vec![b'a'; 10], vec![b'b'; 10], vec![b'c'; 10]];
        // 1 s per chunk: a completed hydrate needs ~3 s, a cancelled one is
        // stopped inside the first chunk. Generous margins because CI runs the
        // whole suite in parallel on shared runners.
        let server = IpcHydrationMock::start_with_chunk_delay(file_key, chunks, Duration::from_millis(1000));
        let db = Arc::new(StateDb::open(dir.path().join("state.db")).unwrap());
        let api = Arc::new(ApiClient::new(server.base_url.clone(), "token".into(), master_key));
        let bridge = Arc::new(EngineBridge::new(db.clone(), api));
        seed_bridge_row(&bridge, TEST_FILE_ID, "/slow.bin", None, FileStatus::CloudOnly, 30);
        let dest = dest_dir.path().join("slow.bin");
        let request = hydrate_request_line(&dest, true);
        let db_probe = db.clone();

        let (status_when_first_frame_arrived, status_600ms_after_hangup) =
            with_ipc_client(db.clone(), bridge, |mut client| {
                Box::pin(async move {
                    client.write_all(&request).await.unwrap();
                    let first = read_json_line(&mut client).await;
                    assert_eq!(
                        first["HydrateProgress"]["done"], 0,
                        "first frame is the initial 0/total"
                    );
                    let status = db_probe.get_file(TEST_FILE_ID).unwrap().unwrap().status;
                    drop(client); // Finder cancelled the transfer
                    // Well inside the first chunk's 1 s delay: a daemon that
                    // watches the socket has already dropped the download; one that
                    // only notices at its next write has not.
                    tokio::time::sleep(Duration::from_millis(600)).await;
                    let after_hangup = db_probe.get_file(TEST_FILE_ID).unwrap().unwrap().status;
                    // Then leave time for a (wrongly) uncancelled hydrate to
                    // finish (3 x 1 s).
                    tokio::time::sleep(Duration::from_millis(3000)).await;
                    (status, after_hangup)
                })
            });

        assert_eq!(
            status_when_first_frame_arrived,
            FileStatus::Downloading,
            "the test must hang up while the row is genuinely mid-download"
        );
        assert_eq!(
            status_600ms_after_hangup,
            FileStatus::CloudOnly,
            "the daemon must notice the hang-up promptly (within 600 ms), not at its next progress write"
        );
        assert_eq!(
            db.get_file(TEST_FILE_ID).unwrap().unwrap().status,
            FileStatus::CloudOnly,
            "a cancelled hydrate must restore the row's status, not leave Downloading/Local"
        );
        assert!(!dest.exists(), "a cancelled hydrate must not write plaintext to disk");
        let served = server.stop_and_count();
        assert!(
            served <= 2,
            "the daemon must stop fetching after the hang-up: metadata + at most the in-flight chunk, got {served}"
        );
    }

    /// Item 5: `serve_ipc_at` hardens the bound socket file to owner-only 0o600.
    #[cfg(unix)]
    #[test]
    fn ipc_socket_file_is_chmod_0600_after_bind() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let sock_dir = tempfile::tempdir().unwrap();
        let sock_path = sock_dir.path().join("perm.sock");

        let db = Arc::new(StateDb::open(dir.path().join("state.db")).unwrap());
        let api = Arc::new(ApiClient::new(
            "https://api.beebeeb.io".into(),
            "token".into(),
            [7u8; 32],
        ));
        let bridge = Arc::new(EngineBridge::new(db.clone(), api));

        let sp = sock_path.clone();
        tokio::runtime::Runtime::new().unwrap().block_on(async move {
            let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
            let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
            let server = tokio::spawn(crate::ipc_socket::serve_ipc_at_with_ready(
                sp.clone(),
                db,
                bridge,
                cancel_rx,
                Some(ready_tx),
                crate::ipc_socket::WriteContentsPolicy::AnyPath,
            ));

            let ready = tokio::time::timeout(Duration::from_secs(3), ready_rx)
                .await
                .expect("IPC readiness timed out");
            if ready.is_err() {
                panic!("IPC startup failed before readiness: {:?}", server.await);
            }

            let mode = std::fs::metadata(&sp).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "the socket file must be chmod 0o600, got {mode:o}");

            let _ = cancel_tx.send(());
            server.await.unwrap().unwrap();
            assert!(!sp.exists(), "shutdown must remove the published socket");
        });
    }

    /// Task 1247 P0 follow-up (TOCTOU symlink race): the write primitive itself
    /// must refuse a symlink destination (O_NOFOLLOW) and create real files
    /// owner-only (0o600). This unit test pins both directly on the primitive.
    #[cfg(unix)]
    #[test]
    fn write_hydrated_plaintext_refuses_symlink_and_sets_0600() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();

        // Success path: a real, new destination is written 0o600 with the exact
        // bytes.
        let real_dest = root.path().join("real-out.bin");
        write_hydrated_plaintext(&real_dest, &[root.path()], b"plaintext-payload").unwrap();
        assert_eq!(std::fs::read(&real_dest).unwrap(), b"plaintext-payload");
        let mode = std::fs::metadata(&real_dest).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o600,
            "freshly written plaintext must be owner-only, got {mode:o}"
        );

        // Attack path: destination is a symlink pointing OUTSIDE the root at a
        // not-yet-existing target. O_NOFOLLOW must make the open fail closed so
        // the symlink is never followed and its target is never created/written.
        let planted_target = outside.path().join("authorized_keys");
        let symlink_dest = root.path().join("bb_planted_link");
        std::os::unix::fs::symlink(&planted_target, &symlink_dest).unwrap();
        assert!(
            !planted_target.exists(),
            "precondition: symlink target must not exist yet"
        );

        let err = write_hydrated_plaintext(&symlink_dest, &[root.path()], b"decrypted-secret").unwrap_err();
        assert!(
            !planted_target.exists(),
            "O_NOFOLLOW must not follow the symlink — the outside target must NOT be created"
        );
        // O_NOFOLLOW hitting a symlink final (leaf) component fails with ELOOP.
        assert_eq!(
            err.raw_os_error(),
            Some(libc::ELOOP),
            "expected O_NOFOLLOW symlink refusal (ELOOP), got {err:?}"
        );
    }

    /// Task 1247 second-review Gap 1 (parent-directory symlink swap): the leaf
    /// O_NOFOLLOW is not enough — an attacker can replace a legitimately-contained
    /// PARENT directory with a symlink during the download window. The write must
    /// anchor to the parent via O_DIRECTORY|O_NOFOLLOW + openat so a symlinked
    /// parent is refused. Here the parent symlink points to a real dir INSIDE the
    /// allowed root, so the containment re-check PASSES — meaning ONLY the
    /// parent-anchoring O_NOFOLLOW can stop it (isolates that defense).
    ///
    /// Load-bearing: with the old leaf-only open, the write would follow the
    /// symlinked parent and create the file at the real inside-root dir.
    #[cfg(unix)]
    #[test]
    fn write_hydrated_plaintext_refuses_symlinked_parent_dir() {
        let root = tempfile::tempdir().unwrap();

        // A real directory inside the allowed root, and a symlink to it (also
        // inside the root) used as the destination's parent.
        let real_subdir = root.path().join("real_dir");
        std::fs::create_dir(&real_subdir).unwrap();
        let symlink_parent = root.path().join("swapped_parent");
        std::os::unix::fs::symlink(&real_subdir, &symlink_parent).unwrap();

        let dest = symlink_parent.join("out.bin");
        let would_leak_to = real_subdir.join("out.bin");

        // Sanity: containment passes (canonicalize resolves the symlink to an
        // in-root real dir), so the parent-anchoring defense is what must refuse.
        assert!(
            hydrate_dest_is_allowed(&dest, &[root.path()]),
            "precondition: the symlinked-parent dest must pass containment"
        );

        let err = write_hydrated_plaintext(&dest, &[root.path()], b"decrypted-secret").unwrap_err();
        assert!(
            !would_leak_to.exists(),
            "must NOT write through a symlinked parent directory"
        );
        // O_DIRECTORY|O_NOFOLLOW on a symlinked parent fails closed. Linux reports
        // ENOTDIR (the un-followed symlink is not a directory); a bare leaf
        // O_NOFOLLOW would report ELOOP — accept either as "symlinked-parent refused".
        assert!(
            matches!(err.raw_os_error(), Some(libc::ELOOP) | Some(libc::ENOTDIR)),
            "O_DIRECTORY|O_NOFOLLOW must refuse a symlinked parent (ELOOP/ENOTDIR), got {err:?}"
        );
    }

    /// Task 1247 second-review Gap 2 (mode ignored for pre-existing files): the
    /// O_CREAT `mode` only applies to a NEWLY created inode, so a pre-existing
    /// world-readable file at an in-root path would keep its perms and leak the
    /// plaintext. The unconditional `fchmod` must force 0o600 regardless.
    ///
    /// Load-bearing: without the fchmod (relying on the open-time mode) the
    /// pre-existing 0o644 file keeps 0o644 after the write.
    #[cfg(unix)]
    #[test]
    fn write_hydrated_plaintext_forces_0600_on_preexisting_file() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        // Attacker pre-creates a mundane, world-readable real file at an in-root
        // path (no symlink trick — passes containment as a genuine regular file).
        let dest = root.path().join("preexisting.bin");
        std::fs::write(&dest, b"attacker-placeholder").unwrap();
        std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(
            std::fs::metadata(&dest).unwrap().permissions().mode() & 0o777,
            0o644,
            "precondition: the pre-existing file must start world-readable"
        );

        write_hydrated_plaintext(&dest, &[root.path()], b"decrypted-secret").unwrap();

        assert_eq!(
            std::fs::read(&dest).unwrap(),
            b"decrypted-secret",
            "the decrypted content must be written"
        );
        let mode = std::fs::metadata(&dest).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o600,
            "fchmod must force owner-only perms even on a pre-existing file, got {mode:o}"
        );
    }

    /// Task 1670 round 2: the temp-then-rename publish step must clean up its
    /// own temp file when the final `renameat` fails, so a failed hydration
    /// never leaves a stray `.part` file behind in the hydrate-cache (or any
    /// other) directory. Forced deterministically (no quota tricks, no
    /// threads): pre-create the destination as a DIRECTORY, so the write to
    /// the temp file succeeds but `renameat(file -> existing dir)` always
    /// fails (EISDIR/ENOTDIR) — exactly the failure shape this cleanup exists
    /// for.
    #[cfg(unix)]
    #[test]
    fn write_hydrated_plaintext_removes_temp_file_when_final_rename_fails() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("out.bin");
        std::fs::create_dir(&dest).unwrap();

        let result = write_hydrated_plaintext(&dest, &[dir.path()], b"plaintext-payload");
        assert!(result.is_err(), "a rename onto an existing directory must fail");

        let leftover: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name())
            .collect();
        assert_eq!(
            leftover,
            vec![std::ffi::OsString::from("out.bin")],
            "the temp file must be removed after a failed rename, leaving only the \
             pre-existing dest untouched, got {leftover:?}"
        );
    }

    /// Task 1247 4th review (close the untested `..`-rejection logic): a
    /// `dest_path` whose relative portion contains a non-Normal component (`..`)
    /// must be refused before any openat descent. Not fixing a known bug — just
    /// covering a branch that had zero direct tests across four rounds.
    ///
    /// Load-bearing: `root/legit` exists, so if the non-normal-component
    /// rejection were removed, the descent would resolve `legit/..` back to
    /// `root` and create `root/evil`. The check must stop it first.
    // Exercises the Unix openat descent; the non-Unix writer has no descent.
    #[cfg(unix)]
    #[test]
    fn write_hydrated_plaintext_rejects_dotdot_components() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("legit")).unwrap();

        // root/legit/../evil — the parent (root/legit/..) carries a ParentDir
        // component after strip_prefix, which must be rejected.
        let dest = root.path().join("legit").join("..").join("evil");
        let result = write_hydrated_plaintext(&dest, &[root.path()], b"decrypted-secret");

        assert!(
            result.is_err(),
            "a dest_path with a `..` component must be rejected, got {result:?}"
        );
        assert!(
            !root.path().join("evil").exists(),
            "nothing must be written when a `..` component is rejected"
        );
    }

    /// Task 1247 third review (rename-swap, NOT a symlink): the write must anchor
    /// to the directory INODE that was validated during the openat descent, not
    /// re-resolve the path a second time. This is proven DETERMINISTICALLY (no
    /// thread, no race): open the real subdir's fd, then swap what the `subdir`
    /// NAME points at via remove_dir+rename, then create the leaf via the fd held
    /// from BEFORE the swap — and confirm the write landed in the original inode
    /// (reachable via the held fd) and NOT in the directory the name now points
    /// at. Load-bearing: re-resolving the leaf by the current path string instead
    /// of the held fd makes it land in the swapped-in directory (see the mutation
    /// note in the task file — the red/green swaps the leaf step to a path write).
    #[cfg(unix)]
    #[test]
    fn create_leaf_relative_is_anchored_to_original_dir_fd_across_rename_swap() {
        use std::io::{Read, Write};
        use std::os::unix::io::{AsRawFd, FromRawFd};

        let root = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();

        // A real subdir inside the root; capture its fd via the anchored descent
        // helpers — this fd is the "validated parent", held from before any swap.
        std::fs::create_dir(root.path().join("subdir")).unwrap();
        let root_fd = open_dir_no_follow(root.path()).unwrap();
        let subdir_fd = open_dir_relative_no_follow(root_fd.as_raw_fd(), std::ffi::OsStr::new("subdir")).unwrap();

        // DETERMINISTIC swap (no thread, no race): make the `subdir` NAME resolve
        // to a DIFFERENT real directory. We move the original out of the way
        // (rename, not remove — so its inode stays linked and can still accept a
        // new file via the held fd) and move an attacker-controlled directory
        // into the `subdir` name.
        let attacker_dir = elsewhere.path().join("attacker_dir");
        std::fs::create_dir(&attacker_dir).unwrap();
        std::fs::rename(root.path().join("subdir"), root.path().join("orig_moved")).unwrap();
        std::fs::rename(&attacker_dir, root.path().join("subdir")).unwrap();

        // Create + write the leaf via the fd captured BEFORE the swap.
        let leaf = std::ffi::OsStr::new("out.bin");
        let file_fd = create_leaf_relative(subdir_fd.as_raw_fd(), leaf).unwrap();
        {
            let mut f = std::fs::File::from(file_fd);
            f.write_all(b"anchored-plaintext").unwrap();
        }

        // Reachable + correct via the held (original) dir fd.
        let read_raw = unsafe {
            libc::openat(
                subdir_fd.as_raw_fd(),
                std::ffi::CString::new("out.bin").unwrap().as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        assert!(read_raw >= 0, "leaf must be reachable via the original anchored dir fd");
        let mut via_fd = unsafe { std::fs::File::from(std::os::unix::io::OwnedFd::from_raw_fd(read_raw)) };
        let mut got = Vec::new();
        via_fd.read_to_end(&mut got).unwrap();
        assert_eq!(
            got, b"anchored-plaintext",
            "the write landed in the originally-validated inode"
        );

        // The write followed the fd: it is visible under the ORIGINAL inode's new
        // name (orig_moved) and NOT under the current `subdir` name (which now
        // resolves to the attacker's swapped-in directory).
        assert!(
            root.path().join("orig_moved").join("out.bin").exists(),
            "the write must land in the originally-validated inode (now named orig_moved)"
        );
        assert!(
            !root.path().join("subdir").join("out.bin").exists(),
            "the write must NOT land in the directory the `subdir` name currently resolves to"
        );
    }

    /// Task 1247 P0 follow-up: the SAME race proven end-to-end through the real
    /// `hydrate_file`. A broken symlink at the destination passes the top-of-fn
    /// containment guard (its `exists()` is false → not-yet-exists branch: parent
    /// contained + single-component name), then a full download+decrypt runs, and
    /// only the O_NOFOLLOW write stops the decrypted plaintext from being written
    /// through the symlink to a target outside the allowed root. Load-bearing:
    /// with a plain `fs::write` the outside target WOULD be created.
    #[cfg(unix)]
    #[test]
    fn hydrate_file_fails_closed_on_symlink_destination_toctou() {
        let dir = tempfile::tempdir().unwrap(); // the allowed root
        let outside = tempfile::tempdir().unwrap(); // outside every allowed root
        let master_key = [9u8; 32];
        let file_key = hydration_test_key(master_key, TEST_FILE_ID);

        let plaintext = b"decrypted-vault-plaintext-must-not-escape".to_vec();
        let server = IpcHydrationMock::start(file_key, vec![plaintext.clone()]);

        let db = Arc::new(StateDb::open(dir.path().join("state.db")).unwrap());
        let api = Arc::new(ApiClient::new(server.base_url.clone(), "token".into(), master_key));
        let bridge = Arc::new(EngineBridge::new(db.clone(), api));
        seed_bridge_row(
            &bridge,
            TEST_FILE_ID,
            "/legit.bin",
            None,
            FileStatus::CloudOnly,
            plaintext.len() as i64,
        );

        // Attacker plants a symlink at the guard-passing destination pointing to a
        // not-yet-existing file OUTSIDE the allowed root (simulating the swap
        // landing during the do_hydrate window — a broken symlink so the guard's
        // exists() check is false and it passes containment).
        let planted_target = outside.path().join("authorized_keys");
        let dest = dir.path().join("bb_target_link");
        std::os::unix::fs::symlink(&planted_target, &dest).unwrap();
        assert!(
            !planted_target.exists(),
            "precondition: symlink target must not exist yet"
        );
        // Sanity: the destination genuinely passes the containment guard, so this
        // test really is exercising the write-site O_NOFOLLOW defense, not the guard.
        assert!(
            hydrate_dest_is_allowed(&dest, &[dir.path()]),
            "the symlink destination must pass the containment guard (so only O_NOFOLLOW can stop it)"
        );

        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async { bridge.hydrate_file(TEST_FILE_ID, &dest, &[dir.path()]).await });

        assert!(
            result.is_err(),
            "hydrate_file must fail closed on a symlink destination, got {result:?}"
        );
        assert!(
            !planted_target.exists(),
            "O_NOFOLLOW must prevent following the planted symlink — decrypted plaintext must NOT reach the outside target"
        );

        server.stop_and_count();
    }

    // ── Task 1698 part 2: trash semantics (ruling: full sync) ────────────────

    #[test]
    fn test_1698_hydrate_final_status_keeps_the_trashing_marker() {
        // Opening a trashed file from the trash view hydrates it; the marker
        // must survive (a `Local` flip would move the item OUT of the trash
        // container in the replica).
        assert_eq!(
            hydrate_final_status(FileStatus::Trashing),
            FileStatus::Trashing,
            "a Trashing row stays Trashing after a successful hydrate"
        );
        assert_eq!(hydrate_final_status(FileStatus::CloudOnly), FileStatus::Local);
        assert_eq!(hydrate_final_status(FileStatus::Downloading), FileStatus::Local);
    }

    #[test]
    // `file_provider_change_payloads`/`FP_TRASH_APPLE` live in `ipc_socket`,
    // which is `#[cfg(unix)]` (lib.rs) — this test compiles out on Windows.
    #[cfg(unix)]
    fn test_1698_queue_finder_delete_parks_trashing_immediately() {
        // The Finder-side trash: `queue_finder_delete` parks the row
        // `Trashing` AFTER enqueueing the op, so the change feed immediately
        // presents the item under the trash container (ruling step 2), while
        // a failed enqueue leaves the row untouched.
        let dir = tempfile::tempdir().unwrap();
        let bridge = test_bridge(&dir.path().join("state.db"));
        let file = "qfd00000-0000-4000-8000-000000000001";
        seed_bridge_entry(&bridge, file, "doomed.txt", None, FileStatus::Local, false, 10);

        let outcome = bridge.queue_finder_delete(file, None).unwrap();
        assert!(matches!(
            outcome,
            crate::engine_bridge::FinderWriteOutcome::Queued { .. }
        ));
        let row = bridge.db().get_file(file).unwrap().unwrap();
        assert_eq!(row.status, FileStatus::Trashing, "the row parks Trashing right away");
        assert_eq!(
            bridge.db().list_due_operations(now_secs()).unwrap().len(),
            1,
            "the server trash op is queued"
        );
        let changes = bridge.db().list_file_changes_paged(None, 100).unwrap().unwrap().0;
        let payloads = crate::ipc_socket::file_provider_change_payloads(bridge.db(), changes);
        let last = payloads.last().and_then(|change| change.item.as_ref()).unwrap();
        assert_eq!(last.status, "trashing");
        assert_eq!(last.parent_identifier, crate::ipc_socket::FP_TRASH_APPLE);
    }

    #[test]
    fn test_1698_queue_finder_delete_unknown_item_is_an_idempotent_success() {
        // Ruling: "unknown items report success" — the item may have been
        // deleted remotely already. No op is queued, no row is touched.
        let dir = tempfile::tempdir().unwrap();
        let bridge = test_bridge(&dir.path().join("state.db"));
        let outcome = bridge
            .queue_finder_delete("does0000-0000-4000-8000-000000000000", None)
            .unwrap();
        assert!(
            matches!(outcome, crate::engine_bridge::FinderWriteOutcome::Ignored { .. }),
            "an unknown item must be an Ignored (idempotent success), got {outcome:?}"
        );
        assert!(
            bridge.db().list_due_operations(now_secs()).unwrap().is_empty(),
            "no trash op is queued for an unknown item"
        );
    }

    #[test]
    fn test_1698_remote_file_trash_echo_marks_subtree_trashing_not_deleted() {
        // Ruling step 3: a REMOTE trash (another device trashed the file) is
        // the server trash — the mirror must flip the subtree to `Trashing`
        // (trash view), NOT delete the rows.
        let dir = tempfile::tempdir().unwrap();
        let bridge = test_bridge(&dir.path().join("state.db"));
        let folder = "fold0000-0000-4000-8000-000000000001";
        let child = "chil0000-0000-4000-8000-000000000002";
        seed_bridge_entry(&bridge, folder, "docs", None, FileStatus::CloudOnly, true, 10);
        seed_bridge_entry(
            &bridge,
            child,
            "docs/notes.txt",
            Some(folder),
            FileStatus::CloudOnly,
            false,
            10,
        );

        let op = crate::api_client::SyncOp {
            seq_id: 4,
            op_type: "file_trash".into(),
            payload: serde_json::json!({ "id": folder }),
        };
        let mut conflicts = Vec::new();
        let applied = apply_sync_op(&bridge, dir.path(), &op, 200, &mut conflicts).unwrap();
        assert!(
            applied.contains(&folder.to_string()) && applied.contains(&child.to_string()),
            "the flip reports the folder AND its descendants for the working-set signal: {applied:?}"
        );
        for id in [folder, child] {
            let row = bridge
                .db()
                .get_file(id)
                .unwrap()
                .expect("the row SURVIVES (trash view)");
            assert_eq!(row.status, FileStatus::Trashing, "{id} must be Trashing, not deleted");
        }
    }

    #[test]
    fn test_1698_file_delete_echo_removes_trashing_rows_permanently() {
        // `file_delete` = server-side PERMANENT delete (app trash view / web /
        // retention janitor). A Trashing row must leave the trash view: the
        // row is deleted and a `deleted` change is recorded.
        let dir = tempfile::tempdir().unwrap();
        let bridge = test_bridge(&dir.path().join("state.db"));
        let file = "perm0000-0000-4000-8000-000000000001";
        seed_bridge_entry(&bridge, file, "doomed.txt", None, FileStatus::Trashing, false, 10);

        let op = crate::api_client::SyncOp {
            seq_id: 5,
            op_type: "file_delete".into(),
            payload: serde_json::json!({ "id": file }),
        };
        let mut conflicts = Vec::new();
        let applied = apply_sync_op(&bridge, dir.path(), &op, 200, &mut conflicts).unwrap();
        assert!(
            bridge.db().get_file(file).unwrap().is_none(),
            "the permanently-deleted row leaves the mirror"
        );
        assert!(
            applied.contains(&file.to_string()),
            "the removal is reported for the working set"
        );
        let changes = bridge.db().list_file_changes_paged(None, 100).unwrap().unwrap().0;
        assert!(
            changes
                .iter()
                .any(|change| change.file_id == file && change.kind == crate::state_db::FpChangeKind::Deleted),
            "a `deleted` change must reach the feed so the replica drops the item"
        );
    }

    #[test]
    fn test_1698_file_restore_echo_un_trashes_the_row() {
        // Un-trashing (ruling: reparent back out of the trash container → the
        // server's existing restore path). A Trashing row flips back to
        // CloudOnly (placeholder re-mints; content re-downloads on open).
        let dir = tempfile::tempdir().unwrap();
        let bridge = test_bridge(&dir.path().join("state.db"));
        let file = "rest0000-0000-4000-8000-000000000001";
        seed_bridge_entry(&bridge, file, "restored.txt", None, FileStatus::Trashing, false, 10);

        let op = crate::api_client::SyncOp {
            seq_id: 6,
            op_type: "file_restore".into(),
            payload: serde_json::json!({ "id": file }),
        };
        let mut conflicts = Vec::new();
        let applied = apply_sync_op(&bridge, dir.path(), &op, 200, &mut conflicts).unwrap();
        let row = bridge.db().get_file(file).unwrap().unwrap();
        assert_eq!(
            row.status,
            FileStatus::CloudOnly,
            "the restored row leaves the trash view"
        );
        assert!(
            applied.contains(&file.to_string()),
            "the flip is reported for the working set"
        );
    }

    #[test]
    fn test_1698_review_hydrate_keeps_a_trashed_row_trashed_after_a_successful_open() {
        // PR #100 review (Codex P1, engine_bridge.rs ~2500): the pre-hydrate
        // status must be the row's status BEFORE the flip to `Downloading`.
        // The trash-view scenario — the user opens a file from the macOS
        // Trash — re-read the DB AFTER `hydrate_file_with_progress` had
        // already flipped the row, so it always saw `Downloading`,
        // `hydrate_final_status` returned `Local`, and the item reparented
        // itself OUT of the trash view (the `Trashing` marker lost).
        let dir = tempfile::tempdir().unwrap();
        let master_key = [7u8; 32];
        let file_key = hydration_test_key(master_key, TEST_FILE_ID);
        let server = HydrationMockServer::start(file_key, vec![vec![b't'; 8]], 2);
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_bridge_row(&bridge, TEST_FILE_ID, "trash-view.txt", None, FileStatus::Trashing, 8);
        let dest = dir.path().join("trash-view.txt");
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async {
                bridge
                    .hydrate_file_with_progress(TEST_FILE_ID, &dest, &[dir.path()], None)
                    .await
            })
            .unwrap();
        server.finish();
        let row = bridge.db.get_file(TEST_FILE_ID).unwrap().unwrap();
        assert_eq!(
            row.status,
            FileStatus::Trashing,
            "opening a file from the macOS Trash view must NOT un-trash it"
        );
    }

    #[test]
    fn test_1698_review_trash_echo_marks_descendants_of_an_already_parked_folder() {
        // PR #100 review (Codex P1, engine_bridge.rs ~5648): Finder trashing a
        // folder parks ONLY the folder row (`queue_finder_delete`); the
        // server's `file_trash` echo is what marks the descendants. The
        // parked-root early return discarded the echo entirely, so nested
        // rows stayed unmarked — and `prune_absent` protects only children
        // whose IMMEDIATE parent is `Trashing`, so grandchildren could drop
        // out of the trash view. The echo must still mark the unmarked
        // subtree when the root is already parked.
        let dir = tempfile::tempdir().unwrap();
        let bridge = test_bridge(&dir.path().join("state.db"));
        let folder = "park0000-0000-4000-8000-000000000001";
        let child = "park0000-0000-4000-8000-000000000002";
        let grandchild = "park0000-0000-4000-8000-000000000003";
        seed_bridge_entry(&bridge, folder, "docs", None, FileStatus::Trashing, true, 10);
        seed_bridge_entry(
            &bridge,
            child,
            "docs/notes.txt",
            Some(folder),
            FileStatus::CloudOnly,
            false,
            10,
        );
        seed_bridge_entry(
            &bridge,
            grandchild,
            "docs/notes/attachment.bin",
            Some(child),
            FileStatus::CloudOnly,
            false,
            10,
        );

        let op = crate::api_client::SyncOp {
            seq_id: 7,
            op_type: "file_trash".into(),
            payload: serde_json::json!({ "id": folder }),
        };
        let mut conflicts = Vec::new();
        let applied = apply_sync_op(&bridge, dir.path(), &op, 200, &mut conflicts).unwrap();
        for id in [folder, child, grandchild] {
            let row = bridge.db().get_file(id).unwrap().unwrap();
            assert_eq!(row.status, FileStatus::Trashing, "{id} must be in the trash view");
        }
        assert!(
            applied.contains(&grandchild.to_string()) && applied.contains(&child.to_string()),
            "newly marked rows are reported for the working-set signal: {applied:?}"
        );
    }

    #[test]
    fn test_1698_review_restore_un_trashes_the_entire_held_subtree() {
        // PR #100 review (Codex P1, engine_bridge.rs ~5715): a restored FOLDER
        // must take its whole held subtree out of the trash view, not just the
        // root row. The authoritative re-snapshot preserves `Trashing` rows by
        // design, so it can never repair the descendants — they would linger
        // in the Trash view, and direct children would even become top-level
        // trash entries after the parent leaves.
        let dir = tempfile::tempdir().unwrap();
        let bridge = test_bridge(&dir.path().join("state.db"));
        let folder = "rest0000-0000-4000-8000-000000000001";
        let child = "rest0000-0000-4000-8000-000000000002";
        let grandchild = "rest0000-0000-4000-8000-000000000003";
        seed_bridge_entry(&bridge, folder, "docs", None, FileStatus::Trashing, true, 10);
        seed_bridge_entry(
            &bridge,
            child,
            "docs/notes.txt",
            Some(folder),
            FileStatus::Trashing,
            false,
            10,
        );
        seed_bridge_entry(
            &bridge,
            grandchild,
            "docs/notes/attachment.bin",
            Some(child),
            FileStatus::Trashing,
            false,
            10,
        );

        let op = crate::api_client::SyncOp {
            seq_id: 8,
            op_type: "file_restore".into(),
            payload: serde_json::json!({ "id": folder }),
        };
        let mut conflicts = Vec::new();
        let applied = apply_sync_op(&bridge, dir.path(), &op, 200, &mut conflicts).unwrap();
        for id in [folder, child, grandchild] {
            let row = bridge.db().get_file(id).unwrap().unwrap();
            assert_eq!(
                row.status,
                FileStatus::CloudOnly,
                "{id} must leave the trash view with the restored folder"
            );
        }
        assert!(
            applied.contains(&grandchild.to_string()) && applied.contains(&folder.to_string()),
            "every flipped row is reported for the working-set signal: {applied:?}"
        );
    }

    // ── A server that keeps versions and refuses a stale base ──────────────

    /// One file on [`VersionedServerMock`]: every completed version's chunks,
    /// encrypted exactly as the client sent them.
    #[derive(Default)]
    struct MockServerFile {
        versions: Vec<Vec<Vec<u8>>>,
    }

    #[derive(Default)]
    struct VersionedServerState {
        files: std::collections::BTreeMap<String, MockServerFile>,
        /// session id -> (file id, chunks received so far)
        sessions: HashMap<String, (String, Vec<Vec<u8>>)>,
        next_file: usize,
        next_session: usize,
        /// Every `uploads/init` body, in order, with the status it got.
        inits: Vec<(serde_json::Value, u16)>,
        /// Every `PATCH /files/{id}`: (file id, body).
        patches: Vec<(String, serde_json::Value)>,
        /// Sessions whose first chunk PUT answers 500 once.
        fail_first_chunk_once: HashSet<String>,
        /// Sessions whose chunk PUTs answer only after this delay.
        delay_chunks: HashMap<String, Duration>,
        /// Every request, in order: (method, path).
        requests: Vec<(String, String)>,
        /// `POST /api/v1/uploads/init` answers only after this delay.
        delay_init: Option<Duration>,
        /// file id -> the status line its `uploads/init` is refused with.
        refuse_init: HashMap<String, &'static str>,
        /// file id -> the 409 message its next `uploads/init` gets, once (spec §8.4).
        conflict_init_once: HashMap<String, String>,
        /// Every `POST /files/{id}/versions/{vid}/restore`: (file id, object version id).
        restores: Vec<(String, String)>,
        /// `GET /api/v1/sync/snapshot` answers these in order: (status line, body). Empty:
        /// `503 Service Unavailable`.
        snapshots: VecDeque<(String, serde_json::Value)>,
        /// Sessions whose `complete` answers only after this delay. The server has completed
        /// the upload before the delay starts.
        delay_complete: HashMap<String, Duration>,
        /// Every `DELETE /api/v1/files/{id}` (a trash): the file id.
        trashes: Vec<String>,
        /// Sessions whose first `complete` completes the upload but loses its reply (500).
        complete_reply_lost_once: HashSet<String>,
        /// Sessions completed with a lost reply: session id -> file id. A repeated `complete`
        /// answers as the server's idempotent retry does, with neither the version nor the
        /// object id (`routes/uploads.rs`, `complete_upload`).
        completed_reply_lost: HashMap<String, String>,
        /// file id -> the encrypted thumbnail blob `GET /files/{id}/thumbnail/{variant}` answers.
        thumbnails: HashMap<String, Vec<u8>>,
        /// Every thumbnail `GET`: the file id.
        thumbnail_requests: Vec<String>,
        /// A create mints a UUID-shaped id instead of `server-file-N` (the hydrate and the
        /// thumbnail fetch parse a UUID before they ask the server).
        uuid_ids: bool,
    }

    /// Upload mock that behaves like the server's version check: a replace
    /// (`file_id` set) whose `base_version_number` is not the file's current
    /// version gets 409; a replace naming an id the server never minted
    /// creates a file under that id, as the server does for a client-chosen
    /// id. A create (`file_id` absent) mints `server-file-N` (a UUID-shaped id with
    /// `uuid_ids`). `GET /files/{id}` has no route: it answers 404, so a hydrate from the
    /// server fails.
    struct VersionedServerMock {
        base_url: String,
        state: Arc<Mutex<VersionedServerState>>,
        stop: Arc<AtomicBool>,
        handle: thread::JoinHandle<()>,
    }

    impl VersionedServerMock {
        fn start() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let base_url = format!("http://{}", listener.local_addr().unwrap());
            let state = Arc::new(Mutex::new(VersionedServerState::default()));
            let stop = Arc::new(AtomicBool::new(false));
            let server_state = Arc::clone(&state);
            let server_stop = Arc::clone(&stop);
            let handle = thread::spawn(move || {
                let started = std::time::Instant::now();
                while !server_stop.load(Ordering::SeqCst) && started.elapsed() < Duration::from_secs(30) {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            stream.set_nonblocking(false).unwrap();
                            let request = read_http_request(&mut stream);
                            let (delay, response) = versioned_server_response(&request, &server_state);
                            if let Some(delay) = delay {
                                std::thread::sleep(delay);
                            }
                            let _ = stream.write_all(&response);
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Err(e) => panic!("versioned server mock accept failed: {e}"),
                    }
                }
            });
            Self {
                base_url,
                state,
                stop,
                handle,
            }
        }

        /// Put `file_id` on the server at `versions` versions (empty
        /// content), as if it had been uploaded before the test.
        fn seed_file(&self, file_id: &str, versions: usize) {
            self.state.lock().unwrap().files.insert(
                file_id.to_string(),
                MockServerFile {
                    versions: vec![Vec::new(); versions],
                },
            );
        }

        fn finish(self) -> VersionedServerState {
            self.stop.store(true, Ordering::SeqCst);
            self.handle.join().unwrap();
            Arc::try_unwrap(self.state)
                .unwrap_or_else(|_| panic!("mock state still shared"))
                .into_inner()
                .unwrap()
        }
    }

    impl VersionedServerState {
        /// Each `uploads/init` as `(file_id, base_version_number, status)`.
        fn init_summary(&self) -> Vec<(serde_json::Value, serde_json::Value, u16)> {
            self.inits
                .iter()
                .map(|(body, status)| (body["file_id"].clone(), body["base_version_number"].clone(), *status))
                .collect()
        }

        /// The plaintext of `file_id`'s latest version.
        fn latest_plaintext(&self, file_id: &str, master_key: [u8; 32]) -> Vec<u8> {
            let file = self
                .files
                .get(file_id)
                .unwrap_or_else(|| panic!("no server file {file_id}"));
            let chunks = file
                .versions
                .last()
                .unwrap_or_else(|| panic!("{file_id} has no version"));
            let master_key = beebeeb_core::kdf::MasterKey::from_bytes(master_key);
            let file_key = beebeeb_core::kdf::derive_file_key(&master_key, file_id.as_bytes());
            chunks
                .iter()
                .flat_map(|chunk| beebeeb_core::encrypt::decrypt_chunk_raw(&file_key, chunk).unwrap())
                .collect()
        }
    }

    fn versioned_server_response(
        request: &RecordedRequest,
        state: &Arc<Mutex<VersionedServerState>>,
    ) -> (Option<Duration>, Vec<u8>) {
        let mut s = state.lock().unwrap();
        s.requests.push((request.method.clone(), request.path.clone()));
        // `GET /files/{id}/thumbnail/{variant}`: the blob a test seeded, or 404 (binary body).
        if request.method == "GET"
            && let Some(rest) = request.path.strip_prefix("/api/v1/files/")
            && let Some((file_id, _variant)) = rest.split_once("/thumbnail/")
        {
            let file_id = file_id.to_string();
            s.thumbnail_requests.push(file_id.clone());
            return match s.thumbnails.get(&file_id) {
                Some(blob) => (None, http_bytes("200 OK", blob)),
                None => (
                    None,
                    http_json("404 Not Found", serde_json::json!({ "error": "no thumbnail" })).into_bytes(),
                ),
            };
        }
        let delay = if request.method == "POST" && request.path == "/api/v1/uploads/init" {
            s.delay_init
        } else if request.method == "POST" && request.path.ends_with("/complete") {
            request
                .path
                .strip_prefix("/api/v1/uploads/")
                .and_then(|rest| rest.split('/').next())
                .and_then(|session| s.delay_complete.get(session).copied())
        } else {
            request
                .path
                .strip_prefix("/api/v1/uploads/")
                .and_then(|rest| rest.split('/').next())
                .filter(|_| request.method == "PUT")
                .and_then(|session| s.delay_chunks.get(session).copied())
        };
        (delay, versioned_server_answer(request, &mut s).into_bytes())
    }

    fn versioned_server_answer(request: &RecordedRequest, s: &mut VersionedServerState) -> String {
        let method = request.method.as_str();
        let path = request.path.as_str();
        if method == "POST" && path == "/api/v1/uploads/init" {
            let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
            if let Some(status) = body["file_id"].as_str().and_then(|id| s.refuse_init.get(id)).copied() {
                let code = status.split(' ').next().and_then(|code| code.parse().ok()).unwrap_or(0);
                s.inits.push((body, code));
                return http_json(status, serde_json::json!({ "error": "refused" }));
            }
            if let Some(message) = body["file_id"].as_str().and_then(|id| s.conflict_init_once.remove(id)) {
                s.inits.push((body, 409));
                return http_json("409 Conflict", serde_json::json!({ "error": message }));
            }
            let file_id = match body["file_id"].as_str() {
                Some(id) => {
                    if let Some(file) = s.files.get(id)
                        && let Some(base) = body["base_version_number"].as_i64()
                        && base != file.versions.len() as i64
                    {
                        s.inits.push((body, 409));
                        return http_json(
                            "409 Conflict",
                            serde_json::json!({ "error": "stale base version for replacement upload" }),
                        );
                    }
                    s.files.entry(id.to_string()).or_default();
                    id.to_string()
                }
                None => {
                    s.next_file += 1;
                    let id = if s.uuid_ids {
                        format!("5e7e0000-0000-4000-8000-{:012}", s.next_file)
                    } else {
                        format!("server-file-{}", s.next_file)
                    };
                    s.files.insert(id.clone(), MockServerFile::default());
                    id
                }
            };
            s.next_session += 1;
            let session = format!("session-{}", s.next_session);
            s.sessions.insert(session.clone(), (file_id.clone(), Vec::new()));
            let response = serde_json::json!({
                "file_id": file_id,
                "tenant_id": "tenant-1",
                "object_version_id": format!("object-init-{}", s.next_session),
                "upload_session_id": session,
                "chunk_size_bytes": body["chunk_size_bytes"],
                "chunk_count": body["chunk_count"],
                "storage_format_version": 2,
                "storage_pool_id": "pool-1",
                "region": "local"
            });
            s.inits.push((body, 201));
            return http_json("201 Created", response);
        }
        // A restore appends a copy of version `vid` (the mock's object ids are
        // `object-{id}-v{n}`) and answers as the server's first branch does (VER:613-618).
        if method == "POST"
            && let Some(rest) = path.strip_prefix("/api/v1/files/")
            && rest.ends_with("/restore")
        {
            let parts: Vec<&str> = rest.split('/').collect(); // [id, "versions", vid, "restore"]
            let (file_id, object) = (parts[0].to_string(), parts[2].to_string());
            let index = object
                .rsplit("-v")
                .next()
                .and_then(|n| n.parse::<usize>().ok())
                .unwrap_or(1)
                .saturating_sub(1);
            let file = s.files.entry(file_id.clone()).or_default();
            let chunks = file.versions.get(index).cloned().unwrap_or_default();
            file.versions.push(chunks);
            let version = file.versions.len();
            s.restores.push((file_id.clone(), object));
            return http_json(
                "200 OK",
                serde_json::json!({
                    "message": "version restored",
                    "version_number": version,
                    "current_object_version_id": format!("object-{file_id}-v{version}"),
                }),
            );
        }
        if method == "DELETE"
            && let Some(id) = path.strip_prefix("/api/v1/files/")
        {
            s.trashes.push(id.to_string());
            return http_json("200 OK", serde_json::json!({ "ok": true }));
        }
        if method == "PATCH"
            && let Some(id) = path.strip_prefix("/api/v1/files/")
        {
            let body = serde_json::from_slice(&request.body).unwrap_or(serde_json::Value::Null);
            s.patches.push((id.to_string(), body));
            return http_json("200 OK", serde_json::json!({ "ok": true }));
        }
        if let Some(rest) = path.strip_prefix("/api/v1/uploads/") {
            let mut parts = rest.split('/');
            let session = parts.next().unwrap_or_default().to_string();
            let action = parts.next().unwrap_or_default();
            if method == "PUT" && action == "chunks" {
                let index: usize = parts.next().unwrap_or("0").parse().unwrap();
                if s.fail_first_chunk_once.remove(&session) {
                    return http_json("500 Internal Server Error", serde_json::json!({ "error": "boom" }));
                }
                let Some((_, chunks)) = s.sessions.get_mut(&session) else {
                    return http_json("404 Not Found", serde_json::json!({ "error": "no session" }));
                };
                if chunks.len() <= index {
                    chunks.resize(index + 1, Vec::new());
                }
                chunks[index] = request.body.clone();
                return http_json(
                    "200 OK",
                    serde_json::json!({ "index": index, "size": request.body.len(), "skipped": false }),
                );
            }
            if method == "POST" && action == "complete" {
                if let Some(file_id) = s.completed_reply_lost.get(&session).cloned() {
                    return http_json(
                        "200 OK",
                        serde_json::json!({ "id": file_id, "status": "completed", "already_completed": true }),
                    );
                }
                let Some((file_id, chunks)) = s.sessions.remove(&session) else {
                    return http_json("404 Not Found", serde_json::json!({ "error": "no session" }));
                };
                // nonce (12) + tag (16) per chunk.
                let size: usize = chunks.iter().map(|chunk| chunk.len().saturating_sub(28)).sum();
                let file = s.files.entry(file_id.clone()).or_default();
                file.versions.push(chunks);
                let version = file.versions.len();
                if s.complete_reply_lost_once.remove(&session) {
                    s.completed_reply_lost.insert(session, file_id);
                    return http_json(
                        "500 Internal Server Error",
                        serde_json::json!({ "error": "reply lost" }),
                    );
                }
                return http_json(
                    "200 OK",
                    serde_json::json!({
                        "file_id": file_id,
                        "version_number": version,
                        "current_object_version_id": format!("object-{file_id}-v{version}"),
                        "size_bytes": size,
                        "mime_type": "text/plain"
                    }),
                );
            }
        }
        if method == "GET" && path == "/api/v1/sync/snapshot" {
            return match s.snapshots.pop_front() {
                Some((status, body)) => http_json(&status, body),
                None => http_json(
                    "503 Service Unavailable",
                    serde_json::json!({ "error": "no snapshot queued" }),
                ),
            };
        }
        if method == "GET"
            && let Some(query) = path.strip_prefix("/api/v1/sync/ops")
        {
            let since = query
                .strip_prefix("?since=")
                .and_then(|since| since.parse::<i64>().ok())
                .unwrap_or(0);
            return http_json("200 OK", serde_json::json!({ "ops": [], "since": since }));
        }
        http_json(
            "404 Not Found",
            serde_json::json!({ "error": format!("unexpected {method} {path}") }),
        )
    }

    /// Run the queue until nothing is due, moving the clock past every
    /// backoff so a refused op is retried. Returns the number of passes.
    async fn drain_upload_queue(bridge: &EngineBridge, sync_root: &Path) -> usize {
        let mut now = now_secs();
        for pass in 1..=12 {
            bridge.process_due_operations(sync_root, now).await.unwrap();
            now = now.saturating_add(10_000);
            if bridge.db.list_due_operations(now).unwrap().is_empty() {
                return pass;
            }
        }
        12
    }

    /// Run `action` on its own thread and wait for it, at most 5 s, and fail the test if it
    /// has not finished. Every seam sits outside every transaction and every lock, so a
    /// correct implementation never blocks the action: a hang is a defect, not a timing
    /// artefact, and a silent timeout would let the action race the rest of the test.
    fn run_competing(action: impl FnOnce() + Send + 'static) {
        let (done, wait) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            action();
            let _ = done.send(());
        });
        wait.recv_timeout(Duration::from_secs(5))
            .expect("the competing action did not finish");
    }

    fn finder_file_target(
        file_id: Option<&str>,
        filename: &str,
        contents: &Path,
        base_version_identifier: Option<String>,
    ) -> FinderWriteTarget {
        FinderWriteTarget {
            file_id: file_id.map(str::to_string),
            parent_id: None,
            filename: filename.to_string(),
            rel_path: None,
            kind: FinderWriteItemKind::File,
            contents_path: Some(contents.to_string_lossy().into_owned()),
            content_type: Some("text/plain".into()),
            base_version_identifier,
        }
    }

    /// The content version the extension holds for `file_id`, read the way
    /// the extension gets it: from the IPC item payload (no IPC socket off
    /// unix, so the same formula directly).
    fn held_content_version(bridge: &EngineBridge, file_id: &str) -> String {
        let entry = bridge.db.get_file(file_id).unwrap().unwrap();
        #[cfg(unix)]
        {
            crate::ipc_socket::file_entry_payload_for_db(&bridge.db, &entry, "root")
                .content_version
                .unwrap()
        }
        #[cfg(not(unix))]
        {
            let contract = bridge.db.get_file_contract_state(file_id).unwrap().unwrap();
            item_content_version(contract.current_version, entry.content_hash.as_deref())
        }
    }

    #[tokio::test]
    async fn a_modify_after_a_desktop_upload_sends_the_server_version_as_its_base() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [21u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);

        let created = dir.path().join("created.txt");
        std::fs::write(&created, b"twenty-eight bytes of text.\n").unwrap();
        bridge
            .queue_file_provider_create(finder_file_target(None, "t.txt", &created, None))
            .unwrap();
        drain_upload_queue(&bridge, &sync_root).await;
        let rows = bridge.db.list_files().unwrap();
        assert_eq!(rows.len(), 1, "one row after the create: {rows:?}");
        let server_id = rows[0].file_id.clone();
        assert_eq!(rows[0].status, FileStatus::Local);

        // The `/sync/ops` echo of that upload re-stamps the row with a later
        // wall-clock second, as it does on a device.
        let mut echoed = bridge.db.get_file(&server_id).unwrap().unwrap();
        echoed.remote_updated_at += 30;
        echoed.modified_at = echoed.remote_updated_at;
        bridge.db.upsert_file(&echoed).unwrap();

        let base = held_content_version(&bridge, &server_id);
        let edited = dir.path().join("edited.txt");
        std::fs::write(&edited, b"twenty-eight bytes of text.\nmore-bytes12").unwrap();
        bridge
            .queue_file_provider_modify(finder_file_target(
                Some(&server_id),
                "t.txt",
                &edited,
                Some(base.clone()),
            ))
            .unwrap();
        drain_upload_queue(&bridge, &sync_root).await;

        let state = server.finish();
        assert!(
            state.inits.iter().all(|(_, status)| *status == 201),
            "no upload may be refused (identifier {base:?}): {:?}",
            state.init_summary()
        );
        let replace = state
            .inits
            .iter()
            .find(|(body, _)| body["file_id"] == serde_json::json!(server_id))
            .unwrap_or_else(|| panic!("no replace upload was sent: {:?}", state.init_summary()));
        assert_eq!(
            replace.0["base_version_number"],
            serde_json::json!(1),
            "the base must be the server version, not the identifier {base:?}"
        );
        assert_eq!(
            state.latest_plaintext(&server_id, master_key),
            b"twenty-eight bytes of text.\nmore-bytes12"
        );
        assert_eq!(state.files[&server_id].versions.len(), 2);
        let row = bridge.db.get_file(&server_id).unwrap().unwrap();
        assert_eq!(
            row.status,
            FileStatus::Local,
            "the edit must not leave the file read-only"
        );
    }

    // ── Identifiers the system still holds from before the version fix ────

    /// A row as an earlier build left it after its own upload and the op
    /// echo: server version 2, wall-clock stamps at 1_791_550_370. That
    /// build told the system `1791550370` (and `1791550370:1791550370:18`).
    fn seed_legacy_row(bridge: &EngineBridge, file_id: &str, content_hash: Option<&str>) {
        bridge
            .db
            .upsert_file(&FileEntry {
                file_id: file_id.into(),
                path: "d2-fixture.txt".into(),
                status: FileStatus::Local,
                size_bytes: 18,
                modified_at: 1_791_550_370,
                content_hash: content_hash.map(str::to_string),
                remote_updated_at: 1_791_550_370,
                parent_id: None,
                item_kind: ItemKind::File,
            })
            .unwrap();
        let mut contract = bridge.db.get_file_contract_state(file_id).unwrap().unwrap();
        contract.current_version = 2;
        bridge.db.set_file_contract_state(&contract).unwrap();
    }

    /// Seed `file_id` as [`seed_legacy_row`] does, queue a content modify of
    /// it with `identifier` as its base, and return the queued op's
    /// `base_version`. Re-seeded per call: a queued write updates the row.
    fn queued_modify_base(
        bridge: &EngineBridge,
        dir: &Path,
        file_id: &str,
        content_hash: Option<&str>,
        identifier: &str,
    ) -> Option<i64> {
        seed_legacy_row(bridge, file_id, content_hash);
        let contents = dir.join(format!("edit-{}.txt", uuid::Uuid::new_v4()));
        std::fs::write(&contents, b"edited bytes").unwrap();
        bridge
            .queue_finder_modify(finder_file_target(
                Some(file_id),
                "d2-fixture.txt",
                &contents,
                Some(identifier.to_string()),
            ))
            .unwrap();
        let queued = bridge.db.list_due_operations(i64::MAX).unwrap();
        let op = queued
            .iter()
            .rev()
            .find(|op| op.file_id.as_deref() == Some(file_id))
            .expect("the modify was queued");
        let base = op.base_version;
        bridge.db.remove_operation(&op.op_id).unwrap();
        bridge.db.set_status(file_id, FileStatus::Local).unwrap();
        base
    }

    #[test]
    fn a_legacy_identifier_of_the_current_content_is_based_on_the_server_version() {
        let dir = tempfile::tempdir().unwrap();
        let bridge = test_bridge(&dir.path().join("state.db"));
        assert_eq!(
            queued_modify_base(&bridge, dir.path(), "legacy-1", None, "1791550370"),
            Some(2),
            "the old content version of the row as it is now"
        );
        assert_eq!(
            queued_modify_base(&bridge, dir.path(), "legacy-1", None, "1791550370:1791550370:18"),
            Some(2),
            "the old full version identifier of the row as it is now"
        );

        assert_eq!(
            queued_modify_base(&bridge, dir.path(), "legacy-hash", Some("abc123"), "1791550370:abc123"),
            Some(2),
            "the old content version with the row's content hash"
        );
    }

    #[test]
    fn a_legacy_identifier_of_older_content_stays_a_stale_base() {
        let dir = tempfile::tempdir().unwrap();
        let bridge = test_bridge(&dir.path().join("state.db"));
        for older in [
            // Stamped before the latest re-stamp of the row.
            "1791550250",
            "1791550250:abc123",
            "1791550250:1791550250:18",
            // The right stamp, other content.
            "1791550370:def456",
            "1791550370:1791550370:17",
        ] {
            assert_eq!(
                queued_modify_base(&bridge, dir.path(), "legacy-2", Some("abc123"), older),
                parse_base_version_number(Some(older)),
                "{older} does not describe the row's current content and is parsed as before"
            );
        }
    }

    #[tokio::test]
    async fn a_legacy_identifier_of_older_content_is_refused_by_the_server() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let server = VersionedServerMock::start();
        server.seed_file("legacy-3", 2);
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), [22u8; 32]);
        seed_legacy_row(&bridge, "legacy-3", None);
        let contents = dir.path().join("edit.txt");
        std::fs::write(&contents, b"edited bytes").unwrap();
        bridge
            .queue_file_provider_modify(finder_file_target(
                Some("legacy-3"),
                "d2-fixture.txt",
                &contents,
                Some("1791550250".into()),
            ))
            .unwrap();
        drain_upload_queue(&bridge, &sync_root).await;
        let state = server.finish();
        assert!(!state.inits.is_empty(), "the upload was attempted");
        assert!(
            state.inits.iter().all(|(_, status)| *status == 409),
            "an identifier of older content is a stale base: {:?}",
            state.init_summary()
        );
        assert_eq!(state.files["legacy-3"].versions.len(), 2, "no version was added");
    }

    #[test]
    fn a_current_format_identifier_is_parsed_as_before() {
        let dir = tempfile::tempdir().unwrap();
        let bridge = test_bridge(&dir.path().join("state.db"));
        for (identifier, base) in [
            ("2", Some(2)),
            ("2:abc123", Some(2)),
            ("2:1791550370:18", Some(2)),
            ("1", Some(1)),
            ("1:abc123", Some(1)),
            ("0", None),
        ] {
            assert_eq!(
                queued_modify_base(&bridge, dir.path(), "current-1", Some("abc123"), identifier),
                base,
                "{identifier}"
            );
        }
    }

    // ── This device's own queued uploads of one file form a chain ─────────

    impl VersionedServerState {
        /// Files that have at least one completed version.
        fn files_with_content(&self) -> Vec<String> {
            self.files
                .iter()
                .filter(|(_, file)| !file.versions.is_empty())
                .map(|(id, _)| id.clone())
                .collect()
        }
    }

    /// A row the server holds at version 1 (seeded on the mock too).
    fn seed_uploaded_row(bridge: &EngineBridge, server: &VersionedServerMock, file_id: &str) {
        server.seed_file(file_id, 1);
        bridge
            .db
            .upsert_file(&FileEntry {
                file_id: file_id.into(),
                path: "notes.txt".into(),
                status: FileStatus::Local,
                size_bytes: 10,
                modified_at: 1_700_000_000,
                content_hash: None,
                remote_updated_at: 1_700_000_000,
                parent_id: None,
                item_kind: ItemKind::File,
            })
            .unwrap();
        let mut contract = bridge.db.get_file_contract_state(file_id).unwrap().unwrap();
        contract.current_version = 1;
        bridge.db.set_file_contract_state(&contract).unwrap();
    }

    fn queue_save(bridge: &EngineBridge, dir: &Path, file_id: &str, filename: &str, bytes: &[u8], base: &str) {
        fp_save(bridge, dir, file_id, filename, bytes, base);
    }

    /// A File Provider save of `bytes` to `file_id` on `base`, as the extension sends it.
    fn fp_save(bridge: &EngineBridge, dir: &Path, file_id: &str, filename: &str, bytes: &[u8], base: &str) -> FpWrite {
        let contents = dir.join(format!("save-{}.txt", uuid::Uuid::new_v4()));
        std::fs::write(&contents, bytes).unwrap();
        bridge
            .queue_file_provider_modify(finder_file_target(
                Some(file_id),
                filename,
                &contents,
                Some(base.to_string()),
            ))
            .unwrap()
    }

    /// A File Provider create of a new file holding `bytes`.
    fn fp_create(bridge: &EngineBridge, dir: &Path, filename: &str, bytes: &[u8]) -> FpWrite {
        let contents = dir.join(format!("create-{}.txt", uuid::Uuid::new_v4()));
        std::fs::write(&contents, bytes).unwrap();
        bridge
            .queue_file_provider_create(finder_file_target(None, filename, &contents, None))
            .unwrap()
    }

    #[tokio::test]
    async fn a_second_queued_save_rebases_after_the_first_lands() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [23u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_uploaded_row(&bridge, &server, "two-saves");

        // Both saves are based on version 1: the second came before the
        // system learned of the version the first produces.
        queue_save(&bridge, dir.path(), "two-saves", "notes.txt", b"first save", "1");
        queue_save(
            &bridge,
            dir.path(),
            "two-saves",
            "notes.txt",
            b"first save, second save",
            "1",
        );
        drain_upload_queue(&bridge, &sync_root).await;

        let state = server.finish();
        assert_eq!(
            state.init_summary(),
            vec![
                (serde_json::json!("two-saves"), serde_json::json!(1), 201),
                (serde_json::json!("two-saves"), serde_json::json!(2), 201),
            ],
            "the second upload is based on the version the first produced"
        );
        assert_eq!(state.files["two-saves"].versions.len(), 3);
        assert_eq!(
            state.latest_plaintext("two-saves", master_key),
            b"first save, second save",
            "the newest bytes are the file's latest version"
        );
        assert!(bridge.db.list_due_operations(i64::MAX).unwrap().is_empty());
        assert_eq!(
            bridge.db.get_file("two-saves").unwrap().unwrap().status,
            FileStatus::Local
        );
    }

    #[tokio::test]
    async fn a_later_save_waits_for_an_earlier_save_that_is_backing_off() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [24u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_uploaded_row(&bridge, &server, "backoff");
        // The first save's upload fails once (a dropped chunk) and backs off.
        server
            .state
            .lock()
            .unwrap()
            .fail_first_chunk_once
            .insert("session-1".into());

        queue_save(&bridge, dir.path(), "backoff", "notes.txt", b"older save", "1");
        queue_save(
            &bridge,
            dir.path(),
            "backoff",
            "notes.txt",
            b"older save, newer save",
            "1",
        );
        drain_upload_queue(&bridge, &sync_root).await;

        let state = server.finish();
        assert_eq!(state.files["backoff"].versions.len(), 3, "{:?}", state.init_summary());
        assert_eq!(
            state.latest_plaintext("backoff", master_key),
            b"older save, newer save",
            "the older save must not land after the newer one: {:?}",
            state.init_summary()
        );
    }

    #[cfg(target_os = "macos")] // other platforms keep created_at order
    #[tokio::test]
    async fn m2_order_is_insertion_order_when_the_clock_steps_back() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [41u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_uploaded_row(&bridge, &server, "clock");
        queue_save(&bridge, dir.path(), "clock", "notes.txt", b"older save", "1");
        queue_save(
            &bridge,
            dir.path(),
            "clock",
            "notes.txt",
            b"older save, newer save",
            "1",
        );
        // The wall clock stepped back between the two saves.
        let ops = bridge.db.list_operations_for_file("clock").unwrap();
        assert_eq!(ops.len(), 2);
        bridge.db.set_created_at_for_test(&ops[0].op_id, 2_000_000_000);
        bridge.db.set_created_at_for_test(&ops[1].op_id, 1_000_000_000);

        drain_upload_queue(&bridge, &sync_root).await;

        let state = server.finish();
        assert_eq!(state.files["clock"].versions.len(), 3, "{:?}", state.init_summary());
        assert_eq!(
            state.latest_plaintext("clock", master_key),
            b"older save, newer save",
            "the newest bytes land last: {:?}",
            state.init_summary()
        );
    }

    #[tokio::test]
    async fn the_runners_copy_never_resurrects_or_drops_a_save() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [42u8; 32];
        let server = VersionedServerMock::start();
        server.state.lock().unwrap().delay_init = Some(Duration::from_millis(400));
        // The purged op's first chunk would fail (its init is the first: session-1). With the
        // guard its attempt ends at the resume write and sends no chunk. Without it, the
        // attempt writes a resume row for the deleted op, fails at the chunk and leaves
        // that row behind, because a failed attempt clears nothing.
        server
            .state
            .lock()
            .unwrap()
            .fail_first_chunk_once
            .insert("session-1".into());
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_uploaded_row(&bridge, &server, "revoked");
        queue_save(
            &bridge,
            dir.path(),
            "revoked",
            "notes.txt",
            b"save under a share that goes away",
            "1",
        );
        seed_uploaded_row(&bridge, &server, "kept");
        queue_save(&bridge, dir.path(), "kept", "other.txt", b"the second file's save", "1");
        let w = bridge.db.list_operations_for_file("revoked").unwrap().remove(0);
        // The first file is content of a share; the share is revoked while W's init is in flight.
        let mut contract = bridge.db.get_file_contract_state("revoked").unwrap().unwrap();
        contract.namespace = Namespace::SharedWithMe;
        contract.shared_root_id = Some("revoked-root".into());
        bridge.db.set_file_contract_state(&contract).unwrap();

        let logs = capture_logs_async(async {
            let ((), ()) = tokio::join!(
                async {
                    bridge.process_due_operations(&sync_root, now_secs()).await.unwrap();
                },
                async {
                    tokio::time::timeout(Duration::from_secs(10), async {
                        while server.state.lock().unwrap().inits.is_empty() {
                            tokio::time::sleep(Duration::from_millis(5)).await;
                        }
                    })
                    .await
                    .expect("init never reached the mock");
                    // The revoked-share purge path (SD:2587-2620): a bulk delete by file id.
                    bridge.db.purge_revoked_shared_content(&[]).unwrap();
                }
            );
        })
        .await;

        let state = server.finish();
        assert!(
            bridge.db.get_upload_resume(&w.op_id).unwrap().is_none(),
            "no resume row for a purged op"
        );
        assert!(
            bridge.db.get_operation(&w.op_id).unwrap().is_none(),
            "a purged op is never re-inserted"
        );
        let moved: Vec<&str> = logs.lines().filter(|line| line.contains("queue state moved")).collect();
        assert_eq!(moved.len(), 1, "one line for the purged attempt:\n{logs}");
        assert!(moved[0].contains(&w.op_id) && moved[0].contains("resume"), "{logs}");
        assert!(
            !state
                .requests
                .iter()
                .any(|(_, path)| path.starts_with("/api/v1/uploads/session-1/")),
            "the purged op sends no chunk and no complete: {:?}",
            state.requests
        );
        assert_eq!(
            state.latest_plaintext("kept", master_key),
            b"the second file's save",
            "the other file's save is untouched and lands"
        );
    }

    /// What one refused attempt left behind when a purge removed its op between the
    /// failure and the runner's record of it.
    struct PurgedAttempt {
        op_id: String,
        outcome: TransferLoopOutcome,
        logs: String,
        init_statuses: Vec<u16>,
        op_left: bool,
    }

    /// One queued save whose `init` the server refuses with `refusal`. The share it
    /// belongs to is revoked after the attempt failed and before the runner records the
    /// outcome (the `outcome:before_tx` seam), from another thread.
    async fn refused_attempt_purged_before_its_outcome(refusal: &'static str) -> PurgedAttempt {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let server = VersionedServerMock::start();
        server
            .state
            .lock()
            .unwrap()
            .refuse_init
            .insert("purged".into(), refusal);
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), [43u8; 32]);
        seed_uploaded_row(&bridge, &server, "purged");
        queue_save(&bridge, dir.path(), "purged", "notes.txt", b"a save in a share", "1");
        let op_id = bridge.db.list_operations_for_file("purged").unwrap().remove(0).op_id;
        let mut contract = bridge.db.get_file_contract_state("purged").unwrap().unwrap();
        contract.namespace = Namespace::SharedWithMe;
        contract.shared_root_id = Some("purged-root".into());
        bridge.db.set_file_contract_state(&contract).unwrap();
        let db = bridge.db.clone();
        bridge.seams.arm("outcome:before_tx", move || {
            run_competing(move || {
                db.purge_revoked_shared_content(&[]).unwrap();
            })
        });

        let mut outcome = None;
        let logs = capture_logs_async(async {
            outcome = Some(bridge.process_due_operations(&sync_root, now_secs()).await.unwrap());
        })
        .await;
        let op_left = bridge.db.get_operation(&op_id).unwrap().is_some();
        let state = server.finish();
        PurgedAttempt {
            op_id,
            outcome: outcome.unwrap(),
            logs,
            init_statuses: state.inits.iter().map(|(_, status)| *status).collect(),
            op_left,
        }
    }

    fn assert_moved_once(run: &PurgedAttempt, step: &str) {
        assert!(!run.op_left, "a purged op is never re-inserted");
        assert!(
            run.outcome.paused_op_ids.is_empty()
                && run.outcome.retried_op_ids.is_empty()
                && run.outcome.completed_op_ids.is_empty(),
            "an attempt whose op moved is not reported: paused {:?}, retried {:?}, completed {:?}",
            run.outcome.paused_op_ids,
            run.outcome.retried_op_ids,
            run.outcome.completed_op_ids
        );
        let moved: Vec<&str> = run
            .logs
            .lines()
            .filter(|line| line.contains("queue state moved"))
            .collect();
        assert_eq!(moved.len(), 1, "one line for the purged attempt:\n{}", run.logs);
        assert!(
            moved[0].contains(&run.op_id) && moved[0].contains(&format!("step=\"{step}\"")),
            "{}",
            run.logs
        );
    }

    #[tokio::test]
    async fn a_pause_for_an_op_purged_under_its_claim_writes_nothing() {
        let run = refused_attempt_purged_before_its_outcome("403 Forbidden").await;
        assert_eq!(run.init_statuses, vec![403], "the attempt was refused as a pause");
        assert_moved_once(&run, "pause");
    }

    #[tokio::test]
    async fn an_attempt_for_an_op_purged_under_its_claim_writes_nothing() {
        let run = refused_attempt_purged_before_its_outcome("500 Internal Server Error").await;
        assert_eq!(run.init_statuses, vec![500], "the attempt failed as a retry");
        assert_moved_once(&run, "attempt");
    }

    #[tokio::test]
    async fn a_modify_queued_while_its_create_uploads_lands_on_the_created_file() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [25u8; 32];
        let server = VersionedServerMock::start();
        // The create's chunk takes a while, so the edit arrives mid-upload.
        server
            .state
            .lock()
            .unwrap()
            .delay_chunks
            .insert("session-1".into(), Duration::from_millis(400));
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);

        let created = dir.path().join("created.txt");
        std::fs::write(&created, b"created bytes").unwrap();
        bridge
            .queue_file_provider_create(finder_file_target(None, "t.txt", &created, None))
            .unwrap();
        let local_id = bridge.db.list_files().unwrap()[0].file_id.clone();

        let ((), ()) = tokio::join!(
            async {
                drain_upload_queue(&bridge, &sync_root).await;
            },
            async {
                while server.state.lock().unwrap().inits.is_empty() {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                // The system holds the item under its provisional id.
                let base = held_content_version(&bridge, &local_id);
                queue_save(&bridge, dir.path(), &local_id, "t.txt", b"created bytes, edited", &base);
                assert!(
                    server.state.lock().unwrap().files_with_content().is_empty(),
                    "the edit must be queued while the create is still uploading"
                );
            }
        );
        drain_upload_queue(&bridge, &sync_root).await;

        let state = server.finish();
        let server_id = {
            let files = state.files_with_content();
            assert_eq!(
                files.len(),
                1,
                "exactly one file on the server: {:?}",
                state.init_summary()
            );
            files[0].clone()
        };
        assert!(
            !state.files.contains_key(&local_id),
            "no file may be created under the provisional id"
        );
        assert_eq!(state.files[&server_id].versions.len(), 2, "{:?}", state.init_summary());
        assert_eq!(
            state.latest_plaintext(&server_id, master_key),
            b"created bytes, edited",
            "the edit is the created file's latest version"
        );
        assert_eq!(
            state.inits[1].0["base_version_number"],
            serde_json::json!(1),
            "the edit is based on the created version"
        );
        let mk = beebeeb_core::kdf::MasterKey::from_bytes(master_key);
        for (id, body) in state.patches.iter().filter(|(id, _)| id == &server_id) {
            let name = body["name_encrypted"].as_str().unwrap();
            assert_eq!(
                beebeeb_core::encrypt::decrypt_name(&mk, id, name).ok().as_deref(),
                Some("t.txt"),
                "every name sent for the server file is encrypted under its id"
            );
        }
        let rows = bridge.db.list_files().unwrap();
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0].file_id, server_id);
        assert_eq!(rows[0].status, FileStatus::Local);
    }

    // ── Observability: a refused upload leaves a trace ─────────────────────

    /// Everything `tracing` emits on this thread while `body` runs, as text.
    /// `#[tokio::test]` runs the future on the test's own thread, so a
    /// thread-local subscriber sees all of it.
    async fn capture_logs_async(body: impl std::future::Future<Output = ()>) -> String {
        #[derive(Clone)]
        struct Capture(Arc<Mutex<Vec<u8>>>);
        impl std::io::Write for Capture {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let writer = Capture(buffer.clone());
        // tracing caches each callsite's interest for the whole process. While
        // only ONE dispatcher is alive, tracing-core takes that interest from
        // the registering thread's own default (`has_just_one`), so a
        // callsite first hit by another test thread, which has no subscriber,
        // is cached as "never" and this capture misses its events. A second
        // live dispatcher keeps registration on the all-dispatchers path.
        let _second_dispatcher = tracing::Dispatch::new(tracing_subscriber::registry());
        let subscriber = tracing_subscriber::fmt()
            .with_writer(move || writer.clone())
            .with_ansi(false)
            .with_max_level(tracing::Level::INFO)
            .finish();
        let guard = tracing::subscriber::set_default(subscriber);
        body.await;
        drop(guard);
        let bytes = buffer.lock().unwrap().clone();
        String::from_utf8(bytes).unwrap()
    }

    #[tokio::test]
    async fn an_upload_refused_with_a_stale_base_is_logged_once_and_parks() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), [26u8; 32]);
        // The server is at version 2; this File Provider save is based on 1 (stale).
        // Only a Finder write parks at once (spec §8.4).
        seed_uploaded_row(&bridge, &server, "stale-file");
        server.seed_file("stale-file", 2);
        fp_save(
            &bridge,
            dir.path(),
            "stale-file",
            "private-notes.txt",
            b"stale save",
            "1",
        );
        let stale = bridge.db.list_operations_for_file("stale-file").unwrap().remove(0);
        // Another upload fails once for a different reason (500): not a 409.
        seed_uploaded_row(&bridge, &server, "flaky-file");
        server
            .state
            .lock()
            .unwrap()
            .fail_first_chunk_once
            .insert("session-1".into());
        fp_save(&bridge, dir.path(), "flaky-file", "other-notes.txt", b"flaky save", "1");
        let flaky = bridge.db.list_operations_for_file("flaky-file").unwrap().remove(0);

        let logs = capture_logs_async(async {
            drain_upload_queue(&bridge, &sync_root).await;
        })
        .await;
        let state = server.finish();
        let stale_inits: Vec<_> = state
            .init_summary()
            .into_iter()
            .filter(|(file, _, _)| *file == json!("stale-file"))
            .collect();
        assert_eq!(
            stale_inits,
            vec![(json!("stale-file"), json!(1), 409)],
            "one refused attempt, then parked: {:?}",
            state.init_summary()
        );
        let refused: Vec<&str> = logs.lines().filter(|line| line.contains("upload refused")).collect();
        assert_eq!(refused.len(), 1, "one line for the one refused attempt, got:\n{logs}");
        assert!(refused[0].contains("WARN"), "{logs}");
        assert!(
            refused[0].contains("parked") && refused[0].contains("stale_base"),
            "the refusal says the op parked, and why:\n{logs}"
        );
        assert!(
            refused[0].contains(&stale.op_id) && refused[0].contains("stale-file") && refused[0].contains("409"),
            "{logs}"
        );
        assert!(
            !refused.iter().any(|line| line.contains(&flaky.op_id)),
            "a failure other than 409 is not logged as one:\n{logs}"
        );
        assert!(
            logs.lines()
                .all(|line| !line.contains("notes.txt") && !line.contains("/api/v1") && !line.contains("127.0.0.1")),
            "no name, path or URL may reach any log line:\n{logs}"
        );
        assert!(
            !logs.contains(crate::api_client::STALE_BASE_MESSAGE),
            "the server's text never reaches the log, only the class:\n{logs}"
        );
        let parked = bridge.db.list_operations_for_file("stale-file").unwrap().remove(0);
        assert_eq!(parked.attempts, parked.max_attempts, "parked after one attempt");
    }

    // ── Rule 4: the 409 classes, the immediate parks and the hand-over (spec §8.4) ──

    #[test]
    fn the_409_messages_are_pinned() {
        use crate::api_client::{InitConflictClass, classify_init_conflict};
        assert_eq!(
            classify_init_conflict("stale base version for replacement upload"),
            InitConflictClass::StaleBase
        );
        assert_eq!(
            classify_init_conflict("upload is already in progress for this file"),
            InitConflictClass::InProgress
        );
        assert_eq!(classify_init_conflict("file is in trash"), InitConflictClass::Other);
        assert_eq!(
            classify_init_conflict(""),
            InitConflictClass::Other,
            "a changed message falls back to retry"
        );
    }

    #[tokio::test]
    async fn a_stale_base_409_parks_at_once() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [65u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_uploaded_row(&bridge, &server, "stale");
        server.seed_file("stale", 2); // the server moved on
        fp_save(&bridge, dir.path(), "stale", "notes.txt", b"edit on v1", "1");
        let logs = capture_logs_async(async {
            bridge.process_due_operations(&sync_root, now_secs()).await.unwrap();
        })
        .await;
        let op = bridge.db.list_operations_for_file("stale").unwrap().remove(0);
        assert_eq!(op.attempts, op.max_attempts, "parked after one attempt");
        let refused: Vec<&str> = logs.lines().filter(|l| l.contains("upload refused")).collect();
        assert_eq!(refused.len(), 1, "{logs}");
        assert!(
            refused[0].contains("stale_base") && refused[0].contains("parked"),
            "{logs}"
        );
        drop(server.finish());
    }

    #[tokio::test]
    async fn an_in_progress_409_is_retried() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [66u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_uploaded_row(&bridge, &server, "busy");
        server
            .state
            .lock()
            .unwrap()
            .conflict_init_once
            .insert("busy".into(), crate::api_client::IN_PROGRESS_MESSAGE.into());
        fp_save(&bridge, dir.path(), "busy", "notes.txt", b"edit", "1");
        drain_upload_queue(&bridge, &sync_root).await;
        let state = server.finish();
        assert_eq!(
            state.init_summary(),
            vec![(json!("busy"), json!(1), 409), (json!("busy"), json!(1), 201)]
        );
        assert_eq!(state.latest_plaintext("busy", master_key), b"edit");
    }

    #[tokio::test]
    async fn i4_a_doomed_earlier_write_never_blocks_hours() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [67u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_uploaded_row(&bridge, &server, "doomed");
        let w = fp_save(&bridge, dir.path(), "doomed", "notes.txt", b"W", "1");
        fp_save(
            &bridge,
            dir.path(),
            "doomed",
            "notes.txt",
            b"W N",
            w.token.as_deref().unwrap(),
        );
        let w_op = bridge.db.list_operations_for_file("doomed").unwrap().remove(0);
        std::fs::remove_file(w_op.payload_path.as_deref().unwrap()).unwrap();
        let (passes, logs) = {
            let mut passes = 0;
            let logs = capture_logs_async(async {
                passes = drain_upload_queue(&bridge, &sync_root).await;
            })
            .await;
            (passes, logs)
        };
        let state = server.finish();
        assert!(
            passes <= 2,
            "the successor lands in the same or the next pass: {passes}"
        );
        assert_eq!(
            state.init_summary(),
            vec![(json!("doomed"), json!(1), 201)],
            "N took W's base"
        );
        assert_eq!(state.latest_plaintext("doomed", master_key), b"W N");
        assert_eq!(logs.matches("queued write took over a parked one").count(), 1, "{logs}");
        assert!(bridge.db.list_operations_for_file("doomed").unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_stale_base_predecessor_hands_over_and_the_successor_parks_with_the_newest_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [68u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_uploaded_row(&bridge, &server, "stale-chain");
        server.seed_file("stale-chain", 2);
        let w = fp_save(&bridge, dir.path(), "stale-chain", "notes.txt", b"W", "1");
        fp_save(
            &bridge,
            dir.path(),
            "stale-chain",
            "notes.txt",
            b"W N",
            w.token.as_deref().unwrap(),
        );
        let logs = capture_logs_async(async {
            drain_upload_queue(&bridge, &sync_root).await;
        })
        .await;
        let state = server.finish();
        assert_eq!(
            state.init_summary(),
            vec![
                (json!("stale-chain"), json!(1), 409),
                (json!("stale-chain"), json!(1), 409)
            ],
            "W and then N, on W's base"
        );
        let ops = bridge.db.list_operations_for_file("stale-chain").unwrap();
        assert_eq!(ops.len(), 1, "W's op is gone; one parked op remains");
        assert_eq!(ops[0].attempts, ops[0].max_attempts);
        assert_eq!(
            std::fs::read(ops[0].payload_path.as_deref().unwrap()).unwrap(),
            b"W N",
            "the newest bytes are kept"
        );
        assert_eq!(logs.matches("queued write took over a parked one").count(), 1, "{logs}");
    }

    #[tokio::test]
    async fn an_earlier_builds_op_is_never_handed_over() {
        // m-2: an earlier build's bytes may not be contained in a newer save's (spec §3).
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [69u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        // (a) an earlier-build op and a newer save on "1": both sets of bytes kept.
        seed_uploaded_row(&bridge, &server, "earlier");
        fp_save(
            &bridge,
            dir.path(),
            "earlier",
            "notes.txt",
            b"earlier build's bytes",
            "1",
        );
        let e = bridge.db.list_operations_for_file("earlier").unwrap().remove(0);
        bridge
            .db
            .set_write_origin_for_test(&e.op_id, crate::state_db::WriteOrigin::EarlierBuild);
        fp_save(&bridge, dir.path(), "earlier", "notes.txt", b"a newer save", "1");
        // (b) a provisional row whose earlier-build create parks with a save queued after it.
        let created = fp_create(&bridge, dir.path(), "made-earlier.txt", b"created");
        let provisional = created.outcome_file_id();
        fp_save(
            &bridge,
            dir.path(),
            &provisional,
            "made-earlier.txt",
            b"created, edited",
            created.token.as_deref().unwrap(),
        );
        let c = bridge.db.list_operations_for_file(&provisional).unwrap().remove(0);
        bridge
            .db
            .set_write_origin_for_test(&c.op_id, crate::state_db::WriteOrigin::EarlierBuild);
        bridge.db.park_for_test(&c.op_id);
        let logs = capture_logs_async(async {
            drain_upload_queue(&bridge, &sync_root).await;
        })
        .await;
        let state = server.finish();
        assert_eq!(
            state.latest_plaintext("earlier", master_key),
            b"earlier build's bytes",
            "(a) E landed"
        );
        let newer = bridge.db.list_operations_for_file("earlier").unwrap().remove(0);
        assert_eq!(
            newer.attempts, newer.max_attempts,
            "(a) the newer save parked, not dropped"
        );
        assert!(std::path::Path::new(newer.payload_path.as_deref().unwrap()).is_file());
        let ops = bridge.db.list_operations_for_file(&provisional).unwrap();
        assert_eq!(ops.len(), 2, "(b) no hand-over: C and N both remain");
        assert!(ops.iter().all(|op| op.attempts == op.max_attempts));
        assert!(logs.contains("predecessor_parked"), "{logs}");
        assert!(!logs.contains("took over"), "{logs}");
        assert!(
            state
                .inits
                .iter()
                .all(|(body, _)| body["file_id"] != json!(provisional)),
            "(b) nothing uploaded"
        );
    }

    #[tokio::test]
    async fn handover_vs_new_save_the_newest_bytes_land_last() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [70u8; 32];
        let server = VersionedServerMock::start();
        let bridge = Arc::new(test_bridge_with_api(
            &dir.path().join("state.db"),
            server.base_url.clone(),
            master_key,
        ));
        seed_uploaded_row(&bridge, &server, "handover");
        let w = fp_save(&bridge, dir.path(), "handover", "notes.txt", b"W", "1");
        let w_op = bridge.db.list_operations_for_file("handover").unwrap().remove(0);
        std::fs::remove_file(w_op.payload_path.as_deref().unwrap()).unwrap();
        bridge.process_due_operations(&sync_root, now_secs()).await.unwrap(); // W parks payload_missing
        let n = fp_save(
            &bridge,
            dir.path(),
            "handover",
            "notes.txt",
            b"W N",
            w.token.as_deref().unwrap(),
        );
        // At N's claim, a newer save N2 is accepted on N's token.
        let saver = Arc::clone(&bridge);
        let save_dir = dir.path().to_path_buf();
        let n_token = n.token.clone().unwrap();
        let fired = Arc::new(AtomicBool::new(false));
        let fired_in_seam = Arc::clone(&fired);
        bridge.seams.arm("claim:before_tx", move || {
            run_competing(move || {
                fp_save(&saver, &save_dir, "handover", "notes.txt", b"W N N2", &n_token);
            });
            fired_in_seam.store(true, Ordering::SeqCst);
        });
        let logs = capture_logs_async(async {
            drain_upload_queue(&bridge, &sync_root).await;
        })
        .await;
        let state = server.finish();
        assert!(fired.load(Ordering::SeqCst), "the seam fired and N2 was accepted");
        assert_eq!(
            state.init_summary(),
            vec![(json!("handover"), json!(1), 201), (json!("handover"), json!(2), 201)],
            "N on W's base, then N2"
        );
        assert_eq!(
            state.latest_plaintext("handover", master_key),
            b"W N N2",
            "the newest bytes land last"
        );
        assert_eq!(logs.matches("queued write took over a parked one").count(), 1, "{logs}");
    }

    #[tokio::test]
    async fn a_lost_reply_base_parks_with_its_bytes() {
        // Review Focus 4: the shipped fallback of the split rule 1c.
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [71u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_uploaded_row(&bridge, &server, "lost-reply");
        let a = fp_save(&bridge, dir.path(), "lost-reply", "notes.txt", b"A", "1");
        drain_upload_queue(&bridge, &sync_root).await;
        let a_token = a.token.unwrap();
        let _b_reply_lost = fp_save(&bridge, dir.path(), "lost-reply", "notes.txt", b"A B", &a_token);
        fp_save(&bridge, dir.path(), "lost-reply", "notes.txt", b"A B C", &a_token); // still A's token
        let logs = capture_logs_async(async {
            drain_upload_queue(&bridge, &sync_root).await;
        })
        .await;
        let state = server.finish();
        assert_eq!(
            state.init_summary(),
            vec![
                (json!("lost-reply"), json!(1), 201),
                (json!("lost-reply"), json!(2), 201),
                (json!("lost-reply"), json!(1), 409)
            ]
        );
        assert_eq!(state.latest_plaintext("lost-reply", master_key), b"A B");
        let parked = bridge.db.list_operations_for_file("lost-reply").unwrap().remove(0);
        assert_eq!(parked.attempts, parked.max_attempts, "a visible park");
        assert_eq!(
            std::fs::read(parked.payload_path.as_deref().unwrap()).unwrap(),
            b"A B C",
            "its bytes are kept"
        );
        assert!(
            logs.lines()
                .any(|l| l.contains("upload refused") && l.contains("parked")),
            "{logs}"
        );
    }

    #[tokio::test]
    async fn a_parked_predecessor_with_a_recorded_completion_is_waited_for_never_taken_over() {
        // M19, spec §8.4: "W has a recorded completion: W never parks (§8.6 rule 6). N waits,
        // and the chain step resolves it when W's local landing succeeds." W stands for an op
        // parked before that rule existed (Task 7 records the completion).
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [72u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_uploaded_row(&bridge, &server, "completed");
        let w = fp_save(&bridge, dir.path(), "completed", "notes.txt", b"W", "1");
        fp_save(
            &bridge,
            dir.path(),
            "completed",
            "notes.txt",
            b"W N",
            w.token.as_deref().unwrap(),
        );
        let ops = bridge.db.list_operations_for_file("completed").unwrap();
        let (w_op, n_op) = (ops[0].clone(), ops[1].clone());
        bridge
            .db
            .put_upload_resume(&UploadResume {
                op_id: w_op.op_id.clone(),
                payload_path: w_op.payload_path.clone().unwrap(),
                payload_size: 1,
                payload_mtime_ns: 1,
                upload_session_id: "session-w".into(),
                server_file_id: "completed".into(),
                object_version_id: "object-w".into(),
                chunk_size_bytes: 1,
                chunk_count: 1,
                acked_chunks: 1,
                metadata_applied: true,
                is_create: false,
                completed_version: None,
                completed_object_version_id: None,
                completed_mime_type: None,
            })
            .unwrap();
        bridge.db.set_completed_for_test(&w_op.op_id, 2);
        bridge.db.park_for_test(&w_op.op_id);
        let logs = capture_logs_async(async {
            bridge.process_due_operations(&sync_root, now_secs()).await.unwrap();
        })
        .await;
        let state = server.finish();
        assert!(
            state.inits.is_empty(),
            "nothing is sent while W's landing is pending: {:?}",
            state.init_summary()
        );
        let ops = bridge.db.list_operations_for_file("completed").unwrap();
        assert_eq!(ops.len(), 2, "W is never taken over: both ops remain");
        assert_eq!(ops[1].op_id, n_op.op_id);
        assert_eq!(ops[1].attempts, 0, "N waits, and waiting is not an attempt");
        assert_eq!(
            bridge.db.finder_write(&n_op.op_id).unwrap().unwrap().after_write_id,
            bridge.db.finder_write(&w_op.op_id).unwrap().map(|write| write.write_id),
            "N still waits after W"
        );
        assert!(!logs.contains("took over"), "{logs}");
        assert!(std::path::Path::new(w_op.payload_path.as_deref().unwrap()).is_file());
    }

    #[tokio::test]
    async fn a_hand_over_that_inherits_an_earlier_builds_predecessor_parks_and_releases_the_copy() {
        // X (an earlier build's op, parked) <- W <- N. W parks behind X at its claim (m-2); at
        // N's claim N takes W's role, inherits X, and parks too. The hand-over is still
        // logged, and W's copy is unlinked after the commit (S6).
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [73u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_uploaded_row(&bridge, &server, "chain-park");
        let x = fp_save(&bridge, dir.path(), "chain-park", "notes.txt", b"X", "1");
        let x_op = bridge.db.list_operations_for_file("chain-park").unwrap().remove(0);
        bridge
            .db
            .set_write_origin_for_test(&x_op.op_id, crate::state_db::WriteOrigin::EarlierBuild);
        bridge.db.park_for_test(&x_op.op_id);
        let w = fp_save(
            &bridge,
            dir.path(),
            "chain-park",
            "notes.txt",
            b"X W",
            x.token.as_deref().unwrap(),
        );
        fp_save(
            &bridge,
            dir.path(),
            "chain-park",
            "notes.txt",
            b"X W N",
            w.token.as_deref().unwrap(),
        );
        let ops = bridge.db.list_operations_for_file("chain-park").unwrap();
        let (w_op, n_op) = (ops[1].clone(), ops[2].clone());
        let logs = capture_logs_async(async {
            bridge.process_due_operations(&sync_root, now_secs()).await.unwrap();
        })
        .await;
        let state = server.finish();
        assert!(state.inits.is_empty(), "nothing is sent: {:?}", state.init_summary());
        let ops = bridge.db.list_operations_for_file("chain-park").unwrap();
        assert_eq!(
            ops.iter().map(|op| op.op_id.clone()).collect::<Vec<_>>(),
            vec![x_op.op_id.clone(), n_op.op_id.clone()],
            "W's op is gone; X and N remain"
        );
        assert!(
            ops.iter().all(|op| op.attempts == op.max_attempts),
            "both parked with their bytes"
        );
        assert_eq!(
            std::fs::read(ops[1].payload_path.as_deref().unwrap()).unwrap(),
            b"X W N"
        );
        assert_eq!(logs.matches("queued write took over a parked one").count(), 1, "{logs}");
        assert!(
            !std::path::Path::new(w_op.payload_path.as_deref().unwrap()).exists(),
            "W's copy is released after the commit"
        );
        assert!(
            logs.lines()
                .any(|line| line.contains("predecessor_parked") && line.contains(&n_op.op_id)),
            "{logs}"
        );
    }

    #[tokio::test]
    async fn a_base_unknown_predecessor_hands_over_and_the_successor_parks_without_a_request() {
        // Ruling [t5-c2]: W parked at accept under rule 6a′, because its base cannot be known.
        // N, saved on W's token, takes W's role at its claim and parks `base_unknown` there:
        // no request goes out with a base the client knows is unknown.
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [74u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_uploaded_row(&bridge, &server, "unknown-base"); // version 1, its version not filled
        let w = fp_save(&bridge, dir.path(), "unknown-base", "notes.txt", b"W", "0");
        let w_op = bridge.db.list_operations_for_file("unknown-base").unwrap().remove(0);
        assert_eq!(w_op.attempts, w_op.max_attempts, "rule 6a′ parked W at accept");
        fp_save(
            &bridge,
            dir.path(),
            "unknown-base",
            "notes.txt",
            b"W N",
            w.token.as_deref().unwrap(),
        );
        let n_op = bridge.db.list_operations_for_file("unknown-base").unwrap().remove(1);
        let logs = capture_logs_async(async {
            bridge.process_due_operations(&sync_root, now_secs()).await.unwrap();
        })
        .await;
        let state = server.finish();
        assert_eq!(
            state.requests.len(),
            0,
            "no request with a base the client knows is unknown: {:?}",
            state.init_summary()
        );
        let ops = bridge.db.list_operations_for_file("unknown-base").unwrap();
        assert_eq!(ops.len(), 1, "W's op is gone; N remains");
        assert_eq!(ops[0].op_id, n_op.op_id);
        assert_eq!(ops[0].attempts, ops[0].max_attempts, "N parked at its claim");
        assert_eq!(ops[0].last_error.as_deref(), Some("base_unknown"));
        assert_eq!(
            std::fs::read(ops[0].payload_path.as_deref().unwrap()).unwrap(),
            b"W N",
            "its bytes are kept"
        );
        assert!(
            !std::path::Path::new(w_op.payload_path.as_deref().unwrap()).exists(),
            "W's copy is released after the commit"
        );
        assert_eq!(logs.matches("queued write took over a parked one").count(), 1, "{logs}");
        assert_eq!(
            logs.lines()
                .filter(|line| line.contains("upload parked")
                    && line.contains(&n_op.op_id)
                    && line.contains("base_unknown"))
                .count(),
            1,
            "{logs}"
        );
    }

    #[tokio::test]
    async fn a_parked_create_hands_over_and_the_save_lands_as_the_create() {
        // The create branch of the hand-over (spec §8.4): a Finder create W whose staged copy
        // is gone parks; the save N on W's token takes W's create at its claim and lands as
        // the new file's first version, with N's bytes (they contain W's).
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [75u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        let created = fp_create(&bridge, dir.path(), "made.txt", b"created");
        let provisional = created.outcome_file_id();
        let w_op = bridge.db.list_operations_for_file(&provisional).unwrap().remove(0);
        std::fs::remove_file(w_op.payload_path.as_deref().unwrap()).unwrap();
        fp_save(
            &bridge,
            dir.path(),
            &provisional,
            "made.txt",
            b"created, edited",
            created.token.as_deref().unwrap(),
        );
        let logs = capture_logs_async(async {
            drain_upload_queue(&bridge, &sync_root).await;
        })
        .await;
        let state = server.finish();
        assert_eq!(
            state.init_summary(),
            vec![(json!(null), json!(null), 201)],
            "one create, without a base"
        );
        let server_id = {
            let files = state.files_with_content();
            assert_eq!(files.len(), 1, "exactly one file on the server: {files:?}");
            files[0].clone()
        };
        assert!(
            !state.files.contains_key(&provisional),
            "no file is created under the provisional id"
        );
        assert_eq!(state.files[&server_id].versions.len(), 1, "N's bytes are version 1");
        assert_eq!(state.latest_plaintext(&server_id, master_key), b"created, edited");
        let rows = bridge.db.list_files().unwrap();
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0].file_id, server_id, "the provisional id was swapped");
        assert_eq!(rows[0].status, FileStatus::Local);
        assert!(bridge.db.list_operations_for_file(&provisional).unwrap().is_empty());
        assert!(bridge.db.list_operations_for_file(&server_id).unwrap().is_empty());
        assert_eq!(logs.matches("queued write took over a parked one").count(), 1, "{logs}");
    }

    #[tokio::test]
    async fn a_hand_over_that_inherits_a_pending_base_waits_and_releases_the_copy() {
        // `ClaimOutcome::WaitAfterHandOver`. Not reachable in production today: nothing parks an
        // op whose base is pending. W is parked by hand here. N takes W's role, inherits the
        // pending base, and waits: the hand-over is still logged once and W's copy unlinked.
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [76u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        bridge
            .db
            .upsert_file(&FileEntry {
                file_id: "versionless".into(),
                path: "notes.txt".into(),
                status: FileStatus::Local,
                size_bytes: 10,
                modified_at: 1_700_000_000,
                content_hash: None,
                remote_updated_at: 1_700_000_000,
                parent_id: None,
                item_kind: ItemKind::File,
            })
            .unwrap();
        assert_eq!(
            bridge
                .db
                .get_file_contract_state("versionless")
                .unwrap()
                .unwrap()
                .current_version,
            0
        );
        let w = fp_save(&bridge, dir.path(), "versionless", "notes.txt", b"W", "0"); // rule 6b
        let w_op = bridge.db.list_operations_for_file("versionless").unwrap().remove(0);
        assert_eq!(bridge.db.finder_write(&w_op.op_id).unwrap().unwrap().base_pending, 1);
        fp_save(
            &bridge,
            dir.path(),
            "versionless",
            "notes.txt",
            b"W N",
            w.token.as_deref().unwrap(),
        );
        let n_op = bridge.db.list_operations_for_file("versionless").unwrap().remove(1);
        bridge.db.park_for_test(&w_op.op_id);
        let logs = capture_logs_async(async {
            bridge.process_due_operations(&sync_root, now_secs()).await.unwrap();
        })
        .await;
        let state = server.finish();
        assert!(state.requests.is_empty(), "nothing is sent: {:?}", state.requests);
        let ops = bridge.db.list_operations_for_file("versionless").unwrap();
        assert_eq!(ops.len(), 1, "W's op is gone; N remains");
        assert_eq!(ops[0].op_id, n_op.op_id);
        assert_eq!(ops[0].attempts, 0, "N waits, and waiting is not an attempt");
        let n_write = bridge.db.finder_write(&n_op.op_id).unwrap().unwrap();
        assert_eq!(
            (n_write.base_pending, n_write.after_write_id),
            (1, None),
            "N inherited W's pending base"
        );
        assert_eq!(logs.matches("queued write took over a parked one").count(), 1, "{logs}");
        assert!(
            !std::path::Path::new(w_op.payload_path.as_deref().unwrap()).exists(),
            "W's copy is released after the commit"
        );
        assert!(!logs.contains("upload parked"), "{logs}");
    }

    // ── The accept transaction and the base mapping (spec §6.1, §6.3.1, §8.7 S1.1, S3, S5) ──

    #[tokio::test]
    async fn i2_a_zero_base_on_a_versioned_row_parks() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [44u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_uploaded_row(&bridge, &server, "zero-base");
        fp_save(&bridge, dir.path(), "zero-base", "notes.txt", b"edit on a stale 0", "0");
        drain_upload_queue(&bridge, &sync_root).await;
        let state = server.finish();
        assert!(state.inits.is_empty(), "nothing is sent: {:?}", state.init_summary());
        let op = bridge.db.list_operations_for_file("zero-base").unwrap().remove(0);
        assert_eq!(op.attempts, op.max_attempts, "parked at once");
        assert!(
            std::path::Path::new(op.payload_path.as_deref().unwrap()).is_file(),
            "the bytes are kept"
        );
    }

    #[tokio::test]
    async fn the_init_guard_refuses_a_finder_replace_without_a_base() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [45u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_uploaded_row(&bridge, &server, "too-big");
        fp_save(
            &bridge,
            dir.path(),
            "too-big",
            "notes.txt",
            b"a base beyond i32",
            "3000000000",
        );
        seed_uploaded_row(&bridge, &server, "no-base");
        fp_save(
            &bridge,
            dir.path(),
            "no-base",
            "notes.txt",
            b"a base that went missing",
            "1",
        );
        let op = bridge.db.list_operations_for_file("no-base").unwrap().remove(0);
        bridge.db.set_base_version_for_test(&op.op_id, None);
        drain_upload_queue(&bridge, &sync_root).await;
        let state = server.finish();
        assert!(
            state.inits.is_empty(),
            "no replace without a base: {:?}",
            state.init_summary()
        );
        for file in ["too-big", "no-base"] {
            let op = bridge.db.list_operations_for_file(file).unwrap().remove(0);
            assert_eq!(op.attempts, op.max_attempts, "{file} parked");
        }
    }

    #[tokio::test]
    async fn a_newer_save_queues_behind_a_write_without_a_session_and_both_land() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [46u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_uploaded_row(&bridge, &server, "two");
        let first = fp_save(&bridge, dir.path(), "two", "notes.txt", b"first", "1");
        fp_save(
            &bridge,
            dir.path(),
            "two",
            "notes.txt",
            b"first, second",
            first.token.as_deref().unwrap_or("1"),
        );
        drain_upload_queue(&bridge, &sync_root).await;
        let state = server.finish();
        assert_eq!(
            state.init_summary(),
            vec![(json!("two"), json!(1), 201), (json!("two"), json!(2), 201)],
            "one upload per save, in order"
        );
        assert_eq!(
            state.files["two"].versions.len(),
            3,
            "both saves are versions in the history"
        );
        assert_eq!(state.latest_plaintext("two", master_key), b"first, second");
    }

    #[tokio::test]
    async fn a_newer_save_waits_behind_a_live_session() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [47u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        server
            .state
            .lock()
            .unwrap()
            .delay_chunks
            .insert("session-1".into(), Duration::from_millis(400));
        seed_uploaded_row(&bridge, &server, "live");
        let first = fp_save(&bridge, dir.path(), "live", "notes.txt", b"first", "1");
        let first_token = first.token.clone().unwrap();
        let ((), ()) = tokio::join!(
            async {
                drain_upload_queue(&bridge, &sync_root).await;
            },
            async {
                tokio::time::timeout(Duration::from_secs(10), async {
                    while server.state.lock().unwrap().inits.is_empty() {
                        tokio::time::sleep(Duration::from_millis(5)).await;
                    }
                })
                .await
                .expect("init never reached the mock");
                fp_save(&bridge, dir.path(), "live", "notes.txt", b"first, second", &first_token);
            }
        );
        drain_upload_queue(&bridge, &sync_root).await;
        let state = server.finish();
        assert_eq!(
            state.init_summary(),
            vec![(json!("live"), json!(1), 201), (json!("live"), json!(2), 201)],
            "the second save is based on what the first produced"
        );
        assert_eq!(state.latest_plaintext("live", master_key), b"first, second");
    }

    #[tokio::test]
    async fn enqueue_vs_runner_a_save_accepted_while_its_predecessor_lands_is_never_orphaned() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [48u8; 32];
        let server = VersionedServerMock::start();
        // The bridge is shared with the competing thread.
        let bridge = Arc::new(test_bridge_with_api(
            &dir.path().join("state.db"),
            server.base_url.clone(),
            master_key,
        ));
        seed_uploaded_row(&bridge, &server, "race");
        let w = fp_save(&bridge, dir.path(), "race", "notes.txt", b"W", "1");
        let w_token = w.token.clone().unwrap();
        // N's accept stops at its seam; W's landing commits meanwhile (review sequence C).
        let runner = Arc::clone(&bridge);
        let root = sync_root.clone();
        bridge.seams.arm("accept:before_tx", move || {
            run_competing(move || {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap()
                    .block_on(async {
                        runner.process_due_operations(&root, now_secs()).await.unwrap();
                    });
            });
        });
        let logs = capture_logs_async(async {
            fp_save(&bridge, dir.path(), "race", "notes.txt", b"W, then N", &w_token);
            drain_upload_queue(&bridge, &sync_root).await;
        })
        .await;
        let state = server.finish();
        assert_eq!(
            state.init_summary(),
            vec![(json!("race"), json!(1), 201), (json!("race"), json!(2), 201)],
            "N is based on W's produced version"
        );
        assert_eq!(state.latest_plaintext("race", master_key), b"W, then N");
        assert!(!logs.contains("predecessor_lost"), "{logs}");
        assert!(
            bridge.db.list_due_operations(i64::MAX).unwrap().is_empty(),
            "0 ops left"
        );
    }

    #[tokio::test]
    async fn a_numeric_base_while_a_minted_write_is_queued_follows_the_newest_write() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [49u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_uploaded_row(&bridge, &server, "chain");
        let w1 = fp_save(&bridge, dir.path(), "chain", "notes.txt", b"1", "1");
        fp_save(
            &bridge,
            dir.path(),
            "chain",
            "notes.txt",
            b"1 2",
            w1.token.as_deref().unwrap(),
        );
        // The system sent this save before it recorded either reply.
        fp_save(&bridge, dir.path(), "chain", "notes.txt", b"1 2 3", "1");
        drain_upload_queue(&bridge, &sync_root).await;
        let state = server.finish();
        assert_eq!(
            state.init_summary(),
            vec![
                (json!("chain"), json!(1), 201),
                (json!("chain"), json!(2), 201),
                (json!("chain"), json!(3), 201)
            ],
            "the third save follows the newest write of the chain"
        );
        assert_eq!(state.latest_plaintext("chain", master_key), b"1 2 3");
    }

    #[tokio::test]
    async fn an_unknown_token_is_sent_as_its_b() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [50u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_uploaded_row(&bridge, &server, "unknown-token");
        server.seed_file("unknown-token", 3);
        let mut contract = bridge.db.get_file_contract_state("unknown-token").unwrap().unwrap();
        contract.current_version = 3;
        bridge.db.set_file_contract_state(&contract).unwrap();
        fp_save(
            &bridge,
            dir.path(),
            "unknown-token",
            "notes.txt",
            b"edit",
            &format!("3:w{}", "c".repeat(32)),
        );
        drain_upload_queue(&bridge, &sync_root).await;
        let state = server.finish();
        assert_eq!(state.init_summary(), vec![(json!("unknown-token"), json!(3), 201)]);
    }

    #[tokio::test]
    async fn a_waiting_op_stores_its_b_as_base_version() {
        // m-4a: an older build would send this, and the server refuses it.
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [51u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_uploaded_row(&bridge, &server, "stores-b");
        let w1 = fp_save(&bridge, dir.path(), "stores-b", "notes.txt", b"1", "1");
        fp_save(
            &bridge,
            dir.path(),
            "stores-b",
            "notes.txt",
            b"1 2",
            w1.token.as_deref().unwrap(),
        );
        let ops = bridge.db.list_operations_for_file("stores-b").unwrap();
        assert_eq!(ops[1].base_version, Some(1), "after W1, stored as W1's b");
        let created = fp_create(&bridge, dir.path(), "new.txt", b"created");
        let provisional = created.outcome_file_id();
        fp_save(
            &bridge,
            dir.path(),
            &provisional,
            "new.txt",
            b"created, edited",
            created.token.as_deref().unwrap(),
        );
        let ops = bridge.db.list_operations_for_file(&provisional).unwrap();
        assert_eq!(ops[1].base_version, Some(0), "after a create, stored as 0");
        drop(server.finish());
    }

    // ── Rule 1: every surface reports the token (spec §5.3–§5.5, §5.4 rows 5–15) ──

    #[cfg(unix)] // asserts a token through `held_content_version`
    #[tokio::test]
    async fn a_landing_keeps_the_content_version_the_reply_named() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [52u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_uploaded_row(&bridge, &server, "named");
        let reply = fp_save(&bridge, dir.path(), "named", "notes.txt", b"edit", "1");
        let token = reply.token.clone().unwrap();
        assert!(token.starts_with("1:w"), "{token}");
        assert_eq!(held_content_version(&bridge, "named"), token, "while queued");
        drain_upload_queue(&bridge, &sync_root).await;
        assert_eq!(held_content_version(&bridge, "named"), token, "after our own landing");
        drop(server.finish());
    }

    /// W landed as v2 on `file_id`; returns W's token. Used by T2, T3, T4, T5, T6, T64.
    #[cfg(unix)] // asserts a token through `held_content_version`
    async fn land_one_save(
        bridge: &EngineBridge,
        server: &VersionedServerMock,
        dir: &Path,
        sync_root: &Path,
        file_id: &str,
    ) -> String {
        seed_uploaded_row(bridge, server, file_id);
        let token = fp_save(bridge, dir, file_id, "notes.txt", b"landed edit", "1")
            .token
            .unwrap();
        drain_upload_queue(bridge, sync_root).await;
        assert_eq!(held_content_version(bridge, file_id), token);
        token
    }

    #[cfg(unix)] // only the unix token tests call it
    fn remote_update(bridge: &EngineBridge, sync_root: &Path, file_id: &str, version: i64, object: &str) {
        let op = crate::api_client::SyncOp {
            seq_id: 100 + version,
            op_type: "file_update".into(),
            payload: serde_json::json!({
                "id": file_id,
                "version_number": version,
                "current_object_version_id": object,
                "size_bytes": 11
            }),
        };
        apply_sync_op(bridge, sync_root, &op, now_secs(), &mut Vec::new()).unwrap();
    }

    #[cfg(unix)] // asserts a token through `held_content_version`
    #[tokio::test]
    async fn a_remote_change_replaces_the_token() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [53u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        land_one_save(&bridge, &server, dir.path(), &sync_root, "remote").await;
        remote_update(&bridge, &sync_root, "remote", 3, "object-elsewhere-v3");
        assert_eq!(held_content_version(&bridge, "remote"), "3");
        assert!(
            bridge.db.item_presentation("remote").unwrap().unwrap().held.is_none(),
            "the builder cleared it"
        );

        // A new version under the SAME object id: the legacy one-shot `file_update` carries no
        // `current_object_version_id`, so the row keeps its own (EB:6169-6174).
        land_one_save(&bridge, &server, dir.path(), &sync_root, "remote-same-object").await;
        let replace = crate::api_client::SyncOp {
            seq_id: 200,
            op_type: "file_update".into(),
            payload: serde_json::json!({ "id": "remote-same-object", "version_number": 3, "size_bytes": 11 }),
        };
        apply_sync_op(&bridge, &sync_root, &replace, now_secs(), &mut Vec::new()).unwrap();
        assert_eq!(held_content_version(&bridge, "remote-same-object"), "3");
        drop(server.finish());
    }

    #[cfg(unix)] // asserts a token through `held_content_version`
    #[tokio::test]
    async fn our_own_echo_keeps_the_token() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [54u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        let token = land_one_save(&bridge, &server, dir.path(), &sync_root, "echo").await;
        remote_update(&bridge, &sync_root, "echo", 2, "object-echo-v2");
        assert_eq!(
            held_content_version(&bridge, "echo"),
            token,
            "the echo of our own landing changes nothing"
        );
        drop(server.finish());
    }

    #[cfg(unix)] // asserts a token through `held_content_version`
    #[cfg(target_os = "macos")] // tests the restore response, which only macOS applies
    #[tokio::test]
    async fn a_restore_from_this_mac_replaces_the_token() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [55u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        land_one_save(&bridge, &server, dir.path(), &sync_root, "restored").await;
        bridge
            .queue_restore_version("restored", "object-restored-v1", None)
            .unwrap();
        drain_upload_queue(&bridge, &sync_root).await;
        assert_eq!(
            held_content_version(&bridge, "restored"),
            "3",
            "the restore's version, so the system re-downloads"
        );
        drop(server.finish());
    }

    #[cfg(unix)] // asserts a token through `held_content_version`
    #[tokio::test]
    async fn the_token_survives_rename_move_and_trash() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [56u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        let token = land_one_save(&bridge, &server, dir.path(), &sync_root, "moved").await;
        let rename = FinderWriteTarget {
            file_id: Some("moved".into()),
            parent_id: None,
            filename: "renamed.txt".into(),
            rel_path: None,
            kind: FinderWriteItemKind::File,
            contents_path: None,
            content_type: Some("text/plain".into()),
            base_version_identifier: Some(token.clone()),
        };
        bridge.queue_file_provider_modify(rename).unwrap();
        assert_eq!(held_content_version(&bridge, "moved"), token, "rename");
        bridge.queue_finder_delete("moved", Some(token.clone())).unwrap();
        assert_eq!(held_content_version(&bridge, "moved"), token, "trash");
        drop(server.finish());
    }

    #[cfg(unix)] // asserts a token through `held_content_version`
    #[tokio::test]
    async fn sign_out_clears_tokens_and_aliases() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [57u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        land_one_save(&bridge, &server, dir.path(), &sync_root, "signed-out").await;
        bridge
            .db
            .insert_alias_for_test("provisional-1", "signed-out", now_secs());
        bridge.db.purge_all_local_state().unwrap();
        assert!(
            bridge
                .db
                .item_presentation("signed-out")
                .unwrap()
                .unwrap()
                .held
                .is_none()
        );
        assert_eq!(bridge.db.alias_count_for_test(), 0);
        drop(server.finish());
    }

    #[tokio::test]
    async fn sign_out_purges_provisional_rows_without_a_contract() {
        let dir = tempfile::tempdir().unwrap();
        let master_key = [58u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_uploaded_row(&bridge, &server, "server-known");
        let created = fp_create(&bridge, dir.path(), "never-uploaded.txt", b"local only");
        let provisional = created.outcome_file_id();
        bridge
            .db
            .insert_alias_for_test("older-provisional", "server-known", now_secs());
        bridge.db.purge_all_local_state().unwrap();
        assert!(
            bridge.db.get_file(&provisional).unwrap().is_none(),
            "no ghost row after sign-out"
        );
        assert!(
            bridge.db.get_file("server-known").unwrap().is_some(),
            "server-known rows are kept"
        );
        assert_eq!(bridge.db.alias_count_for_test(), 0);
        drop(server.finish());
    }

    #[cfg(unix)] // asserts a token through `held_content_version`
    #[tokio::test]
    async fn c1_a_save_after_the_first_landed_is_based_on_what_it_produced() {
        // The review's two passes.
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [59u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_uploaded_row(&bridge, &server, "c1");
        fp_save(&bridge, dir.path(), "c1", "notes.txt", b"A", "1");
        drain_upload_queue(&bridge, &sync_root).await;
        let base = held_content_version(&bridge, "c1"); // what the system holds after re-reading
        fp_save(&bridge, dir.path(), "c1", "notes.txt", b"A B", &base);
        drain_upload_queue(&bridge, &sync_root).await;
        let state = server.finish();
        assert_eq!(
            state.init_summary(),
            vec![(json!("c1"), json!(1), 201), (json!("c1"), json!(2), 201)]
        );
        assert_eq!(state.latest_plaintext("c1", master_key), b"A B");
    }

    #[tokio::test]
    async fn c1_three_saves_in_one_pass() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [60u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        server
            .state
            .lock()
            .unwrap()
            .delay_chunks
            .insert("session-1".into(), Duration::from_millis(400));
        seed_uploaded_row(&bridge, &server, "c3");
        let a = fp_save(&bridge, dir.path(), "c3", "notes.txt", b"A", "1");
        let ((), ()) = tokio::join!(
            async {
                drain_upload_queue(&bridge, &sync_root).await;
            },
            async {
                tokio::time::timeout(Duration::from_secs(10), async {
                    while server.state.lock().unwrap().inits.is_empty() {
                        tokio::time::sleep(Duration::from_millis(5)).await;
                    }
                })
                .await
                .expect("init never reached the mock");
                let b = fp_save(
                    &bridge,
                    dir.path(),
                    "c3",
                    "notes.txt",
                    b"A B",
                    a.token.as_deref().unwrap(),
                );
                fp_save(
                    &bridge,
                    dir.path(),
                    "c3",
                    "notes.txt",
                    b"A B C",
                    b.token.as_deref().unwrap(),
                );
            }
        );
        drain_upload_queue(&bridge, &sync_root).await;
        let state = server.finish();
        assert_eq!(
            state.init_summary(),
            vec![
                (json!("c3"), json!(1), 201),
                (json!("c3"), json!(2), 201),
                (json!("c3"), json!(3), 201)
            ]
        );
        assert_eq!(state.latest_plaintext("c3", master_key), b"A B C");
    }

    #[cfg(unix)] // asserts a token through `held_content_version`
    #[cfg(target_os = "macos")] // tests the restore response, which only macOS applies
    #[tokio::test]
    async fn a_restore_with_a_queued_write_runs_after_it_and_keeps_both_versions() {
        // m-9
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [61u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        // W's first attempt fails, so the runner reaches the restore while W is still
        // queued: only the content order keeps the restore behind it. Without this,
        // the runner's insertion order alone would land W first.
        server
            .state
            .lock()
            .unwrap()
            .fail_first_chunk_once
            .insert("session-1".into());
        seed_uploaded_row(&bridge, &server, "ordered");
        let w = fp_save(&bridge, dir.path(), "ordered", "notes.txt", b"W", "1");
        bridge
            .queue_restore_version("ordered", "object-ordered-v1", None)
            .unwrap();
        assert_eq!(
            held_content_version(&bridge, "ordered"),
            w.token.clone().unwrap(),
            "kept while W is queued"
        );
        drain_upload_queue(&bridge, &sync_root).await;
        let state = server.finish();
        assert_eq!(
            state.files["ordered"].versions.len(),
            3,
            "v1, W as v2, the restore as v3"
        );
        let complete_at = state
            .requests
            .iter()
            .position(|(m, p)| m == "POST" && p.ends_with("/complete"))
            .unwrap();
        let restore_at = state
            .requests
            .iter()
            .position(|(m, p)| m == "POST" && p.ends_with("/restore"))
            .unwrap();
        assert!(complete_at < restore_at, "W lands first: {:?}", state.requests);
        assert_eq!(
            state.restores,
            vec![("ordered".to_string(), "object-ordered-v1".to_string())]
        );
        assert_eq!(held_content_version(&bridge, "ordered"), "3");
    }

    /// Review Minor 3: the server's restore is not idempotent (each call mints a
    /// version, VER:534-535). A local failure after it must not send it again.
    #[cfg(target_os = "macos")] // tests the restore response, which only macOS applies
    #[tokio::test]
    async fn a_restore_the_server_made_is_never_sent_again_after_a_local_failure() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [65u8; 32];
        let server = VersionedServerMock::start();
        let db_path = dir.path().join("state.db");
        let bridge = test_bridge_with_api(&db_path, server.base_url.clone(), master_key);
        seed_uploaded_row(&bridge, &server, "restore-once");
        bridge
            .queue_restore_version("restore-once", "object-restore-once-v1", None)
            .unwrap();
        assert_eq!(
            bridge.db.peek_resnapshot_request().unwrap(),
            None,
            "no snapshot is pending before"
        );
        // The server restores; then the local record fails: the change log is gone.
        let path = db_path.clone();
        bridge.seams.arm("restore:after_server", move || {
            rusqlite::Connection::open(&path)
                .unwrap()
                .execute("ALTER TABLE fp_changes RENAME TO fp_changes_away", [])
                .unwrap();
        });
        let logs = capture_logs_async(async {
            drain_upload_queue(&bridge, &sync_root).await;
        })
        .await;
        let state = server.finish();
        assert_eq!(
            state.restores,
            vec![("restore-once".to_string(), "object-restore-once-v1".to_string())],
            "the restore is sent once"
        );
        assert!(
            bridge.db.list_due_operations(i64::MAX).unwrap().is_empty(),
            "the op is done, not retried"
        );
        assert!(
            bridge.db.peek_resnapshot_request().unwrap().is_some(),
            "a snapshot repairs what the reply could not record"
        );
        let lines: Vec<&str> = logs
            .lines()
            .filter(|line| line.contains("restore_bookkeeping_failed"))
            .collect();
        assert_eq!(lines.len(), 1, "one line for the failed record:\n{logs}");
        assert!(lines[0].contains("WARN") && lines[0].contains("restore-once"), "{logs}");
        assert!(
            !logs.contains("fp_changes"),
            "the raw error never reaches the log:\n{logs}"
        );
    }

    #[cfg(unix)] // names `arm_builder_seam`
    #[tokio::test]
    async fn a_builder_clears_held_columns_only_for_the_write_it_read() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [62u8; 32];
        let server = VersionedServerMock::start();
        let bridge = Arc::new(test_bridge_with_api(
            &dir.path().join("state.db"),
            server.base_url.clone(),
            master_key,
        ));
        land_one_save(&bridge, &server, dir.path(), &sync_root, "guarded").await;
        remote_update(&bridge, &sync_root, "guarded", 3, "object-elsewhere-v3");
        // Between the builder's read (W, no longer current) and its clear, a save sets held = N.
        let saver = Arc::clone(&bridge);
        let save_dir = dir.path().to_path_buf();
        let n_token = Arc::new(Mutex::new(None));
        let n_slot = Arc::clone(&n_token);
        crate::ipc_socket::arm_builder_seam(move || {
            let n = fp_save(&saver, &save_dir, "guarded", "notes.txt", b"N", "3");
            *n_slot.lock().unwrap() = n.token;
        });
        let _ = held_content_version(&bridge, "guarded");
        let n = n_token
            .lock()
            .unwrap()
            .clone()
            .expect("the seam fired and N was accepted");
        let held = bridge.db.item_presentation("guarded").unwrap().unwrap().held;
        assert_eq!(
            held.map(|held| held.token()),
            Some(n),
            "the clear matched no row; N's token is kept"
        );
        drop(server.finish());
    }

    #[cfg(unix)] // names `arm_builder_seam`
    #[tokio::test]
    async fn the_predicate_reads_row_and_queue_together() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [63u8; 32];
        let server = VersionedServerMock::start();
        let bridge = Arc::new(test_bridge_with_api(
            &dir.path().join("state.db"),
            server.base_url.clone(),
            master_key,
        ));
        seed_uploaded_row(&bridge, &server, "one-read");
        let w = fp_save(&bridge, dir.path(), "one-read", "notes.txt", b"W", "1")
            .token
            .unwrap();
        let runner = Arc::clone(&bridge);
        let root = sync_root.clone();
        crate::ipc_socket::arm_builder_seam(move || {
            run_competing(move || {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap()
                    .block_on(async {
                        runner.process_due_operations(&root, now_secs()).await.unwrap();
                    });
            });
        });
        assert_eq!(
            held_content_version(&bridge, "one-read"),
            w,
            "never the old {{cv}} while W lands"
        );
        let state = server.finish();
        assert_eq!(
            state.init_summary(),
            vec![(json!("one-read"), json!(1), 201)],
            "W landed inside the seam"
        );
    }

    #[cfg(unix)] // asserts a token through `held_content_version`
    #[tokio::test]
    async fn twenty_rapid_saves_land_in_order_and_the_last_wins() {
        // Review Focus 1
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [64u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        server
            .state
            .lock()
            .unwrap()
            .delay_chunks
            .insert("session-1".into(), Duration::from_millis(400));
        seed_uploaded_row(&bridge, &server, "autosave");
        let first = fp_save(&bridge, dir.path(), "autosave", "notes.txt", b"save 1", "1");
        let mut base = first.token.unwrap();
        let mut last = b"save 1".to_vec();
        let ((), ()) = tokio::join!(
            async {
                drain_upload_queue(&bridge, &sync_root).await;
            },
            async {
                tokio::time::timeout(Duration::from_secs(10), async {
                    while server.state.lock().unwrap().inits.is_empty() {
                        tokio::time::sleep(Duration::from_millis(5)).await;
                    }
                })
                .await
                .expect("init never reached the mock");
                for n in 2..=20 {
                    let bytes = format!("save {n}").into_bytes();
                    let reply = fp_save(&bridge, dir.path(), "autosave", "notes.txt", &bytes, &base);
                    base = reply.token.unwrap();
                    assert_eq!(
                        held_content_version(&bridge, "autosave"),
                        base,
                        "save {n}: one name for its bytes"
                    );
                    last = bytes;
                }
            }
        );
        drain_upload_queue(&bridge, &sync_root).await;
        let state = server.finish();
        let bases: Vec<i64> = state
            .inits
            .iter()
            .map(|(body, status)| {
                assert_eq!(*status, 201, "{:?}", state.init_summary());
                body["base_version_number"].as_i64().unwrap()
            })
            .collect();
        assert_eq!(bases, (1..=20).collect::<Vec<i64>>(), "one upload per save, in order");
        assert_eq!(
            state.latest_plaintext("autosave", master_key),
            last,
            "the last save wins"
        );
        assert!(bridge.db.list_due_operations(i64::MAX).unwrap().is_empty());
        assert_eq!(
            held_content_version(&bridge, "autosave"),
            base,
            "the last save's name stays after it lands"
        );
    }

    /// Spec §8.4, the old folders: a save staged before the move names its copy by an absolute path in the old
    /// cache root (the op and the journal). It still uploads from that copy, and the copy is released and
    /// unlinked after the landing, with its journal row, as before.
    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn a_save_staged_in_the_old_cache_root_still_uploads_and_its_copy_is_released() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [91u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_uploaded_row(&bridge, &server, "old-root");
        fp_save(
            &bridge,
            dir.path(),
            "old-root",
            "notes.txt",
            b"saved before the move",
            "1",
        );
        let ops = bridge.db.list_due_operations(i64::MAX).unwrap();
        assert_eq!(ops.len(), 1);
        let staged = PathBuf::from(ops[0].payload_path.clone().expect("a save carries its copy"));
        let bases = FinderStagingBases::current();
        assert!(
            staged.starts_with(bases.durable_root().unwrap()),
            "a new save stages in the data root: {staged:?}"
        );
        // Where a build before the move staged it: the old cache root.
        let old_root = bases.cache_root();
        std::fs::create_dir_all(&old_root).unwrap();
        let old_copy = old_root.join(uuid::Uuid::new_v4().to_string());
        std::fs::rename(&staged, &old_copy).unwrap();
        {
            let conn = bridge.db.hold_lock_for_test();
            let (old, new) = (
                old_copy.to_string_lossy().into_owned(),
                staged.to_string_lossy().into_owned(),
            );
            for sql in [
                "UPDATE operation_queue SET payload_path = ?1 WHERE payload_path = ?2",
                "UPDATE staged_payloads SET path = ?1 WHERE path = ?2",
            ] {
                assert_eq!(conn.execute(sql, rusqlite::params![old, new]).unwrap(), 1, "{sql}");
            }
        }

        drain_upload_queue(&bridge, &sync_root).await;

        let state = server.finish();
        assert_eq!(
            state.init_summary(),
            vec![(json!("old-root"), json!(1), 201)],
            "the save uploads once, on its base"
        );
        assert_eq!(
            state.latest_plaintext("old-root", master_key),
            b"saved before the move",
            "from the copy in the old root"
        );
        assert!(bridge.db.list_due_operations(i64::MAX).unwrap().is_empty());
        assert!(
            !old_copy.exists(),
            "the copy in the old root is unlinked after the landing"
        );
        let journalled: i64 = bridge
            .db
            .hold_lock_for_test()
            .query_row(
                "SELECT COUNT(*) FROM staged_payloads WHERE path = ?1",
                [old_copy.to_string_lossy()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(journalled, 0, "and its journal row is gone");
    }

    // ── Rule 2, the snapshot side: version 0 and versionless replaces (spec §6.3.2–§6.3.4) ──

    /// A row this Mac learned from a legacy `file_create` op: at version 0 (FILES:2286-2297).
    /// (Both helpers carry `#[cfg(target_os = "macos")]`: only the Task 6 tests call them.)
    #[cfg(target_os = "macos")]
    fn seed_version_zero_row(bridge: &EngineBridge, server: &VersionedServerMock, file_id: &str, status: FileStatus) {
        seed_uploaded_row(bridge, server, file_id);
        let mut contract = bridge.db.get_file_contract_state(file_id).unwrap().unwrap();
        contract.current_version = 0;
        bridge.db.set_file_contract_state(&contract).unwrap();
        bridge.db.set_status(file_id, status).unwrap();
    }

    /// `name` is the file's display name: Task 11's log tests pass `PLANTED` here, so a line
    /// that logged the name would leak it. Every other caller passes `"notes.txt"`.
    #[cfg(target_os = "macos")]
    fn node(master_key: &[u8; 32], id: &str, name: &str, version: i64, is_uploading: bool) -> serde_json::Value {
        let mut node = snap_node(master_key, id, name, None, false, 0);
        node["version_number"] = serde_json::json!(version);
        node["is_uploading"] = serde_json::json!(is_uploading);
        node
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn i2_a_row_without_a_version_never_uploads_without_a_base() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [72u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_version_zero_row(&bridge, &server, "legacy", FileStatus::Local);
        bridge.db.set_sync_cursor(0).unwrap();
        fp_save(&bridge, dir.path(), "legacy", "notes.txt", b"edit", "0");
        drain_upload_queue(&bridge, &sync_root).await;
        assert!(
            server.state.lock().unwrap().inits.is_empty(),
            "no init until the version is known"
        );
        server.state.lock().unwrap().snapshots.push_back((
            "200 OK".into(),
            serde_json::json!({ "seq_id": 1, "nodes": [node(&master_key, "legacy", "notes.txt", 1, false)] }),
        ));
        let logs = capture_logs_async(async {
            sync_tick_outcome(&bridge, &sync_root).await.unwrap();
            drain_upload_queue(&bridge, &sync_root).await;
        })
        .await;
        let state = server.finish();
        assert_eq!(state.init_summary(), vec![(json!("legacy"), json!(1), 201)]);
        assert!(
            logs.contains("queued write based on the version the snapshot reported"),
            "{logs}"
        );
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn a_versionless_create_op_requests_a_snapshot_that_fills_local_rows() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [73u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        let create = crate::api_client::SyncOp {
            seq_id: 5,
            op_type: "file_create".into(),
            payload: serde_json::json!({
                "id": "phone-file",
                "name_encrypted": enc_name(&master_key, "phone-file", "notes.txt"),
                "parent_id": null,
                "size_bytes": 10
            }),
        };
        apply_sync_op(&bridge, &sync_root, &create, now_secs(), &mut Vec::new()).unwrap();
        assert!(
            bridge.db.peek_resnapshot_request().unwrap().is_some(),
            "a versionless op asks for a snapshot"
        );
        bridge.db.set_status("phone-file", FileStatus::Local).unwrap();
        let snapshot = crate::api_client::SyncSnapshot {
            seq_id: 6,
            nodes: vec![node(&master_key, "phone-file", "notes.txt", 1, false)],
        };
        apply_snapshot(&bridge, &sync_root, &snapshot, now_secs(), now_secs(), &mut Vec::new()).unwrap();
        let presentation = bridge.db.item_presentation("phone-file").unwrap().unwrap();
        assert_eq!(
            presentation.contract.current_version, 1,
            "filled although the row is Local"
        );
        assert!(presentation.version_filled);
        assert_eq!(
            bridge.db.get_file("phone-file").unwrap().unwrap().status,
            FileStatus::Local,
            "content untouched"
        );
        drop(server.finish());
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn i2_a_zero_base_save_after_the_fill_lands_on_the_filled_version() {
        // Rule 6a.
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [74u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_version_zero_row(&bridge, &server, "filled", FileStatus::Local);
        let snapshot = crate::api_client::SyncSnapshot {
            seq_id: 2,
            nodes: vec![node(&master_key, "filled", "notes.txt", 1, false)],
        };
        apply_snapshot(&bridge, &sync_root, &snapshot, now_secs(), now_secs(), &mut Vec::new()).unwrap();
        fp_save(
            &bridge,
            dir.path(),
            "filled",
            "notes.txt",
            b"saved before the re-read",
            "0",
        );
        drain_upload_queue(&bridge, &sync_root).await;
        let state = server.finish();
        assert_eq!(state.init_summary(), vec![(json!("filled"), json!(1), 201)]);
        assert!(
            bridge.db.list_operations_for_file("filled").unwrap().is_empty(),
            "nothing parks"
        );
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn i2_a_failed_snapshot_keeps_the_request_and_base_pending_resolves_on_the_next_success() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [75u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_version_zero_row(&bridge, &server, "flaky-snapshot", FileStatus::Local);
        bridge.db.set_sync_cursor(0).unwrap();
        fp_save(&bridge, dir.path(), "flaky-snapshot", "notes.txt", b"edit", "0");
        {
            let mut s = server.state.lock().unwrap();
            s.snapshots
                .push_back(("503 Service Unavailable".into(), serde_json::json!({ "error": "busy" })));
            s.snapshots.push_back((
                "200 OK".into(),
                serde_json::json!({
                    "seq_id": 3,
                    "nodes": [node(&master_key, "flaky-snapshot", "notes.txt", 1, false)]
                }),
            ));
        }
        // The runner runs the queue only after an Ok tick (RUN:1293-1315).
        assert!(
            sync_tick_outcome(&bridge, &sync_root).await.is_err(),
            "the failed snapshot fails the tick"
        );
        assert!(
            bridge.db.peek_resnapshot_request().unwrap().is_some(),
            "the request survives a failed snapshot"
        );
        sync_tick_outcome(&bridge, &sync_root).await.unwrap();
        drain_upload_queue(&bridge, &sync_root).await;
        let state = server.finish();
        assert_eq!(state.init_summary(), vec![(json!("flaky-snapshot"), json!(1), 201)]);
        assert_eq!(bridge.db.peek_resnapshot_request().unwrap(), None);
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn i2_the_fill_reaches_an_uploading_row() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [76u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_version_zero_row(&bridge, &server, "uploading-zero", FileStatus::Local);
        fp_save(&bridge, dir.path(), "uploading-zero", "notes.txt", b"edit", "0");
        assert_eq!(
            bridge.db.get_file("uploading-zero").unwrap().unwrap().status,
            FileStatus::Uploading
        );
        let snapshot = crate::api_client::SyncSnapshot {
            seq_id: 2,
            nodes: vec![node(&master_key, "uploading-zero", "notes.txt", 1, false)],
        };
        apply_snapshot(&bridge, &sync_root, &snapshot, now_secs(), now_secs(), &mut Vec::new()).unwrap();
        assert_eq!(
            bridge
                .db
                .get_file_contract_state("uploading-zero")
                .unwrap()
                .unwrap()
                .current_version,
            1
        );
        let op = bridge.db.list_operations_for_file("uploading-zero").unwrap().remove(0);
        assert_eq!(op.base_version, Some(1), "the op is based on the filled version");
        assert_eq!(bridge.db.finder_write(&op.op_id).unwrap().unwrap().base_pending, 0);
        drop(server.finish());
    }

    #[cfg(target_os = "macos")]
    #[cfg(unix)] // asserts a token through `held_content_version` and `land_one_save`
    #[tokio::test]
    async fn i3_a_versionless_replace_from_another_device_reaches_the_disk() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [77u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        let token = land_one_save(&bridge, &server, dir.path(), &sync_root, "replaced").await;
        let replace = crate::api_client::SyncOp {
            seq_id: 9,
            op_type: "file_create".into(),
            payload: serde_json::json!({
                "id": "replaced",
                "name_encrypted": enc_name(&master_key, "replaced", "notes.txt"),
                "parent_id": null,
                "size_bytes": 12
            }),
        };
        apply_sync_op(&bridge, &sync_root, &replace, now_secs(), &mut Vec::new()).unwrap();
        let seen = bridge.db.peek_resnapshot_request().unwrap().expect("requested");
        assert_eq!(
            held_content_version(&bridge, "replaced"),
            token,
            "the op alone cannot tell a replace"
        );
        // The legacy init bumped the version before the bytes exist: skipped, the request stays.
        let mid = crate::api_client::SyncSnapshot {
            seq_id: 10,
            nodes: vec![node(&master_key, "replaced", "notes.txt", 3, true)],
        };
        apply_snapshot(&bridge, &sync_root, &mid, now_secs(), now_secs(), &mut Vec::new()).unwrap();
        assert_eq!(held_content_version(&bridge, "replaced"), token);
        assert!(
            !bridge.db.clear_resnapshot_request(seen).unwrap(),
            "the skip kept the request"
        );
        let done = crate::api_client::SyncSnapshot {
            seq_id: 11,
            nodes: vec![node(&master_key, "replaced", "notes.txt", 3, false)],
        };
        let logs = capture_logs_async(async {
            apply_snapshot(&bridge, &sync_root, &done, now_secs(), now_secs(), &mut Vec::new()).unwrap();
        })
        .await;
        assert_eq!(
            held_content_version(&bridge, "replaced"),
            "3",
            "the system re-downloads the phone's bytes"
        );
        assert!(logs.contains("file version raised from the snapshot"), "{logs}");
        drop(server.finish());
    }

    /// The phone-replace path of the device run (D8b): the node is newer than the row, so the
    /// `Local` row continues past the short-circuit into the metadata refresh after the raise.
    /// The raise's facts hold after it.
    #[cfg(target_os = "macos")]
    #[cfg(unix)] // asserts a token through `held_content_version` and `land_one_save`
    #[tokio::test]
    async fn i3_a_newer_versionless_replace_reaches_the_disk_through_the_metadata_refresh() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [82u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        let token = land_one_save(&bridge, &server, dir.path(), &sync_root, "replaced").await;
        let landed = bridge.db.get_file_contract_state("replaced").unwrap().unwrap();
        assert_eq!((landed.current_version, landed.local_base_version), (2, 2));
        assert!(landed.current_object_version_id.is_some());
        let replace = crate::api_client::SyncOp {
            seq_id: 9,
            op_type: "file_create".into(),
            payload: serde_json::json!({
                "id": "replaced",
                "name_encrypted": enc_name(&master_key, "replaced", "notes.txt"),
                "parent_id": null,
                "size_bytes": 12
            }),
        };
        apply_sync_op(&bridge, &sync_root, &replace, now_secs(), &mut Vec::new()).unwrap();
        assert_eq!(held_content_version(&bridge, "replaced"), token);
        let mut newer = node(&master_key, "replaced", "notes.txt", 3, false);
        newer["updated_at"] = serde_json::json!(now_secs() + 60);
        newer["size_bytes"] = serde_json::json!(12);
        let done = crate::api_client::SyncSnapshot {
            seq_id: 11,
            nodes: vec![newer],
        };
        apply_snapshot(&bridge, &sync_root, &done, now_secs(), now_secs(), &mut Vec::new()).unwrap();
        assert_eq!(
            held_content_version(&bridge, "replaced"),
            "3",
            "the system re-downloads the phone's bytes"
        );
        let contract = bridge.db.get_file_contract_state("replaced").unwrap().unwrap();
        assert_eq!(contract.current_version, 3);
        assert_eq!(
            contract.local_base_version, 2,
            "the bytes on disk are still version 2's"
        );
        assert_eq!(
            contract.current_object_version_id, None,
            "the snapshot names no object version, and version 2's is not current"
        );
        assert!(
            !bridge.db.item_presentation("replaced").unwrap().unwrap().version_filled,
            "a raise is not a fill"
        );
        assert_eq!(bridge.db.get_file("replaced").unwrap().unwrap().size_bytes, 12);
        drop(server.finish());
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn base_pending_parks_after_ten_successful_snapshots_without_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [78u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_version_zero_row(&bridge, &server, "never-listed", FileStatus::Local);
        seed_uploaded_row(&bridge, &server, "listed");
        bridge.db.set_sync_cursor(0).unwrap();
        fp_save(&bridge, dir.path(), "never-listed", "notes.txt", b"edit", "0");
        let listed = serde_json::json!({ "seq_id": 1, "nodes": [node(&master_key, "listed", "notes.txt", 1, false)] });
        let op_id = bridge
            .db
            .list_operations_for_file("never-listed")
            .unwrap()
            .remove(0)
            .op_id;
        for success in 1..=10 {
            {
                let mut s = server.state.lock().unwrap();
                s.snapshots
                    .push_back(("503 Service Unavailable".into(), serde_json::json!({ "error": "busy" })));
                s.snapshots.push_back(("200 OK".into(), listed.clone()));
            }
            assert!(
                sync_tick_outcome(&bridge, &sync_root).await.is_err(),
                "a failed snapshot costs nothing"
            );
            sync_tick_outcome(&bridge, &sync_root).await.unwrap();
            let op = bridge.db.get_operation(&op_id).unwrap().unwrap();
            if success < 10 {
                assert!(
                    op.attempts < op.max_attempts,
                    "not parked after {success} successful snapshots"
                );
            } else {
                assert_eq!(op.attempts, op.max_attempts, "parked base_unknown after the 10th");
            }
        }
        let state = server.finish();
        assert!(state.inits.is_empty());
    }

    /// A write the snapshot count parked is parked like any other (spec §8.4): it asks for no
    /// more snapshots, and a save on its token takes its role at the claim and parks the
    /// same way, without a request on a base the client knows is unknown.
    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn a_save_after_a_snapshot_count_park_takes_over_and_parks_without_a_request() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [79u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_version_zero_row(&bridge, &server, "unlisted", FileStatus::Local);
        seed_uploaded_row(&bridge, &server, "listed");
        bridge.db.set_sync_cursor(0).unwrap();
        let w = fp_save(&bridge, dir.path(), "unlisted", "notes.txt", b"first edit", "0");
        let w_op = bridge.db.list_operations_for_file("unlisted").unwrap().remove(0).op_id;
        let listed = serde_json::json!({ "seq_id": 1, "nodes": [node(&master_key, "listed", "notes.txt", 1, false)] });
        for _ in 0..10 {
            server
                .state
                .lock()
                .unwrap()
                .snapshots
                .push_back(("200 OK".into(), listed.clone()));
            sync_tick_outcome(&bridge, &sync_root).await.unwrap();
        }
        let parked = bridge.db.get_operation(&w_op).unwrap().unwrap();
        assert_eq!(parked.attempts, parked.max_attempts, "parked after the 10th snapshot");
        assert_eq!(parked.last_error.as_deref(), Some("base_unknown"));
        // No snapshot is queued on the mock now: a tick that asked for one would fail. The
        // ops path, with nothing to apply, succeeds.
        assert!(
            sync_tick_outcome(&bridge, &sync_root).await.is_ok(),
            "a parked write asks for no more snapshots"
        );
        assert_eq!(bridge.db.peek_resnapshot_request().unwrap(), None);

        let n = fp_save(
            &bridge,
            dir.path(),
            "unlisted",
            "notes.txt",
            b"first edit, second edit",
            &w.token.unwrap(),
        );
        assert!(n.token.is_some());
        drain_upload_queue(&bridge, &sync_root).await;
        let state = server.finish();
        assert!(
            state.init_summary().is_empty(),
            "no request with a base the client knows is unknown: {:?}",
            state.init_summary()
        );
        let ops = bridge.db.list_operations_for_file("unlisted").unwrap();
        assert_eq!(ops.len(), 1, "the save took the parked write's role: {ops:?}");
        assert_ne!(ops[0].op_id, w_op);
        assert_eq!(ops[0].attempts, ops[0].max_attempts, "and parked the same way");
        assert_eq!(ops[0].last_error.as_deref(), Some("base_unknown"));
    }

    /// §6.1: `version_filled` is cleared by a landing, by a content op on the row and (plan
    /// Spec issue 16) by an accepted Finder write.
    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn version_filled_is_cleared_by_a_landing_a_content_op_and_an_accepted_write() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [80u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        let filled = |file_id: &str| bridge.db.item_presentation(file_id).unwrap().unwrap().version_filled;
        let fill = |file_id: &str| {
            let snapshot = crate::api_client::SyncSnapshot {
                seq_id: 2,
                nodes: vec![node(&master_key, file_id, "notes.txt", 1, false)],
            };
            apply_snapshot(&bridge, &sync_root, &snapshot, now_secs(), now_secs(), &mut Vec::new()).unwrap();
        };

        // A landing: the fill reaches the waiting write's row, then the write lands.
        seed_version_zero_row(&bridge, &server, "landing", FileStatus::Local);
        fp_save(&bridge, dir.path(), "landing", "notes.txt", b"edit", "0");
        fill("landing");
        assert!(filled("landing"));
        drain_upload_queue(&bridge, &sync_root).await;
        assert!(!filled("landing"), "a landing clears it");

        // A content op from another device.
        seed_version_zero_row(&bridge, &server, "content-op", FileStatus::Local);
        fill("content-op");
        assert!(filled("content-op"));
        let update = crate::api_client::SyncOp {
            seq_id: 3,
            op_type: "file_update".into(),
            payload: serde_json::json!({ "id": "content-op", "version_number": 2, "size_bytes": 11 }),
        };
        apply_sync_op(&bridge, &sync_root, &update, now_secs(), &mut Vec::new()).unwrap();
        assert!(!filled("content-op"), "a content op clears it");

        // An accepted Finder write, before it lands.
        seed_version_zero_row(&bridge, &server, "accepted", FileStatus::Local);
        fill("accepted");
        assert!(filled("accepted"));
        fp_save(&bridge, dir.path(), "accepted", "notes.txt", b"edit", "1");
        assert!(!filled("accepted"), "an accepted write clears it");
        drop(server.finish());
    }

    /// §6.1 / §6.3.2: only a content op asks for a snapshot or clears `version_filled`. The
    /// server's thumbnail-flag ops (`{id, has_thumbnail}`) carry neither `version_number` nor
    /// `size_bytes`, and follow nearly every image upload; a versionless op that does carry
    /// content still does both.
    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn a_thumbnail_flag_op_is_not_a_content_op() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [81u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_version_zero_row(&bridge, &server, "pictured", FileStatus::Local);
        let snapshot = crate::api_client::SyncSnapshot {
            seq_id: 2,
            nodes: vec![node(&master_key, "pictured", "notes.txt", 1, false)],
        };
        apply_snapshot(&bridge, &sync_root, &snapshot, now_secs(), now_secs(), &mut Vec::new()).unwrap();
        assert_eq!(bridge.db.peek_resnapshot_request().unwrap(), None);
        let filled = || bridge.db.item_presentation("pictured").unwrap().unwrap().version_filled;
        assert!(filled());

        for (seq_id, flag) in [(3, "has_thumbnail"), (4, "has_large_thumbnail")] {
            let thumbnail = crate::api_client::SyncOp {
                seq_id,
                op_type: "file_update".into(),
                payload: serde_json::json!({ "id": "pictured", flag: true }),
            };
            apply_sync_op(&bridge, &sync_root, &thumbnail, now_secs(), &mut Vec::new()).unwrap();
            assert!(filled(), "a {flag} op is not a content op: the fill stands");
            assert_eq!(
                bridge.db.peek_resnapshot_request().unwrap(),
                None,
                "a {flag} op asks for no snapshot"
            );
        }

        // A versionless op that carries content (a legacy replace's size) still does both.
        let content = crate::api_client::SyncOp {
            seq_id: 5,
            op_type: "file_update".into(),
            payload: serde_json::json!({ "id": "pictured", "size_bytes": 12 }),
        };
        apply_sync_op(&bridge, &sync_root, &content, now_secs(), &mut Vec::new()).unwrap();
        assert!(!filled(), "a versionless content op clears the fill");
        assert!(
            bridge.db.peek_resnapshot_request().unwrap().is_some(),
            "a versionless content op asks for a snapshot"
        );
        drop(server.finish());
    }

    // ── M1: the landing is one transaction after a recorded completion (spec §8.6) ──

    #[tokio::test]
    async fn m1_a_chain_failure_after_complete_never_duplicates() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [79u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        let created = fp_create(&bridge, dir.path(), "new.txt", b"created");
        let provisional = created.outcome_file_id();
        fp_save(
            &bridge,
            dir.path(),
            &provisional,
            "new.txt",
            b"created, edited",
            created.token.as_deref().unwrap(),
        );
        // The successor's name cannot be re-encrypted for the server id: no display name, no path.
        let successor = bridge.db.list_operations_for_file(&provisional).unwrap().remove(1);
        bridge.db.set_target_path_for_test(&successor.op_id, None);
        let logs = capture_logs_async(async {
            drain_upload_queue(&bridge, &sync_root).await;
        })
        .await;
        let state = server.finish();
        assert_eq!(
            state.files_with_content().len(),
            1,
            "one server file: {:?}",
            state.init_summary()
        );
        assert_eq!(state.inits.len(), 1, "no second init");
        let server_id = state.files_with_content()[0].clone();
        assert!(
            bridge.db.get_file(&provisional).unwrap().is_none(),
            "the landing was applied"
        );
        assert!(bridge.db.get_file(&server_id).unwrap().is_some());
        let parked = bridge.db.get_operation(&successor.op_id).unwrap().unwrap();
        assert_eq!(
            parked.attempts, parked.max_attempts,
            "the successor parked with its bytes"
        );
        assert!(std::path::Path::new(parked.payload_path.as_deref().unwrap()).is_file());
        assert!(logs.contains("rekey_failed"), "{logs}");
    }

    // T30 (`i1_a_landing_with_a_later_write_queued_keeps_it_uploading`) moved to Task 9:
    // it needs Task 9 step 3.4, because until then the `Rollback` guard sets `Error` on
    // any failed attempt.

    #[cfg(unix)] // asserts tokens through `held_content_version`
    #[tokio::test]
    async fn landing_vs_new_save_the_chain_step_sees_every_successor() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [81u8; 32];
        let server = VersionedServerMock::start();
        let bridge = Arc::new(test_bridge_with_api(
            &dir.path().join("state.db"),
            server.base_url.clone(),
            master_key,
        ));
        server
            .state
            .lock()
            .unwrap()
            .delay_complete
            .insert("session-1".into(), Duration::from_millis(400));
        seed_uploaded_row(&bridge, &server, "landing-race");
        let w = fp_save(&bridge, dir.path(), "landing-race", "notes.txt", b"W", "1")
            .token
            .unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let (saver, save_dir, w_token, seen_at_seam) = (
            Arc::clone(&bridge),
            dir.path().to_path_buf(),
            w.clone(),
            Arc::clone(&seen),
        );
        bridge.seams.arm("landing:after_complete", move || {
            run_competing(move || {
                seen_at_seam
                    .lock()
                    .unwrap()
                    .push(held_content_version(&saver, "landing-race"));
                fp_save(&saver, &save_dir, "landing-race", "notes.txt", b"W N", &w_token);
                seen_at_seam
                    .lock()
                    .unwrap()
                    .push(held_content_version(&saver, "landing-race"));
            });
        });
        drain_upload_queue(&bridge, &sync_root).await;
        let state = server.finish();
        assert_eq!(
            state.init_summary(),
            vec![
                (json!("landing-race"), json!(1), 201),
                (json!("landing-race"), json!(2), 201)
            ],
            "N lands on W's produced version"
        );
        let seen = seen.lock().unwrap().clone();
        assert_eq!(seen[0], w, "at the seam: W's token");
        let n = held_content_version(&bridge, "landing-race");
        assert_eq!(seen[1], n, "after N's accept: N's token, kept through the landing");
        assert!(
            seen.iter()
                .chain(std::iter::once(&n))
                .all(|v| crate::write_token::parse_token(v).is_some()),
            "never a numeric content version: {seen:?}"
        );
    }

    #[tokio::test]
    async fn a_completed_session_is_never_abandoned_at_give_up() {
        // m-11
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [82u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        let created = fp_create(&bridge, dir.path(), "completed.txt", b"bytes the server has");
        let provisional = created.outcome_file_id();
        crate::state_db::fail_next_landings_for_test(30);
        let mut now = now_secs();
        let logs = capture_logs_async(async {
            for _ in 0..26 {
                bridge.process_due_operations(&sync_root, now).await.unwrap();
                now += 10_000;
            }
        })
        .await;
        // Never trashed first: with the attempts cap gone, the give-up guard alone keeps it.
        assert!(
            server.state.lock().unwrap().trashes.is_empty(),
            "the completed server file is never trashed"
        );
        assert_eq!(server.state.lock().unwrap().inits.len(), 1, "no second upload");
        let op = bridge.db.list_operations_for_file(&provisional).unwrap().remove(0);
        assert!(op.attempts < op.max_attempts, "never parked");
        assert!(
            bridge
                .db
                .get_upload_resume(&op.op_id)
                .unwrap()
                .unwrap()
                .completed_version
                .is_some(),
            "the resume row is kept"
        );
        assert!(logs.matches("local landing will be retried").count() >= 25, "{logs}");
        crate::state_db::fail_next_landings_for_test(0);
        bridge.process_due_operations(&sync_root, now).await.unwrap();
        assert!(
            bridge.db.get_file(&provisional).unwrap().is_none(),
            "landed once the disk recovers"
        );
        drop(server.finish());
    }

    #[tokio::test]
    async fn a_recorded_completion_is_applied_without_the_network() {
        // §8.6 rule 4
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [83u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_uploaded_row(&bridge, &server, "recorded");
        fp_save(&bridge, dir.path(), "recorded", "notes.txt", b"edit", "1");
        crate::state_db::fail_next_landings_for_test(1);
        let now = now_secs();
        bridge.process_due_operations(&sync_root, now).await.unwrap();
        let before = server.state.lock().unwrap().requests.len();
        bridge.process_due_operations(&sync_root, now + 10_000).await.unwrap();
        let state = server.finish();
        assert_eq!(
            state.requests.len(),
            before,
            "the retry asked the server nothing: {:?}",
            &state.requests[before..]
        );
        assert!(
            bridge.db.list_operations_for_file("recorded").unwrap().is_empty(),
            "landed"
        );
        let contract = bridge.db.get_file_contract_state("recorded").unwrap().unwrap();
        assert_eq!(contract.current_version, 2);
        assert_eq!(
            contract.content_type.as_deref(),
            Some("text/plain"),
            "the retried landing keeps the save's content type"
        );
    }

    #[cfg(unix)] // asserts a token through `held_content_version`
    #[tokio::test]
    async fn the_create_landing_writes_the_alias_in_its_transaction() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [84u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        let created = fp_create(&bridge, dir.path(), "aliased.txt", b"created");
        let provisional = created.outcome_file_id();
        drain_upload_queue(&bridge, &sync_root).await;
        let state = server.finish();
        let server_id = state.files_with_content()[0].clone();
        assert_eq!(bridge.db.alias_target_for_test(&provisional), Some(server_id.clone()));
        assert!(bridge.db.get_file(&provisional).unwrap().is_none());
        assert_eq!(
            held_content_version(&bridge, &server_id),
            created.token.unwrap(),
            "S carries P's create token (§5.4 row 14)"
        );
    }

    /// [t3-review] Minor 1: a save N is accepted on W's token after W's upload landed and
    /// before the runner records W's outcome (`outcome:before_tx`, after the thumbnail
    /// work). The landing transaction has already removed W's op, so N decides against
    /// the landed W (rule 1b) and never waits on a write that no op carries.
    #[tokio::test]
    async fn a_save_accepted_before_the_landed_outcome_is_recorded_lands_on_the_produced_version() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [85u8; 32];
        let server = VersionedServerMock::start();
        let bridge = Arc::new(test_bridge_with_api(
            &dir.path().join("state.db"),
            server.base_url.clone(),
            master_key,
        ));
        seed_uploaded_row(&bridge, &server, "window");
        let w = fp_save(&bridge, dir.path(), "window", "notes.txt", b"W", "1")
            .token
            .unwrap();
        let (saver, save_dir) = (Arc::clone(&bridge), dir.path().to_path_buf());
        let fired = Arc::new(AtomicBool::new(false));
        let fired_in_seam = Arc::clone(&fired);
        bridge.seams.arm("outcome:before_tx", move || {
            run_competing(move || {
                fp_save(&saver, &save_dir, "window", "notes.txt", b"W N", &w);
            });
            fired_in_seam.store(true, Ordering::SeqCst);
        });
        let logs = capture_logs_async(async {
            drain_upload_queue(&bridge, &sync_root).await;
        })
        .await;
        let state = server.finish();
        assert!(fired.load(Ordering::SeqCst), "the seam fired and N was accepted");
        assert_eq!(
            state.init_summary(),
            vec![(json!("window"), json!(1), 201), (json!("window"), json!(2), 201)],
            "N lands on W's produced version: {logs}"
        );
        assert_eq!(state.latest_plaintext("window", master_key), b"W N");
        assert!(!logs.contains("predecessor_lost"), "{logs}");
        assert!(
            bridge.db.list_operations_for_file("window").unwrap().is_empty(),
            "nothing parked"
        );
    }

    /// [t3-review] Minor 3: the next base is the version the server's reply produced. An
    /// idempotent repeat of `complete` answers with neither the version nor the object id
    /// (`already_completed`); the produced version is then W's base + 1 (§8.6.1), never the
    /// stored record + 1. Here a snapshot raised the stored record to W's own version while
    /// W's first reply was lost.
    #[tokio::test]
    async fn a_repeated_completion_bases_the_next_save_on_the_version_w_produced() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [86u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_uploaded_row(&bridge, &server, "reply");
        let w = fp_save(&bridge, dir.path(), "reply", "notes.txt", b"W", "1");
        fp_save(
            &bridge,
            dir.path(),
            "reply",
            "notes.txt",
            b"W N",
            w.token.as_deref().unwrap(),
        );
        server
            .state
            .lock()
            .unwrap()
            .complete_reply_lost_once
            .insert("session-1".into());
        bridge.process_due_operations(&sync_root, now_secs()).await.unwrap();
        assert_eq!(
            server.state.lock().unwrap().files["reply"].versions.len(),
            2,
            "precondition: W is version 2 on the server, and its reply was lost"
        );
        assert!(matches!(
            bridge.db.apply_snapshot_version("reply", 2, 1, false).unwrap(),
            crate::state_db::SnapshotVersion::Raised { old: 1, new: 2 }
        ));
        drain_upload_queue(&bridge, &sync_root).await;
        let state = server.finish();
        assert_eq!(
            state.init_summary(),
            vec![(json!("reply"), json!(1), 201), (json!("reply"), json!(2), 201)],
            "N is based on the version W produced"
        );
        assert_eq!(state.latest_plaintext("reply", master_key), b"W N");
        assert!(bridge.db.list_operations_for_file("reply").unwrap().is_empty());
        assert_eq!(
            bridge
                .db
                .get_file_contract_state("reply")
                .unwrap()
                .unwrap()
                .current_version,
            3
        );
    }

    /// What an upload left behind when a purge removed its op after the server completed
    /// it: during `complete` (`during_complete`), or at the landing.
    struct PurgedLanding {
        op_id: String,
        outcome: TransferLoopOutcome,
        logs: String,
        server_versions: usize,
        op_left: bool,
        row_left: bool,
        resume_left: bool,
    }

    async fn upload_purged_after_the_server_completed(during_complete: bool, master_key: [u8; 32]) -> PurgedLanding {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_uploaded_row(&bridge, &server, "purged");
        fp_save(&bridge, dir.path(), "purged", "notes.txt", b"a save in a share", "1");
        let op_id = bridge.db.list_operations_for_file("purged").unwrap().remove(0).op_id;
        let mut contract = bridge.db.get_file_contract_state("purged").unwrap().unwrap();
        contract.namespace = Namespace::SharedWithMe;
        contract.shared_root_id = Some("purged-root".into());
        bridge.db.set_file_contract_state(&contract).unwrap();
        let mut outcome = None;
        let logs = if during_complete {
            // The server completes at once and answers 400 ms later; the share goes away
            // while the reply is in flight.
            server
                .state
                .lock()
                .unwrap()
                .delay_complete
                .insert("session-1".into(), Duration::from_millis(400));
            capture_logs_async(async {
                let ((), ()) = tokio::join!(
                    async {
                        outcome = Some(bridge.process_due_operations(&sync_root, now_secs()).await.unwrap());
                    },
                    async {
                        tokio::time::timeout(Duration::from_secs(10), async {
                            while !server
                                .state
                                .lock()
                                .unwrap()
                                .requests
                                .iter()
                                .any(|(_, path)| path.ends_with("/complete"))
                            {
                                tokio::time::sleep(Duration::from_millis(5)).await;
                            }
                        })
                        .await
                        .expect("complete never reached the mock");
                        bridge.db.purge_revoked_shared_content(&[]).unwrap();
                    }
                );
            })
            .await
        } else {
            let db = bridge.db.clone();
            bridge.seams.arm("landing:after_complete", move || {
                run_competing(move || {
                    db.purge_revoked_shared_content(&[]).unwrap();
                })
            });
            capture_logs_async(async {
                outcome = Some(bridge.process_due_operations(&sync_root, now_secs()).await.unwrap());
            })
            .await
        };
        let state = server.finish();
        PurgedLanding {
            outcome: outcome.unwrap(),
            logs,
            server_versions: state.files["purged"].versions.len(),
            op_left: bridge.db.get_operation(&op_id).unwrap().is_some(),
            row_left: bridge.db.get_file("purged").unwrap().is_some(),
            resume_left: bridge.db.get_upload_resume(&op_id).unwrap().is_some(),
            op_id,
        }
    }

    fn assert_purged_landing_wrote_nothing(run: &PurgedLanding, step: &str) {
        assert_eq!(run.server_versions, 2, "precondition: the server completed the upload");
        assert!(!run.op_left, "a purged op is never re-inserted");
        assert!(!run.row_left, "the landing never brings back a purged row");
        assert!(!run.resume_left, "nor its resume row");
        assert!(
            run.outcome.completed_op_ids.is_empty()
                && run.outcome.retried_op_ids.is_empty()
                && run.outcome.paused_op_ids.is_empty(),
            "an attempt whose op moved is not reported: {:?}",
            run.outcome.completed_op_ids
        );
        let moved: Vec<&str> = run
            .logs
            .lines()
            .filter(|line| line.contains("queue state moved"))
            .collect();
        assert_eq!(moved.len(), 1, "one line for the purged attempt:\n{}", run.logs);
        assert!(
            moved[0].contains(&run.op_id) && moved[0].contains(&format!("step=\"{step}\"")),
            "{}",
            run.logs
        );
    }

    /// [t2-review] / [plan-fix-2] M16: the completion record is guarded by the claim (S4).
    #[tokio::test]
    async fn a_completion_for_an_op_purged_under_its_claim_writes_nothing() {
        let run = upload_purged_after_the_server_completed(true, [87u8; 32]).await;
        assert_purged_landing_wrote_nothing(&run, "completion");
    }

    /// [t3-review] the landing moved-op test: the landing transaction is guarded by the
    /// claim (S2, S4) and writes nothing for an op that moved while its upload landed.
    #[tokio::test]
    async fn a_landing_for_an_op_purged_under_its_claim_writes_nothing() {
        let run = upload_purged_after_the_server_completed(false, [88u8; 32]).await;
        assert_purged_landing_wrote_nothing(&run, "landing");
    }

    /// §8.6 rules 4 and 6: a recorded completion lands before anything else is checked, so a
    /// staged copy that is gone by the retry never parks it.
    #[tokio::test]
    async fn a_recorded_completion_lands_even_when_its_staged_copy_is_gone() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [89u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_uploaded_row(&bridge, &server, "copy-gone");
        fp_save(&bridge, dir.path(), "copy-gone", "notes.txt", b"edit", "1");
        let op = bridge.db.list_operations_for_file("copy-gone").unwrap().remove(0);
        crate::state_db::fail_next_landings_for_test(1);
        let now = now_secs();
        bridge.process_due_operations(&sync_root, now).await.unwrap();
        assert!(
            bridge
                .db
                .get_upload_resume(&op.op_id)
                .unwrap()
                .is_some_and(|resume| resume.completed_version == Some(2)),
            "precondition: the completion is recorded"
        );
        std::fs::remove_file(op.payload_path.as_deref().unwrap()).unwrap();
        let logs = capture_logs_async(async {
            bridge.process_due_operations(&sync_root, now + 10_000).await.unwrap();
        })
        .await;
        let state = server.finish();
        assert!(
            bridge.db.list_operations_for_file("copy-gone").unwrap().is_empty(),
            "landed: {logs}"
        );
        assert!(!logs.contains("upload parked"), "{logs}");
        assert_eq!(state.inits.len(), 1);
        assert_eq!(
            bridge
                .db
                .get_file_contract_state("copy-gone")
                .unwrap()
                .unwrap()
                .current_version,
            2
        );
    }

    /// Review Minor 3: a save with no content type of its own lands with the server's
    /// `mime_type`, as its first landing would, also when that landing is retried from the
    /// recorded completion.
    #[tokio::test]
    async fn a_retried_landing_keeps_the_servers_mime_type() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [92u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_uploaded_row(&bridge, &server, "untyped");
        let contents = dir.path().join("save-untyped");
        std::fs::write(&contents, b"edit").unwrap();
        let mut target = finder_file_target(Some("untyped"), "notes", &contents, Some("1".into()));
        target.content_type = None;
        bridge.queue_file_provider_modify(target).unwrap();
        crate::state_db::fail_next_landings_for_test(1);
        let now = now_secs();
        bridge.process_due_operations(&sync_root, now).await.unwrap();
        bridge.process_due_operations(&sync_root, now + 10_000).await.unwrap();
        drop(server.finish());
        assert!(
            bridge.db.list_operations_for_file("untyped").unwrap().is_empty(),
            "landed on the retry"
        );
        assert_eq!(
            bridge
                .db
                .get_file_contract_state("untyped")
                .unwrap()
                .unwrap()
                .content_type
                .as_deref(),
            Some("text/plain"),
            "the server's mime_type: the save has no content type of its own"
        );
    }

    /// m-11: a landing that fails with an error `classify_operation_error` reads as a pause
    /// (SQLite's "database is locked" is `Locked`) is retried, never paused: a paused op is
    /// not listed again.
    #[tokio::test]
    async fn a_recorded_completion_is_never_paused() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [90u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_uploaded_row(&bridge, &server, "busy");
        fp_save(&bridge, dir.path(), "busy", "notes.txt", b"edit", "1");
        crate::state_db::fail_next_landings_as_locked_for_test(1);
        let now = now_secs();
        let outcome = bridge.process_due_operations(&sync_root, now).await.unwrap();
        assert!(outcome.paused_op_ids.is_empty(), "never paused: {outcome:?}");
        let op = bridge.db.list_operations_for_file("busy").unwrap().remove(0);
        assert_eq!(op.attempts, 1, "retried on the normal backoff");
        bridge.process_due_operations(&sync_root, now + 10_000).await.unwrap();
        let state = server.finish();
        assert!(
            bridge.db.list_operations_for_file("busy").unwrap().is_empty(),
            "landed on the retry"
        );
        assert_eq!(state.inits.len(), 1);
    }

    // ── Rule 3: provisional ids, fetches from the queue, thumbnails, unknown ids (spec §7, §5.6) ──

    /// A temporary directory with a sync root, a versioned server and a bridge on `master_key`.
    fn rule3_setup(master_key: [u8; 32]) -> (tempfile::TempDir, PathBuf, VersionedServerMock, EngineBridge) {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        (dir, sync_root, server, bridge)
    }

    /// A create landed: returns (P, S, the create's token).
    async fn landed_create(
        bridge: &EngineBridge,
        server: &VersionedServerMock,
        dir: &Path,
        sync_root: &Path,
        name: &str,
    ) -> (String, String, String) {
        let created = fp_create(bridge, dir, name, b"created bytes");
        let provisional = created.outcome_file_id();
        drain_upload_queue(bridge, sync_root).await;
        let server_id = server.state.lock().unwrap().files_with_content()[0].clone();
        assert_ne!(provisional, server_id, "the landing swapped the id");
        (provisional, server_id, created.token.unwrap())
    }

    /// The encrypted thumbnail blob the server would hold for `file_id`.
    fn thumbnail_blob(master_key: [u8; 32], file_id: &str, plaintext: &[u8]) -> Vec<u8> {
        beebeeb_core::encrypt::encrypt_chunk_raw(
            &beebeeb_core::kdf::derive_file_key(
                &beebeeb_core::kdf::MasterKey::from_bytes(master_key),
                file_id.as_bytes(),
            ),
            plaintext,
        )
        .unwrap()
    }

    /// T15 (I3): a save under P after its create landed modifies S, replied under P.
    #[cfg(unix)] // names `crate::ipc_socket::write_outcome_response`
    #[tokio::test]
    async fn i3_create_lands_then_a_modify_under_the_provisional_id() {
        let master_key = [85u8; 32];
        let (dir, sync_root, server, bridge) = rule3_setup(master_key);
        let (p, s, create_token) = landed_create(&bridge, &server, dir.path(), &sync_root, "t.txt").await;
        let reply = fp_save(
            &bridge,
            dir.path(),
            &p,
            "t.txt",
            b"created bytes, edited",
            &create_token,
        );
        assert_eq!(reply.present_as.as_deref(), Some(p.as_str()));
        let ipc = crate::ipc_socket::write_outcome_response("modify", &bridge.db, Ok(reply.clone()));
        let crate::ipc_socket::IpcResponse::WriteQueued { item: Some(item), .. } = ipc else {
            panic!("an item, never None: {ipc:?}")
        };
        assert_eq!(
            item.identifier, p,
            "presented under P until the system applies the swap"
        );
        assert_eq!(item.content_version, reply.token, "the new token");
        drain_upload_queue(&bridge, &sync_root).await;
        let state = server.finish();
        assert_eq!(state.files_with_content(), vec![s.clone()], "one server file");
        assert_eq!(state.files[&s].versions.len(), 2);
        assert_eq!(state.latest_plaintext(&s, master_key), b"created bytes, edited");
    }

    /// T16: a delete under P naming P's create token trashes S.
    #[tokio::test]
    async fn a_delete_of_the_provisional_id_after_landing_trashes_the_server_file() {
        let (dir, sync_root, server, bridge) = rule3_setup([86u8; 32]);
        let (p, s, create_token) = landed_create(&bridge, &server, dir.path(), &sync_root, "gone.txt").await;
        bridge.queue_finder_delete(&p, Some(create_token)).unwrap();
        drain_upload_queue(&bridge, &sync_root).await;
        assert_eq!(server.finish().trashes, vec![s]);
    }

    /// T58 (m-6): a delete under P that does not name S's held token keeps S.
    #[tokio::test]
    async fn a_delete_through_the_alias_needs_the_held_token_as_base() {
        let (dir, sync_root, server, bridge) = rule3_setup([87u8; 32]);
        let (p, s, _create_token) = landed_create(&bridge, &server, dir.path(), &sync_root, "kept.txt").await;
        let logs = capture_logs_async(async {
            bridge.queue_finder_delete(&p, Some("1".into())).unwrap();
            drain_upload_queue(&bridge, &sync_root).await;
        })
        .await;
        assert!(server.state.lock().unwrap().trashes.is_empty(), "S kept");
        assert!(
            logs.contains("delete of a provisional id not applied to the server file"),
            "{logs}"
        );
        assert!(bridge.db.get_file(&s).unwrap().is_some());
        drop(server.finish());
    }

    /// T17 (§7.3): a modify of an id with no row and no alias is refused and queues nothing.
    #[cfg(unix)] // names `crate::ipc_socket::write_outcome_response`
    #[tokio::test]
    async fn an_unknown_id_is_refused_never_answered_without_an_item() {
        let (dir, _sync_root, server, bridge) = rule3_setup([88u8; 32]);
        let contents = dir.path().join("orphan.txt");
        std::fs::write(&contents, b"edit of an item nobody knows").unwrap();
        let unknown = "3f2a9c1e-0000-4000-8000-00000000dead";
        let logs = capture_logs_async(async {
            let result = bridge.queue_file_provider_modify(finder_file_target(
                Some(unknown),
                "orphan.txt",
                &contents,
                Some("1".into()),
            ));
            let reply = crate::ipc_socket::write_outcome_response("modify", &bridge.db, result);
            assert!(
                matches!(reply, crate::ipc_socket::IpcResponse::Error { .. }),
                "{reply:?}"
            );
            // A rename or move of the same unknown id is refused the same way.
            let rename = bridge.queue_file_provider_modify(FinderWriteTarget {
                file_id: Some(unknown.into()),
                parent_id: None,
                filename: "orphan renamed.txt".into(),
                rel_path: None,
                kind: FinderWriteItemKind::File,
                contents_path: None,
                content_type: None,
                base_version_identifier: Some("1".into()),
            });
            let reply = crate::ipc_socket::write_outcome_response("modify", &bridge.db, rename);
            assert!(
                matches!(reply, crate::ipc_socket::IpcResponse::Error { .. }),
                "{reply:?}"
            );
        })
        .await;
        assert!(
            bridge.db.list_due_operations(i64::MAX).unwrap().is_empty(),
            "nothing queued"
        );
        assert!(
            logs.matches("Finder write refused").count() == 2 && logs.matches("unknown_item").count() == 2,
            "{logs}"
        );
        drop(server.finish());
    }

    /// T18 (R2a): a fetch of an item whose create is queued is served from the staged bytes.
    #[tokio::test]
    async fn r2a_a_fetch_during_the_creates_queue_wait_is_served_locally() {
        // The mock answers 404 to anything unexpected.
        let (dir, _sync_root, server, bridge) = rule3_setup([89u8; 32]);
        let created = fp_create(&bridge, dir.path(), "r2.txt", b"eight million bytes, in spirit");
        let p = created.outcome_file_id();
        let dest = dir.path().join("fetch").join("r2.txt");
        // `hydrate_dest_is_allowed` canonicalizes the parent.
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        let logs = capture_logs_async(async {
            let source = bridge.serve_hydrate(&p, &dest, &[dir.path()], None).await.unwrap();
            assert_eq!(source, HydrateSource::Queue);
        })
        .await;
        assert_eq!(
            std::fs::read(&dest).unwrap(),
            b"eight million bytes, in spirit",
            "the staged bytes"
        );
        assert_eq!(
            bridge.db.get_file(&p).unwrap().unwrap().status,
            FileStatus::Uploading,
            "still writable"
        );
        assert!(
            logs.contains("Finder hydrate served") && logs.contains("source=\"queue\""),
            "{logs}"
        );
        assert!(server.finish().requests.is_empty(), "nothing reached the server");
    }

    /// T19 (§7.4.2): a failed hydrate leaves a row with a live upload as it was.
    #[tokio::test]
    async fn a_failed_hydrate_never_changes_a_row_with_a_live_upload() {
        // UUID-shaped: the hydrate parses the id before it asks the server.
        let live = "3f2a9c1e-0000-4000-8000-0000000000b2";
        let (dir, _sync_root, server, bridge) = rule3_setup([90u8; 32]);
        seed_uploaded_row(&bridge, &server, live);
        fp_save(&bridge, dir.path(), live, "notes.txt", b"queued edit", "1");
        let op = bridge.db.list_operations_for_file(live).unwrap().remove(0);
        std::fs::remove_file(op.payload_path.as_deref().unwrap()).unwrap();
        let dest = dir.path().join("fetch").join("notes.txt");
        // Without it the guard fails first and the test passes for the wrong reason.
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        assert!(
            bridge.serve_hydrate(live, &dest, &[dir.path()], None).await.is_err(),
            "the server has no metadata route"
        );
        assert_eq!(bridge.db.get_file(live).unwrap().unwrap().status, FileStatus::Uploading);
        let state = server.finish();
        assert!(
            state
                .requests
                .iter()
                .any(|(m, path)| m == "GET" && path == &format!("/api/v1/files/{live}")),
            "the hydrate failed at the server, not before it: {:?}",
            state.requests
        );
    }

    /// T43 (I-1): a deletion-conflicted create of P is a content modify of S, replied under S.
    #[tokio::test]
    async fn a_deletion_conflicted_create_of_the_provisional_id_modifies_the_server_file() {
        let master_key = [91u8; 32];
        let (dir, sync_root, server, bridge) = rule3_setup(master_key);
        let (p, s, create_token) = landed_create(&bridge, &server, dir.path(), &sync_root, "t2.txt").await;
        let edited = dir.path().join("t2-edited.txt");
        std::fs::write(&edited, b"created bytes, edited before the swap").unwrap();
        let reply = bridge
            .queue_file_provider_deletion_conflicted_create(
                finder_file_target(None, "t2.txt", &edited, None),
                Some(p.clone()),
                Some(create_token),
                None,
            )
            .unwrap();
        let crate::engine_bridge::FinderWriteOutcome::Queued { file_id: Some(id), .. } = &reply.outcome else {
            panic!("{reply:?}")
        };
        assert_eq!(id, &s, "a content modify of S");
        assert!(
            reply.present_as.is_none(),
            "a create reply names the provider's identifier, S (REPL.h:438-443)"
        );
        // Guard: without an alias it is an ordinary create.
        let other = dir.path().join("t3.txt");
        std::fs::write(&other, b"a file deleted elsewhere, edited here").unwrap();
        bridge
            .queue_file_provider_deletion_conflicted_create(
                finder_file_target(None, "t3.txt", &other, None),
                Some("3f2a9c1e-0000-4000-8000-0000000000aa".into()),
                Some("0".into()),
                None,
            )
            .unwrap();
        drain_upload_queue(&bridge, &sync_root).await;
        let state = server.finish();
        assert_eq!(state.files[&s].versions.len(), 2, "one new version of S");
        assert_eq!(
            state.latest_plaintext(&s, master_key),
            b"created bytes, edited before the swap"
        );
        assert_eq!(
            state.files_with_content().len(),
            2,
            "S and the guard's new file; no second t2"
        );
    }

    /// T54: every request kind under P reaches S (one case per kind).
    #[cfg(unix)] // names `crate::ipc_socket::file_status_response`
    #[tokio::test]
    async fn the_alias_resolves_rename_move_item_thumbnail_and_hydrate() {
        let master_key = [92u8; 32];
        let (dir, sync_root, server, bridge) = rule3_setup(master_key);
        // The thumbnail and hydrate paths parse a UUID before they ask the server.
        server.state.lock().unwrap().uuid_ids = true;
        let (p, s, create_token) = landed_create(&bridge, &server, dir.path(), &sync_root, "aliased.txt").await;
        let blob = thumbnail_blob(master_key, &s, b"thumbnail bytes");
        server.state.lock().unwrap().thumbnails.insert(s.clone(), blob);
        let rename = |parent: Option<&str>, name: &str| FinderWriteTarget {
            file_id: Some(p.clone()),
            parent_id: parent.map(str::to_string),
            filename: name.into(),
            rel_path: None,
            kind: FinderWriteItemKind::File,
            contents_path: None,
            content_type: None,
            base_version_identifier: Some(create_token.clone()),
        };
        // rename
        bridge.queue_file_provider_modify(rename(None, "renamed.txt")).unwrap();
        // move
        bridge
            .queue_file_provider_modify(rename(Some("3f2a9c1e-0000-4000-8000-0000000000f0"), "renamed.txt"))
            .unwrap();
        let ops = bridge.db.list_operations_for_file(&s).unwrap();
        assert_eq!(
            ops.iter().map(|op| op.kind.clone()).collect::<Vec<_>>(),
            vec![OperationKind::RenameFile, OperationKind::MoveFile],
            "rename and move reach S"
        );
        // item
        let crate::ipc_socket::IpcResponse::FileStatus(item) = crate::ipc_socket::file_status_response(&bridge.db, &p)
        else {
            panic!("item under P")
        };
        assert_eq!(item.identifier, p);
        // thumbnail
        assert_eq!(
            &bridge.finder_thumbnail(&p, "small").await.unwrap()[..],
            b"thumbnail bytes"
        );
        // hydrate (no write queued: the server is asked for S)
        let dest = dir.path().join("fetch").join("aliased.txt");
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        let _ = bridge.serve_hydrate(&p, &dest, &[dir.path()], None).await;
        let state = server.finish();
        assert_eq!(state.thumbnail_requests, vec![s.clone()]);
        assert!(
            state
                .requests
                .iter()
                .any(|(m, path)| m == "GET" && path == &format!("/api/v1/files/{s}")),
            "the hydrate asked for S: {:?}",
            state.requests
        );
    }

    /// T55: an alias is kept 30 days; engine start and the daily tick sweep older ones.
    #[test]
    fn aliases_older_than_30_days_are_swept() {
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        let now = now_secs();
        db.insert_alias_for_test("young", "s1", now - 29 * 86_400);
        db.insert_alias_for_test("old", "s2", now - 31 * 86_400);
        db.engine_start_repair().unwrap(); // at start
        assert_eq!(db.resolve_alias("young").unwrap(), Some("s1".into()));
        assert_eq!(db.resolve_alias("old").unwrap(), None);
        db.insert_alias_for_test("old-again", "s3", now - 31 * 86_400);
        assert_eq!(db.sweep_aliases(now, crate::state_db::ALIAS_MAX_AGE_SECS).unwrap(), 1); // daily
        assert_eq!(
            db.resolve_alias("young").unwrap(),
            Some("s1".into()),
            "the young one stays"
        );
    }

    /// T56′ (§5.6, the safe default): no thumbnail is fetched while the write is queued.
    #[tokio::test]
    async fn the_thumbnail_of_a_queued_write_is_an_error_without_a_server_request() {
        // UUID-shaped: the thumbnail fetch parses the id before it asks the server.
        let photo = "3f2a9c1e-0000-4000-8000-0000000000b1";
        let master_key = [93u8; 32];
        let (dir, sync_root, server, bridge) = rule3_setup(master_key);
        seed_uploaded_row(&bridge, &server, photo);
        let blob = thumbnail_blob(master_key, photo, b"new thumbnail");
        server.state.lock().unwrap().thumbnails.insert(photo.into(), blob);
        fp_save(&bridge, dir.path(), photo, "photo.png", b"new image bytes", "1");
        assert!(
            bridge.finder_thumbnail(photo, "small").await.is_err(),
            "an error while the write is queued"
        );
        assert!(
            server.state.lock().unwrap().thumbnail_requests.is_empty(),
            "0 requests: never the old version's thumbnail"
        );
        drain_upload_queue(&bridge, &sync_root).await;
        assert_eq!(
            &bridge.finder_thumbnail(photo, "small").await.unwrap()[..],
            b"new thumbnail",
            "served after the landing"
        );
        assert_eq!(server.finish().thumbnail_requests, vec![photo.to_string()]);
    }

    /// Review Focus 3: an empty save lands, and a fetch while it is queued is an empty file.
    #[tokio::test]
    async fn an_empty_save_lands_and_is_served_from_the_queue() {
        let master_key = [94u8; 32];
        let (dir, sync_root, server, bridge) = rule3_setup(master_key);
        seed_uploaded_row(&bridge, &server, "empty");
        fp_save(&bridge, dir.path(), "empty", "notes.txt", b"", "1");
        let dest = dir.path().join("fetch").join("notes.txt");
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        assert_eq!(
            bridge.serve_hydrate("empty", &dest, &[dir.path()], None).await.unwrap(),
            HydrateSource::Queue
        );
        assert_eq!(
            std::fs::metadata(&dest).unwrap().len(),
            0,
            "an empty file, not an error"
        );
        drain_upload_queue(&bridge, &sync_root).await;
        let state = server.finish();
        assert_eq!(state.files["empty"].versions.len(), 2);
        assert!(state.latest_plaintext("empty", master_key).is_empty());
    }
}
