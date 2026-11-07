//! LD-0113: the backup's local state — its queued uploads (`operation_queue` rows with `origin = 'backup'`), their
//! server sessions (`backup_upload_resume`) and what this device backed up (`backup_records`), per account (R10).
//!
//! The source file of a backup op is the person's own. It is read only through [`BackupUploadOp`], which no generic
//! queue reader returns, and the schema's triggers keep it out of every table desktop deletes paths from
//! (`schema.rs`, `migrate_backup_schema`).

use super::*;
use beebeeb_core::backup::{BackupRecord, RecordRef, RecordState};

/// `backup_records.account_owner` for an owner record: the user id when it is known, else the canonical email.
fn backup_owner_key(owner: &crate::account_binding::Identity) -> Option<String> {
    owner
        .user_id
        .as_deref()
        .map(|id| format!("user:{id}"))
        .or_else(|| {
            owner
                .email
                .as_deref()
                .map(|email| format!("email:{}", crate::account_binding::canonical_email(email)))
        })
}

fn owner_key_conn(conn: &Connection) -> Result<Option<String>> {
    Ok(read_owner(conn)?.as_ref().and_then(backup_owner_key))
}

fn record_state_str(state: RecordState) -> &'static str {
    match state {
        RecordState::BackedUp => "backed_up",
        RecordState::RemovedElsewhere => "removed_elsewhere",
    }
}

fn file_id_text(id: Option<u128>) -> Option<String> {
    id.map(|id| id.to_string())
}

const RESUME_COLUMNS: &str = "r.op_id, r.server_file_id, r.upload_session_id, r.object_version_id, \
    r.chunk_size_bytes, r.chunk_count, r.acked_chunks, r.chunk_format, r.source_size, r.source_modified_ms, \
    r.source_file_id";

fn resume_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<BackupUploadResume> {
    Ok(BackupUploadResume {
        op_id: row.get(0)?,
        server_file_id: row.get(1)?,
        upload_session_id: row.get(2)?,
        object_version_id: row.get(3)?,
        chunk_size_bytes: row.get(4)?,
        chunk_count: row.get(5)?,
        acked_chunks: row.get(6)?,
        chunk_format: row.get(7)?,
        fingerprint: beebeeb_backup::Fingerprint {
            size: u64::try_from(row.get::<_, i64>(8)?).unwrap_or(0),
            modified_ms: row.get(9)?,
            file_id: row.get::<_, Option<String>>(10)?.and_then(|id| id.parse().ok()),
        },
    })
}

impl StateDb {
    /// The current owner's `backup_records.account_owner`, if an owner is recorded.
    pub fn backup_owner_key(&self) -> Result<Option<String>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        owner_key_conn(&conn)
    }

    /// Queue one backup upload for the current owner: an `upload_file` op with `origin = 'backup'`, the source in
    /// `source_path` and no payload. Idempotent per (owner, category, key) (`BackupHost::enqueue_upload`): a key
    /// already queued is not queued twice, and a key whose `BackedUp` record already has the queued size and
    /// `modified_ms` (an upload that completed after the pass read the records) is not queued at all.
    /// `backup_source_key` is the category, so turning the folder off takes it (`take_backup_category_ops`).
    // LD-0113 Task D4: `DesktopBackupHost` (`BackupHost::{enqueue_upload, records}`) is its production caller.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn enqueue_backup_upload(&self, up: &NewBackupUpload<'_>) -> Result<BackupEnqueue> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let Some(owner) = owner_key_conn(&tx)? else {
            return Ok(BackupEnqueue::NoOwner);
        };
        // Review (2026-11-07): an op that used up its retries has parked, and nothing retries a parked op. A backup's
        // retry is the next pass, so the parked op goes (its session, if any, is reaped by the server's lease
        // sweep) and the file is queued afresh; it never holds the key for good.
        let parked = tx.execute(
            "DELETE FROM operation_queue
             WHERE origin = 'backup' AND backup_source_key = ?1 AND attempts >= max_attempts AND claim_id IS NULL
               AND json_extract(metadata_json, '$.backup.key') = ?2
               AND json_extract(metadata_json, '$.backup.account_owner') = ?3",
            params![up.category, up.key, owner],
        )?;
        if parked > 0 {
            super::reconcile::drop_orphaned_upload_resumes(&tx)?;
        }
        let queued: Option<String> = tx
            .query_row(
                "SELECT op_id FROM operation_queue
                 WHERE origin = 'backup' AND backup_source_key = ?1
                   AND json_extract(metadata_json, '$.backup.key') = ?2
                   AND json_extract(metadata_json, '$.backup.account_owner') = ?3
                 LIMIT 1",
                params![up.category, up.key, owner],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(op_id) = queued {
            return Ok(BackupEnqueue::AlreadyQueued(op_id));
        }
        let backed_up: bool = tx.query_row(
            "SELECT EXISTS (SELECT 1 FROM backup_records
                            WHERE account_owner = ?1 AND category = ?2 AND key = ?3 AND state = 'backed_up'
                              AND size = ?4 AND modified_ms = ?5)",
            params![owner, up.category, up.key, i64::try_from(up.queued.size).unwrap_or(i64::MAX), up.queued.modified_ms],
            |row| row.get(0),
        )?;
        if backed_up {
            return Ok(BackupEnqueue::AlreadyBackedUp);
        }
        let metadata = serde_json::json!({
            "operation": if up.replace { "upload_version" } else { "create_file" },
            "name_encrypted": up.name_encrypted,
            "content_type": up.content_type,
            "backup": {
                "category": up.category,
                "key": up.key,
                "account_owner": owner,
                "queued_size": up.queued.size,
                "queued_modified_ms": up.queued.modified_ms,
                "queued_file_id": file_id_text(up.queued.file_id),
                "replace": up.replace,
                "record_key": up.record_key,
            },
        });
        tx.execute(
            "INSERT INTO operation_queue (
                op_id, kind, file_id, parent_id, target_path, metadata_json, payload_path, origin, source_path,
                attempts, max_attempts, next_retry_at, paused_reason, backup_source_key, created_at, updated_at
             ) VALUES (?1, 'upload_file', ?2, ?3, NULL, ?4, NULL, 'backup', ?5, 0, 5, 0,
                CASE WHEN EXISTS (SELECT 1 FROM operation_queue held
                                  WHERE held.paused_reason = ?8 AND held.file_id IN (?2, ?3))
                     THEN ?8 END,
                ?6, ?7, ?7)",
            params![
                up.op_id,
                up.file_id,
                up.parent_id,
                metadata.to_string(),
                up.source_path.to_string_lossy(),
                up.category,
                up.now,
                OperationPauseReason::FoundWhilePaused.as_str(),
            ],
        )?;
        tx.commit()?;
        Ok(BackupEnqueue::Queued(up.op_id.to_string()))
    }

    /// The backup op `op_id` with its source, or `None` when it is gone or is not a backup op.
    pub fn backup_upload_op(&self, op_id: &str) -> Result<Option<BackupUploadOp>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.query_row(
            "SELECT op_id, file_id, parent_id, metadata_json, source_path FROM operation_queue
             WHERE op_id = ?1 AND origin = 'backup'",
            params![op_id],
            |row| {
                let metadata: serde_json::Value = row
                    .get::<_, Option<String>>(3)?
                    .and_then(|raw| serde_json::from_str(&raw).ok())
                    .unwrap_or_default();
                let text = |pointer: &str| metadata.pointer(pointer).and_then(|v| v.as_str()).map(str::to_string);
                Ok(BackupUploadOp {
                    op_id: row.get(0)?,
                    category: text("/backup/category").unwrap_or_default(),
                    key: text("/backup/key").unwrap_or_default(),
                    source_path: std::path::PathBuf::from(row.get::<_, String>(4)?),
                    file_id: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    replace: metadata.pointer("/backup/replace").and_then(|v| v.as_bool()).unwrap_or(false),
                    record_key: text("/backup/record_key"),
                    parent_id: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                    name_encrypted: text("/name_encrypted").unwrap_or_default(),
                    content_type: text("/content_type"),
                    account_owner: text("/backup/account_owner").unwrap_or_default(),
                })
            },
        )
        .optional()
    }

    pub fn backup_upload_resume(&self, op_id: &str) -> Result<Option<BackupUploadResume>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.query_row(
            &format!("SELECT {RESUME_COLUMNS} FROM backup_upload_resume r WHERE r.op_id = ?1"),
            params![op_id],
            resume_from_row,
        )
        .optional()
    }

    /// Persist the session `init` opened, while the claimed op exists (S4). `false`: the op moved since the claim
    /// and nothing was written.
    pub fn put_backup_upload_resume_claimed(&self, resume: &BackupUploadResume, claim_id: &str) -> Result<bool> {
        let claim = ClaimId::parse(claim_id).map_err(StateDbError::into_sql)?;
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let n = conn.execute(
            "INSERT INTO backup_upload_resume (
                op_id, server_file_id, upload_session_id, object_version_id, chunk_size_bytes, chunk_count,
                acked_chunks, chunk_format, source_size, source_modified_ms, source_file_id, updated_at
             )
             SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, strftime('%s','now')
             WHERE EXISTS (SELECT 1 FROM operation_queue WHERE op_id = ?1 AND claim_id = ?12 AND origin = 'backup')
             ON CONFLICT(op_id) DO UPDATE SET
                server_file_id = excluded.server_file_id,
                upload_session_id = excluded.upload_session_id,
                object_version_id = excluded.object_version_id,
                chunk_size_bytes = excluded.chunk_size_bytes,
                chunk_count = excluded.chunk_count,
                acked_chunks = excluded.acked_chunks,
                chunk_format = excluded.chunk_format,
                source_size = excluded.source_size,
                source_modified_ms = excluded.source_modified_ms,
                source_file_id = excluded.source_file_id,
                updated_at = excluded.updated_at",
            params![
                resume.op_id,
                resume.server_file_id,
                resume.upload_session_id,
                resume.object_version_id,
                resume.chunk_size_bytes,
                resume.chunk_count,
                resume.acked_chunks,
                resume.chunk_format,
                i64::try_from(resume.fingerprint.size).unwrap_or(i64::MAX),
                resume.fingerprint.modified_ms,
                file_id_text(resume.fingerprint.file_id),
                claim.as_str(),
            ],
        )?;
        Ok(n == 1)
    }

    /// Chunks `0..acked` of the session were acknowledged.
    pub fn set_backup_upload_resume_acked(&self, op_id: &str, acked: i64) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute(
            "UPDATE backup_upload_resume SET acked_chunks = ?2, updated_at = strftime('%s','now') WHERE op_id = ?1",
            params![op_id, acked],
        )?;
        Ok(())
    }

    pub fn clear_backup_upload_resume(&self, op_id: &str) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute("DELETE FROM backup_upload_resume WHERE op_id = ?1", params![op_id])?;
        Ok(())
    }

    /// The upload of the claimed backup op completed: in one transaction the op and its session go and the record is
    /// written, for `account_owner` only while that is still this database's owner (R10). The record is upserted by
    /// key and any other record of the category naming the same remote file goes, so one remote file never has two
    /// keys (`BackupHost::put_record`'s rule, spec Amendment 5 item 4). `false`: nothing was written, because the op
    /// moved since the claim or the owner is no longer `account_owner`.
    pub fn complete_backup_upload_claimed(
        &self,
        op_id: &str,
        claim_id: &str,
        account_owner: &str,
        category: &str,
        record: &BackupRecord,
    ) -> Result<bool> {
        let claim = ClaimId::parse(claim_id).map_err(StateDbError::into_sql)?;
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if owner_key_conn(&tx)?.as_deref() != Some(account_owner) {
            return Ok(false);
        }
        let n = tx.execute(
            "DELETE FROM operation_queue WHERE op_id = ?1 AND claim_id = ?2 AND origin = 'backup'",
            params![op_id, claim.as_str()],
        )?;
        if n != 1 {
            return Ok(false);
        }
        tx.execute("DELETE FROM backup_upload_resume WHERE op_id = ?1", params![op_id])?;
        tx.execute(
            "DELETE FROM backup_records WHERE account_owner = ?1 AND category = ?2 AND remote_id = ?3 AND key != ?4",
            params![account_owner, category, record.remote_id.to_string(), record.key],
        )?;
        tx.execute(
            "INSERT INTO backup_records (account_owner, category, key, size, modified_ms, remote_id, uploaded_at_ms, state)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(account_owner, category, key) DO UPDATE SET
                size = excluded.size,
                modified_ms = excluded.modified_ms,
                remote_id = excluded.remote_id,
                uploaded_at_ms = excluded.uploaded_at_ms,
                state = excluded.state",
            params![
                account_owner,
                category,
                record.key,
                i64::try_from(record.size).unwrap_or(i64::MAX),
                record.modified_ms,
                record.remote_id.to_string(),
                record.uploaded_at_ms,
                record_state_str(record.state),
            ],
        )?;
        tx.commit()?;
        Ok(true)
    }

    /// Turning the backup of `category` off (`BackupHost::turn_off`, spec §4.2 rule 11): its queued ops and their
    /// sessions go, in one transaction, and the sessions are returned so the caller abandons each on the server. The
    /// records stay: "Back up again" pairs what is still there (rule 10) instead of uploading it twice. An upload in
    /// flight loses its op, so its claim-guarded writes and its completion write nothing.
    pub fn take_backup_category_ops(&self, category: &str) -> Result<(usize, Vec<BackupUploadResume>)> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        // Read and delete in one transaction, so no session opened in between is lost unabandoned.
        let taken = {
            let mut stmt = tx.prepare(&format!(
                "SELECT {RESUME_COLUMNS} FROM backup_upload_resume r JOIN operation_queue q ON q.op_id = r.op_id
                 WHERE q.origin = 'backup' AND q.backup_source_key = ?1 ORDER BY r.op_id"
            ))?;
            let rows = stmt.query_map(params![category], resume_from_row)?;
            rows.collect::<Result<Vec<_>>>()?
        };
        let deleted = tx.execute(
            "DELETE FROM operation_queue WHERE origin = 'backup' AND backup_source_key = ?1",
            params![category],
        )?;
        drop_orphaned_upload_resumes(&tx)?;
        tx.commit()?;
        Ok((deleted, taken))
    }

    /// Backup sessions whose op is gone (the folder was switched off in Settings, a bulk purge): the runner abandons
    /// each on the server, which clears its row.
    pub fn orphaned_backup_sessions(&self) -> Result<Vec<BackupUploadResume>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare(&format!(
            "SELECT {RESUME_COLUMNS} FROM backup_upload_resume r
             WHERE r.op_id NOT IN (SELECT op_id FROM operation_queue) ORDER BY r.op_id"
        ))?;
        let rows = stmt.query_map([], resume_from_row)?;
        rows.collect()
    }

    /// Drop the claimed backup op and its session with no record (its source changed, vanished or cannot be read
    /// now, or it was queued for another account): the next pass plans the file again. `false`: the op moved since
    /// the claim and nothing was written.
    pub fn drop_backup_op_claimed(&self, op_id: &str, claim_id: &str) -> Result<bool> {
        let claim = ClaimId::parse(claim_id).map_err(StateDbError::into_sql)?;
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let n = tx.execute(
            "DELETE FROM operation_queue WHERE op_id = ?1 AND claim_id = ?2 AND origin = 'backup'",
            params![op_id, claim.as_str()],
        )?;
        if n == 1 {
            tx.execute("DELETE FROM backup_upload_resume WHERE op_id = ?1", params![op_id])?;
        }
        tx.commit()?;
        Ok(n == 1)
    }

    /// The current owner's records for `category` (`BackupHost::records`). Another account's records are never
    /// returned, whatever is still on disk.
    // LD-0113 Task D4: `DesktopBackupHost` (`BackupHost::{enqueue_upload, records}`) is its production caller.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn backup_records(&self, category: &str) -> Result<Vec<BackupRecord>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let Some(owner) = owner_key_conn(&conn)? else {
            return Ok(Vec::new());
        };
        let mut stmt = conn.prepare(
            "SELECT key, size, modified_ms, remote_id, uploaded_at_ms, state FROM backup_records
             WHERE account_owner = ?1 AND category = ?2 ORDER BY key",
        )?;
        let rows = stmt.query_map(params![owner, category], |row| {
            let remote_id: String = row.get(3)?;
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                remote_id,
                row.get::<_, i64>(4)?,
                row.get::<_, String>(5)?,
            ))
        })?;
        let mut records = Vec::new();
        for row in rows {
            let (key, size, modified_ms, remote_id, uploaded_at_ms, state) = row?;
            // A row whose remote id does not parse names no server file; it is not a record of anything.
            let Ok(remote_id) = uuid::Uuid::parse_str(&remote_id) else {
                tracing::warn!(category, "backup: a record names no server file id; it is not used");
                continue;
            };
            records.push(BackupRecord {
                key,
                size: u64::try_from(size).unwrap_or(0),
                modified_ms,
                remote_id,
                uploaded_at_ms,
                state: match state.as_str() {
                    "removed_elsewhere" => RecordState::RemovedElsewhere,
                    _ => RecordState::BackedUp,
                },
            });
        }
        Ok(records)
    }

    /// The current owner's backup layout, if one was resolved.
    pub fn backup_layout(&self) -> Result<Option<BackupLayout>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let Some(owner) = owner_key_conn(&conn)? else {
            return Ok(None);
        };
        conn.query_row(
            "SELECT root_id, device_folder_id, device_folder_name FROM backup_layout WHERE account_owner = ?1",
            params![owner],
            |row| {
                Ok(BackupLayout {
                    root_id: row.get(0)?,
                    device_folder_id: row.get(1)?,
                    device_folder_name: row.get(2)?,
                })
            },
        )
        .optional()
    }

    /// Cache the resolved layout for the current owner. `false`: no owner is recorded, nothing was written.
    pub fn set_backup_layout(&self, layout: &BackupLayout) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let Some(owner) = owner_key_conn(&conn)? else {
            return Ok(false);
        };
        conn.execute(
            "INSERT INTO backup_layout (account_owner, root_id, device_folder_id, device_folder_name, updated_at)
             VALUES (?1, ?2, ?3, ?4, strftime('%s','now'))
             ON CONFLICT(account_owner) DO UPDATE SET root_id = excluded.root_id,
                device_folder_id = excluded.device_folder_id, device_folder_name = excluded.device_folder_name,
                updated_at = excluded.updated_at",
            params![owner, layout.root_id, layout.device_folder_id, layout.device_folder_name],
        )?;
        Ok(true)
    }

    /// Forget the layout (its device folder was deleted elsewhere): "Back up again" resolves it anew.
    pub fn clear_backup_layout(&self) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let Some(owner) = owner_key_conn(&conn)? else {
            return Ok(());
        };
        conn.execute("DELETE FROM backup_layout WHERE account_owner = ?1", params![owner])?;
        Ok(())
    }

    /// The folder id of `category` under this device's folder, once created or adopted.
    pub fn backup_category_folder(&self, category: &str) -> Result<Option<String>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let Some(owner) = owner_key_conn(&conn)? else {
            return Ok(None);
        };
        conn.query_row(
            "SELECT folder_id FROM backup_category_folders WHERE account_owner = ?1 AND category = ?2",
            params![owner, category],
            |row| row.get(0),
        )
        .optional()
    }

    /// Remember (`Some`) or forget (`None`, a turned-off backup) the folder of `category`.
    pub fn set_backup_category_folder(&self, category: &str, folder_id: Option<&str>) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let Some(owner) = owner_key_conn(&conn)? else {
            return Ok(());
        };
        match folder_id {
            Some(id) => conn.execute(
                "INSERT INTO backup_category_folders (account_owner, category, folder_id) VALUES (?1, ?2, ?3)
                 ON CONFLICT(account_owner, category) DO UPDATE SET folder_id = excluded.folder_id",
                params![owner, category, id],
            )?,
            None => conn.execute(
                "DELETE FROM backup_category_folders WHERE account_owner = ?1 AND category = ?2",
                params![owner, category],
            )?,
        };
        Ok(())
    }

    /// `BackupHost::put_record`, compare-and-set on the record the pass read (`expected`):
    /// - the record at `expected.key` still names `expected.remote_id`: only its key and state change, its size,
    ///   `modified_ms` and `uploaded_at_ms` stay as stored (a finishing upload may have written newer ones);
    /// - no record names `expected.remote_id`: `record` is stored;
    /// - otherwise nothing changes.
    ///
    /// Storing upserts by key and removes any other record of the category naming the same remote file. `true`
    /// when it stored.
    pub fn put_backup_record_cas(&self, category: &str, expected: &RecordRef, record: &BackupRecord) -> Result<bool> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let Some(owner) = owner_key_conn(&tx)? else {
            return Ok(false);
        };
        let expected_remote = expected.remote_id.to_string();
        let at_expected: Option<(i64, i64, i64)> = tx
            .query_row(
                "SELECT size, modified_ms, uploaded_at_ms FROM backup_records
                 WHERE account_owner = ?1 AND category = ?2 AND key = ?3 AND remote_id = ?4",
                params![owner, category, expected.key, expected_remote],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let (size, modified_ms, uploaded_at_ms) = match at_expected {
            Some(stored) => stored,
            None => {
                let named: bool = tx.query_row(
                    "SELECT EXISTS (SELECT 1 FROM backup_records
                                    WHERE account_owner = ?1 AND category = ?2 AND remote_id = ?3)",
                    params![owner, category, expected_remote],
                    |row| row.get(0),
                )?;
                if named {
                    return Ok(false);
                }
                (i64::try_from(record.size).unwrap_or(i64::MAX), record.modified_ms, record.uploaded_at_ms)
            }
        };
        tx.execute(
            "DELETE FROM backup_records WHERE account_owner = ?1 AND category = ?2 AND remote_id = ?3 AND key != ?4",
            params![owner, category, record.remote_id.to_string(), record.key],
        )?;
        if record.remote_id != expected.remote_id {
            tx.execute(
                "DELETE FROM backup_records WHERE account_owner = ?1 AND category = ?2 AND remote_id = ?3",
                params![owner, category, expected_remote],
            )?;
        }
        tx.execute(
            "INSERT INTO backup_records (account_owner, category, key, size, modified_ms, remote_id, uploaded_at_ms, state)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'backed_up')
             ON CONFLICT(account_owner, category, key) DO UPDATE SET size = excluded.size,
                modified_ms = excluded.modified_ms, remote_id = excluded.remote_id,
                uploaded_at_ms = excluded.uploaded_at_ms, state = excluded.state",
            params![owner, category, record.key, size, modified_ms, record.remote_id.to_string(), uploaded_at_ms],
        )?;
        tx.commit()?;
        Ok(true)
    }

    /// `BackupHost::mark_removed_elsewhere`: each record still at its key, naming its remote id and `BackedUp`, becomes
    /// `RemovedElsewhere`. A record a finishing upload rewrote meanwhile is left alone.
    pub fn mark_backup_records_removed_elsewhere(&self, category: &str, records: &[RecordRef]) -> Result<usize> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction()?;
        let Some(owner) = owner_key_conn(&tx)? else {
            return Ok(0);
        };
        let mut marked = 0;
        for record in records {
            marked += tx.execute(
                "UPDATE backup_records SET state = 'removed_elsewhere'
                 WHERE account_owner = ?1 AND category = ?2 AND key = ?3 AND remote_id = ?4 AND state = 'backed_up'",
                params![owner, category, record.key, record.remote_id.to_string()],
            )?;
        }
        tx.commit()?;
        Ok(marked)
    }

    /// `BackupHost::held_keys`: the keys of `category` with a live backup op in the queue for the current owner
    /// (queued or in flight; one parked after its retries holds nothing, and the next enqueue replaces it). The pass
    /// skips them; the op itself reads the file again when it runs, so a held key never uploads stale bytes.
    pub fn backup_held_keys(&self, category: &str) -> Result<Vec<String>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let Some(owner) = owner_key_conn(&conn)? else {
            return Ok(Vec::new());
        };
        let mut stmt = conn.prepare(
            "SELECT DISTINCT json_extract(metadata_json, '$.backup.key') FROM operation_queue
             WHERE origin = 'backup' AND backup_source_key = ?1 AND attempts < max_attempts
               AND json_extract(metadata_json, '$.backup.account_owner') = ?2
             ORDER BY 1",
        )?;
        let keys = stmt.query_map(params![category, owner], |row| row.get::<_, Option<String>>(0))?;
        Ok(keys.filter_map(|key| key.transpose()).collect::<Result<Vec<_>>>()?)
    }

    /// Record that the backup of `category` was turned off, and why.
    pub fn record_backup_stop(&self, stop: &BackupStop) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let Some(owner) = owner_key_conn(&conn)? else {
            return Ok(());
        };
        conn.execute(
            "INSERT INTO backup_stops (account_owner, category, reason, at_ms) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(account_owner, category) DO UPDATE SET reason = excluded.reason, at_ms = excluded.at_ms",
            params![owner, stop.category, stop.reason, stop.at_ms],
        )?;
        Ok(())
    }

    /// The current owner's turned-off backups, by category.
    pub fn backup_stops(&self) -> Result<Vec<BackupStop>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let Some(owner) = owner_key_conn(&conn)? else {
            return Ok(Vec::new());
        };
        let mut stmt =
            conn.prepare("SELECT category, reason, at_ms FROM backup_stops WHERE account_owner = ?1 ORDER BY category")?;
        let rows = stmt.query_map(params![owner], |row| {
            Ok(BackupStop {
                category: row.get(0)?,
                reason: row.get(1)?,
                at_ms: row.get(2)?,
            })
        })?;
        rows.collect()
    }

    /// "Back up again": the stop of `category` is cleared.
    pub fn clear_backup_stop(&self, category: &str) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let Some(owner) = owner_key_conn(&conn)? else {
            return Ok(());
        };
        conn.execute(
            "DELETE FROM backup_stops WHERE account_owner = ?1 AND category = ?2",
            params![owner, category],
        )?;
        Ok(())
    }

    /// LD-0139: the marks of `category`'s backup (all `None` when it has none).
    pub fn backup_pass_mark(&self, category: &str) -> Result<BackupPassMark> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let Some(owner) = owner_key_conn(&conn)? else {
            return Ok(BackupPassMark::default());
        };
        let mark = conn
            .query_row(
                "SELECT last_pass_at_ms, setup_at_ms FROM backup_pass_marks WHERE account_owner = ?1 AND category = ?2",
                params![owner, category],
                |row| {
                    Ok(BackupPassMark {
                        last_pass_at_ms: row.get(0)?,
                        setup_at_ms: row.get(1)?,
                    })
                },
            )
            .optional()?;
        Ok(mark.unwrap_or_default())
    }

    /// LD-0139: `category`'s pass checked the notices up to `at_ms` (the check's `Date`, server clock).
    pub fn set_backup_last_pass(&self, category: &str, at_ms: i64) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let Some(owner) = owner_key_conn(&conn)? else {
            return Ok(());
        };
        conn.execute(
            "INSERT INTO backup_pass_marks (account_owner, category, last_pass_at_ms) VALUES (?1, ?2, ?3)
             ON CONFLICT(account_owner, category) DO UPDATE SET last_pass_at_ms = excluded.last_pass_at_ms",
            params![owner, category, at_ms],
        )?;
        Ok(())
    }

    /// LD-0139: the person turned `category`'s backup on, or chose "Back up again", at `setup_at_ms` (this device's
    /// clock): it starts over, with no pass yet, so its first pass takes its baseline from this moment. With
    /// `forget_records` ("Back up again" after a stop) its records go too, in the same transaction, so its first pass
    /// rebuilds them from what the server holds (spec rule 10) instead of taking removed files as deleted elsewhere.
    pub fn restart_backup_pass(&self, category: &str, setup_at_ms: i64, forget_records: bool) -> Result<()> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let Some(owner) = owner_key_conn(&conn)? else {
            return Ok(());
        };
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO backup_pass_marks (account_owner, category, last_pass_at_ms, setup_at_ms) VALUES (?1, ?2, NULL, ?3)
             ON CONFLICT(account_owner, category) DO UPDATE SET last_pass_at_ms = NULL, setup_at_ms = excluded.setup_at_ms",
            params![owner, category, setup_at_ms],
        )?;
        if forget_records {
            tx.execute(
                "DELETE FROM backup_records WHERE account_owner = ?1 AND category = ?2",
                params![owner, category],
            )?;
        }
        tx.commit()
    }

    /// The current owner's flagged Backups root, once resolved.
    pub fn backup_root(&self) -> Result<Option<String>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let Some(owner) = owner_key_conn(&conn)? else {
            return Ok(None);
        };
        conn.query_row("SELECT root_id FROM backup_root WHERE account_owner = ?1", params![owner], |row| row.get(0))
            .optional()
    }

    /// Cache the current owner's flagged root. `false`: no owner is recorded, nothing was written.
    pub fn set_backup_root(&self, root_id: &str) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let Some(owner) = owner_key_conn(&conn)? else {
            return Ok(false);
        };
        conn.execute(
            "INSERT INTO backup_root (account_owner, root_id) VALUES (?1, ?2)
             ON CONFLICT(account_owner) DO UPDATE SET root_id = excluded.root_id",
            params![owner, root_id],
        )?;
        Ok(true)
    }

    /// Amendment 9: ask about the moved folder `category` (its default and its moved location) once. `true` only
    /// when the question is new: not asked before, or asked about other locations.
    pub fn ask_backup_source_question(&self, category: &str, default_path: &str, moved_path: &str, at_ms: i64) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let changed = conn.execute(
            "INSERT INTO backup_source_questions (category, default_path, moved_path, asked_at_ms) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(category) DO UPDATE SET default_path = excluded.default_path,
                moved_path = excluded.moved_path, asked_at_ms = excluded.asked_at_ms
             WHERE default_path != excluded.default_path OR moved_path != excluded.moved_path",
            params![category, default_path, moved_path, at_ms],
        )?;
        Ok(changed > 0)
    }

    /// The open questions, by category, with the two locations each names.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn backup_source_questions(&self) -> Result<Vec<(String, String, String)>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn
            .prepare("SELECT category, default_path, moved_path FROM backup_source_questions ORDER BY category")?;
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
        rows.collect()
    }

    /// The question about `category` is settled (answered, the folder is no longer moved, or its backup is off).
    pub fn settle_backup_source_question(&self, category: &str) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute("DELETE FROM backup_source_questions WHERE category = ?1", params![category])?;
        Ok(())
    }
}
