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

/// Durable metadata for a packed snapshot of modified CFAPI ranges. Bytes are
/// stored separately in a flushed payload; missing ranges always use this base.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct PartialWrite {
    pub base_version: i64,
    pub eof: u64,
    pub ranges: Vec<(u64, u64)>,
}

impl PartialWrite {
    pub fn assemble(&self, mut base: zeroize::Zeroizing<Vec<u8>>, patch: &[u8]) -> anyhow::Result<zeroize::Zeroizing<Vec<u8>>> {
        base.resize(usize::try_from(self.eof)?, 0);
        let mut consumed = 0usize;
        let mut previous_end = 0u64;
        for &(offset, length) in &self.ranges {
            let end = offset.checked_add(length).ok_or_else(|| anyhow::anyhow!("partial range overflow"))?;
            anyhow::ensure!(offset >= previous_end && end <= self.eof, "invalid partial snapshot range");
            let next = consumed.checked_add(usize::try_from(length)?).ok_or_else(|| anyhow::anyhow!("partial length overflow"))?;
            anyhow::ensure!(next <= patch.len(), "truncated partial snapshot");
            base[usize::try_from(offset)?..usize::try_from(end)?].copy_from_slice(&patch[consumed..next]);
            consumed = next;
            previous_end = end;
        }
        anyhow::ensure!(consumed == patch.len(), "unexpected partial snapshot bytes");
        Ok(base)
    }
}


/// Durable live-install journal. Its presence excludes the destination from
/// watcher capture, even when a crash has left it empty or only partly written.
#[derive(serde::Serialize, serde::Deserialize)]
struct LiveInstall {
    payload: std::path::PathBuf,
    hash: String,
    version: Option<i64>,
    retain_payload: bool,
}

fn install_marker(root: &Path, path: &Path) -> anyhow::Result<std::path::PathBuf> {
    let relative = path.strip_prefix(root)?;
    Ok(root.join(".beebeeb/windows-writes").join(format!("{}.install.json",
        hash_bytes(relative.to_string_lossy().as_bytes()))))
}

pub(crate) fn install_pending(root: &Path, path: &Path) -> anyhow::Result<bool> {
    Ok(install_marker(root, path)?.try_exists()?)
}

pub(crate) fn open_exclusive(path: &Path) -> anyhow::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true).write(true);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT;
        options.share_mode(0).custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(path)?;
    anyhow::ensure!(!file.metadata()?.file_type().is_symlink(), "refusing symlink destination");
    Ok(file)
}

fn sync_parent(path: &Path) -> anyhow::Result<()> {
    #[cfg(unix)]
    std::fs::File::open(path.parent().ok_or_else(|| anyhow::anyhow!("missing parent"))?)?.sync_all()?;
    let _ = path;
    Ok(())
}

pub(crate) fn durable_bytes(directory: &Path, suffix: &str, bytes: &[u8]) -> anyhow::Result<std::path::PathBuf> {
    use std::io::Write;
    std::fs::create_dir_all(directory)?;
    let path = directory.join(format!("{}.{}", uuid::Uuid::new_v4(), suffix));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    { use std::os::unix::fs::OpenOptionsExt; options.mode(0o600); }
    let mut file = options.open(&path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    sync_parent(&path)?;
    Ok(path)
}

fn write_install(file: &mut std::fs::File, intent: &LiveInstall) -> anyhow::Result<()> {
    use std::io::{Seek, SeekFrom, Write};
    let bytes = zeroize::Zeroizing::new(std::fs::read(&intent.payload)?);
    anyhow::ensure!(hash_bytes(&bytes) == intent.hash, "live-install payload changed");
    // Both payload and journal were flushed BEFORE this first destructive step.
    file.set_len(0)?;
    file.seek(SeekFrom::Start(0))?;
    #[cfg(all(test, target_os = "windows"))]
    if let Some(count) = crate::windows_cf::placeholders::partial_edits::TEST_FAIL_AFTER.with(|v| v.get()) {
        file.write_all(&bytes[..count.min(bytes.len())])?;
        file.sync_all()?;
        anyhow::bail!("injected materialization write failure after {count} bytes");
    }
    file.write_all(&bytes)?;
    file.sync_all()?;
    Ok(())
}

pub(crate) fn install_staged(
    root: &Path, path: &Path, file: &mut std::fs::File, payload: &Path,
    version: Option<i64>, retain_payload: bool, finish: impl FnOnce(Option<i64>, &str) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    // Never replace an unfinished intent, even if the new download succeeded.
    let marker = install_marker(root, path)?;
    let intent = LiveInstall { payload: payload.to_owned(), hash: hash_file(payload)?, version, retain_payload };
    let journal = durable_bytes(marker.parent().unwrap(), "intent", &serde_json::to_vec(&intent)?)?;
    publish_conflict_copy(&journal, &marker)?;
    std::fs::remove_file(journal)?;
    sync_parent(&marker)?;
    write_install(file, &intent)?;
    finish(intent.version, &intent.hash)?;
    std::fs::remove_file(&marker)?;
    sync_parent(&marker)?;
    if !intent.retain_payload { std::fs::remove_file(&intent.payload)?; }
    Ok(())
}

pub(crate) fn recover_install(
    root: &Path, path: &Path,
    finish: impl FnOnce(Option<i64>, &str) -> anyhow::Result<()>,
) -> anyhow::Result<bool> {
    use std::io::{Seek, SeekFrom};
    let marker = install_marker(root, path)?;
    if !marker.try_exists()? { return Ok(false); }
    let intent: LiveInstall = serde_json::from_slice(&std::fs::read(&marker)?)?;
    let mut file = open_exclusive(path)?;
    let mut current = zeroize::Zeroizing::new(Vec::new());
    file.read_to_end(&mut current)?;
    if hash_bytes(&current) != intent.hash {
        // A writer may have saved after the failed provider write. We cannot
        // distinguish it from a provider prefix after a crash. Preserve ALL
        // ambiguous bytes in reserved storage (never a watcher upload source).
        durable_bytes(marker.parent().unwrap(), "recovery", &current)?;
        file.seek(SeekFrom::Start(0))?;
        write_install(&mut file, &intent)?;
    }
    file.sync_all()?;
    finish(intent.version, &intent.hash)?;
    std::fs::remove_file(&marker)?;
    sync_parent(&marker)?;
    if !intent.retain_payload { std::fs::remove_file(&intent.payload)?; }
    Ok(true)
}


#[cfg(all(test, target_os = "windows"))]
#[test]
fn regression_1640_r4_destination_excludes_writers_and_atomic_saves() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("destination");
    let temporary = dir.path().join("save.tmp");
    std::fs::write(&path, b"captured").unwrap();
    std::fs::write(&temporary, b"atomic save").unwrap();
    let held = open_exclusive(&path).unwrap();
    assert!(std::fs::write(&path, b"racing save").is_err(), "exclusive destination must deny writers");
    assert!(std::fs::rename(&temporary, &path).is_err(), "exclusive destination must deny replacement");
    drop(held);
    std::fs::rename(&temporary, &path).unwrap();
    assert_eq!(std::fs::read(path).unwrap(), b"atomic save");
}
