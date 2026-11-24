//! Signing in without a UI toolkit: the gate's rules, whose key is stored next to whose session, the password
//! and recovery-phrase steps, and reading a stored session. The Tauri shell re-exports these under their old
//! paths; the engine's own handle (`crate::app`) calls them directly.

pub mod gate;
pub mod key_owner;
#[cfg(any(test, feature = "test-support"))]
pub mod memory_store;
pub mod password;
pub mod recovery;
pub mod start;
pub mod store;

use beebeeb_core::auth_flow::AuthFlowError;

/// The code string a sign-in screen maps to the board's words (`rate_limited:30` carries the wait).
pub fn flow_code(e: &AuthFlowError) -> String {
    match e {
        AuthFlowError::RateLimited { retry_after_secs } => format!("rate_limited:{retry_after_secs}"),
        other => other.code().to_string(),
    }
}
