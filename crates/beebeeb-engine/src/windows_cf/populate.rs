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
    /// `CfCreatePlaceholders` calls (w36: none for a name its folder's listing already shows).
    pub create_calls: usize,
    pub created: usize,
    pub already_existed: usize,
    pub create_errors: usize,
    /// Task 2407: rows skipped because a folder on their path reaches a different folder (an 8.3 short name).
    pub name_conflicts: usize,
    /// Task 2407: rows skipped because that check could not finish (retried next pass).
    pub name_check_errors: usize,
    /// w36: rows that got per-row file work this pass (every row but the ones the memo skipped).
    pub visited: usize,
    /// w36: folders listed to see that the memo's placeholders are still there.
    pub dirs_listed: usize,
    pub db_ms: u64,
    pub elapsed_ms: u64,
}

/// w36: the rows an earlier pass placed (created, or found already there), by file id with the path they were placed
/// at. A row in it whose name is still in its folder's listing gets no per-row work at all: before, every tick did
/// the path check, the parent check and a `CfCreatePlaceholders` call for every cloud-only row (8 to 19 metadata
/// operations per row), and that call keeps one Process handle (access 0x1410, on this process) each time it answers
/// "already exists" (VM 401: 500 calls, +500 handles; a 41k-row root reaches the 16.7 M per-process limit about
/// 4 hours after start). A row missing from the listing (deleted, restored after the fact), moved, or new is done
/// in full. One memo belongs to one sync root; [`PlacedMemo::default`] is a full pass.
#[derive(Debug, Default)]
pub struct PlacedMemo {
    root: std::path::PathBuf,
    placed: std::collections::HashMap<String, String>,
}

/// The names in `dir`, one listing (`FindFirstFile`/`FindNextFile`: no entry is opened). `None`: it could not be listed.
fn names_in(dir: &std::path::Path) -> Option<std::collections::HashSet<std::ffi::OsString>> {
    Some(std::fs::read_dir(dir).ok()?.flatten().map(|e| e.file_name()).collect())
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
pub(super) fn populate_placeholders(
    db: &crate::state_db::StateDb,
    sync_root: &std::path::Path,
    memo: &mut PlacedMemo,
) -> PopulateStats {
    if memo.root != sync_root {
        *memo = PlacedMemo {
            root: sync_root.to_path_buf(),
            placed: Default::default(),
        };
    }
    // Rebuilt every pass, so a row that left the cloud-only set leaves the memo.
    let mut placed = std::collections::HashMap::with_capacity(memo.placed.len());
    let mut listings: std::collections::HashMap<
        std::path::PathBuf,
        Option<std::collections::HashSet<std::ffi::OsString>>,
    > = Default::default();
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
        // w36: placed at this path by an earlier pass and still in its folder's listing: nothing to do.
        if memo.placed.get(&entry.file_id).is_some_and(|at| at == rel) {
            let listing = listings.entry(parent_dir.clone()).or_insert_with(|| {
                stats.dirs_listed += 1;
                names_in(&parent_dir)
            });
            if listing
                .as_ref()
                .is_some_and(|names| local_path.file_name().is_some_and(|n| names.contains(n)))
            {
                placed.insert(entry.file_id.clone(), rel.to_string());
                continue;
            }
        }
        stats.visited += 1;

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

        // LD-0113 Amendment 7: a file at or under the Backups root is minted read-only.
        let in_backups = backups.below_root(rel).is_some();
        // w36: a name already in its folder's listing is "already exists" without the call: `CfCreatePlaceholders`
        // keeps one Process handle every time it gives that answer. Only a name the listing does not show under
        // these exact characters (another case, an 8.3 short name) still gets that answer from the call.
        let listing = listings.entry(parent_dir.clone()).or_insert_with(|| {
            stats.dirs_listed += 1;
            names_in(&parent_dir)
        });
        let os_name = std::ffi::OsStr::new(&name);
        let there = listing.as_ref().is_some_and(|names| names.contains(os_name));
        let made = if there {
            Ok(false)
        } else {
            stats.create_calls += 1;
            placeholders::create_placeholder_with(
                &parent_dir,
                &entry.file_id,
                &name,
                entry.size_bytes,
                entry.modified_at,
                entry.is_dir(),
                in_backups,
            )
        };
        match made {
            Ok(true) => {
                if let Some(names) = listing.as_mut() {
                    names.insert(os_name.to_os_string());
                }
                stats.created += 1;
                placed.insert(entry.file_id.clone(), rel.to_string());
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
                // w36: a row still waiting for its version record is asked again next pass, as before.
                if entry.is_dir() || !needs_identity.contains(&entry.file_id) {
                    placed.insert(entry.file_id.clone(), rel.to_string());
                }
                // Review P2 (task 2180): a placeholder minted before identities were recorded (0.8.21), or
                // kept across a version change, takes the listing's version when it holds nothing: as at a
                // mint, its first fetch serves that version whole. One that holds bytes stays without.
                // Task 2407: only an entry that is this name's own (never a different file an 8.3 short name
                // reaches) takes the row's identity. Nor the entry of another server item at this place (one name,
                // or one name but for case): the second of two such rows is not placed, and claims nothing here.
                if !entry.is_dir()
                    && needs_identity.contains(&entry.file_id)
                    && matches!(
                        crate::name_alias::conflict_below(&parent_dir, &parent_dir.join(&name)),
                        Ok(None)
                    )
                    && not_another_rows(db, sync_root, &entry)
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
    memo.placed = placed;
    stats.elapsed_ms = started.elapsed().as_millis() as u64;
    stats
}

/// LD-0113 §6.3: put back every placeholder a protected row lost (the after-the-fact restore), now.
pub fn restore_placeholders(db: &crate::state_db::StateDb, sync_root: &std::path::Path) -> usize {
    populate_placeholders(db, sync_root, &mut PlacedMemo::default()).created
}

/// Whether the entry at `entry`'s place is not another row's (`drive_v2::case_place`); asked only of a row whose
/// placeholder already existed and that needs its version recorded. Another row's, or not known: logged by id, and
/// nothing is recorded for this row.
fn not_another_rows(
    db: &crate::state_db::StateDb,
    sync_root: &std::path::Path,
    entry: &crate::state_db::FileEntry,
) -> bool {
    let own = matches!(
        crate::drive_v2::case_place::place_is_another_rows(db, sync_root, entry),
        Ok(false)
    );
    if !own {
        tracing::debug!(
            file_id = %entry.file_id,
            "placeholder version not recorded: another item has the same name at this place"
        );
    }
    own
}
