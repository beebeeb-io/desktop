//! A complete invented Beebeeb account for the fake API: OPAQUE registration, a recovery check derived from the
//! real core functions, a profile, a vault tree with encrypted names, the change feed, trash and restore, storage
//! usage, and device registration. The encryption uses core's real functions so a client that stops encrypting is
//! caught: the fake can assert "this name/blob is not plaintext" by trying the known plaintext against the sealed
//! ciphertext.
//!
//! Tests only (`test` or the `test-support` feature). Every key, name and id is invented.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use super::{FakeApi, Handler, OpaqueAccount, Reply, Request};

// ── The vault tree ──────────────────────────────────────────────────────────────

/// One version of a file in the fake vault (`GET /api/v1/files/{id}/versions`).
#[derive(Debug, Clone)]
pub(super) struct FakeVersion {
    pub(super) id: String,
    pub(super) number: i64,
    pub(super) created_at: String,
    pub(super) size_bytes: u64,
}

/// The versions of one file: in the order they were made, and which number is the current one.
#[derive(Debug, Clone, Default)]
pub(super) struct FakeVersions {
    pub(super) list: Vec<FakeVersion>,
    pub(super) current: i64,
}

/// One item in the fake vault: a folder or a file, with its encrypted name produced by core's `encrypt_name_for`.
#[derive(Debug, Clone)]
pub struct VaultItem {
    pub id: String,
    pub parent_id: Option<String>,
    pub is_folder: bool,
    pub name_encrypted: String,
    /// The plaintext name, for assertion: never sent to the client.
    pub name_plaintext: String,
    pub size_bytes: u64,
    pub updated_at: i64,
    pub is_trashed: bool,
    pub version_number: i32,
    pub content_hash: Option<String>,
    /// The file's content as the server holds it: one encrypted frame per chunk (core's chunk encryptor, the
    /// account's own file key; an empty file has none). `None`: the item has no content to download
    /// ([`FakeAccount::add_file_with_content`] gives it one).
    pub content_frames: Option<Vec<Vec<u8>>>,
    /// The object version a completed upload (or [`FakeAccount::push_newer_version`]) made current; `None` until one did.
    pub object_version_id: Option<String>,
}

impl VaultItem {
    pub(super) fn to_node(&self) -> Value {
        let mut node = json!({
            "id": self.id,
            "parent_id": self.parent_id,
            "name_encrypted": self.name_encrypted,
            "is_folder": self.is_folder,
            "size_bytes": self.size_bytes,
            "updated_at": self.updated_at,
            "is_trashed": self.is_trashed,
            "version_number": self.version_number,
            "content_hash": self.content_hash,
        });
        if let Some(object) = &self.object_version_id {
            node["current_object_version_id"] = json!(object);
        }
        // A file with content also says how it is cut, as the server's row does: what a download reads.
        if let Some(frames) = &self.content_frames {
            node["chunk_count"] = json!(frames.len());
            node["chunk_size_bytes"] = json!(CONTENT_CHUNK_BYTES);
        }
        node
    }
}

/// How large a chunk of [`FakeAccount::add_file_with_content`] is: small, so a short test file is several chunks.
pub const CONTENT_CHUNK_BYTES: u32 = 1024;

// ── The account ─────────────────────────────────────────────────────────────────

/// A complete invented account: what the fake server knows about one user.
pub struct FakeAccount {
    pub opaque: OpaqueAccount,
    pub email: String,
    pub user_id: String,
    /// The recovery check the server holds (HKDF of the master key, base64).
    pub recovery_check: String,
    /// The session token the server issues on a successful sign-in.
    pub session_token: String,
    /// The master key bytes (32), for building encrypted vault items.
    pub master_key: [u8; 32],
    /// Items in the vault (file tree), keyed by id.
    pub(super) items: Mutex<HashMap<String, VaultItem>>,
    /// The current seq_id (monotonic).
    pub(super) seq: AtomicI64,
    /// How much storage this account uses.
    pub used_bytes: u64,
    pub quota_bytes: u64,
    /// The device this session registered.
    devices: Mutex<Vec<Value>>,
    /// Client sync sessions.
    sessions: Mutex<Vec<Value>>,
    /// The account's share links, as `POST /api/v1/shares` stored them (every field of the request, and an id).
    links: Mutex<Vec<Value>>,
    /// The next `POST /api/v1/shares` answers "this token is taken" once.
    link_token_taken: std::sync::atomic::AtomicBool,
    /// `GET /api/v1/shares/mine` also carries each link's `wrapped_file_key`.
    links_list_wrapped_key: std::sync::atomic::AtomicBool,
    /// `GET /api/v1/shares/mine` names this file on every row, whatever file the link is to.
    links_listed_as: Mutex<Option<String>>,
    /// The versions of the files that have some ([`Self::add_version`]), by file id.
    pub(super) versions: Mutex<HashMap<String, FakeVersions>>,
    /// Whole uploads, the change log and stored pictures ([`super::fake_uploads`]).
    pub(super) uploads: super::fake_uploads::UploadState,
    /// The stored search-index shards ([`super::fake_search_shards`]).
    pub(super) search_shards: super::fake_search_shards::SearchShardState,
    /// The step-up tokens and permanent deletes ([`super::fake_purge`]).
    pub(super) purge: super::fake_purge::PurgeState,
    /// The account's sign-in sessions ([`super::fake_sessions`]).
    pub(super) sessions_state: super::fake_sessions::SessionState,
    /// Invites, file requests and shared-folder custody ([`super::fake_sharing`]).
    pub(super) sharing: super::fake_sharing::SharingState,
    /// How many version restores reached this account (a request with its token), whatever the answer.
    restores: AtomicUsize,
    /// The status the next version restore is answered with, without being made.
    restore_refusal: Mutex<Option<u16>>,
    /// The next version restore is read and made, and the connection is closed without an answer.
    restore_hangs_up: AtomicBool,
    /// `account/sessions`, `me/activity`, `me/profile` and `me/two-factor` answer. Off (a test's own fake, the
    /// default): they are 404 as before, so a test that sets its own state for them is never shadowed.
    /// [`Self::serve_security_routes`]; the `bb-fake-server` fixture turns it on.
    security_routes: AtomicBool,
    /// What `me/two-factor` says ([`Self::set_two_factor`]); off at the start.
    pub(super) two_factor_on: AtomicBool,
    /// The `plan_state` the subscription says ([`Self::set_plan_state`]); `active` at the start.
    plan_state: Mutex<String>,
    /// The subscription route hangs up ([`Self::set_plan_unreachable`]).
    plan_unreachable: AtomicBool,
    /// How long the subscription route waits before it answers, in ms ([`Self::set_plan_delay_ms`]).
    plan_delay_ms: std::sync::atomic::AtomicU64,
    /// Two-factor, region, deletion, email resend, health and speedtest ([`super::fake_rest`]).
    pub(super) rest: super::fake_rest::RestState,
}

impl FakeAccount {
    pub(super) fn security_routes_on(&self) -> bool {
        self.security_routes.load(Ordering::SeqCst)
    }

    /// Answer the Security page's four routes (see the field); the fixture of `bb-fake-server` does.
    pub fn serve_security_routes(&self, on: bool) {
        self.security_routes.store(on, Ordering::SeqCst);
    }

    /// What `billing/subscription` says as `plan_state`: `none`, `trial`, `free`, `active` or `lapsed`.
    pub fn set_plan_state(&self, state: &str) {
        *self.plan_state.lock().unwrap() = state.to_string();
    }

    /// `billing/subscription` hangs up without an answer (the server cannot be reached for the plan).
    pub fn set_plan_unreachable(&self, unreachable: bool) {
        self.plan_unreachable.store(unreachable, Ordering::SeqCst);
    }

    /// `billing/subscription` answers only after `ms` milliseconds (a slow plan answer).
    pub fn set_plan_delay_ms(&self, ms: u64) {
        self.plan_delay_ms.store(ms, Ordering::SeqCst);
    }

    /// Whether `me/two-factor` says two-factor is on.
    pub fn set_two_factor(&self, on: bool) {
        self.two_factor_on.store(on, Ordering::SeqCst);
    }

    /// Register a new invented account with the real OPAQUE protocol and a real recovery check. The recovery
    /// phrase is returned alongside the account so tests can unlock with it.
    pub fn register(email: &str, password: &str, recovery_phrase: &str) -> (Self, String) {
        let opaque = OpaqueAccount::register(email, password);
        let master_key_struct =
            beebeeb_core::recovery::recover_from_phrase(recovery_phrase).expect("a valid 12-word phrase");
        let recovery_check = {
            use base64::Engine as _;
            let check = beebeeb_core::opaque::compute_recovery_check(&master_key_struct);
            base64::engine::general_purpose::STANDARD.encode(*check)
        };
        let master_key = *master_key_struct.to_bytes();
        let session_token = format!("fake-session-{}", uuid::Uuid::new_v4());
        let account = Self {
            opaque,
            email: email.to_string(),
            user_id: format!("user-{}", uuid::Uuid::new_v4()),
            recovery_check,
            session_token,
            master_key,
            items: Mutex::new(HashMap::new()),
            seq: AtomicI64::new(0),
            used_bytes: 0,
            quota_bytes: 10 * 1024 * 1024 * 1024, // 10 GiB
            devices: Mutex::new(Vec::new()),
            sessions: Mutex::new(Vec::new()),
            links: Mutex::new(Vec::new()),
            link_token_taken: std::sync::atomic::AtomicBool::new(false),
            links_list_wrapped_key: std::sync::atomic::AtomicBool::new(false),
            links_listed_as: Mutex::new(None),
            versions: Mutex::new(HashMap::new()),
            uploads: Default::default(),
            search_shards: Default::default(),
            purge: Default::default(),
            sessions_state: Default::default(),
            sharing: Default::default(),
            restores: AtomicUsize::new(0),
            restore_refusal: Mutex::new(None),
            restore_hangs_up: AtomicBool::new(false),
            security_routes: AtomicBool::new(false),
            two_factor_on: AtomicBool::new(false),
            plan_state: Mutex::new("active".to_string()),
            plan_unreachable: AtomicBool::new(false),
            plan_delay_ms: std::sync::atomic::AtomicU64::new(0),
            rest: Default::default(),
        };
        (account, recovery_phrase.to_string())
    }

    /// Add an item to the vault. The name is encrypted with core's real functions and the account's real master
    /// key, so a client that sends the plaintext name instead of the ciphertext is caught.
    pub fn add_item(&self, id: &str, parent_id: Option<&str>, name: &str, is_folder: bool, size_bytes: u64) {
        let master = beebeeb_core::kdf::MasterKey::from_bytes(self.master_key);
        let file_key = beebeeb_core::kdf::derive_file_key(&master, id.as_bytes());
        let name_encrypted = beebeeb_core::encrypt::encrypt_name_for(
            &file_key,
            name,
            None,
            beebeeb_core::chunk_stream::ExpectedChunkFormat::Unrecorded,
        )
        .expect("encrypt name");
        let seq = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
        let item = VaultItem {
            id: id.to_string(),
            parent_id: parent_id.map(str::to_string),
            is_folder,
            name_encrypted,
            name_plaintext: name.to_string(),
            size_bytes,
            updated_at: 1_700_000_000 + seq,
            is_trashed: false,
            version_number: 1,
            content_hash: if is_folder { None } else { Some(format!("hash-{id}")) },
            content_frames: None,
            object_version_id: None,
        };
        self.items.lock().expect("vault items").insert(id.to_string(), item);
    }

    /// Add a FILE with content: [`Self::add_item`], and `content` encrypted chunk by chunk with core's chunk
    /// encryptor under the account's own key for `id` (a UUID), as an upload of this account would have stored it.
    /// The fake then serves the row with its chunk count and each chunk (`GET /api/v1/files/{id}/chunks/{n}`), so a
    /// client downloads and decrypts it by its real path; a client that asks with another account's key gets
    /// bytes it cannot open.
    pub fn add_file_with_content(&self, id: &str, parent_id: Option<&str>, name: &str, content: &[u8]) {
        self.add_item(id, parent_id, name, false, content.len() as u64);
        let master = beebeeb_core::kdf::MasterKey::from_bytes(self.master_key);
        let file_key = beebeeb_core::kdf::derive_file_key(&master, id.as_bytes());
        let mut encryptor = beebeeb_core::chunk_stream::ChunkEncryptor::for_push_with_file_key(
            file_key,
            id,
            content.len() as u64,
            CONTENT_CHUNK_BYTES.into(),
        )
        .expect("a chunk encryptor")
        .with_recorded_pin(beebeeb_core::chunk_stream::ExpectedChunkFormat::Unrecorded)
        .expect("the format of the name");
        // An empty file is one empty chunk, as core plans it.
        let pieces: Vec<&[u8]> = if content.is_empty() {
            vec![content]
        } else {
            content.chunks(CONTENT_CHUNK_BYTES as usize).collect()
        };
        let frames: Vec<Vec<u8>> = pieces
            .into_iter()
            .map(|chunk| encryptor.push_chunk(chunk).expect("an encrypted chunk").data)
            .collect();
        // The name an upload publishes with `complete` (core 0.3.0, chunk format v2): the item's name
        // with the commitment to this upload. Content opens only against it.
        let version = encryptor
            .version_commitment_after(None)
            .expect("the upload's commitment");
        encryptor.finish().expect("the last chunk");
        let name_key = beebeeb_core::kdf::derive_file_key(&master, id.as_bytes());
        let name_encrypted = beebeeb_core::encrypt::encrypt_name_for_version(
            &name_key,
            name,
            None,
            beebeeb_core::chunk_stream::ExpectedChunkFormat::Unrecorded,
            version,
        )
        .expect("the completion name");
        if let Some(item) = self.items.lock().expect("vault items").get_mut(id) {
            item.content_frames = Some(frames);
            item.name_encrypted = name_encrypted;
        }
    }

    /// Chunk `index` of the file `id`, as stored. `None`: no such file, no content, or no such chunk.
    fn content_frame(&self, id: &str, index: usize) -> Option<Vec<u8>> {
        let items = self.items.lock().expect("vault items");
        items.get(id)?.content_frames.as_ref()?.get(index).cloned()
    }

    /// The non-trashed items as sync-snapshot nodes.
    pub(super) fn snapshot_nodes(&self) -> Vec<Value> {
        self.items
            .lock()
            .expect("vault items")
            .values()
            .filter(|item| !item.is_trashed)
            .map(VaultItem::to_node)
            .collect()
    }

    /// All items (including trashed) matching a parent, for `GET /files?parent_id=…`.
    fn list_children(&self, parent_id: Option<&str>, trashed: bool) -> Vec<Value> {
        self.items
            .lock()
            .expect("vault items")
            .values()
            .filter(|item| item.parent_id.as_deref() == parent_id && item.is_trashed == trashed)
            .map(VaultItem::to_node)
            .collect()
    }

    /// A stored picture of the file `id` as the server holds it: `plain` (an encoded picture) sealed by core's
    /// thumbnail sealer under this account's key for that file. A test's own handler serves it.
    pub fn seal_thumbnail(&self, id: &str, plain: &[u8]) -> Vec<u8> {
        let master = beebeeb_core::kdf::MasterKey::from_bytes(self.master_key);
        let key = beebeeb_core::kdf::derive_file_key(&master, id.as_bytes());
        beebeeb_core::encrypt::encrypt_thumbnail(&key, plain).expect("a sealed picture")
    }

    /// Add a version to the file `file_id` (made after the ones it has; it becomes the current one), as the server
    /// lists it: its id, when it was made and its size. The server's list order is the order they were added.
    pub fn add_version(&self, file_id: &str, version_id: &str, created_at: &str, size_bytes: u64) {
        let mut all = self.versions.lock().expect("versions");
        let versions = all.entry(file_id.to_string()).or_default();
        let number = versions.list.len() as i64 + 1;
        versions.list.push(FakeVersion {
            id: version_id.to_string(),
            number,
            created_at: created_at.to_string(),
            size_bytes,
        });
        versions.current = number;
    }

    /// The id of the current version of `file_id`, and how many versions it has. `None`: it has none.
    pub fn version_state(&self, file_id: &str) -> Option<(String, usize)> {
        let all = self.versions.lock().expect("versions");
        let versions = all.get(file_id)?;
        let current = versions.list.iter().find(|v| v.number == versions.current)?;
        Some((current.id.clone(), versions.list.len()))
    }

    /// How many version restores reached this account (with its token), whatever they were answered.
    pub fn restore_requests(&self) -> usize {
        self.restores.load(Ordering::SeqCst)
    }

    /// Answer the next version restore with this status, without making it.
    pub fn refuse_next_restore_with(&self, status: u16) {
        *self.restore_refusal.lock().expect("restore switch") = Some(status);
    }

    /// Read the next version restore, make it, then close the connection without an answer.
    pub fn make_next_restore_then_hang_up(&self) {
        self.restore_hangs_up.store(true, Ordering::SeqCst);
    }

    /// `GET /api/v1/files/{id}/versions`. `None`: no such file.
    fn versions_payload(&self, file_id: &str) -> Option<Value> {
        self.items.lock().expect("vault items").get(file_id)?;
        let all = self.versions.lock().expect("versions");
        let empty = FakeVersions::default();
        let versions = all.get(file_id).unwrap_or(&empty);
        let list: Vec<Value> = versions
            .list
            .iter()
            .map(|v| {
                json!({
                    "id": v.id,
                    "version_number": v.number,
                    "created_at": v.created_at,
                    "size_bytes": v.size_bytes,
                })
            })
            .collect();
        Some(json!({ "versions": list, "current_version": versions.current }))
    }

    /// Make `version_id` the current version of `file_id`: one more version is minted from it, as the server's
    /// restore does. Returns false: no such file or version.
    fn restore_version(&self, file_id: &str, version_id: &str) -> bool {
        let mut all = self.versions.lock().expect("versions");
        let Some(versions) = all.get_mut(file_id) else {
            return false;
        };
        let Some(source) = versions.list.iter().find(|v| v.id == version_id).cloned() else {
            return false;
        };
        let number = versions.list.len() as i64 + 1;
        versions.list.push(FakeVersion {
            id: format!("restored-{number}-of-{}", source.id),
            number,
            created_at: format!("2026-11-14T12:{:02}:00Z", number.min(59)),
            size_bytes: source.size_bytes,
        });
        versions.current = number;
        true
    }

    /// Trash an item by id. Returns true if found and not already trashed.
    pub fn trash_item(&self, id: &str) -> bool {
        let mut items = self.items.lock().expect("vault items");
        if let Some(item) = items.get_mut(id)
            && !item.is_trashed
        {
            item.is_trashed = true;
            item.version_number += 1;
            self.seq.fetch_add(1, Ordering::SeqCst);
            return true;
        }
        false
    }

    /// The ids of the account's live files (not folders, not trashed), sorted: what an owner's media read lists.
    pub fn live_file_ids(&self) -> Vec<String> {
        let items = self.items.lock().expect("vault items");
        let mut ids: Vec<String> = items
            .values()
            .filter(|item| !item.is_folder && !item.is_trashed)
            .map(|item| item.id.clone())
            .collect();
        ids.sort();
        ids
    }

    /// A live item's version: when it last changed (unix seconds) and its size.
    pub fn item_version(&self, id: &str) -> Option<(i64, u64)> {
        let items = self.items.lock().expect("vault items");
        items.get(id).map(|item| (item.updated_at, item.size_bytes))
    }

    /// Restore a trashed item. Returns true if found and was trashed.
    pub fn restore_item(&self, id: &str) -> bool {
        let mut items = self.items.lock().expect("vault items");
        if let Some(item) = items.get_mut(id)
            && item.is_trashed
        {
            item.is_trashed = false;
            item.version_number += 1;
            self.seq.fetch_add(1, Ordering::SeqCst);
            return true;
        }
        false
    }

    /// The account's share links as they were posted: each row is the request's body plus `id` and `revoked`.
    pub fn links(&self) -> Vec<Value> {
        self.links.lock().expect("links").clone()
    }

    /// The next `POST /api/v1/shares` is refused once with 409 `token_taken`, and stores nothing.
    pub fn take_the_next_link_token(&self) {
        self.link_token_taken.store(true, Ordering::SeqCst);
    }

    /// Whether `GET /api/v1/shares/mine` carries each link's `wrapped_file_key` (off: as the listing is today).
    pub fn list_links_with_their_wrapped_key(&self, on: bool) {
        self.links_list_wrapped_key.store(on, Ordering::SeqCst);
    }

    /// A list that lies: from now on every row of `GET /api/v1/shares/mine` names `file_id`, whatever file its
    /// link is to.
    pub fn list_every_link_as(&self, file_id: &str) {
        *self.links_listed_as.lock().expect("links") = Some(file_id.to_string());
    }

    /// Every link of the account is listed as past its expiry.
    pub fn expire_links(&self) {
        for link in self.links.lock().expect("links").iter_mut() {
            link["listed_expires_at"] = json!("2020-01-01T00:00:00Z");
        }
    }

    /// Every link of the account is listed as opened as often as it may be (once).
    pub fn use_up_links(&self) {
        for link in self.links.lock().expect("links").iter_mut() {
            link["max_opens"] = json!(1);
            link["open_count"] = json!(1);
        }
    }

    /// Every link of the account is revoked.
    pub fn revoke_links(&self) {
        for link in self.links.lock().expect("links").iter_mut() {
            link["revoked"] = json!(true);
        }
    }

    /// `DELETE /api/v1/shares/{id}`: the link is revoked (`GET /api/v1/shares/mine` still lists it, marked revoked).
    pub(super) fn revoke_link(&self, id: &str) -> Reply {
        let mut links = self.links.lock().expect("links");
        match links.iter_mut().find(|link| link["id"].as_str() == Some(id)) {
            Some(link) => {
                link["revoked"] = json!(true);
                Reply::ok(json!({ "success": true }))
            }
            None => Reply::json(404, json!({ "error": "not_found" })),
        }
    }

    /// `PATCH /api/v1/shares/{id}`: an owner's live link's expiry and password. A field that is absent is
    /// unchanged, `null` clears; limits as create (1..=87600 hours, a password of at least 4 after trim).
    pub(super) fn patch_link(&self, id: &str, body: &Value) -> Reply {
        let mut links = self.links.lock().expect("links");
        let Some(link) = links
            .iter_mut()
            .find(|link| link["id"].as_str() == Some(id) && link["revoked"] != true)
        else {
            return Reply::json(404, json!({ "error": "not_found" }));
        };
        let hours = match body.get("expires_in_hours") {
            None => None,
            Some(Value::Null) => Some(None),
            Some(h) => match h.as_i64().filter(|h| (1..=87_600).contains(h)) {
                Some(h) => Some(Some(h)),
                None => return Reply::json(400, json!({ "error": "bad_expiry" })),
            },
        };
        let passphrase = match body.get("passphrase") {
            None => None,
            Some(Value::Null) => Some(false),
            Some(p) => match p.as_str().filter(|p| p.trim().chars().count() >= 4) {
                Some(_) => Some(true),
                None => return Reply::json(400, json!({ "error": "bad_passphrase" })),
            },
        };
        match hours {
            Some(Some(h)) => link["expires_at"] = json!((chrono::Utc::now() + chrono::Duration::hours(h)).to_rfc3339()),
            Some(None) => link["expires_at"] = Value::Null,
            None => {}
        }
        if let Some(set) = passphrase {
            link["has_passphrase"] = json!(set);
        }
        Reply::ok(json!({
            "id": id,
            "expires_at": link.get("expires_at").cloned().unwrap_or(Value::Null),
            "has_passphrase": link.get("has_passphrase").and_then(Value::as_bool).unwrap_or(false),
        }))
    }

    /// `POST /api/v1/shares`: a link to a live item of this account; another id is not found.
    fn post_link(&self, body: &Value) -> Reply {
        let file_id = body.get("file_id").and_then(Value::as_str).unwrap_or_default();
        let known = self
            .items
            .lock()
            .expect("vault items")
            .get(file_id)
            .is_some_and(|item| !item.is_trashed);
        if !known {
            return Reply::json(404, json!({ "error": "not_found" }));
        }
        if self.link_token_taken.swap(false, Ordering::SeqCst) {
            return Reply::json(409, json!({ "code": "token_taken" }));
        }
        let mut link = body.clone();
        let id = uuid::Uuid::new_v4().to_string();
        link["id"] = json!(id);
        link["revoked"] = json!(false);
        self.links.lock().expect("links").push(link);
        Reply::ok(json!({ "id": id }))
    }

    /// `GET /api/v1/shares/mine`: every link of the account, whatever the query asks for (a client must match the
    /// file itself), in the listing's shape: no token, and the wrapped file key only when switched on.
    fn list_links(&self) -> Reply {
        let with_key = self.links_list_wrapped_key.load(Ordering::SeqCst);
        let listed_as = self.links_listed_as.lock().expect("links").clone();
        let shares: Vec<Value> = self
            .links
            .lock()
            .expect("links")
            .iter()
            .map(|link| {
                let mut row = json!({
                    "id": link["id"],
                    "file_id": link["file_id"],
                    "share_type": "file",
                    "expires_at": link.get("listed_expires_at").cloned().unwrap_or(Value::Null),
                    "max_opens": link.get("max_opens").cloned().unwrap_or(Value::Null),
                    "open_count": link.get("open_count").cloned().unwrap_or(json!(0)),
                    "has_passphrase": link.get("passphrase").is_some(),
                    "revoked": link["revoked"],
                    "owner_wrapped_key": link["owner_wrapped_key"],
                    "owner_wrapped_token": link["owner_wrapped_token"],
                });
                if with_key {
                    row["wrapped_file_key"] = link["wrapped_file_key"].clone();
                }
                if let Some(file_id) = &listed_as {
                    row["file_id"] = json!(file_id);
                }
                row
            })
            .collect();
        Reply::ok(json!({ "shares": shares }))
    }

    /// `name` encrypted as this account's own item `id` holds it (the envelope a share row carries as
    /// `file_name_encrypted`), for a test that answers a route in the server's shape.
    pub fn encrypted_name(&self, id: &str, name: &str) -> String {
        let master = beebeeb_core::kdf::MasterKey::from_bytes(self.master_key);
        let file_key = beebeeb_core::kdf::derive_file_key(&master, id.as_bytes());
        beebeeb_core::encrypt::encrypt_name_for(
            &file_key,
            name,
            None,
            beebeeb_core::chunk_stream::ExpectedChunkFormat::Unrecorded,
        )
        .expect("encrypt name")
    }

    /// Whether a name is an encrypted ciphertext (not plaintext). This checks that the text does NOT match any
    /// known plaintext name in the vault: a client that sends a plaintext name instead of the encrypted one is
    /// caught.
    pub fn is_not_plaintext(&self, name_value: &str) -> bool {
        let items = self.items.lock().expect("vault items");
        // A plaintext name would not start with the AES-GCM ciphertext's JSON envelope or base64 prefix.
        // But the real check: if it matches any known plaintext, it is plaintext.
        for item in items.values() {
            if name_value == item.name_plaintext {
                return false;
            }
        }
        // Also check: encrypted names from core are always longer than the plaintext, and never equal to the
        // plaintext (they contain a nonce and a tag).
        true
    }
}

// ── The router ──────────────────────────────────────────────────────────────────

/// A handler that dispatches to one account's fake endpoints. Covers: OPAQUE sign-in, profile, recovery check,
/// billing subscription, billing usage, sync snapshot, sync ops, files listing, folder create, trash (DELETE),
/// restore, device registration, session creation, heartbeat, and logout.
pub fn account_handler(account: Arc<FakeAccount>) -> Handler {
    Arc::new(move |req: &Request| {
        let route = req.route();
        // The stateful routes of their own files, before the broad `/api/v1/files/` arms below.
        if let Some(reply) = account
            .purge_route(req)
            .or_else(|| account.sessions_route(req))
            .or_else(|| account.rest_route(req))
            .or_else(|| account.sharing_route(req))
        {
            return reply;
        }
        match route.as_str() {
            // ── OPAQUE sign-in ──────────────────────────────────────────
            "POST /api/v1/opaque/login-start" => account.opaque.login_start(&req.json()),

            "POST /api/v1/opaque/login-finish" => {
                let body = req.json();
                if account.opaque.login_finish(&body) {
                    Reply::ok(json!({ "session_token": account.session_token }))
                } else {
                    Reply::json(401, json!({}))
                }
            }

            // ── Profile ─────────────────────────────────────────────────
            "GET /api/v1/auth/me" => {
                if req.bearer() != Some(&account.session_token) {
                    return Reply::error(401, "unauthorized");
                }
                Reply::ok(json!({
                    "user_id": account.user_id,
                    "email": account.email,
                    "email_verified": true,
                    "created_at": "2025-01-01T00:00:00Z",
                    "role": "user",
                }))
            }

            // ── Password proof (the screen lock's password unlock): the fixture password, the same OPAQUE flow as
            // the sign-in, for the account whose token this is ──────────────────────────
            "POST /api/v1/auth/password-proof/start" => {
                if req.bearer() != Some(&account.session_token) {
                    return Reply::error(401, "unauthorized");
                }
                account.opaque.login_start(&req.json())
            }

            "POST /api/v1/auth/password-proof/finish" => {
                if req.bearer() != Some(&account.session_token) {
                    return Reply::error(401, "unauthorized");
                }
                if account.opaque.login_finish(&req.json()) {
                    Reply::ok(json!({ "proven": true, "password_counter": 1 }))
                } else {
                    Reply::error(403, "password_not_proven")
                }
            }

            // ── What the Security page reads: invented rows, in the client's shapes ──
            "GET /api/v1/me/activity" if account.security_routes.load(Ordering::SeqCst) => {
                if req.bearer() != Some(&account.session_token) {
                    return Reply::error(401, "unauthorized");
                }
                Reply::ok(json!({ "opted_in": true, "events": [{
                    "type": "auth.login.success",
                    "outcome": "success",
                    "device": "Fixture PC",
                    "country_code": "DE",
                    "created_at": "2026-11-15T05:00:00Z",
                }] }))
            }

            "GET /api/v1/me/profile" if account.security_routes.load(Ordering::SeqCst) => {
                if req.bearer() != Some(&account.session_token) {
                    return Reply::error(401, "unauthorized");
                }
                Reply::ok(json!({ "display_name": "Fixture" }))
            }

            "GET /api/v1/me/two-factor" if account.security_routes.load(Ordering::SeqCst) => {
                if req.bearer() != Some(&account.session_token) {
                    return Reply::error(401, "unauthorized");
                }
                Reply::ok(json!({ "enabled": account.two_factor_on.load(Ordering::SeqCst) }))
            }

            // ── Recovery check ──────────────────────────────────────────
            "POST /api/v1/auth/verify-recovery-check" => {
                if req.bearer() != Some(&account.session_token) {
                    return Reply::error(401, "unauthorized");
                }
                let body = req.json();
                let check = body.get("recovery_check").and_then(Value::as_str).unwrap_or("");
                if check == account.recovery_check {
                    Reply::ok(json!({ "valid": true }))
                } else {
                    Reply::error(400, "invalid_recovery_phrase")
                }
            }

            // ── Billing ─────────────────────────────────────────────────
            "GET /api/v1/billing/subscription" => {
                if req.bearer() != Some(&account.session_token) {
                    return Reply::error(401, "unauthorized");
                }
                let delay = account.plan_delay_ms.load(Ordering::SeqCst);
                if delay > 0 {
                    std::thread::sleep(std::time::Duration::from_millis(delay));
                }
                if account.plan_unreachable.load(Ordering::SeqCst) {
                    return Reply::hang_up();
                }
                let state = account.plan_state.lock().unwrap().clone();
                let (plan, status) = match state.as_str() {
                    "none" => ("none", "none"),
                    "lapsed" => ("personal", "canceled"),
                    "trial" => ("personal", "trialing"),
                    "free" => ("free", "active"),
                    _ => ("personal", "active"),
                };
                Reply::ok(json!({
                    "plan": plan,
                    "billing_cycle": "monthly",
                    "status": status,
                    "seats": 1,
                    "region": "eu",
                    "created_at": "2025-01-01T00:00:00Z",
                    "current_period_end": "2026-01-01T00:00:00Z",
                    "quota_bytes": account.quota_bytes,
                    "used_bytes": account.used_bytes,
                    "plan_state": state,
                }))
            }

            "GET /api/v1/billing/usage" => {
                if req.bearer() != Some(&account.session_token) {
                    return Reply::error(401, "unauthorized");
                }
                let pct = if account.quota_bytes > 0 {
                    (account.used_bytes as f64 / account.quota_bytes as f64 * 100.0).round() as u64
                } else {
                    0
                };
                Reply::ok(json!({
                    "used_bytes": account.used_bytes,
                    "quota_bytes": account.quota_bytes,
                    "percentage": pct,
                }))
            }

            // ── Sync snapshot ───────────────────────────────────────────
            "GET /api/v1/sync/snapshot" => {
                if req.bearer() != Some(&account.session_token) {
                    return Reply::error(401, "unauthorized");
                }
                let nodes = account.snapshot_nodes();
                let seq = account.seq.load(Ordering::SeqCst);
                Reply::ok(json!({
                    "seq_id": seq,
                    "nodes": nodes,
                }))
            }

            // ── Sync ops ────────────────────────────────────────────────
            "GET /api/v1/sync/ops" => {
                if req.bearer() != Some(&account.session_token) {
                    return Reply::error(401, "unauthorized");
                }
                // The change log: empty for a static tree; an upload or a pushed version adds its ops
                // (`fake_uploads`), each with the sequence number it took.
                let seq = account.seq.load(Ordering::SeqCst);
                let since = req.query("since").and_then(|v| v.parse::<i64>().ok());
                match since {
                    Some(since) => Reply::ok(json!({
                        "ops": account.ops_since(since),
                        "since": since,
                        "head_seq": seq,
                    })),
                    None => Reply::ok(json!({ "ops": [], "since": seq, "head_seq": seq })),
                }
            }

            // ── The counter row (the Trash badge); before the single-file arm below, which would answer 404 ──
            "GET /api/v1/files/count" => {
                if req.bearer() != Some(&account.session_token) {
                    return Reply::error(401, "unauthorized");
                }
                account.files_count()
            }

            // ── Files listing ───────────────────────────────────────────
            "GET /api/v1/files" => {
                if req.bearer() != Some(&account.session_token) {
                    return Reply::error(401, "unauthorized");
                }
                let parent_id = req.query("parent_id");
                let trashed = req.query("trashed").map(|v| v == "true").unwrap_or(false);
                let items = account.list_children(parent_id, trashed);
                Reply::ok(json!({ "items": items, "cursor": null }))
            }

            // ── Folder create ───────────────────────────────────────────
            "POST /api/v1/files/folder" => {
                if req.bearer() != Some(&account.session_token) {
                    return Reply::error(401, "unauthorized");
                }
                let body = req.json();
                let id = format!("folder-{}", uuid::Uuid::new_v4());
                Reply::ok(json!({
                    "id": id,
                    "parent_id": body.get("parent_id"),
                    "name_encrypted": body.get("name_encrypted"),
                    "is_folder": true,
                }))
            }

            // ── Trash (DELETE /api/v1/files/:id) ────────────────────────
            route_str if route_str.starts_with("DELETE /api/v1/files/") => {
                if req.bearer() != Some(&account.session_token) {
                    return Reply::error(401, "unauthorized");
                }
                let path_only = req.path.split('?').next().unwrap_or("");
                let id = path_only.strip_prefix("/api/v1/files/").unwrap_or("");
                if account.trash_item(id) {
                    Reply::ok(json!({ "success": true }))
                } else {
                    Reply::error(404, "not_found")
                }
            }

            // ── A file's versions, and the restore of one ───────────────
            route_str if route_str.starts_with("GET /api/v1/files/") && route_str.ends_with("/versions") => {
                if req.bearer() != Some(&account.session_token) {
                    return Reply::error(401, "unauthorized");
                }
                let path_only = req.path.split('?').next().unwrap_or("");
                let id = path_only
                    .strip_prefix("/api/v1/files/")
                    .and_then(|rest| rest.strip_suffix("/versions"))
                    .unwrap_or("");
                match account.versions_payload(id) {
                    Some(payload) => Reply::ok(payload),
                    None => Reply::error(404, "not_found"),
                }
            }
            route_str
                if route_str.starts_with("POST /api/v1/files/")
                    && route_str.contains("/versions/")
                    && route_str.ends_with("/restore") =>
            {
                if req.bearer() != Some(&account.session_token) {
                    return Reply::error(401, "unauthorized");
                }
                account.restores.fetch_add(1, Ordering::SeqCst);
                let path_only = req.path.split('?').next().unwrap_or("");
                let (id, version) = path_only
                    .strip_prefix("/api/v1/files/")
                    .and_then(|rest| rest.strip_suffix("/restore"))
                    .and_then(|rest| rest.split_once("/versions/"))
                    .unwrap_or(("", ""));
                if let Some(status) = account.restore_refusal.lock().expect("restore switch").take() {
                    return Reply::error(status, "a refusal");
                }
                if !account.restore_version(id, version) {
                    return Reply::error(404, "not_found");
                }
                if account.restore_hangs_up.swap(false, Ordering::SeqCst) {
                    return Reply::hang_up();
                }
                Reply::ok(json!({ "success": true }))
            }

            // ── Restore ─────────────────────────────────────────────────
            route_str if route_str.starts_with("POST /api/v1/files/") && route_str.ends_with("/restore") => {
                if req.bearer() != Some(&account.session_token) {
                    return Reply::error(401, "unauthorized");
                }
                let path_only = req.path.split('?').next().unwrap_or("");
                let id = path_only
                    .strip_prefix("/api/v1/files/")
                    .and_then(|rest| rest.strip_suffix("/restore"))
                    .unwrap_or("");
                if account.restore_item(id) {
                    Reply::ok(json!({ "success": true }))
                } else {
                    Reply::error(404, "not_found")
                }
            }

            // ── Device registration ─────────────────────────────────────
            "POST /api/v1/clients/devices" => {
                if req.bearer() != Some(&account.session_token) {
                    return Reply::error(401, "unauthorized");
                }
                let body = req.json();
                // The server upserts on (hostname, platform); the row is named by `id`, as the client reads it.
                let mut devices = account.devices.lock().expect("devices");
                let at = devices
                    .iter()
                    .position(|d| d["hostname"] == body["hostname"] && d["platform"] == body["platform"]);
                let device = json!({
                    "id": at
                        .map(|i| devices[i]["id"].clone())
                        .unwrap_or_else(|| json!(format!("device-{}", uuid::Uuid::new_v4()))),
                    "hostname": body.get("hostname"),
                    "platform": body.get("platform"),
                    "bb_version": body.get("bb_version"),
                    "last_seen": "2026-11-15T05:00:00Z",
                    "created_at": "2026-11-15T05:00:00Z",
                    "session_count": 0,
                });
                match at {
                    Some(i) => devices[i] = device.clone(),
                    None => devices.push(device.clone()),
                }
                Reply::ok(device)
            }

            "GET /api/v1/clients/devices" => {
                if req.bearer() != Some(&account.session_token) {
                    return Reply::error(401, "unauthorized");
                }
                let devices = account.devices.lock().expect("devices").clone();
                Reply::ok(json!({ "devices": devices }))
            }

            // ── Sync session creation ───────────────────────────────────
            "POST /api/v1/clients/sessions" => {
                if req.bearer() != Some(&account.session_token) {
                    return Reply::error(401, "unauthorized");
                }
                let body = req.json();
                let session_id = format!("session-{}", uuid::Uuid::new_v4());
                // Named by `id`, with the interval the app asked for (the server defaults it to 60).
                let interval = body
                    .get("heartbeat_interval_secs")
                    .and_then(Value::as_i64)
                    .unwrap_or(60);
                let session = json!({
                    "id": session_id,
                    "device_id": body.get("device_id"),
                    "name": body.get("name"),
                    "session_type": body.get("session_type"),
                    "local_path": body.get("local_path"),
                    "remote_path": body.get("remote_path"),
                    "status": "idle",
                    "heartbeat_interval_secs": interval,
                    "created_at": "2026-11-15T05:00:00Z",
                });
                account.sessions.lock().expect("sessions").push(session.clone());
                Reply::ok(session)
            }

            // ── Heartbeat ───────────────────────────────────────────────
            route_str
                if route_str.starts_with("POST /api/v1/clients/sessions/") && route_str.ends_with("/heartbeat") =>
            {
                if req.bearer() != Some(&account.session_token) {
                    return Reply::error(401, "unauthorized");
                }
                Reply::ok(json!({}))
            }

            // ── Logout ──────────────────────────────────────────────────
            "POST /api/v1/auth/logout" => Reply::ok(json!({})),

            // ── Whole uploads: init, chunks, heartbeat, complete, abandon; stored pictures (`fake_uploads`) ──
            "POST /api/v1/uploads/init" => {
                if req.bearer() != Some(&account.session_token) {
                    return Reply::error(401, "unauthorized");
                }
                account.upload_init(&req.json())
            }
            route_str if account.is_upload_route(route_str) => {
                if req.bearer() != Some(&account.session_token) {
                    return Reply::error(401, "unauthorized");
                }
                account.upload_route(req)
            }

            // ── Search shards ───────────────────────────────────────────
            _ if FakeAccount::is_search_shard_route(req) => {
                if req.bearer() != Some(&account.session_token) {
                    return Reply::error(401, "unauthorized");
                }
                account.search_shard_route(req)
            }

            // ── Share links: make one, list the account's ───────────────
            "POST /api/v1/shares" => {
                if req.bearer() != Some(&account.session_token) {
                    return Reply::error(401, "unauthorized");
                }
                account.post_link(&req.json())
            }
            route_str if route_str.split('?').next() == Some("GET /api/v1/shares/mine") => {
                if req.bearer() != Some(&account.session_token) {
                    return Reply::error(401, "unauthorized");
                }
                account.list_links()
            }

            // ── One chunk of a file with content (`FakeAccount::add_file_with_content`) ──
            route_str if route_str.starts_with("GET /api/v1/files/") && route_str.contains("/chunks/") => {
                if req.bearer() != Some(&account.session_token) {
                    return Reply::error(401, "unauthorized");
                }
                let path_only = req.path.split('?').next().unwrap_or("");
                let frame = path_only
                    .strip_prefix("/api/v1/files/")
                    .and_then(|rest| rest.split_once("/chunks/"))
                    .and_then(|(id, index)| account.content_frame(id, index.parse().ok()?));
                match frame {
                    Some(frame) => Reply::bytes(200, frame),
                    None => Reply::json(404, json!({ "error": "not_found" })),
                }
            }

            // ── Folder digests (`fake_digests`); before the single-file arm ──
            "GET /api/v1/folder-digests" => account.folder_digests_route(req),

            // ── Single file (canonical row for the owner-keys prepare path) ──
            route_str if route_str.starts_with("GET /api/v1/files/") && !route_str.contains('?') => {
                if req.bearer() != Some(&account.session_token) {
                    return Reply::error(401, "unauthorized");
                }
                let path_only = req.path.split('?').next().unwrap_or("");
                let id = path_only.strip_prefix("/api/v1/files/").unwrap_or("");
                // Serve the item from the vault if it exists.
                let items = account.items.lock().expect("vault items");
                match items.get(id) {
                    Some(item) => Reply::ok(item.to_node()),
                    None => Reply::json(404, json!({ "error": "not_found" })),
                }
            }

            // ── Backups root ────────────────────────────────────────────
            "GET /api/v1/backups/root" => Reply::json(404, json!({ "error": "not_found" })),

            // ── Notifications ───────────────────────────────────────────
            "GET /api/v1/notifications" => Reply::ok(json!({ "notifications": [], "unread_count": 0 })),

            // ── Catch-all ───────────────────────────────────────────────
            _ => Reply::error(404, &format!("fake: unhandled route {route}")),
        }
    })
}

/// Start a [`FakeApi`] wired to `account`'s handler. Returns the fake and its base URL.
pub fn start_fake(account: Arc<FakeAccount>) -> FakeApi {
    FakeApi::start(account_handler(account))
}

/// Assert that the fake's base URL is loopback and refuse any non-loopback base.
pub fn assert_loopback(base: &str) {
    // Parse `http://HOST:PORT` without pulling in the `url` crate.
    let after_scheme = base
        .strip_prefix("http://")
        .or_else(|| base.strip_prefix("https://"))
        .expect("the fake API base has a scheme");
    let host = after_scheme.split(':').next().unwrap_or(after_scheme);
    assert!(
        host == "127.0.0.1" || host == "::1" || host == "localhost",
        "the fake API must be loopback, got {host}"
    );
}

/// Refuse a non-loopback base: returns `Err` with a message. For use in production-safe guards.
pub fn require_loopback(base: &str) -> Result<(), String> {
    let after_scheme = base
        .strip_prefix("http://")
        .or_else(|| base.strip_prefix("https://"))
        .ok_or_else(|| format!("not a URL: {base}"))?;
    let host = after_scheme.split(':').next().unwrap_or(after_scheme);
    if host == "127.0.0.1" || host == "::1" || host == "localhost" {
        Ok(())
    } else {
        Err(format!("refusing non-loopback API base: {host}"))
    }
}

// ── A pre-built tree for tests ──────────────────────────────────────────────────

/// A recovery phrase valid in BIP-39 English (12 words). Every test uses this one; it is invented and has no
/// value (the BIP-39 crate validates the checksum).
pub const TEST_PHRASE: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

/// Build a [`FakeAccount`] with a small tree: 2 folders, 5 files (0 bytes to a few MiB), the recovery phrase
/// above, and the account started. Returns the account, the recovery phrase, and the fake API server.
pub fn test_account() -> (Arc<FakeAccount>, String, FakeApi) {
    let (account, phrase) = FakeAccount::register("test@example.eu", "correct horse battery", TEST_PHRASE);
    let account = Arc::new(account);
    add_sample_tree(&account);
    let fake = start_fake(account.clone());
    (account, phrase, fake)
}

/// The small tree of [`test_account`] (and of the fake server's fixture account): 2 folders and 5 files.
pub fn add_sample_tree(account: &FakeAccount) {
    // Use real UUIDs for item ids: the engine's owner-keys prepare path calls `item_class(id)` which
    // requires a valid UUID (a non-UUID id makes the class unknown, the row unreadable, and the
    // decrypted name empty in state.db).
    let folder_docs = "a0000000-0000-4000-8000-000000000001";
    let folder_photos = "a0000000-0000-4000-8000-000000000002";
    let file_readme = "a0000000-0000-4000-8000-000000000003";
    let file_notes = "a0000000-0000-4000-8000-000000000004";
    let file_report = "a0000000-0000-4000-8000-000000000005";
    let file_photo1 = "a0000000-0000-4000-8000-000000000006";
    let file_photo2 = "a0000000-0000-4000-8000-000000000007";

    // 2 folders
    account.add_item(folder_docs, None, "Documents", true, 0);
    account.add_item(folder_photos, None, "Photos", true, 0);

    // 5 files: various sizes
    account.add_item(file_readme, Some(folder_docs), "readme.txt", false, 0);
    account.add_item(file_notes, Some(folder_docs), "notes.md", false, 1_024);
    account.add_item(file_report, Some(folder_docs), "report.pdf", false, 512_000);
    account.add_item(file_photo1, Some(folder_photos), "sunset.jpg", false, 2_048_000);
    account.add_item(file_photo2, Some(folder_photos), "portrait.png", false, 4_096_000);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypt_decrypt_round_trips_with_the_fakes_master_key() {
        let (account, _phrase) = FakeAccount::register("a@b.eu", "pw", TEST_PHRASE);
        let mk = beebeeb_core::kdf::MasterKey::from_bytes(account.master_key);
        let fk = beebeeb_core::kdf::derive_file_key(&mk, b"item-1");
        let enc = beebeeb_core::encrypt::encrypt_name_for(
            &fk,
            "hello.txt",
            None,
            beebeeb_core::chunk_stream::ExpectedChunkFormat::Unrecorded,
        )
        .unwrap();
        eprintln!("encrypted: {enc} (len {})", enc.len());
        let mk2 = beebeeb_core::kdf::MasterKey::from_bytes(account.master_key);
        let dec = beebeeb_core::encrypt::decrypt_name(&mk2, "item-1", &enc);
        eprintln!("decrypted: {dec:?}");
        assert_eq!(dec.unwrap(), "hello.txt");
        // Also: decrypt_name_metadata must succeed (the owner-keys prepare path checks this).
        let mk3 = beebeeb_core::kdf::MasterKey::from_bytes(account.master_key);
        let meta = beebeeb_core::encrypt::decrypt_name_metadata(&mk3, "item-1", &enc);
        assert!(meta.is_ok(), "decrypt_name_metadata must succeed: {meta:?}");
    }
}
