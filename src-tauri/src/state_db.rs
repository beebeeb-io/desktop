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
    DiagnosticErrorCode, KnownNames, PAUSE_REASON_LABELS, QUEUE_KIND_LABELS, allowed_label, classify_error_code,
    redact_for_export, redact_secrets_only,
};
use rusqlite::{Connection, OptionalExtension, Result, params};
use serde::Serialize;
use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use std::sync::Mutex;

/// R10 (spec 2026-10-06 §5.6): every table holding one account's data. `has_account_data` counts
/// them and `clear_account_data` empties them. `every_table_is_classified_for_the_account_binding`
/// fails when a new table is not listed here or in `DEVICE_TABLES`.
const ACCOUNT_TABLES: [&str; 17] = [
    "owned_key_records",
    "files",
    "operation_queue",
    "local_activity",
    "transfer_activity",
    "fp_changes",
    "fp_sync_anchor",
    "fp_materialized",
    "upload_finalizations",
    "staged_payloads",
    "upload_resume",
    // A File Provider create's provisional id and the server id it landed as (spec §5.4 row 15): the account's.
    "id_aliases",
    // Task 1640: copies kept from files deleted elsewhere (file ids and names): the account's.
    "kept_copies",
    // Task 1640 (review F3): folders deleted elsewhere that stay on disk for what they hold (names): the account's.
    "removed_folders",
    // Task 2191: the check state of files of unknown content (file ids): the account's.
    "unknown_checks",
    // Task 2191 (D12): placeholders left at another path that hold unknown bytes (names): the account's.
    "unknown_orphans",
    // Task 2190: sync-folder edits held, not sent, because their base is unknown (file ids): the account's.
    "held_edits",
];

/// Task 2191: a file row of unknown content (synced before task 1640: no agreed content and no
/// fetch candidate), whatever its status. The guarded set (spec D2).
const GUARDED_SQL: &str = "item_kind = 'file' AND local_hash IS NULL AND fetched_min_version < 0";

/// Task 2191: a guarded row, as the guard reads it.
#[derive(Debug, Clone, PartialEq)]
pub struct GuardedRow {
    pub file_id: String,
    pub path: String,
    pub status: FileStatus,
    pub fetched_min_version: i64,
}

fn guarded_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<GuardedRow> {
    Ok(GuardedRow {
        file_id: row.get(0)?,
        path: row.get(1)?,
        status: FileStatus::from_str(&row.get::<_, String>(2)?),
        fetched_min_version: row.get(3)?,
    })
}
/// Per-device tables: not counted as account data, emptied by a reset anyway.
const DEVICE_TABLES: [&str; 1] = ["bandwidth_samples"];
const OWNER_USER_ID_KEY: &str = "owner_user_id";
const OWNER_EMAIL_KEY: &str = "owner_email";
/// `PRAGMA user_version` once the one-time adoption window is over (see `adopt_owner_once`). It lives in the
/// database header, not in a table, so no purge or reset can clear it. 0 is every database from before R10.
const ADOPTION_CLOSED: i64 = 1;
/// A Finder domain removal is owed before any engine may start (macOS; task 1834 fix round 1, F3). A `sync_state`
/// row like the owner record: not account data, written in the same transaction as the reset that creates the debt.
const FINDER_REMOVAL_OWED_KEY: &str = "finder_removal_owed";
/// A clear that owed no removal (a sign-out whose Finder removal was confirmed) leaves this mark: the domain is
/// known gone, so the next account's first start, onto an empty and unowned database, owes nothing. Not account data.
const FINDER_DOMAIN_GONE_KEY: &str = "finder_domain_gone";
/// Spec 2026-10-10 §15 question 3, option A. Set by a sign-out's keep step after the engine stop, so a Finder accept
/// that outlived the stop queues nothing. Cleared at once by a sign-out that stops without purging, by the purge with
/// the account's rows, by `engine_start_repair`, and at launch.
const FINDER_ACCEPTS_CLOSED_KEY: &str = "finder_accepts_closed";

pub const LOCAL_ACTIVITY_MAX_ROWS: usize = 200;
/// How long a provisional id keeps resolving to the server id its create landed as (spec §7.1).
pub const ALIAS_MAX_AGE_SECS: i64 = 30 * 86_400;
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

/// The `operation_queue` columns [`pending_operation_from_row`] reads, in order.
const PENDING_OPERATION_COLUMNS: &str = "op_id, kind, file_id, parent_id, target_path, metadata_json, payload_path,
    base_version, base_object_version_id, attempts, max_attempts, next_retry_at,
    last_error, backup_source_key, created_at, updated_at";

/// Queue order (spec §8.5, M2). On macOS it is insertion order, `rowid` alone: a
/// wall clock that steps back must not run a newer save before an older one. Other
/// platforms keep round 3's order, `created_at` then `rowid`.
#[cfg(target_os = "macos")]
const DUE_ORDER_SQL: &str = "ORDER BY rowid ASC";
#[cfg(not(target_os = "macos"))]
const DUE_ORDER_SQL: &str = "ORDER BY created_at ASC, rowid ASC";

/// Parent first (spec D7, I5; task 2226): an op whose parent folder still has a `create_folder` op of its own is not
/// due, whatever state that create is in, and its claim claims nothing, so it spends no attempt on the parent's 404.
/// It becomes due once that create landed (its op is gone). Appended to a `WHERE` on `operation_queue` without an alias;
/// the lookup goes through `idx_operation_queue_file_id`. Empty on macOS: its File Provider path does not change.
#[cfg(target_os = "macos")]
const DUE_PARENT_GATE_SQL: &str = "";
#[cfg(not(target_os = "macos"))]
const DUE_PARENT_GATE_SQL: &str = "AND NOT EXISTS (SELECT 1 FROM operation_queue p
                     WHERE p.kind = 'create_folder' AND p.file_id = operation_queue.parent_id)";

/// D7: the folders whose create cannot land until the person or a pass acts: paused or out of attempts, and every folder
/// made inside one of them, at any depth. A change in one waits for its folder (`waiting_for_folder`) and is no
/// transfer work. For `WITH RECURSIVE`; reads `idx_operation_queue_create_folder` alone.
#[cfg(not(target_os = "macos"))]
const HELD_FOLDER_CTE: &str = "held_folder(file_id) AS (
         SELECT file_id FROM operation_queue
         WHERE kind = 'create_folder' AND file_id IS NOT NULL
           AND (paused_reason IS NOT NULL OR attempts >= max_attempts)
         UNION
         SELECT inner_create.file_id FROM held_folder
         JOIN operation_queue inner_create
           ON inner_create.kind = 'create_folder' AND inner_create.parent_id = held_folder.file_id
         WHERE inner_create.file_id IS NOT NULL
     )";

/// Task 2204: a Windows rename (`metadata.local_rename`) not sent yet: unclaimed, and neither
/// parked nor paused.
const PENDING_LOCAL_RENAME_SQL: &str = "kind IN ('rename_file', 'move_file')
       AND CASE WHEN json_valid(metadata_json) THEN json_extract(metadata_json, '$.local_rename') END = 1
       AND claim_id IS NULL AND attempts < max_attempts AND paused_reason IS NULL";

/// The columns [`file_overview_row`] reads.
const FILE_OVERVIEW_COLUMNS: &str = "file_id, path, status, size_bytes, modified_at, content_hash, remote_updated_at,
     parent_id, item_kind, pin_state, inherited_pin_state";

/// A [`FILE_OVERVIEW_COLUMNS`] row and whether it is pinned.
fn file_overview_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<(FileEntry, bool)> {
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
    Ok((entry, effective == PinState::Pinned))
}

/// Task 2229: the row at a path. The Windows enumeration scan runs this for every file and
/// folder on disk on every pass, so it must reach the row through `idx_files_path`: a full read
/// of `files` per lookup burned a core on a 43k-file tree (0.8.13 to 0.8.15).
const FILE_BY_PATH_SQL: &str = "SELECT file_id, path, status, size_bytes, modified_at, content_hash, remote_updated_at,
            parent_id, item_kind
     FROM files WHERE path = ?1";

/// Task 2229: every column of the row at a path, for [`StateDb::file_row_stamp_by_path`]. The scan
/// runs it for every file it skips, so it too goes through `idx_files_path`.
const FILE_ROW_BY_PATH_SQL: &str = "SELECT * FROM files WHERE path = ?1";

/// Task 2240 (D2): the scan's page cache never goes below SQLite's default, `cache_size = -2000` (2,000 KiB): the value
/// every fresh connection reads, which `task_2240_ui_handles_keep_the_default_cache` checks …
pub(crate) const PAGE_CACHE_FLOOR_KIB: i64 = 2_000;
/// … nor above 128 MiB, which holds `files` and `idx_files_path` up to about 300k rows.
pub(crate) const PAGE_CACHE_CEILING_KIB: i64 = 128 * 1024;

/// Task 2240 (D2): the page cache, in KiB, for a DB of `db_bytes`: the whole DB plus an eighth, within
/// [`PAGE_CACHE_FLOOR_KIB`, `PAGE_CACHE_CEILING_KIB`]. The scan's working set (`files` and `idx_files_path`) is part of
/// the DB, so the DB's size always covers it. SQLite fills the cache only with pages the connection reads, so its memory
/// follows what is read, up to this limit.
pub(crate) fn page_cache_limit_kib(db_bytes: i64) -> i64 {
    let kib = db_bytes.max(0) / 1024;
    (kib + kib / 8).clamp(PAGE_CACHE_FLOOR_KIB, PAGE_CACHE_CEILING_KIB)
}

/// `get_file_by_path`'s other stored shape: `/leaf` ⇄ `leaf`.
fn toggled_slash(path: &str) -> String {
    match path.strip_prefix('/') {
        Some(stripped) => stripped.to_string(),
        None => format!("/{path}"),
    }
}

/// `earlier` comes before `this` in [`DUE_ORDER_SQL`]'s order.
#[cfg(target_os = "macos")]
const EARLIER_IN_QUEUE_SQL: &str = "earlier.rowid < this.rowid";
#[cfg(not(target_os = "macos"))]
const EARLIER_IN_QUEUE_SQL: &str =
    "earlier.created_at < this.created_at OR (earlier.created_at = this.created_at AND earlier.rowid < this.rowid)";

/// Task 2190 (plan §3.1, D2): a parked op the person may decide, as the alias `this`: a File
/// Provider content write (it has a write id) parked on a stale base, unclaimed, with no completion
/// recorded; nothing queued behind it for the file (D2: no newer save, and no trash, move or rename,
/// review-169 #2); and a file the pull has seen at a version past that base, so "Keep this Mac's" has a
/// version to rebase on, and that is not being trashed (review-169 F2: a trash from another device, of
/// the file or of a folder it is in, marks the row `trashing` and queues nothing; a file deleted for
/// good has no row). No folder above it is being trashed or moved on this Mac either (the lead, after
/// F2): a local trash marks only the folder's row, and a local trash, move or rename queues on the
/// folder, never on the file. The walk up `parent_id` is a primary-key lookup per level, and stops at
/// 512 levels (the walk's depth cap, plan §7.4): macOS paths are at most 1024 bytes (`PATH_MAX`) and every
/// level takes at least two (`a/`), so no real file nests deeper. A chain that reaches the cap (a cycle in
/// the rows) is not decidable. The review feed and every decision use this one predicate, so the feed
/// never offers a choice the decision refuses.
const DECIDABLE_PARKED_SQL: &str = "this.kind = 'upload_version' AND this.write_id IS NOT NULL
    AND this.last_error_class = 'stale_base' AND this.attempts >= this.max_attempts
    AND this.claim_id IS NULL AND this.base_version IS NOT NULL
    AND NOT EXISTS (SELECT 1 FROM upload_resume AS resume
                    WHERE resume.op_id = this.op_id AND resume.completed_version IS NOT NULL)
    AND NOT EXISTS (SELECT 1 FROM operation_queue AS later
                    WHERE later.file_id = this.file_id AND later.op_id != this.op_id AND later.rowid > this.rowid)
    AND EXISTS (SELECT 1 FROM files AS file
                WHERE file.file_id = this.file_id AND file.current_version > this.base_version
                  AND file.status != 'trashing')
    AND NOT EXISTS (
        WITH RECURSIVE above(id, depth) AS (
            SELECT parent_id, 1 FROM files WHERE file_id = this.file_id
            UNION ALL
            SELECT folder.parent_id, above.depth + 1 FROM files AS folder JOIN above ON folder.file_id = above.id
             WHERE above.depth < 512)
        SELECT 1 FROM above
         WHERE above.depth >= 512
            OR EXISTS (SELECT 1 FROM files AS folder WHERE folder.file_id = above.id AND folder.status = 'trashing')
            OR EXISTS (SELECT 1 FROM operation_queue AS moving
                       WHERE moving.file_id = above.id AND moving.kind IN ('trash_file', 'move_file', 'rename_file')))";

/// Task 2190: [`StateDb::list_held_edits`]'s read, kept in one place for its query-plan test.
const LIST_HELD_EDITS_SQL: &str = "SELECT held.file_id, file.path, held.reason, held.detected_at
     FROM held_edits AS held INDEXED BY idx_held_edits_detected
     JOIN files AS file ON file.file_id = held.file_id
     ORDER BY held.detected_at, held.file_id";

fn decidable_parked_conn(conn: &Connection, op_id: &str) -> Result<bool> {
    conn.query_row(
        &format!("SELECT EXISTS(SELECT 1 FROM operation_queue AS this WHERE this.op_id = ?1 AND {DECIDABLE_PARKED_SQL})"),
        params![op_id],
        |row| row.get(0),
    )
}

/// Task 2190 (review-169 #1): decidable, and its file still at `shown`, the version the person was
/// shown when they chose. A file past it is a version they have not seen: the decision is `Moved`.
fn decidable_parked_at_conn(conn: &Connection, op_id: &str, shown: i64) -> Result<bool> {
    conn.query_row(
        &format!(
            "SELECT EXISTS(SELECT 1 FROM operation_queue AS this WHERE this.op_id = ?1 AND {DECIDABLE_PARKED_SQL}
               AND EXISTS (SELECT 1 FROM files AS shown WHERE shown.file_id = this.file_id AND shown.current_version = ?2))"
        ),
        params![op_id, shown],
        |row| row.get(0),
    )
}

/// Task 2190 (plan §3.2, review-169 #4): the end of "Keep the other" and "Keep both", in the caller's
/// transaction: the parked op goes, the file's held token is cleared (only this write's), the file
/// presents `local` again, a change is recorded, and the staged copy is RELEASED in the journal
/// (`completed = 1`, as `finish_claimed` does), so a crash before the unlink leaves no unfinished row:
/// the engine-start sweep unlinks it (S6). Returns the copy for the caller to unlink after the commit.
fn retire_parked_conn(tx: &rusqlite::Transaction<'_>, op_id: &str) -> Result<Option<String>> {
    let (file_id, write_id, payload): (String, String, Option<String>) = tx.query_row(
        "SELECT file_id, write_id, payload_path FROM operation_queue WHERE op_id = ?1",
        params![op_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    tx.execute("DELETE FROM operation_queue WHERE op_id = ?1", params![op_id])?;
    tx.execute("DELETE FROM upload_resume WHERE op_id = ?1", params![op_id])?;
    tx.execute(
        "UPDATE files SET held_write_id = NULL, held_base = NULL, held_version = NULL,
                          held_object_version_id = NULL
         WHERE file_id = ?1 AND held_write_id = ?2",
        params![file_id, write_id],
    )?;
    tx.execute(
        "UPDATE files SET status = 'local'
         WHERE file_id = ?1 AND status IN ('error', 'uploading')
           AND NOT EXISTS (SELECT 1 FROM operation_queue
                           WHERE file_id = ?1 AND kind IN ('upload_version', 'upload_file'))",
        params![file_id],
    )?;
    record_file_change_conn(tx, &file_id, FpChangeKind::Modified, None)?;
    if let Some(path) = payload.as_deref() {
        tx.execute(
            "INSERT INTO staged_payloads(path, completed) VALUES (?1, 1)
             ON CONFLICT(path) DO UPDATE SET completed = 1",
            params![path],
        )?;
    }
    Ok(payload)
}

fn pending_operation_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<PendingOperation> {
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
}

/// Task 1640 (review F1): the file's agreed content joins its fetch candidate, as the
/// `files_cloud_only_folds_agreed_into_candidate` trigger does: its bytes are no longer known to be all of
/// that version. An agreed content with no version makes the content not known.
fn fold_agreed_into_candidate_conn(conn: &Connection, file_id: &str) -> Result<()> {
    conn.execute(
        &format!("UPDATE files SET {FOLD_AGREED_INTO_CANDIDATE} WHERE file_id = ?1 AND local_hash IS NOT NULL"),
        params![file_id],
    )?;
    Ok(())
}

/// The fold's SET clause, shared by [`fold_agreed_into_candidate_conn`] and the
/// `files_cloud_only_folds_agreed_into_candidate` trigger (self-review #9: one definition). Every
/// right-hand side reads the row as it was before the update.
const FOLD_AGREED_INTO_CANDIDATE: &str = "
    fetched_hash = CASE
        WHEN fetched_min_version = 0 THEN local_hash
        WHEN fetched_min_version = local_base_version AND fetched_hash = local_hash THEN fetched_hash
        ELSE NULL END,
    fetched_min_version = CASE
        WHEN local_base_version <= 0 THEN -1
        WHEN fetched_min_version = 0 THEN local_base_version
        ELSE MIN(fetched_min_version, local_base_version) END,
    local_hash = NULL";

const KEPT_COPY_COLUMNS: &str = "id, original_file_id, original_path, copy_path, copied_hash, original_removed, reason, kept_at";

fn kept_copy_from_row(row: &rusqlite::Row<'_>) -> Result<KeptCopy> {
    Ok(KeptCopy {
        id: row.get(0)?,
        original_file_id: row.get(1)?,
        original_path: row.get(2)?,
        copy_path: row.get(3)?,
        copied_hash: row.get(4)?,
        original_removed: row.get(5)?,
        reason: KeptReason::from_str(&row.get::<_, String>(6)?),
        kept_at: row.get(7)?,
    })
}

/// [`StateDb::poison_local_content`] on an existing connection.
fn poison_local_content_conn(conn: &Connection, file_id: &str) -> Result<()> {
    fold_agreed_into_candidate_conn(conn, file_id)?;
    conn.execute("UPDATE files SET fetched_hash = NULL WHERE file_id = ?1", params![file_id])?;
    Ok(())
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
    /// Task 1640 (review N2): the file's agreed content when the row was read (a pruned row is
    /// gone from the table by the time its entry is removed).
    pub local_hash: Option<String>,
    /// Task 1640 (review N1): the hash of the one version the range fetches served, when it was one.
    /// Desktop 0.8.15: read only when that version is known (`fetched_min_version > 0`, as
    /// `fetched_content` reads it). A file of unknown content (`-1`, `-2`) can still carry a hash
    /// a fold left, which is no agreed content; with the start-up guard now running beside the
    /// engine, a remote deletion may reach such a file before the guard has marked it, and the
    /// platform's in-sync state is then not trusted for it.
    pub fetched_hash: Option<String>,
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
    /// Task 1640: a file deleted elsewhere held changes this device had not sent; they were kept
    /// as a copy beside it (`file_name` is the copy's).
    KeptLocalCopy,
}

impl LocalActivityKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            LocalActivityKind::MovedToTrash => "moved_to_trash",
            LocalActivityKind::Restored => "restored",
            LocalActivityKind::KeptLocalCopy => "kept_local_copy",
        }
    }

    fn from_str(s: &str) -> Self {
        match s {
            "restored" => LocalActivityKind::Restored,
            "kept_local_copy" => LocalActivityKind::KeptLocalCopy,
            _ => LocalActivityKind::MovedToTrash,
        }
    }
}

/// Task 1640 (review F5): why a file deleted elsewhere was kept on this PC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeptReason {
    /// It held changes not sent: they are kept as a copy (`copy_path`).
    Copy,
    /// Only part of it was on this PC, out of sync: it stays where it is.
    PartlyLocal,
    /// It was open (in use), so it could not be removed yet; a later tick removes it.
    InUse,
    /// It held changes not sent, and they could not be copied: it stays where it is.
    CopyFailed,
}

impl KeptReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Copy => "copy",
            Self::PartlyLocal => "partly_local",
            Self::InUse => "in_use",
            Self::CopyFailed => "copy_failed",
        }
    }

    fn from_str(value: &str) -> Self {
        match value {
            "partly_local" => Self::PartlyLocal,
            "in_use" => Self::InUse,
            "copy_failed" => Self::CopyFailed,
            _ => Self::Copy,
        }
    }
}

/// Task 1640 (review N9, N13): a kept copy to record, [`StateDb::record_kept_copy`].
#[derive(Debug, Clone)]
pub struct KeptCopyInput<'a> {
    pub original_file_id: &'a str,
    pub original_path: &'a str,
    pub copy_path: &'a str,
    pub copied_hash: &'a str,
    pub original_removed: bool,
    pub reason: KeptReason,
    pub kept_at: i64,
}

/// Task 1640 (review N9, N13): one kept copy (paths relative to the sync root, '/'-joined).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeptCopy {
    pub id: i64,
    pub original_file_id: String,
    pub original_path: String,
    pub copy_path: String,
    pub copied_hash: String,
    pub original_removed: bool,
    pub reason: KeptReason,
    pub kept_at: i64,
}

impl KeptCopy {
    /// Task 1640 (review F7): whether the person may dismiss its entry: nothing waits to be
    /// removed, or only part of the file was on this PC and the rest can no longer be fetched,
    /// so no tick will ever resolve it.
    pub fn dismissable(&self) -> bool {
        self.original_removed || self.reason == KeptReason::PartlyLocal
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

/// Spec D5: whether `dir` holds a file anywhere under it. A missing directory holds none; anything that is not a
/// directory (a file, a link, a reparse point) counts as a file, and a directory that cannot be read counts as holding
/// one, so the pass never drops what it could not look at. Links are never followed.
pub(crate) fn tree_holds_a_file(dir: &Path) -> bool {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return false,
        Err(_) => return true,
    };
    for entry in entries {
        let Ok(entry) = entry else { return true };
        match std::fs::symlink_metadata(entry.path()) {
            Ok(meta) if meta.file_type().is_dir() => {
                if tree_holds_a_file(&entry.path()) {
                    return true;
                }
            }
            _ => return true,
        }
    }
    false
}

#[cfg(test)]
#[path = "reconcile_measure_tests.rs"]
mod reconcile_measure_tests;

/// Tasks 1888 + 2226 (spec §3): the rows and ops the reconcile pass plans over, read in one transaction. Rows in rowid
/// order, so folders at the same depth are decided in the order they were written; ops in the queue's order.
fn reconcile_plan_conn(
    conn: &Connection,
    opts: &crate::held_queue::Options,
    disk_has_file: &dyn Fn(&str) -> bool,
) -> Result<(Vec<crate::held_queue::Unit>, crate::held_queue::Counts)> {
    use crate::held_queue::{Op, Row};
    let rows = conn
        .prepare(
            "SELECT file_id, NULLIF(parent_id, ''), path, item_kind = 'folder',
                    current_version > 0 OR remote_updated_at > 0, current_version
             FROM files ORDER BY rowid",
        )?
        .query_map([], |row| {
            Ok(Row {
                file_id: row.get(0)?,
                parent_id: row.get(1)?,
                path: row.get(2)?,
                is_folder: row.get(3)?,
                server_known: row.get(4)?,
                current_version: row.get(5)?,
            })
        })?
        .collect::<Result<Vec<_>>>()?;
    let ops = conn
        .prepare(&format!(
            "SELECT op_id, kind, NULLIF(file_id, ''), NULLIF(parent_id, ''), target_path,
                    kind IN ('upload_version', 'upload_file') AND COALESCE(CASE WHEN json_valid(metadata_json)
                        THEN json_extract(metadata_json, '$.operation') = 'create_file' END, 0),
                    paused_reason,
                    attempts >= max_attempts AND paused_reason IS NULL AND last_error_class IS NULL,
                    last_error_class IS NOT NULL, last_error, payload_path, base_version
             FROM operation_queue {DUE_ORDER_SQL}"
        ))?
        .query_map([], |row| {
            Ok(Op {
                op_id: row.get(0)?,
                kind: row.get(1)?,
                file_id: row.get(2)?,
                parent_id: row.get(3)?,
                target_path: row.get(4)?,
                create_file: row.get(5)?,
                paused_reason: row.get(6)?,
                given_up: row.get(7)?,
                parked: row.get(8)?,
                last_error: row.get(9)?,
                payload_path: row.get(10)?,
                base_version: row.get(11)?,
            })
        })?
        .collect::<Result<Vec<_>>>()?;
    Ok(crate::held_queue::plan(&rows, &ops, opts, &|path| Path::new(path).exists(), disk_has_file))
}

/// A remap's statements, once per remapped folder: each reads through a `parent_id` index (`idx_files_parent_id`,
/// `idx_operation_queue_parent`), never a scan of the whole table (measured on a 43k-row copy: 5.5 s per 256 remaps
/// without them, 0.25 s with).
const REMAP_FILES_SQL: &str = "UPDATE files SET parent_id = ?2 WHERE parent_id = ?1";
const REMAP_OPS_SQL: &str = "UPDATE operation_queue SET parent_id = ?2, updated_at = ?3 WHERE parent_id = ?1";
/// I1: how many rows and ops still name the ghost.
const GHOST_REFERENCES_SQL: &str = "SELECT (SELECT COUNT(*) FROM files WHERE file_id = ?1 OR parent_id = ?1)
      + (SELECT COUNT(*) FROM operation_queue WHERE file_id = ?1 OR parent_id = ?1)";

/// Spec D8: due again now with a fresh retry budget. Never touches `created_at` or `rowid` (the queue's order), nor
/// `paused_reason` (D8 amendment: a change held for the person stays held).
fn rearm_operation_conn(conn: &Connection, op_id: &str, now: i64) -> Result<()> {
    conn.execute(
        "UPDATE operation_queue SET attempts = 0, next_retry_at = ?2, last_error = NULL, updated_at = ?2 WHERE op_id = ?1",
        params![op_id, now],
    )?;
    Ok(())
}

/// One whole unit of the reconcile plan, inside the caller's transaction.
fn apply_reconcile_unit_conn(conn: &Connection, unit: &crate::held_queue::Unit, now: i64) -> Result<()> {
    use crate::held_queue::Unit;
    let needs_review = OperationPauseReason::NeedsReview.as_str();
    let hold = |op_id: &str| {
        conn.execute(
            "UPDATE operation_queue SET paused_reason = ?2, updated_at = ?3 WHERE op_id = ?1",
            params![op_id, needs_review, now],
        )
    };
    match unit {
        Unit::DropLanded { op_id } => {
            conn.execute("DELETE FROM operation_queue WHERE op_id = ?1", params![op_id])?;
            drop_orphaned_upload_resumes(conn)?;
        }
        Unit::Remap { ghost, twin, ghost_op, rearm, hold: superseded } => {
            conn.execute(REMAP_FILES_SQL, params![ghost, twin])?;
            conn.execute(REMAP_OPS_SQL, params![ghost, twin, now])?;
            for op_id in rearm {
                rearm_operation_conn(conn, op_id, now)?;
            }
            for op_id in superseded {
                hold(op_id)?;
            }
            conn.execute("DELETE FROM operation_queue WHERE op_id = ?1", params![ghost_op])?;
            conn.execute("DELETE FROM files WHERE file_id = ?1", params![ghost])?;
            // I1: nothing names the ghost any more, or the whole unit rolls back with its transaction.
            let left: i64 = conn.query_row(GHOST_REFERENCES_SQL, params![ghost], |row| row.get(0))?;
            if left != 0 {
                return Err(rusqlite::Error::StatementChangedRows(left as usize));
            }
        }
        Unit::Hold { op_id } => {
            hold(op_id)?;
        }
        Unit::Release { op_id } => {
            // Back to `auth`, never straight to due (Goal 2, I6): the key check that follows every pass (D11, D12)
            // resumes it under this session's key or holds it as `key_replaced`, as every other `auth` change.
            conn.execute(
                "UPDATE operation_queue SET paused_reason = ?3, next_retry_at = ?2, updated_at = ?2
                 WHERE op_id = ?1 AND paused_reason = ?4",
                params![op_id, now, OperationPauseReason::Auth.as_str(), needs_review],
            )?;
        }
        Unit::Rearm { op_id } => rearm_operation_conn(conn, op_id, now)?,
        Unit::ResolveParent { op_id, file_id, parent } => {
            // D6: the op and its row both get the folder found by path, by primary key. It comes before any remap, so
            // a ghost's remap re-points what was just put under it.
            conn.execute(
                "UPDATE operation_queue SET parent_id = ?2, updated_at = ?3 WHERE op_id = ?1",
                params![op_id, parent, now],
            )?;
            if let Some(file_id) = file_id {
                conn.execute("UPDATE files SET parent_id = ?2 WHERE file_id = ?1", params![file_id, parent])?;
            }
        }
        Unit::DropScaffold { op_ids, file_ids, .. } => {
            // D5: the scaffolding's creates and rows go; its directories are the caller's, after the commit.
            for op_id in op_ids {
                conn.execute("DELETE FROM operation_queue WHERE op_id = ?1", params![op_id])?;
            }
            for file_id in file_ids {
                conn.execute("DELETE FROM files WHERE file_id = ?1", params![file_id])?;
            }
            drop_orphaned_upload_resumes(conn)?;
            // I1: nothing names a dropped row any more, or the whole unit rolls back with its transaction. One scan for
            // the whole subtree (`parent_id` has no index).
            let ids = serde_json::to_string(file_ids).map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
            let left: i64 = conn.query_row(
                "SELECT (SELECT COUNT(*) FROM files
                         WHERE file_id IN (SELECT value FROM json_each(?1)) OR parent_id IN (SELECT value FROM json_each(?1)))
                      + (SELECT COUNT(*) FROM operation_queue
                         WHERE file_id IN (SELECT value FROM json_each(?1)) OR parent_id IN (SELECT value FROM json_each(?1)))",
                params![ids],
                |row| row.get(0),
            )?;
            if left != 0 {
                return Err(rusqlite::Error::StatementChangedRows(left as usize));
            }
        }
    }
    Ok(())
}

/// Drop every persisted upload session whose queued op no longer exists.
/// Called by the bulk `operation_queue` purges so a session never outlives
/// its op (`finish_claimed` already clears its own row).
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
    /// §8.6 rule 1: the version `complete` produced, recorded before any local
    /// bookkeeping. A row with it is never abandoned and lands without the network.
    pub completed_version: Option<i64>,
    /// The object version id that completion produced.
    pub completed_object_version_id: Option<String>,
    /// The server's `mime_type` in that completion: the landing's fallback content type.
    pub completed_mime_type: Option<String>,
}

#[cfg(test)]
thread_local! {
    /// How many of this thread's next landings fail, and whether as "database is locked".
    /// A thread-local, so parallel tests never share it: under `#[tokio::test]` the landing
    /// runs on the test's own thread.
    static FAIL_LANDINGS: std::cell::Cell<(u32, bool)> = const { std::cell::Cell::new((0, false)) };
}

/// T61, P9: the next `n` landings on this thread fail with a database error.
#[cfg(test)]
pub(crate) fn fail_next_landings_for_test(n: u32) {
    FAIL_LANDINGS.with(|left| left.set((n, false)));
}

/// m-11's pause filter: the next `n` landings on this thread fail as SQLite's "database is
/// locked", which `classify_operation_error` reads as `Locked`, a pause.
#[cfg(test)]
pub(crate) fn fail_next_landings_as_locked_for_test(n: u32) {
    FAIL_LANDINGS.with(|left| left.set((n, true)));
}

/// One injected landing failure, if this thread has one left.
#[cfg(test)]
fn injected_landing_failure() -> Option<rusqlite::Error> {
    FAIL_LANDINGS.with(|left| {
        let (n, locked) = left.get();
        if n == 0 {
            return None;
        }
        left.set((n - 1, locked));
        // Not `InvalidQuery`: it displays as "Query is not read-only", which
        // `classify_operation_error` pauses as Permission, and a paused op is never listed
        // again. A full disk classifies as a retryable failure.
        Some(if locked {
            rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_BUSY),
                Some("database is locked".into()),
            )
        } else {
            rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_FULL),
                Some("injected landing failure".into()),
            )
        })
    })
}

/// Who queued a File Provider upload: this build's accept, or an earlier build (spec §10.2).
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteOrigin {
    /// Minted by the round-4 accept transaction: its bytes are the ones its token names.
    Minted,
    /// Queued by an earlier build, given a write id at engine start (spec §10.2).
    EarlierBuild,
}

#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
impl WriteOrigin {
    pub fn as_str(self) -> &'static str {
        match self {
            WriteOrigin::Minted => "minted",
            WriteOrigin::EarlierBuild => "earlier_build",
        }
    }

    pub fn from_db(value: Option<&str>) -> Option<Self> {
        match value? {
            "minted" => Some(WriteOrigin::Minted),
            "earlier_build" => Some(WriteOrigin::EarlierBuild),
            _ => None,
        }
    }
}

/// The round-4 columns of one File Provider upload op. `PendingOperation` is
/// deliberately unchanged (52 literal construction sites); these columns are read
/// and written only by the dedicated round-4 functions.
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinderWrite {
    pub write_id: String,
    pub origin: WriteOrigin,
    pub after_write_id: Option<String>,
    /// 0: the base is known. n >= 1: waiting for a snapshot, after n - 1
    /// successful snapshots that gave no base for the file (spec §6.3.3).
    pub base_pending: i64,
}

/// Everything the payload builder needs, read in one locked call (spec §5.3).
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
#[derive(Debug, Clone)]
pub struct ItemPresentation {
    pub contract: FileContractState,
    pub held: Option<crate::write_token::HeldWrite>,
    /// An op carries `held.write_id`, in any state, parked included.
    pub held_write_queued: bool,
    /// A Finder upload of this file exists that has not parked (spec §9.1).
    pub unparked_finder_upload: bool,
    /// A Finder upload of this file has parked (attempts used up, spec §9.1).
    pub parked_finder_upload: bool,
    pub version_filled: bool,
}

/// One queued operation as a sign-out reads it (spec 2026-10-10 §2, W2): every op but a hydrate or a
/// pin, with its file's path and whether its upload has a recorded completion.
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
#[derive(Debug, Clone, PartialEq)]
pub struct WaitingRow {
    pub op_id: String,
    pub kind: OperationKind,
    pub file_id: Option<String>,
    pub file_path: Option<String>,
    pub payload_path: Option<String>,
    pub origin: Option<WriteOrigin>,
    /// The queue's order (`rowid`): higher was queued later.
    pub queue_order: i64,
    pub attempts: i64,
    pub max_attempts: i64,
    pub paused: bool,
    /// `upload_resume.completed_version` is set: the server has these bytes (§8 item 3).
    pub completed: bool,
}

/// Spec 2026-10-10 §5.2 step 8: what the purge guard read. `unkept`: waiting changes the keep step did not keep or
/// acknowledge (the purge refuses). `waiting_payloads`: every waiting write's staged copy, deleted last (W13).
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PurgeGuardRead {
    pub unkept: Vec<String>,
    pub waiting_payloads: Vec<String>,
}

/// [`StateDb::upsert_file`] on an open connection or transaction.
fn upsert_file_conn<C: std::ops::Deref<Target = Connection>>(conn: &C, e: &FileEntry) -> Result<()> {
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
        record_file_change_conn(conn, &e.file_id, kind, old_parent)?;
    }
    Ok(())
}

/// [`StateDb::get_file`] on an open connection or transaction.
fn get_file_conn<C: std::ops::Deref<Target = Connection>>(conn: &C, file_id: &str) -> Result<Option<FileEntry>> {
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

/// [`StateDb::record_local_write`] on an open connection or transaction.
fn record_local_write_conn<C: std::ops::Deref<Target = Connection>>(
    conn: &C,
    file_id: &str,
    size_bytes: i64,
    modified_at: i64,
) -> Result<()> {
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
        record_file_change_conn(conn, file_id, FpChangeKind::Modified, None)?;
    }
    Ok(())
}

/// The system's trash container, as the File Provider extension names it (the identifier
/// `NSFileProviderTrashContainerItemIdentifier`, or its short form).
pub(crate) fn is_file_provider_trash(id: &str) -> bool {
    matches!(id, "NSFileProviderTrashContainerItemIdentifier" | "trashContainer")
}

/// The last component of a row's path: the item's name.
fn path_leaf(path: &str) -> &str {
    path.trim_matches('/').rsplit('/').next().unwrap_or_default()
}

/// Spec §7.2 (final review): the full relative path of the item `name` under `parent_id`, as a
/// snapshot writes it: the parent row's path, `/`, the name. The name alone at the root, and
/// when the parent row is unknown or has no path. Where an item is listed is decided by its
/// parent alone; the path carries its name, and its folders for the readers that use them.
fn relative_path_under_conn<C: std::ops::Deref<Target = Connection>>(
    conn: &C,
    parent_id: Option<&str>,
    name: &str,
) -> Result<String> {
    let name = path_leaf(name);
    let parent_path: Option<String> = match parent_id {
        Some(parent_id) => conn
            .query_row("SELECT path FROM files WHERE file_id = ?1", params![parent_id], |row| {
                row.get(0)
            })
            .optional()?,
        None => None,
    };
    Ok(match parent_path.as_deref().map(|path| path.trim_matches('/')) {
        Some(parent_path) if !parent_path.is_empty() => format!("{parent_path}/{name}"),
        _ => name.to_string(),
    })
}

/// Spec §7.2 (final review): `file_id` has a queued or parked File Provider write (an upload
/// with a write id). The path sweeps never delete such a row.
fn has_finder_write_conn<C: std::ops::Deref<Target = Connection>>(conn: &C, file_id: &str) -> Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM operation_queue
                        WHERE file_id = ?1 AND write_id IS NOT NULL AND kind IN ('upload_version', 'upload_file'))",
        params![file_id],
        |row| row.get(0),
    )
}

/// Whether a sign-out closed Finder accepts, read inside the caller's transaction (spec 2026-10-10 §15 question 3).
fn finder_accepts_closed_conn(conn: &Connection) -> Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sync_state WHERE key = ?1)",
        params![FINDER_ACCEPTS_CLOSED_KEY],
        |row| row.get(0),
    )
}

/// [`StateDb::enqueue_operation`]'s insert on an open connection or transaction: a new op, or an op with the same id
/// replaced.
/// Queue `op`, or replace the op with its id. Task 2267 (self-review F3): an op acting on an item whose changes are held
/// for the person's decision (`found_while_paused`: the item itself, or the folder it is in) is held with them, and a
/// held op queued again stays held, so nothing of that item is sent before the person decides.
fn insert_operation_conn(conn: &Connection, op: &PendingOperation) -> Result<()> {
    conn.execute(
        "INSERT INTO operation_queue (
                op_id, kind, file_id, parent_id, target_path, metadata_json, payload_path,
                base_version, base_object_version_id, attempts, max_attempts, next_retry_at,
                last_error, last_error_class, paused_reason, backup_source_key, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, NULL,
                CASE WHEN EXISTS (SELECT 1 FROM operation_queue held
                                  WHERE held.paused_reason = ?17 AND held.file_id IN (?3, ?4))
                     THEN ?17 END,
                ?14, ?15, ?16)
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
                paused_reason = CASE WHEN operation_queue.paused_reason = ?17 THEN ?17 ELSE excluded.paused_reason END,
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
            op.updated_at,
            OperationPauseReason::FoundWhilePaused.as_str()
        ],
    )?;
    Ok(())
}

/// Spec 2026-10-10 §5.2 step 6: what a fenced Finder enqueue did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinderEnqueue {
    Queued,
    /// A sign-out closed Finder accepts: nothing was written.
    AcceptsClosed,
}

/// A Finder folder create's row, written in the same transaction as its op, so a refused create leaves no row behind.
pub struct FinderFolderRow<'a> {
    pub entry: &'a FileEntry,
    pub parent_id: Option<&'a str>,
}

/// Spec §7.2 (final review): the path sweeps' last step, in their transaction, after their
/// deletes. Each row with a queued or parked File Provider write whose parent no longer names a
/// row is re-homed at the top level ([`rehome_kept_row_conn`]). A row whose folder is still
/// there is left exactly as it is: a path prefix also reaches the rows of another folder with
/// the same path (one in the trash, or two of the same name), and those are not orphaned.
fn rehome_orphaned_writes_conn<C: std::ops::Deref<Target = Connection>>(conn: &C) -> Result<()> {
    let orphans: Vec<String> = {
        let mut stmt = conn.prepare(
            "SELECT DISTINCT f.file_id FROM files AS f
             JOIN operation_queue AS o ON o.file_id = f.file_id
             WHERE o.write_id IS NOT NULL AND o.kind IN ('upload_version', 'upload_file')
               AND f.parent_id IS NOT NULL
               AND NOT EXISTS(SELECT 1 FROM files AS p WHERE p.file_id = f.parent_id)",
        )?;
        let rows = stmt.query_map([], |row| row.get(0))?;
        rows.collect::<Result<Vec<_>>>()?
    };
    for file_id in orphans {
        rehome_kept_row_conn(conn, &file_id)?;
    }
    Ok(())
}

/// Spec §7.2 (final review): a row a path sweep kept, because a File Provider write of it is
/// queued or parked, whose folder is gone ([`rehome_orphaned_writes_conn`]). It is re-homed at
/// the top level in the sweep's transaction: no parent, its name alone as the path, one
/// `reparented` change, and its writes' parent cleared. A create then lands at the top level; a
/// save sends the move there with its bytes (`parent_changed`). The row stays listed, at the
/// root, throughout.
fn rehome_kept_row_conn<C: std::ops::Deref<Target = Connection>>(conn: &C, file_id: &str) -> Result<()> {
    let Some(row) = get_file_conn(conn, file_id)? else {
        return Ok(());
    };
    conn.execute(
        "UPDATE files SET parent_id = NULL, path = ?2 WHERE file_id = ?1",
        params![file_id, path_leaf(&row.path)],
    )?;
    let writes: Vec<(String, Option<String>)> = {
        let mut stmt = conn.prepare(
            "SELECT op_id, metadata_json FROM operation_queue
             WHERE file_id = ?1 AND write_id IS NOT NULL AND kind IN ('upload_version', 'upload_file')",
        )?;
        let rows = stmt.query_map(params![file_id], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<Result<Vec<_>>>()?
    };
    for (op_id, metadata_json) in writes {
        // A save (not a create, which its `init` places) records the move, so its metadata step
        // sends the top level.
        let metadata_json = metadata_json.map(|raw| match serde_json::from_str::<serde_json::Value>(&raw) {
            Ok(mut metadata) if metadata["operation"].as_str() != Some("create_file") && metadata.is_object() => {
                metadata["parent_changed"] = serde_json::json!(true);
                metadata.to_string()
            }
            _ => raw,
        });
        conn.execute(
            "UPDATE operation_queue SET parent_id = NULL, metadata_json = ?2 WHERE op_id = ?1",
            params![op_id, metadata_json],
        )?;
    }
    record_file_change_conn(conn, file_id, FpChangeKind::Reparented, row.parent_id)
}

/// Spec §7.2 (final review): whether a queued File Provider save of `file_id`, parked or not,
/// recorded that it moved the file.
fn queued_save_moved_conn<C: std::ops::Deref<Target = Connection>>(conn: &C, file_id: &str) -> Result<bool> {
    let mut stmt = conn.prepare(
        "SELECT metadata_json FROM operation_queue
         WHERE file_id = ?1 AND write_id IS NOT NULL AND kind IN ('upload_version', 'upload_file')",
    )?;
    let rows = stmt.query_map(params![file_id], |row| row.get::<_, Option<String>>(0))?;
    for raw in rows {
        let moved = raw?
            .as_deref()
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
            .and_then(|metadata| metadata.get("parent_changed").and_then(serde_json::Value::as_bool));
        if moved == Some(true) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// The name and parent a File Provider save was made under, applied in its accept (spec §7.2).
/// The system hands a rename or a move over with the contents and takes the reply's item as
/// the truth for both, so the row describes them from the accept on; a snapshot or an op echo
/// leaves an `Uploading` or `Error` row's name alone. The path is the full relative path under
/// the new parent. Nothing is written when neither changed.
fn apply_save_name_and_parent_conn<C: std::ops::Deref<Target = Connection>>(
    conn: &C,
    entry: &FileEntry,
    target_path: Option<&str>,
    parent_id: Option<&str>,
) -> Result<()> {
    let Some(target_path) = target_path else {
        return Ok(());
    };
    let moved = entry.parent_id.as_deref() != parent_id;
    if path_leaf(&entry.path) == path_leaf(target_path) && !moved {
        return Ok(());
    }
    let path = relative_path_under_conn(conn, parent_id, target_path)?;
    conn.execute(
        "UPDATE files SET path = ?2, parent_id = ?3 WHERE file_id = ?1",
        params![entry.file_id, path, parent_id],
    )?;
    // A rename or a move, with the old parent, as `upsert_file_conn` records a path change.
    record_file_change_conn(conn, &entry.file_id, FpChangeKind::Reparented, entry.parent_id.clone())
}

/// [`StateDb::delete_file`] on an open connection or transaction.
/// Task 2267, "Keep on this PC only": the queued operations acting on the rows `ids` (their own row or the folder they
/// act in), and the staged copies they hold (payload and resume paths), sorted, once each.
#[cfg(any(target_os = "windows", test))]
fn keep_ops_conn(conn: &Connection, ids: &[String]) -> Result<(Vec<String>, Vec<String>)> {
    let mut ops: Vec<(String, Option<String>)> = Vec::new();
    let mut stmt = conn.prepare("SELECT op_id, payload_path FROM operation_queue WHERE file_id = ?1 OR parent_id = ?1")?;
    for id in ids {
        for op in stmt.query_map(params![id], |row| Ok((row.get(0)?, row.get(1)?)))? {
            let op = op?;
            if !ops.iter().any(|(op_id, _)| *op_id == op.0) {
                ops.push(op);
            }
        }
    }
    let mut staged: Vec<String> = Vec::new();
    for (op_id, payload) in &ops {
        let resumed: Option<String> = conn
            .query_row("SELECT payload_path FROM upload_resume WHERE op_id = ?1", params![op_id], |row| row.get(0))
            .optional()?;
        staged.extend(payload.iter().chain(resumed.iter()).filter(|path| !path.is_empty()).cloned());
    }
    staged.sort();
    staged.dedup();
    Ok((ops.into_iter().map(|(op_id, _)| op_id).collect(), staged))
}

fn delete_file_conn<C: std::ops::Deref<Target = Connection>>(conn: &C, file_id: &str) -> Result<()> {
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
    record_file_change_conn(conn, file_id, FpChangeKind::Deleted, old_parent)?;
    Ok(())
}

/// [`StateDb::set_file_contract_state`] on an open connection or transaction.
fn set_file_contract_state_conn<C: std::ops::Deref<Target = Connection>>(
    conn: &C,
    state: &FileContractState,
) -> Result<()> {
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
            record_file_change_conn(conn, &state.file_id, FpChangeKind::Reparented, old_parent)?;
        } else if metadata_changed {
            record_file_change_conn(conn, &state.file_id, FpChangeKind::Modified, None)?;
        }
    }
    Ok(())
}

fn get_file_contract_state_conn(conn: &Connection, file_id: &str) -> Result<Option<FileContractState>> {
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

#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
fn finder_write_conn(conn: &Connection, op_id: &str) -> Result<Option<FinderWrite>> {
    conn.query_row(
        "SELECT write_id, write_origin, after_write_id, base_pending FROM operation_queue
         WHERE op_id = ?1 AND write_id IS NOT NULL",
        params![op_id],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, i64>(3)?,
            ))
        },
    )
    .optional()
    .map(|found| {
        found.map(|(write_id, origin, after_write_id, base_pending)| FinderWrite {
            write_id,
            // A write id without an origin can only come from a bug; treat it as
            // earlier-build so it is never handed over (spec §8.4, m-2).
            origin: WriteOrigin::from_db(origin.as_deref()).unwrap_or(WriteOrigin::EarlierBuild),
            after_write_id,
            base_pending,
        })
    })
}

/// The claim's transaction ends in a wait, which is not an attempt (S3 steps 2–4). After
/// a hand-over it commits that, and the caller logs it and releases the payload (S6).
fn finish_wait(tx: rusqlite::Transaction<'_>, took_over: Option<TookOver>) -> Result<ClaimOutcome> {
    tx.commit()?;
    Ok(match took_over {
        None => ClaimOutcome::Wait,
        Some(took_over) => ClaimOutcome::WaitAfterHandOver(took_over),
    })
}

/// The park of an op that holds no claim (spec §8.4): attempts used up, the reason
/// recorded, the bytes kept. The claim's parks and the snapshot count's both use it, so
/// a later save on the parked write's token takes its role the same way at its claim.
/// An op whose completion is recorded is never parked (§8.6 rule 6): 0 rows.
fn park_unclaimed_conn(conn: &Connection, op_id: &str, reason: ParkReason, now: i64) -> Result<usize> {
    conn.execute(
        "UPDATE operation_queue
         SET attempts = max_attempts, last_error = ?2, last_error_class = ?2, updated_at = ?3
         WHERE op_id = ?1
           AND NOT EXISTS (SELECT 1 FROM upload_resume WHERE op_id = ?1 AND completed_version IS NOT NULL)",
        params![op_id, reason.as_str(), now],
    )
}

/// §9.1–§9.2, after a park, in the park's own transaction: a parked upload presents `error`
/// (read-only, still not evictable), unless another Finder upload of the file has not parked
/// (§8.4: a successor waiting to take over keeps the item `uploading`). Only an `uploading` row
/// moves: a `Trashing` row keeps its trash presentation (m-8), a `Conflict` row its own.
/// Write-keyed: only a Finder upload is ever parked, so Windows and Linux never reach it. A flip
/// is recorded for the system, as every status change is.
fn settle_status_after_park_conn<C: std::ops::Deref<Target = Connection>>(conn: &C, file_id: &str) -> Result<()> {
    let flipped = conn.execute(
        "UPDATE files SET status = 'error'
         WHERE file_id = ?1 AND status = 'uploading'
           AND NOT EXISTS (SELECT 1 FROM operation_queue
                           WHERE file_id = ?1 AND write_id IS NOT NULL
                             AND kind IN ('upload_version', 'upload_file')
                             AND attempts < max_attempts)",
        params![file_id],
    )?;
    if flipped == 1 {
        record_file_change_conn(conn, file_id, FpChangeKind::Modified, None)?;
    }
    Ok(())
}

/// The claim parks the op with its bytes (spec §8.4, S5), in its own transaction.
fn park_in_claim(
    tx: rusqlite::Transaction<'_>,
    op: &PendingOperation,
    reason: ParkReason,
    took_over: Option<TookOver>,
    now: i64,
) -> Result<ClaimOutcome> {
    // The claim never parks an op whose completion is recorded (it skips steps 3–4 for it);
    // were one to reach here, the park refuses it and it waits, which is not an attempt.
    if park_unclaimed_conn(&tx, &op.op_id, reason, now)? != 1 {
        return finish_wait(tx, took_over);
    }
    if let Some(file_id) = op.file_id.as_deref() {
        settle_status_after_park_conn(&tx, file_id)?;
    }
    tx.commit()?;
    Ok(ClaimOutcome::Parked {
        op_id: op.op_id.clone(),
        file_id: op.file_id.clone(),
        reason,
        took_over,
    })
}

/// A successor's direct predecessor, as the claim reads it (spec §8.4).
struct Predecessor {
    op_id: String,
    kind: String,
    parent_id: Option<String>,
    target_path: Option<String>,
    metadata_json: Option<String>,
    payload_path: Option<String>,
    base_version: Option<i64>,
    base_object_version_id: Option<String>,
    after_write_id: Option<String>,
    base_pending: i64,
    attempts: i64,
    max_attempts: i64,
    origin: Option<WriteOrigin>,
    /// Its `complete` answered and the resume row recorded it (§8.6 rule 1).
    completed: bool,
    /// Why it parked, as the park recorded it (`last_error_class`).
    park_reason: Option<String>,
}

fn predecessor_conn(conn: &Connection, write_id: &str) -> Result<Option<Predecessor>> {
    conn.query_row(
        "SELECT q.op_id, q.kind, q.parent_id, q.target_path, q.metadata_json, q.payload_path,
                q.base_version, q.base_object_version_id, q.after_write_id, q.base_pending,
                q.attempts, q.max_attempts, q.write_origin,
                EXISTS (SELECT 1 FROM upload_resume r
                        WHERE r.op_id = q.op_id AND r.completed_version IS NOT NULL),
                q.last_error_class
         FROM operation_queue q WHERE q.write_id = ?1",
        params![write_id],
        |row| {
            Ok(Predecessor {
                op_id: row.get(0)?,
                kind: row.get(1)?,
                parent_id: row.get(2)?,
                target_path: row.get(3)?,
                metadata_json: row.get(4)?,
                payload_path: row.get(5)?,
                base_version: row.get(6)?,
                base_object_version_id: row.get(7)?,
                after_write_id: row.get(8)?,
                base_pending: row.get(9)?,
                attempts: row.get(10)?,
                max_attempts: row.get(11)?,
                origin: WriteOrigin::from_db(row.get::<_, Option<String>>(12)?.as_deref()),
                completed: row.get(13)?,
                park_reason: row.get(14)?,
            })
        },
    )
    .optional()
}

/// §8.4: successor N takes parked predecessor W's role. N's bytes contain W's (§8.2,
/// last point), so no byte the person saved is lost. W's op and resume row are removed
/// and its payload is journalled for release; the caller unlinks it after the commit
/// (S6). Both rows are addressed exactly; anything else fails and the transaction
/// rolls back.
fn hand_over_conn(
    conn: &Connection,
    n_op_id: &str,
    n_write_id: &str,
    n: &PendingOperation,
    w: &Predecessor,
) -> Result<()> {
    let w_is_create = w
        .metadata_json
        .as_deref()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
        .is_some_and(|m| m["operation"].as_str() == Some("create_file"));
    // A Finder create is `upload_version` with `"operation": "create_file"`.
    // N takes W's create: W's kind, parent and path, and N's metadata as a create.
    let (kind, parent_id, target_path, metadata_json) = if w_is_create {
        let mut metadata: serde_json::Value = n
            .metadata_json
            .as_deref()
            .and_then(|raw| serde_json::from_str(raw).ok())
            .unwrap_or_else(|| serde_json::json!({}));
        metadata["operation"] = serde_json::json!("create_file");
        if let Some(map) = metadata.as_object_mut() {
            map.remove("base_version_identifier");
        }
        (
            w.kind.clone(),
            w.parent_id.clone(),
            w.target_path.clone(),
            Some(metadata.to_string()),
        )
    } else {
        (
            n.kind.as_str().to_string(),
            n.parent_id.clone(),
            n.target_path.clone(),
            n.metadata_json.clone(),
        )
    };
    let moved = conn.execute(
        "UPDATE operation_queue
         SET base_version = ?3, base_object_version_id = ?4, after_write_id = ?5, base_pending = ?6,
             kind = ?7, parent_id = ?8, target_path = ?9, metadata_json = ?10
         WHERE op_id = ?1 AND write_id = ?2",
        params![
            n_op_id,
            n_write_id,
            w.base_version,
            w.base_object_version_id,
            w.after_write_id,
            w.base_pending,
            kind,
            parent_id,
            target_path,
            metadata_json,
        ],
    )?;
    let retired = conn.execute("DELETE FROM operation_queue WHERE op_id = ?1", params![w.op_id])?;
    if moved != 1 || retired != 1 {
        // The caller's `?` drops the transaction, which rolls back.
        return Err(rusqlite::Error::StatementChangedRows(moved + retired));
    }
    conn.execute("DELETE FROM upload_resume WHERE op_id = ?1", params![w.op_id])?;
    if let Some(path) = &w.payload_path {
        conn.execute(
            "INSERT INTO staged_payloads(path, completed) VALUES (?1, 1)
             ON CONFLICT(path) DO UPDATE SET completed = 1",
            params![path],
        )?;
    }
    Ok(())
}

#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
fn held_write_conn(conn: &Connection, file_id: &str) -> Result<(Option<crate::write_token::HeldWrite>, bool)> {
    conn.query_row(
        "SELECT held_write_id, held_base, held_version, held_object_version_id, version_filled
         FROM files WHERE file_id = ?1",
        params![file_id],
        |row| {
            let write_id: Option<String> = row.get(0)?;
            let held = match write_id {
                Some(write_id) => Some(crate::write_token::HeldWrite {
                    write_id,
                    // The two are written together (`set_held_write_conn`). A NULL
                    // `held_base` beside a write id is a bug, and reading it fails
                    // rather than map a base through `b = 0`.
                    base: row.get::<_, i64>(1)?,
                    version: row.get(2)?,
                    object_version_id: row.get(3)?,
                }),
                None => None,
            };
            Ok((held, row.get::<_, i64>(4)? != 0))
        },
    )
}

/// The one writer of a new held write: `held_write_id` and `held_base` are set together,
/// in one statement, and the landing columns are cleared. Exactly one row must match.
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
fn set_held_write_conn(conn: &Connection, file_id: &str, write_id: &str, base: i64) -> Result<()> {
    let n = conn.execute(
        "UPDATE files SET held_write_id = ?2, held_base = ?3, held_version = NULL,
                          held_object_version_id = NULL
         WHERE file_id = ?1",
        params![file_id, write_id, base],
    )?;
    if n == 1 {
        Ok(())
    } else {
        Err(rusqlite::Error::QueryReturnedNoRows)
    }
}

/// The §6.1 facts of `file_id`, read inside the caller's transaction (spec §8.7 S1.1).
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
fn base_facts_conn(
    conn: &Connection,
    file_id: &str,
    contract: &FileContractState,
    legacy_identifiers: Vec<String>,
) -> Result<crate::write_token::BaseFacts> {
    let (held, version_filled) = held_write_conn(conn, file_id)?;
    let held_write_queued = match &held {
        Some(held) => conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM operation_queue WHERE write_id = ?1)",
            params![held.write_id],
            |row| row.get(0),
        )?,
        None => false,
    };
    // The CASE keeps a malformed metadata row from failing every save of the file.
    let create_queued: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM operation_queue
          WHERE file_id = ?1 AND kind IN ('upload_version', 'upload_file')
            AND CASE WHEN json_valid(metadata_json)
                     THEN json_extract(metadata_json, '$.operation') END = 'create_file')",
        params![file_id],
        |row| row.get(0),
    )?;
    let minted_resolved_bases = {
        let mut stmt = conn.prepare(
            "SELECT base_version FROM operation_queue
             WHERE file_id = ?1 AND write_origin = ?2 AND after_write_id IS NULL
               AND base_pending = 0 AND base_version IS NOT NULL",
        )?;
        let rows = stmt.query_map(params![file_id, WriteOrigin::Minted.as_str()], |row| {
            row.get::<_, i64>(0)
        })?;
        rows.collect::<Result<Vec<_>>>()?
    };
    Ok(crate::write_token::BaseFacts {
        current_version: contract.current_version,
        version_filled,
        held,
        held_write_queued,
        create_queued: create_queued && contract.current_version == 0,
        minted_resolved_bases,
        legacy_identifiers,
    })
}

/// One File Provider content write, staged and described, for [`StateDb::accept_finder_write`].
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
#[derive(Debug)]
pub struct FinderAccept<'a> {
    pub op_id: &'a str,
    pub file_id: &'a str,
    pub kind: FinderAcceptKind<'a>,
    pub parent_id: Option<&'a str>,
    pub target_path: Option<&'a str>,
    pub metadata_json: &'a str,
    pub payload_path: &'a str,
    /// The staged copy's size and modification time (a modify's row takes them).
    pub size_bytes: i64,
    pub modified_at: i64,
    pub backup_source_key: Option<&'a str>,
    pub now: i64,
    /// Task 2190 ("Keep both", review-169 #3): a create that takes a parked write's place. In the
    /// same transaction, the parked op is checked (decidable, its file still at the version shown)
    /// and retired ([`retire_parked_conn`]); if it is not decidable, nothing is written (`Moved`).
    pub retire_parked: Option<RetireParked<'a>>,
}

/// Task 2190: the parked write a create retires, and the version the person was shown.
#[derive(Debug, Clone, Copy)]
pub struct RetireParked<'a> {
    pub op_id: &'a str,
    pub shown_version: i64,
}

#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
#[derive(Debug)]
pub enum FinderAcceptKind<'a> {
    /// A new file: `row` is inserted in the same transaction.
    Create { row: &'a FileEntry },
    /// New bytes for an existing file, on the base identifier the system sent.
    Modify { incoming_base: Option<&'a str> },
}

#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AcceptOutcome {
    /// Queued. `decision` is `None` for a create.
    Queued {
        token: String,
        decision: Option<crate::write_token::BaseDecision>,
    },
    /// Queued and parked at once with its bytes (rule 6a′).
    ParkedAtOnce { token: String, reason: ParkReason },
    /// A modify of a file this database has no row for: nothing was written.
    UnknownItem,
    /// The staged copy's journal row is no longer an unfinished one: an engine-start sweep removed
    /// it with its copy between the copy and this transaction (spec §8.7 S6). Nothing was
    /// written; the write is retried.
    CopyGone,
    /// A sign-out closed Finder accepts after its engine stop (spec 2026-10-10 §15 question 3). Nothing was
    /// written; the write is retried, and macOS keeps the change until it is.
    AcceptsClosed,
    /// Task 2190: the parked write this create was to retire is no longer decidable. Nothing was
    /// written. `released` is never set here.
    ParkedMoved,
    /// Task 2190: queued, and the parked write retired in the same transaction; `released` is its
    /// staged copy, released in the journal, for the caller to unlink.
    QueuedRetiring { token: String, released: Option<String> },
    /// A modify of an item in the trash: its row is `Trashing`, or the system names the trash
    /// container as its parent (spec §7.2). Nothing was written; the save is refused.
    InTrash,
}

/// Why a File Provider upload parked with its bytes kept (spec §11, "Parked").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParkReason {
    StaleBase,
    BaseUnknown,
    PayloadMissing,
    PredecessorParked,
    PredecessorLost,
    RekeyFailed,
    /// The server refused the write with 403: waiting will not cure it, so it parks at once
    /// and presents read-only (§8.4).
    PermissionDenied,
}

impl ParkReason {
    pub fn as_str(self) -> &'static str {
        match self {
            ParkReason::StaleBase => "stale_base",
            ParkReason::BaseUnknown => "base_unknown",
            ParkReason::PayloadMissing => "payload_missing",
            ParkReason::PredecessorParked => "predecessor_parked",
            ParkReason::PredecessorLost => "predecessor_lost",
            ParkReason::RekeyFailed => "rekey_failed",
            ParkReason::PermissionDenied => "permission_denied",
        }
    }
}

/// Everything one landing writes (spec §8.6.2, §8.7 S1.2), for [`StateDb::apply_landing`].
#[derive(Debug)]
pub struct LandingInput<'a> {
    pub op_id: &'a str,
    /// The runner's claim. `None` is Keep Mine's inline run, outside the queue.
    pub claim_id: Option<&'a str>,
    /// A File Provider write's id. Everything the landing does beyond round 3 is keyed on it.
    pub write_id: Option<&'a str>,
    pub local_file_id: &'a str,
    pub server_file_id: &'a str,
    pub target_path: Option<&'a str>,
    pub parent_id: Option<&'a str>,
    /// The base the landed upload was sent with.
    pub landed_base: Option<i64>,
    /// The version and object version id the server's completion produced (§8.6.1).
    pub produced_version: i64,
    pub produced_object_version_id: &'a str,
    pub size_bytes: i64,
    pub content_type: Option<&'a str>,
    /// The server's `mime_type`: the fallback for `content_type`.
    pub mime_type: Option<&'a str>,
    /// The op's staged payload, marked `completed = 1` in the release journal on every platform.
    pub completed_payload: Option<&'a str>,
    /// The SHA-256 of that payload's plaintext, for a sync-folder upload (no write id): what this
    /// device now agrees with the server on (task 1640). `None` leaves the file's baseline unknown.
    pub completed_payload_hash: Option<&'a str>,
    /// Windows: a create's finalization journal row, written in the landing's own transaction
    /// so the op is never gone without it. `None` everywhere else.
    pub finalization: Option<&'a UploadFinalization>,
    pub now: i64,
}

/// What the landing did to the ops queued after it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LandingOutcome {
    /// Successors parked because they could not be re-keyed (§8.6 rule 3).
    pub parked_successors: Vec<(String, ParkReason)>,
    /// Successors the chain step gave the produced version.
    pub resolved_successors: usize,
}

/// A finalization journal row, with its payload marked completed: the landing writes it in
/// its own transaction (Windows creates).
fn insert_upload_finalization_conn(conn: &Connection, row: &UploadFinalization) -> Result<()> {
    conn.execute(
        "INSERT INTO upload_finalizations(op_id,local_file_id,server_file_id,target_path,payload_path,stamped)
        VALUES(?1,?2,?3,?4,?5,0)",
        params![
            row.op_id,
            row.local_file_id,
            row.server_file_id,
            row.target_path,
            row.payload_path
        ],
    )?;
    conn.execute(
        "INSERT INTO staged_payloads(path,completed) VALUES(?1,1)
        ON CONFLICT(path) DO UPDATE SET completed=1",
        params![row.payload_path],
    )?;
    Ok(())
}

/// The contract of a file the database has no row for: our own new file.
fn default_contract(file_id: &str) -> FileContractState {
    FileContractState {
        file_id: file_id.to_string(),
        namespace: Namespace::MyFiles,
        parent_id: None,
        shared_root_id: None,
        share_id: None,
        owner_email: None,
        permission_bits: PERMISSION_READ | PERMISSION_WRITE | PERMISSION_OWNER,
        item_kind: ItemKind::File,
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
    }
}

/// What a snapshot node's version did to the row (spec §6.3.2, the fill and the raise).
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotVersion {
    Unchanged,
    /// The row was at 0: filled, `version_filled = 1`, and these ops got their base.
    Filled {
        resolved_ops: Vec<String>,
    },
    /// A newer version than the row's (I-3): the token clears through the predicate.
    Raised {
        old: i64,
        new: i64,
    },
    /// The node is mid-upload: the legacy init bumps the version first (FILES:2814-2830).
    SkippedUploading,
}

/// Tasks 1888 + 2226 (spec I4): whole units per reconcile transaction. Measured on a copy of the maintainer's queue
/// (plan Task 1 Step 7, 29 Oct: 2,828 remaps, 60 re-arms; Windows, test build, `parent_id` indexed): the longest
/// transaction took 300 ms at 256 units, 293 ms at 128 and 278 ms at 64. Each round re-plans the whole queue, and that
/// fixed cost is most of a transaction, so smaller batches barely shorten the lock and multiply the re-plans
/// (2.3 s in all at 256, 7.1 s at 64).
pub const RECONCILE_UNITS_PER_TX: usize = 256;

/// `base_pending` is 1 + the successful snapshots that did not report the file (plan Spec
/// issue 3). The op parks when the count would pass 10 (spec §6.3.3).
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
const BASE_PENDING_PARK_AT: i64 = 11;

/// One op the runner may run now. Every later write for this attempt names `claim_id`
/// (spec §8.7 S2–S4).
#[derive(Debug, Clone)]
pub struct ClaimedOp {
    pub op: PendingOperation,
    pub claim_id: String,
    pub write: Option<FinderWrite>,
    /// The claim handed a parked predecessor's role to this op (spec §8.4).
    pub took_over: Option<TookOver>,
}

/// A parked predecessor whose role its direct successor took at the claim (spec §8.4).
/// Its op and resume row are gone and its staged payload is journalled for release;
/// the caller unlinks the payload after the commit (S6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TookOver {
    pub parked_op_id: String,
    pub released_payload: Option<String>,
}

#[derive(Debug, Clone)]
pub enum ClaimOutcome {
    /// The op is gone.
    Gone,
    /// Not an attempt: an earlier content op of the same file is queued and has not parked.
    Wait,
    /// Not an attempt: the hand-over committed, and the successor itself must still wait
    /// (its inherited base is pending, or its inherited predecessor is queued).
    WaitAfterHandOver(TookOver),
    /// Boxed: the op is large and the other outcomes carry nothing.
    Claimed(Box<ClaimedOp>),
    /// Not an attempt: the claim parked the op with its bytes (spec §8.4, S5). A
    /// hand-over committed in the same transaction rides along.
    Parked {
        op_id: String,
        file_id: Option<String>,
        reason: ParkReason,
        took_over: Option<TookOver>,
    },
}

/// What engine start repaired (spec §8.7 S3, S6; §10.2).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EngineStartRepair {
    pub claims_cleared: usize,
    /// Journalled payloads marked released that no op, resume row or Windows
    /// finalization references: the caller unlinks them.
    pub released_payloads: Vec<String>,
    /// A server-known file row is at version 0, so a snapshot was requested to learn
    /// its version (spec §6.3.2). Always `false` off macOS.
    pub resnapshot_requested: bool,
}

/// Uploads and restores of one file run in insertion order (spec §8.5, m-9).
fn is_content_kind(kind: &OperationKind) -> bool {
    matches!(
        kind,
        OperationKind::UploadVersion | OperationKind::UploadFile | OperationKind::RestoreVersion
    )
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum OperationPauseReason {
    Auth,
    Quota,
    Permission,
    Locked,
    /// Lead ruling, F9 review I-1 (spec 2026-10-06 R8, §5.6): paused for `auth` when the server said the vault key this
    /// Mac kept was no longer the account's. The names in these operations were encrypted under that key, so they are
    /// kept and never sent; no resume makes them due again.
    ///
    /// Built only by the macOS-only hold ([`StateDb::hold_operations_paused_for_auth_as_key_replaced`]); the `as_str`
    /// arm does not count as building it, so other platforms would report it never constructed.
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    KeyReplaced,
    /// Task 2267 (fix round 2): a change the ownership proof could not tie to this account's own data (a new item at
    /// the top level, or in a folder that is not proven its own, such as a shared one), found when Beebeeb adopted the
    /// data on this PC. It is never sent on its own: the person uploads it or keeps it on this PC only.
    FoundWhilePaused,
    /// Tasks 1888 + 2226 (spec D4, D8): the reconcile pass could not fold a folder made on this PC into its server twin
    /// safely (two twins, an op acting on it, parents that disagree, a busy twin), or a re-armed upload would make old
    /// content current. It waits; the next pass looks again.
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    NeedsReview,
}

impl OperationPauseReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            OperationPauseReason::Auth => "auth",
            OperationPauseReason::Quota => "quota",
            OperationPauseReason::Permission => "permission",
            OperationPauseReason::Locked => "locked",
            OperationPauseReason::KeyReplaced => "key_replaced",
            OperationPauseReason::FoundWhilePaused => "found_while_paused",
            OperationPauseReason::NeedsReview => crate::held_queue::NEEDS_REVIEW,
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

/// Task 2267 (fix round 2): the `sync_state` keys of "Keep on this PC only" (`found_while_paused.rs`).
#[cfg(any(target_os = "windows", test))]
const KEEP_JOURNAL_KEY: &str = "found_while_paused.keep_journal";
#[cfg(any(target_os = "windows", test))]
const KEPT_DIR_KEY: &str = "found_while_paused.kept_dir";

/// Task 2267 (fix round 2): one `files` row as the ownership proof judges it.
#[cfg(any(target_os = "windows", test))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProofRow {
    pub file_id: String,
    pub parent_id: Option<String>,
    /// Its path under the sync root ('/'-separated). 128 of the 2,979 nested rows made on the maintainer's PC have no
    /// parent link, so a walk up falls back to the row at the path's folder.
    pub path: String,
    /// In the person's own files (`namespace = 'my_files'`), not a folder shared with them.
    pub my_files: bool,
    /// Judged against the server's tree: not made on this PC. A row made here has `current_version` and
    /// `remote_updated_at` both at 0 and a queued create names it (lead review of round 2, P2-1).
    pub server_named: bool,
}

/// Task 2267 (fix round 2): one queued operation, by what it acts on.
#[cfg(any(target_os = "windows", test))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProofOp {
    pub op_id: String,
    pub file_id: Option<String>,
    pub parent_id: Option<String>,
    /// `operation_queue.kind`, as stored.
    pub kind: String,
}

/// Task 2267 (lead review round 3, P2-2): one change held as `found_while_paused`, as the card counts it.
#[cfg(any(target_os = "windows", test))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldChange {
    pub op_id: String,
    /// `operation_queue.kind`, as stored.
    pub kind: String,
    /// It creates its item (`CREATE_OP_SQL`).
    pub creates: bool,
    /// Its own row or its folder is in a folder shared with this account (`namespace != 'my_files'`).
    pub shared: bool,
}

/// Task 2267 (lead review of round 2, P2-1): an op (aliased `o`) that creates its item on the server: a folder's
/// create, or an upload whose metadata says it creates the file (`engine_bridge::is_create_file_operation`).
#[cfg(any(target_os = "windows", test))]
const CREATE_OP_SQL: &str = "(o.kind = 'create_folder'
     OR (o.kind IN ('upload_version', 'upload_file')
         AND COALESCE(CASE WHEN json_valid(o.metadata_json) THEN json_extract(o.metadata_json, '$.operation') END = 'create_file', 0)))";

/// A [`ProofOp`] from `op_id, file_id, parent_id, kind`.
#[cfg(any(target_os = "windows", test))]
fn proof_op_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProofOp> {
    Ok(ProofOp {
        op_id: row.get(0)?,
        file_id: row.get::<_, Option<String>>(1)?.filter(|id| !id.is_empty()),
        parent_id: row.get::<_, Option<String>>(2)?.filter(|id| !id.is_empty()),
        kind: row.get(3)?,
    })
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

/// Owned handle to the local SQLite state database.
///
/// One instance per running daemon process. The inner `Connection` is
/// wrapped in a `Mutex` so `StateDb: Send + Sync` and an `Arc<StateDb>`
/// can be shared with `tokio::spawn`-ed futures (rusqlite's
/// `Connection` is `Send` but `!Sync` on its own). Lock contention is
/// fine for our access pattern: the daemon does one tick every 5
/// seconds and OS-extension callbacks are infrequent.
pub struct StateDb(Mutex<Connection>, std::sync::atomic::AtomicU64);

impl StateDb {
    /// One bounded maintenance pass per 30 seconds; account purge clears this clock.
    pub fn claim_owned_key_maintenance(&self, now: i64) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        Ok(conn.execute("INSERT INTO sync_state (key,value) VALUES ('owned_key_retry_after',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value WHERE CAST(sync_state.value AS INTEGER)<=?2", params![now.saturating_add(30).to_string(),now])? > 0)
    }
    /// L7c: account-scoped, durable format knowledge; contains no key material.
    pub fn has_seen_folder_custody(&self, folder_id: &str) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        Self::has_seen_folder_custody_conn(&conn, folder_id)
    }

    fn has_seen_folder_custody_conn(conn: &Connection, folder_id: &str) -> Result<bool> {
        Ok(conn.query_row("SELECT 1 FROM sync_state WHERE key=?1",
            [format!("shared_folder_custody_seen:{folder_id}")], |_| Ok(())).optional()?.is_some())
    }

    /// Serialize legacy key opening with custody observation. An old inventory
    /// cannot authorize a read after another request durably remembers custody.
    /// The synchronous callback only opens a key in core; it must not use the DB.
    pub fn read_without_folder_custody<T>(&self, folder_id: &str, read: impl FnOnce() -> T) -> Result<Option<T>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        if Self::has_seen_folder_custody_conn(&conn, folder_id)? {
            return Ok(None);
        }
        Ok(Some(read()))
    }

    pub fn remember_folder_custody_at_revision(&self, folder_id: &str, revision: u64) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        if self.1.load(std::sync::atomic::Ordering::Acquire) != revision {
            return Err(rusqlite::Error::InvalidQuery);
        }
        conn.execute("INSERT INTO sync_state (key,value) VALUES (?1,'1') ON CONFLICT(key) DO NOTHING",
            [format!("shared_folder_custody_seen:{folder_id}")])?;
        Ok(())
    }

    pub fn mark_shared_index_dirty(&self, folder_id: &str) -> Result<()> {
        let conn=self.0.lock().expect("state_db mutex poisoned");
        conn.execute("INSERT INTO sync_state (key,value) VALUES (?1,'1') ON CONFLICT(key) DO UPDATE SET value=CAST(CAST(value AS INTEGER)+1 AS TEXT)", [format!("shared_folder_index_dirty:{folder_id}")])?;
        Ok(())
    }
    pub fn dirty_shared_index_roots(&self) -> Result<Vec<String>> {
        let conn=self.0.lock().expect("state_db mutex poisoned");
        let mut stmt=conn.prepare("SELECT substr(key,27) FROM sync_state WHERE key LIKE 'shared_folder_index_dirty:%'")?;
        stmt.query_map([],|r|r.get(0))?.collect()
    }
    pub fn clear_shared_index_dirty(&self, folder_id: &str) -> Result<()> {
        let conn=self.0.lock().expect("state_db mutex poisoned");
        conn.execute("DELETE FROM sync_state WHERE key=?1", [format!("shared_folder_index_dirty:{folder_id}")])?;
        Ok(())
    }
    pub fn shared_index_dirty_revision(&self,id:&str)->Result<Option<String>> {
        let conn=self.0.lock().expect("state_db mutex poisoned");
        conn.query_row("SELECT value FROM sync_state WHERE key=?1",[format!("shared_folder_index_dirty:{id}")],|r|r.get(0)).optional()
    }
    pub fn clear_shared_index_dirty_at_revision(&self,id:&str,revision:Option<&str>)->Result<()> {
        let conn=self.0.lock().expect("state_db mutex poisoned");
        conn.execute("DELETE FROM sync_state WHERE key=?1 AND value=?2",params![format!("shared_folder_index_dirty:{id}"),revision])?;
        Ok(())
    }
    pub fn list_owned_key_records(&self)->Result<Vec<crate::owner_keys::OwnedKeyRecord>> {
        let conn=self.0.lock().expect("state_db mutex poisoned");
        let mut stmt=conn.prepare("SELECT record FROM owned_key_records")?;
        let records=stmt.query_map([],|r|r.get::<_,String>(0))?.collect::<Result<Vec<_>>>()?;
        records.into_iter().map(|s|serde_json::from_str(&s).map_err(|e|rusqlite::Error::FromSqlConversionFailure(0,rusqlite::types::Type::Text,Box::new(e)))).collect()
    }

    /// Generation of this account-bound DB handle. The runner also stops its
    /// old bridge before reset, so old requests cannot join a fresh engine.
    pub fn account_revision(&self) -> u64 {
        let _conn = self.0.lock().expect("state_db mutex poisoned");
        self.1.load(std::sync::atomic::Ordering::Acquire)
    }

    pub fn get_owned_key_record(&self, id: &str) -> Result<Option<crate::owner_keys::OwnedKeyRecord>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let record: Option<String> = conn.query_row("SELECT record FROM owned_key_records WHERE file_id = ?1", [id], |r|r.get(0)).optional()?;
        record.map(|s| serde_json::from_str(&s).map_err(|e| rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e)))).transpose()
    }

    pub fn put_owned_key_record(&self, row: &crate::owner_keys::OwnedKeyRecord) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        Self::put_owned_key_record_conn(&conn, row)
    }

    pub fn put_owned_key_record_at_revision(&self, row: &crate::owner_keys::OwnedKeyRecord, revision: u64) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        if self.1.load(std::sync::atomic::Ordering::Acquire) != revision {
            return Err(rusqlite::Error::InvalidQuery);
        }
        Self::put_owned_key_record_conn(&conn, row)
    }

    fn put_owned_key_record_conn(conn: &Connection, row: &crate::owner_keys::OwnedKeyRecord) -> Result<()> {
        let record = serde_json::to_string(row).map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        conn.execute("INSERT INTO owned_key_records (file_id, record) VALUES (?1, ?2) ON CONFLICT(file_id) DO UPDATE SET record = excluded.record", params![row.file_id, record])?;
        Ok(())
    }

    pub fn delete_owned_key_record(&self, id: &str) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute("DELETE FROM owned_key_records WHERE file_id = ?1", [id])?;
        Ok(())
    }

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
            CREATE TABLE IF NOT EXISTS owned_key_records (
                file_id TEXT PRIMARY KEY,
                record TEXT NOT NULL
            );
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
        // Task 1640 (review N1, F1): what the fetches put on disk (see `record_fetched_version` and
        // docs/design/1640-local-content-identity.md). -1, the default, is "not known": every row from
        // before this column, and every new one until its placeholder is created where nothing was. A
        // new name (self-review #2): a database from a round-2 build has `fetched_version` DEFAULT 0, which
        // no ALTER can change; that column is left unused.
        ensure_column(&conn, "files", "fetched_min_version", "INTEGER NOT NULL DEFAULT -1")?;
        ensure_column(&conn, "files", "fetched_hash", "TEXT")?;
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
        // Spec 2026-10-09 §5.2: the write token. Additive and nullable, no backfill;
        // only the dedicated functions write them.
        ensure_column(&conn, "files", "held_write_id", "TEXT")?;
        ensure_column(&conn, "files", "held_base", "INTEGER")?;
        ensure_column(&conn, "files", "held_version", "INTEGER")?;
        ensure_column(&conn, "files", "held_object_version_id", "TEXT")?;
        ensure_column(&conn, "files", "version_filled", "INTEGER NOT NULL DEFAULT 0")?;
        ensure_column(&conn, "operation_queue", "write_id", "TEXT")?;
        ensure_column(&conn, "operation_queue", "write_origin", "TEXT")?;
        ensure_column(&conn, "operation_queue", "after_write_id", "TEXT")?;
        ensure_column(&conn, "operation_queue", "base_pending", "INTEGER NOT NULL DEFAULT 0")?;
        ensure_column(&conn, "operation_queue", "claim_id", "TEXT")?;
        ensure_column(&conn, "operation_queue", "claimed_at", "INTEGER")?;
        conn.execute_batch(
            "
            CREATE INDEX IF NOT EXISTS idx_files_namespace ON files(namespace);
            CREATE INDEX IF NOT EXISTS idx_files_shared_root ON files(shared_root_id);
            -- Task 2229: `FILE_BY_PATH_SQL`, once per file and folder on every enumeration pass.
            CREATE INDEX IF NOT EXISTS idx_files_path ON files(path);
            -- Task 2243: the tray's recent list reads the newest rows through it, not every row.
            CREATE INDEX IF NOT EXISTS idx_files_modified_at ON files(modified_at);
            -- Tasks 1888 + 2226: the reconcile pass re-points a ghost folder's children by `parent_id`, once per
            -- remapped folder (`REMAP_FILES_SQL`); without this each remap scans every row.
            CREATE INDEX IF NOT EXISTS idx_files_parent_id ON files(parent_id);
            -- Task 2243: `queue_hold_summary`, polled by the tray every few seconds, reads this alone. It starts
            -- with `paused_reason`, so it also serves every lookup the old `idx_operation_queue_paused` did, and
            -- that index is dropped: one index to keep up to date on each queue write, not two.
            CREATE INDEX IF NOT EXISTS idx_operation_queue_hold_summary
                ON operation_queue(paused_reason, created_at, attempts, max_attempts);
            DROP INDEX IF EXISTS idx_operation_queue_paused;
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
            -- Task 1640 (review N9, N13): a file deleted elsewhere while it held changes this
            -- device had not sent, kept as a copy (paths relative to the sync root). The review
            -- feed lists each until it is dismissed; an original that could not be removed when
            -- the copy was made (`original_removed = 0`) is removed by a later tick.
            CREATE TABLE IF NOT EXISTS kept_copies (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                original_file_id TEXT NOT NULL,
                original_path TEXT NOT NULL,
                copy_path TEXT NOT NULL,
                copied_hash TEXT NOT NULL,
                original_removed INTEGER NOT NULL DEFAULT 0,
                kept_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_kept_copies_original_path ON kept_copies(original_path);
            CREATE TABLE IF NOT EXISTS removed_folders (
                path TEXT PRIMARY KEY,
                removed_at INTEGER NOT NULL
            );
            -- Task 2191: a file of unknown content (spec section 4). In S1 only `mark_pending` is
            -- used: a not-in-sync mark that is still to be made (it failed, or the row joined the
            -- guarded set during the session). The other columns are the check's (S2).
            CREATE TABLE IF NOT EXISTS unknown_checks (
                file_id TEXT PRIMARY KEY,
                state TEXT,
                attempts INTEGER NOT NULL DEFAULT 0,
                stops INTEGER NOT NULL DEFAULT 0,
                next_attempt_at INTEGER NOT NULL DEFAULT 0,
                last_error TEXT,
                copied_hash TEXT,
                mark_pending INTEGER NOT NULL DEFAULT 0,
                recorded_hash TEXT,
                recorded_size INTEGER,
                recorded_mtime INTEGER,
                resume_candidate TEXT,
                resume_chunk INTEGER,
                resume_digest TEXT,
                updated_at INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS idx_unknown_checks_mark_pending ON unknown_checks(mark_pending) WHERE mark_pending = 1;
            -- Task 2191 (D12): a placeholder at `path` whose identity names `file_id`, a file of
            -- unknown content that lives at another path (a remote rename or move left it there).
            -- Task 2190 (the lead's condition on E3): a sync-folder edit the scan holds and does not
            -- send, because the version it is based on is not known (`base_unknown`) or the file's
            -- content is unknown (`content_unknown`, hydrated before task 1640). The review feed
            -- shows it, so a held edit never looks synced in the app. Kept OUT of the `files` row
            -- (#167's scan memo keys on that row), and cleared by the triggers below when the row goes,
            -- moves or leaves `local`. `fingerprint` is the file's content fingerprint when held.
            CREATE TABLE IF NOT EXISTS held_edits (
                file_id TEXT PRIMARY KEY,
                reason TEXT NOT NULL,
                fingerprint TEXT NOT NULL,
                detected_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_held_edits_detected ON held_edits(detected_at, file_id);
            CREATE TRIGGER IF NOT EXISTS files_held_edit_follows_delete
            AFTER DELETE ON files
            BEGIN
                DELETE FROM held_edits WHERE file_id = OLD.file_id;
            END;
            CREATE TRIGGER IF NOT EXISTS files_held_edit_follows_change
            AFTER UPDATE OF file_id, path, status ON files
            WHEN NEW.file_id IS NOT OLD.file_id OR NEW.path IS NOT OLD.path
              OR (OLD.status = 'local' AND NEW.status != 'local')
            BEGIN
                DELETE FROM held_edits WHERE file_id = OLD.file_id;
            END;
            CREATE TABLE IF NOT EXISTS unknown_orphans (
                path TEXT PRIMARY KEY,
                file_id TEXT NOT NULL,
                state TEXT,
                attempts INTEGER NOT NULL DEFAULT 0,
                next_attempt_at INTEGER NOT NULL DEFAULT 0,
                mark_pending INTEGER NOT NULL DEFAULT 0,
                copied_hash TEXT,
                updated_at INTEGER NOT NULL DEFAULT 0
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
        ensure_column(&conn, "upload_resume", "completed_version", "INTEGER")?;
        ensure_column(&conn, "upload_resume", "completed_object_version_id", "TEXT")?;
        ensure_column(&conn, "upload_resume", "completed_mime_type", "TEXT")?;
        conn.execute_batch(
            "
            CREATE INDEX IF NOT EXISTS idx_operation_queue_write_id ON operation_queue(write_id);
            CREATE INDEX IF NOT EXISTS idx_operation_queue_after_write_id ON operation_queue(after_write_id);
            -- The one-read presentation looks up a file's queued uploads on every item.
            CREATE INDEX IF NOT EXISTS idx_operation_queue_file_id ON operation_queue(file_id);
            CREATE TABLE IF NOT EXISTS id_aliases (
                provisional_id TEXT PRIMARY KEY,
                server_id TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );
            ",
        )?;
        // Tasks 1888, 2226 (spec D7): `queue_hold_summary`'s `waiting_for_folder` group reads these two alone, so the
        // tray's poll stays inside task 2243's budget. The first holds only the folder creates; the second finds the
        // changes in a folder. Indexes only, no table or column change. macOS has no parent-first gate.
        #[cfg(not(target_os = "macos"))]
        conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_operation_queue_create_folder
                 ON operation_queue(parent_id, file_id, paused_reason, attempts, max_attempts)
                 WHERE kind = 'create_folder';
             CREATE INDEX IF NOT EXISTS idx_operation_queue_parent
                 ON operation_queue(parent_id, paused_reason, attempts, max_attempts, created_at);",
        )?;
        // Task 1640 (review F5): why a file deleted elsewhere was kept (`KeptReason`).
        ensure_column(&conn, "kept_copies", "reason", "TEXT NOT NULL DEFAULT 'copy'")?;
        // Task 1640 (review-151d P2-1): the person dismissed the entry of a file still kept where it
        // is; the row stays as a tombstone, so its folder is still the server's deletion.
        ensure_column(&conn, "kept_copies", "dismissed", "INTEGER NOT NULL DEFAULT 0")?;
        // Task 1640 (review F1, design T6): a file that goes cloud-only may still hold some of its
        // bytes (a placeholder partly on disk reads as cloud-only on every tick, and no dehydration is
        // known to remove them all). Its agreed content no longer describes every byte, so it joins
        // the fetch candidate (`FOLD_AGREED_INTO_CANDIDATE`); nothing is forgotten. A dehydration only
        // removes bytes, so the candidate stays true of what is left. The earlier triggers (round 2's
        // wipe, and the first fold, on `fetched_version`) are dropped; a change to the fold takes a new
        // trigger name, since `IF NOT EXISTS` keeps an existing definition.
        conn.execute_batch(&format!(
            "DROP TRIGGER IF EXISTS files_cloud_only_forgets_local_content;
             DROP TRIGGER IF EXISTS files_cloud_only_folds_agreed_content;
             CREATE TRIGGER IF NOT EXISTS files_cloud_only_folds_agreed_into_candidate
             AFTER UPDATE OF status ON files
             WHEN NEW.status = 'cloud_only' AND OLD.status != 'cloud_only' AND NEW.local_hash IS NOT NULL
             BEGIN
                 UPDATE files SET {FOLD_AGREED_INTO_CANDIDATE} WHERE file_id = NEW.file_id;
             END;"
        ))?;
        // Task 2191 (review-114 R2-P2-3): a row that joins the guarded set during the session (a
        // pull inserting a row whose 0.8.12 placeholder already exists, or a row losing its
        // agreed content) gets its "not in sync" mark at the next tick, not at the next start.
        // The tick's retry reads its place once and clears the flag.
        // Only the Windows guard reads these flags, so other builds carry no triggers (self-review).
        if !cfg!(any(target_os = "windows", test)) {
            conn.execute_batch(
                "DROP TRIGGER IF EXISTS files_unknown_marks_after_insert;
                 DROP TRIGGER IF EXISTS files_unknown_marks_after_insert_any_status;
                 DROP TRIGGER IF EXISTS files_unknown_marks_after_update;",
            )?;
        } else {
        conn.execute_batch(&format!(
            "CREATE TRIGGER IF NOT EXISTS files_unknown_marks_after_insert_any_status
             AFTER INSERT ON files
             WHEN NEW.local_hash IS NULL AND NEW.fetched_min_version < 0
             BEGIN
                 INSERT INTO unknown_checks(file_id, mark_pending, updated_at)
                 VALUES (NEW.file_id, 1, CAST(strftime('%s', 'now') AS INTEGER))
                 ON CONFLICT(file_id) DO UPDATE SET mark_pending = 1;
             END;
             CREATE TRIGGER IF NOT EXISTS files_unknown_marks_after_update
             AFTER UPDATE OF local_hash, fetched_min_version ON files
             WHEN NEW.local_hash IS NULL AND NEW.fetched_min_version < 0
              AND NOT (OLD.local_hash IS NULL AND OLD.fetched_min_version < 0)
             BEGIN
                 INSERT INTO unknown_checks(file_id, mark_pending, updated_at)
                 VALUES (NEW.file_id, 1, CAST(strftime('%s', 'now') AS INTEGER))
                 ON CONFLICT(file_id) DO UPDATE SET mark_pending = 1;
             END;
             CREATE TRIGGER IF NOT EXISTS files_unknown_checks_follow_delete
             AFTER DELETE ON files
             BEGIN
                 DELETE FROM unknown_checks WHERE file_id = OLD.file_id;
             END;
             DROP TRIGGER IF EXISTS files_unknown_marks_after_insert;"
        ))?;
        }
        // Upgrade inventory: old one-shot Keep Mine rows may have no queue
        // owner. Preserve their plaintext reference before any resume purge.
        conn.execute(
            "INSERT OR IGNORE INTO staged_payloads(path, completed) SELECT payload_path, 0 FROM upload_resume",
            [],
        )?;
        Ok(Self(Mutex::new(conn), std::sync::atomic::AtomicU64::new(0)))
    }

    /// Insert or update a row keyed by `file_id`. ON CONFLICT replaces
    /// the entire row except the primary key.
    pub fn upsert_file(&self, e: &FileEntry) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        upsert_file_conn(&conn, e)
    }

    /// Spec 2026-10-09 §7.2 (final review): insert or update a row and write its contract
    /// (parent, namespace, kind and the rest) in ONE transaction. Where a row is listed is
    /// decided by its parent alone, and a new row's columns default to no parent and the
    /// person's own namespace, so no reader may see a new row between the two writes. `update`
    /// gets the row's contract as it is after the upsert (a new row's: the column defaults);
    /// it runs under the database lock and must not call back into the database.
    pub fn upsert_file_with_contract(
        &self,
        entry: &FileEntry,
        update: impl FnOnce(&mut FileContractState),
    ) -> Result<()> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        upsert_file_conn(&tx, entry)?;
        let mut contract =
            get_file_contract_state_conn(&tx, &entry.file_id)?.ok_or(rusqlite::Error::QueryReturnedNoRows)?;
        update(&mut contract);
        set_file_contract_state_conn(&tx, &contract)?;
        tx.commit()
    }

    /// Task 2204 S0.3 (review-155): the pull's write of a server row, as
    /// [`Self::upsert_file_with_contract`], with the Windows-rename hold decided in the same
    /// transaction: while a `local_rename` op of the file is queued, the row keeps its current path
    /// and parent whatever `entry` and `update` say. Returns the path written.
    pub fn upsert_server_file_row(
        &self,
        entry: &FileEntry,
        update: impl FnOnce(&mut FileContractState),
    ) -> Result<String> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        // Only `handle_rename` (Windows) queues these ops, so other builds skip the query.
        let held: Option<(String, Option<String>)> = if cfg!(any(target_os = "windows", test)) {
            tx.query_row(
                "SELECT path, parent_id FROM files
                 WHERE file_id = ?1 AND item_kind = 'file'
                   AND EXISTS(SELECT 1 FROM operation_queue
                     WHERE file_id = ?1 AND kind IN ('rename_file', 'move_file')
                       AND CASE WHEN json_valid(metadata_json)
                           THEN json_extract(metadata_json, '$.local_rename') END = 1)",
                params![entry.file_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
        } else {
            None
        };
        let mut entry = entry.clone();
        if let Some((path, _)) = &held {
            entry.path = path.clone();
        }
        upsert_file_conn(&tx, &entry)?;
        let mut contract =
            get_file_contract_state_conn(&tx, &entry.file_id)?.ok_or(rusqlite::Error::QueryReturnedNoRows)?;
        update(&mut contract);
        if let Some((_, parent_id)) = held {
            contract.parent_id = parent_id;
        }
        set_file_contract_state_conn(&tx, &contract)?;
        tx.commit()?;
        Ok(entry.path)
    }

    /// Task 2191: `file_id` is a file of unknown content, whatever its status (the guarded set).
    pub fn is_unknown_content(&self, file_id: &str) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.query_row(
            &format!("SELECT EXISTS(SELECT 1 FROM files WHERE file_id = ?1 AND {GUARDED_SQL})"),
            params![file_id],
            |r| r.get(0),
        )
    }

    /// Task 2191 (review-157 F7): `file_id` has agreed content recorded (`local_hash`).
    pub fn has_agreed_content(&self, file_id: &str) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM files WHERE file_id = ?1 AND local_hash IS NOT NULL)",
            params![file_id],
            |r| r.get(0),
        )
    }

    /// Task 2191: a file of unknown content that is `local` or `cloud_only` (the population the
    /// check resolves; a conflict or an error is the person's to resolve).
    pub fn is_unknown_population(&self, file_id: &str) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.query_row(
            &format!(
                "SELECT EXISTS(SELECT 1 FROM files WHERE file_id = ?1 AND {GUARDED_SQL}
                   AND status IN ('local', 'cloud_only'))"
            ),
            params![file_id],
            |r| r.get(0),
        )
    }

    /// Task 2191: every file of unknown content (the start-up guard walks them all).
    pub fn list_guarded(&self) -> Result<Vec<GuardedRow>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare(&format!(
            "SELECT file_id, path, status, fetched_min_version FROM files WHERE {GUARDED_SQL} ORDER BY rowid"
        ))?;
        let rows = stmt.query_map([], guarded_row)?;
        rows.collect()
    }

    /// Task 2191: a "not in sync" mark of `file_id` is still to be made (`true`) or done.
    pub fn set_mark_pending(&self, file_id: &str, pending: bool) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        if pending {
            conn.execute(
                "INSERT INTO unknown_checks(file_id, mark_pending, updated_at)
                 VALUES (?1, 1, CAST(strftime('%s', 'now') AS INTEGER))
                 ON CONFLICT(file_id) DO UPDATE SET mark_pending = 1, updated_at = excluded.updated_at",
                params![file_id],
            )?;
        } else {
            // A cleared flag writes only a row that has one (no row per guarded file, self-review).
            conn.execute(
                "UPDATE unknown_checks SET mark_pending = 0 WHERE file_id = ?1 AND mark_pending = 1",
                params![file_id],
            )?;
        }
        Ok(())
    }

    /// Desktop 0.8.15: the files whose "not in sync" mark is pending now. The start-up pass reads
    /// them once, so a row that needs nothing gets no write.
    pub fn mark_pending_ids(&self) -> Result<HashSet<String>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare("SELECT file_id FROM unknown_checks WHERE mark_pending = 1")?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        rows.collect()
    }

    /// Task 2191: [`Self::mark_no_fetched_content`] for many rows, in one transaction (the
    /// start-up guard's empty placeholders). Returns how many were marked.
    pub fn mark_no_fetched_content_many(&self, file_ids: &[String]) -> Result<usize> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction()?;
        let mut marked = 0;
        for file_id in file_ids {
            marked += tx.execute(
                "UPDATE files SET fetched_min_version = 0, fetched_hash = NULL
                 WHERE file_id = ?1 AND fetched_min_version = -1 AND local_hash IS NULL",
                params![file_id],
            )?;
        }
        tx.commit()?;
        Ok(marked)
    }

    /// Task 2191: the guarded rows whose mark is still to be made, at most `limit`, oldest first.
    /// A flag left on a row that is no longer guarded is cleared here.
    pub fn list_mark_pending(&self, limit: usize) -> Result<Vec<GuardedRow>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute(
            &format!(
                "UPDATE unknown_checks SET mark_pending = 0
                 WHERE mark_pending = 1 AND NOT EXISTS (
                     SELECT 1 FROM files AS f WHERE f.file_id = unknown_checks.file_id AND {})",
                GUARDED_SQL
                    .replace("item_kind", "f.item_kind")
                    .replace("local_hash", "f.local_hash")
                    .replace("fetched_min_version", "f.fetched_min_version")
            ),
            [],
        )?;
        let mut stmt = conn.prepare(&format!(
            "SELECT f.file_id, f.path, f.status, f.fetched_min_version
             FROM unknown_checks AS u JOIN files AS f ON f.file_id = u.file_id
             WHERE u.mark_pending = 1 AND {}
             ORDER BY u.updated_at, u.file_id LIMIT ?1",
            GUARDED_SQL
                .replace("item_kind", "f.item_kind")
                .replace("local_hash", "f.local_hash")
                .replace("fetched_min_version", "f.fetched_min_version")
        ))?;
        let rows = stmt.query_map(params![limit as i64], guarded_row)?;
        rows.collect()
    }

    /// Task 2191 (D12): record a placeholder at `path` whose identity is `file_id`, a file of
    /// unknown content now at another path; `mark_pending` until its "not in sync" mark is made.
    pub fn put_unknown_orphan(&self, path: &str, file_id: &str, mark_pending: bool) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute(
            "INSERT INTO unknown_orphans(path, file_id, mark_pending, updated_at)
             VALUES (?1, ?2, ?3, CAST(strftime('%s', 'now') AS INTEGER))
             ON CONFLICT(path) DO UPDATE SET file_id = ?2, mark_pending = ?3, updated_at = excluded.updated_at",
            params![path, file_id, mark_pending],
        )?;
        Ok(())
    }

    /// Task 2191 (D12): every recorded orphan: (path, the identity's file id, mark still pending).
    pub fn list_unknown_orphans(&self) -> Result<Vec<(String, String, bool)>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare("SELECT path, file_id, mark_pending FROM unknown_orphans ORDER BY path")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        rows.collect()
    }

    /// Task 2191 (D12): forget the orphan at `path` (its placeholder is gone, or holds nothing).
    pub fn forget_unknown_orphan(&self, path: &str) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute("DELETE FROM unknown_orphans WHERE path = ?1", params![path])?;
        Ok(())
    }

    /// Fetch a single row by `file_id`. `Ok(None)` if absent.
    pub fn get_file(&self, file_id: &str) -> Result<Option<FileEntry>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        get_file_conn(&conn, file_id)
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

    /// Task 2229: a stamp of the row [`Self::get_file_by_path`] finds at `path`, over every column,
    /// so any write to the row changes it; `None` without a row. Valid within this process only.
    pub fn file_row_stamp_by_path(&self, path: &str) -> Result<Option<u64>> {
        use rusqlite::types::ValueRef;
        use std::hash::{Hash, Hasher};
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare_cached(FILE_ROW_BY_PATH_SQL)?;
        for candidate in [path.to_string(), toggled_slash(path)] {
            let mut rows = stmt.query(params![candidate])?;
            if let Some(row) = rows.next()? {
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                for column in 0..row.as_ref().column_count() {
                    match row.get_ref(column)? {
                        ValueRef::Null => 0u8.hash(&mut hasher),
                        ValueRef::Integer(i) => (1u8, i).hash(&mut hasher),
                        ValueRef::Real(f) => (2u8, f.to_bits()).hash(&mut hasher),
                        ValueRef::Text(t) => (3u8, t).hash(&mut hasher),
                        ValueRef::Blob(b) => (4u8, b).hash(&mut hasher),
                    }
                }
                return Ok(Some(hasher.finish()));
            }
        }
        Ok(None)
    }

    /// Task 2240: sizes this connection's page cache to the DB ([`page_cache_limit_kib`]) and returns the limit in KiB.
    /// Only the Windows scan calls it, on the engine's long-lived connection, at the start of every pass (D1, D3): with
    /// SQLite's default 2,000 KiB, every pass re-read `files` and its path index from disk (~18.5 MB on a 43k-row
    /// vault). The cache is set only when the limit changed. UI commands open their own connections and keep the
    /// default (I2). A commit by another connection still empties this cache; the next pass reads cold once (I4).
    ///
    /// The engine's connection is shared by the sync tick, the transfers and the Cloud Files callbacks too, so the pages
    /// their queries read stay cached as well: this connection's cache can grow to the whole limit (the DB plus an
    /// eighth, at most 128 MiB) and stays there. That is the memory bound D2 accepts.
    pub fn fit_page_cache(&self) -> Result<i64> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let page_count: i64 = conn.query_row("PRAGMA page_count", [], |row| row.get(0))?;
        let page_size: i64 = conn.query_row("PRAGMA page_size", [], |row| row.get(0))?;
        let limit = page_cache_limit_kib(page_count.saturating_mul(page_size));
        let current: i64 = conn.query_row("PRAGMA cache_size", [], |row| row.get(0))?;
        if current != -limit {
            conn.execute_batch(&format!("PRAGMA cache_size = -{limit}"))?;
        }
        Ok(limit)
    }

    fn get_file_by_exact_path(&self, path: &str) -> Result<Option<FileEntry>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare_cached(FILE_BY_PATH_SQL)?;
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

    /// The runner deletes rows only inside a transaction (`delete_file_conn`, the landing);
    /// tests set up states with this.
    #[cfg(test)]
    pub fn delete_file(&self, file_id: &str) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        delete_file_conn(&conn, file_id)
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
            let mut stmt = tx.prepare("SELECT file_id, path, item_kind, local_hash, CASE WHEN fetched_min_version > 0 THEN fetched_hash END FROM files WHERE file_id = ?1")?;
            let mut rows = stmt.query(params![file_id])?;
            if let Some(row) = rows.next()? {
                Some(PrunedRow {
                    file_id: row.get(0)?,
                    path: row.get(1)?,
                    is_dir: ItemKind::from_str(&row.get::<_, String>(2)?) == ItemKind::Folder,
                    local_hash: row.get(3)?,
                    fetched_hash: row.get(4)?,
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
                        "SELECT file_id, path, item_kind, local_hash, CASE WHEN fetched_min_version > 0 THEN fetched_hash END FROM files
                         WHERE namespace = 'my_files'
                           AND substr(ltrim(path, '/'), 1, length(?1) + 1) = ?1 || '/'
                         ORDER BY length(path) DESC, path DESC",
                    )?;
                    let drows = dstmt.query_map(params![root_path], |r| {
                        Ok(PrunedRow {
                            file_id: r.get(0)?,
                            path: r.get(1)?,
                            is_dir: ItemKind::from_str(&r.get::<_, String>(2)?) == ItemKind::Folder,
                            local_hash: r.get(3)?,
                            fetched_hash: r.get(4)?,
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
            let mut stmt = tx.prepare("SELECT file_id, path, item_kind, local_hash, CASE WHEN fetched_min_version > 0 THEN fetched_hash END FROM files WHERE file_id = ?1")?;
            let mut rows = stmt.query(params![file_id])?;
            if let Some(row) = rows.next()? {
                Some(PrunedRow {
                    file_id: row.get(0)?,
                    path: row.get(1)?,
                    is_dir: ItemKind::from_str(&row.get::<_, String>(2)?) == ItemKind::Folder,
                    local_hash: row.get(3)?,
                    fetched_hash: row.get(4)?,
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
                        "SELECT file_id, path, item_kind, local_hash, CASE WHEN fetched_min_version > 0 THEN fetched_hash END FROM files
                         WHERE namespace = 'my_files'
                           AND substr(ltrim(path, '/'), 1, length(?1) + 1) = ?1 || '/'
                         ORDER BY length(path) DESC, path DESC",
                    )?;
                    let drows = dstmt.query_map(params![root_path], |r| {
                        Ok(PrunedRow {
                            file_id: r.get(0)?,
                            path: r.get(1)?,
                            is_dir: ItemKind::from_str(&r.get::<_, String>(2)?) == ItemKind::Folder,
                            local_hash: r.get(3)?,
                            fetched_hash: r.get(4)?,
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

    pub fn delete_file_subtree(&self, file_id: &str) -> Result<Vec<PrunedRow>> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction()?;

        // Look up the row to delete (its path + kind drives the subtree prune).
        let root: Option<PrunedRow> = {
            let mut stmt = tx.prepare("SELECT file_id, path, item_kind, local_hash, CASE WHEN fetched_min_version > 0 THEN fetched_hash END FROM files WHERE file_id = ?1")?;
            let mut rows = stmt.query(params![file_id])?;
            if let Some(row) = rows.next()? {
                Some(PrunedRow {
                    file_id: row.get(0)?,
                    path: row.get(1)?,
                    is_dir: ItemKind::from_str(&row.get::<_, String>(2)?) == ItemKind::Folder,
                    local_hash: row.get(3)?,
                    fetched_hash: row.get(4)?,
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
                        "SELECT file_id, path, item_kind, local_hash, CASE WHEN fetched_min_version > 0 THEN fetched_hash END FROM files
                         WHERE substr(ltrim(path, '/'), 1, length(?1) + 1) = ?1 || '/'
                         ORDER BY length(path) DESC, path DESC",
                    )?;
                    let drows = dstmt.query_map(params![root_path], |r| {
                        Ok(PrunedRow {
                            file_id: r.get(0)?,
                            path: r.get(1)?,
                            is_dir: ItemKind::from_str(&r.get::<_, String>(2)?) == ItemKind::Folder,
                            local_hash: r.get(3)?,
                            fetched_hash: r.get(4)?,
                        })
                    })?;
                    drows.collect::<Result<Vec<_>>>()?
                };
                for d in descendants {
                    // Spec §7.2 (final review): a row with a queued or parked Finder write is
                    // kept; it is re-homed below only if its own folder is gone.
                    if has_finder_write_conn(&tx, &d.file_id)? {
                        continue;
                    }
                    let old_parent = old_parent_of(&tx, &d.file_id);
                    tx.execute("DELETE FROM files WHERE file_id = ?1", params![d.file_id])?;
                    record_file_change_conn(&tx, &d.file_id, FpChangeKind::Deleted, old_parent)?;
                    removed.push(d);
                }
            }
        }

        // Spec §7.2 (final review): the deleted item itself, with a queued or parked Finder write,
        // is kept where it is: the edit wins, and the save re-creates the file under its id.
        if !has_finder_write_conn(&tx, &root.file_id)? {
            let root_old_parent = old_parent_of(&tx, &root.file_id);
            tx.execute("DELETE FROM files WHERE file_id = ?1", params![root.file_id])?;
            record_file_change_conn(&tx, &root.file_id, FpChangeKind::Deleted, root_old_parent)?;
            removed.push(root);
        }
        rehome_orphaned_writes_conn(&tx)?;
        tx.commit()?;
        Ok(removed)
    }

    /// Append a local-only activity event and prune old rows. This table is the
    /// durable recent-activity surface for events whose canonical file row may
    /// legitimately disappear from `files` during sync convergence.
    /// Task 1640 (review N9, N13): record a kept copy; its id.
    pub fn record_kept_copy(&self, copy: &KeptCopyInput<'_>) -> Result<i64> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute(
            "INSERT INTO kept_copies (original_file_id, original_path, copy_path, copied_hash, original_removed, reason, kept_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                copy.original_file_id,
                copy.original_path,
                copy.copy_path,
                copy.copied_hash,
                copy.original_removed,
                copy.reason.as_str(),
                copy.kept_at
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Task 1640 (review F6): kept copy `id` now says `copy` (the retry resolved it): one entry
    /// per file, updated where it is.
    pub fn update_kept_copy(&self, id: i64, copy: &KeptCopyInput<'_>) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute(
            "UPDATE kept_copies SET original_file_id = ?2, original_path = ?3, copy_path = ?4, copied_hash = ?5,
                                    original_removed = ?6, reason = ?7, kept_at = ?8
             WHERE id = ?1",
            params![
                id,
                copy.original_file_id,
                copy.original_path,
                copy.copy_path,
                copy.copied_hash,
                copy.original_removed,
                copy.reason.as_str(),
                copy.kept_at
            ],
        )?;
        Ok(())
    }

    /// Task 1640 (review F6): kept entry `id` is resolved with nothing kept (the file was removed
    /// with no copy): its entry goes.
    pub fn delete_kept_copy(&self, id: i64) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute("DELETE FROM kept_copies WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// Task 1640: every kept copy not dismissed, newest first.
    pub fn list_kept_copies(&self) -> Result<Vec<KeptCopy>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare(&format!(
            "SELECT {KEPT_COPY_COLUMNS} FROM kept_copies WHERE dismissed = 0 ORDER BY kept_at DESC, id DESC"
        ))?;
        let rows = stmt.query_map([], kept_copy_from_row)?;
        rows.collect()
    }

    /// Task 1640 (review F4): the kept entry of `rel_path` whose original still waits to be
    /// removed, if any.
    pub fn pending_kept_original(&self, rel_path: &str) -> Result<Option<KeptCopy>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.query_row(
            &format!(
                "SELECT {KEPT_COPY_COLUMNS} FROM kept_copies WHERE original_path = ?1 AND original_removed = 0 AND dismissed = 0
                 ORDER BY id DESC LIMIT 1"
            ),
            params![rel_path],
            kept_copy_from_row,
        )
        .optional()
    }

    /// Task 1640 (review F3): folder `rel_path` was deleted elsewhere but stays on disk for what
    /// it still holds. It is never created again on the server from this PC.
    pub fn record_removed_folder(&self, rel_path: &str, now: i64) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute(
            "INSERT INTO removed_folders (path, removed_at) VALUES (?1, ?2) ON CONFLICT(path) DO NOTHING",
            params![rel_path, now],
        )?;
        Ok(())
    }

    /// Task 1640 (review F3): every folder deleted elsewhere that is still on disk, deepest first.
    pub fn list_removed_folders(&self) -> Result<Vec<String>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare("SELECT path FROM removed_folders ORDER BY length(path) DESC, path DESC")?;
        let rows = stmt.query_map([], |row| row.get(0))?;
        rows.collect()
    }

    /// Task 1640 (review F3): folder `rel_path` is gone from disk, or the server lists it again.
    pub fn forget_removed_folder(&self, rel_path: &str) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute("DELETE FROM removed_folders WHERE path = ?1", params![rel_path])?;
        Ok(())
    }

    /// Task 1640 (review F3): `rel_path` is a folder deleted elsewhere that stays on disk.
    pub fn is_removed_folder(&self, rel_path: &str) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM removed_folders WHERE path = ?1)",
            params![rel_path],
            |row| row.get(0),
        )
    }

    /// Task 1640 (review F3): the outermost folder deleted elsewhere that is `rel_path` or holds it.
    pub fn outermost_removed_folder(&self, rel_path: &str) -> Result<Option<String>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.query_row(
            "SELECT path FROM removed_folders
             WHERE path = ?1 OR substr(?1, 1, length(path) + 1) = path || '/'
             ORDER BY length(path) LIMIT 1",
            params![rel_path],
            |row| row.get(0),
        )
        .optional()
    }

    /// Task 1640 (review N13): the original of kept copy `id` is gone now.
    pub fn mark_kept_original_removed(&self, id: i64) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute("UPDATE kept_copies SET original_removed = 1 WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// Task 1640 (review N13): `rel_path` is an original whose kept copy is made but which is not
    /// removed yet: the upload driver never sends it.
    pub fn is_kept_original_pending(&self, rel_path: &str) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM kept_copies WHERE original_path = ?1 AND original_removed = 0 AND dismissed = 0)",
            params![rel_path],
            |row| row.get(0),
        )
    }

    /// Task 1640 (review N9, F7): the person dismissed kept copy `id` (its review entry). The copy
    /// itself stays, and so does a file kept where it is. `false`: no such entry, or its original
    /// still waits to be removed by a later tick ([`KeptCopy::dismissable`]).
    pub fn dismiss_kept_copy(&self, id: i64) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let kept = conn
            .query_row(
                &format!("SELECT {KEPT_COPY_COLUMNS} FROM kept_copies WHERE id = ?1 AND dismissed = 0"),
                params![id],
                kept_copy_from_row,
            )
            .optional()?;
        let Some(kept) = kept.filter(KeptCopy::dismissable) else {
            return Ok(false);
        };
        // Review-151d P2-1: a file still kept where it is leaves a tombstone: its folder stays the
        // server's deletion (never created again from this PC while it holds only that file).
        let changed = if kept.original_removed {
            conn.execute("DELETE FROM kept_copies WHERE id = ?1", params![id])?
        } else {
            conn.execute("UPDATE kept_copies SET dismissed = 1 WHERE id = ?1", params![id])?
        };
        Ok(changed == 1)
    }

    /// Task 1640 (review-151d P2-1): `rel_path` is a file kept from a deletion elsewhere whose
    /// original was never removed: one that waits, or one whose entry the person dismissed.
    pub fn is_kept_original(&self, rel_path: &str) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM kept_copies WHERE original_path = ?1 AND original_removed = 0)",
            params![rel_path],
            |row| row.get(0),
        )
    }

    /// Task 1640 (review-151d P2-1): the dismissed entries' tombstones, as `(id, original_path)`.
    pub fn list_dismissed_kept_originals(&self) -> Result<Vec<(i64, String)>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare("SELECT id, original_path FROM kept_copies WHERE dismissed = 1 ORDER BY id")?;
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect()
    }

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

    /// Desktop 0.8.15 (deliverable 3): queued changes held back (`paused_reason` set: the `auth`
    /// pause, a quota, a review). The UI shows them as "N changes on hold", never as syncing; they
    /// are not transfer work ([`Self::transfer_backlog`] leaves them out).
    pub fn count_operations_on_hold(&self) -> Result<u32> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let held: i64 =
            conn.query_row("SELECT COUNT(*) FROM operation_queue WHERE paused_reason IS NOT NULL", [], |row| row.get(0))?;
        Ok(u32::try_from(held.max(0)).unwrap_or(u32::MAX))
    }

    /// Pages this connection has read from the database file since it opened: SQLite's page-cache misses. Task 2243's
    /// tests measure what one poll costs the disk with it.
    #[cfg(test)]
    pub(crate) fn page_reads(&self) -> i64 {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let (mut current, mut highwater) = (0, 0);
        // SAFETY: `conn` is an open connection, held for the call; both out-pointers live on this frame.
        let rc = unsafe {
            rusqlite::ffi::sqlite3_db_status(
                conn.handle(),
                rusqlite::ffi::SQLITE_DBSTATUS_CACHE_MISS,
                &mut current,
                &mut highwater,
                0,
            )
        };
        assert_eq!(rc, rusqlite::ffi::SQLITE_OK, "sqlite3_db_status");
        i64::from(current)
    }

    /// What is waiting to transfer (see [`TransferBacklog`]). `now` is unix seconds: an upload whose
    /// retry time is still ahead of it is waiting out a backoff and does not count.
    pub fn transfer_backlog(&self, now: i64) -> Result<TransferBacklog> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        // D7: an upload waiting for a held folder is not sent until the hold ends, so it is not transfer work; counted,
        // it would keep the engine "syncing" for as long as the hold lasts. One waiting for a folder still on its way
        // is (`DUE_PARENT_GATE_SQL` holds it for that folder's landing only).
        #[cfg(target_os = "macos")]
        let not_held = "";
        #[cfg(not(target_os = "macos"))]
        let not_held = format!(
            "AND (q.parent_id IS NULL
                  OR q.parent_id NOT IN (WITH RECURSIVE {HELD_FOLDER_CTE} SELECT file_id FROM held_folder))"
        );
        let (upload_files, upload_bytes): (i64, i64) = conn.query_row(
            &format!(
                "SELECT COUNT(*), COALESCE(SUM(MAX(COALESCE(f.size_bytes, 0), 0)), 0)
                 FROM operation_queue q LEFT JOIN files f ON f.file_id = q.file_id
                 WHERE q.kind IN ('upload_file', 'upload_version')
                   AND q.next_retry_at <= ?1
                   AND q.paused_reason IS NULL AND q.attempts < q.max_attempts
                   {not_held}"
            ),
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
            let mut stmt =
                tx.prepare("SELECT file_id, path, item_kind, local_hash, CASE WHEN fetched_min_version > 0 THEN fetched_hash END FROM files WHERE parent_id = ?1 AND status != 'trashing'")?;
            let rows = stmt.query_map(params![folder_id], |r| {
                Ok(PrunedRow {
                    file_id: r.get(0)?,
                    path: r.get(1)?,
                    is_dir: ItemKind::from_str(&r.get::<_, String>(2)?) == ItemKind::Folder,
                    local_hash: r.get(3)?,
                    fetched_hash: r.get(4)?,
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
                            "SELECT file_id, path, item_kind, local_hash, CASE WHEN fetched_min_version > 0 THEN fetched_hash END FROM files
                             WHERE substr(ltrim(path, '/'), 1, length(?1) + 1) = ?1 || '/'
                             ORDER BY length(path) DESC, path DESC",
                        )?;
                        let drows = dstmt.query_map(params![child_path], |r| {
                            Ok(PrunedRow {
                                file_id: r.get(0)?,
                                path: r.get(1)?,
                                is_dir: ItemKind::from_str(&r.get::<_, String>(2)?) == ItemKind::Folder,
                                local_hash: r.get(3)?,
                                fetched_hash: r.get(4)?,
                            })
                        })?;
                        drows.collect::<Result<Vec<_>>>()?
                    };
                    for d in descendants {
                        // Spec §7.2 (final review): kept, as in the other sweeps.
                        if has_finder_write_conn(&tx, &d.file_id)? {
                            continue;
                        }
                        tx.execute("DELETE FROM files WHERE file_id = ?1", params![d.file_id])?;
                        removed.push(d);
                    }
                }
            }
        }

        // Delete the direct children and append them last (after their own
        // descendants) to maintain children-before-parent ordering. Spec §7.2 (final
        // review): a child with a queued or parked Finder write is kept, then re-homed at the
        // top level with every write whose folder is gone.
        for child in direct_children {
            if has_finder_write_conn(&tx, &child.file_id)? {
                continue;
            }
            tx.execute("DELETE FROM files WHERE file_id = ?1", params![child.file_id])?;
            removed.push(child);
        }
        rehome_orphaned_writes_conn(&tx)?;

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
    /// survives a restart between ticks. On macOS each call increments a request
    /// counter (spec §6.3.2, I-2(b)); elsewhere it sets a take-once flag.
    pub fn request_resnapshot(&self) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        Self::request_resnapshot_conn(&conn)
    }

    /// [`Self::request_resnapshot`] inside the caller's connection or transaction.
    /// The value counts the requests, so a request made while a snapshot runs is
    /// still pending after that snapshot clears the one it read.
    #[cfg(target_os = "macos")]
    fn request_resnapshot_conn(conn: &Connection) -> Result<()> {
        conn.execute(
            "INSERT INTO sync_state (key, value) VALUES (?1, '1')
             ON CONFLICT(key) DO UPDATE SET value = CAST(CAST(value AS INTEGER) + 1 AS TEXT)",
            params![Self::NEEDS_RESNAPSHOT_KEY],
        )?;
        Ok(())
    }

    /// [`Self::request_resnapshot`] inside the caller's connection or transaction.
    #[cfg(not(target_os = "macos"))]
    fn request_resnapshot_conn(conn: &Connection) -> Result<()> {
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
    /// concurrent `request_resnapshot`. Not on macOS: there the request is read with
    /// [`Self::peek_resnapshot_request`] and cleared only after the snapshot succeeded.
    #[cfg(not(target_os = "macos"))]
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

    /// The pending request's counter, or `None`. Read at the start of a tick; the
    /// request is cleared only after the snapshot succeeded (§6.3.2, I-2(b)).
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    pub fn peek_resnapshot_request(&self) -> Result<Option<i64>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.query_row(
            "SELECT CAST(value AS INTEGER) FROM sync_state WHERE key = ?1",
            params![Self::NEEDS_RESNAPSHOT_KEY],
            |row| row.get(0),
        )
        .optional()
    }

    /// Clear the request the tick read; a request made meanwhile survives.
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    pub fn clear_resnapshot_request(&self, seen: i64) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let n = conn.execute(
            "DELETE FROM sync_state WHERE key = ?1 AND CAST(value AS INTEGER) = ?2",
            params![Self::NEEDS_RESNAPSHOT_KEY, seen],
        )?;
        Ok(n == 1)
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
    ///    the landing, `apply_landing`, which stamps `remote_updated_at = now`) AT OR
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
                "SELECT file_id, path, item_kind, local_hash, CASE WHEN fetched_min_version > 0 THEN fetched_hash END FROM files
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
                    local_hash: row.get(3)?,
                    fetched_hash: row.get(4)?,
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
                        // any stale queued op against it is moot. Spec §7.2 (final
                        // review): except a queued or parked Finder write, whose bytes
                        // are the person's: its row is kept, and re-homed at the top
                        // level after the deletes when its own folder is gone. Task 1698:
                        // `Trashing` descendants are NOT excluded here by a pending
                        // op, but ARE excluded by STATUS — they are the server
                        // trash's content (recoverable), not dead rows; deleting
                        // them would erase the trash view's folder contents.
                        let mut dstmt = tx.prepare(
                            "SELECT file_id, path, item_kind, local_hash, CASE WHEN fetched_min_version > 0 THEN fetched_hash END FROM files
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
                                local_hash: r.get(3)?,
                                fetched_hash: r.get(4)?,
                            })
                        })?;
                        drows.collect::<Result<Vec<_>>>()?
                    };
                    for d in descendants {
                        // Spec §7.2 (final review): kept; re-homed below only if its own
                        // folder is gone.
                        if has_finder_write_conn(&tx, &d.file_id)? {
                            continue;
                        }
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
        rehome_orphaned_writes_conn(&tx)?;
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
            .query_row("SELECT status FROM files WHERE file_id = ?1", params![file_id], |row| {
                row.get(0)
            })
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

    /// [`Self::set_status`], only while the row's status is `expected`: a writer that set the
    /// status itself (a hydrate's `Downloading`) never writes over a status another writer set
    /// meanwhile (spec §9.2). One statement. A flip is recorded for the system. Returns whether
    /// the row changed.
    pub fn set_status_if(&self, file_id: &str, expected: FileStatus, status: FileStatus) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let changed = conn.execute(
            "UPDATE files SET status = ?3 WHERE file_id = ?1 AND status = ?2",
            params![file_id, expected.as_str(), status.as_str()],
        )? == 1;
        if changed && expected != status {
            record_file_change_conn(&conn, file_id, FpChangeKind::Modified, None)?;
        }
        Ok(changed)
    }

    /// §9.1 (m-8): a Finder write's attempt marks its row `Uploading`, except a `Trashing` row,
    /// which keeps its trash presentation. One statement, so a trash applied meanwhile is never
    /// overwritten. A flip is recorded for the system, as [`Self::set_status`] does.
    pub fn set_uploading_unless_trashing(&self, file_id: &str) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let flipped = conn.execute(
            "UPDATE files SET status = 'uploading' WHERE file_id = ?1 AND status NOT IN ('uploading', 'trashing')",
            params![file_id],
        )?;
        if flipped == 1 {
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
        record_local_write_conn(&conn, file_id, size_bytes, modified_at)
    }

    /// Task 1640: the content this device last agreed with the server on, for a sync-folder
    /// file: the version its bytes are based on and their SHA-256. Written when a hydrate wrote
    /// the server's bytes; a landing writes it in its own transaction (`apply_landing`).
    pub fn record_local_baseline(&self, file_id: &str, version: i64, hash: &str) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute(
            "UPDATE files SET local_base_version = ?2, local_hash = ?3, fetched_min_version = 0, fetched_hash = NULL
             WHERE file_id = ?1",
            params![file_id, version, hash],
        )?;
        Ok(())
    }

    /// Task 1640 (review N1, F1): a fetch is about to put bytes of `version` (whose whole plaintext
    /// hashes to `hash`) on disk. Recorded BEFORE they are transferred, so a fetch that fails or is
    /// cancelled midway is still counted. The file's candidate: with nothing fetched on disk
    /// (`0`), this fetch's; another of the same version keeps it; one of another version makes it
    /// mixed (the lower version, no hash: the bytes on disk are none of them). Agreed content
    /// joins it first (the bytes are no longer all of that version). Not known (`-1`, `-2`) stays
    /// not known, as `-2`: fetched into since (`mark_no_fetched_content` then leaves it). The
    /// candidate only ever moves down or becomes mixed: only a write of the whole file
    /// (`record_local_baseline`) replaces it.
    #[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
    pub fn record_fetched_version(&self, file_id: &str, version: i64, hash: &str) -> Result<()> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction()?;
        fold_agreed_into_candidate_conn(&tx, file_id)?;
        tx.execute(
            "UPDATE files SET
                 fetched_hash = CASE
                     WHEN fetched_min_version = 0 THEN ?3
                     WHEN fetched_min_version = ?2 AND fetched_hash = ?3 THEN fetched_hash
                     ELSE NULL END,
                 fetched_min_version = CASE
                     WHEN fetched_min_version = 0 THEN ?2
                     WHEN fetched_min_version < 0 THEN -2
                     ELSE MIN(fetched_min_version, ?2) END
             WHERE file_id = ?1",
            params![file_id, version, hash],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Task 1640 (review F1): bytes of a version nobody can name are (or may be) on disk: a
    /// download that named no version. The file's content is not known: never agreed, never sent.
    pub fn mark_content_unknown(&self, file_id: &str) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute(
            "UPDATE files SET local_hash = NULL, fetched_min_version = -2, fetched_hash = NULL WHERE file_id = ?1",
            params![file_id],
        )?;
        Ok(())
    }

    /// Task 1640 (review F1): the file's placeholder was just created where nothing was: no
    /// fetched bytes. Only a row whose content is not known and was never fetched into since
    /// (`-1`; a fetch makes it `-2` before it transfers) is marked, so a fetch on the new
    /// placeholder is never forgotten. `true`: marked.
    #[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
    pub fn mark_no_fetched_content(&self, file_id: &str) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        Ok(conn.execute(
            "UPDATE files SET fetched_min_version = 0, fetched_hash = NULL
             WHERE file_id = ?1 AND fetched_min_version = -1 AND local_hash IS NULL",
            params![file_id],
        )? == 1)
    }

    /// Task 1640 (review N1): the file's candidate, `(version, hash)`; `hash` is `None` when
    /// mixed. `None`: no candidate.
    pub fn fetched_content(&self, file_id: &str) -> Result<Option<(i64, Option<String>)>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        Ok(conn
            .query_row(
                "SELECT fetched_min_version, fetched_hash FROM files WHERE file_id = ?1",
                params![file_id],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?)),
            )
            .optional()?
            .filter(|(version, _)| *version > 0))
    }

    /// Task 1640 (review F1): the placeholder's bytes may have changed in part without a fetch (a
    /// resize, a download cut off by a stop): its agreed content joins the candidate, which is then
    /// mixed: never agreed, a change sent on its version. Its version is kept (never reset).
    pub fn poison_local_content(&self, file_id: &str) -> Result<()> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction()?;
        poison_local_content_conn(&tx, file_id)?;
        tx.commit()?;
        Ok(())
    }

    /// Task 1640 (review F1): the file moved on the server; its bytes did not. Its candidate is
    /// never promoted at the new place (it becomes mixed, its version kept); agreed content stays.
    pub fn poison_fetched_content(&self, file_id: &str) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute("UPDATE files SET fetched_hash = NULL WHERE file_id = ?1", params![file_id])?;
        Ok(())
    }

    /// Task 1640 (review N1, condition 2): make the candidate the agreed content, only while the
    /// row is still `Local` with no agreed content and the candidate is still exactly
    /// `(version, hash)`, all in one statement. `false`: something changed; nothing was written.
    pub fn promote_fetched_content(&self, file_id: &str, version: i64, hash: &str) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        Ok(conn.execute(
            "UPDATE files SET local_base_version = ?2, local_hash = ?3, fetched_min_version = 0, fetched_hash = NULL
             WHERE file_id = ?1 AND status = 'local' AND local_hash IS NULL
               AND fetched_min_version = ?2 AND fetched_hash = ?3",
            params![file_id, version, hash],
        )? == 1)
    }

    /// Task 1640 (review P2-5): the `Local` files the stamp pass may mark in sync: a recorded
    /// agreed content and no sync-folder upload queued. Everything else is never marked, so it is
    /// filtered here, before the pass's per-tick cap.
    pub fn list_stamp_candidates(&self) -> Result<Vec<(String, String)>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT file_id, path FROM files
             WHERE status = 'local' AND item_kind = 'file' AND (local_hash IS NOT NULL OR fetched_hash IS NOT NULL)
               AND file_id NOT IN (SELECT file_id FROM operation_queue
                                   WHERE file_id IS NOT NULL AND write_id IS NULL
                                     AND kind IN ('upload_version', 'upload_file'))
             ORDER BY file_id",
        )?;
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect()
    }

    /// Task 1640: a sync-folder upload of this file's content (no write id: Windows) is
    /// queued, in any state, parked included. Its bytes are not sent until it lands.
    pub fn sync_folder_upload_queued(&self, file_id: &str) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM operation_queue
                            WHERE file_id = ?1 AND write_id IS NULL
                              AND kind IN ('upload_version', 'upload_file'))",
            params![file_id],
            |row| row.get(0),
        )
    }

    /// The full relative path of the item `name` under `parent_id` (spec §7.2; see
    /// `relative_path_under_conn`).
    pub fn relative_path_under(&self, parent_id: Option<&str>, name: &str) -> Result<String> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        relative_path_under_conn(&conn, parent_id, name)
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
    /// - Except a row with a Finder upload that has not parked (spec §9.2): it stays
    ///   `Uploading`, because that upload resumes after the relaunch and the item stays
    ///   writable and not evictable meanwhile. Only a Finder upload carries a write id.
    ///
    /// Returns the ids of the rows it reset: their count is what the engine-start line reports,
    /// and the first tick's working-set signal carries them. A row it keeps `Uploading` is not
    /// touched, so not returned. Each reset changes how the item is presented, so it is recorded
    /// for the system, as every status change is; without the record the system kept showing
    /// the item as it was until the item changed again.
    pub fn reconcile_stale_in_flight_on_startup(&self) -> Result<Vec<String>> {
        const STALE: &str = "status = 'downloading'
                OR (status = 'uploading' AND NOT EXISTS (
                        SELECT 1 FROM operation_queue q
                        WHERE q.file_id = files.file_id AND q.write_id IS NOT NULL
                          AND q.kind IN ('upload_version', 'upload_file') AND q.attempts < q.max_attempts))";
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let reset = {
            let mut stmt = tx.prepare(&format!("SELECT file_id FROM files WHERE {STALE} ORDER BY file_id"))?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            rows.collect::<Result<Vec<_>>>()?
        };
        // Task 1640 (review F1): a download cut off by the stop may have written some bytes.
        let downloading = {
            let mut stmt = tx.prepare("SELECT file_id FROM files WHERE status = 'downloading'")?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            rows.collect::<Result<Vec<_>>>()?
        };
        tx.execute(
            &format!(
                "UPDATE files SET status = CASE status WHEN 'downloading' THEN 'cloud_only' ELSE 'error' END
                 WHERE {STALE}"
            ),
            [],
        )?;
        for file_id in &downloading {
            poison_local_content_conn(&tx, file_id)?;
        }
        for file_id in &reset {
            record_file_change_conn(&tx, file_id, FpChangeKind::Modified, None)?;
        }
        tx.commit()?;
        Ok(reset)
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

    /// How many rows (files and folders) have `status`, counted in `idx_status` alone (task 2243). The windows poll it
    /// every few seconds; loading the rows to count them read the whole set from disk each time.
    pub fn count_by_status(&self, status: FileStatus) -> Result<u32> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let count: i64 = conn
            .prepare_cached("SELECT COUNT(*) FROM files WHERE status = ?1")?
            .query_row(params![status.as_str()], |row| row.get(0))?;
        Ok(u32::try_from(count.max(0)).unwrap_or(u32::MAX))
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
        let mut stmt = conn.prepare(&format!("SELECT {FILE_OVERVIEW_COLUMNS} FROM files ORDER BY path ASC"))?;
        let rows = stmt.query_map([], file_overview_row)?;
        rows.collect()
    }

    /// The newest `limit` files (folders left out), newest first and ties by path, in the row shape of
    /// [`StateDb::file_overview_rows`]: the tray's recent list (task 2243). `compute_file_overview` sorts by the same
    /// keys, so its newest `limit` files are among these. Read through `idx_files_modified_at`, not by loading every row.
    pub fn newest_file_rows(&self, limit: usize) -> Result<Vec<(FileEntry, bool)>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare_cached(&format!(
            "SELECT {FILE_OVERVIEW_COLUMNS} FROM files WHERE item_kind != 'folder'
             ORDER BY modified_at DESC, path ASC LIMIT ?1"
        ))?;
        let rows = stmt.query_map(params![i64::try_from(limit).unwrap_or(i64::MAX)], file_overview_row)?;
        rows.collect()
    }

    /// Tests only (spec §7.2, final review): production writes a row's contract together with
    /// the row, through [`StateDb::upsert_file_with_contract`].
    #[cfg(test)]
    pub fn set_file_contract_state(&self, state: &FileContractState) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        set_file_contract_state_conn(&conn, state)
    }

    pub fn get_file_contract_state(&self, file_id: &str) -> Result<Option<FileContractState>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        get_file_contract_state_conn(&conn, file_id)
    }

    /// The predicate's inputs and the status override's, from one locked read
    /// (spec §5.3 "One read", §8.7 S1.9).
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    pub fn item_presentation(&self, file_id: &str) -> Result<Option<ItemPresentation>> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction()?;
        let Some(contract) = get_file_contract_state_conn(&tx, file_id)? else {
            return Ok(None);
        };
        let (held, version_filled) = held_write_conn(&tx, file_id)?;
        let held_write_queued = match &held {
            Some(held) => tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM operation_queue WHERE write_id = ?1)",
                params![held.write_id],
                |row| row.get(0),
            )?,
            None => false,
        };
        let (unparked_finder_upload, parked_finder_upload): (bool, bool) = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM operation_queue
                            WHERE file_id = ?1 AND write_id IS NOT NULL
                              AND kind IN ('upload_version', 'upload_file')
                              AND attempts < max_attempts),
                    EXISTS(SELECT 1 FROM operation_queue
                            WHERE file_id = ?1 AND write_id IS NOT NULL
                              AND kind IN ('upload_version', 'upload_file')
                              AND attempts >= max_attempts)",
            params![file_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        tx.commit()?;
        Ok(Some(ItemPresentation {
            contract,
            held,
            held_write_queued,
            unparked_finder_upload,
            parked_finder_upload,
            version_filled,
        }))
    }

    /// §5.3: housekeeping only, guarded by the write the caller read, so it never
    /// clears a token a concurrent save has just set. `true`: it cleared the row.
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    pub fn clear_held_if(&self, file_id: &str, write_id: &str) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let n = conn.execute(
            "UPDATE files SET held_write_id = NULL, held_base = NULL, held_version = NULL,
                              held_object_version_id = NULL
             WHERE file_id = ?1 AND held_write_id = ?2",
            params![file_id, write_id],
        )?;
        Ok(n == 1)
    }

    /// §5.4 row 10 (m-9): a restore from this Mac. Its version becomes current, so the
    /// system re-downloads; the held columns are cleared only when no File Provider
    /// write of the file is queued. One transaction.
    ///
    /// The content changed on the server whatever the reply holds: a reply without a
    /// version (no server branch answers so today) still records the change, and asks
    /// a snapshot to fill the version. A reply without an object version (the legacy
    /// branch, which keeps the server's own) keeps the row's.
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    pub fn apply_restore_response(
        &self,
        file_id: &str,
        version: Option<i64>,
        object_version_id: Option<&str>,
    ) -> Result<()> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        match version {
            Some(version) => {
                tx.execute(
                    "UPDATE files SET current_version = ?2, local_base_version = ?2,
                                      current_object_version_id = COALESCE(?3, current_object_version_id),
                                      version_filled = 0
                     WHERE file_id = ?1",
                    params![file_id, version, object_version_id],
                )?;
            }
            None => Self::request_resnapshot_conn(&tx)?,
        }
        record_file_change_conn(&tx, file_id, FpChangeKind::Modified, None)?;
        tx.execute(
            "UPDATE files SET held_write_id = NULL, held_base = NULL, held_version = NULL,
                              held_object_version_id = NULL
             WHERE file_id = ?1
               AND NOT EXISTS (SELECT 1 FROM operation_queue
                               WHERE file_id = ?1 AND write_id IS NOT NULL
                                 AND kind IN ('upload_version', 'upload_file'))",
            params![file_id],
        )?;
        tx.commit()
    }

    /// Task 2190 (plan §3.3, D4): the leaf names of the rows in `parent_id`'s folder (the root when
    /// `None`), lowercased: a Mac compares names case-insensitively. A create that has not landed
    /// has its row already (the accept inserts it), so it counts.
    pub fn child_names_lowercase(&self, parent_id: Option<&str>) -> Result<std::collections::HashSet<String>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare("SELECT path FROM files WHERE parent_id IS ?1")?;
        let names = stmt
            .query_map(params![parent_id], |row| row.get::<_, String>(0))?
            .filter_map(|path| path.ok())
            .map(|path| path.rsplit('/').next().unwrap_or_default().to_lowercase())
            .collect();
        Ok(names)
    }

    /// Task 2190 (the lead's condition on E3): the scan holds `file_id`'s edit and does not send it.
    /// Idempotent: while the reason and the content `fingerprint` are unchanged, nothing is written
    /// (`false`), so a scan pass that finds the same held edit again writes no page. The first
    /// detection's time is kept.
    pub fn hold_unsent_edit(&self, file_id: &str, reason: &str, fingerprint: &str, now: i64) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let written = conn.execute(
            "INSERT INTO held_edits (file_id, reason, fingerprint, detected_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(file_id) DO UPDATE SET reason = excluded.reason, fingerprint = excluded.fingerprint
             WHERE held_edits.reason IS NOT excluded.reason OR held_edits.fingerprint IS NOT excluded.fingerprint",
            params![file_id, reason, fingerprint, now],
        )?;
        Ok(written == 1)
    }

    /// Task 2190: `file_id`'s edit is no longer held (its bytes are agreed, or it is queued, or it can
    /// be sent on a base). `false`: there was none.
    pub fn release_unsent_edit(&self, file_id: &str) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        Ok(conn.execute("DELETE FROM held_edits WHERE file_id = ?1", params![file_id])? == 1)
    }

    /// Task 2190: the held edits whose file still has a row, oldest first: `(file_id, path, reason,
    /// detected_at)`. An index read (`idx_held_edits_detected`, then each row by its key).
    pub fn list_held_edits(&self) -> Result<Vec<(String, String, String, i64)>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare(LIST_HELD_EDITS_SQL)?;
        let rows = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Task 2190 (review-169 #1): decidable, and its file still at `shown`.
    pub fn is_decidable_parked_at(&self, op_id: &str, shown: i64) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        decidable_parked_at_conn(&conn, op_id, shown)
    }

    /// Task 2190 (plan §3.1): `op_id` is a parked op the person may decide ([`DECIDABLE_PARKED_SQL`]).
    pub fn is_decidable_parked(&self, op_id: &str) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        decidable_parked_conn(&conn, op_id)
    }

    /// Task 2190 (plan §3.2, "Keep this Mac's"): in one transaction, the parked op is rebased on the
    /// version the person was shown (`shown`, still the file's: review-169 #1) and made due now (attempts back to 0, its error
    /// cleared), and the file presents `uploading` again. The runner then sends it through the
    /// normal path and landing. Its stale session row goes (no completion is recorded, by the
    /// predicate). `None`: the op is no longer decidable, and nothing changed.
    pub fn rebase_parked(&self, op_id: &str, shown: i64, now: i64) -> Result<Option<i64>> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if !decidable_parked_at_conn(&tx, op_id, shown)? {
            return Ok(None);
        }
        let (file_id, version, object_version_id): (String, i64, Option<String>) = tx.query_row(
            "SELECT file.file_id, file.current_version, file.current_object_version_id
             FROM operation_queue AS op JOIN files AS file ON file.file_id = op.file_id WHERE op.op_id = ?1",
            params![op_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        tx.execute(
            "UPDATE operation_queue
             SET base_version = ?2, base_object_version_id = ?3, attempts = 0, last_error = NULL,
                 last_error_class = NULL, next_retry_at = ?4, updated_at = ?4
             WHERE op_id = ?1",
            params![op_id, version, object_version_id, now],
        )?;
        tx.execute("DELETE FROM upload_resume WHERE op_id = ?1", params![op_id])?;
        let flipped = tx.execute(
            "UPDATE files SET status = 'uploading' WHERE file_id = ?1 AND status = 'error'",
            params![file_id],
        )?;
        if flipped == 1 {
            record_file_change_conn(&tx, &file_id, FpChangeKind::Modified, None)?;
        }
        tx.commit()?;
        Ok(Some(version))
    }

    /// Task 2190 (plan §3.2, "Keep the other"): in one transaction, if the op is still decidable at
    /// `shown`, [`retire_parked_conn`]: the op goes, the system reads the server's version, and the
    /// staged copy is released in the journal. Returns that copy for the caller to unlink after the
    /// commit; the caller has already kept the bytes (D1). `None`: not decidable, nothing changed.
    pub fn drop_parked(&self, op_id: &str, shown: i64) -> Result<Option<Option<String>>> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if !decidable_parked_at_conn(&tx, op_id, shown)? {
            return Ok(None);
        }
        let released = retire_parked_conn(&tx, op_id)?;
        tx.commit()?;
        Ok(Some(released))
    }

    /// §6.3.2: a snapshot node's version, applied to the row in one transaction (S1.5).
    /// The fill (the row is at 0) and the raise (the node is newer) set `current_version`
    /// in any status but `Trashing` and `Conflict`; a node still uploading is skipped and
    /// the snapshot request stays. Nothing else of the row changes here.
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    pub fn apply_snapshot_version(
        &self,
        file_id: &str,
        node_version: i64,
        node_size: i64,
        node_is_uploading: bool,
    ) -> Result<SnapshotVersion> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let row: Option<(String, i64)> = tx
            .query_row(
                "SELECT status, current_version FROM files WHERE file_id = ?1",
                params![file_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((status, current)) = row else {
            return Ok(SnapshotVersion::Unchanged);
        };
        // A Conflict row owns its transitions; a Trashing row is leaving.
        if status == FileStatus::Trashing.as_str() || status == FileStatus::Conflict.as_str() {
            return Ok(SnapshotVersion::Unchanged);
        }
        let fill = current == 0 && node_version > 0;
        let raise = current > 0 && node_version > current;
        if !fill && !raise {
            return Ok(SnapshotVersion::Unchanged);
        }
        if node_is_uploading {
            // The clear at the end of this tick then matches no row: the request stays.
            Self::request_resnapshot_conn(&tx)?;
            tx.commit()?;
            return Ok(SnapshotVersion::SkippedUploading);
        }
        let write_queued: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM operation_queue WHERE file_id = ?1 AND write_id IS NOT NULL
                            AND kind IN ('upload_version', 'upload_file'))",
            params![file_id],
            |r| r.get(0),
        )?;
        // §6.3.2 sets `current_version` and the size, and writes `local_base_version` in
        // neither branch: it is the stale marker. On a row with a queued write the raise
        // changes `current_version` only: the size is that write's, and the object id and
        // `version_filled` stay.
        if write_queued {
            tx.execute(
                "UPDATE files SET current_version = ?2 WHERE file_id = ?1",
                params![file_id, node_version],
            )?;
        } else {
            tx.execute(
                "UPDATE files SET current_version = ?2, size_bytes = ?3 WHERE file_id = ?1",
                params![file_id, node_version, node_size],
            )?;
            if raise {
                // The snapshot carries no object version id; the old one is no longer current.
                tx.execute(
                    "UPDATE files SET current_object_version_id = NULL, version_filled = 0 WHERE file_id = ?1",
                    params![file_id],
                )?;
            }
        }
        let outcome = if fill {
            tx.execute(
                "UPDATE files SET version_filled = 1 WHERE file_id = ?1",
                params![file_id],
            )?;
            let resolved_ops = {
                let mut stmt = tx.prepare(
                    "SELECT op_id FROM operation_queue WHERE file_id = ?1 AND base_pending > 0 ORDER BY rowid",
                )?;
                let rows = stmt.query_map(params![file_id], |r| r.get::<_, String>(0))?;
                rows.collect::<Result<Vec<_>>>()?
            };
            tx.execute(
                "UPDATE operation_queue SET base_version = ?2, base_pending = 0
                 WHERE file_id = ?1 AND base_pending > 0",
                params![file_id, node_version],
            )?;
            SnapshotVersion::Filled { resolved_ops }
        } else {
            SnapshotVersion::Raised {
                old: current,
                new: node_version,
            }
        };
        record_file_change_conn(&tx, file_id, FpChangeKind::Modified, None)?;
        tx.commit()?;
        Ok(outcome)
    }

    /// An upload waits for a snapshot to learn its base (spec §6.3.3).
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    pub fn has_base_pending_uploads(&self) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM operation_queue WHERE base_pending > 0)",
            [],
            |r| r.get(0),
        )
    }

    /// After a successful snapshot: one more miss for every op still waiting for its base;
    /// at the 10th it parks `base_unknown` with its bytes (§6.3.3), through the same park
    /// statement the claim uses. Returns the ops it parked: `(op_id, file_id)`.
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    pub fn note_snapshot_for_base_pending(&self, now: i64) -> Result<Vec<(String, Option<String>)>> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute(
            "UPDATE operation_queue SET base_pending = base_pending + 1 WHERE base_pending > 0",
            [],
        )?;
        let counted_out = {
            let mut stmt =
                tx.prepare("SELECT op_id, file_id FROM operation_queue WHERE base_pending >= ?1 ORDER BY rowid")?;
            let rows = stmt.query_map(params![BASE_PENDING_PARK_AT], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
            })?;
            rows.collect::<Result<Vec<_>>>()?
        };
        let mut parked = Vec::new();
        for (op_id, file_id) in counted_out {
            // No longer waiting, so no pass asks a snapshot for it any more.
            tx.execute(
                "UPDATE operation_queue SET base_pending = 0 WHERE op_id = ?1",
                params![op_id],
            )?;
            // A recorded completion is never parked (§8.6 rule 6).
            if park_unclaimed_conn(&tx, &op_id, ParkReason::BaseUnknown, now)? == 1 {
                if let Some(file_id) = file_id.as_deref() {
                    settle_status_after_park_conn(&tx, file_id)?;
                }
                parked.push((op_id, file_id));
            }
        }
        tx.commit()?;
        Ok(parked)
    }

    /// A content op, a landing or a restore touched the row: its version no longer came
    /// from a snapshot fill (spec §6.1, `version_filled`).
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    pub fn clear_version_filled(&self, file_id: &str) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute(
            "UPDATE files SET version_filled = 0 WHERE file_id = ?1",
            params![file_id],
        )?;
        Ok(())
    }

    /// The round-4 columns of one File Provider upload op; `None` when the op
    /// is gone or carries no write id. Production reads them inside its transactions
    /// (`finder_write_conn`).
    #[cfg(test)]
    pub fn finder_write(&self, op_id: &str) -> Result<Option<FinderWrite>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        finder_write_conn(&conn, op_id)
    }

    /// S1.1: accept one File Provider content write in ONE transaction. The §6.1
    /// decision is read from the row as it is inside it; the write id is minted and
    /// held; a modify's row takes the staged copy's size and time, and the name and parent
    /// the save was made under (§7.2); and the op is queued
    /// (or parked at once, rule 6a′). Nothing inside awaits or touches the network.
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    pub fn accept_finder_write(
        &self,
        accept: &FinderAccept<'_>,
        legacy_identifiers: &dyn Fn(&FileEntry, &FileContractState) -> Vec<String>,
    ) -> Result<AcceptOutcome> {
        use crate::write_token::{BaseDecision, WriteToken, decide_base, mint_write_id};
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        // Spec 2026-10-10 §15 question 3: a sign-out's keep step closed accepts after its engine stop. Read in this
        // transaction, so an accept that outlived the stop either committed before the close or queues nothing.
        if finder_accepts_closed_conn(&tx)? {
            return Ok(AcceptOutcome::AcceptsClosed);
        }
        // §8.7 S6: the copy must still be journalled as unfinished. An engine-start sweep deletes
        // such a row, guarded, before it unlinks the copy, so either this transaction comes first
        // and the op then names the copy (the sweep keeps it), or the row is gone and nothing is
        // written here.
        let journalled: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM staged_payloads WHERE path = ?1 AND completed = 0)",
            params![accept.payload_path],
            |row| row.get(0),
        )?;
        if !journalled {
            return Ok(AcceptOutcome::CopyGone);
        }
        // Task 2190 (review-169 #3): a create that takes a parked write's place is written only while
        // that write is still decidable at the version shown, in this transaction.
        if let Some(retire) = accept.retire_parked
            && !decidable_parked_at_conn(&tx, retire.op_id, retire.shown_version)?
        {
            return Ok(AcceptOutcome::ParkedMoved);
        }
        // The parent the op carries: the template's (§7.2).
        let parent_id = accept.parent_id.map(str::to_string);
        // A modify's record, for the upload's metadata step, of whether it moved the file.
        let mut parent_changed: Option<bool> = None;
        let (decision, object_version_id) = match &accept.kind {
            FinderAcceptKind::Create { row } => {
                debug_assert_eq!(row.file_id, accept.file_id);
                // §7.2 (final review): the row is placed where the system created the item:
                // under the template's parent, with the full relative path read here. The
                // parent is the contract's column, which `upsert_file_conn` does not write; it
                // is set after the insert, recorded as a move, as a Finder folder create does.
                let path = relative_path_under_conn(&tx, accept.parent_id, &row.path)?;
                upsert_file_conn(&tx, &FileEntry { path, ..(*row).clone() })?;
                if accept.parent_id.is_some() {
                    tx.execute(
                        "UPDATE files SET parent_id = ?2 WHERE file_id = ?1",
                        params![accept.file_id, accept.parent_id],
                    )?;
                    record_file_change_conn(&tx, accept.file_id, FpChangeKind::Reparented, None)?;
                }
                (None, None)
            }
            FinderAcceptKind::Modify { incoming_base } => {
                let (Some(entry), Some(contract)) = (
                    get_file_conn(&tx, accept.file_id)?,
                    get_file_contract_state_conn(&tx, accept.file_id)?,
                ) else {
                    return Ok(AcceptOutcome::UnknownItem);
                };
                // §7.2 (final review): an item in the trash takes no save, as a create in the
                // trash is refused: the system holds it there, and a reply would describe it
                // somewhere else. Refused before anything is written, in this transaction.
                if entry.status == FileStatus::Trashing || parent_id.as_deref().is_some_and(is_file_provider_trash) {
                    return Ok(AcceptOutcome::InTrash);
                }
                let facts = base_facts_conn(&tx, accept.file_id, &contract, legacy_identifiers(&entry, &contract))?;
                let decision = decide_base(&facts, *incoming_base);
                record_local_write_conn(&tx, accept.file_id, accept.size_bytes, accept.modified_at)?;
                let moved = entry.parent_id != parent_id;
                apply_save_name_and_parent_conn(&tx, &entry, accept.target_path, parent_id.as_deref())?;
                // §7.2 (final review): the upload's metadata step sends a parent only for a save
                // that moved the file, or that follows a queued save of the file that did: that
                // one may park before its own metadata step, and its move must still reach the
                // server.
                parent_changed = Some(moved || queued_save_moved_conn(&tx, accept.file_id)?);
                // A content write touched the row: its version is no longer a snapshot fill.
                tx.execute(
                    "UPDATE files SET version_filled = 0 WHERE file_id = ?1",
                    params![accept.file_id],
                )?;
                (Some(decision), contract.current_object_version_id.clone())
            }
        };
        let write_id = mint_write_id();
        let b = decision.as_ref().map_or(0, BaseDecision::token_base);
        set_held_write_conn(&tx, accept.file_id, &write_id, b)?;
        // m-4a: a waiting op stores its token's `b`, so an older build sends a base the
        // server refuses, never none. This code never sends it (the claim skips the op).
        let (base_version, after_write_id, base_pending, parked) = match &decision {
            None => (None, None, 0_i64, false),
            Some(BaseDecision::After { write_id, b }) => (Some(*b), Some(write_id.clone()), 0, false),
            Some(BaseDecision::Resolved { base }) => (Some(*base), None, 0, false),
            Some(BaseDecision::Pending) => (Some(0), None, 1, false),
            Some(BaseDecision::ParkUnknown) => (Some(0), None, 0, true),
        };
        const MAX_ATTEMPTS: i64 = 25;
        let park = ParkReason::BaseUnknown.as_str();
        let metadata_json = match parent_changed {
            None => accept.metadata_json.to_string(),
            Some(changed) => {
                let mut metadata: serde_json::Value = serde_json::from_str(accept.metadata_json)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(e.into()))?;
                metadata["parent_changed"] = serde_json::json!(changed);
                metadata.to_string()
            }
        };
        tx.execute(
            "INSERT INTO operation_queue (
                op_id, kind, file_id, parent_id, target_path, metadata_json, payload_path,
                base_version, base_object_version_id, attempts, max_attempts, next_retry_at,
                last_error, last_error_class, paused_reason, backup_source_key, created_at, updated_at,
                write_id, write_origin, after_write_id, base_pending
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, NULL, ?15,
                       ?12, ?12, ?16, ?17, ?18, ?19)",
            params![
                accept.op_id,
                OperationKind::UploadVersion.as_str(),
                accept.file_id,
                parent_id,
                accept.target_path,
                metadata_json,
                accept.payload_path,
                base_version,
                object_version_id,
                if parked { MAX_ATTEMPTS } else { 0 },
                MAX_ATTEMPTS,
                accept.now,
                if parked {
                    park
                } else {
                    "queued from Finder; upload worker not yet attached"
                },
                parked.then_some(park),
                accept.backup_source_key,
                write_id,
                WriteOrigin::Minted.as_str(),
                after_write_id,
                base_pending,
            ],
        )?;
        if parked {
            settle_status_after_park_conn(&tx, accept.file_id)?;
        }
        let released = match accept.retire_parked {
            Some(retire) => Some(retire_parked_conn(&tx, retire.op_id)?),
            None => None,
        };
        tx.commit()?;
        let token = WriteToken { base: b, write_id }.render();
        if let Some(released) = released {
            return Ok(AcceptOutcome::QueuedRetiring { token, released });
        }
        Ok(if parked {
            AcceptOutcome::ParkedAtOnce {
                token,
                reason: ParkReason::BaseUnknown,
            }
        } else {
            AcceptOutcome::Queued { token, decision }
        })
    }

    /// §8.6.1: the completion the server confirmed, recorded before any local bookkeeping, only
    /// while the claimed op exists (S4). `false`: the op moved since the claim and nothing was
    /// written.
    pub fn record_completion_claimed(
        &self,
        op_id: &str,
        claim_id: &str,
        version: i64,
        object_version_id: &str,
        mime_type: Option<&str>,
    ) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let n = conn.execute(
            "UPDATE upload_resume
             SET completed_version = ?3, completed_object_version_id = ?4, completed_mime_type = ?5
             WHERE op_id = ?1 AND EXISTS (SELECT 1 FROM operation_queue WHERE op_id = ?1 AND claim_id = ?2)",
            params![op_id, claim_id, version, object_version_id, mime_type],
        )?;
        Ok(n == 1)
    }

    /// §8.6.2 and §8.7 S1.2: everything the landing changes, in one transaction: the row and
    /// contract, the held columns, the id swap and the alias, the chain step, the status,
    /// and the removal of the op and its resume row with its payload marked for release.
    /// `rekey` re-encrypts a queued op's metadata for the server id. `Ok(None)`: the op moved
    /// since the claim, and nothing was written.
    pub fn apply_landing(
        &self,
        input: &LandingInput<'_>,
        rekey: &dyn Fn(&PendingOperation, &str) -> anyhow::Result<Option<String>>,
    ) -> Result<Option<LandingOutcome>> {
        #[cfg(test)]
        if let Some(error) = injected_landing_failure() {
            return Err(error);
        }
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let (local, server) = (input.local_file_id, input.server_file_id);
        if let Some(claim_id) = input.claim_id {
            let claimed: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM operation_queue WHERE op_id = ?1 AND claim_id = ?2)",
                params![input.op_id, claim_id],
                |r| r.get(0),
            )?;
            if !claimed {
                return Ok(None);
            }
        }
        // Spec §7.2: a later Finder save of the file, parked or not, applied its own name and
        // parent in its accept, after this one's; the row keeps them. Read before the rekey
        // loop moves the file's ops to the server id: `EARLIER_IN_QUEUE_SQL`, with this op as
        // `earlier`.
        let later_save: bool = input.write_id.is_some()
            && tx.query_row(
                &format!(
                    "SELECT EXISTS(
                        SELECT 1 FROM operation_queue AS earlier
                        JOIN operation_queue AS this
                          ON this.file_id = earlier.file_id AND this.op_id != earlier.op_id
                        WHERE earlier.op_id = ?1
                          AND this.write_id IS NOT NULL
                          AND this.kind IN ('upload_version', 'upload_file')
                          AND ({EARLIER_IN_QUEUE_SQL}))"
                ),
                params![input.op_id],
                |r| r.get(0),
            )?;
        // Task 2204 S0.2: a Windows rename (`handle_rename`) re-keyed this file's row while the
        // op was queued, so the row's path and parent are where the file is now and the op's are
        // its queue-time ones. Read before the rekey loop moves the file's ops to the server id.
        let renamed_since: bool = input.write_id.is_none()
            && tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM operation_queue
                   WHERE file_id IN (?1, ?2) AND op_id != ?3
                     AND kind IN ('rename_file', 'move_file')
                     AND CASE WHEN json_valid(metadata_json) THEN json_extract(metadata_json, '$.local_rename') END = 1)",
                params![local, server, input.op_id],
                |r| r.get(0),
            )?;
        // The row and contract. The row's status, size and mtime are settled below, after
        // the chain step, because that step can park a later write (spec §9.1–§9.2).
        let existing_row = get_file_conn(&tx, local)?;
        let keep_place = renamed_since && existing_row.is_some();
        let mut entry = existing_row.unwrap_or_else(|| FileEntry {
            file_id: server.to_string(),
            path: input.target_path.unwrap_or(server).to_string(),
            status: FileStatus::Local,
            size_bytes: input.size_bytes,
            modified_at: input.now,
            content_hash: None,
            remote_updated_at: input.now,
            parent_id: input.parent_id.map(str::to_string),
            item_kind: ItemKind::File,
        });
        entry.file_id = server.to_string();
        let kept_path = keep_place.then(|| entry.path.clone());
        if let Some(target_path) = input.target_path.filter(|_| !later_save && !keep_place) {
            // §7.2 (final review): a Finder landing, the id swap included, writes the full
            // relative path under its parent, never the op's name alone. Other landings write
            // their op's path as before.
            entry.path = if input.write_id.is_some() {
                relative_path_under_conn(&tx, input.parent_id, target_path)?
            } else {
                target_path.to_string()
            };
        }
        entry.remote_updated_at = input.now;
        upsert_file_conn(&tx, &entry)?;
        let mut contract = get_file_contract_state_conn(&tx, local)?.unwrap_or_else(|| default_contract(server));
        contract.file_id = server.to_string();
        contract.item_kind = ItemKind::File;
        // The metadata's content type, else the server's `mime_type`, as before. A File Provider
        // write that has neither (its only `complete` answer was `already_completed`) keeps the
        // row's: metadata only, but nothing told the row its type changed.
        let landed_type = input.content_type.or(input.mime_type).map(str::to_string);
        contract.content_type = match landed_type {
            None if input.write_id.is_some() => contract.content_type.take(),
            landed_type => landed_type,
        };
        if !later_save && !keep_place {
            contract.parent_id = input.parent_id.map(str::to_string);
        }
        contract.current_version = input.produced_version;
        contract.local_base_version = input.produced_version;
        // Task 1640 (review-151d P3-6): a landing that could not hash what it sent records no agreed
        // content: the previous hash would pair the new version with the old bytes (a revert to them
        // would read as agreed). For a sync-folder upload the content is mixed at that version below.
        contract.local_hash = input.completed_payload_hash.map(str::to_string);
        contract.current_object_version_id = Some(input.produced_object_version_id.to_string());
        contract.last_sync_at = input.now;
        set_file_contract_state_conn(&tx, &contract)?;
        // Task 1640 (review F1): what it sent is the whole file, so nothing fetched is left on disk.
        if input.completed_payload_hash.is_some() {
            tx.execute(
                "UPDATE files SET fetched_min_version = 0, fetched_hash = NULL WHERE file_id = ?1",
                params![server],
            )?;
        } else if input.write_id.is_none() {
            // Review-151d P3-6: not hashed: never agreed; a change is sent on the version that landed.
            tx.execute(
                "UPDATE files SET fetched_min_version = ?2, fetched_hash = NULL WHERE file_id = ?1",
                params![server, input.produced_version],
            )?;
        }
        // A landing: the row's version is the one the server answered, not a snapshot fill (§6.1).
        tx.execute(
            "UPDATE files SET version_filled = 0 WHERE file_id = ?1",
            params![server],
        )?;

        let mut outcome = LandingOutcome {
            parked_successors: Vec::new(),
            resolved_successors: 0,
        };
        if local != server {
            // §5.4 row 14: S takes P's held columns; the alias; P's queued ops move to S.
            tx.execute(
                "UPDATE files SET
                    held_write_id = (SELECT held_write_id FROM files WHERE file_id = ?1),
                    held_base = (SELECT held_base FROM files WHERE file_id = ?1),
                    held_version = (SELECT held_version FROM files WHERE file_id = ?1),
                    held_object_version_id = (SELECT held_object_version_id FROM files WHERE file_id = ?1)
                 WHERE file_id = ?2",
                params![local, server],
            )?;
            if input.write_id.is_some() {
                // Write-keyed (§7.1): a Windows create leaves no alias.
                tx.execute(
                    "INSERT INTO id_aliases (provisional_id, server_id, created_at) VALUES (?1, ?2, ?3)
                     ON CONFLICT(provisional_id) DO UPDATE SET server_id = excluded.server_id",
                    params![local, server, input.now],
                )?;
            }
            let successors = {
                let mut stmt = tx.prepare(&format!(
                    "SELECT {PENDING_OPERATION_COLUMNS} FROM operation_queue WHERE file_id = ?1 AND op_id != ?2 ORDER BY rowid"
                ))?;
                let rows = stmt.query_map(params![local, input.op_id], pending_operation_from_row)?;
                rows.collect::<Result<Vec<_>>>()?
            };
            for op in successors {
                match rekey(&op, server) {
                    Ok(metadata_json) => {
                        tx.execute(
                            "UPDATE operation_queue SET file_id = ?2, metadata_json = ?3, updated_at = ?4 WHERE op_id = ?1",
                            params![op.op_id, server, metadata_json, input.now],
                        )?;
                    }
                    // §8.6 rule 3: a Finder landing's chain step never fails the landing.
                    Err(_) if input.write_id.is_some() => {
                        tx.execute(
                            "UPDATE operation_queue SET file_id = ?2, attempts = max_attempts, last_error = 'rekey_failed',
                                                        last_error_class = 'rekey_failed', updated_at = ?3
                             WHERE op_id = ?1",
                            params![op.op_id, server, input.now],
                        )?;
                        outcome.parked_successors.push((op.op_id, ParkReason::RekeyFailed));
                    }
                    // Windows and watcher landings propagate the rekey error as before: the
                    // transaction rolls back and the op is retried.
                    Err(error) => return Err(rusqlite::Error::ToSqlConversionFailure(error.into())),
                }
            }
            delete_file_conn(&tx, local)?;
        }
        // §9.1–§9.2: read after the rekey loop, which can park a successor.
        // `later_unparked`: other Finder uploads of the file that have not parked;
        // `later_parked`: the rest.
        let later_unparked: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM operation_queue WHERE file_id = ?1 AND op_id != ?2
                            AND write_id IS NOT NULL AND kind IN ('upload_version', 'upload_file')
                            AND attempts < max_attempts)",
            params![server, input.op_id],
            |r| r.get(0),
        )?;
        let later_parked: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM operation_queue WHERE file_id = ?1 AND op_id != ?2
                            AND write_id IS NOT NULL AND kind IN ('upload_version', 'upload_file')
                            AND attempts >= max_attempts)",
            params![server, input.op_id],
            |r| r.get(0),
        )?;
        if !later_unparked && !later_parked {
            // `Local`: no later write. The save-time presentation is split off (spec §12).
            // §9.1 (m-8): a Finder write's landing leaves a `Trashing` row in the trash until its
            // trash op finishes; it never becomes writable or evictable on the way out.
            // Write-keyed (`?5`): other landings keep today's status.
            tx.execute(
                "UPDATE files SET status = CASE WHEN status = 'trashing' AND ?5 THEN status ELSE ?2 END,
                                  size_bytes = ?3, modified_at = ?4
                 WHERE file_id = ?1",
                params![
                    server,
                    FileStatus::Local.as_str(),
                    input.size_bytes,
                    input.now,
                    input.write_id.is_some()
                ],
            )?;
        }
        // Only parked writes remain: a parked upload presents `error` (spec §9.1), settled once
        // this op is gone (below), because until then it counts as a Finder upload that has not
        // parked. Otherwise a later write is still queued and unparked: the status stays
        // `Uploading`, and the size and mtime stay the newer write's.
        let settle_after_removal = later_parked && !later_unparked;
        if let Some(write_id) = input.write_id {
            // §8.6.2: only WHERE held_write_id = W; a later save's token stays held.
            tx.execute(
                "UPDATE files SET held_version = ?3, held_object_version_id = ?4 WHERE file_id = ?1 AND held_write_id = ?2",
                params![server, write_id, input.produced_version, input.produced_object_version_id],
            )?;
            // §8.1: the chain step, by write id, with the version the server produced.
            outcome.resolved_successors = tx.execute(
                "UPDATE operation_queue SET base_version = ?2, base_object_version_id = ?3, after_write_id = NULL
                 WHERE after_write_id = ?1",
                params![write_id, input.produced_version, input.produced_object_version_id],
            )?;
        } else {
            // Windows and watcher uploads keep round 3's equal-base rule (spec §6.3.1). For a
            // create (`?6`) the moved op's own base must be `None`; without `base_version IS NULL`
            // a create's id swap would rebase every non-Finder upload of the file, including
            // ones with a base.
            outcome.resolved_successors = tx.execute(
                "UPDATE operation_queue SET base_version = ?3, base_object_version_id = ?4
                 WHERE file_id = ?1 AND op_id != ?2 AND write_id IS NULL
                   AND kind IN ('upload_version', 'upload_file')
                   AND ((?5 IS NOT NULL AND base_version = ?5) OR (?5 IS NULL AND base_version IS NULL AND ?6))",
                params![
                    server,
                    input.op_id,
                    input.produced_version,
                    input.produced_object_version_id,
                    input.landed_base,
                    local != server
                ],
            )?;
        }
        // The op and its resume row. The payload is marked completed on every platform
        // (Windows keeps it until finalization, and its sign-out reads the flag); the caller
        // unlinks the released copy after the commit (S6).
        let removed = match input.claim_id {
            Some(claim_id) => tx.execute(
                "DELETE FROM operation_queue WHERE op_id = ?1 AND claim_id = ?2",
                params![input.op_id, claim_id],
            )?,
            None => tx.execute("DELETE FROM operation_queue WHERE op_id = ?1", params![input.op_id])?,
        };
        if input.claim_id.is_some() && removed != 1 {
            return Ok(None); // dropping `tx` rolls back
        }
        if settle_after_removal {
            settle_status_after_park_conn(&tx, server)?;
        }
        tx.execute("DELETE FROM upload_resume WHERE op_id = ?1", params![input.op_id])?;
        if let Some(path) = input.completed_payload {
            tx.execute(
                "INSERT INTO staged_payloads(path, completed) VALUES (?1, 1) ON CONFLICT(path) DO UPDATE SET completed = 1",
                params![path],
            )?;
        }
        // Windows: the journal takes over the op's local finalization in the same commit, so a
        // failure here rolls the whole landing back and the op is retried.
        // Task 2204 S0.2 (review-128 F4): a renamed file is finalized where it is now.
        if let Some(finalization) = input.finalization {
            match kept_path {
                Some(path) => insert_upload_finalization_conn(
                    &tx,
                    &UploadFinalization { target_path: path, ..finalization.clone() },
                )?,
                None => insert_upload_finalization_conn(&tx, finalization)?,
            }
        }
        tx.commit()?;
        Ok(Some(outcome))
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
        let tip: i64 = conn.query_row("SELECT COALESCE((SELECT MAX(seq) FROM fp_changes), 0)", [], |row| {
            row.get(0)
        })?;
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
                    kind: FpChangeKind::from_str(&row.get::<_, String>(2)?).ok_or_else(|| {
                        rusqlite::Error::InvalidColumnType(2, "kind".to_string(), rusqlite::types::Type::Text)
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
                    kind: FpChangeKind::from_str(&row.get::<_, String>(2)?).ok_or_else(|| {
                        rusqlite::Error::InvalidColumnType(2, "kind".to_string(), rusqlite::types::Type::Text)
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
            let tip: i64 = conn.query_row("SELECT COALESCE((SELECT MAX(seq) FROM fp_changes), 0)", [], |row| {
                row.get(0)
            })?;
            let reached_tip = changes.last().map(|change| change.seq >= tip).unwrap_or(tip <= since);
            if reached_tip {
                // Up to date AND the batch ends here: the anchor is the log
                // tip — "you are now current as of tip". Only a completely
                // empty log (no anchor ever minted) reports None.
                let anchor = if tip > 0 {
                    Some(anchor_bytes(tip))
                } else {
                    since_anchor.map(|bytes| bytes.to_vec())
                };
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
            // The ops' staged copies are released in this transaction, so the next start's
            // release path unlinks them (spec §8.7 S6). Before, the op went and its plaintext
            // copy stayed until a sign-out. macOS only: the release journal is read only there.
            #[cfg(target_os = "macos")]
            tx.execute(
                "INSERT INTO staged_payloads(path, completed)
                 SELECT payload_path, 1 FROM operation_queue WHERE file_id = ?1 AND payload_path IS NOT NULL
                 ON CONFLICT(path) DO UPDATE SET completed = 1",
                params![file_id],
            )?;
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
        insert_operation_conn(&conn, op)
    }

    /// Task 2204 S0.1: a Windows rename or move of a FILE, queued with its row moved to
    /// `new_path` (and `new_parent_id`) in the same transaction, so no path-keyed decision sees
    /// the old path for it again. `op` carries `metadata.local_rename`, which holds the row
    /// against stale landings and pulls until it is sent.
    pub fn enqueue_local_rename(
        &self,
        op: &PendingOperation,
        file_id: &str,
        new_path: &str,
        new_parent_id: Option<&str>,
    ) -> Result<FinderEnqueue> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if finder_accepts_closed_conn(&tx)? {
            return Ok(FinderEnqueue::AcceptsClosed);
        }
        insert_operation_conn(&tx, op)?;
        tx.execute(
            "UPDATE files SET path = ?2, parent_id = ?3 WHERE file_id = ?1 AND item_kind = 'file'",
            params![file_id, new_path, new_parent_id],
        )?;
        // A queued hydrate writes where the file is now, never over a new file at the old name.
        tx.execute(
            "UPDATE operation_queue SET target_path = ?2
             WHERE file_id = ?1 AND kind = 'hydrate_file' AND target_path IS NOT NULL",
            params![file_id, new_path],
        )?;
        tx.commit()?;
        Ok(FinderEnqueue::Queued)
    }

    /// Task 2204 S0.3: an op `handle_rename` queued for this file is still queued (parked
    /// included): the row's path and parent are the local truth, ahead of the server's.
    pub fn has_queued_local_rename(&self, file_id: &str) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM operation_queue
               WHERE file_id = ?1 AND kind IN ('rename_file', 'move_file')
                 AND CASE WHEN json_valid(metadata_json) THEN json_extract(metadata_json, '$.local_rename') END = 1)",
            params![file_id],
            |r| r.get(0),
        )
    }

    /// Task 2204 S0.4: `path` was vacated by a Windows rename that is still waiting to be sent
    /// (unclaimed, not parked or paused). A file there waits too: a safe save may still fold
    /// back onto the original ([`Self::fold_local_rename`]).
    pub fn is_vacated_by_pending_rename(&self, path: &str) -> Result<bool> {
        let alt = toggled_slash(path);
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.query_row(
            &format!(
                "SELECT EXISTS(SELECT 1 FROM operation_queue WHERE {PENDING_LOCAL_RENAME_SQL}
                   AND CASE WHEN json_valid(metadata_json) THEN json_extract(metadata_json, '$.vacated_path') END IN (?1, ?2))"
            ),
            params![path, alt],
            |r| r.get(0),
        )
    }

    /// The path this file's pending Windows rename vacated, if one is still waiting to be sent.
    pub fn pending_local_rename_vacated(&self, file_id: &str) -> Result<Option<String>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.query_row(
            &format!(
                "SELECT json_extract(metadata_json, '$.vacated_path') FROM operation_queue
                 WHERE {PENDING_LOCAL_RENAME_SQL} AND attempts = 0 AND file_id = ?1 LIMIT 1"
            ),
            params![file_id],
            |r| r.get::<_, Option<String>>(0),
        )
        .optional()
        .map(Option::flatten)
    }

    /// Task 2204 S0.4: a safe save (Excel, a backup rename): the original was renamed away, a
    /// new file took its name, and the renamed one is now deleted. In one transaction, and only
    /// while the rename is still unsent and no other live row is at `vacated` (a manual swap,
    /// review-128 F3), the rename is cancelled and the row goes back to `vacated`, so the new
    /// bytes are an edit of the original. `false`: nothing changed; the delete is a delete.
    pub fn fold_local_rename(&self, file_id: &str, vacated: &str) -> Result<bool> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let op: Option<(String, Option<String>)> = tx
            .query_row(
                &format!(
                    "SELECT op_id, json_extract(metadata_json, '$.vacated_parent_id') FROM operation_queue
                     WHERE {PENDING_LOCAL_RENAME_SQL} AND attempts = 0 AND file_id = ?1
                       AND json_extract(metadata_json, '$.vacated_path') = ?2 LIMIT 1"
                ),
                params![file_id, vacated],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((op_id, vacated_parent_id)) = op else {
            return Ok(false);
        };
        let taken: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM files WHERE path IN (?1, ?2) AND file_id != ?3 AND status != 'trashing')",
            params![vacated, toggled_slash(vacated), file_id],
            |r| r.get(0),
        )?;
        if taken {
            return Ok(false);
        }
        tx.execute("DELETE FROM operation_queue WHERE op_id = ?1", params![op_id])?;
        tx.execute(
            "UPDATE files SET path = ?2, parent_id = ?3 WHERE file_id = ?1",
            params![file_id, vacated, vacated_parent_id],
        )?;
        tx.execute(
            "UPDATE operation_queue SET target_path = ?2
             WHERE file_id = ?1 AND kind = 'hydrate_file' AND target_path IS NOT NULL",
            params![file_id, vacated],
        )?;
        tx.commit()?;
        Ok(true)
    }

    /// Spec 2026-10-10 §5.2 step 6: every change the Finder IPC queues (a rename, a move, a folder create with its
    /// row, a delete, and the watcher's and non-macOS writes) is queued in one `BEGIN IMMEDIATE` transaction that reads
    /// the sign-out's fence first, as `accept_finder_write` does for a save. A closed fence writes nothing.
    /// Engine-internal enqueues (`enqueue_operation`: pins, hydrates, restores) never read it.
    pub fn enqueue_finder_change(
        &self,
        op: &PendingOperation,
        folder: Option<FinderFolderRow<'_>>,
    ) -> Result<FinderEnqueue> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if finder_accepts_closed_conn(&tx)? {
            return Ok(FinderEnqueue::AcceptsClosed);
        }
        if let Some(folder) = folder {
            // `upsert_file_with_contract`'s two steps, in this transaction.
            upsert_file_conn(&tx, folder.entry)?;
            let mut contract = get_file_contract_state_conn(&tx, &folder.entry.file_id)?
                .ok_or(rusqlite::Error::QueryReturnedNoRows)?;
            contract.item_kind = ItemKind::Folder;
            contract.parent_id = folder.parent_id.map(str::to_string);
            set_file_contract_state_conn(&tx, &contract)?;
        }
        insert_operation_conn(&tx, op)?;
        tx.commit()?;
        Ok(FinderEnqueue::Queued)
    }

    pub fn list_due_operations(&self, now: i64) -> Result<Vec<PendingOperation>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare(&format!(
            "SELECT op_id, kind, file_id, parent_id, target_path, metadata_json, payload_path,
                    base_version, base_object_version_id, attempts, max_attempts, next_retry_at,
                    last_error, backup_source_key, created_at, updated_at
             FROM operation_queue
             WHERE next_retry_at <= ?1 AND attempts < max_attempts AND paused_reason IS NULL
             {DUE_PARENT_GATE_SQL}
             {DUE_ORDER_SQL}"
        ))?;
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

    /// A finalization row as the landing journals it, with the op and its resume row gone.
    /// Production writes it inside the landing (`apply_landing`); tests set up states with this.
    #[cfg(test)]
    pub fn put_upload_finalization(&self, pending: &UploadFinalization) -> Result<()> {
        let mut conn = self.0.lock().unwrap();
        let tx = conn.transaction()?;
        insert_upload_finalization_conn(&tx, pending)?;
        tx.execute("DELETE FROM operation_queue WHERE op_id=?1", params![pending.op_id])?;
        tx.execute("DELETE FROM upload_resume WHERE op_id=?1", params![pending.op_id])?;
        tx.commit()
    }
    #[cfg(any(target_os = "windows", test))]
    pub fn upload_finalizations(&self) -> Result<Vec<UploadFinalization>> {
        let conn = self.0.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT op_id,local_file_id,server_file_id,target_path,payload_path,stamped FROM upload_finalizations",
        )?;
        stmt.query_map([], |r| {
            Ok(UploadFinalization {
                op_id: r.get(0)?,
                local_file_id: r.get(1)?,
                server_file_id: r.get(2)?,
                target_path: r.get(3)?,
                payload_path: r.get(4)?,
                stamped: r.get(5)?,
            })
        })?
        .collect()
    }
    #[cfg(any(target_os = "windows", test))]
    pub fn mark_upload_finalization_stamped(&self, op_id: &str) -> Result<()> {
        self.0.lock().unwrap().execute(
            "UPDATE upload_finalizations SET stamped=1 WHERE op_id=?1",
            params![op_id],
        )?;
        Ok(())
    }
    #[cfg(any(target_os = "windows", test))]
    pub fn forget_upload_finalization(&self, op_id: &str) -> Result<()> {
        self.0.lock().unwrap().execute(
            "DELETE FROM upload_finalizations WHERE op_id=?1 AND stamped=1",
            params![op_id],
        )?;
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

    /// Engine start (spec §8.7 S6): the journalled copies of accepts that never committed. Their
    /// row says `completed = 0`, and no op, resume row or finalization names the path.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub fn unfinished_unreferenced_payloads(&self) -> Result<Vec<String>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT path FROM staged_payloads
             WHERE completed = 0
               AND path NOT IN (SELECT payload_path FROM operation_queue WHERE payload_path IS NOT NULL)
               AND path NOT IN (SELECT payload_path FROM upload_resume)
               AND path NOT IN (SELECT payload_path FROM upload_finalizations)
             ORDER BY path",
        )?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        rows.collect()
    }

    /// Engine start (spec §8.7 S6): delete the journal row of an accept that never committed, only
    /// while it still is one: `completed = 0`, and no op, resume row or finalization names the path.
    /// `true` when it was deleted; the caller then unlinks the copy. An accept's transaction
    /// requires this row, so an accept that committed first keeps the row (and its copy), and one
    /// that commits after finds it gone and writes nothing (`AcceptOutcome::CopyGone`).
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    pub fn forget_unfinished_unreferenced(&self, path: &str) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let deleted = conn.execute(
            "DELETE FROM staged_payloads
             WHERE path = ?1 AND completed = 0
               AND path NOT IN (SELECT payload_path FROM operation_queue WHERE payload_path IS NOT NULL)
               AND path NOT IN (SELECT payload_path FROM upload_resume)
               AND path NOT IN (SELECT payload_path FROM upload_finalizations)",
            params![path],
        )?;
        Ok(deleted == 1)
    }

    /// Engine start (spec §8.7 S6): every path a row of `staged_payloads`, `operation_queue`,
    /// `upload_resume` or `upload_finalizations` names, in one read.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub fn staged_copy_names(&self) -> Result<Vec<String>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT path FROM staged_payloads
             UNION SELECT payload_path FROM operation_queue WHERE payload_path IS NOT NULL
             UNION SELECT payload_path FROM upload_resume
             UNION SELECT payload_path FROM upload_finalizations",
        )?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        rows.collect()
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
        let remaining: i64 = tx.query_row(
            "SELECT (SELECT COUNT(*) FROM staged_payloads) + (SELECT COUNT(*) FROM upload_finalizations)",
            [],
            |r| r.get(0),
        )?;
        if remaining != 0 {
            return Err(rusqlite::Error::InvalidQuery);
        }
        // Every table the binding counts as account data (the one list, `ACCOUNT_TABLES`; the checks above found the
        // three it refuses on empty), the device tables, and `sync_state` (the owner record and the sync cursors), so the
        // next sign-in on this PC finds no account data and no owner. The device tables are cleared on purpose, the same
        // as `clear_account_data` does: a sign-out starts this PC's statistics afresh for the next account.
        for table in ACCOUNT_TABLES.iter().chain(DEVICE_TABLES.iter()) {
            tx.execute(&format!("DELETE FROM {table}"), [])?;
        }
        tx.execute("DELETE FROM sync_state", [])?;
        tx.commit()?;
        self.1.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        Ok(())
    }

    /// R10: the account this local data belongs to, if one is recorded.
    pub fn owner(&self) -> Result<Option<crate::account_binding::Identity>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        read_owner(&conn)
    }

    /// R10: record the owner (both fields at once; an unknown field is removed).
    pub fn set_owner(&self, owner: &crate::account_binding::Identity) -> Result<()> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction()?;
        write_owner(&tx, owner)?;
        tx.commit()?;
        self.1.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        Ok(())
    }

    /// Changes waiting to upload, plus the staged copies of them. Anything of either is a trace of an account:
    /// a sign-in checks it before it decides the Mac holds nothing of a previous one.
    pub fn queued_or_staged_count(&self) -> Result<u64> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let rows: i64 = conn.query_row(
            "SELECT (SELECT COUNT(*) FROM operation_queue) + (SELECT COUNT(*) FROM staged_payloads)",
            [],
            |row| row.get(0),
        )?;
        Ok(u64::try_from(rows).unwrap_or(0))
    }

    /// Changes on this computer that have not uploaded: every queued operation, plus every staged copy that no queued
    /// operation points at (a staged copy outlives its queue row, and can be the only unsynced copy of an edit). Each
    /// change is counted once. The number an account switch warns with.
    pub fn pending_changes_count(&self) -> Result<u64> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        // macOS: a released copy (`completed = 1`) belongs to an upload that landed and waits only
        // for its unlink (spec §8.7 S6), so it is not a change that has not uploaded.
        #[cfg(target_os = "macos")]
        let released = "AND completed = 0";
        #[cfg(not(target_os = "macos"))]
        let released = "";
        let rows: i64 = conn.query_row(
            &format!(
                "SELECT (SELECT COUNT(*) FROM operation_queue)
                      + (SELECT COUNT(*) FROM staged_payloads
                         WHERE path NOT IN (SELECT payload_path FROM operation_queue WHERE payload_path IS NOT NULL)
                         {released})"
            ),
            [],
            |row| row.get(0),
        )?;
        Ok(u64::try_from(rows).unwrap_or(0))
    }

    /// Spec 2026-10-10 W2: the queue a sign-out counts, in ONE read (one mutex hold, one transaction), in queue
    /// order. Hydrates and pins are downloads, never counted.
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    pub fn waiting_rows(&self) -> Result<Vec<WaitingRow>> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction()?;
        let rows = {
            let mut stmt = tx.prepare(
                "SELECT q.op_id, q.kind, q.file_id, f.path, q.payload_path, q.write_origin, q.rowid,
                        q.attempts, q.max_attempts, q.paused_reason IS NOT NULL,
                        EXISTS(SELECT 1 FROM upload_resume r
                               WHERE r.op_id = q.op_id AND r.completed_version IS NOT NULL)
                 FROM operation_queue q LEFT JOIN files f ON f.file_id = q.file_id
                 WHERE q.kind NOT IN ('hydrate_file', 'pin_tree')
                 ORDER BY q.rowid",
            )?;
            let mapped = stmt.query_map([], |row| {
                Ok(WaitingRow {
                    op_id: row.get(0)?,
                    kind: OperationKind::from_str(&row.get::<_, String>(1)?),
                    file_id: row.get(2)?,
                    file_path: row.get(3)?,
                    payload_path: row.get(4)?,
                    origin: WriteOrigin::from_db(row.get::<_, Option<String>>(5)?.as_deref()),
                    queue_order: row.get(6)?,
                    attempts: row.get(7)?,
                    max_attempts: row.get(8)?,
                    paused: row.get(9)?,
                    completed: row.get(10)?,
                })
            })?;
            mapped.collect::<Result<Vec<_>>>()?
        };
        tx.commit()?;
        Ok(rows)
    }

    /// The purge guard's ONE read (spec 2026-10-10 W6, §5.2 step 8): one transaction, before any file is touched. Each
    /// op is placed by `sign_out::waiting::op_role`, as the keep step's read places it, so the two cannot disagree on a
    /// kind.
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    pub fn unkept_waiting_changes(&self, kept: &crate::sign_out::kept_set::KeptSet) -> Result<PurgeGuardRead> {
        use crate::sign_out::waiting::{OpRole, op_role};
        let mut read = PurgeGuardRead::default();
        for row in self.waiting_rows()? {
            match op_role(&row.kind) {
                OpRole::Download => {}
                // A recorded completion: the server has these bytes, so nothing waits (spec §2 "Waiting write").
                OpRole::Upload if row.completed => {}
                OpRole::Upload => {
                    if let Some(path) = &row.payload_path {
                        read.waiting_payloads.push(path.clone());
                    }
                    if !kept.keeps(&row.op_id) {
                        read.unkept.push(row.op_id);
                    }
                }
                OpRole::OtherChange => {
                    if !kept.others_acknowledged.contains(&row.op_id) {
                        read.unkept.push(row.op_id);
                    }
                }
            }
        }
        Ok(read)
    }

    /// R10: does anything of an account live here, apart from the owner record itself?
    pub fn has_account_data(&self) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        Ok(account_rows(&conn)? > 0)
    }

    /// Task 2267 (fix round 2): every `files` row as the ownership proof judges it, read once. A row this PC made and
    /// never uploaded has `current_version` and `remote_updated_at` both at 0 AND a queued create names it (lead review
    /// of round 2, P2-1: absence of a server version alone is not evidence). Every other row is judged: a snapshot row
    /// carries the server's version, a delta or a landing stamps the time (`remote_updated_at` alone is not enough: the
    /// My Files ingest leaves it 0 for every snapshot row, the server sends `updated_at` as RFC 3339 text).
    #[cfg(any(target_os = "windows", test))]
    pub fn own_data_proof_rows(&self) -> Result<Vec<ProofRow>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare(&format!(
            "SELECT file_id, parent_id, namespace = 'my_files',
                    NOT (current_version = 0 AND remote_updated_at = 0
                         AND EXISTS (SELECT 1 FROM operation_queue o WHERE o.file_id = files.file_id AND {CREATE_OP_SQL})),
                    path
             FROM files"
        ))?;
        let rows = stmt.query_map([], |row| {
            Ok(ProofRow {
                file_id: row.get(0)?,
                parent_id: row.get::<_, Option<String>>(1)?.filter(|parent| !parent.is_empty()),
                my_files: row.get(2)?,
                server_named: row.get(3)?,
                path: row.get(4)?,
            })
        })?;
        rows.collect()
    }

    /// Task 2267 (fix round 2): every queued operation by what it acts on: its own row and its parent.
    #[cfg(any(target_os = "windows", test))]
    pub fn own_data_proof_ops(&self) -> Result<Vec<ProofOp>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare("SELECT op_id, file_id, parent_id, kind FROM operation_queue")?;
        let rows = stmt.query_map([], proof_op_from_row)?;
        rows.collect()
    }

    /// Task 2267 (fix round 2): record `owner`, and hold for the person's review every queued operation the proof did
    /// not prove (`found_while_paused`), in ONE transaction. Only if the recorded owner is still `expected` and account
    /// data is still here, so a proof never replaces an owner recorded while it ran, nor names an owner of a database a
    /// sign-out emptied meanwhile. An operation queued after the proof read the queue is not in `proven`, so it is held
    /// too, and so is an unproven one paused for any reason (it would lose that pause when queued again). Returns how
    /// many were held, or `None` when nothing was recorded.
    #[cfg(any(target_os = "windows", test))]
    pub fn adopt_owner_holding(
        &self,
        expected: Option<&crate::account_binding::Identity>,
        owner: &crate::account_binding::Identity,
        proven: &std::collections::HashSet<String>,
        now: i64,
    ) -> Result<Option<usize>> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if read_owner(&tx)?.as_ref() != expected || account_rows(&tx)? == 0 {
            return Ok(None);
        }
        write_owner(&tx, owner)?;
        // Lead review of round 2, P3-2: whatever the pause (quota, permission, a locked file …): queued again, an op
        // loses its pause, and an unproven one would then be sent unjudged.
        let candidates: Vec<String> = {
            let mut stmt = tx.prepare(
                "SELECT op_id FROM operation_queue WHERE paused_reason IS NULL OR paused_reason != ?1",
            )?;
            let rows = stmt.query_map(params![OperationPauseReason::FoundWhilePaused.as_str()], |row| row.get::<_, String>(0))?;
            rows.collect::<Result<Vec<String>>>()?
        };
        let mut held = 0;
        for op_id in candidates.iter().filter(|op_id| !proven.contains(*op_id)) {
            held += tx.execute(
                "UPDATE operation_queue SET paused_reason = ?2, updated_at = ?3 WHERE op_id = ?1",
                params![op_id, OperationPauseReason::FoundWhilePaused.as_str(), now],
            )?;
        }
        tx.commit()?;
        Ok(Some(held))
    }

    /// Task 2267 (fix round 2): the queued operations held for the person's review (`found_while_paused`), by what
    /// they act on, oldest first.
    #[cfg(any(target_os = "windows", test))]
    pub fn found_while_paused_ops(&self) -> Result<Vec<ProofOp>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT op_id, file_id, parent_id, kind FROM operation_queue WHERE paused_reason = ?1
             ORDER BY created_at ASC, rowid ASC",
        )?;
        let rows = stmt.query_map(params![OperationPauseReason::FoundWhilePaused.as_str()], proof_op_from_row)?;
        rows.collect()
    }

    /// Task 2267 (fix round 2), "Keep on this PC only": the rows whose upload has begun, so the server may already
    /// hold part of them: a queued upload with a resumable session, or an upload that landed and awaits its
    /// finalization (its operation is gone then; the finalization names the row).
    #[cfg(any(target_os = "windows", test))]
    pub fn files_with_upload_begun(&self) -> Result<std::collections::HashSet<String>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT o.file_id FROM upload_resume r JOIN operation_queue o ON o.op_id = r.op_id WHERE o.file_id IS NOT NULL
             UNION SELECT local_file_id FROM upload_finalizations",
        )?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        rows.collect()
    }

    /// Task 2267 (fix round 2), "Keep on this PC only": the one move in progress (`found_while_paused::Journal`, as
    /// JSON), written before the move and removed in the transaction that finishes it, so a crash in between is
    /// finished or undone at the next start.
    #[cfg(any(target_os = "windows", test))]
    pub fn keep_journal(&self) -> Result<Option<String>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.query_row("SELECT value FROM sync_state WHERE key = ?1", params![KEEP_JOURNAL_KEY], |row| row.get(0))
            .optional()
    }

    #[cfg(any(target_os = "windows", test))]
    pub fn set_keep_journal(&self, journal: &str) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute(
            "INSERT INTO sync_state (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![KEEP_JOURNAL_KEY, journal],
        )?;
        Ok(())
    }

    #[cfg(any(target_os = "windows", test))]
    pub fn clear_keep_journal(&self) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute("DELETE FROM sync_state WHERE key = ?1", params![KEEP_JOURNAL_KEY])?;
        Ok(())
    }

    /// Task 2267 (self-review of round 3, F3): the staged copies the queued operations acting on the item `ids` hold,
    /// read before the keep forgets them, so an only copy is kept first.
    #[cfg(any(target_os = "windows", test))]
    pub fn keep_staged_copies(&self, ids: &[String]) -> Result<Vec<String>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        Ok(keep_ops_conn(&conn, ids)?.1)
    }

    /// Task 2267 (fix round 2), "Keep on this PC only", after the item `ids` was moved to `kept_dir`: in ONE
    /// transaction, forget every queued operation acting on those rows (with its upload records) and the rows, remove
    /// the journal and remember `kept_dir` for "Show in Explorer". Returns the staged copies the removed operations
    /// held (as [`Self::keep_staged_copies`]).
    #[cfg(any(target_os = "windows", test))]
    pub fn finish_keep(&self, ids: &[String], kept_dir: &str) -> Result<Vec<String>> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let (ops, staged) = keep_ops_conn(&tx, ids)?;
        for op_id in &ops {
            tx.execute("DELETE FROM upload_resume WHERE op_id = ?1", params![op_id])?;
            tx.execute("DELETE FROM upload_finalizations WHERE op_id = ?1", params![op_id])?;
            tx.execute("DELETE FROM operation_queue WHERE op_id = ?1", params![op_id])?;
        }
        for id in ids {
            delete_file_conn(&tx, id)?;
        }
        tx.execute("DELETE FROM sync_state WHERE key = ?1", params![KEEP_JOURNAL_KEY])?;
        tx.execute(
            "INSERT INTO sync_state (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![KEPT_DIR_KEY, kept_dir],
        )?;
        tx.commit()?;
        Ok(staged)
    }

    /// Task 2267 (fix round 2): the folder the last "Keep on this PC only" moved items into.
    #[cfg(any(target_os = "windows", test))]
    pub fn last_kept_dir(&self) -> Result<Option<String>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.query_row("SELECT value FROM sync_state WHERE key = ?1", params![KEPT_DIR_KEY], |row| row.get(0))
            .optional()
    }

    /// Task 2267 (lead review round 3, P2-2): every change held as `found_while_paused`, by kind, for the card,
    /// oldest first. A change is in a shared folder when its row, or any folder up the parent links from its row or
    /// from the folder it acts in, is not a My Files row (self-review of round 3, F8: not only the direct parent).
    #[cfg(any(target_os = "windows", test))]
    pub fn found_while_paused_changes(&self) -> Result<Vec<HeldChange>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut rows: std::collections::HashMap<String, (Option<String>, bool)> = std::collections::HashMap::new();
        {
            let mut stmt = conn.prepare("SELECT file_id, NULLIF(parent_id, ''), namespace != 'my_files' FROM files")?;
            for row in stmt.query_map([], |row| Ok((row.get::<_, String>(0)?, (row.get(1)?, row.get(2)?))))? {
                let (id, row) = row?;
                rows.insert(id, row);
            }
        }
        let shared = |anchor: Option<String>| {
            let mut seen = std::collections::HashSet::new();
            let mut id = anchor;
            while let Some(current) = id.filter(|current| seen.insert(current.clone())) {
                match rows.get(&current) {
                    Some((_, true)) => return true,
                    Some((parent, false)) => id = parent.clone(),
                    None => return false,
                }
            }
            false
        };
        let mut stmt = conn.prepare(&format!(
            "SELECT o.op_id, o.kind, {CREATE_OP_SQL}, NULLIF(o.file_id, ''), NULLIF(o.parent_id, '')
             FROM operation_queue o WHERE o.paused_reason = ?1
             ORDER BY o.created_at ASC, o.rowid ASC"
        ))?;
        let changes = stmt.query_map(params![OperationPauseReason::FoundWhilePaused.as_str()], |row| {
            let (file_id, parent_id): (Option<String>, Option<String>) = (row.get(3)?, row.get(4)?);
            Ok(HeldChange {
                op_id: row.get(0)?,
                kind: row.get(1)?,
                creates: row.get(2)?,
                shared: shared(file_id) || shared(parent_id),
            })
        })?;
        changes.collect()
    }

    /// Task 2267, "Upload to my Beebeeb": the changes in `op_ids` still held as `found_while_paused` are due again at
    /// `now`, keeping their attempts. Returns how many.
    #[cfg(any(target_os = "windows", test))]
    pub fn release_found_while_paused(&self, op_ids: &[String], now: i64) -> Result<usize> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let mut released = 0;
        for op_id in op_ids {
            released += tx.execute(
                "UPDATE operation_queue SET paused_reason = NULL, next_retry_at = ?1, updated_at = ?1
                 WHERE op_id = ?2 AND paused_reason = ?3",
                params![now, op_id, OperationPauseReason::FoundWhilePaused.as_str()],
            )?;
        }
        tx.commit()?;
        Ok(released)
    }

    /// Task 2267 (lead review round 3, P3-3): whether the journal names the staged copy at `path`, and the live file
    /// it was copied from: `None` when no journal row names it, `Some(None)` when the row has no source.
    #[cfg(any(target_os = "windows", test))]
    pub fn staged_payload_source(&self, path: &str) -> Result<Option<Option<String>>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.query_row("SELECT source_path FROM staged_payloads WHERE path = ?1", params![path], |row| row.get(0))
            .optional()
    }

    /// R10, the upgrade path (fix round 1 of Task 10): record `candidate` as the owner of local data that has
    /// no owner, AT MOST ONCE per database, in one transaction. Once means the first startup after the upgrade:
    /// whatever this call finds, it shuts the window (`ADOPTION_CLOSED`), so a later startup, however it looks,
    /// never adopts, and neither does data an R10 build has already bound or purged. `candidate` is the account
    /// whose vault key the Keychain holds, or `None` when it holds none (then nothing is adopted and the window
    /// still closes). A recorded owner is never replaced. Returns whether anybody was adopted.
    pub fn adopt_owner_once(&self, candidate: Option<&crate::account_binding::Identity>) -> Result<bool> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction()?;
        let version: i64 = tx.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if version >= ADOPTION_CLOSED {
            return Ok(false);
        }
        let adopted = match candidate.filter(|candidate| candidate.is_known()) {
            Some(candidate) if read_owner(&tx)?.is_none() && account_rows(&tx)? > 0 => {
                write_owner(&tx, candidate)?;
                true
            }
            _ => false,
        };
        tx.pragma_update(None, "user_version", ADOPTION_CLOSED)?;
        tx.commit()?;
        Ok(adopted)
    }

    /// Shut the upgrade's adoption window (a no-op, and no write, when it is already shut): a database this build
    /// creates has no pre-R10 data to adopt, and every engine start shuts it.
    pub fn close_adoption_window(&self) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if version >= ADOPTION_CLOSED {
            return Ok(());
        }
        conn.pragma_update(None, "user_version", ADOPTION_CLOSED)
    }

    /// R10 reset, after `purge_all_local_state` has handed back the files to delete: every row of the
    /// previous account, the sync cursor and the owner record included, in one transaction.
    ///
    /// `owe_finder_removal` is written in the SAME transaction: a Mac whose previous account's Finder domain is
    /// still registered owes a removal, and no engine may start until it is confirmed (`finder_removal_owed`).
    /// `false` also clears a debt that was already recorded, because the caller only passes it once a removal
    /// was confirmed or none is needed.
    pub fn clear_account_data(&self, owe_finder_removal: bool) -> Result<()> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction()?;
        for table in ACCOUNT_TABLES.iter().chain(DEVICE_TABLES.iter()) {
            tx.execute(&format!("DELETE FROM {table}"), [])?;
        }
        tx.execute("DELETE FROM sync_state", [])?;
        let mark = if owe_finder_removal {
            FINDER_REMOVAL_OWED_KEY
        } else {
            FINDER_DOMAIN_GONE_KEY
        };
        tx.execute("INSERT INTO sync_state (key, value) VALUES (?1, '1')", params![mark])?;
        tx.commit()?;
        self.1.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        Ok(())
    }

    /// Did the last clear leave the Finder domain known gone (a sign-out whose removal was confirmed)?
    pub fn finder_domain_gone(&self) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let gone: Option<String> = conn
            .query_row(
                "SELECT value FROM sync_state WHERE key = ?1",
                params![FINDER_DOMAIN_GONE_KEY],
                |row| row.get(0),
            )
            .optional()?;
        Ok(gone.is_some())
    }

    /// The Finder domain was registered again (`addDomain` succeeded), so it is no longer known gone. Clears only
    /// the mark: no account row, owner or removal debt is touched. Only the macOS reconciler registers a domain.
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    pub fn clear_finder_domain_gone(&self) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute("DELETE FROM sync_state WHERE key = ?1", params![FINDER_DOMAIN_GONE_KEY])?;
        Ok(())
    }

    /// Is a Finder domain removal owed before any engine may start?
    pub fn finder_removal_owed(&self) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let owed: Option<String> = conn
            .query_row(
                "SELECT value FROM sync_state WHERE key = ?1",
                params![FINDER_REMOVAL_OWED_KEY],
                |row| row.get(0),
            )
            .optional()?;
        Ok(owed.is_some())
    }

    /// Record (or release) the debt on its own: a sign-out whose removal was not confirmed owes it after the
    /// rows were cleared; a confirmed removal releases it.
    pub fn set_finder_removal_owed(&self, owed: bool) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        if owed {
            conn.execute(
                "INSERT INTO sync_state (key, value) VALUES (?1, '1') ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![FINDER_REMOVAL_OWED_KEY],
            )?;
            // A domain may be registered again: it is no longer known gone.
            conn.execute("DELETE FROM sync_state WHERE key = ?1", params![FINDER_DOMAIN_GONE_KEY])?;
        } else {
            conn.execute(
                "DELETE FROM sync_state WHERE key = ?1",
                params![FINDER_REMOVAL_OWED_KEY],
            )?;
        }
        Ok(())
    }

    /// Spec 2026-10-10 §15 question 3: a sign-out that stops without purging reopens accepts at once.
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    pub fn reopen_finder_accepts(&self) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute(
            "DELETE FROM sync_state WHERE key = ?1",
            params![FINDER_ACCEPTS_CLOSED_KEY],
        )?;
        Ok(())
    }

    /// One `BEGIN IMMEDIATE` transaction: it waits for an accept transaction already running, which therefore commits
    /// first, and every later accept reads the key in its own transaction and queues nothing.
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    pub fn close_finder_accepts(&self) -> Result<()> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT INTO sync_state (key, value) VALUES (?1, '1') ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![FINDER_ACCEPTS_CLOSED_KEY],
        )?;
        tx.commit()
    }

    /// Whether accepts are closed now. Tests only: production reads the key inside the transaction that queues
    /// (`finder_accepts_closed_conn`).
    #[cfg(test)]
    pub fn finder_accepts_closed(&self) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        finder_accepts_closed_conn(&conn)
    }

    /// Spec 2026-10-10 §6.2: every live waiting change with `attempts + 2 < max_attempts` is made due now, once;
    /// `attempts` unchanged. Parked, paused, hydrates, pins and completed uploads are not touched.
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    pub fn make_due_for_send_phase(&self, now: i64) -> Result<usize> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute(
            "UPDATE operation_queue SET next_retry_at = ?1
             WHERE kind NOT IN ('hydrate_file', 'pin_tree')
               AND attempts + 2 < max_attempts
               AND paused_reason IS NULL
               AND op_id NOT IN (SELECT op_id FROM upload_resume WHERE completed_version IS NOT NULL)",
            params![now],
        )
    }

    /// Every file the rows of this database point at (queued and staged payloads, upload sessions, cached
    /// copies), each path once. Read-only: the purge deletes these files FIRST and clears the rows only when
    /// every one is gone, so a file that cannot be removed leaves the rows in place for a retry.
    pub fn local_state_file_paths(&self) -> Result<Vec<String>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut seen = HashSet::new();
        let mut paths = Vec::new();
        for sql in [
            "SELECT payload_path FROM operation_queue WHERE payload_path IS NOT NULL",
            "SELECT path FROM staged_payloads",
            "SELECT payload_path FROM upload_resume",
            "SELECT payload_path FROM upload_finalizations",
            "SELECT cache_path FROM files WHERE cache_path IS NOT NULL",
        ] {
            let mut stmt = conn.prepare(sql)?;
            for path in stmt.query_map([], |row| row.get::<_, String>(0))? {
                let path = path?;
                if seen.insert(path.clone()) {
                    paths.push(path);
                }
            }
        }
        Ok(paths)
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
    ///
    /// One `BEGIN IMMEDIATE` transaction (spec 2026-10-10 §5.2 step 8): it reads before it writes, and a deferred
    /// transaction's first write fails at once ("database is locked", no busy timeout) while another connection holds
    /// the write lock, such as the keep step's close of Finder accepts or a Finder change that reads that fence. The
    /// caller has deleted the files by then. Taking the lock as it begins, the purge waits for that writer.
    pub fn purge_all_local_state(&self) -> Result<LocalStatePurge> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;

        // m-12 (spec §5.4 row 15, §10.5): a File Provider create that never landed has
        // a provisional row the server never knew. The queue is its only owner, so it
        // is read before the queue goes; the row would otherwise survive as a ghost.
        // Write-keyed: a Windows or watcher create (no write id) is kept as before.
        let provisional: Vec<String> = {
            let mut stmt = tx.prepare(
                "SELECT DISTINCT q.file_id FROM operation_queue q JOIN files f ON f.file_id = q.file_id
                 WHERE q.kind IN ('upload_version', 'upload_file')
                   AND q.write_id IS NOT NULL
                   AND CASE WHEN json_valid(q.metadata_json)
                            THEN json_extract(q.metadata_json, '$.operation') END = 'create_file'
                   AND f.current_version = 0",
            )?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            rows.collect::<Result<Vec<_>>>()?
        };

        let mut payload_paths: Vec<String> = {
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
        // R10 (spec 2026-10-06 §5.6): staged payloads go with the queue. Their files are returned
        // with the other payloads, so the caller deletes them through the same safety gate.
        let staged_paths: Vec<String> = {
            let mut stmt = tx.prepare("SELECT path FROM staged_payloads")?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            rows.collect::<Result<Vec<_>>>()?
        };
        tx.execute("DELETE FROM staged_payloads", [])?;
        for path in staged_paths {
            if !payload_paths.contains(&path) {
                payload_paths.push(path);
            }
        }
        // R10: a sign-out forgets which account this local data belonged to.
        tx.execute(
            "DELETE FROM sync_state WHERE key IN (?1, ?2)",
            params![OWNER_USER_ID_KEY, OWNER_EMAIL_KEY],
        )?;

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

        // The round-4 state goes with the account (spec §5.4 row 15). The provisional
        // rows are removed after every path above was collected, so a path one of them
        // held is still handed to the caller.
        for file_id in &provisional {
            record_file_change_conn(&tx, file_id, FpChangeKind::Deleted, None)?;
            tx.execute("DELETE FROM files WHERE file_id = ?1", params![file_id])?;
        }
        tx.execute(
            "UPDATE files SET held_write_id = NULL, held_base = NULL, held_version = NULL,
                              held_object_version_id = NULL
             WHERE held_write_id IS NOT NULL",
            [],
        )?;
        tx.execute("DELETE FROM id_aliases", [])?;
        tx.execute("DELETE FROM owned_key_records", [])?;
        tx.execute("DELETE FROM sync_state WHERE key LIKE 'shared_folder_index_dirty:%' OR key LIKE 'shared_folder_custody_seen:%' OR key='owned_key_retry_after'", [])?;

        tx.commit()?;
        self.1.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        Ok(LocalStatePurge {
            queued_ops_purged,
            payload_paths,
            cache_paths,
            local_placeholder_paths,
        })
    }

    /// Unguarded; the runner records an attempt with [`Self::record_attempt_claimed`]
    /// (spec §8.7 S2). Kept for tests that set up queue states.
    #[cfg(test)]
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

    /// Tasks 1888 + 2226 (spec D1-D4, D8, D14; I1-I4): the reconcile pass. Plans over the whole queue and applies the
    /// plan in transactions of at most [`RECONCILE_UNITS_PER_TX`] whole units, re-planning after each, until the plan
    /// has no unit left or a round applied nothing. Returns the first round's counts: the plan the dry run shows.
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    /// `disk_has_file` says whether a sync-root-relative directory holds any file (D5); see [`tree_holds_a_file`].
    pub fn reconcile_held_queue(
        &self,
        opts: &crate::held_queue::Options,
        now: i64,
        disk_has_file: &dyn Fn(&str) -> bool,
    ) -> Result<crate::held_queue::Counts> {
        self.reconcile_in_batches(opts, now, RECONCILE_UNITS_PER_TX, disk_has_file, &mut |_, _| {})
    }

    /// The pass with its batch size, and `between` called after each committed batch (its 1-based number) while no
    /// lock is held: the tests' seam for a change made between two batches.
    pub(crate) fn reconcile_in_batches(
        &self,
        opts: &crate::held_queue::Options,
        now: i64,
        per_tx: usize,
        disk_has_file: &dyn Fn(&str) -> bool,
        between: &mut dyn FnMut(&StateDb, usize),
    ) -> Result<crate::held_queue::Counts> {
        let per_tx = per_tx.max(1);
        let mut first: Option<(usize, crate::held_queue::Counts)> = None;
        let mut batch = 0;
        loop {
            let applied = {
                let mut conn = self.0.lock().expect("state_db mutex poisoned");
                let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
                let (units, counts) = reconcile_plan_conn(&tx, opts, disk_has_file)?;
                let rounds_allowed = first.get_or_insert((units.len(), counts)).0 / per_tx + 2;
                // Every unit removes its own cause, so the plan shrinks each round; a plan that does not is a bug, and
                // the pass stops rather than loop.
                if batch >= rounds_allowed {
                    return Err(rusqlite::Error::SqliteFailure(
                        rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_MISUSE),
                        Some("reconcile: the plan did not shrink".into()),
                    ));
                }
                let take = units.len().min(per_tx);
                for unit in &units[..take] {
                    apply_reconcile_unit_conn(&tx, unit, now)?;
                }
                tx.commit()?;
                take
            };
            if applied == 0 {
                break;
            }
            batch += 1;
            between(self, batch);
        }
        Ok(first.map(|(_, counts)| counts).unwrap_or_default())
    }

    /// Spec D14: the plan's counts, from one read transaction; writes nothing.
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    pub fn reconcile_dry_run(
        &self,
        opts: &crate::held_queue::Options,
        disk_has_file: &dyn Fn(&str) -> bool,
    ) -> Result<crate::held_queue::Counts> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction()?;
        let (_, counts) = reconcile_plan_conn(&tx, opts, disk_has_file)?;
        tx.rollback()?;
        Ok(counts)
    }

    /// Lead ruling F9 (spec 2026-10-06 R8): make every operation paused because the server refused the session
    /// (`auth`) due again at `now`, keeping its attempts. Operations paused for any other reason stay paused. Returns how
    /// many were resumed.
    pub fn resume_operations_paused_for_auth(&self, now: i64) -> Result<usize> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute(
            "UPDATE operation_queue
             SET paused_reason = NULL,
                 next_retry_at = ?1,
                 updated_at = ?1
             WHERE paused_reason = ?2",
            params![now, OperationPauseReason::Auth.as_str()],
        )
    }

    /// Lead ruling, F9 review I-1 (spec 2026-10-06 R8, §5.6): the vault key this Mac kept is no longer the account's,
    /// so every operation paused for `auth` (queued with names encrypted under that key) is paused for `key_replaced`
    /// instead: kept, never sent, and never made due by [`Self::resume_operations_paused_for_auth`]. Operations paused
    /// for any other reason are untouched. Returns how many were re-marked. Only the macOS reconciler's re-sign-in
    /// path holds them; Windows and Linux keep today's flow (spec R8).
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    pub fn hold_operations_paused_for_auth_as_key_replaced(&self, now: i64) -> Result<usize> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute(
            "UPDATE operation_queue
             SET paused_reason = ?2,
                 updated_at = ?1
             WHERE paused_reason = ?3",
            params![
                now,
                OperationPauseReason::KeyReplaced.as_str(),
                OperationPauseReason::Auth.as_str()
            ],
        )
    }

    /// Unguarded; the runner records a pause with [`Self::record_pause_claimed`]
    /// (spec §8.7 S2). Kept for tests that set up queue states.
    #[cfg(test)]
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

    /// Every plaintext name this daemon knows locally, for redacting the lifecycle log
    /// (spec 2026-10-06 §8). Same source as the support bundle's redaction.
    pub fn known_names(&self, extra_paths: &[String]) -> Result<KnownNames> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        collect_known_names(&conn, extra_paths)
    }

    /// Like [`Self::queue_diagnostics`], additionally treating every component
    /// of `extra_paths` (for example the sync root) as a name to scrub.
    pub fn queue_diagnostics_with_paths(&self, now: i64, extra_paths: &[String]) -> Result<QueueDiagnostics> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let queued = conn.query_row("SELECT COUNT(*) FROM operation_queue", [], |row| row.get(0))?;
        let due = conn.query_row(
            &format!(
                "SELECT COUNT(*) FROM operation_queue
                 WHERE next_retry_at <= ?1 AND attempts < max_attempts AND paused_reason IS NULL {DUE_PARENT_GATE_SQL}"
            ),
            params![now],
            |row| row.get(0),
        )?;
        let paused = conn.query_row(
            "SELECT COUNT(*) FROM operation_queue WHERE paused_reason IS NOT NULL",
            [],
            |row| row.get(0),
        )?;

        let by_kind = allowed_group_labels(count_queue_groups(&conn, "kind")?, QUEUE_KIND_LABELS);
        let paused_by_reason = allowed_group_labels(count_queue_groups(&conn, "paused_reason")?, PAUSE_REASON_LABELS);
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

    /// [`QueueHold`] for the tray and Settings › Sync (0.8.15). Reads counts only, without the name-redaction pass of
    /// [`Self::queue_diagnostics`], so a window can poll it. Changes nothing.
    pub fn queue_hold_summary(&self) -> Result<QueueHold> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let (waiting, gave_up) = conn.query_row(
            "SELECT COALESCE(SUM(attempts < max_attempts), 0), COALESCE(SUM(attempts >= max_attempts), 0)
             FROM operation_queue
             WHERE paused_reason IS NULL",
            [],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
        )?;
        let mut stmt = conn.prepare(
            "SELECT paused_reason, COUNT(*), MIN(created_at), MAX(created_at)
             FROM operation_queue
             WHERE paused_reason IS NOT NULL
             GROUP BY paused_reason",
        )?;
        let groups = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })?;
        let mut held: BTreeMap<String, HeldGroup> = BTreeMap::new();
        let mut add = |reason: String, count: i64, oldest_at: i64, newest_at: i64| {
            let entry = held.entry(reason.clone()).or_insert(HeldGroup {
                reason,
                count: 0,
                oldest_at,
                newest_at,
            });
            entry.count += count;
            entry.oldest_at = entry.oldest_at.min(oldest_at);
            entry.newest_at = entry.newest_at.max(newest_at);
        };
        for group in groups {
            let (reason, count, oldest_at, newest_at) = group?;
            add(allowed_label(&reason, PAUSE_REASON_LABELS), count, oldest_at, newest_at);
        }
        // D7: a change that waits for its folder's create (`DUE_PARENT_GATE_SQL`) while that create is paused or out of
        // attempts is counted as `waiting_for_folder`, not as waiting: nothing sends it until the hold ends. The same
        // goes for a change in a folder made inside such a folder, at any depth. Read from the two D7 indexes alone;
        // CROSS JOIN keeps the held folders as the outer loop, so no change outside them is read.
        #[cfg(not(target_os = "macos"))]
        let waiting = {
            let (count, oldest_at, newest_at) = conn.query_row(
                &format!(
                    "WITH RECURSIVE {HELD_FOLDER_CTE}
                     SELECT COUNT(*), COALESCE(MIN(op.created_at), 0), COALESCE(MAX(op.created_at), 0)
                     FROM held_folder CROSS JOIN operation_queue op ON op.parent_id = held_folder.file_id
                     WHERE op.paused_reason IS NULL AND op.attempts < op.max_attempts"
                ),
                [],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?)),
            )?;
            if count > 0 {
                add("waiting_for_folder".to_string(), count, oldest_at, newest_at);
            }
            waiting - count
        };
        Ok(QueueHold {
            waiting,
            gave_up,
            held: held.into_values().collect(),
        })
    }

    /// Every `files` row as [`TreeRow`], for the Selective sync tree (0.8.15). Changes nothing.
    pub fn list_tree_rows(&self) -> Result<Vec<TreeRow>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT file_id, path, status, size_bytes, parent_id, item_kind, current_version, remote_updated_at
             FROM files ORDER BY path ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(TreeRow {
                file_id: row.get(0)?,
                path: row.get(1)?,
                status: FileStatus::from_str(&row.get::<_, String>(2)?),
                size_bytes: row.get(3)?,
                parent_id: row.get(4)?,
                item_kind: ItemKind::from_str(&row.get::<_, String>(5)?),
                current_version: row.get(6)?,
                remote_updated_at: row.get(7)?,
            })
        })?;
        rows.collect()
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

    /// One queued operation as it is now, or `None` once it is gone.
    pub fn get_operation(&self, op_id: &str) -> Result<Option<PendingOperation>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.query_row(
            &format!("SELECT {PENDING_OPERATION_COLUMNS} FROM operation_queue WHERE op_id = ?1"),
            params![op_id],
            pending_operation_from_row,
        )
        .optional()
    }

    /// Every queued operation for `file_id`, in queue order, whatever its
    /// retry state. The landing reads a file's later ops inside its transaction.
    #[cfg(test)]
    pub fn list_operations_for_file(&self, file_id: &str) -> Result<Vec<PendingOperation>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare(&format!(
            "SELECT {PENDING_OPERATION_COLUMNS} FROM operation_queue
             WHERE file_id = ?1
             {DUE_ORDER_SQL}"
        ))?;
        let rows = stmt.query_map(params![file_id], pending_operation_from_row)?;
        rows.collect()
    }

    /// S3: re-read the op, enforce the file's content order, and claim it, in one
    /// transaction. Replaces `get_operation` + the earlier-upload check the runner
    /// made in two separate calls. A Finder write also waits while its base is
    /// pending or its predecessor is queued, and parks when that predecessor parked
    /// or no op carries it any more (S5).
    pub fn claim_operation(&self, op_id: &str, now: i64) -> Result<ClaimOutcome> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        // Gone, or no longer due: a pass lists its ops once, and a landing earlier in the same
        // pass can park a later op of the file (a successor it could not re-key, §8.6 rule 3).
        // Nothing retries a parked op. An op whose parent folder's create has not landed is not
        // due either (D7): no claim, and its attempts and retry time stay as they are.
        let Some(op) = tx
            .query_row(
                &format!(
                    "SELECT {PENDING_OPERATION_COLUMNS} FROM operation_queue
                     WHERE op_id = ?1 AND attempts < max_attempts AND paused_reason IS NULL
                     {DUE_PARENT_GATE_SQL}"
                ),
                params![op_id],
                pending_operation_from_row,
            )
            .optional()?
        else {
            return Ok(ClaimOutcome::Gone);
        };
        if is_content_kind(&op.kind) {
            // Uploads of one file wait for its earlier uploads that can still run. A
            // restore waits only for Finder writes (an upload with a write id), and only
            // Finder writes wait for a restore: the watcher's and Windows' uploads carry
            // no write id and keep the upload-only order. The order is
            // `EARLIER_IN_QUEUE_SQL`.
            let earlier: bool = tx.query_row(
                &format!(
                    "SELECT EXISTS(
                        SELECT 1 FROM operation_queue AS this
                        JOIN operation_queue AS earlier
                          ON earlier.file_id = this.file_id AND earlier.op_id != this.op_id
                        WHERE this.op_id = ?1
                          AND earlier.kind IN ('upload_version', 'upload_file', 'restore_version')
                          AND earlier.attempts < earlier.max_attempts
                          AND ({EARLIER_IN_QUEUE_SQL})
                          AND (this.kind != 'restore_version' OR earlier.write_id IS NOT NULL)
                          AND (earlier.kind != 'restore_version' OR this.write_id IS NOT NULL))"
                ),
                params![op_id],
                |row| row.get(0),
            )?;
            if earlier {
                return Ok(ClaimOutcome::Wait);
            }
        }
        // Task 2204 S0 (review-155 R1, P2): a Windows rename waits for the file's earlier
        // sync-folder uploads that can still run, so their queue-time name lands first and the
        // rename lands last; a trash of the file waits for its earlier Windows rename the same way.
        if matches!(op.kind, OperationKind::RenameFile | OperationKind::MoveFile | OperationKind::TrashFile) {
            let earlier: bool = tx.query_row(
                &format!(
                    "SELECT EXISTS(
                        SELECT 1 FROM operation_queue AS this
                        JOIN operation_queue AS earlier
                          ON earlier.file_id = this.file_id AND earlier.op_id != this.op_id
                        WHERE this.op_id = ?1
                          AND this.write_id IS NULL AND earlier.write_id IS NULL
                          AND earlier.attempts < earlier.max_attempts AND earlier.paused_reason IS NULL
                          AND ({EARLIER_IN_QUEUE_SQL})
                          AND ((this.kind IN ('rename_file', 'move_file')
                                AND CASE WHEN json_valid(this.metadata_json)
                                    THEN json_extract(this.metadata_json, '$.local_rename') END = 1
                                AND earlier.kind IN ('upload_version', 'upload_file'))
                            OR (this.kind = 'trash_file'
                                AND earlier.kind IN ('rename_file', 'move_file')
                                AND CASE WHEN json_valid(earlier.metadata_json)
                                    THEN json_extract(earlier.metadata_json, '$.local_rename') END = 1)))"
                ),
                params![op_id],
                |row| row.get(0),
            )?;
            if earlier {
                return Ok(ClaimOutcome::Wait);
            }
        }
        // Steps 3 and 4, with the hand-over (spec §8.4): a parked predecessor this spec
        // minted hands its role to this op, in this transaction. The loop runs again on
        // what the op inherited: a pending base, or the predecessor's own predecessor.
        // §8.6 rules 4 and 6: an op whose completion is recorded waits for no base and no
        // predecessor, and never parks: its attempt repeats the landing.
        let completed: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM upload_resume WHERE op_id = ?1 AND completed_version IS NOT NULL)",
            params![op_id],
            |row| row.get(0),
        )?;
        let mut op = op;
        let mut write = finder_write_conn(&tx, op_id)?;
        let mut took_over: Option<TookOver> = None;
        while let Some(current) = write.clone().filter(|_| !completed) {
            if current.base_pending > 0 {
                return finish_wait(tx, took_over); // step 3, not an attempt
            }
            let Some(predecessor) = current.after_write_id.clone() else {
                break;
            };
            match predecessor_conn(&tx, &predecessor)? {
                // S5: only a bug can orphan a successor; it shows as a parked file.
                None => return park_in_claim(tx, &op, ParkReason::PredecessorLost, took_over, now),
                // Step 4: the predecessor is queued and has not parked.
                Some(pred) if pred.attempts < pred.max_attempts => return finish_wait(tx, took_over),
                // m-2: an earlier build's bytes may not be contained in this save's (§3).
                Some(pred) if pred.origin != Some(WriteOrigin::Minted) => {
                    return park_in_claim(tx, &op, ParkReason::PredecessorParked, took_over, now);
                }
                // A recorded completion never parks (§8.6 rule 6): wait for its landing.
                Some(pred) if pred.completed => return finish_wait(tx, took_over),
                Some(pred) => {
                    hand_over_conn(&tx, op_id, &current.write_id, &op, &pred)?;
                    // One per claim: a parked W only names a predecessor that had parked
                    // unminted, or gone, at W's own claim, so none is left to take over.
                    debug_assert!(took_over.is_none(), "a second hand-over in one claim");
                    took_over = Some(TookOver {
                        parked_op_id: pred.op_id.clone(),
                        released_payload: pred.payload_path.clone(),
                    });
                    op = tx.query_row(
                        &format!("SELECT {PENDING_OPERATION_COLUMNS} FROM operation_queue WHERE op_id = ?1"),
                        params![op_id],
                        pending_operation_from_row,
                    )?;
                    // W parked because its base cannot be known. This op now
                    // carries that base, so it parks the same way here, and no request goes
                    // out with it. Keyed on W's recorded reason, never on an inherited 0.
                    if pred.park_reason.as_deref() == Some(ParkReason::BaseUnknown.as_str()) {
                        return park_in_claim(tx, &op, ParkReason::BaseUnknown, took_over, now);
                    }
                    write = finder_write_conn(&tx, op_id)?;
                }
            }
        }
        let claim_id = uuid::Uuid::new_v4().simple().to_string();
        tx.execute(
            "UPDATE operation_queue SET claim_id = ?2, claimed_at = ?3 WHERE op_id = ?1",
            params![op_id, claim_id, now],
        )?;
        tx.commit()?;
        Ok(ClaimOutcome::Claimed(Box::new(ClaimedOp {
            op,
            claim_id,
            write,
            took_over,
        })))
    }

    /// A failed attempt of a claimed op: its retry schedule, and the claim ends.
    /// `false`: the op moved since the claim and nothing was written (S2).
    /// A Finder upload whose last attempt this was has parked by the queue's own rule
    /// (`attempts < max_attempts` is "not parked"): its row is settled as after every park
    /// (spec §9.1–§9.2), in this transaction.
    pub fn record_attempt_claimed(
        &self,
        op_id: &str,
        claim_id: &str,
        attempts: i64,
        next_retry_at: i64,
        last_error: Option<&str>,
    ) -> Result<bool> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let n = tx.execute(
            "UPDATE operation_queue
             SET attempts = ?3, next_retry_at = ?4, last_error = ?5, last_error_class = NULL,
                 paused_reason = NULL, updated_at = ?4, claim_id = NULL, claimed_at = NULL
             WHERE op_id = ?1 AND claim_id = ?2",
            params![op_id, claim_id, attempts, next_retry_at, last_error],
        )?;
        if n == 1 {
            // Write-keyed: an upload without a write id keeps its own rollback.
            let given_up: Option<Option<String>> = tx
                .query_row(
                    "SELECT file_id FROM operation_queue
                     WHERE op_id = ?1 AND write_id IS NOT NULL AND kind IN ('upload_version', 'upload_file')
                       AND attempts >= max_attempts",
                    params![op_id],
                    |row| row.get(0),
                )
                .optional()?;
            if let Some(Some(file_id)) = given_up {
                settle_status_after_park_conn(&tx, &file_id)?;
            }
        }
        tx.commit()?;
        Ok(n == 1)
    }

    /// A claimed op paused (auth, quota, permission, locked), and the claim ends.
    /// `false`: the op moved since the claim and nothing was written (S2).
    pub fn record_pause_claimed(
        &self,
        op_id: &str,
        claim_id: &str,
        reason: OperationPauseReason,
        last_error: Option<&str>,
        now: i64,
    ) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let n = conn.execute(
            "UPDATE operation_queue
             SET paused_reason = ?3, last_error_class = ?3, last_error = ?4, updated_at = ?5,
                 claim_id = NULL, claimed_at = NULL
             WHERE op_id = ?1 AND claim_id = ?2",
            params![
                op_id,
                claim_id,
                reason.as_str(),
                last_error.map(redact_diagnostic_error),
                now
            ],
        )?;
        Ok(n == 1)
    }

    /// Park with the bytes kept: attempts used up, so nothing retries it (spec §8.4).
    /// `false`: nothing was written, because the op moved since the claim (S2) or its
    /// completion is recorded (§8.6 rule 6: the runner retries such an op instead).
    pub fn park_claimed(&self, op_id: &str, claim_id: &str, reason: ParkReason, now: i64) -> Result<bool> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let file_id: Option<String> = tx
            .query_row(
                "SELECT file_id FROM operation_queue WHERE op_id = ?1",
                params![op_id],
                |row| row.get(0),
            )
            .optional()?
            .flatten();
        let n = tx.execute(
            "UPDATE operation_queue
             SET attempts = max_attempts, last_error = ?3, last_error_class = ?3, updated_at = ?4,
                 claim_id = NULL, claimed_at = NULL
             WHERE op_id = ?1 AND claim_id = ?2
               AND NOT EXISTS (SELECT 1 FROM upload_resume WHERE op_id = ?1 AND completed_version IS NOT NULL)",
            params![op_id, claim_id, reason.as_str(), now],
        )?;
        if n == 1
            && let Some(file_id) = file_id.as_deref()
        {
            settle_status_after_park_conn(&tx, file_id)?;
        }
        tx.commit()?;
        Ok(n == 1)
    }

    /// S4: the resume row is written only while the claimed op exists, with its
    /// payload journalled in the same transaction. `false`: the op moved since the
    /// claim and nothing was written.
    pub fn put_upload_resume_claimed(&self, resume: &UploadResume, claim_id: &str) -> Result<bool> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let n = tx.execute(
            "INSERT INTO upload_resume (
                op_id, payload_path, payload_size, payload_mtime_ns, upload_session_id,
                server_file_id, object_version_id, chunk_size_bytes, chunk_count,
                acked_chunks, metadata_applied, is_create, updated_at
             )
             SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, strftime('%s','now')
             WHERE EXISTS (SELECT 1 FROM operation_queue WHERE op_id = ?1 AND claim_id = ?13)
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
                claim_id,
            ],
        )?;
        if n == 1 {
            tx.execute(
                "INSERT INTO staged_payloads(path, source_path, completed) VALUES (?1, NULL, 0)
                 ON CONFLICT(path) DO NOTHING",
                params![resume.payload_path],
            )?;
        }
        tx.commit()?;
        Ok(n == 1)
    }

    /// The op's removal after it succeeded, with its resume row; a released payload is
    /// marked in the journal in the same transaction, and unlinked by the caller only
    /// after this commits (S6). `false`: the op moved since the claim and nothing was
    /// written.
    pub fn finish_claimed(&self, op_id: &str, claim_id: &str, release_payload: Option<&str>) -> Result<bool> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let n = tx.execute(
            "DELETE FROM operation_queue WHERE op_id = ?1 AND claim_id = ?2",
            params![op_id, claim_id],
        )?;
        if n == 1 {
            tx.execute("DELETE FROM upload_resume WHERE op_id = ?1", params![op_id])?;
            if let Some(path) = release_payload {
                tx.execute(
                    "INSERT INTO staged_payloads(path, completed) VALUES (?1, 1)
                     ON CONFLICT(path) DO UPDATE SET completed = 1",
                    params![path],
                )?;
            }
        }
        tx.commit()?;
        Ok(n == 1)
    }

    /// §7.1–§7.2: the server id a provisional id became, while no live row has it.
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    pub fn resolve_alias(&self, id: &str) -> Result<Option<String>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.query_row(
            "SELECT server_id FROM id_aliases
             WHERE provisional_id = ?1 AND NOT EXISTS (SELECT 1 FROM files WHERE file_id = ?1)",
            params![id],
            |row| row.get(0),
        )
        .optional()
    }

    /// §7.1: drop the aliases older than `max_age_secs`. Returns how many went.
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    pub fn sweep_aliases(&self, now: i64, max_age_secs: i64) -> Result<usize> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute(
            "DELETE FROM id_aliases WHERE created_at < ?1",
            params![now - max_age_secs],
        )
    }

    /// §7.4 and S1.8: the held write's staged copy, when that write is still queued, and
    /// whether that write has parked (attempts used up). One read: the held write and its op
    /// are looked up together.
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    pub fn queue_fetch_source(&self, file_id: &str) -> Result<Option<(String, bool)>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.query_row(
            "SELECT q.payload_path, q.attempts >= q.max_attempts FROM files f
             JOIN operation_queue q ON q.write_id = f.held_write_id
             WHERE f.file_id = ?1 AND q.payload_path IS NOT NULL",
            params![file_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
    }

    /// Engine start: no runner survives a restart, so every claim is cleared (S3), and
    /// aliases older than [`ALIAS_MAX_AGE_SECS`] are swept (§7.1).
    /// On macOS it also lists the journalled payloads marked released that nothing
    /// references any more, for the caller to unlink (S6).
    pub fn engine_start_repair(&self) -> Result<EngineStartRepair> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        // Spec 2026-10-10 §15 question 3: an engine that starts takes Finder writes again, whatever a stopped
        // sign-out left closed. A statement of its own, before the repair's transaction: a repair that fails rolls
        // back only its own work, and the runner serves Finder writes after a failed repair too.
        conn.execute(
            "DELETE FROM sync_state WHERE key = ?1",
            params![FINDER_ACCEPTS_CLOSED_KEY],
        )?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let claims_cleared = tx.execute(
            "UPDATE operation_queue SET claim_id = NULL, claimed_at = NULL WHERE claim_id IS NOT NULL",
            [],
        )?;
        // §7.1: an alias is kept 30 days; the daily sweep is the runner's.
        tx.execute(
            "DELETE FROM id_aliases WHERE created_at < ?1",
            params![now_secs() - ALIAS_MAX_AGE_SECS],
        )?;
        // macOS only: the release journal is written only by the macOS landing.
        #[cfg(target_os = "macos")]
        let released_payloads = {
            let mut stmt = tx.prepare(
                "SELECT path FROM staged_payloads
                 WHERE completed = 1
                   AND path NOT IN (SELECT payload_path FROM operation_queue WHERE payload_path IS NOT NULL)
                   AND path NOT IN (SELECT payload_path FROM upload_resume)
                   AND path NOT IN (SELECT payload_path FROM upload_finalizations)
                 ORDER BY path",
            )?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            rows.collect::<Result<Vec<_>>>()?
        };
        #[cfg(not(target_os = "macos"))]
        let released_payloads: Vec<String> = Vec::new();
        // §6.3.2: a server-known file row at version 0 asks for a snapshot to learn its
        // version. A provisional row (its create is queued) has no server version yet. A
        // malformed metadata row must not fail the start, so it is read as no create.
        #[cfg(target_os = "macos")]
        let resnapshot_requested = {
            let version_zero: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM files f
                  WHERE f.current_version = 0 AND f.item_kind = 'file' AND f.namespace = 'my_files'
                    AND f.status != 'trashing'
                    AND NOT EXISTS (SELECT 1 FROM operation_queue q
                                    WHERE q.file_id = f.file_id
                                      AND CASE WHEN json_valid(q.metadata_json)
                                               THEN json_extract(q.metadata_json, '$.operation')
                                          END = 'create_file'))",
                [],
                |r| r.get(0),
            )?;
            if version_zero {
                Self::request_resnapshot_conn(&tx)?;
            }
            version_zero
        };
        #[cfg(not(target_os = "macos"))]
        let resnapshot_requested = false;
        tx.commit()?;
        Ok(EngineStartRepair {
            claims_cleared,
            released_payloads,
            resnapshot_requested,
        })
    }

    /// §10.2–§10.3: every upload an earlier build queued gets a write id and is marked
    /// earlier-build; its base is left as it is. First (m-4c) the held columns of its file
    /// are cleared: a re-upgrade must not revive a token the system no longer holds.
    /// Nothing else changes: a pause, the attempts and the staged copy's path stay as they are.
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    pub fn mark_earlier_build_uploads(&self) -> Result<usize> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute(
            "UPDATE files SET held_write_id = NULL, held_base = NULL, held_version = NULL, held_object_version_id = NULL
             WHERE file_id IN (SELECT file_id FROM operation_queue
                               WHERE write_id IS NULL AND file_id IS NOT NULL
                                 AND kind IN ('upload_version', 'upload_file'))",
            [],
        )?;
        let ops: Vec<String> = {
            let mut stmt = tx.prepare(
                "SELECT op_id FROM operation_queue
                 WHERE write_id IS NULL AND kind IN ('upload_version', 'upload_file') ORDER BY rowid",
            )?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            rows.collect::<Result<Vec<_>>>()?
        };
        for op_id in &ops {
            tx.execute(
                "UPDATE operation_queue SET write_id = ?2, write_origin = ?3 WHERE op_id = ?1",
                params![
                    op_id,
                    crate::write_token::mint_write_id(),
                    WriteOrigin::EarlierBuild.as_str()
                ],
            )?;
        }
        tx.commit()?;
        Ok(ops.len())
    }

    #[cfg(test)]
    pub(crate) fn set_created_at_for_test(&self, op_id: &str, created_at: i64) {
        let conn = self.0.lock().unwrap();
        conn.execute(
            "UPDATE operation_queue SET created_at = ?2 WHERE op_id = ?1",
            params![op_id, created_at],
        )
        .unwrap();
    }

    /// T14: an op whose base went missing after it was queued.
    #[cfg(test)]
    pub(crate) fn set_base_version_for_test(&self, op_id: &str, base_version: Option<i64>) {
        let conn = self.0.lock().unwrap();
        conn.execute(
            "UPDATE operation_queue SET base_version = ?2 WHERE op_id = ?1",
            params![op_id, base_version],
        )
        .unwrap();
    }

    /// T6, T62: an alias the id swap would record (spec §7.1), without a landing.
    #[cfg(test)]
    pub(crate) fn insert_alias_for_test(&self, provisional_id: &str, server_id: &str, created_at: i64) {
        let conn = self.0.lock().unwrap();
        conn.execute(
            "INSERT INTO id_aliases (provisional_id, server_id, created_at) VALUES (?1, ?2, ?3)",
            params![provisional_id, server_id, created_at],
        )
        .unwrap();
    }

    /// Writes the held pair as given, including the half-set row
    /// (a write id without its base) that no writer produces and every read refuses.
    #[cfg(test)]
    pub(crate) fn set_held_pair_for_test(&self, file_id: &str, write_id: Option<&str>, base: Option<i64>) {
        let conn = self.0.lock().unwrap();
        conn.execute(
            "UPDATE files SET held_write_id = ?2, held_base = ?3 WHERE file_id = ?1",
            params![file_id, write_id, base],
        )
        .unwrap();
    }

    #[cfg(test)]
    pub(crate) fn alias_count_for_test(&self) -> i64 {
        let conn = self.0.lock().unwrap();
        conn.query_row("SELECT COUNT(*) FROM id_aliases", [], |row| row.get(0))
            .unwrap()
    }

    /// T26 (m-2): an op as an earlier build would have queued it (spec §10.2).
    #[cfg(test)]
    pub(crate) fn set_write_origin_for_test(&self, op_id: &str, origin: WriteOrigin) {
        let conn = self.0.lock().unwrap();
        conn.execute(
            "UPDATE operation_queue SET write_origin = ?2 WHERE op_id = ?1",
            params![op_id, origin.as_str()],
        )
        .unwrap();
    }

    /// A resume row whose `complete` answered (spec §8.6 rule 1), without an upload.
    #[cfg(test)]
    pub(crate) fn set_completed_for_test(&self, op_id: &str, version: i64) {
        let conn = self.0.lock().unwrap();
        let n = conn
            .execute(
                "UPDATE upload_resume SET completed_version = ?2 WHERE op_id = ?1",
                params![op_id, version],
            )
            .unwrap();
        assert_eq!(n, 1, "no resume row for {op_id}");
    }

    /// P10: where the landing's alias sends a provisional id (spec §7.1).
    #[cfg(test)]
    pub(crate) fn alias_target_for_test(&self, provisional_id: &str) -> Option<String> {
        let conn = self.0.lock().unwrap();
        conn.query_row(
            "SELECT server_id FROM id_aliases WHERE provisional_id = ?1",
            params![provisional_id],
            |row| row.get(0),
        )
        .optional()
        .unwrap()
    }

    /// T27: an op whose name cannot be re-encrypted (no display name, no path).
    #[cfg(test)]
    pub(crate) fn set_target_path_for_test(&self, op_id: &str, target_path: Option<&str>) {
        let conn = self.0.lock().unwrap();
        conn.execute(
            "UPDATE operation_queue SET target_path = ?2 WHERE op_id = ?1",
            params![op_id, target_path],
        )
        .unwrap();
    }

    /// T26: an op that used up its attempts (parked, with its bytes).
    #[cfg(test)]
    pub(crate) fn park_for_test(&self, op_id: &str) {
        let conn = self.0.lock().unwrap();
        conn.execute(
            "UPDATE operation_queue SET attempts = max_attempts WHERE op_id = ?1",
            params![op_id],
        )
        .unwrap();
    }

    /// P2: a Finder write is an op with a write id; the restore/upload wait is keyed on it.
    #[cfg(test)]
    pub(crate) fn set_write_id_for_test(&self, op_id: &str, write_id: &str) {
        let conn = self.0.lock().unwrap();
        conn.execute(
            "UPDATE operation_queue SET write_id = ?2 WHERE op_id = ?1",
            params![op_id, write_id],
        )
        .unwrap();
    }

    /// Tests outside this module: run one SQL statement (e.g. park an op by hand).
    #[cfg(test)]
    pub(crate) fn test_execute(&self, sql: &str) -> Result<usize> {
        self.0.lock().expect("state_db mutex poisoned").execute(sql, [])
    }

    /// Unguarded; the runner removes an op with [`Self::finish_claimed`] (spec §8.7
    /// S2). Kept for tests that set up queue states.
    #[cfg(test)]
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
                    acked_chunks, metadata_applied, is_create, completed_version,
                    completed_object_version_id, completed_mime_type
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
                    completed_version: row.get(12)?,
                    completed_object_version_id: row.get(13)?,
                    completed_mime_type: row.get(14)?,
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

/// The owner record in `sync_state`, if any field of it is there.
fn read_owner(conn: &Connection) -> Result<Option<crate::account_binding::Identity>> {
    let read = |key: &str| {
        conn.query_row("SELECT value FROM sync_state WHERE key = ?1", params![key], |row| {
            row.get::<_, String>(0)
        })
        .optional()
    };
    let owner =
        crate::account_binding::Identity::new(read(OWNER_USER_ID_KEY)?.as_deref(), read(OWNER_EMAIL_KEY)?.as_deref());
    Ok(owner.is_known().then_some(owner))
}

/// Both owner fields at once; an unknown field is removed.
fn write_owner(conn: &Connection, owner: &crate::account_binding::Identity) -> Result<()> {
    for (key, value) in [(OWNER_USER_ID_KEY, &owner.user_id), (OWNER_EMAIL_KEY, &owner.email)] {
        match value {
            Some(value) => conn.execute(
                "INSERT INTO sync_state (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )?,
            None => conn.execute("DELETE FROM sync_state WHERE key = ?1", params![key])?,
        };
    }
    Ok(())
}

/// How many rows of an account live here: every `ACCOUNT_TABLES` row and every `sync_state` row except the
/// owner record and the device keys (the Finder removal debt, the domain-gone mark, closed Finder accepts).
fn account_rows(conn: &Connection) -> Result<i64> {
    let mut rows: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sync_state WHERE key NOT IN (?1, ?2, ?3, ?4, ?5)",
        params![
            OWNER_USER_ID_KEY,
            OWNER_EMAIL_KEY,
            FINDER_REMOVAL_OWED_KEY,
            FINDER_DOMAIN_GONE_KEY,
            FINDER_ACCEPTS_CLOSED_KEY
        ],
        |row| row.get(0),
    )?;
    for table in ACCOUNT_TABLES {
        rows += conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| row.get::<_, i64>(0))?;
    }
    Ok(rows)
}

#[cfg(test)]
fn has_table(conn: &Connection, table: &str) -> Result<bool> {
    let mut stmt = conn.prepare("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1")?;
    let mut rows = stmt.query(params![table])?;
    Ok(rows.next()?.is_some())
}

#[cfg(test)]
mod tests {
    /// The Windows sign-out leaves no account data behind, whatever `ACCOUNT_TABLES` and `DEVICE_TABLES` list: one row
    /// goes into every listed table it does not refuse on (a dummy value per column, from the schema), and after
    /// `finish_windows_signout` every listed table is empty and `has_account_data` is false. A table added to either list
    /// and not cleared, or listed but missing from the schema, fails here. The three tables it refuses on (an unsent
    /// change is never discarded) must already be empty; the tests above cover each refusal.
    #[test]
    fn a_windows_sign_out_empties_every_account_table() {
        const REFUSED_ON: [&str; 3] = ["operation_queue", "staged_payloads", "upload_finalizations"];
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        let db = StateDb::open(&path).unwrap();
        {
            let conn = Connection::open(&path).unwrap();
            for table in ACCOUNT_TABLES.iter().chain(DEVICE_TABLES.iter()) {
                let columns: Vec<String> = conn
                    .prepare(&format!("PRAGMA table_info({table})"))
                    .unwrap()
                    .query_map([], |row| {
                        Ok(format!(
                            "{}|{}",
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?.to_ascii_uppercase()
                        ))
                    })
                    .unwrap()
                    .collect::<std::result::Result<_, _>>()
                    .unwrap();
                assert!(
                    !columns.is_empty(),
                    "{table} is listed in ACCOUNT_TABLES or DEVICE_TABLES but the schema has no such table"
                );
                if REFUSED_ON.contains(table) {
                    continue;
                }
                let (names, values): (Vec<&str>, Vec<&str>) = columns
                    .iter()
                    .map(|column| {
                        let (name, kind) = column.split_once('|').unwrap();
                        let value = if kind.contains("INT") {
                            "1"
                        } else if kind.contains("REAL") {
                            "1.0"
                        } else if kind.contains("BLOB") {
                            "x'00'"
                        } else {
                            "'x'"
                        };
                        (name, value)
                    })
                    .unzip();
                conn.execute(
                    &format!(
                        "INSERT INTO {table} ({}) VALUES ({})",
                        names.join(", "),
                        values.join(", ")
                    ),
                    [],
                )
                .unwrap();
            }
        }
        assert!(
            db.has_account_data().unwrap(),
            "the premise: the account tables hold rows"
        );
        db.finish_windows_signout().expect("nothing waits to upload");
        let conn = Connection::open(&path).unwrap();
        for table in ACCOUNT_TABLES.iter().chain(DEVICE_TABLES.iter()) {
            let rows: i64 = conn
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| row.get(0))
                .unwrap();
            assert_eq!(rows, 0, "the Windows sign-out left rows in {table}");
        }
        assert!(!db.has_account_data().unwrap(), "no account data is left");
    }

    /// Task 2240: file rows in a fresh state.db, enough to fill several times SQLite's default page cache (about 300
    /// bytes each). The scan's lookups find them as `dir/<i>.txt`.
    const TASK_2240_ROWS: usize = 20_000;

    fn task_2240_add_rows(path: &Path, from: usize, rows: usize, padding: usize) {
        let mut conn = Connection::open(path).unwrap();
        let tx = conn.transaction().unwrap();
        let content_type = "x".repeat(padding);
        for i in from..from + rows {
            tx.execute(
                "INSERT INTO files (file_id, path, content_type) VALUES (?1, ?2, ?3)",
                params![format!("f{i}"), format!("dir/{i}.txt"), content_type],
            )
            .unwrap();
        }
        tx.commit().unwrap();
    }

    fn task_2240_db() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        StateDb::open(&path).unwrap();
        task_2240_add_rows(&path, 0, TASK_2240_ROWS, 200);
        (dir, path)
    }

    /// One pass of the scan's lookups when every file is settled: the row stamp of every file, as
    /// `run_one_scan_with` reads it. Returns the pages the pass read from the file.
    fn task_2240_pass(db: &StateDb) -> i64 {
        let before = db.page_reads();
        for i in 0..TASK_2240_ROWS {
            assert!(db.file_row_stamp_by_path(&format!("dir/{i}.txt")).unwrap().is_some());
        }
        db.page_reads() - before
    }

    fn task_2240_cache_size(db: &StateDb) -> i64 {
        db.0.lock()
            .unwrap()
            .query_row("PRAGMA cache_size", [], |row| row.get(0))
            .unwrap()
    }

    /// Task 2240: runs `measure` in a fresh copy of this test binary that runs only the test `name`.
    ///
    /// The bundled SQLite is built with `SQLITE_ENABLE_MEMORY_MANAGEMENT` (libsqlite3-sys), so all the connections of
    /// a process share one page-cache LRU: a connection that reads more than its own cache takes the oldest pages of
    /// the others. A test that counts page reads in this binary would count the reads of every test running beside
    /// it. The parent passes only when the child ran exactly one test, passed, and printed what it measured.
    fn task_2240_in_own_process(name: &str, measure: impl FnOnce() -> String) {
        const CHILD: &str = "BB_TASK_2240_PAGE_CACHE_CHILD";
        if std::env::var_os(CHILD).is_some() {
            println!("task 2240 child {name}: {}", measure());
            return;
        }
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                &format!("state_db::tests::{name}"),
                "--test-threads=1",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            out.status.success(),
            "the child run of {name} failed:\n{stdout}\n{stderr}"
        );
        assert!(
            stdout.contains("test result: ok. 1 passed;"),
            "the child ran no test:\n{stdout}"
        );
        let proof = format!("task 2240 child {name}: ");
        let line = stdout.lines().find(|line| line.contains(&proof));
        println!(
            "{}",
            line.unwrap_or_else(|| panic!("the child never measured:\n{stdout}"))
        );
    }

    /// Task 2240 (D2, T1): the limit is the DB's size plus an eighth, within [2,000 KiB, 128 MiB].
    #[test]
    fn task_2240_the_page_cache_limit_follows_the_db_size() {
        for (db_bytes, kib) in [
            (0, 2_000),
            (1 << 20, 2_000),
            (30_707_712, 33_736),
            (120 << 20, 131_072),
            (2_i64 << 30, 131_072),
        ] {
            assert_eq!(page_cache_limit_kib(db_bytes), kib, "{db_bytes} bytes");
        }
    }

    /// Task 2240 (T2): with SQLite's default cache, the scan's second pass reads its pages from the file again; with
    /// the cache fitted it reads none. The default handle is measured first and closed: with one shared LRU, it would
    /// otherwise borrow the fitted handle's pages.
    #[test]
    fn task_2240_a_fitted_cache_reads_nothing_on_the_second_pass() {
        task_2240_in_own_process("task_2240_a_fitted_cache_reads_nothing_on_the_second_pass", || {
            let (_dir, path) = task_2240_db();
            let default = StateDb::open(&path).unwrap();
            task_2240_pass(&default);
            let unfitted = task_2240_pass(&default);
            assert!(unfitted > 0, "precondition: the default cache cannot hold the pass");
            drop(default);
            let fitted = StateDb::open(&path).unwrap();
            fitted.fit_page_cache().unwrap();
            let cold = task_2240_pass(&fitted);
            let warm = task_2240_pass(&fitted);
            assert_eq!(warm, 0, "a fitted cache holds every page the pass reads");
            format!("default cache: second pass read {unfitted} pages; fitted: {cold} then {warm}")
        });
    }

    /// Task 2240 (T3, premise P1): every UI command opens the DB on its own connection. Opening it commits nothing, so
    /// the scan's cache survives it. An `open` that writes would empty that cache on every poll.
    #[test]
    fn task_2240_a_ui_open_between_passes_keeps_the_cache() {
        task_2240_in_own_process("task_2240_a_ui_open_between_passes_keeps_the_cache", || {
            let (_dir, path) = task_2240_db();
            let engine = StateDb::open(&path).unwrap();
            engine.fit_page_cache().unwrap();
            task_2240_pass(&engine);
            drop(StateDb::open(&path).unwrap());
            let warm = task_2240_pass(&engine);
            assert_eq!(warm, 0, "opening the DB elsewhere leaves the scan's cache warm");
            format!("pass after a UI open read {warm} pages")
        });
    }

    /// Task 2240 (T4, I4): a commit by another connection empties the cache; the next pass reads cold once, and the
    /// one after it reads nothing again.
    #[test]
    fn task_2240_a_commit_elsewhere_costs_one_cold_pass() {
        task_2240_in_own_process("task_2240_a_commit_elsewhere_costs_one_cold_pass", || {
            let (_dir, path) = task_2240_db();
            let engine = StateDb::open(&path).unwrap();
            engine.fit_page_cache().unwrap();
            task_2240_pass(&engine);
            Connection::open(&path)
                .unwrap()
                .execute(
                    "UPDATE files SET last_opened_at = last_opened_at + 1 WHERE file_id = 'f0'",
                    [],
                )
                .unwrap();
            let cold = task_2240_pass(&engine);
            assert!(cold > 0, "a commit elsewhere empties the cache");
            let warm = task_2240_pass(&engine);
            assert_eq!(warm, 0, "one pass warms it again");
            format!("after a commit elsewhere: {cold} pages, then {warm}")
        });
    }

    /// Task 2240 (I6): a UI command that reads more than its own 2,000 KiB on its connection takes the scan's oldest
    /// pages (one shared LRU). The next pass re-reads at most what that command read, never more than a cold pass, and
    /// the one after it reads nothing.
    #[test]
    fn task_2240_a_large_ui_read_costs_at_most_one_partial_pass() {
        task_2240_in_own_process("task_2240_a_large_ui_read_costs_at_most_one_partial_pass", || {
            let (_dir, path) = task_2240_db();
            let engine = StateDb::open(&path).unwrap();
            engine.fit_page_cache().unwrap();
            let full = task_2240_pass(&engine);
            let ui = StateDb::open(&path).unwrap();
            let before = ui.page_reads();
            let _: i64 =
                ui.0.lock()
                    .unwrap()
                    .query_row("SELECT sum(length(content_type)) FROM files", [], |row| row.get(0))
                    .unwrap();
            let ui_read = ui.page_reads() - before;
            drop(ui);
            let partial = task_2240_pass(&engine);
            assert!(partial > 0, "precondition: the UI read took some of the scan's pages");
            assert!(
                partial <= ui_read && partial <= full,
                "{partial} pages, the UI read {ui_read}, a cold pass {full}"
            );
            let warm = task_2240_pass(&engine);
            assert_eq!(warm, 0, "one pass warms it again");
            format!("cold pass {full}; UI read {ui_read}; next pass {partial}, then {warm}")
        });
    }

    /// Task 2240 (T5, I2): only the connection that is fitted gets the larger cache; a UI command's fresh one keeps
    /// SQLite's default.
    #[test]
    fn task_2240_ui_handles_keep_the_default_cache() {
        let (_dir, path) = task_2240_db();
        let engine = StateDb::open(&path).unwrap();
        let fitted = engine.fit_page_cache().unwrap();
        assert!(
            fitted > PAGE_CACHE_FLOOR_KIB,
            "precondition: this DB fits above the floor"
        );
        assert_eq!(task_2240_cache_size(&engine), -fitted);
        assert_eq!(
            task_2240_cache_size(&StateDb::open(&path).unwrap()),
            -PAGE_CACHE_FLOOR_KIB
        );
    }

    /// Task 2240 (T6, D3): a DB that grows gets a larger cache at the next fit.
    #[test]
    fn task_2240_the_cache_is_refitted_when_the_db_grows() {
        let (_dir, path) = task_2240_db();
        let engine = StateDb::open(&path).unwrap();
        let first = engine.fit_page_cache().unwrap();
        task_2240_add_rows(&path, TASK_2240_ROWS, 10_000, 4_000);
        let second = engine.fit_page_cache().unwrap();
        assert!(
            second > first,
            "the limit grows with the DB: {first} KiB, then {second} KiB"
        );
        assert_eq!(task_2240_cache_size(&engine), -second);
    }

    /// Task 2229: the enumeration scan looks every file and folder on disk up by path on every
    /// pass, so the lookup steps no row of a full scan of `files`, whether the path is there or
    /// not. That holds on a DB opened from a release before the index (0.8.15 and earlier),
    /// because opening adds it. The counters read are those of the app's own cached statement,
    /// so the lookup `get_file_by_path` runs is the one measured.
    #[test]
    fn a_path_lookup_reads_through_an_index_not_the_whole_files_table() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        {
            StateDb::open(&path).unwrap();
            let mut conn = Connection::open(&path).unwrap();
            let tx = conn.transaction().unwrap();
            for i in 0..2000 {
                tx.execute(
                    "INSERT INTO files (file_id, path) VALUES (?1, ?2)",
                    params![format!("f{i}"), format!("dir/{i}.txt")],
                )
                .unwrap();
            }
            tx.commit().unwrap();
            // The schema 0.8.15 shipped: no index on `files.path`.
            conn.execute("DROP INDEX IF EXISTS idx_files_path", []).unwrap();
        }
        let db = StateDb::open(&path).unwrap();
        for (probe, found) in [("dir/1999.txt", Some("f1999")), ("dir/missing.txt", None)] {
            assert_eq!(
                db.get_file_by_path(probe).unwrap().map(|e| e.file_id),
                found.map(str::to_string)
            );
            assert_eq!(db.file_row_stamp_by_path(probe).unwrap().is_some(), found.is_some());
            let conn = db.0.lock().unwrap();
            for sql in [FILE_BY_PATH_SQL, FILE_ROW_BY_PATH_SQL] {
                let stmt = conn.prepare_cached(sql).unwrap();
                assert!(
                    stmt.get_status(rusqlite::StatementStatus::Run) > 0,
                    "the app did not run {sql}"
                );
                let full_scan_steps = stmt.reset_status(rusqlite::StatementStatus::FullscanStep);
                stmt.reset_status(rusqlite::StatementStatus::Run);
                assert_eq!(
                    full_scan_steps, 0,
                    "looking up {probe} stepped {full_scan_steps} rows of a full scan of files: {sql}"
                );
            }
        }
    }

    /// Task 2229: the scan skips a settled file while the stamp of its row is unchanged, so every
    /// write to the row changes the stamp, and either stored slash shape finds it.
    #[test]
    fn a_row_stamp_changes_with_any_write_to_the_row() {
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        assert_eq!(
            db.file_row_stamp_by_path("docs/a.txt").unwrap(),
            None,
            "no row, no stamp"
        );
        {
            let conn = db.0.lock().unwrap();
            conn.execute("INSERT INTO files (file_id, path) VALUES ('f1', '/docs/a.txt')", [])
                .unwrap();
        }
        let first = db.file_row_stamp_by_path("docs/a.txt").unwrap();
        assert!(first.is_some(), "the leading-slash shape is found too");
        assert_eq!(db.file_row_stamp_by_path("/docs/a.txt").unwrap(), first);
        assert_eq!(
            db.file_row_stamp_by_path("docs/a.txt").unwrap(),
            first,
            "unchanged row, same stamp"
        );
        for write in [
            "UPDATE files SET status = 'local' WHERE file_id = 'f1'",
            "UPDATE files SET local_hash = 'h1' WHERE file_id = 'f1'",
            "UPDATE files SET local_hash = NULL WHERE file_id = 'f1'",
            "UPDATE files SET local_base_version = 2 WHERE file_id = 'f1'",
            "UPDATE files SET held_write_id = 'w1' WHERE file_id = 'f1'",
        ] {
            let before = db.file_row_stamp_by_path("docs/a.txt").unwrap();
            db.0.lock().unwrap().execute(write, []).unwrap();
            assert_ne!(db.file_row_stamp_by_path("docs/a.txt").unwrap(), before, "{write}");
        }
    }

    #[test]
    fn round7_finalization_survives_restart_and_blocks_unfinished_signout() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        let db = StateDb::open(&path).unwrap();
        let row = UploadFinalization {
            op_id: "op".into(),
            local_file_id: "local".into(),
            server_file_id: "server".into(),
            target_path: "file".into(),
            payload_path: "proof".into(),
            stamped: false,
        };
        db.put_upload_finalization(&row).unwrap();
        assert_eq!(db.upload_finalizations().unwrap(), vec![row.clone()]);
        db.forget_upload_finalization("op").unwrap();
        assert_eq!(
            db.upload_finalizations().unwrap().len(),
            1,
            "unstamped proof must not be forgotten"
        );
        db.forget_staged_payload("proof").unwrap();
        assert!(
            db.finish_windows_signout().is_err(),
            "finalization owns its proof independently of staging"
        );
        db.mark_upload_finalization_stamped("op").unwrap();
        drop(db);
        let db = StateDb::open(&path).unwrap();
        assert!(db.upload_finalizations().unwrap()[0].stamped);
        db.forget_upload_finalization("op").unwrap();
        db.finish_windows_signout().unwrap();
        assert_eq!(db.upload_finalizations().unwrap().len(), 0);
    }

    // 2026-10-07 (task 1834, R10): renamed from `round5_legacy_resume_inventory_survives_operation_purge`
    // and its assertion changed. It used to require that `purge_all_local_state` KEEP the staged-payload
    // journal row. R10 (spec 2026-10-06 §5.6, lead ruling 4 "staged_payloads rows are included in the
    // purge") makes the purge forget the row in the same transaction and hand the file back for deletion
    // instead. What the test protects is unchanged: a legacy payload whose only durable owner is the
    // resume row is never silently lost. It now holds because the purge RETURNS that path, which the
    // caller (`purge_local_state_files`) deletes through the one disposable-path gate.
    #[test]
    fn round5_legacy_resume_inventory_is_handed_back_by_the_operation_purge() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("state.db");
        let db = StateDb::open(&path).unwrap();
        // Mimic an old database: no staging journal row, no queued owner.
        db.0.lock().unwrap().execute_batch("INSERT INTO upload_resume
            (op_id,payload_path,payload_size,payload_mtime_ns,upload_session_id,server_file_id,object_version_id,chunk_size_bytes,chunk_count)
            VALUES ('old','only-copy',1,0,'session','file','version',1,1)").unwrap();
        drop(db);
        let db = StateDb::open(&path).unwrap();
        assert_eq!(
            db.staged_payloads_for_signout().unwrap(),
            vec![("only-copy".into(), None, false)],
            "the journal owns it after the upgrade"
        );
        let purge = db.purge_all_local_state().unwrap();
        assert!(db.get_upload_resume("old").unwrap().is_none());
        assert!(
            purge.payload_paths.contains(&"only-copy".to_string()),
            "old payload lost its only durable owner on resume purge without being handed back: {:?}",
            purge.payload_paths
        );
        assert!(
            db.staged_payloads_for_signout().unwrap().is_empty(),
            "and the journal row goes with it"
        );
    }

    /// Desktop 0.8.15 (deliverable 3): changes held back (on Guus's PC, 2,832 folder creates and
    /// 92 versions paused for `auth` since 4 Oct) are counted as on hold. They are never transfer
    /// work, so they cannot keep the engine at "syncing"; they are not resumed here either.
    #[test]
    fn desktop_0815_changes_on_hold_are_counted_and_are_never_syncing() {
        let db = super::StateDb::open(":memory:").unwrap();
        db.0.lock()
            .unwrap()
            .execute_batch(
                "INSERT INTO operation_queue (op_id,kind,paused_reason,created_at,updated_at) VALUES
                   ('a','create_folder','auth',1,1), ('b','upload_version','auth',1,1), ('c','upload_version',NULL,1,1);",
            )
            .unwrap();
        assert_eq!(db.count_operations_on_hold().unwrap(), 2);
        assert_eq!(db.transfer_backlog(i64::MAX).unwrap().upload_files, 1, "only the change not on hold is transfer work");
        let still_paused: i64 = db
            .0
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM operation_queue WHERE paused_reason = 'auth'", [], |row| row.get(0))
            .unwrap();
        assert_eq!(still_paused, 2, "counting resumes nothing");
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

    /// Task 2190: a `local` file row for the held-edit tests.
    fn held_row(file_id: &str, path: &str) -> FileEntry {
        FileEntry {
            file_id: file_id.to_string(),
            path: path.to_string(),
            status: FileStatus::Local,
            size_bytes: 1,
            modified_at: 1,
            content_hash: None,
            remote_updated_at: 1,
            parent_id: None,
            item_kind: ItemKind::File,
        }
    }

    /// Task 2190 (glegion's constraint 1): a held edit is written once; the same reason and
    /// fingerprint again write nothing (a scan pass that finds it again writes no page); a new
    /// fingerprint writes again, and the first detection's time is kept.
    #[test]
    fn task_2190_a_held_edit_is_written_once_per_fingerprint() {
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        db.upsert_file(&held_row("h1", "a.txt")).unwrap();
        assert!(db.hold_unsent_edit("h1", "base_unknown", "fp1", 10).unwrap(), "the first detection writes");
        assert!(!db.hold_unsent_edit("h1", "base_unknown", "fp1", 20).unwrap(), "unchanged: nothing written");
        assert!(db.hold_unsent_edit("h1", "base_unknown", "fp2", 30).unwrap(), "a new fingerprint writes");
        assert_eq!(
            db.list_held_edits().unwrap(),
            vec![("h1".to_string(), "a.txt".to_string(), "base_unknown".to_string(), 10)]
        );
        assert!(db.release_unsent_edit("h1").unwrap());
        assert!(!db.release_unsent_edit("h1").unwrap(), "nothing left to release");
    }

    /// Task 2190 (glegion's constraint 2): the paths that skip the classify clear the record, so no
    /// feed entry goes stale: the row's delete, its trash, a rename or move (its path), any status
    /// away from `local`, and a re-key of its id. A write that changes none of them keeps it.
    #[test]
    fn task_2190_a_held_edit_goes_with_its_row_path_status_and_id() {
        let held = |db: &StateDb| -> i64 {
            db.0.lock().unwrap().query_row("SELECT COUNT(*) FROM held_edits", [], |row| row.get(0)).unwrap()
        };
        for (case, sql, left) in [
            ("delete", "DELETE FROM files WHERE file_id = 'h1'", 0),
            ("trash", "UPDATE files SET status = 'trashing' WHERE file_id = 'h1'", 0),
            ("rename or move", "UPDATE files SET path = 'docs/b.txt' WHERE file_id = 'h1'", 0),
            ("uploading", "UPDATE files SET status = 'uploading' WHERE file_id = 'h1'", 0),
            ("re-key", "UPDATE files SET file_id = 'h2' WHERE file_id = 'h1'", 0),
            ("size only", "UPDATE files SET size_bytes = 5 WHERE file_id = 'h1'", 1),
            ("same path", "UPDATE files SET path = 'a.txt' WHERE file_id = 'h1'", 1),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let db = StateDb::open(dir.path().join("state.db")).unwrap();
            db.upsert_file(&held_row("h1", "a.txt")).unwrap();
            db.hold_unsent_edit("h1", "base_unknown", "fp", 1).unwrap();
            db.test_execute(sql).unwrap();
            assert_eq!(held(&db), left, "{case}");
        }
    }

    /// Task 2190 (glegion's constraint 3): the feed's read of held edits is an index read: the
    /// held rows by `idx_held_edits_detected`, each file by its key, and `files` never scanned.
    #[test]
    fn task_2190_the_feed_reads_held_edits_by_index() {
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        let conn = db.0.lock().unwrap();
        let mut stmt = conn.prepare(&format!("EXPLAIN QUERY PLAN {LIST_HELD_EDITS_SQL}")).unwrap();
        let plan: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(3))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert!(plan.iter().any(|step| step.contains("USING INDEX idx_held_edits_detected")), "{plan:?}");
        assert!(plan.iter().any(|step| step.starts_with("SEARCH file")), "each file by its key: {plan:?}");
        assert!(!plan.iter().any(|step| step.starts_with("SCAN file")), "files never scanned: {plan:?}");
    }
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
        assert_eq!(
            (rows[0].file_name.as_str(), rows[0].direction.as_str()),
            ("b.txt", "down")
        );
        assert_eq!(
            (rows[1].file_name.as_str(), rows[1].direction.as_str()),
            ("a.txt", "up")
        );
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
            db.record_transfer_activity(transfer("up", &format!("f{i}.txt"), i as i64))
                .unwrap();
        }
        // Count the table itself: the read path also caps its own result, which would hide a
        // write path that forgot to prune.
        let stored: i64 =
            db.0.lock()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM transfer_activity", [], |row| row.get(0))
                .unwrap();
        assert_eq!(stored, TRANSFER_ACTIVITY_MAX_ROWS as i64, "107 written, 100 kept");
        let rows = db
            .list_recent_transfer_activity(TRANSFER_ACTIVITY_MAX_ROWS + 50)
            .unwrap();
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
        db.enqueue_operation(&queued_op("u1", OperationKind::UploadFile, "f1"))
            .unwrap();
        db.enqueue_operation(&queued_op("u2", OperationKind::UploadVersion, "f2"))
            .unwrap();
        db.enqueue_operation(&queued_op("u3", OperationKind::UploadFile, "f3"))
            .unwrap();
        db.enqueue_operation(&queued_op("r1", OperationKind::RenameFile, "f5"))
            .unwrap();
        db.record_operation_pause("u3", OperationPauseReason::Quota, Some("quota exceeded"), 5)
            .unwrap();

        let backlog = db.transfer_backlog(100).unwrap();
        assert_eq!(
            backlog.upload_files, 2,
            "u1 and u2 are due; u3 is paused; r1 is a rename"
        );
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
        db.enqueue_operation(&queued_op("u1", OperationKind::UploadFile, "f1"))
            .unwrap();
        // Failed once and backed off for ten minutes (from t=1000 to t=1600).
        let mut backing_off = queued_op("u2", OperationKind::UploadFile, "f2");
        backing_off.attempts = 1;
        backing_off.next_retry_at = 1_600;
        db.enqueue_operation(&backing_off).unwrap();

        let during = db.transfer_backlog(1_000).unwrap();
        assert_eq!(during.upload_files, 1, "only u1 is due; u2 waits for t=1600");
        assert_eq!(
            during.upload_bytes, 100,
            "the backed-off file's 5000 bytes are not in the total"
        );
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
        db.enqueue_operation(&queued_op("u2", OperationKind::UploadFile, "f2"))
            .unwrap();
        db.record_operation_pause("u2", OperationPauseReason::Auth, None, 5)
            .unwrap();
        let backlog = db.transfer_backlog(100).unwrap();
        assert_eq!(backlog.upload_files, 0);
        assert_eq!(backlog.paused_for_quota, 0);
        assert_eq!(backlog.queued_ops, 2);
    }

    /// Lead ruling F9: after a sign-in, the operations paused for `auth` are due again at once with their attempts kept,
    /// and the ones paused for quota, permission or a lock stay paused.
    #[test]
    fn only_the_operations_paused_for_auth_are_due_again_with_their_attempts_kept() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        let mut auth = queued_op("a1", OperationKind::CreateFolder, "f-a");
        auth.attempts = 2;
        auth.next_retry_at = 10_000;
        db.enqueue_operation(&auth).unwrap();
        for (id, reason) in [
            ("a1", OperationPauseReason::Auth),
            ("q1", OperationPauseReason::Quota),
            ("p1", OperationPauseReason::Permission),
            ("l1", OperationPauseReason::Locked),
        ] {
            if id != "a1" {
                db.enqueue_operation(&queued_op(id, OperationKind::UploadFile, id))
                    .unwrap();
            }
            db.record_operation_pause(id, reason, Some("HTTP 401 Unauthorized"), 5)
                .unwrap();
        }
        let mut unpaused = queued_op("d1", OperationKind::UploadFile, "f-d");
        unpaused.created_at = 2; // listed after a1, which was queued first
        db.enqueue_operation(&unpaused).unwrap();
        let due = |now| {
            db.list_due_operations(now)
                .unwrap()
                .into_iter()
                .map(|op| (op.op_id, op.attempts, op.next_retry_at))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            due(500),
            vec![("d1".to_string(), 0, 0)],
            "precondition: only the unpaused one is due"
        );

        assert_eq!(db.resume_operations_paused_for_auth(500).unwrap(), 1);

        assert_eq!(
            due(500),
            vec![("a1".to_string(), 2, 500), ("d1".to_string(), 0, 0)],
            "the auth-paused one is due now, its attempts kept"
        );
        let diagnostics = db.queue_diagnostics(500).unwrap();
        assert_eq!(
            diagnostics.paused_by_reason,
            BTreeMap::from([
                ("locked".to_string(), 1),
                ("permission".to_string(), 1),
                ("quota".to_string(), 1)
            ]),
            "every other pause stays"
        );
        assert_eq!(
            db.resume_operations_paused_for_auth(600).unwrap(),
            0,
            "nothing left to resume"
        );
        assert_eq!(due(500).len(), 2, "and a second call moves nothing");
    }

    /// 0.8.15: the tray's and Settings › Sync's queue summary. Waiting (due or backing off) and given-up operations are
    /// counted apart; paused ones are grouped by reason with the dates they were queued; a reason this build does not
    /// know is `other`; nothing in it is a name, a path or an error text. It changes nothing.
    #[test]
    fn queue_hold_summary_counts_waiting_given_up_and_held_by_reason_without_names() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        for (id, created_at) in [("a1", 1_759_579_200), ("a2", 1_759_586_400)] {
            let mut op = queued_op(id, OperationKind::CreateFolder, id);
            op.created_at = created_at;
            db.enqueue_operation(&op).unwrap();
            db.record_operation_pause(id, OperationPauseReason::Auth, Some("HTTP 401 for secret-folder"), 5)
                .unwrap();
        }
        let mut quota = queued_op("q1", OperationKind::UploadFile, "q1");
        quota.created_at = 1_760_000_000;
        db.enqueue_operation(&quota).unwrap();
        db.record_operation_pause("q1", OperationPauseReason::Quota, None, 5).unwrap();
        db.enqueue_operation(&queued_op("o1", OperationKind::UploadFile, "o1")).unwrap();
        db.0.lock()
            .unwrap()
            .execute("UPDATE operation_queue SET paused_reason = 'secret-folder' WHERE op_id = 'o1'", [])
            .unwrap();
        db.enqueue_operation(&queued_op("w1", OperationKind::UploadFile, "w1")).unwrap();
        let mut backing_off = queued_op("w2", OperationKind::UploadFile, "w2");
        backing_off.attempts = 3;
        backing_off.next_retry_at = 4_000_000_000;
        db.enqueue_operation(&backing_off).unwrap();
        let mut given_up = queued_op("g1", OperationKind::CreateFolder, "g1");
        given_up.attempts = 25;
        given_up.max_attempts = 25;
        db.enqueue_operation(&given_up).unwrap();
        let queued_before: i64 =
            db.0.lock().unwrap().query_row("SELECT COUNT(*) FROM operation_queue", [], |row| row.get(0)).unwrap();

        let summary = db.queue_hold_summary().unwrap();

        assert_eq!((summary.waiting, summary.gave_up), (2, 1));
        assert_eq!(
            summary.held,
            vec![
                HeldGroup { reason: "auth".into(), count: 2, oldest_at: 1_759_579_200, newest_at: 1_759_586_400 },
                HeldGroup { reason: "other".into(), count: 1, oldest_at: 1, newest_at: 1 },
                HeldGroup { reason: "quota".into(), count: 1, oldest_at: 1_760_000_000, newest_at: 1_760_000_000 },
            ]
        );
        let json = serde_json::to_string(&summary).unwrap();
        assert!(!json.contains("secret"), "{json}");
        let queued_after: i64 =
            db.0.lock().unwrap().query_row("SELECT COUNT(*) FROM operation_queue", [], |row| row.get(0)).unwrap();
        assert_eq!((queued_before, queued_after), (7, 7), "a read: nothing purged");
        assert_eq!(db.queue_diagnostics(500).unwrap().paused, 4, "and nothing resumed");
    }

    /// 0.8.15: the Selective sync tree reads `current_version` and `remote_updated_at` (whether the server knows the
    /// row); `list_files` does not carry the first.
    #[test]
    fn list_tree_rows_carries_the_server_version_of_every_row() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        {
            let conn = db.0.lock().unwrap();
            conn.execute(
                "INSERT INTO files (file_id, path, status, item_kind, current_version, parent_id, size_bytes, remote_updated_at)
                 VALUES ('ghost', 'Archive', 'local', 'folder', 0, NULL, 0, 0),
                        ('server', 'Archive', 'cloud_only', 'folder', 3, NULL, 0, 0),
                        ('f1', 'Archive/a.txt', 'local', 'file', 2, 'server', 12, 1761480000)",
                [],
            )
            .unwrap();
        }
        let rows = db.list_tree_rows().unwrap();
        let by_id = |id: &str| rows.iter().find(|row| row.file_id == id).unwrap().clone();
        assert_eq!(rows.len(), 3);
        assert_eq!(
            (by_id("ghost").current_version, by_id("ghost").remote_updated_at, by_id("ghost").status),
            (0, 0, FileStatus::Local)
        );
        assert_eq!(by_id("f1").remote_updated_at, 1_761_480_000);
        assert_eq!((by_id("server").current_version, by_id("server").item_kind), (3, ItemKind::Folder));
        assert_eq!(
            (by_id("f1").size_bytes, by_id("f1").parent_id.as_deref(), by_id("f1").item_kind),
            (12, Some("server"), ItemKind::File)
        );
    }

    /// Lead ruling, F9 review I-1: after a key replacement, the operations paused for `auth` are re-marked
    /// `key_replaced`. They stay queued and paused, with their attempts; the auth resume never makes them due again; the
    /// diagnostics count them under their own reason; every other pause, and an unpaused operation, is untouched.
    #[test]
    fn after_a_key_replacement_the_auth_paused_operations_are_kept_and_never_resumed() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        let mut auth = queued_op("a1", OperationKind::CreateFolder, "f-a");
        auth.attempts = 2;
        auth.next_retry_at = 10_000;
        db.enqueue_operation(&auth).unwrap();
        for (id, reason) in [
            ("a1", OperationPauseReason::Auth),
            ("a2", OperationPauseReason::Auth),
            ("q1", OperationPauseReason::Quota),
            ("p1", OperationPauseReason::Permission),
            ("l1", OperationPauseReason::Locked),
        ] {
            if id != "a1" {
                db.enqueue_operation(&queued_op(id, OperationKind::UploadFile, id))
                    .unwrap();
            }
            db.record_operation_pause(id, reason, Some("HTTP 401 Unauthorized"), 5)
                .unwrap();
        }
        let mut unpaused = queued_op("d1", OperationKind::UploadFile, "f-d");
        unpaused.created_at = 2;
        db.enqueue_operation(&unpaused).unwrap();
        let due = |now| {
            db.list_due_operations(now)
                .unwrap()
                .into_iter()
                .map(|op| op.op_id)
                .collect::<Vec<_>>()
        };

        assert_eq!(
            db.hold_operations_paused_for_auth_as_key_replaced(500).unwrap(),
            2,
            "both auth-paused operations"
        );
        let reasons = BTreeMap::from([
            ("key_replaced".to_string(), 2),
            ("locked".to_string(), 1),
            ("permission".to_string(), 1),
            ("quota".to_string(), 1),
        ]);
        let diagnostics = db.queue_diagnostics(500).unwrap();
        assert_eq!(
            diagnostics.paused_by_reason, reasons,
            "counted under their own reason; every other pause stays"
        );
        assert_eq!((diagnostics.queued, diagnostics.paused), (6, 5), "kept, not purged");

        assert_eq!(
            db.resume_operations_paused_for_auth(600).unwrap(),
            0,
            "the auth resume finds none"
        );
        assert_eq!(due(700), vec!["d1".to_string()], "never due again");
        assert_eq!(db.queue_diagnostics(700).unwrap().paused_by_reason, reasons);
        let attempts: i64 =
            db.0.lock()
                .unwrap()
                .query_row("SELECT attempts FROM operation_queue WHERE op_id = 'a1'", [], |row| {
                    row.get(0)
                })
                .unwrap();
        assert_eq!(attempts, 2, "its attempts are kept");
        assert_eq!(
            db.hold_operations_paused_for_auth_as_key_replaced(800).unwrap(),
            0,
            "a second call moves nothing"
        );
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
        db.enqueue_operation(&queued_op("op-1", OperationKind::UploadFile, "x"))
            .unwrap();
        db.record_operation_attempt("op-1", 1, 10, Some("copy of Board minutes 2026 failed"))
            .unwrap();
        let exported = serde_json::to_string(&db.queue_diagnostics(200).unwrap()).unwrap();
        assert!(!exported.contains("2026"), "{exported}");
        assert!(!exported.contains("Board"), "{exported}");
        assert!(exported.contains("failed"), "{exported}");
    }

    #[test]
    fn known_names_include_synced_paths_for_the_lifecycle_log() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        db.upsert_file(&FileEntry {
            file_id: "f1".into(),
            path: "Tax 2025/aangifte.pdf".into(),
            status: FileStatus::Local,
            size_bytes: 1,
            modified_at: 0,
            content_hash: None,
            remote_updated_at: 0,
            parent_id: None,
            item_kind: ItemKind::File,
        })
        .unwrap();
        // `redact_for_export` turns any unknown word into `[name]` by itself, so `aangifte` going
        // missing proves nothing. `2025` is a bare number its allow-list keeps: only the known-name
        // scan removes it. The control proves it survives without the names, so this can go red.
        let message = "could not open Tax 2025";
        let control = crate::diagnostic_redaction::redact_for_export(message, &KnownNames::new());
        assert!(
            control.text.contains("2025"),
            "control: a bare number must survive without names: {}",
            control.text
        );
        let names = db.known_names(&[]).unwrap();
        let out = crate::diagnostic_redaction::redact_for_export(message, &names);
        assert!(
            !out.text.contains("2025"),
            "the synced path was not a known name: {}",
            out.text
        );
        // The extra paths (the sync root) are known names too.
        let with_root = db.known_names(&["Beebeeb Sync 4417".to_string()]).unwrap();
        let out = crate::diagnostic_redaction::redact_for_export("could not open Beebeeb Sync 4417", &with_root);
        assert!(
            !out.text.contains("4417"),
            "an extra path was not a known name: {}",
            out.text
        );
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
        assert!(
            diagnostics.last_error.as_deref().unwrap().contains("failed"),
            "{exported}"
        );
        assert!(diagnostics.last_error_redactions >= 1, "{exported}");
        assert_eq!(
            diagnostics.last_error_code,
            Some(DiagnosticErrorCode::Other),
            "{exported}"
        );
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
        db.record_operation_attempt("op-1", 1, 10, Some("copy of Q3 2026 failed"))
            .unwrap();
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

    /// Spec 2026-10-10 §5.2 steps 7-8: a sign-out's purge can meet another writer on `state.db` (the keep step's close
    /// of Finder accepts, or a Finder change that reads that fence in its own write transaction). The purge reads
    /// before it writes. In a deferred transaction SQLite answers its first write "database is locked" at once while
    /// another connection holds the write lock (a reader that asks to write is not given the busy timeout), and a
    /// sign-out has deleted every file by then. The purge takes the write lock as it begins, so it waits as any other
    /// writer here does.
    #[test]
    fn a_purge_waits_for_another_writer_instead_of_failing() {
        let hold = std::time::Duration::from_millis(200);
        for _ in 0..5 {
            let dir = tempdir().unwrap();
            let path = dir.path().join("state.db");
            let db = StateDb::open(&path).unwrap();
            let writer = StateDb::open(&path).unwrap();
            db.enqueue_operation(&queued("w1", OperationKind::UploadVersion, "f1", Some("/staged/w1")))
                .unwrap();
            let (held, wait_held) = std::sync::mpsc::channel();
            let holder = std::thread::spawn(move || {
                let mut conn = writer.hold_lock_for_test();
                let tx = conn
                    .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                    .unwrap();
                tx.execute(
                    "INSERT INTO sync_state (key, value) VALUES (?1, '1')",
                    params![FINDER_ACCEPTS_CLOSED_KEY],
                )
                .unwrap();
                held.send(()).unwrap();
                std::thread::sleep(hold);
                let released = std::time::Instant::now();
                tx.commit().unwrap();
                released
            });
            wait_held.recv().unwrap();
            let started = std::time::Instant::now();
            let purge = db.purge_all_local_state();
            let waited = started.elapsed();
            let released = holder.join().unwrap();
            if started >= released {
                // This thread ran only after the hold had ended, so this purge never met the writer: once more.
                continue;
            }
            let purge = purge.unwrap_or_else(|error| {
                panic!("a purge that meets another writer waits for it: {error} after {waited:?}")
            });
            assert_eq!(
                (purge.queued_ops_purged, purge.payload_paths),
                (1, vec!["/staged/w1".to_string()]),
                "the purge that waited did its work"
            );
            let left: i64 = db
                .hold_lock_for_test()
                .query_row("SELECT COUNT(*) FROM operation_queue", [], |row| row.get(0))
                .unwrap();
            assert_eq!(left, 0, "nothing is left queued");
            return;
        }
        panic!("in 5 tries the purge never began while the other connection held the write lock");
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

        assert_eq!(touched.len(), 2);
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
    fn the_migration_is_additive_and_idempotent() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("state.db");
        // Build a round-3 database: today's schema without the round-4 columns.
        drop(StateDb::open(&path).unwrap());
        {
            let conn = rusqlite::Connection::open(&path).unwrap();
            for (table, column) in [
                ("files", "held_write_id"),
                ("files", "held_base"),
                ("files", "held_version"),
                ("files", "held_object_version_id"),
                ("files", "version_filled"),
                ("operation_queue", "write_id"),
                ("operation_queue", "write_origin"),
                ("operation_queue", "after_write_id"),
                ("operation_queue", "base_pending"),
                ("operation_queue", "claim_id"),
                ("operation_queue", "claimed_at"),
                ("upload_resume", "completed_version"),
                ("upload_resume", "completed_object_version_id"),
                ("upload_resume", "completed_mime_type"),
            ] {
                let _ = conn.execute(&format!("DROP INDEX IF EXISTS idx_{table}_{column}"), []);
                conn.execute(&format!("ALTER TABLE {table} DROP COLUMN {column}"), [])
                    .unwrap();
            }
            conn.execute("DROP TABLE id_aliases", []).unwrap();
            // The earlier schema had no index on operation_queue.file_id either.
            conn.execute("DROP INDEX IF EXISTS idx_operation_queue_file_id", [])
                .unwrap();
            conn.execute(
                "INSERT INTO files (file_id, path, status, size_bytes, current_version) VALUES ('f1', 'a.txt', 'local', 7, 3)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO operation_queue (op_id, kind, file_id, base_version) VALUES ('op1', 'upload_version', 'f1', 3)",
                [],
            )
            .unwrap();
        }
        // Opened twice: the second open must not fail on an existing column.
        drop(StateDb::open(&path).unwrap());
        let db = StateDb::open(&path).unwrap();

        let conn = rusqlite::Connection::open(&path).unwrap();
        let columns = |table: &str| -> Vec<String> {
            let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})")).unwrap();
            stmt.query_map([], |row| row.get::<_, String>(1))
                .unwrap()
                .map(Result::unwrap)
                .collect()
        };
        for column in [
            "held_write_id",
            "held_base",
            "held_version",
            "held_object_version_id",
            "version_filled",
        ] {
            assert!(columns("files").contains(&column.to_string()), "files.{column}");
        }
        for column in [
            "write_id",
            "write_origin",
            "after_write_id",
            "base_pending",
            "claim_id",
            "claimed_at",
        ] {
            assert!(
                columns("operation_queue").contains(&column.to_string()),
                "operation_queue.{column}"
            );
        }
        for column in [
            "completed_version",
            "completed_object_version_id",
            "completed_mime_type",
        ] {
            assert!(
                columns("upload_resume").contains(&column.to_string()),
                "upload_resume.{column}"
            );
        }
        assert!(columns("id_aliases").contains(&"provisional_id".to_string()));
        let indexes: Vec<String> = {
            let mut stmt = conn.prepare("PRAGMA index_list(operation_queue)").unwrap();
            stmt.query_map([], |row| row.get::<_, String>(1))
                .unwrap()
                .map(Result::unwrap)
                .collect()
        };
        let index_columns = |index: &str| -> Vec<String> {
            let mut stmt = conn.prepare(&format!("PRAGMA index_info({index})")).unwrap();
            stmt.query_map([], |row| row.get::<_, String>(2))
                .unwrap()
                .map(Result::unwrap)
                .collect()
        };
        for (index, column) in [
            ("idx_operation_queue_write_id", "write_id"),
            ("idx_operation_queue_after_write_id", "after_write_id"),
            ("idx_operation_queue_file_id", "file_id"),
        ] {
            assert!(indexes.contains(&index.to_string()), "{index} missing from {indexes:?}");
            assert_eq!(index_columns(index), vec![column.to_string()], "{index}");
        }

        // Rows are unchanged; the new columns read as their defaults.
        let row = db.get_file("f1").unwrap().unwrap();
        assert_eq!((row.path.as_str(), row.size_bytes), ("a.txt", 7));
        let presentation = db.item_presentation("f1").unwrap().unwrap();
        assert_eq!(presentation.contract.current_version, 3);
        assert!(presentation.held.is_none());
        assert!(!presentation.version_filled);
        assert_eq!(db.get_operation("op1").unwrap().unwrap().base_version, Some(3));
        let (base_pending, write_id): (i64, Option<String>) = conn
            .query_row(
                "SELECT base_pending, write_id FROM operation_queue WHERE op_id = 'op1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            (base_pending, write_id),
            (0, None),
            "the earlier op's base is known and it carries no write"
        );
        assert!(
            db.finder_write("op1").unwrap().is_none(),
            "an op without a write id is not a Finder write"
        );
    }

    /// The held columns are written together: the accept sets
    /// `held_write_id` and `held_base` in one statement, and a row with a write id but no
    /// base is an error, never read as a base of 0.
    #[test]
    fn held_columns_are_written_together_and_a_half_set_row_is_an_error() {
        fn accept(db: &StateDb, op_id: &str, file_id: &str, kind: FinderAcceptKind<'_>) -> Result<AcceptOutcome> {
            // The accept requires its copy's journal row (spec §8.7 S6), as production writes it.
            db.track_staged_payload("/staged/x", None, false)?;
            db.accept_finder_write(
                &FinderAccept {
                    retire_parked: None,
                    op_id,
                    file_id,
                    kind,
                    parent_id: None,
                    target_path: Some("a.txt"),
                    metadata_json: "{}",
                    payload_path: "/staged/x",
                    size_bytes: 1,
                    modified_at: 1,
                    backup_source_key: None,
                    now: 1,
                },
                &|_, _| Vec::new(),
            )
        }
        fn held_pair(db: &StateDb, file_id: &str) -> (Option<String>, Option<i64>) {
            db.0.lock()
                .unwrap()
                .query_row(
                    "SELECT held_write_id, held_base FROM files WHERE file_id = ?1",
                    params![file_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap()
        }
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        let row = |file_id: &str| FileEntry {
            file_id: file_id.into(),
            path: "a.txt".into(),
            status: FileStatus::Local,
            size_bytes: 1,
            modified_at: 1,
            content_hash: None,
            remote_updated_at: 0,
            parent_id: None,
            item_kind: ItemKind::File,
        };

        let created = row("created");
        let AcceptOutcome::Queued { token, .. } =
            accept(&db, "op-create", "created", FinderAcceptKind::Create { row: &created }).unwrap()
        else {
            panic!("a create is queued");
        };
        let (write_id, base) = held_pair(&db, "created");
        assert_eq!(base, Some(0), "a create holds b = 0, set with its write id");
        assert_eq!(format!("0:w{}", write_id.unwrap()), token);

        db.upsert_file(&row("versioned")).unwrap();
        let mut contract = db.get_file_contract_state("versioned").unwrap().unwrap();
        contract.current_version = 2;
        db.set_file_contract_state(&contract).unwrap();
        let AcceptOutcome::Queued { token, .. } = accept(
            &db,
            "op-modify",
            "versioned",
            FinderAcceptKind::Modify {
                incoming_base: Some("2"),
            },
        )
        .unwrap() else {
            panic!("a modify on a known base is queued");
        };
        let (write_id, base) = held_pair(&db, "versioned");
        assert_eq!(base, Some(2), "a modify holds its decided base, set with its write id");
        assert_eq!(format!("2:w{}", write_id.unwrap()), token);

        // A write id without its base can only come from a bug: reading it fails.
        db.0.lock()
            .unwrap()
            .execute("UPDATE files SET held_base = NULL WHERE file_id = 'versioned'", [])
            .unwrap();
        assert!(
            db.item_presentation("versioned").is_err(),
            "a NULL held_base is never read as a base of 0"
        );
        assert!(
            accept(
                &db,
                "op-after-bug",
                "versioned",
                FinderAcceptKind::Modify {
                    incoming_base: Some("2"),
                },
            )
            .is_err(),
            "no save is mapped through a half-set row"
        );
        assert_eq!(
            db.list_operations_for_file("versioned").unwrap().len(),
            1,
            "the refused accept queued nothing"
        );
    }

    #[test]
    fn item_presentation_reads_the_held_write_and_the_queue_facts() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        let held = "a".repeat(32);
        let other = "b".repeat(32);
        db.0.lock()
            .unwrap()
            .execute(
                "INSERT INTO files (file_id, path, status, current_version, current_object_version_id,
                                    held_write_id, held_base, held_version, held_object_version_id, version_filled)
                 VALUES ('f1', 'a.txt', 'local', 4, 'o4', ?1, 3, 4, 'o4', 1)",
                params![held],
            )
            .unwrap();
        assert!(
            db.item_presentation("missing").unwrap().is_none(),
            "no row, no presentation"
        );

        let presentation = db.item_presentation("f1").unwrap().unwrap();
        assert_eq!(presentation.contract.current_version, 4);
        assert_eq!(
            presentation.held,
            Some(crate::write_token::HeldWrite {
                write_id: held.clone(),
                base: 3,
                version: Some(4),
                object_version_id: Some("o4".into()),
            }),
            "the held columns map one to one"
        );
        assert!(presentation.version_filled);

        // (op_id, kind, file_id, write_id, attempts, max_attempts)
        type Op<'a> = (&'a str, &'a str, &'a str, Option<&'a str>, i64, i64);
        let check = |ops: &[Op<'_>], queued: bool, unparked: bool, why: &str| {
            {
                let conn = db.0.lock().unwrap();
                conn.execute("DELETE FROM operation_queue", []).unwrap();
                for (op_id, kind, file_id, write_id, attempts, max_attempts) in ops {
                    conn.execute(
                        "INSERT INTO operation_queue (op_id, kind, file_id, write_id, attempts, max_attempts)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                        params![op_id, kind, file_id, write_id, attempts, max_attempts],
                    )
                    .unwrap();
                }
            }
            let presentation = db.item_presentation("f1").unwrap().unwrap();
            assert_eq!(presentation.held_write_queued, queued, "held_write_queued: {why}");
            assert_eq!(
                presentation.unparked_finder_upload, unparked,
                "unparked_finder_upload: {why}"
            );
        };
        let h = Some(held.as_str());
        let o = Some(other.as_str());

        check(&[], false, false, "an empty queue");
        check(
            &[("op", "upload_version", "f1", h, 0, 5)],
            true,
            true,
            "a queued Finder upload of the held write",
        );
        check(
            &[("op", "upload_version", "f1", h, 4, 5)],
            true,
            true,
            "one attempt left is not parked",
        );
        check(
            &[("op", "upload_version", "f1", h, 5, 5)],
            true,
            false,
            "attempts = max_attempts is parked, and still queued",
        );
        check(
            &[("op", "upload_version", "f1", h, 6, 5)],
            true,
            false,
            "attempts past max_attempts is parked",
        );
        check(
            &[("op", "upload_file", "f1", o, 0, 5)],
            false,
            true,
            "another write's create counts as a Finder upload",
        );
        check(
            &[("op", "rename_file", "f1", o, 0, 5)],
            false,
            false,
            "only upload kinds are Finder uploads",
        );
        check(
            &[("op", "upload_version", "f1", None, 0, 5)],
            false,
            false,
            "an upload without a write id is not a Finder upload",
        );
        check(
            &[("op", "upload_version", "f2", o, 0, 5)],
            false,
            false,
            "another file's upload",
        );

        db.0.lock()
            .unwrap()
            .execute("UPDATE files SET held_write_id = NULL WHERE file_id = 'f1'", [])
            .unwrap();
        check(
            &[("op", "upload_version", "f1", h, 0, 5)],
            false,
            true,
            "no held write, nothing to find",
        );
        assert!(db.item_presentation("f1").unwrap().unwrap().held.is_none());
    }

    #[test]
    fn finder_write_maps_its_columns_and_reads_an_unknown_origin_as_earlier_build() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        db.0.lock()
            .unwrap()
            .execute_batch(
                "INSERT INTO operation_queue (op_id, kind, file_id, write_id, write_origin, after_write_id, base_pending)
                 VALUES ('minted', 'upload_version', 'f1', 'w1', 'minted', 'w0', 2),
                        ('earlier', 'upload_file', 'f1', 'w2', 'earlier_build', NULL, 0),
                        ('no-origin', 'upload_version', 'f1', 'w3', NULL, NULL, 0),
                        ('odd-origin', 'upload_version', 'f1', 'w4', 'Minted', NULL, 0),
                        ('plain', 'upload_version', 'f1', NULL, 'minted', 'w0', 1);",
            )
            .unwrap();

        assert_eq!(
            db.finder_write("minted").unwrap(),
            Some(FinderWrite {
                write_id: "w1".into(),
                origin: WriteOrigin::Minted,
                after_write_id: Some("w0".into()),
                base_pending: 2,
            })
        );
        assert_eq!(
            db.finder_write("earlier").unwrap(),
            Some(FinderWrite {
                write_id: "w2".into(),
                origin: WriteOrigin::EarlierBuild,
                after_write_id: None,
                base_pending: 0,
            })
        );
        assert_eq!(
            db.finder_write("no-origin").unwrap().unwrap().origin,
            WriteOrigin::EarlierBuild,
            "a write id without an origin is never treated as minted"
        );
        assert_eq!(
            db.finder_write("odd-origin").unwrap().unwrap().origin,
            WriteOrigin::EarlierBuild,
            "an origin this build does not know reads as earlier-build"
        );
        assert_eq!(
            db.finder_write("plain").unwrap(),
            None,
            "no write id, not a Finder write"
        );
        assert_eq!(db.finder_write("missing").unwrap(), None, "no op");
    }

    #[test]
    fn write_origin_round_trips_through_its_stored_form() {
        for origin in [WriteOrigin::Minted, WriteOrigin::EarlierBuild] {
            assert_eq!(WriteOrigin::from_db(Some(origin.as_str())), Some(origin), "{origin:?}");
        }
        // The stored strings are schema: a row one build writes, the next build reads.
        assert_eq!(WriteOrigin::Minted.as_str(), "minted");
        assert_eq!(WriteOrigin::EarlierBuild.as_str(), "earlier_build");
        assert_eq!(WriteOrigin::from_db(None), None);
        assert_eq!(WriteOrigin::from_db(Some("")), None);
        assert_eq!(WriteOrigin::from_db(Some("Minted")), None);
    }

    fn queued(op_id: &str, kind: OperationKind, file_id: &str, payload: Option<&str>) -> PendingOperation {
        PendingOperation {
            op_id: op_id.into(),
            kind,
            file_id: Some(file_id.into()),
            parent_id: None,
            target_path: None,
            metadata_json: None,
            payload_path: payload.map(str::to_string),
            base_version: None,
            base_object_version_id: None,
            attempts: 0,
            max_attempts: 25,
            next_retry_at: 0,
            last_error: None,
            backup_source_key: None,
            created_at: 0,
            updated_at: 0,
        }
    }

    /// T66, spec §10.2 (m-4c) and m-4a: on a re-upgrade after a downgrade, the held columns of a
    /// file with an upload queued without a write id are cleared before that upload is marked, so
    /// no token the system no longer holds is revived. A waiting op stores its token's `b`.
    #[test]
    fn a_re_upgrade_clears_stale_held_columns_and_waiting_ops_store_b() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed_own_row(&db, "downgraded", FileStatus::Local, 10);
        let mut contract = db.get_file_contract_state("downgraded").unwrap().unwrap();
        contract.current_version = 2;
        db.set_file_contract_state(&contract).unwrap();
        db.track_staged_payload("/staged/d", None, false).unwrap();
        // Borrowed parameters, not `&'static str`: the token below is an owned `String`.
        let accept = |op_id: &str, base: &str| {
            db.accept_finder_write(
                &FinderAccept {
                    retire_parked: None,
                    op_id,
                    file_id: "downgraded",
                    kind: FinderAcceptKind::Modify {
                        incoming_base: Some(base),
                    },
                    parent_id: None,
                    target_path: Some("d.txt"),
                    metadata_json: "{}",
                    payload_path: "/staged/d",
                    size_bytes: 1,
                    modified_at: 1,
                    backup_source_key: None,
                    now: 1,
                },
                &|_, _| Vec::new(),
            )
            .unwrap()
        };
        let AcceptOutcome::Queued { token, .. } = accept("w1", "2") else {
            panic!("queued")
        };
        let AcceptOutcome::Queued { .. } = accept("w2", &token) else {
            panic!("queued")
        };
        assert_eq!(
            db.get_operation("w2").unwrap().unwrap().base_version,
            Some(2),
            "m-4a: the waiting op stores its b"
        );
        let this_build = ["w1", "w2"].map(|op_id| db.finder_write(op_id).unwrap().unwrap());
        assert!(
            this_build.iter().all(|write| write.origin == WriteOrigin::Minted),
            "precondition: this build minted both"
        );
        // A downgraded build then queued an upload of the same file without a write id.
        db.enqueue_operation(&queued("older-build", OperationKind::UploadVersion, "downgraded", None))
            .unwrap();
        assert_eq!(
            db.mark_earlier_build_uploads().unwrap(),
            1,
            "only the upload without a write id is marked"
        );
        assert!(
            db.item_presentation("downgraded").unwrap().unwrap().held.is_none(),
            "m-4c: the held columns are cleared"
        );
        let marked = db.finder_write("older-build").unwrap().unwrap();
        assert_eq!(marked.origin, WriteOrigin::EarlierBuild);
        for (op_id, before) in ["w1", "w2"].iter().zip(&this_build) {
            assert_eq!(
                &db.finder_write(op_id).unwrap().unwrap(),
                before,
                "{op_id}: this build's write keeps its write id and stays minted"
            );
        }
        // The next engine start marks nothing: an op is marked once.
        assert_eq!(
            db.mark_earlier_build_uploads().unwrap(),
            0,
            "a second start marks nothing"
        );
        assert_eq!(
            db.finder_write("older-build").unwrap().unwrap(),
            marked,
            "the marked op keeps its write id"
        );
    }

    /// Spec §10.2 with the auth pauses (spec 2026-10-06, F9): an upload an earlier build queued that is
    /// paused for `auth` or `key_replaced` is marked like any other and keeps its pause. The marking
    /// adds a write id and the earlier-build origin; it never makes a paused op due and keeps its
    /// attempts and base. A same-account sign-in then resumes only the `auth` one, still marked;
    /// the `key_replaced` one stays kept and never sent.
    #[test]
    fn an_earlier_builds_paused_upload_keeps_its_pause_through_the_marking() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        let enqueue_paused = |op_id: &str, file_id: &str| {
            seed_own_row(&db, file_id, FileStatus::Uploading, 10);
            let mut op = queued(
                op_id,
                OperationKind::UploadVersion,
                file_id,
                Some(&format!("/staged/{op_id}")),
            );
            op.attempts = 2;
            op.base_version = Some(1);
            db.enqueue_operation(&op).unwrap();
            db.record_operation_pause(op_id, OperationPauseReason::Auth, Some("HTTP 401 Unauthorized"), 5)
                .unwrap();
        };
        // The key-replaced path re-marks every auth pause, so that one is paused first.
        enqueue_paused("key-replaced", "f-key");
        assert_eq!(db.hold_operations_paused_for_auth_as_key_replaced(6).unwrap(), 1);
        enqueue_paused("auth-paused", "f-auth");
        let paused = |op_id: &str| -> Option<String> {
            db.0.lock()
                .unwrap()
                .query_row(
                    "SELECT paused_reason FROM operation_queue WHERE op_id = ?1",
                    [op_id],
                    |row| row.get(0),
                )
                .unwrap()
        };

        assert_eq!(db.mark_earlier_build_uploads().unwrap(), 2, "both are marked");

        for op_id in ["auth-paused", "key-replaced"] {
            assert_eq!(
                db.finder_write(op_id).unwrap().expect("a write id").origin,
                WriteOrigin::EarlierBuild
            );
            let op = db.get_operation(op_id).unwrap().unwrap();
            assert_eq!(
                (op.attempts, op.base_version),
                (2, Some(1)),
                "{op_id}: attempts and base kept"
            );
        }
        assert_eq!(paused("auth-paused").as_deref(), Some("auth"), "the auth pause is kept");
        assert_eq!(
            paused("key-replaced").as_deref(),
            Some("key_replaced"),
            "the key-replaced pause is kept"
        );
        assert!(db.list_due_operations(i64::MAX).unwrap().is_empty(), "neither is due");

        assert_eq!(db.resume_operations_paused_for_auth(600).unwrap(), 1);
        let due: Vec<String> = db
            .list_due_operations(600)
            .unwrap()
            .into_iter()
            .map(|op| op.op_id)
            .collect();
        assert_eq!(due, vec!["auth-paused".to_string()], "only the auth one resumes");
        assert_eq!(
            db.finder_write("auth-paused").unwrap().unwrap().origin,
            WriteOrigin::EarlierBuild,
            "still marked after the resume"
        );
        assert_eq!(
            paused("key-replaced").as_deref(),
            Some("key_replaced"),
            "kept, never sent"
        );
    }

    #[test]
    fn a_restore_and_an_upload_of_one_file_wait_for_each_other_in_queue_order() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        db.enqueue_operation(&queued("u1", OperationKind::UploadVersion, "f", None))
            .unwrap();
        db.enqueue_operation(&queued("r", OperationKind::RestoreVersion, "f", None))
            .unwrap();
        db.enqueue_operation(&queued("u2", OperationKind::UploadVersion, "f", None))
            .unwrap();
        // A restore and an upload wait for each other only when the upload is a Finder write.
        db.set_write_id_for_test("u1", &"1".repeat(32));
        db.set_write_id_for_test("u2", &"2".repeat(32));

        assert!(
            matches!(db.claim_operation("r", 1).unwrap(), ClaimOutcome::Wait),
            "a restore waits for an earlier upload"
        );
        let ClaimOutcome::Claimed(u1) = db.claim_operation("u1", 1).unwrap() else {
            panic!("u1 is first")
        };
        assert!(db.finish_claimed("u1", &u1.claim_id, None).unwrap());
        assert!(
            matches!(db.claim_operation("u2", 1).unwrap(), ClaimOutcome::Wait),
            "an upload waits for an earlier restore"
        );
        assert!(matches!(db.claim_operation("r", 1).unwrap(), ClaimOutcome::Claimed(_)));

        // The watcher's and Windows' uploads carry no write id. A restore does not wait
        // for them, and they do not wait for a restore: round 3's upload-only order.
        db.enqueue_operation(&queued("g-u0", OperationKind::UploadVersion, "g", None))
            .unwrap();
        db.enqueue_operation(&queued("g-r", OperationKind::RestoreVersion, "g", None))
            .unwrap();
        db.enqueue_operation(&queued("g-u1", OperationKind::UploadVersion, "g", None))
            .unwrap();
        assert!(
            matches!(db.claim_operation("g-r", 1).unwrap(), ClaimOutcome::Claimed(_)),
            "a restore does not wait for an earlier upload without a write id"
        );
        let ClaimOutcome::Claimed(g_u0) = db.claim_operation("g-u0", 1).unwrap() else {
            panic!("g-u0 is first")
        };
        assert!(db.finish_claimed("g-u0", &g_u0.claim_id, None).unwrap());
        assert!(
            matches!(db.claim_operation("g-u1", 1).unwrap(), ClaimOutcome::Claimed(_)),
            "an upload without a write id does not wait for an earlier restore, still queued"
        );
        assert!(db.get_operation("g-r").unwrap().is_some());

        assert!(matches!(db.claim_operation("gone", 1).unwrap(), ClaimOutcome::Gone));
    }

    #[test]
    fn engine_start_clears_claims_and_releases_only_unreferenced_completed_payloads() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        db.enqueue_operation(&queued(
            "u1",
            OperationKind::UploadVersion,
            "f",
            Some("/staged/referenced"),
        ))
        .unwrap();
        let ClaimOutcome::Claimed(stale) = db.claim_operation("u1", 1).unwrap() else {
            panic!("claimable")
        };
        db.track_staged_payload("/staged/free", None, true).unwrap();
        db.track_staged_payload("/staged/referenced", None, true).unwrap();
        db.track_staged_payload("/staged/unfinished", None, false).unwrap();
        // A copy only a resume row references (Keep Mine's one-shot op has no queue row),
        // and one only a Windows finalization references.
        db.put_upload_resume(&UploadResume {
            op_id: "inline".into(),
            payload_path: "/staged/resumed".into(),
            payload_size: 1,
            payload_mtime_ns: 1,
            upload_session_id: "session".into(),
            server_file_id: "f".into(),
            object_version_id: "object".into(),
            chunk_size_bytes: 1,
            chunk_count: 1,
            acked_chunks: 0,
            metadata_applied: false,
            is_create: false,
            completed_version: None,
            completed_object_version_id: None,
            completed_mime_type: None,
        })
        .unwrap();
        db.track_staged_payload("/staged/resumed", None, true).unwrap();
        db.put_upload_finalization(&UploadFinalization {
            op_id: "finalizing".into(),
            local_file_id: "f".into(),
            server_file_id: "f".into(),
            target_path: "a.txt".into(),
            payload_path: "/staged/finalizing".into(),
            stamped: false,
        })
        .unwrap();

        let repair = db.engine_start_repair().unwrap();

        assert_eq!(repair.claims_cleared, 1);
        // The release journal is macOS-only: elsewhere nothing is released at engine start.
        #[cfg(target_os = "macos")]
        assert_eq!(repair.released_payloads, vec!["/staged/free".to_string()]);
        #[cfg(not(target_os = "macos"))]
        assert!(repair.released_payloads.is_empty(), "{:?}", repair.released_payloads);
        assert!(
            !db.record_attempt_claimed("u1", &stale.claim_id, 1, 0, None).unwrap(),
            "a claim from before the restart guards nothing"
        );
    }

    /// Spec §8.4 (the staging folder) and §8.7 S6: engine start unlinks a released copy wherever it was staged,
    /// by its journalled absolute path: in the data root, and in the old cache root that builds before the move
    /// used. A copy a queued op still owns stays in each root, with its journal row. (The release journal is
    /// macOS-only, so the test is too.)
    #[cfg(target_os = "macos")]
    #[test]
    fn engine_start_unlinks_released_copies_in_the_new_and_the_old_root_and_keeps_live_ones() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        let bases = crate::engine_bridge::FinderStagingBases::current();
        let mut released = Vec::new();
        let mut live = Vec::new();
        for (label, root) in [
            ("new", bases.durable_root().expect("the test sandbox has a data dir")),
            ("old", bases.cache_root()),
        ] {
            std::fs::create_dir_all(&root).unwrap();
            // Landed: marked released in the journal, and no op, resume row or finalization names it.
            let done = root.join(uuid::Uuid::new_v4().to_string());
            std::fs::write(&done, b"landed").unwrap();
            db.track_staged_payload(&done.to_string_lossy(), None, true).unwrap();
            // Still queued: an upload reads it.
            let queued_copy = root.join(uuid::Uuid::new_v4().to_string());
            std::fs::write(&queued_copy, b"waiting").unwrap();
            db.track_staged_payload(&queued_copy.to_string_lossy(), Some("/source"), false)
                .unwrap();
            db.enqueue_operation(&queued(
                &format!("live-{label}"),
                OperationKind::UploadVersion,
                &format!("f-{label}"),
                Some(&queued_copy.to_string_lossy()),
            ))
            .unwrap();
            released.push(done);
            live.push(queued_copy);
        }

        let repair = db.engine_start_repair().unwrap();
        let removed = crate::staged_payload::remove_released(&db, &repair.released_payloads);

        assert_eq!(removed, 2, "one released copy in each root");
        for path in &released {
            assert!(!path.exists(), "a released copy is unlinked: {path:?}");
        }
        for path in &live {
            assert!(path.exists(), "a copy a queued op owns stays: {path:?}");
        }
        let mut journalled: Vec<String> = db
            .staged_payloads_for_signout()
            .unwrap()
            .into_iter()
            .map(|(path, _, _)| path)
            .collect();
        journalled.sort();
        let mut expected: Vec<String> = live.iter().map(|path| path.to_string_lossy().into_owned()).collect();
        expected.sort();
        assert_eq!(journalled, expected, "only the live copies keep their journal rows");
        for path in &live {
            std::fs::remove_file(path).unwrap();
        }
    }

    /// The sign-in's auth resume (spec 2026-10-06, F9) never makes a parked Finder write due again. That holds
    /// for one its own attempt parked (`stale_base`, `payload_missing`) and for one the snapshot count parked
    /// (`base_unknown`) while it was paused for `auth`, so a parked write is never resumed as if it were only
    /// auth-paused. Each keeps its park: attempts used up, and its reason. Only the auth-paused write that never
    /// parked is due again.
    #[test]
    fn the_auth_resume_never_makes_a_parked_finder_write_due() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        for (op_id, reason) in [
            ("w-stale", ParkReason::StaleBase),
            ("w-missing", ParkReason::PayloadMissing),
        ] {
            db.enqueue_operation(&queued(
                op_id,
                OperationKind::UploadVersion,
                op_id,
                Some(&format!("/staged/{op_id}")),
            ))
            .unwrap();
            let ClaimOutcome::Claimed(claimed) = db.claim_operation(op_id, 1).unwrap() else {
                panic!("{op_id} is claimable")
            };
            assert!(db.park_claimed(op_id, &claimed.claim_id, reason, 2).unwrap());
            db.set_write_id_for_test(op_id, &format!("{op_id}-write"));
        }
        // Waiting for its base and paused for `auth`; the 10th snapshot without its base parks it.
        db.enqueue_operation(&queued(
            "w-base",
            OperationKind::UploadVersion,
            "w-base",
            Some("/staged/w-base"),
        ))
        .unwrap();
        db.0.lock()
            .unwrap()
            .execute(
                "UPDATE operation_queue SET base_pending = 1, write_id = 'w-base-write' WHERE op_id = 'w-base'",
                [],
            )
            .unwrap();
        db.record_operation_pause("w-base", OperationPauseReason::Auth, Some("HTTP 401 Unauthorized"), 3)
            .unwrap();
        for snapshot in 1..10 {
            assert!(db.note_snapshot_for_base_pending(10 + snapshot).unwrap().is_empty());
        }
        assert_eq!(
            db.note_snapshot_for_base_pending(20).unwrap(),
            vec![("w-base".to_string(), Some("w-base".to_string()))],
            "precondition: parked while paused for auth"
        );
        // Paused for `auth`, never parked.
        db.enqueue_operation(&queued(
            "w-auth",
            OperationKind::UploadVersion,
            "w-auth",
            Some("/staged/w-auth"),
        ))
        .unwrap();
        db.set_write_id_for_test("w-auth", "w-auth-write");
        db.record_operation_pause("w-auth", OperationPauseReason::Auth, Some("HTTP 401 Unauthorized"), 4)
            .unwrap();
        let due = || {
            db.list_due_operations(i64::MAX)
                .unwrap()
                .into_iter()
                .map(|op| op.op_id)
                .collect::<Vec<_>>()
        };
        assert!(due().is_empty(), "precondition: nothing is due");

        db.resume_operations_paused_for_auth(500).unwrap();

        let still_paused_for_auth: i64 =
            db.0.lock()
                .unwrap()
                .query_row(
                    "SELECT COUNT(*) FROM operation_queue WHERE paused_reason = 'auth'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
        assert_eq!(still_paused_for_auth, 0, "nothing paused for auth remains");
        assert_eq!(due(), vec!["w-auth".to_string()], "only the write that never parked");
        for (op_id, reason) in [
            ("w-stale", ParkReason::StaleBase),
            ("w-missing", ParkReason::PayloadMissing),
            ("w-base", ParkReason::BaseUnknown),
        ] {
            assert!(!due().contains(&op_id.to_string()), "{op_id} is not due");
            let op = db.get_operation(op_id).unwrap().expect("a parked write keeps its op");
            assert_eq!(op.attempts, op.max_attempts, "{op_id} stays parked");
            assert_eq!(
                op.last_error.as_deref(),
                Some(reason.as_str()),
                "{op_id} keeps its reason"
            );
            assert_eq!(
                op.payload_path.as_deref(),
                Some(format!("/staged/{op_id}").as_str()),
                "{op_id} keeps its bytes"
            );
        }
    }

    /// The key-replaced re-mark of an auth-paused Finder write keeps the write
    /// held. Its token still names the bytes the system holds; its op and its staged copy stay, with the copy's
    /// journal row; engine start finds nothing to unlink; and the auth resume never makes it due.
    #[test]
    fn a_key_replaced_hold_keeps_a_held_finder_write_and_its_staged_copy() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        db.upsert_file(&FileEntry {
            file_id: "held".into(),
            path: "a.txt".into(),
            status: FileStatus::Local,
            size_bytes: 1,
            modified_at: 1,
            content_hash: None,
            remote_updated_at: 1,
            parent_id: None,
            item_kind: ItemKind::File,
        })
        .unwrap();
        let mut contract = db.get_file_contract_state("held").unwrap().unwrap();
        contract.current_version = 2;
        db.set_file_contract_state(&contract).unwrap();
        let staged = dir.path().join("staged-copy");
        std::fs::write(&staged, b"the only copy of the save").unwrap();
        let staged_path = staged.to_string_lossy().into_owned();
        db.track_staged_payload(&staged_path, Some("/source"), false).unwrap();
        let AcceptOutcome::Queued { token, .. } = db
            .accept_finder_write(
                &FinderAccept {
                    retire_parked: None,
                    op_id: "op-held",
                    file_id: "held",
                    kind: FinderAcceptKind::Modify {
                        incoming_base: Some("2"),
                    },
                    parent_id: None,
                    target_path: Some("a.txt"),
                    metadata_json: "{}",
                    payload_path: &staged_path,
                    size_bytes: 25,
                    modified_at: 2,
                    backup_source_key: None,
                    now: 2,
                },
                &|_, _| Vec::new(),
            )
            .unwrap()
        else {
            panic!("the save is queued");
        };
        db.record_operation_pause("op-held", OperationPauseReason::Auth, Some("HTTP 401 Unauthorized"), 3)
            .unwrap();

        assert_eq!(db.hold_operations_paused_for_auth_as_key_replaced(500).unwrap(), 1);

        let presentation = db.item_presentation("held").unwrap().unwrap();
        assert_eq!(
            presentation.held.map(|held| held.token()),
            Some(token),
            "the write stays held under its token"
        );
        assert!(presentation.held_write_queued, "its op still carries the write");
        let op = db.get_operation("op-held").unwrap().expect("the op is kept");
        assert_eq!(op.payload_path.as_deref(), Some(staged_path.as_str()), "with its copy");
        let reason: Option<String> =
            db.0.lock()
                .unwrap()
                .query_row(
                    "SELECT paused_reason FROM operation_queue WHERE op_id = 'op-held'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
        assert_eq!(reason.as_deref(), Some("key_replaced"));
        assert_eq!(
            db.staged_payloads_for_signout().unwrap(),
            vec![(staged_path.clone(), Some("/source".to_string()), false)],
            "the copy's journal row stays, not released"
        );
        assert!(
            !db.engine_start_repair()
                .unwrap()
                .released_payloads
                .contains(&staged_path),
            "engine start finds nothing to unlink"
        );
        assert!(staged.exists(), "the staged copy is on disk");
        assert_eq!(db.resume_operations_paused_for_auth(600).unwrap(), 0);
        assert!(
            db.list_due_operations(i64::MAX).unwrap().is_empty(),
            "never resumed as if auth-paused"
        );
    }

    /// S2: a park or a pause written under an old claim,
    /// on a row that still exists, changes nothing. The claim id, not the op id alone,
    /// is the key.
    #[test]
    fn a_park_or_a_pause_under_a_stale_claim_changes_nothing() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        db.enqueue_operation(&queued("u1", OperationKind::UploadVersion, "f", Some("/staged/u1")))
            .unwrap();
        let ClaimOutcome::Claimed(old) = db.claim_operation("u1", 1).unwrap() else {
            panic!("claimable")
        };
        // The attempt under `old` ended, and a newer attempt claimed the op.
        assert!(
            db.record_attempt_claimed("u1", &old.claim_id, 1, 0, Some("boom"))
                .unwrap()
        );
        let ClaimOutcome::Claimed(current) = db.claim_operation("u1", 2).unwrap() else {
            panic!("claimable again")
        };
        assert_ne!(old.claim_id, current.claim_id);
        let before = db.get_operation("u1").unwrap().unwrap();

        assert!(
            !db.park_claimed("u1", &old.claim_id, ParkReason::StaleBase, 3).unwrap(),
            "a park under an old claim matches no row"
        );
        assert!(
            !db.record_pause_claimed("u1", &old.claim_id, OperationPauseReason::Quota, Some("quota"), 4)
                .unwrap(),
            "a pause under an old claim matches no row"
        );
        assert_eq!(
            db.get_operation("u1").unwrap().unwrap(),
            before,
            "the row is as the current claim left it"
        );
        assert_eq!(db.queue_diagnostics(i64::MAX).unwrap().paused, 0, "nothing paused");
        // The current claim still holds the op: its own outcome writes.
        assert!(
            db.park_claimed("u1", &current.claim_id, ParkReason::StaleBase, 5)
                .unwrap(),
            "the current claim is intact"
        );
    }

    /// A pass lists its ops once, so the claim skips an op that paused after the listing, as
    /// the listing itself would. It is claimable again once resumed.
    #[test]
    fn the_claim_skips_an_op_that_paused_after_the_pass_listed_it() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        db.enqueue_operation(&queued("u1", OperationKind::UploadVersion, "f", Some("/staged/u1")))
            .unwrap();
        assert_eq!(db.list_due_operations(i64::MAX).unwrap().len(), 1, "listed");
        db.record_operation_pause("u1", OperationPauseReason::Auth, Some("HTTP 401 Unauthorized"), 3)
            .unwrap();
        assert!(
            matches!(db.claim_operation("u1", 4).unwrap(), ClaimOutcome::Gone),
            "a paused op is not claimed"
        );
        let claim_id: Option<String> =
            db.0.lock()
                .unwrap()
                .query_row("SELECT claim_id FROM operation_queue WHERE op_id = 'u1'", [], |row| {
                    row.get(0)
                })
                .unwrap();
        assert_eq!(claim_id, None, "and holds no claim");
        assert_eq!(db.resume_operations_paused_for_auth(5).unwrap(), 1);
        assert!(
            matches!(db.claim_operation("u1", 6).unwrap(), ClaimOutcome::Claimed(_)),
            "claimable again once resumed"
        );
    }

    /// §8.6 rule 6: no park path parks an op whose completion is
    /// recorded. That covers the runner's park (the immediate stale-base arm, a missing
    /// payload, an unknown base), the snapshot count's park and the claim's parks (both
    /// `park_unclaimed_conn`), and the accept's park, which only ever parks the op it inserts.
    #[test]
    fn a_completed_op_is_never_parked() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed_own_row(&db, "f", FileStatus::Local, 10);
        let mut contract = db.get_file_contract_state("f").unwrap().unwrap();
        contract.current_version = 1;
        db.set_file_contract_state(&contract).unwrap();
        db.track_staged_payload("/staged/w", None, false).unwrap();
        let accept = |op_id: &str, base: &str| {
            db.accept_finder_write(
                &FinderAccept {
                    retire_parked: None,
                    op_id,
                    file_id: "f",
                    kind: FinderAcceptKind::Modify {
                        incoming_base: Some(base),
                    },
                    parent_id: None,
                    target_path: Some("f.txt"),
                    metadata_json: "{}",
                    payload_path: "/staged/w",
                    size_bytes: 3,
                    modified_at: 20,
                    backup_source_key: None,
                    now: 20,
                },
                &|_, _| Vec::new(),
            )
        };
        assert!(matches!(accept("w", "1").unwrap(), AcceptOutcome::Queued { .. }));
        let ClaimOutcome::Claimed(claimed) = db.claim_operation("w", 21).unwrap() else {
            panic!("claimable")
        };
        assert!(
            db.put_upload_resume_claimed(
                &UploadResume {
                    op_id: "w".into(),
                    payload_path: "/staged/w".into(),
                    payload_size: 3,
                    payload_mtime_ns: 1,
                    upload_session_id: "session-w".into(),
                    server_file_id: "f".into(),
                    object_version_id: "object-w".into(),
                    chunk_size_bytes: 3,
                    chunk_count: 1,
                    acked_chunks: 1,
                    metadata_applied: true,
                    is_create: false,
                    completed_version: None,
                    completed_object_version_id: None,
                    completed_mime_type: None,
                },
                &claimed.claim_id,
            )
            .unwrap()
        );
        assert!(
            db.record_completion_claimed("w", &claimed.claim_id, 2, "object-2", None)
                .unwrap()
        );
        let unparked = |why: &str| {
            let op = db.get_operation("w").unwrap().expect(why);
            assert!(op.attempts < op.max_attempts, "{why}: parked");
        };

        // The runner's park.
        for reason in [
            ParkReason::StaleBase,
            ParkReason::PayloadMissing,
            ParkReason::BaseUnknown,
        ] {
            assert!(
                !db.park_claimed("w", &claimed.claim_id, reason, 22).unwrap(),
                "the runner parked a completed op: {}",
                reason.as_str()
            );
            unparked(reason.as_str());
        }
        // The snapshot count's park, at the 10th snapshot without a base.
        db.0.lock()
            .unwrap()
            .execute("UPDATE operation_queue SET base_pending = 10 WHERE op_id = 'w'", [])
            .unwrap();
        assert!(
            db.note_snapshot_for_base_pending(23).unwrap().is_empty(),
            "the snapshot count parked a completed op"
        );
        unparked("the snapshot count");
        // The claim's parks: W names a predecessor that no op carries (S5).
        db.0.lock()
            .unwrap()
            .execute(
                "UPDATE operation_queue SET base_pending = 0, after_write_id = 'gone', claim_id = NULL
                 WHERE op_id = 'w'",
                [],
            )
            .unwrap();
        assert!(
            matches!(db.claim_operation("w", 24).unwrap(), ClaimOutcome::Claimed(_)),
            "claimed for its landing, never parked"
        );
        unparked("the claim");
        // The accept's park (rule 6a′) only parks the op it inserts: one naming W's op id fails.
        assert!(accept("w", "0").is_err(), "the accept never touches an existing op");
        unparked("the accept");
        assert_eq!(
            db.get_upload_resume("w").unwrap().unwrap().completed_version,
            Some(2),
            "the completion is kept"
        );
    }

    /// Across a mixed pair (one op with a write id and
    /// one without) the landing re-keys but never rebases.
    /// - (a) A Finder create lands while an op without a write id is queued under its
    ///   provisional id: that op moves to the server id, and its base stays `None`.
    /// - (b) An upload without a write id lands while a Finder save on the same base is queued:
    ///   the save keeps its base (the server then refuses it and it parks with its bytes),
    ///   while another op without a write id is rebased, as in round 3.
    ///
    /// The spec states this behaviour (§8.1, the mixed pair).
    #[test]
    fn a_landing_rekeys_a_mixed_pair_but_never_rebases_across_it() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        let rekey = |op: &PendingOperation, _: &str| -> anyhow::Result<Option<String>> { Ok(op.metadata_json.clone()) };
        let landing = |op_id: &'static str, claim_id: &'static str, write_id: Option<&'static str>| LandingInput {
            op_id,
            claim_id: Some(claim_id),
            write_id,
            local_file_id: "",
            server_file_id: "",
            target_path: None,
            parent_id: None,
            landed_base: None,
            produced_version: 0,
            produced_object_version_id: "",
            size_bytes: 3,
            content_type: None,
            mime_type: None,
            completed_payload: None,
            completed_payload_hash: None,
            finalization: None,
            now: 20,
        };

        // (a)
        seed_own_row(&db, "provisional", FileStatus::Uploading, 10);
        db.0.lock()
            .unwrap()
            .execute_batch(
                r#"INSERT INTO operation_queue (op_id, kind, file_id, metadata_json, base_version, write_id,
                                               write_origin, claim_id, attempts, max_attempts)
                   VALUES ('create', 'upload_version', 'provisional', '{"operation":"create_file"}', NULL,
                           'w-create', 'minted', 'claim-c', 0, 25),
                          ('plain', 'upload_version', 'provisional', '{"operation":"upload_version"}', NULL,
                           NULL, NULL, NULL, 0, 25);"#,
            )
            .unwrap();
        let landed = db
            .apply_landing(
                &LandingInput {
                    local_file_id: "provisional",
                    server_file_id: "server",
                    produced_version: 1,
                    produced_object_version_id: "object-1",
                    ..landing("create", "claim-c", Some("w-create"))
                },
                &rekey,
            )
            .unwrap()
            .expect("the claim holds");
        assert_eq!(landed.resolved_successors, 0);
        let plain = db.get_operation("plain").unwrap().unwrap();
        assert_eq!(
            plain.file_id.as_deref(),
            Some("server"),
            "(a) re-keyed to the server id"
        );
        assert_eq!(plain.base_version, None, "(a) not rebased across the mixed pair");

        // (b)
        seed_own_row(&db, "f", FileStatus::Uploading, 10);
        let mut contract = db.get_file_contract_state("f").unwrap().unwrap();
        contract.current_version = 1;
        db.set_file_contract_state(&contract).unwrap();
        db.0.lock()
            .unwrap()
            .execute_batch(
                "INSERT INTO operation_queue (op_id, kind, file_id, metadata_json, base_version, write_id,
                                              write_origin, claim_id, attempts, max_attempts)
                 VALUES ('x', 'upload_version', 'f', '{}', 1, NULL, NULL, 'claim-x', 0, 25),
                        ('n', 'upload_version', 'f', '{}', 1, 'w-n', 'minted', NULL, 0, 25),
                        ('y', 'upload_version', 'f', '{}', 1, NULL, NULL, NULL, 0, 25);",
            )
            .unwrap();
        let landed = db
            .apply_landing(
                &LandingInput {
                    local_file_id: "f",
                    server_file_id: "f",
                    landed_base: Some(1),
                    produced_version: 2,
                    produced_object_version_id: "object-2",
                    ..landing("x", "claim-x", None)
                },
                &rekey,
            )
            .unwrap()
            .expect("the claim holds");
        assert_eq!(landed.resolved_successors, 1, "(b) only the op without a write id");
        assert_eq!(
            db.get_operation("n").unwrap().unwrap().base_version,
            Some(1),
            "(b) the Finder save keeps its base"
        );
        assert_eq!(
            db.get_operation("y").unwrap().unwrap().base_version,
            Some(2),
            "(b) round 3's equal-base rule"
        );
    }

    /// Task 2204 S0.3 (review-155): the pull read the row (at `a.txt`) before a Windows rename
    /// committed its re-key; its write, which still names `a.txt` and the old parent, keeps the
    /// row where the rename put it.
    #[test]
    fn task_2204_a_pull_write_after_a_rename_keeps_the_new_path() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed_own_row(&db, "f", FileStatus::Local, 10);
        let stale = db.get_file("f").unwrap().unwrap();
        db.0.lock()
            .unwrap()
            .execute_batch(
                r#"INSERT INTO operation_queue (op_id, kind, file_id, target_path, metadata_json, attempts, max_attempts)
                   VALUES ('rename', 'move_file', 'f', 'b.txt', '{"local_rename":true}', 0, 25);
                   UPDATE files SET path = 'docs/b.txt', parent_id = 'docs' WHERE file_id = 'f';"#,
            )
            .unwrap();
        let written = db
            .upsert_server_file_row(&stale, |contract| contract.parent_id = None)
            .unwrap();
        assert_eq!(written, "docs/b.txt");
        assert_eq!(db.get_file("f").unwrap().unwrap().path, "docs/b.txt", "the rename's path stands");
        assert_eq!(
            db.get_file_contract_state("f").unwrap().unwrap().parent_id.as_deref(),
            Some("docs"),
            "and its parent"
        );
        db.remove_operation("rename").unwrap();
        db.upsert_server_file_row(&stale, |contract| contract.parent_id = None).unwrap();
        assert_eq!(db.get_file("f").unwrap().unwrap().path, "f.txt", "nothing queued: the server's row is written");
    }

    /// Task 2204 S0.2 (review-128 F4): a sync-folder create renamed before it landed keeps the
    /// row's path and parent, and its finalization journal row names the row's path, not the
    /// op's queue-time one. Only an op `handle_rename` queued (`local_rename`) holds them.
    #[test]
    fn task_2204_a_landing_keeps_a_renamed_row_s_path_and_journal() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed_own_row(&db, "provisional", FileStatus::Uploading, 10);
        let mut row = db.get_file("provisional").unwrap().unwrap();
        row.path = "docs/b.txt".into();
        db.upsert_file(&row).unwrap();
        let mut contract = db
            .get_file_contract_state("provisional")
            .unwrap()
            .unwrap_or_else(|| default_contract("provisional"));
        contract.parent_id = Some("docs-folder".into());
        db.set_file_contract_state(&contract).unwrap();
        db.0.lock()
            .unwrap()
            .execute_batch(
                r#"INSERT INTO operation_queue (op_id, kind, file_id, target_path, metadata_json, payload_path,
                                               claim_id, attempts, max_attempts)
                   VALUES ('create', 'upload_version', 'provisional', 'a.txt', '{"operation":"create_file"}',
                           '/staged/create', 'claim-c', 0, 25),
                          ('rename', 'move_file', 'provisional', 'b.txt',
                           '{"operation":"metadata_update","local_rename":true}', NULL, NULL, 0, 25);"#,
            )
            .unwrap();
        let finalization = UploadFinalization {
            op_id: "create".into(),
            local_file_id: "provisional".into(),
            server_file_id: "server".into(),
            target_path: "a.txt".into(),
            payload_path: "/staged/create".into(),
            stamped: false,
        };
        let input = LandingInput {
            op_id: "create",
            claim_id: Some("claim-c"),
            write_id: None,
            local_file_id: "provisional",
            server_file_id: "server",
            target_path: Some("a.txt"),
            parent_id: None,
            landed_base: None,
            produced_version: 1,
            produced_object_version_id: "object-1",
            size_bytes: 3,
            content_type: None,
            mime_type: None,
            completed_payload: Some("/staged/create"),
            completed_payload_hash: None,
            finalization: Some(&finalization),
            now: 20,
        };
        let rekey = |op: &PendingOperation, _: &str| -> anyhow::Result<Option<String>> { Ok(op.metadata_json.clone()) };
        db.apply_landing(&input, &rekey).unwrap().expect("the claim holds");
        let landed = db.get_file("server").unwrap().unwrap();
        assert_eq!(landed.path, "docs/b.txt", "the row stays where the file is");
        assert_eq!(
            db.get_file_contract_state("server").unwrap().unwrap().parent_id.as_deref(),
            Some("docs-folder"),
            "and in its folder"
        );
        let journal = db.upload_finalizations().unwrap();
        assert_eq!(journal.len(), 1);
        assert_eq!(journal[0].target_path, "docs/b.txt", "the journal names the row's path");
        assert_eq!(
            db.get_operation("rename").unwrap().unwrap().file_id.as_deref(),
            Some("server"),
            "the rename now names the server id"
        );
    }

    /// Windows: a create's finalization journal row is written in the
    /// landing's own transaction. Either the landing commits with its journal row, or nothing
    /// of it does and the op stays for the retry: here the journal write fails (a row already
    /// holds its key, as a full disk would fail it), and the landing rolls back whole.
    #[test]
    fn a_landing_journals_its_finalization_in_its_own_transaction() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed_own_row(&db, "provisional", FileStatus::Uploading, 10);
        db.0.lock()
            .unwrap()
            .execute_batch(
                r#"INSERT INTO operation_queue (op_id, kind, file_id, target_path, metadata_json, payload_path,
                                               claim_id, attempts, max_attempts)
                   VALUES ('create', 'upload_version', 'provisional', 'p.txt', '{"operation":"create_file"}',
                           '/staged/create', 'claim-c', 0, 25);
                   INSERT INTO upload_finalizations (op_id, local_file_id, server_file_id, target_path, payload_path)
                   VALUES ('create', 'other', 'other', 'other.txt', '/staged/other');"#,
            )
            .unwrap();
        db.put_upload_resume(&UploadResume {
            op_id: "create".into(),
            payload_path: "/staged/create".into(),
            payload_size: 3,
            payload_mtime_ns: 1,
            upload_session_id: "session-c".into(),
            server_file_id: "server".into(),
            object_version_id: "object-1".into(),
            chunk_size_bytes: 3,
            chunk_count: 1,
            acked_chunks: 1,
            metadata_applied: true,
            is_create: true,
            completed_version: None,
            completed_object_version_id: None,
            completed_mime_type: None,
        })
        .unwrap();
        let finalization = UploadFinalization {
            op_id: "create".into(),
            local_file_id: "provisional".into(),
            server_file_id: "server".into(),
            target_path: "p.txt".into(),
            payload_path: "/staged/create".into(),
            stamped: false,
        };
        let input = LandingInput {
            op_id: "create",
            claim_id: Some("claim-c"),
            write_id: None,
            local_file_id: "provisional",
            server_file_id: "server",
            target_path: Some("p.txt"),
            parent_id: None,
            landed_base: None,
            produced_version: 1,
            produced_object_version_id: "object-1",
            size_bytes: 3,
            content_type: None,
            mime_type: None,
            completed_payload: Some("/staged/create"),
            completed_payload_hash: None,
            finalization: Some(&finalization),
            now: 20,
        };
        let rekey = |op: &PendingOperation, _: &str| -> anyhow::Result<Option<String>> { Ok(op.metadata_json.clone()) };

        assert!(db.apply_landing(&input, &rekey).is_err(), "the journal write fails");
        assert!(
            db.get_operation("create").unwrap().is_some(),
            "nothing of the landing committed: the op stays for the retry"
        );
        assert!(db.get_upload_resume("create").unwrap().is_some(), "with its resume row");
        assert!(
            db.get_file("provisional").unwrap().is_some(),
            "the provisional row stays"
        );
        assert!(db.get_file("server").unwrap().is_none(), "no server row");

        db.0.lock()
            .unwrap()
            .execute("DELETE FROM upload_finalizations WHERE op_id = 'create'", [])
            .unwrap();
        assert!(db.apply_landing(&input, &rekey).unwrap().is_some(), "the retry lands");
        assert_eq!(
            db.upload_finalizations().unwrap(),
            vec![finalization.clone()],
            "the journal row is in the landing's commit"
        );
        assert!(db.get_operation("create").unwrap().is_none());
        assert!(db.get_upload_resume("create").unwrap().is_none());
        assert!(db.get_file("server").unwrap().is_some());
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

    /// A row the server holds at `version`, under `object`.
    fn seed_restorable_row(db: &StateDb, file_id: &str, version: i64, object: &str) {
        db.upsert_file(&FileEntry {
            file_id: file_id.into(),
            path: "notes.txt".into(),
            status: FileStatus::Local,
            size_bytes: 1,
            modified_at: 1,
            content_hash: None,
            remote_updated_at: 1,
            parent_id: None,
            item_kind: ItemKind::File,
        })
        .unwrap();
        let mut contract = db.get_file_contract_state(file_id).unwrap().unwrap();
        contract.current_version = version;
        contract.current_object_version_id = Some(object.into());
        db.set_file_contract_state(&contract).unwrap();
    }

    fn version_and_object(db: &StateDb, file_id: &str) -> (i64, Option<String>) {
        let contract = db.get_file_contract_state(file_id).unwrap().unwrap();
        (contract.current_version, contract.current_object_version_id)
    }

    /// Spec §5.4 row 10: a restore reply without `version_number`
    /// still tells the system the content changed, and a snapshot fills the version.
    #[test]
    fn a_restore_reply_without_a_version_records_the_change_and_requests_a_snapshot() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed_restorable_row(&db, "restored", 2, "object-v2");
        #[cfg(target_os = "macos")]
        assert_eq!(
            db.peek_resnapshot_request().unwrap(),
            None,
            "no snapshot is pending before"
        );
        #[cfg(not(target_os = "macos"))]
        assert!(!db.take_needs_resnapshot().unwrap(), "no snapshot is pending before");
        let (_, anchor) = db.list_file_changes(None).unwrap().unwrap();

        db.apply_restore_response("restored", None, None).unwrap();

        let (changes, _) = db.list_file_changes(anchor.as_deref()).unwrap().unwrap();
        let modified = changes
            .iter()
            .filter(|change| change.file_id == "restored" && change.kind == FpChangeKind::Modified)
            .count();
        assert_eq!(modified, 1, "the system is told the content changed: {changes:?}");
        #[cfg(target_os = "macos")]
        assert!(
            db.peek_resnapshot_request().unwrap().is_some(),
            "a snapshot fills the version the reply left out"
        );
        #[cfg(not(target_os = "macos"))]
        assert!(
            db.take_needs_resnapshot().unwrap(),
            "a snapshot fills the version the reply left out"
        );
        assert_eq!(
            version_and_object(&db, "restored"),
            (2, Some("object-v2".to_string())),
            "nothing is guessed"
        );
    }

    /// The server's legacy restore branch answers no object version
    /// and keeps its own (VER:663-674), so the row keeps its own too.
    #[test]
    fn a_restore_reply_without_an_object_version_keeps_the_rows() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed_restorable_row(&db, "legacy", 2, "object-v2");

        db.apply_restore_response("legacy", Some(3), None).unwrap();
        assert_eq!(
            version_and_object(&db, "legacy"),
            (3, Some("object-v2".to_string())),
            "the legacy branch's reply keeps the row's object version"
        );

        db.apply_restore_response("legacy", Some(4), Some("object-v4")).unwrap();
        assert_eq!(
            version_and_object(&db, "legacy"),
            (4, Some("object-v4".to_string())),
            "a reply that names one replaces it"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_resnapshot_request_survives_a_request_made_while_it_runs() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        assert_eq!(db.peek_resnapshot_request().unwrap(), None);
        db.request_resnapshot().unwrap();
        let seen = db.peek_resnapshot_request().unwrap().expect("requested");
        db.request_resnapshot().unwrap(); // a request made during the bootstrap
        assert!(
            !db.clear_resnapshot_request(seen).unwrap(),
            "the newer request survives"
        );
        let again = db.peek_resnapshot_request().unwrap().expect("still requested");
        assert!(db.clear_resnapshot_request(again).unwrap());
        assert_eq!(db.peek_resnapshot_request().unwrap(), None);
    }

    #[cfg(not(target_os = "macos"))]
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

    #[cfg(target_os = "macos")]
    #[test]
    fn engine_start_requests_a_snapshot_for_version_zero_rows() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed_own_row(&db, "versioned", FileStatus::Local, 10);
        let mut contract = db.get_file_contract_state("versioned").unwrap().unwrap();
        contract.current_version = 4;
        db.set_file_contract_state(&contract).unwrap();
        assert!(!db.engine_start_repair().unwrap().resnapshot_requested);
        assert_eq!(db.peek_resnapshot_request().unwrap(), None);

        seed_own_row(&db, "version-zero", FileStatus::Local, 20); // learned from a legacy file_create op
        assert!(db.engine_start_repair().unwrap().resnapshot_requested);
        assert!(db.peek_resnapshot_request().unwrap().is_some());
    }

    /// The tick clears exactly the request value it read (spec §6.3.2). A shared helper, on
    /// every target: one request reads 1 from the counter (macOS) and from the flag (elsewhere).
    #[test]
    fn a_resnapshot_request_is_cleared_only_by_the_value_read() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        assert_eq!(db.peek_resnapshot_request().unwrap(), None);
        assert!(!db.clear_resnapshot_request(1).unwrap(), "nothing to clear");
        db.request_resnapshot().unwrap();
        assert_eq!(db.peek_resnapshot_request().unwrap(), Some(1));
        assert!(!db.clear_resnapshot_request(2).unwrap(), "another value clears nothing");
        assert_eq!(db.peek_resnapshot_request().unwrap(), Some(1));
        assert!(db.clear_resnapshot_request(1).unwrap());
        assert_eq!(db.peek_resnapshot_request().unwrap(), None);
    }

    /// §6.3.3: `base_pending` is 1 + the successful snapshots that did not give the op its
    /// base; the 10th parks it `base_unknown` through the claim's park statement, and it
    /// asks for no more. A shared helper: it runs on every target.
    #[test]
    fn base_pending_counts_snapshots_and_parks_at_the_tenth() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        assert!(!db.has_base_pending_uploads().unwrap());
        db.0.lock()
            .unwrap()
            .execute_batch(
                "INSERT INTO operation_queue (op_id, kind, file_id, write_id, write_origin, base_version,
                                              base_pending, attempts, max_attempts)
                 VALUES ('waiting', 'upload_version', 'f1', 'w1', 'minted', 0, 1, 0, 25),
                        ('known', 'upload_version', 'f2', 'w2', 'minted', 3, 0, 0, 25);",
            )
            .unwrap();
        assert!(db.has_base_pending_uploads().unwrap());
        for snapshot in 1..10 {
            assert!(
                db.note_snapshot_for_base_pending(100 + snapshot).unwrap().is_empty(),
                "not parked after {snapshot} snapshots"
            );
        }
        assert_eq!(
            db.finder_write("waiting").unwrap().unwrap().base_pending,
            10,
            "1 + nine misses"
        );
        assert_eq!(
            db.note_snapshot_for_base_pending(200).unwrap(),
            vec![("waiting".to_string(), Some("f1".to_string()))]
        );
        let parked = db.get_operation("waiting").unwrap().unwrap();
        assert_eq!(parked.attempts, parked.max_attempts, "parked at the 10th");
        assert_eq!(parked.last_error.as_deref(), Some("base_unknown"));
        let class: Option<String> =
            db.0.lock()
                .unwrap()
                .query_row(
                    "SELECT last_error_class FROM operation_queue WHERE op_id = 'waiting'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
        assert_eq!(
            class.as_deref(),
            Some("base_unknown"),
            "the claim's park statement: a save on its token takes over and parks the same way"
        );
        assert_eq!(db.finder_write("waiting").unwrap().unwrap().base_pending, 0);
        assert!(
            !db.has_base_pending_uploads().unwrap(),
            "a parked op asks for no more snapshots"
        );
        assert_eq!(
            db.get_operation("known").unwrap().unwrap().attempts,
            0,
            "an op with a known base is not counted"
        );
        assert!(db.note_snapshot_for_base_pending(300).unwrap().is_empty());
    }

    /// §6.3.2: the fill and the raise leave Conflict and Trashing rows alone, and on a row
    /// with a queued write the raise changes the version only (the size is that write's).
    /// A shared helper: it runs on every target.
    #[test]
    fn the_snapshot_version_skips_conflict_and_trashing_rows_and_keeps_a_queued_writes_size() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        let at = |file_id: &str, status: FileStatus, version: i64| {
            seed_own_row(&db, file_id, status, 10);
            let mut contract = db.get_file_contract_state(file_id).unwrap().unwrap();
            contract.current_version = version;
            contract.current_object_version_id = Some(format!("object-{file_id}"));
            db.set_file_contract_state(&contract).unwrap();
        };
        at("conflicted", FileStatus::Conflict, 1);
        at("trashing", FileStatus::Trashing, 0);
        at("queued", FileStatus::Uploading, 1);
        at("plain", FileStatus::Local, 1);
        db.enqueue_operation(&PendingOperation {
            op_id: "op-queued".into(),
            kind: OperationKind::UploadVersion,
            file_id: Some("queued".into()),
            parent_id: None,
            target_path: Some("queued.txt".into()),
            metadata_json: Some("{}".into()),
            payload_path: None,
            base_version: Some(1),
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
        db.set_write_id_for_test("op-queued", "w-queued");

        let version = |file_id: &str| {
            let contract = db.get_file_contract_state(file_id).unwrap().unwrap();
            let size = db.get_file(file_id).unwrap().unwrap().size_bytes;
            (contract.current_version, size, contract.current_object_version_id)
        };
        assert_eq!(
            db.apply_snapshot_version("conflicted", 3, 99, false).unwrap(),
            SnapshotVersion::Unchanged
        );
        assert_eq!(
            version("conflicted"),
            (1, 1, Some("object-conflicted".into())),
            "a Conflict row keeps its own"
        );
        assert_eq!(
            db.apply_snapshot_version("trashing", 1, 99, false).unwrap(),
            SnapshotVersion::Unchanged
        );
        assert_eq!(version("trashing").0, 0, "a Trashing row is leaving");
        assert_eq!(
            db.apply_snapshot_version("queued", 3, 99, false).unwrap(),
            SnapshotVersion::Raised { old: 1, new: 3 }
        );
        assert_eq!(
            version("queued"),
            (3, 1, Some("object-queued".into())),
            "the queued write keeps its size and object id"
        );
        assert_eq!(
            db.apply_snapshot_version("plain", 3, 99, false).unwrap(),
            SnapshotVersion::Raised { old: 1, new: 3 }
        );
        assert_eq!(
            version("plain"),
            (3, 99, None),
            "the node's size; the old object id is not current"
        );
    }

    /// The engine-start check reads op metadata as JSON: one malformed row must not
    /// fail the start repair (its claims would stay set), as the accept's read does not.
    #[cfg(target_os = "macos")]
    #[test]
    fn engine_start_survives_an_op_with_malformed_metadata() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed_own_row(&db, "version-zero", FileStatus::Local, 20);
        db.enqueue_operation(&PendingOperation {
            op_id: "op-malformed".into(),
            kind: OperationKind::UploadVersion,
            file_id: Some("version-zero".into()),
            parent_id: None,
            target_path: Some("version-zero.txt".into()),
            metadata_json: Some("{not json".into()),
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
        let repair = db
            .engine_start_repair()
            .expect("a malformed op never fails engine start");
        assert!(
            repair.resnapshot_requested,
            "the version-0 row still asks for a snapshot"
        );
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
            completed_version: None,
            completed_object_version_id: None,
            completed_mime_type: None,
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
        db.enqueue_operation(&op("t1", OperationKind::TrashFile, "trashed"))
            .unwrap();
        db.enqueue_operation(&op("u1", OperationKind::UploadVersion, "uploaded"))
            .unwrap();
        assert!(
            db.has_pending_trash("trashed").unwrap(),
            "a queued trash, even out of retries"
        );
        assert!(
            !db.has_pending_trash("uploaded").unwrap(),
            "another kind of op is not a delete"
        );
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

    fn seed_child_under(db: &StateDb, file_id: &str, path: &str, kind: ItemKind, parent_id: Option<&str>) {
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
        let (_, anchor_2) = db
            .list_file_changes(Some(&anchor_1))
            .unwrap()
            .expect("changes since anchor 1");
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
        assert_eq!(
            anchor_2.as_deref(),
            Some(anchor_1.as_ref()),
            "a no-op poll must not move the anchor"
        );
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
        assert_eq!(
            reparent.old_parent_id.as_deref(),
            Some("old-parent"),
            "old parent for materialized-set filtering"
        );
        assert_eq!(
            reparent.new_parent_id.as_deref(),
            Some("new-parent"),
            "new parent read from the files row at record time"
        );
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
        db.record_file_change("child-1", FpChangeKind::Deleted, Some("parent-1".into()))
            .unwrap();
        let (changes, _) = db.list_file_changes(None).unwrap().unwrap();
        let deletion = changes
            .iter()
            .find(|change| change.file_id == "child-1" && change.kind == FpChangeKind::Deleted)
            .expect("the deleted change must come back");
        assert_eq!(
            deletion.new_parent_id.as_deref(),
            Some("parent-1"),
            "deleted items report their old parent as new_parent_id"
        );
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
            assert!(
                changes.len() <= 100,
                "a page must honor the limit, got {}",
                changes.len()
            );
            for change in &changes {
                assert!(
                    seen.insert((change.file_id.clone(), change.kind)),
                    "a change came back twice: {:?}",
                    change
                );
            }
            pages += 1;
            match anchor {
                Some(a) if a == cursor.clone().unwrap_or_default() => {
                    // The up-to-date marker echoes the caller's own cursor —
                    // the window is complete.
                    break;
                }
                Some(a) => {
                    assert!(
                        a > cursor.clone().unwrap_or_default(),
                        "paging anchors must strictly increase"
                    );
                    cursor = Some(a);
                }
                None => break,
            }
            assert!(
                pages < 10,
                "250 changes at 100/page must take 3 pages, not spin forever"
            );
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
        assert!(
            changes.is_empty(),
            "swept changes are no longer deliverable: {changes:?}"
        );
        assert_eq!(
            anchor_after.as_deref(),
            Some(anchor.as_ref()),
            "the anchor survives the sweep"
        );
        // After the sweep the log is empty (tip = 0), so a from-nil listing
        // starts a FRESH enumeration — the sweep means "history older than the
        // cutoff is gone", not "the anchor resets". The persistent anchor ROW
        // (fp_last_anchor) is what must survive for currentSyncAnchor:
        assert_eq!(
            db.fp_last_anchor().unwrap().as_deref(),
            Some(anchor.as_ref()),
            "the persistent anchor survives the sweep (crash recovery reads it)"
        );
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
        assert_eq!(
            reopened.as_deref(),
            Some(anchor.as_ref()),
            "the anchor and consumed state must survive process death"
        );
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
        db.0.lock()
            .expect("state_db mutex poisoned")
            .query_row(
                "SELECT recorded_at FROM fp_changes WHERE file_id = ?1",
                params![file_id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap()
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
        assert!(
            recorded_at > 0,
            "a recorded change must carry its real insertion time, got recorded_at={recorded_at}"
        );
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
        let fresh_rows: i64 =
            db.0.lock()
                .expect("state_db mutex poisoned")
                .query_row("SELECT COUNT(*) FROM fp_changes WHERE file_id = 'f-1'", [], |row| {
                    row.get(0)
                })
                .unwrap();
        assert!(fresh_rows > 0, "the seed must have written change rows");
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let deleted = db.sweep_file_changes(now - 7 * 24 * 3600).unwrap();
        assert_eq!(
            deleted, 0,
            "fresh rows must NEVER be swept (was: every row, recorded_at=0 < cutoff)"
        );
        // The change is still deliverable AFTER the sweep (no pre-consume:
        // a from-nil listing must still see it).
        let (changes, _) = db.list_file_changes(None).unwrap().unwrap();
        assert!(
            !changes.is_empty() && changes.iter().all(|change| change.file_id == "f-1"),
            "the fresh changes are still deliverable after the sweep: {changes:?}"
        );
        // Invariant: the sweep never touches the anchor row.
        assert!(
            db.fp_last_anchor().unwrap().is_some(),
            "the persistent anchor survives the sweep (crash recovery reads it)"
        );
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
            )
            .unwrap();
            conn.execute(
                "UPDATE fp_changes SET recorded_at = ?1 WHERE file_id = 'edge-1'",
                params![cutoff],
            )
            .unwrap();
        }
        let count = |file_id: &str| -> i64 {
            db.0.lock()
                .expect("state_db mutex poisoned")
                .query_row(
                    "SELECT COUNT(*) FROM fp_changes WHERE file_id = ?1",
                    params![file_id],
                    |row| row.get(0),
                )
                .unwrap()
        };
        let old_rows = count("old-1");
        let edge_rows = count("edge-1");
        assert!(
            old_rows > 0 && edge_rows > 0,
            "both files must have backdated change rows"
        );
        let deleted = db.sweep_file_changes(cutoff).unwrap();
        assert_eq!(
            deleted as i64, old_rows,
            "strict <: only the rows strictly OLDER than the cutoff are swept"
        );
        assert_eq!(count("old-1"), 0, "aged-out rows are gone");
        assert_eq!(
            count("edge-1"),
            edge_rows,
            "a row recorded exactly AT the cutoff survives (strict <)"
        );
        // Invariant: the sweep never touches the anchor row.
        assert!(
            db.fp_last_anchor().unwrap().is_some(),
            "the anchor cursor outlives swept history"
        );
    }

    // ── R10 (spec 2026-10-06 §5.6): local data is bound to the account that created it ──

    #[test]
    fn the_owner_round_trips_and_a_sign_out_purge_forgets_it_with_the_staged_payloads() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        assert_eq!(db.owner().unwrap(), None);
        assert!(!db.has_account_data().unwrap(), "a new database holds no account data");
        let owner = crate::account_binding::Identity::new(Some("u-a"), Some("a@beebeeb.io"));
        db.set_owner(&owner).unwrap();
        assert_eq!(db.owner().unwrap(), Some(owner));
        assert!(
            !db.has_account_data().unwrap(),
            "the owner record alone is not account data"
        );
        db.track_staged_payload("/tmp/bb-r10-staged-1.bin", None, false)
            .unwrap();
        assert!(db.has_account_data().unwrap());
        let purge = db.purge_all_local_state().unwrap();
        assert!(
            purge.payload_paths.contains(&"/tmp/bb-r10-staged-1.bin".to_string()),
            "staged files are returned for deletion"
        );
        let staged: i64 =
            db.0.lock()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM staged_payloads", [], |r| r.get(0))
                .unwrap();
        assert_eq!(staged, 0, "staged payload rows are gone");
        assert_eq!(db.owner().unwrap(), None, "a sign-out forgets the owner");
    }

    #[test]
    fn clearing_account_data_leaves_no_row_and_no_owner() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        db.0.lock()
            .unwrap()
            .execute("INSERT INTO sync_state(key, value) VALUES ('cursor', '12')", [])
            .unwrap();
        db.track_staged_payload("/tmp/bb-r10-staged-2.bin", None, true).unwrap();
        db.set_owner(&crate::account_binding::Identity::new(Some("u-a"), None))
            .unwrap();
        assert!(db.has_account_data().unwrap(), "a sync cursor is account data");
        db.clear_account_data(false).unwrap();
        assert!(!db.has_account_data().unwrap());
        assert_eq!(db.owner().unwrap(), None);
    }

    /// Added after a mutation survived (task 1834, M20): the sync cursor is account data on its own, and the
    /// owner record, which lives in the same table, is not. Nothing else is in the database here, so this
    /// is the only row `has_account_data` can be counting.
    #[test]
    fn a_sync_cursor_alone_is_account_data_and_the_owner_record_alone_is_not() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        db.set_owner(&crate::account_binding::Identity::new(
            Some("u-a"),
            Some("a@beebeeb.io"),
        ))
        .unwrap();
        assert!(!db.has_account_data().unwrap(), "the owner record is not account data");
        db.0.lock()
            .unwrap()
            .execute("INSERT INTO sync_state(key, value) VALUES ('cursor', '12')", [])
            .unwrap();
        assert!(db.has_account_data().unwrap(), "a sync cursor is");
    }

    /// F2 (fix round 1 of Task 10): adopting unowned local data is ONE conditional step, once per database, and
    /// the "already done" mark is not a row, so no purge or reset can clear it and re-open the window.
    #[test]
    fn adoption_is_one_conditional_step_that_happens_at_most_once() {
        use crate::account_binding::Identity;
        let dir = tempdir().unwrap();
        let a = Identity::new(None, Some("a@beebeeb.io"));
        let b = Identity::new(None, Some("b@beebeeb.io"));
        let seed = |db: &StateDb| {
            db.0.lock()
                .unwrap()
                .execute("INSERT INTO sync_state(key, value) VALUES ('cursor', '1')", [])
                .unwrap();
        };

        // Unowned data and a candidate: adopted, and the window is shut behind it.
        let db = StateDb::open(dir.path().join("one.db")).unwrap();
        seed(&db);
        assert!(db.adopt_owner_once(Some(&a)).unwrap());
        assert_eq!(db.owner().unwrap(), Some(a.clone()));
        assert!(
            !db.adopt_owner_once(Some(&b)).unwrap(),
            "the window is shut: a second candidate is refused"
        );
        assert_eq!(db.owner().unwrap(), Some(a.clone()), "and the owner is never replaced");

        // A first look with no candidate (no vault key) also shuts the window: later candidates are too late.
        let db = StateDb::open(dir.path().join("two.db")).unwrap();
        seed(&db);
        assert!(!db.adopt_owner_once(None).unwrap());
        assert!(!db.adopt_owner_once(Some(&a)).unwrap(), "the first startup has passed");
        assert_eq!(db.owner().unwrap(), None);

        // Nothing to adopt (no account data) shuts the window without recording anybody.
        let db = StateDb::open(dir.path().join("three.db")).unwrap();
        assert!(!db.adopt_owner_once(Some(&a)).unwrap());
        seed(&db);
        assert!(
            !db.adopt_owner_once(Some(&a)).unwrap(),
            "data that appears later is not the upgrade's"
        );
        assert_eq!(db.owner().unwrap(), None);

        // The mark survives a purge and a reset: it is a header value, not a row.
        let db = StateDb::open(dir.path().join("four.db")).unwrap();
        seed(&db);
        db.close_adoption_window().unwrap();
        db.purge_all_local_state().unwrap();
        db.clear_account_data(false).unwrap();
        seed(&db);
        assert!(
            !db.adopt_owner_once(Some(&a)).unwrap(),
            "a reset does not re-open the window"
        );
        assert!(!db.has_account_data().unwrap_or(false) || db.owner().unwrap().is_none());
    }

    /// An owner that is already recorded is never replaced by an adoption, and shuts the window too.
    #[test]
    fn adoption_never_replaces_a_recorded_owner() {
        use crate::account_binding::Identity;
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        db.0.lock()
            .unwrap()
            .execute("INSERT INTO sync_state(key, value) VALUES ('cursor', '1')", [])
            .unwrap();
        db.set_owner(&Identity::new(Some("u-a"), None)).unwrap();
        assert!(
            !db.adopt_owner_once(Some(&Identity::new(None, Some("b@beebeeb.io"))))
                .unwrap()
        );
        assert_eq!(db.owner().unwrap(), Some(Identity::new(Some("u-a"), None)));
    }

    /// R3 (fix round 2): a clear that owes no removal leaves the "domain gone" mark; one that owes it leaves the debt
    /// and no mark; a debt recorded later wipes the mark (a domain may be registered again). Neither is account data.
    #[test]
    fn the_domain_gone_mark_and_the_removal_debt_exclude_each_other_and_are_not_account_data() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        db.clear_account_data(false).unwrap();
        assert!(db.finder_domain_gone().unwrap() && !db.finder_removal_owed().unwrap());
        assert!(!db.has_account_data().unwrap(), "the mark is not account data");
        db.clear_account_data(true).unwrap();
        assert!(
            !db.finder_domain_gone().unwrap() && db.finder_removal_owed().unwrap(),
            "owing a removal leaves no mark"
        );
        assert!(!db.has_account_data().unwrap(), "the debt is not account data");
        db.clear_account_data(false).unwrap();
        db.set_finder_removal_owed(true).unwrap();
        assert!(
            !db.finder_domain_gone().unwrap(),
            "a debt recorded later wipes the mark"
        );
        db.set_finder_removal_owed(false).unwrap();
        assert!(!db.finder_removal_owed().unwrap());
    }

    /// Fix round 3: a domain registered again is no longer known gone. Clearing the mark touches nothing else: the
    /// owner, a removal debt and the account rows stay.
    #[test]
    fn a_domain_registered_again_clears_the_gone_mark_and_nothing_else() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        db.clear_account_data(false).unwrap();
        assert!(db.finder_domain_gone().unwrap());
        db.clear_finder_domain_gone().unwrap();
        assert!(!db.finder_domain_gone().unwrap(), "the mark is gone");
        db.clear_finder_domain_gone().unwrap(); // nothing to clear is not an error
        let owner = crate::account_binding::Identity::new(Some("u-a"), Some("a@beebeeb.io"));
        db.set_owner(&owner).unwrap();
        db.set_finder_removal_owed(true).unwrap();
        db.clear_finder_domain_gone().unwrap();
        assert_eq!(db.owner().unwrap(), Some(owner), "the owner stays");
        assert!(db.finder_removal_owed().unwrap(), "a removal debt stays");
    }

    #[test]
    fn every_table_is_classified_for_the_account_binding() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        let conn = db.0.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'")
            .unwrap();
        let tables: Vec<String> = stmt
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_>>()
            .unwrap();
        assert!(tables.len() >= 12, "{tables:?}");
        for table in &tables {
            assert!(
                ACCOUNT_TABLES.contains(&table.as_str())
                    || DEVICE_TABLES.contains(&table.as_str())
                    || table == "sync_state",
                "{table}: list it in ACCOUNT_TABLES (R10), or in DEVICE_TABLES if it holds no account data"
            );
        }
    }

    #[test]
    fn queued_or_staged_counts_the_queue_and_the_staged_payloads() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        assert_eq!(db.queued_or_staged_count().unwrap(), 0);
        db.track_staged_payload("/tmp/bb-p1-staged.bin", None, false).unwrap();
        assert_eq!(db.queued_or_staged_count().unwrap(), 1);
        db.enqueue_operation(&PendingOperation {
            op_id: "op-p1".into(),
            kind: OperationKind::UploadVersion,
            file_id: Some("file-p1".into()),
            parent_id: None,
            target_path: Some("/P1.txt".into()),
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
        assert_eq!(
            db.queued_or_staged_count().unwrap(),
            2,
            "a queued change and a staged payload both count"
        );
    }

    // ── Rule 5: the item's status follows the queue (spec §9.1–§9.2) ──────────

    /// T31 (§9.2): the startup reconcile leaves a row whose Finder upload has not parked
    /// `Uploading`. A row with no Finder upload is reset to `Error`, as before.
    #[test]
    fn startup_keeps_a_row_with_a_live_upload_uploading() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed_own_row(&db, "with-save", FileStatus::Local, 10);
        let mut contract = db.get_file_contract_state("with-save").unwrap().unwrap();
        contract.current_version = 1;
        db.set_file_contract_state(&contract).unwrap();
        db.track_staged_payload("/staged/with-save", None, false).unwrap();
        let accepted = db
            .accept_finder_write(
                &FinderAccept {
                    retire_parked: None,
                    op_id: "op-with-save",
                    file_id: "with-save",
                    kind: FinderAcceptKind::Modify {
                        incoming_base: Some("1"),
                    },
                    parent_id: None,
                    target_path: Some("with-save.txt"),
                    metadata_json: "{}",
                    payload_path: "/staged/with-save",
                    size_bytes: 3,
                    modified_at: 100,
                    backup_source_key: None,
                    now: 100,
                },
                &|_, _| Vec::new(),
            )
            .unwrap();
        assert!(matches!(accepted, AcceptOutcome::Queued { .. }));
        seed_own_row(&db, "stale-upload", FileStatus::Uploading, 20);

        db.reconcile_stale_in_flight_on_startup().unwrap();

        assert_eq!(
            db.get_file("with-save").unwrap().unwrap().status,
            FileStatus::Uploading,
            "its save still uploads"
        );
        assert_eq!(
            db.get_file("stale-upload").unwrap().unwrap().status,
            FileStatus::Error,
            "unchanged for a row with no Finder upload"
        );
    }

    /// The startup reconcile returns the rows it reset, which is what its log
    /// line reports. A row it keeps `Uploading` (its Finder upload resumes) is not counted.
    #[test]
    fn the_startup_reconcile_counts_only_the_rows_it_resets() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed_v1_row(&db, "with-save");
        assert!(matches!(
            accept_save(&db, "op-with-save", "with-save", Some("1")),
            AcceptOutcome::Queued { .. }
        ));
        db.set_status("with-save", FileStatus::Uploading).unwrap();
        seed_own_row(&db, "stale-upload", FileStatus::Uploading, 20);
        seed_own_row(&db, "stale-download", FileStatus::Downloading, 30);

        let reset = db.reconcile_stale_in_flight_on_startup().unwrap();

        assert_eq!(
            reset,
            vec!["stale-download".to_string(), "stale-upload".to_string()],
            "the stale upload and the stale download, not the kept row"
        );
        assert_eq!(db.get_file("with-save").unwrap().unwrap().status, FileStatus::Uploading);
        assert_eq!(db.get_file("stale-upload").unwrap().unwrap().status, FileStatus::Error);
        assert_eq!(
            db.get_file("stale-download").unwrap().unwrap().status,
            FileStatus::CloudOnly
        );
        assert!(
            db.reconcile_stale_in_flight_on_startup().unwrap().is_empty(),
            "nothing left to reset"
        );
    }

    /// Spec §9.1–§9.2: a row the startup reconcile resets changes how its item is presented
    /// (an upload that will not resume presents `error`, read-only), so each reset is recorded
    /// for the system like every other status change, in the reconcile's transaction. A row it
    /// keeps records nothing.
    #[test]
    fn the_startup_reconcile_records_each_row_it_resets_for_the_system() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed_v1_row(&db, "with-save");
        assert!(matches!(
            accept_save(&db, "op-with-save", "with-save", Some("1")),
            AcceptOutcome::Queued { .. }
        ));
        seed_own_row(&db, "stale-upload", FileStatus::Uploading, 20);
        seed_own_row(&db, "stale-download", FileStatus::Downloading, 30);
        let tip = |db: &StateDb| -> i64 {
            db.0.lock()
                .unwrap()
                .query_row("SELECT COALESCE(MAX(seq), 0) FROM fp_changes", [], |row| row.get(0))
                .unwrap()
        };
        let before = tip(&db);

        db.reconcile_stale_in_flight_on_startup().unwrap();

        let recorded: Vec<(String, String)> = {
            let conn = db.0.lock().unwrap();
            let mut stmt = conn
                .prepare("SELECT file_id, kind FROM fp_changes WHERE seq > ?1 ORDER BY file_id")
                .unwrap();
            stmt.query_map(params![before], |row| Ok((row.get(0)?, row.get(1)?)))
                .unwrap()
                .collect::<Result<Vec<_>>>()
                .unwrap()
        };
        assert_eq!(
            recorded,
            vec![
                (
                    "stale-download".to_string(),
                    FpChangeKind::Modified.as_str().to_string()
                ),
                ("stale-upload".to_string(), FpChangeKind::Modified.as_str().to_string()),
            ],
            "one change per reset row, none for the row whose save still uploads"
        );
    }

    /// A row the server holds at version 1.
    fn seed_v1_row(db: &StateDb, file_id: &str) {
        seed_own_row(db, file_id, FileStatus::Local, 10);
        let mut contract = db.get_file_contract_state(file_id).unwrap().unwrap();
        contract.current_version = 1;
        db.set_file_contract_state(&contract).unwrap();
    }

    /// One File Provider save of `file_id` on `base`, through the accept transaction.
    fn accept_save(db: &StateDb, op_id: &str, file_id: &str, base: Option<&str>) -> AcceptOutcome {
        let payload_path = format!("/staged/{op_id}");
        db.track_staged_payload(&payload_path, None, false).unwrap();
        db.accept_finder_write(
            &FinderAccept {
                retire_parked: None,
                op_id,
                file_id,
                kind: FinderAcceptKind::Modify { incoming_base: base },
                parent_id: None,
                target_path: Some("a.txt"),
                metadata_json: "{}",
                payload_path: &payload_path,
                size_bytes: 3,
                modified_at: 100,
                backup_source_key: None,
                now: 100,
            },
            &|_, _| Vec::new(),
        )
        .unwrap()
    }

    fn queued_token(outcome: AcceptOutcome) -> String {
        match outcome {
            AcceptOutcome::Queued { token, .. } => token,
            other => panic!("the save is queued: {other:?}"),
        }
    }

    fn status_of(db: &StateDb, file_id: &str) -> FileStatus {
        db.get_file(file_id).unwrap().unwrap().status
    }

    fn changes_of(db: &StateDb, file_id: &str) -> usize {
        db.list_file_changes(None)
            .unwrap()
            .map(|(changes, _)| changes.into_iter().filter(|change| change.file_id == file_id).count())
            .unwrap_or(0)
    }

    /// §9.1, step 3.3: the runner's park of a claimed Finder upload sets `Error` in its own
    /// transaction, and records the change for the system. A file whose later save is still
    /// queued stays `Uploading`.
    #[test]
    fn a_claimed_park_settles_the_row_to_error() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed_v1_row(&db, "parked");
        queued_token(accept_save(&db, "w1", "parked", Some("1")));
        assert_eq!(status_of(&db, "parked"), FileStatus::Uploading, "precondition");
        let changes = changes_of(&db, "parked");
        let ClaimOutcome::Claimed(claimed) = db.claim_operation("w1", 1).unwrap() else {
            panic!("w1 is claimable")
        };
        assert!(
            db.park_claimed("w1", &claimed.claim_id, ParkReason::PayloadMissing, 2)
                .unwrap()
        );
        assert_eq!(
            status_of(&db, "parked"),
            FileStatus::Error,
            "a parked upload presents error"
        );
        assert_eq!(
            changes_of(&db, "parked"),
            changes + 1,
            "the flip is recorded for the system"
        );

        seed_v1_row(&db, "live");
        let first = queued_token(accept_save(&db, "l1", "live", Some("1")));
        queued_token(accept_save(&db, "l2", "live", Some(&first)));
        let ClaimOutcome::Claimed(claimed) = db.claim_operation("l1", 1).unwrap() else {
            panic!("l1 is claimable")
        };
        assert!(
            db.park_claimed("l1", &claimed.claim_id, ParkReason::PayloadMissing, 2)
                .unwrap()
        );
        assert_eq!(
            status_of(&db, "live"),
            FileStatus::Uploading,
            "its successor has not parked: still uploading"
        );
    }

    /// §9.1 (m-8): the settle moves only an `uploading` row. A park never
    /// takes a `Trashing` row out of the trash, nor a `Conflict` row out of its conflict, and
    /// records no change for either.
    #[test]
    fn a_park_never_moves_a_row_out_of_the_trash_or_a_conflict() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        for (file_id, op_id, status) in [
            ("in-trash", "t1", FileStatus::Trashing),
            ("in-conflict", "c1", FileStatus::Conflict),
        ] {
            seed_v1_row(&db, file_id);
            queued_token(accept_save(&db, op_id, file_id, Some("1")));
            db.set_status(file_id, status.clone()).unwrap();
            let changes = changes_of(&db, file_id);
            let ClaimOutcome::Claimed(claimed) = db.claim_operation(op_id, 1).unwrap() else {
                panic!("{op_id} is claimable")
            };
            assert!(
                db.park_claimed(op_id, &claimed.claim_id, ParkReason::PayloadMissing, 2)
                    .unwrap(),
                "precondition: {op_id} parked"
            );
            assert_eq!(
                status_of(&db, file_id),
                status,
                "a park leaves a {status:?} row as it is"
            );
            assert_eq!(changes_of(&db, file_id), changes, "no change recorded for {file_id}");
        }
    }

    /// §9.1, step 3.3 (rule 6a′): a save parked at its accept sets `Error` in the accept's
    /// transaction.
    #[test]
    fn a_park_at_accept_settles_the_row_to_error() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed_v1_row(&db, "unknown-base");
        let outcome = accept_save(&db, "w1", "unknown-base", None);
        assert!(
            matches!(outcome, AcceptOutcome::ParkedAtOnce { .. }),
            "precondition: {outcome:?}"
        );
        assert_eq!(status_of(&db, "unknown-base"), FileStatus::Error);
    }

    /// §9.1, step 3.3: a park inside the claim (here `predecessor_lost`) sets `Error` in the
    /// claim's transaction.
    #[test]
    fn a_park_in_the_claim_settles_the_row_to_error() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed_v1_row(&db, "orphan");
        let first = queued_token(accept_save(&db, "w1", "orphan", Some("1")));
        queued_token(accept_save(&db, "w2", "orphan", Some(&first)));
        db.0.lock()
            .unwrap()
            .execute("DELETE FROM operation_queue WHERE op_id = 'w1'", [])
            .unwrap();
        let outcome = db.claim_operation("w2", 1).unwrap();
        assert!(
            matches!(
                outcome,
                ClaimOutcome::Parked {
                    reason: ParkReason::PredecessorLost,
                    ..
                }
            ),
            "precondition: parked in the claim"
        );
        assert_eq!(status_of(&db, "orphan"), FileStatus::Error);
    }

    /// §9.1, step 3.3: the snapshot count's park (`base_unknown`) sets `Error` for each file it
    /// parked, in its transaction.
    #[test]
    fn the_snapshot_counts_park_settles_the_row_to_error() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        // Version 0 and no base: the save waits for a snapshot (6b).
        seed_own_row(&db, "no-version", FileStatus::Local, 10);
        queued_token(accept_save(&db, "w1", "no-version", None));
        assert_eq!(db.finder_write("w1").unwrap().unwrap().base_pending, 1, "precondition");
        let mut parked = Vec::new();
        for snapshot in 0..BASE_PENDING_PARK_AT {
            parked = db.note_snapshot_for_base_pending(10 + snapshot).unwrap();
            if !parked.is_empty() {
                break;
            }
        }
        assert_eq!(
            parked,
            vec![("w1".to_string(), Some("no-version".to_string()))],
            "precondition"
        );
        assert_eq!(status_of(&db, "no-version"), FileStatus::Error);
    }

    /// §9.1–§9.2: a Finder upload whose last attempt used up its attempts has parked by the
    /// queue's own rule (`attempts < max_attempts` is "not parked"), so it presents `Error`.
    /// An earlier failed attempt leaves `Uploading`. An upload without a write id (Windows, the
    /// watcher) is not touched: its rollback keeps its status.
    #[test]
    fn a_finder_upload_that_uses_up_its_attempts_settles_the_row_to_error() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        for (file_id, op_id) in [("retried", "r1"), ("given-up", "g1")] {
            seed_v1_row(&db, file_id);
            queued_token(accept_save(&db, op_id, file_id, Some("1")));
        }
        let ClaimOutcome::Claimed(claimed) = db.claim_operation("r1", 1).unwrap() else {
            panic!("r1 is claimable")
        };
        assert!(
            db.record_attempt_claimed("r1", &claimed.claim_id, 1, 60, Some("HTTP 500"))
                .unwrap()
        );
        assert_eq!(status_of(&db, "retried"), FileStatus::Uploading, "it will retry");
        let ClaimOutcome::Claimed(claimed) = db.claim_operation("g1", 1).unwrap() else {
            panic!("g1 is claimable")
        };
        let last = claimed.op.max_attempts;
        assert!(
            db.record_attempt_claimed("g1", &claimed.claim_id, last, 60, Some("HTTP 500"))
                .unwrap()
        );
        assert_eq!(status_of(&db, "given-up"), FileStatus::Error, "it will not retry");

        seed_own_row(&db, "watcher", FileStatus::Uploading, 10);
        db.enqueue_operation(&queued(
            "u1",
            OperationKind::UploadVersion,
            "watcher",
            Some("/staged/u1"),
        ))
        .unwrap();
        let ClaimOutcome::Claimed(claimed) = db.claim_operation("u1", 1).unwrap() else {
            panic!("u1 is claimable")
        };
        assert!(
            db.record_attempt_claimed("u1", &claimed.claim_id, 25, 60, Some("HTTP 500"))
                .unwrap()
        );
        assert_eq!(
            status_of(&db, "watcher"),
            FileStatus::Uploading,
            "write-keyed: no write id, not touched here"
        );
    }

    /// P2-1: purging a revoked share's content marks its queued ops' copies released in the same
    /// transaction, so the next start's release path removes them. Before, the op went and its
    /// plaintext copy stayed until a sign-out.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_revoked_shares_queued_copy_is_released_and_removed_at_the_next_start() {
        let temp = tempfile::tempdir().unwrap();
        let db = StateDb::open(temp.path().join("state.db")).unwrap();
        seed_contract_row(&db, "revoked", "/Shared with me/Revoked", None, FileStatus::Local, 20);
        let mut revoked = db.get_file_contract_state("revoked").unwrap().unwrap();
        revoked.namespace = Namespace::SharedWithMe;
        revoked.shared_root_id = Some("revoked".into());
        revoked.share_id = Some("invite-revoked".into());
        revoked.permission_bits = PERMISSION_READ | PERMISSION_WRITE;
        db.set_file_contract_state(&revoked).unwrap();
        let copy = temp.path().join("copy");
        std::fs::write(&copy, b"a save to a share that was revoked").unwrap();
        let copy_path = copy.to_string_lossy().into_owned();
        db.track_staged_payload(&copy_path, Some("/handed-over"), false)
            .unwrap();
        db.enqueue_operation(&PendingOperation {
            op_id: "op-revoked".into(),
            kind: OperationKind::UploadVersion,
            file_id: Some("revoked".into()),
            parent_id: None,
            target_path: None,
            metadata_json: None,
            payload_path: Some(copy_path.clone()),
            base_version: Some(1),
            base_object_version_id: None,
            attempts: 0,
            max_attempts: 25,
            next_retry_at: 0,
            last_error: None,
            backup_source_key: None,
            created_at: 1,
            updated_at: 1,
        })
        .unwrap();

        db.purge_revoked_shared_content(&[]).unwrap();

        assert!(db.get_operation("op-revoked").unwrap().is_none(), "the op is gone");
        assert_eq!(
            db.staged_payloads_for_signout().unwrap(),
            vec![(copy_path.clone(), Some("/handed-over".to_string()), true)],
            "its copy is released in the purge's transaction"
        );
        let repair = db.engine_start_repair().unwrap();
        assert_eq!(repair.released_payloads, vec![copy_path]);
        assert_eq!(
            crate::staged_payload::remove_released(&db, &repair.released_payloads),
            1
        );
        assert!(!copy.exists(), "the next start removed it");
        assert!(db.staged_payloads_for_signout().unwrap().is_empty());
    }

    /// Review M4 (macOS): a released copy (`completed = 1`) is one whose upload landed and that
    /// waits only for its unlink, so the account-switch warning does not count it as a change not
    /// uploaded. An unfinished copy still counts.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_released_copy_is_not_a_pending_change() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        db.track_staged_payload("/staged/released", None, true).unwrap();
        assert_eq!(
            db.pending_changes_count().unwrap(),
            0,
            "a landed copy waiting for its unlink"
        );
        db.track_staged_payload("/staged/unfinished", None, false).unwrap();
        assert_eq!(
            db.pending_changes_count().unwrap(),
            1,
            "an unfinished copy still counts"
        );
    }

    /// Spec §8.7 S6: the accept requires its copy's journal row, unfinished, so an engine-start sweep
    /// that removed the copy first cannot leave an op naming a missing file. Without it,
    /// nothing is written: no op, no held write, no status change.
    #[test]
    fn an_accept_whose_journal_row_is_gone_writes_nothing() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed_own_row(&db, "swept", FileStatus::Local, 10);
        let mut contract = db.get_file_contract_state("swept").unwrap().unwrap();
        contract.current_version = 1;
        db.set_file_contract_state(&contract).unwrap();
        let accept = |payload_path: &str| {
            db.accept_finder_write(
                &FinderAccept {
                    retire_parked: None,
                    op_id: "op-swept",
                    file_id: "swept",
                    kind: FinderAcceptKind::Modify {
                        incoming_base: Some("1"),
                    },
                    parent_id: None,
                    target_path: Some("swept.txt"),
                    metadata_json: "{}",
                    payload_path,
                    size_bytes: 4,
                    modified_at: 20,
                    backup_source_key: None,
                    now: 30,
                },
                &|_, _| Vec::new(),
            )
            .unwrap()
        };
        // No row at all, and a released row: both are not an unfinished copy.
        db.track_staged_payload("/staged/released", None, true).unwrap();
        for path in ["/staged/never-journalled", "/staged/released"] {
            assert!(matches!(accept(path), AcceptOutcome::CopyGone), "{path}");
        }
        assert!(db.list_operations_for_file("swept").unwrap().is_empty(), "no op");
        assert_eq!(db.get_file("swept").unwrap().unwrap().status, FileStatus::Local);
        let (held, _) = held_write_conn(&db.hold_lock_for_test(), "swept").unwrap();
        assert!(held.is_none(), "no held write");
        // Journalled and unfinished: accepted.
        db.track_staged_payload("/staged/journalled", None, false).unwrap();
        assert!(matches!(accept("/staged/journalled"), AcceptOutcome::Queued { .. }));
    }

    fn accept_late_save(db: &StateDb, payload_path: &str) -> AcceptOutcome {
        db.accept_finder_write(
            &FinderAccept {
                retire_parked: None,
                op_id: "op-late",
                file_id: "late",
                kind: FinderAcceptKind::Modify {
                    incoming_base: Some("1"),
                },
                parent_id: None,
                target_path: Some("late.txt"),
                metadata_json: "{}",
                payload_path,
                size_bytes: 4,
                modified_at: 20,
                backup_source_key: None,
                now: 30,
            },
            &|_, _| Vec::new(),
        )
        .unwrap()
    }

    fn seeded_for_a_late_accept() -> (tempfile::TempDir, StateDb) {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed_own_row(&db, "late", FileStatus::Local, 10);
        let mut contract = db.get_file_contract_state("late").unwrap().unwrap();
        contract.current_version = 1;
        db.set_file_contract_state(&contract).unwrap();
        db.track_staged_payload("/staged/late", None, false).unwrap();
        (dir, db)
    }

    /// U45 (spec 2026-10-10, §15 question 3, option A): once a sign-out's keep step has closed Finder accepts, an
    /// accept that outlived the engine stop queues nothing, and its staged copy's journal row stays unfinished.
    #[test]
    fn a_finder_accept_after_the_engine_stop_queues_nothing() {
        let (_dir, db) = seeded_for_a_late_accept();
        db.close_finder_accepts().unwrap();
        assert!(matches!(
            accept_late_save(&db, "/staged/late"),
            AcceptOutcome::AcceptsClosed
        ));
        assert!(db.list_operations_for_file("late").unwrap().is_empty(), "no op");
        let unfinished: bool = db
            .hold_lock_for_test()
            .query_row(
                "SELECT completed = 0 FROM staged_payloads WHERE path = '/staged/late'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            unfinished,
            "the journal row is left for the purge, or for the next start's sweep"
        );
    }

    /// U45: an engine start, and a cleared account, take saves again.
    #[test]
    fn accepts_reopen_at_engine_start_and_with_the_cleared_account() {
        let (_dir, db) = seeded_for_a_late_accept();
        db.close_finder_accepts().unwrap();
        db.engine_start_repair().unwrap();
        assert!(!db.finder_accepts_closed().unwrap());
        assert!(matches!(
            accept_late_save(&db, "/staged/late"),
            AcceptOutcome::Queued { .. }
        ));
        db.close_finder_accepts().unwrap();
        db.clear_account_data(false).unwrap();
        assert!(!db.finder_accepts_closed().unwrap());
        db.close_finder_accepts().unwrap();
        db.reopen_finder_accepts().unwrap();
        assert!(
            !db.finder_accepts_closed().unwrap(),
            "a sign-out that stops reopens them at once"
        );
    }

    /// U45: reopening accepts deletes the key and nothing else, wherever it happens: a sign-out that stops and the
    /// launch (`reopen_finder_accepts`), and an engine start (`engine_start_repair`). The rest of `sync_state` is the
    /// R10 owner record, a Finder removal debt and the sync cursor. Without the owner, the next engine start would find
    /// unowned local data and reset it (`account_binding::decide`), and the waiting saves with it.
    #[test]
    fn reopening_accepts_clears_the_key_and_nothing_else() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        let owner = crate::account_binding::Identity::new(Some("u-a"), Some("a@beebeeb.io"));
        db.set_owner(&owner).unwrap();
        db.set_finder_removal_owed(true).unwrap();
        db.set_sync_cursor(12).unwrap();
        let sync_state = |db: &StateDb| -> Vec<(String, String)> {
            let conn = db.hold_lock_for_test();
            let mut stmt = conn.prepare("SELECT key, value FROM sync_state ORDER BY key").unwrap();
            let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?))).unwrap();
            rows.collect::<Result<Vec<_>>>().unwrap()
        };
        let before = sync_state(&db);
        let reopened = |when: &str| {
            assert!(!db.finder_accepts_closed().unwrap(), "{when}: the key is gone");
            assert_eq!(db.owner().unwrap(), Some(owner.clone()), "{when}: the owner stays");
            assert!(db.finder_removal_owed().unwrap(), "{when}: a removal debt stays");
            assert_eq!(
                sync_state(&db),
                before,
                "{when}: every other sync_state row is as it was"
            );
        };
        db.close_finder_accepts().unwrap();
        db.reopen_finder_accepts().unwrap();
        reopened("a sign-out that stops, or the launch");
        db.close_finder_accepts().unwrap();
        db.engine_start_repair().unwrap();
        reopened("an engine start");
    }

    /// The closed-accepts key is a device key, as the Finder removal debt and the domain-gone mark are: it records what
    /// a sign-out on this Mac closed, not anything of an account. Alone, or beside the owner record, it is not account
    /// data, so it never makes the R10 binding reset a database or the upgrade adopt one.
    #[test]
    fn the_closed_accepts_key_is_not_account_data() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        db.close_finder_accepts().unwrap();
        assert!(
            !db.has_account_data().unwrap(),
            "the closed-accepts key is not account data"
        );
        db.set_owner(&crate::account_binding::Identity::new(
            Some("u-a"),
            Some("a@beebeeb.io"),
        ))
        .unwrap();
        assert!(!db.has_account_data().unwrap(), "nor is it beside the owner record");
        db.set_sync_cursor(12).unwrap();
        assert!(
            db.has_account_data().unwrap(),
            "a sync cursor beside them is account data"
        );
    }

    /// U45: the engine start's reopen does not ride the repair's transaction. A repair that fails rolls back only its
    /// own work; the runner logs it and serves Finder writes anyway, so accepts are open by then.
    #[test]
    fn a_failed_engine_start_repair_still_reopens_accepts() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        db.close_finder_accepts().unwrap();
        // The repair's alias sweep is refused: an alias old enough to be swept, and a trigger that aborts its delete.
        db.hold_lock_for_test()
            .execute_batch(
                "INSERT INTO id_aliases (provisional_id, server_id, created_at) VALUES ('p', 's', 0);
                 CREATE TRIGGER no_sweep BEFORE DELETE ON id_aliases BEGIN SELECT RAISE(ABORT, 'refused'); END;",
            )
            .unwrap();
        assert!(db.engine_start_repair().is_err(), "the repair failed");
        assert!(
            !db.finder_accepts_closed().unwrap(),
            "a failed repair still reopens accepts"
        );
    }

    /// U45: an accept whose transaction began first commits first, and a read after the close sees it. Two
    /// connections to one file, as the engine and the sign-out have.
    #[test]
    fn an_accept_already_running_commits_before_the_close() {
        for round in 0..50 {
            let dir = tempdir().unwrap();
            let path = dir.path().join("state.db");
            let engine = StateDb::open(&path).unwrap();
            let sign_out = StateDb::open(&path).unwrap();
            let (started, wait_started) = std::sync::mpsc::channel();
            let accept = std::thread::spawn(move || {
                let mut conn = engine.hold_lock_for_test();
                let tx = conn
                    .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                    .unwrap();
                started.send(()).unwrap();
                std::thread::sleep(std::time::Duration::from_millis(20));
                tx.execute(
                    "INSERT INTO operation_queue (op_id, kind, file_id) VALUES ('late', 'upload_version', 'f')",
                    [],
                )
                .unwrap();
                tx.commit().unwrap();
            });
            wait_started.recv().unwrap();
            sign_out.close_finder_accepts().unwrap();
            let rows = sign_out.waiting_rows().unwrap();
            accept.join().unwrap();
            assert!(rows.iter().any(|row| row.op_id == "late"), "round {round}");
        }
    }

    /// U45, the other half of the barrier: the close takes the write lock first. An accept that starts while the
    /// close's transaction is open waits for it, then reads the key in its own transaction and queues nothing. The
    /// close is held open by hand here (`close_finder_accepts` commits at once), so the accept starts inside it.
    #[test]
    fn an_accept_waiting_behind_the_close_queues_nothing() {
        for round in 0..20 {
            let (dir, engine) = seeded_for_a_late_accept();
            let sign_out = StateDb::open(dir.path().join("state.db")).unwrap();
            let mut conn = sign_out.hold_lock_for_test();
            let close = conn
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .unwrap();
            let accept = std::thread::spawn(move || {
                let outcome = accept_late_save(&engine, "/staged/late");
                (outcome, engine.list_operations_for_file("late").unwrap().len())
            });
            std::thread::sleep(std::time::Duration::from_millis(100));
            close
                .execute(
                    "INSERT INTO sync_state (key, value) VALUES (?1, '1')",
                    params![FINDER_ACCEPTS_CLOSED_KEY],
                )
                .unwrap();
            close.commit().unwrap();
            drop(conn);
            let (outcome, ops) = accept.join().unwrap();
            assert!(
                matches!(outcome, AcceptOutcome::AcceptsClosed) && ops == 0,
                "round {round}: an accept that waited behind the close queued: {outcome:?}, {ops} op(s)"
            );
        }
    }

    /// U45, the same for every other Finder change: one that starts while the close holds the write lock waits for it,
    /// then reads the key in `enqueue_finder_change`'s own transaction and queues nothing.
    #[test]
    fn a_finder_change_waiting_behind_the_close_queues_nothing() {
        for round in 0..20 {
            let dir = tempdir().unwrap();
            let path = dir.path().join("state.db");
            let engine = StateDb::open(&path).unwrap();
            let sign_out = StateDb::open(&path).unwrap();
            let mut conn = sign_out.hold_lock_for_test();
            let close = conn
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .unwrap();
            let change = std::thread::spawn(move || {
                let outcome = engine
                    .enqueue_finder_change(&queued_op("rename", OperationKind::RenameFile, "f"), None)
                    .unwrap();
                (outcome, engine.list_operations_for_file("f").unwrap().len())
            });
            std::thread::sleep(std::time::Duration::from_millis(100));
            close
                .execute(
                    "INSERT INTO sync_state (key, value) VALUES (?1, '1')",
                    params![FINDER_ACCEPTS_CLOSED_KEY],
                )
                .unwrap();
            close.commit().unwrap();
            drop(conn);
            assert_eq!(
                change.join().unwrap(),
                (FinderEnqueue::AcceptsClosed, 0),
                "round {round}: a change that waited behind the close queued"
            );
        }
    }

    /// U45: in both fenced bodies the key is the first thing read inside the `BEGIN IMMEDIATE` transaction that queues,
    /// never before it: a read taken before the write lock can see accepts open, wait behind the close, and then queue.
    #[test]
    fn the_fence_is_read_first_inside_the_transaction_that_queues() {
        let source: Vec<&str> = include_str!("state_db.rs").lines().collect();
        for definition in ["pub fn accept_finder_write(", "pub fn enqueue_finder_change("] {
            let start = source
                .iter()
                .position(|line| line.trim_start().starts_with(definition))
                .unwrap_or_else(|| panic!("{definition} not found"));
            let end = start
                + source[start..]
                    .iter()
                    .position(|line| *line == "    }")
                    .expect("the body's end");
            let body = &source[start..end];
            let begin = body
                .iter()
                .position(|line| {
                    line.contains(".transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;")
                })
                .unwrap_or_else(|| panic!("{definition}: no BEGIN IMMEDIATE"));
            let first = body[begin + 1..]
                .iter()
                .map(|line| line.trim())
                .find(|line| !line.is_empty() && !line.starts_with("//"))
                .unwrap();
            assert_eq!(
                first, "if finder_accepts_closed_conn(&tx)? {",
                "{definition}: the fence is the first read inside the transaction"
            );
            assert_eq!(
                body.iter()
                    .filter(|line| line.contains("finder_accepts_closed_conn("))
                    .count(),
                1,
                "{definition}: one read of the fence"
            );
        }
    }

    /// Spec §8.7 S6: the sweep deletes an unfinished row only while nothing names it, so an
    /// accept that committed first keeps its row; a released row is never this step's.
    #[test]
    fn the_sweep_never_forgets_a_row_an_accept_committed() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        db.track_staged_payload("/staged/accepted", None, false).unwrap();
        db.hold_lock_for_test()
            .execute(
                "INSERT INTO operation_queue (op_id, kind, file_id, payload_path)
                 VALUES ('op-accepted', 'upload_version', 'f', '/staged/accepted')",
                [],
            )
            .unwrap();
        db.track_staged_payload("/staged/orphan", None, false).unwrap();
        db.track_staged_payload("/staged/released", None, true).unwrap();
        assert!(!db.forget_unfinished_unreferenced("/staged/accepted").unwrap());
        assert!(db.forget_unfinished_unreferenced("/staged/orphan").unwrap());
        assert!(!db.forget_unfinished_unreferenced("/staged/released").unwrap());
        let mut left: Vec<String> = db
            .staged_payloads_for_signout()
            .unwrap()
            .into_iter()
            .map(|(path, _, _)| path)
            .collect();
        left.sort();
        assert_eq!(
            left,
            vec!["/staged/accepted".to_string(), "/staged/released".to_string()]
        );
    }

    /// U17, the margin; U18.
    #[test]
    fn the_send_phase_makes_live_changes_due_with_a_margin() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        db.hold_lock_for_test()
            .execute_batch(
                "INSERT INTO operation_queue (op_id, kind, file_id, attempts, max_attempts, next_retry_at, paused_reason) VALUES
                   ('a0', 'upload_version', 'f', 0, 5, 9999, NULL),
                   ('a2', 'rename_file', 'f', 2, 5, 9999, NULL),
                   ('a3', 'upload_version', 'f', 3, 5, 9999, NULL),
                   ('a4', 'upload_version', 'f', 4, 5, 9999, NULL),
                   ('parked', 'upload_version', 'f', 5, 5, 9999, NULL),
                   ('paused', 'upload_version', 'f', 0, 5, 9999, 'auth'),
                   ('hydrate', 'hydrate_file', 'f', 0, 5, 9999, NULL);",
            )
            .unwrap();
        assert_eq!(db.make_due_for_send_phase(100).unwrap(), 2);
        let due: Vec<(String, i64, i64)> = {
            let conn = db.hold_lock_for_test();
            let mut stmt = conn
                .prepare("SELECT op_id, next_retry_at, attempts FROM operation_queue ORDER BY op_id")
                .unwrap();
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                .unwrap()
                .map(Result::unwrap)
                .collect()
        };
        for (op, next, attempts) in due {
            let expected_next = if op == "a0" || op == "a2" { 100 } else { 9999 };
            assert_eq!(next, expected_next, "{op}");
            let inserted = match op.as_str() {
                "a2" => 2,
                "a3" => 3,
                "a4" => 4,
                "parked" => 5,
                _ => 0,
            };
            assert_eq!(attempts, inserted, "attempts never change: {op}");
        }
    }

    #[test]
    fn the_send_phase_never_touches_parked_or_paused_changes() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        db.hold_lock_for_test()
            .execute_batch(
                "INSERT INTO operation_queue (op_id, kind, file_id, attempts, max_attempts, next_retry_at, paused_reason) VALUES
                   ('parked', 'upload_version', 'f', 25, 25, 9999, NULL), ('paused', 'create_folder', 'f', 0, 5, 9999, 'quota');",
            )
            .unwrap();
        assert_eq!(db.make_due_for_send_phase(100).unwrap(), 0);
        let untouched: i64 = db
            .hold_lock_for_test()
            .query_row(
                "SELECT COUNT(*) FROM operation_queue WHERE next_retry_at = 9999 AND attempts IN (0, 25)",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(untouched, 2);
    }
}

/// Tasks 1888 + 2226 (spec §4 D1-D4, D8, D14; I1-I4, §5): the reconcile pass against a real database.
#[cfg(test)]
mod reconcile_tests {
    use super::*;
    use crate::held_queue::{Counts, Options};
    use tempfile::{tempdir, TempDir};

    const NOT_FOUND: &str = "HTTP status client error (404 Not Found) for url (https://api.example/api/v1/uploads/init)";
    const SERVER_ERROR: &str = "HTTP status server error (500 Internal Server Error) for url (https://api.example/x)";
    const NOW: i64 = 1_800_000_000;

    fn opts() -> Options {
        Options { device_segment: Some("PC-1".to_string()), case_insensitive: true }
    }

    /// Every directory holds a file: the pass never drops scaffolding (D5) in these tests.
    fn holds_a_file(_: &str) -> bool {
        true
    }

    /// The query plan of `sql`, one line per step.
    fn query_plan(db: &StateDb, sql: &str) -> String {
        let conn = db.0.lock().unwrap();
        let mut stmt = conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
        let args = rusqlite::params_from_iter(std::iter::repeat("x").take(stmt.parameter_count()));
        let steps = stmt.query_map(args, |row| row.get::<_, String>(3)).unwrap();
        steps.map(Result::unwrap).collect::<Vec<_>>().join("\n")
    }

    #[test]
    fn a_remap_reads_the_files_table_through_an_index() {
        let pc = Pc::new();
        for sql in [REMAP_FILES_SQL, GHOST_REFERENCES_SQL] {
            let plan = query_plan(&pc.db, sql);
            assert!(!plan.contains("SCAN files"), "a remap must not scan every row:\n{plan}");
        }
    }

    #[test]
    fn a_tree_holds_a_file_only_when_a_file_is_somewhere_under_it() {
        let root = tempdir().unwrap();
        assert!(!tree_holds_a_file(&root.path().join("missing")), "a missing directory holds no file");
        std::fs::create_dir_all(root.path().join("a/b/c")).unwrap();
        assert!(!tree_holds_a_file(&root.path().join("a")), "empty nested directories hold no file");
        std::fs::write(root.path().join("a/b/c/f.txt"), b"x").unwrap();
        assert!(tree_holds_a_file(&root.path().join("a")), "a file three levels down counts");
        assert!(tree_holds_a_file(&root.path().join("a/b/c")));
    }

    type RowTuple = (String, Option<String>, String, i64);
    type OpTuple = (String, Option<String>, Option<String>, i64, i64, Option<String>, Option<String>);

    struct Pc {
        dir: TempDir,
        db: StateDb,
    }

    impl Pc {
        fn new() -> Self {
            let dir = tempdir().unwrap();
            let db = StateDb::open(dir.path().join("state.db")).unwrap();
            Self { dir, db }
        }

        fn sql(&self, sql: &str, args: impl rusqlite::Params) {
            self.db.0.lock().unwrap().execute(sql, args).unwrap();
        }

        /// A row: `version` > 0 is server-known (the snapshot's shape: `remote_updated_at` stays 0).
        fn row(&self, id: &str, parent: Option<&str>, path: &str, folder: bool, version: i64) {
            self.sql(
                "INSERT INTO files (file_id, path, status, item_kind, current_version, parent_id, size_bytes, remote_updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, 0)",
                params![id, path, if version > 0 { "synced" } else { "local" }, if folder { "folder" } else { "file" }, version, parent],
            );
        }

        fn folder(&self, id: &str, parent: Option<&str>, path: &str, version: i64) {
            self.row(id, parent, path, true, version);
        }

        fn file(&self, id: &str, parent: Option<&str>, path: &str, version: i64) {
            self.row(id, parent, path, false, version);
        }

        /// A folder's create, paused for `auth` as on 4 Oct.
        fn create_folder(&self, op_id: &str, file_id: &str, parent: Option<&str>) {
            self.sql(
                "INSERT INTO operation_queue (op_id, kind, file_id, parent_id, attempts, max_attempts, paused_reason, created_at, updated_at)
                 VALUES (?1, 'create_folder', ?2, ?3, 0, 25, 'auth', 1, 1)",
                params![op_id, file_id, parent],
            );
        }

        /// A new file's upload as the scan queues it, with its staged payload on disk.
        fn upload(&self, op_id: &str, file_id: &str, parent: Option<&str>, created_at: i64) {
            let payload = self.dir.path().join(format!("staged-{op_id}"));
            std::fs::write(&payload, op_id.as_bytes()).unwrap();
            self.sql(
                "INSERT INTO operation_queue (op_id, kind, file_id, parent_id, metadata_json, payload_path, attempts, max_attempts,
                                              next_retry_at, created_at, updated_at)
                 VALUES (?1, 'upload_version', ?2, ?3, '{\"operation\":\"create_file\"}', ?4, 0, 25, 0, ?5, ?5)",
                params![op_id, file_id, parent, payload.to_string_lossy(), created_at],
            );
        }

        /// The op used its retries on `error`.
        fn gave_up(&self, op_id: &str, error: &str) {
            self.sql("UPDATE operation_queue SET attempts = 25, last_error = ?2 WHERE op_id = ?1", params![op_id, error]);
        }

        fn pause(&self, op_id: &str, reason: &str) {
            self.sql("UPDATE operation_queue SET paused_reason = ?2 WHERE op_id = ?1", params![op_id, reason]);
        }

        fn count(&self, sql: &str, args: impl rusqlite::Params) -> i64 {
            self.db.0.lock().unwrap().query_row(sql, args, |row| row.get(0)).unwrap()
        }

        /// How many rows and ops still name `id` (I1).
        fn references(&self, id: &str) -> i64 {
            self.count(
                "SELECT (SELECT COUNT(*) FROM files WHERE file_id = ?1 OR parent_id = ?1)
                      + (SELECT COUNT(*) FROM operation_queue WHERE file_id = ?1 OR parent_id = ?1)",
                params![id],
            )
        }

        fn parent_of_row(&self, id: &str) -> Option<String> {
            self.db.0.lock().unwrap().query_row("SELECT parent_id FROM files WHERE file_id = ?1", params![id], |row| row.get(0)).unwrap()
        }

        /// (parent, attempts, next_retry_at, last_error, paused_reason)
        fn op(&self, op_id: &str) -> (Option<String>, i64, i64, Option<String>, Option<String>) {
            self.db
                .0
                .lock()
                .unwrap()
                .query_row(
                    "SELECT parent_id, attempts, next_retry_at, last_error, paused_reason FROM operation_queue WHERE op_id = ?1",
                    params![op_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
                )
                .unwrap()
        }

        fn has_op(&self, op_id: &str) -> bool {
            self.count("SELECT COUNT(*) FROM operation_queue WHERE op_id = ?1", params![op_id]) == 1
        }

        fn due(&self) -> Vec<String> {
            self.db.list_due_operations(NOW).unwrap().into_iter().map(|op| op.op_id).collect()
        }

        /// Every row and op, sorted: what "unchanged" and "equal" compare.
        fn snapshot(&self) -> (Vec<RowTuple>, Vec<OpTuple>) {
            let conn = self.db.0.lock().unwrap();
            let rows = conn
                .prepare("SELECT file_id, parent_id, path, current_version FROM files ORDER BY file_id")
                .unwrap()
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)))
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            let ops = conn
                .prepare(
                    "SELECT op_id, file_id, parent_id, attempts, next_retry_at, last_error, paused_reason
                     FROM operation_queue ORDER BY op_id",
                )
                .unwrap()
                .query_map([], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?))
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            (rows, ops)
        }

        /// A server folder `t<n>`, the ghost `g<n>` made here at the same path (its create paused for `auth`), and a
        /// new file under the ghost whose upload gave up on the parent 404 the ghost caused.
        fn ghost_with_twin(&self, n: usize, name: &str) {
            let (t, g, f) = (format!("t{n}"), format!("g{n}"), format!("f{n}"));
            self.folder(&t, None, name, 3);
            self.folder(&g, None, name, 0);
            self.file(&f, Some(&g), &format!("{name}/{n}.jpg"), 0);
            self.create_folder(&format!("op-{g}"), &g, None);
            self.upload(&format!("op-{f}"), &f, Some(&g), 10 + n as i64);
            self.gave_up(&format!("op-{f}"), NOT_FOUND);
        }

        fn six_ghosts(&self) {
            for (n, name) in ["A", "B", "C", "D", "E", "F"].iter().enumerate() {
                self.ghost_with_twin(n + 1, name);
            }
        }

        /// A test-only trigger that makes deleting row `id` fail, so the transaction that deletes it rolls back.
        fn fail_deleting(&self, id: &str) {
            self.db
                .0
                .lock()
                .unwrap()
                .execute_batch(&format!(
                    "CREATE TEMP TRIGGER stop_here BEFORE DELETE ON files WHEN OLD.file_id = '{id}'
                     BEGIN SELECT RAISE(ABORT, 'test'); END;"
                ))
                .unwrap();
        }

        fn stop_failing(&self) {
            self.db.0.lock().unwrap().execute_batch("DROP TRIGGER stop_here;").unwrap();
        }
    }

    /// I1: after the pass no row and no op names the ghost; its child and its child's change moved to the twin, and the
    /// change that gave up on the ghost's 404 is due again (D8: attempts 0, no error, due now).
    #[test]
    fn reconcile_remaps_a_ghost_and_leaves_no_reference_to_it() {
        let pc = Pc::new();
        pc.ghost_with_twin(1, "Photos");
        let counts = pc.db.reconcile_held_queue(&opts(), NOW, &holds_a_file).unwrap();
        assert_eq!(counts, Counts { remapped: 1, rearmed: 1, ..Counts::default() });
        assert_eq!(pc.references("g1"), 0);
        assert!(!pc.has_op("op-g1"));
        assert_eq!(pc.parent_of_row("f1").as_deref(), Some("t1"));
        assert_eq!(pc.op("op-f1"), (Some("t1".to_string()), 0, NOW, None, None));
    }

    /// I3: the dry run's counts are the run's, and the dry run writes nothing.
    #[test]
    fn the_dry_run_counts_equal_the_run_and_change_nothing() {
        let pc = Pc::new();
        pc.ghost_with_twin(1, "Photos");
        pc.ghost_with_twin(2, "Music");
        pc.folder("new", None, "New", 0);
        pc.create_folder("op-new", "new", None);
        let before = pc.snapshot();
        let dry = pc.db.reconcile_dry_run(&opts(), &holds_a_file).unwrap();
        assert_eq!(pc.snapshot(), before, "a dry run writes nothing");
        assert_eq!(dry, Counts { remapped: 2, rearmed: 2, replay: 1, ..Counts::default() });
        assert_eq!(pc.db.reconcile_held_queue(&opts(), NOW, &holds_a_file).unwrap(), dry);
    }

    /// I3: a second pass applies nothing; only the standing counts repeat (the ghost kept for replay).
    #[test]
    fn a_second_pass_changes_nothing() {
        let pc = Pc::new();
        pc.ghost_with_twin(1, "Photos");
        pc.folder("new", None, "New", 0);
        pc.create_folder("op-new", "new", None);
        pc.db.reconcile_held_queue(&opts(), NOW, &holds_a_file).unwrap();
        let after_first = pc.snapshot();
        assert_eq!(pc.db.reconcile_held_queue(&opts(), NOW + 60, &holds_a_file).unwrap(), Counts { replay: 1, ..Counts::default() });
        assert_eq!(pc.snapshot(), after_first);
    }

    /// I4 (lead review of #243, fix 1): a failure inside a batch undoes that batch only; a rerun completes, and ends
    /// where an uninterrupted run ends.
    #[test]
    fn a_failure_mid_batch_undoes_only_that_batch_and_a_rerun_completes() {
        let pc = Pc::new();
        pc.six_ghosts();
        let untouched = Pc::new();
        untouched.six_ghosts();
        let before = pc.snapshot();
        pc.fail_deleting("g4");
        assert!(pc.db.reconcile_in_batches(&opts(), NOW, 2, &holds_a_file, &mut |_, _| {}).is_err());
        for g in ["g1", "g2"] {
            assert_eq!(pc.references(g), 0, "{g}: batch 1 committed");
        }
        let (rows, ops) = pc.snapshot();
        let of = |ids: &[&str]| {
            let rows: Vec<_> = rows.iter().filter(|r| ids.contains(&r.0.as_str())).cloned().collect();
            let ops: Vec<_> = ops.iter().filter(|o| ids.iter().any(|id| o.0 == format!("op-{id}"))).cloned().collect();
            (rows, ops)
        };
        let of_before = |ids: &[&str]| {
            let rows: Vec<_> = before.0.iter().filter(|r| ids.contains(&r.0.as_str())).cloned().collect();
            let ops: Vec<_> = before.1.iter().filter(|o| ids.iter().any(|id| o.0 == format!("op-{id}"))).cloned().collect();
            (rows, ops)
        };
        let rest = ["g3", "f3", "g4", "f4", "g5", "f5", "g6", "f6"];
        assert_eq!(of(&rest), of_before(&rest), "batch 2 rolled back, batch 3 never ran");
        pc.stop_failing();
        pc.db.reconcile_in_batches(&opts(), NOW, 2, &holds_a_file, &mut |_, _| {}).unwrap();
        untouched.db.reconcile_in_batches(&opts(), NOW, 2, &holds_a_file, &mut |_, _| {}).unwrap();
        assert_eq!(pc.snapshot(), untouched.snapshot());
    }

    /// I4: a failure at the first unit of a batch keeps every batch committed before it.
    #[test]
    fn a_failure_at_a_batch_boundary_keeps_the_committed_batches_and_a_rerun_completes() {
        let pc = Pc::new();
        pc.six_ghosts();
        let untouched = Pc::new();
        untouched.six_ghosts();
        let before = pc.snapshot();
        pc.fail_deleting("g5");
        assert!(pc.db.reconcile_in_batches(&opts(), NOW, 2, &holds_a_file, &mut |_, _| {}).is_err());
        for g in ["g1", "g2", "g3", "g4"] {
            assert_eq!(pc.references(g), 0, "{g}: batches 1 and 2 committed");
        }
        let (rows, ops) = pc.snapshot();
        let ids = ["g5", "f5", "g6", "f6"];
        let pick = |rows: &[RowTuple], ops: &[OpTuple]| {
            (
                rows.iter().filter(|r| ids.contains(&r.0.as_str())).cloned().collect::<Vec<_>>(),
                ops.iter().filter(|o| ids.iter().any(|id| o.0 == format!("op-{id}"))).cloned().collect::<Vec<_>>(),
            )
        };
        assert_eq!(pick(&rows, &ops), pick(&before.0, &before.1));
        pc.stop_failing();
        pc.db.reconcile_in_batches(&opts(), NOW, 2, &holds_a_file, &mut |_, _| {}).unwrap();
        untouched.db.reconcile_in_batches(&opts(), NOW, 2, &holds_a_file, &mut |_, _| {}).unwrap();
        assert_eq!(pc.snapshot(), untouched.snapshot());
    }

    /// Spec §3 (corrected 2026-10-28): a twin the snapshot wrote (`current_version` 3, `remote_updated_at` 0) is found.
    #[test]
    fn a_twin_written_by_the_snapshot_is_found() {
        let pc = Pc::new();
        pc.ghost_with_twin(1, "Photos");
        assert_eq!(pc.count("SELECT remote_updated_at FROM files WHERE file_id = 't1'", []), 0);
        assert_eq!(pc.db.reconcile_held_queue(&opts(), NOW, &holds_a_file).unwrap().remapped, 1);
    }

    /// D8 order (lead review of #243, fix 2): a re-armed upload keeps its place, so it runs before a newer upload of the
    /// same file, and the newer content lands last.
    #[test]
    fn a_rearmed_upload_runs_before_a_newer_upload_of_the_same_file() {
        let pc = Pc::new();
        pc.ghost_with_twin(1, "Photos");
        pc.upload("op-newer", "f1", Some("g1"), 500);
        pc.db.reconcile_held_queue(&opts(), NOW, &holds_a_file).unwrap();
        assert_eq!(pc.due(), vec!["op-f1".to_string(), "op-newer".to_string()]);
        assert!(matches!(pc.db.claim_operation("op-newer", NOW).unwrap(), ClaimOutcome::Wait));
        let ClaimOutcome::Claimed(older) = pc.db.claim_operation("op-f1", NOW).unwrap() else {
            panic!("the re-armed upload is claimed first")
        };
        assert!(pc.db.finish_claimed("op-f1", &older.claim_id, None).unwrap());
        assert!(matches!(pc.db.claim_operation("op-newer", NOW).unwrap(), ClaimOutcome::Claimed(_)));
    }

    /// A child written between two batches is re-pointed by its ghost's unit.
    #[test]
    fn a_child_added_between_batches_moves_with_its_ghost() {
        let pc = Pc::new();
        pc.ghost_with_twin(1, "A");
        pc.ghost_with_twin(2, "B");
        pc.db
            .reconcile_in_batches(&opts(), NOW, 1, &holds_a_file, &mut |db, batch| {
                if batch == 1 {
                    db.0.lock()
                        .unwrap()
                        .execute(
                            "INSERT INTO files (file_id, path, status, item_kind, current_version, parent_id, size_bytes, remote_updated_at)
                             VALUES ('late', 'B/late.txt', 'local', 'file', 0, 'g2', 0, 0)",
                            [],
                        )
                        .unwrap();
                }
            })
            .unwrap();
        assert_eq!(pc.parent_of_row("late").as_deref(), Some("t2"));
        assert_eq!(pc.references("g2"), 0);
    }

    /// D8 amendment (lead ruling 2026-10-29): a change held for the person that used its retries on the 404 its ghost
    /// caused gets its budget back in the pass and keeps its pause; "Upload to my Beebeeb" then makes it due, and it runs.
    #[test]
    fn a_held_upload_exhausted_on_a_parent_404_under_a_remapped_ghost_uploads_after_upload() {
        let pc = Pc::new();
        pc.ghost_with_twin(1, "Photos");
        pc.pause("op-f1", "found_while_paused");
        let counts = pc.db.reconcile_held_queue(&opts(), NOW, &holds_a_file).unwrap();
        assert_eq!(counts, Counts { remapped: 1, rearmed_paused: 1, ..Counts::default() });
        assert_eq!(pc.op("op-f1"), (Some("t1".to_string()), 0, NOW, None, Some("found_while_paused".to_string())));
        assert!(pc.due().is_empty(), "still held: the pass never releases a change held for the person");
        assert_eq!(pc.db.release_found_while_paused(&["op-f1".to_string()], NOW).unwrap(), 1);
        assert_eq!(pc.due(), vec!["op-f1".to_string()]);
        assert!(matches!(pc.db.claim_operation("op-f1", NOW).unwrap(), ClaimOutcome::Claimed(_)));
    }

    /// D8 amendment: a held change that used its retries on another error keeps them; after Upload it stays stopped,
    /// with its own error.
    #[test]
    fn a_held_upload_exhausted_on_another_error_stays_stopped_after_upload() {
        let pc = Pc::new();
        pc.ghost_with_twin(1, "Photos");
        pc.gave_up("op-f1", SERVER_ERROR);
        pc.pause("op-f1", "found_while_paused");
        let counts = pc.db.reconcile_held_queue(&opts(), NOW, &holds_a_file).unwrap();
        assert_eq!(counts, Counts { remapped: 1, ..Counts::default() });
        assert_eq!(
            pc.op("op-f1"),
            (Some("t1".to_string()), 25, 0, Some(SERVER_ERROR.to_string()), Some("found_while_paused".to_string()))
        );
        assert_eq!(pc.db.release_found_while_paused(&["op-f1".to_string()], NOW).unwrap(), 1);
        assert!(pc.due().is_empty(), "stopped after repeated errors: Upload only clears the pause");
        assert_eq!(pc.op("op-f1").3.as_deref(), Some(SERVER_ERROR));
    }

    impl Pc {
        /// The op's sync-root-relative key, as every create queues it.
        fn queued_at(&self, op_id: &str, path: &str) {
            self.sql("UPDATE operation_queue SET target_path = ?2 WHERE op_id = ?1", params![op_id, path]);
        }

        /// A new file `f` at `A/B/f.txt` whose upload was queued with no parent (D6).
        fn upload_without_a_parent(&self) {
            self.file("f", None, "A/B/f.txt", 0);
            self.upload("op-f", "f", None, 10);
            self.queued_at("op-f", "A/B/f.txt");
        }
    }

    /// D6: after the pass, the change and its row both name the folder found by path, and the change is due under it.
    #[test]
    fn reconcile_gives_a_root_level_upload_its_parent_by_path_in_both_columns() {
        let pc = Pc::new();
        pc.folder("a", None, "A", 3);
        pc.folder("b", Some("a"), "A/B", 3);
        pc.upload_without_a_parent();
        let counts = pc.db.reconcile_held_queue(&opts(), NOW, &holds_a_file).unwrap();
        assert_eq!(counts, Counts { parent_resolved: 1, ..Counts::default() });
        assert_eq!(pc.op("op-f").0.as_deref(), Some("b"));
        assert_eq!(pc.parent_of_row("f").as_deref(), Some("b"));
        assert_eq!(pc.due(), vec!["op-f".to_string()]);
    }

    /// D6 then D2: the folder found by path is a ghost; its remap, in the same plan, points both columns at its twin.
    /// The twin is found by parent and leaf (§3); its path here does not match, so the ghost is the only folder at the
    /// directory. Split over two batches the result is the same.
    #[test]
    fn reconcile_points_both_columns_at_the_twin_when_the_parent_found_by_path_is_a_ghost() {
        let setup = |pc: &Pc| {
            pc.folder("a", None, "A", 3);
            pc.folder("g", Some("a"), "A/B", 0);
            pc.create_folder("op-g", "g", Some("a"));
            pc.folder("t", Some("a"), "Old/B", 3);
            pc.upload_without_a_parent();
        };
        let pc = Pc::new();
        setup(&pc);
        let mut batches = 0;
        let counts = pc
            .db
            .reconcile_in_batches(&opts(), NOW, RECONCILE_UNITS_PER_TX, &holds_a_file, &mut |_, n| batches = n)
            .unwrap();
        assert_eq!(counts, Counts { parent_resolved: 1, remapped: 1, ..Counts::default() });
        assert_eq!(batches, 1, "the resolve and the remap are in one plan");
        assert_eq!(pc.op("op-f").0.as_deref(), Some("t"));
        assert_eq!(pc.parent_of_row("f").as_deref(), Some("t"));
        assert_eq!(pc.references("g"), 0);

        let split = Pc::new();
        setup(&split);
        split.db.reconcile_in_batches(&opts(), NOW, 1, &holds_a_file, &mut |_, _| {}).unwrap();
        assert_eq!(split.snapshot(), pc.snapshot());
    }

    /// D10: a new file that meets a server file of the same name is held (`needs_review`, not due); once the server
    /// file is gone, the next pass releases it to the key check (`auth`), and a key that opens its name resumes it.
    #[test]
    fn reconcile_holds_a_new_file_that_meets_a_server_file_and_releases_it_once_the_twin_is_gone() {
        let pc = Pc::new();
        pc.folder("p", None, "Docs", 3);
        pc.file("s", Some("p"), "Docs/report.txt", 2);
        pc.file("n", Some("p"), "Docs/Report.txt", 0);
        pc.upload("op-n", "n", Some("p"), 10);
        let counts = pc.db.reconcile_held_queue(&opts(), NOW, &holds_a_file).unwrap();
        assert_eq!(counts, Counts { file_twin: 1, ..Counts::default() });
        assert_eq!(pc.op("op-n").4.as_deref(), Some(crate::held_queue::NEEDS_REVIEW));
        assert!(pc.due().is_empty());

        pc.sql("DELETE FROM files WHERE file_id = 's'", []);
        let counts = pc.db.reconcile_held_queue(&opts(), NOW, &holds_a_file).unwrap();
        assert_eq!(counts, Counts { released: 1, ..Counts::default() });
        assert_eq!(pc.op("op-n").4.as_deref(), Some("auth"));
        assert!(pc.due().is_empty());
        assert_eq!(pc.db.resume_operations_paused_for_auth(NOW).unwrap(), 1);
        assert_eq!(pc.due(), vec!["op-n".to_string()]);
    }

    /// Goal 2, I6 (review finding on Task 3): the pass holds `auth` changes as `needs_review` (D4, D6, D10), out of
    /// the key check's reach (D12 reads `auth` only). A later release must not make them due past that check: under a
    /// replaced key they end up `key_replaced`, never sent with a name sealed under the old key.
    #[test]
    fn a_change_the_pass_held_is_never_released_past_the_key_check() {
        let pc = Pc::new();
        // D10: a new file that meets a server file of the same name.
        pc.folder("p", None, "Docs", 3);
        pc.file("s", Some("p"), "Docs/report.txt", 2);
        pc.file("n", Some("p"), "Docs/report.txt", 0);
        pc.upload("op-n", "n", Some("p"), 10);
        pc.pause("op-n", "auth");
        // D6: a new file queued with no parent, whose folder is not known yet.
        pc.upload_without_a_parent();
        pc.pause("op-f", "auth");
        // D4: a new folder with two twins (`≡`).
        pc.folder("t1", None, "Music", 3);
        pc.folder("t2", None, "music", 3);
        pc.folder("g", None, "Music", 0);
        pc.create_folder("op-g", "g", None);

        let counts = pc.db.reconcile_held_queue(&opts(), NOW, &holds_a_file).unwrap();
        assert_eq!(counts, Counts { held_ambiguous: 1, parent_unresolved: 1, file_twin: 1, ..Counts::default() });
        // The key check fails: the vault key was replaced. The held changes are not `auth`, so it does not see them.
        pc.db.hold_operations_paused_for_auth_as_key_replaced(NOW).unwrap();

        // Every cause goes away.
        pc.sql("DELETE FROM files WHERE file_id IN ('s', 't1', 't2')", []);
        pc.folder("a", None, "A", 3);
        pc.folder("b", Some("a"), "A/B", 3);
        let counts = pc.db.reconcile_held_queue(&opts(), NOW, &holds_a_file).unwrap();
        assert_eq!(counts, Counts { replay: 1, released: 3, parent_resolved: 1, ..Counts::default() });
        assert!(pc.due().is_empty(), "released into the key check, never straight to due: {:?}", pc.due());
        // The key check after the pass fails again: all three are held as `key_replaced`.
        assert_eq!(pc.db.hold_operations_paused_for_auth_as_key_replaced(NOW).unwrap(), 3);
        for op_id in ["op-n", "op-f", "op-g"] {
            assert_eq!(pc.op(op_id).4.as_deref(), Some("key_replaced"), "{op_id}");
        }
        assert!(pc.due().is_empty());
    }

    /// D5: empty `Backups/unknown` scaffolding, under a `Backups` the pass folds into its twin, goes in the pass. No op
    /// and no row is left under it, nothing names a dropped row (I1), and the pass returns its directories deepest first
    /// for the caller to remove after the commit; the pass itself removes none. A second pass changes nothing (I3).
    #[test]
    fn the_pass_drops_empty_backups_unknown_scaffolding_and_returns_its_directories() {
        let pc = Pc::new();
        let sync_root = tempdir().unwrap();
        pc.folder("bt", None, "Backups", 3);
        pc.folder("bg", None, "Backups", 0);
        pc.folder("u", Some("bg"), "Backups/unknown", 0);
        pc.folder("d", Some("u"), "Backups/unknown/Documents", 0);
        pc.folder("o", Some("d"), "Backups/unknown/Documents/Old", 0);
        for (op_id, id, parent) in [("op-bg", "bg", None), ("op-u", "u", Some("bg")), ("op-d", "d", Some("u")), ("op-o", "o", Some("d"))] {
            pc.create_folder(op_id, id, parent);
        }
        std::fs::create_dir_all(sync_root.path().join("Backups/unknown/Documents/Old")).unwrap();
        let on_disk = |dir: &str| tree_holds_a_file(&sync_root.path().join(dir));
        let dry = pc.db.reconcile_dry_run(&opts(), &on_disk).unwrap();

        let counts = pc.db.reconcile_held_queue(&opts(), NOW, &on_disk);
        assert!(counts.is_ok(), "{counts:?}");
        let dirs = ["Backups/unknown/Documents/Old", "Backups/unknown/Documents", "Backups/unknown"];
        let expected = Counts {
            remapped: 1,
            scaffold_dropped: 3,
            scaffold_dirs: dirs.iter().map(|dir| dir.to_string()).collect(),
            ..Counts::default()
        };
        assert_eq!(counts.unwrap(), expected);
        assert_eq!(dry, expected, "the dry run shows what the run does");
        assert_eq!(pc.count("SELECT COUNT(*) FROM files WHERE path LIKE 'Backups/unknown%'", []), 0);
        assert_eq!(pc.count("SELECT COUNT(*) FROM operation_queue", []), 0);
        for id in ["bg", "u", "d", "o"] {
            assert_eq!(pc.references(id), 0, "{id}");
        }
        assert!(sync_root.path().join("Backups/unknown/Documents/Old").is_dir(), "the pass removes no directory");
        assert_eq!(pc.db.reconcile_held_queue(&opts(), NOW + 60, &on_disk).unwrap(), Counts::default());
    }

    #[cfg(not(target_os = "macos"))]
    fn unpause(pc: &Pc, op_id: &str) {
        pc.sql("UPDATE operation_queue SET paused_reason = NULL WHERE op_id = ?1", params![op_id]);
    }

    /// A folder made here, not on the server yet, whose create is due, and a new file in it whose upload is due too.
    #[cfg(not(target_os = "macos"))]
    fn new_folder_with_a_new_file(pc: &Pc) {
        pc.folder("new", None, "New", 0);
        pc.create_folder("op-new", "new", None);
        unpause(pc, "op-new");
        pc.file("f", Some("new"), "New/f.txt", 0);
        pc.upload("op-f", "f", Some("new"), 20);
    }

    /// D7, I5 (2226's verification line: "not retried"): a change under a folder whose create has not landed is not
    /// due, its claim claims nothing, and it keeps its attempts and its retry time.
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn an_upload_under_a_folder_not_yet_on_the_server_is_not_due_and_keeps_its_attempts() {
        let pc = Pc::new();
        new_folder_with_a_new_file(&pc);
        assert_eq!(pc.due(), vec!["op-new".to_string()], "only the folder's create is due");
        assert!(matches!(pc.db.claim_operation("op-f", NOW).unwrap(), ClaimOutcome::Gone), "the change is not claimed");
        assert_eq!(pc.op("op-f"), (Some("new".to_string()), 0, 0, None, None), "attempts and retry time untouched");
        assert_eq!(pc.count("SELECT COUNT(*) FROM operation_queue WHERE op_id = 'op-f' AND claim_id IS NULL", []), 1);
    }

    /// D7: once the folder's create lands (its op is gone), the change under it is due and is claimed.
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn it_becomes_due_once_its_folder_lands() {
        let pc = Pc::new();
        new_folder_with_a_new_file(&pc);
        assert!(!pc.due().contains(&"op-f".to_string()), "not due while its folder's create is queued");
        let ClaimOutcome::Claimed(folder) = pc.db.claim_operation("op-new", NOW).unwrap() else {
            panic!("the folder's create is claimed")
        };
        assert!(pc.db.finish_claimed("op-new", &folder.claim_id, None).unwrap());
        assert_eq!(pc.due(), vec!["op-f".to_string()]);
        assert!(matches!(pc.db.claim_operation("op-f", NOW).unwrap(), ClaimOutcome::Claimed(_)));
    }

    /// D7, D13: a change under a folder whose create is held (paused, or out of attempts) is counted as waiting for its
    /// folder, not as waiting; under a folder whose create can still run it is waiting.
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn the_hold_summary_counts_children_of_a_held_folder_as_waiting_for_their_folder() {
        let pc = Pc::new();
        new_folder_with_a_new_file(&pc);
        let for_folder = HeldGroup { reason: "waiting_for_folder".to_string(), count: 1, oldest_at: 20, newest_at: 20 };

        pc.pause("op-new", "auth");
        let hold = pc.db.queue_hold_summary().unwrap();
        assert_eq!((hold.waiting, hold.gave_up), (0, 0));
        assert_eq!(
            hold.held,
            vec![HeldGroup { reason: "auth".to_string(), count: 1, oldest_at: 1, newest_at: 1 }, for_folder.clone()]
        );

        unpause(&pc, "op-new");
        pc.gave_up("op-new", SERVER_ERROR);
        let hold = pc.db.queue_hold_summary().unwrap();
        assert_eq!((hold.waiting, hold.gave_up), (0, 1));
        assert_eq!(hold.held, vec![for_folder]);

        pc.sql("UPDATE operation_queue SET attempts = 0 WHERE op_id = 'op-new'", []);
        let hold = pc.db.queue_hold_summary().unwrap();
        assert_eq!((hold.waiting, hold.gave_up), (2, 0), "the folder's create can run: both are waiting");
        assert!(hold.held.is_empty(), "no group: {:?}", hold.held);
    }

    /// D7 below one level: a folder made here inside a held folder waits for it, and so does a change inside that
    /// folder. Counted as waiting, it would show the queue busy for as long as the hold lasts.
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn a_change_in_a_folder_inside_a_held_folder_also_waits_for_its_folder() {
        let pc = Pc::new();
        pc.folder("new", None, "New", 0);
        pc.create_folder("op-new", "new", None);
        pc.folder("sub", Some("new"), "New/Sub", 0);
        pc.create_folder("op-sub", "sub", Some("new"));
        unpause(&pc, "op-sub");
        pc.file("f", Some("sub"), "New/Sub/f.txt", 0);
        pc.upload("op-f", "f", Some("sub"), 20);
        assert!(pc.due().is_empty(), "nothing runs before its folder");
        let hold = pc.db.queue_hold_summary().unwrap();
        assert_eq!((hold.waiting, hold.gave_up), (0, 0));
        assert_eq!(
            hold.held,
            vec![
                HeldGroup { reason: "auth".to_string(), count: 1, oldest_at: 1, newest_at: 1 },
                HeldGroup { reason: "waiting_for_folder".to_string(), count: 2, oldest_at: 1, newest_at: 20 },
            ]
        );
    }

    /// Task 2243's budget holds with the D7 group in it: 300 held folders and 2,700 changes waiting in them are
    /// counted from the indexes, without reading the changes themselves (about 250 pages here).
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn the_waiting_for_folder_group_reads_indexes_not_the_changes() {
        let pc = Pc::new();
        {
            let mut conn = pc.db.0.lock().unwrap();
            let tx = conn.transaction().unwrap();
            let id = |n: i64| format!("0a000000-0000-4000-8000-{n:012}");
            for folder in 0..300_i64 {
                tx.execute(
                    "INSERT INTO operation_queue (op_id, kind, file_id, target_path, paused_reason, created_at)
                     VALUES (?1, 'create_folder', ?2, ?3, 'auth', 1)",
                    params![id(folder), id(folder), format!("Projects/Folder {folder:05}")],
                )
                .unwrap();
                for n in 0..9_i64 {
                    let op = 1_000 + folder * 9 + n;
                    tx.execute(
                        "INSERT INTO operation_queue (op_id, kind, file_id, parent_id, target_path, metadata_json, created_at)
                         VALUES (?1, 'upload_version', ?2, ?3, ?4, ?5, ?6)",
                        params![
                            id(op),
                            id(op),
                            id(folder),
                            format!("Projects/Folder {folder:05}/draft-{n}.txt"),
                            format!("{{\"operation\":\"create_file\",\"name_ct\":\"{}\"}}", "x".repeat(120)),
                            10 + op,
                        ],
                    )
                    .unwrap();
                }
            }
            tx.commit().unwrap();
        }
        let db = StateDb::open(pc.dir.path().join("state.db")).unwrap();
        let hold = db.queue_hold_summary().unwrap();
        let reads = db.page_reads();
        assert_eq!(hold.waiting, 0);
        assert_eq!(
            hold.held,
            vec![
                HeldGroup { reason: "auth".to_string(), count: 300, oldest_at: 1, newest_at: 1 },
                HeldGroup { reason: "waiting_for_folder".to_string(), count: 2_700, oldest_at: 1_010, newest_at: 3_709 },
            ]
        );
        assert!(reads <= 80, "the hold summary (the open and its reads) read {reads} pages from disk");
    }

    /// D7 and the tray's activity: a change waiting for a held folder is not transfer work, or the engine would show
    /// "syncing" for as long as the hold lasts. Under a folder whose create can still run it is.
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn a_change_waiting_for_a_held_folder_is_not_transfer_work() {
        let pc = Pc::new();
        new_folder_with_a_new_file(&pc);
        pc.pause("op-new", "auth");
        assert_eq!(pc.db.transfer_backlog(NOW).unwrap().upload_files, 0);
        unpause(&pc, "op-new");
        assert_eq!(pc.db.transfer_backlog(NOW).unwrap().upload_files, 1, "it is sent once its folder lands");
    }

    /// D7: the diagnostics' `due` is what the queue would send now, gate included.
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn the_diagnostics_due_count_is_what_the_queue_would_send() {
        let pc = Pc::new();
        new_folder_with_a_new_file(&pc);
        assert_eq!(pc.due().len(), 1);
        assert_eq!(pc.db.queue_diagnostics(NOW).unwrap().due, 1);
    }
}
