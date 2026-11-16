//! Test support: the account's sign-in sessions as the fake API keeps them, in the shapes the engine's client reads
//! (`api_client::auth`, `account_dto::AccountSession`).
//!
//! * `GET /api/v1/account/sessions` lists them (`{"sessions": [...]}`, the current one marked `is_current`).
//! * `DELETE /api/v1/account/sessions/{id}` ends one (`{"revoked": true}`); the one this token belongs to is refused
//!   (`400`), an unknown id is `404`.
//! * `POST /api/v1/account/sessions/revoke-all-others` ends every session but the current one (`{"revoked": <count>}`).
//!
//! The routes answer only after [`FakeAccount::serve_security_routes`] (a test's own fake is never shadowed).
//! Must not: be reachable from a product build (`test-support` code, like the rest of `test_api`).

use std::sync::Mutex;

use serde_json::{Value, json};

use super::fake_account::FakeAccount;
use super::{Reply, Request};

const LIST: &str = "GET /api/v1/account/sessions";
const REVOKE_OTHERS: &str = "POST /api/v1/account/sessions/revoke-all-others";
const ONE: &str = "DELETE /api/v1/account/sessions/";

/// The session ids and device names (one field of [`FakeAccount`]); the first is the current one.
pub(super) struct SessionState {
    rows: Mutex<Vec<(String, String, String)>>,
}

impl Default for SessionState {
    fn default() -> Self {
        let row = |id: &str, name: &str, kind: &str| (id.to_string(), name.to_string(), kind.to_string());
        Self {
            rows: Mutex::new(vec![row("fixture-session-1", "Fixture PC", "desktop")]),
        }
    }
}

impl FakeAccount {
    /// Another signed-in session (a device other than the current one).
    pub fn add_session(&self, id: &str, device_name: &str) {
        self.sessions_state.rows.lock().expect("sessions").push((
            id.to_string(),
            device_name.to_string(),
            "desktop".to_string(),
        ));
    }

    /// The ids of the sessions that are still signed in, in the order they were made.
    pub fn session_ids(&self) -> Vec<String> {
        self.sessions_state
            .rows
            .lock()
            .expect("sessions")
            .iter()
            .map(|(id, ..)| id.clone())
            .collect()
    }

    /// Answers the sessions routes; `None` for any other route (or while the security routes are off).
    pub(super) fn sessions_route(&self, request: &Request) -> Option<Reply> {
        let route = request.route();
        if route != LIST && route != REVOKE_OTHERS && !route.starts_with(ONE) {
            return None;
        }
        if !self.security_routes_on() {
            return None;
        }
        if request.bearer() != Some(&self.session_token) {
            return Some(Reply::error(401, "unauthorized"));
        }
        let mut rows = self.sessions_state.rows.lock().expect("sessions");
        let current = rows.first().map(|(id, ..)| id.clone()).unwrap_or_default();
        Some(if route == LIST {
            let sessions: Vec<Value> = rows
                .iter()
                .map(|(id, name, kind)| {
                    json!({
                        "id": id,
                        "device_name": name,
                        "device_kind": kind,
                        "country_code": "DE",
                        "last_active_at": "2026-11-15T05:00:00Z",
                        "is_current": *id == current,
                    })
                })
                .collect();
            Reply::ok(json!({ "sessions": sessions }))
        } else if route == REVOKE_OTHERS {
            let before = rows.len();
            rows.retain(|(id, ..)| *id == current);
            Reply::ok(json!({ "revoked": before - rows.len() }))
        } else {
            let id = route.strip_prefix(ONE).unwrap_or("");
            if id == current {
                Reply::error(400, "cannot_revoke_current_session")
            } else if let Some(at) = rows.iter().position(|(row, ..)| row == id) {
                rows.remove(at);
                Reply::ok(json!({ "revoked": true }))
            } else {
                Reply::error(404, "not_found")
            }
        })
    }
}
