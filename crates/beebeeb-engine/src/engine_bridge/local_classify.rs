//! `local_classify` — the `EngineBridge` impl for local path classification and held/tracked edits, split out of `engine_bridge.rs`.

use super::*;

/// RC2-4: at most this many files are checked a tick after a rebuild ([`EngineBridge::check_after_rebuild`]).
pub const REBUILT_CHECKS_PER_TICK: usize = 20;

/// RC2-4: about this many bytes are downloaded a tick for that check (one file at least, whatever its size).
pub const REBUILT_CHECK_BYTES_PER_TICK: u64 = 256 * 1024 * 1024;

/// RC2-4: what one tick of the rebuild's check did. Counts only.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RebuiltCheckPass {
    /// A check is still owed after this tick.
    pub owed: bool,
    /// Files found to be the account's current version: agreed now.
    pub agreed: usize,
    /// Files found different: held, both versions kept.
    pub differing: usize,
    /// Files left for a later tick.
    pub left: usize,
}

enum RebuiltVerdict {
    Agreed,
    Differs,
    Later,
}

impl EngineBridge {
    #[cfg(any(test, feature = "test-support"))]
    pub fn classify_local_path(&self, sync_root: &Path, path: &Path) -> Option<FinderWriteTarget> {
        match self.classify_local_path_verdict(sync_root, path) {
            LocalPathVerdict::Write(target) => Some(target),
            LocalPathVerdict::Settled | LocalPathVerdict::Skip => None,
        }
    }

    /// Single, parent-aware classifier shared by every local-write trigger
    /// (the retired `notify` watcher *and* the Windows Cloud Files NOTIFY
    /// callbacks). Given an absolute on-disk `path` under `sync_root`, it
    /// decides whether `path` is a *genuinely-new* user file that should be
    /// uploaded, and if so returns a ready-to-queue [`FinderWriteTarget`] with
    /// `parent_id` resolved from the path's parent directory.
    ///
    /// It runs the three feedback-loop filters, in order, so a download / a
    /// hydration / an engine-internal write is NEVER mistaken for a new upload:
    ///
    /// 1. **Engine-internal paths + OS junk** — anything under `<root>/.beebeeb/`,
    ///    the `<root>/.beebeeb-sync.lock`, any `.beebeeb` path component, and the
    ///    ignored-name set ([`is_ignored_finder_name`]: `.tmp`, `~$…`, `.DS_Store`,
    ///    …). These are writes the engine itself (or the OS) makes constantly.
    /// 2. **Cloud Files placeholders** (Windows only) — a reparse point under the
    ///    sync root is something WE minted (placeholder seed or post-upload
    ///    convert). Hydration fills a placeholder's data stream WITHOUT clearing
    ///    its reparse-point attribute, so a hydration-write still trips this guard
    ///    and never re-uploads. Checked via
    ///    [`crate::windows_cf::placeholders`]'s `is_cloud_placeholder`.
    /// 3. **Already a known server file** (authoritative) — look the path up in
    ///    the state DB. A row means the file already lives on the server
    ///    (cloud-only / downloading / local / uploading), so a write to it is a
    ///    hydration or a re-download, NOT a new creation. Only a path with NO DB
    ///    row survives.
    ///
    /// `parent_id` resolution (nested uploads): for a survivor at
    /// `<parent_dir>/<name>`, the parent directory's server folder id is looked
    /// up by its server-relative path. A root-level file resolves to `None`; a
    /// nested file resolves to `Some(<parent folder file_id>)` so the upload
    /// lands in the right server folder. If the parent directory has no DB row
    /// yet (its folder placeholder hasn't reconciled), `parent_id` falls back to
    /// `None` rather than guessing — the file uploads to the root and a later
    /// tick can reconcile, which is strictly safer than attaching it to the
    /// wrong parent.
    ///
    /// Returns [`LocalPathVerdict::Skip`] when `path` is not a regular file, is filtered out, or the
    /// DB lookup fails (fail-closed: a lookup error skips the upload rather than
    /// risk a spurious one). Cross-platform so it type-checks everywhere; only
    /// the Windows triggers call it. Task 2229: also tells a tracked file that needs nothing while
    /// its row and its bytes stay as they are ([`LocalPathVerdict::Settled`]).
    pub fn classify_local_path_verdict(&self, sync_root: &Path, path: &Path) -> LocalPathVerdict {
        // The file may have been deleted/renamed during a debounce window — if
        // it is no longer a regular file there is nothing to upload.
        match std::fs::symlink_metadata(path) {
            Ok(m) if m.is_file() => {}
            Ok(m) => {
                let file_type = m.file_type();
                tracing::debug!(
                    is_dir = file_type.is_dir(),
                    is_symlink = file_type.is_symlink(),
                    "classify_local_path: rejected non-regular file"
                );
                return LocalPathVerdict::Skip;
            }
            Err(e) => {
                tracing::debug!(
                    error = %e,
                    "classify_local_path: stat failed; skipping"
                );
                return LocalPathVerdict::Skip;
            }
        }

        // Filter 1 — engine-internal paths + OS junk.
        if path_is_engine_internal(sync_root, path) {
            tracing::debug!("classify_local_path: rejected engine-internal path");
            return LocalPathVerdict::Skip;
        }
        let Some(file_name) = path.file_name().and_then(|n| n.to_str()).map(str::to_string) else {
            tracing::debug!("classify_local_path: rejected path with non-utf8 filename");
            return LocalPathVerdict::Skip;
        };
        if is_ignored_finder_name(&file_name) {
            tracing::debug!("classify_local_path: rejected ignored/temp filename");
            return LocalPathVerdict::Skip;
        }

        // Filter 3 (authoritative) — already a known server file? Then it is not a new
        // upload. Task 1640: it may be an edit of that file; `classify_tracked_edit` decides
        // from the content this device last agreed with the server on, before filter 2, which
        // rejects every placeholder (a synced file is one).
        let Some(rel) = relative_db_path(sync_root, path) else {
            tracing::debug!(
                sync_root = %sync_root.display(),
                "classify_local_path: rejected path outside sync root"
            );
            return LocalPathVerdict::Skip;
        };
        // Task 1640 (review N13): an original whose kept copy is made but which is not removed
        // yet (it was in use) is never sent: a later tick removes it.
        if self.db.is_kept_original_pending(&rel).unwrap_or(false) {
            tracing::debug!("classify_local_path: rejected an original whose copy is kept");
            return LocalPathVerdict::Skip;
        }
        // Task 2204 S0.4: a file at the path a Windows rename vacated waits until that rename is
        // sent: it may be a safe save's new bytes, which fold back onto the original.
        if self.db.is_vacated_by_pending_rename(&rel).unwrap_or(true) {
            tracing::debug!("classify_local_path: the path a pending rename vacated; skipped this pass");
            return LocalPathVerdict::Skip;
        }
        match self.row_judging_bytes_at(&rel, path) {
            Ok(RowAtPath::One(existing)) => return self.classify_tracked_edit(path, existing, file_name, rel),
            Ok(RowAtPath::Ambiguous { file_ids, rows }) => {
                // file ids and the row count, never the path.
                tracing::warn!(
                    ?file_ids,
                    rows,
                    "sync folder: several synced files share one path and its bytes are none of them; kept on this PC, not sent"
                );
                return LocalPathVerdict::Settled;
            }
            Ok(RowAtPath::NotWhollyOnDisk) => {
                tracing::debug!("classify_local_path: several rows at a path not wholly on disk; asked again later");
                return LocalPathVerdict::Skip;
            }
            Ok(RowAtPath::None) => { /* not tracked — filter 2, then new */ }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "classify_local_path: could not tell which row the path's bytes belong to (a state DB lookup or a read of the bytes failed); skipping to be safe"
                );
                return LocalPathVerdict::Skip;
            }
        }

        // Filter 2 — Cloud Files placeholders are engine-owned (Windows only). An
        // untracked reparse point under the sync root is a placeholder we minted or
        // converted whose row is not written yet; the hydration write that fills it
        // does NOT turn it back into a plain file, so we must never treat a
        // placeholder write as a new upload.
        #[cfg(target_os = "windows")]
        if crate::windows_cf::placeholders::is_cloud_placeholder(path) {
            // Task 2191 (D12): an untracked placeholder may be one a remote rename left behind,
            // holding unknown bytes; the tick's retry checks its identity.
            if let Err(e) = self.db.note_untracked_orphan(&rel) {
                tracing::debug!(error = %e, "classify_local_path: could not record an untracked placeholder");
            }
            tracing::debug!("classify_local_path: rejected Cloud Files placeholder");
            return LocalPathVerdict::Skip;
        }

        // parent_id resolution: a survivor at `<parent_dir>/<name>`. If the parent
        // directory maps to a known server FOLDER row, attach the new file to it;
        // otherwise upload at the root (None). We never attach to a row that isn't
        // a folder.
        let parent_id = self.resolve_parent_id_for(sync_root, path);
        tracing::debug!(
            nested = parent_id.is_some(),
            "classify_local_path: accepted new local file"
        );

        LocalPathVerdict::Write(FinderWriteTarget {
            file_id: None,
            parent_id,
            filename: file_name.clone(),
            // The FULL '/'-joined relative key (e.g. `docs/a.txt`), so the row is
            // stored + the upload's target_path is threaded under the same key
            // that filter-3, finalize, and delete all query. `rel` was computed
            // above for the filter-3 lookup; reuse it verbatim.
            rel_path: Some(rel),
            kind: FinderWriteItemKind::File,
            contents_path: Some(path.to_string_lossy().into_owned()),
            content_type: beebeeb_core::media::guess_mime_type(&file_name).map(str::to_string),
            base_version_identifier: None,
        })
    }

    /// Task 2190 (the lead's condition on E3; the Windows test machine's constraints from #167's scan memo): the ONE
    /// call each held branch of [`Self::classify_tracked_edit`] makes. Records that `file_id`'s edit
    /// is held, not sent, with the file's content fingerprint, so the review feed shows it. Nothing
    /// is written while neither the reason nor the fingerprint changed, and the `files` row is never
    /// touched (the memo skips a settled file while that row is unchanged). A failure is logged: the
    /// edit stays held either way.
    /// Task 2244: a hold that writes is logged at DEBUG and counted; the scan logs the count once a
    /// pass, not a line per file (28,224 on every start of 0.8.17 on the maintainer's PC).
    fn hold_unsent_edit(&self, file_id: &str, path: &Path, reason: &str) {
        let fingerprint = ContentFingerprint::of(path)
            .map(|fp| format!("{fp:?}"))
            .unwrap_or_default();
        match self.db.hold_unsent_edit(file_id, reason, &fingerprint, now_secs()) {
            Ok(true) => {
                tracing::debug!(file_id = %file_id, reason, "sync folder: an edit of a synced file is kept on this PC, not sent");
                self.newly_held[usize::from(reason == HELD_BASE_UNKNOWN)].fetch_add(1, Ordering::Relaxed);
            }
            Ok(false) => {}
            Err(e) => tracing::warn!(file_id = %file_id, error = %e, "sync folder: could not record a held edit"),
        }
    }

    /// Task 2244: the edits newly held since the last call, for the scan's one line a pass:
    /// (content unknown, base unknown).
    pub fn take_newly_held(&self) -> (usize, usize) {
        let [content_unknown, base_unknown] = &self.newly_held;
        (
            content_unknown.swap(0, Ordering::Relaxed),
            base_unknown.swap(0, Ordering::Relaxed),
        )
    }

    /// RC2-4: after a corrupt `state.db` was moved aside and a fresh one listed the files again
    /// (`local_state::RebuiltCheck`), the files on disk have rows with no agreed content, so every edit of
    /// one would be held as `content_unknown` (a hold made for files hydrated before 0.8.13, not for a
    /// rebuild). Once the first pull is in, each such file is checked against the server's content: its
    /// current version is downloaded (to memory; nothing is written to disk or to its row on the way) and
    /// hashed, and the bytes on disk are hashed. Equal: that version is recorded as the agreed content and
    /// the hold ends, so a later edit is sent on it. Different: it stays held as a real edit, both versions
    /// kept, and is not downloaded again. Never decided from size or time. At most
    /// [`REBUILT_CHECKS_PER_TICK`] files and about [`REBUILT_CHECK_BYTES_PER_TICK`] bytes a tick; the check
    /// ends (its marker removed) once no file is left to check. A failure leaves the file held, as before.
    pub async fn check_after_rebuild(&self, sync_root: &Path) -> RebuiltCheckPass {
        let mut pass = RebuiltCheckPass::default();
        let Ok(state_dir) = crate::state_paths::beebeeb_state_dir_for(self.identity) else {
            return pass;
        };
        let mut owed = match crate::local_state::RebuiltCheck::read(&state_dir) {
            Ok(Some(owed)) => owed,
            Ok(None) => return pass,
            Err(e) => {
                tracing::warn!(error = %e, "rebuilt file list: the check could not be read; asked again next tick");
                return pass;
            }
        };
        pass.owed = true;
        // The first pull lists the files again; until it is in, no row says what to check.
        if !matches!(self.db.get_sync_cursor(), Ok(Some(_))) {
            return pass;
        }
        let now = now_secs();
        let pulled_at = match owed.pulled_at {
            Some(at) => at,
            None => {
                owed.pulled_at = Some(now);
                if let Err(e) = owed.save(&state_dir) {
                    tracing::warn!(error = %e, "rebuilt file list: the check's start could not be recorded");
                }
                now
            }
        };
        let candidates = match self.rebuilt_check_candidates(sync_root, &owed) {
            Ok(candidates) => candidates,
            Err(e) => {
                tracing::warn!(error = %e, "rebuilt file list: the files to check could not be read; asked again next tick");
                return pass;
            }
        };
        if candidates.is_empty() {
            if now.saturating_sub(pulled_at) < crate::local_state::REBUILT_CHECK_SETTLE_SECS {
                return pass;
            }
            match crate::local_state::RebuiltCheck::finish(&state_dir) {
                Ok(()) => tracing::info!(
                    differing = owed.differing.len(),
                    "rebuilt file list: every file on this PC is checked against your account"
                ),
                Err(e) => tracing::warn!(error = %e, "rebuilt file list: the finished check could not be cleared"),
            }
            pass.owed = false;
            return pass;
        }
        let mut bytes_spent: u64 = 0;
        for (file_id, path, size) in candidates {
            if pass.agreed + pass.differing >= REBUILT_CHECKS_PER_TICK
                || (bytes_spent > 0 && bytes_spent.saturating_add(size) > REBUILT_CHECK_BYTES_PER_TICK)
            {
                pass.left += 1;
                continue;
            }
            if self.stopping.load(Ordering::SeqCst) {
                pass.left += 1;
                continue;
            }
            if size > REBUILT_CHECK_BYTES_PER_TICK {
                // Too large to hold in memory for the check: never downloaded; it stays held, as before.
                owed.differing.insert(file_id);
                pass.differing += 1;
                continue;
            }
            bytes_spent = bytes_spent.saturating_add(size);
            match self.check_one_after_rebuild(&file_id, &path).await {
                Ok(RebuiltVerdict::Agreed) => pass.agreed += 1,
                Ok(RebuiltVerdict::Differs) => {
                    owed.differing.insert(file_id);
                    pass.differing += 1;
                }
                Ok(RebuiltVerdict::Later) => pass.left += 1,
                Err(e) => {
                    tracing::warn!(file_id = %file_id, error = %e, "rebuilt file list: a file could not be checked; it stays held and is checked again");
                    pass.left += 1;
                }
            }
        }
        if pass.differing > 0
            && let Err(e) = owed.save(&state_dir)
        {
            tracing::warn!(error = %e, "rebuilt file list: the files found changed could not be recorded; they are checked again");
        }
        tracing::info!(
            agreed = pass.agreed,
            differing = pass.differing,
            left = pass.left,
            "rebuilt file list: files on this PC checked against your account"
        );
        pass
    }

    /// RC2-4: the files a rebuild's check still owes, `(file_id, absolute path, size on disk)`, oldest row
    /// first: synced (`Local`) files with no agreed content and no fetched candidate (held as
    /// `content_unknown`, or not classified yet), wholly on disk, not already found differing.
    fn rebuilt_check_candidates(
        &self,
        sync_root: &Path,
        owed: &crate::local_state::RebuiltCheck,
    ) -> Result<Vec<(String, PathBuf, u64)>, EngineError> {
        let mut ids: Vec<String> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        for (file_id, _, reason, _) in self.db.list_held_edits()? {
            if reason == HELD_CONTENT_UNKNOWN && seen.insert(file_id.clone()) {
                ids.push(file_id);
            }
        }
        for row in self.db.list_guarded()? {
            if row.status == FileStatus::Local && seen.insert(row.file_id.clone()) {
                ids.push(row.file_id);
            }
        }
        let mut out = Vec::new();
        for file_id in ids {
            if owed.differing.contains(&file_id) {
                continue;
            }
            let Some(row) = self.db.get_file(&file_id)? else {
                continue;
            };
            if row.is_dir() || row.status != FileStatus::Local {
                continue;
            }
            let Ok(path) = local_file_path_under_sync_root(sync_root, &row.path) else {
                continue;
            };
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if !meta.is_file() {
                continue;
            }
            #[cfg(target_os = "windows")]
            if !crate::windows_cf::placeholders::fully_on_disk(&path) {
                // Nothing of a file not wholly on disk can be compared; it is not this check's.
                continue;
            }
            if !matches!(
                sync_folder_content(&self.db, &file_id, &path, None)?,
                SyncFolderContent::Unknown
            ) {
                continue;
            }
            out.push((file_id, path, meta.len()));
        }
        Ok(out)
    }

    /// RC2-4: one file of [`Self::check_after_rebuild`].
    async fn check_one_after_rebuild(&self, file_id: &str, path: &Path) -> Result<RebuiltVerdict, EngineError> {
        // The row's version, read before the download, as a legacy download records it: it can say less
        // than the bytes are (a later edit then parks as stale), never more.
        let legacy_version = self.legacy_download_version(file_id)?;
        let meta = self.api.get_file(file_id).await?;
        let Some(version) = meta["download_version"]["version_number"].as_i64().or(legacy_version) else {
            // No version can be named: never agreed; it stays held.
            return Ok(RebuiltVerdict::Differs);
        };
        let (plaintext, _) = self.hydrate_whole_file(file_id, &meta, None).await?;
        let server_hash = sha256_hex(&plaintext);
        drop(plaintext);
        let disk_hash = sha256_file_hex(path)?;
        if disk_hash != server_hash {
            tracing::debug!(file_id = %file_id, "rebuilt file list: a file on this PC differs from your account's; it stays held, both kept");
            return Ok(RebuiltVerdict::Differs);
        }
        // Still a synced file with no agreed content (nothing landed, queued or fetched meanwhile).
        let still = self
            .db
            .get_file(file_id)?
            .is_some_and(|row| row.status == FileStatus::Local && !row.is_dir());
        if !still
            || !matches!(
                sync_folder_content(&self.db, file_id, path, None)?,
                SyncFolderContent::Unknown
            )
        {
            return Ok(RebuiltVerdict::Later);
        }
        // A change of the bytes after they were hashed reads as an edit on this version: sent, never lost.
        self.db.record_local_baseline(file_id, version, &server_hash)?;
        self.release_unsent_edit(file_id);
        tracing::debug!(file_id = %file_id, version, "rebuilt file list: a file on this PC is your account's current version");
        Ok(RebuiltVerdict::Agreed)
    }

    /// Task 2190: the one call that ends a held edit's record, when the file reads as agreed, is
    /// queued, or can be sent on a known base.
    fn release_unsent_edit(&self, file_id: &str) {
        if let Err(e) = self.db.release_unsent_edit(file_id) {
            tracing::warn!(file_id = %file_id, error = %e, "sync folder: could not clear a held edit");
        }
    }

    /// Task 2271: the row the scan judges the bytes at `rel` by. Normally the one row there. When
    /// several share the path (0.8.18's reconcile pass left the server's file of that name and the
    /// copy this PC sent at one path), the row that records content this PC agreed on decides: the
    /// only such row, or the one whose agreed content the bytes are. With no such row it is the
    /// path's first, as before. With several and bytes that are none of them, no row is guessed.
    /// Choosing among several reads the bytes once, and only when they are wholly on this PC.
    fn row_judging_bytes_at(&self, rel: &str, path: &Path) -> Result<RowAtPath, EngineError> {
        let mut rows = self.db.file_rows_at_path(rel)?;
        if rows.len() < 2 {
            // Task 2230: a lone `Trashing` row answers only for the file it records; bytes of
            // another size or hash are a file re-created at the path before the trash landed.
            if let Some(row) = rows.first()
                && row.status == FileStatus::Trashing
                && !row.is_dir()
            {
                let recreated = match std::fs::metadata(path) {
                    Ok(m) if m.len() as i64 != row.size_bytes => true,
                    Ok(_) => match &row.content_hash {
                        Some(recorded) => {
                            #[cfg(target_os = "windows")]
                            if !crate::windows_cf::placeholders::fully_on_disk(path) {
                                return Ok(RowAtPath::NotWhollyOnDisk);
                            }
                            cached_sha256_file_hex(&row.file_id, path)? != *recorded
                        }
                        None => false,
                    },
                    Err(_) => false,
                };
                if recreated {
                    return Ok(RowAtPath::None);
                }
            }
            return Ok(rows.pop().map_or(RowAtPath::None, RowAtPath::One));
        }
        let mut recorded = Vec::new();
        for (at, row) in rows.iter().enumerate() {
            if row.is_dir() {
                continue;
            }
            let agreed = match self
                .db
                .get_file_contract_state(&row.file_id)?
                .and_then(|c| c.local_hash)
            {
                Some(hash) => Some(Some(hash)),
                None => self.db.fetched_content(&row.file_id)?.map(|(_, hash)| hash),
            };
            if let Some(agreed) = agreed {
                recorded.push((at, agreed));
            }
        }
        let at = match recorded[..] {
            [] => 0,
            [(at, _)] => at,
            [(first, _), ..] => {
                // Windows: reading a placeholder that is not wholly on disk would fetch the rest of
                // it (as in `classify_tracked_edit`); the scan asks again once it is.
                #[cfg(target_os = "windows")]
                if !crate::windows_cf::placeholders::fully_on_disk(path) {
                    return Ok(RowAtPath::NotWhollyOnDisk);
                }
                let bytes = cached_sha256_file_hex(&rows[first].file_id, path)?;
                // A mixed fetch (no single version) is never agreed.
                let matching: Vec<usize> = recorded
                    .iter()
                    .filter(|(_, agreed)| agreed.as_deref() == Some(bytes.as_str()))
                    .map(|(at, _)| *at)
                    .collect();
                match matching[..] {
                    [at] => at,
                    _ => {
                        let file_ids = recorded.iter().map(|(at, _)| rows[*at].file_id.clone()).collect();
                        return Ok(RowAtPath::Ambiguous {
                            file_ids,
                            rows: rows.len(),
                        });
                    }
                }
            }
        };
        Ok(RowAtPath::One(rows.swap_remove(at)))
    }

    /// Task 1640: a close or scan of a tracked sync-folder file is an edit when the file is a
    /// synced (`Local`) file and its bytes differ from the content this device last agreed with
    /// the server on. The edit is a content modify of that file, sent with the version its bytes
    /// are based on, so the server refuses it as stale when the file changed elsewhere meanwhile
    /// (it then parks with its bytes). Anything else is not an upload: a folder, a row in
    /// transfer or in conflict, agreed bytes, an upload already queued, no agreed content
    /// recorded, or an edit whose base is not known (never sent without one: it would replace
    /// the server's version unseen).
    fn classify_tracked_edit(
        &self,
        path: &Path,
        existing: FileEntry,
        file_name: String,
        rel: String,
    ) -> LocalPathVerdict {
        if existing.is_dir() || existing.status != FileStatus::Local {
            tracing::debug!(
                file_id = %existing.file_id,
                "classify_local_path: rejected already-tracked server file"
            );
            return LocalPathVerdict::Settled;
        }
        // Windows: reading a placeholder that is not wholly on disk would fetch the rest of it.
        #[cfg(target_os = "windows")]
        let os_in_sync = {
            if !crate::windows_cf::placeholders::fully_on_disk(path) {
                return LocalPathVerdict::Skip;
            }
            crate::windows_cf::placeholders::placeholder_in_sync(path)
        };
        #[cfg(not(target_os = "windows"))]
        let os_in_sync = None;
        let base = match sync_folder_content(&self.db, &existing.file_id, path, os_in_sync) {
            Ok(SyncFolderContent::Changed { base: Some(base) }) => {
                self.release_unsent_edit(&existing.file_id);
                base
            }
            Ok(SyncFolderContent::Changed { base: None }) => {
                self.hold_unsent_edit(&existing.file_id, path, HELD_BASE_UNKNOWN);
                // Task 2229: held until its row records a base; no change of its bytes sends it.
                return LocalPathVerdict::Settled;
            }
            // A file hydrated before this build has no agreed content: its change is kept on
            // disk (out of sync, so never stamped or freed) but not sent (task 2191).
            // Task 2190: not reported in sync (Windows), or not a placeholder (an atomic save left a
            // plain file; and every build without Cloud Files) may hold an edit.
            Ok(SyncFolderContent::Unknown) if os_in_sync != Some(true) => {
                self.hold_unsent_edit(&existing.file_id, path, HELD_CONTENT_UNKNOWN);
                return LocalPathVerdict::Settled;
            }
            // Task 2229: unknown content is held until its row records the agreed content, whatever
            // its bytes and its in-sync state; the scan remembers it like agreed bytes.
            Ok(SyncFolderContent::Agreed | SyncFolderContent::Unknown) => {
                self.release_unsent_edit(&existing.file_id);
                return LocalPathVerdict::Settled;
            }
            Ok(SyncFolderContent::Queued) => {
                self.release_unsent_edit(&existing.file_id);
                return LocalPathVerdict::Skip;
            }
            Err(e) => {
                tracing::warn!(file_id = %existing.file_id, error = %e, "sync folder: could not compare a synced file with its agreed content");
                return LocalPathVerdict::Skip;
            }
        };
        let contract = self.db.get_file_contract_state(&existing.file_id).ok().flatten();
        tracing::debug!(
            file_id = %existing.file_id,
            base_version = base,
            "classify_local_path: accepted an edit of a synced file"
        );
        LocalPathVerdict::Write(FinderWriteTarget {
            file_id: Some(existing.file_id),
            parent_id: contract.as_ref().and_then(|contract| contract.parent_id.clone()),
            content_type: contract
                .and_then(|contract| contract.content_type)
                .or_else(|| beebeeb_core::media::guess_mime_type(&file_name).map(str::to_string)),
            filename: file_name,
            rel_path: Some(rel),
            kind: FinderWriteItemKind::File,
            contents_path: Some(path.to_string_lossy().into_owned()),
            base_version_identifier: Some(base.to_string()),
        })
    }
}
