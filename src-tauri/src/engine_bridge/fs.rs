//! `fs` items split out of `engine_bridge.rs`.

use super::*;

/// Name of the per-sync-root state directory (mirrors `runner::STATE_DIR`).
pub(super) const SYNC_STATE_DIR: &str = ".beebeeb";

/// Name of the cross-process lock file the engine writes at the sync root.
pub(super) const SYNC_LOCK_FILE: &str = ".beebeeb-sync.lock";

/// True if `path` is something the engine itself writes (so it must never be
/// fed back as a user upload): the `<sync_root>/.beebeeb-sync.lock`, anything
/// inside `<sync_root>/.beebeeb/`, or any path component named `.beebeeb`
/// (defense in depth for a nested-root layout).
///
/// Shared by [`EngineBridge::classify_local_path`] (filter 1) so the retired
/// `notify` path and the Windows Cloud Files NOTIFY callbacks run ONE identical
/// engine-internal check.
pub(crate) fn path_is_engine_internal(sync_root: &Path, path: &Path) -> bool {
    let state_dir = sync_root.join(SYNC_STATE_DIR);
    let lock = sync_root.join(SYNC_LOCK_FILE);
    if path == lock || path.starts_with(&state_dir) {
        return true;
    }
    path.components()
        .any(|c| matches!(c.as_os_str().to_str(), Some(SYNC_STATE_DIR)))
}

/// Map an absolute on-disk path under `sync_root` to the server-relative,
/// '/'-separated, leading-slash-free key the state DB stores in `files.path`
/// (the same shape [`crate::state_db::StateDb::get_file_by_path`] expects, and
/// the same the nested-enumeration sweep writes). Returns `None` if `path` is
/// not under `sync_root` or resolves to the empty (root) key.
///
/// Shared by [`EngineBridge::classify_local_path`] and
/// [`EngineBridge::resolve_parent_id_for`] so the DB lookup key is computed in
/// exactly one place.
pub(crate) fn relative_db_path(sync_root: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(sync_root).ok()?;
    let mut out = String::new();
    for (i, comp) in rel.components().enumerate() {
        let part = comp.as_os_str().to_str()?;
        if i > 0 {
            out.push('/');
        }
        out.push_str(part);
    }
    if out.is_empty() { None } else { Some(out) }
}

/// Task 2229: what [`EngineBridge::classify_local_path_verdict`] makes of a local file.
pub(crate) enum LocalPathVerdict {
    /// An upload to queue: a new file, or a change of a synced one.
    Write(FinderWriteTarget),
    /// A tracked file that needs nothing while its row and its bytes stay as they are: a folder
    /// row, a row not `Local`, bytes that are the agreed content, or a change held until its row
    /// records the agreed content or a base (task 2191). The scan may skip it until either changes.
    Settled,
    /// Nothing this pass; ask again next pass.
    Skip,
}

/// Task 2271: what [`EngineBridge::row_judging_bytes_at`] found at a path.
pub(super) enum RowAtPath {
    None,
    One(FileEntry),
    /// Several rows record content this PC agreed on, and the bytes are none of them (or more than one): the ids of
    /// those rows, and how many rows share the path.
    Ambiguous { file_ids: Vec<String>, rows: usize },
    /// Windows: several rows, and choosing would read bytes that are not wholly on this PC.
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    NotWhollyOnDisk,
}

/// Open `dir` as a directory fd, refusing to follow a symlink at its final
/// component. Used only for the trusted allowed-root itself.
#[cfg(unix)]
pub(super) fn open_dir_no_follow(dir: &Path) -> std::io::Result<std::os::unix::io::OwnedFd> {
    use std::os::unix::io::FromRawFd;
    let c = path_to_cstring(dir.as_os_str())?;
    let raw = unsafe { libc::open(c.as_ptr(), libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC) };
    if raw < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { std::os::unix::io::OwnedFd::from_raw_fd(raw) })
}

/// Open a single child directory `component` relative to `dir_fd`, refusing to
/// follow a symlink. This is the anchored descent step: because the open is
/// relative to an already-open dir fd, a rename of `component` racing this call
/// cannot redirect it outside the subtree, and once returned the fd tracks that
/// exact inode regardless of later renames. `component` must be a single normal
/// name (callers pass `Component::Normal` only); reject the obvious escapes as
/// belt-and-braces.
#[cfg(unix)]
pub(super) fn open_dir_relative_no_follow(
    dir_fd: std::os::unix::io::RawFd,
    component: &std::ffi::OsStr,
) -> std::io::Result<std::os::unix::io::OwnedFd> {
    use std::os::unix::io::FromRawFd;
    if component.is_empty() || component == std::ffi::OsStr::new(".") || component == std::ffi::OsStr::new("..") {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "unsafe path component in hydrate destination",
        ));
    }
    let c = path_to_cstring(component)?;
    let raw = unsafe {
        libc::openat(
            dir_fd,
            c.as_ptr(),
            libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if raw < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { std::os::unix::io::OwnedFd::from_raw_fd(raw) })
}

/// Create/open the leaf file `leaf` relative to the anchored parent `dir_fd`
/// (O_NOFOLLOW|O_CREAT|O_WRONLY|O_TRUNC). Refuses a symlink at the leaf.
#[cfg(unix)]
pub(super) fn create_leaf_relative(
    dir_fd: std::os::unix::io::RawFd,
    leaf: &std::ffi::OsStr,
) -> std::io::Result<std::os::unix::io::OwnedFd> {
    use std::os::unix::io::FromRawFd;
    let c = path_to_cstring(leaf)?;
    let raw = unsafe {
        libc::openat(
            dir_fd,
            c.as_ptr(),
            libc::O_NOFOLLOW | libc::O_CREAT | libc::O_WRONLY | libc::O_TRUNC | libc::O_CLOEXEC,
            0o600 as libc::c_int,
        )
    };
    if raw < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { std::os::unix::io::OwnedFd::from_raw_fd(raw) })
}

/// Convert an `OsStr` path component to a NUL-terminated `CString` for the raw
/// `libc::open`/`openat` calls in `write_hydrated_plaintext`.
#[cfg(unix)]
pub(super) fn path_to_cstring(s: &std::ffi::OsStr) -> std::io::Result<std::ffi::CString> {
    use std::os::unix::ffi::OsStrExt;
    std::ffi::CString::new(s.as_bytes())
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "path contains an interior NUL byte"))
}

/// Create (if needed) and harden the staging root: owner-only `0700`, excluded from backups,
/// a real directory this user owns, never a symlink (the App Group staging folders' hardening).
#[cfg(target_os = "macos")]
pub(super) fn prepare_private_staging_root(root: &Path) -> std::io::Result<()> {
    crate::ipc_socket::macos_ensure_private_staging_dir(root)
}

/// Test builds off macOS: the same `0700` folder (there is no backup exclusion to set).
#[cfg(all(unix, test, not(target_os = "macos")))]
pub(super) fn prepare_private_staging_root(root: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(root)?;
    std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o700))
}

/// Test builds on Windows: the folder only.
#[cfg(all(windows, test))]
pub(super) fn prepare_private_staging_root(root: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(root)
}

pub(super) fn verify_staging_root_writable(root: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(root)?;
    let probe = root.join(format!(".beebeeb-write-test-{}", uuid::Uuid::new_v4()));
    std::fs::write(&probe, b"")?;
    let _ = std::fs::remove_file(probe);
    Ok(())
}

pub(super) fn display_name_for_path(path: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(path)
        .to_string()
}

/// Resolve the plaintext relative path for a server file row.
///
/// This is an **E2EE** product: the `GET /api/v1/files` listing returns the
/// filename only as the encrypted `name_encrypted` blob — there is NO
/// plaintext `path`/`display_path`/`name` field on the wire (confirmed in
/// `server/.../routes/files.rs::list_files`). The desktop must therefore
/// decrypt the name itself, with the unlocked master key, before it can
/// place a placeholder or compute a hydration destination.
///
/// Decryption goes through the **shared core primitive**
/// [`beebeeb_core::encrypt::decrypt_name`] — the exact same path the CLI
/// (`repos/cli/src/crypto::decrypt_name`) and the desktop SelectiveSync page
/// ([`crate::try_decrypt_name`]) use, so every client decrypts identically.
/// The per-file key is HKDF-derived in core (`derive_file_key(master_key,
/// file_id)`); the `MasterKey` zeroizes on drop. We never log the plaintext
/// name here.
///
/// `sync_tick` lists only the vault root (`list_files(None)` is one level —
/// the server endpoint is non-recursive), so a decrypted name IS the file's
/// relative path under the sync root. When nested traversal is added, this is
/// the single place that must compose `<parent rel path>/<name>`.
///
/// Fallback order (keeps legacy/test rows that DO carry a plaintext name
/// working, and degrades safely if a blob is missing/garbled):
///   1. decrypt `name_encrypted` (the canonical zero-knowledge path)
///   2. a plaintext `path`/`display_path`/`name` field if the server ever
///      surfaces one (older rows / test fixtures)
///   3. empty string — caller skips placeholder seeding for the row
///
/// Task 2413: what it returns is never a server field unchecked. The name that
/// resolves (decrypted and trimmed, as before, or the first plaintext field
/// present, as it is) must be ONE name this host can hold
/// ([`crate::stored_path::check_name_for`]); otherwise `Err` and the caller
/// refuses the row. Nothing resolving is `Ok("")`, as before.
pub(super) fn resolve_relative_path(
    f: &serde_json::Value,
    file_id: &str,
    master_key: &[u8; 32],
) -> Result<String, beebeeb_core::path_safety::UnsafeName> {
    let one_name = |name: &str| -> Result<String, beebeeb_core::path_safety::UnsafeName> {
        if name.is_empty() {
            return Ok(String::new());
        }
        crate::stored_path::check_name(name)?;
        Ok(name.to_string())
    };
    if let Some(name_enc) = f["name_encrypted"].as_str() {
        let mk_bytes: [u8; 32] = *master_key;
        let mk = beebeeb_core::kdf::MasterKey::from_bytes(mk_bytes);
        if let Ok(name) = beebeeb_core::encrypt::decrypt_name(&mk, file_id, name_enc) {
            // Trimmed as before, so a row stored under the trimmed name keeps its path.
            let name = name.trim();
            if !name.is_empty() {
                return one_name(name);
            }
        }
        // Decryption failed (wrong key, garbled blob, legacy envelope) —
        // fall through to any plaintext field rather than abort the sweep.
    }
    one_name(
        f["path"]
            .as_str()
            .or_else(|| f["display_path"].as_str())
            .or_else(|| f["name"].as_str())
            .unwrap_or(""),
    )
}

/// Task 1640 (review N5): move `from` to `to`, never over an existing file (`AlreadyExists`).
/// Windows: `MoveFileExW` without `MOVEFILE_REPLACE_EXISTING` (a hard link may be refused inside a
/// Cloud Files sync root). Elsewhere: a hard link, then the old name is removed.
#[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
pub(crate) fn move_no_replace(from: &Path, to: &Path) -> std::io::Result<()> {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows::Win32::Storage::FileSystem::{MOVEFILE_WRITE_THROUGH, MoveFileExW};
        let wide = |path: &Path| path.as_os_str().encode_wide().chain(Some(0)).collect::<Vec<u16>>();
        let (from_wide, to_wide) = (wide(from), wide(to));
        // SAFETY: both wide strings are NUL-terminated and alive for the call.
        unsafe {
            MoveFileExW(
                windows::core::PCWSTR(from_wide.as_ptr()),
                windows::core::PCWSTR(to_wide.as_ptr()),
                MOVEFILE_WRITE_THROUGH,
            )
        }
        // An HRESULT from a Win32 error carries the code in its low 16 bits.
        .map_err(|e| std::io::Error::from_raw_os_error(e.code().0 & 0xFFFF))
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::fs::hard_link(from, to)?;
        remove_after_link(from, to)
    }
}

/// The second step of a move by hard link: remove the old name `from` (`to` is its new link). Task 2407: when that
/// fails, the new link `to` is removed again and the error returned, so a failed move leaves one name, never two.
#[cfg_attr(target_os = "windows", allow(dead_code))]
pub(crate) fn remove_after_link(from: &Path, to: &Path) -> std::io::Result<()> {
    #[cfg(test)]
    let removed = if FAIL_REMOVE_AFTER_LINK.with(std::cell::Cell::get) {
        Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied))
    } else {
        std::fs::remove_file(from)
    };
    #[cfg(not(test))]
    let removed = std::fs::remove_file(from);
    removed.inspect_err(|_| {
        let _ = std::fs::remove_file(to);
    })
}

#[cfg(test)]
thread_local! {
    /// Tests only: the removal after a hard link fails on this thread (an immutable file, a sticky folder).
    pub(crate) static FAIL_REMOVE_AFTER_LINK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// `path` relative to `sync_root`, '/'-joined.
#[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
pub(super) fn rel_under(sync_root: &Path, path: &Path) -> String {
    path.strip_prefix(sync_root)
        .map(|rel| rel.to_string_lossy().replace('\\', "/"))
        .unwrap_or_default()
}

/// Where an op puts a row, by its parent ([`parent_rel_path_by_id`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ParentPath {
    /// Under this stored path ("" at the top level).
    At(String),
    /// Under a folder the store refused and does not hold (task 2413): the row is not placed either, and is recorded
    /// as refused (`StateDb::note_refused_server_row`) so the next op under it asks for nothing.
    Refused,
    /// Under a folder this device does not hold yet: a re-snapshot is requested, which places the row under it.
    Unknown,
}

/// Resolve a parent's FULL relative path from the local mirror by its `file_id`
/// — the parent path a freshly-created/moved child must be prefixed with. This
/// mirrors how [`apply_snapshot`] resolves nesting (it threads each parent's
/// resolved full path into its children), so an op-driven create/move lands at
/// the SAME path a fresh snapshot would produce.
///
///   * `parent_id == None`            → `At("")` (root child),
///   * parent row present locally     → `At` the parent's stored full `path` (its
///     leading slash trimmed so it composes cleanly as a prefix),
///   * a parent the store refused and does not hold (task 2413) → `Refused`: the
///     row is not placed either, and asks for no re-snapshot (one would refuse
///     the parent again),
///   * any other parent NOT in the local mirror yet (the create/move arrived
///     before its parent's create op landed) → `Unknown`, and a re-snapshot is
///     requested: the caller applies nothing now, and the authoritative snapshot
///     places the row under its parent on the next tick. Task 2413: before, the
///     row went to the top level for this tick.
pub(super) fn parent_rel_path_by_id(bridge: &EngineBridge, parent_id: Option<&str>) -> ParentPath {
    let Some(pid) = parent_id.filter(|p| !p.is_empty()) else {
        return ParentPath::At(String::new()); // root child — no prefix
    };
    match bridge.db().get_file(pid).ok().flatten() {
        Some(parent) => ParentPath::At(parent.path.trim_start_matches('/').to_string()),
        // Task 2413: a re-snapshot would refuse the parent again; the parent's own write asks for one once it is
        // stored.
        None if bridge.db().is_refused_server_row(pid).unwrap_or(false) => {
            tracing::debug!(parent_id = %pid, "sync_tick: op under a folder not placed; not placed either");
            ParentPath::Refused
        }
        None => {
            // Parent not locally known — can't compute the prefix from the op
            // alone. Schedule a re-snapshot so the next tick reconciles nesting
            // authoritatively; a best-effort log keeps this observable.
            if let Err(e) = bridge.db().request_resnapshot() {
                tracing::warn!(error = %e, "sync_tick: could not request re-snapshot for unknown parent");
            }
            tracing::info!(
                parent_id = %pid,
                "sync_tick: op references a parent not in the local mirror; scheduling re-snapshot"
            );
            ParentPath::Unknown
        }
    }
}

/// The PARENT relative path of an existing local row (everything before the last
/// `/` in its stored `path`), or "" when the row is unknown / at the vault root.
/// Used so an op-driven re-ingest keeps a nested row under its folder prefix
/// without needing the parent's plaintext name in the op.
pub(super) fn existing_parent_rel_path(bridge: &EngineBridge, id: &str) -> String {
    bridge
        .db()
        .get_file(id)
        .ok()
        .flatten()
        .and_then(|e| {
            let p = e.path.trim_start_matches('/');
            p.rsplit_once('/').map(|(parent, _leaf)| parent.to_string())
        })
        .unwrap_or_default()
}
