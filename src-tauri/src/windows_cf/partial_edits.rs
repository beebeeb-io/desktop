//! Snapshot dirty partial files without recalling their missing ranges. The
//! metadata/range query and reads share one handle that denies writes/deletion.
use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom},
    os::windows::{fs::OpenOptionsExt, io::AsRawHandle},
    path::Path,
};
use windows::Win32::{
    Foundation::HANDLE,
    Storage::{CloudFilters::*, FileSystem::*},
};

pub(crate) struct PartialSnapshot {
    pub eof: u64,
    pub ranges: Vec<(u64, u64)>,
    pub bytes: zeroize::Zeroizing<Vec<u8>>,
}

#[cfg(test)]
thread_local! {
    pub(crate) static TEST_RANGES: std::cell::RefCell<Option<(std::path::PathBuf, bool, Vec<(u64,u64)>)>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
thread_local! {
    pub(crate) static TEST_FAIL_AFTER: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

fn modified_ranges(file: &File, path: &Path, eof: u64) -> anyhow::Result<Option<Vec<(u64, u64)>>> {
    #[cfg(test)]
    if let Some((_, dirty, ranges)) = TEST_RANGES.with(|v| v.borrow().clone()).filter(|v| v.0 == path) {
        return Ok(dirty.then_some(ranges));
    }
    let _ = path;
    let handle = HANDLE(file.as_raw_handle());
    let mut info = vec![0u64; 1024];
    unsafe {
        CfGetPlaceholderInfo(
            handle,
            CF_PLACEHOLDER_INFO_STANDARD,
            info.as_mut_ptr().cast(),
            (info.len() * 8) as u32,
            None,
        )?;
        let info = &*info.as_ptr().cast::<CF_PLACEHOLDER_STANDARD_INFO>();
        if info.ModifiedDataSize == 0 && info.InSyncState == CF_IN_SYNC_STATE_IN_SYNC {
            return Ok(None);
        }
    }
    let mut result = Vec::new();
    let mut offset = 0i64;
    while offset < i64::try_from(eof)? {
        let mut buffer = [CF_FILE_RANGE::default(); 256];
        let mut returned = 0u32;
        let status = unsafe {
            CfGetPlaceholderRangeInfo(
                handle,
                CF_PLACEHOLDER_RANGE_INFO_MODIFIED,
                offset,
                i64::try_from(eof)? - offset,
                buffer.as_mut_ptr().cast(),
                std::mem::size_of_val(&buffer) as u32,
                Some(&mut returned),
            )
        };
        // ERROR_MORE_DATA returns the ranges that fit, then we continue after
        // the last range. Every other failure preserves the file for retry.
        if let Err(e) = status {
            if e.code() != windows::core::HRESULT::from_win32(234) {
                return Err(e.into());
            }
        }
        anyhow::ensure!(
            returned as usize <= std::mem::size_of_val(&buffer)
                && returned as usize % std::mem::size_of::<CF_FILE_RANGE>() == 0,
            "invalid CF range response"
        );
        let count = returned as usize / std::mem::size_of::<CF_FILE_RANGE>();
        if count == 0 {
            break;
        }
        for range in &buffer[..count] {
            anyhow::ensure!(
                range.StartingOffset >= offset && range.Length > 0,
                "invalid modified range"
            );
            let end = range
                .StartingOffset
                .checked_add(range.Length)
                .ok_or_else(|| anyhow::anyhow!("range overflow"))?;
            // CFAPI reports allocation-aligned ranges; clip the last to EOF.
            result.push((
                range.StartingOffset as u64,
                (end as u64).min(eof) - range.StartingOffset as u64,
            ));
            offset = end;
        }
    }
    Ok(Some(result))
}

pub(crate) fn capture(path: &Path) -> anyhow::Result<Option<PartialSnapshot>> {
    let mut file = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ.0)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
        .open(path)?;
    snapshot_from_file(&mut file, path)
}

fn snapshot_from_file(file: &mut File, path: &Path) -> anyhow::Result<Option<PartialSnapshot>> {
    let eof = file.metadata()?.len();
    let Some(ranges) = modified_ranges(&file, path, eof)? else {
        return Ok(None);
    };
    let mut bytes = zeroize::Zeroizing::new(Vec::new());
    for &(offset, length) in &ranges {
        file.seek(SeekFrom::Start(offset))?;
        let start = bytes.len();
        bytes.resize(
            start
                .checked_add(usize::try_from(length)?)
                .ok_or_else(|| anyhow::anyhow!("snapshot overflow"))?,
            0,
        );
        file.read_exact(&mut bytes[start..])?;
    }
    Ok(Some(PartialSnapshot { eof, ranges, bytes }))
}

/// Deny concurrent writes/deletion while comparing and materializing. This does
/// not call CF hydration (which could fetch the current, rather than base, version).
pub(crate) fn materialize_if_unchanged(
    path: &Path,
    spec: &crate::windows_edits::PartialWrite,
    patch: &[u8],
    bytes: &[u8],
) -> anyhow::Result<bool> {
    use std::io::Write;
    if super::resident_for_edit(path)? {
        return Ok(false);
    }
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(0)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
        .open(path)?;
    let Some(current) = snapshot_from_file(&mut file, path)? else {
        return Ok(false);
    };
    if current.eof != spec.eof || current.ranges != spec.ranges || current.bytes.as_slice() != patch {
        return Ok(false);
    }
    // The complete snapshot is fsynced and queued before this write. Even an
    // interrupted local materialization cannot lose the settled user save.
    file.set_len(0)?;
    file.seek(SeekFrom::Start(0))?;
    #[cfg(test)]
    if let Some(count) = TEST_FAIL_AFTER.with(|v| v.get()) {
        file.write_all(&bytes[..count.min(bytes.len())])?;
        file.sync_all()?;
        anyhow::bail!("injected materialization write failure after {count} bytes");
    }
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(true)
}
