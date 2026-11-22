//! The placeholder pass: mint a cloud-only placeholder for every `CloudOnly` row (task 2240 slice 2;
//! LD-0113: the Backups tree read-only, its manifest never minted). Moved here verbatim from `mod.rs`
//! to keep that file under the size ratchet.

#![cfg(target_os = "windows")]

use super::placeholders;

// dc-164: one create call per folder batch; placeholders while the first pull still writes rows.
#[cfg(test)]
#[path = "populate_tests.rs"]
mod tests;

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
    /// dc-164: rows an early pass left for later (their folder is not on disk yet).
    pub deferred: usize,
    /// w36: folders listed to see that the memo's placeholders are still there.
    pub dirs_listed: usize,
    pub db_ms: u64,
    pub elapsed_ms: u64,
}

/// w36: the rows an earlier pass placed (created, or found already there), by file id with the path they were placed
/// at. A row in it whose name is still in its folder's listing gets no per-row work at all: before, every tick did
/// the path check, the parent check and a `CfCreatePlaceholders` call for every cloud-only row (8 to 19 metadata
/// operations per row), and that call keeps one Process handle (access 0x1410, on this process) each time it answers
/// "already exists" (the Windows test machine: 500 calls, +500 handles; a 41k-row root reaches the 16.7 M per-process limit about
/// 4 hours after start). A row missing from the listing (deleted, restored after the fact), moved, or new is done
/// in full. One memo belongs to one sync root; [`PlacedMemo::default`] is a full pass.
///
/// w37: with the path, the name the folder LISTS for the row's entry. It is the row's own name, or, for a row whose
/// name reaches its entry in another case or through an 8.3 short name, that entry's long name: such a row is
/// skipped like any other while that name is listed (before, it was called for, and kept a handle, every tick).
#[derive(Debug, Default)]
pub struct PlacedMemo {
    root: std::path::PathBuf,
    placed: std::collections::HashMap<String, (String, std::ffi::OsString)>,
}

/// The names in `dir`, one listing (`FindFirstFile`/`FindNextFile`: no entry is opened). `None`: it could not be listed.
fn names_in(dir: &std::path::Path) -> Option<std::collections::HashSet<std::ffi::OsString>> {
    Some(std::fs::read_dir(dir).ok()?.flatten().map(|e| e.file_name()).collect())
}

/// dc-164: placeholders per `CfCreatePlaceholders` call, at most (one call per folder holds up to this many).
const MINT_BATCH: usize = 1_000;

/// dc-164: how a pass treats a row whose folder is not on disk yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Mode {
    /// The tick's pass: the folder is made (`ensure_directory`), as before.
    Full,
    /// A pass while the first pull still writes rows ([`EarlyPasses`]): the row waits for a later pass, so a folder
    /// whose own row is not in yet never becomes a plain directory where its placeholder belongs.
    Early,
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
///
/// dc-164: in batches. The rows of one folder are minted in one `CfCreatePlaceholders` call (per [`MINT_BATCH`]), and
/// the two marks of the files it created are written in one transaction.
pub(super) fn populate_placeholders(
    db: &crate::state_db::StateDb,
    sync_root: &std::path::Path,
    memo: &mut PlacedMemo,
) -> PopulateStats {
    populate_placeholders_in(db, sync_root, memo, Mode::Full)
}

/// [`populate_placeholders`] in `mode`.
pub(super) fn populate_placeholders_in(
    db: &crate::state_db::StateDb,
    sync_root: &std::path::Path,
    memo: &mut PlacedMemo,
    mode: Mode,
) -> PopulateStats {
    if memo.root != sync_root {
        *memo = PlacedMemo {
            root: sync_root.to_path_buf(),
            placed: Default::default(),
        };
    }
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
    // dc-164: within a depth, by folder, so one folder's rows are one run (one create call).
    entries.sort_by_cached_key(|e| {
        let rel = e.path.trim_matches('/');
        (
            rel.split('/').count(),
            rel.rsplit_once('/').map_or("", |(parent, _)| parent).to_string(),
        )
    });
    let placed = {
        let mut pass = Pass {
            db,
            sync_root,
            mode,
            earlier: &memo.placed,
            // Rebuilt every pass, so a row that left the cloud-only set leaves the memo.
            placed: std::collections::HashMap::with_capacity(memo.placed.len()),
            listings: Default::default(),
            needs_identity: db.files_needing_identity().unwrap_or_default(),
            // LD-0113: the Backups tree, read once for the pass.
            backups: crate::backup_protect::BackupsTree::read(db),
            // Task 2407 (core 2404): the folders this pass found to be their own names, each looked up once per pass.
            dirs: crate::name_alias::CheckedDirs::new(sync_root),
            stats: &mut stats,
        };
        let folder_of = |e: &crate::state_db::FileEntry| {
            let rel = e.path.trim_matches('/');
            (
                rel.split('/').count(),
                rel.rsplit_once('/').map(|(parent, _)| parent.to_string()),
            )
        };
        for folder in entries.chunk_by(|a, b| folder_of(a) == folder_of(b)) {
            pass.folder(folder);
        }
        pass.placed
    };
    memo.placed = placed;
    stats.elapsed_ms = started.elapsed().as_millis() as u64;
    stats
}

/// One row on its way to a create call.
struct ToMint<'e> {
    entry: &'e crate::state_db::FileEntry,
    rel: &'e str,
    parent_dir: std::path::PathBuf,
    name: String,
    in_backups: bool,
}

/// What one pass carries from folder to folder.
struct Pass<'a> {
    db: &'a crate::state_db::StateDb,
    sync_root: &'a std::path::Path,
    mode: Mode,
    earlier: &'a std::collections::HashMap<String, (String, std::ffi::OsString)>,
    placed: std::collections::HashMap<String, (String, std::ffi::OsString)>,
    listings: std::collections::HashMap<std::path::PathBuf, Option<std::collections::HashSet<std::ffi::OsString>>>,
    needs_identity: std::collections::HashSet<String>,
    backups: crate::backup_protect::BackupsTree,
    dirs: crate::name_alias::CheckedDirs<'a>,
    stats: &'a mut PopulateStats,
}

impl Pass<'_> {
    /// The folder's listing, read once per pass.
    fn listing(&mut self, dir: &std::path::Path) -> &mut Option<std::collections::HashSet<std::ffi::OsString>> {
        let stats = &mut *self.stats;
        self.listings.entry(dir.to_path_buf()).or_insert_with(|| {
            stats.dirs_listed += 1;
            names_in(dir)
        })
    }

    /// The rows of one folder (one depth, one parent): the checks per row, then one create call per [`MINT_BATCH`].
    fn folder<'e>(&mut self, rows: &'e [crate::state_db::FileEntry]) {
        let mut batch: Vec<ToMint<'e>> = Vec::new();
        // Names one call may hold: a second row whose name differs from a queued one only in case is done after the
        // call, on its own, as before (the file system then finds the entry the first one made).
        let mut queued: std::collections::HashSet<String> = Default::default();
        let mut after: Vec<ToMint<'e>> = Vec::new();
        for entry in rows {
            let Some(row) = self.prepare(entry) else { continue };
            if queued.insert(row.name.to_lowercase()) {
                batch.push(row);
            } else {
                after.push(row);
            }
        }
        for chunk in batch.chunks(MINT_BATCH) {
            let parent_dir = &chunk[0].parent_dir;
            let specs: Vec<placeholders::MintSpec<'_>> = chunk
                .iter()
                .map(|r| placeholders::MintSpec {
                    file_id: &r.entry.file_id,
                    name: &r.name,
                    size_bytes: r.entry.size_bytes,
                    modified_at: r.entry.modified_at,
                    is_dir: r.entry.is_dir(),
                    read_only: r.in_backups,
                })
                .collect();
            self.stats.create_calls += 1;
            let made = placeholders::mint_batch(parent_dir, &specs);
            let mut fresh = Vec::new();
            for (row, made) in chunk.iter().zip(made) {
                let listed = std::ffi::OsString::from(&row.name);
                if self.settle(row, made, listed) {
                    fresh.push((row.entry.file_id.as_str(), row.entry.size_bytes));
                }
            }
            self.mark_fresh(&fresh);
        }
        for row in after {
            let (made, listed) = self.mint_one(&row);
            if self.settle(&row, made, listed) {
                self.mark_fresh(&[(row.entry.file_id.as_str(), row.entry.size_bytes)]);
            }
        }
    }

    /// The checks of one row before its create call. `None`: nothing to create (skipped, waiting, or already there
    /// and settled here).
    fn prepare<'e>(&mut self, entry: &'e crate::state_db::FileEntry) -> Option<ToMint<'e>> {
        // Stored paths are server-relative, '/'-separated, possibly with a
        // leading slash. Map onto the local sync root with native
        // separators.
        let rel = entry.path.trim_matches('/');
        if rel.is_empty() {
            self.stats.skipped += 1;
            return None;
        }
        // LD-0113 (core's `is_hidden_backup_item`): a device folder's manifest is never in the drive.
        if self.backups.is_hidden(rel) {
            self.stats.skipped += 1;
            return None;
        }
        let local_path = self.sync_root.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
        let (parent_dir, name) = match (local_path.parent(), local_path.file_name()) {
            (Some(parent), Some(name)) => (parent.to_path_buf(), name.to_string_lossy().into_owned()),
            _ => {
                // Zero-knowledge: log the file_id only, never the decrypted
                // plaintext path/name.
                tracing::warn!(file_id = %entry.file_id, "skipping placeholder for path with no file name");
                self.stats.skipped += 1;
                return None;
            }
        };

        // w36: placed at this path by an earlier pass and still in its folder's listing: nothing to do.
        let earlier = self.earlier;
        if let Some((_, listed)) = earlier.get(&entry.file_id).filter(|(at, _)| at == rel) {
            let listed = listed.clone();
            if self
                .listing(&parent_dir)
                .as_ref()
                .is_some_and(|names| names.contains(&listed))
            {
                self.placed.insert(entry.file_id.clone(), (rel.to_string(), listed));
                return None;
            }
        }
        // dc-164: while the first pull still writes rows, a row whose folder is not there yet waits for a later pass.
        if self.mode == Mode::Early && parent_dir != self.sync_root && self.listing(&parent_dir).is_none() {
            self.stats.deferred += 1;
            return None;
        }
        self.stats.visited += 1;

        // Task 2407 (core 2404): a folder on the way whose name reaches a DIFFERENT folder (a Windows 8.3 short name)
        // would put the placeholder into that folder, and every later delete or fetch by this row's path would reach
        // it. The row is skipped: its file stays on the server, in the web app. Placeholder creation itself never
        // replaces (`CF_CREATE_FLAG_NONE`), so a leaf name that reaches another file only answers "already exists".
        match self.dirs.check(&parent_dir) {
            Ok(None) => {}
            Ok(Some(_)) => {
                tracing::warn!(file_id = %entry.file_id, "placeholder skipped: a folder on its path reaches a different folder (8.3 alias)");
                self.stats.name_conflicts += 1;
                return None;
            }
            Err(e) => {
                // Zero-knowledge: the kind only (the error text can carry the plaintext path).
                tracing::warn!(kind = ?e.kind(), file_id = %entry.file_id, "placeholder skipped: its path could not be checked");
                self.stats.name_check_errors += 1;
                return None;
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
            self.stats.ensure_dir_errors += 1;
            return None;
        }

        // LD-0113 Amendment 7: a file at or under the Backups root is minted read-only.
        let in_backups = self.backups.below_root(rel).is_some();
        let row = ToMint {
            entry,
            rel,
            parent_dir,
            name,
            in_backups,
        };
        // w36: a name already in its folder's listing is "already exists" without the call: `CfCreatePlaceholders`
        // keeps one Process handle every time it gives that answer. w37: a name the listing does not show under
        // these exact characters is asked of the file system, which also finds the entry it reaches in another case
        // or through an 8.3 short name. `listed` is the name the listing has for the row's entry.
        if let Some(listed) = self.already_there(&row) {
            self.settle(&row, Ok(false), listed);
            return None;
        }
        Some(row)
    }

    /// The name the folder lists for `row`'s entry, when one is there (see [`Self::prepare`]).
    fn already_there(&mut self, row: &ToMint<'_>) -> Option<std::ffi::OsString> {
        let os_name = std::ffi::OsStr::new(&row.name);
        if self
            .listing(&row.parent_dir)
            .as_ref()
            .is_some_and(|names| names.contains(os_name))
        {
            return Some(os_name.to_os_string());
        }
        placeholders::entry_name_at(&row.parent_dir, &row.name)
    }

    /// One row on its own (a name one call already holds in another case): asked first, then created alone.
    fn mint_one(&mut self, row: &ToMint<'_>) -> (anyhow::Result<bool>, std::ffi::OsString) {
        if let Some(listed) = self.already_there(row) {
            return (Ok(false), listed);
        }
        self.stats.create_calls += 1;
        let made = placeholders::mint(
            &row.parent_dir,
            &row.entry.file_id,
            &row.name,
            row.entry.size_bytes,
            row.entry.modified_at,
            row.entry.is_dir(),
            row.in_backups,
        );
        (made, std::ffi::OsString::from(&row.name))
    }

    /// What a create call's answer for `row` means. `true`: a FILE placeholder was created now, and its two marks are
    /// owed ([`Self::mark_fresh`]).
    fn settle(&mut self, row: &ToMint<'_>, made: anyhow::Result<bool>, listed: std::ffi::OsString) -> bool {
        let (entry, rel, parent_dir, name) = (row.entry, row.rel, &row.parent_dir, &row.name);
        match made {
            Ok(true) => {
                if let Some(names) = self.listing(parent_dir).as_mut() {
                    names.insert(std::ffi::OsString::from(name));
                }
                self.stats.created += 1;
                self.placed.insert(entry.file_id.clone(), (rel.to_string(), listed));
                // Created under exactly its name: rows below it need no lookup of it this pass.
                if entry.is_dir() {
                    self.dirs.mark_own(parent_dir.join(name));
                }
                // LD-0113 Amendment 8a: a Backups root minted again (the after-the-fact restore) shows its
                // badge again.
                if entry.is_dir() && self.backups.position(rel) == beebeeb_core::backup::BackupPosition::Root {
                    crate::backup_protect::show_read_only_badge(&parent_dir.join(name), true);
                }
                !entry.is_dir()
            }
            Ok(false) => {
                self.stats.already_existed += 1;
                // w36: a row still waiting for its version record is asked again next pass, as before.
                if entry.is_dir() || !self.needs_identity.contains(&entry.file_id) {
                    self.placed.insert(entry.file_id.clone(), (rel.to_string(), listed));
                }
                // Review P2 (task 2180): a placeholder minted before identities were recorded (0.8.21), or
                // kept across a version change, takes the listing's version when it holds nothing: as at a
                // mint, its first fetch serves that version whole. One that holds bytes stays without.
                // Task 2407: only an entry that is this name's own (never a different file an 8.3 short name
                // reaches) takes the row's identity. Nor the entry of another server item at this place (one name,
                // or one name but for case): the second of two such rows is not placed, and claims nothing here.
                if !entry.is_dir()
                    && self.needs_identity.contains(&entry.file_id)
                    && matches!(
                        crate::name_alias::conflict_below(parent_dir, &parent_dir.join(name)),
                        Ok(None)
                    )
                    && not_another_rows(self.db, self.sync_root, entry)
                    && matches!(
                        placeholders::platform_view(&parent_dir.join(name)),
                        crate::unknown_check::PlatformView::Placeholder {
                            on_disk: crate::unknown_check::OnDisk::Nothing,
                            ..
                        }
                    )
                    && let Err(e) = self.db.record_minted_version(&entry.file_id, entry.size_bytes)
                {
                    tracing::debug!(error = %e, file_id = %entry.file_id, "could not record the placeholder's version");
                }
                false
            }
            Err(e) => {
                self.stats.create_errors += 1;
                // Zero-knowledge: log the file_id only — never the decrypted
                // plaintext filename.
                tracing::warn!(
                    error = %e,
                    file_id = %entry.file_id,
                    "placeholder creation failed"
                );
                false
            }
        }
    }

    /// The two marks of the file placeholders one call created, in one transaction (dc-164: they were two commits
    /// per row). Task 1640 (review F1): created where nothing was, so no fetched bytes are on disk: a first fetch
    /// starts the file's candidate. Only a row never fetched into is marked (a fetch on the new placeholder records
    /// first), so none is forgotten. Task 2180 / LD-0105 S3: the version the listing shows now is the placeholder's
    /// identity. Without a record its bytes are never trusted, so a failure only costs a later refetch, never mixed
    /// versions.
    fn mark_fresh(&mut self, files: &[(&str, i64)]) {
        if files.is_empty() {
            return;
        }
        // The batch holds the database for this thread only while these rows are written: nothing in it waits.
        let batch = match self.db.write_batch(usize::MAX) {
            Ok(batch) => Some(batch),
            Err(e) => {
                tracing::debug!(error = %e, "placeholder marks: no batch; written one by one");
                None
            }
        };
        for &(file_id, size) in files {
            if let Err(e) = self.db.mark_no_fetched_content(file_id) {
                tracing::debug!(error = %e, file_id = %file_id, "could not mark the new placeholder empty");
            }
            if let Err(e) = self.db.record_minted_version(file_id, size) {
                tracing::debug!(error = %e, file_id = %file_id, "could not record the placeholder's version");
            }
        }
        if let Some(batch) = batch
            && let Err(e) = batch.finish()
        {
            tracing::warn!(error = %e, files = files.len(), "placeholder marks: the batch did not commit");
        }
    }
}

/// dc-164: placeholder passes while the first pull is still writing rows, so Explorer shows the top folders within a
/// minute instead of after the whole list (10-20 minutes at 41,510 items, task dc-086). The pull writes rows parent
/// first (breadth first) and commits them in batches; each pass here mints what is committed so far, top level first,
/// and leaves a row whose folder is not on disk yet to a later pass ([`Mode::Early`]). Off the async workers. Started
/// by the runner before the first pull and stopped (and waited for) right after it, before the tick's own pass.
pub struct EarlyPasses {
    stop: Option<std::sync::mpsc::Sender<()>>,
    worker: Option<tokio::task::JoinHandle<()>>,
}

/// The pause between two early passes, at least: a pass that took longer waits twice its own time.
const EARLY_EVERY: std::time::Duration = std::time::Duration::from_secs(2);

impl EarlyPasses {
    /// Start the passes on `sync_root`, each with the engine bridge of that moment (none while there is none).
    pub fn start(sync_root: &std::path::Path) -> Self {
        let root = sync_root.to_path_buf();
        Self::start_with(EARLY_EVERY, move || {
            let bridge = super::bridge()?;
            Some(populate_placeholders_in(
                bridge.db(),
                &root,
                &mut super::placed_memo(),
                Mode::Early,
            ))
        })
    }

    /// The passes, each one `pass()`, every `every` at least, until [`Self::stop`].
    fn start_with(
        every: std::time::Duration,
        mut pass: impl FnMut() -> Option<PopulateStats> + Send + 'static,
    ) -> Self {
        let (stop, stopped) = std::sync::mpsc::channel::<()>();
        let worker = tokio::task::spawn_blocking(move || {
            let mut wait = std::time::Duration::ZERO;
            // A message or a dropped sender both end it.
            while let Err(std::sync::mpsc::RecvTimeoutError::Timeout) = stopped.recv_timeout(wait) {
                let started = std::time::Instant::now();
                if let Some(s) = pass() {
                    // Counts and milliseconds only, never a path.
                    tracing::debug!(
                        created = s.created,
                        deferred = s.deferred,
                        create_calls = s.create_calls,
                        elapsed_ms = s.elapsed_ms,
                        "early placeholder pass"
                    );
                }
                wait = every.max(started.elapsed() * 2);
            }
        });
        Self {
            stop: Some(stop),
            worker: Some(worker),
        }
    }

    /// End the passes and wait for the one running.
    pub async fn stop(mut self) {
        drop(self.stop.take());
        if let Some(worker) = self.worker.take()
            && let Err(e) = worker.await
        {
            tracing::warn!(error = %e, "early placeholder pass panicked; the tick's pass follows");
        }
    }
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
