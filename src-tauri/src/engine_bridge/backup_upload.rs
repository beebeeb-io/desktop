//! LD-0113 (spec 2026-11-06 §6.1 point 4, §4.3): the backup uploader. A backup op uploads a file from the person's
//! own folder where it is; nothing is staged, copied, moved or written.
//!
//! - The source is read through [`BackupUploadOp`] only, opened read-only with full sharing and without following a
//!   link, after a check that its bytes are on this device (reading a cloud-only file would download it).
//! - The fingerprint (size, mtime, inode) is taken before the first chunk and again after the last. Any difference
//!   abandons the session (`POST …/upload/abandon`) before `complete`, so a torn copy never lands, and the op is
//!   dropped: the next pass finds no record and queues the file again.
//! - On completion the op, its session and the `backup_records` row change in one transaction.
//! - It never runs the sync-root landing: no `files` row, no staged-copy cleanup, no finalization journal.
//!
//! Logs carry ids, counts and categories, never a name or a path.

use super::*;
use crate::state_db::{BackupEnqueue, BackupUploadOp, BackupUploadResume, NewBackupUpload};

/// Why a backup op was dropped without a record. The next pass plans the file again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BackupDrop {
    /// The fingerprint after the last chunk (or at a resume) differs from the one before the first.
    SourceChanged,
    /// The source is gone. It stays in the backup (spec §1: a local delete is not mirrored).
    SourceGone,
    /// The source cannot be read now: locked by another program, no permission, not a regular file.
    SourceUnreadable,
    /// Its bytes are not on this device (OneDrive Files On-Demand, iCloud dataless): never read, never hydrated.
    SourceNotLocal,
    /// Queued for an account that is not this database's owner (R10).
    OtherAccount,
    /// A persisted session that core's `resume_policy` does not let this writer continue, or that the server
    /// no longer has.
    SessionNotResumable,
}

impl BackupDrop {
    fn as_str(self) -> &'static str {
        match self {
            BackupDrop::SourceChanged => "source_changed",
            BackupDrop::SourceGone => "source_gone",
            BackupDrop::SourceUnreadable => "source_unreadable",
            BackupDrop::SourceNotLocal => "source_not_local",
            BackupDrop::OtherAccount => "other_account",
            BackupDrop::SessionNotResumable => "session_not_resumable",
        }
    }
}

/// The source's fingerprint, from its metadata only and without following a link, or why it is not read now.
fn source_fingerprint(path: &Path) -> Result<beebeeb_backup::Fingerprint, BackupDrop> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(BackupDrop::SourceGone),
        Err(_) => return Err(BackupDrop::SourceUnreadable),
    };
    let attrs = beebeeb_backup::attrs_of(&meta);
    if attrs.not_local {
        return Err(BackupDrop::SourceNotLocal);
    }
    if attrs.is_link || attrs.is_dir {
        return Err(BackupDrop::SourceUnreadable);
    }
    beebeeb_backup::fingerprint(path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => BackupDrop::SourceGone,
        _ => BackupDrop::SourceUnreadable,
    })
}

/// Open the person's file for reading only. Windows: every share mode, so other programs keep writing, renaming
/// and deleting it meanwhile (the fingerprint after the last chunk catches a change). Unix: a link is never followed.
pub(crate) fn open_source_read_only(path: &Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE.
        options.share_mode(0x1 | 0x2 | 0x4);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    options.open(path)
}

fn unix_millis_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| i64::try_from(since.as_millis()).unwrap_or(i64::MAX))
}

impl EngineBridge {
    /// `BackupHost::enqueue_upload` (spec §4.3): queue one file of a backed-up folder for this account. The id of
    /// the new server file is minted here and the name sealed for it; the source path goes into the queue's
    /// `source_path` and nowhere else. Idempotent per (category, key).
    // LD-0113 Task D4: `DesktopBackupHost` (`BackupHost::{enqueue_upload, records}`) is its production caller.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn enqueue_backup_upload(&self, req: &beebeeb_backup::UploadRequest) -> Result<BackupEnqueue, EngineError> {
        if self.is_stopping() {
            return Err(anyhow::anyhow!("engine is stopping; refusing to enqueue a backup upload").into());
        }
        let master = beebeeb_core::kdf::MasterKey::from_bytes(*self.api.master_key());
        let file_id = beebeeb_core::item_id::NewItemId::owner(&master)?.as_str().to_string();
        let mime = beebeeb_core::media::guess_mime_type(&req.name);
        let name_encrypted = owned_metadata_for_name(&self.db, self.api.master_key(), &file_id, &req.name, mime)?;
        let op_id = uuid::Uuid::new_v4().to_string();
        let category = req.category.key();
        let parent_id = req.parent_id.to_string();
        let source_path = req.source_path.as_path();
        Ok(self.db.enqueue_backup_upload(&NewBackupUpload {
            op_id: &op_id,
            category: &category,
            key: &req.key,
            source_path,
            file_id: &file_id,
            parent_id: &parent_id,
            name_encrypted: &name_encrypted,
            content_type: mime,
            queued: req.fingerprint,
            now: now_secs(),
        })?)
    }

    /// Run one claimed backup op (`origin = 'backup'`). `OpDone::Removed`: uploaded, recorded, op gone.
    /// `OpDone::Dropped`: no record and the op gone, so the next pass plans the file again. An error is the queue's
    /// usual retry; a persisted session is kept for it.
    pub(super) async fn upload_backup(
        &self,
        claimed: &ClaimedOp,
        post_complete_errors: &mut Vec<String>,
    ) -> Result<OpDone, EngineError> {
        let op_id = claimed.op.op_id.as_str();
        if claimed.op.kind != OperationKind::UploadFile {
            return Err(anyhow::anyhow!("a backup op is always an upload").into());
        }
        let Some(op) = self.db.backup_upload_op(op_id)? else {
            log_queue_state_moved(op_id, "backup");
            return Err(QueueStateMoved.into());
        };
        let previous = self.db.backup_upload_resume(op_id)?;
        // R10 (Review Focus 3): an op queued for another account is never sent, whatever is still in the queue.
        if self.db.backup_owner_key()?.as_deref() != Some(op.account_owner.as_str()) {
            return self.drop_backup(claimed, previous.as_ref(), BackupDrop::OtherAccount).await;
        }
        // Spec §4.3: the fingerprint before the first chunk, taken now (the pass's is only what it saw).
        let before = match source_fingerprint(&op.source_path) {
            Ok(fingerprint) => fingerprint,
            Err(reason) => return self.drop_backup(claimed, previous.as_ref(), reason).await,
        };
        let (file_key, pin) = owned_key_and_pin(&self.db, self.api.master_key(), &op.file_id)?;
        let writing = beebeeb_core::chunk_stream::chunk_format_for_pin(pin);

        // A resume continues only onto the same bytes, only in the session this op's item opened, and only where
        // core's `resume_policy` allows it (spec §4.3). Anything else abandons the old session and the op.
        let resumed = match previous {
            Some(previous) => {
                let stored = beebeeb_core::chunk_stream::ChunkFormat::from_u8(previous.chunk_format).ok();
                let continues = beebeeb_core::chunk_stream::resume_policy(stored, writing)
                    == beebeeb_core::chunk_stream::SessionResume::ContinueSession;
                let plan_ok = previous.chunk_size_bytes > 0
                    && previous.chunk_count > 0
                    && (0..=previous.chunk_count).contains(&previous.acked_chunks);
                if previous.fingerprint != before {
                    return self.drop_backup(claimed, Some(&previous), BackupDrop::SourceChanged).await;
                }
                if previous.server_file_id != op.file_id || !continues || !plan_ok {
                    return self.drop_backup(claimed, Some(&previous), BackupDrop::SessionNotResumable).await;
                }
                tracing::info!(
                    op_id = %op_id,
                    file_id = %op.file_id,
                    acked_chunks = previous.acked_chunks,
                    chunk_count = previous.chunk_count,
                    "backup: resuming persisted upload session"
                );
                Some(previous)
            }
            None => None,
        };
        let was_resumed = resumed.is_some();
        let session = match resumed {
            Some(session) => session,
            None => {
                let init_request = upload_init_request_for_operation(
                    &op.file_id,
                    &op.name_encrypted,
                    op.content_type.clone(),
                    Some(op.parent_id.clone()),
                    before.size,
                    None,
                );
                let upload = self.api.init_upload(&init_request).await?;
                if upload.chunk_size_bytes <= 0 || upload.chunk_count <= 0 {
                    return Err(anyhow::anyhow!("upload init returned invalid chunk plan").into());
                }
                // SF-0001 (L13): the server never picks this device's item id.
                if upload.file_id != op.file_id {
                    return Err(anyhow::anyhow!(
                        "server_changed_item_id: the upload init answered with an id this device did not choose"
                    )
                    .into());
                }
                let session = BackupUploadResume {
                    op_id: op_id.to_string(),
                    server_file_id: upload.file_id,
                    upload_session_id: upload.upload_session_id,
                    object_version_id: upload.object_version_id,
                    chunk_size_bytes: upload.chunk_size_bytes,
                    chunk_count: upload.chunk_count,
                    acked_chunks: 0,
                    chunk_format: writing.as_u8(),
                    fingerprint: before,
                };
                // Persist before the next await, so a cut anywhere after init leaves the session to the retry.
                if !self.db.put_backup_upload_resume_claimed(&session, &claimed.claim_id)? {
                    log_queue_state_moved(op_id, "backup_resume");
                    return Err(QueueStateMoved.into());
                }
                session
            }
        };

        let progress = match self.send_backup_chunks(&op, &session, file_key, pin).await {
            Ok(progress) => progress,
            Err(error) => {
                let error = land_error_to_anyhow(error);
                // A session the server no longer accepts is abandoned; the op starts over next pass.
                if was_resumed && upload_session_is_gone(&error) && matches!(error_http_status(&error), Some(404 | 410)) {
                    return self.drop_backup(claimed, Some(&session), BackupDrop::SessionNotResumable).await;
                }
                if upload_session_is_gone(&error) {
                    self.abandon_backup_session(&session).await?;
                }
                return Err(error.into());
            }
        };

        // Spec §4.3: the fingerprint after the last chunk. Any change abandons the session before `complete`, so the
        // bytes read from a file that changed meanwhile never become a version.
        let after = source_fingerprint(&op.source_path);
        if after != Ok(before) {
            return self.drop_backup(claimed, Some(&session), BackupDrop::SourceChanged).await;
        }
        let completed = match self.api.complete_upload_session(&session.upload_session_id).await {
            Ok(completed) => {
                progress.finish();
                completed
            }
            Err(error) => {
                if upload_session_is_gone(&error) {
                    self.abandon_backup_session(&session).await?;
                }
                return Err(error.into());
            }
        };
        let remote_id = uuid::Uuid::parse_str(&session.server_file_id)
            .map_err(|_| anyhow::anyhow!("a backup upload's item id is not a uuid"))?;
        let record = beebeeb_core::backup::BackupRecord {
            key: op.key.clone(),
            size: before.size,
            modified_ms: before.modified_ms,
            remote_id,
            uploaded_at_ms: unix_millis_now(),
            state: beebeeb_core::backup::RecordState::BackedUp,
        };
        if !self
            .db
            .complete_backup_upload_claimed(op_id, &claimed.claim_id, &op.account_owner, &op.category, &record)?
        {
            log_queue_state_moved(op_id, "backup_record");
            return Err(QueueStateMoved.into());
        }
        tracing::info!(
            op_id = %op_id,
            file_id = %session.server_file_id,
            bytes = before.size,
            version = completed["version_number"].as_i64(),
            "backup: uploaded from the source in place and recorded"
        );
        // Best effort, as for a sync-folder upload: the gallery's thumbnails, read from the source (read-only).
        let file_key = owned_key_and_pin(&self.db, self.api.master_key(), &session.server_file_id)?.0;
        if let Err(e) = self
            .upload_thumbnails_for_plaintext_media(
                ServerFileId::new(&session.server_file_id),
                &op.source_path,
                op.content_type.as_deref(),
                &file_key,
            )
            .await
        {
            tracing::warn!(
                file_id = %session.server_file_id,
                reason = error_category(&e),
                "backup: thumbnail skipped"
            );
            post_complete_errors.push(format!("{op_id}: {e}"));
        }
        Ok(OpDone::Removed { release: None })
    }

    /// The chunk PUTs from the acknowledged watermark on, read from the source in place. Every acknowledgement is
    /// persisted before the next await, so a cut resumes where the server's acknowledgements stop.
    async fn send_backup_chunks(
        &self,
        op: &BackupUploadOp,
        session: &BackupUploadResume,
        file_key: beebeeb_core::kdf::FileKey,
        pin: beebeeb_core::chunk_stream::ExpectedChunkFormat,
    ) -> Result<crate::transfer_progress::TransferGuard, EngineError> {
        let file = match open_source_read_only(&op.source_path) {
            Ok(file) => file,
            Err(_) => return Err(anyhow::anyhow!("backup source could not be opened").into()),
        };
        let chunk_size = u64::try_from(session.chunk_size_bytes).unwrap_or(0);
        let chunk_count = u64::try_from(session.chunk_count).unwrap_or(0);
        let first_chunk = u64::try_from(session.acked_chunks).unwrap_or(0);
        let mut writer = beebeeb_core::chunk_stream::ChunkEncryptor::from_reader_with_file_key(
            file_key,
            &session.server_file_id,
            session.fingerprint.size,
            chunk_size,
            file,
        )?
        .with_recorded_pin(pin)?;
        let total = session.fingerprint.size;
        let label = beebeeb_types::backup::Category::from_key(&op.category)
            .map(|category| format!("{}/{}", category.display_name(), op.key))
            .unwrap_or_else(|| op.key.clone());
        let progress = self
            .transfers
            .begin_named(&op.file_id, &label, crate::transfer_progress::Direction::Up, total);
        progress.update(crate::transfer_progress::uploaded_bytes(first_chunk, chunk_size, total));
        let upload_kbps_limit = crate::config::DesktopConfig::load()
            .map(|c| c.upload_kbps_limit)
            .unwrap_or(0);
        for chunk_index in 0..chunk_count {
            let encrypted = writer
                .next_chunk()?
                .ok_or_else(|| anyhow::anyhow!("backup source ended early"))?;
            if chunk_index < first_chunk {
                continue;
            }
            let read = encrypted.data.len().saturating_sub(28);
            let chunk_start = std::time::Instant::now();
            self.api
                .upload_session_chunk(&session.upload_session_id, chunk_index as u32, &encrypted.data)
                .await?;
            self.db
                .set_backup_upload_resume_acked(&op.op_id, chunk_index as i64 + 1)?;
            progress.update(crate::transfer_progress::uploaded_bytes(chunk_index + 1, chunk_size, total));
            self.wire.upload_bytes.fetch_add(read as u64, Ordering::Relaxed);
            if upload_kbps_limit > 0 {
                let budget_secs = read as f64 / (upload_kbps_limit as f64 * 1024.0);
                let elapsed_secs = chunk_start.elapsed().as_secs_f64();
                if budget_secs > elapsed_secs {
                    let sleep_ms = ((budget_secs - elapsed_secs) * 1000.0) as u64;
                    if sleep_ms > 0 {
                        tokio::time::sleep(Duration::from_millis(sleep_ms)).await;
                    }
                }
            }
        }
        writer.finish()?;
        Ok(progress)
    }

    /// Drop the claimed op with no record, abandoning its server session first (core's never-break rule: an
    /// unfinished create is abandoned, never trashed).
    async fn drop_backup(
        &self,
        claimed: &ClaimedOp,
        session: Option<&BackupUploadResume>,
        reason: BackupDrop,
    ) -> Result<OpDone, EngineError> {
        if let Some(session) = session {
            self.abandon_backup_session(session).await?;
        }
        if !self.db.drop_backup_op_claimed(&claimed.op.op_id, &claimed.claim_id)? {
            log_queue_state_moved(&claimed.op.op_id, "backup_drop");
            return Err(QueueStateMoved.into());
        }
        tracing::info!(
            op_id = %claimed.op.op_id,
            reason = reason.as_str(),
            "backup: op dropped with no record; the next pass plans the file again"
        );
        Ok(OpDone::Dropped)
    }

    /// `POST /api/v1/files/{id}/upload/abandon` for the session's never-completed create, then its row goes. A failed
    /// abandon is logged; the server's stale-upload sweep reaps the row.
    pub(super) async fn abandon_backup_session(&self, session: &BackupUploadResume) -> Result<(), EngineError> {
        match self.api.abandon_upload(&session.server_file_id).await {
            Ok(()) => tracing::info!(
                op_id = %session.op_id,
                file_id = %session.server_file_id,
                "backup: abandoned the unfinished upload session"
            ),
            Err(e) => tracing::warn!(
                op_id = %session.op_id,
                file_id = %session.server_file_id,
                reason = error_category(e.as_ref()),
                "backup: could not abandon the upload session; the server's stale-upload sweep reaps it"
            ),
        }
        self.db.clear_backup_upload_resume(&session.op_id)?;
        Ok(())
    }
}
