//! v2 batch B11: create an account and recover one with the 12 words. Thin Tauri commands over core's
//! `signup_flow` / `recovery_flow` (Guus 2026-11-08: account flows live in core's Rust).
//!
//! Crypto review rules 1 and 9: the password, the confirmation, the typed words and the email code go one way, web view
//! to Rust. Nothing secret comes back, with one exception the board asks for: `auth_signup_phrase_words` returns the 12
//! words for DW-A39, the one screen that shows them, once per flow (allow-listed in `dto_tests`). The session token,
//! the signup ticket, the recovery token and the master key stay in Rust and go to the credential store through
//! `apply_session`. Errors are codes (`SignupError::code`), never a server body.
use std::sync::Mutex;

use beebeeb_core::auth_flow::HttpResponse;
use beebeeb_core::onboarding::{BreachVerdict, PasswordHint, PasswordStrength};
use beebeeb_core::recovery_flow::{PhraseRecovery, Recovered};
use beebeeb_core::signup_flow::{SignupError, SignupFlow, SignupPolicy};
use serde::Serialize;
use zeroize::Zeroizing;

use crate::auth_v2::transport;

#[cfg(test)]
mod dto_tests;

static SIGNUP: Mutex<Option<SignupFlow>> = Mutex::new(None);
static RECOVERY: Mutex<Option<PhraseRecovery>> = Mutex::new(None);
/// The device name chosen on DW-A42, kept for the device row (see the PR: no route takes it yet).
static DEVICE_NAME: Mutex<Option<String>> = Mutex::new(None);
/// Whether DW-A39's words were already handed to the web view for this flow.
static PHRASE_SHOWN: Mutex<bool> = Mutex::new(false);

fn code(e: SignupError) -> String {
    match e {
        SignupError::RateLimited { retry_after_secs } => format!("rate_limited:{retry_after_secs}"),
        SignupError::PasswordTooShort { missing } => format!("password_too_short:{missing}"),
        other => other.code().to_string(),
    }
}

fn take<T>(slot: &Mutex<Option<T>>) -> Result<T, String> {
    slot.lock()
        .map_err(|_| "internal".to_string())?
        .take()
        .ok_or_else(|| "wrong_state".to_string())
}

fn put<T>(slot: &Mutex<Option<T>>, v: T) {
    if let Ok(mut g) = slot.lock() {
        *g = Some(v);
    }
}

fn with<T, R>(slot: &Mutex<Option<T>>, f: impl FnOnce(&mut T) -> Result<R, String>) -> Result<R, String> {
    let mut g = slot.lock().map_err(|_| "internal".to_string())?;
    let v = g.as_mut().ok_or_else(|| "wrong_state".to_string())?;
    f(v)
}

async fn fetch_policy(client: &reqwest::Client) -> Result<SignupPolicy, String> {
    let resp = transport::send(client, None, SignupPolicy::request()).await?;
    SignupPolicy::from_response(&resp).map_err(code)
}

/// The numbers the screens show (DW-A32's countdown, DW-A36's minimum). No secret.
#[derive(Serialize)]
pub struct PolicyView {
    pub code_length: u32,
    pub resend_after_secs: u32,
    pub min_length: u32,
    pub verify_word_count: u32,
    pub terms_url: Option<String>,
    pub privacy_url: Option<String>,
}

impl From<&SignupPolicy> for PolicyView {
    fn from(p: &SignupPolicy) -> Self {
        Self {
            code_length: p.code_length,
            resend_after_secs: p.resend_after_secs,
            min_length: p.min_length,
            verify_word_count: p.verify_word_count,
            terms_url: p.terms_url.clone(),
            privacy_url: p.privacy_url.clone(),
        }
    }
}

/// DW-A31 opens: a fresh flow with the server's policy. Any earlier flow is wiped.
#[tauri::command]
pub(crate) async fn auth_signup_begin() -> Result<PolicyView, String> {
    let client = transport::client()?;
    let policy = fetch_policy(&client).await?;
    let view = PolicyView::from(&policy);
    put(&SIGNUP, SignupFlow::new(policy));
    if let Ok(mut s) = PHRASE_SHOWN.lock() {
        *s = false;
    }
    Ok(view)
}

/// DW-A31 / DW-A34: send (or resend) the email code. The same answer for any address.
#[tauri::command]
pub(crate) async fn auth_signup_send_code(email: String) -> Result<(), String> {
    let req = with(&SIGNUP, |f| f.send_code(&email).map_err(code))?;
    let resp = transport::send(&transport::client()?, None, req).await?;
    with(&SIGNUP, |f| f.on_code_sent(&resp).map_err(code))
}

/// DW-A32: check the code; the ticket stays in Rust.
#[tauri::command]
pub(crate) async fn auth_signup_verify_code(code_typed: String) -> Result<(), String> {
    let typed = Zeroizing::new(code_typed);
    let req = with(&SIGNUP, |f| f.verify_code(&typed).map_err(code))?;
    let resp = transport::send(&transport::client()?, None, req).await?;
    with(&SIGNUP, |f| f.on_code_verified(&resp).map_err(code))
}

/// DW-A36/A37's meter: core's evaluator. Lengths and labels only.
#[derive(Serialize)]
pub struct PasswordView {
    pub length: u32,
    pub min_length: u32,
    pub missing: u32,
    /// `too_short`, `fair`, `good`, `strong`.
    pub strength: &'static str,
    /// `none`, `more`, `mix_case_and_number`, `mix_case`, `number`.
    pub hint: &'static str,
}

fn password_view(e: beebeeb_core::onboarding::PasswordEvaluation) -> PasswordView {
    PasswordView {
        length: e.length,
        min_length: e.min_length,
        missing: e.missing_characters,
        strength: match e.strength {
            PasswordStrength::TooShort => "too_short",
            PasswordStrength::Fair => "fair",
            PasswordStrength::Good => "good",
            PasswordStrength::Strong => "strong",
        },
        hint: match e.hint {
            PasswordHint::None => "none",
            PasswordHint::NeedMoreCharacters => "more",
            PasswordHint::MixCaseAndAddNumberOrSymbol => "mix_case_and_number",
            PasswordHint::MixCase => "mix_case",
            PasswordHint::AddNumberOrSymbol => "number",
        },
    }
}

fn verdict_code(v: Option<BreachVerdict>) -> &'static str {
    match v {
        Some(BreachVerdict::Breached { .. }) => "breached",
        Some(BreachVerdict::CheckFailedBlocked) | None => "blocked",
        Some(_) => "clean",
    }
}

#[tauri::command]
pub(crate) fn auth_signup_password_meter(password: String) -> Result<PasswordView, String> {
    let password = Zeroizing::new(password);
    if let Ok(v) = with(&SIGNUP, |f| Ok(password_view(f.evaluate(&password)))) {
        return Ok(v);
    }
    with(&RECOVERY, |f| Ok(password_view(f.evaluate(&password))))
}

async fn breach_answer(req: beebeeb_core::auth_flow::HttpRequest) -> (Option<u16>, Zeroizing<String>) {
    breach_answer_at(&req.path).await
}

/// The breach endpoint's answer for one prefix path (no session: the endpoint is public). `None` is a network
/// failure; a body over 256 KiB reads as empty. Shared with the password change (`account_v2`).
pub(crate) async fn breach_answer_at(path: &str) -> (Option<u16>, Zeroizing<String>) {
    let Ok(client) = transport::client() else {
        return (None, Zeroizing::new(String::new()));
    };
    let url = format!("{}{}", crate::runner::api_base_url(), path);
    match client.get(&url).send().await {
        Ok(r) => {
            let status = r.status().as_u16();
            // At most 256 KiB, as web reads it.
            let text = r.text().await.unwrap_or_default();
            let text = if text.len() > 256 * 1024 { String::new() } else { text };
            (Some(status), Zeroizing::new(text))
        }
        Err(_) => (None, Zeroizing::new(String::new())),
    }
}

/// DW-A38: the k-anonymity check (only the 5-character prefix leaves this PC). `clean`, `breached` or `blocked`.
#[tauri::command]
pub(crate) async fn auth_signup_breach_check(password: String) -> Result<&'static str, String> {
    let password = Zeroizing::new(password);
    let in_signup = SIGNUP.lock().map(|g| g.is_some()).unwrap_or(false);
    let req = if in_signup {
        with(&SIGNUP, |f| Ok(f.breach_request(&password)))?
    } else {
        with(&RECOVERY, |f| Ok(f.breach_request(&password)))?
    };
    let Some(req) = req else { return Ok("clean") };
    let (status, body) = breach_answer(req).await;
    let v = if in_signup {
        with(&SIGNUP, |f| Ok(f.on_breach_response(status, &body)))?
    } else {
        with(&RECOVERY, |f| Ok(f.on_breach_response(status, &body)))?
    };
    Ok(verdict_code(v))
}

/// DW-A36 Continue: the gate (match, length, breach verdict for this exact password).
#[tauri::command]
pub(crate) fn auth_signup_set_password(password: String, confirmation: String) -> Result<(), String> {
    let (password, confirmation) = (Zeroizing::new(password), Zeroizing::new(confirmation));
    with(&SIGNUP, |f| {
        f.set_password(&password, &confirmation).map(|_| ()).map_err(code)
    })
}

/// DW-A39: the 12 words, generated in core on this PC. Handed to the web view once per flow; "Show the words again"
/// (DW-A40) asks again and gets the same words while the flow lives.
#[tauri::command]
pub(crate) fn auth_signup_phrase_words() -> Result<Vec<String>, String> {
    let words = with(&SIGNUP, |f| f.phrase_words().map_err(code))?;
    if let Ok(mut s) = PHRASE_SHOWN.lock() {
        *s = true;
    }
    Ok(words.iter().map(|w| w.as_str().to_owned()).collect())
}

/// DW-A39 Copy (B11 ruling): Rust puts the 12 words on the clipboard itself, numbered, and clears them after 60 s if
/// the clipboard still holds them. The web view never runs the copy. The engine owns the policy; the shell provides the
/// clipboard through `ClipboardAccess`.
#[tauri::command]
pub(crate) fn auth_signup_phrase_copy(app: tauri::AppHandle) -> Result<(), String> {
    let words = with(&SIGNUP, |f| f.phrase_words().map_err(code))?;
    let text = Zeroizing::new(words.iter().map(|w| w.as_str()).collect::<Vec<_>>().join(" "));
    let clipboard = std::sync::Arc::new(TauriClipboard(app));
    beebeeb_engine::clipboard::copy_and_clear_after(clipboard, text)
}

/// Tauri's clipboard plugin, behind the engine's [`beebeeb_engine::clipboard::ClipboardAccess`] trait.
struct TauriClipboard(tauri::AppHandle);

impl beebeeb_engine::clipboard::ClipboardAccess for TauriClipboard {
    fn write_text(&self, text: String) -> Result<(), String> {
        use tauri_plugin_clipboard_manager::ClipboardExt;
        self.0.clipboard().write_text(text).map_err(|e| e.to_string())
    }
    fn read_text(&self) -> Result<String, String> {
        use tauri_plugin_clipboard_manager::ClipboardExt;
        self.0.clipboard().read_text().map_err(|e| e.to_string())
    }
    fn clear(&self) -> Result<(), String> {
        use tauri_plugin_clipboard_manager::ClipboardExt;
        self.0.clipboard().clear().map_err(|e| e.to_string())
    }
}

/// DW-A39 Save as file: written from Rust's copy of the words, a plain text file with the email in its name.
#[tauri::command]
pub(crate) async fn auth_signup_phrase_save(app: tauri::AppHandle) -> Result<bool, String> {
    use tauri_plugin_dialog::DialogExt;
    let (email, words) = with(&SIGNUP, |f| {
        Ok((
            f.email().unwrap_or("account").to_owned(),
            f.phrase_words().map_err(code)?,
        ))
    })?;
    let mut text = Zeroizing::new(format!("Beebeeb recovery phrase for {email}\n\n"));
    for (i, w) in words.iter().enumerate() {
        text.push_str(&format!("{:>2}. {}\n", i + 1, w.as_str()));
    }
    let name = format!("Beebeeb recovery phrase ({email}).txt");
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .add_filter("Text", &["txt"])
            .set_file_name(&name)
            .blocking_save_file()
    })
    .await
    .map_err(|_| "internal".to_string())?;
    let Some(path) = picked.and_then(|p| p.into_path().ok()) else {
        return Ok(false);
    };
    std::fs::write(&path, text.as_bytes()).map_err(|_| "write_failed".to_string())?;
    Ok(true)
}

/// DW-A39's checkbox, then DW-A40's three positions (1-based, chosen in core).
#[tauri::command]
pub(crate) fn auth_signup_phrase_challenge() -> Result<Vec<u32>, String> {
    with(&SIGNUP, |f| {
        f.acknowledge_phrase().map_err(code)?;
        f.challenge_positions().map_err(code)
    })
}

/// DW-A40 Confirm: core compares; the words are not kept in app state.
#[tauri::command]
pub(crate) fn auth_signup_confirm_phrase(words: Vec<String>) -> Result<(), String> {
    let words: Vec<Zeroizing<String>> = words.into_iter().map(Zeroizing::new).collect();
    with(&SIGNUP, |f| f.confirm_phrase(&words).map_err(code))
}

/// DW-A42: the computer name from Windows, as the prefill.
#[tauri::command]
pub(crate) fn auth_signup_default_device_name() -> String {
    crate::runner::device_name().unwrap_or_default()
}

/// DW-A42 Create my account then DW-A43: register, store the session and the key, record the terms version.
/// Errors: `registration_failed` (nothing stored, Try again), `ticket_expired` (back to Verify, the rest kept),
/// `finish_failed` (the account exists; Sign in is the only way on).
#[tauri::command]
pub(crate) async fn auth_signup_create(app: tauri::AppHandle, device_name: String) -> Result<(), String> {
    let name: String = device_name.trim().chars().take(40).collect();
    let name = if name.is_empty() {
        crate::runner::device_name().unwrap_or_default()
    } else {
        name
    };
    if let Ok(mut d) = DEVICE_NAME.lock() {
        *d = Some(name);
    }
    let client = transport::client()?;
    let mut flow = take(&SIGNUP)?;
    let result = async {
        let req = flow.register_start().map_err(code)?;
        let resp = transport::send(&client, None, req).await?;
        let req = flow.on_register_start(&resp).map_err(code)?;
        let resp: HttpResponse = transport::send(&client, None, req).await?;
        flow.on_register_finish(&resp).map_err(code)
    }
    .await;
    let created = match result {
        Ok(c) => c,
        Err(e) => {
            put(&SIGNUP, flow);
            return Err(e);
        }
    };
    let terms = flow.terms_request();
    flow.wipe();
    let key = Zeroizing::new(created.master_key.to_bytes());
    if install(&app, &created.session_token, &key).await.is_err() {
        return Err("finish_failed".into());
    }
    if let Some(req) = terms {
        // Best effort: the account is made; a missed record is asked again by the server's onboarding document.
        let _ = transport::send(&client, Some(&created.session_token), req).await;
    }
    Ok(())
}

/// Leaving the create flow (Back on DW-A31, or the window closes): everything is forgotten.
#[tauri::command]
pub(crate) fn auth_signup_cancel() {
    if let Ok(mut g) = SIGNUP.lock() {
        if let Some(f) = g.as_mut() {
            f.wipe();
        }
        *g = None;
    }
    if let Ok(mut g) = RECOVERY.lock() {
        *g = None;
    }
}

/// DW-A47 Verify phrase: the words become a master key here; only its recovery check is sent. Errors:
/// `invalid_phrase` (not 12 list words with a valid checksum), `phrase_not_accepted`, `rate_limited:<s>`.
#[tauri::command]
pub(crate) async fn auth_recover_with_phrase(email: String, words: Vec<String>) -> Result<PolicyView, String> {
    let phrase = Zeroizing::new(
        words
            .iter()
            .map(|w| w.trim().to_lowercase())
            .collect::<Vec<_>>()
            .join(" "),
    );
    let mut words = words;
    for w in &mut words {
        zeroize::Zeroize::zeroize(w);
    }
    let client = transport::client()?;
    let policy = fetch_policy(&client).await?;
    let view = PolicyView::from(&policy);
    let (mut flow, req) = PhraseRecovery::start(policy, &email, &phrase).map_err(code)?;
    let resp = transport::send(&client, None, req).await?;
    flow.on_start_response(&resp).map_err(code)?;
    put(&RECOVERY, flow);
    Ok(view)
}

/// DW-A49 Set new password: the OPAQUE re-registration around the same master key. `signed_in` or
/// `sign_in_required` (two-factor on: the person signs in with the new password next).
#[tauri::command]
pub(crate) async fn auth_recover_set_password(
    app: tauri::AppHandle,
    password: String,
    confirmation: String,
) -> Result<&'static str, String> {
    let (password, confirmation) = (Zeroizing::new(password), Zeroizing::new(confirmation));
    let client = transport::client()?;
    let mut flow = take(&RECOVERY)?;
    let req = match flow.set_password(&password, &confirmation) {
        Ok(r) => r,
        Err(e) => {
            put(&RECOVERY, flow);
            return Err(code(e));
        }
    };
    let resp = transport::send(&client, None, req).await?;
    let req = match flow.on_register_response(&resp) {
        Ok(r) => r,
        Err(SignupError::RecoveryExpired) => return Err("recovery_expired".into()),
        Err(e) => {
            put(&RECOVERY, flow);
            return Err(code(e));
        }
    };
    let resp = transport::send(&client, None, req).await?;
    match flow.on_finalize_response(&resp).map_err(code)? {
        Recovered::SignedIn {
            session_token,
            master_key,
        } => {
            let key = Zeroizing::new(master_key.to_bytes());
            install(&app, &session_token, &key)
                .await
                .map_err(|_| "finish_failed".to_string())?;
            Ok("signed_in")
        }
        Recovered::SignInRequired => Ok("sign_in_required"),
    }
}

/// Store the session and key the way every sign-in does (`apply_session`, under `claim_session_write`).
#[cfg(target_os = "windows")]
async fn install(app: &tauri::AppHandle, session_token: &str, master_key: &[u8; 32]) -> Result<(), String> {
    use tauri::Manager;
    let state = app.state::<crate::AppState>();
    let attempt = crate::AUTH_ATTEMPTS.begin()?;
    let work = async {
        let mut turn = state.active_account()?.session_generation();
        let profile = crate::fetch_canonical_profile(session_token).await;
        crate::apply_session(
            app.clone(),
            &state,
            session_token.to_string(),
            master_key,
            profile,
            &mut turn,
            &attempt,
        )
        .await
    };
    attempt.run(work).await
}

/// v2 is Windows first (Guus 2026-11-08); macOS gets this on the same core flow later.
#[cfg(not(target_os = "windows"))]
async fn install(_app: &tauri::AppHandle, _session_token: &str, _master_key: &[u8; 32]) -> Result<(), String> {
    Err("unsupported_platform".into())
}

/// DW-A40's suggestion strip: up to 6 words of the public 2048-word list that start with what was typed (2+ letters).
#[tauri::command]
pub(crate) fn auth_phrase_suggest(prefix: String) -> Vec<String> {
    let prefix = Zeroizing::new(prefix.trim().to_lowercase());
    if prefix.len() < 2 {
        return Vec::new();
    }
    bip39::Language::English
        .words_by_prefix(&prefix)
        .iter()
        .take(6)
        .map(|w| w.to_string())
        .collect()
}
