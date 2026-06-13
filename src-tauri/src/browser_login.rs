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
use tokio_tungstenite::{connect_async, tungstenite::Message};

use crate::AppState;
use crate::runner;

/// HKDF `info` string — MUST match the CLI client and the web browser side.
const HKDF_INFO: &[u8] = b"beebeeb-cli-auth-v1";

/// Tauri event name the UI subscribes to for handoff progress.
const EVENT: &str = "browser-login";

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
    match run_handoff(&app, &state).await {
        Ok(()) => Ok(()),
        Err(e) => {
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

    // 2. Connect the WebSocket.
    emit(app, "connecting", serde_json::json!({}));
    let url = ws_url();
    let (mut ws, _) = connect_async(&url)
        .await
        .map_err(|e| format!("Could not reach Beebeeb to start sign-in: {e}"))?;

    // 3. Send our public key so the browser can complete ECDH on its end.
    let init = serde_json::json!({ "ecdh_public_key_b64": pub_key_b64 });
    ws.send(Message::Text(init.to_string()))
        .await
        .map_err(|e| format!("Sign-in handshake failed: {e}"))?;

    // 4. Receive the device code + verification URI.
    let text = recv_text(&mut ws).await?;
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
    open_browser(app, &verification_uri);

    // 6. Wait up to the code lifetime for the browser to authorise. The server
    //    closes the socket with a `{"error":"timeout"}` text frame on expiry,
    //    but we also bound the wait ourselves so a dropped connection can't hang
    //    the UI forever.
    let result_text = tokio::time::timeout(std::time::Duration::from_secs(expires_in), recv_text(&mut ws))
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
