//! B05d: the Shared read and member commands (web `pages/shared.tsx`, `api-shares.ts`): the lists,
//! accept / decline / leave, the people of an item and a member's access. No crypto step is built
//! here: an incoming item's name is opened with core's `share_flow::open_member_file_key`, and
//! nothing that carries a key, a token or a link fragment is returned to the web view.
//! Kept `unsupported` (named in the PR): `share_link_update` (the server has no route that changes a
//! link's expiry or passphrase) and `file_request_remove` (no server route). The safety number, the
//! "compared" mark and the access PATCH through core are B05e (`super::safety`).
use base64::Engine as _;
use beebeeb_core::auth_flow::{HttpRequest, Method};
use beebeeb_core::share_flow::{ShareFlowError, request::FILE_REQUESTS_PATH};
use serde_json::{Value, json};
use tauri::State;

use super::flows::{Unlocked, flow_reason, get, item_id, list, send, state_db, unlocked};
use super::{Answer, NOT_YET, ShareLists};
use crate::AppState;
use crate::auth_v2::transport;

const INCOMING: &str = "/api/v1/shares/invites/incoming";
pub(super) const SENT: &str = "/api/v1/shares/invites/sent";
const MINE: &str = "/api/v1/shares/mine";

fn text<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    v.get(k).and_then(Value::as_str)
}

/// A server path with one id the window sent: a canonical UUID, or nothing is sent.
pub(super) fn invite_path(id: &str, tail: &str) -> Answer<String> {
    Ok(format!("/api/v1/shares/invites/{}{tail}", item_id(id)?))
}

pub(super) fn post(path: String) -> HttpRequest {
    super::flows::request(Method::Post, path, None)
}

/// The window's label for the grant the server enforces (`read` | `write`).
fn access_label(permission: &str) -> &'static str {
    if permission == "read" { "Can view" } else { "Can edit" }
}

/// The name of an item the state DB knows: the last path component.
fn local_name(file_id: &str) -> Option<String> {
    let entry = state_db().ok()?.get_file(file_id).ok()??;
    entry.path.rsplit('/').find(|p| !p.is_empty()).map(str::to_owned)
}

/// An incoming invite's name, opened with the member key flow; `Encrypted item` when it cannot be.
fn incoming_name(u: &Unlocked, invite: &Value) -> String {
    let opened = (|| {
        let b64 = base64::engine::general_purpose::STANDARD;
        let sender: [u8; 32] = b64.decode(text(invite, "sender_public_key")?).ok()?.try_into().ok()?;
        let sealed = b64.decode(text(invite, "encrypted_file_key")?).ok()?;
        let key = beebeeb_core::share_flow::open_member_file_key(&u.master, &sender, text(invite, "file_id")?, &sealed)
            .ok()?;
        beebeeb_core::encrypt::decrypt_name_with_key(&key, text(invite, "file_name_encrypted")?)
    })();
    opened.unwrap_or_else(|| "Encrypted item".to_string())
}

/// DW-0501: an invite waiting for an answer (`invited`); a claimed or approved one is not a card.
pub(super) fn invitation_row(invite: &Value, name: String) -> Option<Value> {
    if text(invite, "status") != Some("invited") {
        return None;
    }
    Some(json!({
        "id": text(invite, "id")?,
        "name": name,
        "from": text(invite, "sender_email").unwrap_or_default(),
        "isFolder": invite.get("is_folder").and_then(Value::as_bool).unwrap_or(false),
    }))
}

/// A live link of mine (not revoked). The token never leaves: the row has the name, expiry, a
/// password flag and the size.
pub(super) fn link_row(share: &Value, name: String) -> Option<Value> {
    if share.get("revoked").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    Some(json!({
        "id": text(share, "file_id")?,
        "name": name,
        "expiresAt": share.get("expires_at").cloned().unwrap_or(Value::Null),
        "password": share.get("has_passphrase").and_then(Value::as_bool).unwrap_or(false),
    }))
}

pub(super) fn request_row(r: &Value, into: Option<String>) -> Option<Value> {
    let max = r.get("max_files").and_then(Value::as_u64).filter(|m| *m > 0);
    Some(json!({
        "id": text(r, "id")?,
        "title": text(r, "title")?,
        "files": r.get("files_received").and_then(Value::as_u64).unwrap_or(0),
        "maxFiles": max,
        "into": into,
        "expiresAt": r.get("expires_at").cloned().unwrap_or(Value::Null),
        "closed": r.get("closed").and_then(Value::as_bool).unwrap_or(false),
    }))
}

/// One sent invite of an item as a member. `Member.id` is the invite id.
pub(super) fn member_row(invite: &Value) -> Option<Value> {
    let email = text(invite, "recipient_email")?;
    let status = text(invite, "status")?;
    if !matches!(status, "invited" | "claimed" | "approved") {
        return None;
    }
    Some(json!({
        "id": text(invite, "id")?,
        "name": email,
        "email": email,
        "access": access_label(text(invite, "permission").unwrap_or("read")),
        "invited": status != "approved",
    }))
}

pub(super) async fn fetch(u: &Unlocked, path: &str) -> Answer<Value> {
    let client = transport::client()?;
    let resp = send(&client, u, get(path.to_string())).await?;
    if !resp.ok() {
        return Err(flow_reason(ShareFlowError::from_status(&resp)));
    }
    Ok(resp.body.clone())
}

async fn act(u: &Unlocked, req: HttpRequest) -> Answer<()> {
    let client = transport::client()?;
    let resp = send(&client, u, req).await?;
    if resp.ok() {
        Ok(())
    } else {
        Err(flow_reason(ShareFlowError::from_status(&resp)))
    }
}

#[tauri::command]
pub(crate) async fn share_list(state: State<'_, AppState>) -> Answer<ShareLists> {
    let u = unlocked(&state)?;
    let (incoming, sent, mine, requests) = (
        fetch(&u, INCOMING).await?,
        fetch(&u, SENT).await?,
        fetch(&u, MINE).await?,
        fetch(&u, FILE_REQUESTS_PATH).await?,
    );
    Ok(ShareLists {
        invitations: list(&incoming, "invites")
            .filter_map(|i| invitation_row(i, incoming_name(&u, i)))
            .collect(),
        links: list(&mine, "shares")
            .filter_map(|s| {
                link_row(
                    s,
                    text(s, "file_id")
                        .and_then(local_name)
                        .unwrap_or_else(|| "Shared item".into()),
                )
            })
            .collect(),
        requests: list(&requests, "file_requests")
            .filter_map(|r| request_row(r, text(r, "target_folder_id").and_then(local_name)))
            .collect(),
        pending: list(&sent, "invites")
            .filter(|i| matches!(text(i, "status"), Some("invited" | "claimed")))
            .filter_map(|i| text(i, "file_id").map(str::to_owned))
            .collect(),
    })
}

/// Accept: claim the invite; the owner approves and seals the key.
#[tauri::command]
pub(crate) async fn share_accept(state: State<'_, AppState>, id: String) -> Answer<()> {
    let u = unlocked(&state)?;
    act(&u, post(invite_path(&id, "/claim")?)).await
}

/// Decline: remove the invite from my list (the recipient's route; `deny` is the sender's).
#[tauri::command]
pub(crate) async fn share_decline(state: State<'_, AppState>, id: String) -> Answer<()> {
    let u = unlocked(&state)?;
    act(&u, post(invite_path(&id, "/hide")?)).await
}

/// Leave: `id` is an invite id or the shared item's id; the invite is found in my incoming list.
#[tauri::command]
pub(crate) async fn share_leave(state: State<'_, AppState>, id: String) -> Answer<()> {
    let u = unlocked(&state)?;
    let id = item_id(&id)?;
    let incoming = fetch(&u, INCOMING).await?;
    let invite = list(&incoming, "invites")
        .find(|i| text(i, "id") == Some(id.as_str()) || text(i, "file_id") == Some(id.as_str()))
        .and_then(|i| text(i, "id"))
        .ok_or_else(|| "not-found".to_string())?;
    act(&u, post(invite_path(invite, "/hide")?)).await
}

#[tauri::command]
pub(crate) async fn share_people(state: State<'_, AppState>, item_id: String) -> Answer<Vec<Value>> {
    let u = unlocked(&state)?;
    let item = self::item_id(&item_id)?;
    let sent = fetch(&u, SENT).await?;
    let me = crate::account_commands::last_signed_in_email()
        .ok()
        .flatten()
        .unwrap_or_default();
    let mut members = vec![json!({ "id": "owner", "name": me, "email": me, "you": true, "owner": true })];
    let (invites, mut rows): (Vec<&Value>, Vec<Value>) = list(&sent, "invites")
        .filter(|i| text(i, "file_id") == Some(item.as_str()))
        .filter_map(|i| member_row(i).map(|row| (i, row)))
        .unzip();
    super::safety::annotate_members(&u, &sent, &invites, &mut rows).await;
    members.extend(rows);
    Ok(members)
}

/// `member_id` is the invite id.
#[tauri::command]
pub(crate) async fn share_set_access(
    state: State<'_, AppState>,
    item_id: String,
    member_id: String,
    permission: String,
) -> Answer<()> {
    let u = unlocked(&state)?;
    self::item_id(&item_id)?;
    let req = super::safety::set_access_request(&member_id, &permission)?;
    let resp = send(&transport::client()?, &u, req).await?;
    set_access_reason(resp.status).map_or(Ok(()), |reason| Err(reason.into()))
}

/// What the set-access PATCH's status answers the window (`None` on success). A 403 here is the
/// server refusing this account (frozen, not the owner), not an ended session: the session is still
/// valid, so it is `failed`, while only a 401 means signed-out.
pub(super) fn set_access_reason(status: u16) -> Option<&'static str> {
    Some(match status {
        200..=299 => return None,
        404 => "not-found",
        401 => "signed-out",
        _ => "failed",
    })
}

macro_rules! not_yet {
    ($(fn $name:ident($($arg:ident: $ty:ty),*) -> $ret:ty;)*) => {$(
        #[tauri::command]
        #[allow(unused_variables)]
        pub(crate) async fn $name(state: State<'_, AppState>, $($arg: $ty),*) -> Answer<$ret> {
            unlocked(&state)?;
            Err(NOT_YET.into())
        }
    )*};
}

not_yet! {
    fn share_link_update(item_id: String, expires_days: Option<u32>, set_expiry: bool, password: Option<String>, set_password: bool) -> ();
    fn file_request_remove(id: String) -> ();
}
