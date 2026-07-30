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

pub fn ipc_socket_path() -> std::path::PathBuf {
    let runtime_dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into());
    std::path::PathBuf::from(runtime_dir).join("beebeeb-daemon.sock")
}

pub async fn serve_ipc(
    db: std::sync::Arc<crate::state_db::StateDb>,
    bridge: std::sync::Arc<crate::engine_bridge::EngineBridge>,
    cancel: oneshot::Receiver<()>,
) {
    serve_ipc_at(ipc_socket_path(), db, bridge, cancel).await
}

/// Real IPC server, bound to an explicit `path`. `serve_ipc` calls this with the
/// production `ipc_socket_path()`; tests bind it to a throwaway temp socket so
/// they can exercise the real accept/dispatch path without touching
/// `$XDG_RUNTIME_DIR/beebeeb-daemon.sock` (task 1247).
pub async fn serve_ipc_at(
    path: std::path::PathBuf,
    db: std::sync::Arc<crate::state_db::StateDb>,
    bridge: std::sync::Arc<crate::engine_bridge::EngineBridge>,
    mut cancel: oneshot::Receiver<()>,
) {
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).expect("bind IPC socket");
    // Harden the socket file to owner-only (0o600) so no other user can even
    // connect() — defense-in-depth with the per-connection peer-UID check
    // (task 1247). Best-effort: a chmod failure is logged, not fatal. This
    // mirrors `config.rs::DesktopConfig::save`'s 0o600 chmod precedent, but is
    // non-fatal here since this is a fire-and-forget async server loop.
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)) {
            tracing::warn!(error = %e, "failed to chmod IPC socket to 0o600");
        }
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
}

/// Read the connecting peer process's UID from a connected Unix stream.
///
/// Linux uses `SO_PEERCRED` (the kernel fills a `libc::ucred`). macOS and other
/// unix platforms would need `getpeereid()` / `LOCAL_PEERCRED` instead — that is
/// deliberately NOT implemented here: there is no macOS environment available to
/// verify the FFI struct/constant layout is correct, and macOS builds are
/// disabled in CI today (per `repos/desktop/CLAUDE.md`). Rather than ship
/// unverified, security-critical `unsafe` code, the non-Linux branch fails
/// closed (returns `Err`, so the caller rejects the connection). It MUST be
/// implemented and verified on real macOS hardware before macOS ships this
/// daemon — a blocking item in the macOS bring-up brief (task 1247).
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

#[cfg(not(target_os = "linux"))]
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
    // (task 1247). The `peer_uid` check fails closed on non-Linux unix.
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
                let sync_root = crate::config::DesktopConfig::load().ok().and_then(|cfg| cfg.sync_root);
                let temp_root = std::env::temp_dir();
                let dest = std::path::Path::new(&dest_path);
                let result = match &sync_root {
                    Some(root) => bridge.hydrate_file(&file_id, dest, &[root.as_path(), temp_root.as_path()]).await,
                    None => bridge.hydrate_file(&file_id, dest, &[temp_root.as_path()]).await,
                };
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
}
