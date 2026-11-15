//! A photo preview's seal: the iPhone app's recipe, from core's public functions, so a preview made on either opens
//! on the other.
//!
//! - key: `kdf::derive_file_key(master, file_id as UTF-8)` (the file's own id, as the server and the local state
//!   name it: canonical lowercase);
//! - wire: `nonce(12) || ciphertext || tag(16)` from `encrypt::encrypt_chunk` (AES-256-GCM, a fresh random nonce);
//! - open: `encrypt::decrypt_chunk` of that blob.
//!
//! NOT the thumbnail subkey and seal (`encrypt_thumbnail_for` / `decrypt_thumbnail`), and not the owned-key record's
//! key (the binary or uppercase id conventions): a preview sealed that way would not open on the phone.
//!
//! Only an owner-class id (`item_id::item_class == Owner`) has a derived key; any other id is refused before a key
//! is made. Plaintext and key material are wiped when dropped.

use beebeeb_core::item_id::{ItemClass, item_class};
use beebeeb_core::kdf::{FileKey, MasterKey, derive_file_key};
use beebeeb_types::{CipherSuite, EncryptedBlob};
use zeroize::Zeroizing;

/// The largest sealed preview the server stores.
pub const MAX_WIRE: usize = 8 * 1024 * 1024;
/// What the seal adds to the plaintext: the nonce and the tag.
pub const OVERHEAD: usize = 12 + 16;
/// The largest plaintext that still fits in [`MAX_WIRE`] once sealed.
pub const MAX_PLAINTEXT: usize = MAX_WIRE - OVERHEAD;

/// Why a preview was not sealed or opened. Fixed words only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SealRefused {
    /// Not an owner-class id: there is no derived key for it.
    NotOwned,
    /// Over [`MAX_WIRE`] once sealed.
    TooLarge,
    /// The cipher refused (a blob that does not open under this key, or a cut one).
    Failed,
}

fn key_for(master: &[u8; 32], file_id: &str) -> Result<FileKey, SealRefused> {
    match item_class(file_id) {
        Ok(ItemClass::Owner) => {}
        _ => return Err(SealRefused::NotOwned),
    }
    Ok(derive_file_key(&MasterKey::from_bytes(*master), file_id.as_bytes()))
}

/// Seal `plaintext` (an encoded preview) for the file `file_id` of the account whose master key is `master`.
pub fn seal_preview(master: &[u8; 32], file_id: &str, plaintext: &[u8]) -> Result<Vec<u8>, SealRefused> {
    if plaintext.len() > MAX_PLAINTEXT {
        return Err(SealRefused::TooLarge);
    }
    let key = key_for(master, file_id)?;
    let blob = beebeeb_core::encrypt::encrypt_chunk(&key, plaintext).map_err(|_| SealRefused::Failed)?;
    let mut wire = Vec::with_capacity(blob.nonce.len() + blob.ciphertext.len());
    wire.extend_from_slice(&blob.nonce);
    wire.extend_from_slice(&blob.ciphertext);
    Ok(wire)
}

/// Open a sealed preview of the file `file_id`. The plaintext is wiped when dropped.
pub fn open_preview(master: &[u8; 32], file_id: &str, wire: &[u8]) -> Result<Zeroizing<Vec<u8>>, SealRefused> {
    if wire.len() > MAX_WIRE {
        return Err(SealRefused::TooLarge);
    }
    if wire.len() < OVERHEAD {
        return Err(SealRefused::Failed);
    }
    let key = key_for(master, file_id)?;
    let blob = EncryptedBlob {
        cipher_suite: CipherSuite::V1Aes256Gcm,
        nonce: wire[..12].to_vec(),
        ciphertext: wire[12..].to_vec(),
    };
    beebeeb_core::encrypt::decrypt_chunk(&key, &blob)
        .map(Zeroizing::new)
        .map_err(|_| SealRefused::Failed)
}
