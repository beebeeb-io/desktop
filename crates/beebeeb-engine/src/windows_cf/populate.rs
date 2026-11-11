//! The placeholder pass: mint a cloud-only placeholder for every `CloudOnly` row (task 2240 slice 2;
//! LD-0113: the Backups tree read-only, its manifest never minted). Moved here verbatim from `mod.rs`
//! to keep that file under the size ratchet.

#![cfg(target_os = "windows")]

use super::placeholders;

/// What one [`populate_placeholders`] pass did (task 2240 slice 2, spec D6: measured, not changed). Counts and
/// milliseconds only, never a path.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PopulateStats {
    pub rows: usize,
    pub skipped: usize,
    pub ensure_dir_errors: usize,
    pub create_calls: usize,
    pub created: usize,
    pub already_existed: usize,
    pub create_errors: usize,
    /// Task 2407: rows skipped because a folder on their path reaches a different folder (an 8.3 short name).
    pub name_conflicts: usize,
    /// Task 2407: rows skipped because that check could not finish (retried next pass).
    pub name_check_errors: usize,
    pub db_ms: u64,
    pub elapsed_ms: u64,
}

/// Walk the engine bridge's DB file list and drop a cloud-only placeholder
/// into Explorer for every file currently marked `CloudOnly`. Returns what it
/// did; `created` is the number of placeholders created now (task 1640: an
/// already-existing one is no error, but is not counted as created).
///
/// The placeholder is created under `sync_root joined with the file's
/// parent directory` — we mint the parent path from the stored relative
/// path and ensure it exists first, so nested cloud files land in the right
/// Explorer subfolder.
pub(super) fn populate_placeholders(db: &crate::state_db::StateDb, sync_root: &std::path::Path) -> PopulateStats {
    let started = std::time::Instant::now();
    let mut stats = PopulateStats::default();
    let mut entries = match db.list_by_status(crate::state_db::FileStatus::CloudOnly) {
        Ok(entries) => entries,
        Err(e) => {
            tracing::warn!(error = %e, "could not list cloud-only files for placeholder seeding");
            return stats;
        }
    };
    stats.db_ms = started.elapsed().as_millis() as u64;
    stats.rows = entries.len();

    // PARENTS-BEFORE-CHILDREN. A nested file's placeholder must land INSIDE its
    // parent directory's placeholder, and a directory placeholder must exist
    // before anything is created under it. Sorting by path depth (number of
    // '/'-separated segments, ascending) guarantees every ancestor directory
    // row is processed before its descendants. Folders and files at the same
    // depth are independent, so their relative order is irrelevant.
    //
    // Why this matters with directory placeholders: a folder row is minted as a
    // real CF directory placeholder (DISABLE_ON_DEMAND_POPULATION) so Explorer
    // treats it as fully enumerated by its on-disk children. If we instead
    // processed a child first, `ensure_directory` would `mkdir` a PLAIN OS
    // directory at the parent path, and the later folder-row `create_placeholder`
    // would no-op on ERROR_ALREADY_EXISTS — leaving a plain dir where a CF
    // directory placeholder was wanted. Depth-ordering avoids that race.
    entries.sort_by_key(|e| e.path.trim_matches('/').split('/').count());
    let needs_identity = db.files_needing_identity().unwrap_or_default();
    // LD-0113: the Backups tree, read once for the pass.
    let backups = crate::backup_protect::BackupsTree::read(db);
    // Task 2407 (core 2404): the folders this pass found to be their own names, each looked up once per pass.
    let mut dirs = crate::name_alias::CheckedDirs::new(sync_root);

    for entry in entries {
        // Stored paths are server-relative, '/'-separated, possibly with a
        // leading slash. Map onto the local sync root with native
        // separators.
        let rel = entry.path.trim_matches('/');
        if rel.is_empty() {
            stats.skipped += 1;
            continue;
        }
        // LD-0113 (core's `is_hidden_backup_item`): a device folder's manifest is never in the drive.
        if backups.is_hidden(rel) {
            stats.skipped += 1;
            continue;
        }
        let local_path = sync_root.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
        let (parent_dir, name) = match (local_path.parent(), local_path.file_name()) {
            (Some(parent), Some(name)) => (parent.to_path_buf(), name.to_string_lossy().into_owned()),
            _ => {
                // Zero-knowledge: log the file_id only, never the decrypted
                // plaintext path/name.
                tracing::warn!(file_id = %entry.file_id, "skipping placeholder for path with no file name");
                stats.skipped += 1;
                continue;
            }
        };

        // Task 2407 (core 2404): a folder on the way whose name reaches a DIFFERENT folder (a Windows 8.3 short name)
        // would put the placeholder into that folder, and every later delete or fetch by this row's path would reach
        // it. The row is skipped: its file stays on the server, in the web app. Placeholder creation itself never
        // replaces (`CF_CREATE_FLAG_NONE`), so a leaf name that reaches another file only answers "already exists".
        match dirs.check(&parent_dir) {
            Ok(None) => {}
            Ok(Some(_)) => {
                tracing::warn!(file_id = %entry.file_id, "placeholder skipped: a folder on its path reaches a different folder (8.3 alias)");
                stats.name_conflicts += 1;
                continue;
            }
            Err(e) => {
                // Zero-knowledge: the kind only (the error text can carry the plaintext path).
                tracing::warn!(kind = ?e.kind(), file_id = %entry.file_id, "placeholder skipped: its path could not be checked");
                stats.name_check_errors += 1;
                continue;
            }
        }

        // Ensure the immediate parent directory exists on disk. For a nested
        // row, depth-ordering means the parent FOLDER row was already minted as
        // a CF directory placeholder above, so this is a no-op for it; this call
        // only materialises the sync root itself (depth-1 rows) or backfills any
        // intermediate dir the server didn't surface as its own row.
        if let Err(e) = crate::config::ensure_directory(&parent_dir) {
            // Zero-knowledge: the parent dir is built from the decrypted
            // relative path, so it can carry plaintext folder names — log the
            // file_id only, not the path.
            tracing::warn!(error = %e, file_id = %entry.file_id, "could not create placeholder parent dir");
            stats.ensure_dir_errors += 1;
            continue;
        }

        stats.create_calls += 1;
        // LD-0113 Amendment 7: a file at or under the Backups root is minted read-only.
        let in_backups = backups.below_root(rel).is_some();
        match placeholders::create_placeholder_with(
            &parent_dir,
            &entry.file_id,
            &name,
            entry.size_bytes,
            entry.modified_at,
            entry.is_dir(),
            in_backups,
        ) {
            Ok(true) => {
                stats.created += 1;
                // Created under exactly its name: rows below it need no lookup of it this pass.
                if entry.is_dir() {
                    dirs.mark_own(parent_dir.join(&name));
                }
                // LD-0113 Amendment 8a: a Backups root minted again (the after-the-fact restore) shows its
                // badge again.
                if entry.is_dir() && backups.position(rel) == beebeeb_core::backup::BackupPosition::Root {
                    crate::backup_protect::show_read_only_badge(&parent_dir.join(&name), true);
                }
                // Task 1640 (review F1): created where nothing was, so no fetched bytes are on
                // disk: a first fetch starts the file's candidate. Only a row never fetched into
                // is marked (a fetch on the new placeholder records first), so none is forgotten.
                if !entry.is_dir() {
                    if let Err(e) = db.mark_no_fetched_content(&entry.file_id) {
                        tracing::debug!(error = %e, file_id = %entry.file_id, "could not mark the new placeholder empty");
                    }
                    // Task 2180 / LD-0105 S3: the version the listing shows now is this placeholder's
                    // identity. Without a record its bytes are never trusted, so a failure only costs a
                    // later refetch, never mixed versions.
                    if let Err(e) = db.record_minted_version(&entry.file_id, entry.size_bytes) {
                        tracing::debug!(error = %e, file_id = %entry.file_id, "could not record the placeholder's version");
                    }
                }
            }
            Ok(false) => {
                stats.already_existed += 1;
                // Review P2 (task 2180): a placeholder minted before identities were recorded (0.8.21), or
                // kept across a version change, takes the listing's version when it holds nothing: as at a
                // mint, its first fetch serves that version whole. One that holds bytes stays without.
                // Task 2407: only an entry that is this name's own (never a different file an 8.3 short name
                // reaches) takes the row's identity.
                if !entry.is_dir()
                    && needs_identity.contains(&entry.file_id)
                    && matches!(
                        crate::name_alias::conflict_below(&parent_dir, &parent_dir.join(&name)),
                        Ok(None)
                    )
                    && matches!(
                        placeholders::platform_view(&parent_dir.join(&name)),
                        crate::unknown_check::PlatformView::Placeholder {
                            on_disk: crate::unknown_check::OnDisk::Nothing,
                            ..
                        }
                    )
                    && let Err(e) = db.record_minted_version(&entry.file_id, entry.size_bytes)
                {
                    tracing::debug!(error = %e, file_id = %entry.file_id, "could not record the placeholder's version");
                }
            }
            Err(e) => {
                stats.create_errors += 1;
                // Zero-knowledge: log the file_id only — never the decrypted
                // plaintext filename.
                tracing::warn!(
                    error = %e,
                    file_id = %entry.file_id,
                    "placeholder creation failed"
                );
            }
        }
    }
    stats.elapsed_ms = started.elapsed().as_millis() as u64;
    stats
}

/// LD-0113 §6.3: put back every placeholder a protected row lost (the after-the-fact restore), now.
pub fn restore_placeholders(db: &crate::state_db::StateDb, sync_root: &std::path::Path) -> usize {
    populate_placeholders(db, sync_root).created
}
