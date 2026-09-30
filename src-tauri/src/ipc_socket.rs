//! Unix-domain-socket IPC between the desktop daemon and the macOS File
//! Provider extension (and the Linux FUSE mount). Unix sockets do not
//! exist on Windows, where the Cloud Files callback runs in-process
//! (see `crate::windows_cf`), so the whole module is gated to `unix`.

#![cfg(unix)]

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
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
    HydrateFile {
        file_id: String,
        dest_path: String,
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
    },
    QueueFinderModify {
        file_id: String,
        parent_id: Option<String>,
        filename: String,
        kind: String,
        contents_path: Option<String>,
        content_type: Option<String>,
        base_version_identifier: Option<String>,
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

#[derive(Debug, Serialize, Deserialize)]
pub enum IpcResponse {
    FileStatus(FileProviderItemPayload),
    FileProviderItems {
        items: Vec<FileProviderItemPayload>,
    },
    Ok,
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
    PinUpdated {
        changed_item_ids: Vec<String>,
        hydrate_operations: usize,
    },
    CacheCleanup {
        evicted_file_ids: Vec<String>,
    },
}

#[derive(Debug, Serialize, Deserialize)]
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
}

const FP_ROOT: &str = "__fp_root__";
const FP_ROOT_APPLE: &str = "NSFileProviderRootContainerItemIdentifier";
const NAMESPACE_MY_FILES: &str = "namespace:my_files";
const NAMESPACE_SHARED_WITH_ME: &str = "namespace:shared_with_me";
const NAMESPACE_OFFLINE: &str = "namespace:offline";
const NAMESPACE_CONFLICTS: &str = "namespace:conflicts";
const CAP_READ: u32 = 1 << 0;
const CAP_WRITE: u32 = 1 << 1;
const CAP_RENAME: u32 = 1 << 2;
const CAP_DELETE: u32 = 1 << 3;

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

    loop {
        connections.retain(|handle| !handle.is_finished());
        tokio::select! {
            biased;
            _ = &mut cancel => break,
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) => {
                    let db = db.clone();
                    let bridge = bridge.clone();
                    connections.push(tokio::spawn(handle_connection(stream, db, bridge)));
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
    mut stream: UnixStream,
    db: std::sync::Arc<crate::state_db::StateDb>,
    bridge: std::sync::Arc<crate::engine_bridge::EngineBridge>,
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

    let mut buf = vec![0u8; 65536];
    loop {
        let n = match stream.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        let req: IpcRequest = match serde_json::from_slice(&buf[..n]) {
            Ok(r) => r,
            Err(e) => {
                let err = serde_json::to_vec(&IpcResponse::Error { message: e.to_string() }).unwrap();
                let _ = stream.write_all(&err).await;
                continue;
            }
        };
        let resp = match req {
            IpcRequest::GetFileStatus { file_id } => match db.get_file(&file_id) {
                Ok(Some(e)) => IpcResponse::FileStatus(file_entry_payload_for_db(&db, &e, NAMESPACE_MY_FILES)),
                _ => IpcResponse::Error {
                    message: "not found".into(),
                },
            },
            IpcRequest::ListFileProviderItems { container_id } => IpcResponse::FileProviderItems {
                items: list_file_provider_items(&db, &container_id),
            },
            IpcRequest::HydrateFile { file_id, dest_path } => {
                // `dest_path` arrives straight off the wire (untrusted). Bound
                // it to the caller's legitimate destinations before handing it
                // to `hydrate_file`, which decrypts vault plaintext to disk
                // (task 1247). The only real IPC caller (the File Provider
                // extension / FUSE mount) writes either under the sync root or
                // into the per-file temp cache, so both are allowed roots; if
                // no sync root is configured yet, only the temp dir is.
                // Resolve sync_root the same way the SetRecursivePin handler
                // below already does.
                //
                // Task 1670: on macOS, `temp_root` above is THIS (sandboxed)
                // process's own private container temp dir — the File Provider
                // extension is a DIFFERENT sandboxed process with its own
                // separate container temp dir, so a real destination it builds
                // can never be inside `temp_root` (see `macos_hydrate_cache_dir`'s
                // doc comment for the full root-cause). Add the shared App
                // Group hydrate-cache directory as a third allowed root, and
                // make sure it exists before the containment check runs.
                let sync_root = crate::config::DesktopConfig::load().ok().and_then(|cfg| cfg.sync_root);
                let temp_root = std::env::temp_dir();
                let dest = std::path::Path::new(&dest_path);
                #[cfg(target_os = "macos")]
                let macos_hydrate_dir = {
                    let dir = macos_hydrate_cache_dir();
                    if let Err(e) = std::fs::create_dir_all(&dir) {
                        tracing::warn!(error = %e, dir = %dir.display(), "could not create macOS hydrate-cache dir");
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
                let result = bridge.hydrate_file(&file_id, dest, &allowed_roots).await;
                match result {
                    Ok(_) => IpcResponse::Ok,
                    Err(e) => IpcResponse::Error { message: e.to_string() },
                }
            }
            IpcRequest::QueueFinderCreate {
                parent_id,
                filename,
                kind,
                contents_path,
                content_type,
            } => {
                if parent_id.as_deref() == Some(NAMESPACE_SHARED_WITH_ME) {
                    return write_ipc_response(
                        &mut stream,
                        IpcResponse::Error {
                            message: "Shared with me is read-only at the namespace root".into(),
                        },
                    )
                    .await;
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
                write_outcome_response(&db, bridge.queue_finder_create(target))
            }
            IpcRequest::QueueFinderModify {
                file_id,
                parent_id,
                filename,
                kind,
                contents_path,
                content_type,
                base_version_identifier,
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
                write_outcome_response(&db, bridge.queue_finder_modify(target))
            }
            IpcRequest::QueueFinderDelete {
                file_id,
                base_version_identifier,
            } => write_outcome_response(&db, bridge.queue_finder_delete(&file_id, base_version_identifier)),
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
            IpcRequest::SetFileStatus { .. } => IpcResponse::Ok,
        };
        let _ = stream.write_all(&serde_json::to_vec(&resp).unwrap()).await;
    }
}

async fn write_ipc_response(stream: &mut UnixStream, response: IpcResponse) {
    let _ = stream.write_all(&serde_json::to_vec(&response).unwrap()).await;
}

fn parse_write_kind(kind: &str) -> crate::engine_bridge::FinderWriteItemKind {
    if kind.eq_ignore_ascii_case("folder") || kind.eq_ignore_ascii_case("namespace") {
        crate::engine_bridge::FinderWriteItemKind::Folder
    } else {
        crate::engine_bridge::FinderWriteItemKind::File
    }
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
                .map(|entry| file_entry_payload_for_db(db, &entry, NAMESPACE_MY_FILES)),
            ignored,
            message,
        },
        Err(e) => IpcResponse::Error { message: e.to_string() },
    }
}

fn list_file_provider_items(db: &crate::state_db::StateDb, container_id: &str) -> Vec<FileProviderItemPayload> {
    if is_file_provider_root(container_id) {
        return vec![
            namespace_payload(NAMESPACE_MY_FILES, "My files"),
            namespace_payload(NAMESPACE_SHARED_WITH_ME, "Shared with me"),
            namespace_payload(NAMESPACE_OFFLINE, "Offline"),
            namespace_payload(NAMESPACE_CONFLICTS, "Conflicts"),
        ];
    }

    match container_id {
        NAMESPACE_MY_FILES => db
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
            .map(|entry| file_entry_payload_for_db(db, &entry, NAMESPACE_MY_FILES))
            .collect(),
        NAMESPACE_OFFLINE => db
            .list_by_status(crate::state_db::FileStatus::Local)
            .unwrap_or_default()
            .into_iter()
            .map(|entry| file_entry_payload_for_db(db, &entry, NAMESPACE_OFFLINE))
            .collect(),
        NAMESPACE_CONFLICTS => db
            .list_by_status(crate::state_db::FileStatus::Conflict)
            .unwrap_or_default()
            .into_iter()
            .map(|entry| file_entry_payload_for_db(db, &entry, NAMESPACE_CONFLICTS))
            .collect(),
        NAMESPACE_SHARED_WITH_ME => db
            .list_contract_states_by_namespace(crate::state_db::Namespace::SharedWithMe)
            .unwrap_or_default()
            .into_iter()
            .filter(|contract| contract.parent_id.is_none())
            .filter_map(|contract| {
                db.get_file(&contract.file_id)
                    .ok()
                    .flatten()
                    .map(|entry| file_entry_payload(&entry, &contract, NAMESPACE_SHARED_WITH_ME))
            })
            .collect(),
        _ => db
            .list_files()
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
            .collect(),
    }
}

fn namespace_payload(identifier: &str, filename: &str) -> FileProviderItemPayload {
    FileProviderItemPayload {
        identifier: identifier.to_string(),
        parent_identifier: FP_ROOT_APPLE.to_string(),
        filename: filename.to_string(),
        kind: "namespace".to_string(),
        size_bytes: 0,
        content_type: Some("public.folder".to_string()),
        status: "local".to_string(),
        capabilities: CAP_READ,
        version_identifier: None,
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
    match db.get_file_contract_state(&entry.file_id).ok().flatten() {
        Some(contract) => file_entry_payload(entry, &contract, parent_identifier),
        None => file_entry_payload_without_contract(entry, parent_identifier),
    }
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
    }
}

fn file_entry_payload_without_contract(
    entry: &crate::state_db::FileEntry,
    parent_identifier: &str,
) -> FileProviderItemPayload {
    let status = file_status_string(&entry.status);
    let capabilities = capabilities_for_status(&entry.status);

    FileProviderItemPayload {
        identifier: entry.file_id.clone(),
        parent_identifier: parent_identifier.to_string(),
        filename: filename_from_path(&entry.path),
        kind: "file".to_string(),
        size_bytes: entry.size_bytes,
        content_type: None,
        status: status.to_string(),
        capabilities,
        version_identifier: Some(format!(
            "{}:{}:{}",
            entry.remote_updated_at, entry.modified_at, entry.size_bytes
        )),
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
        crate::state_db::FileStatus::Local | crate::state_db::FileStatus::Uploading => {
            CAP_READ | CAP_WRITE | CAP_RENAME | CAP_DELETE
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
mod tests {
    use super::*;
    use crate::state_db::{FileEntry, FileStatus, ItemKind, Namespace, PERMISSION_READ, PERMISSION_WRITE, StateDb};
    use tempfile::tempdir;

    #[test]
    fn test_shared_namespace_lists_roots_with_permission_capabilities() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed_shared_root(&db, "read-root", "Shared with me/Read only", PERMISSION_READ);
        seed_shared_root(
            &db,
            "write-root",
            "Shared with me/Editable",
            PERMISSION_READ | PERMISSION_WRITE,
        );

        let items = list_file_provider_items(&db, NAMESPACE_SHARED_WITH_ME);
        assert_eq!(items.len(), 2);
        let read_only = items.iter().find(|item| item.identifier == "read-root").unwrap();
        assert_eq!(read_only.kind, "folder");
        assert_eq!(read_only.capabilities, CAP_READ);

        let editable = items.iter().find(|item| item.identifier == "write-root").unwrap();
        assert_eq!(
            editable.capabilities & (CAP_READ | CAP_WRITE | CAP_RENAME | CAP_DELETE),
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
}
