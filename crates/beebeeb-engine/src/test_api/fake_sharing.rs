//! Test support: sharing as the fake API keeps it, in the shapes the engine's client reads and sends
//! (`app::shares_api`, `api_client::shares`, core's `share_flow`). Rows are what a test (or the fixture) put there; every
//! write changes them, so the next listing shows it.
//!
//! * Invites: `GET /api/v1/shares/invites/incoming` and `/sent` (`{"invites": [...]}`); `POST .../{id}/decline` and
//!   `.../{id}/claim` (an incoming invite: gone / `claimed`, which the listing hides until the sender approves),
//!   `DELETE .../{id}` (the sender cancels) and `PATCH .../{id}` `{permission}` (read or write).
//! * Share links: `PATCH /api/v1/shares/{id}` sets expiry/password, `DELETE /api/v1/shares/{id}` marks the link revoked, as `GET /api/v1/shares/mine` then lists it.
//! * File requests: `GET /api/v1/file-requests` (`{"file_requests": [...]}`), `POST .../{id}/close` and `.../{id}/hide`.
//! * Shared folders (the owner's custody record): `PUT /api/v1/shared-folders/{id}/key` stores it, `GET
//!   /api/v1/shared-folders/{id}` reads it (404 until stored), `GET .../{id}/keys` is the member key inventory (empty).
//!
//! An id the account does not have is `404`; a request without the account's token is `401`.
//! Must not: be reachable from a product build (`test-support` code, like the rest of `test_api`).

use std::collections::HashMap;
use std::sync::Mutex;

use serde_json::{Value, json};

use super::fake_account::FakeAccount;
use super::{Reply, Request};

/// The invites, file requests and shared-folder custody records (one field of [`FakeAccount`]).
#[derive(Default)]
pub(super) struct SharingState {
    incoming: Mutex<Vec<Value>>,
    sent: Mutex<Vec<Value>>,
    requests: Mutex<Vec<Value>>,
    custody: Mutex<HashMap<String, Value>>,
}

fn id_of(row: &Value) -> &str {
    row["id"].as_str().unwrap_or("")
}

impl FakeAccount {
    /// An invite sent TO this account (status `invited`): what the listing shows as waiting.
    pub fn add_incoming_invite(&self, id: &str, file_id: &str, sender_email: &str) {
        self.sharing.incoming.lock().expect("invites").push(json!({
            "id": id,
            "file_id": file_id,
            "sender_email": sender_email,
            "status": "invited",
            "is_folder": false,
            "size_bytes": 0,
            "created_at": "2026-11-15T05:00:00Z",
        }));
    }

    /// An invite this account sent (status `invited`, read access) to `recipient_email`.
    pub fn add_sent_invite(&self, id: &str, file_id: &str, recipient_email: &str) {
        self.sharing.sent.lock().expect("invites").push(json!({
            "id": id,
            "file_id": file_id,
            "recipient_email": recipient_email,
            "status": "invited",
            "permission": "read",
            "is_folder_share": false,
        }));
    }

    /// A file request the account made: open, nothing received yet.
    pub fn add_file_request(&self, id: &str, title: &str, max_files: u64) {
        self.sharing.requests.lock().expect("requests").push(json!({
            "id": id,
            "title": title,
            "files_received": 0,
            "max_files": max_files,
            "expires_at": null,
            "closed": false,
            "hidden": false,
        }));
    }

    /// One field of the file request `id` (`closed`, `hidden`), `None` when there is no such request.
    pub fn file_request_flag(&self, id: &str, field: &str) -> Option<bool> {
        let requests = self.sharing.requests.lock().expect("requests");
        requests.iter().find(|r| id_of(r) == id)?[field].as_bool()
    }

    /// An incoming invite the sender approved (the recipient may open it, and leave it).
    pub fn approve_incoming_invite(&self, id: &str) {
        let mut incoming = self.sharing.incoming.lock().expect("invites");
        if let Some(row) = incoming.iter_mut().find(|r| id_of(r) == id) {
            row["status"] = json!("approved");
        }
    }

    /// The `status` and `permission` of the invite the account sent, `None` when it is gone.
    pub fn sent_invite_state(&self, id: &str) -> Option<(String, String)> {
        let sent = self.sharing.sent.lock().expect("invites");
        let row = sent.iter().find(|r| id_of(r) == id)?;
        Some((
            row["status"].as_str().unwrap_or("").to_string(),
            row["permission"].as_str().unwrap_or("").to_string(),
        ))
    }

    /// The `status` of an incoming invite, `None` when it is gone (declined).
    pub fn incoming_invite_status(&self, id: &str) -> Option<String> {
        let incoming = self.sharing.incoming.lock().expect("invites");
        Some(
            incoming.iter().find(|r| id_of(r) == id)?["status"]
                .as_str()?
                .to_string(),
        )
    }

    /// Answers the sharing routes; `None` for any other route.
    pub(super) fn sharing_route(&self, request: &Request) -> Option<Reply> {
        let path = request.path.split('?').next().unwrap_or("").to_string();
        let method = request.method.as_str();
        let ours = path.starts_with("/api/v1/shares/invites/")
            || path.starts_with("/api/v1/file-requests")
            || path.starts_with("/api/v1/shared-folders/")
            || (matches!(method, "DELETE" | "PATCH") && path.starts_with("/api/v1/shares/"));
        if !ours {
            return None;
        }
        // `/shares/mine` and `/shares/folder-links` are other routes' (a link's revoke is `DELETE /shares/{id}`).
        if path.starts_with("/api/v1/shares/") && !path.starts_with("/api/v1/shares/invites/") {
            let id = &path["/api/v1/shares/".len()..];
            if id.contains('/') || id == "mine" {
                return None;
            }
        }
        if request.bearer() != Some(&self.session_token) {
            return Some(Reply::error(401, "unauthorized"));
        }
        let segments: Vec<&str> = path.trim_start_matches("/api/v1/").split('/').collect();
        let state = &self.sharing;
        Some(match (method, segments.as_slice()) {
            ("GET", ["shares", "invites", "incoming"]) => {
                Reply::ok(json!({ "invites": *state.incoming.lock().expect("invites") }))
            }
            ("GET", ["shares", "invites", "sent"]) => {
                Reply::ok(json!({ "invites": *state.sent.lock().expect("invites") }))
            }
            ("POST", ["shares", "invites", id, "decline"]) => {
                let mut incoming = state.incoming.lock().expect("invites");
                match incoming.iter().position(|r| id_of(r) == *id) {
                    Some(at) => {
                        incoming.remove(at);
                        Reply::ok(json!({ "id": id, "status": "declined" }))
                    }
                    None => Reply::error(404, "not_found"),
                }
            }
            ("POST", ["shares", "invites", id, "claim"]) => {
                let mut incoming = state.incoming.lock().expect("invites");
                match incoming.iter_mut().find(|r| id_of(r) == *id) {
                    Some(row) if row["status"] == "invited" => {
                        row["status"] = json!("claimed");
                        Reply::ok(json!({ "id": id, "status": "claimed" }))
                    }
                    Some(_) => Reply::error(400, "invite is not in the invited state"),
                    None => Reply::error(404, "not_found"),
                }
            }
            ("DELETE", ["shares", "invites", id]) => {
                let mut sent = state.sent.lock().expect("invites");
                match sent.iter().position(|r| id_of(r) == *id) {
                    Some(at) => {
                        sent.remove(at);
                        Reply::ok(json!({ "success": true }))
                    }
                    None => Reply::error(404, "not_found"),
                }
            }
            ("PATCH", ["shares", "invites", id]) => {
                let permission = request.json()["permission"].as_str().unwrap_or("").to_string();
                if !matches!(permission.as_str(), "read" | "write") {
                    return Some(Reply::error(400, "bad_permission"));
                }
                let mut sent = state.sent.lock().expect("invites");
                match sent.iter_mut().find(|r| id_of(r) == *id) {
                    Some(row) => {
                        row["permission"] = json!(permission);
                        Reply::ok(json!({ "id": id, "permission": permission }))
                    }
                    None => Reply::error(404, "not_found"),
                }
            }
            ("DELETE", ["shares", id]) => self.revoke_link(id),
            ("PATCH", ["shares", id]) => self.patch_link(id, &request.json()),
            ("POST", ["shares", "invites", id, "leave"]) => {
                let mut incoming = state.incoming.lock().expect("invites");
                match incoming
                    .iter()
                    .position(|r| id_of(r) == *id && r["status"] == "approved")
                {
                    Some(at) => {
                        incoming.remove(at);
                        Reply::ok(json!({ "status": "left" }))
                    }
                    None => Reply::error(404, "not_found"),
                }
            }
            ("PATCH", ["file-requests", id]) => {
                let body = request.json();
                let mut requests = state.requests.lock().expect("requests");
                match requests.iter_mut().find(|r| id_of(r) == *id) {
                    Some(row) if row["closed"] == true => Reply::error(409, "closed"),
                    Some(row) => match &body["expires_in_secs"] {
                        Value::Null => {
                            row["expires_at"] = Value::Null;
                            Reply::ok(json!({ "id": id, "expires_at": null }))
                        }
                        secs => match secs.as_i64().filter(|s| *s > 0) {
                            Some(secs) => {
                                let at = (chrono::Utc::now() + chrono::Duration::seconds(secs)).to_rfc3339();
                                row["expires_at"] = json!(at);
                                Reply::ok(json!({ "id": id, "expires_at": at }))
                            }
                            None => Reply::error(400, "bad_expiry"),
                        },
                    },
                    None => Reply::error(404, "not_found"),
                }
            }
            ("GET", ["file-requests"]) => {
                Reply::ok(json!({ "file_requests": *state.requests.lock().expect("requests") }))
            }
            ("POST", ["file-requests", id, action @ ("close" | "hide")]) => {
                let mut requests = state.requests.lock().expect("requests");
                match requests.iter_mut().find(|r| id_of(r) == *id) {
                    Some(row) => {
                        row[if *action == "close" { "closed" } else { "hidden" }] = json!(true);
                        Reply::ok(json!({ "success": true }))
                    }
                    None => Reply::error(404, "not_found"),
                }
            }
            ("PUT", ["shared-folders", id, "key"]) => {
                let body = request.json();
                let Some(wrapped) = body["owner_wrapped_folder_key"].as_str() else {
                    return Some(Reply::error(400, "bad_request"));
                };
                let record = json!({
                    "owner_wrapped_folder_key": wrapped,
                    "key_version": body["key_version"].as_i64().unwrap_or(1),
                });
                state
                    .custody
                    .lock()
                    .expect("custody")
                    .insert(id.to_string(), record.clone());
                Reply::ok(record)
            }
            ("GET", ["shared-folders", id]) => match state.custody.lock().expect("custody").get(*id) {
                Some(record) => Reply::ok(record.clone()),
                None => Reply::error(404, "not_found"),
            },
            ("GET", ["shared-folders", _, "keys"]) => Reply::ok(json!({ "keys": [], "next_cursor": null })),
            _ => Reply::error(404, "not_found"),
        })
    }
}
