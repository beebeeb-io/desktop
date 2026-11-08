//! B05e: the share safety number and the local "compared" mark (contract: workspace
//! `.claude/tasks/backlog/v2-b05e-safety-number-contract-ruled.md`).
//!
//! The number comes from core (`share_flow::safety`); only its 12 groups of 5 digits go to the web
//! view. Core derives the own key from the master key; the peer key is the exact key the share
//! was sealed to, which this app records in the state DB when it seals (never a separate
//! `GET /auth/public-key` call). A mark is kept per (account id, peer user id) with SHA-256 of
//! that key; every later seal to the person passes it to core as `expected_peer_fingerprint`, and
//! a server key that differs stops before sealing (`peer_key_changed`, DW-0515).
use base64::Engine;
use beebeeb_core::auth_flow::HttpRequest;
use beebeeb_core::kdf::{FileKey, MasterKey};
use beebeeb_core::share_flow::access::MemberPermission;
use beebeeb_core::share_flow::member::{self, InviteOutcome};
use beebeeb_core::share_flow::{access, safety};
use serde_json::{Value, json};
use tauri::State;

use super::Answer;
use super::flows::{Unlocked, flow_reason, item_id, list, state_db, unlocked};
use super::reads::fetch;
use crate::AppState;

const SENT: &str = "/api/v1/shares/invites/sent";
const ME: &str = "/api/v1/auth/me";
const NO_PEER_ID: &str = "unsupported: the server does not list this person's account id yet";
const NO_SEALED_KEY: &str = "unsupported: this share was sealed before this app kept the key it sealed to";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Trust {
    NotCompared,
    Compared,
    Mismatch,
    KeyChanged,
}

fn hex(bytes: &[u8]) -> String {
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

pub(super) fn trust(mark: Option<(String, bool)>, current_fp: &str) -> Trust {
    match mark {
        None => Trust::NotCompared,
        Some((fp, _)) if fp != current_fp => Trust::KeyChanged,
        Some((_, true)) => Trust::Compared,
        Some((_, false)) => Trust::Mismatch,
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
pub(super) fn sealed_key(own: &str, member_id: &str) -> Answer<[u8; 32]> {
    state_db()?
        .share_sealed_key(own, member_id)
        .map_err(|_| "failed".to_string())?
        .as_deref()
        .and_then(key_hex)
        .ok_or_else(|| NO_SEALED_KEY.to_string())
}

/// The fingerprint of my "compared" mark for the person a sent invite to `email` names, if any:
/// core refuses to seal to a different key (`peer_key_changed`).
pub(super) fn expected_fingerprint(sent: &Value, own: &str, email: &str) -> Option<[u8; 32]> {
    let email = email.trim().to_lowercase();
    let db = state_db().ok()?;
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
        .find_map(|peer| db.share_mark(own, &peer).ok().flatten())
        .and_then(|(fp, _)| key_hex(&fp))
}

/// What an invite answers, and the key record it leaves: the sealed key for a share, the offered
/// key for a refused one (DW-0515), so "Share anyway" seals to exactly the key the user was shown.
pub(super) fn record_outcome(own: &str, outcome: &InviteOutcome) -> Answer<Value> {
    let db = state_db()?;
    match outcome {
        InviteOutcome::Shared {
            invite_id,
            recipient_public_key,
        } => {
            db.set_share_sealed_key(own, invite_id, &hex(recipient_public_key))
                .map_err(|_| "failed".to_string())?;
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

/// DW-0515 "Share anyway" for a file: seal to the key the user was shown, with no expected
/// fingerprint, then forget the mark (the person reads as not compared again).
pub(super) async fn share_anyway(u: &Unlocked, item: &str, email: &str, key: &FileKey) -> Answer<Value> {
    let own = own_id(u).await?;
    let sent = fetch(u, SENT).await?;
    let invite = refused_invite(&sent, item, email)?;
    let db = state_db()?;
    let offered = db
        .share_offered_key(&own, &invite)
        .map_err(|_| "failed".to_string())?
        .as_deref()
        .and_then(key_hex)
        .ok_or_else(|| "not-found".to_string())?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(offered);
    let sealed = member::approve_request(&invite, item, &b64, &u.master, key, None).map_err(flow_reason)?;
    let resp = super::flows::send(&crate::auth_v2::transport::client()?, u, sealed.request).await?;
    if !resp.ok() {
        return Err("failed".into());
    }
    db.set_share_sealed_key(&own, &invite, &hex(&sealed.recipient_public_key))
        .map_err(|_| "failed".to_string())?;
    if let Ok(peer) = peer_user_id(&sent, &invite) {
        db.clear_share_mark(&own, &peer).map_err(|_| "failed".to_string())?;
    }
    Ok(json!({ "status": "shared" }))
}

/// Everything a number or a mark needs for invite `member_id`: from the key it was sealed to.
async fn resolve(u: &Unlocked, member_id: &str) -> Answer<(String, String, Vec<String>, String)> {
    let member_id = item_id(member_id)?;
    let peer = peer_user_id(&fetch(u, SENT).await?, &member_id)?;
    let own = own_id(u).await?;
    let key = sealed_key(&own, &member_id)?;
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

/// They match / They don't match: a local mark against the peer's current key.
#[tauri::command]
pub(crate) async fn share_mark_compared(state: State<'_, AppState>, member_id: String, matched: bool) -> Answer<()> {
    let u = unlocked(&state)?;
    let (own, peer, _, fp) = resolve(&u, &member_id).await?;
    state_db()?
        .set_share_mark(&own, &peer, &fp, matched)
        .map_err(|_| "failed".to_string())
}

/// DW-0515 "Share anyway": accept the new key; the person reads as not compared again.
#[tauri::command]
pub(crate) async fn share_forget_compared(state: State<'_, AppState>, member_id: String) -> Answer<()> {
    let u = unlocked(&state)?;
    let peer = peer_user_id(&fetch(&u, SENT).await?, &item_id(&member_id)?)?;
    let own = own_id(&u).await?;
    state_db()?
        .clear_share_mark(&own, &peer)
        .map_err(|_| "failed".to_string())
}

/// Adds `compared` / `mismatch` / `keyChanged` to People rows that have a mark. A key is fetched
/// only for a person with a mark; any failure leaves the row as it was.
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
        let Ok(Some(mark)) = db.share_mark(own, &peer) else {
            continue;
        };
        let Some(member) = invite.get("id").and_then(Value::as_str).and_then(|m| item_id(m).ok()) else {
            continue;
        };
        let Ok(key) = sealed_key(own, &member) else { continue };
        let Ok(fp) = safety::key_fingerprint(&key).map(|f| hex(&f)) else {
            continue;
        };
        if let (Some(obj), Value::Object(extra)) = (row.as_object_mut(), trust_fields(trust(Some(mark), &fp))) {
            obj.extend(extra);
        }
    }
}
