//! `open_in_memory`: a file for a viewer, downloaded and decrypted in memory by the hydrate path, nothing recorded.

use super::*;

/// What [`EngineBridge::open_in_memory`] answered.
pub(crate) enum InMemoryOpen {
    /// The whole file, decrypted. Wiped when dropped.
    Opened(Zeroizing<Vec<u8>>),
    /// The file is larger than the caller holds in memory, by the size its metadata states: `size`. Nothing of
    /// it was downloaded.
    TooLarge { size: u64 },
    /// The download passed the caller's limit although the metadata said it would not: it was given up, and
    /// what had been decrypted was wiped. The file's size is not known. `held`: how much plaintext was in
    /// memory when it was given up.
    OverLimit { held: u64 },
}

/// A limited download passed its limit ([`EngineBridge::hydrate_whole_file_within`]). What was decrypted until
/// then (`held` bytes) is wiped with the download. How large the file is, is not known: no size is made up
/// here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct OverInMemoryLimit {
    pub(super) held: u64,
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

/// The longest body a chunk with no manifest may have while `held` bytes of plaintext are in memory under
/// `limit`: what is left of the limit, times [`ENVELOPE_BYTES_PER_BYTE`], plus [`CAPPED_FRAME_ROOM`]. A file
/// with no version named is an old one, and an old chunk may be the JSON envelope (`content_frame`), which
/// writes every byte as a decimal number and a comma: a cap of the plaintext's own length would refuse a file
/// that is within the limit.
pub(super) fn capped_body_len(limit: u64, held: u64) -> u64 {
    limit
        .saturating_sub(held)
        .saturating_mul(ENVELOPE_BYTES_PER_BYTE)
        .saturating_add(CAPPED_FRAME_ROOM)
}

/// The most bytes the JSON envelope spends on one byte of a chunk: "255,".
const ENVELOPE_BYTES_PER_BYTE: u64 = 4;

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
    ///   before a chunk is asked for; then every chunk body is given up while it streams once it passes what a
    ///   chunk of the plaintext still allowed may be on the wire ([`capped_body_len`]), and the plaintext total
    ///   is checked after every chunk. A file that understates its size costs at most the limit in plaintext
    ///   and one capped body in memory, never the file, and answers [`InMemoryOpen::OverLimit`].
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
                    Some(over) => Ok(InMemoryOpen::OverLimit { held: over.held }),
                    None => Err(e),
                },
            }
        })
        .await
    }
}
