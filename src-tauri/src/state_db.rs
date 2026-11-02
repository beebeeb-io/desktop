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







#[cfg(test)]
thread_local! {
    /// How many of this thread's next landings fail, and whether as "database is locked".
    /// A thread-local, so parallel tests never share it: under `#[tokio::test]` the landing
    /// runs on the test's own thread.
    static FAIL_LANDINGS: std::cell::Cell<(u32, bool)> = const { std::cell::Cell::new((0, false)) };
}






































































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


















































    pub fn enqueue_operation(&self, op: &PendingOperation) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        insert_operation_conn(&conn, op)
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
mod removals;
mod sync;
mod landing;
mod schema;
mod custody;
mod files;
















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
    mod pass;
}

mod types;
use types::*;
pub use types::{UploadFinalization, FileStatus, PERMISSION_READ, PERMISSION_WRITE, PERMISSION_SHARE, PERMISSION_OWNER, Namespace, ItemKind, PinState, OperationKind, FpChangeKind, FileChange, PrunedRow, FileEntry, FileContractState, RevokedSharedCache, LocalStatePurge, LocalActivityKind, KeptReason, KeptCopyInput, KeptCopy, LocalActivityEventInput, LocalActivityEvent, TransferActivityInput, TransferActivity, TransferBacklog, PendingOperation, UploadResume, WriteOrigin, FinderWrite, ItemPresentation, WaitingRow, PurgeGuardRead, FinderEnqueue, FinderFolderRow, FinderAccept, RetireParked, FinderAcceptKind, AcceptOutcome, ParkReason, LandingInput, LandingOutcome, SnapshotVersion, ClaimedOp, TookOver, ClaimOutcome, EngineStartRepair, OperationPauseReason, QueueDiagnostics, QueueHold, HeldGroup, TreeRow, BandwidthSample};

mod sql;
use sql::*;
pub use sql::{GuardedRow, LOCAL_ACTIVITY_MAX_ROWS, ALIAS_MAX_AGE_SECS, TRANSFER_ACTIVITY_MAX_ROWS, decode_os_state};
pub(crate) use sql::{page_cache_limit_kib, tree_holds_a_file, is_file_provider_trash};

mod conn;
use conn::*;

mod reconcile;
use reconcile::*;
pub use reconcile::{RECONCILE_UNITS_PER_TX};
