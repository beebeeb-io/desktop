//! v2 B15: the four passkey commands (crypto review 2026-11-08, rule 6), thin wrappers over core's passkey flows
//! (`beebeeb_core::passkey`). Desktop keeps only the platform call (webauthn.dll, `passkey_windows`) and the HTTP.
//!
//! Every secret goes one way, web view to Rust (rule 1): a command takes a password and returns flags, ids and
//! names only. The PRF output, the wrap key, the escrow blob, the vault key, the session and the step-up token stay in
//! Rust. Add and remove hand the password to core, which runs the OPAQUE step-up inside the flow and puts the
//! single-use token on the one request that spends it (core v0.2.8 item 2; rules 2 and 3): no token is ever held
//! here, and there is no plaintext-password path.
//!
//! The relying party is this build's own ([`expected_rp`], core v0.2.8 item 1): a server answer for another one is
//! refused before the platform prompt.

use super::*;

use beebeeb_core::passkey::ceremony::{ExpectedRp, PasskeyError};
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
            let body = resp
                .json::<serde_json::Value>()
                .await
                .unwrap_or(serde_json::Value::Null);
            Ok(ApiResponse { status, body })
        }
    }
}

/// A core error as the code the web view maps to its words (rule 9). Never a server body. A step-up that failed
/// inside core keeps its reason where the window has words for it (a wrong password, too many tries).
fn code(e: PasskeyError) -> String {
    match e {
        PasskeyError::StepUp { code } => match code {
            "invalid_credentials" => "wrong_password".to_string(),
            "rate_limited" | "opaque_unavailable" => code.to_string(),
            "invalid_input" => "password_required".to_string(),
            _ => "passkey_step_up_failed".to_string(),
        },
        PasskeyError::Network => "network".to_string(),
        other => other.code().to_string(),
    }
}

/// The production relying party (the server's `WEBAUTHN_RP_ID`).
const PRODUCTION_RP_ID: &str = "beebeeb.io";

/// The relying party this build expects, from its own build configuration: never the server's answer, never a
/// setting. A release build always expects `beebeeb.io`. A debug build built with `BB_PASSKEY_RP_ID=localhost`
/// expects the server's development default instead (core accepts `localhost` only in a debug build).
pub(crate) fn expected_rp() -> ExpectedRp {
    ExpectedRp::new(expected_rp_id(cfg!(debug_assertions), option_env!("BB_PASSKEY_RP_ID")))
}

fn expected_rp_id(debug_build: bool, build_env: Option<&str>) -> &'static str {
    match build_env.map(str::trim) {
        Some("localhost") if debug_build => "localhost",
        _ => PRODUCTION_RP_ID,
    }
}

/// The session token and vault key of the signed-in account, copied out of the lock and zeroized on drop.
fn session_secrets(state: &AppState) -> Result<(Zeroizing<String>, Zeroizing<[u8; 32]>), String> {
    let acct = state.active_account()?;
    let guard = acct.session.lock().map_err(|_| "session_unavailable".to_string())?;
    let session = guard.as_ref().ok_or_else(|| "vault_locked".to_string())?;
    Ok((
        Zeroizing::new(session.token.clone()),
        Zeroizing::new(session.master_key),
    ))
}

/// Add a passkey over any HTTP and platform call: core's flow with this build's relying party. The password goes
/// into core for the step-up; the answer is the id and one flag.
pub(crate) async fn add_with<H: PasskeyHttp, A: flow::PasskeyAuthenticator>(
    http: &H,
    authenticator: &A,
    session: &str,
    password: &str,
    name: &str,
    vault_key: &beebeeb_core::kdf::MasterKey,
) -> Result<PasskeyAddedDto, String> {
    if password.is_empty() {
        return Err("password_required".into());
    }
    let added = flow::add_passkey(
        http,
        authenticator,
        session,
        password.as_bytes(),
        Some(name),
        Some(vault_key),
        &expected_rp(),
    )
    .await
    .map_err(code)?;
    Ok(PasskeyAddedDto {
        id: added.id,
        can_unlock: added.prf && added.escrowed,
    })
}

/// Remove a passkey: core runs the step-up from the password and spends its token on the `DELETE`.
pub(crate) async fn remove_with<H: PasskeyHttp>(
    http: &H,
    session: &str,
    password: &str,
    id: &str,
) -> Result<(), String> {
    if password.is_empty() {
        return Err("password_required".into());
    }
    flow::remove_passkey(http, session, password.as_bytes(), id)
        .await
        .map_err(code)
}

/// Passkey sign-in over any HTTP and platform call, against this build's relying party.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(crate) async fn sign_in_with<H: PasskeyHttp, A: flow::PasskeyAuthenticator>(
    http: &H,
    authenticator: &A,
    email: &str,
) -> Result<flow::SignedIn, String> {
    flow::sign_in(http, authenticator, email, &expected_rp())
        .await
        .map_err(code)
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
        let vault_key = beebeeb_core::kdf::MasterKey::from_bytes(*key);
        add_with(&http, &authenticator, &session, &password, &name, &vault_key).await
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
        remove_with(&http, &session, &password, &id).await
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
            let signed = sign_in_with(&http, &authenticator, &email).await?;
            let token = signed.session_token;
            match signed.vault.into_master_key() {
                Some(vault_key) => {
                    let key = Zeroizing::new(vault_key.to_bytes());
                    drop(vault_key);
                    let profile = fetch_canonical_profile(&token).await;
                    apply_session(
                        app.clone(),
                        &state,
                        token.to_string(),
                        &key,
                        profile,
                        &mut turn,
                        &attempt,
                    )
                    .await?;
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
fn platform_authenticator(
    window: &tauri::WebviewWindow,
) -> Result<crate::passkey_windows::WindowsAuthenticator, String> {
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
    fn the_step_up_runs_inside_core_with_no_plaintext_path() {
        let src = include_str!("passkey_commands.rs");
        let production = &src[..src.find("#[cfg(test)]\nmod tests").unwrap()];
        // Core v0.2.8 item 2: the password goes into core; this file runs no OPAQUE leg and holds no token.
        assert!(!production.contains(concat!("confirm_action_", "plaintext")));
        assert!(!production.contains(concat!("/auth/", "confirm\"")));
        assert!(!production.contains(concat!("confirm-", "opaque")));
        assert!(!production.contains(concat!("confirmation_", "token")));
        let add = &production[production.find("pub(crate) async fn add_with").unwrap()..];
        assert!(add[..add.find("\n}\n").unwrap()].contains("password.as_bytes()"));
        let remove = &production[production.find("pub(crate) async fn remove_with").unwrap()..];
        assert!(remove[..remove.find("\n}\n").unwrap()].contains("password.as_bytes()"));
    }

    #[test]
    fn the_relying_party_comes_from_the_build_never_the_server() {
        assert_eq!(expected_rp_id(false, None), "beebeeb.io");
        assert_eq!(expected_rp_id(true, None), "beebeeb.io");
        assert_eq!(expected_rp_id(true, Some("localhost")), "localhost");
        assert_eq!(
            expected_rp_id(false, Some("localhost")),
            "beebeeb.io",
            "a release build never expects localhost"
        );
        assert_eq!(
            expected_rp_id(true, Some("evil.example")),
            "beebeeb.io",
            "only localhost overrides"
        );
        assert_eq!(
            expected_rp().rp_id(),
            "beebeeb.io",
            "this test build was built without the override"
        );
    }

    /// The server's answers, scripted per path; every request is counted.
    struct Server {
        rp_id: &'static str,
        sent: std::sync::Mutex<Vec<String>>,
    }

    impl PasskeyHttp for Server {
        fn send(&self, r: ApiRequest) -> impl std::future::Future<Output = Result<ApiResponse, PasskeyError>> + Send {
            self.sent.lock().unwrap().push(r.path.clone());
            let body = match r.path.as_str() {
                "/api/v1/auth/passkey/login-start" => serde_json::json!({
                    "publicKey": { "challenge": "AAAA", "rpId": self.rp_id, "allowCredentials": [] },
                    "auth_state": "state",
                    "user_id": "user",
                }),
                _ => serde_json::Value::Null,
            };
            std::future::ready(Ok(ApiResponse { status: 200, body }))
        }
    }

    /// A platform call that only counts: it answers "cancelled".
    #[derive(Default)]
    struct Prompt {
        asked: std::sync::atomic::AtomicUsize,
    }

    impl flow::PasskeyAuthenticator for Prompt {
        fn make_credential(
            &self,
            _: &beebeeb_core::passkey::ceremony::CreationOptions,
        ) -> impl std::future::Future<Output = Result<flow::Registration, PasskeyError>> + Send {
            self.asked.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            std::future::ready(Err(PasskeyError::Cancelled))
        }
        fn get_assertion(
            &self,
            _: &beebeeb_core::passkey::ceremony::RequestOptions,
        ) -> impl std::future::Future<Output = Result<flow::Assertion, PasskeyError>> + Send {
            self.asked.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            std::future::ready(Err(PasskeyError::Cancelled))
        }
    }

    fn sign_in_against(rp_id: &'static str) -> (Result<(), String>, usize, usize) {
        let server = Server {
            rp_id,
            sent: std::sync::Mutex::new(Vec::new()),
        };
        let prompt = Prompt::default();
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        let result = rt
            .block_on(sign_in_with(&server, &prompt, "anna@example.com"))
            .map(|_| ());
        let sent = server.sent.lock().unwrap().len();
        (result, sent, prompt.asked.load(std::sync::atomic::Ordering::SeqCst))
    }

    #[test]
    fn a_server_naming_another_relying_party_is_refused_before_the_prompt() {
        // The expected relying party reaches the platform prompt (which this fake cancels).
        let (ok_rp, sent, asked) = sign_in_against("beebeeb.io");
        assert_eq!((ok_rp, sent, asked), (Err("passkey_cancelled".to_string()), 1, 1));
        // Any other: refused by core with this build's relying party, and Windows Hello never opens.
        let mut refused = 0;
        for rp in [
            "evil.example",
            "localhost",
            "beebeeb.io.evil.example",
            "app.beebeeb.io",
            "",
        ] {
            let (result, sent, asked) = sign_in_against(rp);
            assert_eq!(result, Err("passkey_rp_mismatch".to_string()), "{rp}");
            assert_eq!((sent, asked), (1, 0), "{rp}: one login-start, no prompt");
            refused += 1;
        }
        assert_eq!(refused, 5);
    }

    #[test]
    fn step_up_reasons_keep_their_words_and_never_carry_a_body() {
        assert_eq!(
            code(PasskeyError::StepUp {
                code: "invalid_credentials"
            }),
            "wrong_password"
        );
        assert_eq!(code(PasskeyError::StepUp { code: "rate_limited" }), "rate_limited");
        assert_eq!(code(PasskeyError::StepUp { code: "server" }), "passkey_step_up_failed");
        assert_eq!(code(PasskeyError::RpMismatch), "passkey_rp_mismatch");
        assert_eq!(code(PasskeyError::Network), "network");
        assert_eq!(
            code(PasskeyError::Server {
                status: 500,
                code: Some("anything the server said".into())
            }),
            "passkey_server_error"
        );
    }
}
