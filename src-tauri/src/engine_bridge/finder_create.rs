//! `finder_create` — the `EngineBridge` impl for Finder/File Provider creates and their reads (hydrate, thumbnails), split out of `engine_bridge.rs`.

use super::*;

impl EngineBridge {
    pub fn queue_finder_create(&self, target: FinderWriteTarget) -> anyhow::Result<FinderWriteOutcome> {
        self.queue_finder_create_from(target, None)
    }

    /// [`Self::queue_finder_create`], reading a file's contents from
    /// `contents` when given: a file the caller already opened and checked
    /// (the macOS handed-over copy, see `ipc_socket::open_staged_contents`).
    /// `target.contents_path` is then only the journal's label for it, and is
    /// never opened.
    pub fn queue_finder_create_from(
        &self,
        target: FinderWriteTarget,
        contents: Option<&std::fs::File>,
    ) -> anyhow::Result<FinderWriteOutcome> {
        let Some(start) = self.start_finder_create(&target)? else {
            return Ok(ignored_finder_item(&target.filename));
        };
        match target.kind {
            FinderWriteItemKind::Folder => {
                let FinderCreateStart {
                    parent_contract,
                    file_id,
                    rel_path,
                } = start;
                let metadata = owned_metadata_for_name(&self.db, self.api.master_key(), &file_id, &target.filename, None)?;
                // Spec §7.2 (final review): an extension's create names its folder by parent
                // only, so the row takes the full relative path under that parent, as every
                // Finder writer does. A caller's own relative path (the watcher's) is kept.
                let row_path = match &target.rel_path {
                    Some(_) => rel_path.clone(),
                    None => self
                        .db
                        .relative_path_under(target.parent_id.as_deref(), &target.filename)?,
                };
                // Write the local folder row + contract synchronously, the same
                // way the File branch writes its `Uploading` row, so a nested
                // child dispatched later in the SAME scan resolves this folder as
                // its parent (`resolve_parent_id_for` reads `files.item_kind`)
                // without waiting for a server `/sync` round-trip. `Local` =
                // present on disk, server CreateFolder op pending in the queue.
                // Spec §7.2 (final review): the row and its parent in one transaction, so no
                // listing sees the new folder without its parent.
                let entry = FileEntry {
                    file_id: file_id.clone(),
                    path: row_path,
                    status: FileStatus::Local,
                    size_bytes: 0,
                    modified_at: now_secs(),
                    content_hash: None,
                    remote_updated_at: 0,
                    // Not persisted by upsert_file; the contract update owns these.
                    parent_id: None,
                    item_kind: ItemKind::Folder,
                };
                let mut payload = serde_json::json!({
                    "operation": Op::CreateFolder.as_str(),
                    "name_encrypted": metadata,
                });
                apply_shared_context(&mut payload, parent_contract.as_ref());
                self.enqueue_finder_operation(
                    OperationKind::CreateFolder,
                    Some(file_id),
                    target.parent_id.clone(),
                    Some(rel_path),
                    payload,
                    None,
                    None,
                    None,
                    // The row, its contract (`Folder`, under its parent) and the op in one transaction, so a
                    // nested child dispatched later still resolves this folder as its parent, and a refused
                    // create leaves no row.
                    Some(crate::state_db::FinderFolderRow {
                        entry: &entry,
                        parent_id: target.parent_id.as_deref(),
                    }),
                )
            }
            FinderWriteItemKind::File => {
                let PreparedFinderCreate {
                    file_id,
                    rel_path,
                    staged,
                    staged_path,
                    row,
                    payload,
                    ..
                } = self.prepare_finder_create(&target, contents, start)?;
                // Spec §7.2 (final review): the row and its parent in one transaction, as the
                // landing will write them, so no listing sees a new nested file without its
                // parent while it is queued.
                self.db.upsert_file_with_contract(&row, |contract| {
                    contract.item_kind = ItemKind::File;
                    contract.parent_id = target.parent_id.clone();
                })?;
                let outcome = self.enqueue_finder_operation(
                    OperationKind::UploadVersion,
                    Some(file_id),
                    target.parent_id,
                    // target_path = the FULL relative key, so
                    // defer_local_upload_finalization joins sync_root + this
                    // and finds the nested file on disk to re-stamp it in-sync.
                    Some(rel_path),
                    payload,
                    Some(staged_path),
                    None,
                    None,
                    None,
                )?;
                staged.retain();
                Ok(outcome)
            }
        }
    }

    /// What every Finder create checks first, and the ids it gets. `None`: the name is
    /// one Finder writes for itself, which is ignored.
    fn start_finder_create(&self, target: &FinderWriteTarget) -> anyhow::Result<Option<FinderCreateStart>> {
        // Task 1538 Codex P1 (PR #49, lib.rs:1087 thread): refuse a brand-new
        // enqueue once this engine has been asked to stop. Both the Windows
        // upload watcher (`watcher::spawn`'s debounce/scan loops) and the
        // macOS/Linux File Provider extension (via `ipc_socket::handle_connection`'s
        // `QueueFinderCreate` dispatch) call this directly, so this single
        // check covers "the watcher" on every platform without needing a
        // separate flag check duplicated in each caller.
        if self.is_stopping() {
            anyhow::bail!("engine is stopping; refusing to enqueue a new local write");
        }
        if is_ignored_finder_name(&target.filename) {
            return Ok(None);
        }

        let parent_contract = self.ensure_shared_parent_allows_write(target.parent_id.as_deref())?;
        // Honour a caller-supplied id (task 0811 folder scaffolding writes the
        // local row under a known id and needs the server CreateFolder to reuse
        // it — the server accepts a client `folder_id`). The file path and the
        // legacy IPC always pass `None`, getting a fresh uuid as before.
        let file_id = target
            .file_id
            .clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        // Full server-relative key for this item: the FULL nested path when the
        // caller supplied one (`docs/a.txt`), else the leaf filename (top-level /
        // legacy IPC). This is stored as the row's `path` AND threaded as the
        // upload's `target_path`, so filter-3, finalize, and delete all key off
        // the same path. `clone` because `filename` is still needed for the
        // server `name` (always the leaf).
        let rel_path = target.rel_path.clone().unwrap_or_else(|| target.filename.clone());
        Ok(Some(FinderCreateStart {
            parent_contract,
            file_id,
            rel_path,
        }))
    }

    /// A Finder file create, staged and described: the daemon's own copy of the bytes,
    /// the row it inserts and the upload's metadata. Shared by both create entry points.
    fn prepare_finder_create(
        &self,
        target: &FinderWriteTarget,
        contents: Option<&std::fs::File>,
        start: FinderCreateStart,
    ) -> anyhow::Result<PreparedFinderCreate> {
        let FinderCreateStart {
            parent_contract,
            file_id,
            rel_path,
        } = start;
        let contents_path = target
            .contents_path
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("Finder create file callback did not include contents"))?;
        let staged = stage_finder_contents(
            &self.db,
            self.finder_staging_root()?,
            Path::new(contents_path),
            contents,
        )?;
        let staged_path = staged.path().to_string();
        let size_bytes = std::fs::metadata(&staged_path).map(|m| m.len() as i64).unwrap_or(0);
        let mime = target
            .content_type
            .as_deref()
            .or_else(|| beebeeb_core::media::guess_mime_type(&target.filename));
        let name_encrypted = owned_metadata_for_name(&self.db, self.api.master_key(), &file_id, &target.filename, mime)?;
        let row = FileEntry {
            file_id: file_id.clone(),
            // FULL relative key (e.g. `docs/a.txt`), so a nested file's
            // row is found by `get_file_by_path(full_key)` immediately —
            // not keyed by the leaf, which classify_local_path's filter-3
            // would miss for a nested file → spurious re-upload.
            path: rel_path.clone(),
            status: FileStatus::Uploading,
            size_bytes,
            modified_at: now_secs(),
            content_hash: None,
            remote_updated_at: 0,
            // Not persisted by upsert_file; contract owns these.
            parent_id: None,
            item_kind: ItemKind::File,
        };
        let mut payload = serde_json::json!({
            "operation": Op::CreateFile.as_str(),
            "name_encrypted": name_encrypted,
            "content_type": target.content_type,
            "uploaded_by": "authenticated_desktop_user",
        });
        apply_shared_context(&mut payload, parent_contract.as_ref());
        Ok(PreparedFinderCreate {
            file_id,
            rel_path,
            staged,
            staged_path,
            size_bytes,
            row,
            payload,
        })
    }

    /// The File Provider create (spec §8.8: minting is keyed on the IPC entries, never on
    /// the shared `queue_finder_*`, which the watcher and the upload driver call too).
    /// One accept transaction inserts the row, mints the write token and queues the
    /// upload (spec §8.7 S1.1). A folder carries no content and gets no token.
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    pub fn queue_file_provider_create_from(
        &self,
        target: FinderWriteTarget,
        contents: Option<&std::fs::File>,
    ) -> anyhow::Result<FpWrite> {
        if target.kind == FinderWriteItemKind::Folder {
            return self.queue_finder_create_from(target, contents).map(FpWrite::plain);
        }
        self.queue_file_provider_create_retiring(target, contents, None)?
            .map(|(write, _)| write)
            .ok_or_else(|| anyhow::anyhow!("a create that retires no parked write is never refused as moved"))
    }

    /// Task 2190 ("Keep both", review-169 #3): [`Self::queue_file_provider_create_from`], taking the
    /// place of the parked write `retire` names in the SAME transaction. `Ok(None)`: that write is no
    /// longer decidable at the version shown, nothing was queued, and the copy staged for the create
    /// is removed with its guard. With `retire`, the second value is the parked write's staged copy,
    /// released in the journal, for the caller to unlink. A file only: a folder create goes through
    /// [`Self::queue_file_provider_create_from`], and "Keep both" copies a file.
    pub(super) fn queue_file_provider_create_retiring(
        &self,
        target: FinderWriteTarget,
        contents: Option<&std::fs::File>,
        retire: Option<crate::state_db::RetireParked<'_>>,
    ) -> anyhow::Result<Option<(FpWrite, Option<String>)>> {
        let Some(start) = self.start_finder_create(&target)? else {
            return Ok(Some((FpWrite::plain(ignored_finder_item(&target.filename)), None)));
        };
        let prepared = self.prepare_finder_create(&target, contents, start)?;
        let op_id = uuid::Uuid::new_v4().to_string();
        let metadata_json = serde_json::to_string(&prepared.payload)?;
        let backup_source_key = crate::known_folder::backup_source_key_for_this_device(&prepared.rel_path);
        self.seam("accept:before_tx");
        let accepted = self.db.accept_finder_write(
            &crate::state_db::FinderAccept {
                retire_parked: retire,
                op_id: &op_id,
                file_id: &prepared.file_id,
                kind: crate::state_db::FinderAcceptKind::Create { row: &prepared.row },
                parent_id: target.parent_id.as_deref(),
                // The FULL relative key, as the Finder create queues it.
                target_path: Some(&prepared.rel_path),
                metadata_json: &metadata_json,
                payload_path: &prepared.staged_path,
                size_bytes: prepared.size_bytes,
                modified_at: prepared.row.modified_at,
                backup_source_key: backup_source_key.as_deref(),
                now: now_secs(),
            },
            &|entry, contract| legacy_item_identifiers(entry, contract).to_vec(),
        )?;
        let (token, released) = match accepted {
            crate::state_db::AcceptOutcome::Queued { token, .. } => (token, None),
            crate::state_db::AcceptOutcome::QueuedRetiring { token, released } => (token, released),
            crate::state_db::AcceptOutcome::ParkedAtOnce { token, reason } => {
                log_parked(&op_id, Some(&prepared.file_id), reason);
                (token, None)
            }
            crate::state_db::AcceptOutcome::ParkedMoved => return Ok(None),
            crate::state_db::AcceptOutcome::UnknownItem => anyhow::bail!("a create always inserts its row"),
            crate::state_db::AcceptOutcome::InTrash => anyhow::bail!("a create is never refused as in the trash"),
            crate::state_db::AcceptOutcome::CopyGone => return Err(anyhow::Error::new(StagedCopyGone)),
            crate::state_db::AcceptOutcome::AcceptsClosed => return Err(anyhow::Error::new(AcceptsClosed)),
        };
        prepared.staged.retain();
        Ok(Some((
            FpWrite {
                outcome: FinderWriteOutcome::Queued {
                    op_id,
                    file_id: Some(prepared.file_id),
                    kind: OperationKind::UploadVersion,
                    ignored: false,
                    message: "queued for encrypted sync".to_string(),
                },
                token: Some(token),
                present_as: None,
            },
            released,
        )))
    }

    /// A create with `.deletionConflicted` (spec §7.2, I-1): the system could not apply our
    /// delete of the template's item because the person edited it. Resolved through the alias
    /// (P→S), or a live row with the template's id, it is a content modify of that row,
    /// replied under the provider's identifier, S (`REPL.h:438-443`), so the system's reuse
    /// rule replaces item S on disk with the edited file. Otherwise an ordinary create.
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    pub fn queue_file_provider_deletion_conflicted_create(
        &self,
        target: FinderWriteTarget,
        template_identifier: Option<String>,
        template_content_version: Option<String>,
        contents: Option<&std::fs::File>,
    ) -> anyhow::Result<FpWrite> {
        let resolved = match template_identifier.as_deref() {
            Some(id) => match self.db.get_file(id)? {
                Some(row) => Some(row),
                None => match self.db.resolve_alias(id)? {
                    Some(server_id) => {
                        log_alias_resolved("deletion_conflicted_create", id, &server_id);
                        self.db.get_file(&server_id)?
                    }
                    None => None,
                },
            },
            None => None,
        };
        match resolved {
            // A content modify of S. Only a file has content to modify.
            Some(row)
                if row.status != FileStatus::Trashing
                    && row.item_kind == ItemKind::File
                    && target.kind == FinderWriteItemKind::File =>
            {
                self.queue_file_provider_modify_from(
                    FinderWriteTarget {
                        file_id: Some(row.file_id),
                        parent_id: target.parent_id,
                        filename: target.filename,
                        rel_path: None,
                        kind: FinderWriteItemKind::File,
                        contents_path: target.contents_path,
                        content_type: target.content_type,
                        base_version_identifier: template_content_version,
                    },
                    contents,
                )
            }
            // No alias and no row (a file deleted elsewhere: the edit becomes a new file), or a
            // row going to the trash (the edit must not follow it there): an ordinary create.
            _ => self.queue_file_provider_create_from(target, contents),
        }
    }

    /// §7.2: the id a request under `id` acts on, and the id its reply is presented under. A
    /// live row with `id` is acted on as is; otherwise an alias sends a provisional id to the
    /// server id its create landed as, presented under `id`. An id with neither is returned
    /// as is, for the caller to refuse or to let fail.
    #[cfg_attr(not(any(unix, test)), allow(dead_code))]
    pub fn resolve_provisional(&self, id: &str, request: &'static str) -> anyhow::Result<(String, Option<String>)> {
        if self.db.get_file(id)?.is_some() {
            return Ok((id.to_string(), None));
        }
        match self.db.resolve_alias(id)? {
            Some(server_id) => {
                log_alias_resolved(request, id, &server_id);
                Ok((server_id, Some(id.to_string())))
            }
            None => Ok((id.to_string(), None)),
        }
    }

    /// §7.4: a fetch of an item whose held write is queued is served from that write's
    /// staged bytes, the bytes its token names, and never reaches the server; otherwise
    /// from the server, as before. A write that has parked with its copy gone is the one
    /// exception: nothing local can serve it, so the server's version is fetched.
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    pub async fn serve_hydrate(
        &self,
        id: &str,
        dest: &Path,
        allowed_roots: &[&Path],
        progress: Option<&HydrateProgressFn>,
    ) -> anyhow::Result<HydrateSource> {
        if !hydrate_dest_is_allowed(dest, allowed_roots) {
            anyhow::bail!("hydrate destination is not within an allowed root");
        }
        let (file_id, _) = self.resolve_provisional(id, "hydrate")?;
        // A copy that is gone at the open is decided once more. If the landing unlinked it, the
        // write is no longer queued and the server has its bytes (§7.4, 4b). A write that is
        // still queued is never fetched from the server, which holds only the previous version:
        // with its copy missing, the fetch fails (`QueuedCopyMissing`), and the system retries.
        // A write that has parked never leaves the queue by itself and
        // its copy cannot come back, so that retry would never end. Its fetch goes to the
        // server for the current version, and the item keeps the parked status (§9.1).
        let mut copy_missing = false;
        for _ in 0..2 {
            let Some((path, parked)) = self.db.queue_fetch_source(&file_id)? else {
                copy_missing = false;
                break;
            };
            self.seam("hydrate:before_open");
            match std::fs::File::open(&path) {
                Ok(mut staged) => {
                    if let Some(parent) = dest.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    let bytes = write_hydrated_from_reader(dest, allowed_roots, &mut staged)?;
                    // Recorded as the server path records it, so the sign-out sweep finds a
                    // copy written outside the hydrate cache.
                    self.record_hydrated_destination(&file_id, dest)?;
                    if let Some(progress) = progress {
                        progress(bytes, bytes);
                    }
                    log_hydrate_served(&file_id, HydrateSource::Queue);
                    return Ok(HydrateSource::Queue);
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound && parked => {
                    copy_missing = false;
                    break;
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    copy_missing = true;
                    continue;
                }
                Err(e) => return Err(e.into()),
            }
        }
        if copy_missing {
            return Err(anyhow::Error::new(QueuedCopyMissing));
        }
        self.hydrate_file_with_progress(&file_id, dest, allowed_roots, progress)
            .await?;
        log_hydrate_served(&file_id, HydrateSource::Server);
        Ok(HydrateSource::Server)
    }

    /// A Finder thumbnail (spec §5.6, §7.2): through the alias, and never while the item's
    /// held write is queued, nor after its landing until that version's thumbnails are
    /// uploaded (the safe default: a per-item error, and nothing asked of the server).
    #[cfg_attr(not(any(unix, test)), allow(dead_code))]
    pub async fn finder_thumbnail(&self, id: &str, variant: &str) -> anyhow::Result<Zeroizing<Vec<u8>>> {
        let (file_id, _) = self.resolve_provisional(id, "thumbnail")?;
        // The queue first, then the mark. A landing sets the mark before it commits the op's
        // removal and clears it only after the upload, so a request that finds the write no
        // longer queued finds the mark. Read the other way round, a landing between the two
        // reads served the previous version's thumbnail under the new token.
        let queued = self
            .db
            .item_presentation(&file_id)?
            .is_some_and(|presentation| presentation.held_write_queued);
        self.seam("thumbnail:between_reads");
        let marked = self.thumbnails_pending(&file_id);
        if queued || marked {
            return Err(anyhow::Error::new(ThumbnailNotYet));
        }
        self.fetch_thumbnail_to_memory(&file_id, variant).await
    }
}
