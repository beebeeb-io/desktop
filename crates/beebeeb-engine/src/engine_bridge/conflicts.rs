//! `conflicts` — the `EngineBridge` impl for conflict resolution and content previews, split out of `engine_bridge.rs`.

use super::*;

impl EngineBridge {
    /// Fetch the encrypted server thumbnail for `file_id`, decrypt it **in
    /// memory only**, and return the decoded image bytes (WebP / JPEG / PNG)
    /// inside a [`Zeroizing`] wrapper.
    ///
    /// This is the shared read path for OS thumbnail consumers. It mirrors
    /// [`Self::do_hydrate`]'s per-file-key derivation but hits
    /// `GET /files/:id/thumbnail/{variant}` instead of the chunk range and reuses
    /// the **existing** encrypted thumbnail the original uploading client
    /// generated (a downscaled still for images, a poster frame for video). OS
    /// integrations never regenerate a thumbnail and never decode the original
    /// media just to preview it — they decrypt this small blob.
    ///
    /// Core's purpose-specific thumbnail reader handles legacy raw-key blobs and
    /// subkey thumbnails. File content always uses the separate stream reader.
    ///
    /// ## Zero-knowledge
    ///
    /// The decrypted bytes live ONLY in the returned `Zeroizing<Vec<u8>>`, which
    /// is wiped on drop (normal return, `?`, or panic unwind). Callers decide how
    /// to consume the decoded image bytes: Windows decodes straight to an
    /// in-memory `HBITMAP`; Linux writes the freedesktop.org cache PNG derivative
    /// required by file managers.
    ///
    /// `variant` is `small` / `medium` / `large`. Any error (no thumbnail for
    /// this file, network failure, decrypt failure) bubbles as `Err` so the
    /// caller can fall back to the file-type icon — it is never reported as a
    /// success with empty bytes.
    pub async fn fetch_thumbnail_to_memory(&self, file_id: &str, variant: &str) -> anyhow::Result<Zeroizing<Vec<u8>>> {
        let _: uuid::Uuid = file_id.parse()?;
        let blob = self.api.download_thumbnail(file_id, variant).await?;
        let key = self.file_key_for_download(file_id).await?;
        Ok(Zeroizing::new(beebeeb_core::encrypt::decrypt_thumbnail(&key, &blob)?))
    }

    /// Apply a "Keep Mine" resolution: stage the current local bytes and upload
    /// them as a new server version using the same chunked upload path as normal
    /// Finder writes. On success, [`Self::upload_version`] applies the regular
    /// upload completion bookkeeping (status, version, object version, size, and
    /// remote timestamp). On failure, the row is left conflicted and the error is
    /// returned to the caller so the UI does not report a false resolution.
    ///
    /// `sync_root` is supplied by the caller because the bridge itself doesn't
    /// know it (the runner owns that).
    pub async fn resolve_keep_mine(&self, file_id: &str, sync_root: &Path) -> anyhow::Result<()> {
        let entry = self
            .db
            .get_file(file_id)?
            .ok_or_else(|| anyhow::anyhow!("no state.db row for {file_id}"))?;
        if entry.is_dir() {
            return Err(anyhow::anyhow!("Keep Mine upload requires a file row: {file_id}").into());
        }

        let shared_contract = self.ensure_item_allows_shared_write(file_id, "keep mine")?;
        let contract = self.db.get_file_contract_state(file_id)?;
        let parent_id = contract.as_ref().and_then(|contract| contract.parent_id.clone());
        let file_name = display_name_for_path(&entry.path);
        let content_type = contract
            .as_ref()
            .and_then(|contract| contract.content_type.clone())
            .or_else(|| beebeeb_core::media::guess_mime_type(&file_name).map(str::to_string));
        let name_encrypted = owned_metadata_for_name(
            &self.db,
            self.api.master_key(),
            file_id,
            &file_name,
            content_type.as_deref(),
        )?;

        let local_path = local_file_path_under_sync_root(sync_root, &entry.path)?;
        // Task 2407 (core 2404): through a name that reaches a different file (a Windows 8.3 short name), "mine" is
        // that other file: its bytes are never uploaded as this file's version. The row stays in conflict.
        crate::name_alias::refuse_conflict_below(sync_root, &local_path)?;
        let staged =
            crate::staged_payload::StagedPayload::copy(self.db.clone(), &local_path, self.finder_staging_root()?)?;
        let staged_path = staged.path().to_string();
        let staged_size = std::fs::metadata(&staged_path).map(|m| m.len()).unwrap_or(0);

        let mut metadata = serde_json::json!({
            "operation": Op::UploadVersion.as_str(),
            "name_encrypted": name_encrypted,
            "content_type": content_type,
            "size_bytes": staged_size,
            "uploaded_by": "authenticated_desktop_user",
        });
        apply_shared_context(&mut metadata, shared_contract.as_ref());

        let now = now_secs();
        let op = PendingOperation {
            op_id: uuid::Uuid::new_v4().to_string(),
            kind: OperationKind::UploadVersion,
            file_id: Some(file_id.to_string()),
            parent_id,
            target_path: Some(entry.path.clone()),
            metadata_json: Some(serde_json::to_string(&metadata)?),
            payload_path: Some(staged_path.clone()),
            // Keep Mine is an explicit conflict override. Passing the stale base
            // that caused the conflict would make the server reject the upload.
            base_version: None,
            base_object_version_id: None,
            attempts: 0,
            max_attempts: 1,
            next_retry_at: now,
            last_error: None,
            backup_source_key: None,
            created_at: now,
            updated_at: now,
        };

        let mut post_complete_errors = Vec::new();
        let released = match self
            .upload_version(&op, None, sync_root, &mut post_complete_errors)
            .await
        {
            Ok(OpDone::Remove { release } | OpDone::Removed { release }) => release,
            // Only a backup op is dropped; Keep Mine's op is a sync-folder upload.
            Ok(OpDone::Dropped) => return Err(anyhow::anyhow!("Keep Mine upload was dropped").into()),
            Err(e) => {
                // One-shot op (never queued): nothing will resume its session.
                self.abandon_upload_after_give_up(&op).await;
                return Err(anyhow::anyhow!("Keep Mine upload failed: {e}").into());
            }
        };
        // macOS: the upload leaves its staged copy for the caller to release. This op
        // was never queued, so there is no removal to wait for.
        if let Some(path) = released.as_deref() {
            crate::staged_payload::remove_or_log(&self.db, Path::new(path));
        }
        // Task 1700: thumbnail failures never fail the resolution, but they
        // must not vanish silently either.
        for error in &post_complete_errors {
            tracing::warn!(file_id = %file_id, error = %error, "conflict keep-mine upload: post-complete thumbnail work skipped");
        }

        tracing::info!(file_id = %file_id, "conflict resolved: keep mine uploaded local version");
        Ok(())
    }

    /// Apply a "Keep Theirs" resolution: download the remote version
    /// and overwrite the local file at `<sync_root>/<entry.path>`.
    /// Status ends in `Local`; `remote_updated_at` is anchored to
    /// "now" so the next tick treats the file as freshly synced.
    ///
    /// Reuses [`Self::hydrate_file`], which already handles status
    /// transitions and Error-on-failure rollback. The extra
    /// `remote_updated_at` bump after a successful hydrate prevents
    /// the next tick from re-flagging a conflict if the user's old
    /// `content_hash` is still on the row (it isn't anymore — the row
    /// was already in Conflict — but we belt-and-brace anchor anyway).
    pub async fn resolve_keep_theirs(&self, file_id: &str, sync_root: &Path) -> anyhow::Result<()> {
        let mut entry = self
            .db
            .get_file(file_id)?
            .ok_or_else(|| anyhow::anyhow!("no state.db row for {file_id}"))?;
        let dest = local_file_path_under_sync_root(sync_root, &entry.path)?;
        // hydrate_file flips Conflict → Downloading → Local on success
        // (or Error on failure). We don't care about the intermediate
        // state for this path.
        self.hydrate_file(file_id, &dest, &[sync_root]).await?;
        let now_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        entry.status = FileStatus::Local;
        entry.remote_updated_at = now_secs;
        entry.modified_at = now_secs;
        self.db.upsert_file(&entry)?;
        tracing::info!(file_id = %file_id, dest = %dest.display(), "conflict resolved: keep theirs");
        Ok(())
    }

    /// Apply a "Keep Both" resolution: rename the local copy to a
    /// device-suffixed conflict filename, hydrate the remote into the
    /// original path, flip status back to `Local`. Called by Task 13's
    /// auto-resolution timer in [`crate::runner`] and (eventually) by
    /// the user-driven `resolve_conflict` IPC when the user picks
    /// "Keep Both" from the conflict window.
    ///
    /// Filesystem ops are done in this order so a partial failure
    /// leaves recoverable state:
    ///
    ///   1. Rename local → conflict copy (cheap, instantaneous)
    ///   2. Hydrate remote into the original path (network — may
    ///      fail; if it does, the user still has both their original
    ///      *and* the conflict copy on disk, so no data is lost; we
    ///      flip status to `Error` so the next tick retries).
    ///
    /// `sync_root` is supplied by the caller because the bridge
    /// itself doesn't know it (the runner owns that).
    pub async fn auto_resolve_keep_both(&self, sync_root: &Path, entry: &FileEntry) -> anyhow::Result<String> {
        let original = local_file_path_under_sync_root(sync_root, &entry.path)?;
        // Task 2407 (core 2404): through a name that reaches a different entry (a Windows 8.3 short name), "the local
        // copy" is another file: it is never renamed or written over, and the row stays in conflict.
        crate::name_alias::refuse_conflict_below(sync_root, &original)?;

        // If the local file no longer exists on disk (user deleted it
        // outside the daemon), Keep Both collapses to Keep Remote.
        if !original.exists() {
            self.hydrate_file(&entry.file_id, &original, &[sync_root]).await?;
            self.db.set_status(&entry.file_id, FileStatus::Local)?;
            return Ok(entry.path.clone());
        }

        let host = hostname::get()
            .map(|h| h.to_string_lossy().to_string())
            .unwrap_or_else(|_| "device".into());
        let date = chrono::Utc::now().format("%Y-%m-%d");
        let stem = original.file_stem().and_then(|s| s.to_str()).unwrap_or("file");
        let ext = original
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| format!(".{e}"))
            .unwrap_or_default();
        // Task 2407: the copy never replaces what holds its name (an earlier conflict copy of the same day): the next
        // free name, `… 2`, `… 3`, is taken instead.
        let mut conflict_name = format!("{stem} (conflict - {host} - {date}){ext}");
        for n in 2u32.. {
            let conflict_path = original.with_file_name(&conflict_name);
            match rename_without_replacing(&original, &conflict_path) {
                Ok(()) => break,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && n <= 100 => {
                    conflict_name = format!("{stem} (conflict - {host} - {date}) {n}{ext}");
                }
                Err(e) => {
                    return Err(anyhow::anyhow!(
                        "rename {} -> {}: {e}",
                        original.display(),
                        conflict_path.display()
                    ));
                }
            }
        }

        // Hydrate remote into the now-vacant original path. On failure
        // we mark Error rather than Conflict — the conflict copy is
        // safe on disk, the remote download just needs a retry.
        if let Err(e) = self.hydrate_file(&entry.file_id, &original, &[sync_root]).await {
            self.db.set_status(&entry.file_id, FileStatus::Error)?;
            return Err(e);
        }
        self.db.set_status(&entry.file_id, FileStatus::Local)?;
        Ok(conflict_name)
    }

    /// Read-only content preview for the conflict-resolution window (task
    /// 1546 finding 2): reads the LOCAL file straight off disk and
    /// downloads+decrypts the CURRENT REMOTE version, without touching
    /// state.db — unlike [`Self::hydrate_file`] / [`Self::hydrate_file_to_memory`],
    /// which both flip the row's status to `Downloading`/`Local` and record a
    /// cache entry. The row is mid-conflict and neither side has been chosen
    /// yet, so daemon bookkeeping must not move just because the user opened
    /// the window — that's why this calls the private `do_hydrate` directly
    /// instead of either public hydrate wrapper.
    ///
    /// Replaces `ConflictWindow.tsx`'s previous hardcoded placeholder text
    /// ("Content from this device…" / "Content from other device…"), which
    /// its own doc-comment admitted was fake for every conflict.
    ///
    /// Task 1546 Codex round 2, finding 3: textness is decided HERE, from
    /// the file's own path via [`is_text_file`], never taken from a
    /// caller-supplied flag — the VersionCenter-initiated open always passed
    /// a hardcoded `isText: false` (only the daemon's auto-open path derived
    /// it correctly from the filename), which made every manually-reviewed
    /// text conflict render as binary and silently discard both text bodies.
    pub async fn conflict_content_preview(
        &self,
        file_id: &str,
        sync_root: &Path,
    ) -> anyhow::Result<ConflictContentPreview> {
        let entry = self
            .db
            .get_file(file_id)?
            .ok_or_else(|| anyhow::anyhow!("no state.db row for {file_id}"))?;
        let kind = ContentKind::from_flag(is_text_file(&entry.path));

        let local = self.local_content_preview(sync_root, &entry.path, kind);
        let (remote, _) = self.remote_content_preview(file_id, kind).await;

        Ok(ConflictContentPreview {
            is_text: kind.is_text(),
            local,
            remote,
            version: None,
        })
    }

    /// Task 2190 (plan §3.4): both sides of a parked conflict. The Mac's side is the parked write's
    /// staged copy, read with the same bounds as [`Self::local_content_preview`]; the other side is
    /// the server's current version. Read-only.
    pub async fn parked_conflict_preview(&self, op_id: &str) -> anyhow::Result<ConflictContentPreview> {
        let op = self
            .db
            .get_operation(op_id)?
            .ok_or_else(|| anyhow::anyhow!("no queued write {op_id}"))?;
        let file_id = op
            .file_id
            .clone()
            .ok_or_else(|| anyhow::anyhow!("the write names no file"))?;
        let entry = self
            .db
            .get_file(&file_id)?
            .ok_or_else(|| anyhow::anyhow!("no state.db row for {file_id}"))?;
        let kind = ContentKind::from_flag(is_text_file(&entry.path));
        let payload = PathBuf::from(
            op.payload_path
                .ok_or_else(|| anyhow::anyhow!("the write has no staged copy"))?,
        );
        let local = match (payload.parent(), payload.file_name().and_then(|name| name.to_str())) {
            (Some(dir), Some(name)) => self.local_content_preview(dir, name, kind),
            _ => ConflictContentSide {
                size_bytes: None,
                text: None,
                unavailable_reason: Some("Couldn't locate this Mac's copy.".to_string()),
            },
        };
        // Review-169 #1 and F1: the version the person decides against is the version of the bytes
        // shown, from the metadata the download ran on (after a restart, the fresh one). Never the
        // mirror read after the download: the pull can raise it to a version nobody has seen while
        // the download runs. A file whose metadata names no version (legacy, or the fetch failed)
        // gets the mirror as it was BEFORE the fetch, which is never newer than the bytes: at worst
        // the decision is `Moved`, or the write is sent on an older base and the server refuses it.
        let before = self
            .db
            .get_file_contract_state(&file_id)?
            .map(|contract| contract.current_version);
        let (remote, shown) = self.remote_content_preview(&file_id, kind).await;
        let version = shown.or(before);
        Ok(ConflictContentPreview {
            is_text: kind.is_text(),
            local,
            remote,
            version,
        })
    }

    /// Task 2190 (plan §3.2): the person's decision on a parked conflict, against `shown_version`,
    /// the version the conflict window showed. Every choice re-checks the op inside the transaction
    /// that acts on it (decidable, and its file still at that version), and returns `Moved`, with
    /// nothing changed, when it moved since.
    /// - `Local`: the same op is rebased on the server's version and sent again.
    /// - `Remote` (D1): `keep_copy` copies the Mac's bytes into the "Changes not sent" folder FIRST.
    ///   If it fails, nothing changed. Then the op goes and the system reads the server's version.
    ///   The staged copy is released only after that commit.
    /// - `Both` (D4): the Mac's bytes are queued as a new file named [`keep_both_name`] beside the
    ///   original, and the op is retired, in ONE transaction (review-169 #3): if it moved, nothing
    ///   is queued.
    pub fn resolve_parked(
        &self,
        op_id: &str,
        choice: ParkedChoice,
        shown_version: Option<i64>,
        keep_copy: &mut dyn FnMut(&Path, &str) -> anyhow::Result<PathBuf>,
        device: &str,
        hhmm: &str,
    ) -> anyhow::Result<ParkedResolution> {
        // Review-169 #1: the person decides against the version they were shown. Without it, or
        // with the file past it, nothing changes.
        let Some(shown) = shown_version else {
            return Ok(ParkedResolution::Moved);
        };
        if choice == ParkedChoice::Local {
            return Ok(match self.db.rebase_parked(op_id, shown, now_secs())? {
                Some(version) => ParkedResolution::Rebased { version },
                None => ParkedResolution::Moved,
            });
        }
        if !self.db.is_decidable_parked_at(op_id, shown)? {
            return Ok(ParkedResolution::Moved);
        }
        let op = self
            .db
            .get_operation(op_id)?
            .ok_or_else(|| anyhow::anyhow!("no queued write {op_id}"))?;
        let file_id = op
            .file_id
            .clone()
            .ok_or_else(|| anyhow::anyhow!("the write names no file"))?;
        let row = self
            .db
            .get_file(&file_id)?
            .ok_or_else(|| anyhow::anyhow!("no state.db row for {file_id}"))?;
        let payload = op
            .payload_path
            .clone()
            .ok_or_else(|| anyhow::anyhow!("the parked write has no staged copy"))?;
        let (done, released) = match choice {
            ParkedChoice::Remote => {
                // D1: the copy first; a failure changes nothing.
                let kept = keep_copy(Path::new(&payload), &row.path)?;
                match self.db.drop_parked(op_id, shown)? {
                    Some(released) => (
                        ParkedResolution::KeptOther {
                            kept_copy: kept.to_string_lossy().into_owned(),
                        },
                        released,
                    ),
                    // The kept copy stays (nothing is lost) and stays listed.
                    None => return Ok(ParkedResolution::Moved),
                }
            }
            ParkedChoice::Both => {
                let contract = self
                    .db
                    .get_file_contract_state(&file_id)?
                    .ok_or_else(|| anyhow::anyhow!("no contract for {file_id}"))?;
                let (parent_rel, leaf) = match row.path.rsplit_once('/') {
                    Some((parent, leaf)) => (Some(parent), leaf),
                    None => (None, row.path.as_str()),
                };
                let taken = self.db.child_names_lowercase(contract.parent_id.as_deref())?;
                let name = keep_both_name(leaf, device, hhmm, &taken);
                let target = FinderWriteTarget {
                    file_id: None,
                    parent_id: contract.parent_id.clone(),
                    filename: name.clone(),
                    rel_path: Some(match parent_rel {
                        Some(parent) => format!("{parent}/{name}"),
                        None => name.clone(),
                    }),
                    kind: FinderWriteItemKind::File,
                    contents_path: Some(payload.clone()),
                    content_type: contract.content_type.clone(),
                    base_version_identifier: None,
                };
                // Review-169 #3: the copy is queued and the parked write retired in ONE transaction;
                // a write that moved queues nothing.
                let retire = crate::state_db::RetireParked {
                    op_id,
                    shown_version: shown,
                };
                let Some((written, released)) = self.queue_file_provider_create_retiring(
                    target,
                    Some(&std::fs::File::open(&payload)?),
                    Some(retire),
                )?
                else {
                    return Ok(ParkedResolution::Moved);
                };
                match written.outcome {
                    FinderWriteOutcome::Queued {
                        file_id: Some(new_id), ..
                    } => (ParkedResolution::KeptBoth { file_id: new_id, name }, released),
                    other => {
                        return Err(anyhow::anyhow!("the copy could not be queued as a new file: {other:?}").into());
                    }
                }
            }
            ParkedChoice::Local => unreachable!("handled above"),
        };
        // The staged copy was released in the decision's transaction; unlink it now (S6).
        if let Some(released) = released {
            crate::staged_payload::remove_or_log(&self.db, Path::new(&released));
        }
        Ok(done)
    }

    /// Local half of [`Self::conflict_content_preview`] (task 1546 Codex
    /// round 2, finding 1): stats the file to learn its size WITHOUT reading
    /// it, and only reads bytes at all when a text preview applies AND the
    /// file is within [`CONFLICT_PREVIEW_TEXT_MAX_BYTES`] — bounded to one
    /// byte past the cap via [`std::io::Read::take`] so a race where the
    /// file grows between the stat and the read can only ever push the
    /// result to "too large," never load an oversized buffer. Conflict
    /// windows open automatically, so a multi-gigabyte local file must never
    /// be pulled fully into memory just to report its size.
    fn local_content_preview(&self, sync_root: &Path, entry_path: &str, kind: ContentKind) -> ConflictContentSide {
        let local_path = match local_file_path_under_sync_root(sync_root, entry_path) {
            Ok(p) => p,
            Err(e) => {
                return ConflictContentSide {
                    size_bytes: None,
                    text: None,
                    unavailable_reason: Some(format!("Couldn't locate the local file: {e}")),
                };
            }
        };
        // Task 2407 (core 2404): a name that reaches a different file (a Windows 8.3 short name) never shows that
        // file's content as this one's.
        if !matches!(crate::name_alias::conflict_below(sync_root, &local_path), Ok(None)) {
            return ConflictContentSide {
                size_bytes: None,
                text: None,
                unavailable_reason: Some(
                    "On this PC the file's name leads to a different file, so it is not shown.".to_string(),
                ),
            };
        }

        let size_bytes = match std::fs::metadata(&local_path) {
            Ok(m) => m.len(),
            Err(e) => {
                return ConflictContentSide {
                    size_bytes: None,
                    text: None,
                    unavailable_reason: Some(format!("Couldn't read the local file: {e}")),
                };
            }
        };

        if kind == ContentKind::Binary {
            // Binary preview only ever shows size — the bytes are never read.
            return ConflictContentSide {
                size_bytes: Some(size_bytes),
                text: None,
                unavailable_reason: None,
            };
        }
        if size_bytes > CONFLICT_PREVIEW_TEXT_MAX_BYTES as u64 {
            return ConflictContentSide {
                size_bytes: Some(size_bytes),
                text: None,
                unavailable_reason: Some(format!("File is too large to preview, {size_bytes} bytes")),
            };
        }

        let file = match std::fs::File::open(&local_path) {
            Ok(f) => f,
            Err(e) => {
                return ConflictContentSide {
                    size_bytes: Some(size_bytes),
                    text: None,
                    unavailable_reason: Some(format!("Couldn't read the local file: {e}")),
                };
            }
        };
        let mut bytes = Vec::with_capacity((size_bytes as usize).min(CONFLICT_PREVIEW_TEXT_MAX_BYTES) + 1);
        // `take(LIMIT + 1)` bounds the read itself — never `std::fs::read`
        // (unbounded) — so even a TOCTOU race where the file grows after the
        // `metadata()` call above can produce at most LIMIT+1 bytes.
        match file
            .take(CONFLICT_PREVIEW_TEXT_MAX_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
        {
            Ok(_) => content_side_from_bytes(bytes, kind),
            Err(e) => ConflictContentSide {
                size_bytes: Some(size_bytes),
                text: None,
                unavailable_reason: Some(format!("Couldn't read the local file: {e}")),
            },
        }
    }

    /// Remote half of [`Self::conflict_content_preview`] (task 1546 Codex
    /// round 2, finding 1): fetches file metadata — one small JSON response —
    /// to learn the remote size BEFORE deciding whether to download
    /// anything. [`Self::do_hydrate_with_meta`] (which downloads and
    /// decrypts every chunk) is called only when BOTH a text preview applies
    /// AND the metadata size is within [`CONFLICT_PREVIEW_TEXT_MAX_BYTES`] —
    /// a multi-gigabyte conflict, text or binary, therefore never costs
    /// bandwidth or a full decrypt just to be previewed. Reuses the SAME
    /// metadata fetch `do_hydrate_with_meta` needs instead of letting
    /// `do_hydrate` re-fetch it, so the small-file path costs exactly the
    /// metadata GET + the chunk GETs it always cost.
    ///
    /// Also returns the server version the side shows: the `download_version.version_number` of the
    /// metadata the attempt ran on (task 2190, review-169 F1), `None` when no metadata named one.
    pub(super) async fn remote_content_preview(
        &self,
        file_id: &str,
        kind: ContentKind,
    ) -> (ConflictContentSide, Option<i64>) {
        let unavailable = |reason: String| ConflictContentSide {
            size_bytes: None,
            text: None,
            unavailable_reason: Some(reason),
        };
        if let Err(e) = file_id.parse::<uuid::Uuid>() {
            return (unavailable(format!("invalid file_id (not a UUID): {e}")), None);
        }
        let meta = match self.api.get_file(file_id).await {
            Ok(m) => m,
            Err(e) => {
                return (
                    unavailable(format!("Couldn't download the other device's version: {e}")),
                    None,
                );
            }
        };
        // Tasks 1643 + 2173: a version that goes away mid-download restarts once from fresh metadata, as
        // `do_hydrate` does, and the size is checked again on the new version.
        self.with_one_restart(file_id, meta, |meta| async move {
            let version = meta["download_version"]["version_number"].as_i64();
            self.preview_attempt(file_id, kind, &meta)
                .await
                .map(|side| (side, version))
        })
        .await
        .unwrap_or_else(|e| {
            (
                unavailable(format!("Couldn't download the other device's version: {e}")),
                None,
            )
        })
    }

    /// One pass of [`Self::remote_content_preview`] on `meta`. `Err` only for what the restart handles (the version
    /// went away) or what ends it; every other failure is the side's own "unavailable" line.
    async fn preview_attempt(
        &self,
        file_id: &str,
        kind: ContentKind,
        meta: &serde_json::Value,
    ) -> Result<ConflictContentSide, EngineError> {
        // Task 1643: the size of the version that would be downloaded. While a replace runs, the top-level
        // `size_bytes` can be another version's.
        let size_bytes = match crate::download_pin::PinnedVersion::from_metadata(meta)? {
            Some(pin) => Some(pin.plaintext_size_bytes),
            None => meta.get("size_bytes").and_then(|v| v.as_u64()),
        };

        if kind == ContentKind::Binary {
            // Binary preview only ever shows size — never download the bytes.
            return Ok(ConflictContentSide {
                size_bytes,
                text: None,
                unavailable_reason: None,
            });
        }
        let within_limit = matches!(size_bytes, Some(s) if s <= CONFLICT_PREVIEW_TEXT_MAX_BYTES as u64);
        if !within_limit {
            return Ok(ConflictContentSide {
                size_bytes,
                text: None,
                unavailable_reason: Some(match size_bytes {
                    Some(s) => format!("File is too large to preview, {s} bytes"),
                    None => "Couldn't determine the other device's file size".to_string(),
                }),
            });
        }

        // Between the metadata and the chunks: where another device's save and the pull's mirror of it
        // can land while the preview downloads (review-169 F1's test).
        self.seam("preview:before_download");
        match self.do_hydrate_with_meta(file_id, meta, None).await {
            Ok(mut bytes) => {
                // Move the plaintext out instead of `bytes.to_vec()` — a
                // clone would briefly hold two live copies of the remote
                // plaintext in memory. `content_side_from_bytes` consumes
                // the Vec by value (no further copy), and the now-empty
                // `bytes` is zeroized below for defense in depth.
                let side = content_side_from_bytes(std::mem::take(&mut *bytes), kind);
                bytes.zeroize();
                Ok(side)
            }
            Err(e) if e.is_caused_by::<crate::download_pin::VersionGone>() => Err(e),
            Err(e) => Ok(ConflictContentSide {
                size_bytes,
                text: None,
                unavailable_reason: Some(format!("Couldn't download the other device's version: {e}")),
            }),
        }
    }
}

/// Task 2407: move `from` to `to`, never over anything `to` reaches (`AlreadyExists`). Windows: [`move_no_replace`]
/// (`MoveFileExW` without `MOVEFILE_REPLACE_EXISTING`, which also refuses another entry's 8.3 short name). Elsewhere: a
/// hard link, then the old name is removed ([`super::fs::remove_after_link`]: when that fails, the link is removed again
/// and its error returned). Only a volume without hard links falls back: a taken name is refused, then a rename, as
/// before this task.
#[cfg(target_os = "windows")]
fn rename_without_replacing(from: &Path, to: &Path) -> std::io::Result<()> {
    move_no_replace(from, to)
}

#[cfg(not(target_os = "windows"))]
fn rename_without_replacing(from: &Path, to: &Path) -> std::io::Result<()> {
    match std::fs::hard_link(from, to) {
        Ok(()) => super::fs::remove_after_link(from, to),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Err(error),
        Err(_) => {
            if std::fs::symlink_metadata(to).is_ok() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::AlreadyExists,
                    "the conflict copy's name is taken",
                ));
            }
            std::fs::rename(from, to)
        }
    }
}
