use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};

#[derive(Debug, Serialize, Deserialize)]
pub enum IpcRequest {
    GetFileStatus { file_id: String },
    SetFileStatus { file_id: String, status: String },
    HydrateFile { file_id: String, dest_path: String },
    GetSyncSummary,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum IpcResponse {
    FileStatus { file_id: String, status: String },
    Ok,
    Error { message: String },
    SyncSummary {
        syncing: u32,
        cloud_only: u32,
        conflicts: u32,
    },
}

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
                let err = serde_json::to_vec(&IpcResponse::Error {
                    message: e.to_string(),
                })
                .unwrap();
                let _ = stream.write_all(&err).await;
                continue;
            }
        };
        let resp = match req {
            IpcRequest::GetFileStatus { file_id } => match db.get_file(&file_id) {
                Ok(Some(e)) => IpcResponse::FileStatus {
                    file_id: e.file_id,
                    status: format!("{:?}", e.status).to_lowercase(),
                },
                _ => IpcResponse::Error {
                    message: "not found".into(),
                },
            },
            IpcRequest::HydrateFile { file_id, dest_path } => {
                match bridge
                    .hydrate_file(&file_id, std::path::Path::new(&dest_path))
                    .await
                {
                    Ok(_) => IpcResponse::Ok,
                    Err(e) => IpcResponse::Error {
                        message: e.to_string(),
                    },
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
            _ => IpcResponse::Ok,
        };
        let _ = stream.write_all(&serde_json::to_vec(&resp).unwrap()).await;
    }
}
