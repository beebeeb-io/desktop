//! The operation queue: enqueue, claim, attempts, pauses, upload resume and aliases (split out of `state_db.rs`).

use super::*;

impl StateDb {
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
    #[cfg(target_os = "windows")]
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
        if matches!(
            op.kind,
            OperationKind::RenameFile | OperationKind::MoveFile | OperationKind::TrashFile
        ) {
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
        // LD-0113: a backup op is never a Finder write, so it never waits on, parks for or takes over a predecessor;
        // the hand-over below (which releases the predecessor's payload for unlinking) never runs for it.
        let origin: OpOrigin = tx.query_row(
            "SELECT origin FROM operation_queue WHERE op_id = ?1",
            params![op_id],
            |row| row.get::<_, String>(0).map(|value| OpOrigin::from_db(&value)),
        )?;
        let mut write = match origin {
            OpOrigin::Backup => None,
            OpOrigin::Sync => finder_write_conn(&tx, op_id)?,
        };
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
            params![op_id, ClaimId::new(&claim_id).as_str(), now],
        )?;
        tx.commit()?;
        Ok(ClaimOutcome::Claimed(Box::new(ClaimedOp {
            op,
            origin,
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
        let claim = ClaimId::parse(claim_id).map_err(StateDbError::into_sql)?;
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let n = tx.execute(
            "UPDATE operation_queue
             SET attempts = ?3, next_retry_at = ?4, last_error = ?5, last_error_class = NULL,
                 paused_reason = NULL, updated_at = ?4, claim_id = NULL, claimed_at = NULL
             WHERE op_id = ?1 AND claim_id = ?2",
            params![op_id, claim.as_str(), attempts, next_retry_at, last_error],
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
        let claim = ClaimId::parse(claim_id).map_err(StateDbError::into_sql)?;
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let n = conn.execute(
            "UPDATE operation_queue
             SET paused_reason = ?3, last_error_class = ?3, last_error = ?4, updated_at = ?5,
                 claim_id = NULL, claimed_at = NULL
             WHERE op_id = ?1 AND claim_id = ?2",
            params![
                op_id,
                claim.as_str(),
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
        let claim = ClaimId::parse(claim_id).map_err(StateDbError::into_sql)?;
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
            params![op_id, claim.as_str(), reason.as_str(), now],
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
        let claim = ClaimId::parse(claim_id).map_err(StateDbError::into_sql)?;
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
                claim.as_str(),
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
        let claim = ClaimId::parse(claim_id).map_err(StateDbError::into_sql)?;
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let n = tx.execute(
            "DELETE FROM operation_queue WHERE op_id = ?1 AND claim_id = ?2",
            params![op_id, claim.as_str()],
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
}
