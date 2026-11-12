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
pub(crate) mod key_owner;
pub(crate) mod password;
pub(crate) mod phrase;
// private_screen: deleted (auth_screen_private was web-view capture exclusion; copy_for_60s moved to the engine's
// clipboard module behind ClipboardAccess).
pub(crate) mod qr;
/// v2 b11: create an account, recover one (declared here: lib.rs is not rustfmt-clean on v2).
#[path = "../signup_v2/mod.rs"]
pub(crate) mod signup_v2;
pub(crate) mod transport;
// webview_autofill: deleted (v2-native has no web views; task 2422's autofill switches are not needed).

#[cfg(test)]
mod dto_tests;

pub(crate) use beebeeb_engine::signin::flow_code;
