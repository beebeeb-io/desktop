//! `downloads` engine tests, split out of `engine_bridge.rs`.

use super::*;
/// Task 1643: every chunk of one download comes from ONE version. A loopback server that behaves
/// like the real one (contract "Version-bound downloads"): the metadata names the current version
/// with its manifest; an unpinned chunk request gets the CURRENT version's chunk; a pinned one gets
/// the pinned version's chunk while it is retained, else 409. Between two chunk requests the test
/// commits the next version (the "pause, commit an equal-size next version, resume" of the task).
/// Self-contained on purpose: the same module runs on main, where it shows the mixed bytes.
mod version_bound_downloads_1643 {
    use super::*;
    use sha2::Digest;

    const KEY: [u8; 32] = [43; 32];
    const STRIDE: usize = 1024;
    const GONE: &str = "download version unavailable; restart download from fresh metadata";

    struct Version {
        id: &'static str,
        plain: Vec<u8>,
        stride: usize,
        frames: Vec<Vec<u8>>,
        retained: bool,
    }

    impl Version {
        fn new(id: &'static str, plain: Vec<u8>) -> Self {
            Self::with_stride(id, plain, STRIDE)
        }
        fn with_stride(id: &'static str, plain: Vec<u8>, stride: usize) -> Self {
            let key = hydration_test_key(KEY, TEST_FILE_ID);
            let frames = plain.chunks(stride).map(|c| encrypt_chunk_wire(&key, c)).collect();
            Version {
                id,
                plain,
                stride,
                frames,
                retained: true,
            }
        }
        /// The last chunk in the legacy JSON framing `content_frame` still accepts, padded
        /// to the manifest's exact wire size but holding `plain`: a chunk whose wire size is right and
        /// whose plaintext is not (only the plaintext totals can see it).
        fn with_json_last_chunk(mut self, plain: &[u8]) -> Self {
            let key = hydration_test_key(KEY, TEST_FILE_ID);
            let last = self.frames.len() - 1;
            let wire = self.frames[last].len();
            let mut json = serde_json::to_vec(&beebeeb_core::encrypt::encrypt_chunk(&key, plain).unwrap()).unwrap();
            assert!(
                json.len() <= wire,
                "the JSON frame ({} bytes) must fit the wire size {wire}",
                json.len()
            );
            json.resize(wire, b' ');
            self.frames[last] = json;
            self
        }
        fn manifest(&self) -> serde_json::Value {
            let chunks: Vec<serde_json::Value> = self
                .frames
                .iter()
                .enumerate()
                .map(|(i, f)| json!({"index": i, "size_bytes": f.len()}))
                .collect();
            json!({
                "object_version_id": self.id, "version_number": 1, "chunk_count": self.frames.len(),
                "chunk_size_bytes": self.stride, "plaintext_size_bytes": self.plain.len(),
                "ciphertext_size_bytes": self.frames.iter().map(Vec::len).sum::<usize>(), "chunks": chunks,
            })
        }
    }

    /// Equal-size versions with different bytes: a mix is a third hash.
    fn content(fill: u8, len: usize) -> Vec<u8> {
        (0..len).map(|i| fill ^ (i as u8)).collect()
    }

    struct State {
        versions: Vec<Version>,
        current: usize,
        /// (after this many chunk requests were answered, the version that becomes current,
        /// whether the old one is pruned at that moment)
        commits: Vec<(usize, usize, bool)>,
        chunk_requests: usize,
        /// A server bug for one test: pinned chunk requests are answered from the current version.
        ignores_pin: bool,
        legacy: bool,
        /// A pruning commit also leaves the file with no servable version yet: the metadata after it says
        /// `download_version: null` (review of #138, P2).
        null_manifest_on_commit: bool,
        /// What the next chunk requests get instead of their chunk, one each, in order.
        faults: std::collections::VecDeque<Fault>,
        /// The metadata's top-level `size_bytes`, when it must differ from the version's (a replace running).
        size_bytes_override: Option<usize>,
        seen: Vec<String>,
    }

    #[derive(Clone, Copy)]
    enum Fault {
        /// The server's real 429 (`rate_limit::too_many_requests`) with this Retry-After.
        RateLimited(u64),
        Status(u16),
    }

    struct Server {
        base: String,
        state: Arc<Mutex<State>>,
    }

    impl Server {
        fn start(versions: Vec<Version>, commits: Vec<(usize, usize, bool)>) -> Self {
            let state = Arc::new(Mutex::new(State {
                versions,
                current: 0,
                commits,
                chunk_requests: 0,
                ignores_pin: false,
                legacy: false,
                null_manifest_on_commit: false,
                faults: std::collections::VecDeque::new(),
                size_bytes_override: None,
                seen: Vec::new(),
            }));
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let shared = state.clone();
            thread::spawn(move || {
                for stream in listener.incoming() {
                    let Ok(mut stream) = stream else { continue };
                    let state = shared.clone();
                    thread::spawn(move || {
                        let Some(request) = read_http_request_or_hangup(&mut stream) else {
                            return;
                        };
                        let reply = answer(&mut state.lock().unwrap(), &request.method, &request.path);
                        let _ = stream.write_all(&reply);
                    });
                }
            });
            Server { base, state }
        }

        fn seen(&self) -> Vec<String> {
            self.state.lock().unwrap().seen.clone()
        }
    }

    fn bytes_reply(body: &[u8]) -> Vec<u8> {
        let mut out = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .into_bytes();
        out.extend_from_slice(body);
        out
    }

    fn answer(s: &mut State, method: &str, target: &str) -> Vec<u8> {
        s.seen.push(format!("{method} {target}"));
        let file = format!("/api/v1/files/{TEST_FILE_ID}");
        let (path, query) = target.split_once('?').unwrap_or((target, ""));
        let pin = query.split('&').find_map(|kv| kv.strip_prefix("object_version_id="));
        if method == "GET" && path == file {
            let v = &s.versions[s.current];
            // The server numbers its versions: the current one is the `current + 1`th (task 2190, F1).
            let mut manifest = v.manifest();
            manifest["version_number"] = json!(s.current + 1);
            return http_json(
                    "200 OK",
                    json!({
                        "id": TEST_FILE_ID, "size_bytes": s.size_bytes_override.unwrap_or(v.plain.len()), "chunk_count": v.frames.len(),
                        "chunk_size_bytes": v.stride,
                        "name_encrypted": enc_name(&KEY, TEST_FILE_ID, "version.bin"),
                        "download_version": if s.legacy { serde_json::Value::Null } else { manifest },
                    }),
                )
                .into_bytes();
        }
        if let Some(index) = path
            .strip_prefix(&format!("{file}/chunks/"))
            .and_then(|i| i.parse::<usize>().ok())
        {
            match s.faults.pop_front() {
                Some(Fault::RateLimited(secs)) => {
                    let body = json!({
                        "error": "rate_limited",
                        "message": format!("Too many requests. Try again in {secs} seconds."),
                        "retry_after": secs,
                    })
                    .to_string();
                    return format!(
                            "HTTP/1.1 429 Too Many Requests\r\nContent-Type: application/json\r\nRetry-After: {secs}\r\nX-RateLimit-Limit: 600\r\nX-RateLimit-Remaining: 0\r\nX-RateLimit-Reset: {secs}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        )
                        .into_bytes();
                }
                Some(Fault::Status(status)) => {
                    return http_json(&format!("{status} Scripted"), json!({"error": "scripted"})).into_bytes();
                }
                None => {}
            }
            let served = match pin {
                Some(id) if !s.ignores_pin => s.versions.iter().position(|v| v.id == id && v.retained),
                _ => Some(s.current),
            };
            let reply = match served {
                Some(v) => bytes_reply(&s.versions[v].frames[index]),
                None => http_json("409 Conflict", json!({"error": GONE})).into_bytes(),
            };
            // The chunk is answered; now the next version may commit (the pause).
            s.chunk_requests += 1;
            let done = s.chunk_requests;
            for &(after, next, prune) in &s.commits.clone() {
                if after == done {
                    if prune {
                        let old = s.current;
                        s.versions[old].retained = false;
                        if s.null_manifest_on_commit {
                            s.legacy = true;
                        }
                    }
                    s.current = next;
                }
            }
            return reply;
        }
        // Anything else (a newer client's discovery): this server has no such route.
        http_json("404 Not Found", json!({"error": "not found"})).into_bytes()
    }

    fn sha(bytes: &[u8]) -> String {
        sha2::Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
    }

    /// The final bytes are exactly one version's, never a mix.
    fn one_version<'a>(got: &[u8], versions: &[&'a Version]) -> &'a str {
        versions
            .iter()
            .find(|v| v.plain == got)
            .map(|v| v.id)
            .unwrap_or_else(|| {
                panic!(
                    "MIXED BYTES: sha256 {} is none of {:?}",
                    sha(got),
                    versions.iter().map(|v| (v.id, sha(&v.plain))).collect::<Vec<_>>()
                )
            })
    }

    fn three_versions() -> (Vec<Version>, [Vec<u8>; 3]) {
        let len = 3 * STRIDE + 77;
        let a = Version::new("aaaaaaaa-0000-4000-8000-00000000000a", content(0xA1, len));
        let b = Version::new("bbbbbbbb-0000-4000-8000-00000000000b", content(0xB1, len));
        let c = Version::new("cccccccc-0000-4000-8000-00000000000c", content(0xC1, len));
        let plains = [a.plain.clone(), b.plain.clone(), c.plain.clone()];
        (vec![a, b, c], plains)
    }

    fn hydrate(server: &Server, dir: &Path) -> (anyhow::Result<()>, EngineBridge, PathBuf) {
        let bridge = test_bridge_with_api(&dir.join("state.db"), server.base.clone(), KEY);
        seed_bridge_row(&bridge, TEST_FILE_ID, "/pinned.bin", None, FileStatus::CloudOnly, 0);
        let dest = dir.join("pinned.bin");
        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async { bridge.hydrate_file(TEST_FILE_ID, &dest, &[dir]).await });
        (result, bridge, dest)
    }

    #[test]
    fn a_replace_committed_between_chunk_requests_never_mixes_versions() {
        let dir = tempfile::tempdir().unwrap();
        let (versions, [a, b, _]) = three_versions();
        // B commits after the first chunk is answered; A stays retained.
        let server = Server::start(versions, vec![(1, 1, false)]);
        let (result, bridge, dest) = hydrate(&server, dir.path());
        result.unwrap();
        let got = std::fs::read(&dest).unwrap();
        let (va, vb) = (
            Version::new("aaaaaaaa-0000-4000-8000-00000000000a", a),
            Version::new("bbbbbbbb-0000-4000-8000-00000000000b", b),
        );
        assert_eq!(one_version(&got, &[&va, &vb]), va.id, "the version the metadata named");
        let pinned: Vec<String> = server.seen().into_iter().filter(|r| r.contains("/chunks/")).collect();
        assert!(
            pinned
                .iter()
                .all(|r| r.ends_with(&format!("?object_version_id={}", va.id))),
            "every chunk pinned: {pinned:?}"
        );
        assert_eq!(
            bridge.db.get_file(TEST_FILE_ID).unwrap().unwrap().status,
            FileStatus::Local
        );
    }

    #[test]
    fn a_pruned_version_restarts_from_fresh_metadata_and_hydrates_the_new_one() {
        let dir = tempfile::tempdir().unwrap();
        let (versions, [a, b, _]) = three_versions();
        // B commits after the first chunk and A is gone at once: the next pinned request is a 409.
        let server = Server::start(versions, vec![(1, 1, true)]);
        let (result, _bridge, dest) = hydrate(&server, dir.path());
        result.unwrap();
        let got = std::fs::read(&dest).unwrap();
        let (va, vb) = (
            Version::new("aaaaaaaa-0000-4000-8000-00000000000a", a),
            Version::new("bbbbbbbb-0000-4000-8000-00000000000b", b),
        );
        assert_eq!(one_version(&got, &[&va, &vb]), vb.id, "the new version, whole");
        let metadata = server
            .seen()
            .iter()
            .filter(|r| r.as_str() == format!("GET /api/v1/files/{TEST_FILE_ID}"))
            .count();
        assert_eq!(metadata, 2, "fresh metadata once");
    }

    #[test]
    fn two_replacements_in_a_row_fail_cleanly_and_write_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let (versions, _) = three_versions();
        // B commits (A pruned) after chunk request 1; C commits (B pruned) after request 3, the
        // first chunk of the restarted download.
        let server = Server::start(versions, vec![(1, 1, true), (3, 2, true)]);
        let (result, bridge, dest) = hydrate(&server, dir.path());
        assert!(
            result.is_err(),
            "a second replacement is an error, not a third attempt or a mix"
        );
        assert!(!dest.exists(), "nothing is written");
        assert_eq!(
            bridge.db.get_file(TEST_FILE_ID).unwrap().unwrap().status,
            FileStatus::Error,
            "never marked in sync"
        );
    }

    #[test]
    fn a_chunk_whose_size_is_not_the_manifests_is_refused_at_that_chunk() {
        let dir = tempfile::tempdir().unwrap();
        let len = 3 * STRIDE + 77;
        let a = Version::new("aaaaaaaa-0000-4000-8000-00000000000a", content(0xA1, len));
        // B has the same size in another stride; a server that ignored the pin would hand out B's
        // chunks, and only their sizes give them away.
        let b = Version::with_stride("bbbbbbbb-0000-4000-8000-00000000000b", content(0xB1, len), 1000);
        let server = Server::start(vec![a, b], vec![(1, 1, false)]);
        server.state.lock().unwrap().ignores_pin = true;
        let (result, bridge, dest) = hydrate(&server, dir.path());
        let err = format!("{:#}", result.expect_err("a chunk of another size is refused"));
        assert!(
            err.contains("chunk 1 through the API is not the length its version's manifest states"),
            "refused at the chunk, by the chunk's size: {err}"
        );
        let chunks = server.seen().iter().filter(|r| r.contains("/chunks/")).count();
        assert_eq!(chunks, 2, "nothing after the wrong chunk was fetched");
        assert!(!dest.exists(), "nothing is written");
        assert_eq!(
            bridge.db.get_file(TEST_FILE_ID).unwrap().unwrap().status,
            FileStatus::Error
        );
    }

    #[test]
    fn a_chunk_that_decrypts_to_another_length_than_the_manifest_says_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let a = Version::new("aaaaaaaa-0000-4000-8000-00000000000a", content(0xA1, 3 * STRIDE + 200))
            .with_json_last_chunk(b"x");
        let server = Server::start(vec![a], vec![]);
        let (result, bridge, dest) = hydrate(&server, dir.path());
        let err = format!("{:#}", result.expect_err("a plaintext of another size is refused"));
        assert!(
            err.contains("the download is 3073 bytes, the pinned version's manifest says 3272"),
            "{err}"
        );
        assert!(!dest.exists(), "nothing is written");
        assert_eq!(
            bridge.db.get_file(TEST_FILE_ID).unwrap().unwrap().status,
            FileStatus::Error
        );
    }

    #[test]
    fn a_range_read_of_a_chunk_that_decrypts_to_another_length_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let a = Version::new("aaaaaaaa-0000-4000-8000-00000000000a", content(0xA1, 3 * STRIDE + 200))
            .with_json_last_chunk(b"x");
        let server = Server::start(vec![a], vec![]);
        let err = format!(
            "{:#}",
            read_range(&server, dir.path(), 3 * STRIDE as u64, 200).expect_err("refused")
        );
        assert!(err.contains("the range read is 1 bytes"), "{err}");
    }

    #[test]
    fn a_file_without_a_download_version_is_downloaded_unpinned_as_before() {
        let dir = tempfile::tempdir().unwrap();
        let (versions, [a, ..]) = three_versions();
        let server = Server::start(versions, vec![]);
        server.state.lock().unwrap().legacy = true;
        let (result, _bridge, dest) = hydrate(&server, dir.path());
        result.unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), a);
        let chunks: Vec<String> = server.seen().into_iter().filter(|r| r.contains("/chunks/")).collect();
        assert_eq!(chunks.len(), 4);
        assert!(
            chunks.iter().all(|r| !r.contains('?')),
            "no pin is invented: {chunks:?}"
        );
    }

    // ── review of #138 ──

    fn no_unpinned_chunk_request(server: &Server) {
        let unpinned: Vec<String> = server
            .seen()
            .into_iter()
            .filter(|r| r.contains("/chunks/") && !r.contains("?object_version_id="))
            .collect();
        assert!(unpinned.is_empty(), "never a chunk without its pin: {unpinned:?}");
    }

    #[test]
    fn a_restart_whose_fresh_metadata_names_no_version_is_an_error_never_an_unpinned_download() {
        let dir = tempfile::tempdir().unwrap();
        let (versions, _) = three_versions();
        let server = Server::start(versions, vec![(1, 1, true)]);
        server.state.lock().unwrap().null_manifest_on_commit = true;
        let (result, bridge, dest) = hydrate(&server, dir.path());
        let err = result.expect_err("an error, not an unpinned second attempt");
        assert!(err.downcast_ref::<crate::download_pin::PinLost>().is_some(), "{err:#}");
        assert!(!dest.exists(), "nothing is written");
        assert_eq!(
            bridge.db.get_file(TEST_FILE_ID).unwrap().unwrap().status,
            FileStatus::Error
        );
        no_unpinned_chunk_request(&server);
    }

    #[test]
    fn a_range_read_whose_restart_finds_no_version_errs_and_leaves_the_row_in_error() {
        let dir = tempfile::tempdir().unwrap();
        let (versions, _) = three_versions();
        let server = Server::start(versions, vec![(1, 1, true)]);
        server.state.lock().unwrap().null_manifest_on_commit = true;
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base.clone(), KEY);
        seed_bridge_row(&bridge, TEST_FILE_ID, "/pinned.bin", None, FileStatus::CloudOnly, 0);
        let read = tokio::runtime::Runtime::new().unwrap().block_on(async {
            bridge
                .hydrate_file_range(TEST_FILE_ID, STRIDE as u64, 2 * STRIDE as u64)
                .await
        });
        let err = read.err().expect("an error");
        assert!(err.downcast_ref::<crate::download_pin::PinLost>().is_some(), "{err:#}");
        assert_eq!(
            bridge.db.get_file(TEST_FILE_ID).unwrap().unwrap().status,
            FileStatus::Error,
            "never left on Downloading"
        );
        no_unpinned_chunk_request(&server);
    }

    #[test]
    fn a_conflict_preview_whose_restart_finds_no_version_shows_no_text_and_fetches_nothing_unpinned() {
        let dir = tempfile::tempdir().unwrap();
        let (versions, _) = three_versions();
        let server = Server::start(versions, vec![(1, 1, true)]);
        server.state.lock().unwrap().null_manifest_on_commit = true;
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base.clone(), KEY);
        let (side, _) = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(bridge.remote_content_preview(TEST_FILE_ID, ContentKind::Text));
        assert_eq!(side.text, None);
        assert!(
            side.unavailable_reason
                .as_deref()
                .is_some_and(|r| r.contains("cannot be downloaded yet")),
            "{side:?}"
        );
        no_unpinned_chunk_request(&server);
    }

    /// Task 2190 (review-169 F1): a conflict preview names the version of the bytes it returns, from the
    /// metadata its download ran on. After a restart that is the fresh metadata's version, never the first's.
    #[test]
    fn a_conflict_preview_names_the_version_of_its_bytes_after_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let text = |name: &str| format!("remote version {name} ").repeat(200);
        let versions = |b: bool| {
            let mut list = vec![Version::new(
                "aaaaaaaa-0000-4000-8000-00000000000a",
                text("A").into_bytes(),
            )];
            if b {
                list.push(Version::new(
                    "bbbbbbbb-0000-4000-8000-00000000000b",
                    text("B").into_bytes(),
                ));
            }
            list
        };
        // B commits after the first chunk and A is gone at once: the next pinned chunk is a 409, and the
        // preview restarts on fresh metadata naming B.
        let server = Server::start(versions(true), vec![(1, 1, true)]);
        let bridge = test_bridge_with_api(&dir.path().join("restart.db"), server.base.clone(), KEY);
        let (side, version) = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(bridge.remote_content_preview(TEST_FILE_ID, ContentKind::Text));
        assert_eq!(side.text, Some(text("B")), "the new version's bytes, whole");
        assert_eq!(version, Some(2), "and its version, from the fresh metadata");
        assert_eq!(
            server
                .seen()
                .iter()
                .filter(|r| r.as_str() == format!("GET /api/v1/files/{TEST_FILE_ID}"))
                .count(),
            2,
            "precondition: it restarted"
        );
        // No restart: the first metadata's version.
        let server = Server::start(versions(false), vec![]);
        let bridge = test_bridge_with_api(&dir.path().join("plain.db"), server.base.clone(), KEY);
        let (side, version) = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(bridge.remote_content_preview(TEST_FILE_ID, ContentKind::Text));
        assert_eq!((side.text, version), (Some(text("A")), Some(1)));
    }

    #[test]
    fn the_pinned_api_path_rides_out_the_servers_own_429() {
        let dir = tempfile::tempdir().unwrap();
        let (versions, [a, ..]) = three_versions();
        let server = Server::start(versions, vec![]);
        server.state.lock().unwrap().faults.push_back(Fault::RateLimited(1));
        let (result, _bridge, dest) = hydrate(&server, dir.path());
        result.expect("the 429 is waited out, not a failed download");
        assert_eq!(std::fs::read(&dest).unwrap(), a);
        assert_eq!(
            server.seen().iter().filter(|r| r.contains("/chunks/")).count(),
            5,
            "4 chunks and one retry"
        );
    }

    #[test]
    fn a_pinned_api_failure_names_no_url() {
        let dir = tempfile::tempdir().unwrap();
        let (versions, _) = three_versions();
        let server = Server::start(versions, vec![]);
        server.state.lock().unwrap().faults.push_back(Fault::Status(400));
        let (result, _bridge, _dest) = hydrate(&server, dir.path());
        let err = format!("{:#}", result.expect_err("a refused chunk"));
        assert!(
            !err.contains("object_version_id") && !err.contains("http://"),
            "no URL in the error: {err}"
        );
        assert!(err.contains("chunk 0"), "{err}");
    }

    #[test]
    fn a_conflict_preview_checks_the_size_of_the_version_it_would_download() {
        let dir = tempfile::tempdir().unwrap();
        // The pinned version is over the preview limit; the top-level size says it is tiny (a replace running).
        let big = Version::new(
            "aaaaaaaa-0000-4000-8000-00000000000a",
            content(0xA1, CONFLICT_PREVIEW_TEXT_MAX_BYTES + 1),
        );
        let server = Server::start(vec![big], vec![]);
        server.state.lock().unwrap().size_bytes_override = Some(100);
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base.clone(), KEY);
        let (side, _) = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(bridge.remote_content_preview(TEST_FILE_ID, ContentKind::Text));
        assert_eq!(side.size_bytes, Some(CONFLICT_PREVIEW_TEXT_MAX_BYTES as u64 + 1));
        assert!(
            side.unavailable_reason
                .as_deref()
                .is_some_and(|r| r.starts_with("File is too large to preview")),
            "{side:?}"
        );
        assert_eq!(
            server.seen().iter().filter(|r| r.contains("/chunks/")).count(),
            0,
            "nothing downloaded"
        );
    }

    fn read_range(server: &Server, dir: &Path, offset: u64, length: u64) -> anyhow::Result<Vec<u8>> {
        let bridge = test_bridge_with_api(&dir.join("state.db"), server.base.clone(), KEY);
        seed_bridge_row(&bridge, TEST_FILE_ID, "/pinned.bin", None, FileStatus::CloudOnly, 0);
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async { bridge.hydrate_file_range(TEST_FILE_ID, offset, length).await })
            .map(|r| r.expect("a planned range").data.to_vec())
    }

    #[test]
    fn a_range_read_across_a_replace_reads_one_version() {
        let dir = tempfile::tempdir().unwrap();
        let (versions, [a, b, _]) = three_versions();
        // Chunks 1 and 2; B commits after chunk 1 is answered.
        let server = Server::start(versions, vec![(1, 1, false)]);
        let got = read_range(&server, dir.path(), STRIDE as u64, 2 * STRIDE as u64).unwrap();
        let span = STRIDE..3 * STRIDE;
        assert!(
            got == a[span.clone()] || got == b[span.clone()],
            "MIXED range: sha256 {}",
            sha(&got)
        );
        assert_eq!(got, a[span].to_vec(), "the version the metadata named");
    }

    #[test]
    fn a_range_read_whose_version_is_pruned_restarts_and_reads_the_new_one() {
        let dir = tempfile::tempdir().unwrap();
        let (versions, [a, b, _]) = three_versions();
        let server = Server::start(versions, vec![(1, 1, true)]);
        let got = read_range(&server, dir.path(), STRIDE as u64, 2 * STRIDE as u64).unwrap();
        let span = STRIDE..3 * STRIDE;
        assert!(
            got == a[span.clone()] || got == b[span.clone()],
            "MIXED range: sha256 {}",
            sha(&got)
        );
        assert_eq!(got, b[span].to_vec(), "the new version, after one restart");
    }
}

// ── Task 1640: an edit of a synced sync-folder file is sent on its base and never overwritten ──
//
// The engine as Windows (and Linux) runs it: no write id, the upload driver's
// `dispatch_local_create` is the trigger (a close, or the scan). Before the fix an edit of a
// tracked file was never queued, and the next hydrate wrote the server's bytes over it.
