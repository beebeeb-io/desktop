//! `local_layout` — the `EngineBridge` impl for local folders, pins, the local cache and Linux thumbnails, split out of `engine_bridge.rs`.

use super::*;

impl EngineBridge {
    /// Task 1640 (review N1b): the row's version (`None` when unknown), the version a legacy
    /// download records. Accepted for legacy files only: it can say less than the bytes are
    /// (a later change then parks as stale), never more.
    pub(super) fn legacy_download_version(&self, file_id: &str) -> Result<Option<i64>, EngineError> {
        Ok(self
            .db
            .get_file_contract_state(file_id)?
            .map(|contract| contract.current_version)
            .filter(|version| *version > 0))
    }

    /// Task 1640: whether `dest` is `file_id`'s own place in the sync folder, under one of
    /// `allowed_roots`, rather than a staging copy (a Finder fetch, a Linux FUSE temp file).
    pub(super) fn dest_is_files_sync_folder_place(&self, file_id: &str, dest: &Path, allowed_roots: &[&Path]) -> Result<bool, EngineError> {
        let Some(entry) = self.db.get_file(file_id)? else {
            return Ok(false);
        };
        Ok(allowed_roots.iter().any(|root| {
            local_file_path_under_sync_root(root, &entry.path).is_ok_and(|place| place.as_path() == dest)
        }))
    }

    /// Resolve the server folder id of `path`'s immediate parent directory, for
    /// nested uploads. Returns `None` for a root-level file (parent == sync root)
    /// or when the parent directory has no known server FOLDER row yet. The
    /// parent's server-relative key is built in the same '/'-joined shape the
    /// state DB stores, then looked up via [`StateDb::get_file_by_path`]; a hit
    /// that is a folder yields its `file_id`.
    ///
    /// `pub(crate)` so the upload driver's rename handler can resolve the NEW
    /// parent of a moved file through the exact same logic.
    pub(crate) fn resolve_parent_id_for(&self, sync_root: &Path, path: &Path) -> Option<String> {
        let parent_dir = path.parent()?;
        // Root-level file: parent IS the sync root → no server parent.
        if parent_dir == sync_root {
            return None;
        }
        let parent_rel = relative_db_path(sync_root, parent_dir)?;
        match self.db.get_file_by_path(&parent_rel) {
            Ok(Some(entry)) if entry.is_dir() => Some(entry.file_id),
            // No row, or a row that is somehow a file (shouldn't happen for a
            // directory path) → upload at the root rather than mis-parent.
            _ => None,
        }
    }

    /// Ensure the on-disk DIRECTORY at `path` (under `sync_root`) has a server
    /// vault folder, returning its `file_id`. This is the folder-hierarchy half
    /// of the local-create pipeline (task 0811): the enumeration scan only
    /// uploaded *files*, so a freshly-mirrored nested tree
    /// (`Backup/<device>/<folder>/<sub>/file`) had no folder rows — every nested
    /// file's `resolve_parent_id_for` missed and the file uploaded FLAT to the
    /// vault root. Scaffolding the folder first (parents before children, which
    /// the top-down walk already guarantees) gives each child a real parent to
    /// attach to, so the server vault mirrors the on-disk hierarchy.
    ///
    /// Idempotent + parent-aware:
    /// - If a DB row already exists for this directory's relative key, returns its
    ///   `file_id` (a folder row, or `None` if it is somehow a file row).
    /// - Otherwise mints a client folder id, writes a local `Folder` row + contract
    ///   IMMEDIATELY (so a child file dispatched later in the SAME scan resolves
    ///   this parent without waiting for a server round-trip), and enqueues a
    ///   `CreateFolder` op carrying that same id — the server honours the
    ///   client-supplied `folder_id`, so the local row and the server folder share
    ///   one id and the parent linkage is consistent.
    ///
    /// The op's `target_path` is the folder's relative key, so a backup folder is
    /// auto-tagged with its origin key (Part C) and purged on disable. Returns
    /// `None` for the sync root itself, a non-directory, or on a DB error
    /// (fail-closed: the file then uploads at the root rather than mis-parenting).
    pub fn ensure_local_folder(&self, sync_root: &Path, path: &Path) -> Option<String> {
        // Root itself has no server folder.
        if path == sync_root {
            return None;
        }
        match std::fs::symlink_metadata(path) {
            Ok(m) if m.is_dir() => {}
            _ => return None,
        }
        if path_is_engine_internal(sync_root, path) {
            return None;
        }
        let name = path.file_name().and_then(|n| n.to_str())?.to_string();
        if is_ignored_finder_name(&name) {
            return None;
        }
        let rel = relative_db_path(sync_root, path)?;

        // Already known? Return the existing folder id (idempotent across scans).
        match self.db.get_file_by_path(&rel) {
            Ok(Some(entry)) if entry.is_dir() => return Some(entry.file_id),
            Ok(Some(_)) => return None, // a file row at a dir path — never mis-parent
            Ok(None) => { /* mint below */ }
            Err(e) => {
                tracing::warn!(error = %e, "ensure_local_folder: DB lookup failed; skipping");
                return None;
            }
        }

        // Task 1640 (review F3): a folder deleted elsewhere that stays on disk only for what this PC
        // keeps (an original still open, a file partly on disk) is never created again on the
        // server; one the person put something new into is wanted again.
        match self.db.outermost_removed_folder(&rel) {
            Ok(Some(outermost)) => {
                let held = Self::placeholder_path_under(sync_root, &outermost)
                    .is_some_and(|dir| held_only_for_kept(&self.db, sync_root, &dir, 0));
                if held {
                    tracing::debug!("ensure_local_folder: a folder deleted elsewhere, kept for what it holds; not created again");
                    return None;
                }
                let _ = self.db.forget_removed_folder(&outermost);
            }
            Ok(None) => {}
            Err(e) => {
                tracing::warn!(error = %e, "ensure_local_folder: kept-folder lookup failed; skipping");
                return None;
            }
        }

        // Parent linkage: resolve THIS directory's parent dir to its folder id
        // (None at the Backup root level — a top-level folder under the vault).
        let parent_id = self.resolve_parent_id_for(sync_root, path);
        let folder_id = uuid::Uuid::new_v4().to_string();

        // Delegate the row write + contract + CreateFolder enqueue to the one
        // shared create path, carrying our pre-minted id (which the server
        // honours) so the local row and the server folder share an id. The
        // `rel`-path `target_path` auto-tags a backup folder with its origin key.
        let target = FinderWriteTarget {
            file_id: Some(folder_id.clone()),
            parent_id,
            filename: name,
            rel_path: Some(rel),
            kind: FinderWriteItemKind::Folder,
            contents_path: None,
            content_type: None,
            base_version_identifier: None,
        };
        match self.queue_finder_create(target) {
            Ok(FinderWriteOutcome::Queued { op_id, .. }) => {
                tracing::info!(op_id = %op_id, folder_id = %folder_id, "upload driver: scaffolded vault folder for nested upload");
                Some(folder_id)
            }
            Ok(FinderWriteOutcome::Ignored { .. }) => None,
            Err(e) => {
                tracing::warn!(error = %e, "ensure_local_folder: failed to scaffold vault folder");
                None
            }
        }
    }

    /// Apply a pin ("available offline") toggle to `file_id` and its subtree.
    ///
    /// `sync_root` is the on-disk root of the vault — needed on Windows to
    /// resolve each affected DB row to its placeholder path and set the OS-level
    /// pin state. It is unused on macOS/Linux (the `#[cfg]` below silences the
    /// warning) but threaded in unconditionally to keep one signature.
    ///
    /// Three things happen on a pin change:
    /// 1. **DB** — `db.set_recursive_pin` flips the `pin_state` column for the
    ///    whole subtree and returns the ids whose state actually changed.
    /// 2. **Server** — `enqueue_pin_tree_operation` propagates the pin so OTHER
    ///    devices learn about it. Always runs (every platform).
    /// 3. **OS hydration** —
    ///    - **Windows**: `CfSetPinState` is the OS contract. A pinned placeholder
    ///      is hydrated by Windows via the normal FETCH_DATA path AND is exempt
    ///      from auto-dehydration, so it stays resident. We therefore do NOT
    ///      enqueue our own out-of-band hydrate ops (the old DB-only model did,
    ///      because Windows was never told about the pin — that loop is dropped on
    ///      Windows). New children inherit the parent pin via CF_PIN_STATE_INHERIT.
    ///    - **macOS/Linux**: there is no CF pin primitive, so we keep enqueuing an
    ///      explicit hydrate op per newly-pinned cloud-only file to materialise it.
    pub fn set_recursive_pin(
        &self,
        #[cfg_attr(not(target_os = "windows"), allow(unused_variables))] sync_root: &Path,
        file_id: &str,
        pinned: bool,
    ) -> anyhow::Result<PinUpdateOutcome> {
        let now = now_secs();
        let toggle = PinToggle::from_flag(pinned);
        let changed_item_ids = self.db.set_recursive_pin(file_id, pinned, now)?;
        // Mutated only on the non-Windows hydrate-enqueue path below; on Windows
        // the OS (CfSetPinState) drives hydration, so this stays 0.
        #[cfg_attr(target_os = "windows", allow(unused_mut))]
        let mut hydrate_operations = 0usize;

        self.enqueue_pin_tree_operation(file_id, toggle, now)?;

        #[cfg(target_os = "windows")]
        {
            // OS-level pin: tell Windows the actual "available offline" state so a
            // pinned file is kept resident (no auto-dehydration) and an unpinned
            // one becomes reclaim-eligible again. We pin/unpin only the TOP item
            // the user toggled, with RECURSE when it is a directory — the
            // recursive flag stamps existing descendants in one call and
            // CF_PIN_STATE_INHERIT covers descendants created later — rather than
            // issuing a per-row CfSetPinState for every changed id.
            if let Some(top) = self.db.get_file(file_id)? {
                if let Some(path) = Self::placeholder_path_under(sync_root, &top.path) {
                    let recurse = top.is_dir();
                    if let Err(e) = crate::windows_cf::placeholders::set_pin_state(&path, toggle.is_pin(), recurse) {
                        // Zero-knowledge: never log the path. A pin-state failure
                        // must not abort the DB+server pin that already succeeded;
                        // surface it as a log line and continue.
                        tracing::warn!(file_id = %file_id, pinned = toggle.is_pin(), recurse, error = %e, "CfSetPinState failed for pin toggle");
                    }
                }

                // Proactive hydrate. CfSetPinState(PINNED) marks the subtree
                // pinned but Windows does NOT eagerly download a not-yet-opened
                // placeholder — a pinned-but-unopened file stays cloud-only
                // (RECALL_ON_DATA_ACCESS) until something reads it. So "available
                // offline" is only real once we force the download. Collect the
                // cloud-only file descendants (the toggled item itself if it is a
                // cloud-only file, or every cloud-only file under it if a folder)
                // and CfHydratePlaceholder each. We only ever hydrate on pin, not
                // unpin (unpinning just makes files reclaim-eligible again).
                if toggle.is_pin() {
                    match self.db.cloud_only_file_descendants(file_id) {
                        Ok(entries) if !entries.is_empty() => {
                            // Resolve placeholder paths the SAME way the pin does,
                            // dropping any that fail the containment guard.
                            let paths: Vec<PathBuf> = entries
                                .iter()
                                .filter_map(|e| Self::placeholder_path_under(sync_root, &e.path))
                                .collect();
                            // CfHydratePlaceholder BLOCKS until each file's bytes
                            // are on disk, and a folder may hold many/large files,
                            // so run the whole sweep OFF the IPC thread — set_recursive_pin
                            // returns immediately while hydration proceeds in the
                            // background. A per-file failure is logged and never
                            // aborts the rest of the sweep.
                            let total = paths.len();
                            std::thread::spawn(move || {
                                tracing::debug!(count = total, "proactive pin hydrate: starting");
                                let mut ok = 0usize;
                                for path in &paths {
                                    match crate::windows_cf::placeholders::hydrate_placeholder(path) {
                                        Ok(()) => ok += 1,
                                        // Zero-knowledge: never log the path.
                                        Err(e) => tracing::warn!(error = %e, "proactive pin hydrate: file failed"),
                                    }
                                }
                                tracing::debug!(hydrated = ok, total, "proactive pin hydrate: finished");
                            });
                        }
                        Ok(_) => {} // nothing cloud-only to hydrate
                        Err(e) => {
                            // A query failure must not abort the DB+server pin that
                            // already succeeded; log and continue.
                            tracing::warn!(file_id = %file_id, error = %e, "proactive pin hydrate: descendant query failed");
                        }
                    }
                }
            }
        }

        #[cfg(not(target_os = "windows"))]
        if toggle.is_pin() {
            for changed_id in &changed_item_ids {
                let Some(entry) = self.db.get_file(changed_id)? else {
                    continue;
                };
                let Some(contract) = self.db.get_file_contract_state(changed_id)? else {
                    continue;
                };
                if contract.effective_pin_state() == crate::state_db::PinState::Pinned
                    && entry.status == FileStatus::CloudOnly
                    && entry.size_bytes > 0
                {
                    self.enqueue_hydrate_operation(&entry, now)?;
                    hydrate_operations += 1;
                }
            }
        }

        Ok(PinUpdateOutcome {
            changed_item_ids,
            hydrate_operations,
        })
    }

    /// Map a server-relative, '/'-joined DB key (`FileEntry::path`) to its on-disk placeholder path under
    /// `sync_root`. Task 2413: [`crate::stored_path::path_under`], the one map every caller shares (the Windows fetch
    /// fallback's `safe_join_under_root` and the placeholder pass included): it splits on both `/` and `\`, refuses
    /// any segment that is not one plain Windows name, and checks containment by component. `None` for an empty key.
    #[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
    pub(crate) fn placeholder_path_under(sync_root: &Path, rel_path: &str) -> Option<PathBuf> {
        crate::stored_path::path_under(sync_root, rel_path)
    }

    pub fn enforce_smart_cache(&self, policy: CachePolicy) -> anyhow::Result<CacheCleanupOutcome> {
        let evicted = self
            .db
            .evict_unpinned_cache_until_under(policy.max_unpinned_cache_bytes, now_secs())?;
        #[cfg(target_os = "linux")]
        self.remove_linux_freedesktop_thumbnails_for_file_ids(&evicted);
        Ok(CacheCleanupOutcome {
            evicted_file_ids: evicted,
        })
    }

    pub fn enforce_local_cache_limit(&self, max_local_cache_bytes: Option<i64>) -> anyhow::Result<CacheCleanupOutcome> {
        self.enforce_local_cache_limit_after_guard(max_local_cache_bytes, &crate::unknown_check::STARTUP_GUARD)
            .map_err(land_error_to_anyhow)
    }

    /// Desktop 0.8.15: [`Self::enforce_local_cache_limit`], which evicts nothing until `guard`'s
    /// start-up pass has walked every file of unknown content (the pass runs beside the engine).
    pub(crate) fn enforce_local_cache_limit_after_guard(
        &self,
        max_local_cache_bytes: Option<i64>,
        guard: &crate::unknown_check::StartupGuard,
    ) -> Result<CacheCleanupOutcome, EngineError> {
        if !guard.allows_freeing() {
            return Ok(CacheCleanupOutcome {
                evicted_file_ids: Vec::new(),
            });
        }
        let Some(max_local_cache_bytes) = max_local_cache_bytes else {
            return Ok(CacheCleanupOutcome {
                evicted_file_ids: Vec::new(),
            });
        };
        if max_local_cache_bytes <= 0 {
            return Ok(CacheCleanupOutcome {
                evicted_file_ids: Vec::new(),
            });
        }

        let pinned_bytes = self.db.cache_bytes_by_effective_pin(true)?.max(0);
        let max_unpinned_cache_bytes = max_local_cache_bytes.saturating_sub(pinned_bytes);
        self.enforce_smart_cache(CachePolicy {
            max_unpinned_cache_bytes,
            ..CachePolicy::default()
        })
            .map_err(EngineError::from)
    }

    pub fn local_cache_usage_bytes(&self) -> anyhow::Result<i64> {
        let pinned_bytes = self.db.cache_bytes_by_effective_pin(true)?.max(0);
        let unpinned_bytes = self.db.cache_bytes_by_effective_pin(false)?.max(0);
        Ok(pinned_bytes.saturating_add(unpinned_bytes))
    }

    pub fn enforce_configured_cache_limit(&self) -> anyhow::Result<CacheCleanupOutcome> {
        let cfg = crate::config::DesktopConfig::load().unwrap_or_default();
        self.enforce_local_cache_limit(cfg.local_cache_limit_for_eviction())
    }

    #[cfg(target_os = "linux")]
    pub(super) async fn write_linux_freedesktop_thumbnails(&self, file_id: &str, dest_path: &Path) {
        let entry = match self.db.get_file(file_id) {
            Ok(Some(entry)) if !entry.is_dir() => entry,
            Ok(_) => return,
            Err(error) => {
                tracing::debug!(file_id = %file_id, error = %error, "linux thumbnail skipped: state row unavailable");
                return;
            }
        };

        let source_path = linux_thumbnail_source_path_for_entry(&entry).unwrap_or_else(|| dest_path.to_path_buf());
        let thumbnail = match tokio::time::timeout(
            Duration::from_secs(8),
            self.fetch_thumbnail_to_memory(file_id, "medium"),
        )
        .await
        {
            Ok(Ok(bytes)) => bytes,
            Ok(Err(error)) => {
                tracing::debug!(file_id = %file_id, error = %error, "linux thumbnail skipped: server thumbnail unavailable");
                return;
            }
            Err(_) => {
                tracing::debug!(file_id = %file_id, "linux thumbnail skipped: server thumbnail fetch timed out");
                return;
            }
        };

        match crate::linux_thumbnail::write_freedesktop_thumbnails(&source_path, entry.modified_at, &thumbnail) {
            Ok(paths) => {
                tracing::debug!(
                    file_id = %file_id,
                    count = paths.len(),
                    "linux freedesktop thumbnails written"
                );
            }
            Err(error) => {
                tracing::warn!(file_id = %file_id, error = %error, "linux freedesktop thumbnail write failed");
            }
        }
    }

    #[cfg(target_os = "linux")]
    fn remove_linux_freedesktop_thumbnails_for_file_ids(&self, file_ids: &[String]) {
        for file_id in file_ids {
            let entry = match self.db.get_file(file_id) {
                Ok(Some(entry)) if !entry.is_dir() => entry,
                Ok(_) => continue,
                Err(error) => {
                    tracing::debug!(file_id = %file_id, error = %error, "linux thumbnail cleanup skipped: state row unavailable");
                    continue;
                }
            };
            let Some(source_path) = linux_thumbnail_source_path_for_entry(&entry) else {
                continue;
            };
            if let Err(error) = crate::linux_thumbnail::remove_freedesktop_thumbnails(&source_path) {
                tracing::warn!(file_id = %file_id, error = %error, "linux freedesktop thumbnail cleanup failed");
            }
        }
    }
}
