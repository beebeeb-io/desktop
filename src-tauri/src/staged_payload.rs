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
                tracing::warn!(%error, "staged payload cleanup deferred; journal retained");
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
            Err(e) => tracing::warn!(error = %e, "released upload copy kept; removal is retried at the next start"),
        }
    }
    removed
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
    std::io::copy(&mut reader, &mut writer)?;
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
}
