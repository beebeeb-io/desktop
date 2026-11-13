use crate::owner_keys::OwnedKeyRecord;
use beebeeb_core::{
    chunk_stream::{ChunkDecryptor, ChunkEncryptor, ExpectedChunkFormat},
    kdf::{FileKey, MasterKey},
};
const ID: &str = "30000000-0000-8000-8000-000000000003";
/// An owner-class item: the same flows with no attribution and no envelope.
const OWNER: &str = "40000000-0000-4000-8000-000000000004";
const ROOT: &str = "10000000-0000-4000-8000-000000000001";
const OBJECT: &str = "90000000-0000-4000-8000-000000000009";
fn random_key() -> FileKey {
    FileKey::from_bytes([66; 32])
}
thread_local! {
    /// The commitments of the v2 uploads [`frames`] made on this test's thread, per key, oldest
    /// first. A v2 version opens only against the commitment in its file's name, so the names this
    /// fixture seals ([`named`]) carry the commitment of the newest upload under their key, as a
    /// writer's completion name does. Per thread: every test here builds its world on its own.
    static COMMITMENTS: std::cell::RefCell<Vec<([u8; 32], beebeeb_core::chunk_stream::VersionCommitment)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}
fn commitments(key: &FileKey) -> Vec<beebeeb_core::chunk_stream::VersionCommitment> {
    COMMITMENTS.with(|all| {
        all.borrow()
            .iter()
            .filter(|(of, _)| of == key.as_bytes())
            .map(|(_, version)| *version)
            .collect()
    })
}
/// A sealed name. Under a v2 pin it carries the commitment of the newest v2 upload [`frames`] made
/// under `key` on this thread (none was made: none, as on a row whose content is not uploaded yet).
fn named(key: &FileKey, text: &str, pin: ExpectedChunkFormat) -> String {
    match (pin, commitments(key).last()) {
        (ExpectedChunkFormat::V2, Some(version)) => {
            beebeeb_core::encrypt::encrypt_name_for_version(key, text, Some("text/plain"), pin, *version).unwrap()
        }
        _ => beebeeb_core::encrypt::encrypt_name_for(key, text, Some("text/plain"), pin).unwrap(),
    }
}
/// A file's content in 4-byte chunks. No pin: v1 frames, as every build before chunk format v2 wrote
/// them (a file with v1 history). A v2 pin: a v2 upload, whose commitment the next [`named`] carries.
fn frames(key: &FileKey, id: &str, plain: &[u8], pin: ExpectedChunkFormat) -> Vec<Vec<u8>> {
    if pin == ExpectedChunkFormat::Unrecorded {
        return plain
            .chunks(4)
            .map(|bytes| beebeeb_core::encrypt::encrypt_chunk_raw(key, bytes).unwrap())
            .collect();
    }
    let mut enc =
        ChunkEncryptor::for_push_with_file_key(FileKey::from_bytes(*key.as_bytes()), id, plain.len() as u64, 4)
            .unwrap()
            .with_recorded_pin(pin)
            .unwrap();
    let mut result = Vec::new();
    for bytes in plain.chunks(4) {
        result.push(enc.push_chunk(bytes).unwrap().data);
    }
    let previous = commitments(key).last().map(|version| version.counter);
    let version = enc.version_commitment_after(previous).unwrap();
    enc.finish().unwrap();
    COMMITMENTS.with(|all| all.borrow_mut().push((*key.as_bytes(), version)));
    result
}
/// The plaintext of frames [`frames`] made: v1 frames under no pin, a v2 upload against the one of
/// this thread's commitments under `key` that opens it.
fn opened(key: &FileKey, id: &str, wire: &[Vec<u8>], pin: ExpectedChunkFormat) -> Vec<u8> {
    let read = |version| {
        let mut dec = ChunkDecryptor::for_version_with_file_key(FileKey::from_bytes(*key.as_bytes()), id, pin, version);
        let mut bytes = Vec::new();
        for frame in wire {
            bytes.extend(dec.push_frame(frame).ok()?.data);
        }
        dec.finish().ok()?;
        Some(bytes)
    };
    if pin == ExpectedChunkFormat::Unrecorded {
        return read(None).expect("v1 frames open under no pin");
    }
    commitments(key)
        .into_iter()
        .rev()
        .find_map(|version| read(Some(version)))
        .expect("one of this thread's uploads under the key is these frames")
}
/// The plaintext of frames the ENGINE uploaded, read against the record of the name it sealed for
/// them.
fn opened_as(key: &FileKey, id: &str, wire: &[Vec<u8>], sealed_name: &str) -> Vec<u8> {
    let record = crate::content_record::ContentRecord::of(
        &beebeeb_core::encrypt::decrypt_file_metadata_with_key(key, sealed_name).unwrap(),
    );
    let mut dec = record.decryptor(FileKey::from_bytes(*key.as_bytes()), id);
    let mut bytes = Vec::new();
    for frame in wire {
        bytes.extend(dec.push_frame(frame).unwrap().data);
    }
    dec.finish().unwrap();
    bytes
}
fn manifest(wire: &[Vec<u8>], size: usize) -> serde_json::Value {
    json!({"object_version_id":OBJECT,"version_number":1,"chunk_count":wire.len(),"chunk_size_bytes":4,"plaintext_size_bytes":size,"ciphertext_size_bytes":wire.iter().map(Vec::len).sum::<usize>(),"chunks":wire.iter().enumerate().map(|(i,f)|json!({"index":i,"size_bytes":f.len()})).collect::<Vec<_>>()})
}
struct ContentState {
    rv_routes: std::collections::HashMap<String, (u16, Vec<u8>)>,
    nodes: Vec<serde_json::Value>,
    ops: Vec<serde_json::Value>,
    seq: i64,
    invites: serde_json::Value,
    meta: serde_json::Value,
    frames: Vec<Vec<u8>>,
    requests: Vec<RecordedRequest>,
    uploads: Vec<Vec<u8>>,
    gone: usize,
    meta_reads: usize,
}
struct ContentServer {
    base: String,
    state: Arc<Mutex<ContentState>>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}
impl ContentServer {
    fn start(key: &FileKey, pin: ExpectedChunkFormat) -> Self {
        Self::start_as(ID, key, pin)
    }
    /// An owner-class item served from the same fixture.
    fn start_owner(key: &FileKey, pin: ExpectedChunkFormat) -> Self {
        Self::start_as(OWNER, key, pin)
    }
    fn start_as(id: &str, key: &FileKey, pin: ExpectedChunkFormat) -> Self {
        let plain = b"guest bytes!";
        // The content first: the name carries the commitment of its upload.
        let content = frames(key, id, plain, pin);
        let state = Arc::new(Mutex::new(ContentState {
            rv_routes: Default::default(),
            nodes: vec![],
            ops: vec![],
            seq: 0,
            invites: json!({"invites":[]}),
            meta: json!({"id":id,"name_encrypted":named(key,"guest.txt",pin),"size_bytes":plain.len(),"chunk_count":3,"chunk_size_bytes":4,"version_number":1}),
            frames: content,
            requests: vec![],
            uploads: vec![],
            gone: 0,
            meta_reads: 0,
        }));
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let signal = stop.clone();
        let world = state.clone();
        let thread = thread::spawn(move || {
            while !signal.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream.set_nonblocking(false).unwrap();
                        stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
                        let req = read_http_request(&mut stream);
                        let mut w = world.lock().unwrap();
                        let (status, body) = if let Some(reply) = w.rv_routes.get(&req.path).cloned() {
                            reply
                        } else if req.path == "/api/v1/sync/snapshot" {
                            (
                                200,
                                serde_json::to_vec(&json!({"seq_id":w.seq,"nodes":w.nodes})).unwrap(),
                            )
                        } else if let Some(since) = req.path.strip_prefix("/api/v1/sync/ops?since=") {
                            let since = since.parse::<i64>().unwrap();
                            (200,serde_json::to_vec(&json!({"since":since,"ops":w.ops.iter().filter(|o|o["seq_id"].as_i64().unwrap()>since).collect::<Vec<_>>()})).unwrap())
                        } else if req.method == "GET"
                            && w.nodes
                                .iter()
                                .any(|n| req.path == format!("/api/v1/files/{}", n["id"].as_str().unwrap()))
                        {
                            let n = w
                                .nodes
                                .iter()
                                .find(|n| req.path == format!("/api/v1/files/{}", n["id"].as_str().unwrap()))
                                .unwrap();
                            (200, serde_json::to_vec(n).unwrap())
                        } else if req.method == "GET"
                            && req.path == format!("/api/v1/files/{}", w.meta["id"].as_str().unwrap())
                        {
                            w.meta_reads += 1;
                            (200, serde_json::to_vec(&w.meta).unwrap())
                        } else if req.method == "GET" && req.path.contains("/chunks/") {
                            let index = req
                                .path
                                .split("/chunks/")
                                .nth(1)
                                .unwrap()
                                .split('?')
                                .next()
                                .unwrap()
                                .parse::<usize>()
                                .unwrap();
                            if w.gone > 0 {
                                w.gone -= 1;
                                (409, b"{}".to_vec())
                            } else {
                                (200, w.frames[index].clone())
                            }
                        } else if req.path == "/api/v1/shares/invites/incoming" {
                            (200, serde_json::to_vec(&w.invites).unwrap())
                        } else if req.path.contains("/thumbnail/") {
                            (
                                200,
                                beebeeb_core::encrypt::encrypt_thumbnail_for(&random_key(), b"thumbnail", pin).unwrap(),
                            )
                        } else if req.method == "PUT" && req.path.contains("/uploads/") {
                            w.uploads.push(req.body.clone());
                            (
                                200,
                                serde_json::to_vec(&json!({"index":w.uploads.len()-1,"size":req.body.len()})).unwrap(),
                            )
                        } else if req.method == "POST" && req.path.ends_with("/complete") {
                            (200, b"{}".to_vec())
                        } else {
                            (404, b"{}".to_vec())
                        };
                        w.requests.push(req);
                        drop(w);
                        let header = format!(
                            "HTTP/1.1 {status} Fixture\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        );
                        stream.write_all(header.as_bytes()).unwrap();
                        stream.write_all(&body).unwrap();
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => thread::sleep(Duration::from_millis(2)),
                    Err(e) => panic!("{e}"),
                }
            }
        });
        Self {
            base,
            state,
            stop,
            thread: Some(thread),
        }
    }
    fn bridge(&self, path: &Path, foreign: bool) -> EngineBridge {
        let b = test_bridge_with_api(path, self.base.clone(), [7; 32]);
        let id = self.state.lock().unwrap().meta["id"].as_str().unwrap().to_string();
        if foreign {
            b.db.put_owned_key_record(&OwnedKeyRecord {
                file_id: id.clone(),
                name_encrypted: self.state.lock().unwrap().meta["name_encrypted"]
                    .as_str()
                    .unwrap()
                    .into(),
                owner_wrap: Some(
                    beebeeb_sync::key_resolution::wrap_owner_file_key(
                        &MasterKey::from_bytes([7; 32]),
                        &id,
                        &random_key(),
                    )
                    .unwrap(),
                ),
                foreign_added: true,
                shared_root_id: Some(ROOT.into()),
                ..Default::default()
            })
            .unwrap();
        }
        seed_bridge_row(&b, &id, "guest.txt", None, FileStatus::CloudOnly, 0);
        b
    }
}
impl Drop for ContentServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let result = self.thread.take().unwrap().join();
        if !thread::panicking() {
            result.unwrap();
        }
    }
}
fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Runtime::new().unwrap()
}
