//! SQLite-backed per-file sync state.
//!
//! Tracks the local mirror's view of every file in the vault: cloud-only
//! placeholder, downloading, fully local, uploading, conflicting, or in
//! some error state. The OS extensions (File Provider on macOS, Cloud
//! Files on Windows, FUSE on Linux) read this table to render the right
//! Finder/Explorer overlay icon; the daemon writes it as it makes
//! progress.
//!
//! Spec: `docs/superpowers/plans/2026-05-07-desktop-sync-client.md`
//! Phase 1 Task 1.
//!
//! ## Schema
//!
//! Single `files` table keyed by `file_id` (the server's UUID for the
//! file). `path` is the relative path inside the sync root. `status`
//! drives the overlay icon; the rest are bookkeeping.
//!
//! WAL journal mode so reads from the OS extension don't block the
//! daemon's writes.

use crate::diagnostic_redaction::{
    allowed_label, classify_error_code, redact_for_export, redact_secrets_only, DiagnosticErrorCode, KnownNames,
    PAUSE_REASON_LABELS, QUEUE_KIND_LABELS,
};
use rusqlite::{Connection, OptionalExtension, Result, params};
use serde::Serialize;
use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use std::sync::Mutex;

pub const LOCAL_ACTIVITY_MAX_ROWS: usize = 200;
/// Cap on `transfer_activity` (task 1683 slice 2). The popover shows five rows.
pub const TRANSFER_ACTIVITY_MAX_ROWS: usize = 100;

/// Server completion is independent from native identity stamping. This journal
/// survives queue removal, restart and failed proof unlink.
#[derive(Debug, Clone, PartialEq)]
pub struct UploadFinalization {
    pub op_id: String,
    pub local_file_id: String,
    pub server_file_id: String,
    pub target_path: String,
    pub payload_path: String,
    pub stamped: bool,
}


/// High-level sync status for a single file. Maps 1:1 to the icon
/// overlays rendered by the platform extensions.
#[derive(Debug, Clone, PartialEq)]
pub enum FileStatus {
    CloudOnly,
    Downloading,
    Local,
    Uploading,
    Conflict,
    Error,
    /// A locally-deleted file whose server-trash is still pending. The on-disk
    /// placeholder is already gone (the user deleted it); the row is kept ONLY
    /// so the queued `TrashFile` op stays coherent with it and so the Windows
    /// placeholder seeder (`populate_placeholders`, which only mints for
    /// `CloudOnly`) does NOT re-create the disk placeholder before the trash
    /// round-trips — the "deleted file comes back" bug (task 0802). The
    /// `TrashFile` op deletes this row on success; if the trash permanently
    /// fails the row stays `Trashing` (recoverable — the file still exists on
    /// the server) rather than reappearing on disk.
    Trashing,
}

impl FileStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            FileStatus::CloudOnly => "cloud_only",
            FileStatus::Downloading => "downloading",
            FileStatus::Local => "local",
            FileStatus::Uploading => "uploading",
            FileStatus::Conflict => "conflict",
            FileStatus::Error => "error",
            FileStatus::Trashing => "trashing",
        }
    }
    fn from_str(s: &str) -> Self {
        match s {
            "cloud_only" => FileStatus::CloudOnly,
            "downloading" => FileStatus::Downloading,
            "local" => FileStatus::Local,
            "uploading" => FileStatus::Uploading,
            "conflict" => FileStatus::Conflict,
            "trashing" => FileStatus::Trashing,
            _ => FileStatus::Error,
        }
    }
}

pub const PERMISSION_READ: i64 = 1 << 0;
pub const PERMISSION_WRITE: i64 = 1 << 1;
pub const PERMISSION_SHARE: i64 = 1 << 2;
pub const PERMISSION_OWNER: i64 = 1 << 3;

#[derive(Debug, Clone, PartialEq)]
pub enum Namespace {
    MyFiles,
    SharedWithMe,
    Offline,
    Conflicts,
}

impl Namespace {
    fn as_str(&self) -> &'static str {
        match self {
            Namespace::MyFiles => "my_files",
            Namespace::SharedWithMe => "shared_with_me",
            Namespace::Offline => "offline",
            Namespace::Conflicts => "conflicts",
        }
    }

    fn from_str(s: &str) -> Self {
        match s {
            "shared_with_me" => Namespace::SharedWithMe,
            "offline" => Namespace::Offline,
            "conflicts" => Namespace::Conflicts,
            _ => Namespace::MyFiles,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ItemKind {
    File,
    Folder,
}

impl ItemKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ItemKind::File => "file",
            ItemKind::Folder => "folder",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "folder" => ItemKind::Folder,
            _ => ItemKind::File,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum PinState {
    Inherit,
    Pinned,
    Unpinned,
}

impl PinState {
    fn as_str(&self) -> &'static str {
        match self {
            PinState::Inherit => "inherit",
            PinState::Pinned => "pinned",
            PinState::Unpinned => "unpinned",
        }
    }

    fn from_str(s: &str) -> Self {
        match s {
            "pinned" => PinState::Pinned,
            "unpinned" => PinState::Unpinned,
            _ => PinState::Inherit,
        }
    }
}

/// Windows Cloud Files on-disk placeholder attribute bits, decoded into the
/// app's [`FileStatus`] / [`PinState`] vocabulary. **Pure + cross-platform** so
/// it is unit-testable on Linux: it takes a raw `u32` attribute mask (what
/// `GetFileAttributesW` returns) plus the row's current status, and returns the
/// *desired* status/pin — the caller (the Windows reconcile pass) does the delta
/// check against the live row and only persists when something actually changed.
///
/// Bit values are the `Win32_Storage_FileSystem` `FILE_ATTRIBUTE_*` constants
/// (hardcoded here as plain `u32` so this fn never references a windows-only
/// symbol):
/// - `RECALL_ON_DATA_ACCESS` (`0x0040_0000`) — the placeholder is *dehydrated*
///   (cloud-only). Cleared ⇒ the bytes are resident on disk (local).
/// - `PINNED` (`0x0008_0000`) — "Always keep on this device".
/// - `UNPINNED` (`0x0010_0000`) — "Free up space" / online-only pin.
///
/// ## Status rule (never fight the engine)
///
/// Status is only decoded when the current status is one of the two
/// *user/OS-owned* terminal states — [`FileStatus::Local`] or
/// [`FileStatus::CloudOnly`]. The transient/engine-owned states
/// (`Uploading` / `Downloading` / `Conflict` / `Error`) are left ALONE
/// (`None`) because a native Explorer attribute snapshot must never clobber an
/// in-flight transfer or a conflict the engine is mid-resolving.
///
/// Returned status is the *desired* one for Local/CloudOnly candidates; the
/// caller compares it to the live row and skips no-op writes.
///
/// ## Pin rule (don't fight inheritance)
///
/// `PINNED` set ⇒ [`PinState::Pinned`]; `UNPINNED` set ⇒ [`PinState::Unpinned`];
/// neither bit set ⇒ `None` (leave the row's pin as-is — most placeholders
/// simply inherit their parent's pin and carry no explicit bit).
pub fn decode_os_state(attrs: u32, current_status: FileStatus) -> (Option<FileStatus>, Option<PinState>) {
    const FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS: u32 = 0x0040_0000;
    const FILE_ATTRIBUTE_PINNED: u32 = 0x0008_0000;
    const FILE_ATTRIBUTE_UNPINNED: u32 = 0x0010_0000;

    let status = match current_status {
        // Only the user/OS-owned terminal states are reconciled from disk.
        FileStatus::Local | FileStatus::CloudOnly => {
            if attrs & FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS != 0 {
                Some(FileStatus::CloudOnly)
            } else {
                Some(FileStatus::Local)
            }
        }
        // Engine-owned: leave it alone. `Trashing` (a locally-deleted file whose
        // server-trash is pending) is included here so a native attribute
        // snapshot can never flip it back to `CloudOnly`/`Local` and re-seed the
        // placeholder we just removed.
        FileStatus::Uploading
        | FileStatus::Downloading
        | FileStatus::Conflict
        | FileStatus::Error
        | FileStatus::Trashing => None,
    };

    let pin = if attrs & FILE_ATTRIBUTE_PINNED != 0 {
        Some(PinState::Pinned)
    } else if attrs & FILE_ATTRIBUTE_UNPINNED != 0 {
        Some(PinState::Unpinned)
    } else {
        None
    };

    (status, pin)
}

#[derive(Debug, Clone, PartialEq)]
pub enum OperationKind {
    HydrateFile,

    PinTree,
    UploadVersion,
    UploadFile,
    CreateFolder,
    RenameFile,
    MoveFile,
    TrashFile,
    RestoreFile,
    RestoreVersion,
}

impl OperationKind {
    fn as_str(&self) -> &'static str {
        match self {
            OperationKind::HydrateFile => "hydrate_file",
            OperationKind::PinTree => "pin_tree",
            OperationKind::UploadVersion => "upload_version",
            OperationKind::UploadFile => "upload_file",
            OperationKind::CreateFolder => "create_folder",
            OperationKind::RenameFile => "rename_file",
            OperationKind::MoveFile => "move_file",
            OperationKind::TrashFile => "trash_file",
            OperationKind::RestoreFile => "restore_file",
            OperationKind::RestoreVersion => "restore_version",
        }
    }

    fn from_str(s: &str) -> Self {
        match s {
            "hydrate_file" => OperationKind::HydrateFile,
            "pin_tree" => OperationKind::PinTree,
            "upload_version" => OperationKind::UploadVersion,
            "create_folder" => OperationKind::CreateFolder,
            "rename_file" => OperationKind::RenameFile,
            "move_file" => OperationKind::MoveFile,
            "trash_file" => OperationKind::TrashFile,
            "restore_file" => OperationKind::RestoreFile,
            "restore_version" => OperationKind::RestoreVersion,
            _ => OperationKind::UploadFile,
        }
    }
}

/// Task 1697: what changed about an item, for the File Provider change log.
/// `Reparented` exists because the materialized-set filter (Apple's contract,
/// `NSFileProviderReplicatedExtension.h`) tests "old OR new parent is
/// materialized" for a MOVE — a plain `Modified` row cannot carry the old
/// parent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FpChangeKind {
    Created,
    Modified,
    Deleted,
    Reparented,
}

impl FpChangeKind {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            FpChangeKind::Created => "created",
            FpChangeKind::Modified => "modified",
            FpChangeKind::Deleted => "deleted",
            FpChangeKind::Reparented => "reparented",
        }
    }

    fn from_str(s: &str) -> Option<Self> {
        match s {
            "created" => Some(FpChangeKind::Created),
            "modified" => Some(FpChangeKind::Modified),
            "deleted" => Some(FpChangeKind::Deleted),
            "reparented" => Some(FpChangeKind::Reparented),
            _ => None,
        }
    }
}

/// One change-log row returned by [`StateDb::list_file_changes`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange {
    /// Opaque, strictly-ascending change-log cursor (also what `seq` stores).
    pub seq: i64,
    pub file_id: String,
    pub kind: FpChangeKind,
    /// Parent BEFORE the change (recorded at record time; survives the row).
    pub old_parent_id: Option<String>,
    /// Parent AFTER the change — read from the `files` row at record time.
    /// For a `Deleted` item this is the parent it was deleted FROM (the row is
    /// already gone by the time the consumer reads the log). The Swift
    /// enumerator maps this onto `parentItemIdentifier`.
    pub new_parent_id: Option<String>,
}

/// The sync-anchor wire encoding for change-log sequence `seq`: decimal ASCII
/// of the rowid. Strictly ascending by construction (`seq` is an AUTOINCREMENT
/// rowid), lexicographically ordered while under 10 digits, and far inside
/// Apple's 500-byte anchor budget.
fn anchor_bytes(seq: i64) -> Vec<u8> {
    seq.to_string().into_bytes()
}

/// Wall-clock seconds since the epoch (0 on a clock before 1970 — a degraded
/// stamp beats a panic in the change-log insert path).
fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Append one change to `fp_changes` and advance the persistent anchor
/// cursor, on an existing connection (the public [`StateDb::record_file_change`]
/// and the mutating row operations below share this).
fn record_file_change_conn<C: std::ops::Deref<Target = Connection>>(
    conn: &C,
    file_id: &str,
    kind: FpChangeKind,
    old_parent_id: Option<String>,
) -> Result<()> {
    // The NEW parent is whatever the row says NOW. `parent_id` lives in the
    // files column (written by set_file_contract_state).
    let new_parent_id: Option<String> = conn
        .query_row(
            "SELECT parent_id FROM files WHERE file_id = ?1",
            params![file_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten()
        // Deleted rows no longer exist: the change must still carry the
        // parent it was deleted FROM (the materialized-set filter reads it),
        // which the caller passes as old_parent_id.
        .or_else(|| old_parent_id.clone());
    // Task 1697 review fix: stamp the REAL insertion time. The old
    // `recorded_at = 0` sentinel made the signal path's 7-day sweep
    // (`sweep_file_changes(now - 7d)`) delete the ENTIRE fresh log right
    // after asking File Provider to enumerate it — Finder received an empty
    // feed and missed every update.
    let recorded_at = now_secs();
    conn.execute(
        "INSERT INTO fp_changes (seq, file_id, kind, old_parent_id, new_parent_id, recorded_at)
         VALUES (0, ?1, ?2, ?3, ?4, ?5)",
        params![file_id, kind.as_str(), old_parent_id, new_parent_id, recorded_at],
    )?;
    let seq = conn.last_insert_rowid();
    conn.execute("UPDATE fp_changes SET seq = ?1 WHERE id = ?1", params![seq])?;
    // The anchor cursor tracks the log tip even before any consumer reads:
    // `list_file_changes` never rewinds it, so a poll that races a write
    // cannot lose changes.
    conn.execute(
        "INSERT INTO fp_sync_anchor (domain_id, last_anchor) VALUES ('__default__', ?1)
         ON CONFLICT(domain_id) DO UPDATE SET last_anchor = MAX(last_anchor, excluded.last_anchor)",
        params![seq],
    )?;
    Ok(())
}

/// A row removed by [`StateDb::prune_absent`] (task 0806). Carries the minimum
/// needed to locate and delete the row's on-disk Cloud Files placeholder on
/// Windows: the `file_id` (for logging / dedupe), the server-relative `path`
/// (joined onto the sync root the same way `populate_placeholders` builds it),
/// and `is_dir` (a folder placeholder is removed with `remove_dir_all`, a file
/// with `remove_file`). Previously `prune_absent` returned only `Vec<String>`
/// (file_ids), which the snapshot reconcile path could LOG but not act on — so
/// the placeholder lingered in Explorer (the ghost-file bug).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrunedRow {
    pub file_id: String,
    pub path: String,
    pub is_dir: bool,
}

/// One row in the `files` table.
#[derive(Debug, Clone)]
pub struct FileEntry {
    pub file_id: String,
    pub path: String,
    pub status: FileStatus,
    pub size_bytes: i64,
    /// Seconds since Unix epoch. Conflated meaning today: usually the
    /// server's `updated_at` from the last sweep, but Task 10's
    /// conflict detector overwrites this with `now` when it flips a
    /// file to `Conflict` so the auto-resolution deadline (Task 13)
    /// can read it as "detected_at." This conflation is acceptable
    /// for MVP — the value is only meaningful in the context of the
    /// row's current `status`.
    pub modified_at: i64,
    pub content_hash: Option<String>,
    /// Server's `updated_at` snapshot at the time we last considered
    /// this file fully synced (status transitioned to `Local`). Used
    /// by [`crate::engine_bridge::sync_tick`] as the "base version"
    /// in [`crate::conflict::is_conflict`]: a remote `updated_at` past
    /// this value means a sibling device has touched the file since
    /// our last sync.
    ///
    /// Defaults to `0` for rows that pre-date this column — those
    /// will look like "remote always changed" until the next
    /// successful sync re-anchors them, which is the safe direction
    /// (over-detect rather than miss a real conflict).
    pub remote_updated_at: i64,
    /// Server UUID of this row's parent folder, or `None` at the vault
    /// root. **Read-only on `FileEntry`**: it is populated from the
    /// `files.parent_id` column by the SELECT mappers below, but
    /// [`StateDb::upsert_file`] does NOT write it — the parent linkage
    /// (and `item_kind`) is owned by [`StateDb::set_file_contract_state`]
    /// via [`FileContractState`], which every metadata sweep calls right
    /// after `upsert_file`. Exposing it here lets
    /// [`crate::windows_cf::populate_placeholders`] order parents-before-
    /// children and place nested placeholders without an N+1 contract fetch.
    pub parent_id: Option<String>,
    /// Whether this row is a folder or a file. **Read-only on `FileEntry`**
    /// for the same reason as [`Self::parent_id`]: read from the
    /// `files.item_kind` column, written only via
    /// [`StateDb::set_file_contract_state`]. The Windows Cloud Files layer
    /// uses it to mint a DIRECTORY placeholder (vs a file placeholder) so
    /// folders are real, openable directories in Explorer.
    pub item_kind: ItemKind,
}

impl FileEntry {
    /// True if this row is a folder. Used by the Windows Cloud Files
    /// placeholder seeder to decide between a directory and a file
    /// placeholder.
    pub fn is_dir(&self) -> bool {
        self.item_kind == ItemKind::Folder
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FileContractState {
    pub file_id: String,
    pub namespace: Namespace,
    pub parent_id: Option<String>,
    pub shared_root_id: Option<String>,
    pub share_id: Option<String>,
    pub owner_email: Option<String>,
    pub permission_bits: i64,
    pub item_kind: ItemKind,
    pub content_type: Option<String>,
    pub current_version: i64,
    pub current_object_version_id: Option<String>,
    pub local_base_version: i64,
    pub local_hash: Option<String>,
    pub cache_path: Option<String>,
    pub cache_bytes: i64,
    pub pin_state: PinState,
    pub inherited_pin_state: PinState,
    pub last_sync_at: i64,
}

impl FileContractState {
    pub fn effective_pin_state(&self) -> PinState {
        match self.pin_state {
            PinState::Inherit => self.inherited_pin_state.clone(),
            _ => self.pin_state.clone(),
        }
    }

    pub fn can_read(&self) -> bool {
        self.permission_bits & PERMISSION_READ != 0
    }

    pub fn can_write(&self) -> bool {
        self.permission_bits & PERMISSION_WRITE != 0
    }

    pub fn is_shared(&self) -> bool {
        self.namespace == Namespace::SharedWithMe
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RevokedSharedCache {
    pub file_id: String,
    pub cache_path: Option<String>,
}

/// Result of [`StateDb::purge_all_local_state`] (task 1538): every queued
/// operation and cached plaintext path that was cleared from the DB, for the
/// caller to delete from disk.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LocalStatePurge {
    /// Number of `operation_queue` rows deleted.
    pub queued_ops_purged: usize,
    /// Staged plaintext payload paths (`stage_finder_payload`) from the
    /// deleted `operation_queue` rows — the on-disk file at each path must
    /// still be removed by the caller.
    pub payload_paths: Vec<String>,
    /// Every `files.cache_path` that was cleared — the on-disk file at each
    /// path must still be removed by the caller.
    pub cache_paths: Vec<String>,
    /// `(file_id, server_relative_path)` for every row that was in `local`
    /// status (task 1538 Codex P1) — captured BEFORE the status flip, so a
    /// Windows Cloud Files placeholder (whose plaintext lives in the sync
    /// root, not at `cache_path`) can still be found and dehydrated/removed
    /// by the caller even though `cache_paths` above never pointed at it.
    /// Present regardless of platform (the query itself is cross-platform);
    /// only the Windows caller acts on it.
    pub local_placeholder_paths: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalActivityKind {
    MovedToTrash,
    Restored,
}

impl LocalActivityKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            LocalActivityKind::MovedToTrash => "moved_to_trash",
            LocalActivityKind::Restored => "restored",
        }
    }

    fn from_str(s: &str) -> Self {
        match s {
            "restored" => LocalActivityKind::Restored,
            _ => LocalActivityKind::MovedToTrash,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalActivityEventInput {
    pub event_type: LocalActivityKind,
    pub file_id: Option<String>,
    pub file_name: String,
    pub rel_path: Option<String>,
    pub occurred_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LocalActivityEvent {
    pub id: i64,
    pub event_type: LocalActivityKind,
    pub file_id: Option<String>,
    pub file_name: String,
    pub rel_path: Option<String>,
    pub occurred_at: i64,
}

/// One finished transfer, for the popover's "Recent activity" (task 1683 slice 2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferActivityInput {
    /// `"up"` (uploaded) or `"down"` (downloaded).
    pub direction: &'static str,
    pub file_id: Option<String>,
    pub file_name: String,
    pub rel_path: Option<String>,
    pub bytes: i64,
    pub occurred_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferActivity {
    pub id: i64,
    pub direction: String,
    pub file_id: Option<String>,
    pub file_name: String,
    pub rel_path: Option<String>,
    pub bytes: i64,
    pub occurred_at: i64,
}

/// What is waiting to transfer, read in one locked pass (task 1683 slice 2).
/// `upload_files` counts queued upload operations that are due to run (`next_retry_at` has
/// come, not paused, attempts left), so an upload sitting out a retry backoff is not a file
/// left; `download_*` counts rows currently `downloading`. The bytes are
/// the plaintext sizes of those files' rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TransferBacklog {
    pub upload_files: u32,
    pub upload_bytes: u64,
    pub download_files: u32,
    pub download_bytes: u64,
    /// Every queued operation (renames, trashes and uploads), paused or not.
    pub queued_ops: u32,
    /// Queued operations paused because the account is over its storage quota.
    pub paused_for_quota: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PendingOperation {
    pub op_id: String,
    pub kind: OperationKind,
    pub file_id: Option<String>,
    pub parent_id: Option<String>,
    pub target_path: Option<String>,
    pub metadata_json: Option<String>,
    pub payload_path: Option<String>,
    pub base_version: Option<i64>,
    pub base_object_version_id: Option<String>,
    pub attempts: i64,
    pub max_attempts: i64,
    pub next_retry_at: i64,
    pub last_error: Option<String>,
    /// Origin known-folder key (task 0811) when this op was enqueued by the
    /// known-folder backup mirror/upload (e.g. `"music"`); `None` for every
    /// normal user-initiated op. Disabling a folder's backup deletes exactly the
    /// queue rows carrying its key — see [`StateDb::purge_backup_source_ops`].
    pub backup_source_key: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Drop every persisted upload session whose queued op no longer exists.
/// Called by the bulk `operation_queue` purges so a session never outlives
/// its op (`remove_operation` already clears its own row).
fn drop_orphaned_upload_resumes(conn: &Connection) -> Result<usize> {
    conn.execute(
        "DELETE FROM upload_resume WHERE op_id NOT IN (SELECT op_id FROM operation_queue)",
        [],
    )
}

/// Persisted resumable upload session for one queued upload op (flow 7).
/// See the `upload_resume` table in [`StateDb::open`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadResume {
    pub op_id: String,
    pub payload_path: String,
    pub payload_size: i64,
    pub payload_mtime_ns: i64,
    pub upload_session_id: String,
    pub server_file_id: String,
    pub object_version_id: String,
    pub chunk_size_bytes: i64,
    pub chunk_count: i64,
    /// Chunks `0..acked_chunks` were acknowledged (2xx) by the server.
    pub acked_chunks: i64,
    /// The post-init `PATCH /files/{id}` (encrypted name/parent) succeeded.
    pub metadata_applied: bool,
    /// The op created a NEW server file (no prior version): on abandonment
    /// its `is_uploading` row is an orphan the client may trash.
    pub is_create: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum OperationPauseReason {
    Auth,
    Quota,
    Permission,
    Locked,
}

impl OperationPauseReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            OperationPauseReason::Auth => "auth",
            OperationPauseReason::Quota => "quota",
            OperationPauseReason::Permission => "permission",
            OperationPauseReason::Locked => "locked",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct QueueDiagnostics {
    pub queued: i64,
    pub due: i64,
    pub paused: i64,
    pub by_kind: BTreeMap<String, i64>,
    pub paused_by_reason: BTreeMap<String, i64>,
    /// Last queue error with secrets, paths and known file/folder names removed
    /// (task 1685). A name the state DB does not know survives only if it is a
    /// standard error word or digits. See [`crate::diagnostic_redaction`].
    pub last_error: Option<String>,
    /// Closed-enum classification of `last_error`; cannot carry a name.
    pub last_error_code: Option<DiagnosticErrorCode>,
    /// Number of placeholders (`[path]`, `[name]`, `[redacted]`) written into
    /// `last_error`. `0` with a non-null `last_error` means nothing was removed.
    pub last_error_redactions: u32,
    pub last_error_class: Option<String>,
}

/// Owned handle to the local SQLite state database.
///
/// One instance per running daemon process. The inner `Connection` is
/// wrapped in a `Mutex` so `StateDb: Send + Sync` and an `Arc<StateDb>`
/// can be shared with `tokio::spawn`-ed futures (rusqlite's
/// `Connection` is `Send` but `!Sync` on its own). Lock contention is
/// fine for our access pattern: the daemon does one tick every 5
/// seconds and OS-extension callbacks are infrequent.
pub struct StateDb(Mutex<Connection>);

impl StateDb {
    /// Open or create the state database at `path`. Idempotent — the
    /// schema migration uses `CREATE TABLE IF NOT EXISTS` so re-opens
    /// are cheap.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        // Base schema. `CREATE TABLE IF NOT EXISTS` covers both the
        // first-run case and subsequent opens against an existing DB
        // that already has every column we need.
        conn.execute_batch(
            "
            PRAGMA journal_mode = WAL;
            CREATE TABLE IF NOT EXISTS files (
                file_id TEXT PRIMARY KEY,
                path TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'cloud_only',
                size_bytes INTEGER NOT NULL DEFAULT 0,
                modified_at INTEGER NOT NULL DEFAULT 0,
                content_hash TEXT,
                remote_updated_at INTEGER NOT NULL DEFAULT 0,
                namespace TEXT NOT NULL DEFAULT 'my_files',
                parent_id TEXT,
                shared_root_id TEXT,
                share_id TEXT,
                owner_email TEXT,
                permission_bits INTEGER NOT NULL DEFAULT 0,
                item_kind TEXT NOT NULL DEFAULT 'file',
                content_type TEXT,
                current_version INTEGER NOT NULL DEFAULT 0,
                current_object_version_id TEXT,
                local_base_version INTEGER NOT NULL DEFAULT 0,
                local_hash TEXT,
                cache_path TEXT,
                cache_bytes INTEGER NOT NULL DEFAULT 0,
                pin_state TEXT NOT NULL DEFAULT 'inherit',
                inherited_pin_state TEXT NOT NULL DEFAULT 'unpinned',
                last_opened_at INTEGER NOT NULL DEFAULT 0,
                last_sync_at INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS idx_status ON files(status);
            CREATE TABLE IF NOT EXISTS operation_queue (
                op_id TEXT PRIMARY KEY,
                kind TEXT NOT NULL,
                file_id TEXT,
                parent_id TEXT,
                target_path TEXT,
                metadata_json TEXT,
                payload_path TEXT,
                base_version INTEGER,
                base_object_version_id TEXT,
                attempts INTEGER NOT NULL DEFAULT 0,
                max_attempts INTEGER NOT NULL DEFAULT 5,
                next_retry_at INTEGER NOT NULL DEFAULT 0,
                last_error TEXT,
                last_error_class TEXT,
                paused_reason TEXT,
                -- Origin known-folder key (task 0811) for backup-originated ops.
                -- NULL for every normal (non-backup) op. Disabling a known-folder
                -- backup purges exactly its tagged ops so nothing resumes on boot.
                backup_source_key TEXT,
                created_at INTEGER NOT NULL DEFAULT 0,
                updated_at INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS idx_operation_queue_due ON operation_queue(next_retry_at, created_at);
            CREATE TABLE IF NOT EXISTS sync_state (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS bandwidth_samples (
                sampled_at  INTEGER NOT NULL,
                up_bytes    INTEGER NOT NULL DEFAULT 0,
                down_bytes  INTEGER NOT NULL DEFAULT 0,
                period_secs INTEGER NOT NULL DEFAULT 20
            );
            CREATE INDEX IF NOT EXISTS idx_bandwidth_samples_at ON bandwidth_samples(sampled_at);
            CREATE TABLE IF NOT EXISTS local_activity (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                event_type TEXT NOT NULL,
                file_id TEXT,
                file_name TEXT NOT NULL,
                rel_path TEXT,
                occurred_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_local_activity_recent ON local_activity(occurred_at DESC, id DESC);
            -- Task 1683 slice 2: which way each recent transfer went (the popover's
            -- arrow). A separate table, not new `local_activity` kinds: that table's
            -- cap is 200 rows and a large sync would push the trash events out of it.
            CREATE TABLE IF NOT EXISTS transfer_activity (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                direction TEXT NOT NULL,
                file_id TEXT,
                file_name TEXT NOT NULL,
                rel_path TEXT,
                bytes INTEGER NOT NULL DEFAULT 0,
                occurred_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_transfer_activity_recent ON transfer_activity(occurred_at DESC, id DESC);
            -- Task 1697 (File Provider FPFS conformance): the daemon-side change
            -- log the replica's enumerator pages through. `id` is the change
            -- cursor: an AUTOINCREMENT rowid, so anchors are monotonic and
            -- compact (<= 500 bytes as decimal ASCII). `seq` mirrors `id` so the
            -- sweep can leave the anchor row while pruning consumed changes.
            -- `old_parent_id`/`new_parent_id` drive the materialized-set filter
            -- (old OR new parent materialized, per Apple's Replicated contract).
            -- A change is never rewritten once appended; readers page by cursor.
            CREATE TABLE IF NOT EXISTS fp_changes (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                seq INTEGER NOT NULL,
                file_id TEXT NOT NULL,
                kind TEXT NOT NULL,
                old_parent_id TEXT,
                new_parent_id TEXT,
                recorded_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_fp_changes_seq ON fp_changes(seq);
            -- The persistent, monotonic sync-anchor cursor the replica reads
            -- back after process death (keyed per domain id; one domain today).
            CREATE TABLE IF NOT EXISTS fp_sync_anchor (
                domain_id TEXT PRIMARY KEY,
                last_anchor INTEGER NOT NULL
            );
            -- The materialized container set: folders the SYSTEM reports as
            -- materialized on disk (via the extension's App Group journal,
            -- tracked here so the working-set filter can run daemon-side).
            CREATE TABLE IF NOT EXISTS fp_materialized (
                container_id TEXT PRIMARY KEY,
                updated_at INTEGER NOT NULL
            );
        ",
        )?;
        ensure_column(&conn, "files", "remote_updated_at", "INTEGER NOT NULL DEFAULT 0")?;
        ensure_column(&conn, "files", "namespace", "TEXT NOT NULL DEFAULT 'my_files'")?;
        ensure_column(&conn, "files", "parent_id", "TEXT")?;
        ensure_column(&conn, "files", "shared_root_id", "TEXT")?;
        ensure_column(&conn, "files", "share_id", "TEXT")?;
        ensure_column(&conn, "files", "owner_email", "TEXT")?;
        ensure_column(&conn, "files", "permission_bits", "INTEGER NOT NULL DEFAULT 0")?;
        ensure_column(&conn, "files", "item_kind", "TEXT NOT NULL DEFAULT 'file'")?;
        ensure_column(&conn, "files", "content_type", "TEXT")?;
        ensure_column(&conn, "files", "current_version", "INTEGER NOT NULL DEFAULT 0")?;
        ensure_column(&conn, "files", "current_object_version_id", "TEXT")?;
        ensure_column(&conn, "files", "local_base_version", "INTEGER NOT NULL DEFAULT 0")?;
        ensure_column(&conn, "files", "local_hash", "TEXT")?;
        ensure_column(&conn, "files", "cache_path", "TEXT")?;
        ensure_column(&conn, "files", "cache_bytes", "INTEGER NOT NULL DEFAULT 0")?;
        ensure_column(&conn, "files", "pin_state", "TEXT NOT NULL DEFAULT 'inherit'")?;
        ensure_column(
            &conn,
            "files",
            "inherited_pin_state",
            "TEXT NOT NULL DEFAULT 'unpinned'",
        )?;
        ensure_column(&conn, "files", "last_opened_at", "INTEGER NOT NULL DEFAULT 0")?;
        ensure_column(&conn, "files", "last_sync_at", "INTEGER NOT NULL DEFAULT 0")?;
        ensure_column(&conn, "operation_queue", "last_error_class", "TEXT")?;
        ensure_column(&conn, "operation_queue", "paused_reason", "TEXT")?;
        // Task 0811: additive backup-origin tag. Existing rows migrate to NULL
        // (untagged → treated as normal ops, never purged by a folder disable).
        ensure_column(&conn, "operation_queue", "backup_source_key", "TEXT")?;
        // Task 1697: which local write created the row (watcher paths set this;
        // Finder-queue paths do not) — the change-log recorder reads it.
        ensure_column(&conn, "files", "creator_for_fp", "TEXT")?;
        conn.execute_batch(
            "
            CREATE INDEX IF NOT EXISTS idx_files_namespace ON files(namespace);
            CREATE INDEX IF NOT EXISTS idx_files_shared_root ON files(shared_root_id);
            CREATE INDEX IF NOT EXISTS idx_operation_queue_paused ON operation_queue(paused_reason);
            CREATE INDEX IF NOT EXISTS idx_operation_queue_backup_source ON operation_queue(backup_source_key);
            -- Flow 7 (interrupted upload resume): the server upload session an
            -- UploadVersion/UploadFile op is writing to, persisted the moment
            -- `POST /uploads/init` returns and advanced after every
            -- acknowledged chunk, so a retry resumes instead of minting a
            -- second server file row. Keyed by op_id; the payload fingerprint
            -- (path + size + mtime) guards against resuming onto other bytes.
            CREATE TABLE IF NOT EXISTS upload_finalizations (
                op_id TEXT PRIMARY KEY,
                local_file_id TEXT NOT NULL,
                server_file_id TEXT NOT NULL,
                target_path TEXT NOT NULL,
                payload_path TEXT NOT NULL,
                stamped INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS staged_payloads (
                path TEXT PRIMARY KEY,
                source_path TEXT,
                completed INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS upload_resume (
                op_id TEXT PRIMARY KEY,
                payload_path TEXT NOT NULL,
                payload_size INTEGER NOT NULL,
                payload_mtime_ns INTEGER NOT NULL,
                upload_session_id TEXT NOT NULL,
                server_file_id TEXT NOT NULL,
                object_version_id TEXT NOT NULL,
                chunk_size_bytes INTEGER NOT NULL,
                chunk_count INTEGER NOT NULL,
                acked_chunks INTEGER NOT NULL DEFAULT 0,
                metadata_applied INTEGER NOT NULL DEFAULT 0,
                is_create INTEGER NOT NULL DEFAULT 0,
                updated_at INTEGER NOT NULL DEFAULT 0
            );
            ",
        )?;
        // Upgrade inventory: old one-shot Keep Mine rows may have no queue
        // owner. Preserve their plaintext reference before any resume purge.
        conn.execute("INSERT OR IGNORE INTO staged_payloads(path, completed) SELECT payload_path, 0 FROM upload_resume", [])?;
        Ok(Self(Mutex::new(conn)))
    }

    /// Insert or update a row keyed by `file_id`. ON CONFLICT replaces
    /// the entire row except the primary key.
    pub fn upsert_file(&self, e: &FileEntry) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        // Task 1697: capture the OLD row before the upsert so the change log
        // can (a) tell created from modified, (b) skip no-op re-upserts — the
        // metadata sweeps re-upsert every row every tick, and recording every
        // one would advance the anchor with no real change — and (c) carry
        // the old parent for reparent detection.
        let old: Option<(String, String, i64, i64, i64, Option<String>)> = conn
            .query_row(
                "SELECT path, status, size_bytes, modified_at, remote_updated_at, parent_id
                 FROM files WHERE file_id = ?1",
                params![e.file_id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get::<_, Option<String>>(5)?,
                    ))
                },
            )
            .optional()?;
        conn.execute(
            "INSERT INTO files (file_id, path, status, size_bytes, modified_at, content_hash, remote_updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(file_id) DO UPDATE SET
               path=excluded.path, status=excluded.status,
               size_bytes=excluded.size_bytes, modified_at=excluded.modified_at,
               content_hash=excluded.content_hash,
               remote_updated_at=excluded.remote_updated_at",
            params![
                e.file_id,
                e.path,
                e.status.as_str(),
                e.size_bytes,
                e.modified_at,
                e.content_hash,
                e.remote_updated_at
            ],
        )?;
        let changed = match &old {
            None => true,
            Some((path, status, size, modified_at, remote_updated_at, _)) => {
                path != &e.path
                    || status != e.status.as_str()
                    || *size != e.size_bytes
                    || *modified_at != e.modified_at
                    || *remote_updated_at != e.remote_updated_at
            }
        };
        if changed {
            // A path change is a rename OR a move; Reparented carries the old
            // parent so the materialized-set filter can test old-OR-new
            // (Apple's Replicated contract). Created/Modified need no old
            // parent.
            let (kind, old_parent): (FpChangeKind, Option<String>) = match &old {
                None => (FpChangeKind::Created, None),
                Some((old_path, _, _, _, _, old_parent)) => {
                    if old_path != &e.path {
                        (FpChangeKind::Reparented, old_parent.clone())
                    } else {
                        (FpChangeKind::Modified, None)
                    }
                }
            };
            record_file_change_conn(&conn, &e.file_id, kind, old_parent)?;
        }
        Ok(())
    }

    /// Fetch a single row by `file_id`. `Ok(None)` if absent.
    pub fn get_file(&self, file_id: &str) -> Result<Option<FileEntry>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT file_id, path, status, size_bytes, modified_at, content_hash, remote_updated_at,
                    parent_id, item_kind
             FROM files WHERE file_id = ?1",
        )?;
        let mut rows = stmt.query(params![file_id])?;
        if let Some(row) = rows.next()? {
            Ok(Some(FileEntry {
                file_id: row.get(0)?,
                path: row.get(1)?,
                status: FileStatus::from_str(&row.get::<_, String>(2)?),
                size_bytes: row.get(3)?,
                modified_at: row.get(4)?,
                content_hash: row.get(5)?,
                remote_updated_at: row.get(6)?,
                parent_id: row.get(7)?,
                item_kind: ItemKind::from_str(&row.get::<_, String>(8)?),
            }))
        } else {
            Ok(None)
        }
    }

    /// Fetch a single row by its path inside the sync root. `Ok(None)`
    /// if absent. Used by OS virtual-filesystem layers that receive
    /// path/inode callbacks instead of server UUIDs.
    /// Look up a file row by its server-relative path.
    ///
    /// Stored `files.path` values come from `resolve_relative_path`, which can
    /// yield EITHER a leading-slash form (`/leaf`, e.g. when the row falls
    /// through to a server-provided plaintext `path` field) OR a bare relative
    /// form (`docs/a.txt`). The upload watcher (`watcher::relative_db_path`)
    /// always queries the bare form. An exact-only match would therefore MISS a
    /// row stored as `/leaf` when the watcher asks for `leaf`, causing the
    /// watcher to treat an already-tracked server file as brand-new and
    /// re-upload it. To stay robust to that shape mismatch we try the path as
    /// given first, then the leading-slash-toggled variant. This is a read-only
    /// defense-in-depth lookup — it never mutates state.
    pub fn get_file_by_path(&self, path: &str) -> Result<Option<FileEntry>> {
        if let Some(entry) = self.get_file_by_exact_path(path)? {
            return Ok(Some(entry));
        }
        // Toggle the leading slash and try once more: `/leaf` ⇄ `leaf`.
        let alt = match path.strip_prefix('/') {
            Some(stripped) => stripped.to_string(),
            None => format!("/{path}"),
        };
        if alt == path {
            return Ok(None);
        }
        self.get_file_by_exact_path(&alt)
    }

    fn get_file_by_exact_path(&self, path: &str) -> Result<Option<FileEntry>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT file_id, path, status, size_bytes, modified_at, content_hash, remote_updated_at,
                    parent_id, item_kind
             FROM files WHERE path = ?1",
        )?;
        let mut rows = stmt.query(params![path])?;
        if let Some(row) = rows.next()? {
            Ok(Some(FileEntry {
                file_id: row.get(0)?,
                path: row.get(1)?,
                status: FileStatus::from_str(&row.get::<_, String>(2)?),
                size_bytes: row.get(3)?,
                modified_at: row.get(4)?,
                content_hash: row.get(5)?,
                remote_updated_at: row.get(6)?,
                parent_id: row.get(7)?,
                item_kind: ItemKind::from_str(&row.get::<_, String>(8)?),
            }))
        } else {
            Ok(None)
        }
    }

    pub fn delete_file(&self, file_id: &str) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        // Task 1697: capture the parent BEFORE the delete — the change row is
        // what tells the replica's materialized filter where the item was.
        let old_parent: Option<String> = conn
            .query_row(
                "SELECT parent_id FROM files WHERE file_id = ?1",
                params![file_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten();
        conn.execute("DELETE FROM files WHERE file_id = ?1", params![file_id])?;
        record_file_change_conn(&conn, file_id, FpChangeKind::Deleted, old_parent)?;
        Ok(())
    }

    /// Delete the row `file_id` AND — when it is a FOLDER — its whole descendant
    /// subtree by PATH-PREFIX, returning every removed row as a [`PrunedRow`]
    /// ordered CHILDREN-BEFORE-PARENT (deepest path first, then the root last).
    /// Task 0806: the OPS reconcile path (`apply_sync_op` for `file_trash` /
    /// `file_delete`) uses this so a remotely-trashed FOLDER also prunes its
    /// orphaned children (the server trash is NOT recursive — task 0807 — so a
    /// `file_trash` op for a folder is the ONLY signal its children are gone) and
    /// so the Windows caller can remove each on-disk placeholder, leaves first.
    ///
    /// The descendant match is the same metacharacter-safe `substr(path,…) = R || '/'`
    /// prefix equality used by [`Self::prune_absent`] and
    /// [`Self::cloud_only_file_descendants`] — `parent_id` is universally empty on
    /// live rows, so the hierarchy is path-based. A FILE row (or an unknown id)
    /// simply removes the single row. Returns an empty vec if the id is unknown.
    /// One transaction so a concurrent reader never sees a half-pruned subtree.
    /// Task 1698 (trash ruling — full sync): mark `file_id` and, for a folder,
    /// its whole path-prefix subtree as `Trashing` — the local mirror of the
    /// server's trash (`DELETE /files/{id}` sets `is_trashed=TRUE`; the
    /// server trash is the ONE trash model). Every flipped row records a
    /// `Modified` change so the replica moves it into the trash container
    /// (the payload builder parents top-of-trash rows there). Rows already
    /// `Trashing` are left untouched (no duplicate changes). Returns the
    /// affected rows (path + kind, `PrunedRow` shape) so the caller can still
    /// remove on-disk placeholders the way the delete path did.
    pub fn mark_subtree_trashing(&self, file_id: &str) -> Result<Vec<PrunedRow>> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction()?;

        // Same shape as `delete_file_subtree`: the row itself, plus (for a
        // folder) every descendant matched by PATH PREFIX — children always
        // stored leading-slash-free, the folder possibly leading-slash-first
        // (task 0806). Deepest-first ordering keeps the returned rows in the
        // children-before-parent order the placeholder remover expects.
        let root: Option<PrunedRow> = {
            let mut stmt = tx.prepare("SELECT file_id, path, item_kind FROM files WHERE file_id = ?1")?;
            let mut rows = stmt.query(params![file_id])?;
            if let Some(row) = rows.next()? {
                Some(PrunedRow {
                    file_id: row.get(0)?,
                    path: row.get(1)?,
                    is_dir: ItemKind::from_str(&row.get::<_, String>(2)?) == ItemKind::Folder,
                })
            } else {
                None
            }
        };
        let Some(root) = root else {
            tx.rollback()?;
            return Ok(Vec::new());
        };

        let mut marked: Vec<PrunedRow> = Vec::new();
        let mut mark = |tx: &rusqlite::Connection, row: &PrunedRow| -> Result<bool> {
            // Flip only when it changes anything: a no-op flip must not mint
            // a duplicate change row (same contract as `set_status`).
            let status: String = tx
                .query_row(
                    "SELECT status FROM files WHERE file_id = ?1",
                    params![row.file_id],
                    |r| r.get(0),
                )
                .optional()?
                .unwrap_or_default();
            if status == FileStatus::Trashing.as_str() {
                return Ok(false);
            }
            tx.execute(
                "UPDATE files SET status = ?1 WHERE file_id = ?2",
                params![FileStatus::Trashing.as_str(), row.file_id],
            )?;
            record_file_change_conn(&tx, &row.file_id, FpChangeKind::Modified, None)?;
            Ok(true)
        };

        if root.is_dir {
            let root_path = root.path.trim_matches('/').to_string();
            if !root_path.is_empty() {
                let descendants: Vec<PrunedRow> = {
                    let mut dstmt = tx.prepare(
                        "SELECT file_id, path, item_kind FROM files
                         WHERE namespace = 'my_files'
                           AND substr(ltrim(path, '/'), 1, length(?1) + 1) = ?1 || '/'
                         ORDER BY length(path) DESC, path DESC",
                    )?;
                    let drows = dstmt.query_map(params![root_path], |r| {
                        Ok(PrunedRow {
                            file_id: r.get(0)?,
                            path: r.get(1)?,
                            is_dir: ItemKind::from_str(&r.get::<_, String>(2)?) == ItemKind::Folder,
                        })
                    })?;
                    drows.collect::<Result<Vec<_>>>()?
                };
                for d in descendants {
                    if mark(&tx, &d)? {
                        marked.push(d);
                    }
                }
            }
        }
        if mark(&tx, &root)? {
            marked.push(root);
        }
        tx.commit()?;
        Ok(marked)
    }

    /// PR #100 review (Codex P1): the trash view's inverse of
    /// [`Self::mark_subtree_trashing`] — a restored FOLDER must take its
    /// whole held subtree out of the trash view, not just the root row (the
    /// authoritative re-snapshot preserves `Trashing` rows by design, so it
    /// can never repair the descendants). Flips every `Trashing` row in the
    /// subtree (the row itself plus, for a folder, every descendant matched
    /// by PATH PREFIX — same shape as `mark_subtree_trashing`) to `CloudOnly`,
    /// records a `Modified` change per actual flip, and returns the flipped
    /// rows deepest-first (children before parent). Rows not in `Trashing`
    /// are left untouched; a no-op flip records no change row. Same subtree
    /// matching contract as `mark_subtree_trashing` (children always stored
    /// leading-slash-free, the folder possibly leading-slash-first, task
    /// 0806), and the same caller contract: the caller removes on-disk
    /// placeholders and reports the returned ids for the working-set signal.
    pub fn untrash_subtree(&self, file_id: &str) -> Result<Vec<PrunedRow>> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction()?;

        let root: Option<PrunedRow> = {
            let mut stmt = tx.prepare("SELECT file_id, path, item_kind FROM files WHERE file_id = ?1")?;
            let mut rows = stmt.query(params![file_id])?;
            if let Some(row) = rows.next()? {
                Some(PrunedRow {
                    file_id: row.get(0)?,
                    path: row.get(1)?,
                    is_dir: ItemKind::from_str(&row.get::<_, String>(2)?) == ItemKind::Folder,
                })
            } else {
                None
            }
        };
        let Some(root) = root else {
            tx.rollback()?;
            return Ok(Vec::new());
        };

        let mut flipped: Vec<PrunedRow> = Vec::new();
        let mut flip = |tx: &rusqlite::Connection, row: &PrunedRow| -> Result<bool> {
            // Flip only Trashing rows: the restore must not touch live rows
            // that happen to sit in the subtree (e.g. a child the local
            // flow never parked), and a no-op must not mint a change row.
            let status: String = tx
                .query_row(
                    "SELECT status FROM files WHERE file_id = ?1",
                    params![row.file_id],
                    |r| r.get(0),
                )
                .optional()?
                .unwrap_or_default();
            if status != FileStatus::Trashing.as_str() {
                return Ok(false);
            }
            tx.execute(
                "UPDATE files SET status = ?1 WHERE file_id = ?2",
                params![FileStatus::CloudOnly.as_str(), row.file_id],
            )?;
            record_file_change_conn(&tx, &row.file_id, FpChangeKind::Modified, None)?;
            Ok(true)
        };

        if root.is_dir {
            let root_path = root.path.trim_matches('/').to_string();
            if !root_path.is_empty() {
                let descendants: Vec<PrunedRow> = {
                    let mut dstmt = tx.prepare(
                        "SELECT file_id, path, item_kind FROM files
                         WHERE namespace = 'my_files'
                           AND substr(ltrim(path, '/'), 1, length(?1) + 1) = ?1 || '/'
                         ORDER BY length(path) DESC, path DESC",
                    )?;
                    let drows = dstmt.query_map(params![root_path], |r| {
                        Ok(PrunedRow {
                            file_id: r.get(0)?,
                            path: r.get(1)?,
                            is_dir: ItemKind::from_str(&r.get::<_, String>(2)?) == ItemKind::Folder,
                        })
                    })?;
                    drows.collect::<Result<Vec<_>>>()?
                };
                for d in descendants {
                    if flip(&tx, &d)? {
                        flipped.push(d);
                    }
                }
            }
        }
        if flip(&tx, &root)? {
            flipped.push(root);
        }
        tx.commit()?;
        Ok(flipped)
    }

    pub fn delete_file_subtree(&self, file_id: &str) -> Result<Vec<PrunedRow>> {        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction()?;

        // Look up the row to delete (its path + kind drives the subtree prune).
        let root: Option<PrunedRow> = {
            let mut stmt = tx.prepare("SELECT file_id, path, item_kind FROM files WHERE file_id = ?1")?;
            let mut rows = stmt.query(params![file_id])?;
            if let Some(row) = rows.next()? {
                Some(PrunedRow {
                    file_id: row.get(0)?,
                    path: row.get(1)?,
                    is_dir: ItemKind::from_str(&row.get::<_, String>(2)?) == ItemKind::Folder,
                })
            } else {
                None
            }
        };
        let Some(root) = root else {
            tx.rollback()?;
            return Ok(Vec::new());
        };

        let mut removed: Vec<PrunedRow> = Vec::new();
        // Task 1697: capture each removed row's parent BEFORE its delete —
        // the change log must tell the replica's materialized filter where
        // each deleted item lived. (parent_id read per row inside the tx.)
        let old_parent_of = |tx: &rusqlite::Transaction, file_id: &str| -> Option<String> {
            tx.query_row(
                "SELECT parent_id FROM files WHERE file_id = ?1",
                params![file_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()
            .ok()
            .flatten()
            .flatten()
        };
        if root.is_dir {
            // Normalize BOTH ends (`trim_matches`), matching `placeholder_path_under`
            // / `safe_join_under_root`: a folder row can be stored leading-slash-
            // first (`/docs`) on the degraded-decrypt path while its children are
            // ALWAYS stored leading-slash-free (`docs/a.txt`). Trimming only the
            // trailing slash would make the prefix `/docs/` miss `docs/a.txt` and
            // orphan the children (task 0806 review, high). The descendant match
            // also `ltrim`s the stored path so a mixed-form child in either shape
            // is caught.
            let root_path = root.path.trim_matches('/').to_string();
            if !root_path.is_empty() {
                let descendants: Vec<PrunedRow> = {
                    let mut dstmt = tx.prepare(
                        "SELECT file_id, path, item_kind FROM files
                         WHERE substr(ltrim(path, '/'), 1, length(?1) + 1) = ?1 || '/'
                         ORDER BY length(path) DESC, path DESC",
                    )?;
                    let drows = dstmt.query_map(params![root_path], |r| {
                        Ok(PrunedRow {
                            file_id: r.get(0)?,
                            path: r.get(1)?,
                            is_dir: ItemKind::from_str(&r.get::<_, String>(2)?) == ItemKind::Folder,
                        })
                    })?;
                    drows.collect::<Result<Vec<_>>>()?
                };
                for d in descendants {
                    let old_parent = old_parent_of(&tx, &d.file_id);
                    tx.execute("DELETE FROM files WHERE file_id = ?1", params![d.file_id])?;
                    record_file_change_conn(&tx, &d.file_id, FpChangeKind::Deleted, old_parent)?;
                    removed.push(d);
                }
            }
        }

        let root_old_parent = old_parent_of(&tx, &root.file_id);
        tx.execute("DELETE FROM files WHERE file_id = ?1", params![root.file_id])?;
        record_file_change_conn(&tx, &root.file_id, FpChangeKind::Deleted, root_old_parent)?;
        removed.push(root);
        tx.commit()?;
        Ok(removed)
    }

    /// Append a local-only activity event and prune old rows. This table is the
    /// durable recent-activity surface for events whose canonical file row may
    /// legitimately disappear from `files` during sync convergence.
    pub fn record_local_activity(&self, input: LocalActivityEventInput) -> Result<()> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO local_activity (event_type, file_id, file_name, rel_path, occurred_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                input.event_type.as_str(),
                input.file_id,
                input.file_name,
                input.rel_path,
                input.occurred_at
            ],
        )?;
        tx.execute(
            "DELETE FROM local_activity
             WHERE id NOT IN (
               SELECT id FROM local_activity
               ORDER BY occurred_at DESC, id DESC
               LIMIT ?1
             )",
            params![LOCAL_ACTIVITY_MAX_ROWS as i64],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Return newest local activity events, capped by the caller's limit.
    pub fn list_recent_local_activity(&self, limit: usize) -> Result<Vec<LocalActivityEvent>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let limit = limit.min(LOCAL_ACTIVITY_MAX_ROWS);
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, event_type, file_id, file_name, rel_path, occurred_at
             FROM local_activity
             ORDER BY occurred_at DESC, id DESC
             LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |row| {
            let event_type = LocalActivityKind::from_str(&row.get::<_, String>(1)?);
            Ok(LocalActivityEvent {
                id: row.get(0)?,
                event_type,
                file_id: row.get(2)?,
                file_name: row.get(3)?,
                rel_path: row.get(4)?,
                occurred_at: row.get(5)?,
            })
        })?;
        rows.collect()
    }

    /// Record a finished upload or download and prune old rows.
    pub fn record_transfer_activity(&self, input: TransferActivityInput) -> Result<()> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO transfer_activity (direction, file_id, file_name, rel_path, bytes, occurred_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                input.direction,
                input.file_id,
                input.file_name,
                input.rel_path,
                input.bytes.max(0),
                input.occurred_at
            ],
        )?;
        tx.execute(
            "DELETE FROM transfer_activity
             WHERE id NOT IN (
               SELECT id FROM transfer_activity
               ORDER BY occurred_at DESC, id DESC
               LIMIT ?1
             )",
            params![TRANSFER_ACTIVITY_MAX_ROWS as i64],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Newest finished transfers first, capped by the caller's limit.
    pub fn list_recent_transfer_activity(&self, limit: usize) -> Result<Vec<TransferActivity>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let limit = limit.min(TRANSFER_ACTIVITY_MAX_ROWS);
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, direction, file_id, file_name, rel_path, bytes, occurred_at
             FROM transfer_activity
             ORDER BY occurred_at DESC, id DESC
             LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok(TransferActivity {
                id: row.get(0)?,
                direction: row.get(1)?,
                file_id: row.get(2)?,
                file_name: row.get(3)?,
                rel_path: row.get(4)?,
                bytes: row.get(5)?,
                occurred_at: row.get(6)?,
            })
        })?;
        rows.collect()
    }

    /// What is waiting to transfer (see [`TransferBacklog`]). `now` is unix seconds: an upload whose
    /// retry time is still ahead of it is waiting out a backoff and does not count.
    pub fn transfer_backlog(&self, now: i64) -> Result<TransferBacklog> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let (upload_files, upload_bytes): (i64, i64) = conn.query_row(
            "SELECT COUNT(*), COALESCE(SUM(MAX(COALESCE(f.size_bytes, 0), 0)), 0)
             FROM operation_queue q LEFT JOIN files f ON f.file_id = q.file_id
             WHERE q.kind IN ('upload_file', 'upload_version')
               AND q.next_retry_at <= ?1
               AND q.paused_reason IS NULL AND q.attempts < q.max_attempts",
            params![now],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let (download_files, download_bytes): (i64, i64) = conn.query_row(
            "SELECT COUNT(*), COALESCE(SUM(MAX(size_bytes, 0)), 0) FROM files WHERE status = 'downloading'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let queued_ops: i64 = conn.query_row("SELECT COUNT(*) FROM operation_queue", [], |row| row.get(0))?;
        let paused_for_quota: i64 = conn.query_row(
            "SELECT COUNT(*) FROM operation_queue WHERE paused_reason = 'quota'",
            [],
            |row| row.get(0),
        )?;
        let to_u32 = |n: i64| u32::try_from(n.max(0)).unwrap_or(u32::MAX);
        Ok(TransferBacklog {
            upload_files: to_u32(upload_files),
            upload_bytes: upload_bytes.max(0) as u64,
            download_files: to_u32(download_files),
            download_bytes: download_bytes.max(0) as u64,
            queued_ops: to_u32(queued_ops),
            paused_for_quota: to_u32(paused_for_quota),
        })
    }

    /// Sweep and delete every descendant of `folder_id` from the DB when the
    /// folder's OWN row is ABSENT (the "ghost children" bug, task 0828).
    ///
    /// ## When is this needed?
    ///
    /// `delete_file_subtree` (used by the normal `file_trash`/`file_delete`
    /// reconcile path) first looks up the folder's own row to obtain its `path`,
    /// then prunes descendants via a PATH-PREFIX query.  When the folder row is
    /// absent — because the folder was already trashed on the server when this
    /// desktop's snapshot was taken, so the snapshot excluded it, but its
    /// CHILDREN were already ingested (stored with bare leaf-name paths) — there
    /// is no path to start from, so `delete_file_subtree` returns an empty vec
    /// and the children are never removed.  They then show as permanent ghost
    /// placeholders in Explorer.
    ///
    /// ## How this fixes it
    ///
    /// The snapshot ingest path (`apply_metadata_file_row`) calls
    /// `set_file_contract_state` right after `upsert_file`, which writes the
    /// server's `parent_id` into `files.parent_id`.  So even when the parent
    /// FOLDER row is absent, its children carry `files.parent_id = folder_id`
    /// and can be found by a direct column scan — which is exactly what this
    /// function does.
    ///
    /// For any found child that is itself a folder we additionally remove its
    /// descendants via the same PATH-PREFIX sweep `delete_file_subtree` uses,
    /// so multi-level subtrees are fully pruned.
    ///
    /// All removals happen in a SINGLE transaction so a concurrent reader
    /// never sees a half-pruned subtree.  Returns the removed rows ordered
    /// CHILDREN-BEFORE-PARENTS (deepest path first) so the Windows caller can
    /// remove leaf placeholders before their containing directory placeholder.
    /// Returns an empty vec if no children are found (idempotent).
    ///
    /// ## Scope guarantee
    ///
    /// Only rows whose `parent_id` column equals `folder_id` (and their
    /// path-prefix descendants) are ever touched.  No broader match is possible:
    /// the query is `WHERE parent_id = ?1` with a single bound parameter.
    pub fn delete_orphaned_children_of_absent_folder(&self, folder_id: &str) -> Result<Vec<PrunedRow>> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction()?;

        // Find every direct child whose parent_id matches the absent folder.
        // Using `parent_id` (not path-prefix) because we have no path to start
        // from — that is precisely the condition that triggered this call.
        // Task 1698: `Trashing` children are NOT swept — they are the server
        // trash's content (each has its own `file_trash` op converging it
        // into the trash view); deleting them would erase trash-view rows.
        let direct_children: Vec<PrunedRow> = {
            let mut stmt = tx.prepare(
                "SELECT file_id, path, item_kind FROM files WHERE parent_id = ?1 AND status != 'trashing'",
            )?;
            let rows = stmt.query_map(params![folder_id], |r| {
                Ok(PrunedRow {
                    file_id: r.get(0)?,
                    path: r.get(1)?,
                    is_dir: ItemKind::from_str(&r.get::<_, String>(2)?) == ItemKind::Folder,
                })
            })?;
            rows.collect::<Result<Vec<_>>>()?
        };

        if direct_children.is_empty() {
            tx.rollback()?;
            return Ok(Vec::new());
        }

        // For the children-before-parents ordering we collect: deep descendants
        // first (for each child folder, via path-prefix), then the direct
        // children themselves (folders after their leaves).
        let mut removed: Vec<PrunedRow> = Vec::new();

        for child in &direct_children {
            if child.is_dir {
                // Remove the folder's descendants first (children-before-parent
                // ordering within this subtree).  Mirror the path-prefix sweep
                // in `delete_file_subtree`.
                let child_path = child.path.trim_matches('/').to_string();
                if !child_path.is_empty() {
                    let descendants: Vec<PrunedRow> = {
                        let mut dstmt = tx.prepare(
                            "SELECT file_id, path, item_kind FROM files
                             WHERE substr(ltrim(path, '/'), 1, length(?1) + 1) = ?1 || '/'
                             ORDER BY length(path) DESC, path DESC",
                        )?;
                        let drows = dstmt.query_map(params![child_path], |r| {
                            Ok(PrunedRow {
                                file_id: r.get(0)?,
                                path: r.get(1)?,
                                is_dir: ItemKind::from_str(&r.get::<_, String>(2)?) == ItemKind::Folder,
                            })
                        })?;
                        drows.collect::<Result<Vec<_>>>()?
                    };
                    for d in descendants {
                        tx.execute("DELETE FROM files WHERE file_id = ?1", params![d.file_id])?;
                        removed.push(d);
                    }
                }
            }
        }

        // Delete the direct children and append them last (after their own
        // descendants) to maintain children-before-parent ordering.
        for child in direct_children {
            tx.execute("DELETE FROM files WHERE file_id = ?1", params![child.file_id])?;
            removed.push(child);
        }

        tx.commit()?;
        Ok(removed)
    }

    // ── /sync delta-engine cursor + prune (task 0789) ──────────────────────────
    //
    // The `sync_state` kv table holds the `/sync/ops` cursor (the highest
    // `seq_id` we have applied). On boot the cursor is unset → the engine pulls
    // a full `/sync/snapshot` (authoritative tree + seq_id), then advances the
    // cursor by `seq_id` as it applies each delta op. This replaces the old
    // per-tick full-tree `/files` re-walk and is what fixes the silent
    // deletion-reconciliation bug (the snapshot path prunes server-deleted
    // rows; the ops path applies trash/delete ops).

    const SYNC_CURSOR_KEY: &'static str = "sync_ops_cursor";
    const NEEDS_RESNAPSHOT_KEY: &'static str = "sync_needs_resnapshot";

    /// Read the persisted `/sync/ops` cursor (highest applied `seq_id`).
    /// `Ok(None)` when unset (fresh DB / never bootstrapped) — the caller treats
    /// that as "needs a full snapshot". A stored-but-unparseable value is also
    /// reported as `None` (forces a safe re-bootstrap rather than a panic).
    pub fn get_sync_cursor(&self) -> Result<Option<i64>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let raw: Option<String> = conn
            .query_row(
                "SELECT value FROM sync_state WHERE key = ?1",
                params![Self::SYNC_CURSOR_KEY],
                |row| row.get(0),
            )
            .optional()?;
        Ok(raw.and_then(|s| s.parse::<i64>().ok()))
    }

    /// Persist the `/sync/ops` cursor (highest applied `seq_id`). Stored as TEXT
    /// so the kv table stays type-agnostic; idempotent upsert on the fixed key.
    pub fn set_sync_cursor(&self, seq_id: i64) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute(
            "INSERT INTO sync_state (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![Self::SYNC_CURSOR_KEY, seq_id.to_string()],
        )?;
        Ok(())
    }

    /// Mark that the next `sync_tick` must re-bootstrap from a fresh
    /// `/sync/snapshot` regardless of the cursor. Used by gap-recovery cases the
    /// delta path cannot reconcile from the op alone — notably `file_restore`,
    /// whose op payload is only `{ id }`, so the row it un-trashes cannot be
    /// rebuilt without the authoritative snapshot. Persisted so the request
    /// survives a restart between ticks; idempotent.
    pub fn request_resnapshot(&self) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute(
            "INSERT INTO sync_state (key, value) VALUES (?1, '1')
             ON CONFLICT(key) DO UPDATE SET value = '1'",
            params![Self::NEEDS_RESNAPSHOT_KEY],
        )?;
        Ok(())
    }

    /// Atomically read-and-clear the "needs re-snapshot" flag. Returns `true`
    /// exactly once per [`Self::request_resnapshot`] call, so the bootstrap runs
    /// on the very next tick and not on every subsequent tick. The DELETE in the
    /// same locked critical section makes the take-and-clear race-free against a
    /// concurrent `request_resnapshot`.
    pub fn take_needs_resnapshot(&self) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let present: Option<String> = conn
            .query_row(
                "SELECT value FROM sync_state WHERE key = ?1",
                params![Self::NEEDS_RESNAPSHOT_KEY],
                |row| row.get(0),
            )
            .optional()?;
        if present.is_some() {
            conn.execute(
                "DELETE FROM sync_state WHERE key = ?1",
                params![Self::NEEDS_RESNAPSHOT_KEY],
            )?;
        }
        Ok(present.is_some())
    }

    /// Reconcile a fresh `/sync/snapshot` against the local mirror: delete every
    /// OWN-tree row whose `file_id` is NOT in `seen_file_ids` (the snapshot is
    /// authoritative for the user's non-trashed tree, so an absent row was
    /// deleted/trashed server-side). Returns the deleted rows as [`PrunedRow`]s
    /// (file_id + path + is_dir) so the caller can locate and remove each row's
    /// on-disk Cloud Files placeholder + cache — NOT just the bare file_ids
    /// (task 0806: the snapshot path used to only LOG the pruned ids, leaving the
    /// placeholder ghosting in Explorer).
    ///
    /// ## Orphan-subtree removal (no recursive server trash — task 0807)
    ///
    /// The server's trash is NOT recursive: trashing a FOLDER removes only the
    /// folder node from the snapshot; its CHILDREN remain present in `seen`. So a
    /// pruned folder's descendants are NOT caught by the snapshot-absence test
    /// above — they'd be left orphaned (rows + placeholders pointing at a deleted
    /// parent). For every pruned FOLDER row we therefore additionally remove its
    /// DESCENDANTS by PATH-PREFIX (the hierarchy is path-based — `parent_id` is
    /// universally empty on live rows, see [`Self::cloud_only_file_descendants`]),
    /// using the same metacharacter-safe `substr(path,…) = R || '/'` prefix
    /// equality. Descendants are returned in the result too, ordered
    /// CHILDREN-BEFORE-PARENTS (deepest path first) so the Windows caller can
    /// remove leaf placeholders before their containing directory placeholder.
    /// Descendants are removed even if they appear in `seen` (an orphan whose
    /// trashed parent is gone is itself gone).
    ///
    /// ## What is intentionally NEVER pruned
    ///
    /// 1. **Shared rows** (`namespace != 'my_files'`). The snapshot only returns
    ///    the user's OWN tree; shared-with-me content has its own purge path
    ///    ([`Self::purge_revoked_shared_content`]). Pruning here would wrongly
    ///    nuke every shared file on the first snapshot.
    /// 2. **Rows with a pending operation** (`file_id` present in
    ///    `operation_queue`). A locally-created-but-not-yet-uploaded file lives
    ///    in the mirror under a CLIENT-minted UUID (re-keyed to the server id by
    ///    `defer_local_upload_finalization` only AFTER the upload completes),
    ///    and is referenced by its `operation_queue` row. That client UUID is
    ///    NOT in the server snapshot's `seen` set, so without this guard the very
    ///    next snapshot would delete the user's in-flight upload. The
    ///    `operation_queue` join is the authoritative "do not touch" signal.
    /// 3. **Rows still `uploading`** — belt-and-suspenders for the same in-flight
    ///    case, in the narrow window where the queue row was already consumed but
    ///    the re-key hasn't landed.
    ///
    /// 4. **Rows stamped at/after `snapshot_fetched_at`** — a row whose
    ///    `remote_updated_at >= snapshot_fetched_at` was touched locally (e.g. a
    ///    just-completed upload re-keyed to the server id via
    ///    `apply_completed_upload`, which stamps `remote_updated_at = now`) AT OR
    ///    AFTER the snapshot was taken, so the server snapshot legitimately
    ///    predates it and CANNOT be authoritative about its existence. Pruning it
    ///    would delete a freshly-uploaded file whenever the server snapshot lags
    ///    the upload by a tick. The re-keyed server row is `Local` and no longer
    ///    has an `operation_queue` entry (the queue row is keyed on the old client
    ///    UUID and removed by `process_due_operations`), so guards (2)/(3) miss
    ///    it — this freshness cutoff is what protects it.
    ///
    /// ## Empty-snapshot safety
    ///
    /// An EMPTY `seen` set against a NON-empty prunable own-tree is treated as a
    /// suspicious/degraded snapshot (server bug, degraded replica, a future
    /// paginated response with no `has_more` contract) and is REFUSED — we return
    /// `Ok(vec![])` without deleting anything. Refusing costs at worst a stale row
    /// for one tick; wrongly pruning on a spurious empty snapshot destroys the
    /// user's whole tree in one transaction. A genuinely-emptied vault converges
    /// the moment the next non-empty snapshot (or the per-row trash/delete ops)
    /// arrives, so this fail-closed choice loses no correctness, only immediacy.
    ///
    /// Done in ONE transaction so a concurrent reader never sees a half-pruned
    /// tree.
    pub fn prune_absent(&self, seen_file_ids: &HashSet<String>, snapshot_fetched_at: i64) -> Result<Vec<PrunedRow>> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction()?;
        // Candidate set: own-tree rows that are NOT pending an upload/op, not
        // mid-upload, and NOT stamped at/after the snapshot fetch time (a row a
        // recent local completion just touched can't be contradicted by an older
        // snapshot). Everything else is off-limits per the doc above. We now also
        // pull `path` + `item_kind` so the caller can locate the on-disk
        // placeholder and (for a folder) we can prune its orphaned descendants.
        //
        // Task 1698 (trash ruling): `Trashing` rows are the macOS Trash view
        // and the snapshot NEVER lists trashed files — absence is now their
        // steady state, not a convergence signal. Rows whose PARENT row is
        // `Trashing` are that trash view's folder content (same server trash)
        // and are equally protected. Removal converges through the server's
        // `file_delete` (permanent) / `file_restore` ops instead.
        let candidates: Vec<PrunedRow> = {
            let mut stmt = tx.prepare(
                "SELECT file_id, path, item_kind FROM files
                 WHERE namespace = 'my_files'
                   AND status != 'uploading'
                   AND status != 'trashing'
                   AND remote_updated_at < ?1
                   AND (parent_id IS NULL OR parent_id NOT IN
                        (SELECT file_id FROM files WHERE status = 'trashing'))
                   AND file_id NOT IN (
                       SELECT file_id FROM operation_queue WHERE file_id IS NOT NULL
                   )",
            )?;
            let rows = stmt.query_map(params![snapshot_fetched_at], |row| {
                Ok(PrunedRow {
                    file_id: row.get(0)?,
                    path: row.get(1)?,
                    is_dir: ItemKind::from_str(&row.get::<_, String>(2)?) == ItemKind::Folder,
                })
            })?;
            rows.collect::<Result<Vec<_>>>()?
        };

        // Empty-snapshot guard: if the snapshot says "nothing exists" but we DO
        // have prunable own-tree rows, the snapshot is almost certainly degraded
        // (empty 200, replica lag, or an unannounced pagination cut). Fail closed
        // — refuse to prune the entire tree on a single suspicious empty list.
        if seen_file_ids.is_empty() && !candidates.is_empty() {
            tx.rollback()?;
            tracing::warn!(
                local_prunable = candidates.len(),
                "prune_absent: refusing to prune — empty snapshot against a non-empty own-tree (suspected degraded snapshot)"
            );
            return Ok(Vec::new());
        }

        // Track everything removed (directly absent + orphaned descendants) so we
        // never delete or return a row twice (a pruned folder and an
        // independently-absent child both reaching the descendant sweep).
        let mut removed_ids: HashSet<String> = HashSet::new();
        let mut pruned: Vec<PrunedRow> = Vec::new();

        for row in candidates {
            if seen_file_ids.contains(&row.file_id) {
                continue;
            }
            if !removed_ids.insert(row.file_id.clone()) {
                continue; // already removed as a descendant of an earlier folder
            }

            // ORPHAN SUBTREE (task 0806/0807): a trashed FOLDER leaves its children
            // in the snapshot (no recursive server trash), so they won't be caught
            // by the absence test. Remove the folder's descendants by PATH-PREFIX
            // FIRST (children-before-parent) and emit them ahead of the folder so
            // the Windows caller removes leaf placeholders before the directory.
            if row.is_dir {
                // `trim_matches` (both ends), matching `placeholder_path_under`:
                // a folder stored leading-slash-first (`/docs`, degraded-decrypt
                // path) has children stored leading-slash-free (`docs/a.txt`), so
                // trimming only the trailing slash would orphan them (task 0806
                // review, high). The match `ltrim`s the stored path so a child in
                // either shape is caught.
                let root_path = row.path.trim_matches('/').to_string();
                if !root_path.is_empty() {
                    let descendants: Vec<PrunedRow> = {
                        // Strict descendants: `substr(ltrim(path,'/'),1,len(R)+1)
                        // = R || '/'` — metacharacter-safe prefix equality (NOT
                        // `LIKE`), the same machinery as `cloud_only_file_descendants`.
                        // Excludes the root row itself. Deepest-first so children
                        // precede their parent dirs in the returned order. Descendants
                        // with a pending op / mid-upload are NOT excluded here: their
                        // parent is gone server-side, so the orphan must go too —
                        // any stale queued op against it is moot. Task 1698:
                        // `Trashing` descendants are NOT excluded here by a pending
                        // op, but ARE excluded by STATUS — they are the server
                        // trash's content (recoverable), not dead rows; deleting
                        // them would erase the trash view's folder contents.
                        let mut dstmt = tx.prepare(
                            "SELECT file_id, path, item_kind FROM files
                             WHERE namespace = 'my_files'
                               AND status != 'trashing'
                               AND substr(ltrim(path, '/'), 1, length(?1) + 1) = ?1 || '/'
                             ORDER BY length(path) DESC, path DESC",
                        )?;
                        let drows = dstmt.query_map(params![root_path], |r| {
                            Ok(PrunedRow {
                                file_id: r.get(0)?,
                                path: r.get(1)?,
                                is_dir: ItemKind::from_str(&r.get::<_, String>(2)?) == ItemKind::Folder,
                            })
                        })?;
                        drows.collect::<Result<Vec<_>>>()?
                    };
                    for d in descendants {
                        if removed_ids.insert(d.file_id.clone()) {
                            let old_parent = tx
                                .query_row(
                                    "SELECT parent_id FROM files WHERE file_id = ?1",
                                    params![d.file_id],
                                    |row| row.get::<_, Option<String>>(0),
                                )
                                .optional()
                                .ok()
                                .flatten()
                                .flatten();
                            tx.execute("DELETE FROM files WHERE file_id = ?1", params![d.file_id])?;
                            record_file_change_conn(&tx, &d.file_id, FpChangeKind::Deleted, old_parent)?;
                            pruned.push(d);
                        }
                    }
                }
            }

            let old_parent = tx
                .query_row(
                    "SELECT parent_id FROM files WHERE file_id = ?1",
                    params![row.file_id],
                    |row2| row2.get::<_, Option<String>>(0),
                )
                .optional()
                .ok()
                .flatten()
                .flatten();
            tx.execute("DELETE FROM files WHERE file_id = ?1", params![row.file_id])?;
            record_file_change_conn(&tx, &row.file_id, FpChangeKind::Deleted, old_parent)?;
            pruned.push(row);
        }
        tx.commit()?;
        Ok(pruned)
    }

    /// Update just the `status` column for a known file. No-op if
    /// `file_id` doesn't exist; callers should pair with `upsert_file`
    /// when they want create-or-update semantics.
    pub fn set_status(&self, file_id: &str, status: FileStatus) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        // Task 1697: a status flip changes the item's metadata (isUploaded
        // gating etc.) — record it so the replica's metadata version moves.
        // A no-op flip records nothing (compare first).
        let old_status: Option<String> = conn
            .query_row(
                "SELECT status FROM files WHERE file_id = ?1",
                params![file_id],
                |row| row.get(0),
            )
            .optional()?;
        conn.execute(
            "UPDATE files SET status = ?1 WHERE file_id = ?2",
            params![status.as_str(), file_id],
        )?;
        if old_status.as_deref() != Some(status.as_str()) {
            record_file_change_conn(&conn, file_id, FpChangeKind::Modified, None)?;
        }
        Ok(())
    }

    /// A local content write was queued: the row is `Uploading` and describes
    /// the bytes the write holds (`size_bytes`, `modified_at`). Path, content
    /// hash and the remote stamp are left alone, so the content version does
    /// not move until the upload lands. A missing row is left missing.
    pub fn record_local_write(&self, file_id: &str, size_bytes: i64, modified_at: i64) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let old: Option<(String, i64, i64)> = conn
            .query_row(
                "SELECT status, size_bytes, modified_at FROM files WHERE file_id = ?1",
                params![file_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let Some((status, old_size, old_modified_at)) = old else {
            return Ok(());
        };
        conn.execute(
            "UPDATE files SET status = ?1, size_bytes = ?2, modified_at = ?3 WHERE file_id = ?4",
            params![FileStatus::Uploading.as_str(), size_bytes, modified_at, file_id],
        )?;
        if status != FileStatus::Uploading.as_str() || old_size != size_bytes || old_modified_at != modified_at {
            record_file_change_conn(&conn, file_id, FpChangeKind::Modified, None)?;
        }
        Ok(())
    }

    /// Clear transient transfer states left behind by a previous process.
    ///
    /// `Uploading` and `Downloading` mean "the current engine is actively moving
    /// bytes". After a crash, abort, logout, or process kill there is no active
    /// transfer owning those rows anymore, so startup must reconcile them before
    /// tray/status snapshots count them as live work:
    /// - `Downloading` falls back to `CloudOnly`; a later open/pin can rehydrate.
    /// - `Uploading` becomes `Error`; the durable operation queue still carries
    ///   any staged upload retry, but the row is not reported as in-flight.
    pub fn reconcile_stale_in_flight_on_startup(&self) -> Result<usize> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute(
            "UPDATE files
             SET status = CASE status
                WHEN 'downloading' THEN 'cloud_only'
                WHEN 'uploading' THEN 'error'
                ELSE status
             END
             WHERE status IN ('downloading', 'uploading')",
            [],
        )
    }

    /// Apply a batch of native-Explorer-derived state deltas in ONE
    /// transaction. Each tuple is `(file_id, status_opt, pin_opt)` where a
    /// `Some` means "the on-disk placeholder disagrees with the DB, write this";
    /// a `None` means "leave that column untouched".
    ///
    /// Drives the Windows per-tick reconcile pass
    /// ([`crate::windows_cf::reconcile_placeholder_state`]): a NATIVE pin /
    /// free-up the user did in Explorer (not through the in-app pin path) shows
    /// up here as a delta and is written back so the app's view matches reality.
    /// Callers pass DELTAS ONLY — the reconcile pass compares each desired value
    /// to the live row and never enqueues a no-op — so this method stays churn-
    /// free in steady state. The in-app pin path writes DB + OS together, so the
    /// next reconcile sees no delta for those files.
    ///
    /// - A `Some(status)` updates `status` and re-stamps `last_sync_at = now`.
    /// - A `Some(pin)` updates `pin_state` only (inheritance/effective resolution
    ///   is unchanged — we only persist the explicit per-file pin the OS reports).
    ///
    /// Returns the number of rows touched (a row updated for both status and pin
    /// counts the affected-row total across both statements). Empty input is a
    /// fast `Ok(0)` with no transaction opened.
    pub fn reconcile_os_state(
        &self,
        deltas: &[(String, Option<FileStatus>, Option<PinState>)],
        now: i64,
    ) -> Result<usize> {
        if deltas.is_empty() {
            return Ok(0);
        }
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction()?;
        let mut touched = 0usize;
        for (file_id, status_opt, pin_opt) in deltas {
            if let Some(status) = status_opt {
                touched += tx.execute(
                    "UPDATE files SET status = ?1, last_sync_at = ?2 WHERE file_id = ?3",
                    params![status.as_str(), now, file_id],
                )?;
            }
            if let Some(pin) = pin_opt {
                touched += tx.execute(
                    "UPDATE files SET pin_state = ?1 WHERE file_id = ?2",
                    params![pin.as_str(), file_id],
                )?;
            }
        }
        tx.commit()?;
        Ok(touched)
    }

    /// Correct the logical plaintext `size_bytes` for a known file row.
    ///
    /// Used by the Windows Cloud Files hydration resolve-or-error path
    /// (task 0783): when a placeholder's recorded `size_bytes` (mirrored
    /// from the server) disagrees with the AES-256-GCM-authenticated
    /// decrypted plaintext length, the decrypted length is ground truth —
    /// so we rewrite the row to the true plaintext size. The next
    /// placeholder (re)creation then mints an aligned size and hydration
    /// succeeds. Returns the number of rows affected (0 if `file_id` is
    /// absent). A negative size is clamped to 0: a caller should never
    /// pass a negative length, but clamping keeps a pathological value
    /// from corrupting the column.
    pub fn set_size_bytes(&self, file_id: &str, size_bytes: i64) -> Result<usize> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let updated = conn.execute(
            "UPDATE files SET size_bytes = ?1 WHERE file_id = ?2",
            params![size_bytes.max(0), file_id],
        )?;
        // Task 1697: a size change moves the metadata version — record it.
        if updated > 0 {
            record_file_change_conn(&conn, file_id, FpChangeKind::Modified, None)?;
        }
        Ok(updated)
    }

    /// Return every row whose `status` matches. Used by the conflict
    /// resolution UI (status = Conflict) and the daemon's
    /// "what still needs uploading?" sweep (status = Uploading).
    pub fn list_by_status(&self, status: FileStatus) -> Result<Vec<FileEntry>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT file_id, path, status, size_bytes, modified_at, content_hash, remote_updated_at,
                    parent_id, item_kind
             FROM files WHERE status = ?1",
        )?;
        let rows = stmt.query_map(params![status.as_str()], |row| {
            Ok(FileEntry {
                file_id: row.get(0)?,
                path: row.get(1)?,
                status: FileStatus::from_str(&row.get::<_, String>(2)?),
                size_bytes: row.get(3)?,
                modified_at: row.get(4)?,
                content_hash: row.get(5)?,
                remote_updated_at: row.get(6)?,
                parent_id: row.get(7)?,
                item_kind: ItemKind::from_str(&row.get::<_, String>(8)?),
            })
        })?;
        rows.collect()
    }

    /// Return every tracked file. Used by virtual filesystem directory
    /// enumeration to expose known cloud-only and local files.
    pub fn list_files(&self) -> Result<Vec<FileEntry>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT file_id, path, status, size_bytes, modified_at, content_hash, remote_updated_at,
                    parent_id, item_kind
             FROM files ORDER BY path ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(FileEntry {
                file_id: row.get(0)?,
                path: row.get(1)?,
                status: FileStatus::from_str(&row.get::<_, String>(2)?),
                size_bytes: row.get(3)?,
                modified_at: row.get(4)?,
                content_hash: row.get(5)?,
                remote_updated_at: row.get(6)?,
                parent_id: row.get(7)?,
                item_kind: ItemKind::from_str(&row.get::<_, String>(8)?),
            })
        })?;
        rows.collect()
    }

    /// Return every tracked file paired with its **effective-pinned** flag —
    /// the per-PC sync-state lens behind the in-app "Files" tab
    /// (`account_dto::compute_file_overview`).
    ///
    /// This exists alongside [`Self::list_files`] because `FileEntry` does not
    /// carry pin state: pinning lives on the `pin_state` / `inherited_pin_state`
    /// columns owned by [`FileContractState`]. Rather than widen `FileEntry`
    /// (and every caller of it), this method SELECTs those two extra columns and
    /// computes `pinned` with the SAME predicate as
    /// [`FileContractState::effective_pin_state`]: the row's own `pin_state` if
    /// it is set (`pinned` / `unpinned`), otherwise the `inherited_pin_state` —
    /// pinned only when that resolves to [`PinState::Pinned`].
    pub fn file_overview_rows(&self) -> Result<Vec<(FileEntry, bool)>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT file_id, path, status, size_bytes, modified_at, content_hash, remote_updated_at,
                    parent_id, item_kind, pin_state, inherited_pin_state
             FROM files ORDER BY path ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            let entry = FileEntry {
                file_id: row.get(0)?,
                path: row.get(1)?,
                status: FileStatus::from_str(&row.get::<_, String>(2)?),
                size_bytes: row.get(3)?,
                modified_at: row.get(4)?,
                content_hash: row.get(5)?,
                remote_updated_at: row.get(6)?,
                parent_id: row.get(7)?,
                item_kind: ItemKind::from_str(&row.get::<_, String>(8)?),
            };
            let pin_state = PinState::from_str(&row.get::<_, String>(9)?);
            let inherited_pin_state = PinState::from_str(&row.get::<_, String>(10)?);
            // Mirror FileContractState::effective_pin_state: own pin wins unless
            // it's `inherit`, in which case the inherited state decides.
            let effective = match pin_state {
                PinState::Inherit => inherited_pin_state,
                explicit => explicit,
            };
            let pinned = effective == PinState::Pinned;
            Ok((entry, pinned))
        })?;
        rows.collect()
    }

    pub fn set_file_contract_state(&self, state: &FileContractState) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        // Task 1697: the contract write is what moves an item BETWEEN
        // containers (parent change) — record a reparent with the old parent
        // so the materialized filter can test old-or-new. Other contract
        // metadata changes (kind, current_version, content_type) record a
        // plain modified. A no-op write records nothing.
        let old: Option<(Option<String>, String, String, i64)> = conn
            .query_row(
                "SELECT parent_id, item_kind, content_type, current_version
                 FROM files WHERE file_id = ?1",
                params![state.file_id],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                        row.get::<_, i64>(3)?,
                    ))
                },
            )
            .optional()?;
        conn.execute(
            "UPDATE files SET
               namespace = ?2,
               parent_id = ?3,
               shared_root_id = ?4,
               share_id = ?5,
               permission_bits = ?6,
               item_kind = ?7,
               content_type = ?8,
               current_version = ?9,
               current_object_version_id = ?10,
               local_base_version = ?11,
               local_hash = ?12,
               cache_path = ?13,
               cache_bytes = ?14,
               pin_state = ?15,
               inherited_pin_state = ?16,
               last_sync_at = ?17,
               owner_email = ?18
             WHERE file_id = ?1",
            params![
                state.file_id,
                state.namespace.as_str(),
                state.parent_id,
                state.shared_root_id,
                state.share_id,
                state.permission_bits,
                state.item_kind.as_str(),
                state.content_type,
                state.current_version,
                state.current_object_version_id,
                state.local_base_version,
                state.local_hash,
                state.cache_path,
                state.cache_bytes,
                state.pin_state.as_str(),
                state.inherited_pin_state.as_str(),
                state.last_sync_at,
                state.owner_email,
            ],
        )?;
        if let Some((old_parent, old_kind, old_content_type, old_version)) = old {
            let parent_changed = old_parent != state.parent_id;
            let metadata_changed = old_kind != state.item_kind.as_str()
                || old_content_type != state.content_type.clone().unwrap_or_default()
                || old_version != state.current_version;
            if parent_changed {
                record_file_change_conn(&conn, &state.file_id, FpChangeKind::Reparented, old_parent)?;
            } else if metadata_changed {
                record_file_change_conn(&conn, &state.file_id, FpChangeKind::Modified, None)?;
            }
        }
        Ok(())
    }

    pub fn get_file_contract_state(&self, file_id: &str) -> Result<Option<FileContractState>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT file_id, namespace, parent_id, shared_root_id, share_id, permission_bits,
                    item_kind, content_type, current_version, current_object_version_id,
                    local_base_version, local_hash, cache_path, cache_bytes, pin_state,
                    inherited_pin_state, last_sync_at, owner_email
             FROM files WHERE file_id = ?1",
        )?;
        let mut rows = stmt.query(params![file_id])?;
        if let Some(row) = rows.next()? {
            Ok(Some(FileContractState {
                file_id: row.get(0)?,
                namespace: Namespace::from_str(&row.get::<_, String>(1)?),
                parent_id: row.get(2)?,
                shared_root_id: row.get(3)?,
                share_id: row.get(4)?,
                owner_email: row.get(17)?,
                permission_bits: row.get(5)?,
                item_kind: ItemKind::from_str(&row.get::<_, String>(6)?),
                content_type: row.get(7)?,
                current_version: row.get(8)?,
                current_object_version_id: row.get(9)?,
                local_base_version: row.get(10)?,
                local_hash: row.get(11)?,
                cache_path: row.get(12)?,
                cache_bytes: row.get(13)?,
                pin_state: PinState::from_str(&row.get::<_, String>(14)?),
                inherited_pin_state: PinState::from_str(&row.get::<_, String>(15)?),
                last_sync_at: row.get(16)?,
            }))
        } else {
            Ok(None)
        }
    }

    // ── File Provider change log + sync anchor (task 1697) ────────────────────
    //
    // The replica's enumerator (`NSFileProviderReplicatedExtension`) needs a
    // daemon-side change cursor to answer `enumerateChanges(for:from:)` and
    // `currentSyncAnchor`. Every mutating row operation funnels through
    // `record_file_change`, which appends to `fp_changes` and advances the
    // persistent `fp_sync_anchor` cursor. Anchors are the change-log rowid:
    // strictly ascending, compact (decimal ASCII ≤ 500 bytes for any realistic
    // history), stable across a no-op poll, and durable across process death
    // (they live in state.db, next to the log they point into).

    /// Append one change to the log and advance the persistent anchor cursor.
    /// `old_parent_id` is only meaningful for [`FpChangeKind::Reparented`] (and
    /// informational elsewhere); the NEW parent is read from the current
    /// `files` row when it exists (for `Deleted` it was captured by the caller
    /// BEFORE `delete_file` — see `delete_file_for_fp`).
    pub fn record_file_change(&self, file_id: &str, kind: FpChangeKind, old_parent_id: Option<String>) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        record_file_change_conn(&conn, file_id, kind, old_parent_id)
    }

    /// Changes after `since_anchor`, ALL of them, with the anchor to persist
    /// afterwards. The daemon RPC adds paging on top of this (see
    /// [`Self::list_file_changes_paged`]); this all-at-once form is the unit
    /// level the change-log tests pin.
    ///
    /// Returns `(changes, Some(next_anchor))` — or `(vec![], Some(anchor))`
    /// unchanged when the caller is already up to date — and `None` (no rows)
    /// never happens for the default domain: a fresh, empty log reports an
    /// empty anchor. `next_anchor` moves only when changes were delivered.
    pub fn list_file_changes(&self, since_anchor: Option<&[u8]>) -> Result<Option<(Vec<FileChange>, Option<Vec<u8>>)>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let since: i64 = match since_anchor {
            None => 0,
            Some(bytes) => std::str::from_utf8(bytes)
                .ok()
                .and_then(|text| text.parse::<i64>().ok())
                .ok_or_else(|| {
                    rusqlite::Error::InvalidParameterName(
                        "fp sync anchor is not a recognisable cursor (expired)".into(),
                    )
                })?,
        };
        let tip: i64 = conn
            .query_row(
                "SELECT COALESCE((SELECT MAX(seq) FROM fp_changes), 0)",
                [],
                |row| row.get(0),
            )?;
        if tip <= since {
            // Up to date: echo the caller's anchor (or nil at genesis) and
            // deliver nothing. The anchor must not move — a stable anchor on a
            // no-op poll is what tells the system `moreComing: false` is honest.
            return Ok(Some((Vec::new(), since_anchor.map(|bytes| bytes.to_vec()))));
        }
        let mut stmt = conn.prepare(
            "SELECT seq, file_id, kind, old_parent_id, new_parent_id
             FROM fp_changes WHERE seq > ?1 ORDER BY seq ASC",
        )?;
        let changes = stmt
            .query_map(params![since], |row| {
                Ok(FileChange {
                    seq: row.get(0)?,
                    file_id: row.get(1)?,
                    kind: FpChangeKind::from_str(&row.get::<_, String>(2)?)
                        .ok_or_else(|| {
                            rusqlite::Error::InvalidColumnType(
                                2,
                                "kind".to_string(),
                                rusqlite::types::Type::Text,
                            )
                        })?,
                    old_parent_id: row.get(3)?,
                    new_parent_id: row.get(4)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let next_anchor = anchor_bytes(tip);
        Ok(Some((changes, Some(next_anchor))))
    }

    /// Paged variant of [`Self::list_file_changes`]: at most `limit` changes
    /// per call; `next_anchor` is `Some(next_cursor_bytes)` while MORE changes
    /// remain (resume token for the next page) and `None` when the batch
    /// completes the change window. The resume token IS the anchor: paging
    /// consumes the same monotonic cursor, so a client that stops mid-window
    /// resumes exactly where it stopped, and a client that never resumes
    /// cannot lose data (the anchor only advances when it re-polls from nil).
    pub fn list_file_changes_paged(
        &self,
        since_anchor: Option<&[u8]>,
        limit: usize,
    ) -> Result<Option<(Vec<FileChange>, Option<Vec<u8>>)>> {
        let limit = limit.max(1);
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let since: i64 = match since_anchor {
            None => 0,
            Some(bytes) => std::str::from_utf8(bytes)
                .ok()
                .and_then(|text| text.parse::<i64>().ok())
                .ok_or_else(|| {
                    rusqlite::Error::InvalidParameterName(
                        "fp sync anchor is not a recognisable cursor (expired)".into(),
                    )
                })?,
        };
        let mut stmt = conn.prepare(
            "SELECT seq, file_id, kind, old_parent_id, new_parent_id
             FROM fp_changes WHERE seq > ?1 ORDER BY seq ASC LIMIT ?2",
        )?;
        let changes = stmt
            .query_map(params![since, limit as i64], |row| {
                Ok(FileChange {
                    seq: row.get(0)?,
                    file_id: row.get(1)?,
                    kind: FpChangeKind::from_str(&row.get::<_, String>(2)?)
                        .ok_or_else(|| {
                            rusqlite::Error::InvalidColumnType(
                                2,
                                "kind".to_string(),
                                rusqlite::types::Type::Text,
                            )
                        })?,
                    old_parent_id: row.get(3)?,
                    new_parent_id: row.get(4)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        if changes.len() < limit {
            // Short page. It is a COMPLETED batch when it reaches the log tip
            // (or the tip is at/below the caller's cursor) — otherwise the
            // caller still needs a resume token to fetch the rest.
            let tip: i64 = conn.query_row(
                "SELECT COALESCE((SELECT MAX(seq) FROM fp_changes), 0)",
                [],
                |row| row.get(0),
            )?;
            let reached_tip = changes
                .last()
                .map(|change| change.seq >= tip)
                .unwrap_or(tip <= since);
            if reached_tip {
                // Up to date AND the batch ends here: the anchor is the log
                // tip — "you are now current as of tip". Only a completely
                // empty log (no anchor ever minted) reports None.
                let anchor = if tip > 0 { Some(anchor_bytes(tip)) } else { since_anchor.map(|bytes| bytes.to_vec()) };
                return Ok(Some((changes, anchor)));
            }
            let next = changes.last().map(|change| change.seq).unwrap_or(since);
            return Ok(Some((changes, Some(anchor_bytes(next)))));
        }
        // Full page: there may be more — resume from the last delivered seq.
        let next = changes.last().map(|change| change.seq).unwrap_or(since);
        Ok(Some((changes, Some(anchor_bytes(next)))))
    }

    /// Delete consumed change rows recorded before `before_epoch` (the sweep
    /// runs on a delay so a slow replica can still page through recent
    /// history). NEVER touches `fp_sync_anchor`: the replica's cursor position
    /// must survive even after the changes it points past are gone.
    pub fn sweep_file_changes(&self, before_epoch: i64) -> Result<usize> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute("DELETE FROM fp_changes WHERE recorded_at < ?1", params![before_epoch])
    }

    /// The persistent anchor cursor (per domain; one default domain today).
    /// `currentSyncAnchor` is served from here after extension process death.
    pub fn fp_last_anchor(&self) -> Result<Option<Vec<u8>>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let value: Option<i64> = conn
            .query_row(
                "SELECT last_anchor FROM fp_sync_anchor WHERE domain_id = '__default__'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        Ok(value.map(anchor_bytes))
    }

    /// Number of live rows whose contract parent is `folder_id` (task 1697:
    /// the File Provider `childItemCount`). Counts every row regardless of
    /// status — a folder's cloud-only children still show in Finder.
    pub fn child_item_count(&self, folder_id: &str) -> Result<i64> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.query_row(
            "SELECT COUNT(*) FROM files WHERE parent_id = ?1",
            params![folder_id],
            |row| row.get(0),
        )
    }

    /// Record the materialized containers the system reported. Idempotent; the
    /// full set is written each time the extension reports it.
    pub fn set_materialized_containers(&self, container_ids: &[String]) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.unchecked_transaction()?;
        tx.execute("DELETE FROM fp_materialized", [])?;
        for id in container_ids {
            tx.execute(
                "INSERT OR IGNORE INTO fp_materialized (container_id, updated_at) VALUES (?1, 0)",
                params![id],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn materialized_containers(&self) -> Result<Vec<String>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare("SELECT container_id FROM fp_materialized ORDER BY container_id ASC")?;
        let rows = stmt.query_map([], |row| row.get(0))?;
        rows.collect()
    }

    pub fn list_contract_states_by_namespace(&self, namespace: Namespace) -> Result<Vec<FileContractState>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT file_id, namespace, parent_id, shared_root_id, share_id, permission_bits,
                    item_kind, content_type, current_version, current_object_version_id,
                    local_base_version, local_hash, cache_path, cache_bytes, pin_state,
                    inherited_pin_state, last_sync_at, owner_email
             FROM files WHERE namespace = ?1 ORDER BY path ASC",
        )?;
        let rows = stmt.query_map(params![namespace.as_str()], |row| {
            Ok(FileContractState {
                file_id: row.get(0)?,
                namespace: Namespace::from_str(&row.get::<_, String>(1)?),
                parent_id: row.get(2)?,
                shared_root_id: row.get(3)?,
                share_id: row.get(4)?,
                owner_email: row.get(17)?,
                permission_bits: row.get(5)?,
                item_kind: ItemKind::from_str(&row.get::<_, String>(6)?),
                content_type: row.get(7)?,
                current_version: row.get(8)?,
                current_object_version_id: row.get(9)?,
                local_base_version: row.get(10)?,
                local_hash: row.get(11)?,
                cache_path: row.get(12)?,
                cache_bytes: row.get(13)?,
                pin_state: PinState::from_str(&row.get::<_, String>(14)?),
                inherited_pin_state: PinState::from_str(&row.get::<_, String>(15)?),
                last_sync_at: row.get(16)?,
            })
        })?;
        rows.collect()
    }

    pub fn purge_revoked_shared_content(&self, active_shared_root_ids: &[String]) -> Result<Vec<RevokedSharedCache>> {
        let active: HashSet<&str> = active_shared_root_ids.iter().map(String::as_str).collect();
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction()?;
        let candidates = {
            let mut stmt = tx.prepare(
                "SELECT file_id, shared_root_id, cache_path
                 FROM files
                 WHERE namespace = 'shared_with_me'",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })?;
            rows.collect::<Result<Vec<_>>>()?
        };

        let mut revoked = Vec::new();
        for (file_id, shared_root_id, cache_path) in candidates {
            let root_id = shared_root_id.as_deref().unwrap_or(file_id.as_str());
            if active.contains(root_id) {
                continue;
            }
            tx.execute("DELETE FROM operation_queue WHERE file_id = ?1", params![file_id])?;
            tx.execute("DELETE FROM files WHERE file_id = ?1", params![file_id])?;
            revoked.push(RevokedSharedCache { file_id, cache_path });
        }
        drop_orphaned_upload_resumes(&tx)?;
        tx.commit()?;
        Ok(revoked)
    }

    pub fn enqueue_operation(&self, op: &PendingOperation) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute(
            "INSERT INTO operation_queue (
                op_id, kind, file_id, parent_id, target_path, metadata_json, payload_path,
                base_version, base_object_version_id, attempts, max_attempts, next_retry_at,
                last_error, last_error_class, paused_reason, backup_source_key, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, NULL, NULL, ?14, ?15, ?16)
             ON CONFLICT(op_id) DO UPDATE SET
                kind = excluded.kind,
                file_id = excluded.file_id,
                parent_id = excluded.parent_id,
                target_path = excluded.target_path,
                metadata_json = excluded.metadata_json,
                payload_path = excluded.payload_path,
                base_version = excluded.base_version,
                base_object_version_id = excluded.base_object_version_id,
                attempts = excluded.attempts,
                max_attempts = excluded.max_attempts,
                next_retry_at = excluded.next_retry_at,
                last_error = excluded.last_error,
                last_error_class = excluded.last_error_class,
                paused_reason = excluded.paused_reason,
                backup_source_key = excluded.backup_source_key,
                updated_at = excluded.updated_at",
            params![
                op.op_id,
                op.kind.as_str(),
                op.file_id,
                op.parent_id,
                op.target_path,
                op.metadata_json,
                op.payload_path,
                op.base_version,
                op.base_object_version_id,
                op.attempts,
                op.max_attempts,
                op.next_retry_at,
                op.last_error,
                op.backup_source_key,
                op.created_at,
                op.updated_at
            ],
        )?;
        Ok(())
    }

    pub fn list_due_operations(&self, now: i64) -> Result<Vec<PendingOperation>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT op_id, kind, file_id, parent_id, target_path, metadata_json, payload_path,
                    base_version, base_object_version_id, attempts, max_attempts, next_retry_at,
                    last_error, backup_source_key, created_at, updated_at
             FROM operation_queue
             WHERE next_retry_at <= ?1 AND attempts < max_attempts AND paused_reason IS NULL
             ORDER BY created_at ASC",
        )?;
        let rows = stmt.query_map(params![now], |row| {
            Ok(PendingOperation {
                op_id: row.get(0)?,
                kind: OperationKind::from_str(&row.get::<_, String>(1)?),
                file_id: row.get(2)?,
                parent_id: row.get(3)?,
                target_path: row.get(4)?,
                metadata_json: row.get(5)?,
                payload_path: row.get(6)?,
                base_version: row.get(7)?,
                base_object_version_id: row.get(8)?,
                attempts: row.get(9)?,
                max_attempts: row.get(10)?,
                next_retry_at: row.get(11)?,
                last_error: row.get(12)?,
                backup_source_key: row.get(13)?,
                created_at: row.get(14)?,
                updated_at: row.get(15)?,
            })
        })?;
        rows.collect()
    }

    /// Return operations that need user-visible review. This is broader
    /// than the retry worker's "due now" view: terminal failures whose
    /// attempts hit max_attempts must still appear in the conflict/version
    /// center instead of disappearing from the UI.
    pub fn list_review_operations(&self) -> Result<Vec<PendingOperation>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT op_id, kind, file_id, parent_id, target_path, metadata_json, payload_path,
                    base_version, base_object_version_id, attempts, max_attempts, next_retry_at,
                    last_error, backup_source_key, created_at, updated_at
             FROM operation_queue
             WHERE last_error IS NOT NULL
                OR kind IN ('upload_version', 'restore_version')
             ORDER BY updated_at DESC, created_at DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(PendingOperation {
                op_id: row.get(0)?,
                kind: OperationKind::from_str(&row.get::<_, String>(1)?),
                file_id: row.get(2)?,
                parent_id: row.get(3)?,
                target_path: row.get(4)?,
                metadata_json: row.get(5)?,
                payload_path: row.get(6)?,
                base_version: row.get(7)?,
                base_object_version_id: row.get(8)?,
                attempts: row.get(9)?,
                max_attempts: row.get(10)?,
                next_retry_at: row.get(11)?,
                last_error: row.get(12)?,
                backup_source_key: row.get(13)?,
                created_at: row.get(14)?,
                updated_at: row.get(15)?,
            })
        })?;
        rows.collect()
    }

    /// Delete every queued operation tagged with the given backup origin
    /// `source_key` (task 0811). Called when a known-folder backup is disabled:
    /// it stops that folder's pending uploads immediately AND prevents them from
    /// resuming on the next boot (they no longer exist in the durable queue).
    ///
    /// Surgical by design: rows with a `NULL` `backup_source_key` (every normal,
    /// user-initiated op) and rows tagged with a DIFFERENT folder's key are never
    /// touched. Returns the number of rows deleted (for the disable log).
    pub fn purge_backup_source_ops(&self, source_key: &str) -> Result<usize> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction()?;
        let deleted = tx.execute(
            "DELETE FROM operation_queue WHERE backup_source_key = ?1",
            params![source_key],
        )?;
        drop_orphaned_upload_resumes(&tx)?;
        tx.commit()?;
        Ok(deleted)
    }

    #[cfg(any(target_os = "windows", test))]
    pub fn put_upload_finalization(&self, pending: &UploadFinalization) -> Result<()> {
        let mut conn = self.0.lock().unwrap();
        let tx = conn.transaction()?;
        tx.execute("INSERT INTO upload_finalizations(op_id,local_file_id,server_file_id,target_path,payload_path,stamped)
            VALUES(?1,?2,?3,?4,?5,0)", params![pending.op_id,pending.local_file_id,pending.server_file_id,pending.target_path,pending.payload_path])?;
        tx.execute("INSERT INTO staged_payloads(path,completed) VALUES(?1,1)
            ON CONFLICT(path) DO UPDATE SET completed=1", params![pending.payload_path])?;
        // Once the server completed, only local finalization may be retried.
        // Remove the upload before the next cancellable thumbnail await.
        tx.execute("DELETE FROM operation_queue WHERE op_id=?1", params![pending.op_id])?;
        tx.execute("DELETE FROM upload_resume WHERE op_id=?1", params![pending.op_id])?;
        tx.commit()
    }
    #[cfg(any(target_os = "windows", test))]
    pub fn upload_finalizations(&self) -> Result<Vec<UploadFinalization>> {
        let conn = self.0.lock().unwrap();
        let mut stmt = conn.prepare("SELECT op_id,local_file_id,server_file_id,target_path,payload_path,stamped FROM upload_finalizations")?;
        stmt.query_map([], |r| Ok(UploadFinalization { op_id:r.get(0)?,local_file_id:r.get(1)?,server_file_id:r.get(2)?,target_path:r.get(3)?,payload_path:r.get(4)?,stamped:r.get(5)? }))?.collect()
    }
    #[cfg(any(target_os = "windows", test))]
    pub fn mark_upload_finalization_stamped(&self, op_id: &str) -> Result<()> {
        self.0.lock().unwrap().execute("UPDATE upload_finalizations SET stamped=1 WHERE op_id=?1",params![op_id])?;
        Ok(())
    }
    #[cfg(any(target_os = "windows", test))]
    pub fn forget_upload_finalization(&self, op_id: &str) -> Result<()> {
        self.0.lock().unwrap().execute("DELETE FROM upload_finalizations WHERE op_id=?1 AND stamped=1",params![op_id])?;
        Ok(())
    }

    /// Upload payload ownership outlives operation/resume rows and unlink errors.
    pub fn track_staged_payload(&self, path: &str, source: Option<&str>, completed: bool) -> Result<()> {
        self.0.lock().unwrap().execute(
            "INSERT INTO staged_payloads(path,source_path,completed) VALUES (?1,?2,?3)
             ON CONFLICT(path) DO UPDATE SET completed = MAX(completed, excluded.completed)",
            params![path, source, completed],
        )?;
        Ok(())
    }
    pub fn forget_staged_payload(&self, path: &str) -> Result<()> {
        self.0
            .lock()
            .unwrap()
            .execute("DELETE FROM staged_payloads WHERE path = ?1", params![path])?;
        Ok(())
    }
    #[cfg(any(target_os = "windows", test))]
    pub fn staged_payloads_for_signout(&self) -> Result<Vec<(String, Option<String>, bool)>> {
        let conn = self.0.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT path, source_path, completed FROM staged_payloads
             UNION ALL SELECT payload_path, NULL, 0 FROM upload_resume
             WHERE payload_path NOT IN (SELECT path FROM staged_payloads)",
        )?;
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect()
    }

    // Windows account cleanup must not silently discard paused/exhausted writes.
    // Portable so both Windows CI and Linux exercise the exact DB policy.
    #[cfg(any(target_os = "windows", test))]
    pub fn windows_signout_preflight(&self) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let pending: i64 = conn.query_row("SELECT COUNT(*) FROM operation_queue", [], |r| r.get(0))?;
        let dirty: i64 = conn.query_row(
            "SELECT COUNT(*) FROM files WHERE status IN ('uploading', 'conflict', 'error', 'trashing')",
            [],
            |r| r.get(0),
        )?;
        if pending != 0 || dirty != 0 {
            return Err(rusqlite::Error::InvalidQuery);
        }
        Ok(())
    }

    #[cfg(any(target_os = "windows", test))]
    pub fn finish_windows_signout(&self) -> Result<()> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction()?;
        let pending: i64 = tx.query_row("SELECT COUNT(*) FROM operation_queue", [], |r| r.get(0))?;
        if pending != 0 {
            return Err(rusqlite::Error::InvalidQuery);
        }
        let remaining: i64 = tx.query_row("SELECT (SELECT COUNT(*) FROM staged_payloads) + (SELECT COUNT(*) FROM upload_finalizations)", [], |r| r.get(0))?;
        if remaining != 0 { return Err(rusqlite::Error::InvalidQuery); }
        tx.execute_batch("DELETE FROM upload_resume; DELETE FROM files; DELETE FROM sync_state; DELETE FROM local_activity; DELETE FROM transfer_activity; DELETE FROM bandwidth_samples;")?;
        tx.commit()
    }

    /// Task 1538 findings 1+2: unconditional local-state wipe for sign-out /
    /// account switch. `operation_queue` and `files.cache_path` are both
    /// per-device (not per-account) state — see `state_paths::beebeeb_state_dir`,
    /// which resolves from `app_local_data_dir` alone — so leaving either in
    /// place across a sign-out lets a LATER account's engine execute an
    /// earlier account's still-queued upload (finding 1), or leaves an
    /// earlier account's decrypted file content permanently orphaned on disk
    /// once the next account's first sync prunes the row that pointed at it
    /// (finding 2).
    ///
    /// Unlike the other `operation_queue`/cache purges in this file —
    /// `purge_backup_source_ops` (scoped to one backup tag),
    /// `purge_revoked_shared_content` (scoped to revoked share roots),
    /// `evict_unpinned_cache_until_under` / `disposable_unpinned_cache_paths`
    /// (both explicitly skip pinned and non-`local`-status files) — this
    /// clears EVERY row regardless of tag, pause state, pin state, or
    /// status: none of those distinctions mean anything once the account
    /// that created them is gone.
    ///
    /// `files` rows themselves are left in place (only `cache_path`/
    /// `cache_bytes`/`status` are reset) — their metadata isn't secret, and
    /// the next account's first sync naturally supersedes or prunes them.
    /// The actual decrypted bytes are what must never survive a sign-out, so
    /// this returns every `payload_path`/`cache_path` for the caller to
    /// delete from disk.
    pub fn purge_all_local_state(&self) -> Result<LocalStatePurge> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction()?;

        let payload_paths: Vec<String> = {
            let mut stmt = tx.prepare("SELECT payload_path FROM operation_queue WHERE payload_path IS NOT NULL")?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            rows.collect::<Result<Vec<_>>>()?
        };
        let queued_ops_purged = tx.execute("DELETE FROM operation_queue", [])?;
        // Flow 7: the leaving account's persisted upload sessions (session id,
        // server file id, staged path) go with its queue.
        tx.execute("DELETE FROM upload_resume", [])?;
        // Task 1683 slice 2: the leaving account's recent-transfer names go too, so the
        // next account's popover never lists them.
        tx.execute("DELETE FROM transfer_activity", [])?;

        let cache_paths: Vec<String> = {
            let mut stmt = tx.prepare("SELECT cache_path FROM files WHERE cache_path IS NOT NULL")?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            rows.collect::<Result<Vec<_>>>()?
        };

        // Task 1538 Codex P1 (PR #49, state_db.rs review thread): capture
        // every `local`-status row's (file_id, server-relative path) BEFORE
        // the status flip below runs — regardless of pin state (unlike
        // `unpinned_local_files_for_dehydration`, sign-out must sweep pinned
        // files too; the account is leaving the device, so "keep offline"
        // no longer means anything).
        //
        // On Windows, a materialized Cloud Files placeholder holds its
        // plaintext directly in the sync root — `cache_path` is at best a
        // stale, already-deleted `%TEMP%` decrypt path and at worst NULL
        // (see `unpinned_local_files_for_dehydration`'s doc comment), so the
        // `cache_paths` list above can never be the caller's cue to clean up
        // that plaintext. Flipping `status` to `cloud_only` without first
        // dehydrating/removing the real placeholder would also hide the row
        // from `unpinned_local_files_for_dehydration()` forever, orphaning
        // it — so this list MUST be read before that UPDATE runs, in the
        // same transaction, and the caller must act on it before (or
        // instead of) trusting the status flip alone.
        //
        // On macOS/Linux this list is a harmless superset of `cache_paths`
        // (those platforms store hydrated bytes as a separate cache copy,
        // already covered above); the File Provider domain removal
        // `clear_session_impl` also runs on macOS handles cleanup there.
        let local_placeholder_paths: Vec<(String, String)> = {
            let mut stmt = tx.prepare("SELECT file_id, path FROM files WHERE status = 'local'")?;
            let rows = stmt.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?;
            rows.collect::<Result<Vec<_>>>()?
        };

        // Broadened to `OR status = 'local'` (was `cache_path IS NOT NULL`
        // alone) so a Windows `local` row with a NULL `cache_path` — which
        // `local_placeholder_paths` above has already captured for the
        // caller to dehydrate — still loses its stale `local` status here
        // too, instead of surviving the purge unchanged.
        tx.execute(
            "UPDATE files SET cache_path = NULL, cache_bytes = 0, status = 'cloud_only' \
             WHERE cache_path IS NOT NULL OR status = 'local'",
            [],
        )?;

        tx.commit()?;
        Ok(LocalStatePurge {
            queued_ops_purged,
            payload_paths,
            cache_paths,
            local_placeholder_paths,
        })
    }

    pub fn record_operation_attempt(
        &self,
        op_id: &str,
        attempts: i64,
        next_retry_at: i64,
        last_error: Option<&str>,
    ) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute(
            "UPDATE operation_queue
             SET attempts = ?2,
                 next_retry_at = ?3,
                 last_error = ?4,
                 last_error_class = NULL,
                 paused_reason = NULL,
                 updated_at = ?3
             WHERE op_id = ?1",
            params![op_id, attempts, next_retry_at, last_error],
        )?;
        Ok(())
    }

    pub fn record_operation_pause(
        &self,
        op_id: &str,
        reason: OperationPauseReason,
        last_error: Option<&str>,
        now: i64,
    ) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute(
            "UPDATE operation_queue
             SET paused_reason = ?2,
                 last_error_class = ?2,
                 last_error = ?3,
                 updated_at = ?4
             WHERE op_id = ?1",
            params![op_id, reason.as_str(), last_error.map(redact_diagnostic_error), now],
        )?;
        Ok(())
    }

    /// Queue counts and the last error, redacted for the support bundle.
    /// Names known to the state DB are scrubbed from the error text.
    pub fn queue_diagnostics(&self, now: i64) -> Result<QueueDiagnostics> {
        self.queue_diagnostics_with_paths(now, &[])
    }

    /// Like [`Self::queue_diagnostics`], additionally treating every component
    /// of `extra_paths` (for example the sync root) as a name to scrub.
    pub fn queue_diagnostics_with_paths(&self, now: i64, extra_paths: &[String]) -> Result<QueueDiagnostics> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let queued = conn.query_row("SELECT COUNT(*) FROM operation_queue", [], |row| row.get(0))?;
        let due = conn.query_row(
            "SELECT COUNT(*) FROM operation_queue WHERE next_retry_at <= ?1 AND attempts < max_attempts AND paused_reason IS NULL",
            params![now],
            |row| row.get(0),
        )?;
        let paused = conn.query_row(
            "SELECT COUNT(*) FROM operation_queue WHERE paused_reason IS NOT NULL",
            [],
            |row| row.get(0),
        )?;

        let by_kind = allowed_group_labels(count_queue_groups(&conn, "kind")?, QUEUE_KIND_LABELS);
        let paused_by_reason =
            allowed_group_labels(count_queue_groups(&conn, "paused_reason")?, PAUSE_REASON_LABELS);
        let (last_error, last_error_class) = conn
            .query_row(
                "SELECT last_error, last_error_class
                 FROM operation_queue
                 WHERE last_error IS NOT NULL
                 ORDER BY updated_at DESC, created_at DESC
                 LIMIT 1",
                [],
                |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, Option<String>>(1)?)),
            )
            .unwrap_or((None, None));

        let (last_error, last_error_code, last_error_redactions) = match last_error.as_deref() {
            Some(raw) => {
                let names = collect_known_names(&conn, extra_paths)?;
                let redacted = redact_for_export(raw, &names);
                (Some(redacted.text), Some(classify_error_code(raw)), redacted.redactions)
            }
            None => (None, None, 0),
        };

        Ok(QueueDiagnostics {
            queued,
            due,
            paused,
            by_kind,
            paused_by_reason,
            last_error,
            last_error_code,
            last_error_redactions,
            last_error_class: last_error_class
                .as_deref()
                .map(|class| allowed_label(class, PAUSE_REASON_LABELS)),
        })
    }

    /// Test hook: hold the database lock so a test can park every writer behind
    /// it and force two requests to overlap deterministically (task 1684).
    #[cfg(test)]
    pub(crate) fn hold_lock_for_test(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.0.lock().expect("state_db mutex poisoned")
    }

    /// True while a `TrashFile` operation for `file_id` is still in the queue,
    /// whatever its retry state (due, backing off, paused, or out of attempts).
    /// A queued trash means the user deleted the item; the `files` row stays in
    /// place until the server trash converges, so the row alone does not say so
    /// (task 1684 fix round: the write-dedup guard needs this).
    pub fn has_pending_trash(&self, file_id: &str) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let found: Option<i64> = conn
            .query_row(
                "SELECT 1 FROM operation_queue WHERE file_id = ?1 AND kind = 'trash_file' LIMIT 1",
                params![file_id],
                |row| row.get(0),
            )
            .optional()?;
        Ok(found.is_some())
    }

    pub fn remove_operation(&self, op_id: &str) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute("DELETE FROM operation_queue WHERE op_id = ?1", params![op_id])?;
        conn.execute("DELETE FROM upload_resume WHERE op_id = ?1", params![op_id])?;
        Ok(())
    }

    /// Persist (insert or replace) the resumable upload session for `op_id`.
    pub fn put_upload_resume(&self, resume: &UploadResume) -> Result<()> {
        self.track_staged_payload(&resume.payload_path, None, false)?;
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute(
            "INSERT INTO upload_resume (
                op_id, payload_path, payload_size, payload_mtime_ns, upload_session_id,
                server_file_id, object_version_id, chunk_size_bytes, chunk_count,
                acked_chunks, metadata_applied, is_create, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, strftime('%s','now'))
             ON CONFLICT(op_id) DO UPDATE SET
                payload_path = excluded.payload_path,
                payload_size = excluded.payload_size,
                payload_mtime_ns = excluded.payload_mtime_ns,
                upload_session_id = excluded.upload_session_id,
                server_file_id = excluded.server_file_id,
                object_version_id = excluded.object_version_id,
                chunk_size_bytes = excluded.chunk_size_bytes,
                chunk_count = excluded.chunk_count,
                acked_chunks = excluded.acked_chunks,
                metadata_applied = excluded.metadata_applied,
                is_create = excluded.is_create,
                updated_at = excluded.updated_at",
            params![
                resume.op_id,
                resume.payload_path,
                resume.payload_size,
                resume.payload_mtime_ns,
                resume.upload_session_id,
                resume.server_file_id,
                resume.object_version_id,
                resume.chunk_size_bytes,
                resume.chunk_count,
                resume.acked_chunks,
                resume.metadata_applied,
                resume.is_create,
            ],
        )?;
        Ok(())
    }

    pub fn get_upload_resume(&self, op_id: &str) -> Result<Option<UploadResume>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.query_row(
            "SELECT op_id, payload_path, payload_size, payload_mtime_ns, upload_session_id,
                    server_file_id, object_version_id, chunk_size_bytes, chunk_count,
                    acked_chunks, metadata_applied, is_create
             FROM upload_resume WHERE op_id = ?1",
            params![op_id],
            |row| {
                Ok(UploadResume {
                    op_id: row.get(0)?,
                    payload_path: row.get(1)?,
                    payload_size: row.get(2)?,
                    payload_mtime_ns: row.get(3)?,
                    upload_session_id: row.get(4)?,
                    server_file_id: row.get(5)?,
                    object_version_id: row.get(6)?,
                    chunk_size_bytes: row.get(7)?,
                    chunk_count: row.get(8)?,
                    acked_chunks: row.get(9)?,
                    metadata_applied: row.get(10)?,
                    is_create: row.get(11)?,
                })
            },
        )
        .optional()
    }

    /// Advance the acknowledged-chunk watermark. Monotonic: a late write can
    /// never move it backwards.
    pub fn set_upload_resume_acked(&self, op_id: &str, acked_chunks: i64) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute(
            "UPDATE upload_resume
             SET acked_chunks = MAX(acked_chunks, ?2), updated_at = strftime('%s','now')
             WHERE op_id = ?1",
            params![op_id, acked_chunks],
        )?;
        Ok(())
    }

    pub fn set_upload_resume_metadata_applied(&self, op_id: &str) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute(
            "UPDATE upload_resume SET metadata_applied = 1, updated_at = strftime('%s','now') WHERE op_id = ?1",
            params![op_id],
        )?;
        Ok(())
    }

    pub fn clear_upload_resume(&self, op_id: &str) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute("DELETE FROM upload_resume WHERE op_id = ?1", params![op_id])?;
        Ok(())
    }

    pub fn set_recursive_pin(&self, root_file_id: &str, pinned: bool, now: i64) -> Result<Vec<String>> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction()?;

        // The subtree is identified by PATH PREFIX, not `parent_id`: live
        // `files` rows leave `parent_id` empty and encode the hierarchy only in
        // `path` (folder "R", child "R/leaf"). A `parent_id` walk would match
        // only the root and never reach descendants, so a folder pin would never
        // propagate. Look up the root's path R first; descendants are
        // `path = R` (the root itself) OR rows whose path starts with `R || '/'`.
        // The prefix test is `substr(path, 1, length(R) + 1) = R || '/'` — a
        // metacharacter-safe equality (NOT `LIKE`, whose `%`/`_` in R would
        // misbehave). If the root id is unknown, this is a no-op returning [].
        let Some(root_path) = ({
            let mut stmt = tx.prepare("SELECT path FROM files WHERE file_id = ?1")?;
            let mut rows = stmt.query(params![root_file_id])?;
            match rows.next()? {
                Some(row) => Some(row.get::<_, String>(0)?),
                None => None,
            }
        }) else {
            tx.commit()?;
            return Ok(Vec::new());
        };

        let ids = {
            let mut stmt = tx.prepare(
                "
                SELECT file_id FROM files
                WHERE path = ?1 OR substr(path, 1, length(?1) + 1) = ?1 || '/'
                ORDER BY file_id ASC
                ",
            )?;
            let rows = stmt.query_map(params![root_path], |row| row.get::<_, String>(0))?;
            rows.collect::<Result<Vec<_>>>()?
        };

        let state = if pinned { PinState::Pinned } else { PinState::Unpinned };
        let state_str = state.as_str();
        tx.execute(
            "UPDATE files
             SET pin_state = ?2, inherited_pin_state = ?2, last_sync_at = ?3
             WHERE file_id = ?1",
            params![root_file_id, state_str, now],
        )?;
        // Descendants (strict prefix `R || '/'`, excluding the root row itself)
        // get only `inherited_pin_state` — the root keeps its explicit
        // `pin_state` set above.
        tx.execute(
            "UPDATE files
             SET inherited_pin_state = ?2, last_sync_at = ?3
             WHERE substr(path, 1, length(?1) + 1) = ?1 || '/'
               AND file_id != ?4",
            params![root_path, state_str, now, root_file_id],
        )?;
        tx.commit()?;
        Ok(ids)
    }

    /// All cloud-only FILE rows in the subtree rooted at `root_file_id`,
    /// INCLUDING the root itself when the root is a cloud-only file.
    ///
    /// Used by the Windows proactive-hydrate-on-pin path
    /// ([`crate::engine_bridge::EngineBridge::set_recursive_pin`]): after
    /// `CfSetPinState(PINNED)` marks the subtree pinned, Windows does NOT
    /// eagerly download a not-yet-opened placeholder, so we walk the descendants
    /// that are still cloud-only and `CfHydratePlaceholder` each one to make them
    /// genuinely available offline.
    ///
    /// Folders are excluded (`item_kind != 'folder'`): a directory placeholder
    /// has no data stream to hydrate — only its file children do. Zero-byte
    /// files are excluded too (nothing to fetch). The subtree is identified by
    /// PATH PREFIX, not `parent_id`: live `files` rows leave `parent_id` empty
    /// and encode the hierarchy only in `path`, so a `parent_id` walk would
    /// match only the root. We look up the root's path R, then take rows where
    /// `path = R` (the root itself, when it is a cloud-only file) OR
    /// `substr(path, 1, length(R) + 1) = R || '/'` (strict descendants) — a
    /// metacharacter-safe prefix equality (NOT `LIKE`). An unknown root id
    /// returns []. The row mapper mirrors [`Self::list_files`].
    pub fn cloud_only_file_descendants(&self, root_file_id: &str) -> Result<Vec<FileEntry>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let root_path: Option<String> = conn
            .query_row(
                "SELECT path FROM files WHERE file_id = ?1",
                params![root_file_id],
                |row| row.get(0),
            )
            .optional()?;
        let Some(root_path) = root_path else {
            return Ok(Vec::new());
        };
        let mut stmt = conn.prepare(
            "
            SELECT file_id, path, status, size_bytes, modified_at, content_hash, remote_updated_at,
                   parent_id, item_kind
            FROM files
            WHERE (path = ?1 OR substr(path, 1, length(?1) + 1) = ?1 || '/')
              AND status = 'cloud_only'
              AND item_kind != 'folder'
              AND size_bytes > 0
            ORDER BY path ASC
            ",
        )?;
        let rows = stmt.query_map(params![root_path], |row| {
            Ok(FileEntry {
                file_id: row.get(0)?,
                path: row.get(1)?,
                status: FileStatus::from_str(&row.get::<_, String>(2)?),
                size_bytes: row.get(3)?,
                modified_at: row.get(4)?,
                content_hash: row.get(5)?,
                remote_updated_at: row.get(6)?,
                parent_id: row.get(7)?,
                item_kind: ItemKind::from_str(&row.get::<_, String>(8)?),
            })
        })?;
        rows.collect()
    }

    /// Register hydrated content as cached and stamp the row `local` — EXCEPT
    /// statuses that mean something else right now (`uploading`, `conflict`,
    /// `error`, and since the trash ruling `trashing`: a row opened from the
    /// macOS Trash view keeps its marker even after its content lands).
    pub fn mark_cached(&self, file_id: &str, cache_path: &str, cache_bytes: i64, opened_at: i64) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute(
            "UPDATE files
             SET cache_path = ?2,
                 cache_bytes = ?3,
                 last_opened_at = ?4,
                 status = CASE
                    WHEN status IN ('uploading', 'conflict', 'error', 'trashing') THEN status
                    ELSE 'local'
                 END
             WHERE file_id = ?1",
            params![file_id, cache_path, cache_bytes.max(0), opened_at],
        )?;
        Ok(())
    }

    pub fn unpinned_cache_bytes(&self) -> Result<i64> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.query_row(
            "
            SELECT COALESCE(SUM(cache_bytes), 0)
            FROM files
            WHERE cache_bytes > 0
              AND cache_path IS NOT NULL
              AND NOT (pin_state = 'pinned' OR (pin_state = 'inherit' AND inherited_pin_state = 'pinned'))
            ",
            [],
            |row| row.get(0),
        )
    }

    pub fn cache_bytes_by_effective_pin(&self, pinned: bool) -> Result<i64> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let predicate = if pinned {
            "pin_state = 'pinned' OR (pin_state = 'inherit' AND inherited_pin_state = 'pinned')"
        } else {
            "NOT (pin_state = 'pinned' OR (pin_state = 'inherit' AND inherited_pin_state = 'pinned'))"
        };
        conn.query_row(
            &format!(
                "
                SELECT COALESCE(SUM(cache_bytes), 0)
                FROM files
                WHERE cache_bytes > 0
                  AND cache_path IS NOT NULL
                  AND ({predicate})
                "
            ),
            [],
            |row| row.get(0),
        )
    }

    pub fn evict_unpinned_cache_until_under(&self, max_unpinned_cache_bytes: i64, now: i64) -> Result<Vec<String>> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction()?;
        let mut total: i64 = tx.query_row(
            "
            SELECT COALESCE(SUM(cache_bytes), 0)
            FROM files
            WHERE cache_bytes > 0
              AND cache_path IS NOT NULL
              AND NOT (pin_state = 'pinned' OR (pin_state = 'inherit' AND inherited_pin_state = 'pinned'))
            ",
            [],
            |row| row.get(0),
        )?;
        if total <= max_unpinned_cache_bytes.max(0) {
            tx.commit()?;
            return Ok(Vec::new());
        }

        let candidates = {
            let mut stmt = tx.prepare(
                "
                SELECT file_id, cache_bytes
                FROM files
                WHERE cache_bytes > 0
                  AND cache_path IS NOT NULL
                  AND status = 'local'
                  AND NOT (pin_state = 'pinned' OR (pin_state = 'inherit' AND inherited_pin_state = 'pinned'))
                ORDER BY last_opened_at ASC, modified_at ASC, file_id ASC
                ",
            )?;
            let rows = stmt.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)))?;
            rows.collect::<Result<Vec<_>>>()?
        };

        let mut evicted = Vec::new();
        for (file_id, bytes) in candidates {
            if total <= max_unpinned_cache_bytes.max(0) {
                break;
            }
            tx.execute(
                "UPDATE files
                 SET cache_path = NULL,
                     cache_bytes = 0,
                     status = 'cloud_only',
                     modified_at = ?2
                 WHERE file_id = ?1",
                params![file_id, now],
            )?;
            total = total.saturating_sub(bytes);
            evicted.push(file_id);
        }
        tx.commit()?;
        Ok(evicted)
    }

    pub fn disposable_unpinned_cache_paths(&self) -> Result<Vec<String>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare(
            "
            SELECT cache_path
            FROM files
            WHERE cache_bytes > 0
              AND cache_path IS NOT NULL
              AND status = 'local'
              AND NOT (pin_state = 'pinned' OR (pin_state = 'inherit' AND inherited_pin_state = 'pinned'))
            ORDER BY last_opened_at ASC, modified_at ASC, file_id ASC
            ",
        )?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        rows.collect()
    }

    /// Unpinned, locally-present files eligible for Windows Cloud Files
    /// dehydration (task 0781). Returns `(file_id, server_relative_path,
    /// size_bytes)` for every row in `local` status that is NOT effectively
    /// pinned.
    ///
    /// Unlike [`Self::disposable_unpinned_cache_paths`], this does NOT key on
    /// `cache_path`: on Windows CF the hydrated bytes live INSIDE the
    /// placeholder in the sync root, and `cache_path` actually records the
    /// transient `%TEMP%` decrypt path the fetch callback already deleted — so
    /// it can never be the dehydration target. The caller reconstructs the real
    /// on-disk placeholder path from `path` joined onto the sync root (exactly
    /// as `windows_cf::populate_placeholders` does) and dehydrates THAT.
    pub fn unpinned_local_files_for_dehydration(&self) -> Result<Vec<(String, String, i64)>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare(
            "
            SELECT file_id, path, size_bytes
            FROM files
            WHERE status = 'local'
              AND NOT (pin_state = 'pinned' OR (pin_state = 'inherit' AND inherited_pin_state = 'pinned'))
            ORDER BY last_opened_at ASC, modified_at ASC, file_id ASC
            ",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })?;
        rows.collect()
    }

    /// Flip specific files to `cloud_only` after a successful Windows
    /// dehydration (task 0781), keyed by `file_id`. Mirrors
    /// [`Self::clear_cache_metadata_for_paths`] but for the Windows path where
    /// the dehydration target is the placeholder (addressed by `file_id` +
    /// reconstructed path), not the stale `cache_path`. Pinned files are
    /// excluded by the predicate so a caller bug can never dehydrate-then-mark
    /// a pinned file.
    pub fn mark_cloud_only_after_dehydrate(&self, file_ids: &[String], now: i64) -> Result<usize> {
        if file_ids.is_empty() {
            return Ok(0);
        }
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction()?;
        let mut updated = 0usize;
        for file_id in file_ids {
            updated += tx.execute(
                "UPDATE files
                 SET cache_path = NULL,
                     cache_bytes = 0,
                     status = 'cloud_only',
                     modified_at = ?2
                 WHERE file_id = ?1
                   AND status = 'local'
                   AND NOT (pin_state = 'pinned' OR (pin_state = 'inherit' AND inherited_pin_state = 'pinned'))",
                params![file_id, now],
            )?;
        }
        tx.commit()?;
        Ok(updated)
    }

    pub fn clear_cache_metadata_for_paths(&self, cache_paths: &[String], now: i64) -> Result<usize> {
        if cache_paths.is_empty() {
            return Ok(0);
        }

        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction()?;
        let mut updated = 0usize;
        for path in cache_paths {
            updated += tx.execute(
                "UPDATE files
                 SET cache_path = NULL,
                     cache_bytes = 0,
                     status = 'cloud_only',
                     modified_at = ?2
                 WHERE cache_path = ?1
                   AND status = 'local'
                   AND NOT (pin_state = 'pinned' OR (pin_state = 'inherit' AND inherited_pin_state = 'pinned'))",
                params![path, now],
            )?;
        }
        tx.commit()?;
        Ok(updated)
    }

    // ── Bandwidth samples (P3 — 20h traffic chart, task 0810) ────────────────
    //
    // One row per heartbeat beat (~20 s).  The chart reads the last N hours and
    // downsamples to a fixed number of display buckets client-side (in React).
    // Only the last ~25 h are kept on disk; `prune_bandwidth_samples` removes older
    // rows after every write so the table stays bounded.

    /// Insert one bandwidth sample.  `sampled_at` is seconds-since-epoch.
    pub fn insert_bandwidth_sample(
        &self,
        sampled_at: i64,
        up_bytes: u64,
        down_bytes: u64,
        period_secs: u32,
    ) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute(
            "INSERT INTO bandwidth_samples (sampled_at, up_bytes, down_bytes, period_secs)
             VALUES (?1, ?2, ?3, ?4)",
            params![sampled_at, up_bytes as i64, down_bytes as i64, period_secs as i64],
        )?;
        Ok(())
    }

    /// Fetch bandwidth samples with `sampled_at >= since_secs`, oldest first.
    /// Deterministic (no wall-clock read) — used by callers that already have a
    /// cutoff and by tests.
    pub fn get_bandwidth_history_since(&self, since_secs: i64) -> Result<Vec<BandwidthSample>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT sampled_at, up_bytes, down_bytes, period_secs
             FROM bandwidth_samples
             WHERE sampled_at >= ?1
             ORDER BY sampled_at ASC",
        )?;
        let rows = stmt.query_map(params![since_secs], |row| {
            Ok(BandwidthSample {
                sampled_at: row.get(0)?,
                up_bytes: row.get::<_, i64>(1)? as u64,
                down_bytes: row.get::<_, i64>(2)? as u64,
                period_secs: row.get::<_, i64>(3)? as u32,
            })
        })?;
        rows.collect()
    }

    /// Fetch bandwidth samples from the last `hours` hours (oldest first).
    pub fn get_bandwidth_history(&self, hours: u32) -> Result<Vec<BandwidthSample>> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        self.get_bandwidth_history_since(now - hours as i64 * 3600)
    }

    /// Fetch the newest bandwidth sample, if any.
    pub fn latest_bandwidth_sample(&self) -> Result<Option<BandwidthSample>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT sampled_at, up_bytes, down_bytes, period_secs
             FROM bandwidth_samples
             ORDER BY sampled_at DESC
             LIMIT 1",
        )?;
        let mut rows = stmt.query([])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        Ok(Some(BandwidthSample {
            sampled_at: row.get(0)?,
            up_bytes: row.get::<_, i64>(1)? as u64,
            down_bytes: row.get::<_, i64>(2)? as u64,
            period_secs: row.get::<_, i64>(3)? as u32,
        }))
    }

    /// Remove samples older than `cutoff_secs` (seconds-since-epoch) to bound DB size.
    pub fn prune_bandwidth_samples(&self, cutoff_secs: i64) -> Result<usize> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let n = conn.execute(
            "DELETE FROM bandwidth_samples WHERE sampled_at < ?1",
            params![cutoff_secs],
        )?;
        Ok(n)
    }
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

fn ensure_column(conn: &Connection, table: &str, column: &str, definition: &str) -> Result<()> {
    if !has_column(conn, table, column)? {
        conn.execute(&format!("ALTER TABLE {table} ADD COLUMN {column} {definition}"), [])?;
    }
    Ok(())
}

fn has_column(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let names: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .filter_map(|r| r.ok())
        .collect();
    Ok(names.iter().any(|n| n == column))
}

fn count_queue_groups(conn: &Connection, column: &str) -> Result<BTreeMap<String, i64>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {column}, COUNT(*) FROM operation_queue WHERE {column} IS NOT NULL GROUP BY {column}"
    ))?;
    let rows = stmt.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)))?;
    rows.collect()
}

/// Credential-token stripping for error text persisted in the state DB. The
/// privacy boundary for names and paths is the export pass
/// ([`crate::diagnostic_redaction::redact_for_export`]), not this.
fn redact_diagnostic_error(error: &str) -> String {
    redact_secrets_only(error)
}

fn allowed_group_labels(groups: BTreeMap<String, i64>, allowed: &[&str]) -> BTreeMap<String, i64> {
    let mut out = BTreeMap::new();
    for (label, count) in groups {
        *out.entry(allowed_label(&label, allowed)).or_insert(0) += count;
    }
    out
}

/// Every plaintext name the daemon knows locally: synced paths, queued
/// targets and staged payload leaves, recent activity, plus `extra_paths`.
fn collect_known_names(conn: &Connection, extra_paths: &[String]) -> Result<KnownNames> {
    let mut names = KnownNames::new();
    for path in extra_paths {
        names.add_path(path);
    }
    for sql in [
        "SELECT path FROM files",
        "SELECT target_path FROM operation_queue WHERE target_path IS NOT NULL",
        "SELECT payload_path FROM operation_queue WHERE payload_path IS NOT NULL",
        "SELECT file_name FROM local_activity",
        "SELECT rel_path FROM local_activity WHERE rel_path IS NOT NULL",
        // Task 1683 slice 2: the recent-transfer names are known names too, so the
        // support bundle's error text is scrubbed of them (task 1685).
        "SELECT file_name FROM transfer_activity",
        "SELECT rel_path FROM transfer_activity WHERE rel_path IS NOT NULL",
    ] {
        let mut stmt = conn.prepare(sql)?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        for path in rows {
            names.add_path(&path?);
        }
    }
    Ok(names.finish())
}

#[cfg(test)]
fn has_table(conn: &Connection, table: &str) -> Result<bool> {
    let mut stmt = conn.prepare("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1")?;
    let mut rows = stmt.query(params![table])?;
    Ok(rows.next()?.is_some())
}

#[cfg(test)]
mod tests {
    #[test]
    fn round7_finalization_survives_restart_and_blocks_unfinished_signout() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        let db = StateDb::open(&path).unwrap();
        let row = UploadFinalization { op_id: "op".into(), local_file_id:"local".into(), server_file_id:"server".into(), target_path:"file".into(), payload_path:"proof".into(), stamped:false };
        db.put_upload_finalization(&row).unwrap();
        assert_eq!(db.upload_finalizations().unwrap(), vec![row.clone()]);
        db.forget_upload_finalization("op").unwrap();
        assert_eq!(db.upload_finalizations().unwrap().len(), 1, "unstamped proof must not be forgotten");
        db.forget_staged_payload("proof").unwrap();
        assert!(db.finish_windows_signout().is_err(), "finalization owns its proof independently of staging");
        db.mark_upload_finalization_stamped("op").unwrap();
        drop(db);
        let db = StateDb::open(&path).unwrap();
        assert!(db.upload_finalizations().unwrap()[0].stamped);
        db.forget_upload_finalization("op").unwrap();
        db.finish_windows_signout().unwrap();
        assert_eq!(db.upload_finalizations().unwrap().len(), 0);
    }

    #[test]
    fn round5_legacy_resume_inventory_survives_operation_purge() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("state.db");
        let db = StateDb::open(&path).unwrap();
        // Mimic an old database: no staging journal row, no queued owner.
        db.0.lock().unwrap().execute_batch("INSERT INTO upload_resume
            (op_id,payload_path,payload_size,payload_mtime_ns,upload_session_id,server_file_id,object_version_id,chunk_size_bytes,chunk_count)
            VALUES ('old','only-copy',1,0,'session','file','version',1,1)").unwrap();
        drop(db);
        let db = StateDb::open(&path).unwrap();
        db.purge_all_local_state().unwrap();
        assert!(db.get_upload_resume("old").unwrap().is_none());
        assert_eq!(db.staged_payloads_for_signout().unwrap(), vec![("only-copy".into(), None, false)],
            "old payload lost its only durable owner on resume purge");
    }

    #[test]
    fn windows_signout_refuses_paused_pending_bytes_without_mutation() {
        let db = super::StateDb::open(":memory:").unwrap();
        db.0.lock().unwrap().execute_batch(
            "INSERT INTO operation_queue (op_id,kind,payload_path,paused_reason,created_at,updated_at) VALUES ('a','upload_file','only-copy','review',1,1);").unwrap();
        assert!(db.windows_signout_preflight().is_err());
        assert!(db.finish_windows_signout().is_err());
        let remaining: i64 =
            db.0.lock()
                .unwrap()
                .query_row(
                    "SELECT COUNT(*) FROM operation_queue WHERE payload_path='only-copy'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
        assert_eq!(remaining, 1);
    }

    #[test]
    fn windows_signout_clears_all_account_rows_before_relogin_twice() {
        let db = super::StateDb::open(":memory:").unwrap();
        for account in ["a", "b"] {
            db.0.lock()
                .unwrap()
                .execute(
                    "INSERT INTO files(file_id,path,status) VALUES (?1,?1,'cloud_only')",
                    [account],
                )
                .unwrap();
            db.0.lock()
                .unwrap()
                .execute("INSERT INTO sync_state(key,value) VALUES ('cursor','12')", [])
                .unwrap();
            db.windows_signout_preflight().unwrap();
            db.finish_windows_signout().unwrap();
            assert_eq!(db.list_files().unwrap().len(), 0);
            let cursors: i64 =
                db.0.lock()
                    .unwrap()
                    .query_row("SELECT COUNT(*) FROM sync_state", [], |r| r.get(0))
                    .unwrap();
            assert_eq!(cursors, 0);
        }
    }

    #[test]
    fn windows_signout_refuses_conflicted_content_even_without_queue() {
        let db = super::StateDb::open(":memory:").unwrap();
        db.0.lock()
            .unwrap()
            .execute("INSERT INTO files(file_id,path,status) VALUES ('a','a','conflict')", [])
            .unwrap();
        assert!(db.windows_signout_preflight().is_err());
        assert_eq!(db.list_files().unwrap().len(), 1);
    }

    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_upsert_and_get_file() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        let entry = FileEntry {
            file_id: "abc123".to_string(),
            path: "/test/file.txt".to_string(),
            status: FileStatus::CloudOnly,
            size_bytes: 1024,
            modified_at: 1700000000,
            content_hash: None,
            remote_updated_at: 1700000000,
            parent_id: None,
            item_kind: ItemKind::File,
        };
        db.upsert_file(&entry).unwrap();
        let got = db.get_file("abc123").unwrap().unwrap();
        assert_eq!(got.status, FileStatus::CloudOnly);
        assert_eq!(got.size_bytes, 1024);
        assert_eq!(got.remote_updated_at, 1700000000);
    }

    #[test]
    fn local_activity_round_trips_newest_first() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();

        db.record_local_activity(LocalActivityEventInput {
            event_type: LocalActivityKind::MovedToTrash,
            file_id: Some("file-1".into()),
            file_name: "report.pdf".into(),
            rel_path: Some("docs/report.pdf".into()),
            occurred_at: 10,
        })
        .unwrap();
        db.record_local_activity(LocalActivityEventInput {
            event_type: LocalActivityKind::Restored,
            file_id: Some("file-2".into()),
            file_name: "notes.txt".into(),
            rel_path: Some("notes.txt".into()),
            occurred_at: 20,
        })
        .unwrap();

        let events = db.list_recent_local_activity(10).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event_type, LocalActivityKind::Restored);
        assert_eq!(events[0].file_id.as_deref(), Some("file-2"));
        assert_eq!(events[0].file_name, "notes.txt");
        assert_eq!(events[0].rel_path.as_deref(), Some("notes.txt"));
        assert_eq!(events[0].occurred_at, 20);
        assert_eq!(events[1].event_type, LocalActivityKind::MovedToTrash);
        assert_eq!(events[1].file_name, "report.pdf");
    }

    #[test]
    fn local_activity_prunes_to_cap() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();

        for i in 0..(LOCAL_ACTIVITY_MAX_ROWS + 5) {
            db.record_local_activity(LocalActivityEventInput {
                event_type: LocalActivityKind::MovedToTrash,
                file_id: Some(format!("file-{i}")),
                file_name: format!("file-{i}.txt"),
                rel_path: Some(format!("file-{i}.txt")),
                occurred_at: i as i64,
            })
            .unwrap();
        }

        let events = db.list_recent_local_activity(LOCAL_ACTIVITY_MAX_ROWS + 10).unwrap();
        assert_eq!(events.len(), LOCAL_ACTIVITY_MAX_ROWS);
        assert_eq!(events[0].occurred_at, (LOCAL_ACTIVITY_MAX_ROWS + 4) as i64);
        assert_eq!(events.last().unwrap().occurred_at, 5);
    }

    // ── transfer activity + backlog (task 1683 slice 2) ──────────────────────

    fn queued_op(id: &str, kind: OperationKind, file_id: &str) -> PendingOperation {
        PendingOperation {
            op_id: id.into(),
            kind,
            file_id: Some(file_id.into()),
            parent_id: None,
            target_path: None,
            metadata_json: None,
            payload_path: None,
            base_version: None,
            base_object_version_id: None,
            attempts: 0,
            max_attempts: 5,
            next_retry_at: 0,
            last_error: None,
            backup_source_key: None,
            created_at: 1,
            updated_at: 1,
        }
    }

    fn transfer(direction: &'static str, name: &str, at: i64) -> TransferActivityInput {
        TransferActivityInput {
            direction,
            file_id: Some(format!("id-{name}")),
            file_name: name.into(),
            rel_path: Some(format!("Work/{name}")),
            bytes: 10,
            occurred_at: at,
        }
    }

    #[test]
    fn transfer_activity_round_trips_newest_first_with_its_direction() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        db.record_transfer_activity(transfer("up", "a.txt", 10)).unwrap();
        db.record_transfer_activity(transfer("down", "b.txt", 20)).unwrap();
        let rows = db.list_recent_transfer_activity(10).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!((rows[0].file_name.as_str(), rows[0].direction.as_str()), ("b.txt", "down"));
        assert_eq!((rows[1].file_name.as_str(), rows[1].direction.as_str()), ("a.txt", "up"));
        assert_eq!(rows[0].rel_path.as_deref(), Some("Work/b.txt"));
        assert_eq!(db.list_recent_transfer_activity(1).unwrap().len(), 1);
        assert!(db.list_recent_transfer_activity(0).unwrap().is_empty());
    }

    #[test]
    fn transfer_activity_prunes_to_its_own_cap_and_leaves_local_activity_alone() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        db.record_local_activity(LocalActivityEventInput {
            event_type: LocalActivityKind::MovedToTrash,
            file_id: Some("t".into()),
            file_name: "trashed.txt".into(),
            rel_path: None,
            occurred_at: 1,
        })
        .unwrap();
        for i in 0..(TRANSFER_ACTIVITY_MAX_ROWS + 7) {
            db.record_transfer_activity(transfer("up", &format!("f{i}.txt"), i as i64)).unwrap();
        }
        // Count the table itself: the read path also caps its own result, which would hide a
        // write path that forgot to prune.
        let stored: i64 = db
            .0
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM transfer_activity", [], |row| row.get(0))
            .unwrap();
        assert_eq!(stored, TRANSFER_ACTIVITY_MAX_ROWS as i64, "107 written, 100 kept");
        let rows = db.list_recent_transfer_activity(TRANSFER_ACTIVITY_MAX_ROWS + 50).unwrap();
        assert_eq!(rows.len(), TRANSFER_ACTIVITY_MAX_ROWS);
        assert_eq!(rows[0].occurred_at, (TRANSFER_ACTIVITY_MAX_ROWS + 6) as i64);
        assert_eq!(rows.last().unwrap().occurred_at, 7);
        // A large sync must not push the trash event out of its own table.
        assert_eq!(db.list_recent_local_activity(10).unwrap().len(), 1);
    }

    #[test]
    fn the_backlog_counts_due_uploads_downloads_and_quota_pauses() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed_contract_row(&db, "f1", "A/one.bin", None, FileStatus::Uploading, 100);
        seed_contract_row(&db, "f2", "A/two.bin", None, FileStatus::Local, 200);
        seed_contract_row(&db, "f3", "A/three.bin", None, FileStatus::Local, 300);
        seed_contract_row(&db, "f4", "A/four.bin", None, FileStatus::Downloading, 50);
        seed_contract_row(&db, "f5", "A/five.bin", None, FileStatus::Local, 7);
        db.enqueue_operation(&queued_op("u1", OperationKind::UploadFile, "f1")).unwrap();
        db.enqueue_operation(&queued_op("u2", OperationKind::UploadVersion, "f2")).unwrap();
        db.enqueue_operation(&queued_op("u3", OperationKind::UploadFile, "f3")).unwrap();
        db.enqueue_operation(&queued_op("r1", OperationKind::RenameFile, "f5")).unwrap();
        db.record_operation_pause("u3", OperationPauseReason::Quota, Some("quota exceeded"), 5).unwrap();

        let backlog = db.transfer_backlog(100).unwrap();
        assert_eq!(backlog.upload_files, 2, "u1 and u2 are due; u3 is paused; r1 is a rename");
        assert_eq!(backlog.upload_bytes, 300, "100 + 200");
        assert_eq!((backlog.download_files, backlog.download_bytes), (1, 50));
        assert_eq!(backlog.queued_ops, 4, "every queued operation, paused or not");
        assert_eq!(backlog.paused_for_quota, 1);
    }

    #[test]
    fn an_upload_waiting_out_a_retry_backoff_is_not_a_file_left() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed_contract_row(&db, "f1", "one.bin", None, FileStatus::Uploading, 100);
        seed_contract_row(&db, "f2", "two.bin", None, FileStatus::Local, 5_000);
        db.enqueue_operation(&queued_op("u1", OperationKind::UploadFile, "f1")).unwrap();
        // Failed once and backed off for ten minutes (from t=1000 to t=1600).
        let mut backing_off = queued_op("u2", OperationKind::UploadFile, "f2");
        backing_off.attempts = 1;
        backing_off.next_retry_at = 1_600;
        db.enqueue_operation(&backing_off).unwrap();

        let during = db.transfer_backlog(1_000).unwrap();
        assert_eq!(during.upload_files, 1, "only u1 is due; u2 waits for t=1600");
        assert_eq!(during.upload_bytes, 100, "the backed-off file's 5000 bytes are not in the total");
        assert_eq!(during.queued_ops, 2, "it is still a queued operation");

        let at_the_retry_time = db.transfer_backlog(1_600).unwrap();
        assert_eq!(at_the_retry_time.upload_files, 2, "due again exactly at next_retry_at");
        assert_eq!(at_the_retry_time.upload_bytes, 5_100);
    }

    #[test]
    fn an_exhausted_or_auth_paused_upload_is_not_work_left() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed_contract_row(&db, "f1", "one.bin", None, FileStatus::Local, 10);
        seed_contract_row(&db, "f2", "two.bin", None, FileStatus::Local, 20);
        let mut exhausted = queued_op("u1", OperationKind::UploadFile, "f1");
        exhausted.attempts = 5;
        db.enqueue_operation(&exhausted).unwrap();
        db.enqueue_operation(&queued_op("u2", OperationKind::UploadFile, "f2")).unwrap();
        db.record_operation_pause("u2", OperationPauseReason::Auth, None, 5).unwrap();
        let backlog = db.transfer_backlog(100).unwrap();
        assert_eq!(backlog.upload_files, 0);
        assert_eq!(backlog.paused_for_quota, 0);
        assert_eq!(backlog.queued_ops, 2);
    }

    #[test]
    fn diagnostics_scrubs_names_known_only_from_recent_transfers() {
        // Task 1685 guard: a name that lives only in `transfer_activity` is a known
        // name, so the support bundle must not carry it. `2026` is a bare number the
        // allow-list keeps, so only the name scan can remove it.
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        db.record_transfer_activity(TransferActivityInput {
            direction: "up",
            file_id: None,
            file_name: "Board minutes 2026".into(),
            rel_path: None,
            bytes: 1,
            occurred_at: 1,
        })
        .unwrap();
        db.enqueue_operation(&queued_op("op-1", OperationKind::UploadFile, "x")).unwrap();
        db.record_operation_attempt("op-1", 1, 10, Some("copy of Board minutes 2026 failed")).unwrap();
        let exported = serde_json::to_string(&db.queue_diagnostics(200).unwrap()).unwrap();
        assert!(!exported.contains("2026"), "{exported}");
        assert!(!exported.contains("Board"), "{exported}");
        assert!(exported.contains("failed"), "{exported}");
    }

    #[test]
    fn leaving_an_account_clears_its_recent_transfers() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        db.record_transfer_activity(transfer("up", "secret.txt", 1)).unwrap();
        db.purge_all_local_state().unwrap();
        assert!(db.list_recent_transfer_activity(10).unwrap().is_empty());
    }

    #[test]
    fn get_file_by_path_matches_across_leading_slash() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();

        // Row stored WITH a leading slash (the `/leaf` shape resolve_relative_path
        // can produce). The watcher queries the bare form — must still hit.
        db.upsert_file(&FileEntry {
            file_id: "slash-row".into(),
            path: "/docs/a.txt".into(),
            status: FileStatus::CloudOnly,
            size_bytes: 1,
            modified_at: 0,
            content_hash: None,
            remote_updated_at: 0,
            parent_id: None,
            item_kind: ItemKind::File,
        })
        .unwrap();
        assert_eq!(
            db.get_file_by_path("docs/a.txt").unwrap().map(|e| e.file_id),
            Some("slash-row".to_string())
        );
        // Exact form still hits.
        assert_eq!(
            db.get_file_by_path("/docs/a.txt").unwrap().map(|e| e.file_id),
            Some("slash-row".to_string())
        );

        // Row stored WITHOUT a leading slash (the bare relative shape). A query
        // that arrives with a leading slash must still hit.
        db.upsert_file(&FileEntry {
            file_id: "bare-row".into(),
            path: "photo.jpg".into(),
            status: FileStatus::CloudOnly,
            size_bytes: 1,
            modified_at: 0,
            content_hash: None,
            remote_updated_at: 0,
            parent_id: None,
            item_kind: ItemKind::File,
        })
        .unwrap();
        assert_eq!(
            db.get_file_by_path("/photo.jpg").unwrap().map(|e| e.file_id),
            Some("bare-row".to_string())
        );
        assert_eq!(
            db.get_file_by_path("photo.jpg").unwrap().map(|e| e.file_id),
            Some("bare-row".to_string())
        );

        // A genuinely-absent path returns None (and the slash-toggle of an
        // empty string must not panic / false-match).
        assert!(db.get_file_by_path("nope.txt").unwrap().is_none());
        assert!(db.get_file_by_path("").unwrap().is_none());
    }

    #[test]
    fn test_list_by_status() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        let e = FileEntry {
            file_id: "x1".into(),
            path: "/f.txt".into(),
            status: FileStatus::Conflict,
            size_bytes: 0,
            modified_at: 0,
            content_hash: None,
            remote_updated_at: 0,
            parent_id: None,
            item_kind: ItemKind::File,
        };
        db.upsert_file(&e).unwrap();
        let conflicts = db.list_by_status(FileStatus::Conflict).unwrap();
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].file_id, "x1");
    }

    #[test]
    fn decode_os_state_status_only_for_user_owned_states() {
        // Cloud-only on disk → RECALL bit set.
        const RECALL: u32 = 0x0040_0000;
        const PINNED: u32 = 0x0008_0000;
        const UNPINNED: u32 = 0x0010_0000;

        // Local row, RECALL set on disk ⇒ desired CloudOnly; no pin bits ⇒ None.
        let (status, pin) = decode_os_state(RECALL, FileStatus::Local);
        assert_eq!(status, Some(FileStatus::CloudOnly));
        assert_eq!(pin, None);

        // CloudOnly row, RECALL clear (bytes resident) ⇒ desired Local.
        let (status, pin) = decode_os_state(0, FileStatus::CloudOnly);
        assert_eq!(status, Some(FileStatus::Local));
        assert_eq!(pin, None);

        // Engine-owned statuses are NEVER touched, regardless of attrs.
        // `Trashing` is included so a native attribute snapshot can't flip a
        // locally-deleted-but-not-yet-trashed file back to a seedable state.
        for owned in [
            FileStatus::Uploading,
            FileStatus::Downloading,
            FileStatus::Conflict,
            FileStatus::Error,
            FileStatus::Trashing,
        ] {
            let (status, _) = decode_os_state(RECALL, owned.clone());
            assert_eq!(status, None, "{owned:?} must be left alone");
        }

        // Pin bits decode independently of status.
        let (_, pin) = decode_os_state(PINNED, FileStatus::Local);
        assert_eq!(pin, Some(PinState::Pinned));
        let (_, pin) = decode_os_state(UNPINNED, FileStatus::Local);
        assert_eq!(pin, Some(PinState::Unpinned));
        // Both set: PINNED wins (checked first — a placeholder shouldn't carry
        // both, but be deterministic if it does).
        let (_, pin) = decode_os_state(PINNED | UNPINNED, FileStatus::CloudOnly);
        assert_eq!(pin, Some(PinState::Pinned));
        // RECALL set, no pin bit, engine-owned status: status None, pin None.
        let (status, pin) = decode_os_state(RECALL, FileStatus::Uploading);
        assert_eq!(status, None);
        assert_eq!(pin, None);
    }

    #[test]
    fn trashing_status_round_trips_and_is_not_seeded_as_cloud_only() {
        // task 0802: the `Trashing` status (a locally-deleted file awaiting its
        // server-trash) must (a) round-trip through the string codec the DB uses
        // and (b) be EXCLUDED from `list_by_status(CloudOnly)` — the exact query
        // the Windows placeholder seeder (`populate_placeholders`) walks. If a
        // `Trashing` row leaked into that list the placeholder would be re-minted
        // and the deleted file would reappear on disk.
        assert_eq!(FileStatus::Trashing.as_str(), "trashing");
        assert_eq!(FileStatus::from_str("trashing"), FileStatus::Trashing);

        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();

        // One cloud-only row (would be seeded) and one trashing row (must NOT be).
        db.upsert_file(&FileEntry {
            file_id: "cloud-1".into(),
            path: "/keep.txt".into(),
            status: FileStatus::CloudOnly,
            size_bytes: 1,
            modified_at: 0,
            content_hash: None,
            remote_updated_at: 0,
            parent_id: None,
            item_kind: ItemKind::File,
        })
        .unwrap();
        db.upsert_file(&FileEntry {
            file_id: "trash-1".into(),
            path: "/gone.txt".into(),
            status: FileStatus::Trashing,
            size_bytes: 1,
            modified_at: 0,
            content_hash: None,
            remote_updated_at: 0,
            parent_id: None,
            item_kind: ItemKind::File,
        })
        .unwrap();

        // Persisted status survives a reload (string codec round-trip).
        assert_eq!(db.get_file("trash-1").unwrap().unwrap().status, FileStatus::Trashing);

        // The seeder's source list contains ONLY the cloud-only row.
        let cloud_only = db.list_by_status(FileStatus::CloudOnly).unwrap();
        assert_eq!(cloud_only.len(), 1);
        assert_eq!(cloud_only[0].file_id, "cloud-1");
        assert!(
            !cloud_only.iter().any(|e| e.file_id == "trash-1"),
            "a Trashing row must never be returned for CloudOnly seeding"
        );

        // The transition `watcher::handle_delete` performs after enqueuing the
        // trash op: flip a CloudOnly row to Trashing → it drops out of the seed
        // list, so the just-removed placeholder is not re-created.
        db.set_status("cloud-1", FileStatus::Trashing).unwrap();
        assert_eq!(db.get_file("cloud-1").unwrap().unwrap().status, FileStatus::Trashing);
        assert!(
            db.list_by_status(FileStatus::CloudOnly).unwrap().is_empty(),
            "after handle_delete marks the row Trashing, nothing remains to seed"
        );
    }

    #[test]
    fn reconcile_os_state_applies_status_and_pin_deltas() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();

        // Empty input → fast Ok(0), no writes.
        assert_eq!(db.reconcile_os_state(&[], 100).unwrap(), 0);

        let seed = |file_id: &str| FileEntry {
            file_id: file_id.into(),
            path: format!("/{file_id}.txt"),
            status: FileStatus::CloudOnly,
            size_bytes: 0,
            modified_at: 0,
            content_hash: None,
            remote_updated_at: 0,
            parent_id: None,
            item_kind: ItemKind::File,
        };
        // status-only target, pin-only target, both target.
        for id in ["s_only", "p_only", "both"] {
            db.upsert_file(&seed(id)).unwrap();
        }

        let deltas = vec![
            // status-only: flip CloudOnly → Local, re-stamps last_sync_at.
            ("s_only".to_string(), Some(FileStatus::Local), None),
            // pin-only: leave status, set pinned.
            ("p_only".to_string(), None, Some(PinState::Pinned)),
            // both: status + pin.
            (
                "both".to_string(),
                Some(FileStatus::CloudOnly),
                Some(PinState::Unpinned),
            ),
            // missing row: WHERE matches nothing, contributes 0 to the count.
            ("ghost".to_string(), Some(FileStatus::Local), None),
        ];
        // s_only(1) + p_only(1) + both(2) + ghost(0) = 4 rows touched.
        let touched = db.reconcile_os_state(&deltas, 12345).unwrap();
        assert_eq!(touched, 4);

        // status-only applied and last_sync_at re-stamped.
        let s = db.get_file("s_only").unwrap().unwrap();
        assert_eq!(s.status, FileStatus::Local);
        let s_contract = db.get_file_contract_state("s_only").unwrap().unwrap();
        assert_eq!(s_contract.last_sync_at, 12345);

        // pin-only applied, status untouched.
        let p = db.get_file("p_only").unwrap().unwrap();
        assert_eq!(p.status, FileStatus::CloudOnly);
        let p_contract = db.get_file_contract_state("p_only").unwrap().unwrap();
        assert_eq!(p_contract.pin_state, PinState::Pinned);

        // both applied.
        let b_contract = db.get_file_contract_state("both").unwrap().unwrap();
        assert_eq!(b_contract.pin_state, PinState::Unpinned);
        assert_eq!(b_contract.last_sync_at, 12345);
        assert_eq!(db.get_file("both").unwrap().unwrap().status, FileStatus::CloudOnly);
    }

    #[test]
    fn set_size_bytes_corrects_row_to_decrypted_length() {
        // Task 0783 Tier-1: a placeholder seeded with the (wrong) encrypted
        // size must be correctable to the AES-256-GCM-authenticated plaintext
        // length so the next hydration mints an aligned placeholder.
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();

        // file_size = plaintext_len + 28 (one single-chunk GCM nonce+tag) is
        // exactly the bad-row shape from the live trace.
        let plaintext_len: i64 = 20;
        let bad_size: i64 = plaintext_len + 28; // 48
        db.upsert_file(&FileEntry {
            file_id: "size-mismatch".into(),
            path: "/bb-test-upload.txt".into(),
            status: FileStatus::CloudOnly,
            size_bytes: bad_size,
            modified_at: 0,
            content_hash: None,
            remote_updated_at: 0,
            parent_id: None,
            item_kind: ItemKind::File,
        })
        .unwrap();

        // Correcting an existing row rewrites the column and reports 1 row.
        let updated = db.set_size_bytes("size-mismatch", plaintext_len).unwrap();
        assert_eq!(updated, 1);
        assert_eq!(db.get_file("size-mismatch").unwrap().unwrap().size_bytes, plaintext_len);

        // A negative length is clamped to 0 (defensive; never expected).
        db.set_size_bytes("size-mismatch", -5).unwrap();
        assert_eq!(db.get_file("size-mismatch").unwrap().unwrap().size_bytes, 0);

        // An absent file_id is a no-op (0 rows), not an error.
        assert_eq!(db.set_size_bytes("does-not-exist", 10).unwrap(), 0);
    }

    #[test]
    fn test_migrates_drive_contract_columns_and_operation_queue() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("state.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "
                CREATE TABLE files (
                    file_id TEXT PRIMARY KEY,
                    path TEXT NOT NULL,
                    status TEXT NOT NULL DEFAULT 'cloud_only',
                    size_bytes INTEGER NOT NULL DEFAULT 0,
                    modified_at INTEGER NOT NULL DEFAULT 0,
                    content_hash TEXT
                );
                INSERT INTO files (file_id, path, status)
                VALUES ('legacy', '/legacy.txt', 'local');
                ",
            )
            .unwrap();
        }

        let db = StateDb::open(&path).unwrap();
        let conn = db.0.lock().expect("state_db mutex poisoned");
        for column in [
            "remote_updated_at",
            "namespace",
            "parent_id",
            "shared_root_id",
            "share_id",
            "permission_bits",
            "item_kind",
            "content_type",
            "current_version",
            "current_object_version_id",
            "local_base_version",
            "local_hash",
            "cache_path",
            "cache_bytes",
            "pin_state",
            "inherited_pin_state",
            "last_sync_at",
            "last_opened_at",
        ] {
            assert!(has_column(&conn, "files", column).unwrap(), "missing {column}");
        }
        assert!(has_table(&conn, "operation_queue").unwrap());
        for column in [
            "op_id",
            "kind",
            "attempts",
            "next_retry_at",
            "last_error",
            "last_error_class",
            "paused_reason",
        ] {
            assert!(
                has_column(&conn, "operation_queue", column).unwrap(),
                "missing {column}"
            );
        }

        let row: (String, i64, String) = conn
            .query_row(
                "SELECT namespace, permission_bits, pin_state FROM files WHERE file_id = 'legacy'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(row, ("my_files".into(), 0, "inherit".into()));
    }

    #[test]
    fn test_persists_drive_contract_state() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        db.upsert_file(&FileEntry {
            file_id: "file1".into(),
            path: "/Shared/report.pdf".into(),
            status: FileStatus::Local,
            size_bytes: 2048,
            modified_at: 10,
            content_hash: Some("remote-hash".into()),
            remote_updated_at: 11,
            parent_id: None,
            item_kind: ItemKind::File,
        })
        .unwrap();

        let contract = FileContractState {
            file_id: "file1".into(),
            namespace: Namespace::SharedWithMe,
            parent_id: Some("parent1".into()),
            shared_root_id: Some("root1".into()),
            share_id: Some("share1".into()),
            owner_email: None,
            permission_bits: PERMISSION_READ | PERMISSION_WRITE,
            item_kind: ItemKind::Folder,
            content_type: Some("public.folder".into()),
            current_version: 7,
            current_object_version_id: Some("object7".into()),
            local_base_version: 6,
            local_hash: Some("local-hash".into()),
            cache_path: Some("/tmp/beebeeb-cache/file1".into()),
            cache_bytes: 2048,
            pin_state: PinState::Inherit,
            inherited_pin_state: PinState::Pinned,
            last_sync_at: 1234,
        };
        db.set_file_contract_state(&contract).unwrap();

        let got = db.get_file_contract_state("file1").unwrap().unwrap();
        assert_eq!(got.namespace, Namespace::SharedWithMe);
        assert_eq!(got.shared_root_id.as_deref(), Some("root1"));
        assert_eq!(got.share_id.as_deref(), Some("share1"));
        assert_eq!(got.permission_bits, PERMISSION_READ | PERMISSION_WRITE);
        assert_eq!(got.item_kind, ItemKind::Folder);
        assert_eq!(got.content_type.as_deref(), Some("public.folder"));
        assert!(got.can_read());
        assert!(got.can_write());
        assert!(got.is_shared());
        assert_eq!(got.current_version, 7);
        assert_eq!(got.current_object_version_id.as_deref(), Some("object7"));
        assert_eq!(got.local_base_version, 6);
        assert_eq!(got.local_hash.as_deref(), Some("local-hash"));
        assert_eq!(got.cache_path.as_deref(), Some("/tmp/beebeeb-cache/file1"));
        assert_eq!(got.cache_bytes, 2048);
        assert_eq!(got.effective_pin_state(), PinState::Pinned);
    }

    #[test]
    fn test_operation_queue_persists_retry_state() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        let op = PendingOperation {
            op_id: "op-1".into(),
            kind: OperationKind::UploadVersion,
            file_id: Some("file1".into()),
            parent_id: Some("parent1".into()),
            target_path: Some("/report.pdf".into()),
            metadata_json: Some(r#"{"name":"encrypted"}"#.into()),
            payload_path: Some("/tmp/payload".into()),
            base_version: Some(3),
            base_object_version_id: Some("object3".into()),
            attempts: 0,
            max_attempts: 5,
            next_retry_at: 0,
            last_error: None,
            backup_source_key: None,
            created_at: 100,
            updated_at: 100,
        };
        db.enqueue_operation(&op).unwrap();

        db.record_operation_attempt("op-1", 2, 300, Some("timeout")).unwrap();
        let queued = db.list_due_operations(301).unwrap();
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].kind, OperationKind::UploadVersion);
        assert_eq!(queued[0].attempts, 2);
        assert_eq!(queued[0].next_retry_at, 300);
        assert_eq!(queued[0].last_error.as_deref(), Some("timeout"));

        db.remove_operation("op-1").unwrap();
        assert!(db.list_due_operations(999).unwrap().is_empty());
    }

    /// Task 0811: disabling a known-folder backup purges EXACTLY its tagged ops.
    /// A normal (untagged) op and a DIFFERENT folder's tagged op must survive,
    /// and the surviving ops must keep their tag through an enqueue→read round
    /// trip (so a later disable of the other folder still finds them).
    #[test]
    fn test_purge_backup_source_ops_is_surgical() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();

        let mk = |op_id: &str, key: Option<&str>| PendingOperation {
            op_id: op_id.into(),
            kind: OperationKind::UploadVersion,
            file_id: Some(format!("file-{op_id}")),
            parent_id: None,
            target_path: Some(format!("/{op_id}.bin")),
            metadata_json: Some(r#"{"operation":"upload_version"}"#.into()),
            payload_path: Some("/tmp/payload".into()),
            base_version: None,
            base_object_version_id: None,
            attempts: 0,
            max_attempts: 5,
            next_retry_at: 0,
            last_error: None,
            backup_source_key: key.map(str::to_string),
            created_at: 100,
            updated_at: 100,
        };

        // Two ops for `music`, one for `pictures`, one normal (untagged).
        db.enqueue_operation(&mk("music-1", Some("music"))).unwrap();
        db.enqueue_operation(&mk("music-2", Some("music"))).unwrap();
        db.enqueue_operation(&mk("pics-1", Some("pictures"))).unwrap();
        db.enqueue_operation(&mk("normal-1", None)).unwrap();

        // The tag survives the enqueue→read round trip.
        let due = db.list_due_operations(999).unwrap();
        assert_eq!(due.len(), 4);
        let music_2 = due.iter().find(|o| o.op_id == "music-2").unwrap();
        assert_eq!(music_2.backup_source_key.as_deref(), Some("music"));
        let normal = due.iter().find(|o| o.op_id == "normal-1").unwrap();
        assert_eq!(normal.backup_source_key, None);

        // Disabling `music` purges exactly its two ops.
        let purged = db.purge_backup_source_ops("music").unwrap();
        assert_eq!(purged, 2, "both music ops purged");

        let remaining = db.list_due_operations(999).unwrap();
        let ids: std::collections::HashSet<&str> = remaining.iter().map(|o| o.op_id.as_str()).collect();
        assert_eq!(remaining.len(), 2, "only the pictures op + the normal op remain");
        assert!(ids.contains("pics-1"), "other folder's op untouched");
        assert!(ids.contains("normal-1"), "non-backup op untouched");
        assert!(!ids.contains("music-1") && !ids.contains("music-2"), "music ops gone");

        // Purging a key with no rows is a harmless no-op.
        assert_eq!(db.purge_backup_source_ops("videos").unwrap(), 0);
    }

    /// Task 0811: a tagged op survives a DB close/reopen with its tag intact
    /// (the column is durable), so a disable AFTER a restart still purges it —
    /// the whole point of tagging at insertion rather than in memory.
    #[test]
    fn test_backup_source_key_survives_restart() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("state.db");
        {
            let db = StateDb::open(&path).unwrap();
            db.enqueue_operation(&PendingOperation {
                op_id: "music-boot".into(),
                kind: OperationKind::UploadVersion,
                file_id: Some("file-boot".into()),
                parent_id: None,
                target_path: Some("/Backup/Dev/Music/a.mp3".into()),
                metadata_json: Some(r#"{"operation":"upload_version"}"#.into()),
                payload_path: Some("/tmp/payload".into()),
                base_version: None,
                base_object_version_id: None,
                attempts: 0,
                max_attempts: 5,
                next_retry_at: 0,
                last_error: None,
                backup_source_key: Some("music".into()),
                created_at: 100,
                updated_at: 100,
            })
            .unwrap();
        }
        // Reopen (simulates an app restart) and confirm the tag persisted, then
        // a disable purges it.
        let reopened = StateDb::open(&path).unwrap();
        let due = reopened.list_due_operations(999).unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].backup_source_key.as_deref(), Some("music"));
        assert_eq!(reopened.purge_backup_source_ops("music").unwrap(), 1);
        assert!(reopened.list_due_operations(999).unwrap().is_empty());
    }

    #[test]
    fn test_operation_pause_survives_restart_and_diagnostics_redacts() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("state.db");
        let db = StateDb::open(&path).unwrap();
        db.enqueue_operation(&PendingOperation {
            op_id: "op-auth".into(),
            kind: OperationKind::UploadVersion,
            file_id: Some("file1".into()),
            parent_id: None,
            target_path: Some("/secret.txt".into()),
            metadata_json: Some(r#"{"operation":"upload_version","token":"not-diagnostic"}"#.into()),
            payload_path: Some("/tmp/payload".into()),
            base_version: Some(3),
            base_object_version_id: Some("object3".into()),
            attempts: 1,
            max_attempts: 5,
            next_retry_at: 0,
            last_error: None,
            backup_source_key: None,
            created_at: 100,
            updated_at: 100,
        })
        .unwrap();
        db.record_operation_pause(
            "op-auth",
            OperationPauseReason::Auth,
            Some("401 unauthorized Bearer abc.def.ghi session_token=super-secret"),
            200,
        )
        .unwrap();

        drop(db);
        let reopened = StateDb::open(&path).unwrap();
        assert!(reopened.list_due_operations(999).unwrap().is_empty());

        let diagnostics = reopened.queue_diagnostics(999).unwrap();
        assert_eq!(diagnostics.queued, 1);
        assert_eq!(diagnostics.due, 0);
        assert_eq!(diagnostics.paused, 1);
        assert_eq!(diagnostics.paused_by_reason.get("auth"), Some(&1));
        assert_eq!(diagnostics.last_error_class.as_deref(), Some("auth"));
        let last_error = diagnostics.last_error.unwrap();
        assert!(last_error.contains("Bearer [redacted]"));
        assert!(last_error.contains("session_token=[redacted]"));
        assert!(!last_error.contains("abc.def.ghi"));
        assert!(!last_error.contains("super-secret"));
    }

    /// Task 1685 (RED-FIRST): the support bundle promised "without secrets or
    /// plaintext names", but only tokens were redacted. A real upload failure
    /// carries the local path and the (possibly multi-word) folder/file names.
    #[test]
    fn diagnostics_export_never_contains_paths_or_known_names() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed_contract_row(&db, "f-tax", "Tax 2025/aangifte.pdf", None, FileStatus::Local, 10);
        db.enqueue_operation(&PendingOperation {
            op_id: "op-1".into(),
            kind: OperationKind::UploadVersion,
            file_id: Some("f-tax".into()),
            parent_id: None,
            target_path: Some("Tax 2025/aangifte.pdf".into()),
            metadata_json: None,
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
        db.record_operation_attempt(
            "op-1",
            1,
            130,
            Some("/Users/guus/Library/CloudStorage/Beebeeb-Drive/Tax 2025/aangifte.pdf failed"),
        )
        .unwrap();

        let diagnostics = db.queue_diagnostics(200).unwrap();
        let exported = serde_json::to_string(&diagnostics).unwrap();
        assert!(!exported.contains("/Users/guus"), "path leaked: {exported}");
        assert!(!exported.contains("CloudStorage"), "path leaked: {exported}");
        assert!(!exported.contains("Tax 2025"), "known folder name leaked: {exported}");
        assert!(!exported.contains("aangifte"), "known file name leaked: {exported}");
        // The failure itself is still reported, and the removal is counted.
        assert!(diagnostics.last_error.as_deref().unwrap().contains("failed"), "{exported}");
        assert!(diagnostics.last_error_redactions >= 1, "{exported}");
        assert_eq!(diagnostics.last_error_code, Some(DiagnosticErrorCode::Other), "{exported}");
    }

    /// Task 1685: a name that exists only in a queued op's staged payload path
    /// (not in `files`) is still known to the daemon and must be scrubbed. `2026`
    /// is a bare number the allow-list keeps, so only the name scan can remove it.
    #[test]
    fn diagnostics_scrubs_names_known_only_from_a_staged_payload_path() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        db.enqueue_operation(&PendingOperation {
            op_id: "op-1".into(),
            kind: OperationKind::UploadFile,
            file_id: None,
            parent_id: None,
            target_path: None,
            metadata_json: None,
            payload_path: Some("/staging/ab12/Q3 2026".into()),
            base_version: None,
            base_object_version_id: None,
            attempts: 0,
            max_attempts: 5,
            next_retry_at: 0,
            last_error: None,
            backup_source_key: None,
            created_at: 1,
            updated_at: 1,
        })
        .unwrap();
        db.record_operation_attempt("op-1", 1, 10, Some("copy of Q3 2026 failed")).unwrap();
        let exported = serde_json::to_string(&db.queue_diagnostics(200).unwrap()).unwrap();
        assert!(!exported.contains("2026"), "{exported}");
        assert!(!exported.contains("Q3"), "{exported}");
        assert!(exported.contains("failed"), "{exported}");
    }

    /// Task 1685: group labels are an allow-list, so a stray DB value (a name in
    /// `kind` or `paused_reason`) can never become a key in the export.
    #[test]
    fn diagnostics_group_labels_outside_the_allow_list_become_other() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        for id in ["op-a", "op-b"] {
            db.enqueue_operation(&PendingOperation {
                op_id: id.into(),
                kind: OperationKind::UploadVersion,
                file_id: None,
                parent_id: None,
                target_path: None,
                metadata_json: None,
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
        }
        {
            let conn = db.0.lock().unwrap();
            conn.execute("UPDATE operation_queue SET kind = 'Tax 2025' WHERE op_id = 'op-a'", [])
                .unwrap();
            conn.execute(
                "UPDATE operation_queue SET paused_reason = '/Users/guus/Tax 2025', last_error_class = 'aangifte.pdf' WHERE op_id = 'op-b'",
                [],
            )
            .unwrap();
        }
        let diagnostics = db.queue_diagnostics(200).unwrap();
        let exported = serde_json::to_string(&diagnostics).unwrap();
        assert!(!exported.contains("Tax"), "{exported}");
        assert!(!exported.contains("guus"), "{exported}");
        assert!(!exported.contains("aangifte"), "{exported}");
        assert_eq!(diagnostics.by_kind.get("other"), Some(&1), "{exported}");
        assert_eq!(diagnostics.by_kind.get("upload_version"), Some(&1), "{exported}");
        assert_eq!(diagnostics.paused_by_reason.get("other"), Some(&1), "{exported}");
    }

    #[test]
    fn test_revoked_shared_content_is_removed_and_cache_paths_returned() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed_contract_row(&db, "active", "/Shared with me/Active", None, FileStatus::Local, 10);
        seed_contract_row(&db, "revoked", "/Shared with me/Revoked", None, FileStatus::Local, 20);

        let mut active = db.get_file_contract_state("active").unwrap().unwrap();
        active.namespace = Namespace::SharedWithMe;
        active.shared_root_id = Some("active".into());
        active.share_id = Some("invite-active".into());
        active.permission_bits = PERMISSION_READ;
        active.cache_path = Some("/cache/active".into());
        db.set_file_contract_state(&active).unwrap();

        let mut revoked = db.get_file_contract_state("revoked").unwrap().unwrap();
        revoked.namespace = Namespace::SharedWithMe;
        revoked.shared_root_id = Some("revoked".into());
        revoked.share_id = Some("invite-revoked".into());
        revoked.permission_bits = PERMISSION_READ;
        revoked.cache_path = Some("/cache/revoked".into());
        db.set_file_contract_state(&revoked).unwrap();

        let removed = db.purge_revoked_shared_content(&["active".to_string()]).unwrap();

        assert_eq!(
            removed,
            vec![RevokedSharedCache {
                file_id: "revoked".into(),
                cache_path: Some("/cache/revoked".into()),
            }]
        );
        assert!(db.get_file("active").unwrap().is_some());
        assert!(db.get_file("revoked").unwrap().is_none());
    }

    #[test]
    fn test_recursive_pin_inheritance_persists_for_folder_tree() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed_contract_row(&db, "folder-a", "/Projects", None, FileStatus::CloudOnly, 0);
        seed_contract_row(
            &db,
            "file-a",
            "/Projects/report.txt",
            Some("folder-a"),
            FileStatus::CloudOnly,
            100,
        );
        seed_contract_row(
            &db,
            "nested-folder",
            "/Projects/Nested",
            Some("folder-a"),
            FileStatus::CloudOnly,
            0,
        );
        seed_contract_row(
            &db,
            "file-b",
            "/Projects/Nested/spec.md",
            Some("nested-folder"),
            FileStatus::CloudOnly,
            200,
        );

        let changed = db.set_recursive_pin("folder-a", true, 1000).unwrap();
        assert_eq!(changed.len(), 4);
        assert_eq!(
            db.get_file_contract_state("folder-a").unwrap().unwrap().pin_state,
            PinState::Pinned
        );
        assert_eq!(
            db.get_file_contract_state("file-b")
                .unwrap()
                .unwrap()
                .effective_pin_state(),
            PinState::Pinned
        );

        db.set_recursive_pin("folder-a", false, 2000).unwrap();
        assert_eq!(
            db.get_file_contract_state("file-b")
                .unwrap()
                .unwrap()
                .effective_pin_state(),
            PinState::Unpinned
        );
    }

    #[test]
    fn test_cache_eviction_preserves_pinned_and_uploading_files() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed_contract_row(&db, "pinned", "/Pinned.txt", None, FileStatus::Local, 800);
        seed_contract_row(&db, "uploading", "/Uploading.txt", None, FileStatus::Uploading, 900);
        seed_contract_row(&db, "old", "/Old.txt", None, FileStatus::Local, 700);
        seed_contract_row(&db, "new", "/New.txt", None, FileStatus::Local, 600);

        db.set_recursive_pin("pinned", true, 10).unwrap();
        db.mark_cached("pinned", "/cache/pinned", 800, 10).unwrap();
        db.mark_cached("uploading", "/cache/uploading", 900, 20).unwrap();
        db.mark_cached("old", "/cache/old", 700, 30).unwrap();
        db.mark_cached("new", "/cache/new", 600, 40).unwrap();

        let evicted = db.evict_unpinned_cache_until_under(1_000, 50).unwrap();
        assert_eq!(evicted, vec!["old".to_string(), "new".to_string()]);
        assert_eq!(db.get_file("old").unwrap().unwrap().status, FileStatus::CloudOnly);
        assert_eq!(db.get_file("new").unwrap().unwrap().status, FileStatus::CloudOnly);
        assert_eq!(db.get_file("pinned").unwrap().unwrap().status, FileStatus::Local);
        assert_eq!(db.get_file("uploading").unwrap().unwrap().status, FileStatus::Uploading);
    }

    #[test]
    fn test_disposable_cache_cleanup_candidates_skip_pinned_and_uploading_files() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed_contract_row(&db, "pinned", "/Pinned.txt", None, FileStatus::Local, 800);
        seed_contract_row(&db, "uploading", "/Uploading.txt", None, FileStatus::Uploading, 900);
        seed_contract_row(&db, "local", "/Local.txt", None, FileStatus::Local, 700);

        db.set_recursive_pin("pinned", true, 10).unwrap();
        db.mark_cached("pinned", "/cache/pinned", 800, 10).unwrap();
        db.mark_cached("uploading", "/cache/uploading", 900, 20).unwrap();
        db.mark_cached("local", "/cache/local", 700, 30).unwrap();

        assert_eq!(
            db.disposable_unpinned_cache_paths().unwrap(),
            vec!["/cache/local".to_string()]
        );

        let cleared = db
            .clear_cache_metadata_for_paths(&["/cache/local".to_string()], 100)
            .unwrap();
        assert_eq!(cleared, 1);
        assert_eq!(db.get_file("local").unwrap().unwrap().status, FileStatus::CloudOnly);
        assert_eq!(db.get_file_contract_state("local").unwrap().unwrap().cache_path, None);
        assert_eq!(
            db.get_file_contract_state("pinned")
                .unwrap()
                .unwrap()
                .cache_path
                .as_deref(),
            Some("/cache/pinned")
        );
        assert_eq!(
            db.get_file_contract_state("uploading")
                .unwrap()
                .unwrap()
                .cache_path
                .as_deref(),
            Some("/cache/uploading")
        );
    }

    /// Task 1538 finding 1 + 2: sign-out must purge EVERY queued operation
    /// (regardless of retry/paused state) and EVERY cached plaintext path
    /// (regardless of pin state), unlike `purge_backup_source_ops` (scoped to
    /// one backup tag) or `evict_unpinned_cache_until_under`/
    /// `disposable_unpinned_cache_paths` (both explicitly skip pinned and
    /// non-`local`-status files) — a pin set by the previous account must not
    /// protect that account's plaintext from being purged on sign-out.
    #[test]
    fn test_purge_all_local_state_clears_every_queued_op_and_cached_path() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();

        // A pinned file and an uploading file survive normal cache eviction —
        // sign-out must clear their cache_path anyway.
        seed_contract_row(&db, "pinned", "/Pinned.txt", None, FileStatus::Local, 800);
        seed_contract_row(&db, "uploading", "/Uploading.txt", None, FileStatus::Uploading, 900);
        db.set_recursive_pin("pinned", true, 10).unwrap();
        db.mark_cached("pinned", "/cache/pinned", 800, 10).unwrap();
        db.mark_cached("uploading", "/cache/uploading", 900, 20).unwrap();

        // A paused op (would never show up in `list_due_operations`) still
        // must be purged — an account switch must not leave it to resume
        // silently if a later `resume`/retry ever clears the pause.
        let queued_op = PendingOperation {
            op_id: "op-1".into(),
            kind: OperationKind::UploadVersion,
            file_id: Some("pinned".into()),
            parent_id: None,
            target_path: Some("/Pinned.txt".into()),
            metadata_json: None,
            payload_path: Some("/staging/op-1-payload".into()),
            base_version: None,
            base_object_version_id: None,
            attempts: 0,
            max_attempts: 5,
            next_retry_at: 0,
            last_error: None,
            backup_source_key: None,
            created_at: 100,
            updated_at: 100,
        };
        db.enqueue_operation(&queued_op).unwrap();
        db.record_operation_pause("op-1", OperationPauseReason::Auth, Some("offline"), 100)
            .unwrap();

        let purge = db.purge_all_local_state().unwrap();

        assert_eq!(purge.queued_ops_purged, 1);
        assert_eq!(purge.payload_paths, vec!["/staging/op-1-payload".to_string()]);
        let mut cache_paths = purge.cache_paths.clone();
        cache_paths.sort();
        assert_eq!(
            cache_paths,
            vec!["/cache/pinned".to_string(), "/cache/uploading".to_string()]
        );
        // Only the `local`-status row ("pinned") is a placeholder-dehydration
        // candidate — "uploading" is mid-transfer, not a materialized local
        // copy, so it must not appear here.
        assert_eq!(
            purge.local_placeholder_paths,
            vec![("pinned".to_string(), "/Pinned.txt".to_string())]
        );

        // Nothing is left behind to be found by a later account's engine.
        assert!(db.list_due_operations(i64::MAX).unwrap().is_empty());
        assert!(db.list_review_operations().unwrap().is_empty());
        assert_eq!(db.get_file("pinned").unwrap().unwrap().status, FileStatus::CloudOnly);
        assert_eq!(db.get_file_contract_state("pinned").unwrap().unwrap().cache_path, None);
        assert_eq!(
            db.get_file_contract_state("uploading").unwrap().unwrap().cache_path,
            None
        );
    }

    /// Task 1538 Codex P1 (PR #49, state_db.rs:1785 thread): a Windows Cloud
    /// Files placeholder stores its plaintext directly in the sync root, not
    /// at `cache_path` — a materialized `local` row can have `cache_path =
    /// NULL` the whole time (see `unpinned_local_files_for_dehydration`'s doc
    /// comment). The OLD `cache_path IS NOT NULL`-only purge silently skips
    /// such a row entirely: it stays `local` forever, and its on-disk
    /// plaintext (the placeholder) is never found by ANY later cleanup pass,
    /// because `unpinned_local_files_for_dehydration()` also filters on
    /// `status = 'local'` — a status this purge would have left unchanged.
    #[test]
    fn test_purge_all_local_state_captures_windows_style_local_row_with_no_cache_path() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();

        // A materialized Windows placeholder: `local` status, PINNED (a
        // sign-out purge must sweep it anyway — pins don't survive the
        // account that set them), and — unlike every other seeded row in
        // this file — `cache_path` is never populated via `mark_cached`.
        // This is the exact shape a real Windows Cloud Files placeholder
        // leaves in the DB (the fetch callback's `%TEMP%` decrypt copy is
        // already deleted by the time the placeholder is materialized).
        seed_contract_row(&db, "win-local", "/Docs/report.docx", None, FileStatus::Local, 4096);
        db.set_recursive_pin("win-local", true, 10).unwrap();
        assert_eq!(
            db.get_file_contract_state("win-local").unwrap().unwrap().cache_path,
            None,
            "precondition: this row must never have a cache_path, like a real Windows placeholder"
        );

        let purge = db.purge_all_local_state().unwrap();

        // The row has no cache_path, so it can never appear in `cache_paths`
        // — proving the assertions below exercise a genuinely different code
        // path, not a duplicate of the existing cache_path-based one.
        assert!(purge.cache_paths.is_empty());
        assert_eq!(
            purge.local_placeholder_paths,
            vec![("win-local".to_string(), "/Docs/report.docx".to_string())],
            "a `local` row with no cache_path must still surface as a purge candidate \
             for the caller to dehydrate/remove"
        );
        assert_eq!(
            db.get_file("win-local").unwrap().unwrap().status,
            FileStatus::CloudOnly,
            "the row must not be left at `local` status after a sign-out purge — a \
             lingering `local` row hides real on-disk plaintext from every later cleanup pass"
        );
    }

    fn seed_contract_row(
        db: &StateDb,
        file_id: &str,
        path: &str,
        parent_id: Option<&str>,
        status: FileStatus,
        cache_bytes: i64,
    ) {
        db.upsert_file(&FileEntry {
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
        let mut contract = db.get_file_contract_state(file_id).unwrap().unwrap();
        contract.parent_id = parent_id.map(str::to_string);
        contract.cache_bytes = cache_bytes;
        db.set_file_contract_state(&contract).unwrap();
    }

    // ── prune_absent direct unit coverage (issues 4/5/6) ──────────────────────

    /// Insert a settled own-tree row with an explicit `remote_updated_at`.
    fn seed_own_row(db: &StateDb, file_id: &str, status: FileStatus, remote_updated_at: i64) {
        db.upsert_file(&FileEntry {
            file_id: file_id.into(),
            path: format!("{file_id}.txt"),
            status,
            size_bytes: 1,
            modified_at: 0,
            content_hash: None,
            remote_updated_at,
            parent_id: None,
            item_kind: ItemKind::File,
        })
        .unwrap();
    }

    const FAR_FUTURE: i64 = 4_000_000_000; // > any realistic remote_updated_at

    #[test]
    fn prune_absent_deletes_settled_cloud_only_row_absent_from_snapshot() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed_own_row(&db, "stale", FileStatus::CloudOnly, 10);

        let seen: HashSet<String> = ["kept".to_string()].into_iter().collect();
        let pruned = db.prune_absent(&seen, FAR_FUTURE).unwrap();

        assert_eq!(
            pruned.iter().map(|r| r.file_id.clone()).collect::<Vec<_>>(),
            vec!["stale".to_string()]
        );
        assert!(db.get_file("stale").unwrap().is_none());
    }

    #[test]
    fn prune_absent_keeps_row_with_pending_operation() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        // A locally-created-but-not-yet-uploaded file under its client UUID.
        seed_own_row(&db, "in-flight", FileStatus::CloudOnly, 10);
        db.enqueue_operation(&PendingOperation {
            op_id: "op-1".into(),
            kind: OperationKind::UploadVersion,
            file_id: Some("in-flight".into()),
            parent_id: None,
            target_path: Some("/in-flight.txt".into()),
            metadata_json: None,
            payload_path: None,
            base_version: None,
            base_object_version_id: None,
            attempts: 0,
            max_attempts: 5,
            next_retry_at: 0,
            last_error: None,
            backup_source_key: None,
            created_at: 0,
            updated_at: 0,
        })
        .unwrap();

        // Empty snapshot can't see the client UUID; the operation_queue join must
        // keep it (and the empty-snapshot guard also protects it — assert it
        // survives regardless).
        let pruned = db.prune_absent(&HashSet::new(), FAR_FUTURE).unwrap();
        assert!(pruned.is_empty());
        assert!(
            db.get_file("in-flight").unwrap().is_some(),
            "pending-op row never pruned"
        );
    }

    #[test]
    fn prune_absent_keeps_trashing_row_with_pending_trash_op_present_in_snapshot() {
        // task 0802: a locally-deleted file is `Trashing` with its TrashFile op
        // still IN FLIGHT (not yet succeeded), and is STILL PRESENT in the
        // snapshot. prune must not remove it — the `operation_queue` join protects
        // a row with a pending op — so the trash op is left to complete. (Once it
        // succeeds the op is dropped and the row stays `Trashing`; convergence then
        // happens via snapshot-absence — see the `..._absent_from_snapshot` test.)
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed_own_row(&db, "trashing", FileStatus::Trashing, 10);
        db.enqueue_operation(&PendingOperation {
            op_id: "op-trash".into(),
            kind: OperationKind::TrashFile,
            file_id: Some("trashing".into()),
            parent_id: None,
            target_path: None,
            metadata_json: None,
            payload_path: None,
            base_version: None,
            base_object_version_id: None,
            attempts: 0,
            max_attempts: 25,
            next_retry_at: 0,
            last_error: None,
            backup_source_key: None,
            created_at: 0,
            updated_at: 0,
        })
        .unwrap();

        // Snapshot still lists the file (server-side trash not yet applied).
        let seen: HashSet<String> = ["trashing".to_string()].into_iter().collect();
        let pruned = db.prune_absent(&seen, FAR_FUTURE).unwrap();
        assert!(pruned.is_empty());
        assert!(
            db.get_file("trashing").unwrap().is_some(),
            "a Trashing row with a pending trash op must survive prune"
        );
    }

    #[test]
    fn prune_absent_keeps_trashing_row_absent_from_snapshot_trash_view() {
        // task 0802 — FLIPPED BY TASK 1698 (trash ruling, RED-first): after
        // `api.trash_file` succeeds the TrashFile op is REMOVED (the
        // `Trashing` status, not the op, is the durable marker). The file is
        // ABSENT from `/sync/snapshot` — and under the 1698 ruling it stays
        // ABSENT forever (the snapshot never lists trashed files), but the
        // row IS the macOS Trash view now. prune_absent must NOT delete it:
        // convergence to removal flows through the server's `file_delete` op
        // (permanent delete), `file_restore`, or the retention janitor — the
        // same server trash model, no second one invented.
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        // Trashing row, NO pending op (op was dropped on a successful trash).
        seed_own_row(&db, "trashed-gone", FileStatus::Trashing, 10);

        // Snapshot no longer lists the file (server trash has propagated).
        let seen: HashSet<String> = ["still-here".to_string()].into_iter().collect();
        let pruned = db.prune_absent(&seen, FAR_FUTURE).unwrap();

        assert!(
            pruned.is_empty(),
            "a Trashing row absent from the snapshot must NOT be pruned (it is the trash view): {pruned:?}"
        );
        assert!(
            db.get_file("trashed-gone").unwrap().is_some(),
            "a Trashing row absent from the snapshot is the TRASH VIEW (task 1698): the row survives. \
             Convergence to removal now flows through the server's file_delete op (permanent delete), \
             the file_restore op, or the retention janitor — NOT prune_absent."
        );
    }

    #[test]
    fn prune_absent_keeps_children_of_trashing_rows_trash_view_content() {
        // Task 1698: a child inside a trashed folder is the trash view's
        // folder content — the server cascade_trash trashed it too, so the
        // row must survive a snapshot that no longer lists either of them.
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        db.upsert_file(&FileEntry {
            file_id: "trashed-dir".into(),
            path: "trashed-dir".into(),
            status: FileStatus::Trashing,
            size_bytes: 0,
            modified_at: 0,
            content_hash: None,
            remote_updated_at: 10,
            parent_id: None,
            item_kind: ItemKind::Folder,
        })
        .unwrap();
        db.upsert_file(&FileEntry {
            file_id: "trashed-child".into(),
            path: "trashed-dir/leaf.txt".into(),
            status: FileStatus::CloudOnly,
            size_bytes: 1,
            modified_at: 0,
            content_hash: None,
            remote_updated_at: 10,
            parent_id: Some("trashed-dir".into()),
            item_kind: ItemKind::File,
        })
        .unwrap();
        // `upsert_file` does NOT write the parent linkage — the contract does.
        let mut contract = db.get_file_contract_state("trashed-child").unwrap().unwrap();
        contract.parent_id = Some("trashed-dir".into());
        db.set_file_contract_state(&contract).unwrap();

        let seen: HashSet<String> = ["still-here".to_string()].into_iter().collect();
        let pruned = db.prune_absent(&seen, FAR_FUTURE).unwrap();
        assert!(
            pruned.is_empty(),
            "trash-view rows (parent + content) must survive the prune: {pruned:?}"
        );
        assert!(db.get_file("trashed-dir").unwrap().is_some());
        assert!(db.get_file("trashed-child").unwrap().is_some());
    }

    #[test]
    fn prune_absent_keeps_uploading_row() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed_own_row(&db, "uploading-row", FileStatus::Uploading, 10);

        let seen: HashSet<String> = ["other".to_string()].into_iter().collect();
        let pruned = db.prune_absent(&seen, FAR_FUTURE).unwrap();
        assert!(pruned.is_empty());
        assert!(
            db.get_file("uploading-row").unwrap().is_some(),
            "uploading row never pruned"
        );
    }

    #[test]
    fn reconcile_stale_in_flight_on_startup_resets_transient_rows() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed_own_row(&db, "stale-upload", FileStatus::Uploading, 10);
        seed_own_row(&db, "stale-download", FileStatus::Downloading, 20);
        seed_own_row(&db, "local", FileStatus::Local, 30);
        seed_own_row(&db, "conflict", FileStatus::Conflict, 40);

        let touched = db.reconcile_stale_in_flight_on_startup().unwrap();

        assert_eq!(touched, 2);
        assert_eq!(db.get_file("stale-upload").unwrap().unwrap().status, FileStatus::Error);
        assert_eq!(
            db.get_file("stale-download").unwrap().unwrap().status,
            FileStatus::CloudOnly
        );
        assert_eq!(db.get_file("local").unwrap().unwrap().status, FileStatus::Local);
        assert_eq!(db.get_file("conflict").unwrap().unwrap().status, FileStatus::Conflict);
        assert!(db.list_by_status(FileStatus::Uploading).unwrap().is_empty());
        assert!(db.list_by_status(FileStatus::Downloading).unwrap().is_empty());
    }

    #[test]
    fn prune_absent_keeps_shared_with_me_row_absent_from_snapshot() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        // A shared-with-me row (namespace != my_files) absent from the OWN-tree
        // snapshot must survive — shared content has its own purge path.
        seed_contract_row(&db, "shared", "/Shared with me/x.txt", None, FileStatus::CloudOnly, 0);
        let mut c = db.get_file_contract_state("shared").unwrap().unwrap();
        c.namespace = Namespace::SharedWithMe;
        c.shared_root_id = Some("shared".into());
        db.set_file_contract_state(&c).unwrap();

        // Own-tree snapshot does not mention the shared row.
        let seen: HashSet<String> = ["my-own".to_string()].into_iter().collect();
        let pruned = db.prune_absent(&seen, FAR_FUTURE).unwrap();
        assert!(pruned.is_empty());
        assert!(
            db.get_file("shared").unwrap().is_some(),
            "shared-with-me row never pruned by own-tree snapshot"
        );
    }

    #[test]
    fn prune_absent_keeps_row_touched_at_or_after_snapshot_fetch() {
        // Issue 5: a just-completed upload re-keyed to the server id is `Local`
        // with `remote_updated_at = now` and NO operation_queue row. A snapshot
        // taken before the completion must NOT prune it.
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        let fetched_at = 1_000_000;
        // Row stamped AT the fetch instant (the boundary) — must be protected.
        seed_own_row(&db, "just-uploaded", FileStatus::Local, fetched_at);
        // A genuinely older settled row that the snapshot omits — must be pruned.
        seed_own_row(&db, "old-deleted", FileStatus::CloudOnly, fetched_at - 100);

        let seen: HashSet<String> = ["something-else".to_string()].into_iter().collect();
        let pruned = db.prune_absent(&seen, fetched_at).unwrap();

        assert_eq!(
            pruned.iter().map(|r| r.file_id.clone()).collect::<Vec<_>>(),
            vec!["old-deleted".to_string()]
        );
        assert!(
            db.get_file("just-uploaded").unwrap().is_some(),
            "row stamped at/after snapshot fetch survives (upload re-key window)"
        );
    }

    #[test]
    fn prune_absent_refuses_empty_snapshot_against_non_empty_tree() {
        // Issue 4: an empty `seen` set against a non-empty prunable own-tree is a
        // suspected degraded snapshot — refuse to prune anything.
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed_own_row(&db, "a", FileStatus::CloudOnly, 10);
        seed_own_row(&db, "b", FileStatus::CloudOnly, 10);

        let pruned = db.prune_absent(&HashSet::new(), FAR_FUTURE).unwrap();
        assert!(pruned.is_empty(), "empty snapshot must not prune (fail-closed)");
        assert!(db.get_file("a").unwrap().is_some());
        assert!(db.get_file("b").unwrap().is_some());
    }

    #[test]
    fn prune_absent_empty_snapshot_with_only_protected_rows_is_a_noop() {
        // Empty snapshot is allowed to proceed when there are NO prunable
        // candidates (only protected rows): nothing is deleted, no false refusal
        // signal needed. Here the only own-tree row is uploading (protected).
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed_own_row(&db, "uploading-only", FileStatus::Uploading, 10);

        let pruned = db.prune_absent(&HashSet::new(), FAR_FUTURE).unwrap();
        assert!(pruned.is_empty());
        assert!(db.get_file("uploading-only").unwrap().is_some());
    }

    // ── orphan-subtree + delete_file_subtree coverage (task 0806) ──────────────

    /// Seed an own-tree row at an explicit `path` with a given kind, so the
    /// path-prefix orphan logic can be exercised (a folder + its descendants).
    fn seed_own_at(db: &StateDb, file_id: &str, path: &str, is_dir: bool) {
        db.upsert_file(&FileEntry {
            file_id: file_id.into(),
            path: path.into(),
            status: FileStatus::CloudOnly,
            size_bytes: 1,
            modified_at: 0,
            content_hash: None,
            remote_updated_at: 0,
            parent_id: None,
            item_kind: if is_dir { ItemKind::Folder } else { ItemKind::File },
        })
        .unwrap();
        // upsert_file does NOT persist item_kind (it's owned by the contract row),
        // so a folder must have its kind stamped via the contract state.
        if is_dir {
            db.set_file_contract_state(&FileContractState {
                file_id: file_id.into(),
                namespace: Namespace::MyFiles,
                parent_id: None,
                shared_root_id: None,
                share_id: None,
                owner_email: None,
                permission_bits: 0,
                item_kind: ItemKind::Folder,
                content_type: None,
                current_version: 0,
                current_object_version_id: None,
                local_base_version: 0,
                local_hash: None,
                cache_path: None,
                cache_bytes: 0,
                pin_state: PinState::Inherit,
                inherited_pin_state: PinState::Inherit,
                last_sync_at: 0,
            })
            .unwrap();
        }
    }

    #[test]
    fn prune_absent_removes_orphaned_descendants_of_a_trashed_folder() {
        // task 0806/0807: the server trash is NOT recursive, so trashing a FOLDER
        // drops only the folder from the snapshot; its CHILDREN remain in `seen`.
        // prune_absent must still remove those orphans by PATH-PREFIX, ordered
        // children-before-parent.
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed_own_at(&db, "folder", "docs", true);
        seed_own_at(&db, "child", "docs/a.txt", false);
        seed_own_at(&db, "subfolder", "docs/sub", true);
        seed_own_at(&db, "grandchild", "docs/sub/b.txt", false);
        // A sibling that is NOT under the folder must survive.
        seed_own_at(&db, "outside", "top.txt", false);

        // Snapshot still lists the children + the sibling but NOT the folder.
        let seen: HashSet<String> = ["child", "subfolder", "grandchild", "outside"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let pruned = db.prune_absent(&seen, FAR_FUTURE).unwrap();

        let ids: HashSet<String> = pruned.iter().map(|r| r.file_id.clone()).collect();
        assert!(ids.contains("folder"), "the trashed folder is pruned");
        assert!(ids.contains("child"), "orphaned child pruned by path-prefix");
        assert!(ids.contains("subfolder"), "orphaned subfolder pruned");
        assert!(ids.contains("grandchild"), "orphaned grandchild pruned");
        assert!(!ids.contains("outside"), "a non-descendant sibling is NOT pruned");

        // Children precede their parent folder in the returned order (so the
        // Windows caller removes leaf placeholders before the directory).
        let pos = |id: &str| pruned.iter().position(|r| r.file_id == id).unwrap();
        assert!(pos("grandchild") < pos("subfolder"), "grandchild before its subfolder");
        assert!(pos("child") < pos("folder"), "child before the root folder");
        assert!(pos("subfolder") < pos("folder"), "subfolder before the root folder");

        assert!(db.get_file("folder").unwrap().is_none());
        assert!(db.get_file("child").unwrap().is_none());
        assert!(db.get_file("grandchild").unwrap().is_none());
        assert!(db.get_file("outside").unwrap().is_some(), "sibling row survives");
    }

    #[test]
    fn prune_absent_prunes_descendants_when_folder_row_has_leading_slash() {
        // task 0806 review (high): on the degraded-decrypt path a ROOT-level folder
        // can be stored leading-slash-first (`/docs`) while its children are ALWAYS
        // stored leading-slash-free (`docs/a.txt`). Trimming only the trailing slash
        // would make the prefix `/docs/` miss `docs/a.txt` → orphan leak. The match
        // normalizes both ends, so the child is still pruned.
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed_own_at(&db, "folder", "/docs", true);
        seed_own_at(&db, "child", "docs/a.txt", false);

        // Snapshot lists the child but NOT the trashed folder.
        let seen: HashSet<String> = ["child"].iter().map(|s| s.to_string()).collect();
        let pruned = db.prune_absent(&seen, FAR_FUTURE).unwrap();

        let ids: HashSet<String> = pruned.iter().map(|r| r.file_id.clone()).collect();
        assert!(ids.contains("folder"), "the trashed leading-slash folder is pruned");
        assert!(
            ids.contains("child"),
            "leading-slash-free child of a leading-slash folder is still pruned"
        );
        assert!(db.get_file("child").unwrap().is_none(), "child row removed");
    }

    #[test]
    fn delete_file_subtree_prunes_descendants_when_folder_row_has_leading_slash() {
        // task 0806 review (high): same leading-slash mismatch on the OPS path
        // (`apply_sync_op` file_trash). A folder stored `/docs` must still sweep its
        // leading-slash-free child `docs/a.txt`.
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed_own_at(&db, "folder", "/docs", true);
        seed_own_at(&db, "child", "docs/a.txt", false);

        let removed = db.delete_file_subtree("folder").unwrap();
        let ids: HashSet<String> = removed.iter().map(|r| r.file_id.clone()).collect();
        assert!(ids.contains("folder"), "leading-slash folder removed");
        assert!(ids.contains("child"), "leading-slash-free child swept");
        // children-before-parent ordering still holds.
        let pos = |id: &str| removed.iter().position(|r| r.file_id == id).unwrap();
        assert!(pos("child") < pos("folder"), "child before its root folder");
        assert!(db.get_file("child").unwrap().is_none());
    }

    #[test]
    fn delete_file_subtree_removes_folder_and_descendants_children_first() {
        // The OPS reconcile path (`apply_sync_op` file_trash for a FOLDER) calls
        // this: the folder row + its path-prefix descendants are removed in one
        // transaction, children-before-parent.
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed_own_at(&db, "folder", "Projects", true);
        seed_own_at(&db, "f1", "Projects/report.txt", false);
        seed_own_at(&db, "sub", "Projects/deep", true);
        seed_own_at(&db, "f2", "Projects/deep/data.bin", false);
        // A path that merely SHARES a prefix string but is not a child
        // ("Projects2") must NOT be swept (prefix equality uses the `/` boundary).
        seed_own_at(&db, "decoy", "Projects2/x.txt", false);

        let removed = db.delete_file_subtree("folder").unwrap();
        let ids: HashSet<String> = removed.iter().map(|r| r.file_id.clone()).collect();
        assert_eq!(
            ids,
            ["folder", "f1", "sub", "f2"].iter().map(|s| s.to_string()).collect()
        );
        let pos = |id: &str| removed.iter().position(|r| r.file_id == id).unwrap();
        assert!(pos("f2") < pos("sub"), "deep file before its folder");
        assert!(pos("f1") < pos("folder"), "child file before the root folder");
        // Root folder is LAST (deepest-first ordering then the root appended).
        assert_eq!(removed.last().unwrap().file_id, "folder");

        assert!(db.get_file("folder").unwrap().is_none());
        assert!(db.get_file("f2").unwrap().is_none());
        assert!(
            db.get_file("decoy").unwrap().is_some(),
            "a sibling sharing only a prefix STRING (Projects2) must not be swept"
        );
    }

    #[test]
    fn delete_file_subtree_of_a_plain_file_removes_only_that_row() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed_own_at(&db, "lonefile", "a.txt", false);
        seed_own_at(&db, "other", "b.txt", false);

        let removed = db.delete_file_subtree("lonefile").unwrap();
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].file_id, "lonefile");
        assert!(!removed[0].is_dir);
        assert!(db.get_file("lonefile").unwrap().is_none());
        assert!(db.get_file("other").unwrap().is_some());

        // Unknown id → empty, no-op.
        assert!(db.delete_file_subtree("does-not-exist").unwrap().is_empty());
    }

    #[test]
    fn needs_resnapshot_flag_is_take_once() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        // Unset → false.
        assert!(!db.take_needs_resnapshot().unwrap());
        // Set → next take returns true, then clears.
        db.request_resnapshot().unwrap();
        assert!(db.take_needs_resnapshot().unwrap(), "first take sees the request");
        assert!(!db.take_needs_resnapshot().unwrap(), "flag cleared after one take");
        // Idempotent set.
        db.request_resnapshot().unwrap();
        db.request_resnapshot().unwrap();
        assert!(db.take_needs_resnapshot().unwrap());
        assert!(!db.take_needs_resnapshot().unwrap());
    }

    // ── bandwidth_samples (task 0810 — P3) ───────────────────────────────────

    #[test]
    fn bandwidth_samples_insert_and_history() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();

        // Empty history — no rows yet.
        let history = db.get_bandwidth_history(20).unwrap();
        assert!(history.is_empty(), "fresh DB should have no samples");

        // Insert a few samples at different times.
        let t0: i64 = 1_700_000_000; // arbitrary epoch anchor
        db.insert_bandwidth_sample(t0, 100, 200, 20).unwrap();
        db.insert_bandwidth_sample(t0 + 20, 300, 400, 20).unwrap();
        db.insert_bandwidth_sample(t0 + 40, 500, 600, 20).unwrap();

        // get_bandwidth_history_since(0) returns all rows regardless of wall clock,
        // making this test time-independent.
        let all = db.get_bandwidth_history_since(0).unwrap();
        assert_eq!(all.len(), 3, "expected 3 samples");
        assert_eq!(all[0].up_bytes, 100);
        assert_eq!(all[0].down_bytes, 200);
        assert_eq!(all[0].period_secs, 20);
        assert_eq!(all[2].up_bytes, 500);

        // Samples are ordered oldest-first.
        assert!(all[0].sampled_at < all[1].sampled_at);
        assert!(all[1].sampled_at < all[2].sampled_at);
    }

    #[test]
    fn latest_bandwidth_sample_returns_newest_row() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();

        assert!(db.latest_bandwidth_sample().unwrap().is_none());

        let t0: i64 = 1_700_000_000;
        db.insert_bandwidth_sample(t0, 100, 200, 20).unwrap();
        db.insert_bandwidth_sample(t0 + 40, 500, 600, 20).unwrap();
        db.insert_bandwidth_sample(t0 + 20, 300, 400, 20).unwrap();

        let latest = db.latest_bandwidth_sample().unwrap().expect("latest sample");
        assert_eq!(latest.sampled_at, t0 + 40);
        assert_eq!(latest.up_bytes, 500);
        assert_eq!(latest.down_bytes, 600);
        assert_eq!(latest.period_secs, 20);
    }

    #[test]
    fn bandwidth_samples_prune() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();

        let t0: i64 = 1_700_000_000;
        db.insert_bandwidth_sample(t0, 1, 2, 20).unwrap();
        db.insert_bandwidth_sample(t0 + 3600, 3, 4, 20).unwrap();
        db.insert_bandwidth_sample(t0 + 7200, 5, 6, 20).unwrap();

        // Prune everything before t0 + 3600: should remove the first sample.
        let removed = db.prune_bandwidth_samples(t0 + 3600).unwrap();
        assert_eq!(removed, 1, "one sample should be pruned");

        let remaining = db.get_bandwidth_history_since(0).unwrap();
        assert_eq!(remaining.len(), 2);
        // The remaining ones start at t0 + 3600.
        assert_eq!(remaining[0].sampled_at, t0 + 3600);
    }

    // ── Mbps <-> kbps conversion (task 0810 — E) ─────────────────────────────
    //
    // The desktop UI exposes bandwidth caps as Mbps; config.rs stores them as
    // kbps.  These tests encode the conversion contract so a future refactor
    // can't silently break it.

    #[test]
    fn mbps_to_kbps_conversion() {
        // 1 Mbps = 1000 kbps (SI decimal, matching the network convention used
        // by `formatBytes` on the frontend and the `speed_bar` helper in the CLI).
        // NOTE: the conversion is intentionally decimal (1 Mbps = 1000 kbps),
        // not binary (1 Mibps = 1024 Kibps), to match browser and CLI display.
        let mbps_to_kbps = |mbps: f64| -> u64 { (mbps * 1000.0) as u64 };
        let kbps_to_mbps = |kbps: u64| -> f64 { kbps as f64 / 1000.0 };

        assert_eq!(mbps_to_kbps(1.0), 1000, "1 Mbps = 1000 kbps");
        assert_eq!(mbps_to_kbps(10.0), 10_000, "10 Mbps = 10 000 kbps");
        assert_eq!(mbps_to_kbps(100.0), 100_000, "100 Mbps = 100 000 kbps");
        assert_eq!(mbps_to_kbps(0.0), 0, "0 Mbps (unlimited) = 0 kbps");

        assert!((kbps_to_mbps(1000) - 1.0).abs() < 1e-9);
        assert!((kbps_to_mbps(10_000) - 10.0).abs() < 1e-9);
        assert_eq!(kbps_to_mbps(0), 0.0);

        // Round-trip: any value survives mbps → kbps → mbps within ±0.001 Mbps.
        for mbps in [0.5f64, 2.5, 50.0, 200.0] {
            let kbps = mbps_to_kbps(mbps);
            let back = kbps_to_mbps(kbps);
            assert!((back - mbps).abs() < 0.001, "round-trip {mbps} Mbps failed: got {back}");
        }
    }

    // ── task 0828 regression: orphaned children of an absent folder ───────────
    //
    // Scenario: a folder was already trashed on the server when the desktop's
    // snapshot ran, so the snapshot excluded the folder row.  The folder's
    // CHILDREN were ingested by a prior snapshot and stored with
    // `parent_id = <folder_id>` via `set_file_contract_state`.  When the
    // desktop later receives a `file_trash` sync op for the folder,
    // `get_file(folder_id)` returns `None` — the old `None => {}` no-op left
    // the children as permanent ghost rows.
    //
    // `delete_orphaned_children_of_absent_folder` must sweep and remove ALL
    // descendant rows even when the folder row itself is absent.

    #[test]
    fn orphaned_children_of_absent_folder_are_swept() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();

        // Seed two direct children of an absent folder ("folder-gone").
        // `set_file_contract_state` writes `parent_id` into `files`, so we
        // simulate what the snapshot ingest path does: upsert the file row,
        // then call set_file_contract_state to write parent_id.
        let seed = |file_id: &str, path: &str, parent_id: &str, is_folder: bool| {
            let kind = if is_folder { ItemKind::Folder } else { ItemKind::File };
            db.upsert_file(&FileEntry {
                file_id: file_id.to_string(),
                path: path.to_string(),
                status: FileStatus::CloudOnly,
                size_bytes: 0,
                modified_at: 0,
                content_hash: None,
                remote_updated_at: 0,
                parent_id: None, // upsert_file ignores this field
                item_kind: kind.clone(),
            })
            .unwrap();
            db.set_file_contract_state(&FileContractState {
                file_id: file_id.to_string(),
                namespace: Namespace::MyFiles,
                parent_id: Some(parent_id.to_string()),
                shared_root_id: None,
                share_id: None,
                owner_email: None,
                permission_bits: PERMISSION_READ,
                item_kind: kind,
                content_type: None,
                current_version: 1,
                current_object_version_id: None,
                local_base_version: 0,
                local_hash: None,
                cache_path: None,
                cache_bytes: 0,
                pin_state: PinState::Inherit,
                inherited_pin_state: PinState::Unpinned,
                last_sync_at: 0,
            })
            .unwrap();
        };

        // Two direct children of the absent folder.
        seed("child-file-1", "docs/report.pdf", "folder-gone", false);
        seed("child-file-2", "docs/notes.txt", "folder-gone", false);
        // A sub-folder child (also orphaned) and ITS child, so we test
        // recursive path-prefix sweep for nested orphans.
        seed("child-folder", "docs/sub", "folder-gone", true);
        seed("grandchild", "docs/sub/deep.txt", "child-folder", false);

        // Verify the folder row itself is absent.
        assert!(
            db.get_file("folder-gone").unwrap().is_none(),
            "folder-gone must not exist (mimics the bug condition)"
        );

        // All four rows exist before the sweep.
        assert!(db.get_file("child-file-1").unwrap().is_some());
        assert!(db.get_file("child-file-2").unwrap().is_some());
        assert!(db.get_file("child-folder").unwrap().is_some());
        assert!(db.get_file("grandchild").unwrap().is_some());

        // Execute the fix: sweep orphaned children of the absent folder.
        let removed = db.delete_orphaned_children_of_absent_folder("folder-gone").unwrap();

        // All four rows must be gone.
        assert!(
            db.get_file("child-file-1").unwrap().is_none(),
            "child-file-1 should be removed"
        );
        assert!(
            db.get_file("child-file-2").unwrap().is_none(),
            "child-file-2 should be removed"
        );
        assert!(
            db.get_file("child-folder").unwrap().is_none(),
            "child-folder should be removed"
        );
        assert!(
            db.get_file("grandchild").unwrap().is_none(),
            "grandchild should be removed (nested path-prefix sweep)"
        );

        // The returned PrunedRow vec must contain all four (order may vary, but
        // the grandchild must precede the child-folder it lives under — that is
        // the children-before-parent contract the Windows placeholder remover
        // depends on).
        assert_eq!(removed.len(), 4, "should have removed exactly 4 rows");
        let ids: Vec<&str> = removed.iter().map(|r| r.file_id.as_str()).collect();
        assert!(ids.contains(&"child-file-1"));
        assert!(ids.contains(&"child-file-2"));
        assert!(ids.contains(&"child-folder"));
        assert!(ids.contains(&"grandchild"));

        let grandchild_pos = ids.iter().position(|&id| id == "grandchild").unwrap();
        let child_folder_pos = ids.iter().position(|&id| id == "child-folder").unwrap();
        assert!(
            grandchild_pos < child_folder_pos,
            "grandchild ({grandchild_pos}) must appear before child-folder ({child_folder_pos}) \
             so the Windows placeholder remover can remove leaf-before-dir"
        );
    }

    #[test]
    fn orphaned_children_sweep_is_noop_when_no_children() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();

        // No rows in the DB at all — the sweep must be a safe no-op.
        let removed = db
            .delete_orphaned_children_of_absent_folder("nonexistent-folder")
            .unwrap();
        assert!(removed.is_empty(), "no children → should return empty vec");
    }

    /// Flow 7 / PR #58 merge with task 1538: every purge that drops queued
    /// upload ops must drop their persisted upload sessions too. A resume row
    /// outliving its op is dead state; across a sign-out it would carry the
    /// previous account's upload session id, server file id and staged path
    /// into the next account's database.
    #[test]
    fn test_every_operation_purge_drops_the_upload_resume_rows() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();

        let mk_op = |op_id: &str, file_id: &str, key: Option<&str>| PendingOperation {
            op_id: op_id.into(),
            kind: OperationKind::UploadFile,
            file_id: Some(file_id.into()),
            parent_id: None,
            target_path: Some(format!("/{op_id}.bin")),
            metadata_json: Some(r#"{"operation":"create_file"}"#.into()),
            payload_path: Some(format!("/staging/{op_id}")),
            base_version: None,
            base_object_version_id: None,
            attempts: 1,
            max_attempts: 5,
            next_retry_at: 0,
            last_error: None,
            backup_source_key: key.map(str::to_string),
            created_at: 100,
            updated_at: 100,
        };
        let mk_resume = |op_id: &str| UploadResume {
            op_id: op_id.into(),
            payload_path: format!("/staging/{op_id}"),
            payload_size: 3 * 1024,
            payload_mtime_ns: 42,
            upload_session_id: format!("session-{op_id}"),
            server_file_id: format!("server-{op_id}"),
            object_version_id: format!("ov-{op_id}"),
            chunk_size_bytes: 1024,
            chunk_count: 3,
            acked_chunks: 1,
            metadata_applied: true,
            is_create: true,
        };
        let seed = |op_id: &str, file_id: &str, key: Option<&str>| {
            db.enqueue_operation(&mk_op(op_id, file_id, key)).unwrap();
            db.put_upload_resume(&mk_resume(op_id)).unwrap();
        };

        // (1) Disabling a known-folder backup: only that folder's session goes.
        seed("music-1", "file-music-1", Some("music"));
        seed("normal-1", "file-normal-1", None);
        assert_eq!(db.purge_backup_source_ops("music").unwrap(), 1);
        assert!(
            db.get_upload_resume("music-1").unwrap().is_none(),
            "a purged backup op's upload session must go with it"
        );
        assert!(
            db.get_upload_resume("normal-1").unwrap().is_some(),
            "a surviving op keeps its upload session"
        );

        // (2) Revoked shared content: the revoked file's queued upload goes, and
        //     so does its session.
        seed_contract_row(&db, "revoked", "/Shared with me/Revoked", None, FileStatus::Local, 20);
        let mut revoked = db.get_file_contract_state("revoked").unwrap().unwrap();
        revoked.namespace = Namespace::SharedWithMe;
        revoked.shared_root_id = Some("revoked".into());
        revoked.share_id = Some("invite-revoked".into());
        revoked.permission_bits = PERMISSION_READ | PERMISSION_WRITE;
        db.set_file_contract_state(&revoked).unwrap();
        seed("shared-1", "revoked", None);
        db.purge_revoked_shared_content(&[]).unwrap();
        assert!(
            db.get_upload_resume("shared-1").unwrap().is_none(),
            "a revoked share's queued upload must not keep its upload session"
        );
        assert!(db.get_upload_resume("normal-1").unwrap().is_some());

        // (3) Sign-out: nothing of the leaving account's uploads survives.
        let purge = db.purge_all_local_state().unwrap();
        assert!(purge.queued_ops_purged >= 1);
        assert!(
            db.get_upload_resume("normal-1").unwrap().is_none(),
            "sign-out must purge every persisted upload session"
        );
    }

    #[test]
    fn has_pending_trash_sees_only_a_queued_trash_for_that_file() {
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        let op = |op_id: &str, kind: OperationKind, file_id: &str| PendingOperation {
            op_id: op_id.into(),
            kind,
            file_id: Some(file_id.into()),
            parent_id: None,
            target_path: None,
            metadata_json: None,
            payload_path: None,
            base_version: None,
            base_object_version_id: None,
            attempts: 25,
            max_attempts: 25, // out of attempts: still a queued delete
            next_retry_at: i64::MAX,
            last_error: None,
            backup_source_key: None,
            created_at: 0,
            updated_at: 0,
        };
        db.enqueue_operation(&op("t1", OperationKind::TrashFile, "trashed")).unwrap();
        db.enqueue_operation(&op("u1", OperationKind::UploadVersion, "uploaded")).unwrap();
        assert!(db.has_pending_trash("trashed").unwrap(), "a queued trash, even out of retries");
        assert!(!db.has_pending_trash("uploaded").unwrap(), "another kind of op is not a delete");
        assert!(!db.has_pending_trash("never-seen").unwrap());
        db.remove_operation("t1").unwrap();
        assert!(!db.has_pending_trash("trashed").unwrap(), "gone once the op is removed");
    }

    // ------------------------------------------------------------------
    // Task 1697: per-domain change log + monotonic anchor (state_db).
    // RED-first: these were written against unmodified code and were seen
    // failing (no fp_changes table, no API) before the implementation.
    // ------------------------------------------------------------------

    use super::FpChangeKind;

    fn change_entry() -> FileEntry {
        FileEntry {
            file_id: "ch-1".into(),
            path: "/docs/report.txt".into(),
            status: FileStatus::Local,
            size_bytes: 10,
            modified_at: 100,
            content_hash: None,
            remote_updated_at: 100,
            parent_id: None,
            item_kind: ItemKind::File,
        }
    }

    fn seed_child(db: &StateDb, file_id: &str, path: &str, kind: ItemKind) {
        seed_child_under(db, file_id, path, kind, None);
    }

    fn seed_child_under(
        db: &StateDb,
        file_id: &str,
        path: &str,
        kind: ItemKind,
        parent_id: Option<&str>,
    ) {
        db.upsert_file(&FileEntry {
            file_id: file_id.into(),
            path: path.into(),
            status: FileStatus::Local,
            size_bytes: 5,
            modified_at: 1,
            content_hash: None,
            remote_updated_at: 1,
            parent_id: None,
            item_kind: kind.clone(),
        })
        .unwrap();
        // parent_id is a contract-owned column (upsert_file never writes it);
        // the contract write is what puts the row in its real parent folder.
        let parent = parent_id.map(str::to_string);
        db.set_file_contract_state(&FileContractState {
            file_id: file_id.into(),
            namespace: Namespace::MyFiles,
            parent_id: parent,
            shared_root_id: None,
            share_id: None,
            owner_email: None,
            permission_bits: 0,
            item_kind: kind.clone(),
            content_type: None,
            current_version: 1,
            current_object_version_id: None,
            local_base_version: 0,
            local_hash: None,
            cache_path: None,
            cache_bytes: 0,
            pin_state: PinState::Inherit,
            inherited_pin_state: PinState::Unpinned,
            last_sync_at: 0,
        })
        .unwrap();
    }

    #[test]
    fn change_log_anchors_are_monotonic_across_batches() {
        let db = StateDb::open(":memory:").unwrap();
        seed_child(&db, "f-1", "/a.txt", ItemKind::File);
        db.record_file_change("f-1", FpChangeKind::Created, None).unwrap();
        let first = db.list_file_changes(None).unwrap();
        let (_, anchor_1) = first.expect("changes since nil");
        let anchor_1 = anchor_1.expect("an anchor after real changes");
        assert!(
            anchor_1.len() <= 500,
            "anchor must stay within Apple's 500-byte budget, got {} bytes",
            anchor_1.len()
        );
        assert!(!anchor_1.is_empty(), "the anchor after changes must not be empty");

        db.record_file_change("f-1", FpChangeKind::Modified, None).unwrap();
        let (_, anchor_2) = db.list_file_changes(Some(&anchor_1)).unwrap().expect("changes since anchor 1");
        let anchor_2 = anchor_2.expect("an anchor after further changes");
        assert!(
            anchor_2 > anchor_1,
            "anchors must sort strictly ascending so 'anchor_1 > anchor_2' can never be true (got {:?} then {:?})",
            anchor_1,
            anchor_2
        );
    }

    #[test]
    fn change_log_anchor_is_stable_when_nothing_changed() {
        let db = StateDb::open(":memory:").unwrap();
        seed_child(&db, "f-1", "/a.txt", ItemKind::File);
        db.record_file_change("f-1", FpChangeKind::Created, None).unwrap();
        let (_, anchor_1) = db.list_file_changes(None).unwrap().unwrap();
        let anchor_1 = anchor_1.unwrap();
        // Nothing happened since: the anchor must be stable and the batch empty.
        let (changes, anchor_2) = db.list_file_changes(Some(&anchor_1)).unwrap().unwrap();
        assert!(changes.is_empty(), "no changes since the fresh anchor: {changes:?}");
        assert_eq!(anchor_2.as_deref(), Some(anchor_1.as_ref()), "a no-op poll must not move the anchor");
    }

    #[test]
    fn change_log_reports_old_and_new_parent_for_reparents() {
        let db = StateDb::open(":memory:").unwrap();
        seed_child_under(&db, "child-1", "/docs/child-1", ItemKind::File, Some("old-parent"));
        seed_child(&db, "old-parent", "/docs", ItemKind::Folder);
        seed_child(&db, "new-parent", "/archive", ItemKind::Folder);
        db.record_file_change("child-1", FpChangeKind::Created, None).unwrap();
        // The move itself re-parents the row (what a real reparent does) —
        // `set_file_contract_state` records the Reparented change itself now.
        db.set_file_contract_state(&FileContractState {
            parent_id: Some("new-parent".into()),
            ..db.get_file_contract_state("child-1").unwrap().unwrap()
        })
        .unwrap();
        let (changes, _) = db.list_file_changes(None).unwrap().unwrap();
        let reparent = changes
            .iter()
            .find(|change| {
                change.file_id == "child-1"
                    && change.kind == FpChangeKind::Reparented
                    && change.old_parent_id.as_deref() == Some("old-parent")
            })
            .expect("the real reparent (old parent = old-parent) must come back");
        assert_eq!(reparent.old_parent_id.as_deref(), Some("old-parent"), "old parent for materialized-set filtering");
        assert_eq!(reparent.new_parent_id.as_deref(), Some("new-parent"), "new parent read from the files row at record time");
    }

    #[test]
    fn change_log_records_deletes_with_the_old_parent() {
        let db = StateDb::open(":memory:").unwrap();
        seed_child_under(&db, "child-1", "/docs/child-1", ItemKind::File, Some("parent-1"));
        seed_child(&db, "parent-1", "/docs", ItemKind::Folder);
        db.record_file_change("child-1", FpChangeKind::Created, None).unwrap();
        db.delete_file("child-1").unwrap();
        // The row is gone; the recorder must have captured the parent
        // (from the files row) BEFORE the delete — the caller passes it as
        // old_parent_id and record_file_change pins it as the change's parent
        // when the row no longer exists.
        db.record_file_change("child-1", FpChangeKind::Deleted, Some("parent-1".into())).unwrap();
        let (changes, _) = db.list_file_changes(None).unwrap().unwrap();
        let deletion = changes
            .iter()
            .find(|change| change.file_id == "child-1" && change.kind == FpChangeKind::Deleted)
            .expect("the deleted change must come back");
        assert_eq!(deletion.new_parent_id.as_deref(), Some("parent-1"), "deleted items report their old parent as new_parent_id");
    }
    #[test]
    fn change_log_paging_returns_every_change_exactly_once() {
        let db = StateDb::open(":memory:").unwrap();
        // Raw upserts only: each one records its own Created change (the
        // wired behavior). No contracts — parent/contract writes would add
        // their own changes and shift the page math.
        for i in 0..250 {
            db.upsert_file(&FileEntry {
                file_id: format!("f-{i}"),
                path: format!("/f-{i}"),
                status: FileStatus::Local,
                size_bytes: 5,
                modified_at: 1,
                content_hash: None,
                remote_updated_at: 1,
                parent_id: None,
                item_kind: ItemKind::File,
            })
            .unwrap();
        }
        let mut seen = std::collections::HashSet::new();
        let mut pages = 0;
        let mut cursor: Option<Vec<u8>> = None;
        loop {
            let (changes, anchor) = db.list_file_changes_paged(cursor.as_deref(), 100).unwrap().unwrap();
            assert!(changes.len() <= 100, "a page must honor the limit, got {}", changes.len());
            for change in &changes {
                assert!(seen.insert((change.file_id.clone(), change.kind)), "a change came back twice: {:?}", change);
            }
            pages += 1;
            match anchor {
                Some(a) if a == cursor.clone().unwrap_or_default() => {
                    // The up-to-date marker echoes the caller's own cursor —
                    // the window is complete.
                    break;
                }
                Some(a) => {
                    assert!(a > cursor.clone().unwrap_or_default(), "paging anchors must strictly increase");
                    cursor = Some(a);
                }
                None => break,
            }
            assert!(pages < 10, "250 changes at 100/page must take 3 pages, not spin forever");
        }
        assert_eq!(seen.len(), 250, "every change must be delivered exactly once");
        // 3 delivery pages + 1 final round trip whose echoed anchor breaks
        // the loop.
        assert_eq!(pages, 4, "3 delivery pages + 1 up-to-date round trip, got {pages}");
    }

    #[test]
    fn change_log_is_per_domain_and_sweeps_only_old_rows() {
        let db = StateDb::open(":memory:").unwrap();
        seed_child(&db, "f-1", "/a.txt", ItemKind::File);
        db.record_file_change("f-1", FpChangeKind::Created, None).unwrap();
        let (_, anchor) = db.list_file_changes(None).unwrap().unwrap();
        let anchor = anchor.unwrap();
        // Sweeping everything older than now prunes the consumed change but
        // must never touch the anchor row (the replica's cursor survives).
        db.sweep_file_changes(2_000_000_000).unwrap();
        let (changes, anchor_after) = db.list_file_changes(Some(&anchor)).unwrap().unwrap();
        assert!(changes.is_empty(), "swept changes are no longer deliverable: {changes:?}");
        assert_eq!(anchor_after.as_deref(), Some(anchor.as_ref()), "the anchor survives the sweep");
        // After the sweep the log is empty (tip = 0), so a from-nil listing
        // starts a FRESH enumeration — the sweep means "history older than the
        // cutoff is gone", not "the anchor resets". The persistent anchor ROW
        // (fp_last_anchor) is what must survive for currentSyncAnchor:
        assert_eq!(db.fp_last_anchor().unwrap().as_deref(), Some(anchor.as_ref()),
            "the persistent anchor survives the sweep (crash recovery reads it)");
    }

    #[test]
    fn change_log_anchor_survives_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        let db = StateDb::open(&path).unwrap();
        seed_child(&db, "f-1", "/a.txt", ItemKind::File);
        db.record_file_change("f-1", FpChangeKind::Created, None).unwrap();
        let (_, anchor) = db.list_file_changes(None).unwrap().unwrap();
        let anchor = anchor.unwrap();
        drop(db);
        let db = StateDb::open(&path).unwrap();
        let (_, reopened) = db.list_file_changes(Some(&anchor)).unwrap().unwrap();
        assert_eq!(reopened.as_deref(), Some(anchor.as_ref()), "the anchor and consumed state must survive process death");
    }

    // ------------------------------------------------------------------
    // Task 1697 review fix (T2): `record_file_change` stamped
    // `recorded_at = 0`, so the macOS signal path's 7-day sweep
    // (`sweep_file_changes(now - 7d)`) deleted the ENTIRE fresh log right
    // after asking File Provider to enumerate it — Finder received an empty
    // feed and missed every update. RED-first: these tests were run against
    // the `recorded_at = 0` insert and were seen failing (0 fresh rows
    // survived the sweep; recorded_at read back 0).
    // ------------------------------------------------------------------

    fn recorded_at_of(db: &StateDb, file_id: &str) -> i64 {
        db.0.lock().expect("state_db mutex poisoned").query_row(
            "SELECT recorded_at FROM fp_changes WHERE file_id = ?1",
            params![file_id],
            |row| row.get::<_, i64>(0),
        ).unwrap()
    }

    #[test]
    fn change_log_rows_record_the_real_insertion_time() {
        let db = StateDb::open(":memory:").unwrap();
        seed_child(&db, "f-1", "/a.txt", ItemKind::File);
        db.record_file_change("f-1", FpChangeKind::Created, None).unwrap();
        let recorded_at = recorded_at_of(&db, "f-1");
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        assert!(recorded_at > 0, "a recorded change must carry its real insertion time, got recorded_at={recorded_at}");
        assert!(
            recorded_at >= now - 60 && recorded_at <= now + 60,
            "recorded_at={recorded_at} must be wall-clock now (~{now}), not the 0 sentinel"
        );
    }

    #[test]
    fn change_log_sweep_keeps_fresh_rows() {
        // The signal path sweeps with cutoff = now - 7 days. A change
        // recorded NOW must survive it — the sweep exists to age out history,
        // not to wipe the feed File Provider was just asked to enumerate.
        let db = StateDb::open(":memory:").unwrap();
        seed_child(&db, "f-1", "/a.txt", ItemKind::File);
        db.record_file_change("f-1", FpChangeKind::Created, None).unwrap();
        let fresh_rows: i64 = db.0.lock().expect("state_db mutex poisoned").query_row(
            "SELECT COUNT(*) FROM fp_changes WHERE file_id = 'f-1'",
            [],
            |row| row.get(0),
        ).unwrap();
        assert!(fresh_rows > 0, "the seed must have written change rows");
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let deleted = db.sweep_file_changes(now - 7 * 24 * 3600).unwrap();
        assert_eq!(deleted, 0, "fresh rows must NEVER be swept (was: every row, recorded_at=0 < cutoff)");
        // The change is still deliverable AFTER the sweep (no pre-consume:
        // a from-nil listing must still see it).
        let (changes, _) = db.list_file_changes(None).unwrap().unwrap();
        assert!(
            !changes.is_empty() && changes.iter().all(|change| change.file_id == "f-1"),
            "the fresh changes are still deliverable after the sweep: {changes:?}"
        );
        // Invariant: the sweep never touches the anchor row.
        assert!(db.fp_last_anchor().unwrap().is_some(), "the persistent anchor survives the sweep (crash recovery reads it)");
    }

    #[test]
    fn change_log_sweep_deletes_rows_older_than_cutoff_and_keeps_the_boundary() {
        // Backdated rows (direct SQL in test setup — production inserts are
        // always fresh) must age out; the comparison is STRICT `<`, so a row
        // recorded exactly AT the cutoff survives.
        let db = StateDb::open(":memory:").unwrap();
        seed_child(&db, "old-1", "/old.txt", ItemKind::File);
        seed_child(&db, "edge-1", "/edge.txt", ItemKind::File);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let cutoff = now - 7 * 24 * 3600;
        {
            let conn = db.0.lock().expect("state_db mutex poisoned");
            conn.execute(
                "UPDATE fp_changes SET recorded_at = ?1 WHERE file_id = 'old-1'",
                params![cutoff - 1],
            ).unwrap();
            conn.execute(
                "UPDATE fp_changes SET recorded_at = ?1 WHERE file_id = 'edge-1'",
                params![cutoff],
            ).unwrap();
        }
        let count = |file_id: &str| -> i64 {
            db.0.lock().expect("state_db mutex poisoned").query_row(
                "SELECT COUNT(*) FROM fp_changes WHERE file_id = ?1",
                params![file_id],
                |row| row.get(0),
            ).unwrap()
        };
        let old_rows = count("old-1");
        let edge_rows = count("edge-1");
        assert!(old_rows > 0 && edge_rows > 0, "both files must have backdated change rows");
        let deleted = db.sweep_file_changes(cutoff).unwrap();
        assert_eq!(deleted as i64, old_rows, "strict <: only the rows strictly OLDER than the cutoff are swept");
        assert_eq!(count("old-1"), 0, "aged-out rows are gone");
        assert_eq!(count("edge-1"), edge_rows, "a row recorded exactly AT the cutoff survives (strict <)");
        // Invariant: the sweep never touches the anchor row.
        assert!(db.fp_last_anchor().unwrap().is_some(), "the anchor cursor outlives swept history");
    }
}
