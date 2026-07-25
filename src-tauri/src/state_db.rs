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

use rusqlite::{Connection, OptionalExtension, Result, params};
use serde::Serialize;
use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use std::sync::Mutex;

pub const LOCAL_ACTIVITY_MAX_ROWS: usize = 200;

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
    pub last_error: Option<String>,
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
        conn.execute_batch(
            "
            CREATE INDEX IF NOT EXISTS idx_files_namespace ON files(namespace);
            CREATE INDEX IF NOT EXISTS idx_files_shared_root ON files(shared_root_id);
            CREATE INDEX IF NOT EXISTS idx_operation_queue_paused ON operation_queue(paused_reason);
            CREATE INDEX IF NOT EXISTS idx_operation_queue_backup_source ON operation_queue(backup_source_key);
            ",
        )?;
        Ok(Self(Mutex::new(conn)))
    }

    /// Insert or update a row keyed by `file_id`. ON CONFLICT replaces
    /// the entire row except the primary key.
    pub fn upsert_file(&self, e: &FileEntry) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
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
        conn.execute("DELETE FROM files WHERE file_id = ?1", params![file_id])?;
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
    pub fn delete_file_subtree(&self, file_id: &str) -> Result<Vec<PrunedRow>> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
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
                    tx.execute("DELETE FROM files WHERE file_id = ?1", params![d.file_id])?;
                    removed.push(d);
                }
            }
        }

        tx.execute("DELETE FROM files WHERE file_id = ?1", params![root.file_id])?;
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
        let direct_children: Vec<PrunedRow> = {
            let mut stmt = tx.prepare("SELECT file_id, path, item_kind FROM files WHERE parent_id = ?1")?;
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
    ///    `finalize_local_upload_placeholder` only AFTER the upload completes),
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
        let candidates: Vec<PrunedRow> = {
            let mut stmt = tx.prepare(
                "SELECT file_id, path, item_kind FROM files
                 WHERE namespace = 'my_files'
                   AND status != 'uploading'
                   AND remote_updated_at < ?1
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
                        // any stale queued op against it is moot.
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
                        if removed_ids.insert(d.file_id.clone()) {
                            tx.execute("DELETE FROM files WHERE file_id = ?1", params![d.file_id])?;
                            pruned.push(d);
                        }
                    }
                }
            }

            tx.execute("DELETE FROM files WHERE file_id = ?1", params![row.file_id])?;
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
        conn.execute(
            "UPDATE files SET status = ?1 WHERE file_id = ?2",
            params![status.as_str(), file_id],
        )?;
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
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let deleted = conn.execute(
            "DELETE FROM operation_queue WHERE backup_source_key = ?1",
            params![source_key],
        )?;
        Ok(deleted)
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

    pub fn queue_diagnostics(&self, now: i64) -> Result<QueueDiagnostics> {
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

        let by_kind = count_queue_groups(&conn, "kind")?;
        let paused_by_reason = count_queue_groups(&conn, "paused_reason")?;
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

        Ok(QueueDiagnostics {
            queued,
            due,
            paused,
            by_kind,
            paused_by_reason,
            last_error: last_error.as_deref().map(redact_diagnostic_error),
            last_error_class,
        })
    }

    pub fn remove_operation(&self, op_id: &str) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute("DELETE FROM operation_queue WHERE op_id = ?1", params![op_id])?;
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

    pub fn mark_cached(&self, file_id: &str, cache_path: &str, cache_bytes: i64, opened_at: i64) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute(
            "UPDATE files
             SET cache_path = ?2,
                 cache_bytes = ?3,
                 last_opened_at = ?4,
                 status = CASE
                    WHEN status IN ('uploading', 'conflict', 'error') THEN status
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

fn redact_diagnostic_error(error: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut skip_next = false;
    for part in error.split_whitespace() {
        if skip_next {
            out.push("[redacted]".into());
            skip_next = false;
            continue;
        }
        let lower = part.to_ascii_lowercase();
        if lower == "bearer" || lower == "token" || lower == "authorization:" || lower == "session" {
            out.push(part.into());
            skip_next = true;
        } else if lower.starts_with("bearer=")
            || lower.starts_with("token=")
            || lower.starts_with("session_token=")
            || lower.starts_with("authorization=")
        {
            let key = part.split_once('=').map(|(k, _)| k).unwrap_or(part);
            out.push(format!("{key}=[redacted]"));
        } else if part.len() > 96 {
            out.push("[redacted]".into());
        } else {
            out.push(part.into());
        }
    }
    out.join(" ")
}

#[cfg(test)]
fn has_table(conn: &Connection, table: &str) -> Result<bool> {
    let mut stmt = conn.prepare("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1")?;
    let mut rows = stmt.query(params![table])?;
    Ok(rows.next()?.is_some())
}

#[cfg(test)]
mod tests {
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
    fn prune_absent_deletes_trashing_row_absent_from_snapshot_after_op_cleared() {
        // task 0802 — CONVERGENCE: the server-authoritative path. After
        // `api.trash_file` succeeds the TrashFile op is REMOVED (the `Trashing`
        // status, not the op, is now the durable marker). Later the trash finally
        // propagates and the file is ABSENT from `/sync/snapshot`. At that point
        // the op-less `Trashing` row MUST be pruned — there is no `operation_queue`
        // row to protect it and `prune_absent` has no blanket "skip Trashing" rule
        // — giving final convergence with the (already-removed) on-disk placeholder.
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        // Trashing row, NO pending op (op was dropped on a successful trash).
        seed_own_row(&db, "trashed-gone", FileStatus::Trashing, 10);

        // Snapshot no longer lists the file (server trash has propagated).
        let seen: HashSet<String> = ["still-here".to_string()].into_iter().collect();
        let pruned = db.prune_absent(&seen, FAR_FUTURE).unwrap();

        assert_eq!(
            pruned.iter().map(|r| r.file_id.clone()).collect::<Vec<_>>(),
            vec!["trashed-gone".to_string()]
        );
        assert!(
            db.get_file("trashed-gone").unwrap().is_none(),
            "an op-less Trashing row absent from the snapshot must be pruned (final convergence)"
        );
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
}
