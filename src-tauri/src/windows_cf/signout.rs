//! Fail-closed Windows account cleanup. Prepare every owned placeholder before
//! deleting any: an untracked, dirty, busy or inaccessible item refuses sign-out.
//! Exclusive Win32 handles bind the identity/dirty check and deletion to the same
//! file, so a save or path replacement cannot turn cleanup into data loss.
#[path = "signout_cleanup.rs"]
mod cleanup;

use crate::state_db::StateDb;
use std::collections::HashMap;
use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
use std::path::Path;
use windows::Win32::Foundation::{BOOLEAN, HANDLE};
use windows::Win32::Storage::{CloudFilters::*, FileSystem::*};

pub fn purge(db: &StateDb, root: Option<&Path>) -> anyhow::Result<()> {
    db.windows_signout_preflight().map_err(|_| {
        anyhow::anyhow!("Pending changes remain. Unlock, finish syncing and resolve failed changes before signing out.")
    })?;
    let rows = db.list_files()?;
    let cache_paths = rows
        .iter()
        .map(|row| {
            db.get_file_contract_state(&row.file_id)
                .map(|state| state.and_then(|state| state.cache_path))
        })
        .collect::<Result<Vec<_>, _>>()?;
    cleanup::purge(
        cache_paths,
        crate::is_disposable_cache_path,
        || {
            let mut handles = Vec::new();
            let mut directories = Vec::new();
            if let Some(root) = root {
                let known: HashMap<_, _> = rows
                    .iter()
                    .map(|r| (r.path.trim_start_matches('/').replace('/', "\\"), r))
                    .collect();
                anyhow::ensure!(
                    root.is_dir(),
                    "Sync folder is unavailable; restore it before signing out"
                );
                prepare(root, root, &known, &mut handles, &mut directories)?;
            } else if !rows.is_empty() {
                anyhow::bail!("Cannot locate the sync folder. Restore its location before signing out.");
            }

            // Delete only files proven clean while their exclusive handle is held.
            // No dehydrate/remove fallback: dirty bytes never qualify for this list.
            for file in &handles {
                let info = FILE_DISPOSITION_INFO { DeleteFile: BOOLEAN(1) };
                unsafe {
                    SetFileInformationByHandle(
                        HANDLE(file.as_raw_handle()),
                        FileDispositionInfo,
                        &info as *const _ as *const std::ffi::c_void,
                        std::mem::size_of_val(&info) as u32,
                    )?;
                }
            }
            drop(handles);
            // Never recursively remove directories: newly created children make this
            // fail, leaving the account and DB available for recovery/retry.
            for directory in directories.iter().rev() {
                std::fs::remove_dir(directory)?;
            }
            if let Some(root) = root {
                if root.exists() && std::fs::read_dir(root)?.next().is_some() {
                    anyhow::bail!("New files appeared in the sync folder. Sync or move them out before signing out.");
                }
            }
            Ok(())
        },
        || Ok(db.finish_windows_signout()?),
    )
}

fn prepare(
    root: &Path,
    dir: &Path,
    known: &HashMap<String, &crate::state_db::FileEntry>,
    handles: &mut Vec<std::fs::File>,
    directories: &mut Vec<std::path::PathBuf>,
) -> anyhow::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        let relative = path.strip_prefix(root)?.to_string_lossy().replace('/', "\\");
        let row = known.get(&relative).ok_or_else(|| anyhow::anyhow!(
            "Unsynced files remain in the sync folder. Sync them or move them outside Beebeeb, then unlock and retry sign-out."
        ))?;
        let file = std::fs::OpenOptions::new()
            .access_mode(FILE_READ_ATTRIBUTES.0 | DELETE.0)
            .share_mode(0)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0 | FILE_FLAG_BACKUP_SEMANTICS.0)
            .open(&path)?;
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
                info.FileIdentityLength as usize == row.file_id.len(),
                "Placeholder identity changed; sign-out refused"
            );
            let identity = std::slice::from_raw_parts(info.FileIdentity.as_ptr(), info.FileIdentityLength as usize);
            anyhow::ensure!(
                identity == row.file_id.as_bytes(),
                "Placeholder belongs to another file; sign-out refused"
            );
            anyhow::ensure!(
                info.InSyncState == CF_IN_SYNC_STATE_IN_SYNC && info.ModifiedDataSize == 0,
                "Unsynced changes remain. Sync them or move a copy outside Beebeeb, then unlock and retry sign-out."
            );
        }
        if file.metadata()?.is_dir() {
            // CfGetPlaceholderInfo above rejects junctions/symlinks/normal dirs.
            directories.push(path.clone());
            drop(file);
            prepare(root, &path, known, handles, directories)?;
        } else {
            handles.push(file);
        }
    }
    Ok(())
}
