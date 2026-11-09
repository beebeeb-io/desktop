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
use windows::Win32::Foundation::HANDLE;
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

/// Slice 11: unsent work a sign-out cannot keep is refused in the person's words, with one plain
/// way forward. Never our internals: no unlock, no syncing, no resolving, no "failed changes".
const UNSENT_WORK_REFUSAL: &str =
    "Some files you changed haven’t been sent yet. Move them out of the Beebeeb folder and sign out again.";

/// Slice 11: a copy that cannot be made stops the sign-out with what the person can do about it.
const KEEP_COPY_FAILED: &str =
    "Beebeeb couldn’t keep a copy of a file on this PC, so it didn’t sign out. Free some space, then sign out again.";

/// Task 2191 (D11): what the database says is at each path that may hold unknown bytes (keyed as
/// `prepare` keys paths, `\`-joined): a file of unknown content's own path, or a place recorded
/// as possibly holding another file's. Never trusted without the identity there (self-review).
fn unknown_paths(db: &StateDb) -> anyhow::Result<HashMap<String, crate::unknown_check::ExpectedAt>> {
    use crate::unknown_check::ExpectedAt;
    let key = |path: &str| path.trim_start_matches('/').replace('/', "\\");
    let mut paths = HashMap::new();
    for (path, _, _) in db.list_unknown_orphans()? {
        paths.insert(key(&path), ExpectedAt::OrphanCandidate);
    }
    for row in db.list_guarded()? {
        paths.insert(key(&row.path), ExpectedAt::Guarded(row.file_id));
    }
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
    // Tasks 2270 / 2280 (spec X1): a row held for review does not block sign-out; the person cannot act on it. Its
    // staged copy goes with every other staged copy below (`staged_payloads`, not this journal, lists them), and the
    // row itself with the account data once the sign-out goes ahead (`finish_windows_signout`), never before: a
    // sign-out refused below keeps it for the tray and the next start (self-review of #195).
    anyhow::ensure!(
        db.upload_finalizations()?.iter().all(|row| row.review.is_some()),
        "An uploaded file is still open in another app. Close it and sign out again."
    );
    // Slice 11: the pass above first tries to settle what it can. If work remains, a sign-out that
    // may keep copies keeps them and goes ahead; one that may not says so plainly. It never sends
    // the person away with our homework.
    let keep_dir = match (db.windows_signout_preflight().is_ok(), unknown) {
        (true, _) => None,
        (false, UnknownFiles::Keep { dir }) => Some(dir.clone()),
        (false, UnknownFiles::Delete) => anyhow::bail!("{}", UNSENT_WORK_REFUSAL),
    };
    let rows = db.list_files()?;
    let mut cache_paths = rows
        .iter()
        .map(|row| {
            db.get_file_contract_state(&row.file_id)
                .map(|state| state.and_then(|state| state.cache_path))
        })
        .collect::<Result<Vec<_>, _>>()?;
    // Slice 11: a keeping sign-out copies every queued plaintext below before anything goes, so
    // the inventory it needs is the plain list. A deleting sign-out keeps the older protection:
    // an upload whose only copy is the staged one refuses before any deletion.
    let staged: Vec<(String, Option<String>, bool)> = db.staged_payloads_for_signout()?;
    cache_paths.extend(if keep_dir.is_some() {
        staged.iter().map(|(path, _, _)| Some(path.clone())).collect()
    } else {
        crate::staged_payload::signout_paths(db)?
    });
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
                    "The Beebeeb folder isn’t available. Restore its location and sign out again."
                );
                prepare(
                    db,
                    root,
                    root,
                    &known,
                    &unknown_set,
                    unknown,
                    keep_dir.as_deref(),
                    &mut kept,
                    &mut handles,
                    &mut directories,
                )?;
            } else if !rows.is_empty() {
                anyhow::bail!("Beebeeb can’t find its folder on this PC. Restore its location and sign out again.");
            }
            // Slice 11: the queued plaintext copies are the only copy of some unsent work. Keep
            // them before the queue's rows are cleared.
            let staged_kept = match keep_dir.as_deref() {
                Some(dir) => keep_staged_payloads(&staged, dir)?,
                None => 0,
            };

            // Task 2191 (D11): every copy was made in `prepare`, before any deletion. An original
            // goes only while it still holds what was copied, re-read through the same handle
            // (only the ranges on this PC, for a partly downloaded one).
            for entry in &mut kept {
                let now = crate::unknown_check::hash_source(&mut entry.file, entry.size, entry.ranges.as_deref())?;
                anyhow::ensure!(
                    now == entry.hash,
                    "A file changed while Beebeeb was keeping a copy. Sign out again to keep the newest version."
                );
            }
            kept_files = kept.len() + staged_kept;
            for entry in &kept {
                mark_for_deletion(&entry.file)?;
            }
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
                    anyhow::bail!(
                        "New files appeared in the Beebeeb folder during sign-out. Sign out again to keep them."
                    );
                }
            }
            Ok(())
        },
        || {
            // Slice 11: keeping unsent work means the queue and its copies may still be here. Their
            // bytes are in the kept folder now, so clear the rows before the account data goes.
            if keep_dir.is_some() {
                let purged = db.purge_all_local_state().map_err(|_| {
                    anyhow::anyhow!("Beebeeb kept a copy, but couldn’t finish signing out. Sign out again to finish.")
                })?;
                for path in purged.payload_paths.iter().chain(purged.cache_paths.iter()) {
                    match std::fs::remove_file(path) {
                        Ok(()) => (),
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                        Err(_) => anyhow::bail!(
                            "Beebeeb kept a copy, but couldn’t finish signing out. Sign out again to finish."
                        ),
                    }
                }
            }
            // cleanup::purge has unlinked every inventoried payload. Keep rows
            // until here so a native/unlink failure can be retried safely.
            for (path, _, _) in db.staged_payloads_for_signout()? {
                db.forget_staged_payload(&path)?;
            }
            db.finish_windows_signout().map_err(|_| {
                if keep_dir.is_some() {
                    anyhow::anyhow!("Beebeeb kept a copy, but couldn’t finish signing out. Sign out again to finish.")
                } else {
                    anyhow::anyhow!("Beebeeb couldn’t finish signing out. Sign out again to finish.")
                }
            })
        },
    )
    // Self-review: a sign-out that stops after copying says where the copies are, so the person
    // is never left with unencrypted copies nobody told them about.
    .map_err(|e| match unknown {
        UnknownFiles::Keep { dir } if dir.is_dir() => {
            anyhow::anyhow!(
                "{e} Beebeeb kept the files it couldn’t send. They are in {}.",
                dir.display()
            )
        }
        _ => e,
    })?;
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
    db: &StateDb,
    root: &Path,
    dir: &Path,
    known: &HashMap<String, &crate::state_db::FileEntry>,
    unknown_set: &HashMap<String, crate::unknown_check::ExpectedAt>,
    unknown: &UnknownFiles,
    keep_dir: Option<&Path>,
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
        if let Some(expected) = unknown_set.get(&relative).filter(|_| !path.is_dir()) {
            if prepare_unknown(db, &path, &relative, expected, unknown, kept, handles)? {
                continue;
            }
        }
        let Some(row) = known.get(&relative) else {
            // Slice 11: with a keep folder, a file or folder the account has no row for is kept
            // and removed; without one, the person is told one plain way forward.
            if let Some(keep_dir) = keep_dir {
                keep_untracked(&path, &relative, keep_dir, kept, directories)?;
                continue;
            }
            anyhow::bail!(
                "Some files in the Beebeeb folder aren’t on this PC’s list. Move them out of the folder and sign out again."
            );
        };
        let mut access = FILE_READ_ATTRIBUTES.0 | DELETE.0;
        if keep_dir.is_some() && !row.is_dir() {
            access |= FILE_READ_DATA.0;
        }
        let file = std::fs::OpenOptions::new()
            .access_mode(access)
            // A folder must permit enumeration while we hold it against
            // replacement/reparse writes. File content remains exclusive.
            .share_mode(if row.is_dir() { FILE_SHARE_READ.0 } else { 0 })
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0 | FILE_FLAG_BACKUP_SEMANTICS.0)
            .open(&path)?;
        let metadata = file.metadata()?;
        anyhow::ensure!(
            metadata.is_dir() == row.is_dir(),
            "The item at {} changed kind during sign-out. Sign out again.",
            relative
        );
        // ensure_local_folder leaves successfully synced local folders as
        // ordinary directories. Only these may omit a CFAPI identity; never
        // treat a junction, symlink, or ordinary file as a clean placeholder.
        let plain_directory = metadata.is_dir() && metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 == 0;
        if !plain_directory {
            if let Err(error) = validate_placeholder(&file, &row.file_id) {
                let Some(keep_dir) = keep_dir else { return Err(error) };
                if metadata.is_dir() {
                    // A junction/symlink is removed, never followed; its target is not ours.
                    directories.push(file);
                } else if let Some(entry) = keep_pending_file(file, &relative, keep_dir)? {
                    kept.push(entry);
                }
                continue;
            }
        }
        if metadata.is_dir() {
            directories.push(file);
            prepare(
                db,
                root,
                &path,
                known,
                unknown_set,
                unknown,
                keep_dir,
                kept,
                handles,
                directories,
            )?;
        } else {
            handles.push(file);
        }
    }
    Ok(())
}

/// Slice 11: keep a file the account has no row for (a folder the engine has not seen yet, or its
/// children) and remove it with the rest. A junction or symlink is removed, never followed.
fn keep_untracked(
    path: &Path,
    relative: &str,
    keep_dir: &Path,
    kept: &mut Vec<Kept>,
    directories: &mut Vec<std::fs::File>,
) -> anyhow::Result<()> {
    let file = std::fs::OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES.0 | FILE_READ_DATA.0 | DELETE.0)
        // A folder must permit enumeration while we hold it; a file still refuses writers.
        .share_mode(FILE_SHARE_READ.0)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0 | FILE_FLAG_BACKUP_SEMANTICS.0)
        .open(path)?;
    let metadata = file.metadata()?;
    let reparse = metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0;
    if metadata.is_dir() {
        directories.push(file);
        if !reparse {
            for entry in std::fs::read_dir(path)? {
                let child = entry?.path();
                let name = child.file_name().and_then(|name| name.to_str()).unwrap_or_default();
                keep_untracked(&child, &format!("{relative}\\{name}"), keep_dir, kept, directories)?;
            }
        }
    } else if let Some(entry) = keep_pending_file(file, relative, keep_dir)? {
        kept.push(entry);
    }
    Ok(())
}

/// Slice 11: copy one unsent file out of the sync folder before anything is deleted. `Ok(None)`
/// means it holds no bytes on this PC, so there is nothing to keep. Any failure is a plain
/// sentence about space; no byte is touched before the copy is verified.
fn keep_pending_file(mut file: std::fs::File, relative: &str, keep_dir: &Path) -> anyhow::Result<Option<Kept>> {
    let size = file.metadata()?.len();
    let (placeholder, _) = super::placeholders::handle_identity(&file);
    let ranges = if placeholder && !super::placeholders::held_fully_on_disk(&file) {
        Some(super::placeholders::on_disk_ranges(&file, size)?)
    } else {
        None
    };
    let holds_nothing = ranges
        .as_ref()
        .is_some_and(|ranges| ranges.iter().all(|(_, length)| *length == 0));
    if holds_nothing {
        // Nothing on this PC to keep; the handle goes with the rest (closing it deletes the file).
        mark_for_deletion(&file)?;
        return Ok(None);
    }
    let dest = keep_dir.join(relative);
    let hash = match crate::unknown_check::copy_out(&mut file, size, ranges.as_deref(), &dest) {
        Ok((hash, _)) => hash,
        // A retry of a sign-out that failed after copying: the same copy is already there.
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            match crate::unknown_check::hash_source(&mut file, size, ranges.as_deref()).and_then(|mine| {
                let mut copy = std::fs::File::open(crate::unknown_check::copy_dest(&dest, ranges.is_some()))?;
                Ok((
                    mine,
                    crate::unknown_check::hash_source(&mut copy, size, ranges.as_deref())?,
                ))
            }) {
                Ok((mine, existing)) => {
                    anyhow::ensure!(
                        mine == existing,
                        "A different copy of a file is already in {}. Move it away and sign out again.",
                        keep_dir.display()
                    );
                    mine
                }
                Err(_) => anyhow::bail!("{}", KEEP_COPY_FAILED),
            }
        }
        Err(_) => anyhow::bail!("{}", KEEP_COPY_FAILED),
    };
    Ok(Some(Kept {
        file,
        size,
        ranges,
        hash,
    }))
}

/// Slice 11: the queued plaintext copies hold work that never reached the server. Keep them in
/// the same folder before the queue clears, so the sign-out never drops a byte.
fn keep_staged_payloads(staged: &[(String, Option<String>, bool)], keep_dir: &Path) -> anyhow::Result<usize> {
    let mut kept = 0;
    for (index, (path, source, _)) in staged.iter().enumerate() {
        let path = std::path::PathBuf::from(path);
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => anyhow::bail!("{}", KEEP_COPY_FAILED),
        };
        // A queued copy is a plain file Beebeeb wrote. Anything else is not ours to read through.
        anyhow::ensure!(
            metadata.is_file() && !metadata.file_type().is_symlink(),
            "Beebeeb found something unexpected where a queued copy should be. Move it away and sign out again."
        );
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&path)
            .map_err(|_| anyhow::anyhow!("{}", KEEP_COPY_FAILED))?;
        let size = file
            .metadata()
            .map_err(|_| anyhow::anyhow!("{}", KEEP_COPY_FAILED))?
            .len();
        let name = source
            .as_deref()
            .and_then(|source| Path::new(source).file_name())
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| format!("file {index}"));
        let dest = keep_dir.join("unsent").join(format!("{index:03} {name}"));
        match crate::unknown_check::copy_out(&mut file, size, None, &dest) {
            Ok(_) => kept += 1,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                match crate::unknown_check::hash_source(&mut file, size, None).and_then(|mine| {
                    let mut copy = std::fs::File::open(&dest)?;
                    Ok((mine, crate::unknown_check::hash_source(&mut copy, size, None)?))
                }) {
                    Ok((mine, existing)) => anyhow::ensure!(mine == existing, "{}", KEEP_COPY_FAILED),
                    Err(_) => anyhow::bail!("{}", KEEP_COPY_FAILED),
                }
                kept += 1;
            }
            Err(_) => anyhow::bail!("{}", KEEP_COPY_FAILED),
        }
    }
    Ok(kept)
}

/// Task 2191 (D11): open a file of unknown content exclusively, with read access. With
/// `UnknownFiles::Keep`, copy what is on this PC (the ranges on disk, for a partly downloaded
/// placeholder: the engine is stopped, so nothing can be fetched) before any deletion; with
/// `Delete`, it goes with the rest. A placeholder with nothing on disk holds nothing to keep.
fn prepare_unknown(
    db: &StateDb,
    path: &Path,
    relative: &str,
    expected: &crate::unknown_check::ExpectedAt,
    unknown: &UnknownFiles,
    kept: &mut Vec<Kept>,
    handles: &mut Vec<std::fs::File>,
) -> anyhow::Result<bool> {
    let mut file = std::fs::OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES.0 | FILE_READ_DATA.0 | DELETE.0)
        .share_mode(0)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0 | FILE_FLAG_BACKUP_SEMANTICS.0)
        .open(path)?;
    // Self-review: the database is trusted only when the identity there agrees; otherwise the
    // path goes through the ordinary rules, which refuse unsent changes.
    let (placeholder, identity) = super::placeholders::handle_identity(&file);
    let owner = match identity.as_deref() {
        Some(id) => db
            .get_file(id)?
            .map(|row| (row.path, db.has_agreed_content(id).unwrap_or(true))),
        None => None,
    };
    let rel_db = relative.replace('\\', "/");
    if !crate::unknown_check::unknown_at_signout(
        expected,
        &rel_db,
        placeholder,
        identity.as_deref(),
        owner.as_ref().map(|(path, guarded)| (path.as_str(), *guarded)),
    ) {
        return Ok(false);
    }
    let size = file.metadata()?.len();
    let ranges = if placeholder && !super::placeholders::held_fully_on_disk(&file) {
        Some(super::placeholders::on_disk_ranges(&file, size)?)
    } else {
        None
    };
    let holds_nothing = ranges
        .as_ref()
        .is_some_and(|ranges| ranges.iter().all(|(_, length)| *length == 0));
    match unknown {
        UnknownFiles::Keep { dir } if !holds_nothing => {
            let dest = dir.join(relative);
            let hash = match crate::unknown_check::copy_out(&mut file, size, ranges.as_deref(), &dest) {
                Ok((hash, _)) => hash,
                // A retry of a sign-out that failed after copying: the same copy is already there.
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    match crate::unknown_check::hash_source(&mut file, size, ranges.as_deref()).and_then(|mine| {
                        let mut copy = std::fs::File::open(crate::unknown_check::copy_dest(&dest, ranges.is_some()))?;
                        Ok((
                            mine,
                            crate::unknown_check::hash_source(&mut copy, size, ranges.as_deref())?,
                        ))
                    }) {
                        Ok((mine, existing)) => {
                            anyhow::ensure!(
                                mine == existing,
                                "A different copy of a file is already in {}. Move it away and sign out again.",
                                dir.display()
                            );
                            mine
                        }
                        Err(_) => anyhow::bail!("{}", KEEP_COPY_FAILED),
                    }
                }
                Err(_) => anyhow::bail!("{}", KEEP_COPY_FAILED),
            };
            kept.push(Kept {
                file,
                size,
                ranges,
                hash,
            });
        }
        _ => handles.push(file),
    }
    Ok(true)
}

// Directory deletion through this same handle is non-recursive. A child that
// arrives after prepare makes this fail without erasing that child's bytes.
// LD-0113 Amendment 7: a read-only Backups file is deleted too (the plain disposition refuses one).
fn mark_for_deletion(file: &std::fs::File) -> anyhow::Result<()> {
    super::read_only::mark_for_deletion(HANDLE(file.as_raw_handle()))?;
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
            "The file at this path isn’t the one Beebeeb expected. Move it out of the Beebeeb folder and sign out again."
        );
        let identity = std::slice::from_raw_parts(info.FileIdentity.as_ptr(), info.FileIdentityLength as usize);
        anyhow::ensure!(
            identity == file_id.as_bytes(),
            "The file at this path isn’t the one Beebeeb expected. Move it out of the Beebeeb folder and sign out again."
        );
        anyhow::ensure!(
            info.InSyncState == CF_IN_SYNC_STATE_IN_SYNC && info.ModifiedDataSize == 0,
            "This file has changes that haven’t been sent. Move it out of the Beebeeb folder and sign out again."
        );
    }
    Ok(())
}

#[cfg(test)]
#[path = "signout_tests.rs"]
mod tests;
