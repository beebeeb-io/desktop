//! B05e: the share safety number and the local "compared" mark (contract: workspace
//! `.claude/tasks/backlog/v2-b05e-safety-number-contract-ruled.md`).
//!
//! The number comes from core (`share_flow::safety`); only its 12 groups of 5 digits go to the web
//! view. The own key is derived from the master key here, never fetched; the peer key comes from
//! `GET /auth/public-key/{peer}`, the key the member seal uses. A mark is kept in the state DB per
//! (account id, peer user id) with SHA-256 of the peer key it was made against; a different key
//! now reads as "key changed" (DW-0515).
use base64::Engine;
use beebeeb_core::auth_flow::HttpRequest;
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
    own: &str,
    own_key: &[u8; 32],
    peer: &str,
    peer_key: &[u8; 32],
) -> Answer<(Vec<String>, String)> {
    let n = safety::safety_number(own, own_key, peer, peer_key).map_err(flow_reason)?;
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

/// A `GET /auth/public-key/{peer}` body: the 32 raw X25519 bytes, for that peer only.
pub(super) fn public_key_of(body: &Value, peer: &str) -> Answer<[u8; 32]> {
    let failed = || "failed".to_string();
    let user = body.get("user_id").and_then(Value::as_str).ok_or_else(failed)?;
    if item_id(user).ok().as_deref() != Some(peer) {
        return Err(failed());
    }
    let b64 = body.get("public_key").and_then(Value::as_str).ok_or_else(failed)?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|_| failed())?;
    bytes.try_into().map_err(|_| failed())
}

/// `share_set_access` through core: `PATCH /shares/invites/{id}`.
pub(super) fn set_access_request(member_id: &str, can_edit: bool) -> Answer<HttpRequest> {
    access::set_member_access_request(&item_id(member_id)?, can_edit).map_err(|_| "not-found".to_string())
}

/// My account id (`/auth/me`) and my sharing public key, derived from the master key here.
pub(super) async fn own_identity(u: &Unlocked) -> Answer<(String, [u8; 32])> {
    let me = fetch(u, ME).await?;
    let id = item_id(me.get("user_id").and_then(Value::as_str).ok_or("failed")?)?;
    let private = beebeeb_core::opaque::derive_x25519_private(&u.master);
    Ok((id, beebeeb_core::opaque::derive_x25519_public(&private)))
}

pub(super) async fn peer_key(u: &Unlocked, peer: &str) -> Answer<[u8; 32]> {
    public_key_of(&fetch(u, &format!("/api/v1/auth/public-key/{peer}")).await?, peer)
}

/// Everything a number or a mark needs for invite `member_id`.
async fn resolve(u: &Unlocked, member_id: &str) -> Answer<(String, String, Vec<String>, String)> {
    let peer = peer_user_id(&fetch(u, SENT).await?, &item_id(member_id)?)?;
    let (own, own_key) = own_identity(u).await?;
    let (groups, fp) = safety_view(&own, &own_key, &peer, &peer_key(u, &peer).await?)?;
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
    let (own, _) = own_identity(&u).await?;
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
            match own_identity(u).await {
                Ok((id, _)) => own = Some(id),
                Err(_) => return,
            }
        }
        let Some(own) = own.as_deref() else { return };
        let Ok(Some(mark)) = db.share_mark(own, &peer) else {
            continue;
        };
        let Ok(key) = peer_key(u, &peer).await else { continue };
        let fp = hex(&<sha2::Sha256 as sha2::Digest>::digest(key));
        if let (Some(obj), Value::Object(extra)) = (row.as_object_mut(), trust_fields(trust(Some(mark), &fp))) {
            obj.extend(extra);
        }
    }
}
