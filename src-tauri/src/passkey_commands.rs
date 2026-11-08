//! v2 B15: the four passkey commands (crypto review 2026-11-08, rule 6), thin wrappers over core's passkey flows
//! (`beebeeb_core::passkey`). Desktop keeps only the platform call (webauthn.dll, `passkey_windows`) and the HTTP.
//!
//! Every secret goes one way, web view to Rust (rule 1): a command takes a password and returns flags, ids and
//! names only. The PRF output, the wrap key, the escrow blob, the vault key, the session and the step-up token stay in
//! Rust. Add and remove each do their own OPAQUE step-up here and drop the token (rules 2 and 3): there is no
//! plaintext-password path.

use super::*;

use beebeeb_core::passkey::ceremony::PasskeyError;
use beebeeb_core::passkey::flow::{self, ApiRequest, ApiResponse, Method, PasskeyHttp};
use zeroize::Zeroizing;

/// One row of Settings → Account → Passkeys. `prf` is `None` while the server does not say whether a passkey can
/// unlock the vault (today's list route sends only id, name and created_at); `last_used_at` likewise.
#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct PasskeyRowDto {
    pub id: String,
    pub name: String,
    pub created_at: String,
    pub last_used_at: Option<String>,
    pub prf: Option<bool>,
}

/// What adding a passkey answers: its id, and whether it can unlock the vault on its own (it has PRF and its vault
/// key escrow was stored). `false` is the DW-A30 case: it signs in, the password is still needed.
#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct PasskeyAddedDto {
    pub id: String,
    pub can_unlock: bool,
}

/// The client's HTTP for core's flows: the app's API base URL and provenance headers.
pub(crate) struct ApiHttp {
    client: reqwest::Client,
    base_url: String,
}

impl ApiHttp {
    pub(crate) fn new() -> Result<Self, String> {
        Self::at(runner::api_base_url())
    }

    pub(crate) fn at(base_url: String) -> Result<Self, String> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .default_headers(api_client::provenance_headers())
            .build()
            .map_err(|_| "passkey_network".to_string())?;
        Ok(Self { client, base_url })
    }
}

impl PasskeyHttp for ApiHttp {
    fn send(&self, r: ApiRequest) -> impl std::future::Future<Output = Result<ApiResponse, PasskeyError>> + Send {
        let client = self.client.clone();
        let url = format!("{}{}", self.base_url, r.path);
        async move {
            let mut req = match r.method {
                Method::Get => client.get(&url),
                Method::Post => client.post(&url),
                Method::Delete => client.delete(&url),
            };
            if let Some(token) = &r.bearer {
                req = req.bearer_auth(token.as_str());
            }
            if let Some(confirm) = &r.confirm_token {
                req = req.header("X-Confirm-Token", confirm.as_str());
            }
            if let Some(body) = &r.body {
                req = req.json(body);
            }
            let resp = req.send().await.map_err(|_| PasskeyError::Network)?;
            let status = resp.status().as_u16();
            let body = resp.json::<serde_json::Value>().await.unwrap_or(serde_json::Value::Null);
            Ok(ApiResponse { status, body })
        }
    }
}

/// A core error as the code the web view maps to its words (rule 9). Never a server body.
fn code(e: PasskeyError) -> String {
    e.code().to_string()
}

/// The session token and vault key of the signed-in account, copied out of the lock and zeroized on drop.
fn session_secrets(state: &AppState) -> Result<(Zeroizing<String>, Zeroizing<[u8; 32]>), String> {
    let acct = state.active_account()?;
    let guard = acct.session.lock().map_err(|_| "session_unavailable".to_string())?;
    let session = guard.as_ref().ok_or_else(|| "vault_locked".to_string())?;
    Ok((Zeroizing::new(session.token.clone()), Zeroizing::new(session.master_key)))
}

/// The OPAQUE step-up (`/auth/confirm-opaque-*`), with `ksf_version` from the server. Returns the single-use confirm
/// token to the CALLER IN RUST, which spends it on one request and drops it. A server that cannot do OPAQUE for this
/// account is an error: there is no plaintext fallback (rule 2).
pub(crate) async fn step_up(http: &ApiHttp, session: &str, password: &str) -> Result<Zeroizing<String>, String> {
    if password.is_empty() {
        return Err("password_required".into());
    }
    let start = beebeeb_core::opaque_protocol::client_login_start(password.as_bytes())
        .map_err(|_| "step_up_failed".to_string())?;
    let resp = http
        .client
        .post(format!("{}/api/v1/auth/confirm-opaque-start", http.base_url))
        .bearer_auth(session)
        .json(&serde_json::json!({ "client_message": encode_base64(&start.message) }))
        .send()
        .await
        .map_err(|_| "passkey_network".to_string())?;
    match resp.status().as_u16() {
        200..=299 => {}
        401 => return Err("passkey_unauthorized".into()),
        409 => return Err("step_up_unavailable".into()),
        _ => return Err("step_up_failed".into()),
    }
    let body: serde_json::Value = resp.json().await.map_err(|_| "step_up_failed".to_string())?;
    let field = |k: &str| body.get(k).and_then(|v| v.as_str()).ok_or_else(|| "step_up_failed".to_string());
    let server_message = decode_base64(field("server_message")?).map_err(|_| "step_up_failed".to_string())?;
    let server_state = field("server_state")?.to_string();
    let ksf_version = body
        .get("ksf_version")
        .and_then(|v| v.as_u64())
        .and_then(|v| u32::try_from(v).ok())
        .ok_or_else(|| "step_up_failed".to_string())?;
    let finish = beebeeb_core::opaque_protocol::client_login_finish(
        &start.state,
        password.as_bytes(),
        &server_message,
        ksf_version,
    )
    .map_err(|_| "password_incorrect".to_string())?;
    let resp = http
        .client
        .post(format!("{}/api/v1/auth/confirm-opaque-finish", http.base_url))
        .bearer_auth(session)
        .json(&serde_json::json!({
            "client_message": encode_base64(&finish.message),
            "server_state": server_state,
        }))
        .send()
        .await
        .map_err(|_| "passkey_network".to_string())?;
    match resp.status().as_u16() {
        200..=299 => {}
        401 => return Err("password_incorrect".into()),
        _ => return Err("step_up_failed".into()),
    }
    let body: serde_json::Value = resp.json().await.map_err(|_| "step_up_failed".to_string())?;
    body.get("confirmation_token")
        .and_then(|v| v.as_str())
        .map(|t| Zeroizing::new(t.to_string()))
        .ok_or_else(|| "step_up_failed".to_string())
}

/// The account's passkeys.
#[tauri::command]
pub(crate) async fn account_passkeys_list(state: State<'_, AppState>) -> Result<Vec<PasskeyRowDto>, String> {
    session_command!(async {
        let (session, _key) = session_secrets(&state)?;
        let http = ApiHttp::new()?;
        let rows = flow::list_passkeys(&http, &session).await.map_err(code)?;
        Ok(rows
            .into_iter()
            .map(|r| PasskeyRowDto {
                id: r.id,
                name: r.name,
                created_at: r.created_at,
                last_used_at: r.last_used_at,
                prf: r.prf,
            })
            .collect())
    })
}

/// Add a passkey: step-up with the password inside Rust, the platform ceremony, and (when the passkey has PRF) the
/// vault key escrow.
#[tauri::command]
pub(crate) async fn account_passkeys_add(
    window: tauri::WebviewWindow,
    state: State<'_, AppState>,
    name: String,
    password: String,
) -> Result<PasskeyAddedDto, String> {
    let password = Zeroizing::new(password);
    session_command!(async {
        let name = name.trim().to_string();
        if name.is_empty() || name.chars().count() > 64 {
            return Err("passkey_name_invalid".to_string());
        }
        let authenticator = platform_authenticator(&window)?;
        let (session, key) = session_secrets(&state)?;
        let http = ApiHttp::new()?;
        let confirm = step_up(&http, &session, &password).await?;
        let vault_key = beebeeb_core::kdf::MasterKey::from_bytes(*key);
        let added = flow::add_passkey(&http, &authenticator, &session, &confirm, Some(&name), Some(&vault_key))
            .await
            .map_err(code)?;
        drop(confirm);
        Ok(PasskeyAddedDto {
            id: added.id,
            can_unlock: added.prf && added.escrowed,
        })
    })
}

/// Remove a passkey, after a step-up with the password inside Rust.
#[tauri::command]
pub(crate) async fn account_passkeys_remove(
    state: State<'_, AppState>,
    id: String,
    password: String,
) -> Result<(), String> {
    let password = Zeroizing::new(password);
    session_command!(async {
        let (session, _key) = session_secrets(&state)?;
        let http = ApiHttp::new()?;
        let confirm = step_up(&http, &session, &password).await?;
        flow::remove_passkey(&http, &session, Some(&confirm), &id)
            .await
            .map_err(code)
    })
}

/// Sign in with a passkey. The server needs the account's email to start the ceremony (its `login-start` takes one),
/// so the sign-in screen passes the email it shows. A passkey with PRF and an escrow unlocks the vault
/// (`vault_unlocked: true`); one without gives a session only, and the password or recovery phrase follows (DW-A30).
#[tauri::command]
pub(crate) async fn auth_passkey_sign_in(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    state: State<'_, AppState>,
    email: String,
) -> Result<LoginOutcome, String> {
    #[cfg(target_os = "windows")]
    {
        let attempt = AUTH_ATTEMPTS.begin()?;
        let work = async {
            let mut turn = state.active_account()?.session_generation();
            if state.auth_present.lock().map(|present| *present).unwrap_or(true) {
                return Err("Sign out of the current account before signing in again.".into());
            }
            let email = email.trim().to_string();
            if email.is_empty() {
                return Err("email_required".to_string());
            }
            let authenticator = platform_authenticator(&window)?;
            let http = ApiHttp::new()?;
            let signed = flow::sign_in(&http, &authenticator, &email).await.map_err(code)?;
            let token = signed.session_token;
            match signed.vault.into_master_key() {
                Some(vault_key) => {
                    let key = Zeroizing::new(vault_key.to_bytes());
                    drop(vault_key);
                    let profile = fetch_canonical_profile(&token).await;
                    apply_session(app.clone(), &state, token.to_string(), &key, profile, &mut turn, &attempt).await?;
                    Ok(LoginOutcome::browser_signed_in())
                }
                None => {
                    let profile = match fetch_session_profile(&http.client, &http.base_url, &token).await {
                        Ok(profile) => profile,
                        Err(_) => {
                            let _ = revoke_desktop_session(&http.client, &http.base_url, &token).await;
                            return Err("sign_in_account_unknown".to_string());
                        }
                    };
                    let _transition = SESSION_TRANSITION.lock().await;
                    AUTH_ATTEMPTS.validate(&attempt)?;
                    if state.auth_present.lock().map(|present| *present).unwrap_or(true) {
                        let _ = revoke_desktop_session(&http.client, &http.base_url, &token).await;
                        return Err("Sign out of the current account before signing in again.".into());
                    }
                    let acct = state.active_account()?;
                    let email = session_email(Some(&profile)).unwrap_or(email);
                    if let Err(e) = store_first_sign_in(&state, &acct, &mut turn, &token, &email, profile) {
                        drop(_transition);
                        let _ = revoke_desktop_session(&http.client, &http.base_url, &token).await;
                        return Err(e);
                    }
                    Ok(LoginOutcome::signed_in())
                }
            }
        };
        attempt.run(work).await
    }
    #[cfg(not(target_os = "windows"))]
    {
        // macOS (ASAuthorization) comes later over the same core flows.
        let _ = (app, window, state, email);
        Err("passkey_unavailable".to_string())
    }
}

#[cfg(target_os = "windows")]
fn platform_authenticator(window: &tauri::WebviewWindow) -> Result<crate::passkey_windows::WindowsAuthenticator, String> {
    let hwnd = window.hwnd().map_err(|_| "passkey_unavailable".to_string())?;
    Ok(crate::passkey_windows::WindowsAuthenticator::new(hwnd.0 as isize))
}

/// No platform call yet off Windows: every ceremony answers `passkey_unavailable`.
#[cfg(not(target_os = "windows"))]
fn platform_authenticator(_window: &tauri::WebviewWindow) -> Result<Unavailable, String> {
    Err("passkey_unavailable".to_string())
}

#[cfg(not(target_os = "windows"))]
pub(crate) struct Unavailable;

#[cfg(not(target_os = "windows"))]
impl flow::PasskeyAuthenticator for Unavailable {
    fn make_credential(
        &self,
        _: &beebeeb_core::passkey::ceremony::CreationOptions,
    ) -> impl std::future::Future<Output = Result<flow::Registration, PasskeyError>> + Send {
        std::future::ready(Err(PasskeyError::Unavailable))
    }
    fn get_assertion(
        &self,
        _: &beebeeb_core::passkey::ceremony::RequestOptions,
    ) -> impl std::future::Future<Output = Result<flow::Assertion, PasskeyError>> + Send {
        std::future::ready(Err(PasskeyError::Unavailable))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Crypto rule 10: the extended no-secret test over every DTO these commands return. Seen red on a planted
    /// `prf_output` field (see `a_planted_secret_field_is_caught`).
    const FORBIDDEN: &[&str] = &[
        "password",
        "phrase",
        "mnemonic",
        "secret",
        "prf",
        "master",
        "backup_codes",
        "otpauth",
        "fragment",
        "token",
        "wrap",
        "blob",
    ];

    fn offending(value: &serde_json::Value) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(map) = value.as_object() {
            for (k, v) in map {
                let lower = k.to_ascii_lowercase();
                // `prf: bool` is a capability flag, not PRF output: the one allow-listed name.
                if lower != "prf" && FORBIDDEN.iter().any(|f| lower.contains(f)) {
                    out.push(k.clone());
                }
                out.extend(offending(v));
            }
        }
        out
    }

    #[test]
    fn passkey_dtos_carry_no_secret_field() {
        let row = serde_json::to_value(PasskeyRowDto {
            id: "a".into(),
            name: "Laptop".into(),
            created_at: "t".into(),
            last_used_at: None,
            prf: Some(true),
        })
        .unwrap();
        let added = serde_json::to_value(PasskeyAddedDto {
            id: "a".into(),
            can_unlock: true,
        })
        .unwrap();
        let outcome = serde_json::to_value(LoginOutcome::browser_signed_in()).unwrap();
        for dto in [&row, &added, &outcome] {
            assert_eq!(offending(dto), Vec::<String>::new(), "{dto}");
        }
        assert_eq!(row.as_object().unwrap().len(), 5);
        assert_eq!(added.as_object().unwrap().len(), 2);
    }

    #[test]
    fn a_planted_secret_field_is_caught() {
        let planted = serde_json::json!({ "id": "a", "prf_output": "00", "nested": { "session_token": "x" } });
        let mut found = offending(&planted);
        found.sort();
        assert_eq!(found, ["prf_output", "session_token"]);
    }

    #[test]
    fn step_up_never_takes_the_plaintext_path() {
        let src = include_str!("passkey_commands.rs");
        let body = &src[src.find("pub(crate) async fn step_up(").unwrap()..src.find("/// The account's passkeys.").unwrap()];
        assert!(!body.contains(concat!("confirm_action_", "plaintext")));
        assert!(!body.contains(concat!("/auth/", "confirm\"")));
        assert!(body.contains("ksf_version"));
    }
}
