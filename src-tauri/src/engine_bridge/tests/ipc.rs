//! `ipc` engine tests, split out of `engine_bridge.rs`.

#[cfg(unix)]
use super::*;
    /// Self-terminating HTTP mock for the IPC end-to-end hydration tests. Unlike
    /// `HydrationMockServer` (blocking accept, fixed request count), this one is
    /// non-blocking with a stop flag + deadline, so the malicious test — where
    /// the destination guard fires BEFORE any HTTP call and the server therefore
    /// sees zero requests — can never hang. It also reports how many requests it
    /// actually served, which is what proves `do_hydrate` never ran on rejection.
    #[cfg(unix)]
    pub(super) struct IpcHydrationMock {
        pub(super) base_url: String,
        pub(super) requests: Arc<std::sync::atomic::AtomicUsize>,
        pub(super) stop: Arc<std::sync::atomic::AtomicBool>,
        pub(super) handle: thread::JoinHandle<()>,
    }

    #[cfg(unix)]
    impl IpcHydrationMock {
        pub(super) fn start(file_key: beebeeb_core::kdf::FileKey, chunks: Vec<Vec<u8>>) -> Self {
            Self::start_with_chunk_delay(file_key, chunks, Duration::ZERO)
        }

        /// Like [`Self::start`], but every chunk response is held back for
        /// `chunk_delay` (task 1670 issue 3: lets a test hang the IPC client
        /// up while the daemon is provably mid-download). With a non-zero delay
        /// a failed write to the (hung-up) daemon is tolerated instead of
        /// panicking the mock thread.
        pub(super) fn start_with_chunk_delay(
            file_key: beebeeb_core::kdf::FileKey,
            chunks: Vec<Vec<u8>>,
            chunk_delay: Duration,
        ) -> Self {
            use std::sync::atomic::{AtomicBool, AtomicUsize};
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let base_url = format!("http://{}", listener.local_addr().unwrap());
            let size_bytes: usize = chunks.iter().map(|c| c.len()).sum();
            let chunk_count = chunks.len();
            let chunk_size_bytes = chunks.first().map(|c| c.len()).unwrap_or(0);
            let requests = Arc::new(AtomicUsize::new(0));
            let stop = Arc::new(AtomicBool::new(false));
            let (rq, st) = (requests.clone(), stop.clone());
            let handle = thread::spawn(move || {
                let started = std::time::Instant::now();
                loop {
                    if st.load(Ordering::Relaxed) || started.elapsed() > Duration::from_secs(15) {
                        break;
                    }
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            stream.set_nonblocking(false).unwrap();
                            let request = if chunk_delay.is_zero() {
                                read_http_request(&mut stream)
                            } else {
                                // Delayed mode is only used by the cancellation test.
                                match read_http_request_or_hangup(&mut stream) {
                                    Some(request) => request,
                                    None => continue,
                                }
                            };
                            rq.fetch_add(1, Ordering::Relaxed);
                            let response = hydration_mock_response(
                                &request,
                                &file_key,
                                &chunks,
                                size_bytes,
                                chunk_count,
                                chunk_size_bytes,
                            );
                            if !chunk_delay.is_zero() && request.path.contains("/chunks/") {
                                std::thread::sleep(chunk_delay);
                            }
                            let written: std::io::Result<()> = match response {
                                MockResponse::Text(body) => stream.write_all(body.as_bytes()),
                                MockResponse::Binary(body) => {
                                    let header = format!(
                                        "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                                        body.len()
                                    );
                                    stream
                                        .write_all(header.as_bytes())
                                        .and_then(|()| stream.write_all(&body))
                                }
                            };
                            if chunk_delay.is_zero() {
                                written.unwrap();
                            }
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Err(e) => panic!("ipc hydration mock accept failed: {e}"),
                    }
                }
            });
            Self {
                base_url,
                requests,
                stop,
                handle,
            }
        }

        pub(super) fn stop_and_count(self) -> usize {
            self.stop.store(true, Ordering::Relaxed);
            self.handle.join().unwrap();
            self.requests.load(Ordering::Relaxed)
        }
    }

    /// Drive one real `HydrateFile` request over a real Unix socket against a
    /// real `serve_ipc_at` server, returning the daemon's response. This is the
    /// actual IPC entry point an attacker would use — not an internal function.
    #[cfg(unix)]
    fn ipc_hydrate_roundtrip(
        db: Arc<StateDb>,
        bridge: Arc<EngineBridge>,
        file_id: &str,
        dest_path: &Path,
    ) -> crate::ipc_socket::IpcResponse {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let sock_dir = tempfile::tempdir().unwrap();
        let sock_path = sock_dir.path().join("ipc.sock");
        let file_id = file_id.to_string();
        let dest_string = dest_path.to_string_lossy().to_string();
        let sp = sock_path.clone();

        tokio::runtime::Runtime::new().unwrap().block_on(async move {
            let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
            let server = tokio::spawn(crate::ipc_socket::serve_ipc_at(sp.clone(), db, bridge, cancel_rx));

            // Wait for the listener to bind, then connect a real client.
            let mut client = {
                let mut attempt = 0;
                loop {
                    match tokio::net::UnixStream::connect(&sp).await {
                        Ok(s) => break s,
                        Err(_) if attempt < 300 => {
                            attempt += 1;
                            tokio::time::sleep(Duration::from_millis(10)).await;
                        }
                        Err(e) => panic!("could not connect to test IPC socket: {e}"),
                    }
                }
            };

            // Exact wire shape the server deserializes off the socket.
            let mut req = serde_json::to_vec(&crate::ipc_socket::IpcRequest::HydrateFile {
                file_id: file_id.clone(),
                dest_path: dest_string.clone(),
                progress: false,
            })
            .unwrap();
            req.push(b'\n');
            client.write_all(&req).await.unwrap();

            // One delimited reply line (task 1670 issue 3 framing).
            let mut reply = Vec::new();
            loop {
                let mut byte = [0u8; 1];
                client.read_exact(&mut byte).await.unwrap();
                if byte[0] == b'\n' {
                    break;
                }
                reply.push(byte[0]);
            }
            let resp: crate::ipc_socket::IpcResponse =
                serde_json::from_slice(&reply).expect("daemon must return a valid IpcResponse");

            drop(client);
            let _ = cancel_tx.send(());
            let _ = server.await;
            resp
        })
    }

    /// Item 3 (load-bearing): a malicious `HydrateFile` whose `dest_path` is
    /// outside every allowed root, sent over the REAL IPC socket, is rejected —
    /// the daemon returns `Error`, the target file is NOT created, and the
    /// hydration backend is never even contacted (0 requests), proving the
    /// destination guard short-circuits before any decrypt/download happens.
    #[cfg(unix)]
    #[test]
    fn ipc_rejects_hydrate_write_outside_allowed_roots() {
        let dir = tempfile::tempdir().unwrap();
        let master_key = [9u8; 32];
        let file_key = hydration_test_key(master_key, TEST_FILE_ID);

        // A working backend, so the ONLY thing that can stop the out-of-root
        // write is the guard. If the guard were removed, do_hydrate would
        // succeed and the file would land on disk — failing this test.
        let server = IpcHydrationMock::start(file_key, vec![b"attacker-would-read-this".to_vec()]);

        let db = Arc::new(StateDb::open(dir.path().join("state.db")).unwrap());
        let api = Arc::new(ApiClient::new(server.base_url.clone(), "token".into(), master_key));
        let bridge = Arc::new(EngineBridge::new(db.clone(), api));
        seed_bridge_row(&bridge, TEST_FILE_ID, "/secret.bin", None, FileStatus::CloudOnly, 24);

        // Destination under a non-existent subdirectory of a real temp dir: it
        // is outside the sync root AND its parent cannot be canonicalized, so it
        // is outside every allowed root the IPC handler passes (sync_root? +
        // temp_dir) regardless of this machine's config.
        let malicious_dest = dir.path().join("attacker-controlled-dir").join("authorized_keys");

        let resp = ipc_hydrate_roundtrip(db, bridge, TEST_FILE_ID, &malicious_dest);

        match resp {
            crate::ipc_socket::IpcResponse::Error { message } => {
                assert!(
                    message.contains("allowed root"),
                    "rejection must come from the destination guard, got: {message}"
                );
            }
            other => panic!("expected the malicious write to be rejected, got {other:?}"),
        }

        assert!(
            !malicious_dest.exists(),
            "the malicious destination file must NOT have been created"
        );
        assert!(
            !malicious_dest.parent().unwrap().exists(),
            "the guard must run before create_dir_all — the parent dir must not exist either"
        );

        let served = server.stop_and_count();
        assert_eq!(
            served, 0,
            "the hydration backend must never be contacted when the destination is rejected"
        );
    }

    /// Item 4: the fix does NOT break the legitimate case. The same real IPC
    /// server, given a `dest_path` genuinely under an allowed root (the temp
    /// dir, always an allowed root in the handler), decrypts the real content
    /// and writes it to disk, returning `Ok`.
    #[cfg(unix)]
    #[test]
    fn ipc_allows_hydrate_write_under_allowed_root() {
        let dir = tempfile::tempdir().unwrap();
        let dest_dir = tempfile::tempdir().unwrap(); // under std::env::temp_dir()
        let master_key = [9u8; 32];
        let file_key = hydration_test_key(master_key, TEST_FILE_ID);

        let plaintext = b"beebeeb-1247-real-decrypted-plaintext".to_vec();
        let server = IpcHydrationMock::start(file_key, vec![plaintext.clone()]);

        let db = Arc::new(StateDb::open(dir.path().join("state.db")).unwrap());
        let api = Arc::new(ApiClient::new(server.base_url.clone(), "token".into(), master_key));
        let bridge = Arc::new(EngineBridge::new(db.clone(), api));
        seed_bridge_row(
            &bridge,
            TEST_FILE_ID,
            "/legit.bin",
            None,
            FileStatus::CloudOnly,
            plaintext.len() as i64,
        );

        // dest_dir is created by tempfile under std::env::temp_dir(), which the
        // IPC handler always includes as an allowed root, so this is accepted.
        let legit_dest = dest_dir.path().join("legit-out.bin");

        let resp = ipc_hydrate_roundtrip(db, bridge, TEST_FILE_ID, &legit_dest);

        assert!(
            matches!(resp, crate::ipc_socket::IpcResponse::Ok {}),
            "a legitimate in-root hydrate must succeed, got {resp:?}"
        );
        assert_eq!(
            std::fs::read(&legit_dest).expect("the decrypted file must exist on disk"),
            plaintext,
            "the real decrypted plaintext must land at the requested path"
        );

        let served = server.stop_and_count();
        assert!(
            served >= 2,
            "the legitimate path must contact the backend for metadata + chunk (got {served})"
        );
    }

    /// Start a real `serve_ipc_at` server on a throwaway socket and hand `body`
    /// a connected client (task 1670 issue 3 hydrate-streaming tests).
    #[cfg(unix)]
    fn with_ipc_client<T>(
        db: Arc<StateDb>,
        bridge: Arc<EngineBridge>,
        body: impl FnOnce(tokio::net::UnixStream) -> std::pin::Pin<Box<dyn std::future::Future<Output = T>>>,
    ) -> T {
        let sock_dir = tempfile::tempdir().unwrap();
        let sp = sock_dir.path().join("ipc.sock");
        tokio::runtime::Runtime::new().unwrap().block_on(async move {
            let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
            let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
            let server = tokio::spawn(crate::ipc_socket::serve_ipc_at_with_ready(
                sp.clone(),
                db,
                bridge,
                cancel_rx,
                Some(ready_tx),
                crate::ipc_socket::WriteContentsPolicy::AnyPath,
            ));
            tokio::time::timeout(Duration::from_secs(5), ready_rx)
                .await
                .expect("IPC readiness timed out")
                .expect("IPC startup failed");
            let client = tokio::net::UnixStream::connect(&sp).await.unwrap();
            let out = body(client).await;
            let _ = cancel_tx.send(());
            let _ = server.await;
            out
        })
    }

    /// Read one `\n`-terminated JSON line, byte at a time, with a deadline.
    #[cfg(unix)]
    async fn read_json_line(client: &mut tokio::net::UnixStream) -> serde_json::Value {
        use tokio::io::AsyncReadExt;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        let mut line = Vec::new();
        loop {
            let mut byte = [0u8; 1];
            let n = tokio::time::timeout_at(deadline, client.read(&mut byte))
                .await
                .expect("no delimited line from the daemon within 10s")
                .unwrap();
            assert!(n > 0, "daemon closed the connection mid-reply");
            if byte[0] == b'\n' {
                return serde_json::from_slice(&line).expect("reply line must be valid JSON");
            }
            line.push(byte[0]);
        }
    }

    #[cfg(unix)]
    fn hydrate_request_line(dest: &Path, progress: bool) -> Vec<u8> {
        let mut req = serde_json::to_vec(&serde_json::json!({
            "HydrateFile": {
                "file_id": TEST_FILE_ID,
                "dest_path": dest.to_string_lossy(),
                "progress": progress,
            }
        }))
        .unwrap();
        req.push(b'\n');
        req
    }

    /// Task 1670 issue 3: a hydrate that opts in streams `HydrateProgress`
    /// frames (initial 0/total, then one per decrypted chunk) BEFORE the single
    /// final `{"Ok":{}}`, and the byte counts are the real plaintext sizes.
    #[cfg(unix)]
    #[test]
    fn ipc_hydrate_streams_progress_frames_before_the_final_reply() {
        use tokio::io::AsyncWriteExt;
        let dir = tempfile::tempdir().unwrap();
        let dest_dir = tempfile::tempdir().unwrap();
        let master_key = [9u8; 32];
        let file_key = hydration_test_key(master_key, TEST_FILE_ID);
        let chunks: Vec<Vec<u8>> = vec![vec![b'a'; 10], vec![b'b'; 10], vec![b'c'; 10]];
        let server = IpcHydrationMock::start(file_key, chunks);
        let db = Arc::new(StateDb::open(dir.path().join("state.db")).unwrap());
        let api = Arc::new(ApiClient::new(server.base_url.clone(), "token".into(), master_key));
        let bridge = Arc::new(EngineBridge::new(db.clone(), api));
        seed_bridge_row(
            &bridge,
            TEST_FILE_ID,
            "/three-chunks.bin",
            None,
            FileStatus::CloudOnly,
            30,
        );
        let dest = dest_dir.path().join("three-chunks.bin");
        let request = hydrate_request_line(&dest, true);

        let lines = with_ipc_client(db, bridge, |mut client| {
            Box::pin(async move {
                client.write_all(&request).await.unwrap();
                let mut lines = Vec::new();
                loop {
                    let line = read_json_line(&mut client).await;
                    let is_progress = line.get("HydrateProgress").is_some();
                    lines.push(line);
                    if !is_progress {
                        return lines;
                    }
                }
            })
        });

        let progress: Vec<(u64, u64)> = lines
            .iter()
            .filter_map(|l| l.get("HydrateProgress"))
            .map(|p| (p["done"].as_u64().unwrap(), p["total"].as_u64().unwrap()))
            .collect();
        assert_eq!(
            progress,
            vec![(0, 30), (10, 30), (20, 30), (30, 30)],
            "progress frames must be the initial 0/total then cumulative plaintext bytes per chunk"
        );
        assert_eq!(
            lines.last().unwrap(),
            &serde_json::json!({"Ok": {}}),
            "the final reply must come AFTER every progress frame"
        );
        assert_eq!(lines.len(), 5, "4 progress frames + 1 final reply, nothing else");
        assert_eq!(std::fs::read(&dest).unwrap().len(), 30);
        server.stop_and_count();
    }

    /// Task 1670 issue 3: a client that does not ask for progress (the 0.8.6
    /// extension) must get exactly ONE reply line and no progress frames — it
    /// reads a single reply and would take the first progress frame for it.
    #[cfg(unix)]
    #[test]
    fn ipc_hydrate_without_opt_in_sends_no_progress_frames() {
        use tokio::io::AsyncWriteExt;
        let dir = tempfile::tempdir().unwrap();
        let dest_dir = tempfile::tempdir().unwrap();
        let master_key = [9u8; 32];
        let file_key = hydration_test_key(master_key, TEST_FILE_ID);
        let chunks: Vec<Vec<u8>> = vec![vec![b'a'; 10], vec![b'b'; 10]];
        let server = IpcHydrationMock::start(file_key, chunks);
        let db = Arc::new(StateDb::open(dir.path().join("state.db")).unwrap());
        let api = Arc::new(ApiClient::new(server.base_url.clone(), "token".into(), master_key));
        let bridge = Arc::new(EngineBridge::new(db.clone(), api));
        seed_bridge_row(
            &bridge,
            TEST_FILE_ID,
            "/two-chunks.bin",
            None,
            FileStatus::CloudOnly,
            20,
        );
        let request = hydrate_request_line(&dest_dir.path().join("two-chunks.bin"), false);

        let (first, quiet) = with_ipc_client(db, bridge, |mut client| {
            Box::pin(async move {
                use tokio::io::AsyncReadExt;
                client.write_all(&request).await.unwrap();
                let first = read_json_line(&mut client).await;
                let mut extra = [0u8; 16];
                let quiet = tokio::time::timeout(Duration::from_millis(300), client.read(&mut extra))
                    .await
                    .is_err();
                (first, quiet)
            })
        });
        assert_eq!(
            first,
            serde_json::json!({"Ok": {}}),
            "the first and only line is the final reply"
        );
        assert!(quiet, "nothing may follow the single reply");
        server.stop_and_count();
    }

    /// Task 1670 issue 3: when the client hangs up mid-download (Finder cancelled
    /// the Progress), the daemon stops downloading, never writes the plaintext,
    /// and puts the row's status back instead of leaving it on `Downloading`.
    #[cfg(unix)]
    #[test]
    fn ipc_hydrate_is_cancelled_and_status_restored_when_the_client_hangs_up() {
        use tokio::io::AsyncWriteExt;
        let dir = tempfile::tempdir().unwrap();
        let dest_dir = tempfile::tempdir().unwrap();
        let master_key = [9u8; 32];
        let file_key = hydration_test_key(master_key, TEST_FILE_ID);
        let chunks: Vec<Vec<u8>> = vec![vec![b'a'; 10], vec![b'b'; 10], vec![b'c'; 10]];
        // 1 s per chunk: a completed hydrate needs ~3 s, a cancelled one is
        // stopped inside the first chunk. Generous margins because CI runs the
        // whole suite in parallel on shared runners.
        let server = IpcHydrationMock::start_with_chunk_delay(file_key, chunks, Duration::from_millis(1000));
        let db = Arc::new(StateDb::open(dir.path().join("state.db")).unwrap());
        let api = Arc::new(ApiClient::new(server.base_url.clone(), "token".into(), master_key));
        let bridge = Arc::new(EngineBridge::new(db.clone(), api));
        seed_bridge_row(&bridge, TEST_FILE_ID, "/slow.bin", None, FileStatus::CloudOnly, 30);
        let dest = dest_dir.path().join("slow.bin");
        let request = hydrate_request_line(&dest, true);
        let db_probe = db.clone();

        let (status_when_first_frame_arrived, status_600ms_after_hangup) =
            with_ipc_client(db.clone(), bridge, |mut client| {
                Box::pin(async move {
                    client.write_all(&request).await.unwrap();
                    let first = read_json_line(&mut client).await;
                    assert_eq!(
                        first["HydrateProgress"]["done"], 0,
                        "first frame is the initial 0/total"
                    );
                    let status = db_probe.get_file(TEST_FILE_ID).unwrap().unwrap().status;
                    drop(client); // Finder cancelled the transfer
                    // Well inside the first chunk's 1 s delay: a daemon that
                    // watches the socket has already dropped the download; one that
                    // only notices at its next write has not.
                    tokio::time::sleep(Duration::from_millis(600)).await;
                    let after_hangup = db_probe.get_file(TEST_FILE_ID).unwrap().unwrap().status;
                    // Then leave time for a (wrongly) uncancelled hydrate to
                    // finish (3 x 1 s).
                    tokio::time::sleep(Duration::from_millis(3000)).await;
                    (status, after_hangup)
                })
            });

        assert_eq!(
            status_when_first_frame_arrived,
            FileStatus::Downloading,
            "the test must hang up while the row is genuinely mid-download"
        );
        assert_eq!(
            status_600ms_after_hangup,
            FileStatus::CloudOnly,
            "the daemon must notice the hang-up promptly (within 600 ms), not at its next progress write"
        );
        assert_eq!(
            db.get_file(TEST_FILE_ID).unwrap().unwrap().status,
            FileStatus::CloudOnly,
            "a cancelled hydrate must restore the row's status, not leave Downloading/Local"
        );
        assert!(!dest.exists(), "a cancelled hydrate must not write plaintext to disk");
        let served = server.stop_and_count();
        assert!(
            served <= 3,
            "the daemon must stop fetching after the hang-up: metadata + discovery (task 2173; this server has \
             no direct route) + at most the in-flight chunk, got {served}"
        );
    }

    /// Item 5: `serve_ipc_at` hardens the bound socket file to owner-only 0o600.
    #[cfg(unix)]
    #[test]
    fn ipc_socket_file_is_chmod_0600_after_bind() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let sock_dir = tempfile::tempdir().unwrap();
        let sock_path = sock_dir.path().join("perm.sock");

        let db = Arc::new(StateDb::open(dir.path().join("state.db")).unwrap());
        let api = Arc::new(ApiClient::new(
            "https://api.beebeeb.io".into(),
            "token".into(),
            [7u8; 32],
        ));
        let bridge = Arc::new(EngineBridge::new(db.clone(), api));

        let sp = sock_path.clone();
        tokio::runtime::Runtime::new().unwrap().block_on(async move {
            let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
            let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
            let server = tokio::spawn(crate::ipc_socket::serve_ipc_at_with_ready(
                sp.clone(),
                db,
                bridge,
                cancel_rx,
                Some(ready_tx),
                crate::ipc_socket::WriteContentsPolicy::AnyPath,
            ));

            let ready = tokio::time::timeout(Duration::from_secs(3), ready_rx)
                .await
                .expect("IPC readiness timed out");
            if ready.is_err() {
                panic!("IPC startup failed before readiness: {:?}", server.await);
            }

            let mode = std::fs::metadata(&sp).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "the socket file must be chmod 0o600, got {mode:o}");

            let _ = cancel_tx.send(());
            server.await.unwrap().unwrap();
            assert!(!sp.exists(), "shutdown must remove the published socket");
        });
    }

    /// Task 1247 P0 follow-up (TOCTOU symlink race): the write primitive itself
    /// must refuse a symlink destination (O_NOFOLLOW) and create real files
    /// owner-only (0o600). This unit test pins both directly on the primitive.
    #[cfg(unix)]
    #[test]
    fn write_hydrated_plaintext_refuses_symlink_and_sets_0600() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();

        // Success path: a real, new destination is written 0o600 with the exact
        // bytes.
        let real_dest = root.path().join("real-out.bin");
        write_hydrated_plaintext(&real_dest, &[root.path()], b"plaintext-payload").unwrap();
        assert_eq!(std::fs::read(&real_dest).unwrap(), b"plaintext-payload");
        let mode = std::fs::metadata(&real_dest).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o600,
            "freshly written plaintext must be owner-only, got {mode:o}"
        );

        // Attack path: destination is a symlink pointing OUTSIDE the root at a
        // not-yet-existing target. O_NOFOLLOW must make the open fail closed so
        // the symlink is never followed and its target is never created/written.
        let planted_target = outside.path().join("authorized_keys");
        let symlink_dest = root.path().join("bb_planted_link");
        std::os::unix::fs::symlink(&planted_target, &symlink_dest).unwrap();
        assert!(
            !planted_target.exists(),
            "precondition: symlink target must not exist yet"
        );

        let err = write_hydrated_plaintext(&symlink_dest, &[root.path()], b"decrypted-secret").unwrap_err();
        assert!(
            !planted_target.exists(),
            "O_NOFOLLOW must not follow the symlink — the outside target must NOT be created"
        );
        // O_NOFOLLOW hitting a symlink final (leaf) component fails with ELOOP.
        assert_eq!(
            err.raw_os_error(),
            Some(libc::ELOOP),
            "expected O_NOFOLLOW symlink refusal (ELOOP), got {err:?}"
        );
    }

    /// Task 1247 second-review Gap 1 (parent-directory symlink swap): the leaf
    /// O_NOFOLLOW is not enough — an attacker can replace a legitimately-contained
    /// PARENT directory with a symlink during the download window. The write must
    /// anchor to the parent via O_DIRECTORY|O_NOFOLLOW + openat so a symlinked
    /// parent is refused. Here the parent symlink points to a real dir INSIDE the
    /// allowed root, so the containment re-check PASSES — meaning ONLY the
    /// parent-anchoring O_NOFOLLOW can stop it (isolates that defense).
    ///
    /// Load-bearing: with the old leaf-only open, the write would follow the
    /// symlinked parent and create the file at the real inside-root dir.
    #[cfg(unix)]
    #[test]
    fn write_hydrated_plaintext_refuses_symlinked_parent_dir() {
        let root = tempfile::tempdir().unwrap();

        // A real directory inside the allowed root, and a symlink to it (also
        // inside the root) used as the destination's parent.
        let real_subdir = root.path().join("real_dir");
        std::fs::create_dir(&real_subdir).unwrap();
        let symlink_parent = root.path().join("swapped_parent");
        std::os::unix::fs::symlink(&real_subdir, &symlink_parent).unwrap();

        let dest = symlink_parent.join("out.bin");
        let would_leak_to = real_subdir.join("out.bin");

        // Sanity: containment passes (canonicalize resolves the symlink to an
        // in-root real dir), so the parent-anchoring defense is what must refuse.
        assert!(
            hydrate_dest_is_allowed(&dest, &[root.path()]),
            "precondition: the symlinked-parent dest must pass containment"
        );

        let err = write_hydrated_plaintext(&dest, &[root.path()], b"decrypted-secret").unwrap_err();
        assert!(
            !would_leak_to.exists(),
            "must NOT write through a symlinked parent directory"
        );
        // O_DIRECTORY|O_NOFOLLOW on a symlinked parent fails closed. Linux reports
        // ENOTDIR (the un-followed symlink is not a directory); a bare leaf
        // O_NOFOLLOW would report ELOOP — accept either as "symlinked-parent refused".
        assert!(
            matches!(err.raw_os_error(), Some(libc::ELOOP) | Some(libc::ENOTDIR)),
            "O_DIRECTORY|O_NOFOLLOW must refuse a symlinked parent (ELOOP/ENOTDIR), got {err:?}"
        );
    }

    /// Task 1247 second-review Gap 2 (mode ignored for pre-existing files): the
    /// O_CREAT `mode` only applies to a NEWLY created inode, so a pre-existing
    /// world-readable file at an in-root path would keep its perms and leak the
    /// plaintext. The unconditional `fchmod` must force 0o600 regardless.
    ///
    /// Load-bearing: without the fchmod (relying on the open-time mode) the
    /// pre-existing 0o644 file keeps 0o644 after the write.
    #[cfg(unix)]
    #[test]
    fn write_hydrated_plaintext_forces_0600_on_preexisting_file() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        // Attacker pre-creates a mundane, world-readable real file at an in-root
        // path (no symlink trick — passes containment as a genuine regular file).
        let dest = root.path().join("preexisting.bin");
        std::fs::write(&dest, b"attacker-placeholder").unwrap();
        std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(
            std::fs::metadata(&dest).unwrap().permissions().mode() & 0o777,
            0o644,
            "precondition: the pre-existing file must start world-readable"
        );

        write_hydrated_plaintext(&dest, &[root.path()], b"decrypted-secret").unwrap();

        assert_eq!(
            std::fs::read(&dest).unwrap(),
            b"decrypted-secret",
            "the decrypted content must be written"
        );
        let mode = std::fs::metadata(&dest).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o600,
            "fchmod must force owner-only perms even on a pre-existing file, got {mode:o}"
        );
    }

    /// Task 1670 round 2: the temp-then-rename publish step must clean up its
    /// own temp file when the final `renameat` fails, so a failed hydration
    /// never leaves a stray `.part` file behind in the hydrate-cache (or any
    /// other) directory. Forced deterministically (no quota tricks, no
    /// threads): pre-create the destination as a DIRECTORY, so the write to
    /// the temp file succeeds but `renameat(file -> existing dir)` always
    /// fails (EISDIR/ENOTDIR) — exactly the failure shape this cleanup exists
    /// for.
    #[cfg(unix)]
    #[test]
    fn write_hydrated_plaintext_removes_temp_file_when_final_rename_fails() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("out.bin");
        std::fs::create_dir(&dest).unwrap();

        let result = write_hydrated_plaintext(&dest, &[dir.path()], b"plaintext-payload");
        assert!(result.is_err(), "a rename onto an existing directory must fail");

        let leftover: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name())
            .collect();
        assert_eq!(
            leftover,
            vec![std::ffi::OsString::from("out.bin")],
            "the temp file must be removed after a failed rename, leaving only the \
             pre-existing dest untouched, got {leftover:?}"
        );
    }

    /// Task 1247 4th review (close the untested `..`-rejection logic): a
    /// `dest_path` whose relative portion contains a non-Normal component (`..`)
    /// must be refused before any openat descent. Not fixing a known bug — just
    /// covering a branch that had zero direct tests across four rounds.
    ///
    /// Load-bearing: `root/legit` exists, so if the non-normal-component
    /// rejection were removed, the descent would resolve `legit/..` back to
    /// `root` and create `root/evil`. The check must stop it first.
    // Exercises the Unix openat descent; the non-Unix writer has no descent.
    #[cfg(unix)]
    #[test]
    fn write_hydrated_plaintext_rejects_dotdot_components() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("legit")).unwrap();

        // root/legit/../evil — the parent (root/legit/..) carries a ParentDir
        // component after strip_prefix, which must be rejected.
        let dest = root.path().join("legit").join("..").join("evil");
        let result = write_hydrated_plaintext(&dest, &[root.path()], b"decrypted-secret");

        assert!(
            result.is_err(),
            "a dest_path with a `..` component must be rejected, got {result:?}"
        );
        assert!(
            !root.path().join("evil").exists(),
            "nothing must be written when a `..` component is rejected"
        );
    }

    /// Task 1247 third review (rename-swap, NOT a symlink): the write must anchor
    /// to the directory INODE that was validated during the openat descent, not
    /// re-resolve the path a second time. This is proven DETERMINISTICALLY (no
    /// thread, no race): open the real subdir's fd, then swap what the `subdir`
    /// NAME points at via remove_dir+rename, then create the leaf via the fd held
    /// from BEFORE the swap — and confirm the write landed in the original inode
    /// (reachable via the held fd) and NOT in the directory the name now points
    /// at. Load-bearing: re-resolving the leaf by the current path string instead
    /// of the held fd makes it land in the swapped-in directory (see the mutation
    /// note in the task file — the red/green swaps the leaf step to a path write).
    #[cfg(unix)]
    #[test]
    fn create_leaf_relative_is_anchored_to_original_dir_fd_across_rename_swap() {
        use std::io::{Read, Write};
        use std::os::unix::io::{AsRawFd, FromRawFd};

        let root = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();

        // A real subdir inside the root; capture its fd via the anchored descent
        // helpers — this fd is the "validated parent", held from before any swap.
        std::fs::create_dir(root.path().join("subdir")).unwrap();
        let root_fd = open_dir_no_follow(root.path()).unwrap();
        let subdir_fd = open_dir_relative_no_follow(root_fd.as_raw_fd(), std::ffi::OsStr::new("subdir")).unwrap();

        // DETERMINISTIC swap (no thread, no race): make the `subdir` NAME resolve
        // to a DIFFERENT real directory. We move the original out of the way
        // (rename, not remove — so its inode stays linked and can still accept a
        // new file via the held fd) and move an attacker-controlled directory
        // into the `subdir` name.
        let attacker_dir = elsewhere.path().join("attacker_dir");
        std::fs::create_dir(&attacker_dir).unwrap();
        std::fs::rename(root.path().join("subdir"), root.path().join("orig_moved")).unwrap();
        std::fs::rename(&attacker_dir, root.path().join("subdir")).unwrap();

        // Create + write the leaf via the fd captured BEFORE the swap.
        let leaf = std::ffi::OsStr::new("out.bin");
        let file_fd = create_leaf_relative(subdir_fd.as_raw_fd(), leaf).unwrap();
        {
            let mut f = std::fs::File::from(file_fd);
            f.write_all(b"anchored-plaintext").unwrap();
        }

        // Reachable + correct via the held (original) dir fd.
        let read_raw = unsafe {
            libc::openat(
                subdir_fd.as_raw_fd(),
                std::ffi::CString::new("out.bin").unwrap().as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        assert!(read_raw >= 0, "leaf must be reachable via the original anchored dir fd");
        let mut via_fd = unsafe { std::fs::File::from(std::os::unix::io::OwnedFd::from_raw_fd(read_raw)) };
        let mut got = Vec::new();
        via_fd.read_to_end(&mut got).unwrap();
        assert_eq!(
            got, b"anchored-plaintext",
            "the write landed in the originally-validated inode"
        );

        // The write followed the fd: it is visible under the ORIGINAL inode's new
        // name (orig_moved) and NOT under the current `subdir` name (which now
        // resolves to the attacker's swapped-in directory).
        assert!(
            root.path().join("orig_moved").join("out.bin").exists(),
            "the write must land in the originally-validated inode (now named orig_moved)"
        );
        assert!(
            !root.path().join("subdir").join("out.bin").exists(),
            "the write must NOT land in the directory the `subdir` name currently resolves to"
        );
    }

