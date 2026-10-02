//! Unix-domain-socket IPC between the desktop daemon and the macOS File
//! Provider extension (and the Linux FUSE mount). Unix sockets do not
//! exist on Windows, where the Cloud Files callback runs in-process
//! (see `crate::windows_cf`), so the whole module is gated to `unix`.

#![cfg(unix)]

use serde::{Deserialize, Serialize};
use crate::ipc_frame::{FrameError, FrameReader, MAX_REQUEST_BYTES, write_frame};
use crate::ipc_write_dedup::{MAX_KEY_BYTES, WriteDedup};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

#[derive(Debug, Serialize, Deserialize)]
pub enum IpcRequest {
    GetFileStatus {
        file_id: String,
    },
    SetFileStatus {
        file_id: String,
        status: String,
    },
    /// Task 1697: the replica enumerator's change feed. `since_anchor` is the
    /// opaque sync-anchor cursor from a previous `FileProviderChanges` reply
    /// (or absent at genesis). Paged: at most 100 changes per reply, with
    /// `next_anchor` doubling as the resume token while more remain.
    ListChanges {
        /// Opaque cursor bytes (decimal ASCII of the change-log rowid).
        /// Absent = enumerate from the beginning of retained history.
        #[serde(default)]
        since_anchor: Option<String>,
        /// Requested page size; clamped to `[1, 100]`.
        #[serde(default)]
        limit: Option<u32>,
    },
    /// Task 1697: the replica's `currentSyncAnchor` — served from the
    /// daemon's persistent cursor so it survives extension process death.
    GetSyncAnchor,
    /// Task 1697: publish the materialized container set (from the
    /// system's materialized-items callbacks) so the daemon can filter
    /// working-set signals to materialized parents.
    ReportMaterialized {
        container_ids: Vec<String>,
    },
    HydrateFile {
        file_id: String,
        dest_path: String,
        /// Opt in to `HydrateProgress` frames before the final reply (task
        /// 1670 issue 3). Absent/false for old extensions, which read exactly
        /// one reply and would mistake the first progress frame for it.
        #[serde(default)]
        progress: bool,
    },
    ListFileProviderItems {
        container_id: String,
    },
    QueueFinderCreate {
        parent_id: Option<String>,
        filename: String,
        kind: String,
        contents_path: Option<String>,
        content_type: Option<String>,
        /// Idempotency key (task 1684), stable across the system's retries of
        /// the same logical `createItem`. Absent from an old extension, which
        /// then gets exactly the pre-1684 behaviour. See `crate::ipc_write_dedup`
        /// and docs/IPC_PROTOCOL.md.
        #[serde(default)]
        request_id: Option<String>,
    },
    QueueFinderModify {
        file_id: String,
        parent_id: Option<String>,
        filename: String,
        kind: String,
        contents_path: Option<String>,
        content_type: Option<String>,
        base_version_identifier: Option<String>,
        /// Idempotency key (task 1684); see `QueueFinderCreate::request_id`.
        #[serde(default)]
        request_id: Option<String>,
    },
    QueueFinderDelete {
        file_id: String,
        base_version_identifier: Option<String>,
    },
    SetRecursivePin {
        file_id: String,
        pinned: bool,
    },
    RecordOpenedFile {
        file_id: String,
        cache_path: String,
        cache_bytes: i64,
    },
    EnforceSmartCache {
        max_unpinned_cache_bytes: Option<i64>,
        disk_pressure_min_free_bytes: Option<u64>,
    },
    GetSyncSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IpcResponse {
    FileStatus(FileProviderItemPayload),
    FileProviderItems {
        items: Vec<FileProviderItemPayload>,
    },
    /// Success with no payload. Deliberately an (empty) object on the wire —
    /// `{"Ok":{}}` — not the bare string `"Ok"` a unit variant serialises to:
    /// JSONSerialization on the Swift side rejects a top-level string unless
    /// fragments are allowed, which is what made every successful Finder
    /// hydrate surface as "daemon response was not valid JSON" (task 1670
    /// issue 3). The Swift reader still accepts the legacy bare string.
    Ok {},
    /// Sent only in answer to `HydrateFile { progress: true }`, zero or more
    /// times before the final reply. `total` is the plaintext size in bytes
    /// (0 when unknown); `done` is plaintext bytes decrypted so far.
    HydrateProgress {
        done: u64,
        total: u64,
    },
    Error {
        message: String,
    },
    WriteQueued {
        item: Option<FileProviderItemPayload>,
        ignored: bool,
        message: String,
    },
    SyncSummary {
        syncing: u32,
        cloud_only: u32,
        conflicts: u32,
    },
    /// Task 1697: one page of the change log. `next_anchor` is present while
    /// more changes remain or after a delivered batch (the resume token /
    /// new anchor); `None` only when the log is empty.
    FileProviderChanges {
        changes: Vec<FileProviderChangePayload>,
        next_anchor: Option<String>,
    },
    /// Task 1697: `currentSyncAnchor` is served from the daemon's persistent
    /// cursor so it survives extension process death.
    FileProviderSyncAnchor {
        anchor: Option<String>,
    },
    PinUpdated {
        changed_item_ids: Vec<String>,
        hydrate_operations: usize,
    },
    CacheCleanup {
        evicted_file_ids: Vec<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileProviderItemPayload {
    pub identifier: String,
    pub parent_identifier: String,
    pub filename: String,
    pub kind: String,
    pub size_bytes: i64,
    pub content_type: Option<String>,
    pub status: String,
    pub capabilities: u32,
    pub version_identifier: Option<String>,
    /// Task 1697: server item creation time, seconds since the Unix epoch.
    /// `#[serde(default)]` keeps the payload readable by the 0.8.8 Swift
    /// decoder (absent -> no date).
    #[serde(default)]
    pub created_at: Option<i64>,
    /// Task 1697: content modification time, seconds since the Unix epoch —
    /// what Finder renders as "Modified".
    #[serde(default)]
    pub modified_at: Option<i64>,
    /// Task 1697: real child count for folders (files: absent). The Swift
    /// `childItemCount` was hardcoded to 0 before this field existed.
    #[serde(default)]
    pub child_item_count: Option<i64>,
    /// Task 1697: content-version component of `itemVersion` — the value whose
    /// change means "re-download + invalidate the thumbnail cache".
    #[serde(default)]
    pub content_version: Option<String>,
    /// Task 1697: metadata-version component — mtime/size/parent/name identity.
    /// Its change alone means "refresh metadata, keep the cached content".
    #[serde(default)]
    pub metadata_version: Option<String>,
}

/// One change-log row crossing the IPC bridge (task 1697).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileProviderChangePayload {
    pub file_id: String,
    /// created | modified | deleted | reparented
    pub kind: String,
    #[serde(default)]
    pub old_parent_id: Option<String>,
    #[serde(default)]
    pub new_parent_id: Option<String>,
    /// The FULL item payload for created/modified/reparented rows (absent for
    /// deletions) so the replica's `didUpdateItems` needs no second
    /// round-trip. `#[serde(default)]` keeps the decoder tolerant.
    #[serde(default)]
    pub item: Option<FileProviderItemPayload>,
}

const FP_ROOT: &str = "__fp_root__";
const FP_ROOT_APPLE: &str = "NSFileProviderRootContainerItemIdentifier";
// Task 1701: the OTHER namespace constants (`my_files`, `offline`,
// `conflicts`) are gone with the synthetic containers. This one survives as
// a WRITE-path guard only: a stale Finder client can still drop onto a
// cached `namespace:shared_with_me` folder, and that drop must be refused
// (read-only), not silently re-parented to the vault root by
// `normalize_parent_id`. It is never an enumeration surface any more.
const NAMESPACE_SHARED_WITH_ME: &str = "namespace:shared_with_me";
const CAP_READ: u32 = 1 << 0;
const CAP_WRITE: u32 = 1 << 1;
const CAP_RENAME: u32 = 1 << 2;
const CAP_DELETE: u32 = 1 << 3;
/// `.allowsAddingSubItems` on the Swift side (task 1694). Bit 4, with the
/// SAME numbering on both sides of the XPC bridge — the payload crosses it
/// as a plain u32 (`FileProviderItemPayload.capabilities` ↔
/// `BeebeebProviderItem.capabilities`, Swift: `BeebeebProviderItem.addSubItems`).
const CAP_ADD_SUBITEMS: u32 = 1 << 4;
/// `.allowsReparenting` on the Swift side (task 1697). Bit 5, same numbering
/// both sides of the bridge — Finder refuses a drag-MOVE out of an item
/// without it.
const CAP_REPARENT: u32 = 1 << 5;
/// `.allowsTrashing` on the Swift side (task 1697). Bit 6, same numbering both
/// sides. Trash SEMANTICS (trashContainer handling) are task 1698; this bit
/// only advertises the capability so Finder stops blocking drag-to-Trash.
const CAP_TRASH: u32 = 1 << 6;

/// The macOS App Group shared between the containing app
/// (`src-tauri/entitlements.plist`) and the File Provider extension
/// (`BeebeebFileProvider/BeebeebFileProvider.entitlements`) — both declare
/// `com.apple.security.application-groups: [<this>]`. This is the ONE place
/// the id is defined on the Rust side; `BeebeebFileProvider/XPCBridge.swift`
/// mirrors the identical literal (Swift has no practical way to `include!` a
/// Rust const, and there is no existing shared-codegen step in this repo —
/// see `docs/MACOS_BRINGUP_BRIEF.md`). Keep all three in sync if it ever
/// changes.
#[cfg(target_os = "macos")]
pub const MACOS_APP_GROUP_ID: &str = "R8352WDJJR.io.beebeeb.app.fileprovider";

/// Deliberately short — see [`macos_ipc_socket_path_in`]'s doc comment.
#[cfg(target_os = "macos")]
const MACOS_IPC_SOCKET_FILENAME: &str = "ipc.sock";

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
fn macos_ipc_socket_path_in(home_dir: &std::path::Path) -> std::path::PathBuf {
    home_dir
        .join("Library")
        .join("Group Containers")
        .join(MACOS_APP_GROUP_ID)
        .join(MACOS_IPC_SOCKET_FILENAME)
}

/// Task 1524: the daemon is sandboxed on macOS (`com.apple.security.app-
/// sandbox`), so `/tmp` and `$XDG_RUNTIME_DIR` (which macOS never sets in the
/// first place — that's a Linux/systemd convention) are NOT reachable; only
/// the app's own container and any shared App Group container are. Binding
/// outside those fails the sandbox's file-access check, which a bare
/// `.expect()` on the bind turned into a panicked fire-and-forget task and,
/// from the user's side, an unconditional 3-second timeout on "Install
/// Finder location" (`Timed out waiting for the local Beebeeb sync daemon…`)
/// with no indication of the real cause.
#[cfg(target_os = "macos")]
pub fn ipc_socket_path() -> std::path::PathBuf {
    macos_ipc_socket_path_in(&macos_real_home_dir())
}

/// Task 1670: subdirectory (of the same shared App Group container as
/// `ipc.sock` above) that stages a freshly-hydrated file's decrypted plaintext
/// before the File Provider extension hands its URL to Finder.
///
/// Deliberately a DIFFERENT name than the socket file, in the SAME directory —
/// nothing about that collides with `MACOS_IPC_SOCKET_FILENAME`.
#[cfg(target_os = "macos")]
const MACOS_HYDRATE_CACHE_DIRNAME: &str = "hydrate-cache";

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
fn macos_hydrate_cache_dir_in(home_dir: &std::path::Path) -> std::path::PathBuf {
    home_dir
        .join("Library")
        .join("Group Containers")
        .join(MACOS_APP_GROUP_ID)
        .join(MACOS_HYDRATE_CACHE_DIRNAME)
}

#[cfg(target_os = "macos")]
pub fn macos_hydrate_cache_dir() -> std::path::PathBuf {
    macos_hydrate_cache_dir_in(&macos_real_home_dir())
}

/// Task 1670 round 3 (lead review of round 2): Apple's own `fetchContents`
/// docs say only "After you call the completion handler, the system takes
/// complete control over the local copy" and that the system "can clone it"
/// — never that the clone happens SYNCHRONOUSLY, inside the
/// `completionHandler` call itself. Round 2 deleted the staged file
/// unconditionally right after a successful `completionHandler`, which is
/// only safe if that clone is synchronous; since the docs don't say either
/// way, round 3 made `FileProviderExtension.fetchContents` (Swift) stop
/// deleting on the success path, and made THIS TTL/sweep the primary bound
/// on plaintext lifetime instead of a crash-only backstop.
///
/// **Round 4 (Codex P1 on PR #75, review thread PRRT_kwDOSLX6Xs6ncqQ7): round
/// 3's fix was still not enough** — it handed OUR staging URL straight to
/// `completionHandler` and relied on THIS TTL to eventually delete it, and
/// Codex's review made the sharper point: once `completionHandler` is
/// called, "the File Provider contract transfers control of that local copy
/// to the system; there is no documented maximum delay before the system
/// finishes consuming it" — deleting that SAME handed-off URL later, on ANY
/// timer, can still race a busy or suspended `fileproviderd`. So
/// `fetchContents` no longer hands this staging URL to the system AT ALL: it
/// copies the file into `NSFileProviderManager(for:).temporaryDirectoryURL()`
/// (a directory this daemon never touches) and deletes OUR copy immediately
/// after that copy succeeds — see `FileProviderExtension
/// .copyToSystemTemporaryDirectory(stagedAt:)`'s doc comment for the full
/// mechanism.
///
/// That makes this TTL/sweep a **crash backstop, not the primary
/// mechanism**: with round 4's fix, the ONLY thing that can be left behind
/// in this directory past the per-request delete is staging orphaned by a
/// crash between the decrypt and the copy-then-delete (a force-quit
/// mid-fetch, the extension being killed) — never a file the system is
/// still relying on, because we now never hand a file in THIS directory to
/// the system in the first place.
///
/// **Round 5 (Codex P1 on PR #75, review thread PRRT_kwDOSLX6Xs6ndhN1):
/// round 4's "never a file the system is still relying on" claim had a real
/// gap.** This TTL/sweep is purely mtime-based and has no cross-process
/// signal for "the extension is still mid-copy" — Codex's finding: if the
/// extension is suspended by the system between `ipc.hydrateFile` finishing
/// and `copyToSystemTemporaryDirectory` actually running `copyItem`, or if
/// `copyItem` itself takes longer than this TTL for a very large file, the
/// sweep can delete a file the extension is STILL actively copying,
/// reopening this task's original bug. Fixed on the Swift side —
/// `copyToSystemTemporaryDirectory` (`FileProviderExtension.swift`) now
/// touches the source file's mtime immediately before starting the copy, so
/// a slow `copyItem` (Codex's second trigger, now fully covered regardless
/// of file size) always gets the FULL TTL below as headroom. That does NOT
/// theoretically close the first trigger (suspension landing in the much
/// narrower window before that touch runs) — see that function's doc
/// comment for the honest accounting of what remains open and why closing
/// it fully would need a lease/heartbeat protocol, out of scope here.
///
/// Raised from 2 to **10 minutes** this round as a second, independent
/// mitigation for that residual window: still a crash-only backstop
/// (sign-out/lock/daemon-startup purge is the real bound on ordinary
/// plaintext exposure, unchanged since round 2), but with an order of
/// magnitude more headroom against any realistic suspension duration this
/// Mac might impose on a File Provider extension process, at negligible
/// security cost — this directory is owner-only (`0700`) and backup-excluded
/// regardless of TTL.
///
/// Swept periodically (`MACOS_HYDRATE_SWEEP_EVERY_N_TICKS`, `runner.rs`,
/// every 60s on the daemon's existing tick loop — not a new timer/thread)
/// AND opportunistically on every real hydration (the `HydrateFile` handler
/// below, catches anything the next periodic tick hasn't reached yet in a
/// hydration-heavy session).
#[cfg(target_os = "macos")]
pub(crate) const MACOS_HYDRATE_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(10 * 60);

/// Task 1670 round 2: reject a raw File Provider item identifier BEFORE it is
/// used as a path component — either here or in its Swift mirror,
/// `XPCBridge.sanitizedHydrateFilename(for:)`
/// (`BeebeebFileProvider/XPCBridge.swift`, doc comment there cross-references
/// this function; this repo's File Provider extension target has no XCTest
/// target, so THIS copy — not the Swift one — carries the tested contract:
/// keep both in sync).
///
/// In normal operation `identifier` is always one of OUR OWN generated file
/// ids (a UUID) — never attacker input in the traditional sense — but
/// `fetchContents` receives it from the File Provider framework as an opaque
/// string and both `XPCBridge.hydrateDestinationURL(for:)` (Swift) and the
/// `HydrateFile` IPC handler below (Rust) treat it as (part of) a path
/// component. Defense in depth: refuse anything that could turn a hydrate
/// destination into a path outside the hydrate-cache directory — a literal
/// `/` or `\` (also covers a leading-`/` absolute path, called out separately
/// in the task brief), an embedded `..` traversal segment, an empty string,
/// or an embedded NUL.
#[cfg(target_os = "macos")]
pub(crate) fn macos_validate_hydrate_item_identifier(identifier: &str) -> Result<(), &'static str> {
    if identifier.is_empty() {
        return Err("hydrate item identifier must not be empty");
    }
    if identifier.contains('/') || identifier.contains('\\') {
        return Err("hydrate item identifier must not contain a path separator");
    }
    if identifier.contains("..") {
        return Err("hydrate item identifier must not contain '..'");
    }
    if identifier.contains('\0') {
        return Err("hydrate item identifier must not contain a NUL byte");
    }
    Ok(())
}

/// Create (if needed) and harden the macOS hydrate-cache directory: owner-only
/// `0o700` (task 1670 round 2 — it only ever holds decrypted plaintext) and
/// excluded from Time Machine / iCloud backups (see
/// [`macos_exclude_from_backups`]). Idempotent: safe to call on every
/// hydration, not just the first.
#[cfg(target_os = "macos")]
pub(crate) fn macos_ensure_hydrate_cache_dir(dir: &std::path::Path) -> std::io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};

    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    // Belt-and-braces: `DirBuilder::mode` is subject to `mkdir`'s normal
    // umask handling like any other creation call, and a directory left over
    // from an older build (before this fix) may already exist with looser
    // permissions. Force it down explicitly rather than trusting creation
    // alone.
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    if let Err(e) = macos_exclude_from_backups(dir) {
        tracing::warn!(error = %e, dir = %dir.display(), "could not exclude macOS hydrate-cache dir from backups");
    }
    Ok(())
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
#[cfg(target_os = "macos")]
fn macos_exclude_from_backups(dir: &std::path::Path) -> std::io::Result<()> {
    use std::os::unix::ffi::OsStrExt;

    let path_c = std::ffi::CString::new(dir.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "path contains a NUL byte"))?;
    let attr_name = std::ffi::CString::new("com.apple.metadata:com_apple_backup_excludeItem")
        .expect("static attribute name has no interior NUL");
    let value = b"com.apple.backupd";
    // SAFETY: `path_c`/`attr_name` are NUL-terminated and live for the call;
    // `value` is a plain byte slice we own and pass with its exact length.
    let rc = unsafe {
        libc::setxattr(
            path_c.as_ptr(),
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

/// Delete every entry directly inside `dir` (files and any stray
/// subdirectories), leaving `dir` itself in place. Used for the sign-out /
/// lock / daemon-startup hydrate-cache purge (task 1670 round 2): this
/// directory only ever holds short-lived staged plaintext, so wiping it on
/// every account-security-boundary event bounds how long a decrypted copy
/// can survive on disk even across a crash that skipped normal per-request
/// cleanup.
///
/// Best-effort per entry: one un-removable entry (a permissions race, or held
/// open by another process) is logged by the caller and does not stop the
/// rest from being purged. A missing `dir` (nothing hydrated yet this run) is
/// not an error — returns `Ok(0)`.
#[cfg(target_os = "macos")]
pub(crate) fn macos_purge_hydrate_cache_dir(dir: &std::path::Path) -> std::io::Result<usize> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e),
    };
    let mut removed = 0usize;
    for entry in entries.flatten() {
        let path = entry.path();
        let result = if path.is_dir() {
            std::fs::remove_dir_all(&path)
        } else {
            std::fs::remove_file(&path)
        };
        match result {
            Ok(()) => removed += 1,
            Err(e) => tracing::warn!(path = %path.display(), error = %e, "hydrate-cache purge: could not remove entry"),
        }
    }
    Ok(removed)
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
fn macos_is_hydrate_cache_temp_name(name: &std::ffi::OsStr) -> bool {
    match name.to_str() {
        Some(s) => s.starts_with('.') && s.ends_with(".part"),
        None => false,
    }
}

/// Remove hydrate-cache entries whose mtime is at least `ttl` old, relative
/// to `now`. `now`/`ttl` are parameters (never `SystemTime::now()` read
/// inline) so this is deterministic and testable without a real clock or
/// real sleeps.
///
/// Task 1670 round 3: never touches an entry that is still being written.
/// Skips anything matching [`macos_is_hydrate_cache_temp_name`] before even
/// reading its mtime — not "old temp files are probably safe to age-check
/// too", an outright skip, so a sweep landing between a temp file's creation
/// and its publishing `renameat` (`write_hydrated_plaintext`,
/// `engine_bridge.rs`) can never remove or race a partially-written file.
#[cfg(target_os = "macos")]
pub(crate) fn macos_sweep_stale_hydrate_cache_entries(
    dir: &std::path::Path,
    ttl: std::time::Duration,
    now: std::time::SystemTime,
) -> std::io::Result<usize> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e),
    };
    let mut removed = 0usize;
    for entry in entries.flatten() {
        let path = entry.path();
        if path
            .file_name()
            .map(macos_is_hydrate_cache_temp_name)
            .unwrap_or(false)
        {
            continue;
        }
        let age = std::fs::metadata(&path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|modified| now.duration_since(modified).ok());
        let Some(age) = age else { continue };
        if age < ttl {
            continue;
        }
        let result = if path.is_dir() {
            std::fs::remove_dir_all(&path)
        } else {
            std::fs::remove_file(&path)
        };
        match result {
            Ok(()) => removed += 1,
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "hydrate-cache TTL sweep: could not remove stale entry")
            }
        }
    }
    Ok(removed)
}

/// Task 1524 follow-up: resolve the real user home directory from the OS
/// password database (`getpwuid_r(getuid())` → `pw_dir`), never from `$HOME`.
///
/// A sandboxed macOS process has `$HOME` rewritten by the OS to the app's
/// *container* directory (`~/Library/Containers/io.beebeeb.app/Data`), so
/// `dirs::home_dir()` — which is just a `$HOME` read — returned a path ~40
/// bytes longer than the real home once joined with `Library/Group
/// Containers/<group id>/ipc.sock`: 132 bytes, over the 104-byte
/// `sockaddr_un.sun_path` budget (`macos_ipc_socket_path_in`'s doc comment),
/// so the bind failed and "Install Finder location" still timed out even
/// after the first 1524 fix moved the socket into the group container.
///
/// `pw_dir` from the password database is NOT redirected by the sandbox — it
/// is the same real home the Swift side resolves via `FileManager
/// .homeDirectoryForCurrentUser` / `NSHomeDirectory()`, and the same one a
/// plain `getpwuid` reads outside any container. Falls back to
/// `dirs::home_dir()` only if the password-database lookup itself fails
/// (not expected on a real macOS install).
#[cfg(target_os = "macos")]
fn macos_real_home_dir() -> std::path::PathBuf {
    match getpwuid_home_dir() {
        Some(dir) => {
            tracing::debug!("ipc_socket_path: resolved real home via getpwuid_r (bypassing $HOME)");
            dir
        }
        None => {
            tracing::warn!(
                "ipc_socket_path: getpwuid_r lookup failed; falling back to dirs::home_dir(), \
                 which reads $HOME and is WRONG under the app sandbox"
            );
            dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
        }
    }
}

/// `getpwuid_r(getuid())` → `pw_dir`, as a `PathBuf`. `None` on any failure
/// (lookup error, null result, non-UTF8 or empty `pw_dir`) so the caller can
/// fall back. Split out as a pure(ish) helper so the mutation test in `mod
/// tests` can compute the expected real-home path directly, independent of
/// `$HOME`.
#[cfg(target_os = "macos")]
fn getpwuid_home_dir() -> Option<std::path::PathBuf> {
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

/// Linux: unchanged — `$XDG_RUNTIME_DIR` (a systemd-managed, per-user,
/// tmpfs-backed, already-private directory) with a `/tmp` fallback for
/// environments without a systemd user session. Linux desktop builds are not
/// sandboxed, so both are always reachable.
#[cfg(not(target_os = "macos"))]
pub fn ipc_socket_path() -> std::path::PathBuf {
    let runtime_dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into());
    std::path::PathBuf::from(runtime_dir).join("beebeeb-daemon.sock")
}

/// Bind and 0600-harden the daemon's Unix IPC listener at `path`. Split out
/// of `serve_ipc_at` (task 1524) so a bind failure can be tested in
/// isolation and, more importantly, so it no longer panics: this used to be
/// a bare `UnixListener::bind(&path).expect("bind IPC socket")` inside a
/// fire-and-forget `tokio::spawn`ed task, so a failure (e.g. the sandbox
/// refusing a path outside the app's container) silently killed the whole
/// IPC accept loop with no error ever reaching the caller — the readiness
/// probe just timed out with a generic message. Callers now get the real
/// `std::io::Error` back and can log/surface it.
fn bind_ipc_listener(path: &std::path::Path) -> std::io::Result<UnixListener> {
    bind_ipc_listener_with(path, |path| UnixListener::bind(path))
}

fn bind_ipc_listener_with<T>(
    path: &std::path::Path,
    bind: impl FnOnce(&std::path::Path) -> std::io::Result<T>,
) -> std::io::Result<T> {
    use std::os::unix::fs::PermissionsExt;

    // Binding at the public endpoint exposes its umask-derived mode before
    // chmod. Stage in a fresh owner-only directory instead; never change the
    // process-wide umask in this multithreaded application.
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| std::path::Path::new("."));
    let staging = tempfile::Builder::new()
        .prefix("")
        .rand_bytes(6)
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir_in(parent)?;
    // Six directory characters + "/s" = eight bytes, matching "ipc.sock":
    // staging also fits the macOS App Group sockaddr_un path budget.
    let staged_path = staging.path().join("s");
    let listener = bind(&staged_path)?;
    // Fail closed: neither chmod nor publication failure may return a listener.
    // TempDir removes the private socket/directory on every error path.
    std::fs::set_permissions(&staged_path, std::fs::Permissions::from_mode(0o600))?;
    std::fs::rename(&staged_path, path)?;
    Ok(listener)
}

pub async fn serve_ipc(
    db: std::sync::Arc<crate::state_db::StateDb>,
    bridge: std::sync::Arc<crate::engine_bridge::EngineBridge>,
    cancel: oneshot::Receiver<()>,
) -> std::io::Result<()> {
    serve_ipc_at(ipc_socket_path(), db, bridge, cancel).await
}

/// Real IPC server, bound to an explicit `path`. `serve_ipc` calls this with the
/// production `ipc_socket_path()`; tests bind it to a throwaway temp socket so
/// they can exercise the real accept/dispatch path without touching the real
/// production socket path (task 1247).
///
/// Returns `Err` if binding, hardening or publication fails (see [`bind_ipc_listener`]) —
/// never panics. A bind failure means the loop below never starts; the
/// caller is responsible for logging/surfacing the error (`runner.rs` logs
/// it and records it for `wait_for_file_provider_ipc_ready` to report
/// verbatim instead of just timing out).
pub async fn serve_ipc_at(
    path: std::path::PathBuf,
    db: std::sync::Arc<crate::state_db::StateDb>,
    bridge: std::sync::Arc<crate::engine_bridge::EngineBridge>,
    cancel: oneshot::Receiver<()>,
) -> std::io::Result<()> {
    serve_ipc_at_with_ready(path, db, bridge, cancel, None).await
}

/// Readiness means the listener is bound, hardened and published. On startup
/// failure the sender is dropped and the server returns the original error.
pub(crate) async fn serve_ipc_at_with_ready(
    path: std::path::PathBuf,
    db: std::sync::Arc<crate::state_db::StateDb>,
    bridge: std::sync::Arc<crate::engine_bridge::EngineBridge>,
    mut cancel: oneshot::Receiver<()>,
    ready: Option<oneshot::Sender<()>>,
) -> std::io::Result<()> {
    let listener = bind_ipc_listener(&path)?;
    if let Some(ready) = ready {
        let _ = ready.send(());
    }
    tracing::info!("IPC socket listening at {:?}", path);
    let mut connections: Vec<JoinHandle<()>> = Vec::new();
    // One table per daemon process, shared by every connection: a retry arrives
    // on a NEW connection (task 1684). It is deliberately in-memory only.
    let write_dedup = WriteDedup::<IpcResponse>::new();

    loop {
        connections.retain(|handle| !handle.is_finished());
        tokio::select! {
            biased;
            _ = &mut cancel => break,
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) => {
                    let db = db.clone();
                    let bridge = bridge.clone();
                    let write_dedup = write_dedup.clone();
                    connections.push(tokio::spawn(handle_connection(stream, db, bridge, write_dedup)));
                }
                Err(e) => tracing::warn!("IPC accept error: {e}"),
            }
        }
    }
    for handle in connections {
        handle.abort();
    }
    drop(listener);
    let _ = std::fs::remove_file(&path);
    tracing::info!("IPC socket stopped at {:?}", path);
    Ok(())
}

/// Read the connecting peer process's UID from a connected Unix stream.
///
/// Linux uses `SO_PEERCRED` (the kernel fills a `libc::ucred`). macOS/iOS use
/// `getpeereid()` — the sanctioned BSD API for exactly this (simpler than the
/// `LOCAL_PEERCRED` getsockopt: no struct layout to get wrong, libc fills two
/// plain `uid_t`/`gid_t` out-params). Implemented and VERIFIED on real macOS
/// hardware 2026-08-29 (Darwin 25.6, Apple Silicon): with this in place the two
/// real-socket engine_bridge IPC tests (`ipc_allows_hydrate_write_under_allowed_root`,
/// `ipc_rejects_hydrate_write_outside_allowed_roots`) go from failing-closed
/// (connection dropped, client sees EOF/BrokenPipe) to passing — closing the
/// blocking item in the macOS bring-up brief (task 1247). Platforms with
/// neither implementation still fail closed (returns `Err`, so the caller
/// rejects the connection).
#[cfg(target_os = "linux")]
fn peer_uid(stream: &UnixStream) -> std::io::Result<u32> {
    let fd = std::os::unix::io::AsRawFd::as_raw_fd(stream);
    // SAFETY: `cred` is a plain-old-data struct the kernel fills; we pass its
    // size in `len` (updated in-place) and only read `cred` after a 0 return.
    let mut cred: libc::ucred = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let rc = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            &mut cred as *mut libc::ucred as *mut libc::c_void,
            &mut len,
        )
    };
    if rc != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(cred.uid)
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
fn peer_uid(stream: &UnixStream) -> std::io::Result<u32> {
    let fd = std::os::unix::io::AsRawFd::as_raw_fd(stream);
    let mut euid: libc::uid_t = 0;
    let mut egid: libc::gid_t = 0;
    // SAFETY: getpeereid only writes the two out-params on success (rc == 0);
    // both are plain integers we own on the stack. We read them only after
    // checking rc.
    let rc = unsafe { libc::getpeereid(fd, &mut euid, &mut egid) };
    if rc != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(euid)
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "ios")))]
fn peer_uid(_stream: &UnixStream) -> std::io::Result<u32> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "peer credential check not implemented for this platform",
    ))
}

/// Pure authorization predicate, split out so the comparison logic is unit-
/// testable without a real cross-UID connection (which would need root/setuid
/// privileges no sandboxed test runner has). The daemon only serves a peer
/// running as its own OS user.
fn is_authorized_peer(daemon_uid: u32, peer_uid: u32) -> bool {
    daemon_uid == peer_uid
}

async fn handle_connection(
    stream: UnixStream,
    db: std::sync::Arc<crate::state_db::StateDb>,
    bridge: std::sync::Arc<crate::engine_bridge::EngineBridge>,
    write_dedup: std::sync::Arc<WriteDedup<IpcResponse>>,
) {
    // Authenticate the peer BEFORE reading or dispatching anything. The daemon
    // holds vault keys in memory and `hydrate_file` is a decrypt oracle, so a
    // connection from any other OS user is refused outright — no request is
    // parsed, no response is written, the connection is just dropped
    // (task 1247). `peer_uid` is implemented for Linux (SO_PEERCRED) and
    // macOS/iOS (getpeereid); any other platform fails closed.
    let daemon_uid = unsafe { libc::getuid() };
    match peer_uid(&stream) {
        Ok(uid) if is_authorized_peer(daemon_uid, uid) => {}
        Ok(uid) => {
            tracing::warn!(peer_uid = uid, daemon_uid, "IPC connection rejected: peer UID does not match daemon");
            return;
        }
        Err(e) => {
            tracing::warn!(error = %e, "IPC connection rejected: could not verify peer credentials");
            return;
        }
    }

    // Framing (task 1670 issue 3): one compact-JSON request per `\n`-terminated
    // line, exactly one delimited reply per request (hydrate may send
    // `HydrateProgress` lines first when asked). See `crate::ipc_frame` and
    // `docs/IPC_PROTOCOL.md`.
    let (read_half, mut write_half) = stream.into_split();
    let mut frames = FrameReader::new(read_half, MAX_REQUEST_BYTES);
    loop {
        let line = match frames.next_frame().await {
            Ok(Some(line)) => line,
            Ok(None) => break,
            Err(FrameError::TooLarge { limit }) => {
                tracing::warn!(limit, "IPC request exceeded the size limit; closing connection");
                let _ = write_frame(
                    &mut write_half,
                    &IpcResponse::Error {
                        message: format!("request exceeds the {limit}-byte limit"),
                    },
                )
                .await;
                break;
            }
            Err(FrameError::Io(_)) => break,
        };
        let req: IpcRequest = match serde_json::from_slice(&line) {
            Ok(r) => r,
            Err(e) => {
                // The stream is still in sync (we consumed exactly one line),
                // so answer and keep serving the connection.
                if write_frame(&mut write_half, &IpcResponse::Error { message: e.to_string() })
                    .await
                    .is_err()
                {
                    break;
                }
                continue;
            }
        };
        let resp = match req {
            IpcRequest::GetFileStatus { file_id } => match db.get_file(&file_id) {
                Ok(Some(e)) => IpcResponse::FileStatus(file_entry_payload_for_db(&db, &e, FP_ROOT_APPLE)),
                _ => IpcResponse::Error {
                    message: "not found".into(),
                },
            },
            IpcRequest::ListFileProviderItems { container_id } => IpcResponse::FileProviderItems {
                items: list_file_provider_items(&db, &container_id),
            },
            IpcRequest::HydrateFile {
                file_id,
                dest_path,
                progress,
            } => {
                match hydrate_over_ipc(&mut frames, &mut write_half, &bridge, &file_id, &dest_path, progress).await {
                    HydrateOutcome::Reply(resp) => resp,
                    // The client hung up mid-hydrate: nobody is left to reply to.
                    HydrateOutcome::ClientGone => break,
                }
            }
            IpcRequest::QueueFinderCreate {
                parent_id,
                filename,
                kind,
                contents_path,
                content_type,
                request_id,
            } => {
                if parent_id.as_deref() == Some(NAMESPACE_SHARED_WITH_ME) {
                    // Answer and keep serving the connection (this used to
                    // `return`, silently dropping the connection).
                    if write_frame(
                        &mut write_half,
                        &IpcResponse::Error {
                            message: "Shared with me is read-only at the namespace root".into(),
                        },
                    )
                    .await
                    .is_err()
                    {
                        break;
                    }
                    continue;
                }
                let target = crate::engine_bridge::FinderWriteTarget {
                    file_id: None,
                    parent_id: normalize_parent_id(parent_id),
                    filename,
                    // OS-extension IPC supplies the leaf name only; keep the
                    // leaf-as-path fallback (queue_finder_create defaults
                    // rel_path → filename). The macOS/Linux extensions thread
                    // their own nesting via parent_id, not a relative path.
                    rel_path: None,
                    kind: parse_write_kind(&kind),
                    contents_path,
                    content_type,
                    base_version_identifier: None,
                };
                let fingerprint = format!(
                    "create|{}|{}|{}",
                    target.parent_id.as_deref().unwrap_or(""),
                    target.filename,
                    parse_write_kind_name(&target.kind)
                );
                let (work_db, work_bridge) = (db.clone(), bridge.clone());
                dedup_write(&write_dedup, &db, request_id, fingerprint, move || {
                    write_outcome_response(&work_db, work_bridge.queue_finder_create(target))
                })
                .await
            }
            IpcRequest::QueueFinderModify {
                file_id,
                parent_id,
                filename,
                kind,
                contents_path,
                content_type,
                base_version_identifier,
                request_id,
            } => {
                let target = crate::engine_bridge::FinderWriteTarget {
                    file_id: Some(file_id),
                    parent_id: normalize_parent_id(parent_id),
                    filename,
                    // Modify is metadata/version only; no new-file path key.
                    rel_path: None,
                    kind: parse_write_kind(&kind),
                    contents_path,
                    content_type,
                    base_version_identifier,
                };
                let fingerprint = format!(
                    "modify|{}|{}|{}",
                    target.file_id.as_deref().unwrap_or(""),
                    target.filename,
                    parse_write_kind_name(&target.kind)
                );
                let (work_db, work_bridge) = (db.clone(), bridge.clone());
                dedup_write(&write_dedup, &db, request_id, fingerprint, move || {
                    write_outcome_response(&work_db, work_bridge.queue_finder_modify(target))
                })
                .await
            }
            IpcRequest::QueueFinderDelete {
                file_id,
                base_version_identifier,
            } => {
                // The user is deleting this item: no remembered create may hand
                // it back to a later request that happens to carry the same key
                // (same file copied in again). Forget BEFORE queueing, so a
                // create that arrives while the trash is being queued is a new
                // create. The row stays in place until the server trash
                // converges, so the row alone cannot say "deleted" (task 1684).
                forget_dedup_for_item(&write_dedup, &file_id);
                write_outcome_response(&db, bridge.queue_finder_delete(&file_id, base_version_identifier))
            }
            IpcRequest::SetRecursivePin { file_id, pinned } => {
                // `set_recursive_pin` takes `sync_root` for the Windows pin-state
                // path; this module is `#![cfg(unix)]`, so it is never compiled on
                // Windows and the parameter is unused here. Resolve the real root
                // from config so the call is honest; fall back to the engine-internal
                // dir if unavailable (the value is never dereferenced on unix).
                let sync_root = crate::config::DesktopConfig::load()
                    .ok()
                    .and_then(|cfg| cfg.sync_root)
                    .unwrap_or_default();
                match bridge.set_recursive_pin(&sync_root, &file_id, pinned) {
                    Ok(outcome) => IpcResponse::PinUpdated {
                        changed_item_ids: outcome.changed_item_ids,
                        hydrate_operations: outcome.hydrate_operations,
                    },
                    Err(e) => IpcResponse::Error { message: e.to_string() },
                }
            }
            IpcRequest::RecordOpenedFile {
                file_id,
                cache_path,
                cache_bytes,
            } => match bridge.record_smart_cache_open(&file_id, std::path::Path::new(&cache_path), cache_bytes) {
                Ok(outcome) => IpcResponse::CacheCleanup {
                    evicted_file_ids: outcome.evicted_file_ids,
                },
                Err(e) => IpcResponse::Error { message: e.to_string() },
            },
            IpcRequest::EnforceSmartCache {
                max_unpinned_cache_bytes,
                disk_pressure_min_free_bytes,
            } => {
                let policy = crate::engine_bridge::CachePolicy {
                    max_unpinned_cache_bytes: max_unpinned_cache_bytes
                        .unwrap_or_else(|| crate::engine_bridge::CachePolicy::default().max_unpinned_cache_bytes),
                    disk_pressure_min_free_bytes: disk_pressure_min_free_bytes
                        .unwrap_or_else(|| crate::engine_bridge::CachePolicy::default().disk_pressure_min_free_bytes),
                };
                match bridge.enforce_smart_cache(policy) {
                    Ok(outcome) => IpcResponse::CacheCleanup {
                        evicted_file_ids: outcome.evicted_file_ids,
                    },
                    Err(e) => IpcResponse::Error { message: e.to_string() },
                }
            }
            IpcRequest::GetSyncSummary => {
                let syncing = db
                    .list_by_status(crate::state_db::FileStatus::Downloading)
                    .map(|v| v.len() as u32)
                    .unwrap_or(0);
                let cloud_only = db
                    .list_by_status(crate::state_db::FileStatus::CloudOnly)
                    .map(|v| v.len() as u32)
                    .unwrap_or(0);
                let conflicts = db
                    .list_by_status(crate::state_db::FileStatus::Conflict)
                    .map(|v| v.len() as u32)
                    .unwrap_or(0);
                IpcResponse::SyncSummary {
                    syncing,
                    cloud_only,
                    conflicts,
                }
            }
            IpcRequest::ListChanges {
                since_anchor,
                limit,
            } => {
                // Page limit: honor the caller's suggested size, clamped to a
                // 100-change ceiling (Apple caps the system at 100x its
                // suggestion; our ceiling keeps replies well inside the frame
                // budget). Absent -> 100.
                let page = limit.unwrap_or(100).clamp(1, 100) as usize;
                match db.list_file_changes_paged(since_anchor.as_deref().map(str::as_bytes), page) {
                    Ok(Some((changes, anchor))) => IpcResponse::FileProviderChanges {
                        changes: file_provider_change_payloads(&db, changes),
                        next_anchor: anchor.map(|bytes| String::from_utf8_lossy(&bytes).into_owned()),
                    },
                    Ok(None) => IpcResponse::FileProviderChanges {
                        changes: Vec::new(),
                        next_anchor: None,
                    },
                    // An unparseable anchor means the log no longer contains
                    // that cursor (sweep/gap): the replica must fall back to a
                    // full enumeration, so surface a real error rather than a
                    // silently-wrong empty reply.
                    Err(e) => IpcResponse::Error {
                        message: format!("sync anchor expired: {e}"),
                    },
                }
            }
            IpcRequest::GetSyncAnchor => IpcResponse::FileProviderSyncAnchor {
                // The persistent cursor survives extension process death (it
                // lives in state.db); nil only when nothing ever changed.
                anchor: db
                    .fp_last_anchor()
                    .ok()
                    .flatten()
                    .map(|bytes| String::from_utf8_lossy(&bytes).into_owned()),
            },
            IpcRequest::ReportMaterialized { container_ids } => match db.set_materialized_containers(&container_ids) {
                Ok(()) => IpcResponse::Ok {},
                Err(e) => IpcResponse::Error {
                    message: e.to_string(),
                },
            },
            IpcRequest::SetFileStatus { .. } => IpcResponse::Ok {},
        };
        if write_frame(&mut write_half, &resp).await.is_err() {
            break;
        }
    }
}

enum HydrateOutcome {
    Reply(IpcResponse),
    ClientGone,
}

/// Serve one `HydrateFile` request: validate, hydrate, and (when the client
/// opted in) stream `HydrateProgress` frames while the download + decrypt run.
///
/// While the hydrate future runs, this also watches the read half: if the
/// client hangs up (Finder cancelled the transfer, the extension was killed)
/// the future is dropped — the download stops and `hydrate_file`'s guard puts
/// the row's status back — instead of decrypting a file nobody will collect.
async fn hydrate_over_ipc(
    frames: &mut FrameReader<tokio::net::unix::OwnedReadHalf>,
    write_half: &mut tokio::net::unix::OwnedWriteHalf,
    bridge: &crate::engine_bridge::EngineBridge,
    file_id: &str,
    dest_path: &str,
    want_progress: bool,
) -> HydrateOutcome {
    // Task 1670 round 2: `file_id` here is the raw File Provider
    // item identifier straight off the wire — reject it before it
    // is trusted for anything else in this arm. See
    // `macos_validate_hydrate_item_identifier`'s doc comment.
    #[cfg(target_os = "macos")]
    let identifier_check = macos_validate_hydrate_item_identifier(file_id);
    #[cfg(not(target_os = "macos"))]
    let identifier_check: Result<(), &'static str> = Ok(());

    if let Err(msg) = identifier_check {
        return HydrateOutcome::Reply(IpcResponse::Error { message: msg.to_string() });
    }
    // `dest_path` arrives straight off the wire (untrusted). Bound
    // it to the caller's legitimate destinations before handing it
    // to `hydrate_file`, which decrypts vault plaintext to disk
    // (task 1247). The only real IPC caller (the File Provider
    // extension / FUSE mount) writes either under the sync root or
    // into the per-file temp cache, so both are allowed roots; if
    // no sync root is configured yet, only the temp dir is.
    // Resolve sync_root the same way the SetRecursivePin handler
    // resolves it.
    //
    // Task 1670: on macOS, `temp_root` above is THIS (sandboxed)
    // process's own private container temp dir — the File Provider
    // extension is a DIFFERENT sandboxed process with its own
    // separate container temp dir, so a real destination it builds
    // can never be inside `temp_root` (see `macos_hydrate_cache_dir`'s
    // doc comment for the full root-cause). Add the shared App
    // Group hydrate-cache directory as a third allowed root, and
    // make sure it exists (owner-only, backup-excluded — round 2)
    // before the containment check runs. Round 4: the Swift side
    // copies its staging file into the SYSTEM's own temp
    // directory and deletes THIS copy immediately after (see
    // `MACOS_HYDRATE_CACHE_TTL`'s doc comment), so this and the
    // periodic sweep in `runner.rs` are now a crash backstop —
    // they only ever find staging orphaned by a crash between
    // decrypt and that copy, not a file the system still holds.
    let sync_root = crate::config::DesktopConfig::load().ok().and_then(|cfg| cfg.sync_root);
    let temp_root = std::env::temp_dir();
    let dest = std::path::Path::new(dest_path);
    #[cfg(target_os = "macos")]
    let macos_hydrate_dir = {
        let dir = macos_hydrate_cache_dir();
        if let Err(e) = macos_ensure_hydrate_cache_dir(&dir) {
            tracing::warn!(error = %e, dir = %dir.display(), "could not create macOS hydrate-cache dir");
        }
        if let Err(e) = macos_sweep_stale_hydrate_cache_entries(&dir, MACOS_HYDRATE_CACHE_TTL, std::time::SystemTime::now()) {
            tracing::warn!(error = %e, dir = %dir.display(), "hydrate-cache TTL sweep failed (best-effort)");
        }
        dir
    };
    let mut allowed_roots: Vec<&std::path::Path> = Vec::new();
    if let Some(root) = &sync_root {
        allowed_roots.push(root.as_path());
    }
    allowed_roots.push(temp_root.as_path());
    #[cfg(target_os = "macos")]
    allowed_roots.push(macos_hydrate_dir.as_path());

    // The engine calls `report` synchronously from inside the download loop;
    // hop through an unbounded channel so this task does the socket writes.
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(u64, u64)>();
    let report = move |done: u64, total: u64| {
        let _ = tx.send((done, total));
    };
    let progress_cb: Option<&(dyn Fn(u64, u64) + Send + Sync)> = if want_progress { Some(&report) } else { None };
    let hydrate = bridge.hydrate_file_with_progress(file_id, dest, &allowed_roots, progress_cb);
    tokio::pin!(hydrate);

    let result = loop {
        tokio::select! {
            biased;
            result = &mut hydrate => break Some(result),
            Some((done, total)) = rx.recv() => {
                if write_frame(write_half, &IpcResponse::HydrateProgress { done, total }).await.is_err() {
                    break None;
                }
            }
            read = frames.fill_more() => {
                // 0 / error = the peer closed. Bytes = a pipelined request,
                // which stays buffered for the next `next_frame()`; the
                // extension never pipelines during a hydrate, so a flood
                // beyond the request cap is treated as a hang-up.
                match read {
                    Ok(n) if n > 0 && frames.buffered_len() <= MAX_REQUEST_BYTES => {}
                    _ => break None,
                }
            }
        }
    };

    match result {
        Some(Ok(())) => HydrateOutcome::Reply(IpcResponse::Ok {}),
        Some(Err(e)) => HydrateOutcome::Reply(IpcResponse::Error { message: e.to_string() }),
        None => {
            // `hydrate` is dropped when this function returns.
            tracing::info!(file_id, "IPC client went away mid-hydrate; hydration cancelled");
            HydrateOutcome::ClientGone
        }
    }
}

fn parse_write_kind(kind: &str) -> crate::engine_bridge::FinderWriteItemKind {
    if kind.eq_ignore_ascii_case("folder") || kind.eq_ignore_ascii_case("namespace") {
        crate::engine_bridge::FinderWriteItemKind::Folder
    } else {
        crate::engine_bridge::FinderWriteItemKind::File
    }
}

fn parse_write_kind_name(kind: &crate::engine_bridge::FinderWriteItemKind) -> &'static str {
    match kind {
        crate::engine_bridge::FinderWriteItemKind::Folder => "folder",
        crate::engine_bridge::FinderWriteItemKind::File => "file",
    }
}

/// Run a write-queue request, deduplicated by `request_id` when the extension
/// sent one (task 1684). Without a key this is exactly the pre-1684 behaviour:
/// `work` runs inline on this connection's task.
///
/// With a key, `work` runs at most once per key within the TTL: a repeat while
/// the first is still copying waits for and returns its reply, and a repeat
/// after it finished returns the stored reply (refreshed from the current row)
/// if the row it created still exists under the same name and is not being
/// trashed (otherwise the user deleted or renamed it since, and the repeat is a
/// new operation; a delete through this socket also forgets the stored reply). Only queued results are remembered; an error is
/// handed to whoever was already waiting but a later retry runs again.
/// Drop every remembered write reply that describes `file_id`.
fn forget_dedup_for_item(dedup: &std::sync::Arc<WriteDedup<IpcResponse>>, file_id: &str) {
    dedup.forget_where(|cached| {
        matches!(cached, IpcResponse::WriteQueued { item: Some(item), .. } if item.identifier == file_id)
    });
}

/// Decide whether a remembered write reply still describes reality, and bring it
/// up to date. `None` means the item it created is gone, renamed, or on its way
/// to the trash, so the repeat is a NEW operation and must run.
///
/// "Up to date" matters: the reply was built when the first attempt finished,
/// and finalizing the upload since then changed the item's version and status.
/// Serving the old `version_identifier` would hand the system a stale base
/// version for the user's next edit, so the item is rebuilt from the current row.
fn refresh_cached_write(db: &crate::state_db::StateDb, cached: IpcResponse) -> Option<IpcResponse> {
    let IpcResponse::WriteQueued {
        item: Some(item),
        ignored,
        message,
    } = cached
    else {
        return Some(cached);
    };
    let entry = match db.get_file(&item.identifier) {
        Ok(Some(entry)) => entry,
        Ok(None) => return None,
        // Cannot tell: prefer returning the stored result over queueing twice.
        Err(_) => {
            return Some(IpcResponse::WriteQueued {
                item: Some(item),
                ignored,
                message,
            });
        }
    };
    if entry.status == crate::state_db::FileStatus::Trashing {
        return None;
    }
    // A failed lookup of the trash queue falls through to "not deleted": the
    // alternative (re-queueing) is only ever a duplicate, never a lost file, but
    // a lookup that cannot run also cannot prove a delete, and the delete path
    // forgets the entry itself.
    if db.has_pending_trash(&item.identifier).unwrap_or(false) {
        return None;
    }
    let fresh = file_entry_payload_for_db(db, &entry, FP_ROOT_APPLE);
    if fresh.filename != item.filename {
        return None;
    }
    Some(IpcResponse::WriteQueued {
        item: Some(fresh),
        ignored,
        message,
    })
}

async fn dedup_write(
    dedup: &std::sync::Arc<WriteDedup<IpcResponse>>,
    db: &std::sync::Arc<crate::state_db::StateDb>,
    request_id: Option<String>,
    fingerprint: String,
    work: impl FnOnce() -> IpcResponse + Send + 'static,
) -> IpcResponse {
    let Some(key) = request_id else {
        return work();
    };
    if key.is_empty() || key.len() > MAX_KEY_BYTES || key.chars().any(|c| c.is_control()) {
        return IpcResponse::Error {
            message: format!("request_id must be 1 to {MAX_KEY_BYTES} printable bytes"),
        };
    }
    let validate_db = db.clone();
    dedup
        .run(
            &key,
            &fingerprint,
            std::time::Instant::now,
            move |cached: IpcResponse| refresh_cached_write(&validate_db, cached),
            |resp: &IpcResponse| matches!(resp, IpcResponse::WriteQueued { .. }),
            work,
            IpcResponse::Error {
                message: "the Finder write did not complete; try again".into(),
            },
        )
        .await
}

fn normalize_parent_id(parent_id: Option<String>) -> Option<String> {
    parent_id.and_then(|value| {
        let trimmed = value.trim();
        if is_file_provider_root(trimmed) || trimmed.starts_with("namespace:") {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn write_outcome_response(
    db: &crate::state_db::StateDb,
    result: anyhow::Result<crate::engine_bridge::FinderWriteOutcome>,
) -> IpcResponse {
    match result {
        Ok(crate::engine_bridge::FinderWriteOutcome::Ignored { message }) => IpcResponse::WriteQueued {
            item: None,
            ignored: true,
            message,
        },
        Ok(crate::engine_bridge::FinderWriteOutcome::Queued {
            file_id,
            ignored,
            message,
            ..
        }) => IpcResponse::WriteQueued {
            item: file_id
                .as_deref()
                .and_then(|id| db.get_file(id).ok().flatten())
                .map(|entry| file_entry_payload_for_db(db, &entry, FP_ROOT_APPLE)),
            ignored,
            message,
        },
        Err(e) => IpcResponse::Error { message: e.to_string() },
    }
}

/// The change-feed item assembly, extracted from the `ListChanges` arm so
/// tests can drive it without a live socket. `Deleted` rows are gone; the
/// replica only needs the identifier. For every other kind the FULL item
/// payload rides along so `didUpdateItems` needs no second round-trip. A row
/// that vanished since the change was recorded yields no item (the Swift
/// enumerator skips it; its later `deleted` row reports the exit).
fn file_provider_change_payloads(
    db: &crate::state_db::StateDb,
    changes: Vec<crate::state_db::FileChange>,
) -> Vec<FileProviderChangePayload> {
    changes
        .into_iter()
        .map(|change| FileProviderChangePayload {
            item: match change.kind {
                crate::state_db::FpChangeKind::Deleted => None,
                _ => db
                    .get_file(&change.file_id)
                    .ok()
                    .flatten()
                    .map(|entry| file_entry_payload_for_db(db, &entry, FP_ROOT_APPLE)),
            },
            file_id: change.file_id,
            kind: change.kind.as_str().to_string(),
            old_parent_id: change.old_parent_id,
            new_parent_id: change.new_parent_id,
        })
        .collect()
}

fn list_file_provider_items(db: &crate::state_db::StateDb, container_id: &str) -> Vec<FileProviderItemPayload> {
    if is_file_provider_root(container_id) {
        // Task 1701 (ruling: `.claude/tasks/decisions/finder-sidebar-single-root.md`):
        // Finder shows ONE root folder — "Beebeeb", the user's own files,
        // matching the Windows app. The root container's children are the
        // user's REAL top-level tree; the synthetic `My files` /
        // `Shared with me` / `Offline` / `Conflicts` namespace containers are
        // no longer enumerated. Shared items are not a Finder surface for
        // now (webapp only); Offline/Conflicts data is untouched — it simply
        // stops being a synthetic Finder folder (native offline affordances
        // are task 1698, conflicts the dialog design pass).
        return db
            .list_files()
            .unwrap_or_default()
            .into_iter()
            .filter(|entry| {
                is_top_level_path(&entry.path)
                    && db
                        .get_file_contract_state(&entry.file_id)
                        .ok()
                        .flatten()
                        .map(|contract| contract.namespace == crate::state_db::Namespace::MyFiles)
                        .unwrap_or(true)
            })
            .map(|entry| file_entry_payload_for_db(db, &entry, FP_ROOT_APPLE))
            .collect();
    }

    // Real folders: children by the contract's parent (the server's parent
    // UUID, which is what `files.parent_id` stores).
    db.list_files()
        .unwrap_or_default()
        .into_iter()
        .filter(|entry| {
            db.get_file_contract_state(&entry.file_id)
                .ok()
                .flatten()
                .and_then(|contract| contract.parent_id)
                .as_deref()
                == Some(container_id)
        })
        .map(|entry| file_entry_payload_for_db(db, &entry, container_id))
        .collect()
}

/// Task 1694 rule (lead decision, 2026-10-01): every LIVE folder accepts
/// adding sub-items. Finder refuses a drop into any item whose
/// `.allowsAddingSubItems` is unset, which is why drops onto Beebeeb folders
/// showed the blocked icon everywhere. The grant is gated ONLY on kind +
/// status — deliberately NOT on `permission_bits`: a read-only shared root
/// is still a live folder, and the daemon rejects unauthorized writes at the
/// write path anyway. `Trashing` is the read-only terminal state (the item
/// is on its way out) and keeps the bit unset. Files never get the bit.
/// `capabilities_for_status` stays status-only; the folder grant is applied
/// at the payload construction sites where the kind is known.
fn with_folder_add_subitems(
    kind: &str,
    status: &crate::state_db::FileStatus,
    capabilities: u32,
) -> u32 {
    if kind == "folder" && !matches!(status, crate::state_db::FileStatus::Trashing) {
        capabilities | CAP_ADD_SUBITEMS
    } else {
        capabilities
    }
}

fn is_file_provider_root(container_id: &str) -> bool {
    container_id == FP_ROOT
        || container_id == FP_ROOT_APPLE
        || container_id == "rootContainer"
        || container_id.is_empty()
        || container_id.to_ascii_lowercase().contains("root")
}

fn file_entry_payload_for_db(
    db: &crate::state_db::StateDb,
    entry: &crate::state_db::FileEntry,
    parent_identifier: &str,
) -> FileProviderItemPayload {
    let mut payload = match db.get_file_contract_state(&entry.file_id).ok().flatten() {
        Some(contract) => file_entry_payload(entry, &contract, parent_identifier),
        None => file_entry_payload_without_contract(entry, parent_identifier),
    };
    // Task 1697: real dates + child count, stamped where the DB is at hand.
    // `modified_at` conflates "server updated_at" and "conflict detected at"
    // (see the FileEntry doc comment) but is the best mtime the daemon has.
    payload.modified_at = Some(entry.modified_at.max(entry.remote_updated_at));
    if payload.created_at.is_none() {
        payload.created_at = payload.modified_at;
    }
    if entry.is_dir() {
        payload.child_item_count = Some(db.child_item_count(&entry.file_id).unwrap_or(0));
    }
    payload
}

fn file_entry_payload(
    entry: &crate::state_db::FileEntry,
    contract: &crate::state_db::FileContractState,
    fallback_parent_identifier: &str,
) -> FileProviderItemPayload {
    let parent_identifier = contract.parent_id.as_deref().unwrap_or(fallback_parent_identifier);
    let kind = match contract.item_kind {
        crate::state_db::ItemKind::Folder => "folder",
        crate::state_db::ItemKind::File => "file",
    };
    let mut capabilities = capabilities_for_status(&entry.status);
    if contract.permission_bits != 0 {
        capabilities = 0;
        if contract.permission_bits & crate::state_db::PERMISSION_READ != 0 {
            capabilities |= CAP_READ;
        }
        if contract.permission_bits & crate::state_db::PERMISSION_WRITE != 0
            && matches!(
                entry.status,
                crate::state_db::FileStatus::Local
                    | crate::state_db::FileStatus::Uploading
                    | crate::state_db::FileStatus::Conflict
                    | crate::state_db::FileStatus::CloudOnly
            )
        {
            capabilities |= CAP_WRITE | CAP_RENAME | CAP_DELETE;
        }
    }
    // The contract branch zeroes `capabilities` above, so the folder
    // add-subitems grant is re-applied here, after the permission rebuild
    // (task 1694). Task 1697: live items may also be reparented and trashed
    // (drag-move / drag-to-Trash; trash SEMANTICS are task 1698).
    let capabilities = with_folder_add_subitems(kind, &entry.status, capabilities);

    // Task 1697: version split. contentVersion changes only when the content
    // identity changes (server version + content hash when present);
    // metadataVersion tracks mtime/size/parent/name so a rename no longer
    // forces a content re-download.
    let content_version = match &entry.content_hash {
        Some(hash) => format!("{}:{}", contract.current_version.max(entry.remote_updated_at), hash),
        None => format!("{}", contract.current_version.max(entry.remote_updated_at)),
    };
    let metadata_version = format!(
        "{}:{}:{}:{}:{}",
        entry.modified_at,
        entry.size_bytes,
        parent_identifier,
        filename_from_path(&entry.path),
        entry.status.as_str()
    );

    FileProviderItemPayload {
        identifier: entry.file_id.clone(),
        parent_identifier: parent_identifier.to_string(),
        filename: filename_from_path(&entry.path),
        kind: kind.to_string(),
        size_bytes: entry.size_bytes,
        content_type: contract.content_type.clone(),
        status: file_status_string(&entry.status).to_string(),
        capabilities,
        version_identifier: Some(format!(
            "{}:{}:{}",
            contract.current_version.max(entry.remote_updated_at),
            entry.modified_at,
            entry.size_bytes
        )),
        // Dates/child count are stamped by `file_entry_payload_for_db`, where
        // the DB handle is available.
        created_at: None,
        modified_at: None,
        child_item_count: None,
        content_version: Some(content_version),
        metadata_version: Some(metadata_version),
    }
}

fn file_entry_payload_without_contract(
    entry: &crate::state_db::FileEntry,
    parent_identifier: &str,
) -> FileProviderItemPayload {
    let status = file_status_string(&entry.status);
    // Kind is derivable WITHOUT a contract: `FileEntry.item_kind` is read
    // from the `files.item_kind` column by the SELECT mappers (written only
    // by `set_file_contract_state`). It used to be hardcoded to "file",
    // which mis-reported folders AND lost them the folder add-subitems
    // grant (task 1694).
    let kind = match entry.item_kind {
        crate::state_db::ItemKind::Folder => "folder",
        crate::state_db::ItemKind::File => "file",
    };
    let capabilities = with_folder_add_subitems(
        kind,
        &entry.status,
        capabilities_for_status(&entry.status),
    );

    // Task 1697: version split without a contract — content identity falls
    // back to the row's remote clock (no hash available), metadata to
    // mtime/size/parent/name.
    let content_version = format!("{}", entry.remote_updated_at);
    let metadata_version = format!(
        "{}:{}:{}:{}:{}",
        entry.modified_at,
        entry.size_bytes,
        parent_identifier,
        filename_from_path(&entry.path),
        status
    );

    FileProviderItemPayload {
        identifier: entry.file_id.clone(),
        parent_identifier: parent_identifier.to_string(),
        filename: filename_from_path(&entry.path),
        kind: kind.to_string(),
        size_bytes: entry.size_bytes,
        content_type: None,
        status: status.to_string(),
        capabilities,
        version_identifier: Some(format!(
            "{}:{}:{}",
            entry.remote_updated_at, entry.modified_at, entry.size_bytes
        )),
        created_at: None,
        modified_at: None,
        child_item_count: None,
        content_version: Some(content_version),
        metadata_version: Some(metadata_version),
    }
}

fn capabilities_for_status(status: &crate::state_db::FileStatus) -> u32 {
    match status {
        // `Trashing` (a locally-deleted file whose server-trash is pending) gets
        // no write/rename/delete caps — the item is on its way out; it is grouped
        // with the other read-only terminal states.
        crate::state_db::FileStatus::CloudOnly
        | crate::state_db::FileStatus::Downloading
        | crate::state_db::FileStatus::Error
        | crate::state_db::FileStatus::Trashing => CAP_READ,
        crate::state_db::FileStatus::Conflict => CAP_READ | CAP_RENAME,
        // Task 1697: live items may be reparented (drag-move) and trashed.
        // Trash SEMANTICS remain 1698's decision; the capability only stops
        // Finder from blocking the gesture outright.
        crate::state_db::FileStatus::Local | crate::state_db::FileStatus::Uploading => {
            CAP_READ | CAP_WRITE | CAP_RENAME | CAP_DELETE | CAP_REPARENT | CAP_TRASH
        }
    }
}

fn file_status_string(status: &crate::state_db::FileStatus) -> &'static str {
    match status {
        crate::state_db::FileStatus::CloudOnly => "cloud_only",
        crate::state_db::FileStatus::Downloading => "downloading",
        crate::state_db::FileStatus::Local => "local",
        crate::state_db::FileStatus::Uploading => "uploading",
        crate::state_db::FileStatus::Conflict => "conflict",
        crate::state_db::FileStatus::Error => "error",
        crate::state_db::FileStatus::Trashing => "trashing",
    }
}

fn filename_from_path(path: &str) -> String {
    let trimmed = path.trim_matches('/');
    trimmed
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or(trimmed)
        .to_string()
}

fn is_top_level_path(path: &str) -> bool {
    let trimmed = path.trim_matches('/');
    !trimmed.is_empty() && !trimmed.contains('/')
}

#[cfg(test)]
#[path = "ipc_socket_framing_tests.rs"]
mod framing_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state_db::{FileEntry, FileStatus, ItemKind, Namespace, PERMISSION_READ, PERMISSION_WRITE, StateDb};
    use tempfile::tempdir;

    // ------------------------------------------------------------------
    // Task 1701: Finder single root. The root container enumerates the
    // user's REAL top-level tree; the synthetic namespace containers
    // (`My files` / `Shared with me` / `Offline` / `Conflicts`) stop being a
    // Finder surface. Ruling: .claude/tasks/decisions/
    // finder-sidebar-single-root.md. RED-first: these were seen failing
    // against the namespace-split enumeration before the root arm changed.
    //
    // parent_id finding (spec point 1): `files.parent_id` stores the
    // SERVER's parent UUID, or NULL for vault-root items — never a
    // `namespace:` id (`normalize_parent_id` strips those from Finder
    // writes). The namespace parentage existed ONLY at payload-assembly
    // time, as the `file_entry_payload*` fallback identifier. So no state.db
    // remap is needed: pointing the fallback at the root container presents
    // top-level items as the root's children.
    // ------------------------------------------------------------------

    #[test]
    fn test_1701_root_container_enumerates_real_files_not_namespaces() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        db.upsert_file(&FileEntry {
            file_id: "1701-file".into(),
            path: "/notes.txt".into(),
            status: FileStatus::Local,
            size_bytes: 3,
            modified_at: 1,
            content_hash: None,
            remote_updated_at: 1,
            parent_id: None,
            item_kind: ItemKind::File,
        })
        .unwrap();
        db.upsert_file(&FileEntry {
            file_id: "1701-folder".into(),
            path: "/Projects".into(),
            status: FileStatus::Local,
            size_bytes: 0,
            modified_at: 1,
            content_hash: None,
            remote_updated_at: 1,
            parent_id: None,
            item_kind: ItemKind::Folder,
        })
        .unwrap();
        // A shared root stays in state.db (NO data loss) but is no longer a
        // Finder surface — the ruling sends shared files to the webapp.
        seed_shared_root(&db, "1701-shared", "Shared with me/Team", PERMISSION_READ);

        let items = list_file_provider_items(&db, FP_ROOT_APPLE);
        let names: Vec<&str> = items.iter().map(|i| i.filename.as_str()).collect();
        assert!(
            items.iter().all(|i| i.kind != "namespace"),
            "zero synthetic namespace items may be enumerated at the root, got {names:?}"
        );
        for synthetic in ["My files", "Shared with me", "Offline", "Conflicts"] {
            assert!(
                !items.iter().any(|i| i.filename == synthetic),
                "synthetic container {synthetic:?} must not be enumerated at the root"
            );
        }
        assert!(
            items.iter().any(|i| i.identifier == "1701-file"),
            "the user's top-level file must be a child of the root container"
        );
        assert!(
            items.iter().any(|i| i.identifier == "1701-folder"),
            "the user's top-level folder must be a child of the root container"
        );
        assert!(
            !items.iter().any(|i| i.identifier == "1701-shared"),
            "shared roots are not a Finder surface (1701 ruling: webapp only)"
        );
    }

    #[test]
    fn test_1701_top_level_items_are_presented_as_children_of_the_root() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        db.upsert_file(&FileEntry {
            file_id: "1701-file".into(),
            path: "/notes.txt".into(),
            status: FileStatus::Local,
            size_bytes: 3,
            modified_at: 1,
            content_hash: None,
            remote_updated_at: 1,
            parent_id: None,
            item_kind: ItemKind::File,
        })
        .unwrap();
        // With a contract whose parent is the server root (`parent_id` NULL —
        // exactly what snapshot ingest writes for top-level rows).
        let mut contract = db.get_file_contract_state("1701-file").unwrap().unwrap();
        contract.namespace = Namespace::MyFiles;
        contract.item_kind = ItemKind::File;
        db.set_file_contract_state(&contract).unwrap();

        let items = list_file_provider_items(&db, FP_ROOT_APPLE);
        let item = items.iter().find(|i| i.identifier == "1701-file").unwrap();
        assert_eq!(
            item.parent_identifier, FP_ROOT_APPLE,
            "a top-level item must be presented as a child of the ROOT container, not of a synthetic namespace"
        );
    }

    #[test]
    fn test_1701_change_feed_top_level_item_parents_onto_the_root() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        db.upsert_file(&FileEntry {
            file_id: "1701-file".into(),
            path: "/notes.txt".into(),
            status: FileStatus::Local,
            size_bytes: 3,
            modified_at: 1,
            content_hash: None,
            remote_updated_at: 1,
            parent_id: None,
            item_kind: ItemKind::File,
        })
        .unwrap();
        db.record_file_change("1701-file", crate::state_db::FpChangeKind::Modified, None)
            .unwrap();

        let (changes, _) = db.list_file_changes_paged(None, 100).unwrap().unwrap();
        let payloads = file_provider_change_payloads(&db, changes);
        let payload = payloads.iter().find(|p| p.file_id == "1701-file").unwrap();
        let item = payload
            .item
            .as_ref()
            .expect("a modified row carries the full item payload");
        assert_eq!(
            item.parent_identifier, FP_ROOT_APPLE,
            "the change feed must present a top-level item as a child of the ROOT container — the Swift materialized-set filter keys off this parent"
        );
    }

    #[test]
    fn test_1701_shared_root_folder_payload_keeps_read_only_caps_with_add_subitems() {
        // Migrated from `test_shared_namespace_lists_roots_with_permission_
        // capabilities` (task 1694/1697): the `namespace:shared_with_me`
        // container is NO LONGER an enumeration surface (1701 ruling — shared
        // files live in the webapp), so the intent moves to the payload
        // builder itself: a shared root is a LIVE folder, and its capability
        // contract is unchanged should the system ever hold it.
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed_shared_root(&db, "read-root", "Shared with me/Read only", PERMISSION_READ);
        seed_shared_root(
            &db,
            "write-root",
            "Shared with me/Editable",
            PERMISSION_READ | PERMISSION_WRITE,
        );

        let read_only = db.get_file("read-root").unwrap().unwrap();
        let payload = file_entry_payload_for_db(&db, &read_only, FP_ROOT_APPLE);
        assert_eq!(payload.kind, "folder");
        // Task 1694 (lead rule): every LIVE folder accepts adding sub-items,
        // gated only on kind+status — NOT on permission_bits. A read-only
        // shared root is still a live folder, so it carries
        // READ | ADD_SUBITEMS (unauthorized writes are rejected at the write
        // path, not by hiding the folder's drop affordance).
        assert_eq!(payload.capabilities, CAP_READ | CAP_ADD_SUBITEMS);

        let editable = db.get_file("write-root").unwrap().unwrap();
        let payload = file_entry_payload_for_db(&db, &editable, FP_ROOT_APPLE);
        assert_eq!(
            payload.capabilities & (CAP_READ | CAP_WRITE | CAP_RENAME | CAP_DELETE),
            CAP_READ | CAP_WRITE | CAP_RENAME | CAP_DELETE
        );
    }

    #[test]
    fn test_is_authorized_peer_matches_only_same_uid() {
        // The real cross-UID rejection cannot be exercised end-to-end in a
        // sandboxed test runner (it needs root/setuid to connect as a different
        // real UID), so this pins the exact comparison the live peer-cred check
        // uses (task 1247).
        assert!(is_authorized_peer(1000, 1000));
        assert!(!is_authorized_peer(1000, 1001));
        assert!(!is_authorized_peer(0, 1000));
        assert!(is_authorized_peer(0, 0));
    }

    // ── Task 1694: Finder blocks adding files to folders — add-subitems
    // capability bit ─────────────────────────────────────────────────────────

    // Task 1701: the namespace-payload variant of this test
    // (`test_1694_namespace_payload_grants_add_subitems`) died WITH
    // `namespace_payload` — the synthetic containers are no longer built, and
    // dead code does not ship. The 1694 intent lives on in the folder payload
    // tests below and in the 1701 shared-root test above.

    fn seed_1694_item(
        db: &StateDb,
        file_id: &str,
        status: FileStatus,
        permissions: i64,
        item_kind: ItemKind,
    ) {
        db.upsert_file(&FileEntry {
            file_id: file_id.into(),
            path: format!("My files/{file_id}.txt"),
            status,
            size_bytes: 0,
            modified_at: 1,
            content_hash: None,
            remote_updated_at: 1,
            parent_id: None,
            item_kind: item_kind.clone(),
        })
        .unwrap();
        let mut contract = db.get_file_contract_state(file_id).unwrap().unwrap();
        contract.namespace = Namespace::MyFiles;
        contract.permission_bits = permissions;
        contract.item_kind = item_kind.clone();
        db.set_file_contract_state(&contract).unwrap();
    }

    #[test]
    fn test_1694_local_folder_payload_grants_add_subitems() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed_1694_item(
            &db,
            "1694-local-folder",
            FileStatus::Local,
            PERMISSION_READ | PERMISSION_WRITE,
            ItemKind::Folder,
        );
        let entry = db.get_file("1694-local-folder").unwrap().unwrap();
        let payload = file_entry_payload_for_db(&db, &entry, FP_ROOT_APPLE);
        assert_eq!(payload.kind, "folder");
        assert!(
            payload.capabilities & CAP_ADD_SUBITEMS != 0,
            "a live Local folder must grant ADD_SUBITEMS (task 1694), got {:#b}",
            payload.capabilities
        );
    }

    #[test]
    fn test_1694_cloud_only_folder_payload_grants_add_subitems() {
        // The majority real-world case: CloudOnly entries dominate the tree
        // (Guus's vault: 8195 cloud_only files), so the fix is worthless
        // unless a CloudOnly folder also accepts drops.
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed_1694_item(
            &db,
            "1694-cloud-folder",
            FileStatus::CloudOnly,
            PERMISSION_READ | PERMISSION_WRITE,
            ItemKind::Folder,
        );
        let entry = db.get_file("1694-cloud-folder").unwrap().unwrap();
        let payload = file_entry_payload_for_db(&db, &entry, FP_ROOT_APPLE);
        assert_eq!(payload.kind, "folder");
        assert!(
            payload.capabilities & CAP_ADD_SUBITEMS != 0,
            "a live CloudOnly folder must grant ADD_SUBITEMS (task 1694), got {:#b}",
            payload.capabilities
        );
    }

    #[test]
    fn test_1694_trashing_folder_payload_does_not_grant_add_subitems() {
        // `Trashing` is the read-only terminal state (a locally-deleted item
        // whose server-trash is pending); a folder on its way out must NOT
        // accept new sub-items — in either payload branch.
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed_1694_item(
            &db,
            "1694-trashing-folder",
            FileStatus::Trashing,
            PERMISSION_READ | PERMISSION_WRITE,
            ItemKind::Folder,
        );
        let entry = db.get_file("1694-trashing-folder").unwrap().unwrap();
        let contract_payload = file_entry_payload_for_db(&db, &entry, FP_ROOT_APPLE);
        assert_eq!(contract_payload.kind, "folder");
        assert!(
            contract_payload.capabilities & CAP_ADD_SUBITEMS == 0,
            "a Trashing folder must NOT grant ADD_SUBITEMS, got {:#b}",
            contract_payload.capabilities
        );

        let no_contract_payload =
            file_entry_payload_without_contract(&entry, FP_ROOT_APPLE);
        assert!(
            no_contract_payload.capabilities & CAP_ADD_SUBITEMS == 0,
            "a Trashing folder must NOT grant ADD_SUBITEMS in the no-contract fallback either, got {:#b}",
            no_contract_payload.capabilities
        );
    }

    #[test]
    fn test_1694_file_payload_does_not_grant_add_subitems() {
        // Only folders accept sub-items; a file payload must never carry the
        // bit, whatever its status.
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed_1694_item(
            &db,
            "1694-local-file",
            FileStatus::Local,
            PERMISSION_READ | PERMISSION_WRITE,
            ItemKind::File,
        );
        let entry = db.get_file("1694-local-file").unwrap().unwrap();
        let payload = file_entry_payload_for_db(&db, &entry, FP_ROOT_APPLE);
        assert_eq!(payload.kind, "file");
        assert!(
            payload.capabilities & CAP_ADD_SUBITEMS == 0,
            "a file payload must NOT grant ADD_SUBITEMS, got {:#b}",
            payload.capabilities
        );
    }

    #[test]
    fn test_1694_no_contract_folder_payload_grants_add_subitems_and_reports_folder_kind() {
        // The no-contract fallback used to hardcode kind "file". It must
        // derive the kind from FileEntry.item_kind (the files.item_kind
        // column) — otherwise a contract-less folder is mis-reported as a
        // file AND loses the folder ADD grant.
        let entry = FileEntry {
            file_id: "1694-nc-folder".into(),
            path: "My files/1694-nc-folder".into(),
            status: FileStatus::Local,
            size_bytes: 0,
            modified_at: 1,
            content_hash: None,
            remote_updated_at: 1,
            parent_id: None,
            item_kind: ItemKind::Folder,
        };
        let payload = file_entry_payload_without_contract(&entry, FP_ROOT_APPLE);
        assert_eq!(
            payload.kind, "folder",
            "no-contract fallback must derive kind from FileEntry.item_kind, not hardcode \"file\""
        );
        assert!(
            payload.capabilities & CAP_ADD_SUBITEMS != 0,
            "a no-contract live folder must grant ADD_SUBITEMS, got {:#b}",
            payload.capabilities
        );
    }

    #[test]
    fn test_1694_no_contract_file_payload_does_not_grant_add_subitems() {
        let entry = FileEntry {
            file_id: "1694-nc-file".into(),
            path: "My files/1694-nc-file.txt".into(),
            status: FileStatus::Local,
            size_bytes: 0,
            modified_at: 1,
            content_hash: None,
            remote_updated_at: 1,
            parent_id: None,
            item_kind: ItemKind::File,
        };
        let payload = file_entry_payload_without_contract(&entry, FP_ROOT_APPLE);
        assert_eq!(payload.kind, "file");
        assert!(
            payload.capabilities & CAP_ADD_SUBITEMS == 0,
            "a no-contract file payload must NOT grant ADD_SUBITEMS, got {:#b}",
            payload.capabilities
        );
    }

    fn seed_shared_root(db: &StateDb, file_id: &str, path: &str, permissions: i64) {
        db.upsert_file(&FileEntry {
            file_id: file_id.into(),
            path: path.into(),
            status: FileStatus::Local,
            size_bytes: 0,
            modified_at: 1,
            content_hash: None,
            remote_updated_at: 1,
            parent_id: None,
            item_kind: ItemKind::File,
        })
        .unwrap();
        let mut contract = db.get_file_contract_state(file_id).unwrap().unwrap();
        contract.namespace = Namespace::SharedWithMe;
        contract.shared_root_id = Some(file_id.into());
        contract.share_id = Some(format!("invite-{file_id}"));
        contract.permission_bits = permissions;
        contract.item_kind = ItemKind::Folder;
        contract.content_type = Some("public.folder".into());
        db.set_file_contract_state(&contract).unwrap();
    }

    // ── Task 1524: macOS IPC socket path + non-panicking bind ──────────────

    #[cfg(target_os = "macos")]
    #[test]
    fn test_macos_ipc_socket_path_is_inside_group_container_not_tmp() {
        // Regression pin for the actual bug: the daemon must resolve the
        // socket inside the sandbox-reachable shared App Group container,
        // never under /tmp or $XDG_RUNTIME_DIR (which the sandbox blocks
        // entirely, causing the original "Timed out waiting for the local
        // Beebeeb sync daemon" report).
        let home = std::path::Path::new("/Users/guuslangelaar");
        let path = macos_ipc_socket_path_in(home);

        assert!(
            path.starts_with(home.join("Library").join("Group Containers")),
            "must live inside the shared App Group container, got {path:?}"
        );
        assert!(
            path.to_str().unwrap().contains(MACOS_APP_GROUP_ID),
            "must be namespaced under the app's own App Group id, got {path:?}"
        );
        assert!(
            !path.starts_with("/tmp"),
            "must not fall back to /tmp under the sandbox, got {path:?}"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_macos_ipc_socket_path_fits_sun_path_length_budget() {
        // sockaddr_un.sun_path on macOS is 104 bytes INCLUDING the NUL
        // terminator (<sys/un.h>), so the usable path length is 103 bytes.
        // Exercise a realistically long macOS short username (20 chars,
        // longer than this dev machine's own "guuslangelaar" at 13, and
        // longer than macOS's own 20-char "short name" convention allows in
        // most default cases) to prove there is real headroom, not a value
        // that only happens to fit this one machine. Built by repetition
        // (not hand-counted) so the length is exact by construction.
        let username: String = "x".repeat(20);
        let home = std::path::PathBuf::from(format!("/Users/{username}"));
        let home = home.as_path();

        let path = macos_ipc_socket_path_in(home);
        let byte_len = path.to_str().unwrap().len();
        assert!(
            byte_len < 104,
            "path must fit sockaddr_un.sun_path (104 bytes incl. NUL); \
             got a {byte_len}-byte path for a 20-char username: {path:?}"
        );
    }

    // ── Task 1670: macOS hydrate-cache dir (fetchContents helper error) ────

    #[cfg(target_os = "macos")]
    #[test]
    fn test_macos_hydrate_cache_dir_is_inside_group_container_not_tmp() {
        // Regression pin for the actual bug: a hydration destination built
        // from THIS process's own `std::env::temp_dir()` can never match one
        // the File Provider extension builds from ITS OWN (different)
        // sandboxed temp dir — see `macos_hydrate_cache_dir`'s doc comment.
        // The shared App Group container is the one directory both sides can
        // actually agree on.
        let home = std::path::Path::new("/Users/guuslangelaar");
        let path = macos_hydrate_cache_dir_in(home);

        assert!(
            path.starts_with(home.join("Library").join("Group Containers")),
            "must live inside the shared App Group container, got {path:?}"
        );
        assert!(
            path.to_str().unwrap().contains(MACOS_APP_GROUP_ID),
            "must be namespaced under the app's own App Group id, got {path:?}"
        );
        assert!(
            !path.starts_with(std::env::temp_dir()),
            "must not be this process's own private sandboxed temp dir \
             (that is exactly the bug), got {path:?}"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_macos_hydrate_cache_dir_does_not_collide_with_ipc_socket_path() {
        // Both live under the same App Group container by design (task
        // 1670's doc comment); they must still be distinct paths, or a
        // hydrated file could shadow (or be shadowed by) the IPC socket.
        let home = std::path::Path::new("/Users/guuslangelaar");
        let socket = macos_ipc_socket_path_in(home);
        let hydrate_dir = macos_hydrate_cache_dir_in(home);

        assert_ne!(socket, hydrate_dir, "must not reuse the socket's own path");
        assert!(
            !hydrate_dir.starts_with(&socket) && !socket.starts_with(&hydrate_dir),
            "must not nest one inside the other: socket={socket:?} hydrate_dir={hydrate_dir:?}"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_macos_hydrate_cache_dir_is_a_real_allowed_root_for_hydration() {
        // End-to-end proof (within what a sandbox-free `cargo test` process
        // can exercise) that a destination built the way
        // `XPCBridge.hydrateDestinationURL(for:)` builds it — a file directly
        // inside the hydrate-cache dir — passes the SAME containment check
        // `hydrate_dest_is_allowed` (engine_bridge.rs) runs against the
        // `allowed_roots` this handler now includes it in.
        let dir = std::env::temp_dir().join(format!("bb-hydrate-cache-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let dest = dir.join("some-file-id");

        assert!(
            crate::engine_bridge::hydrate_dest_is_allowed(&dest, &[dir.as_path()]),
            "a destination directly inside the hydrate-cache dir must be an allowed hydration target"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── Task 1670 round 2: identifier validation, dir hardening, purge/TTL ──

    #[cfg(target_os = "macos")]
    #[test]
    fn test_macos_validate_hydrate_item_identifier_accepts_a_real_uuid() {
        assert!(macos_validate_hydrate_item_identifier("3f9a2b7e-1234-4c1a-8f9a-abcdef012345").is_ok());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_macos_validate_hydrate_item_identifier_rejects_hostile_identifiers() {
        for hostile in [
            "",
            "..",
            "../../etc/passwd",
            "/etc/passwd",
            "a/b",
            "a\\b",
            "foo..bar",
            "trailing/",
        ] {
            assert!(
                macos_validate_hydrate_item_identifier(hostile).is_err(),
                "must reject hostile identifier {hostile:?}"
            );
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_macos_ensure_hydrate_cache_dir_sets_owner_only_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let parent = tempdir().unwrap();
        let dir = parent.path().join("hydrate-cache");
        // Precondition: the dir does not exist yet, AND (defense-in-depth
        // case) an existing dir with looser perms must still be forced down.
        macos_ensure_hydrate_cache_dir(&dir).expect("must create the dir");
        let mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700, "hydrate-cache dir must be owner-only, got {mode:o}");

        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        macos_ensure_hydrate_cache_dir(&dir).expect("must be idempotent on an existing dir");
        let mode_after = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            mode_after, 0o700,
            "a pre-existing dir with looser perms must be forced back to 0o700, got {mode_after:o}"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_macos_exclude_from_backups_sets_the_time_machine_xattr() {
        use std::os::unix::ffi::OsStrExt;

        let dir = tempdir().unwrap();
        macos_exclude_from_backups(dir.path()).expect("setxattr must succeed on a writable dir");

        let attr_name =
            std::ffi::CString::new("com.apple.metadata:com_apple_backup_excludeItem").unwrap();
        let path_c = std::ffi::CString::new(dir.path().as_os_str().as_bytes()).unwrap();
        let mut buf = vec![0u8; 64];
        let n = unsafe {
            libc::getxattr(
                path_c.as_ptr(),
                attr_name.as_ptr(),
                buf.as_mut_ptr() as *mut libc::c_void,
                buf.len(),
                0,
                0,
            )
        };
        assert!(n > 0, "the backup-exclude xattr must be readable back after setting it, got {n}");
        assert_eq!(&buf[..n as usize], b"com.apple.backupd");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_macos_purge_hydrate_cache_dir_removes_all_entries_but_keeps_the_dir() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("staged-a.bin"), b"plaintext-a").unwrap();
        std::fs::write(dir.path().join("staged-b.bin"), b"plaintext-b").unwrap();
        std::fs::create_dir(dir.path().join("stray-subdir")).unwrap();
        std::fs::write(dir.path().join("stray-subdir").join("c.bin"), b"c").unwrap();

        let removed = macos_purge_hydrate_cache_dir(dir.path()).expect("purge must succeed");

        assert_eq!(removed, 3, "must report one removal per top-level entry (2 files + 1 dir)");
        assert!(dir.path().exists(), "the hydrate-cache dir itself must remain");
        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            0,
            "every entry inside the dir must be gone"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_macos_purge_hydrate_cache_dir_missing_dir_is_not_an_error() {
        let parent = tempdir().unwrap();
        let missing = parent.path().join("never-created");
        assert_eq!(macos_purge_hydrate_cache_dir(&missing).unwrap(), 0);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_macos_sweep_stale_hydrate_cache_entries_removes_old_keeps_fresh() {
        let dir = tempdir().unwrap();
        let stale = dir.path().join("stale.bin");
        let fresh = dir.path().join("fresh.bin");
        std::fs::write(&stale, b"old-plaintext").unwrap();
        std::fs::write(&fresh, b"new-plaintext").unwrap();

        let now = std::time::SystemTime::now();
        let ttl = std::time::Duration::from_secs(600);
        // Backdate only `stale`'s mtime past the TTL; `fresh` keeps its
        // just-written (now-ish) mtime.
        let old_mtime = now - std::time::Duration::from_secs(700);
        std::fs::File::open(&stale).unwrap().set_modified(old_mtime).unwrap();

        let removed = macos_sweep_stale_hydrate_cache_entries(dir.path(), ttl, now).expect("sweep must succeed");

        assert_eq!(removed, 1, "exactly the stale entry must be swept");
        assert!(!stale.exists(), "the stale entry must be removed");
        assert!(fresh.exists(), "the fresh entry must survive the sweep");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_macos_sweep_stale_hydrate_cache_entries_skips_in_progress_temp_files() {
        // Task 1670 round 3: `write_hydrated_plaintext` (`engine_bridge.rs`)
        // stages a hydration under a temp name shaped exactly like this
        // (`.{leaf}.{uuid}.part`) BEFORE it writes a single byte, then
        // publishes it with one atomic `renameat`. Give the temp-shaped entry
        // an mtime far past the TTL — if the sweep age-checked it like any
        // other file, this would (wrongly) remove it out from under a write
        // that could still be in flight. A real, already-published (non-temp)
        // stale entry alongside it proves the sweep is still doing real work,
        // not just skipping everything.
        let dir = tempdir().unwrap();
        let in_progress = dir.path().join(".real-file.9f8e7d6c-1234-4abc-9def-0123456789ab.part");
        let stale_final = dir.path().join("real-file");
        std::fs::write(&in_progress, b"partial-write-in-flight").unwrap();
        std::fs::write(&stale_final, b"already-published-plaintext").unwrap();

        let now = std::time::SystemTime::now();
        let ttl = std::time::Duration::from_secs(600);
        let old_mtime = now - std::time::Duration::from_secs(700);
        std::fs::File::open(&in_progress).unwrap().set_modified(old_mtime).unwrap();
        std::fs::File::open(&stale_final).unwrap().set_modified(old_mtime).unwrap();

        let removed = macos_sweep_stale_hydrate_cache_entries(dir.path(), ttl, now).expect("sweep must succeed");

        assert_eq!(removed, 1, "only the final (non-temp) stale entry must be swept");
        assert!(
            in_progress.exists(),
            "an in-progress temp-named entry must never be removed by the sweep, however old its mtime"
        );
        assert!(!stale_final.exists(), "the stale, already-published entry must still be removed");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_macos_is_hydrate_cache_temp_name_matches_write_hydrated_plaintexts_shape() {
        assert!(macos_is_hydrate_cache_temp_name(std::ffi::OsStr::new(
            ".some-file-id.3f9a2b7e-1234-4c1a-8f9a-abcdef012345.part"
        )));
        // A final, already-published name must never be mistaken for a temp
        // name — this is exactly what `macos_sweep_stale_hydrate_cache_entries`
        // relies on to still age-check real entries.
        assert!(!macos_is_hydrate_cache_temp_name(std::ffi::OsStr::new(
            "some-file-id.3f9a2b7e"
        )));
        assert!(!macos_is_hydrate_cache_temp_name(std::ffi::OsStr::new(
            ".dotfile-without-the-part-suffix"
        )));
        assert!(!macos_is_hydrate_cache_temp_name(std::ffi::OsStr::new(
            "no-leading-dot.uuid.part"
        )));
    }

    /// Serializes every test that mutates the process-wide `$HOME` env var,
    /// so `cargo test`'s default parallel test threads can't interleave two
    /// mutations of the same global state.
    #[cfg(target_os = "macos")]
    static HOME_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[cfg(target_os = "macos")]
    #[test]
    fn test_macos_ipc_socket_path_ignores_sandboxed_home_env_var() {
        // Task 1524 follow-up regression pin: a sandboxed process has $HOME
        // rewritten to the app's container directory by macOS itself. This
        // proves `ipc_socket_path()` does NOT trust $HOME for that — it must
        // resolve the same real home regardless of what $HOME says, via the
        // password database (getpwuid_r), matching the Swift side's
        // `FileManager.homeDirectoryForCurrentUser`.
        let _guard = HOME_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        let real_home =
            getpwuid_home_dir().expect("getpwuid_r must resolve a real home directory on this machine");
        let expected = macos_ipc_socket_path_in(&real_home);

        let original_home = std::env::var_os("HOME");
        // A realistic sandboxed $HOME, per the task 1524 root-cause report:
        // "/Users/guuslangelaar/Library/Containers/io.beebeeb.app/Data".
        let sandbox_home = "/Users/x/Library/Containers/io.beebeeb.app/Data";
        // SAFETY: mutation is serialized by HOME_ENV_LOCK above, and every
        // path out of this test (including panics, via catch_unwind below)
        // restores the original value before returning.
        unsafe { std::env::set_var("HOME", sandbox_home) };

        let result = std::panic::catch_unwind(ipc_socket_path);

        // SAFETY: same guard as the set_var above.
        match &original_home {
            Some(v) => unsafe { std::env::set_var("HOME", v) },
            None => unsafe { std::env::remove_var("HOME") },
        }

        let path = result.unwrap_or_else(|e| std::panic::resume_unwind(e));

        assert_eq!(
            path, expected,
            "ipc_socket_path() must ignore a sandboxed $HOME and resolve the real home via \
             getpwuid_r instead, got {path:?}, expected {expected:?}"
        );
        assert!(
            !path.to_str().unwrap().contains("/Library/Containers/"),
            "must never resolve into the sandbox container path even when $HOME points there, \
             got {path:?}"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_macos_ipc_socket_path_fits_sun_path_on_this_machine() {
        // Complements test_macos_ipc_socket_path_fits_sun_path_budget's
        // synthetic 20-char username: this exercises the REAL
        // `ipc_socket_path()` (through getpwuid_home_dir()/the dirs
        // fallback) against whatever machine cargo test actually runs on,
        // so a regression that only shows up with this machine's real home
        // directory doesn't hide behind the synthetic case.
        let _guard = HOME_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        let path = ipc_socket_path();
        let byte_len = path.to_str().expect("path must be valid UTF-8").len();
        // `byte_len + 1 <= 104` (path bytes + NUL terminator, within the
        // 104-byte sun_path budget) rewritten as `byte_len < 104` for clippy
        // (int_plus_one) — same inequality, since both sides are integers.
        assert!(
            byte_len < 104,
            "ipc_socket_path() ({byte_len} bytes) + NUL terminator must fit \
             sockaddr_un.sun_path (104 bytes total) on this machine, got {path:?}"
        );
    }

    #[test]
    fn test_bind_ipc_listener_returns_error_instead_of_panicking_on_bad_path() {
        // A path under a directory that does not exist: UnixListener::bind
        // must fail with a real OS error (ENOENT), not panic and not
        // silently succeed. This is the exact failure shape the sandbox
        // produced on macOS before this fix (bind refused, formerly hidden
        // behind `.expect()`).
        let bad_path = std::path::Path::new("/nonexistent-beebeeb-1524-dir/socket.sock");

        let result = std::panic::catch_unwind(|| bind_ipc_listener(bad_path));

        match result {
            Ok(Ok(_listener)) => panic!("binding under a nonexistent directory must not succeed"),
            Ok(Err(_io_error)) => {} // expected: a real error, not a panic
            Err(_) => panic!("bind_ipc_listener must return Err on a bind failure, not panic"),
        }
    }

    #[tokio::test]
    async fn ipc_publication_failure_does_not_signal_readiness() {
        use std::sync::Arc;

        let dir = tempdir().unwrap();
        let db = Arc::new(crate::state_db::StateDb::open(dir.path().join("state.db")).unwrap());
        let api = Arc::new(crate::api_client::ApiClient::new(
            "https://api.beebeeb.io".into(),
            "token".into(),
            [7u8; 32],
        ));
        let bridge = Arc::new(crate::engine_bridge::EngineBridge::new(db.clone(), api));
        let (_cancel_tx, cancel_rx) = oneshot::channel();
        let (ready_tx, ready_rx) = oneshot::channel();
        let result = serve_ipc_at_with_ready(
            dir.path().join("missing/ipc.sock"),
            db,
            bridge,
            cancel_rx,
            Some(ready_tx),
        )
        .await;
        assert!(result.is_err(), "startup must fail for a nonexistent parent");
        assert!(ready_rx.await.is_err(), "startup failure must not signal readiness");
    }

    // Exercise filesystem publication separately from the OS socket call. The
    // real-socket test below also verifies that clients can reach the listener.
    #[test]
    fn ipc_publication_is_private_until_hardened() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        let path = dir.path().join("ipc.sock");
        let mut staging_path = None;
        bind_ipc_listener_with(&path, |bound_path| {
            std::fs::write(bound_path, b"socket stand-in")?;
            // Force the permissive creation mode independently of the test umask.
            std::fs::set_permissions(bound_path, std::fs::Permissions::from_mode(0o755))?;
            assert!(!path.exists(), "endpoint must not be published before hardening");
            let parent = bound_path.parent().unwrap();
            assert_eq!(std::fs::metadata(parent)?.permissions().mode() & 0o777, 0o700);
            assert!(bound_path.as_os_str().len() <= path.as_os_str().len());
            staging_path = Some(parent.to_path_buf());
            Ok(())
        })
        .unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(std::fs::read(&path).unwrap(), b"socket stand-in");
        assert!(!staging_path.unwrap().exists(), "staging directory must be cleaned");
    }

    #[test]
    fn ipc_publication_fails_closed_when_hardening_fails() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("ipc.sock");
        // A dangling symlink makes chmod fail while rename would succeed.
        // Ignoring the chmod error must therefore be caught by this test.
        let result = bind_ipc_listener_with(&path, |bound_path| {
            std::os::unix::fs::symlink(dir.path().join("missing"), bound_path)
        });
        assert!(result.is_err(), "chmod failure must prevent a ready listener");
        assert!(!path.exists());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn ipc_publication_cleans_up_when_bind_or_rename_fails() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("ipc.sock");
        let result = bind_ipc_listener_with::<()>(&path, |_| {
            Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied))
        });
        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::PermissionDenied);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);

        std::fs::create_dir(&path).unwrap(); // a directory cannot be replaced by the socket
        let result = bind_ipc_listener_with(&path, |bound_path| std::fs::write(bound_path, b""));
        assert!(result.is_err());
        assert!(path.is_dir());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[tokio::test]
    async fn test_bind_ipc_listener_succeeds_and_chmods_0600_on_a_valid_path() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempdir().unwrap();
        let sock_path = dir.path().join("valid.sock");

        let listener = bind_ipc_listener(&sock_path).expect("bind must succeed on a valid, writable path");
        assert!(sock_path.exists(), "the socket file must exist after a successful bind");
        let mode = std::fs::metadata(&sock_path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "the socket file must be chmod 0o600, got {mode:o}");

        // The synchronous return is the bind+harden completion barrier. Also
        // prove the published pathname reaches the listener after its rename.
        let client = UnixStream::connect(&sock_path).await.unwrap();
        let (_accepted, _) = tokio::time::timeout(std::time::Duration::from_secs(3), listener.accept())
            .await
            .unwrap()
            .unwrap();
        drop(client);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
        drop(listener);
    }

    // ------------------------------------------------------------------
    // Task 1697: capabilities completion + item metadata. RED-first: these
    // were seen failing before the CAP_REPARENT/CAP_TRASH bits and the
    // dates/childItemCount/version-split fields existed.
    // ------------------------------------------------------------------

    fn live_file_entry(status: FileStatus) -> FileEntry {
        FileEntry {
            file_id: "1697-item".into(),
            path: "/docs/report.txt".into(),
            status,
            size_bytes: 120,
            modified_at: 1_700_000_100,
            content_hash: None,
            remote_updated_at: 1_700_000_200,
            parent_id: None,
            item_kind: ItemKind::File,
        }
    }

    fn live_contract() -> crate::state_db::FileContractState {
        crate::state_db::FileContractState {
            file_id: "1697-item".into(),
            namespace: crate::state_db::Namespace::MyFiles,
            parent_id: None,
            shared_root_id: None,
            share_id: None,
            owner_email: None,
            permission_bits: 0,
            item_kind: crate::state_db::ItemKind::File,
            content_type: None,
            current_version: 7,
            current_object_version_id: None,
            local_base_version: 0,
            local_hash: None,
            cache_path: None,
            cache_bytes: 0,
            pin_state: crate::state_db::PinState::Inherit,
            inherited_pin_state: crate::state_db::PinState::Unpinned,
            last_sync_at: 0,
        }
    }

    #[test]
    fn test_1697_local_item_payload_grants_reparent_and_trash() {
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        let entry = live_file_entry(FileStatus::Local);
        db.upsert_file(&entry).unwrap();
        db.set_file_contract_state(&live_contract()).unwrap();
        let payload = file_entry_payload_for_db(&db, &entry, FP_ROOT_APPLE);
        assert_eq!(
            payload.capabilities & CAP_REPARENT,
            CAP_REPARENT,
            "a live item must advertise reparenting (drag-move), got {:#b}",
            payload.capabilities
        );
        assert_eq!(
            payload.capabilities & CAP_TRASH,
            CAP_TRASH,
            "a live item must advertise trashing, got {:#b}",
            payload.capabilities
        );
    }

    #[test]
    fn test_1697_cloud_only_item_keeps_reparent_and_trash_unset() {
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        let entry = live_file_entry(FileStatus::CloudOnly);
        db.upsert_file(&entry).unwrap();
        db.set_file_contract_state(&live_contract()).unwrap();
        let payload = file_entry_payload_for_db(&db, &entry, FP_ROOT_APPLE);
        assert_eq!(payload.capabilities & CAP_REPARENT, 0, "read-only terminal states stay read-only");
        assert_eq!(payload.capabilities & CAP_TRASH, 0, "read-only terminal states stay read-only");
    }

    #[test]
    fn test_1697_trashing_item_keeps_reparent_and_trash_unset() {
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        let entry = live_file_entry(FileStatus::Trashing);
        db.upsert_file(&entry).unwrap();
        db.set_file_contract_state(&live_contract()).unwrap();
        let payload = file_entry_payload_for_db(&db, &entry, FP_ROOT_APPLE);
        assert_eq!(
            payload.capabilities & (CAP_REPARENT | CAP_TRASH),
            0,
            "an item on its way out cannot be moved or trashed again"
        );
    }

    #[test]
    fn test_1697_payload_carries_dates_and_folder_child_count() {
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        db.upsert_file(&FileEntry {
            file_id: "folder-1697".into(),
            path: "/docs".into(),
            status: FileStatus::Local,
            size_bytes: 0,
            modified_at: 1,
            content_hash: None,
            remote_updated_at: 1,
            parent_id: None,
            item_kind: ItemKind::Folder,
        })
        .unwrap();
        // item_kind is contract-owned (upsert_file never writes it) — the
        // contract write is what makes this row a folder.
        db.set_file_contract_state(&crate::state_db::FileContractState {
            file_id: "folder-1697".into(),
            item_kind: ItemKind::Folder,
            ..live_contract()
        })
        .unwrap();
        for (id, path) in [("c-1", "/docs/a.txt"), ("c-2", "/docs/b.txt")] {
            db.upsert_file(&FileEntry {
                file_id: id.into(),
                path: path.into(),
                status: FileStatus::CloudOnly,
                size_bytes: 1,
                modified_at: 1,
                content_hash: None,
                remote_updated_at: 1,
                parent_id: None,
                item_kind: ItemKind::File,
            })
            .unwrap();
            db.set_file_contract_state(&crate::state_db::FileContractState {
                file_id: id.into(),
                parent_id: Some("folder-1697".into()),
                ..live_contract()
            })
            .unwrap();
        }
        let folder = db.get_file("folder-1697").unwrap().unwrap();
        let payload = file_entry_payload_for_db(&db, &folder, FP_ROOT_APPLE);
        assert_eq!(
            payload.child_item_count,
            Some(2),
            "childItemCount must be the real child count, was hardcoded 0 before 1697"
        );
        assert!(payload.modified_at.is_some(), "modification date must be populated");
        assert!(payload.created_at.is_some(), "creation date must be populated");
        let file = db.get_file("c-1").unwrap().unwrap();
        let file_payload = file_entry_payload_for_db(&db, &file, "folder-1697");
        assert_eq!(file_payload.child_item_count, None, "files have no child count");
    }

    #[test]
    fn test_1697_version_split_content_vs_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        let mut entry = live_file_entry(FileStatus::Local);
        entry.content_hash = Some("abc123".into());
        db.upsert_file(&entry).unwrap();
        db.set_file_contract_state(&live_contract()).unwrap();

        let before = file_entry_payload_for_db(&db, &entry, FP_ROOT_APPLE);
        // A rename: metadata changes, content does not.
        let mut renamed = entry.clone();
        renamed.path = "/docs/report-renamed.txt".into();
        db.upsert_file(&renamed).unwrap();
        let after = file_entry_payload_for_db(&db, &renamed, FP_ROOT_APPLE);
        assert_eq!(
            after.content_version, before.content_version,
            "a rename must not change the content version (it would force a re-download)"
        );
        assert_ne!(
            after.metadata_version, before.metadata_version,
            "a rename must change the metadata version"
        );
        assert_ne!(
            before.content_version, before.metadata_version,
            "the two versions are distinct identities"
        );
    }
}
