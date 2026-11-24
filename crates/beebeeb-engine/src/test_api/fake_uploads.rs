//! Whole uploads and newer versions for the fake API (wave 2b, slice S7): what a native check needs to build a file
//! on the "server" through the engine's own upload client, and to make another device save a newer version.
//!
//! Serves, for the fixture account: `POST /uploads/init` (a NEW file, or a NEW VERSION of one the account holds),
//! `PUT /uploads/{session}/chunks/{n}`, `POST /uploads/{session}/heartbeat`, `POST /uploads/{session}/complete`,
//! `POST /files/{id}/upload/abandon`, the picture routes (`PUT /files/{id}/thumbnail[/{variant}]`, `GET
//! /files/{id}/thumbnail/{variant}`) and the change log (`GET /sync/ops?since=`, [`FakeAccount::ops_since`]).
//! The fake keeps the ciphertext it is sent, frame by frame, and the name it is given at `complete`: nothing is
//! re-sealed, so what a client reads back is what it wrote. [`FakeAccount::push_newer_version`] is the other
//! device's save: it seals with the account's own keys through core's real functions (the same ones
//! [`FakeAccount::add_file_with_content`] uses), with the commitment counter one above the file's current name.
//!
//! Must not: be reachable from a product build (this whole module is `test-support` code); invent a weaker sealing.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Mutex;
use std::sync::atomic::Ordering;

use serde_json::{Value, json};

use super::fake_account::{CONTENT_CHUNK_BYTES, FakeAccount, FakeVersion};
use super::{Reply, Request};

/// The 5 MiB chunk the fake offers an upload that does not name its own size (what `init` answered before).
const DEFAULT_UPLOAD_CHUNK_BYTES: u64 = 5_242_880;

/// One open upload session.
struct Session {
    file_id: String,
    parent_id: Option<String>,
    /// The sealed name `init` carried: a new file's row name until `complete` publishes the one with the commitment.
    name: String,
    size: u64,
    chunk_count: u64,
    object_version_id: String,
    chunks: BTreeMap<u32, Vec<u8>>,
}

/// What the account keeps for uploads, the change log and pictures (one field of [`FakeAccount`]).
#[derive(Default)]
pub(super) struct UploadState {
    sessions: Mutex<HashMap<String, Session>>,
    completed: Mutex<HashSet<String>>,
    /// `(seq_id, op_type, payload)`, in the order they were made.
    ops: Mutex<Vec<(i64, &'static str, Value)>>,
    /// Stored pictures by `(file id, variant)`.
    pictures: Mutex<HashMap<(String, String), Vec<u8>>>,
}

/// What [`FakeAccount::push_newer_version`] made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewerVersion {
    pub file_id: String,
    pub version_number: i32,
    pub size_bytes: u64,
    pub updated_at: i64,
    pub object_version_id: String,
}

impl FakeAccount {
    fn name_key(&self, id: &str) -> beebeeb_core::kdf::FileKey {
        let master = beebeeb_core::kdf::MasterKey::from_bytes(self.master_key);
        beebeeb_core::kdf::derive_file_key(&master, id.as_bytes())
    }

    /// The ops after `since`, as `GET /sync/ops` lists them.
    pub(super) fn ops_since(&self, since: i64) -> Vec<Value> {
        self.uploads
            .ops
            .lock()
            .expect("ops")
            .iter()
            .filter(|(seq, ..)| *seq > since)
            .map(|(seq, op_type, payload)| json!({ "seq_id": seq, "op_type": op_type, "payload": payload }))
            .collect()
    }

    /// The chunks of file `id` exactly as the server holds them. `None`: no such file or no content.
    pub fn stored_frames(&self, id: &str) -> Option<Vec<Vec<u8>>> {
        self.items.lock().expect("vault items").get(id)?.content_frames.clone()
    }

    /// The current sealed name of `id` as the server holds it.
    pub fn stored_name(&self, id: &str) -> Option<String> {
        Some(self.items.lock().expect("vault items").get(id)?.name_encrypted.clone())
    }

    /// `(version_number, updated_at, size_bytes)` of `id`.
    pub fn item_state(&self, id: &str) -> Option<(i32, i64, u64)> {
        let items = self.items.lock().expect("vault items");
        let item = items.get(id)?;
        Some((item.version_number, item.updated_at, item.size_bytes))
    }

    /// Whether a picture is stored for `id` in any variant.
    pub fn has_picture(&self, id: &str) -> bool {
        self.uploads
            .pictures
            .lock()
            .expect("pictures")
            .keys()
            .any(|(file, _)| file == id)
    }

    /// The id of the live file or folder at `path` (names joined by `/`, from the top level), by the plaintext names
    /// the fake holds. `None`: nothing there, or two items with that name.
    pub fn find_by_path(&self, path: &str) -> Option<String> {
        let items = self.items.lock().expect("vault items");
        let mut parent: Option<String> = None;
        for name in path.split('/').filter(|part| !part.is_empty()) {
            let mut hits = items
                .values()
                .filter(|item| !item.is_trashed && item.parent_id == parent && item.name_plaintext == name);
            let hit = hits.next()?;
            if hits.next().is_some() {
                return None;
            }
            parent = Some(hit.id.clone());
        }
        parent
    }

    /// Install `frames` as the current content of `id` (a new version, or the first of a new file). Returns the
    /// version number and object version id now current. The caller holds nothing: this takes the locks itself.
    #[allow(clippy::too_many_arguments)]
    fn install_content(
        &self,
        id: &str,
        parent_id: Option<&str>,
        name_encrypted: &str,
        size: u64,
        frames: Vec<Vec<u8>>,
        object_version_id: String,
    ) -> (i32, i64) {
        let name_plain = beebeeb_core::encrypt::decrypt_file_metadata_with_key(&self.name_key(id), name_encrypted)
            .map(|opened| opened.name)
            .unwrap_or_default();
        let mut items = self.items.lock().expect("vault items");
        // The sequence numbers of this change: the item's `updated_at` and the ops share them.
        let seq = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
        let created = !items.contains_key(id);
        let item = items
            .entry(id.to_string())
            .or_insert_with(|| super::fake_account::VaultItem {
                id: id.to_string(),
                parent_id: parent_id.map(str::to_string),
                is_folder: false,
                name_encrypted: String::new(),
                name_plaintext: String::new(),
                size_bytes: 0,
                updated_at: 0,
                is_trashed: false,
                version_number: 0,
                content_hash: None,
                content_frames: None,
                object_version_id: None,
            });
        item.version_number += 1;
        item.name_encrypted = name_encrypted.to_string();
        item.name_plaintext = name_plain;
        item.size_bytes = size;
        item.updated_at = 1_700_000_000 + seq;
        item.content_hash = Some(format!("hash-{id}-v{}", item.version_number));
        item.content_frames = Some(frames);
        item.object_version_id = Some(object_version_id.clone());
        let (version, updated_at, parent) = (item.version_number, item.updated_at, item.parent_id.clone());
        drop(items);

        {
            let mut all = self.versions.lock().expect("versions");
            let versions = all.entry(id.to_string()).or_default();
            versions.list.push(FakeVersion {
                id: object_version_id.clone(),
                number: i64::from(version),
                created_at: format!("2026-11-15T12:{:02}:00Z", version.clamp(0, 59)),
                size_bytes: size,
            });
            versions.current = i64::from(version);
        }
        // The ops a real server emits for this: a create, or a rename and an update together.
        let mut ops = self.uploads.ops.lock().expect("ops");
        if created {
            ops.push((
                seq,
                "file_create",
                json!({
                    "id": id,
                    "parent_id": parent,
                    "name_encrypted": name_encrypted,
                    "size_bytes": size,
                    "version_number": version,
                    "current_object_version_id": object_version_id,
                }),
            ));
        } else {
            ops.push((
                seq,
                "file_rename",
                json!({ "id": id, "parent_id": parent, "name_encrypted": name_encrypted }),
            ));
            let update_seq = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
            ops.push((
                update_seq,
                "file_update",
                json!({
                    "id": id,
                    "parent_id": parent,
                    "name_encrypted": name_encrypted,
                    "size_bytes": size,
                    "version_number": version,
                    "current_object_version_id": object_version_id,
                }),
            ));
        }
        (version, updated_at)
    }

    /// Another device saved a newer version of the file `id` with `content`: sealed with this account's key for the
    /// file by core's chunk encryptor, the name re-sealed with the commitment counter one above the current name's;
    /// the version number goes up by one, `updated_at` moves, the stored pictures are cleared, a version row is
    /// added and the change log gets the ops a server emits, so the next `sync/ops` tells a client.
    pub fn push_newer_version(&self, id: &str, content: &[u8]) -> Result<NewerVersion, String> {
        let (name_encrypted, parent) = {
            let items = self.items.lock().expect("vault items");
            let item = items.get(id).ok_or("no such file")?;
            if item.is_folder || item.is_trashed {
                return Err("not a live file".into());
            }
            (item.name_encrypted.clone(), item.parent_id.clone())
        };
        let key = self.name_key(id);
        let current = beebeeb_core::encrypt::decrypt_file_metadata_with_key(&key, &name_encrypted)
            .map_err(|e| format!("the current name does not open: {e}"))?;
        let mut encryptor = beebeeb_core::chunk_stream::ChunkEncryptor::for_push_with_file_key(
            self.name_key(id),
            id,
            content.len() as u64,
            CONTENT_CHUNK_BYTES.into(),
        )
        .map_err(|e| e.to_string())?
        .with_recorded_pin(current.chunk_format)
        .map_err(|e| e.to_string())?;
        let pieces: Vec<&[u8]> = if content.is_empty() {
            vec![content]
        } else {
            content.chunks(CONTENT_CHUNK_BYTES as usize).collect()
        };
        let mut frames = Vec::with_capacity(pieces.len());
        for piece in pieces {
            frames.push(encryptor.push_chunk(piece).map_err(|e| e.to_string())?.data);
        }
        let commitment = encryptor
            .version_commitment_after(current.version.map(|version| version.counter))
            .map_err(|e| e.to_string())?;
        encryptor.finish().map_err(|e| e.to_string())?;
        let sealed = beebeeb_core::encrypt::encrypt_name_for_version(
            &key,
            &current.name,
            current.mime_type.as_deref(),
            current.chunk_format,
            commitment,
        )
        .map_err(|e| e.to_string())?;
        let object_version_id = uuid::Uuid::new_v4().to_string();
        let (version_number, updated_at) = self.install_content(
            id,
            parent.as_deref(),
            &sealed,
            content.len() as u64,
            frames,
            object_version_id.clone(),
        );
        self.uploads
            .pictures
            .lock()
            .expect("pictures")
            .retain(|(file, _), _| file != id);
        Ok(NewerVersion {
            file_id: id.to_string(),
            version_number,
            size_bytes: content.len() as u64,
            updated_at,
            object_version_id,
        })
    }

    /// `POST /api/v1/uploads/init`: a new file (an id the account does not hold) or a new version of a file it does.
    pub(super) fn upload_init(&self, body: &Value) -> Reply {
        let file_id = body
            .get("file_id")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| format!("file-{}", uuid::Uuid::new_v4()));
        if let Some(item) = self.items.lock().expect("vault items").get(&file_id) {
            if item.is_folder {
                return Reply::json(409, json!({ "error": "not a file" }));
            }
            // A replacement names the version it replaces; the server refuses one that is not the current one.
            if let Some(base) = body.get("base_version_number").and_then(Value::as_i64)
                && base != i64::from(item.version_number)
            {
                return Reply::json(409, json!({ "error": crate::api_client::STALE_BASE_MESSAGE }));
            }
        }
        let size = body.get("file_size_bytes").and_then(Value::as_u64).unwrap_or(0);
        let chunk_size = body
            .get("chunk_size_bytes")
            .and_then(Value::as_u64)
            .filter(|bytes| *bytes > 0)
            .unwrap_or(DEFAULT_UPLOAD_CHUNK_BYTES);
        let chunk_count = body
            .get("chunk_count")
            .and_then(Value::as_u64)
            .filter(|count| *count > 0)
            .unwrap_or_else(|| size.div_ceil(chunk_size).max(1));
        let session_id = format!("upload-{}", uuid::Uuid::new_v4());
        let object_version_id = uuid::Uuid::new_v4().to_string();
        self.uploads.sessions.lock().expect("upload sessions").insert(
            session_id.clone(),
            Session {
                file_id: file_id.clone(),
                parent_id: body.get("parent_id").and_then(Value::as_str).map(str::to_string),
                name: body
                    .get("file_name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                size,
                chunk_count,
                object_version_id: object_version_id.clone(),
                chunks: BTreeMap::new(),
            },
        );
        Reply::ok(json!({
            "file_id": file_id,
            "tenant_id": "tenant-1",
            "object_version_id": object_version_id,
            "upload_session_id": session_id,
            "chunk_size_bytes": chunk_size,
            "chunk_count": chunk_count,
            "storage_format_version": 1,
            "storage_pool_id": "pool-1",
            "region": "europe",
        }))
    }

    /// Whether `route` (`METHOD /path`, no query) is one of the upload-session, abandon or picture routes here.
    pub(super) fn is_upload_route(&self, route: &str) -> bool {
        let Some((method, path)) = route.split_once(' ') else {
            return false;
        };
        let parts: Vec<&str> = path.trim_start_matches('/').split('/').collect();
        matches!(
            (method, parts.as_slice()),
            ("PUT", ["api", "v1", "uploads", _, "chunks", _])
                | ("POST", ["api", "v1", "uploads", _, "complete" | "heartbeat"])
                | ("POST", ["api", "v1", "files", _, "upload", "abandon"])
                | ("PUT", ["api", "v1", "files", _, "thumbnail"])
                | ("PUT" | "GET", ["api", "v1", "files", _, "thumbnail", _])
                | ("PATCH", ["api", "v1", "files", _])
        )
    }

    /// Answer one of the routes [`Self::is_upload_route`] named (the caller checked the token).
    pub(super) fn upload_route(&self, req: &Request) -> Reply {
        let path = req.path.split('?').next().unwrap_or_default();
        let parts: Vec<&str> = path.trim_start_matches('/').split('/').collect();
        match (req.method.as_str(), parts.as_slice()) {
            ("PUT", ["api", "v1", "uploads", session, "chunks", index]) => {
                let Ok(index) = index.parse::<u32>() else {
                    return Reply::error(400, "bad_chunk_index");
                };
                let mut sessions = self.uploads.sessions.lock().expect("upload sessions");
                let Some(open) = sessions.get_mut(*session) else {
                    return Reply::error(404, "not_found");
                };
                if u64::from(index) >= open.chunk_count {
                    return Reply::error(400, "chunk_out_of_range");
                }
                let size = req.body.len();
                let skipped = open.chunks.insert(index, req.body.clone()).is_some();
                Reply::ok(json!({ "index": index, "size": size, "skipped": skipped }))
            }
            ("POST", ["api", "v1", "uploads", session, "heartbeat"]) => {
                if self
                    .uploads
                    .sessions
                    .lock()
                    .expect("upload sessions")
                    .contains_key(*session)
                {
                    Reply::ok(json!({}))
                } else {
                    Reply::error(404, "not_found")
                }
            }
            ("POST", ["api", "v1", "uploads", session, "complete"]) => self.upload_complete(session, &req.json()),
            ("POST", ["api", "v1", "files", id, "upload", "abandon"]) => {
                self.uploads
                    .sessions
                    .lock()
                    .expect("upload sessions")
                    .retain(|_, open| open.file_id != *id);
                Reply::ok(json!({}))
            }
            ("PATCH", ["api", "v1", "files", id]) => self.patch_file(id, &req.json()),
            ("PUT", ["api", "v1", "files", id, "thumbnail"]) => self.put_picture(id, "medium", &req.body),
            ("PUT", ["api", "v1", "files", id, "thumbnail", variant]) => self.put_picture(id, variant, &req.body),
            ("GET", ["api", "v1", "files", id, "thumbnail", variant]) => {
                let key = ((*id).to_string(), (*variant).to_string());
                match self.uploads.pictures.lock().expect("pictures").get(&key) {
                    Some(bytes) => Reply::bytes(200, bytes.clone()),
                    None => Reply::error(404, "not_found"),
                }
            }
            _ => Reply::error(404, "not_found"),
        }
    }

    /// `PATCH /api/v1/files/{id}`: the metadata update the client sends (`ApiClient::update_metadata`): a body with
    /// `name_encrypted` (the sealed name, as is) and/or `parent_id` (a folder id, or `null` for the top level; a body
    /// without the key leaves the parent alone). Answers the row as the snapshot shows it. A file this account does
    /// not hold and has no open upload for (another account's, or none) is `404 not_found`, the same answer for both, as the real API gives so
    /// that a stranger's id is not confirmed; a trashed row is `404` too. A body that names neither field is `400`.
    fn patch_file(&self, id: &str, body: &Value) -> Reply {
        let name = body.get("name_encrypted").and_then(Value::as_str);
        let parent = body.get("parent_id");
        if name.is_none() && parent.is_none() {
            return Reply::error(400, "nothing_to_update");
        }
        let mut items = self.items.lock().expect("vault items");
        if !items.contains_key(id) {
            // A new file whose upload is open: the server made its row at `init`; the fake makes it at `complete`,
            // so the update waits on the session and is applied there.
            let mut sessions = self.uploads.sessions.lock().expect("upload sessions");
            let Some(open) = sessions.values_mut().find(|open| open.file_id == id) else {
                return Reply::error(404, "not_found");
            };
            match parent {
                None => {}
                Some(Value::Null) => open.parent_id = None,
                Some(Value::String(folder))
                    if items.get(folder).is_some_and(|item| item.is_folder && !item.is_trashed) =>
                {
                    open.parent_id = Some(folder.clone());
                }
                Some(Value::String(_)) => return Reply::error(404, "parent_not_found"),
                Some(_) => return Reply::error(400, "bad_parent_id"),
            }
            if let Some(name) = name {
                open.name = name.to_string();
            }
            return Reply::ok(json!({
                "id": id,
                "parent_id": open.parent_id,
                "name_encrypted": open.name,
                "is_folder": false,
                "is_trashed": false,
            }));
        }
        if items.get(id).is_some_and(|item| item.is_trashed) {
            return Reply::error(404, "not_found");
        }
        let new_parent = match parent {
            None => None,
            Some(Value::Null) => Some(None),
            Some(Value::String(folder)) => {
                if !items.get(folder).is_some_and(|item| item.is_folder && !item.is_trashed) {
                    return Reply::error(404, "parent_not_found");
                }
                Some(Some(folder.clone()))
            }
            Some(_) => return Reply::error(400, "bad_parent_id"),
        };
        let seq = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
        let item = items.get_mut(id).expect("the row was just read");
        if let Some(name) = name {
            item.name_encrypted = name.to_string();
            if let Ok(opened) = beebeeb_core::encrypt::decrypt_file_metadata_with_key(&self.name_key(id), name) {
                item.name_plaintext = opened.name;
            }
        }
        if let Some(new_parent) = new_parent {
            item.parent_id = new_parent;
        }
        item.updated_at = 1_700_000_000 + seq;
        let (row, parent_id, name_now) = (item.to_node(), item.parent_id.clone(), item.name_encrypted.clone());
        drop(items);
        self.uploads.ops.lock().expect("ops").push((
            seq,
            "file_rename",
            json!({ "id": id, "parent_id": parent_id, "name_encrypted": name_now }),
        ));
        Reply::ok(row)
    }

    fn put_picture(&self, id: &str, variant: &str, bytes: &[u8]) -> Reply {
        if !self.items.lock().expect("vault items").contains_key(id) {
            return Reply::error(404, "not_found");
        }
        self.uploads
            .pictures
            .lock()
            .expect("pictures")
            .insert((id.to_string(), variant.to_string()), bytes.to_vec());
        Reply::ok(json!({}))
    }

    /// `POST /api/v1/uploads/{session}/complete`: every chunk must be there; the file takes them as they came and the
    /// name `body` carries (a new file's `init` name when it carries none).
    fn upload_complete(&self, session: &str, body: &Value) -> Reply {
        let open = {
            let mut sessions = self.uploads.sessions.lock().expect("upload sessions");
            match sessions.get(session) {
                None => {
                    // A repeated `complete` after a lost answer: the server says it already did.
                    return if self.uploads.completed.lock().expect("completed").contains(session) {
                        Reply::ok(json!({ "already_completed": true }))
                    } else {
                        Reply::error(404, "not_found")
                    };
                }
                Some(open) if open.chunks.len() as u64 != open.chunk_count => {
                    return Reply::json(409, json!({ "error": "chunks_missing" }));
                }
                Some(_) => sessions.remove(session).expect("the session was just read"),
            }
        };
        let current_name = self.stored_name(&open.file_id);
        let name = body
            .get("name_encrypted")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or(current_name)
            .unwrap_or_else(|| open.name.clone());
        let frames: Vec<Vec<u8>> = open.chunks.into_values().collect();
        let (version, _) = self.install_content(
            &open.file_id,
            open.parent_id.as_deref(),
            &name,
            open.size,
            frames,
            open.object_version_id.clone(),
        );
        self.uploads
            .completed
            .lock()
            .expect("completed")
            .insert(session.to_string());
        Reply::ok(json!({
            "file_id": open.file_id,
            "version_number": version,
            "current_object_version_id": open.object_version_id,
            "size_bytes": open.size,
            "name_encrypted": name,
        }))
    }
}
