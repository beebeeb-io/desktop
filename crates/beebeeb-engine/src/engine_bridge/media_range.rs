//! Slice M1 (`media-range`): the bridge's half of the media player's range reader (`app::bindings::media_stream`).
//!
//! [`EngineBridge::media_plan`] reads the file's metadata ONCE and pins what every later read is held to: the file
//! key, the version the server names (`download_version`, task 1643; `None` for a legacy file with none), the content
//! record (the chunk-format pin and the version commitment, from the authenticated name), the chunk stride, the chunk
//! count and the plaintext size. [`EngineBridge::media_chunks`] then reads a span of chunks of that version through
//! the index-bound partial read of `chunk_frames.rs` (`download_and_decrypt_chunk_range`): a chunk that is not the
//! last of a whole read is opened by core at its OWN index against the record's version commitment
//! (`ContentRecord::open_chunk` → `open_recorded_chunk` → `open_chunk`, whose AAD is
//! `chunk_aad_v2(file_id, version_id, index, is_last)`), so a served chunk of another index, another upload or
//! another file never opens. No crypto of its own.
//!
//! Not cfg-gated. Nothing is written: no row status, no cache row, no file on disk. No transfer-board entry: the
//! bridge a reader builds is a one-shot one (`EngineBridge::new`) whose board is private and holds no transfer, so
//! the range read's `transfers.report` is a no-op. The plaintext comes back in a [`Zeroizing`] buffer sized once,
//! for exactly the bytes the plan says the span holds.
//!
//! Refused here, before a chunk is asked for (each is a [`MediaPlanRefused`]): metadata that does not state a
//! stride, a count and a size that agree (a legacy row without them), a stride past core's own cap on a chunk
//! (`MAX_CHUNK_SIZE`, 256 MiB: the stride sizes the buffer a read allocates, and the server states it, so a larger
//! one is never allocated for), and v2 content whose record carries no version
//! commitment (core's rule: it opens nothing; the player offers "Open in another app…"). A v1 file is read: v1 frames
//! bind no position, version or truncation, the I9 gap core names for every v1 read (the whole-file read has it too).
//! Must not: log or return a name, a key, an id or a byte; write anything.

use std::ops::Range;

use beebeeb_core::chunk_stream::{ExpectedChunkFormat, MAX_CHUNK_SIZE};
use beebeeb_core::kdf::FileKey;
use serde_json::Value;

use super::*;
use crate::content_record::ContentRecord;
use crate::download_pin::PinnedVersion;

/// Why a file has no media plan. Fixed kinds; none carries a name, an id or a server's words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MediaPlanRefused {
    /// The metadata does not state a chunk stride, a chunk count and a size that agree: which bytes a chunk holds
    /// cannot be known, so no range of the file is read.
    NoGeometry,
    /// The record pins v2 and commits to no version: no frame of it may open (core's rule).
    UncommittedV2,
    /// A span outside the plan was asked for.
    OutsidePlan,
}

impl std::fmt::Display for MediaPlanRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NoGeometry => "the file's metadata does not say how its content is cut",
            Self::UncommittedV2 => "the file's content opens only against a version commitment, and it has none",
            Self::OutsidePlan => "a chunk outside the file was asked for",
        })
    }
}

impl std::error::Error for MediaPlanRefused {}

/// One file, as a player reads it: everything every read is held to, from ONE metadata read. The file key wipes
/// itself when the plan is dropped (core's `FileKey` is `ZeroizeOnDrop`).
pub(crate) struct MediaPlan {
    /// Never printed (see the `Debug` below).
    pub(crate) file_id: String,
    pub(crate) file_key: FileKey,
    /// The version the server named; every chunk request carries it. `None`: a legacy file with none named (the
    /// unpinned route, as the whole-file read takes it); its v2 content stays bound to the record's commitment.
    pub(crate) pin: Option<PinnedVersion>,
    pub(crate) record: ContentRecord,
    /// Plaintext bytes in every chunk but the last.
    pub(crate) chunk_size: u64,
    pub(crate) chunk_count: u32,
    pub(crate) plaintext_size: u64,
    /// The version number the metadata states, when it states one (what a local copy's fallback compares).
    pub(crate) version_number: Option<i64>,
    /// The user's download limit (`desktop.toml`), read once.
    download_kbps_limit: u64,
}

/// The sizes, never the id, the key or the version.
impl std::fmt::Debug for MediaPlan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MediaPlan")
            .field("chunk_size", &self.chunk_size)
            .field("chunk_count", &self.chunk_count)
            .field("plaintext_size", &self.plaintext_size)
            .finish_non_exhaustive()
    }
}

impl MediaPlan {
    /// Plaintext bytes chunk `index` holds by the plan: the stride, or what is left for the last. `None` outside.
    pub(crate) fn chunk_len(&self, index: u32) -> Option<u64> {
        if index >= self.chunk_count {
            return None;
        }
        if let Some(pin) = &self.pin {
            return pin.plaintext_size_of(index..index + 1);
        }
        let before = u64::from(index).checked_mul(self.chunk_size)?;
        Some(self.plaintext_size.checked_sub(before)?.min(self.chunk_size))
    }

    /// Plaintext bytes `chunks` hold by the plan. `None`: not a span inside the file.
    fn span_len(&self, chunks: Range<u32>) -> Option<u64> {
        if chunks.start >= chunks.end || chunks.end > self.chunk_count {
            return None;
        }
        chunks.map(|index| self.chunk_len(index)).sum()
    }
}

/// Whether `count` chunks of `stride` bytes (the last one shorter) hold exactly `size` bytes. An empty file is one
/// empty chunk, as core plans it. A stride past core's cap on a chunk (`MAX_CHUNK_SIZE`) is no real plan: the server
/// states it, and it sizes the buffer each read allocates.
fn geometry_holds(size: u64, stride: u64, count: u32) -> bool {
    if stride == 0 || stride > MAX_CHUNK_SIZE || count == 0 {
        return false;
    }
    let needed = if size == 0 { 1 } else { size.div_ceil(stride) };
    needed == u64::from(count)
}

impl EngineBridge {
    /// The plan of `file_id` for a player: ONE metadata read, its version pinned (see the module's doc). The keys
    /// are resolved the way every download resolves them (`file_keys_for_download`; a file in a shared folder may
    /// record that folder's custody, the one write any download of it can cause).
    pub(crate) async fn media_plan(&self, file_id: &str) -> Result<MediaPlan, EngineError> {
        let _file_uuid: uuid::Uuid = file_id
            .parse()
            .map_err(|e| anyhow::anyhow!("invalid file_id (not a UUID): {e}"))?;
        let meta = self.api.get_file(file_id).await?;
        let pin = PinnedVersion::from_metadata(&meta)?;
        let (chunk_count, plaintext_size, chunk_size) = match &pin {
            Some(pin) => (pin.chunk_count, pin.plaintext_size_bytes, pin.chunk_size_bytes),
            None => {
                let count = meta
                    .get("chunk_count")
                    .and_then(Value::as_u64)
                    .and_then(|count| u32::try_from(count).ok());
                let size = meta.get("size_bytes").and_then(Value::as_u64);
                let stride = meta.get("chunk_size_bytes").and_then(Value::as_u64);
                match (count, size, stride) {
                    (Some(count), Some(size), Some(stride)) => (count, size, stride),
                    _ => return Err(anyhow::Error::new(MediaPlanRefused::NoGeometry).into()),
                }
            }
        };
        if !geometry_holds(plaintext_size, chunk_size, chunk_count) {
            return Err(anyhow::Error::new(MediaPlanRefused::NoGeometry).into());
        }
        let (file_key, name_key) = self.file_keys_for_download(file_id).await?;
        let record = recorded_content_format(&name_key, &meta)?;
        if record.format == ExpectedChunkFormat::V2 && record.version.is_none() {
            return Err(anyhow::Error::new(MediaPlanRefused::UncommittedV2).into());
        }
        let download_kbps_limit = crate::config::DesktopConfig::load_for(self.identity)
            .map(|c| c.download_kbps_limit)
            .unwrap_or(0);
        Ok(MediaPlan {
            file_id: file_id.to_owned(),
            file_key,
            pin,
            record,
            chunk_size,
            chunk_count,
            plaintext_size,
            version_number: meta.get("version_number").and_then(Value::as_i64),
            download_kbps_limit,
        })
    }

    /// The plaintext of chunks `chunks` of the plan's version, decrypted in memory: every chunk request carries the
    /// pinned version (a version that went away is `download_pin::VersionGone`, never a restart on another one), and
    /// every chunk is opened at its own index (see the module's doc). The buffer is sized once, for exactly what the
    /// plan says the span holds, and the read is held to that (a longer body is given up while it streams); a span
    /// that decrypts to any other length is an error and is wiped. A span over `MAX_CHUNK_SIZE` is refused before
    /// anything is allocated (the reader asks for one chunk at a time, which the plan holds under it).
    pub(crate) async fn media_chunks(
        &self,
        plan: &MediaPlan,
        chunks: Range<u32>,
    ) -> Result<Zeroizing<Vec<u8>>, EngineError> {
        let want = plan
            .span_len(chunks.clone())
            .filter(|want| *want <= MAX_CHUNK_SIZE)
            .ok_or_else(|| anyhow::Error::new(MediaPlanRefused::OutsidePlan))?;
        let hint = usize::try_from(want).map_err(|_| anyhow::Error::new(MediaPlanRefused::OutsidePlan))?;
        let data = self
            .download_and_decrypt_chunk_range(
                &plan.file_id,
                &plan.file_key,
                chunks,
                plan.download_kbps_limit,
                hint,
                None,
                None,
                plan.pin.as_ref(),
                plan.record,
                plan.chunk_count,
                Some(want),
            )
            .await?;
        if data.len() as u64 != want {
            return Err(anyhow::anyhow!("the range read is {} bytes, the plan says {want}", data.len()).into());
        }
        Ok(data)
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_CHUNK_SIZE, geometry_holds};

    #[test]
    fn media_plan_geometry_is_the_stride_rule() {
        assert!(geometry_holds(3_500, 1_024, 4));
        assert!(geometry_holds(4_096, 1_024, 4));
        assert!(!geometry_holds(4_097, 1_024, 4), "a byte past the last chunk");
        assert!(!geometry_holds(3_072, 1_024, 4), "a chunk with nothing in it");
        assert!(geometry_holds(0, 1_024, 1), "an empty file is one empty chunk");
        assert!(!geometry_holds(10, 0, 1), "no stride");
        assert!(!geometry_holds(10, 1_024, 0), "no chunk");
        // The server states the stride, and it sizes what a read allocates: never past core's cap on a chunk.
        assert!(
            geometry_holds(MAX_CHUNK_SIZE, MAX_CHUNK_SIZE, 1),
            "core's largest chunk"
        );
        assert!(
            !geometry_holds(MAX_CHUNK_SIZE + 1, MAX_CHUNK_SIZE + 1, 1),
            "a stride past core's cap is refused before any allocation"
        );
        assert!(!geometry_holds(1 << 40, 1 << 40, 1), "a stated terabyte chunk");
    }
}
