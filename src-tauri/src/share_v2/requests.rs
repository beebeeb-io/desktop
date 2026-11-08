//! B05c: file requests through core's `share_flow::request` (web `pages/file-request.tsx`).
//! The link carries the request's public key after `#`: Rust puts it on the clipboard, and the
//! web view gets the display form. Changing a request's expiry and removing one have no server
//! route (web closes a request, nothing else): they stay `unsupported`.
use beebeeb_core::share_flow::request::{CreateRequest, RequestOptions, close_request, rebuild_request_link};
use beebeeb_core::share_flow::{ShareFlowError, request::FILE_REQUESTS_PATH};
use serde_json::Value;
use tauri::State;
use tauri_plugin_clipboard_manager::ClipboardExt;
use zeroize::Zeroizing;

use super::flows::{flow_reason, get, list, send, state_db, unlocked};
use super::{Answer, CopiedLink, FileRequestInput, NOT_YET, display_form};
use crate::AppState;
use crate::auth_v2::transport;

/// The folder a request delivers into: `Files` is the root (no target); another name is the
/// top-level folder of that name in the state DB.
fn target_folder(name: &str) -> Answer<Option<String>> {
    let name = name.trim();
    if name.is_empty() || name == "Files" {
        return Ok(None);
    }
    let entry = state_db()?
        .get_file_by_path(name)
        .map_err(|_| "failed".to_string())?
        .filter(|e| e.is_dir())
        .ok_or_else(|| "not-found".to_string())?;
    Ok(Some(entry.file_id))
}

fn copy(app: &tauri::AppHandle, link: &Zeroizing<String>) -> Answer<CopiedLink> {
    app.clipboard()
        .write_text(link.as_str())
        .map_err(|_| "clipboard".to_string())?;
    Ok(CopiedLink {
        display: display_form(link),
    })
}

#[tauri::command]
pub(crate) async fn file_request_create(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    input: FileRequestInput,
) -> Answer<CopiedLink> {
    let u = unlocked(&state)?;
    let options = RequestOptions {
        title: input.title,
        description: Some(input.note),
        target_folder_id: target_folder(&input.folder_name)?,
        max_files: input.max_files,
        max_total_bytes: input.max_bytes,
        expires_in_secs: input.expires_days.map(|d| u64::from(d) * 86_400),
    };
    let (mut flow, req) =
        CreateRequest::start(crate::native_menu::WEB_APP_URL, &options, &u.master).map_err(flow_reason)?;
    let client = transport::client()?;
    let resp = send(&client, &u, req).await?;
    let created = flow.on_response(resp).map_err(flow_reason)?;
    copy(&app, &created.link)
}

#[tauri::command]
pub(crate) async fn file_request_copy(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> Answer<CopiedLink> {
    let u = unlocked(&state)?;
    let client = transport::client()?;
    let resp = send(&client, &u, get(FILE_REQUESTS_PATH.into())).await?;
    if !resp.ok() {
        return Err(flow_reason(ShareFlowError::from_status(&resp)));
    }
    let row = list(&resp.body, "file_requests")
        .find(|r| r.get("id").and_then(Value::as_str) == Some(id.as_str()))
        .ok_or_else(|| "not-found".to_string())?;
    let s = |k: &str| row.get(k).and_then(Value::as_str);
    let link = rebuild_request_link(
        crate::native_menu::WEB_APP_URL,
        &u.master,
        s("token"),
        s("wrapped_private_key"),
        s("wrap_nonce"),
    )
    .ok_or_else(|| "key-unavailable".to_string())?;
    copy(&app, &link)
}

/// Close a request (no new uploads; received files stay). An expiry change has no server route.
#[tauri::command]
#[allow(unused_variables)]
pub(crate) async fn file_request_update(
    state: State<'_, AppState>,
    id: String,
    expires_days: Option<u32>,
    set_expiry: bool,
    close: bool,
) -> Answer<()> {
    let u = unlocked(&state)?;
    if set_expiry || !close {
        return Err(NOT_YET.into());
    }
    let client = transport::client()?;
    let resp = send(&client, &u, close_request(&id).map_err(flow_reason)?).await?;
    if !resp.ok() {
        return Err(flow_reason(ShareFlowError::from_status(&resp)));
    }
    Ok(())
}
