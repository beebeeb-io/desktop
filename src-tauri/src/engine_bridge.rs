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

use std::path::Path;
use std::sync::Arc;

use beebeeb_types::EncryptedBlob;

use crate::api_client::ApiClient;
use crate::state_db::{FileEntry, FileStatus, StateDb};

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
    pub async fn hydrate_file(
        &self,
        file_id: &str,
        dest_path: &Path,
    ) -> anyhow::Result<()> {
        // RAII-style: any early return below the status flip should
        // leave the file in `Error`, not `Downloading`. We do that by
        // wrapping the body in an inner async fn whose Err branch we
        // catch.
        self.db.set_status(file_id, FileStatus::Downloading)?;
        match self.do_hydrate(file_id, dest_path).await {
            Ok(()) => {
                self.db.set_status(file_id, FileStatus::Local)?;
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
            .ok_or_else(|| anyhow::anyhow!("server response missing chunk_count"))?
            as u32;

        // Derive the per-file key. MasterKey::from_bytes consumes the
        // array (it zeroizes on drop), so we copy from the borrow.
        let mk_bytes: [u8; 32] = *self.api.master_key();
        let master_key = beebeeb_core::kdf::MasterKey::from_bytes(mk_bytes);
        let file_key = beebeeb_core::kdf::derive_file_key(&master_key, file_uuid.as_bytes());

        // Walk chunks. Pre-allocate roughly the file size if known,
        // but fall back to defaults — chunks are encrypted so the
        // ciphertext is always larger than plaintext anyway.
        let approx_size = meta
            .get("size_bytes")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;
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
        std::fs::write(dest_path, &plaintext)
            .map_err(|e| anyhow::anyhow!("write {}: {e}", dest_path.display()))?;

        Ok(())
    }
}

// ── Periodic sync tick ────────────────────────────────────────────────────────

/// Pull the user's file list from the API and insert any unknown
/// entries into the state DB as `cloud_only`. Called from
/// [`crate::runner`]'s tick loop. Idempotent — files we already know
/// about are left alone (status, hash, etc. are local truth and must
/// not be clobbered by a remote sweep).
pub async fn sync_tick(bridge: &EngineBridge) -> anyhow::Result<()> {
    let files = bridge.api().list_files(None).await?;
    for f in &files {
        let file_id = f["id"].as_str().unwrap_or_default();
        if file_id.is_empty() {
            continue;
        }
        if bridge.db().get_file(file_id)?.is_some() {
            continue;
        }
        let size = f["size_bytes"].as_i64().unwrap_or(0);
        let modified = f["updated_at"].as_i64().unwrap_or(0);
        let path = f["path"].as_str().unwrap_or("").to_string();
        bridge.db().upsert_file(&FileEntry {
            file_id: file_id.to_string(),
            path,
            status: FileStatus::CloudOnly,
            size_bytes: size,
            modified_at: modified,
            content_hash: None,
        })?;
    }
    Ok(())
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
}
