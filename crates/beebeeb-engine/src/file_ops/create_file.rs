//! New file (DW-1522): a new file named by the person, made in a folder of the sync root, holding the bytes it was
//! given (empty for a new text or Markdown file). The engine uploads it as any new file in the folder: its watcher
//! sees it, and core encrypts it on this PC first.
//!
//! The 0.8 desktop's rules, unchanged: the name is checked as every other name is ([`super::validate_name`]), a
//! read-only share and the Backups tree refuse, and a taken name is refused, never overwritten:
//! - against the folder's names, on disk and in the mirror, in any case ([`super::name_taken`]);
//! - against what else the name reaches on Windows, an 8.3 short name (task 2407, [`name_alias::conflict_below`]);
//! - and the file is opened `create_new`, so an entry that appeared since those checks is a taken name too.
//!
//! A write that fails part way (the disk filled) leaves nothing behind: a half file would be uploaded as the file,
//! and would hold the name the retry needs.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

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

/// Open `path` as a NEW file and write `bytes` into it. An entry already there is a taken name (`AlreadyExists`),
/// never truncated. A write that fails part way removes the file: it is ours, `create_new` made it a moment ago.
fn write_new(
    path: &Path,
    bytes: &[u8],
    write: impl FnOnce(&mut File, &[u8]) -> io::Result<()>,
) -> Result<(), DriveError> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(DriveError::io)?;
    if let Err(error) = write(&mut file, bytes) {
        drop(file);
        // Best effort: when even the removal fails, the write's own error is still the answer.
        let _ = std::fs::remove_file(path);
        return Err(DriveError::io(error));
    }
    Ok(())
}

#[cfg(test)]
#[path = "create_file_tests.rs"]
mod tests;
