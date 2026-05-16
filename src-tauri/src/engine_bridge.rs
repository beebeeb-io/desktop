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

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use beebeeb_types::EncryptedBlob;
use serde::{Deserialize, Serialize};

use crate::api_client::ApiClient;
use crate::conflict::{VersionInfo, is_conflict, is_text_file};
use crate::state_db::{
    FileContractState, FileEntry, FileStatus, ItemKind, Namespace, OperationKind, PERMISSION_READ, PERMISSION_SHARE,
    PERMISSION_WRITE, PendingOperation, StateDb,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SharedRootRefreshOutcome {
    pub active_shared_root_ids: Vec<String>,
    pub removed_shared_file_ids: Vec<String>,
    pub removed_cache_paths: Vec<String>,
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
        let file_uuid: uuid::Uuid = file_id
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
        let file_key = beebeeb_core::kdf::derive_file_key(&master_key, file_uuid.as_bytes());

        // Walk chunks. Pre-allocate roughly the file size if known,
        // but fall back to defaults — chunks are encrypted so the
        // ciphertext is always larger than plaintext anyway.
        let approx_size = meta.get("size_bytes").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        let mut plaintext: Vec<u8> = Vec::with_capacity(approx_size);

        for i in 0..chunk_count {
            let chunk_bytes = self.api.download_chunk(file_id, i).await?;
            // Stored format: serde_json::to_vec(&EncryptedBlob) — see
            // repos/cli/src/commands/push.rs:257.
            let blob: EncryptedBlob = serde_json::from_slice(&chunk_bytes)
                .map_err(|e| anyhow::anyhow!("parse chunk {i} as EncryptedBlob: {e}"))?;
            let decrypted = beebeeb_core::encrypt::decrypt_chunk(&file_key, &blob)
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
        let path = f["path"].as_str().unwrap_or("").to_string();

        match bridge.db().get_file(file_id)? {
            None => {
                // (1) New file — insert as cloud_only. base = remote
                // (next-tick conflicts compare against this).
                bridge.db().upsert_file(&FileEntry {
                    file_id: file_id.to_string(),
                    path,
                    status: FileStatus::CloudOnly,
                    size_bytes: size,
                    modified_at: remote_updated,
                    content_hash: None,
                    remote_updated_at: remote_updated,
                })?;
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
            Some(_) => {
                // (3) Pending download/upload/conflict/error — let
                // the dedicated path own its transitions.
                continue;
            }
        }
    }
    Ok(conflicts)
}

#[cfg(test)]
mod tests {
    use super::*;

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
