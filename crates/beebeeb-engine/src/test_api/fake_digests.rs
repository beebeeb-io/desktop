//! `GET /api/v1/folder-digests` on the fake server: the digests of the [`FakeAccount`] tree, computed with core's
//! `folder_digest` from the same fields the fake listing serves (the name ciphertext string, size, version number,
//! current object version id; nothing is uploading and no row carries an owner-wrapped key in the fake), paged by
//! id ascending with a keyset cursor, as the server's endpoint (task 06) answers.
//!
//! Two switches a test sets: answer as an old server (the generic 404 of an unknown route) and answer 503
//! `digests_not_ready` with a `Retry-After`.
//!
//! Tests only (`test` or the `test-support` feature).

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use beebeeb_core::folder_digest::{self as fd, ChildRecord, Digest};
use serde_json::{Value, json};
use uuid::Uuid;

use super::fake_account::{FakeAccount, VaultItem};
use super::{Reply, Request};

/// The endpoint's default and largest page.
const DEFAULT_LIMIT: usize = 500;
const MAX_LIMIT: usize = 1000;

/// What a test switched on for one account (by its user id: the fake account has no field for it).
#[derive(Debug, Default, Clone, Copy)]
struct Switches {
    old_server: bool,
    not_ready: Option<u64>,
}

fn switches() -> &'static Mutex<HashMap<String, Switches>> {
    static SWITCHES: OnceLock<Mutex<HashMap<String, Switches>>> = OnceLock::new();
    SWITCHES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The UUID an item id goes on the wire as: the id itself when it is one; else (a few older fixtures name items
/// `folder-…`) a stable UUID derived from it, so every item has one.
pub fn wire_uuid(id: &str) -> Uuid {
    Uuid::parse_str(id).unwrap_or_else(|_| {
        let hash = fd::name_hash(id);
        let mut bytes = [0u8; 16];
        bytes.copy_from_slice(&hash[..16]);
        Uuid::from_bytes(bytes)
    })
}

/// The live tree: children by parent (`None` = the root), and each folder's digest computed bottom-up on demand.
struct Tree<'a> {
    children: HashMap<Option<&'a str>, Vec<&'a VaultItem>>,
    digests: HashMap<Option<&'a str>, Digest>,
}

impl<'a> Tree<'a> {
    fn new(items: &'a HashMap<String, VaultItem>) -> Self {
        let mut children: HashMap<Option<&str>, Vec<&VaultItem>> = HashMap::new();
        for item in items.values().filter(|item| !item.is_trashed) {
            children.entry(item.parent_id.as_deref()).or_default().push(item);
        }
        for list in children.values_mut() {
            list.sort_by_key(|item| wire_uuid(&item.id));
        }
        Self {
            children,
            digests: HashMap::new(),
        }
    }

    fn children(&self, folder: Option<&'a str>) -> &[&'a VaultItem] {
        self.children.get(&folder).map(Vec::as_slice).unwrap_or(&[])
    }

    fn digest(&mut self, folder: Option<&'a str>) -> Digest {
        if let Some(digest) = self.digests.get(&folder) {
            return *digest;
        }
        let kids: Vec<&'a VaultItem> = self.children(folder).to_vec();
        let records = kids
            .into_iter()
            .map(|item| {
                let id = wire_uuid(&item.id);
                let name = fd::name_hash(&item.name_encrypted);
                if item.is_folder {
                    ChildRecord::folder(id, name, self.digest(Some(item.id.as_str())))
                } else {
                    ChildRecord::file(
                        id,
                        false,
                        false,
                        item.size_bytes as i64,
                        i64::from(item.version_number),
                        item.object_version_id.as_deref().map(wire_uuid),
                        name,
                    )
                }
            })
            .collect();
        let digest = fd::folder_digest(records);
        self.digests.insert(folder, digest);
        digest
    }
}

impl FakeAccount {
    fn digest_switches(&self) -> Switches {
        switches()
            .lock()
            .expect("digest switches")
            .get(&self.user_id)
            .copied()
            .unwrap_or_default()
    }

    fn set_digest_switches(&self, change: impl FnOnce(&mut Switches)) {
        let mut all = switches().lock().expect("digest switches");
        change(all.entry(self.user_id.clone()).or_default());
    }

    /// From now on `GET /api/v1/folder-digests` answers as a server without the endpoint (the generic 404).
    pub fn digests_as_old_server(&self, on: bool) {
        self.set_digest_switches(|s| s.old_server = on);
    }

    /// From now on `GET /api/v1/folder-digests` answers 503 `digests_not_ready` with this `Retry-After` (`None`:
    /// answers again).
    pub fn digests_not_ready(&self, retry_after_secs: Option<u64>) {
        self.set_digest_switches(|s| s.not_ready = retry_after_secs);
    }

    /// The digest hex of every live folder by its wire id (UUID string), and of the root under `""`: what a caller
    /// that read the whole tree right now would hold.
    pub fn folder_digests_now(&self) -> HashMap<String, String> {
        let items = self.items.lock().expect("vault items");
        let mut tree = Tree::new(&items);
        let mut out = HashMap::new();
        out.insert(String::new(), fd::to_hex(&tree.digest(None)));
        for item in items.values().filter(|item| item.is_folder && !item.is_trashed) {
            out.insert(
                wire_uuid(&item.id).to_string(),
                fd::to_hex(&tree.digest(Some(item.id.as_str()))),
            );
        }
        out
    }

    /// `GET /api/v1/folder-digests?folder_id=&cursor=&limit=`.
    pub(super) fn folder_digests_route(&self, req: &Request) -> Reply {
        if req.bearer() != Some(&self.session_token) {
            return Reply::error(401, "unauthorized");
        }
        let switches = self.digest_switches();
        if switches.old_server {
            return Reply::error(404, "not_found");
        }
        if let Some(seconds) = switches.not_ready {
            return Reply::error(503, "digests_not_ready").retry_after(seconds);
        }
        let limit = match req.query("limit") {
            None => DEFAULT_LIMIT,
            Some(raw) => match raw.parse::<usize>() {
                Ok(limit) if limit <= MAX_LIMIT => limit,
                _ => return Reply::error(400, "bad_limit"),
            },
        };
        let cursor = match req.query("cursor") {
            None => None,
            Some(raw) => match Uuid::parse_str(raw) {
                Ok(cursor) => Some(cursor),
                Err(_) => return Reply::error(400, "bad_cursor"),
            },
        };
        let items = self.items.lock().expect("vault items");
        let folder: Option<&str> = match req.query("folder_id") {
            None => None,
            Some(raw) => {
                let wanted = Uuid::parse_str(raw).ok();
                let found = items.values().find(|item| {
                    item.is_folder && !item.is_trashed && (item.id == raw || Some(wire_uuid(&item.id)) == wanted)
                });
                match found {
                    Some(item) => Some(item.id.as_str()),
                    None => return Reply::error(404, "folder_not_found"),
                }
            }
        };
        let mut tree = Tree::new(&items);
        let digest = tree.digest(folder);
        let all: Vec<&VaultItem> = tree.children(folder).to_vec();
        let child_count = all.len();
        let (children, next_cursor) = if limit == 0 {
            (Vec::new(), None)
        } else {
            let after: Vec<&VaultItem> = all
                .into_iter()
                .filter(|item| cursor.is_none_or(|c| wire_uuid(&item.id) > c))
                .collect();
            let more = after.len() > limit;
            let page: Vec<&VaultItem> = after.into_iter().take(limit).collect();
            let next = more.then(|| wire_uuid(&page[page.len() - 1].id).to_string());
            let children: Vec<Value> = page
                .into_iter()
                .map(|item| {
                    json!({
                        "id": wire_uuid(&item.id),
                        "is_folder": item.is_folder,
                        "digest": item.is_folder.then(|| fd::to_hex(&tree.digest(Some(item.id.as_str())))),
                    })
                })
                .collect();
            (children, next)
        };
        Reply::ok(json!({
            "folder_id": folder.map(wire_uuid),
            "algorithm": fd::ALGORITHM,
            "digest": fd::to_hex(&digest),
            "child_count": child_count,
            "children": children,
            "next_cursor": next_cursor,
        }))
    }
}
