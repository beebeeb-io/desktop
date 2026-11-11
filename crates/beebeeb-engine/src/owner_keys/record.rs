//! The durable key-custody record of a row and the label of a row whose name cannot be shown: the
//! parts of the desktop's `owner_keys` the state database stores and the listing reads. Moved out of
//! `src-tauri/src/owner_keys.rs` and `owner_keys/requests.rs` unchanged; the network and bridge code stays there.

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Where core found a request row's name (P2-2): a derived owner convention,
/// the content key `C` the uploader sealed it under, or an upper-repair name
/// (READ-ONLY: no rename, no new version; an edit is saved as a new item).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RequestNameKey {
    /// Derived under the canonical string id.
    Derived,
    /// Derived under the legacy 16-byte binary UUID.
    DerivedBinary,
    /// The 2236 request-upload content key `C`.
    Content,
    /// Derived under the uppercase id. READ-ONLY.
    UpperRepair,
}

/// Core's resolution of a request row's name: what authenticated it and the
/// chunk-format code its envelope records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestName {
    pub key: RequestNameKey,
    /// [`beebeeb_core::chunk_stream::ExpectedChunkFormat`]'s stable code.
    pub pin: u8,
}

impl From<beebeeb_core::owned_keys::KeyOrigin> for RequestNameKey {
    fn from(origin: beebeeb_core::owned_keys::KeyOrigin) -> Self {
        use beebeeb_core::owned_keys::KeyOrigin;
        match origin {
            KeyOrigin::Derived => Self::Derived,
            KeyOrigin::DerivedBinary => Self::DerivedBinary,
            KeyOrigin::DerivedUpperRepair => Self::UpperRepair,
            // The name authenticated under the content key `C`; a name read
            // through an L8/L7 envelope is not a request row's name at all.
            KeyOrigin::RequestContent | KeyOrigin::OwnerWrap | KeyOrigin::FolderWrap => Self::Content,
        }
    }
}

/// A failed row remains visible and deletable; this label is never ciphertext
/// or a path supplied by the server.
pub fn blocked_name(id: &str) -> String {
    let suffix = Sha256::digest(id.as_bytes())[..8]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    format!("Unavailable file {suffix}")
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnedKeyRecord {
    pub file_id: String,
    pub name_encrypted: String,
    pub owner_wrap: Option<Vec<u8>>,
    pub pending_owner_wrap: Option<Vec<u8>>,
    pub shared_root_id: Option<String>,
    pub folder_wrap: Option<Vec<u8>>,
    pub folder_key_version: Option<i32>,
    pub foreign_added: bool,
    #[serde(default)]
    pub member_added: bool,
    /// Durable L12 fact: this row carries a 2236 request envelope. Persisted so
    /// a download in a later process still takes the request content key.
    #[serde(default)]
    pub request_row: bool,
    /// Core's resolution of a request row's name (P2-2), for a later rename.
    #[serde(default)]
    pub request_name: Option<RequestName>,
    pub blocked_fingerprint: Option<String>,
}

impl OwnedKeyRecord {
    pub fn fingerprint(&self) -> String {
        let mut opaque = self.clone();
        opaque.blocked_fingerprint = None;
        STANDARD.encode(Sha256::digest(serde_json::to_vec(&opaque).expect("opaque record")))
    }
}
