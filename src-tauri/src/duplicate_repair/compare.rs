//! Task 2274, spec phase 2: compare B's bytes with A's known content (rows L10-L12). Only for the number of pairs the
//! person agreed to download (A4). Each chunk is decrypted into a zeroized buffer, fed to the hash and dropped; no
//! plaintext is kept whole and nothing is written (I6). B is read again first: one that changed since the server read
//! is not downloaded.
use super::model::{Class, Pair, ServerFile};
use super::server::RepairApi;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

/// What the compare found.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Counts {
    pub identical: usize,
    pub differs: usize,
    /// B changed on the server since the server read: left waiting, not downloaded.
    pub changed: usize,
}

/// One chunk of a file, decrypted. The buffer zeroizes itself when it is dropped.
pub(crate) fn decrypt_chunk(key: &beebeeb_core::kdf::FileKey, sealed: &[u8]) -> anyhow::Result<Zeroizing<Vec<u8>>> {
    crate::engine_bridge::decrypt_downloaded_chunk(key, sealed)
        .map(Zeroizing::new)
        .map_err(|_| anyhow::anyhow!("a chunk did not decrypt"))
}

/// The pairs waiting for a compare: L10, after the server read.
fn waiting(pairs: &[Pair]) -> usize {
    pairs.iter().filter(|pair| pair.class == Class::Unknown).count()
}

/// Rows L11/L12 for every pair waiting, when `download_ok` is exactly how many there are (A4).
pub(crate) async fn compare(api: &RepairApi, pairs: &mut [Pair], download_ok: usize) -> anyhow::Result<Counts> {
    if pairs.iter().any(|pair| pair.class == Class::Unknown && pair.server.is_none()) {
        anyhow::bail!("The server read has not run for every pair yet. Run server-read first; nothing was downloaded.");
    }
    let waiting = waiting(pairs);
    if download_ok != waiting {
        anyhow::bail!(
            "--download-ok {download_ok} does not match the {waiting} pairs waiting for a compare; nothing was downloaded."
        );
    }
    let mut counts = Counts::default();
    for pair in pairs.iter_mut().filter(|pair| pair.class == Class::Unknown) {
        let n = pair.n;
        let Some(Some(ServerFile::Found(listed))) = pair.server.as_ref().map(|read| read.b.clone()) else {
            anyhow::bail!("pair {n}: the server read holds no B; run server-read again. Nothing was downloaded.");
        };
        let now = api.file(&pair.b.file_id).await.map_err(|failure| failure.at(n))?;
        let ServerFile::Found(now) = now else {
            counts.changed += 1;
            continue;
        };
        if now.trashed || now.updated_at != listed.updated_at || now.size_bytes != listed.size_bytes {
            counts.changed += 1;
            continue;
        }
        let key = api.file_key(&pair.b.file_id);
        let mut hash = Sha256::new();
        for index in 0..u32::try_from(now.chunk_count)? {
            let sealed = api.chunk(&pair.b.file_id, index).await.map_err(|failure| failure.at(n))?;
            let plain = decrypt_chunk(&key, &sealed).map_err(|_| anyhow::anyhow!("pair {n}: a chunk of B did not decrypt"))?;
            hash.update(plain.as_slice());
        }
        let same = pair.a.local_hash.as_deref().map(str::to_ascii_lowercase) == Some(format!("{:x}", hash.finalize()));
        if same {
            pair.class = Class::Identical;
            counts.identical += 1;
        } else {
            pair.class = Class::Differs;
            counts.differs += 1;
        }
    }
    Ok(counts)
}
