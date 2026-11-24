//! The share safety number and the local "compared" marks (DW-0513, DW-0515; B05e with its Amendment 1), for a host
//! without a UI toolkit. Ported from the old shell's `share_v2/safety.rs` (`safety_view`, `trust`, `share_trust_in`,
//! `mark_compared_in`, `accept_share_key_in`, `refused_invite`, `anyway_seal`, `record_anyway_in`); the seal's
//! expectation from the marks and the outcome records are `invite_for`'s (`share_invite_api.rs`), read here, not
//! copied.
//!
//! ```mermaid
//! flowchart TD
//!   N[safety_number_for: session, invite] --> S[GET the sent invites: the person's account id and address]
//!   S --> K[the key the share was sealed to, else the key a changed-key warning offered]
//!   K --> C[core: 12 groups of 5 digits, own key derived in core]
//!   M[mark_compared_for] --> S
//!   C -->|They match / They don't match| W[one mark, written to the address and every account id, one transaction]
//!   A[share_anyway_for: refused invite] --> R[the refused invite the dialog named, claimed, of this item and address]
//!   R -->|file| X[seal to the exact offered bytes, Compared fingerprint of them] --> P[share keeps that key, every slot pinned Accepted]
//!   R -->|folder| D[withdraw the refused invite]
//!   T[share_trust_for] --> S2[GET the sent invites] --> Q[each share's sealed key read against the person's marks]
//! ```
//!
//! Rules held here:
//! - The number is core's (`share_flow::safety`): the own key is derived inside core from the vault key; the peer key
//!   is the EXACT key the share was sealed to (recorded by `invite_for` when it sealed), or, for an invite a changed
//!   key stopped (DW-0515 "Compare safety numbers"), the key that warning carried. Never a separate key lookup.
//! - Only the 12 groups of 5 digits leave the engine: never a key, a fingerprint, an address or a server's words.
//! - A mark goes to every slot of the person (the digest of their address, and every account id my sent invites to
//!   that address name) in one transaction, so no slot can pin another key. "They don't match" on a key a changed-key
//!   warning offered keeps marks already there (a "They match" on the key the live shares were sealed to still vouches
//!   for them, and the next seal to the offered key is refused again); with no mark at all it is written down, so
//!   nothing is sealed to the person unasked. Closing that comparison withdraws the refused invite: the host's
//!   [`Engine::share_change`] with `CancelInvites` (it knows the invite from `InviteAnswer::PeerKeyChanged`).
//! - A share's trust is read PER SHARE against the key that share was sealed to: a mark set later never vouches for a
//!   share sealed to another key; a share with no sealed-key record shows nothing; with several marks the least
//!   trusted wins; a mark that cannot be read is [`ShareTrust::Unreadable`] (the seal stops on it too), while a
//!   database that cannot be read says nothing about the mark.
//! - "Share anyway" (DW-0515) on a file seals to exactly the bytes the refused invite carried (its recorded offered
//!   key), with `Compared(fingerprint(those bytes))`. There is no input for a server key: a key fetched again can never
//!   be what is sealed to. After Beebeeb took the approval, the share keeps that key and every slot of the person is
//!   pinned to it ([`MarkKind::Accepted`]: pinned, never shown as compared), so a further swap is refused again. A
//!   "They match" on that same key stays. On a folder (core has no folder approval for a changed key yet) the refused
//!   invite the dialog named is withdrawn instead of left waiting as an invitation that cannot open.
//! - "Share anyway" over People (a share flagged as key changed) records only that THIS share's key is accepted; the
//!   person's marks stay and keep guarding later seals. Nothing is sent.
//! - Session-bound like its neighbours: every call is asked for the session of the press; a request is sent, and an
//!   answer handed over, only while that session is still the one in memory; a session that moved answers
//!   [`SafetyRefused::NotSignedIn`], which the host drops. A locked screen refuses every call.
//!
//! Every method BLOCKS (the network and the state database): a worker's, never the window's thread.
//! Must not: log or return a key, a fingerprint, a sealed key, an address, an id the person did not pass in, or a
//! server's words. A `ShareFlowError`, an `HttpRequest` or a `SealedApproval` is never formatted.

use beebeeb_core::auth_flow::{HttpRequest, HttpResponse};
use beebeeb_core::kdf::{FileKey, MasterKey};
use beebeeb_core::share_flow::member::{self, PeerKeyExpectation, SealedApproval};
use beebeeb_core::share_flow::{self, ShareFlowError, safety};
use serde_json::Value;
use zeroize::Zeroizing;

use crate::account::SessionGeneration;
use crate::app::bindings::share_folder_link::get;
use crate::app::lock_api::PersonRefused;
use crate::app::owned_state::{LocalStateRefused, OwnedState};
use crate::app::share_invite_api::{canonical, file_key, own_item, peers_for_email};
use crate::app::{CredentialStores, Engine};
use crate::auth_transport;
use crate::hexfmt;
use crate::state_db::{MarkKind, StateDb, is_unreadable_mark, normalized_email};

const SENT: &str = "/api/v1/shares/invites/sent";
const ME: &str = "/api/v1/auth/me";

/// DW-0513: the safety number of one share, as the 12 groups of 5 digits two people read out to each other. Nothing
/// else: no key, no fingerprint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SafetyNumber {
    pub groups: Vec<String>,
}

/// One share's trust, declared from the most to the least trusted: a person with several marks shows the least
/// trusted of them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ShareTrust {
    /// The numbers were compared on the key this share was sealed to, and they matched.
    Compared,
    /// Nothing compared for this share's key (an accepted key, or a key accepted for this share over People).
    NotCompared,
    /// The person's mark pins another key than the one this share was sealed to.
    KeyChanged,
    /// "They don't match" on the key this share was sealed to.
    Mismatch,
    /// A mark this build cannot read: the seal stops on it, so People says so too.
    Unreadable,
}

/// The trust of one share of the account's, by the invite id People names the person by.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemberTrust {
    pub invite_id: String,
    pub trust: ShareTrust,
}

/// How "Share anyway" (DW-0515) ended. Ids only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShareAnyway {
    /// The file's key was sealed to the key the warning showed, and Beebeeb took the approval: they have access.
    Shared { invite_id: String },
    /// A folder: the refused invite was withdrawn (core cannot seal a folder to a changed key yet). Nothing is shared.
    Withdrawn { invite_id: String },
}

/// Why a safety call did nothing (or, for [`SafetyRefused::ApprovalUnanswered`], why its end is not known). Fixed
/// words: the host has its own copy for each.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SafetyRefused {
    /// No session in memory, or not the session of the press any more: the host drops the answer.
    NotSignedIn,
    /// The screen is locked, or the vault is closed.
    Locked,
    /// Beebeeb no longer accepts this session.
    SessionEnded,
    /// Beebeeb did not answer; nothing was changed on it.
    Offline,
    /// Not one of the account's sent invites, not the refused invite the dialog named, or no key it was offered.
    NotFound,
    /// Not one of the account's own items here.
    NotYours,
    /// Beebeeb does not list this person's account id yet: a safety number cannot be computed.
    NoPeerId,
    /// This share was sealed before this app kept the key it sealed to: there is no key to compare.
    NoSealedKey,
    /// "Share anyway": the approval left and no answer said whether Beebeeb took it, so they may already have access.
    /// Nothing was recorded here: the next seal to them still expects the key their marks pin.
    ApprovalUnanswered,
    Failed,
}

impl SafetyRefused {
    pub fn kind(self) -> &'static str {
        match self {
            Self::NotSignedIn => "not_signed_in",
            Self::Locked => "locked",
            Self::SessionEnded => "session_ended",
            Self::Offline => "offline",
            Self::NotFound => "not_found",
            Self::NotYours => "not_yours",
            Self::NoPeerId => "no_peer_id",
            Self::NoSealedKey => "no_sealed_key",
            Self::ApprovalUnanswered => "approval_unanswered",
            Self::Failed => "failed",
        }
    }
}

impl std::fmt::Display for SafetyRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.kind())
    }
}

/// Which key a safety number was computed from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NumberKey {
    /// The key a live share was sealed to.
    Sealed,
    /// The key a changed-key warning offered for a refused invite (DW-0515 "Compare safety numbers").
    Offered,
}

/// What every send of one call needs: the session it was asked under and that session's token and vault key (copied
/// once, wiped on drop), and where to send.
struct Asked {
    session: SessionGeneration,
    token: Zeroizing<String>,
    key: Zeroizing<[u8; 32]>,
    base: String,
    client: reqwest::Client,
}

/// A request that got no answer (the network dropped, or it never left).
struct Unanswered;

/// Everything a number or a mark needs for one invite: my id, the person (their address and every account id it
/// names), the digits, the fingerprint of the key and which key it was.
struct Resolved {
    own: String,
    email: Option<String>,
    peers: Vec<String>,
    groups: Vec<String>,
    fingerprint: String,
    key: NumberKey,
}

impl<C: CredentialStores> Engine<C> {
    /// DW-0513: the safety number of the account's share `member_id` (the invite id People names the person by),
    /// from the key that share was sealed to, or, for an invite a changed key stopped, from the key it was offered.
    /// BLOCKS.
    pub fn safety_number_for(
        &self,
        session: SessionGeneration,
        member_id: &str,
    ) -> Result<SafetyNumber, SafetyRefused> {
        let asked = self.safety_asked(session)?;
        let state = self.safety_state(session, false)?;
        let resolved = self.resolve(&asked, &state, member_id)?;
        self.safety_still(&asked)?;
        Ok(SafetyNumber {
            groups: resolved.groups,
        })
    }

    /// They match (`matched`) / They don't match, on the key the number of `member_id` was computed from: a local
    /// mark, written to every slot of the person. Nothing is sent but the reads. See the module's doc for "They don't
    /// match" on an offered key. BLOCKS.
    pub fn mark_compared_for(
        &self,
        session: SessionGeneration,
        member_id: &str,
        matched: bool,
    ) -> Result<(), SafetyRefused> {
        let asked = self.safety_asked(session)?;
        let state = self.safety_state(session, true)?;
        let r = self.resolve(&asked, &state, member_id)?;
        self.safety_still(&asked)?;
        state
            .with(|db| mark_compared_in(db, &r.own, r.email.as_deref(), &r.peers, &r.fingerprint, matched, r.key))
            .map_err(state_refusal)?
    }

    /// DW-0515 over People, "Share anyway": keep the share `member_id` as it is, its key accepted for this share
    /// only. The person's marks stay. Nothing is sealed or sent (but `/auth/me` when the session does not name the
    /// account). BLOCKS.
    pub fn accept_changed_key_for(&self, session: SessionGeneration, member_id: &str) -> Result<(), SafetyRefused> {
        let asked = self.safety_asked(session)?;
        let member = canonical(member_id).ok_or(SafetyRefused::NotFound)?;
        let state = self.safety_state(session, true)?;
        let own = self.safety_own_id(&asked)?;
        self.safety_still(&asked)?;
        state
            .with(|db| accept_share_key_in(db, &own, member))
            .map_err(state_refusal)?
    }

    /// WRITES on the server. DW-0515 "Share anyway" on the refused invite `member_id` of the account's item `item_id`
    /// to `email` (the invite `invite_for` answered as `PeerKeyChanged`): a file is sealed to exactly the key that
    /// answer carried, a folder's invite is withdrawn. See the module's doc. BLOCKS.
    pub fn share_anyway_for(
        &self,
        session: SessionGeneration,
        member_id: &str,
        item_id: &str,
        email: &str,
    ) -> Result<ShareAnyway, SafetyRefused> {
        let asked = self.safety_asked(session)?;
        let item = canonical(item_id).ok_or(SafetyRefused::NotYours)?;
        let member = canonical(member_id).ok_or(SafetyRefused::NotFound)?;
        let state = self.safety_state(session, true)?;
        let is_folder = state
            .with(|db| own_item(db, item))
            .map_err(state_refusal)?
            .map_err(|_| SafetyRefused::Failed)?
            .ok_or(SafetyRefused::NotYours)?;
        let own = self.safety_own_id(&asked)?;
        let sent = self.safety_read(&asked, SENT)?;
        let invite = refused_invite(&sent, member, item, email)?;

        if is_folder {
            let withdrawal = share_flow::cancel_invite_request(&invite).map_err(|_| SafetyRefused::Failed)?;
            let resp = self
                .safety_send(&asked, withdrawal)?
                .map_err(|Unanswered| SafetyRefused::Offline)?;
            return match resp.status {
                // An invite already gone is withdrawn.
                200..=299 | 404 => Ok(ShareAnyway::Withdrawn { invite_id: invite }),
                401 => Err(SafetyRefused::SessionEnded),
                _ => Err(SafetyRefused::Failed),
            };
        }

        let carried = state
            .with(|db| db.share_offered_key(&own, &invite))
            .map_err(state_refusal)?
            .map_err(|_| SafetyRefused::Failed)?
            .as_deref()
            .and_then(hexfmt::decode32)
            .ok_or(SafetyRefused::NotFound)?;
        let master = MasterKey::from_bytes(*asked.key);
        let file_key = state
            .with(|db| file_key(db, &master, item))
            .map_err(state_refusal)?
            .ok_or(SafetyRefused::NotYours)?;
        let sealed = anyway_seal(&invite, item, &carried, &master, &file_key);
        drop(file_key);
        drop(master);
        let SealedApproval {
            request,
            recipient_public_key,
        } = sealed.map_err(|_| SafetyRefused::Failed)?;

        let resp = match self.safety_send(&asked, request)? {
            Ok(resp) => resp,
            // The approval left and its answer was lost: Beebeeb may have taken it.
            Err(Unanswered) => return Err(SafetyRefused::ApprovalUnanswered),
        };
        match resp.status {
            200..=299 => {}
            401 => return Err(SafetyRefused::SessionEnded),
            // Only a 4xx proves Beebeeb did not take it.
            400..=499 => return Err(SafetyRefused::Failed),
            _ => return Err(SafetyRefused::ApprovalUnanswered),
        }

        let mut peers = peers_for_email(&sent, &normalized_email(email));
        if let Ok(peer) = peer_user_id(&sent, &invite)
            && !peers.contains(&peer)
        {
            peers.push(peer);
        }
        // The share is live: a record that cannot be written is logged, never reported as a failed share.
        let recorded = state.with(|db| record_anyway_in(db, &own, &invite, Some(email), &peers, &recipient_public_key));
        if !matches!(recorded, Ok(Ok(()))) {
            tracing::warn!("share anyway: the records were not written; the share is live");
        }
        Ok(ShareAnyway::Shared { invite_id: invite })
    }

    /// People's marks: the trust of each of the account's shares whose person has a mark, per share (see
    /// [`ShareTrust`]), by invite id. A share with no mark, or with no record of the key it was sealed to, is not in
    /// the answer. One read of the sent invites. BLOCKS.
    pub fn share_trust_for(&self, session: SessionGeneration) -> Result<Vec<MemberTrust>, SafetyRefused> {
        let asked = self.safety_asked(session)?;
        let state = self.safety_state(session, false)?;
        let sent = self.safety_read(&asked, SENT)?;
        let own = self.safety_own_id(&asked)?;
        let rows = state
            .with(|db| {
                list(&sent, "invites")
                    .filter_map(|invite| {
                        let member = text(invite, "id").and_then(canonical)?;
                        let (email, peers) = person_of(&sent, invite);
                        if email.is_none() && peers.is_empty() {
                            return None;
                        }
                        share_trust_in(db, &own, email.as_deref(), &peers, member).map(|trust| MemberTrust {
                            invite_id: member.to_string(),
                            trust,
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .map_err(state_refusal)?;
        self.safety_still(&asked)?;
        Ok(rows)
    }

    /// The session of the press, its token and vault key: refused when it is not the session in memory, or a locked
    /// screen or a closed vault refuses a person's call.
    fn safety_asked(&self, session: SessionGeneration) -> Result<Asked, SafetyRefused> {
        let held = self.person_session(|current, generation| {
            (generation == session).then(|| {
                (
                    Zeroizing::new(current.token.clone()),
                    Zeroizing::new(current.master_key),
                )
            })
        });
        let (token, key) = match held {
            Ok(Some(held)) => held,
            Ok(None) | Err(PersonRefused::NoSession) => return Err(SafetyRefused::NotSignedIn),
            Err(PersonRefused::ScreenLocked | PersonRefused::VaultClosed) => return Err(SafetyRefused::Locked),
        };
        let client = auth_transport::client_for(&self.config.api_base).map_err(|_| SafetyRefused::Offline)?;
        Ok(Asked {
            session,
            token,
            key,
            base: self.config.api_base.trim_end_matches('/').to_string(),
            client,
        })
    }

    /// The account's local state for the session of the press (`change`: for a write).
    fn safety_state(&self, session: SessionGeneration, change: bool) -> Result<OwnedState<'_>, SafetyRefused> {
        let state = if change {
            self.local_state_for_changes()
        } else {
            self.local_state()
        }
        .map_err(state_refusal)?;
        if state.session() != session {
            return Err(SafetyRefused::NotSignedIn);
        }
        Ok(state)
    }

    /// Whether the session in memory is still the very one `asked` was copied from.
    fn safety_still(&self, asked: &Asked) -> Result<(), SafetyRefused> {
        if self.session_is_still(asked.session, &asked.key) {
            Ok(())
        } else {
            Err(SafetyRefused::NotSignedIn)
        }
    }

    /// The account's id, as `invite_for` keys the marks by: the id the session named, else Beebeeb's answer for it.
    fn safety_own_id(&self, asked: &Asked) -> Result<String, SafetyRefused> {
        if let Some(own) = self.own_account_id() {
            return Ok(own);
        }
        let me = self.safety_read(asked, ME)?;
        text(&me, "user_id")
            .and_then(canonical)
            .map(str::to_string)
            .ok_or(SafetyRefused::Failed)
    }

    /// Send one request, only while the session of the press is still the session in memory.
    fn safety_send(&self, asked: &Asked, req: HttpRequest) -> Result<Result<HttpResponse, Unanswered>, SafetyRefused> {
        self.safety_still(asked)?;
        Ok(self
            .runtime
            .block_on(auth_transport::send_to(
                &asked.base,
                &asked.client,
                Some(&asked.token),
                req,
            ))
            .map_err(|_| Unanswered))
    }

    fn safety_read(&self, asked: &Asked, path: &str) -> Result<Value, SafetyRefused> {
        let resp = self
            .safety_send(asked, get(path))?
            .map_err(|Unanswered| SafetyRefused::Offline)?;
        match resp.status {
            200..=299 if resp.body.is_object() => Ok(resp.body),
            401 => Err(SafetyRefused::SessionEnded),
            _ => Err(SafetyRefused::Failed),
        }
    }

    /// Tauri `resolve`: the person and the number of my sent invite `member_id`.
    fn resolve(&self, asked: &Asked, state: &OwnedState<'_>, member_id: &str) -> Result<Resolved, SafetyRefused> {
        let member = canonical(member_id).ok_or(SafetyRefused::NotFound)?;
        let sent = self.safety_read(asked, SENT)?;
        let peer = peer_user_id(&sent, member)?;
        let invite = invite_of(&sent, member).ok_or(SafetyRefused::NotFound)?;
        let (email, peers) = person_of(&sent, invite);
        let own = self.safety_own_id(asked)?;
        let (key, which) = state
            .with(|db| number_key_in(db, &own, member))
            .map_err(state_refusal)??;
        let master = MasterKey::from_bytes(*asked.key);
        let (groups, fingerprint) = safety_view(&master, &own, &peer, &key)?;
        drop(master);
        Ok(Resolved {
            own,
            email,
            peers,
            groups,
            fingerprint,
            key: which,
        })
    }
}

/// The 12 groups of 5 digits, and SHA-256 of the peer key (hex) for the mark. Public keys only; the own key is
/// derived in core from the vault key. Core refuses the own id as the peer.
fn safety_view(
    master: &MasterKey,
    own: &str,
    peer: &str,
    sealed_to: &[u8; 32],
) -> Result<(Vec<String>, String), SafetyRefused> {
    let n = safety::safety_number(master, own, peer, sealed_to).map_err(|_| SafetyRefused::Failed)?;
    let groups = n.groups().into_iter().map(str::to_owned).collect();
    Ok((groups, hexfmt::encode(&n.peer_fingerprint)))
}

/// One share's trust: its sealed-key fingerprint read against one mark of the person, and against the key accepted
/// for this share over People (DW-0515 "Share anyway"), if any.
fn trust(mark: Option<(&str, MarkKind)>, share_fp: &str, accepted_for_share: Option<&str>) -> ShareTrust {
    match mark {
        None => ShareTrust::NotCompared,
        Some((fp, kind)) if fp == share_fp => match kind {
            MarkKind::Matched => ShareTrust::Compared,
            MarkKind::NotMatched => ShareTrust::Mismatch,
            MarkKind::Accepted => ShareTrust::NotCompared,
        },
        Some(_) if accepted_for_share == Some(share_fp) => ShareTrust::NotCompared,
        Some(_) => ShareTrust::KeyChanged,
    }
}

fn list<'a>(body: &'a Value, field: &str) -> impl Iterator<Item = &'a Value> {
    body.get(field).and_then(Value::as_array).into_iter().flatten()
}

fn text<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    v.get(k).and_then(Value::as_str)
}

/// My sent invite `member_id`.
fn invite_of<'a>(sent: &'a Value, member_id: &str) -> Option<&'a Value> {
    list(sent, "invites").find(|i| text(i, "id") == Some(member_id))
}

/// The recipient account id of my sent invite `member_id`.
fn peer_user_id(sent: &Value, member_id: &str) -> Result<String, SafetyRefused> {
    let invite = invite_of(sent, member_id).ok_or(SafetyRefused::NotFound)?;
    match text(invite, "recipient_id") {
        Some(id) => canonical(id).map(str::to_string).ok_or(SafetyRefused::NotFound),
        None => Err(SafetyRefused::NoPeerId),
    }
}

/// The person one of my sent invites names: the normalized address, and every account id my sent invites to that
/// address name, with the invite's own id last when the address does not list it. The seal, a comparison and People
/// read and write the same marks through it.
fn person_of(sent: &Value, invite: &Value) -> (Option<String>, Vec<String>) {
    let email = text(invite, "recipient_email").map(normalized_email);
    let mut peers = email.as_deref().map(|e| peers_for_email(sent, e)).unwrap_or_default();
    if let Some(peer) = text(invite, "recipient_id").and_then(canonical)
        && !peers.iter().any(|p| p == peer)
    {
        peers.push(peer.to_string());
    }
    (email, peers)
}

/// The key invite `member_id` was sealed to, as `invite_for` recorded it when sealing.
fn sealed_key_in(db: &StateDb, own: &str, member_id: &str) -> Result<[u8; 32], SafetyRefused> {
    db.share_sealed_key(own, member_id)
        .map_err(|_| SafetyRefused::Failed)?
        .as_deref()
        .and_then(hexfmt::decode32)
        .ok_or(SafetyRefused::NoSealedKey)
}

/// The key a safety number is computed from for invite `member_id`: the key it was sealed to, or, for an invite a
/// changed key stopped (DW-0515 "Compare safety numbers"), the key it was offered.
fn number_key_in(db: &StateDb, own: &str, member_id: &str) -> Result<([u8; 32], NumberKey), SafetyRefused> {
    match sealed_key_in(db, own, member_id) {
        Ok(key) => Ok((key, NumberKey::Sealed)),
        Err(e) => db
            .share_offered_key(own, member_id)
            .map_err(|_| SafetyRefused::Failed)?
            .as_deref()
            .and_then(hexfmt::decode32)
            .map(|key| (key, NumberKey::Offered))
            .ok_or(e),
    }
}

/// They match / They don't match on the key `fp` (see [`NumberKey`]), written to every slot of the person in one
/// transaction. "They don't match" on a key a changed-key warning offered keeps the person's marks as they are; with
/// no mark at all, it is written as `NotMatched`, so nothing is sealed to the person unasked. On that path only, a
/// mark that cannot be read stops it. They match, and They don't match on a sealed key, write every slot without
/// reading it (the old shell's rule): the comparison just made replaces whatever a slot held, a row this build
/// cannot read included.
fn mark_compared_in(
    db: &StateDb,
    own: &str,
    email: Option<&str>,
    peers: &[String],
    fp: &str,
    matched: bool,
    key: NumberKey,
) -> Result<(), SafetyRefused> {
    if !matched
        && key == NumberKey::Offered
        && !db
            .person_marks(own, email, peers)
            .map_err(|_| SafetyRefused::Failed)?
            .is_empty()
    {
        return Ok(());
    }
    let kind = if matched {
        MarkKind::Matched
    } else {
        MarkKind::NotMatched
    };
    db.set_person_marks(own, email, peers, fp, kind, false)
        .map_err(|_| SafetyRefused::Failed)
}

/// What "Share anyway" over People records: this share's sealed key is accepted for this share. The person's mark is
/// left as it is, so later seals to them stay guarded.
fn accept_share_key_in(db: &StateDb, own: &str, member_id: &str) -> Result<(), SafetyRefused> {
    let key = sealed_key_in(db, own, member_id)?;
    let fp = safety::key_fingerprint(&key).map_err(|_| SafetyRefused::Failed)?;
    db.set_share_accepted_fingerprint(own, member_id, &hexfmt::encode(&fp))
        .map_err(|_| SafetyRefused::Failed)
}

/// The trust of one share of mine (`member_id`) to the person (`email` and `peers`), or `None` when they have no mark,
/// or when this app has no record of the key the share was sealed to (a mark never vouches for such a share). A mark
/// that cannot be read is [`ShareTrust::Unreadable`]; with several marks, the least trusted wins.
fn share_trust_in(
    db: &StateDb,
    own: &str,
    email: Option<&str>,
    peers: &[String],
    member_id: &str,
) -> Option<ShareTrust> {
    let marks = match db.person_marks(own, email, peers) {
        Ok(marks) if marks.iter().all(|(fp, _)| hexfmt::decode32(fp).is_some()) => marks,
        Ok(_) => return Some(ShareTrust::Unreadable),
        Err(e) if is_unreadable_mark(&e) => return Some(ShareTrust::Unreadable),
        // A read that failed on the database (busy, locked) says nothing about the mark: the row stays as it was,
        // and the seal path still stops on any failed read.
        Err(_) => return None,
    };
    if marks.is_empty() {
        return None;
    }
    let key = sealed_key_in(db, own, member_id).ok()?;
    let fp = hexfmt::encode(&safety::key_fingerprint(&key).ok()?);
    let accepted = db.share_accepted_fingerprint(own, member_id).ok().flatten();
    marks
        .iter()
        .map(|(mark, kind)| trust(Some((mark.as_str(), *kind)), &fp, accepted.as_deref()))
        .max()
}

/// The refused invite `member` the changed-key answer named, checked against my sent list: a claimed invite of `item`
/// to `email`. Never a search: with several claimed invites to the same person, only the one the dialog was about.
fn refused_invite(sent: &Value, member: &str, item: &str, email: &str) -> Result<String, SafetyRefused> {
    let email = normalized_email(email);
    list(sent, "invites")
        .find(|i| {
            text(i, "id") == Some(member)
                && text(i, "file_id").and_then(canonical) == Some(item)
                && text(i, "recipient_email").map(normalized_email).as_deref() == Some(email.as_str())
                && text(i, "status") == Some("claimed")
        })
        .and_then(|i| text(i, "id"))
        .and_then(canonical)
        .map(str::to_string)
        .ok_or(SafetyRefused::NotFound)
}

/// The "Share anyway" seal: the file key sealed to exactly the bytes the changed-key answer carried, with
/// `Compared(fingerprint(those bytes))`. There is no input for a server key; core checks the bytes against the
/// expectation, and the key it returns must be those bytes.
fn anyway_seal(
    invite: &str,
    item: &str,
    carried: &[u8; 32],
    master: &MasterKey,
    key: &FileKey,
) -> Result<SealedApproval, ShareFlowError> {
    use base64::Engine as _;
    let expected = safety::key_fingerprint(carried)?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(carried);
    let sealed = member::approve_request(invite, item, &b64, master, key, PeerKeyExpectation::Compared(expected))?;
    if sealed.recipient_public_key != *carried {
        // Core returns the key it sealed to; anything else is not this seal.
        return Err(ShareFlowError::PeerKeyChanged);
    }
    Ok(sealed)
}

/// After "Share anyway" went through: the share's sealed key, and the person's mark in every slot pinned to the
/// accepted key's fingerprint ([`MarkKind::Accepted`]), so the next seal to the person refuses another key, and an
/// older account id's mark can never stop every later share again. A "They match" on that same key stays.
fn record_anyway_in(
    db: &StateDb,
    own: &str,
    invite: &str,
    email: Option<&str>,
    peers: &[String],
    sealed_to: &[u8; 32],
) -> Result<(), SafetyRefused> {
    let hex = Zeroizing::new(hexfmt::encode(sealed_to));
    db.set_share_sealed_key(own, invite, &hex)
        .map_err(|_| SafetyRefused::Failed)?;
    let fp = hexfmt::encode(&safety::key_fingerprint(sealed_to).map_err(|_| SafetyRefused::Failed)?);
    db.set_person_marks(own, email, peers, &fp, MarkKind::Accepted, true)
        .map_err(|_| SafetyRefused::Failed)
}

fn state_refusal(refused: LocalStateRefused) -> SafetyRefused {
    match refused {
        LocalStateRefused::SignedOut | LocalStateRefused::SessionChanged => SafetyRefused::NotSignedIn,
        LocalStateRefused::Locked => SafetyRefused::Locked,
        LocalStateRefused::SessionEnded => SafetyRefused::SessionEnded,
        _ => SafetyRefused::Failed,
    }
}

#[cfg(test)]
#[path = "share_safety_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "share_safety_engine_tests.rs"]
mod engine_tests;
