//! Chunk format v2 on the upload path: what a retry does with a persisted session, the sealed name
//! a new version publishes at `complete`, and the check that the server holds that name before the
//! upload counts as done.
//!
//! Owns: the resume rule, the completion name and its publication, for the sync-root upload and the
//! backup's uploads alike. Must not: seal or open anything itself (core's `ChunkEncryptor` and
//! `ContentRecord` do) or decide to retry (the queue does).
//!
//! Core's rules this follows (its `CLAUDE.md`, "Never break"): every new version is written as v2
//! and its metadata MUST carry the version commitment; a v2 writer never continues an old session
//! (`resume_policy` continues only v1 onto v1, and this core writes no v1); a replacement's counter
//! is above both the record's and this device's high-water mark.
//! Tests: `engine_bridge/tests/format_v2.rs`.

use super::*;
use crate::content_record::ContentRecord;
use beebeeb_core::chunk_stream::{ChunkEncryptor, ExpectedChunkFormat};
use beebeeb_core::kdf::FileKey;

/// What a retry does with the upload session an earlier attempt persisted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ResumeStep {
    /// The server holds every chunk and `complete` may already have been sent: send `complete`
    /// (again), and nothing else. Abandoning here could delete a version that just became valid.
    CompleteOnly,
    /// Anything else: abandon the session on the server and upload from chunk 0 in a fresh one.
    /// A new writer has a new version id, so its frames never join the frames of an old stream,
    /// whether those are v1 (a session an older build opened) or v2.
    Restart,
}

/// The resume rule. `same_payload`: the retry is for the bytes the session was opened for.
pub(super) fn resume_step(same_payload: bool, acked_chunks: i64, chunk_count: i64) -> ResumeStep {
    if same_payload && chunk_count > 0 && acked_chunks == chunk_count {
        ResumeStep::CompleteOnly
    } else {
        ResumeStep::Restart
    }
}

/// The counter a new version's commitment follows: the higher of the record's and the one this
/// device remembers (core: "above both the served record's and the store's high-water mark").
pub(super) fn previous_counter(record: Option<u64>, remembered: Option<u64>) -> Option<u64> {
    match (record, remembered) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (a, b) => a.or(b),
    }
}

/// The sealed name a NEW version publishes, once its writer sealed frame 0.
///
/// `record` is the item's current record (`ContentRecord::UNRECORDED` for a create). A create
/// records the format its content was written in; a replacement keeps the file's pin (v2 is never
/// recorded on a file with v1 history) and follows its counter.
pub(super) fn seal_completion_name(
    db: &StateDb,
    file_id: &str,
    writer: &ChunkEncryptor,
    name_key: &FileKey,
    record: ContentRecord,
    is_create: bool,
    name: &str,
    mime: Option<&str>,
) -> Result<String, EngineError> {
    let pin = if is_create {
        ExpectedChunkFormat::recorded_for(writer.format())
    } else {
        record.format
    };
    let counter = previous_counter(
        record.version.map(|version| version.counter),
        db.highest_file_version(file_id)?,
    );
    let version = writer.version_commitment_after(counter)?;
    Ok(ContentRecord::pin_only(pin).seal_name_for_version(name_key, name, mime, version)?)
}

/// The plaintext name and MIME type a completion name seals: what the queued sealed name holds
/// under the item's key, else (a create whose id was minted at init, so the queued name was sealed
/// for another id) the op's display name and content type.
pub(super) fn completion_plain_name(
    name_key: &FileKey,
    queued_name_encrypted: &str,
    display_name: Option<String>,
    content_type: Option<&str>,
) -> Result<(String, Option<String>), EngineError> {
    if let Ok(opened) = beebeeb_core::encrypt::decrypt_file_metadata_with_key(name_key, queued_name_encrypted) {
        return Ok((opened.name, opened.mime_type));
    }
    let name = display_name.ok_or_else(|| anyhow::anyhow!("upload operation has no name to seal at completion"))?;
    let mime = content_type
        .map(str::to_string)
        .or_else(|| beebeeb_core::media::guess_mime_type(&name).map(str::to_string));
    Ok((name, mime))
}

/// A name sealed when its op was queued, as it is SENT: sealed again when the item's record moved on
/// since (an upload of the same item completed in between, so the queued name carries the previous
/// version's commitment and publishing it would make the new version unreadable). A name that
/// already carries the current record, and one this device cannot open under the item's key (a
/// request row's, a blocked row's), is sent as it was queued.
pub(super) fn queued_name_for_sending(db: &StateDb, master: &[u8; 32], id: &str, queued: &str) -> String {
    let Ok((key, current, _)) = owned_material(db, master, id) else {
        return queued.to_string();
    };
    let Ok(opened) = beebeeb_core::encrypt::decrypt_file_metadata_with_key(&key, queued) else {
        return queued.to_string();
    };
    if ContentRecord::of(&opened) == current {
        return queued.to_string();
    }
    current
        .seal_name(&key, &opened.name, opened.mime_type.as_deref())
        .unwrap_or_else(|_| queued.to_string())
}

/// The fresh current metadata a version restore publishes, as core's client builds it
/// (`beebeeb-client` `restore_version`): the restored version's own id under a counter above every
/// one this device can see. `Ok(None)`: no version of the file is protected, so the restore sends no
/// body, as before. An unprotected version is never restored over a protected file: its content
/// would have no commitment to open against.
pub(super) fn restore_metadata(
    key: &FileKey,
    current_name_encrypted: &str,
    versions: &serde_json::Value,
    version_id: &str,
    remembered: Option<u64>,
) -> Result<Option<(String, u64)>, EngineError> {
    use beebeeb_core::chunk_stream::VersionCommitment;
    use beebeeb_core::encrypt::decrypt_file_metadata_with_key;

    let current = decrypt_file_metadata_with_key(key, current_name_encrypted)?;
    let listed = versions["versions"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("the version listing has no versions"))?;
    let mut highest = previous_counter(current.version.map(|version| version.counter), remembered);
    let mut restored = None;
    let mut found = false;
    for version in listed {
        let sealed = version["version_metadata_encrypted"].as_str();
        let opened = sealed
            .map(|sealed| decrypt_file_metadata_with_key(key, sealed))
            .transpose()?;
        if let Some(counter) = opened.as_ref().and_then(|opened| opened.version).map(|v| v.counter) {
            highest = previous_counter(highest, Some(counter));
        }
        if version["id"].as_str() == Some(version_id) {
            found = true;
            restored = opened;
        }
    }
    if !found {
        return Err(anyhow::anyhow!("the version to restore is not in the file's version listing").into());
    }
    match restored.and_then(|opened| opened.version.map(|version| (opened, version))) {
        Some((restored, restored_version)) => {
            let counter = highest
                .unwrap_or(restored_version.counter)
                .checked_add(1)
                .ok_or_else(|| anyhow::anyhow!("version counter overflow"))?;
            let commitment = VersionCommitment::new(restored_version.version_id, counter)?;
            let sealed = ContentRecord::pin_only(restored.chunk_format).seal_name_for_version(
                key,
                &current.name,
                current.mime_type.as_deref(),
                commitment,
            )?;
            Ok(Some((sealed, counter)))
        }
        None if current.version.is_none() && remembered.is_none() => Ok(None),
        None => Err(anyhow::anyhow!("a version without a commitment is not restored over a protected file").into()),
    }
}

impl EngineBridge {
    /// `POST .../versions/{version}/restore` for `file_id`, with the fresh metadata a protected file
    /// needs ([`restore_metadata`]), then the same check as after `complete`: the server must hold
    /// that name before the restore counts as done.
    pub(super) async fn restore_version_checked(
        &self,
        file_id: &str,
        version_id: &str,
    ) -> Result<serde_json::Value, EngineError> {
        let (key, _, _) = owned_material(&self.db, self.api.master_key(), file_id)?;
        let row = self.api.get_file(file_id).await?;
        let current_name = row["name_encrypted"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("the file row carries no sealed name"))?;
        let versions = self.api.list_versions(file_id).await?;
        let sealed = restore_metadata(
            &key,
            current_name,
            &versions,
            version_id,
            self.db.highest_file_version(file_id)?,
        )?;
        let response = self
            .api
            .restore_version(file_id, version_id, sealed.as_ref().map(|(name, _)| name.as_str()))
            .await?;
        if let Some((name, _)) = sealed.as_ref() {
            self.publish_completion_name(&self.api, file_id, &key, name, &response)
                .await?;
        }
        Ok(response)
    }

    /// Abandon a persisted session that will not be continued, before the fresh init. Best effort:
    /// a session the server already reaped answers 404, and a refusal leaves the lease to expire
    /// (the fresh init then answers "in progress" and the queue retries). The resume row stays
    /// until the fresh init replaces it.
    pub(super) async fn abandon_before_restart(&self, api: &ApiClient, op_id: &str, file_id: &str) {
        match api.abandon_upload(file_id).await {
            Ok(()) => tracing::info!(
                op_id = %op_id,
                file_id = %file_id,
                "upload: persisted session abandoned; this attempt uploads from the first chunk in a new one"
            ),
            Err(e) => tracing::warn!(
                op_id = %op_id,
                file_id = %file_id,
                reason = error_category(e.as_ref()),
                "upload: could not abandon the persisted session; the new init waits for its lease"
            ),
        }
    }

    /// After `complete` answered: make sure the server holds `sent` as the file's name, then
    /// remember it. A server that publishes the completion name answers with it. One that does not
    /// (`BB_VERSION_METADATA` off, or an `already_completed` repeat, which names nothing) gets the
    /// name as a metadata update: without it the new version's content would be v2 under a name
    /// with no commitment, which opens nowhere. An error leaves the session row in place, so the
    /// retry sends `complete` and this update again.
    pub(super) async fn publish_completion_name(
        &self,
        api: &ApiClient,
        file_id: &str,
        name_key: &FileKey,
        sent: &str,
        answer: &serde_json::Value,
    ) -> Result<(), EngineError> {
        if answer["name_encrypted"].as_str() != Some(sent) {
            api.update_metadata(file_id, Some(sent), crate::api_client::ParentChange::Unchanged)
                .await?;
        }
        self.remember_published_name(file_id, name_key, sent)
    }

    /// The key and CURRENT record of a row the backup wrote, read from the server's row: the backup's
    /// rows are not in the sync tree, so this device holds no key record of them, and a re-seal or a
    /// new version from `ContentRecord::UNRECORDED` would drop the row's commitment or restart its
    /// counter. The row's sealed name must open under the item's own key; the third value is that
    /// name. Through the backup client, as every request of the pass.
    pub(super) async fn backup_row_material(
        &self,
        id: &str,
    ) -> Result<(FileKey, ContentRecord, Option<String>), EngineError> {
        let (key, local, local_name) = owned_material(&self.db, self.api.master_key(), id)?;
        let row = self.backup_api().get_file(id).await?;
        // A folder has no content and so no record to carry.
        if row["is_folder"].as_bool() == Some(true) {
            return Ok((key, local, local_name));
        }
        let Some(served) = row["name_encrypted"].as_str() else {
            return Err(anyhow::anyhow!("the backup row carries no sealed name").into());
        };
        let opened = beebeeb_core::encrypt::decrypt_file_metadata_with_key(&key, served)?;
        let served_record = ContentRecord::of(&opened);
        // What this device published itself may be newer than a row read just before its
        // completion landed; the higher counter wins, as in `owned_material`.
        let local_is_newer = match (local.version, served_record.version) {
            (Some(local), Some(served)) => local.counter > served.counter,
            (Some(_), None) => true,
            _ => false,
        };
        if local_is_newer {
            return Ok((key, local, local_name));
        }
        Ok((key, served_record, Some(served.to_string())))
    }

    /// The server holds `sealed` as `file_id`'s name: remember its counter and the name itself
    /// (`file_version_counters`), so this device's next re-seal or replacement of the item carries
    /// this version's commitment before a sync pass serves the row (`owned_material`).
    pub(super) fn remember_published_name(
        &self,
        file_id: &str,
        name_key: &FileKey,
        sealed: &str,
    ) -> Result<(), EngineError> {
        let opened = beebeeb_core::encrypt::decrypt_file_metadata_with_key(name_key, sealed)?;
        if let Some(version) = opened.version {
            self.db.observe_file_version(file_id, version.counter, Some(sealed))?;
        }
        Ok(())
    }
}
