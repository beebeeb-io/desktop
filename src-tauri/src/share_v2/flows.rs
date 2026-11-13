//! B05b: the Shared commands that run core's `share_flow` (person share of a file, the
//! double-encrypted link, revoke). Rust resolves the key that opens the row, drives the flow over
//! `auth_v2::transport`, and puts a key-carrying link on the clipboard itself: the web view gets
//! the display form only.
use beebeeb_core::auth_flow::{HttpRequest, HttpResponse, Method};
use beebeeb_core::kdf::{FileKey, MasterKey};
use beebeeb_core::share_flow::{
    self as sf, ShareFlowError,
    link::{CreateLink, LinkOptions, LinkStep},
    member::{InviteOutcome, InviteStep, MemberInvite, ShareTarget},
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
    /// My account id from the profile cached at sign-in (`/auth/me`), when there is one.
    pub(super) own_cached: Option<String>,
}

pub(super) fn unlocked(state: &AppState) -> Answer<Unlocked> {
    let acct = state.active_account().map_err(|_| "signed-out".to_string())?;
    let own_cached = acct
        .cached_profile
        .lock()
        .ok()
        .and_then(|p| p.as_ref().map(|p| p.user_id.clone()));
    let guard = acct.session.lock().map_err(|_| "failed".to_string())?;
    let session = guard.as_ref().ok_or_else(|| "locked".to_string())?;
    Ok(Unlocked {
        session: Zeroizing::new(session.token.clone()),
        master: MasterKey::from_bytes(session.master_key),
        own_cached,
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

/// A server id the window sent: already a canonical lower-case hyphenated UUID, or nothing is sent.
/// Never normalized (core v0.2.8 items 10 and 13): an id is HKDF info for the file key and the
/// match key of the server's invite rows, so another spelling of the same UUID would derive another
/// key. Core refuses it too; this refuses it before any request or key lookup.
pub(super) fn item_id(id: &str) -> Answer<String> {
    match uuid::Uuid::parse_str(id) {
        Ok(u) if u.hyphenated().to_string() == id => Ok(id.to_owned()),
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

/// A request on the signed-in session with no confirm token. Core v0.2.8 keeps the token field
/// private, so a request starts from `HttpRequest::session_post` and only a step-up inside core can
/// put a token on it.
pub(super) fn request(method: Method, path: String, body: Option<Value>) -> HttpRequest {
    let mut req = HttpRequest::session_post(path, Value::Null);
    req.method = method;
    req.body = body;
    req
}

pub(super) fn get(path: String) -> HttpRequest {
    request(Method::Get, path, None)
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

const FOLDER_SHARE_ANYWAY: &str =
    "unsupported: sharing a folder to a changed key needs a folder approval core does not offer yet";

/// Share a file or a folder (`folder_flows`) with a person; `accept_changed_key` is DW-0515's
/// "Share anyway" for the refused invite `member_id` (the id the `key-changed` answer named, so the
/// key sealed to is the one that dialog was about). `can_edit` is not sent: web's invite has no
/// permission field, so neither does this one.
#[tauri::command]
#[allow(unused_variables)]
pub(crate) async fn share_invite(
    state: State<'_, AppState>,
    item_id: String,
    email: String,
    can_edit: bool,
    accept_changed_key: bool,
    member_id: Option<String>,
) -> Answer<Value> {
    let id = self::item_id(&item_id)?;
    let u = unlocked(&state)?;
    let own = super::safety::own_id(&u).await?;
    let sent = super::reads::fetch(&u, super::reads::SENT).await?;
    let folder = is_folder(&id)?;
    if accept_changed_key {
        let member = self::item_id(member_id.as_deref().unwrap_or_default())?;
        if folder {
            // The refused folder invite holds no key and core cannot seal a folder to a changed key
            // yet: it is withdrawn, never left waiting as an invitation that cannot open.
            let withdrawal = super::safety::folder_anyway_withdrawal(&sent, &member, &id, &email)?;
            super::safety::send_withdrawal(&u, withdrawal).await?;
            return Err(FOLDER_SHARE_ANYWAY.into());
        }
        let key = file_key(&u.master, &id)?;
        return super::safety::share_anyway(&u, &own, &sent, &member, &id, &email, &key).await;
    }
    // Core v0.2.8 item 15: every member seal states what this app knows of the person's key. A mark
    // for them is `Compared(its fingerprint)`, and core refuses any other key before a seal byte
    // exists (DW-0515); no mark is `NotCompared`. A mark that cannot be read stops here.
    let expected = super::safety::expectation(&sent, &own, &email)?;
    if folder {
        return super::folder_flows::folder_invite(&u, &own, &id, &email, expected).await;
    }
    let key = file_key(&u.master, &id)?;
    let target = ShareTarget {
        item_id: id,
        is_folder: false,
    };
    let (mut flow, mut req): (MemberInvite, HttpRequest) =
        MemberInvite::start(&target, &email, &u.master, &key, expected).map_err(flow_reason)?;
    drop(key);
    let client = transport::client()?;
    loop {
        let resp = match send(&client, &u, req).await {
            Ok(resp) => resp,
            Err(reason) => {
                return match unanswered_invite(&mut flow) {
                    Some(outcome) => super::safety::finish_invite(&u, &own, &outcome).await,
                    None => Err(reason),
                };
            }
        };
        match flow.on_response(resp).map_err(flow_reason)? {
            InviteStep::Send(next) => req = next,
            InviteStep::Done(outcome) => return super::safety::finish_invite(&u, &own, &outcome).await,
        }
    }
}

/// An invite request that got no answer (the network dropped, or the request never left): core
/// ends the flow as if the server had refused it. After the approval was sent, that is
/// `AwaitingApproval`, a claimed invite with no key the other side can open, which
/// [`super::safety::finish_invite`] withdraws. Before it nothing was claimed: `None`, and the
/// transport's reason is answered.
pub(super) fn unanswered_invite(flow: &mut MemberInvite) -> Option<InviteOutcome> {
    match flow.on_response(HttpResponse::new(0, Value::Null)) {
        Ok(InviteStep::Done(outcome @ InviteOutcome::AwaitingApproval { .. })) => Some(outcome),
        _ => None,
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
    let had_live_link = !existing.is_empty();
    let existing = existing.into_iter().find_map(|s| rebuilt_link(app_url, &u.master, &s));
    let link = match existing {
        Some(link) => link,
        // Core v0.2.8 item 9: a live link is rebuilt only when its stored `wrapped_file_key` opens
        // under the rebuilt key. One that cannot be checked is not handed out, and no second public
        // link is minted behind the person's back: Copy link answers honestly instead.
        None if had_live_link => return Err(LINK_NOT_REBUILT.into()),
        None => {
            let key = file_key(&u.master, &id)?;
            let (flow, req) = CreateLink::start(app_url, &id, &u.master, &key, options).map_err(flow_reason)?;
            drop(key);
            drive_link(client, u, flow, req).await?
        }
    };
    Ok(link)
}

/// What Copy link answers for a live link this app cannot rebuild and check: the owner listing
/// (`GET /shares/mine`) does not carry the link's `wrapped_file_key` yet.
pub(super) const LINK_NOT_REBUILT: &str = "unsupported: this link can't be copied again in this version";

/// A live file link of the owner's, rebuilt from its owner blobs and checked against its stored
/// `wrapped_file_key` (core's `rebuild_link`; `None` when the key is absent or does not open).
pub(super) fn rebuilt_link(app_url: &str, master: &MasterKey, share: &Value) -> Option<Zeroizing<String>> {
    sf::link::rebuild_link(
        app_url,
        master,
        share.get("owner_wrapped_key").and_then(Value::as_str),
        share.get("owner_wrapped_token").and_then(Value::as_str),
        share
            .get("wrapped_file_key")
            .and_then(Value::as_str)
            .unwrap_or_default(),
    )
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

/// What one withdrawal's answer says (the rule `safety::send_withdrawal` already keeps: 200 and
/// 404 are answers, everything else is not). Only a no-answer is worth remembering and retrying:
/// a refusal is the server's answer and is reported, never queued.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Withdrawal {
    /// The server answered: gone (2xx), or already gone (404).
    Answered,
    /// No answer (5xx, or a transport error): keep a record and retry on the next refresh.
    Retry,
    /// The server answered with a refusal (any other status): report it, do not queue it.
    Refused,
}

pub(super) fn withdrawal_answer(status: u16) -> Withdrawal {
    match status {
        200..=299 | 404 => Withdrawal::Answered,
        500..=599 => Withdrawal::Retry,
        _ => Withdrawal::Refused,
    }
}

/// The decision for one attempt, transport error included. Shared by the command and the refresh so
/// there is one rule, never a second reading of a status.
pub(super) fn withdrawal_outcome(result: &Answer<HttpResponse>) -> Withdrawal {
    match result {
        Ok(resp) => withdrawal_answer(resp.status),
        Err(_) => Withdrawal::Retry,
    }
}

/// Remember a cancellation the server did not answer, so the next share-list refresh retries it.
/// A record that cannot be written is logged: the person still hears the failure, and a revocation
/// is never reported as gone.
pub(super) fn note_withdrawal_pending(db: Option<&crate::state_db::StateDb>, account: Option<&str>, invite: &str) {
    let written = db
        .zip(account)
        .is_some_and(|(db, account)| db.set_share_withdraw_pending(account, invite).is_ok());
    if !written {
        tracing::warn!("share: the pending withdrawal was not recorded; the invite may still stand");
    }
}

/// The server answered: drop the record. A row that cannot be cleared is logged; the next refresh
/// clears it with the same answer.
pub(super) fn clear_withdrawal_pending(db: Option<&crate::state_db::StateDb>, account: Option<&str>, invite: &str) {
    if db
        .zip(account)
        .is_some_and(|(db, account)| db.clear_share_withdraw_pending(account, invite).is_err())
    {
        tracing::warn!("share: the answered withdrawal record was not cleared");
    }
}

/// Stop sharing: one person (`member_id`, the invite id: the server's `DELETE /shares/invites/{id}`
/// takes the invite id) or, with none, every live link of the item (a folder's folder links too)
/// and every invite the owner sent for it.
///
/// An invite cancellation the server does not answer (transport error or 5xx) is recorded in the
/// state DB and retried on the next `share_list`; the command still answers its honest failure, so
/// a dropped revocation never pretends the invite is gone.
#[tauri::command]
pub(crate) async fn share_revoke(state: State<'_, AppState>, item_id: String, member_id: Option<String>) -> Answer<()> {
    let id = self::item_id(&item_id)?;
    let u = unlocked(&state)?;
    let client = transport::client()?;
    let mut requests: Vec<(Option<String>, HttpRequest)> = Vec::new();
    match member_id {
        Some(member) => {
            let invite = self::item_id(&member)?;
            requests.push((
                Some(invite.clone()),
                sf::cancel_invite_request(&invite).map_err(flow_reason)?,
            ));
        }
        None => {
            if is_folder(&id)? {
                requests.extend(
                    super::folder_flows::folder_link_revokes(&client, &u, &id)
                        .await?
                        .into_iter()
                        .map(|req| (None, req)),
                );
            }
            for share in my_links(&client, &u, &id).await? {
                let sid = share.get("id").and_then(Value::as_str).unwrap_or_default();
                requests.push((None, sf::revoke_link_request(sid).map_err(flow_reason)?));
            }
            let sent = send(&client, &u, get(super::reads::SENT.into())).await?;
            if !sent.ok() {
                return Err(flow_reason(ShareFlowError::from_status(&sent)));
            }
            for invite in list(&sent.body, "invites").filter(|i| i.get("file_id").and_then(Value::as_str) == Some(&id))
            {
                let iid = invite.get("id").and_then(Value::as_str).unwrap_or_default();
                requests.push((
                    Some(iid.to_owned()),
                    sf::cancel_invite_request(iid).map_err(flow_reason)?,
                ));
            }
        }
    }
    // Only an invite cancellation can be retried later; a link revoke has no record to keep.
    let needs_record = requests.iter().any(|(invite, _)| invite.is_some());
    let own = if needs_record {
        super::safety::own_id(&u).await.ok()
    } else {
        None
    };
    let db = if needs_record { state_db().ok() } else { None };
    for (invite, req) in requests {
        let result = send(&client, &u, req).await;
        match withdrawal_outcome(&result) {
            Withdrawal::Answered => {
                if let Some(invite) = invite.as_deref() {
                    clear_withdrawal_pending(db.as_ref(), own.as_deref(), invite);
                }
            }
            Withdrawal::Retry => {
                if let Some(invite) = invite.as_deref() {
                    note_withdrawal_pending(db.as_ref(), own.as_deref(), invite);
                }
                return Err(match &result {
                    Ok(resp) => flow_reason(ShareFlowError::from_status(resp)),
                    Err(e) => e.clone(),
                });
            }
            Withdrawal::Refused => {
                return Err(match &result {
                    Ok(resp) => flow_reason(ShareFlowError::from_status(resp)),
                    Err(_) => "failed".into(),
                });
            }
        }
    }
    Ok(())
}
