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
pub enum BackupDrop {
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
    /// A `Changed` upload whose remote file the server refused a new version of (409: trashed, or an upload of it in
    /// flight). The next pass reads the tree again and decides (a trashed copy is never given a version, ruling A).
    RemoteRefused,
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
            BackupDrop::RemoteRefused => "remote_refused",
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
pub fn open_source_read_only(path: &Path) -> std::io::Result<std::fs::File> {
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
    /// The backup pass's client: this session's, with `X-Beebeeb-Origin: backup` on every request (spec Amendment 8).
    /// Every request the backup pass makes goes through it, never through `self.api`.
    pub(super) fn backup_api(&self) -> &ApiClient {
        self.backup_api.get_or_init(|| Arc::new(self.api.for_backup_pass()))
    }

    /// `BackupHost::enqueue_upload` (spec §4.3): queue one file of a backed-up folder for this account. A `New` file
    /// gets a server id minted here; a `Changed` one is a new version of its record's `remote_id` (plan Amendment 1
    /// item 11), never a second file. The name is sealed for that id; the source path goes into the queue's
    /// `source_path` and nowhere else. Idempotent per (category, key), and a key whose record already has this
    /// fingerprint is not queued (`BackupEnqueue::AlreadyBackedUp`).
    // LD-0113 Task D4: `DesktopBackupHost` (`BackupHost::{enqueue_upload, records}`) is its production caller.
    #[cfg_attr(not(any(test, feature = "test-support")), allow(dead_code))]
    pub fn enqueue_backup_upload(&self, req: &beebeeb_backup::UploadRequest) -> Result<BackupEnqueue, EngineError> {
        if self.is_stopping() {
            return Err(anyhow::anyhow!("engine is stopping; refusing to enqueue a backup upload").into());
        }
        let (file_id, replace) = match (req.reason, req.remote_id) {
            (beebeeb_core::backup::UploadReason::Changed, Some(remote_id)) => (remote_id.to_string(), true),
            (beebeeb_core::backup::UploadReason::Changed, None) => {
                return Err(anyhow::anyhow!("a changed backup upload names no remote file").into());
            }
            (beebeeb_core::backup::UploadReason::New, _) => {
                let master = beebeeb_core::kdf::MasterKey::from_bytes(*self.api.master_key());
                (
                    beebeeb_core::item_id::NewItemId::owner(&master)?.as_str().to_string(),
                    false,
                )
            }
        };
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
            replace,
            record_key: req.record_key.as_deref(),
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
            return self
                .drop_backup(claimed, previous.as_ref(), BackupDrop::OtherAccount)
                .await;
        }
        // Spec §4.3: the fingerprint before the first chunk, taken now (the pass's is only what it saw).
        let before = match source_fingerprint(&op.source_path) {
            Ok(fingerprint) => fingerprint,
            Err(reason) => return self.drop_backup(claimed, previous.as_ref(), reason).await,
        };
        // Review (2026-11-07): a file another program holds open without read sharing (an Outlook .pst, a running
        // VM disk) is dropped as unreadable and planned again next pass, never retried in the queue until it parks.
        let source = match open_source_read_only(&op.source_path) {
            Ok(file) => file,
            Err(_) => {
                return self
                    .drop_backup(claimed, previous.as_ref(), BackupDrop::SourceUnreadable)
                    .await;
            }
        };
        // The item's key and current record. A new version of a remote file takes them from the server's row (the
        // backup's rows have no key record on this device): its pin, its counter and its current sealed name.
        let (file_key, record, current_name) = if op.replace {
            match self.backup_row_material(&op.file_id).await {
                Ok(material) => material,
                Err(error) => {
                    let error = land_error_to_anyhow(error);
                    // The remote file is gone: the next pass decides (a new file, or nothing).
                    if matches!(error_http_status(&error), Some(404 | 410)) {
                        return self
                            .drop_backup(claimed, previous.as_ref(), BackupDrop::RemoteRefused)
                            .await;
                    }
                    return Err(error.into());
                }
            }
        } else {
            owned_material(&self.db, self.api.master_key(), &op.file_id)?
        };
        let writing = beebeeb_core::chunk_stream::chunk_format_for_pin(record.format);

        // A persisted session and the retry (spec §4.3, chunk format v2). A v2 writer never continues a stream it did
        // not start (core's `resume_policy` continues only v1 onto v1, and this core writes v2), so a session with
        // chunks still to send is abandoned with its op and the next pass plans the file again. One the server holds
        // EVERY chunk of, for the same bytes, with its completion name stored, is completed again and nothing else:
        // its `complete` may already have been sent, and dropping it would back the file up a second time under a
        // new id (`upload_v2::resume_step`).
        let resumed = match previous {
            Some(previous) => {
                if previous.fingerprint != before {
                    return self
                        .drop_backup(claimed, Some(&previous), BackupDrop::SourceChanged)
                        .await;
                }
                let plan_ok = previous.chunk_size_bytes > 0
                    && previous.chunk_count > 0
                    && (0..=previous.chunk_count).contains(&previous.acked_chunks);
                let complete_only = previous.server_file_id == op.file_id
                    && plan_ok
                    && upload_v2::resume_step(true, previous.acked_chunks, previous.chunk_count)
                        == upload_v2::ResumeStep::CompleteOnly;
                match self.db.backup_upload_completion_name(op_id)? {
                    Some(name) if complete_only => {
                        tracing::info!(
                            op_id = %op_id,
                            file_id = %op.file_id,
                            chunk_count = previous.chunk_count,
                            "backup: the server holds every chunk of the persisted session — completing it"
                        );
                        Some((previous, name))
                    }
                    _ => {
                        return self
                            .drop_backup(claimed, Some(&previous), BackupDrop::SessionNotResumable)
                            .await;
                    }
                }
            }
            None => None,
        };
        let (resumed, stored_completion_name) = match resumed {
            Some((session, name)) => (Some(session), Some(name)),
            None => (None, None),
        };
        let session = match resumed {
            Some(session) => session,
            None => {
                let mut init_request = upload_init_request_for_operation(
                    &op.file_id,
                    &op.name_encrypted,
                    op.content_type.clone(),
                    Some(op.parent_id.clone()),
                    before.size,
                    None,
                );
                // A new version of a version-protected file: the server wants its current sealed name.
                if op.replace && record.version.is_some() {
                    init_request.version_metadata_encrypted = current_name.clone();
                }
                let upload = match self.backup_api().init_upload(&init_request).await {
                    Ok(upload) => upload,
                    // A new version of a remote file the server will not add one to now: the next pass decides.
                    Err(error) if op.replace && error_http_status(&error) == Some(409) => {
                        return self.drop_backup(claimed, None, BackupDrop::RemoteRefused).await;
                    }
                    Err(error) => return Err(error.into()),
                };
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

        let name_key = beebeeb_core::kdf::FileKey::from_bytes(*file_key.as_bytes());
        let (progress, completion_name) = match stored_completion_name {
            // Every chunk is on the server: nothing is read or sent again.
            Some(name) => (None, name),
            None => match self.send_backup_chunks(&op, &session, file_key, record, source).await {
                Ok((progress, name)) => (Some(progress), name),
                Err(error) => {
                    let error = land_error_to_anyhow(error);
                    // A session the server no longer accepts is abandoned; the retry opens a fresh one.
                    if upload_session_is_gone(&error) {
                        self.abandon_backup_session(&session).await?;
                    }
                    return Err(error.into());
                }
            },
        };

        // Spec §4.3: the fingerprint after the last chunk. Any change abandons the session before `complete`, so the
        // bytes read from a file that changed meanwhile never become a version.
        let after = source_fingerprint(&op.source_path);
        if after != Ok(before) {
            return self
                .drop_backup(claimed, Some(&session), BackupDrop::SourceChanged)
                .await;
        }
        let completed = match self
            .backup_api()
            .complete_upload_session(&session.upload_session_id, Some(&completion_name))
            .await
        {
            Ok(completed) => completed,
            Err(error) => {
                // The remote file was renamed while this version uploaded: the completion name sealed for the old
                // name is refused. The session and the op go; the next pass plans the file from the row as it is.
                if upload_file_changed(&error) {
                    return self
                        .drop_backup(claimed, Some(&session), BackupDrop::RemoteRefused)
                        .await;
                }
                if upload_session_is_gone(&error) {
                    self.abandon_backup_session(&session).await?;
                }
                return Err(error.into());
            }
        };
        // Recorded only once the server holds this upload's commitment in the file's name
        // (`publish_completion_name`): without it the new version opens nowhere. A failure here is the queue's retry,
        // which finds every chunk acknowledged and completes again (the server answers `already_completed`).
        self.publish_completion_name(
            self.backup_api(),
            &session.server_file_id,
            &name_key,
            &completion_name,
            &session.object_version_id,
            &completed,
        )
        .await?;
        if let Some(progress) = progress {
            progress.finish();
        }
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
        if !self.db.complete_backup_upload_claimed(
            op_id,
            &claimed.claim_id,
            &op.account_owner,
            &op.category,
            &record,
        )? {
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
        if let Err(e) = self.upload_backup_thumbnails(&session.server_file_id, &op).await {
            tracing::warn!(
                file_id = %session.server_file_id,
                reason = error_category(&e),
                "backup: thumbnail skipped"
            );
            post_complete_errors.push(format!("{op_id}: {e}"));
        }
        Ok(OpDone::Removed { release: None })
    }

    /// The gallery's thumbnails of a completed photo or video backup, read from the source (read-only), sent through
    /// the backup client.
    async fn upload_backup_thumbnails(&self, server_file_id: &str, op: &BackupUploadOp) -> Result<(), EngineError> {
        let (file_key, pin) = owned_key_and_pin(&self.db, self.api.master_key(), server_file_id)?;
        let uploads = prepare_thumbnail_uploads_with_pin(&op.source_path, op.content_type.as_deref(), &file_key, pin)?;
        for upload in uploads {
            self.backup_api()
                .upload_thumbnail(
                    server_file_id,
                    upload.variant.label(),
                    &upload.encrypted,
                    upload.blurhash.as_deref(),
                )
                .await
                .map_err(|e| anyhow::anyhow!("upload {} thumbnail: {e}", upload.variant.label()))?;
        }
        Ok(())
    }

    /// The chunk PUTs of a fresh session, read from the source in place, and the sealed name `complete` publishes
    /// for them (the item's name with this upload's version commitment, sealed once frame 0 is). Every
    /// acknowledgement is persisted before the next await. A session an earlier attempt left is never continued
    /// here: core's `resume_policy` continues only v1 onto v1, and this writer writes v2.
    async fn send_backup_chunks(
        &self,
        op: &BackupUploadOp,
        session: &BackupUploadResume,
        file_key: beebeeb_core::kdf::FileKey,
        record: crate::content_record::ContentRecord,
        file: std::fs::File,
    ) -> Result<(crate::transfer_progress::TransferGuard, String), EngineError> {
        let chunk_size = u64::try_from(session.chunk_size_bytes).unwrap_or(0);
        let chunk_count = u64::try_from(session.chunk_count).unwrap_or(0);
        let first_chunk = u64::try_from(session.acked_chunks).unwrap_or(0);
        if first_chunk != 0 {
            return Err(anyhow::anyhow!("a backup upload never continues a persisted session").into());
        }
        let name_key = beebeeb_core::kdf::FileKey::from_bytes(*file_key.as_bytes());
        let mut writer = beebeeb_core::chunk_stream::ChunkEncryptor::from_reader_with_file_key(
            file_key,
            &session.server_file_id,
            session.fingerprint.size,
            chunk_size,
            file,
        )?
        .with_recorded_pin(record.format)?;
        let mut completion_name = None;
        let total = session.fingerprint.size;
        let label = beebeeb_types::backup::Category::from_key(&op.category)
            .map(|category| format!("{}/{}", category.display_name(), op.key))
            .unwrap_or_else(|| op.key.clone());
        let progress = self
            .transfers
            .begin_named(&op.file_id, &label, crate::transfer_progress::Direction::Up, total);
        progress.update(crate::transfer_progress::uploaded_bytes(first_chunk, chunk_size, total));
        let upload_kbps_limit = crate::config::DesktopConfig::load_for(self.identity)
            .map(|c| c.upload_kbps_limit)
            .unwrap_or(0);
        for chunk_index in 0..chunk_count {
            let encrypted = writer
                .next_chunk()?
                .ok_or_else(|| anyhow::anyhow!("backup source ended early"))?;
            if chunk_index == 0 {
                let (name, mime) =
                    upload_v2::completion_plain_name(&name_key, &op.name_encrypted, None, op.content_type.as_deref())?;
                let sealed = upload_v2::seal_completion_name(
                    &self.db,
                    &session.server_file_id,
                    &writer,
                    &name_key,
                    record,
                    !op.replace,
                    &name,
                    mime.as_deref(),
                )?;
                // Stored BEFORE the first chunk is sent: a retry that finds every chunk acknowledged completes
                // with it, without the writer.
                self.db.set_backup_upload_completion_name(&op.op_id, &sealed)?;
                completion_name = Some(sealed);
            }
            let read = encrypted.data.len().saturating_sub(28);
            let chunk_start = std::time::Instant::now();
            self.backup_api()
                .upload_session_chunk(&session.upload_session_id, chunk_index as u32, &encrypted.data)
                .await?;
            self.db
                .set_backup_upload_resume_acked(&op.op_id, chunk_index as i64 + 1)?;
            progress.update(crate::transfer_progress::uploaded_bytes(
                chunk_index + 1,
                chunk_size,
                total,
            ));
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
        let completion_name =
            completion_name.ok_or_else(|| anyhow::anyhow!("a backup upload sealed no chunk and so no name"))?;
        Ok((progress, completion_name))
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

    /// `BackupHost::turn_off`'s queue half (spec §4.2 rule 11): drop every queued upload of `category` and abandon
    /// each open session on the server. A failed abandon is logged; the server's stale-upload sweep reaps it. The
    /// records stay (see `StateDb::take_backup_category_ops`). Answers how many ops were dropped.
    // LD-0113 Task D4: `DesktopBackupHost::turn_off` and the settings' "turn off" are its production callers.
    #[cfg_attr(not(any(test, feature = "test-support")), allow(dead_code))]
    pub async fn turn_off_backup_uploads(&self, category: &str) -> Result<usize, EngineError> {
        let (dropped, sessions) = self.db.take_backup_category_ops(category)?;
        let abandoned = sessions.len();
        for session in &sessions {
            self.abandon_backup_session(session).await?;
        }
        tracing::info!(
            dropped,
            abandoned,
            "backup: turned off; queued uploads dropped and sessions abandoned"
        );
        Ok(dropped)
    }

    /// Abandon every backup session whose op is gone (review 2026-11-07: the Settings switch purges the queue but
    /// has no API client). A failed abandon keeps the row for the next tick.
    pub async fn abandon_orphaned_backup_sessions(&self) {
        let sessions = match self.db.orphaned_backup_sessions() {
            Ok(sessions) => sessions,
            Err(e) => {
                tracing::warn!(error = %e, "backup: orphaned sessions not read");
                return;
            }
        };
        for session in &sessions {
            if let Err(e) = self.backup_api().abandon_upload(&session.server_file_id).await {
                tracing::warn!(file_id = %session.server_file_id, reason = error_category(e.as_ref()), "backup: orphaned session not abandoned yet");
                continue;
            }
            if let Err(e) = self.db.clear_backup_upload_resume(&session.op_id) {
                tracing::warn!(op_id = %session.op_id, error = %e, "backup: abandoned session row not cleared");
            }
        }
        if !sessions.is_empty() {
            tracing::info!(sessions = sessions.len(), "backup: orphaned upload sessions abandoned");
        }
    }

    /// `POST /api/v1/files/{id}/upload/abandon` for the session's never-completed create, then its row goes. A failed
    /// abandon is logged; the server's stale-upload sweep reaps the row.
    pub(super) async fn abandon_backup_session(&self, session: &BackupUploadResume) -> Result<(), EngineError> {
        match self.backup_api().abandon_upload(&session.server_file_id).await {
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
