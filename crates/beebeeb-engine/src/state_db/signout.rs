//! Account ownership, sign-out keep and purge, staged payloads (split out of `state_db.rs`).

use super::*;

/// The `sync_state` purge of a sign-out and of an account's rows: every row but the device's custody memory (L7d)
/// and B05e's share rows (`share_marks::SHARE_TRUST_ROWS`; lead ruling 2026-11-09). A reset of local data removes
/// the share rows first ([`StateDb::forget_share_trust_rows`]).
fn sign_out_sync_state_purge() -> String {
    format!(
        "DELETE FROM sync_state WHERE key NOT LIKE 'shared_folder_custody_seen:%'{}",
        share_marks::keep_share_trust_rows_sql()
    )
}

impl StateDb {
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
    /// Upload payload ownership outlives operation/resume rows and unlink errors.
    pub fn track_staged_payload(
        &self,
        path: &str,
        source: Option<&str>,
        completed: impl Into<PayloadCompleted>,
    ) -> Result<()> {
        let completed: PayloadCompleted = completed.into();
        self.0.lock().unwrap().execute(
            "INSERT INTO staged_payloads(path,source_path,completed) VALUES (?1,?2,?3)
             ON CONFLICT(path) DO UPDATE SET completed = MAX(completed, excluded.completed)",
            params![path, source, completed.as_bool()],
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
    #[cfg(any(target_os = "windows", test, feature = "test-support"))]
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
    #[cfg(any(target_os = "windows", test, feature = "test-support"))]
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
    #[cfg(any(target_os = "windows", test, feature = "test-support"))]
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
        // L7d: the custody memory is kept. It belongs to the device, so a sign-out (user-started or forced) keeps it;
        // removing it would let a server 404 downgrade this PC to the legacy reader. B05e's share rows are kept too
        // (`sign_out_sync_state_purge`).
        for table in ACCOUNT_TABLES.iter().chain(DEVICE_TABLES.iter()) {
            tx.execute(&format!("DELETE FROM {table}"), [])?;
        }
        tx.execute(&sign_out_sync_state_purge(), [])?;
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
                   -- LD-0113 (spec §6.1 point 4): the keep step and the purge guard skip backup ops. There is nothing
                   -- to keep (the source is the person's own file and stays where it is), and a backup op has no
                   -- staged copy, so it would read as a waiting write whose copy is missing.
                   AND q.origin != 'backup'
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
    #[cfg(any(target_os = "windows", test, feature = "test-support"))]
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
    #[cfg(any(target_os = "windows", test, feature = "test-support"))]
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
    #[cfg(any(target_os = "windows", test, feature = "test-support"))]
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
            let mut stmt =
                tx.prepare("SELECT op_id FROM operation_queue WHERE paused_reason IS NULL OR paused_reason != ?1")?;
            let rows = stmt.query_map(params![OperationPauseReason::FoundWhilePaused.as_str()], |row| {
                row.get::<_, String>(0)
            })?;
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
    #[cfg(any(target_os = "windows", test, feature = "test-support"))]
    pub fn found_while_paused_ops(&self) -> Result<Vec<ProofOp>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT op_id, file_id, parent_id, kind FROM operation_queue WHERE paused_reason = ?1
             ORDER BY created_at ASC, rowid ASC",
        )?;
        let rows = stmt.query_map(
            params![OperationPauseReason::FoundWhilePaused.as_str()],
            proof_op_from_row,
        )?;
        rows.collect()
    }
    /// Task 2267 (fix round 2), "Keep on this PC only": the rows whose upload has begun, so the server may already
    /// hold part of them: a queued upload with a resumable session, or an upload that landed and awaits its
    /// finalization (its operation is gone then; the finalization names the row).
    #[cfg(any(target_os = "windows", test, feature = "test-support"))]
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
    #[cfg(any(target_os = "windows", test, feature = "test-support"))]
    pub fn keep_journal(&self) -> Result<Option<String>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.query_row(
            "SELECT value FROM sync_state WHERE key = ?1",
            params![KEEP_JOURNAL_KEY],
            |row| row.get(0),
        )
        .optional()
    }
    #[cfg(any(target_os = "windows", test, feature = "test-support"))]
    pub fn set_keep_journal(&self, journal: &str) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute(
            "INSERT INTO sync_state (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![KEEP_JOURNAL_KEY, journal],
        )?;
        Ok(())
    }
    #[cfg(any(target_os = "windows", test, feature = "test-support"))]
    pub fn clear_keep_journal(&self) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute("DELETE FROM sync_state WHERE key = ?1", params![KEEP_JOURNAL_KEY])?;
        Ok(())
    }
    /// Task 2267 (self-review of round 3, F3): the staged copies the queued operations acting on the item `ids` hold,
    /// read before the keep forgets them, so an only copy is kept first.
    #[cfg(any(target_os = "windows", test, feature = "test-support"))]
    pub fn keep_staged_copies(&self, ids: &[String]) -> Result<Vec<String>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        Ok(keep_ops_conn(&conn, ids)?.1)
    }
    /// Task 2267 (fix round 2), "Keep on this PC only", after the item `ids` was moved to `kept_dir`: in ONE
    /// transaction, forget every queued operation acting on those rows (with its upload records) and the rows, remove
    /// the journal and remember `kept_dir` for "Show in Explorer". Returns the staged copies the removed operations
    /// held (as [`Self::keep_staged_copies`]).
    #[cfg(any(target_os = "windows", test, feature = "test-support"))]
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
    #[cfg(any(target_os = "windows", test, feature = "test-support"))]
    pub fn last_kept_dir(&self) -> Result<Option<String>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.query_row(
            "SELECT value FROM sync_state WHERE key = ?1",
            params![KEPT_DIR_KEY],
            |row| row.get(0),
        )
        .optional()
    }
    /// Task 2267 (lead review round 3, P2-2): every change held as `found_while_paused`, by kind, for the card,
    /// oldest first. A change is in a shared folder when its row, or any folder up the parent links from its row or
    /// from the folder it acts in, is not a My Files row (self-review of round 3, F8: not only the direct parent).
    #[cfg(any(target_os = "windows", test, feature = "test-support"))]
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
    #[cfg(any(target_os = "windows", test, feature = "test-support"))]
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
    #[cfg(any(target_os = "windows", test, feature = "test-support"))]
    pub fn staged_payload_source(&self, path: &str) -> Result<Option<Option<String>>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.query_row(
            "SELECT source_path FROM staged_payloads WHERE path = ?1",
            params![path],
            |row| row.get(0),
        )
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
    /// The sign-out's clear (macOS and Linux; Windows has `finish_windows_signout`) and the last step of the R10
    /// reset, after `purge_all_local_state` has handed back the files to delete: every row of the previous account,
    /// the sync cursor and the owner record included, in one transaction.
    ///
    /// L7d: the custody memory survives, because it belongs to the device and an account switch must not
    /// shrink it. B05e's share rows survive too (lead ruling 2026-11-09): the person's compared marks and the keys
    /// their shares were sealed to, public keys and fingerprints only, keyed by the own account id, so signing in
    /// again keeps the safety-number pin, and another account on this PC never reads them. A reset of local data
    /// removes them first ([`Self::forget_share_trust_rows`]). Every other `sync_state` row is cleared.
    ///
    /// `owe_finder_removal` is written in the SAME transaction: a Mac whose previous account's Finder domain is
    /// still registered owes a removal, and no engine may start until it is confirmed (`finder_removal_owed`).
    /// `false` also clears a debt that was already recorded, because the caller only passes it once a removal
    /// was confirmed or none is needed.
    pub fn clear_account_data(&self, owe_finder_removal: impl Into<SignOutMarker>) -> Result<()> {
        let owe_finder_removal: SignOutMarker = owe_finder_removal.into();
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction()?;
        for table in ACCOUNT_TABLES.iter().chain(DEVICE_TABLES.iter()) {
            tx.execute(&format!("DELETE FROM {table}"), [])?;
        }
        tx.execute(&sign_out_sync_state_purge(), [])?;
        let mark = if owe_finder_removal.as_bool() {
            FINDER_REMOVAL_OWED_KEY
        } else {
            FINDER_DOMAIN_GONE_KEY
        };
        tx.execute("INSERT INTO sync_state (key, value) VALUES (?1, '1')", params![mark])?;
        tx.commit()?;
        self.1.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        Ok(())
    }
    /// B05e: a reset of local data (R10's `reset`) removes the share marks, sealed, offered and accepted keys that a
    /// sign-out keeps (`share_marks::SHARE_TRUST_ROWS`), of every account on this PC, in one statement.
    pub fn forget_share_trust_rows(&self) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let any = vec!["key LIKE ?"; share_marks::SHARE_TRUST_ROWS.len()].join(" OR ");
        conn.execute(
            &format!("DELETE FROM sync_state WHERE {any}"),
            rusqlite::params_from_iter(share_marks::SHARE_TRUST_ROWS),
        )?;
        Ok(())
    }
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
    pub fn set_finder_removal_owed(&self, owed: impl Into<FinderRemovalOwed>) -> Result<()> {
        let owed: FinderRemovalOwed = owed.into();
        let conn = self.0.lock().expect("state_db mutex poisoned");
        if owed.as_bool() {
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
    #[cfg(any(test, feature = "test-support"))]
    pub fn finder_accepts_closed(&self) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        finder_accepts_closed_conn(&conn)
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
        // LD-0113: a backup op's session goes with it. The op held no payload path (the schema refuses one), so the
        // list above names no backup source: a sign-out never deletes the person's own file.
        tx.execute("DELETE FROM backup_upload_resume", [])?;
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
}
