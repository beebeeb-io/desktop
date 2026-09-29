//! Content authority for the Windows watcher and reclaim paths. No schema change:
//! local_hash is the last *confirmed server* plaintext hash, never a scan result.
#![cfg_attr(not(target_os = "windows"), allow(dead_code))]
use crate::state_db::{FileStatus, StateDb};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::Path;

pub(crate) fn hash_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub(crate) fn hash_reader(reader: &mut impl Read) -> anyhow::Result<String> {
    let mut hash = Sha256::new();
    let mut buffer = zeroize::Zeroizing::new([0u8; 64 * 1024]);
    loop {
        let count = reader.read(&mut *buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

pub(crate) fn hash_file(path: &Path) -> anyhow::Result<String> {
    hash_reader(&mut std::fs::File::open(path)?)
}

/// Call under an exclusive native file handle for destructive operations. Unknown
/// hashes, errors, stale versions, queued payloads and dirty statuses fail closed.
pub(crate) fn content_is_confirmed(db: &StateDb, file_id: &str, digest: &str) -> anyhow::Result<bool> {
    if db.has_pending_content(file_id)? {
        return Ok(false);
    }
    let Some(entry) = db.get_file(file_id)? else {
        return Ok(false);
    };
    if entry.status != FileStatus::Local {
        return Ok(false);
    }
    let Some(contract) = db.get_file_contract_state(file_id)? else {
        return Ok(false);
    };
    Ok(contract.local_base_version == contract.current_version && contract.local_hash.as_deref() == Some(digest))
}

/// Publish a fully flushed conflict copy without exposing a partial file to
/// the scanner and without replacing a same-named user file. The source staging
/// directory is inside this sync root, so the portable hard-link publish stays
/// on the same filesystem; Windows uses MoveFileW's no-replace contract.
pub(crate) fn publish_conflict_copy(payload: &Path, destination: &Path) -> anyhow::Result<()> {
    let temp = payload.with_file_name(format!("{}.conflict.tmp", uuid::Uuid::new_v4()));
    let result = (|| -> anyhow::Result<()> {
        let mut input = std::fs::File::open(payload)?;
        let mut output = std::fs::OpenOptions::new().write(true).create_new(true).open(&temp)?;
        std::io::copy(&mut input, &mut output)?;
        output.sync_all()?;
        drop(output);
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::ffi::OsStrExt;
            use windows::Win32::Storage::FileSystem::MoveFileW;
            use windows::core::PCWSTR;
            let from: Vec<u16> = temp.as_os_str().encode_wide().chain(Some(0)).collect();
            let to: Vec<u16> = destination.as_os_str().encode_wide().chain(Some(0)).collect();
            unsafe {
                MoveFileW(PCWSTR(from.as_ptr()), PCWSTR(to.as_ptr()))?;
            }
        }
        #[cfg(not(target_os = "windows"))]
        std::fs::hard_link(&temp, destination)?;
        Ok(())
    })();
    let _ = std::fs::remove_file(&temp);
    result
}

/// CF close flags have no "user wrote" bit. Exclude deletions/provider reads;
/// the remaining application's close still needs a confirmed-hash comparison.
pub(crate) fn should_inspect_close(deleted: bool, process_id: Option<u32>, provider: u32) -> bool {
    !deleted && process_id != Some(provider)
}

#[cfg(test)]
mod tests {
    #[test]
    fn regression_1640_close_classifier() {
        use super::should_inspect_close;
        assert!(!should_inspect_close(false, Some(10), 10));
        assert!(!should_inspect_close(true, Some(11), 10));
        assert!(!should_inspect_close(true, None, 10));
        assert!(should_inspect_close(false, Some(11), 10));
        assert!(should_inspect_close(false, None, 10));
    }
}
