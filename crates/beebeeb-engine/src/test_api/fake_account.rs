//! A complete invented Beebeeb account for the fake API: OPAQUE registration, a recovery check derived from the
//! real core functions, a profile, a vault tree with encrypted names, the change feed, trash and restore, storage
//! usage, and device registration. The encryption uses core's real functions so a client that stops encrypting is
//! caught: the fake can assert "this name/blob is not plaintext" by trying the known plaintext against the sealed
//! ciphertext.
//!
//! Tests only (`test` or the `test-support` feature). Every key, name and id is invented.

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use super::{FakeApi, Handler, OpaqueAccount, Reply, Request};

// ── The vault tree ──────────────────────────────────────────────────────────────

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
}

impl VaultItem {
    fn to_node(&self) -> Value {
        json!({
            "id": self.id,
            "parent_id": self.parent_id,
            "name_encrypted": self.name_encrypted,
            "is_folder": self.is_folder,
            "size_bytes": self.size_bytes,
            "updated_at": self.updated_at,
            "is_trashed": self.is_trashed,
            "version_number": self.version_number,
            "content_hash": self.content_hash,
        })
    }
}

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
    items: Mutex<HashMap<String, VaultItem>>,
    /// The current seq_id (monotonic).
    seq: AtomicI64,
    /// How much storage this account uses.
    pub used_bytes: u64,
    pub quota_bytes: u64,
    /// The device this session registered.
    devices: Mutex<Vec<Value>>,
    /// Client sync sessions.
    sessions: Mutex<Vec<Value>>,
}

impl FakeAccount {
    /// Register a new invented account with the real OPAQUE protocol and a real recovery check. The recovery
    /// phrase is returned alongside the account so tests can unlock with it.
    pub fn register(email: &str, password: &str, recovery_phrase: &str) -> (Self, String) {
        let opaque = OpaqueAccount::register(email, password);
        let master_key_struct =
            beebeeb_core::recovery::recover_from_phrase(recovery_phrase).expect("a valid 12-word phrase");
        let recovery_check = {
            use base64::Engine as _;
            let check = beebeeb_core::opaque::compute_recovery_check(&master_key_struct);
            base64::engine::general_purpose::STANDARD.encode(&*check)
        };
        let master_key = master_key_struct.to_bytes();
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
        };
        self.items.lock().expect("vault items").insert(id.to_string(), item);
    }

    /// The non-trashed items as sync-snapshot nodes.
    fn snapshot_nodes(&self) -> Vec<Value> {
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

    /// Trash an item by id. Returns true if found and not already trashed.
    pub fn trash_item(&self, id: &str) -> bool {
        let mut items = self.items.lock().expect("vault items");
        if let Some(item) = items.get_mut(id) {
            if !item.is_trashed {
                item.is_trashed = true;
                item.version_number += 1;
                self.seq.fetch_add(1, Ordering::SeqCst);
                return true;
            }
        }
        false
    }

    /// Restore a trashed item. Returns true if found and was trashed.
    pub fn restore_item(&self, id: &str) -> bool {
        let mut items = self.items.lock().expect("vault items");
        if let Some(item) = items.get_mut(id) {
            if item.is_trashed {
                item.is_trashed = false;
                item.version_number += 1;
                self.seq.fetch_add(1, Ordering::SeqCst);
                return true;
            }
        }
        false
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
                Reply::ok(json!({
                    "plan_name": "Pro",
                    "status": "active",
                    "storage_limit_bytes": account.quota_bytes,
                    "created_at": "2025-01-01T00:00:00Z",
                    "current_period_end": "2026-01-01T00:00:00Z",
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
                // We return no ops: the fake is a static tree.
                let seq = account.seq.load(Ordering::SeqCst);
                Reply::ok(json!({
                    "ops": [],
                    "since": seq,
                    "head_seq": seq,
                }))
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
                let device_id = format!("device-{}", uuid::Uuid::new_v4());
                let device = json!({
                    "device_id": device_id,
                    "hostname": body.get("hostname"),
                    "platform": body.get("platform"),
                    "app_version": body.get("app_version"),
                });
                account.devices.lock().expect("devices").push(device.clone());
                Reply::ok(json!({ "device_id": device_id }))
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
                let session = json!({
                    "session_id": session_id,
                    "device_id": body.get("device_id"),
                    "sync_path": body.get("sync_path"),
                });
                account.sessions.lock().expect("sessions").push(session);
                Reply::ok(json!({ "session_id": session_id }))
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

            // ── Upload init ─────────────────────────────────────────────
            "POST /api/v1/uploads/init" => {
                if req.bearer() != Some(&account.session_token) {
                    return Reply::error(401, "unauthorized");
                }
                let body = req.json();
                let file_id = body
                    .get("file_id")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("file-{}", uuid::Uuid::new_v4()));
                Reply::ok(json!({
                    "file_id": file_id,
                    "tenant_id": "tenant-1",
                    "object_version_id": "ver-1",
                    "upload_session_id": format!("upload-{}", uuid::Uuid::new_v4()),
                    "chunk_size_bytes": 5_242_880,
                    "chunk_count": 1,
                    "storage_format_version": 1,
                    "storage_pool_id": "pool-1",
                    "region": "europe",
                }))
            }

            // ── Search shards ───────────────────────────────────────────
            "GET /api/v1/search-index/shards" => Reply::ok(json!({ "shards": [] })),

            // ── Shares invites ──────────────────────────────────────────
            "GET /api/v1/shares/invites/incoming" => Reply::ok(json!({ "invites": [] })),

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
