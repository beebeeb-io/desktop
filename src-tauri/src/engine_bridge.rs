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
    /// Files whose landed Finder write still waits for its new thumbnails (spec §5.6). The
    /// landing takes the op off the queue before they are uploaded, and until they are, the
    /// server holds the previous version's (one per file): [`Self::finder_thumbnail`] refuses.
    thumbnails_pending: std::sync::Mutex<HashSet<String>>,
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
/// disk; never "no such item", which would make the system delete it. It carries the id
/// the request named, for the refusal's log line (spec §11, through [`loggable_id`]); the
/// text never names it.
#[derive(Debug)]
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
pub(crate) struct UnknownItem {
    pub requested_id: String,
}

impl std::fmt::Display for UnknownItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("this item is not known to Beebeeb")
    }
}

impl std::error::Error for UnknownItem {}

/// A fetch of an item whose held write is still queued, while that write's staged copy is
/// missing (spec §7.4). The bytes the token names exist nowhere else: the server holds only
/// the previous version, which is never served under the new token. Path-free on purpose: the
/// text reaches the extension and the person.
#[derive(Debug)]
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
pub(crate) struct QueuedCopyMissing;

impl std::fmt::Display for QueuedCopyMissing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the latest save of this file is waiting to upload, and its copy cannot be read")
    }
}

impl std::error::Error for QueuedCopyMissing {}

/// Task 1640: a hydrate that would write over a sync-folder file holding a change that is not
/// sent (its bytes differ from the content this device last agreed with the server on, or its
/// upload is queued). Path-free on purpose.
#[derive(Debug)]
pub(crate) struct UnsentLocalChange;

impl std::fmt::Display for UnsentLocalChange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("this file has a change that is not uploaded yet, so it is not replaced")
    }
}

impl std::error::Error for UnsentLocalChange {}

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

/// What [`loggable_id`] answers for an id that is not a UUID.
pub(crate) const NON_UUID: &str = "non_uuid";

/// Spec §11 M5: an id taken from the wire is logged only when it parses as a UUID. Anything
/// else (on Linux, where the identifier check is a no-op, any caller-supplied text) is logged as
/// its kind. Log hygiene on every platform, not a behaviour change.
pub(crate) fn loggable_id(id: &str) -> &str {
    if uuid::Uuid::parse_str(id).is_ok() {
        id
    } else {
        NON_UUID
    }
}

/// The two fields of a line that logs an id from the wire (M5): the id field (`Some` when it
/// is a UUID) and `id_kind` (`Some("non_uuid")` otherwise). Exactly one is `Some`; `tracing`
/// writes nothing for a `None` field.
pub(crate) fn wire_id_fields(id: &str) -> (Option<tracing::field::DisplayValue<&str>>, Option<&'static str>) {
    match loggable_id(id) {
        NON_UUID => (None, Some(NON_UUID)),
        id => (Some(tracing::field::display(id)), None),
    }
}

/// Spec §11: the fixed category of an error, for a log line about a step that
/// failed. The error's text can carry a path (an I/O error's context, a database file name),
/// so a line logs this instead. The first cause it recognises decides. The category comes from
/// closed codes only (SQLite's result code, the I/O error kind), never from a message, and it
/// tells apart the causes that have different answers: a busy database, a full disk, a corrupt
/// file, and a statement the schema does not match (a defect of ours).
pub(crate) fn error_category(error: &(dyn std::error::Error + 'static)) -> &'static str {
    let mut cause = Some(error);
    while let Some(current) = cause {
        if let Some(database) = current.downcast_ref::<rusqlite::Error>() {
            return database_error_category(database);
        }
        if let Some(io) = current.downcast_ref::<std::io::Error>() {
            return match io.kind() {
                std::io::ErrorKind::NotFound => "not_found",
                std::io::ErrorKind::PermissionDenied => "permission_denied",
                std::io::ErrorKind::StorageFull | std::io::ErrorKind::QuotaExceeded => "disk_full",
                std::io::ErrorKind::ReadOnlyFilesystem => "read_only",
                _ => "io",
            };
        }
        if current.downcast_ref::<reqwest::Error>().is_some() {
            return "network";
        }
        // Task 2173: a direct download's authorization answer, in the categories above.
        if let Some(crate::direct_download::DirectError::Refused(status)) =
            current.downcast_ref::<crate::direct_download::DirectError>()
        {
            return match status {
                401 | 403 => "permission_denied",
                404 => "not_found",
                _ => "other",
            };
        }
        cause = current.source();
    }
    "other"
}

/// [`error_category`] for a database error, from rusqlite's variant and SQLite's result code.
fn database_error_category(error: &rusqlite::Error) -> &'static str {
    use rusqlite::ffi::ErrorCode;
    match error {
        rusqlite::Error::SqliteFailure(failure, _) => match failure.code {
            ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked => "database_busy",
            ErrorCode::DiskFull => "database_full",
            ErrorCode::DatabaseCorrupt | ErrorCode::NotADatabase => "database_corrupt",
            ErrorCode::ReadOnly => "database_read_only",
            ErrorCode::CannotOpen => "database_cannot_open",
            ErrorCode::SystemIoFailure => "database_io",
            ErrorCode::ConstraintViolation => "database_constraint",
            // The generic SQLITE_ERROR: a statement that names a table or column the schema
            // does not have.
            ErrorCode::Unknown if failure.extended_code & 0xff == rusqlite::ffi::SQLITE_ERROR => "database_schema",
            _ => "database",
        },
        // The code and the schema disagree about a column, a value or a statement. A statement
        // SQLite rejects at prepare (a column it does not have, a syntax error) is
        // `SqlInputError`, which this crate's `bundled` build always has.
        rusqlite::Error::SqlInputError { .. }
        | rusqlite::Error::InvalidColumnType(..)
        | rusqlite::Error::InvalidColumnIndex(_)
        | rusqlite::Error::InvalidColumnName(_)
        | rusqlite::Error::FromSqlConversionFailure(..)
        | rusqlite::Error::IntegralValueOutOfRange(..)
        | rusqlite::Error::InvalidParameterCount(..)
        | rusqlite::Error::InvalidParameterName(_)
        | rusqlite::Error::MultipleStatement
        | rusqlite::Error::ExecuteReturnedResults => "database_schema",
        _ => "database",
    }
}

/// One line per request that reached the server id through an alias (spec §7.2, §11). The
/// provisional id comes from the wire (M5); the server id is the row's.
pub(crate) fn log_alias_resolved(request: &'static str, provisional_id: &str, file_id: &str) {
    let (provisional_id, id_kind) = wire_id_fields(provisional_id);
    tracing::warn!(
        request,
        provisional_id,
        id_kind,
        file_id = %file_id,
        "provisional id resolved to the server id"
    );
}

/// A delete under a provisional id that does not name the server file's held token (m-6).
fn log_alias_delete_kept(provisional_id: &str, file_id: &str) {
    let (provisional_id, id_kind) = wire_id_fields(provisional_id);
    tracing::warn!(
        provisional_id,
        id_kind,
        file_id = %file_id,
        "delete of a provisional id not applied to the server file"
    );
}

/// The device plan's count of fetches that reached the daemon (spec §11, §14 instrument 2).
/// The id is the request's (M5).
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
fn log_hydrate_served(file_id: &str, source: HydrateSource) {
    let (file_id, id_kind) = wire_id_fields(file_id);
    tracing::info!(file_id, id_kind, source = source.as_str(), "Finder hydrate served");
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

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
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
    /// Spec 2026-10-10 §6.4: an attempt failed at the link level (`offline`), the server refused the session
    /// (`session`), or, by the §15 question 4 ruling, the account refused a Finder write it can cure (`account`).
    pub link_failed: bool,
    pub session_refused: bool,
    pub account_refused: bool,
}

/// How many times one pass of `process_due_operations` may list its due work. Spec D7 (tasks 1888, 2226): on Windows
/// and Linux a change in a folder made here is not due until that folder's create landed, so a pass that lands a
/// folder create lists again for what the landing made due; a new tree up to this many levels deep goes up in one
/// pass. macOS has no gate and lists once, as before.
#[cfg(target_os = "macos")]
const DUE_LISTINGS_PER_PASS: usize = 1;
#[cfg(not(target_os = "macos"))]
const DUE_LISTINGS_PER_PASS: usize = 16;

/// The due work of one pass, in due order. It lists again only after a folder create of the pass landed (D7), at most
/// `DUE_LISTINGS_PER_PASS` times in all, and an op listed earlier in the pass is not run again in it.
struct DueOperations {
    queue: VecDeque<PendingOperation>,
    listed: HashSet<String>,
    listings: usize,
    folder_landed: bool,
}

impl DueOperations {
    fn new(operations: Vec<PendingOperation>) -> Self {
        Self {
            listed: operations.iter().map(|op| op.op_id.clone()).collect(),
            queue: operations.into(),
            listings: 1,
            folder_landed: false,
        }
    }

    /// A folder create of this pass landed: the changes that waited for it may be due now.
    fn folder_landed(&mut self) {
        self.folder_landed = true;
    }

    fn next(&mut self, db: &StateDb, now: i64) -> anyhow::Result<Option<PendingOperation>> {
        while self.queue.is_empty() && self.folder_landed && self.listings < DUE_LISTINGS_PER_PASS {
            self.folder_landed = false;
            self.listings += 1;
            let listed = &mut self.listed;
            self.queue = db
                .list_due_operations(now)?
                .into_iter()
                .filter(|op| listed.insert(op.op_id.clone()))
                .collect();
        }
        Ok(self.queue.pop_front())
    }
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

/// Spec §8.4: does waiting cure this failed attempt of a File Provider write? A transient one
/// counts for the backoff but never uses up the write's attempts; any other uses them up and the
/// write parks at the cap, presented read-only, so it never shows `uploading` for good while
/// later saves wait behind it. Decided by the error's type and kind, never by its text:
/// - `Locked` (the vault, or SQLite busy) and `Quota`: transient;
/// - `Retryable` with an HTTP status: transient for 408, 425, 429 and every 5xx, counted for any
///   other (400, 404, 410, 413, 422, a 409 that is not a stale base, ...);
/// - `Retryable` whose resumed upload session the server reaped (404, 410) and the attempt
///   abandoned ([`SessionAbandoned`]): transient, because the next attempt starts a fresh upload;
/// - `Retryable` with no HTTP status: transient only when the chain carries a transport failure
///   (see [`has_transport_cause`]); a local error is counted;
/// - `Auth` and `Permission` never reach it (they pause and park): `false`.
fn finder_write_failure_is_transient(class: OperationFailureClass, error: &anyhow::Error) -> bool {
    match class {
        OperationFailureClass::Locked | OperationFailureClass::Quota => true,
        // A gone session, abandoned: the next attempt starts a fresh upload.
        OperationFailureClass::Retryable if error.chain().any(|cause| cause.is::<SessionAbandoned>()) => true,
        OperationFailureClass::Retryable => match error_http_status(error) {
            Some(status) => matches!(status, 408 | 425 | 429 | 500..=599),
            None => has_transport_cause(error),
        },
        OperationFailureClass::Auth | OperationFailureClass::Permission => false,
    }
}

/// A transport failure in `error`'s chain: a `reqwest::Error` that never got a response (it could
/// not connect, timed out, or failed while sending), or, below a `reqwest::Error`, an I/O error of a
/// kind the network path gives (a refused, reset, aborted or broken connection, a timeout, an
/// early end). The same I/O kinds with no request above them come from a local operation and are
/// not a transport failure.
fn has_transport_cause(error: &anyhow::Error) -> bool {
    let mut on_the_network_path = false;
    for cause in error.chain() {
        if let Some(request) = cause.downcast_ref::<reqwest::Error>() {
            if request.status().is_none() && (request.is_connect() || request.is_timeout() || request.is_request()) {
                return true;
            }
            on_the_network_path = true;
        }
        if on_the_network_path
            && let Some(io) = cause.downcast_ref::<std::io::Error>()
            && matches!(
                io.kind(),
                std::io::ErrorKind::ConnectionRefused
                    | std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::ConnectionAborted
                    | std::io::ErrorKind::NotConnected
                    | std::io::ErrorKind::TimedOut
                    | std::io::ErrorKind::BrokenPipe
                    | std::io::ErrorKind::UnexpectedEof
            )
        {
            return true;
        }
    }
    false
}

/// Spec §8.4: a refusal the person cures by freeing space, subscribing or paying, read from the
/// server's code by type, never from text, at init, a chunk or complete: 413 `quota_exceeded`, 403
/// `object_budget_exceeded`, 403 `billing_read_only`, and the upload gate's 409s
/// (`account_lapsed`, `plan_required`, `trial_cancelled_read_only`). For a File Provider write it is
/// a quota refusal: never paused, never parked, so the save lands once it is cured; the app's
/// account-state banner, not the file, says why. Any other 413, 403 or 409 keeps its rule.
fn is_curable_refusal(error: &anyhow::Error) -> bool {
    use crate::api_client::{InitConflict, UploadRefusalCode as Code, UploadRefused};
    let upload_gate = |code: Option<Code>| {
        matches!(
            code,
            Some(Code::AccountLapsed | Code::PlanRequired | Code::TrialCancelledReadOnly)
        )
    };
    error.chain().any(|cause| {
        cause
            .downcast_ref::<UploadRefused>()
            .is_some_and(|refused| match (refused.status(), refused.code) {
                (Some(413), Some(Code::QuotaExceeded)) => true,
                (Some(403), Some(Code::ObjectBudgetExceeded | Code::BillingReadOnly)) => true,
                (Some(409), code) => upload_gate(code),
                _ => false,
            })
            || cause
                .downcast_ref::<InitConflict>()
                .is_some_and(|conflict| upload_gate(conflict.refusal))
    })
}

/// The accept found its staged copy's journal row gone: an engine-start sweep removed the copy
/// between the copy and the accept's transaction (spec §8.7 S6). Nothing was queued; the reply is
/// `WriteRetryLater`, so the system hands the write over again. Path-free on purpose: the text
/// reaches the extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
pub(crate) struct StagedCopyGone;

impl std::fmt::Display for StagedCopyGone {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the staged copy of this write was removed before it was queued")
    }
}

impl std::error::Error for StagedCopyGone {}

/// A sign-out closed Finder accepts after its engine stop (spec 2026-10-10 §15 question 3). Nothing was queued; the
/// reply is `WriteRetryLater`, so macOS keeps the change and retries. Path-free: the text reaches the extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AcceptsClosed;

impl std::fmt::Display for AcceptsClosed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Beebeeb is signing out; this write was not queued")
    }
}

impl std::error::Error for AcceptsClosed {}

/// A content save of an item in the trash (spec §7.2): its row is `Trashing`, or the system names
/// the trash container as its parent. Refused as a create in the trash is, so the item stays in
/// the trash; nothing was queued. Path-free: the text reaches the extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
pub(crate) struct ItemInTrash;

impl std::fmt::Display for ItemInTrash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Beebeeb cannot change items inside the Trash.")
    }
}

impl std::error::Error for ItemInTrash {}

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

/// What the runner does with a failed attempt that neither parked at once nor found its op
/// gone ([`failed_attempt_rule`]).
#[derive(Debug, Clone, PartialEq, Eq)]
enum FailedAttemptRule {
    /// Paused until something resumes it (only `auth` is resumed, at sign-in).
    Pause(OperationPauseReason),
    /// Parked with its bytes kept (spec §8.4).
    Park(ParkReason),
    /// Recorded for the backoff, the count moved as [`AttemptCount`] says.
    Retry(AttemptCount),
}

/// How a retried attempt moves the op's attempt count ([`next_attempt_count`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AttemptCount {
    /// One more: at `max_attempts` the op has parked by the queue's rule.
    UsesUp,
    /// One more, but never past `max_attempts - 1` (a recorded completion, §8.6 rule 6).
    ShortOfTheCap,
    /// One more, but never past the attempt where the backoff stops growing
    /// ([`backoff_cap_attempts`]), and never past `max_attempts - 1`; a count already there or
    /// above stays (a transient failure of a File Provider write, §8.4). The backoff still reaches
    /// its cap, and an outage leaves the remaining attempts to the failures waiting does not cure.
    UpToTheBackoffCap,
}

/// The count an attempt records ([`AttemptCount`]).
fn next_attempt_count(attempts: i64, max_attempts: i64, count: AttemptCount) -> i64 {
    let below_the_cap = max_attempts.saturating_sub(1);
    match count {
        AttemptCount::UsesUp => attempts.saturating_add(1),
        AttemptCount::ShortOfTheCap => attempts.saturating_add(1).min(below_the_cap),
        AttemptCount::UpToTheBackoffCap => {
            let limit = backoff_cap_attempts().min(below_the_cap);
            if attempts >= limit {
                attempts
            } else {
                attempts.saturating_add(1)
            }
        }
    }
}

/// The rule for a failed attempt of class `class`.
///
/// - A recorded completion (`completed`, §8.6 rule 6) only ever retries, and never uses up its
///   attempts.
/// - A File Provider write (`finder_write`: an upload with a write id) never stops uploading for
///   a reason that waiting or the person can cure (§8.4): a `transient` failure
///   ([`finder_write_failure_is_transient`]: the network, a 5xx, 408/425/429, a lock, a quota
///   refusal) retries on the normal backoff without using up the attempts, so the save lands once
///   the network, the lock or the space comes back; any other `Retryable` failure uses them up and
///   parks the write at the cap; a permission refusal parks it at once as `permission_denied`,
///   which presents it read-only; `auth` pauses it until the next sign-in.
/// - Every other upload keeps the rules it had: `Retryable` uses up its attempts, every other
///   class pauses it.
fn failed_attempt_rule(
    class: OperationFailureClass,
    finder_write: bool,
    completed: bool,
    transient: bool,
) -> FailedAttemptRule {
    if completed {
        return FailedAttemptRule::Retry(AttemptCount::ShortOfTheCap);
    }
    if !finder_write {
        return match class.pause_reason() {
            Some(reason) => FailedAttemptRule::Pause(reason),
            None => FailedAttemptRule::Retry(AttemptCount::UsesUp),
        };
    }
    match class {
        OperationFailureClass::Auth => FailedAttemptRule::Pause(OperationPauseReason::Auth),
        OperationFailureClass::Permission => FailedAttemptRule::Park(ParkReason::PermissionDenied),
        OperationFailureClass::Retryable | OperationFailureClass::Locked | OperationFailureClass::Quota => {
            FailedAttemptRule::Retry(if transient {
                AttemptCount::UpToTheBackoffCap
            } else {
                AttemptCount::UsesUp
            })
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
            thumbnails_pending: std::sync::Mutex::new(HashSet::new()),
            failed_snapshots: std::sync::atomic::AtomicU32::new(0),
            newly_held: Default::default(),
            resume: crate::runner::ResumeTicket::default(),
            #[cfg(test)]
            seams: Seams::default(),
        }
    }

    pub(crate) fn owner_key_scope(&self) -> crate::owner_keys::AccountScope<'_> {
        crate::owner_keys::AccountScope::new(&self.db, &self.api, &self.stopping)
    }

    /// Mark or clear `file_id`'s landed write as waiting for its new thumbnails (spec §5.6).
    fn set_thumbnails_pending(&self, file_id: &str, pending: bool) {
        let mut set = self
            .thumbnails_pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if pending {
            set.insert(file_id.to_string());
        } else {
            set.remove(file_id);
        }
    }

    /// Whether `file_id`'s landed write still waits for its new thumbnails (spec §5.6).
    #[cfg_attr(not(any(unix, test)), allow(dead_code))]
    fn thumbnails_pending(&self, file_id: &str) -> bool {
        self.thumbnails_pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains(file_id)
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

    /// Share the account's resume and pass requests (spec D11; review P2). [`crate::runner::run`] passes the same
    /// ticket it serves in [`Self::reconcile_then_resume`].
    pub fn with_resume(mut self, resume: crate::runner::ResumeTicket) -> Self {
        self.resume = resume;
        self
    }

    /// Review P2 (task 1888): a release asked for a pass that no committed pass has served yet. A released change may
    /// be a ghost folder's create the pass has not seen; sent now, it would make a folder the server already has a
    /// second time (anti-goal 1). The next successful tick runs the pass.
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    fn a_pass_is_owed(&self) -> bool {
        self.resume.pass_wanted.load(Ordering::SeqCst) > self.resume.pass_served.load(Ordering::SeqCst)
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

    /// Spec D11, D12 and I6 (task 1888): Windows and Linux resume the operations paused for `auth` here, in the engine,
    /// after a successful pull proved that the server accepts this session's token. It acts at the run's first
    /// successful tick (`*reconciled` still false; the first-tick rule) and at any later one where a sign-in asked
    /// (`asked > done`). Each time:
    /// - the reconcile pass (D1-D10), then the removal of the scaffolding directories it dropped (D5);
    /// - the key check (D12): every name that can be checked opens under this session's key, or every `auth` op is held
    ///   as `key_replaced`;
    /// - the resume (or that hold), then `done = asked`.
    ///
    /// A release that asked for a pass (`pass_wanted > pass_served`; review P2) gets the pass alone: no key check, no
    /// resume. Any committed pass serves the release ask it read before it planned.
    ///
    /// `*reconciled` turns true only when all of it committed, so a step that fails is tried again at the next tick,
    /// and nothing is resumed before a pass of this run (I6). Local only, counts only in the log (I7).
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    pub async fn reconcile_then_resume(
        &self,
        sync_root: &Path,
        resume: &crate::runner::ResumeTicket,
        reconciled: &mut bool,
    ) -> anyhow::Result<()> {
        let asked = resume.asked.load(Ordering::SeqCst);
        // Review P2: a release's ask, read before the pass plans, so the pass that serves it has seen the release.
        let wanted = resume.pass_wanted.load(Ordering::SeqCst);
        let first_pass = !*reconciled;
        let resume_owed = first_pass || asked > resume.done.load(Ordering::SeqCst);
        if !resume_owed && wanted <= resume.pass_served.load(Ordering::SeqCst) {
            return Ok(());
        }
        let opts = crate::held_queue::Options {
            device_segment: crate::runner::device_name()
                .map(|name| crate::known_folder::sanitize_device_segment(&name)),
            case_insensitive: cfg!(windows),
        };
        let now = now_secs();
        let counts = self
            .db
            .reconcile_held_queue(&opts, now, &|rel| crate::state_db::tree_holds_a_file(&sync_root.join(rel)))?;
        remove_scaffold_dirs(sync_root, &counts.scaffold_dirs);
        tracing::info!(
            landed = counts.landed,
            remapped = counts.remapped,
            replay = counts.replay,
            held = counts.held_ambiguous
                + counts.held_acts_on_ghost
                + counts.held_parents_disagree
                + counts.held_twin_busy,
            released = counts.released,
            rearmed = counts.rearmed + counts.rearmed_paused,
            parent_resolved = counts.parent_resolved,
            scaffold_dropped = counts.scaffold_dropped,
            backup_unknown_kept = counts.backup_unknown_kept,
            file_twin = counts.file_twin,
            "reconcile pass committed"
        );
        // A pass that only a release asked for resumes nothing: a release is not a sign-in (spec D11).
        let (resumed, held_key_replaced) = if !resume_owed {
            (0, 0)
        } else if self.auth_ops_open_under_this_key()? {
            (self.db.resume_operations_paused_for_auth(now)?, 0)
        } else {
            (0, self.db.hold_operations_paused_for_auth_as_key_replaced(now)?)
        };
        if resume_owed {
            resume.done.fetch_max(asked, Ordering::SeqCst);
        }
        resume.pass_served.fetch_max(wanted, Ordering::SeqCst);
        *reconciled = true;
        tracing::info!(
            resumed,
            held_key_replaced,
            first_pass,
            "after the reconcile pass: the operations paused for auth were resumed, or held under a replaced key"
        );
        Ok(())
    }

    /// Spec D12 (task 1888): whether the operations paused for `auth` were queued under this session's vault key. An op
    /// is checked when its metadata carries a sealed name (an envelope, as the 2267 proof reads it) and no shared
    /// context (`shared_root_id`, `share_id`), so its name is sealed by its own `file_id`: it must open under the
    /// master key in memory ([`name_opens`]). Ops with no name, or with a shared context, are not checked and follow
    /// the verdict; with none checked, nothing contradicts this key and the verdict is that they open. The key never
    /// leaves memory, and nothing is logged.
    fn auth_ops_open_under_this_key(&self) -> anyhow::Result<bool> {
        let master_key = self.api.master_key();
        for op in self.db.operations_paused_for_auth()? {
            let Some(file_id) = op.file_id.as_deref() else {
                continue;
            };
            let Some(metadata) = op
                .metadata_json
                .as_deref()
                .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
            else {
                continue;
            };
            let shared = ["shared_root_id", "share_id"]
                .iter()
                .any(|field| metadata.get(*field).is_some_and(|value| !value.is_null() && value.as_str() != Some("")));
            let sealed = metadata
                .get("name_encrypted")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|sealed| sealed.starts_with('{'));
            if let Some(sealed) = sealed
                && !shared
                && !name_opens(master_key, file_id, sealed)
            {
                return Ok(false);
            }
        }
        Ok(true)
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
            link_failed: false,
            session_refused: false,
            account_refused: false,
        };
        #[cfg(target_os = "windows")]
        if !self.is_stopping() {
            crate::windows_cf::upload_finalization::retry(&self.db, sync_root);
        }
        let mut due = DueOperations::new(self.db.list_due_operations(now)?);

        while let Some(op) = due.next(&self.db, now)? {
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
            // Review P2 (task 1888): a release that asked for a pass after this list was read stops the queue here, so
            // a released ghost create is never sent before a pass has seen it; the next successful tick runs the pass.
            #[cfg(not(target_os = "macos"))]
            if self.a_pass_is_owed() {
                break;
            }
            // The list is read once per pass, and an upload that lands moves
            // the later ops of its file along (`StateDb::apply_landing`),
            // so run each op as it is NOW: the claim re-reads it, enforces its
            // file's content order and marks this attempt, in one transaction.
            // Every write that records the attempt's outcome names the claim.
            // Every outcome below that changes how an item is presented (spec §9.1) puts the
            // item's id in `invalidated_item_ids`, once its transaction committed, so the tick's
            // one working-set signal tells the system to read it again. Without it a park
            // reached Finder only at the next unrelated signal, and the person could keep
            // editing a file whose saves would not upload.
            self.seam("claim:before_tx");
            let claimed = match self.db.claim_operation(&op.op_id, now)? {
                // Waiting for an earlier content op of its file is not an attempt.
                ClaimOutcome::Gone | ClaimOutcome::Wait => continue,
                // The hand-over committed (spec §8.4); the successor itself still waits.
                ClaimOutcome::WaitAfterHandOver(took_over) => {
                    self.after_hand_over(&op, &took_over);
                    outcome.invalidated_item_ids.extend(op.file_id.clone());
                    continue;
                }
                ClaimOutcome::Claimed(claimed) => {
                    if let Some(took_over) = &claimed.took_over {
                        self.after_hand_over(&claimed.op, took_over);
                        outcome.invalidated_item_ids.extend(claimed.op.file_id.clone());
                    }
                    *claimed
                }
                // The park committed in the claim's transaction, with any hand-over: the
                // successor is the op that parked, so its id covers both.
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
                    outcome.invalidated_item_ids.extend(file_id);
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
                    if let Some(path) = release.as_deref() {
                        crate::staged_payload::remove_or_log(&self.db, Path::new(path));
                    }
                    if let Some(file_id) = &op.file_id {
                        outcome.invalidated_item_ids.push(file_id.clone());
                    }
                    if op.kind == OperationKind::CreateFolder {
                        due.folder_landed();
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
                    outcome.invalidated_item_ids.extend(op.file_id.clone());
                    outcome.retried_op_ids.push(op.op_id);
                }
                // A stale base never becomes valid again: the server's version only grows
                // (spec §8.4). A File Provider write parks at once, and so does every other
                // upload sent with a base (a sync-folder edit, task 1640), its bytes kept; an
                // upload without a base keeps today's retries.
                Err(error)
                    if !completed
                        && init_conflict_class(&error) == Some(crate::api_client::InitConflictClass::StaleBase)
                        && (claimed.write.is_some() || op.base_version.is_some()) =>
                {
                    if !self
                        .db
                        .park_claimed(&op.op_id, &claimed.claim_id, ParkReason::StaleBase, now)?
                    {
                        log_queue_state_moved(&op.op_id, "park");
                        continue;
                    }
                    log_refused_upload(
                        &op,
                        op.attempts.saturating_add(1),
                        crate::api_client::InitConflictClass::StaleBase,
                        true,
                    );
                    outcome.invalidated_item_ids.extend(op.file_id.clone());
                    outcome.retried_op_ids.push(op.op_id);
                }
                Err(error) => {
                    let finder_write = claimed.write.is_some()
                        && matches!(op.kind, OperationKind::UploadVersion | OperationKind::UploadFile);
                    // §8.4: for a File Provider write, a refusal the person cures is a quota refusal.
                    let class = if finder_write && is_curable_refusal(&error) {
                        OperationFailureClass::Quota
                    } else {
                        classify_operation_error(&error.to_string())
                    };
                    outcome.link_failed |= crate::link_health::classify_error(&error).is_some();
                    outcome.session_refused |= class == OperationFailureClass::Auth;
                    outcome.account_refused |= finder_write && is_curable_refusal(&error);
                    // m-11: a recorded completion is never paused (a busy database reads as
                    // `Locked`), because a paused op is not listed again.
                    let transient = finder_write && finder_write_failure_is_transient(class, &error);
                    match failed_attempt_rule(class, finder_write, completed, transient) {
                        FailedAttemptRule::Pause(reason) => {
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
                        }
                        // The park settled the row in its transaction (spec §9.2).
                        FailedAttemptRule::Park(reason) => {
                            if !self.db.park_claimed(&op.op_id, &claimed.claim_id, reason, now)? {
                                log_queue_state_moved(&op.op_id, "park");
                                continue;
                            }
                            log_parked(&op.op_id, op.file_id.as_deref(), reason);
                            outcome.invalidated_item_ids.extend(op.file_id.clone());
                            outcome.retried_op_ids.push(op.op_id);
                        }
                        FailedAttemptRule::Retry(count) => {
                            // §8.6 rule 6 and §8.4: a recorded completion, and a File Provider
                            // write whose failure waiting cures, keep retrying on the normal
                            // backoff and never use up their attempts.
                            let uses_up_attempts = count == AttemptCount::UsesUp;
                            let attempts = next_attempt_count(op.attempts, op.max_attempts, count);
                            if completed {
                                log_completed_landing_retried(&op.op_id, op.file_id.as_deref(), attempts);
                            }
                            if matches!(op.kind, OperationKind::UploadVersion | OperationKind::UploadFile)
                                && error_http_status(&error) == Some(409)
                            {
                                let class =
                                    init_conflict_class(&error).unwrap_or(crate::api_client::InitConflictClass::Other);
                                log_refused_upload(&op, attempts, class, false);
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
                            if uses_up_attempts && attempts >= op.max_attempts {
                                // A File Provider write parked by the queue's rule: its line, once,
                                // after the record committed. A 409 says so on its own line above.
                                if finder_write && error_http_status(&error) != Some(409) {
                                    log_parked_attempts_used_up(&op.op_id, op.file_id.as_deref());
                                }
                                self.abandon_upload_after_give_up(&op).await;
                                // Its attempts are used up, so it has parked by the queue's rule,
                                // and the attempt's transaction settled the row (spec §9.2).
                                outcome.invalidated_item_ids.extend(op.file_id.clone());
                            }
                            outcome.retried_op_ids.push(op.op_id);
                        }
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
        if let Some(path) = took_over.released_payload.as_deref() {
            crate::staged_payload::remove_or_log(&self.db, Path::new(path));
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
                // Spec §7.2 (final review): a rename leaves the parent alone; a move names its
                // target, the top level (no parent) included.
                let parent = match op.kind {
                    OperationKind::MoveFile => crate::api_client::ParentChange::to(op.parent_id.as_deref()),
                    _ => crate::api_client::ParentChange::Unchanged,
                };
                self.api.update_metadata(file_id, name, parent).await?;
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
        // Write-keyed: a File Provider write (spec §9.1–§9.2). Keep Mine (`claim: None`) and
        // Windows and watcher uploads keep today's status handling.
        let finder_write = claim.and_then(|c| c.write.as_ref()).is_some();
        // §9.1 (m-8): an item in the trash never looks writable. A Finder write's attempt
        // leaves a `Trashing` row in the trash; its `init` meets the server's refusal and retries.
        if finder_write {
            self.db.set_uploading_unless_trashing(local_file_id)?;
        } else {
            self.db.set_status(local_file_id, FileStatus::Uploading)?;
        }

        struct Rollback<'a> {
            db: &'a StateDb,
            id: &'a str,
            previous: Option<FileStatus>,
            finder_write: bool,
        }
        impl Drop for Rollback<'_> {
            fn drop(&mut self) {
                // §9.2: a Finder write's row stays `Uploading` while its op will retry; a park
                // sets `Error` in the park's own transaction (`settle_status_after_park_conn`).
                if self.finder_write {
                    return;
                }
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
            finder_write,
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
            // (`test_process_due_operations_records_retry_for_upload_worker_handoff`), without
            // the path: `classify_operation_error` matches bare substrings such as `403`, so a
            // path would pause the op instead.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(anyhow::anyhow!("staged upload payload is missing"));
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
            if previous.is_create && !previous.self_minted {
                // SF-0001 (L13): an older version let the server mint a create's id.
                // Such a session can never be ours again, whatever its payload: it
                // is abandoned and this attempt starts a fresh, self-minted upload.
                tracing::info!(
                    op_id = %op.op_id,
                    file_id = %previous.server_file_id,
                    "upload: persisted create session was not self-minted — abandoning it and starting a fresh upload"
                );
                self.abandon_upload_session(&previous).await?;
            } else {
                let same_payload = previous.payload_path == payload_path_str
                    && previous.payload_size == plaintext_size as i64
                    && previous.payload_mtime_ns == payload_mtime_ns
                    && previous.chunk_size_bytes > 0
                    && previous.chunk_count > 0
                    && previous.acked_chunks >= 0
                    && previous.acked_chunks <= previous.chunk_count;
                let v2 = owned_key_and_pin(&self.db, self.api.master_key(), &previous.server_file_id)?.1
                    == beebeeb_core::chunk_stream::ExpectedChunkFormat::V2;
                if same_payload && v2 {
                    // A fresh core writer has a fresh stream identity. Never append it to an old v2 stream.
                    // Fresh init can refuse a live lease; preserve the old resume record until it succeeds.
                } else if same_payload {
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
        }

        // Only a session resumed from its persisted row can have been reaped by the server (each
        // chunk renews a session's lease, so one opened in this attempt cannot be).
        let resumed = session.is_some();
        let session = match session {
            Some(session) => session,
            None => {
                guard_init_base(
                    op.base_version,
                    is_create,
                    claim.and_then(|claim| claim.write.as_ref()).is_some(),
                )?;
                // SF-0001 (L13): this device chooses the item id. A create proposes
                // its own id when that id is canonical; otherwise (a local-only id)
                // it mints a fresh one for this init. A replace keeps the row's id.
                let proposed_id = if is_create && !is_canonical_item_id(local_file_id) {
                    uuid::Uuid::new_v4().to_string()
                } else {
                    local_file_id.to_string()
                };
                let init_request = upload_init_request_for_operation(
                    &proposed_id,
                    name_encrypted,
                    content_type.clone(),
                    op.parent_id.clone(),
                    plaintext_size,
                    op.base_version,
                );
                let upload = self.api.init_upload(&init_request).await?;
                if upload.chunk_size_bytes <= 0 || upload.chunk_count <= 0 {
                    return Err(anyhow::anyhow!("upload init returned invalid chunk plan"));
                }
                // Before anything is persisted or sealed: an init that answers with
                // another id is not this device's item. Exact equality — a re-spelled
                // echo (upper case, trimming) is a different id.
                if upload.file_id != proposed_id {
                    return Err(anyhow::anyhow!(
                        "server_changed_item_id: the upload init answered with an id this device did not choose"
                    ));
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
                    // The op kind, not whether init carried an id: every init now does.
                    is_create,
                    // Every session this code opens sends an id it chose itself.
                    self_minted: true,
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
        // SF-0001 (L13): whatever the session's origin (resumed or fresh), it must
        // target the item this op writes before any byte or name goes to the server.
        ensure_session_item_id(&session, local_file_id)?;

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
                ensure_session_item_id(&session, local_file_id)?;
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
                    let status = error_http_status(&error);
                    // The status and a category only: the error's text carries the request URL
                    // with the session id (spec §11).
                    tracing::warn!(
                        op_id = %op.op_id,
                        file_id = %session.server_file_id,
                        status,
                        resumed,
                        reason = error_category(error.as_ref()),
                        "upload: server no longer accepts the upload session — abandoning it; the next attempt starts a fresh upload"
                    );
                    self.abandon_upload_session(&session).await?;
                    // §8.4: a resumed session the server reaped (404, 410) is not a counted
                    // failure: the next attempt starts a fresh upload. A 400, 404 or 410 at a
                    // session opened in this attempt, and a 400 at a resumed one, is counted, so a
                    // server that refuses every new session still parks the write.
                    if resumed && matches!(status, Some(404 | 410)) {
                        return Err(anyhow::Error::new(SessionAbandoned(error)));
                    }
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
        // §5.6: a Finder write's thumbnails are uploaded after the landing commits, and the
        // commit takes the op off the queue. From just before it to the end of that upload the
        // server holds only the previous version's thumbnail, which the system would cache under
        // the token: `finder_thumbnail` refuses meanwhile. Keyed on a minted write (macOS).
        let marks_thumbnails = claim.and_then(|c| c.write.as_ref()).is_some() && payload_path.is_some();
        if marks_thumbnails {
            self.set_thumbnails_pending(server_file_id, true);
        }
        // Task 1640: a sync-folder upload (no write id) records what it sent as the content this
        // device now agrees with the server on. A Finder write's copy is not read again.
        let completed_payload_hash = match (claim.and_then(|c| c.write.as_ref()), payload_path.as_deref()) {
            (None, Some(path)) => match sha256_file_hex(Path::new(path)) {
                Ok(hash) => Some(hash),
                Err(e) => {
                    tracing::warn!(
                        op_id = %op.op_id,
                        error = %e,
                        "landing: could not hash the copy that was sent; the file keeps its previous agreed content"
                    );
                    None
                }
            },
            _ => None,
        };
        let master_key = self.api.master_key();
        let landing_material = owned_key_and_pin(&self.db, master_key, server_file_id)?;
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
                completed_payload_hash: completed_payload_hash.as_deref(),
                finalization: finalization.as_ref(),
                now: now_secs(),
            },
            &|queued, server_id| metadata_rekeyed_with_material(master_key, queued, server_id,
                (server_id == server_file_id).then_some((&landing_material.0, landing_material.1))),
        );
        // Nothing landed: the op is still queued, and the refusal is the queue's again.
        let landed = match landed {
            Ok(Some(landed)) => landed,
            Ok(None) => {
                if marks_thumbnails {
                    self.set_thumbnails_pending(server_file_id, false);
                }
                log_queue_state_moved(&op.op_id, "landing");
                return Err(anyhow::Error::new(QueueStateMoved));
            }
            Err(e) => {
                if marks_thumbnails {
                    self.set_thumbnails_pending(server_file_id, false);
                }
                return Err(e.into());
            }
        };
        for (successor, reason) in &landed.parked_successors {
            log_parked(successor, Some(server_file_id), *reason);
        }
        self.record_transfer_done(crate::transfer_progress::Direction::Up, server_file_id, size);
        if let Some(path) = payload_path.as_deref() {
            // Task 1700: post-complete thumbnail work never fails the upload, but its
            // failure is surfaced on the outcome so a red run names the real error.
            let file_key = owned_key_and_pin(&self.db, self.api.master_key(), server_file_id)?.0;
            match self
                .finish_completed_upload(op, server_file_id, Path::new(path), content_type, &file_key, sync_root)
                .await
            {
                Ok(()) => {
                    if marks_thumbnails {
                        self.set_thumbnails_pending(server_file_id, false);
                    }
                }
                // The refusal stays: the server still holds the previous version's thumbnail.
                Err(e) => {
                    // A category only: the error's text can carry the request URL (spec §11).
                    tracing::warn!(
                        file_id = %server_file_id,
                        reason = error_category(e.as_ref()),
                        "upload-time thumbnail generation/upload skipped"
                    );
                    post_complete_errors.push(format!("{}: {e}", op.op_id));
                }
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
        // SF-0001 (L13): never seal a name or send a chunk under an id this device
        // did not choose, even when the session was read back from disk.
        ensure_session_item_id(session, local_file_id)?;
        let server_file_id = session.server_file_id.clone();
        if !session.metadata_applied {
            let effective_name_encrypted = if server_file_id != local_file_id {
                owned_metadata_for_name(
                    &self.db, self.api.master_key(),
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
                    upload_parent_change(op, metadata),
                )
                .await?;
            self.db.set_upload_resume_metadata_applied(&op.op_id)?;
        }

        let (file_key, pin) = owned_key_and_pin(&self.db, self.api.master_key(), &server_file_id)?;

        let file = std::fs::File::open(payload_path)?;
        let chunk_size = session.chunk_size_bytes as usize;
        let chunk_count = session.chunk_count as u64;
        if chunk_size == 0 || chunk_count == 0 {
            return Err(anyhow::anyhow!("upload init returned invalid chunk plan"));
        }
        let first_chunk = session.acked_chunks.max(0) as u64;
        anyhow::ensure!(first_chunk == 0 || pin != beebeeb_core::chunk_stream::ExpectedChunkFormat::V2,
            "a v2 upload must restart in a fresh session");
        let mut writer = beebeeb_core::chunk_stream::ChunkEncryptor::from_reader_with_file_key(
            file_key, &server_file_id, session.payload_size as u64, chunk_size as u64, file)?
            .with_recorded_pin(pin)?;
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

        for chunk_index in 0..chunk_count {
            let encrypted = writer.next_chunk()?.ok_or_else(|| anyhow::anyhow!("staged upload ended early"))?;
            if chunk_index < first_chunk { continue; }
            let read = encrypted.data.len().saturating_sub(28);
            let chunk_start = std::time::Instant::now();
            self.api
                .upload_session_chunk(&session.upload_session_id, chunk_index as u32, &encrypted.data)
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

        writer.finish()?;
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
        crate::staged_payload::remove_or_log(&self.db, payload_path);
        thumbnails
    }

    /// Give up on a persisted upload session (payload changed, session gone,
    /// not self-minted, or the op exhausted its retries). For a CREATE the
    /// server row holds no completed content and is only a broken
    /// `is_uploading` duplicate, so its upload is abandoned — never trashed:
    /// it is this client's own row, not something the user deleted
    /// (best-effort: the server's stale-upload sweep still reaps it after 7
    /// days if this fails). A REPLACE targets the user's existing file and its
    /// session is only dropped locally. The resume row is always dropped so the
    /// next attempt starts a fresh session.
    async fn abandon_upload_session(&self, session: &UploadResume) -> anyhow::Result<()> {
        if session.is_create {
            match self.api.abandon_upload(&session.server_file_id).await {
                Ok(()) => tracing::info!(
                    op_id = %session.op_id,
                    file_id = %session.server_file_id,
                    "upload: abandoned the in-progress server row of an abandoned create"
                ),
                Err(e) => tracing::warn!(
                    op_id = %session.op_id,
                    file_id = %session.server_file_id,
                    error = %e,
                    "upload: could not abandon the in-progress server row; the server's stale-upload sweep reaps it"
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
        let pin = owned_key_and_pin(&self.db, self.api.master_key(), server_file_id)?.1;
        let uploads = prepare_thumbnail_uploads_with_pin(payload_path, mime_type, file_key, pin)?;
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
        let Some(queued_path) = op.target_path.as_deref() else {
            return Ok(None);
        };
        // Task 2204 S0.2 (review-128 F4): renamed since it was queued, the file is at its row's
        // path; the landing writes the journal there too.
        let renamed_path = match op.file_id.as_deref() {
            Some(id) if self.db.has_queued_local_rename(id)? => self.db.get_file(id)?.map(|row| row.path),
            _ => None,
        };
        let target_path = renamed_path.as_deref().unwrap_or(queued_path);
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
            review: None,
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
        let scope = self.owner_key_scope();
        scope.check()?;
        let response = self.api.get_shared_folder(&root.file_id).await;
        scope.check()?;
        let custody = match response {
            Ok(row) => {
                anyhow::ensure!(row["folder_id"].as_str() == Some(&root.file_id)
                    && row["key_version"].as_u64().is_some_and(|v| v > 0), "invalid folder custody record");
                // The server returns the caller's copy: L6 for owners, the
                // existing authenticated invite envelope for members. A member
                // cannot open another account's owner/L6 envelope.
                let custody_key = if row["role"] == "member" {
                    let wrapped = row["wrapped_folder_key"].as_str()
                        .ok_or_else(|| anyhow::anyhow!("missing member custody key"))?;
                    unwrap_folder_share_key(self.api.master_key(), sender_public_key, &root.file_id, wrapped)?
                } else {
                    anyhow::ensure!(row["role"] == "owner", "invalid custody role");
                    let wrap = crate::owner_keys::normalize_wrap(&row["owner_wrapped_folder_key"])?
                        .ok_or_else(|| anyhow::anyhow!("missing owner custody key"))?;
                    let key = beebeeb_sync::key_resolution::unwrap_folder_custody(
                        &beebeeb_core::kdf::MasterKey::from_bytes(*self.api.master_key()), &root.file_id, &wrap)?;
                    Zeroizing::new(*key.as_bytes())
                };
                anyhow::ensure!(*custody_key == *folder_key, "custody and invite keys differ");
                scope.remember_custody(&root.file_id)?;
                true
            }
            Err(error) if crate::owner_keys::custody_absent(&error) => {
                anyhow::ensure!(!self.db.has_seen_folder_custody(&root.file_id)?, "This folder's keys are unavailable");
                false
            }
            Err(error) => return Err(error),
        };
        let mut folder_keys_response = self.api.get_folder_keys(&root.invite_id).await?;
        scope.check()?;
        folder_keys_response["desktop_has_custody"] = serde_json::json!(custody);
        Ok((folder_key, folder_keys_response))
    }

    fn shared_folder_child_key(
        &self,
        folder: &[u8; 32],
        root: &str,
        id: &str,
        kind: u8,
        wrap: &str,
        custody: bool,
    ) -> anyhow::Result<beebeeb_core::kdf::FileKey> {
        if custody {
            return unwrap_folder_child_for_format(folder, root, id, kind, wrap, true);
        }
        self.db.read_without_folder_custody(root, || {
            unwrap_folder_child_for_format(folder, root, id, kind, wrap, false)
        })?.ok_or_else(|| anyhow::anyhow!("This folder's keys are unavailable"))?
    }

    fn decrypted_folder_root_name(
        &self,
        root: &SharedRootMapping,
        folder_key: &[u8; 32],
        folder_keys_response: &serde_json::Value,
    ) -> Option<String> {
        let name_encrypted = root.file_name_encrypted.as_deref()?;
        let encrypted_file_key = encrypted_file_key_from_folder_keys_response(folder_keys_response, &root.file_id)?;
        let file_key = self.shared_folder_child_key(folder_key, &root.file_id, &root.file_id, 2,
            encrypted_file_key, folder_keys_response["desktop_has_custody"].as_bool() == Some(true)).ok()?;
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
                    .and_then(|encrypted| self.shared_folder_child_key(folder_key, &root.file_id, &child_id,
                        if item_kind_from_metadata(&child) == ItemKind::Folder { 2 } else { 1 }, encrypted,
                        folder_keys_response["desktop_has_custody"].as_bool() == Some(true)).ok());
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

            let item_kind = if root.is_folder {
                ItemKind::Folder
            } else {
                ItemKind::File
            };
            // Spec §7.2 (final review): the row and its contract in one transaction, so no
            // listing sees a new shared root in the person's own namespace.
            self.db.upsert_file_with_contract(
                &FileEntry {
                    file_id: root.file_id.clone(),
                    path: root_path,
                    status: FileStatus::CloudOnly,
                    size_bytes: root.size_bytes,
                    modified_at: root.approved_at.unwrap_or(now),
                    content_hash: None,
                    remote_updated_at: root.approved_at.unwrap_or(0),
                    // upsert_file does not persist these; the contract update sets the
                    // authoritative parent_id/item_kind.
                    parent_id: None,
                    item_kind: item_kind.clone(),
                },
                |contract| {
                    contract.namespace = Namespace::SharedWithMe;
                    contract.parent_id = None;
                    contract.shared_root_id = Some(root.file_id.clone());
                    contract.share_id = Some(root.invite_id.clone());
                    contract.owner_email = root.owner_email.clone();
                    contract.permission_bits = root.permission_bits;
                    contract.item_kind = item_kind;
                    contract.content_type = root.content_type.clone();
                    contract.last_sync_at = now;
                },
            )?;

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
                let metadata = owned_metadata_for_name(&self.db, self.api.master_key(), &file_id, &target.filename, None)?;
                // Spec §7.2 (final review): an extension's create names its folder by parent
                // only, so the row takes the full relative path under that parent, as every
                // Finder writer does. A caller's own relative path (the watcher's) is kept.
                let row_path = match &target.rel_path {
                    Some(_) => rel_path.clone(),
                    None => self
                        .db
                        .relative_path_under(target.parent_id.as_deref(), &target.filename)?,
                };
                // Write the local folder row + contract synchronously, the same
                // way the File branch writes its `Uploading` row, so a nested
                // child dispatched later in the SAME scan resolves this folder as
                // its parent (`resolve_parent_id_for` reads `files.item_kind`)
                // without waiting for a server `/sync` round-trip. `Local` =
                // present on disk, server CreateFolder op pending in the queue.
                // Spec §7.2 (final review): the row and its parent in one transaction, so no
                // listing sees the new folder without its parent.
                let entry = FileEntry {
                    file_id: file_id.clone(),
                    path: row_path,
                    status: FileStatus::Local,
                    size_bytes: 0,
                    modified_at: now_secs(),
                    content_hash: None,
                    remote_updated_at: 0,
                    // Not persisted by upsert_file; the contract update owns these.
                    parent_id: None,
                    item_kind: ItemKind::Folder,
                };
                let mut payload = serde_json::json!({
                    "operation": "create_folder",
                    "name_encrypted": metadata,
                });
                apply_shared_context(&mut payload, parent_contract.as_ref());
                self.enqueue_finder_operation(
                    OperationKind::CreateFolder,
                    Some(file_id),
                    target.parent_id.clone(),
                    Some(rel_path),
                    payload,
                    None,
                    None,
                    None,
                    // The row, its contract (`Folder`, under its parent) and the op in one transaction, so a
                    // nested child dispatched later still resolves this folder as its parent, and a refused
                    // create leaves no row.
                    Some(crate::state_db::FinderFolderRow {
                        entry: &entry,
                        parent_id: target.parent_id.as_deref(),
                    }),
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
                // Spec §7.2 (final review): the row and its parent in one transaction, as the
                // landing will write them, so no listing sees a new nested file without its
                // parent while it is queued.
                self.db.upsert_file_with_contract(&row, |contract| {
                    contract.item_kind = ItemKind::File;
                    contract.parent_id = target.parent_id.clone();
                })?;
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
        let name_encrypted = owned_metadata_for_name(&self.db, self.api.master_key(), &file_id, &target.filename, mime)?;
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
        self.queue_file_provider_create_retiring(target, contents, None)?
            .map(|(write, _)| write)
            .ok_or_else(|| anyhow::anyhow!("a create that retires no parked write is never refused as moved"))
    }

    /// Task 2190 ("Keep both", review-169 #3): [`Self::queue_file_provider_create_from`], taking the
    /// place of the parked write `retire` names in the SAME transaction. `Ok(None)`: that write is no
    /// longer decidable at the version shown, nothing was queued, and the copy staged for the create
    /// is removed with its guard. With `retire`, the second value is the parked write's staged copy,
    /// released in the journal, for the caller to unlink. A file only: a folder create goes through
    /// [`Self::queue_file_provider_create_from`], and "Keep both" copies a file.
    fn queue_file_provider_create_retiring(
        &self,
        target: FinderWriteTarget,
        contents: Option<&std::fs::File>,
        retire: Option<crate::state_db::RetireParked<'_>>,
    ) -> anyhow::Result<Option<(FpWrite, Option<String>)>> {
        let Some(start) = self.start_finder_create(&target)? else {
            return Ok(Some((FpWrite::plain(ignored_finder_item(&target.filename)), None)));
        };
        let prepared = self.prepare_finder_create(&target, contents, start)?;
        let op_id = uuid::Uuid::new_v4().to_string();
        let metadata_json = serde_json::to_string(&prepared.payload)?;
        let backup_source_key = crate::known_folder::backup_source_key_for_this_device(&prepared.rel_path);
        self.seam("accept:before_tx");
        let accepted = self.db.accept_finder_write(
            &crate::state_db::FinderAccept {
                retire_parked: retire,
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
        let (token, released) = match accepted {
            crate::state_db::AcceptOutcome::Queued { token, .. } => (token, None),
            crate::state_db::AcceptOutcome::QueuedRetiring { token, released } => (token, released),
            crate::state_db::AcceptOutcome::ParkedAtOnce { token, reason } => {
                log_parked(&op_id, Some(&prepared.file_id), reason);
                (token, None)
            }
            crate::state_db::AcceptOutcome::ParkedMoved => return Ok(None),
            crate::state_db::AcceptOutcome::UnknownItem => anyhow::bail!("a create always inserts its row"),
            crate::state_db::AcceptOutcome::InTrash => anyhow::bail!("a create is never refused as in the trash"),
            crate::state_db::AcceptOutcome::CopyGone => return Err(anyhow::Error::new(StagedCopyGone)),
            crate::state_db::AcceptOutcome::AcceptsClosed => return Err(anyhow::Error::new(AcceptsClosed)),
        };
        prepared.staged.retain();
        Ok(Some((
            FpWrite {
                outcome: FinderWriteOutcome::Queued {
                    op_id,
                    file_id: Some(prepared.file_id),
                    kind: OperationKind::UploadVersion,
                    ignored: false,
                    message: "queued for encrypted sync".to_string(),
                },
                token: Some(token),
                present_as: None,
            },
            released,
        )))
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
    /// from the server, as before. A write that has parked with its copy gone is the one
    /// exception: nothing local can serve it, so the server's version is fetched.
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
        // A copy that is gone at the open is decided once more. If the landing unlinked it, the
        // write is no longer queued and the server has its bytes (§7.4, 4b). A write that is
        // still queued is never fetched from the server, which holds only the previous version:
        // with its copy missing, the fetch fails (`QueuedCopyMissing`), and the system retries.
        // A write that has parked never leaves the queue by itself and
        // its copy cannot come back, so that retry would never end. Its fetch goes to the
        // server for the current version, and the item keeps the parked status (§9.1).
        let mut copy_missing = false;
        for _ in 0..2 {
            let Some((path, parked)) = self.db.queue_fetch_source(&file_id)? else {
                copy_missing = false;
                break;
            };
            self.seam("hydrate:before_open");
            match std::fs::File::open(&path) {
                Ok(mut staged) => {
                    if let Some(parent) = dest.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    let bytes = write_hydrated_from_reader(dest, allowed_roots, &mut staged)?;
                    // Recorded as the server path records it, so the sign-out sweep finds a
                    // copy written outside the hydrate cache.
                    self.record_hydrated_destination(&file_id, dest)?;
                    if let Some(progress) = progress {
                        progress(bytes, bytes);
                    }
                    log_hydrate_served(&file_id, HydrateSource::Queue);
                    return Ok(HydrateSource::Queue);
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound && parked => {
                    copy_missing = false;
                    break;
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    copy_missing = true;
                    continue;
                }
                Err(e) => return Err(e.into()),
            }
        }
        if copy_missing {
            return Err(anyhow::Error::new(QueuedCopyMissing));
        }
        self.hydrate_file_with_progress(&file_id, dest, allowed_roots, progress)
            .await?;
        log_hydrate_served(&file_id, HydrateSource::Server);
        Ok(HydrateSource::Server)
    }

    /// A Finder thumbnail (spec §5.6, §7.2): through the alias, and never while the item's
    /// held write is queued, nor after its landing until that version's thumbnails are
    /// uploaded (the safe default: a per-item error, and nothing asked of the server).
    #[cfg_attr(not(any(unix, test)), allow(dead_code))]
    pub async fn finder_thumbnail(&self, id: &str, variant: &str) -> anyhow::Result<Zeroizing<Vec<u8>>> {
        let (file_id, _) = self.resolve_provisional(id, "thumbnail")?;
        // The queue first, then the mark. A landing sets the mark before it commits the op's
        // removal and clears it only after the upload, so a request that finds the write no
        // longer queued finds the mark. Read the other way round, a landing between the two
        // reads served the previous version's thumbnail under the new token.
        let queued = self
            .db
            .item_presentation(&file_id)?
            .is_some_and(|presentation| presentation.held_write_queued);
        self.seam("thumbnail:between_reads");
        let marked = self.thumbnails_pending(&file_id);
        if queued || marked {
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
            // A landing writes an op's target path to the row: a sync-folder edit (task 1640)
            // names the file's full relative path, so a nested file stays in its folder.
            let target_path = target.rel_path.unwrap_or(target.filename);
            let outcome = self.enqueue_finder_operation(
                OperationKind::UploadVersion,
                Some(file_id),
                target.parent_id,
                Some(target_path),
                payload,
                Some(staged_path),
                base_version,
                item_contract
                    .as_ref()
                    .and_then(|contract| contract.current_object_version_id.clone()),
                None,
            )?;
            staged.retain();
            Ok(outcome)
        } else {
            let (kind, payload) = self.metadata_update_payload(&file_id, &target, item_contract.as_ref())?;
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
                None,
            )
        }
    }

    /// A metadata-only modify's kind and payload: a move is a change of parent, read against
    /// the row BEFORE the op changes it.
    fn metadata_update_payload(
        &self,
        file_id: &str,
        target: &FinderWriteTarget,
        item_contract: Option<&crate::state_db::FileContractState>,
    ) -> anyhow::Result<(OperationKind, serde_json::Value)> {
        let name_encrypted = owned_metadata_for_name(
            &self.db, self.api.master_key(),
            file_id,
            &target.filename,
            target.content_type.as_deref(),
        )?;
        // Spec §7.2 (final review): a move is a change of parent, read against the row, so a
        // move to the top level (no parent) is a move too, and a rename in a folder is a
        // rename. With no row, a parent means a move, as before.
        let moved = match self.db.get_file(file_id)? {
            Some(row) => row.parent_id != target.parent_id,
            None => target.parent_id.is_some(),
        };
        let kind = if moved {
            OperationKind::MoveFile
        } else {
            OperationKind::RenameFile
        };
        let mut payload = serde_json::json!({
            "operation": "metadata_update",
            "name_encrypted": name_encrypted,
            "base_version_identifier": target.base_version_identifier,
        });
        apply_shared_context(&mut payload, item_contract);
        Ok((kind, payload))
    }

    /// Task 2204 S0.1: a Windows rename or move of a tracked FILE, queued with its row moved to
    /// `new_path` in the same transaction (`StateDb::enqueue_local_rename`). The op is marked
    /// `local_rename`, which holds the row against stale landings and pulls until it is sent.
    pub fn queue_local_rename(&self, target: FinderWriteTarget, new_path: &str) -> anyhow::Result<FinderWriteOutcome> {
        let Some((file_id, item_contract)) = self.start_finder_modify(&target)? else {
            return Ok(ignored_finder_item(&target.filename));
        };
        let (kind, mut payload) = self.metadata_update_payload(&file_id, &target, item_contract.as_ref())?;
        let row = self.db.get_file(&file_id)?;
        payload["local_rename"] = serde_json::Value::Bool(true);
        // S0.4: where the file was, so a safe save can fold back onto it.
        payload["vacated_path"] = row.as_ref().map(|row| row.path.clone()).into();
        payload["vacated_parent_id"] = row.and_then(|row| row.parent_id).into();
        let mut op = self.finder_operation(
            kind.clone(),
            Some(file_id.clone()),
            target.parent_id.clone(),
            Some(target.filename.clone()),
            payload,
            None,
            None,
            item_contract.and_then(|contract| contract.current_object_version_id),
        )?;
        // S0.4: a safe save's rename, new file and delete arrive within this; until then the
        // rename is not sent, and a file at the vacated path waits.
        op.next_retry_at = now_secs() + RENAME_SETTLE_SECS;
        match self.db.enqueue_local_rename(&op, &file_id, new_path, target.parent_id.as_deref())? {
            crate::state_db::FinderEnqueue::AcceptsClosed => Err(anyhow::Error::new(AcceptsClosed)),
            crate::state_db::FinderEnqueue::Queued => Ok(FinderWriteOutcome::Queued {
                op_id: op.op_id,
                file_id: Some(file_id),
                kind,
                ignored: false,
                message: "queued for encrypted sync".to_string(),
            }),
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
        let name_encrypted = owned_metadata_for_name(&self.db, self.api.master_key(), file_id, &target.filename, mime)?;
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
                return Err(anyhow::Error::new(UnknownItem { requested_id }));
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
                retire_parked: None,
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
            // (§7.3). The staged copy is dropped with `prepared`; nothing was queued. The
            // request named the provisional id it is presented under, else this one.
            crate::state_db::AcceptOutcome::UnknownItem => {
                return Err(anyhow::Error::new(UnknownItem {
                    requested_id: present_as.unwrap_or(file_id),
                }));
            }
            crate::state_db::AcceptOutcome::CopyGone => return Err(anyhow::Error::new(StagedCopyGone)),
            crate::state_db::AcceptOutcome::AcceptsClosed => return Err(anyhow::Error::new(AcceptsClosed)),
            // §7.2: refused; the staged copy is dropped with `prepared`, and nothing was queued.
            crate::state_db::AcceptOutcome::InTrash => return Err(anyhow::Error::new(ItemInTrash)),
            // A modify retires no parked write (task 2190's "Keep both" is a create).
            crate::state_db::AcceptOutcome::ParkedMoved | crate::state_db::AcceptOutcome::QueuedRetiring { .. } => {
                anyhow::bail!("a modify retires no parked write")
            }
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
                // The id is the request's, from the wire (spec §11 M5).
                let (file_id, id_kind) = wire_id_fields(file_id);
                tracing::info!(file_id, id_kind, "finder delete: unknown item — idempotent success");
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
            None,
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
        match self.classify_local_path_verdict(sync_root, path) {
            LocalPathVerdict::Write(target) => Some(target),
            LocalPathVerdict::Settled | LocalPathVerdict::Skip => None,
        }
    }

    /// Task 2229: [`Self::classify_local_path`], telling a tracked file that needs nothing while its row
    /// and its bytes stay as they are ([`LocalPathVerdict::Settled`]) from one to ask about again.
    pub(crate) fn classify_local_path_verdict(&self, sync_root: &Path, path: &Path) -> LocalPathVerdict {
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
                return LocalPathVerdict::Skip;
            }
            Err(e) => {
                tracing::debug!(
                    path = %path.display(),
                    error = %e,
                    "classify_local_path: stat failed; skipping"
                );
                return LocalPathVerdict::Skip;
            }
        }

        // Filter 1 — engine-internal paths + OS junk.
        if path_is_engine_internal(sync_root, path) {
            tracing::debug!(
                path = %path.display(),
                "classify_local_path: rejected engine-internal path"
            );
            return LocalPathVerdict::Skip;
        }
        let Some(file_name) = path.file_name().and_then(|n| n.to_str()).map(str::to_string) else {
            tracing::debug!(
                path = %path.display(),
                "classify_local_path: rejected path with non-utf8 filename"
            );
            return LocalPathVerdict::Skip;
        };
        if is_ignored_finder_name(&file_name) {
            tracing::debug!(
                path = %path.display(),
                filename = %file_name,
                "classify_local_path: rejected ignored/temp filename"
            );
            return LocalPathVerdict::Skip;
        }

        // Filter 3 (authoritative) — already a known server file? Then it is not a new
        // upload. Task 1640: it may be an edit of that file; `classify_tracked_edit` decides
        // from the content this device last agreed with the server on, before filter 2, which
        // rejects every placeholder (a synced file is one).
        let Some(rel) = relative_db_path(sync_root, path) else {
            tracing::debug!(
                path = %path.display(),
                sync_root = %sync_root.display(),
                "classify_local_path: rejected path outside sync root"
            );
            return LocalPathVerdict::Skip;
        };
        // Task 1640 (review N13): an original whose kept copy is made but which is not removed
        // yet (it was in use) is never sent: a later tick removes it.
        if self.db.is_kept_original_pending(&rel).unwrap_or(false) {
            tracing::debug!(rel_path = %rel, "classify_local_path: rejected an original whose copy is kept");
            return LocalPathVerdict::Skip;
        }
        // Task 2204 S0.4: a file at the path a Windows rename vacated waits until that rename is
        // sent: it may be a safe save's new bytes, which fold back onto the original.
        if self.db.is_vacated_by_pending_rename(&rel).unwrap_or(true) {
            tracing::debug!(rel_path = %rel, "classify_local_path: the path a pending rename vacated; skipped this pass");
            return LocalPathVerdict::Skip;
        }
        match self.row_judging_bytes_at(&rel, path) {
            Ok(RowAtPath::One(existing)) => return self.classify_tracked_edit(path, existing, file_name, rel),
            Ok(RowAtPath::Ambiguous { file_ids, rows }) => {
                // Lead ruling P2 (2271 contract §7): file ids and the row count, never the path.
                tracing::warn!(
                    ?file_ids,
                    rows,
                    "sync folder: several synced files share one path and its bytes are none of them; kept on this PC, not sent"
                );
                return LocalPathVerdict::Settled;
            }
            Ok(RowAtPath::NotWhollyOnDisk) => {
                tracing::debug!(rel_path = %rel, "classify_local_path: several rows at a path not wholly on disk; asked again later");
                return LocalPathVerdict::Skip;
            }
            Ok(RowAtPath::None) => { /* not tracked — filter 2, then new */ }
            Err(e) => {
                tracing::warn!(
                    path = %path.display(),
                    rel_path = %rel,
                    error = %e,
                    "classify_local_path: could not tell which row the path's bytes belong to (a state DB lookup or a read of the bytes failed); skipping to be safe"
                );
                return LocalPathVerdict::Skip;
            }
        }

        // Filter 2 — Cloud Files placeholders are engine-owned (Windows only). An
        // untracked reparse point under the sync root is a placeholder we minted or
        // converted whose row is not written yet; the hydration write that fills it
        // does NOT turn it back into a plain file, so we must never treat a
        // placeholder write as a new upload.
        #[cfg(target_os = "windows")]
        if crate::windows_cf::placeholders::is_cloud_placeholder(path) {
            // Task 2191 (D12): an untracked placeholder may be one a remote rename left behind,
            // holding unknown bytes; the tick's retry checks its identity.
            if let Err(e) = self.db.put_unknown_orphan(&rel, "", true) {
                tracing::debug!(error = %e, "classify_local_path: could not record an untracked placeholder");
            }
            tracing::debug!(
                path = %path.display(),
                "classify_local_path: rejected Cloud Files placeholder"
            );
            return LocalPathVerdict::Skip;
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

        LocalPathVerdict::Write(FinderWriteTarget {
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

    /// Task 2190 (the lead's condition on E3; glegion's constraints from #167's scan memo): the ONE
    /// call each held branch of [`Self::classify_tracked_edit`] makes. Records that `file_id`'s edit
    /// is held, not sent, with the file's content fingerprint, so the review feed shows it. Nothing
    /// is written while neither the reason nor the fingerprint changed, and the `files` row is never
    /// touched (the memo skips a settled file while that row is unchanged). A failure is logged: the
    /// edit stays held either way.
    /// Task 2244: a hold that writes is logged at DEBUG and counted; the scan logs the count once a
    /// pass, not a line per file (28,224 on every start of 0.8.17 on the maintainer's PC).
    fn hold_unsent_edit(&self, file_id: &str, path: &Path, reason: &str) {
        let fingerprint = ContentFingerprint::of(path).map(|fp| format!("{fp:?}")).unwrap_or_default();
        match self.db.hold_unsent_edit(file_id, reason, &fingerprint, now_secs()) {
            Ok(true) => {
                tracing::debug!(file_id = %file_id, reason, "sync folder: an edit of a synced file is kept on this PC, not sent");
                self.newly_held[usize::from(reason == HELD_BASE_UNKNOWN)].fetch_add(1, Ordering::Relaxed);
            }
            Ok(false) => {}
            Err(e) => tracing::warn!(file_id = %file_id, error = %e, "sync folder: could not record a held edit"),
        }
    }

    /// Task 2244: the edits newly held since the last call, for the scan's one line a pass:
    /// (content unknown, base unknown).
    pub(crate) fn take_newly_held(&self) -> (usize, usize) {
        let [content_unknown, base_unknown] = &self.newly_held;
        (
            content_unknown.swap(0, Ordering::Relaxed),
            base_unknown.swap(0, Ordering::Relaxed),
        )
    }

    /// Task 2190: the one call that ends a held edit's record, when the file reads as agreed, is
    /// queued, or can be sent on a known base.
    fn release_unsent_edit(&self, file_id: &str) {
        if let Err(e) = self.db.release_unsent_edit(file_id) {
            tracing::warn!(file_id = %file_id, error = %e, "sync folder: could not clear a held edit");
        }
    }

    /// Task 2271: the row the scan judges the bytes at `rel` by. Normally the one row there. When
    /// several share the path (0.8.18's reconcile pass left the server's file of that name and the
    /// copy this PC sent at one path), the row that records content this PC agreed on decides: the
    /// only such row, or the one whose agreed content the bytes are. With no such row it is the
    /// path's first, as before. With several and bytes that are none of them, no row is guessed.
    /// Choosing among several reads the bytes once, and only when they are wholly on this PC.
    fn row_judging_bytes_at(&self, rel: &str, path: &Path) -> anyhow::Result<RowAtPath> {
        let mut rows = self.db.file_rows_at_path(rel)?;
        if rows.len() < 2 {
            return Ok(rows.pop().map_or(RowAtPath::None, RowAtPath::One));
        }
        let mut recorded = Vec::new();
        for (at, row) in rows.iter().enumerate() {
            if row.is_dir() {
                continue;
            }
            let agreed = match self.db.get_file_contract_state(&row.file_id)?.and_then(|c| c.local_hash) {
                Some(hash) => Some(Some(hash)),
                None => self.db.fetched_content(&row.file_id)?.map(|(_, hash)| hash),
            };
            if let Some(agreed) = agreed {
                recorded.push((at, agreed));
            }
        }
        let at = match recorded[..] {
            [] => 0,
            [(at, _)] => at,
            [(first, _), ..] => {
                // Windows: reading a placeholder that is not wholly on disk would fetch the rest of
                // it (as in `classify_tracked_edit`); the scan asks again once it is.
                #[cfg(target_os = "windows")]
                if !crate::windows_cf::placeholders::fully_on_disk(path) {
                    return Ok(RowAtPath::NotWhollyOnDisk);
                }
                let bytes = cached_sha256_file_hex(&rows[first].file_id, path)?;
                // A mixed fetch (no single version) is never agreed.
                let matching: Vec<usize> = recorded
                    .iter()
                    .filter(|(_, agreed)| agreed.as_deref() == Some(bytes.as_str()))
                    .map(|(at, _)| *at)
                    .collect();
                match matching[..] {
                    [at] => at,
                    _ => {
                        let file_ids = recorded.iter().map(|(at, _)| rows[*at].file_id.clone()).collect();
                        return Ok(RowAtPath::Ambiguous { file_ids, rows: rows.len() });
                    }
                }
            }
        };
        Ok(RowAtPath::One(rows.swap_remove(at)))
    }

    /// Task 1640: a close or scan of a tracked sync-folder file is an edit when the file is a
    /// synced (`Local`) file and its bytes differ from the content this device last agreed with
    /// the server on. The edit is a content modify of that file, sent with the version its bytes
    /// are based on, so the server refuses it as stale when the file changed elsewhere meanwhile
    /// (it then parks with its bytes). Anything else is not an upload: a folder, a row in
    /// transfer or in conflict, agreed bytes, an upload already queued, no agreed content
    /// recorded, or an edit whose base is not known (never sent without one: it would replace
    /// the server's version unseen).
    fn classify_tracked_edit(
        &self,
        path: &Path,
        existing: FileEntry,
        file_name: String,
        rel: String,
    ) -> LocalPathVerdict {
        if existing.is_dir() || existing.status != FileStatus::Local {
            tracing::debug!(
                path = %path.display(),
                rel_path = %rel,
                "classify_local_path: rejected already-tracked server file"
            );
            return LocalPathVerdict::Settled;
        }
        // Windows: reading a placeholder that is not wholly on disk would fetch the rest of it.
        #[cfg(target_os = "windows")]
        let os_in_sync = {
            if !crate::windows_cf::placeholders::fully_on_disk(path) {
                return LocalPathVerdict::Skip;
            }
            crate::windows_cf::placeholders::placeholder_in_sync(path)
        };
        #[cfg(not(target_os = "windows"))]
        let os_in_sync = None;
        let base = match sync_folder_content(&self.db, &existing.file_id, path, os_in_sync) {
            Ok(SyncFolderContent::Changed { base: Some(base) }) => {
                self.release_unsent_edit(&existing.file_id);
                base
            }
            Ok(SyncFolderContent::Changed { base: None }) => {
                self.hold_unsent_edit(&existing.file_id, path, HELD_BASE_UNKNOWN);
                // Task 2229: held until its row records a base; no change of its bytes sends it.
                return LocalPathVerdict::Settled;
            }
            // A file hydrated before this build has no agreed content: its change is kept on
            // disk (out of sync, so never stamped or freed) but not sent (task 2191).
            // Task 2190: not reported in sync (Windows), or not a placeholder (an atomic save left a
            // plain file; and every build without Cloud Files) may hold an edit.
            Ok(SyncFolderContent::Unknown) if os_in_sync != Some(true) => {
                self.hold_unsent_edit(&existing.file_id, path, HELD_CONTENT_UNKNOWN);
                return LocalPathVerdict::Settled;
            }
            // Task 2229: unknown content is held until its row records the agreed content, whatever
            // its bytes and its in-sync state; the scan remembers it like agreed bytes.
            Ok(SyncFolderContent::Agreed | SyncFolderContent::Unknown) => {
                self.release_unsent_edit(&existing.file_id);
                return LocalPathVerdict::Settled;
            }
            Ok(SyncFolderContent::Queued) => {
                self.release_unsent_edit(&existing.file_id);
                return LocalPathVerdict::Skip;
            }
            Err(e) => {
                tracing::warn!(file_id = %existing.file_id, error = %e, "sync folder: could not compare a synced file with its agreed content");
                return LocalPathVerdict::Skip;
            }
        };
        let contract = self.db.get_file_contract_state(&existing.file_id).ok().flatten();
        tracing::debug!(
            file_id = %existing.file_id,
            base_version = base,
            "classify_local_path: accepted an edit of a synced file"
        );
        LocalPathVerdict::Write(FinderWriteTarget {
            file_id: Some(existing.file_id),
            parent_id: contract.as_ref().and_then(|contract| contract.parent_id.clone()),
            content_type: contract
                .and_then(|contract| contract.content_type)
                .or_else(|| beebeeb_core::media::guess_mime_type(&file_name).map(str::to_string)),
            filename: file_name,
            rel_path: Some(rel),
            kind: FinderWriteItemKind::File,
            contents_path: Some(path.to_string_lossy().into_owned()),
            base_version_identifier: Some(base.to_string()),
        })
    }

    /// Task 1640 (review N1b): the row's version (`None` when unknown), the version a legacy
    /// download records. Accepted for legacy files only: it can say less than the bytes are
    /// (a later change then parks as stale), never more.
    fn legacy_download_version(&self, file_id: &str) -> anyhow::Result<Option<i64>> {
        Ok(self
            .db
            .get_file_contract_state(file_id)?
            .map(|contract| contract.current_version)
            .filter(|version| *version > 0))
    }

    /// Task 1640: whether `dest` is `file_id`'s own place in the sync folder, under one of
    /// `allowed_roots`, rather than a staging copy (a Finder fetch, a Linux FUSE temp file).
    fn dest_is_files_sync_folder_place(&self, file_id: &str, dest: &Path, allowed_roots: &[&Path]) -> anyhow::Result<bool> {
        let Some(entry) = self.db.get_file(file_id)? else {
            return Ok(false);
        };
        Ok(allowed_roots.iter().any(|root| {
            local_file_path_under_sync_root(root, &entry.path).is_ok_and(|place| place.as_path() == dest)
        }))
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

        // Task 1640 (review F3): a folder deleted elsewhere that stays on disk only for what this PC
        // keeps (an original still open, a file partly on disk) is never created again on the
        // server; one the person put something new into is wanted again.
        match self.db.outermost_removed_folder(&rel) {
            Ok(Some(outermost)) => {
                let held = Self::placeholder_path_under(sync_root, &outermost)
                    .is_some_and(|dir| held_only_for_kept(&self.db, sync_root, &dir, 0));
                if held {
                    tracing::debug!("ensure_local_folder: a folder deleted elsewhere, kept for what it holds; not created again");
                    return None;
                }
                let _ = self.db.forget_removed_folder(&outermost);
            }
            Ok(None) => {}
            Err(e) => {
                tracing::warn!(error = %e, "ensure_local_folder: kept-folder lookup failed; skipping");
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
    #[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
    pub(crate) fn placeholder_path_under(sync_root: &Path, rel_path: &str) -> Option<PathBuf> {
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
        self.enforce_local_cache_limit_after_guard(max_local_cache_bytes, &crate::unknown_check::STARTUP_GUARD)
    }

    /// Desktop 0.8.15: [`Self::enforce_local_cache_limit`], which evicts nothing until `guard`'s
    /// start-up pass has walked every file of unknown content (the pass runs beside the engine).
    pub(crate) fn enforce_local_cache_limit_after_guard(
        &self,
        max_local_cache_bytes: Option<i64>,
        guard: &crate::unknown_check::StartupGuard,
    ) -> anyhow::Result<CacheCleanupOutcome> {
        if !guard.allows_freeing() {
            return Ok(CacheCleanupOutcome {
                evicted_file_ids: Vec::new(),
            });
        }
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

    #[allow(clippy::too_many_arguments)]
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
        folder: Option<crate::state_db::FinderFolderRow<'_>>,
    ) -> anyhow::Result<FinderWriteOutcome> {
        let op = self.finder_operation(
            kind.clone(),
            file_id.clone(),
            parent_id,
            target_path,
            metadata,
            payload_path,
            base_version,
            base_object_version_id,
        )?;
        let op_id = op.op_id.clone();
        // Spec 2026-10-10 §5.2 step 6: the sign-out's fence is read in the transaction that queues the op, and a
        // folder's row is written in it too.
        match self.db.enqueue_finder_change(&op, folder)? {
            crate::state_db::FinderEnqueue::AcceptsClosed => Err(anyhow::Error::new(AcceptsClosed)),
            crate::state_db::FinderEnqueue::Queued => Ok(FinderWriteOutcome::Queued {
                op_id,
                file_id,
                kind,
                ignored: false,
                message: "queued for encrypted sync".to_string(),
            }),
        }
    }

    /// The op a Finder or sync-folder write queues (`enqueue_finder_operation`'s first half).
    #[allow(clippy::too_many_arguments)]
    fn finder_operation(
        &self,
        kind: OperationKind,
        file_id: Option<String>,
        parent_id: Option<String>,
        target_path: Option<String>,
        metadata: serde_json::Value,
        payload_path: Option<String>,
        base_version: Option<i64>,
        base_object_version_id: Option<String>,
    ) -> anyhow::Result<PendingOperation> {
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
        Ok(op)
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
        // Task 1640: a hydrate into the file's own place in the sync folder (on Windows, only
        // conflict resolution reaches it) never writes over an unsent local change: bytes that differ from the content this
        // device last agreed with the server on, or whose upload is queued, are the person's
        // only copy. It refuses, and the file keeps its bytes. A Finder fetch (macOS) writes
        // to the extension's own staging copy, never there.
        let in_sync_folder = self.dest_is_files_sync_folder_place(file_id, dest_path, allowed_roots)?;
        // Task 1640 (review N1b): the version a legacy download (no `download_version`) records,
        // read BEFORE the download: at most the version the bytes are, so a later change can only
        // be refused as stale, never accepted over a newer one.
        let legacy_version = self.legacy_download_version(file_id)?;
        if in_sync_folder && dest_path.exists() {
            let content = sync_folder_content(&self.db, file_id, dest_path, None)?;
            // Task 2191 (G-engine): a file of unknown content (in the population, conflicts and
            // errors excepted: their resolution is the person's) is never written over while its
            // place holds bytes; they may be the person's only copy of a change.
            // Review-157 F3: Windows only (elsewhere nothing marks a row empty, and a hydrate never
            // writes in place).
            let unknown_with_bytes = cfg!(any(target_os = "windows", test))
                && matches!(content, SyncFolderContent::Unknown)
                && self.db.is_unknown_population(file_id)?
                && place_holds_bytes(dest_path);
            if unknown_with_bytes || matches!(content, SyncFolderContent::Changed { .. } | SyncFolderContent::Queued) {
                tracing::warn!(file_id = %file_id, "hydrate refused: the local file holds a change that is not sent");
                return Err(anyhow::Error::new(UnsentLocalChange));
            }
        }
        // Spec §7.4.2: a hydrate never changes the status of a row with a live upload of its
        // own (a Finder upload that has not parked): no `Downloading`, `Local` or `Error`.
        // That status is the upload's, and `Error` would make the item read-only. Nor
        // of a row whose Finder upload has parked. Its status is the
        // parked one (§9.1), and `Local` would make it writable and evictable.
        let touch_status = !self
            .db
            .item_presentation(file_id)?
            .is_some_and(|presentation| presentation.unparked_finder_upload || presentation.parked_finder_upload);
        // Every status the hydrate writes after its `Downloading` is written only over that
        // `Downloading`: a save accepted meanwhile has made the row `Uploading`, and its status
        // stays (spec §9.2). Before, the hydrate's end wrote over it.
        let set_failed = || {
            if touch_status {
                // Best-effort status flip; if the DB is broken we still
                // return the original error.
                let _ = self
                    .db
                    .set_status_if(file_id, FileStatus::Downloading, FileStatus::Error);
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
        let hydrated = self.do_hydrate_versioned(file_id, progress).await;
        // Past the only cancellation point (the download await): every path
        // below sets the final status itself.
        if let Some(guard) = downloading_guard.as_mut() {
            guard.disarm();
        }
        match hydrated {
            Ok((mut buf, downloaded_version)) => {
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
                let hydrated_hash = in_sync_folder.then(|| sha256_hex(&buf));
                // Zeroize the in-memory copy now that it is on disk (or on
                // error) so the allocation does not linger with plaintext.
                buf.zeroize();
                if let Err(e) = write_result {
                    set_failed();
                    return Err(anyhow::anyhow!("write {}: {e}", dest_path.display()));
                }
                // Task 1640: the bytes just written (one atomic rename: the whole file) are the
                // content this device agrees with the server on, at the version the download
                // metadata named, else (a legacy download, review N1b) the row's version read
                // before the download. Recorded before the row leaves `Downloading`, so no
                // classification sees the new bytes against the old baseline. With no version
                // either, the content is not known (review F1), rather than left naming other bytes.
                if let Some(hash) = hydrated_hash.as_deref() {
                    let recorded = match downloaded_version.or(legacy_version) {
                        Some(version) => self.db.record_local_baseline(file_id, version, hash),
                        None => self.db.mark_content_unknown(file_id),
                    };
                    if let Err(e) = recorded {
                        set_failed();
                        return Err(e.into());
                    }
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
                self.seam("hydrate:before_final_status");
                if let Some(guard) = &downloading_guard {
                    self.db.set_status_if(
                        file_id,
                        FileStatus::Downloading,
                        hydrate_final_status(guard.restore.clone()),
                    )?;
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
                self.record_hydrated_destination(file_id, dest_path)?;
                #[cfg(target_os = "linux")]
                self.write_linux_freedesktop_thumbnails(file_id, dest_path).await;
                let _ = self.enforce_configured_cache_limit();
                Ok(())
            }
            Err(e) => {
                let e = self.owned_open_failure(file_id, e).await;
                set_failed(); // only writes over Downloading; the recoverable item already left it.
                Err(e)
            }
        }
    }

    /// Record where a hydrate wrote `file_id`'s plaintext, for the cache accounting and the
    /// sign-out sweep. Both hydrate paths, the server's and the queue's, call it. On macOS a
    /// Finder hand-off into the hydrate cache is not a cached copy ([`record_hydration_cache_state`]).
    fn record_hydrated_destination(&self, file_id: &str, dest_path: &Path) -> anyhow::Result<()> {
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
        Ok(())
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
    pub async fn hydrate_file_to_memory(&self, file_id: &str) -> anyhow::Result<FetchedBytes> {
        // Self-review #1: counted from before anything is recorded until the caller's transfer ended.
        let ticket = self.begin_fetch(file_id);
        let legacy_version = self.legacy_download_version(file_id)?;
        self.db.set_status(file_id, FileStatus::Downloading)?;
        match self.do_hydrate_versioned(file_id, None).await {
            Ok((buf, downloaded_version)) => {
                // Task 1640 (review N1, F1): every fetch, of a range or of the whole file, is
                // recorded BEFORE its bytes are transferred: the version it serves and the hash of
                // all of that version's bytes join the file's candidate (`record_fetched_version`),
                // so a transfer that fails or is cut off midway still counts. Only once the whole
                // file is transferred does the caller make it the agreed content
                // ([`Self::record_whole_fetch`]); a range never does: the placeholder's other
                // ranges may hold other versions, or writes that were not sent. The stamp pass
                // promotes a candidate once the whole file is on disk, in sync, with exactly those
                // bytes. A download that names no version leaves the content not known. A failure
                // to record fails the fetch, as every other failure here does.
                let hash = sha256_hex(&buf);
                let fetched = downloaded_version.or(legacy_version).map(|version| FetchedVersion { version, hash });
                let recorded = match &fetched {
                    Some(fetched) => self.db.record_fetched_version(file_id, fetched.version, &fetched.hash),
                    None => self.db.mark_content_unknown(file_id),
                };
                if let Err(e) = recorded {
                    let _ = self.db.set_status(file_id, FileStatus::Error);
                    return Err(e.into());
                }
                self.db.set_status(file_id, FileStatus::Local)?;
                // cache_path is empty: on Windows CF the hydrated bytes live
                // INSIDE the placeholder (the CF runtime writes them there after
                // our CfExecute(TRANSFER_DATA) calls). There is no separate temp
                // file. Byte count is still recorded for smart-cache accounting.
                self.db.mark_cached(file_id, "", buf.len() as i64, now_secs())?;
                let _ = self.enforce_configured_cache_limit();
                Ok(FetchedBytes { bytes: buf, version: fetched, ticket })
            }
            Err(e) => {
                let e = self.owned_open_failure(file_id, e).await;
                let _ = self.db.set_status_if(file_id, FileStatus::Downloading, FileStatus::Error);
                Err(e)
            }
        }
    }

    /// Task 1640 (review F1, design T5): a fetch transferred the whole file (`fetched`, from
    /// [`Self::hydrate_file_to_memory`]): every byte on disk is that version's now, so they are the
    /// content this device agrees with the server on, and nothing else fetched is left. Only when no
    /// other fetch of the file overlapped this one (`ticket`, self-review #1): another fetch's bytes,
    /// of another version, could land around this transfer. Then the candidate this fetch merged
    /// stays (lower or mixed). `Ok(true)`: recorded.
    #[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
    pub fn record_whole_fetch(&self, file_id: &str, fetched: &FetchedVersion, ticket: &FetchTicket) -> anyhow::Result<bool> {
        // Review-151d P3-4: the check and the write under the tracker's lock, so no fetch can begin
        // (and record) between them. Lock order: the tracker, then the database; nothing takes the
        // tracker while holding the database.
        let fetches = FETCHES_IN_FLIGHT.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if !ticket.alone_in(&fetches) {
            tracing::debug!(file_id = %file_id, "whole fetch overlapped by another fetch of the file; its candidate stays");
            return Ok(false);
        }
        self.db.record_local_baseline(file_id, fetched.version, &fetched.hash)?;
        drop(fetches);
        Ok(true)
    }

    /// Task 1640 (self-review #1): a fetch of `file_id` starts ([`FetchTicket`]).
    #[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
    fn begin_fetch(&self, file_id: &str) -> FetchTicket {
        let mut fetches = FETCHES_IN_FLIGHT.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let activity = fetches.entry(file_id.to_string()).or_default();
        activity.in_flight += 1;
        activity.events += 1;
        let alone_since = (activity.in_flight == 1).then_some(activity.events);
        FetchTicket {
            file_id: file_id.to_string(),
            alone_since,
        }
    }

    /// Core download + decrypt loop. Returns the full plaintext in a
    /// [`Zeroizing`] wrapper so the allocation is wiped on drop regardless of
    /// which exit path is taken (normal return, early `?`, or a panic unwind).
    ///
    /// Intermediate per-chunk buffers from core are
    /// explicitly zeroized after being copied into the accumulator, so at most
    /// one extra chunk's worth of plaintext is live in memory at any time.
    ///
    /// This function deliberately does NOT write anything to disk. Callers that
    /// need the bytes on disk (`hydrate_file`) or in memory (`hydrate_file_to_memory`)
    /// handle that themselves so the disk-write decision stays at the call site,
    /// not buried inside the crypto loop.
    pub async fn encrypt_owned_name(&self, file_id: &str, name: &str, pin: beebeeb_core::chunk_stream::ExpectedChunkFormat) -> anyhow::Result<String> {
        let key = self.resolved_owned_key(file_id).await?;
        Ok(beebeeb_core::encrypt::encrypt_name_for(&key, name, None, pin)?)
    }

    async fn owned_open_failure(&self, file_id: &str, error: anyhow::Error) -> anyhow::Error {
        if let Ok(Some(record)) = self.db.get_owned_key_record(file_id)
            && record.foreign_added {
            let _ = self.db.set_status_if(file_id, FileStatus::Downloading, FileStatus::CloudOnly);
            let label = if let (Some(root), Some(encrypted)) = (record.shared_root_id.as_deref(), record.origin_label_encrypted.as_deref()) {
                // Display only, read-only custody lookup. Never re-seed custody during an open failure.
                let scope = self.owner_key_scope();
                let custody = self.api.get_shared_folder(root).await.ok();
                if let Err(error) = scope.check() { return error; }
                let key = custody.and_then(|row| crate::owner_keys::normalize_wrap(&row["owner_wrapped_folder_key"]).ok().flatten())
                    .and_then(|wrap| beebeeb_sync::key_resolution::unwrap_folder_custody(
                        &beebeeb_core::kdf::MasterKey::from_bytes(*self.api.master_key()), root, &wrap).ok());
                if let Some(key) = key {
                    if let Err(error) = scope.remember_custody(root) { return error; }
                    beebeeb_core::encrypt::decrypt_file_metadata_with_key(&key, encrypted).ok().map(|metadata| metadata.name)
                } else { None }
            } else { None };
            return anyhow::anyhow!(crate::owner_keys::cannot_open_message(label.as_deref(), record.member_added));
        }
        error
    }

    async fn file_key_for_download(&self, file_id: &str) -> anyhow::Result<beebeeb_core::kdf::FileKey> {
        let Some(contract) = self.db.get_file_contract_state(file_id)? else {
            return self.resolved_owned_key(file_id).await.map(|key| beebeeb_core::kdf::FileKey::from_bytes(*key.as_bytes()));
        };
        if contract.namespace != Namespace::SharedWithMe {
            return self.resolved_owned_key(file_id).await.map(|key| beebeeb_core::kdf::FileKey::from_bytes(*key.as_bytes()));
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
            let (folder_key, folder_keys) = self.shared_folder_material(&mapping).await?;
            let wrap = encrypted_file_key_from_folder_keys_response(&folder_keys, file_id)
                .ok_or_else(|| anyhow::anyhow!("folder share is missing the child key"))?;
            self.shared_folder_child_key(&folder_key, &mapping.file_id, file_id, 1, wrap,
                folder_keys["desktop_has_custody"].as_bool() == Some(true))
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
        self.do_hydrate_versioned(file_id, progress).await.map(|(plaintext, _)| plaintext)
    }

    /// [`Self::do_hydrate`], also saying which server version the bytes are: the download
    /// metadata's `download_version.version_number` (task 1640), `None` for a legacy file whose
    /// metadata names no version.
    async fn do_hydrate_versioned(
        &self,
        file_id: &str,
        progress: Option<&HydrateProgressFn>,
    ) -> anyhow::Result<(Zeroizing<Vec<u8>>, Option<i64>)> {
        let _file_uuid: uuid::Uuid = file_id
            .parse()
            .map_err(|e| anyhow::anyhow!("invalid file_id (not a UUID): {e}"))?;

        // Per-file metadata. We trust the server's chunk_count rather
        // than the local entry's because a file we know about as
        // `cloud_only` may have been re-uploaded with a new chunk
        // layout since we last saw it.
        let meta = self.api.get_file(file_id).await?;
        self.with_one_restart(file_id, meta, |meta| async move {
            let version = meta["download_version"]["version_number"].as_i64();
            self.do_hydrate_with_meta(file_id, &meta, progress)
                .await
                .map(|plaintext| (plaintext, version))
        })
        .await
    }

    /// Tasks 1643 + 2173: run one download `attempt` on `meta`, and when the version it was bound to went away
    /// ([`crate::download_pin::VersionGone`]) run it ONCE more on fresh metadata; nothing of the first attempt is kept,
    /// and versions are never mixed. The restart keeps the binding: when the first attempt was pinned and the fresh
    /// metadata names no version (`download_version: null`), it is [`crate::download_pin::PinLost`], never an unpinned
    /// download. The one copy of this rule (whole-file hydrate, range read, conflict preview).
    async fn with_one_restart<T, F, Fut>(&self, file_id: &str, meta: serde_json::Value, attempt: F) -> anyhow::Result<T>
    where
        F: Fn(serde_json::Value) -> Fut,
        Fut: std::future::Future<Output = anyhow::Result<T>>,
    {
        let first_was_pinned = crate::download_pin::PinnedVersion::from_metadata(&meta)?.is_some();
        match attempt(meta).await {
            Err(e) if e.downcast_ref::<crate::download_pin::VersionGone>().is_some() => {
                tracing::debug!("download: the file changed while it was downloading; restarting once");
                let fresh = self.api.get_file(file_id).await?;
                if first_was_pinned && crate::download_pin::PinnedVersion::from_metadata(&fresh)?.is_none() {
                    return Err(anyhow::Error::new(crate::download_pin::PinLost));
                }
                attempt(fresh).await
            }
            other => other,
        }
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
        self.hydrate_whole_file(file_id, meta, progress)
            .await
            .map(|(plaintext, _)| plaintext)
    }

    /// [`Self::do_hydrate_with_meta`], also saying which transport carried the file. Task 2173:
    /// straight from object storage when the server offers it and the handshake succeeds
    /// ([`crate::direct_download`]); `UseApi` means nothing of that attempt was kept, and the file
    /// goes through the API's per-chunk route below, as it always did.
    ///
    /// Task 1643: when the metadata names a `download_version`, both paths are bound to it: every
    /// API chunk request carries `?object_version_id=`, the direct path must land on the same
    /// version, each chunk's wire size and the final plaintext size are checked against its
    /// manifest, and a version that goes away is [`crate::download_pin::VersionGone`] (the caller
    /// starts again from fresh metadata). Without one (a legacy file), today's unpinned download.
    async fn hydrate_whole_file(
        &self,
        file_id: &str,
        meta: &serde_json::Value,
        progress: Option<&HydrateProgressFn>,
    ) -> anyhow::Result<(Zeroizing<Vec<u8>>, crate::direct_download::FileTransport)> {
        let pin = crate::download_pin::PinnedVersion::from_metadata(meta)?;
        let chunk_count = match &pin {
            Some(pin) => pin.chunk_count,
            None => meta
                .get("chunk_count")
                .and_then(|v| v.as_i64())
                .ok_or_else(|| anyhow::anyhow!("server response missing chunk_count"))? as u32,
        };

        let file_key = self.file_key_for_download(file_id).await?;
        let expected_format = recorded_content_format(&file_key, meta)?;

        // Rate-limit ceiling for downloads, and the transport (task 2173).
        let config = crate::config::DesktopConfig::load().ok();
        let download_kbps_limit = config.as_ref().map(|c| c.download_kbps_limit).unwrap_or(0);
        let transport = self.download_transport(config.as_ref().and_then(|c| c.transport.as_deref()));

        // Walk chunks. Pre-allocate roughly the file size if known,
        // but fall back to defaults — chunks are encrypted so the
        // ciphertext is always larger than plaintext anyway.
        // Use Zeroizing so that if we bail mid-loop (network error,
        // decrypt error) the partial plaintext is still wiped.
        let approx_size = match &pin {
            Some(pin) => usize::try_from(pin.plaintext_size_bytes).unwrap_or(0),
            None => meta.get("size_bytes").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
        };

        let report = HydrateReport {
            bridge: self,
            file_id,
            total: approx_size as u64,
            progress,
        };
        let direct = crate::direct_download::download(
            &self.api,
            crate::direct_download::Request {
                file_id,
                file_key: &file_key,
                chunk_count,
                expected_format,
                download_kbps_limit,
                approx_size,
                pin: pin.as_ref(),
            },
            transport,
            self.storage_override(),
            &report,
        )
        .await
        .map_err(|e| match e {
            crate::direct_download::DirectError::VersionChanged => anyhow::Error::new(crate::download_pin::VersionGone),
            other => anyhow::Error::new(other),
        })?;
        let (plaintext, transport) = match direct {
            crate::direct_download::Outcome::Done { plaintext, transport } => (plaintext, transport),
            crate::direct_download::Outcome::UseApi => (
                self.download_and_decrypt_chunk_range(
                    file_id,
                    &file_key,
                    0..chunk_count,
                    download_kbps_limit,
                    approx_size,
                    progress,
                    pin.as_ref(),
                    expected_format,
                    chunk_count,
                )
                .await?,
                crate::direct_download::FileTransport::Api,
            ),
        };
        // Task 1643: the whole file is exactly the pinned version's plaintext, or nothing.
        if let Some(pin) = &pin
            && plaintext.len() as u64 != pin.plaintext_size_bytes
        {
            anyhow::bail!(
                "the download is {} bytes, the pinned version's manifest says {}",
                plaintext.len(),
                pin.plaintext_size_bytes
            );
        }
        tracing::debug!(transport = transport.as_str(), "download finished");
        Ok((plaintext, transport))
    }

    /// The download transport: `BB_TRANSPORT`, else `desktop.toml`'s `transport`, else `auto`
    /// (task 2173). A test bridge takes the one its test set, else `auto`.
    fn download_transport(&self, configured: Option<&str>) -> crate::direct_download::Transport {
        #[cfg(test)]
        {
            let _ = configured;
            self.seams
                .transport
                .lock()
                .unwrap()
                .unwrap_or(crate::direct_download::Transport::Auto)
        }
        #[cfg(not(test))]
        crate::direct_download::Transport::selected(configured)
    }

    /// A test's storage stand-in (task 2173); production always fetches.
    fn storage_override(&self) -> Option<crate::direct_download::Storage<'static>> {
        #[cfg(test)]
        if let Some(status) = *self.seams.storage_fail.lock().unwrap() {
            return Some(crate::direct_download::Storage::Fail(status));
        }
        None
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
    ///
    /// Task 1643: with a `pin`, every chunk request carries the pinned version's id and every chunk's
    /// wire size must be the manifest's; a 409 comes back as [`crate::download_pin::VersionGone`].
    /// Without one (a legacy file) the requests are today's unpinned ones.
    async fn download_content_frame(&self, file_id: &str, i: u32,
        pin: Option<&crate::download_pin::PinnedVersion>) -> anyhow::Result<Vec<u8>> {
        Ok(match pin {
                // Task 1643: the pinned route (shared with the direct path's fallback): exactly the manifest's
                // length, a transient failure tried again, a 409 = the version is gone, no URL in an error.
                Some(pin) => {
                    let expected = pin
                        .wire_size(i)
                        .ok_or_else(|| anyhow::anyhow!("chunk {i} is outside the pinned version"))?;
                    self.api
                        .download_chunk_pinned(file_id, i, &pin.object_version_id, expected)
                        .await
                        .map_err(|e| match e {
                            crate::api_client::ChunkApiError::Restart => {
                                anyhow::Error::new(crate::download_pin::VersionGone)
                            }
                            crate::api_client::ChunkApiError::Terminal(status) => {
                                anyhow::Error::new(crate::direct_download::DirectError::Refused(status))
                            }
                            crate::api_client::ChunkApiError::Failed(message) => anyhow::anyhow!(message),
                        })?
                }
                None => self.api.download_chunk(file_id, i).await?,
        })

    }

    #[allow(clippy::too_many_arguments)]
    async fn download_and_decrypt_chunk_range(
        &self,
        file_id: &str,
        file_key: &beebeeb_core::kdf::FileKey,
        range: std::ops::Range<u32>,
        download_kbps_limit: u64,
        approx_size_hint: usize,
        progress: Option<&HydrateProgressFn>,
        pin: Option<&crate::download_pin::PinnedVersion>,
        expected: beebeeb_core::chunk_stream::ExpectedChunkFormat,
        chunk_count: u32,
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

        // Public ChunkDecryptor delegates to core's StreamOpener; desktop owns no crypto.
        let whole = range.start == 0 && range.end == chunk_count;
        let mut reader = beebeeb_core::chunk_stream::ChunkDecryptor::for_push_with_file_key(
            beebeeb_core::kdf::FileKey::from_bytes(*file_key.as_bytes()), file_id, expected);
        let context = if whole { None } else {
            let first = self.download_content_frame(file_id, 0, pin).await?;
            let mut opened = reader.push_frame(&content_frame(&first)?)?;
            opened.data.zeroize();
            Some((reader.format().ok_or_else(|| anyhow::anyhow!("missing chunk format"))?, reader.version_id()))
        };
        for i in range {
            let chunk_start = std::time::Instant::now();
            let chunk_bytes = self.download_content_frame(file_id, i, pin).await?;
            let wire_len = chunk_bytes.len() as u64;
            // Decrypt into a temporary buffer, copy into the accumulator,
            // then zeroize the temporary. This limits live plaintext to
            // the accumulator + one chunk at any moment.
            let frame = content_frame(&chunk_bytes)?;
            let mut decrypted = if let Some((format, version)) = context {
                beebeeb_core::chunk_stream::open_chunk(file_key, file_id,
                    version.as_ref().map(|v| v.as_slice()).unwrap_or(&[]), i,
                    i + 1 == chunk_count, format, &frame)?
            } else { reader.push_frame(&frame)?.data };
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

        if whole { reader.finish()?; }
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

        // Task 1643: the range is planned on, and read from, one pinned version; a version that goes away
        // mid-read starts the read again, once, from fresh metadata (`with_one_restart`).
        let meta = self.api.get_file(file_id).await?;
        let read = self
            .with_one_restart(file_id, meta, |meta| async move {
                self.range_attempt(file_id, required_offset, required_length, &meta).await
            })
            .await;
        let read = match read { Ok(value) => Ok(value), Err(e) => Err(self.owned_open_failure(file_id, e).await) };
        if read.is_err() {
            // Every failing exit (a second pass's own error too) ends with the row in Error, never left on
            // Downloading, and the placeholder never in sync.
            let _ = self.db.set_status_if(file_id, FileStatus::Downloading, FileStatus::Error);
        }
        read
    }

    /// One pass of [`Self::hydrate_file_range`] on `meta`.
    #[cfg(any(target_os = "windows", test))]
    async fn range_attempt(
        &self,
        file_id: &str,
        required_offset: u64,
        required_length: u64,
        meta: &serde_json::Value,
    ) -> anyhow::Result<Option<HydratedRange>> {
        let pin = crate::download_pin::PinnedVersion::from_metadata(meta)?;
        let (chunk_count, size_bytes, chunk_size_bytes) = match &pin {
            Some(pin) => (
                u64::from(pin.chunk_count),
                Some(pin.plaintext_size_bytes),
                Some(pin.chunk_size_bytes),
            ),
            None => (
                meta.get("chunk_count").and_then(|v| v.as_i64()).unwrap_or(0) as u64,
                meta.get("size_bytes").and_then(|v| v.as_u64()),
                meta.get("chunk_size_bytes").and_then(|v| v.as_u64()),
            ),
        };

        let (Some(size_bytes), Some(chunk_size_bytes), true) = (size_bytes, chunk_size_bytes, chunk_count > 0)
        else {
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
        let expected_format = recorded_content_format(&file_key, meta)?;
        let download_kbps_limit = crate::config::DesktopConfig::load()
            .map(|c| c.download_kbps_limit)
            .unwrap_or(0);

        let chunks = plan.first_chunk..plan.last_chunk_exclusive;
        let read = self
            .download_and_decrypt_chunk_range(
                file_id,
                &file_key,
                chunks.clone(),
                download_kbps_limit,
                plan.covering_span_hint,
                None,
                pin.as_ref(),
                expected_format,
                chunk_count as u32,
            )
            .await
            .and_then(|data| {
                // Task 1643: exactly the pinned version's plaintext for those chunks.
                match pin.as_ref().map(|pin| pin.plaintext_size_of(chunks.clone())) {
                    Some(want) if want != Some(data.len() as u64) => anyhow::bail!(
                        "the range read is {} bytes, the pinned version's manifest says {want:?}",
                        data.len()
                    ),
                    _ => Ok(data),
                }
            });
        let data = read?;
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
    /// Core's purpose-specific thumbnail reader handles legacy raw-key blobs and
    /// subkey thumbnails. File content always uses the separate stream reader.
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
        let _: uuid::Uuid = file_id.parse()?;
        let blob = self.api.download_thumbnail(file_id, variant).await?;
        let key = self.file_key_for_download(file_id).await?;
        Ok(Zeroizing::new(beebeeb_core::encrypt::decrypt_thumbnail(&key, &blob)?))
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
            owned_metadata_for_name(&self.db, self.api.master_key(), file_id, &file_name, content_type.as_deref())?;

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
        if let Some(path) = released.as_deref() {
            crate::staged_payload::remove_or_log(&self.db, Path::new(path));
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
        let (remote, _) = self.remote_content_preview(file_id, is_text).await;

        Ok(ConflictContentPreview {
            is_text,
            local,
            remote,
            version: None,
        })
    }

    /// Task 2190 (plan §3.4): both sides of a parked conflict. The Mac's side is the parked write's
    /// staged copy, read with the same bounds as [`Self::local_content_preview`]; the other side is
    /// the server's current version. Read-only.
    pub async fn parked_conflict_preview(&self, op_id: &str) -> anyhow::Result<ConflictContentPreview> {
        let op = self
            .db
            .get_operation(op_id)?
            .ok_or_else(|| anyhow::anyhow!("no queued write {op_id}"))?;
        let file_id = op.file_id.clone().ok_or_else(|| anyhow::anyhow!("the write names no file"))?;
        let entry = self
            .db
            .get_file(&file_id)?
            .ok_or_else(|| anyhow::anyhow!("no state.db row for {file_id}"))?;
        let is_text = is_text_file(&entry.path);
        let payload = PathBuf::from(op.payload_path.ok_or_else(|| anyhow::anyhow!("the write has no staged copy"))?);
        let local = match (payload.parent(), payload.file_name().and_then(|name| name.to_str())) {
            (Some(dir), Some(name)) => self.local_content_preview(dir, name, is_text),
            _ => ConflictContentSide {
                size_bytes: None,
                text: None,
                unavailable_reason: Some("Couldn't locate this Mac's copy.".to_string()),
            },
        };
        // Review-169 #1 and F1: the version the person decides against is the version of the bytes
        // shown, from the metadata the download ran on (after a restart, the fresh one). Never the
        // mirror read after the download: the pull can raise it to a version nobody has seen while
        // the download runs. A file whose metadata names no version (legacy, or the fetch failed)
        // gets the mirror as it was BEFORE the fetch, which is never newer than the bytes: at worst
        // the decision is `Moved`, or the write is sent on an older base and the server refuses it.
        let before = self.db.get_file_contract_state(&file_id)?.map(|contract| contract.current_version);
        let (remote, shown) = self.remote_content_preview(&file_id, is_text).await;
        let version = shown.or(before);
        Ok(ConflictContentPreview { is_text, local, remote, version })
    }

    /// Task 2190 (plan §3.2): the person's decision on a parked conflict, against `shown_version`,
    /// the version the conflict window showed. Every choice re-checks the op inside the transaction
    /// that acts on it (decidable, and its file still at that version), and returns `Moved`, with
    /// nothing changed, when it moved since.
    /// - `Local`: the same op is rebased on the server's version and sent again.
    /// - `Remote` (D1): `keep_copy` copies the Mac's bytes into the "Changes not sent" folder FIRST.
    ///   If it fails, nothing changed. Then the op goes and the system reads the server's version.
    ///   The staged copy is released only after that commit.
    /// - `Both` (D4): the Mac's bytes are queued as a new file named [`keep_both_name`] beside the
    ///   original, and the op is retired, in ONE transaction (review-169 #3): if it moved, nothing
    ///   is queued.
    pub fn resolve_parked(
        &self,
        op_id: &str,
        choice: ParkedChoice,
        shown_version: Option<i64>,
        keep_copy: &mut dyn FnMut(&Path, &str) -> anyhow::Result<PathBuf>,
        device: &str,
        hhmm: &str,
    ) -> anyhow::Result<ParkedResolution> {
        // Review-169 #1: the person decides against the version they were shown. Without it, or
        // with the file past it, nothing changes.
        let Some(shown) = shown_version else {
            return Ok(ParkedResolution::Moved);
        };
        if choice == ParkedChoice::Local {
            return Ok(match self.db.rebase_parked(op_id, shown, now_secs())? {
                Some(version) => ParkedResolution::Rebased { version },
                None => ParkedResolution::Moved,
            });
        }
        if !self.db.is_decidable_parked_at(op_id, shown)? {
            return Ok(ParkedResolution::Moved);
        }
        let op = self
            .db
            .get_operation(op_id)?
            .ok_or_else(|| anyhow::anyhow!("no queued write {op_id}"))?;
        let file_id = op.file_id.clone().ok_or_else(|| anyhow::anyhow!("the write names no file"))?;
        let row = self
            .db
            .get_file(&file_id)?
            .ok_or_else(|| anyhow::anyhow!("no state.db row for {file_id}"))?;
        let payload = op
            .payload_path
            .clone()
            .ok_or_else(|| anyhow::anyhow!("the parked write has no staged copy"))?;
        let (done, released) = match choice {
            ParkedChoice::Remote => {
                // D1: the copy first; a failure changes nothing.
                let kept = keep_copy(Path::new(&payload), &row.path)?;
                match self.db.drop_parked(op_id, shown)? {
                    Some(released) => (
                        ParkedResolution::KeptOther {
                            kept_copy: kept.to_string_lossy().into_owned(),
                        },
                        released,
                    ),
                    // The kept copy stays (nothing is lost) and stays listed.
                    None => return Ok(ParkedResolution::Moved),
                }
            }
            ParkedChoice::Both => {
                let contract = self
                    .db
                    .get_file_contract_state(&file_id)?
                    .ok_or_else(|| anyhow::anyhow!("no contract for {file_id}"))?;
                let (parent_rel, leaf) = match row.path.rsplit_once('/') {
                    Some((parent, leaf)) => (Some(parent), leaf),
                    None => (None, row.path.as_str()),
                };
                let taken = self.db.child_names_lowercase(contract.parent_id.as_deref())?;
                let name = keep_both_name(leaf, device, hhmm, &taken);
                let target = FinderWriteTarget {
                    file_id: None,
                    parent_id: contract.parent_id.clone(),
                    filename: name.clone(),
                    rel_path: Some(match parent_rel {
                        Some(parent) => format!("{parent}/{name}"),
                        None => name.clone(),
                    }),
                    kind: FinderWriteItemKind::File,
                    contents_path: Some(payload.clone()),
                    content_type: contract.content_type.clone(),
                    base_version_identifier: None,
                };
                // Review-169 #3: the copy is queued and the parked write retired in ONE transaction;
                // a write that moved queues nothing.
                let retire = crate::state_db::RetireParked { op_id, shown_version: shown };
                let Some((written, released)) =
                    self.queue_file_provider_create_retiring(target, Some(&std::fs::File::open(&payload)?), Some(retire))?
                else {
                    return Ok(ParkedResolution::Moved);
                };
                match written.outcome {
                    FinderWriteOutcome::Queued { file_id: Some(new_id), .. } => {
                        (ParkedResolution::KeptBoth { file_id: new_id, name }, released)
                    }
                    other => anyhow::bail!("the copy could not be queued as a new file: {other:?}"),
                }
            }
            ParkedChoice::Local => unreachable!("handled above"),
        };
        // The staged copy was released in the decision's transaction; unlink it now (S6).
        if let Some(released) = released {
            crate::staged_payload::remove_or_log(&self.db, Path::new(&released));
        }
        Ok(done)
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
    ///
    /// Also returns the server version the side shows: the `download_version.version_number` of the
    /// metadata the attempt ran on (task 2190, review-169 F1), `None` when no metadata named one.
    async fn remote_content_preview(&self, file_id: &str, is_text: bool) -> (ConflictContentSide, Option<i64>) {
        let unavailable = |reason: String| ConflictContentSide {
            size_bytes: None,
            text: None,
            unavailable_reason: Some(reason),
        };
        if let Err(e) = file_id.parse::<uuid::Uuid>() {
            return (unavailable(format!("invalid file_id (not a UUID): {e}")), None);
        }
        let meta = match self.api.get_file(file_id).await {
            Ok(m) => m,
            Err(e) => return (unavailable(format!("Couldn't download the other device's version: {e}")), None),
        };
        // Tasks 1643 + 2173: a version that goes away mid-download restarts once from fresh metadata, as
        // `do_hydrate` does, and the size is checked again on the new version.
        self.with_one_restart(file_id, meta, |meta| async move {
            let version = meta["download_version"]["version_number"].as_i64();
            self.preview_attempt(file_id, is_text, &meta).await.map(|side| (side, version))
        })
        .await
        .unwrap_or_else(|e| (unavailable(format!("Couldn't download the other device's version: {e}")), None))
    }

    /// One pass of [`Self::remote_content_preview`] on `meta`. `Err` only for what the restart handles (the version
    /// went away) or what ends it; every other failure is the side's own "unavailable" line.
    async fn preview_attempt(&self, file_id: &str, is_text: bool, meta: &serde_json::Value) -> anyhow::Result<ConflictContentSide> {
        // Task 1643: the size of the version that would be downloaded. While a replace runs, the top-level
        // `size_bytes` can be another version's.
        let size_bytes = match crate::download_pin::PinnedVersion::from_metadata(meta)? {
            Some(pin) => Some(pin.plaintext_size_bytes),
            None => meta.get("size_bytes").and_then(|v| v.as_u64()),
        };

        if !is_text {
            // Binary preview only ever shows size — never download the bytes.
            return Ok(ConflictContentSide {
                size_bytes,
                text: None,
                unavailable_reason: None,
            });
        }
        let within_limit = matches!(size_bytes, Some(s) if s <= CONFLICT_PREVIEW_TEXT_MAX_BYTES as u64);
        if !within_limit {
            return Ok(ConflictContentSide {
                size_bytes,
                text: None,
                unavailable_reason: Some(match size_bytes {
                    Some(s) => format!("File is too large to preview, {s} bytes"),
                    None => "Couldn't determine the other device's file size".to_string(),
                }),
            });
        }

        // Between the metadata and the chunks: where another device's save and the pull's mirror of it
        // can land while the preview downloads (review-169 F1's test).
        self.seam("preview:before_download");
        match self.do_hydrate_with_meta(file_id, meta, None).await {
            Ok(mut bytes) => {
                // Move the plaintext out instead of `bytes.to_vec()` — a
                // clone would briefly hold two live copies of the remote
                // plaintext in memory. `content_side_from_bytes` consumes
                // the Vec by value (no further copy), and the now-empty
                // `bytes` is zeroized below for defense in depth.
                let side = content_side_from_bytes(std::mem::take(&mut *bytes), is_text);
                bytes.zeroize();
                Ok(side)
            }
            Err(e) if e.downcast_ref::<crate::download_pin::VersionGone>().is_some() => Err(e),
            Err(e) => Ok(ConflictContentSide {
                size_bytes,
                text: None,
                unavailable_reason: Some(format!("Couldn't download the other device's version: {e}")),
            }),
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

/// Task 2190 (plan §3.2): what the person chose for a parked conflict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ParkedChoice {
    /// "Keep this Mac's".
    Local,
    /// "Keep the other".
    Remote,
    /// "Keep both".
    Both,
}

/// Task 2190: what a decision on a parked conflict did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum ParkedResolution {
    /// The write is sent again, on the server's version `version`.
    Rebased { version: i64 },
    /// The Mac's bytes are kept at `kept_copy`, and the system reads the server's version.
    KeptOther { kept_copy: String },
    /// The Mac's bytes are queued as the new file `name`, beside the original.
    KeptBoth { file_id: String, name: String },
    /// The write moved since the review was read: nothing changed.
    Moved,
}

/// Task 2190 (plan §3.3, D4): the name of the "Keep both" copy, `{stem} ({device}, HH.MM){ext}`.
/// While that is taken, it becomes `{stem} ({device}, HH.MM) 2{ext}`, then ` 3`, and so on. It is
/// never a name in `taken` (the parent's names, lowercased). The time uses a dot, not D4's colon:
/// a `:` in a Mac file name shows as `/` in Finder, which is why 1887's kept folders use dots too. A
/// leading dot (`.env`) is not an extension.
///
/// Review-169 F3: the whole name fits the file-name limit, as a kept file's does
/// ([`crate::sign_out::kept_names::with_suffix`], 255 bytes): the stem is cut at a character boundary,
/// never the suffix, the counter or the extension. The device is cut first, to
/// [`KEEP_BOTH_DEVICE_MAX_BYTES`], so the suffix always leaves the stem room.
pub(crate) fn keep_both_name(original: &str, device: &str, hhmm: &str, taken: &std::collections::HashSet<String>) -> String {
    let device = crate::sign_out::kept_names::prefix_within(device, KEEP_BOTH_DEVICE_MAX_BYTES);
    let mut counter = 1u32;
    loop {
        let suffix = if counter == 1 {
            format!(" ({device}, {hhmm})")
        } else {
            format!(" ({device}, {hhmm}) {counter}")
        };
        let candidate = crate::sign_out::kept_names::with_suffix(original, &suffix);
        if !taken.contains(&candidate.to_lowercase()) {
            return candidate;
        }
        counter += 1;
    }
}

/// The longest device name a "Keep both" copy names, in UTF-8 bytes: a host name's label is at most 63
/// characters, so a Mac's own one-label name is never cut, and the cap keeps the suffix far inside the
/// 255-byte name limit (review-169 F3, plan §7.4).
const KEEP_BOTH_DEVICE_MAX_BYTES: usize = 64;

/// Result of [`EngineBridge::conflict_content_preview`] — the real content
/// (or an honest reason it's unavailable) for both sides of a conflict.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ConflictContentPreview {
    pub is_text: bool,
    pub local: ConflictContentSide,
    pub remote: ConflictContentSide,
    /// Task 2190 (review-169 #1): for a parked conflict, the file's version the person decides
    /// against; the decision is refused when the file is past it. `None` for a `Conflict` row.
    pub version: Option<i64>,
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

    // Task 2190 (the lead's condition on E3): a sync-folder edit the scan holds is shown, so it
    // never looks synced in the app. It is not sent, never marked in sync and never written over
    // (task 1640), and both versions are kept.
    // - `base_unknown`: a known edit whose base is not known: one row each, needing a decision.
    // - `content_unknown`: a file hydrated before task 1640. The scan cannot tell an edited one from
    //   an untouched one (task 2191 S1 marks them all not in sync), so they are ONE row that says
    //   they are not checked yet, never "needs a decision" per file.
    let mut not_checked: Vec<(String, String, i64)> = Vec::new();
    let held = db.list_held_edits().unwrap_or_else(|e| {
        tracing::warn!(error = %e, "review feed: the held edits could not be read; they are left out");
        Vec::new()
    });
    for (file_id, path, reason, detected_at) in held {
        if reason == HELD_CONTENT_UNKNOWN {
            not_checked.push((file_id, path, detected_at));
            continue;
        }
        entries.push(VersionConflictEntry {
            id: format!("held:{file_id}"),
            file_id,
            file_name: display_name_for_path(&path),
            kind: "held_edit".to_string(),
            status: "needs a decision".to_string(),
            updated_at: Some(detected_at),
            detail: "Changed on this PC, but Beebeeb doesn't know which version it was based on, so it \
                     isn't sent. Both versions are kept: yours on this PC, the other in the file's history."
                .to_string(),
            action: "review_upload".to_string(),
            op_id: None,
            version_id: None,
            base_version: None,
            last_error: None,
        });
    }
    if let Some((first_id, _, first_at)) = not_checked.first().cloned() {
        let names: Vec<String> = not_checked.iter().take(5).map(|(_, path, _)| display_name_for_path(path)).collect();
        let more = not_checked.len().saturating_sub(names.len());
        let files = if not_checked.len() == 1 { "1 file".to_string() } else { format!("{} files", not_checked.len()) };
        entries.push(VersionConflictEntry {
            id: "held:content_unknown".to_string(),
            file_id: first_id,
            file_name: format!("{files} from before version 0.8.13"),
            kind: "held_unknown".to_string(),
            status: "not checked yet".to_string(),
            updated_at: Some(first_at),
            detail: format!(
                "A change to one of them stays on this PC and isn't sent until a later update checks it. \
                 Nothing is lost. {}{}",
                names.join(", "),
                if more > 0 { format!(" and {more} more") } else { String::new() }
            ),
            action: "review_upload".to_string(),
            op_id: None,
            version_id: None,
            base_version: None,
            last_error: None,
        });
    }

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

    // Review-169 #5: one entry that cannot be read is logged and left out; the rest of the feed shows.
    for op in db.list_review_operations()? {
        match review_entry_for_operation(&op, db) {
            Ok(entry) => entries.push(entry),
            Err(e) => tracing::warn!(op_id = %op.op_id, error = %e, "review feed: an entry could not be read; it is left out"),
        }
    }

    // Task 1640 (review N9): a file deleted elsewhere while it held changes this device had not
    // sent; they were kept as a copy, which uploads as a new file. Informational: no decision is
    // needed, so the person can dismiss it.
    for kept in db.list_kept_copies()? {
        // Review F5: the text says why the file was kept.
        let original_name = display_name_for_path(&kept.original_path);
        let (file_name, status, detail) = match kept.reason {
            KeptReason::Copy => {
                let copy_name = display_name_for_path(&kept.copy_path);
                let place = match kept.copy_path.rsplit_once('/') {
                    Some((folder, _)) => format!("in the folder {folder}"),
                    None => "at the top of your Beebeeb folder".to_string(),
                };
                let original = if kept.original_removed {
                    ""
                } else {
                    " The original stays on this PC until it can be removed."
                };
                (
                    copy_name.clone(),
                    "kept as a copy",
                    format!(
                        "Deleted elsewhere while it held changes not sent from this PC. Those changes are kept as {copy_name}, {place}.{original}"
                    ),
                )
            }
            KeptReason::InUse => (
                original_name,
                "waiting to close",
                "Deleted elsewhere while it was open on this PC. It is removed from this PC once it is closed.".to_string(),
            ),
            KeptReason::PartlyLocal => (
                original_name,
                "kept in place",
                "Deleted elsewhere while only part of it was on this PC, so it could not be checked for changes. It stays where it is, as it is. Dismiss this once you have dealt with it.".to_string(),
            ),
            KeptReason::CopyFailed => (
                original_name,
                "kept in place",
                "Deleted elsewhere while it held changes not sent from this PC, and they could not be copied. It stays where it is; the copy is tried again.".to_string(),
            ),
        };
        entries.push(VersionConflictEntry {
            id: format!("kept:{}", kept.id),
            file_id: kept.original_file_id.clone(),
            file_name,
            kind: "kept_copy".to_string(),
            status: status.to_string(),
            updated_at: Some(kept.kept_at),
            detail,
            // Dismissable once nothing waits, or when no tick can resolve it (review F7).
            action: if kept.dismissable() { "dismiss" } else { "none" }.to_string(),
            op_id: None,
            version_id: None,
            base_version: None,
            last_error: None,
        });
    }

    entries.sort_by(|a, b| {
        b.updated_at
            .unwrap_or_default()
            .cmp(&a.updated_at.unwrap_or_default())
            .then_with(|| a.id.cmp(&b.id))
    });
    Ok(entries)
}

fn owned_key_and_pin(db: &StateDb, master: &[u8; 32], id: &str)
    -> anyhow::Result<(beebeeb_core::kdf::FileKey, beebeeb_core::chunk_stream::ExpectedChunkFormat)> {
    if let Some(record) = db.get_owned_key_record(id)? {
        let (key, metadata) = crate::owner_keys::record_key(&beebeeb_core::kdf::MasterKey::from_bytes(*master), &record)?;
        return Ok((key, metadata.chunk_format));
    }
    Ok((file_key_for(master, id), beebeeb_core::chunk_stream::ExpectedChunkFormat::Unrecorded))
}

fn owned_metadata_for_name(db: &StateDb, master: &[u8; 32], id: &str, name: &str, mime: Option<&str>) -> anyhow::Result<String> {
    let (key, pin) = owned_key_and_pin(db, master, id)?;
    Ok(beebeeb_core::encrypt::encrypt_name_for(&key, name, mime, pin)?)
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

/// Task 1640 (review F1): what a fetch served: the version, and the SHA-256 of all of that
/// version's plaintext.
#[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchedVersion {
    pub version: i64,
    pub hash: String,
}

/// Task 1640: one fetch's plaintext ([`EngineBridge::hydrate_file_to_memory`]), what it served
/// (`None`: the download named no version), and its [`FetchTicket`], held until the transfer ended.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub struct FetchedBytes {
    pub bytes: Zeroizing<Vec<u8>>,
    pub version: Option<FetchedVersion>,
    pub ticket: FetchTicket,
}

/// Task 1640 (self-review #1): a file's fetches in flight, and how often one started or ended.
#[derive(Debug, Default)]
struct FetchActivity {
    in_flight: usize,
    events: u64,
}

/// Task 1640 (self-review #1; design "The fetch tracker"): every file's fetches in flight, for the
/// whole process, not one bridge: a bridge made later in the same process (a new sign-in) sees what
/// an earlier one still has in flight. It is empty only when the process starts, and then no fetch
/// of an earlier process can transfer anything any more: only this process's callbacks write into
/// its placeholders.
static FETCHES_IN_FLIGHT: std::sync::LazyLock<std::sync::Mutex<HashMap<String, FetchActivity>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));

/// Task 1640 (self-review #1): one fetch of a file, from before it records anything until its
/// transfer ended (when it is dropped). It is "alone" when it was the file's only fetch in flight in
/// this process when it started, and no other fetch of the file has started or ended since.
#[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
pub struct FetchTicket {
    file_id: String,
    alone_since: Option<u64>,
}

impl FetchTicket {
    /// Whether this fetch is alone, read from the tracker's locked map.
    #[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
    fn alone_in(&self, fetches: &HashMap<String, FetchActivity>) -> bool {
        let Some(since) = self.alone_since else {
            return false;
        };
        fetches.get(&self.file_id).is_some_and(|activity| activity.events == since)
    }
}

impl Drop for FetchTicket {
    fn drop(&mut self) {
        {
            let mut fetches = FETCHES_IN_FLIGHT.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(activity) = fetches.get_mut(&self.file_id) {
                activity.in_flight = activity.in_flight.saturating_sub(1);
                activity.events += 1;
                if activity.in_flight == 0 {
                    fetches.remove(&self.file_id);
                }
            }
        }
    }
}

/// Task 2229: what [`EngineBridge::classify_local_path_verdict`] makes of a local file.
pub(crate) enum LocalPathVerdict {
    /// An upload to queue: a new file, or a change of a synced one.
    Write(FinderWriteTarget),
    /// A tracked file that needs nothing while its row and its bytes stay as they are: a folder
    /// row, a row not `Local`, bytes that are the agreed content, or a change held until its row
    /// records the agreed content or a base (task 2191). The scan may skip it until either changes.
    Settled,
    /// Nothing this pass; ask again next pass.
    Skip,
}

/// Task 1640: what a tracked sync-folder file's bytes are (Windows: Linux runs no upload driver), against the content
/// this device last agreed with the server on: `local_hash` at `local_base_version`, written by a
/// hydrate into the sync folder and by a landing. The tick's detector is not this: it never sees
/// the device's bytes (task 2190).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SyncFolderContent {
    /// The bytes are the agreed content: they may be replaced, freed or marked in sync.
    Agreed,
    /// An upload of the file's content is queued (parked included): the bytes are not sent yet.
    Queued,
    /// The bytes differ from the agreed content and nothing is queued: a change that is not
    /// sent. `base` is the version they are based on, `None` when it is not known.
    Changed { base: Option<i64> },
    /// No agreed content is recorded (a file hydrated before task 1640).
    Unknown,
}

/// Task 2190: a held edit's reasons (`held_edits.reason`).
const HELD_BASE_UNKNOWN: &str = "base_unknown";
const HELD_CONTENT_UNKNOWN: &str = "content_unknown";

/// [`SyncFolderContent`] of `file_id`'s bytes at `path`. `os_in_sync` is the Cloud Files in-sync
/// state of its placeholder (Windows; `None` elsewhere): the platform clears it when the bytes
/// change, and this engine sets it only on agreed content, so `Some(true)` is agreed without
/// reading the file. The file is read only when an agreed hash is recorded.
/// Task 2191: whether `path` holds bytes on this PC (a placeholder partly or wholly on disk, or
/// a plain file). Unreadable counts as holding bytes.
fn place_holds_bytes(path: &Path) -> bool {
    #[cfg(target_os = "windows")]
    {
        !matches!(
            crate::windows_cf::placeholders::placeholder_local_entry_trusting(path, false),
            LocalEntry::NothingLocal
        )
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::fs::metadata(path).is_ok_and(|meta| meta.len() > 0)
    }
}

/// Task 2271: what [`EngineBridge::row_judging_bytes_at`] found at a path.
enum RowAtPath {
    None,
    One(FileEntry),
    /// Several rows record content this PC agreed on, and the bytes are none of them (or more than one): the ids of
    /// those rows, and how many rows share the path.
    Ambiguous { file_ids: Vec<String>, rows: usize },
    /// Windows: several rows, and choosing would read bytes that are not wholly on this PC.
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    NotWhollyOnDisk,
}

pub(crate) fn sync_folder_content(
    db: &StateDb,
    file_id: &str,
    path: &Path,
    os_in_sync: Option<bool>,
) -> anyhow::Result<SyncFolderContent> {
    if db.sync_folder_upload_queued(file_id)? {
        return Ok(SyncFolderContent::Queued);
    }
    // Task 2191 (G-engine): a file of unknown content (synced before task 1640) may hold an edit
    // that 0.8.12 marked in sync, so the platform's in-sync state never makes it Agreed.
    if os_in_sync == Some(true) && !db.is_unknown_content(file_id)? {
        return Ok(SyncFolderContent::Agreed);
    }
    let Some(contract) = db.get_file_contract_state(file_id)? else {
        return Ok(SyncFolderContent::Unknown);
    };
    let (agreed, base) = match contract.local_hash {
        Some(agreed) => (agreed, contract.local_base_version),
        // Task 1640 (review N1): no agreed content yet, but range fetches served the bytes: the
        // candidate decides like a baseline. Its bytes are exactly version `version`'s, so equal
        // bytes are agreed, and any other is a change sent on `version` (never too high: a server
        // that moved refuses it as stale).
        None => match db.fetched_content(file_id)? {
            Some((version, Some(hash))) => (hash, version),
            // Mixed: the fetches served more than one version, so the bytes are none of them.
            // Never agreed; a change is sent on the oldest version served.
            Some((version, None)) => return Ok(SyncFolderContent::Changed { base: Some(version) }),
            None => return Ok(SyncFolderContent::Unknown),
        },
    };
    if cached_sha256_file_hex(file_id, path)? == agreed {
        return Ok(SyncFolderContent::Agreed);
    }
    Ok(SyncFolderContent::Changed {
        base: Some(base).filter(|version| *version > 0),
    })
}

/// Task 1640 (review N1, condition 2): make a file's candidate its agreed content. Only for a
/// file the platform reports in sync with no modified data (`local`), wholly on disk, whose bytes
/// read under one exclusive handle (`hash_under_handle`) are the candidate's, with a candidate of
/// ONE version (a mixed one is none), and only while the row is still `Local` with that same
/// candidate (re-checked in the write). `Ok(true)`: promoted.
#[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
pub(crate) fn promote_fetched_content(
    db: &StateDb,
    file_id: &str,
    local: LocalEntry,
    wholly_on_disk: bool,
    hash_under_handle: &mut dyn FnMut() -> std::io::Result<String>,
) -> anyhow::Result<bool> {
    if local != LocalEntry::InSync || !wholly_on_disk {
        return Ok(false);
    }
    let Some(row) = db.get_file(file_id)? else {
        return Ok(false);
    };
    if row.status != FileStatus::Local
        || db.get_file_contract_state(file_id)?.is_some_and(|contract| contract.local_hash.is_some())
    {
        return Ok(false);
    }
    let Some((version, Some(candidate))) = db.fetched_content(file_id)? else {
        return Ok(false);
    };
    if hash_under_handle()? != candidate {
        return Ok(false);
    }
    Ok(db.promote_fetched_content(file_id, version, &candidate)?)
}

/// Task 2204 S0.4: a Windows rename is sent this long after it is queued, so a safe save
/// (rename away, new file, delete) completes first and folds back onto the original.
pub(crate) const RENAME_SETTLE_SECS: i64 = 10;

/// Lower-case hex SHA-256 of `bytes`: a sync-folder file's agreed content (task 1640).
pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}

/// Task 2232: what a cached content hash is reused against. Size and modification time alone let a
/// write that keeps the size and sets the modification time back return the old hash, so an edit
/// read as the agreed content was never sent. The change time moves on every write and on every
/// time change, and an ordinary tool cannot set it back (Unix `ctime`; Windows `ChangeTime`). A
/// file renamed over the path is another file (Unix device and inode; Windows volume and file index).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct ContentFingerprint {
    len: u64,
    modified: std::time::SystemTime,
    changed: std::time::SystemTime,
    identity: (u64, u64),
}

impl ContentFingerprint {
    pub(crate) fn of(path: &Path) -> std::io::Result<Self> {
        let meta = std::fs::metadata(path)?;
        let modified = meta.modified()?;
        #[cfg(unix)]
        let (changed, identity) = {
            use std::os::unix::fs::MetadataExt;
            let changed = u64::try_from(meta.ctime())
                .ok()
                .and_then(|secs| {
                    std::time::UNIX_EPOCH.checked_add(std::time::Duration::new(secs, meta.ctime_nsec() as u32))
                })
                .unwrap_or(std::time::UNIX_EPOCH);
            (changed, (meta.dev(), meta.ino()))
        };
        #[cfg(target_os = "windows")]
        let (changed, identity) = {
            let stamp = crate::windows_cf::placeholders::change_stamp(path)?;
            (stamp.change_time, (u64::from(stamp.volume_serial), stamp.file_index))
        };
        #[cfg(not(any(unix, target_os = "windows")))]
        let (changed, identity) = (modified, (0, 0));
        Ok(Self {
            len: meta.len(),
            modified,
            changed,
            identity,
        })
    }

    /// The later of the modification and the change time: a hash taken within [`HASH_CACHE_SETTLED`] of it
    /// may have missed a write in the same timer tick.
    fn last_touched(&self) -> std::time::SystemTime {
        self.modified.max(self.changed)
    }
}

/// Task 1640 (review P2-5): [`sha256_file_hex`] of `file_id`'s file at `path`, read again only
/// when its [`ContentFingerprint`] changed since the last read: a scan, a close and a stamp
/// pass otherwise re-hashed every out-of-sync file (a plain file an atomic save left stays one)
/// every few seconds. A cached hash is trusted only when it was taken at least
/// [`HASH_CACHE_SETTLED`] after the file's modification and change times: file times are coarse (a
/// timer tick), so a later write inside the same tick would otherwise look unchanged. A hash whose
/// file changed while it was read is not kept. The cache is one per process, keyed by the file id and
/// the path together.
pub(crate) fn cached_sha256_file_hex(file_id: &str, path: &Path) -> std::io::Result<String> {
    cached_sha256_file_hex_at(file_id, path, std::time::SystemTime::now())
}

/// [`cached_sha256_file_hex`], the hash taken at `now`: a test moves it past [`HASH_CACHE_SETTLED`], as a
/// change time cannot be set back.
fn cached_sha256_file_hex_at(file_id: &str, path: &Path, now: std::time::SystemTime) -> std::io::Result<String> {
    static CACHE: std::sync::OnceLock<
        std::sync::Mutex<HashMap<(String, PathBuf), (ContentFingerprint, std::time::SystemTime, String)>>,
    > = std::sync::OnceLock::new();
    let fingerprint = ContentFingerprint::of;
    let before = fingerprint(path)?;
    let cache = CACHE.get_or_init(Default::default);
    let key = (file_id.to_string(), path.to_path_buf());
    if let Some((seen, hashed_at, hash)) = cache.lock().unwrap_or_else(|p| p.into_inner()).get(&key)
        && *seen == before
        && *hashed_at >= before.last_touched() + HASH_CACHE_SETTLED
    {
        return Ok(hash.clone());
    }
    let hashed_at = now;
    #[cfg(test)]
    HASH_READS.with(|reads| reads.set(reads.get() + 1));
    let hash = sha256_file_hex(path)?;
    if fingerprint(path)? == before {
        cache
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(key, (before, hashed_at, hash.clone()));
    }
    Ok(hash)
}

/// Files [`cached_sha256_file_hex`] read on this thread (tests count them).
#[cfg(test)]
thread_local! {
    static HASH_READS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// How long after a file's modification and change times a hash of it must have been taken to be reused.
const HASH_CACHE_SETTLED: std::time::Duration = std::time::Duration::from_secs(2);

/// [`sha256_hex`] of the file at `path`, read in a stream.
pub(crate) fn sha256_file_hex(path: &Path) -> std::io::Result<String> {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    std::io::copy(&mut std::fs::File::open(path)?, &mut hasher)?;
    Ok(format!("{:x}", hasher.finalize()))
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

/// The `init` request for one upload attempt. `proposed_id` is the item id this
/// device has chosen for the operation — a create's own (possibly freshly
/// minted) id, a replace's row id — and it is always sent: the server never
/// picks an id for this client (SF-0001 L13).
fn upload_init_request_for_operation(
    proposed_id: &str,
    name_encrypted: &str,
    content_type: Option<String>,
    parent_id: Option<String>,
    plaintext_size: u64,
    base_version: Option<i64>,
) -> DesktopUploadInitRequest {
    let plan = beebeeb_types::plan_chunks(plaintext_size, beebeeb_types::ChunkProfile::Desktop);
    DesktopUploadInitRequest {
        file_id: Some(proposed_id.to_string()),
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

/// SF-0001 (L13): whether `id` is exactly the canonical lowercase hyphenated
/// form of a UUID — the form a client-chosen item id must have. A UUID in any
/// other spelling (uppercase, braces, simple form, surrounding whitespace) or a
/// non-UUID local id is not canonical and must not be sent as the new file id.
///
fn is_canonical_item_id(id: &str) -> bool {
    uuid::Uuid::parse_str(id).is_ok_and(|parsed| parsed.hyphenated().to_string() == id)
}

/// SF-0001 (L13): assert that a persisted session still targets the item the op
/// is writing. A create session must be self-minted (its id was chosen by this
/// desktop); a replace session's `server_file_id` must equal the local id the
/// op is updating. Anything else means the server handed back a different item
/// than the client chose, so the op must abort.
///
fn ensure_session_item_id(session: &crate::state_db::UploadResume, local_file_id: &str) -> anyhow::Result<()> {
    let ours = if session.is_create {
        session.self_minted
    } else {
        session.server_file_id == local_file_id
    };
    if ours {
        return Ok(());
    }
    Err(anyhow::anyhow!(
        "server_changed_item_id: the upload init answered with an id this device did not choose"
    ))
}

#[cfg(test)]
fn prepare_thumbnail_uploads_for_plaintext_media(
    payload_path: &Path,
    mime_type: Option<&str>,
    file_key: &beebeeb_core::kdf::FileKey,
) -> anyhow::Result<Vec<PreparedThumbnailUpload>> {
    prepare_thumbnail_uploads_with_pin(payload_path, mime_type, file_key, beebeeb_core::chunk_stream::ExpectedChunkFormat::Unrecorded)
}

fn prepare_thumbnail_uploads_with_pin(payload_path: &Path, mime_type: Option<&str>, file_key: &beebeeb_core::kdf::FileKey,
    pin: beebeeb_core::chunk_stream::ExpectedChunkFormat) -> anyhow::Result<Vec<PreparedThumbnailUpload>> {
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
            beebeeb_core::encrypt::encrypt_thumbnail_for(file_key, &plaintext, pin)
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
/// distinct one when that attempt parks the op, worded by why: a stale base
/// parked it at once (`parked_at_once`, spec §8.4; `attempt` is the attempt that
/// was made), or its attempts are used up. Ids, counts and the class only: the
/// file's name, the request URL and the server's text never reach the log.
fn log_refused_upload(
    op: &PendingOperation,
    attempt: i64,
    class: crate::api_client::InitConflictClass,
    parked_at_once: bool,
) {
    let file_id = op.file_id.as_deref().unwrap_or_default();
    if parked_at_once {
        tracing::warn!(
            op_id = %op.op_id,
            file_id,
            attempt,
            max_attempts = op.max_attempts,
            base_version = ?op.base_version,
            class = class.as_str(),
            reason = ParkReason::StaleBase.as_str(),
            "upload refused by the server (409 Conflict); its base is stale, parked with its bytes kept in the queue"
        );
    } else if attempt >= op.max_attempts {
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

/// Spec §11: a File Provider write whose attempts a failure waiting does not cure used up parks by
/// the queue's rule. A 409 says so on its refused-upload line; any other failure logs the park line
/// once, with `reason="attempts_used_up"`, a log reason and not a [`ParkReason`]. `file_id` by M5.
fn log_parked_attempts_used_up(op_id: &str, file_id: Option<&str>) {
    let (file_id, id_kind) = wire_id_fields(file_id.unwrap_or_default());
    tracing::warn!(
        op_id = %op_id,
        file_id,
        id_kind,
        reason = "attempts_used_up",
        "upload parked with its bytes kept in the queue"
    );
}

/// `op`'s metadata with its encrypted name (if it carries one) re-encrypted
/// for `file_id`. The plaintext name is the op's display name; the MIME hint
/// is the op's content type, else the one guessed from the name, as the
/// Finder write path does.
fn metadata_rekeyed_with_material(master_key: &[u8; 32], op: &PendingOperation, file_id: &str, material: Option<(&beebeeb_core::kdf::FileKey, beebeeb_core::chunk_stream::ExpectedChunkFormat)>) -> anyhow::Result<Option<String>> {
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
    metadata["name_encrypted"] = serde_json::json!(match material {
        Some((key, pin)) => beebeeb_core::encrypt::encrypt_name_for(key, &name, mime.as_deref(), pin)?,
        None => encrypted_metadata_for_name(master_key, file_id, &name, mime.as_deref())?,
    });
    Ok(Some(serde_json::to_string(&metadata)?))
}

/// Spec §7.2 (final review): the parent an upload's metadata step sends. A File Provider save
/// records in its accept whether it moved the file, or follows a queued save that did
/// (`parent_changed`): then its parent, the top level as `null`; otherwise none. An upload with
/// no record (a create, the watcher's, Windows', an earlier build's) sends its parent when it
/// has one, as before.
fn upload_parent_change<'a>(
    op: &'a PendingOperation,
    metadata: &serde_json::Value,
) -> crate::api_client::ParentChange<'a> {
    use crate::api_client::ParentChange;
    match metadata.get("parent_changed").and_then(serde_json::Value::as_bool) {
        Some(true) => ParentChange::to(op.parent_id.as_deref()),
        Some(false) => ParentChange::Unchanged,
        None => op
            .parent_id
            .as_deref()
            .map_or(ParentChange::Unchanged, ParentChange::Folder),
    }
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

/// Normalize the legacy JSON transport envelope, without decryption or format fallback.
pub(crate) fn content_frame(bytes: &[u8]) -> anyhow::Result<Vec<u8>> {
    if let Ok(blob) = serde_json::from_slice::<EncryptedBlob>(bytes) {
        anyhow::ensure!(blob.nonce.len() == 12, "invalid chunk envelope");
        let mut frame = blob.nonce;
        frame.extend(blob.ciphertext);
        Ok(frame)
    } else { Ok(bytes.to_vec()) }
}

fn recorded_content_format(key: &beebeeb_core::kdf::FileKey, meta: &serde_json::Value)
    -> anyhow::Result<beebeeb_core::chunk_stream::ExpectedChunkFormat> {
    let name = meta["name_encrypted"].as_str().ok_or_else(|| anyhow::anyhow!("missing authenticated file metadata"))?;
    Ok(beebeeb_core::encrypt::decrypt_file_metadata_with_key(key, name)?.chunk_format)
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

/// Where a direct download (task 2173) reports: the same wire counter, transfer board and
/// progress callback the API path's chunk loop feeds.
struct HydrateReport<'a> {
    bridge: &'a EngineBridge,
    file_id: &'a str,
    /// The server's plaintext size (the API path's `total`).
    total: u64,
    progress: Option<&'a HydrateProgressFn>,
}

impl crate::direct_download::Progress for HydrateReport<'_> {
    fn received(&self, wire_bytes: u64) {
        self.bridge.wire.download_bytes.fetch_add(wire_bytes, Ordering::Relaxed);
    }

    fn decrypted(&self, plaintext_bytes: u64) {
        self.decrypted_bytes(plaintext_bytes);
    }
}

impl HydrateReport<'_> {
    fn decrypted_bytes(&self, done: u64) {
        self.bridge.transfers.report(self.file_id, done, self.total);
        if let Some(report) = self.progress {
            report(done, self.total);
        }
    }
}

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
            // Only over the hydrate's own `Downloading` (spec §9.2).
            let _ = self
                .db
                .set_status_if(self.file_id, FileStatus::Downloading, self.restore.clone());
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

/// Task 2190 (review-169 #5): whether the review row may open the decision. A read that fails offers
/// no decision (the row stays, with its old action), and is logged.
fn decidable_or_logged(db: &StateDb, op_id: &str) -> bool {
    decidable_check(db, op_id).unwrap_or_else(|e| {
        tracing::warn!(op_id = %op_id, error = %e, "review feed: the decision check failed");
        false
    })
}

fn decidable_check(db: &StateDb, op_id: &str) -> anyhow::Result<bool> {
    #[cfg(test)]
    if FAIL_DECIDABLE_READ.with(|fail| fail.borrow().as_deref() == Some(op_id)) {
        anyhow::bail!("the decision check failed (test)");
    }
    Ok(db.is_decidable_parked(op_id)?)
}

#[cfg(test)]
thread_local! {
    /// Task 2190: the op whose decision check fails, for the feed's per-item test.
    static FAIL_DECIDABLE_READ: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
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
    // Task 2190 (plan §3.4): a parked stale-base Finder write the person may decide opens the
    // conflict; every other row keeps its action (D3).
    let action = if kind == "stale_base" && decidable_or_logged(db, &op.op_id) {
        "open_conflict"
    } else {
        action
    };

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

/// The attempt count at which [`retry_delay_seconds`] stops growing: the most a transient failure
/// raises a File Provider write's count to (spec §8.4). Derived from the backoff, so changing the
/// backoff moves it too.
fn backoff_cap_attempts() -> i64 {
    /// Where the search stops if the backoff ever had no cap; the backoff reaches its cap at 6 today.
    const SEARCH_LIMIT: i64 = 64;
    let cap = retry_delay_seconds(i64::MAX);
    let found = (1..=SEARCH_LIMIT).find(|&attempts| retry_delay_seconds(attempts) >= cap);
    debug_assert!(
        found.is_some(),
        "retry_delay_seconds reaches its cap within {SEARCH_LIMIT} attempts"
    );
    found.unwrap_or(SEARCH_LIMIT)
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

#[cfg(test)]
thread_local! { static LEGACY_CHILD_READS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

fn unwrap_folder_child_for_format(folder: &[u8;32], root: &str, id: &str, kind: u8, wrap: &str, custody: bool) -> anyhow::Result<beebeeb_core::kdf::FileKey> {
    if custody {
        let frame = base64::engine::general_purpose::STANDARD.decode(wrap)?;
        Ok(beebeeb_sync::key_resolution::unwrap_child_key(
            &beebeeb_core::kdf::FileKey::from_bytes(*folder), root, id, kind, &frame)?)
    } else { unwrap_child_file_key(folder, wrap) }
}

fn unwrap_child_file_key(
    folder_key: &[u8; 32],
    encrypted_file_key_b64: &str,
) -> anyhow::Result<beebeeb_core::kdf::FileKey> {
    #[cfg(test)]
    LEGACY_CHILD_READS.with(|n| n.set(n.get()+1));
    let file_key = unwrap_key_frame(folder_key, encrypted_file_key_b64, "encrypted_file_key")?;
    Ok(beebeeb_core::kdf::FileKey::from_bytes(*file_key))
}

#[cfg(test)]
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

/// Spec D5 (task 1888): after the pass committed, the directories of the scaffolding it dropped (sync-root-relative)
/// are removed deepest first, each with `std::fs::remove_dir`: never recursive, so one that holds anything errors and
/// stays, as does one already gone. A path that is empty, absolute or climbs out of the sync root is skipped. The
/// paths are never logged.
#[cfg_attr(target_os = "macos", allow(dead_code))]
fn remove_scaffold_dirs(sync_root: &Path, dirs: &[String]) {
    let mut inside: Vec<&Path> = dirs
        .iter()
        .map(Path::new)
        .filter(|rel| {
            rel.components().next().is_some()
                && rel.components().all(|part| matches!(part, std::path::Component::Normal(_)))
        })
        .collect();
    inside.sort_by_key(|rel| std::cmp::Reverse(rel.components().count()));
    for rel in inside {
        let _ = std::fs::remove_dir(sync_root.join(rel));
    }
}

/// Whether `sealed` (a `name_encrypted` envelope) is this account's name for `file_id`. Core's file metadata reader
/// (`decrypt_file_metadata`) opens it under the file key derived from `master_key` and `file_id`, in every format the
/// engine reads: the string id (as every client seals today) and the 16-byte binary id (the old CLI); the name subkey
/// (`beebeeb/name/v1`, task 1908) and the legacy raw file key; both envelopes. It refuses a plaintext name, which proves
/// nothing. The key is derived from this id, so a name sealed for another file, or under another account's key, never
/// opens. Task 2267's proof (`own_data_proof::probe`) and task 1888's key check (spec D12) read names this way.
pub fn name_opens(master_key: &[u8; 32], file_id: &str, sealed: &str) -> bool {
    let master = beebeeb_core::kdf::MasterKey::from_bytes(*master_key);
    beebeeb_core::encrypt::decrypt_file_metadata(&master, file_id, sealed)
        .is_ok_and(|metadata| !metadata.name.trim().is_empty())
}

pub(crate) fn decrypt_shared_name_with_key(file_key: &beebeeb_core::kdf::FileKey, name_encrypted: &str) -> Option<String> {
    beebeeb_core::encrypt::decrypt_file_metadata_with_key(file_key, name_encrypted).ok().map(|m| m.name)
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
    // Spec §7.2 (final review): the row and its contract in one transaction, so no listing
    // sees a new shared row in the person's own namespace.
    db.upsert_file_with_contract(&entry, |contract| {
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
            .or(contract.current_object_version_id.take());
        contract.last_sync_at = now;
    })?;
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
    let leaf = crate::owner_keys::stored_name(db,file_id,master_key)?.unwrap_or_else(||resolve_relative_path(f,file_id,master_key));
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
    // Task 2204 S0.3: while a Windows rename of this file is queued (parked included), the row's
    // path and parent are where the file is on disk; the server's are still the old ones.
    let held = match existing.as_ref() {
        // Only `handle_rename` (Windows) queues these ops, so other builds skip the query.
        Some(entry) if cfg!(any(target_os = "windows", test)) && !entry.is_dir() && db.has_queued_local_rename(file_id)? => {
            Some(entry.clone())
        }
        _ => None,
    };
    let path = held.as_ref().map_or(path, |entry| entry.path.clone());
    let status = existing
        .as_ref()
        .map(|entry| entry.status.clone())
        .unwrap_or(FileStatus::CloudOnly);
    // Task 1640 (review N1, condition 1): a remote move or rename gives the row another place;
    // the fetch candidate described the entry at the old one.
    let moved = existing.as_ref().is_some_and(|entry| entry.path != path);

    let item_kind = if f["is_folder"].as_bool().unwrap_or(false)
        || f["kind"].as_str() == Some("folder")
        || f["type"].as_str() == Some("folder")
    {
        ItemKind::Folder
    } else {
        ItemKind::File
    };
    let is_file = item_kind == ItemKind::File;
    let server_parent_id = match held.as_ref() {
        Some(entry) => entry.parent_id.clone(),
        None => f["parent_id"].as_str().map(str::to_string),
    };

    let mut entry = FileEntry {
        file_id: file_id.to_string(),
        path,
        status,
        size_bytes: size,
        modified_at: remote_updated,
        content_hash: existing.as_ref().and_then(|entry| entry.content_hash.clone()),
        remote_updated_at: remote_updated,
        // NOTE: parent_id/item_kind are NOT persisted by upsert_file — the
        // contract write in the same transaction (below) writes the
        // authoritative `files.parent_id` / `files.item_kind` columns. These
        // struct fields exist so the in-memory `FileEntry` returned to callers
        // (notably the windows_cf placeholder seeder reading via list_by_status)
        // carries the correct folder/parent classification.
        parent_id: server_parent_id.clone(),
        item_kind: item_kind.clone(),
    };
    // Spec §7.2 (final review): the row and its contract in one transaction, so no listing
    // sees a new nested row without its parent.
    // Task 2204 S0.3 (review-155): the hold is decided again inside the write's own
    // transaction, so a Windows rename committed after the reads above is never written over.
    entry.path = db.upsert_server_file_row(&entry, |contract| {
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
            .or(contract.current_object_version_id.take());
        // Task 2190 (plan §7.1 E3): a `Local` row's base is never invented from the server's
        // version. 0 stays unknown; an edit on an unknown base is held, not sent (task 2229).
        contract.last_sync_at = now;
    })?;
    // Task 1640 (review F1): a move changes no byte, but the candidate is never promoted at the
    // new place: it becomes mixed, its version kept.
    if moved {
        db.poison_fetched_content(file_id)?;
    }
    // Task 2191 (D12, review-114 F-P1-7, R2-P2-2): a remote rename or move leaves this file's
    // placeholder at the old path (T9), and the new path may still hold another file's. Both
    // places are recorded for the tick's retry (Windows), which keeps one only when the identity
    // there names a file of unknown content living elsewhere (`unknown_check::verify_orphan`).
    if moved && cfg!(any(target_os = "windows", test)) && is_file {
        if let Some(previous) = existing.as_ref() {
            db.put_unknown_orphan(&previous.path, file_id, true)?;
        }
        db.put_unknown_orphan(&entry.path, file_id, true)?;
    }
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
///    `remote_updated_at`. The row is re-anchored to the server's
///    version (task 2190, plan §7.2). A `Local` row has no pending write;
///    an edit not yet queued keeps its base (1640's agreed content), is
///    queued on it and parks as stale when the server moved: the parked
///    op is the conflict (E1). The pull reads no file.
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
    /// Spec D11 (task 1888, review P1): the pull came back from the server: a snapshot, or the `/sync/ops` delta. A
    /// tick whose `GET /sync/ops` failed (offline, a 5xx, a 401) still returns `Ok`, so `Ok` alone proves neither that
    /// the server accepts this session's token nor that the rows the reconcile pass plans against are current; the
    /// pass and the resume wait for a tick where this is true.
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    pub reached_server: bool,
}

/// Re-snapshots that must fail in a row before a failed one no longer ends the tick (spec §6.3.2).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
const FAILED_SNAPSHOTS_BEFORE_THE_QUEUE_RUNS: u32 = 3;

/// A requested re-snapshot failed (spec §6.3.2). On macOS the request survives a failure, so the
/// next tick tries again. The first two failures in a row end the tick, as before; from the third
/// on, the tick goes on with the ops pull from `cursor` and the queue runs: an upload whose base is
/// known needs no snapshot, and one still waiting for its base keeps waiting at the claim. Before,
/// a snapshot that kept failing for a reason waiting does not cure (a local database error, an
/// endpoint that times out on a large tree) stopped every upload on the Mac. A successful
/// snapshot resets the count. Elsewhere the request was taken before the snapshot, so a failure
/// ends only this tick.
fn after_failed_snapshot(bridge: &EngineBridge, cursor: i64, error: anyhow::Error) -> anyhow::Result<i64> {
    #[cfg(target_os = "macos")]
    {
        let failures = bridge.failed_snapshots.fetch_add(1, Ordering::SeqCst).saturating_add(1);
        if failures >= FAILED_SNAPSHOTS_BEFORE_THE_QUEUE_RUNS {
            tracing::warn!(
                failures,
                reason = error_category(error.as_ref()),
                "sync_tick: re-snapshot failed again; pulling ops and running the queue without it"
            );
            return Ok(cursor);
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (bridge, cursor);
    Err(error)
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
    bridge.maintain_owned_keys(now_secs).await?;

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
            {
                bridge.failed_snapshots.store(0, Ordering::SeqCst);
                if let Some(seen) = resnapshot_request {
                    bridge.db().clear_resnapshot_request(seen)?;
                }
            }
            applied_item_ids.extend(applied);
            return Ok(SyncTickOutcome {
                conflicts,
                applied_item_ids: dedupe(applied_item_ids),
                reached_server: true,
            });
        }
        Some(c) if needs_resnapshot => {
            tracing::info!("sync_tick: re-snapshot requested (gap recovery); bootstrapping");
            match bootstrap_from_snapshot(bridge, sync_root, now_secs, &mut conflicts).await {
                Ok(applied) => {
                    #[cfg(target_os = "macos")]
                    {
                        bridge.failed_snapshots.store(0, Ordering::SeqCst);
                        if let Some(seen) = resnapshot_request {
                            bridge.db().clear_resnapshot_request(seen)?;
                        }
                    }
                    applied_item_ids.extend(applied);
                    return Ok(SyncTickOutcome {
                        conflicts,
                        applied_item_ids: dedupe(applied_item_ids),
                        reached_server: true,
                    });
                }
                Err(e) => after_failed_snapshot(bridge, c, e)?,
            }
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
            // The pull did not come back: `Ok`, but not a tick that reached the server (spec D11, review P1).
            return Ok(SyncTickOutcome {
                conflicts,
                applied_item_ids: dedupe(applied_item_ids),
                reached_server: false,
            });
        }
    };

    // New servers carry the authoritative per-user head in the delta response.
    // Idle ticks need no full tree fetch. Keep the snapshot probe only for a
    // reset (cursor ahead) or an old server that does not advertise its head.
    if ops.ops.is_empty() {
        if ops.head_seq.is_some_and(|head| head >= cursor) {
            // The delta came back (empty, at the head): this tick reached the server.
            return Ok(SyncTickOutcome {
                conflicts,
                applied_item_ids: dedupe(applied_item_ids),
                reached_server: true,
            });
        }
        // `now_secs` (captured at tick start) is the prune freshness cutoff: it
        // predates this snapshot fetch, so it can't be later than any row a
        // concurrent completion stamps during the fetch.
        let fetched_at = now_secs;
        match bridge.api().sync_snapshot().await {
            Ok(mut snap) if snap.seq_id < cursor => {
                snap.nodes=bridge.prepare_owned_rows(&snap.nodes).await?.into_iter().map(|p|p.row).collect();
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
            reached_server: true,
        });
    }

    // Apply the delta ops in order, advancing the cursor to the max seq_id.
    let mut max_seq = cursor;
    for original in &ops.ops {
        let prepared = prepare_owned_sync_op(bridge, original).await?;
        bridge.owner_key_scope().check()?;
        max_seq = max_seq.max(original.seq_id);
        let Some(op) = prepared else { continue; };
        match apply_sync_op(bridge, sync_root, &op, now_secs, &mut conflicts) {
            Ok(applied) => applied_item_ids.extend(applied),
            Err(e) => {
                // A failed op applied nothing: it contributes no ids.
                tracing::warn!(error = %e, op_type = %op.op_type, seq_id = op.seq_id, "sync_tick: op apply error; skipping op");
            }
        }
    }
    if max_seq > cursor {
        bridge.db().set_sync_cursor(max_seq)?;
    }
    Ok(SyncTickOutcome {
        conflicts,
        applied_item_ids: dedupe(applied_item_ids),
        reached_server: true,
    })
}

/// Enrich actual sparse events before entering the synchronous reconciler.
async fn prepare_owned_sync_op(bridge:&EngineBridge,op:&crate::api_client::SyncOp)->anyhow::Result<Option<crate::api_client::SyncOp>> {
    if !matches!(op.op_type.as_str(),"file_create"|"folder_create"|"file_update"|"file_rename"|"folder_rename"|"file_move"|"folder_move") {return Ok(Some(op.clone()));}
    let Some(id)=op.payload["id"].as_str() else{return Ok(Some(op.clone()))};
    let known_foreign=bridge.db().get_owned_key_record(id)?.is_some_and(|r|r.foreign_added || r.owner_wrap.is_some());
    let mut row=synthesize_op_row(bridge,id,op,op.payload["new_name_encrypted"].as_str().or_else(||op.payload["name_encrypted"].as_str()));
    if known_foreign && op.payload["name_encrypted"].is_null() && op.payload["new_name_encrypted"].is_null() {row=serde_json::json!({"id":id});}
    let mut prepared=bridge.prepare_owned_rows(&[row]).await?;
    let Some(prepared)=prepared.pop() else{return Ok(None)};
    let mut result=op.clone();result.payload=prepared.row;
    if matches!(op.op_type.as_str(),"file_move"|"folder_move") {result.payload["new_parent_id"]=result.payload["parent_id"].clone();}
    if matches!(op.op_type.as_str(),"file_rename"|"folder_rename") {result.payload["new_name_encrypted"]=result.payload["name_encrypted"].clone();}
    Ok(Some(result))
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
    let mut snapshot = bridge.api().sync_snapshot().await?;
    snapshot.nodes=bridge.prepare_owned_rows(&snapshot.nodes).await?.into_iter().map(|p|p.row).collect();
    #[cfg_attr(not(target_os = "macos"), allow(unused_mut))]
    let mut applied = apply_snapshot(bridge, sync_root, &snapshot, now_secs, fetched_at, conflicts)?;
    // §6.3.3: an upload still waiting for its base after this successful snapshot counts
    // one more; at the 10th it parks with its bytes. The park changes how its item is
    // presented (spec §9.1), and the snapshot did not list the file, so its id joins the
    // tick's applied ids, which the tick's one working-set signal carries.
    #[cfg(target_os = "macos")]
    for (op_id, file_id) in bridge.db().note_snapshot_for_base_pending(now_secs)? {
        log_parked(&op_id, file_id.as_deref(), ParkReason::BaseUnknown);
        applied.extend(file_id);
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
        remove_pruned_placeholders(bridge.db(), sync_root, &pruned);
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
/// Task 1640: what a remotely deleted file's local entry holds, as the platform reports it.
#[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LocalEntry {
    /// A placeholder in sync with no modified data: the agreed bytes, or none.
    InSync,
    /// A placeholder with no data on disk: nothing local to keep.
    NothingLocal,
    /// Every byte is on disk: a hydrated placeholder out of sync, or a plain file (what an atomic
    /// save leaves).
    WhollyLocal,
    /// Some bytes are on disk and it is out of sync: reading it would fetch the rest.
    PartlyLocal,
    /// The platform's view could not be read (self-review #5: the file is open exclusively, or the
    /// query failed): treated as in use, kept and retried, never as partly on disk.
    Unreadable,
}

/// Task 1640 (review P1-1): what removing one remotely deleted sync-folder entry did.
#[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PrunedEntry {
    /// Removed: agreed bytes, no local bytes, or an empty folder.
    Removed,
    /// Nothing was there.
    Gone,
    /// The file held bytes the server does not have: they were copied to `copy` (whose SHA-256
    /// is `copied_hash`), where the copy can upload, then the original was removed, or not
    /// (`original_removed`: it was in use; a later tick removes it).
    KeptCopy { copy: PathBuf, copied_hash: String, original_removed: bool },
    /// A file left as it is, and why (review F5): partly on disk (reading it would fetch the
    /// rest), in use (it could not be removed or read), or holding changes whose copy failed.
    KeptInPlace(KeptReason),
    /// A folder that still holds something (review F3).
    FolderKept,
}

/// Task 1640 (review P1-1, N2, N6): remove the local entry of a file or folder the server no
/// longer lists (a web trash, a snapshot prune) without losing a change that was never sent.
///
/// A file in sync, or with no bytes on disk, is removed. A file wholly on disk (out of sync after
/// an edit, or a plain file an atomic save left) is removed when its bytes are `agreed` (the
/// row's agreed content); other bytes are copied first, with [`copy_as_this_pc`], into
/// `copy_dir` (a folder the copy can upload from), and only then is the original removed. A file
/// partly on disk is left: reading it would fetch the rest from inside the tick, and the server's
/// own bytes would come back as a copy. A folder is removed only when it is empty.
#[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
pub(crate) fn remove_pruned_entry(
    path: &Path,
    is_dir: bool,
    agreed: Option<&str>,
    local: LocalEntry,
    copy_dir: &Path,
) -> PrunedEntry {
    if std::fs::symlink_metadata(path).is_err() {
        return PrunedEntry::Gone;
    }
    let removed = |result: std::io::Result<()>, kept: PrunedEntry| match result {
        Ok(()) => PrunedEntry::Removed,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => PrunedEntry::Gone,
        Err(_) => kept,
    };
    let in_use = || PrunedEntry::KeptInPlace(KeptReason::InUse);
    if is_dir {
        return removed(engine_remove(path, true), PrunedEntry::FolderKept);
    }
    match local {
        LocalEntry::InSync | LocalEntry::NothingLocal => removed(engine_remove(path, false), in_use()),
        LocalEntry::PartlyLocal => PrunedEntry::KeptInPlace(KeptReason::PartlyLocal),
        LocalEntry::Unreadable => in_use(),
        LocalEntry::WhollyLocal => {
            let Ok(hash) = sha256_file_hex(path) else {
                return in_use();
            };
            if agreed == Some(hash.as_str()) {
                return removed(engine_remove(path, false), in_use());
            }
            match copy_as_this_pc(path, copy_dir) {
                Ok((copy, copied_hash)) => {
                    let original_removed = match engine_remove(path, false) {
                        Ok(()) => true,
                        Err(e) => e.kind() == std::io::ErrorKind::NotFound,
                    };
                    PrunedEntry::KeptCopy { copy, copied_hash, original_removed }
                }
                Err(_) => PrunedEntry::KeptInPlace(KeptReason::CopyFailed),
            }
        }
    }
}

/// Task 1640 (review-151d P3-1): the engine's own remove of a sync-folder entry. It is registered
/// with the watcher right before it happens, so its delete notification queues no trash, and the
/// registration is dropped again when it did not happen (a later delete by the person counts).
#[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
fn engine_remove(path: &Path, is_dir: bool) -> std::io::Result<()> {
    crate::watcher::suppress_engine_delete(path);
    let removed = if is_dir { std::fs::remove_dir(path) } else { std::fs::remove_file(path) };
    if removed.is_err() {
        crate::watcher::cancel_engine_delete_suppression(path);
    }
    removed
}

/// Task 1640 (review-151d P2-2): once `original` is removed, the Office owner file it left (`~$`
/// and its name, or `~$` and its name without the first two characters) goes too: it belongs to
/// that file, and would keep its folder. No other file is touched: names the sync folder ignores
/// are never synced, so they would be lost for good.
#[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
fn remove_office_owner_files(original: &Path) {
    let (Some(dir), Some(name)) = (original.parent(), original.file_name().and_then(|n| n.to_str())) else {
        return;
    };
    let mut owners = vec![format!("~${name}")];
    if let Some((cut, _)) = name.char_indices().nth(2) {
        owners.push(format!("~${}", &name[cut..]));
    }
    for owner in owners {
        let path = dir.join(owner);
        if std::fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.is_file()) {
            let _ = std::fs::remove_file(&path);
        }
    }
}

/// Task 1640 (review P1-1, N5): copy `path` into `dir` as `<stem> (this PC)<ext>`, then
/// `<stem> (this PC 2)<ext>` and so on, never over an existing file. The bytes are written and
/// flushed under an ignored temporary name first and linked to the final name only then, so the
/// upload driver never sees a partial copy. Returns the copy and the SHA-256 of what it holds.
#[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
pub(crate) fn copy_as_this_pc(path: &Path, dir: &Path) -> std::io::Result<(PathBuf, String)> {
    use sha2::{Digest, Sha256};
    use std::io::{Read, Write};
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("file");
    let ext = path.extension().and_then(|s| s.to_str());
    let temp = dir.join(format!(".{stem}.{}.tmp", uuid::Uuid::new_v4()));
    let written = (|| -> std::io::Result<String> {
        let mut source = std::fs::File::open(path)?;
        let mut out = std::fs::OpenOptions::new().write(true).create_new(true).open(&temp)?;
        let mut hasher = Sha256::new();
        let mut buffer = vec![0u8; 64 * 1024];
        loop {
            let read = source.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
            out.write_all(&buffer[..read])?;
        }
        out.sync_all()?;
        Ok(format!("{:x}", hasher.finalize()))
    })();
    let hash = match written {
        Ok(hash) => hash,
        Err(e) => {
            let _ = std::fs::remove_file(&temp);
            return Err(e);
        }
    };
    for n in 1..=99 {
        let suffix = if n == 1 { " (this PC)".to_string() } else { format!(" (this PC {n})") };
        let name = match ext {
            Some(ext) => format!("{stem}{suffix}.{ext}"),
            None => format!("{stem}{suffix}"),
        };
        let copy = dir.join(name);
        match move_no_replace(&temp, &copy) {
            Ok(()) => return Ok((copy, hash)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => {
                let _ = std::fs::remove_file(&temp);
                return Err(e);
            }
        }
    }
    let _ = std::fs::remove_file(&temp);
    Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, "no free name for the copy"))
}

/// Task 1640 (review N5): move `from` to `to`, never over an existing file (`AlreadyExists`).
/// Windows: `MoveFileExW` without `MOVEFILE_REPLACE_EXISTING` (a hard link may be refused inside a
/// Cloud Files sync root). Elsewhere: a hard link, then the old name is removed.
#[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
pub(crate) fn move_no_replace(from: &Path, to: &Path) -> std::io::Result<()> {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows::Win32::Storage::FileSystem::{MOVEFILE_WRITE_THROUGH, MoveFileExW};
        let wide = |path: &Path| path.as_os_str().encode_wide().chain(Some(0)).collect::<Vec<u16>>();
        let (from_wide, to_wide) = (wide(from), wide(to));
        // SAFETY: both wide strings are NUL-terminated and alive for the call.
        unsafe {
            MoveFileExW(
                windows::core::PCWSTR(from_wide.as_ptr()),
                windows::core::PCWSTR(to_wide.as_ptr()),
                MOVEFILE_WRITE_THROUGH,
            )
        }
        // An HRESULT from a Win32 error carries the code in its low 16 bits.
        .map_err(|e| std::io::Error::from_raw_os_error(e.code().0 & 0xFFFF))
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::fs::hard_link(from, to)?;
        std::fs::remove_file(from)
    }
}

/// Task 1640 (review N3): where a copy of `path` can upload from: its folder, or the nearest
/// folder above it that is neither being removed now (`removed_dirs`) nor in the trash, else
/// the sync root. Inside a folder trashed on the web the copy would name that folder as its
/// parent, which the server refuses.
#[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
fn kept_copy_dir(db: &StateDb, sync_root: &Path, path: &Path, removed_dirs: &HashSet<PathBuf>) -> PathBuf {
    let mut dir = path.parent().unwrap_or(sync_root).to_path_buf();
    while dir != sync_root && dir.starts_with(sync_root) {
        let trashed = relative_db_path(sync_root, &dir)
            .and_then(|rel| db.get_file_by_path(&rel).ok().flatten())
            .is_some_and(|row| row.status == FileStatus::Trashing);
        if !removed_dirs.contains(&dir) && !trashed {
            return dir;
        }
        match dir.parent() {
            Some(parent) => dir = parent.to_path_buf(),
            None => break,
        }
    }
    sync_root.to_path_buf()
}

/// `path` relative to `sync_root`, '/'-joined.
#[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
fn rel_under(sync_root: &Path, path: &Path) -> String {
    path.strip_prefix(sync_root)
        .map(|rel| rel.to_string_lossy().replace('\\', "/"))
        .unwrap_or_default()
}

/// Record a kept copy (review N9, N13): one review entry until it is dismissed, a tray activity
/// row, and, while its original could not be removed, the retry's record. `copy: None` is a file
/// kept where it is (`reason`, review F5); `copied_hash` is then its agreed content, if any, for
/// the retry to compare with. `existing`: the entry to update where it is (review F6), not a new one.
#[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
#[allow(clippy::too_many_arguments)]
fn record_kept_copy(
    db: &StateDb,
    sync_root: &Path,
    file_id: &str,
    original: &Path,
    copy: Option<&Path>,
    copied_hash: &str,
    original_removed: bool,
    reason: KeptReason,
    existing: Option<i64>,
) {
    let now = now_secs();
    let original_path = rel_under(sync_root, original);
    let copy_path = copy.map(|copy| rel_under(sync_root, copy)).unwrap_or_default();
    let input = crate::state_db::KeptCopyInput {
        original_file_id: file_id,
        original_path: &original_path,
        copy_path: &copy_path,
        copied_hash,
        original_removed,
        reason,
        kept_at: now,
    };
    let recorded = match existing {
        Some(id) => db.update_kept_copy(id, &input),
        None => db.record_kept_copy(&input).map(|_| ()),
    }
    .and_then(|()| match copy {
        Some(copy) => db.record_local_activity(LocalActivityEventInput {
            event_type: LocalActivityKind::KeptLocalCopy,
            file_id: Some(file_id.to_string()),
            file_name: copy.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_string(),
            rel_path: Some(copy_path.clone()),
            occurred_at: now,
        }),
        None => Ok(()),
    });
    if let Err(e) = recorded {
        tracing::warn!(file_id = %file_id, error = %e, "remote-deletion reconcile: could not record the kept copy");
    }
}

/// Task 1640 (review F3): the folders deleted elsewhere that stay on disk for what they hold, as
/// paths under `sync_root`.
#[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
fn removed_folder_paths(db: &StateDb, sync_root: &Path) -> HashSet<PathBuf> {
    db.list_removed_folders()
        .unwrap_or_default()
        .iter()
        .filter_map(|rel| EngineBridge::placeholder_path_under(sync_root, rel))
        .collect()
}

/// Remove the on-disk entries of `rows` (a remote deletion), each through [`remove_pruned_entry`],
/// with `local` the platform's view of each file.
#[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
fn remove_pruned_entries(
    db: &StateDb,
    sync_root: &Path,
    rows: &[crate::state_db::PrunedRow],
    local: &dyn Fn(&Path, bool) -> LocalEntry,
) {
    let mut removed_dirs: HashSet<PathBuf> = rows
        .iter()
        .filter(|row| row.is_dir)
        .filter_map(|row| EngineBridge::placeholder_path_under(sync_root, &row.path))
        .collect();
    removed_dirs.extend(removed_folder_paths(db, sync_root));
    for row in rows {
        let Some(path) = EngineBridge::placeholder_path_under(sync_root, &row.path) else {
            continue;
        };
        let rel = rel_under(sync_root, &path);
        // Review F4: a file whose kept entry still waits (a copy made, the original in use; a
        // file kept where it is) is the retry's. A second remote op (the trash emptied after a
        // web trash, a folder trash after a file trash) copies nothing again. Unreadable: kept.
        if !row.is_dir && db.pending_kept_original(&rel).map_or(true, |pending| pending.is_some()) {
            continue;
        }
        // The remove itself registers with the watcher right before it happens (`engine_remove`),
        // so the NOTIFY_DELETE_COMPLETION it fires queues no trash.
        let agreed = row.local_hash.as_deref().or(row.fetched_hash.as_deref());
        // Task 2191 (G-engine, review-114 F-P1-4): the platform's in-sync state is trusted only
        // for a file with agreed content; one of unknown content is read by its size on disk.
        let state = if row.is_dir { LocalEntry::InSync } else { local(&path, agreed.is_some()) };
        let copy_dir = kept_copy_dir(db, sync_root, &path, &removed_dirs);
        match remove_pruned_entry(&path, row.is_dir, agreed, state, &copy_dir) {
            PrunedEntry::Removed if !row.is_dir => remove_office_owner_files(&path),
            PrunedEntry::Removed | PrunedEntry::Gone => {}
            PrunedEntry::KeptCopy { copy, copied_hash, original_removed } => {
                if original_removed {
                    remove_office_owner_files(&path);
                }
                // Zero-knowledge: never log the path, only the file_id.
                tracing::warn!(
                    file_id = %row.file_id,
                    original_removed,
                    "remote-deletion reconcile: the file held changes that were not sent; they are kept as a copy"
                );
                record_kept_copy(
                    db,
                    sync_root,
                    &row.file_id,
                    &path,
                    Some(&copy),
                    &copied_hash,
                    original_removed,
                    KeptReason::Copy,
                    None,
                );
            }
            PrunedEntry::KeptInPlace(reason) => {
                tracing::warn!(
                    file_id = %row.file_id,
                    reason = reason.as_str(),
                    "remote-deletion reconcile: kept the file where it is"
                );
                // Condition 5: ONE entry per file (a second remote op skips it, above); the retry
                // resolves it once it can be removed, copied or is gone.
                record_kept_copy(db, sync_root, &row.file_id, &path, None, agreed.unwrap_or_default(), false, reason, None);
            }
            PrunedEntry::FolderKept => {
                tracing::warn!(file_id = %row.file_id, "remote-deletion reconcile: kept a folder that still holds something");
                // Review F3: never created again from this PC while it only holds what is kept.
                if let Err(e) = db.record_removed_folder(&rel, now_secs()) {
                    tracing::warn!(file_id = %row.file_id, error = %e, "remote-deletion reconcile: could not record the kept folder");
                }
            }
        }
    }
}

/// Task 1640 (review N13, F2, F6): every file kept from a remote deletion whose original still
/// waits (it was in use, partly on disk, or its copy failed) is retried: the original is removed
/// when it still holds the copied bytes; one the person changed since is a new change, kept as a
/// new copy, and then removed. A copy is never made twice of the same bytes. The entry is updated
/// where it is, or goes when nothing is kept. An original whose place belongs to a file the
/// server lists again (restored on the web, or another file moved there) is left alone and no
/// longer waits. Then every kept folder that is empty now is removed (review F3).
#[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
pub(crate) fn retry_kept_originals(db: &StateDb, sync_root: &Path, local: &dyn Fn(&Path, bool) -> LocalEntry) {
    // Review-151d P2-1: a dismissed entry's tombstone goes once its file is gone, or once the place
    // is the server's again.
    for (id, rel) in db.list_dismissed_kept_originals().unwrap_or_default() {
        let gone = EngineBridge::placeholder_path_under(sync_root, &rel).is_none_or(|path| std::fs::symlink_metadata(path).is_err());
        let listed = db.get_file_by_path(&rel).is_ok_and(|row| row.is_some_and(|row| row.status != FileStatus::Trashing));
        if gone || listed {
            let _ = db.delete_kept_copy(id);
        }
    }
    let Ok(kept) = db.list_kept_copies() else {
        return;
    };
    let removed_dirs = removed_folder_paths(db, sync_root);
    for copy in kept.into_iter().filter(|copy| !copy.original_removed) {
        // Resolved with nothing left to wait for: a copy's entry stays (dismissable); a file kept
        // where it is had nothing kept, so its entry goes.
        let resolve = |copy: &crate::state_db::KeptCopy| {
            let _ = if copy.copy_path.is_empty() {
                db.delete_kept_copy(copy.id)
            } else {
                db.mark_kept_original_removed(copy.id)
            };
        };
        let Some(original) = EngineBridge::placeholder_path_under(sync_root, &copy.original_path) else {
            resolve(&copy);
            continue;
        };
        if std::fs::symlink_metadata(&original).is_err() {
            resolve(&copy);
            continue;
        }
        // Review F2: the file was restored on the web (its own row, live again): it is tracked by
        // its row, never removed or trashed again. Any other row at the place (another file moved
        // there) does not own these bytes: they are still the deleted file's placeholder, which would
        // keep the other file's from being created, so they go as below (a change copied first); the
        // remove is the engine's own, so nothing is trashed (self-review #4).
        match db.get_file_by_path(&copy.original_path) {
            Ok(Some(row)) if row.file_id == copy.original_file_id && row.status != FileStatus::Trashing => {
                resolve(&copy);
                continue;
            }
            Ok(_) => {}
            Err(_) => continue,
        }
        let copy_dir = kept_copy_dir(db, sync_root, &original, &removed_dirs);
        let agreed = Some(copy.copied_hash.as_str()).filter(|hash| !hash.is_empty());
        // Review F2: the engine's own remove queues no trash (`engine_remove` registers it).
        // Task 2191 (review-114 F-P1-4): an entry with no agreed content never trusts in sync.
        match remove_pruned_entry(&original, false, agreed, local(&original, agreed.is_some()), &copy_dir) {
            PrunedEntry::Removed | PrunedEntry::Gone => {
                remove_office_owner_files(&original);
                resolve(&copy);
            }
            PrunedEntry::KeptCopy { copy: new_copy, copied_hash, original_removed } => {
                if original_removed {
                    remove_office_owner_files(&original);
                }
                // Changed since (or wholly on disk now, with changes): a new change, in the same entry.
                record_kept_copy(
                    db,
                    sync_root,
                    &copy.original_file_id,
                    &original,
                    Some(&new_copy),
                    &copied_hash,
                    original_removed,
                    KeptReason::Copy,
                    Some(copy.id),
                );
            }
            PrunedEntry::KeptInPlace(reason) => {
                // Self-review #5: an entry whose changes could not be copied was read wholly once; it
                // never becomes "partly on disk", which the person may dismiss.
                let downgrade = copy.reason == KeptReason::CopyFailed && reason == KeptReason::PartlyLocal;
                if copy.copy_path.is_empty() && reason != copy.reason && !downgrade {
                    record_kept_copy(
                        db,
                        sync_root,
                        &copy.original_file_id,
                        &original,
                        None,
                        &copy.copied_hash,
                        false,
                        reason,
                        Some(copy.id),
                    );
                }
            }
            PrunedEntry::FolderKept => {}
        }
    }
    remove_empty_kept_folders(db, sync_root);
}

/// Task 1640 (review F3): remove every kept folder (deepest first) that holds nothing now, and
/// forget every one that is gone or that the server lists again.
#[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
fn remove_empty_kept_folders(db: &StateDb, sync_root: &Path) {
    for rel in db.list_removed_folders().unwrap_or_default() {
        let Some(path) = EngineBridge::placeholder_path_under(sync_root, &rel) else {
            let _ = db.forget_removed_folder(&rel);
            continue;
        };
        match db.get_file_by_path(&rel) {
            // Listed again (restored on the web, or another folder moved there): it is not removed.
            Ok(Some(row)) if row.status != FileStatus::Trashing => {
                let _ = db.forget_removed_folder(&rel);
                continue;
            }
            Ok(_) => {}
            Err(_) => continue,
        }
        // Review-151d P2-2: only an empty folder goes. One that keeps a file the sync folder ignores
        // (never synced, so the person's only copy) stays, and is not created again on the server.
        if std::fs::read_dir(&path).is_ok_and(|mut entries| entries.next().is_some()) {
            continue;
        }
        match engine_remove(&path, true) {
            Ok(()) => {
                let _ = db.forget_removed_folder(&rel);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let _ = db.forget_removed_folder(&rel);
            }
            Err(_) => {}
        }
    }
}

/// Task 1640 (review F3): whether `dir` holds only what this PC keeps from a deletion elsewhere:
/// originals that still wait to be removed or whose entry was dismissed (review-151d P2-1),
/// folders that themselves hold only that, and names the sync folder ignores (an Office owner
/// file). Anything else is the person's, and the folder is wanted again. Fails closed (review-151d
/// P3-5): what cannot be read, and a tree deeper than 32, count as held, so a folder the server
/// removed is never created again on an error.
fn held_only_for_kept(db: &StateDb, sync_root: &Path, dir: &Path, depth: usize) -> bool {
    if depth >= 32 {
        return true;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return true;
    };
    for entry in entries {
        let Ok(entry) = entry else {
            return true;
        };
        let path = entry.path();
        if entry.file_name().to_str().is_some_and(is_ignored_finder_name) {
            continue;
        }
        let Some(rel) = relative_db_path(sync_root, &path) else {
            return true;
        };
        let held = if entry.file_type().map_or(true, |kind| kind.is_dir()) {
            db.is_removed_folder(&rel).unwrap_or(true) && held_only_for_kept(db, sync_root, &path, depth + 1)
        } else {
            db.is_kept_original(&rel).unwrap_or(true)
        };
        if !held {
            return false;
        }
    }
    true
}

#[cfg(target_os = "windows")]
fn remove_pruned_placeholders(db: &StateDb, sync_root: &Path, rows: &[crate::state_db::PrunedRow]) {
    remove_pruned_entries(db, sync_root, rows, &crate::windows_cf::placeholders::placeholder_local_entry_trusting);
}

/// No-op stand-in on non-Windows builds so the call sites stay platform-agnostic.
#[cfg(not(target_os = "windows"))]
fn remove_pruned_placeholders(_db: &StateDb, _sync_root: &Path, _rows: &[crate::state_db::PrunedRow]) {}

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
                remove_pruned_placeholders(bridge.db(), sync_root, &marked);
                applied.extend(marked.iter().map(|row| row.file_id.clone()));
            }
            Some(_) => {
                // Mark the subtree Trashing (the trash view), remove the
                // on-disk placeholders exactly as the delete path did (the
                // item is hidden locally — that part of 0802 stands), and
                // report the ids so the working-set signal fires.
                let marked = bridge.db().mark_subtree_trashing(id)?;
                remove_pruned_placeholders(bridge.db(), sync_root, &marked);
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
                    remove_pruned_placeholders(bridge.db(), sync_root, &orphans);
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
                remove_pruned_placeholders(bridge.db(), sync_root, &removed);
                applied.push(id.to_string());
                applied.extend(removed.iter().map(|row| row.file_id.clone()));
            }
            None => {
                let orphans = bridge.db().delete_orphaned_children_of_absent_folder(id)?;
                if !orphans.is_empty() {
                    remove_pruned_placeholders(bridge.db(), sync_root, &orphans);
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
                remove_pruned_placeholders(bridge.db(), sync_root, &flipped);
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
            // Spec §7.2 (final review): with no `new_parent_id` key the op names no
            // new parent, so the row keeps its path's folders as it keeps its parent
            // (`synthesize_op_row`); a key decides both, `null` meaning the top level.
            let parent_rel = if payload.get("new_parent_id").is_some() {
                parent_rel_path_by_id(bridge, payload["new_parent_id"].as_str())
            } else {
                existing_parent_rel_path(bridge, id)
            };
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
    // A sparse folder rename/move has no descendant events. Reuse the same
    // metadata write for their new parent paths, then signal those identities
    // with the folder. This preserves content baselines and queued rename holds.
    if matches!(op.op_type.as_str(), "folder_rename" | "folder_move" | "file_rename" | "file_move")
        && applied.iter().any(|changed| changed == id)
        && bridge.db().get_file(id)?.is_some_and(|row| row.is_dir())
    {
        rebase_folder_descendants(bridge, id, now_secs, &mut applied)?;
    }
    Ok(applied)
}

fn rebase_folder_descendants(
    bridge: &EngineBridge,
    folder_id: &str,
    now: i64,
    applied: &mut Vec<String>,
) -> anyhow::Result<()> {
    let rows = bridge.db().list_files()?;
    let mut parents = vec![folder_id.to_string()];
    let mut visited = std::collections::HashSet::new();
    while let Some(parent_id) = parents.pop() {
        if !visited.insert(parent_id.clone()) {
            continue;
        }
        let Some(parent) = bridge.db().get_file(&parent_id)? else { continue };
        for child in rows.iter().filter(|row| row.parent_id.as_deref() == Some(&parent_id)) {
            let Some(contract) = bridge.db().get_file_contract_state(&child.file_id)? else { continue };
            let row = serde_json::json!({
                "id": child.file_id,
                "name": child.path.rsplit('/').next().unwrap_or_default(),
                "parent_id": parent_id,
                "is_folder": child.is_dir(),
                "size_bytes": child.size_bytes,
                "updated_at": child.remote_updated_at,
                "version_number": contract.current_version,
                "current_object_version_id": contract.current_object_version_id,
                "content_type": contract.content_type,
            });
            if let Some(updated) = apply_metadata_file_row(
                bridge.db(), &row, contract.namespace, contract.shared_root_id,
                contract.share_id, contract.permission_bits, now, bridge.api().master_key(), &parent.path,
            )? {
                if updated.path != child.path {
                    applied.push(child.file_id.clone());
                }
                if updated.is_dir() {
                    parents.push(child.file_id.clone());
                }
            }
        }
    }
    Ok(())
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
    // the existing parent. Spec §7.2 (final review): a move's `new_parent_id` of `null` is
    // the top level, never "keep the parent".
    let parent_id = if op.op_type.ends_with("_move") && payload.get("new_parent_id").is_some() {
        payload["new_parent_id"].as_str().map(str::to_string)
    } else {
        payload["new_parent_id"]
            .as_str()
            .or_else(|| payload["parent_id"].as_str())
            .map(str::to_string)
            .or_else(|| existing.as_ref().and_then(|e| e.parent_id.clone()))
    };
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
    // Task 2190 (plan §7.1 E1): the detector reports no `ConflictDetected`; a concurrent edit is
    // the parked op. Kept so the tick's conflict plumbing stays as it is.
    _conflicts: &mut Vec<ConflictDetected>,
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

            // Task 2190 (follow-up 1, plan §7.2 as corrected after review-169 #7): a `Local` row has no
            // pending write (a queued or parked one makes the row `Uploading` or `Error`, branch 3), so
            // the pull re-anchors it to the server's version. An edit on disk that the scan has not
            // queued yet keeps its base (1640's agreed content, which this write never touches), is
            // queued on it, refused as stale and parked: the parked op is the conflict (E1). The pull
            // reads no file: `content_hash` is not read either (E2), and no hash is taken on a cold
            // cache just to choose a log line.
            let mut updated = entry.clone();
            updated.remote_updated_at = remote_updated;
            updated.size_bytes = size;
            updated.modified_at = remote_updated;
            bridge.db().upsert_file(&updated)?;
            apply(bridge)
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
    fn test_open_content_frame(key: &beebeeb_core::kdf::FileKey, bytes: &[u8]) -> anyhow::Result<Vec<u8>> {
        let mut reader = beebeeb_core::chunk_stream::ChunkDecryptor::for_push_with_file_key(
            beebeeb_core::kdf::FileKey::from_bytes(*key.as_bytes()), "legacy", beebeeb_core::chunk_stream::ExpectedChunkFormat::Unrecorded);
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
mod uploads;
mod hydration;
mod trash_ops;
mod shared;
mod metadata;
mod finder_queue;
mod uploads_2;
mod conflicts;
mod uploads_3;
mod uploads_4;
mod shared_2;
mod sync;
mod sync_2;
mod ipc;
mod hydrate_files;
mod trash_markers;
mod versions;
mod uploads_5;
mod finder_queue_2;
mod uploads_6;
mod refusals;
mod sync_3;
mod uploads_7;
mod versions_2;
mod finder_queue_3;
mod trash_ops_2;
mod metadata_2;
mod finder_queue_4;
mod versions_3;
mod sync_4;
mod uploads_8;
mod finder_queue_5;
mod refusals_2;
mod finder_queue_6;
mod thumbnails;
mod sync_5;
mod conflicts_2;
mod thumbnails_2;
mod uploads_9;
mod refusals_3;
mod logging;
mod downloads;
mod reg_1640;
mod reg_1640_2;
mod scan;
mod review_2190;
mod review_2190_2;

use basics::{RecordedRequest, UploadMockServer, read_http_request_or_hangup, test_bridge, test_bridge_with_api, write_test_png};
use finder_queue_2::{capture_logs_async, fp_create};
use finder_queue_5::{Refused, refuse, stop_refusing};
use ipc::{IpcHydrationMock};
use metadata_2::{patched_parents};
use refusals_2::{reqwest_status_error, transient};
use refusals_3::{PLANTED, assert_clean_line};
use reg_1640::{regression_1640_chunk, regression_1640_edited_file_in_a_folder, regression_1640_edited_file_in_a_folder_in, regression_1640_synced_file, regression_1640_web_saves};
use reg_1640_2::{regression_1640_new_placeholder, regression_1640_uploads_queued_for};
use scan::{regression_2229_set_modified, task_2190_files_holding, task_2190_fu1_tick_after, task_2190_server_chunk, task_2190_web_saves};
use shared_2::{enqueue_test_operation, seed_bridge_entry, seed_bridge_row};
use sync::{SyncMockServer, enc_name, snap_node};
use sync_5::{landed_create, rule3_setup, thumbnail_blob};
use trash_markers::{MockServerFile, VersionedServerMock, VersionedServerState};
use uploads::{HydrationMockServer, MockResponse, TEST_FILE_ID, encrypt_chunk_wire, http_bytes, http_json, hydration_mock_response, hydration_test_key, read_http_request, upload_mock_response};
use uploads_3::{ResumableUploadMock, ResumeMockMode, count_requests, init_item_id, resumable_create_op};
use uploads_5::{fp_save, queue_save, seed_uploaded_db_row, seed_uploaded_row};
use uploads_7::{land_one_save, remote_update};
use uploads_8::{assert_kept_uploading, failing_passes};
use versions::{drain_upload_queue, finder_file_target, held_content_version, run_competing, versioned_server_answer};
use versions_2::{FP_ROOT, fp_create_in, fp_save_moved, listed_under, name_parent_status, reparented_changes, seed_folder_row, seed_row_in_folder};
use versions_3::{node, seed_version_zero_row};
}
