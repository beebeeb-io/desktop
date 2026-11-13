//! `upload` — the `EngineBridge` impl for the upload path: version upload, landing and the upload session, split out of `engine_bridge.rs`.

use super::*;

impl EngineBridge {
    /// Upload one op's staged bytes. `claim` is the runner's claim of the op; every
    /// queue write the upload makes is guarded by it (spec §8.7 S4). `None` is Keep
    /// Mine's inline run, outside the queue: its writes stay unguarded. The `release` of
    /// the returned `OpDone` is, on macOS, the staged payload, which the caller unlinks
    /// once the op is gone.
    pub(super) async fn upload_version(
        &self,
        op: &PendingOperation,
        claim: Option<&ClaimedOp>,
        sync_root: &Path,
        post_complete_errors: &mut Vec<String>,
    ) -> Result<OpDone, EngineError> {
        // LD-0113: a backup op never takes the sync-root upload: no `files` row, no landing, no staged-copy cleanup,
        // no finalization. `execute_operation` sends it to `upload_backup`; this refusal keeps every other caller out.
        if claim.is_some_and(|claim| claim.origin == crate::state_db::OpOrigin::Backup) {
            return Err(anyhow::anyhow!("a backup op never takes the sync-root upload path").into());
        }
        let local_file_id = op
            .file_id
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("upload operation missing file_id"))?;
        // L13: a request row's content lives under the anonymous uploader's `C`
        // and is read-only. An edit is saved as a NEW owner item under a fresh
        // id; the row itself is never re-sealed (P1-1).
        if self.is_request_row(local_file_id) {
            return self.replan_request_edit_as_new_item(local_file_id, op, sync_root).await;
        }
        let previous_status = self.db.get_file(local_file_id)?.map(|entry| entry.status);
        // Write-keyed: a File Provider write (spec §9.1–§9.2). Keep Mine (`claim: None`) and
        // Windows and watcher uploads keep today's status handling.
        let finder_write = claim.and_then(|c| c.write.as_ref()).is_some();
        // §9.1 (m-8): an item in the trash never looks writable. A Finder write's attempt
        // leaves a `Trashing` row in the trash; its `init` meets the server's refusal and retries.
        if finder_write {
            self.db.set_uploading_unless_trashing(local_file_id)?;
        } else {
            self.db.set_status(local_file_id, FileStatus::Uploading)?;
        }

        struct Rollback<'a> {
            db: &'a StateDb,
            id: LocalFileId<'a>,
            previous: Option<FileStatus>,
            finder_write: bool,
        }
        impl Drop for Rollback<'_> {
            fn drop(&mut self) {
                // §9.2: a Finder write's row stays `Uploading` while its op will retry; a park
                // sets `Error` in the park's own transaction (`settle_status_after_park_conn`).
                if self.finder_write {
                    return;
                }
                // A server-completed upload has already committed Local; don't
                // undo that if cancellation happens during thumbnail work.
                if self
                    .db
                    .get_file(self.id.as_str())
                    .ok()
                    .flatten()
                    .is_some_and(|r| r.status == FileStatus::Uploading)
                {
                    let status = if self.previous == Some(FileStatus::Conflict) {
                        FileStatus::Conflict
                    } else {
                        FileStatus::Error
                    };
                    let _ = self.db.set_status(self.id.as_str(), status);
                }
            }
        }
        // The row's key from here on is a local file id, not a bare string: the
        // upload and its rollback both address the same `files` row by it.
        let local_file_id = LocalFileId::new(local_file_id);
        let _rollback = Rollback {
            db: &self.db,
            id: local_file_id,
            previous: previous_status,
            finder_write,
        };
        self.do_upload_version(local_file_id, op, claim, sync_root, post_complete_errors)
            .await
    }

    async fn do_upload_version(
        &self,
        local_file_id: LocalFileId<'_>,
        op: &PendingOperation,
        claim: Option<&ClaimedOp>,
        #[cfg_attr(not(target_os = "windows"), allow(unused_variables))] sync_root: &Path,
        post_complete_errors: &mut Vec<String>,
    ) -> Result<OpDone, EngineError> {
        // §8.6 rules 4 and 6: a recorded completion lands without the network, before anything
        // else is checked, because it never parks, not even on a missing payload. Write-keyed:
        // a completion is recorded only for a File Provider write, so only one can be found
        // here; Windows and watcher uploads never take this shortcut.
        if claim.is_some_and(|claim| claim.write.is_some())
            && let Some(previous) = self.db.get_upload_resume(&op.op_id)?
            && let (Some(version), Some(object)) =
                (previous.completed_version, previous.completed_object_version_id.clone())
        {
            // The save's own content type, as the first attempt had it, and the server's
            // `mime_type` recorded with the completion, its fallback.
            let content_type = op
                .metadata_json
                .as_deref()
                .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
                .and_then(|metadata| metadata["content_type"].as_str().map(str::to_string));
            return self
                .land(LandRequest {
                    local_file_id,
                    op,
                    claim,
                    server_file_id: ServerFileId::new(&previous.server_file_id),
                    produced: ProducedObject {
                        version_number: version,
                        object_version_id: VersionId::new(&object),
                    },
                    size_bytes: previous.payload_size as u64,
                    content_type,
                    mime_type: previous.completed_mime_type.clone(),
                    post_complete_errors,
                    sync_root,
                })
                .await;
        }
        let payload_path = op
            .payload_path
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("upload operation missing staged payload"))?;
        let payload_path = Path::new(payload_path);
        match std::fs::metadata(payload_path) {
            Ok(meta) if meta.is_file() => {}
            // The staged copy lives in the app's own container and cannot come back
            // (spec §8.4). Only a File Provider write parks on it.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && claim.and_then(|c| c.write.as_ref()).is_some() => {
                return Err(anyhow::Error::new(ParkNow(ParkReason::PayloadMissing)).into());
            }
            // An op without a write id keeps today's retry and today's text
            // (`test_process_due_operations_records_retry_for_upload_worker_handoff`), without
            // the path: `classify_operation_error` matches bare substrings such as `403`, so a
            // path would pause the op instead.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(anyhow::anyhow!("staged upload payload is missing").into());
            }
            // Every other read error (permission, I/O, not a regular file) retries as
            // today (spec §8.4), with a fixed text. Neither the path nor the I/O error
            // reaches `classify_operation_error`: it matches bare substrings such as
            // `403` and `permission`, and would pause the op instead.
            _ => return Err(anyhow::anyhow!("staged upload payload could not be read").into()),
        }

        let metadata = operation_metadata(op)?;
        let name_encrypted = metadata["name_encrypted"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("upload operation missing encrypted name"))?;
        let content_type = metadata["content_type"].as_str().map(str::to_string);
        // An empty (0-byte) file is a normal upload: the canonical plan for it is
        // ONE chunk carrying the AEAD of zero bytes (`plan_chunks(0, _)` →
        // `chunk_count == 1`), which the chunk loop below sends as a 28-byte
        // nonce + tag. Requires a server whose `/uploads/init` accepts
        // `file_size_bytes: 0`.
        let plaintext_size = std::fs::metadata(payload_path)?.len();

        let is_create = is_create_file_operation(&metadata);
        let payload_path_str = payload_path.to_string_lossy().into_owned();
        let payload_mtime_ns = payload_mtime_ns(payload_path);

        // A persisted session and the retry (chunk format v2). The server holds every chunk and the
        // bytes are the same: `complete` is sent (again) and nothing else, so an answer lost on the
        // way never uploads the file twice. Anything else is abandoned and restarted from chunk 0 in
        // a fresh session: a new writer has a new version id, and its frames never join an older
        // stream's, v1 (a session a build before v2 opened) or v2 (`upload_v2::resume_step`).
        let mut session: Option<UploadResume> = None;
        if let Some(previous) = self.db.get_upload_resume(&op.op_id)? {
            if previous.is_create && !previous.self_minted {
                // SF-0001 (L13): an older version let the server mint a create's id.
                // Such a session can never be ours again, whatever its payload: it
                // is abandoned and this attempt starts a fresh, self-minted upload.
                tracing::info!(
                    op_id = %op.op_id,
                    file_id = %previous.server_file_id,
                    "upload: persisted create session was not self-minted — abandoning it and starting a fresh upload"
                );
                self.abandon_upload_session(&previous).await?;
            } else {
                let same_payload = previous.payload_path == payload_path_str
                    && previous.payload_size == plaintext_size as i64
                    && previous.payload_mtime_ns == payload_mtime_ns
                    && previous.chunk_size_bytes > 0
                    && previous.chunk_count > 0
                    && previous.acked_chunks >= 0
                    && previous.acked_chunks <= previous.chunk_count;
                match upload_v2::resume_step(same_payload, previous.acked_chunks, previous.chunk_count) {
                    upload_v2::ResumeStep::CompleteOnly => {
                        tracing::info!(
                            op_id = %op.op_id,
                            file_id = %previous.server_file_id,
                            chunk_count = previous.chunk_count,
                            "upload: the server holds every chunk of the persisted session — completing it"
                        );
                        session = Some(previous);
                    }
                    upload_v2::ResumeStep::Restart if same_payload => {
                        // The resume row stays until the fresh init replaces it: an init the
                        // server refuses (a lease still live) leaves the session findable.
                        self.abandon_before_restart(&self.api, &op.op_id, &previous.server_file_id)
                            .await;
                    }
                    upload_v2::ResumeStep::Restart => {
                        tracing::info!(
                            op_id = %op.op_id,
                            file_id = %previous.server_file_id,
                            "upload: staged payload changed since the persisted session — abandoning it"
                        );
                        self.abandon_upload_session(&previous).await?;
                    }
                }
            }
        }

        // Only a session resumed from its persisted row can have been reaped by the server (each
        // chunk renews a session's lease, so one opened in this attempt cannot be).
        let resumed = session.is_some();
        let session = match session {
            Some(session) => session,
            None => {
                let init_kind = if is_create {
                    InitKind::Create
                } else if claim.and_then(|claim| claim.write.as_ref()).is_some() {
                    InitKind::FinderReplace
                } else {
                    InitKind::Replace
                };
                guard_init_base(op.base_version, init_kind)?;
                // SF-0001 (L13): this device chooses the item id. A create proposes
                // its own id when that id is canonical; otherwise (a local-only id)
                // it mints a fresh one for this init. A replace keeps the row's id.
                let proposed_id = if is_create && !is_canonical_item_id(local_file_id.as_str()) {
                    uuid::Uuid::new_v4().to_string()
                } else {
                    local_file_id.as_str().to_string()
                };
                // The name init carries. A create's is the row's name until `complete` publishes the
                // one with the version commitment: sealed for the id this init proposes (the queued
                // name was sealed for the local id). A replacement's is not stored by the server,
                // which keeps the file's name until `complete`; it wants the file's CURRENT sealed
                // name as the version metadata instead, when that version is protected.
                let init_name = if is_create && proposed_id != local_file_id.as_str() {
                    owned_metadata_for_name(
                        &self.db,
                        self.api.master_key(),
                        &proposed_id,
                        metadata_display_name(&metadata, op).as_deref().unwrap_or(&proposed_id),
                        content_type.as_deref(),
                    )?
                } else {
                    name_encrypted.to_string()
                };
                let mut init_request = upload_init_request_for_operation(
                    &proposed_id,
                    &init_name,
                    content_type.clone(),
                    op.parent_id.clone(),
                    plaintext_size,
                    op.base_version,
                );
                if !is_create {
                    let (_, current, current_name) = owned_material(&self.db, self.api.master_key(), &proposed_id)?;
                    init_request.version_metadata_encrypted = current_name.filter(|_| current.version.is_some());
                }
                let upload = self.api.init_upload(&init_request).await?;
                if upload.chunk_size_bytes <= 0 || upload.chunk_count <= 0 {
                    return Err(anyhow::anyhow!("upload init returned invalid chunk plan").into());
                }
                // Before anything is persisted or sealed: an init that answers with
                // another id is not this device's item. Exact equality — a re-spelled
                // echo (upper case, trimming) is a different id.
                if upload.file_id != proposed_id {
                    return Err(anyhow::anyhow!(
                        "server_changed_item_id: the upload init answered with an id this device did not choose"
                    )
                    .into());
                }
                let session = UploadResume {
                    op_id: op.op_id.clone(),
                    payload_path: payload_path_str.clone(),
                    payload_size: plaintext_size as i64,
                    payload_mtime_ns,
                    upload_session_id: upload.upload_session_id,
                    server_file_id: upload.file_id,
                    object_version_id: upload.object_version_id,
                    chunk_size_bytes: upload.chunk_size_bytes,
                    chunk_count: upload.chunk_count,
                    acked_chunks: 0,
                    metadata_applied: false,
                    // The op kind, not whether init carried an id: every init now does.
                    is_create,
                    // Every session this code opens sends an id it chose itself.
                    self_minted: true,
                    completed_version: None,
                    completed_object_version_id: None,
                    completed_mime_type: None,
                };
                // Persist BEFORE the next await: a cut anywhere after init must
                // leave the session discoverable by the retry.
                match claim {
                    Some(claim) => {
                        if !self.db.put_upload_resume_claimed(&session, &claim.claim_id)? {
                            log_queue_state_moved(&op.op_id, "resume");
                            return Err(anyhow::Error::new(QueueStateMoved).into());
                        }
                    }
                    // Keep Mine runs inline, outside the queue: no claim guards it.
                    None => self.db.put_upload_resume(&session)?,
                }
                session
            }
        };
        // SF-0001 (L13): whatever the session's origin (resumed or fresh), it must
        // target the item this op writes before any byte or name goes to the server.
        ensure_session_item_id(&session, local_file_id)?;

        let body = self
            .upload_session_body(
                local_file_id,
                op,
                &metadata,
                name_encrypted,
                content_type.clone(),
                payload_path,
                &session,
            )
            .await;
        match body {
            Ok(completed) => {
                // §8.6.1: the version this upload produced, from the server's reply. An
                // idempotent repeat answers `already_completed` with neither field; its version
                // is then base + 1 (1 for a create), its object id the session's. An upload
                // without a base (Windows and the watcher) keeps the stored version + 1.
                let produced_version = match completed["version_number"].as_i64() {
                    Some(version) => version,
                    None if is_create => 1,
                    None => match op.base_version {
                        Some(base) => base.saturating_add(1),
                        None => self
                            .db
                            .get_file_contract_state(local_file_id.as_str())?
                            .map_or(0, |contract| contract.current_version)
                            .saturating_add(1),
                    },
                };
                let produced_object = completed["current_object_version_id"]
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| session.object_version_id.clone());
                let mime_type = completed["mime_type"].as_str().map(str::to_string);
                if let Some(claim) = claim
                    && claim.write.is_some() // write-keyed
                    && !self.db.record_completion_claimed(
                        &op.op_id,
                        &claim.claim_id,
                        produced_version,
                        &produced_object,
                        mime_type.as_deref(),
                    )?
                {
                    log_queue_state_moved(&op.op_id, "completion");
                    return Err(anyhow::Error::new(QueueStateMoved).into());
                }
                let size = completed["size_bytes"].as_u64().unwrap_or(plaintext_size);
                ensure_session_item_id(&session, local_file_id)?;
                self.land(LandRequest {
                    local_file_id,
                    op,
                    claim,
                    server_file_id: ServerFileId::new(&session.server_file_id),
                    produced: ProducedObject {
                        version_number: produced_version,
                        object_version_id: VersionId::new(&produced_object),
                    },
                    size_bytes: size,
                    content_type,
                    mime_type,
                    post_complete_errors,
                    sync_root,
                })
                .await
            }
            Err(error) => {
                // The session checks and `SessionAbandoned` speak `anyhow`, as the API client
                // does; `land_error_to_anyhow` hands them the error itself, so a marker it
                // carries stays findable.
                let error = land_error_to_anyhow(error);
                if upload_session_is_gone(&error) {
                    let status = error_http_status(&error);
                    // The status and a category only: the error's text carries the request URL
                    // with the session id (spec §11).
                    tracing::warn!(
                        op_id = %op.op_id,
                        file_id = %session.server_file_id,
                        status,
                        resumed,
                        reason = error_category(error.as_ref()),
                        "upload: server no longer accepts the upload session — abandoning it; the next attempt starts a fresh upload"
                    );
                    self.abandon_upload_session(&session).await?;
                    // §8.4: a resumed session the server reaped (404, 410) is not a counted
                    // failure: the next attempt starts a fresh upload. A 400, 404 or 410 at a
                    // session opened in this attempt, and a 400 at a resumed one, is counted, so a
                    // server that refuses every new session still parks the write.
                    if resumed && matches!(status, Some(404 | 410)) {
                        return Err(anyhow::Error::new(SessionAbandoned(error)).into());
                    }
                }
                Err(error.into())
            }
        }
    }

    /// The landing (spec §8.6.2): one transaction (`StateDb::apply_landing`), then the work
    /// that follows its commit. Keep Mine passes `claim: None`: the landing removes nothing
    /// it does not own, and the caller unlinks the released copy. The error type is the
    /// bridge's own; `land_error_to_anyhow` hands it back at the call sites.
    async fn land(&self, request: LandRequest<'_>) -> Result<OpDone, EngineError> {
        let LandRequest {
            local_file_id,
            op,
            claim,
            server_file_id,
            produced,
            size_bytes,
            content_type,
            mime_type,
            post_complete_errors,
            sync_root,
        } = request;
        self.seam("landing:after_complete");
        // LD-0113: every cleanup below acts on this path (the release, the Windows finalization journal,
        // `finish_completed_upload`'s unlink). A backup claim never gets here (`upload_version` refuses it); if one
        // did, it would carry no path to remove.
        let payload_path = match claim.map(|claim| claim.origin) {
            Some(crate::state_db::OpOrigin::Backup) => None,
            _ => op.payload_path.clone(),
        };
        // macOS only: `release` says only what the caller unlinks after the commit (S6).
        // Elsewhere `finish_completed_upload` unlinks the copy, as before.
        let release = if cfg!(target_os = "macos") {
            payload_path.clone()
        } else {
            None
        };
        // Windows: a create's finalization journal row, read from local state only, and
        // written in the landing's own transaction (the journal then owns the retry).
        #[cfg(target_os = "windows")]
        let finalization = match payload_path.as_deref() {
            Some(path) => self
                .defer_local_upload_finalization(op, server_file_id, sync_root, Path::new(path))
                .map_err(EngineError::from)?,
            None => None,
        };
        #[cfg(not(target_os = "windows"))]
        let finalization: Option<crate::state_db::UploadFinalization> = None;
        // §5.6: a Finder write's thumbnails are uploaded after the landing commits, and the
        // commit takes the op off the queue. From just before it to the end of that upload the
        // server holds only the previous version's thumbnail, which the system would cache under
        // the token: `finder_thumbnail` refuses meanwhile. Keyed on a minted write (macOS).
        let marks_thumbnails = claim.and_then(|c| c.write.as_ref()).is_some() && payload_path.is_some();
        if marks_thumbnails {
            self.set_thumbnails_pending(server_file_id.as_str(), ThumbnailWait::Waiting);
        }
        // Task 1640: a sync-folder upload (no write id) records what it sent as the content this
        // device now agrees with the server on. A Finder write's copy is not read again.
        let completed_payload_hash = match (claim.and_then(|c| c.write.as_ref()), payload_path.as_deref()) {
            (None, Some(path)) => match sha256_file_hex(Path::new(path)) {
                Ok(hash) => Some(hash),
                Err(e) => {
                    tracing::warn!(
                        op_id = %op.op_id,
                        error = %e,
                        "landing: could not hash the copy that was sent; the file keeps its previous agreed content"
                    );
                    None
                }
            },
            _ => None,
        };
        let master_key = self.api.master_key();
        // The item's key and CURRENT record (the completed upload's commitment included): a queued
        // successor's name is sealed again with them.
        let landing_material =
            owned_key_and_record(&self.db, master_key, server_file_id.as_str()).map_err(EngineError::from)?;
        let landed = self.db.apply_landing(
            &crate::state_db::LandingInput {
                op_id: &op.op_id,
                claim_id: claim.map(|c| c.claim_id.as_str()),
                write_id: claim.and_then(|c| c.write.as_ref()).map(|w| w.write_id.as_str()),
                local_file_id: local_file_id.as_str(),
                server_file_id: server_file_id.as_str(),
                target_path: op.target_path.as_deref(),
                parent_id: op.parent_id.as_deref(),
                landed_base: op.base_version,
                produced_version: produced.version_number,
                produced_object_version_id: produced.object_version_id.as_str(),
                size_bytes: i64::try_from(size_bytes).unwrap_or(i64::MAX),
                content_type: content_type.as_deref(),
                mime_type: mime_type.as_deref(),
                // Always the op's payload path, whatever `release` is.
                completed_payload: payload_path.as_deref(),
                completed_payload_hash: completed_payload_hash.as_deref(),
                finalization: finalization.as_ref(),
                now: now_secs(),
            },
            &|queued, server_id| {
                metadata_rekeyed_with_material(
                    master_key,
                    queued,
                    server_id,
                    (server_id == server_file_id.as_str()).then_some((&landing_material.0, landing_material.1)),
                )
                .map_err(land_error_to_anyhow)
            },
        );
        // Nothing landed: the op is still queued, and the refusal is the queue's again.
        let landed = match landed {
            Ok(Some(landed)) => landed,
            Ok(None) => {
                if marks_thumbnails {
                    self.set_thumbnails_pending(server_file_id.as_str(), ThumbnailWait::Settled);
                }
                log_queue_state_moved(&op.op_id, "landing");
                return Err(QueueStateMoved.into());
            }
            Err(e) => {
                if marks_thumbnails {
                    self.set_thumbnails_pending(server_file_id.as_str(), ThumbnailWait::Settled);
                }
                return Err(e.into());
            }
        };
        for (successor, reason) in &landed.parked_successors {
            log_parked(successor, Some(server_file_id.as_str()), *reason);
        }
        self.record_transfer_done(
            crate::transfer_progress::Direction::Up,
            server_file_id.as_str(),
            size_bytes,
        );
        if let Some(path) = payload_path.as_deref() {
            // Task 1700: post-complete thumbnail work never fails the upload, but its
            // failure is surfaced on the outcome so a red run names the real error.
            let file_key = owned_key_and_pin(&self.db, self.api.master_key(), server_file_id.as_str())
                .map_err(EngineError::from)?
                .0;
            match self
                .finish_completed_upload(op, server_file_id, Path::new(path), content_type, &file_key, sync_root)
                .await
            {
                Ok(()) => {
                    if marks_thumbnails {
                        self.set_thumbnails_pending(server_file_id.as_str(), ThumbnailWait::Settled);
                    }
                }
                // The refusal stays: the server still holds the previous version's thumbnail.
                Err(e) => {
                    // A category only: the error's text can carry the request URL (spec §11).
                    tracing::warn!(
                        file_id = %server_file_id,
                        reason = error_category(&e),
                        "upload-time thumbnail generation/upload skipped"
                    );
                    post_complete_errors.push(format!("{}: {e}", op.op_id));
                }
            }
        }
        Ok(if claim.is_some() {
            OpDone::Removed { release }
        } else {
            OpDone::Remove { release }
        })
    }

    /// Everything between init and a successful `complete`: the post-init
    /// metadata PATCH and the chunk PUTs from the acknowledged watermark on.
    /// Every step's success is persisted before the next await so a cut
    /// resumes exactly where the server's acknowledgements stop.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn upload_session_body(
        &self,
        local_file_id: LocalFileId<'_>,
        op: &PendingOperation,
        metadata: &serde_json::Value,
        name_encrypted: &str,
        content_type: Option<String>,
        payload_path: &Path,
        session: &UploadResume,
    ) -> Result<serde_json::Value, EngineError> {
        // SF-0001 (L13): never seal a name or send a chunk under an id this device
        // did not choose, even when the session was read back from disk.
        ensure_session_item_id(session, local_file_id)?;
        let server_file_id = session.server_file_id.clone();
        if !session.metadata_applied {
            // The move this upload carries (never the name: `complete` publishes the name together
            // with the new version's commitment, and the server refuses a completion name for a file
            // renamed since init).
            // A19 (P2-7): an upload whose metadata step moves the item out of a
            // shared folder takes the same adoption guard as a queued move.
            // P2-3: only a recorded move with a recorded source is one. A
            // watcher/legacy save sends its parent without claiming a move, and
            // a follow-on save or a rehome carries the source it copied
            // (state_db::landing, state_db::conn).
            let parent_change = upload_parent_change(op, metadata);
            if metadata["parent_changed"].as_bool() == Some(true)
                && let Some(from_parent) = metadata["vacated_parent_id"].as_str()
            {
                // P1-2: the row is already at the destination here, so the DB
                // parent is never the source; only the op's recorded vacated
                // parent can name it (moves.rs).
                self.adopt_foreign_subtree(&server_file_id, Some(from_parent), op.parent_id.as_deref())
                    .await?;
            }
            if parent_change != crate::api_client::ParentChange::Unchanged {
                self.api.update_metadata(&server_file_id, None, parent_change).await?;
            }
            self.db.set_upload_resume_metadata_applied(&op.op_id)?;
        }

        let (file_key, record) = owned_key_and_record(&self.db, self.api.master_key(), &server_file_id)?;
        // The name is sealed under the item's own key; the writer takes the other copy.
        let name_key = beebeeb_core::kdf::FileKey::from_bytes(*file_key.as_bytes());

        let chunk_size = session.chunk_size_bytes as usize;
        let chunk_count = session.chunk_count as u64;
        if chunk_size == 0 || chunk_count == 0 {
            return Err(anyhow::anyhow!("upload init returned invalid chunk plan").into());
        }
        let payload_total = u64::try_from(session.payload_size).unwrap_or(0);
        // LD-0105 U1: named by the file's vault path (else the op's), never failing the upload.
        let path = self
            .db
            .get_file(local_file_id.as_str())
            .ok()
            .flatten()
            .map(|entry| entry.path)
            .or_else(|| op.target_path.clone())
            .unwrap_or_default();
        // Task 1683 slice 2: per-file bytes for the popover. Keyed by the LOCAL file
        // id (the `files` row the activity list reads).
        let progress = self.transfers.begin_named(
            local_file_id.as_str(),
            &path,
            crate::transfer_progress::Direction::Up,
            payload_total,
        );

        // The server holds every chunk (`upload_v2::resume_step`): `complete` again, with the name
        // this session stored before its first chunk. A session with no stored name was opened by a
        // build that wrote v1: its frames are v1 and its name is the row's, so it completes bare.
        let acked = session.acked_chunks.max(0) as u64;
        let completion_name = if acked == chunk_count {
            progress.update(payload_total);
            self.db.upload_completion_name(&op.op_id)?
        } else {
            // A v2 writer never continues a stream it did not start: this session is a fresh one.
            if acked != 0 {
                return Err(anyhow::anyhow!("an upload never continues a persisted session").into());
            }
            let file = std::fs::File::open(payload_path)?;
            let mut writer = beebeeb_core::chunk_stream::ChunkEncryptor::from_reader_with_file_key(
                file_key,
                &server_file_id,
                session.payload_size as u64,
                chunk_size as u64,
                file,
            )?
            .with_recorded_pin(record.format)?;
            progress.update(0);
            // Rate-limit ceiling: read once per file, not per chunk (config is on
            // disk but the file is fast to parse; at most one read per upload op).
            let upload_kbps_limit = crate::config::DesktopConfig::load_for(self.identity)
                .map(|c| c.upload_kbps_limit)
                .unwrap_or(0);

            let mut completion_name = None;
            for chunk_index in 0..chunk_count {
                let encrypted = writer
                    .next_chunk()?
                    .ok_or_else(|| anyhow::anyhow!("staged upload ended early"))?;
                if chunk_index == 0 {
                    // Frame 0 is sealed, so the version id exists: seal the name `complete` will
                    // publish and store it BEFORE the first chunk is sent. From here on a retry
                    // that finds every chunk acknowledged can complete without the writer.
                    let (name, mime) = upload_v2::completion_plain_name(
                        &name_key,
                        name_encrypted,
                        metadata_display_name(metadata, op),
                        content_type.as_deref(),
                    )?;
                    let sealed = upload_v2::seal_completion_name(
                        &self.db,
                        &server_file_id,
                        &writer,
                        &name_key,
                        record,
                        session.is_create,
                        &name,
                        mime.as_deref(),
                    )?;
                    self.db.set_upload_completion_name(&op.op_id, &sealed)?;
                    completion_name = Some(sealed);
                }
                let read = encrypted.data.len().saturating_sub(28);
                let chunk_start = std::time::Instant::now();
                self.api
                    .upload_session_chunk(&session.upload_session_id, chunk_index as u32, &encrypted.data)
                    .await?;
                self.db.set_upload_resume_acked(&op.op_id, chunk_index as i64 + 1)?;
                progress.update(crate::transfer_progress::uploaded_bytes(
                    chunk_index + 1,
                    chunk_size as u64,
                    payload_total,
                ));

                // P1 — wire-byte counter: count plaintext bytes (the user-data rate).
                self.wire.upload_bytes.fetch_add(read as u64, Ordering::Relaxed);

                // E — token-bucket pacing: if a limit is set and the chunk transferred
                // faster than the budget allows, sleep the remainder.
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
            completion_name
        };

        let completed = self
            .api
            .complete_upload_session(&session.upload_session_id, completion_name.as_deref())
            .await
            .map_err(EngineError::from)?;
        // Done only once the server holds the name with the commitment: until then the new
        // version's content opens nowhere, and the session row stays for the retry.
        if let Some(sent) = completion_name.as_deref() {
            self.publish_completion_name(&self.api, &server_file_id, &name_key, sent, &completed)
                .await?;
        }
        progress.finish();
        Ok(completed)
    }

    /// Post-`complete` best-effort work: thumbnails, staged-payload cleanup and
    /// (Windows) placeholder conversion. Never fails the upload — but the
    /// thumbnail outcome is RETURNED (task 1700) instead of being swallowed
    /// into a `tracing::warn!` that no test subscriber captures, so callers
    /// can surface it for diagnostics. Cleanup and Windows finalization stay
    /// warn-only inside.
    async fn finish_completed_upload(
        &self,
        #[cfg_attr(not(target_os = "windows"), allow(unused_variables))] op: &PendingOperation,
        server_file_id: ServerFileId<'_>,
        payload_path: &Path,
        thumbnail_content_type: Option<String>,
        file_key: &beebeeb_core::kdf::FileKey,
        #[cfg_attr(not(target_os = "windows"), allow(unused_variables))] sync_root: &Path,
    ) -> Result<(), EngineError> {
        let thumbnails = self
            .upload_thumbnails_for_plaintext_media(
                server_file_id,
                payload_path,
                thumbnail_content_type.as_deref(),
                file_key,
            )
            .await;

        #[cfg(target_os = "windows")]
        {
            crate::windows_cf::upload_finalization::retry(&self.db, sync_root);
            // A failed stamp or unlink retains its durable completed proof.
            match self.db.upload_finalizations() {
                Ok(rows) if rows.iter().any(|row| row.op_id == op.op_id) => return thumbnails,
                Err(e) => {
                    tracing::warn!(error = %e, "cannot inspect finalization journal");
                    return thumbnails;
                }
                _ => {}
            }
        }
        // On macOS the caller releases the payload once the op's removal has
        // committed (spec §8.7 S6).
        #[cfg(not(target_os = "macos"))]
        crate::staged_payload::remove_or_log(&self.db, payload_path);
        thumbnails
    }

    /// The request row's display name as core resolves it: the name key is the
    /// derived convention first, else `C` (§15). P3-6: a name that is not a
    /// safe single path component never names the copy. Falls back to the
    /// stored record's name when the canonical row cannot be read.
    async fn request_row_display_name(&self, request_id: &str) -> String {
        // The common path: the stored record's name opens under the derived
        // convention, so no network call is made.
        if let Ok(crate::owner_keys::StoredName::Name(name)) =
            crate::owner_keys::stored_name(&self.db, request_id, self.api.master_key())
        {
            if crate::owner_keys::safe_display_name(&name) {
                return name;
            }
        }
        if let Ok(row) = self.api.get_file(request_id).await
            && let Ok(keys) = self.request_row_keys(&row).await
            && crate::owner_keys::safe_display_name(&keys.name.name)
        {
            return keys.name.name;
        }
        "edited file".to_string()
    }

    /// L13/P1-1: a content edit of a 2236 request row is saved as a NEW owner
    /// item under a fresh id (its derived key), never written over the row,
    /// whose bytes are sealed under the anonymous uploader's `C`. The edited
    /// bytes are copied to a sibling path and queued as a normal create, which
    /// mints and seals under its own id. The request row is left untouched.
    async fn replan_request_edit_as_new_item(
        &self,
        request_id: &str,
        op: &PendingOperation,
        sync_root: &Path,
    ) -> Result<OpDone, EngineError> {
        let entry = self
            .db
            .get_file(request_id)?
            .ok_or_else(|| anyhow::anyhow!("request row missing from state"))?;
        // P3-8: the copy's name must come from core's resolved request-row keys:
        // `stored_name` runs the derived convention only and returns None for a
        // name sealed under `C`, which would name every un-renamed row
        // "edited file".
        let current = self.request_row_display_name(request_id).await;
        let leaf = edited_sibling_name(&current, 1);
        let mut rel = sibling_path(&entry.path, &leaf);
        // P3-8 (data loss): a free DB row is not enough. A file on disk that the
        // scan has not seen yet must never be overwritten by the copy.
        for n in 2.. {
            let candidate = local_file_path_under_sync_root(sync_root, &rel)?;
            // P3-7: `exists()` follows a symlink, so a dangling link at the
            // candidate looked free and the copy was written through it. Only a
            // path with no name at all on disk is free; anything present —
            // including a link — is skipped, and an unreadable parent fails
            // closed rather than being called free.
            let free =
                std::fs::symlink_metadata(&candidate).is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound);
            if self.db.get_file_by_path(&rel).ok().flatten().is_none() && free {
                break;
            }
            rel = sibling_path(&entry.path, &edited_sibling_name(&current, n));
        }
        let source = match op.payload_path.as_deref().map(Path::new).filter(|path| path.is_file()) {
            Some(staged) => staged.to_path_buf(),
            None => {
                // Task 2407 (core 2404): the row's own file only, never a different one its name reaches (a Windows
                // 8.3 short name), whose bytes would be copied and uploaded.
                let source = local_file_path_under_sync_root(sync_root, &entry.path)?;
                crate::name_alias::refuse_conflict_below(sync_root, &source)?;
                source
            }
        };
        let dest = local_file_path_under_sync_root(sync_root, &rel)?;
        // Task 2407 (core 2404): the copy goes beside the request row, never into a different folder that a folder's
        // name reaches (a Windows 8.3 short name). It is written to an exclusive temp sibling, then published without
        // replacing anything that appeared at its name meanwhile (core's `TempSibling`).
        crate::name_alias::refuse_conflict_below(sync_root, &dest)?;
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Zero-knowledge: core's errors here name the file; a conflict travels as `NameConflict`, the rest by kind.
        let nameless = |error: std::io::Error| match error.kind() {
            std::io::ErrorKind::AlreadyExists => {
                std::io::Error::new(std::io::ErrorKind::AlreadyExists, crate::name_alias::NameConflict)
            }
            kind => std::io::Error::from(kind),
        };
        let (temp, mut copy) = beebeeb_core::path_safety::TempSibling::create(&dest).map_err(nameless)?;
        std::io::copy(&mut std::fs::File::open(&source)?, &mut copy)?;
        drop(copy);
        temp.publish().map_err(nameless)?;
        let content_type = op
            .metadata_json
            .as_deref()
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
            .and_then(|metadata| metadata["content_type"].as_str().map(str::to_string));
        self.queue_finder_create(FinderWriteTarget {
            file_id: None,
            parent_id: entry.parent_id.clone(),
            filename: rel.rsplit('/').next().unwrap_or(&leaf).to_string(),
            rel_path: Some(rel),
            kind: crate::engine_bridge::FinderWriteItemKind::File,
            contents_path: Some(dest.to_string_lossy().into_owned()),
            content_type,
            base_version_identifier: None,
        })?;
        // The edited bytes now belong to the queued create; the original op is
        // done and the request row keeps its server row and version.
        Ok(OpDone::Remove { release: None })
    }

    /// Give up on a persisted upload session (payload changed, session gone,
    /// not self-minted, or the op exhausted its retries). For a CREATE the
    /// server row holds no completed content and is only a broken
    /// `is_uploading` duplicate, so its upload is abandoned — never trashed:
    /// it is this client's own row, not something the user deleted
    /// (best-effort: the server's stale-upload sweep still reaps it after 7
    /// days if this fails). A REPLACE targets the user's existing file and its
    /// session is only dropped locally. The resume row is always dropped so the
    /// next attempt starts a fresh session.
    async fn abandon_upload_session(&self, session: &UploadResume) -> Result<(), EngineError> {
        if session.is_create {
            match self.api.abandon_upload(&session.server_file_id).await {
                Ok(()) => tracing::info!(
                    op_id = %session.op_id,
                    file_id = %session.server_file_id,
                    "upload: abandoned the in-progress server row of an abandoned create"
                ),
                Err(e) => tracing::warn!(
                    op_id = %session.op_id,
                    file_id = %session.server_file_id,
                    error = %e,
                    "upload: could not abandon the in-progress server row; the server's stale-upload sweep reaps it"
                ),
            }
        }
        self.db.clear_upload_resume(&session.op_id)?;
        Ok(())
    }

    /// Called when an upload op has exhausted its retries: nothing will ever
    /// resume its session, so abandon it now instead of leaving the orphan
    /// visible for the server's 7-day stale-upload window.
    pub(super) async fn abandon_upload_after_give_up(&self, op: &PendingOperation) {
        if !matches!(op.kind, OperationKind::UploadVersion | OperationKind::UploadFile) {
            return;
        }
        // LD-0113: a backup op's session lives in its own table.
        match self.db.backup_upload_resume(&op.op_id) {
            Ok(Some(session)) => {
                if let Err(e) = self.abandon_backup_session(&session).await {
                    tracing::warn!(op_id = %op.op_id, error = %e, "backup: failed to abandon given-up session");
                }
                return;
            }
            Ok(None) => {}
            Err(e) => tracing::warn!(op_id = %op.op_id, error = %e, "backup: failed to read resume state"),
        }
        match self.db.get_upload_resume(&op.op_id) {
            Ok(Some(session)) => {
                if let Err(e) = self.abandon_upload_session(&session).await {
                    tracing::warn!(op_id = %op.op_id, error = %e, "upload: failed to abandon given-up session");
                }
            }
            Ok(None) => {}
            Err(e) => tracing::warn!(op_id = %op.op_id, error = %e, "upload: failed to read resume state"),
        }
    }

    pub(super) async fn upload_thumbnails_for_plaintext_media(
        &self,
        server_file_id: ServerFileId<'_>,
        payload_path: &Path,
        mime_type: Option<&str>,
        file_key: &beebeeb_core::kdf::FileKey,
    ) -> Result<(), EngineError> {
        let pin = owned_key_and_pin(&self.db, self.api.master_key(), server_file_id.as_str())?.1;
        let uploads = prepare_thumbnail_uploads_with_pin(payload_path, mime_type, file_key, pin)?;
        for upload in uploads {
            self.api
                .upload_thumbnail(
                    server_file_id.as_str(),
                    upload.variant.label(),
                    &upload.encrypted,
                    upload.blurhash.as_deref(),
                )
                .await
                .map_err(|e| anyhow::anyhow!("upload {} thumbnail: {e}", upload.variant.label()))?;
        }
        Ok(())
    }

    /// The journal row that defers a create's native stamping past the landing, or `None`
    /// when there is nothing on disk to stamp. The landing writes it in its own transaction,
    /// before any cancellable post-completion work, so the op is never gone without it.
    #[cfg(target_os = "windows")]
    fn defer_local_upload_finalization(
        &self,
        op: &PendingOperation,
        server_file_id: ServerFileId<'_>,
        sync_root: &Path,
        payload_path: &Path,
    ) -> Result<Option<crate::state_db::UploadFinalization>, EngineError> {
        let is_create = op
            .metadata_json
            .as_deref()
            .and_then(|m| serde_json::from_str::<serde_json::Value>(m).ok())
            .is_some_and(|m| m["operation"].as_str() == Some("create_file"));
        if !is_create {
            return Ok(None);
        }
        let Some(queued_path) = op.target_path.as_deref() else {
            return Ok(None);
        };
        // Task 2204 S0.2 (review-128 F4): renamed since it was queued, the file is at its row's
        // path; the landing writes the journal there too.
        let renamed_path = match op.file_id.as_deref() {
            Some(id) if self.db.has_queued_local_rename(id)? => self.db.get_file(id)?.map(|row| row.path),
            _ => None,
        };
        let target_path = renamed_path.as_deref().unwrap_or(queued_path);
        let on_disk = local_file_path_under_sync_root(sync_root, target_path)?;
        if !on_disk.is_file() {
            return Ok(None);
        }
        Ok(Some(crate::state_db::UploadFinalization {
            op_id: op.op_id.clone(),
            local_file_id: op.file_id.as_deref().unwrap_or(server_file_id.as_str()).to_string(),
            server_file_id: server_file_id.as_str().to_string(),
            target_path: target_path.to_string(),
            payload_path: payload_path.to_string_lossy().into_owned(),
            stamped: false,
            review: None,
        }))
    }
}

/// The leaf of the save-as-new item a request row's edit becomes: the row's
/// own name, marked as an edited copy. `n` makes a second copy distinct.
fn edited_sibling_name(name: &str, n: u32) -> String {
    let (stem, extension) = match name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() && !extension.is_empty() => (stem, Some(extension)),
        _ => (name, None),
    };
    let suffix = if n <= 1 {
        " (edited)".to_string()
    } else {
        format!(" (edited {n})")
    };
    match extension {
        Some(extension) => format!("{stem}{suffix}.{extension}"),
        None => format!("{stem}{suffix}"),
    }
}

/// `leaf` beside the row at `path`: the same parent directory, a new name.
fn sibling_path(path: &str, leaf: &str) -> String {
    match path.rsplit_once('/') {
        Some((dir, _)) if !dir.is_empty() => format!("{dir}/{leaf}"),
        _ => leaf.to_string(),
    }
}
