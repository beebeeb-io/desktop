//! Test support: Beebeeb's routes that change the account password, over an [`OpaqueAccount`], for a host's tests of a
//! change that goes through: the step-up (`confirm-opaque-start` / `-finish`, the account's OPAQUE login), then the
//! new OPAQUE registration (`change-password-start` / `-finish`), whose finish answers a new session token. Every name
//! is invented; nothing here checks what a real server checks beyond the confirmation token being spent once.

use base64::Engine as _;
use beebeeb_core::opaque_protocol as opaque;
use serde_json::json;

use super::{ConfirmTokens, OpaqueAccount, Reply, Request};

/// The session token the change's finish answers with.
pub const NEW_SESSION_TOKEN: &str = "an-invented-session-token-after-a-password-change";

pub struct PasswordChangeRoutes {
    setup: Vec<u8>,
    tokens: ConfirmTokens,
    identifier: Vec<u8>,
}

impl PasswordChangeRoutes {
    /// The routes for the account registered under `email`.
    pub fn new(email: &str) -> Self {
        Self {
            setup: opaque::create_server_setup(),
            tokens: ConfirmTokens::default(),
            identifier: email.as_bytes().to_vec(),
        }
    }

    /// The answer to `request` when it is one of these routes (`account`: the OPAQUE record the step-up proves the
    /// password against); `None` for any other route.
    pub fn answer(&self, request: &Request, account: &OpaqueAccount) -> Option<Reply> {
        let b64 = base64::engine::general_purpose::STANDARD;
        let body = request.json();
        let message = || {
            body["client_message"]
                .as_str()
                .and_then(|m| b64.decode(m).ok())
                .unwrap_or_default()
        };
        Some(match request.route().as_str() {
            "GET /api/v1/onboarding" => Reply::ok(json!({ "policy": { "password": { "min_length": 12 } } })),
            "POST /api/v1/auth/confirm-opaque-start" => account.login_start(&body),
            "POST /api/v1/auth/confirm-opaque-finish" if account.login_finish(&body) => {
                Reply::ok(json!({ "confirmation_token": self.tokens.mint(), "expires_at": "soon" }))
            }
            "POST /api/v1/auth/confirm-opaque-finish" => Reply::error(401, "invalid_credentials"),
            "POST /api/v1/auth/change-password-start" if self.tokens.spend(request) => {
                match opaque::server_registration_start(&self.setup, &message(), &self.identifier) {
                    Ok(reply) => Reply::ok(json!({ "server_message": b64.encode(reply) })),
                    Err(_) => Reply::error(400, "bad_request"),
                }
            }
            "POST /api/v1/auth/change-password-finish" if self.tokens.spend(request) => {
                match opaque::server_registration_finish(&message()) {
                    Ok(_) => Reply::ok(json!({ "session_token": NEW_SESSION_TOKEN })),
                    Err(_) => Reply::error(400, "bad_request"),
                }
            }
            "POST /api/v1/auth/change-password-start" | "POST /api/v1/auth/change-password-finish" => {
                Reply::error(403, "confirmation_required")
            }
            _ => return None,
        })
    }
}
