//! `open_in_memory`: a file for a viewer, downloaded and decrypted in memory by the hydrate path, nothing recorded.

use super::*;

/// What [`EngineBridge::open_in_memory`] answered.
pub(crate) enum InMemoryOpen {
    /// The whole file, decrypted. Wiped when dropped.
    Opened(Zeroizing<Vec<u8>>),
    /// The version that would be downloaded is larger than the caller holds in memory: nothing of it was fetched.
    TooLarge { size: u64 },
}

impl EngineBridge {
    /// A file for a viewer (the native app's open file): the whole file, downloaded and decrypted IN MEMORY by the
    /// one download path every hydrate uses ([`Self::hydrate_whole_file_metered`]: the same keys, the same pin to
    /// the version, the same per-chunk checks, one restart when the version goes away), and nothing else. Unlike a
    /// hydrate it records nothing about the file: no status, no cache row, no fetched version, no file on disk, so
    /// what this PC holds of the file and what sync believes about it are untouched.
    ///
    /// The size is read from the metadata of the version that would be downloaded BEFORE a chunk is asked for: a
    /// file over `max_bytes` is [`InMemoryOpen::TooLarge`] and costs the metadata request alone. `progress` is
    /// `(decrypted bytes, bytes in all)`, called from the download loop.
    pub(crate) async fn open_in_memory(
        &self,
        file_id: &str,
        max_bytes: u64,
        progress: Option<&HydrateProgressFn>,
    ) -> Result<InMemoryOpen, EngineError> {
        let _file_uuid: uuid::Uuid = file_id
            .parse()
            .map_err(|e| anyhow::anyhow!("invalid file_id (not a UUID): {e}"))?;
        let meta = self.api.get_file(file_id).await?;
        self.with_one_restart(file_id, meta, |meta| async move {
            let size = match crate::download_pin::PinnedVersion::from_metadata(&meta)? {
                Some(pin) => Some(pin.plaintext_size_bytes),
                None => meta.get("size_bytes").and_then(|v| v.as_u64()),
            };
            if let Some(size) = size.filter(|size| *size > max_bytes) {
                return Ok(InMemoryOpen::TooLarge { size });
            }
            let (plaintext, _) = self.hydrate_whole_file_metered(file_id, &meta, progress, None).await?;
            // A legacy file whose metadata named no size: held to the same limit once its size is known.
            if plaintext.len() as u64 > max_bytes {
                return Ok(InMemoryOpen::TooLarge {
                    size: plaintext.len() as u64,
                });
            }
            Ok(InMemoryOpen::Opened(plaintext))
        })
        .await
    }
}
