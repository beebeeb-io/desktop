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
use std::time::{SystemTime, UNIX_EPOCH};

use beebeeb_types::EncryptedBlob;

use crate::api_client::ApiClient;
use crate::conflict::{is_conflict, is_text_file, VersionInfo};
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
    pub async fn auto_resolve_keep_both(
        &self,
        sync_root: &Path,
        entry: &FileEntry,
    ) -> anyhow::Result<String> {
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
        let stem = original
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("file");
        let ext = original
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| format!(".{e}"))
            .unwrap_or_default();
        let conflict_name = format!("{stem} (conflict - {host} - {date}){ext}");
        let conflict_path = original.with_file_name(&conflict_name);

        std::fs::rename(&original, &conflict_path).map_err(|e| {
            anyhow::anyhow!(
                "rename {} -> {}: {e}",
                original.display(),
                conflict_path.display()
            )
        })?;

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
}
