//! A link to a folder, one link over several files (a bundle), and taking a folder's links off (DW-0502, DW-0506),
//! for a host without a UI toolkit (the native Share dialog and Shared): [`Engine::folder_link_for`],
//! [`Engine::bundle_link_for`], [`Engine::folder_links_revoke_for`]. The flows moved out of the old shell's
//! `share_v2/folder_flows.rs` (`folder_link`, `bundle_link`, `folder_link_revokes`), which is gone.
//! A link to ONE file stays [`Engine::share_link_for`] (`share_link_api.rs`), which refuses a folder.
//!
//! No crypto step is built here. A folder link is core's `FolderShare::link` (the folder's key from its custody
//! record, a wrap per descendant, the link's key after `#key=`); a bundle is core's `CreateLink::bundle` over the key
//! that opens each file here; a folder link the account already has is rebuilt and checked by core
//! (`rebuild_folder_link`).
//!
//! Rules held here, before or while core runs:
//! - One session only. `session` is [`Engine::session_turn`] as the host read it AT THE PRESS. The token and the vault
//!   key of that session are copied together while it is the session in memory, and the same question is asked again
//!   before EACH request and before the link is handed over: a press made under one session never sends under
//!   another, and its link is never handed to another. Each of those checks is `link_session.rs`'s, shared with the
//!   file requests.
//! - The folder comes from the account's local state: one of its own folders (never a received share, never the
//!   Backups tree). A file, or an id the local state does not hold as the account's, is refused with nothing sent.
//! - Every folder id core asks to remember (L7d) is written to the custody memory BEFORE the request that uses it; a
//!   write that fails stops the flow, that request unsent.
//! - Never two links behind a person's back. A plain Copy link on a folder (no choices) first reads the folder's links.
//!   A live one (named for this folder, not revoked, not past its expiry, its opens not used up) is rebuilt by core and
//!   handed out only when core could check it: no password on it, and its stored wrapped folder key present and
//!   opening under the link's key. A live link that cannot be checked ends the call with
//!   [`FolderLinkError::HasLink`] and nothing is made; a read that fails makes nothing either.
//! - A bundle is 1 to [`BUNDLE_MAX_ITEMS`] distinct files, each one the account's local state holds and opens here. A
//!   folder among them, the same file twice, or a file whose key is not on this computer: nothing is sent.
//! - A 403 is Beebeeb refusing (a frozen account, sharing paused), never a session that ended. Only a 401 is that.
//!
//! The link carries the key after `#`. It leaves this file only inside a [`MadeLink`], which prints nothing of it; no
//! error holds a byte of it, of a token or of anything the server sent.
//!
//! BLOCKS on the handle's runtime: call it from a worker thread, never from the UI thread.
//!
//! Must not: log an id, a name, a link or anything the server sends; build a key, a token or a link by hand. A
//! `ShareFlowError`, an `HttpRequest` or a `FolderSend` is never formatted: they can carry server text and key blobs.

use beebeeb_core::auth_flow::{HttpRequest, HttpResponse};
use beebeeb_core::kdf::{FileKey, MasterKey};
use beebeeb_core::share_flow::ShareFlowError;
use beebeeb_core::share_flow::bundle::{BUNDLE_MAX_ITEMS, BundleItem};
use beebeeb_core::share_flow::folder::{
    FolderLinkInput, FolderSend, FolderShare, FolderStep, list_folder_links_request, rebuild_folder_link,
    revoke_folder_link_request,
};
use beebeeb_core::share_flow::folder_keys::CustodyMemory;
use beebeeb_core::share_flow::link::{CreateLink, LinkOptions, LinkStep};
use serde_json::Value;
use zeroize::Zeroizing;

use super::link_session::state_refusal;
// The one `get` of the link code, for the callers outside `bindings` (the module is private to it).
pub(in crate::app) use super::link_session::get;
use crate::account::SessionGeneration;
use crate::app::share_invite_api::{canonical, own_item};
use crate::app::share_link_api::{LinkChoices, MadeLink, ShareLinkError, key_of, web_origin};
use crate::app::{CredentialStores, Engine};
use crate::state_db::StateDb;

#[cfg(test)]
#[path = "share_folder_link_tests.rs"]
mod tests;

/// Why no folder or bundle link was made, or a folder's links were not all taken off. Fixed words only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FolderLinkError {
    /// No session in memory, or not the session of the press any more. Nothing (more) was sent.
    NotSignedIn,
    Offline,
    /// Beebeeb no longer accepts this session (401).
    SessionEnded,
    /// Not an id, not an item of this account's own in its local state, or Beebeeb knows no such item.
    NotFound,
    /// A folder link was asked for a file. Nothing was sent.
    NotAFolder,
    /// A bundle holds a folder. Nothing was sent.
    NotAFile,
    /// The key that opens a file of the bundle is not on this computer. Nothing was sent.
    KeyUnavailable,
    /// The folder has a live link that cannot be rebuilt and checked here (a password on it, or a stored key that
    /// is absent or does not open). No second link was made.
    HasLink,
    /// A choice that is refused: a password of spaces only, an expiry that is too long, an empty bundle or the same
    /// file twice in it. Nothing was sent.
    Invalid,
    /// More than a link can carry: a bundle of more than [`BUNDLE_MAX_ITEMS`] files, or a folder with more
    /// descendants than a share takes.
    TooLarge,
    /// The folder was shared before shared folders and has no custody record yet: its keys need the upgrade first.
    NeedsUpgrade,
    /// The folder has people on it and no copy of its key opens with proof here: no new key is made for it.
    KeyUnverified,
    /// Beebeeb refused (a 403 included) or failed.
    Refused,
}

impl FolderLinkError {
    pub fn words(self) -> &'static str {
        match self {
            Self::NotSignedIn => "not signed in",
            Self::Offline => "offline",
            Self::SessionEnded => "the session ended",
            Self::NotFound => "not found",
            Self::NotAFolder => "not a folder",
            Self::NotAFile => "a folder is in it",
            Self::KeyUnavailable => "the key is not available",
            Self::HasLink => "it has a link that cannot be copied again yet",
            Self::Invalid => "a choice was refused",
            Self::TooLarge => "too many items",
            Self::NeedsUpgrade => "the folder needs an upgrade first",
            Self::KeyUnverified => "the folder's key cannot be checked",
            Self::Refused => "refused",
        }
    }
}

impl std::fmt::Display for FolderLinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.words())
    }
}

impl<C: CredentialStores> Engine<C> {
    /// WRITES on the server (at most one new link): the link to the account's folder `folder_id`, for the account of
    /// `session`. `choices`: `None`, or choices with nothing chosen, is the plain Copy link (the folder's live link if
    /// it has one that can be checked, else a new view link that never expires); an expiry or a password asks for a
    /// new link with them. See the module's doc for every rule.
    pub fn folder_link_for(
        &self,
        session: SessionGeneration,
        folder_id: &str,
        choices: Option<&LinkChoices>,
    ) -> Result<MadeLink, FolderLinkError> {
        let id = canonical(folder_id).ok_or(FolderLinkError::NotFound)?;
        let app_url = web_origin(&self.config.api_base).ok_or(FolderLinkError::Refused)?;
        if choices.is_some_and(blank_password) {
            return Err(FolderLinkError::Invalid);
        }
        let run = self.link_run(session)?;
        let state = self.local_state_for_changes().map_err(state_refusal)?;
        if state.session() != session {
            return Err(FolderLinkError::NotSignedIn);
        }
        let (revision, seen) = state.with(|db| own_folder(db, id)).map_err(state_refusal)??;
        let master = MasterKey::from_bytes(*run.key());

        // Nothing chosen is the plain request: the folder's own live link, never a second one behind it.
        let plain = choices.is_none_or(|c| c.expires_in_hours.is_none() && c.passphrase.is_none());
        if plain {
            let req = list_folder_links_request(id).map_err(|_| FolderLinkError::NotFound)?;
            let links = listed(self.link_send(&run, req)?)?;
            let live = live_folder_links(&links, id);
            if let Some(link) = live.iter().find_map(|row| rebuilt_folder_link(&app_url, &master, row)) {
                return self.hand_over(&run, link).map_err(FolderLinkError::from);
            }
            if !live.is_empty() {
                return Err(FolderLinkError::HasLink);
            }
        }

        let input = FolderLinkInput {
            folder_id: id,
            app_url: &app_url,
            access_level: "view",
            label: None,
            expires_in_hours: choices.and_then(|c| c.expires_in_hours),
            cap_bytes: None,
            max_opens: None,
            passphrase: choices.and_then(|c| c.passphrase.clone()),
            custody: CustodyMemory::new(seen, false),
        };
        let started = FolderShare::link(input, &master);
        drop(master);
        let (mut flow, first) = started.map_err(flow_refusal)?;
        let mut step = first;
        let link = loop {
            let FolderSend {
                request,
                mark_custody_seen,
            } = match step {
                FolderStep::Send(send) => send,
                FolderStep::LinkCreated(created) => break created.link,
                // An invite's outcome is never what a link makes.
                FolderStep::Done(_) => return Err(FolderLinkError::Refused),
            };
            // L7d: remembered before any key from the record is used. A write that fails: this request is not sent.
            if let Some(folder) = mark_custody_seen {
                state
                    .with(|db| remember_custody(db, &folder, revision))
                    .map_err(state_refusal)?
                    .map_err(|_| FolderLinkError::Refused)?;
            }
            let resp = self.link_send(&run, request)?;
            step = flow.on_response(resp).map_err(flow_refusal)?;
        };
        self.hand_over(&run, link).map_err(FolderLinkError::from)
    }

    /// WRITES on the server (one new link): one link over the account's files `file_ids`, in that order, for the
    /// account of `session`. A bundle is always a new link. See the module's doc for every rule.
    pub fn bundle_link_for(
        &self,
        session: SessionGeneration,
        file_ids: &[String],
        choices: Option<&LinkChoices>,
    ) -> Result<MadeLink, FolderLinkError> {
        if file_ids.is_empty() {
            return Err(FolderLinkError::Invalid);
        }
        if file_ids.len() > BUNDLE_MAX_ITEMS {
            return Err(FolderLinkError::TooLarge);
        }
        let ids = file_ids
            .iter()
            .map(|id| canonical(id).ok_or(FolderLinkError::NotFound))
            .collect::<Result<Vec<_>, _>>()?;
        if ids.iter().enumerate().any(|(n, id)| ids[..n].contains(id)) {
            return Err(FolderLinkError::Invalid);
        }
        let app_url = web_origin(&self.config.api_base).ok_or(FolderLinkError::Refused)?;
        if choices.is_some_and(blank_password) {
            return Err(FolderLinkError::Invalid);
        }
        let run = self.link_run(session)?;
        let master = MasterKey::from_bytes(*run.key());
        // From the signed-in account's local state: every file's key, or nothing is sent.
        let state = self.local_state().map_err(state_refusal)?;
        if state.session() != session {
            return Err(FolderLinkError::NotSignedIn);
        }
        let keys: Vec<FileKey> = state
            .with(|db| {
                ids.iter()
                    .map(|id| key_of(db, &master, id))
                    .collect::<Result<Vec<_>, _>>()
            })
            .map_err(state_refusal)?
            .map_err(from_file_refusal)?;
        let items: Vec<BundleItem<'_>> = ids
            .iter()
            .zip(&keys)
            .zip(0u32..)
            .map(|((id, key), position)| BundleItem {
                file_id: id,
                position,
                file_key: key,
            })
            .collect();
        let options = LinkOptions {
            expires_in_hours: choices.and_then(|c| c.expires_in_hours),
            max_opens: None,
            passphrase: choices.and_then(|c| c.passphrase.clone()),
        };
        let started = CreateLink::bundle(&app_url, &items, &master, &options);
        drop(items);
        drop(keys);
        drop(master);
        let (mut flow, mut req) = started.map_err(flow_refusal)?;
        let link = loop {
            match flow.on_response(self.link_send(&run, req)?).map_err(flow_refusal)? {
                LinkStep::Send(next) => req = next,
                LinkStep::Created(created) => break created.link,
            }
        };
        self.hand_over(&run, link).map_err(FolderLinkError::from)
    }

    /// WRITES on the server: "Stop sharing" a folder's links. Every link of the account's on the folder `folder_id`
    /// that is not revoked is revoked, one request each; a link already gone (404) counts as done. Refused before
    /// anything is sent when `session` is not the session in memory any more.
    pub fn folder_links_revoke_for(&self, session: SessionGeneration, folder_id: &str) -> Result<(), FolderLinkError> {
        let id = canonical(folder_id).ok_or(FolderLinkError::NotFound)?;
        let run = self.link_run(session)?;
        let req = list_folder_links_request(id).map_err(|_| FolderLinkError::NotFound)?;
        let links = listed(self.link_send(&run, req)?)?;
        let revokes = links
            .iter()
            .filter(|row| text(row, "folder_id") == Some(id) && !revoked(row))
            .map(|row| {
                let share = text(row, "id").and_then(canonical).ok_or(FolderLinkError::Refused)?;
                revoke_folder_link_request(share).map_err(|_| FolderLinkError::Refused)
            })
            .collect::<Result<Vec<HttpRequest>, _>>()?;
        for req in revokes {
            match self.link_send(&run, req)?.status {
                200..=299 | 404 => {}
                401 => return Err(FolderLinkError::SessionEnded),
                _ => return Err(FolderLinkError::Refused),
            }
        }
        Ok(())
    }
}

/// A password that was asked for and is only spaces: never a link that asks for none.
fn blank_password(choices: &LinkChoices) -> bool {
    choices.passphrase.as_ref().is_some_and(|typed| typed.trim().is_empty())
}

/// One of the account's own folders (the invite's rule, `own_item`): the account revision the custody writes go
/// under, and the folders this computer has seen custody for.
fn own_folder(db: &StateDb, id: &str) -> Result<(u64, Vec<String>), FolderLinkError> {
    match own_item(db, id).map_err(|_| FolderLinkError::Refused)? {
        Some(true) => {}
        Some(false) => return Err(FolderLinkError::NotAFolder),
        None => return Err(FolderLinkError::NotFound),
    }
    let seen = db.custody_seen_ids().map_err(|_| FolderLinkError::Refused)?;
    Ok((db.account_revision(), seen))
}

/// The custody write a folder step asks for, under the revision the press started on (the account-bound handle).
fn remember_custody(db: &StateDb, folder: &str, revision: u64) -> rusqlite::Result<()> {
    #[cfg(test)]
    if tests::CUSTODY_WRITE_FAILS.with(std::cell::Cell::get) {
        return Err(rusqlite::Error::InvalidQuery);
    }
    db.remember_folder_custody_at_revision(folder, revision)
}

fn text<'a>(row: &'a Value, field: &str) -> Option<&'a str> {
    row.get(field).and_then(Value::as_str)
}

/// A folder link row is revoked when it says so (`revoked_at`, or a `revoked` flag).
fn revoked(row: &Value) -> bool {
    text(row, "revoked_at").is_some() || row.get("revoked").and_then(Value::as_bool) == Some(true)
}

/// The folder's links as Beebeeb listed them. An answer that is not a list of links is a read that failed: it is
/// never taken for "no links" (a link would be made on top of ones that were not seen).
fn listed(resp: HttpResponse) -> Result<Vec<Value>, FolderLinkError> {
    match resp.status {
        200..=299 => match resp.body.get("links").and_then(Value::as_array) {
            Some(links) => Ok(links.clone()),
            None => Err(FolderLinkError::Refused),
        },
        401 => Err(FolderLinkError::SessionEnded),
        _ => Err(FolderLinkError::Refused),
    }
}

/// The live links of the folder `id`: named for this folder (asked of each row here, not left to the server's
/// filter), not revoked, not past its expiry, its opens not used up.
fn live_folder_links<'a>(links: &'a [Value], id: &str) -> Vec<&'a Value> {
    let now = chrono::Utc::now();
    let count = |row: &Value, field: &str| row.get(field).and_then(Value::as_i64);
    links
        .iter()
        .filter(|row| {
            let expired = text(row, "expires_at")
                .and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok())
                .is_some_and(|at| at <= now);
            let used_up = count(row, "max_opens").is_some_and(|most| count(row, "open_count").unwrap_or(0) >= most);
            text(row, "folder_id") == Some(id) && !revoked(row) && !expired && !used_up
        })
        .collect()
}

/// A live folder link rebuilt from its owner copies by core, and handed out only when core checked it: no password
/// on it (core can check a password link only with the password) and its stored wrapped folder key present, so core
/// opens it under the link's key. `None` otherwise.
fn rebuilt_folder_link(app_url: &str, master: &MasterKey, row: &Value) -> Option<Zeroizing<String>> {
    if row.get("passphrase_protected").and_then(Value::as_bool) != Some(false) {
        return None;
    }
    let wrapped = text(row, "wrapped_folder_key").filter(|w| !w.is_empty())?;
    rebuild_folder_link(
        app_url,
        master,
        text(row, "id")?,
        text(row, "owner_wrapped_key"),
        text(row, "owner_wrapped_token"),
        Some(wrapped),
        false,
    )
}

/// A file of the bundle refused by the file link's key rule (`key_of`).
fn from_file_refusal(refused: ShareLinkError) -> FolderLinkError {
    match refused {
        ShareLinkError::NotFound => FolderLinkError::NotFound,
        ShareLinkError::NotAFile => FolderLinkError::NotAFile,
        ShareLinkError::KeyUnavailable => FolderLinkError::KeyUnavailable,
        _ => FolderLinkError::Refused,
    }
}

/// A core refusal as fixed words. Never formats the error. A 403 is core's `Forbidden`: Beebeeb refusing.
fn flow_refusal(error: ShareFlowError) -> FolderLinkError {
    match error {
        ShareFlowError::NotFound => FolderLinkError::NotFound,
        ShareFlowError::Unauthorized => FolderLinkError::SessionEnded,
        ShareFlowError::InvalidInput("no items") => FolderLinkError::Invalid,
        ShareFlowError::InvalidInput("too many items") | ShareFlowError::FolderTooLarge => FolderLinkError::TooLarge,
        ShareFlowError::InvalidInput(_) => FolderLinkError::Invalid,
        ShareFlowError::FolderNeedsUpgrade => FolderLinkError::NeedsUpgrade,
        ShareFlowError::FolderKeyUnverified => FolderLinkError::KeyUnverified,
        _ => FolderLinkError::Refused,
    }
}
