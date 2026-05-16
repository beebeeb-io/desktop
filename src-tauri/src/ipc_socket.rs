use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};

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
) {
    let path = ipc_socket_path();
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).expect("bind IPC socket");
    tracing::info!("IPC socket listening at {:?}", path);

    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let db = db.clone();
                let bridge = bridge.clone();
                tokio::spawn(handle_connection(stream, db, bridge));
            }
            Err(e) => tracing::warn!("IPC accept error: {e}"),
        }
    }
}

async fn handle_connection(
    mut stream: UnixStream,
    db: std::sync::Arc<crate::state_db::StateDb>,
    bridge: std::sync::Arc<crate::engine_bridge::EngineBridge>,
) {
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
                Ok(Some(e)) => IpcResponse::FileStatus(file_entry_payload(&e, NAMESPACE_MY_FILES)),
                _ => IpcResponse::Error {
                    message: "not found".into(),
                },
            },
            IpcRequest::ListFileProviderItems { container_id } => IpcResponse::FileProviderItems {
                items: list_file_provider_items(&db, &container_id),
            },
            IpcRequest::HydrateFile { file_id, dest_path } => {
                match bridge.hydrate_file(&file_id, std::path::Path::new(&dest_path)).await {
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
                let target = crate::engine_bridge::FinderWriteTarget {
                    file_id: None,
                    parent_id: normalize_parent_id(parent_id),
                    filename,
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
            IpcRequest::SetRecursivePin { file_id, pinned } => match bridge.set_recursive_pin(&file_id, pinned) {
                Ok(outcome) => IpcResponse::PinUpdated {
                    changed_item_ids: outcome.changed_item_ids,
                    hydrate_operations: outcome.hydrate_operations,
                },
                Err(e) => IpcResponse::Error { message: e.to_string() },
            },
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
                .map(|entry| file_entry_payload(&entry, NAMESPACE_MY_FILES)),
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
            .filter(|entry| is_top_level_path(&entry.path))
            .map(|entry| file_entry_payload(&entry, NAMESPACE_MY_FILES))
            .collect(),
        NAMESPACE_OFFLINE => db
            .list_by_status(crate::state_db::FileStatus::Local)
            .unwrap_or_default()
            .into_iter()
            .map(|entry| file_entry_payload(&entry, NAMESPACE_OFFLINE))
            .collect(),
        NAMESPACE_CONFLICTS => db
            .list_by_status(crate::state_db::FileStatus::Conflict)
            .unwrap_or_default()
            .into_iter()
            .map(|entry| file_entry_payload(&entry, NAMESPACE_CONFLICTS))
            .collect(),
        NAMESPACE_SHARED_WITH_ME => Vec::new(),
        _ => Vec::new(),
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

fn file_entry_payload(entry: &crate::state_db::FileEntry, parent_identifier: &str) -> FileProviderItemPayload {
    let status = file_status_string(&entry.status);
    let capabilities = match entry.status {
        crate::state_db::FileStatus::CloudOnly
        | crate::state_db::FileStatus::Downloading
        | crate::state_db::FileStatus::Error => CAP_READ,
        crate::state_db::FileStatus::Conflict => CAP_READ | CAP_RENAME,
        crate::state_db::FileStatus::Local | crate::state_db::FileStatus::Uploading => {
            CAP_READ | CAP_WRITE | CAP_RENAME | CAP_DELETE
        }
    };

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

fn file_status_string(status: &crate::state_db::FileStatus) -> &'static str {
    match status {
        crate::state_db::FileStatus::CloudOnly => "cloud_only",
        crate::state_db::FileStatus::Downloading => "downloading",
        crate::state_db::FileStatus::Local => "local",
        crate::state_db::FileStatus::Uploading => "uploading",
        crate::state_db::FileStatus::Conflict => "conflict",
        crate::state_db::FileStatus::Error => "error",
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
