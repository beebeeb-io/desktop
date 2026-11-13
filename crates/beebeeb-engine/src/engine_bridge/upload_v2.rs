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

/// A name sealed when its op was queued, as it is SENT: sealed again on `current` when the item's
/// record moved on since (an upload of the same item completed in between, here or on another
/// device, so the queued name carries the previous version's commitment and publishing it would
/// make the new version unreadable). A name that already carries the current record, and one this
/// device cannot open under the item's key (a request row's, a blocked row's), is sent as queued.
pub(super) fn reseal_queued_name(key: &FileKey, current: ContentRecord, queued: &str) -> String {
    let Ok(opened) = beebeeb_core::encrypt::decrypt_file_metadata_with_key(key, queued) else {
        return queued.to_string();
    };
    if ContentRecord::of(&opened) == current {
        return queued.to_string();
    }
    current
        .seal_name(key, &opened.name, opened.mime_type.as_deref())
        .unwrap_or_else(|_| queued.to_string())
}

/// What is done about the file's name once `complete` answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum PublishStep {
    /// The server holds this upload's commitment under this name: remember it, send nothing.
    Held(String),
    /// The server holds no commitment for this upload: publish this name as a metadata update.
    Write(String),
    /// The file's current content is no longer this upload's (a newer version, or the row is
    /// gone): nothing is written and nothing remembered.
    Superseded,
}

/// This upload's commitment (the record of `sent`) on the plaintext name `onto` carries, so a
/// rename the server already holds is kept. `sent` itself when `onto` does not open under the key.
fn commitment_carried_onto(name_key: &FileKey, sent: &str, onto: &str) -> String {
    use beebeeb_core::encrypt::decrypt_file_metadata_with_key;
    let (Ok(ours), Ok(theirs)) = (
        decrypt_file_metadata_with_key(name_key, sent),
        decrypt_file_metadata_with_key(name_key, onto),
    ) else {
        return sent.to_string();
    };
    ContentRecord::of(&ours)
        .seal_name(name_key, &theirs.name, theirs.mime_type.as_deref())
        .unwrap_or_else(|_| sent.to_string())
}

/// A FRESH `complete` answer names the row as that transaction left it. The name this upload sent:
/// the server published it. Any other: the server ignored the completion name, and the row's
/// content is this upload's as of that answer, so its commitment is written onto the name the
/// server named.
pub(super) fn after_fresh_complete(name_key: &FileKey, sent: &str, answered: &str) -> PublishStep {
    if answered == sent {
        PublishStep::Held(sent.to_string())
    } else {
        PublishStep::Write(commitment_carried_onto(name_key, sent, answered))
    }
}

/// A repeated `complete` (the first answer was lost) says nothing about the row, so the row as it is
/// now decides. Its current object version is another one than this session's, or its name carries
/// another upload's commitment under a counter that is not below this one's: superseded. Its name
/// carries this upload's version id (under any name: a rename since is kept): held. Otherwise the
/// server took no commitment for this upload: write it onto the row's current name.
pub(super) fn after_repeated_complete(
    name_key: &FileKey,
    sent: &str,
    session_object_version_id: &str,
    row: &serde_json::Value,
) -> PublishStep {
    use beebeeb_core::encrypt::decrypt_file_metadata_with_key;
    if row["download_version"]["object_version_id"]
        .as_str()
        .is_some_and(|current| current != session_object_version_id)
    {
        return PublishStep::Superseded;
    }
    let Some(served) = row["name_encrypted"].as_str() else {
        return PublishStep::Write(sent.to_string());
    };
    if served == sent {
        return PublishStep::Held(sent.to_string());
    }
    let ours = decrypt_file_metadata_with_key(name_key, sent)
        .ok()
        .and_then(|opened| opened.version);
    let theirs = decrypt_file_metadata_with_key(name_key, served)
        .ok()
        .and_then(|opened| opened.version);
    match (ours, theirs) {
        (Some(ours), Some(theirs)) if theirs.version_id == ours.version_id => PublishStep::Held(served.to_string()),
        (Some(ours), Some(theirs)) if theirs.counter >= ours.counter => PublishStep::Superseded,
        _ => PublishStep::Write(commitment_carried_onto(name_key, sent, served)),
    }
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
    /// needs ([`restore_metadata`]). The server publishes that name in the transaction that makes the
    /// restored version current, and nothing is sent after it, as in core's client: each restore
    /// mints a version, so a request that could fail after it must never make the queue restore again.
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
        if let Some((_, counter)) = sealed {
            // The counter is spent whatever follows. A failure to remember it is logged, never an
            // error: the restore is done, and an error here would run it again.
            if let Err(e) = self.db.observe_file_version(file_id, counter, None) {
                tracing::warn!(file_id = %file_id, error = %e, "restore: the version counter was not remembered");
            }
        }
        Ok(response)
    }

    /// After `complete` answered: make sure the server holds this upload's commitment in the file's
    /// name, then remember it ([`after_fresh_complete`], [`after_repeated_complete`]). A server
    /// that publishes the completion name answers with it, and nothing is sent. A fresh answer that
    /// names another name is a server that ignored it (`BB_VERSION_METADATA` off): the commitment
    /// goes onto that name as a metadata update. A repeat (`already_completed`, which names
    /// nothing) is decided from the row as it is NOW: another upload or a restore may have followed,
    /// and this upload's name is never written over a newer one.
    pub(super) async fn publish_completion_name(
        &self,
        api: &ApiClient,
        file_id: &str,
        name_key: &FileKey,
        sent: &str,
        session_object_version_id: &str,
        answer: &serde_json::Value,
    ) -> Result<(), EngineError> {
        let fresh = answer["already_completed"].as_bool() != Some(true);
        let step = match answer["name_encrypted"].as_str() {
            Some(answered) if fresh => after_fresh_complete(name_key, sent, answered),
            _ => match api.get_file(file_id).await {
                Ok(row) => after_repeated_complete(name_key, sent, session_object_version_id, &row),
                // The row is gone (trashed and erased, or removed elsewhere): nothing to publish.
                Err(error) if matches!(error_http_status(&error), Some(404 | 410)) => PublishStep::Superseded,
                Err(error) => return Err(error.into()),
            },
        };
        match step {
            PublishStep::Held(name) => self.remember_published_name(file_id, name_key, &name),
            PublishStep::Write(name) => {
                api.update_metadata(file_id, Some(&name), crate::api_client::ParentChange::Unchanged)
                    .await?;
                self.remember_published_name(file_id, name_key, &name)
            }
            PublishStep::Superseded => Ok(()),
        }
    }

    /// The key and CURRENT record of `id`, read from the server's row through `api`, as core's
    /// client takes a replacement's from the served metadata: another device may have published a
    /// newer version than this one has seen, and a new version or a re-sealed name built on the
    /// local record alone would reuse a counter or carry a commitment that is no longer current
    /// (the content would then open nowhere). The served name must open under the item's own key;
    /// the third value is the current sealed name. Of this device's record and the served one the
    /// higher counter wins (what this device published may be newer than a row read just before
    /// its completion landed). The served counter is remembered.
    ///
    /// With `local_when_unreadable`, a read the server refuses with 403, 404 or 410 (the row is gone,
    /// or is not this account's to read) and a served name that does not open under this device's key (a request
    /// row's, a blocked row's) give the local record.
    pub(super) async fn row_material(
        &self,
        api: &ApiClient,
        id: &str,
        local_when_unreadable: bool,
    ) -> Result<(FileKey, ContentRecord, Option<String>), EngineError> {
        let (key, local, local_name) = owned_material(&self.db, self.api.master_key(), id)?;
        let row = match api.get_file(id).await {
            Ok(row) => row,
            // The row is gone, or is not this account's to read: the local record. Any other failure
            // of the read (a timeout, a rate limit) fails the attempt: a write built on a stale record
            // is worse than a retry.
            Err(error) if local_when_unreadable && matches!(error_http_status(&error), Some(403 | 404 | 410)) => {
                return Ok((key, local, local_name));
            }
            Err(error) => return Err(error.into()),
        };
        // A folder has no content and so no record to carry.
        if row["is_folder"].as_bool() == Some(true) {
            return Ok((key, local, local_name));
        }
        let opened = row["name_encrypted"].as_str().map(|served| {
            (
                served,
                beebeeb_core::encrypt::decrypt_file_metadata_with_key(&key, served),
            )
        });
        let (served, opened) = match opened {
            Some((served, Ok(opened))) => (served, opened),
            _ if local_when_unreadable => return Ok((key, local, local_name)),
            Some((_, Err(error))) => return Err(error.into()),
            None => return Err(anyhow::anyhow!("the row carries no sealed name").into()),
        };
        let served_record = ContentRecord::of(&opened);
        if let Some(version) = served_record.version {
            self.db.observe_file_version(id, version.counter, None)?;
        }
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

    /// [`Self::row_material`] of a row the backup wrote, through the backup client, as every
    /// request of the pass. The backup's rows are not in the sync tree, so this device holds no key
    /// record of them: a row that is gone or does not authenticate is an error, never a guess.
    pub(super) async fn backup_row_material(
        &self,
        id: &str,
    ) -> Result<(FileKey, ContentRecord, Option<String>), EngineError> {
        self.row_material(self.backup_api(), id, false).await
    }

    /// A name sealed when its op was queued, as it is SENT ([`queued_name_for_sending`]), on the
    /// item's record as the server holds it now ([`Self::row_material`]).
    pub(super) async fn queued_name_on_the_served_record(&self, id: &str, queued: &str) -> Result<String, EngineError> {
        let (key, current, _) = self.row_material(&self.api, id, true).await?;
        Ok(reseal_queued_name(&key, current, queued))
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
