//! LD-0113 Task D3: the backup uploader reads the person's own file in place and never deletes, moves, writes or
//! stages it (spec 2026-11-06 §6.1 point 4, §4.3; plan Task D3, Review Focus 2 and 3).

use super::*;
use crate::state_db::BackupEnqueue;

    const ALICE: (&str, &str) = ("u-a", "a@beebeeb.io");
    const BOB: (&str, &str) = ("u-b", "b@beebeeb.io");

    fn identity(who: (&str, &str)) -> crate::account_binding::Identity {
        crate::account_binding::Identity::new(Some(who.0), Some(who.1))
    }

    /// A bridge for `who`, with its sync root, outside the backed-up folder.
    fn backup_bridge(dir: &Path, base_url: String, who: (&str, &str)) -> (EngineBridge, PathBuf) {
        let bridge = test_bridge_with_api(&dir.join("state.db"), base_url, [31u8; 32]);
        bridge.db.set_owner(&identity(who)).unwrap();
        let sync_root = dir.join("Beebeeb");
        std::fs::create_dir_all(&sync_root).unwrap();
        (bridge, sync_root)
    }

    /// The person's own file in their `Pictures`.
    fn person_file(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
        let pictures = dir.join("Pictures");
        std::fs::create_dir_all(&pictures).unwrap();
        let path = pictures.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    /// What must not change: the bytes (sha-256), the length and the modification time.
    #[derive(Debug, PartialEq, Eq)]
    struct SourceState {
        sha256: String,
        len: u64,
        modified: std::time::SystemTime,
    }

    fn source_state(path: &Path) -> SourceState {
        let meta = std::fs::metadata(path).unwrap_or_else(|e| panic!("the source is gone: {e}"));
        SourceState {
            sha256: sha256_file_hex(path).unwrap(),
            len: meta.len(),
            modified: meta.modified().unwrap(),
        }
    }

    fn pictures_request(source: &Path) -> beebeeb_backup::UploadRequest {
        beebeeb_backup::UploadRequest {
            category: beebeeb_types::backup::Category::Pictures,
            key: source.file_name().unwrap().to_string_lossy().into_owned(),
            source_path: source.to_path_buf(),
            parent_id: uuid::Uuid::parse_str("8d3f1b2a-4c5e-4f60-8a7b-9c0d1e2f3a4b").unwrap(),
            name: source.file_name().unwrap().to_string_lossy().into_owned(),
            fingerprint: beebeeb_backup::fingerprint(source).unwrap(),
            reason: beebeeb_core::backup::UploadReason::New,
        }
    }

    fn enqueue(bridge: &EngineBridge, source: &Path) -> String {
        match bridge.enqueue_backup_upload(&pictures_request(source)).unwrap() {
            BackupEnqueue::Queued(op_id) => op_id,
            other => panic!("not queued: {other:?}"),
        }
    }

    fn backup_ops(bridge: &EngineBridge) -> i64 {
        bridge
            .db
            .hold_lock_for_test()
            .query_row("SELECT COUNT(*) FROM operation_queue WHERE origin = 'backup'", [], |row| row.get(0))
            .unwrap()
    }

    fn count_rows(bridge: &EngineBridge, table: &str) -> i64 {
        bridge
            .db
            .hold_lock_for_test()
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| row.get(0))
            .unwrap()
    }

    /// Spec §6.1 point 4 (the mutant test): a completed backup upload, a cancelled one, a resumed one, a sign-out with
    /// backup ops queued and a turn-off of the folder each leave the source present and byte-identical (hash, length
    /// and mtime before and after). Storing the source in `payload_path` turns it red (PR notes, "The required mutant").
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn backup_upload_leaves_source_byte_identical() {
        let mut checked = Vec::new();

        // 1. Completed: uploaded, recorded, the op gone.
        {
            let dir = tempfile::tempdir().unwrap();
            let source = person_file(dir.path(), "beach.txt", b"the person's own bytes");
            let before = source_state(&source);
            let server = UploadMockServer::start(false);
            let (bridge, sync_root) = backup_bridge(dir.path(), server.base_url.clone(), ALICE);
            let op_id = enqueue(&bridge, &source);
            let outcome = bridge.process_due_operations(&sync_root, 200).await.unwrap();
            assert_eq!(outcome.completed_op_ids, vec![op_id], "the upload completed");
            assert_eq!(bridge.db.backup_records("pictures").unwrap().len(), 1, "and was recorded");
            server.finish();
            assert_eq!(source_state(&source), before, "completed: the source changed");
            checked.push("completed");
        }

        // 2. Cancelled mid-upload (the pass is cut while chunk 1 hangs), then 3. resumed from the watermark.
        {
            let dir = tempfile::tempdir().unwrap();
            let source = person_file(dir.path(), "resume.bin", b"0123456789abcdefghij");
            let before = source_state(&source);
            let server = ResumableUploadMock::start(ResumeMockMode::HangOnce);
            let (bridge, sync_root) = backup_bridge(dir.path(), server.base_url.clone(), ALICE);
            let op_id = enqueue(&bridge, &source);
            let cut = tokio::time::timeout(Duration::from_millis(1500), bridge.process_due_operations(&sync_root, 200)).await;
            assert!(cut.is_err(), "the first pass is cut mid-upload");
            let session = bridge.db.backup_upload_resume(&op_id).unwrap().expect("the session is kept for the retry");
            assert_eq!(session.acked_chunks, 1, "chunk 0 was acknowledged before the cut");
            assert_eq!(source_state(&source), before, "cancelled: the source changed");
            checked.push("cancelled");

            let outcome = bridge.process_due_operations(&sync_root, 200).await.unwrap();
            assert_eq!(outcome.completed_op_ids, vec![op_id.clone()], "the retry completed");
            let requests = server.finish();
            assert_eq!(count_requests(&requests, "POST", "/api/v1/uploads/init"), 1, "resumed, not re-initialised");
            assert_eq!(
                count_requests(&requests, "PUT", "/api/v1/uploads/session-1/chunks/0"),
                1,
                "chunk 0 is not sent again"
            );
            assert_eq!(count_requests(&requests, "POST", "/api/v1/uploads/session-1/complete"), 1);
            assert!(bridge.db.backup_upload_resume(&op_id).unwrap().is_none());
            assert_eq!(source_state(&source), before, "resumed: the source changed");
            checked.push("resumed");
        }

        // 4. Sign-out with backup ops queued, one with an open session: the keep step's read does not list them, and
        //    the sign-out purge (the same function the R10 reset runs) deletes nothing of the source.
        {
            let dir = tempfile::tempdir().unwrap();
            let in_flight = person_file(dir.path(), "in-flight.txt", b"chunk 0 fails, the session stays");
            let waiting = person_file(dir.path(), "waiting.txt", b"queued, never started");
            let before = (source_state(&in_flight), source_state(&waiting));
            let server = UploadMockServer::start(true);
            let (bridge, sync_root) = backup_bridge(dir.path(), server.base_url.clone(), ALICE);
            let in_flight_op = enqueue(&bridge, &in_flight);
            let outcome = bridge.process_due_operations(&sync_root, 200).await.unwrap();
            assert_eq!(outcome.retried_op_ids, vec![in_flight_op.clone()], "chunk 0 failed, so it retries");
            assert!(bridge.db.backup_upload_resume(&in_flight_op).unwrap().is_some(), "with its session kept");
            enqueue(&bridge, &waiting);
            assert_eq!(backup_ops(&bridge), 2, "precondition: two backup ops are queued");
            assert!(
                bridge.db.waiting_rows().unwrap().is_empty(),
                "the keep step and the purge guard never see a backup op"
            );
            crate::local_state_purge::purge_local_state_files_in(&bridge.db, Some(&sync_root), &[], None).unwrap();
            assert_eq!(backup_ops(&bridge), 0, "the sign-out purge drops the queue");
            assert_eq!(count_rows(&bridge, "backup_upload_resume"), 0, "and the session with it");
            server.finish();
            assert_eq!(
                (source_state(&in_flight), source_state(&waiting)),
                before,
                "sign-out: a source changed"
            );
            checked.push("sign-out");
        }

        // 5. Turning the folder off mid-upload: today's purge drops the folder's ops and their sessions.
        {
            let dir = tempfile::tempdir().unwrap();
            let source = person_file(dir.path(), "turn-off.txt", b"the folder is turned off meanwhile");
            let before = source_state(&source);
            let server = UploadMockServer::start(true);
            let (bridge, sync_root) = backup_bridge(dir.path(), server.base_url.clone(), ALICE);
            let op_id = enqueue(&bridge, &source);
            bridge.process_due_operations(&sync_root, 200).await.unwrap();
            assert!(bridge.db.backup_upload_resume(&op_id).unwrap().is_some(), "precondition: a session is open");
            assert_eq!(bridge.db.purge_backup_source_ops("pictures").unwrap(), 1, "the folder's op is purged");
            assert_eq!(backup_ops(&bridge), 0);
            assert_eq!(count_rows(&bridge, "backup_upload_resume"), 0, "with its session");
            server.finish();
            assert_eq!(source_state(&source), before, "turn-off: the source changed");
            checked.push("turn-off");
        }

        assert_eq!(checked, ["completed", "cancelled", "resumed", "sign-out", "turn-off"]);
    }

    /// Serves a 3-chunk plan (8 + 8 + 4 bytes) and, while chunk 1 of the first session is in flight, changes the
    /// source the way another program saving it would: new bytes, the same length, a later mtime.
    struct ChangingSourceMock {
        base_url: String,
        requests: Arc<Mutex<Vec<RecordedRequest>>>,
        stop: Arc<AtomicBool>,
        handle: thread::JoinHandle<()>,
    }

    impl ChangingSourceMock {
        fn start(source: PathBuf) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let base_url = format!("http://{}", listener.local_addr().unwrap());
            let requests = Arc::new(Mutex::new(Vec::new()));
            let stop = Arc::new(AtomicBool::new(false));
            let (server_requests, server_stop) = (Arc::clone(&requests), Arc::clone(&stop));
            let handle = thread::spawn(move || {
                let mut inits = 0;
                let mut changed = false;
                let started = std::time::Instant::now();
                while !server_stop.load(Ordering::SeqCst) && started.elapsed() < Duration::from_secs(20) {
                    let mut stream = match listener.accept() {
                        Ok((stream, _)) => stream,
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(5));
                            continue;
                        }
                        Err(e) => panic!("changing-source mock accept failed: {e}"),
                    };
                    stream.set_nonblocking(false).unwrap();
                    let request = read_http_request(&mut stream);
                    server_requests.lock().unwrap().push(request.clone());
                    let (method, path) = (request.method.as_str(), request.path.as_str());
                    let response = if method == "POST" && path == "/api/v1/uploads/init" {
                        inits += 1;
                        let file_id = serde_json::from_slice::<serde_json::Value>(&request.body).unwrap()["file_id"]
                            .as_str()
                            .unwrap()
                            .to_string();
                        http_json(
                            "201 Created",
                            serde_json::json!({
                                "file_id": file_id, "tenant_id": "tenant-1",
                                "object_version_id": format!("object-init-{inits}"),
                                "upload_session_id": format!("session-{inits}"),
                                "chunk_size_bytes": 8, "chunk_count": 3,
                                "storage_format_version": 1, "storage_pool_id": "pool-1", "region": "local"
                            }),
                        )
                    } else if let Some(index) = (method == "PUT").then(|| path.rsplit_once("/chunks/")).flatten() {
                        let index: u32 = index.1.parse().unwrap();
                        if path == "/api/v1/uploads/session-1/chunks/1" && !changed {
                            changed = true;
                            std::fs::write(&source, b"ABCDEFGHIJKLMNOPQRST").unwrap();
                            let later = std::time::SystemTime::now() + Duration::from_secs(60);
                            std::fs::File::options().write(true).open(&source).unwrap().set_modified(later).unwrap();
                        }
                        http_json(
                            "200 OK",
                            serde_json::json!({ "index": index, "size": request.body.len(), "skipped": false }),
                        )
                    } else if method == "POST" && path.ends_with("/complete") {
                        http_json("200 OK", serde_json::json!({ "version_number": 1, "size_bytes": 20 }))
                    } else if method == "POST" && path.ends_with("/upload/abandon") {
                        http_json("200 OK", serde_json::json!({ "abandoned": true }))
                    } else {
                        http_json("404 Not Found", serde_json::json!({ "error": format!("unexpected {method} {path}") }))
                    };
                    let _ = stream.write_all(response.as_bytes());
                }
            });
            Self { base_url, requests, stop, handle }
        }

        fn finish(self) -> Vec<RecordedRequest> {
            self.stop.store(true, Ordering::SeqCst);
            self.handle.join().unwrap();
            self.requests.lock().unwrap().clone()
        }
    }

    /// Review Focus 2: a file that changes while it uploads never lands torn. The session is abandoned
    /// (`POST …/upload/abandon`) before `complete`, no record is written and the op is dropped; the next pass queues
    /// the file again and uploads the new bytes, recording their fingerprint.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn backup_upload_source_changed_midway_abandons_and_retries() {
        let dir = tempfile::tempdir().unwrap();
        let source = person_file(dir.path(), "edited.txt", b"0123456789abcdefghij");
        let server = ChangingSourceMock::start(source.clone());
        let (bridge, sync_root) = backup_bridge(dir.path(), server.base_url.clone(), ALICE);
        let first_op = enqueue(&bridge, &source);

        let outcome = bridge.process_due_operations(&sync_root, 200).await.unwrap();
        assert_eq!(outcome.retried_op_ids, vec![first_op.clone()], "the changed file is retried");
        assert!(outcome.completed_op_ids.is_empty(), "and never reported as completed");
        assert!(bridge.db.backup_records("pictures").unwrap().is_empty(), "no record of the torn read");
        assert_eq!(backup_ops(&bridge), 0, "the op is dropped; the next pass queues the file again");
        assert!(bridge.db.backup_upload_resume(&first_op).unwrap().is_none());

        // The next pass: it sees no record, so it queues the file again with what it finds now.
        let second_op = enqueue(&bridge, &source);
        let outcome = bridge.process_due_operations(&sync_root, 300).await.unwrap();
        assert_eq!(outcome.completed_op_ids, vec![second_op]);

        let requests = server.finish();
        let inits: Vec<String> = requests
            .iter()
            .filter(|r| r.method == "POST" && r.path == "/api/v1/uploads/init")
            .map(|r| serde_json::from_slice::<serde_json::Value>(&r.body).unwrap()["file_id"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(inits.len(), 2, "one session per pass");
        assert_eq!(
            count_requests(&requests, "POST", &format!("/api/v1/files/{}/upload/abandon", inits[0])),
            1,
            "the torn session is abandoned"
        );
        assert_eq!(
            count_requests(&requests, "POST", "/api/v1/uploads/session-1/complete"),
            0,
            "the torn session is never completed"
        );
        assert_eq!(count_requests(&requests, "POST", "/api/v1/uploads/session-2/complete"), 1);
        assert_eq!(count_requests(&requests, "DELETE", &format!("/api/v1/files/{}", inits[0])), 0, "never trashed");
        let records = bridge.db.backup_records("pictures").unwrap();
        let now = beebeeb_backup::fingerprint(&source).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(
            (records[0].size, records[0].modified_ms, records[0].remote_id.to_string()),
            (now.size, now.modified_ms, inits[1].clone()),
            "the record is of the new bytes, in the second session's file"
        );
        assert_eq!(std::fs::read(&source).unwrap(), b"ABCDEFGHIJKLMNOPQRST", "the other program's save is intact");
    }

    /// Spec §6.1 point 4: a backup op writes its record and nothing else. It never takes the sync-root upload: no
    /// `files` row, no landing, no finalization journal, no staged copy, no session in `upload_resume`, nothing in the
    /// sync root; and the sync uploader refuses a backup claim outright.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn backup_op_never_runs_sync_root_finalization() {
        let dir = tempfile::tempdir().unwrap();
        let source = person_file(dir.path(), "notes.txt", b"backed up, never finalized");
        let server = UploadMockServer::start(false);
        let (bridge, sync_root) = backup_bridge(dir.path(), server.base_url.clone(), ALICE);

        // The sync uploader refuses a backup claim, before any request or row.
        let refused_op = enqueue(&bridge, &source);
        let ClaimOutcome::Claimed(claimed) = bridge.db.claim_operation(&refused_op, 200).unwrap() else {
            panic!("the backup op is claimable");
        };
        assert_eq!(claimed.origin, crate::state_db::OpOrigin::Backup);
        let mut errors = Vec::new();
        let refusal = bridge
            .upload_version(&claimed.op, Some(&claimed), &sync_root, &mut errors)
            .await
            .expect_err("the sync uploader refuses a backup op");
        assert!(refusal.to_string().contains("never takes the sync-root upload path"), "{refusal}");

        // The runner sends it to the backup uploader.
        let landings = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let seen = Arc::clone(&landings);
        bridge.seams.arm("landing:after_complete", move || {
            seen.fetch_add(1, Ordering::SeqCst);
        });
        let outcome = bridge.process_due_operations(&sync_root, 300).await.unwrap();
        assert_eq!(outcome.completed_op_ids, vec![refused_op.clone()]);
        let requests = server.finish();
        let file_id = init_item_id(&requests);

        assert_eq!(landings.load(Ordering::SeqCst), 0, "the sync-root landing never ran");
        assert!(bridge.db.get_file(&file_id).unwrap().is_none(), "no files row");
        assert!(bridge.db.upload_finalizations().unwrap().is_empty(), "no finalization journal row");
        assert!(bridge.db.staged_payloads_for_signout().unwrap().is_empty(), "no staged copy");
        assert!(bridge.db.get_upload_resume(&refused_op).unwrap().is_none(), "no sync session row");
        assert_eq!(count_requests(&requests, "PATCH", &format!("/api/v1/files/{file_id}")), 0, "no metadata step");
        assert_eq!(std::fs::read_dir(&sync_root).unwrap().count(), 0, "nothing was written into the sync root");
        let records = bridge.db.backup_records("pictures").unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].remote_id.to_string(), file_id, "the record names the uploaded file");
        assert_eq!(std::fs::read(&source).unwrap(), b"backed up, never finalized");
    }

    /// Review Focus 3 (R10): an account switch with backups queued never uses the old account's records or ops for
    /// the new one. The R10 reset (the engine start's `bind_before_engine_start`, as production runs it) clears the
    /// records, the ops and their sessions; on Windows the start is refused instead. Even when no reset ran, the new
    /// account reads none of the old one's records and its uploader sends none of the old one's ops.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn account_switch_resets_backup_records_and_ops() {
        // Alice backed up one file and has another queued, its session open.
        async fn alices_backups(dir: &Path, server: &str) -> (EngineBridge, PathBuf, String) {
            let (bridge, sync_root) = backup_bridge(dir, server.to_string(), ALICE);
            let done = person_file(dir, "done.txt", b"backed up for alice");
            let queued = person_file(dir, "queued.txt", b"queued for alice");
            enqueue(&bridge, &done);
            bridge.process_due_operations(&sync_root, 200).await.unwrap();
            assert_eq!(bridge.db.backup_records("pictures").unwrap().len(), 1, "precondition: alice has a record");
            let queued_op = enqueue(&bridge, &queued);
            (bridge, sync_root, queued_op)
        }

        // The R10 reset.
        {
            let dir = tempfile::tempdir().unwrap();
            let server = UploadMockServer::start(false);
            let (bridge, _sync_root, queued_op) = alices_backups(dir.path(), &server.base_url).await;
            bridge
                .db
                .put_backup_upload_resume_claimed(
                    &crate::state_db::BackupUploadResume {
                        op_id: queued_op.clone(),
                        server_file_id: "00000000-0000-4000-8000-000000000001".into(),
                        upload_session_id: "s".into(),
                        object_version_id: "o".into(),
                        chunk_size_bytes: 8,
                        chunk_count: 1,
                        acked_chunks: 0,
                        chunk_format: 1,
                        fingerprint: beebeeb_backup::Fingerprint { size: 1, modified_ms: 1, file_id: None },
                    },
                    &{
                        let ClaimOutcome::Claimed(claimed) = bridge.db.claim_operation(&queued_op, 300).unwrap() else {
                            panic!("claimable");
                        };
                        claimed.claim_id
                    },
                )
                .unwrap();
            assert_eq!(
                (count_rows(&bridge, "backup_records"), backup_ops(&bridge), count_rows(&bridge, "backup_upload_resume")),
                (1, 1, 1),
                "precondition: alice's record, op and session"
            );
            let data = crate::session_reauth::StateDbLocalData {
                db: StateDb::open(dir.path().join("state.db")).unwrap(),
                sync_root: None,
                staging_dirs: Vec::new(),
                kept: Default::default(),
            };
            let bound = crate::account_binding::bind_before_engine_start(
                &data,
                &identity(BOB),
                !cfg!(target_os = "windows"),
            );
            if cfg!(target_os = "windows") {
                assert_eq!(bound, Err(crate::account_binding::OTHER_ACCOUNT_ON_WINDOWS.to_string()));
                assert_eq!(bridge.db.owner().unwrap(), Some(identity(ALICE)), "Windows refuses; nothing starts");
            } else {
                assert_eq!(bound, Ok(crate::account_binding::Bound::Reset));
                assert_eq!(
                    (count_rows(&bridge, "backup_records"), backup_ops(&bridge), count_rows(&bridge, "backup_upload_resume")),
                    (0, 0, 0),
                    "the reset cleared alice's records, ops and sessions"
                );
                assert_eq!(bridge.db.owner().unwrap(), Some(identity(BOB)));
                assert!(bridge.db.backup_records("pictures").unwrap().is_empty());
            }
            server.finish();
        }

        // No reset ran (defence in depth): bob reads none of alice's records, and alice's op is never sent for him.
        {
            let dir = tempfile::tempdir().unwrap();
            let server = UploadMockServer::start(false);
            let (bridge, sync_root, queued_op) = alices_backups(dir.path(), &server.base_url).await;
            let inits_before = count_requests(&server.requests.lock().unwrap(), "POST", "/api/v1/uploads/init");
            bridge.db.set_owner(&identity(BOB)).unwrap();
            assert!(bridge.db.backup_records("pictures").unwrap().is_empty(), "alice's records are not bob's");
            assert_eq!(count_rows(&bridge, "backup_records"), 1, "they are still on disk, unread");
            let outcome = bridge.process_due_operations(&sync_root, 300).await.unwrap();
            assert_eq!(outcome.retried_op_ids, vec![queued_op], "alice's op is dropped, not sent");
            assert_eq!(backup_ops(&bridge), 0);
            let requests = server.finish();
            assert_eq!(
                count_requests(&requests, "POST", "/api/v1/uploads/init"),
                inits_before,
                "no upload was started for bob from alice's queue"
            );
            // And a record is only ever written for the owner the op was queued for.
            bridge.db.set_owner(&identity(ALICE)).unwrap();
            assert_eq!(bridge.db.backup_records("pictures").unwrap().len(), 1, "alice's record is hers again");
        }
    }
