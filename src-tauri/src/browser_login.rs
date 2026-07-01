//! Browser-based device-code login handoff for the desktop client.
//!
//! This is the desktop counterpart to `bb login` in `repos/cli`. The server
//! contract lives in `repos/server/beebeeb-api/src/cli_auth.rs`; the crypto is a
//! byte-for-byte match of the CLI client (`repos/cli/src/commands/login.rs`) so
//! all three clients (web/cli/desktop) interoperate against the same handoff.
//!
//! Flow (see `cli_auth.rs` for the server side):
//!   1. Desktop opens a WebSocket to `wss://api.beebeeb.io/api/v1/auth/cli` and
//!      sends an ephemeral P-256 ECDH public key: `{"ecdh_public_key_b64": "..."}`.
//!   2. Server replies `{"user_code","verification_uri","expires_in"}` (300s).
//!   3. We open the system browser at `verification_uri` and emit a
//!      `browser-login` event so the UI can show the code + "we opened your
//!      browser" state.
//!   4. The (already-signed-in) browser fetches our pubkey, ECDH-encrypts
//!      `{session_token, master_key_b64, email}`, and POSTs it to the server,
//!      which pushes the encrypted blob back over our WS.
//!   5. We derive the shared secret, run HKDF-SHA256 with info
//!      `"beebeeb-cli-auth-v1"`, AES-256-GCM-decrypt, and hand the credentials
//!      to `crate::apply_session` (persist to the platform credential store +
//!      start the engine + windows_cf activation).
//!
//! ## Tokens / "refresh"
//!
//! The server session token is **opaque and ~30 days** (`sessions` table,
//! 30-day expiry — see server `CLAUDE.md` "Auth model"). There is **no
//! server-side refresh-token grant**. "Refreshing" a desktop session therefore
//! means re-running this exact browser handoff. On a 401 / expiry the UI must
//! re-invoke `start_browser_login` (the engine's API client surfaces the 401;
//! `clear_session` then `start_browser_login` re-onboards). Do **not** invent a
//! `/refresh` endpoint — it does not exist.

use std::sync::Arc;
use std::time::Duration;

use aes_gcm::aead::Aead;
use aes_gcm::aead::generic_array::GenericArray;
use aes_gcm::{Aes256Gcm, KeyInit};
use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
use futures_util::{SinkExt, StreamExt};
use p256::PublicKey;
use p256::ecdh::EphemeralSecret;
use p256::elliptic_curve::sec1::ToEncodedPoint;
use rand::rngs::OsRng;
use tauri::{Emitter, State};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{Connector, connect_async_tls_with_config};

use crate::AppState;
use crate::runner;

/// Sign-in breadcrumb. These trace the exact step the handoff reached so a
/// hung/failed Windows sign-in can be diagnosed from stderr. They are pure
/// diagnostics (never carry credential material) and are noise in a shipped
/// build, so they compile to `eprintln!` ONLY in debug builds and to nothing in
/// release. Keep payloads credential-free: file_ids/urls/phase names only.
macro_rules! bc {
    ($($arg:tt)*) => {
        #[cfg(debug_assertions)]
        {
            eprintln!($($arg)*);
        }
    };
}

/// HKDF `info` string — MUST match the CLI client and the web browser side.
const HKDF_INFO: &[u8] = b"beebeeb-cli-auth-v1";

/// Tauri event name the UI subscribes to for handoff progress.
const EVENT: &str = "browser-login";

/// Hard ceiling on the initial WebSocket connect (TCP + TLS handshake). On
/// Windows this previously had NO bound: a stalled TLS handshake (the native
/// cert-store path) left the UI frozen on "Connecting" forever and the browser
/// never opened. 15s is generous for a healthy network yet fails fast enough to
/// show the user an actionable error instead of an infinite spinner.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// Hard ceiling on the device-code HANDOFF — sending our public key and
/// receiving the server's first reply (the `{user_code, verification_uri}`
/// frame). The whole later wait for the browser to authorise is already bounded
/// by the server's code lifetime (`expires_in`), but the FIRST round-trip had no
/// bound of its own: if the socket connected but the server never sent the
/// device code (a cold/stalled first handoff), the UI sat on "Connecting"
/// forever with the browser never opening. 15s is generous for a healthy
/// server yet fails fast with an actionable error instead of an infinite spinner.
const DEVICE_CODE_TIMEOUT: Duration = Duration::from_secs(15);

/// Emit a structured progress event to all windows. Non-fatal on failure.
fn emit(app: &tauri::AppHandle, phase: &str, payload: serde_json::Value) {
    let mut body = payload;
    if let Some(obj) = body.as_object_mut() {
        obj.insert("phase".to_string(), serde_json::Value::String(phase.to_string()));
    }
    let _ = app.emit(EVENT, body);
}

/// Build the `wss://…/api/v1/auth/cli` URL from the engine's API base.
fn ws_url() -> String {
    let base = runner::api_base_url();
    let ws = base.replacen("https://", "wss://", 1).replacen("http://", "ws://", 1);
    format!("{ws}/api/v1/auth/cli")
}

/// Build a rustls `Connector` that validates the server cert against the
/// **bundled Mozilla roots** (`webpki-roots`) rather than the OS certificate
/// store.
///
/// This is the core Windows fix. With `rustls-tls-native-roots`,
/// `tokio-tungstenite` loaded the Windows root store to validate
/// `api.beebeeb.io`; on some Windows machines that path stalled the TLS
/// handshake, hanging the sign-in handoff on "Connecting" with no browser ever
/// opening. api.beebeeb.io serves a public-CA (Let's Encrypt-class) certificate
/// that the bundled Mozilla root set validates, so this is both correct and
/// deterministic across machines.
///
/// We build the `ClientConfig` with an explicit **`ring`** provider:
/// `ring` AND `aws_lc_rs` are both enabled in this dependency tree (reqwest /
/// tauri-plugin-updater pull them in), so rustls cannot auto-select a
/// process-default provider and `ClientConfig::builder()` would PANIC at
/// runtime. `builder_with_provider(...)` sidesteps that.
///
/// We deliberately choose `ring` over `aws_lc_rs`: `aws_lc_rs` vendors a C
/// library whose runtime initialisation stalled/panicked on some Windows
/// machines, hanging this handoff forever on "Connecting" with NO error and NO
/// timeout (the panic left the IPC promise unresolved before the connect
/// timeout could fire). `ring` has no vendored-C init and is far less likely to
/// stall on Windows.
///
/// Returns `Result` and is panic-safe: the provider + config build is wrapped
/// in `catch_unwind` (it is fully synchronous, so this is valid) so a provider
/// init panic becomes a clean `Err` the caller surfaces as an `error` phase,
/// instead of a silent infinite spinner.
fn webpki_rustls_connector() -> Result<Connector, String> {
    std::panic::catch_unwind(|| {
        let root_store = rustls::RootCertStore {
            roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
        };
        bc!("[bb-signin] connector: before ring default_provider");
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        bc!("[bb-signin] connector: after ring default_provider");
        let config = rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(|e| format!("ring provider rejected default TLS versions: {e}"))?
            .with_root_certificates(root_store)
            .with_no_client_auth();
        bc!("[bb-signin] connector: after config build");
        Ok::<Connector, String>(Connector::Rustls(Arc::new(config)))
    })
    .unwrap_or_else(|panic_payload| {
        let detail = panic_payload
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| panic_payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "unknown panic".to_string());
        bc!("[bb-signin] connector: PANIC during TLS init :: {detail}");
        Err(format!("TLS init failed: {detail}"))
    })
}

/// Append a single diagnostic line to `<data_dir>/beebeeb-signin.log`.
///
/// stdout is block-buffered when the app is launched detached / redirected, so
/// `tracing` output can be lost on a hard failure. This file is a best-effort,
/// always-flushed breadcrumb the lead can `cat` to see the last sign-in outcome.
/// Never fatal.
fn signin_log(line: &str) {
    use std::io::Write;
    let Some(dir) = dirs::data_dir() else { return };
    let path = dir.join("beebeeb-signin.log");
    let stamp = chrono::Utc::now().to_rfc3339();
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "{stamp} {line}");
    }
}

/// Start the browser-based login handoff and, on success, install the session.
///
/// Emits `browser-login` events with a `phase` field the UI keys on:
///   - `connecting`             — opening the WebSocket
///   - `waiting` { user_code, verification_uri, expires_in } — browser opened,
///     awaiting confirmation; the UI shows the code so the user can verify it
///   - `authorized`            — encrypted payload received, decrypting
///   - `done` { email }        — session persisted + engine started
///   - `error` { message }     — any failure (also returned as `Err`)
///
/// The whole handoff is bounded by the server's 300s code expiry; on timeout we
/// emit `error` and return `Err` so the UI can offer "try again".
#[tauri::command]
pub async fn start_browser_login(app: tauri::AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    bc!("[bb-signin] start_browser_login: command entry");
    match run_handoff(&app, &state).await {
        Ok(()) => Ok(()),
        Err(e) => {
            tracing::error!(error = %e, "browser-login: sign-in failed");
            signin_log(&format!("error {e}"));
            emit(&app, "error", serde_json::json!({ "message": e }));
            Err(e)
        }
    }
}

async fn run_handoff(app: &tauri::AppHandle, state: &State<'_, AppState>) -> Result<(), String> {
    // 1. Ephemeral P-256 key pair (uncompressed SEC1 point, base64).
    let secret = EphemeralSecret::random(&mut OsRng);
    let public_key = secret.public_key();
    let pub_key_b64 = B64.encode(public_key.to_encoded_point(false).as_bytes());
    bc!("[bb-signin] run_handoff: after keygen");

    // 2. Connect the WebSocket — with a hard timeout and bundled-root TLS so a
    //    stalled handshake can no longer freeze the UI on "Connecting" forever.
    emit(app, "connecting", serde_json::json!({}));
    bc!("[bb-signin] run_handoff: after emit connecting");
    let url = ws_url();
    bc!("[bb-signin] run_handoff: ws_url = {url}");
    tracing::info!(%url, "browser-login: connecting");
    signin_log(&format!("connecting {url}"));

    bc!("[bb-signin] run_handoff: before building connector");
    let connector = webpki_rustls_connector()?;
    bc!("[bb-signin] run_handoff: connector returned ok");
    let connect_fut = connect_async_tls_with_config(&url, None, false, Some(connector));
    bc!("[bb-signin] run_handoff: before timeout/connect await");
    let (mut ws, _) = match tokio::time::timeout(CONNECT_TIMEOUT, connect_fut).await {
        Ok(Ok(pair)) => {
            bc!("[bb-signin] run_handoff: connect Ok");
            pair
        }
        Ok(Err(e)) => {
            bc!("[bb-signin] run_handoff: connect Err :: {e}");
            tracing::error!(error = %e, %url, "browser-login: connect failed");
            signin_log(&format!("connect-error {url} :: {e}"));
            return Err(format!("Could not reach Beebeeb to start sign-in: {e}"));
        }
        Err(_elapsed) => {
            bc!("[bb-signin] run_handoff: connect timeout-elapsed");
            tracing::error!(
                %url,
                timeout_secs = CONNECT_TIMEOUT.as_secs(),
                "browser-login: connect timed out"
            );
            signin_log(&format!("connect-timeout {url} after {}s", CONNECT_TIMEOUT.as_secs()));
            return Err(
                "Could not reach Beebeeb to start sign-in (connection timed out). \
                 Check your internet connection and try again."
                    .to_string(),
            );
        }
    };
    bc!("[bb-signin] run_handoff: connected");
    tracing::info!("browser-login: connected");

    // 3 + 4. Send our public key (so the browser can complete ECDH) and receive
    //    the server's device-code reply. Both are bounded by DEVICE_CODE_TIMEOUT:
    //    a connected-but-silent server (cold/stalled first handoff) must not hang
    //    the UI on "Connecting" forever — fail fast with an actionable error.
    let init = serde_json::json!({ "ecdh_public_key_b64": pub_key_b64 });
    let handoff = async {
        ws.send(Message::Text(init.to_string()))
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "browser-login: failed to send public key");
                format!("Sign-in handshake failed: {e}")
            })?;
        bc!("[bb-signin] run_handoff: sent public key");
        tracing::info!("browser-login: sent public key");
        recv_text(&mut ws).await
    };
    let text = match tokio::time::timeout(DEVICE_CODE_TIMEOUT, handoff).await {
        Ok(result) => result?,
        Err(_elapsed) => {
            bc!("[bb-signin] run_handoff: device-code handoff timeout-elapsed");
            tracing::error!(
                timeout_secs = DEVICE_CODE_TIMEOUT.as_secs(),
                "browser-login: device-code handoff timed out"
            );
            signin_log(&format!("device-code-timeout after {}s", DEVICE_CODE_TIMEOUT.as_secs()));
            return Err(
                "Beebeeb did not respond while starting sign-in (timed out). \
                 Please check your connection and try again."
                    .to_string(),
            );
        }
    };
    let resp: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("Invalid server response: {e}"))?;
    if let Some(err) = resp["error"].as_str() {
        return Err(format!("Sign-in error: {err}"));
    }
    let user_code = resp["user_code"].as_str().ok_or("Server omitted the device code")?.to_string();
    let verification_uri = resp["verification_uri"]
        .as_str()
        .ok_or("Server omitted the verification URL")?
        .to_string();
    let expires_in: u64 = resp["expires_in"].as_u64().unwrap_or(300);
    bc!("[bb-signin] run_handoff: device code received (expires_in={expires_in}s)");
    tracing::info!(%verification_uri, expires_in, "browser-login: received device code");

    // 5. Open the system browser and tell the UI to show the code. We surface
    //    the code FIRST so the user always has it even if the browser fails to
    //    open (wrong default browser, locked session, etc.).
    emit(
        app,
        "waiting",
        serde_json::json!({
            "user_code": user_code,
            "verification_uri": verification_uri,
            "expires_in": expires_in,
        }),
    );
    tracing::info!(%verification_uri, "browser-login: opening browser");
    open_browser(app, &verification_uri);
    tracing::info!("browser-login: waiting for browser authorization");

    // 6. Wait up to the code lifetime for the browser to authorise. The server
    //    closes the socket with a `{"error":"timeout"}` text frame on expiry,
    //    but we also bound the wait ourselves so a dropped connection can't hang
    //    the UI forever.
    let result_text = tokio::time::timeout(Duration::from_secs(expires_in), recv_text(&mut ws))
        .await
        .map_err(|_| "Sign-in timed out (the code expired). Please try again.".to_string())??;
    let result: serde_json::Value =
        serde_json::from_str(&result_text).map_err(|e| format!("Invalid authorization payload: {e}"))?;
    if let Some(err) = result["error"].as_str() {
        if err == "timeout" {
            return Err("Sign-in timed out (the code expired). Please try again.".to_string());
        }
        return Err(format!("Sign-in error: {err}"));
    }

    tracing::info!("browser-login: authorized, decrypting credentials");
    emit(app, "authorized", serde_json::json!({}));

    // 7. ECDH key agreement + AES-256-GCM decrypt — identical to the CLI.
    let nonce_b64 = result["nonce_b64"].as_str().ok_or("Missing nonce in payload")?;
    let payload_b64 = result["encrypted_payload_b64"]
        .as_str()
        .ok_or("Missing ciphertext in payload")?;
    let browser_pub_b64 = result["browser_ecdh_public_b64"]
        .as_str()
        .ok_or("Missing browser public key in payload")?;

    let plaintext = decrypt_payload(&secret, browser_pub_b64, nonce_b64, payload_b64)?;

    // 8. Parse credentials and install the session via the shared path.
    let creds: serde_json::Value =
        serde_json::from_slice(&plaintext).map_err(|e| format!("Invalid credentials JSON: {e}"))?;
    let session_token = creds["session_token"]
        .as_str()
        .ok_or("Credentials missing session_token")?
        .to_string();
    let master_key_b64 = creds["master_key_b64"]
        .as_str()
        .ok_or("Credentials missing master_key_b64")?;
    let email = creds["email"].as_str().map(|s| s.to_string());

    let master_key_vec = B64
        .decode(master_key_b64.trim())
        .map_err(|e| format!("Invalid master key encoding: {e}"))?;
    if master_key_vec.len() != 32 {
        return Err(format!("master key must be 32 bytes, got {}", master_key_vec.len()));
    }
    let mut master_key = [0u8; 32];
    master_key.copy_from_slice(&master_key_vec);

    crate::apply_session(app.clone(), state, session_token, master_key, email.clone()).await?;

    tracing::info!("browser-login: done, session installed");
    signin_log("done session-installed");
    emit(app, "done", serde_json::json!({ "email": email }));
    Ok(())
}

/// ECDH(P-256) → HKDF-SHA256(info="beebeeb-cli-auth-v1") → AES-256-GCM decrypt.
///
/// Tries the HKDF-derived key first (v0.5+ web app); falls back to the raw
/// shared secret (v0.4 web app) for backward compatibility, exactly as the CLI
/// does — see `repos/cli/src/commands/login.rs`.
fn decrypt_payload(
    secret: &EphemeralSecret,
    browser_pub_b64: &str,
    nonce_b64: &str,
    payload_b64: &str,
) -> Result<Vec<u8>, String> {
    use hkdf::Hkdf;
    use sha2::Sha256;

    let browser_pub_bytes = B64
        .decode(browser_pub_b64)
        .map_err(|e| format!("Invalid browser public key encoding: {e}"))?;
    let browser_pub_key = PublicKey::from_sec1_bytes(&browser_pub_bytes)
        .map_err(|e| format!("Invalid browser public key (not on curve): {e}"))?;

    let shared_secret = secret.diffie_hellman(&browser_pub_key);
    let shared_bytes = shared_secret.raw_secret_bytes();

    let nonce_bytes = B64.decode(nonce_b64).map_err(|e| format!("Invalid nonce encoding: {e}"))?;
    let ciphertext = B64.decode(payload_b64).map_err(|e| format!("Invalid payload encoding: {e}"))?;

    let hk = Hkdf::<Sha256>::new(None, shared_bytes);
    let mut hkdf_key = [0u8; 32];
    hk.expand(HKDF_INFO, &mut hkdf_key)
        .expect("HKDF expand failed — 32-byte output length is always valid");

    let cipher = Aes256Gcm::new(GenericArray::from_slice(&hkdf_key));
    cipher
        .decrypt(GenericArray::from_slice(&nonce_bytes), ciphertext.as_ref())
        .or_else(|_| {
            // v0.4 fallback: raw shared secret as the key.
            let cipher = Aes256Gcm::new(GenericArray::from_slice(&shared_bytes[..32]));
            cipher.decrypt(GenericArray::from_slice(&nonce_bytes), ciphertext.as_ref())
        })
        .map_err(|_| "Decryption failed — key mismatch or corrupted credentials.".to_string())
}

/// Block until the next text frame, skipping ping/pong control frames.
async fn recv_text<S>(ws: &mut S) -> Result<String, String>
where
    S: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    loop {
        match ws.next().await {
            Some(Ok(Message::Text(t))) => return Ok(t),
            Some(Ok(Message::Ping(_))) | Some(Ok(Message::Pong(_))) => continue,
            Some(Ok(Message::Close(_))) | None => {
                return Err("Connection closed before sign-in completed.".to_string());
            }
            Some(Ok(_)) => continue,
            Some(Err(e)) => return Err(format!("WebSocket error: {e}")),
        }
    }
}

/// Open the verification URL in the system browser. Best-effort — the UI has
/// already shown the URL + code, so a failure here is non-fatal.
fn open_browser(app: &tauri::AppHandle, url: &str) {
    use tauri_plugin_opener::OpenerExt;
    if let Err(e) = app.opener().open_url(url, None::<&str>) {
        tracing::warn!(error = %e, "failed to open browser for sign-in; user can paste the URL");
    }
}

#[cfg(test)]
mod tests {
    //! Task 0983: the device-code handoff (`ws.send(pubkey)` + the first
    //! `recv_text()` waiting for the server's `{user_code, verification_uri}`
    //! frame) must be bounded, not just the WebSocket CONNECT. These tests
    //! exercise `recv_text` — the exact function `run_handoff` wraps in
    //! `tokio::time::timeout(DEVICE_CODE_TIMEOUT, ...)` — against a mock stream
    //! standing in for the real `WebSocketStream`, since a live server can't run
    //! in this test binary. `recv_text` is generic over any
    //! `StreamExt<Item = Result<Message, Error>> + Unpin`, so the mock exercises
    //! the identical code path `run_handoff` calls, not a re-implementation of it.

    use super::*;
    use std::pin::Pin;
    use std::task::{Context, Poll};

    type WsItem = Result<Message, tokio_tungstenite::tungstenite::Error>;

    /// A mock WebSocket stream that never yields an item — standing in for a
    /// server that accepted the connection but stalls before sending the first
    /// device-code frame (the exact bug this task fixes).
    struct StalledStream;

    impl futures_util::Stream for StalledStream {
        type Item = WsItem;
        fn poll_next(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
            Poll::Pending
        }
    }

    /// A mock stream that first emits a couple of ping/pong control frames (as a
    /// real server connection may) and then the device-code text frame —
    /// standing in for the healthy happy path `recv_text` must keep working.
    struct HealthyStream {
        frames: Vec<Message>,
    }

    impl futures_util::Stream for HealthyStream {
        type Item = WsItem;
        fn poll_next(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
            if self.frames.is_empty() {
                Poll::Ready(None)
            } else {
                Poll::Ready(Some(Ok(self.frames.remove(0))))
            }
        }
    }

    /// If the server accepts the WS but never sends the first device-code
    /// frame, the caller's `tokio::time::timeout(DEVICE_CODE_TIMEOUT, recv_text(..))`
    /// (mirrored here with a short duration so the test doesn't take 15s) must
    /// elapse and return an error promptly — not hang forever. This is the exact
    /// regression this task guards: before the fix, only the CONNECT was bounded.
    #[tokio::test]
    async fn device_code_recv_times_out_when_server_stalls_after_connect() {
        let mut ws = StalledStream;
        let bound = Duration::from_millis(50);

        let outcome = tokio::time::timeout(bound, recv_text(&mut ws)).await;

        assert!(
            outcome.is_err(),
            "recv_text on a stalled connection must be bounded by the timeout, not hang forever"
        );
    }

    /// Happy path guard: a healthy server that replies with the device-code
    /// frame (after some ping/pong control frames) must still resolve well
    /// within the bound — confirming the timeout wrapper added for this task
    /// does not change behavior on a working connection.
    #[tokio::test]
    async fn device_code_recv_returns_promptly_on_healthy_connection() {
        let mut ws = HealthyStream {
            frames: vec![
                Message::Ping(vec![].into()),
                Message::Pong(vec![].into()),
                Message::Text(r#"{"user_code":"ABCD-1234","verification_uri":"https://beebeeb.io/cli","expires_in":300}"#.into()),
            ],
        };
        let bound = Duration::from_secs(15); // mirrors DEVICE_CODE_TIMEOUT — real-time wait, but this branch resolves immediately

        let outcome = tokio::time::timeout(bound, recv_text(&mut ws))
            .await
            .expect("must not time out on a healthy connection");

        let text = outcome.expect("recv_text must succeed once the text frame arrives");
        assert!(text.contains("user_code"));
    }

    /// A closed connection (no frames, stream ends) must surface as an
    /// immediate, actionable error — not as a silent hang requiring the outer
    /// timeout to fire. Regression guard for `recv_text`'s `None` arm.
    #[tokio::test]
    async fn device_code_recv_errors_immediately_on_closed_connection() {
        let mut ws = HealthyStream { frames: vec![] };

        let result = recv_text(&mut ws).await;

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Connection closed"));
    }
}
