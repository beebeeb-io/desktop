//! The server's side of creating an account, for the fake API: the onboarding document, the email code, the breach
//! range, a real OPAQUE registration (core's `opaque_protocol`), the profile of the account that was made, the terms
//! acceptance and the logout. Every address, code, token and password is invented.
//!
//! Tests only (`test` or the `test-support` feature). It answers what `beebeeb_core::signup_flow` sends and nothing
//! else; any other route is a 404.

use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use base64::Engine as _;
use beebeeb_core::onboarding::BreachQuery;
use beebeeb_core::opaque_protocol as opaque;
use serde_json::{Value, json};

use super::{Handler, Reply, Request};

/// The email code the fake accepts.
pub const CODE: &str = "48291650";
/// The token of the session the fake makes at registration.
pub const SESSION_TOKEN: &str = "an-invented-new-session";
pub const USER_ID: &str = "an-invented-new-user";

/// How the fake treats sign-up.
pub const OPEN: u8 = 0;
/// The onboarding document says `signup.allowed: false` (the server today, for a non-web app).
pub const WEB_ONLY_DOCUMENT: u8 = 1;
/// The document allows it, and every create call answers 403 `signup_web_only`.
pub const WEB_ONLY_CALLS: u8 = 2;

/// The state of one fake sign-up, shared with the test that looks at it.
pub struct FakeSignup {
    pub mode: AtomicU8,
    /// The breach range lists this password's hash.
    pub breached_password: Mutex<Option<String>>,
    /// The breach range answers 500.
    pub breach_down: AtomicBool,
    /// `register-finish` answers 500 (and creates nothing).
    pub finish_fails: AtomicBool,
    /// `GET /auth/me` answers 500 for the new session (the account exists; this computer cannot sign in to it).
    pub profile_fails: AtomicBool,
    /// The OPAQUE password file, once the registration finished.
    pub registered: Mutex<Option<Vec<u8>>>,
    pub email: Mutex<String>,
    pub terms_accepted: AtomicUsize,
    pub logouts: AtomicUsize,
    setup: Vec<u8>,
}

impl Default for FakeSignup {
    fn default() -> Self {
        Self {
            mode: AtomicU8::new(OPEN),
            breached_password: Mutex::new(None),
            breach_down: AtomicBool::new(false),
            finish_fails: AtomicBool::new(false),
            profile_fails: AtomicBool::new(false),
            registered: Mutex::new(None),
            email: Mutex::new(String::new()),
            terms_accepted: AtomicUsize::new(0),
            logouts: AtomicUsize::new(0),
            setup: opaque::create_server_setup(),
        }
    }
}

fn b64() -> base64::engine::general_purpose::GeneralPurpose {
    base64::engine::general_purpose::STANDARD
}

/// The handler for `FakeApi::start`.
pub fn signup_handler(state: Arc<FakeSignup>) -> Handler {
    Arc::new(move |req: &Request| {
        let web_only_calls = state.mode.load(Ordering::SeqCst) == WEB_ONLY_CALLS;
        let refused = || {
            Reply::json(
                403,
                json!({ "error": "signup_web_only", "web_url": "https://app.beebeeb.io/signup" }),
            )
        };
        match req.route().as_str() {
            "GET /api/v1/onboarding" => {
                let allowed = state.mode.load(Ordering::SeqCst) != WEB_ONLY_DOCUMENT;
                let mut signup = json!({ "allowed": allowed, "mode": if allowed { "native" } else { "web_only" } });
                if !allowed {
                    signup["reason"] = json!("signup_web_only");
                    signup["web_url"] = json!("https://app.beebeeb.io/signup");
                }
                Reply::ok(json!({
                    "signup": signup,
                    "policy": {
                        "password": {
                            "min_length": 12,
                            "breach_check": { "endpoint": "/api/v1/auth/pwned-range/{prefix}", "fail_open": false }
                        },
                        "recovery_phrase": { "word_count": 12, "verify_word_count": 3 },
                        "terms": { "version": "2026-09-29", "url": "https://beebeeb.io/terms", "privacy_url": "https://beebeeb.io/privacy" },
                        "email_code": { "length": 8, "ttl_seconds": 900, "resend_after_seconds": 60 }
                    }
                }))
            }
            "POST /api/v1/auth/signup/email-start" => {
                if web_only_calls {
                    return refused();
                }
                *state.email.lock().unwrap() = req.json()["email"].as_str().unwrap_or_default().to_string();
                Reply::json(202, json!({}))
            }
            "POST /api/v1/auth/signup/email-verify" => {
                if req.json()["code"] == CODE {
                    Reply::ok(json!({ "signup_ticket": "an-invented-ticket" }))
                } else {
                    Reply::error(400, "invalid_code")
                }
            }
            route if route.starts_with("GET /api/v1/auth/pwned-range/") => {
                if state.breach_down.load(Ordering::SeqCst) {
                    return Reply::error(500, "down");
                }
                let mut lines = String::from("00000000000000000000000000000000000:3\r\n");
                if let Some(password) = state.breached_password.lock().unwrap().as_deref() {
                    let query = BreachQuery::from_password(password);
                    if route.ends_with(&format!("/{}", query.prefix())) {
                        lines.push_str(&format!("{}:41\r\n", query.suffix()));
                    }
                }
                Reply::bytes(200, lines.into_bytes())
            }
            "POST /api/v1/opaque/register-start" => {
                if web_only_calls {
                    return refused();
                }
                let body = req.json();
                let email = body["email"].as_str().unwrap_or_default();
                let message = body["client_message"].as_str().and_then(|m| b64().decode(m).ok());
                match message.and_then(|m| opaque::server_registration_start(&state.setup, &m, email.as_bytes()).ok()) {
                    Some(answer) => Reply::ok(json!({ "server_message": b64().encode(answer) })),
                    None => Reply::error(400, "bad_request"),
                }
            }
            "POST /api/v1/opaque/register-finish" => {
                if state.finish_fails.load(Ordering::SeqCst) {
                    return Reply::error(500, "boom");
                }
                let upload = req.json()["client_message"].as_str().and_then(|m| b64().decode(m).ok());
                match upload.and_then(|u| opaque::server_registration_finish(&u).ok()) {
                    Some(file) => {
                        *state.registered.lock().unwrap() = Some(file);
                        Reply::ok(json!({ "session_token": SESSION_TOKEN, "user_id": USER_ID }))
                    }
                    None => Reply::error(400, "bad_request"),
                }
            }
            "GET /api/v1/auth/me" => {
                if req.bearer() != Some(SESSION_TOKEN) || state.profile_fails.load(Ordering::SeqCst) {
                    return Reply::error(
                        if state.profile_fails.load(Ordering::SeqCst) {
                            500
                        } else {
                            401
                        },
                        "no",
                    );
                }
                Reply::ok(json!({
                    "user_id": USER_ID,
                    "email": state.email.lock().unwrap().clone(),
                    "email_verified": true,
                    "created_at": "2026-11-14T00:00:00Z",
                    "role": "user",
                }))
            }
            "POST /api/v1/account/terms-acceptance" => {
                state.terms_accepted.fetch_add(1, Ordering::SeqCst);
                Reply::ok(Value::Null)
            }
            "POST /api/v1/auth/logout" => {
                state.logouts.fetch_add(1, Ordering::SeqCst);
                Reply::ok(Value::Null)
            }
            _ => Reply::error(404, "not_found"),
        }
    })
}
