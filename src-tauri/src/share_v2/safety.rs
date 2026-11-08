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
//!   a mark set later never vouches for a share sealed to another key.
use base64::Engine;
use beebeeb_core::auth_flow::HttpRequest;
use beebeeb_core::kdf::{FileKey, MasterKey};
use beebeeb_core::share_flow::access::MemberPermission;
use beebeeb_core::share_flow::member::{self, InviteOutcome, PeerKeyExpectation, SealedApproval};
use beebeeb_core::share_flow::{ShareFlowError, access, safety};
use serde_json::{Value, json};
use tauri::State;

use super::Answer;
use super::flows::{Unlocked, flow_reason, item_id, list, state_db, unlocked};
use super::reads::{SENT, fetch};
use crate::AppState;
use crate::state_db::{MarkKind, StateDb};

const ME: &str = "/api/v1/auth/me";
/// The expectation of a "They don't match" mark: a fingerprint no key has (32 zero bytes is no
/// known SHA-256 output), so core refuses every server key with `peer_key_changed` and nothing is
/// sealed until the person chooses on DW-0515.
const NO_KEY_MATCHES: [u8; 32] = [0; 32];
const NO_PEER_ID: &str = "unsupported: the server does not list this person's account id yet";
const NO_SEALED_KEY: &str = "unsupported: this share was sealed before this app kept the key it sealed to";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Trust {
    NotCompared,
    Compared,
    Mismatch,
    KeyChanged,
}

pub(super) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
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
        // An accepted, never compared key is no reference a share could differ from.
        Some((_, MarkKind::Accepted)) => Trust::NotCompared,
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

/// My account id (`/auth/me`). My sharing key is derived inside core from the master key.
pub(super) async fn own_id(u: &Unlocked) -> Answer<String> {
    let me = fetch(u, ME).await?;
    item_id(me.get("user_id").and_then(Value::as_str).ok_or("failed")?)
}

fn key_hex(hex_str: &str) -> Option<[u8; 32]> {
    let bytes: Vec<u8> = (0..hex_str.len())
        .step_by(2)
        .map(|i| hex_str.get(i..i + 2).and_then(|b| u8::from_str_radix(b, 16).ok()))
        .collect::<Option<_>>()?;
    bytes.try_into().ok()
}

/// The key invite `member_id` was sealed to, as this app recorded it when sealing.
pub(super) fn sealed_key_in(db: &StateDb, own: &str, member_id: &str) -> Answer<[u8; 32]> {
    db.share_sealed_key(own, member_id)
        .map_err(|_| "failed".to_string())?
        .as_deref()
        .and_then(key_hex)
        .ok_or_else(|| NO_SEALED_KEY.to_string())
}

/// The key a safety number is computed from for invite `member_id`: the key it was sealed to, or,
/// for an invite a changed key stopped (DW-0515 "Compare safety numbers"), the key it was offered.
pub(super) fn number_key_in(db: &StateDb, own: &str, member_id: &str) -> Answer<[u8; 32]> {
    sealed_key_in(db, own, member_id).or_else(|e| {
        db.share_offered_key(own, member_id)
            .map_err(|_| "failed".to_string())?
            .as_deref()
            .and_then(key_hex)
            .ok_or(e)
    })
}

/// The account ids of the people my sent invites to `email` name (more than one when the address
/// was registered again), in the server's order, without repeats.
pub(super) fn peers_for_email(sent: &Value, email: &str) -> Vec<String> {
    let email = email.trim().to_lowercase();
    list(sent, "invites")
        .filter(|i| {
            i.get("recipient_email")
                .and_then(Value::as_str)
                .map(str::to_lowercase)
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

/// What this app knows of the key of the person a sent invite to `email` names: the fingerprint of
/// their mark (`Matched` and `Accepted` pin that key), no key at all for "They don't match", or
/// `NotCompared` when there is no mark. Fails closed: a mark that cannot be read is an error, never
/// `NotCompared`, so a failed read can never let a swapped key through unpinned.
pub(super) fn expectation_in(db: &StateDb, sent: &Value, own: &str, email: &str) -> Answer<PeerKeyExpectation> {
    for peer in peers_for_email(sent, email) {
        match db.share_mark(own, &peer).map_err(|_| "failed".to_string())? {
            None => continue,
            Some((_, MarkKind::NotMatched)) => return Ok(PeerKeyExpectation::Compared(NO_KEY_MATCHES)),
            Some((fp, _)) => {
                return key_hex(&fp)
                    .map(PeerKeyExpectation::Compared)
                    .ok_or_else(|| "failed".to_string());
            }
        }
    }
    Ok(PeerKeyExpectation::NotCompared)
}

pub(super) fn expectation(sent: &Value, own: &str, email: &str) -> Answer<PeerKeyExpectation> {
    expectation_in(&state_db()?, sent, own, email)
}

/// What an invite answers, and the key record it leaves: the sealed key for a share, the offered
/// key for a refused one (DW-0515), so "Share anyway" seals to exactly the key the user was shown.
/// A share is live once the server took the approval: a record that cannot be written then is
/// logged and the answer stays `shared` (its safety number later answers `unsupported`), so the
/// person is never told a live share failed and sent to make a second one.
pub(super) fn record_outcome_in(db: &StateDb, own: &str, outcome: &InviteOutcome) -> Answer<Value> {
    match outcome {
        InviteOutcome::Shared {
            invite_id,
            recipient_public_key,
        } => {
            if db
                .set_share_sealed_key(own, invite_id, &hex(recipient_public_key))
                .is_err()
            {
                tracing::warn!("share: the sealed-key record was not written; the share is live");
            }
            Ok(json!({ "status": "shared" }))
        }
        InviteOutcome::PeerKeyChanged {
            invite_id,
            recipient_public_key,
        } => {
            db.set_share_offered_key(own, invite_id, &hex(recipient_public_key))
                .map_err(|_| "failed".to_string())?;
            Ok(json!({ "status": "key-changed", "memberId": invite_id }))
        }
        _ => Ok(json!({ "status": "invited" })),
    }
}

pub(super) fn record_outcome(own: &str, outcome: &InviteOutcome) -> Answer<Value> {
    record_outcome_in(&state_db()?, own, outcome)
}

/// The claimed invite of `item` to `email` that a changed key stopped.
pub(super) fn refused_invite(sent: &Value, item: &str, email: &str) -> Answer<String> {
    let email = email.trim().to_lowercase();
    list(sent, "invites")
        .find(|i| {
            i.get("file_id")
                .and_then(Value::as_str)
                .and_then(|f| item_id(f).ok())
                .as_deref()
                == Some(item)
                && i.get("recipient_email")
                    .and_then(Value::as_str)
                    .map(str::to_lowercase)
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

/// After "Share anyway" went through: the share's sealed key, and the mark of every account id the
/// address names (`peers`, see [`peers_for_email`]) pinned to the accepted key's fingerprint
/// ([`MarkKind::Accepted`]), so the next seal to the person refuses another key, and an older
/// account id's mark can never stop every later share again.
pub(super) fn record_anyway_in(
    db: &StateDb,
    own: &str,
    invite: &str,
    peers: &[String],
    sealed_to: &[u8; 32],
) -> Answer<()> {
    db.set_share_sealed_key(own, invite, &hex(sealed_to))
        .map_err(|_| "failed".to_string())?;
    let fp = hex(&safety::key_fingerprint(sealed_to).map_err(flow_reason)?);
    for peer in peers {
        db.set_share_mark(own, peer, &fp, MarkKind::Accepted)
            .map_err(|_| "failed".to_string())?;
    }
    Ok(())
}

/// DW-0515 "Share anyway" for a file: seal to the key the user was shown (see [`anyway_seal`]),
/// then pin the mark to it. `own` and `sent` are the ones `share_invite` already read.
pub(super) async fn share_anyway(
    u: &Unlocked,
    own: &str,
    sent: &Value,
    item: &str,
    email: &str,
    key: &FileKey,
) -> Answer<Value> {
    let invite = refused_invite(sent, item, email)?;
    let db = state_db()?;
    let carried = db
        .share_offered_key(own, &invite)
        .map_err(|_| "failed".to_string())?
        .as_deref()
        .and_then(key_hex)
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
    if record_anyway_in(&db, own, &invite, &peers, &sealed.recipient_public_key).is_err() {
        tracing::warn!("share: the Share anyway records were not written; the share is live");
    }
    Ok(json!({ "status": "shared" }))
}

/// Everything a number or a mark needs for invite `member_id`: from the key it was sealed to.
async fn resolve(u: &Unlocked, member_id: &str) -> Answer<(String, String, Vec<String>, String)> {
    let member_id = item_id(member_id)?;
    let peer = peer_user_id(&fetch(u, SENT).await?, &member_id)?;
    let own = own_id(u).await?;
    let key = number_key_in(&state_db()?, &own, &member_id)?;
    let (groups, fp) = safety_view(&u.master, &own, &peer, &key)?;
    Ok((own, peer, groups, fp))
}

/// DW-0513: the 60 digits as 12 groups of 5, and nothing else.
#[tauri::command]
pub(crate) async fn share_safety_number(state: State<'_, AppState>, member_id: String) -> Answer<Value> {
    let u = unlocked(&state)?;
    let (_, _, groups, _) = resolve(&u, &member_id).await?;
    Ok(groups_answer(&groups))
}

/// They match / They don't match: a local mark against the key this share was sealed to.
#[tauri::command]
pub(crate) async fn share_mark_compared(state: State<'_, AppState>, member_id: String, matched: bool) -> Answer<()> {
    let u = unlocked(&state)?;
    let (own, peer, _, fp) = resolve(&u, &member_id).await?;
    let kind = if matched {
        MarkKind::Matched
    } else {
        MarkKind::NotMatched
    };
    state_db()?
        .set_share_mark(&own, &peer, &fp, kind)
        .map_err(|_| "failed".to_string())
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

/// The trust of one share of mine (`member_id`, the invite id) to `peer`, or `None` when this app
/// has no record of the key it was sealed to (a mark never vouches for such a share).
pub(super) fn share_trust_in(db: &StateDb, own: &str, peer: &str, member_id: &str) -> Option<Trust> {
    let mark = db.share_mark(own, peer).ok()??;
    let key = sealed_key_in(db, own, member_id).ok()?;
    let fp = hex(&safety::key_fingerprint(&key).ok()?);
    let accepted = db.share_accepted_fingerprint(own, member_id).ok().flatten();
    Some(trust(Some((mark.0.as_str(), mark.1)), &fp, accepted.as_deref()))
}

/// Adds `compared` / `mismatch` / `keyChanged` to People rows that have a mark, per share. Any
/// failure leaves the row as it was.
pub(super) async fn annotate_members(u: &Unlocked, invites: &[&Value], rows: &mut [Value]) {
    let Ok(db) = state_db() else { return };
    let mut own: Option<String> = None;
    for (invite, row) in invites.iter().zip(rows.iter_mut()) {
        let Some(peer) = invite
            .get("recipient_id")
            .and_then(Value::as_str)
            .and_then(|p| item_id(p).ok())
        else {
            continue;
        };
        if own.is_none() {
            match own_id(u).await {
                Ok(id) => own = Some(id),
                Err(_) => return,
            }
        }
        let Some(own) = own.as_deref() else { return };
        let Some(member) = invite.get("id").and_then(Value::as_str).and_then(|m| item_id(m).ok()) else {
            continue;
        };
        let Some(t) = share_trust_in(&db, own, &peer, &member) else {
            continue;
        };
        if let (Some(obj), Value::Object(extra)) = (row.as_object_mut(), trust_fields(t)) {
            obj.extend(extra);
        }
    }
}
