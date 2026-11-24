//! Test support: the Trash count and the search-index shard routes of the fake API, in the shapes the engine's client
//! sends and reads (`api_client::files::trashed_file_count`, `api_client::search`).
//!
//! * `GET /api/v1/files/count` answers the counter row; the engine reads `trashed_files` (the account's trashed items).
//! * `GET /api/v1/search-index/shards` is the manifest (`{"shards": [{bucket, page, version}]}`), `GET`/`PUT`/`DELETE
//!   /api/v1/search-index/shards/{bucket}/{page}` read, store and drop one opaque blob. A `PUT` takes an octet-stream
//!   body (empty: 400; over the client's 160 KiB cap: 413), bumps the version and answers the shard's ref as JSON.
//!   The fake never reads a blob. A request without the account's token is `401`, and nothing is read or stored.
//!
//! Must not: be reachable from a product build (`test-support` code, like the rest of `test_api`).

use std::collections::BTreeMap;
use std::sync::Mutex;

use serde_json::json;

use super::fake_account::FakeAccount;
use super::{Reply, Request};
use crate::api_client::MAX_SEARCH_SHARD_BLOB_BYTES;

const SHARDS: &str = "/api/v1/search-index/shards";

/// The stored shards of one account (one field of [`FakeAccount`]).
/// A shard's version and bytes.
type VersionedShard = (i64, Vec<u8>);

#[derive(Default)]
pub(super) struct SearchShardState {
    next_version: Mutex<i64>,
    shards: Mutex<BTreeMap<(u32, u32), VersionedShard>>,
}

impl FakeAccount {
    /// `GET /api/v1/files/count`: the counter row. Only `trashed_files` is the account's real count of trashed items.
    pub(super) fn files_count(&self) -> Reply {
        let trashed = self
            .items
            .lock()
            .expect("vault items")
            .values()
            .filter(|item| item.is_trashed)
            .count();
        Reply::ok(json!({ "trashed_files": trashed }))
    }

    /// Whether `request` is one of the search-index shard routes.
    pub(super) fn is_search_shard_route(request: &Request) -> bool {
        let path = request.path.split('?').next().unwrap_or("");
        path == SHARDS || path.starts_with(&format!("{SHARDS}/"))
    }

    /// Answers a search-index shard route (the caller has checked the token).
    pub(super) fn search_shard_route(&self, request: &Request) -> Reply {
        let path = request.path.split('?').next().unwrap_or("");
        let state = &self.search_shards;
        let Some(rest) = path.strip_prefix(SHARDS) else {
            return Reply::error(404, "not_found");
        };
        if rest.is_empty() {
            return match request.method.as_str() {
                "GET" => {
                    let shards = state.shards.lock().expect("shards");
                    let list: Vec<_> = shards
                        .iter()
                        .map(|(&(bucket, page), (version, _))| json!({ "bucket": bucket, "page": page, "version": version }))
                        .collect();
                    Reply::ok(json!({ "shards": list }))
                }
                _ => Reply::error(405, "method_not_allowed"),
            };
        }
        let coordinate = rest
            .strip_prefix('/')
            .and_then(|r| r.split_once('/'))
            .and_then(|(bucket, page)| Some((bucket.parse::<u32>().ok()?, page.parse::<u32>().ok()?)));
        let Some(key) = coordinate else {
            return Reply::error(400, "bad_shard");
        };
        let mut shards = state.shards.lock().expect("shards");
        match request.method.as_str() {
            "GET" => match shards.get(&key) {
                Some((_, blob)) => Reply::bytes(200, blob.clone()),
                None => Reply::json(404, json!({ "error": "not_found" })),
            },
            "PUT" => {
                if request.body.is_empty() {
                    return Reply::error(400, "empty_shard");
                }
                if request.body.len() > MAX_SEARCH_SHARD_BLOB_BYTES {
                    return Reply::error(413, "shard_too_large");
                }
                let mut next = state.next_version.lock().expect("version");
                *next += 1;
                shards.insert(key, (*next, request.body.clone()));
                Reply::ok(json!({ "bucket": key.0, "page": key.1, "version": *next }))
            }
            "DELETE" => {
                if shards.remove(&key).is_some() {
                    Reply::bytes(204, Vec::new())
                } else {
                    Reply::json(404, json!({ "error": "not_found" }))
                }
            }
            _ => Reply::error(405, "method_not_allowed"),
        }
    }
}
