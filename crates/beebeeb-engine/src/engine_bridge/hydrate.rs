//! `hydrate` — the `EngineBridge` impl for hydrate/download of a file, its ranges and versions, split out of `engine_bridge.rs`.

use super::*;

impl EngineBridge {
    pub(super) fn ensure_shared_parent_allows_write(
        &self,
        parent_id: Option<&str>,
    ) -> Result<Option<FileContractState>, EngineError> {
        let Some(parent_id) = parent_id else {
            return Ok(None);
        };
        if parent_id == "namespace:shared_with_me" {
            return Err(anyhow::anyhow!(
                "Shared with me is read-only at the namespace root; open an editable shared folder first"
            )
            .into());
        }
        let Some(contract) = self.db.get_file_contract_state(parent_id)? else {
            return Ok(None);
        };
        if contract.is_shared() && !contract.can_write() {
            return Err(anyhow::anyhow!("read-only shared folder cannot accept new Finder items").into());
        }
        Ok(Some(contract).filter(|contract| contract.is_shared()))
    }

    pub(super) fn ensure_item_allows_shared_write(
        &self,
        file_id: &str,
        operation: &str,
    ) -> Result<Option<FileContractState>, EngineError> {
        let Some(contract) = self.db.get_file_contract_state(file_id)? else {
            return Ok(None);
        };
        if contract.is_shared() && !contract.can_write() {
            return Err(
                anyhow::anyhow!("read-only shared item cannot be changed from Finder during {operation}").into(),
            );
        }
        Ok(Some(contract).filter(|contract| contract.is_shared()))
    }

    /// Download `file_id` from the vault, decrypt, write to `dest_path`.
    /// Called by the Linux FUSE driver, the IPC socket, and conflict-resolution
    /// paths where the decrypted file must land on disk (sync root or a caller-
    /// chosen path). Status transitions: `Downloading` → `Local` on success,
    /// `Error` on failure.
    ///
    /// Steps mirror `repos/cli/src/commands/pull.rs::pull_single_file`:
    ///
    /// 1. Flip status to `Downloading` so the Finder/Explorer overlay
    ///    shows a spinner immediately.
    /// 2. Fetch fresh metadata to learn `chunk_count` (the local
    ///    placeholder may not have it).
    /// 3. Resolve the per-file key. Owned files derive it from the master key;
    ///    shared-with-me files unwrap the sender-provided key envelope using
    ///    the share invite's X25519/HKDF contract.
    /// 4. For each chunk index: GET the bytes, parse as
    ///    `EncryptedBlob` JSON, decrypt with `decrypt_chunk`,
    ///    accumulate plaintext.
    /// 5. Write the whole plaintext to `dest_path` (caller chose the
    ///    layout — `dest_path` is usually `<sync_root>/<decrypted_name>`
    ///    or a per-extension cache path).
    /// 6. Flip status to `Local`.
    ///
    /// On any error the status is flipped to `Error` so the overlay can
    /// render a problem indicator, then the error is bubbled.
    ///
    /// **Windows Cloud Files callers must use [`Self::hydrate_file_to_memory`]
    /// instead** — that variant never writes plaintext to disk, satisfying the
    /// zero-knowledge requirement for on-demand CF hydration.
    pub async fn hydrate_file(&self, file_id: &str, dest_path: &Path, allowed_roots: &[&Path]) -> anyhow::Result<()> {
        self.hydrate_file_with_progress(file_id, dest_path, allowed_roots, None)
            .await
    }

    /// [`Self::hydrate_file`] plus an optional `progress(done_bytes,
    /// total_bytes)` callback, invoked synchronously from the download loop
    /// (task 1670 issue 3 — the macOS IPC handler forwards it to Finder).
    ///
    /// Cancel-safe: if this future is dropped while the download is in
    /// flight (the IPC client hung up), the row's status is restored instead
    /// of being left on `Downloading` forever.
    pub async fn hydrate_file_with_progress(
        &self,
        file_id: &str,
        dest_path: &Path,
        allowed_roots: &[&Path],
        progress: Option<&HydrateProgressFn>,
    ) -> anyhow::Result<()> {
        self.hydrate_from(file_id, dest_path, allowed_roots, progress, HydrateStart::Here)
            .await
    }

    /// Dm-72: the download of a Finder fetch (the File Provider's `fetchContents`, [`Self::serve_hydrate`]): the
    /// bytes written to `dest_path` are handed to the system and become Finder's copy of the file. On a Mac that copy
    /// is the file's one copy on this computer, so the version and content it holds are recorded as agreed, as for
    /// a copy in the sync folder: the app's own reads then know Finder's copy is the current version (and stop
    /// reading it when a newer one comes in), and the stale pass knows which copies to signal the system for.
    pub(super) async fn hydrate_for_finder(
        &self,
        file_id: &str,
        dest_path: &Path,
        allowed_roots: &[&Path],
        progress: Option<&HydrateProgressFn>,
    ) -> anyhow::Result<()> {
        self.hydrate_from(file_id, dest_path, allowed_roots, progress, HydrateStart::Finder)
            .await
    }

    /// S6 (R7 b): a hydrate the queue runs (a pin, a re-download, a kept fetch). Never the person's resolution: unknown
    /// bytes at the place refuse it whatever the row's status, so the retry of a failed fetch (its row left `Error`)
    /// never replaces a file made at the place meanwhile.
    pub(super) async fn hydrate_queued(&self, file_id: &str, dest: &Path, roots: &[&Path]) -> anyhow::Result<()> {
        self.hydrate_from(file_id, dest, roots, None, HydrateStart::Queued)
            .await
    }

    /// S6 (R9): a hydrate whose start look the caller took (Keep there, before its safety copy reads a byte), so a
    /// save made after that look is never replaced. A destination other than the place it looked at refuses.
    pub async fn hydrate_seen(
        &self,
        file_id: &str,
        dest: &Path,
        roots: &[&Path],
        seen: PlaceSeen,
    ) -> anyhow::Result<()> {
        self.hydrate_from(file_id, dest, roots, None, HydrateStart::Seen(seen))
            .await
    }

    /// Keep there for a caller that takes no look of its own (the Tauri-era command): the look is taken here, before
    /// anything is downloaded. The native Keep there looks before its safety copy ([`Self::resolve_theirs_seen`]).
    /// L409: this caller keeps no copy of the place, so the hydrate's relief for Keep there (an edit against agreed
    /// content is the person's to replace) is not its own: an unsent change at the place is refused here, after the look
    /// and before the hydrate, as before L409. A save after the look is refused by the hydrate's compare with the look.
    pub async fn resolve_keep_theirs(&self, file_id: &str, sync_root: &Path) -> anyhow::Result<()> {
        let entry = self.db.get_file(file_id)?;
        let path = entry
            .ok_or_else(|| anyhow::anyhow!("no state.db row for {file_id}"))?
            .path;
        let place = local_file_path_under_sync_root(sync_root, &path)?;
        let seen = PlaceSeen::take_async(&place).await?;
        if self.place_holds_an_unsent_change(file_id, &place, true, false)? {
            tracing::warn!("hydrate refused: the local file holds a change that is not sent");
            return Err(anyhow::Error::new(UnsentLocalChange));
        }
        self.resolve_theirs_seen(file_id, sync_root, seen).await
    }

    async fn hydrate_from(
        &self,
        file_id: &str,
        dest_path: &Path,
        allowed_roots: &[&Path],
        progress: Option<&HydrateProgressFn>,
        start: HydrateStart,
    ) -> anyhow::Result<()> {
        // Task 1247 self-defense: validate `dest_path` against the caller's
        // trusted root(s) BEFORE any directory creation or plaintext write.
        // `dest_path` reaches the one untrusted caller (the IPC socket handler)
        // straight off the wire, and the `file_id`-keyed guard below
        // (`ensure_shared_hydrate_path_safe`) validates an entirely SEPARATE
        // value (`entry.path`), so it can never vouch for `dest_path`. Without
        // this check any local process could turn the daemon into a
        // decrypt-oracle + arbitrary-write primitive (e.g. writing decrypted
        // vault plaintext over ~/.ssh/authorized_keys).
        if !hydrate_dest_is_allowed(dest_path, allowed_roots) {
            return Err(anyhow::anyhow!("hydrate destination is not within an allowed root"));
        }
        // Task 2407 (core 2404): a destination whose name reaches a different entry (a Windows 8.3 short name) is
        // refused before anything is downloaded; the Windows writer checks again right before it writes.
        crate::name_alias::refuse_conflict_under_roots(dest_path, allowed_roots)?;
        self.ensure_shared_hydrate_path_safe(file_id)?;
        // Task 1640: a hydrate into the file's own place in the sync folder (on Windows, only
        // conflict resolution reaches it) never writes over an unsent local change: bytes that differ from the content this
        // device last agreed with the server on, or whose upload is queued, are the person's
        // only copy. It refuses, and the file keeps its bytes. A Finder fetch (macOS) writes
        // to the extension's own staging copy, never there.
        let in_sync_folder = self.dest_is_files_sync_folder_place(file_id, dest_path, allowed_roots)?;
        // Dm-72: on a Mac the bytes of a Finder fetch become the file's one copy on this computer (Finder's). Not
        // while a change of the file's own is on its way out (a held Finder write, a queued upload): its landing
        // records the content.
        let finders_copy = !in_sync_folder
            && matches!(start, HydrateStart::Finder)
            && kept_bytes_come_by_the_systems_fetch()
            && !self.db.sync_folder_upload_queued(file_id)?
            && !self
                .db
                .item_presentation(file_id)?
                .is_some_and(|presentation| presentation.held_write_queued);
        // Task 1640 (review N1b): the version a legacy download (no `download_version`) records,
        // read BEFORE the download: at most the version the bytes are, so a later change can only
        // be refused as stale, never accepted over a newer one.
        let legacy_version = self.legacy_download_version(file_id)?;
        // A conflict or an error is the person's to resolve (Keep there keeps a copy first): read once, here, before
        // the row becomes `Downloading`, for both checks below. Never for a queued hydrate (S6 R7 b).
        let persons_resolution = !matches!(start, HydrateStart::Queued)
            && self
                .db
                .get_file(file_id)?
                .is_some_and(|entry| matches!(entry.status, FileStatus::Conflict | FileStatus::Error));
        // L409: Keep there into the file's own place (`HydrateStart::Seen`) is the person's resolution, and the place's
        // bytes were kept before this hydrate began: the native Keep there makes its safety copy after its look, and the
        // Tauri-era one (`resolve_keep_theirs`) keeps none, so it refuses an unsent change itself before it comes here.
        // An edit against agreed content is then theirs to replace. Every other hydrate keeps the guard's rule as it was.
        let keep_there = persons_resolution && matches!(start, HydrateStart::Seen(_));
        // S6 (R8): the start look, before the start content look: right before the place is replaced it must still be
        // what this look saw, so a save made at any point after it (during the download included) is never replaced.
        let seen_at_start = match (in_sync_folder, start) {
            (false, _) => None,
            (true, HydrateStart::Seen(seen)) if seen.place == dest_path => Some(seen.fingerprint),
            (true, HydrateStart::Seen(_)) => {
                tracing::warn!(file_id = %file_id, "hydrate refused: not the place that was looked at");
                return Err(anyhow::Error::new(UnsentLocalChange));
            }
            (true, HydrateStart::Here | HydrateStart::Queued | HydrateStart::Finder) => {
                Some(PlaceSeen::take_async(dest_path).await?.fingerprint)
            }
        };
        if in_sync_folder && self.place_holds_an_unsent_change(file_id, dest_path, persons_resolution, keep_there)? {
            tracing::warn!(file_id = %file_id, "hydrate refused: the local file holds a change that is not sent");
            return Err(anyhow::Error::new(UnsentLocalChange));
        }
        // Spec §7.4.2: a hydrate never changes the status of a row with a live upload of its
        // own (a Finder upload that has not parked): no `Downloading`, `Local` or `Error`.
        // That status is the upload's, and `Error` would make the item read-only. Nor
        // of a row whose Finder upload has parked. Its status is the
        // parked one (§9.1), and `Local` would make it writable and evictable.
        let touch_status = !self
            .db
            .item_presentation(file_id)?
            .is_some_and(|presentation| presentation.unparked_finder_upload || presentation.parked_finder_upload);
        // Every status the hydrate writes after its `Downloading` is written only over that
        // `Downloading`: a save accepted meanwhile has made the row `Uploading`, and its status
        // stays (spec §9.2). Before, the hydrate's end wrote over it.
        let set_failed = || {
            if touch_status {
                // Best-effort status flip; if the DB is broken we still
                // return the original error.
                let _ = self
                    .db
                    .set_status_if(file_id, FileStatus::Downloading, FileStatus::Error);
            }
        };
        // RAII-style: any early return below the status flip should
        // leave the file in `Error`, not `Downloading`. We do that by
        // wrapping the body in an inner async fn whose Err branch we
        // catch.
        let mut downloading_guard = touch_status.then(|| DownloadingStatusGuard::arm(&self.db, file_id));
        if touch_status {
            self.db.set_status(file_id, FileStatus::Downloading)?;
        }
        // Task 1683 slice 2: the popover's per-file bytes. The caller's own progress
        // callback (the IPC handler forwarding to Finder) still gets every report.
        // LD-0105 U1: named by the file's vault path, and joining a fetch of the file already running.
        let path = self
            .db
            .get_file(file_id)
            .ok()
            .flatten()
            .map(|entry| entry.path)
            .unwrap_or_default();
        let transfer = self.transfers.begin_download(file_id, &path, 0);
        let hydrated = self.do_hydrate_versioned(file_id, progress, None).await;
        // Past the only cancellation point (the download await): every path
        // below sets the final status itself.
        if let Some(guard) = downloading_guard.as_mut() {
            guard.disarm();
        }
        match hydrated {
            Ok((mut buf, downloaded_version)) => {
                // Write the decrypted bytes to disk (this is the intentional
                // disk-writing path — sync root / conflict resolution / FUSE).
                // Make the destination directory if needed.
                if let Some(parent) = dest_path.parent()
                    && let Err(e) = std::fs::create_dir_all(parent)
                {
                    buf.zeroize();
                    // this used to `?` straight out of the
                    // match arm below, which skipped the `Err(e) =>` arm's
                    // `FileStatus::Error` flip entirely — a directory-create
                    // failure left the row stuck on `Downloading` forever.
                    // Route it through the same status flip as every other
                    // failure in this function.
                    set_failed();
                    return Err(anyhow::anyhow!("create dest dir {}: {e}", parent.display()));
                }
                // Write via an O_NOFOLLOW handle (task 1247): the containment
                // guard at the top of this fn is a check-then-use, and there is
                // a real time window here (`do_hydrate` did a network download +
                // decrypt). A same-UID attacker could plant a symlink at
                // `dest_path` during that window pointing at, say,
                // ~/.ssh/authorized_keys; a plain `fs::write` would follow it
                // and overwrite the real target with decrypted plaintext.
                // `write_hydrated_plaintext` fails closed if the final component
                // is (or race-becomes) a symlink, closing the race atomically at
                // open() time rather than re-checking-then-hoping. Since task
                // it also stages the write under a temp name and
                // publishes with one atomic rename, so a failure anywhere in
                // that sequence is guaranteed to leave NOTHING new at
                // `dest_path` — there is no separate "delete the half-written
                // file" step needed here because the primitive itself never
                // makes one externally visible.
                // Task 2523: the download took time, and the place may have been changed meanwhile (on Linux
                // nothing would ever notice it): asked again right before the place is replaced. A change keeps
                // its bytes; the row goes back to what it was.
                self.seam("hydrate:before_write");
                let refuse_changed = |buf: &mut Vec<u8>, why: &'static str| {
                    buf.zeroize();
                    if let Some(guard) = &downloading_guard {
                        let _ = self
                            .db
                            .set_status_if(file_id, FileStatus::Downloading, guard.restore.clone());
                    }
                    tracing::warn!(file_id = %file_id, "hydrate refused: {why}");
                    anyhow::Error::new(UnsentLocalChange)
                };
                if in_sync_folder
                    && self.place_holds_an_unsent_change(file_id, dest_path, persons_resolution, keep_there)?
                {
                    return Err(refuse_changed(&mut buf, "the local file was changed during the download"));
                }
                // Review P2-1, S6 (R8): the place (its size, times and inode) compared with the start look once the copy
                // is written, right before it replaces the place: a save after the start look is never replaced.
                let mut place_unchanged = || {
                    self.seam("hydrate:before_replace");
                    seen_at_start
                        .as_ref()
                        .is_none_or(|before| *before == place_fingerprint(dest_path))
                };
                let write_result =
                    write_hydrated_plaintext_checked(dest_path, allowed_roots, &buf, &mut place_unchanged);
                if write_result.as_ref().is_err_and(place_changed_before_replace) {
                    return Err(
                        refuse_changed(&mut buf, "the local file was changed while the download was written"),
                    );
                }
                let hydrated_hash = (in_sync_folder || finders_copy).then(|| sha256_hex(&buf));
                // Zeroize the in-memory copy now that it is on disk (or on
                // error) so the allocation does not linger with plaintext.
                buf.zeroize();
                if let Err(e) = write_result {
                    set_failed();
                    return Err(anyhow::anyhow!("write {}: {e}", dest_path.display()));
                }
                // Task 1640: the bytes just written (one atomic rename: the whole file) are the
                // content this device agrees with the server on, at the version the download
                // metadata named, else (a legacy download, review N1b) the row's version read
                // before the download. Recorded before the row leaves `Downloading`, so no
                // classification sees the new bytes against the old baseline. With no version
                // either, the content is not known (review F1), rather than left naming other bytes.
                if let Some(hash) = hydrated_hash.as_deref() {
                    let recorded = match downloaded_version.or(legacy_version) {
                        Some(version) => self.db.record_local_baseline(file_id, version, hash),
                        None => self.db.mark_content_unknown(file_id),
                    };
                    if let Err(e) = recorded {
                        set_failed();
                        return Err(e.into());
                    }
                }

                // a Trashing row keeps its marker (viewing a file
                // from the macOS Trash view must not un-trash it in the
                // replica); everything else completes as `Local`.
                // the pre-hydrate status is the
                // row's status BEFORE the flip to `Downloading` — re-reading
                // the DB here always saw `Downloading`, so a Trashing row
                // completed as `Local` and reparented itself out of the trash
                // view. `DownloadingStatusGuard::arm` captured exactly that
                // pre-flip status when it armed above — reuse it.
                self.seam("hydrate:before_final_status");
                if let Some(guard) = &downloading_guard {
                    self.db.set_status_if(
                        file_id,
                        FileStatus::Downloading,
                        hydrate_final_status(guard.restore.clone()),
                    )?;
                }
                let downloaded_bytes = self.transfers.get(file_id).map(|t| t.total).unwrap_or(0);
                transfer.finish();
                self.record_transfer_done(crate::transfer_progress::Direction::Down, file_id, downloaded_bytes);

                // See [`record_hydration_cache_state`]'s
                // doc comment for the full "why" — in short, a macOS Finder
                // `fetchContents` hydration writes here as a HANDOFF, not a
                // durable local cache copy, and registering it via
                // `mark_cached` anyway left a phantom cache-usage entry in
                // `desktop_storage_summary` / smart-cache eviction after the
                // system (not us) took over the materialized content.
                self.record_hydrated_destination(file_id, dest_path)?;
                #[cfg(target_os = "linux")]
                self.write_linux_freedesktop_thumbnails(file_id, dest_path).await;
                let _ = self.enforce_configured_cache_limit();
                Ok(())
            }
            Err(e) => {
                let e = self.owned_open_failure(file_id, land_error_to_anyhow(e)).await;
                set_failed(); // only writes over Downloading; the recoverable item already left it.
                Err(e)
            }
        }
    }

    /// Task 1640 / 2191 / 2523: the bytes at `place` (the file's own place in the sync folder) may be the person's
    /// only copy of a change, so a hydrate never writes over them: bytes that differ from the content this device last
    /// agreed with the server on, a queued upload, or bytes of unknown content (no agreed content and nothing fetched
    /// recorded) while the place holds any. Every row, wherever its content came from (task 2523, F3: before, the
    /// unknown case covered only files hydrated before 0.8.13, and a held edit after a viewer save was not one).
    /// `persons_resolution`: the row was a conflict or an error when the hydrate began; its unknown bytes are then the
    /// person's to resolve (Keep there keeps a copy first), and only a change against agreed content refuses.
    /// `keep_there` (L409): that resolution is Keep there into the file's own place, whose bytes were kept before the
    /// hydrate began (the native Keep there's safety copy; the Tauri-era [`Self::resolve_keep_theirs`] keeps none and
    /// refuses an unsent change before it hydrates). Its file with agreed content recorded and edited bytes is then the
    /// person's to replace; a queued upload, and a change against content known only from fetches, still refuse.
    pub(super) fn place_holds_an_unsent_change(
        &self,
        file_id: &str,
        place: &Path,
        persons_resolution: bool,
        keep_there: bool,
    ) -> Result<bool, EngineError> {
        if !place.exists() {
            return Ok(false);
        }
        let content = sync_folder_content(&self.db, file_id, place, None)?;
        if matches!(content, SyncFolderContent::Changed { .. }) && keep_there && self.db.has_agreed_content(file_id)? {
            return Ok(false);
        }
        if matches!(content, SyncFolderContent::Changed { .. } | SyncFolderContent::Queued) {
            return Ok(true);
        }
        if !matches!(content, SyncFolderContent::Unknown) {
            return Ok(false);
        }
        Ok(!persons_resolution && place_holds_bytes(place))
    }

    /// Record where a hydrate wrote `file_id`'s plaintext, for the cache accounting and the
    /// sign-out sweep. Both hydrate paths, the server's and the queue's, call it. On macOS a
    /// Finder hand-off into the hydrate cache is not a cached copy ([`record_hydration_cache_state`]).
    pub(super) fn record_hydrated_destination(&self, file_id: &str, dest_path: &Path) -> Result<(), EngineError> {
        #[cfg(target_os = "macos")]
        record_hydration_cache_state(
            &self.db,
            file_id,
            dest_path,
            &crate::ipc_socket::macos_hydrate_cache_dir(self.identity),
        )?;
        #[cfg(not(target_os = "macos"))]
        {
            let cache_bytes = std::fs::metadata(dest_path).map(|m| m.len() as i64).unwrap_or(0);
            self.db
                .mark_cached(file_id, &dest_path.to_string_lossy(), cache_bytes, now_secs())?;
        }
        Ok(())
    }

    pub(super) fn ensure_shared_hydrate_path_safe(&self, file_id: &str) -> Result<(), EngineError> {
        let Some(contract) = self.db.get_file_contract_state(file_id)? else {
            return Ok(());
        };
        if contract.namespace != Namespace::SharedWithMe {
            return Ok(());
        }

        let entry = self
            .db
            .get_file(file_id)?
            .ok_or_else(|| anyhow::anyhow!("shared hydrate target missing state row for {file_id}"))?;
        crate::reject_unsafe_rel_path(&entry.path)
            .map_err(|e| anyhow::anyhow!("unsafe shared hydrate path for {file_id}: {e}"))?;
        Ok(())
    }

    /// Download `file_id` from the vault, decrypt, and return the plaintext
    /// **in memory only** — it is NEVER written to disk.
    ///
    /// This is the Windows Cloud Files hydration path. The Cloud Files runtime
    /// calls `fetch_data_callback` on a filter-driver thread and expects us to
    /// deliver bytes via `CfExecute(TRANSFER_DATA)`; there is no requirement to
    /// materialise the plaintext as a file. Writing to `%TEMP%` would expose
    /// decrypted user data on disk, violating the zero-knowledge contract.
    ///
    /// The returned [`Zeroizing`] wrapper overwrites the buffer with zeros on
    /// drop, so plaintext is wiped from memory as soon as the caller is done
    /// with it — even if an early `return` or `?` is taken. Intermediate
    /// per-chunk decrypted buffers are also explicitly zeroized after being
    /// copied into the accumulator (see [`Self::do_hydrate`]).
    ///
    /// Status transitions: `Downloading` → `Local` on success, `Error` on
    /// failure. The `cache_path` recorded in state_db is `""` (empty) because
    /// the bytes live inside the CF placeholder in the sync root, not at a
    /// separate cache file — `unpinned_local_files_for_dehydration` reconstructs
    /// the real on-disk path from `path` + sync root (see state_db comment).
    #[cfg(target_os = "windows")]
    pub async fn hydrate_file_to_memory(&self, file_id: &str) -> anyhow::Result<FetchedBytes> {
        self.hydrate_file_to_memory_metered(file_id, None).await
    }

    /// LD-0105 U1: the Windows whole-file fetch's row on the transfer board, the "Downloading" row of the
    /// U0 design, named by the file's vault path (the `files` row's, the path the tray's recent rows
    /// carry), sized `size` until the download's metadata says otherwise. A fetch of the file already
    /// running is joined ([`crate::transfer_progress::TransferBoard::begin_download`]). No `files` row
    /// (or no readable one): no board row, since nothing could name it. A fetch cut off by an engine
    /// stop (lock, sign-out, switch, quit) ends its row without a failure.
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    pub fn whole_fetch_row(&self, file_id: &str, size: u64) -> crate::transfer_progress::FetchRow {
        match self.db.get_file(file_id) {
            Ok(Some(entry)) => {
                crate::transfer_progress::FetchRow::new(self.transfers.begin_download(file_id, &entry.path, size))
                    .stopped_by(self.stopping.clone())
            }
            _ => crate::transfer_progress::FetchRow::none(),
        }
    }

    /// [`Self::hydrate_file_to_memory`], also counting every body byte into `meter` as it arrives,
    /// read by read (LD-0105 S0): the Windows fetch callback reports that count to Cloud Files so a
    /// download that takes longer than Windows' 60 s callback limit is not failed and fetched again.
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    pub async fn hydrate_file_to_memory_metered(
        &self,
        file_id: &str,
        meter: Option<&crate::transfer_progress::ReceiveMeter>,
    ) -> anyhow::Result<FetchedBytes> {
        // Self-review #1: counted from before anything is recorded until the caller's transfer ended.
        let ticket = self.begin_fetch(file_id);
        let legacy_version = self.legacy_download_version(file_id)?;
        self.db.set_status(file_id, FileStatus::Downloading)?;
        match self.do_hydrate_versioned(file_id, None, meter).await {
            Ok((buf, downloaded_version)) => {
                // Task 1640 (review N1, F1): every fetch, of a range or of the whole file, is
                // recorded BEFORE its bytes are transferred: the version it serves and the hash of
                // all of that version's bytes join the file's candidate (`record_fetched_version`),
                // so a transfer that fails or is cut off midway still counts. Only once the whole
                // file is transferred does the caller make it the agreed content
                // ([`Self::record_whole_fetch`]); a range never does: the placeholder's other
                // ranges may hold other versions, or writes that were not sent. The stamp pass
                // promotes a candidate once the whole file is on disk, in sync, with exactly those
                // bytes. A download that names no version leaves the content not known. A failure
                // to record fails the fetch, as every other failure here does.
                let hash = sha256_hex(&buf);
                let fetched = downloaded_version
                    .or(legacy_version)
                    .map(|version| FetchedVersion { version, hash });
                let recorded = match &fetched {
                    Some(fetched) => self.db.record_fetched_version(file_id, fetched.version, &fetched.hash),
                    None => self.db.mark_content_unknown(file_id),
                };
                if let Err(e) = recorded {
                    let _ = self.db.set_status(file_id, FileStatus::Error);
                    return Err(e.into());
                }
                self.db.set_status(file_id, FileStatus::Local)?;
                // cache_path is empty: on Windows CF the hydrated bytes live
                // INSIDE the placeholder (the CF runtime writes them there after
                // our CfExecute(TRANSFER_DATA) calls). There is no separate temp
                // file. Byte count is still recorded for smart-cache accounting.
                self.db.mark_cached(file_id, "", buf.len() as i64, now_secs())?;
                let _ = self.enforce_configured_cache_limit();
                Ok(FetchedBytes {
                    bytes: buf,
                    version: fetched,
                    ticket,
                })
            }
            Err(e) => {
                let e = self.owned_open_failure(file_id, land_error_to_anyhow(e)).await;
                let _ = self
                    .db
                    .set_status_if(file_id, FileStatus::Downloading, FileStatus::Error);
                Err(e)
            }
        }
    }

    /// Drive-complete 169: the whole-file fetch, streamed. Chunk by chunk in file order, each chunk is
    /// downloaded, decrypted through core's sequential reader (order, last frame and `finish` checked)
    /// and handed to `sink(offset, plaintext)`, then wiped before the next one is fetched. At most one
    /// chunk of plaintext (and, for a legacy file, one more chunk's wire bytes) is held at a time, so a
    /// multi-GB file costs a few chunks of memory, not its size.
    ///
    /// The file's real size is known before any byte reaches the sink: a pinned version's manifest says
    /// it; a legacy file's last chunk is fetched (and opened) first, its wire bytes kept for the stream's
    /// end. When it is not `file_size` (the placeholder's), nothing is handed over: `Some(Err(true_size))`,
    /// the authenticated length the caller resizes the placeholder to (task 0783).
    ///
    /// `None`: no layout to stream by (no chunk count, no stride, or a stride that is not a multiple of
    /// the 4 KiB page Cloud Files requires of every transfer but the last): the caller takes the
    /// buffered path. `Some(Ok(fetched))`: every byte went to the sink; `fetched.bytes` is empty and
    /// `fetched.version` carries the hash of all of them, for [`Self::record_whole_fetch`].
    ///
    /// Task 1640: a fetch is recorded before its first byte is transferred. Its hash is not known then,
    /// so the version joins the candidate with [`STREAMING_HASH`], which no content ever hashes to: a
    /// stream cut off midway leaves a candidate never promoted, at a version never too high.
    #[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
    pub async fn stream_whole_file<S>(
        &self,
        file_id: &str,
        file_size: u64,
        meter: Option<&crate::transfer_progress::ReceiveMeter>,
        mut sink: S,
    ) -> anyhow::Result<Option<Result<FetchedBytes, u64>>>
    where
        S: FnMut(u64, &[u8]) -> anyhow::Result<()>,
    {
        let _file_uuid: uuid::Uuid = file_id
            .parse()
            .map_err(|e| anyhow::anyhow!("invalid file_id (not a UUID): {e}"))?;
        let meta = self.api.get_file(file_id).await?;
        let pin = crate::download_pin::PinnedVersion::from_metadata(&meta).map_err(EngineError::from)?;
        let (chunk_count, stride) = match &pin {
            Some(pin) => (pin.chunk_count, pin.chunk_size_bytes),
            None => (
                meta.get("chunk_count").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                meta.get("chunk_size_bytes").and_then(|v| v.as_u64()).unwrap_or(0),
            ),
        };
        if chunk_count == 0 || stride == 0 || stride % STREAM_PAGE != 0 {
            return Ok(None);
        }
        let (file_key, name_key) = self.file_keys_for_download(file_id).await?;
        let record = recorded_content_format(&name_key, &meta)?;
        let download_kbps_limit = crate::config::DesktopConfig::load_for(self.identity)
            .map(|c| c.download_kbps_limit)
            .unwrap_or(0);
        let last = chunk_count - 1;

        // The real size before anything is handed over.
        let (true_size, mut last_wire) = match &pin {
            Some(pin) => (pin.plaintext_size_bytes, None),
            None => {
                let wire = self.download_content_frame(file_id, last, None, meter, None).await?;
                let mut plain = record.open_chunk(&file_key, file_id, last, true, &content_frame(&wire)?)?;
                let last_len = plain.len() as u64;
                plain.zeroize();
                if last_len > stride || (last_len == 0 && chunk_count > 1) {
                    return Ok(None);
                }
                (u64::from(last) * stride + last_len, Some(wire))
            }
        };
        if true_size != file_size {
            return Ok(Some(Err(true_size)));
        }

        let ticket = self.begin_fetch(file_id);
        let legacy_version = self.legacy_download_version(file_id)?;
        let version = meta["download_version"]["version_number"].as_i64().or(legacy_version);
        self.db.set_status(file_id, FileStatus::Downloading)?;
        let recorded = match version {
            Some(version) => self.db.record_fetched_version(file_id, version, STREAMING_HASH),
            None => self.db.mark_content_unknown(file_id),
        };
        if let Err(e) = recorded {
            let _ = self.db.set_status(file_id, FileStatus::Error);
            return Err(e.into());
        }

        let streamed: Result<String, EngineError> = async {
            use sha2::Digest;
            let mut hash = sha2::Sha256::new();
            let mut reader = record.decryptor(beebeeb_core::kdf::FileKey::from_bytes(*file_key.as_bytes()), file_id);
            let mut offset: u64 = 0;
            for i in 0..chunk_count {
                let started = std::time::Instant::now();
                let wire = match if i == last { last_wire.take() } else { None } {
                    Some(wire) => wire,
                    None => {
                        self.download_content_frame(file_id, i, pin.as_ref(), meter, None)
                            .await?
                    }
                };
                let wire_len = wire.len() as u64;
                let mut plain = reader.push_frame(&content_frame(&wire)?)?.data;
                drop(wire);
                let want = if i == last { true_size - offset } else { stride };
                if plain.len() as u64 != want {
                    plain.zeroize();
                    return Err(
                        anyhow::anyhow!("chunk {i} is {} bytes, the file's layout says {want}", plain.len()).into(),
                    );
                }
                hash.update(&plain);
                let handed = sink(offset, &plain);
                plain.zeroize();
                handed?;
                offset += want;
                self.wire.download_bytes.fetch_add(wire_len, Ordering::Relaxed);
                if download_kbps_limit > 0 {
                    let budget = wire_len as f64 / (download_kbps_limit as f64 * 1024.0);
                    let spent = started.elapsed().as_secs_f64();
                    if budget > spent {
                        tokio::time::sleep(Duration::from_secs_f64(budget - spent)).await;
                    }
                }
            }
            reader.finish()?;
            Ok(format!("{:x}", hash.finalize()))
        }
        .await;
        match streamed {
            Ok(hash) => {
                // The whole version is in the placeholder; its candidate gets its real hash only through
                // `record_whole_fetch` (alone) or stays unpromotable (overlapped), never too high.
                self.db.set_status(file_id, FileStatus::Local)?;
                self.db.mark_cached(file_id, "", true_size as i64, now_secs())?;
                let _ = self.enforce_configured_cache_limit();
                Ok(Some(Ok(FetchedBytes {
                    bytes: Zeroizing::new(Vec::new()),
                    version: version.map(|version| FetchedVersion { version, hash }),
                    ticket,
                })))
            }
            Err(e) => {
                let e = self.owned_open_failure(file_id, land_error_to_anyhow(e)).await;
                let _ = self
                    .db
                    .set_status_if(file_id, FileStatus::Downloading, FileStatus::Error);
                Err(e)
            }
        }
    }

    /// Task 1640 (review F1, design T5): a fetch transferred the whole file (`fetched`, from
    /// [`Self::hydrate_file_to_memory`]): every byte on disk is that version's now, so they are the
    /// content this device agrees with the server on, and nothing else fetched is left. Only when no
    /// other fetch of the file overlapped this one (`ticket`, self-review #1): another fetch's bytes,
    /// of another version, could land around this transfer. Then the candidate this fetch merged
    /// stays (lower or mixed). `Ok(true)`: recorded.
    #[cfg_attr(
        not(any(target_os = "windows", any(test, feature = "test-support"))),
        allow(dead_code)
    )]
    pub fn record_whole_fetch(
        &self,
        file_id: &str,
        fetched: &FetchedVersion,
        ticket: &FetchTicket,
    ) -> anyhow::Result<bool> {
        // Review-151d P3-4: the check and the write under the tracker's lock, so no fetch can begin
        // (and record) between them. Lock order: the tracker, then the database; nothing takes the
        // tracker while holding the database.
        let fetches = FETCHES_IN_FLIGHT
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !ticket.alone_in(&fetches) {
            tracing::debug!(file_id = %file_id, "whole fetch overlapped by another fetch of the file; its candidate stays");
            return Ok(false);
        }
        self.db.record_local_baseline(file_id, fetched.version, &fetched.hash)?;
        drop(fetches);
        Ok(true)
    }

    /// Task 1640 (self-review #1): a fetch of `file_id` starts ([`FetchTicket`]).
    #[cfg_attr(
        not(any(target_os = "windows", any(test, feature = "test-support"))),
        allow(dead_code)
    )]
    pub(super) fn begin_fetch(&self, file_id: &str) -> FetchTicket {
        let mut fetches = FETCHES_IN_FLIGHT
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let activity = fetches.entry(file_id.to_string()).or_default();
        activity.in_flight += 1;
        activity.events += 1;
        let alone_since = (activity.in_flight == 1).then_some(activity.events);
        FetchTicket {
            file_id: file_id.to_string(),
            alone_since,
        }
    }

    /// Core download + decrypt loop. Returns the full plaintext in a
    /// [`Zeroizing`] wrapper so the allocation is wiped on drop regardless of
    /// which exit path is taken (normal return, early `?`, or a panic unwind).
    ///
    /// Intermediate per-chunk buffers from core are
    /// explicitly zeroized after being copied into the accumulator, so at most
    /// one extra chunk's worth of plaintext is live in memory at any time.
    ///
    /// This function deliberately does NOT write anything to disk. Callers that
    /// need the bytes on disk (`hydrate_file`) or in memory (`hydrate_file_to_memory`)
    /// handle that themselves so the disk-write decision stays at the call site,
    /// not buried inside the crypto loop.
    #[cfg(any(test, feature = "test-support"))]
    pub async fn encrypt_owned_name(
        &self,
        file_id: &str,
        name: &str,
        pin: beebeeb_core::chunk_stream::ExpectedChunkFormat,
    ) -> anyhow::Result<String> {
        let key = self.resolved_owned_key(file_id).await?;
        Ok(beebeeb_core::encrypt::encrypt_name_for(&key, name, None, pin)?)
    }

    async fn owned_open_failure(&self, file_id: &str, error: anyhow::Error) -> anyhow::Error {
        if let Ok(Some(record)) = self.db.get_owned_key_record(file_id)
            && record.foreign_added
        {
            let _ = self
                .db
                .set_status_if(file_id, FileStatus::Downloading, FileStatus::CloudOnly);
            let message = crate::owner_keys::cannot_open_message(None, record.member_added);
            return anyhow::anyhow!("{} — {message}", crate::owner_keys::ADDED_BY_SOMEONE_ELSE);
        }
        error
    }

    /// T3.2/P2-3: the content key and the key the served name authenticates
    /// under. For an ordinary row both are the same. A request row's content is
    /// under `C` while a rename by an old client seals the name under the
    /// derived string key, so the chunk-format pin must come from the name key
    /// (A9c).
    pub(super) async fn file_keys_for_download(
        &self,
        file_id: &str,
    ) -> Result<(beebeeb_core::kdf::FileKey, beebeeb_core::kdf::FileKey), EngineError> {
        if self.is_request_row(file_id) {
            let row = self.api.get_file(file_id).await?;
            let found = self.request_row_keys(&row).await.map_err(EngineError::from)?;
            return Ok((found.content_key, found.name_key));
        }
        let key = self.file_key_for_download(file_id).await?;
        // `FileKey` is not `Copy`; the content key keeps this instance.
        let name_key = beebeeb_core::kdf::FileKey::from_bytes(*key.as_bytes());
        Ok((key, name_key))
    }

    pub(super) async fn file_key_for_download(&self, file_id: &str) -> Result<beebeeb_core::kdf::FileKey, EngineError> {
        // T3.2: a request row's content is sealed under the request key `C`, not
        // a derived key. Read-only: the row is never re-sealed or adopted.
        if self.is_request_row(file_id) {
            let row = self.api.get_file(file_id).await?;
            return self.request_content_key(&row).await.map_err(EngineError::from);
        }
        let Some(contract) = self.db.get_file_contract_state(file_id)? else {
            return self
                .resolved_owned_key(file_id)
                .await
                .map(|key| beebeeb_core::kdf::FileKey::from_bytes(*key.as_bytes()))
                .map_err(EngineError::from);
        };
        if contract.namespace != Namespace::SharedWithMe {
            return self
                .resolved_owned_key(file_id)
                .await
                .map(|key| beebeeb_core::kdf::FileKey::from_bytes(*key.as_bytes()))
                .map_err(EngineError::from);
        }
        if !contract.can_read() {
            return Err(anyhow::anyhow!("shared item is not readable").into());
        }

        let share_id = contract
            .share_id
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("shared item is missing invite id"))?;
        let body = self.api.list_shared_roots().await?;
        let invite = body
            .get("invites")
            .and_then(|value| value.as_array())
            .into_iter()
            .flatten()
            .find(|invite| shared_invite_id_matches(invite, share_id))
            .ok_or_else(|| anyhow::anyhow!("shared invite {share_id} was not returned by the server"))?;
        let mapping = shared_root_from_invite(invite)
            .ok_or_else(|| anyhow::anyhow!("shared invite {share_id} is not approved or is malformed"))?;
        if mapping.claimed {
            return Err(anyhow::anyhow!("shared invite {share_id} is still opening: its key has not arrived").into());
        }
        let sender_public_key = mapping
            .sender_public_key
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("shared invite {share_id} is missing sender_public_key"))?;

        if mapping.is_folder {
            let (folder_key, folder_keys) = self.shared_folder_material(&mapping).await?;
            let wrap = encrypted_file_key_from_folder_keys_response(&folder_keys, file_id)
                .ok_or_else(|| anyhow::anyhow!("folder share is missing the child key"))?;
            self.shared_folder_child_key(
                &folder_key,
                &mapping.file_id,
                file_id,
                1,
                wrap,
                KeyCustody::from_folder_keys(&folder_keys),
            )
        } else {
            let encrypted_file_key = mapping
                .encrypted_file_key
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("share invite {share_id} is missing encrypted_file_key"))?;
            unwrap_direct_shared_file_key(self.api.master_key(), sender_public_key, file_id, encrypted_file_key)
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub async fn do_hydrate(
        &self,
        file_id: &str,
        progress: Option<&HydrateProgressFn>,
    ) -> Result<Zeroizing<Vec<u8>>, EngineError> {
        self.do_hydrate_versioned(file_id, progress, None)
            .await
            .map(|(plaintext, _)| plaintext)
    }

    /// [`Self::do_hydrate`], also saying which server version the bytes are: the download
    /// metadata's `download_version.version_number` (task 1640), `None` for a legacy file whose
    /// metadata names no version.
    async fn do_hydrate_versioned(
        &self,
        file_id: &str,
        progress: Option<&HydrateProgressFn>,
        meter: Option<&crate::transfer_progress::ReceiveMeter>,
    ) -> Result<(Zeroizing<Vec<u8>>, Option<i64>), EngineError> {
        let _file_uuid: uuid::Uuid = file_id
            .parse()
            .map_err(|e| anyhow::anyhow!("invalid file_id (not a UUID): {e}"))?;

        // Per-file metadata. We trust the server's chunk_count rather
        // than the local entry's because a file we know about as
        // `cloud_only` may have been re-uploaded with a new chunk
        // layout since we last saw it.
        let meta = self.api.get_file(file_id).await?;
        self.with_one_restart(file_id, meta, |meta| async move {
            let version = meta["download_version"]["version_number"].as_i64();
            self.hydrate_whole_file_metered(file_id, &meta, progress, meter)
                .await
                .map(|(plaintext, _)| (plaintext, version))
        })
        .await
    }

    /// Tasks 1643 + 2173: run one download `attempt` on `meta`, and when the version it was bound to went away
    /// ([`crate::download_pin::VersionGone`]) run it ONCE more on fresh metadata; nothing of the first attempt is kept,
    /// and versions are never mixed. The restart keeps the binding: when the first attempt was pinned and the fresh
    /// metadata names no version (`download_version: null`), it is [`crate::download_pin::PinLost`], never an unpinned
    /// download. The one copy of this rule (whole-file hydrate, range read, conflict preview).
    pub(super) async fn with_one_restart<T, F, Fut>(
        &self,
        file_id: &str,
        meta: serde_json::Value,
        attempt: F,
    ) -> Result<T, EngineError>
    where
        F: Fn(serde_json::Value) -> Fut,
        Fut: std::future::Future<Output = Result<T, EngineError>>,
    {
        let first_was_pinned = crate::download_pin::PinnedVersion::from_metadata(&meta)?.is_some();
        match attempt(meta).await {
            Err(e) if e.is_caused_by::<crate::download_pin::VersionGone>() => {
                tracing::debug!("download: the file changed while it was downloading; restarting once");
                let fresh = self.api.get_file(file_id).await?;
                if first_was_pinned && crate::download_pin::PinnedVersion::from_metadata(&fresh)?.is_none() {
                    return Err(anyhow::Error::new(crate::download_pin::PinLost).into());
                }
                attempt(fresh).await
            }
            other => other,
        }
    }

    /// Shared core of [`Self::do_hydrate`] and [`Self::remote_content_preview`]
    /// downloads + decrypts every chunk
    /// for `file_id` given ALREADY-FETCHED metadata. `remote_content_preview`
    /// needs the metadata anyway to check the remote size before deciding
    /// whether to download at all — routing through this shared helper
    /// instead of `do_hydrate` means that check doesn't cost a second
    /// `GET /files/{id}` round trip for files it does end up downloading.
    ///
    /// Internal helper only — callers are responsible for their own
    /// `file_id` UUID validation (both current callers already do theirs
    /// before this is reached).
    pub(super) async fn do_hydrate_with_meta(
        &self,
        file_id: &str,
        meta: &serde_json::Value,
        progress: Option<&HydrateProgressFn>,
    ) -> Result<Zeroizing<Vec<u8>>, EngineError> {
        self.hydrate_whole_file(file_id, meta, progress)
            .await
            .map(|(plaintext, _)| plaintext)
    }

    /// [`Self::do_hydrate_with_meta`], also saying which transport carried the file. Task 2173:
    /// straight from object storage when the server offers it and the handshake succeeds
    /// ([`crate::direct_download`]); `UseApi` means nothing of that attempt was kept, and the file
    /// goes through the API's per-chunk route below, as it always did.
    ///
    /// Task 1643: when the metadata names a `download_version`, both paths are bound to it: every
    /// API chunk request carries `?object_version_id=`, the direct path must land on the same
    /// version, each chunk's wire size and the final plaintext size are checked against its
    /// manifest, and a version that goes away is [`crate::download_pin::VersionGone`] (the caller
    /// starts again from fresh metadata). Without one (a legacy file), today's unpinned download.
    pub(super) async fn hydrate_whole_file(
        &self,
        file_id: &str,
        meta: &serde_json::Value,
        progress: Option<&HydrateProgressFn>,
    ) -> Result<(Zeroizing<Vec<u8>>, crate::direct_download::FileTransport), EngineError> {
        self.hydrate_whole_file_metered(file_id, meta, progress, None).await
    }

    /// [`Self::hydrate_whole_file`], also counting every body byte into `meter` per read, on the
    /// direct route and the API route alike (LD-0105 S0).
    pub(super) async fn hydrate_whole_file_metered(
        &self,
        file_id: &str,
        meta: &serde_json::Value,
        progress: Option<&HydrateProgressFn>,
        meter: Option<&crate::transfer_progress::ReceiveMeter>,
    ) -> Result<(Zeroizing<Vec<u8>>, crate::direct_download::FileTransport), EngineError> {
        self.hydrate_whole_file_within(file_id, meta, progress, meter, None)
            .await
    }

    /// [`Self::hydrate_whole_file_metered`] for a caller that holds the file in memory and takes at most
    /// `limit` plaintext bytes ([`Self::open_in_memory`]). `None`: no limit, every other hydrate.
    ///
    /// With a limit, a file whose metadata names no version (so no manifest bounds its chunks) goes through
    /// the API's per-chunk route only, every chunk body is given up while it streams once it passes what is
    /// left of the limit, and the running plaintext total is held to the limit after every chunk
    /// ([`OverInMemoryLimit`]). A pinned version needs none of that: its manifest bounds every body and the
    /// total already.
    pub(super) async fn hydrate_whole_file_within(
        &self,
        file_id: &str,
        meta: &serde_json::Value,
        progress: Option<&HydrateProgressFn>,
        meter: Option<&crate::transfer_progress::ReceiveMeter>,
        limit: Option<u64>,
    ) -> Result<(Zeroizing<Vec<u8>>, crate::direct_download::FileTransport), EngineError> {
        let pin = crate::download_pin::PinnedVersion::from_metadata(meta)?;
        let chunk_count = match &pin {
            Some(pin) => pin.chunk_count,
            None => meta
                .get("chunk_count")
                .and_then(|v| v.as_i64())
                .ok_or_else(|| anyhow::anyhow!("server response missing chunk_count"))? as u32,
        };

        let (file_key, name_key) = self.file_keys_for_download(file_id).await?;
        let record = recorded_content_format(&name_key, meta)?;

        // Rate-limit ceiling for downloads, and the transport (task 2173).
        let config = crate::config::DesktopConfig::load_for(self.identity).ok();
        let download_kbps_limit = config.as_ref().map(|c| c.download_kbps_limit).unwrap_or(0);
        let transport = self.download_transport(config.as_ref().and_then(|c| c.transport.as_deref()));
        // A limited download with no manifest: the per-chunk API route alone, where each body is capped.
        let limit = limit.filter(|_| pin.is_none());
        let transport = if limit.is_some() {
            crate::direct_download::Transport::Api
        } else {
            transport
        };

        // Walk chunks. Pre-allocate roughly the file size if known,
        // but fall back to defaults — chunks are encrypted so the
        // ciphertext is always larger than plaintext anyway.
        // Use Zeroizing so that if we bail mid-loop (network error,
        // decrypt error) the partial plaintext is still wiped.
        let approx_size = match &pin {
            Some(pin) => usize::try_from(pin.plaintext_size_bytes).unwrap_or(0),
            None => meta.get("size_bytes").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
        };

        let report = HydrateReport {
            bridge: self,
            file_id,
            total: approx_size as u64,
            progress,
            meter,
            counted: Default::default(),
        };
        // Where this attempt's progress starts: a failed attempt goes back here (lead decision
        // 2026-11-06), so a retried or restarted download never shows more than the file.
        let mark = meter.map(|m| m.kept());
        let rollback = || {
            if let (Some(meter), Some(mark)) = (meter, mark) {
                meter.rollback_to(mark);
            }
        };
        let direct = crate::direct_download::download(
            &self.api,
            crate::direct_download::Request {
                file_id,
                file_key: &file_key,
                chunk_count,
                record,
                download_kbps_limit,
                approx_size,
                pin: pin.as_ref(),
            },
            transport,
            self.storage_override(),
            &report,
        )
        .await
        .map_err(|e| {
            report.discard_attempt();
            rollback();
            let error = match e {
                crate::direct_download::DirectError::VersionChanged => {
                    anyhow::Error::new(crate::download_pin::VersionGone)
                }
                other => anyhow::Error::new(other),
            };
            EngineError::from(error)
        })?;
        let (plaintext, transport) = match direct {
            crate::direct_download::Outcome::Done { plaintext, transport } => (plaintext, transport),
            crate::direct_download::Outcome::UseApi => (
                {
                    // The direct attempt's bytes are not this file's progress any more.
                    report.discard_attempt();
                    rollback();
                    self.download_and_decrypt_chunk_range(
                        file_id,
                        &file_key,
                        0..chunk_count,
                        download_kbps_limit,
                        approx_size,
                        progress,
                        meter,
                        pin.as_ref(),
                        record,
                        chunk_count,
                        limit,
                    )
                    .await
                    .inspect_err(|_| rollback())?
                },
                crate::direct_download::FileTransport::Api,
            ),
        };
        // Task 1643: the whole file is exactly the pinned version's plaintext, or nothing.
        if let Some(pin) = &pin
            && plaintext.len() as u64 != pin.plaintext_size_bytes
        {
            return Err(anyhow::anyhow!(
                "the download is {} bytes, the pinned version's manifest says {}",
                plaintext.len(),
                pin.plaintext_size_bytes
            )
            .into());
        }
        tracing::info!(
            transport = transport.as_str(),
            bytes = plaintext.len() as u64,
            "download finished"
        );
        Ok((plaintext, transport))
    }

    /// The download transport: `BB_TRANSPORT`, else `desktop.toml`'s `transport`, else `auto`
    /// (task 2173). A test bridge takes the one its test set, else `auto`.
    pub(super) fn download_transport(&self, configured: Option<&str>) -> crate::direct_download::Transport {
        #[cfg(any(test, feature = "test-support"))]
        {
            let _ = configured;
            self.seams
                .transport
                .lock()
                .unwrap()
                .unwrap_or(crate::direct_download::Transport::Auto)
        }
        #[cfg(not(any(test, feature = "test-support")))]
        crate::direct_download::Transport::selected(configured)
    }

    /// A test's storage stand-in (task 2173); production always fetches.
    fn storage_override(&self) -> Option<crate::direct_download::Storage<'static>> {
        #[cfg(any(test, feature = "test-support"))]
        if let Some(status) = *self.seams.storage_fail.lock().unwrap() {
            return Some(crate::direct_download::Storage::Fail(status));
        }
        None
    }

    /// Download `file_id` from the vault, decrypt, and return ONLY the plaintext
    /// bytes covering `[required_offset, required_offset + required_length)` —
    /// **in memory only**, never written to disk. This is the range-targeted
    /// counterpart to [`Self::hydrate_file_to_memory`] (task 1024, follow-up to
    /// 0769): instead of decrypting the whole file on every CF fetch callback, it
    /// downloads + decrypts only the chunks that cover the requested range, so
    /// peak memory is bounded by one covering-range buffer (typically a handful
    /// of chunks) rather than the entire file.
    ///
    /// Requires an authoritative, uniform `chunk_size_bytes` for the file. This
    /// value must come from the server's stored `object_versions` row; it cannot
    /// be recovered from `size_bytes / chunk_count`, because real files use a
    /// fixed chunk size with only the final chunk shortened.
    ///
    /// Returns `Ok(None)` (never an error) when the covering-chunk math can't be
    /// established from the metadata (`chunk_count` is `0`, `size_bytes` is
    /// missing, or `chunk_size_bytes` is missing/invalid) — the caller falls
    /// back to
    /// [`Self::hydrate_file_to_memory`] for the whole-file path in that case.
    ///
    /// Status bookkeeping: unlike [`Self::hydrate_file_to_memory`], this method
    /// does **not** flip the row to `FileStatus::Local` — a single range fetch is
    /// not evidence the whole file is now cached, so marking it fully `Local`
    /// here would be a stronger claim than the bytes we actually have. See the
    /// caller (`windows_cf::callbacks::fetch_data_callback`) for how "fully
    /// in-sync" is decided instead.
    #[cfg(any(target_os = "windows", any(test, feature = "test-support")))]
    pub async fn hydrate_file_range(
        &self,
        file_id: &str,
        required_offset: u64,
        required_length: u64,
    ) -> anyhow::Result<Option<HydratedRange>> {
        let _file_uuid: uuid::Uuid = file_id
            .parse()
            .map_err(|e| anyhow::anyhow!("invalid file_id (not a UUID): {e}"))?;

        // Task 1643: the range is planned on, and read from, one pinned version; a version that goes away
        // mid-read starts the read again, once, from fresh metadata (`with_one_restart`).
        let meta = self.api.get_file(file_id).await?;
        let read = self
            .with_one_restart(file_id, meta, |meta| async move {
                self.range_attempt(file_id, required_offset, required_length, &meta)
                    .await
            })
            .await;
        let read = match read {
            Ok(value) => Ok(value),
            Err(e) => Err(self.owned_open_failure(file_id, land_error_to_anyhow(e)).await),
        };
        if read.is_err() {
            // Every failing exit (a second pass's own error too) ends with the row in Error, never left on
            // Downloading, and the placeholder never in sync.
            let _ = self
                .db
                .set_status_if(file_id, FileStatus::Downloading, FileStatus::Error);
        }
        read
    }

    /// One pass of [`Self::hydrate_file_range`] on `meta`.
    #[cfg(any(target_os = "windows", any(test, feature = "test-support")))]
    async fn range_attempt(
        &self,
        file_id: &str,
        required_offset: u64,
        required_length: u64,
        meta: &serde_json::Value,
    ) -> Result<Option<HydratedRange>, EngineError> {
        let pin = crate::download_pin::PinnedVersion::from_metadata(meta)?;
        let (chunk_count, size_bytes, chunk_size_bytes) = match &pin {
            Some(pin) => (
                u64::from(pin.chunk_count),
                Some(pin.plaintext_size_bytes),
                Some(pin.chunk_size_bytes),
            ),
            None => (
                meta.get("chunk_count").and_then(|v| v.as_i64()).unwrap_or(0) as u64,
                meta.get("size_bytes").and_then(|v| v.as_u64()),
                meta.get("chunk_size_bytes").and_then(|v| v.as_u64()),
            ),
        };

        let (Some(size_bytes), Some(chunk_size_bytes), true) = (size_bytes, chunk_size_bytes, chunk_count > 0) else {
            // Missing metadata — caller falls back to the whole-file path.
            return Ok(None);
        };
        if chunk_size_bytes == 0 {
            return Ok(None);
        }

        // Clamp the requested range to the file's real extent before computing
        // covering chunks, so a stale/over-wide CF request never derives an
        // out-of-bounds chunk index.
        let range_end = required_offset.saturating_add(required_length).min(size_bytes);
        if range_end <= required_offset {
            // Degenerate/empty range — nothing to hydrate.
            return Ok(Some(HydratedRange {
                data: Zeroizing::new(Vec::new()),
                range_start_bytes: required_offset,
                covers_whole_file: size_bytes == 0,
            }));
        }

        let Some(plan) = plan_hydration_chunk_range(
            size_bytes,
            chunk_count,
            chunk_size_bytes,
            required_offset,
            required_length,
        )?
        else {
            return Ok(None);
        };

        self.db.set_status(file_id, FileStatus::Downloading)?;

        let (file_key, name_key) = self.file_keys_for_download(file_id).await?;
        let record = recorded_content_format(&name_key, meta)?;
        let download_kbps_limit = crate::config::DesktopConfig::load_for(self.identity)
            .map(|c| c.download_kbps_limit)
            .unwrap_or(0);

        let chunks = plan.first_chunk..plan.last_chunk_exclusive;
        let read = self
            .download_and_decrypt_chunk_range(
                file_id,
                &file_key,
                chunks.clone(),
                download_kbps_limit,
                plan.covering_span_hint,
                None,
                None,
                pin.as_ref(),
                record,
                chunk_count as u32,
                None,
            )
            .await
            .and_then(|data| {
                // Task 1643: exactly the pinned version's plaintext for those chunks.
                match pin.as_ref().map(|pin| pin.plaintext_size_of(chunks.clone())) {
                    Some(want) if want != Some(data.len() as u64) => {
                        Err(anyhow::anyhow!(
                            "the range read is {} bytes, the pinned version's manifest says {want:?}",
                            data.len()
                        )
                        .into())
                    }
                    _ => Ok(data),
                }
            });
        let data = read?;
        // Only a whole-file covering range is evidence the file is fully
        // cached; a partial range leaves the row `Downloading` for the
        // caller (the CF callback) to resolve based on what CF itself
        // reports (see `windows_cf::callbacks`).
        if plan.covers_whole_file {
            self.db.set_status(file_id, FileStatus::Local)?;
            self.db.mark_cached(file_id, "", data.len() as i64, now_secs())?;
            let _ = self.enforce_configured_cache_limit();
        }
        Ok(Some(HydratedRange {
            data,
            range_start_bytes: plan.range_start_bytes,
            covers_whole_file: plan.covers_whole_file,
        }))
    }
}

/// Who starts a hydrate, for its start look and the person's-resolution exemption (S6).
enum HydrateStart {
    /// A caller here and now (the IPC hydrate, Keep both, a conflict's own path): its own start look.
    Here,
    /// Dm-72: a Finder fetch (the File Provider's): as `Here`, and on a Mac its bytes are recorded as Finder's copy.
    Finder,
    /// The queue: its own start look, and never the person's resolution (R7 b).
    Queued,
    /// The caller's start look (Keep there's, taken before its safety copy).
    Seen(PlaceSeen),
}

/// Review P2-1: what the place is (its size, times, device and inode), `None` when nothing is there. Nothing there before
/// and something there now is a change too.
fn place_fingerprint(place: &Path) -> Option<ContentFingerprint> {
    ContentFingerprint::of(place).ok()
}

/// Drive-complete 169: the page Cloud Files requires every transfer but the last to be a multiple of.
/// A stride that is not one cannot be streamed chunk by chunk (each chunk is one transfer's offset).
const STREAM_PAGE: u64 = 4096;

/// Drive-complete 169 (task 1640): the candidate hash a streamed fetch records before its first byte,
/// when the real hash is not known yet. Not hex, so no content ever hashes to it: never promoted.
const STREAMING_HASH: &str = "streaming:not-hashed-yet";

/// Drive-complete 169: the streamed whole-file refetch holds a chunk at a time, never the file.
#[cfg(test)]
mod refetch_streaming_tests {
    use super::*;
    use sha2::Digest;
    use std::io::Write as _;
    use std::sync::Mutex;

    const KEY: [u8; 32] = [69; 32];
    const ID: &str = "169a6c1e-0000-4000-8000-000000000169";
    const VERSION: &str = "169b0000-0000-4000-8000-000000000169";
    const STRIDE: usize = 64 * 1024;

    struct Server {
        base: String,
        chunk_requests: Arc<Mutex<Vec<String>>>,
    }

    /// A loopback API: the file's metadata (pinned or legacy) and its chunks, one connection each.
    #[allow(deprecated)] // the name sealer the other download fixtures use
    fn serve(plain: &[u8], pinned: bool) -> Server {
        let mk = beebeeb_core::kdf::MasterKey::from_bytes(KEY);
        let key = beebeeb_core::kdf::derive_file_key(&mk, ID.as_bytes());
        let frames: Vec<Vec<u8>> = plain
            .chunks(STRIDE)
            .map(|c| {
                let blob = beebeeb_core::encrypt::encrypt_chunk(&key, c).unwrap();
                let mut wire = blob.nonce.clone();
                wire.extend_from_slice(&blob.ciphertext);
                wire
            })
            .collect();
        let manifest = serde_json::json!({
            "object_version_id": VERSION, "version_number": 3, "chunk_count": frames.len(),
            "chunk_size_bytes": STRIDE, "plaintext_size_bytes": plain.len(),
            "ciphertext_size_bytes": frames.iter().map(Vec::len).sum::<usize>(),
            "chunks": frames.iter().enumerate().map(|(i, f)| serde_json::json!({"index": i, "size_bytes": f.len()})).collect::<Vec<_>>(),
        });
        let meta = serde_json::json!({
            "id": ID, "size_bytes": plain.len(), "chunk_count": frames.len(), "chunk_size_bytes": STRIDE,
            "name_encrypted": beebeeb_core::encrypt::encrypt_name(&mk, ID, "big.bin", None).unwrap(),
            "download_version": if pinned { manifest } else { serde_json::Value::Null },
        })
        .to_string();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let chunk_requests = Arc::new(Mutex::new(Vec::new()));
        let seen = chunk_requests.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut head = Vec::new();
                let mut buf = [0u8; 4096];
                while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                    match std::io::Read::read(&mut stream, &mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => head.extend_from_slice(&buf[..n]),
                    }
                }
                let head = String::from_utf8_lossy(&head).to_string();
                let target = head.split_whitespace().nth(1).unwrap_or("").to_string();
                let path = target.split('?').next().unwrap_or("").to_string();
                let (kind, body) = match path.strip_prefix(&format!("/api/v1/files/{ID}/chunks/")) {
                    Some(i) => {
                        seen.lock().unwrap().push(target.clone());
                        ("application/octet-stream", frames[i.parse::<usize>().unwrap()].clone())
                    }
                    None => ("application/json", meta.clone().into_bytes()),
                };
                let mut reply = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .into_bytes();
                reply.extend_from_slice(&body);
                let _ = stream.write_all(&reply);
            }
        });
        Server { base, chunk_requests }
    }

    fn bridge(dir: &Path, server: &Server) -> EngineBridge {
        let db = Arc::new(StateDb::open(dir.join("state.db")).unwrap());
        let api = Arc::new(ApiClient::new(server.base.clone(), "token".into(), KEY));
        EngineBridge::new(&crate::identity::TEST, db, api)
    }

    fn content(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i as u8) ^ ((i >> 8) as u8) ^ 0x69).collect()
    }

    /// What the sink saw: the most plaintext handed over at once, the bytes in order, and their hash.
    #[derive(Default)]
    struct Seen {
        most_at_once: usize,
        pieces: usize,
        next_offset: u64,
        hash: sha2::Sha256,
    }

    fn stream(
        bridge: &EngineBridge,
        file_size: u64,
        seen: &mut Seen,
    ) -> anyhow::Result<Option<Result<FetchedBytes, u64>>> {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(bridge.stream_whole_file(ID, file_size, None, |offset, piece| {
            assert_eq!(offset, seen.next_offset, "pieces arrive in file order, back to back");
            seen.next_offset += piece.len() as u64;
            seen.most_at_once = seen.most_at_once.max(piece.len());
            seen.pieces += 1;
            seen.hash.update(piece);
            Ok(())
        }))
    }

    /// The red test: a file many times the bound is handed over a chunk at a time (the buffered path
    /// held all of it), every byte once and in order, and the hash it records is the whole file's.
    #[test]
    fn a_refetch_hands_over_a_chunk_at_a_time_never_the_whole_file() {
        for pinned in [true, false] {
            let dir = tempfile::tempdir().unwrap();
            let plain = content(40 * STRIDE + 1234);
            let server = serve(&plain, pinned);
            let bridge = bridge(dir.path(), &server);
            let mut seen = Seen::default();
            let fetched = stream(&bridge, plain.len() as u64, &mut seen)
                .unwrap()
                .expect("a chunked file streams")
                .expect("the sizes agree");
            assert!(
                seen.most_at_once <= STRIDE,
                "at most one chunk held: {} bytes",
                seen.most_at_once
            );
            assert_eq!(seen.pieces, 41);
            assert_eq!(seen.next_offset, plain.len() as u64);
            assert!(fetched.bytes.is_empty(), "nothing of the file is returned in memory");
            let whole = format!("{:x}", seen.hash.finalize());
            assert_eq!(whole, sha256_hex(&plain));
            if pinned {
                let version = fetched.version.expect("a pinned version is named");
                assert_eq!((version.version, version.hash.as_str()), (3, whole.as_str()));
            }
            // One download: every chunk fetched once (a legacy file's last chunk first, then reused).
            assert_eq!(server.chunk_requests.lock().unwrap().len(), 41, "pinned: {pinned}");
        }
    }

    /// A row for the served file: cloud-only, at the served version.
    #[cfg(not(target_os = "windows"))]
    fn seed_cloud_only(bridge: &EngineBridge, size: usize) {
        let entry = FileEntry {
            file_id: ID.into(),
            path: "big.bin".into(),
            status: FileStatus::CloudOnly,
            size_bytes: size as i64,
            modified_at: 1,
            content_hash: None,
            remote_updated_at: 1,
            parent_id: None,
            item_kind: ItemKind::File,
        };
        bridge
            .db
            .upsert_file_with_contract(&entry, |contract| {
                contract.item_kind = ItemKind::File;
                contract.current_version = 3;
            })
            .unwrap();
    }

    /// Dm-68: keeping a file on this Mac is ONE download. The pin's own fetch brings the bytes down (the row then
    /// reads `local`, what the Available offline row shows as "On this Mac"); the system's fetch of the same file
    /// that follows (the File Provider's, for the Finder copy) is served from that copy and asks the server for no
    /// chunk. Before, the hydrate of a keep ran twice: 2 x the file's chunks.
    #[cfg(not(target_os = "windows"))]
    #[test]
    fn a_kept_files_hydrate_is_one_download_and_the_finder_fetch_reads_its_copy() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("state-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let plain = content(6 * STRIDE + 321);
        let server = serve(&plain, true);
        let bridge = bridge(dir.path(), &server);
        seed_cloud_only(&bridge, plain.len());
        let rt = tokio::runtime::Runtime::new().unwrap();

        // The keep: the pin queues the file's fetch, the queue step runs it.
        let pinned = bridge.set_recursive_pin(&sync_root, ID, true).unwrap();
        assert_eq!(pinned.hydrate_operations, 1);
        rt.block_on(bridge.process_due_operations(&sync_root, now_secs())).unwrap();
        assert_eq!(bridge.db.get_file(ID).unwrap().unwrap().status, FileStatus::Local);
        assert_eq!(std::fs::read(sync_root.join("big.bin")).unwrap(), plain);
        let downloaded = server.chunk_requests.lock().unwrap().len();
        assert_eq!(downloaded, 7, "the keep's one download: every chunk once");

        // The system's fetch of the kept file: the copy, with its whole size as the progress, and no request.
        let staging = dir.path().join("hydrate-cache");
        std::fs::create_dir_all(&staging).unwrap();
        let dest = staging.join("big.bin");
        let said = Arc::new(Mutex::new(Vec::new()));
        let heard = said.clone();
        let progress = move |done: u64, total: u64| heard.lock().unwrap().push((done, total));
        let source = rt
            .block_on(bridge.serve_hydrate(ID, &dest, &[staging.as_path()], Some(&progress)))
            .unwrap();
        assert_eq!(source, HydrateSource::Copy);
        assert_eq!(std::fs::read(&dest).unwrap(), plain);
        assert_eq!(*said.lock().unwrap(), vec![(plain.len() as u64, plain.len() as u64)]);
        assert_eq!(
            server.chunk_requests.lock().unwrap().len(),
            downloaded,
            "one download per keep: the second fetch read the first's copy"
        );
        assert_eq!(bridge.db.get_file(ID).unwrap().unwrap().status, FileStatus::Local);

        // A copy that is no longer the current version's bytes is never handed over: the server is asked.
        std::fs::write(sync_root.join("big.bin"), b"edited here").unwrap();
        let again = staging.join("again.bin");
        let source = rt
            .block_on(bridge.serve_hydrate(ID, &again, &[staging.as_path()], None))
            .unwrap();
        assert_eq!(source, HydrateSource::Server);
        assert_eq!(std::fs::read(&again).unwrap(), plain);
    }

    /// Dm-71: a keep on a Mac is ONE download and ONE copy. The pin queues no fetch of the engine's own (nothing comes
    /// down, nothing lands in the hidden state folder); the system's fetch (the File Provider's, for Finder's copy) is
    /// the download: every chunk once, its progress on the transfer board the Available offline row reads, to the
    /// whole size, and the row then reads `local` ("On this Mac").
    #[cfg(not(target_os = "windows"))]
    #[test]
    fn a_keep_on_a_mac_is_one_download_by_the_finder_fetch_and_leaves_no_hidden_copy() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("state-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let plain = content(6 * STRIDE + 321);
        let server = serve(&plain, true);
        let bridge = bridge(dir.path(), &server);
        seed_cloud_only(&bridge, plain.len());
        let rt = tokio::runtime::Runtime::new().unwrap();

        // The keep: pinned, and nothing queued for the engine to fetch.
        system_fetch_for_test(true);
        let pinned = bridge.set_recursive_pin(&sync_root, ID, true);
        system_fetch_for_test(false);
        assert_eq!(pinned.unwrap().hydrate_operations, 0);
        rt.block_on(bridge.process_due_operations(&sync_root, now_secs())).unwrap();
        assert_eq!(
            server.chunk_requests.lock().unwrap().len(),
            0,
            "the engine downloads nothing itself"
        );
        assert!(!sync_root.join("big.bin").exists(), "no hidden copy");
        assert_eq!(bridge.db.get_file(ID).unwrap().unwrap().status, FileStatus::CloudOnly);

        // The system's fetch of the kept item: the one download.
        let staging = dir.path().join("hydrate-cache");
        std::fs::create_dir_all(&staging).unwrap();
        let dest = staging.join("big.bin");
        let said = Arc::new(Mutex::new(Vec::new()));
        let on_board = Arc::new(Mutex::new(Vec::new()));
        let (says, rows, board) = (said.clone(), on_board.clone(), bridge.transfers.clone());
        let progress = move |done: u64, total: u64| {
            says.lock().unwrap().push((done, total));
            rows.lock().unwrap().push(board.get(ID).map(|t| t.total));
        };
        let source = rt
            .block_on(bridge.serve_hydrate(ID, &dest, &[staging.as_path()], Some(&progress)))
            .unwrap();
        assert_eq!(source, HydrateSource::Server);
        assert_eq!(
            server.chunk_requests.lock().unwrap().len(),
            7,
            "one download: every chunk once"
        );
        assert_eq!(std::fs::read(&dest).unwrap(), plain);
        let whole = plain.len() as u64;
        assert_eq!(
            said.lock().unwrap().last(),
            Some(&(whole, whole)),
            "progress reaches 100%"
        );
        let on_board = on_board.lock().unwrap();
        assert!(
            !on_board.is_empty() && on_board.iter().all(|row| *row == Some(whole)),
            "the fetch is on the transfer board while it runs: {on_board:?}"
        );
        assert_eq!(bridge.db.get_file(ID).unwrap().unwrap().status, FileStatus::Local);
        assert!(
            !sync_root.join("big.bin").exists(),
            "one copy: the system's, none in the state folder"
        );
    }

    /// Dm-71: one download in flight per file. A Finder fetch that starts while the keep's fetch of the same file
    /// runs asks the server for no chunk: it waits, hears the progress and copies the finished file.
    #[cfg(not(target_os = "windows"))]
    #[test]
    fn a_finder_fetch_started_mid_keep_reuses_the_one_download() {
        let dir = tempfile::tempdir().unwrap();
        let plain = content(6 * STRIDE + 321);
        let server = serve(&plain, true);
        let bridge = bridge(dir.path(), &server);
        seed_cloud_only(&bridge, plain.len());
        let staging = dir.path().join("hydrate-cache");
        std::fs::create_dir_all(&staging).unwrap();
        let (keep, finder) = (staging.join("keep.bin"), staging.join("finder.bin"));
        let roots = [staging.as_path()];
        let heard = Arc::new(Mutex::new(Vec::new()));
        let hears = heard.clone();
        let progress = move |done: u64, total: u64| hears.lock().unwrap().push((done, total));

        let rt = tokio::runtime::Runtime::new().unwrap();
        // Polled in this order: the keep's fetch leads before the second one looks.
        let (first, second) = rt.block_on(async {
            tokio::join!(
                bridge.serve_hydrate(ID, &keep, &roots, None),
                bridge.serve_hydrate(ID, &finder, &roots, Some(&progress)),
            )
        });
        assert_eq!(
            (first.unwrap(), second.unwrap()),
            (HydrateSource::Server, HydrateSource::Copy)
        );
        assert_eq!(server.chunk_requests.lock().unwrap().len(), 7, "0 extra chunk requests");
        assert_eq!(std::fs::read(&keep).unwrap(), plain);
        assert_eq!(std::fs::read(&finder).unwrap(), plain);
        let whole = plain.len() as u64;
        assert_eq!(heard.lock().unwrap().last(), Some(&(whole, whole)));
        assert_eq!(bridge.db.get_file(ID).unwrap().unwrap().status, FileStatus::Local);

        // Nothing is left in flight: the next fetch leads again and downloads.
        let again = staging.join("again.bin");
        let source = rt.block_on(bridge.serve_hydrate(ID, &again, &roots, None)).unwrap();
        assert_eq!(source, HydrateSource::Server);
        assert_eq!(server.chunk_requests.lock().unwrap().len(), 14);
    }

    /// Dm-71: a leader that fails leaves nothing in flight, and the fetch behind it does not wait for ever: it starts
    /// over and answers for itself.
    #[cfg(not(target_os = "windows"))]
    #[test]
    fn a_finder_fetch_behind_one_that_fails_starts_over() {
        let dir = tempfile::tempdir().unwrap();
        let plain = content(2 * STRIDE);
        let server = serve(&plain, true);
        let bridge = bridge(dir.path(), &server);
        seed_cloud_only(&bridge, plain.len());
        let staging = dir.path().join("hydrate-cache");
        std::fs::create_dir_all(&staging).unwrap();
        // The leader's destination cannot be written: a folder is in its place.
        let blocked = staging.join("blocked.bin");
        std::fs::create_dir_all(blocked.join("in-the-way")).unwrap();
        let behind = staging.join("behind.bin");
        let roots = [staging.as_path()];
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (first, second) = rt.block_on(async {
            tokio::join!(
                bridge.serve_hydrate(ID, &blocked, &roots, None),
                bridge.serve_hydrate(ID, &behind, &roots, None),
            )
        });
        assert!(first.is_err());
        assert_eq!(second.unwrap(), HydrateSource::Server);
        assert_eq!(std::fs::read(&behind).unwrap(), plain);
    }

    /// A placeholder whose size is not the file's: nothing is handed over, the real size comes back,
    /// and a pinned file costs no chunk download at all.
    #[test]
    fn a_size_mismatch_is_found_before_any_byte_is_handed_over() {
        for pinned in [true, false] {
            let dir = tempfile::tempdir().unwrap();
            let plain = content(5 * STRIDE + 77);
            let server = serve(&plain, pinned);
            let bridge = bridge(dir.path(), &server);
            let mut seen = Seen::default();
            let encrypted_size = plain.len() as u64 + 6 * 28;
            let outcome = stream(&bridge, encrypted_size, &mut seen)
                .unwrap()
                .expect("a chunked file streams");
            assert!(matches!(outcome, Err(n) if n == plain.len() as u64), "pinned: {pinned}");
            assert_eq!(seen.pieces, 0);
            let chunks = server.chunk_requests.lock().unwrap().len();
            assert_eq!(
                chunks,
                usize::from(!pinned),
                "pinned: no chunk; legacy: the last one only"
            );
        }
    }
}
