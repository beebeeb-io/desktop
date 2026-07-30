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

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use zeroize::{Zeroize, Zeroizing};

use base64::Engine;
use beebeeb_types::{CipherSuite, EncryptedBlob};
use serde::{Deserialize, Serialize};

use crate::api_client::{ApiClient, DesktopUploadInitRequest};
use crate::conflict::{VersionInfo, is_conflict, is_text_file};
use crate::state_db::{
    FileContractState, FileEntry, FileStatus, ItemKind, LocalActivityEventInput, LocalActivityKind, Namespace,
    OperationKind, OperationPauseReason, PERMISSION_OWNER, PERMISSION_READ, PERMISSION_SHARE, PERMISSION_WRITE,
    PendingOperation, QueueDiagnostics, StateDb,
};

// ── Wire-byte counters (P1 — live throughput) ────────────────────────────────
//
// Two `AtomicU64` counters track bytes actually sent/received on the wire in
// the chunk loops (upload + download).  The heartbeat producer drains them with
// `swap(0)` each beat so it gets the delta over the beat interval — the true
// wire speed, not a file-completion delta.
//
// Both are incremented from async task context (no blocking), so `Relaxed`
// ordering is sufficient: each counter is touched only from the engine thread,
// and the heartbeat producer races on a separate task.  The only requirement is
// that the swap and the add are individually atomic — a consistent view across
// both counters simultaneously is NOT required (the heartbeat is a telemetry
// estimate, not an accounting figure).

/// Shared wire-byte counters threaded from `EngineBridge` to `runner`
/// to the heartbeat producer. Wrapped in `Arc` so it can be cloned
/// cheaply into the spawned heartbeat task.
pub struct WireCounters {
    /// Bytes written to the server in upload chunk loops (plaintext length
    /// of each chunk before encryption — the user's data rate, not wire overhead).
    pub upload_bytes: AtomicU64,
    /// Bytes received from the server in download chunk loops (raw wire bytes
    /// including the encryption envelope, which is all the client can measure
    /// without re-decrypting — close enough for throughput display).
    pub download_bytes: AtomicU64,
}

impl WireCounters {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            upload_bytes: AtomicU64::new(0),
            download_bytes: AtomicU64::new(0),
        })
    }

    /// Drain and return `(upload_bytes, download_bytes)` since the last drain.
    /// Both counters are atomically reset to 0. Called once per heartbeat beat.
    pub fn drain(&self) -> (u64, u64) {
        let up = self.upload_bytes.swap(0, Ordering::Relaxed);
        let dn = self.download_bytes.swap(0, Ordering::Relaxed);
        (up, dn)
    }
}

const THUMBNAIL_VARIANTS_FOR_UPLOAD: [ThumbnailUploadVariant; 2] =
    [ThumbnailUploadVariant::Medium, ThumbnailUploadVariant::Large];
const MEDIUM_ENCRYPTED_THUMBNAIL_MAX_BYTES: usize = 128 * 1024;
const LARGE_ENCRYPTED_THUMBNAIL_MAX_BYTES: usize = 512 * 1024;
const BLURHASH_COMPONENTS_X: usize = 4;
const BLURHASH_COMPONENTS_Y: usize = 3;
const BLURHASH_SOURCE_MAX_DIMENSION: u32 = 64;
const BLURHASH_BASE83: &[u8; 83] =
    b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz#$%*+,-.:;=?@[]^_{|}~";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ThumbnailUploadVariant {
    Medium,
    Large,
}

impl ThumbnailUploadVariant {
    fn label(self) -> &'static str {
        match self {
            Self::Medium => "medium",
            Self::Large => "large",
        }
    }

    fn config(self) -> beebeeb_core::thumbnail::ThumbnailConfig {
        match self {
            Self::Medium => beebeeb_core::thumbnail::ThumbnailConfig::medium(),
            Self::Large => beebeeb_core::thumbnail::ThumbnailConfig::large(),
        }
    }

    fn encrypted_max_bytes(self) -> usize {
        match self {
            Self::Medium => MEDIUM_ENCRYPTED_THUMBNAIL_MAX_BYTES,
            Self::Large => LARGE_ENCRYPTED_THUMBNAIL_MAX_BYTES,
        }
    }
}

#[derive(Debug)]
struct ThumbnailSource {
    rgba: Zeroizing<Vec<u8>>,
    width: u32,
    height: u32,
    is_video: bool,
}

#[derive(Debug)]
struct PreparedThumbnailUpload {
    variant: ThumbnailUploadVariant,
    encrypted: Zeroizing<Vec<u8>>,
    blurhash: Option<String>,
}

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
    /// Wire-byte counters shared with the heartbeat producer. Both are
    /// drained (swapped to 0) once per beat; incremented by the chunk loops.
    pub wire: Arc<WireCounters>,
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
            max_unpinned_cache_bytes: crate::config::LOCAL_CACHE_LIMIT_100_GB_BYTES,
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
    owner_email: Option<String>,
    sender_public_key: Option<String>,
    encrypted_file_key: Option<String>,
    encrypted_folder_key: Option<String>,
    file_name_encrypted: Option<String>,
    permission_bits: i64,
    approved_at: Option<i64>,
}

impl EngineBridge {
    pub fn new(db: Arc<StateDb>, api: Arc<ApiClient>) -> Self {
        Self {
            db,
            api,
            wire: WireCounters::new(),
        }
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
            let result = self.execute_operation(&op, sync_root, now).await;
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

    async fn execute_operation(&self, op: &PendingOperation, sync_root: &Path, now: i64) -> anyhow::Result<()> {
        match op.kind {
            OperationKind::PinTree => Ok(()),
            OperationKind::HydrateFile => {
                let file_id = op
                    .file_id
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("hydrate operation missing file_id"))?;
                let dest = if let Some(target_path) = op.target_path.as_deref() {
                    local_file_path_under_sync_root(sync_root, target_path)?
                } else if let Some(entry) = self.db.get_file(file_id)? {
                    local_file_path_under_sync_root(sync_root, &entry.path)?
                } else {
                    return Err(anyhow::anyhow!("hydrate operation target missing from state"));
                };
                self.hydrate_file(file_id, &dest, &[sync_root]).await
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
                let activity_entry = self.db.get_file(file_id)?;
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
                if let Some(entry) = activity_entry {
                    record_moved_to_trash_activity(self.db.as_ref(), file_id, &entry.path, now)?;
                }
                // Trace logs keep only the opaque id; plaintext names stay in the
                // local state DB, where file paths already live.
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

    async fn upload_version(
        &self,
        op: &PendingOperation,
        #[cfg_attr(not(target_os = "windows"), allow(unused_variables))] sync_root: &Path,
    ) -> anyhow::Result<()> {
        let local_file_id = op
            .file_id
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("upload operation missing file_id"))?;
        let previous_status = self.db.get_file(local_file_id)?.map(|entry| entry.status);
        self.db.set_status(local_file_id, FileStatus::Uploading)?;

        let result = self.do_upload_version(local_file_id, op, sync_root).await;
        if result.is_err() {
            let fallback_status = match previous_status {
                Some(FileStatus::Conflict) => FileStatus::Conflict,
                _ => FileStatus::Error,
            };
            let _ = self.db.set_status(local_file_id, fallback_status);
        }
        result
    }

    async fn do_upload_version(
        &self,
        local_file_id: &str,
        op: &PendingOperation,
        #[cfg_attr(not(target_os = "windows"), allow(unused_variables))] sync_root: &Path,
    ) -> anyhow::Result<()> {
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
        // Rate-limit ceiling: read once per file, not per chunk (config is on
        // disk but the file is fast to parse; at most one read per upload op).
        let upload_kbps_limit = crate::config::DesktopConfig::load()
            .map(|c| c.upload_kbps_limit)
            .unwrap_or(0);

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
            let chunk_start = std::time::Instant::now();
            self.api
                .upload_session_chunk(&upload.upload_session_id, chunk_index as u32, &encrypted)
                .await?;

            // P1 — wire-byte counter: count plaintext bytes (the user-data rate).
            self.wire.upload_bytes.fetch_add(read as u64, Ordering::Relaxed);

            // E — token-bucket pacing: if a limit is set and the chunk transferred
            // faster than the budget allows, sleep the remainder.
            if upload_kbps_limit > 0 {
                let budget_secs = read as f64 / (upload_kbps_limit as f64 * 1024.0);
                let elapsed_secs = chunk_start.elapsed().as_secs_f64();
                if budget_secs > elapsed_secs {
                    let sleep_ms = ((budget_secs - elapsed_secs) * 1000.0) as u64;
                    if sleep_ms > 0 {
                        tokio::time::sleep(Duration::from_millis(sleep_ms)).await;
                    }
                }
            }
        }

        let completed = self.api.complete_upload_session(&upload.upload_session_id).await?;
        let thumbnail_content_type = content_type.clone();
        self.apply_completed_upload(
            local_file_id,
            &server_file_id,
            op,
            &completed,
            plaintext_size,
            content_type,
            Some(upload.object_version_id),
        )?;
        if let Err(e) = self
            .upload_thumbnails_for_plaintext_media(
                &server_file_id,
                payload_path,
                thumbnail_content_type.as_deref(),
                &file_key,
            )
            .await
        {
            tracing::warn!(
                file_id = %server_file_id,
                error = %e,
                "upload-time thumbnail generation/upload skipped"
            );
        }
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

    async fn upload_thumbnails_for_plaintext_media(
        &self,
        server_file_id: &str,
        payload_path: &Path,
        mime_type: Option<&str>,
        file_key: &beebeeb_core::kdf::FileKey,
    ) -> anyhow::Result<()> {
        let uploads = prepare_thumbnail_uploads_for_plaintext_media(payload_path, mime_type, file_key)?;
        for upload in uploads {
            self.api
                .upload_thumbnail(
                    server_file_id,
                    upload.variant.label(),
                    &upload.encrypted,
                    upload.blurhash.as_deref(),
                )
                .await
                .map_err(|e| anyhow::anyhow!("upload {} thumbnail: {e}", upload.variant.label()))?;
        }
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
        let on_disk = sync_root.join(
            target_path
                .trim_start_matches('/')
                .replace('/', std::path::MAIN_SEPARATOR_STR),
        );
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
                owner_email: None,
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

    fn decrypted_direct_shared_root_name(&self, root: &SharedRootMapping) -> Option<String> {
        let sender_public_key = root.sender_public_key.as_deref()?;
        let encrypted_file_key = root.encrypted_file_key.as_deref()?;
        let name_encrypted = root.file_name_encrypted.as_deref()?;
        let file_key = unwrap_direct_shared_file_key(
            self.api.master_key(),
            sender_public_key,
            &root.file_id,
            encrypted_file_key,
        )
        .ok()?;
        decrypt_shared_name_with_key(&file_key, name_encrypted)
    }

    async fn shared_folder_material(
        &self,
        root: &SharedRootMapping,
    ) -> anyhow::Result<(Zeroizing<[u8; 32]>, serde_json::Value)> {
        let sender_public_key = root
            .sender_public_key
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("folder share {} is missing sender_public_key", root.invite_id))?;
        let encrypted_folder_key = root
            .encrypted_folder_key
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("folder share {} is missing encrypted_folder_key", root.invite_id))?;
        let folder_key = unwrap_folder_share_key(
            self.api.master_key(),
            sender_public_key,
            &root.file_id,
            encrypted_folder_key,
        )?;
        let folder_keys_response = self.api.get_folder_keys(&root.invite_id).await?;
        Ok((folder_key, folder_keys_response))
    }

    fn decrypted_folder_root_name(
        &self,
        root: &SharedRootMapping,
        folder_key: &[u8; 32],
        folder_keys_response: &serde_json::Value,
    ) -> Option<String> {
        let name_encrypted = root.file_name_encrypted.as_deref()?;
        let encrypted_file_key = encrypted_file_key_from_folder_keys_response(folder_keys_response, &root.file_id)?;
        let file_key = unwrap_child_file_key(folder_key, encrypted_file_key).ok()?;
        decrypt_shared_name_with_key(&file_key, name_encrypted)
    }

    fn shared_root_display_name(
        &self,
        root: &SharedRootMapping,
        folder_key: Option<&[u8; 32]>,
        folder_keys_response: Option<&serde_json::Value>,
    ) -> String {
        let item_kind = if root.is_folder {
            ItemKind::Folder
        } else {
            ItemKind::File
        };
        let decrypted = if root.is_folder {
            folder_key
                .zip(folder_keys_response)
                .and_then(|(key, keys_response)| self.decrypted_folder_root_name(root, key, keys_response))
        } else {
            self.decrypted_direct_shared_root_name(root)
        };

        decrypted
            .as_deref()
            .and_then(safe_shared_leaf_name)
            .unwrap_or_else(|| shared_placeholder_name(&item_kind).to_string())
    }

    async fn refresh_shared_folder_children(
        &self,
        root: &SharedRootMapping,
        root_display_name: &str,
        folder_key: &[u8; 32],
        folder_keys_response: &serde_json::Value,
        now: i64,
    ) -> anyhow::Result<()> {
        let key_map = folder_keys_map(folder_keys_response);
        let root_path = format!("Shared with me/{root_display_name}");
        let mut queue: VecDeque<(Option<String>, String, String)> =
            VecDeque::from([(None, root.file_id.clone(), root_path)]);

        while let Some((api_parent_id, db_parent_id, parent_path)) = queue.pop_front() {
            let children = self
                .api
                .list_shared_folder_files(&root.file_id, api_parent_id.as_deref())
                .await?;

            for child in children {
                let Some(child_id) = child["id"].as_str().map(str::to_string) else {
                    continue;
                };
                let child_file_key = key_map
                    .get(&child_id)
                    .and_then(|encrypted| unwrap_child_file_key(folder_key, encrypted).ok());
                let Some(entry) = apply_shared_metadata_file_row(
                    &self.db,
                    &child,
                    root,
                    now,
                    Some(db_parent_id.clone()),
                    &parent_path,
                    child_file_key.as_ref(),
                )?
                else {
                    continue;
                };

                if entry.item_kind == ItemKind::Folder {
                    queue.push_back((Some(child_id.clone()), child_id, entry.path));
                }
            }
        }

        Ok(())
    }

    pub async fn refresh_shared_roots(&self) -> anyhow::Result<SharedRootRefreshOutcome> {
        let body = self.api.list_shared_roots().await?;
        let roots = shared_roots_from_invite_response(&body);
        let active_shared_root_ids: Vec<String> = roots.iter().map(|root| root.file_id.clone()).collect();
        let now = now_secs();

        for root in &roots {
            let mut folder_material = None;
            if root.is_folder {
                match self.shared_folder_material(root).await {
                    Ok((folder_key, folder_keys_response)) => {
                        folder_material = Some((folder_key, folder_keys_response));
                    }
                    Err(e) => {
                        tracing::warn!(
                            invite_id = %root.invite_id,
                            error = %e,
                            "shared folder key unwrap failed during refresh"
                        );
                    }
                }
            }
            let display_name = self.shared_root_display_name(
                root,
                folder_material.as_ref().map(|(folder_key, _)| &**folder_key),
                folder_material.as_ref().map(|(_, keys_response)| keys_response),
            );
            let root_path = format!("Shared with me/{display_name}");
            crate::reject_unsafe_rel_path(&root_path)
                .map_err(|e| anyhow::anyhow!("unsafe shared root path for {}: {e}", root.file_id))?;

            self.db.upsert_file(&FileEntry {
                file_id: root.file_id.clone(),
                path: root_path,
                status: FileStatus::CloudOnly,
                size_bytes: root.size_bytes,
                modified_at: root.approved_at.unwrap_or(now),
                content_hash: None,
                remote_updated_at: root.approved_at.unwrap_or(0),
                // upsert_file does not persist these; the contract update
                // just below sets the authoritative parent_id/item_kind.
                parent_id: None,
                item_kind: if root.is_folder {
                    ItemKind::Folder
                } else {
                    ItemKind::File
                },
            })?;

            let mut contract = self
                .db
                .get_file_contract_state(&root.file_id)?
                .ok_or_else(|| anyhow::anyhow!("missing state row for shared root {}", root.file_id))?;
            contract.namespace = Namespace::SharedWithMe;
            contract.parent_id = None;
            contract.shared_root_id = Some(root.file_id.clone());
            contract.share_id = Some(root.invite_id.clone());
            contract.owner_email = root.owner_email.clone();
            contract.permission_bits = root.permission_bits;
            contract.item_kind = if root.is_folder {
                ItemKind::Folder
            } else {
                ItemKind::File
            };
            contract.content_type = root.content_type.clone();
            contract.last_sync_at = now;
            self.db.set_file_contract_state(&contract)?;

            if let Some((folder_key, folder_keys_response)) = folder_material
                && let Err(e) = self
                    .refresh_shared_folder_children(root, &display_name, &folder_key, &folder_keys_response, now)
                    .await
            {
                tracing::warn!(
                    invite_id = %root.invite_id,
                    error = %e,
                    "shared folder child refresh failed"
                );
            }
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
        // Honour a caller-supplied id (task 0811 folder scaffolding writes the
        // local row under a known id and needs the server CreateFolder to reuse
        // it — the server accepts a client `folder_id`). The file path and the
        // legacy IPC always pass `None`, getting a fresh uuid as before.
        let file_id = target
            .file_id
            .clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
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
                // Write the local folder row + contract synchronously, the same
                // way the File branch writes its `Uploading` row, so a nested
                // child dispatched later in the SAME scan resolves this folder as
                // its parent (`resolve_parent_id_for` reads `files.item_kind`)
                // without waiting for a server `/sync` round-trip. `Local` =
                // present on disk, server CreateFolder op pending in the queue.
                self.db.upsert_file(&FileEntry {
                    file_id: file_id.clone(),
                    path: rel_path.clone(),
                    status: FileStatus::Local,
                    size_bytes: 0,
                    modified_at: now_secs(),
                    content_hash: None,
                    remote_updated_at: 0,
                    // Not persisted by upsert_file; the contract update owns these.
                    parent_id: None,
                    item_kind: ItemKind::Folder,
                })?;
                if let Some(mut contract) = self.db.get_file_contract_state(&file_id)? {
                    contract.item_kind = ItemKind::Folder;
                    contract.parent_id = target.parent_id.clone();
                    self.db.set_file_contract_state(&contract)?;
                }
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
            Ok(m) => {
                let file_type = m.file_type();
                tracing::debug!(
                    path = %path.display(),
                    is_dir = file_type.is_dir(),
                    is_symlink = file_type.is_symlink(),
                    "classify_local_path: rejected non-regular file"
                );
                return None;
            }
            Err(e) => {
                tracing::debug!(
                    path = %path.display(),
                    error = %e,
                    "classify_local_path: stat failed; skipping"
                );
                return None;
            }
        }

        // Filter 1 — engine-internal paths + OS junk.
        if path_is_engine_internal(sync_root, path) {
            tracing::debug!(
                path = %path.display(),
                "classify_local_path: rejected engine-internal path"
            );
            return None;
        }
        let Some(file_name) = path.file_name().and_then(|n| n.to_str()).map(str::to_string) else {
            tracing::debug!(
                path = %path.display(),
                "classify_local_path: rejected path with non-utf8 filename"
            );
            return None;
        };
        if is_ignored_finder_name(&file_name) {
            tracing::debug!(
                path = %path.display(),
                filename = %file_name,
                "classify_local_path: rejected ignored/temp filename"
            );
            return None;
        }

        // Filter 2 — Cloud Files placeholders are engine-owned (Windows only). A
        // reparse point under the sync root is a placeholder we minted or
        // converted; the hydration write that fills it does NOT turn it back into
        // a plain file, so we must never treat a placeholder write as a new
        // upload.
        #[cfg(target_os = "windows")]
        if crate::windows_cf::placeholders::is_cloud_placeholder(path) {
            tracing::debug!(
                path = %path.display(),
                "classify_local_path: rejected Cloud Files placeholder"
            );
            return None;
        }

        // Filter 3 (authoritative) — already a known server file?
        let Some(rel) = relative_db_path(sync_root, path) else {
            tracing::debug!(
                path = %path.display(),
                sync_root = %sync_root.display(),
                "classify_local_path: rejected path outside sync root"
            );
            return None;
        };
        match self.db.get_file_by_path(&rel) {
            Ok(Some(_existing)) => {
                tracing::debug!(
                    path = %path.display(),
                    rel_path = %rel,
                    "classify_local_path: rejected already-tracked server file"
                );
                return None;
            }
            Ok(None) => { /* genuinely new — fall through */ }
            Err(e) => {
                tracing::warn!(
                    path = %path.display(),
                    rel_path = %rel,
                    error = %e,
                    "classify_local_path: state DB lookup failed; skipping to be safe"
                );
                return None;
            }
        }

        // parent_id resolution: a survivor at `<parent_dir>/<name>`. If the parent
        // directory maps to a known server FOLDER row, attach the new file to it;
        // otherwise upload at the root (None). We never attach to a row that isn't
        // a folder.
        let parent_id = self.resolve_parent_id_for(sync_root, path);
        tracing::debug!(
            path = %path.display(),
            rel_path = %rel,
            nested = parent_id.is_some(),
            "classify_local_path: accepted new local file"
        );

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

    /// Ensure the on-disk DIRECTORY at `path` (under `sync_root`) has a server
    /// vault folder, returning its `file_id`. This is the folder-hierarchy half
    /// of the local-create pipeline (task 0811): the enumeration scan only
    /// uploaded *files*, so a freshly-mirrored nested tree
    /// (`Backup/<device>/<folder>/<sub>/file`) had no folder rows — every nested
    /// file's `resolve_parent_id_for` missed and the file uploaded FLAT to the
    /// vault root. Scaffolding the folder first (parents before children, which
    /// the top-down walk already guarantees) gives each child a real parent to
    /// attach to, so the server vault mirrors the on-disk hierarchy.
    ///
    /// Idempotent + parent-aware:
    /// - If a DB row already exists for this directory's relative key, returns its
    ///   `file_id` (a folder row, or `None` if it is somehow a file row).
    /// - Otherwise mints a client folder id, writes a local `Folder` row + contract
    ///   IMMEDIATELY (so a child file dispatched later in the SAME scan resolves
    ///   this parent without waiting for a server round-trip), and enqueues a
    ///   `CreateFolder` op carrying that same id — the server honours the
    ///   client-supplied `folder_id`, so the local row and the server folder share
    ///   one id and the parent linkage is consistent.
    ///
    /// The op's `target_path` is the folder's relative key, so a backup folder is
    /// auto-tagged with its origin key (Part C) and purged on disable. Returns
    /// `None` for the sync root itself, a non-directory, or on a DB error
    /// (fail-closed: the file then uploads at the root rather than mis-parenting).
    pub fn ensure_local_folder(&self, sync_root: &Path, path: &Path) -> Option<String> {
        // Root itself has no server folder.
        if path == sync_root {
            return None;
        }
        match std::fs::symlink_metadata(path) {
            Ok(m) if m.is_dir() => {}
            _ => return None,
        }
        if path_is_engine_internal(sync_root, path) {
            return None;
        }
        let name = path.file_name().and_then(|n| n.to_str())?.to_string();
        if is_ignored_finder_name(&name) {
            return None;
        }
        let rel = relative_db_path(sync_root, path)?;

        // Already known? Return the existing folder id (idempotent across scans).
        match self.db.get_file_by_path(&rel) {
            Ok(Some(entry)) if entry.is_dir() => return Some(entry.file_id),
            Ok(Some(_)) => return None, // a file row at a dir path — never mis-parent
            Ok(None) => { /* mint below */ }
            Err(e) => {
                tracing::warn!(error = %e, "ensure_local_folder: DB lookup failed; skipping");
                return None;
            }
        }

        // Parent linkage: resolve THIS directory's parent dir to its folder id
        // (None at the Backup root level — a top-level folder under the vault).
        let parent_id = self.resolve_parent_id_for(sync_root, path);
        let folder_id = uuid::Uuid::new_v4().to_string();

        // Delegate the row write + contract + CreateFolder enqueue to the one
        // shared create path, carrying our pre-minted id (which the server
        // honours) so the local row and the server folder share an id. The
        // `rel`-path `target_path` auto-tags a backup folder with its origin key.
        let target = FinderWriteTarget {
            file_id: Some(folder_id.clone()),
            parent_id,
            filename: name,
            rel_path: Some(rel),
            kind: FinderWriteItemKind::Folder,
            contents_path: None,
            content_type: None,
            base_version_identifier: None,
        };
        match self.queue_finder_create(target) {
            Ok(FinderWriteOutcome::Queued { op_id, .. }) => {
                tracing::info!(op_id = %op_id, folder_id = %folder_id, "upload driver: scaffolded vault folder for nested upload");
                Some(folder_id)
            }
            Ok(FinderWriteOutcome::Ignored { .. }) => None,
            Err(e) => {
                tracing::warn!(error = %e, "ensure_local_folder: failed to scaffold vault folder");
                None
            }
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
        self.enforce_configured_cache_limit()
    }

    pub fn enforce_smart_cache(&self, policy: CachePolicy) -> anyhow::Result<CacheCleanupOutcome> {
        let evicted = self
            .db
            .evict_unpinned_cache_until_under(policy.max_unpinned_cache_bytes, now_secs())?;
        #[cfg(target_os = "linux")]
        self.remove_linux_freedesktop_thumbnails_for_file_ids(&evicted);
        Ok(CacheCleanupOutcome {
            evicted_file_ids: evicted,
        })
    }

    pub fn enforce_local_cache_limit(&self, max_local_cache_bytes: Option<i64>) -> anyhow::Result<CacheCleanupOutcome> {
        let Some(max_local_cache_bytes) = max_local_cache_bytes else {
            return Ok(CacheCleanupOutcome {
                evicted_file_ids: Vec::new(),
            });
        };
        if max_local_cache_bytes <= 0 {
            return Ok(CacheCleanupOutcome {
                evicted_file_ids: Vec::new(),
            });
        }

        let pinned_bytes = self.db.cache_bytes_by_effective_pin(true)?.max(0);
        let max_unpinned_cache_bytes = max_local_cache_bytes.saturating_sub(pinned_bytes);
        self.enforce_smart_cache(CachePolicy {
            max_unpinned_cache_bytes,
            ..CachePolicy::default()
        })
    }

    pub fn local_cache_usage_bytes(&self) -> anyhow::Result<i64> {
        let pinned_bytes = self.db.cache_bytes_by_effective_pin(true)?.max(0);
        let unpinned_bytes = self.db.cache_bytes_by_effective_pin(false)?.max(0);
        Ok(pinned_bytes.saturating_add(unpinned_bytes))
    }

    pub fn enforce_configured_cache_limit(&self) -> anyhow::Result<CacheCleanupOutcome> {
        let cfg = crate::config::DesktopConfig::load().unwrap_or_default();
        self.enforce_local_cache_limit(cfg.local_cache_limit_for_eviction())
    }

    #[cfg(target_os = "linux")]
    async fn write_linux_freedesktop_thumbnails(&self, file_id: &str, dest_path: &Path) {
        let entry = match self.db.get_file(file_id) {
            Ok(Some(entry)) if !entry.is_dir() => entry,
            Ok(_) => return,
            Err(error) => {
                tracing::debug!(file_id = %file_id, error = %error, "linux thumbnail skipped: state row unavailable");
                return;
            }
        };

        let source_path = linux_thumbnail_source_path_for_entry(&entry).unwrap_or_else(|| dest_path.to_path_buf());
        let thumbnail = match tokio::time::timeout(
            Duration::from_secs(8),
            self.fetch_thumbnail_to_memory(file_id, "medium"),
        )
        .await
        {
            Ok(Ok(bytes)) => bytes,
            Ok(Err(error)) => {
                tracing::debug!(file_id = %file_id, error = %error, "linux thumbnail skipped: server thumbnail unavailable");
                return;
            }
            Err(_) => {
                tracing::debug!(file_id = %file_id, "linux thumbnail skipped: server thumbnail fetch timed out");
                return;
            }
        };

        match crate::linux_thumbnail::write_freedesktop_thumbnails(&source_path, entry.modified_at, &thumbnail) {
            Ok(paths) => {
                tracing::debug!(
                    file_id = %file_id,
                    count = paths.len(),
                    "linux freedesktop thumbnails written"
                );
            }
            Err(error) => {
                tracing::warn!(file_id = %file_id, error = %error, "linux freedesktop thumbnail write failed");
            }
        }
    }

    #[cfg(target_os = "linux")]
    fn remove_linux_freedesktop_thumbnails_for_file_ids(&self, file_ids: &[String]) {
        for file_id in file_ids {
            let entry = match self.db.get_file(file_id) {
                Ok(Some(entry)) if !entry.is_dir() => entry,
                Ok(_) => continue,
                Err(error) => {
                    tracing::debug!(file_id = %file_id, error = %error, "linux thumbnail cleanup skipped: state row unavailable");
                    continue;
                }
            };
            let Some(source_path) = linux_thumbnail_source_path_for_entry(&entry) else {
                continue;
            };
            if let Err(error) = crate::linux_thumbnail::remove_freedesktop_thumbnails(&source_path) {
                tracing::warn!(file_id = %file_id, error = %error, "linux freedesktop thumbnail cleanup failed");
            }
        }
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
            backup_source_key: None,
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
        // Task 0811: tag known-folder backup ops with their origin key (derived
        // from the server-relative path the upload threads as `target_path`), so
        // disabling that folder can purge exactly its queued ops. `None` for any
        // normal user upload — those are never purged by a folder disable. The
        // `_this_device` form pins segment 2 to THIS machine's sanitized device
        // name (the only value the mirror writes), so a user's own
        // `Backup/<other>/<CatalogName>/…` file is never mis-tagged + wrongly
        // purged (review fix).
        let backup_source_key = target_path
            .as_deref()
            .and_then(crate::known_folder::backup_source_key_for_this_device);
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
            backup_source_key,
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
            backup_source_key: None,
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
            backup_source_key: None,
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
    /// Called by the Linux FUSE driver, the IPC socket, and conflict-resolution
    /// paths where the decrypted file must land on disk (sync root or a caller-
    /// chosen path). Status transitions: `Downloading` → `Local` on success,
    /// `Error` on failure.
    ///
    /// Steps mirror `repos/cli/src/commands/pull.rs::pull_single_file`:
    ///
    /// 1. Flip status to `Downloading` so the Finder/Explorer overlay
    ///    shows a spinner immediately.
    /// 2. Fetch fresh metadata to learn `chunk_count` (the local
    ///    placeholder may not have it).
    /// 3. Resolve the per-file key. Owned files derive it from the master key;
    ///    shared-with-me files unwrap the sender-provided key envelope using
    ///    the share invite's X25519/HKDF contract.
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
    ///
    /// **Windows Cloud Files callers must use [`Self::hydrate_file_to_memory`]
    /// instead** — that variant never writes plaintext to disk, satisfying the
    /// zero-knowledge requirement for on-demand CF hydration.
    pub async fn hydrate_file(&self, file_id: &str, dest_path: &Path, allowed_roots: &[&Path]) -> anyhow::Result<()> {
        // Task 1247 self-defense: validate `dest_path` against the caller's
        // trusted root(s) BEFORE any directory creation or plaintext write.
        // `dest_path` reaches the one untrusted caller (the IPC socket handler)
        // straight off the wire, and the `file_id`-keyed guard below
        // (`ensure_shared_hydrate_path_safe`) validates an entirely SEPARATE
        // value (`entry.path`), so it can never vouch for `dest_path`. Without
        // this check any local process could turn the daemon into a
        // decrypt-oracle + arbitrary-write primitive (e.g. writing decrypted
        // vault plaintext over ~/.ssh/authorized_keys).
        if !hydrate_dest_is_allowed(dest_path, allowed_roots) {
            return Err(anyhow::anyhow!(
                "hydrate destination is not within an allowed root"
            ));
        }
        self.ensure_shared_hydrate_path_safe(file_id)?;
        // RAII-style: any early return below the status flip should
        // leave the file in `Error`, not `Downloading`. We do that by
        // wrapping the body in an inner async fn whose Err branch we
        // catch.
        self.db.set_status(file_id, FileStatus::Downloading)?;
        match self.do_hydrate(file_id).await {
            Ok(mut buf) => {
                // Write the decrypted bytes to disk (this is the intentional
                // disk-writing path — sync root / conflict resolution / FUSE).
                // Make the destination directory if needed.
                if let Some(parent) = dest_path.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|e| anyhow::anyhow!("create dest dir {}: {e}", parent.display()))?;
                }
                // Write via an O_NOFOLLOW handle (task 1247): the containment
                // guard at the top of this fn is a check-then-use, and there is
                // a real time window here (`do_hydrate` did a network download +
                // decrypt). A same-UID attacker could plant a symlink at
                // `dest_path` during that window pointing at, say,
                // ~/.ssh/authorized_keys; a plain `fs::write` would follow it
                // and overwrite the real target with decrypted plaintext.
                // `write_hydrated_plaintext` fails closed if the final component
                // is (or race-becomes) a symlink, closing the race atomically at
                // open() time rather than re-checking-then-hoping.
                let write_result = write_hydrated_plaintext(dest_path, allowed_roots, &buf)
                    .map_err(|e| anyhow::anyhow!("write {}: {e}", dest_path.display()));
                // Zeroize the in-memory copy now that it is on disk (or on
                // error) so the allocation does not linger with plaintext.
                buf.zeroize();
                write_result?;

                self.db.set_status(file_id, FileStatus::Local)?;
                let cache_bytes = std::fs::metadata(dest_path).map(|m| m.len() as i64).unwrap_or(0);
                self.db
                    .mark_cached(file_id, &dest_path.to_string_lossy(), cache_bytes, now_secs())?;
                #[cfg(target_os = "linux")]
                self.write_linux_freedesktop_thumbnails(file_id, dest_path).await;
                let _ = self.enforce_configured_cache_limit();
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

    fn ensure_shared_hydrate_path_safe(&self, file_id: &str) -> anyhow::Result<()> {
        let Some(contract) = self.db.get_file_contract_state(file_id)? else {
            return Ok(());
        };
        if contract.namespace != Namespace::SharedWithMe {
            return Ok(());
        }

        let entry = self
            .db
            .get_file(file_id)?
            .ok_or_else(|| anyhow::anyhow!("shared hydrate target missing state row for {file_id}"))?;
        crate::reject_unsafe_rel_path(&entry.path)
            .map_err(|e| anyhow::anyhow!("unsafe shared hydrate path for {file_id}: {e}"))?;
        Ok(())
    }

    /// Download `file_id` from the vault, decrypt, and return the plaintext
    /// **in memory only** — it is NEVER written to disk.
    ///
    /// This is the Windows Cloud Files hydration path. The Cloud Files runtime
    /// calls `fetch_data_callback` on a filter-driver thread and expects us to
    /// deliver bytes via `CfExecute(TRANSFER_DATA)`; there is no requirement to
    /// materialise the plaintext as a file. Writing to `%TEMP%` would expose
    /// decrypted user data on disk, violating the zero-knowledge contract.
    ///
    /// The returned [`Zeroizing`] wrapper overwrites the buffer with zeros on
    /// drop, so plaintext is wiped from memory as soon as the caller is done
    /// with it — even if an early `return` or `?` is taken. Intermediate
    /// per-chunk decrypted buffers are also explicitly zeroized after being
    /// copied into the accumulator (see [`Self::do_hydrate`]).
    ///
    /// Status transitions: `Downloading` → `Local` on success, `Error` on
    /// failure. The `cache_path` recorded in state_db is `""` (empty) because
    /// the bytes live inside the CF placeholder in the sync root, not at a
    /// separate cache file — `unpinned_local_files_for_dehydration` reconstructs
    /// the real on-disk path from `path` + sync root (see state_db comment).
    #[cfg(target_os = "windows")]
    pub async fn hydrate_file_to_memory(&self, file_id: &str) -> anyhow::Result<Zeroizing<Vec<u8>>> {
        self.db.set_status(file_id, FileStatus::Downloading)?;
        match self.do_hydrate(file_id).await {
            Ok(buf) => {
                self.db.set_status(file_id, FileStatus::Local)?;
                // cache_path is empty: on Windows CF the hydrated bytes live
                // INSIDE the placeholder (the CF runtime writes them there after
                // our CfExecute(TRANSFER_DATA) calls). There is no separate temp
                // file. Byte count is still recorded for smart-cache accounting.
                self.db.mark_cached(file_id, "", buf.len() as i64, now_secs())?;
                let _ = self.enforce_configured_cache_limit();
                Ok(buf)
            }
            Err(e) => {
                let _ = self.db.set_status(file_id, FileStatus::Error);
                Err(e)
            }
        }
    }

    /// Core download + decrypt loop. Returns the full plaintext in a
    /// [`Zeroizing`] wrapper so the allocation is wiped on drop regardless of
    /// which exit path is taken (normal return, early `?`, or a panic unwind).
    ///
    /// Intermediate per-chunk buffers from `decrypt_downloaded_chunk` are
    /// explicitly zeroized after being copied into the accumulator, so at most
    /// one extra chunk's worth of plaintext is live in memory at any time.
    ///
    /// This function deliberately does NOT write anything to disk. Callers that
    /// need the bytes on disk (`hydrate_file`) or in memory (`hydrate_file_to_memory`)
    /// handle that themselves so the disk-write decision stays at the call site,
    /// not buried inside the crypto loop.
    fn owned_file_key(&self, file_id: &str) -> beebeeb_core::kdf::FileKey {
        let mk_bytes: [u8; 32] = *self.api.master_key();
        let master_key = beebeeb_core::kdf::MasterKey::from_bytes(mk_bytes);
        beebeeb_core::kdf::derive_file_key(&master_key, file_id.as_bytes())
    }

    async fn file_key_for_download(&self, file_id: &str) -> anyhow::Result<beebeeb_core::kdf::FileKey> {
        let Some(contract) = self.db.get_file_contract_state(file_id)? else {
            return Ok(self.owned_file_key(file_id));
        };
        if contract.namespace != Namespace::SharedWithMe {
            return Ok(self.owned_file_key(file_id));
        }
        if !contract.can_read() {
            anyhow::bail!("shared item is not readable");
        }

        let share_id = contract
            .share_id
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("shared item is missing invite id"))?;
        let body = self.api.list_shared_roots().await?;
        let invite = body
            .get("invites")
            .and_then(|value| value.as_array())
            .into_iter()
            .flatten()
            .find(|invite| shared_invite_id_matches(invite, share_id))
            .ok_or_else(|| anyhow::anyhow!("shared invite {share_id} was not returned by the server"))?;
        let mapping = shared_root_from_invite(invite)
            .ok_or_else(|| anyhow::anyhow!("shared invite {share_id} is not approved or is malformed"))?;
        let sender_public_key = mapping
            .sender_public_key
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("shared invite {share_id} is missing sender_public_key"))?;

        if mapping.is_folder {
            let encrypted_folder_key = mapping
                .encrypted_folder_key
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("folder share {share_id} is missing encrypted_folder_key"))?;
            let folder_keys = self.api.get_folder_keys(share_id).await?;
            unwrap_folder_share_file_key(
                self.api.master_key(),
                sender_public_key,
                &mapping.file_id,
                encrypted_folder_key,
                file_id,
                &folder_keys,
            )
        } else {
            let encrypted_file_key = mapping
                .encrypted_file_key
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("share invite {share_id} is missing encrypted_file_key"))?;
            unwrap_direct_shared_file_key(self.api.master_key(), sender_public_key, file_id, encrypted_file_key)
        }
    }

    async fn do_hydrate(&self, file_id: &str) -> anyhow::Result<Zeroizing<Vec<u8>>> {
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

        let file_key = self.file_key_for_download(file_id).await?;

        // Rate-limit ceiling for downloads.
        let download_kbps_limit = crate::config::DesktopConfig::load()
            .map(|c| c.download_kbps_limit)
            .unwrap_or(0);

        // Walk chunks. Pre-allocate roughly the file size if known,
        // but fall back to defaults — chunks are encrypted so the
        // ciphertext is always larger than plaintext anyway.
        // Use Zeroizing so that if we bail mid-loop (network error,
        // decrypt error) the partial plaintext is still wiped.
        let approx_size = meta.get("size_bytes").and_then(|v| v.as_u64()).unwrap_or(0) as usize;

        self.download_and_decrypt_chunk_range(file_id, &file_key, 0..chunk_count, download_kbps_limit, approx_size)
            .await
    }

    /// Download + decrypt chunk indices `[range.start, range.end)` for
    /// `file_id`, concatenating the plaintext into one [`Zeroizing`] buffer.
    /// Shared by [`Self::do_hydrate`] (the whole-file range `0..chunk_count`)
    /// and [`Self::hydrate_file_range`] (a covering sub-range) so the
    /// pacing/wire-counting/zeroize discipline lives in exactly one place.
    ///
    /// `approx_size_hint` only sizes the initial allocation (`Vec::with_capacity`)
    /// — it never bounds or truncates the actual read, so a wrong hint costs at
    /// most a reallocation, never a silent short read.
    async fn download_and_decrypt_chunk_range(
        &self,
        file_id: &str,
        file_key: &beebeeb_core::kdf::FileKey,
        range: std::ops::Range<u32>,
        download_kbps_limit: u64,
        approx_size_hint: usize,
    ) -> anyhow::Result<Zeroizing<Vec<u8>>> {
        let mut acc: Zeroizing<Vec<u8>> = Zeroizing::new(Vec::with_capacity(approx_size_hint));

        for i in range {
            let chunk_start = std::time::Instant::now();
            let chunk_bytes = self.api.download_chunk(file_id, i).await?;
            let wire_len = chunk_bytes.len() as u64;
            // Decrypt into a temporary buffer, copy into the accumulator,
            // then zeroize the temporary. This limits live plaintext to
            // the accumulator + one chunk at any moment.
            let mut decrypted = decrypt_downloaded_chunk(file_key, &chunk_bytes)
                .map_err(|e| anyhow::anyhow!("decrypt chunk {i}: {e}"))?;
            acc.extend_from_slice(&decrypted);
            decrypted.zeroize();

            // P1 — wire-byte counter: count raw wire bytes received.
            self.wire.download_bytes.fetch_add(wire_len, Ordering::Relaxed);

            // E — token-bucket pacing for downloads.
            if download_kbps_limit > 0 {
                let budget_secs = wire_len as f64 / (download_kbps_limit as f64 * 1024.0);
                let elapsed_secs = chunk_start.elapsed().as_secs_f64();
                if budget_secs > elapsed_secs {
                    let sleep_ms = ((budget_secs - elapsed_secs) * 1000.0) as u64;
                    if sleep_ms > 0 {
                        tokio::time::sleep(Duration::from_millis(sleep_ms)).await;
                    }
                }
            }
        }

        Ok(acc)
    }

    /// Download `file_id` from the vault, decrypt, and return ONLY the plaintext
    /// bytes covering `[required_offset, required_offset + required_length)` —
    /// **in memory only**, never written to disk. This is the range-targeted
    /// counterpart to [`Self::hydrate_file_to_memory`] (task 1024, follow-up to
    /// 0769): instead of decrypting the whole file on every CF fetch callback, it
    /// downloads + decrypts only the chunks that cover the requested range, so
    /// peak memory is bounded by one covering-range buffer (typically a handful
    /// of chunks) rather than the entire file.
    ///
    /// Requires an authoritative, uniform `chunk_size_bytes` for the file. This
    /// value must come from the server's stored `object_versions` row; it cannot
    /// be recovered from `size_bytes / chunk_count`, because real files use a
    /// fixed chunk size with only the final chunk shortened.
    ///
    /// Returns `Ok(None)` (never an error) when the covering-chunk math can't be
    /// established from the metadata (`chunk_count` is `0`, `size_bytes` is
    /// missing, or `chunk_size_bytes` is missing/invalid) — the caller falls
    /// back to
    /// [`Self::hydrate_file_to_memory`] for the whole-file path in that case.
    ///
    /// Status bookkeeping: unlike [`Self::hydrate_file_to_memory`], this method
    /// does **not** flip the row to `FileStatus::Local` — a single range fetch is
    /// not evidence the whole file is now cached, so marking it fully `Local`
    /// here would be a stronger claim than the bytes we actually have. See the
    /// caller (`windows_cf::callbacks::fetch_data_callback`) for how "fully
    /// in-sync" is decided instead.
    #[cfg(any(target_os = "windows", test))]
    pub async fn hydrate_file_range(
        &self,
        file_id: &str,
        required_offset: u64,
        required_length: u64,
    ) -> anyhow::Result<Option<HydratedRange>> {
        let _file_uuid: uuid::Uuid = file_id
            .parse()
            .map_err(|e| anyhow::anyhow!("invalid file_id (not a UUID): {e}"))?;

        let meta = self.api.get_file(file_id).await?;
        let chunk_count = meta.get("chunk_count").and_then(|v| v.as_i64()).unwrap_or(0) as u64;
        let size_bytes = meta.get("size_bytes").and_then(|v| v.as_u64());
        let chunk_size_bytes = meta.get("chunk_size_bytes").and_then(|v| v.as_u64());

        let (Some(size_bytes), Some(chunk_size_bytes), true) = (size_bytes, chunk_size_bytes, chunk_count > 0) else {
            // Missing metadata — caller falls back to the whole-file path.
            return Ok(None);
        };
        if chunk_size_bytes == 0 {
            return Ok(None);
        }

        // Clamp the requested range to the file's real extent before computing
        // covering chunks, so a stale/over-wide CF request never derives an
        // out-of-bounds chunk index.
        let range_end = required_offset.saturating_add(required_length).min(size_bytes);
        if range_end <= required_offset {
            // Degenerate/empty range — nothing to hydrate.
            return Ok(Some(HydratedRange {
                data: Zeroizing::new(Vec::new()),
                range_start_bytes: required_offset,
                covers_whole_file: size_bytes == 0,
            }));
        }

        let Some(plan) = plan_hydration_chunk_range(
            size_bytes,
            chunk_count,
            chunk_size_bytes,
            required_offset,
            required_length,
        )?
        else {
            return Ok(None);
        };

        self.db.set_status(file_id, FileStatus::Downloading)?;

        let file_key = self.file_key_for_download(file_id).await?;
        let download_kbps_limit = crate::config::DesktopConfig::load()
            .map(|c| c.download_kbps_limit)
            .unwrap_or(0);

        match self
            .download_and_decrypt_chunk_range(
                file_id,
                &file_key,
                plan.first_chunk..plan.last_chunk_exclusive,
                download_kbps_limit,
                plan.covering_span_hint,
            )
            .await
        {
            Ok(data) => {
                // Only a whole-file covering range is evidence the file is fully
                // cached; a partial range leaves the row `Downloading` for the
                // caller (the CF callback) to resolve based on what CF itself
                // reports (see `windows_cf::callbacks`).
                if plan.covers_whole_file {
                    self.db.set_status(file_id, FileStatus::Local)?;
                    self.db.mark_cached(file_id, "", data.len() as i64, now_secs())?;
                    let _ = self.enforce_configured_cache_limit();
                }
                Ok(Some(HydratedRange {
                    data,
                    range_start_bytes: plan.range_start_bytes,
                    covers_whole_file: plan.covers_whole_file,
                }))
            }
            Err(e) => {
                let _ = self.db.set_status(file_id, FileStatus::Error);
                Err(e)
            }
        }
    }

    /// Fetch the encrypted server thumbnail for `file_id`, decrypt it **in
    /// memory only**, and return the decoded image bytes (WebP / JPEG / PNG)
    /// inside a [`Zeroizing`] wrapper.
    ///
    /// This is the shared read path for OS thumbnail consumers. It mirrors
    /// [`Self::do_hydrate`]'s per-file-key derivation but hits
    /// `GET /files/:id/thumbnail/{variant}` instead of the chunk range and reuses
    /// the **existing** encrypted thumbnail the original uploading client
    /// generated (a downscaled still for images, a poster frame for video). OS
    /// integrations never regenerate a thumbnail and never decode the original
    /// media just to preview it — they decrypt this small blob.
    ///
    /// ## Wire envelope (NOT the chunk envelope)
    ///
    /// Thumbnails are stored in the **raw** `nonce(12) || AES-256-GCM(ct+tag)`
    /// envelope the web/mobile clients write (`thumbnail.ts encryptThumbnailBlob`):
    /// a 12-byte random nonce followed by the AES-256-GCM ciphertext+tag, keyed
    /// directly by the per-file key, with no AAD and no JSON. This differs from
    /// file *chunks* (which are the `EncryptedBlob` JSON / `decrypt_chunk_raw`
    /// frame), so this path decrypts the blob directly rather than going through
    /// [`decrypt_downloaded_chunk`].
    ///
    /// ## Zero-knowledge
    ///
    /// The decrypted bytes live ONLY in the returned `Zeroizing<Vec<u8>>`, which
    /// is wiped on drop (normal return, `?`, or panic unwind). Callers decide how
    /// to consume the decoded image bytes: Windows decodes straight to an
    /// in-memory `HBITMAP`; Linux writes the freedesktop.org cache PNG derivative
    /// required by file managers.
    ///
    /// `variant` is `small` / `medium` / `large`. Any error (no thumbnail for
    /// this file, network failure, decrypt failure) bubbles as `Err` so the
    /// caller can fall back to the file-type icon — it is never reported as a
    /// success with empty bytes.
    pub async fn fetch_thumbnail_to_memory(&self, file_id: &str, variant: &str) -> anyhow::Result<Zeroizing<Vec<u8>>> {
        use aes_gcm::aead::Aead;
        use aes_gcm::{Aes256Gcm, KeyInit, Nonce};

        // Validate the id shape up front (same guard as do_hydrate).
        let _file_uuid: uuid::Uuid = file_id
            .parse()
            .map_err(|e| anyhow::anyhow!("invalid file_id (not a UUID): {e}"))?;

        // Pull the encrypted thumbnail blob. A 404 (no thumbnail) is an Err here
        // and the COM caller maps it to the type-icon fallback.
        let blob = self.api.download_thumbnail(file_id, variant).await?;
        if blob.len() < 12 + 16 {
            // Must hold at least a 12-byte nonce + 16-byte GCM tag.
            anyhow::bail!("thumbnail blob too short ({} bytes)", blob.len());
        }

        let file_key = self.file_key_for_download(file_id).await?;

        // Split nonce || ciphertext+tag and AES-256-GCM decrypt with no AAD.
        let (nonce_bytes, ciphertext) = blob.split_at(12);
        let cipher = Aes256Gcm::new_from_slice(file_key.as_bytes())
            .map_err(|e| anyhow::anyhow!("init thumbnail cipher: {e}"))?;
        let nonce = Nonce::from_slice(nonce_bytes);
        let plaintext = cipher
            .decrypt(nonce, ciphertext)
            // The error type carries no plaintext; a generic message avoids
            // leaking anything about the ciphertext.
            .map_err(|_| anyhow::anyhow!("thumbnail decrypt failed (bad key or corrupt blob)"))?;

        // Hand the decoded image bytes back wrapped so they are wiped on drop.
        Ok(Zeroizing::new(plaintext))
    }

    /// Apply a "Keep Mine" resolution: stage the current local bytes and upload
    /// them as a new server version using the same chunked upload path as normal
    /// Finder writes. On success, [`Self::upload_version`] applies the regular
    /// upload completion bookkeeping (status, version, object version, size, and
    /// remote timestamp). On failure, the row is left conflicted and the error is
    /// returned to the caller so the UI does not report a false resolution.
    ///
    /// `sync_root` is supplied by the caller because the bridge itself doesn't
    /// know it (the runner owns that).
    pub async fn resolve_keep_mine(&self, file_id: &str, sync_root: &Path) -> anyhow::Result<()> {
        let entry = self
            .db
            .get_file(file_id)?
            .ok_or_else(|| anyhow::anyhow!("no state.db row for {file_id}"))?;
        if entry.is_dir() {
            return Err(anyhow::anyhow!("Keep Mine upload requires a file row: {file_id}"));
        }

        let shared_contract = self.ensure_item_allows_shared_write(file_id, "keep mine")?;
        let contract = self.db.get_file_contract_state(file_id)?;
        let parent_id = contract.as_ref().and_then(|contract| contract.parent_id.clone());
        let file_name = display_name_for_path(&entry.path);
        let content_type = contract
            .as_ref()
            .and_then(|contract| contract.content_type.clone())
            .or_else(|| beebeeb_core::media::guess_mime_type(&file_name).map(str::to_string));
        let name_encrypted =
            encrypted_metadata_for_name(self.api.master_key(), file_id, &file_name, content_type.as_deref())?;

        let local_path = local_file_path_under_sync_root(sync_root, &entry.path)?;
        let staged_path = stage_finder_payload(&local_path.to_string_lossy())?;
        let staged_size = std::fs::metadata(&staged_path).map(|m| m.len()).unwrap_or(0);

        let mut metadata = serde_json::json!({
            "operation": "upload_version",
            "name_encrypted": name_encrypted,
            "content_type": content_type,
            "size_bytes": staged_size,
            "uploaded_by": "authenticated_desktop_user",
        });
        apply_shared_context(&mut metadata, shared_contract.as_ref());

        let now = now_secs();
        let op = PendingOperation {
            op_id: uuid::Uuid::new_v4().to_string(),
            kind: OperationKind::UploadVersion,
            file_id: Some(file_id.to_string()),
            parent_id,
            target_path: Some(entry.path.clone()),
            metadata_json: Some(serde_json::to_string(&metadata)?),
            payload_path: Some(staged_path.clone()),
            // Keep Mine is an explicit conflict override. Passing the stale base
            // that caused the conflict would make the server reject the upload.
            base_version: None,
            base_object_version_id: None,
            attempts: 0,
            max_attempts: 1,
            next_retry_at: now,
            last_error: None,
            backup_source_key: None,
            created_at: now,
            updated_at: now,
        };

        if let Err(e) = self.upload_version(&op, sync_root).await {
            if let Err(cleanup_error) = std::fs::remove_file(&staged_path) {
                if cleanup_error.kind() != std::io::ErrorKind::NotFound {
                    tracing::warn!(
                        file_id = %file_id,
                        error = %cleanup_error,
                        "failed to remove staged Keep Mine payload after upload error"
                    );
                }
            }
            return Err(anyhow::anyhow!("Keep Mine upload failed: {e}"));
        }

        tracing::info!(file_id = %file_id, "conflict resolved: keep mine uploaded local version");
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
        let dest = local_file_path_under_sync_root(sync_root, &entry.path)?;
        // hydrate_file flips Conflict → Downloading → Local on success
        // (or Error on failure). We don't care about the intermediate
        // state for this path.
        self.hydrate_file(file_id, &dest, &[sync_root]).await?;
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
        let original = local_file_path_under_sync_root(sync_root, &entry.path)?;

        // If the local file no longer exists on disk (user deleted it
        // outside the daemon), Keep Both collapses to Keep Remote.
        if !original.exists() {
            self.hydrate_file(&entry.file_id, &original, &[sync_root]).await?;
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
        if let Err(e) = self.hydrate_file(&entry.file_id, &original, &[sync_root]).await {
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

fn prepare_thumbnail_uploads_for_plaintext_media(
    payload_path: &Path,
    mime_type: Option<&str>,
    file_key: &beebeeb_core::kdf::FileKey,
) -> anyhow::Result<Vec<PreparedThumbnailUpload>> {
    if !beebeeb_core::media::is_media(mime_type) {
        return Ok(Vec::new());
    }

    let source = decode_thumbnail_source(payload_path, mime_type)?;
    let blurhash = if source.is_video {
        None
    } else {
        blurhash_for_source(&source).ok().flatten()
    };

    let mut uploads = Vec::new();
    for variant in THUMBNAIL_VARIANTS_FOR_UPLOAD {
        let config = variant.config();
        let output =
            match beebeeb_core::thumbnail::generate_thumbnail(&source.rgba, source.width, source.height, &config) {
                Ok(output) => output,
                Err(e) => {
                    tracing::warn!(
                        variant = variant.label(),
                        error = %e,
                        "thumbnail generation skipped for variant"
                    );
                    continue;
                }
            };
        let plaintext = Zeroizing::new(output.data);
        let encrypted = Zeroizing::new(
            beebeeb_core::encrypt::encrypt_chunk_raw(file_key, &plaintext)
                .map_err(|e| anyhow::anyhow!("encrypt thumbnail {}: {e}", variant.label()))?,
        );
        if encrypted.len() > variant.encrypted_max_bytes() {
            tracing::warn!(
                variant = variant.label(),
                bytes = encrypted.len(),
                max_bytes = variant.encrypted_max_bytes(),
                "thumbnail generation skipped because encrypted payload is too large"
            );
            continue;
        }
        uploads.push(PreparedThumbnailUpload {
            variant,
            encrypted,
            blurhash: if variant == ThumbnailUploadVariant::Medium {
                blurhash.clone()
            } else {
                None
            },
        });
    }

    Ok(uploads)
}

fn decode_thumbnail_source(payload_path: &Path, mime_type: Option<&str>) -> anyhow::Result<ThumbnailSource> {
    match mime_type {
        Some(mime) if mime.starts_with("video/") => decode_video_thumbnail_source(payload_path),
        _ => decode_image_thumbnail_source(payload_path),
    }
}

fn decode_image_thumbnail_source(payload_path: &Path) -> anyhow::Result<ThumbnailSource> {
    let image = image::ImageReader::open(payload_path)?
        .with_guessed_format()?
        .decode()
        .map_err(|e| anyhow::anyhow!("decode image for thumbnail: {e}"))?;
    dynamic_image_to_thumbnail_source(image, false)
}

fn decode_video_thumbnail_source(payload_path: &Path) -> anyhow::Result<ThumbnailSource> {
    let mut last_error: Option<anyhow::Error> = None;
    for seek in ["1", "0"] {
        match decode_video_frame_with_ffmpeg(payload_path, seek) {
            Ok(source) => return Ok(source),
            Err(e) => last_error = Some(e),
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("video thumbnail extraction failed")))
}

fn decode_video_frame_with_ffmpeg(payload_path: &Path, seek: &str) -> anyhow::Result<ThumbnailSource> {
    let ffmpeg = std::env::var_os("BEEBEEB_FFMPEG_PATH").unwrap_or_else(|| "ffmpeg".into());
    let output = Command::new(ffmpeg)
        .arg("-nostdin")
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-ss")
        .arg(seek)
        .arg("-i")
        .arg(payload_path)
        .arg("-frames:v")
        .arg("1")
        .arg("-f")
        .arg("image2pipe")
        .arg("-vcodec")
        .arg("png")
        .arg("-")
        .output()
        .map_err(|e| anyhow::anyhow!("spawn ffmpeg for video thumbnail: {e}"))?;

    if !output.status.success() || output.stdout.is_empty() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let snippet: String = stderr.chars().take(240).collect();
        anyhow::bail!("ffmpeg video thumbnail failed: {snippet}");
    }

    let frame_png = Zeroizing::new(output.stdout);
    let image =
        image::load_from_memory(&frame_png).map_err(|e| anyhow::anyhow!("decode ffmpeg thumbnail frame: {e}"))?;
    dynamic_image_to_thumbnail_source(image, true)
}

fn dynamic_image_to_thumbnail_source(image: image::DynamicImage, is_video: bool) -> anyhow::Result<ThumbnailSource> {
    let rgba = image.into_rgba8();
    let width = rgba.width();
    let height = rgba.height();
    if width == 0 || height == 0 {
        anyhow::bail!("thumbnail source has zero dimensions");
    }
    Ok(ThumbnailSource {
        rgba: Zeroizing::new(rgba.into_raw()),
        width,
        height,
        is_video,
    })
}

fn blurhash_for_source(source: &ThumbnailSource) -> anyhow::Result<Option<String>> {
    let (small_rgba, width, height) = beebeeb_core::thumbnail::resize_for_thumbnail(
        &source.rgba,
        source.width,
        source.height,
        BLURHASH_SOURCE_MAX_DIMENSION,
    )
    .map_err(|e| anyhow::anyhow!("resize blurhash source: {e}"))?;
    let small_rgba = Zeroizing::new(small_rgba);
    Ok(encode_blurhash_rgba(
        &small_rgba,
        width,
        height,
        BLURHASH_COMPONENTS_X,
        BLURHASH_COMPONENTS_Y,
    ))
}

fn encode_blurhash_rgba(
    rgba: &[u8],
    width: u32,
    height: u32,
    components_x: usize,
    components_y: usize,
) -> Option<String> {
    if width == 0
        || height == 0
        || components_x == 0
        || components_x > 9
        || components_y == 0
        || components_y > 9
        || rgba.len() != width as usize * height as usize * 4
    {
        return None;
    }

    let mut factors = Vec::with_capacity(components_x * components_y);
    for y in 0..components_y {
        for x in 0..components_x {
            factors.push(multiply_blurhash_basis(rgba, width, height, x, y));
        }
    }

    let size_flag = (components_x - 1) + (components_y - 1) * 9;
    let mut encoded = String::with_capacity(4 + 2 * factors.len());
    encoded.push_str(&encode_base83(size_flag as u32, 1));

    let maximum_value = if factors.len() > 1 {
        let actual_max = factors[1..]
            .iter()
            .flat_map(|factor| factor.iter())
            .fold(0.0_f64, |max, value| max.max(value.abs()));
        let quantized = ((actual_max * 166.0 - 0.5).floor() as i32).clamp(0, 82) as u32;
        encoded.push_str(&encode_base83(quantized, 1));
        (quantized + 1) as f64 / 166.0
    } else {
        encoded.push_str(&encode_base83(0, 1));
        1.0
    };

    encoded.push_str(&encode_base83(encode_blurhash_dc(factors[0]), 4));
    for factor in factors.iter().skip(1) {
        encoded.push_str(&encode_base83(encode_blurhash_ac(*factor, maximum_value), 2));
    }

    if encoded.len() <= 64 { Some(encoded) } else { None }
}

fn multiply_blurhash_basis(rgba: &[u8], width: u32, height: u32, component_x: usize, component_y: usize) -> [f64; 3] {
    let normalisation = if component_x == 0 && component_y == 0 { 1.0 } else { 2.0 };
    let width_f = width as f64;
    let height_f = height as f64;
    let mut r = 0.0;
    let mut g = 0.0;
    let mut b = 0.0;

    for y in 0..height {
        for x in 0..width {
            let basis = (std::f64::consts::PI * component_x as f64 * x as f64 / width_f).cos()
                * (std::f64::consts::PI * component_y as f64 * y as f64 / height_f).cos();
            let idx = ((y * width + x) as usize) * 4;
            let alpha = rgba[idx + 3] as f64 / 255.0;
            let sr = (rgba[idx] as f64 / 255.0) * alpha + (1.0 - alpha);
            let sg = (rgba[idx + 1] as f64 / 255.0) * alpha + (1.0 - alpha);
            let sb = (rgba[idx + 2] as f64 / 255.0) * alpha + (1.0 - alpha);
            r += basis * srgb_to_linear(sr);
            g += basis * srgb_to_linear(sg);
            b += basis * srgb_to_linear(sb);
        }
    }

    let scale = normalisation / (width_f * height_f);
    [r * scale, g * scale, b * scale]
}

fn srgb_to_linear(value: f64) -> f64 {
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(value: f64) -> u32 {
    let value = value.clamp(0.0, 1.0);
    let srgb = if value <= 0.0031308 {
        value * 12.92
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    };
    (srgb * 255.0 + 0.5).floor().clamp(0.0, 255.0) as u32
}

fn encode_blurhash_dc(value: [f64; 3]) -> u32 {
    (linear_to_srgb(value[0]) << 16) + (linear_to_srgb(value[1]) << 8) + linear_to_srgb(value[2])
}

fn encode_blurhash_ac(value: [f64; 3], maximum_value: f64) -> u32 {
    let quant_r = quantize_blurhash_ac(value[0], maximum_value);
    let quant_g = quantize_blurhash_ac(value[1], maximum_value);
    let quant_b = quantize_blurhash_ac(value[2], maximum_value);
    quant_r * 19 * 19 + quant_g * 19 + quant_b
}

fn quantize_blurhash_ac(value: f64, maximum_value: f64) -> u32 {
    let normalized = if maximum_value > 0.0 {
        value / maximum_value
    } else {
        0.0
    };
    (sign_pow(normalized.clamp(-1.0, 1.0), 0.5) * 9.0 + 9.5)
        .floor()
        .clamp(0.0, 18.0) as u32
}

fn sign_pow(value: f64, exp: f64) -> f64 {
    value.abs().powf(exp).copysign(value)
}

fn encode_base83(mut value: u32, length: usize) -> String {
    let mut chars = vec![0u8; length];
    for i in (0..length).rev() {
        chars[i] = BLURHASH_BASE83[(value % 83) as usize];
        value /= 83;
    }
    String::from_utf8(chars).expect("base83 alphabet is ASCII")
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

/// The result of [`EngineBridge::hydrate_file_range`]: the decrypted plaintext
/// covering the requested range, plus enough context for the caller to splice
/// out the exact `[required_offset, required_offset + required_length)` span.
#[cfg(any(target_os = "windows", test))]
pub struct HydratedRange {
    /// Decrypted plaintext for chunks `[first_chunk, last_chunk]` — i.e. the
    /// smallest chunk-aligned span that covers the requested byte range. This
    /// is NOT necessarily aligned to the requested range itself; the caller
    /// must slice `data[required_offset - range_start_bytes ..]`.
    pub data: Zeroizing<Vec<u8>>,
    /// Absolute byte offset (into the full file) of `data[0]` — i.e.
    /// `first_chunk * chunk_size_bytes`.
    pub range_start_bytes: u64,
    /// `true` when `data` happens to cover the ENTIRE file (the covering-chunk
    /// range was `[0, chunk_count)`). Only then is a range hydration equivalent
    /// to a whole-file hydration for status-bookkeeping / resolve-or-error
    /// purposes.
    pub covers_whole_file: bool,
}

#[cfg(any(target_os = "windows", test))]
struct HydrationChunkRangePlan {
    first_chunk: u32,
    last_chunk_exclusive: u32,
    range_start_bytes: u64,
    covering_span_hint: usize,
    covers_whole_file: bool,
}

#[cfg(any(target_os = "windows", test))]
fn chunk_layout_matches_metadata(size_bytes: u64, chunk_count: u64, chunk_size_bytes: u64) -> bool {
    if chunk_count == 0 || chunk_size_bytes == 0 {
        return false;
    }

    let size = size_bytes as u128;
    let count = chunk_count as u128;
    let chunk = chunk_size_bytes as u128;
    if count == 1 {
        return size <= chunk;
    }

    let full_prefix = (count - 1) * chunk;
    let full_span = count * chunk;
    size > full_prefix && size <= full_span
}

#[cfg(any(target_os = "windows", test))]
fn plan_hydration_chunk_range(
    size_bytes: u64,
    chunk_count: u64,
    chunk_size_bytes: u64,
    required_offset: u64,
    required_length: u64,
) -> anyhow::Result<Option<HydrationChunkRangePlan>> {
    if !chunk_layout_matches_metadata(size_bytes, chunk_count, chunk_size_bytes) {
        return Ok(None);
    }

    let range_end = required_offset.saturating_add(required_length).min(size_bytes);
    if range_end <= required_offset {
        return Ok(None);
    }

    let first_chunk = required_offset / chunk_size_bytes;
    let last_chunk = (range_end - 1) / chunk_size_bytes;
    if last_chunk >= chunk_count {
        return Ok(None);
    }

    let first_chunk_u32 =
        u32::try_from(first_chunk).map_err(|_| anyhow::anyhow!("chunk index {first_chunk} exceeds u32 range"))?;
    let last_chunk_u32 =
        u32::try_from(last_chunk).map_err(|_| anyhow::anyhow!("chunk index {last_chunk} exceeds u32 range"))?;
    let last_chunk_exclusive = last_chunk_u32
        .checked_add(1)
        .ok_or_else(|| anyhow::anyhow!("chunk range end exceeds u32 range"))?;
    let chunk_size_usize = usize::try_from(chunk_size_bytes)
        .map_err(|_| anyhow::anyhow!("chunk_size_bytes {chunk_size_bytes} exceeds usize range"))?;
    let chunk_count_in_range = usize::try_from(last_chunk_u32 - first_chunk_u32 + 1)
        .map_err(|_| anyhow::anyhow!("covering chunk count exceeds usize range"))?;
    let covering_span_hint = chunk_count_in_range
        .checked_mul(chunk_size_usize)
        .ok_or_else(|| anyhow::anyhow!("covering range size exceeds usize range"))?;

    Ok(Some(HydrationChunkRangePlan {
        first_chunk: first_chunk_u32,
        last_chunk_exclusive,
        range_start_bytes: first_chunk * chunk_size_bytes,
        covering_span_hint,
        covers_whole_file: first_chunk == 0 && last_chunk == chunk_count - 1,
    }))
}

fn stage_finder_payload(contents_path: &str) -> anyhow::Result<String> {
    stage_finder_payload_with_root(contents_path, default_finder_staging_root())
}

fn local_file_path_under_sync_root(sync_root: &Path, rel_path: &str) -> anyhow::Result<PathBuf> {
    crate::reject_unsafe_rel_path(rel_path)
        .map_err(|e| anyhow::anyhow!("local path must stay under the sync root: {e}"))?;
    Ok(sync_root.join(rel_path.replace('/', std::path::MAIN_SEPARATOR_STR)))
}

/// Task 1247: is `dest_path` a safe hydration target — i.e. inside (or about to
/// be created inside) at least one of the caller's trusted `allowed_roots`?
///
/// Pure and free-standing so the containment decision is directly unit-testable
/// without a live `EngineBridge`. Mirrors the two-layer containment pattern
/// already used by `lib.rs::reveal_and_open_file` (see its documented 3-layer
/// comment block):
///   - If `dest_path` already exists, canonicalize both and require containment.
///   - If it does not exist yet (the normal hydrate case — we are about to
///     create it), require the PARENT directory to be contained AND the file
///     name to be a single normal component with no embedded separator, so a
///     traversal like `<root>/../evil` cannot slip past the parent check.
///
/// Returns true if ANY root passes; false if none do. An empty `allowed_roots`
/// slice returns false (fail-closed), never vacuously true.
fn hydrate_dest_is_allowed(dest_path: &Path, allowed_roots: &[&Path]) -> bool {
    for &root in allowed_roots {
        let ok = if dest_path.exists() {
            crate::is_contained(root, dest_path)
        } else {
            let parent_ok = dest_path
                .parent()
                .map(|p| crate::is_contained(root, p))
                .unwrap_or(false);
            let name_ok = dest_path
                .file_name()
                .map(|n| {
                    let s = n.to_string_lossy();
                    !s.contains('/') && !s.contains('\\')
                })
                .unwrap_or(false);
            parent_ok && name_ok
        };
        if ok {
            return true;
        }
    }
    false
}

/// Task 1247: write hydrated plaintext to `dest_path`, failing closed if the
/// final path component is (or race-becomes) a symlink.
///
/// This is the atomic half of the destination defense: `hydrate_dest_is_allowed`
/// validates the path *before* the network download/decrypt, but the write only
/// happens *after* — a real TOCTOU window in which a same-UID attacker could
/// swap `dest_path` for a symlink to an unauthorized target (e.g.
/// `~/.ssh/authorized_keys`). Opening with `O_NOFOLLOW` makes the kernel refuse
/// to follow a symlink at open() time, so the swap fails the write instead of
/// redirecting decrypted plaintext outside the allowed root — no re-check race.
///
/// Unix also forces the file to `0o600` (owner-only) via `fchmod` so freshly
/// decrypted vault plaintext is never world-readable, even under a
/// world-traversable `/tmp`. `fchmod` (not the `O_CREAT` mode arg) is used
/// deliberately: POSIX applies the open-time mode ONLY to a newly created inode,
/// so a pre-existing (e.g. attacker-planted `0o644`) file at a legitimately
/// in-root path would otherwise keep its permissions and leak the plaintext to
/// other local users — a deterministic bypass with no race needed.
///
/// Defense against a parent-directory swap (task 1247, second review): the leaf
/// `O_NOFOLLOW` alone only refuses a symlink at the FINAL component; an attacker
/// could pass containment with a real subdir, then swap that subdir for a
/// symlink during the download/decrypt window. So the parent directory is opened
/// with `O_DIRECTORY|O_NOFOLLOW` (fails closed if the parent's own basename is a
/// symlink) and the leaf is created/written via `openat` RELATIVE to that dir
/// fd — anchoring the whole write to the directory inode that was
/// containment-checked, so a later parent swap cannot redirect it. Containment
/// is re-verified against the freshly-resolved path right before the open, since
/// the top-of-`hydrate_file` guard ran before `do_hydrate`.
///
/// Non-unix keeps `std::fs::write` (`O_NOFOLLOW`/`openat`/`fchmod` are Unix-only,
/// and the Windows Cloud Files path never writes plaintext to disk via this fn —
/// it uses `hydrate_file_to_memory`).
///
/// Known residual (documented follow-up, not closed here): a same-filesystem
/// HARD link to a file outside all allowed roots bypasses containment
/// (canonicalize cannot see hard links), and `O_TRUNC` would overwrite the
/// linked inode. Noted in the task file.
#[cfg(unix)]
fn write_hydrated_plaintext(dest_path: &Path, allowed_roots: &[&Path], buf: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::io::{AsRawFd, FromRawFd, OwnedFd};

    let parent = match dest_path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    let leaf = dest_path
        .file_name()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "hydrate destination has no file name"))?;

    // Re-verify containment right before the anchored open. canonicalize()
    // resolves symlinks, so a parent/ancestor swapped to point outside an
    // allowed root during the do_hydrate window is caught here.
    if !hydrate_dest_is_allowed(dest_path, allowed_roots) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "hydrate destination no longer within an allowed root",
        ));
    }

    // Anchor to the parent directory inode: O_DIRECTORY|O_NOFOLLOW fails closed
    // (ELOOP) if the parent's basename is currently a symlink, and the resulting
    // fd tracks that exact inode so it cannot be swapped out from under us.
    let dir_c = path_to_cstring(parent.as_os_str())?;
    let dir_raw = unsafe { libc::open(dir_c.as_ptr(), libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC) };
    if dir_raw < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let dir_fd = unsafe { OwnedFd::from_raw_fd(dir_raw) };

    // Create/open the leaf RELATIVE to the anchored dir fd, still refusing to
    // follow a symlink at the leaf itself.
    let leaf_c = path_to_cstring(leaf)?;
    let file_raw = unsafe {
        libc::openat(
            dir_fd.as_raw_fd(),
            leaf_c.as_ptr(),
            libc::O_NOFOLLOW | libc::O_CREAT | libc::O_WRONLY | libc::O_TRUNC | libc::O_CLOEXEC,
            0o600 as libc::c_int,
        )
    };
    if file_raw < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let file_fd = unsafe { OwnedFd::from_raw_fd(file_raw) };

    // Force 0o600 on the fd unconditionally (covers the pre-existing-file case
    // the O_CREAT mode misses), BEFORE any plaintext is written.
    if unsafe { libc::fchmod(file_fd.as_raw_fd(), 0o600 as libc::mode_t) } != 0 {
        return Err(std::io::Error::last_os_error());
    }

    let mut file = std::fs::File::from(file_fd);
    file.write_all(buf)
}

#[cfg(not(unix))]
fn write_hydrated_plaintext(dest_path: &Path, _allowed_roots: &[&Path], buf: &[u8]) -> std::io::Result<()> {
    std::fs::write(dest_path, buf)
}

/// Convert an `OsStr` path component to a NUL-terminated `CString` for the raw
/// `libc::open`/`openat` calls in `write_hydrated_plaintext`.
#[cfg(unix)]
fn path_to_cstring(s: &std::ffi::OsStr) -> std::io::Result<std::ffi::CString> {
    use std::os::unix::ffi::OsStrExt;
    std::ffi::CString::new(s.as_bytes())
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "path contains an interior NUL byte"))
}

#[cfg(target_os = "linux")]
fn linux_thumbnail_source_path_for_entry(entry: &FileEntry) -> Option<PathBuf> {
    let sync_root = crate::config::DesktopConfig::load().ok()?.sync_root?;
    crate::linux_thumbnail::source_path_under_sync_root(&sync_root, &entry.path)
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
    let primary = dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("beebeeb")
        .join("finder-writes");
    match verify_staging_root_writable(&primary) {
        Ok(()) => primary,
        Err(e) => {
            let fallback = std::env::temp_dir().join("beebeeb").join("finder-writes");
            tracing::warn!(
                path = %primary.display(),
                fallback = %fallback.display(),
                error = %e,
                "Finder staging cache root is unavailable; falling back to temp directory"
            );
            fallback
        }
    }
}

fn verify_staging_root_writable(root: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(root)?;
    let probe = root.join(format!(".beebeeb-write-test-{}", uuid::Uuid::new_v4()));
    std::fs::write(&probe, b"")?;
    let _ = std::fs::remove_file(probe);
    Ok(())
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

fn record_moved_to_trash_activity(db: &StateDb, file_id: &str, rel_path: &str, occurred_at: i64) -> anyhow::Result<()> {
    db.record_local_activity(LocalActivityEventInput {
        event_type: LocalActivityKind::MovedToTrash,
        file_id: Some(file_id.to_string()),
        file_name: display_name_for_path(rel_path),
        rel_path: Some(rel_path.to_string()),
        occurred_at,
    })?;
    Ok(())
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
    // Classify on the failure MESSAGE only, never on the request URL. reqwest's
    // Display appends `" for url (…)"` (for both HTTP-status and connection
    // errors), and that URL carries the random ephemeral port plus opaque path
    // ids. Those digits are not status codes, but the bare-code substring checks
    // below ("401", "403") would happily match them — so a plain HTTP 500 sent
    // to, say, `127.0.0.1:45401` was being misread as an Auth failure and
    // *paused* (never retried). That is the entire flake in task 1252: it fired
    // only when the OS handed the mock server a port whose digits contained
    // "401"/"403", which is why it reproduced in CI but ~never locally. Dropping
    // the URL suffix keeps classification keyed on reqwest's status reason phrase
    // ("401 Unauthorized", "403 Forbidden", "507 Insufficient Storage", …) and
    // on our own descriptive errors, none of which live past `" for url ("`.
    let message = error.split(" for url (").next().unwrap_or(error);
    let lower = message.to_ascii_lowercase();
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

fn shared_invite_id_matches(invite: &serde_json::Value, share_id: &str) -> bool {
    string_field(invite, &["id", "invite_id"]).as_deref() == Some(share_id)
}

fn shared_root_from_invite(invite: &serde_json::Value) -> Option<SharedRootMapping> {
    if invite.get("status").and_then(|value| value.as_str()) != Some("approved") {
        return None;
    }

    let invite_id = string_field(invite, &["id", "invite_id"])?;
    let file_id = string_field(invite, &["file_id"])?;
    let is_folder = invite
        .get("is_folder_share")
        .and_then(|value| value.as_bool())
        .or_else(|| invite.get("is_folder").and_then(|value| value.as_bool()))
        .unwrap_or(false);
    let item_kind = if is_folder { ItemKind::Folder } else { ItemKind::File };
    let display_name = shared_placeholder_name(&item_kind).to_string();
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
        owner_email: string_field(invite, &["sender_email", "owner_email", "shared_by"]),
        sender_public_key: string_field(invite, &["sender_public_key", "owner_public_key"]),
        encrypted_file_key: string_field(invite, &["encrypted_file_key", "wrapped_file_key"]),
        encrypted_folder_key: string_field(invite, &["encrypted_folder_key"]),
        file_name_encrypted: string_field(invite, &["file_name_encrypted", "name_encrypted"]),
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
    ["permission", "capability", "permissions", "role", "access"]
        .iter()
        .filter_map(|key| invite.get(*key).and_then(|value| value.as_str()))
        .any(|value| {
            matches!(
                value.to_ascii_lowercase().as_str(),
                "write" | "edit" | "editable" | "editor" | "admin" | "owner"
            )
        })
}

fn decode_standard_b64(label: &str, value: &str) -> anyhow::Result<Vec<u8>> {
    base64::engine::general_purpose::STANDARD
        .decode(value)
        .map_err(|e| anyhow::anyhow!("{label} is not valid standard base64: {e}"))
}

fn decode_x25519_public_key(label: &str, value: &str) -> anyhow::Result<[u8; 32]> {
    let bytes = decode_standard_b64(label, value)?;
    if bytes.len() != 32 {
        anyhow::bail!("{label} must be 32 bytes, got {}", bytes.len());
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&bytes);
    Ok(out)
}

fn derive_recipient_share_key(
    recipient_master_key: &[u8; 32],
    sender_public_key_b64: &str,
    file_id: &str,
) -> anyhow::Result<Zeroizing<[u8; 32]>> {
    let master_key = beebeeb_core::kdf::MasterKey::from_bytes(*recipient_master_key);
    let recipient_private = beebeeb_core::opaque::derive_x25519_private(&master_key);
    let sender_public = decode_x25519_public_key("sender_public_key", sender_public_key_b64)?;
    let shared_secret = beebeeb_core::opaque::x25519_shared_secret(&recipient_private, &sender_public)
        .map_err(|e| anyhow::anyhow!("derive X25519 shared secret: {e}"))?;
    Ok(beebeeb_core::opaque::derive_share_key(
        &shared_secret,
        file_id.as_bytes(),
    ))
}

fn unwrap_key_frame(wrap_key: &[u8; 32], encrypted_key_b64: &str, label: &str) -> anyhow::Result<Zeroizing<[u8; 32]>> {
    let raw = decode_standard_b64(label, encrypted_key_b64)?;
    let frame_key = beebeeb_core::kdf::FileKey::from_bytes(*wrap_key);
    let plaintext = Zeroizing::new(
        beebeeb_core::encrypt::decrypt_chunk_raw(&frame_key, &raw)
            .map_err(|_| anyhow::anyhow!("{label} decrypt failed"))?,
    );
    if plaintext.len() != 32 {
        let len = plaintext.len();
        anyhow::bail!("{label} decrypted key must be 32 bytes, got {len}");
    }
    let mut out = Zeroizing::new([0u8; 32]);
    out.copy_from_slice(&plaintext);
    Ok(out)
}

fn unwrap_direct_shared_file_key(
    recipient_master_key: &[u8; 32],
    sender_public_key_b64: &str,
    file_id: &str,
    encrypted_file_key_b64: &str,
) -> anyhow::Result<beebeeb_core::kdf::FileKey> {
    let share_key = derive_recipient_share_key(recipient_master_key, sender_public_key_b64, file_id)?;
    let file_key = unwrap_key_frame(&share_key, encrypted_file_key_b64, "encrypted_file_key")?;
    Ok(beebeeb_core::kdf::FileKey::from_bytes(*file_key))
}

fn unwrap_folder_share_key(
    recipient_master_key: &[u8; 32],
    sender_public_key_b64: &str,
    folder_id: &str,
    encrypted_folder_key_b64: &str,
) -> anyhow::Result<Zeroizing<[u8; 32]>> {
    let share_key = derive_recipient_share_key(recipient_master_key, sender_public_key_b64, folder_id)?;
    unwrap_key_frame(&share_key, encrypted_folder_key_b64, "encrypted_folder_key")
}

fn encrypted_file_key_from_folder_keys_response<'a>(
    folder_keys_response: &'a serde_json::Value,
    file_id: &str,
) -> Option<&'a str> {
    folder_keys_response
        .get("keys")
        .and_then(|value| value.as_array())
        .into_iter()
        .flatten()
        .find(|entry| entry.get("file_id").and_then(|value| value.as_str()) == Some(file_id))
        .and_then(|entry| entry.get("encrypted_file_key").and_then(|value| value.as_str()))
}

fn unwrap_child_file_key(
    folder_key: &[u8; 32],
    encrypted_file_key_b64: &str,
) -> anyhow::Result<beebeeb_core::kdf::FileKey> {
    let file_key = unwrap_key_frame(folder_key, encrypted_file_key_b64, "encrypted_file_key")?;
    Ok(beebeeb_core::kdf::FileKey::from_bytes(*file_key))
}

fn unwrap_folder_share_file_key(
    recipient_master_key: &[u8; 32],
    sender_public_key_b64: &str,
    folder_id: &str,
    encrypted_folder_key_b64: &str,
    file_id: &str,
    folder_keys_response: &serde_json::Value,
) -> anyhow::Result<beebeeb_core::kdf::FileKey> {
    let folder_key = unwrap_folder_share_key(
        recipient_master_key,
        sender_public_key_b64,
        folder_id,
        encrypted_folder_key_b64,
    )?;
    let encrypted_file_key = encrypted_file_key_from_folder_keys_response(folder_keys_response, file_id)
        .ok_or_else(|| anyhow::anyhow!("folder share is missing encrypted_file_key for {file_id}"))?;
    unwrap_child_file_key(&folder_key, encrypted_file_key)
}

fn folder_keys_map(folder_keys_response: &serde_json::Value) -> HashMap<String, String> {
    folder_keys_response
        .get("keys")
        .and_then(|value| value.as_array())
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            Some((
                entry.get("file_id")?.as_str()?.to_string(),
                entry.get("encrypted_file_key")?.as_str()?.to_string(),
            ))
        })
        .collect()
}

fn json_timestamp_secs(value: &serde_json::Value, keys: &[&str]) -> Option<i64> {
    keys.iter().find_map(|key| {
        let field = value.get(*key)?;
        field.as_i64().or_else(|| field.as_str().and_then(parse_rfc3339_secs))
    })
}

fn item_kind_from_metadata(f: &serde_json::Value) -> ItemKind {
    if f["is_folder"].as_bool().unwrap_or(false)
        || f["kind"].as_str() == Some("folder")
        || f["type"].as_str() == Some("folder")
    {
        ItemKind::Folder
    } else {
        ItemKind::File
    }
}

fn shared_placeholder_name(item_kind: &ItemKind) -> &'static str {
    match item_kind {
        ItemKind::Folder => "Encrypted folder",
        ItemKind::File => "Encrypted file",
    }
}

fn safe_shared_leaf_name(name: &str) -> Option<String> {
    let leaf = name.trim();
    if crate::reject_unsafe_rel_path(leaf).is_err() || leaf.contains('/') || leaf.contains('\\') {
        return None;
    }
    if leaf.is_empty() {
        return None;
    }
    Some(leaf.to_string())
}

fn metadata_name_from_plaintext(plaintext: &str) -> String {
    serde_json::from_str::<serde_json::Value>(plaintext)
        .ok()
        .and_then(|value| value.get("name").and_then(|name| name.as_str()).map(str::to_string))
        .unwrap_or_else(|| plaintext.to_string())
}

fn decrypt_shared_name_with_key(file_key: &beebeeb_core::kdf::FileKey, name_encrypted: &str) -> Option<String> {
    if let Ok(blob) = serde_json::from_str::<EncryptedBlob>(name_encrypted)
        && let Ok(plaintext) = beebeeb_core::encrypt::decrypt_metadata(file_key, &blob)
    {
        return Some(metadata_name_from_plaintext(&plaintext));
    }

    if let Ok((nonce, ciphertext)) = beebeeb_core::metadata_wire::parse_encrypted_metadata(name_encrypted) {
        let blob = EncryptedBlob {
            cipher_suite: CipherSuite::V1Aes256Gcm,
            nonce,
            ciphertext,
        };
        if let Ok(plaintext) = beebeeb_core::encrypt::decrypt_metadata(file_key, &blob) {
            return Some(metadata_name_from_plaintext(&plaintext));
        }
    }

    None
}

fn shared_name_from_metadata(
    f: &serde_json::Value,
    item_kind: &ItemKind,
    file_key: Option<&beebeeb_core::kdf::FileKey>,
) -> String {
    if let (Some(key), Some(name_encrypted)) = (file_key, f["name_encrypted"].as_str()) {
        if let Some(name) = decrypt_shared_name_with_key(key, name_encrypted) {
            if let Some(leaf) = safe_shared_leaf_name(&name) {
                return leaf;
            }
        }
    }

    shared_placeholder_name(item_kind).to_string()
}

fn apply_shared_metadata_file_row(
    db: &StateDb,
    f: &serde_json::Value,
    root: &SharedRootMapping,
    now: i64,
    parent_id: Option<String>,
    parent_rel_path: &str,
    file_key: Option<&beebeeb_core::kdf::FileKey>,
) -> anyhow::Result<Option<FileEntry>> {
    let file_id = f["id"].as_str().unwrap_or_default();
    if file_id.is_empty() {
        return Ok(None);
    }

    let existing = db.get_file(file_id)?;
    let size = f["size_bytes"].as_i64().or_else(|| f["size"].as_i64()).unwrap_or(0);
    let remote_updated = json_timestamp_secs(f, &["updated_at", "uploaded_at", "created_at"]).unwrap_or(now);
    let item_kind = item_kind_from_metadata(f);
    let leaf = shared_name_from_metadata(f, &item_kind, file_key);
    let path = if parent_rel_path.is_empty() {
        leaf
    } else {
        format!("{}/{}", parent_rel_path.trim_end_matches('/'), leaf)
    };
    crate::reject_unsafe_rel_path(&path).map_err(|e| anyhow::anyhow!("unsafe shared item path for {file_id}: {e}"))?;
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
        parent_id: parent_id.clone(),
        item_kind: item_kind.clone(),
    };
    db.upsert_file(&entry)?;

    let mut contract = db
        .get_file_contract_state(file_id)?
        .ok_or_else(|| anyhow::anyhow!("missing state row for shared item {file_id}"))?;
    contract.namespace = Namespace::SharedWithMe;
    contract.parent_id = parent_id;
    contract.shared_root_id = Some(root.file_id.clone());
    contract.share_id = Some(root.invite_id.clone());
    contract.owner_email = root.owner_email.clone();
    contract.permission_bits = root.permission_bits;
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
    contract.last_sync_at = now;
    db.set_file_contract_state(&contract)?;
    Ok(Some(entry))
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
        format!(
            "{}/{}",
            parent_rel_path.trim_end_matches('/'),
            leaf.trim_start_matches('/')
        )
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
            owner_email: None,
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
        let parent_rel_path = match f["parent_id"].as_str().filter(|pid| !pid.is_empty()) {
            Some(pid) => resolved_paths.get(pid).cloned().unwrap_or_else(|| {
                // If the parent is missing from this snapshot but the child already
                // exists locally, keep its current parent prefix for this ingest.
                // A concurrent folder trash can legitimately produce a partial
                // snapshot that still lists the folder's children but not the folder;
                // re-rooting those children before `prune_absent` runs would hide
                // them from the absent-folder subtree sweep.
                let existing_parent = existing_parent_rel_path(bridge, file_id);
                if !existing_parent.is_empty() {
                    tracing::debug!(
                        file_id = %file_id,
                        parent_id = %pid,
                        parent_rel_path = %existing_parent,
                        "sync_tick: preserving existing parent path for snapshot node whose parent is absent"
                    );
                }
                existing_parent
            }),
            None => String::new(),
        };

        if let Some((rel_path, _kind)) = process_metadata_row(bridge, f, &parent_rel_path, now_secs, conflicts)? {
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
                // No row for the folder itself — the folder was absent from
                // this desktop's snapshot (already trashed when the snapshot
                // ran), but its CHILDREN may have been ingested and stored with
                // `parent_id = id`.  Sweep those orphaned children by
                // `parent_id` so they don't linger as ghost placeholders in
                // Explorer (task 0828).  The sweep is scoped strictly to
                // descendants of `id` and is a no-op when none are found.
                None => {
                    let orphans = bridge.db().delete_orphaned_children_of_absent_folder(id)?;
                    if !orphans.is_empty() {
                        tracing::info!(
                            folder_id = %id,
                            count = orphans.len(),
                            "sync_tick: trashed folder had no local row but \
                             {} orphaned child(ren) — pruned (task 0828)",
                            orphans.len()
                        );
                        remove_pruned_placeholders(sync_root, &orphans);
                    }
                }
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
    use base64::Engine;
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
        assert!(path_is_engine_internal(
            &root,
            &root.join("sub").join(".beebeeb").join("x")
        ));
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
        assert_eq!(relative_db_path(&root, &std::path::PathBuf::from("/other/c.txt")), None);
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

    fn write_test_png(path: &Path) {
        let image = image::RgbaImage::from_fn(8, 6, |x, y| {
            image::Rgba([(x * 31) as u8, (y * 41) as u8, ((x + y) * 17) as u8, 255])
        });
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image)
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        std::fs::write(path, png.into_inner()).unwrap();
    }

    #[test]
    fn test_prepare_thumbnail_uploads_encrypts_medium_and_large_with_blurhash() {
        let dir = tempfile::tempdir().unwrap();
        let payload = dir.path().join("photo.png");
        write_test_png(&payload);

        let master_key = beebeeb_core::kdf::MasterKey::from_bytes([31u8; 32]);
        let file_key = beebeeb_core::kdf::derive_file_key(&master_key, b"server-file-1");
        let uploads = prepare_thumbnail_uploads_for_plaintext_media(&payload, Some("image/png"), &file_key).unwrap();

        assert_eq!(uploads.len(), 2);
        assert_eq!(uploads[0].variant, ThumbnailUploadVariant::Medium);
        assert_eq!(uploads[1].variant, ThumbnailUploadVariant::Large);
        assert!(uploads[0].blurhash.as_ref().is_some_and(|hash| hash.len() <= 64));
        assert!(uploads[1].blurhash.is_none());

        for upload in &uploads {
            assert!(upload.encrypted.len() <= upload.variant.encrypted_max_bytes());
            let plaintext = beebeeb_core::encrypt::decrypt_chunk_raw(&file_key, &upload.encrypted).unwrap();
            assert!(plaintext.starts_with(b"RIFF"), "thumbnail plaintext must be WebP");
            assert_ne!(&*upload.encrypted, plaintext.as_slice());
        }
    }

    #[test]
    fn test_prepare_thumbnail_uploads_extracts_video_frame_when_ffmpeg_available() {
        if std::process::Command::new("ffmpeg").arg("-version").output().is_err() {
            return;
        }

        let dir = tempfile::tempdir().unwrap();
        let payload = dir.path().join("clip.mp4");
        let status = std::process::Command::new("ffmpeg")
            .arg("-nostdin")
            .arg("-hide_banner")
            .arg("-loglevel")
            .arg("error")
            .arg("-y")
            .arg("-f")
            .arg("lavfi")
            .arg("-i")
            .arg("testsrc=size=16x16:rate=1")
            .arg("-t")
            .arg("1")
            .arg("-pix_fmt")
            .arg("yuv420p")
            .arg(&payload)
            .status()
            .unwrap();
        if !status.success() {
            return;
        }

        let master_key = beebeeb_core::kdf::MasterKey::from_bytes([32u8; 32]);
        let file_key = beebeeb_core::kdf::derive_file_key(&master_key, b"server-video-1");
        let uploads = prepare_thumbnail_uploads_for_plaintext_media(&payload, Some("video/mp4"), &file_key).unwrap();

        assert_eq!(uploads.len(), 2);
        assert_eq!(uploads[0].variant, ThumbnailUploadVariant::Medium);
        assert_eq!(uploads[1].variant, ThumbnailUploadVariant::Large);
        assert!(uploads[0].blurhash.is_none(), "video thumbnails do not carry blurhash");
        assert!(uploads[1].blurhash.is_none());

        for upload in &uploads {
            let plaintext = beebeeb_core::encrypt::decrypt_chunk_raw(&file_key, &upload.encrypted).unwrap();
            assert!(plaintext.starts_with(b"RIFF"), "video thumbnail plaintext must be WebP");
        }
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
            listener.set_nonblocking(true).unwrap();
            let base_url = format!("http://{}", listener.local_addr().unwrap());
            let requests = Arc::new(Mutex::new(Vec::new()));
            let server_requests = Arc::clone(&requests);
            let handle = thread::spawn(move || {
                let expected_min_requests = if fail_chunk { 3 } else { 4 };
                let started = std::time::Instant::now();
                let mut idle_after_min_since: Option<std::time::Instant> = None;
                loop {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            idle_after_min_since = None;
                            let request = read_http_request(&mut stream);
                            let response = upload_mock_response(&request, fail_chunk);
                            server_requests.lock().unwrap().push(request);
                            stream.write_all(response.as_bytes()).unwrap();
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            let count = server_requests.lock().unwrap().len();
                            if count >= expected_min_requests {
                                let idle_since = idle_after_min_since.get_or_insert_with(std::time::Instant::now);
                                if idle_since.elapsed() >= Duration::from_millis(300) {
                                    break;
                                }
                            }
                            if started.elapsed() >= Duration::from_secs(5) {
                                break;
                            }
                            std::thread::sleep(Duration::from_millis(10));
                        }
                        Err(e) => panic!("upload mock accept failed: {e}"),
                    }
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
            ("PUT", path) if path.starts_with("/api/v1/files/server-file-1/thumbnail") => {
                http_json("200 OK", serde_json::json!({ "message": "thumbnail uploaded" }))
            }
            _ => http_json(
                "404 Not Found",
                serde_json::json!({ "error": format!("unexpected {} {}", request.method, request.path) }),
            ),
        }
    }

    // ── hydrate_file_range (task 1024) ───────────────────────────────────────

    /// `hydrate_file_range` parses `file_id` as a UUID up front, so every
    /// hydration test needs a real one — this is an arbitrary fixed value, not
    /// tied to any real file.
    const TEST_FILE_ID: &str = "d6c090b4-69c8-437e-a61b-2023ea99ef89";

    /// Encrypt `plaintext` under `file_key` the same wire format
    /// `decrypt_downloaded_chunk` accepts (`decrypt_chunk_raw`'s
    /// `nonce(12) || ciphertext || tag(16)`), returning the raw bytes a
    /// `download_chunk` response body would carry.
    fn encrypt_chunk_wire(file_key: &beebeeb_core::kdf::FileKey, plaintext: &[u8]) -> Vec<u8> {
        let blob = beebeeb_core::encrypt::encrypt_chunk(file_key, plaintext).unwrap();
        let mut wire = blob.nonce.clone();
        wire.extend_from_slice(&blob.ciphertext);
        wire
    }

    /// Minimal mock server for hydration tests: serves `GET /api/v1/files/:id`
    /// (metadata) and `GET /api/v1/files/:id/chunks/:idx` (one encrypted chunk
    /// each) from a fixed plan, so `hydrate_file_range` can be exercised against
    /// real encrypted bytes without a live server.
    struct HydrationMockServer {
        base_url: String,
        handle: thread::JoinHandle<()>,
    }

    impl HydrationMockServer {
        /// `chunks` are PLAINTEXT chunk bodies (already split by the caller to
        /// the desired chunk_size_bytes); `size_bytes`/`chunk_count` reported by
        /// the metadata endpoint are derived from `chunks` so the test plan is
        /// internally consistent (mirrors how `plan_with_cap` lays out chunks:
        /// uniform except the last).
        fn start(file_key: beebeeb_core::kdf::FileKey, chunks: Vec<Vec<u8>>, requests: usize) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let base_url = format!("http://{}", listener.local_addr().unwrap());
            let size_bytes: usize = chunks.iter().map(|c| c.len()).sum();
            let chunk_count = chunks.len();
            let chunk_size_bytes = chunks.first().map(|c| c.len()).unwrap_or(0);
            let handle = thread::spawn(move || {
                for _ in 0..requests {
                    let (mut stream, _) = listener.accept().unwrap();
                    let request = read_http_request(&mut stream);
                    let response = hydration_mock_response(
                        &request,
                        &file_key,
                        &chunks,
                        size_bytes,
                        chunk_count,
                        chunk_size_bytes,
                    );
                    match response {
                        MockResponse::Text(body) => stream.write_all(body.as_bytes()).unwrap(),
                        MockResponse::Binary(body) => {
                            let header = format!(
                                "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                                body.len()
                            );
                            stream.write_all(header.as_bytes()).unwrap();
                            stream.write_all(&body).unwrap();
                        }
                    }
                }
            });
            Self { base_url, handle }
        }

        fn finish(self) {
            self.handle.join().unwrap();
        }
    }

    enum MockResponse {
        Text(String),
        Binary(Vec<u8>),
    }

    fn hydration_mock_response(
        request: &RecordedRequest,
        file_key: &beebeeb_core::kdf::FileKey,
        chunks: &[Vec<u8>],
        size_bytes: usize,
        chunk_count: usize,
        chunk_size_bytes: usize,
    ) -> MockResponse {
        if request.method == "GET" && request.path == format!("/api/v1/files/{TEST_FILE_ID}") {
            return MockResponse::Text(http_json(
                "200 OK",
                serde_json::json!({
                    "id": TEST_FILE_ID,
                    "size_bytes": size_bytes,
                    "chunk_count": chunk_count,
                    "chunk_size_bytes": chunk_size_bytes,
                }),
            ));
        }
        if let Some(idx_str) = request
            .path
            .strip_prefix(&format!("/api/v1/files/{TEST_FILE_ID}/chunks/"))
        {
            let idx: usize = idx_str.parse().expect("chunk index");
            let wire = encrypt_chunk_wire(file_key, &chunks[idx]);
            return MockResponse::Binary(wire);
        }
        MockResponse::Text(http_json(
            "404 Not Found",
            serde_json::json!({ "error": format!("unexpected {} {}", request.method, request.path) }),
        ))
    }

    fn hydration_test_key(master_key: [u8; 32], file_id: &str) -> beebeeb_core::kdf::FileKey {
        let mk = beebeeb_core::kdf::MasterKey::from_bytes(master_key);
        beebeeb_core::kdf::derive_file_key(&mk, file_id.as_bytes())
    }

    /// A multi-chunk file: request a range that spans exactly ONE interior
    /// chunk. `hydrate_file_range` must download+decrypt ONLY that chunk — the
    /// mock server accepts exactly 2 requests (the metadata GET + the one
    /// covering chunk GET) and would hang/fail on a third, so this test fails
    /// loudly if the whole file were downloaded instead.
    #[test]
    fn hydrate_file_range_downloads_only_covering_chunk() {
        let dir = tempfile::tempdir().unwrap();
        let master_key = [9u8; 32];
        let file_key = hydration_test_key(master_key, TEST_FILE_ID);

        // 3 chunks of 10 bytes each (uniform, no remainder) — chunk 1 is the
        // interior chunk under test.
        let chunks: Vec<Vec<u8>> = vec![b"AAAAAAAAAA".to_vec(), b"BBBBBBBBBB".to_vec(), b"CCCCCCCCCC".to_vec()];
        // 1 metadata GET + 1 chunk GET (only chunk index 1).
        let server = HydrationMockServer::start(file_key, chunks, 2);

        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_bridge_row(&bridge, TEST_FILE_ID, "/range.bin", None, FileStatus::CloudOnly, 30);

        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async { bridge.hydrate_file_range(TEST_FILE_ID, 10, 10).await })
            .unwrap()
            .expect("server-provided chunk_size_bytes should enable range hydration");

        assert_eq!(result.range_start_bytes, 10);
        assert!(!result.covers_whole_file);
        assert_eq!(&*result.data, b"BBBBBBBBBB");

        server.finish();

        // The row must NOT be marked Local from a single interior-range fetch —
        // only a whole-file covering range is evidence of full hydration.
        let entry = bridge.db.get_file(TEST_FILE_ID).unwrap().unwrap();
        assert_eq!(entry.status, FileStatus::Downloading);
    }

    /// Real desktop chunk planning is fixed-size per tier with only the last
    /// chunk short. For 9 MiB at the desktop tier the layout is 4 MiB + 4 MiB +
    /// 1 MiB, so a range inside the short final chunk must resolve to byte
    /// offset 8 MiB. The old `ceil(size_bytes / chunk_count)` derivation would
    /// incorrectly use 3 MiB and report offset 6 MiB.
    #[test]
    fn hydrate_file_range_uses_server_chunk_size_for_short_last_chunk_boundary() {
        const MIB: usize = 1024 * 1024;

        let dir = tempfile::tempdir().unwrap();
        let master_key = [12u8; 32];
        let file_key = hydration_test_key(master_key, TEST_FILE_ID);

        let mut last = vec![b'C'; MIB];
        last[123..127].copy_from_slice(b"LAST");
        let chunks: Vec<Vec<u8>> = vec![vec![b'A'; 4 * MIB], vec![b'B'; 4 * MIB], last];
        let server = HydrationMockServer::start(file_key, chunks, 2);

        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_bridge_row(
            &bridge,
            TEST_FILE_ID,
            "/short-last.bin",
            None,
            FileStatus::CloudOnly,
            9 * MIB as i64,
        );

        let required_offset = (8 * MIB + 123) as u64;
        let required_length = 4u64;
        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async {
                bridge
                    .hydrate_file_range(TEST_FILE_ID, required_offset, required_length)
                    .await
            })
            .unwrap()
            .expect("server-provided chunk_size_bytes should enable range hydration");

        assert_eq!(result.range_start_bytes, (8 * MIB) as u64);
        assert!(!result.covers_whole_file);
        let relative_start = (required_offset - result.range_start_bytes) as usize;
        let relative_end = relative_start + required_length as usize;
        assert_eq!(&result.data[relative_start..relative_end], b"LAST");

        server.finish();
    }

    #[test]
    fn hydration_range_plan_uses_authoritative_chunk_size_for_short_last_chunk() {
        const MIB: u64 = 1024 * 1024;

        let plan = plan_hydration_chunk_range(9 * MIB, 3, 4 * MIB, 8 * MIB + 123, 4)
            .unwrap()
            .expect("valid server chunk size should produce a range plan");

        assert_eq!(plan.first_chunk, 2);
        assert_eq!(plan.last_chunk_exclusive, 3);
        assert_eq!(plan.range_start_bytes, 8 * MIB);
        assert_eq!(plan.covering_span_hint, 4 * MIB as usize);
        assert!(!plan.covers_whole_file);
        assert_ne!(
            plan.range_start_bytes,
            2 * (9 * MIB).div_ceil(3),
            "regression guard: ceil(size/chunk_count) would pick the wrong base offset"
        );
    }

    /// A range request that happens to cover the WHOLE file (offset 0, length
    /// >= size) — this is the `covers_whole_file` branch, which mirrors
    /// `hydrate_file_to_memory`'s bookkeeping: mark `Local` + `mark_cached`.
    #[test]
    fn hydrate_file_range_covering_whole_file_marks_local() {
        let dir = tempfile::tempdir().unwrap();
        let master_key = [11u8; 32];
        let file_key = hydration_test_key(master_key, TEST_FILE_ID);

        let chunks: Vec<Vec<u8>> = vec![b"HELLOWORLD".to_vec()]; // 1 chunk, 10 bytes
        let server = HydrationMockServer::start(file_key, chunks, 2);

        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_bridge_row(&bridge, TEST_FILE_ID, "/whole.bin", None, FileStatus::CloudOnly, 10);

        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async { bridge.hydrate_file_range(TEST_FILE_ID, 0, 10).await })
            .unwrap()
            .unwrap();

        assert!(result.covers_whole_file);
        assert_eq!(&*result.data, b"HELLOWORLD");

        server.finish();

        let entry = bridge.db.get_file(TEST_FILE_ID).unwrap().unwrap();
        assert_eq!(entry.status, FileStatus::Local);
    }

    /// Metadata missing `chunk_count` (or it's `0`) must fall back cleanly —
    /// `Ok(None)`, never an error — so the caller can retry via the whole-file
    /// `hydrate_file_to_memory` path.
    #[test]
    fn hydrate_file_range_falls_back_when_chunk_count_absent() {
        let dir = tempfile::tempdir().unwrap();
        let master_key = [3u8; 32];
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let _ = read_http_request(&mut stream);
            let body = http_json(
                "200 OK",
                serde_json::json!({ "id": TEST_FILE_ID, "size_bytes": 100 }), // no chunk_count
            );
            stream.write_all(body.as_bytes()).unwrap();
        });

        let bridge = test_bridge_with_api(&dir.path().join("state.db"), base_url, master_key);
        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async { bridge.hydrate_file_range(TEST_FILE_ID, 0, 10).await })
            .unwrap();
        assert!(result.is_none());

        handle.join().unwrap();
    }

    #[test]
    fn hydrate_file_range_falls_back_when_chunk_size_absent_even_if_size_and_count_present() {
        let dir = tempfile::tempdir().unwrap();
        let master_key = [4u8; 32];
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let _ = read_http_request(&mut stream);
            let body = http_json(
                "200 OK",
                serde_json::json!({
                    "id": TEST_FILE_ID,
                    "size_bytes": 24,
                    "chunk_count": 3,
                }),
            );
            stream.write_all(body.as_bytes()).unwrap();
        });

        let bridge = test_bridge_with_api(&dir.path().join("state.db"), base_url, master_key);
        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async { bridge.hydrate_file_range(TEST_FILE_ID, 0, 10).await })
            .unwrap();
        assert!(result.is_none());

        handle.join().unwrap();
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

    #[test]
    fn record_moved_to_trash_activity_writes_deletion_event() {
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();

        record_moved_to_trash_activity(&db, "server-file-1", "docs/doomed.txt", 200).unwrap();

        let events = db.list_recent_local_activity(5).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, LocalActivityKind::MovedToTrash);
        assert_eq!(events[0].file_id.as_deref(), Some("server-file-1"));
        assert_eq!(events[0].file_name, "doomed.txt");
        assert_eq!(events[0].rel_path.as_deref(), Some("docs/doomed.txt"));
        assert_eq!(events[0].occurred_at, 200);
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
                backup_source_key: None,
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
        let events = bridge.db.list_recent_local_activity(5).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, crate::state_db::LocalActivityKind::MovedToTrash);
        assert_eq!(events[0].file_id.as_deref(), Some("server-file-1"));
        assert_eq!(events[0].file_name, "doomed.txt");
        assert_eq!(events[0].rel_path.as_deref(), Some("doomed.txt"));
        assert_eq!(events[0].occurred_at, 200);

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
                backup_source_key: None,
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
    fn test_local_cache_limit_counts_pinned_bytes_against_total_cap() {
        let dir = tempfile::tempdir().unwrap();
        let bridge = test_bridge(&dir.path().join("state.db"));
        seed_bridge_row(&bridge, "pinned", "/Pinned.txt", None, FileStatus::Local, 700);
        seed_bridge_row(&bridge, "old", "/Old.txt", None, FileStatus::Local, 400);
        seed_bridge_row(&bridge, "new", "/New.txt", None, FileStatus::Local, 300);

        bridge.db.set_recursive_pin("pinned", true, 1).unwrap();
        bridge.db.mark_cached("pinned", "/cache/pinned", 700, 10).unwrap();
        bridge.db.mark_cached("old", "/cache/old", 400, 20).unwrap();
        bridge.db.mark_cached("new", "/cache/new", 300, 30).unwrap();

        let evicted = bridge.enforce_local_cache_limit(Some(1_000)).unwrap();

        assert_eq!(evicted.evicted_file_ids, vec!["old".to_string()]);
        assert_eq!(bridge.db.get_file("pinned").unwrap().unwrap().status, FileStatus::Local);
        assert_eq!(bridge.db.get_file("new").unwrap().unwrap().status, FileStatus::Local);
        assert_eq!(
            bridge.db.get_file("old").unwrap().unwrap().status,
            FileStatus::CloudOnly
        );
    }

    #[test]
    fn test_local_cache_usage_bytes_sums_pinned_and_unpinned_cache() {
        let dir = tempfile::tempdir().unwrap();
        let bridge = test_bridge(&dir.path().join("state.db"));
        seed_bridge_row(&bridge, "pinned", "/Pinned.txt", None, FileStatus::Local, 700);
        seed_bridge_row(&bridge, "unpinned", "/Unpinned.txt", None, FileStatus::Local, 600);

        bridge.db.set_recursive_pin("pinned", true, 1).unwrap();
        bridge.db.mark_cached("pinned", "/cache/pinned", 700, 10).unwrap();
        bridge.db.mark_cached("unpinned", "/cache/unpinned", 600, 20).unwrap();

        assert_eq!(bridge.local_cache_usage_bytes().unwrap(), 1_300);
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
                    "capability": "write"
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
        assert_eq!(roots[0].display_name, "Encrypted folder");
        assert_eq!(roots[0].permission_bits, PERMISSION_READ | PERMISSION_SHARE);
        assert_eq!(roots[1].permission_bits, PERMISSION_READ | PERMISSION_WRITE);
    }

    #[test]
    fn shared_invite_id_match_does_not_match_same_root_wrong_invite() {
        let wrong_invite_same_root = serde_json::json!({
            "id": "invite-wrong",
            "file_id": "root-folder",
            "status": "approved"
        });
        let right_invite_different_root = serde_json::json!({
            "invite_id": "invite-target",
            "file_id": "other-root",
            "status": "approved"
        });

        assert!(!shared_invite_id_matches(&wrong_invite_same_root, "invite-target"));
        assert!(shared_invite_id_matches(&right_invite_different_root, "invite-target"));
    }

    fn fixed_aes_gcm_frame(key: &[u8; 32], nonce_byte: u8, plaintext: &[u8]) -> Vec<u8> {
        use aes_gcm::aead::Aead;
        use aes_gcm::{Aes256Gcm, KeyInit, Nonce};

        let cipher = Aes256Gcm::new_from_slice(key).unwrap();
        let nonce_bytes = [nonce_byte; 12];
        let mut out = nonce_bytes.to_vec();
        let ciphertext = cipher.encrypt(Nonce::from_slice(&nonce_bytes), plaintext).unwrap();
        out.extend_from_slice(&ciphertext);
        out
    }

    #[test]
    fn unwrap_folder_share_file_key_fixture_decrypts_content_chunk() {
        let owner_master = [0x11u8; 32];
        let recipient_master = [0x22u8; 32];
        let folder_id = "0bb0f451-986d-43e0-bbef-f1e8acb27a01";
        let child_id = "4a823d35-3ae2-4fc7-8266-762096405bc7";
        let folder_key = [0x33u8; 32];
        let child_file_key = [0x44u8; 32];
        let plaintext = b"shared plaintext fixture";

        let owner_mk = beebeeb_core::kdf::MasterKey::from_bytes(owner_master);
        let owner_private = beebeeb_core::opaque::derive_x25519_private(&owner_mk);
        let owner_public = beebeeb_core::opaque::derive_x25519_public(&owner_private);
        let recipient_mk = beebeeb_core::kdf::MasterKey::from_bytes(recipient_master);
        let recipient_private = beebeeb_core::opaque::derive_x25519_private(&recipient_mk);
        let recipient_public = beebeeb_core::opaque::derive_x25519_public(&recipient_private);
        let shared_secret = beebeeb_core::opaque::x25519_shared_secret(&owner_private, &recipient_public).unwrap();
        let share_key = beebeeb_core::opaque::derive_share_key(&shared_secret, folder_id.as_bytes());

        let encrypted_folder_key =
            base64::engine::general_purpose::STANDARD.encode(fixed_aes_gcm_frame(&share_key, 0xa1, &folder_key));
        let encrypted_child_file_key =
            base64::engine::general_purpose::STANDARD.encode(fixed_aes_gcm_frame(&folder_key, 0xb2, &child_file_key));
        let content_wire = fixed_aes_gcm_frame(&child_file_key, 0xc3, plaintext);

        let unwrapped = unwrap_folder_share_file_key(
            &recipient_master,
            &base64::engine::general_purpose::STANDARD.encode(owner_public),
            folder_id,
            &encrypted_folder_key,
            child_id,
            &serde_json::json!({
                "keys": [{ "file_id": child_id, "encrypted_file_key": encrypted_child_file_key }]
            }),
        )
        .unwrap();

        assert_eq!(unwrapped.as_bytes(), &child_file_key);
        assert_eq!(decrypt_downloaded_chunk(&unwrapped, &content_wire).unwrap(), plaintext);
    }

    #[test]
    fn unwrap_folder_share_file_key_web_generated_vector_decrypts_child_key() {
        // Generated from repos/web/src/lib/folder-share-crypto.ts using the real
        // encryptFolderKeyForRecipient/encryptChildFileKey functions with
        // fixed inputs and beebeeb-wasm backing their crypto worker proxy.
        let recipient_master = [0x22u8; 32];
        let folder_id = "0bb0f451-986d-43e0-bbef-f1e8acb27a01";
        let child_id = "4a823d35-3ae2-4fc7-8266-762096405bc7";
        let owner_public_key = "zr4yc0fCIWlQgKPrOGWQG25jB3Kbm13rDDezBBz61Uc=";
        let encrypted_folder_key = "PVZ8a7JfzaV/m2p53h33xsIHsXtwaFmfcttbly6GoKnRVlnh8V5LbXiwCGFLbZhNLskOuwJX0NJQ3UzJ";
        let encrypted_child_file_key = "xbKO8C1APBQytc8VqT0mfT5oSaP+8h02CWObRL2kegAOsBbzFE6N4ZLfRQy2m63bLsPBjmNXLEUaHlPu";

        let folder_key =
            unwrap_folder_share_key(&recipient_master, owner_public_key, folder_id, encrypted_folder_key).unwrap();
        assert_eq!(&*folder_key, &[0x33u8; 32]);

        let child_key = unwrap_folder_share_file_key(
            &recipient_master,
            owner_public_key,
            folder_id,
            encrypted_folder_key,
            child_id,
            &serde_json::json!({
                "keys": [{ "file_id": child_id, "encrypted_file_key": encrypted_child_file_key }]
            }),
        )
        .unwrap();

        assert_eq!(child_key.as_bytes(), &[0x44u8; 32]);
    }

    #[test]
    fn decrypt_shared_name_with_key_rejects_unauthenticated_plaintext_name() {
        let file_key = beebeeb_core::kdf::FileKey::from_bytes([0x55u8; 32]);

        assert_eq!(
            decrypt_shared_name_with_key(&file_key, "../../../../.ssh/authorized_keys"),
            None
        );
    }

    #[test]
    fn decrypt_shared_name_with_key_accepts_authenticated_metadata_name() {
        let file_key = beebeeb_core::kdf::FileKey::from_bytes([0x55u8; 32]);
        let plaintext = serde_json::json!({
            "name": "Quarterly report.txt",
            "mime_type": "text/plain"
        })
        .to_string();
        let name_encrypted =
            serde_json::to_string(&beebeeb_core::encrypt::encrypt_metadata(&file_key, &plaintext).unwrap()).unwrap();

        assert_eq!(
            decrypt_shared_name_with_key(&file_key, &name_encrypted),
            Some("Quarterly report.txt".to_string())
        );
    }

    #[test]
    fn shared_metadata_ignores_raw_server_name_fields_when_decrypt_fails() {
        let file_key = beebeeb_core::kdf::FileKey::from_bytes([0x55u8; 32]);
        let metadata = serde_json::json!({
            "id": "child-file",
            "name_encrypted": "not authenticated ciphertext",
            "display_name": "../../../../.ssh/authorized_keys",
            "decrypted_name": "server-claimed.txt",
            "path": "nested/server-claimed.txt"
        });

        assert_eq!(
            shared_name_from_metadata(&metadata, &ItemKind::File, Some(&file_key)),
            "Encrypted file"
        );
    }

    #[test]
    fn shared_metadata_uses_placeholder_for_authenticated_traversal_name() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = StateDb::open(dir.path().join("state.db")).expect("state db");
        let root = SharedRootMapping {
            invite_id: "invite-folder".into(),
            file_id: "root-folder".into(),
            display_name: "Shared folder".into(),
            is_folder: true,
            size_bytes: 0,
            content_type: None,
            owner_email: Some("owner@example.com".into()),
            sender_public_key: None,
            encrypted_file_key: None,
            encrypted_folder_key: None,
            file_name_encrypted: None,
            permission_bits: PERMISSION_READ,
            approved_at: None,
        };
        let file_key = beebeeb_core::kdf::FileKey::from_bytes([0x66u8; 32]);
        let malicious_name = serde_json::json!({
            "name": "../../../../.ssh/authorized_keys",
            "mime_type": "text/plain"
        })
        .to_string();
        let name_encrypted =
            serde_json::to_string(&beebeeb_core::encrypt::encrypt_metadata(&file_key, &malicious_name).unwrap())
                .unwrap();

        let entry = apply_shared_metadata_file_row(
            &db,
            &serde_json::json!({
                "id": "child-file",
                "name_encrypted": name_encrypted,
                "size_bytes": 12,
                "is_folder": false,
            }),
            &root,
            1_700_000_000,
            Some(root.file_id.clone()),
            "Shared with me/Shared folder",
            Some(&file_key),
        )
        .expect("apply shared metadata")
        .expect("entry");

        assert_eq!(entry.path, "Shared with me/Shared folder/Encrypted file");
        crate::reject_unsafe_rel_path(&entry.path).expect("shared path must stay safe");
    }

    #[test]
    fn shared_hydrate_write_guard_rejects_persisted_traversal_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        let bridge = test_bridge(&dir.path().join("state.db"));
        bridge
            .db
            .upsert_file(&FileEntry {
                file_id: "malicious-shared-file".into(),
                path: "Shared with me/folder/../../outside.txt".into(),
                status: FileStatus::CloudOnly,
                size_bytes: 12,
                modified_at: 0,
                content_hash: None,
                remote_updated_at: 0,
                parent_id: None,
                item_kind: ItemKind::File,
            })
            .unwrap();
        let mut contract = bridge
            .db
            .get_file_contract_state("malicious-shared-file")
            .unwrap()
            .unwrap();
        contract.namespace = Namespace::SharedWithMe;
        contract.shared_root_id = Some("root-folder".into());
        contract.share_id = Some("invite-folder".into());
        contract.permission_bits = PERMISSION_READ;
        bridge.db.set_file_contract_state(&contract).unwrap();

        let err = bridge
            .ensure_shared_hydrate_path_safe("malicious-shared-file")
            .unwrap_err()
            .to_string();

        assert!(err.contains("unsafe shared hydrate path"));
        assert!(err.contains(".."));
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
    fn classify_ignores_status_like_digits_in_the_request_url() {
        // Task 1252: reqwest's Display suffixes `" for url (…)"`, and the URL's
        // ephemeral port / path ids can contain "401", "403", etc. Those are not
        // status codes and must never flip a retryable failure into a pausable
        // one. A real 5xx to such a port must stay Retryable.
        for port in ["45401", "40113", "34012", "50403", "44039"] {
            let err = format!(
                "HTTP status server error (500 Internal Server Error) for url (http://127.0.0.1:{port}/api/v1/uploads/upload-session-1/chunks/0)"
            );
            assert_eq!(
                classify_operation_error(&err),
                OperationFailureClass::Retryable,
                "500 to port {port} must stay Retryable, not be misread from the URL"
            );
        }
        // A connection error to the same kind of port is likewise Retryable.
        assert_eq!(
            classify_operation_error("error sending request for url (http://127.0.0.1:45401/api/v1/uploads/upload-session-1/chunks/0)"),
            OperationFailureClass::Retryable
        );
        // Genuine auth/permission failures still classify from reqwest's status
        // reason phrase (which sits BEFORE the stripped URL), even when the URL
        // also carries unrelated digits.
        assert_eq!(
            classify_operation_error("HTTP status client error (401 Unauthorized) for url (http://127.0.0.1:8080/api/v1/uploads/init)"),
            OperationFailureClass::Auth
        );
        assert_eq!(
            classify_operation_error("HTTP status client error (403 Forbidden) for url (http://127.0.0.1:8080/api/v1/files/abc)"),
            OperationFailureClass::Permission
        );
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
            beebeeb_core::encrypt::encrypt_name(&mk, file_id, "Quarterly Report.pdf", Some("application/pdf")).unwrap();

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
                backup_source_key: None,
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
        seed_bridge_row(&bridge, "file-1", "Draft.txt", None, FileStatus::Uploading, 0);
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
                backup_source_key: None,
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
        assert_eq!(
            bridge.db.get_file("file-1").unwrap().unwrap().status,
            FileStatus::Error,
            "a failed/deferred upload must not remain counted as active Uploading"
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
                backup_source_key: None,
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
    async fn test_resolve_keep_mine_uploads_local_payload_and_commits_state() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(sync_root.join("Docs")).unwrap();
        let local_path = sync_root.join("Docs/conflict.txt");
        std::fs::write(&local_path, b"mine upload payload").unwrap();

        let server = UploadMockServer::start(false);
        let master_key = [13u8; 32];
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        bridge
            .db
            .upsert_file(&FileEntry {
                file_id: "server-file-1".into(),
                path: "Docs/conflict.txt".into(),
                status: FileStatus::Conflict,
                size_bytes: 99,
                modified_at: 100,
                content_hash: Some("local-conflict-hash".into()),
                remote_updated_at: 90,
                parent_id: None,
                item_kind: ItemKind::File,
            })
            .unwrap();
        let mut contract = bridge.db.get_file_contract_state("server-file-1").unwrap().unwrap();
        contract.parent_id = Some("folder-1".into());
        contract.content_type = Some("text/plain".into());
        contract.current_version = 3;
        contract.local_base_version = 2;
        contract.current_object_version_id = Some("object-before".into());
        bridge.db.set_file_contract_state(&contract).unwrap();

        let before = now_secs();
        bridge.resolve_keep_mine("server-file-1", &sync_root).await.unwrap();

        let entry = bridge.db.get_file("server-file-1").unwrap().unwrap();
        assert_eq!(entry.status, FileStatus::Local);
        assert_eq!(entry.path, "Docs/conflict.txt");
        assert_eq!(entry.size_bytes, 19);
        assert_eq!(entry.content_hash.as_deref(), Some("local-conflict-hash"));
        assert!(entry.remote_updated_at >= before);

        let contract = bridge.db.get_file_contract_state("server-file-1").unwrap().unwrap();
        assert_eq!(contract.current_version, 1);
        assert_eq!(contract.local_base_version, 1);
        assert_eq!(contract.current_object_version_id.as_deref(), Some("object-complete-1"));
        assert_eq!(contract.parent_id.as_deref(), Some("folder-1"));
        assert_eq!(contract.content_type.as_deref(), Some("text/plain"));

        let requests = server.finish();
        assert_eq!(requests.len(), 4);
        assert_eq!(requests[0].method, "POST");
        assert_eq!(requests[0].path, "/api/v1/uploads/init");
        let init_body: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(init_body["file_id"], "server-file-1");
        assert_eq!(init_body["file_size_bytes"], 19);
        assert_eq!(init_body["parent_id"], "folder-1");
        assert_eq!(init_body["chunk_count"], 1);
        assert!(
            init_body.get("base_version_number").is_none(),
            "Keep Mine is an explicit conflict override, not a stale-base retry"
        );
        assert_eq!(requests[1].method, "PATCH");
        assert_eq!(requests[1].path, "/api/v1/files/server-file-1");
        assert_eq!(requests[2].method, "PUT");
        assert_eq!(requests[2].path, "/api/v1/uploads/upload-session-1/chunks/0");
        assert_ne!(requests[2].body, b"mine upload payload");

        let master_key = beebeeb_core::kdf::MasterKey::from_bytes(master_key);
        let file_key = beebeeb_core::kdf::derive_file_key(&master_key, b"server-file-1");
        assert_eq!(
            beebeeb_core::encrypt::decrypt_chunk_raw(&file_key, &requests[2].body).unwrap(),
            b"mine upload payload"
        );
        assert_eq!(requests[3].method, "POST");
        assert_eq!(requests[3].path, "/api/v1/uploads/upload-session-1/complete");
    }

    #[tokio::test]
    async fn test_resolve_keep_mine_returns_error_when_upload_fails() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        std::fs::write(sync_root.join("conflict.txt"), b"retry me").unwrap();

        let server = UploadMockServer::start(true);
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), [14u8; 32]);
        bridge
            .db
            .upsert_file(&FileEntry {
                file_id: "server-file-1".into(),
                path: "conflict.txt".into(),
                status: FileStatus::Conflict,
                size_bytes: 8,
                modified_at: 100,
                content_hash: Some("local-conflict-hash".into()),
                remote_updated_at: 90,
                parent_id: None,
                item_kind: ItemKind::File,
            })
            .unwrap();
        let mut contract = bridge.db.get_file_contract_state("server-file-1").unwrap().unwrap();
        contract.content_type = Some("text/plain".into());
        contract.current_version = 3;
        contract.local_base_version = 2;
        bridge.db.set_file_contract_state(&contract).unwrap();

        let err = bridge
            .resolve_keep_mine("server-file-1", &sync_root)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("500 Internal Server Error"));

        let entry = bridge.db.get_file("server-file-1").unwrap().unwrap();
        assert_eq!(entry.status, FileStatus::Conflict);
        assert_eq!(entry.remote_updated_at, 90);

        let requests = server.finish();
        assert_eq!(requests.len(), 3);
        assert_eq!(requests[0].path, "/api/v1/uploads/init");
        assert_eq!(requests[1].path, "/api/v1/files/server-file-1");
        assert_eq!(requests[2].path, "/api/v1/uploads/upload-session-1/chunks/0");
    }

    #[tokio::test]
    async fn audit_1244_auto_resolve_keep_both_preserves_local_copy_when_remote_hydrate_fails() {
        // Production mutation caught: hydrating before renaming, or leaving the row
        // in Conflict after a hydrate failure, would either risk the local bytes or
        // hide the retry state from the normal Error flow.
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let original = sync_root.join("conflict.txt");
        std::fs::write(&original, b"local conflict bytes").unwrap();

        let file_id = "bbbb0000-0000-4000-8000-000000000010";
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), "http://127.0.0.1:9".into(), [15u8; 32]);
        bridge
            .db
            .upsert_file(&FileEntry {
                file_id: file_id.into(),
                path: "conflict.txt".into(),
                status: FileStatus::Conflict,
                size_bytes: 20,
                modified_at: 100,
                content_hash: Some("local-conflict-hash".into()),
                remote_updated_at: 90,
                parent_id: None,
                item_kind: ItemKind::File,
            })
            .unwrap();
        let entry = bridge.db.get_file(file_id).unwrap().unwrap();

        let err = bridge
            .auto_resolve_keep_both(&sync_root, &entry)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("error sending request") || err.contains("Connection refused"));

        let row = bridge.db.get_file(file_id).unwrap().unwrap();
        assert_eq!(row.status, FileStatus::Error);
        assert!(!original.exists(), "original path is left vacant for the remote retry");
        let conflict_copy = std::fs::read_dir(&sync_root)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("conflict (conflict - ") && name.ends_with(".txt"))
            })
            .expect("local conflict copy should be renamed with a device/date suffix");
        assert_eq!(std::fs::read(conflict_copy).unwrap(), b"local conflict bytes");
    }

    #[tokio::test]
    async fn test_process_due_operations_uploads_encrypted_image_thumbnails_after_complete() {
        let dir = tempfile::tempdir().unwrap();
        let payload = dir.path().join("photo.png");
        write_test_png(&payload);

        let server = UploadMockServer::start(false);
        let master_key = [31u8; 32];
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        bridge
            .db
            .enqueue_operation(&PendingOperation {
                op_id: "op-upload-photo".into(),
                kind: OperationKind::UploadVersion,
                file_id: Some("local-photo-1".into()),
                parent_id: None,
                target_path: Some("Photos/photo.png".into()),
                metadata_json: Some(
                    serde_json::json!({
                        "operation": "create_file",
                        "name_encrypted": "{\"cipher_suite\":\"V1Aes256Gcm\"}",
                        "display_name": "photo.png",
                        "content_type": "image/png"
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
                backup_source_key: None,
                created_at: 100,
                updated_at: 100,
            })
            .unwrap();

        let outcome = bridge.process_due_operations(dir.path(), 200).await.unwrap();
        assert_eq!(outcome.completed_op_ids, vec!["op-upload-photo".to_string()]);

        let requests = server.finish();
        let medium = requests
            .iter()
            .find(|r| r.method == "PUT" && r.path.starts_with("/api/v1/files/server-file-1/thumbnail?blurhash="))
            .expect("medium thumbnail upload with blurhash query");
        let large = requests
            .iter()
            .find(|r| r.method == "PUT" && r.path == "/api/v1/files/server-file-1/thumbnail/large")
            .expect("large thumbnail upload");

        let master_key = beebeeb_core::kdf::MasterKey::from_bytes(master_key);
        let file_key = beebeeb_core::kdf::derive_file_key(&master_key, b"server-file-1");
        let medium_plain = beebeeb_core::encrypt::decrypt_chunk_raw(&file_key, &medium.body).unwrap();
        let large_plain = beebeeb_core::encrypt::decrypt_chunk_raw(&file_key, &large.body).unwrap();

        assert_ne!(medium.body, medium_plain);
        assert_ne!(large.body, large_plain);
        assert!(medium_plain.starts_with(b"RIFF"), "medium thumbnail must be WebP");
        assert!(large_plain.starts_with(b"RIFF"), "large thumbnail must be WebP");

        let complete_index = requests
            .iter()
            .position(|r| r.method == "POST" && r.path == "/api/v1/uploads/upload-session-1/complete")
            .unwrap();
        let medium_index = requests.iter().position(|r| std::ptr::eq(r, medium)).unwrap();
        let large_index = requests.iter().position(|r| std::ptr::eq(r, large)).unwrap();
        assert!(medium_index > complete_index);
        assert!(large_index > complete_index);
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
                backup_source_key: None,
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
                backup_source_key: None,
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
                backup_source_key: None,
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
                backup_source_key: None,
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

    fn seed_bridge_entry(
        bridge: &EngineBridge,
        file_id: &str,
        path: &str,
        parent_id: Option<&str>,
        status: FileStatus,
        is_folder: bool,
        remote_updated_at: i64,
    ) {
        let item_kind = if is_folder { ItemKind::Folder } else { ItemKind::File };
        bridge
            .db
            .upsert_file(&FileEntry {
                file_id: file_id.into(),
                path: path.into(),
                status,
                size_bytes: if is_folder { 0 } else { 10 },
                modified_at: remote_updated_at,
                content_hash: None,
                remote_updated_at,
                parent_id: parent_id.map(str::to_string),
                item_kind: item_kind.clone(),
            })
            .unwrap();
        let mut contract = bridge.db.get_file_contract_state(file_id).unwrap().unwrap();
        contract.namespace = Namespace::MyFiles;
        contract.parent_id = parent_id.map(str::to_string);
        contract.item_kind = item_kind;
        contract.permission_bits = PERMISSION_READ | PERMISSION_WRITE | PERMISSION_OWNER;
        bridge.db.set_file_contract_state(&contract).unwrap();
    }

    fn enqueue_test_operation(bridge: &EngineBridge, op_id: &str, kind: OperationKind, file_id: &str, created_at: i64) {
        bridge
            .db
            .enqueue_operation(&PendingOperation {
                op_id: op_id.into(),
                kind,
                file_id: Some(file_id.into()),
                parent_id: None,
                target_path: Some(format!("{file_id}.txt")),
                metadata_json: None,
                payload_path: None,
                base_version: None,
                base_object_version_id: None,
                attempts: 0,
                max_attempts: 25,
                next_retry_at: 0,
                last_error: None,
                backup_source_key: None,
                created_at,
                updated_at: created_at,
            })
            .unwrap();
    }

    #[test]
    fn audit_1244_apply_sync_op_trash_echo_preserves_local_trashing_owner() {
        // Production mutation caught: removing a Trashing row on a file_trash echo
        // would make the local-delete-in-flight path lose its durable owner state.
        let dir = tempfile::tempdir().unwrap();
        let bridge = test_bridge(&dir.path().join("state.db"));
        let file = "echo0000-0000-4000-8000-000000000001";
        seed_bridge_entry(&bridge, file, "doomed.txt", None, FileStatus::Trashing, false, 10);
        enqueue_test_operation(&bridge, "op-trash-echo", OperationKind::TrashFile, file, 100);

        let op = crate::api_client::SyncOp {
            seq_id: 12,
            op_type: "file_trash".into(),
            payload: serde_json::json!({ "id": file }),
        };
        let mut conflicts = Vec::new();
        apply_sync_op(&bridge, dir.path(), &op, 200, &mut conflicts).unwrap();

        let row = bridge.db().get_file(file).unwrap().unwrap();
        assert_eq!(row.status, FileStatus::Trashing);
        let queued = bridge.db().list_due_operations(999).unwrap();
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].op_id, "op-trash-echo");
        assert!(conflicts.is_empty());
    }

    #[test]
    fn audit_1244_double_trash_restore_cycle_keeps_resnapshot_for_final_restore() {
        // Production mutation caught: treating file_restore as a no-op, or clearing
        // the resnapshot request during the same op batch, would leave the final
        // restored row invisible after two fast trash+restore cycles.
        let dir = tempfile::tempdir().unwrap();
        let mk = [8u8; 32];
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), "http://placeholder".into(), mk);
        let file = "cycle000-0000-4000-8000-000000000001";
        seed_bridge_entry(&bridge, file, "doc.txt", None, FileStatus::CloudOnly, false, 10);

        let mut conflicts = Vec::new();
        for (seq_id, op_type) in [
            (2, "file_trash"),
            (3, "file_restore"),
            (4, "file_trash"),
            (5, "file_restore"),
        ] {
            let op = crate::api_client::SyncOp {
                seq_id,
                op_type: op_type.into(),
                payload: serde_json::json!({ "id": file }),
            };
            apply_sync_op(&bridge, dir.path(), &op, 200, &mut conflicts).unwrap();
        }

        assert!(
            bridge.db().get_file(file).unwrap().is_none(),
            "trash ops remove the row until the forced snapshot re-materialises it"
        );
        assert!(
            bridge.db().take_needs_resnapshot().unwrap(),
            "at least one restore in the batch must force the next tick to snapshot"
        );

        let snapshot = crate::api_client::SyncSnapshot {
            seq_id: 5,
            nodes: vec![snap_node(&mk, file, "doc.txt", None, false, 50)],
        };
        apply_snapshot(&bridge, dir.path(), &snapshot, 250, 250, &mut conflicts).unwrap();

        let row = bridge.db().get_file(file).unwrap().unwrap();
        assert_eq!(row.path, "doc.txt");
        assert_eq!(row.status, FileStatus::CloudOnly);
        assert!(conflicts.is_empty());
    }

    #[test]
    fn audit_1244_gap_rebootstrap_keeps_offline_local_upload_when_remote_delete_absent() {
        // Production mutation caught: pruning rows with pending UploadVersion work
        // during a gap-recovery snapshot would silently delete a local offline edit
        // just because another device remotely deleted the server row.
        let dir = tempfile::tempdir().unwrap();
        let mk = [4u8; 32];
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), "http://placeholder".into(), mk);
        let local = "local000-0000-4000-8000-000000000001";
        let other = "other000-0000-4000-8000-000000000002";
        seed_bridge_entry(&bridge, local, "report.txt", None, FileStatus::Error, false, 10);
        enqueue_test_operation(&bridge, "op-upload-offline", OperationKind::UploadVersion, local, 100);

        let snapshot = crate::api_client::SyncSnapshot {
            seq_id: 77,
            nodes: vec![snap_node(&mk, other, "other.txt", None, false, 80)],
        };
        let mut conflicts = Vec::new();
        apply_snapshot(&bridge, dir.path(), &snapshot, 200, 200, &mut conflicts).unwrap();

        let row = bridge.db().get_file(local).unwrap().unwrap();
        assert_eq!(row.path, "report.txt");
        assert_eq!(row.status, FileStatus::Error);
        assert_eq!(
            bridge.db().list_due_operations(999).unwrap()[0].op_id,
            "op-upload-offline",
            "pending local upload remains durable after rebootstrap"
        );
        assert!(bridge.db().get_file(other).unwrap().is_some());
        assert!(conflicts.is_empty());
    }

    #[test]
    fn audit_1244_snapshot_partial_folder_trash_prunes_children_seen_without_parent() {
        // Production mutation caught: re-rooting children whose parent folder is
        // absent from the snapshot before prune_absent runs leaks ghost rows.
        let dir = tempfile::tempdir().unwrap();
        let mk = [6u8; 32];
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), "http://placeholder".into(), mk);
        let folder = "fold0000-0000-4000-8000-000000000001";
        let child = "fold0000-0000-4000-8000-000000000002";
        let subfolder = "fold0000-0000-4000-8000-000000000003";
        let grandchild = "fold0000-0000-4000-8000-000000000004";
        let sibling = "fold0000-0000-4000-8000-000000000005";

        seed_bridge_entry(&bridge, folder, "docs", None, FileStatus::CloudOnly, true, 10);
        seed_bridge_entry(
            &bridge,
            child,
            "docs/a.txt",
            Some(folder),
            FileStatus::CloudOnly,
            false,
            10,
        );
        seed_bridge_entry(
            &bridge,
            subfolder,
            "docs/sub",
            Some(folder),
            FileStatus::CloudOnly,
            true,
            10,
        );
        seed_bridge_entry(
            &bridge,
            grandchild,
            "docs/sub/b.txt",
            Some(subfolder),
            FileStatus::CloudOnly,
            false,
            10,
        );
        seed_bridge_entry(&bridge, sibling, "outside.txt", None, FileStatus::CloudOnly, false, 10);

        let snapshot = crate::api_client::SyncSnapshot {
            seq_id: 88,
            nodes: vec![
                // Concurrent partial sync: the server snapshot still contains the
                // folder's independent children, but the trashed folder node itself
                // is absent. The local mirror must remove the subtree, not re-root it.
                snap_node(&mk, child, "a.txt", Some(folder), false, 80),
                snap_node(&mk, subfolder, "sub", Some(folder), true, 80),
                snap_node(&mk, grandchild, "b.txt", Some(subfolder), false, 80),
                snap_node(&mk, sibling, "outside.txt", None, false, 80),
            ],
        };
        let mut conflicts = Vec::new();
        apply_snapshot(&bridge, dir.path(), &snapshot, 200, 200, &mut conflicts).unwrap();

        assert!(bridge.db().get_file(folder).unwrap().is_none());
        assert!(
            bridge.db().get_file(child).unwrap().is_none(),
            "child listed in a partial snapshot must still be pruned with its absent folder"
        );
        assert!(bridge.db().get_file(subfolder).unwrap().is_none());
        assert!(bridge.db().get_file(grandchild).unwrap().is_none());
        assert!(bridge.db().get_file(sibling).unwrap().is_some());
        assert!(conflicts.is_empty());
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
        bridge
            .db()
            .upsert_file(&FileEntry {
                file_id: stale.into(),
                path: "deleted-server-side.txt".into(),
                status: FileStatus::CloudOnly,
                size_bytes: 1,
                modified_at: 0,
                content_hash: None,
                remote_updated_at: 0,
                parent_id: None,
                item_kind: ItemKind::File,
            })
            .unwrap();

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
        assert!(
            bridge.db().get_file(stale).unwrap().is_none(),
            "snapshot-absent row pruned"
        );
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
        assert!(
            row.is_some(),
            "Trashing row must NOT be pruned while still listed in the snapshot"
        );
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

        let server = SyncMockServer::start(vec![("200 OK".into(), boot), ("200 OK".into(), ops)]);
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
        assert!(
            bridge.db().get_file(doomed).unwrap().is_none(),
            "file_trash removed the row"
        );
        assert_eq!(
            bridge.db().get_sync_cursor().unwrap(),
            Some(3),
            "cursor advanced to max seq_id"
        );
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
        assert_eq!(
            bridge.db().get_sync_cursor().unwrap(),
            Some(8),
            "cursor preserved across 429"
        );
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

        let server = SyncMockServer::start(vec![("200 OK".into(), boot), ("200 OK".into(), ops)]);
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
            bridge
                .db()
                .upsert_file(&FileEntry {
                    file_id: kept.into(),
                    path: "important.txt".into(),
                    status: FileStatus::CloudOnly,
                    size_bytes: 1,
                    modified_at: 0,
                    content_hash: None,
                    remote_updated_at: 0,
                    parent_id: None,
                    item_kind: ItemKind::File,
                })
                .unwrap();
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

    // ── Task 1247: hydrate destination allow-list + real IPC entry point ──────

    /// Item 1: the pure containment predicate `hydrate_file` uses to reject an
    /// out-of-root destination before writing plaintext.
    #[test]
    fn hydrate_dest_is_allowed_covers_the_containment_matrix() {
        let root = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let root_path = root.path();
        let other_path = other.path();

        // An absolute path outside all allowed roots (its parent exists but is
        // NOT under `root`) — the ~/.ssh/authorized_keys style attack — is
        // rejected. `other` exists, so this hits the parent-containment branch.
        let outside = other_path.join("evil.txt");
        assert!(
            !hydrate_dest_is_allowed(&outside, &[root_path]),
            "an existing-parent path outside every root must be rejected"
        );

        // A path with `..` that resolves outside the root is rejected: the
        // parent (`root/..`) is not contained even though the file name is a
        // single component.
        let traversal = root_path.join("../evil.txt");
        assert!(
            !hydrate_dest_is_allowed(&traversal, &[root_path]),
            "a `..` traversal escaping the root must be rejected"
        );

        // The normal hydrate case: a not-yet-existing file directly under the
        // allowed root is accepted (parent contained + single-component name).
        let legit = root_path.join("newfile.bin");
        assert!(
            !legit.exists(),
            "precondition: destination must not exist yet for the common case"
        );
        assert!(
            hydrate_dest_is_allowed(&legit, &[root_path]),
            "a not-yet-existing file under the allowed root must be accepted"
        );

        // Accepted when the destination is under a SECOND allowed root even
        // though it is outside the first (ANY root passing is enough).
        let legit_second = other_path.join("under-second.bin");
        assert!(
            hydrate_dest_is_allowed(&legit_second, &[root_path, other_path]),
            "a destination under any allowed root must be accepted"
        );

        // An empty allow-list fails closed — never vacuously true.
        assert!(
            !hydrate_dest_is_allowed(&legit, &[]),
            "an empty allowed_roots slice must reject everything"
        );

        // A destination whose parent directory does not exist is rejected too
        // (canonicalization of a missing parent fails) — this is exactly the
        // construction the malicious IPC test below relies on.
        let missing_parent = root_path.join("does-not-exist-dir").join("x.txt");
        assert!(
            !hydrate_dest_is_allowed(&missing_parent, &[root_path]),
            "a path under a non-existent subdirectory must be rejected"
        );
    }

    /// Self-terminating HTTP mock for the IPC end-to-end hydration tests. Unlike
    /// `HydrationMockServer` (blocking accept, fixed request count), this one is
    /// non-blocking with a stop flag + deadline, so the malicious test — where
    /// the destination guard fires BEFORE any HTTP call and the server therefore
    /// sees zero requests — can never hang. It also reports how many requests it
    /// actually served, which is what proves `do_hydrate` never ran on rejection.
    struct IpcHydrationMock {
        base_url: String,
        requests: Arc<std::sync::atomic::AtomicUsize>,
        stop: Arc<std::sync::atomic::AtomicBool>,
        handle: thread::JoinHandle<()>,
    }

    impl IpcHydrationMock {
        fn start(file_key: beebeeb_core::kdf::FileKey, chunks: Vec<Vec<u8>>) -> Self {
            use std::sync::atomic::{AtomicBool, AtomicUsize};
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let base_url = format!("http://{}", listener.local_addr().unwrap());
            let size_bytes: usize = chunks.iter().map(|c| c.len()).sum();
            let chunk_count = chunks.len();
            let chunk_size_bytes = chunks.first().map(|c| c.len()).unwrap_or(0);
            let requests = Arc::new(AtomicUsize::new(0));
            let stop = Arc::new(AtomicBool::new(false));
            let (rq, st) = (requests.clone(), stop.clone());
            let handle = thread::spawn(move || {
                let started = std::time::Instant::now();
                loop {
                    if st.load(Ordering::Relaxed) || started.elapsed() > Duration::from_secs(15) {
                        break;
                    }
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            stream.set_nonblocking(false).unwrap();
                            let request = read_http_request(&mut stream);
                            rq.fetch_add(1, Ordering::Relaxed);
                            let response = hydration_mock_response(
                                &request,
                                &file_key,
                                &chunks,
                                size_bytes,
                                chunk_count,
                                chunk_size_bytes,
                            );
                            match response {
                                MockResponse::Text(body) => stream.write_all(body.as_bytes()).unwrap(),
                                MockResponse::Binary(body) => {
                                    let header = format!(
                                        "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                                        body.len()
                                    );
                                    stream.write_all(header.as_bytes()).unwrap();
                                    stream.write_all(&body).unwrap();
                                }
                            }
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Err(e) => panic!("ipc hydration mock accept failed: {e}"),
                    }
                }
            });
            Self {
                base_url,
                requests,
                stop,
                handle,
            }
        }

        fn stop_and_count(self) -> usize {
            self.stop.store(true, Ordering::Relaxed);
            self.handle.join().unwrap();
            self.requests.load(Ordering::Relaxed)
        }
    }

    /// Drive one real `HydrateFile` request over a real Unix socket against a
    /// real `serve_ipc_at` server, returning the daemon's response. This is the
    /// actual IPC entry point an attacker would use — not an internal function.
    fn ipc_hydrate_roundtrip(
        db: Arc<StateDb>,
        bridge: Arc<EngineBridge>,
        file_id: &str,
        dest_path: &Path,
    ) -> crate::ipc_socket::IpcResponse {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let sock_dir = tempfile::tempdir().unwrap();
        let sock_path = sock_dir.path().join("ipc.sock");
        let file_id = file_id.to_string();
        let dest_string = dest_path.to_string_lossy().to_string();
        let sp = sock_path.clone();

        tokio::runtime::Runtime::new().unwrap().block_on(async move {
            let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
            let server = tokio::spawn(crate::ipc_socket::serve_ipc_at(sp.clone(), db, bridge, cancel_rx));

            // Wait for the listener to bind, then connect a real client.
            let mut client = {
                let mut attempt = 0;
                loop {
                    match tokio::net::UnixStream::connect(&sp).await {
                        Ok(s) => break s,
                        Err(_) if attempt < 300 => {
                            attempt += 1;
                            tokio::time::sleep(Duration::from_millis(10)).await;
                        }
                        Err(e) => panic!("could not connect to test IPC socket: {e}"),
                    }
                }
            };

            // Exact wire shape the server deserializes off the socket.
            let req = serde_json::to_vec(&crate::ipc_socket::IpcRequest::HydrateFile {
                file_id: file_id.clone(),
                dest_path: dest_string.clone(),
            })
            .unwrap();
            client.write_all(&req).await.unwrap();

            let mut buf = vec![0u8; 65536];
            let n = client.read(&mut buf).await.unwrap();
            let resp: crate::ipc_socket::IpcResponse =
                serde_json::from_slice(&buf[..n]).expect("daemon must return a valid IpcResponse");

            drop(client);
            let _ = cancel_tx.send(());
            let _ = server.await;
            resp
        })
    }

    /// Item 3 (load-bearing): a malicious `HydrateFile` whose `dest_path` is
    /// outside every allowed root, sent over the REAL IPC socket, is rejected —
    /// the daemon returns `Error`, the target file is NOT created, and the
    /// hydration backend is never even contacted (0 requests), proving the
    /// destination guard short-circuits before any decrypt/download happens.
    #[test]
    fn ipc_rejects_hydrate_write_outside_allowed_roots() {
        let dir = tempfile::tempdir().unwrap();
        let master_key = [9u8; 32];
        let file_key = hydration_test_key(master_key, TEST_FILE_ID);

        // A working backend, so the ONLY thing that can stop the out-of-root
        // write is the guard. If the guard were removed, do_hydrate would
        // succeed and the file would land on disk — failing this test.
        let server = IpcHydrationMock::start(file_key, vec![b"attacker-would-read-this".to_vec()]);

        let db = Arc::new(StateDb::open(dir.path().join("state.db")).unwrap());
        let api = Arc::new(ApiClient::new(server.base_url.clone(), "token".into(), master_key));
        let bridge = Arc::new(EngineBridge::new(db.clone(), api));
        seed_bridge_row(&bridge, TEST_FILE_ID, "/secret.bin", None, FileStatus::CloudOnly, 24);

        // Destination under a non-existent subdirectory of a real temp dir: it
        // is outside the sync root AND its parent cannot be canonicalized, so it
        // is outside every allowed root the IPC handler passes (sync_root? +
        // temp_dir) regardless of this machine's config.
        let malicious_dest = dir.path().join("attacker-controlled-dir").join("authorized_keys");

        let resp = ipc_hydrate_roundtrip(db, bridge, TEST_FILE_ID, &malicious_dest);

        match resp {
            crate::ipc_socket::IpcResponse::Error { message } => {
                assert!(
                    message.contains("allowed root"),
                    "rejection must come from the destination guard, got: {message}"
                );
            }
            other => panic!("expected the malicious write to be rejected, got {other:?}"),
        }

        assert!(
            !malicious_dest.exists(),
            "the malicious destination file must NOT have been created"
        );
        assert!(
            !malicious_dest.parent().unwrap().exists(),
            "the guard must run before create_dir_all — the parent dir must not exist either"
        );

        let served = server.stop_and_count();
        assert_eq!(
            served, 0,
            "the hydration backend must never be contacted when the destination is rejected"
        );
    }

    /// Item 4: the fix does NOT break the legitimate case. The same real IPC
    /// server, given a `dest_path` genuinely under an allowed root (the temp
    /// dir, always an allowed root in the handler), decrypts the real content
    /// and writes it to disk, returning `Ok`.
    #[test]
    fn ipc_allows_hydrate_write_under_allowed_root() {
        let dir = tempfile::tempdir().unwrap();
        let dest_dir = tempfile::tempdir().unwrap(); // under std::env::temp_dir()
        let master_key = [9u8; 32];
        let file_key = hydration_test_key(master_key, TEST_FILE_ID);

        let plaintext = b"beebeeb-1247-real-decrypted-plaintext".to_vec();
        let server = IpcHydrationMock::start(file_key, vec![plaintext.clone()]);

        let db = Arc::new(StateDb::open(dir.path().join("state.db")).unwrap());
        let api = Arc::new(ApiClient::new(server.base_url.clone(), "token".into(), master_key));
        let bridge = Arc::new(EngineBridge::new(db.clone(), api));
        seed_bridge_row(
            &bridge,
            TEST_FILE_ID,
            "/legit.bin",
            None,
            FileStatus::CloudOnly,
            plaintext.len() as i64,
        );

        // dest_dir is created by tempfile under std::env::temp_dir(), which the
        // IPC handler always includes as an allowed root, so this is accepted.
        let legit_dest = dest_dir.path().join("legit-out.bin");

        let resp = ipc_hydrate_roundtrip(db, bridge, TEST_FILE_ID, &legit_dest);

        assert!(
            matches!(resp, crate::ipc_socket::IpcResponse::Ok),
            "a legitimate in-root hydrate must succeed, got {resp:?}"
        );
        assert_eq!(
            std::fs::read(&legit_dest).expect("the decrypted file must exist on disk"),
            plaintext,
            "the real decrypted plaintext must land at the requested path"
        );

        let served = server.stop_and_count();
        assert!(
            served >= 2,
            "the legitimate path must contact the backend for metadata + chunk (got {served})"
        );
    }

    /// Item 5: `serve_ipc_at` hardens the bound socket file to owner-only 0o600.
    #[test]
    fn ipc_socket_file_is_chmod_0600_after_bind() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let sock_dir = tempfile::tempdir().unwrap();
        let sock_path = sock_dir.path().join("perm.sock");

        let db = Arc::new(StateDb::open(dir.path().join("state.db")).unwrap());
        let api = Arc::new(ApiClient::new("https://api.beebeeb.io".into(), "token".into(), [7u8; 32]));
        let bridge = Arc::new(EngineBridge::new(db.clone(), api));

        let sp = sock_path.clone();
        tokio::runtime::Runtime::new().unwrap().block_on(async move {
            let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
            let server = tokio::spawn(crate::ipc_socket::serve_ipc_at(sp.clone(), db, bridge, cancel_rx));

            // Wait for the socket file to appear (i.e. bind + chmod done).
            let mut attempt = 0;
            while !sp.exists() && attempt < 300 {
                attempt += 1;
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            assert!(sp.exists(), "the IPC socket file must be created by serve_ipc_at");

            let mode = std::fs::metadata(&sp).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "the socket file must be chmod 0o600, got {mode:o}");

            let _ = cancel_tx.send(());
            let _ = server.await;
        });
    }

    /// Task 1247 P0 follow-up (TOCTOU symlink race): the write primitive itself
    /// must refuse a symlink destination (O_NOFOLLOW) and create real files
    /// owner-only (0o600). This unit test pins both directly on the primitive.
    #[test]
    fn write_hydrated_plaintext_refuses_symlink_and_sets_0600() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();

        // Success path: a real, new destination is written 0o600 with the exact
        // bytes.
        let real_dest = root.path().join("real-out.bin");
        write_hydrated_plaintext(&real_dest, &[root.path()], b"plaintext-payload").unwrap();
        assert_eq!(std::fs::read(&real_dest).unwrap(), b"plaintext-payload");
        let mode = std::fs::metadata(&real_dest).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "freshly written plaintext must be owner-only, got {mode:o}");

        // Attack path: destination is a symlink pointing OUTSIDE the root at a
        // not-yet-existing target. O_NOFOLLOW must make the open fail closed so
        // the symlink is never followed and its target is never created/written.
        let planted_target = outside.path().join("authorized_keys");
        let symlink_dest = root.path().join("bb_planted_link");
        std::os::unix::fs::symlink(&planted_target, &symlink_dest).unwrap();
        assert!(!planted_target.exists(), "precondition: symlink target must not exist yet");

        let err = write_hydrated_plaintext(&symlink_dest, &[root.path()], b"decrypted-secret").unwrap_err();
        assert!(
            !planted_target.exists(),
            "O_NOFOLLOW must not follow the symlink — the outside target must NOT be created"
        );
        // O_NOFOLLOW hitting a symlink final (leaf) component fails with ELOOP.
        assert_eq!(
            err.raw_os_error(),
            Some(libc::ELOOP),
            "expected O_NOFOLLOW symlink refusal (ELOOP), got {err:?}"
        );
    }

    /// Task 1247 second-review Gap 1 (parent-directory symlink swap): the leaf
    /// O_NOFOLLOW is not enough — an attacker can replace a legitimately-contained
    /// PARENT directory with a symlink during the download window. The write must
    /// anchor to the parent via O_DIRECTORY|O_NOFOLLOW + openat so a symlinked
    /// parent is refused. Here the parent symlink points to a real dir INSIDE the
    /// allowed root, so the containment re-check PASSES — meaning ONLY the
    /// parent-anchoring O_NOFOLLOW can stop it (isolates that defense).
    ///
    /// Load-bearing: with the old leaf-only open, the write would follow the
    /// symlinked parent and create the file at the real inside-root dir.
    #[test]
    fn write_hydrated_plaintext_refuses_symlinked_parent_dir() {
        let root = tempfile::tempdir().unwrap();

        // A real directory inside the allowed root, and a symlink to it (also
        // inside the root) used as the destination's parent.
        let real_subdir = root.path().join("real_dir");
        std::fs::create_dir(&real_subdir).unwrap();
        let symlink_parent = root.path().join("swapped_parent");
        std::os::unix::fs::symlink(&real_subdir, &symlink_parent).unwrap();

        let dest = symlink_parent.join("out.bin");
        let would_leak_to = real_subdir.join("out.bin");

        // Sanity: containment passes (canonicalize resolves the symlink to an
        // in-root real dir), so the parent-anchoring defense is what must refuse.
        assert!(
            hydrate_dest_is_allowed(&dest, &[root.path()]),
            "precondition: the symlinked-parent dest must pass containment"
        );

        let err = write_hydrated_plaintext(&dest, &[root.path()], b"decrypted-secret").unwrap_err();
        assert!(
            !would_leak_to.exists(),
            "must NOT write through a symlinked parent directory"
        );
        // O_DIRECTORY|O_NOFOLLOW on a symlinked parent fails closed. Linux reports
        // ENOTDIR (the un-followed symlink is not a directory); a bare leaf
        // O_NOFOLLOW would report ELOOP — accept either as "symlinked-parent refused".
        assert!(
            matches!(err.raw_os_error(), Some(libc::ELOOP) | Some(libc::ENOTDIR)),
            "O_DIRECTORY|O_NOFOLLOW must refuse a symlinked parent (ELOOP/ENOTDIR), got {err:?}"
        );
    }

    /// Task 1247 second-review Gap 2 (mode ignored for pre-existing files): the
    /// O_CREAT `mode` only applies to a NEWLY created inode, so a pre-existing
    /// world-readable file at an in-root path would keep its perms and leak the
    /// plaintext. The unconditional `fchmod` must force 0o600 regardless.
    ///
    /// Load-bearing: without the fchmod (relying on the open-time mode) the
    /// pre-existing 0o644 file keeps 0o644 after the write.
    #[test]
    fn write_hydrated_plaintext_forces_0600_on_preexisting_file() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        // Attacker pre-creates a mundane, world-readable real file at an in-root
        // path (no symlink trick — passes containment as a genuine regular file).
        let dest = root.path().join("preexisting.bin");
        std::fs::write(&dest, b"attacker-placeholder").unwrap();
        std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(
            std::fs::metadata(&dest).unwrap().permissions().mode() & 0o777,
            0o644,
            "precondition: the pre-existing file must start world-readable"
        );

        write_hydrated_plaintext(&dest, &[root.path()], b"decrypted-secret").unwrap();

        assert_eq!(
            std::fs::read(&dest).unwrap(),
            b"decrypted-secret",
            "the decrypted content must be written"
        );
        let mode = std::fs::metadata(&dest).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o600,
            "fchmod must force owner-only perms even on a pre-existing file, got {mode:o}"
        );
    }

    /// Task 1247 P0 follow-up: the SAME race proven end-to-end through the real
    /// `hydrate_file`. A broken symlink at the destination passes the top-of-fn
    /// containment guard (its `exists()` is false → not-yet-exists branch: parent
    /// contained + single-component name), then a full download+decrypt runs, and
    /// only the O_NOFOLLOW write stops the decrypted plaintext from being written
    /// through the symlink to a target outside the allowed root. Load-bearing:
    /// with a plain `fs::write` the outside target WOULD be created.
    #[test]
    fn hydrate_file_fails_closed_on_symlink_destination_toctou() {
        let dir = tempfile::tempdir().unwrap(); // the allowed root
        let outside = tempfile::tempdir().unwrap(); // outside every allowed root
        let master_key = [9u8; 32];
        let file_key = hydration_test_key(master_key, TEST_FILE_ID);

        let plaintext = b"decrypted-vault-plaintext-must-not-escape".to_vec();
        let server = IpcHydrationMock::start(file_key, vec![plaintext.clone()]);

        let db = Arc::new(StateDb::open(dir.path().join("state.db")).unwrap());
        let api = Arc::new(ApiClient::new(server.base_url.clone(), "token".into(), master_key));
        let bridge = Arc::new(EngineBridge::new(db.clone(), api));
        seed_bridge_row(
            &bridge,
            TEST_FILE_ID,
            "/legit.bin",
            None,
            FileStatus::CloudOnly,
            plaintext.len() as i64,
        );

        // Attacker plants a symlink at the guard-passing destination pointing to a
        // not-yet-existing file OUTSIDE the allowed root (simulating the swap
        // landing during the do_hydrate window — a broken symlink so the guard's
        // exists() check is false and it passes containment).
        let planted_target = outside.path().join("authorized_keys");
        let dest = dir.path().join("bb_target_link");
        std::os::unix::fs::symlink(&planted_target, &dest).unwrap();
        assert!(!planted_target.exists(), "precondition: symlink target must not exist yet");
        // Sanity: the destination genuinely passes the containment guard, so this
        // test really is exercising the write-site O_NOFOLLOW defense, not the guard.
        assert!(
            hydrate_dest_is_allowed(&dest, &[dir.path()]),
            "the symlink destination must pass the containment guard (so only O_NOFOLLOW can stop it)"
        );

        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async { bridge.hydrate_file(TEST_FILE_ID, &dest, &[dir.path()]).await });

        assert!(
            result.is_err(),
            "hydrate_file must fail closed on a symlink destination, got {result:?}"
        );
        assert!(
            !planted_target.exists(),
            "O_NOFOLLOW must prevent following the planted symlink — decrypted plaintext must NOT reach the outside target"
        );

        server.stop_and_count();
    }
}
