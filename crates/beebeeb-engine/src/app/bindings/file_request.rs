//! File requests (DW-0503, DW-0504, DW-0507): a link anyone can upload into, for a host without a UI toolkit (the
//! native Shared window): [`Engine::file_request_create_for`] and [`Engine::file_request_copy_for`]. The flows moved
//! from the Tauri shell's `share_v2/requests.rs` (`file_request_create`, `file_request_copy`); those are left as they
//! were. Closing a request and taking a closed one off the list are [`Engine::share_change`]'s
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
//!   copied together; asked again before the request and before the link is handed over.
//! - The folder a request delivers into is one of the account's own folders in its local state (never a received
//!   share, never the Backups tree); none chosen is the account's top level. Anything else: nothing is sent.
//! - Only a 401 is a session that ended. A 403 is Beebeeb refusing (a frozen account), never a signed-out session.
//! - A request Beebeeb does not list for the account, or lists as taken off the list, is not found; one whose stored
//!   key does not open under this vault key is never handed out.
//!
//! BLOCKS on the handle's runtime: call it from a worker thread, never from the UI thread.
//!
//! Must not: log an id, a title, a link or anything the server sends; build a key, a token or a link by hand.

use beebeeb_core::auth_flow::{HttpRequest, HttpResponse, Method};
use beebeeb_core::kdf::MasterKey;
use beebeeb_core::share_flow::ShareFlowError;
use beebeeb_core::share_flow::request::{CreateRequest, FILE_REQUESTS_PATH, RequestOptions, rebuild_request_link};
use serde_json::Value;
use zeroize::Zeroizing;

use crate::account::SessionGeneration;
use crate::app::owned_state::LocalStateRefused;
use crate::app::share_invite_api::own_item;
use crate::app::share_link_api::{MadeLink, ShareLinkError, web_origin};
use crate::app::{CredentialStores, Engine};
use crate::auth_transport;

#[cfg(test)]
#[path = "file_request_tests.rs"]
mod tests;

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

/// The session of the press, its token and vault key (copied together), and the transport.
struct Run<'a> {
    session: SessionGeneration,
    token: Zeroizing<String>,
    key: Zeroizing<[u8; 32]>,
    base: &'a str,
    client: reqwest::Client,
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
        let run = self.request_run(session)?;
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
        let master = MasterKey::from_bytes(*run.key);
        let started = CreateRequest::start(&app_url, &options, &master);
        drop(master);
        let (mut flow, req) = started.map_err(flow_refusal)?;
        let resp = self.request_send(&run, req)?;
        // Read here, before core: only a 401 is a session that ended.
        match resp.status {
            200..=299 => {}
            401 => return Err(ShareLinkError::SessionEnded),
            _ => return Err(ShareLinkError::Refused),
        }
        let created = flow.on_response(resp).map_err(flow_refusal)?;
        self.request_hand_over(&run, created.link)
    }

    /// READS the server: the link of the account's request `request_id` again, rebuilt by core from what Beebeeb
    /// stored for it. See the module's doc for every rule.
    pub fn file_request_copy_for(
        &self,
        session: SessionGeneration,
        request_id: &str,
    ) -> Result<MadeLink, ShareLinkError> {
        let id = canonical(request_id).ok_or(ShareLinkError::NotFound)?;
        let app_url = web_origin(&self.config.api_base).ok_or(ShareLinkError::Refused)?;
        let run = self.request_run(session)?;
        let resp = self.request_send(&run, get(FILE_REQUESTS_PATH))?;
        let rows = match resp.status {
            200..=299 => resp
                .body
                .get("file_requests")
                .and_then(Value::as_array)
                .ok_or(ShareLinkError::Refused)?,
            401 => return Err(ShareLinkError::SessionEnded),
            _ => return Err(ShareLinkError::Refused),
        };
        let text = |row: &'_ Value, field: &str| row.get(field).and_then(Value::as_str).map(str::to_string);
        let row = rows
            .iter()
            .find(|row| {
                text(row, "id").as_deref() == Some(id) && row.get("hidden").and_then(Value::as_bool) != Some(true)
            })
            .ok_or(ShareLinkError::NotFound)?;
        let master = MasterKey::from_bytes(*run.key);
        let link = rebuild_request_link(
            &app_url,
            &master,
            text(row, "token").as_deref(),
            text(row, "wrapped_private_key").as_deref(),
            text(row, "wrap_nonce").as_deref(),
        )
        .ok_or(ShareLinkError::KeyUnavailable);
        drop(master);
        self.request_hand_over(&run, link?)
    }

    /// `folder` as one of the account's own folders in the local state of `session`.
    fn own_folder_of(&self, session: SessionGeneration, folder: &str) -> Result<String, ShareLinkError> {
        let folder = canonical(folder).ok_or(ShareLinkError::NotFound)?;
        let state = self.local_state().map_err(local_refusal)?;
        if state.session() != session {
            return Err(ShareLinkError::NotSignedIn);
        }
        match state
            .with(|db| own_item(db, folder))
            .map_err(local_refusal)?
            .map_err(|_| ShareLinkError::Refused)?
        {
            Some(true) => Ok(folder.to_string()),
            // A file is no place to deliver into: the choice is refused.
            Some(false) => Err(ShareLinkError::Invalid),
            None => Err(ShareLinkError::NotFound),
        }
    }

    /// The token and the vault key of `session`, copied together while it is the session in memory.
    fn request_run(&self, session: SessionGeneration) -> Result<Run<'_>, ShareLinkError> {
        let (token, key) = self
            .person_session(|current, _| {
                (
                    Zeroizing::new(current.token.clone()),
                    Zeroizing::new(current.master_key),
                )
            })
            .map_err(|_| ShareLinkError::NotSignedIn)?;
        if !self.session_is_still(session, &key) {
            return Err(ShareLinkError::NotSignedIn);
        }
        let client = auth_transport::client_for(&self.config.api_base).map_err(|_| ShareLinkError::Offline)?;
        Ok(Run {
            session,
            token,
            key,
            base: self.config.api_base.trim_end_matches('/'),
            client,
        })
    }

    /// Send the request of the press, only while its session is still the session in memory.
    fn request_send(&self, run: &Run<'_>, req: HttpRequest) -> Result<HttpResponse, ShareLinkError> {
        if !self.session_is_still(run.session, &run.key) {
            return Err(ShareLinkError::NotSignedIn);
        }
        self.runtime
            .block_on(auth_transport::send_to(run.base, &run.client, Some(&run.token), req))
            .map_err(|_| ShareLinkError::Offline)
    }

    /// The hand-over: only while the session in memory is the very one the token and the key were copied from. The
    /// link goes over as core made it, its key after `#` and nowhere else.
    fn request_hand_over(&self, run: &Run<'_>, link: Zeroizing<String>) -> Result<MadeLink, ShareLinkError> {
        if !self.session_is_still(run.session, &run.key) {
            return Err(ShareLinkError::NotSignedIn);
        }
        let query = link
            .split_once('#')
            .map(|(display, fragment)| format!("{}?key={}", display, fragment));
        let link = query.map(Zeroizing::new).unwrap_or(link);
        Ok(MadeLink {
            session: run.session,
            link,
        })
    }
}

/// An id as the server writes it: a canonical lower-case UUID, never normalized.
fn canonical(id: &str) -> Option<&str> {
    match uuid::Uuid::parse_str(id) {
        Ok(u) if u.hyphenated().to_string() == id => Some(id),
        _ => None,
    }
}

/// A read on the session with no confirm token and no body.
fn get(path: &str) -> HttpRequest {
    let mut req = HttpRequest::session_post(path.to_string(), Value::Null);
    req.method = Method::Get;
    req.body = None;
    req
}

fn local_refusal(refused: LocalStateRefused) -> ShareLinkError {
    match refused {
        LocalStateRefused::SessionEnded => ShareLinkError::SessionEnded,
        LocalStateRefused::Unreadable => ShareLinkError::Refused,
        LocalStateRefused::SignedOut | LocalStateRefused::Locked | LocalStateRefused::SessionChanged => {
            ShareLinkError::NotSignedIn
        }
        // Nothing synced, or a local state that is not this account's: no folder of it is this account's.
        _ => ShareLinkError::NotFound,
    }
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
