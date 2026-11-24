//! Task dc-103c (design 1640 "Not done" (b)), Windows: a placeholder holding ranges of a version the server moved past
//! is emptied, given the current version's size and time, and its row written so the next open fetches the current
//! version (`StateDb::list_stale_ranges`, the rule).
//!
//! The order, under ONE handle opened with share 0 (`placeholders::open_exclusive_without_write_data`, as the stale
//! copy pass): while it is open nobody else can open the file, so no fetch starts and nothing writes.
//! 1. `content_seq` read with no fetch of the file in flight in this process (`content_seq_for_wipe`), before the
//!    handle, and again once it is open: a fetch that started in between moved it on, and the copy waits.
//! 2. The placeholder holds some bytes but not all (nothing: populate refreshes its identity; all: see `wipe`),
//!    carries this row's file id and is not pinned (a kept file is being fetched whole; left as it is).
//! 3. `CfDehydratePlaceholder` WITHOUT marking it in sync first: a placeholder that is not in sync holds a write
//!    (a change) and the dehydrate refuses it (`0x80070179`), so a change is never emptied.
//! 4. Nothing on disk, checked through the handle; the current size and time, marked in sync.
//! 5. The row, a compare-and-set on the sequence of step 1 (`StateDb::record_stale_ranges_wiped`).

use std::os::windows::io::AsRawHandle;
use std::path::Path;

use windows::Win32::Foundation::HANDLE;
use windows::Win32::Storage::CloudFilters::*;
use windows::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_NORMAL, FILE_BASIC_INFO};

use super::poison::{identity_of_handle, standard_info};
use crate::state_db::{StaleRanges, StateDb};

/// What the wipe did with one placeholder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wiped {
    /// Emptied at the current version's size; the row names the current version. The next open fetches it.
    Emptied,
    /// Not now; the word says why. The next tick tries again.
    Later(&'static str),
}

/// Empty `stale`'s placeholder at `path` (see the module doc).
pub fn wipe(db: &StateDb, stale: &StaleRanges, path: &Path) -> Wiped {
    let Some(seq_before) = crate::engine_bridge::content_seq_for_wipe(db, &stale.file_id) else {
        return Wiped::Later("fetch_in_flight");
    };
    // LD-0113 Amendment 7: a read-only Backups file is changed by the engine too; the attribute is set again after.
    let _read_only = super::read_only::Lifted::new(path);
    const SHARING_VIOLATION: i32 = 32;
    let file = match super::placeholders::open_exclusive_without_write_data(path) {
        Ok(file) => file,
        Err(e) if e.raw_os_error() == Some(SHARING_VIOLATION) => return Wiped::Later("in_use"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Wiped::Later("missing"),
        Err(_) => return Wiped::Later("open"),
    };
    // Only RANGES: a copy wholly on disk with no agreed content is not known to be the server's bytes, and the app
    // never empties such a copy (as Free up space refuses it, `local_bytes_agreed`).
    if super::placeholders::held_fully_on_disk(&file) {
        return Wiped::Later("whole");
    }
    let handle = HANDLE(file.as_raw_handle());
    // SAFETY: `handle` is `file`'s, open for the whole call.
    let outcome = unsafe { wipe_under(db, stale, handle, seq_before) };
    drop(file);
    outcome
}

/// Steps 1 (again) to 5 under `handle`.
///
/// SAFETY: `handle` must be open (exclusively) for the whole call.
unsafe fn wipe_under(db: &StateDb, stale: &StaleRanges, handle: HANDLE, seq_before: i64) -> Wiped {
    if crate::engine_bridge::content_seq_for_wipe(db, &stale.file_id) != Some(seq_before) {
        return Wiped::Later("fetch_since");
    }
    if identity_of_handle(handle).as_deref() != Some(stale.file_id.as_bytes()) {
        return Wiped::Later("identity");
    }
    match standard_info(handle) {
        None => return Wiped::Later("info"),
        // Nothing on disk: populate gives it the current identity (no wipe needed).
        Some((_, 0)) => return Wiped::Later("empty"),
        Some((pin, _)) if pin == CF_PIN_STATE_PINNED => return Wiped::Later("pinned"),
        Some(_) => {}
    }
    // SAFETY: `handle` is open; offset 0, length -1 is the whole file. Not in sync (a write): refused, kept.
    if unsafe { CfDehydratePlaceholder(handle, 0, -1, CF_DEHYDRATE_FLAG_NONE, None) }.is_err() {
        return Wiped::Later("dehydrate");
    }
    match standard_info(handle) {
        Some((_, 0)) => {}
        _ => return Wiped::Later("still_on_disk"),
    }
    // The bytes are gone: a failure here leaves the older length, which the first fetch corrects (a failed fetch,
    // then a resize), so it is logged, not undone.
    // SAFETY: `handle` is open.
    if let Err(e) = unsafe { set_current_metadata(handle, stale) } {
        tracing::warn!(file_id = %stale.file_id, error = %e, "stale ranges: emptied, but the new size was not set");
    }
    match db.record_stale_ranges_wiped(stale, seq_before) {
        Ok(true) => Wiped::Emptied,
        // The placeholder is empty whatever the row says: with nothing on disk the next session serves the current
        // version (`range_prep`), and populate records its identity.
        Ok(false) => {
            tracing::warn!(file_id = %stale.file_id, "stale ranges: emptied, but the row had moved");
            Wiped::Later("row_moved")
        }
        Err(e) => {
            tracing::warn!(file_id = %stale.file_id, error = %e, "stale ranges: emptied, but the row was not written");
            Wiped::Later("row")
        }
    }
}

/// The current version's size and time on the (empty) placeholder behind `handle`, marked in sync (the server's
/// truth, never a local change). The FileIdentity is kept.
///
/// SAFETY: `handle` is open.
unsafe fn set_current_metadata(handle: HANDLE, stale: &StaleRanges) -> windows::core::Result<()> {
    let metadata = CF_FS_METADATA {
        BasicInfo: FILE_BASIC_INFO {
            CreationTime: 0,
            LastAccessTime: 0,
            LastWriteTime: super::placeholders::unix_to_filetime(stale.modified_at),
            ChangeTime: 0,
            FileAttributes: FILE_ATTRIBUTE_NORMAL.0,
        },
        FileSize: stale.size_bytes.max(0),
    };
    // SAFETY: the caller's contract; `metadata` outlives the call, which copies what it needs.
    unsafe {
        CfUpdatePlaceholder(
            handle,
            Some(&metadata as *const CF_FS_METADATA),
            None,
            0,
            None,
            CF_UPDATE_FLAG_MARK_IN_SYNC,
            None,
            None,
        )
    }
}
