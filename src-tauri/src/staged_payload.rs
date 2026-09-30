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
            retained: false,
        };
        std::fs::copy(source, &staged.path)?;
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
        let duplicate = same_bytes(Path::new(&self.path), &self.source).unwrap_or(false);
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
pub fn same_bytes(a: &Path, b: &Path) -> std::io::Result<bool> {
    let mut a = std::fs::File::open(a)?;
    let mut b = std::fs::File::open(b)?;
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
