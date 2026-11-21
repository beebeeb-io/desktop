//! "Invite someone" (DW-0514) for a host without a UI toolkit: share one of the account's own items with a person by
//! their email address. Ported from the Tauri shell's `share_v2/flows.rs::share_invite`,
//! `share_v2/folder_flows.rs::folder_invite` and `share_v2/safety.rs` (`expectation_of`, `record_outcome_in`,
//! `finish_invite`); those are left as they were.
//!
//! ```mermaid
//! flowchart TD
//!   P[invite_for: session, item, email] --> R[the item from the account's local state: folder or file, its own]
//!   R --> S[GET the sent invites: already on it? the person's account ids]
//!   S --> M[the person's compared marks: Compared or NotCompared, an unreadable mark stops]
//!   M -->|file| F[core MemberInvite]
//!   M -->|folder| G[flag read now, custody memory, core FolderShare]
//!   F --> D[core's steps over HTTP, session checked before each send]
//!   G --> D
//!   D --> O[outcome: record the key bytes it carried, or withdraw an invite that holds no key]
//! ```
//!
//! The keys are core's: a file goes through `MemberInvite`, a folder through `FolderShare::invite` (task 2133: one key
//! per folder, never a mint while an active invite's owner copy does not open). Nothing here builds a seal, a wrap or
//! a key. The key sealed to is the one the server answered in this flow, checked by core against the person's compared
//! mark before any seal byte exists.
//!
//! Rules held here, before or while core runs:
//! - Folder or file comes from the account's local state, never from the window (core: a folder sent through the file
//!   path gives a share that never opens). An item that is not the account's own (a received share, anything in the
//!   Backups tree, an id this computer has no key for) is refused before anything is sent.
//! - The `shared_folders_v1` flag is read at send time, for a folder only; a flag that cannot be read stops the
//!   invite (web `FeatureFlagsUnavailableError`), never "off".
//! - Every folder id core asks to remember (L7d) is written to the custody memory BEFORE the request that uses it;
//!   a write that fails stops the flow, that request unsent.
//! - The person's marks (by address and every account id the sent list names for it) decide the expectation; a mark
//!   that cannot be read stops the invite, never `NotCompared`.
//! - A request is sent only while the session it was asked under is still the session in memory. One exception, the
//!   withdrawal of an invite this run claimed: after a turn that moved for the SAME account (a screen lock, a session
//!   write; the session in memory still holds the run's vault key) it goes out on the run's token, so a session
//!   change mid-invite never leaves a claimed invite behind (review of desktop#424). Another account, a sign-out or a
//!   closed vault: nothing is sent.
//! - The outcome's key bytes are recorded as the share's sealed key (or a changed key's offered key), so a safety
//!   number is later computed from exactly those bytes. A claimed invite this engine could not finish holds no key
//!   the other side can open: it is withdrawn and the answer says nothing was shared (Tauri `record_outcome_in`).
//!   When the withdrawal cannot be made, the answer is [`InviteAnswer::Unfinished`], never "nothing was shared"; it
//!   says whether the approval left without an answer (then nobody can say whether they can open the item).
//!
//! Every method BLOCKS on the handle's runtime: call it from a worker thread, never from the UI thread, except
//! [`Engine::share_target_of`], which reads the local state only.
//!
//! Must not: log or return a key, a sealed or wrapped key, an email, a name, an id or a server's words. A
//! `ShareFlowError`, an `HttpRequest` or a `FolderSend` is never formatted: they can carry server text and key blobs.

use beebeeb_core::auth_flow::{HttpRequest, HttpResponse};
use beebeeb_core::item_id::{ItemClass, item_class};
use beebeeb_core::kdf::{FileKey, MasterKey};
use beebeeb_core::share_flow::folder::{FolderInviteInput, FolderSend, FolderShare, FolderStep};
use beebeeb_core::share_flow::folder_keys::CustodyMemory;
use beebeeb_core::share_flow::member::{InviteOutcome, InviteStep, MemberInvite, PeerKeyExpectation, ShareTarget};
use beebeeb_core::share_flow::{self, ShareFlowError};
use serde_json::Value;
use zeroize::Zeroizing;

use super::bindings::share_folder_link::get;
use super::owned_state::{LocalStateRefused, OwnedState};
use super::{CredentialStores, Engine};
use crate::account::SessionGeneration;
use crate::state_db::{FileStatus, MarkKind, Namespace, StateDb, normalized_email};
use crate::{auth_transport, hexfmt, owner_keys};

const SENT: &str = "/api/v1/shares/invites/sent";
const ME: &str = "/api/v1/auth/me";
const FLAGS: &str = "/api/v1/feature-flags/evaluate";
const SHARED_FOLDERS_FLAG: &str = "shared_folders_v1";
/// An invite that still names a person of the share (core's `ACTIVE_STATUSES`).
const ACTIVE: [&str; 3] = ["invited", "claimed", "approved"];
/// The expectation for a person whose marks say "They don't match", or pin different keys: no key is this
/// fingerprint, so core refuses every key and nothing is sealed until the person chooses (Tauri `NO_KEY_MATCHES`).
const NO_KEY_MATCHES: [u8; 32] = [0; 32];

/// How an invite ended. Ids only: no key, address or name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InviteAnswer {
    /// The person had a key: the item's key was sealed to it and they have access now.
    Shared { invite_id: String },
    /// The invitation went out; the key goes over once they accept (they have no account or key yet).
    Invited { invite_id: String },
    /// They accepted, and the approval is still to come. Not answered by this engine yet: a claimed invite it could
    /// not finish is withdrawn instead (the module's doc), and approving one later is not in this version.
    AwaitingApproval { invite_id: String },
    /// Their key is not the one the person compared safety numbers against: nothing was sealed. The invite exists and
    /// stays claimed; the key it carried is kept as this invite's offered key.
    PeerKeyChanged { invite_id: String },
    /// They accepted, but the invite could not be finished (its key never went over) and could not be withdrawn
    /// either: it stays on Beebeeb, claimed, until it is cancelled in Shared.
    /// `approval_sent`: the approval request LEFT and its answer was lost, so Beebeeb may have applied it and they
    /// may already be able to open the item; `false`: it was refused, never left, or there was no approval (a
    /// changed key), so they hold no key they can open.
    Unfinished { invite_id: String, approval_sent: bool },
    /// Nothing was shared.
    Refused(InviteRefused),
}

/// Why nothing was shared. Fixed words: the host has its own copy for each.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InviteRefused {
    /// The person is on this share already (an invite that is not over).
    AlreadyOnIt,
    /// Not one of the account's own items here (a received share, Backups, gone, or no key on this computer).
    NotYours,
    /// The folder has invites from before shared folders and no custody record: its keys need the upgrade first.
    FolderNeedsUpgrade,
    /// The folder has active invites and no owner copy that opens with proof: no new key is made.
    FolderKeyUnverified,
    InvalidEmail,
    /// Whether shared folders are on could not be read: a folder invite is not started.
    SharedFoldersFlagUnknown,
    Offline,
    /// No session in memory, not the session it was asked under, or Beebeeb no longer accepts it.
    SessionEnded,
    Failed,
}

impl InviteRefused {
    pub fn kind(self) -> &'static str {
        match self {
            Self::AlreadyOnIt => "already_on_it",
            Self::NotYours => "not_yours",
            Self::FolderNeedsUpgrade => "folder_needs_upgrade",
            Self::FolderKeyUnverified => "folder_key_unverified",
            Self::InvalidEmail => "invalid_email",
            Self::SharedFoldersFlagUnknown => "shared_folders_flag_unknown",
            Self::Offline => "offline",
            Self::SessionEnded => "session_ended",
            Self::Failed => "failed",
        }
    }
}

impl std::fmt::Display for InviteRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.kind())
    }
}

/// What every send of one invite needs: the session it was asked under and that session's token and vault key
/// (copied once, wiped on drop), and where to send.
struct Run<'a> {
    session: SessionGeneration,
    token: Zeroizing<String>,
    key: Zeroizing<[u8; 32]>,
    base: &'a str,
    client: reqwest::Client,
}

/// Whether an approval that did not complete may have been applied: it left and the answer was a server error
/// (5xx), not an answer that proves a refusal. Only a 4xx proves the server did not take it.
fn approval_may_have_landed(outcome: &InviteOutcome, status: u16) -> bool {
    // `Some(key)`: the key was sealed and only the approval failed. `None`: sealing failed, so no approval was ever
    // sent and nothing can have landed.
    matches!(
        outcome,
        InviteOutcome::AwaitingApproval {
            recipient_public_key: Some(_),
            ..
        }
    ) && !(400..500).contains(&status)
}

/// A request that got no answer (the network dropped, or it never left).
struct Unanswered;

impl<C: CredentialStores> Engine<C> {
    /// WRITES on the server: invite `email` to the account's item `item_id`, for the account of `session`. Blocks.
    /// See the module's doc for every rule; refused before anything is sent when `session` is not the session in
    /// memory any more.
    pub fn invite_for(&self, session: SessionGeneration, item_id: &str, email: &str) -> InviteAnswer {
        self.invite_run(session, item_id, email)
            .unwrap_or_else(InviteAnswer::Refused)
    }

    /// The id the Shared section names this item's share with people by: the item's own id, while it is one of the
    /// account's own items (`None` for a received share, anything in the Backups tree, or an item this computer
    /// does not know). Reads the account-bound local state only: never the network, so a window may ask on a press.
    pub fn share_target_of(&self, session: SessionGeneration, item_id: &str) -> Option<String> {
        let item = canonical(item_id)?;
        let state = self.local_state().ok()?;
        if state.session() != session {
            return None;
        }
        state
            .with(|db| own_item(db, item))
            .ok()?
            .ok()?
            .map(|_| item.to_string())
    }

    fn invite_run(
        &self,
        session: SessionGeneration,
        item_id: &str,
        email: &str,
    ) -> Result<InviteAnswer, InviteRefused> {
        let (token, key) = self.invite_session(session).ok_or(InviteRefused::SessionEnded)?;
        let item = canonical(item_id).ok_or(InviteRefused::NotYours)?;
        let state = self.local_state_for_changes().map_err(state_refusal)?;
        if state.session() != session {
            return Err(InviteRefused::SessionEnded);
        }
        let (is_folder, revision) = state
            .with(|db| own_item(db, item).map(|found| found.map(|folder| (folder, db.account_revision()))))
            .map_err(state_refusal)?
            .map_err(|_| InviteRefused::Failed)?
            .ok_or(InviteRefused::NotYours)?;
        let client = auth_transport::client_for(&self.config.api_base).map_err(|_| InviteRefused::Offline)?;
        let run = Run {
            session,
            token,
            key,
            base: self.config.api_base.trim_end_matches('/'),
            client,
        };
        // Tauri `own_id`: the id the session named, else Beebeeb's answer for this session.
        let own = match self.own_account_id() {
            Some(own) => own,
            None => self.invite_me(&run)?,
        };

        let sent = self.invite_read(&run, SENT)?;
        let person = normalized_email(email);
        if already_on_it(&sent, item, &person) {
            return Err(InviteRefused::AlreadyOnIt);
        }
        let peers = peers_for_email(&sent, &person);
        let marks = state
            .with(|db| db.person_marks(&own, Some(&person), &peers))
            .map_err(state_refusal)?
            .map_err(|_| InviteRefused::Failed)?;
        let peer_key = expectation_of(&marks).ok_or(InviteRefused::Failed)?;

        let (outcome, approval_sent) = if is_folder {
            let shared_folders_write = self.shared_folders_write(&run)?;
            let seen = state
                .with(|db| db.custody_seen_ids())
                .map_err(state_refusal)?
                .map_err(|_| InviteRefused::Failed)?;
            let input = FolderInviteInput {
                folder_id: item,
                email,
                shared_folders_write,
                custody: CustodyMemory::new(seen, false),
                peer_key,
            };
            let master = MasterKey::from_bytes(*run.key);
            let started = FolderShare::invite(input, &master);
            drop(master);
            let (flow, first) = started.map_err(flow_refusal)?;
            self.drive_folder(&run, &state, revision, flow, first)?
        } else {
            let master = MasterKey::from_bytes(*run.key);
            let file_key = state
                .with(|db| file_key(db, &master, item))
                .map_err(state_refusal)?
                .ok_or(InviteRefused::NotYours)?;
            let target = ShareTarget {
                item_id: item.to_string(),
                is_folder: false,
            };
            let started = MemberInvite::start(&target, email, &master, &file_key, peer_key);
            drop(file_key);
            drop(master);
            let (flow, first) = started.map_err(flow_refusal)?;
            self.drive_file(&run, flow, first)?
        };
        Ok(self.finish(&run, &state, &own, outcome, approval_sent))
    }

    /// The token and the vault key of `session`, while it is the session in memory.
    fn invite_session(&self, session: SessionGeneration) -> Option<(Zeroizing<String>, Zeroizing<[u8; 32]>)> {
        let (token, key) = self
            .person_session(|current, _| {
                (
                    Zeroizing::new(current.token.clone()),
                    Zeroizing::new(current.master_key),
                )
            })
            .ok()?;
        self.session_is_still(session, &key).then_some((token, key))
    }

    /// The account's id, as a canonical UUID: the marks are kept under it.
    pub(super) fn own_account_id(&self) -> Option<String> {
        let recorded = self
            .session_user_id
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        recorded.or_else(|| self.user_id()).filter(|id| canonical(id).is_some())
    }

    /// Beebeeb's id for the account of this session (`/auth/me`), as a canonical UUID.
    fn invite_me(&self, run: &Run<'_>) -> Result<String, InviteRefused> {
        let me = self.invite_read(run, ME)?;
        text(&me, "user_id")
            .and_then(canonical)
            .map(str::to_string)
            .ok_or(InviteRefused::Failed)
    }

    /// Send one request of the invite, only while its session is still the session in memory.
    fn invite_send(&self, run: &Run<'_>, req: HttpRequest) -> Result<Result<HttpResponse, Unanswered>, InviteRefused> {
        if !self.session_is_still(run.session, &run.key) {
            return Err(InviteRefused::SessionEnded);
        }
        Ok(self
            .runtime
            .block_on(auth_transport::send_to(run.base, &run.client, Some(&run.token), req))
            .map_err(|_| Unanswered))
    }

    fn invite_read(&self, run: &Run<'_>, path: &str) -> Result<Value, InviteRefused> {
        let resp = self
            .invite_send(run, get(path))?
            .map_err(|Unanswered| InviteRefused::Offline)?;
        match resp.status {
            200..=299 if resp.body.is_object() => Ok(resp.body),
            401 => Err(InviteRefused::SessionEnded),
            _ => Err(InviteRefused::Failed),
        }
    }

    /// Web `sharedFoldersEnabledForWrite`: the flag now, or a refusal. Never "off" for a read that failed.
    fn shared_folders_write(&self, run: &Run<'_>) -> Result<bool, InviteRefused> {
        let resp = self
            .invite_send(run, get(FLAGS))?
            .map_err(|Unanswered| InviteRefused::Offline)?;
        if resp.status == 401 {
            return Err(InviteRefused::SessionEnded);
        }
        if !resp.ok() {
            return Err(InviteRefused::SharedFoldersFlagUnknown);
        }
        let flags = resp
            .body
            .get("flags")
            .and_then(Value::as_object)
            .ok_or(InviteRefused::SharedFoldersFlagUnknown)?;
        Ok(flags.get(SHARED_FOLDERS_FLAG).and_then(Value::as_bool) == Some(true))
    }

    fn drive_file(
        &self,
        run: &Run<'_>,
        mut flow: MemberInvite,
        first: HttpRequest,
    ) -> Result<(InviteOutcome, bool), InviteRefused> {
        let mut req = first;
        loop {
            let resp = match self.invite_send(run, req) {
                Ok(Ok(resp)) => resp,
                // After the approval was sent this is `AwaitingApproval` (withdrawn in `finish`); before it nothing
                // was claimed (Tauri `unanswered_invite`).
                Ok(Err(Unanswered)) => {
                    return match flow.on_response(HttpResponse::new(0, Value::Null)) {
                        // The approval left and was not answered: whether it was applied is not known.
                        Ok(InviteStep::Done(outcome @ InviteOutcome::AwaitingApproval { .. })) => Ok((outcome, true)),
                        _ => Err(InviteRefused::Offline),
                    };
                }
                // The session moved before this request left. When it was the approval, the invite was claimed and
                // holds no key: `AwaitingApproval`, which `finish` withdraws. Before that nothing was claimed.
                Err(refused) => {
                    return match flow.on_response(HttpResponse::new(0, Value::Null)) {
                        Ok(InviteStep::Done(outcome @ InviteOutcome::AwaitingApproval { .. })) => Ok((outcome, false)),
                        _ => Err(refused),
                    };
                }
            };
            let status = resp.status;
            match flow.on_response(resp).map_err(flow_refusal)? {
                InviteStep::Send(next) => req = next,
                InviteStep::Done(outcome) => {
                    let sent = approval_may_have_landed(&outcome, status);
                    return Ok((outcome, sent));
                }
            }
        }
    }

    fn drive_folder(
        &self,
        run: &Run<'_>,
        state: &OwnedState<'_>,
        revision: u64,
        mut flow: FolderShare,
        first: FolderStep,
    ) -> Result<(InviteOutcome, bool), InviteRefused> {
        let mut step = first;
        // The status of the last answer: only a 4xx proves the approval was refused (see `approval_may_have_landed`).
        let mut status = 0;
        loop {
            let FolderSend {
                request,
                mark_custody_seen,
            } = match step {
                FolderStep::Send(send) => send,
                FolderStep::Done(outcome) => {
                    let sent = approval_may_have_landed(&outcome, status);
                    return Ok((outcome, sent));
                }
                // A link is never what an invite makes.
                FolderStep::LinkCreated(_) => return Err(InviteRefused::Failed),
            };
            // L7d: remembered before any key from the record is used. A write that fails: this request is not sent.
            if let Some(folder) = mark_custody_seen {
                state
                    .with(|db| remember_custody(db, &folder, revision))
                    .map_err(state_refusal)?
                    .map_err(|_| InviteRefused::Failed)?;
            }
            let resp = match self.invite_send(run, request) {
                Ok(Ok(resp)) => resp,
                Ok(Err(Unanswered)) => {
                    return match flow.on_response(HttpResponse::new(0, Value::Null)) {
                        Ok(FolderStep::Done(outcome @ InviteOutcome::AwaitingApproval { .. })) => Ok((outcome, true)),
                        _ => Err(InviteRefused::Offline),
                    };
                }
                // As for a file: a claimed invite whose approval never left is withdrawn in `finish`.
                Err(refused) => {
                    return match flow.on_response(HttpResponse::new(0, Value::Null)) {
                        Ok(FolderStep::Done(outcome @ InviteOutcome::AwaitingApproval { .. })) => Ok((outcome, false)),
                        _ => Err(refused),
                    };
                }
            };
            status = resp.status;
            step = flow.on_response(resp).map_err(flow_refusal)?;
        }
    }

    /// Tauri `record_outcome_in` + `finish_invite`: what the outcome answers, the key record it leaves, and the
    /// withdrawal of a claimed invite that holds no key the other side can open.
    fn finish(
        &self,
        run: &Run<'_>,
        state: &OwnedState<'_>,
        own: &str,
        outcome: InviteOutcome,
        approval_sent: bool,
    ) -> InviteAnswer {
        match outcome {
            InviteOutcome::Shared {
                invite_id,
                recipient_public_key,
            } => {
                let hex = Zeroizing::new(hexfmt::encode(&recipient_public_key));
                let written = state
                    .with(|db| db.set_share_sealed_key(own, &invite_id, &hex).is_ok())
                    .unwrap_or(false);
                if !written {
                    // The share is live once the server took the approval: never answered as failed, which would send
                    // the person to make a second one.
                    tracing::warn!("share invite: the sealed-key record was not written; the share is live");
                }
                InviteAnswer::Shared { invite_id }
            }
            InviteOutcome::PeerKeyChanged {
                invite_id,
                recipient_public_key,
            } => {
                let hex = Zeroizing::new(hexfmt::encode(&recipient_public_key));
                let written = state
                    .with(|db| db.set_share_offered_key(own, &invite_id, &hex).is_ok())
                    .unwrap_or(false);
                if written {
                    return InviteAnswer::PeerKeyChanged { invite_id };
                }
                // A changed key that is not recorded can never be accepted later ("Share anyway" seals only recorded
                // bytes): the invite goes.
                match self.withdraw(run, &invite_id) {
                    Ok(()) => InviteAnswer::Refused(InviteRefused::Failed),
                    // No approval was made for a changed key: nothing they can open.
                    Err(_) => InviteAnswer::Unfinished {
                        invite_id,
                        approval_sent: false,
                    },
                }
            }
            InviteOutcome::AwaitingApproval { invite_id, .. } => {
                // The session moved before the approval left (review of desktop#424): withdrawn all the same.
                let moved = !self.session_is_still(run.session, &run.key);
                match self.withdraw(run, &invite_id) {
                    Ok(()) if moved => InviteAnswer::Refused(InviteRefused::SessionEnded),
                    Ok(()) => InviteAnswer::Refused(InviteRefused::Failed),
                    // It stays claimed: never "nothing was shared".
                    Err(_) => InviteAnswer::Unfinished {
                        invite_id,
                        approval_sent,
                    },
                }
            }
            InviteOutcome::Invited { invite_id, .. } => InviteAnswer::Invited { invite_id },
        }
    }

    /// Cancel one invite this run claimed; one already gone counts as withdrawn. Sent while the run's session is the
    /// session in memory, or, after a turn that moved for the same account (the session in memory now holds the run's
    /// vault key: a screen lock, a session write), on the run's token: the module's doc.
    fn withdraw(&self, run: &Run<'_>, invite_id: &str) -> Result<(), InviteRefused> {
        let req = share_flow::cancel_invite_request(invite_id).map_err(|_| InviteRefused::Failed)?;
        let same_account = || self.session_is_still(self.acct.session_generation(), &run.key);
        let sent = if self.session_is_still(run.session, &run.key) || same_account() {
            self.runtime
                .block_on(auth_transport::send_to(run.base, &run.client, Some(&run.token), req))
                .map_err(|_| Unanswered)
        } else {
            return Err(InviteRefused::SessionEnded);
        };
        let resp = sent.map_err(|Unanswered| InviteRefused::Offline)?;
        if resp.ok() || resp.status == 404 {
            Ok(())
        } else {
            tracing::warn!("share invite: an invite that holds no key could not be withdrawn");
            Err(InviteRefused::Failed)
        }
    }
}

/// The custody write a folder step asks for, under the revision the invite started on (the account-bound handle).
fn remember_custody(db: &StateDb, folder: &str, revision: u64) -> rusqlite::Result<()> {
    #[cfg(test)]
    if tests::CUSTODY_WRITE_FAILS.with(std::cell::Cell::get) {
        return Err(rusqlite::Error::InvalidQuery);
    }
    db.remember_folder_custody_at_revision(folder, revision)
}

/// An id as the server writes it: a canonical lower-case hyphenated UUID, never normalized (an id is HKDF info for
/// a key and the match key of the server's invite rows).
pub(super) fn canonical(id: &str) -> Option<&str> {
    match uuid::Uuid::parse_str(id) {
        Ok(u) if u.hyphenated().to_string() == id => Some(id),
        _ => None,
    }
}

/// One of the account's own items, from its local state: `Some(is_folder)`. `None` for an item that is gone or
/// going, a received share's, the Backups root and anything in its tree, and a file whose key this computer cannot
/// have (a foreign id with no owned-key record).
pub(super) fn own_item(db: &StateDb, id: &str) -> rusqlite::Result<Option<bool>> {
    let Some(entry) = db.get_file(id)?.filter(|e| e.status != FileStatus::Trashing) else {
        return Ok(None);
    };
    if db
        .get_file_contract_state(id)?
        .is_some_and(|c| c.namespace == Namespace::SharedWithMe)
    {
        return Ok(None);
    }
    // A row may be stored with a leading slash (`/Backups/x`): both forms are the tree.
    let path = entry.path.trim_start_matches('/');
    let in_backups = db.backups_roots()?.iter().any(|(_, root)| {
        let root = root.trim_start_matches('/');
        path == root || path.strip_prefix(root).is_some_and(|rest| rest.starts_with('/'))
    });
    if in_backups {
        return Ok(None);
    }
    let folder = entry.is_dir();
    if !folder && !matches!(item_class(id), Ok(ItemClass::Owner)) && db.get_owned_key_record(id)?.is_none() {
        return Ok(None);
    }
    Ok(Some(folder))
}

/// The key that opens a file (web `shareFileKey`, Tauri `flows::file_key`): its owned-key record, else an owner id's
/// derivation; a foreign id with no record has none.
pub(super) fn file_key(db: &StateDb, master: &MasterKey, id: &str) -> Option<FileKey> {
    if let Some(record) = db.get_owned_key_record(id).ok()? {
        let kind = owner_keys::kind_in_db(db, id);
        return owner_keys::record_key(master, &record, kind).ok().map(|(key, _)| key);
    }
    matches!(item_class(id), Ok(ItemClass::Owner)).then(|| beebeeb_core::kdf::derive_file_key(master, id.as_bytes()))
}

fn list<'a>(body: &'a Value, field: &str) -> impl Iterator<Item = &'a Value> {
    body.get(field).and_then(Value::as_array).into_iter().flatten()
}

fn text<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    v.get(k).and_then(Value::as_str)
}

/// Whether the account's sent list has an invite of `item` to `person` (normalized) that is not over.
fn already_on_it(sent: &Value, item: &str, person: &str) -> bool {
    list(sent, "invites").any(|i| {
        text(i, "file_id") == Some(item)
            && text(i, "recipient_email").map(normalized_email).as_deref() == Some(person)
            && text(i, "status").is_some_and(|s| ACTIVE.contains(&s))
    })
}

/// Tauri `peers_for_email`: the account ids the sent invites to `person` (normalized) name, in the server's order,
/// each once.
pub(super) fn peers_for_email(sent: &Value, person: &str) -> Vec<String> {
    let mut peers: Vec<String> = Vec::new();
    for invite in list(sent, "invites") {
        if text(invite, "recipient_email").map(normalized_email).as_deref() != Some(person) {
            continue;
        }
        if let Some(peer) = text(invite, "recipient_id").and_then(canonical)
            && !peers.iter().any(|p| p == peer)
        {
            peers.push(peer.to_string());
        }
    }
    peers
}

/// Tauri `expectation_of`: no mark is `NotCompared`; a "They don't match" mark, or marks that pin different keys,
/// expect no key at all ([`NO_KEY_MATCHES`]); else the one key they all pin. A fingerprint that is not 64 hex digits
/// is `None` (stop), never `NotCompared`.
pub(super) fn expectation_of(marks: &[(String, MarkKind)]) -> Option<PeerKeyExpectation> {
    let mut pinned: Option<[u8; 32]> = None;
    let mut no_key = false;
    for (fingerprint, kind) in marks {
        let fingerprint = hexfmt::decode32(fingerprint)?;
        no_key |= *kind == MarkKind::NotMatched || pinned.is_some_and(|p| p != fingerprint);
        pinned = Some(fingerprint);
    }
    Some(match pinned {
        None => PeerKeyExpectation::NotCompared,
        Some(_) if no_key => PeerKeyExpectation::Compared(NO_KEY_MATCHES),
        Some(fingerprint) => PeerKeyExpectation::Compared(fingerprint),
    })
}

/// A core refusal as the answer's fixed words. Never formats the error.
fn flow_refusal(error: ShareFlowError) -> InviteRefused {
    match error {
        ShareFlowError::InvalidInput("email") => InviteRefused::InvalidEmail,
        ShareFlowError::FolderNeedsUpgrade => InviteRefused::FolderNeedsUpgrade,
        ShareFlowError::FolderKeyUnverified => InviteRefused::FolderKeyUnverified,
        ShareFlowError::NotFound => InviteRefused::NotYours,
        ShareFlowError::Unauthorized => InviteRefused::SessionEnded,
        _ => InviteRefused::Failed,
    }
}

fn state_refusal(refused: LocalStateRefused) -> InviteRefused {
    match refused {
        LocalStateRefused::SignedOut
        | LocalStateRefused::Locked
        | LocalStateRefused::SessionEnded
        | LocalStateRefused::SessionChanged => InviteRefused::SessionEnded,
        _ => InviteRefused::Failed,
    }
}

#[cfg(test)]
#[path = "share_invite_api_tests.rs"]
mod tests;
