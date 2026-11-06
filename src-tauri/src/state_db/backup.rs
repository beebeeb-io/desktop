//! LD-0113: the backup's local state — its queued uploads (`operation_queue` rows with `origin = 'backup'`), their
//! server sessions (`backup_upload_resume`) and what this device backed up (`backup_records`), per account (R10).
//!
//! The source file of a backup op is the person's own. It is read only through [`BackupUploadOp`], which no generic
//! queue reader returns, and the schema's triggers keep it out of every table desktop deletes paths from
//! (`schema.rs`, `migrate_backup_schema`).

use super::*;
use beebeeb_core::backup::{BackupRecord, RecordState};

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
}
