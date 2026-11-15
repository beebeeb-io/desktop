//! A link to one of the account's files, for a host without a UI toolkit (the native Files window's Copy link and
//! its Share dialog): [`Engine::share_link_for`]. The flow moved from the Tauri shell's `share_v2/flows.rs`
//! (`share_link_copy`, a single file); that command is left as it was.
//!
//! No crypto step is built here: the link is core's double-encrypted link (`share_flow::link::CreateLink`), and a
//! link the account already has is rebuilt and checked by core (`share_flow::link::rebuild_link`). The key that
//! opens the file is resolved as the sync engine resolves it: the item's key record in the account's local state,
//! else, for an id of the owner's class, core's derivation; any other id has no key here.
//!
//! One session only. `session` is [`Engine::session_turn`] as the host read it AT THE PRESS. The token and the
//! vault key are copied together, only while the session in memory is that one (`AccountRuntime::while_session_is`),
//! and the same question is asked again before EACH request and before the link is handed over: a press made
//! under one session never sends under another, and its link is never handed to another.
//!
//! Never two links behind a person's back: every `POST` is a new public link. A plain Copy link (no choices) first
//! reads the account's links; when the file has a live one, that one is rebuilt (and handed out only if it opens
//! to this file's own key: the list is the server's word), or the call ends with [`ShareLinkError::HasLink`] and
//! nothing is made. A read that fails makes nothing either.
//!
//! The link carries the key that opens the file (after `#`). It leaves this file only inside a [`MadeLink`], which
//! prints nothing of it; no error holds a byte of it, of a token or of anything the server sent.
//!
//! BLOCKS on the handle's runtime: call it from a worker thread, never from the UI thread.
//!
//! Must not: log an id, a name, a link or anything the server sends; build a key, a token or a link by hand.

use beebeeb_core::auth_flow::{HttpRequest, HttpResponse, Method};
use beebeeb_core::item_id::{ItemClass, item_class};
use beebeeb_core::kdf::{FileKey, MasterKey};
use beebeeb_core::share_flow::link::{CreateLink, LinkOptions, LinkStep, open_link_file_key, rebuild_link};
use beebeeb_core::share_flow::{self, ShareFlowError};
use serde_json::Value;
use zeroize::Zeroizing;

use super::owned_state::LocalStateRefused;
use super::{CredentialStores, Engine};
use crate::account::SessionGeneration;
use crate::state_db::{FileStatus, StateDb};
use crate::{auth_transport, owner_keys};

#[cfg(test)]
#[path = "share_link_api_tests.rs"]
mod tests;

/// The local web app of a development stack (the API on loopback).
const LOCAL_WEB_APP: &str = "http://localhost:5173";

/// Where a link points: the web app of the SAME deployment as the API this handle talks to, so a build pointed
/// at another API never hands out a link to production. `https://api.<domain>` is `https://app.<domain>`; an
/// API on loopback is the local web app; for any other API base no link is made. (Core refuses anything that
/// is not `https` or loopback as well.)
fn web_origin(api_base: &str) -> Option<String> {
    let base = api_base.trim_end_matches('/');
    if let Some(domain) = base.strip_prefix("https://api.") {
        let plain = |c: char| c.is_ascii_alphanumeric() || c == '.' || c == '-';
        return (!domain.is_empty() && domain.chars().all(plain)).then(|| format!("https://app.{domain}"));
    }
    let host = base.strip_prefix("http://")?;
    let host = host.split(':').next().unwrap_or_default();
    matches!(host, "localhost" | "127.0.0.1").then(|| LOCAL_WEB_APP.to_string())
}
const MINE: &str = "/api/v1/shares/mine";

/// What the Share dialog chose for a new link. `None` is "never" / "no password".
#[derive(Default)]
pub struct LinkChoices {
    pub expires_in_hours: Option<u32>,
    pub passphrase: Option<Zeroizing<String>>,
}

/// Whether there is a password, never the password.
impl std::fmt::Debug for LinkChoices {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LinkChoices")
            .field("expires_in_hours", &self.expires_in_hours)
            .field("passphrase", &self.passphrase.as_ref().map(|_| "(set)"))
            .finish()
    }
}

/// A link Beebeeb accepted, and the session it was made for. The link is wiped when this is dropped.
pub struct MadeLink {
    /// A host compares it with [`Engine::session_turn`] and drops a link whose session is gone.
    pub session: SessionGeneration,
    link: Zeroizing<String>,
}

impl MadeLink {
    /// The whole link, key part included: for the clipboard and nothing else.
    pub fn expose(&self) -> &str {
        &self.link
    }

    /// The link without its key part: what may be drawn.
    pub fn display(&self) -> String {
        share_flow::display_form(&self.link)
    }
}

/// Nothing of the link: its key part opens the file.
impl std::fmt::Debug for MadeLink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MadeLink")
            .field("session", &self.session)
            .field("link", &"(a link)")
            .finish()
    }
}

/// Why no link was handed over. Fixed words only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShareLinkError {
    /// No session in memory, or not the session of the press any more. Nothing (more) was sent.
    NotSignedIn,
    Offline,
    /// Beebeeb no longer accepts this session (401).
    SessionEnded,
    /// Not an id, not an item of this account's local state, or Beebeeb knows no such file for this account.
    NotFound,
    /// A folder: this call makes a link to one file. Nothing was sent.
    NotAFile,
    /// The key that opens the file is not on this computer. Nothing was sent.
    KeyUnavailable,
    /// The file has a live link that cannot be rebuilt and checked here (the account's link list does not carry
    /// what the check needs yet). No second link was made.
    HasLink,
    /// A choice that is refused (a password of spaces only or too short, an expiry that is too long). Nothing
    /// was sent.
    Invalid,
    /// Beebeeb refused or failed.
    Refused,
}

impl ShareLinkError {
    pub fn words(self) -> &'static str {
        match self {
            Self::NotSignedIn => "not signed in",
            Self::Offline => "offline",
            Self::SessionEnded => "the session ended",
            Self::NotFound => "not found",
            Self::NotAFile => "not a file",
            Self::KeyUnavailable => "the key is not available",
            Self::HasLink => "it has a link that cannot be copied again yet",
            Self::Invalid => "a choice was refused",
            Self::Refused => "refused",
        }
    }
}

impl std::fmt::Display for ShareLinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.words())
    }
}

impl<C: CredentialStores> Engine<C> {
    /// WRITES on the server (at most one new link): the link to the file `file_id` of the account of `session`.
    /// `choices`: `None`, or choices with nothing chosen, is the plain Copy link (the file's live link if it has
    /// one, else a new link that never expires); an expiry or a password asks for a new link with them. See the module's doc for the session rule,
    /// for "never two links" and for what leaves this call.
    pub fn share_link_for(
        &self,
        session: SessionGeneration,
        file_id: &str,
        choices: Option<&LinkChoices>,
    ) -> Result<MadeLink, ShareLinkError> {
        let id = canonical(file_id)?;
        let app_url = web_origin(&self.config.api_base).ok_or(ShareLinkError::Refused)?;
        // A password that was asked for and is only spaces: core would drop it and make a link that asks for
        // none. Such a link is never made.
        let blank = |c: &LinkChoices| c.passphrase.as_ref().is_some_and(|typed| typed.trim().is_empty());
        if choices.is_some_and(blank) {
            return Err(ShareLinkError::Invalid);
        }
        // A session mutex a panic poisoned: nothing is sent.
        if self.acct.session.is_poisoned() {
            return Err(ShareLinkError::NotSignedIn);
        }
        // The token and the vault key of the SAME session, and only the one of the press.
        let (token, key) = self
            .acct
            .while_session_is(&self.writes, session, |current| {
                (
                    Zeroizing::new(current.token.clone()),
                    Zeroizing::new(current.master_key),
                )
            })
            .ok_or(ShareLinkError::NotSignedIn)?;
        let master = MasterKey::from_bytes(*key);
        // From the signed-in account's local state: an id it does not hold is not found, and nothing is sent.
        let file_key = self
            .local_state()
            .map_err(local_refusal)?
            .with(|db| key_of(db, &master, id))
            .map_err(local_refusal)??;
        let client = auth_transport::client_for(&self.config.api_base).map_err(|_| ShareLinkError::Offline)?;
        let base = self.config.api_base.trim_end_matches('/');
        // Asked last before every request, with no session change half done.
        let send = |req: HttpRequest| -> Result<HttpResponse, ShareLinkError> {
            if !self.session_is_still(session, &key) {
                return Err(ShareLinkError::NotSignedIn);
            }
            self.runtime
                .block_on(auth_transport::send_to(base, &client, Some(&token), req))
                .map_err(|_| ShareLinkError::Offline)
        };

        let mut link = None;
        // Nothing chosen (no choices at all, or a dialog left on "never" with no password) is the plain request:
        // the file's own live link, never a second one behind it. Only a real choice asks for a new link.
        let plain = choices.is_none_or(|c| c.expires_in_hours.is_none() && c.passphrase.is_none());
        if plain {
            let mine = listed(send(get(format!("{MINE}?file_id={id}")))?)?;
            let live = live_links(&mine, id);
            link = live
                .iter()
                .find_map(|share| rebuilt(&app_url, &master, &file_key, share));
            // A live link that cannot be checked is not handed out, and no second one is made.
            if link.is_none() && !live.is_empty() {
                return Err(ShareLinkError::HasLink);
            }
        }
        let link = match link {
            Some(link) => link,
            None => {
                let options = LinkOptions {
                    expires_in_hours: choices.and_then(|c| c.expires_in_hours),
                    max_opens: None,
                    passphrase: choices.and_then(|c| c.passphrase.clone()),
                };
                let (mut flow, mut req) =
                    CreateLink::start(&app_url, id, &master, &file_key, &options).map_err(flow_refusal)?;
                loop {
                    match flow.on_response(send(req)?).map_err(flow_refusal)? {
                        LinkStep::Send(next) => req = next,
                        LinkStep::Created(created) => break created.link,
                    }
                }
            }
        };
        drop(file_key);
        // The hand-over: only while the session in memory is the very one the token and the key were copied from.
        if !self.session_is_still(session, &key) {
            return Err(ShareLinkError::NotSignedIn);
        }
        Ok(MadeLink { session, link })
    }
}

/// An id as the server writes it: a canonical lower-case UUID, never normalized (an id is key-derivation input).
fn canonical(id: &str) -> Result<&str, ShareLinkError> {
    match uuid::Uuid::parse_str(id) {
        Ok(u) if u.hyphenated().to_string() == id => Ok(id),
        _ => Err(ShareLinkError::NotFound),
    }
}

fn local_refusal(refusal: LocalStateRefused) -> ShareLinkError {
    match refusal {
        LocalStateRefused::SessionEnded => ShareLinkError::SessionEnded,
        LocalStateRefused::Unreadable => ShareLinkError::Refused,
        LocalStateRefused::SignedOut | LocalStateRefused::Locked => ShareLinkError::NotSignedIn,
        // Nothing synced, or a local state that is not this account's: no item of it is this account's.
        _ => ShareLinkError::NotFound,
    }
}

/// The key that opens the file `id`, from the account's local state (the rule of `owner_keys`'
/// `resolved_owned_key`): its key record, else an owner-class id's derivation. A folder is refused before any key
/// is looked up.
fn key_of(db: &StateDb, master: &MasterKey, id: &str) -> Result<FileKey, ShareLinkError> {
    let entry = db
        .get_file(id)
        .map_err(|_| ShareLinkError::Refused)?
        .filter(|entry| entry.status != FileStatus::Trashing)
        .ok_or(ShareLinkError::NotFound)?;
    if entry.is_dir() {
        return Err(ShareLinkError::NotAFile);
    }
    let unavailable = ShareLinkError::KeyUnavailable;
    if let Some(record) = db.get_owned_key_record(id).map_err(|_| unavailable)? {
        return owner_keys::record_key(master, &record, owner_keys::kind_in_db(db, id))
            .map(|(key, _)| key)
            .map_err(|_| unavailable);
    }
    match item_class(id) {
        Ok(ItemClass::Owner) => Ok(beebeeb_core::kdf::derive_file_key(master, id.as_bytes())),
        _ => Err(unavailable),
    }
}

/// A read on the session with no confirm token and no body.
fn get(path: String) -> HttpRequest {
    let mut req = HttpRequest::session_post(path, Value::Null);
    req.method = Method::Get;
    req.body = None;
    req
}

/// The account's links as Beebeeb listed them. An answer that is not a list of links is a read that failed: it
/// is never taken for "no links" (a link would be made on top of ones that were not seen).
fn listed(resp: HttpResponse) -> Result<Vec<Value>, ShareLinkError> {
    match resp.status {
        200..=299 => match resp.body.get("shares").and_then(Value::as_array) {
            Some(shares) => Ok(shares.clone()),
            None => Err(ShareLinkError::Refused),
        },
        401 => Err(ShareLinkError::SessionEnded),
        _ => Err(ShareLinkError::Refused),
    }
}

/// The live links to the file `id` in the account's link list: named for this file (asked of each row here, not
/// left to the server's filter), a file link, not revoked, not past its expiry, its opens not used up.
fn live_links<'a>(mine: &'a [Value], id: &str) -> Vec<&'a Value> {
    let now = chrono::Utc::now();
    let text = |share: &'a Value, field: &str| share.get(field).and_then(Value::as_str);
    let count = |share: &Value, field: &str| share.get(field).and_then(Value::as_i64);
    mine.iter()
        .filter(|share| {
            let expired = text(share, "expires_at")
                .and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok())
                .is_some_and(|at| at <= now);
            let used_up = count(share, "max_opens").is_some_and(|most| count(share, "open_count").unwrap_or(0) >= most);
            text(share, "file_id") == Some(id)
                && share.get("revoked").and_then(Value::as_bool) != Some(true)
                && text(share, "share_type").unwrap_or("file") == "file"
                && !expired
                && !used_up
        })
        .collect()
}

/// A live link rebuilt from its owner copies and checked against its stored wrapped file key, by core, and then
/// held to THIS file: the link's key part must open the row's wrapped key to the very key that opens the file
/// asked for (core's recipient-side opener). The list is the server's word: a row that names this file and
/// carries another file's link is not handed out. `None` when a part is absent, does not open, or is not this
/// file's.
fn rebuilt(app_url: &str, master: &MasterKey, file_key: &FileKey, share: &Value) -> Option<Zeroizing<String>> {
    let text = |field: &str| share.get(field).and_then(Value::as_str);
    let wrapped = text("wrapped_file_key")?;
    let link = rebuild_link(
        app_url,
        master,
        text("owner_wrapped_key"),
        text("owner_wrapped_token"),
        wrapped,
    )?;
    // The key part, read by core's own link parser (never split out by hand).
    let key_part = Zeroizing::new(beebeeb_core::share_link::parse(&link).ok()?.key);
    let opens = open_link_file_key(&key_part, wrapped).ok()?;
    // Every byte is compared, whatever the first difference.
    let differs = opens
        .as_bytes()
        .iter()
        .zip(file_key.as_bytes().iter())
        .fold(0u8, |differs, (a, b)| differs | (a ^ b));
    (differs == 0).then_some(link)
}

fn flow_refusal(error: ShareFlowError) -> ShareLinkError {
    match error {
        ShareFlowError::NotFound => ShareLinkError::NotFound,
        ShareFlowError::Unauthorized => ShareLinkError::SessionEnded,
        ShareFlowError::InvalidInput(_) => ShareLinkError::Invalid,
        _ => ShareLinkError::Refused,
    }
}
