//! Unix-domain-socket IPC between the desktop daemon and the macOS File
//! Provider extension (and the Linux FUSE mount). Unix sockets do not
//! exist on Windows, where the Cloud Files callback runs in-process
//! (see `crate::windows_cf`), so the whole module is gated to `unix`.

#![cfg(unix)]

use crate::ipc_frame::{FrameError, FrameReader, MAX_REQUEST_BYTES, write_frame};
use crate::ipc_write_dedup::{MAX_KEY_BYTES, WriteDedup};
use serde::{Deserialize, Serialize};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

#[derive(Debug, Serialize, Deserialize)]
pub enum IpcRequest {
    GetFileStatus {
        file_id: String,
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
        /// The system recreates an item it could not delete because the person edited it
        /// (`NSFileProviderCreateItemDeletionConflicted`, spec 2026-10-09 §7.2). Absent from
        /// an older extension, which then gets an ordinary create.
        #[serde(default)]
        deletion_conflicted: bool,
        /// With `deletion_conflicted`: the template's identifier, the item the delete was for.
        #[serde(default)]
        template_identifier: Option<String>,
        /// With `deletion_conflicted`: the template's content version, the base of the edit.
        #[serde(default)]
        template_content_version: Option<String>,
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
    /// Task 1699: fetch the server's encrypted thumbnail variant for one
    /// file, decrypt it, and stage the plaintext at `dest_path` (bounded to
    /// the allowed roots, written `.part` + rename). The extension reads the
    /// staging file back and deletes it, mirroring the hydrate flow.
    FetchThumbnail {
        file_id: String,
        dest_path: String,
        /// Requested maximum pixel dimension; picks the server variant
        /// (`thumbnail_variant`, mirroring the Windows Cloud Files picker).
        max_dimension: u32,
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
    /// A create or modify the app could not take NOW: nothing was staged or
    /// queued, and the person's file is unchanged on disk (spec
    /// 2026-10-09 §8.4: the app's staging folder cannot be created or
    /// written to). The extension reports it as a transient error
    /// (`uploadStagingFailed`, `serverUnreachable`), so the system keeps the
    /// change and retries the write; an `Error` would reach the system as
    /// `cannotSynchronize`, which it does not retry until the item changes
    /// again. `message` is a fixed reason, never a path.
    WriteRetryLater {
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
    /// Task 1699: `FetchThumbnail` succeeded; the decrypted thumbnail
    /// plaintext was staged atomically at the requested destination.
    /// `size_bytes` is the plaintext byte count (the extension reads the
    /// staging file back and deletes it).
    ThumbnailWritten {
        size_bytes: u64,
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
    /// Task 1698: the row's effective pin state (own `pin_state`, else
    /// `inherited_pin_state`). Swift maps it to
    /// `.downloadEagerlyAndKeepDownloaded`; the root's `.downloadLazily`
    /// governs everything else. The cross-device pin backend is task 1683 —
    /// the pin set is this device's own today (deviation 1, task 1698).
    #[serde(default)]
    pub pinned: bool,
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
/// Task 1698: the SYSTEM trash container (macOS Trash / `supportsSyncingTrash`,
/// default YES — ruling `.claude/tasks/decisions/finder-trash-full-sync.md`).
/// Trashed items are presented as children of THIS container, outside the
/// single-root ruling (the trash view is system-managed, not a synthetic
/// folder under "Beebeeb").
pub(crate) const FP_TRASH_APPLE: &str = "NSFileProviderTrashContainerItemIdentifier";
// Task 1701: the OTHER namespace constants (`my_files`, `offline`,
// `conflicts`) are gone with the synthetic containers. This one survives as
// a WRITE-path guard only: a stale Finder client can still drop onto a
// cached `namespace:shared_with_me` folder, and that drop must be refused
// (read-only), not silently re-parented to the vault root by
// `normalize_parent_id`. It is never an enumeration surface any more.
const NAMESPACE_SHARED_WITH_ME: &str = "namespace:shared_with_me";
const CAP_READ: u32 = 1 << 0;
pub(crate) const CAP_WRITE: u32 = 1 << 1;
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
/// sides. Trash SEMANTICS landed in task 1698 (ruling: full trash sync —
/// the trash-container reparent maps to the server trash op).
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

#[cfg(all(target_os = "macos", not(test)))]
pub fn macos_hydrate_cache_dir() -> std::path::PathBuf {
    macos_hydrate_cache_dir_in(&macos_real_home_dir())
}

/// A test build resolves a per-process sandbox with the same
/// `Library/Group Containers/<group>/hydrate-cache` shape, never the real App Group directory
/// the installed app shares (`getpwuid`-resolved, so `$HOME` cannot redirect it). Without this,
/// 9 tests ran the production code that creates the real dir and TTL-sweeps it (the hydrate and
/// thumbnail handlers) or empties it (`purge_macos_hydrate_cache`: sign-out and lock). Pinned
/// by `unit_tests_resolve_the_hydrate_dir_in_a_sandbox_never_the_real_group_container`.
#[cfg(all(target_os = "macos", test))]
pub fn macos_hydrate_cache_dir() -> std::path::PathBuf {
    let home = crate::test_sandbox::dir("real-home").expect("create the unit-test sandbox");
    macos_hydrate_cache_dir_in(&home)
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
    macos_ensure_private_staging_dir(dir)
}

/// Create (if needed) and harden a staging directory: owner-only `0o700` and
/// excluded from backups. Shared by the App Group hydrate-cache and
/// upload-staging directory ([`macos_upload_staging_dir`]), which only ever
/// hold short-lived plaintext, and the engine's own Finder-write staging root
/// (`engine_bridge::durable_finder_staging_root`), which holds a save's copy
/// until it uploads.
#[cfg(target_os = "macos")]
pub(crate) fn macos_ensure_private_staging_dir(dir: &std::path::Path) -> std::io::Result<()> {
    macos_open_private_staging_dir(dir)
        .map(|_| ())
        .map_err(StagingDirRefusal::into_io_error)
}

/// [`macos_ensure_private_staging_dir`], returning the directory held open.
///
/// The directory is opened once, without following a symlink, and must be a
/// real directory owned by this user ([`StagingDir::open`]). The mode and
/// the backup exclusion are then set through that descriptor, so a symlink
/// put in the directory's place can never redirect them to another folder.
#[cfg(target_os = "macos")]
fn macos_open_private_staging_dir(dir: &std::path::Path) -> Result<StagingDir, StagingDirRefusal> {
    macos_open_private_staging_dir_with(dir, macos_exclude_from_backups)
}

/// [`macos_open_private_staging_dir`] with the backup exclusion as a
/// parameter, so a test can make it fail.
#[cfg(target_os = "macos")]
fn macos_open_private_staging_dir_with(
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
fn macos_exclude_from_backups(dir: &StagingDir) -> std::io::Result<()> {
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
        if path.file_name().map(macos_is_hydrate_cache_temp_name).unwrap_or(false) {
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

// ── Upload staging: write contents handed over by the File Provider extension ──
//
// The system gives the extension's `createItem` / `modifyItem` a contents URL
// that only the extension's sandbox can read. The daemon is a different
// sandboxed process and cannot open it, so every Finder file create used to
// fail with "Upload source is not a file". The extension now copies the
// contents into this App Group directory first and sends that copy's path
// (`UploadStaging` in `BeebeebFileProvider/UploadStaging.swift`). The daemon
// accepts write contents from this directory only, copies them into its own
// staging (`StagedPayload::copy`), and deletes the handed-over copy once the
// request is answered.

/// Subdirectory of the App Group container that holds write contents handed
/// over by the extension. Mirrors `UploadStaging.directoryName` on the Swift
/// side; keep both in sync.
#[cfg(target_os = "macos")]
const MACOS_UPLOAD_STAGING_DIRNAME: &str = "upload-staging";

#[cfg(target_os = "macos")]
fn macos_upload_staging_dir_in(home_dir: &std::path::Path) -> std::path::PathBuf {
    home_dir
        .join("Library")
        .join("Group Containers")
        .join(MACOS_APP_GROUP_ID)
        .join(MACOS_UPLOAD_STAGING_DIRNAME)
}

#[cfg(target_os = "macos")]
pub fn macos_upload_staging_dir() -> std::path::PathBuf {
    macos_upload_staging_dir_in(&macos_real_home_dir())
}

/// How old a handed-over copy must be before the purge removes it. Each copy
/// is deleted by both sides as soon as its request is answered, so anything
/// left here was orphaned by a crash. The bound must exceed the longest a
/// request can legitimately be in flight: the extension's write timeout is
/// 600 s (`IPCFraming.stagedCopyTimeoutSeconds`). An hour is six times that.
///
/// Age-bound rather than "purge everything at startup": the extension runs
/// independently of the daemon and can stage a copy at any moment, including
/// while the daemon is starting, so the daemon cannot know that nothing is in
/// flight. Age is measured from each copy's ctime, which the kernel sets
/// when the copy is made and no process can set back.
#[cfg(any(target_os = "macos", test))]
pub(crate) const UPLOAD_STAGING_MAX_AGE: std::time::Duration = std::time::Duration::from_secs(60 * 60);

/// Why a staging directory itself was refused. Logged as this fixed
/// category, never with the directory's path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StagingDirRefusal {
    /// Nothing at the path.
    Missing,
    /// A symlink, not a directory, or owned by another user.
    NotPrivate,
    /// It could not be opened or inspected.
    Unavailable(std::io::ErrorKind),
}

impl StagingDirRefusal {
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    pub(crate) fn category(self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::NotPrivate => "not_private",
            Self::Unavailable(_) => "unavailable",
        }
    }

    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    pub(crate) fn into_io_error(self) -> std::io::Error {
        match self {
            Self::Missing => std::io::ErrorKind::NotFound.into(),
            Self::NotPrivate => std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "staging directory is not a private directory",
            ),
            Self::Unavailable(kind) => kind.into(),
        }
    }
}

/// A staging directory opened once, without following a symlink, and checked
/// to be a real directory owned by this user. Listing and deleting its
/// entries goes through this descriptor (`fstatat`, `unlinkat`), so renaming
/// the directory away and putting a symlink in its place cannot redirect
/// them to another folder.
pub(crate) struct StagingDir {
    fd: std::os::fd::OwnedFd,
    /// Device and inode of the opened directory, to recognise it by.
    dev: u64,
    ino: u64,
}

impl StagingDir {
    /// Open `path` as a staging directory owned by this process's user.
    pub(crate) fn open(path: &std::path::Path) -> Result<Self, StagingDirRefusal> {
        // SAFETY: `geteuid` has no preconditions and cannot fail.
        Self::open_owned_by(path, unsafe { libc::geteuid() })
    }

    /// [`StagingDir::open`] with the expected owner as a parameter, so the
    /// owner check is testable without a second account.
    fn open_owned_by(path: &std::path::Path, owner: libc::uid_t) -> Result<Self, StagingDirRefusal> {
        use std::os::fd::{AsRawFd, FromRawFd};
        use std::os::unix::ffi::OsStrExt;

        let c_path = std::ffi::CString::new(path.as_os_str().as_bytes())
            .map_err(|_| StagingDirRefusal::Unavailable(std::io::ErrorKind::InvalidInput))?;
        // SAFETY: `c_path` is NUL-terminated and lives for the call.
        let raw = unsafe {
            libc::open(
                c_path.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if raw < 0 {
            let error = std::io::Error::last_os_error();
            return Err(match error.raw_os_error() {
                Some(libc::ENOENT) => StagingDirRefusal::Missing,
                // ELOOP: the final component is a symlink (O_NOFOLLOW).
                // ENOTDIR: it is not a directory.
                Some(libc::ELOOP) | Some(libc::ENOTDIR) => StagingDirRefusal::NotPrivate,
                _ => StagingDirRefusal::Unavailable(error.kind()),
            });
        }
        // SAFETY: `raw` is a descriptor we just opened and own.
        let fd = unsafe { std::os::fd::OwnedFd::from_raw_fd(raw) };
        let stat = fstat_fd(fd.as_raw_fd()).map_err(|e| StagingDirRefusal::Unavailable(e.kind()))?;
        if !is_dir_mode(stat.st_mode) || stat.st_uid != owner {
            return Err(StagingDirRefusal::NotPrivate);
        }
        // `as u64` matches `MetadataExt::dev()` / `ino()` on every unix.
        #[allow(clippy::unnecessary_cast)]
        Ok(Self {
            fd,
            dev: stat.st_dev as u64,
            ino: stat.st_ino as u64,
        })
    }

    /// Force the directory to owner-only `0o700`, through the descriptor.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    fn restrict_to_owner(&self) -> std::io::Result<()> {
        use std::os::fd::AsRawFd;
        // SAFETY: the descriptor is open for the call.
        if unsafe { libc::fchmod(self.fd.as_raw_fd(), 0o700) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }

    /// The names directly inside, without `.` and `..`.
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    pub(crate) fn entry_names(&self) -> std::io::Result<Vec<std::ffi::OsString>> {
        use std::os::fd::AsRawFd;
        list_dir_fd(self.fd.as_raw_fd())
    }

    /// `lstat` of one entry: a symlink is described, never followed.
    pub(crate) fn stat_entry(&self, name: &std::ffi::OsStr) -> std::io::Result<libc::stat> {
        use std::os::fd::AsRawFd;
        fstatat_nofollow(self.fd.as_raw_fd(), name)
    }

    /// Unlink one non-directory entry (a symlink itself, never its target).
    pub(crate) fn unlink_entry(&self, name: &std::ffi::OsStr) -> std::io::Result<()> {
        use std::os::fd::AsRawFd;
        unlink_at(self.fd.as_raw_fd(), name, 0)
    }

    /// Remove one entry: a file or a symlink is unlinked (a symlink itself,
    /// never its target); a directory is removed with everything below it,
    /// through descriptors only ([`remove_tree_at`]).
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    fn remove_entry(&self, name: &std::ffi::OsStr, stat: &libc::stat) -> std::io::Result<()> {
        use std::os::fd::AsRawFd;
        if is_dir_mode(stat.st_mode) {
            remove_tree_at(self.fd.as_raw_fd(), name, 0)
        } else {
            unlink_at(self.fd.as_raw_fd(), name, 0)
        }
    }
}

/// How deep [`remove_tree_at`] descends before it gives up on a tree. The
/// extension only ever stages single files; this only bounds a stray tree.
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
const STAGING_TREE_MAX_DEPTH: usize = 32;

fn is_dir_mode(mode: libc::mode_t) -> bool {
    mode & libc::S_IFMT == libc::S_IFDIR
}

fn fstat_fd(fd: std::os::fd::RawFd) -> std::io::Result<libc::stat> {
    let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
    // SAFETY: `stat` is writable storage of the right type; the descriptor is
    // open for the call.
    if unsafe { libc::fstat(fd, stat.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: `fstat` returned 0, so it filled `stat` in.
    Ok(unsafe { stat.assume_init() })
}

fn entry_cstring(name: &std::ffi::OsStr) -> std::io::Result<std::ffi::CString> {
    use std::os::unix::ffi::OsStrExt;
    std::ffi::CString::new(name.as_bytes())
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "name contains a NUL byte"))
}

fn fstatat_nofollow(dir_fd: std::os::fd::RawFd, name: &std::ffi::OsStr) -> std::io::Result<libc::stat> {
    let c_name = entry_cstring(name)?;
    let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
    // SAFETY: `c_name` is NUL-terminated and lives for the call; `stat` is
    // writable storage of the right type; the descriptor is open.
    if unsafe { libc::fstatat(dir_fd, c_name.as_ptr(), stat.as_mut_ptr(), libc::AT_SYMLINK_NOFOLLOW) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: `fstatat` returned 0, so it filled `stat` in.
    Ok(unsafe { stat.assume_init() })
}

fn unlink_at(dir_fd: std::os::fd::RawFd, name: &std::ffi::OsStr, flags: libc::c_int) -> std::io::Result<()> {
    let c_name = entry_cstring(name)?;
    // SAFETY: `c_name` is NUL-terminated and lives for the call; the
    // descriptor is open.
    if unsafe { libc::unlinkat(dir_fd, c_name.as_ptr(), flags) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// The names directly inside the directory open at `dir_fd`, without `.`
/// and `..`. Reads a duplicate of the descriptor, so `dir_fd` stays usable.
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
fn list_dir_fd(dir_fd: std::os::fd::RawFd) -> std::io::Result<Vec<std::ffi::OsString>> {
    use std::os::unix::ffi::OsStrExt;

    // SAFETY: duplicating an open descriptor has no other preconditions.
    let dup = unsafe { libc::fcntl(dir_fd, libc::F_DUPFD_CLOEXEC, 0) };
    if dup < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: `dup` is an open directory descriptor we own; on success
    // `fdopendir` takes it over and `closedir` below closes it.
    let dirp = unsafe { libc::fdopendir(dup) };
    if dirp.is_null() {
        let error = std::io::Error::last_os_error();
        // SAFETY: `fdopendir` failed, so `dup` is still ours to close.
        unsafe { libc::close(dup) };
        return Err(error);
    }
    // The duplicate shares the read position with `dir_fd`: start over.
    // SAFETY: `dirp` is a valid stream until `closedir`.
    unsafe { libc::rewinddir(dirp) };
    let mut names = Vec::new();
    loop {
        // SAFETY: `dirp` is a valid stream; the entry it returns stays valid
        // until the next `readdir` on it, and is copied out before that.
        let entry = unsafe { libc::readdir(dirp) };
        if entry.is_null() {
            break;
        }
        // SAFETY: `d_name` is NUL-terminated within the entry.
        let name = unsafe { std::ffi::CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
        if name != b"." && name != b".." {
            names.push(std::ffi::OsStr::from_bytes(name).to_os_string());
        }
    }
    // SAFETY: `dirp` came from `fdopendir` and is closed exactly once.
    unsafe { libc::closedir(dirp) };
    Ok(names)
}

/// Remove the directory `name` inside `parent_fd` and everything below it,
/// through descriptors only: each level is opened with `O_NOFOLLOW`, and a
/// symlink anywhere in the tree is unlinked, never followed.
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
fn remove_tree_at(parent_fd: std::os::fd::RawFd, name: &std::ffi::OsStr, depth: usize) -> std::io::Result<()> {
    use std::os::fd::{AsRawFd, FromRawFd};

    if depth >= STAGING_TREE_MAX_DEPTH {
        return Err(std::io::Error::other("staging tree too deep"));
    }
    let c_name = entry_cstring(name)?;
    // SAFETY: `c_name` is NUL-terminated and lives for the call; the parent
    // descriptor is open.
    let raw = unsafe {
        libc::openat(
            parent_fd,
            c_name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if raw < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: `raw` is a descriptor we just opened and own.
    let dir = unsafe { std::os::fd::OwnedFd::from_raw_fd(raw) };
    // A directory without its search bit (an older extension set a copied
    // package to 0600) lets nothing inside it be inspected or unlinked. We
    // own it, so give it back to ourselves first.
    // SAFETY: the descriptor is open for the call.
    if unsafe { libc::fchmod(dir.as_raw_fd(), 0o700) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    for child in list_dir_fd(dir.as_raw_fd())? {
        let stat = fstatat_nofollow(dir.as_raw_fd(), &child)?;
        if is_dir_mode(stat.st_mode) {
            remove_tree_at(dir.as_raw_fd(), &child, depth + 1)?;
        } else {
            unlink_at(dir.as_raw_fd(), &child, 0)?;
        }
    }
    drop(dir);
    unlink_at(parent_fd, name, libc::AT_REMOVEDIR)
}

/// `SystemTime` of a `stat` timestamp; `None` before the epoch.
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
fn stat_time(secs: libc::time_t, nanos: libc::c_long) -> Option<std::time::SystemTime> {
    let secs = u64::try_from(secs).ok()?;
    let nanos = u32::try_from(nanos).ok()?;
    std::time::UNIX_EPOCH.checked_add(std::time::Duration::new(secs, nanos))
}

/// Where `QueueFinderCreate` / `QueueFinderModify` may take file contents from.
#[derive(Debug, Clone)]
pub(crate) enum WriteContentsPolicy {
    /// Any path the daemon can read, left in place. Linux (the daemon is not
    /// sandboxed and no client sends writes over this socket) and tests that
    /// predate the staging directory.
    #[cfg_attr(all(target_os = "macos", not(test)), allow(dead_code))]
    AnyPath,
    /// Only a regular file directly inside this directory, deleted by the
    /// daemon once the request is answered. macOS (and the socket tests on
    /// every platform).
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    StagingDir(std::path::PathBuf),
}

impl WriteContentsPolicy {
    pub(crate) fn platform_default() -> Self {
        #[cfg(target_os = "macos")]
        {
            Self::StagingDir(macos_upload_staging_dir())
        }
        #[cfg(not(target_os = "macos"))]
        {
            Self::AnyPath
        }
    }
}

/// Why a write's `contents_path` was refused. Logged and returned by
/// [`ContentsRefusal::category`] only: never the path itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ContentsRefusal {
    /// Empty, relative, or containing a NUL byte.
    MalformedPath,
    /// Contains a `..` or `.` segment.
    Traversal,
    /// The staging directory is missing, is a symlink, or cannot be resolved.
    StagingUnavailable,
    /// Resolves outside the staging directory, or below a subdirectory of it.
    OutsideStaging,
    /// Nothing at the path.
    Missing,
    /// The path is a symbolic link.
    Symlink,
    /// Not a regular file (a directory, FIFO, socket, ...).
    NotAFile,
    /// A regular file with more than one link: a hard link to a file
    /// elsewhere, not a copy the extension made.
    HardLinked,
    /// A regular file owned by another user.
    ForeignOwner,
    /// The entry could not be inspected.
    Unreadable,
}

impl ContentsRefusal {
    pub(crate) fn category(self) -> &'static str {
        match self {
            Self::MalformedPath => "malformed_path",
            Self::Traversal => "traversal",
            Self::StagingUnavailable => "staging_unavailable",
            Self::OutsideStaging => "outside_staging",
            Self::Missing => "missing",
            Self::Symlink => "symlink",
            Self::NotAFile => "not_a_file",
            Self::HardLinked => "hard_linked",
            Self::ForeignOwner => "foreign_owner",
            Self::Unreadable => "unreadable",
        }
    }
}

/// Open `candidate` only if it names a regular file directly inside
/// `staging_dir`, and hold it open.
///
/// The staging directory is opened first ([`StagingDir::open`]: never
/// through a symlink, owned by this user). The candidate's parent must be
/// that same directory (device and inode), and the leaf is then opened
/// RELATIVE to the held descriptor with `O_NOFOLLOW | O_NONBLOCK`: a symlink
/// is refused at the open itself and a FIFO cannot block it. Every check that
/// decides acceptance runs on the opened descriptor
/// ([`check_staged_contents_stat`]), and the daemon reads the contents from
/// that descriptor only. Swapping the entry after this returns therefore
/// changes nothing the daemon reads.
pub(crate) fn open_staged_contents(
    staging_dir: &std::path::Path,
    candidate: &str,
) -> Result<OpenedContents, ContentsRefusal> {
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::fs::MetadataExt;

    let path = std::path::Path::new(candidate);
    if candidate.is_empty() || candidate.contains('\0') || !path.is_absolute() {
        return Err(ContentsRefusal::MalformedPath);
    }
    // Checked on the raw text: `Path::components` silently drops interior
    // `.` segments, and the extension never sends either.
    if candidate.split('/').any(|segment| segment == ".." || segment == ".") {
        return Err(ContentsRefusal::Traversal);
    }
    // The configured directory itself must be a real directory owned by this
    // user: if it were replaced by a symlink, another folder's files would
    // become acceptable, and the daemon deletes what it accepts.
    let dir = StagingDir::open(staging_dir).map_err(|_| ContentsRefusal::StagingUnavailable)?;
    let (Some(parent), Some(leaf)) = (path.parent(), path.file_name()) else {
        return Err(ContentsRefusal::OutsideStaging);
    };
    // Directly inside only: the parent must be the held directory itself.
    match std::fs::metadata(parent) {
        Ok(meta) if meta.dev() == dir.dev && meta.ino() == dir.ino => {}
        _ => return Err(ContentsRefusal::OutsideStaging),
    }
    let c_leaf = entry_cstring(leaf).map_err(|_| ContentsRefusal::MalformedPath)?;
    // SAFETY: `c_leaf` is NUL-terminated and lives for the call; the
    // directory descriptor is open.
    let raw = unsafe {
        libc::openat(
            dir.fd.as_raw_fd(),
            c_leaf.as_ptr(),
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
        )
    };
    if raw < 0 {
        let error = std::io::Error::last_os_error();
        return Err(match error.raw_os_error() {
            Some(libc::ENOENT) => ContentsRefusal::Missing,
            // O_NOFOLLOW on a symlink.
            Some(libc::ELOOP) => ContentsRefusal::Symlink,
            // A socket cannot be opened.
            Some(libc::ENXIO) | Some(libc::EOPNOTSUPP) => ContentsRefusal::NotAFile,
            _ => ContentsRefusal::Unreadable,
        });
    }
    // SAFETY: `raw` is a descriptor we just opened and own.
    let fd = unsafe { std::os::fd::OwnedFd::from_raw_fd(raw) };
    let stat = fstat_fd(fd.as_raw_fd()).map_err(|_| ContentsRefusal::Unreadable)?;
    // SAFETY: `geteuid` has no preconditions and cannot fail.
    check_staged_contents_stat(&stat, unsafe { libc::geteuid() })?;
    // A label for the engine's journal, rebuilt from the canonical staging
    // dir: the daemon never reads through it.
    let label = std::fs::canonicalize(staging_dir)
        .map_err(|_| ContentsRefusal::StagingUnavailable)?
        .join(leaf);
    #[allow(clippy::unnecessary_cast)]
    let identity = (stat.st_dev as u64, stat.st_ino as u64);
    Ok(OpenedContents {
        path: label,
        file: std::fs::File::from(fd),
        dir,
        leaf: leaf.to_os_string(),
        identity,
    })
}

/// A handed-over contents file accepted by [`open_staged_contents`], held
/// open. Opening it changes nothing on disk.
pub(crate) struct OpenedContents {
    /// A label for the engine's journal; the daemon never reads through it.
    path: std::path::PathBuf,
    /// The opened file: what the engine reads.
    file: std::fs::File,
    /// The staging directory it was opened in, and its name there: the
    /// deletion goes through these, never through a path.
    dir: StagingDir,
    leaf: std::ffi::OsString,
    /// Device and inode of the opened file.
    identity: (u64, u64),
}

/// The checks on an opened handed-over file: a regular file with exactly one
/// link (the extension's copy is always a fresh file with one), owned by
/// `owner`. A hard link to another file, a FIFO, a device or a directory is
/// refused. Split out so the checks are testable on a constructed `stat`.
fn check_staged_contents_stat(stat: &libc::stat, owner: libc::uid_t) -> Result<(), ContentsRefusal> {
    if stat.st_mode & libc::S_IFMT != libc::S_IFREG {
        return Err(ContentsRefusal::NotAFile);
    }
    if stat.st_nlink != 1 {
        return Err(ContentsRefusal::HardLinked);
    }
    if stat.st_uid != owner {
        return Err(ContentsRefusal::ForeignOwner);
    }
    Ok(())
}

/// An admitted handed-over contents file. The engine copies from
/// [`StagedContents::file`], never by path. Dropping it deletes the file: by
/// then the daemon holds its own copy (`StagedPayload::copy_from_file`), or
/// the request failed and the extension's retry stages a fresh one.
///
/// The deletion is `unlinkat` on the held staging-directory descriptor, and
/// only while the name still refers to the file that was handed over (same
/// device and inode, checked with `fstatat` without following a link).
/// Renaming the staging directory away and putting a symlink in its place,
/// or putting another file under the copy's name, therefore deletes
/// nothing else; a name left behind is the age-bound purge's.
pub(crate) struct StagedContents {
    contents: OpenedContents,
}

impl StagedContents {
    fn path_string(&self) -> String {
        self.contents.path.to_string_lossy().into_owned()
    }

    /// The opened file: what the engine reads.
    pub(crate) fn file(&self) -> &std::fs::File {
        &self.contents.file
    }
}

impl Drop for StagedContents {
    fn drop(&mut self) {
        let contents = &self.contents;
        let result = match contents.dir.stat_entry(&contents.leaf) {
            #[allow(clippy::unnecessary_cast)]
            Ok(stat) if (stat.st_dev as u64, stat.st_ino as u64) == contents.identity => {
                contents.dir.unlink_entry(&contents.leaf)
            }
            // The name now refers to something else: not ours to delete.
            Ok(_) => Ok(()),
            Err(e) => Err(e),
        };
        match result {
            Ok(()) => {}
            // The extension deletes its copy after the reply too; either side may be first.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => tracing::warn!(
                error_kind = ?e.kind(),
                "could not delete a handed-over upload copy; the age-bound purge will"
            ),
        }
    }
}

/// Apply `policy` to one write request's `contents_path`. `Ok` carries the
/// path to hand the engine and, under [`WriteContentsPolicy::StagingDir`],
/// the guard that deletes the handed-over copy when dropped. `Err` is the
/// refusal, already logged; [`contents_refusal_response`] is its reply.
fn admit_write_contents(
    policy: &WriteContentsPolicy,
    op: &'static str,
    contents_path: Option<String>,
) -> Result<(Option<String>, Option<StagedContents>), ContentsRefusal> {
    let (WriteContentsPolicy::StagingDir(staging_dir), Some(candidate)) = (policy, contents_path.as_deref()) else {
        return Ok((contents_path, None));
    };
    match open_staged_contents(staging_dir, candidate) {
        Ok(contents) => {
            let staged = StagedContents { contents };
            Ok((Some(staged.path_string()), Some(staged)))
        }
        Err(refusal) => {
            log_refused_write(op, refusal.category());
            Err(refusal)
        }
    }
}

/// The reply for a refused `contents_path`: the category, never the path.
fn contents_refusal_response(refusal: ContentsRefusal) -> IpcResponse {
    IpcResponse::Error {
        message: format!("Upload contents refused ({})", refusal.category()),
    }
}

/// Remove upload-staging entries that have existed for at least `max_age`
/// before `now`, by their ctime (see the loop). A missing directory is not an error. `now` / `max_age` are
/// parameters so this is testable without a clock.
///
/// The directory is opened once ([`StagingDir::open`]) and refused unless it
/// is a real directory owned by this user: through a symlink, this purge
/// would delete another folder's files. Every entry is inspected and removed
/// through that descriptor.
#[cfg(any(target_os = "macos", test))]
pub(crate) fn purge_stale_upload_staging(
    dir: &std::path::Path,
    max_age: std::time::Duration,
    now: std::time::SystemTime,
) -> Result<usize, StagingDirRefusal> {
    purge_upload_staging(dir, UploadStagingPurge::OlderThan { max_age, now })
}

/// Which upload-staging entries a purge removes.
#[derive(Debug, Clone, Copy)]
#[cfg(any(target_os = "macos", test))]
pub(crate) enum UploadStagingPurge {
    /// Entries that have existed for at least `max_age` before `now`: daemon
    /// startup and the periodic sweep, while the extension may be staging.
    OlderThan {
        max_age: std::time::Duration,
        now: std::time::SystemTime,
    },
    /// Every entry: sign-out and Lock, once no IPC listener can still be
    /// serving (the engine is gone, or its stop is confirmed). A request
    /// that then reaches no daemon fails transiently and the extension
    /// stages a fresh copy for the system's retry.
    Everything,
}

/// [`purge_stale_upload_staging`] with the scope as a parameter.
#[cfg(any(target_os = "macos", test))]
pub(crate) fn purge_upload_staging(
    dir: &std::path::Path,
    scope: UploadStagingPurge,
) -> Result<usize, StagingDirRefusal> {
    match StagingDir::open(dir) {
        Ok(staging) => purge_upload_staging_in(&staging, scope),
        Err(StagingDirRefusal::Missing) => Ok(0),
        Err(refusal) => Err(refusal),
    }
}

/// [`purge_upload_staging`] on a directory already held open.
#[cfg(any(target_os = "macos", test))]
fn purge_upload_staging_in(staging: &StagingDir, scope: UploadStagingPurge) -> Result<usize, StagingDirRefusal> {
    let names = staging
        .entry_names()
        .map_err(|e| StagingDirRefusal::Unavailable(e.kind()))?;
    let mut removed = 0usize;
    for name in names {
        // `lstat` of the entry itself: a symlink is described, not followed.
        let Ok(stat) = staging.stat_entry(&name) else {
            continue;
        };
        let stale = match scope {
            UploadStagingPurge::Everything => true,
            // Aged by the status-change time (ctime), never the mtime: the
            // kernel sets ctime when the copy is made (an APFS clone keeps
            // the source's mtime), and no process can set it back. A future
            // ctime (clock change) counts as fresh; a later sweep gets it.
            UploadStagingPurge::OlderThan { max_age, now } => stat_time(stat.st_ctime, stat.st_ctime_nsec)
                .and_then(|changed| now.duration_since(changed).ok())
                .is_some_and(|age| age >= max_age),
        };
        if !stale {
            continue;
        }
        match staging.remove_entry(&name, &stat) {
            Ok(()) => removed += 1,
            Err(e) => tracing::warn!(error_kind = ?e.kind(), "upload-staging purge: could not remove an entry"),
        }
    }
    Ok(removed)
}

/// Daemon startup: create and harden the upload-staging directory, then purge
/// copies orphaned by a crash (see [`UPLOAD_STAGING_MAX_AGE`]).
#[cfg(target_os = "macos")]
pub(crate) fn macos_prepare_upload_staging_dir(
    dir: &std::path::Path,
    max_age: std::time::Duration,
    now: std::time::SystemTime,
) -> Result<usize, StagingDirRefusal> {
    let staging = macos_open_private_staging_dir(dir)?;
    purge_upload_staging_in(&staging, UploadStagingPurge::OlderThan { max_age, now })
}

/// Runner startup: prepare the real directory and purge it. Best-effort.
#[cfg(target_os = "macos")]
pub(crate) fn macos_prepare_upload_staging() {
    let dir = macos_upload_staging_dir();
    let result = macos_prepare_upload_staging_dir(&dir, UPLOAD_STAGING_MAX_AGE, std::time::SystemTime::now());
    log_upload_staging_purge("daemon-startup", result);
}

/// Runner tick: purge the real directory. Best-effort.
#[cfg(target_os = "macos")]
pub(crate) fn macos_sweep_upload_staging() {
    sweep_upload_staging_at(&macos_upload_staging_dir(), "periodic", std::time::SystemTime::now());
}

/// Age-bound purge of `dir`, logged. Split from the real-directory caller so a
/// test can run it on a temp dir.
#[cfg(any(target_os = "macos", test))]
fn sweep_upload_staging_at(dir: &std::path::Path, context: &'static str, now: std::time::SystemTime) {
    let result = purge_stale_upload_staging(dir, UPLOAD_STAGING_MAX_AGE, now);
    log_upload_staging_purge(context, result);
}

/// Sign-out and Lock: remove every entry of `dir`, logged. The caller
/// guarantees that no IPC listener can still be serving (see
/// [`UploadStagingPurge::Everything`]).
#[cfg(any(target_os = "macos", test))]
pub(crate) fn purge_all_upload_staging_at(dir: &std::path::Path, context: &'static str) {
    log_upload_staging_purge(context, purge_upload_staging(dir, UploadStagingPurge::Everything));
}

/// Log a purge: a count, or the refusal's fixed category. Never a path.
#[cfg(any(target_os = "macos", test))]
fn log_upload_staging_purge(context: &'static str, result: Result<usize, StagingDirRefusal>) {
    match result {
        Ok(removed) if removed > 0 => {
            tracing::info!(removed, context, "purged upload-staging copies");
        }
        Ok(_) => {}
        Err(refusal) => {
            tracing::warn!(
                reason = refusal.category(),
                context,
                "upload-staging purge refused or failed; nothing was removed"
            );
        }
    }
}

/// Log one refused Finder write: the operation and a fixed reason category,
/// never a file name, path or contents.
fn log_refused_write(op: &'static str, reason: &'static str) {
    tracing::warn!(op, reason, "Finder write refused");
}

/// [`log_refused_write`] for an id with no row and no alias (spec §11, "Unknown id"): it also
/// logs the id the request named, which comes from the wire (M5: as `id` when it is a UUID,
/// else as `id_kind = "non_uuid"`).
fn log_refused_unknown_item(op: &'static str, requested_id: &str) {
    let (id, id_kind) = crate::engine_bridge::wire_id_fields(requested_id);
    tracing::warn!(op, reason = "unknown_item", id, id_kind, "Finder write refused");
}

/// Fixed category for an engine error on the write path, for
/// [`log_refused_write`]. The error's text can carry user data (names, paths),
/// so it is classified, never logged.
fn write_refusal_category(error: &anyhow::Error) -> &'static str {
    if error.downcast_ref::<crate::engine_bridge::UnknownItem>().is_some() {
        return "unknown_item";
    }
    if error.is::<crate::engine_bridge::FinderStagingUnavailable>() {
        return "staging_root_unavailable";
    }
    if error.is::<crate::engine_bridge::StagedCopyGone>() {
        return "staged_copy_gone";
    }
    if error.is::<crate::engine_bridge::AcceptsClosed>() {
        return "accepts_closed";
    }
    if error.is::<crate::engine_bridge::ItemInTrash>() {
        return "in_trash";
    }
    if error.downcast_ref::<std::io::Error>().is_some() {
        return "io";
    }
    if error.downcast_ref::<rusqlite::Error>().is_some() {
        return "database";
    }
    let text = error.to_string();
    if text.contains("engine is stopping") {
        "engine_stopping"
    } else if text.contains("Upload source is not a file") {
        "contents_not_a_file"
    } else if text.contains("read-only") {
        "write_policy"
    } else if text.contains("did not include") {
        "malformed_request"
    } else {
        "other"
    }
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
pub(crate) fn macos_real_home_dir() -> std::path::PathBuf {
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
///
/// Write contents follow [`WriteContentsPolicy::platform_default`]: on macOS
/// only the App Group upload-staging directory is accepted.
pub async fn serve_ipc_at(
    path: std::path::PathBuf,
    db: std::sync::Arc<crate::state_db::StateDb>,
    bridge: std::sync::Arc<crate::engine_bridge::EngineBridge>,
    cancel: oneshot::Receiver<()>,
) -> std::io::Result<()> {
    serve_ipc_at_with_ready(path, db, bridge, cancel, None, WriteContentsPolicy::platform_default()).await
}

/// Readiness means the listener is bound, hardened and published. On startup
/// failure the sender is dropped and the server returns the original error.
/// `contents` decides where write requests may take file contents from.
pub(crate) async fn serve_ipc_at_with_ready(
    path: std::path::PathBuf,
    db: std::sync::Arc<crate::state_db::StateDb>,
    bridge: std::sync::Arc<crate::engine_bridge::EngineBridge>,
    mut cancel: oneshot::Receiver<()>,
    ready: Option<oneshot::Sender<()>>,
    contents: WriteContentsPolicy,
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
                    let contents = contents.clone();
                    connections.push(tokio::spawn(handle_connection(stream, db, bridge, write_dedup, contents)));
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
    contents_policy: WriteContentsPolicy,
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
            tracing::warn!(
                peer_uid = uid,
                daemon_uid,
                "IPC connection rejected: peer UID does not match daemon"
            );
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
            IpcRequest::GetFileStatus { file_id } => file_status_response(&db, &file_id),
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
                deletion_conflicted,
                template_identifier,
                template_content_version,
            } => {
                match admit_write_contents(&contents_policy, "create", contents_path) {
                    Err(refusal) => contents_refusal_response(refusal),
                    Ok((_, staged)) if parent_id.as_deref() == Some(NAMESPACE_SHARED_WITH_ME) => {
                        // Refused: the copy it handed over is deleted now.
                        drop(staged);
                        log_refused_write("create", "read_only_namespace");
                        // Answer and keep serving the connection (this used to
                        // `return`, silently dropping the connection).
                        IpcResponse::Error {
                            message: "Shared with me is read-only at the namespace root".into(),
                        }
                    }
                    Ok((contents_path, staged)) => {
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
                        // `staged` rides in the work closure: it is dropped (the
                        // handed-over copy deleted) once the engine has its own
                        // copy or refused, or unrun when a repeat is answered
                        // from the first attempt's result.
                        dedup_write(&write_dedup, &db, request_id, fingerprint, move || {
                            // The engine reads the opened file, never the path.
                            let opened = staged.as_ref().map(StagedContents::file);
                            // Only the macOS File Provider path mints write tokens (spec §8.8),
                            // and only it resolves a deletion-conflicted create (§7.2).
                            #[cfg(target_os = "macos")]
                            let result = if deletion_conflicted {
                                work_bridge.queue_file_provider_deletion_conflicted_create(
                                    target,
                                    template_identifier,
                                    template_content_version,
                                    opened,
                                )
                            } else {
                                work_bridge.queue_file_provider_create_from(target, opened)
                            };
                            #[cfg(not(target_os = "macos"))]
                            let result = {
                                let _ = (deletion_conflicted, template_identifier, template_content_version);
                                work_bridge
                                    .queue_finder_create_from(target, opened)
                                    .map(crate::engine_bridge::FpWrite::plain)
                            };
                            let response = write_outcome_response("create", &work_db, result);
                            drop(staged);
                            response
                        })
                        .await
                    }
                }
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
            } => match admit_write_contents(&contents_policy, "modify", contents_path) {
                Err(refusal) => contents_refusal_response(refusal),
                Ok((contents_path, staged)) => {
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
                    // See the create arm for when `staged` is dropped.
                    dedup_write(&write_dedup, &db, request_id, fingerprint, move || {
                        let opened = staged.as_ref().map(StagedContents::file);
                        // Only the macOS File Provider path mints write tokens (spec §8.8).
                        #[cfg(target_os = "macos")]
                        let result = work_bridge.queue_file_provider_modify_from(target, opened);
                        #[cfg(not(target_os = "macos"))]
                        let result = work_bridge
                            .queue_finder_modify_from(target, opened)
                            .map(crate::engine_bridge::FpWrite::plain);
                        let response = write_outcome_response("modify", &work_db, result);
                        drop(staged);
                        response
                    })
                    .await
                }
            },
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
                write_outcome_response(
                    "delete",
                    &db,
                    bridge
                        .queue_finder_delete(&file_id, base_version_identifier)
                        .map(crate::engine_bridge::FpWrite::plain),
                )
            }
            // Task 1698: the `SetRecursivePin` IPC RPC is RETIRED — it had no
            // caller (audit G13): the File Provider extension never sent it
            // and the pinning surface is the Tauri `set_recursive_pin`
            // command (Onboarding / Mac Settings), which calls
            // `EngineBridge::set_recursive_pin` directly. Task 1698 replaces
            // the concept with per-item `contentPolicy` (`.pinned` on the
            // payload → `.downloadEagerlyAndKeepDownloaded`); the pin STATE
            // itself stays on `files.pin_state` (task 1683 owns the backend).
            //
            // Task 1699 (audit G13): three more never-called RPCs are
            // RETIRED from the wire — `SetFileStatus` (a no-op stub the
            // extension never sent), `RecordOpenedFile` and
            // `EnforceSmartCache` (smart-cache bookkeeping with no File
            // Provider caller; the daemon enforces the cache limit itself
            // via `EngineBridge::enforce_configured_cache_limit`, and
            // `IpcResponse::CacheCleanup` went with them). Unknown variants
            // are refused per-request by the deserialization error path
            // above without dropping the connection; see
            // `retired_rpcs_are_refused_and_the_connection_survives`.
            IpcRequest::FetchThumbnail {
                file_id,
                dest_path,
                max_dimension,
            } => {
                // `file_id` is a raw item identifier straight off the wire;
                // validate it exactly like the hydrate arm does before it is
                // trusted for anything else.
                #[cfg(target_os = "macos")]
                let identifier_check = macos_validate_hydrate_item_identifier(&file_id);
                #[cfg(not(target_os = "macos"))]
                let identifier_check: Result<(), &'static str> = Ok(());
                let validated = match identifier_check {
                    Err(msg) => Err(IpcResponse::Error {
                        message: msg.to_string(),
                    }),
                    Ok(()) => {
                        // `dest_path` arrives straight off the wire
                        // (untrusted). Bound it to the caller's legitimate
                        // destinations — the same allowed roots as
                        // `HydrateFile` (sync root, temp dir, and on macOS
                        // the shared App Group hydrate-cache directory) —
                        // BEFORE any fetch runs, and keep the plaintext
                        // staging write atomic (`.part` + rename).
                        let allowed_roots = thumbnail_allowed_roots();
                        let dest = std::path::PathBuf::from(&dest_path);
                        match thumbnail_destination_error(&dest, &allowed_roots) {
                            Some(message) => Err(IpcResponse::Error { message }),
                            None => {
                                // `finder_thumbnail` is async (HTTP + decrypt;
                                // through the alias, and an error while the
                                // item's write is queued, spec §5.6); the
                                // staging write that follows is a small
                                // blocking file write, which the other
                                // dispatch arms also do inline.
                                let fetched = bridge
                                    .finder_thumbnail(&file_id, thumbnail_variant(max_dimension))
                                    .await;
                                match fetched {
                                    Ok(plaintext) => {
                                        match persist_thumbnail_plaintext(&dest, &allowed_roots, &plaintext) {
                                            Ok(size_bytes) => Ok(IpcResponse::ThumbnailWritten { size_bytes }),
                                            Err(e) => Ok(IpcResponse::Error {
                                                message: format!("thumbnail fetch failed: {e}"),
                                            }),
                                        }
                                    }
                                    Err(e) => Ok(IpcResponse::Error {
                                        message: format!("thumbnail fetch failed: {e}"),
                                    }),
                                }
                            }
                        }
                    }
                };
                validated.unwrap_or_else(|resp| resp)
            }
            IpcRequest::GetSyncSummary => {
                let syncing = db
                    .count_by_status(crate::state_db::FileStatus::Downloading)
                    .unwrap_or(0);
                let cloud_only = db.count_by_status(crate::state_db::FileStatus::CloudOnly).unwrap_or(0);
                let conflicts = db.count_by_status(crate::state_db::FileStatus::Conflict).unwrap_or(0);
                IpcResponse::SyncSummary {
                    syncing,
                    cloud_only,
                    conflicts,
                }
            }
            IpcRequest::ListChanges { since_anchor, limit } => {
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
                Err(e) => IpcResponse::Error { message: e.to_string() },
            },
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
        // The identifier is untrusted wire input: never logged.
        tracing::warn!(reason = "invalid_identifier", "Finder hydrate failed");
        return HydrateOutcome::Reply(IpcResponse::Error {
            message: msg.to_string(),
        });
    }
    // `dest_path` arrives straight off the wire (untrusted). Bound
    // it to the caller's legitimate destinations before handing it
    // to `hydrate_file`, which decrypts vault plaintext to disk
    // (task 1247). The only real IPC caller (the File Provider
    // extension / FUSE mount) writes either under the sync root or
    // into the per-file temp cache, so both are allowed roots; if
    // no sync root is configured yet, only the temp dir is.
    // Resolve sync_root from config (the way the retired `SetRecursivePin`
    // handler used to); on unix the value is only compared against, never
    // dereferenced by Windows-specific code.
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
        if let Err(e) =
            macos_sweep_stale_hydrate_cache_entries(&dir, MACOS_HYDRATE_CACHE_TTL, std::time::SystemTime::now())
        {
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
    // macOS: through the alias, and from the queue while the held write is queued (spec §7.4).
    // Other unix builds hydrate from the server as before.
    #[cfg(target_os = "macos")]
    let hydrate = async {
        bridge
            .serve_hydrate(file_id, dest, &allowed_roots, progress_cb)
            .await
            .map(|_| ())
    };
    #[cfg(not(target_os = "macos"))]
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
        Some(outcome) => {
            // Task 1693 — root cause of the missing terminal progress frame:
            // `report(done, total)` fires SYNCHRONOUSLY inside the hydrate
            // future's last poll, so the final chunk's frame is queued on `rx`
            // BEFORE the future returns Ready; the `biased` select polls the
            // hydrate branch first, completion outranks the queued frame, and
            // the loop above broke without ever writing it. Drain whatever the
            // future queued before answering, so the terminal (done == total)
            // frame always precedes the final reply, as the protocol promises
            // (`docs/IPC_PROTOCOL.md`: progress frames stream BEFORE the reply).
            while let Ok((done, total)) = rx.try_recv() {
                if write_frame(write_half, &IpcResponse::HydrateProgress { done, total })
                    .await
                    .is_err()
                {
                    let (file_id, id_kind) = crate::engine_bridge::wire_id_fields(file_id);
                    tracing::info!(
                        file_id,
                        id_kind,
                        "IPC client went away while flushing the terminal hydrate progress frames"
                    );
                    return HydrateOutcome::ClientGone;
                }
            }
            match outcome {
                Ok(()) => HydrateOutcome::Reply(IpcResponse::Ok {}),
                Err(e) => HydrateOutcome::Reply(hydrate_failure_reply(file_id, e)),
            }
        }
        None => {
            // `hydrate` is dropped when this function returns.
            let (file_id, id_kind) = crate::engine_bridge::wire_id_fields(file_id);
            tracing::info!(
                file_id,
                id_kind,
                "IPC client went away mid-hydrate; hydration cancelled"
            );
            HydrateOutcome::ClientGone
        }
    }
}

/// The reply to a hydrate that failed: the extension's `fetchContents` fails
/// with it, and the system shows the item as failing to download. It leaves
/// one warning in the daemon log with the item id and a fixed category; the
/// error's text can carry a path or a URL, so it is classified, never logged.
/// The id is the request's, from the wire (M5): on Linux the identifier check is a no-op,
/// so it can be any caller-supplied text.
pub(crate) fn hydrate_failure_reply(file_id: &str, error: anyhow::Error) -> IpcResponse {
    let (file_id, id_kind) = crate::engine_bridge::wire_id_fields(file_id);
    tracing::warn!(
        file_id,
        id_kind,
        reason = hydrate_failure_category(&error),
        "Finder hydrate failed"
    );
    IpcResponse::Error {
        message: error.to_string(),
    }
}

/// Fixed category for a failed hydrate, for [`hydrate_failure_reply`].
fn hydrate_failure_category(error: &anyhow::Error) -> &'static str {
    if error.is::<crate::engine_bridge::QueuedCopyMissing>() {
        return "queued_copy_missing";
    }
    if let Some(status) = crate::engine_bridge::error_http_status(error) {
        return match status {
            404 => "not_found",
            401 => "unauthorized",
            403 => "forbidden",
            409 => "conflict",
            500..=599 => "server_error",
            _ => "http_other",
        };
    }
    if error
        .chain()
        .any(|cause| cause.downcast_ref::<reqwest::Error>().is_some())
    {
        return "network";
    }
    if error
        .chain()
        .any(|cause| cause.downcast_ref::<std::io::Error>().is_some())
    {
        return "io";
    }
    if error
        .chain()
        .any(|cause| cause.downcast_ref::<rusqlite::Error>().is_some())
    {
        return "database";
    }
    "other"
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
    dedup.forget_where(
        |cached| matches!(cached, IpcResponse::WriteQueued { item: Some(item), .. } if item.identifier == file_id),
    );
}

/// Decide whether a remembered write reply still describes reality, and bring it
/// up to date. `None` means the item it created is gone, renamed, or on its way
/// to the trash, so the repeat is a NEW operation and must run.
///
/// "Up to date" matters: the reply was built when the first attempt finished,
/// and finalizing the upload since then changed the item's version and status.
/// Serving the old `version_identifier` would hand the system a stale base
/// version for the user's next edit, so the item is rebuilt from the current row.
///
/// `through_alias` (a modify's reply): an item named by a provisional id whose create
/// has landed is rebuilt from the server file and still presented under the provisional
/// id, as the modify was (spec 2026-10-09 §7.2). A create's reply never resolves through
/// the alias: its repeat after the landing runs again, as before (§12).
fn refresh_cached_write(
    db: &crate::state_db::StateDb,
    cached: IpcResponse,
    through_alias: bool,
) -> Option<IpcResponse> {
    let IpcResponse::WriteQueued {
        item: Some(item),
        ignored,
        message,
    } = cached
    else {
        return Some(cached);
    };
    let (entry, presented) = match db.get_file(&item.identifier) {
        Ok(Some(entry)) => (entry, false),
        Ok(None) if through_alias => {
            match db
                .resolve_alias(&item.identifier)
                .ok()
                .flatten()
                .and_then(|server_id| db.get_file(&server_id).ok().flatten())
            {
                Some(entry) => (entry, true),
                None => return None,
            }
        }
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
    if db.has_pending_trash(&entry.file_id).unwrap_or(false) {
        return None;
    }
    let mut fresh = file_entry_payload_for_db(db, &entry, FP_ROOT_APPLE);
    if fresh.filename != item.filename {
        return None;
    }
    if presented {
        fresh.identifier = item.identifier;
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
        log_refused_write("write", "invalid_request_id");
        return IpcResponse::Error {
            message: format!("request_id must be 1 to {MAX_KEY_BYTES} printable bytes"),
        };
    }
    let validate_db = db.clone();
    let through_alias = fingerprint.starts_with("modify|");
    dedup
        .run(
            &key,
            &fingerprint,
            std::time::Instant::now,
            move |cached: IpcResponse| refresh_cached_write(&validate_db, cached, through_alias),
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

/// The `GetFileStatus` reply: a live row as before; otherwise a provisional id whose create
/// landed is answered with the server file's item under the provisional id (spec §7.2), so
/// the system's item keeps its identity until it applies the swap; otherwise "not found".
pub(crate) fn file_status_response(db: &crate::state_db::StateDb, file_id: &str) -> IpcResponse {
    if let Ok(Some(entry)) = db.get_file(file_id) {
        return IpcResponse::FileStatus(file_entry_payload_for_db(db, &entry, FP_ROOT_APPLE));
    }
    if let Ok(Some(server_id)) = db.resolve_alias(file_id)
        && let Ok(Some(entry)) = db.get_file(&server_id)
    {
        crate::engine_bridge::log_alias_resolved("item", file_id, &server_id);
        let mut item = file_entry_payload_for_db(db, &entry, FP_ROOT_APPLE);
        item.identifier = file_id.to_string();
        return IpcResponse::FileStatus(item);
    }
    IpcResponse::Error {
        message: "not found".into(),
    }
}

/// The reply to a Finder write. A queued write answers with its item, read back from the
/// row, presented under `present_as` when the write came under a provisional id (spec §7.2).
pub(crate) fn write_outcome_response(
    op: &'static str,
    db: &crate::state_db::StateDb,
    result: anyhow::Result<crate::engine_bridge::FpWrite>,
) -> IpcResponse {
    match result.map(|write| (write.outcome, write.present_as)) {
        Ok((crate::engine_bridge::FinderWriteOutcome::Ignored { message }, _)) => IpcResponse::WriteQueued {
            item: None,
            ignored: true,
            message,
        },
        Ok((
            crate::engine_bridge::FinderWriteOutcome::Queued {
                file_id,
                ignored,
                message,
                ..
            },
            present_as,
        )) => {
            let item = file_id
                .as_deref()
                .and_then(|id| db.get_file(id).ok().flatten())
                .map(|entry| {
                    let mut item = file_entry_payload_for_db(db, &entry, FP_ROOT_APPLE);
                    if let Some(identifier) = present_as {
                        item.identifier = identifier;
                    }
                    item
                });
            // §7.3: a create or modify reply always carries an item unless the write was ignored:
            // for a modify, the system reads a nil item as "delete the item on disk"
            // (REPL.h:629-634). When the row cannot be read back, the write is still queued and
            // uploads with its bytes kept. The reply is an `Error`, which the extension reports as
            // `cannotSynchronize`: the system presents an error and does not retry the item until
            // it changes on disk (NSFileProviderReplicatedExtension.h on modifyItem;
            // NSFileProviderError.h on `CannotSynchronize`; the extension never signals it
            // resolved). A transient error would be worse: the system's retry would run the write
            // again (an `Error` is not remembered by the dedup) and queue it a second time. A
            // delete's reply item is never read, so a delete is not refused.
            #[cfg(target_os = "macos")]
            if item.is_none() && matches!(op, "create" | "modify") {
                log_refused_write(op, "row_unreadable");
                return IpcResponse::Error {
                    message: "Beebeeb saved this change and will upload it, but could not describe the file to \
                              Finder. Finder shows an error on it until the file changes again."
                        .into(),
                };
            }
            IpcResponse::WriteQueued { item, ignored, message }
        }
        // Spec §8.4: the staging folder cannot take the copy now. Nothing was staged or
        // queued; the extension reports a transient error, so the system retries the write.
        // The text is the typed error's fixed reason, never a path.
        Err(e) if e.is::<crate::engine_bridge::FinderStagingUnavailable>() => {
            log_refused_write(op, write_refusal_category(&e));
            IpcResponse::WriteRetryLater { message: e.to_string() }
        }
        // Spec §8.7 S6: an engine-start sweep removed the staged copy before the accept
        // committed. Nothing was queued; the system hands the write over again.
        Err(e) if e.is::<crate::engine_bridge::StagedCopyGone>() => {
            log_refused_write(op, write_refusal_category(&e));
            IpcResponse::WriteRetryLater { message: e.to_string() }
        }
        // Spec 2026-10-10 §15 question 3: a sign-out closed accepts after its engine stop. Nothing was queued; the
        // extension reports a transient error, so the system keeps the change and hands it over again.
        Err(e) if e.is::<crate::engine_bridge::AcceptsClosed>() => {
            log_refused_write(op, write_refusal_category(&e));
            IpcResponse::WriteRetryLater { message: e.to_string() }
        }
        // The disk or the state database is full. A create or modify that failed queued nothing
        // (the accept's transaction is its last step that can fail), so the system retries it, as
        // for an unavailable staging folder, and it is accepted once there is room. An `Error`
        // would reach macOS as `cannotSynchronize`, which it does not retry until the item
        // changes again. A fixed text, never the error's.
        Err(e)
            if matches!(op, "create" | "modify")
                && matches!(
                    crate::engine_bridge::error_category(e.as_ref()),
                    "disk_full" | "database_full"
                ) =>
        {
            log_refused_write(op, write_refusal_category(&e));
            IpcResponse::WriteRetryLater {
                message: "the disk or Beebeeb's local database is full".into(),
            }
        }
        Err(e) => {
            // The extension turns this reply into a definitive Finder error,
            // so it must leave a trace in the daemon log (it used to leave
            // none). The category only: the error text can name the file.
            match e.downcast_ref::<crate::engine_bridge::UnknownItem>() {
                Some(unknown) => log_refused_unknown_item(op, &unknown.requested_id),
                None => log_refused_write(op, write_refusal_category(&e)),
            }
            IpcResponse::Error { message: e.to_string() }
        }
    }
}

/// The change-feed item assembly, extracted from the `ListChanges` arm so
/// tests can drive it without a live socket. `Deleted` rows are gone; the
/// replica only needs the identifier. For every other kind the FULL item
/// payload rides along so `didUpdateItems` needs no second round-trip. A row
/// that vanished since the change was recorded yields no item (the Swift
/// enumerator skips it; its later `deleted` row reports the exit).
pub(crate) fn file_provider_change_payloads(
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
                    .filter(|entry| {
                        // Task 1701 + Codex P1 (PR #99 review): shared content
                        // is webapp-only by ruling
                        // (.claude/tasks/decisions/finder-sidebar-single-root.md).
                        // A live `SharedWithMe` row has a nil contract parent,
                        // so its ride-along payload would fall back to
                        // FP_ROOT_APPLE; the Swift materialized filter treats a
                        // root parent as always materialized (and fails open
                        // while the set is unknown), which would surface shared
                        // files directly beneath "Beebeeb". Report the change
                        // row with NO item — the Swift enumerator skips
                        // payload-less rows, identical to a row that vanished.
                        db.get_file_contract_state(&entry.file_id)
                            .ok()
                            .flatten()
                            .map(|contract| contract.namespace != crate::state_db::Namespace::SharedWithMe)
                            .unwrap_or(true)
                    })
                    .map(|entry| file_entry_payload_for_db(db, &entry, FP_ROOT_APPLE)),
            },
            file_id: change.file_id,
            kind: change.kind.as_str().to_string(),
            old_parent_id: change.old_parent_id,
            new_parent_id: change.new_parent_id,
        })
        .collect()
}

pub(crate) fn list_file_provider_items(
    db: &crate::state_db::StateDb,
    container_id: &str,
) -> Vec<FileProviderItemPayload> {
    // Task 1698: the SYSTEM trash container lists the trash view — the
    // Trashing rows that are TOP of trash (their parent row is not itself
    // Trashing; the children of a trashed folder enumerate INSIDE that
    // folder, mirroring the server trash's shape).
    if is_trash_container(container_id) {
        return db
            .list_files()
            .unwrap_or_default()
            .into_iter()
            .filter(|entry| entry.status == crate::state_db::FileStatus::Trashing && !parent_row_is_trashing(db, entry))
            .map(|entry| file_entry_payload_for_db(db, &entry, FP_TRASH_APPLE))
            .collect();
    }
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
        //
        // Task 1698: Trashing rows are excluded — a trashed item left "Beebeeb"
        // (the system reparented it into the trash container); surfacing it at
        // the root would duplicate it in Finder.
        //
        // Spec 2026-10-09 §7.2 (final review): the root lists the rows with no
        // parent, as a folder lists the rows whose parent it is, and as every
        // reply names the parent. A path with no folder in it decided this
        // before, so a Finder create or save that wrote its name alone as the
        // path was listed at the top level whatever its parent. A row with no
        // name (it did not decrypt) is still left out.
        return db
            .list_files()
            .unwrap_or_default()
            .into_iter()
            .filter(|entry| entry.status != crate::state_db::FileStatus::Trashing)
            .filter(|entry| {
                entry.parent_id.is_none()
                    && !entry.path.trim_matches('/').is_empty()
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
    //
    // Task 1698: Trashing children are listed only when the CONTAINER itself
    // is trashed (they are the trash view's folder contents); a live folder
    // stops listing children the moment they move to the trash container.
    let container_is_trashed = db
        .get_file(container_id)
        .ok()
        .flatten()
        .map(|entry| entry.status == crate::state_db::FileStatus::Trashing)
        .unwrap_or(false);
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
        .filter(move |entry| container_is_trashed || entry.status != crate::state_db::FileStatus::Trashing)
        .map(|entry| file_entry_payload_for_db(db, &entry, container_id))
        .collect()
}

/// Task 1698: is `entry`'s PARENT row itself in `Trashing`? A child inside a
/// trashed folder stays parented to the folder (trash-view content); a row
/// whose parent row is NOT trashed is TOP of trash — presented directly under
/// the trash container.
fn parent_row_is_trashing(db: &crate::state_db::StateDb, entry: &crate::state_db::FileEntry) -> bool {
    entry
        .parent_id
        .as_deref()
        .and_then(|parent_id| db.get_file(parent_id).ok().flatten())
        .map(|parent| parent.status == crate::state_db::FileStatus::Trashing)
        .unwrap_or(false)
}

fn is_trash_container(container_id: &str) -> bool {
    // FP_TRASH_APPLE or its short form; one definition, which the write accept reads too.
    crate::state_db::is_file_provider_trash(container_id)
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
fn with_folder_add_subitems(kind: &str, status: &crate::state_db::FileStatus, capabilities: u32) -> u32 {
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

#[cfg(test)]
thread_local! {
    static BUILDER_SEAM: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
}

/// T64/T65: fires once, on this thread, right after the builder's one read.
#[cfg(test)]
pub(crate) fn arm_builder_seam(hook: impl FnOnce() + 'static) {
    BUILDER_SEAM.with(|seam| *seam.borrow_mut() = Some(Box::new(hook)));
}

/// A no-op outside tests.
fn builder_seam() {
    #[cfg(test)]
    {
        let hook = BUILDER_SEAM.with(|seam| seam.borrow_mut().take());
        if let Some(hook) = hook {
            hook();
        }
    }
}

/// How often one item's failed presentation read is logged. The change feed, every enumeration
/// and every item lookup build the same item again, so a row whose read keeps failing would
/// otherwise add a line each time (spec §11: at most one line per write or per attempt).
const PRESENTATION_WARN_INTERVAL: std::time::Duration = std::time::Duration::from_secs(10 * 60);

/// At most this many items are remembered; past it, the ones whose interval is over go first,
/// then all of them (a line too many, never unbounded memory).
const PRESENTATION_WARN_MAX_ITEMS: usize = 1024;

/// Per item, per kind of failure (a fixed category): when its last line was written, and how
/// many failed reads were not logged since. An item is forgotten when it reads cleanly again.
type PresentationWarned =
    std::collections::HashMap<String, std::collections::HashMap<&'static str, (std::time::Instant, u64)>>;

static PRESENTATION_WARNED: std::sync::LazyLock<std::sync::Mutex<PresentationWarned>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(PresentationWarned::new()));

/// Whether this failed presentation read of `file_id`, of `kind`, is logged now: `Some(n)`,
/// with the `n` earlier ones of that kind that were not, or `None` while its interval runs. A
/// different kind of failure on the same item has its own interval, so it is logged at once.
fn presentation_warn_due(
    seen: &mut PresentationWarned,
    file_id: &str,
    kind: &'static str,
    now: std::time::Instant,
) -> Option<u64> {
    if let Some((last, held_back)) = seen.get_mut(file_id).and_then(|kinds| kinds.get_mut(kind)) {
        if now.saturating_duration_since(*last) < PRESENTATION_WARN_INTERVAL {
            *held_back += 1;
            return None;
        }
        let logged = Some(*held_back);
        *last = now;
        *held_back = 0;
        return logged;
    }
    if !seen.contains_key(file_id) && seen.len() >= PRESENTATION_WARN_MAX_ITEMS {
        seen.retain(|_, kinds| {
            kinds.retain(|_, (last, _)| now.saturating_duration_since(*last) < PRESENTATION_WARN_INTERVAL);
            !kinds.is_empty()
        });
        if seen.len() >= PRESENTATION_WARN_MAX_ITEMS {
            seen.clear();
        }
    }
    seen.entry(file_id.to_string()).or_default().insert(kind, (now, 0));
    Some(0)
}

/// The item's presentation read succeeded: a later failure is a new episode, logged at once.
fn presentation_read_ok(seen: &mut PresentationWarned, file_id: &str) {
    seen.remove(file_id);
}

/// [`PRESENTATION_WARNED`], locked; a poisoned lock is taken as it is (it only holds counts).
fn presentation_warned() -> std::sync::MutexGuard<'static, PresentationWarned> {
    PRESENTATION_WARNED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub(crate) fn file_entry_payload_for_db(
    db: &crate::state_db::StateDb,
    entry: &crate::state_db::FileEntry,
    parent_identifier: &str,
) -> FileProviderItemPayload {
    // Spec §5.3: the row and the queue come from ONE read, so the predicate never
    // sees a row from before a landing beside a queue from after it.
    let read = db.item_presentation(&entry.file_id);
    builder_seam();
    // §9.1: a Finder upload that has not parked keeps the item writable and not evictable
    // (ITEM.h:531-533). A Trashing row keeps its trash presentation (m-8).
    let shown;
    let entry = match &read {
        Ok(Some(p)) if p.unparked_finder_upload && entry.status != crate::state_db::FileStatus::Trashing => {
            shown = crate::state_db::FileEntry {
                status: crate::state_db::FileStatus::Uploading,
                ..entry.clone()
            };
            &shown
        }
        _ => entry,
    };
    if read.is_ok() {
        presentation_read_ok(&mut presentation_warned(), &entry.file_id);
    }
    let mut payload = match &read {
        Ok(Some(p)) => file_entry_payload(entry, &p.contract, parent_identifier),
        Ok(None) => file_entry_payload_without_contract(entry, parent_identifier),
        Err(error) => {
            // A failed read (a SQLite error, or a held write without its base) is
            // presented as round 3 did, from the contract alone, with the capabilities
            // kept. When the row holds a write token, the numeric content version this
            // gives is not the one the system holds, so the system reads it as a content
            // change and fetches the item again (ITEM.h:93-95). That fetch is served from
            // the queued write's staged copy, or, once the write landed, from the server's
            // same bytes (spec §7.4): an extra fetch, never other bytes. The held columns
            // alone are not a fallback: without the queue they cannot tell a write that is
            // queued or landed here from one a remote change has replaced, and naming the
            // token for that one would hide the remote change. A fixed category only: the
            // error can carry a path (spec §11). Once per item and kind of failure per
            // interval, with how many were held back.
            let kind = crate::engine_bridge::error_category(error);
            let due = presentation_warn_due(
                &mut presentation_warned(),
                &entry.file_id,
                kind,
                std::time::Instant::now(),
            );
            if let Some(held_back) = due {
                tracing::warn!(
                    file_id = %entry.file_id,
                    category = "presentation_read_failed",
                    kind,
                    held_back,
                    "item presented from its contract alone"
                );
            }
            match db.get_file_contract_state(&entry.file_id).ok().flatten() {
                Some(contract) => file_entry_payload(entry, &contract, parent_identifier),
                None => file_entry_payload_without_contract(entry, parent_identifier),
            }
        }
    };
    if let Ok(Some(p)) = &read {
        // Rule 1: the bytes the system holds keep the name their write was given,
        // while that write is queued or is what the server holds now.
        match crate::write_token::held_token(
            p.held.as_ref(),
            p.held_write_queued,
            p.contract.current_version,
            p.contract.current_object_version_id.as_deref(),
        ) {
            Some(token) => payload.content_version = Some(token),
            None => {
                // Housekeeping only; correctness comes from the predicate. Guarded by
                // the write this read saw, so a token set since is never cleared.
                if let Some(held) = &p.held {
                    let _ = db.clear_held_if(&entry.file_id, &held.write_id);
                }
            }
        }
    }
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
    // Task 1698 (trash ruling): a Trashing row that is TOP of trash (its
    // parent row is not itself Trashing) is presented under the SYSTEM trash
    // container, wherever it used to live. Children of a trashed folder keep
    // their real parent so the trash view mirrors the server trash's shape.
    if entry.status == crate::state_db::FileStatus::Trashing && !parent_row_is_trashing(db, entry) {
        payload.parent_identifier = FP_TRASH_APPLE.to_string();
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
    // (drag-move / drag-to-Trash; trash semantics: task 1698 ruling).
    let capabilities = with_folder_add_subitems(kind, &entry.status, capabilities);

    // Task 1697: version split. contentVersion changes only when the content
    // identity changes (server version + content hash when present);
    // metadataVersion tracks mtime/size/parent/name so a rename no longer
    // forces a content re-download. Both identifiers lead with the server
    // version and never with `remote_updated_at`: the extension sends the
    // content version back as the base of its next write.
    let content_version =
        crate::engine_bridge::item_content_version(contract.current_version, entry.content_hash.as_deref());
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
        version_identifier: Some(crate::engine_bridge::item_version_identifier(
            contract.current_version,
            entry.modified_at,
            entry.size_bytes,
        )),
        // Dates/child count are stamped by `file_entry_payload_for_db`, where
        // the DB handle is available.
        created_at: None,
        modified_at: None,
        child_item_count: None,
        content_version: Some(content_version),
        metadata_version: Some(metadata_version),
        pinned: matches!(contract.effective_pin_state(), crate::state_db::PinState::Pinned),
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
    let capabilities = with_folder_add_subitems(kind, &entry.status, capabilities_for_status(&entry.status));

    // Task 1697: version split without a contract. With no contract there is
    // no known server version, so both identifiers lead with 0, which the
    // write path reads as "no base" (`parse_base_version_number` ignores 0).
    // The row's wall-clock stamp is not a version and is never used as one.
    let content_version = crate::engine_bridge::item_content_version(0, None);
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
        version_identifier: Some(crate::engine_bridge::item_version_identifier(
            0,
            entry.modified_at,
            entry.size_bytes,
        )),
        created_at: None,
        modified_at: None,
        child_item_count: None,
        content_version: Some(content_version),
        metadata_version: Some(metadata_version),
        // No contract = no pin state to resolve (fail-safe: unpinned).
        pinned: false,
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
        // Trash semantics: task 1698 ruling (the trash-container reparent maps
        // to the server trash op); the capability only stops Finder from
        // blocking the gesture outright.
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

/// Task 1699: mirror of the Windows Cloud Files thumbnail variant picker
/// (`windows_cf/thumbnail_provider.rs`): the requested maximum pixel
/// dimension maps to the smallest server variant that satisfies it. The
/// server serves three encrypted variants (small/medium/large) per file.
pub(crate) fn thumbnail_variant(max_dimension: u32) -> &'static str {
    if max_dimension <= 96 {
        "small"
    } else if max_dimension <= 256 {
        "medium"
    } else {
        "large"
    }
}

/// The destinations a `FetchThumbnail` staging write may land in — the same
/// allowed roots as `HydrateFile` (task 1247): the configured sync root, this
/// process's temp dir, and on macOS the shared App Group hydrate-cache
/// directory (ensured to exist, owner-only and backup-excluded, and swept for
/// crash-orphaned staging files first — the extension deletes its staging
/// file right after reading it, so the sweep stays a crash backstop).
fn thumbnail_allowed_roots() -> Vec<std::path::PathBuf> {
    let mut roots = Vec::new();
    if let Some(root) = crate::config::DesktopConfig::load().ok().and_then(|cfg| cfg.sync_root) {
        roots.push(root);
    }
    roots.push(std::env::temp_dir());
    #[cfg(target_os = "macos")]
    {
        let dir = macos_hydrate_cache_dir();
        if let Err(e) = macos_ensure_hydrate_cache_dir(&dir) {
            tracing::warn!(error = %e, dir = %dir.display(), "could not create macOS hydrate-cache dir");
        }
        if let Err(e) =
            macos_sweep_stale_hydrate_cache_entries(&dir, MACOS_HYDRATE_CACHE_TTL, std::time::SystemTime::now())
        {
            tracing::warn!(error = %e, dir = %dir.display(), "hydrate-cache TTL sweep failed (best-effort)");
        }
        roots.push(dir);
    }
    roots
}

/// `Some(error reply message)` when `dest` is not safely stageable: its
/// parent must exist (nothing is created outside an allowed root) and
/// canonicalize to a path under one of the allowed roots. Both sides are
/// canonicalized so macOS `/var` ↔ `/private/var` symlinks cannot split the
/// check (mirrors the hydrate containment semantics).
fn thumbnail_destination_error(dest: &std::path::Path, allowed_roots: &[std::path::PathBuf]) -> Option<String> {
    let parent = dest.parent()?;
    let canonical_parent = match parent.canonicalize() {
        Ok(p) => p,
        Err(_) => {
            return Some(format!(
                "thumbnail destination {} is not under an allowed root (parent does not exist)",
                dest.display()
            ));
        }
    };
    let contained = allowed_roots.iter().any(|root| match root.canonicalize() {
        Ok(canonical_root) => canonical_parent == canonical_root || canonical_parent.starts_with(&canonical_root),
        Err(_) => false,
    });
    if contained {
        None
    } else {
        Some(format!(
            "thumbnail destination {} is not under an allowed root",
            dest.display()
        ))
    }
}

/// Stage decrypted thumbnail plaintext at `dest` atomically: write a
/// fresh temp leaf, force owner-only permissions, then rename onto `dest` —
/// the extension can never observe a partial thumbnail, and a crash leaves
/// only the temp file for the TTL sweep to collect. Returns the plaintext byte
/// count. Caller has already validated containment
/// ([`thumbnail_destination_error`]); this is defense in depth for the same
/// rule and refuses to write outside the allowed roots regardless.
///
/// Task 1699 review (PR #103 thread PRRT_kwDOSLX6Xs6oeNfW, P1): this used to
/// be a plain `File::create` on `<dest>.part` after the parent-only
/// containment check — but `File::create` FOLLOWS a pre-created symlink, so a
/// socket client could plant `<dest>.part` → arbitrary file and have the
/// daemon truncate it outside every allowed root. Delegates to the
/// fd-anchored staging writer (`write_hydrated_plaintext`, tasks 1247/1670),
/// which descends O_NOFOLLOW from a trusted root fd, stages under a fresh
/// random temp leaf, and publishes with an anchored `renameat` — it can
/// REPLACE a planted path but can never WRITE THROUGH one. The temp shape
/// (`.{leaf}.{uuid}.part`) is the same one the macOS TTL sweep is proven to
/// leave alone while in flight.
pub(crate) fn persist_thumbnail_plaintext(
    dest: &std::path::Path,
    allowed_roots: &[std::path::PathBuf],
    plaintext: &[u8],
) -> anyhow::Result<u64> {
    if thumbnail_destination_error(dest, allowed_roots).is_some() {
        anyhow::bail!("thumbnail destination {} is not under an allowed root", dest.display());
    }
    let root_paths: Vec<&std::path::Path> = allowed_roots.iter().map(|p| p.as_path()).collect();
    crate::engine_bridge::write_hydrated_plaintext(dest, &root_paths, plaintext)
        .map(|_| plaintext.len() as u64)
        .map_err(|e| anyhow::anyhow!("could not stage thumbnail at {}: {e}", dest.display()))
}

#[cfg(test)]
#[path = "ipc_socket_framing_tests.rs"]
mod framing_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state_db::{FileEntry, FileStatus, ItemKind, Namespace, PERMISSION_READ, PERMISSION_WRITE, StateDb};
    use tempfile::tempdir;

    fn seed_1694_item(db: &StateDb, file_id: &str, status: FileStatus, permissions: i64, item_kind: ItemKind) {
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

    fn upload_staging_fixture() -> (tempfile::TempDir, std::path::PathBuf) {
        let root = tempdir().unwrap();
        let staging = root.path().join("upload-staging");
        std::fs::create_dir(&staging).unwrap();
        (root, staging)
    }

    fn write_file(path: &std::path::Path, bytes: &[u8]) -> String {
        std::fs::write(path, bytes).unwrap();
        path.to_string_lossy().into_owned()
    }

    fn capture_logs(body: impl FnOnce()) -> String {
        #[derive(Clone)]
        struct Capture(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
        impl std::io::Write for Capture {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let buffer = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let writer = Capture(buffer.clone());
        // tracing caches each callsite's interest for the whole process. While
        // only ONE dispatcher is alive, tracing-core takes that interest from
        // the registering thread's own default (`has_just_one`), so a
        // callsite first hit by another test thread, which has no subscriber,
        // is cached as "never" and this capture misses its events. A second
        // live dispatcher keeps registration on the all-dispatchers path.
        let _second_dispatcher = tracing::Dispatch::new(tracing_subscriber::registry());
        let subscriber = tracing_subscriber::fmt()
            .with_writer(move || writer.clone())
            .with_ansi(false)
            .with_max_level(tracing::Level::TRACE)
            .finish();
        tracing::subscriber::with_default(subscriber, body);
        let bytes = buffer.lock().unwrap().clone();
        String::from_utf8(bytes).unwrap()
    }

    mod root_payloads;
    mod macos_socket_cache;
    mod payload_contracts;
    mod upload_staging;
    mod presentation_write;
}
