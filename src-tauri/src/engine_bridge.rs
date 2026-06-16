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

use std::collections::{HashSet, VecDeque};
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
    /// Full server-relative, '/'-joined path of this item under the sync root
    /// (e.g. `docs/a.txt` for a nested file, `a.txt` at the root). When `None`,
    /// the path defaults to the leaf `filename` — the legacy top-level behaviour.
    ///
    /// **Why this exists (task 0780 nested correctness):** the local-create path
    /// stores this as the row's `path` and threads it through as the upload's
    /// `target_path`, so (a) `classify_local_path`'s "already a known server
    /// file?" filter (which queries the FULL relative key) matches a nested
    /// file's row immediately, (b) `finalize_local_upload_placeholder` joins
    /// `sync_root + rel_path` to find the file on disk and re-stamp it in-sync,
    /// and (c) a local delete looks up the full key and trashes it on the server.
    /// Without it, nested rows were keyed by the leaf only and all three broke
    /// until the next `sync_tick` re-keyed them. Top-level is unaffected (leaf ==
    /// full key). The OS-extension IPC paths that don't supply it keep the leaf
    /// fallback.
    #[serde(default)]
    pub rel_path: Option<String>,
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
                // Server-authoritative deletion (task 0802). `trash_file` calls
                // `.error_for_status()`, so an `Ok` here means the server returned
                // HTTP 2xx for `DELETE /files/{id}` (it set `is_trashed=TRUE` and
                // emitted a `file_trash` sync op).
                //
                // We deliberately DO NOT delete the local row here. The row was
                // parked in `Trashing` by `watcher::handle_delete`; we KEEP it in
                // `Trashing` as the durable hidden-locally marker. Returning `Ok`
                // makes `process_due_operations` REMOVE this op — so the
                // `Trashing` status (not the op) is now what protects the row.
                //
                // Why not delete now: deleting + a same-tick / replica-lagged
                // `/sync/snapshot` that still lists the (just-trashed) file would
                // re-insert a FRESH `CloudOnly` row and re-mint the placeholder —
                // the "deleted file comes back" race. Instead we let convergence
                // happen authoritatively: while the snapshot still lists the file
                // the `Trashing` guard in `process_metadata_row` preserves it; once
                // the trash propagates the file is ABSENT from the snapshot and
                // `prune_absent` removes the (now op-less) `Trashing` row. If the
                // `file_trash` op echo arrives first, `apply_sync_op` deletes the
                // row directly — either path converges.
                self.api.trash_file(file_id).await?;
                // Zero-knowledge: log the file_id (an opaque server id) only —
                // NEVER the path. Lets the lead confirm the 2xx in the trace.
                tracing::info!(
                    file_id = %file_id,
                    "trash op: server DELETE /files/{{id}} returned 2xx — keeping row Trashing, dropping op"
                );
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
            OperationKind::UploadVersion | OperationKind::UploadFile => self.upload_version(op, sync_root).await,
        }
    }

    async fn upload_version(&self, op: &PendingOperation, sync_root: &Path) -> anyhow::Result<()> {
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

        // Windows Cloud Files (task 0780): the file the user dropped in the
        // sync root is, at this point, a PLAIN local file — Explorer shows it
        // as always-local, and (worse) it has no Cloud Files identity, so it
        // can never be dehydrated by "Free up space" or re-hydrated on demand.
        // Convert it IN PLACE into an in-sync placeholder carrying the server
        // file_id, so it shows the synced overlay and round-trips through the
        // same fetch/dehydrate machinery as a downloaded file. Keyed on a
        // `create_file` op with a `target_path` (the happy path: a brand-new
        // user file). Best-effort — a failure here leaves a working local file,
        // it just won't show the synced overlay until the next reconcile.
        #[cfg(target_os = "windows")]
        self.finalize_local_upload_placeholder(op, &server_file_id, sync_root);

        Ok(())
    }

    /// Convert a freshly-uploaded NEW local file (still a plain file on disk)
    /// into an in-sync Cloud Files placeholder. Windows-only, best-effort.
    /// Only runs for `create_file` operations that carry a `target_path` — the
    /// happy path of task 0780. Modify-as-new-version is a deferred follow-up
    /// and is intentionally not converted here.
    #[cfg(target_os = "windows")]
    fn finalize_local_upload_placeholder(&self, op: &PendingOperation, server_file_id: &str, sync_root: &Path) {
        let Some(metadata) = op.metadata_json.as_deref() else {
            return;
        };
        let is_create = serde_json::from_str::<serde_json::Value>(metadata)
            .map(|m| m["operation"].as_str() == Some("create_file"))
            .unwrap_or(false);
        if !is_create {
            return;
        }
        let Some(target_path) = op.target_path.as_deref() else {
            return;
        };
        let on_disk = sync_root.join(target_path.trim_start_matches('/').replace('/', std::path::MAIN_SEPARATOR_STR));
        if !on_disk.is_file() {
            // The user moved/deleted it between drop and upload — nothing to
            // convert; the placeholder seeder will mint a cloud-only stub on a
            // later tick from the server row instead.
            return;
        }
        if let Err(e) = crate::windows_cf::placeholders::convert_to_in_sync_placeholder(&on_disk, server_file_id) {
            // Zero-knowledge: log the file_id only, never the path/filename.
            tracing::warn!(file_id = %server_file_id, error = %e, "could not convert uploaded file to in-sync placeholder");
        }
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
            // parent_id/item_kind are not written by upsert_file — the
            // contract update below owns them. These are inert defaults.
            parent_id: op.parent_id.clone(),
            item_kind: ItemKind::File,
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
                // upsert_file does not persist these; the contract update
                // just below sets the authoritative parent_id/item_kind.
                parent_id: None,
                item_kind: if root.is_folder { ItemKind::Folder } else { ItemKind::File },
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
        // Full server-relative key for this item: the FULL nested path when the
        // caller supplied one (`docs/a.txt`), else the leaf filename (top-level /
        // legacy IPC). This is stored as the row's `path` AND threaded as the
        // upload's `target_path`, so filter-3, finalize, and delete all key off
        // the same path. `clone` because `filename` is still needed for the
        // server `name` (always the leaf).
        let rel_path = target.rel_path.clone().unwrap_or_else(|| target.filename.clone());
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
                    Some(rel_path),
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
                    // FULL relative key (e.g. `docs/a.txt`), so a nested file's
                    // row is found by `get_file_by_path(full_key)` immediately —
                    // not keyed by the leaf, which classify_local_path's filter-3
                    // would miss for a nested file → spurious re-upload.
                    path: rel_path.clone(),
                    status: FileStatus::Uploading,
                    size_bytes,
                    modified_at: now_secs(),
                    content_hash: None,
                    remote_updated_at: 0,
                    // Not persisted by upsert_file; contract owns these.
                    parent_id: None,
                    item_kind: ItemKind::File,
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
                    // target_path = the FULL relative key, so
                    // finalize_local_upload_placeholder joins sync_root + this
                    // and finds the nested file on disk to re-stamp it in-sync.
                    Some(rel_path),
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

    /// Single, parent-aware classifier shared by every local-write trigger
    /// (the retired `notify` watcher *and* the Windows Cloud Files NOTIFY
    /// callbacks). Given an absolute on-disk `path` under `sync_root`, it
    /// decides whether `path` is a *genuinely-new* user file that should be
    /// uploaded, and if so returns a ready-to-queue [`FinderWriteTarget`] with
    /// `parent_id` resolved from the path's parent directory.
    ///
    /// It runs the three feedback-loop filters, in order, so a download / a
    /// hydration / an engine-internal write is NEVER mistaken for a new upload:
    ///
    /// 1. **Engine-internal paths + OS junk** — anything under `<root>/.beebeeb/`,
    ///    the `<root>/.beebeeb-sync.lock`, any `.beebeeb` path component, and the
    ///    ignored-name set ([`is_ignored_finder_name`]: `.tmp`, `~$…`, `.DS_Store`,
    ///    …). These are writes the engine itself (or the OS) makes constantly.
    /// 2. **Cloud Files placeholders** (Windows only) — a reparse point under the
    ///    sync root is something WE minted (placeholder seed or post-upload
    ///    convert). Hydration fills a placeholder's data stream WITHOUT clearing
    ///    its reparse-point attribute, so a hydration-write still trips this guard
    ///    and never re-uploads. Checked via
    ///    [`crate::windows_cf::placeholders::is_cloud_placeholder`].
    /// 3. **Already a known server file** (authoritative) — look the path up in
    ///    the state DB. A row means the file already lives on the server
    ///    (cloud-only / downloading / local / uploading), so a write to it is a
    ///    hydration or a re-download, NOT a new creation. Only a path with NO DB
    ///    row survives.
    ///
    /// `parent_id` resolution (nested uploads): for a survivor at
    /// `<parent_dir>/<name>`, the parent directory's server folder id is looked
    /// up by its server-relative path. A root-level file resolves to `None`; a
    /// nested file resolves to `Some(<parent folder file_id>)` so the upload
    /// lands in the right server folder. If the parent directory has no DB row
    /// yet (its folder placeholder hasn't reconciled), `parent_id` falls back to
    /// `None` rather than guessing — the file uploads to the root and a later
    /// tick can reconcile, which is strictly safer than attaching it to the
    /// wrong parent.
    ///
    /// Returns `None` when `path` is not a regular file, is filtered out, or the
    /// DB lookup fails (fail-closed: a lookup error skips the upload rather than
    /// risk a spurious one). Cross-platform so it type-checks everywhere; only
    /// the Windows triggers call it.
    pub fn classify_local_path(&self, sync_root: &Path, path: &Path) -> Option<FinderWriteTarget> {
        // The file may have been deleted/renamed during a debounce window — if
        // it is no longer a regular file there is nothing to upload.
        match std::fs::symlink_metadata(path) {
            Ok(m) if m.is_file() => {}
            _ => return None,
        }

        // Filter 1 — engine-internal paths + OS junk.
        if path_is_engine_internal(sync_root, path) {
            return None;
        }
        let file_name = path.file_name().and_then(|n| n.to_str())?.to_string();
        if is_ignored_finder_name(&file_name) {
            return None;
        }

        // Filter 2 — Cloud Files placeholders are engine-owned (Windows only). A
        // reparse point under the sync root is a placeholder we minted or
        // converted; the hydration write that fills it does NOT turn it back into
        // a plain file, so we must never treat a placeholder write as a new
        // upload.
        #[cfg(target_os = "windows")]
        if crate::windows_cf::placeholders::is_cloud_placeholder(path) {
            return None;
        }

        // Filter 3 (authoritative) — already a known server file?
        let rel = relative_db_path(sync_root, path)?;
        match self.db.get_file_by_path(&rel) {
            Ok(Some(_existing)) => {
                tracing::trace!("classify_local_path: skipping write to already-tracked server file");
                return None;
            }
            Ok(None) => { /* genuinely new — fall through */ }
            Err(e) => {
                tracing::warn!(error = %e, "classify_local_path: state DB lookup failed; skipping to be safe");
                return None;
            }
        }

        // parent_id resolution: a survivor at `<parent_dir>/<name>`. If the parent
        // directory maps to a known server FOLDER row, attach the new file to it;
        // otherwise upload at the root (None). We never attach to a row that isn't
        // a folder.
        let parent_id = self.resolve_parent_id_for(sync_root, path);

        Some(FinderWriteTarget {
            file_id: None,
            parent_id,
            filename: file_name.clone(),
            // The FULL '/'-joined relative key (e.g. `docs/a.txt`), so the row is
            // stored + the upload's target_path is threaded under the same key
            // that filter-3, finalize, and delete all query. `rel` was computed
            // above for the filter-3 lookup; reuse it verbatim.
            rel_path: Some(rel),
            kind: FinderWriteItemKind::File,
            contents_path: Some(path.to_string_lossy().into_owned()),
            content_type: beebeeb_core::media::guess_mime_type(&file_name).map(str::to_string),
            base_version_identifier: None,
        })
    }

    /// Resolve the server folder id of `path`'s immediate parent directory, for
    /// nested uploads. Returns `None` for a root-level file (parent == sync root)
    /// or when the parent directory has no known server FOLDER row yet. The
    /// parent's server-relative key is built in the same '/'-joined shape the
    /// state DB stores, then looked up via [`StateDb::get_file_by_path`]; a hit
    /// that is a folder yields its `file_id`.
    ///
    /// `pub(crate)` so the upload driver's rename handler can resolve the NEW
    /// parent of a moved file through the exact same logic.
    pub(crate) fn resolve_parent_id_for(&self, sync_root: &Path, path: &Path) -> Option<String> {
        let parent_dir = path.parent()?;
        // Root-level file: parent IS the sync root → no server parent.
        if parent_dir == sync_root {
            return None;
        }
        let parent_rel = relative_db_path(sync_root, parent_dir)?;
        match self.db.get_file_by_path(&parent_rel) {
            Ok(Some(entry)) if entry.is_dir() => Some(entry.file_id),
            // No row, or a row that is somehow a file (shouldn't happen for a
            // directory path) → upload at the root rather than mis-parent.
            _ => None,
        }
    }

    /// Apply a pin ("available offline") toggle to `file_id` and its subtree.
    ///
    /// `sync_root` is the on-disk root of the vault — needed on Windows to
    /// resolve each affected DB row to its placeholder path and set the OS-level
    /// pin state. It is unused on macOS/Linux (the `#[cfg]` below silences the
    /// warning) but threaded in unconditionally to keep one signature.
    ///
    /// Three things happen on a pin change:
    /// 1. **DB** — `db.set_recursive_pin` flips the `pin_state` column for the
    ///    whole subtree and returns the ids whose state actually changed.
    /// 2. **Server** — `enqueue_pin_tree_operation` propagates the pin so OTHER
    ///    devices learn about it. Always runs (every platform).
    /// 3. **OS hydration** —
    ///    - **Windows**: `CfSetPinState` is the OS contract. A pinned placeholder
    ///      is hydrated by Windows via the normal FETCH_DATA path AND is exempt
    ///      from auto-dehydration, so it stays resident. We therefore do NOT
    ///      enqueue our own out-of-band hydrate ops (the old DB-only model did,
    ///      because Windows was never told about the pin — that loop is dropped on
    ///      Windows). New children inherit the parent pin via CF_PIN_STATE_INHERIT.
    ///    - **macOS/Linux**: there is no CF pin primitive, so we keep enqueuing an
    ///      explicit hydrate op per newly-pinned cloud-only file to materialise it.
    pub fn set_recursive_pin(
        &self,
        #[cfg_attr(not(target_os = "windows"), allow(unused_variables))] sync_root: &Path,
        file_id: &str,
        pinned: bool,
    ) -> anyhow::Result<PinUpdateOutcome> {
        let now = now_secs();
        let changed_item_ids = self.db.set_recursive_pin(file_id, pinned, now)?;
        // Mutated only on the non-Windows hydrate-enqueue path below; on Windows
        // the OS (CfSetPinState) drives hydration, so this stays 0.
        #[cfg_attr(target_os = "windows", allow(unused_mut))]
        let mut hydrate_operations = 0usize;

        self.enqueue_pin_tree_operation(file_id, pinned, now)?;

        #[cfg(target_os = "windows")]
        {
            // OS-level pin: tell Windows the actual "available offline" state so a
            // pinned file is kept resident (no auto-dehydration) and an unpinned
            // one becomes reclaim-eligible again. We pin/unpin only the TOP item
            // the user toggled, with RECURSE when it is a directory — the
            // recursive flag stamps existing descendants in one call and
            // CF_PIN_STATE_INHERIT covers descendants created later — rather than
            // issuing a per-row CfSetPinState for every changed id.
            if let Some(top) = self.db.get_file(file_id)? {
                if let Some(path) = Self::placeholder_path_under(sync_root, &top.path) {
                    let recurse = top.is_dir();
                    if let Err(e) = crate::windows_cf::placeholders::set_pin_state(&path, pinned, recurse) {
                        // Zero-knowledge: never log the path. A pin-state failure
                        // must not abort the DB+server pin that already succeeded;
                        // surface it as a log line and continue.
                        tracing::warn!(file_id = %file_id, pinned, recurse, error = %e, "CfSetPinState failed for pin toggle");
                    }
                }

                // Proactive hydrate. CfSetPinState(PINNED) marks the subtree
                // pinned but Windows does NOT eagerly download a not-yet-opened
                // placeholder — a pinned-but-unopened file stays cloud-only
                // (RECALL_ON_DATA_ACCESS) until something reads it. So "available
                // offline" is only real once we force the download. Collect the
                // cloud-only file descendants (the toggled item itself if it is a
                // cloud-only file, or every cloud-only file under it if a folder)
                // and CfHydratePlaceholder each. We only ever hydrate on pin, not
                // unpin (unpinning just makes files reclaim-eligible again).
                if pinned {
                    match self.db.cloud_only_file_descendants(file_id) {
                        Ok(entries) if !entries.is_empty() => {
                            // Resolve placeholder paths the SAME way the pin does,
                            // dropping any that fail the containment guard.
                            let paths: Vec<PathBuf> = entries
                                .iter()
                                .filter_map(|e| Self::placeholder_path_under(sync_root, &e.path))
                                .collect();
                            // CfHydratePlaceholder BLOCKS until each file's bytes
                            // are on disk, and a folder may hold many/large files,
                            // so run the whole sweep OFF the IPC thread — set_recursive_pin
                            // returns immediately while hydration proceeds in the
                            // background. A per-file failure is logged and never
                            // aborts the rest of the sweep.
                            let total = paths.len();
                            std::thread::spawn(move || {
                                tracing::debug!(count = total, "proactive pin hydrate: starting");
                                let mut ok = 0usize;
                                for path in &paths {
                                    match crate::windows_cf::placeholders::hydrate_placeholder(path) {
                                        Ok(()) => ok += 1,
                                        // Zero-knowledge: never log the path.
                                        Err(e) => tracing::warn!(error = %e, "proactive pin hydrate: file failed"),
                                    }
                                }
                                tracing::debug!(hydrated = ok, total, "proactive pin hydrate: finished");
                            });
                        }
                        Ok(_) => {} // nothing cloud-only to hydrate
                        Err(e) => {
                            // A query failure must not abort the DB+server pin that
                            // already succeeded; log and continue.
                            tracing::warn!(file_id = %file_id, error = %e, "proactive pin hydrate: descendant query failed");
                        }
                    }
                }
            }
        }

        #[cfg(not(target_os = "windows"))]
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

    /// Map a server-relative, '/'-joined DB key (`FileEntry::path`) to its
    /// on-disk placeholder path under `sync_root`, rejecting empty/traversal
    /// keys. Mirrors `windows_cf::callbacks::safe_join_under_root` (kept private
    /// there) so the pin path resolution uses the same containment guard as the
    /// fetch fallback. Returns `None` for an empty key or any `.`/`..`/empty
    /// segment, or if the join escapes the root.
    #[cfg(target_os = "windows")]
    fn placeholder_path_under(sync_root: &Path, rel_path: &str) -> Option<PathBuf> {
        let rel = rel_path.trim_matches('/');
        if rel.is_empty() {
            return None;
        }
        if rel.split('/').any(|seg| seg.is_empty() || seg == "." || seg == "..") {
            return None;
        }
        let native_rel = rel.replace('/', std::path::MAIN_SEPARATOR_STR);
        let candidate = sync_root.join(&native_rel);
        if !candidate.starts_with(sync_root) {
            return None;
        }
        Some(candidate)
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

/// Name of the per-sync-root state directory (mirrors `runner::STATE_DIR`).
const SYNC_STATE_DIR: &str = ".beebeeb";
/// Name of the cross-process lock file the engine writes at the sync root.
const SYNC_LOCK_FILE: &str = ".beebeeb-sync.lock";

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
    // Server-relative path of the PARENT directory ("" at the vault root).
    // The decrypted leaf name is composed under it so nested files/folders
    // get a full `<parent>/<leaf>` path that round-trips with the upload
    // watcher's `relative_db_path` (also '/'-joined, no leading slash).
    parent_rel_path: &str,
) -> anyhow::Result<Option<FileEntry>> {
    let file_id = f["id"].as_str().unwrap_or_default();
    if file_id.is_empty() {
        return Ok(None);
    }

    let existing = db.get_file(file_id)?;
    let size = f["size_bytes"].as_i64().or_else(|| f["size"].as_i64()).unwrap_or(0);
    let remote_updated = f["updated_at"].as_i64().unwrap_or(0);
    let leaf = resolve_relative_path(f, file_id, master_key);
    // Compose the nested path. An empty leaf means the name didn't decrypt;
    // keep it empty so the placeholder seeder skips the row (and retries next
    // tick) rather than seeding a bare parent dir. A leaf that already carries
    // a leading slash (legacy plaintext-`path` fallback) is normalised.
    let path = if parent_rel_path.is_empty() || leaf.is_empty() {
        leaf
    } else {
        format!("{}/{}", parent_rel_path.trim_end_matches('/'), leaf.trim_start_matches('/'))
    };
    let status = existing
        .as_ref()
        .map(|entry| entry.status.clone())
        .unwrap_or(FileStatus::CloudOnly);

    let item_kind = if f["is_folder"].as_bool().unwrap_or(false)
        || f["kind"].as_str() == Some("folder")
        || f["type"].as_str() == Some("folder")
    {
        ItemKind::Folder
    } else {
        ItemKind::File
    };
    let server_parent_id = f["parent_id"].as_str().map(str::to_string);

    let entry = FileEntry {
        file_id: file_id.to_string(),
        path,
        status,
        size_bytes: size,
        modified_at: remote_updated,
        content_hash: existing.as_ref().and_then(|entry| entry.content_hash.clone()),
        remote_updated_at: remote_updated,
        // NOTE: parent_id/item_kind are NOT persisted by upsert_file — the
        // `set_file_contract_state` call at the end of this fn writes the
        // authoritative `files.parent_id` / `files.item_kind` columns. These
        // struct fields exist so the in-memory `FileEntry` returned to callers
        // (notably the windows_cf placeholder seeder reading via list_by_status)
        // carries the correct folder/parent classification.
        parent_id: server_parent_id.clone(),
        item_kind: item_kind.clone(),
    };
    db.upsert_file(&entry)?;
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
    contract.parent_id = server_parent_id;
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
pub async fn sync_tick(
    bridge: &EngineBridge,
    // The on-disk vault root. Needed on Windows so the deletion-reconcile paths
    // (`apply_sync_op` / `apply_snapshot`) can locate and remove the on-disk Cloud
    // Files placeholder of a remotely-deleted row — not just its DB row (task
    // 0806). Unused on macOS/Linux (the OS extension owns the namespace there);
    // `#[cfg_attr]` silences the unused warning on those builds.
    #[cfg_attr(not(target_os = "windows"), allow(unused_variables))] sync_root: &Path,
) -> anyhow::Result<Vec<ConflictDetected>> {
    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let mut conflicts: Vec<ConflictDetected> = Vec::new();

    // ── /sync delta engine (task 0789) ────────────────────────────────────────
    //
    // Replaces the old per-folder full-tree `/files` BFS re-walk (`1 + folders`
    // requests/tick, never pruned server deletions, self-saturated the per-IP
    // 429 limit). Two cheap paths, gated on whether the op cursor has EVER been
    // persisted (`Option`, NOT a 0 sentinel):
    //
    //   cursor UNSET (never bootstrapped) → BOOTSTRAP via ONE `GET /sync/snapshot`:
    //     the snapshot is the authoritative non-trashed tree, so we (a) apply
    //     every node through the SAME ingest path the BFS used
    //     (`process_metadata_row`, which keeps conflict detection identical +
    //     flows rows through `upsert_file` for Windows `refresh_placeholders`),
    //     (b) PRUNE every own-tree row the snapshot omits (the
    //     deletion-reconciliation fix), and (c) store the snapshot's `seq_id` as
    //     the cursor — which may legitimately be 0. The server returns
    //     `MAX(seq_id) … unwrap_or(0)` and `sync_ops` is NOT backfilled, so a
    //     pre-existing vault (or a brand-new one before its first create-op
    //     lands) snapshots at seq_id 0. seq_id 0 is therefore a VALID
    //     bootstrapped cursor, NOT an "unset" sentinel: gating on `== 0` would
    //     re-snapshot + re-prune the WHOLE tree every tick forever and never
    //     reach the cheap ops path (defeating the 429 fix).
    //
    //   cursor SET (incl. Some(0)) → CATCH-UP via ONE `GET /sync/ops?since={cursor}`:
    //     apply each op by `op_type` and advance the cursor to the max applied
    //     `seq_id`. With `since=0` the server returns every op `> 0`, which is
    //     exactly right for a just-bootstrapped empty/low op log. ~1
    //     request/tick in steady state.
    //
    // Gap / since-too-old fallback: if applying ops can't proceed coherently
    // (the cursor is ahead of the server, or an op references a row we can't
    // place), we re-bootstrap from a fresh snapshot — the snapshot is always
    // authoritative, so a re-bootstrap can never lose data.

    // A pending re-snapshot request (e.g. a `file_restore` op the previous tick
    // couldn't materialise from its `{id}`-only payload) forces a bootstrap this
    // tick regardless of the cursor, then clears the flag.
    let needs_resnapshot = bridge.db().take_needs_resnapshot()?;

    let cursor = match bridge.db().get_sync_cursor()? {
        // Never bootstrapped, or a re-snapshot was explicitly requested → full
        // snapshot. (Some(0) is a real cursor and does NOT bootstrap here.)
        None => {
            bootstrap_from_snapshot(bridge, sync_root, now_secs, &mut conflicts).await?;
            return Ok(conflicts);
        }
        Some(_) if needs_resnapshot => {
            tracing::info!("sync_tick: re-snapshot requested (gap recovery); bootstrapping");
            bootstrap_from_snapshot(bridge, sync_root, now_secs, &mut conflicts).await?;
            return Ok(conflicts);
        }
        Some(c) => c,
    };

    let ops = match bridge.api().sync_ops(cursor).await {
        Ok(ops) => ops,
        Err(e) => {
            // Transient list failure (network / lingering 429 after backoff): do
            // NOT advance the cursor and do NOT fall back to a (heavier)
            // snapshot — just skip this tick and retry next time. The cursor is
            // unchanged, so no op is missed.
            tracing::warn!(error = %e, cursor, "sync_tick: /sync/ops failed; retrying next tick");
            return Ok(conflicts);
        }
    };

    // Detect a cursor that is somehow AHEAD of the server's op log (e.g. a server
    // history reset / op-log truncation): the server clamps `since` to whatever
    // we sent and returns only `seq_id > since`, so a stale/over-large cursor
    // yields an empty op list forever and we'd never reconcile. We can't
    // distinguish "nothing changed" from "cursor too old" from the ops response
    // alone, so we re-anchor against the snapshot's authoritative `seq_id`: only
    // when the ops list is empty do we cheaply confirm the cursor still tracks
    // the server (a single extra call ONLY on the empty-delta path is acceptable;
    // any non-empty delta means the cursor is valid and we skip the check).
    if ops.ops.is_empty() {
        // No new ops. Verify the cursor hasn't drifted past the server head; if
        // it has (server reset its op log), re-bootstrap. The snapshot call here
        // is the single concession — it runs only when there is genuinely
        // nothing to apply, so steady state stays at ~1 request/tick.
        // `now_secs` (captured at tick start) is the prune freshness cutoff: it
        // predates this snapshot fetch, so it can't be later than any row a
        // concurrent completion stamps during the fetch.
        let fetched_at = now_secs;
        match bridge.api().sync_snapshot().await {
            Ok(snap) if snap.seq_id < cursor => {
                tracing::warn!(
                    cursor,
                    server_seq = snap.seq_id,
                    "sync_tick: cursor ahead of server op-log head; re-bootstrapping from snapshot"
                );
                apply_snapshot(bridge, sync_root, &snap, now_secs, fetched_at, &mut conflicts)?;
            }
            Ok(_) => { /* cursor still valid, nothing to do */ }
            Err(e) => {
                tracing::warn!(error = %e, "sync_tick: snapshot freshness probe failed; retrying next tick");
            }
        }
        return Ok(conflicts);
    }

    // Apply the delta ops in order, advancing the cursor to the max seq_id.
    let mut max_seq = cursor;
    for op in &ops.ops {
        if let Err(e) = apply_sync_op(bridge, sync_root, op, now_secs, &mut conflicts) {
            tracing::warn!(error = %e, op_type = %op.op_type, seq_id = op.seq_id, "sync_tick: op apply error; skipping op");
        }
        max_seq = max_seq.max(op.seq_id);
    }
    if max_seq > cursor {
        bridge.db().set_sync_cursor(max_seq)?;
    }
    Ok(conflicts)
}

/// Pull a fresh `/sync/snapshot` and reconcile the whole mirror against it, then
/// store its `seq_id` as the new cursor. The bootstrap path (cursor unset) AND
/// the gap-recovery path both funnel through here so the behaviour is identical.
async fn bootstrap_from_snapshot(
    bridge: &EngineBridge,
    #[cfg_attr(not(target_os = "windows"), allow(unused_variables))] sync_root: &Path,
    now_secs: i64,
    conflicts: &mut Vec<ConflictDetected>,
) -> anyhow::Result<()> {
    // `now_secs` is captured at tick start, BEFORE this request — so it is the
    // prune freshness cutoff that never prunes a row a concurrent local
    // completion stamps while this snapshot is in flight (see `prune_absent`).
    let fetched_at = now_secs;
    let snapshot = bridge.api().sync_snapshot().await?;
    apply_snapshot(bridge, sync_root, &snapshot, now_secs, fetched_at, conflicts)
}

/// Reconcile the local mirror against an already-fetched snapshot:
///   1. order nodes parent-before-child and compute each node's parent rel path,
///   2. ingest every node through `process_metadata_row` (same conflict logic +
///      `upsert_file` placeholder-freshness flow the old BFS used),
///   3. PRUNE every own-tree row the snapshot omits (deletion reconciliation),
///      EXCEPT rows touched at/after `snapshot_fetched_at` (a just-completed
///      upload) and EXCEPT the whole tree when the snapshot is suspiciously empty
///      (both handled inside `prune_absent`),
///   4. store the snapshot's `seq_id` as the cursor.
///
/// `snapshot_fetched_at` is the wall-clock second the snapshot HTTP fetch began;
/// it is the prune freshness cutoff so an upload re-keyed mid-flight survives.
///
/// Split out from `bootstrap_from_snapshot` so it's directly unit-testable with
/// a synthetic `SyncSnapshot` (no HTTP).
fn apply_snapshot(
    bridge: &EngineBridge,
    #[cfg_attr(not(target_os = "windows"), allow(unused_variables))] sync_root: &Path,
    snapshot: &crate::api_client::SyncSnapshot,
    now_secs: i64,
    snapshot_fetched_at: i64,
    conflicts: &mut Vec<ConflictDetected>,
) -> anyhow::Result<()> {
    // Resolve each node's PARENT relative path. The snapshot is a flat node list
    // (each with `parent_id`); the old BFS got nesting for free by listing
    // folder-by-folder. Here we topologically order so a parent's resolved path
    // is known before its children, then thread it into `process_metadata_row`
    // exactly as the BFS threaded the folder's path.
    let order = order_snapshot_nodes(&snapshot.nodes);

    let mut seen: HashSet<String> = HashSet::new();
    // file_id → resolved FULL relative path (so children can prefix their leaf).
    let mut resolved_paths: std::collections::HashMap<String, String> = std::collections::HashMap::new();

    for f in order {
        let file_id = f["id"].as_str().unwrap_or_default();
        if file_id.is_empty() {
            continue;
        }
        if !seen.insert(file_id.to_string()) {
            continue;
        }
        // Parent's resolved path ("" at the vault root, or when the parent
        // didn't resolve — the leaf then sits at root, matching BFS behaviour
        // where an unresolved parent frame simply wasn't descended).
        let parent_rel_path = f["parent_id"]
            .as_str()
            .and_then(|pid| resolved_paths.get(pid))
            .cloned()
            .unwrap_or_default();

        if let Some((rel_path, _kind)) =
            process_metadata_row(bridge, f, &parent_rel_path, now_secs, conflicts)?
        {
            if !rel_path.is_empty() {
                resolved_paths.insert(file_id.to_string(), rel_path);
            }
        }
    }

    // Deletion reconciliation: anything in the local own-tree the snapshot did
    // NOT mention was deleted/trashed server-side. `prune_absent` excludes shared
    // rows, rows with a pending upload/op, rows touched at/after the snapshot
    // fetch (a just-completed upload the snapshot predates), and refuses to prune
    // the whole tree on a suspicious EMPTY snapshot (see its doc) — so neither an
    // in-flight local create nor a just-uploaded file is ever collateral.
    let pruned = bridge.db().prune_absent(&seen, snapshot_fetched_at)?;
    if !pruned.is_empty() {
        tracing::info!(count = pruned.len(), "sync_tick: pruned rows absent from snapshot");
        // task 0806: removing only the DB row left the on-disk Cloud Files
        // placeholder ghosting in Explorer. Now ALSO remove each pruned row's
        // placeholder. `prune_absent` returns rows CHILDREN-BEFORE-PARENTS (orphan
        // descendants of a pruned folder precede the folder), which is exactly the
        // order placeholder removal needs (leaf files before their directory).
        remove_pruned_placeholders(sync_root, &pruned);
    }

    bridge.db().set_sync_cursor(snapshot.seq_id)?;
    Ok(())
}

/// Windows: remove the on-disk Cloud Files placeholder for each row the deletion
/// reconcile just dropped from the DB (task 0806). The DB row is ALREADY gone (the
/// caller deleted it before calling this), so the watcher's `handle_delete` finds
/// no row and queues no server trash; we ALSO register each path in the watcher's
/// engine-delete suppression set so the `NOTIFY_DELETE_COMPLETION` our own remove
/// fires is dropped deterministically rather than echoing into a redundant trash.
///
/// `rows` MUST be ordered children-before-parents (both `prune_absent` and
/// `delete_file_subtree` return that order) so a directory's leaf placeholders are
/// removed before the directory placeholder itself.
///
/// A row whose path can't be safely resolved under the root (empty/traversal) is
/// skipped. Per-row failures are logged (file_id only — zero-knowledge) and never
/// abort the sweep. No-op on macOS/Linux.
#[cfg(target_os = "windows")]
fn remove_pruned_placeholders(sync_root: &Path, rows: &[crate::state_db::PrunedRow]) {
    for row in rows {
        let Some(path) = EngineBridge::placeholder_path_under(sync_root, &row.path) else {
            continue;
        };
        // Register BEFORE the remove so the NOTIFY_DELETE_COMPLETION the remove
        // fires is already suppressed when it lands in the watcher.
        crate::watcher::suppress_engine_delete(&path);
        if let Err(e) = crate::windows_cf::placeholders::delete_placeholder(&path, row.is_dir) {
            // Zero-knowledge: never log the path — only the file_id + kind.
            tracing::warn!(file_id = %row.file_id, is_dir = row.is_dir, error = %e, "remote-deletion reconcile: placeholder removal failed");
        }
    }
}

/// No-op stand-in on non-Windows builds so the call sites stay platform-agnostic.
#[cfg(not(target_os = "windows"))]
fn remove_pruned_placeholders(_sync_root: &Path, _rows: &[crate::state_db::PrunedRow]) {}

/// Order snapshot nodes so every node appears AFTER its parent (parents-first),
/// which lets `apply_snapshot` resolve a parent's path before its children.
///
/// Roots (no `parent_id`, or a `parent_id` not present in the node set — a
/// shared-into / cross-tree parent) come first; remaining nodes are emitted as
/// their parents become available. Any nodes left in a `parent_id` cycle the
/// server should never produce are appended at the end so they're still ingested
/// (just possibly with an empty parent path), never dropped.
fn order_snapshot_nodes(nodes: &[serde_json::Value]) -> Vec<&serde_json::Value> {
    use std::collections::HashMap;
    let present: HashSet<&str> = nodes.iter().filter_map(|n| n["id"].as_str()).collect();
    // children[parent_id] = [node, …]
    let mut children: HashMap<&str, Vec<&serde_json::Value>> = HashMap::new();
    let mut roots: Vec<&serde_json::Value> = Vec::new();
    for n in nodes {
        match n["parent_id"].as_str() {
            Some(pid) if present.contains(pid) => children.entry(pid).or_default().push(n),
            _ => roots.push(n), // root, or parent outside the snapshot set
        }
    }
    let mut ordered: Vec<&serde_json::Value> = Vec::with_capacity(nodes.len());
    let mut queue: VecDeque<&serde_json::Value> = roots.into_iter().collect();
    let mut emitted: HashSet<&str> = HashSet::new();
    while let Some(n) = queue.pop_front() {
        let Some(id) = n["id"].as_str() else { continue };
        if !emitted.insert(id) {
            continue;
        }
        ordered.push(n);
        if let Some(kids) = children.get(id) {
            for k in kids {
                queue.push_back(k);
            }
        }
    }
    // Defensive: emit any node not reached (a parent_id cycle) so nothing is lost.
    for n in nodes {
        if let Some(id) = n["id"].as_str() {
            if emitted.insert(id) {
                ordered.push(n);
            }
        }
    }
    ordered
}

/// Apply ONE `/sync/ops` delta to the local mirror by `op_type`. Mirrors the
/// server's op vocabulary (`server/.../routes/sync.rs` + the `emit_sync_op`
/// call sites in `routes/files.rs` / `routes/uploads.rs`):
///   * `file_create` / `folder_create` → ingest the new row (cloud_only),
///   * `file_update`                   → refresh the existing row's metadata,
///   * `file_rename` / `folder_rename` → update the leaf name (re-derives path),
///   * `file_move`   / `folder_move`   → re-parent the row,
///   * `file_trash`  / `file_delete`   → remove the local mirror row,
///   * `file_restore`                  → request a re-snapshot ({id}-only payload
///     can't rebuild the row; the authoritative snapshot re-materialises it).
///
/// Each op payload is `{ id, … }`. For create/update/rename/move we synthesise a
/// `serde_json::Value` row and feed it through the SAME `process_metadata_row`
/// ingest path the snapshot uses, so conflict detection + placeholder freshness
/// are identical across both paths. Nested paths ARE resolved from the op's
/// `parent_id`/`new_parent_id` against the local mirror (`parent_rel_path_by_id`),
/// matching how `apply_snapshot` threads parent paths — because the Windows
/// placeholder seeder (`populate_placeholders`) derives the on-disk location
/// purely from the row's stored `path`, NOT from `parent_id`, and there is no
/// periodic snapshot to fix a mis-prefixed row later. When a parent isn't locally
/// known yet, the op requests a re-snapshot so nesting converges next tick.
fn apply_sync_op(
    bridge: &EngineBridge,
    #[cfg_attr(not(target_os = "windows"), allow(unused_variables))] sync_root: &Path,
    op: &crate::api_client::SyncOp,
    now_secs: i64,
    conflicts: &mut Vec<ConflictDetected>,
) -> anyhow::Result<()> {
    let payload = &op.payload;
    let id = payload["id"].as_str().unwrap_or_default();
    if id.is_empty() {
        return Ok(());
    }

    match op.op_type.as_str() {
        "file_trash" | "file_delete" => {
            // Reconcile a REMOTE deletion (another client trashed/deleted this).
            // Look up the row first: we need its status (to NOT clobber the local
            // 0802 delete path) and, on Windows, its path/kind to remove the
            // on-disk placeholder — not just the DB row (task 0806).
            match bridge.db().get_file(id)? {
                // 0802 distinction: a `Trashing` row is the LOCAL→server delete in
                // flight (the user already removed the on-disk placeholder; the
                // queued TrashFile op owns this row). This op is the server echo of
                // that very delete — let the 0802 path converge it (the TrashFile op
                // deletes the row on success). Removing the placeholder here would be
                // a no-op (already gone), and we must NOT re-handle it, so skip.
                Some(entry) if entry.status == crate::state_db::FileStatus::Trashing => {}
                Some(_) => {
                    // Remove the row (and, for a folder, its orphaned descendants by
                    // path-prefix — the server trash is NOT recursive, task 0807) in
                    // one transaction, children-before-parent. The DB row is gone
                    // BEFORE we touch the placeholder, so the watcher's handle_delete
                    // can't queue a redundant server trash.
                    let removed = bridge.db().delete_file_subtree(id)?;
                    remove_pruned_placeholders(sync_root, &removed);
                }
                // No row: a delete for a file we never mirrored (or already
                // pruned) — nothing to reconcile.
                None => {}
            }
        }
        "file_restore" => {
            // The restore payload is only `{ id }` (server `routes/files.rs`), so
            // the un-trashed row's full metadata is NOT recoverable from the op.
            // We already DELETED this row on the preceding `file_trash`/
            // `file_delete` op, and there is NO periodic snapshot in steady state
            // (the cursor only advances via ops once bootstrapped) — so a no-op
            // here would make a trash-then-restore on another device leave the
            // file permanently invisible on THIS device. Request a re-bootstrap
            // on the next tick: the authoritative snapshot re-materialises the
            // restored row. Cheap and matches the existing gap-recovery design.
            bridge.db().request_resnapshot()?;
            tracing::info!(
                file_id = %id,
                "sync_tick: file_restore op — scheduling re-snapshot to re-materialise the row"
            );
        }
        "file_rename" | "folder_rename" => {
            // Re-key the leaf name. Build a row carrying the NEW name_encrypted so
            // `resolve_relative_path` re-derives the plaintext leaf. We keep the
            // existing row's parent path by reading its current path's parent.
            let new_name = payload["new_name_encrypted"].as_str();
            let row = synthesize_op_row(bridge, id, op, new_name);
            let parent_rel = existing_parent_rel_path(bridge, id);
            process_metadata_row(bridge, &row, &parent_rel, now_secs, conflicts)?;
        }
        "file_move" | "folder_move" => {
            // Re-parent. The op gives `new_parent_id`; the leaf name is unchanged.
            // The move op carries no name blob, so `synthesize_op_row` supplies the
            // unchanged leaf as a plaintext `"name"` fallback from the existing
            // row's stored path (the `files` table doesn't persist name_encrypted).
            let row = synthesize_op_row(bridge, id, op, None);
            // Resolve the NEW parent's full relative path from the local mirror by
            // `new_parent_id` (the same way `apply_snapshot` resolves nesting), so
            // a move-into-folder lands the row under its folder prefix instead of
            // at the sync root. If the new parent isn't locally known yet, fall
            // back to a re-snapshot rather than mis-placing the row at root.
            let parent_rel = parent_rel_path_by_id(bridge, payload["new_parent_id"].as_str());
            process_metadata_row(bridge, &row, &parent_rel, now_secs, conflicts)?;
        }
        "file_create" | "folder_create" | "file_update" => {
            let row = synthesize_op_row(bridge, id, op, payload["name_encrypted"].as_str());
            // For a CREATE there is no existing local row, so the parent path must
            // come from the op's `parent_id` resolved against the local mirror —
            // NOT from the (nonexistent) row's own path, which would yield "" and
            // mis-place a nested create at the sync root. `populate_placeholders`
            // derives the on-disk location purely from the row's `path`, so the
            // path prefix MUST be correct here; there is no periodic snapshot to
            // fix it later. For an UPDATE the row already exists, so keep its
            // current parent prefix (`existing_parent_rel_path`).
            let parent_rel = if op.op_type == "file_update" {
                existing_parent_rel_path(bridge, id)
            } else {
                parent_rel_path_by_id(bridge, payload["parent_id"].as_str())
            };
            process_metadata_row(bridge, &row, &parent_rel, now_secs, conflicts)?;
        }
        other => {
            tracing::debug!(op_type = other, "sync_tick: ignoring unknown op_type");
        }
    }
    Ok(())
}

/// Build a `/files`-shaped `serde_json::Value` from a sync op + the existing
/// local row, so `process_metadata_row` ingests it identically to a snapshot
/// node. Carries the op's `id` plus any of `name_encrypted` / `parent_id` /
/// `size_bytes` / `version_number` / `is_folder` it can determine, falling back
/// to the existing row's values where the op is partial (e.g. a `file_update`
/// that only changed `has_thumbnail`).
fn synthesize_op_row(
    bridge: &EngineBridge,
    id: &str,
    op: &crate::api_client::SyncOp,
    name_encrypted: Option<&str>,
) -> serde_json::Value {
    let payload = &op.payload;
    let existing = bridge.db().get_file(id).ok().flatten();
    let existing_contract = bridge.db().get_file_contract_state(id).ok().flatten();

    let is_folder = op.op_type.starts_with("folder_")
        || existing
            .as_ref()
            .map(|e| e.item_kind == ItemKind::Folder)
            .unwrap_or(false);

    let mut row = serde_json::json!({ "id": id, "is_folder": is_folder });

    // name_encrypted: op-provided wins (create/rename carry it). A MOVE op carries
    // NO name blob, and the `files` table does NOT persist `name_encrypted`, so it
    // is genuinely unrecoverable from the op for a move. To still compose the
    // correct path, fall back to the EXISTING row's plaintext leaf (the last
    // segment of its already-decrypted stored `path`) as the `"name"` field —
    // `resolve_relative_path` prefers `name_encrypted` but falls through to
    // `"name"`, so the moved row keeps its leaf under the new parent prefix
    // instead of resolving to an empty path (which would mis-seed at the root or
    // skip the row entirely).
    if let Some(name) = name_encrypted.or_else(|| payload["name_encrypted"].as_str()) {
        row["name_encrypted"] = serde_json::json!(name);
    } else if let Some(leaf) = existing.as_ref().and_then(|e| {
        e.path
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .map(str::to_string)
            .filter(|l| !l.is_empty())
    }) {
        row["name"] = serde_json::json!(leaf);
    }
    // parent_id: a move sets new_parent_id; create/update set parent_id; else keep
    // the existing parent.
    let parent_id = payload["new_parent_id"]
        .as_str()
        .or_else(|| payload["parent_id"].as_str())
        .map(str::to_string)
        .or_else(|| existing.as_ref().and_then(|e| e.parent_id.clone()));
    if let Some(pid) = parent_id {
        row["parent_id"] = serde_json::json!(pid);
    }
    // size_bytes: op-provided wins, else existing.
    let size = payload["size_bytes"]
        .as_i64()
        .or_else(|| existing.as_ref().map(|e| e.size_bytes));
    if let Some(sz) = size {
        row["size_bytes"] = serde_json::json!(sz);
    }
    // version_number: op-provided wins, else the existing contract's.
    let version = payload["version_number"]
        .as_i64()
        .or_else(|| existing_contract.as_ref().map(|c| c.current_version));
    if let Some(v) = version {
        row["version_number"] = serde_json::json!(v);
    }
    // updated_at: stamp "now" so the conflict check treats an op as a fresh
    // remote change (the op log carries no file timestamp the client can read).
    row["updated_at"] = serde_json::json!(now_secs());
    row
}

/// Resolve a parent's FULL relative path from the local mirror by its `file_id`
/// — the parent path a freshly-created/moved child must be prefixed with. This
/// mirrors how [`apply_snapshot`] resolves nesting (it threads each parent's
/// resolved full path into its children), so an op-driven create/move lands at
/// the SAME path a fresh snapshot would produce.
///
///   * `parent_id == None`            → "" (root child),
///   * parent row present locally     → the parent's stored full `path` (its
///     leading slash trimmed so it composes cleanly as a prefix),
///   * `parent_id == Some` but the parent row is NOT in the local mirror yet
///     (the create/move arrived before its parent's create op landed) → "" for
///     THIS tick AND a re-snapshot is requested, so the authoritative snapshot
///     re-places the row under its correct prefix on the next tick rather than
///     leaving it permanently mis-placed at the sync root.
fn parent_rel_path_by_id(bridge: &EngineBridge, parent_id: Option<&str>) -> String {
    let Some(pid) = parent_id.filter(|p| !p.is_empty()) else {
        return String::new(); // root child — no prefix
    };
    match bridge.db().get_file(pid).ok().flatten() {
        Some(parent) => parent.path.trim_start_matches('/').to_string(),
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
            String::new()
        }
    }
}

/// The PARENT relative path of an existing local row (everything before the last
/// `/` in its stored `path`), or "" when the row is unknown / at the vault root.
/// Used so an op-driven re-ingest keeps a nested row under its folder prefix
/// without needing the parent's plaintext name in the op.
fn existing_parent_rel_path(bridge: &EngineBridge, id: &str) -> String {
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

/// Apply one server metadata row during [`sync_tick`]'s recursive walk,
/// running the same three-way decision the flat sweep used (new → cloud_only;
/// local + remote-moved → conflict check; otherwise refresh metadata). On
/// success returns `Some((resolved_rel_path, item_kind))` so the BFS can
/// recurse into folders using the row's full nested path; returns `None` only
/// when the row is unusable (empty id / undecryptable name).
fn process_metadata_row(
    bridge: &EngineBridge,
    f: &serde_json::Value,
    parent_rel_path: &str,
    now_secs: i64,
    conflicts: &mut Vec<ConflictDetected>,
) -> anyhow::Result<Option<(String, ItemKind)>> {
    let file_id = f["id"].as_str().unwrap_or_default();
    if file_id.is_empty() {
        return Ok(None);
    }
    let size = f["size_bytes"].as_i64().unwrap_or(0);
    let remote_updated = f["updated_at"].as_i64().unwrap_or(0);

    // Helper to refresh metadata + return the resolved path/kind. The single
    // place that writes the row's nested path and folder/file classification.
    let apply = |bridge: &EngineBridge| -> anyhow::Result<Option<(String, ItemKind)>> {
        let entry = apply_metadata_file_row(
            bridge.db(),
            f,
            Namespace::MyFiles,
            None,
            None,
            PERMISSION_READ | PERMISSION_WRITE | PERMISSION_OWNER,
            now_secs,
            bridge.api().master_key(),
            parent_rel_path,
        )?;
        Ok(entry.map(|e| (e.path, e.item_kind)))
    };

    match bridge.db().get_file(file_id)? {
        // (0) Locally-deleted, server-trash pending (task 0802). The row is in
        // `Trashing` because the user deleted the file and the `TrashFile` op has
        // already succeeded (op removed) OR is still in flight. A `/sync/snapshot`
        // that STILL lists this file is just propagation lag — the server has set
        // `is_trashed=TRUE` but the snapshot read hasn't caught up. We MUST NOT
        // re-materialise the row: neither re-insert it nor flip it back to
        // `CloudOnly`, which would re-mint the placeholder (the "deleted file comes
        // back" race). Keep it EXACTLY as-is (`Trashing`, no placeholder). When the
        // trash finally propagates the file drops out of the snapshot and
        // `prune_absent` removes the row; if the `file_trash` op echo arrives first
        // `apply_sync_op` deletes it. Either way it converges WITHOUT resurrecting.
        Some(entry) if entry.status == FileStatus::Trashing => {
            // Still report path/kind so a (rare) trashing folder's independent
            // children continue to enumerate; do NOT touch this row's status.
            Ok(Some((entry.path, entry.item_kind)))
        }
        None => {
            // (1) New to us — insert as cloud_only. base = remote.
            apply(bridge)
        }
        Some(entry) if entry.status == FileStatus::Local => {
            // (2) Local copy + remote moved? Nothing to check if the
            // timestamps haven't drifted past base. Still report the row's
            // path/kind so a folder we already have locally is still descended.
            if remote_updated <= entry.remote_updated_at {
                return Ok(Some((entry.path, entry.item_kind)));
            }

            // Synthesise version triplet (no server content hash on metadata;
            // `size-mtime` is an opaque-but-stable surrogate — conservative,
            // a same-size+mtime edit won't flag, but those collisions are rare).
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
            // Local matches base → only remote changed. Quietly re-anchor and
            // let a future hydrate replace bytes when the user opens it.
            if !is_conflict(&local, &remote, &base) {
                let mut updated = entry.clone();
                updated.remote_updated_at = remote_updated;
                updated.size_bytes = size;
                updated.modified_at = remote_updated;
                bridge.db().upsert_file(&updated)?;
                return apply(bridge);
            }

            // True conflict. Flip status, anchor `modified_at` to "now" so the
            // auto-resolution clock starts from detection.
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
            // A conflicted folder is unusual, but still report its path/kind so
            // enumeration of its (independent) children continues.
            Ok(Some((entry.path, entry.item_kind)))
        }
        Some(entry) => {
            // (3) Pending download/upload/conflict/error — let the dedicated
            // path own its transitions, but still refresh cloud_only/downloading
            // metadata (and always report path/kind so folders are descended).
            if matches!(entry.status, FileStatus::CloudOnly | FileStatus::Downloading) {
                return apply(bridge);
            }
            Ok(Some((entry.path, entry.item_kind)))
        }
    }
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

    // ── Shared upload-trigger helpers (task 0780) ────────────────────────────
    // Relocated from the retired `watcher`-local copies so the one shared
    // engine-internal + relative-path logic stays covered.

    #[test]
    fn engine_internal_filters_state_dir_and_lock() {
        let root = std::path::PathBuf::from("/sync");
        assert!(path_is_engine_internal(&root, &root.join(".beebeeb").join("state.db")));
        assert!(path_is_engine_internal(&root, &root.join(".beebeeb-sync.lock")));
        assert!(path_is_engine_internal(&root, &root.join("sub").join(".beebeeb").join("x")));
        assert!(!path_is_engine_internal(&root, &root.join("photo.jpg")));
        assert!(!path_is_engine_internal(&root, &root.join("docs").join("a.txt")));
    }

    #[test]
    fn relative_db_path_is_slash_joined_without_leading_slash() {
        let root = std::path::PathBuf::from("/sync");
        assert_eq!(relative_db_path(&root, &root.join("a.txt")).as_deref(), Some("a.txt"));
        assert_eq!(
            relative_db_path(&root, &root.join("docs").join("b.md")).as_deref(),
            Some("docs/b.md")
        );
        assert_eq!(
            relative_db_path(&root, &std::path::PathBuf::from("/other/c.txt")),
            None
        );
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
                rel_path: None,
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
                rel_path: None,
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

    #[tokio::test]
    async fn test_process_due_operations_trash_calls_api_and_keeps_row_trashing() {
        // task 0802 (server-authoritative deletion): a locally-deleted file is
        // parked in `Trashing` and its `TrashFile` op enqueued. Processing that op
        // must (a) issue the server trash DELETE and (b) on success DROP THE OP but
        // KEEP the row in `Trashing`. The `Trashing` status — not the op — is now
        // the durable hidden-locally marker; convergence (row removal) happens
        // authoritatively once the trash propagates out of `/sync/snapshot`
        // (`prune_absent`) or the `file_trash` op echo arrives (`apply_sync_op`).
        // Deleting the row HERE is what caused the "deleted file comes back" race
        // (a same-tick lagged snapshot would re-insert it as CloudOnly).
        let dir = tempfile::tempdir().unwrap();
        let server = SyncMockServer::start(vec![("200 OK".into(), serde_json::json!({ "ok": true }))]);
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), [9u8; 32]);

        // Row in the post-local-delete `Trashing` state + a queued trash op.
        bridge
            .db
            .upsert_file(&FileEntry {
                file_id: "server-file-1".into(),
                path: "doomed.txt".into(),
                status: FileStatus::Trashing,
                size_bytes: 5,
                modified_at: 0,
                content_hash: None,
                remote_updated_at: 0,
                parent_id: None,
                item_kind: ItemKind::File,
            })
            .unwrap();
        bridge
            .db
            .enqueue_operation(&PendingOperation {
                op_id: "op-trash-1".into(),
                kind: OperationKind::TrashFile,
                file_id: Some("server-file-1".into()),
                parent_id: None,
                target_path: None,
                metadata_json: Some(serde_json::json!({ "operation": "trash" }).to_string()),
                payload_path: None,
                base_version: None,
                base_object_version_id: None,
                attempts: 0,
                max_attempts: 25,
                next_retry_at: 0,
                last_error: None,
                created_at: 100,
                updated_at: 100,
            })
            .unwrap();

        let outcome = bridge.process_due_operations(dir.path(), 200).await.unwrap();
        assert_eq!(outcome.completed_op_ids, vec!["op-trash-1".to_string()]);
        assert!(outcome.retried_op_ids.is_empty());

        // The op is gone (success removes it) but the row REMAINS in `Trashing` —
        // the durable marker. The seeder only re-mints `CloudOnly`, so a `Trashing`
        // row never re-creates the placeholder; the row itself is removed later by
        // prune/op-echo once the server trash propagates.
        assert!(bridge.db.list_due_operations(999).unwrap().is_empty());
        let row = bridge.db.get_file("server-file-1").unwrap();
        assert!(row.is_some(), "trash op must KEEP the row on success (not delete it)");
        assert_eq!(
            row.unwrap().status,
            FileStatus::Trashing,
            "row stays Trashing after a successful trash — the durable hidden-locally marker"
        );

        let requests = server.finish();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method, "DELETE");
        assert_eq!(requests[0].path, "/api/v1/files/server-file-1");
    }

    #[tokio::test]
    async fn test_process_due_operations_trash_retries_and_keeps_row_on_server_error() {
        // If the server trash fails, the op must be retried (not dropped) and the
        // row must be PRESERVED — recoverable rather than lost. (The row is left
        // in `Trashing`, so the placeholder still does not reappear meanwhile.)
        let dir = tempfile::tempdir().unwrap();
        let server = SyncMockServer::start(vec![(
            "500 Internal Server Error".into(),
            serde_json::json!({ "error": "boom" }),
        )]);
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), [9u8; 32]);

        bridge
            .db
            .upsert_file(&FileEntry {
                file_id: "server-file-2".into(),
                path: "doomed2.txt".into(),
                status: FileStatus::Trashing,
                size_bytes: 5,
                modified_at: 0,
                content_hash: None,
                remote_updated_at: 0,
                parent_id: None,
                item_kind: ItemKind::File,
            })
            .unwrap();
        bridge
            .db
            .enqueue_operation(&PendingOperation {
                op_id: "op-trash-2".into(),
                kind: OperationKind::TrashFile,
                file_id: Some("server-file-2".into()),
                parent_id: None,
                target_path: None,
                metadata_json: Some(serde_json::json!({ "operation": "trash" }).to_string()),
                payload_path: None,
                base_version: None,
                base_object_version_id: None,
                attempts: 0,
                max_attempts: 25,
                next_retry_at: 0,
                last_error: None,
                created_at: 100,
                updated_at: 100,
            })
            .unwrap();

        let outcome = bridge.process_due_operations(dir.path(), 200).await.unwrap();
        assert!(outcome.completed_op_ids.is_empty());
        assert_eq!(outcome.retried_op_ids, vec!["op-trash-2".to_string()]);
        // Row preserved (still Trashing) so a future retry can complete and the
        // file is recoverable from the server in the meantime.
        let row = bridge.db.get_file("server-file-2").unwrap().unwrap();
        assert_eq!(row.status, FileStatus::Trashing);

        let _ = server.finish();
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

        // `sync_root` is only used on Windows (to resolve placeholder paths for
        // CfSetPinState). On the non-Windows test host the hydrate-enqueue path
        // runs and `sync_root` is unused, so any path is fine here.
        let queued = bridge.set_recursive_pin(dir.path(), "folder-a", true).unwrap();
        assert_eq!(queued.changed_item_ids.len(), 3);
        // hydrate_operations is the non-Windows materialise path; on Windows the
        // OS (CfSetPinState) hydrates pinned files, so this count is 0 there.
        #[cfg(not(target_os = "windows"))]
        assert_eq!(queued.hydrate_operations, 1);

        let due = bridge.db.list_due_operations(now_secs()).unwrap();
        assert!(due.iter().any(|op| {
            op.kind == OperationKind::PinTree
                && op.file_id.as_deref() == Some("folder-a")
                && op.metadata_json.as_deref().unwrap_or("").contains("\"pinned\":true")
        }));
        // On Windows the pin path calls CfSetPinState; the OS drives hydration
        // via FETCH_DATA rather than us enqueuing HydrateFile ops, so no
        // HydrateFile entries exist in the queue on that platform.
        #[cfg(not(target_os = "windows"))]
        {
            assert!(
                due.iter()
                    .any(|op| { op.kind == OperationKind::HydrateFile && op.file_id.as_deref() == Some("file-a") })
            );
            assert!(
                !due.iter()
                    .any(|op| { op.kind == OperationKind::HydrateFile && op.file_id.as_deref() == Some("file-local") })
            );
        }
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
            "",
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
            "",
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
            "",
        )
        .unwrap()
        .unwrap();

        // No plaintext field present → empty path (caller skips seeding),
        // but the row still upserts so the rest of the sweep proceeds.
        assert_eq!(entry.path, "");
    }

    #[test]
    fn test_metadata_file_row_composes_nested_path_under_parent() {
        // NESTED enumeration: a child discovered under a folder must be stored
        // at `<parent_rel_path>/<decrypted_leaf>` (slash-joined, no leading
        // slash) so the path round-trips with the upload watcher's
        // `relative_db_path` and the windows_cf placeholder seeder.
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();

        let master_key = [3u8; 32];
        let file_id = "aaaaaaaa-0000-4000-8000-000000000003";
        let mk = beebeeb_core::kdf::MasterKey::from_bytes(master_key);
        let name_encrypted =
            beebeeb_core::encrypt::encrypt_name(&mk, file_id, "notes.txt", Some("text/plain")).unwrap();

        let metadata = serde_json::json!({
            "id": file_id,
            "name_encrypted": name_encrypted,
            "size_bytes": 12,
            "updated_at": 42,
            "is_folder": false,
            "parent_id": "folder-docs",
        });

        // Parent folder resolved to "docs" earlier in the BFS walk.
        let entry = apply_metadata_file_row(
            &db,
            &metadata,
            Namespace::MyFiles,
            None,
            None,
            PERMISSION_READ | PERMISSION_WRITE | PERMISSION_OWNER,
            100,
            &master_key,
            "docs",
        )
        .unwrap()
        .unwrap();

        assert_eq!(entry.path, "docs/notes.txt");
        assert_eq!(entry.item_kind, ItemKind::File);
        assert_eq!(entry.parent_id.as_deref(), Some("folder-docs"));
        // The stored row (what list_by_status / the seeder reads) agrees, and
        // the contract carries the authoritative parent_id + item_kind.
        let stored = db.get_file(file_id).unwrap().unwrap();
        assert_eq!(stored.path, "docs/notes.txt");
        assert_eq!(stored.item_kind, ItemKind::File);
        assert_eq!(stored.parent_id.as_deref(), Some("folder-docs"));
        let contract = db.get_file_contract_state(file_id).unwrap().unwrap();
        assert_eq!(contract.parent_id.as_deref(), Some("folder-docs"));
        assert_eq!(contract.item_kind, ItemKind::File);
    }

    #[test]
    fn test_metadata_file_row_classifies_folder_rows() {
        // A row the server marks as a folder must surface item_kind == Folder
        // both on the returned FileEntry and the stored row, so the BFS walk
        // descends into it and the placeholder seeder mints a DIRECTORY.
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();

        let master_key = [5u8; 32];
        let file_id = "bbbbbbbb-0000-4000-8000-000000000004";
        let mk = beebeeb_core::kdf::MasterKey::from_bytes(master_key);
        let name_encrypted = beebeeb_core::encrypt::encrypt_name(&mk, file_id, "Photos", None).unwrap();

        let metadata = serde_json::json!({
            "id": file_id,
            "name_encrypted": name_encrypted,
            "size_bytes": 0,
            "updated_at": 7,
            "is_folder": true,
        });

        let entry = apply_metadata_file_row(
            &db,
            &metadata,
            Namespace::MyFiles,
            None,
            None,
            PERMISSION_READ | PERMISSION_WRITE | PERMISSION_OWNER,
            100,
            &master_key,
            "",
        )
        .unwrap()
        .unwrap();

        assert_eq!(entry.path, "Photos");
        assert_eq!(entry.item_kind, ItemKind::Folder);
        assert!(entry.is_dir());
        let stored = db.get_file(file_id).unwrap().unwrap();
        assert_eq!(stored.item_kind, ItemKind::Folder);
        assert!(stored.is_dir());
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
                rel_path: None,
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
                parent_id: None,
                item_kind: ItemKind::File,
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
                parent_id: parent_id.map(str::to_string),
                item_kind: ItemKind::File,
            })
            .unwrap();
        let mut contract = bridge.db.get_file_contract_state(file_id).unwrap().unwrap();
        contract.parent_id = parent_id.map(str::to_string);
        contract.cache_bytes = cache_bytes;
        bridge.db.set_file_contract_state(&contract).unwrap();
    }

    // ── /sync delta engine tests (task 0789) ──────────────────────────────────
    //
    // A scriptable mock that answers `/api/v1/sync/snapshot` and
    // `/api/v1/sync/ops` from a FIFO of canned responses (status + JSON body)
    // and records every request path. Mirrors `UploadMockServer`'s raw-TCP shape
    // (the existing test harness) so it shares the request-capture pattern the
    // 0789 plan calls out.

    struct SyncMockServer {
        base_url: String,
        requests: Arc<Mutex<Vec<RecordedRequest>>>,
        handle: thread::JoinHandle<()>,
    }

    impl SyncMockServer {
        /// `responses`: ordered `(http_status, json_body)` answered one per
        /// incoming request. The server serves exactly `responses.len()`
        /// requests then exits, so the test must drive precisely that many.
        fn start(responses: Vec<(String, serde_json::Value)>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let base_url = format!("http://{}", listener.local_addr().unwrap());
            let requests = Arc::new(Mutex::new(Vec::new()));
            let server_requests = Arc::clone(&requests);
            let handle = thread::spawn(move || {
                for (status, body) in responses {
                    let (mut stream, _) = listener.accept().unwrap();
                    let request = read_http_request(&mut stream);
                    server_requests.lock().unwrap().push(request);
                    let response = http_json(&status, body);
                    stream.write_all(response.as_bytes()).unwrap();
                }
            });
            Self { base_url, requests, handle }
        }

        fn finish(self) -> Vec<RecordedRequest> {
            self.handle.join().unwrap();
            Arc::try_unwrap(self.requests).unwrap().into_inner().unwrap()
        }
    }

    fn enc_name(master_key: &[u8; 32], file_id: &str, name: &str) -> String {
        let mk = beebeeb_core::kdf::MasterKey::from_bytes(*master_key);
        beebeeb_core::encrypt::encrypt_name(&mk, file_id, name, None).unwrap()
    }

    /// Build a snapshot node the way the server's `/sync/snapshot` does.
    fn snap_node(
        master_key: &[u8; 32],
        id: &str,
        name: &str,
        parent_id: Option<&str>,
        is_folder: bool,
        updated_at: i64,
    ) -> serde_json::Value {
        serde_json::json!({
            "id": id,
            "parent_id": parent_id,
            "name_encrypted": enc_name(master_key, id, name),
            "size_bytes": 10,
            "is_folder": is_folder,
            "content_hash": serde_json::Value::Null,
            "version_number": 1,
            "is_trashed": false,
            "is_starred": false,
            "updated_at": updated_at,
        })
    }

    #[tokio::test]
    async fn test_sync_tick_bootstrap_snapshot_ingests_same_rows_as_walk() {
        // cursor unset → ONE /sync/snapshot bootstrap. Every node lands as a
        // cloud_only row with the decrypted nested path, and the cursor is set
        // to the snapshot's seq_id.
        let dir = tempfile::tempdir().unwrap();
        let mk = [7u8; 32];
        let folder = "f0000000-0000-4000-8000-000000000001";
        let child = "c0000000-0000-4000-8000-000000000002";
        let root_file = "a0000000-0000-4000-8000-000000000003";
        let snapshot = serde_json::json!({
            "seq_id": 12,
            "nodes": [
                snap_node(&mk, folder, "docs", None, true, 100),
                snap_node(&mk, child, "notes.txt", Some(folder), false, 100),
                snap_node(&mk, root_file, "top.txt", None, false, 100),
            ],
        });
        let server = SyncMockServer::start(vec![("200 OK".into(), snapshot)]);
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), mk);

        let conflicts = sync_tick(&bridge, dir.path()).await.unwrap();
        assert!(conflicts.is_empty());

        let requests = server.finish();
        assert_eq!(requests.len(), 1, "bootstrap = exactly one request");
        assert_eq!(requests[0].path, "/api/v1/sync/snapshot");

        // Nested + root rows all present, paths nested correctly, all cloud_only.
        let f = bridge.db().get_file(folder).unwrap().unwrap();
        assert_eq!(f.path, "docs");
        assert_eq!(f.item_kind, ItemKind::Folder);
        let c = bridge.db().get_file(child).unwrap().unwrap();
        assert_eq!(c.path, "docs/notes.txt", "child nested under parent's resolved path");
        assert_eq!(c.status, FileStatus::CloudOnly);
        let r = bridge.db().get_file(root_file).unwrap().unwrap();
        assert_eq!(r.path, "top.txt");
        // Cursor advanced to the snapshot seq_id.
        assert_eq!(bridge.db().get_sync_cursor().unwrap(), Some(12));
    }

    #[tokio::test]
    async fn test_sync_tick_snapshot_prunes_row_absent_from_snapshot() {
        // A known local row the fresh snapshot OMITS must be pruned (the silent
        // deletion-reconciliation fix).
        let dir = tempfile::tempdir().unwrap();
        let mk = [4u8; 32];
        let kept = "11111111-0000-4000-8000-000000000001";
        let stale = "22222222-0000-4000-8000-000000000002";
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), "http://placeholder".into(), mk);
        // Pre-seed a settled own-tree row that the snapshot will NOT mention.
        bridge.db().upsert_file(&FileEntry {
            file_id: stale.into(),
            path: "deleted-server-side.txt".into(),
            status: FileStatus::CloudOnly,
            size_bytes: 1,
            modified_at: 0,
            content_hash: None,
            remote_updated_at: 0,
            parent_id: None,
            item_kind: ItemKind::File,
        }).unwrap();

        let snapshot = serde_json::json!({
            "seq_id": 5,
            "nodes": [ snap_node(&mk, kept, "kept.txt", None, false, 50) ],
        });
        let server = SyncMockServer::start(vec![("200 OK".into(), snapshot)]);
        // Re-point the bridge at the mock by rebuilding it on the same DB path.
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), mk);

        sync_tick(&bridge, dir.path()).await.unwrap();
        server.finish();

        assert!(bridge.db().get_file(kept).unwrap().is_some());
        assert!(bridge.db().get_file(stale).unwrap().is_none(), "snapshot-absent row pruned");
    }

    #[tokio::test]
    async fn test_sync_tick_snapshot_window_does_not_resurrect_trashing_row() {
        // task 0802 — THE WINDOW (the bug this fix closes). A file was just
        // locally deleted: its row is `Trashing`, its `TrashFile` op already
        // SUCCEEDED (so the op is gone). On the very next tick the server's
        // `/sync/snapshot` read is replica-lagged / same-second and STILL lists
        // the (now-trashed) file. The OLD code's None/insert arm re-inserted a
        // FRESH `CloudOnly` row here and re-minted the placeholder — the "deleted
        // file comes back" bug. The fix: `process_metadata_row`'s `Trashing` guard
        // preserves the row untouched, and `prune_absent` does NOT remove it while
        // it's still listed. Assert: row stays `Trashing`, never `CloudOnly`.
        let dir = tempfile::tempdir().unwrap();
        let mk = [7u8; 32];
        let trashing = "33333333-0000-4000-8000-000000000003";
        // First bridge only to create the DB file; rebuilt below against the mock.
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), "http://placeholder".into(), mk);
        // Pre-seed the post-local-delete state: Trashing row, NO pending op.
        bridge
            .db()
            .upsert_file(&FileEntry {
                file_id: trashing.into(),
                path: "doomed.txt".into(),
                status: FileStatus::Trashing,
                size_bytes: 10,
                modified_at: 0,
                content_hash: None,
                remote_updated_at: 10,
                parent_id: None,
                item_kind: ItemKind::File,
            })
            .unwrap();

        // Snapshot STILL lists the trashed file (propagation lag) — exactly the
        // race window. (is_trashed flag on the node is irrelevant: the lagged
        // snapshot is what the engine sees.)
        let snapshot = serde_json::json!({
            "seq_id": 9,
            "nodes": [ snap_node(&mk, trashing, "doomed.txt", None, false, 10) ],
        });
        let server = SyncMockServer::start(vec![("200 OK".into(), snapshot)]);
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), mk);

        sync_tick(&bridge, dir.path()).await.unwrap();
        server.finish();

        let row = bridge.db().get_file(trashing).unwrap();
        assert!(row.is_some(), "Trashing row must NOT be pruned while still listed in the snapshot");
        assert_eq!(
            row.unwrap().status,
            FileStatus::Trashing,
            "a snapshot still listing a Trashing file must NOT resurrect it to CloudOnly (the bug)"
        );
    }

    #[tokio::test]
    async fn test_sync_tick_snapshot_absent_prunes_trashing_row_after_propagation() {
        // task 0802 — CONVERGENCE end-to-end: once the trash propagates the file
        // is ABSENT from the snapshot, and the op-less `Trashing` row is pruned —
        // final convergence with the (already-removed) on-disk placeholder.
        let dir = tempfile::tempdir().unwrap();
        let mk = [7u8; 32];
        let trashing = "44444444-0000-4000-8000-000000000004";
        let kept = "55555555-0000-4000-8000-000000000005";
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), "http://placeholder".into(), mk);
        bridge
            .db()
            .upsert_file(&FileEntry {
                file_id: trashing.into(),
                path: "doomed.txt".into(),
                status: FileStatus::Trashing,
                size_bytes: 10,
                modified_at: 0,
                content_hash: None,
                remote_updated_at: 10,
                parent_id: None,
                item_kind: ItemKind::File,
            })
            .unwrap();

        // Snapshot no longer lists the trashed file (propagation complete); it
        // lists an unrelated row so the empty-snapshot guard doesn't trip.
        let snapshot = serde_json::json!({
            "seq_id": 9,
            "nodes": [ snap_node(&mk, kept, "kept.txt", None, false, 10) ],
        });
        let server = SyncMockServer::start(vec![("200 OK".into(), snapshot)]);
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), mk);

        sync_tick(&bridge, dir.path()).await.unwrap();
        server.finish();

        assert!(bridge.db().get_file(kept).unwrap().is_some());
        assert!(
            bridge.db().get_file(trashing).unwrap().is_none(),
            "an op-less Trashing row absent from the snapshot must be pruned (convergence)"
        );
    }

    #[tokio::test]
    async fn test_sync_tick_ops_sequence_converges_to_fresh_snapshot() {
        // Bootstrap, then apply a stream of ops, and assert the resulting mirror
        // matches what a fresh snapshot of the post-op tree would produce.
        let dir = tempfile::tempdir().unwrap();
        let mk = [9u8; 32];
        let keep = "aaaa0000-0000-4000-8000-000000000001";
        let doomed = "bbbb0000-0000-4000-8000-000000000002";
        let created = "cccc0000-0000-4000-8000-000000000003";

        // Bootstrap snapshot: {keep, doomed}, seq 1.
        let boot = serde_json::json!({
            "seq_id": 1,
            "nodes": [
                snap_node(&mk, keep, "keep.txt", None, false, 10),
                snap_node(&mk, doomed, "doomed.txt", None, false, 10),
            ],
        });
        // Ops: create `created`, trash `doomed`. Highest seq = 3.
        let ops = serde_json::json!({
            "since": 1,
            "ops": [
                { "seq_id": 2, "op_type": "file_create",
                  "payload": { "id": created, "name_encrypted": enc_name(&mk, created, "fresh.txt"),
                               "parent_id": serde_json::Value::Null, "size_bytes": 7,
                               "storage_pool_id": "pool" } },
                { "seq_id": 3, "op_type": "file_trash", "payload": { "id": doomed } },
            ],
        });

        let server = SyncMockServer::start(vec![
            ("200 OK".into(), boot),
            ("200 OK".into(), ops),
        ]);
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), mk);

        // Tick 1: bootstrap (cursor 0 → snapshot). Tick 2: ops?since=1.
        sync_tick(&bridge, dir.path()).await.unwrap();
        assert_eq!(bridge.db().get_sync_cursor().unwrap(), Some(1));
        sync_tick(&bridge, dir.path()).await.unwrap();

        let requests = server.finish();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].path, "/api/v1/sync/snapshot");
        assert_eq!(requests[1].path, "/api/v1/sync/ops?since=1");

        // Convergence: keep + created present, doomed gone, cursor at 3 — exactly
        // a fresh snapshot of the post-op tree.
        assert!(bridge.db().get_file(keep).unwrap().is_some());
        assert!(bridge.db().get_file(created).unwrap().is_some(), "file_create applied");
        assert_eq!(bridge.db().get_file(created).unwrap().unwrap().path, "fresh.txt");
        assert!(bridge.db().get_file(doomed).unwrap().is_none(), "file_trash removed the row");
        assert_eq!(bridge.db().get_sync_cursor().unwrap(), Some(3), "cursor advanced to max seq_id");
    }

    #[tokio::test]
    async fn test_sync_tick_ops_429_backs_off_and_preserves_cursor() {
        // A 429 on /sync/ops must NOT advance the cursor and must NOT lose data —
        // send_with_retry rides it out; if it ultimately surfaces, the tick
        // simply retries next time. Here the mock 429s every attempt; sync_tick
        // must return Ok with the cursor unchanged.
        let dir = tempfile::tempdir().unwrap();
        let mk = [3u8; 32];
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), "http://placeholder".into(), mk);
        // Pre-set a cursor so we take the ops path (not bootstrap).
        bridge.db().set_sync_cursor(8).unwrap();

        // send_with_retry does up to 1 + MAX_429_RETRIES (=3) attempts → 4 total.
        let err_body = serde_json::json!({ "error": "rate limit exceeded", "retry_after": 0 });
        let responses = vec![
            ("429 Too Many Requests".into(), err_body.clone()),
            ("429 Too Many Requests".into(), err_body.clone()),
            ("429 Too Many Requests".into(), err_body.clone()),
            ("429 Too Many Requests".into(), err_body),
        ];
        let server = SyncMockServer::start(responses);
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), mk);

        // Must not panic / error out the runner; cursor stays put for a retry.
        let conflicts = sync_tick(&bridge, dir.path()).await.unwrap();
        assert!(conflicts.is_empty());

        let requests = server.finish();
        assert_eq!(requests.len(), 4, "one initial + 3 retries (429 backoff)");
        for r in &requests {
            assert_eq!(r.path, "/api/v1/sync/ops?since=8");
        }
        assert_eq!(bridge.db().get_sync_cursor().unwrap(), Some(8), "cursor preserved across 429");
    }

    #[tokio::test]
    async fn test_sync_tick_seq_id_zero_bootstrap_then_takes_ops_path_not_resnapshot() {
        // BLOCKER regression (issue 1): the server returns seq_id 0 for any vault
        // whose `sync_ops` log is empty (MAX(seq_id) → NULL → unwrap_or(0)). seq_id
        // 0 is a VALID bootstrapped cursor, NOT an "unset" sentinel. The 2nd tick
        // MUST issue /sync/ops?since=0 and MUST NOT re-snapshot.
        let dir = tempfile::tempdir().unwrap();
        let mk = [5u8; 32];
        let only = "dddd0000-0000-4000-8000-000000000001";

        // Bootstrap snapshot with seq_id 0 (empty op log) and one node.
        let boot = serde_json::json!({
            "seq_id": 0,
            "nodes": [ snap_node(&mk, only, "only.txt", None, false, 10) ],
        });
        // Empty ops list at since=0 (nothing happened yet). Because the ops list
        // is empty, the freshness-probe snapshot also fires; it returns seq_id 0,
        // which is NOT < cursor(0), so it does NOT re-bootstrap.
        let ops_empty = serde_json::json!({ "since": 0, "ops": [] });
        let probe = serde_json::json!({ "seq_id": 0, "nodes": [
            snap_node(&mk, only, "only.txt", None, false, 10)
        ] });

        let server = SyncMockServer::start(vec![
            ("200 OK".into(), boot),
            ("200 OK".into(), ops_empty),
            ("200 OK".into(), probe),
        ]);
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), mk);

        // Tick 1: cursor UNSET → bootstrap snapshot, stores cursor Some(0).
        sync_tick(&bridge, dir.path()).await.unwrap();
        assert_eq!(
            bridge.db().get_sync_cursor().unwrap(),
            Some(0),
            "bootstrap with seq_id 0 stores a real Some(0) cursor"
        );

        // Tick 2: cursor Some(0) → MUST take the ops path (since=0), NOT re-bootstrap.
        sync_tick(&bridge, dir.path()).await.unwrap();

        let requests = server.finish();
        assert_eq!(requests.len(), 3);
        assert_eq!(requests[0].path, "/api/v1/sync/snapshot", "tick1 = bootstrap");
        assert_eq!(
            requests[1].path, "/api/v1/sync/ops?since=0",
            "tick2 takes the ops delta path (NOT a re-bootstrap snapshot)"
        );
        // The 3rd request is the empty-delta freshness probe, not a re-bootstrap.
        assert_eq!(requests[2].path, "/api/v1/sync/snapshot");
        // The row from bootstrap survives (no spurious re-prune).
        assert!(bridge.db().get_file(only).unwrap().is_some());
    }

    #[tokio::test]
    async fn test_sync_tick_nested_create_and_move_match_fresh_snapshot_path() {
        // HIGH regression (issue 2): a nested file_create and a move-into-folder
        // applied via ops must land at the SAME mirror path a fresh snapshot would
        // produce — under the parent's prefix, not at the sync root.
        let dir = tempfile::tempdir().unwrap();
        let mk = [6u8; 32];
        let folder = "f1110000-0000-4000-8000-000000000001";
        let nested = "f1110000-0000-4000-8000-000000000002";
        let mover = "f1110000-0000-4000-8000-000000000003";

        // Bootstrap: just the folder (parents-known-first), seq 1.
        let boot = serde_json::json!({
            "seq_id": 1,
            "nodes": [ snap_node(&mk, folder, "docs", None, true, 10) ],
        });
        // Ops: create a NESTED file under `folder`; create a root file then move
        // it INTO `folder`. Highest seq = 4.
        let ops = serde_json::json!({
            "since": 1,
            "ops": [
                { "seq_id": 2, "op_type": "file_create",
                  "payload": { "id": nested, "name_encrypted": enc_name(&mk, nested, "notes.txt"),
                               "parent_id": folder, "size_bytes": 3, "storage_pool_id": "pool" } },
                { "seq_id": 3, "op_type": "file_create",
                  "payload": { "id": mover, "name_encrypted": enc_name(&mk, mover, "moved.txt"),
                               "parent_id": serde_json::Value::Null, "size_bytes": 3,
                               "storage_pool_id": "pool" } },
                { "seq_id": 4, "op_type": "file_move",
                  "payload": { "id": mover, "old_parent_id": serde_json::Value::Null,
                               "new_parent_id": folder } },
            ],
        });

        let server = SyncMockServer::start(vec![
            ("200 OK".into(), boot),
            ("200 OK".into(), ops),
        ]);
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), mk);

        sync_tick(&bridge, dir.path()).await.unwrap(); // bootstrap
        sync_tick(&bridge, dir.path()).await.unwrap(); // ops
        server.finish();

        // Both items nested under the folder prefix, exactly as a fresh snapshot
        // (which resolves nesting via parent_id) would place them.
        assert_eq!(
            bridge.db().get_file(nested).unwrap().unwrap().path,
            "docs/notes.txt",
            "nested file_create lands under the parent prefix, not at root"
        );
        assert_eq!(
            bridge.db().get_file(mover).unwrap().unwrap().path,
            "docs/moved.txt",
            "move-into-folder lands under the new parent prefix, not at root"
        );
    }

    #[tokio::test]
    async fn test_sync_tick_trash_then_restore_keeps_row_via_resnapshot() {
        // HIGH regression (issue 3/7): a trash op deletes the local row; the
        // following restore op ({id}-only payload) cannot rebuild it, so it must
        // schedule a re-snapshot that re-materialises the row on the next tick.
        let dir = tempfile::tempdir().unwrap();
        let mk = [8u8; 32];
        let file = "e1110000-0000-4000-8000-000000000001";

        // Bootstrap: the file exists, seq 1.
        let boot = serde_json::json!({
            "seq_id": 1,
            "nodes": [ snap_node(&mk, file, "doc.txt", None, false, 10) ],
        });
        // Ops: trash then restore. Highest seq = 3.
        let ops = serde_json::json!({
            "since": 1,
            "ops": [
                { "seq_id": 2, "op_type": "file_trash", "payload": { "id": file } },
                { "seq_id": 3, "op_type": "file_restore", "payload": { "id": file } },
            ],
        });
        // The restore scheduled a re-snapshot, so the NEXT tick bootstraps from
        // this fresh snapshot (which includes the restored, no-longer-trashed row).
        let resnap = serde_json::json!({
            "seq_id": 3,
            "nodes": [ snap_node(&mk, file, "doc.txt", None, false, 10) ],
        });

        let server = SyncMockServer::start(vec![
            ("200 OK".into(), boot),
            ("200 OK".into(), ops),
            ("200 OK".into(), resnap),
        ]);
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), mk);

        sync_tick(&bridge, dir.path()).await.unwrap(); // bootstrap
        sync_tick(&bridge, dir.path()).await.unwrap(); // ops: trash + restore (deletes, schedules re-snapshot)
        // After the ops tick, the trash removed the row and the restore could not
        // rebuild it from {id} alone — it is gone for THIS tick, but a re-snapshot
        // is pending.
        assert!(
            bridge.db().get_file(file).unwrap().is_none(),
            "trash removed the row; restore can't rebuild it from {{id}} alone"
        );
        sync_tick(&bridge, dir.path()).await.unwrap(); // re-snapshot re-materialises the row

        let requests = server.finish();
        assert_eq!(requests.len(), 3);
        assert_eq!(requests[2].path, "/api/v1/sync/snapshot", "tick3 = forced re-snapshot");
        assert!(
            bridge.db().get_file(file).unwrap().is_some(),
            "re-snapshot re-materialised the restored row — it is NOT permanently invisible"
        );
    }

    #[tokio::test]
    async fn test_sync_tick_empty_snapshot_does_not_prune_existing_tree() {
        // BLOCKER regression (issue 4): a degraded EMPTY snapshot (200 with no
        // nodes) must NOT prune the entire local own-tree.
        let dir = tempfile::tempdir().unwrap();
        let mk = [2u8; 32];
        let kept = "c2220000-0000-4000-8000-000000000001";

        // Pre-seed a settled own-tree row stamped in the past (so the freshness
        // cutoff would otherwise allow pruning it).
        {
            let bridge = test_bridge_with_api(&dir.path().join("state.db"), "http://placeholder".into(), mk);
            bridge.db().upsert_file(&FileEntry {
                file_id: kept.into(),
                path: "important.txt".into(),
                status: FileStatus::CloudOnly,
                size_bytes: 1,
                modified_at: 0,
                content_hash: None,
                remote_updated_at: 0,
                parent_id: None,
                item_kind: ItemKind::File,
            }).unwrap();
        }

        // Bootstrap with an EMPTY snapshot.
        let boot = serde_json::json!({ "seq_id": 7, "nodes": [] });
        let server = SyncMockServer::start(vec![("200 OK".into(), boot)]);
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), mk);

        sync_tick(&bridge, dir.path()).await.unwrap();
        server.finish();

        assert!(
            bridge.db().get_file(kept).unwrap().is_some(),
            "empty snapshot must NOT prune the existing own-tree (fail-closed)"
        );
    }
}
