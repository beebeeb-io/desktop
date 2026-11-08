//! `uploads` engine tests, split out of `engine_bridge.rs`.

use super::*;
    pub(super) fn read_http_request(stream: &mut std::net::TcpStream) -> RecordedRequest {
        let mut buffer = Vec::new();
        let mut temp = [0u8; 4096];
        let header_end;
        loop {
            let read = std::io::Read::read(stream, &mut temp).unwrap();
            assert!(read > 0, "mock server connection closed before headers");
            buffer.extend_from_slice(&temp[..read]);
            if let Some(pos) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
                header_end = pos + 4;
                break;
            }
        }

        let headers = String::from_utf8_lossy(&buffer[..header_end]);
        let mut lines = headers.lines();
        let request_line = lines.next().unwrap();
        let mut request_parts = request_line.split_whitespace();
        let method = request_parts.next().unwrap().to_string();
        let path = request_parts.next().unwrap().to_string();
        let content_length = headers
            .lines()
            .find_map(|line| {
                line.strip_prefix("Content-Length:")
                    .or_else(|| line.strip_prefix("content-length:"))
                    .and_then(|value| value.trim().parse::<usize>().ok())
            })
            .unwrap_or(0);

        let mut body = buffer[header_end..].to_vec();
        while body.len() < content_length {
            let read = std::io::Read::read(stream, &mut temp).unwrap();
            assert!(read > 0, "mock server connection closed before body");
            body.extend_from_slice(&temp[..read]);
        }
        body.truncate(content_length);
        let origin = headers.lines().find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.trim()
                .eq_ignore_ascii_case(beebeeb_types::backup::ORIGIN_HEADER)
                .then(|| value.trim().to_string())
        });

        RecordedRequest { method, path, body, origin }
    }

    pub(super) fn http_json(status: &str, body: serde_json::Value) -> String {
        let body = body.to_string();
        format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    /// LD-0139: [`http_json`] with a `Date` header `ahead_s` seconds ahead of this machine's clock, as a server
    /// answers (the notice check reads the server's clock from it).
    pub(super) fn http_json_dated(status: &str, body: serde_json::Value, ahead_s: i64) -> String {
        let server_now = chrono::Utc::now() + chrono::Duration::seconds(ahead_s);
        let date = server_now.format("%a, %d %b %Y %H:%M:%S GMT");
        http_json(status, body).replacen("\r\n", &format!("\r\nDate: {date}\r\n"), 1)
    }

    /// [`http_json`] for a binary body (`application/octet-stream`).
    pub(super) fn http_bytes(status: &str, body: &[u8]) -> Vec<u8> {
        let mut response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .into_bytes();
        response.extend_from_slice(body);
        response
    }

    pub(super) fn upload_mock_response(request: &RecordedRequest, fail_chunk: bool, fail_thumbnails: bool) -> String {
        match (request.method.as_str(), request.path.as_str()) {
            ("POST", "/api/v1/uploads/init") => {
                // SF-0001 (L13): an honest server answers with the item id the
                // request carried; the client chose it.
                let file_id = serde_json::from_slice::<serde_json::Value>(&request.body)
                    .ok()
                    .and_then(|body| body["file_id"].as_str().map(str::to_string))
                    .unwrap_or_else(|| "server-file-1".to_string());
                http_json(
                    "200 OK",
                    serde_json::json!({
                        "file_id": file_id,
                        "tenant_id": "tenant-1",
                        "object_version_id": "object-init-1",
                        "upload_session_id": "upload-session-1",
                        "chunk_size_bytes": 4 * 1024 * 1024,
                        "chunk_count": 1,
                        "storage_format_version": 1,
                        "storage_pool_id": "pool-1",
                        "region": "local"
                    }),
                )
            }
            ("PATCH", path) if path.starts_with("/api/v1/files/") => {
                http_json("200 OK", serde_json::json!({ "id": path }))
            }
            ("PUT", "/api/v1/uploads/upload-session-1/chunks/0") if fail_chunk => {
                http_json("500 Internal Server Error", serde_json::json!({ "error": "boom" }))
            }
            ("PUT", "/api/v1/uploads/upload-session-1/chunks/0") => http_json(
                "200 OK",
                serde_json::json!({ "index": 0, "size": request.body.len() as i64, "skipped": false }),
            ),
            ("POST", "/api/v1/uploads/upload-session-1/complete") => http_json(
                "200 OK",
                serde_json::json!({
                    "file_id": "server-file-1",
                    "version_number": 1,
                    "current_object_version_id": "object-complete-1",
                    "size_bytes": 19,
                    "mime_type": "text/plain"
                }),
            ),
            ("POST", path) if path.ends_with("/upload/abandon") => {
                http_json("200 OK", serde_json::json!({ "abandoned": true }))
            }
            ("PUT", path) if path.starts_with("/api/v1/files/") && path.contains("/thumbnail") => {
                if fail_thumbnails {
                    http_json(
                        "500 Internal Server Error",
                        serde_json::json!({ "error": "thumbnail boom" }),
                    )
                } else {
                    http_json("200 OK", serde_json::json!({ "message": "thumbnail uploaded" }))
                }
            }
            _ => http_json(
                "404 Not Found",
                serde_json::json!({ "error": format!("unexpected {} {}", request.method, request.path) }),
            ),
        }
    }

    // ── hydrate_file_range (task 1024) ───────────────────────────────────────

    /// `hydrate_file_range` parses `file_id` as a UUID up front, so every
    /// hydration test needs a real one — this is an arbitrary fixed value, not
    /// tied to any real file.
    pub(super) const TEST_FILE_ID: &str = "d6c090b4-69c8-437e-a61b-2023ea99ef89";

    /// Encrypt `plaintext` under `file_key` the same wire format
    /// `content_frame` accepts (`decrypt_chunk_raw`'s
    /// `nonce(12) || ciphertext || tag(16)`), returning the raw bytes a
    /// `download_chunk` response body would carry.
    pub(super) fn encrypt_chunk_wire(file_key: &beebeeb_core::kdf::FileKey, plaintext: &[u8]) -> Vec<u8> {
        let blob = beebeeb_core::encrypt::encrypt_chunk(file_key, plaintext).unwrap();
        let mut wire = blob.nonce.clone();
        wire.extend_from_slice(&blob.ciphertext);
        wire
    }

    /// Minimal mock server for hydration tests: serves `GET /api/v1/files/:id`
    /// (metadata) and `GET /api/v1/files/:id/chunks/:idx` (one encrypted chunk
    /// each) from a fixed plan, so `hydrate_file_range` can be exercised against
    /// real encrypted bytes without a live server.
    pub(super) struct HydrationMockServer {
        pub(super) base_url: String,
        pub(super) handle: thread::JoinHandle<()>,
    }

    impl HydrationMockServer {
        /// `chunks` are PLAINTEXT chunk bodies (already split by the caller to
        /// the desired chunk_size_bytes); `size_bytes`/`chunk_count` reported by
        /// the metadata endpoint are derived from `chunks` so the test plan is
        /// internally consistent (mirrors how `plan_with_cap` lays out chunks:
        /// uniform except the last).
        pub(super) fn start(file_key: beebeeb_core::kdf::FileKey, chunks: Vec<Vec<u8>>, requests: usize) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let base_url = format!("http://{}", listener.local_addr().unwrap());
            let size_bytes: usize = chunks.iter().map(|c| c.len()).sum();
            let chunk_count = chunks.len();
            let chunk_size_bytes = chunks.first().map(|c| c.len()).unwrap_or(0);
            let handle = thread::spawn(move || {
                for _ in 0..requests {
                    let (mut stream, _) = listener.accept().unwrap();
                    let request = read_http_request(&mut stream);
                    let response = hydration_mock_response(
                        &request,
                        &file_key,
                        &chunks,
                        size_bytes,
                        chunk_count,
                        chunk_size_bytes,
                    );
                    match response {
                        MockResponse::Text(body) => stream.write_all(body.as_bytes()).unwrap(),
                        MockResponse::Binary(body) => {
                            let header = format!(
                                "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                                body.len()
                            );
                            stream.write_all(header.as_bytes()).unwrap();
                            stream.write_all(&body).unwrap();
                        }
                    }
                }
            });
            Self { base_url, handle }
        }

        pub(super) fn finish(self) {
            self.handle.join().unwrap();
        }
    }

    pub(super) enum MockResponse {
        Text(String),
        Binary(Vec<u8>),
    }

    pub(super) fn hydration_mock_response(
        request: &RecordedRequest,
        file_key: &beebeeb_core::kdf::FileKey,
        chunks: &[Vec<u8>],
        size_bytes: usize,
        chunk_count: usize,
        chunk_size_bytes: usize,
    ) -> MockResponse {
        if request.method == "GET" && request.path == format!("/api/v1/files/{TEST_FILE_ID}") {
            return MockResponse::Text(http_json(
                "200 OK",
                serde_json::json!({
                    "id": TEST_FILE_ID,
                    "size_bytes": size_bytes,
                    "chunk_count": chunk_count,
                    "chunk_size_bytes": chunk_size_bytes,
                    "name_encrypted": beebeeb_core::encrypt::encrypt_name_for(file_key,"range.bin",None,beebeeb_core::chunk_stream::ExpectedChunkFormat::Unrecorded).unwrap(),
                }),
            ));
        }
        if let Some(idx_str) = request
            .path
            .strip_prefix(&format!("/api/v1/files/{TEST_FILE_ID}/chunks/"))
        {
            let idx: usize = idx_str.parse().expect("chunk index");
            let wire = encrypt_chunk_wire(file_key, &chunks[idx]);
            return MockResponse::Binary(wire);
        }
        MockResponse::Text(http_json(
            "404 Not Found",
            serde_json::json!({ "error": format!("unexpected {} {}", request.method, request.path) }),
        ))
    }

    pub(super) fn hydration_test_key(master_key: [u8; 32], file_id: &str) -> beebeeb_core::kdf::FileKey {
        let mk = beebeeb_core::kdf::MasterKey::from_bytes(master_key);
        beebeeb_core::kdf::derive_file_key(&mk, file_id.as_bytes())
    }

    /// A multi-chunk file: request a range that spans exactly ONE interior
    /// chunk. `hydrate_file_range` must download+decrypt ONLY that chunk — the
    /// mock server accepts exactly 3 requests (the metadata GET + the one
    /// covering chunk GET) and would hang/fail on a third, so this test fails
    /// loudly if the whole file were downloaded instead.
    #[test]
    fn hydrate_file_range_downloads_only_covering_chunk() {
        let dir = tempfile::tempdir().unwrap();
        let master_key = [9u8; 32];
        let file_key = hydration_test_key(master_key, TEST_FILE_ID);

        // 3 chunks of 10 bytes each (uniform, no remainder) — chunk 1 is the
        // interior chunk under test.
        let chunks: Vec<Vec<u8>> = vec![b"AAAAAAAAAA".to_vec(), b"BBBBBBBBBB".to_vec(), b"CCCCCCCCCC".to_vec()];
        // Metadata, authenticated frame zero, and the requested interior frame.
        let server = HydrationMockServer::start(file_key, chunks, 3);

        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_bridge_row(&bridge, TEST_FILE_ID, "/range.bin", None, FileStatus::CloudOnly, 30);

        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async { bridge.hydrate_file_range(TEST_FILE_ID, 10, 10).await })
            .unwrap()
            .expect("server-provided chunk_size_bytes should enable range hydration");

        assert_eq!(result.range_start_bytes, 10);
        assert!(!result.covers_whole_file);
        assert_eq!(&*result.data, b"BBBBBBBBBB");

        server.finish();

        // The row must NOT be marked Local from a single interior-range fetch —
        // only a whole-file covering range is evidence of full hydration.
        let entry = bridge.db.get_file(TEST_FILE_ID).unwrap().unwrap();
        assert_eq!(entry.status, FileStatus::Downloading);
    }

    /// Real desktop chunk planning is fixed-size per tier with only the last
    /// chunk short. For 9 MiB at the desktop tier the layout is 4 MiB + 4 MiB +
    /// 1 MiB, so a range inside the short final chunk must resolve to byte
    /// offset 8 MiB. The old `ceil(size_bytes / chunk_count)` derivation would
    /// incorrectly use 3 MiB and report offset 6 MiB.
    #[test]
    fn hydrate_file_range_uses_server_chunk_size_for_short_last_chunk_boundary() {
        const MIB: usize = 1024 * 1024;

        let dir = tempfile::tempdir().unwrap();
        let master_key = [12u8; 32];
        let file_key = hydration_test_key(master_key, TEST_FILE_ID);

        let mut last = vec![b'C'; MIB];
        last[123..127].copy_from_slice(b"LAST");
        let chunks: Vec<Vec<u8>> = vec![vec![b'A'; 4 * MIB], vec![b'B'; 4 * MIB], last];
        let server = HydrationMockServer::start(file_key, chunks, 3);

        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_bridge_row(
            &bridge,
            TEST_FILE_ID,
            "/short-last.bin",
            None,
            FileStatus::CloudOnly,
            9 * MIB as i64,
        );

        let required_offset = (8 * MIB + 123) as u64;
        let required_length = 4u64;
        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async {
                bridge
                    .hydrate_file_range(TEST_FILE_ID, required_offset, required_length)
                    .await
            })
            .unwrap()
            .expect("server-provided chunk_size_bytes should enable range hydration");

        assert_eq!(result.range_start_bytes, (8 * MIB) as u64);
        assert!(!result.covers_whole_file);
        let relative_start = (required_offset - result.range_start_bytes) as usize;
        let relative_end = relative_start + required_length as usize;
        assert_eq!(&result.data[relative_start..relative_end], b"LAST");

        server.finish();
    }

    #[test]
    fn hydration_range_plan_uses_authoritative_chunk_size_for_short_last_chunk() {
        const MIB: u64 = 1024 * 1024;

        let plan = plan_hydration_chunk_range(9 * MIB, 3, 4 * MIB, 8 * MIB + 123, 4)
            .unwrap()
            .expect("valid server chunk size should produce a range plan");

        assert_eq!(plan.first_chunk, 2);
        assert_eq!(plan.last_chunk_exclusive, 3);
        assert_eq!(plan.range_start_bytes, 8 * MIB);
        assert_eq!(plan.covering_span_hint, 4 * MIB as usize);
        assert!(!plan.covers_whole_file);
        assert_ne!(
            plan.range_start_bytes,
            2 * (9 * MIB).div_ceil(3),
            "regression guard: ceil(size/chunk_count) would pick the wrong base offset"
        );
    }

    /// A range request that happens to cover the WHOLE file (offset 0, length
    /// >= size) — this is the `covers_whole_file` branch, which mirrors
    /// `hydrate_file_to_memory`'s bookkeeping: mark `Local` + `mark_cached`.
    #[test]
    fn hydrate_file_range_covering_whole_file_marks_local() {
        let dir = tempfile::tempdir().unwrap();
        let master_key = [11u8; 32];
        let file_key = hydration_test_key(master_key, TEST_FILE_ID);

        let chunks: Vec<Vec<u8>> = vec![b"HELLOWORLD".to_vec()]; // 1 chunk, 10 bytes
        let server = HydrationMockServer::start(file_key, chunks, 2);

        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_bridge_row(&bridge, TEST_FILE_ID, "/whole.bin", None, FileStatus::CloudOnly, 10);

        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async { bridge.hydrate_file_range(TEST_FILE_ID, 0, 10).await })
            .unwrap()
            .unwrap();

        assert!(result.covers_whole_file);
        assert_eq!(&*result.data, b"HELLOWORLD");

        server.finish();

        let entry = bridge.db.get_file(TEST_FILE_ID).unwrap().unwrap();
        assert_eq!(entry.status, FileStatus::Local);
    }

