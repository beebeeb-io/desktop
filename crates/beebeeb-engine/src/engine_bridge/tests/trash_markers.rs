//! `trash_markers` engine tests, split out of `engine_bridge.rs`.

use super::*;
#[test]
fn test_1698_review_trash_echo_marks_descendants_of_an_already_parked_folder() {
    // PR #100 review (Codex P1, engine_bridge.rs ~5648): Finder trashing a
    // folder parks ONLY the folder row (`queue_finder_delete`); the
    // server's `file_trash` echo is what marks the descendants. The
    // parked-root early return discarded the echo entirely, so nested
    // rows stayed unmarked — and `prune_absent` protects only children
    // whose IMMEDIATE parent is `Trashing`, so grandchildren could drop
    // out of the trash view. The echo must still mark the unmarked
    // subtree when the root is already parked.
    let dir = tempfile::tempdir().unwrap();
    let bridge = test_bridge(&dir.path().join("state.db"));
    let folder = "park0000-0000-4000-8000-000000000001";
    let child = "park0000-0000-4000-8000-000000000002";
    let grandchild = "park0000-0000-4000-8000-000000000003";
    seed_bridge_entry(&bridge, folder, "docs", None, FileStatus::Trashing, true, 10);
    seed_bridge_entry(
        &bridge,
        child,
        "docs/notes.txt",
        Some(folder),
        FileStatus::CloudOnly,
        false,
        10,
    );
    seed_bridge_entry(
        &bridge,
        grandchild,
        "docs/notes/attachment.bin",
        Some(child),
        FileStatus::CloudOnly,
        false,
        10,
    );

    let op = crate::api_client::SyncOp {
        seq_id: 7,
        op_type: "file_trash".into(),
        payload: serde_json::json!({ "id": folder }),
    };
    let mut conflicts = Vec::new();
    let applied = apply_sync_op(&bridge, dir.path(), &op, 200, &mut conflicts).unwrap();
    for id in [folder, child, grandchild] {
        let row = bridge.db().get_file(id).unwrap().unwrap();
        assert_eq!(row.status, FileStatus::Trashing, "{id} must be in the trash view");
    }
    assert!(
        applied.contains(&grandchild.to_string()) && applied.contains(&child.to_string()),
        "newly marked rows are reported for the working-set signal: {applied:?}"
    );
}

#[test]
fn test_1698_review_restore_un_trashes_the_entire_held_subtree() {
    // PR #100 review (Codex P1, engine_bridge.rs ~5715): a restored FOLDER
    // must take its whole held subtree out of the trash view, not just the
    // root row. The authoritative re-snapshot preserves `Trashing` rows by
    // design, so it can never repair the descendants — they would linger
    // in the Trash view, and direct children would even become top-level
    // trash entries after the parent leaves.
    let dir = tempfile::tempdir().unwrap();
    let bridge = test_bridge(&dir.path().join("state.db"));
    let folder = "rest0000-0000-4000-8000-000000000001";
    let child = "rest0000-0000-4000-8000-000000000002";
    let grandchild = "rest0000-0000-4000-8000-000000000003";
    seed_bridge_entry(&bridge, folder, "docs", None, FileStatus::Trashing, true, 10);
    seed_bridge_entry(
        &bridge,
        child,
        "docs/notes.txt",
        Some(folder),
        FileStatus::Trashing,
        false,
        10,
    );
    seed_bridge_entry(
        &bridge,
        grandchild,
        "docs/notes/attachment.bin",
        Some(child),
        FileStatus::Trashing,
        false,
        10,
    );

    let op = crate::api_client::SyncOp {
        seq_id: 8,
        op_type: "file_restore".into(),
        payload: serde_json::json!({ "id": folder }),
    };
    let mut conflicts = Vec::new();
    let applied = apply_sync_op(&bridge, dir.path(), &op, 200, &mut conflicts).unwrap();
    for id in [folder, child, grandchild] {
        let row = bridge.db().get_file(id).unwrap().unwrap();
        assert_eq!(
            row.status,
            FileStatus::CloudOnly,
            "{id} must leave the trash view with the restored folder"
        );
    }
    assert!(
        applied.contains(&grandchild.to_string()) && applied.contains(&folder.to_string()),
        "every flipped row is reported for the working-set signal: {applied:?}"
    );
}

// ── A server that keeps versions and refuses a stale base ──────────────

/// One file on [`VersionedServerMock`]: every completed version's chunks,
/// encrypted exactly as the client sent them.
#[derive(Default)]
pub(super) struct MockServerFile {
    pub(super) versions: Vec<Vec<Vec<u8>>>,
}

#[derive(Default)]
pub(super) struct VersionedServerState {
    pub(super) files: std::collections::BTreeMap<String, MockServerFile>,
    pub(super) authenticated_names: HashMap<String, String>,
    /// session id -> (file id, chunks received so far)
    pub(super) sessions: HashMap<String, (String, Vec<Vec<u8>>)>,
    pub(super) next_file: usize,
    pub(super) next_session: usize,
    /// Every `uploads/init` body, in order, with the status it got.
    pub(super) inits: Vec<(serde_json::Value, u16)>,
    /// Every `PATCH /files/{id}`: (file id, body).
    pub(super) patches: Vec<(String, serde_json::Value)>,
    /// Sessions whose first chunk PUT answers 500 once.
    pub(super) fail_first_chunk_once: HashSet<String>,
    /// Sessions whose chunk PUTs answer only after this delay.
    pub(super) delay_chunks: HashMap<String, Duration>,
    /// Every request, in order: (method, path).
    pub(super) requests: Vec<(String, String)>,
    /// `POST /api/v1/uploads/init` answers only after this delay.
    pub(super) delay_init: Option<Duration>,
    /// file id -> the status line its `uploads/init` is refused with.
    pub(super) refuse_init: HashMap<String, &'static str>,
    /// file id -> the status line and the exact JSON body its `uploads/init` is refused with
    /// (the server's real refusal bodies, spec §8.4).
    pub(super) refuse_init_with: HashMap<String, (&'static str, serde_json::Value)>,
    /// file id -> the status line and body every chunk `PUT` of its sessions is refused with.
    pub(super) refuse_chunk_with: HashMap<String, (&'static str, serde_json::Value)>,
    /// file id -> the status line and body every `complete` of its sessions is refused with;
    /// the session stays.
    pub(super) refuse_complete_with: HashMap<String, (&'static str, serde_json::Value)>,
    /// file id -> the 409 message its next `uploads/init` gets, once (spec §8.4).
    pub(super) conflict_init_once: HashMap<String, String>,
    /// Every `POST /files/{id}/versions/{vid}/restore`: (file id, object version id).
    pub(super) restores: Vec<(String, String)>,
    /// `GET /api/v1/sync/snapshot` answers these in order: (status line, body). Empty:
    /// `503 Service Unavailable`.
    pub(super) snapshots: VecDeque<(String, serde_json::Value)>,
    /// Sessions whose `complete` answers only after this delay. The server has completed
    /// the upload before the delay starts.
    pub(super) delay_complete: HashMap<String, Duration>,
    /// Every `DELETE /api/v1/files/{id}` (a trash): the file id.
    pub(super) trashes: Vec<String>,
    /// `GET /api/v1/shares/invites/incoming` answers this body; unset: no invites.
    pub(super) incoming_invites: Option<serde_json::Value>,
    /// Sessions whose first `complete` completes the upload but loses its reply (500).
    pub(super) complete_reply_lost_once: HashSet<String>,
    /// Sessions completed with a lost reply: session id -> file id. A repeated `complete`
    /// answers as the server's idempotent retry does, with neither the version nor the
    /// object id (`routes/uploads.rs`, `complete_upload`).
    pub(super) completed_reply_lost: HashMap<String, String>,
    /// file id -> the encrypted thumbnail blob `GET /files/{id}/thumbnail/{variant}` answers.
    pub(super) thumbnails: HashMap<String, Vec<u8>>,
    /// Every thumbnail `GET`: the file id.
    pub(super) thumbnail_requests: Vec<String>,
    /// Every thumbnail `PUT`: the file id. The blob replaces `thumbnails[id]`.
    pub(super) thumbnail_puts: Vec<String>,
    /// Thumbnail `PUT`s answer only after this delay (the server stored the blob first).
    pub(super) delay_thumbnail_put: Option<Duration>,
    /// Thumbnail `PUT`s answer 500 and store nothing.
    pub(super) fail_thumbnail_put: bool,
    /// A create mints a UUID-shaped id instead of `server-file-N` (the hydrate and the
    /// thumbnail fetch parse a UUID before they ask the server).
    pub(super) uuid_ids: bool,
    /// Sessions whose next `complete` answers 500 and completes nothing; the session stays.
    pub(super) fail_complete_once: HashSet<String>,
    /// Files trashed on the server: every `uploads/init` naming one answers
    /// `409 {"error": "file is in trash"}` (spec §9.1, m-8).
    pub(super) trashed_files: HashSet<String>,
    /// `GET /files/{id}` and `GET /files/{id}/chunks/{i}` answer with the file's latest
    /// version. Off: `GET /files/{id}` answers 404, as before.
    pub(super) serve_downloads: bool,
    /// The metadata names no `download_version` (a legacy file, an older server).
    pub(super) legacy_downloads: bool,
}

/// Upload mock that behaves like the server's version check: a replace
/// (`file_id` set) whose `base_version_number` is not the file's current
/// version gets 409; a replace naming an id the server never minted
/// creates a file under that id, as the server does for a client-chosen
/// id. A create (`file_id` absent) mints `server-file-N` (a UUID-shaped id with
/// `uuid_ids`). `GET /files/{id}` has no route: it answers 404, so a hydrate from the
/// server fails.
pub(super) struct VersionedServerMock {
    pub(super) base_url: String,
    pub(super) state: Arc<Mutex<VersionedServerState>>,
    pub(super) stop: Arc<AtomicBool>,
    pub(super) handle: thread::JoinHandle<()>,
}

impl VersionedServerMock {
    pub(super) fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let state = Arc::new(Mutex::new(VersionedServerState::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let server_state = Arc::clone(&state);
        let server_stop = Arc::clone(&stop);
        let handle = thread::spawn(move || {
            let started = std::time::Instant::now();
            while !server_stop.load(Ordering::SeqCst) && started.elapsed() < Duration::from_secs(30) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream.set_nonblocking(false).unwrap();
                        let request = read_http_request(&mut stream);
                        let (delay, response) = versioned_server_response(&request, &server_state);
                        if let Some(delay) = delay {
                            std::thread::sleep(delay);
                        }
                        let _ = stream.write_all(&response);
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(e) => panic!("versioned server mock accept failed: {e}"),
                }
            }
        });
        Self {
            base_url,
            state,
            stop,
            handle,
        }
    }

    /// Put `file_id` on the server at `versions` versions (empty
    /// content), as if it had been uploaded before the test.
    pub(super) fn seed_file(&self, file_id: &str, versions: usize) {
        self.state.lock().unwrap().files.insert(
            file_id.to_string(),
            MockServerFile {
                versions: vec![Vec::new(); versions],
            },
        );
    }

    pub(super) fn finish(self) -> VersionedServerState {
        self.stop.store(true, Ordering::SeqCst);
        self.handle.join().unwrap();
        Arc::try_unwrap(self.state)
            .unwrap_or_else(|_| panic!("mock state still shared"))
            .into_inner()
            .unwrap()
    }
}

impl VersionedServerState {
    /// Each `uploads/init` as `(file_id, base_version_number, status)`.
    pub(super) fn init_summary(&self) -> Vec<(serde_json::Value, serde_json::Value, u16)> {
        self.inits
            .iter()
            .map(|(body, status)| (body["file_id"].clone(), body["base_version_number"].clone(), *status))
            .collect()
    }

    /// The plaintext of `file_id`'s latest version.
    pub(super) fn latest_plaintext(&self, file_id: &str, master_key: [u8; 32]) -> Vec<u8> {
        let file = self
            .files
            .get(file_id)
            .unwrap_or_else(|| panic!("no server file {file_id}"));
        let chunks = file
            .versions
            .last()
            .unwrap_or_else(|| panic!("{file_id} has no version"));
        let master_key = beebeeb_core::kdf::MasterKey::from_bytes(master_key);
        let file_key = beebeeb_core::kdf::derive_file_key(&master_key, file_id.as_bytes());
        // Read as a reader does: against the record of the name the server holds for the file (the
        // completion name of its latest upload). A file with no such name is a v1 file.
        let record = self
            .authenticated_names
            .get(file_id)
            .and_then(|name| beebeeb_core::encrypt::decrypt_file_metadata_with_key(&file_key, name).ok())
            .map_or(crate::content_record::ContentRecord::UNRECORDED, |opened| {
                crate::content_record::ContentRecord::of(&opened)
            });
        let mut reader = record.decryptor(file_key, file_id);
        let mut plain = Vec::new();
        for chunk in chunks {
            plain.extend_from_slice(&reader.push_frame(chunk).expect("a stored frame opens").data);
        }
        reader.finish().expect("the stored version is whole");
        plain
    }
}

/// The UUID the mock names version `n` (1-based) of `file_id` by in a download manifest.
fn mock_object_version_id(file_id: &str, n: usize) -> String {
    let tag = file_id
        .bytes()
        .fold(0u32, |acc, b| acc.wrapping_mul(31).wrapping_add(u32::from(b)));
    format!("{tag:08x}-0000-4000-8000-{n:012x}")
}

fn versioned_server_response(
    request: &RecordedRequest,
    state: &Arc<Mutex<VersionedServerState>>,
) -> (Option<Duration>, Vec<u8>) {
    let mut s = state.lock().unwrap();
    s.requests.push((request.method.clone(), request.path.clone()));
    // `PUT /files/{id}/thumbnail[/{variant}]` (medium has no variant segment): the blob
    // replaces the file's thumbnail, as the server stores one per file, not per version.
    if request.method == "PUT"
        && let Some(rest) = request.path.strip_prefix("/api/v1/files/")
        && let Some((file_id, _variant)) = rest.split_once("/thumbnail")
    {
        let file_id = file_id.to_string();
        s.thumbnail_puts.push(file_id.clone());
        if s.fail_thumbnail_put {
            return (
                None,
                http_json("500 Internal Server Error", serde_json::json!({ "error": "boom" })).into_bytes(),
            );
        }
        s.thumbnails.insert(file_id, request.body.clone());
        return (
            s.delay_thumbnail_put,
            http_json("200 OK", serde_json::json!({ "ok": true })).into_bytes(),
        );
    }
    // `GET /files/{id}/thumbnail/{variant}`: the blob a test seeded, or 404 (binary body).
    if request.method == "GET"
        && let Some(rest) = request.path.strip_prefix("/api/v1/files/")
        && let Some((file_id, _variant)) = rest.split_once("/thumbnail/")
    {
        let file_id = file_id.to_string();
        s.thumbnail_requests.push(file_id.clone());
        return match s.thumbnails.get(&file_id) {
            Some(blob) => (None, http_bytes("200 OK", blob)),
            None => (
                None,
                http_json("404 Not Found", serde_json::json!({ "error": "no thumbnail" })).into_bytes(),
            ),
        };
    }
    // `GET /files/{id}[?object_version_id=]` and `GET /files/{id}/chunks/{i}[?object_version_id=]`:
    // the file's latest version, or the pinned one, as the client uploaded it (binary chunk
    // bodies). The metadata carries the server's `download_version` manifest (routes/files.rs,
    // routes/version_download.rs), with the version's number.
    if s.serve_downloads
        && request.method == "GET"
        && let Some(rest) = request.path.strip_prefix("/api/v1/files/")
        && !rest.contains("/versions")
    {
        let (rest, query) = rest.split_once('?').unwrap_or((rest, ""));
        let pinned = query.strip_prefix("object_version_id=").map(str::to_string);
        let (file_id, chunk) = match rest.split_once("/chunks/") {
            Some((file_id, index)) => (file_id.to_string(), index.parse::<usize>().ok()),
            None => (rest.to_string(), None),
        };
        let selected = s.files.get(&file_id).and_then(|file| match &pinned {
            Some(pin) => (1..=file.versions.len())
                .find(|n| mock_object_version_id(&file_id, *n) == *pin)
                .map(|n| (n, file.versions[n - 1].clone())),
            None => file.versions.last().map(|chunks| (file.versions.len(), chunks.clone())),
        });
        let latest = selected.as_ref().map(|(_, chunks)| chunks.clone());
        return match (latest, chunk) {
            (Some(chunks), None) => {
                let version = selected.map_or(0, |(n, _)| n);
                // nonce (12) + tag (16) per chunk.
                let size: usize = chunks.iter().map(|chunk| chunk.len().saturating_sub(28)).sum();
                let wire: usize = chunks.iter().map(Vec::len).sum();
                let name = s.authenticated_names.get(&file_id).cloned().or_else(|| {
                    s.patches
                        .iter()
                        .rev()
                        .find(|(id, _)| id == &file_id)
                        .and_then(|(_, body)| body["name_encrypted"].as_str().map(str::to_string))
                });

                if s.legacy_downloads {
                    return (
                            None,
                            http_json(
                                "200 OK",
                                serde_json::json!({ "id": file_id, "name_encrypted": name, "chunk_count": chunks.len(), "size_bytes": size }),
                            )
                            .into_bytes(),
                        );
                }
                (
                        None,
                        http_json(
                            "200 OK",
                            serde_json::json!({
                                "id": file_id,
                                "chunk_count": chunks.len(),
                                "name_encrypted": name,
                                "size_bytes": size,
                                "download_version": {
                                    "object_version_id": mock_object_version_id(&file_id, version),
                                    "version_number": version,
                                    "chunk_count": chunks.len(),
                                    "chunk_size_bytes": 1_048_576,
                                    "plaintext_size_bytes": size,
                                    "ciphertext_size_bytes": wire,
                                    "chunks": chunks
                                        .iter()
                                        .enumerate()
                                        .map(|(index, chunk)| serde_json::json!({ "index": index, "size_bytes": chunk.len() }))
                                        .collect::<Vec<_>>(),
                                },
                            }),
                        )
                        .into_bytes(),
                    )
            }
            (Some(chunks), Some(index)) if index < chunks.len() => (None, http_bytes("200 OK", &chunks[index])),
            _ => (
                None,
                http_json("404 Not Found", serde_json::json!({ "error": "no such file" })).into_bytes(),
            ),
        };
    }
    let delay = if request.method == "POST" && request.path == "/api/v1/uploads/init" {
        s.delay_init
    } else if request.method == "POST" && request.path.ends_with("/complete") {
        request
            .path
            .strip_prefix("/api/v1/uploads/")
            .and_then(|rest| rest.split('/').next())
            .and_then(|session| s.delay_complete.get(session).copied())
    } else {
        request
            .path
            .strip_prefix("/api/v1/uploads/")
            .and_then(|rest| rest.split('/').next())
            .filter(|_| request.method == "PUT")
            .and_then(|session| s.delay_chunks.get(session).copied())
    };
    (delay, versioned_server_answer(request, &mut s).into_bytes())
}
