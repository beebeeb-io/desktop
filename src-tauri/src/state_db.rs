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

/// Spec D8: a fresh retry budget, due from now. Never touches `created_at` or `rowid` (the queue's order). A change
/// held for the person keeps its pause (D8 amendment). Any other goes under `auth` (Goal 2, I6; cross-lane review
/// P2-2): its name was sealed under the key in force when it was queued, so the key check that follows every pass
/// (D11, D12) resumes it under this session's key or holds it as `key_replaced`, as every other `auth` change.
fn rearm_operation_conn(conn: &Connection, op_id: &str, now: i64) -> Result<()> {
    conn.execute(
        "UPDATE operation_queue
         SET attempts = 0, next_retry_at = ?2, last_error = NULL, updated_at = ?2, paused_reason = COALESCE(paused_reason, ?3)
         WHERE op_id = ?1",
        params![op_id, now, OperationPauseReason::Auth.as_str()],
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


#[cfg(test)]
thread_local! {
    /// How many of this thread's next landings fail, and whether as "database is locked".
    /// A thread-local, so parallel tests never share it: under `#[tokio::test]` the landing
    /// runs on the test's own thread.
    static FAIL_LANDINGS: std::cell::Cell<(u32, bool)> = const { std::cell::Cell::new((0, false)) };
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
    /// L7d: device-wide, durable format knowledge; contains no key material.
    /// Nothing the server says and no sign-out path removes an entry (see
    /// `purge_all_local_state`, `clear_account_data` and `finish_windows_signout`).
    pub fn has_seen_folder_custody(&self, folder_id: &str) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        Self::has_seen_folder_custody_conn(&conn, folder_id)
    }

    /// L7d: the set stores canonical lowercase hyphenated UUIDs only. A write
    /// with any other form is refused, so an id shape the server reports can
    /// never create a second entry for a folder this device has seen.
    fn custody_seen_key(folder_id: &str) -> Result<String> {
        match uuid::Uuid::parse_str(folder_id) {
            Ok(canonical) if canonical.hyphenated().to_string() == folder_id => {
                Ok(format!("shared_folder_custody_seen:{folder_id}"))
            }
            _ => Err(rusqlite::Error::InvalidParameterName(folder_id.to_string())),
        }
    }

    /// The same key for a read, canonicalized: a folder id reported in another
    /// UUID form (upper case, no hyphens, braced) maps to the entry its
    /// canonical form stored, so a server cannot shrink the set by re-spelling
    /// an id. An id that is not a UUID cannot be in the set (a real folder id
    /// always is), so it reads as never seen rather than failing every read.
    fn custody_lookup_key(folder_id: &str) -> Option<String> {
        uuid::Uuid::parse_str(folder_id)
            .ok()
            .map(|canonical| format!("shared_folder_custody_seen:{}", canonical.hyphenated()))
    }

    fn has_seen_folder_custody_conn(conn: &Connection, folder_id: &str) -> Result<bool> {
        let Some(key) = Self::custody_lookup_key(folder_id) else {
            return Ok(false);
        };
        Ok(conn
            .query_row("SELECT 1 FROM sync_state WHERE key=?1", [key], |_| Ok(()))
            .optional()?
            .is_some())
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
        let key = Self::custody_seen_key(folder_id)?;
        let conn = self.0.lock().expect("state_db mutex poisoned");
        if self.1.load(std::sync::atomic::Ordering::Acquire) != revision {
            return Err(rusqlite::Error::InvalidQuery);
        }
        conn.execute(
            "INSERT INTO sync_state (key,value) VALUES (?1,'1') ON CONFLICT(key) DO NOTHING",
            [key],
        )?;
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
    /// Task 2279 item 1: index work that cannot run before the root has
    /// custody. It is kept durably instead of being cleared, but is not part of
    /// the dirty set, so maintenance does not retry it every 30 seconds.
    pub fn defer_shared_index_until_custody(&self, folder_id: &str) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute(
            "INSERT INTO sync_state (key,value) VALUES (?1,'1') ON CONFLICT(key) DO NOTHING",
            [format!("shared_folder_index_deferred:{folder_id}")],
        )?;
        Ok(())
    }
    /// Custody became available for the root: revive the deferred repair by
    /// marking the index dirty again. Returns whether work was waiting.
    pub fn revive_deferred_shared_index(&self, folder_id: &str) -> Result<bool> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        // Both statements in one transaction: a failure or crash between the delete and the dirty marker must not
        // consume the deferral, or nobody would ever schedule the repair again.
        let tx = conn.transaction()?;
        let removed = tx.execute(
            "DELETE FROM sync_state WHERE key=?1",
            [format!("shared_folder_index_deferred:{folder_id}")],
        )?;
        if removed > 0 {
            tx.execute(
                "INSERT INTO sync_state (key,value) VALUES (?1,'1') ON CONFLICT(key) DO UPDATE SET value=CAST(CAST(value AS INTEGER)+1 AS TEXT)",
                [format!("shared_folder_index_dirty:{folder_id}")],
            )?;
        }
        tx.commit()?;
        Ok(removed > 0)
    }
    /// A repair that got its custody key runs to completion, so the provisional
    /// deferral it wrote must go. Unlike `revive_deferred_shared_index`, this
    /// does not mark the index dirty: the repair itself just ran.
    pub fn cancel_shared_index_deferral(&self, folder_id: &str) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute(
            "DELETE FROM sync_state WHERE key=?1",
            [format!("shared_folder_index_deferred:{folder_id}")],
        )?;
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
        let mut conn = Connection::open(path)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        // Task 2278: the journal mode cannot be changed inside a transaction, so it is set first, on its own.
        enable_wal(&conn)?;
        // Task 2278: the schema and every column migration run under ONE write lock. Two connections opening a
        // fresh database at the same moment used to both see a column missing — `has_column`, then `ALTER TABLE
        // ... ADD COLUMN`, with no transaction around them — and the second ALTER returned "duplicate column
        // name: fetched_min_version" (macOS CI, 2026-10-29, `a_sign_out_leaves_no_account_row_and_no_owner`).
        // A second opener now waits on the busy timeout above for this transaction, then finds the schema done.
        {
            let migration = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            Self::migrate_schema(&migration)?;
            migration.commit()?;
        }
        Ok(Self(Mutex::new(conn), std::sync::atomic::AtomicU64::new(0)))
    }

    /// The schema of a state database, created if missing and brought up to today's columns. Runs inside
    /// [`StateDb::open`]'s `BEGIN IMMEDIATE`, so one opener applies it and any other opener waits and then finds it
    /// finished. `CREATE TABLE IF NOT EXISTS` covers both the first-run case and a re-open of a database that already
    /// has every column.
    fn migrate_schema(conn: &Connection) -> Result<()> {
        conn.execute_batch(
            "
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
        ensure_column(conn, "files", "remote_updated_at", "INTEGER NOT NULL DEFAULT 0")?;
        ensure_column(conn, "files", "namespace", "TEXT NOT NULL DEFAULT 'my_files'")?;
        ensure_column(conn, "files", "parent_id", "TEXT")?;
        ensure_column(conn, "files", "shared_root_id", "TEXT")?;
        ensure_column(conn, "files", "share_id", "TEXT")?;
        ensure_column(conn, "files", "owner_email", "TEXT")?;
        ensure_column(conn, "files", "permission_bits", "INTEGER NOT NULL DEFAULT 0")?;
        ensure_column(conn, "files", "item_kind", "TEXT NOT NULL DEFAULT 'file'")?;
        ensure_column(conn, "files", "content_type", "TEXT")?;
        ensure_column(conn, "files", "current_version", "INTEGER NOT NULL DEFAULT 0")?;
        ensure_column(conn, "files", "current_object_version_id", "TEXT")?;
        ensure_column(conn, "files", "local_base_version", "INTEGER NOT NULL DEFAULT 0")?;
        ensure_column(conn, "files", "local_hash", "TEXT")?;
        // Task 1640 (review N1, F1): what the fetches put on disk (see `record_fetched_version` and
        // docs/design/1640-local-content-identity.md). -1, the default, is "not known": every row from
        // before this column, and every new one until its placeholder is created where nothing was. A
        // new name (self-review #2): a database from a round-2 build has `fetched_version` DEFAULT 0, which
        // no ALTER can change; that column is left unused.
        ensure_column(conn, "files", "fetched_min_version", "INTEGER NOT NULL DEFAULT -1")?;
        ensure_column(conn, "files", "fetched_hash", "TEXT")?;
        ensure_column(conn, "files", "cache_path", "TEXT")?;
        ensure_column(conn, "files", "cache_bytes", "INTEGER NOT NULL DEFAULT 0")?;
        ensure_column(conn, "files", "pin_state", "TEXT NOT NULL DEFAULT 'inherit'")?;
        ensure_column(
            conn,
            "files",
            "inherited_pin_state",
            "TEXT NOT NULL DEFAULT 'unpinned'",
        )?;
        ensure_column(conn, "files", "last_opened_at", "INTEGER NOT NULL DEFAULT 0")?;
        ensure_column(conn, "files", "last_sync_at", "INTEGER NOT NULL DEFAULT 0")?;
        ensure_column(conn, "operation_queue", "last_error_class", "TEXT")?;
        ensure_column(conn, "operation_queue", "paused_reason", "TEXT")?;
        // Task 0811: additive backup-origin tag. Existing rows migrate to NULL
        // (untagged → treated as normal ops, never purged by a folder disable).
        ensure_column(conn, "operation_queue", "backup_source_key", "TEXT")?;
        // Task 1697: which local write created the row (watcher paths set this;
        // Finder-queue paths do not) — the change-log recorder reads it.
        ensure_column(conn, "files", "creator_for_fp", "TEXT")?;
        // Spec 2026-10-09 §5.2: the write token. Additive and nullable, no backfill;
        // only the dedicated functions write them.
        ensure_column(conn, "files", "held_write_id", "TEXT")?;
        ensure_column(conn, "files", "held_base", "INTEGER")?;
        ensure_column(conn, "files", "held_version", "INTEGER")?;
        ensure_column(conn, "files", "held_object_version_id", "TEXT")?;
        ensure_column(conn, "files", "version_filled", "INTEGER NOT NULL DEFAULT 0")?;
        ensure_column(conn, "operation_queue", "write_id", "TEXT")?;
        ensure_column(conn, "operation_queue", "write_origin", "TEXT")?;
        ensure_column(conn, "operation_queue", "after_write_id", "TEXT")?;
        ensure_column(conn, "operation_queue", "base_pending", "INTEGER NOT NULL DEFAULT 0")?;
        ensure_column(conn, "operation_queue", "claim_id", "TEXT")?;
        ensure_column(conn, "operation_queue", "claimed_at", "INTEGER")?;
        conn.execute_batch(
            "
            CREATE INDEX IF NOT EXISTS idx_files_namespace ON files(namespace);
            CREATE INDEX IF NOT EXISTS idx_files_shared_root ON files(shared_root_id);
            -- Task 2229: `FILE_BY_PATH_SQL`, once per file and folder on every enumeration pass.
            CREATE INDEX IF NOT EXISTS idx_files_path ON files(path);
            -- Task 2243: the tray's recent list reads the newest rows through it, not every row.
            CREATE INDEX IF NOT EXISTS idx_files_modified_at ON files(modified_at);
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
        ensure_column(conn, "upload_resume", "completed_version", "INTEGER")?;
        ensure_column(conn, "upload_resume", "completed_object_version_id", "TEXT")?;
        ensure_column(conn, "upload_resume", "completed_mime_type", "TEXT")?;
        // SF-0001 (L13): sessions persisted before this flag existed were opened
        // under a server-chosen id; only a self-minted CREATE may be resumed.
        ensure_column(conn, "upload_resume", "self_minted", "INTEGER NOT NULL DEFAULT 0")?;
        // Task 2270 (spec M1): a finalization that cannot complete for a lasting reason is held for review.
        ensure_column(conn, "upload_finalizations", "review", "TEXT")?;
        ensure_column(conn, "upload_finalizations", "review_at", "INTEGER")?;
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
        // changes in a folder, and with `idx_files_parent_id` serves the reconcile pass's remaps (`REMAP_FILES_SQL`,
        // `REMAP_OPS_SQL`), which would otherwise scan every row once per remapped folder. Indexes only, no table or
        // column change. macOS has neither the parent-first gate nor the pass.
        #[cfg(not(target_os = "macos"))]
        conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_files_parent_id ON files(parent_id);
             CREATE INDEX IF NOT EXISTS idx_operation_queue_create_folder
                 ON operation_queue(parent_id, file_id, paused_reason, attempts, max_attempts)
                 WHERE kind = 'create_folder';
             CREATE INDEX IF NOT EXISTS idx_operation_queue_parent
                 ON operation_queue(parent_id, paused_reason, attempts, max_attempts, created_at);",
        )?;
        // Task 1640 (review F5): why a file deleted elsewhere was kept (`KeptReason`).
        ensure_column(conn, "kept_copies", "reason", "TEXT NOT NULL DEFAULT 'copy'")?;
        // Task 1640 (review-151d P2-1): the person dismissed the entry of a file still kept where it
        // is; the row stays as a tombstone, so its folder is still the server's deletion.
        ensure_column(conn, "kept_copies", "dismissed", "INTEGER NOT NULL DEFAULT 0")?;
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
        Ok(())
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

    /// Task 2229: a stamp of the rows [`Self::file_rows_at_path`] finds at `path`, over every column,
    /// so any write to any of them changes it; `None` without a row. Valid within this process only.
    /// Task 2271: every row, not only the first: when several share a path the scan may judge the
    /// bytes by another one.
    pub fn file_row_stamp_by_path(&self, path: &str) -> Result<Option<u64>> {
        use rusqlite::types::ValueRef;
        use std::hash::{Hash, Hasher};
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare_cached(FILE_ROW_BY_PATH_SQL)?;
        for candidate in [path.to_string(), toggled_slash(path)] {
            let mut rows = stmt.query(params![candidate])?;
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            let mut found = false;
            while let Some(row) = rows.next()? {
                found = true;
                for column in 0..row.as_ref().column_count() {
                    match row.get_ref(column)? {
                        ValueRef::Null => 0u8.hash(&mut hasher),
                        ValueRef::Integer(i) => (1u8, i).hash(&mut hasher),
                        ValueRef::Real(f) => (2u8, f.to_bits()).hash(&mut hasher),
                        ValueRef::Text(t) => (3u8, t).hash(&mut hasher),
                        ValueRef::Blob(b) => (4u8, b).hash(&mut hasher),
                    }
                }
            }
            if found {
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

    /// Task 2271: every row at `path`, in the order [`Self::get_file_by_path`] meets them (its first
    /// is that one), in the stored slash shape it finds; empty without a row.
    pub fn file_rows_at_path(&self, path: &str) -> Result<Vec<FileEntry>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare_cached(FILE_BY_PATH_SQL)?;
        for candidate in [path.to_string(), toggled_slash(path)] {
            let entries = stmt
                .query_map(params![candidate], file_entry_from_path_row)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            if !entries.is_empty() {
                return Ok(entries);
            }
        }
        Ok(Vec::new())
    }

    fn get_file_by_exact_path(&self, path: &str) -> Result<Option<FileEntry>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare_cached(FILE_BY_PATH_SQL)?;
        let mut rows = stmt.query(params![path])?;
        match rows.next()? {
            Some(row) => Ok(Some(file_entry_from_path_row(row)?)),
            None => Ok(None),
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
            "SELECT op_id,local_file_id,server_file_id,target_path,payload_path,stamped,review FROM upload_finalizations",
        )?;
        stmt.query_map([], |r| {
            Ok(UploadFinalization {
                op_id: r.get(0)?,
                local_file_id: r.get(1)?,
                server_file_id: r.get(2)?,
                target_path: r.get(3)?,
                payload_path: r.get(4)?,
                stamped: r.get(5)?,
                review: r.get(6)?,
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
    /// Task 2270 (spec R2): hold the row for review with a reason label, at `at` (Unix seconds). It is not retried
    /// on every tick after this; the tray counts it as `not_marked_synced`.
    #[cfg(any(target_os = "windows", test))]
    pub fn mark_upload_finalization_review(&self, op_id: &str, reason: &str, at: i64) -> Result<()> {
        self.0.lock().unwrap().execute(
            "UPDATE upload_finalizations SET review=?2, review_at=?3 WHERE op_id=?1",
            params![op_id, reason, at],
        )?;
        Ok(())
    }
    /// Task 2270 (spec F8, self-review of #195): a row in review that meets a transient error leaves review, so the
    /// next tick tries it again and the tray stops counting it.
    #[cfg(any(target_os = "windows", test))]
    pub fn clear_upload_finalization_review(&self, op_id: &str) -> Result<()> {
        self.0.lock().unwrap().execute(
            "UPDATE upload_finalizations SET review=NULL, review_at=NULL WHERE op_id=?1",
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
        // Task 2270 (spec X1): a finalization held for review does not refuse sign-out; it goes with the account data
        // below, in this transaction. A pending one still refuses it.
        let remaining: i64 = tx.query_row(
            "SELECT (SELECT COUNT(*) FROM staged_payloads)
                  + (SELECT COUNT(*) FROM upload_finalizations WHERE review IS NULL)",
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
        //
        // L7d: the custody memory is the one exception. It belongs to the device, so a sign-out (user-started or
        // forced) keeps it; removing it would let a server 404 downgrade this PC to the legacy reader.
        for table in ACCOUNT_TABLES.iter().chain(DEVICE_TABLES.iter()) {
            tx.execute(&format!("DELETE FROM {table}"), [])?;
        }
        tx.execute(
            "DELETE FROM sync_state WHERE key NOT LIKE 'shared_folder_custody_seen:%'",
            [],
        )?;
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
    /// L7d: the custody memory survives, because it belongs to the device and an account switch must not
    /// shrink it. Only `shared_folder_custody_seen:%` is kept; every other `sync_state` row is cleared.
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
        tx.execute(
            "DELETE FROM sync_state WHERE key NOT LIKE 'shared_folder_custody_seen:%'",
            [],
        )?;
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
        // L7d: the custody memory (`shared_folder_custody_seen:%`) is deliberately NOT deleted here.
        // It is device-wide and monotonic: no sign-out, sign-in, 401 or account switch removes it.
        tx.execute("DELETE FROM sync_state WHERE key LIKE 'shared_folder_index_dirty:%' OR key LIKE 'shared_folder_index_deferred:%' OR key='owned_key_retry_after'", [])?;

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
    /// for any other reason are untouched. Returns how many were re-marked. A Mac's re-sign-in path holds them; on
    /// Windows and Linux the engine's key check does, when a queued name does not open under the session's key (spec
    /// D12, task 1888).
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

    /// Spec D12 (task 1888): every operation paused for `auth`, for the key check before a resume (Windows and Linux).
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    pub fn operations_paused_for_auth(&self) -> Result<Vec<PendingOperation>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare(&format!(
            "SELECT {PENDING_OPERATION_COLUMNS} FROM operation_queue WHERE paused_reason = ?1 ORDER BY rowid"
        ))?;
        let rows = stmt.query_map(params![OperationPauseReason::Auth.as_str()], pending_operation_from_row)?;
        rows.collect()
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
        // Task 2270 (spec U1): uploads this PC could not mark as synced for a lasting reason, dated by when each was
        // held. Not a pause reason, and not an operation: the upload itself landed.
        let (count, oldest_at, newest_at) = conn.query_row(
            "SELECT COUNT(*), COALESCE(MIN(review_at), 0), COALESCE(MAX(review_at), 0)
             FROM upload_finalizations WHERE review IS NOT NULL",
            [],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?)),
        )?;
        if count > 0 {
            add("not_marked_synced".to_string(), count, oldest_at, newest_at);
        }
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
                acked_chunks, metadata_applied, is_create, self_minted, updated_at
             )
             SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, strftime('%s','now')
             WHERE EXISTS (SELECT 1 FROM operation_queue WHERE op_id = ?1 AND claim_id = ?14)
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
                self_minted = excluded.self_minted,
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
                resume.self_minted,
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
                acked_chunks, metadata_applied, is_create, self_minted, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, strftime('%s','now'))
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
                self_minted = excluded.self_minted,
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
                resume.self_minted,
            ],
        )?;
        Ok(())
    }

    pub fn get_upload_resume(&self, op_id: &str) -> Result<Option<UploadResume>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.query_row(
            "SELECT op_id, payload_path, payload_size, payload_mtime_ns, upload_session_id,
                    server_file_id, object_version_id, chunk_size_bytes, chunk_count,
                    acked_chunks, metadata_applied, is_create, self_minted, completed_version,
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
                    self_minted: row.get(12)?,
                    completed_version: row.get(13)?,
                    completed_object_version_id: row.get(14)?,
                    completed_mime_type: row.get(15)?,
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
















#[cfg(test)]
mod tests {
    mod open_and_cache;



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











    mod signout_and_activity;

    fn finalization(op: &str) -> UploadFinalization {
        UploadFinalization {
            op_id: op.into(),
            local_file_id: format!("local-{op}"),
            server_file_id: format!("server-{op}"),
            target_path: format!("dir/{op}.txt"),
            payload_path: format!("proof-{op}"),
            stamped: false,
            review: None,
        }
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



    use tempfile::tempdir;




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













    mod files_and_purge;






















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

    mod prune_and_finder;












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







    mod landing_and_snapshots;












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










    // ── bandwidth_samples (task 0810 — P3) ───────────────────────────────────

    mod change_log_and_account;



    // ── Mbps <-> kbps conversion (task 0810 — E) ─────────────────────────────
    //
    // The desktop UI exposes bandwidth caps as Mbps; config.rs stores them as
    // kbps.  These tests encode the conversion contract so a future refactor
    // can't silently break it.


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




    // ── R10 (spec 2026-10-06 §5.6): local data is bound to the account that created it ──















    // ── Rule 5: the item's status follows the queue (spec §9.1–§9.2) ──────────

    mod startup_and_accepts;



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

    #[cfg(not(target_os = "macos"))]
    /// The query plan of `sql`, one line per step.
    fn query_plan(db: &StateDb, sql: &str) -> String {
        let conn = db.0.lock().unwrap();
        let mut stmt = conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
        let args = rusqlite::params_from_iter(std::iter::repeat("x").take(stmt.parameter_count()));
        let steps = stmt.query_map(args, |row| row.get::<_, String>(3)).unwrap();
        steps.map(Result::unwrap).collect::<Vec<_>>().join("\n")
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn a_remap_reads_the_files_table_through_an_index() {
        let pc = Pc::new();
        for sql in [REMAP_FILES_SQL, REMAP_OPS_SQL, GHOST_REFERENCES_SQL] {
            let plan = query_plan(&pc.db, sql);
            assert!(!plan.contains("SCAN files"), "a remap must not scan every row:\n{plan}");
            assert!(!plan.contains("SCAN operation_queue"), "a remap must not scan every op:\n{plan}");
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
        // Re-armed into the key check (cross-lane review P2-2), which resumes it under this session's key.
        assert_eq!(pc.op("op-f1"), (Some("t1".to_string()), 0, NOW, None, Some("auth".to_string())));
        assert_eq!(pc.db.resume_operations_paused_for_auth(NOW).unwrap(), 1);
        assert_eq!(pc.due(), vec!["op-f1".to_string()]);
    }

    /// Goal 2, cross-lane review P2-2: a change D8 gives its retries back to was queued with its name sealed under the
    /// key in force then, like every `auth` change. The pass puts it under `auth`, so the key check that follows the
    /// pass decides it too: under a replaced key it is held as `key_replaced`, never sent.
    #[test]
    fn a_change_the_pass_rearms_goes_through_the_key_check() {
        let pc = Pc::new();
        pc.ghost_with_twin(1, "Photos");
        let counts = pc.db.reconcile_held_queue(&opts(), NOW, &holds_a_file).unwrap();
        assert_eq!(counts, Counts { remapped: 1, rearmed: 1, ..Counts::default() });
        assert!(pc.due().is_empty(), "re-armed into the key check, never straight to due: {:?}", pc.due());
        assert_eq!(pc.db.hold_operations_paused_for_auth_as_key_replaced(NOW).unwrap(), 1);
        assert_eq!(pc.op("op-f1"), (Some("t1".to_string()), 0, NOW, None, Some("key_replaced".to_string())));
        assert!(pc.due().is_empty());
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
        // Under `auth` until the key check (cross-lane review P2-2), it already holds its place: the newer one waits.
        assert!(matches!(pc.db.claim_operation("op-newer", NOW).unwrap(), ClaimOutcome::Wait));
        assert_eq!(pc.db.resume_operations_paused_for_auth(NOW).unwrap(), 1);
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

mod types;
use types::*;
pub use types::{UploadFinalization, FileStatus, PERMISSION_READ, PERMISSION_WRITE, PERMISSION_SHARE, PERMISSION_OWNER, Namespace, ItemKind, PinState, OperationKind, FpChangeKind, FileChange, PrunedRow, FileEntry, FileContractState, RevokedSharedCache, LocalStatePurge, LocalActivityKind, KeptReason, KeptCopyInput, KeptCopy, LocalActivityEventInput, LocalActivityEvent, TransferActivityInput, TransferActivity, TransferBacklog, PendingOperation, UploadResume, WriteOrigin, FinderWrite, ItemPresentation, WaitingRow, PurgeGuardRead, FinderEnqueue, FinderFolderRow, FinderAccept, RetireParked, FinderAcceptKind, AcceptOutcome, ParkReason, LandingInput, LandingOutcome, SnapshotVersion, ClaimedOp, TookOver, ClaimOutcome, EngineStartRepair, OperationPauseReason, QueueDiagnostics, QueueHold, HeldGroup, ProofRow, ProofOp, HeldChange, TreeRow, BandwidthSample};

mod sql;
use sql::*;
pub use sql::{GuardedRow, LOCAL_ACTIVITY_MAX_ROWS, ALIAS_MAX_AGE_SECS, TRANSFER_ACTIVITY_MAX_ROWS, decode_os_state};
pub(crate) use sql::{PAGE_CACHE_FLOOR_KIB, PAGE_CACHE_CEILING_KIB, page_cache_limit_kib, tree_holds_a_file, fail_next_landings_for_test, fail_next_landings_as_locked_for_test, is_file_provider_trash};

mod conn;
use conn::*;
