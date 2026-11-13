//! `open_in_memory`: a file for a viewer, downloaded and decrypted in memory by the hydrate path, nothing recorded.

use super::*;

/// What [`EngineBridge::open_in_memory`] answered.
pub(crate) enum InMemoryOpen {
    /// The whole file, decrypted. Wiped when dropped.
    Opened(Zeroizing<Vec<u8>>),
    /// The file is larger than the caller holds in memory. `size` is what is known of it: the size its metadata
    /// states, or how much there was at least when the download was given up.
    TooLarge { size: u64 },
}

/// A limited download passed its limit ([`EngineBridge::hydrate_whole_file_within`]): the file is at least
/// `at_least` bytes. What was decrypted until then is wiped with the download.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct OverInMemoryLimit {
    pub(super) at_least: u64,
}

impl std::fmt::Display for OverInMemoryLimit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the file is larger than its caller holds in memory")
    }
}

impl std::error::Error for OverInMemoryLimit {}

/// What a chunk's body may be longer than its plaintext: the chunk's own framing (header, nonce, tag), with
/// room to spare for a later chunk format. The plaintext total is held to the limit exactly, after decryption.
pub(super) const CAPPED_FRAME_ROOM: u64 = 64 * 1024;

/// What the metadata of a file with no `download_version` says about its size, before a chunk is asked for:
/// the size it states and the size its chunk count implies (every chunk but the last is a full stride; with no
/// stride named, every chunk but the last holds at least one byte), whichever is larger. `None`: it says nothing
/// usable, and the download's own caps are the bound.
fn unpinned_size_floor(meta: &serde_json::Value) -> Result<Option<u64>, EngineError> {
    let stated = meta.get("size_bytes").and_then(|v| v.as_u64());
    let Some(count) = meta.get("chunk_count") else {
        return Ok(stated);
    };
    // A count that is not a chunk index range (negative, or past `u32`) is not downloaded at all.
    let count = count
        .as_u64()
        .and_then(|count| u32::try_from(count).ok())
        .ok_or_else(|| anyhow::anyhow!("server response has a chunk_count that is not a count"))?;
    let stride = meta
        .get("chunk_size_bytes")
        .and_then(|v| v.as_u64())
        .filter(|stride| *stride > 0)
        .unwrap_or(1);
    let implied = u64::from(count.saturating_sub(1)).saturating_mul(stride);
    Ok(Some(stated.map_or(implied, |stated| stated.max(implied))))
}

impl EngineBridge {
    /// A file for a viewer (the native app's open file): the whole file, downloaded and decrypted IN MEMORY by the
    /// one download path every hydrate uses ([`Self::hydrate_whole_file_within`]: the same keys, the same pin to
    /// the version, the same per-chunk checks, one restart when the version goes away), and nothing else. Unlike a
    /// hydrate it records nothing about the file: no status, no cache row, no fetched version, no file on disk, so
    /// what this PC holds of the file and what sync believes about it are untouched.
    ///
    /// `max_bytes` bounds what is held, whatever the metadata says:
    /// - a pinned version (`download_version`): its manifest's size is checked BEFORE a chunk is asked for, and
    ///   the download path holds every chunk body and the total to that manifest;
    /// - no version named: the size the metadata states, and the size its chunk count implies, are checked
    ///   before a chunk is asked for; then every chunk body is given up while it streams once it passes what is
    ///   left of the limit, and the plaintext total is checked after every chunk. A file that understates its
    ///   size costs at most the limit (and one chunk's framing) in memory, never the file.
    ///
    /// `progress` is `(decrypted bytes, bytes in all)`, called from the download loop.
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
                None => unpinned_size_floor(&meta)?,
            };
            if let Some(size) = size.filter(|size| *size > max_bytes) {
                return Ok(InMemoryOpen::TooLarge { size });
            }
            match self
                .hydrate_whole_file_within(file_id, &meta, progress, None, Some(max_bytes))
                .await
            {
                Ok((plaintext, _)) => Ok(InMemoryOpen::Opened(plaintext)),
                Err(e) => match e.caused_by::<OverInMemoryLimit>() {
                    Some(over) => Ok(InMemoryOpen::TooLarge { size: over.at_least }),
                    None => Err(e),
                },
            }
        })
        .await
    }
}
