//! "Sign in with another device", the requesting side (DW-A07 to DW-A10).
//!
//! `auth_device_request_start` opens the `/api/v1/auth/cli` socket, sends core's init frame (this PC's ephemeral P-256
//! key and what it can say about itself) and returns the code and the QR. A task then waits on the socket for the
//! approver's relay; core decrypts it (`DeviceRequest::on_frame`), and the session and the master key go straight into
//! the credential store through `apply_session` (which writes under `claim_session_write`). The web view only ever
//! sees the status and the code (crypto review rule 5). Leaving the screen (`auth_device_request_cancel`) closes the
//! socket and forgets the code.
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use beebeeb_core::auth_flow::AuthFlowError;
use beebeeb_core::auth_flow::device::{DeviceFacts, DeviceRequest, DeviceRequestEvent};
use futures_util::SinkExt;
use tauri::Manager;
use tokio_tungstenite::connect_async_tls_with_config;
use tokio_tungstenite::tungstenite::Message;

use crate::browser_login::{
    CLIENT_VERSION, CONNECT_TIMEOUT, build_ws_request, device_hostname, recv_text, webpki_rustls_connector, ws_url,
};

/// What the requesting screen shows. `qr_svg` is set by start only.
#[derive(Clone, Debug, serde::Serialize, PartialEq, Eq)]
pub(crate) struct DeviceRequestView {
    /// `waiting`, `reconnecting`, `approved`, `signed_in`, `denied`, `expired`, `keys_did_not_arrive`, `failed`.
    pub(crate) status: String,
    pub(crate) user_code: Option<String>,
    pub(crate) seconds_left: u64,
    /// The name the approver will be shown ("This device asks as …").
    pub(crate) device_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) qr_svg: Option<String>,
}

struct Slot {
    generation: u64,
    status: &'static str,
    user_code: Option<String>,
    expires_at: Instant,
    device_name: Option<String>,
    task: Option<tokio::task::AbortHandle>,
}

static SLOT: LazyLock<Mutex<Option<Slot>>> = LazyLock::new(|| Mutex::new(None));
static GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn view(slot: &Slot, qr_svg: Option<String>) -> DeviceRequestView {
    DeviceRequestView {
        status: slot.status.to_string(),
        user_code: slot.user_code.clone(),
        seconds_left: slot.expires_at.saturating_duration_since(Instant::now()).as_secs(),
        device_name: slot.device_name.clone(),
        qr_svg,
    }
}

fn set_status(generation: u64, status: &'static str) {
    if let Ok(mut guard) = SLOT.lock()
        && let Some(slot) = guard.as_mut()
        && slot.generation == generation
    {
        slot.status = status;
    }
}

fn status_for(e: &AuthFlowError) -> &'static str {
    match e {
        AuthFlowError::Expired => "expired",
        AuthFlowError::Denied => "denied",
        AuthFlowError::KeysDidNotArrive => "keys_did_not_arrive",
        _ => "failed",
    }
}

fn cancel_current() {
    if let Ok(mut guard) = SLOT.lock()
        && let Some(slot) = guard.take()
        && let Some(task) = slot.task
    {
        task.abort();
    }
}

/// Start a request: a new code every time (Ask again, Get a new code).
#[tauri::command]
pub(crate) async fn auth_device_request_start(app: tauri::AppHandle) -> Result<DeviceRequestView, String> {
    cancel_current();
    let generation = GENERATION.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
    let mut request = DeviceRequest::new();
    let device_name = device_hostname().map(|h| h.trim().to_string()).filter(|h| !h.is_empty());
    let facts = DeviceFacts {
        device_name: device_name.clone(),
        client: "desktop".into(),
        client_version: CLIENT_VERSION.into(),
        os: std::env::consts::OS.into(),
    };

    let connector = webpki_rustls_connector().map_err(|_| "offline".to_string())?;
    let ws_request = build_ws_request(&ws_url()).map_err(|_| "offline".to_string())?;
    let (mut ws, _) = tokio::time::timeout(CONNECT_TIMEOUT, connect_async_tls_with_config(ws_request, None, false, Some(connector)))
        .await
        .map_err(|_| "offline".to_string())?
        .map_err(|_| "offline".to_string())?;
    ws.send(Message::Text(request.init_frame(&facts))).await.map_err(|_| "offline".to_string())?;
    let first = tokio::time::timeout(Duration::from_secs(15), recv_text(&mut ws))
        .await
        .map_err(|_| "offline".to_string())?
        .map_err(|_| "offline".to_string())?;
    let (user_code, link, expires_in) = match request.on_frame(&first).map_err(|e| super::flow_code(&e))? {
        DeviceRequestEvent::Waiting { user_code, verification_uri, expires_in } => (user_code, verification_uri, expires_in),
        DeviceRequestEvent::Approved(_) => return Err("protocol".into()),
    };
    let qr_svg = super::qr::svg(&link);
    let expires_at = Instant::now() + Duration::from_secs(expires_in);

    let task = tokio::spawn(async move {
        let outcome = match tokio::time::timeout(Duration::from_secs(expires_in), recv_text(&mut ws)).await {
            Err(_) => Err(AuthFlowError::Expired),
            // The socket dropped: the code is lost only when it expires (DW-A07); the server keeps no second socket,
            // so a dropped one cannot receive the relay. Say so honestly as expired-at-the-deadline.
            Ok(Err(_)) => {
                set_status(generation, "reconnecting");
                tokio::time::sleep(expires_at.saturating_duration_since(Instant::now())).await;
                Err(AuthFlowError::Expired)
            }
            Ok(Ok(text)) => request.on_frame(&text),
        };
        match outcome {
            Ok(DeviceRequestEvent::Approved(creds)) => {
                set_status(generation, "approved");
                let installed = install(&app, &creds.session_token, &creds.master_key).await;
                drop(creds);
                set_status(generation, if installed.is_ok() { "signed_in" } else { "failed" });
            }
            Ok(DeviceRequestEvent::Waiting { .. }) => set_status(generation, "failed"),
            Err(e) => set_status(generation, status_for(&e)),
        }
    });

    let slot = Slot {
        generation,
        status: "waiting",
        user_code: Some(user_code),
        expires_at,
        device_name,
        task: Some(task.abort_handle()),
    };
    let out = view(&slot, qr_svg);
    *SLOT.lock().map_err(|_| "failed".to_string())? = Some(slot);
    Ok(out)
}

/// Status and code only (crypto review rule 5).
#[tauri::command]
pub(crate) fn auth_device_request_poll() -> Result<DeviceRequestView, String> {
    let guard = SLOT.lock().map_err(|_| "failed".to_string())?;
    let slot = guard.as_ref().ok_or_else(|| "no_request".to_string())?;
    let mut out = view(slot, None);
    if out.status == "waiting" && out.seconds_left == 0 {
        out.status = "expired".into();
    }
    Ok(out)
}

/// Leaving the screen closes the socket and forgets the code.
#[tauri::command]
pub(crate) fn auth_device_request_cancel() {
    cancel_current();
}

/// Store the arrived session and key the way a browser sign-in does (Windows: `apply_session`, which writes the
/// credential store under `claim_session_write` and starts the engine).
#[cfg(target_os = "windows")]
async fn install(app: &tauri::AppHandle, session_token: &str, master_key: &[u8; 32]) -> Result<(), String> {
    let state = app.state::<crate::AppState>();
    let attempt = crate::AUTH_ATTEMPTS.begin()?;
    let work = async {
        let mut turn = state.active_account()?.session_generation();
        let profile = crate::fetch_canonical_profile(session_token).await;
        crate::apply_session(app.clone(), &state, session_token.to_string(), master_key, profile, &mut turn, &attempt).await
    };
    attempt.run(work).await
}

/// v2 is Windows first (Guus 2026-11-08); macOS gets this on the same core flow later.
#[cfg(not(target_os = "windows"))]
async fn install(app: &tauri::AppHandle, _session_token: &str, _master_key: &[u8; 32]) -> Result<(), String> {
    let _ = app.state::<crate::AppState>();
    Err("unsupported_platform".into())
}
