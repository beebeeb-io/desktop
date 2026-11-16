//! File requests (DW-0503, DW-0504, DW-0507): a link anyone can upload into, for a host without a UI toolkit (the
//! native Shared window): [`Engine::file_request_create_for`] and [`Engine::file_request_copy_for`]. The flows moved
//! out of the old shell's `share_v2/requests.rs` (`file_request_create`, `file_request_copy`), which is gone. Closing a request and taking a closed one off the list are [`Engine::share_change`]'s
//! (`ShareChange::CloseRequest`, `ShareChange::HideRequest`); changing a request's expiry has no route on Beebeeb and
//! is not offered.
//!
//! No crypto step is built here. The request is core's `CreateRequest` (a fresh key pair per request, its private
//! half wrapped under the vault key, its public half ONLY in the link, after `#`); copying it again is core's
//! `rebuild_request_link` from what Beebeeb stored. The link leaves this file only inside a [`MadeLink`], which
//! prints nothing of it: a host puts [`MadeLink::expose`] on the clipboard and draws [`MadeLink::display`].
//!
//! Rules held here:
//! - One session only (as [`Engine::share_link_for`]): the token and the vault key of the session of the press,
//!   copied together; asked again before the request and before the link is handed over. Each of those checks is
//!   `link_session.rs`'s, shared with the folder and bundle links.
//! - The folder a request delivers into is one of the account's own folders in its local state (never a received
//!   share, never the Backups tree); none chosen is the account's top level. Anything else: nothing is sent.
//! - Only a 401 is a session that ended. A 403 is Beebeeb refusing (a frozen account), never a signed-out session.
//! - A request Beebeeb does not list for the account, or lists as taken off the list, is not found; one whose stored
//!   key does not open under this vault key is never handed out.
//! - A request that is closed, or past its expiry, takes no more files: its link is refused with words
//!   ([`RequestCopyError::Closed`], [`RequestCopyError::Expired`]) and nothing is copied. Closed is said first, as the
//!   Shared list draws it.
//!
//! BLOCKS on the handle's runtime: call it from a worker thread, never from the UI thread.
//!
//! Must not: log an id, a title, a link or anything the server sends; build a key, a token or a link by hand.

use beebeeb_core::kdf::MasterKey;
use beebeeb_core::share_flow::ShareFlowError;
use beebeeb_core::share_flow::request::{CreateRequest, FILE_REQUESTS_PATH, RequestOptions, rebuild_request_link};
use serde_json::Value;

use super::link_session::{LinkRefusal, get, state_refusal};
use crate::account::SessionGeneration;
use crate::app::share_invite_api::{canonical, own_item};
use crate::app::share_link_api::{MadeLink, ShareLinkError, web_origin};
use crate::app::{CredentialStores, Engine};

#[cfg(test)]
#[path = "file_request_tests.rs"]
mod tests;

/// Why a request's link was not copied again. Fixed words only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestCopyError {
    /// The request is closed: it takes no more files. Nothing was copied.
    Closed,
    /// The request is past its expiry: it takes no more files. Nothing was copied.
    Expired,
    /// Any other refusal, in the link's words.
    Link(ShareLinkError),
}

impl RequestCopyError {
    pub fn words(self) -> &'static str {
        match self {
            Self::Closed => "This request is closed",
            Self::Expired => "This request has expired",
            Self::Link(refused) => refused.words(),
        }
    }
}

impl std::fmt::Display for RequestCopyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.words())
    }
}

impl From<ShareLinkError> for RequestCopyError {
    fn from(refused: ShareLinkError) -> Self {
        Self::Link(refused)
    }
}

impl From<LinkRefusal> for RequestCopyError {
    fn from(refused: LinkRefusal) -> Self {
        Self::Link(refused.into())
    }
}

/// A refusal that is a link's is equal to that link refusal.
impl PartialEq<ShareLinkError> for RequestCopyError {
    fn eq(&self, other: &ShareLinkError) -> bool {
        matches!(self, Self::Link(refused) if refused == other)
    }
}

/// What the new-request form chose. `None` is "not set" (no note, the top level, no limit, never expires).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RequestChoices {
    pub title: String,
    pub note: Option<String>,
    /// One of the account's own folders, by id: where what arrives is put. `None`: the account's top level.
    pub folder_id: Option<String>,
    pub max_files: Option<u32>,
    pub max_bytes: Option<u64>,
    pub expires_in_days: Option<u32>,
}

impl<C: CredentialStores> Engine<C> {
    /// WRITES on the server (one new request): a file request of the account of `session`, and its link. See the
    /// module's doc for every rule.
    pub fn file_request_create_for(
        &self,
        session: SessionGeneration,
        choices: &RequestChoices,
    ) -> Result<MadeLink, ShareLinkError> {
        let app_url = web_origin(&self.config.api_base).ok_or(ShareLinkError::Refused)?;
        let title = choices.title.trim();
        if title.is_empty() {
            return Err(ShareLinkError::Invalid);
        }
        let run = self.link_run(session)?;
        let target_folder_id = match choices.folder_id.as_deref() {
            None => None,
            Some(folder) => Some(self.own_folder_of(session, folder)?),
        };
        let options = RequestOptions {
            title: title.to_string(),
            description: choices.note.clone(),
            target_folder_id,
            max_files: choices.max_files,
            max_total_bytes: choices.max_bytes,
            expires_in_secs: choices.expires_in_days.map(|days| u64::from(days) * 86_400),
        };
        let master = MasterKey::from_bytes(*run.key());
        let started = CreateRequest::start(&app_url, &options, &master);
        drop(master);
        let (mut flow, req) = started.map_err(flow_refusal)?;
        let resp = self.link_send(&run, req)?;
        // Read here, before core: only a 401 is a session that ended.
        match resp.status {
            200..=299 => {}
            401 => return Err(ShareLinkError::SessionEnded),
            _ => return Err(ShareLinkError::Refused),
        }
        let created = flow.on_response(resp).map_err(flow_refusal)?;
        self.hand_over(&run, created.link).map_err(ShareLinkError::from)
    }

    /// READS the server: the link of the account's request `request_id` again, rebuilt by core from what Beebeeb
    /// stored for it. See the module's doc for every rule.
    pub fn file_request_copy_for(
        &self,
        session: SessionGeneration,
        request_id: &str,
    ) -> Result<MadeLink, RequestCopyError> {
        let id = canonical(request_id).ok_or(ShareLinkError::NotFound)?;
        let app_url = web_origin(&self.config.api_base).ok_or(ShareLinkError::Refused)?;
        let run = self.link_run(session)?;
        let resp = self.link_send(&run, get(FILE_REQUESTS_PATH))?;
        let rows = match resp.status {
            200..=299 => resp
                .body
                .get("file_requests")
                .and_then(Value::as_array)
                .ok_or(ShareLinkError::Refused)?,
            401 => return Err(ShareLinkError::SessionEnded.into()),
            _ => return Err(ShareLinkError::Refused.into()),
        };
        let text = |row: &'_ Value, field: &str| row.get(field).and_then(Value::as_str).map(str::to_string);
        let row = rows
            .iter()
            .find(|row| {
                text(row, "id").as_deref() == Some(id) && row.get("hidden").and_then(Value::as_bool) != Some(true)
            })
            .ok_or(ShareLinkError::NotFound)?;
        // A request that takes no more files: its link is refused before its key is opened.
        if let Some(over) = over(row, chrono::Utc::now().timestamp()) {
            return Err(over);
        }
        let master = MasterKey::from_bytes(*run.key());
        let link = rebuild_request_link(
            &app_url,
            &master,
            text(row, "token").as_deref(),
            text(row, "wrapped_private_key").as_deref(),
            text(row, "wrap_nonce").as_deref(),
        )
        .ok_or(ShareLinkError::KeyUnavailable);
        drop(master);
        self.hand_over(&run, link?).map_err(RequestCopyError::from)
    }

    /// `folder` as one of the account's own folders in the local state of `session`.
    fn own_folder_of(&self, session: SessionGeneration, folder: &str) -> Result<String, ShareLinkError> {
        let folder = canonical(folder).ok_or(ShareLinkError::NotFound)?;
        let state = self.local_state().map_err(state_refusal)?;
        if state.session() != session {
            return Err(ShareLinkError::NotSignedIn);
        }
        match state
            .with(|db| own_item(db, folder))
            .map_err(state_refusal)?
            .map_err(|_| ShareLinkError::Refused)?
        {
            Some(true) => Ok(folder.to_string()),
            // A file is no place to deliver into: the choice is refused.
            Some(false) => Err(ShareLinkError::Invalid),
            None => Err(ShareLinkError::NotFound),
        }
    }
}

/// Why a listed request takes no more files at `now` (seconds since 1970), as the Shared list reads its row: closed
/// first, then an expiry at or before `now`. `None`: it is open.
fn over(row: &Value, now: i64) -> Option<RequestCopyError> {
    if row.get("closed").and_then(Value::as_bool) == Some(true) {
        return Some(RequestCopyError::Closed);
    }
    let expiry = row.get("expires_at").and_then(Value::as_str).and_then(seconds_of)?;
    (expiry <= now).then_some(RequestCopyError::Expired)
}

/// A stamp as Beebeeb writes it (RFC 3339), or seconds since 1970.
fn seconds_of(stamp: &str) -> Option<i64> {
    let stamp = stamp.trim();
    chrono::DateTime::parse_from_rfc3339(stamp)
        .map(|at| at.timestamp())
        .ok()
        .or_else(|| stamp.parse::<i64>().ok())
}

/// A core refusal as fixed words. Never formats the error.
fn flow_refusal(error: ShareFlowError) -> ShareLinkError {
    match error {
        ShareFlowError::InvalidInput(_) => ShareLinkError::Invalid,
        ShareFlowError::NotFound => ShareLinkError::NotFound,
        ShareFlowError::Unauthorized => ShareLinkError::SessionEnded,
        _ => ShareLinkError::Refused,
    }
}
