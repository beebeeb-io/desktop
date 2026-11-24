//! What a file's authenticated metadata says about its content: the chunk-format pin and the
//! version commitment, as core read them from the sealed name.
//!
//! Owns: the one place the engine turns that record into a content reader, a seeking open and a
//! name seal, so no call site builds a reader from a pin alone. Must not: do any cryptography of
//! its own (every seal and open is core's) or take either field from a server field. Core's rule
//! (its `CLAUDE.md`, "Never break"): a v2 version opens only against its record's commitment; a
//! record with neither field opens v1 content only.
//! Tests: `content_record_tests.rs`.

use beebeeb_core::chunk_stream::{ChunkDecryptor, ExpectedChunkFormat, VersionCommitment};
use beebeeb_core::encrypt::NameMetadata;
use beebeeb_core::kdf::FileKey;

/// The content half of a decrypted name record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContentRecord {
    /// The format pin the record carries.
    pub format: ExpectedChunkFormat,
    /// The version the record commits to; `None` for metadata written before commitments.
    pub version: Option<VersionCommitment>,
}

impl ContentRecord {
    /// A record that names nothing: a v1 file's, or a new item's before its first upload.
    pub const UNRECORDED: Self = Self {
        format: ExpectedChunkFormat::Unrecorded,
        version: None,
    };

    /// The content half of `metadata` (from `decrypt_file_metadata[_with_key]`).
    pub fn of(metadata: &NameMetadata) -> Self {
        Self {
            format: metadata.chunk_format,
            version: metadata.version,
        }
    }

    /// A pin with no commitment: a request row's rename material, whose content is not the owner's.
    pub fn pin_only(format: ExpectedChunkFormat) -> Self {
        Self { format, version: None }
    }

    fn metadata(&self) -> NameMetadata {
        NameMetadata {
            name: String::new(),
            mime_type: None,
            chunk_format: self.format,
            version: self.version,
        }
    }

    /// The sequential reader of this record's version. With a commitment it opens only that v2
    /// upload; without one, v1 frames only (a v2 pin without a commitment opens nothing).
    pub fn decryptor(&self, file_key: FileKey, file_id: &str) -> ChunkDecryptor {
        ChunkDecryptor::for_version_with_file_key(file_key, file_id, self.format, self.version)
    }

    /// Core's record-driven seeking open of chunk `index`.
    pub fn open_chunk(
        &self,
        file_key: &FileKey,
        file_id: &str,
        index: u32,
        is_last: bool,
        frame: &[u8],
    ) -> Result<Vec<u8>, beebeeb_core::CoreError> {
        beebeeb_core::chunk_stream::open_recorded_chunk(file_key, file_id, &self.metadata(), index, is_last, frame)
    }

    /// Seal a name for the SAME version (a rename, a move's re-seal): the pin and the commitment
    /// are carried forward unchanged, so the content stays readable under the new name.
    pub fn seal_name(
        &self,
        file_key: &FileKey,
        name: &str,
        mime: Option<&str>,
    ) -> Result<String, beebeeb_core::CoreError> {
        beebeeb_core::encrypt::encrypt_name_carrying_version(file_key, name, mime, &self.metadata())
    }

    /// Seal the name a NEW version publishes at `complete`: this record's pin with the commitment
    /// its writer produced (`ChunkEncryptor::version_commitment_after`).
    pub fn seal_name_for_version(
        &self,
        file_key: &FileKey,
        name: &str,
        mime: Option<&str>,
        version: VersionCommitment,
    ) -> Result<String, beebeeb_core::CoreError> {
        beebeeb_core::encrypt::encrypt_name_for_version(file_key, name, mime, self.format, version)
    }
}

#[cfg(test)]
#[path = "content_record_tests.rs"]
mod tests;
