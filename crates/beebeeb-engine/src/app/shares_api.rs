//! What the account shares and what is shared with it, for a host without a UI toolkit (the native Shared window):
//! the invitations waiting for an answer, the items shared with the account, the items it shares with people and
//! who they are, its links and its file requests; and three changes (accept an invitation, change a person's access,
//! take a person off a share). The view-model moved from the Tauri shell's `share_v2/reads.rs` and `flows.rs`
//! (`share_list`, `share_people`, `share_accept`, `share_set_access`, `share_revoke` with a member); those commands
//! are left as they were.
//!
//! From the SERVER only, for the session in memory: nothing is read from this computer's local state. A name is
//! decrypted here, with the session's vault key: an incoming item's through core's member key flow
//! (`share_flow::open_member_file_key`), one of the account's own through `owner_keys::display_name`. No crypto step
//! is built here. A name that cannot be opened is fixed words, never a field the server sent.
//!
//! What is handed out ([`SharesOverview`]) carries ids, names, emails, choices and times only. The server's rows also
//! hold link tokens, wrapped keys and sealed keys: none of them leaves this file, in an answer or in an error.
//!
//! Every method BLOCKS on the handle's runtime: call it from a worker thread, never from the UI thread. Every answer
//! is bound to the session that asked ([`SharesOverview::session`]); a change names the session it was decided under
//! and is refused, before anything is sent, when that is no longer the session.
//!
//! Must not: log a name, an email, an id or anything the server sends.

use base64::Engine as _;
use beebeeb_core::auth_flow::{HttpRequest, Method};
use beebeeb_core::kdf::MasterKey;
use beebeeb_core::share_flow::{self, access, request::FILE_REQUESTS_PATH};
use serde_json::{Value, json};
use zeroize::Zeroizing;

use super::settings_api::AccountCallError;
use super::{CredentialStores, Engine};
use crate::account::SessionGeneration;
use crate::{auth_transport, owner_keys};

const INCOMING: &str = "/api/v1/shares/invites/incoming";
const SENT: &str = "/api/v1/shares/invites/sent";
const MINE: &str = "/api/v1/shares/mine";

/// The name of an incoming item whose name cannot be opened (not sealed for this account yet, or a folder's).
pub const ENCRYPTED_ITEM: &str = "Encrypted item";
/// The name of one of the account's own shared items whose name cannot be opened.
pub const SHARED_ITEM: &str = "Shared item";

/// Everything Shared shows, and the session it was read for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharesOverview {
    /// A host compares it with [`Engine::session_turn`] and drops an overview whose session is gone.
    pub session: SessionGeneration,
    /// The session's own email (the owner of everything in `by_me`).
    pub owner_email: Option<String>,
    /// Invitations waiting for an answer, and the items shared with the account (accepted and approved).
    pub incoming: Vec<IncomingShare>,
    /// The items the account shares with people, in the server's order, each with its people.
    pub by_me: Vec<SentShare>,
    pub links: Vec<OwnedLink>,
    pub requests: Vec<FileRequestRow>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncomingShare {
    pub invite_id: String,
    pub item_id: String,
    pub name: String,
    pub is_folder: bool,
    pub from_email: String,
    pub size_bytes: i64,
    /// `true`: an invitation waiting for this account's answer. `false`: shared with the account.
    pub waiting: bool,
    /// When it was approved (an invitation: when it was sent), as the server wrote it.
    pub shared_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SentShare {
    pub item_id: String,
    pub name: String,
    pub is_folder: bool,
    pub members: Vec<ShareMember>,
}

/// One person on a share of the account's. `invite_id` is what a change names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShareMember {
    pub invite_id: String,
    pub email: String,
    pub can_edit: bool,
    /// Approved: the person has the item. Otherwise invited, waiting.
    pub accepted: bool,
}

/// A live (not revoked) link of the account's. Its token and keys stay in the engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedLink {
    pub item_id: String,
    pub name: String,
    pub expires_at: Option<String>,
    pub has_password: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRequestRow {
    pub id: String,
    pub title: String,
    pub files_received: u64,
    pub max_files: Option<u64>,
    pub expires_at: Option<String>,
    pub closed: bool,
}

/// A change to a share, on the server, for the session's account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShareChange {
    /// Accept an invitation (claim it; the owner approves and seals the key).
    Accept { invite_id: String },
    /// What a person may do with a share of the account's.
    SetAccess { invite_id: String, can_edit: bool },
    /// Take one person off a share of the account's.
    RemoveMember { invite_id: String },
}

/// Why a change was not made. Fixed words only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShareChangeError {
    /// No session in memory, or not the session the change was decided under. Nothing was sent.
    NotSignedIn,
    Offline,
    /// Beebeeb no longer accepts this session (401).
    SessionEnded,
    /// Not an id, or Beebeeb knows no such invitation or person.
    NotFound,
    /// Beebeeb refused or failed.
    Refused,
}

impl ShareChangeError {
    pub fn words(self) -> &'static str {
        match self {
            Self::NotSignedIn => "not signed in",
            Self::Offline => "offline",
            Self::SessionEnded => "the session ended",
            Self::NotFound => "not found",
            Self::Refused => "refused",
        }
    }
}

impl std::fmt::Display for ShareChangeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.words())
    }
}

/// The token, the vault key, the email and the generation of the session in memory, copied out under one lock.
type SessionParts = (
    Zeroizing<String>,
    Zeroizing<[u8; 32]>,
    Option<String>,
    SessionGeneration,
);

impl<C: CredentialStores> Engine<C> {
    fn share_session(&self) -> Option<SessionParts> {
        let guard = self.acct.session.lock().ok()?;
        let current = guard.as_ref()?;
        Some((
            Zeroizing::new(current.token.clone()),
            Zeroizing::new(current.master_key),
            current.email.clone(),
            self.acct.session_generation(),
        ))
    }

    /// Everything Shared shows (see [`SharesOverview`]): four reads, every name decrypted here. No session in memory,
    /// or a session that changed while the call ran: [`AccountCallError::NotSignedIn`], and the answer is dropped.
    pub fn shares_overview(&self) -> Result<SharesOverview, AccountCallError> {
        let (token, key, owner_email, session) = self.share_session().ok_or(AccountCallError::NotSignedIn)?;
        let client = auth_transport::client_for(&self.config.api_base).map_err(|_| AccountCallError::Offline)?;
        let base = self.config.api_base.trim_end_matches('/');
        let read = self.runtime.block_on(async {
            let mut bodies = Vec::with_capacity(4);
            for path in [INCOMING, SENT, MINE, FILE_REQUESTS_PATH] {
                bodies.push(fetch(base, &client, &token, path).await?);
            }
            Ok::<_, AccountCallError>(bodies)
        });
        drop(token);
        let bodies = read?;
        if !self.acct.session_unchanged_since(session) {
            return Err(AccountCallError::NotSignedIn);
        }
        let master = MasterKey::from_bytes(*key);
        let by_me = sent_shares(&bodies[1], &key);
        let overview = SharesOverview {
            session,
            owner_email,
            incoming: list(&bodies[0], "invites")
                .filter_map(|i| incoming_share(i, &master))
                .collect(),
            links: owned_links(&bodies[2], &by_me),
            by_me,
            requests: list(&bodies[3], "file_requests").filter_map(request_row).collect(),
        };
        if !self.acct.session_unchanged_since(session) {
            return Err(AccountCallError::NotSignedIn);
        }
        Ok(overview)
    }

    /// WRITES on the server: one change, for the account of `session`. Refused before anything is sent when
    /// `session` is not the session in memory any more, or an id is not a canonical id.
    pub fn share_change(&self, session: SessionGeneration, change: &ShareChange) -> Result<(), ShareChangeError> {
        let (token, _key, _, now) = self.share_session().ok_or(ShareChangeError::NotSignedIn)?;
        if now != session {
            return Err(ShareChangeError::NotSignedIn);
        }
        let (req, gone_is_done) = change_request(change)?;
        let client = auth_transport::client_for(&self.config.api_base).map_err(|_| ShareChangeError::Offline)?;
        let base = self.config.api_base.trim_end_matches('/');
        let resp = self
            .runtime
            .block_on(auth_transport::send_to(base, &client, Some(&token), req))
            .map_err(|_| ShareChangeError::Offline)?;
        match resp.status {
            200..=299 => Ok(()),
            // Taking off a person who is already off: done.
            404 if gone_is_done => Ok(()),
            404 => Err(ShareChangeError::NotFound),
            401 => Err(ShareChangeError::SessionEnded),
            _ => Err(ShareChangeError::Refused),
        }
    }
}

/// The request of a change, through core where core builds it, and whether a 404 means it is already so. An id that
/// is not a canonical lower-case UUID is refused here (never normalized: core v0.2.8 items 10 and 13).
fn change_request(change: &ShareChange) -> Result<(HttpRequest, bool), ShareChangeError> {
    let not_found = |_| ShareChangeError::NotFound;
    match change {
        ShareChange::Accept { invite_id } => {
            let path = format!("/api/v1/shares/invites/{}/claim", canonical(invite_id)?);
            Ok((request(Method::Post, path), false))
        }
        ShareChange::SetAccess { invite_id, can_edit } => {
            let permission = if *can_edit {
                access::MemberPermission::parse("write")
            } else {
                access::MemberPermission::parse("read")
            }
            .map_err(|_| ShareChangeError::Refused)?;
            let req = access::set_member_access_request(canonical(invite_id)?, permission).map_err(not_found)?;
            Ok((req, false))
        }
        ShareChange::RemoveMember { invite_id } => {
            let req = share_flow::cancel_invite_request(canonical(invite_id)?).map_err(not_found)?;
            Ok((req, true))
        }
    }
}

fn canonical(id: &str) -> Result<&str, ShareChangeError> {
    match uuid::Uuid::parse_str(id) {
        Ok(u) if u.hyphenated().to_string() == id => Ok(id),
        _ => Err(ShareChangeError::NotFound),
    }
}

/// A request on the session with no confirm token and no body.
fn request(method: Method, path: String) -> HttpRequest {
    let mut req = HttpRequest::session_post(path, Value::Null);
    req.method = method;
    req.body = None;
    req
}

async fn fetch(base: &str, client: &reqwest::Client, token: &str, path: &str) -> Result<Value, AccountCallError> {
    let resp = auth_transport::send_to(base, client, Some(token), request(Method::Get, path.to_string()))
        .await
        .map_err(|_| AccountCallError::Offline)?;
    match resp.status {
        200..=299 if resp.body.is_object() => Ok(resp.body),
        200..=299 => Err(AccountCallError::Unreadable),
        401 => Err(AccountCallError::SessionEnded),
        _ => Err(AccountCallError::ServerError),
    }
}

fn text<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    v.get(k).and_then(Value::as_str)
}

fn flag(v: &Value, k: &str) -> bool {
    v.get(k).and_then(Value::as_bool).unwrap_or(false)
}

fn list<'a>(body: &'a Value, field: &str) -> impl Iterator<Item = &'a Value> {
    body.get(field).and_then(Value::as_array).into_iter().flatten()
}

/// An incoming invite as a row: `invited` waits for an answer, `approved` is shared with the account. A `claimed`
/// one (accepted, the owner has not approved yet) is neither: there is nothing to answer and nothing to open.
fn incoming_share(invite: &Value, master: &MasterKey) -> Option<IncomingShare> {
    let waiting = match text(invite, "status")? {
        "invited" => true,
        "approved" => false,
        _ => return None,
    };
    Some(IncomingShare {
        invite_id: text(invite, "id")?.to_string(),
        item_id: text(invite, "file_id")?.to_string(),
        name: incoming_name(invite, master).unwrap_or_else(|| ENCRYPTED_ITEM.to_string()),
        is_folder: flag(invite, "is_folder") || flag(invite, "is_folder_share"),
        from_email: text(invite, "sender_email").unwrap_or_default().to_string(),
        size_bytes: invite.get("size_bytes").and_then(Value::as_i64).unwrap_or(0),
        waiting,
        shared_at: text(invite, "approved_at")
            .or_else(|| text(invite, "created_at"))
            .unwrap_or_default()
            .to_string(),
    })
}

/// An incoming item's name, opened with core's member key flow. Only an encrypted envelope is opened: a name field
/// that is not one is never shown as the name.
fn incoming_name(invite: &Value, master: &MasterKey) -> Option<String> {
    let b64 = base64::engine::general_purpose::STANDARD;
    let sender: [u8; 32] = b64.decode(text(invite, "sender_public_key")?).ok()?.try_into().ok()?;
    let sealed = b64.decode(text(invite, "encrypted_file_key")?).ok()?;
    let name = text(invite, "file_name_encrypted").filter(|name| name.starts_with('{'))?;
    let key = share_flow::open_member_file_key(master, &sender, text(invite, "file_id")?, &sealed).ok()?;
    beebeeb_core::encrypt::decrypt_name_with_key(&key, name).filter(|name| owner_keys::safe_display_name(name))
}

/// The invites the account sent, grouped by item in the server's order; an invite that is over (revoked, declined,
/// expired, denied) is no person of the share.
fn sent_shares(sent: &Value, master_key: &[u8; 32]) -> Vec<SentShare> {
    let mut out: Vec<SentShare> = Vec::new();
    for invite in list(sent, "invites") {
        let (Some(id), Some(item), Some(email), Some(status)) = (
            text(invite, "id"),
            text(invite, "file_id"),
            text(invite, "recipient_email"),
            text(invite, "status"),
        ) else {
            continue;
        };
        if !matches!(status, "invited" | "claimed" | "approved") {
            continue;
        }
        let member = ShareMember {
            invite_id: id.to_string(),
            email: email.to_string(),
            can_edit: text(invite, "permission").unwrap_or("read") != "read",
            accepted: status == "approved",
        };
        match out.iter_mut().find(|share| share.item_id == item) {
            Some(share) => share.members.push(member),
            None => out.push(SentShare {
                item_id: item.to_string(),
                name: own_name(invite, "file_name_encrypted", item, master_key),
                is_folder: flag(invite, "is_folder_share"),
                members: vec![member],
            }),
        }
    }
    out
}

/// The name of one of the account's own items, from the row's encrypted name (`owner_keys::display_name`).
fn own_name(row: &Value, field: &str, item: &str, master_key: &[u8; 32]) -> String {
    text(row, field)
        .and_then(|name| owner_keys::display_name(&json!({ "name_encrypted": name }), item, master_key))
        .unwrap_or_else(|| SHARED_ITEM.to_string())
}

/// The account's live links, one per item. The listing carries no name: an item also shared with people has the
/// name read there, any other reads [`SHARED_ITEM`].
fn owned_links(mine: &Value, by_me: &[SentShare]) -> Vec<OwnedLink> {
    let mut out: Vec<OwnedLink> = Vec::new();
    for share in list(mine, "shares") {
        let Some(item) = text(share, "file_id") else { continue };
        if flag(share, "revoked") || out.iter().any(|link| link.item_id == item) {
            continue;
        }
        out.push(OwnedLink {
            item_id: item.to_string(),
            name: by_me
                .iter()
                .find(|s| s.item_id == item)
                .map_or_else(|| SHARED_ITEM.to_string(), |s| s.name.clone()),
            expires_at: text(share, "expires_at").map(str::to_string),
            has_password: flag(share, "has_passphrase"),
        });
    }
    out
}

fn request_row(r: &Value) -> Option<FileRequestRow> {
    Some(FileRequestRow {
        id: text(r, "id")?.to_string(),
        title: text(r, "title")?.to_string(),
        files_received: r.get("files_received").and_then(Value::as_u64).unwrap_or(0),
        max_files: r.get("max_files").and_then(Value::as_u64).filter(|max| *max > 0),
        expires_at: text(r, "expires_at").map(str::to_string),
        closed: flag(r, "closed"),
    })
}

#[cfg(test)]
#[path = "shares_api_tests.rs"]
mod tests;
