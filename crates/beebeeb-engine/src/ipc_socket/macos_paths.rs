//! `macos_paths` items split out of `ipc_socket.rs`.

#[cfg(target_os = "macos")]
use super::*;

/// Resolve the daemon's IPC socket path inside the macOS shared App Group
/// container, given a caller-supplied home directory (a pure function so the
/// length budget is unit-testable without depending on the real environment
/// or the app-group entitlement itself).
///
/// This plays the same role as `FileManager
/// .containerURL(forSecurityApplicationGroupIdentifier:)` on the Swift side:
/// both the containing app and the File Provider extension
/// (`XPCBridge.swift`) resolve to the SAME real directory purely by sharing
/// the `application-groups` entitlement — `~/Library/Group Containers/<group
/// id>/` is not a guess, it's how macOS defines that entitlement. The daemon
/// builds the path directly rather than calling the real Foundation API
/// because that API requires the calling process to actually hold the
/// entitlement (a plain `cargo test` binary does not), which would make the
/// path-resolution logic itself untestable.
///
/// A Unix domain socket path is capped at `sizeof(sockaddr_un.sun_path)` on
/// macOS: **104 bytes, including the NUL terminator** (`<sys/un.h>`) — so the
/// file name under the (already fairly long) group-container directory must
/// stay short. The old `beebeeb-daemon.sock` name does not fit once the
/// group-container prefix is added, for most real usernames; `ipc.sock`
/// does, with headroom to spare.
#[cfg(target_os = "macos")]
pub(super) fn macos_ipc_socket_path_in(
    identity: &crate::identity::EngineIdentity,
    home_dir: &std::path::Path,
) -> std::path::PathBuf {
    home_dir
        .join("Library")
        .join("Group Containers")
        .join(identity.macos_app_group())
        .join(MACOS_IPC_SOCKET_FILENAME)
}

/// Root cause of task 1670 issue 2 ("Couldn't communicate with a helper
/// application" opening any file from Finder): BOTH the daemon
/// (`io.beebeeb.app`) and the File Provider extension
/// (`io.beebeeb.app.FileProvider`) are sandboxed (`com.apple.security.app-
/// sandbox`, both entitlements files), and macOS gives every sandboxed
/// process its OWN per-bundle-ID temp directory — `std::env::temp_dir()` here
/// and `FileManager.default.temporaryDirectory` in `XPCBridge.swift` resolve
/// to two DIFFERENT real directories on disk, one per container. The
/// extension built its `fetchContents` destination from ITS OWN temp dir, so
/// it could never be `is_contained` in this process's `temp_root` — every real
/// hydration was rejected with "hydrate destination is not within an allowed
/// root" (confirmed via the unified log, task 1670's evidence capture: 5
/// occurrences in a 2h window, each immediately followed by the File Provider
/// host logging `[CRIT] Provider returned error 0 from domain
/// BeebeebFileProvider.BeebeebIPCError which is unsupported` — which is what
/// Finder surfaces as the generic "Couldn't communicate with a helper
/// application", masking this real cause).
///
/// The shared App Group container is the one directory both sandboxes are
/// ACTUALLY entitled to and already use for the IPC socket itself
/// (`ipc_socket_path`, proven working since task 1524) — this mirrors that
/// exact pattern for hydration staging instead of inventing a new mechanism.
/// `XPCBridge.hydrateDestinationURL(for:)` is the Swift-side mirror; keep the
/// directory name in sync with `MACOS_HYDRATE_CACHE_DIRNAME` there.
#[cfg(target_os = "macos")]
pub(super) fn macos_hydrate_cache_dir_in(
    identity: &crate::identity::EngineIdentity,
    home_dir: &std::path::Path,
) -> std::path::PathBuf {
    home_dir
        .join("Library")
        .join("Group Containers")
        .join(identity.macos_app_group())
        .join(MACOS_HYDRATE_CACHE_DIRNAME)
}

/// [`macos_ensure_private_staging_dir`], returning the directory held open.
///
/// The directory is opened once, without following a symlink, and must be a
/// real directory owned by this user ([`StagingDir::open`]). The mode and
/// the backup exclusion are then set through that descriptor, so a symlink
/// put in the directory's place can never redirect them to another folder.
#[cfg(target_os = "macos")]
pub(super) fn macos_open_private_staging_dir(dir: &std::path::Path) -> Result<StagingDir, StagingDirRefusal> {
    macos_open_private_staging_dir_with(dir, macos_exclude_from_backups)
}

/// [`macos_open_private_staging_dir`] with the backup exclusion as a
/// parameter, so a test can make it fail.
#[cfg(target_os = "macos")]
pub(super) fn macos_open_private_staging_dir_with(
    dir: &std::path::Path,
    exclude_from_backups: impl FnOnce(&StagingDir) -> std::io::Result<()>,
) -> Result<StagingDir, StagingDirRefusal> {
    use std::os::unix::fs::DirBuilderExt;

    // `recursive` returns Ok when `dir` already exists, including as a
    // symlink to a directory: the open below is what refuses that.
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .map_err(|e| StagingDirRefusal::Unavailable(e.kind()))?;
    let staging = StagingDir::open(dir)?;
    // Belt-and-braces: `DirBuilder::mode` is subject to `mkdir`'s normal
    // umask handling like any other creation call, and a directory left over
    // from an older build may already exist with looser permissions. Force it
    // down explicitly rather than trusting creation alone.
    staging
        .restrict_to_owner()
        .map_err(|e| StagingDirRefusal::Unavailable(e.kind()))?;
    if let Err(e) = exclude_from_backups(&staging) {
        // No path: it holds the home directory, and so the account name.
        tracing::warn!(error_kind = ?e.kind(), "could not exclude a macOS staging dir from backups");
    }
    Ok(staging)
}

/// Exclude the macOS hydrate-cache directory from Time Machine / iCloud
/// backups. It only ever holds short-lived DECRYPTED plaintext staged for a
/// live Finder open, and a stale local backup silently retaining a copy would
/// defeat the whole point of purging it on sign-out/lock/TTL (task 1670 round
/// 2).
///
/// Sets the extended attribute `com.apple.metadata:com_apple_backup_excludeItem`
/// = `com.apple.backupd` directly — the same xattr Foundation's
/// `URLResourceKey.isExcludedFromBackupKey` (formerly
/// `NSURLIsExcludedFromBackupKey`) sets under the hood — rather than going
/// through an ObjC/Foundation call, so this is callable, and testable via
/// `cargo test`, from the plain Rust daemon with no FFI round-trip.
/// `XPCBridge`'s directory-creation call (`hydrateDestinationURL(for:)`) sets
/// the SAME exclusion via the Foundation resource-key API as a second,
/// redundant guarantee for whichever process creates the directory first.
/// Recursive by macOS's own backup semantics — an excluded directory's entire
/// contents are skipped — so this only needs to run once, on the directory
/// itself, not per hydrated file.
///
/// Set on the held descriptor (`fsetxattr`), never by path, so it cannot be
/// redirected through a symlink.
#[cfg(target_os = "macos")]
pub(super) fn macos_exclude_from_backups(dir: &StagingDir) -> std::io::Result<()> {
    use std::os::fd::AsRawFd;

    let attr_name = std::ffi::CString::new("com.apple.metadata:com_apple_backup_excludeItem")
        .expect("static attribute name has no interior NUL");
    let value = b"com.apple.backupd";
    // SAFETY: the descriptor is open for the call; `attr_name` is
    // NUL-terminated and lives for the call; `value` is a plain byte slice we
    // own and pass with its exact length.
    let rc = unsafe {
        libc::fsetxattr(
            dir.fd.as_raw_fd(),
            attr_name.as_ptr(),
            value.as_ptr() as *const libc::c_void,
            value.len(),
            0,
            0,
        )
    };
    if rc != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// Task 1670 round 3: `true` for a hydrate-cache entry that
/// `write_hydrated_plaintext` (`engine_bridge.rs`) is still (or was, if it
/// crashed) mid-writing — its per-call temp name, `.{leaf}.{uuid}.part`
/// (dot-prefixed, `.part`-suffixed), created BEFORE any bytes are written and
/// published to the real, final leaf name only by one atomic `renameat` at
/// the very end. Used by [`macos_sweep_stale_hydrate_cache_entries`] to skip
/// these outright rather than age-checking them: this function's caller only
/// ever needs to look at the mtime of a FINAL (already-renamed) name.
#[cfg(target_os = "macos")]
pub(super) fn macos_is_hydrate_cache_temp_name(name: &std::ffi::OsStr) -> bool {
    match name.to_str() {
        Some(s) => s.starts_with('.') && s.ends_with(".part"),
        None => false,
    }
}

/// `getpwuid_r(getuid())` → `pw_dir`, as a `PathBuf`. `None` on any failure
/// (lookup error, null result, non-UTF8 or empty `pw_dir`) so the caller can
/// fall back. Split out as a pure(ish) helper so the mutation test in `mod
/// tests` can compute the expected real-home path directly, independent of
/// `$HOME`.
#[cfg(target_os = "macos")]
pub(super) fn getpwuid_home_dir() -> Option<std::path::PathBuf> {
    use std::ffi::CStr;

    // SAFETY: `pwd` is a plain-old-data struct the libc call fills in place;
    // `buf` backs any string fields `pwd` points into for the duration of
    // this call, and we only read through `pwd.pw_dir` after checking `rc`
    // and `result` for success. `getuid()` never fails.
    let uid = unsafe { libc::getuid() };
    let mut pwd: libc::passwd = unsafe { std::mem::zeroed() };
    let mut buf: Vec<libc::c_char> = vec![0; 4096];
    let mut result: *mut libc::passwd = std::ptr::null_mut();

    let rc = unsafe { libc::getpwuid_r(uid, &mut pwd, buf.as_mut_ptr(), buf.len(), &mut result) };

    if rc != 0 || result.is_null() || pwd.pw_dir.is_null() {
        return None;
    }
    // SAFETY: `pwd.pw_dir` is non-null (checked above) and, on success,
    // points at a NUL-terminated string owned by `buf`, which is still
    // alive here.
    let s = unsafe { CStr::from_ptr(pwd.pw_dir) }.to_str().ok()?;
    if s.is_empty() {
        return None;
    }
    Some(std::path::PathBuf::from(s))
}
