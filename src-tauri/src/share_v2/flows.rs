//! B05b: the Shared commands that run core's `share_flow` (person share of a file, the
//! double-encrypted link, revoke). Rust resolves the key that opens the row, drives the flow over
//! `auth_v2::transport`, and puts a key-carrying link on the clipboard itself: the web view gets
//! the display form only.
use beebeeb_core::auth_flow::{HttpRequest, HttpResponse, Method, RequestAuth};
use beebeeb_core::kdf::{FileKey, MasterKey};
use beebeeb_core::share_flow::{
    self as sf, ShareFlowError,
    link::{CreateLink, LinkOptions, LinkStep},
    member::{InviteStep, MemberInvite, ShareTarget},
};
use serde_json::Value;
use tauri::State;
use tauri_plugin_clipboard_manager::ClipboardExt;
use zeroize::Zeroizing;

use super::{Answer, CopiedLink, display_form};
use crate::AppState;
use crate::auth_v2::transport;

/// The unlocked session: the bearer token and the master key, both wiped on drop.
pub(super) struct Unlocked {
    session: Zeroizing<String>,
    pub(super) master: MasterKey,
}

pub(super) fn unlocked(state: &AppState) -> Answer<Unlocked> {
    let acct = state.active_account().map_err(|_| "signed-out".to_string())?;
    let guard = acct.session.lock().map_err(|_| "failed".to_string())?;
    let session = guard.as_ref().ok_or_else(|| "locked".to_string())?;
    Ok(Unlocked {
        session: Zeroizing::new(session.token.clone()),
        master: MasterKey::from_bytes(session.master_key),
    })
}

/// A flow error as the window's reason: `unsupported` for what core does not have yet.
pub(super) fn flow_reason(e: ShareFlowError) -> String {
    match e {
        ShareFlowError::NotFound => "not-found".into(),
        ShareFlowError::Unauthorized => "signed-out".into(),
        ShareFlowError::InvalidInput(_) => "invalid".into(),
        other => other.code().into(),
    }
}

/// A server id the window sent: a canonical UUID, or nothing is sent.
pub(super) fn item_id(id: &str) -> Answer<String> {
    match id.parse::<uuid::Uuid>() {
        Ok(u) if u.hyphenated().to_string() == id.to_ascii_lowercase() => Ok(u.hyphenated().to_string()),
        _ => Err("not-found".into()),
    }
}

/// The key that opens a file, from the state DB (web's `shareFileKey`; the same rule as
/// `EngineBridge::resolved_owned_key`): its owned-key record, else an owner id's derivation; a
/// foreign id with no record is unavailable. A folder answers `unsupported` before any key lookup:
/// folder shares and folder links are not in core yet.
pub(super) fn state_db() -> Answer<crate::state_db::StateDb> {
    let cfg = crate::config::DesktopConfig::load().map_err(|_| "failed".to_string())?;
    crate::local_state_purge::state_db_for_config(&cfg)
        .map_err(|_| "failed".to_string())?
        .ok_or_else(|| "not-found".to_string())
}

/// Whether a live item is a folder (`not-found` for an unknown or trashing one).
pub(super) fn is_folder(id: &str) -> Answer<bool> {
    let entry = state_db()?
        .get_file(id)
        .map_err(|_| "failed".to_string())?
        .filter(|e| e.status != crate::state_db::FileStatus::Trashing)
        .ok_or_else(|| "not-found".to_string())?;
    Ok(entry.is_dir())
}

pub(super) fn file_key(master: &MasterKey, id: &str) -> Answer<FileKey> {
    use beebeeb_core::item_id::{ItemClass, item_class};
    let db = state_db()?;
    let entry = db
        .get_file(id)
        .map_err(|_| "failed".to_string())?
        .filter(|e| e.status != crate::state_db::FileStatus::Trashing)
        .ok_or_else(|| "not-found".to_string())?;
    if entry.is_dir() {
        // A folder has no file key: folders go through `folder_flows` (core's `FolderShare`).
        return Err("invalid".into());
    }
    fn unavailable<E>(_: E) -> String {
        "key-unavailable".into()
    }
    if let Some(record) = db.get_owned_key_record(id).map_err(unavailable)? {
        let kind = crate::owner_keys::kind_in_db(&db, id);
        return crate::owner_keys::record_key(master, &record, kind)
            .map(|(key, _)| key)
            .map_err(unavailable);
    }
    match item_class(id) {
        Ok(ItemClass::Owner) => Ok(beebeeb_core::kdf::derive_file_key(master, id.as_bytes())),
        _ => Err("key-unavailable".into()),
    }
}

pub(super) async fn send(client: &reqwest::Client, u: &Unlocked, req: HttpRequest) -> Answer<HttpResponse> {
    transport::send(client, Some(&u.session), req).await
}

pub(super) fn get(path: String) -> HttpRequest {
    HttpRequest {
        method: Method::Get,
        path,
        body: None,
        auth: RequestAuth::Session,
        confirm_token: None,
    }
}

pub(super) fn list<'a>(body: &'a Value, field: &str) -> impl Iterator<Item = &'a Value> {
    body.get(field).and_then(Value::as_array).into_iter().flatten()
}

/// A link of the owner's that is still live: not revoked, a file share, and not past its expiry.
pub(super) fn live_link(share: &Value, now: chrono::DateTime<chrono::Utc>) -> bool {
    let expired = share
        .get("expires_at")
        .and_then(Value::as_str)
        .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
        .is_some_and(|t| t <= now);
    share.get("revoked").and_then(Value::as_bool) != Some(true)
        && share.get("share_type").and_then(Value::as_str).unwrap_or("file") == "file"
        && !expired
}

async fn my_links(client: &reqwest::Client, u: &Unlocked, id: &str) -> Answer<Vec<Value>> {
    let resp = send(client, u, get(format!("/api/v1/shares/mine?file_id={id}"))).await?;
    if !resp.ok() {
        return Err(flow_reason(ShareFlowError::from_status(&resp)));
    }
    let now = chrono::Utc::now();
    Ok(list(&resp.body, "shares")
        .filter(|s| live_link(s, now))
        .cloned()
        .collect())
}

/// Share one file with a person. A folder answers `unsupported` (core has no folder invite yet);
/// `can_edit` is not sent: web's invite has no permission field, so neither does this one.
const SENT: &str = "/api/v1/shares/invites/sent";
const FOLDER_SHARE_ANYWAY: &str =
    "unsupported: sharing a folder to a changed key needs a folder approval core does not offer yet";

#[tauri::command]
#[allow(unused_variables)]
pub(crate) async fn share_invite(
    state: State<'_, AppState>,
    item_id: String,
    email: String,
    can_edit: bool,
    accept_changed_key: bool,
) -> Answer<Value> {
    let id = self::item_id(&item_id)?;
    let u = unlocked(&state)?;
    let own = super::safety::own_id(&u).await?;
    // A "compared" mark for this person: core refuses to seal to any other key (DW-0515).
    let expected = super::safety::expected_fingerprint(&super::reads::fetch(&u, SENT).await?, &own, &email);
    if is_folder(&id)? {
        if accept_changed_key {
            return Err(FOLDER_SHARE_ANYWAY.into());
        }
        return super::folder_flows::folder_invite(&u, &own, &id, &email, expected).await;
    }
    let key = file_key(&u.master, &id)?;
    if accept_changed_key {
        return super::safety::share_anyway(&u, &id, &email, &key).await;
    }
    let target = ShareTarget {
        item_id: id,
        is_folder: false,
    };
    let (mut flow, mut req) = MemberInvite::start(&target, &email, &u.master, &key, expected).map_err(flow_reason)?;
    drop(key);
    let client = transport::client()?;
    loop {
        let resp = send(&client, &u, req).await?;
        match flow.on_response(resp).map_err(flow_reason)? {
            InviteStep::Send(next) => req = next,
            InviteStep::Done(outcome) => return super::safety::record_outcome(&own, &outcome),
        }
    }
}

/// Copy an item's link: the owner's live link rebuilt from its owner blobs, else a new
/// double-encrypted link (no expiry, unlimited opens). Rust writes the full link to the clipboard;
/// the answer is the display form only.
#[tauri::command]
pub(crate) async fn share_link_copy(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    item_id: String,
    item_ids: Option<Vec<String>>,
    expires_hours: Option<u32>,
    password: Option<String>,
) -> Answer<CopiedLink> {
    let password = password.map(Zeroizing::new);
    let id = self::item_id(&item_id)?;
    let u = unlocked(&state)?;
    let client = transport::client()?;
    let app_url = crate::native_menu::WEB_APP_URL;
    // The Files dialog's choices (DW-0407): a new link with them. Without any (Shared's
    // "Copy link"), the owner's live link is rebuilt first.
    let options = LinkOptions {
        expires_in_hours: expires_hours,
        max_opens: None,
        passphrase: password.clone().filter(|p| !p.trim().is_empty()),
    };
    let chosen = options.expires_in_hours.is_some() || options.passphrase.is_some();
    let link = match item_ids.filter(|ids| ids.len() > 1) {
        Some(ids) => super::folder_flows::bundle_link(&client, &u, app_url, &ids, &options).await?,
        None if is_folder(&id)? => {
            super::folder_flows::folder_link(&client, &u, app_url, &id, chosen, &options).await?
        }
        None => file_link(&client, &u, app_url, &id, chosen, &options).await?,
    };
    app.clipboard()
        .write_text(link.as_str())
        .map_err(|_| "clipboard".to_string())?;
    Ok(CopiedLink {
        display: display_form(&link),
    })
}

async fn file_link(
    client: &reqwest::Client,
    u: &Unlocked,
    app_url: &str,
    id: &str,
    chosen: bool,
    options: &LinkOptions,
) -> Answer<Zeroizing<String>> {
    let id = id.to_owned();
    let existing = if chosen {
        Vec::new()
    } else {
        my_links(client, u, &id).await?
    };
    let existing = existing.into_iter().find_map(|s| {
        sf::link::rebuild_link(
            app_url,
            &u.master,
            s.get("owner_wrapped_key").and_then(Value::as_str),
            s.get("owner_wrapped_token").and_then(Value::as_str),
        )
    });
    let link = match existing {
        Some(link) => link,
        None => {
            let key = file_key(&u.master, &id)?;
            let (flow, req) = CreateLink::start(app_url, &id, &u.master, &key, options).map_err(flow_reason)?;
            drop(key);
            drive_link(&client, u, flow, req).await?
        }
    };
    Ok(link)
}

/// Drive a file or bundle link to its full URL.
pub(super) async fn drive_link(
    client: &reqwest::Client,
    u: &Unlocked,
    mut flow: CreateLink,
    mut req: HttpRequest,
) -> Answer<Zeroizing<String>> {
    loop {
        let resp = send(client, u, req).await?;
        match flow.on_response(resp).map_err(flow_reason)? {
            LinkStep::Send(next) => req = next,
            LinkStep::Created(created) => return Ok(created.link),
        }
    }
}

/// Stop sharing: one person (`member_id`, the invite id: the server's `DELETE /shares/invites/{id}`
/// takes the invite id) or, with none, every live link of the item (a folder's folder links too)
/// and every invite the owner sent for it.
#[tauri::command]
pub(crate) async fn share_revoke(state: State<'_, AppState>, item_id: String, member_id: Option<String>) -> Answer<()> {
    let id = self::item_id(&item_id)?;
    let u = unlocked(&state)?;
    let client = transport::client()?;
    let mut requests = Vec::new();
    match member_id {
        Some(member) => requests.push(sf::cancel_invite_request(&self::item_id(&member)?).map_err(flow_reason)?),
        None => {
            if is_folder(&id)? {
                requests.extend(super::folder_flows::folder_link_revokes(&client, &u, &id).await?);
            }
            for share in my_links(&client, &u, &id).await? {
                let sid = share.get("id").and_then(Value::as_str).unwrap_or_default();
                requests.push(sf::revoke_link_request(sid).map_err(flow_reason)?);
            }
            let sent = send(&client, &u, get("/api/v1/shares/invites/sent".into())).await?;
            if !sent.ok() {
                return Err(flow_reason(ShareFlowError::from_status(&sent)));
            }
            for invite in list(&sent.body, "invites").filter(|i| i.get("file_id").and_then(Value::as_str) == Some(&id))
            {
                let iid = invite.get("id").and_then(Value::as_str).unwrap_or_default();
                requests.push(sf::cancel_invite_request(iid).map_err(flow_reason)?);
            }
        }
    }
    for req in requests {
        let resp = send(&client, &u, req).await?;
        if !resp.ok() && resp.status != 404 {
            return Err(flow_reason(ShareFlowError::from_status(&resp)));
        }
    }
    Ok(())
}
