// Behavioral review regressions; the mock uses real HTTP/encryption/SQLite.
struct Round3Server {
    url: String,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
}
impl Round3Server {
    fn start(mut reply: impl FnMut(&RecordedRequest) -> Vec<u8> + Send + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let done = stop.clone();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = requests.clone();
        let worker = thread::spawn(move || {
            while !done.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream.set_nonblocking(false).unwrap();
                        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                        let request = read_http_request(&mut stream);
                        let response = reply(&request);
                        recorded.lock().unwrap().push(request);
                        let _ = stream.write_all(&response);
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => thread::sleep(Duration::from_millis(2)),
                    Err(e) => panic!("accept: {e}"),
                }
            }
        });
        Self {
            url,
            stop,
            worker: Some(worker),
            requests,
        }
    }
}
impl Drop for Round3Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.worker.take().unwrap().join().unwrap();
    }
}
fn round3_binary(bytes: &[u8]) -> Vec<u8> {
    let mut response = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        bytes.len()
    )
    .into_bytes();
    response.extend_from_slice(bytes);
    response
}
fn round3_seed(bridge: &EngineBridge, root: &Path, bytes: &[u8]) {
    seed_bridge_entry(bridge, TEST_FILE_ID, "edit.txt", None, FileStatus::Local, false, 1);
    let mut c = bridge.db.get_file_contract_state(TEST_FILE_ID).unwrap().unwrap();
    c.current_version = 7;
    c.local_base_version = 7;
    c.local_hash = Some(crate::windows_edits::hash_bytes(bytes));
    bridge.db.set_file_contract_state(&c).unwrap();
    std::fs::write(root.join("edit.txt"), bytes).unwrap();
}

#[tokio::test]
async fn regression_1640_r3_download_baseline_survives_remote_update() {
    let dir = tempfile::tempdir().unwrap();
    let db = Arc::new(StateDb::open(dir.path().join("state.db")).unwrap());
    let during = db.clone();
    let key = hydration_test_key([9; 32], TEST_FILE_ID);
    let server = Round3Server::start(move |request| {
        if request.path.ends_with("/chunks/0") {
            // Metadata ingestion advances CURRENT while v7's bytes are in flight.
            let mut c = during.get_file_contract_state(TEST_FILE_ID).unwrap().unwrap();
            c.current_version = 8;
            c.current_object_version_id = Some("remote-eight".into());
            during.set_file_contract_state(&c).unwrap();
            round3_binary(&beebeeb_core::encrypt::encrypt_chunk_raw(&key, b"version seven").unwrap())
        } else {
            http_json(
                "200 OK",
                serde_json::json!({"id":TEST_FILE_ID,"version_number":7,"chunk_count":1,"size_bytes":13}),
            )
            .into_bytes()
        }
    });
    let bridge = EngineBridge::new(
        db,
        Arc::new(ApiClient::new(server.url.clone(), "token".into(), [9; 32])),
    );
    round3_seed(&bridge, dir.path(), b"version seven");
    let bytes = bridge.hydrate_file_to_memory(TEST_FILE_ID).await.unwrap();
    assert_eq!(&*bytes, b"version seven");
    let c = bridge.db.get_file_contract_state(TEST_FILE_ID).unwrap().unwrap();
    assert_eq!(c.current_version, 8);
    assert_eq!(c.local_base_version, 7, "downloaded v7 must never be labelled v8");
    assert_eq!(c.local_hash, Some(crate::windows_edits::hash_bytes(b"version seven")));
    std::fs::write(dir.path().join("edit.txt"), b"edited seven").unwrap();
    assert_eq!(
        bridge
            .queue_windows_tracked_edit(dir.path(), &dir.path().join("edit.txt"))
            .unwrap(),
        Some(true)
    );
    assert_eq!(bridge.db.list_review_operations().unwrap()[0].base_version, Some(7));
}

async fn round3_resolution(choice: &str) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("root");
    std::fs::create_dir(&root).unwrap();
    let db_path = dir.path().join("state.db");
    let key = hydration_test_key([9; 32], TEST_FILE_ID);
    let mut version = 20;
    let server = Round3Server::start(move |request| {
        if request.path.ends_with("/chunks/0") && request.method == "GET" {
            return round3_binary(&beebeeb_core::encrypt::encrypt_chunk_raw(&key, b"remote winner").unwrap());
        }
        if request.path == format!("/api/v1/files/{TEST_FILE_ID}") && request.method == "GET" {
            return http_json(
                "200 OK",
                serde_json::json!({"id":TEST_FILE_ID,"version_number":version,"chunk_count":1,"size_bytes":13}),
            )
            .into_bytes();
        }
        if request.path.ends_with("/uploads/init") {
            let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
            if body["base_version_number"] == 7 {
                return http_json("409 Conflict", serde_json::json!({"error":"remote advanced"})).into_bytes();
            }
            return http_json("200 OK", serde_json::json!({"upload_session_id":"round3-session","file_id":TEST_FILE_ID,"object_version_id":"obj-r3","chunk_size_bytes":4194304,"chunk_count":1,"storage_format_version":2,"tenant_id":"tenant","storage_pool_id":"pool","region":"local"})).into_bytes();
        }
        if request.path.ends_with("/complete") {
            version += 1;
            return http_json(
                "200 OK",
                serde_json::json!({"file_id":TEST_FILE_ID,"version_number":version,"size_bytes":13}),
            )
            .into_bytes();
        }
        http_json(
            "200 OK",
            serde_json::json!({"ok":true,"index":0,"size":request.body.len()}),
        )
        .into_bytes()
    });
    let bridge = test_bridge_with_api(&db_path, server.url.clone(), [9; 32]);
    round3_seed(&bridge, &root, b"baseline");
    for bytes in [b"first save".as_slice(), b"second save"] {
        std::fs::write(root.join("edit.txt"), bytes).unwrap();
        assert_eq!(
            bridge
                .queue_windows_tracked_edit(&root, &root.join("edit.txt"))
                .unwrap(),
            Some(true)
        );
    }
    let ops = bridge.db.list_review_operations().unwrap();
    assert_eq!(ops.len(), 2);
    let conflict = bridge.process_due_operations(&root, i64::MAX / 4).await.unwrap();
    assert_eq!(conflict.paused_op_ids.len(), 1);
    assert_eq!(conflict.completed_op_ids.len(), 0);
    assert_eq!(
        bridge
            .db
            .list_review_operations()
            .unwrap()
            .iter()
            .filter(|op| op.attempts == op.max_attempts)
            .count(),
        1
    );
    match choice {
        "mine" => bridge.resolve_keep_mine(TEST_FILE_ID, &root).await.unwrap(),
        "theirs" => bridge.resolve_keep_theirs(TEST_FILE_ID, &root).await.unwrap(),
        "both" => {
            bridge
                .auto_resolve_keep_both(&root, &bridge.db.get_file(TEST_FILE_ID).unwrap().unwrap())
                .await
                .unwrap();
        }
        _ => unreachable!(),
    }
    assert_eq!(
        bridge.db.list_review_operations().unwrap().len(),
        0,
        "resolved predecessor and successors must retire"
    );
    // Every older settled snapshot remains recoverable, including successor 2.
    let copies: Vec<_> = std::fs::read_dir(&root)
        .unwrap()
        .filter_map(|e| {
            let p = e.unwrap().path();
            if p.is_file() {
                std::fs::read(p).ok()
            } else {
                None
            }
        })
        .collect();
    assert!(copies.contains(&b"first save".to_vec()));
    assert!(copies.contains(&b"second save".to_vec()));
    drop(bridge);
    let bridge = test_bridge_with_api(&db_path, server.url.clone(), [9; 32]);
    std::fs::write(root.join("edit.txt"), b"after restart").unwrap();
    assert_eq!(
        bridge
            .queue_windows_tracked_edit(&root, &root.join("edit.txt"))
            .unwrap(),
        Some(true)
    );
    let queued = bridge.db.list_review_operations().unwrap();
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].base_version, Some(if choice == "mine" { 21 } else { 20 }));
    let result = bridge.process_due_operations(&root, i64::MAX / 2).await.unwrap();
    assert_eq!(
        result.completed_op_ids.len(),
        1,
        "{:?}",
        bridge.db.list_review_operations().unwrap()
    );
    assert_eq!(bridge.db.list_review_operations().unwrap().len(), 0);
    let requests = server.requests.lock().unwrap();
    assert_eq!(
        requests.iter().filter(|r| r.path.ends_with("/complete")).count(),
        if choice == "mine" { 2 } else { 1 }
    );
}
#[tokio::test]
async fn regression_1640_r3_keep_mine_chain_restart_save() {
    round3_resolution("mine").await;
}
#[tokio::test]
async fn regression_1640_r3_keep_theirs_chain_restart_save() {
    round3_resolution("theirs").await;
}
#[tokio::test]
async fn regression_1640_r3_keep_both_chain_restart_save() {
    round3_resolution("both").await;
}

async fn round3_partial(append: bool, native_queue: bool) {
    use std::io::{Seek, SeekFrom, Write};
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("root");
    std::fs::create_dir(&root).unwrap();
    let db_path = dir.path().join("state.db");
    let base = vec![b'b'; 16384];
    let offset = if append { base.len() as u64 } else { 4096 };
    let patch = b"USER-WRITE";
    let eof = if append {
        offset + patch.len() as u64
    } else {
        base.len() as u64
    };
    let mut expected = base.clone();
    expected.resize(eof as usize, 0);
    expected[offset as usize..offset as usize + patch.len()].copy_from_slice(patch);
    let received = Arc::new(Mutex::new(Vec::new()));
    let uploaded = received.clone();
    let offline = Arc::new(AtomicBool::new(true));
    let unavailable = offline.clone();
    let key = hydration_test_key([9; 32], TEST_FILE_ID);
    let server = Round3Server::start(move |request| {
        if unavailable.load(Ordering::SeqCst) {
            return http_json("503 Service Unavailable", serde_json::json!({"error":"offline"})).into_bytes();
        }
        if request.path.ends_with("/versions") {
            return http_json("200 OK", serde_json::json!({"versions":[{"version_number":7,"object_version_id":"base-seven","chunk_count":1,"chunk_size_bytes":16384}]})).into_bytes();
        }
        if request.path.ends_with("/versions/base-seven/download") {
            return round3_binary(&beebeeb_core::encrypt::encrypt_chunk_raw(&key, &base).unwrap());
        }
        if request.method == "GET" {
            // Remote has advanced: missing bytes MUST use immutable v7, not v8.
            return http_json("200 OK", serde_json::json!({"version_number":8,"chunk_count":1})).into_bytes();
        }
        if request.path.ends_with("/init") {
            return http_json("200 OK", serde_json::json!({"file_id":TEST_FILE_ID,"tenant_id":"t","object_version_id":"new","upload_session_id":"p","chunk_size_bytes":4194304,"chunk_count":1,"storage_format_version":2,"storage_pool_id":"pool","region":"local"})).into_bytes();
        }
        if request.method == "PUT" {
            uploaded
                .lock()
                .unwrap()
                .push(beebeeb_core::encrypt::decrypt_chunk_raw(&key, &request.body).unwrap());
        }
        http_json("200 OK", serde_json::json!({"file_id":TEST_FILE_ID,"version_number":9,"size_bytes":eof,"index":0,"size":request.body.len()})).into_bytes()
    });
    let bridge = test_bridge_with_api(&db_path, server.url.clone(), [9; 32]);
    round3_seed(&bridge, &root, b"baseline");
    let path = root.join("edit.txt");
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(&path)
        .unwrap();
    file.set_len(eof).unwrap();
    file.seek(SeekFrom::Start(offset)).unwrap();
    file.write_all(patch).unwrap();
    file.sync_all().unwrap();
    drop(file);
    if native_queue {
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::ffi::OsStrExt;
            use windows::{
                core::PCWSTR,
                Win32::Storage::FileSystem::{SetFileAttributesW, FILE_ATTRIBUTE_OFFLINE},
            };
            let wide: Vec<_> = path.as_os_str().encode_wide().chain(Some(0)).collect();
            unsafe {
                SetFileAttributesW(PCWSTR(wide.as_ptr()), FILE_ATTRIBUTE_OFFLINE).unwrap();
            }
            crate::windows_cf::placeholders::partial_edits::TEST_RANGES
                .with(|v| *v.borrow_mut() = Some((path.clone(), false, vec![])));
            assert_eq!(
                bridge.queue_windows_tracked_edit(&root, &path).unwrap(),
                Some(false),
                "clean partial must stay cloud-only"
            );
            crate::windows_cf::placeholders::partial_edits::TEST_RANGES
                .with(|v| *v.borrow_mut() = Some((path.clone(), true, vec![(offset, patch.len() as u64)])));
            let queued = bridge.queue_windows_tracked_edit(&root, &path).unwrap();
            assert_eq!(
                queued,
                Some(true),
                "dirty partial must be durably queued before hydration"
            );
        }
        #[cfg(not(target_os = "windows"))]
        panic!("native test requires Windows");
    } else {
        let staged = root.join("patch");
        std::fs::write(&staged, patch).unwrap();
        bridge
            .db
            .enqueue_operation(&PendingOperation {
                op_id: "partial-op".into(),
                kind: OperationKind::UploadVersion,
                file_id: Some(TEST_FILE_ID.into()),
                parent_id: None,
                target_path: Some("edit.txt".into()),
                metadata_json: Some(
                    serde_json::json!({"operation":"upload_version","windows_edit":true,
                "name_encrypted":"name","windows_partial":{"base_version":7,"eof":eof,"ranges":[[offset,patch.len()]]}})
                    .to_string(),
                ),
                payload_path: Some(staged.to_string_lossy().into_owned()),
                base_version: Some(7),
                base_object_version_id: None,
                attempts: 0,
                max_attempts: 2147483647,
                next_retry_at: 0,
                last_error: None,
                backup_source_key: None,
                created_at: 0,
                updated_at: 0,
            })
            .unwrap();
    }
    assert_eq!(bridge.db.list_review_operations().unwrap().len(), 1);
    let result = bridge.process_due_operations(&root, i64::MAX / 4).await.unwrap();
    assert_eq!(result.retried_op_ids.len(), 1);
    assert_eq!(bridge.db.list_review_operations().unwrap().len(), 1);
    assert_eq!(received.lock().unwrap().len(), 0);
    drop(bridge);
    offline.store(false, Ordering::SeqCst);
    let bridge = test_bridge_with_api(&db_path, server.url.clone(), [9; 32]);
    let result = bridge.process_due_operations(&root, i64::MAX / 2).await.unwrap();
    assert_eq!(
        result.completed_op_ids.len(),
        1,
        "{:?}",
        bridge.db.list_review_operations().unwrap()
    );
    #[cfg(target_os = "windows")]
    if native_queue {
        use std::os::windows::ffi::OsStrExt;
        use windows::{
            core::PCWSTR,
            Win32::Storage::FileSystem::{SetFileAttributesW, FILE_ATTRIBUTE_NORMAL},
        };
        crate::windows_cf::placeholders::partial_edits::TEST_RANGES.with(|v| *v.borrow_mut() = None);
        let wide: Vec<_> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        unsafe {
            SetFileAttributesW(PCWSTR(wide.as_ptr()), FILE_ATTRIBUTE_NORMAL).unwrap();
        }
        assert_eq!(
            crate::windows_edits::hash_file(&path).unwrap(),
            crate::windows_edits::hash_bytes(&expected),
            "native live materialization must fill missing base ranges"
        );
    }
    let contents = received.lock().unwrap();
    assert_eq!(contents.len(), 1);
    assert_eq!(
        contents[0].len(),
        expected.len(),
        "missing base ranges must be reconstructed"
    );
    assert_eq!(
        crate::windows_edits::hash_bytes(&contents[0]),
        crate::windows_edits::hash_bytes(&expected)
    );
}
#[tokio::test]
async fn regression_1640_r3_partial_range_restart_reconstructs_base() {
    round3_partial(false, false).await;
}
#[tokio::test]
async fn regression_1640_r3_partial_append_restart_reconstructs_base() {
    round3_partial(true, false).await;
}
#[cfg(target_os = "windows")]
#[tokio::test]
async fn regression_1640_r3_native_range_write_queues_dirty_partial() {
    round3_partial(false, true).await;
}
#[cfg(target_os = "windows")]
#[tokio::test]
async fn regression_1640_r3_native_append_queues_dirty_partial() {
    round3_partial(true, true).await;
}

#[tokio::test]
async fn regression_1640_r3_changed_download_never_establishes_baseline() {
    let dir = tempfile::tempdir().unwrap();
    let key = hydration_test_key([9; 32], TEST_FILE_ID);
    let mut reads = 0;
    let server = Round3Server::start(move |request| {
        if request.path.ends_with("/chunks/0") {
            round3_binary(&beebeeb_core::encrypt::encrypt_chunk_raw(&key, b"old bytes").unwrap())
        } else {
            reads += 1;
            http_json(
                "200 OK",
                serde_json::json!({"version_number": if reads == 1 {7} else {8},"chunk_count":1}),
            )
            .into_bytes()
        }
    });
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.url.clone(), [9; 32]);
    round3_seed(&bridge, dir.path(), b"confirmed baseline");
    let result = bridge.hydrate_file_to_memory(TEST_FILE_ID).await;
    assert!(result.is_err(), "mixed/current download must be rejected");
    let c = bridge.db.get_file_contract_state(TEST_FILE_ID).unwrap().unwrap();
    assert_eq!(c.local_base_version, 7);
    assert_eq!(
        c.local_hash,
        Some(crate::windows_edits::hash_bytes(b"confirmed baseline"))
    );
}

#[test]
fn regression_1640_r3_resolution_rebases_later_save_atomically() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("state.db");
    let bridge = test_bridge_with_api(&db_path, "http://127.0.0.1:9".into(), [9;32]);
    round3_seed(&bridge,dir.path(),b"baseline");
    std::fs::write(dir.path().join("edit.txt"), b"captured at resolution").unwrap();
    bridge.queue_windows_tracked_edit(dir.path(),&dir.path().join("edit.txt")).unwrap();
    let captured = bridge.db.list_review_operations().unwrap()[0].op_id.clone();
    std::fs::write(dir.path().join("edit.txt"), b"save while resolution in flight").unwrap();
    bridge.queue_windows_tracked_edit(dir.path(),&dir.path().join("edit.txt")).unwrap();
    bridge.db.record_download_baseline(TEST_FILE_ID,20,"resolved hash").unwrap();
    bridge.db.finish_windows_resolution(TEST_FILE_ID,&[captured]).unwrap();
    drop(bridge);
    let bridge = test_bridge_with_api(&db_path,"http://127.0.0.1:9".into(),[9;32]);
    let ops = bridge.db.list_review_operations().unwrap();
    assert_eq!(ops.len(),1);
    assert_eq!(ops[0].base_version,Some(20));
    assert_eq!(ops[0].attempts,0);
    assert_eq!(std::fs::read(ops[0].payload_path.as_ref().unwrap()).unwrap(),b"save while resolution in flight");
    assert_eq!(bridge.db.get_file(TEST_FILE_ID).unwrap().unwrap().status,FileStatus::Uploading);
}
