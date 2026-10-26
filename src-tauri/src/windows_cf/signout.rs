//! Fail-closed Windows account cleanup. Prepare every owned item before
//! deleting any: an untracked, dirty, busy or inaccessible item refuses sign-out.
//! Exclusive Win32 handles bind the identity/dirty check and deletion to the same
//! file, so a save or path replacement cannot turn cleanup into data loss.
#[path = "signout_cleanup.rs"]
mod cleanup;

use crate::state_db::StateDb;
use std::collections::HashMap;
use std::os::windows::{
    fs::{MetadataExt, OpenOptionsExt},
    io::AsRawHandle,
};
use std::path::Path;
use windows::Win32::Foundation::{BOOLEAN, HANDLE};
use windows::Win32::Storage::{CloudFilters::*, FileSystem::*};

/// Task 2191 (spec D11; Decision (lead, 2026-10-26)): what sign-out does with files of unknown
/// content (synced before task 1640) that hold bytes on this PC. The person chose it in the
/// sign-out confirmation; nothing is decided from the platform's in-sync state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnknownFiles {
    /// Copy each one to this folder beside the sync folder first, then delete the original.
    Keep { dir: std::path::PathBuf },
    /// Delete them with the rest.
    Delete,
}

/// What the purge kept (`UnknownFiles::Keep`): how many files, and where.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct KeptAtSignOut {
    pub files: usize,
    pub dir: Option<std::path::PathBuf>,
}

/// Task 2191 (D11): the relative paths (as `prepare` keys them, `\`-joined) of the files of
/// unknown content and of the placeholders recorded as left at another path.
fn unknown_paths(db: &StateDb) -> anyhow::Result<std::collections::HashSet<String>> {
    let key = |path: &str| path.trim_start_matches('/').replace('/', "\\");
    let mut paths: std::collections::HashSet<String> = db.list_guarded()?.iter().map(|row| key(&row.path)).collect();
    paths.extend(db.list_unknown_orphans()?.iter().map(|(path, _, _)| key(path)));
    Ok(paths)
}

/// A file of unknown content opened for the purge, and what its copy holds.
struct Kept {
    file: std::fs::File,
    size: u64,
    ranges: Option<Vec<(u64, u64)>>,
    hash: String,
}

pub fn purge(db: &StateDb, root: Option<&Path>, unknown: &UnknownFiles) -> anyhow::Result<KeptAtSignOut> {
    if let Some(root) = root {
        super::upload_finalization::retry(db, root);
    }
    anyhow::ensure!(db.upload_finalizations()?.is_empty(),
        "An uploaded file still needs its local identity repaired. Close open files, unlock and retry sign-out.");
    db.windows_signout_preflight().map_err(|_| {
        anyhow::anyhow!("Pending changes remain. Unlock, finish syncing and resolve failed changes before signing out.")
    })?;
    let rows = db.list_files()?;
    let mut cache_paths = rows
        .iter()
        .map(|row| {
            db.get_file_contract_state(&row.file_id)
                .map(|state| state.and_then(|state| state.cache_path))
        })
        .collect::<Result<Vec<_>, _>>()?;
    cache_paths.extend(crate::staged_payload::signout_paths(db)?);
    let mut kept_files = 0usize;
    cleanup::purge(
        cache_paths,
        crate::is_disposable_cache_path,
        || {
            let mut handles = Vec::new();
            let mut directories = Vec::new();
            let mut kept: Vec<Kept> = Vec::new();
            let unknown_set = unknown_paths(db)?;
            if let Some(root) = root {
                let known: HashMap<_, _> = rows
                    .iter()
                    .map(|r| (r.path.trim_start_matches('/').replace('/', "\\"), r))
                    .collect();
                anyhow::ensure!(
                    root.is_dir(),
                    "Sync folder is unavailable; restore it before signing out"
                );
                prepare(root, root, &known, &unknown_set, unknown, &mut kept, &mut handles, &mut directories)?;
            } else if !rows.is_empty() {
                anyhow::bail!("Cannot locate the sync folder. Restore its location before signing out.");
            }

            // Task 2191 (D11): every copy was made in `prepare`, before any deletion. An original
            // goes only while it still holds what was copied, re-read through the same handle
            // (only the ranges on this PC, for a partly downloaded one).
            for entry in &mut kept {
                let now = crate::unknown_check::hash_source(&mut entry.file, entry.size, entry.ranges.as_deref())?;
                anyhow::ensure!(
                    now == entry.hash,
                    "A file changed while sign-out was copying it. Unlock and retry sign-out."
                );
            }
            for entry in &kept {
                mark_for_deletion(&entry.file)?;
            }
            kept_files = kept.len();
            drop(kept);
            // Delete only files proven clean while their exclusive handle is held.
            // No dehydrate/remove fallback: dirty bytes never qualify for this list.
            for file in &handles {
                mark_for_deletion(file)?;
            }
            drop(handles);
            // Never recursively remove directories: newly created children make this
            // fail, leaving the account and DB available for recovery/retry.
            while let Some(directory) = directories.pop() {
                mark_for_deletion(&directory)?;
                // Close each child before marking its parent for deletion.
                drop(directory);
            }
            if let Some(root) = root {
                if root.exists() && std::fs::read_dir(root)?.next().is_some() {
                    anyhow::bail!("New files appeared in the sync folder. Sync or move them out before signing out.");
                }
            }
            Ok(())
        },
        || {
            // cleanup::purge has unlinked every inventoried payload. Keep rows
            // until here so a native/unlink failure can be retried safely.
            for (path, _, _) in db.staged_payloads_for_signout()? { db.forget_staged_payload(&path)?; }
            Ok(db.finish_windows_signout()?)
        },
    )?;
    Ok(KeptAtSignOut {
        files: kept_files,
        dir: match unknown {
            UnknownFiles::Keep { dir } if kept_files > 0 => Some(dir.clone()),
            _ => None,
        },
    })
}

#[allow(clippy::too_many_arguments)]
fn prepare(
    root: &Path,
    dir: &Path,
    known: &HashMap<String, &crate::state_db::FileEntry>,
    unknown_set: &std::collections::HashSet<String>,
    unknown: &UnknownFiles,
    kept: &mut Vec<Kept>,
    handles: &mut Vec<std::fs::File>,
    directories: &mut Vec<std::fs::File>,
) -> anyhow::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        let relative = path.strip_prefix(root)?.to_string_lossy().replace('/', "\\");
        // Task 2191 (D11): a file of unknown content, or a placeholder left at another path that
        // holds unknown bytes, is never deleted unasked: decided from the database, never from
        // the platform's in-sync state (which 0.8.12 set over edits).
        if unknown_set.contains(&relative) && !path.is_dir() {
            prepare_unknown(&path, &relative, unknown, kept, handles)?;
            continue;
        }
        let row = known.get(&relative).ok_or_else(|| anyhow::anyhow!(
            "Unsynced files remain in the sync folder. Sync them or move them outside Beebeeb, then unlock and retry sign-out."
        ))?;
        let file = std::fs::OpenOptions::new()
            .access_mode(FILE_READ_ATTRIBUTES.0 | DELETE.0)
            // A folder must permit enumeration while we hold it against
            // replacement/reparse writes. File content remains exclusive.
            .share_mode(if row.is_dir() { FILE_SHARE_READ.0 } else { 0 })
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0 | FILE_FLAG_BACKUP_SEMANTICS.0)
            .open(&path)?;
        let metadata = file.metadata()?;
        anyhow::ensure!(metadata.is_dir() == row.is_dir(), "Item type changed; sign-out refused");
        // ensure_local_folder leaves successfully synced local folders as
        // ordinary directories. Only these may omit a CFAPI identity; never
        // treat a junction, symlink, or ordinary file as a clean placeholder.
        let plain_directory = metadata.is_dir() && metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 == 0;
        if !plain_directory {
            validate_placeholder(&file, &row.file_id)?;
        }
        if metadata.is_dir() {
            directories.push(file);
            prepare(root, &path, known, unknown_set, unknown, kept, handles, directories)?;
        } else {
            handles.push(file);
        }
    }
    Ok(())
}

/// Task 2191 (D11): open a file of unknown content exclusively, with read access. With
/// `UnknownFiles::Keep`, copy what is on this PC (the ranges on disk, for a partly downloaded
/// placeholder: the engine is stopped, so nothing can be fetched) before any deletion; with
/// `Delete`, it goes with the rest. A placeholder with nothing on disk holds nothing to keep.
fn prepare_unknown(
    path: &Path,
    relative: &str,
    unknown: &UnknownFiles,
    kept: &mut Vec<Kept>,
    handles: &mut Vec<std::fs::File>,
) -> anyhow::Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES.0 | FILE_READ_DATA.0 | DELETE.0)
        .share_mode(0)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0 | FILE_FLAG_BACKUP_SEMANTICS.0)
        .open(path)?;
    let metadata = file.metadata()?;
    let size = metadata.len();
    let placeholder = metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0;
    let ranges = if placeholder && !super::placeholders::fully_on_disk(path) {
        Some(super::placeholders::on_disk_ranges(&file, size)?)
    } else {
        None
    };
    let holds_nothing = ranges.as_ref().is_some_and(|ranges| ranges.iter().all(|(_, length)| *length == 0));
    match unknown {
        UnknownFiles::Keep { dir } if !holds_nothing => {
            let (hash, _) = crate::unknown_check::copy_out(&mut file, size, ranges.as_deref(), &dir.join(relative))
                .map_err(|e| anyhow::anyhow!("Couldn’t keep a copy of a file before signing out ({e}). Free some space, then retry sign-out."))?;
            kept.push(Kept { file, size, ranges, hash });
        }
        _ => handles.push(file),
    }
    Ok(())
}

// Directory deletion through this same handle is non-recursive. A child that
// arrives after prepare makes this fail without erasing that child's bytes.
fn mark_for_deletion(file: &std::fs::File) -> anyhow::Result<()> {
    let info = FILE_DISPOSITION_INFO { DeleteFile: BOOLEAN(1) };
    unsafe {
        SetFileInformationByHandle(
            HANDLE(file.as_raw_handle()),
            FileDispositionInfo,
            &info as *const _ as *const std::ffi::c_void,
            std::mem::size_of_val(&info) as u32,
        )?;
    }
    Ok(())
}

fn validate_placeholder(file: &std::fs::File, file_id: &str) -> anyhow::Result<()> {
    // Aligned buffer with room for the maximum Cloud Files identity (4 KiB).
    let mut info_buffer = vec![0u64; 1024];
    unsafe {
        CfGetPlaceholderInfo(
            HANDLE(file.as_raw_handle()),
            CF_PLACEHOLDER_INFO_STANDARD,
            info_buffer.as_mut_ptr().cast(),
            (info_buffer.len() * 8) as u32,
            None,
        )?;
        let info = &*(info_buffer.as_ptr().cast::<CF_PLACEHOLDER_STANDARD_INFO>());
        anyhow::ensure!(
            info.FileIdentityLength as usize == file_id.len(),
            "Placeholder identity changed; sign-out refused"
        );
        let identity = std::slice::from_raw_parts(info.FileIdentity.as_ptr(), info.FileIdentityLength as usize);
        anyhow::ensure!(
            identity == file_id.as_bytes(),
            "Placeholder belongs to another file; sign-out refused"
        );
        anyhow::ensure!(
            info.InSyncState == CF_IN_SYNC_STATE_IN_SYNC && info.ModifiedDataSize == 0,
            "Unsynced changes remain. Sync them or move a copy outside Beebeeb, then unlock and retry sign-out."
        );
    }
    Ok(())
}

#[cfg(test)]
#[path = "signout_tests.rs"]
mod tests;
