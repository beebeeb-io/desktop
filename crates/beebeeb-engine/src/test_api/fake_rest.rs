//! Test support: the rest of the account REST routes the native app reads, in the shapes the engine's client takes.
//!
//! * Two-factor: `POST /api/v1/auth/2fa/{setup,enable,disable,backup-codes,verify}`. The authenticator is invented:
//!   the key [`TOTP_SECRET`], every code [`TOTP_CODE`]. `setup` when two-factor is on needs the current code or an
//!   `X-Confirm-Token` (else 403); `enable` turns it on and `disable` off (a wrong code is 401 `invalid_code`).
//! * `GET /api/v1/account/activity` (events + summary), `GET /api/v1/account/security-score`, `GET /api/v1/activity`
//!   (the audit feed, `page`/`limit`), `GET /api/v1/me/region` (+ `PUT`), `GET /api/v1/feature-flags/evaluate`.
//! * `POST /api/v1/me/email/resend`: 404 until [`FakeAccount::start_email_change`], then three resends, then 429
//!   `email_code_resend_limit`.
//! * `GET /api/v1/auth/account/deletion` ([`FakeAccount::set_deletion_state`]: `none`, `requested`, `scheduled`).
//! * `GET /api/v1/health` (no token), `GET /api/v1/speedtest?size=N` (N zero bytes, at most 16 MB) and
//!   `POST /api/v1/speedtest` (204).
//!
//! Every route but health and speedtest wants the account's token (401 otherwise). Like the sessions routes, all of
//! them answer only after [`FakeAccount::serve_security_routes`]. Invented data only; nothing here is a secret.
//! Must not: be reachable from a product build (`test-support` code, like the rest of `test_api`).

use std::sync::Mutex;
use std::sync::atomic::Ordering;

use serde_json::{Value, json};

use super::fake_account::FakeAccount;
use super::{Reply, Request};

/// The invented authenticator key `2fa/setup` hands out.
pub const TOTP_SECRET: &str = "JBSWY3DPEHPK3PXP";
/// The only 6-digit code the fake takes (also the one `2fa/verify` takes with `partial_token` `partial`).
pub const TOTP_CODE: &str = "123456";
const RESENDS: u64 = 3;
const SPEEDTEST_MAX: usize = 16_000_000;

/// What the routes of this file keep (one field of [`FakeAccount`]).
pub(super) struct RestState {
    pending_email: Mutex<Option<String>>,
    resends_left: Mutex<u64>,
    deletion: Mutex<&'static str>,
    region: Mutex<(Option<String>, bool)>,
}

impl Default for RestState {
    fn default() -> Self {
        Self {
            pending_email: Mutex::new(None),
            resends_left: Mutex::new(RESENDS),
            deletion: Mutex::new("none"),
            region: Mutex::new((Some("europe".to_string()), false)),
        }
    }
}

fn is_code(body: &Value) -> bool {
    body.get("code").and_then(Value::as_str) == Some(TOTP_CODE)
}

fn backup_codes() -> Value {
    json!(["11111111", "22222222", "33333333"])
}

impl FakeAccount {
    /// A change of the sign-in email is waiting for its code (what `me/email/resend` needs).
    pub fn start_email_change(&self, new_email: &str) {
        *self.rest.pending_email.lock().expect("pending email") = Some(new_email.to_string());
        *self.rest.resends_left.lock().expect("resends") = RESENDS;
    }

    /// Where `auth/account/deletion` says the account stands: `none`, `requested` or `scheduled`.
    pub fn set_deletion_state(&self, state: &str) {
        let state = match state {
            "requested" => "requested",
            "scheduled" => "scheduled",
            _ => "none",
        };
        *self.rest.deletion.lock().expect("deletion") = state;
    }

    /// Answers the routes of this file; `None` for any other route (or while the security routes are off).
    pub(super) fn rest_route(&self, request: &Request) -> Option<Reply> {
        let route = request.route();
        let path = route.split_once(' ').map(|(_, p)| p).unwrap_or("");
        let known = path.starts_with("/api/v1/auth/2fa/")
            || matches!(
                path,
                "/api/v1/account/activity"
                    | "/api/v1/account/security-score"
                    | "/api/v1/activity"
                    | "/api/v1/me/region"
                    | "/api/v1/me/email/resend"
                    | "/api/v1/auth/account/deletion"
                    | "/api/v1/feature-flags/evaluate"
                    | "/api/v1/health"
                    | "/api/v1/speedtest"
            );
        if !known || !self.security_routes_on() {
            return None;
        }
        let open = path == "/api/v1/health" || path == "/api/v1/speedtest" || path == "/api/v1/auth/2fa/verify";
        if !open && request.bearer() != Some(&self.session_token) {
            return Some(Reply::error(401, "unauthorized"));
        }
        let body = request.json();
        let on = &self.two_factor_on;
        Some(match route.as_str() {
            "POST /api/v1/auth/2fa/setup" => {
                if on.load(Ordering::SeqCst) && !is_code(&body) && request.header("x-confirm-token").is_none() {
                    Reply::error(403, "step_up_required")
                } else {
                    Reply::ok(json!({
                        "secret": TOTP_SECRET,
                        "qr_uri": format!("otpauth://totp/Beebeeb:fixture?secret={TOTP_SECRET}&issuer=Beebeeb"),
                        "backup_codes": backup_codes(),
                    }))
                }
            }
            "POST /api/v1/auth/2fa/enable" if is_code(&body) => {
                on.store(true, Ordering::SeqCst);
                Reply::ok(json!({}))
            }
            "POST /api/v1/auth/2fa/disable" if is_code(&body) => {
                on.store(false, Ordering::SeqCst);
                Reply::ok(json!({}))
            }
            "POST /api/v1/auth/2fa/enable" | "POST /api/v1/auth/2fa/disable" | "POST /api/v1/auth/2fa/backup-codes"
                if !is_code(&body) =>
            {
                Reply::error(401, "invalid_code")
            }
            "POST /api/v1/auth/2fa/backup-codes" => Reply::ok(json!({ "backup_codes": backup_codes() })),
            "POST /api/v1/auth/2fa/verify" => {
                if is_code(&body) && body.get("partial_token").and_then(Value::as_str) == Some("partial") {
                    Reply::ok(json!({ "session_token": self.session_token }))
                } else {
                    Reply::error(401, "invalid_code")
                }
            }
            "GET /api/v1/account/activity" => Reply::ok(json!({
                "events": [{
                    "id": "fixture-event-1",
                    "type": "auth.login.success",
                    "description": "Signed in",
                    "category": "auth",
                    "outcome": "success",
                    "device": "Fixture PC",
                    "country_code": "DE",
                    "created_at": "2026-11-15T05:00:00Z",
                }],
                "summary": {
                    "last_login_at": "2026-11-15T05:00:00Z",
                    "last_login_device": "Fixture PC",
                    "active_sessions": self.session_ids().len(),
                    "active_shares": 0,
                    "security_score": if on.load(Ordering::SeqCst) { "Strong" } else { "Good" },
                },
            })),
            "GET /api/v1/account/security-score" => {
                let two = on.load(Ordering::SeqCst);
                let score = 4 + u8::from(two);
                let factor = |key: &str, satisfied: bool| json!({ "key": key, "satisfied": satisfied });
                Reply::ok(json!({
                    "score": score,
                    "max": 5,
                    "label": if two { "Strong" } else { "Good" },
                    "factors": [
                        factor("email_verified", true),
                        factor("phrase_saved", true),
                        factor("two_factor_enabled", two),
                        factor("phrase_recently_tested", true),
                        factor("all_devices_recognized", true),
                    ],
                }))
            }
            "GET /api/v1/activity" => {
                let page = request.query("page").and_then(|v| v.parse::<i64>().ok()).unwrap_or(1);
                let limit = request
                    .query("limit")
                    .and_then(|v| v.parse::<usize>().ok())
                    .unwrap_or(20);
                let all = [
                    ("fixture-feed-1", "auth.login.success"),
                    ("fixture-feed-2", "file.upload"),
                ];
                let skip = usize::try_from((page - 1).max(0)).unwrap_or(0).saturating_mul(limit);
                let events: Vec<Value> = all
                    .iter()
                    .skip(skip)
                    .take(limit)
                    .map(|(id, kind)| {
                        json!({
                            "id": id, "type": kind, "subject": null, "details": null,
                            "where": "Fixture PC", "created_at": "2026-11-15T05:00:00Z",
                        })
                    })
                    .collect();
                Reply::ok(json!({ "events": events, "total": all.len(), "page": page }))
            }
            "GET /api/v1/me/region" | "PUT /api/v1/me/region" => {
                let mut region = self.rest.region.lock().expect("region");
                if request.method == "PUT" {
                    region.0 = body.get("preferred_region").and_then(Value::as_str).map(str::to_string);
                    if let Some(strict) = body.get("region_strict").and_then(Value::as_bool) {
                        region.1 = strict;
                    }
                    return Some(Reply::ok(
                        json!({ "preferred_region": region.0, "region_strict": region.1 }),
                    ));
                }
                Reply::ok(json!({
                    "preferred_region": region.0,
                    "region_strict": region.1,
                    "available_regions": [{
                        "continent": "Europe", "display_name": "Europe",
                        "example_country": "Germany", "is_default": true,
                    }],
                    "choices": [
                        { "id": "europe", "kind": "region", "name": "Europe", "status": "live",
                          "selectable": true, "is_default": true },
                        { "id": "de", "kind": "country", "name": "Germany", "country": "Germany",
                          "country_code": "DE", "status": "live", "selectable": true, "is_default": false },
                    ],
                }))
            }
            "GET /api/v1/feature-flags/evaluate" => Reply::ok(json!({ "flags": { "shared_folders_v1": true } })),
            "POST /api/v1/me/email/resend" => {
                let pending = self.rest.pending_email.lock().expect("pending email").clone();
                let mut left = self.rest.resends_left.lock().expect("resends");
                match pending {
                    None => Reply::error(404, "no_pending_email_change"),
                    Some(_) if *left == 0 => Reply::error(429, "email_code_resend_limit"),
                    Some(email) => {
                        *left -= 1;
                        Reply::ok(json!({ "pending_email": email, "code_expires_in": 900, "resends_left": *left }))
                    }
                }
            }
            "GET /api/v1/auth/account/deletion" => match *self.rest.deletion.lock().expect("deletion") {
                "scheduled" => Reply::ok(json!({
                    "state": "scheduled",
                    "shred_after": "2099-02-01T00:00:00Z",
                    "cancel_until": "2099-01-31T00:00:00Z",
                })),
                "requested" => Reply::ok(json!({ "state": "requested", "link_expires_at": "2099-01-01T00:00:00Z" })),
                _ => Reply::ok(json!({ "state": "none" })),
            },
            "GET /api/v1/health" => Reply::ok(json!({ "status": "ok" })),
            "GET /api/v1/speedtest" => {
                let size = request.query("size").and_then(|v| v.parse::<usize>().ok()).unwrap_or(0);
                Reply::bytes(200, vec![0u8; size.min(SPEEDTEST_MAX)])
            }
            "POST /api/v1/speedtest" => Reply::bytes(204, Vec::new()),
            _ => Reply::error(404, "not_found"),
        })
    }
}
