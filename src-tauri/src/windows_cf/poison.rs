//! LD-0105 S6 / I5: emptying a poisoned placeholder.
//!
//! A placeholder that a failed fetch has written into holds bytes of an attempt that did not finish
//! (or of another version). It is recorded as poisoned (`state_db` `cf_poison`), marked not in sync,
//! and every later fetch of it fails at once. This module removes the bytes it holds, so the next
//! open fetches the file afresh, in the one order the P1b spike measured to work
//! (`.claude/tasks/verification-evidence/LD-0105/p1b-spike`, spec §5.3):
//!
//! 1. `CfOpenFileWithOplock(EXCLUSIVE | WRITE_ACCESS)`: ONE handle for every step. A sharing
//!    violation means someone still has the file open: nothing is done, a later trigger retries.
//! 2. The file's change stamp must equal the one recorded with the poison: a write the person made
//!    since is never thrown away (the placeholder is then left as it is, not in sync).
//! 3. Record the pin state, then set it to unspecified (a pinned placeholder refuses to dehydrate,
//!    `0x80070188`).
//! 4. Set IN_SYNC (a placeholder that is not in sync refuses to dehydrate, `0x80070179`). Only the
//!    connected provider may (`0x8007018B` from any other process), so this runs in our own process.
//! 5. `CfDehydratePlaceholder`, then check that nothing is on disk.
//! 6. On any failure in 3–5: NOT_IN_SYNC and the pin state are restored BEFORE the handle is
//!    released, so a partial file is never left in sync (I6).
//! 7. On success: the pin state is restored, the poison cleared, the handle closed.
//!
//! It never goes through [`super::placeholders::dehydrate_placeholder`], whose not-in-sync answer is
//! `Ok(0)`. It runs outside any callback: on a close completion (spawned off the callback thread),
//! on every engine tick (the sweep), and so at engine start for poisons that outlived a restart.

use std::path::Path;

use windows::Win32::Foundation::HANDLE;
use windows::Win32::Storage::CloudFilters::*;
use windows::core::{HRESULT, PCWSTR};

// TEMPORARY (desktop PR 224 diagnostic for the windows-latest red; removed by the fix commit).
#[cfg(test)]
pub(crate) static DIAG: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
#[cfg(test)]
pub(crate) fn diag_push(line: String) {
    static T0: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    let t = T0.get_or_init(std::time::Instant::now).elapsed();
    DIAG.lock()
        .unwrap_or_else(|p| p.into_inner())
        .push(format!("{:10.3}ms {:?} {line}", t.as_secs_f64() * 1e3, std::thread::current().id()));
}
macro_rules! diag {
    ($($a:tt)*) => {{
        #[cfg(test)]
        $crate::windows_cf::poison::diag_push(format!($($a)*));
    }};
}
pub(crate) use diag;
fn rc(r: &windows::core::Result<()>) -> String {
    match r {
        Ok(()) => "ok".into(),
        Err(e) => format!("{:#010x}", e.code().0 as u32),
    }
}
/// Whether the protected handle is still valid (an oplock break invalidates it).
unsafe fn alive(handle: HANDLE) -> bool {
    let ok = unsafe { CfReferenceProtectedHandle(handle) }.as_bool();
    if ok {
        unsafe { CfReleaseProtectedHandle(handle) };
    }
    ok
}
fn info_diag(handle: HANDLE) -> String {
    let mut buffer = vec![0u64; 512];
    let r = unsafe {
        CfGetPlaceholderInfo(handle, CF_PLACEHOLDER_INFO_STANDARD, buffer.as_mut_ptr().cast(), (buffer.len() * 8) as u32, None)
    };
    match r {
        Ok(()) => {
            let i = unsafe { &*(buffer.as_ptr() as *const CF_PLACEHOLDER_STANDARD_INFO) };
            format!("info ok ondisk={} validated={} modified={} pin={} insync={}", i.OnDiskDataSize, i.ValidatedDataSize, i.ModifiedDataSize, i.PinState.0, i.InSyncState.0)
        }
        Err(e) => format!("info err {:#010x}", e.code().0 as u32),
    }
}

/// What the empty step did with one placeholder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Emptied {
    /// Nothing is on disk any more; the poison is cleared.
    Emptied,
    /// Another handle is open (a sharing violation): try again at the next trigger.
    Busy,
    /// The file changed after it was poisoned (or its stamp was never recorded): it is left as it is,
    /// not in sync, and the poison stays.
    Changed,
    /// The path is no placeholder any more (a plain file): nothing to empty; the poison is cleared.
    Gone,
    /// Nothing at the row's path: the file may have been renamed or moved and the database not told
    /// yet. The poison stays (it follows the file id), and a later sweep tries again.
    Missing,
    /// A step failed: NOT_IN_SYNC and the pin state were restored first. The status word only.
    Failed(&'static str),
}

const SHARING_VIOLATION: HRESULT = HRESULT(0x8007_0020_u32 as i32);
const NOT_A_CLOUD_FILE: HRESULT = HRESULT(0x8007_0178_u32 as i32);
const FILE_NOT_FOUND: HRESULT = HRESULT(0x8007_0002_u32 as i32);
const PATH_NOT_FOUND: HRESULT = HRESULT(0x8007_0003_u32 as i32);

/// The change stamp `(ChangeTime in 100 ns ticks since 1970, file index)` of `path`, read through an
/// attribute-only handle opened as the reparse point (nothing is fetched). A poison records it right
/// after the file is marked not in sync; the empty step reads the same pair through its own handle
/// ([`stamp_of_handle`]), so one function computes both sides of the comparison.
pub(crate) fn poison_stamp(path: &Path) -> Option<(i64, i64)> {
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Storage::FileSystem::{
        FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };
    let file = std::fs::OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES.0)
        .share_mode(FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0 | FILE_SHARE_DELETE.0)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
        .open(path)
        .ok()?;
    stamp_of_handle(HANDLE(file.as_raw_handle()))
}

/// The change stamp of the file behind `win32` (an open Win32 handle).
fn stamp_of_handle(win32: HANDLE) -> Option<(i64, i64)> {
    use windows::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, FILE_BASIC_INFO, FileBasicInfo, GetFileInformationByHandle,
        GetFileInformationByHandleEx,
    };
    let mut basic = FILE_BASIC_INFO::default();
    let mut by_handle = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: `win32` is open for both calls, and each writes only into the struct whose size it is given.
    unsafe {
        GetFileInformationByHandleEx(
            win32,
            FileBasicInfo,
            (&mut basic as *mut FILE_BASIC_INFO).cast(),
            std::mem::size_of::<FILE_BASIC_INFO>() as u32,
        )
        .ok()?;
        GetFileInformationByHandle(win32, &mut by_handle).ok()?;
    }
    const UNIX_EPOCH_TICKS: i64 = 116_444_736_000_000_000;
    let index = (u64::from(by_handle.nFileIndexHigh) << 32) | u64::from(by_handle.nFileIndexLow);
    Some((basic.ChangeTime.saturating_sub(UNIX_EPOCH_TICKS), index as i64))
}

/// `(pin state, bytes on disk)` of the placeholder behind `handle` (standard info, no fetch).
fn standard_info(handle: HANDLE) -> Option<(CF_PIN_STATE, i64)> {
    let mut buffer = vec![0u64; 512];
    // SAFETY: the buffer is writable for its stated length; `handle` is an open placeholder handle.
    unsafe {
        CfGetPlaceholderInfo(
            handle,
            CF_PLACEHOLDER_INFO_STANDARD,
            buffer.as_mut_ptr().cast(),
            (buffer.len() * 8) as u32,
            None,
        )
    }
    .ok()?;
    // SAFETY: on success the buffer starts with a CF_PLACEHOLDER_STANDARD_INFO, 8-byte aligned.
    let info = unsafe { &*(buffer.as_ptr() as *const CF_PLACEHOLDER_STANDARD_INFO) };
    Some((info.PinState, info.OnDiskDataSize))
}

/// Run the empty step on the poisoned placeholder at `path`, whose poison recorded `recorded`.
pub(crate) fn empty_poisoned(path: &Path, recorded: Option<(i64, i64)>) -> Emptied {
    let wide: Vec<u16> = path
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    // 1. One exclusive handle for every step.
    // SAFETY: `wide` is NUL-terminated and outlives the call.
    let handle = match unsafe {
        CfOpenFileWithOplock(
            PCWSTR(wide.as_ptr()),
            CF_OPEN_FILE_FLAG_EXCLUSIVE | CF_OPEN_FILE_FLAG_WRITE_ACCESS,
        )
    } {
        Ok(handle) => {
            diag!("{} open ok", path.display());
            handle
        }
        Err(e) if e.code() == SHARING_VIOLATION => return Emptied::Busy,
        Err(e) if e.code() == NOT_A_CLOUD_FILE => return Emptied::Gone,
        Err(e) if [FILE_NOT_FOUND, PATH_NOT_FOUND].contains(&e.code()) => return Emptied::Missing,
        Err(e) => {
            diag!("{} open err {:#010x}", path.display(), e.code().0 as u32);
            return Emptied::Failed("open");
        }
    };
    // SAFETY: `handle` is the protected handle opened above; it is closed exactly once, below.
    let outcome = unsafe { empty_under(handle, recorded) };
    diag!("{} outcome {outcome:?}, alive before close {}", path.display(), unsafe { alive(handle) });
    // SAFETY: as above.
    unsafe { CfCloseHandle(handle) };
    if matches!(outcome, Emptied::Failed(_)) {
        // Step 6 restored NOT_IN_SYNC through the handle; when that call itself failed, nothing else
        // would ever mark a partial file not in sync again, so it is marked once more by path (I6).
        let r = super::placeholders::mark_not_in_sync_path(path);
        diag!("{} by-path NOT_IN_SYNC {:?}", path.display(), r.map_err(|e| e.to_string()));
    }
    diag!("{} by-path view after close {:?}", path.display(), super::placeholders::platform_view(path));
    outcome
}

/// Steps 2–7 under `handle`.
///
/// SAFETY: `handle` must be an open protected handle from `CfOpenFileWithOplock`.
unsafe fn empty_under(handle: HANDLE, recorded: Option<(i64, i64)>) -> Emptied {
    // 2. Nothing written since the poison.
    // SAFETY: `handle` is open (the caller's contract).
    let win32 = unsafe { CfGetWin32HandleFromProtectedHandle(handle) };
    diag!("stamp recorded {recorded:?} now {:?} alive {}", stamp_of_handle(win32), unsafe { alive(handle) });
    match (recorded, stamp_of_handle(win32)) {
        (Some(recorded), Some(now)) if recorded == now => {}
        _ => return Emptied::Changed,
    }
    let Some((pin, _)) = standard_info(handle) else {
        return Emptied::Failed("info");
    };
    // On any failure from here: NOT_IN_SYNC, then the pin, before the handle is released (step 6).
    diag!("pre {} alive {}", info_diag(handle), unsafe { alive(handle) });
    let restore = |reason: &'static str| {
        diag!("restore for {reason}: alive {}", unsafe { alive(handle) });
        // SAFETY: `handle` is still open; both calls only set state on it.
        unsafe {
            let a = CfSetInSyncState(handle, CF_IN_SYNC_STATE_NOT_IN_SYNC, CF_SET_IN_SYNC_FLAG_NONE, None);
            let b = CfSetPinState(handle, pin, CF_SET_PIN_FLAG_NONE, None);
            diag!("restore NOT_IN_SYNC {} pin {}", rc(&a), rc(&b));
        }
        Emptied::Failed(reason)
    };
    // 3. The pin, recorded above, set to unspecified.
    if pin != CF_PIN_STATE_UNSPECIFIED {
        // SAFETY: `handle` is open.
        let r = unsafe { CfSetPinState(handle, CF_PIN_STATE_UNSPECIFIED, CF_SET_PIN_FLAG_NONE, None) };
        diag!("unpin {} alive {}", rc(&r), unsafe { alive(handle) });
        if r.is_err() {
            return restore("unpin");
        }
    }
    // 4. IN_SYNC: only the connected provider may.
    // SAFETY: `handle` is open.
    let r = unsafe { CfSetInSyncState(handle, CF_IN_SYNC_STATE_IN_SYNC, CF_SET_IN_SYNC_FLAG_NONE, None) };
    diag!("in_sync {} alive {}", rc(&r), unsafe { alive(handle) });
    if r.is_err() {
        return restore("in_sync");
    }
    // 5. Dehydrate the whole file, then check that nothing is left on disk.
    // SAFETY: `handle` is open; offset 0, length -1 is the whole file.
    let r = unsafe { CfDehydratePlaceholder(handle, 0, -1, CF_DEHYDRATE_FLAG_NONE, None) };
    diag!("dehydrate {} alive {}", rc(&r), unsafe { alive(handle) });
    if r.is_err() {
        return restore("dehydrate");
    }
    diag!("verify {} alive {}", info_diag(handle), unsafe { alive(handle) });
    match standard_info(handle) {
        Some((_, 0)) => {}
        _ => return restore("still_on_disk"),
    }
    // 7. The pin as it was.
    if pin != CF_PIN_STATE_UNSPECIFIED {
        // SAFETY: `handle` is open.
        let r = unsafe { CfSetPinState(handle, pin, CF_SET_PIN_FLAG_NONE, None) };
        diag!("re-pin {}", rc(&r));
    }
    Emptied::Emptied
}

/// The empty step for every poisoned placeholder of `db` under `sync_root`; clears the poison of each
/// one emptied (or gone). Returns each file's outcome. Runs on every engine tick, and so at engine
/// start, and after each close completion of a poisoned file.
pub(crate) fn sweep_poisoned(db: &crate::state_db::StateDb, sync_root: &Path) -> Vec<(String, Emptied)> {
    let rows = match db.poisoned() {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!(error = %e, "poison sweep: could not read the poisoned placeholders");
            return Vec::new();
        }
    };
    let mut outcomes = Vec::new();
    for row in rows {
        let Ok(Some(entry)) = db.get_file(&row.file_id) else {
            // The row is gone (deleted or renamed away): nothing of it is left to empty here.
            let _ = db.clear_poison(&row.file_id);
            continue;
        };
        let Ok(path) = crate::engine_bridge::local_file_path_under_sync_root(sync_root, &entry.path) else {
            continue;
        };
        let stamp = row.change_ticks.zip(row.file_index);
        let outcome = empty_poisoned(&path, stamp);
        match &outcome {
            Emptied::Emptied | Emptied::Gone => {
                // Nothing of the failed attempt is on disk: the row says so too (as "Free up space" does).
                if outcome == Emptied::Emptied
                    && let Err(e) = db.mark_cloud_only_after_dehydrate(
                        std::slice::from_ref(&row.file_id),
                        chrono::Utc::now().timestamp(),
                    )
                {
                    tracing::warn!(error = %e, file_id = %row.file_id, "poison sweep: emptied, but the row could not be marked cloud-only");
                }
                if let Err(e) = db.clear_poison(&row.file_id) {
                    tracing::warn!(error = %e, file_id = %row.file_id, "poison sweep: emptied, but the poison could not be cleared");
                }
            }
            Emptied::Busy => tracing::debug!(file_id = %row.file_id, "poison sweep: still open; next time"),
            Emptied::Missing => {
                tracing::debug!(file_id = %row.file_id, "poison sweep: nothing at its path yet; next time")
            }
            Emptied::Changed => {
                tracing::warn!(file_id = %row.file_id, "poison sweep: changed since it was poisoned; left as it is")
            }
            Emptied::Failed(step) => {
                tracing::warn!(file_id = %row.file_id, step, "poison sweep: a step failed; restored")
            }
        }
        outcomes.push((row.file_id, outcome));
    }
    outcomes
}

/// The engine's periodic trigger: the connected bridge's database, this sync root.
pub fn sweep_poisoned_now(sync_root: &Path) {
    let Some(bridge) = super::bridge() else { return };
    if bridge.is_revoked() || bridge.is_stopping() {
        return;
    }
    let emptied = sweep_poisoned(bridge.db(), sync_root)
        .iter()
        .filter(|(_, o)| *o == Emptied::Emptied)
        .count();
    if emptied > 0 {
        tracing::info!(emptied, "poisoned placeholders emptied");
    }
}

/// The close trigger: a handle on `file_id` closed. If it is poisoned, the sweep runs off the callback
/// thread (a Cloud Files callback must never wait on the empty step's oplock).
pub(super) fn after_close(file_id: &str) {
    let Some(bridge) = super::bridge() else { return };
    if !matches!(bridge.db().poison(file_id), Ok(Some(_))) {
        return;
    }
    let (Some(runtime), Some(root)) = (super::runtime(), super::sync_root()) else {
        return;
    };
    runtime.spawn_blocking(move || sweep_poisoned_now(&root));
}
