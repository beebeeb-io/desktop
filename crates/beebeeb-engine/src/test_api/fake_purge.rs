//! Test support: delete from Trash for good, as the fake API answers it: the password step-up that mints the single-use
//! confirmation token, the single `DELETE /api/v1/files/{id}/permanent` and the bulk `POST /api/v1/files/permanent`,
//! in the shapes the engine's client sends and reads (`app::trash_purge_api`, `app::trash_bulk_api`).
//!
//! * `POST /api/v1/auth/confirm-opaque-start` / `-finish`: the account's OPAQUE login; the finish answers
//!   `{confirmation_token, expires_at}`. Each token is spent once (`X-Confirm-Token`); a request without a live token is
//!   `403 confirmation_required` and deletes nothing.
//! * A single delete: an item that is not in Trash is `409 not_trashed`, a trashed folder that still holds a live item is
//!   `409 has_live_descendants` with the count, anything else is gone with its whole subtree (`200 {"success": true}`),
//!   and its bytes leave the listing; an unknown id is `404`.
//! * The bulk delete (at most 500 ids): `200 {"deleted": [ids]}`; when a folder holds live items it is refused, the rest
//!   is deleted, and the answer is `409 has_live_descendants` with `deleted` and `refused: [{id, live_descendants}]`.
//!   An id the account does not have, or one that is not in Trash, is skipped (not listed as deleted).
//!
//! Must not: be reachable from a product build (`test-support` code, like the rest of `test_api`).

use std::collections::HashSet;

use serde_json::{Value, json};

use super::fake_account::FakeAccount;
use super::{ConfirmTokens, Reply, Request};

const START: &str = "POST /api/v1/auth/confirm-opaque-start";
const FINISH: &str = "POST /api/v1/auth/confirm-opaque-finish";
const BULK: &str = "POST /api/v1/files/permanent";
const MAX_BULK: usize = 500;

/// The confirmation tokens this account's step-up minted (one field of [`FakeAccount`]).
#[derive(Default)]
pub(super) struct PurgeState {
    tokens: ConfirmTokens,
}

impl FakeAccount {
    /// Answers the step-up and the permanent-delete routes; `None` for any other route.
    pub(super) fn purge_route(&self, request: &Request) -> Option<Reply> {
        let route = request.route();
        let single = route
            .strip_prefix("DELETE /api/v1/files/")
            .and_then(|rest| rest.strip_suffix("/permanent"));
        if route != START && route != FINISH && route != BULK && single.is_none() {
            return None;
        }
        if request.bearer() != Some(&self.session_token) {
            return Some(Reply::error(401, "unauthorized"));
        }
        Some(match (route.as_str(), single) {
            (START, _) => self.opaque.login_start(&request.json()),
            (FINISH, _) => {
                if self.opaque.login_finish(&request.json()) {
                    let expires_at = (chrono::Utc::now() + chrono::Duration::seconds(300)).to_rfc3339();
                    Reply::ok(json!({ "confirmation_token": self.purge.tokens.mint(), "expires_at": expires_at }))
                } else {
                    Reply::error(401, "invalid_credentials")
                }
            }
            (_, Some(id)) => {
                if !self.purge.tokens.spend(request) {
                    return Some(Reply::error(403, "confirmation_required"));
                }
                self.purge_one(id)
            }
            _ => {
                if !self.purge.tokens.spend(request) {
                    return Some(Reply::error(403, "confirmation_required"));
                }
                self.purge_many(&request.json())
            }
        })
    }

    /// The item `id` and every item below it.
    fn subtree_of(&self, id: &str) -> Vec<String> {
        let items = self.items.lock().expect("vault items");
        let mut found = vec![id.to_string()];
        let mut at = 0;
        while at < found.len() {
            let parent = found[at].clone();
            found.extend(
                items
                    .values()
                    .filter(|item| item.parent_id.as_deref() == Some(parent.as_str()))
                    .map(|item| item.id.clone()),
            );
            at += 1;
        }
        found
    }

    /// How many items below the folder `id` are not in Trash.
    fn live_descendants(&self, id: &str) -> usize {
        let below = self.subtree_of(id);
        let items = self.items.lock().expect("vault items");
        below
            .iter()
            .skip(1)
            .filter(|below| items.get(*below).is_some_and(|item| !item.is_trashed))
            .count()
    }

    /// Drop the item and its subtree for good: from the listing, the versions and the change log.
    fn drop_for_good(&self, id: &str) {
        let below = self.subtree_of(id);
        let mut items = self.items.lock().expect("vault items");
        let mut versions = self.versions.lock().expect("versions");
        for gone in &below {
            items.remove(gone);
            versions.remove(gone);
        }
        self.seq.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }

    fn purge_one(&self, id: &str) -> Reply {
        let trashed = match self.items.lock().expect("vault items").get(id) {
            None => return Reply::error(404, "not_found"),
            Some(item) => item.is_trashed,
        };
        if !trashed {
            return Reply::error(409, "not_trashed");
        }
        let live = self.live_descendants(id);
        if live > 0 {
            return Reply::json(
                409,
                json!({ "error": "has_live_descendants", "live_descendants": live }),
            );
        }
        self.drop_for_good(id);
        Reply::ok(json!({ "success": true }))
    }

    fn purge_many(&self, body: &Value) -> Reply {
        let ids: Vec<String> = body["ids"]
            .as_array()
            .map(|ids| ids.iter().filter_map(|id| id.as_str().map(str::to_string)).collect())
            .unwrap_or_default();
        if ids.is_empty() || ids.len() > MAX_BULK {
            return Reply::error(400, "bad_request");
        }
        let mut deleted = Vec::new();
        let mut refused = Vec::new();
        let mut seen = HashSet::new();
        for id in ids.iter().filter(|id| seen.insert(id.as_str())) {
            let trashed = self
                .items
                .lock()
                .expect("vault items")
                .get(id.as_str())
                .map(|item| item.is_trashed);
            if trashed != Some(true) {
                continue;
            }
            let live = self.live_descendants(id);
            if live > 0 {
                refused.push(json!({ "id": id, "live_descendants": live }));
            } else {
                self.drop_for_good(id);
                deleted.push(id.clone());
            }
        }
        if refused.is_empty() {
            return Reply::ok(json!({ "deleted": deleted }));
        }
        let total: u64 = refused.iter().filter_map(|r| r["live_descendants"].as_u64()).sum();
        Reply::json(
            409,
            json!({ "error": "has_live_descendants", "live_descendants": total, "deleted": deleted, "refused": refused }),
        )
    }
}
