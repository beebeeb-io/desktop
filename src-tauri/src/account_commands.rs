//! `account_commands` items split out of `lib.rs`.

use super::*;
pub(crate) use beebeeb_engine::status_views::{
    file_overview_activity, file_overview_inputs, listable_rows, recent_files, shown_transfers,
};

#[cfg(test)]
mod tests;

/// The last email that signed in on this install, for prefilling the
/// onboarding sign-in form after a sign-out or a startup-401 auto sign-out
/// (the keychain account-email credential is erased by sign-out, so this
/// reads the non-secret `desktop.toml` field `last_signed_in_email` — see
/// its doc for the privacy posture). `Ok(None)` when nothing was ever
/// signed in on this install, or the user hand-edited the value away.
#[tauri::command]
pub(super) fn last_signed_in_email() -> Result<Option<String>, String> {
    Ok(DesktopConfig::load().ok().and_then(|cfg| cfg.last_signed_in_email))
}

// ── IPC commands: data layer (account / billing / devices / activity) ─────────
//
// PKG-DATA: wrap the EXISTING server endpoints the desktop never called, so
// the data-backed views ("all pages empty") have something to render. Each
// command builds a short-lived `ApiClient` from the in-memory session,
// fetches a typed DTO, and maps any transport error to a `String` the
// WebView can surface. They share the same "vault locked" gate as
// `desktop_storage_summary`: no in-memory session → `Err("vault is locked")`.

/// Build an [`api_client::ApiClient`] from the current in-memory session.
///
/// Returns `Err("vault is locked")` when there is no session — the same
/// signal `desktop_storage_summary` uses, so the frontend can treat "locked"
/// uniformly. Clones the token + master key out from under the lock so the
/// returned client owns its credentials (the `ApiClient` is intentionally
/// `!Clone`, but constructing a fresh one per call is cheap and keeps the
/// one-authoritative-copy invariant on `Session` itself intact).
pub(super) fn api_client_from_session(state: &State<'_, AppState>) -> Result<api_client::ApiClient, String> {
    // Routing the session lookup here to the active account fixes ALL ~18
    // `account_*` data IPCs in one place (decision 0800): each builds its client
    // through this helper. The `master_key: [u8; 32]` copied into the client is
    // the SAME existing pattern — a `Copy` of the key bytes, not a second
    // authoritative `Session` — so the master-key isolation invariant holds.
    let acct = state.active_account()?;
    let guard = acct.session.lock().map_err(|_| "session mutex poisoned".to_string())?;
    let session = guard.as_ref().ok_or_else(|| "vault is locked".to_string())?;
    Ok(api_client::ApiClient::new(
        runner::api_base_url(),
        session.token.clone(),
        session.master_key,
    ))
}

/// `GET /api/v1/auth/me` (cached). Serves the profile cached at login if
/// present; otherwise fetches once and caches it. Never double-fetches.
#[tauri::command]
pub(super) async fn account_profile(state: State<'_, AppState>) -> Result<account_dto::AccountProfile, String> {
    session_command!(async {
        let acct = state.active_account()?;
        {
            let guard = acct
                .cached_profile
                .lock()
                .map_err(|_| "profile mutex poisoned".to_string())?;
            if let Some(profile) = guard.as_ref() {
                return Ok(profile.clone());
            }
        }
        // Captured before the fetch (Task 12 fix round 1, item 6): a Lock or a Sign-out that completes while it is in
        // flight clears the cached profile, and this answer must not put it back.
        let turn = acct.session_generation();
        let api = api_client_from_session(&state)?;
        let profile = api.account_profile().await.map_err(|e| e.to_string())?;
        cache_profile_in_turn(&acct, &turn, profile.clone());
        Ok(profile)
    })
}

/// `GET /api/v1/auth/me`, always from the server. `account_profile` serves what the session cached at sign-in, which is only
/// as old as the sign-in: Settings › Security shows whether the authenticator app is on, and that must be what the account
/// says now (turned off on the web, it said On until the next sign-in; PR #140 delta review, P2-B).
///
/// It writes nothing: not the cache, not the session. The cached profile is an input to `identity_of_session`, which takes
/// it only when its email is exactly the session's; a profile fetched after the email was changed on the web (the legacy
/// path) carries the new address, and caching it would make the session lose the user id it knows (follow-up P3-6). A caller
/// that sees the session move while this runs is told "Account changed" by the command layer, so no turn is captured here.
#[tauri::command]
pub(super) async fn account_profile_fresh(state: State<'_, AppState>) -> Result<account_dto::AccountProfile, String> {
    session_command!(async {
        let api = api_client_from_session(&state)?;
        api.account_profile().await.map_err(|e| e.to_string())
    })
}

/// The `account_profile` command's cache (Task 12 fix round 1, item 6): written only while no session transition
/// happened since the fetch began (`turn`), checked under the session-write lock. The cached profile IS an input to a
/// sign-in's decision (`credentials_identity`, the `cached_profile` trace), but this one is the session's OWN record,
/// fetched with its own token, so it only makes what is known about the same account more precise; it names no session
/// and does not move the generation on. `true` when it cached.
pub(super) fn cache_profile_in_turn(
    acct: &account::AccountRuntime,
    turn: &account::SessionGeneration,
    profile: account_dto::AccountProfile,
) -> bool {
    let Ok(_write) = check_session_turn(acct, turn) else {
        return false;
    };
    match acct.cached_profile.lock() {
        Ok(mut guard) => {
            *guard = Some(profile);
            true
        }
        Err(_) => false,
    }
}

/// `GET /api/v1/billing/subscription` — plan + quota + lifecycle.
#[tauri::command]
pub(super) async fn account_subscription(state: State<'_, AppState>) -> Result<account_dto::Subscription, String> {
    session_command!(async {
        api_client_from_session(&state)?
            .subscription()
            .await
            .map_err(|e| e.to_string())
    })
}

/// `GET /api/v1/billing/usage` — used/quota bytes + percentage.
#[tauri::command]
pub(super) async fn account_usage(state: State<'_, AppState>) -> Result<account_dto::BillingUsage, String> {
    session_command!(async {
        api_client_from_session(&state)?
            .billing_usage()
            .await
            .map_err(|e| e.to_string())
    })
}

/// `GET /api/v1/me/region` — preferred region + available list. The list is by COUNTRY (task
/// 2050); no city and no provider reaches the frontend.
#[tauri::command]
pub(super) async fn account_region(state: State<'_, AppState>) -> Result<account_dto::UserRegionResponse, String> {
    session_command!(async {
        api_client_from_session(&state)?
            .account_region()
            .await
            .map_err(|e| e.to_string())
    })
}

/// `PUT /api/v1/me/region` — set or clear the preferred storage region (`europe` or a country
/// id) and, for a country, "only store in <country>" (`region_strict`).
#[tauri::command]
pub(super) async fn account_set_region(
    state: State<'_, AppState>,
    preferred_region: Option<String>,
    region_strict: Option<bool>,
) -> Result<account_dto::SetPreferredRegionResponse, String> {
    session_command!(async {
        api_client_from_session(&state)?
            .set_preferred_region(preferred_region.as_deref(), region_strict)
            .await
            .map_err(|e| e.to_string())
    })
}

/// `GET /api/v1/account/activity` — recent events + security summary.
#[tauri::command]
pub(super) async fn account_activity(state: State<'_, AppState>) -> Result<account_dto::AccountActivity, String> {
    session_command!(async {
        api_client_from_session(&state)?
            .account_activity()
            .await
            .map_err(|e| e.to_string())
    })
}

/// `GET /api/v1/account/security-score` — numeric score + factors.
#[tauri::command]
pub(super) async fn account_security_score(state: State<'_, AppState>) -> Result<account_dto::SecurityScore, String> {
    session_command!(async {
        api_client_from_session(&state)?
            .security_score()
            .await
            .map_err(|e| e.to_string())
    })
}

/// `GET /api/v1/account/sessions` — active account sessions.
#[tauri::command]
pub(super) async fn account_session_list(
    state: State<'_, AppState>,
) -> Result<account_dto::AccountSessionList, String> {
    session_command!(async {
        api_client_from_session(&state)?
            .account_sessions()
            .await
            .map_err(|e| e.to_string())
    })
}

/// `DELETE /api/v1/account/sessions/{id}` — revoke one session.
#[tauri::command]
pub(super) async fn account_revoke_session(state: State<'_, AppState>, session_id: String) -> Result<(), String> {
    session_command!(async {
        api_client_from_session(&state)?
            .revoke_account_session(&session_id)
            .await
            .map_err(|e| e.to_string())
    })
}

/// `POST /api/v1/account/sessions/revoke-all-others` — revoke every other
/// session; returns the count revoked.
#[tauri::command]
pub(super) async fn account_revoke_other_sessions(
    state: State<'_, AppState>,
) -> Result<account_dto::RevokeAllResult, String> {
    session_command!(async {
        api_client_from_session(&state)?
            .revoke_other_sessions()
            .await
            .map_err(|e| e.to_string())
    })
}

/// `GET /api/v1/clients/devices` — registered client devices.
#[tauri::command]
pub(super) async fn account_devices(state: State<'_, AppState>) -> Result<account_dto::ClientDeviceList, String> {
    session_command!(async {
        api_client_from_session(&state)?
            .client_devices()
            .await
            .map_err(|e| e.to_string())
    })
}

/// `GET /api/v1/clients/sessions` — per-device sync sessions.
#[tauri::command]
pub(super) async fn account_client_sessions(
    state: State<'_, AppState>,
) -> Result<account_dto::ClientSyncSessionList, String> {
    session_command!(async {
        api_client_from_session(&state)?
            .client_sync_sessions()
            .await
            .map_err(|e| e.to_string())
    })
}

/// `GET /api/v1/activity` — paginated audit-log feed. `page`/`limit` are
/// optional; `None` lets the server defaults apply.
#[tauri::command]
pub(super) async fn account_activity_feed(
    state: State<'_, AppState>,
    page: Option<u32>,
    limit: Option<u32>,
) -> Result<account_dto::ActivityFeed, String> {
    session_command!(async {
        let mut feed = api_client_from_session(&state)?
            .activity_feed(page, limit)
            .await
            .map_err(|e| e.to_string())?;
        if page.unwrap_or(1) == 1 {
            let bridge = parked_conflict_bridge(&state)?;
            bridge
                .append_shared_activity(&mut feed)
                .await
                .map_err(|e| e.to_string())?;
        }
        Ok(feed)
    })
}

/// `GET /api/v1/notifications` — notification list + unread count.
#[tauri::command]
pub(super) async fn account_notifications(state: State<'_, AppState>) -> Result<account_dto::NotificationList, String> {
    session_command!(async {
        api_client_from_session(&state)?
            .notifications()
            .await
            .map_err(|e| e.to_string())
    })
}

/// `GET /api/v1/notifications/preferences` — push-preference toggles.
#[tauri::command]
pub(super) async fn account_notification_preferences(
    state: State<'_, AppState>,
) -> Result<account_dto::NotificationPreferences, String> {
    session_command!(async {
        api_client_from_session(&state)?
            .notification_preferences()
            .await
            .map_err(|e| e.to_string())
    })
}

/// `PUT /api/v1/notifications/preferences` — partial update; omitted fields
/// keep their current server value.
#[tauri::command]
pub(super) async fn account_update_notification_preferences(
    state: State<'_, AppState>,
    update: account_dto::NotificationPreferencesUpdate,
) -> Result<account_dto::NotificationPreferences, String> {
    session_command!(async {
        api_client_from_session(&state)?
            .update_notification_preferences(&update)
            .await
            .map_err(|e| e.to_string())
    })
}

pub(super) fn trash_name_fallback(file_meta: &serde_json::Value, id: &str) -> String {
    if !file_meta["acted_via_share_id"].is_null()
        || !file_meta["uploaded_by"].is_null()
        || !file_meta["owner_wrapped_file_key"].is_null()
    {
        return owner_keys::blocked_name(id);
    }
    let from_plain = ["display_name", "decrypted_name", "filename", "file_name", "name"]
        .into_iter()
        .find_map(|key| {
            file_meta
                .get(key)
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(|s| s.trim().to_string())
        });
    if let Some(name) = from_plain {
        return name;
    }
    if let Some(path) = file_meta
        .get("path")
        .and_then(|v| v.as_str())
        .or_else(|| file_meta.get("display_path").and_then(|v| v.as_str()))
        .and_then(folder_leaf_name)
    {
        return path;
    }
    let short = id.get(..8).unwrap_or(id);
    format!("Encrypted item {short}…")
}

pub(super) fn trash_item_from_meta(
    file_meta: &serde_json::Value,
    master_key: &[u8; 32],
) -> Option<account_dto::DesktopTrashItem> {
    let id = file_meta.get("id").and_then(|v| v.as_str())?.to_string();
    let name =
        owner_keys::display_name(file_meta, &id, master_key).unwrap_or_else(|| trash_name_fallback(file_meta, &id));
    let updated_at = file_meta
        .get("updated_at")
        .and_then(|v| {
            v.as_str()
                .map(str::to_string)
                .or_else(|| v.as_i64().map(|n| n.to_string()))
        })
        .unwrap_or_default();

    Some(account_dto::DesktopTrashItem {
        id,
        name,
        is_folder: file_meta
            .get("is_folder")
            .and_then(|v| v.as_bool())
            .unwrap_or_else(|| file_meta.get("kind").and_then(|v| v.as_str()) == Some("folder")),
        size_bytes: file_meta
            .get("size_bytes")
            .and_then(|v| v.as_i64())
            .or_else(|| file_meta.get("size").and_then(|v| v.as_i64()))
            .unwrap_or(0),
        updated_at,
        parent_id: file_meta.get("parent_id").and_then(|v| v.as_str()).map(str::to_string),
    })
}

pub(super) async fn confirm_action_plaintext(
    client: &reqwest::Client,
    base_url: &str,
    token: &str,
    password: &str,
) -> Result<account_dto::ConfirmActionResult, String> {
    let res = client
        .post(format!("{base_url}/api/v1/auth/confirm"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&serde_json::json!({ "password": password }))
        .send()
        .await
        .map_err(|e| format!("network error: {e}"))?;
    if res.status() == reqwest::StatusCode::UNAUTHORIZED {
        let body = res.json::<serde_json::Value>().await.unwrap_or_default();
        if body.get("error").and_then(|v| v.as_str()) == Some("session_too_old_for_confirmation") {
            return Err(body
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("Sign in again before confirming this action.")
                .to_string());
        }
        return Err("Incorrect password".to_string());
    }
    if !res.status().is_success() {
        let status = res.status();
        let body = res.text().await.unwrap_or_default();
        return Err(format!("Confirm action failed ({status}): {body}"));
    }
    res.json()
        .await
        .map_err(|e| format!("parse confirm action response: {e}"))
}

/// Exchange the current user's password for a single-use destructive-action
/// token. Mirrors the web app's OPAQUE step-up flow, with a plaintext fallback
/// only for legacy accounts that explicitly report `opaque_unavailable`.
#[tauri::command]
pub(super) async fn desktop_confirm_action(
    state: State<'_, AppState>,
    password: String,
) -> Result<account_dto::ConfirmActionResult, String> {
    session_command!(async {
        if password.is_empty() {
            return Err("Password is required.".to_string());
        }
        let token = {
            let acct = state.active_account()?;
            let guard = acct.session.lock().map_err(|_| "session mutex poisoned".to_string())?;
            guard
                .as_ref()
                .map(|s| s.token.clone())
                .ok_or_else(|| "vault is locked".to_string())?
        };

        let base_url = runner::api_base_url();
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .default_headers(api_client::provenance_headers())
            .build()
            .map_err(|e| format!("reqwest build: {e}"))?;

        let login_start = beebeeb_core::opaque_protocol::client_login_start(password.as_bytes())
            .map_err(|e| format!("opaque confirm start: {e}"))?;
        let start_resp = client
            .post(format!("{base_url}/api/v1/auth/confirm-opaque-start"))
            .header("Authorization", format!("Bearer {token}"))
            .json(&serde_json::json!({ "client_message": encode_base64(&login_start.message) }))
            .send()
            .await
            .map_err(|e| format!("network error: {e}"))?;

        if start_resp.status() == reqwest::StatusCode::CONFLICT {
            let body = start_resp.json::<serde_json::Value>().await.unwrap_or_default();
            if body.get("opaque_unavailable").and_then(|v| v.as_bool()) == Some(true) {
                return confirm_action_plaintext(&client, &base_url, &token, &password).await;
            }
            return Err(body
                .get("message")
                .or_else(|| body.get("error"))
                .and_then(|v| v.as_str())
                .unwrap_or("Confirm action failed.")
                .to_string());
        }
        if !start_resp.status().is_success() {
            let status = start_resp.status();
            let body = start_resp.text().await.unwrap_or_default();
            return Err(format!("Confirm action start failed ({status}): {body}"));
        }

        let start_body: serde_json::Value = start_resp
            .json()
            .await
            .map_err(|e| format!("parse confirm start response: {e}"))?;
        let server_message = start_body
            .get("server_message")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "No server_message in confirm start response".to_string())?;
        let server_state = start_body
            .get("server_state")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "No server_state in confirm start response".to_string())?;
        let ksf_version = start_body
            .get("ksf_version")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| "No ksf_version in confirm start response".to_string())? as u32;
        let server_message_bytes =
            decode_base64(server_message).map_err(|e| format!("invalid OPAQUE server message: {e}"))?;
        let login_finish = beebeeb_core::opaque_protocol::client_login_finish(
            &login_start.state,
            password.as_bytes(),
            &server_message_bytes,
            ksf_version,
        )
        .map_err(|_| "Incorrect password".to_string())?;

        let finish_resp = client
            .post(format!("{base_url}/api/v1/auth/confirm-opaque-finish"))
            .header("Authorization", format!("Bearer {token}"))
            .json(&serde_json::json!({
                "client_message": encode_base64(&login_finish.message),
                "server_state": server_state,
            }))
            .send()
            .await
            .map_err(|e| format!("network error: {e}"))?;
        if finish_resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err("Incorrect password".to_string());
        }
        if !finish_resp.status().is_success() {
            let status = finish_resp.status();
            let body = finish_resp.text().await.unwrap_or_default();
            return Err(format!("Confirm action finish failed ({status}): {body}"));
        }
        finish_resp
            .json()
            .await
            .map_err(|e| format!("parse confirm finish response: {e}"))
    })
}

#[tauri::command]
pub(super) async fn desktop_trash_list(
    state: State<'_, AppState>,
) -> Result<Vec<account_dto::DesktopTrashItem>, String> {
    session_command!(async {
        let api = api_client_from_session(&state)?;
        let raw = api.list_trashed_files_all().await.map_err(|e| e.to_string())?;
        let db = DesktopConfig::load()
            .ok()
            .and_then(|cfg| state_db_for_config(&cfg).ok().flatten());
        Ok(raw
            .iter()
            .filter_map(|file_meta| {
                let mut item = trash_item_from_meta(file_meta, api.master_key())?;
                if let Some(db) = db.as_ref()
                    && let Ok(owner_keys::StoredName::Name(name)) =
                        owner_keys::stored_name(db, &item.id, api.master_key())
                {
                    item.name = name;
                }
                Some(item)
            })
            .collect())
    })
}

#[tauri::command]
pub(super) async fn desktop_trash_restore(
    state: State<'_, AppState>,
    file_id: String,
    file_name: Option<String>,
) -> Result<(), String> {
    session_command!(async {
        let api = api_client_from_session(&state)?;
        api.restore_file(&file_id).await.map_err(|e| e.to_string())?;

        if let Ok(Some(db)) = DesktopConfig::load().and_then(|cfg| state_db_for_config(&cfg)) {
            let _ = db.record_local_activity(state_db::LocalActivityEventInput {
                event_type: state_db::LocalActivityKind::Restored,
                file_id: Some(file_id.clone()),
                file_name: file_name
                    .filter(|name| !name.trim().is_empty())
                    .unwrap_or_else(|| file_id.clone()),
                rel_path: None,
                occurred_at: now_unix_seconds(),
            });
            let _ = db.request_resnapshot();
        }

        Ok(())
    })
}

#[tauri::command]
pub(super) async fn desktop_trash_delete_permanently(
    state: State<'_, AppState>,
    file_id: String,
    confirm_token: String,
) -> Result<(), String> {
    session_command!(async {
        if confirm_token.trim().is_empty() {
            return Err("Confirmation token is required.".to_string());
        }
        api_client_from_session(&state)?
            .permanent_delete_file(&file_id, &confirm_token)
            .await
            .map_err(|e| e.to_string())?;
        if let Ok(Some(db)) = DesktopConfig::load().and_then(|cfg| state_db_for_config(&cfg)) {
            let _ = db.request_resnapshot();
        }
        Ok(())
    })
}

/// Local storage breakdown by content type (Media / Documents / Other) plus
/// the N largest files, computed from the desktop SQLite mirror. No server
/// endpoint exists for this — it is computed client-side from
/// `state_db::list_files` (size + path extension). Folders are excluded.
///
/// `largest_limit` caps the returned `largest_files` list; defaults to 10
/// when `None`. Returns an empty breakdown (all three categories present at
/// zero) when no sync root is configured or the state DB is absent — the UI
/// renders a stable empty state rather than an error.
///
/// Gated on an in-memory session (`Err("vault is locked")` when absent) for
/// parity with the other data IPCs: this surfaces local vault file paths, so
/// it must not answer before the user has unlocked.
#[tauri::command]
pub(super) async fn account_storage_breakdown(
    state: State<'_, AppState>,
    largest_limit: Option<usize>,
) -> Result<account_dto::StorageBreakdown, String> {
    session_command!(async {
        // Auth gate: require an unlocked session, same signal as the others.
        {
            let acct = state.active_account()?;
            let guard = acct.session.lock().map_err(|_| "session mutex poisoned".to_string())?;
            if guard.is_none() {
                return Err("vault is locked".to_string());
            }
        }

        let limit = largest_limit.unwrap_or(10);

        // Resolve the state DB from app-local data once a sync root is configured.
        // No root / no DB → empty (not an error).
        if DesktopConfig::load().ok().and_then(|cfg| cfg.sync_root).is_none() {
            return Ok(account_dto::compute_storage_breakdown(&[], limit));
        }
        let db_path = state_paths::beebeeb_state_dir()?.join(state_paths::STATE_DB_FILENAME);
        if !db_path.exists() {
            return Ok(account_dto::compute_storage_breakdown(&[], limit));
        }

        // SQLite work is blocking; run it off the async executor (mirrors
        // `free_up_space`, which uses the same `tokio::task::spawn_blocking` + `??`).
        let breakdown = tokio::task::spawn_blocking(move || -> Result<_, String> {
            let db = state_db::StateDb::open(&db_path).map_err(|e| format!("open state db: {e}"))?;
            let entries = db.list_files().map_err(|e| format!("list files: {e}"))?;
            // Folders carry no real size; exclude them from the breakdown.
            let inputs: Vec<account_dto::BreakdownInput> = entries
                .into_iter()
                .filter(|e| !e.is_dir())
                .map(|e| account_dto::BreakdownInput {
                    file_id: e.file_id,
                    path: e.path,
                    size_bytes: e.size_bytes,
                })
                .collect();
            Ok(account_dto::compute_storage_breakdown(&inputs, limit))
        })
        .await
        .map_err(|e| format!("storage breakdown task failed: {e}"))??;

        Ok(breakdown)
    })
}

/// Per-PC **sync-state** overview for the in-app "Files" tab — the device
/// lens that complements `account_storage_breakdown` (the by-TYPE / billing
/// lens). Computed LOCALLY from the desktop SQLite mirror
/// (`state_db::file_overview_rows`): files grouped by sync status
/// (local / cloud_only / downloading / uploading / conflict / error), the N
/// most-recently-changed files (with effective-pinned flag), and totals.
/// Folders are excluded.
///
/// `recent_limit` caps the returned `recent` list; defaults to 15 when `None`.
/// Returns an empty overview when no sync root is configured or the state DB is
/// absent — the UI renders a stable empty state rather than an error.
///
/// Gated on an in-memory session (`Err("vault is locked")` when absent) for
/// parity with the other data IPCs: this surfaces local vault file paths, so it
/// must not answer before the user has unlocked.
#[tauri::command]
pub(super) async fn desktop_file_overview(
    state: State<'_, AppState>,
    recent_limit: Option<usize>,
) -> Result<account_dto::FileOverview, String> {
    session_command!(async {
        // Auth gate: require an unlocked session, same signal as the others.
        {
            let acct = state.active_account()?;
            let guard = acct.session.lock().map_err(|_| "session mutex poisoned".to_string())?;
            if guard.is_none() {
                return Err("vault is locked".to_string());
            }
        }

        let limit = recent_limit.unwrap_or(15);

        // Resolve the state DB from app-local data once a sync root is configured.
        // No root / no DB → empty (not an error).
        if DesktopConfig::load().ok().and_then(|cfg| cfg.sync_root).is_none() {
            return Ok(account_dto::compute_file_overview(&[], &[], limit));
        }
        let db_path = state_paths::beebeeb_state_dir()?.join(state_paths::STATE_DB_FILENAME);
        if !db_path.exists() {
            return Ok(account_dto::compute_file_overview(&[], &[], limit));
        }

        // SQLite work is blocking; run it off the async executor (mirrors
        // `account_storage_breakdown`).
        let overview = tokio::task::spawn_blocking(move || -> Result<_, String> {
            let db = state_db::StateDb::open(&db_path).map_err(|e| format!("open state db: {e}"))?;
            let rows = listable_rows(&db, db.file_overview_rows().map_err(|e| format!("list files: {e}"))?);
            Ok(account_dto::compute_file_overview(
                &file_overview_inputs(rows),
                &file_overview_activity(&db, limit)?,
                limit,
            ))
        })
        .await
        .map_err(|e| format!("file overview task failed: {e}"))??;

        Ok(overview)
    })
}

/// The tray's recent list (task 2243), which it polls every few seconds while it is open. [`desktop_file_overview`]
/// reads every row for its totals; this reads the newest rows alone.
#[tauri::command]
pub(super) async fn desktop_recent_files(
    state: State<'_, AppState>,
    recent_limit: Option<usize>,
) -> Result<Vec<account_dto::RecentFile>, String> {
    session_command!(async {
        {
            let acct = state.active_account()?;
            let guard = acct.session.lock().map_err(|_| "session mutex poisoned".to_string())?;
            if guard.is_none() {
                return Err("vault is locked".to_string());
            }
        }
        let limit = recent_limit.unwrap_or(15);
        tokio::task::spawn_blocking(move || -> Result<_, String> {
            // As in the overview: no readable config, no sync root or no DB yet is an empty list, not an error (the tray
            // shows an error as signed out).
            let config = DesktopConfig::load().ok();
            let Some(db) = config.as_ref().map(state_db_for_config).transpose()?.flatten() else {
                return Ok(Vec::new());
            };
            recent_files(&db, limit)
        })
        .await
        .map_err(|e| format!("recent files task failed: {e}"))?
    })
}

/// How long a poll waits for state.db's needs-attention rows before it shows the rows kept for the
/// same engine generation.
const ATTENTION_READ_WAIT: std::time::Duration = std::time::Duration::from_millis(300);

/// Every file transfer currently in flight, as a lightweight snapshot for the tray's
/// "In progress" card and the Activity › Timeline view (LD-0105 U1). The board is
/// process-wide and needs no session gate: a not-signed-in tray sees an empty list.
/// Desktop #225 (Amendment 4): plus a row for every file that needs the person (a failed download
/// changed on this device since), only while the vault is unlocked (as `desktop_recent_files`), asked
/// again after the wait. The board is read first and lock-free. The rows come through the running
/// engine's state.db (its lease dropped before the query: no open, no migration lock, no lease held
/// across the DB mutex), waited for at most [`ATTENTION_READ_WAIT`], else the rows kept for this same
/// engine generation (delta review P2-2: never another session's).
#[tauri::command]
pub(super) async fn get_active_transfers(
    state: State<'_, AppState>,
    runtime: State<'_, popover_data::PopoverRuntime>,
) -> Result<Vec<crate::transfer_progress::ActiveTransferDto>, String> {
    use crate::transfer_progress::{ATTENTION_CACHE, ATTENTION_READS, attention_for_poll};
    let rows = runtime.transfers.snapshot_dtos();
    let poll = |unlocked: bool, generation: Option<u64>, fresh| {
        ATTENTION_CACHE
            .lock()
            .map(|mut cache| attention_for_poll(&mut cache, unlocked, generation, fresh))
            .unwrap_or_default()
    };
    // The running engine's generation and its DB, the lease dropped at once (review P3-5).
    #[cfg(target_os = "windows")]
    let engine = crate::windows_cf::bridge().map(|bridge| (bridge.generation_id(), bridge.db_arc()));
    // Off Windows no file is ever poisoned (the Cloud Files provider is Windows-only).
    #[cfg(not(target_os = "windows"))]
    let engine: Option<(u64, std::sync::Arc<crate::state_db::StateDb>)> = None;
    let Some((generation, db)) = engine.filter(|_| vault_unlocked(&state)) else {
        return Ok(shown_transfers(rows, false, &poll(false, None, None)));
    };
    let seq = ATTENTION_READS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let read = tokio::task::spawn_blocking(move || {
        let fresh = crate::transfer_progress::needs_attention(&db);
        if let (Some(fresh), Ok(mut cache)) = (&fresh, ATTENTION_CACHE.lock()) {
            cache.store(generation, seq, fresh.clone());
        }
        fresh
    });
    let fresh = match tokio::time::timeout(ATTENTION_READ_WAIT, read).await {
        Ok(Ok(fresh)) => fresh,
        _ => None,
    };
    // A lock that landed during the wait shows no name (delta review P2-2).
    let unlocked = vault_unlocked(&state);
    let attention = poll(unlocked, Some(generation), fresh);
    Ok(shown_transfers(rows, unlocked, &attention))
}

/// Is the active account's vault unlocked (a session held)? No account, or a poisoned lock: no.
pub(super) fn vault_unlocked(state: &AppState) -> bool {
    state
        .active_account()
        .ok()
        .is_some_and(|acct| acct.session.lock().is_ok_and(|guard| guard.is_some()))
}

/// Query the local desktop file-name search index.
///
/// The index is built from the local StateDb mirror and queried in-process, so
/// typing into the desktop quick-open never waits on the network. When the sync
/// root is not configured or the local mirror has not been created yet, return a
/// typed `syncing` response instead of an empty-looking "no matches" result.
#[tauri::command]
pub(super) async fn desktop_search_files(
    state: State<'_, AppState>,
    query: String,
    limit: Option<usize>,
    for_finder: Option<bool>,
) -> Result<desktop_search::DesktopSearchResponse, String> {
    session_command!(async {
        {
            let acct = state.active_account()?;
            let guard = acct.session.lock().map_err(|_| "session mutex poisoned".to_string())?;
            if guard.is_none() {
                return Err("vault is locked".to_string());
            }
        }

        let Some(db) = DesktopConfig::load().and_then(|cfg| state_db_for_config(&cfg))? else {
            return Ok(desktop_search::syncing_response(query));
        };

        let max_results = limit.unwrap_or(12).clamp(1, 50);
        // Quick search asks for rows it will show in Finder; a Mac's Finder has no shared-with-me rows (ruling 1701), and
        // they are dropped BEFORE the limit (task 1885 fix round 2, n1). Version history asks for none of this.
        let exclude_shared = for_finder.unwrap_or(false) && cfg!(target_os = "macos");
        let response = tokio::task::spawn_blocking(move || -> Result<_, String> {
            let local = desktop_search::build_local_index(&db).map_err(|e| format!("build search index: {e}"))?;
            Ok(desktop_search::query_local_index(
                &local,
                &query,
                max_results,
                exclude_shared,
            ))
        })
        .await
        .map_err(|e| format!("search task failed: {e}"))??;

        Ok(response)
    })
}

// ── IPC commands: App Activity — local process resources (task 1191) ─────────
