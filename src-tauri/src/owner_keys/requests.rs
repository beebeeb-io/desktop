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

use crate::api_client::ApiClient;
use beebeeb_core::encrypt::NameMetadata;
use beebeeb_core::kdf::{FileKey, MasterKey};

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
    pub(crate) fn row_keys(
        &mut self,
        master: &MasterKey,
        row: &Value,
    ) -> Option<beebeeb_core::owned_keys::RequestRowKeys> {
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

/// What one row's carried 2236 envelope turned out to be (review 4 P2-1/P2-2).
pub(crate) enum EnvelopeOutcome {
    /// The row carries no envelope: an ordinary row.
    Absent,
    /// It carries a well-formed envelope this pass cannot resolve. It is never
    /// an owner row: it stays blocked, with no derived write key.
    Unresolved,
    /// Core resolved it; the record now carries the durable request fact.
    Resolved { key: FileKey, metadata: NameMetadata },
}

impl crate::engine_bridge::EngineBridge {
    /// Apply a carried 2236 envelope to `record` (P1-1/P2-1/P2-2). A malformed
    /// envelope is `Err` for that row only; a well-formed one core cannot
    /// resolve is [`EnvelopeOutcome::Unresolved`], never an owner row.
    pub(crate) async fn resolve_envelope(
        &self,
        master: &MasterKey,
        row: &Value,
        cached: &mut Option<RequestKeys>,
        record: &mut crate::owner_keys::OwnedKeyRecord,
        id: &str,
    ) -> anyhow::Result<EnvelopeOutcome> {
        let Some(_) = request_envelope(row)? else {
            return Ok(EnvelopeOutcome::Absent);
        };
        if cached.is_none() {
            *cached = Some(RequestKeys::load(&self.api()).await);
        }
        let Some(found) = cached.as_mut().and_then(|keys| keys.row_keys(master, row)) else {
            return Ok(EnvelopeOutcome::Unresolved);
        };
        record.request_name = Some(request_name(&found));
        if !record.request_row {
            record.request_row = true;
            self.set_request_row(id, true);
        }
        Ok(EnvelopeOutcome::Resolved {
            key: found.name_key,
            metadata: found.name,
        })
    }
}

/// Review 5 P1-1: a malformed envelope's block must reach the DB even when the
/// row was never seen — only the stored flag refuses every write seam. The
/// served name is untrusted, so a previously stored name stays.
pub(crate) fn block_malformed_row(
    scope: &crate::owner_keys::AccountScope<'_>,
    old: Option<&crate::owner_keys::OwnedKeyRecord>,
    record: &mut crate::owner_keys::OwnedKeyRecord,
) -> anyhow::Result<()> {
    if let Some(old) = old {
        record.name_encrypted.clone_from(&old.name_encrypted);
    }
    record.blocked_fingerprint = Some(record.fingerprint());
    scope.put(record)
}

/// The 2236 envelope a row carries (P1-1): `Ok(None)` when it carries none —
/// a synthesized sync row never decides the durable fact. A row that carries
/// all three fields must carry a well-formed envelope: a malformed one is an
/// error, so the row is unreadable and its stored record is left alone.
/// Whether a carrying row IS a request row is core's to say
/// ([`RequestKeys::row_keys`]), never field presence: the server can plant a
/// well-formed-shaped envelope on a normal owner row.
pub(crate) fn request_envelope(row: &Value) -> anyhow::Result<Option<RequestUploadFields>> {
    // Task 2445: a field counts as carried only when present AND non-null.
    // The server snapshot always emits `"file_request_id": null` (etc.) for
    // ordinary rows; `row.get(field).is_some()` is true for JSON null and
    // would block every snapshot row as "malformed envelope".
    let non_null = |field: &str| row.get(field).is_some_and(|v| !v.is_null());
    let present: Vec<bool> = ["file_request_id", "sender_ephemeral_pubkey", "wrapped_content_key"]
        .iter()
        .map(|field| non_null(field))
        .collect();
    // All absent/null → no envelope.
    if present.iter().all(|p| !p) {
        return Ok(None);
    }
    // All present and non-null → must be well-formed.
    if present.iter().all(|p| *p) {
        return request_upload_fields(row)
            .map(Some)
            .ok_or_else(|| anyhow::anyhow!("a row carries a malformed 2236 request envelope"));
    }
    // A mix of null and non-null is malformed.
    anyhow::bail!("a row carries a malformed 2236 request envelope")
}

/// The key and pin a rename of a request row must seal under (§15, P2-2): the
/// string-derived key from the row id and the pin core read from its current
/// name — whatever key authenticated that name, `C` included. Never `C` itself;
/// an upper-repair name is read-only.
pub(crate) fn rename_material(
    master: &MasterKey,
    id: &str,
    record: &crate::owner_keys::OwnedKeyRecord,
) -> anyhow::Result<(
    beebeeb_core::kdf::FileKey,
    beebeeb_core::chunk_stream::ExpectedChunkFormat,
)> {
    use beebeeb_core::chunk_stream::ExpectedChunkFormat;

    let name = record
        .request_name
        .ok_or_else(|| anyhow::anyhow!("a request row's name is not resolved on this device"))?;
    anyhow::ensure!(
        name.key != RequestNameKey::UpperRepair,
        "a request row with an upper-repair name is read-only"
    );
    Ok((
        beebeeb_core::kdf::derive_file_key(master, id.as_bytes()),
        ExpectedChunkFormat::from_u8(name.pin)?,
    ))
}

pub use beebeeb_engine::owner_keys::{RequestName, RequestNameKey};

/// Core's resolution of a request row's name, in the shape the durable
/// record keeps (P2-2).
pub(crate) fn request_name(keys: &beebeeb_core::owned_keys::RequestRowKeys) -> RequestName {
    RequestName {
        key: RequestNameKey::from(keys.name_origin),
        pin: keys.name.chunk_format.as_u8(),
    }
}
