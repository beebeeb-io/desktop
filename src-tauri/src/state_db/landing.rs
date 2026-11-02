//! Finder writes, accept, the landing transaction and held edits (split out of `state_db.rs`).

use super::*;

impl StateDb {
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
}
