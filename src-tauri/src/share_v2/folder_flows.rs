//! B05c: the folder share flows and bundles, driven over `auth_v2::transport` like `flows.rs`.
//!
//! - `share_invite` on a folder runs core's `FolderShare::invite` (task 2133 key rules). The
//!   `shared_folders_v1` flag is read at send time; a failed read aborts (web
//!   `sharedFoldersEnabledForWrite`), never "off". The device's custody memory (L7d, the state DB)
//!   goes in, and every id the flow asks to remember is written BEFORE its request is sent.
//! - `share_link_copy` on a folder rebuilds the owner's live folder link, else runs
//!   `FolderShare::link`; on several items it makes one bundle link.
use beebeeb_core::auth_flow::HttpRequest;
use beebeeb_core::share_flow::{
    ShareFlowError,
    bundle::BundleItem,
    folder::{
        FolderInviteInput, FolderLinkInput, FolderSend, FolderShare, FolderStep, list_folder_links_request,
        rebuild_folder_link, revoke_folder_link_request,
    },
    folder_keys::CustodyMemory,
    link::{CreateLink, LinkOptions},
    member::PeerKeyExpectation,
};
use serde_json::Value;
use zeroize::Zeroizing;

use super::Answer;
use super::flows::{Unlocked, drive_link, file_key, flow_reason, get, item_id, list, send, state_db};

const FLAGS_PATH: &str = "/api/v1/feature-flags/evaluate";
const SHARED_FOLDERS_FLAG: &str = "shared_folders_v1";

/// The device's custody memory from the state DB. A read failure is not "never seen": it aborts.
fn custody_memory() -> Answer<CustodyMemory> {
    let ids = state_db()?.custody_seen_ids().map_err(|_| "failed".to_string())?;
    Ok(CustodyMemory::new(ids, false))
}

/// Web `sharedFoldersEnabledForWrite`: the flag, or an error. Never "off" for a failed read.
async fn shared_folders_write(client: &reqwest::Client, u: &Unlocked) -> Answer<bool> {
    let resp = send(client, u, get(FLAGS_PATH.into())).await?;
    if !resp.ok() {
        return Err("flags-unavailable".into());
    }
    let flags = resp
        .body
        .get("flags")
        .and_then(Value::as_object)
        .ok_or_else(|| "flags-unavailable".to_string())?;
    Ok(flags.get(SHARED_FOLDERS_FLAG).and_then(Value::as_bool) == Some(true))
}

/// Send one folder-flow request, remembering a custody id first when the flow asks.
async fn send_folder(
    client: &reqwest::Client,
    u: &Unlocked,
    s: FolderSend,
) -> Answer<beebeeb_core::auth_flow::HttpResponse> {
    if let Some(id) = s.mark_custody_seen {
        state_db()?
            .remember_folder_custody(&id)
            .map_err(|_| "failed".to_string())?;
    }
    send(client, u, s.request).await
}

async fn drive_folder(
    client: &reqwest::Client,
    u: &Unlocked,
    mut flow: FolderShare,
    first: FolderStep,
) -> Answer<FolderStep> {
    let mut step = first;
    loop {
        match step {
            FolderStep::Send(s) => {
                let resp = send_folder(client, u, s).await?;
                step = flow.on_response(resp).map_err(flow_reason)?;
            }
            done => return Ok(done),
        }
    }
}

pub(super) async fn folder_invite(
    u: &Unlocked,
    own: &str,
    id: &str,
    email: &str,
    peer_key: PeerKeyExpectation,
) -> Answer<Value> {
    let client = crate::auth_v2::transport::client()?;
    let input = FolderInviteInput {
        folder_id: id,
        email,
        shared_folders_write: shared_folders_write(&client, u).await?,
        custody: custody_memory()?,
        peer_key,
    };
    let (flow, first) = FolderShare::invite(input, &u.master).map_err(flow_reason)?;
    match drive_folder(&client, u, flow, first).await? {
        FolderStep::Done(outcome) => super::safety::record_outcome(own, &outcome),
        _ => Err(flow_reason(ShareFlowError::WrongState)),
    }
}

pub(super) async fn folder_link(
    client: &reqwest::Client,
    u: &Unlocked,
    app_url: &str,
    id: &str,
    chosen: bool,
    options: &LinkOptions,
) -> Answer<Zeroizing<String>> {
    if !chosen {
        let resp = send(client, u, list_folder_links_request(id).map_err(flow_reason)?).await?;
        if !resp.ok() {
            return Err(flow_reason(ShareFlowError::from_status(&resp)));
        }
        let rebuilt = list(&resp.body, "links").find_map(|l| {
            let s = |k: &str| l.get(k).and_then(Value::as_str);
            if l.get("revoked").and_then(Value::as_bool) == Some(true) {
                return None;
            }
            rebuild_folder_link(
                app_url,
                &u.master,
                s("id")?,
                s("owner_wrapped_key"),
                s("owner_wrapped_token"),
                s("wrapped_folder_key"),
                l.get("passphrase_protected").and_then(Value::as_bool).unwrap_or(false),
            )
        });
        if let Some(link) = rebuilt {
            return Ok(link);
        }
    }
    let input = FolderLinkInput {
        folder_id: id,
        app_url,
        access_level: "view",
        label: None,
        expires_in_hours: options.expires_in_hours,
        cap_bytes: None,
        max_opens: options.max_opens,
        passphrase: options.passphrase.clone(),
        custody: custody_memory()?,
    };
    let (flow, first) = FolderShare::link(input, &u.master).map_err(flow_reason)?;
    match drive_folder(client, u, flow, first).await? {
        FolderStep::LinkCreated(c) => Ok(c.link),
        _ => Err(flow_reason(ShareFlowError::WrongState)),
    }
}

/// One link over several files (web's bundle branch). A folder among them is refused: a folder
/// snapshot is its own flow.
pub(super) async fn bundle_link(
    client: &reqwest::Client,
    u: &Unlocked,
    app_url: &str,
    ids: &[String],
    options: &LinkOptions,
) -> Answer<Zeroizing<String>> {
    let ids = ids.iter().map(|i| item_id(i)).collect::<Answer<Vec<_>>>()?;
    let keys = ids.iter().map(|i| file_key(&u.master, i)).collect::<Answer<Vec<_>>>()?;
    let items: Vec<BundleItem<'_>> = ids
        .iter()
        .zip(&keys)
        .enumerate()
        .map(|(n, (id, key))| BundleItem {
            file_id: id,
            position: n as u32,
            file_key: key,
        })
        .collect();
    let (flow, req): (CreateLink, HttpRequest) =
        CreateLink::bundle(app_url, &items, &u.master, options).map_err(flow_reason)?;
    drop(items);
    drop(keys);
    drive_link(client, u, flow, req).await
}

/// The revoke requests for every live folder link of `id` ("Stop sharing" on a folder).
pub(super) async fn folder_link_revokes(client: &reqwest::Client, u: &Unlocked, id: &str) -> Answer<Vec<HttpRequest>> {
    let resp = send(client, u, list_folder_links_request(id).map_err(flow_reason)?).await?;
    if !resp.ok() {
        return Err(flow_reason(ShareFlowError::from_status(&resp)));
    }
    list(&resp.body, "links")
        .filter(|l| l.get("revoked").and_then(Value::as_bool) != Some(true))
        .map(|l| {
            revoke_folder_link_request(l.get("id").and_then(Value::as_str).unwrap_or_default()).map_err(flow_reason)
        })
        .collect()
}
