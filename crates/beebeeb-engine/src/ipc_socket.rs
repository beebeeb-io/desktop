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

// The macOS App Group shared between the containing app (`src-tauri/entitlements.plist`) and the File
// Provider extension (`BeebeebFileProvider/BeebeebFileProvider.entitlements`), both of which declare
// `com.apple.security.application-groups: [<the id>]`, is the app identity's:
// `EngineIdentity::macos_app_group` (pinned in `identity.rs`). `BeebeebFileProvider/XPCBridge.swift`
// mirrors the real app's literal (Swift has no practical way to `include!` a Rust const, and there is no
// shared-codegen step in this repo: see `docs/MACOS_BRINGUP_BRIEF.md`). Keep all three in sync if it ever
// changes.

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
pub fn ipc_socket_path(identity: &crate::identity::EngineIdentity) -> std::path::PathBuf {
    macos_ipc_socket_path_in(identity, &macos_real_home_dir())
}

/// Task 1670: subdirectory (of the same shared App Group container as
/// `ipc.sock` above) that stages a freshly-hydrated file's decrypted plaintext
/// before the File Provider extension hands its URL to Finder.
///
/// Deliberately a DIFFERENT name than the socket file, in the SAME directory —
/// nothing about that collides with `MACOS_IPC_SOCKET_FILENAME`.
#[cfg(target_os = "macos")]
const MACOS_HYDRATE_CACHE_DIRNAME: &str = "hydrate-cache";

#[cfg(all(target_os = "macos", not(any(test, feature = "test-support"))))]
pub fn macos_hydrate_cache_dir(identity: &crate::identity::EngineIdentity) -> std::path::PathBuf {
    macos_hydrate_cache_dir_in(identity, &macos_real_home_dir())
}

/// A test build resolves a per-process sandbox with the same
/// `Library/Group Containers/<group>/hydrate-cache` shape, never the real App Group directory
/// the installed app shares (`getpwuid`-resolved, so `$HOME` cannot redirect it). Without this,
/// 9 tests ran the production code that creates the real dir and TTL-sweeps it (the hydrate and
/// thumbnail handlers) or empties it (`purge_macos_hydrate_cache`: sign-out and lock). Pinned
/// by `unit_tests_resolve_the_hydrate_dir_in_a_sandbox_never_the_real_group_container`.
#[cfg(all(target_os = "macos", any(test, feature = "test-support")))]
pub fn macos_hydrate_cache_dir(identity: &crate::identity::EngineIdentity) -> std::path::PathBuf {
    let home = crate::test_sandbox::dir("real-home").expect("create the unit-test sandbox");
    macos_hydrate_cache_dir_in(identity, &home)
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
pub const MACOS_HYDRATE_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(10 * 60);

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
pub fn macos_validate_hydrate_item_identifier(identifier: &str) -> Result<(), &'static str> {
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
pub fn macos_ensure_hydrate_cache_dir(dir: &std::path::Path) -> std::io::Result<()> {
    macos_ensure_private_staging_dir(dir)
}

/// Create (if needed) and harden a staging directory: owner-only `0o700` and
/// excluded from backups. Shared by the App Group hydrate-cache and
/// upload-staging directory ([`macos_upload_staging_dir`]), which only ever
/// hold short-lived plaintext, and the engine's own Finder-write staging root
/// (`engine_bridge::durable_finder_staging_root`), which holds a save's copy
/// until it uploads.
#[cfg(target_os = "macos")]
pub fn macos_ensure_private_staging_dir(dir: &std::path::Path) -> std::io::Result<()> {
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
pub fn macos_sweep_stale_hydrate_cache_entries(
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
pub fn macos_upload_staging_dir(identity: &crate::identity::EngineIdentity) -> std::path::PathBuf {
    macos_upload_staging_dir_in(identity, &macos_real_home_dir())
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
#[cfg(any(target_os = "macos", any(test, feature = "test-support")))]
pub const UPLOAD_STAGING_MAX_AGE: std::time::Duration = std::time::Duration::from_secs(60 * 60);

/// Remove upload-staging entries that have existed for at least `max_age`
/// before `now`, by their ctime (see the loop). A missing directory is not an error. `now` / `max_age` are
/// parameters so this is testable without a clock.
///
/// The directory is opened once ([`StagingDir::open`]) and refused unless it
/// is a real directory owned by this user: through a symlink, this purge
/// would delete another folder's files. Every entry is inspected and removed
/// through that descriptor.
#[cfg(any(target_os = "macos", any(test, feature = "test-support")))]
pub(crate) fn purge_stale_upload_staging(
    dir: &std::path::Path,
    max_age: std::time::Duration,
    now: std::time::SystemTime,
) -> Result<usize, StagingDirRefusal> {
    purge_upload_staging(dir, UploadStagingPurge::OlderThan { max_age, now })
}

/// Which upload-staging entries a purge removes.
#[derive(Debug, Clone, Copy)]
#[cfg(any(target_os = "macos", any(test, feature = "test-support")))]
pub enum UploadStagingPurge {
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
#[cfg(any(target_os = "macos", any(test, feature = "test-support")))]
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
pub fn macos_prepare_upload_staging_dir(
    dir: &std::path::Path,
    max_age: std::time::Duration,
    now: std::time::SystemTime,
) -> Result<usize, StagingDirRefusal> {
    let staging = macos_open_private_staging_dir(dir)?;
    purge_upload_staging_in(&staging, UploadStagingPurge::OlderThan { max_age, now })
}

/// Runner startup: prepare the real directory and purge it. Best-effort.
#[cfg(target_os = "macos")]
pub fn macos_prepare_upload_staging(identity: &crate::identity::EngineIdentity) {
    let dir = macos_upload_staging_dir(identity);
    let result = macos_prepare_upload_staging_dir(&dir, UPLOAD_STAGING_MAX_AGE, std::time::SystemTime::now());
    log_upload_staging_purge("daemon-startup", result);
}

/// Runner tick: purge the real directory. Best-effort.
#[cfg(target_os = "macos")]
pub fn macos_sweep_upload_staging(identity: &crate::identity::EngineIdentity) {
    sweep_upload_staging_at(
        &macos_upload_staging_dir(identity),
        "periodic",
        std::time::SystemTime::now(),
    );
}

/// Sign-out and Lock: remove every entry of `dir`, logged. The caller
/// guarantees that no IPC listener can still be serving (see
/// [`UploadStagingPurge::Everything`]).
#[cfg(any(target_os = "macos", any(test, feature = "test-support")))]
pub fn purge_all_upload_staging_at(dir: &std::path::Path, context: &'static str) {
    log_upload_staging_purge(context, purge_upload_staging(dir, UploadStagingPurge::Everything));
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
pub fn macos_real_home_dir() -> std::path::PathBuf {
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
pub fn ipc_socket_path(identity: &crate::identity::EngineIdentity) -> std::path::PathBuf {
    let runtime_dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into());
    std::path::PathBuf::from(runtime_dir).join(identity.daemon_socket_name())
}

/// The reply to a Finder write, for the engine's own `anyhow` result: the
/// boundary the engine's tests call. The error is classified once into
/// [`IpcError`], and the typed core answers.
pub fn write_outcome_response(
    op: &'static str,
    db: &crate::state_db::StateDb,
    result: anyhow::Result<crate::engine_bridge::FpWrite>,
) -> IpcResponse {
    write_outcome_response_classified(op, db, result.map_err(|error| IpcError::from_engine(op, error)))
}

/// The reply to a Finder write whose error is already classified. A queued
/// write answers with its item, read back from the row, presented under
/// `present_as` when the write came under a provisional id (spec §7.2).
fn write_outcome_response_classified(
    op: &'static str,
    db: &crate::state_db::StateDb,
    result: Result<crate::engine_bridge::FpWrite, IpcError>,
) -> IpcResponse {
    let (outcome, present_as) = match result {
        Ok(write) => (write.outcome, write.present_as),
        Err(error) => return refused_write_reply(op, error),
    };
    match outcome {
        crate::engine_bridge::FinderWriteOutcome::Ignored { message } => IpcResponse::WriteQueued {
            item: None,
            ignored: true,
            message,
        },
        crate::engine_bridge::FinderWriteOutcome::Queued {
            file_id,
            ignored,
            message,
            ..
        } => {
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
    }
}

/// The reply to a refused write. The category (or the unknown id) goes to the
/// log first; then a transient error when the system must hand the write over
/// again (spec §8.4, §8.7 S6, 2026-10-10 §15 question 3, a full disk), and a
/// definitive Finder error otherwise — the extension turns that into
/// `cannotSynchronize`, so it must leave a trace.
fn refused_write_reply(op: &'static str, error: IpcError) -> IpcResponse {
    match error.unknown_item_id() {
        Some(requested_id) => log_refused_unknown_item(op, requested_id),
        None => log_refused_write(op, error.category()),
    }
    let message = error.message();
    if error.is_retry_later() {
        IpcResponse::WriteRetryLater { message }
    } else {
        IpcResponse::Error { message }
    }
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static BUILDER_SEAM: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
}

/// T64/T65: fires once, on this thread, right after the builder's one read.
#[cfg(any(test, feature = "test-support"))]
pub fn arm_builder_seam(hook: impl FnOnce() + 'static) {
    BUILDER_SEAM.with(|seam| *seam.borrow_mut() = Some(Box::new(hook)));
}

/// A no-op outside tests.
fn builder_seam() {
    #[cfg(any(test, feature = "test-support"))]
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

    mod macos_socket_cache;
    mod payload_contracts;
    mod root_payloads;
    // LD-0113 D2: the payload of a protected item.
    mod ld0113_protect;
    mod presentation_write;
    mod upload_staging;
}

mod error;
use error::*;

mod ids;
use ids::*;

mod requests;
use requests::*;

mod protocol;
use protocol::*;
pub use protocol::{CAP_WRITE, FP_TRASH_APPLE};
pub use protocol::{FileProviderChangePayload, FileProviderItemPayload, IpcRequest, IpcResponse};

mod payloads;
pub use payloads::file_entry_payload_for_db;
#[cfg(any(test, feature = "test-support"))]
use payloads::*;

mod thumbnails;
use thumbnails::*;
pub use thumbnails::{persist_thumbnail_plaintext, thumbnail_variant};

mod macos_paths;
// Only the macOS paths use these.
#[cfg(target_os = "macos")]
use macos_paths::*;

mod macos_staging;
use macos_staging::*;
pub use macos_staging::{StagedContents, StagingDir, StagingDirRefusal, WriteContentsPolicy};
// The staging admission is exercised by the upload-staging tests.
#[cfg(any(test, feature = "test-support"))]
pub use macos_staging::{ContentsRefusal, open_staged_contents};

mod server;
pub use server::serve_ipc;
#[cfg(any(test, feature = "test-support"))]
pub use server::serve_ipc_at;
#[cfg(any(test, feature = "test-support"))]
pub use server::serve_ipc_at_with_ready;
#[cfg(any(test, feature = "test-support"))]
use server::*;

mod hydrate;
#[cfg(any(test, feature = "test-support"))]
pub use hydrate::hydrate_failure_reply;
use hydrate::*;

mod write_path;
pub use write_path::file_status_response;
use write_path::*;

mod listing;
use listing::*;
pub use listing::{file_provider_change_payloads, list_file_provider_items};
