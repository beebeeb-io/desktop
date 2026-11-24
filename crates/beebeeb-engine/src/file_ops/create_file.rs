//! New file (DW-1522): a new file named by the person, made in a folder of the sync root, holding the bytes it was
//! given (empty for a new text or Markdown file). The engine uploads it as any new file in the folder: its watcher
//! sees it, and core encrypts it on this PC first.
//!
//! The 0.8 desktop's rules, unchanged: the name is checked as every other name is ([`super::validate_name`]), a
//! read-only share and the Backups tree refuse, and a taken name is refused, never overwritten:
//! - against the folder's names, on disk and in the mirror, in any case ([`super::name_taken`]);
//! - against what else the name reaches on Windows, an 8.3 short name (task 2407, [`name_alias::conflict_below`]);
//! - and the file is given its name without replacing anything, so an entry that appeared since those checks is a
//!   taken name too.
//!
//! The bytes are written to a partial name the folder watcher ignores, and the file gets its name only when it is
//! whole: the watcher never sees a half file. A write that fails part way (the disk filled) leaves nothing behind: a
//! half file would be uploaded as the file, and would hold the name the retry needs.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::{folder_rel, full_path, join_rel, name_taken, refuse_if_shared_read_only, validate_name};
use crate::backup_protect;
use crate::drive_v2::DriveError;
use crate::name_alias;
use crate::state_db::{FileEntry, StateDb};

/// The most bytes a new file is made with: the 0.8 desktop's limit for what its viewers and editors hand over.
pub const CREATE_FILE_LIMIT: u64 = 256 * 1024 * 1024;

/// Make a file named `name` in `parent` (`None`: the top of the sync root) holding `bytes`. Returns its state DB
/// path. Refused: an invalid name ([`DriveError::InvalidName`]), more than [`CREATE_FILE_LIMIT`] bytes
/// ([`DriveError::TooLarge`]), a read-only share or the Backups tree ([`DriveError::ReadOnly`]), a taken name or one
/// that reaches another entry ([`DriveError::NameTaken`]), a name that could not be checked
/// ([`DriveError::Failed`]). A full disk is [`DriveError::Quota`], and nothing of the file stays.
pub fn create_file(
    db: &StateDb,
    sync_root: &Path,
    parent: Option<&FileEntry>,
    name: &str,
    bytes: &[u8],
) -> Result<String, DriveError> {
    create_file_with(db, sync_root, parent, name, bytes, CREATE_FILE_LIMIT, write_and_sync)
}

/// The file [`create_file`] just made at `rel`, listed and queued NOW (w36): its row is written and its upload queued
/// before this returns, by the one sequence the folder scan uses ([`crate::watcher::dispatch_local_create`]). Without
/// it the file has no row until the next scan finds it (8 s, longer on a large folder), so no list shows it, while a
/// second New file of the same name is refused as taken by the file on disk. `false`: not queued here (the scan
/// still finds it, as before).
pub fn list_new_file(bridge: &crate::engine_bridge::EngineBridge, sync_root: &Path, rel: &str) -> bool {
    match full_path(sync_root, rel) {
        Ok(path) => crate::watcher::dispatch_local_create(bridge, sync_root, &path, "new_file"),
        Err(_) => false,
    }
}

/// The bytes, on the disk: written, then synced (a write the disk only took into its cache is not made yet).
fn write_and_sync(file: &mut File, bytes: &[u8]) -> io::Result<()> {
    file.write_all(bytes)?;
    file.sync_all()
}

/// [`create_file`] with its limit and its write given: the tests' way to a full disk and to the limit, without
/// filling a disk or 256 MiB of memory. Every check and the clean-up are this function's own.
fn create_file_with(
    db: &StateDb,
    sync_root: &Path,
    parent: Option<&FileEntry>,
    name: &str,
    bytes: &[u8],
    limit: u64,
    write: impl FnOnce(&mut File, &[u8]) -> io::Result<()>,
) -> Result<String, DriveError> {
    let name = validate_name(name)?;
    if bytes.len() as u64 > limit {
        return Err(DriveError::TooLarge);
    }
    refuse_if_shared_read_only(db, parent)?;
    let parent_rel = folder_rel(parent);
    let rel = join_rel(&parent_rel, &name);
    if backup_protect::protected_destination(db, &rel).is_some() {
        return Err(DriveError::ReadOnly);
    }
    if name_taken(db, sync_root, parent.map(|p| p.file_id.as_str()), &parent_rel, &name) {
        return Err(DriveError::NameTaken);
    }
    crate::drive_v2::case_place::refuse_shared_place(db, sync_root, parent)?;
    let path = full_path(sync_root, &rel)?;
    match name_alias::conflict_below(sync_root, &path) {
        Ok(None) => {}
        Ok(Some(_)) => return Err(DriveError::NameTaken),
        Err(_) => return Err(DriveError::Failed("name check")),
    }
    write_new(&path, bytes, write)?;
    Ok(rel)
}

/// A name the watcher and the scan ignore (`.part`, [`crate::engine_bridge::is_ignored_finder_name`]), beside `path`
/// (the same folder, so the move to the final name stays on one volume). Never a name a person made.
fn partial_path(path: &Path, attempt: u32) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let name = format!(".beebeeb-new-{}-{n}-{attempt}.part", std::process::id());
    path.with_file_name(name)
}

/// Make `path` hold `bytes` as a NEW file. The bytes are written to a partial name the folder watcher ignores, then
/// the file is given its name WITHOUT replacing an entry already there (a hard link: it fails on a taken name, where
/// a rename replaces on Windows), so the watcher sees the whole file or none, never a half file. An entry already
/// there is a taken name (`AlreadyExists`), never truncated. A write that fails part way, or a name taken since the
/// checks, removes the partial file: it is ours, `create_new` made it a moment ago.
fn write_new(
    path: &Path,
    bytes: &[u8],
    write: impl FnOnce(&mut File, &[u8]) -> io::Result<()>,
) -> Result<(), DriveError> {
    let (mut file, partial) = open_partial(path)?;
    if let Err(error) = write(&mut file, bytes) {
        drop(file);
        // Best effort: when even the removal fails, the write's own error is still the answer.
        let _ = std::fs::remove_file(&partial);
        return Err(DriveError::io(error));
    }
    drop(file);
    let named = give_name(&partial, path);
    // The partial name goes either way: after a link it is the second name of the finished file.
    let _ = std::fs::remove_file(&partial);
    named
}

/// Open a fresh partial file beside `path`.
fn open_partial(path: &Path) -> Result<(File, PathBuf), DriveError> {
    for attempt in 0..8 {
        let partial = partial_path(path, attempt);
        match OpenOptions::new().write(true).create_new(true).open(&partial) {
            Ok(file) => return Ok((file, partial)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(DriveError::io(error)),
        }
    }
    // Eight partial names all taken is not a taken name of the person's: it is a failure.
    Err(DriveError::Failed("disk"))
}

/// `partial` becomes `path`, which must not exist: a taken name is [`DriveError::NameTaken`].
fn give_name(partial: &Path, path: &Path) -> Result<(), DriveError> {
    match std::fs::hard_link(partial, path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Err(DriveError::NameTaken),
        // A folder that cannot link (FAT, a network share): check, then rename. The window between the two is the
        // only one left, and only on such a folder.
        Err(_) => {
            if std::fs::symlink_metadata(path).is_ok() {
                return Err(DriveError::NameTaken);
            }
            std::fs::rename(partial, path).map_err(DriveError::io)
        }
    }
}

#[cfg(test)]
#[path = "create_file_tests.rs"]
mod tests;
