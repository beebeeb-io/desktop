//! Task 2275 T3.2 (L12/L13): the desktop's request-row envelope fields and the
//! owner's request private keys.
//!
//! Owns: decoding the 2236 envelope fields and loading the owner's wrapped
//! request private keys. The row's keys are core's: [`resolve_request_row`]
//! runs the class and encrypted-name checks and opens the bound seal, then the
//! shipped empty-id seal (§15). Desktop keeps no reader of its own.
//! Must not: decide a row's class (core does) or touch any other row kind.
//! Tests: `owner_keys::tests::request`.
//!
//! [`resolve_request_row`]: beebeeb_core::owned_keys::resolve_request_row

use base64::{
    Engine as _,
    engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD},
};
use serde_json::Value;
use std::collections::HashMap;

use beebeeb_core::kdf::MasterKey;
use crate::api_client::ApiClient;

/// A 2236 request upload row's envelope fields (L12). The row is an owner-class
/// item whose content key is the request key `C`, not a derived key.
pub(crate) struct RequestUploadFields {
    pub request_id: String,
    pub e_pub: [u8; 32],
    pub wrapped_content_key: Vec<u8>,
}

/// Decode base64 in any of the four common shapes (the server has sent each).
pub(crate) fn b64_any(text: &str) -> Option<Vec<u8>> {
    let text = text.trim();
    STANDARD
        .decode(text)
        .or_else(|_| STANDARD_NO_PAD.decode(text))
        .or_else(|_| URL_SAFE.decode(text))
        .or_else(|_| URL_SAFE_NO_PAD.decode(text))
        .ok()
}

pub(crate) fn request_upload_fields(row: &Value) -> Option<RequestUploadFields> {
    let request_id = row.get("file_request_id")?.as_str()?.to_string();
    request_id.parse::<uuid::Uuid>().ok()?;
    let e_pub = b64_any(row.get("sender_ephemeral_pubkey")?.as_str()?)?;
    let e_pub: [u8; 32] = e_pub.try_into().ok()?;
    if e_pub.iter().all(|byte| *byte == 0) {
        return None;
    }
    let wrapped_content_key = b64_any(row.get("wrapped_content_key")?.as_str()?)?;
    if wrapped_content_key.is_empty() {
        return None;
    }
    Some(RequestUploadFields {
        request_id,
        e_pub,
        wrapped_content_key,
    })
}

/// The owner's upload requests, loaded once per pass.
#[derive(Default)]
pub(crate) struct RequestKeys {
    wrapped: HashMap<String, (Vec<u8>, Vec<u8>)>,
}

impl RequestKeys {
    /// Build from `GET /api/v1/file-requests`; a failure is an empty set, so a
    /// request row stays unavailable rather than failing the pass.
    pub(crate) async fn load(api: &ApiClient) -> Self {
        let mut keys = Self::default();
        let Ok(body) = api.list_file_requests().await else {
            return keys;
        };
        for row in body["file_requests"].as_array().cloned().unwrap_or_default() {
            let (Some(id), Some(wrapped), Some(nonce)) = (
                row["id"].as_str(),
                row["wrapped_private_key"].as_str(),
                row["wrap_nonce"].as_str(),
            ) else {
                continue;
            };
            if let (Some(wrapped), Some(nonce)) = (b64_any(wrapped), b64_any(nonce)) {
                keys.wrapped.insert(id.to_string(), (wrapped, nonce));
            }
        }
        keys
    }

    /// A request row's keys through core (§15): the class and encrypted-name
    /// checks, the row-bound seal, then the shipped empty-id seal. Content is
    /// `C`; the rename target is the string-derived key and the recorded pin,
    /// never `C`, and `None` for an upper-repair (read-only) name.
    pub(crate) fn row_keys(&mut self, master: &MasterKey, row: &Value) -> Option<beebeeb_core::owned_keys::RequestRowKeys> {
        let fields = request_upload_fields(row)?;
        let id = row.get("id")?.as_str()?;
        let encrypted = row.get("name_encrypted")?.as_str()?;
        let (wrapped, nonce) = self.wrapped.get(&fields.request_id)?;
        // Every request created so far wraps its private key under the empty
        // context: the server mints the request id AFTER the wrap.
        beebeeb_core::owned_keys::resolve_request_row(
            master,
            id,
            b"",
            wrapped,
            nonce,
            &fields.e_pub,
            &fields.wrapped_content_key,
            encrypted,
        )
        .ok()
    }
}

/// The durable request-row fact a row may set or clear (P1-1): only a row that
/// carries the 2236 envelope decides; a synthesized sync row returns `None`.
pub(crate) fn request_row_fact(row: &Value) -> Option<bool> {
    let carries = ["file_request_id", "sender_ephemeral_pubkey", "wrapped_content_key"]
        .iter()
        .all(|field| row.get(field).is_some());
    carries.then(|| request_upload_fields(row).is_some())
}

/// The key and pin a rename of a request row must seal under (§15): core's
/// `RenameTarget` — the string-convention key derived from the row id and the
/// pin the current name records. Never `C`; an upper-repair name is read-only.
pub(crate) fn rename_material(
    master: &MasterKey,
    id: &str,
    name_encrypted: &str,
) -> anyhow::Result<(beebeeb_core::kdf::FileKey, beebeeb_core::chunk_stream::ExpectedChunkFormat)> {
    use beebeeb_core::kdf::OwnerKeyConvention;
    let (name_key, convention) = beebeeb_core::kdf::owner_file_key_for_name(master, id, name_encrypted)?;
    anyhow::ensure!(
        convention != OwnerKeyConvention::UpperRepair,
        "a request row with an upper-repair name is read-only"
    );
    let metadata = beebeeb_core::encrypt::decrypt_name_metadata_with_key(&name_key, name_encrypted)?;
    Ok((
        beebeeb_core::kdf::derive_file_key(master, id.as_bytes()),
        metadata.chunk_format,
    ))
}
