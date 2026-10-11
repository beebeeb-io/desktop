//! Durable ownership of plaintext copies, including work cancelled before an
//! upload session exists. Rows survive failed unlink and operation/resume purges.
use crate::state_db::StateDb;
use std::{
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
};

pub struct StagedPayload {
    db: Arc<StateDb>,
    path: String,
    source: PathBuf,
    /// Set by [`StagedPayload::copy_from_file`]: the open source, which the
    /// drop compares against instead of re-opening `source` by path.
    source_file: Option<std::fs::File>,
    retained: bool,
}
impl StagedPayload {
    pub fn copy(db: Arc<StateDb>, source: &Path, root: PathBuf) -> anyhow::Result<Self> {
        anyhow::ensure!(source.is_file(), "Upload source is not a file");
        std::fs::create_dir_all(&root)?;
        let path = root
            .join(uuid::Uuid::new_v4().to_string())
            .to_string_lossy()
            .into_owned();
        // Journal BEFORE creating any plaintext; a crash cannot orphan the copy.
        db.track_staged_payload(&path, Some(&source.to_string_lossy()), false)?;
        let staged = Self {
            db,
            path,
            source: source.to_path_buf(),
            source_file: None,
            retained: false,
        };
        std::fs::copy(source, &staged.path)?;
        Ok(staged)
    }
    /// [`Self::copy`] from a file the caller already holds open and checked
    /// (the macOS handed-over Finder write contents, opened without following
    /// links). Only that descriptor is read; `source` is the journal's label
    /// and is never opened.
    pub fn copy_from_file(
        db: Arc<StateDb>,
        file: &std::fs::File,
        source: &Path,
        root: PathBuf,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(file.metadata()?.is_file(), "Upload source is not a file");
        let source_file = file.try_clone()?;
        std::fs::create_dir_all(&root)?;
        let path = root
            .join(uuid::Uuid::new_v4().to_string())
            .to_string_lossy()
            .into_owned();
        // Journal BEFORE creating any plaintext; a crash cannot orphan the copy.
        db.track_staged_payload(&path, Some(&source.to_string_lossy()), false)?;
        let staged = Self {
            db,
            path,
            source: source.to_path_buf(),
            source_file: Some(source_file),
            retained: false,
        };
        copy_open_file(file, Path::new(&staged.path))?;
        Ok(staged)
    }
    pub fn path(&self) -> &str {
        &self.path
    }
    pub fn retain(mut self) {
        self.retained = true;
    }
}
impl Drop for StagedPayload {
    fn drop(&mut self) {
        if self.retained {
            return;
        }
        // Never discard the only unsynced copy if the original disappeared or
        // changed during the upload. Its journal blocks sign-out for recovery.
        // A copy taken from an open file is compared with that file: its
        // directory entry may already be gone, its contents are not.
        let duplicate = match &self.source_file {
            Some(file) => same_bytes_as_open_file(Path::new(&self.path), file),
            None => same_bytes(Path::new(&self.path), &self.source),
        }
        .unwrap_or(false);
        if !Path::new(&self.path).exists() || duplicate {
            if let Err(error) = remove(&self.db, Path::new(&self.path)) {
                tracing::warn!(
                    reason = crate::engine_bridge::error_category(error.as_ref()),
                    "staged payload cleanup deferred; journal retained"
                );
            }
        }
    }
}
pub fn remove(db: &StateDb, path: &Path) -> anyhow::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => (),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
        Err(e) => return Err(e.into()),
    }
    db.forget_staged_payload(&path.to_string_lossy())?;
    Ok(())
}
/// [`remove`], for a copy whose upload is done: a failure keeps the journal row, so a later
/// start or sign-out still finds the copy. The line logs the error's category only; its text
/// can carry a path (spec §11).
pub fn remove_or_log(db: &StateDb, path: &Path) {
    if let Err(e) = remove(db, path) {
        tracing::warn!(
            reason = crate::engine_bridge::error_category(e.as_ref()),
            "staged upload cleanup deferred; journal retained"
        );
    }
}
/// Engine start (spec §8.7 S6): unlink the released copies `StateDb::engine_start_repair` listed,
/// each by its journalled absolute path, so a copy staged in a folder an earlier build used goes
/// too (spec §8.4). A copy that cannot be removed keeps its row for the next start. Returns how
/// many were removed. macOS only: the release journal is written only there.
#[cfg(target_os = "macos")]
pub fn remove_released(db: &StateDb, paths: &[String]) -> usize {
    let mut removed = 0;
    for path in paths {
        match remove(db, Path::new(path)) {
            Ok(()) => removed += 1,
            Err(e) => tracing::warn!(
                reason = crate::engine_bridge::error_category(e.as_ref()),
                "released upload copy kept; removal is retried at the next start"
            ),
        }
    }
    removed
}
/// What [`sweep_unreferenced`] did at engine start. Counts only (spec §11).
#[cfg(unix)]
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct UnreferencedSweep {
    /// Journal rows of accepts that never committed, cleared with their copies.
    pub unfinished: usize,
    /// Files that no row names, unlinked.
    pub unjournalled: usize,
    /// Copies the sweep would remove but could not; the next start or a sign-out finds them.
    pub kept: usize,
    /// The category of the first failure: a copy that could not be removed, or a staging folder
    /// that could not be opened or listed.
    pub failure: Option<&'static str>,
}

#[cfg(unix)]
impl UnreferencedSweep {
    fn kept(&mut self, error: &std::io::Error) {
        self.kept += 1;
        self.failed(error);
    }

    fn failed(&mut self, error: &std::io::Error) {
        self.failure.get_or_insert(crate::engine_bridge::error_category(error));
    }
}

/// Engine start on macOS, after the queue claims are cleared and before the IPC server serves a
/// write (spec §8.7 S6): remove every staged plaintext copy in the staging `roots` that nothing
/// will upload.
///
/// 1. A journal row with `completed = 0` that no op, resume row or finalization names: an accept
///    that never committed (a crash between its journal row and its transaction, or a refusal
///    after the copy). Its row is deleted first, guarded ([`StateDb::forget_unfinished_unreferenced`]:
///    only while it is still unfinished and nothing names it), and only then is its copy unlinked.
///    An accept still running (a stopping engine's handler) requires that row in its own
///    transaction, so it either committed first, and its op keeps the row and the copy, or finds
///    the row gone, queues nothing and is retried. A copy whose unlink fails has no row left, so the
///    next start's step 2 finds it.
/// 2. A regular file directly in a staging folder that no row of `staged_payloads`,
///    `operation_queue`, `upload_resume` or `upload_finalizations` names: a copy whose row an
///    older build's sign-out deleted. It is unlinked. Each folder is listed before the rows are
///    read: a copy is made only after its journal row, so a copy the listing saw is named by the
///    rows read after it, unless nothing names it. Read the other way round, an accept that
///    journalled after the read and copied before the listing would lose its copy.
///
/// On macOS the system hands a write's bytes over again when it retries the write, so a copy no
/// row names is never the only copy of a save still to upload. A write whose accept never
/// committed was never acknowledged, so the system still holds its bytes as a pending change. A
/// committed write's copy is named by its op until it lands, and released after. A copy whose op
/// an older build's sign-out purged belongs to a save that sign-out already discarded.
///
/// It never follows a link. Each folder is opened without following one, and must be a directory
/// this user owns; inside it, only regular files are touched (`lstat`), never a link, a subfolder
/// or the folder itself. A file counts as named when a row names a path with the same file name:
/// a copy is named by a fresh UUID, and comparing names rather than whole paths keeps a copy whose
/// row spells its folder another way. A row of step 1 whose folder is not one of `roots` is left
/// alone. A copy that cannot be removed stays, counted in `kept`.
#[cfg(unix)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn sweep_unreferenced(db: &StateDb, roots: &[PathBuf]) -> rusqlite::Result<UnreferencedSweep> {
    use crate::ipc_socket::{StagingDir, StagingDirRefusal};
    let mut sweep = UnreferencedSweep::default();
    let mut folders: Vec<(&Path, StagingDir)> = Vec::new();
    for root in roots {
        match StagingDir::open(root) {
            Ok(dir) => folders.push((root.as_path(), dir)),
            Err(StagingDirRefusal::Missing) => {}
            Err(refusal) => sweep.failed(&refusal.into_io_error()),
        }
    }
    // 1. Accepts that never committed. The row goes first, guarded (`forget_unfinished_unreferenced`),
    // then the copy: an accept's transaction requires the row, so an accept still running (a
    // stopping engine's handler) either committed first, and its op keeps the row and the copy, or
    // finds the row gone and queues nothing (`WriteRetryLater`).
    for path in db.unfinished_unreferenced_payloads()? {
        let path_ref = Path::new(&path);
        let (Some(parent), Some(name)) = (path_ref.parent(), path_ref.file_name()) else {
            continue;
        };
        let Some((_, dir)) = folders.iter().find(|(root, _)| *root == parent) else {
            continue;
        };
        let regular = match dir.stat_entry(name) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
            Err(e) => {
                sweep.kept(&e);
                continue;
            }
            // A link or a folder at the journalled name is never followed or removed, and keeps
            // its row.
            Ok(stat) if !is_regular_file(&stat) => continue,
            Ok(_) => true,
        };
        if !db.forget_unfinished_unreferenced(&path)? {
            continue;
        }
        sweep.unfinished += 1;
        // A copy already gone (its accept's own cleanup won the race) is done, not kept. Any other
        // failure leaves the copy without a row: the next start's step 2 finds it.
        if regular
            && let Err(e) = dir.unlink_entry(name)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            sweep.kept(&e);
        }
    }
    // 2. Files no row names. Every folder is listed before the rows are read: a copy is made only
    // after its journal row (`StagedPayload`), so a copy the listing saw is named by the rows read
    // after it unless nothing names it.
    let mut listings = Vec::with_capacity(folders.len());
    for (_, dir) in &folders {
        match dir.entry_names() {
            Ok(names) => listings.push((dir, names)),
            Err(e) => sweep.failed(&e),
        }
    }
    #[cfg(test)]
    run_between_listing_and_names();
    let named: std::collections::HashSet<std::ffi::OsString> = db
        .staged_copy_names()?
        .iter()
        .filter_map(|path| Path::new(path).file_name().map(std::ffi::OsStr::to_os_string))
        .collect();
    for (dir, names) in listings {
        for name in names.iter().filter(|name| !named.contains(*name)) {
            match dir.stat_entry(name) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => sweep.kept(&e),
                Ok(stat) if !is_regular_file(&stat) => {}
                Ok(_) => match dir.unlink_entry(name) {
                    Ok(()) => sweep.unjournalled += 1,
                    // Gone already: done, not kept.
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => sweep.unjournalled += 1,
                    Err(e) => sweep.kept(&e),
                },
            }
        }
    }
    Ok(sweep)
}

#[cfg(test)]
thread_local! {
    /// Runs once, on this thread, between step 2's listing and its read of the rows (tests only).
    static BETWEEN_LISTING_AND_NAMES: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        const { std::cell::RefCell::new(None) };
}

/// Run `hook` at the next sweep on this thread, between its listing and its read of the rows.
#[cfg(test)]
pub(crate) fn between_listing_and_names_for_test(hook: impl FnOnce() + 'static) {
    BETWEEN_LISTING_AND_NAMES.with(|slot| *slot.borrow_mut() = Some(Box::new(hook)));
}

#[cfg(test)]
fn run_between_listing_and_names() {
    if let Some(hook) = BETWEEN_LISTING_AND_NAMES.with(|slot| slot.borrow_mut().take()) {
        hook();
    }
}

#[cfg(unix)]
fn is_regular_file(stat: &libc::stat) -> bool {
    stat.st_mode & libc::S_IFMT == libc::S_IFREG
}

pub fn same_bytes(a: &Path, b: &Path) -> std::io::Result<bool> {
    same_contents(std::fs::File::open(a)?, std::fs::File::open(b)?)
}

/// [`same_bytes`] against a file already open, read from its start.
fn same_bytes_as_open_file(path: &Path, open: &std::fs::File) -> std::io::Result<bool> {
    use std::io::Seek;
    let mut open = open.try_clone()?;
    open.seek(std::io::SeekFrom::Start(0))?;
    same_contents(std::fs::File::open(path)?, open)
}

/// Copy everything in `source` (from its start) into a new file at `dest`.
/// On macOS this first tries an APFS clone of the open file
/// (`fclonefileat`), which is what `std::fs::copy` does by path: instant and
/// no extra disk blocks for a large file.
fn copy_open_file(source: &std::fs::File, dest: &Path) -> std::io::Result<()> {
    use std::io::Seek;
    #[cfg(target_os = "macos")]
    {
        use std::os::fd::AsRawFd;
        use std::os::unix::ffi::OsStrExt;
        let c_dest = std::ffi::CString::new(dest.as_os_str().as_bytes())
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "path contains a NUL byte"))?;
        // SAFETY: the source descriptor is open for the call; `c_dest` is
        // NUL-terminated and lives for the call.
        let rc = unsafe { libc::fclonefileat(source.as_raw_fd(), libc::AT_FDCWD, c_dest.as_ptr(), 0) };
        if rc == 0 {
            // The clone keeps the source's mode: make the copy private through the file itself
            // (`fchmod`). A copy that cannot be made private is not kept.
            use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
            let private = std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW)
                .open(dest)
                .and_then(|copy| copy.set_permissions(std::fs::Permissions::from_mode(0o600)));
            if let Err(error) = private {
                let _ = std::fs::remove_file(dest);
                return Err(error);
            }
            return Ok(());
        }
        // Not clonable (another volume, a filesystem without clones): copy
        // the bytes below. A failed clone creates nothing at `dest`.
    }
    let mut reader = source.try_clone()?;
    reader.seek(std::io::SeekFrom::Start(0))?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut writer = options.open(dest)?;
    // A copy that fails partway is never kept: a partial plaintext copy would outlive its write,
    // and the drop would keep it as "not a duplicate". The file is this call's own (`create_new`).
    if let Err(error) = std::io::copy(&mut reader, &mut writer) {
        drop(writer);
        let _ = std::fs::remove_file(dest);
        return Err(error);
    }
    Ok(())
}

fn same_contents(mut a: std::fs::File, mut b: std::fs::File) -> std::io::Result<bool> {
    if a.metadata()?.len() != b.metadata()?.len() {
        return Ok(false);
    }
    let mut left = zeroize::Zeroizing::new([0u8; 8192]);
    let mut right = zeroize::Zeroizing::new([0u8; 8192]);
    loop {
        let n = a.read(&mut *left)?;
        if n == 0 {
            return Ok(b.read(&mut right[..1])? == 0);
        }
        b.read_exact(&mut right[..n])?;
        if left[..n] != right[..n] {
            return Ok(false);
        }
    }
}

/// Preflight all payloads BEFORE native removal, retaining the DB until unlink
/// succeeds. Legacy upload_resume paths are also inventoried, never forgotten.
#[cfg(any(target_os = "windows", test))]
pub fn signout_paths(db: &StateDb) -> anyhow::Result<Vec<Option<String>>> {
    let mut paths = Vec::new();
    for (path, source, completed) in db.staged_payloads_for_signout()? {
        let payload = Path::new(&path);
        match std::fs::symlink_metadata(payload) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                paths.push(Some(path));
                continue;
            }
            Err(e) => return Err(e.into()),
            Ok(meta) => anyhow::ensure!(
                meta.is_file() && !meta.file_type().is_symlink(),
                "Unexpected upload staging entry"
            ),
        }
        // Completed server upload is authoritative. Otherwise keep the only
        // copy; an unchanged original remains protected by native preflight.
        let duplicate = source
            .as_deref()
            .is_some_and(|source| same_bytes(payload, Path::new(source)).unwrap_or(false));
        anyhow::ensure!(
            completed || duplicate,
            "An unfinished upload has recoverable bytes at {}. Move them to a safe folder before retrying sign-out.",
            payload.display()
        );
        paths.push(Some(path));
    }
    Ok(paths)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn round5_cancelled_staging_preserves_only_unsynced_copy() {
        let temp = tempfile::tempdir().unwrap();
        let db = Arc::new(StateDb::open(temp.path().join("state.db")).unwrap());
        let source = temp.path().join("original");
        std::fs::write(&source, b"only unsynced version").unwrap();
        let staged = StagedPayload::copy(db.clone(), &source, temp.path().join("staging")).unwrap();
        let payload = staged.path().to_string();
        std::fs::write(&source, b"different newer version").unwrap();
        drop(staged);
        assert_eq!(std::fs::read(&payload).unwrap(), b"only unsynced version");
        assert_eq!(db.staged_payloads_for_signout().unwrap().len(), 1);
        assert!(signout_paths(&db).is_err(), "sign-out discarded the only unsynced copy");
        assert!(db.finish_windows_signout().is_err());
    }
    #[test]
    fn copy_from_file_reads_the_open_file_never_the_path() {
        let temp = tempfile::tempdir().unwrap();
        let db = Arc::new(StateDb::open(temp.path().join("state.db")).unwrap());
        let source = temp.path().join("handed-over");
        std::fs::write(&source, b"validated bytes").unwrap();
        let opened = std::fs::File::open(&source).unwrap();
        // The entry is replaced after it was opened: the copy must not see it.
        std::fs::rename(&source, temp.path().join("moved-away")).unwrap();
        std::fs::write(&source, b"swapped in later").unwrap();
        let staged = StagedPayload::copy_from_file(db.clone(), &opened, &source, temp.path().join("staging")).unwrap();
        assert_eq!(std::fs::read(staged.path()).unwrap(), b"validated bytes");
        staged.retain();
    }

    #[test]
    fn copy_from_file_drops_its_copy_when_the_open_file_still_holds_the_same_bytes() {
        // The handed-over entry may already be unlinked (the extension or a
        // purge removed it) when a failed enqueue drops the payload. The open
        // file still proves the daemon's copy is a duplicate, so the copy and
        // its journal row go, instead of lingering as "the only copy".
        let temp = tempfile::tempdir().unwrap();
        let db = Arc::new(StateDb::open(temp.path().join("state.db")).unwrap());
        let source = temp.path().join("handed-over");
        std::fs::write(&source, b"same bytes").unwrap();
        let opened = std::fs::File::open(&source).unwrap();
        let staged = StagedPayload::copy_from_file(db.clone(), &opened, &source, temp.path().join("staging")).unwrap();
        let path = staged.path().to_string();
        std::fs::remove_file(&source).unwrap();
        drop(staged);
        assert!(!Path::new(&path).exists(), "a duplicate of the open file is removed");
        assert_eq!(db.staged_payloads_for_signout().unwrap().len(), 0);
    }

    #[test]
    fn round5_completed_payload_cleanup_is_retryable_and_survives_restart() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("payload");
        // A directory deliberately fails unlink on all hosts.
        std::fs::create_dir(&path).unwrap();
        let db_path = temp.path().join("state.db");
        let db = StateDb::open(&db_path).unwrap();
        db.track_staged_payload(&path.to_string_lossy(), None, true).unwrap();
        assert!(remove(&db, &path).is_err());
        assert_eq!(db.staged_payloads_for_signout().unwrap().len(), 1);
        drop(db);
        let db = StateDb::open(&db_path).unwrap();
        std::fs::remove_dir(&path).unwrap();
        std::fs::write(&path, b"completed upload").unwrap();
        assert_eq!(signout_paths(&db).unwrap().len(), 1);
        remove(&db, &path).unwrap();
        db.finish_windows_signout().unwrap();
        assert_eq!(db.staged_payloads_for_signout().unwrap().len(), 0);
        assert!(!path.exists());
    }
    #[test]
    fn round5_failed_enqueue_removes_duplicate_staging() {
        let temp = tempfile::tempdir().unwrap();
        let db = Arc::new(StateDb::open(temp.path().join("state.db")).unwrap());
        let source = temp.path().join("original");
        std::fs::write(&source, b"unchanged original").unwrap();
        let staged = StagedPayload::copy(db.clone(), &source, temp.path().join("staging")).unwrap();
        assert_eq!(db.staged_payloads_for_signout().unwrap().len(), 1);
        let path = staged.path().to_string();
        drop(staged); // same error/unwind path used before a Finder enqueue succeeds
        assert!(!Path::new(&path).exists());
        assert_eq!(db.staged_payloads_for_signout().unwrap().len(), 0);
        assert_eq!(std::fs::read(source).unwrap(), b"unchanged original");
    }

    /// A private staging folder, as `durable_finder_staging_root` makes it.
    #[cfg(unix)]
    fn private_root(parent: &Path) -> PathBuf {
        use std::os::unix::fs::DirBuilderExt;
        let root = parent.join("finder-writes");
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&root)
            .unwrap();
        root
    }

    /// A copy in `root`, named as the accept names one: a fresh UUID.
    #[cfg(unix)]
    fn copy_in(root: &Path, bytes: &[u8]) -> PathBuf {
        let path = root.join(uuid::Uuid::new_v4().to_string());
        std::fs::write(&path, bytes).unwrap();
        path
    }

    /// I3: an older build's sign-out deleted the journal row and skipped the copy, or a copy
    /// lost its row some other way. No row names it, so the start removes it.
    #[cfg(unix)]
    #[test]
    fn the_start_sweep_removes_a_copy_no_row_names() {
        let temp = tempfile::tempdir().unwrap();
        let db = StateDb::open(temp.path().join("state.db")).unwrap();
        let root = private_root(temp.path());
        let stray = copy_in(&root, b"plaintext an older build left behind");
        let sweep = sweep_unreferenced(&db, &[root.clone(), temp.path().join("never-made")]).unwrap();
        assert!(!stray.exists(), "removed");
        assert_eq!(
            sweep,
            UnreferencedSweep {
                unfinished: 0,
                unjournalled: 1,
                kept: 0,
                failure: None
            },
            "a folder that does not exist is no failure"
        );
        assert!(root.is_dir(), "the folder itself stays");
    }

    /// P2-1: a crash between the journal row and the accept's transaction leaves a
    /// `completed = 0` row that no op names. Its copy and its row go; so does a row whose copy
    /// was never made.
    #[cfg(unix)]
    #[test]
    fn the_start_sweep_removes_an_unfinished_copy_and_its_row() {
        let temp = tempfile::tempdir().unwrap();
        let db = StateDb::open(temp.path().join("state.db")).unwrap();
        let root = private_root(temp.path());
        let orphan = copy_in(&root, b"an accept that never committed");
        db.track_staged_payload(&orphan.to_string_lossy(), Some("/handed-over"), false)
            .unwrap();
        let never_made = root.join(uuid::Uuid::new_v4().to_string());
        db.track_staged_payload(&never_made.to_string_lossy(), None, false)
            .unwrap();
        let sweep = sweep_unreferenced(&db, std::slice::from_ref(&root)).unwrap();
        assert!(!orphan.exists(), "the copy is removed");
        assert!(
            db.staged_payloads_for_signout().unwrap().is_empty(),
            "both rows are deleted"
        );
        assert_eq!(sweep.unfinished, 2, "{sweep:?}");
        assert_eq!(sweep.unjournalled, 0, "{sweep:?}");
        assert_eq!(sweep.failure, None);
    }

    /// A copy any of the four tables names is kept, file and row: an op's payload, a resume row,
    /// a finalization, and a released journal row (the release path removes that one, not the
    /// sweep). A `completed = 0` row that an op names is kept too.
    #[cfg(unix)]
    #[test]
    fn the_start_sweep_keeps_every_copy_a_row_names() {
        let temp = tempfile::tempdir().unwrap();
        let db = StateDb::open(temp.path().join("state.db")).unwrap();
        let root = private_root(temp.path());
        let queued = copy_in(&root, b"named by its op");
        let resumed = copy_in(&root, b"named by a resume row");
        let finalizing = copy_in(&root, b"named by a finalization");
        let released = copy_in(&root, b"named by a released journal row");
        let journalled_and_queued = copy_in(&root, b"journalled, and named by its op");
        let text = |path: &Path| path.to_string_lossy().into_owned();
        {
            let conn = db.hold_lock_for_test();
            conn.execute(
                "INSERT INTO operation_queue (op_id, kind, file_id, payload_path)
                 VALUES ('op-queued', 'upload_version', 'f-queued', ?1),
                        ('op-journalled', 'upload_version', 'f-journalled', ?2)",
                rusqlite::params![text(&queued), text(&journalled_and_queued)],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO upload_resume (op_id, payload_path, payload_size, payload_mtime_ns,
                     upload_session_id, server_file_id, object_version_id, chunk_size_bytes, chunk_count)
                 VALUES ('op-gone', ?1, 1, 1, 'session', 'server-file', 'object', 1, 1)",
                rusqlite::params![text(&resumed)],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO upload_finalizations (op_id, local_file_id, server_file_id, target_path, payload_path)
                 VALUES ('op-finalizing', 'local', 'server', 'target', ?1)",
                rusqlite::params![text(&finalizing)],
            )
            .unwrap();
        }
        db.track_staged_payload(&text(&released), None, true).unwrap();
        db.track_staged_payload(&text(&journalled_and_queued), Some("/handed-over"), false)
            .unwrap();
        let sweep = sweep_unreferenced(&db, std::slice::from_ref(&root)).unwrap();
        for (path, bytes) in [
            (&queued, &b"named by its op"[..]),
            (&resumed, b"named by a resume row"),
            (&finalizing, b"named by a finalization"),
            (&released, b"named by a released journal row"),
            (&journalled_and_queued, b"journalled, and named by its op"),
        ] {
            assert_eq!(std::fs::read(path).unwrap(), bytes, "{}", path.display());
        }
        assert_eq!(
            db.staged_payloads_for_signout().unwrap().len(),
            3,
            "the two journal rows and the resume row's path stay"
        );
        assert_eq!(sweep, UnreferencedSweep::default());
    }

    /// The sweep never follows a link: a link in the folder (to a file or a folder) is left as
    /// it is and its target is untouched, a subfolder is not entered, a journalled path that is a
    /// link keeps its row, and a staging root that is itself a link is refused.
    #[cfg(unix)]
    #[test]
    fn the_start_sweep_never_follows_a_link() {
        let temp = tempfile::tempdir().unwrap();
        let db = StateDb::open(temp.path().join("state.db")).unwrap();
        let root = private_root(&temp.path().join("data"));
        let outside = temp.path().join("outside");
        std::fs::create_dir(&outside).unwrap();
        let target = outside.join("not-ours");
        std::fs::write(&target, b"a file the sweep must not touch").unwrap();
        let file_link = root.join(uuid::Uuid::new_v4().to_string());
        std::os::unix::fs::symlink(&target, &file_link).unwrap();
        let dir_link = root.join(uuid::Uuid::new_v4().to_string());
        std::os::unix::fs::symlink(&outside, &dir_link).unwrap();
        let journalled_link = root.join(uuid::Uuid::new_v4().to_string());
        std::os::unix::fs::symlink(&target, &journalled_link).unwrap();
        db.track_staged_payload(&journalled_link.to_string_lossy(), None, false)
            .unwrap();
        let sub = root.join("sub");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("leaf"), b"below the folder").unwrap();
        let linked_root = temp.path().join("linked-root");
        std::os::unix::fs::symlink(&outside, &linked_root).unwrap();

        let sweep = sweep_unreferenced(&db, &[root.clone(), linked_root.clone()]).unwrap();

        assert_eq!(std::fs::read(&target).unwrap(), b"a file the sweep must not touch");
        for link in [&file_link, &dir_link, &journalled_link] {
            assert!(
                link.symlink_metadata().unwrap().file_type().is_symlink(),
                "a link is left as it is: {}",
                link.display()
            );
        }
        assert_eq!(std::fs::read(sub.join("leaf")).unwrap(), b"below the folder");
        assert_eq!(
            db.staged_payloads_for_signout().unwrap().len(),
            1,
            "the journalled link keeps its row"
        );
        assert_eq!(sweep.unfinished, 0, "{sweep:?}");
        assert_eq!(sweep.unjournalled, 0, "{sweep:?}");
        assert_eq!(
            sweep.failure,
            Some("permission_denied"),
            "the linked root is refused: {sweep:?}"
        );
    }

    /// P2-1: a copy that fails partway is never kept, and neither is its journal row.
    #[cfg(unix)]
    #[test]
    fn a_failed_copy_from_file_leaves_no_file_and_no_row() {
        let temp = tempfile::tempdir().unwrap();
        let db = Arc::new(StateDb::open(temp.path().join("state.db")).unwrap());
        let source = temp.path().join("handed-over");
        std::fs::write(&source, b"bytes the copy cannot read").unwrap();
        // Open for writing only: every read fails, once the copy's file exists.
        let write_only = std::fs::OpenOptions::new().write(true).open(&source).unwrap();
        let root = temp.path().join("staging");
        let result = StagedPayload::copy_from_file(db.clone(), &write_only, &source, root.clone());
        assert!(result.is_err(), "the copy failed");
        drop(result);
        let left: Vec<_> = std::fs::read_dir(&root).unwrap().collect();
        assert!(left.is_empty(), "no partial copy is kept: {left:?}");
        assert!(
            db.staged_payloads_for_signout().unwrap().is_empty(),
            "and no journal row"
        );
    }

    /// Security P3-3: the copy is private (`0600`) whatever the source's mode. An APFS clone keeps
    /// the source's mode, so the copy is `fchmod`ed after it.
    #[cfg(unix)]
    #[test]
    fn a_copy_is_private_whatever_the_sources_mode() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let db = Arc::new(StateDb::open(temp.path().join("state.db")).unwrap());
        let source = temp.path().join("handed-over");
        std::fs::write(&source, b"plaintext").unwrap();
        std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o644)).unwrap();
        let opened = std::fs::File::open(&source).unwrap();
        let staged = StagedPayload::copy_from_file(db, &opened, &source, temp.path().join("staging")).unwrap();
        let mode = std::fs::metadata(staged.path()).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "{mode:o}");
        staged.retain();
    }

    /// Spec §8.7 S6: an accept still running that journals its row and makes its copy after
    /// step 2 listed the folder, but before it read the rows, keeps its copy, and its accept then
    /// queues. With the rows read before the listing, the copy would be unnamed and unlinked.
    #[cfg(unix)]
    #[test]
    fn a_copy_made_between_the_listing_and_the_rows_survives_and_its_accept_queues() {
        use crate::state_db::{AcceptOutcome, FileEntry, FileStatus, FinderAccept, FinderAcceptKind, ItemKind};
        let temp = tempfile::tempdir().unwrap();
        let db = Arc::new(StateDb::open(temp.path().join("state.db")).unwrap());
        let root = private_root(temp.path());
        db.upsert_file(&FileEntry {
            file_id: "mid-accept".into(),
            path: "notes.txt".into(),
            status: FileStatus::Local,
            size_bytes: 4,
            modified_at: 1,
            content_hash: None,
            remote_updated_at: 1,
            parent_id: None,
            item_kind: ItemKind::File,
        })
        .unwrap();
        let mut contract = db.get_file_contract_state("mid-accept").unwrap().unwrap();
        contract.current_version = 1;
        db.set_file_contract_state(&contract).unwrap();
        let copy = root.join(uuid::Uuid::new_v4().to_string());
        let copy_path = copy.to_string_lossy().into_owned();
        let (accepting, copy_at_seam, path_at_seam) = (db.clone(), copy.clone(), copy_path.clone());
        between_listing_and_names_for_test(move || {
            // The accept's journal row, then its copy (`StagedPayload`'s order).
            accepting
                .track_staged_payload(&path_at_seam, Some("/handed-over"), false)
                .unwrap();
            std::fs::write(&copy_at_seam, b"a save mid-accept").unwrap();
        });
        let sweep = sweep_unreferenced(&db, std::slice::from_ref(&root)).unwrap();
        assert!(copy.exists(), "the copy survives: {sweep:?}");
        let accepted = db
            .accept_finder_write(
                &FinderAccept {
                    op_id: "op-mid-accept",
                    file_id: "mid-accept",
                    kind: FinderAcceptKind::Modify {
                        incoming_base: Some("1"),
                    },
                    parent_id: None,
                    target_path: Some("notes.txt"),
                    metadata_json: "{}",
                    payload_path: &copy_path,
                    size_bytes: 17,
                    modified_at: 2,
                    backup_source_key: None,
                    now: 3,
                },
                &|_, _| Vec::new(),
            )
            .unwrap();
        assert!(matches!(accepted, AcceptOutcome::Queued { .. }), "{accepted:?}");
        assert_eq!(
            db.list_operations_for_file("mid-accept").unwrap()[0]
                .payload_path
                .as_deref(),
            Some(copy_path.as_str())
        );
        assert_eq!(std::fs::read(&copy).unwrap(), b"a save mid-accept");
    }
}
