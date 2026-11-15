//! Delete many items from the account's Trash for good with ONE password proof: "Empty Trash" and a multi-selection
//! "Delete permanently" for a host without a UI toolkit. The proof (core's OPAQUE step-up) mints one single-use
//! confirmation token; it is spent on ONE call, `POST /api/v1/files/permanent` with `{ "ids": [..] }`, for up to
//! [`MAX_BULK_IDS`] items. A single item keeps its own route ([`super::trash_purge_api`]).
//!
//! More than [`MAX_BULK_IDS`]: the first [`MAX_BULK_IDS`] go, the rest are NOT sent and [`BulkPurged::left_out`] says
//! how many (the window says so before the press; never a silent partial).
//!
//! Beebeeb answers per item: a trashed folder that still holds something not in the Trash is kept (with the number
//! of items inside) and everything else in the call is deleted. [`BulkPurged::deleted`] is what the server reports
//! deleted, never what was asked.
//!
//! Session rule (as [`Engine::purge_trash_for`]): bound to the session of the press, asked as a person's call under its
//! session-write lock; a session that moved on, or a locked screen, sends nothing. BLOCKS on the handle's runtime:
//! call it from a worker thread.
//!
//! Must not: log the password, a token, an id or a server's words; retry with the same token; call without it.

use beebeeb_core::auth_flow::{HttpRequest, HttpResponse};

use super::trash_purge_api::{PurgeError, is_file_id, prove};
use super::{CredentialStores, Engine};
use crate::account::SessionGeneration;
use crate::auth_transport;
use zeroize::Zeroizing;

/// The server's cap on ids per call (`MAX_BULK_TRASH_IDS`).
pub const MAX_BULK_IDS: usize = 500;

/// Why a folder was kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeptWhy {
    /// It holds this many items that are not in the Trash (restore the folder to see them).
    HoldsLiveItems(u64),
    /// It sits in a loop of folders.
    Loop,
}

/// A folder Beebeeb kept, with the reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Kept {
    pub id: String,
    pub why: KeptWhy,
}

/// What a bulk delete got through.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BulkPurged {
    /// The ids Beebeeb reports deleted for good.
    pub deleted: Vec<String>,
    /// The folders it kept, each with its reason (everything else in the call was deleted).
    pub kept: Vec<Kept>,
    /// How many of the asked ids were not sent (over [`MAX_BULK_IDS`]).
    pub left_out: usize,
    /// Why the call as a whole failed: nothing was deleted then, except for [`PurgeError::NotConfirmed`] (the delete
    /// left and its answer never came back: the items may be gone for good).
    pub error: Option<PurgeError>,
}

fn ids_of(body: &serde_json::Value, key: &str) -> Vec<String> {
    body.get(key)
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str()).map(str::to_string).collect())
        .unwrap_or_default()
}

/// Read the answer of `POST /api/v1/files/permanent`: 200, or 409 `has_live_descendants` / `folder_cycle` (some
/// deleted, some kept), or a failure of the whole call. The caller sets `left_out`.
pub fn parse_answer(resp: &HttpResponse) -> BulkPurged {
    let mut out = BulkPurged::default();
    match resp.status {
        200..=299 => out.deleted = ids_of(&resp.body, "deleted"),
        409 => {
            let code = resp.body.get("error").and_then(|v| v.as_str());
            if matches!(code, Some("has_live_descendants" | "folder_cycle")) {
                out.deleted = ids_of(&resp.body, "deleted");
                for item in resp
                    .body
                    .get("refused")
                    .and_then(|v| v.as_array())
                    .into_iter()
                    .flatten()
                {
                    if let Some(id) = item.get("id").and_then(|v| v.as_str()) {
                        let live = item.get("live_descendants").and_then(|v| v.as_u64()).unwrap_or(0);
                        out.kept.push(Kept {
                            id: id.to_string(),
                            why: KeptWhy::HoldsLiveItems(live),
                        });
                    }
                }
                for id in ids_of(&resp.body, "cycles") {
                    out.kept.push(Kept { id, why: KeptWhy::Loop });
                }
            } else {
                out.error = Some(PurgeError::Failed);
            }
        }
        401 => out.error = Some(PurgeError::SessionEnded),
        429 => out.error = Some(PurgeError::RateLimited(resp.retry_after_secs.unwrap_or(30))),
        // A gateway that answers for a server that may have done it: as unknown as a dropped connection.
        502..=504 => out.error = Some(PurgeError::NotConfirmed),
        _ => out.error = Some(PurgeError::Failed),
    }
    out
}

fn bulk_request(ids: &[String]) -> HttpRequest {
    HttpRequest::session_post("/api/v1/files/permanent".to_string(), serde_json::json!({ "ids": ids }))
}

/// Prove `password` for the session `token` ONCE, then delete `ids` (at most [`MAX_BULK_IDS`], each a file id) with the
/// one token the proof minted. `api_base` has no trailing slash.
pub async fn purge_many(
    api_base: &str,
    client: &reqwest::Client,
    token: &str,
    ids: &[String],
    password: &[u8],
) -> BulkPurged {
    let failed = |error| BulkPurged {
        error: Some(error),
        ..BulkPurged::default()
    };
    if ids.is_empty() || ids.len() > MAX_BULK_IDS || !ids.iter().all(|id| is_file_id(id)) {
        return failed(PurgeError::Failed);
    }
    let grant = match prove(api_base, client, token, password).await {
        Ok(grant) => grant,
        Err(error) => return failed(error),
    };
    match auth_transport::send_to(api_base, client, Some(token), grant.attach(bulk_request(ids))).await {
        Ok(resp) => parse_answer(&resp),
        Err(_) => failed(PurgeError::NotConfirmed),
    }
}

impl<C: CredentialStores> Engine<C> {
    /// WRITES, for good: delete `file_ids` (duplicates dropped) from Trash after proving `password` ONCE, for the
    /// session a person pressed under and no other. Only the first [`MAX_BULK_IDS`] are sent; the others are counted
    /// in [`BulkPurged::left_out`]. BLOCKS on the handle's runtime: call it from a worker thread.
    pub fn purge_trash_bulk_for(&self, session: SessionGeneration, file_ids: &[String], password: &[u8]) -> BulkPurged {
        let mut seen = std::collections::HashSet::new();
        let unique: Vec<String> = file_ids.iter().filter(|id| seen.insert(id.as_str())).cloned().collect();
        let sent = &unique[..unique.len().min(MAX_BULK_IDS)];
        let left_out = unique.len() - sent.len();
        // A person's call (`person_session`): a locked screen refuses it, also for a turn read while it was locked.
        let token = if self.acct.session.is_poisoned() {
            None
        } else {
            self.person_session(|current, generation| {
                (generation == session).then(|| Zeroizing::new(current.token.clone()))
            })
            .ok()
            .flatten()
        };
        let Some(token) = token else {
            return BulkPurged {
                left_out,
                error: Some(PurgeError::NotSignedIn),
                ..BulkPurged::default()
            };
        };
        let base = self.config.api_base.trim_end_matches('/').to_string();
        let mut out = self
            .runtime
            .block_on(purge_many(&base, &self.client, token.as_str(), sent, password));
        out.left_out = left_out;
        if !out.deleted.is_empty() || out.error == Some(PurgeError::NotConfirmed) {
            self.request_resnapshot();
        }
        out
    }
}

#[cfg(test)]
#[path = "trash_bulk_api_tests.rs"]
mod tests;
