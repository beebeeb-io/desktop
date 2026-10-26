//! Local recording never waits for network credit, and never hydrates a partial placeholder.
use crate::state_db::{StateDb, UnknownRow};
use std::path::Path;

pub(super) fn stamp(path: &Path) -> std::io::Result<(i64, i64)> {
    let m = std::fs::metadata(path)?;
    if !m.is_file() {
        return Err(std::io::Error::other("not a file"));
    }
    Ok((
        i64::try_from(m.len()).map_err(std::io::Error::other)?,
        i64::try_from(
            m.modified()?
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(std::io::Error::other)?
                .as_nanos(),
        )
        .map_err(std::io::Error::other)?,
    ))
}

#[cfg(test)]
pub(super) fn record_one(
    db: &StateDb,
    row: &UnknownRow,
    path: &Path,
    after_read: impl FnOnce(),
) -> anyhow::Result<bool> {
    record_checked(db, row, path, after_read, None)
}
pub(super) fn record_checked(
    db: &StateDb,
    row: &UnknownRow,
    path: &Path,
    after_read: impl FnOnce(),
    scope: Option<&super::job::Scope>,
) -> anyhow::Result<bool> {
    use sha2::{Digest, Sha256};
    let before = stamp(path)?;
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options
            .share_mode(3)
            .custom_flags(windows::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT.0);
    }
    let mut f = options.open(path)?;
    if !super::resolve::owns(&f, &row.file_id) {
        return Ok(false);
    }
    let mut h = Sha256::new();
    std::io::copy(&mut f, &mut h)?;
    after_read();
    if stamp(path)? != before {
        return Ok(false);
    }
    let mut record = row.record.clone();
    record.recorded_hash = Some(format!("{:x}", h.finalize()));
    record.recorded_size = Some(before.0);
    record.recorded_mtime = Some(before.1);
    let save = || {
        db.record_unknown_once(&row.file_id, &row.path, &record)
            .map_err(Into::into)
    };
    match scope {
        Some(s) => s.commit(save),
        None => save(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state_db::{FileEntry, FileStatus, ItemKind};
    #[test]
    fn task_2191_s2_records_once_and_discards_a_read_during_write() {
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        let path = dir.path().join("a.txt");
        std::fs::write(&path, b"old").unwrap();
        db.upsert_file(&FileEntry {
            file_id: "u".into(),
            path: "a.txt".into(),
            status: FileStatus::Local,
            size_bytes: 3,
            modified_at: 0,
            content_hash: None,
            remote_updated_at: 0,
            parent_id: None,
            item_kind: ItemKind::File,
        })
        .unwrap();
        let row = db.list_unknown_to_record(1).unwrap().remove(0);
        assert!(!record_one(&db, &row, &path, || std::fs::write(&path, b"newer").unwrap()).unwrap());
        assert_eq!(db.list_unknown_to_record(1).unwrap().len(), 1);
        assert!(record_one(&db, &row, &path, || {}).unwrap());
        assert!(db.list_unknown_to_record(1).unwrap().is_empty());
        let r = db.list_unknown_to_check(0, 1).unwrap().remove(0).record;
        assert_eq!(r.recorded_size, Some(5));
        use sha2::{Digest, Sha256};
        assert_eq!(r.recorded_hash, Some(format!("{:x}", Sha256::digest(b"newer"))));
    }
}
