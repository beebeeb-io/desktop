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
//!    The handle is referenced (`CfReferenceProtectedHandle`) until step 7 ends, and every step
//!    runs on its Win32 handle. Another open that breaks the oplock then waits until the reference
//!    is released. Through the protected handle itself, every call after such a break fails with
//!    `0x80070006`, the restore of step 6 included: on the windows-latest runner that lost the pin
//!    and left NOT_IN_SYNC to the by-path mark below (desktop PR 224). A handle that cannot be
//!    referenced any more when the step starts is `Busy`: nothing was changed yet.
//! 2. The file's content key (`LastWriteTime` + size, contract Amendment 4) must equal the one recorded
//!    with the poison. Our own restore writes change metadata only, so an unchanged key means the bytes
//!    are the failed attempt's own. A changed key, or none, is "needs attention": the file is never
//!    emptied (it may hold a person's write) and never cleared silently; only "Discard and download
//!    again" ([`discard_now`]) ends it.
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
//! `Ok(0)`. It runs outside any callback: on a close completion of that one file (spawned off the
//! callback thread), on every engine tick (the sweep), and at the first sweep of the process for
//! poisons that outlived a restart. A pin that cannot be put back through the handle is set again by
//! path, and if that fails too it is kept on the row and retried ("Always keep on this device" is
//! never silently lost).

use std::path::Path;

use windows::Win32::Foundation::HANDLE;
use windows::Win32::Storage::CloudFilters::*;
use windows::core::{HRESULT, PCWSTR};

/// What the empty step did with one placeholder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Emptied {
    /// Nothing is on disk any more; the poison is cleared.
    Emptied,
    /// Another handle is open (a sharing violation): try again at the next trigger.
    Busy,
    /// Amendment 4: the content key changed since the poison (something wrote to the file), or none was
    /// recorded. Left as it is, not in sync, still fetch-refused; the row needs attention.
    NeedsAttention,
    /// The path is no placeholder any more (a plain file): nothing to empty; the poison is cleared.
    Gone,
    /// Nothing at the row's path (a rename the database does not know yet, or a delete). Cleared only
    /// when it is missing at the first sweep of a new process (Amendment 4).
    Missing,
    /// A step failed: NOT_IN_SYNC and the pin state were restored first. The status word only.
    Failed(&'static str),
}

/// What step 2 checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Check {
    /// A sweep: empty only while the content key is the one the poison recorded.
    Key(Option<(i64, i64)>),
    /// The person chose "Discard and download again": empty whatever the key.
    Discard,
}

/// The points between the empty step's calls, for a native test that acts there (another process
/// opening the file mid-step, or a state change through the step's own handle). Production passes a
/// closure that does nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Step {
    /// Referenced, the stamp checked, the pin recorded; nothing changed yet.
    Checked,
    /// IN_SYNC is set and the bytes are still on disk: the one moment I6 must survive.
    InSync,
    /// Dehydrated, not yet verified.
    Dehydrated,
}

const SHARING_VIOLATION: HRESULT = HRESULT(0x8007_0020_u32 as i32);
const NOT_A_CLOUD_FILE: HRESULT = HRESULT(0x8007_0178_u32 as i32);
const FILE_NOT_FOUND: HRESULT = HRESULT(0x8007_0002_u32 as i32);
const PATH_NOT_FOUND: HRESULT = HRESULT(0x8007_0003_u32 as i32);

/// The content key `(LastWriteTime in 100 ns ticks since 1970, size)` of `path` (Amendment 4), read
/// through an attribute-only handle opened as the reparse point (nothing is fetched). A poison records
/// it right after the file is marked not in sync; the empty step reads the same pair through its own
/// handle ([`key_of_handle`]), so one function computes both sides of the comparison.
pub(crate) fn content_key(path: &Path) -> Option<(i64, i64)> {
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
    key_of_handle(HANDLE(file.as_raw_handle()))
}

/// The content key of the file behind `win32` (an open Win32 handle).
pub(crate) fn key_of_handle(win32: HANDLE) -> Option<(i64, i64)> {
    use windows::Win32::Storage::FileSystem::{
        FILE_BASIC_INFO, FILE_STANDARD_INFO, FileBasicInfo, FileStandardInfo, GetFileInformationByHandleEx,
    };
    let mut basic = FILE_BASIC_INFO::default();
    let mut standard = FILE_STANDARD_INFO::default();
    // SAFETY: `win32` is open for both calls, and each writes only into the struct whose size it is given.
    unsafe {
        GetFileInformationByHandleEx(
            win32,
            FileBasicInfo,
            (&mut basic as *mut FILE_BASIC_INFO).cast(),
            std::mem::size_of::<FILE_BASIC_INFO>() as u32,
        )
        .ok()?;
        GetFileInformationByHandleEx(
            win32,
            FileStandardInfo,
            (&mut standard as *mut FILE_STANDARD_INFO).cast(),
            std::mem::size_of::<FILE_STANDARD_INFO>() as u32,
        )
        .ok()?;
    }
    const UNIX_EPOCH_TICKS: i64 = 116_444_736_000_000_000;
    Some((basic.LastWriteTime.saturating_sub(UNIX_EPOCH_TICKS), standard.EndOfFile))
}

/// The change time of `path` (100 ns ticks): printed by the native rungs next to the content key, to
/// show which of the two our own writes move. Not a key.
#[cfg(test)]
pub(crate) fn change_time(path: &Path) -> Option<i64> {
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Storage::FileSystem::{
        FILE_BASIC_INFO, FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ,
        FILE_SHARE_WRITE, FileBasicInfo, GetFileInformationByHandleEx,
    };
    let file = std::fs::OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES.0)
        .share_mode(FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0 | FILE_SHARE_DELETE.0)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
        .open(path)
        .ok()?;
    let mut basic = FILE_BASIC_INFO::default();
    // SAFETY: as in `key_of_handle`.
    unsafe {
        GetFileInformationByHandleEx(
            HANDLE(file.as_raw_handle()),
            FileBasicInfo,
            (&mut basic as *mut FILE_BASIC_INFO).cast(),
            std::mem::size_of::<FILE_BASIC_INFO>() as u32,
        )
        .ok()?;
    }
    Some(basic.ChangeTime)
}

/// Test seam (Amendment 4's pin rung): make every pin restore through the step's handle fail, so the
/// by-path retry is what puts the pin back.
#[cfg(test)]
pub(crate) static FAIL_PIN_RESTORE_BY_HANDLE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// `CfSetPinState(pin)` by path, with its own handle (the retry when the step's handle could not).
fn set_pin_by_path(path: &Path, pin: CF_PIN_STATE) -> bool {
    let wide: Vec<u16> = path.to_string_lossy().encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: `wide` is NUL-terminated and outlives the calls; the handle is closed once.
    unsafe {
        let Ok(handle) = CfOpenFileWithOplock(PCWSTR(wide.as_ptr()), CF_OPEN_FILE_FLAG_WRITE_ACCESS) else {
            return false;
        };
        let ok = CfSetPinState(handle, pin, CF_SET_PIN_FLAG_NONE, None).is_ok();
        CfCloseHandle(handle);
        ok
    }
}

/// `(pin state, bytes held)` of the placeholder behind `handle` (standard info, no fetch). Bytes held
/// are on-disk plus modified data, as `placeholders::platform_view` counts "nothing on disk".
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
    Some((info.PinState, info.OnDiskDataSize.saturating_add(info.ModifiedDataSize)))
}

/// Run the empty step on the poisoned placeholder at `path`.
pub(crate) fn empty_poisoned(path: &Path, check: Check) -> Emptied {
    attempt(path, check, &mut |_, _| {}).0
}

/// [`empty_poisoned`], calling `between` at each [`Step`] with the Win32 handle every step uses.
pub(crate) fn empty_poisoned_with(path: &Path, check: Check, between: &mut dyn FnMut(Step, HANDLE)) -> Emptied {
    attempt(path, check, between).0
}

/// The empty step; and the pin state that could be put back neither through the step's handle nor by
/// path (Amendment 4: kept on the row, retried at the next sweep).
fn attempt(path: &Path, check: Check, between: &mut dyn FnMut(Step, HANDLE)) -> (Emptied, Option<CF_PIN_STATE>) {
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
        Ok(handle) => handle,
        Err(e) if e.code() == SHARING_VIOLATION => return (Emptied::Busy, None),
        Err(e) if e.code() == NOT_A_CLOUD_FILE => return (Emptied::Gone, None),
        Err(e) if [FILE_NOT_FOUND, PATH_NOT_FOUND].contains(&e.code()) => return (Emptied::Missing, None),
        Err(_) => return (Emptied::Failed("open"), None),
    };
    let mut restore = Restore::default();
    // Referenced for every step, so an oplock break cannot invalidate the handle between two of
    // them (step 1, above). Not referenceable: the oplock broke since the open, someone has the file.
    // SAFETY: `handle` is the protected handle opened above; it is closed exactly once, below, and
    // released exactly once when this reference was taken.
    let outcome = if unsafe { CfReferenceProtectedHandle(handle) }.as_bool() {
        // SAFETY: referenced, so its Win32 handle stays open until the release below.
        let win32 = unsafe { CfGetWin32HandleFromProtectedHandle(handle) };
        // SAFETY: open for the whole call (the contract of `empty_under`).
        let outcome = unsafe { empty_under(win32, check, between, &mut restore) };
        // SAFETY: the reference taken above.
        unsafe { CfReleaseProtectedHandle(handle) };
        outcome
    } else {
        Emptied::Busy
    };
    // SAFETY: as above.
    unsafe { CfCloseHandle(handle) };
    if matches!(outcome, Emptied::Failed(_)) && !restore.not_in_sync {
        // Step 6 could not restore NOT_IN_SYNC through the handle: nothing else would ever mark a
        // partial file not in sync again, so it is marked by path (I6). Our own write: the content key
        // does not move (Amendment 4), so the next sweep can still empty it.
        let _ = super::placeholders::mark_not_in_sync_path(path);
    }
    // Amendment 4: a pin the handle could not put back is set by path; if that fails too, the caller
    // keeps it on the row.
    let lost = restore.pin_lost.filter(|pin| !set_pin_by_path(path, *pin));
    (outcome, lost)
}

/// What step 6 / 7 managed to put back through the step's handle.
#[derive(Default)]
struct Restore {
    not_in_sync: bool,
    pin_lost: Option<CF_PIN_STATE>,
}

/// `CfSetPinState(pin)` through the step's handle (or the test seam's failure).
///
/// SAFETY: `handle` is open.
unsafe fn set_pin(handle: HANDLE, pin: CF_PIN_STATE) -> bool {
    #[cfg(test)]
    if FAIL_PIN_RESTORE_BY_HANDLE.load(std::sync::atomic::Ordering::SeqCst) && pin != CF_PIN_STATE_UNSPECIFIED {
        return false;
    }
    // SAFETY: the caller's contract.
    unsafe { CfSetPinState(handle, pin, CF_SET_PIN_FLAG_NONE, None) }.is_ok()
}

/// Steps 2–7 under `handle`.
///
/// SAFETY: `handle` must be the Win32 handle of a protected handle from `CfOpenFileWithOplock` that
/// stays referenced (`CfReferenceProtectedHandle`) for the whole call. Every step goes through it,
/// never through the protected handle, whose calls fail once an oplock break is pending.
unsafe fn empty_under(handle: HANDLE, check: Check, between: &mut dyn FnMut(Step, HANDLE), restore: &mut Restore) -> Emptied {
    // 2. Nothing written since the poison (Amendment 4: the content key), unless the person discards.
    if let Check::Key(recorded) = check
        && crate::cf_range::key_verdict(recorded, key_of_handle(handle)) == crate::cf_range::KeyVerdict::NeedsAttention
    {
        return Emptied::NeedsAttention;
    }
    let Some((pin, _)) = standard_info(handle) else {
        return Emptied::Failed("info");
    };
    between(Step::Checked, handle);
    // On any failure from here: NOT_IN_SYNC, then the pin, before the handle is released (step 6).
    let mut fail = |reason: &'static str| {
        // SAFETY: `handle` is still open; both calls only set state on it.
        unsafe {
            restore.not_in_sync = CfSetInSyncState(handle, CF_IN_SYNC_STATE_NOT_IN_SYNC, CF_SET_IN_SYNC_FLAG_NONE, None).is_ok();
            if !set_pin(handle, pin) {
                restore.pin_lost = Some(pin);
            }
        }
        Emptied::Failed(reason)
    };
    // 3. The pin, recorded above, set to unspecified.
    // SAFETY: `handle` is open.
    if pin != CF_PIN_STATE_UNSPECIFIED && !unsafe { set_pin(handle, CF_PIN_STATE_UNSPECIFIED) } {
        return fail("unpin");
    }
    // 4. IN_SYNC: only the connected provider may.
    // SAFETY: `handle` is open.
    if unsafe { CfSetInSyncState(handle, CF_IN_SYNC_STATE_IN_SYNC, CF_SET_IN_SYNC_FLAG_NONE, None) }.is_err() {
        return fail("in_sync");
    }
    between(Step::InSync, handle);
    // 5. Dehydrate the whole file, then check that nothing is left on disk.
    // SAFETY: `handle` is open; offset 0, length -1 is the whole file.
    if unsafe { CfDehydratePlaceholder(handle, 0, -1, CF_DEHYDRATE_FLAG_NONE, None) }.is_err() {
        return fail("dehydrate");
    }
    between(Step::Dehydrated, handle);
    match standard_info(handle) {
        Some((_, 0)) => {}
        Some(_) => return fail("still_on_disk"),
        None => return fail("verify"),
    }
    // 7. The pin as it was (by path after the close when the handle cannot).
    // SAFETY: `handle` is open.
    if pin != CF_PIN_STATE_UNSPECIFIED && !unsafe { set_pin(handle, pin) } {
        restore.pin_lost = Some(pin);
    }
    Emptied::Emptied
}

/// The empty step for every poisoned placeholder of `db` under `sync_root` (Amendment 4's rules for
/// `kind`). Returns each file's outcome.
pub(crate) fn sweep_poisoned(db: &crate::state_db::StateDb, sync_root: &Path, kind: SweepKind) -> Vec<(String, Emptied)> {
    sweep_poisoned_with(db, sync_root, kind, None, &mut |_, _| {})
}

/// [`sweep_poisoned`], for `only` one file when given, with [`empty_poisoned_with`]'s `between`.
pub(crate) fn sweep_poisoned_with(
    db: &crate::state_db::StateDb,
    sync_root: &Path,
    kind: SweepKind,
    only: Option<&str>,
    between: &mut dyn FnMut(Step, HANDLE),
) -> Vec<(String, Emptied)> {
    let rows = match only {
        Some(id) => db.poison(id).map(|row| row.into_iter().collect::<Vec<_>>()),
        None => db.poisoned(),
    };
    let rows = match rows {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!(error = %e, "poison sweep: could not read the poisoned placeholders");
            return Vec::new();
        }
    };
    let mut outcomes = Vec::new();
    for row in rows {
        // Amendment 4: a needs-attention row is never opened on a tick, and never emptied or cleared by
        // a sweep: only "Discard and download again" ends it.
        if row.needs_attention {
            // Checked at the process start and on close only: a row whose file is gone ends (its row
            // removed, or nothing at its path at the first sweep of the process); the file itself is
            // never opened, emptied or cleared here.
            if !crate::cf_range::sweep_opens(true, kind) {
                continue;
            }
            match db.get_file(&row.file_id) {
                Ok(None) => {
                    let _ = db.clear_poison(&row.file_id);
                    outcomes.push((row.file_id, Emptied::Gone));
                }
                Ok(Some(entry)) => {
                    let missing = crate::engine_bridge::local_file_path_under_sync_root(sync_root, &entry.path)
                        .is_ok_and(|p| std::fs::symlink_metadata(p).is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound));
                    if missing && crate::cf_range::clear_missing(kind) {
                        let _ = db.clear_poison(&row.file_id);
                        outcomes.push((row.file_id, Emptied::Missing));
                    } else if logged_once(&row.file_id, "needs_attention") {
                        tracing::warn!(file_id = %row.file_id, "poison sweep: changed on this device; kept as it is until discarded");
                    }
                }
                Err(_) => {}
            }
            continue;
        }
        let entry = match db.get_file(&row.file_id) {
            Ok(Some(entry)) => entry,
            Ok(None) => {
                // The row is gone (deleted or renamed away): nothing of it is left to empty here.
                let _ = db.clear_poison(&row.file_id);
                continue;
            }
            Err(e) => {
                // An unreadable row is not a gone one: the poison stays (I5), as the fetch check
                // counts a database error as poisoned.
                if logged_once(&row.file_id, "unreadable") {
                    tracing::warn!(error = %e, file_id = %row.file_id, "poison sweep: could not read the row; the poison stays");
                }
                continue;
            }
        };
        let Ok(path) = crate::engine_bridge::local_file_path_under_sync_root(sync_root, &entry.path) else {
            continue;
        };
        // A pin an earlier step could not put back: by path first.
        if let Some(pin) = row.pending_pin
            && set_pin_by_path(&path, CF_PIN_STATE(pin))
        {
            let _ = db.set_poison_pending_pin(&row.file_id, None);
        }
        let (outcome, pin_lost) = attempt(&path, Check::Key(row.key), between);
        if let Some(pin) = pin_lost {
            tracing::warn!(file_id = %row.file_id, "poison sweep: the pin could not be put back yet; kept to retry");
            let _ = db.set_poison_pending_pin(&row.file_id, Some(pin.0));
        }
        settle(db, &row.file_id, &outcome, kind);
        outcomes.push((row.file_id, outcome));
    }
    outcomes
}

/// What the database records for one outcome.
fn settle(db: &crate::state_db::StateDb, file_id: &str, outcome: &Emptied, kind: SweepKind) {
    match outcome {
        Emptied::Emptied | Emptied::Gone => {
            // Nothing of the failed attempt is on disk: the row says so too (as "Free up space" does).
            if *outcome == Emptied::Emptied
                && let Err(e) = db.mark_cloud_only_after_dehydrate(&[file_id.to_string()], chrono::Utc::now().timestamp())
            {
                tracing::warn!(error = %e, file_id = %file_id, "poison sweep: emptied, but the row could not be marked cloud-only");
            }
            if let Err(e) = db.clear_poison(file_id) {
                tracing::warn!(error = %e, file_id = %file_id, "poison sweep: emptied, but the poison could not be cleared");
            }
        }
        Emptied::Busy => tracing::debug!(file_id = %file_id, "poison sweep: still open; next time"),
        Emptied::Missing if crate::cf_range::clear_missing(kind) => {
            tracing::info!(file_id = %file_id, "poison sweep: missing at the first sweep of this process; poison cleared");
            let _ = db.clear_poison(file_id);
        }
        Emptied::Missing => tracing::debug!(file_id = %file_id, "poison sweep: nothing at its path yet; next time"),
        Emptied::NeedsAttention => {
            let _ = db.mark_poison_needs_attention(file_id);
            let _ = db.set_status(file_id, crate::state_db::FileStatus::Error);
            if logged_once(file_id, "needs_attention") {
                tracing::warn!(file_id = %file_id, "poison sweep: changed on this device since the failed download; kept as it is");
            }
        }
        Emptied::Failed(step) => {
            if logged_once(file_id, step) {
                tracing::warn!(file_id = %file_id, step, "poison sweep: a step failed; restored");
            }
        }
    }
    if !matches!(outcome, Emptied::Failed(_) | Emptied::NeedsAttention) {
        forget_logged(file_id);
    }
}

pub(crate) use crate::cf_range::SweepKind;

/// The last warning logged per row, so a row that fails the same way every tick warns once.
static LOGGED: std::sync::Mutex<std::collections::BTreeMap<String, &'static str>> =
    std::sync::Mutex::new(std::collections::BTreeMap::new());

/// Whether `what` is new for `file_id` (and remember it).
fn logged_once(file_id: &str, what: &'static str) -> bool {
    LOGGED.lock().unwrap_or_else(|p| p.into_inner()).insert(file_id.to_string(), what) != Some(what)
}

fn forget_logged(file_id: &str) {
    LOGGED.lock().unwrap_or_else(|p| p.into_inner()).remove(file_id);
}

/// Has this process swept yet? The first sweep is the process-start pass (Amendment 4: a process, not
/// an engine run or a bridge address).
static SWEPT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The engine's periodic trigger (and so the process-start pass): the connected bridge's database.
pub fn sweep_poisoned_now(sync_root: &Path) {
    let Some(bridge) = super::bridge() else { return };
    if bridge.is_revoked() || bridge.is_stopping() {
        return;
    }
    let kind = if SWEPT.swap(true, std::sync::atomic::Ordering::SeqCst) {
        SweepKind::Tick
    } else {
        SweepKind::ProcessStart
    };
    let emptied = sweep_poisoned(bridge.db(), sync_root, kind)
        .iter()
        .filter(|(_, o)| *o == Emptied::Emptied)
        .count();
    if emptied > 0 {
        tracing::info!(emptied, "poisoned placeholders emptied");
    }
}

/// The close trigger: a handle on `file_id` closed. If it is poisoned, that one file's empty step runs
/// off the callback thread (a Cloud Files callback must never wait on the empty step's oplock).
pub(super) fn after_close(file_id: &str) {
    let Some(bridge) = super::bridge() else { return };
    if !matches!(bridge.db().poison(file_id), Ok(Some(_))) {
        return;
    }
    let (Some(runtime), Some(root)) = (super::runtime(), super::sync_root()) else {
        return;
    };
    let file_id = file_id.to_string();
    runtime.spawn_blocking(move || {
        if let Some(bridge) = super::bridge() {
            let _ = sweep_poisoned_with(bridge.db(), &root, SweepKind::Close, Some(&file_id), &mut |_, _| {});
        }
    });
}

/// Spec S6, the first poisoner (W2): a fetch failed after it wrote into `path` (or found another
/// identity's bytes there). Not in sync first, then the content key, then the durable record, then
/// the row's Error status. Every later fetch is refused at once; the empty step runs at the next
/// close, tick or process start.
pub(crate) fn poison_now(db: &crate::state_db::StateDb, file_id: &str, path: &Path, reason: &'static str) {
    if let Err(e) = super::placeholders::mark_not_in_sync_path(path) {
        tracing::warn!(file_id = %file_id, error = %e, "poison: could not mark the placeholder not in sync");
    }
    let key = content_key(path);
    if let Err(e) = db.record_poison(file_id, reason, key, chrono::Utc::now().timestamp()) {
        tracing::error!(file_id = %file_id, error = %e, "poison: could not record it; the fetch failed all the same");
    }
    let _ = db.set_status(file_id, crate::state_db::FileStatus::Error);
    tracing::warn!(file_id = %file_id, reason, "placeholder poisoned after a failed download");
}

/// Why "Discard and download again" did not empty the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DiscardRefused {
    NotPoisoned,
    /// The file is open somewhere: close it first.
    InUse,
    Missing,
    Failed(&'static str),
}

/// Amendment 4's one action, "Discard and download again": empty the poisoned file at `file_id`'s path
/// under the exclusive handle whatever its content key, clear the poison, then `fetch` it (production:
/// our own `CfHydratePlaceholder`, a normal whole-file fetch).
pub(crate) fn discard_now(
    db: &crate::state_db::StateDb,
    sync_root: &Path,
    file_id: &str,
    fetch: impl FnOnce(std::path::PathBuf),
) -> Result<(), DiscardRefused> {
    if !matches!(db.poison(file_id), Ok(Some(_))) {
        return Err(DiscardRefused::NotPoisoned);
    }
    let entry = db.get_file(file_id).ok().flatten().ok_or(DiscardRefused::Missing)?;
    let path = crate::engine_bridge::local_file_path_under_sync_root(sync_root, &entry.path).map_err(|_| DiscardRefused::Missing)?;
    let (outcome, pin_lost) = attempt(&path, Check::Discard, &mut |_, _| {});
    if let Some(pin) = pin_lost {
        let _ = db.set_poison_pending_pin(file_id, Some(pin.0));
    }
    match outcome {
        Emptied::Emptied | Emptied::Gone => {
            settle(db, file_id, &outcome, SweepKind::Close);
            forget_logged(file_id);
            fetch(path);
            Ok(())
        }
        Emptied::Busy => Err(DiscardRefused::InUse),
        Emptied::Missing => Err(DiscardRefused::Missing),
        Emptied::Failed(step) => Err(DiscardRefused::Failed(step)),
        Emptied::NeedsAttention => Err(DiscardRefused::Failed("check")),
    }
}

/// The IPC command's half: the connected bridge, its sync root, and a background whole-file fetch.
pub(crate) fn discard_and_download_again(file_id: &str) -> Result<(), DiscardRefused> {
    let bridge = super::bridge().ok_or(DiscardRefused::Failed("not_connected"))?;
    let root = super::sync_root().ok_or(DiscardRefused::Failed("not_connected"))?;
    let runtime = super::runtime().ok_or(DiscardRefused::Failed("not_connected"))?;
    discard_now(bridge.db(), &root, file_id, |path| {
        runtime.spawn_blocking(move || {
            if let Err(e) = super::placeholders::hydrate_placeholder(&path) {
                tracing::warn!(error = %e, "discard and download again: the new download failed");
            }
        });
    })
}
