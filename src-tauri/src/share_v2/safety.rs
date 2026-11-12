//! B05e: the share safety number and the local "compared" mark (contract: workspace
//! `.claude/tasks/backlog/v2-b05e-safety-number-contract-ruled.md`, with Amendment 1).
//!
//! The number comes from core (`share_flow::safety`); only its 12 groups of 5 digits go to the web
//! view. Core derives the own key from the master key; the peer key is the exact key the share
//! was sealed to, which this app records in the state DB when it seals (never a separate
//! `GET /auth/public-key` call).
//!
//! The seal is bound to the mark (core v0.2.8 item 15):
//! - every member seal passes a REQUIRED `PeerKeyExpectation`: `Compared(fingerprint of the mark)`
//!   when this app holds a mark for the person, else `NotCompared`. A server key that differs is
//!   refused by core before a seal byte exists (`peer_key_changed`, DW-0515). A "They don't match"
//!   mark expects no key at all, so nothing is sealed to that person until they choose; a mark
//!   that cannot be read stops the share (it never falls back to `NotCompared`);
//! - "Share anyway" seals to the exact bytes `peer_key_changed` carried, with
//!   `Compared(fingerprint(those bytes))`, never to a re-fetched key, and then pins the mark to
//!   that fingerprint ([`MarkKind::Accepted`]: pinned, never shown as compared), so a further swap
//!   is refused again;
//! - each share keeps the key it was sealed to, and its trust is read against the mark PER SHARE:
//!   a mark set later never vouches for a share sealed to another key;
//! - a person's marks are kept under their address and under every account id my sent invites to
//!   that address name, and every mark write goes to all of them. A seal reads all of them: a
//!   "They don't match", or marks that pin different keys, expect no key at all.
use base64::Engine;
use beebeeb_core::auth_flow::HttpRequest;
use beebeeb_core::kdf::{FileKey, MasterKey};
use beebeeb_core::share_flow::access::MemberPermission;
use beebeeb_core::share_flow::member::{self, InviteOutcome, PeerKeyExpectation, SealedApproval};
use beebeeb_core::share_flow::{ShareFlowError, access, cancel_invite_request, safety};
use serde_json::{Value, json};
use tauri::State;

use super::Answer;
use super::flows::{Unlocked, flow_reason, item_id, list, state_db, unlocked};
use super::reads::{SENT, fetch};
use crate::AppState;
use crate::hexfmt::{decode32, encode as hex};
use crate::state_db::{MarkKind, StateDb, is_unreadable_mark, normalized_email};

const ME: &str = "/api/v1/auth/me";
/// The expectation of a "They don't match" mark: a fingerprint no key has (32 zero bytes is no
/// known SHA-256 output), so core refuses every server key with `peer_key_changed` and nothing is
/// sealed until the person chooses on DW-0515.
const NO_KEY_MATCHES: [u8; 32] = [0; 32];
const NO_PEER_ID: &str = "unsupported: the server does not list this person's account id yet";
/// The reason a share stops on a mark this build cannot read (a row it cannot decode, or a
/// fingerprint that is not 64 hex digits). Distinct from a database that was busy, so the window
/// can offer Compare again rather than a retry: the literal frontend slice 04b reads.
const MARK_UNREADABLE: &str = "mark_unreadable";
const NO_SEALED_KEY: &str = "unsupported: this share was sealed before this app kept the key it sealed to";
/// What a claimed invite answers when core sealed (or could not seal) and the approval did not go
/// through: the invite was withdrawn, so nothing was shared and Try again starts afresh.
pub(super) const NOT_SHARED: &str = "not-shared: the approval did not go through";

/// One share's trust, declared from the most to the least trusted: a person with several marks
/// shows the least trusted of them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Trust {
    Compared,
    NotCompared,
    KeyChanged,
    Mismatch,
    /// A mark this build cannot read: the seal path stops on it, so People says so too.
    Unreadable,
}

/// The 12 groups of 5 digits, and SHA-256 of the peer key (hex) for the mark. Public keys only.
pub(super) fn safety_view(
    master: &MasterKey,
    own: &str,
    peer: &str,
    sealed_to: &[u8; 32],
) -> Answer<(Vec<String>, String)> {
    let n = safety::safety_number(master, own, peer, sealed_to).map_err(flow_reason)?;
    let groups = n.groups().into_iter().map(str::to_owned).collect();
    Ok((groups, hex(&n.peer_fingerprint)))
}

/// What `share_safety_number` answers: the digits, nothing else.
pub(super) fn groups_answer(groups: &[String]) -> Value {
    json!({ "groups": groups })
}

/// One share's trust: its sealed-key fingerprint read against the person's mark, and against the
/// key accepted for this share over People (DW-0515 "Share anyway"), if any.
pub(super) fn trust(mark: Option<(&str, MarkKind)>, share_fp: &str, accepted_for_share: Option<&str>) -> Trust {
    match mark {
        None => Trust::NotCompared,
        Some((fp, kind)) if fp == share_fp => match kind {
            MarkKind::Matched => Trust::Compared,
            MarkKind::NotMatched => Trust::Mismatch,
            MarkKind::Accepted => Trust::NotCompared,
        },
        Some(_) if accepted_for_share == Some(share_fp) => Trust::NotCompared,
        Some(_) => Trust::KeyChanged,
    }
}

/// The `Member` fields a trust state sets.
pub(super) fn trust_fields(t: Trust) -> Value {
    match t {
        Trust::NotCompared => json!({}),
        Trust::Compared => json!({ "compared": true }),
        Trust::Mismatch => json!({ "mismatch": true }),
        Trust::KeyChanged => json!({ "keyChanged": true }),
        Trust::Unreadable => json!({ "markUnreadable": true }),
    }
}

/// The recipient account id of my sent invite `member_id`.
pub(super) fn peer_user_id(sent: &Value, member_id: &str) -> Answer<String> {
    let invite = list(sent, "invites")
        .find(|i| i.get("id").and_then(Value::as_str) == Some(member_id))
        .ok_or_else(|| "not-found".to_string())?;
    match invite.get("recipient_id").and_then(Value::as_str) {
        Some(id) => item_id(id),
        None => Err(NO_PEER_ID.into()),
    }
}

/// `share_set_access` through core: `PATCH /shares/invites/{id}` with the server's permission
/// (`read`, `add`, `write`, `admin`).
pub(super) fn set_access_request(member_id: &str, permission: &str) -> Answer<HttpRequest> {
    let permission = MemberPermission::parse(permission).map_err(|_| "failed".to_string())?;
    access::set_member_access_request(&item_id(member_id)?, permission).map_err(|_| "not-found".to_string())
}

/// My account id: from the profile cached at sign-in, else `/auth/me`. My sharing key is derived
/// inside core from the master key.
pub(super) async fn own_id(u: &Unlocked) -> Answer<String> {
    if let Some(id) = u.own_cached.as_deref().and_then(|id| item_id(id).ok()) {
        return Ok(id);
    }
    let me = fetch(u, ME).await?;
    item_id(me.get("user_id").and_then(Value::as_str).ok_or("failed")?)
}

/// The key invite `member_id` was sealed to, as this app recorded it when sealing.
pub(super) fn sealed_key_in(db: &StateDb, own: &str, member_id: &str) -> Answer<[u8; 32]> {
    db.share_sealed_key(own, member_id)
        .map_err(|_| "failed".to_string())?
        .as_deref()
        .and_then(decode32)
        .ok_or_else(|| NO_SEALED_KEY.to_string())
}

/// Which key a safety number was computed from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum NumberKey {
    /// The key a live share was sealed to.
    Sealed,
    /// The key a changed-key warning offered for a refused invite (DW-0515 "Compare safety numbers").
    Offered,
}

/// The key a safety number is computed from for invite `member_id`: the key it was sealed to, or,
/// for an invite a changed key stopped (DW-0515 "Compare safety numbers"), the key it was offered.
pub(super) fn number_key_in(db: &StateDb, own: &str, member_id: &str) -> Answer<([u8; 32], NumberKey)> {
    match sealed_key_in(db, own, member_id) {
        Ok(key) => Ok((key, NumberKey::Sealed)),
        Err(e) => db
            .share_offered_key(own, member_id)
            .map_err(|_| "failed".to_string())?
            .as_deref()
            .and_then(decode32)
            .map(|key| (key, NumberKey::Offered))
            .ok_or(e),
    }
}

/// The account ids of the people my sent invites to `email` name (more than one when the address
/// was registered again), in the server's order, without repeats.
pub(super) fn peers_for_email(sent: &Value, email: &str) -> Vec<String> {
    let email = normalized_email(email);
    list(sent, "invites")
        .filter(|i| {
            i.get("recipient_email")
                .and_then(Value::as_str)
                .map(normalized_email)
                .as_deref()
                == Some(&email)
        })
        .filter_map(|i| {
            i.get("recipient_id")
                .and_then(Value::as_str)
                .and_then(|p| item_id(p).ok())
        })
        .fold(Vec::new(), |mut peers, p| {
            if !peers.contains(&p) {
                peers.push(p);
            }
            peers
        })
}

/// The person one of my sent invites names: the normalized address, and every account id my sent
/// invites to that address name, with the invite's own id last when the address does not list it.
/// The seal, a comparison and People read and write the same marks through it.
pub(super) fn person_of(sent: &Value, invite: &Value) -> (Option<String>, Vec<String>) {
    let text = |k: &str| invite.get(k).and_then(Value::as_str);
    let email = text("recipient_email").map(normalized_email);
    let mut peers = email.as_deref().map(|e| peers_for_email(sent, e)).unwrap_or_default();
    if let Some(peer) = text("recipient_id").and_then(|p| item_id(p).ok())
        && !peers.contains(&peer)
    {
        peers.push(peer);
    }
    (email, peers)
}

/// Every mark this app holds for a person (their address and each account id). A mark that cannot
/// be read is an error, never "no mark", and gets its own reason ([`MARK_UNREADABLE`]) so the window
/// can tell it from a database that was busy: only the former offers Compare again.
pub(super) fn person_marks_in(
    db: &StateDb,
    own: &str,
    email: Option<&str>,
    peers: &[String],
) -> Answer<Vec<(String, MarkKind)>> {
    db.person_marks(own, email, peers).map_err(|e| {
        if is_unreadable_mark(&e) {
            MARK_UNREADABLE.to_string()
        } else {
            "failed".to_string()
        }
    })
}

/// Writes one mark to every key of the person in one transaction, so no key can pin another key.
pub(super) fn write_person_mark_in(
    db: &StateDb,
    own: &str,
    email: Option<&str>,
    peers: &[String],
    fp: &str,
    kind: MarkKind,
) -> Answer<()> {
    db.set_person_marks(own, email, peers, fp, kind, false)
        .map_err(|_| "failed".to_string())
}

/// What a person's marks expect of the key a seal goes to: `NotCompared` with no mark; no key at
/// all ([`NO_KEY_MATCHES`]) for any "They don't match" or for marks that pin different keys, so
/// nothing is sealed until the person chooses on DW-0515; else the one key they all pin. A
/// fingerprint that is not 64 hex digits is a mark this build cannot read, never `NotCompared`.
pub(super) fn expectation_of(marks: &[(String, MarkKind)]) -> Answer<PeerKeyExpectation> {
    let mut pinned: Option<[u8; 32]> = None;
    let mut no_key = false;
    for (fp, kind) in marks {
        let fp = decode32(fp).ok_or_else(|| MARK_UNREADABLE.to_string())?;
        no_key |= *kind == MarkKind::NotMatched || pinned.is_some_and(|p| p != fp);
        pinned = Some(fp);
    }
    Ok(match pinned {
        None => PeerKeyExpectation::NotCompared,
        Some(_) if no_key => PeerKeyExpectation::Compared(NO_KEY_MATCHES),
        Some(fp) => PeerKeyExpectation::Compared(fp),
    })
}

/// What this app knows of the key of the person `email` names: the marks under the address and
/// under every account id my sent invites to it name (see [`expectation_of`]). The address is
/// always read, so a mark is still found after the last invite to the person is gone. Fails
/// closed: a mark that cannot be read is an error, never `NotCompared`, so a failed read can never
/// let a swapped key through unpinned.
pub(super) fn expectation_in(db: &StateDb, sent: &Value, own: &str, email: &str) -> Answer<PeerKeyExpectation> {
    let peers = peers_for_email(sent, email);
    expectation_of(&person_marks_in(db, own, Some(email), &peers)?)
}

pub(super) fn expectation(sent: &Value, own: &str, email: &str) -> Answer<PeerKeyExpectation> {
    expectation_in(&state_db()?, sent, own, email)
}

/// How an invite ends: an answer for the window, or a claimed invite this app could not finish,
/// which holds no key the other side can open: it is withdrawn before `then` is answered.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Finish {
    Answer(Value),
    Withdraw { invite_id: String, then: &'static str },
}

/// What an invite answers, and the key record it leaves: the sealed key for a share, the offered
/// key for a refused one (DW-0515), so "Share anyway" seals to exactly the key the user was shown.
/// A share is live once the server took the approval: a record that cannot be written then is
/// logged and the answer stays `shared` (its safety number later answers `unsupported`), so the
/// person is never told a live share failed and sent to make a second one. A refused invite whose
/// offered key cannot be recorded can never be sealed ("Share anyway" seals only recorded bytes),
/// and an approval that failed leaves a claimed invite without a key: both are withdrawn.
pub(super) fn record_outcome_in(db: Option<&StateDb>, own: &str, outcome: &InviteOutcome) -> Finish {
    match outcome {
        InviteOutcome::Shared {
            invite_id,
            recipient_public_key,
        } => {
            let written = db.is_some_and(|db| {
                db.set_share_sealed_key(own, invite_id, &hex(recipient_public_key))
                    .is_ok()
            });
            if !written {
                tracing::warn!("share: the sealed-key record was not written; the share is live");
            }
            Finish::Answer(json!({ "status": "shared" }))
        }
        InviteOutcome::PeerKeyChanged {
            invite_id,
            recipient_public_key,
        } => {
            let written = db.is_some_and(|db| {
                db.set_share_offered_key(own, invite_id, &hex(recipient_public_key))
                    .is_ok()
            });
            if written {
                Finish::Answer(json!({ "status": "key-changed", "memberId": invite_id }))
            } else {
                Finish::Withdraw {
                    invite_id: invite_id.clone(),
                    then: "failed",
                }
            }
        }
        InviteOutcome::AwaitingApproval { invite_id, .. } => Finish::Withdraw {
            invite_id: invite_id.clone(),
            then: NOT_SHARED,
        },
        InviteOutcome::Invited { .. } => Finish::Answer(json!({ "status": "invited" })),
    }
}

/// Send a withdrawal (`DELETE /shares/invites/{id}`); an invite already gone is withdrawn.
pub(super) async fn send_withdrawal(u: &Unlocked, req: HttpRequest) -> Answer<()> {
    let resp = super::flows::send(&crate::auth_v2::transport::client()?, u, req).await?;
    if resp.ok() || resp.status == 404 {
        Ok(())
    } else {
        Err(flow_reason(ShareFlowError::from_status(&resp)))
    }
}

/// The end of a file or folder invite: [`record_outcome_in`], and the withdrawal it asks for. When
/// the withdrawal fails too, its own reason is answered (the invite may still be there).
pub(super) async fn finish_invite(u: &Unlocked, own: &str, outcome: &InviteOutcome) -> Answer<Value> {
    let db = state_db().ok();
    match record_outcome_in(db.as_ref(), own, outcome) {
        Finish::Answer(answer) => Ok(answer),
        Finish::Withdraw { invite_id, then } => {
            send_withdrawal(u, cancel_invite_request(&invite_id).map_err(flow_reason)?).await?;
            Err(then.into())
        }
    }
}

/// The refused invite `member` the `key-changed` answer named, checked against my sent list: a
/// claimed invite of `item` to `email`. Never a search: with several claimed invites to the same
/// person, only the one the dialog was about may be sealed.
pub(super) fn refused_invite(sent: &Value, member: &str, item: &str, email: &str) -> Answer<String> {
    let email = normalized_email(email);
    list(sent, "invites")
        .find(|i| {
            i.get("id").and_then(Value::as_str) == Some(member)
                && i.get("file_id")
                    .and_then(Value::as_str)
                    .and_then(|f| item_id(f).ok())
                    .as_deref()
                    == Some(item)
                && i.get("recipient_email")
                    .and_then(Value::as_str)
                    .map(normalized_email)
                    .as_deref()
                    == Some(&email)
                && i.get("status").and_then(Value::as_str) == Some("claimed")
        })
        .and_then(|i| i.get("id").and_then(Value::as_str))
        .ok_or_else(|| "not-found".to_string())
        .and_then(item_id)
}

/// The "Share anyway" seal: the file key sealed to exactly the bytes `peer_key_changed` carried,
/// with `Compared(fingerprint(those bytes))`. There is no input for a server key: a key fetched
/// again can never be what is sealed to, and core checks the bytes against the expectation.
pub(super) fn anyway_seal(
    invite: &str,
    item: &str,
    carried: &[u8; 32],
    master: &MasterKey,
    key: &FileKey,
) -> Result<SealedApproval, ShareFlowError> {
    let expected = safety::key_fingerprint(carried)?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(carried);
    let sealed = member::approve_request(invite, item, &b64, master, key, PeerKeyExpectation::Compared(expected))?;
    if sealed.recipient_public_key != *carried {
        // Core returns the key it sealed to; anything else is not this seal.
        return Err(ShareFlowError::PeerKeyChanged);
    }
    Ok(sealed)
}

/// After "Share anyway" went through: the share's sealed key, and the person's mark in every slot
/// (their address and every account id it names, see [`peers_for_email`]) pinned to the accepted
/// key's fingerprint ([`MarkKind::Accepted`]), so the next seal to the person refuses another key,
/// and an older account id's mark can never stop every later share again. A "They match" mark on
/// that same key (DW-0515 Compare, then the seal) stays `Matched`.
pub(super) fn record_anyway_in(
    db: &StateDb,
    own: &str,
    invite: &str,
    email: Option<&str>,
    peers: &[String],
    sealed_to: &[u8; 32],
) -> Answer<()> {
    db.set_share_sealed_key(own, invite, &hex(sealed_to))
        .map_err(|_| "failed".to_string())?;
    let fp = hex(&safety::key_fingerprint(sealed_to).map_err(flow_reason)?);
    db.set_person_marks(own, email, peers, &fp, MarkKind::Accepted, true)
        .map_err(|_| "failed".to_string())
}

/// DW-0515 "Share anyway" on a folder: core has no folder approval for a changed key yet, so the
/// refused invite the dialog named (checked as [`refused_invite`] checks it) is withdrawn instead of
/// left waiting on the other side as an invitation that cannot open. The `DELETE` for it.
pub(super) fn folder_anyway_withdrawal(sent: &Value, member: &str, item: &str, email: &str) -> Answer<HttpRequest> {
    cancel_invite_request(&refused_invite(sent, member, item, email)?).map_err(flow_reason)
}

/// DW-0515 "Share anyway" for a file: seal the refused invite `member` to the key the user was
/// shown (see [`anyway_seal`]), then pin the mark to it. `own` and `sent` are the ones
/// `share_invite` already read.
pub(super) async fn share_anyway(
    u: &Unlocked,
    own: &str,
    sent: &Value,
    member: &str,
    item: &str,
    email: &str,
    key: &FileKey,
) -> Answer<Value> {
    let invite = refused_invite(sent, member, item, email)?;
    let db = state_db()?;
    let carried = db
        .share_offered_key(own, &invite)
        .map_err(|_| "failed".to_string())?
        .as_deref()
        .and_then(decode32)
        .ok_or_else(|| "not-found".to_string())?;
    let sealed = anyway_seal(&invite, item, &carried, &u.master, key).map_err(flow_reason)?;
    let resp = super::flows::send(&crate::auth_v2::transport::client()?, u, sealed.request).await?;
    if !resp.ok() {
        return Err("failed".into());
    }
    let mut peers = peers_for_email(sent, email);
    if let Ok(peer) = peer_user_id(sent, &invite)
        && !peers.contains(&peer)
    {
        peers.push(peer);
    }
    // The share is live: a record that cannot be written is logged, never reported as a failed share.
    if record_anyway_in(&db, own, &invite, Some(email), &peers, &sealed.recipient_public_key).is_err() {
        tracing::warn!("share: the Share anyway records were not written; the share is live");
    }
    Ok(json!({ "status": "shared" }))
}

/// Everything a number or a mark needs for invite `member_id`, from the key it was sealed to (or
/// was offered): my id, the person (their address and every account id it names), the digits, the
/// fingerprint and which key it was.
struct Resolved {
    own: String,
    email: Option<String>,
    peers: Vec<String>,
    groups: Vec<String>,
    fp: String,
    key: NumberKey,
}

async fn resolve(u: &Unlocked, member_id: &str) -> Answer<Resolved> {
    let member_id = item_id(member_id)?;
    let sent = fetch(u, SENT).await?;
    let peer = peer_user_id(&sent, &member_id)?;
    let invite = list(&sent, "invites")
        .find(|i| i.get("id").and_then(Value::as_str) == Some(member_id.as_str()))
        .ok_or_else(|| "not-found".to_string())?;
    let (email, peers) = person_of(&sent, invite);
    let own = own_id(u).await?;
    let (key, which) = number_key_in(&state_db()?, &own, &member_id)?;
    let (groups, fp) = safety_view(&u.master, &own, &peer, &key)?;
    Ok(Resolved {
        own,
        email,
        peers,
        groups,
        fp,
        key: which,
    })
}

/// DW-0513: the 60 digits as 12 groups of 5, and nothing else.
#[tauri::command]
pub(crate) async fn share_safety_number(state: State<'_, AppState>, member_id: String) -> Answer<Value> {
    let u = unlocked(&state)?;
    Ok(groups_answer(&resolve(&u, &member_id).await?.groups))
}

/// They match / They don't match on the key `fp` (see [`NumberKey`]), written to every slot of the
/// person. "They don't match" on a key a changed-key warning offered keeps the person's marks as
/// they are: the window withdraws that refused invite, a "They match" on the key their shares were
/// sealed to still vouches for those shares, and the next seal to the key is refused again. With
/// no mark at all, it is written as `NotMatched`, so nothing is sealed to the person unasked.
pub(super) fn mark_compared_in(
    db: &StateDb,
    own: &str,
    email: Option<&str>,
    peers: &[String],
    fp: &str,
    matched: bool,
    key: NumberKey,
) -> Answer<()> {
    if !matched && key == NumberKey::Offered && !person_marks_in(db, own, email, peers)?.is_empty() {
        return Ok(());
    }
    let kind = if matched {
        MarkKind::Matched
    } else {
        MarkKind::NotMatched
    };
    write_person_mark_in(db, own, email, peers, fp, kind)
}

/// They match / They don't match: a local mark against the key this share was sealed to.
#[tauri::command]
pub(crate) async fn share_mark_compared(state: State<'_, AppState>, member_id: String, matched: bool) -> Answer<()> {
    let u = unlocked(&state)?;
    let r = resolve(&u, &member_id).await?;
    mark_compared_in(
        &state_db()?,
        &r.own,
        r.email.as_deref(),
        &r.peers,
        &r.fp,
        matched,
        r.key,
    )
}

/// What "Share anyway" over People records: this share's sealed key is accepted for this share.
/// The person's mark is left as it is, so later seals to them stay guarded.
pub(super) fn accept_share_key_in(db: &StateDb, own: &str, member_id: &str) -> Answer<()> {
    let key = sealed_key_in(db, own, member_id)?;
    let fp = safety::key_fingerprint(&key).map_err(flow_reason)?;
    db.set_share_accepted_fingerprint(own, member_id, &hex(&fp))
        .map_err(|_| "failed".to_string())
}

/// DW-0515 over People, "Share anyway": keep this share as it is. Nothing is sealed or sent.
#[tauri::command]
pub(crate) async fn share_accept_changed_key(state: State<'_, AppState>, member_id: String) -> Answer<()> {
    let u = unlocked(&state)?;
    let member_id = item_id(&member_id)?;
    let own = own_id(&u).await?;
    accept_share_key_in(&state_db()?, &own, &member_id)
}

/// The trust of one share of mine (`member_id`, the invite id) to the person (`email` and `peers`,
/// see [`person_marks_in`]), or `None` when they have no mark, or when this app has no record of
/// the key the share was sealed to (a mark never vouches for such a share). A mark that cannot be
/// read is [`Trust::Unreadable`]: the seal path stops on it, and People says so instead of showing
/// an unmarked row. With several marks, the least trusted wins.
pub(super) fn share_trust_in(
    db: &StateDb,
    own: &str,
    email: Option<&str>,
    peers: &[String],
    member_id: &str,
) -> Option<Trust> {
    let marks = match db.person_marks(own, email, peers) {
        Ok(marks) if marks.iter().all(|(fp, _)| decode32(fp).is_some()) => marks,
        Ok(_) => return Some(Trust::Unreadable),
        Err(e) if is_unreadable_mark(&e) => return Some(Trust::Unreadable),
        // A read that failed on the database (busy, locked) says nothing about the mark: the row
        // stays as it was, and the seal path still stops on any failed read.
        Err(_) => return None,
    };
    if marks.is_empty() {
        return None;
    }
    let key = sealed_key_in(db, own, member_id).ok()?;
    let fp = hex(&safety::key_fingerprint(&key).ok()?);
    let accepted = db.share_accepted_fingerprint(own, member_id).ok().flatten();
    marks
        .iter()
        .map(|(mark, kind)| trust(Some((mark.as_str(), *kind)), &fp, accepted.as_deref()))
        .max()
}

/// Adds `compared` / `mismatch` / `keyChanged` / `markUnreadable` to People rows whose person has a
/// mark, per share. `sent` is my whole sent list (a person's other account ids, [`person_of`]).
/// Without the state DB or my id, the rows stay as they were.
pub(super) async fn annotate_members(u: &Unlocked, sent: &Value, invites: &[&Value], rows: &mut [Value]) {
    let Ok(db) = state_db() else { return };
    let mut own: Option<String> = None;
    for (invite, row) in invites.iter().zip(rows.iter_mut()) {
        let Some(member) = invite.get("id").and_then(Value::as_str).and_then(|m| item_id(m).ok()) else {
            continue;
        };
        let (email, peers) = person_of(sent, invite);
        if email.is_none() && peers.is_empty() {
            continue;
        }
        if own.is_none() {
            match own_id(u).await {
                Ok(id) => own = Some(id),
                Err(_) => return,
            }
        }
        let Some(own) = own.as_deref() else { return };
        let Some(t) = share_trust_in(&db, own, email.as_deref(), &peers, &member) else {
            continue;
        };
        if let (Some(obj), Value::Object(extra)) = (row.as_object_mut(), trust_fields(t)) {
            obj.extend(extra);
        }
    }
}
