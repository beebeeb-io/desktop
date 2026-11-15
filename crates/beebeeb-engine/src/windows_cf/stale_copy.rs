//! Task 2523 (F1), Windows: a placeholder that holds the agreed content of a version older than the current one is
//! emptied and given the current version's size and time, so the next open fetches the current version. ONE exclusive
//! handle for every step, in [`crate::engine_bridge::REFRESH_ORDER`] (platform-neutral data, unit-tested on every
//! platform): the stamp pass's handle (`placeholders::open_exclusive_without_write_data`: share 0, read data, WRITE_DAC),
//! through which `stamp_if_agreed` hashes the bytes and marks them in sync. While it is open no other handle can be
//! opened, so nothing can write between the check of the bytes and the dehydrate. The calls that empty the
//! placeholder are LD-0105's, in the order its P1b spike measured (`poison.rs`): record the pin, unspecified, IN_SYNC,
//! dehydrate, verify nothing is on disk, put the pin back. (The poison step's own `CfOpenFileWithOplock` handle cannot
//! read the bytes: the native test measured `Later("read")` on it.) Before any change the placeholder must be this
//! row's (its FileIdentity), its bytes must be the row's agreed content, and the rule must still name the row: a
//! change is never touched.
//!
//! A kept file is fetched again by the caller after the handle is closed (`placeholders::hydrate_placeholder`, as a
//! pin does), off the tick.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{LazyLock, Mutex, PoisonError};

use std::os::windows::io::AsRawHandle;

use windows::Win32::Foundation::HANDLE;
use windows::Win32::Storage::CloudFilters::*;
use windows::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_NORMAL, FILE_BASIC_INFO};

use super::poison::{identity_of_handle, keep_lost_pin, set_pin, set_pin_by_path, standard_info};
use crate::engine_bridge::{ContentFingerprint, REFRESH_ORDER, RefreshStep};
use crate::state_db::{StaleCopy, StateDb};

/// What the refresh did with one placeholder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refreshed {
    /// Emptied, at the current version's size and time; the row records it (cloud-only, its older agreed content
    /// kept as the fetched candidate). `kept`: the placeholder was pinned, as its handle read it (review P2-2: a pin set
    /// in Explorer that the database has not recorded yet is kept too), so the caller fetches it again.
    Emptied { kept: bool },
    /// Its bytes are not the agreed content: a change, left exactly as it is.
    Changed,
    /// Not now; the word says why (`in_use`, `not_wholly_on_disk`, `identity`, `row_moved`, a failed step whose
    /// state was put back). The next tick tries again.
    Later(&'static str),
}

/// Refresh `copy`'s placeholder at `path` (see the module doc).
pub fn refresh(db: &StateDb, copy: &StaleCopy, path: &Path) -> Refreshed {
    refresh_with(db, copy, path, &mut |_, _| {})
}

/// [`refresh`], calling `between` after each step with the handle every step uses (a native test acts there).
pub fn refresh_with(
    db: &StateDb,
    copy: &StaleCopy,
    path: &Path,
    between: &mut dyn FnMut(RefreshStep, HANDLE),
) -> Refreshed {
    // This process's own fetch of the file is refused while the pass holds it (task 2284 A1.2), and the read below
    // is never a fetch: a placeholder not wholly on disk is left for now (its open would ask for the rest).
    let _scope = crate::no_fetch_scope::enter(&[copy.file_id.as_str()]);
    if !super::placeholders::fully_on_disk(path) {
        return Refreshed::Later("not_wholly_on_disk");
    }
    // Review P3-5: a copy read last time and found to differ is not opened and read again while it is the same.
    let before = ContentFingerprint::of(path).ok();
    if still_differs(copy, before) {
        return Refreshed::Changed;
    }
    // LD-0113 Amendment 7: a read-only Backups file is changed by the engine too; the attribute is set again after.
    let _read_only = super::read_only::Lifted::new(path);
    const SHARING_VIOLATION: i32 = 32;
    let mut undo = Undo::default();
    let outcome = match super::placeholders::open_exclusive_without_write_data(path) {
        // SAFETY: the handle belongs to `file`, open for the whole call; `file` is closed when this arm ends, before
        // anything below opens the path again.
        Ok(mut file) => unsafe {
            let handle = HANDLE(file.as_raw_handle());
            refresh_under(db, copy, handle, &mut file, between, &mut undo)
        },
        Err(e) if e.raw_os_error() == Some(SHARING_VIOLATION) => return Refreshed::Later("in_use"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Refreshed::Later("missing"),
        Err(_) => return Refreshed::Later("open"),
    };
    remember_differs(copy, before, outcome);
    if undo.failed && !undo.not_in_sync {
        // A change step failed and NOT_IN_SYNC could not be put back through the handle: by path, so what is left
        // on disk is never left marked in sync (LD-0105 I6). Agreed bytes are marked in sync again by the stamp pass.
        let _ = super::placeholders::mark_not_in_sync_path(path);
    }
    // A pin the handle could not put back: by path, else recorded for the sweep to retry (`cf_pin_restore`).
    keep_lost_pin(
        db,
        &copy.file_id,
        undo.pin_lost.filter(|pin| !set_pin_by_path(path, *pin)),
    );
    outcome
}

/// Review P3-5: the copies whose bytes were read through the handle and found to differ from their agreed content,
/// with that agreed content and the fingerprint (size, times, volume and file index) the copy had before the read.
/// While a copy keeps both it still differs (a write moves its change time, a file renamed over it has another
/// index), so it is not opened with share 0, locking it against its own editor, and read in full every tick. Only
/// "differs" is remembered: a copy is emptied only after its bytes were read through the exclusive handle.
static DIFFERS: LazyLock<Mutex<HashMap<String, (String, ContentFingerprint)>>> = LazyLock::new(Default::default);

fn still_differs(copy: &StaleCopy, now: Option<ContentFingerprint>) -> bool {
    now.is_some_and(|now| {
        DIFFERS
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&copy.file_id)
            .is_some_and(|(agreed, seen)| *agreed == copy.local_hash && *seen == now)
    })
}

fn remember_differs(copy: &StaleCopy, before: Option<ContentFingerprint>, outcome: Refreshed) {
    let mut differs = DIFFERS.lock().unwrap_or_else(PoisonError::into_inner);
    match (outcome, before) {
        (Refreshed::Changed, Some(seen)) => {
            differs.insert(copy.file_id.clone(), (copy.local_hash.clone(), seen));
        }
        _ => {
            differs.remove(&copy.file_id);
        }
    }
}

/// What a failed change step put back through the handle.
#[derive(Default)]
struct Undo {
    failed: bool,
    not_in_sync: bool,
    pin_lost: Option<CF_PIN_STATE>,
}

/// A change step failed: NOT_IN_SYNC, then the pin, through the handle, before it is released (LD-0105 step 6).
///
/// SAFETY: `handle` is open.
unsafe fn fail(handle: HANDLE, step: &'static str, pin: CF_PIN_STATE, undo: &mut Undo) -> Refreshed {
    undo.failed = true;
    // SAFETY: the caller's contract; both calls only set state on the handle.
    unsafe {
        undo.not_in_sync =
            CfSetInSyncState(handle, CF_IN_SYNC_STATE_NOT_IN_SYNC, CF_SET_IN_SYNC_FLAG_NONE, None).is_ok();
        if pin != CF_PIN_STATE_UNSPECIFIED && !set_pin(handle, pin) {
            undo.pin_lost = Some(pin);
        }
    }
    Refreshed::Later(step)
}

/// The steps of [`REFRESH_ORDER`] under `handle`.
///
/// SAFETY: `handle` must be `file`'s, open (exclusively) for the whole call; every step goes through it.
unsafe fn refresh_under(
    db: &StateDb,
    copy: &StaleCopy,
    handle: HANDLE,
    file: &mut std::fs::File,
    between: &mut dyn FnMut(RefreshStep, HANDLE),
    undo: &mut Undo,
) -> Refreshed {
    let mut pin = CF_PIN_STATE_UNSPECIFIED;
    for step in REFRESH_ORDER {
        match step {
            RefreshStep::CheckIdentity => {
                if identity_of_handle(handle).as_deref() != Some(copy.file_id.as_bytes()) {
                    return Refreshed::Later("identity");
                }
            }
            // Review F9 (as `stamp_if_agreed`): wholly on disk, read through the handle that then reads the bytes.
            RefreshStep::CheckAgreedBytes if !super::placeholders::held_fully_on_disk(file) => {
                return Refreshed::Later("not_wholly_on_disk");
            }
            RefreshStep::CheckAgreedBytes => match hash_through(file) {
                Some(hash) if hash == copy.local_hash => {}
                Some(_) => return Refreshed::Changed,
                None => return Refreshed::Later("read"),
            },
            RefreshStep::CheckRow => {
                if !db.stale_copy_still_behind(copy).unwrap_or(false) {
                    return Refreshed::Later("row_moved");
                }
            }
            RefreshStep::RecordState => match standard_info(handle) {
                Some((now, _)) => pin = now,
                None => return Refreshed::Later("info"),
            },
            RefreshStep::Unpin => {
                // SAFETY: `handle` is open.
                if pin != CF_PIN_STATE_UNSPECIFIED && !unsafe { set_pin(handle, CF_PIN_STATE_UNSPECIFIED) } {
                    // SAFETY: as above.
                    return unsafe { fail(handle, "unpin", pin, undo) };
                }
            }
            RefreshStep::MarkInSync => {
                // SAFETY: `handle` is open.
                if unsafe { CfSetInSyncState(handle, CF_IN_SYNC_STATE_IN_SYNC, CF_SET_IN_SYNC_FLAG_NONE, None) }
                    .is_err()
                {
                    // SAFETY: as above.
                    return unsafe { fail(handle, "in_sync", pin, undo) };
                }
            }
            RefreshStep::Dehydrate => {
                // SAFETY: `handle` is open; offset 0, length -1 is the whole file.
                if unsafe { CfDehydratePlaceholder(handle, 0, -1, CF_DEHYDRATE_FLAG_NONE, None) }.is_err() {
                    // SAFETY: as above.
                    return unsafe { fail(handle, "dehydrate", pin, undo) };
                }
            }
            RefreshStep::VerifyEmpty => match standard_info(handle) {
                Some((_, 0)) => {}
                // SAFETY: `handle` is open.
                Some(_) => return unsafe { fail(handle, "still_on_disk", pin, undo) },
                // SAFETY: as above.
                None => return unsafe { fail(handle, "verify", pin, undo) },
            },
            RefreshStep::SetCurrentMetadata => {
                // The bytes are gone: a failure here leaves the older length, which the first fetch corrects (a
                // failed fetch, then a resize), so it is logged, not undone.
                // SAFETY: `handle` is open.
                if let Err(e) = unsafe { set_current_metadata(handle, copy) } {
                    tracing::warn!(file_id = %copy.file_id, error = %e, "stale copy: emptied, but its new size was not set");
                }
            }
            RefreshStep::RecordRow => match db.record_stale_copy_emptied(copy) {
                Ok(true) => {}
                // The placeholder is empty whatever the row says; the next reconcile reads it as cloud-only.
                Ok(false) => tracing::warn!(file_id = %copy.file_id, "stale copy: emptied, but the row had moved"),
                Err(e) => {
                    tracing::warn!(file_id = %copy.file_id, error = %e, "stale copy: emptied, but the row was not written")
                }
            },
            RefreshStep::RestorePin => {
                // SAFETY: `handle` is open.
                if pin != CF_PIN_STATE_UNSPECIFIED && !unsafe { set_pin(handle, pin) } {
                    undo.pin_lost = Some(pin);
                }
            }
        }
        between(step, handle);
    }
    Refreshed::Emptied {
        kept: pin == CF_PIN_STATE_PINNED,
    }
}

/// The SHA-256 of `file`'s bytes, read through its handle from the start (the placeholder is wholly on disk, so
/// nothing is fetched). `None` when it cannot be read.
fn hash_through(file: &mut std::fs::File) -> Option<String> {
    use sha2::{Digest, Sha256};
    use std::io::{Seek, SeekFrom};
    file.seek(SeekFrom::Start(0)).ok()?;
    let mut hasher = Sha256::new();
    std::io::copy(file, &mut hasher).ok()?;
    Some(format!("{:x}", hasher.finalize()))
}

/// The current version's size and time on the (empty) placeholder behind `handle`, marked in sync: the metadata IS
/// the server's truth, never a local change. The FileIdentity is kept.
///
/// SAFETY: `handle` is open.
unsafe fn set_current_metadata(handle: HANDLE, copy: &StaleCopy) -> windows::core::Result<()> {
    let metadata = CF_FS_METADATA {
        BasicInfo: FILE_BASIC_INFO {
            CreationTime: 0,
            LastAccessTime: 0,
            LastWriteTime: super::placeholders::unix_to_filetime(copy.modified_at),
            ChangeTime: 0,
            FileAttributes: FILE_ATTRIBUTE_NORMAL.0,
        },
        FileSize: copy.size_bytes.max(0),
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
