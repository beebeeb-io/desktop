pub(super) fn test_bridge(db_path: &Path) -> EngineBridge {
    let db = Arc::new(StateDb::open(db_path).unwrap());
    let api = Arc::new(ApiClient::new(
        "https://api.beebeeb.io".into(),
        "token".into(),
        [7u8; 32],
    ));
    EngineBridge::new(&crate::identity::REAL, db, api)
}

pub(super) fn test_bridge_with_api(db_path: &Path, base_url: String, master_key: [u8; 32]) -> EngineBridge {
    let db = Arc::new(StateDb::open(db_path).unwrap());
    let api = Arc::new(ApiClient::new(base_url, "token".into(), master_key));
    EngineBridge::new(&crate::identity::REAL, db, api)
}

#[derive(Debug, Clone)]
pub(super) struct RecordedRequest {
    pub(super) method: String,
    pub(super) path: String,
    pub(super) body: Vec<u8>,
    /// LD-0113: the `X-Beebeeb-Origin` header, when the request carried one.
    pub(super) origin: Option<String>,
}

pub(super) struct UploadMockServer {
    pub(super) base_url: String,
    pub(super) requests: Arc<Mutex<Vec<RecordedRequest>>>,
    pub(super) handle: thread::JoinHandle<()>,
    pub(super) shutdown: Arc<AtomicBool>,
}

impl UploadMockServer {
    pub(super) fn start(fail_chunk: bool) -> Self {
        Self::start_inner(fail_chunk, false)
    }

    /// Variant whose `PUT …/thumbnail` responses are 500s — drives the
    /// task-1700 diagnosability test (failed post-complete work must be
    /// visible on the outcome, never silent).
    pub(super) fn start_failing_thumbnail_uploads(fail_chunk: bool) -> Self {
        Self::start_inner(fail_chunk, true)
    }

    pub(super) fn start_inner(fail_chunk: bool, fail_thumbnails: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let server_requests = Arc::clone(&requests);
        let shutdown = Arc::new(AtomicBool::new(false));
        let server_shutdown = Arc::clone(&shutdown);
        let handle = thread::spawn(move || {
            let started = std::time::Instant::now();
            loop {
                // Task 1700: shutdown is owned by `finish()`, not by an
                // idle timer. The old heuristic — stop 300ms after the
                // request count reached a minimum — raced the client's
                // post-complete thumbnail work: every mock response says
                // `Connection: close`, so the thumbnail PUTs need a fresh
                // accept, and under enough runner load the decode + 2×
                // WebP-encode + blurhash + encrypt gap exceeds the window.
                // The listener then dies first, the upload fails with a
                // connection error, the product's warn swallows it (no
                // subscriber in tests), and the test panicked at the
                // thumbnail expect with no trail — exactly the Windows CI
                // flake of task 1700. Recording happens-before the
                // response is written, and the response happens-before
                // the client's next step, so by the time `finish()` runs
                // every recorded request is already in the list.
                if server_shutdown.load(Ordering::SeqCst) {
                    break;
                }
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        // The listener above is non-blocking so this accept
                        // loop can poll it (see the WouldBlock arm below),
                        // but the *accepted* stream must be put back into
                        // blocking mode before handing it to
                        // `read_http_request`, which does a raw, un-retried
                        // `.read().unwrap()`. Without this, under enough
                        // scheduler contention the request bytes can still
                        // be in flight when `read` is called, and a
                        // non-blocking read returns `WouldBlock` instead of
                        // waiting — panicking the mock server's accept
                        // thread. `IpcHydrationMock::start` (below) already
                        // does this; this server predates that fix and was
                        // missing it, causing an intermittent
                        // `Os { code: 35, kind: WouldBlock }` panic under
                        // shared-machine load (confirmed via task 1546's
                        // and task 1538's independent gate runs: 3 distinct
                        // tests using this helper each panicked here under
                        // load and passed 3/3 when rerun in isolation).
                        stream.set_nonblocking(false).unwrap();
                        let request = read_http_request(&mut stream);
                        let response = upload_mock_response(&request, fail_chunk, fail_thumbnails);
                        server_requests.lock().unwrap().push(request);
                        stream.write_all(response.as_bytes()).unwrap();
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        // Hang backstop only; the normal exit is the
                        // shutdown flag above.
                        if started.elapsed() >= Duration::from_secs(30) {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(e) => panic!("upload mock accept failed: {e}"),
                }
            }
        });
        Self {
            base_url,
            requests,
            handle,
            shutdown,
        }
    }

    pub(super) fn finish(self) -> Vec<RecordedRequest> {
        self.shutdown.store(true, Ordering::SeqCst);
        self.handle.join().unwrap();
        Arc::try_unwrap(self.requests).unwrap().into_inner().unwrap()
    }
}

/// Headers-only variant for bodiless GETs that treats a peer hang-up before
/// the headers arrive as `None` instead of a panic (task 1670 issue 3: the
/// cancelled-hydrate test hangs the daemon up while it is connecting, and on
/// a busy CI runner that can land before the request line is sent).
pub(super) fn read_http_request_or_hangup(stream: &mut std::net::TcpStream) -> Option<RecordedRequest> {
    let mut buffer = Vec::new();
    let mut temp = [0u8; 4096];
    loop {
        let read = std::io::Read::read(stream, &mut temp).ok()?;
        if read == 0 {
            return None;
        }
        buffer.extend_from_slice(&temp[..read]);
        if buffer.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    let head = String::from_utf8_lossy(&buffer).to_string();
    let mut parts = head.lines().next()?.split_whitespace();
    let method = parts.next()?.to_string();
    let path = parts.next()?.to_string();
    Some(RecordedRequest {
        method,
        path,
        body: Vec::new(),
        origin: None,
    })
}
