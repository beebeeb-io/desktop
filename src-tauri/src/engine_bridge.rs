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
//! ## Divergence from the plan, called out loudly so the next agent
//! doesn't burn cycles rediscovering it
//!
//! 1. **Per-chunk download endpoint does not exist on the server yet.**
//!    The plan calls `api.download_chunk(file_id, idx)` which would hit
//!    `GET /api/v1/files/:id/chunks/:idx`. The server's `chunk_router`
//!    in `repos/server/beebeeb-api/src/routes/files.rs:71` only
//!    registers a PUT for that path. The existing GET is
//!    `/api/v1/files/:id/download` which streams all chunks
//!    concatenated as JSON-serialised `EncryptedBlob`s. To match the
//!    plan's per-chunk model we'd need a small server-side addition
//!    (or to refactor the daemon to use the bulk-download path like
//!    the CLI does). Flagged for follow-up — until then,
//!    `hydrate_file` is a stub that returns an explicit error.
//!
//! 2. **`beebeeb_core::crypto::*` paths in the plan don't exist.** The
//!    real API surface is `beebeeb_core::kdf::derive_file_key(&MasterKey,
//!    &[u8]) -> FileKey` and `beebeeb_core::encrypt::decrypt_chunk(&FileKey,
//!    &EncryptedBlob) -> Result<Vec<u8>>`. The chunk format on the
//!    wire is also a serialised `EncryptedBlob` JSON, not raw nonce ||
//!    ciphertext as the plan's example shows. When the per-chunk GET
//!    endpoint lands, `hydrate_file` will use `kdf::derive_file_key`
//!    + `encrypt::decrypt_chunk` against parsed `EncryptedBlob`s —
//!    same shape as the working code in `repos/cli/src/commands/pull.rs`.

use std::path::Path;
use std::sync::Arc;

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

    /// Download `file_id` from the vault, decrypt, write to
    /// `dest_path`. Called by an OS extension when a cloud-only file
    /// is first opened (Tasks 5-7).
    ///
    /// **Currently a stub.** The per-chunk GET endpoint that the plan
    /// references doesn't exist on the server (see module docs). When
    /// it does, the body will mirror `repos/cli/src/commands/pull.rs::pull_single_file`:
    /// derive a per-file key from the master key + UUID, fetch each
    /// chunk, parse as `EncryptedBlob`, decrypt, concatenate, write.
    pub async fn hydrate_file(
        &self,
        file_id: &str,
        _dest_path: &Path,
    ) -> anyhow::Result<()> {
        self.db.set_status(file_id, FileStatus::Downloading)?;
        // Mark the failure so the OS extension can render an error
        // overlay rather than a blank file. Real implementation lands
        // when the per-chunk GET endpoint ships.
        self.db.set_status(file_id, FileStatus::Error)?;
        Err(anyhow::anyhow!(
            "hydrate_file: server-side per-chunk GET endpoint not implemented yet — \
             see engine_bridge.rs module docs"
        ))
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
