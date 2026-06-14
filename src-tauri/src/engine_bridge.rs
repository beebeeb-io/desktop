//! Bridge between the Tauri runner, the SQLite state DB, and the
//! Beebeeb API client. Everything that "does sync work" goes through
//! this module:
//!
//! - **State machine** ([`FileSM`]) — explicit transition table for
//!   per-file lifecycle. The plan's value here is preventing nonsense
//!   transitions like Uploading → Downloading mid-flight.
//! - **`EngineBridge`** — owns the [`StateDb`] + [`ApiClient`] handles
//!   and exposes async operations: `hydrate_file` (cloud-only → local
//!   on demand) and the periodic `sync_tick` (metadata sweep, called
//!   from [`crate::runner`]).
//!
//! Spec: `docs/superpowers/plans/2026-05-07-desktop-sync-client.md`
//! Phase 1 Task 3.
//!
//! ## Divergence from the plan
//!
//! The plan's `hydrate_file` body referenced
//! `beebeeb_core::crypto::derive_file_key` and a raw `nonce || ciphertext`
//! chunk format. The real API surface is
//! `beebeeb_core::kdf::derive_file_key(&MasterKey, &[u8]) -> FileKey` and
//! `beebeeb_core::encrypt::decrypt_chunk(&FileKey, &EncryptedBlob)`, with
//! chunks stored on the wire as JSON-serialised `EncryptedBlob`s (matches
//! `repos/cli/src/commands/push.rs`'s `serde_json::to_vec(&blob)` upload
//! path and `pull.rs`'s `serde_json::from_slice` decode path). The
//! implementation below uses the real API.
//!
//! The per-chunk GET endpoint (`GET /api/v1/files/:id/chunks/:idx`) was
//! added in `server` commit `d3cf0e2`. Before that landed, `hydrate_file`
//! was a stub returning `anyhow::Err`.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use beebeeb_types::EncryptedBlob;
use serde::{Deserialize, Serialize};

use crate::api_client::{ApiClient, DesktopUploadInitRequest};
use crate::conflict::{VersionInfo, is_conflict, is_text_file};
use crate::state_db::{
    FileContractState, FileEntry, FileStatus, ItemKind, Namespace, OperationKind, OperationPauseReason,
    PERMISSION_OWNER, PERMISSION_READ, PERMISSION_SHARE, PERMISSION_WRITE, PendingOperation, QueueDiagnostics, StateDb,
};

// ── Per-file state machine ────────────────────────────────────────────────────

/// Events that drive a file through its sync lifecycle.
pub enum FileEvent {
    DownloadStart,
    DownloadComplete,
    DownloadFail,
    UploadStart,
    UploadComplete,
    UploadFail,
    ConflictDetected,
    ConflictResolved,
    Evict,
}

/// In-memory state tracker for a single file. The persisted state is
/// in [`StateDb`]; this struct exists so callers can validate the
/// _next_ state before writing it.
pub struct FileSM {
    state: FileStatus,
}

impl FileSM {
    pub fn new(state: FileStatus) -> Self {
        Self { state }
    }
    pub fn state(&self) -> FileStatus {
        self.state.clone()
    }

    /// Apply `event`. Returns `Err` if the event isn't legal in the
    /// current state — callers should treat that as a programming bug,
    /// not a runtime error to recover from. The whitelist below is the
    /// canonical sync state machine; deny by default.
    pub fn transition(&mut self, event: FileEvent) -> Result<(), &'static str> {
        use FileEvent::*;
        use FileStatus::*;
        self.state = match (&self.state, event) {
            (CloudOnly, DownloadStart) => Downloading,
            (Downloading, DownloadComplete) => Local,
            (Downloading, DownloadFail) => Error,
            (Local, UploadStart) => Uploading,
            (Uploading, UploadComplete) => Local,
            (Uploading, UploadFail) => Error,
            (Local, ConflictDetected) => Conflict,
            (Conflict, ConflictResolved) => Local,
            (Local, Evict) => CloudOnly,
            _ => return Err("invalid state transition"),
        };
        Ok(())
    }
}

// ── EngineBridge ──────────────────────────────────────────────────────────────

pub struct EngineBridge {
    db: Arc<StateDb>,
    api: Arc<ApiClient>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FinderWriteItemKind {
    File,
    Folder,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FinderWriteTarget {
    pub file_id: Option<String>,
    pub parent_id: Option<String>,
    pub filename: String,
    pub kind: FinderWriteItemKind,
    pub contents_path: Option<String>,
    pub content_type: Option<String>,
    pub base_version_identifier: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FinderWriteOutcome {
    Queued {
        op_id: String,
        file_id: Option<String>,
        kind: OperationKind,
        ignored: bool,
        message: String,
    },
    Ignored {
        message: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachePolicy {
    pub max_unpinned_cache_bytes: i64,
    pub disk_pressure_min_free_bytes: u64,
}

impl Default for CachePolicy {
    fn default() -> Self {
        Self {
            max_unpinned_cache_bytes: 2 * 1024 * 1024 * 1024,
            disk_pressure_min_free_bytes: 5 * 1024 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PinUpdateOutcome {
    pub changed_item_ids: Vec<String>,
    pub hydrate_operations: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheCleanupOutcome {
    pub evicted_file_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VersionConflictEntry {
    pub id: String,
    pub file_id: String,
    pub file_name: String,
    pub kind: String,
    pub status: String,
    pub updated_at: Option<i64>,
    pub detail: String,
    pub action: String,
    pub op_id: Option<String>,
    pub version_id: Option<String>,
    pub base_version: Option<i64>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SharedRootRefreshOutcome {
    pub active_shared_root_ids: Vec<String>,
    pub removed_shared_file_ids: Vec<String>,
    pub removed_cache_paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TransferLoopOutcome {
    pub completed_op_ids: Vec<String>,
    pub retried_op_ids: Vec<String>,
    pub paused_op_ids: Vec<String>,
    pub invalidated_item_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationFailureClass {
    Retryable,
    Auth,
    Quota,
    Permission,
    Locked,
}

impl OperationFailureClass {
    fn pause_reason(self) -> Option<OperationPauseReason> {
        match self {
            OperationFailureClass::Retryable => None,
            OperationFailureClass::Auth => Some(OperationPauseReason::Auth),
            OperationFailureClass::Quota => Some(OperationPauseReason::Quota),
            OperationFailureClass::Permission => Some(OperationPauseReason::Permission),
            OperationFailureClass::Locked => Some(OperationPauseReason::Locked),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SharedRootMapping {
    invite_id: String,
    file_id: String,
    display_name: String,
    is_folder: bool,
    size_bytes: i64,
    content_type: Option<String>,
    permission_bits: i64,
    approved_at: Option<i64>,
}

impl EngineBridge {
    pub fn new(db: Arc<StateDb>, api: Arc<ApiClient>) -> Self {
        Self { db, api }
    }

    /// Borrow the underlying state DB so callers (e.g. [`sync_tick`])
    /// can do their own writes without piping every operation through
    /// the bridge.
    pub fn db(&self) -> &StateDb {
        &self.db
    }

    /// Borrow the API client for the same reason.
    pub fn api(&self) -> &ApiClient {
        &self.api
    }

    pub fn queue_diagnostics(&self, now: i64) -> anyhow::Result<QueueDiagnostics> {
        Ok(self.db.queue_diagnostics(now)?)
    }

    pub async fn process_due_operations(&self, sync_root: &Path, now: i64) -> anyhow::Result<TransferLoopOutcome> {
        let mut outcome = TransferLoopOutcome {
            completed_op_ids: Vec::new(),
            retried_op_ids: Vec::new(),
            paused_op_ids: Vec::new(),
            invalidated_item_ids: Vec::new(),
        };
        let operations = self.db.list_due_operations(now)?;

        for op in operations {
            let result = self.execute_operation(&op, sync_root).await;
            match result {
                Ok(()) => {
                    self.db.remove_operation(&op.op_id)?;
                    if let Some(file_id) = &op.file_id {
                        outcome.invalidated_item_ids.push(file_id.clone());
                    }
                    outcome.completed_op_ids.push(op.op_id);
                }
                Err(error) => {
                    let class = classify_operation_error(&error.to_string());
                    if let Some(reason) = class.pause_reason() {
                        self.db
                            .record_operation_pause(&op.op_id, reason, Some(&error.to_string()), now)?;
                        outcome.paused_op_ids.push(op.op_id);
                    } else {
                        let attempts = op.attempts.saturating_add(1);
                        let next_retry_at = now.saturating_add(retry_delay_seconds(attempts));
                        self.db.record_operation_attempt(
                            &op.op_id,
                            attempts,
                            next_retry_at,
                            Some(&error.to_string()),
                        )?;
                        outcome.retried_op_ids.push(op.op_id);
                    }
                }
            }
        }

        Ok(outcome)
    }

    async fn execute_operation(&self, op: &PendingOperation, sync_root: &Path) -> anyhow::Result<()> {
        match op.kind {
            OperationKind::PinTree => Ok(()),
            OperationKind::HydrateFile => {
                let file_id = op
                    .file_id
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("hydrate operation missing file_id"))?;
                let dest = if let Some(target_path) = op.target_path.as_deref() {
                    sync_root.join(target_path.trim_start_matches('/'))
                } else if let Some(entry) = self.db.get_file(file_id)? {
                    sync_root.join(entry.path.trim_start_matches('/'))
                } else {
                    return Err(anyhow::anyhow!("hydrate operation target missing from state"));
                };
                self.hydrate_file(file_id, &dest).await
            }
            OperationKind::CreateFolder => {
                let metadata = operation_metadata(op)?;
                let name = metadata["name_encrypted"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("create folder operation missing encrypted name"))?;
                self.api
                    .create_folder(name, op.parent_id.as_deref(), op.file_id.as_deref())
                    .await?;
                Ok(())
            }
            OperationKind::MoveFile | OperationKind::RenameFile => {
                let file_id = op
                    .file_id
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("metadata operation missing file_id"))?;
                let metadata = operation_metadata(op)?;
                let name = metadata["name_encrypted"].as_str();
                self.api.update_metadata(file_id, name, op.parent_id.as_deref()).await?;
                Ok(())
            }
            OperationKind::TrashFile => {
                let file_id = op
                    .file_id
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("trash operation missing file_id"))?;
                self.api.trash_file(file_id).await?;
                Ok(())
            }
            OperationKind::RestoreFile => {
                let file_id = op
                    .file_id
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("restore file operation missing file_id"))?;
                self.api.restore_file(file_id).await?;
                Ok(())
            }
            OperationKind::RestoreVersion => {
                let file_id = op
                    .file_id
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("restore operation missing file_id"))?;
                let metadata = operation_metadata(op)?;
                let version_id = metadata["version_id"]
                    .as_str()
                    .or(op.base_object_version_id.as_deref())
                    .ok_or_else(|| anyhow::anyhow!("restore operation missing version id"))?;
                self.api.restore_version(file_id, version_id).await?;
                Ok(())
            }
            OperationKind::UploadVersion | OperationKind::UploadFile => self.upload_version(op).await,
        }
    }

    async fn upload_version(&self, op: &PendingOperation) -> anyhow::Result<()> {
        let local_file_id = op
            .file_id
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("upload operation missing file_id"))?;
        let payload_path = op
            .payload_path
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("upload operation missing staged payload"))?;
        let payload_path = Path::new(payload_path);
        if !payload_path.is_file() {
            return Err(anyhow::anyhow!(
                "staged upload payload is missing: {}",
                payload_path.display()
            ));
        }

        let metadata = operation_metadata(op)?;
        let name_encrypted = metadata["name_encrypted"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("upload operation missing encrypted name"))?;
        let content_type = metadata["content_type"].as_str().map(str::to_string);
        let plaintext_size = std::fs::metadata(payload_path)?.len();
        if plaintext_size == 0 {
            return Err(anyhow::anyhow!(
                "empty Finder uploads are not supported by the v2 upload endpoint yet"
            ));
        }

        let init_request = upload_init_request_for_operation(
            local_file_id,
            name_encrypted,
            content_type.clone(),
            op.parent_id.clone(),
            plaintext_size,
            op.base_version,
            is_create_file_operation(&metadata),
        );
        let upload = self.api.init_upload(&init_request).await?;

        let server_file_id = upload.file_id.clone();
        let effective_name_encrypted = if server_file_id != local_file_id {
            encrypted_metadata_for_name(
                self.api.master_key(),
                &server_file_id,
                metadata_display_name(&metadata, op)
                    .as_deref()
                    .unwrap_or(&server_file_id),
                content_type.as_deref(),
            )?
        } else {
            name_encrypted.to_string()
        };
        self.api
            .update_metadata(
                &server_file_id,
                Some(&effective_name_encrypted),
                op.parent_id.as_deref(),
            )
            .await?;

        let mk_bytes: [u8; 32] = *self.api.master_key();
        let master_key = beebeeb_core::kdf::MasterKey::from_bytes(mk_bytes);
        let file_key = beebeeb_core::kdf::derive_file_key(&master_key, server_file_id.as_bytes());

        let mut file = std::fs::File::open(payload_path)?;
        let chunk_size = upload.chunk_size_bytes as usize;
        let chunk_count = upload.chunk_count as u64;
        if chunk_size == 0 || chunk_count == 0 {
            return Err(anyhow::anyhow!("upload init returned invalid chunk plan"));
        }
        let mut buffer = vec![0u8; chunk_size];
        for chunk_index in 0..chunk_count {
            let read = file.read(&mut buffer)?;
            if read == 0 {
                return Err(anyhow::anyhow!(
                    "staged upload ended before expected chunk {} of {}",
                    chunk_index + 1,
                    chunk_count
                ));
            }
            let encrypted = beebeeb_core::encrypt::encrypt_chunk_raw(&file_key, &buffer[..read])
                .map_err(|e| anyhow::anyhow!("encrypt upload chunk {chunk_index}: {e}"))?;
            self.api
                .upload_session_chunk(&upload.upload_session_id, chunk_index as u32, &encrypted)
                .await?;
        }

        let completed = self.api.complete_upload_session(&upload.upload_session_id).await?;
        self.apply_completed_upload(
            local_file_id,
            &server_file_id,
            op,
            &completed,
            plaintext_size,
            content_type,
            Some(upload.object_version_id),
        )?;
        if let Err(e) = std::fs::remove_file(payload_path) {
            if e.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!(path = %payload_path.display(), error = %e, "failed to remove staged upload payload");
            }
        }
        Ok(())
    }

    fn apply_completed_upload(
        &self,
        local_file_id: &str,
        server_file_id: &str,
        op: &PendingOperation,
        completed: &serde_json::Value,
        plaintext_size: u64,
        content_type: Option<String>,
        object_version_id: Option<String>,
    ) -> anyhow::Result<()> {
        let now = now_secs();
        let mut entry = self.db.get_file(local_file_id)?.unwrap_or_else(|| FileEntry {
            file_id: server_file_id.to_string(),
            path: op.target_path.clone().unwrap_or_else(|| server_file_id.to_string()),
            status: FileStatus::Local,
            size_bytes: plaintext_size as i64,
            modified_at: now,
            content_hash: None,
            remote_updated_at: now,
        });
        entry.file_id = server_file_id.to_string();
        if let Some(target_path) = op.target_path.as_ref() {
            entry.path = target_path.clone();
        }
        entry.status = FileStatus::Local;
        entry.size_bytes = completed["size_bytes"].as_i64().unwrap_or(plaintext_size as i64);
        entry.modified_at = now;
        entry.remote_updated_at = now;
        self.db.upsert_file(&entry)?;

        let mut contract = self
            .db
            .get_file_contract_state(local_file_id)?
            .unwrap_or_else(|| FileContractState {
                file_id: server_file_id.to_string(),
                namespace: Namespace::MyFiles,
                parent_id: None,
                shared_root_id: None,
                share_id: None,
                permission_bits: PERMISSION_READ | PERMISSION_WRITE | PERMISSION_OWNER,
                item_kind: ItemKind::File,
                content_type: None,
                current_version: 0,
                current_object_version_id: None,
                local_base_version: 0,
                local_hash: None,
                cache_path: None,
                cache_bytes: 0,
                pin_state: crate::state_db::PinState::Inherit,
                inherited_pin_state: crate::state_db::PinState::Inherit,
                last_sync_at: 0,
            });
        contract.file_id = server_file_id.to_string();
        contract.item_kind = ItemKind::File;
        contract.content_type = content_type.or_else(|| completed["mime_type"].as_str().map(str::to_string));
        contract.parent_id = op.parent_id.clone();
        contract.current_version = completed["version_number"]
            .as_i64()
            .unwrap_or(contract.current_version.saturating_add(1));
        contract.local_base_version = contract.current_version;
        contract.current_object_version_id = completed["current_object_version_id"]
            .as_str()
            .map(str::to_string)
            .or(object_version_id);
        contract.last_sync_at = now;
        self.db.set_file_contract_state(&contract)?;
        if local_file_id != server_file_id {
            self.db.delete_file(local_file_id)?;
        }
        Ok(())
    }

    pub async fn refresh_shared_roots(&self) -> anyhow::Result<SharedRootRefreshOutcome> {
        let body = self.api.list_shared_roots().await?;
        let roots = shared_roots_from_invite_response(&body);
        let active_shared_root_ids: Vec<String> = roots.iter().map(|root| root.file_id.clone()).collect();
        let now = now_secs();

        for root in &roots {
            self.db.upsert_file(&FileEntry {
                file_id: root.file_id.clone(),
                path: format!("Shared with me/{}", root.display_name),
                status: FileStatus::CloudOnly,
                size_bytes: root.size_bytes,
                modified_at: root.approved_at.unwrap_or(now),
                content_hash: None,
                remote_updated_at: root.approved_at.unwrap_or(0),
            })?;

            let mut contract = self
                .db
                .get_file_contract_state(&root.file_id)?
                .ok_or_else(|| anyhow::anyhow!("missing state row for shared root {}", root.file_id))?;
            contract.namespace = Namespace::SharedWithMe;
            contract.parent_id = None;
            contract.shared_root_id = Some(root.file_id.clone());
            contract.share_id = Some(root.invite_id.clone());
            contract.permission_bits = root.permission_bits;
            contract.item_kind = if root.is_folder {
                ItemKind::Folder
            } else {
                ItemKind::File
            };
            contract.content_type = root.content_type.clone();
            contract.last_sync_at = now;
            self.db.set_file_contract_state(&contract)?;
        }

        let removed = self.db.purge_revoked_shared_content(&active_shared_root_ids)?;
        let mut removed_shared_file_ids = Vec::new();
        let mut removed_cache_paths = Vec::new();
        for cache in removed {
            removed_shared_file_ids.push(cache.file_id);
            if let Some(path) = cache.cache_path {
                if let Err(e) = std::fs::remove_file(&path) {
                    if e.kind() != std::io::ErrorKind::NotFound {
                        tracing::warn!(path = %path, error = %e, "failed to remove revoked shared cache file");
                    }
                }
                removed_cache_paths.push(path);
            }
        }

        Ok(SharedRootRefreshOutcome {
            active_shared_root_ids,
            removed_shared_file_ids,
            removed_cache_paths,
        })
    }

    pub fn queue_finder_create(&self, target: FinderWriteTarget) -> anyhow::Result<FinderWriteOutcome> {
        if is_ignored_finder_name(&target.filename) {
            return Ok(FinderWriteOutcome::Ignored {
                message: format!("ignored temporary Finder item {}", target.filename),
            });
        }

        let parent_contract = self.ensure_shared_parent_allows_write(target.parent_id.as_deref())?;
        let file_id = uuid::Uuid::new_v4().to_string();
        match target.kind {
            FinderWriteItemKind::Folder => {
                let metadata = encrypted_metadata_for_name(self.api.master_key(), &file_id, &target.filename, None)?;
                let mut payload = serde_json::json!({
                    "operation": "create_folder",
                    "name_encrypted": metadata,
                });
                apply_shared_context(&mut payload, parent_contract.as_ref());
                self.enqueue_finder_operation(
                    OperationKind::CreateFolder,
                    Some(file_id),
                    target.parent_id,
                    Some(target.filename),
                    payload,
                    None,
                    None,
                    None,
                )
            }
            FinderWriteItemKind::File => {
                let contents_path = target
                    .contents_path
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("Finder create file callback did not include contents"))?;
                let staged_path = stage_finder_payload(contents_path)?;
                let size_bytes = std::fs::metadata(&staged_path).map(|m| m.len() as i64).unwrap_or(0);
                let mime = target
                    .content_type
                    .as_deref()
                    .or_else(|| beebeeb_core::media::guess_mime_type(&target.filename));
                let name_encrypted =
                    encrypted_metadata_for_name(self.api.master_key(), &file_id, &target.filename, mime)?;
                self.db.upsert_file(&FileEntry {
                    file_id: file_id.clone(),
                    path: target.filename.clone(),
                    status: FileStatus::Uploading,
                    size_bytes,
                    modified_at: now_secs(),
                    content_hash: None,
                    remote_updated_at: 0,
                })?;
                let mut payload = serde_json::json!({
                    "operation": "create_file",
                    "name_encrypted": name_encrypted,
                    "content_type": target.content_type,
                    "uploaded_by": "authenticated_desktop_user",
                });
                apply_shared_context(&mut payload, parent_contract.as_ref());
                self.enqueue_finder_operation(
                    OperationKind::UploadVersion,
                    Some(file_id),
                    target.parent_id,
                    Some(target.filename),
                    payload,
                    Some(staged_path),
                    None,
                    None,
                )
            }
        }
    }

    pub fn queue_finder_modify(&self, target: FinderWriteTarget) -> anyhow::Result<FinderWriteOutcome> {
        if is_ignored_finder_name(&target.filename) {
            return Ok(FinderWriteOutcome::Ignored {
                message: format!("ignored temporary Finder item {}", target.filename),
            });
        }

        let file_id = target
            .file_id
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Finder modify callback did not include a file id"))?;
        let item_contract = self.ensure_item_allows_shared_write(&file_id, "modify")?;

        if let Some(contents_path) = target.contents_path.as_deref() {
            let staged_path = stage_finder_payload(contents_path)?;
            let size_bytes = std::fs::metadata(&staged_path).map(|m| m.len() as i64).unwrap_or(0);
            let mime = target
                .content_type
                .as_deref()
                .or_else(|| beebeeb_core::media::guess_mime_type(&target.filename));
            let name_encrypted = encrypted_metadata_for_name(self.api.master_key(), &file_id, &target.filename, mime)?;
            self.db.set_status(&file_id, FileStatus::Uploading)?;
            let mut payload = serde_json::json!({
                "operation": "upload_version",
                "name_encrypted": name_encrypted,
                "content_type": target.content_type,
                "size_bytes": size_bytes,
                "base_version_identifier": target.base_version_identifier,
                "uploaded_by": "authenticated_desktop_user",
            });
            apply_shared_context(&mut payload, item_contract.as_ref());
            self.enqueue_finder_operation(
                OperationKind::UploadVersion,
                Some(file_id),
                target.parent_id,
                Some(target.filename),
                payload,
                Some(staged_path),
                parse_base_version_number(target.base_version_identifier.as_deref()),
                item_contract
                    .as_ref()
                    .and_then(|contract| contract.current_object_version_id.clone()),
            )
        } else {
            let name_encrypted = encrypted_metadata_for_name(
                self.api.master_key(),
                &file_id,
                &target.filename,
                target.content_type.as_deref(),
            )?;
            let kind = if target.parent_id.is_some() {
                OperationKind::MoveFile
            } else {
                OperationKind::RenameFile
            };
            let mut payload = serde_json::json!({
                "operation": "metadata_update",
                "name_encrypted": name_encrypted,
                "base_version_identifier": target.base_version_identifier,
            });
            apply_shared_context(&mut payload, item_contract.as_ref());
            self.enqueue_finder_operation(
                kind,
                Some(file_id),
                target.parent_id,
                Some(target.filename),
                payload,
                None,
                None,
                item_contract
                    .as_ref()
                    .and_then(|contract| contract.current_object_version_id.clone()),
            )
        }
    }

    pub fn queue_finder_delete(
        &self,
        file_id: &str,
        base_version_identifier: Option<String>,
    ) -> anyhow::Result<FinderWriteOutcome> {
        let item_contract = self.ensure_item_allows_shared_write(file_id, "delete")?;
        let mut payload = serde_json::json!({
            "operation": "trash",
            "base_version_identifier": base_version_identifier,
        });
        apply_shared_context(&mut payload, item_contract.as_ref());
        self.enqueue_finder_operation(
            OperationKind::TrashFile,
            Some(file_id.to_string()),
            None,
            None,
            payload,
            None,
            parse_base_version_number(base_version_identifier.as_deref()),
            item_contract
                .as_ref()
                .and_then(|contract| contract.current_object_version_id.clone()),
        )
    }

    pub fn set_recursive_pin(&self, file_id: &str, pinned: bool) -> anyhow::Result<PinUpdateOutcome> {
        let now = now_secs();
        let changed_item_ids = self.db.set_recursive_pin(file_id, pinned, now)?;
        let mut hydrate_operations = 0usize;

        self.enqueue_pin_tree_operation(file_id, pinned, now)?;
        if pinned {
            for changed_id in &changed_item_ids {
                let Some(entry) = self.db.get_file(changed_id)? else {
                    continue;
                };
                let Some(contract) = self.db.get_file_contract_state(changed_id)? else {
                    continue;
                };
                if contract.effective_pin_state() == crate::state_db::PinState::Pinned
                    && entry.status == FileStatus::CloudOnly
                    && entry.size_bytes > 0
                {
                    self.enqueue_hydrate_operation(&entry, now)?;
                    hydrate_operations += 1;
                }
            }
        }

        Ok(PinUpdateOutcome {
            changed_item_ids,
            hydrate_operations,
        })
    }

    pub fn record_smart_cache_open(
        &self,
        file_id: &str,
        cache_path: &Path,
        cache_bytes: i64,
    ) -> anyhow::Result<CacheCleanupOutcome> {
        self.db
            .mark_cached(file_id, &cache_path.to_string_lossy(), cache_bytes, now_secs())?;
        self.enforce_smart_cache(CachePolicy::default())
    }

    pub fn enforce_smart_cache(&self, policy: CachePolicy) -> anyhow::Result<CacheCleanupOutcome> {
        let evicted = self
            .db
            .evict_unpinned_cache_until_under(policy.max_unpinned_cache_bytes, now_secs())?;
        Ok(CacheCleanupOutcome {
            evicted_file_ids: evicted,
        })
    }

    pub fn version_conflict_feed(&self) -> anyhow::Result<Vec<VersionConflictEntry>> {
        version_conflict_feed_from_db(self.db.as_ref())
    }

    pub fn queue_restore_version(
        &self,
        file_id: &str,
        version_id: &str,
        last_error: Option<String>,
    ) -> anyhow::Result<PendingOperation> {
        let now = now_secs();
        let op = PendingOperation {
            op_id: uuid::Uuid::new_v4().to_string(),
            kind: OperationKind::RestoreVersion,
            file_id: Some(file_id.to_string()),
            parent_id: None,
            target_path: None,
            metadata_json: Some(
                serde_json::json!({
                    "operation": "restore_version",
                    "version_id": version_id,
                })
                .to_string(),
            ),
            payload_path: None,
            base_version: None,
            base_object_version_id: Some(version_id.to_string()),
            attempts: 0,
            max_attempts: 25,
            next_retry_at: now,
            last_error,
            created_at: now,
            updated_at: now,
        };
        self.db.enqueue_operation(&op)?;
        Ok(op)
    }

    fn enqueue_finder_operation(
        &self,
        kind: OperationKind,
        file_id: Option<String>,
        parent_id: Option<String>,
        target_path: Option<String>,
        metadata: serde_json::Value,
        payload_path: Option<String>,
        base_version: Option<i64>,
        base_object_version_id: Option<String>,
    ) -> anyhow::Result<FinderWriteOutcome> {
        let op_id = uuid::Uuid::new_v4().to_string();
        let now = now_secs();
        let op = PendingOperation {
            op_id: op_id.clone(),
            kind: kind.clone(),
            file_id: file_id.clone(),
            parent_id,
            target_path,
            metadata_json: Some(serde_json::to_string(&metadata)?),
            payload_path,
            base_version,
            base_object_version_id,
            attempts: 0,
            max_attempts: 25,
            next_retry_at: now,
            last_error: Some("queued from Finder; upload worker not yet attached".to_string()),
            created_at: now,
            updated_at: now,
        };
        self.db.enqueue_operation(&op)?;
        Ok(FinderWriteOutcome::Queued {
            op_id,
            file_id,
            kind,
            ignored: false,
            message: "queued for encrypted sync".to_string(),
        })
    }

    fn enqueue_pin_tree_operation(&self, file_id: &str, pinned: bool, now: i64) -> anyhow::Result<()> {
        let op = PendingOperation {
            op_id: uuid::Uuid::new_v4().to_string(),
            kind: OperationKind::PinTree,
            file_id: Some(file_id.to_string()),
            parent_id: None,
            target_path: None,
            metadata_json: Some(
                serde_json::json!({
                    "operation": "pin_tree",
                    "pinned": pinned,
                })
                .to_string(),
            ),
            payload_path: None,
            base_version: None,
            base_object_version_id: None,
            attempts: 0,
            max_attempts: 25,
            next_retry_at: now,
            last_error: Some("queued recursive pin state; hydration worker not yet attached".to_string()),
            created_at: now,
            updated_at: now,
        };
        self.db.enqueue_operation(&op)?;
        Ok(())
    }

    fn enqueue_hydrate_operation(&self, entry: &FileEntry, now: i64) -> anyhow::Result<()> {
        let op = PendingOperation {
            op_id: uuid::Uuid::new_v4().to_string(),
            kind: OperationKind::HydrateFile,
            file_id: Some(entry.file_id.clone()),
            parent_id: None,
            target_path: Some(entry.path.clone()),
            metadata_json: Some(
                serde_json::json!({
                    "operation": "hydrate_file",
                    "reason": "recursive_pin",
                })
                .to_string(),
            ),
            payload_path: None,
            base_version: None,
            base_object_version_id: None,
            attempts: 0,
            max_attempts: 25,
            next_retry_at: now,
            last_error: Some("queued pinned content hydration; transfer worker not yet attached".to_string()),
            created_at: now,
            updated_at: now,
        };
        self.db.enqueue_operation(&op)?;
        Ok(())
    }

    fn ensure_shared_parent_allows_write(&self, parent_id: Option<&str>) -> anyhow::Result<Option<FileContractState>> {
        let Some(parent_id) = parent_id else {
            return Ok(None);
        };
        if parent_id == "namespace:shared_with_me" {
            return Err(anyhow::anyhow!(
                "Shared with me is read-only at the namespace root; open an editable shared folder first"
            ));
        }
        let Some(contract) = self.db.get_file_contract_state(parent_id)? else {
            return Ok(None);
        };
        if contract.is_shared() && !contract.can_write() {
            return Err(anyhow::anyhow!(
                "read-only shared folder cannot accept new Finder items"
            ));
        }
        Ok(Some(contract).filter(|contract| contract.is_shared()))
    }

    fn ensure_item_allows_shared_write(
        &self,
        file_id: &str,
        operation: &str,
    ) -> anyhow::Result<Option<FileContractState>> {
        let Some(contract) = self.db.get_file_contract_state(file_id)? else {
            return Ok(None);
        };
        if contract.is_shared() && !contract.can_write() {
            return Err(anyhow::anyhow!(
                "read-only shared item cannot be changed from Finder during {operation}"
            ));
        }
        Ok(Some(contract).filter(|contract| contract.is_shared()))
    }

    /// Download `file_id` from the vault, decrypt, write to `dest_path`.
    /// Called by an OS extension when a cloud-only placeholder is first
    /// opened (Tasks 5-7) and by `bb pull`-style flows we may add later.
    ///
    /// Steps mirror `repos/cli/src/commands/pull.rs::pull_single_file`:
    ///
    /// 1. Flip status to `Downloading` so the Finder/Explorer overlay
    ///    shows a spinner immediately.
    /// 2. Fetch fresh metadata to learn `chunk_count` (the local
    ///    placeholder may not have it).
    /// 3. Derive per-file key via `beebeeb_core::kdf::derive_file_key`
    ///    over the file's UUID bytes (NOT the string).
    /// 4. For each chunk index: GET the bytes, parse as
    ///    `EncryptedBlob` JSON, decrypt with `decrypt_chunk`,
    ///    accumulate plaintext.
    /// 5. Write the whole plaintext to `dest_path` (caller chose the
    ///    layout — `dest_path` is usually `<sync_root>/<decrypted_name>`
    ///    or a per-extension cache path).
    /// 6. Flip status to `Local`.
    ///
    /// On any error the status is flipped to `Error` so the overlay can
    /// render a problem indicator, then the error is bubbled.
    pub async fn hydrate_file(&self, file_id: &str, dest_path: &Path) -> anyhow::Result<()> {
        // RAII-style: any early return below the status flip should
        // leave the file in `Error`, not `Downloading`. We do that by
        // wrapping the body in an inner async fn whose Err branch we
        // catch.
        self.db.set_status(file_id, FileStatus::Downloading)?;
        match self.do_hydrate(file_id, dest_path).await {
            Ok(()) => {
                self.db.set_status(file_id, FileStatus::Local)?;
                let cache_bytes = std::fs::metadata(dest_path).map(|m| m.len() as i64).unwrap_or(0);
                self.db
                    .mark_cached(file_id, &dest_path.to_string_lossy(), cache_bytes, now_secs())?;
                let _ = self.enforce_smart_cache(CachePolicy::default());
                Ok(())
            }
            Err(e) => {
                // Best-effort status flip; if the DB is broken we still
                // return the original error.
                let _ = self.db.set_status(file_id, FileStatus::Error);
                Err(e)
            }
        }
    }

    async fn do_hydrate(&self, file_id: &str, dest_path: &Path) -> anyhow::Result<()> {
        let _file_uuid: uuid::Uuid = file_id
            .parse()
            .map_err(|e| anyhow::anyhow!("invalid file_id (not a UUID): {e}"))?;

        // Per-file metadata. We trust the server's chunk_count rather
        // than the local entry's because a file we know about as
        // `cloud_only` may have been re-uploaded with a new chunk
        // layout since we last saw it.
        let meta = self.api.get_file(file_id).await?;
        let chunk_count = meta
            .get("chunk_count")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| anyhow::anyhow!("server response missing chunk_count"))? as u32;

        // Derive the per-file key. MasterKey::from_bytes consumes the
        // array (it zeroizes on drop), so we copy from the borrow.
        let mk_bytes: [u8; 32] = *self.api.master_key();
        let master_key = beebeeb_core::kdf::MasterKey::from_bytes(mk_bytes);
        let file_key = beebeeb_core::kdf::derive_file_key(&master_key, file_id.as_bytes());

        // Walk chunks. Pre-allocate roughly the file size if known,
        // but fall back to defaults — chunks are encrypted so the
        // ciphertext is always larger than plaintext anyway.
        let approx_size = meta.get("size_bytes").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        let mut plaintext: Vec<u8> = Vec::with_capacity(approx_size);

        for i in 0..chunk_count {
            let chunk_bytes = self.api.download_chunk(file_id, i).await?;
            let decrypted = decrypt_downloaded_chunk(&file_key, &chunk_bytes)
                .map_err(|e| anyhow::anyhow!("decrypt chunk {i}: {e}"))?;
            plaintext.extend_from_slice(&decrypted);
        }

        // Make the destination directory if needed (OS extensions that
        // hand us a per-file cache path will already have created it,
        // but a generic mirror layout might not).
        if let Some(parent) = dest_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| anyhow::anyhow!("create dest dir {}: {e}", parent.display()))?;
        }
        std::fs::write(dest_path, &plaintext).map_err(|e| anyhow::anyhow!("write {}: {e}", dest_path.display()))?;

        Ok(())
    }

    /// Apply a "Keep Mine" resolution: the local copy stays as-is,
    /// status flips to `Local`, and `remote_updated_at` is bumped to
    /// `max(now, prev + 1)` so the next [`sync_tick`] doesn't see the
    /// server's still-divergent `updated_at` as a fresh conflict.
    ///
    /// **Open gap**: this does *not* upload local to the server. The
    /// dampening above only suppresses re-detection until a sibling
    /// device next touches the file — at which point the conflict
    /// re-fires (correctly, because the user's "Keep Mine" wasn't
    /// actually published). The full fix is a real upload path on
    /// the bridge; tracked alongside the chunked-upload work that
    /// `repos/cli/src/commands/push.rs` already implements.
    ///
    /// `Err` if the entry doesn't exist or the row update fails.
    pub async fn resolve_keep_mine(&self, file_id: &str) -> anyhow::Result<()> {
        let mut entry = self
            .db
            .get_file(file_id)?
            .ok_or_else(|| anyhow::anyhow!("no state.db row for {file_id}"))?;
        let now_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        // Use whichever is later: clock-now, or one tick past whatever
        // we already have. Guards against a clock that's running
        // behind the server.
        let new_anchor = now_secs.max(entry.remote_updated_at.saturating_add(1));
        entry.status = FileStatus::Local;
        entry.remote_updated_at = new_anchor;
        // modified_at goes back to the local mtime semantic since the
        // row is no longer in Conflict — best-effort: read the
        // filesystem mtime if we can, otherwise reuse the new anchor.
        entry.modified_at = new_anchor;
        self.db.upsert_file(&entry)?;
        tracing::info!(file_id = %file_id, "conflict resolved: keep mine (no upload yet — see TODO)");
        Ok(())
    }

    /// Apply a "Keep Theirs" resolution: download the remote version
    /// and overwrite the local file at `<sync_root>/<entry.path>`.
    /// Status ends in `Local`; `remote_updated_at` is anchored to
    /// "now" so the next tick treats the file as freshly synced.
    ///
    /// Reuses [`Self::hydrate_file`], which already handles status
    /// transitions and Error-on-failure rollback. The extra
    /// `remote_updated_at` bump after a successful hydrate prevents
    /// the next tick from re-flagging a conflict if the user's old
    /// `content_hash` is still on the row (it isn't anymore — the row
    /// was already in Conflict — but we belt-and-brace anchor anyway).
    pub async fn resolve_keep_theirs(&self, file_id: &str, sync_root: &Path) -> anyhow::Result<()> {
        let mut entry = self
            .db
            .get_file(file_id)?
            .ok_or_else(|| anyhow::anyhow!("no state.db row for {file_id}"))?;
        let dest = sync_root.join(&entry.path);
        // hydrate_file flips Conflict → Downloading → Local on success
        // (or Error on failure). We don't care about the intermediate
        // state for this path.
        self.hydrate_file(file_id, &dest).await?;
        let now_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        entry.status = FileStatus::Local;
        entry.remote_updated_at = now_secs;
        entry.modified_at = now_secs;
        self.db.upsert_file(&entry)?;
        tracing::info!(file_id = %file_id, dest = %dest.display(), "conflict resolved: keep theirs");
        Ok(())
    }

    /// Apply a "Keep Both" resolution: rename the local copy to a
    /// device-suffixed conflict filename, hydrate the remote into the
    /// original path, flip status back to `Local`. Called by Task 13's
    /// auto-resolution timer in [`crate::runner`] and (eventually) by
    /// the user-driven `resolve_conflict` IPC when the user picks
    /// "Keep Both" from the conflict window.
    ///
    /// Filesystem ops are done in this order so a partial failure
    /// leaves recoverable state:
    ///
    ///   1. Rename local → conflict copy (cheap, instantaneous)
    ///   2. Hydrate remote into the original path (network — may
    ///      fail; if it does, the user still has both their original
    ///      *and* the conflict copy on disk, so no data is lost; we
    ///      flip status to `Error` so the next tick retries).
    ///
    /// `sync_root` is supplied by the caller because the bridge
    /// itself doesn't know it (the runner owns that).
    pub async fn auto_resolve_keep_both(&self, sync_root: &Path, entry: &FileEntry) -> anyhow::Result<String> {
        let original = sync_root.join(&entry.path);

        // If the local file no longer exists on disk (user deleted it
        // outside the daemon), Keep Both collapses to Keep Remote.
        if !original.exists() {
            self.hydrate_file(&entry.file_id, &original).await?;
            self.db.set_status(&entry.file_id, FileStatus::Local)?;
            return Ok(entry.path.clone());
        }

        let host = hostname::get()
            .map(|h| h.to_string_lossy().to_string())
            .unwrap_or_else(|_| "device".into());
        let date = chrono::Utc::now().format("%Y-%m-%d");
        let stem = original.file_stem().and_then(|s| s.to_str()).unwrap_or("file");
        let ext = original
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| format!(".{e}"))
            .unwrap_or_default();
        let conflict_name = format!("{stem} (conflict - {host} - {date}){ext}");
        let conflict_path = original.with_file_name(&conflict_name);

        std::fs::rename(&original, &conflict_path)
            .map_err(|e| anyhow::anyhow!("rename {} -> {}: {e}", original.display(), conflict_path.display()))?;

        // Hydrate remote into the now-vacant original path. On failure
        // we mark Error rather than Conflict — the conflict copy is
        // safe on disk, the remote download just needs a retry.
        if let Err(e) = self.hydrate_file(&entry.file_id, &original).await {
            self.db.set_status(&entry.file_id, FileStatus::Error)?;
            return Err(e);
        }
        self.db.set_status(&entry.file_id, FileStatus::Local)?;
        Ok(conflict_name)
    }
}

pub fn is_ignored_finder_name(name: &str) -> bool {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return true;
    }
    let lower = trimmed.to_ascii_lowercase();
    matches!(
        trimmed,
        ".DS_Store" | ".DocumentRevisions-V100" | ".Spotlight-V100" | ".TemporaryItems" | ".Trashes" | "TemporaryItems"
    ) || trimmed.starts_with("._")
        || trimmed.starts_with("~$")
        || trimmed.ends_with('~')
        || lower.ends_with(".tmp")
        || lower.ends_with(".temp")
        || lower.ends_with(".swp")
        || lower.ends_with(".swo")
        || lower.ends_with(".part")
        || lower.ends_with(".crdownload")
}

pub fn version_conflict_feed_from_db(db: &StateDb) -> anyhow::Result<Vec<VersionConflictEntry>> {
    let mut entries = Vec::new();

    for file in db.list_by_status(FileStatus::Conflict)? {
        entries.push(VersionConflictEntry {
            id: format!("conflict:{}", file.file_id),
            file_id: file.file_id.clone(),
            file_name: display_name_for_path(&file.path),
            kind: "conflict".to_string(),
            status: "needs review".to_string(),
            updated_at: Some(file.modified_at),
            detail: "Local and remote both changed from the last synced base.".to_string(),
            action: "open_conflict".to_string(),
            op_id: None,
            version_id: None,
            base_version: None,
            last_error: None,
        });
    }

    for op in db.list_review_operations()? {
        entries.push(review_entry_for_operation(&op, db)?);
    }

    entries.sort_by(|a, b| {
        b.updated_at
            .unwrap_or_default()
            .cmp(&a.updated_at.unwrap_or_default())
            .then_with(|| a.id.cmp(&b.id))
    });
    Ok(entries)
}

fn encrypted_metadata_for_name(
    master_key_bytes: &[u8; 32],
    file_id: &str,
    filename: &str,
    mime_type: Option<&str>,
) -> anyhow::Result<String> {
    let master_key = beebeeb_core::kdf::MasterKey::from_bytes(*master_key_bytes);
    beebeeb_core::encrypt::encrypt_name(&master_key, file_id, filename, mime_type)
        .map_err(|e| anyhow::anyhow!("encrypt Finder metadata: {e}"))
}

fn upload_init_request_for_operation(
    file_id: &str,
    name_encrypted: &str,
    content_type: Option<String>,
    parent_id: Option<String>,
    plaintext_size: u64,
    base_version: Option<i64>,
    is_new_file: bool,
) -> DesktopUploadInitRequest {
    let plan = beebeeb_types::plan_chunks(plaintext_size, beebeeb_types::ChunkProfile::Desktop);
    DesktopUploadInitRequest {
        file_id: if is_new_file { None } else { Some(file_id.to_string()) },
        file_name: name_encrypted.to_string(),
        file_size_bytes: plaintext_size,
        mime_type: None,
        parent_id,
        profile: "desktop".to_string(),
        is_media: beebeeb_core::media::is_media(content_type.as_deref()),
        chunk_size_bytes: Some(plan.chunk_size_bytes),
        chunk_count: Some(plan.chunk_count),
        base_version_number: base_version.and_then(|version| i32::try_from(version).ok()),
    }
}

fn is_create_file_operation(metadata: &serde_json::Value) -> bool {
    metadata["operation"].as_str() == Some("create_file")
}

fn metadata_display_name(metadata: &serde_json::Value, op: &PendingOperation) -> Option<String> {
    metadata["display_name"]
        .as_str()
        .map(str::to_string)
        .or_else(|| op.target_path.as_deref().map(display_name_for_path))
}

fn decrypt_downloaded_chunk(
    file_key: &beebeeb_core::kdf::FileKey,
    chunk_bytes: &[u8],
) -> Result<Vec<u8>, beebeeb_core::CoreError> {
    match beebeeb_core::encrypt::decrypt_chunk_raw(file_key, chunk_bytes) {
        Ok(bytes) => Ok(bytes),
        Err(raw_error) => {
            let blob: EncryptedBlob = serde_json::from_slice(chunk_bytes).map_err(|_| raw_error)?;
            beebeeb_core::encrypt::decrypt_chunk(file_key, &blob)
        }
    }
}

fn stage_finder_payload(contents_path: &str) -> anyhow::Result<String> {
    stage_finder_payload_with_root(contents_path, default_finder_staging_root())
}

fn stage_finder_payload_with_root(contents_path: &str, staging_root: PathBuf) -> anyhow::Result<String> {
    let source = Path::new(contents_path);
    if !source.is_file() {
        return Err(anyhow::anyhow!(
            "Finder content path is not a file: {}",
            source.display()
        ));
    }
    std::fs::create_dir_all(&staging_root)
        .map_err(|e| anyhow::anyhow!("create Finder staging dir {}: {e}", staging_root.display()))?;
    let op_id = uuid::Uuid::new_v4();
    let safe_name = source
        .file_name()
        .and_then(|s| s.to_str())
        .map(sanitize_staging_name)
        .unwrap_or_else(|| "payload".to_string());
    let dest = staging_root.join(format!("{op_id}-{safe_name}"));
    std::fs::copy(source, &dest)
        .map_err(|e| anyhow::anyhow!("copy Finder payload {} -> {}: {e}", source.display(), dest.display()))?;
    Ok(dest.to_string_lossy().into_owned())
}

fn default_finder_staging_root() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("beebeeb")
        .join("finder-writes")
}

fn sanitize_staging_name(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            '/' | '\\' | ':' => '_',
            _ if c.is_control() => '_',
            _ => c,
        })
        .collect()
}

fn parse_base_version_number(version_identifier: Option<&str>) -> Option<i64> {
    version_identifier.and_then(|value| {
        value
            .split(':')
            .next()
            .and_then(|part| part.parse::<i64>().ok())
            .filter(|version| *version > 0)
    })
}

fn display_name_for_path(path: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(path)
        .to_string()
}

fn review_entry_for_operation(op: &PendingOperation, db: &StateDb) -> anyhow::Result<VersionConflictEntry> {
    let file_id = op.file_id.clone().unwrap_or_else(|| op.op_id.clone());
    let file_name = op
        .target_path
        .as_deref()
        .map(display_name_for_path)
        .or_else(|| {
            op.file_id
                .as_deref()
                .and_then(|id| db.get_file(id).ok().flatten())
                .map(|entry| display_name_for_path(&entry.path))
        })
        .unwrap_or_else(|| file_id.clone());
    let (kind, status, detail, action) = classify_review_operation(op);

    Ok(VersionConflictEntry {
        id: format!("op:{}", op.op_id),
        file_id,
        file_name,
        kind: kind.to_string(),
        status: status.to_string(),
        updated_at: Some(op.updated_at),
        detail,
        action: action.to_string(),
        op_id: Some(op.op_id.clone()),
        version_id: operation_version_id(op),
        base_version: op.base_version,
        last_error: op.last_error.clone(),
    })
}

fn classify_review_operation(op: &PendingOperation) -> (&'static str, &'static str, String, &'static str) {
    let error = op.last_error.as_deref().unwrap_or("").to_ascii_lowercase();
    let metadata = op.metadata_json.as_deref().unwrap_or("").to_ascii_lowercase();
    let stale_base = error.contains("stale")
        || error.contains("base version")
        || metadata.contains("base_version_identifier")
        || op.base_version.is_some();

    if matches!(op.kind, OperationKind::RestoreVersion) {
        return (
            "restore",
            "restore review",
            "Restore is queued or failed; the server restore endpoint creates a new current version when it succeeds."
                .to_string(),
            "restore_review",
        );
    }
    if error.contains("quota") || error.contains("insufficient storage") {
        return (
            "quota_failure",
            "quota blocked",
            op.last_error
                .clone()
                .unwrap_or_else(|| "Upload is blocked by account storage quota.".to_string()),
            "review_upload",
        );
    }
    if error.contains("permission")
        || error.contains("forbidden")
        || error.contains("read-only")
        || error.contains("403")
    {
        return (
            "permission_failure",
            "permission blocked",
            op.last_error
                .clone()
                .unwrap_or_else(|| "Write is blocked by folder permissions.".to_string()),
            "review_upload",
        );
    }
    if stale_base && matches!(op.kind, OperationKind::UploadVersion) {
        return (
            "stale_base",
            "stale base kept local",
            "Local bytes are preserved in the durable queue and need version review before retry.".to_string(),
            "review_upload",
        );
    }

    match op.kind {
        OperationKind::UploadVersion | OperationKind::UploadFile => (
            "failed_upload",
            "upload review",
            op.last_error
                .clone()
                .unwrap_or_else(|| "Upload is queued for the encrypted transfer worker.".to_string()),
            "review_upload",
        ),
        OperationKind::RenameFile | OperationKind::MoveFile => (
            "metadata",
            "metadata review",
            op.last_error
                .clone()
                .unwrap_or_else(|| "Rename or move is queued for metadata sync.".to_string()),
            "review_upload",
        ),
        OperationKind::TrashFile => (
            "delete",
            "delete review",
            op.last_error
                .clone()
                .unwrap_or_else(|| "Delete is queued as a trash/soft-delete operation.".to_string()),
            "review_upload",
        ),
        _ => (
            "failed_upload",
            "queued review",
            op.last_error
                .clone()
                .unwrap_or_else(|| "Operation is queued for a worker that is not attached yet.".to_string()),
            "review_upload",
        ),
    }
}

fn operation_version_id(op: &PendingOperation) -> Option<String> {
    op.base_object_version_id.clone().or_else(|| {
        op.metadata_json
            .as_deref()
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
            .and_then(|value| value.get("version_id").and_then(|v| v.as_str()).map(str::to_string))
    })
}

fn operation_metadata(op: &PendingOperation) -> anyhow::Result<serde_json::Value> {
    Ok(op
        .metadata_json
        .as_deref()
        .map(serde_json::from_str::<serde_json::Value>)
        .transpose()?
        .unwrap_or_else(|| serde_json::json!({})))
}

pub fn classify_operation_error(error: &str) -> OperationFailureClass {
    let lower = error.to_ascii_lowercase();
    if lower.contains("401") || lower.contains("unauthorized") || lower.contains("invalid token") {
        OperationFailureClass::Auth
    } else if lower.contains("quota") || lower.contains("insufficient storage") || lower.contains("storage limit") {
        OperationFailureClass::Quota
    } else if lower.contains("403")
        || lower.contains("forbidden")
        || lower.contains("permission")
        || lower.contains("read-only")
    {
        OperationFailureClass::Permission
    } else if lower.contains("vault locked") || lower.contains("locked") || lower.contains("unlock") {
        OperationFailureClass::Locked
    } else {
        OperationFailureClass::Retryable
    }
}

pub fn retry_delay_seconds(attempts: i64) -> i64 {
    let exponent = attempts.clamp(1, 6) as u32;
    30_i64.saturating_mul(2_i64.saturating_pow(exponent - 1))
}

fn shared_roots_from_invite_response(body: &serde_json::Value) -> Vec<SharedRootMapping> {
    body.get("invites")
        .and_then(|value| value.as_array())
        .into_iter()
        .flatten()
        .filter_map(shared_root_from_invite)
        .collect()
}

fn shared_root_from_invite(invite: &serde_json::Value) -> Option<SharedRootMapping> {
    if invite.get("status").and_then(|value| value.as_str()) != Some("approved") {
        return None;
    }

    let invite_id = string_field(invite, &["id", "invite_id"])?;
    let file_id = string_field(invite, &["file_id"])?;
    let display_name = string_field(
        invite,
        &["display_name", "decrypted_name", "filename", "file_name", "name"],
    )
    .unwrap_or_else(|| format!("Shared item {}", file_id.chars().take(8).collect::<String>()));
    let is_folder = invite
        .get("is_folder_share")
        .and_then(|value| value.as_bool())
        .or_else(|| invite.get("is_folder").and_then(|value| value.as_bool()))
        .unwrap_or(false);
    let content_type = string_field(invite, &["mime_type", "content_type"]);
    let size_bytes = invite.get("size_bytes").and_then(|value| value.as_i64()).unwrap_or(0);
    let mut permission_bits = PERMISSION_READ;
    if invite
        .get("can_reshare")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
    {
        permission_bits |= PERMISSION_SHARE;
    }
    if shared_invite_allows_write(invite) {
        permission_bits |= PERMISSION_WRITE;
    }

    Some(SharedRootMapping {
        invite_id,
        file_id,
        display_name,
        is_folder,
        size_bytes,
        content_type,
        permission_bits,
        approved_at: invite
            .get("approved_at")
            .and_then(|value| value.as_str())
            .and_then(parse_rfc3339_secs),
    })
}

fn shared_invite_allows_write(invite: &serde_json::Value) -> bool {
    if ["can_write", "can_edit", "editable"]
        .iter()
        .any(|key| invite.get(*key).and_then(|value| value.as_bool()).unwrap_or(false))
    {
        return true;
    }
    ["permission", "permissions", "role", "access"]
        .iter()
        .filter_map(|key| invite.get(*key).and_then(|value| value.as_str()))
        .any(|value| {
            matches!(
                value.to_ascii_lowercase().as_str(),
                "write" | "edit" | "editable" | "editor" | "admin" | "owner"
            )
        })
}

fn string_field(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .filter_map(|key| value.get(*key).and_then(|field| field.as_str()))
        .map(str::trim)
        .find(|field| !field.is_empty())
        .map(str::to_string)
}

fn parse_rfc3339_secs(value: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|dt| dt.timestamp())
}

fn apply_shared_context(metadata: &mut serde_json::Value, contract: Option<&FileContractState>) {
    let Some(contract) = contract else {
        return;
    };
    metadata["shared_root_id"] = serde_json::json!(contract.shared_root_id);
    metadata["share_id"] = serde_json::json!(contract.share_id);
    metadata["permission_bits"] = serde_json::json!(contract.permission_bits);
    metadata["uploaded_by"] = serde_json::json!("authenticated_desktop_user");
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
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
/// Fallback order (keeps legacy/test rows that DO carry a plaintext path
/// working, and degrades safely if a blob is missing/garbled):
///   1. decrypt `name_encrypted` (the canonical zero-knowledge path)
///   2. a plaintext `path`/`display_path`/`name` field if the server ever
///      surfaces one (older rows / test fixtures)
///   3. empty string — caller skips placeholder seeding for the row
fn resolve_relative_path(f: &serde_json::Value, file_id: &str, master_key: &[u8; 32]) -> String {
    if let Some(name_enc) = f["name_encrypted"].as_str() {
        let mk_bytes: [u8; 32] = *master_key;
        let mk = beebeeb_core::kdf::MasterKey::from_bytes(mk_bytes);
        if let Ok(name) = beebeeb_core::encrypt::decrypt_name(&mk, file_id, name_enc) {
            let name = name.trim();
            if !name.is_empty() {
                return name.to_string();
            }
        }
        // Decryption failed (wrong key, garbled blob, legacy envelope) —
        // fall through to any plaintext field rather than abort the sweep.
    }
    f["path"]
        .as_str()
        .or_else(|| f["display_path"].as_str())
        .or_else(|| f["name"].as_str())
        .unwrap_or("")
        .to_string()
}

fn apply_metadata_file_row(
    db: &StateDb,
    f: &serde_json::Value,
    namespace: Namespace,
    shared_root_id: Option<String>,
    share_id: Option<String>,
    permission_bits: i64,
    now: i64,
    master_key: &[u8; 32],
) -> anyhow::Result<Option<FileEntry>> {
    let file_id = f["id"].as_str().unwrap_or_default();
    if file_id.is_empty() {
        return Ok(None);
    }

    let existing = db.get_file(file_id)?;
    let size = f["size_bytes"].as_i64().or_else(|| f["size"].as_i64()).unwrap_or(0);
    let remote_updated = f["updated_at"].as_i64().unwrap_or(0);
    let path = resolve_relative_path(f, file_id, master_key);
    let status = existing
        .as_ref()
        .map(|entry| entry.status.clone())
        .unwrap_or(FileStatus::CloudOnly);

    let entry = FileEntry {
        file_id: file_id.to_string(),
        path,
        status,
        size_bytes: size,
        modified_at: remote_updated,
        content_hash: existing.as_ref().and_then(|entry| entry.content_hash.clone()),
        remote_updated_at: remote_updated,
    };
    db.upsert_file(&entry)?;

    let item_kind = if f["is_folder"].as_bool().unwrap_or(false)
        || f["kind"].as_str() == Some("folder")
        || f["type"].as_str() == Some("folder")
    {
        ItemKind::Folder
    } else {
        ItemKind::File
    };
    let mut contract = db
        .get_file_contract_state(file_id)?
        .unwrap_or_else(|| FileContractState {
            file_id: file_id.to_string(),
            namespace: namespace.clone(),
            parent_id: None,
            shared_root_id: shared_root_id.clone(),
            share_id: share_id.clone(),
            permission_bits,
            item_kind: item_kind.clone(),
            content_type: None,
            current_version: 0,
            current_object_version_id: None,
            local_base_version: 0,
            local_hash: None,
            cache_path: None,
            cache_bytes: 0,
            pin_state: crate::state_db::PinState::Inherit,
            inherited_pin_state: crate::state_db::PinState::Unpinned,
            last_sync_at: now,
        });
    contract.namespace = namespace;
    contract.parent_id = f["parent_id"].as_str().map(str::to_string);
    contract.shared_root_id = shared_root_id;
    contract.share_id = share_id;
    contract.permission_bits = permission_bits;
    contract.item_kind = item_kind;
    contract.content_type = f["content_type"]
        .as_str()
        .or_else(|| f["mime_type"].as_str())
        .map(str::to_string);
    contract.current_version = f["current_version"]
        .as_i64()
        .or_else(|| f["version"].as_i64())
        .or_else(|| f["version_number"].as_i64())
        .unwrap_or(contract.current_version);
    contract.current_object_version_id = f["current_object_version_id"]
        .as_str()
        .or_else(|| f["object_version_id"].as_str())
        .map(str::to_string)
        .or(contract.current_object_version_id);
    if entry.status == FileStatus::Local && contract.local_base_version == 0 {
        contract.local_base_version = contract.current_version;
    }
    contract.last_sync_at = now;
    db.set_file_contract_state(&contract)?;
    Ok(Some(entry))
}

// ── Periodic sync tick ────────────────────────────────────────────────────────

/// One file the latest tick noticed has diverged on both sides.
/// Returned from [`sync_tick`] so [`crate::runner`] can fan it out
/// to UI: open a conflict window + fire a notification.
#[derive(Debug, Clone)]
pub struct ConflictDetected {
    pub file_id: String,
    pub file_name: String,
    pub is_text: bool,
}

/// Pull the user's file list from the API, refresh the state DB, and
/// flag any newly-divergent files. Called from [`crate::runner`]'s
/// tick loop.
///
/// Three-way decision per remote file:
///
/// 1. **New to us** — no row exists. Insert as `cloud_only`. The OS
///    extension surfaces it as a placeholder; the user gets it on
///    demand.
///
/// 2. **Known and locally `Local`, remote moved** — the row exists,
///    its status is `Local`, and the server's `updated_at` is past
///    `remote_updated_at`. We use the
///    [`crate::conflict::is_conflict`] predicate to decide whether
///    that's a one-sided update we can quietly accept (no local edit
///    since base) or a divergent edit that needs the user. The
///    "hashes" we feed are synthetic right now: the server doesn't
///    expose a content hash on file metadata yet, so we compose
///    `<size>-<updated_at>` for remote and reuse the local
///    `content_hash` for local. False negatives (different bytes,
///    same size + mtime) are theoretically possible but rare; the
///    engine bridge will catch them on the next chunk diff once that
///    code lands.
///
/// 3. **Known and not in `Local`** — pending download/upload, etc.
///    Leave alone; the upload/download path owns those transitions.
pub async fn sync_tick(bridge: &EngineBridge) -> anyhow::Result<Vec<ConflictDetected>> {
    let files = bridge.api().list_files(None).await?;
    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let mut conflicts: Vec<ConflictDetected> = Vec::new();

    for f in &files {
        let file_id = f["id"].as_str().unwrap_or_default();
        if file_id.is_empty() {
            continue;
        }
        let size = f["size_bytes"].as_i64().unwrap_or(0);
        let remote_updated = f["updated_at"].as_i64().unwrap_or(0);
        match bridge.db().get_file(file_id)? {
            None => {
                // (1) New file — insert as cloud_only. base = remote
                // (next-tick conflicts compare against this).
                apply_metadata_file_row(
                    bridge.db(),
                    f,
                    Namespace::MyFiles,
                    None,
                    None,
                    PERMISSION_READ | PERMISSION_WRITE | PERMISSION_OWNER,
                    now_secs,
                    bridge.api().master_key(),
                )?;
            }
            Some(entry) if entry.status == FileStatus::Local => {
                // (2) Local copy + remote moved? Nothing to check if
                // the timestamps haven't drifted past base.
                if remote_updated <= entry.remote_updated_at {
                    continue;
                }

                // Synthesise version triplet. We don't have the
                // server's content hash on file metadata, so we hash
                // by `size-mtime` as an opaque-but-stable surrogate.
                // Treating the surrogate's equality as "same version"
                // is conservative — a file edited to the same
                // size+mtime won't be flagged, but those collisions
                // are vanishingly rare in practice.
                let local = VersionInfo {
                    hash: entry.content_hash.clone().unwrap_or_default(),
                    modified_at: entry.modified_at as u64,
                };
                let remote = VersionInfo {
                    hash: format!("{size}-{remote_updated}"),
                    modified_at: remote_updated as u64,
                };
                let base = VersionInfo {
                    hash: entry.content_hash.clone().unwrap_or_default(),
                    modified_at: entry.remote_updated_at as u64,
                };
                // Local matches base → only remote changed. Quietly
                // re-anchor and let a future hydrate replace bytes
                // when the user opens it.
                if !is_conflict(&local, &remote, &base) {
                    let mut updated = entry.clone();
                    updated.remote_updated_at = remote_updated;
                    updated.size_bytes = size;
                    updated.modified_at = remote_updated;
                    bridge.db().upsert_file(&updated)?;
                    apply_metadata_file_row(
                        bridge.db(),
                        f,
                        Namespace::MyFiles,
                        None,
                        None,
                        PERMISSION_READ | PERMISSION_WRITE | PERMISSION_OWNER,
                        now_secs,
                        bridge.api().master_key(),
                    )?;
                    continue;
                }

                // True conflict. Flip status, anchor `modified_at` to
                // "now" so Task 13's auto-resolution clock starts
                // from detection rather than from whatever the row
                // happened to carry before.
                let mut updated = entry.clone();
                updated.status = FileStatus::Conflict;
                updated.modified_at = now_secs;
                bridge.db().upsert_file(&updated)?;

                let file_name = std::path::Path::new(&entry.path)
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or(&entry.path)
                    .to_string();
                let is_text = is_text_file(&file_name);
                tracing::warn!(
                    file_id = %entry.file_id,
                    name = %file_name,
                    "conflict detected on tick — both sides moved past base"
                );
                conflicts.push(ConflictDetected {
                    file_id: entry.file_id.clone(),
                    file_name,
                    is_text,
                });
            }
            Some(entry) => {
                // (3) Pending download/upload/conflict/error — let
                // the dedicated path own its transitions.
                if matches!(entry.status, FileStatus::CloudOnly | FileStatus::Downloading) {
                    apply_metadata_file_row(
                        bridge.db(),
                        f,
                        Namespace::MyFiles,
                        None,
                        None,
                        PERMISSION_READ | PERMISSION_WRITE | PERMISSION_OWNER,
                        now_secs,
                        bridge.api().master_key(),
                    )?;
                }
                continue;
            }
        }
    }
    Ok(conflicts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;
    use std::net::TcpListener;
    use std::sync::Mutex;
    use std::thread;

    #[test]
    fn test_file_state_machine_transitions() {
        // cloud_only -> downloading -> local
        let mut sm = FileSM::new(FileStatus::CloudOnly);
        sm.transition(FileEvent::DownloadStart).unwrap();
        assert_eq!(sm.state(), FileStatus::Downloading);
        sm.transition(FileEvent::DownloadComplete).unwrap();
        assert_eq!(sm.state(), FileStatus::Local);
    }

    #[test]
    fn test_invalid_transition_rejected() {
        let mut sm = FileSM::new(FileStatus::CloudOnly);
        assert!(sm.transition(FileEvent::UploadStart).is_err());
    }

    #[test]
    fn test_temp_file_filter_covers_editor_and_system_names() {
        for name in [
            ".DS_Store",
            "._report.pdf",
            "~$budget.xlsx",
            "draft.txt~",
            ".report.swp",
            "upload.tmp",
            "video.crdownload",
        ] {
            assert!(is_ignored_finder_name(name), "{name} should be ignored");
        }
        assert!(!is_ignored_finder_name("report.pdf"));
        assert!(!is_ignored_finder_name(".env.sample"));
    }

    #[test]
    fn test_base_version_parser_reads_current_version_prefix() {
        assert_eq!(parse_base_version_number(Some("7:1700000000:1024")), Some(7));
        assert_eq!(parse_base_version_number(Some("0:1700000000:1024")), None);
        assert_eq!(parse_base_version_number(Some("local:1700000000")), None);
        assert_eq!(parse_base_version_number(None), None);
    }

    #[test]
    fn test_upload_init_request_omits_new_file_id_and_preserves_existing_base() {
        let req = upload_init_request_for_operation(
            "file-new",
            "{\"cipher_suite\":\"V1Aes256Gcm\"}",
            Some("image/png".into()),
            Some("folder-1".into()),
            11,
            None,
            true,
        );
        assert_eq!(req.file_id, None);
        assert_eq!(req.file_size_bytes, 11);
        assert_eq!(req.profile, "desktop");
        assert!(req.is_media);
        assert_eq!(req.chunk_count, Some(1));

        let replace = upload_init_request_for_operation(
            "file-existing",
            "{\"cipher_suite\":\"V1Aes256Gcm\"}",
            Some("text/plain".into()),
            None,
            42,
            Some(7),
            false,
        );
        assert_eq!(replace.file_id.as_deref(), Some("file-existing"));
        assert_eq!(replace.base_version_number, Some(7));
        assert!(!replace.is_media);
    }

    #[test]
    fn test_download_chunk_decrypts_raw_and_json_formats() {
        let file_id = uuid::Uuid::new_v4().to_string();
        let master_key = beebeeb_core::kdf::MasterKey::from_bytes([9u8; 32]);
        let file_key = beebeeb_core::kdf::derive_file_key(&master_key, file_id.as_bytes());

        let raw = beebeeb_core::encrypt::encrypt_chunk_raw(&file_key, b"raw bytes").unwrap();
        assert_eq!(decrypt_downloaded_chunk(&file_key, &raw).unwrap(), b"raw bytes");

        let blob = beebeeb_core::encrypt::encrypt_chunk(&file_key, b"json bytes").unwrap();
        let json = serde_json::to_vec(&blob).unwrap();
        assert_eq!(decrypt_downloaded_chunk(&file_key, &json).unwrap(), b"json bytes");
    }

    #[test]
    fn test_staging_payload_copies_to_durable_location() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("report.txt");
        std::fs::write(&source, b"finder data").unwrap();
        let staged = stage_finder_payload_with_root(source.to_str().unwrap(), dir.path().join("staging")).unwrap();
        assert_eq!(std::fs::read(staged).unwrap(), b"finder data");
    }

    fn test_bridge(db_path: &Path) -> EngineBridge {
        let db = Arc::new(StateDb::open(db_path).unwrap());
        let api = Arc::new(ApiClient::new(
            "https://api.beebeeb.io".into(),
            "token".into(),
            [7u8; 32],
        ));
        EngineBridge::new(db, api)
    }

    fn test_bridge_with_api(db_path: &Path, base_url: String, master_key: [u8; 32]) -> EngineBridge {
        let db = Arc::new(StateDb::open(db_path).unwrap());
        let api = Arc::new(ApiClient::new(base_url, "token".into(), master_key));
        EngineBridge::new(db, api)
    }

    #[derive(Debug, Clone)]
    struct RecordedRequest {
        method: String,
        path: String,
        body: Vec<u8>,
    }

    struct UploadMockServer {
        base_url: String,
        requests: Arc<Mutex<Vec<RecordedRequest>>>,
        handle: thread::JoinHandle<()>,
    }

    impl UploadMockServer {
        fn start(fail_chunk: bool) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let base_url = format!("http://{}", listener.local_addr().unwrap());
            let requests = Arc::new(Mutex::new(Vec::new()));
            let server_requests = Arc::clone(&requests);
            let handle = thread::spawn(move || {
                let expected_requests = if fail_chunk { 3 } else { 4 };
                for _ in 0..expected_requests {
                    let (mut stream, _) = listener.accept().unwrap();
                    let request = read_http_request(&mut stream);
                    let response = upload_mock_response(&request, fail_chunk);
                    server_requests.lock().unwrap().push(request);
                    stream.write_all(response.as_bytes()).unwrap();
                }
            });
            Self {
                base_url,
                requests,
                handle,
            }
        }

        fn finish(self) -> Vec<RecordedRequest> {
            self.handle.join().unwrap();
            Arc::try_unwrap(self.requests).unwrap().into_inner().unwrap()
        }
    }

    fn read_http_request(stream: &mut std::net::TcpStream) -> RecordedRequest {
        let mut buffer = Vec::new();
        let mut temp = [0u8; 4096];
        let header_end;
        loop {
            let read = std::io::Read::read(stream, &mut temp).unwrap();
            assert!(read > 0, "mock server connection closed before headers");
            buffer.extend_from_slice(&temp[..read]);
            if let Some(pos) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
                header_end = pos + 4;
                break;
            }
        }

        let headers = String::from_utf8_lossy(&buffer[..header_end]);
        let mut lines = headers.lines();
        let request_line = lines.next().unwrap();
        let mut request_parts = request_line.split_whitespace();
        let method = request_parts.next().unwrap().to_string();
        let path = request_parts.next().unwrap().to_string();
        let content_length = headers
            .lines()
            .find_map(|line| {
                line.strip_prefix("Content-Length:")
                    .or_else(|| line.strip_prefix("content-length:"))
                    .and_then(|value| value.trim().parse::<usize>().ok())
            })
            .unwrap_or(0);

        let mut body = buffer[header_end..].to_vec();
        while body.len() < content_length {
            let read = std::io::Read::read(stream, &mut temp).unwrap();
            assert!(read > 0, "mock server connection closed before body");
            body.extend_from_slice(&temp[..read]);
        }
        body.truncate(content_length);

        RecordedRequest { method, path, body }
    }

    fn http_json(status: &str, body: serde_json::Value) -> String {
        let body = body.to_string();
        format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    fn upload_mock_response(request: &RecordedRequest, fail_chunk: bool) -> String {
        match (request.method.as_str(), request.path.as_str()) {
            ("POST", "/api/v1/uploads/init") => http_json(
                "200 OK",
                serde_json::json!({
                    "file_id": "server-file-1",
                    "tenant_id": "tenant-1",
                    "object_version_id": "object-init-1",
                    "upload_session_id": "upload-session-1",
                    "chunk_size_bytes": 4 * 1024 * 1024,
                    "chunk_count": 1,
                    "storage_format_version": 1,
                    "storage_pool_id": "pool-1",
                    "region": "local"
                }),
            ),
            ("PATCH", "/api/v1/files/server-file-1") => {
                http_json("200 OK", serde_json::json!({ "id": "server-file-1" }))
            }
            ("PUT", "/api/v1/uploads/upload-session-1/chunks/0") if fail_chunk => {
                http_json("500 Internal Server Error", serde_json::json!({ "error": "boom" }))
            }
            ("PUT", "/api/v1/uploads/upload-session-1/chunks/0") => http_json(
                "200 OK",
                serde_json::json!({ "index": 0, "size": request.body.len() as i64, "skipped": false }),
            ),
            ("POST", "/api/v1/uploads/upload-session-1/complete") => http_json(
                "200 OK",
                serde_json::json!({
                    "file_id": "server-file-1",
                    "version_number": 1,
                    "current_object_version_id": "object-complete-1",
                    "size_bytes": 19,
                    "mime_type": "text/plain"
                }),
            ),
            _ => http_json(
                "404 Not Found",
                serde_json::json!({ "error": format!("unexpected {} {}", request.method, request.path) }),
            ),
        }
    }

    #[test]
    fn test_create_file_queues_upload_version_and_preserves_payload() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("report.txt");
        std::fs::write(&source, b"queued payload").unwrap();
        let bridge = test_bridge(&dir.path().join("state.db"));

        let outcome = bridge
            .queue_finder_create(FinderWriteTarget {
                file_id: None,
                parent_id: None,
                filename: "report.txt".into(),
                kind: FinderWriteItemKind::File,
                contents_path: Some(source.to_string_lossy().into_owned()),
                content_type: Some("text/plain".into()),
                base_version_identifier: None,
            })
            .unwrap();

        let FinderWriteOutcome::Queued {
            file_id: Some(file_id),
            kind,
            ..
        } = outcome
        else {
            panic!("expected queued file upload");
        };
        assert_eq!(kind, OperationKind::UploadVersion);

        let queued = bridge.db.list_due_operations(now_secs()).unwrap();
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].kind, OperationKind::UploadVersion);
        assert_eq!(queued[0].file_id.as_deref(), Some(file_id.as_str()));
        let payload_path = queued[0].payload_path.as_deref().unwrap();
        assert_eq!(std::fs::read(payload_path).unwrap(), b"queued payload");

        let metadata: serde_json::Value = serde_json::from_str(queued[0].metadata_json.as_deref().unwrap()).unwrap();
        assert_eq!(metadata["operation"], "create_file");
        assert!(metadata["name_encrypted"].as_str().unwrap().starts_with('{'));
        assert_eq!(
            bridge.db.get_file(&file_id).unwrap().unwrap().status,
            FileStatus::Uploading
        );
    }

    #[test]
    fn test_metadata_modify_does_not_queue_content_version() {
        let dir = tempfile::tempdir().unwrap();
        let bridge = test_bridge(&dir.path().join("state.db"));

        let outcome = bridge
            .queue_finder_modify(FinderWriteTarget {
                file_id: Some("file-1".into()),
                parent_id: Some("folder-1".into()),
                filename: "renamed.txt".into(),
                kind: FinderWriteItemKind::File,
                contents_path: None,
                content_type: Some("text/plain".into()),
                base_version_identifier: Some("4:1700000000:12".into()),
            })
            .unwrap();
        let FinderWriteOutcome::Queued { kind, .. } = outcome else {
            panic!("expected queued metadata op");
        };
        assert_eq!(kind, OperationKind::MoveFile);

        let queued = bridge.db.list_due_operations(now_secs()).unwrap();
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].kind, OperationKind::MoveFile);
        assert!(queued[0].payload_path.is_none());
        let metadata: serde_json::Value = serde_json::from_str(queued[0].metadata_json.as_deref().unwrap()).unwrap();
        assert_eq!(metadata["operation"], "metadata_update");
        assert!(metadata["name_encrypted"].as_str().unwrap().starts_with('{'));
    }

    #[test]
    fn test_delete_maps_to_trash_operation() {
        let dir = tempfile::tempdir().unwrap();
        let bridge = test_bridge(&dir.path().join("state.db"));

        let outcome = bridge
            .queue_finder_delete("file-1", Some("9:1700000000:100".into()))
            .unwrap();
        let FinderWriteOutcome::Queued { kind, .. } = outcome else {
            panic!("expected queued trash op");
        };
        assert_eq!(kind, OperationKind::TrashFile);

        let queued = bridge.db.list_due_operations(now_secs()).unwrap();
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].kind, OperationKind::TrashFile);
        assert_eq!(queued[0].base_version, Some(9));
        assert!(queued[0].payload_path.is_none());
    }

    #[test]
    fn test_recursive_pin_queues_hydration_for_cloud_only_descendants() {
        let dir = tempfile::tempdir().unwrap();
        let bridge = test_bridge(&dir.path().join("state.db"));
        seed_bridge_row(&bridge, "folder-a", "/Projects", None, FileStatus::CloudOnly, 0);
        seed_bridge_row(
            &bridge,
            "file-a",
            "/Projects/report.txt",
            Some("folder-a"),
            FileStatus::CloudOnly,
            128,
        );
        seed_bridge_row(
            &bridge,
            "file-local",
            "/Projects/local.txt",
            Some("folder-a"),
            FileStatus::Local,
            256,
        );

        let queued = bridge.set_recursive_pin("folder-a", true).unwrap();
        assert_eq!(queued.changed_item_ids.len(), 3);
        assert_eq!(queued.hydrate_operations, 1);

        let due = bridge.db.list_due_operations(now_secs()).unwrap();
        assert!(due.iter().any(|op| {
            op.kind == OperationKind::PinTree
                && op.file_id.as_deref() == Some("folder-a")
                && op.metadata_json.as_deref().unwrap_or("").contains("\"pinned\":true")
        }));
        assert!(
            due.iter()
                .any(|op| { op.kind == OperationKind::HydrateFile && op.file_id.as_deref() == Some("file-a") })
        );
        assert!(
            !due.iter()
                .any(|op| { op.kind == OperationKind::HydrateFile && op.file_id.as_deref() == Some("file-local") })
        );
    }

    #[test]
    fn test_smart_cache_cleanup_never_evicts_effectively_pinned_files() {
        let dir = tempfile::tempdir().unwrap();
        let bridge = test_bridge(&dir.path().join("state.db"));
        seed_bridge_row(&bridge, "pinned", "/Pinned.txt", None, FileStatus::Local, 700);
        seed_bridge_row(&bridge, "unpinned", "/Unpinned.txt", None, FileStatus::Local, 600);
        bridge.db.set_recursive_pin("pinned", true, 1).unwrap();
        bridge.db.mark_cached("pinned", "/cache/pinned", 700, 10).unwrap();
        bridge.db.mark_cached("unpinned", "/cache/unpinned", 600, 20).unwrap();

        let evicted = bridge
            .enforce_smart_cache(CachePolicy {
                max_unpinned_cache_bytes: 500,
                disk_pressure_min_free_bytes: 0,
            })
            .unwrap();

        assert_eq!(evicted.evicted_file_ids, vec!["unpinned".to_string()]);
        assert_eq!(bridge.db.get_file("pinned").unwrap().unwrap().status, FileStatus::Local);
    }

    #[test]
    fn test_shared_invite_mapping_filters_approved_and_permissions() {
        let body = serde_json::json!({
            "invites": [
                {
                    "id": "invite-read",
                    "file_id": "root-read",
                    "status": "approved",
                    "display_name": "Client dropbox",
                    "is_folder_share": true,
                    "can_reshare": true
                },
                {
                    "id": "invite-write",
                    "file_id": "root-write",
                    "status": "approved",
                    "decrypted_name": "Editorial",
                    "is_folder": true,
                    "permission": "write"
                },
                {
                    "id": "invite-pending",
                    "file_id": "root-pending",
                    "status": "claimed"
                }
            ]
        });

        let roots = shared_roots_from_invite_response(&body);
        assert_eq!(roots.len(), 2);
        assert_eq!(roots[0].display_name, "Client dropbox");
        assert_eq!(roots[0].permission_bits, PERMISSION_READ | PERMISSION_SHARE);
        assert_eq!(roots[1].permission_bits, PERMISSION_READ | PERMISSION_WRITE);
    }

    #[test]
    fn test_operation_error_classification_and_backoff() {
        assert_eq!(
            classify_operation_error("401 unauthorized Bearer token"),
            OperationFailureClass::Auth
        );
        assert_eq!(classify_operation_error("quota exceeded"), OperationFailureClass::Quota);
        assert_eq!(
            classify_operation_error("403 forbidden permission denied"),
            OperationFailureClass::Permission
        );
        assert_eq!(
            classify_operation_error("vault locked, unlock required"),
            OperationFailureClass::Locked
        );
        assert_eq!(
            classify_operation_error("connection reset by peer"),
            OperationFailureClass::Retryable
        );
        assert_eq!(retry_delay_seconds(1), 30);
        assert_eq!(retry_delay_seconds(2), 60);
        assert_eq!(retry_delay_seconds(10), 960);
    }

    #[test]
    fn test_metadata_file_row_persists_contract_for_own_file() {
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        let metadata = serde_json::json!({
            "id": "file-1",
            "path": "Projects/spec.docx",
            "parent_id": "folder-1",
            "size_bytes": 4096,
            "updated_at": 1234,
            "content_type": "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            "version_number": 7,
            "object_version_id": "object-7"
        });

        // This fixture carries a plaintext `path` and no `name_encrypted`, so
        // it exercises the plaintext fallback in `resolve_relative_path`; the
        // master key is unused on that branch.
        let entry = apply_metadata_file_row(
            &db,
            &metadata,
            Namespace::MyFiles,
            None,
            None,
            PERMISSION_READ | PERMISSION_WRITE | PERMISSION_OWNER,
            2000,
            &[0u8; 32],
        )
        .unwrap()
        .unwrap();

        assert_eq!(entry.status, FileStatus::CloudOnly);
        assert_eq!(entry.remote_updated_at, 1234);
        let contract = db.get_file_contract_state("file-1").unwrap().unwrap();
        assert_eq!(contract.namespace, Namespace::MyFiles);
        assert_eq!(contract.parent_id.as_deref(), Some("folder-1"));
        assert_eq!(
            contract.permission_bits,
            PERMISSION_READ | PERMISSION_WRITE | PERMISSION_OWNER
        );
        assert_eq!(
            contract.content_type.as_deref(),
            Some("application/vnd.openxmlformats-officedocument.wordprocessingml.document")
        );
        assert_eq!(contract.current_version, 7);
        assert_eq!(contract.current_object_version_id.as_deref(), Some("object-7"));
        assert_eq!(contract.last_sync_at, 2000);
    }

    #[test]
    fn test_metadata_file_row_decrypts_encrypted_name_into_path() {
        // E2EE invariant: the server returns the filename only as the
        // encrypted `name_encrypted` blob (no plaintext path). The ingest must
        // decrypt it with the master key and store the plaintext relative path
        // so placeholder seeding + hydration can resolve a real destination.
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();

        let master_key = [7u8; 32];
        let file_id = "f1e2d3c4-0000-4000-8000-000000000001";
        // Encrypt the name exactly as every client does (shared core path).
        let mk = beebeeb_core::kdf::MasterKey::from_bytes(master_key);
        let name_encrypted =
            beebeeb_core::encrypt::encrypt_name(&mk, file_id, "Quarterly Report.pdf", Some("application/pdf"))
                .unwrap();

        let metadata = serde_json::json!({
            "id": file_id,
            "name_encrypted": name_encrypted,
            "size_bytes": 2048,
            "updated_at": 5555,
            "is_folder": false,
        });

        let entry = apply_metadata_file_row(
            &db,
            &metadata,
            Namespace::MyFiles,
            None,
            None,
            PERMISSION_READ | PERMISSION_WRITE | PERMISSION_OWNER,
            6000,
            &master_key,
        )
        .unwrap()
        .unwrap();

        // The decrypted name lands in `path` (a root-level item's relative
        // path under the sync root IS its name), not an empty string.
        assert_eq!(entry.path, "Quarterly Report.pdf");
        assert_eq!(entry.status, FileStatus::CloudOnly);
        let stored = db.get_file(file_id).unwrap().unwrap();
        assert_eq!(stored.path, "Quarterly Report.pdf");
    }

    #[test]
    fn test_metadata_file_row_falls_back_when_name_undecryptable() {
        // A garbled/foreign blob must not abort the sweep nor poison the row:
        // resolve_relative_path falls through to any plaintext field, else "".
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();

        let metadata = serde_json::json!({
            "id": "f1e2d3c4-0000-4000-8000-000000000002",
            "name_encrypted": "{\"cipher_suite\":\"V1Aes256Gcm\",\"nonce\":[],\"ciphertext\":[]}",
            "size_bytes": 10,
            "updated_at": 1,
            "is_folder": false,
        });

        let entry = apply_metadata_file_row(
            &db,
            &metadata,
            Namespace::MyFiles,
            None,
            None,
            PERMISSION_READ | PERMISSION_WRITE | PERMISSION_OWNER,
            1,
            &[9u8; 32],
        )
        .unwrap()
        .unwrap();

        // No plaintext field present → empty path (caller skips seeding),
        // but the row still upserts so the rest of the sweep proceeds.
        assert_eq!(entry.path, "");
    }

    #[tokio::test]
    async fn test_process_due_operations_completes_pin_marker_and_invalidates_file() {
        let dir = tempfile::tempdir().unwrap();
        let bridge = test_bridge(&dir.path().join("state.db"));
        bridge
            .db
            .enqueue_operation(&PendingOperation {
                op_id: "op-pin".into(),
                kind: OperationKind::PinTree,
                file_id: Some("folder-1".into()),
                parent_id: None,
                target_path: None,
                metadata_json: Some(r#"{"operation":"pin_tree","pinned":true}"#.into()),
                payload_path: None,
                base_version: None,
                base_object_version_id: None,
                attempts: 0,
                max_attempts: 5,
                next_retry_at: 0,
                last_error: None,
                created_at: 100,
                updated_at: 100,
            })
            .unwrap();

        let outcome = bridge.process_due_operations(dir.path(), 200).await.unwrap();
        assert_eq!(outcome.completed_op_ids, vec!["op-pin".to_string()]);
        assert_eq!(outcome.invalidated_item_ids, vec!["folder-1".to_string()]);
        assert!(bridge.db.list_due_operations(999).unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_process_due_operations_records_retry_for_upload_worker_handoff() {
        let dir = tempfile::tempdir().unwrap();
        let bridge = test_bridge(&dir.path().join("state.db"));
        bridge
            .db
            .enqueue_operation(&PendingOperation {
                op_id: "op-upload".into(),
                kind: OperationKind::UploadVersion,
                file_id: Some("file-1".into()),
                parent_id: None,
                target_path: Some("Draft.txt".into()),
                metadata_json: Some(r#"{"operation":"upload_version"}"#.into()),
                payload_path: Some(dir.path().join("payload").to_string_lossy().into_owned()),
                base_version: Some(1),
                base_object_version_id: None,
                attempts: 0,
                max_attempts: 5,
                next_retry_at: 0,
                last_error: None,
                created_at: 100,
                updated_at: 100,
            })
            .unwrap();

        let outcome = bridge.process_due_operations(dir.path(), 200).await.unwrap();
        assert_eq!(outcome.retried_op_ids, vec!["op-upload".to_string()]);
        assert!(outcome.paused_op_ids.is_empty());

        let due_too_early = bridge.db.list_due_operations(229).unwrap();
        assert!(due_too_early.is_empty());
        let due = bridge.db.list_due_operations(230).unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].attempts, 1);
        assert!(
            due[0]
                .last_error
                .as_deref()
                .unwrap_or("")
                .contains("staged upload payload is missing")
        );
    }

    #[tokio::test]
    async fn test_process_due_operations_uploads_encrypted_payload_and_commits_state() {
        let dir = tempfile::tempdir().unwrap();
        let payload = dir.path().join("payload.txt");
        std::fs::write(&payload, b"live upload payload").unwrap();
        let server = UploadMockServer::start(false);
        let master_key = [11u8; 32];
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        bridge
            .db
            .enqueue_operation(&PendingOperation {
                op_id: "op-upload-live".into(),
                kind: OperationKind::UploadVersion,
                file_id: Some("local-file-1".into()),
                parent_id: Some("folder-1".into()),
                target_path: Some("Reports/report.txt".into()),
                metadata_json: Some(
                    serde_json::json!({
                        "operation": "create_file",
                        "name_encrypted": "{\"cipher_suite\":\"V1Aes256Gcm\"}",
                        "display_name": "report.txt",
                        "content_type": "text/plain"
                    })
                    .to_string(),
                ),
                payload_path: Some(payload.to_string_lossy().into_owned()),
                base_version: None,
                base_object_version_id: None,
                attempts: 0,
                max_attempts: 5,
                next_retry_at: 0,
                last_error: None,
                created_at: 100,
                updated_at: 100,
            })
            .unwrap();

        let outcome = bridge.process_due_operations(dir.path(), 200).await.unwrap();
        assert_eq!(outcome.completed_op_ids, vec!["op-upload-live".to_string()]);
        assert!(outcome.retried_op_ids.is_empty());
        assert!(
            !payload.exists(),
            "staged payload should be removed only after complete succeeds"
        );
        assert!(bridge.db.list_due_operations(999).unwrap().is_empty());
        assert!(bridge.db.get_file("local-file-1").unwrap().is_none());

        let entry = bridge.db.get_file("server-file-1").unwrap().unwrap();
        assert_eq!(entry.status, FileStatus::Local);
        assert_eq!(entry.path, "Reports/report.txt");
        assert_eq!(entry.size_bytes, 19);

        let contract = bridge.db.get_file_contract_state("server-file-1").unwrap().unwrap();
        assert_eq!(contract.current_version, 1);
        assert_eq!(contract.local_base_version, 1);
        assert_eq!(contract.current_object_version_id.as_deref(), Some("object-complete-1"));
        assert_eq!(contract.content_type.as_deref(), Some("text/plain"));

        let requests = server.finish();
        assert_eq!(requests.len(), 4);
        assert_eq!(requests[0].method, "POST");
        assert_eq!(requests[0].path, "/api/v1/uploads/init");
        let init_body: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert!(
            init_body.get("file_id").is_none(),
            "new-file uploads must let the server mint the id"
        );
        assert_eq!(init_body["file_size_bytes"], 19);
        assert_eq!(init_body["parent_id"], "folder-1");
        assert_eq!(init_body["chunk_count"], 1);
        assert_eq!(requests[1].method, "PATCH");
        assert_eq!(requests[2].method, "PUT");
        assert_eq!(requests[2].path, "/api/v1/uploads/upload-session-1/chunks/0");
        assert_ne!(requests[2].body, b"live upload payload");

        let master_key = beebeeb_core::kdf::MasterKey::from_bytes(master_key);
        let file_key = beebeeb_core::kdf::derive_file_key(&master_key, b"server-file-1");
        assert_eq!(
            beebeeb_core::encrypt::decrypt_chunk_raw(&file_key, &requests[2].body).unwrap(),
            b"live upload payload"
        );
        assert_eq!(requests[3].method, "POST");
        assert_eq!(requests[3].path, "/api/v1/uploads/upload-session-1/complete");
    }

    #[tokio::test]
    async fn test_process_due_operations_preserves_payload_when_upload_chunk_fails() {
        let dir = tempfile::tempdir().unwrap();
        let payload = dir.path().join("payload.txt");
        std::fs::write(&payload, b"retry me").unwrap();
        let server = UploadMockServer::start(true);
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), [12u8; 32]);
        bridge
            .db
            .enqueue_operation(&PendingOperation {
                op_id: "op-upload-retry".into(),
                kind: OperationKind::UploadVersion,
                file_id: Some("local-file-2".into()),
                parent_id: None,
                target_path: Some("retry.txt".into()),
                metadata_json: Some(
                    serde_json::json!({
                        "operation": "create_file",
                        "name_encrypted": "{\"cipher_suite\":\"V1Aes256Gcm\"}",
                        "display_name": "retry.txt",
                        "content_type": "text/plain"
                    })
                    .to_string(),
                ),
                payload_path: Some(payload.to_string_lossy().into_owned()),
                base_version: None,
                base_object_version_id: None,
                attempts: 0,
                max_attempts: 5,
                next_retry_at: 0,
                last_error: None,
                created_at: 100,
                updated_at: 100,
            })
            .unwrap();

        let outcome = bridge.process_due_operations(dir.path(), 200).await.unwrap();
        assert_eq!(outcome.retried_op_ids, vec!["op-upload-retry".to_string()]);
        assert!(outcome.completed_op_ids.is_empty());
        assert!(payload.exists(), "failed uploads must preserve the staged payload");

        let due_too_early = bridge.db.list_due_operations(229).unwrap();
        assert!(due_too_early.is_empty());
        let due = bridge.db.list_due_operations(230).unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].attempts, 1);
        assert!(
            due[0]
                .last_error
                .as_deref()
                .unwrap_or("")
                .contains("500 Internal Server Error")
        );

        let requests = server.finish();
        assert_eq!(requests.len(), 3);
        assert_eq!(requests[0].path, "/api/v1/uploads/init");
        assert_eq!(requests[1].path, "/api/v1/files/server-file-1");
        assert_eq!(requests[2].path, "/api/v1/uploads/upload-session-1/chunks/0");
    }

    #[test]
    fn test_read_only_shared_item_rejects_finder_writes() {
        let dir = tempfile::tempdir().unwrap();
        let bridge = test_bridge(&dir.path().join("state.db"));
        seed_bridge_row(
            &bridge,
            "shared-readonly",
            "/Shared with me/Readonly.txt",
            None,
            FileStatus::Local,
            100,
        );
        let mut contract = bridge.db.get_file_contract_state("shared-readonly").unwrap().unwrap();
        contract.namespace = Namespace::SharedWithMe;
        contract.shared_root_id = Some("shared-readonly".into());
        contract.share_id = Some("invite-1".into());
        contract.permission_bits = PERMISSION_READ;
        bridge.db.set_file_contract_state(&contract).unwrap();

        let err = bridge
            .queue_finder_delete("shared-readonly", Some("1:1:100".into()))
            .unwrap_err()
            .to_string();
        assert!(err.contains("read-only shared item"));
        assert!(bridge.db.list_due_operations(now_secs()).unwrap().is_empty());
    }

    #[test]
    fn test_editable_shared_write_queues_actor_and_share_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("shared.txt");
        std::fs::write(&source, b"shared payload").unwrap();
        let bridge = test_bridge(&dir.path().join("state.db"));
        seed_bridge_row(
            &bridge,
            "shared-editable",
            "/Shared with me/Editable.txt",
            None,
            FileStatus::Local,
            100,
        );
        let mut contract = bridge.db.get_file_contract_state("shared-editable").unwrap().unwrap();
        contract.namespace = Namespace::SharedWithMe;
        contract.shared_root_id = Some("shared-root".into());
        contract.share_id = Some("invite-2".into());
        contract.permission_bits = PERMISSION_READ | PERMISSION_WRITE;
        contract.current_object_version_id = Some("object-1".into());
        bridge.db.set_file_contract_state(&contract).unwrap();

        bridge
            .queue_finder_modify(FinderWriteTarget {
                file_id: Some("shared-editable".into()),
                parent_id: None,
                filename: "Editable.txt".into(),
                kind: FinderWriteItemKind::File,
                contents_path: Some(source.to_string_lossy().into_owned()),
                content_type: Some("text/plain".into()),
                base_version_identifier: Some("3:2:100".into()),
            })
            .unwrap();

        let queued = bridge.db.list_due_operations(now_secs()).unwrap();
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].kind, OperationKind::UploadVersion);
        assert_eq!(queued[0].base_version, Some(3));
        assert_eq!(queued[0].base_object_version_id.as_deref(), Some("object-1"));
        let metadata: serde_json::Value = serde_json::from_str(queued[0].metadata_json.as_deref().unwrap()).unwrap();
        assert_eq!(metadata["shared_root_id"], "shared-root");
        assert_eq!(metadata["share_id"], "invite-2");
        assert_eq!(metadata["uploaded_by"], "authenticated_desktop_user");
    }

    #[test]
    fn test_version_center_feed_maps_conflicts_and_review_operations() {
        let dir = tempfile::tempdir().unwrap();
        let bridge = test_bridge(&dir.path().join("state.db"));
        bridge
            .db
            .upsert_file(&FileEntry {
                file_id: "conflict-file".into(),
                path: "/Work/conflict.md".into(),
                status: FileStatus::Conflict,
                size_bytes: 64,
                modified_at: 100,
                content_hash: Some("local".into()),
                remote_updated_at: 90,
            })
            .unwrap();
        bridge
            .db
            .enqueue_operation(&PendingOperation {
                op_id: "op-quota".into(),
                kind: OperationKind::UploadVersion,
                file_id: Some("quota-file".into()),
                parent_id: None,
                target_path: Some("/Work/quota.png".into()),
                metadata_json: Some(r#"{"operation":"upload_version"}"#.into()),
                payload_path: Some("/tmp/quota".into()),
                base_version: None,
                base_object_version_id: None,
                attempts: 3,
                max_attempts: 3,
                next_retry_at: 100,
                last_error: Some("quota exceeded".into()),
                created_at: 101,
                updated_at: 101,
            })
            .unwrap();
        bridge
            .db
            .enqueue_operation(&PendingOperation {
                op_id: "op-permission".into(),
                kind: OperationKind::MoveFile,
                file_id: Some("shared-file".into()),
                parent_id: Some("shared-folder".into()),
                target_path: Some("/Shared/blocked.txt".into()),
                metadata_json: Some(r#"{"operation":"metadata_update"}"#.into()),
                payload_path: None,
                base_version: None,
                base_object_version_id: None,
                attempts: 1,
                max_attempts: 5,
                next_retry_at: 100,
                last_error: Some("403 forbidden: permission denied".into()),
                created_at: 102,
                updated_at: 102,
            })
            .unwrap();
        bridge
            .db
            .enqueue_operation(&PendingOperation {
                op_id: "op-stale".into(),
                kind: OperationKind::UploadVersion,
                file_id: Some("stale-file".into()),
                parent_id: None,
                target_path: Some("/Work/stale.txt".into()),
                metadata_json: Some(r#"{"operation":"upload_version","base_version_identifier":"7:1700:12"}"#.into()),
                payload_path: Some("/tmp/stale".into()),
                base_version: Some(7),
                base_object_version_id: None,
                attempts: 0,
                max_attempts: 25,
                next_retry_at: 100,
                last_error: Some("stale base version rejected".into()),
                created_at: 103,
                updated_at: 103,
            })
            .unwrap();

        let feed = bridge.version_conflict_feed().unwrap();
        assert!(feed.iter().any(|entry| {
            entry.kind == "conflict"
                && entry.file_id == "conflict-file"
                && entry.file_name == "conflict.md"
                && entry.action == "open_conflict"
        }));
        assert!(
            feed.iter()
                .any(|entry| entry.kind == "quota_failure" && entry.file_name == "quota.png")
        );
        assert!(
            feed.iter()
                .any(|entry| entry.kind == "permission_failure" && entry.file_name == "blocked.txt")
        );
        assert!(feed.iter().any(|entry| {
            entry.kind == "stale_base"
                && entry.file_name == "stale.txt"
                && entry.base_version == Some(7)
                && entry.detail.contains("preserved")
        }));
    }

    #[test]
    fn test_restore_version_routes_to_durable_operation_for_review() {
        let dir = tempfile::tempdir().unwrap();
        let bridge = test_bridge(&dir.path().join("state.db"));

        let queued = bridge
            .queue_restore_version(
                "file-restore",
                "version-2",
                Some("direct restore failed: network offline".into()),
            )
            .unwrap();

        assert_eq!(queued.kind, OperationKind::RestoreVersion);
        assert_eq!(queued.file_id.as_deref(), Some("file-restore"));
        assert_eq!(queued.base_object_version_id.as_deref(), Some("version-2"));
        let metadata: serde_json::Value = serde_json::from_str(queued.metadata_json.as_deref().unwrap()).unwrap();
        assert_eq!(metadata["operation"], "restore_version");
        assert_eq!(metadata["version_id"], "version-2");

        let feed = bridge.version_conflict_feed().unwrap();
        assert!(feed.iter().any(|entry| {
            entry.kind == "restore"
                && entry.file_id == "file-restore"
                && entry.version_id.as_deref() == Some("version-2")
                && entry.last_error.as_deref() == Some("direct restore failed: network offline")
        }));
    }

    fn seed_bridge_row(
        bridge: &EngineBridge,
        file_id: &str,
        path: &str,
        parent_id: Option<&str>,
        status: FileStatus,
        cache_bytes: i64,
    ) {
        bridge
            .db
            .upsert_file(&FileEntry {
                file_id: file_id.into(),
                path: path.into(),
                status,
                size_bytes: cache_bytes,
                modified_at: 0,
                content_hash: None,
                remote_updated_at: 0,
            })
            .unwrap();
        let mut contract = bridge.db.get_file_contract_state(file_id).unwrap().unwrap();
        contract.parent_id = parent_id.map(str::to_string);
        contract.cache_bytes = cache_bytes;
        bridge.db.set_file_contract_state(&contract).unwrap();
    }
}
