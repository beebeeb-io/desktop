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


/// Sign-out and Lock: remove every entry of `dir`, logged. The caller
/// guarantees that no IPC listener can still be serving (see
/// [`UploadStagingPurge::Everything`]).
#[cfg(any(target_os = "macos", test))]
pub(crate) fn purge_all_upload_staging_at(dir: &std::path::Path, context: &'static str) {
    log_upload_staging_purge(context, purge_upload_staging(dir, UploadStagingPurge::Everything));
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

mod protocol;
use protocol::*;
pub use protocol::{IpcRequest, IpcResponse, FileProviderItemPayload, FileProviderChangePayload};
pub(crate) use protocol::{FP_TRASH_APPLE, CAP_WRITE};

mod payloads;
use payloads::*;
pub(crate) use payloads::{file_entry_payload_for_db};

mod thumbnails;
use thumbnails::*;
pub(crate) use thumbnails::{thumbnail_variant, persist_thumbnail_plaintext};

mod macos_paths;
use macos_paths::*;

mod macos_staging;
use macos_staging::*;
pub(crate) use macos_staging::{StagingDirRefusal, StagingDir, WriteContentsPolicy, ContentsRefusal, open_staged_contents, OpenedContents, StagedContents};
