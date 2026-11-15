//! The non-Windows reclaim (task 2526): one copy at a time, checked right before it is removed.

use super::*;

/// Unix reclaim path (macOS File Provider / Linux FUSE): the engine stores hydrated copies as separate files, so a
/// `remove_file` genuinely reclaims the bytes. `go_on` is asked before each file, and once more right before the
/// unlink: once it says no, no further file is touched, and what was freed until then is the answer.
///
/// Task 2526: a copy INSIDE the sync folder (Linux: the pin's hydrate writes the folder file itself and records it
/// as the cache copy; nothing watches it) may hold an edit nobody detected. Each such copy is decided and removed
/// on its own, in this order; a failed step keeps the copy and its row, and lists it in `not_freed`:
/// 1. the recorded path is a plain file (not a link, not a pipe), the row holds agreed content (`local_hash`) and
///    no upload of the file is queued, and the copy was last changed at least `HASH_CACHE_SETTLED` ago (file times
///    are coarse, so a younger copy cannot be told from a write in the same tick);
/// 2. its [`ContentFingerprint`] (size, times, device and inode) is the same before and after its SHA-256 is
///    streamed, the open handle is the file the path names, and the hash equals `local_hash`;
/// 3. right before the unlink, for every copy: `go_on` again, the row is still `Local` and unpinned, no upload is
///    queued;
/// 4. sync folder copies only: a write lease (Linux `F_SETLEASE`, `F_WRLCK`) is taken on the open descriptor, which
///    the kernel grants only while no other descriptor has the file open, then the fingerprint is taken once more;
/// 5. the file is removed, and only then is its row marked online only.
///
/// **Detected:** a program that wrote the copy, or replaced it by a rename, at any point from step 1 to the last
/// fingerprint; a program that has the copy open (read or write) when the lease is asked for (the copy is kept,
/// `KEPT_OPEN_ELSEWHERE`); a file system or user that cannot grant a lease, and every non-Linux system, where this
/// cannot be told (`KEPT_NO_LEASE`: the copy is kept).
/// **Not detected:** a program that opens the copy, or writes it, in the microseconds between the lease and the
/// unlink (the kernel makes such an opener wait for the lease to end, and it then holds a file no path names); a
/// write that keeps the size and the times on a file system whose time steps are coarser than `HASH_CACHE_SETTLED`
/// (FAT's two seconds). The lease's break signal is sent as `SIGURG` (ignored by default), never as the default
/// `SIGIO`, which would end the process. This is not Windows' exclusive handle, which holds across the check and the
/// action.
///
/// A crash between the unlink and the mark leaves a `Local` row whose copy is gone; the next reclaim finds the
/// path missing (confirmed with `symlink_metadata`) and marks the row online only. Copies in the engine's own cache
/// directories are the engine's alone and need no byte check, but get step 3.
#[cfg(not(target_os = "windows"))]
pub fn free_up_space_unix(
    identity: &crate::identity::EngineIdentity,
    db: &crate::state_db::StateDb,
    sync_root: &std::path::Path,
    go_on: &dyn Fn() -> bool,
) -> Result<FreeUpSpaceResult, String> {
    reclaim_unix(
        identity,
        db,
        sync_root,
        go_on,
        &Seams {
            remove: &|path| std::fs::remove_file(path),
            at: &|_, _| {},
            now: &std::time::SystemTime::now,
        },
    )
}

/// What [`reclaim_unix`] does that a test replaces: the unlink, the clock, and a call at two points of one copy's
/// check.
#[cfg(not(target_os = "windows"))]
pub(crate) struct Seams<'a> {
    pub remove: &'a dyn Fn(&std::path::Path) -> std::io::Result<()>,
    /// Called with `("read", file_id)` after the bytes were hashed and before the second fingerprint, and with
    /// `("delete", file_id)` before step 3.
    pub at: &'a dyn Fn(&str, &str),
    /// The time a copy's age is measured at (step 1).
    pub now: &'a dyn Fn() -> std::time::SystemTime,
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn reclaim_unix(
    identity: &crate::identity::EngineIdentity,
    db: &crate::state_db::StateDb,
    sync_root: &std::path::Path,
    go_on: &dyn Fn() -> bool,
    seams: &Seams<'_>,
) -> Result<FreeUpSpaceResult, String> {
    // Snapshot every UNPINNED, LOCAL file's cache copy; each row is looked at again right before it is touched.
    let rows = db
        .disposable_unpinned_cache_rows()
        .map_err(|e| format!("list cache paths: {e}"))?;
    let allowed_roots = free_up_space_allowed_roots(identity, sync_root);
    let sync_canonical = sync_root.canonicalize().unwrap_or_else(|_| sync_root.to_path_buf());

    let mut bytes_freed: u64 = 0;
    let mut not_freed: Vec<NotFreed> = Vec::new();
    let mut freed_ids: Vec<String> = Vec::new();
    for (file_id, path, agreed) in rows {
        if !go_on() {
            tracing::info!("free_up_space: stopped before the last file (the session that asked is gone)");
            break;
        }
        let outcome = reclaim_row(
            db,
            &file_id,
            std::path::Path::new(&path),
            agreed.as_deref(),
            (&allowed_roots, &sync_canonical),
            go_on,
            seams,
        );
        match outcome {
            Ok(freed) => {
                bytes_freed = bytes_freed.saturating_add(freed);
                // Only a copy that is gone (or has none to remove) has its row marked online only.
                match db.mark_cloud_only_after_dehydrate(std::slice::from_ref(&file_id), now_unix_seconds()) {
                    Ok(1) => freed_ids.push(file_id),
                    Ok(_) => {
                        tracing::warn!(file_id = %file_id, "free_up_space: row no longer eligible after its copy was removed")
                    }
                    Err(error) => {
                        tracing::warn!(file_id = %file_id, error = %error, "free_up_space: could not mark row online only")
                    }
                }
            }
            Err(reason) => {
                tracing::warn!(file_id = %file_id, reason, "free_up_space: kept a copy");
                not_freed.push(NotFreed { file_id, reason });
            }
        }
    }
    #[cfg(target_os = "linux")]
    remove_linux_freedesktop_thumbnails_for_evicted_files(db, sync_root, &freed_ids);

    tracing::info!(
        bytes_freed,
        kept = not_freed.len(),
        "free_up_space evicted unpinned cache"
    );
    Ok(FreeUpSpaceResult { bytes_freed, not_freed })
}

/// One row's copy, steps 1 to 5 of [`free_up_space_unix`]. `Ok(bytes removed)` when the copy is gone (or was
/// already), or the plain words for why it stays. `roots` = (allowed cache roots, the sync root).
#[cfg(not(target_os = "windows"))]
fn reclaim_row(
    db: &crate::state_db::StateDb,
    file_id: &str,
    original: &std::path::Path,
    agreed: Option<&str>,
    roots: (&[PathBuf], &std::path::Path),
    go_on: &dyn Fn() -> bool,
    seams: &Seams<'_>,
) -> Result<u64, &'static str> {
    let (allowed_roots, sync_canonical) = roots;
    // Gone is NotFound, confirmed on the recorded path itself (a dangling link, or a denied folder, is not gone).
    let canonical = match original.canonicalize() {
        Ok(canonical) => canonical,
        Err(error)
            if error.kind() == std::io::ErrorKind::NotFound
                && matches!(std::fs::symlink_metadata(original), Err(e) if e.kind() == std::io::ErrorKind::NotFound) =>
        {
            return Ok(0);
        }
        Err(_) => return Err(KEPT_UNREADABLE),
    };
    // Outside every allowed cache root: nothing is deleted (the row still goes online only, as before).
    if !allowed_roots.iter().any(|root| canonical.starts_with(root)) {
        return Ok(0);
    }
    if canonical.starts_with(sync_canonical) {
        reclaim_sync_copy(db, file_id, original, &canonical, agreed, go_on, seams)
    } else {
        // The engine's own cache directory: no byte check, by design.
        let size = std::fs::metadata(&canonical).map(|m| m.len()).unwrap_or(0);
        last_look(db, file_id, go_on, seams)?;
        remove_copy(&canonical, seams).map(|gone| if gone { size } else { 0 })
    }
}

/// Steps 1, 2, 4 and 5 for one sync folder copy (step 3 is [`last_look`]).
#[cfg(not(target_os = "windows"))]
fn reclaim_sync_copy(
    db: &crate::state_db::StateDb,
    file_id: &str,
    original: &std::path::Path,
    canonical: &std::path::Path,
    agreed: Option<&str>,
    go_on: &dyn Fn() -> bool,
    seams: &Seams<'_>,
) -> Result<u64, &'static str> {
    use crate::engine_bridge::ContentFingerprint;
    use sha2::{Digest, Sha256};

    // 1. A plain file, never a link or a pipe: opening a pipe blocks, and a link would act on its target. Looked
    // at before anything is opened.
    for path in [original, canonical] {
        match std::fs::symlink_metadata(path) {
            Ok(meta) if meta.file_type().is_file() => {}
            Ok(_) => return Err(KEPT_NOT_A_FILE),
            Err(_) => return Err(KEPT_UNREADABLE),
        }
    }
    let Some(agreed) = agreed else {
        return Err(KEPT_NO_AGREED);
    };
    // Anti-goal 3 (as the Windows stamp): the agreed content counts only while no upload of the file is queued.
    upload_not_queued(db, file_id)?;

    // 2. The same file, unchanged, across the read; and not written within one timer tick of the read.
    let before = ContentFingerprint::of(canonical).map_err(|_| KEPT_UNREADABLE)?;
    if !before.settled_at((seams.now)()) {
        return Err(KEPT_JUST_WRITTEN);
    }
    let mut file = std::fs::File::open(canonical).map_err(|_| KEPT_UNREADABLE)?;
    let mut hasher = Sha256::new();
    // Stream it; never load a large file into memory.
    std::io::copy(&mut file, &mut hasher).map_err(|_| KEPT_UNREADABLE)?;
    (seams.at)("read", file_id);
    unchanged(canonical, &before, &file)?;
    if format!("{:x}", hasher.finalize()) != agreed {
        return Err(KEPT_EDITED);
    }

    // 3 and 4. The row, the queue and the session as they are now; then no other descriptor on the file, and the
    // file once more.
    last_look(db, file_id, go_on, seams)?;
    hold_write_lease(&file)?;
    unchanged(canonical, &before, &file)?;

    // 5. Gone, or kept: the caller marks the row only for a copy that is gone.
    let len = file.metadata().map_err(|_| KEPT_UNREADABLE)?.len();
    remove_copy(canonical, seams).map(|gone| if gone { len } else { 0 })
}

/// Step 3: right before an unlink, the session still wants this, the row is still `Local` and unpinned, and no
/// upload of the file is queued.
#[cfg(not(target_os = "windows"))]
fn last_look(
    db: &crate::state_db::StateDb,
    file_id: &str,
    go_on: &dyn Fn() -> bool,
    seams: &Seams<'_>,
) -> Result<(), &'static str> {
    (seams.at)("delete", file_id);
    if !go_on() {
        return Err(KEPT_STOPPED);
    }
    row_still_free(db, file_id)?;
    upload_not_queued(db, file_id)
}

/// The row is still a `Local`, unpinned file. A database error is not an answer: the copy is kept.
#[cfg(not(target_os = "windows"))]
fn row_still_free(db: &crate::state_db::StateDb, file_id: &str) -> Result<(), &'static str> {
    use crate::state_db::{FileStatus, PinState};
    match (db.get_file(file_id), db.get_file_contract_state(file_id)) {
        (Ok(Some(entry)), Ok(Some(contract))) => {
            if entry.status != FileStatus::Local {
                Err(KEPT_STATE_CHANGED)
            } else if contract.effective_pin_state() == PinState::Pinned {
                Err(KEPT_NOW_KEPT)
            } else {
                Ok(())
            }
        }
        (Ok(_), Ok(_)) => Err(KEPT_STATE_CHANGED),
        _ => Err(KEPT_CHECK_FAILED),
    }
}

/// No upload of the file is queued. A database error is not an answer: the copy is kept, in its own words.
#[cfg(not(target_os = "windows"))]
fn upload_not_queued(db: &crate::state_db::StateDb, file_id: &str) -> Result<(), &'static str> {
    if db.sync_folder_upload_queued(file_id).map_err(|_| KEPT_CHECK_FAILED)? {
        Err(KEPT_UPLOAD_QUEUED)
    } else {
        Ok(())
    }
}

/// The file at `path` is the one `before` fingerprinted, and the open `file` is the file the path names.
#[cfg(not(target_os = "windows"))]
fn unchanged(
    path: &std::path::Path,
    before: &crate::engine_bridge::ContentFingerprint,
    file: &std::fs::File,
) -> Result<(), &'static str> {
    use std::os::unix::fs::MetadataExt;
    let now = crate::engine_bridge::ContentFingerprint::of(path).map_err(|_| KEPT_CHANGED)?;
    let handle = file.metadata().map_err(|_| KEPT_UNREADABLE)?;
    let named = std::fs::symlink_metadata(path).map_err(|_| KEPT_CHANGED)?;
    if now != *before || (handle.dev(), handle.ino()) != (named.dev(), named.ino()) {
        return Err(KEPT_CHANGED);
    }
    Ok(())
}

/// Step 4: a write lease on `file`, which the kernel grants only while no other descriptor has the file open (any
/// process, any mode). Refused or unsupported: the copy is kept.
#[cfg(all(
    target_os = "linux",
    any(
        target_arch = "x86",
        target_arch = "x86_64",
        target_arch = "arm",
        target_arch = "aarch64",
        target_arch = "riscv64"
    )
))]
fn hold_write_lease(file: &std::fs::File) -> Result<(), &'static str> {
    use std::os::fd::AsRawFd;
    // `fcntl` command 10 on these architectures (the `libc` crate does not export it for every target).
    const F_SETSIG: libc::c_int = 10;
    let fd = file.as_raw_fd();
    // SAFETY: `fcntl` on a descriptor this function's caller owns, with integer arguments only.
    unsafe {
        // The break signal as SIGURG (default: ignored); the default SIGIO would end the process.
        if libc::fcntl(fd, F_SETSIG, libc::SIGURG) == -1 {
            return Err(KEPT_NO_LEASE);
        }
        if libc::fcntl(fd, libc::F_SETLEASE, libc::F_WRLCK) == -1 {
            return Err(match std::io::Error::last_os_error().raw_os_error() {
                Some(code) if code == libc::EAGAIN => KEPT_OPEN_ELSEWHERE,
                _ => KEPT_NO_LEASE,
            });
        }
    }
    Ok(())
}

/// Where there are no leases (macOS), or the signal number is not known here, a copy in the sync folder cannot be
/// shown to be open nowhere else: kept.
#[cfg(not(any(
    target_os = "windows",
    all(
        target_os = "linux",
        any(
            target_arch = "x86",
            target_arch = "x86_64",
            target_arch = "arm",
            target_arch = "aarch64",
            target_arch = "riscv64"
        )
    )
)))]
fn hold_write_lease(_file: &std::fs::File) -> Result<(), &'static str> {
    Err(KEPT_NO_LEASE)
}

/// The unlink: `Ok(true)` removed, `Ok(false)` already gone, `Err` kept (the system refused).
#[cfg(not(target_os = "windows"))]
fn remove_copy(path: &std::path::Path, seams: &Seams<'_>) -> Result<bool, &'static str> {
    match (seams.remove)(path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => {
            tracing::warn!(error = %error, "free_up_space: could not remove a copy");
            Err(KEPT_REMOVE_FAILED)
        }
    }
}
