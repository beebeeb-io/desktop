//! Slice K5 (batch 2): sign in on this PC by approving it on another device (DW-A07 to DW-A10). The requesting side
//! of the `/api/v1/auth/cli` handoff, for a host without a UI toolkit (the native sign-in screen).
//!
//! The steps are the Tauri shell's `browser_login::run_handoff` (its rules, and its tests ported as `device_request_*`),
//! on this handle's runtime and with this handle's install:
//! 1. the session's turn is the one the host read AT ITS PRESS, checked before the key exists: a press whose session
//!    moved on starts nothing, and the install later claims that same turn (`Overtaken` when it moved);
//! 2. a fresh P-256 key (core's `CliEphemeralKey`), the socket ([`ws_url`]: the API base's `wss://`), and the first frame
//!    ([`init_frame`]: the public key and what this PC can say about itself; never a code, none exists yet);
//! 3. the server's `{user_code, verification_uri, expires_in}`: the code is shown on this PC and TYPED on the other one;
//!    the address opens the request there and holds no code (task 1734; an address that holds it is refused here);
//! 4. the wait, counted down here once a second ([`DeviceRequestEvent::Tick`]): the other device's answer, the code's
//!    end or the host's stop, whichever comes first (the stop is raced against every wait);
//! 5. the sealed payload, opened with core's `decrypt_cli_payload` (`beebeeb-cli-auth-v1`): only a payload that opens is
//!    read. Then the install, in the engine's step: the account behind the arriving session is checked against the
//!    session stored here and against this PC's local data (another account's: `AccountMismatch`, nothing stored); the
//!    session goes through the password sign-in's own install (`finish_sign_in`), and the handed-over key is kept as a
//!    passkey's is (`passkeys_api::keep_handed_over_key`, the one helper both use).
//!
//! Every way out before the install stores nothing, and a session the server made for a request that is not installed
//! is ended at the server (best effort).
//!
//! The server's answers on the socket: `{"error":"denied"}` when the account's owner said no, `{"error":"timeout"}` at
//! the code's end, else the sealed payload. It names no approving device, so `by` is
//! [`ANOTHER_DEVICE`]. A socket that drops cannot be joined again (a new socket is a new code): the request ends.
//!
//! Threads: [`Engine::device_request_for`] BLOCKS until the request ends (up to the code's five minutes). Call it from a
//! thread of its own, never from the window's thread and never from a task of this handle's runtime. It holds the
//! engine's step lock only for the install, never while it waits for the other device.
//!
//! Must not: log, print, return or put in an event the payload, the session token or the master key; log the code; keep
//! the ephemeral key beyond the call.

use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use beebeeb_core::CoreError;
use beebeeb_core::auth_flow::device::DEVICE_SOCKET_PATH;
use beebeeb_core::cli_auth::{CliEphemeralKey, decrypt_cli_payload};
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::{Connector, connect_async_tls_with_config};
use zeroize::Zeroizing;

use crate::account::SessionGeneration;
use crate::account_binding::{self, Binding, BindingKind, Identity};
use crate::account_dto::AccountProfile;
use crate::app::{CredentialStores, Engine};
use crate::state_db::{ReadOnlyOpenError, StateDb};
use crate::state_paths;

/// The name a request ends with when the server names no approving device (today's server never does).
pub const ANOTHER_DEVICE: &str = "another device";

/// Hard ceiling on the socket's connect (TCP and TLS). Ported from `browser_login::CONNECT_TIMEOUT`: a stalled TLS
/// handshake once left the window on "Connecting" with no end.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// Hard ceiling on the first round trip (the public key out, the code back): a connected server that sends no code
/// must not hold the screen either (`browser_login::DEVICE_CODE_TIMEOUT`).
const DEVICE_CODE_TIMEOUT: Duration = Duration::from_secs(15);

/// The code's life when the server names none (five minutes, what it names today).
const DEFAULT_EXPIRES_IN: u64 = 300;

/// The longest wait this PC keeps a request open, whatever the server says.
const MAX_WAIT_SECS: u64 = 900;

/// How often a wait looks at the host's stop: a cancel ends the wait within this.
const STOP_POLL: Duration = Duration::from_millis(25);

/// One step of the countdown.
const TICK: Duration = Duration::from_secs(1);

/// What a running request tells the host, in this order: the code once, then a tick each second while it waits;
/// `Connection(false)` when the socket dropped, right before the request ends.
#[derive(Clone, PartialEq, Eq)]
pub enum DeviceRequestEvent {
    /// The server issued a code: the host shows it with a QR code of `verification_uri` (which holds no code).
    Code {
        code: String,
        verification_uri: String,
        expires_in: u64,
    },
    /// One second passed; `0` is the code's end.
    Tick { seconds_left: u32 },
    /// The socket is up (`true`) or dropped (`false`).
    Connection(bool),
}

impl std::fmt::Debug for DeviceRequestEvent {
    /// Without the code: it is shown to the person, never written to a log.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Code { expires_in, .. } => write!(f, "Code {{ expires_in: {expires_in} }}"),
            Self::Tick { seconds_left } => write!(f, "Tick {{ seconds_left: {seconds_left} }}"),
            Self::Connection(up) => write!(f, "Connection({up})"),
        }
    }
}

/// The request ended with this PC signed in (DW-A08).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceSignedIn {
    /// The approving device's name as the server sent it, else [`ANOTHER_DEVICE`].
    pub by: String,
    /// The vault is open on this PC. `false` only when the credential store refused the handed-over key: the session
    /// is stored, and the gate asks for the key as it does after a password sign-in (a passkey's install does the same).
    pub vault_unlocked: bool,
}

/// How a request ended without a sign-in. Nothing was stored for any of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceRequestRefused {
    /// Beebeeb could not be reached, or did not answer in time, before a code was shown.
    Offline,
    /// The server refused to start a request, or answered in a shape this app does not read.
    Failed,
    /// The socket dropped while the code was shown. The request is lost: a new socket is a new code.
    ConnectionLost,
    /// DW-A09: the account's owner said no on the other device.
    Denied { by: String },
    /// DW-A10: the code's time ran out.
    Expired,
    /// DW-A08's error state: an answer arrived and did not open with this request's key.
    KeysNotReceived { by: String },
    /// The host's stop was seen.
    Stopped,
    /// The arriving account is not the one whose session is stored here, or not the one this PC's local data belongs
    /// to (the binding's rule, `account_binding::decide`). The new session was ended at the server.
    AccountMismatch,
    /// The session's turn moved since the press (a sign-in, a lock, a sign-out), or this PC is open already.
    Overtaken,
    /// The arriving session could not be stored: its account could not be read, or the credential store refused.
    NotStored,
}

impl DeviceRequestRefused {
    /// The refusal's name, for a log line: the kind only.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Offline => "offline",
            Self::Failed => "failed",
            Self::ConnectionLost => "connection_lost",
            Self::Denied { .. } => "denied",
            Self::Expired => "expired",
            Self::KeysNotReceived { .. } => "keys_not_received",
            Self::Stopped => "stopped",
            Self::AccountMismatch => "account_mismatch",
            Self::Overtaken => "overtaken",
            Self::NotStored => "not_stored",
        }
    }
}

/// What arrived from the approving device, opened. Every secret wipes on drop; no `Debug`.
struct Received {
    token: Zeroizing<String>,
    key: Zeroizing<[u8; 32]>,
    by: String,
}

/// The code the server issued, as read from its first answer.
struct IssuedCode {
    code: String,
    verification_uri: String,
    expires_in: u64,
}

/// This PC's name as the init frame reports it (and as a host shows it, "asking as"): the machine's host name, trimmed.
/// `None` when it has none; the frame then carries no name.
pub fn device_request_name() -> Option<String> {
    hostname::get()
        .ok()
        .and_then(|name| name.into_string().ok())
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
}

/// The socket's address from an API base: `https://` becomes `wss://` (production), `http://` becomes `ws://` (a
/// loopback base in a test or a development build).
pub fn ws_url(api_base: &str) -> String {
    let base = api_base.trim().trim_end_matches('/');
    let ws = base.replacen("https://", "wss://", 1).replacen("http://", "ws://", 1);
    format!("{ws}{DEVICE_SOCKET_PATH}")
}

impl<C: CredentialStores> Engine<C> {
    /// Ask to be signed in by another device, for the session turn `turn` the host read at its press (see the module's
    /// doc). `events` hears the code, the ticks and a dropped socket; `stop` (set by the host) ends the request within
    /// [`STOP_POLL`] and stores nothing. BLOCKS until the request ends: call it from a thread of its own.
    pub fn device_request_for(
        &self,
        turn: SessionGeneration,
        events: impl Fn(DeviceRequestEvent) + Send + 'static,
        stop: Arc<AtomicBool>,
    ) -> Result<DeviceSignedIn, DeviceRequestRefused> {
        let url = ws_url(&self.config.api_base);
        let result = self.device_request_at(&url, turn, &events, &stop);
        match &result {
            Ok(_) => tracing::info!("device request approved, signed in on this PC"),
            Err(refused) => tracing::info!(kind = refused.kind(), "device request ended, nothing signed in"),
        }
        self.publish();
        result
    }

    /// [`Self::device_request_for`] on the socket at `url`: the one place the steps are written (a test points it at a
    /// loopback socket of its own, beside the fake account's HTTP base).
    fn device_request_at(
        &self,
        url: &str,
        turn: SessionGeneration,
        events: &dyn Fn(DeviceRequestEvent),
        stop: &AtomicBool,
    ) -> Result<DeviceSignedIn, DeviceRequestRefused> {
        // The turn first, before the key exists: a request belongs to the session of its press.
        if !self.acct.session_unchanged_since(turn) {
            return Err(DeviceRequestRefused::Overtaken);
        }
        if stop.load(Ordering::SeqCst) {
            return Err(DeviceRequestRefused::Stopped);
        }
        let secret = CliEphemeralKey::generate();
        let received = self.runtime.block_on(handoff(url, &secret, events, stop));
        // The ephemeral key has done its work (it opened the payload, or nothing arrived): it goes now.
        drop(secret);
        let received = received?;
        self.runtime.block_on(self.install_device_session(turn, received, stop))
    }

    /// The arriving session, in the engine's step and the press's turn (module doc, step 5). A refusal ends the new
    /// session at the server.
    async fn install_device_session(
        &self,
        turn: SessionGeneration,
        received: Received,
        stop: &AtomicBool,
    ) -> Result<DeviceSignedIn, DeviceRequestRefused> {
        let Received { token, key, by } = received;
        let _step = self.transition.lock().await;
        // After the wait for the step: the host's stop, and the session's turn as the press read it.
        if stop.load(Ordering::SeqCst) {
            self.end_unstored(&token).await;
            return Err(DeviceRequestRefused::Stopped);
        }
        if !self.acct.session_unchanged_since(turn) || self.unlocked() {
            self.end_unstored(&token).await;
            return Err(DeviceRequestRefused::Overtaken);
        }
        let profile = match crate::app::fetch_profile(&self.client, &self.config.api_base, &token).await {
            Ok(profile) => profile,
            Err(_) => {
                self.end_unstored(&token).await;
                return Err(DeviceRequestRefused::NotStored);
            }
        };
        match self.arriving_account_fits(&profile) {
            Ok(true) => {}
            Ok(false) => {
                self.end_unstored(&token).await;
                return Err(DeviceRequestRefused::AccountMismatch);
            }
            Err(()) => {
                self.end_unstored(&token).await;
                return Err(DeviceRequestRefused::NotStored);
            }
        }
        // The password sign-in's install, claimed in the press's turn (`store_first_sign_in` refuses a turn that
        // moved). It ends the new session at the server itself on every refusal.
        let email = profile.email.trim().to_string();
        if self
            .finish_sign_in(turn, Zeroizing::new(token.as_str().to_owned()), email)
            .await
            .is_err()
        {
            return Err(DeviceRequestRefused::NotStored);
        }
        // The ONE key-storing step of a sign-in that hands a key over (the passkey's too): `passkeys_api`.
        let vault_unlocked = self.keep_handed_over_key(&token, &key);
        Ok(DeviceSignedIn { by, vault_unlocked })
    }

    /// Whether the account behind the arriving session may be installed here. `Ok(false)`: a session of another account
    /// is stored (or one whose account cannot be told), or this PC's local data is another account's (or unowned with
    /// data: the binding's `Reset`, or an owner it cannot compare: `Refuse`). `Err`: the store or the local data could
    /// not be read. Reads only; never resets, adopts or records an owner.
    fn arriving_account_fits(&self, profile: &AccountProfile) -> Result<bool, ()> {
        let arriving = Identity::new(Some(profile.user_id.as_str()), Some(profile.email.as_str()));
        if !arriving.is_known() {
            return Err(());
        }
        let vault = self.vault();
        if vault.session_token().map_err(|_| ())?.is_some() {
            let stored = Identity::new(
                vault.session_owner().map_err(|_| ())?.as_deref(),
                vault.account_email().map_err(|_| ())?.as_deref(),
            );
            if account_binding::same_account(&stored, &arriving) != Some(true) {
                return Ok(false);
            }
        }
        let path = self.device_local_state_path();
        if !path.exists() {
            return Ok(true);
        }
        let db = match StateDb::open_read_only(&path) {
            Ok(db) => db,
            Err(ReadOnlyOpenError::NotFound) => return Ok(true),
            Err(_) => return Err(()),
        };
        let owner = db.owner().map_err(|_| ())?.filter(Identity::is_known);
        let kind = BindingKind::of(db.has_account_data().map_err(|_| ())?);
        Ok(account_binding::decide(owner.as_ref(), &arriving, kind) == Binding::Proceed)
    }

    /// Where this PC's local state database is: the one `owned_state` reads (a test's own database when it named one).
    fn device_local_state_path(&self) -> PathBuf {
        #[cfg(any(test, feature = "test-support"))]
        if let Some(path) = self
            .local_state_for_test
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
        {
            return path;
        }
        state_paths::state_db_path_from_state_dir(&self.config.state_dir)
    }

    /// End a session the server made for a request that is not installed. Best effort; never logs.
    async fn end_unstored(&self, token: &str) {
        crate::app::revoke(&self.client, &self.config.api_base, token).await;
    }
}

/// Steps 2 to 5 up to the opened payload (module doc). Stores nothing.
async fn handoff(
    url: &str,
    secret: &CliEphemeralKey,
    events: &dyn Fn(DeviceRequestEvent),
    stop: &AtomicBool,
) -> Result<Received, DeviceRequestRefused> {
    let request = build_ws_request(url)?;
    let connector = webpki_rustls_connector()?;
    let connect = connect_async_tls_with_config(request, None, false, Some(connector));
    let mut ws = match or_stop(stop, tokio::time::timeout(CONNECT_TIMEOUT, connect)).await? {
        Ok(Ok((ws, _response))) => ws,
        Ok(Err(_)) | Err(_) => return Err(DeviceRequestRefused::Offline),
    };
    let init = init_frame(&ephemeral_public_b64(secret), device_request_name());
    let first = async {
        ws.send(Message::Text(init.to_string()))
            .await
            .map_err(|_| DeviceRequestRefused::Offline)?;
        recv_text(&mut ws).await.map_err(|_| DeviceRequestRefused::Offline)
    };
    let text = match or_stop(stop, tokio::time::timeout(DEVICE_CODE_TIMEOUT, first)).await? {
        Ok(answer) => answer?,
        Err(_elapsed) => return Err(DeviceRequestRefused::Offline),
    };
    let issued = read_code(&text)?;
    tracing::info!(
        expires_in = issued.expires_in,
        "device request waiting for the other device"
    );
    events(DeviceRequestEvent::Code {
        code: issued.code.clone(),
        verification_uri: issued.verification_uri.clone(),
        expires_in: issued.expires_in,
    });
    // The wait: this PC's clock counts the code down; the stop is raced against every receive.
    let started = tokio::time::Instant::now();
    let deadline = started + Duration::from_secs(issued.expires_in);
    let mut ticks = tokio::time::interval_at(started + TICK, TICK);
    let answer = loop {
        tokio::select! {
            biased;
            () = stopped(stop) => return Err(DeviceRequestRefused::Stopped),
            _ = ticks.tick() => {
                let left = seconds_left(deadline, tokio::time::Instant::now());
                events(DeviceRequestEvent::Tick { seconds_left: left });
                if left == 0 {
                    return Err(DeviceRequestRefused::Expired);
                }
            }
            frame = recv_text(&mut ws) => match frame {
                Ok(text) => break text,
                Err(_) => {
                    events(DeviceRequestEvent::Connection(false));
                    return Err(DeviceRequestRefused::ConnectionLost);
                }
            },
        }
    };
    read_answer(secret, &answer)
}

/// `work`, unless the host's stop comes first.
async fn or_stop<T>(stop: &AtomicBool, work: impl Future<Output = T>) -> Result<T, DeviceRequestRefused> {
    tokio::select! {
        biased;
        () = stopped(stop) => Err(DeviceRequestRefused::Stopped),
        out = work => Ok(out),
    }
}

/// Ready once the host's stop is set; looks every [`STOP_POLL`].
async fn stopped(stop: &AtomicBool) {
    while !stop.load(Ordering::SeqCst) {
        tokio::time::sleep(STOP_POLL).await;
    }
}

/// Whole seconds until `deadline`, rounded to the nearest (a tick lands on a second boundary, a little late).
fn seconds_left(deadline: tokio::time::Instant, now: tokio::time::Instant) -> u32 {
    let left = deadline.saturating_duration_since(now);
    u32::try_from(left.as_millis().saturating_add(500) / 1000).unwrap_or(u32::MAX)
}

/// The server's first answer: the code, the address and the code's life. An error, a missing field, an address that is
/// not a web page or one that holds the code (task 1734): `Failed`.
fn read_code(text: &str) -> Result<IssuedCode, DeviceRequestRefused> {
    let frame: serde_json::Value = serde_json::from_str(text).map_err(|_| DeviceRequestRefused::Failed)?;
    if frame.get("error").is_some() {
        return Err(DeviceRequestRefused::Failed);
    }
    let field = |name: &str| {
        frame
            .get(name)
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    };
    let code = field("user_code").ok_or(DeviceRequestRefused::Failed)?;
    let verification_uri = field("verification_uri").ok_or(DeviceRequestRefused::Failed)?;
    if !(verification_uri.starts_with("https://") || verification_uri.starts_with("http://")) {
        return Err(DeviceRequestRefused::Failed);
    }
    if holds_code(&verification_uri, &code) {
        return Err(DeviceRequestRefused::Failed);
    }
    let expires_in = frame
        .get("expires_in")
        .and_then(|value| value.as_u64())
        .unwrap_or(DEFAULT_EXPIRES_IN)
        .clamp(1, MAX_WAIT_SECS);
    Ok(IssuedCode {
        code,
        verification_uri,
        expires_in,
    })
}

/// Whether `address` carries `code`, with or without its dash, in any letter case.
fn holds_code(address: &str, code: &str) -> bool {
    let fold = |text: &str| -> String {
        text.chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .map(|c| c.to_ascii_uppercase())
            .collect()
    };
    let code = fold(code);
    !code.is_empty() && fold(address).contains(&code)
}

/// The answer that ends the wait: the owner's no, the code's end, or the sealed payload (`browser_login::run_handoff`'s
/// step 7, with the server's `denied`). A payload is read only once it opens with this request's key; anything else
/// that is not an error is `KeysNotReceived`.
fn read_answer(secret: &CliEphemeralKey, text: &str) -> Result<Received, DeviceRequestRefused> {
    let not_received = || DeviceRequestRefused::KeysNotReceived {
        by: ANOTHER_DEVICE.to_string(),
    };
    let Ok(frame) = serde_json::from_str::<serde_json::Value>(text) else {
        return Err(not_received());
    };
    if let Some(error) = frame.get("error").and_then(|value| value.as_str()) {
        return Err(match error {
            "denied" => DeviceRequestRefused::Denied {
                by: ANOTHER_DEVICE.to_string(),
            },
            "timeout" | "expired" => DeviceRequestRefused::Expired,
            _ => DeviceRequestRefused::Failed,
        });
    }
    let field = |name: &str| frame.get(name).and_then(|value| value.as_str());
    let (Some(nonce), Some(payload), Some(approver)) = (
        field("nonce_b64"),
        field("encrypted_payload_b64"),
        field("browser_ecdh_public_b64"),
    ) else {
        return Err(not_received());
    };
    let plaintext = Zeroizing::new(decrypt_payload(secret, approver, nonce, payload).map_err(|_| not_received())?);
    let (token, key) = read_credentials(&plaintext).ok_or_else(not_received)?;
    Ok(Received {
        token,
        key,
        by: ANOTHER_DEVICE.to_string(),
    })
}

/// `{session_token, master_key_b64}` from an opened payload (the email the web also sends is not read: it is that
/// client's text, not an identity; the account is the server's record of the token). A key that is not 32 bytes, or
/// an empty token: `None`.
fn read_credentials(plaintext: &[u8]) -> Option<(Zeroizing<String>, Zeroizing<[u8; 32]>)> {
    #[derive(serde::Deserialize, zeroize::Zeroize, zeroize::ZeroizeOnDrop)]
    struct Credentials {
        session_token: String,
        master_key_b64: String,
    }
    let credentials: Credentials = serde_json::from_slice(plaintext).ok()?;
    let decoded = Zeroizing::new(B64.decode(credentials.master_key_b64.trim()).ok()?);
    if decoded.len() != 32 || credentials.session_token.trim().is_empty() {
        return None;
    }
    let mut key = Zeroizing::new([0u8; 32]);
    key.copy_from_slice(&decoded);
    Some((Zeroizing::new(credentials.session_token.clone()), key))
}

/// The version this build reports: the one every request's `X-Beebeeb-Client-Version` header carries.
fn client_version() -> String {
    crate::api_client::provenance_headers()
        .get("x-beebeeb-client-version")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string()
}

/// The first frame: the public key (so the approving device can do its half of the ECDH) and what this PC can honestly
/// say about itself, which the approval page shows labelled "reported by the device". Never a code (none exists yet).
fn init_frame(pub_key_b64: &str, device_name: Option<String>) -> serde_json::Value {
    let mut frame = serde_json::json!({
        "ecdh_public_key_b64": pub_key_b64,
        "client": "desktop",
        "client_version": client_version(),
        "os": std::env::consts::OS,
    });
    if let Some(name) = device_name
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
    {
        frame["device_name"] = serde_json::Value::String(name);
    }
    frame
}

/// The socket's upgrade request, with the writer-provenance headers every other request of this app carries
/// (`browser_login::build_ws_request`, task 1436).
fn build_ws_request(
    url: &str,
) -> Result<tokio_tungstenite::tungstenite::handshake::client::Request, DeviceRequestRefused> {
    let mut request = url.into_client_request().map_err(|_| DeviceRequestRefused::Failed)?;
    for (name, value) in crate::api_client::provenance_headers().iter() {
        request.headers_mut().insert(name.clone(), value.clone());
    }
    Ok(request)
}

/// A rustls connector on the bundled Mozilla roots with the `ring` provider, built panic-safe
/// (`browser_login::webpki_rustls_connector`: the OS store and `aws_lc_rs` stalled sign-ins on some Windows PCs, and
/// both providers are linked, so none is picked by default). A `ws://` address does not use it.
fn webpki_rustls_connector() -> Result<Connector, DeviceRequestRefused> {
    std::panic::catch_unwind(|| {
        let roots = rustls::RootCertStore {
            roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
        };
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let config = rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .ok()?
            .with_root_certificates(roots)
            .with_no_client_auth();
        Some(Connector::Rustls(Arc::new(config)))
    })
    .ok()
    .flatten()
    .ok_or(DeviceRequestRefused::Failed)
}

/// The public half of this request's key as the init frame carries it: the SEC1-uncompressed point, standard base64.
fn ephemeral_public_b64(secret: &CliEphemeralKey) -> String {
    B64.encode(secret.public_key_bytes())
}

/// ECDH (P-256), HKDF-SHA256 (`beebeeb-cli-auth-v1`) and AES-256-GCM, all of it core's `beebeeb_core::cli_auth`
/// (`browser_login::decrypt_payload`, ported). The errors name the step that failed and carry no key material.
fn decrypt_payload(
    secret: &CliEphemeralKey,
    approver_public_b64: &str,
    nonce_b64: &str,
    payload_b64: &str,
) -> Result<Vec<u8>, String> {
    let approver_public = B64
        .decode(approver_public_b64)
        .map_err(|e| format!("Invalid browser public key encoding: {e}"))?;
    let shared_secret = secret
        .shared_secret(&approver_public)
        .map_err(|_| "Invalid browser public key (not on curve).".to_string())?;
    let nonce = B64
        .decode(nonce_b64)
        .map_err(|e| format!("Invalid nonce encoding: {e}"))?;
    let ciphertext = B64
        .decode(payload_b64)
        .map_err(|e| format!("Invalid payload encoding: {e}"))?;
    decrypt_cli_payload(&shared_secret, &nonce, &ciphertext).map_err(|e| match e {
        CoreError::InvalidInput(_) => format!("Invalid nonce: {} bytes, expected 12.", nonce.len()),
        _ => "Decryption failed — key mismatch or corrupted credentials.".to_string(),
    })
}

/// The next text frame, skipping ping and pong (`browser_login::recv_text`). A closed socket is an error at once.
async fn recv_text<S>(ws: &mut S) -> Result<String, String>
where
    S: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    loop {
        match ws.next().await {
            Some(Ok(Message::Text(text))) => return Ok(text),
            Some(Ok(Message::Ping(_))) | Some(Ok(Message::Pong(_))) => continue,
            Some(Ok(Message::Close(_))) | None => {
                return Err("Connection closed before sign-in completed.".to_string());
            }
            Some(Ok(_)) => continue,
            Some(Err(e)) => return Err(format!("WebSocket error: {e}")),
        }
    }
}

#[cfg(test)]
#[path = "device_request_tests.rs"]
mod tests;
