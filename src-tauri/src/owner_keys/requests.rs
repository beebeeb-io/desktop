//! Task 2275 T3.2 (L12/L13): the desktop's reader for 2236 request rows.
//!
//! Owns: the request envelope fields, the owner's request private keys, and the
//! row's keys — content under the sealed request key `C`, name under the derived
//! string convention first, with the named R5 legacy reader behind the class and
//! encrypted-name gates.
//! Must not: decide a row's class (core does) or touch any other row kind.
//! Tests: `owner_keys::tests::request`.

use base64::{
    Engine as _,
    engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD},
};
use serde_json::Value;
use std::collections::HashMap;

use beebeeb_core::{
    item_id::ItemClass,
    kdf::{FileKey, MasterKey},
};
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

/// The owner's upload requests, loaded once per resolution. Mirrors the CLI's
/// `RequestKeyResolver`: core's bound reader first (2236), then the named R5
/// legacy reader behind the class and name gates.
#[derive(Default)]
pub(crate) struct RequestKeys {
    wrapped: HashMap<String, (Vec<u8>, Vec<u8>)>,
    privs: HashMap<String, [u8; 32]>,
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

    /// The request's X25519 private key. The pre-2236 wrap binds no request id,
    /// so the context is empty — the same shape the CLI reads.
    fn private(&mut self, master: &MasterKey, request_id: &str) -> Option<[u8; 32]> {
        if let Some(key) = self.privs.get(request_id) {
            return Some(*key);
        }
        let (wrapped, nonce) = self.wrapped.get(request_id)?;
        let key = beebeeb_core::file_request::unwrap_request_private(master, b"", wrapped, nonce).ok()?;
        self.privs.insert(request_id.to_string(), *key);
        Some(*key)
    }

    /// A request row's keys (L12): content key `C`; name key from the derived
    /// convention (string only) first, else `C`. Read-only either way, and the
    /// row is never adopted.
    pub(crate) fn row_keys(&mut self, master: &MasterKey, row: &Value) -> Option<beebeeb_core::owned_keys::RequestRowKeys> {
        use beebeeb_core::kdf::OwnerKeyConvention;
        use beebeeb_core::owned_keys::{KeyOrigin, RequestRowKeys};
        let fields = request_upload_fields(row)?;
        let id = row.get("id")?.as_str()?;
        let encrypted = row.get("name_encrypted")?.as_str()?;
        let (wrapped, nonce) = self.wrapped.get(&fields.request_id)?;
        // Core binds the row id and the request id (2236). It wins when it opens.
        if let Ok(keys) = beebeeb_core::owned_keys::resolve_request_row(
            master,
            id,
            fields.request_id.as_bytes(),
            wrapped,
            nonce,
            &fields.e_pub,
            &fields.wrapped_content_key,
            encrypted,
        ) {
            return Some(keys);
        }
        // Gate the legacy reader exactly as core would: an owner-class id with an
        // encrypted name. A plaintext name authenticates nothing.
        if beebeeb_core::item_id::item_class(id).ok()? != ItemClass::Owner || !encrypted.starts_with('{') {
            return None;
        }
        let private = self.private(master, &fields.request_id)?;
        let content = beebeeb_core::file_request::open_request_upload(
            &private,
            &fields.e_pub,
            b"",
            &fields.wrapped_content_key,
        )
        .ok()?;
        let content_key = FileKey::from_bytes(*content);
        // R5 binds no ids, so its three fields can be grafted onto a binary or
        // upper-repair owner row. Only the string convention is the request shape.
        let derived = match beebeeb_core::kdf::owner_file_key_for_name(master, id, encrypted) {
            Ok((key, OwnerKeyConvention::String)) => beebeeb_core::encrypt::decrypt_name_metadata_with_key(&key, encrypted)
                .ok()
                .map(|name| (key, KeyOrigin::Derived, name)),
            Ok(_) => return None,
            Err(_) => None,
        };
        let (name_key, name_origin, name) = match derived {
            Some(found) => found,
            None => (
                FileKey::from_bytes(*content_key.as_bytes()),
                KeyOrigin::RequestContent,
                beebeeb_core::encrypt::decrypt_name_metadata_with_key(&content_key, encrypted).ok()?,
            ),
        };
        Some(RequestRowKeys {
            content_key,
            name_key,
            name_origin,
            name,
        })
    }
}
