//! Desktop v2 sign-in commands (batch B10): thin Tauri commands over core's `auth_flow`.
//!
//! The flows (OPAQUE step-up, device approval, the requester's socket frames and decrypt) are core's; this module
//! keeps only what is the platform's: the HTTP and WebSocket transport, Windows Hello (`hello`), the WebView2
//! autofill switches (`webview_autofill`, task 2422), and installing an arrived session through the existing
//! `apply_session` path.
//!
//! Secrets travel one way, from the web view to Rust (the password for the approval step-up). No command returns a
//! key, a session or partial token, a confirm token, an ECDH secret or a server URL: every DTO here is in
//! `dto_tests`'s no-secret list, which fails on a planted field.
pub(crate) mod approve;
pub(crate) mod device;
pub(crate) mod gate;
pub(crate) mod hello;
pub(crate) mod password;
pub(crate) mod phrase;
pub(crate) mod private_screen;
pub(crate) mod qr;
/// v2 b11: create an account, recover one (declared here: lib.rs is not rustfmt-clean on v2).
#[path = "../signup_v2/mod.rs"]
pub(crate) mod signup_v2;
pub(crate) mod transport;
pub(crate) mod webview_autofill;

#[cfg(test)]
mod dto_tests;

use beebeeb_core::auth_flow::AuthFlowError;

/// The code string the web view maps to the board's words (`rate_limited:30` carries the wait).
pub(crate) fn flow_code(e: &AuthFlowError) -> String {
    match e {
        AuthFlowError::RateLimited { retry_after_secs } => format!("rate_limited:{retry_after_secs}"),
        other => other.code().to_string(),
    }
}
