//! Reusable test-only API for 1612/1615 and native Mac/Linux regression harnesses.
//! No production credentials, external hosts, global state or native registration.
use anyhow::{Context, Result, ensure};
use base64::Engine;
use beebeeb_core::{
    encrypt::{decrypt_chunk_raw, encrypt_chunk, encrypt_name},
    kdf::{FileKey, MasterKey, derive_file_key},
    opaque,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

pub fn id(n: usize) -> String {
    format!("16120000-0000-4000-8000-{n:012}")
}
pub fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[derive(Clone, Deserialize, Serialize)]
pub struct Blob {
    pub number: i64,
    pub source: String,
    pub size: usize,
    pub sha256: String,
    pub variant: Option<String>,
}
#[derive(Clone, Deserialize, Serialize)]
pub struct Object {
    pub id: String,
    pub owner: String,
    pub path: String,
    pub parent_id: Option<String>,
    pub kind: String,
    pub mime_type: String,
    pub versions: Vec<Blob>,
    pub thumbnails: Vec<Blob>,
}
#[derive(Clone, Deserialize, Serialize)]
pub struct Scenario {
    pub id: String,
    pub object_ids: Vec<String>,
}
#[derive(Clone, Deserialize, Serialize)]
pub struct Corpus {
    pub schema_version: usize,
    pub accounts: Vec<String>,
    pub chunk_size: usize,
    pub scenarios: Vec<Scenario>,
    pub objects: Vec<Object>,
}
impl Corpus {
    pub fn root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/native-parity")
    }
    pub fn load() -> Result<Self> {
        let corpus: Self = serde_json::from_str(include_str!("../../../fixtures/native-parity/manifest.json"))?;
        corpus.validate()?;
        Ok(corpus)
    }
    pub fn bytes(blob: &Blob) -> Result<Vec<u8>> {
        ensure!(
            blob.source.starts_with("plaintext/") && !blob.source.contains(".."),
            "unsafe fixture source"
        );
        let bytes = std::fs::read(Self::root().join(&blob.source))?;
        let actual_hash = hash(&bytes);
        ensure!(
            bytes.len() == blob.size,
            "fixture size mismatch: {}; expected {} bytes, SHA-256 {}; actual {} bytes, SHA-256 {}",
            blob.source, blob.size, blob.sha256, bytes.len(), actual_hash
        );
        ensure!(actual_hash == blob.sha256, "fixture SHA-256 mismatch: {}", blob.source);
        Ok(bytes)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.schema_version == 1 && self.chunk_size == 8,
            "fixture schema/chunk size"
        );
        ensure!(self.accounts == ["alice", "bob"], "fixture accounts");
        ensure!(self.objects.len() == 14, "fixture object count");
        let mut scenarios: Vec<_> = self.scenarios.iter().map(|s| s.id.as_str()).collect();
        scenarios.sort();
        ensure!(
            scenarios
                == [
                    "edit-conflict",
                    "expired-session",
                    "image-thumbnail",
                    "interrupted-upload",
                    "nested-tree",
                    "revoked-share",
                    "subtree-trash",
                    "two-accounts",
                    "video-poster"
                ],
            "fixture scenario set"
        );
        let mut hashes = 0;
        for (i, object) in self.objects.iter().enumerate() {
            ensure!(object.id == id(i + 1), "fixture identity/order");
            ensure!(self.accounts.contains(&object.owner), "fixture owner");
            if let Some(parent) = &object.parent_id {
                ensure!(
                    self.objects.iter().any(|o| &o.id == parent
                        && o.kind == "folder"
                        && o.owner == object.owner
                        && object.path.starts_with(&format!("{}/", o.path))),
                    "fixture parent"
                );
            }
            for blob in object.versions.iter().chain(&object.thumbnails) {
                Self::bytes(blob)?;
                hashes += 1;
            }
        }
        ensure!(hashes == 16, "fixture hash count");
        for scenario in &self.scenarios {
            ensure!(
                !scenario.object_ids.is_empty()
                    && scenario
                        .object_ids
                        .iter()
                        .all(|id| self.objects.iter().any(|o| &o.id == id)),
                "fixture scenario objects"
            );
        }
        Ok(())
    }
}

// Deliberately neither Debug nor Serialize. Values travel only in process memory.
struct Account {
    key: Zeroizing<[u8; 32]>,
    token: Zeroizing<String>,
    expired: bool,
}
struct Stored {
    object: Object,
    active: usize,
    trashed: bool,
    uploading: bool,
}
#[derive(Default, Clone, Debug, Serialize)]
pub struct Counts {
    pub requests: usize,
    pub chunks_read: usize,
    pub thumbnails_read: usize,
    pub faults: usize,
    pub upload_inits: usize,
    pub chunk_writes: usize,
    pub commits: usize,
}
struct State {
    objects: Vec<Stored>,
    accounts: BTreeMap<String, Account>,
    ops: Vec<Value>,
    share_active: bool,
    upload: Option<BTreeMap<usize, Vec<u8>>>,
    upload_complete: bool,
    counts: Counts,
}
#[derive(Debug)]
pub struct Response {
    pub status: u16,
    pub body: Vec<u8>,
}
impl Response {
    fn json(status: u16, body: Value) -> Self {
        Self {
            status,
            body: serde_json::to_vec(&body).unwrap(),
        }
    }
    pub fn value(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap()
    }
}
fn wire(key: &FileKey, bytes: &[u8]) -> Vec<u8> {
    let blob = encrypt_chunk(key, bytes).unwrap();
    [blob.nonce, blob.ciphertext].concat()
}
impl State {
    fn key(&self, object: &Object) -> FileKey {
        derive_file_key(
            &MasterKey::from_bytes(*self.accounts[&object.owner].key),
            object.id.as_bytes(),
        )
    }
    fn row(&self, stored: &Stored) -> Value {
        let o = &stored.object;
        let size = o.versions.get(stored.active).map(|v| v.size).unwrap_or(0);
        let name = encrypt_name(
            &MasterKey::from_bytes(*self.accounts[&o.owner].key),
            &o.id,
            o.path.rsplit('/').next().unwrap(),
            Some(&o.mime_type),
        )
        .unwrap();
        json!({"id":o.id, "parent_id":o.parent_id, "name_encrypted":name, "is_folder":o.kind=="folder", "size_bytes":size, "mime_type":o.mime_type, "version_number":o.versions.get(stored.active).map(|v|v.number).unwrap_or(1), "current_object_version_id":format!("{}-v{}",o.id,stored.active+1), "chunk_count":if o.kind=="folder" {0} else {size.div_ceil(8).max(1)}, "chunk_size_bytes":8, "is_uploading":stored.uploading, "is_trashed":stored.trashed, "updated_at":"2026-09-29T12:00:00Z"})
    }
    fn invite(&self) -> Value {
        let owner = MasterKey::from_bytes(*self.accounts["alice"].key);
        let recipient = MasterKey::from_bytes(*self.accounts["bob"].key);
        let private = opaque::derive_x25519_private(&owner);
        let public = opaque::derive_x25519_public(&private);
        let recipient_public = opaque::derive_x25519_public(&opaque::derive_x25519_private(&recipient));
        let secret = opaque::x25519_shared_secret(&private, &recipient_public).unwrap();
        let share_key = opaque::derive_share_key(&secret, id(13).as_bytes());
        let stored = &self.objects[12];
        let b64 = base64::engine::general_purpose::STANDARD;
        json!({"id":"fixture-share", "file_id":id(13), "status":"approved", "is_folder_share":false, "sender_email":"alice@native-parity.invalid", "sender_public_key":b64.encode(public), "encrypted_file_key":b64.encode(wire(&FileKey::from_bytes(*share_key), self.key(&stored.object).as_bytes())), "file_name_encrypted":self.row(stored)["name_encrypted"], "size_bytes":stored.object.versions[0].size, "mime_type":"text/plain"})
    }
    fn dispatch(&mut self, method: &str, target: &str, bearer: &str, body: &[u8]) -> Result<Response> {
        self.counts.requests += 1;
        let Some((name, account)) = self.accounts.iter().find(|(_, a)| a.token.as_str() == bearer) else {
            return Ok(Response::json(401, json!({"error":"fixture unauthorized"})));
        };
        if account.expired {
            return Ok(Response::json(401, json!({"error":"fixture session expired"})));
        }
        let name = name.clone();
        let (path, query) = target.split_once('?').unwrap_or((target, ""));
        if method == "GET" && path == "/api/v1/sync/snapshot" {
            return Ok(Response::json(
                200,
                json!({"seq_id":self.ops.len(), "nodes":self.objects.iter().filter(|s|s.object.owner==name && !s.trashed && !s.uploading).map(|s|self.row(s)).collect::<Vec<_>>()}),
            ));
        }
        if method == "GET" && path == "/api/v1/sync/ops" {
            let since: usize = query.strip_prefix("since=").unwrap_or("0").parse()?;
            return Ok(Response::json(
                200,
                json!({"since":since,"ops":if name=="alice" { self.ops.iter().skip(since).cloned().collect::<Vec<_>>() } else { vec![] }}),
            ));
        }
        if method == "GET" && path == "/api/v1/shares/invites/incoming" {
            return Ok(Response::json(
                200,
                json!({"invites":if name=="bob" && self.share_active {vec![self.invite()]} else {vec![]}}),
            ));
        }
        if let Some(rest) = path.strip_prefix("/api/v1/files/") {
            let parts: Vec<_> = rest.split('/').collect();
            let Some(stored) = self.objects.iter().find(|s| s.object.id == parts[0]) else {
                return Ok(Response::json(404, json!({"error":"unknown fixture object"})));
            };
            if stored.object.owner != name && !(name == "bob" && parts[0] == id(13) && self.share_active) {
                return Ok(Response::json(403, json!({"error":"fixture access denied"})));
            }
            if stored.trashed {
                return Ok(Response::json(404, json!({"error":"fixture trashed"})));
            }
            if method == "GET" && parts.len() == 1 {
                return Ok(Response::json(200, self.row(stored)));
            }
            if method == "GET" && parts.len() == 3 && parts[1] == "chunks" {
                if stored.uploading {
                    return Ok(Response::json(409, json!({"error":"upload incomplete"})));
                }
                if parts[0] == id(10) && self.upload_complete {
                    let index: usize = parts[2].parse()?;
                    let Some(encrypted) = self.upload.as_ref().and_then(|chunks| chunks.get(&index)).cloned() else {
                        return Ok(Response::json(404, json!({"error":"missing uploaded chunk"})));
                    };
                    self.counts.chunks_read += 1;
                    return Ok(Response {
                        status: 200,
                        body: encrypted,
                    });
                }
                let bytes = Corpus::bytes(&stored.object.versions[stored.active])?;
                let index: usize = parts[2].parse()?;
                let Some(chunk) = (if bytes.is_empty() && index == 0 {
                    Some(&[][..])
                } else {
                    bytes.chunks(8).nth(index)
                }) else {
                    return Ok(Response::json(404, json!({"error":"chunk index"})));
                };
                let encrypted = wire(&self.key(&stored.object), chunk);
                self.counts.chunks_read += 1;
                return Ok(Response {
                    status: 200,
                    body: encrypted,
                });
            }
            if method == "GET" && parts.len() == 3 && parts[1] == "thumbnail" {
                let Some(blob) = stored
                    .object
                    .thumbnails
                    .iter()
                    .find(|b| b.variant.as_deref() == Some(parts[2]))
                else {
                    return Ok(Response::json(404, json!({"error":"no thumbnail variant"})));
                };
                let encrypted = wire(&self.key(&stored.object), &Corpus::bytes(blob)?);
                self.counts.thumbnails_read += 1;
                return Ok(Response {
                    status: 200,
                    body: encrypted,
                });
            }
        }
        if name == "alice" && method == "POST" && path == "/api/v1/uploads/init" {
            let request: Value = serde_json::from_slice(body)?;
            if request["file_id"] == id(7)
                && request["base_version_number"].as_i64()
                    != Some(self.objects[6].object.versions[self.objects[6].active].number)
            {
                return Ok(Response::json(
                    409,
                    json!({"error":"version conflict", "current_version":self.objects[6].object.versions[self.objects[6].active].number}),
                ));
            }
            ensure!(
                request["file_id"] == id(10) && request["file_size_bytes"] == 20,
                "fixture only accepts the declared upload object"
            );
            self.counts.upload_inits += 1;
            self.upload.get_or_insert_with(BTreeMap::new);
            return Ok(Response::json(
                201,
                json!({"file_id":id(10),"tenant_id":"fixture", "object_version_id":"fixture-upload-v1", "upload_session_id":"fixture-upload", "chunk_size_bytes":8,"chunk_count":3,"storage_format_version":2,"storage_pool_id":"fixture","region":"local"}),
            ));
        }
        if name == "alice" && path.starts_with("/api/v1/uploads/fixture-upload/") {
            if self.upload.is_none() {
                return Ok(Response::json(404, json!({"error":"no upload session"})));
            }
            if method == "PUT" {
                if let Some(index) = path.strip_prefix("/api/v1/uploads/fixture-upload/chunks/") {
                    let index: usize = index.parse()?;
                    if index == 1 && self.counts.faults == 0 {
                        self.counts.faults += 1;
                        return Ok(Response::json(
                            503,
                            json!({"error":"fixture interruption before acknowledgement"}),
                        ));
                    }
                    ensure!(index < 3 && !self.upload_complete, "invalid upload chunk/state");
                    let decoded = decrypt_chunk_raw(&self.key(&self.objects[9].object), body)
                        .map_err(|_| anyhow::anyhow!("fixture upload authentication failed"))?;
                    let expected = Corpus::bytes(&self.objects[9].object.versions[0])?;
                    ensure!(
                        decoded == expected.chunks(8).nth(index).unwrap(),
                        "fixture uploaded plaintext mismatch"
                    );
                    let upload = self.upload.as_mut().unwrap();
                    let skipped = upload.insert(index, body.to_vec()).is_some();
                    if !skipped {
                        self.counts.chunk_writes += 1;
                    }
                    return Ok(Response::json(
                        200,
                        json!({"index":index,"size":body.len(),"skipped":skipped}),
                    ));
                }
            }
            if method == "POST" && path == "/api/v1/uploads/fixture-upload/complete" {
                if self.upload.as_ref().unwrap().len() != 3 {
                    return Ok(Response::json(409, json!({"error":"missing chunks"})));
                }
                if !self.upload_complete {
                    self.counts.commits += 1;
                    self.upload_complete = true;
                    self.objects[9].uploading = false;
                }
                return Ok(Response::json(
                    200,
                    json!({"file_id":id(10),"version_number":1,"current_object_version_id":"fixture-upload-v1","size_bytes":20}),
                ));
            }
        }
        Ok(Response::json(404, json!({"error":"unsupported fixture route"})))
    }
}

pub struct Fixture {
    pub corpus: Corpus,
    pub root: tempfile::TempDir,
    state: Arc<Mutex<State>>,
    server: Option<Server>,
}
#[derive(Debug, Serialize)]
pub struct Cleanup {
    pub removed_objects: usize,
    pub removed_accounts: usize,
    pub removed_uploads: usize,
    pub remaining_objects: usize,
    pub remaining_accounts: usize,
    pub remaining_uploads: usize,
    pub remaining_paths: usize,
    pub remaining_listeners: usize,
}
impl Fixture {
    pub fn setup() -> Result<Self> {
        let corpus = Corpus::load()?;
        let accounts = corpus
            .accounts
            .iter()
            .map(|n| {
                (
                    n.clone(),
                    Account {
                        key: Zeroizing::new(rand::random()),
                        token: Zeroizing::new(uuid::Uuid::new_v4().to_string()),
                        expired: false,
                    },
                )
            })
            .collect();
        let state = State {
            objects: corpus
                .objects
                .iter()
                .map(|o| Stored {
                    object: o.clone(),
                    active: 0,
                    trashed: false,
                    uploading: o.id == id(10),
                })
                .collect(),
            accounts,
            ops: vec![],
            share_active: true,
            upload: None,
            upload_complete: false,
            counts: Counts::default(),
        };
        let root = tempfile::Builder::new().prefix("beebeeb-1612-").tempdir()?;
        Ok(Self {
            corpus,
            root,
            state: Arc::new(Mutex::new(state)),
            server: None,
        })
    }
    pub fn credentials(&self, account: &str) -> (Zeroizing<String>, Zeroizing<[u8; 32]>) {
        let state = self.state.lock().unwrap();
        let a = &state.accounts[account];
        (a.token.clone(), a.key.clone())
    }
    pub fn counts(&self) -> Counts {
        self.state.lock().unwrap().counts.clone()
    }
    pub fn request(&self, account: &str, method: &str, path: &str, body: &[u8]) -> Result<Response> {
        let (token, _) = self.credentials(account);
        self.state.lock().unwrap().dispatch(method, path, &token, body)
    }
    pub fn expire(&self, account: &str) {
        self.state.lock().unwrap().accounts.get_mut(account).unwrap().expired = true;
    }
    pub fn revoke_share(&self) {
        self.state.lock().unwrap().share_active = false;
    }
    pub fn remote_edit(&self) {
        let mut state = self.state.lock().unwrap();
        state.objects[6].active = 2;
        let row = state.row(&state.objects[6]);
        let seq = state.ops.len() + 1;
        state
            .ops
            .push(json!({"seq_id":seq,"op_type":"file_update","payload":row}));
    }
    pub fn trash_subtree(&self) {
        let mut state = self.state.lock().unwrap();
        state.objects[7].trashed = true;
        state.objects[8].trashed = true;
        let seq = state.ops.len() + 1;
        state
            .ops
            .push(json!({"seq_id":seq,"op_type":"file_trash","payload":{"id":id(8)}}));
    }
    pub fn start_http(&mut self) -> Result<String> {
        ensure!(self.server.is_none(), "fixture listener already started");
        let server = Server::start(Arc::clone(&self.state))?;
        let url = server.url.clone();
        self.server = Some(server);
        Ok(url)
    }
    pub fn cleanup(mut self) -> Result<Cleanup> {
        if let Some(server) = self.server.take() {
            server.finish()?;
        }
        let mut state = self.state.lock().unwrap();
        let removed_objects = state.objects.len();
        let removed_accounts = state.accounts.len();
        let removed_uploads = usize::from(state.upload.is_some());
        ensure!(
            removed_objects == 14 && removed_accounts == 2,
            "cleanup must remove seeded state"
        );
        state.objects.clear();
        state.accounts.clear();
        state.ops.clear();
        state.upload = None;
        state.share_active = false;
        let path = self.root.path().to_path_buf();
        self.root.close()?;
        let report = Cleanup {
            removed_objects,
            removed_accounts,
            removed_uploads,
            remaining_objects: state.objects.len(),
            remaining_accounts: state.accounts.len(),
            remaining_uploads: usize::from(state.upload.is_some()),
            remaining_paths: usize::from(path.exists()),
            remaining_listeners: usize::from(self.server.is_some()),
        };
        ensure!(report.remaining_paths == 0, "fixture directory remains");
        Ok(report)
    }
}

struct Server {
    url: String,
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<Result<()>>>,
}
impl Server {
    fn start(state: Arc<Mutex<State>>) -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").context("fixture loopback bind (unavailable is not a pass)")?;
        listener.set_nonblocking(true)?;
        let url = format!("http://{}", listener.local_addr()?);
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let handle = thread::spawn(move || {
            while !worker_stop.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream.set_read_timeout(Some(Duration::from_secs(2)))?;
                        stream.set_write_timeout(Some(Duration::from_secs(2)))?;
                        let response = match read_request(&mut stream) {
                            Ok((method, path, token, body)) => state
                                .lock()
                                .unwrap()
                                .dispatch(&method, &path, &token, &body)
                                .unwrap_or_else(|_| Response::json(400, json!({"error":"invalid fixture request"}))),
                            Err(_) => Response::json(400, json!({"error":"invalid fixture HTTP framing"})),
                        };
                        // A disconnected client is a normal interruption; don't log its headers.
                        let _ = write!(
                            stream,
                            "HTTP/1.1 {} Fixture\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            response.status,
                            response.body.len()
                        )
                        .and_then(|_| stream.write_all(&response.body));
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => thread::sleep(Duration::from_millis(5)),
                    Err(e) => return Err(e.into()),
                }
            }
            Ok(())
        });
        Ok(Self {
            url,
            stop,
            handle: Some(handle),
        })
    }
    fn finish(mut self) -> Result<()> {
        self.stop.store(true, Ordering::SeqCst);
        self.handle
            .take()
            .unwrap()
            .join()
            .map_err(|_| anyhow::anyhow!("fixture HTTP worker panicked"))?
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}
fn read_request(stream: &mut TcpStream) -> Result<(String, String, Zeroizing<String>, Vec<u8>)> {
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut bytes = Zeroizing::new(Vec::new());
    let header_end = loop {
        ensure!(bytes.len() < 16384 && Instant::now() < deadline, "fixture header limit");
        let mut byte = [0];
        ensure!(stream.read(&mut byte)? == 1, "fixture EOF");
        bytes.push(byte[0]);
        if bytes.ends_with(b"\r\n\r\n") {
            break bytes.len();
        }
    };
    let headers = std::str::from_utf8(&bytes)?;
    let mut lines = headers.split("\r\n");
    let first = lines.next().unwrap().split_whitespace().collect::<Vec<_>>();
    ensure!(first.len() == 3, "fixture request line");
    let (method, path) = (first[0].to_owned(), first[1].to_owned());
    let mut token = Zeroizing::new(String::new());
    let mut size = 0;
    for line in lines {
        if let Some((key, value)) = line.split_once(':') {
            let value = value.trim();
            if key.eq_ignore_ascii_case("authorization") {
                *token = value.strip_prefix("Bearer ").unwrap_or("").to_owned();
            }
            if key.eq_ignore_ascii_case("content-length") {
                size = value.parse::<usize>()?;
            }
            ensure!(
                !key.eq_ignore_ascii_case("transfer-encoding"),
                "unsupported transfer encoding"
            );
        }
    }
    ensure!(size <= 1024 * 1024, "fixture body limit");
    bytes.resize(header_end + size, 0);
    let mut offset = header_end;
    while offset < bytes.len() {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .context("fixture body deadline")?;
        stream.set_read_timeout(Some(remaining))?;
        let n = stream.read(&mut bytes[offset..])?;
        ensure!(n > 0, "fixture body EOF");
        offset += n;
    }
    Ok((method, path, token, bytes[header_end..].to_vec()))
}
