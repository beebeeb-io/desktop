//! Approve a sign-in on a new device with the typed code (no camera): list pending links, begin with the code
//! shown by the new device, compare its name, approve (seals the vault key to it) or deny.
//! Crypto: `beebeeb_core::device_link::{TypedCodeApprover, AccountKeyMaterial}`; this file only relays.

use base64::{engine::general_purpose::STANDARD, Engine as _};
use beebeeb_core::device_link::{AccountKeyMaterial, TypedCodeApprover};
use beebeeb_core::kdf::MasterKey;
use beebeeb_core::CoreError;
use std::sync::Mutex;
use tauri::State;

use super::*;

/// The one attempt in flight (id, approver, new device answer). Spent by approve, deny, or a new begin.
static PENDING: Mutex<Option<(String, TypedCodeApprover, Vec<u8>)>> = Mutex::new(None);

const ANSWER_WAIT: std::time::Duration = std::time::Duration::from_secs(45);

fn take_pending() -> Option<(String, TypedCodeApprover, Vec<u8>)> {
    PENDING.lock().ok().and_then(|mut g| g.take())
}

#[tauri::command]
pub(super) async fn device_approval_pending(
    state: State<'_, AppState>,
) -> Result<Vec<api_client::PendingLink>, String> {
    session_command!(async {
        api_client_from_session(&state)?
            .pending_device_links()
            .await
            .map_err(|e| e.to_string())
    })
}

/// Starts the attempt with the typed code and waits for the new device's answer. Returns the device name the
/// new device proved it reported; the person compares it, then calls `device_approval_approve`.
#[tauri::command]
pub(super) async fn device_approval_begin(
    state: State<'_, AppState>,
    id: String,
    code: String,
) -> Result<String, String> {
    session_command!(async {
        let _ = take_pending();
        let api = api_client_from_session(&state)?;
        let detail = api.device_link_detail(&id).await.map_err(|e| e.to_string())?;
        if detail.status != "pending" {
            return Err("not_pending".to_string());
        }
        let session_id: [u8; 16] = STANDARD
            .decode(&detail.session_id)
            .ok()
            .and_then(|v| v.try_into().ok())
            .ok_or_else(|| "malformed".to_string())?;
        let (approver, outbound) = TypedCodeApprover::start(&code, &session_id, &detail.device_name).map_err(|e| match e {
            CoreError::InvalidInput(_) => "invalid_code".to_string(),
            _ => "crypto".to_string(),
        })?;
        api.send_link_pake_approver(&id, &STANDARD.encode(&outbound))
            .await
            .map_err(|e| e.to_string())?;
        let name = approver.device_name().to_owned();
        // The answer is kept with the attempt; approve checks it after the person compared the name.
        let deadline = std::time::Instant::now() + ANSWER_WAIT;
        let answer = loop {
            if let Some(m) = api.get_link_pake_new_device(&id).await.map_err(|e| e.to_string())? {
                break STANDARD.decode(m).map_err(|_| "malformed".to_string())?;
            }
            if std::time::Instant::now() >= deadline {
                return Err("timeout".to_string());
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        };
        if let Ok(mut g) = PENDING.lock() {
            *g = Some((id, approver, answer));
        }
        Ok(name)
    })
}

/// Confirms the new device's answer (wrong code or tampered name denies the link) and seals the vault key to it.
#[tauri::command]
pub(super) async fn device_approval_approve(state: State<'_, AppState>, id: String) -> Result<(), String> {
    session_command!(async {
        let (pid, approver, answer) = take_pending().ok_or_else(|| "not_pending".to_string())?;
        if pid != id {
            return Err("not_pending".to_string());
        }
        let api = api_client_from_session(&state)?;
        let confirmed = match approver.confirm(&answer) {
            Ok(c) => c,
            Err(e) => {
                let _ = api.deny_link(&id).await;
                return Err(match e {
                    CoreError::Decryption => "mismatch".to_string(),
                    _ => "malformed".to_string(),
                });
            }
        };
        let key = {
            let acct = state.active_account()?;
            let guard = acct.session.lock().map_err(|_| "session mutex poisoned".to_string())?;
            guard.as_ref().ok_or_else(|| "vault is locked".to_string())?.master_key
        };
        let sealed = confirmed
            .seal(AccountKeyMaterial::from_master_key(MasterKey::from_bytes(key)))
            .map_err(|_| "crypto".to_string())?;
        api.approve_link(&id, &STANDARD.encode(sealed.nonce()), &STANDARD.encode(sealed.ciphertext()))
            .await
            .map_err(|e| e.to_string())
    })
}

#[tauri::command]
pub(super) async fn device_approval_deny(state: State<'_, AppState>, id: String) -> Result<(), String> {
    session_command!(async {
        let _ = take_pending();
        api_client_from_session(&state)?.deny_link(&id).await.map_err(|e| e.to_string())
    })
}

