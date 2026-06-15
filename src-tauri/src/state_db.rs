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

use rusqlite::{Connection, Result, params};
use serde::Serialize;
use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use std::sync::Mutex;

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
}

impl FileStatus {
    fn as_str(&self) -> &'static str {
        match self {
            FileStatus::CloudOnly => "cloud_only",
            FileStatus::Downloading => "downloading",
            FileStatus::Local => "local",
            FileStatus::Uploading => "uploading",
            FileStatus::Conflict => "conflict",
            FileStatus::Error => "error",
        }
    }
    fn from_str(s: &str) -> Self {
        match s {
            "cloud_only" => FileStatus::CloudOnly,
            "downloading" => FileStatus::Downloading,
            "local" => FileStatus::Local,
            "uploading" => FileStatus::Uploading,
            "conflict" => FileStatus::Conflict,
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
    fn as_str(&self) -> &'static str {
        match self {
            ItemKind::File => "file",
            ItemKind::Folder => "folder",
        }
    }

    fn from_str(s: &str) -> Self {
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
}

#[derive(Debug, Clone, PartialEq)]
pub struct FileContractState {
    pub file_id: String,
    pub namespace: Namespace,
    pub parent_id: Option<String>,
    pub shared_root_id: Option<String>,
    pub share_id: Option<String>,
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
                created_at INTEGER NOT NULL DEFAULT 0,
                updated_at INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS idx_operation_queue_due ON operation_queue(next_retry_at, created_at);
        ",
        )?;
        ensure_column(&conn, "files", "remote_updated_at", "INTEGER NOT NULL DEFAULT 0")?;
        ensure_column(&conn, "files", "namespace", "TEXT NOT NULL DEFAULT 'my_files'")?;
        ensure_column(&conn, "files", "parent_id", "TEXT")?;
        ensure_column(&conn, "files", "shared_root_id", "TEXT")?;
        ensure_column(&conn, "files", "share_id", "TEXT")?;
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
        conn.execute_batch(
            "
            CREATE INDEX IF NOT EXISTS idx_files_namespace ON files(namespace);
            CREATE INDEX IF NOT EXISTS idx_files_shared_root ON files(shared_root_id);
            CREATE INDEX IF NOT EXISTS idx_operation_queue_paused ON operation_queue(paused_reason);
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
            "SELECT file_id, path, status, size_bytes, modified_at, content_hash, remote_updated_at
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
            "SELECT file_id, path, status, size_bytes, modified_at, content_hash, remote_updated_at
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

    /// Return every row whose `status` matches. Used by the conflict
    /// resolution UI (status = Conflict) and the daemon's
    /// "what still needs uploading?" sweep (status = Uploading).
    pub fn list_by_status(&self, status: FileStatus) -> Result<Vec<FileEntry>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT file_id, path, status, size_bytes, modified_at, content_hash, remote_updated_at
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
            })
        })?;
        rows.collect()
    }

    /// Return every tracked file. Used by virtual filesystem directory
    /// enumeration to expose known cloud-only and local files.
    pub fn list_files(&self) -> Result<Vec<FileEntry>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT file_id, path, status, size_bytes, modified_at, content_hash, remote_updated_at
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
            })
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
               last_sync_at = ?17
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
                state.last_sync_at
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
                    inherited_pin_state, last_sync_at
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
                    inherited_pin_state, last_sync_at
             FROM files WHERE namespace = ?1 ORDER BY path ASC",
        )?;
        let rows = stmt.query_map(params![namespace.as_str()], |row| {
            Ok(FileContractState {
                file_id: row.get(0)?,
                namespace: Namespace::from_str(&row.get::<_, String>(1)?),
                parent_id: row.get(2)?,
                shared_root_id: row.get(3)?,
                share_id: row.get(4)?,
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
                last_error, last_error_class, paused_reason, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, NULL, NULL, ?14, ?15)
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
                    last_error, created_at, updated_at
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
                created_at: row.get(13)?,
                updated_at: row.get(14)?,
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
                    last_error, created_at, updated_at
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
                created_at: row.get(13)?,
                updated_at: row.get(14)?,
            })
        })?;
        rows.collect()
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
        let ids = {
            let mut stmt = tx.prepare(
                "
                WITH RECURSIVE tree(file_id) AS (
                    SELECT file_id FROM files WHERE file_id = ?1
                    UNION ALL
                    SELECT f.file_id
                    FROM files f
                    JOIN tree t ON f.parent_id = t.file_id
                )
                SELECT file_id FROM tree ORDER BY file_id ASC
                ",
            )?;
            let rows = stmt.query_map(params![root_file_id], |row| row.get::<_, String>(0))?;
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
        tx.execute(
            "
            WITH RECURSIVE tree(file_id) AS (
                SELECT file_id FROM files WHERE file_id = ?1
                UNION ALL
                SELECT f.file_id
                FROM files f
                JOIN tree t ON f.parent_id = t.file_id
            )
            UPDATE files
            SET inherited_pin_state = ?2, last_sync_at = ?3
            WHERE file_id IN (SELECT file_id FROM tree WHERE file_id != ?1)
            ",
            params![root_file_id, state_str, now],
        )?;
        tx.commit()?;
        Ok(ids)
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
        };
        db.upsert_file(&entry).unwrap();
        let got = db.get_file("abc123").unwrap().unwrap();
        assert_eq!(got.status, FileStatus::CloudOnly);
        assert_eq!(got.size_bytes, 1024);
        assert_eq!(got.remote_updated_at, 1700000000);
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
        };
        db.upsert_file(&e).unwrap();
        let conflicts = db.list_by_status(FileStatus::Conflict).unwrap();
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].file_id, "x1");
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
        })
        .unwrap();

        let contract = FileContractState {
            file_id: "file1".into(),
            namespace: Namespace::SharedWithMe,
            parent_id: Some("parent1".into()),
            shared_root_id: Some("root1".into()),
            share_id: Some("share1".into()),
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
        })
        .unwrap();
        let mut contract = db.get_file_contract_state(file_id).unwrap().unwrap();
        contract.parent_id = parent_id.map(str::to_string);
        contract.cache_bytes = cache_bytes;
        db.set_file_contract_state(&contract).unwrap();
    }
}
