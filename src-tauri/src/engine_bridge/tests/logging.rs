//! `logging` engine tests, split out of `engine_bridge.rs`.

use super::*;
    /// T37: a versionless replace from another device raises the version (T48). The raising
    /// node carries the planted name.
    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn log_version_raised() {
        let master_key = [108u8; 32];
        let (dir, sync_root, server, bridge) = rule3_setup(master_key);
        land_one_save(&bridge, &server, dir.path(), &sync_root, "replaced").await;
        let replace = crate::api_client::SyncOp {
            seq_id: 9,
            op_type: "file_create".into(),
            payload: serde_json::json!({
                "id": "replaced",
                "name_encrypted": enc_name(&master_key, "replaced", PLANTED),
                "parent_id": null,
                "size_bytes": 12
            }),
        };
        apply_sync_op(&bridge, &sync_root, &replace, now_secs(), &mut Vec::new()).unwrap();
        let mid = crate::api_client::SyncSnapshot {
            seq_id: 10,
            nodes: vec![node(&master_key, "replaced", PLANTED, 3, true)],
        };
        apply_snapshot(&bridge, &sync_root, &mid, now_secs(), now_secs(), &mut Vec::new()).unwrap();
        let done = crate::api_client::SyncSnapshot {
            seq_id: 11,
            nodes: vec![node(&master_key, "replaced", PLANTED, 3, false)],
        };
        let logs = capture_logs_async(async {
            apply_snapshot(&bridge, &sync_root, &done, now_secs(), now_secs(), &mut Vec::new()).unwrap();
        })
        .await;
        let line = assert_clean_line(
            &logs,
            "file version raised from the snapshot",
            &["file_id", "old_version", "new_version"],
            dir.path(),
        );
        assert!(
            line.contains("file_id=replaced") && line.contains("old_version=2") && line.contains("new_version=3"),
            "{line}"
        );
        drop(server.finish());
    }

    /// T37: an attempt whose op was purged since its claim says which step found it gone (T42).
    #[tokio::test]
    async fn log_queue_state_moved() {
        let master_key = [109u8; 32];
        let (dir, sync_root, server, bridge) = rule3_setup(master_key);
        server.state.lock().unwrap().delay_init = Some(Duration::from_millis(400));
        server
            .state
            .lock()
            .unwrap()
            .fail_first_chunk_once
            .insert("session-1".into());
        seed_uploaded_row(&bridge, &server, "revoked");
        fp_save(
            &bridge,
            dir.path(),
            "revoked",
            PLANTED,
            b"save under a share that goes away",
            "1",
        );
        let w = bridge.db.list_operations_for_file("revoked").unwrap().remove(0);
        let mut contract = bridge.db.get_file_contract_state("revoked").unwrap().unwrap();
        contract.namespace = Namespace::SharedWithMe;
        contract.shared_root_id = Some("revoked-root".into());
        bridge.db.set_file_contract_state(&contract).unwrap();
        let logs = capture_logs_async(async {
            let ((), ()) = tokio::join!(
                async {
                    bridge.process_due_operations(&sync_root, now_secs()).await.unwrap();
                },
                async {
                    tokio::time::timeout(Duration::from_secs(10), async {
                        while server.state.lock().unwrap().inits.is_empty() {
                            tokio::time::sleep(Duration::from_millis(5)).await;
                        }
                    })
                    .await
                    .expect("init never reached the mock");
                    bridge.db.purge_revoked_shared_content(&[]).unwrap();
                }
            );
        })
        .await;
        let line = assert_clean_line(&logs, "queue state moved", &["op_id", "step"], dir.path());
        assert!(line.contains(&w.op_id) && line.contains("\"resume\""), "{line}");
        drop(server.finish());
    }

    /// T37: an upload the server completed, whose local landing failed once, says the landing
    /// is retried (P9, one injected failure).
    #[tokio::test]
    async fn log_completed_landing_retried() {
        let master_key = [110u8; 32];
        let (dir, sync_root, server, bridge) = rule3_setup(master_key);
        seed_uploaded_row(&bridge, &server, "recorded");
        fp_save(&bridge, dir.path(), "recorded", PLANTED, b"edit", "1");
        crate::state_db::fail_next_landings_for_test(1);
        let now = now_secs();
        let logs = capture_logs_async(async {
            bridge.process_due_operations(&sync_root, now).await.unwrap();
        })
        .await;
        let line = assert_clean_line(
            &logs,
            "upload completed on the server; local landing will be retried",
            &["op_id", "file_id", "attempt"],
            dir.path(),
        );
        assert!(
            line.contains("file_id=\"recorded\"") && line.contains("attempt=1"),
            "{line}"
        );
        bridge.process_due_operations(&sync_root, now + 10_000).await.unwrap();
        assert!(
            bridge.db.list_operations_for_file("recorded").unwrap().is_empty(),
            "landed on the retry"
        );
        drop(server.finish());
    }

    /// T37: every hydrate the daemon serves is counted at `INFO` with its source: a queued
    /// create (T18) and an empty queued save (RF3), both from the queue.
    #[tokio::test]
    async fn log_hydrate_served() {
        let (dir, _sync_root, server, bridge) = rule3_setup([111u8; 32]);
        let created = fp_create(&bridge, dir.path(), PLANTED, b"queued bytes");
        let p = created.outcome_file_id();
        let dest = dir.path().join("fetch").join(PLANTED);
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        let logs = capture_logs_async(async {
            let source = bridge.serve_hydrate(&p, &dest, &[dir.path()], None).await.unwrap();
            assert_eq!(source, HydrateSource::Queue);
        })
        .await;
        let line = assert_clean_line(&logs, "Finder hydrate served", &["file_id", "source"], dir.path());
        assert!(
            line.contains("INFO") && line.contains(&format!("file_id={p}")) && line.contains("source=\"queue\""),
            "{line}"
        );

        let empty = "3f2a9c1e-0000-4000-8000-0000000000e3";
        seed_uploaded_row(&bridge, &server, empty);
        fp_save(&bridge, dir.path(), empty, PLANTED, b"", "1");
        let dest = dir.path().join("fetch-empty").join(PLANTED);
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        let logs = capture_logs_async(async {
            let source = bridge.serve_hydrate(empty, &dest, &[dir.path()], None).await.unwrap();
            assert_eq!(source, HydrateSource::Queue);
        })
        .await;
        assert_eq!(std::fs::metadata(&dest).unwrap().len(), 0, "an empty file");
        let line = assert_clean_line(&logs, "Finder hydrate served", &["file_id", "source"], dir.path());
        assert!(
            line.contains("INFO") && line.contains(&format!("file_id={empty}")) && line.contains("source=\"queue\""),
            "{line}"
        );
        assert!(server.finish().requests.is_empty(), "nothing reached the server");
    }

    /// T38 (M5): an id taken from the wire is logged only when it parses as a UUID; otherwise
    /// the line says `id_kind="non_uuid"`. The write path and the hydrate failure line, which
    /// on Linux logs any caller-supplied text (no identifier check there).
    #[cfg(unix)] // names `crate::ipc_socket`
    #[tokio::test]
    async fn m5_a_non_uuid_wire_id_is_never_logged() {
        let (dir, _sync_root, server, bridge) = rule3_setup([101u8; 32]);
        assert_eq!(
            loggable_id("3f2a9c1e-0000-4000-8000-000000000001"),
            "3f2a9c1e-0000-4000-8000-000000000001"
        );
        assert_eq!(loggable_id("../../etc/passwd"), "non_uuid");
        let contents = dir.path().join("x.txt");
        std::fs::write(&contents, b"x").unwrap();
        let logs = capture_logs_async(async {
            // An unknown non-UUID id on the write path …
            let result = bridge.queue_file_provider_modify(finder_file_target(
                Some("not-a-uuid/../evil"),
                "x.txt",
                &contents,
                Some("1".into()),
            ));
            let _ = crate::ipc_socket::write_outcome_response("modify", &bridge.db, result);
            // … and the hydrate failure line.
            let _ = crate::ipc_socket::hydrate_failure_reply("not-a-uuid/../evil", anyhow::anyhow!("boom"));
        })
        .await;
        assert!(!logs.contains("not-a-uuid"), "{logs}");
        assert!(logs.contains("non_uuid"), "{logs}");
        assert_eq!(
            logs.matches("id_kind=\"non_uuid\"").count(),
            2,
            "one refused write and one failed hydrate, each with the kind only:\n{logs}"
        );
        drop(server.finish());
    }

    /// M5 for every other line that logs an id from the wire: the alias lines (the request's
    /// provisional id), the hydrate count (the requested id) and the delete of an unknown item.
    #[tokio::test]
    async fn m5_every_wire_id_line_logs_the_kind_of_a_non_uuid() {
        let dir = tempfile::tempdir().unwrap();
        let bridge = test_bridge(&dir.path().join("state.db"));
        let wire = "not-a-uuid/../evil";
        let logs = capture_logs_async(async {
            // `super::`: the T37 tests above are named after these helpers.
            super::log_alias_resolved("modify", wire, "server-file-1");
            super::log_alias_delete_kept(wire, "server-file-1");
            super::log_hydrate_served(wire, HydrateSource::Queue);
            super::log_alias_resolved("modify", "3f2a9c1e-0000-4000-8000-000000000002", "server-file-2");
            assert!(matches!(
                bridge.queue_finder_delete(wire, None).unwrap(),
                FinderWriteOutcome::Ignored { .. }
            ));
        })
        .await;
        assert!(!logs.contains("not-a-uuid"), "{logs}");
        assert_eq!(logs.matches("id_kind=\"non_uuid\"").count(), 4, "{logs}");
        assert!(
            logs.contains("provisional_id=3f2a9c1e-0000-4000-8000-000000000002"),
            "a UUID is logged as itself:\n{logs}"
        );
    }

    // ── Errors in log lines: a category, never the text (spec §11) ──

    /// A line about a failed step logs the error's category. The text can carry a
    /// path (an I/O error's context, a database file name) and never reaches the log.
    #[test]
    fn an_error_is_logged_by_its_category_never_its_text() {
        use rusqlite::ffi;
        let planted = format!("/Users/someone/{PLANTED}");
        let io = |kind| anyhow::Error::new(std::io::Error::new(kind, planted.clone()));
        // A SQLite failure with a result code, and a message that names a path.
        let sqlite = |code| {
            anyhow::Error::new(rusqlite::Error::SqliteFailure(
                ffi::Error::new(code),
                Some(format!("unable to use {planted}")),
            ))
        };
        let memory = rusqlite::Connection::open_in_memory().unwrap();
        let no_such_table = memory.execute("UPDATE no_such_table SET x = 1", []).unwrap_err();
        memory.execute("CREATE TABLE t (a INTEGER)", []).unwrap();
        // A column the schema does not have, named in a SELECT: SQLite reports it at prepare,
        // as rusqlite's `SqlInputError`.
        let no_such_column = memory
            .query_row("SELECT nope FROM t", [], |row| row.get::<_, i64>(0))
            .unwrap_err();
        let wrong_type = memory
            .query_row("SELECT 'text'", [], |row| row.get::<_, i64>(0))
            .unwrap_err();
        let no_row = memory
            .query_row("SELECT 1 WHERE 0", [], |row| row.get::<_, i64>(0))
            .unwrap_err();
        // Each cause support answers differently has its own category.
        let cases: Vec<(&str, anyhow::Error, &str)> = vec![
            ("missing file", io(std::io::ErrorKind::NotFound), "not_found"),
            (
                "permission",
                io(std::io::ErrorKind::PermissionDenied),
                "permission_denied",
            ),
            ("full disk (I/O)", io(std::io::ErrorKind::StorageFull), "disk_full"),
            (
                "read-only volume",
                io(std::io::ErrorKind::ReadOnlyFilesystem),
                "read_only",
            ),
            ("other I/O", io(std::io::ErrorKind::Other), "io"),
            ("busy database", sqlite(ffi::SQLITE_BUSY), "database_busy"),
            ("locked table", sqlite(ffi::SQLITE_LOCKED), "database_busy"),
            ("full disk (database)", sqlite(ffi::SQLITE_FULL), "database_full"),
            ("corrupt database", sqlite(ffi::SQLITE_CORRUPT), "database_corrupt"),
            ("not a database", sqlite(ffi::SQLITE_NOTADB), "database_corrupt"),
            ("read-only database", sqlite(ffi::SQLITE_READONLY), "database_read_only"),
            (
                "database cannot open",
                sqlite(ffi::SQLITE_CANTOPEN),
                "database_cannot_open",
            ),
            ("database I/O", sqlite(ffi::SQLITE_IOERR), "database_io"),
            ("constraint", sqlite(ffi::SQLITE_CONSTRAINT), "database_constraint"),
            (
                "schema: no such table",
                anyhow::Error::new(no_such_table),
                "database_schema",
            ),
            (
                "schema: no such column in a SELECT",
                anyhow::Error::new(no_such_column),
                "database_schema",
            ),
            (
                "schema: wrong column type",
                anyhow::Error::new(wrong_type),
                "database_schema",
            ),
            ("no row", anyhow::Error::new(no_row), "database"),
            (
                "busy, under a path-bearing context",
                sqlite(ffi::SQLITE_BUSY).context(format!("open {planted}")),
                "database_busy",
            ),
            (
                "text only",
                anyhow::anyhow!("staged upload payload is missing: {planted}"),
                "other",
            ),
        ];
        let wrong: Vec<String> = cases
            .iter()
            .filter(|(_, error, expected)| error_category(error.as_ref()) != *expected)
            .map(|(what, error, expected)| format!("{what}: {} (expected {expected})", error_category(error.as_ref())))
            .collect();
        assert!(
            wrong.is_empty(),
            "categories that do not tell the cause apart:\n{}",
            wrong.join("\n")
        );
        for (what, error, _) in &cases {
            let category = error_category(error.as_ref());
            assert!(
                !category.contains('/') && !category.contains("Quarterly"),
                "{what}: {category}"
            );
        }
    }

    /// A staged copy that cannot be unlinked
    /// is logged by its category; the I/O error's text never reaches the line.
    #[tokio::test]
    async fn a_staged_copy_kept_after_a_failed_unlink_is_logged_by_category() {
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        // A directory: `remove_file` fails on it (EPERM on a Mac, EISDIR on Linux).
        let not_a_file = dir.path().join(PLANTED);
        std::fs::create_dir(&not_a_file).unwrap();
        db.track_staged_payload(&not_a_file.to_string_lossy(), None, false)
            .unwrap();
        let logs = capture_logs_async(async {
            crate::staged_payload::remove_or_log(&db, &not_a_file);
        })
        .await;
        let line = assert_clean_line(
            &logs,
            "staged upload cleanup deferred; journal retained",
            &["reason"],
            dir.path(),
        );
        assert!(!line.contains("os error"), "the error's text: {line}");
        assert!(not_a_file.exists(), "kept");
    }

    /// The line in `staged_payload.rs`: a released copy that engine start
    /// cannot remove is logged by its category, and its journal row stays for the next start.
    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn a_released_copy_engine_start_cannot_remove_is_logged_by_category() {
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        let not_a_file = dir.path().join(PLANTED);
        std::fs::create_dir(&not_a_file).unwrap();
        let path = not_a_file.to_string_lossy().into_owned();
        db.track_staged_payload(&path, None, false).unwrap();
        let mut removed = usize::MAX;
        let logs = capture_logs_async(async {
            removed = crate::staged_payload::remove_released(&db, std::slice::from_ref(&path));
        })
        .await;
        assert_eq!(removed, 0);
        let line = assert_clean_line(
            &logs,
            "released upload copy kept; removal is retried at the next start",
            &["reason"],
            dir.path(),
        );
        assert!(!line.contains("os error"), "the error's text: {line}");
    }

    // ── Paths kept out of retry texts, refused-attempt wording, Show in Finder through the alias ──

    /// A missing staged copy of an upload without a write id keeps today's
    /// retry, and its text carries no path: a path containing `403` would otherwise be read
    /// as a refused request by `classify_operation_error` and pause the op.
    #[tokio::test]
    async fn a_missing_staged_copy_is_retried_whatever_its_path_contains() {
        let dir = tempfile::tempdir().unwrap();
        let bridge = test_bridge(&dir.path().join("state.db"));
        seed_bridge_row(&bridge, "file-1", "Draft.txt", None, FileStatus::Uploading, 0);
        let missing = dir.path().join("staged-403").join("payload");
        bridge
            .db
            .enqueue_operation(&PendingOperation {
                op_id: "op-upload".into(),
                kind: OperationKind::UploadVersion,
                file_id: Some("file-1".into()),
                parent_id: None,
                target_path: Some("Draft.txt".into()),
                metadata_json: Some(r#"{"operation":"upload_version"}"#.into()),
                payload_path: Some(missing.to_string_lossy().into_owned()),
                base_version: Some(1),
                base_object_version_id: None,
                attempts: 0,
                max_attempts: 5,
                next_retry_at: 0,
                last_error: None,
                backup_source_key: None,
                created_at: 100,
                updated_at: 100,
            })
            .unwrap();
        let outcome = bridge.process_due_operations(dir.path(), 200).await.unwrap();
        assert!(
            outcome.paused_op_ids.is_empty(),
            "a missing copy is not a refused request: {outcome:?}"
        );
        assert_eq!(outcome.retried_op_ids, vec!["op-upload".to_string()]);
        let op = bridge.db.get_operation("op-upload").unwrap().unwrap();
        assert_eq!(
            op.last_error.as_deref(),
            Some("staged upload payload is missing"),
            "a fixed text, without the path"
        );
    }

    /// A refused attempt's line is worded by why the op left the queue: a
    /// stale base parks at once (the attempt that was made, not the cap), attempts used up
    /// park at the cap, and any other refusal retries.
    #[tokio::test]
    async fn a_refused_upload_line_is_worded_by_why_it_parked() {
        let op = PendingOperation {
            op_id: "op-refused".into(),
            kind: OperationKind::UploadVersion,
            file_id: Some("file-refused".into()),
            parent_id: None,
            target_path: Some(PLANTED.into()),
            metadata_json: None,
            payload_path: None,
            base_version: Some(1),
            base_object_version_id: None,
            attempts: 0,
            max_attempts: 25,
            next_retry_at: 0,
            last_error: None,
            backup_source_key: None,
            created_at: 100,
            updated_at: 100,
        };
        use crate::api_client::InitConflictClass;
        let at_once =
            capture_logs_async(async { log_refused_upload(&op, 1, InitConflictClass::StaleBase, true) }).await;
        let used_up =
            capture_logs_async(async { log_refused_upload(&op, 25, InitConflictClass::StaleBase, false) }).await;
        let retry =
            capture_logs_async(async { log_refused_upload(&op, 3, InitConflictClass::InProgress, false) }).await;
        assert!(
            at_once.contains("its base is stale, parked with its bytes kept in the queue")
                && !at_once.contains("attempts used up")
                && at_once.contains("attempt=1 ")
                && at_once.contains("reason=\"stale_base\""),
            "{at_once}"
        );
        assert!(
            used_up.contains("attempts used up, parked with its bytes kept in the queue")
                && used_up.contains("attempt=25 "),
            "{used_up}"
        );
        assert!(retry.contains("; will retry") && !retry.contains("parked"), "{retry}");
    }

    /// "Show in Finder" under a provisional id whose create has landed (a stale quick
    /// search result, a request after the swap) reaches the server file through the alias; the
    /// server file's row decides. Without the alias it answered "not known on this Mac".
    #[tokio::test]
    async fn show_in_finder_under_a_provisional_id_after_the_landing_reaches_the_server_file() {
        let (dir, sync_root, server, bridge) = rule3_setup([112u8; 32]);
        let (p, s, _token) = landed_create(&bridge, &server, dir.path(), &sync_root, "shown.txt").await;
        let check =
            |id: &str| crate::finder_open::finder_may_show(id, |id| crate::finder_open::item_namespace(&bridge.db, id));
        assert_eq!(check(&s), Ok(()), "the server id, as before");
        assert_eq!(check(&p), Ok(()), "the provisional id reaches S");
        let mut contract = bridge.db.get_file_contract_state(&s).unwrap().unwrap();
        contract.namespace = Namespace::SharedWithMe;
        bridge.db.set_file_contract_state(&contract).unwrap();
        assert_eq!(
            check(&p),
            Err(crate::finder_open::SHARED_NOT_IN_FINDER.to_string()),
            "S's row decides, not a row of P's"
        );
        assert_eq!(
            check(&s),
            Err(crate::finder_open::SHARED_NOT_IN_FINDER.to_string()),
            "the server id's own row decides when no alias is followed"
        );
        assert_eq!(
            check("3f2a9c1e-0000-4000-8000-00000000dead"),
            Err(crate::finder_open::ITEM_NOT_KNOWN.to_string()),
            "an id with neither a row nor an alias is still unknown"
        );
        drop(server.finish());
    }
    /// Task 1919, against a REAL local API (ignored by default). The desktop's own code path end to end:
    /// `ApiClient::set_preferred_region` (the call `account_set_region` makes), then the desktop's own upload
    /// (`EngineBridge::process_due_operations` -> `POST /uploads/init`, which sends no region) and then the
    /// file's `storage_location` as the API reports it (task 2050: by COUNTRY). A choice must land the file in a
    /// pool of THAT country; "europe" must take the default placement; a coming-soon country must be refused and
    /// change nothing.
    ///
    /// ```text
    /// BB_REAL_API_BASE=http://127.0.0.1:39191 BB_REAL_API_TOKEN=<session token> \
    ///   cargo test --lib real_api_region_choice -- --ignored --nocapture --test-threads=1
    /// ```
    #[tokio::test]
    #[ignore = "needs a running local API with de/fi pools: BB_REAL_API_BASE and BB_REAL_API_TOKEN"]
    async fn real_api_region_choice_upload_lands_in_the_chosen_country() {
        let base = std::env::var("BB_REAL_API_BASE").expect("BB_REAL_API_BASE");
        let token = std::env::var("BB_REAL_API_TOKEN").expect("BB_REAL_API_TOKEN");
        assert!(
            base.starts_with("http://127.0.0.1:") || base.starts_with("http://localhost:"),
            "this test only ever talks to a local API, got {base}"
        );
        let master_key = [21u8; 32];
        let api = Arc::new(ApiClient::new(base.clone(), token.clone(), master_key));
        let http = reqwest::Client::new();
        let dir = tempfile::tempdir().unwrap();
        let bridge = EngineBridge::new(Arc::new(StateDb::open(&dir.path().join("state.db")).unwrap()), api.clone());

        // The list the desktop's `account_region` decodes is the API's.
        let listed = api.account_region().await.expect("GET /me/region");
        let ids: Vec<&str> = listed.choices.iter().map(|c| c.id.as_str()).collect();
        println!("choices from the API: {ids:?}");
        assert_eq!(ids, ["europe", "fi", "de", "nl"]);
        let nl = listed.choices.iter().find(|c| c.id == "nl").unwrap();
        assert!(!nl.selectable && nl.status == "coming_soon");

        let mut op_no = 0;
        let mut landed = Vec::new();
        // "europe" lands in the default pool, which is a German one.
        for (choice, expected_country) in [("fi", "Finland"), ("de", "Germany"), ("europe", "Germany")] {
            let saved = api.set_preferred_region(Some(choice), Some(false)).await.expect("PUT /me/region");
            assert_eq!(saved.preferred_region.as_deref(), Some(choice));
            assert_eq!(
                api.account_region().await.unwrap().preferred_region.as_deref(),
                Some(choice),
                "the stored preference reads back"
            );

            op_no += 1;
            let payload = dir.path().join(format!("payload-{op_no}.txt"));
            std::fs::write(&payload, format!("1919 desktop upload for {choice}")).unwrap();
            let before: std::collections::HashSet<String> =
                bridge.db.list_files().unwrap().into_iter().map(|f| f.file_id).collect();
            bridge
                .db
                .enqueue_operation(&PendingOperation {
                    op_id: format!("op-1919-{op_no}"),
                    kind: OperationKind::UploadVersion,
                    file_id: Some(format!("local-1919-{op_no}")),
                    parent_id: None,
                    target_path: Some(format!("rc1919-{choice}.txt")),
                    metadata_json: Some(
                        serde_json::json!({
                            "operation": "create_file",
                            "name_encrypted": "{}",
                            "display_name": format!("rc1919-{choice}.txt"),
                            "content_type": "text/plain"
                        })
                        .to_string(),
                    ),
                    payload_path: Some(payload.to_string_lossy().into_owned()),
                    base_version: None,
                    base_object_version_id: None,
                    attempts: 0,
                    max_attempts: 1,
                    next_retry_at: 0,
                    last_error: None,
                    backup_source_key: None,
                    created_at: 100,
                    updated_at: 100,
                })
                .unwrap();
            let outcome = bridge.process_due_operations(dir.path(), 200).await.expect("the upload pass runs");
            assert_eq!(outcome.completed_op_ids, vec![format!("op-1919-{op_no}")], "{outcome:?}");

            let after = bridge.db.list_files().unwrap();
            let fresh: Vec<_> = after.iter().filter(|f| !before.contains(&f.file_id)).collect();
            assert_eq!(fresh.len(), 1, "exactly one new file row per upload: {fresh:?}");
            let file_id = fresh[0].file_id.clone();

            let body: serde_json::Value = http
                .get(format!("{base}/api/v1/files/{file_id}"))
                .bearer_auth(&token)
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            let country = body["storage_location"]["country"].as_str().unwrap_or("<none>").to_string();
            println!("UPLOAD choice={choice} file_id={file_id} storage_location.country={country}");
            assert_eq!(country, expected_country, "choice {choice}: {body}");
            landed.push((choice, file_id, country));
        }

        // A coming-soon country is refused and the stored choice is left alone.
        let refused = api.set_preferred_region(Some("nl"), Some(false)).await.expect_err("nl must be refused");
        println!("PUT nl -> {refused}");
        assert!(refused.to_string().contains("region_unavailable"), "{refused}");
        assert_eq!(api.account_region().await.unwrap().preferred_region.as_deref(), Some("europe"));
        println!("LANDED {landed:?}");
    }

    /// Task 2173: the engine's whole-file hydrate over the direct transport, against a loopback API
    /// and storage pair that speaks the contract (`direct_download::test_support`).
    mod direct_download_engine {
        use super::*;
        use crate::direct_download::test_support::{GrantScript, Pair, SECRET, SIGNATURE, Version, capture_logs, content, file_key};
        use crate::direct_download::{FileTransport, Transport};

        const KEY: [u8; 32] = [31; 32];
        const SIZE: usize = 10 * 1024 + 77;

        fn bridge_for(dir: &Path, pair: &Pair) -> EngineBridge {
            let bridge = test_bridge_with_api(&dir.join("state.db"), pair.api.base.clone(), KEY);
            seed_bridge_row(&bridge, TEST_FILE_ID, "/direct.bin", None, FileStatus::CloudOnly, SIZE as i64);
            bridge
        }

        #[tokio::test]
        async fn a_direct_hydrate_writes_the_same_bytes_and_reports_like_the_api_path() {
            let dir = tempfile::tempdir().unwrap();
            let pair = Pair::start(KEY, TEST_FILE_ID, 1024, SIZE);
            let bridge = bridge_for(dir.path(), &pair);
            let reports: Arc<Mutex<Vec<(u64, u64)>>> = Arc::default();
            let seen = reports.clone();
            let progress = move |done: u64, total: u64| seen.lock().unwrap().push((done, total));
            let dest = dir.path().join("direct.bin");
            bridge
                .hydrate_file_with_progress(TEST_FILE_ID, &dest, &[dir.path()], Some(&progress))
                .await
                .unwrap();
            assert_eq!(std::fs::read(&dest).unwrap(), pair.plain(), "byte-identical on disk");
            assert_eq!(pair.storage_requests(), 11);
            assert!(pair.api_chunk_requests().is_empty(), "no chunk through the API");
            let encrypted = pair.knobs().version.encrypted_size();
            assert_eq!(bridge.wire.download_bytes.load(Ordering::Relaxed), encrypted, "wire bytes counted");
            let reports = reports.lock().unwrap().clone();
            let total = SIZE as u64;
            assert_eq!(reports.first(), Some(&(0, total)), "the start, as on the API path");
            assert_eq!(reports.last(), Some(&(total, total)));
            assert_eq!(reports.len(), 12, "the start, then once per chunk");
            assert_eq!(bridge.db.get_file(TEST_FILE_ID).unwrap().unwrap().status, FileStatus::Local);
        }

        #[tokio::test]
        async fn a_version_replaced_mid_download_restarts_once_from_fresh_metadata() {
            let dir = tempfile::tempdir().unwrap();
            let pair = Pair::start(KEY, TEST_FILE_ID, 1024, SIZE);
            let replacement = Version::new(
                &file_key(KEY, TEST_FILE_ID),
                "99999999-2222-4333-8444-555555555555",
                1024,
                content(SIZE + 5000, 9),
            );
            {
                let mut k = pair.knobs();
                k.replacement = Some(replacement.clone());
                k.grants.push_back(GrantScript::Serve);
                k.grants.push_back(GrantScript::Replace);
            }
            let bridge = bridge_for(dir.path(), &pair);
            let plaintext = bridge.do_hydrate(TEST_FILE_ID, None).await.unwrap();
            assert_eq!(&*plaintext, replacement.plain.as_slice(), "the new version, whole, never mixed");
            let metadata = pair.api.count(|s| s.method == "GET" && s.path() == format!("/api/v1/files/{TEST_FILE_ID}"));
            assert_eq!(metadata, 2, "fresh metadata once");
            assert!(pair.api_chunk_requests().is_empty());
        }

        #[tokio::test]
        async fn a_replace_with_more_chunks_before_the_first_window_restarts_and_never_publishes_a_truncated_file() {
            // Review P2-2: get_file names A; B, with more chunks, completes before the first grant.
            let dir = tempfile::tempdir().unwrap();
            let pair = Pair::start(KEY, TEST_FILE_ID, 1024, SIZE);
            let more = Version::new(
                &file_key(KEY, TEST_FILE_ID),
                "99999999-2222-4333-8444-555555555555",
                1024,
                content(SIZE + 5000, 9),
            );
            pair.knobs().replacement = Some(more.clone());
            pair.knobs().grants.push_back(GrantScript::Replace);
            let bridge = bridge_for(dir.path(), &pair);
            let dest = dir.path().join("direct.bin");
            bridge.hydrate_file(TEST_FILE_ID, &dest, &[dir.path()]).await.unwrap();
            assert_eq!(std::fs::read(&dest).unwrap(), more.plain, "B, whole: never the old count's prefix");
            let metadata = pair.api.count(|s| s.method == "GET" && s.path() == format!("/api/v1/files/{TEST_FILE_ID}"));
            assert_eq!(metadata, 2, "fresh metadata once");
            assert!(pair.api_chunk_requests().is_empty(), "the unpinned API loop never ran");
        }

        #[tokio::test]
        async fn a_conflict_preview_restarts_once_when_the_version_goes_away() {
            let dir = tempfile::tempdir().unwrap();
            // 12 chunks: two grant windows, so the second one meets the replacement.
            let pair = Pair::start_with(KEY, TEST_FILE_ID, 1024, b"remote version A ".repeat(700));
            let b = Version::new(
                &file_key(KEY, TEST_FILE_ID),
                "99999999-2222-4333-8444-555555555555",
                1024,
                b"remote version B ".repeat(700),
            );
            {
                let mut k = pair.knobs();
                k.replacement = Some(b.clone());
                k.grants.push_back(GrantScript::Serve);
                k.grants.push_back(GrantScript::Replace);
            }
            let bridge = bridge_for(dir.path(), &pair);
            let (side, _) = bridge.remote_content_preview(TEST_FILE_ID, ContentKind::Text).await;
            assert_eq!(side.unavailable_reason, None);
            assert_eq!(side.text.as_deref(), Some(String::from_utf8(b.plain.clone()).unwrap().as_str()));
        }

        #[test]
        fn a_refused_download_logs_by_category() {
            use crate::direct_download::DirectError;
            for (status, want) in [(401, "permission_denied"), (403, "permission_denied"), (404, "not_found"), (500, "other")] {
                let e = anyhow::Error::new(DirectError::Refused(status));
                assert_eq!(error_category(e.as_ref()), want, "{status}");
            }
        }

        #[tokio::test]
        async fn forcing_the_api_transport_keeps_the_old_requests() {
            let dir = tempfile::tempdir().unwrap();
            let pair = Pair::start(KEY, TEST_FILE_ID, 1024, SIZE);
            let bridge = bridge_for(dir.path(), &pair);
            bridge.seams.use_transport(Transport::Api);
            let meta = bridge.api.get_file(TEST_FILE_ID).await.unwrap();
            let (plaintext, transport) = bridge.hydrate_whole_file(TEST_FILE_ID, &meta, None).await.unwrap();
            assert_eq!((plaintext.to_vec(), transport), (pair.plain(), FileTransport::Api));
            assert_eq!(pair.discovery_requests(), 0);
            assert!(pair.grant_requests().is_empty());
            let chunks = pair.api_chunk_requests();
            assert_eq!(chunks.len(), 11);
            // Task 1643: the API path is bound to the metadata's version.
            let pinned = format!("object_version_id={}", pair.knobs().version.id);
            assert!(chunks.iter().all(|r| r.query() == pinned), "every chunk pinned");
        }

        #[tokio::test]
        async fn a_legacy_file_takes_no_metadata_pin_on_either_path() {
            // `download_version: null`: the client invents no pin. The API path is today's unpinned route; the direct
            // path is bound by the planner's own pin, the version the first grant window names.
            let dir = tempfile::tempdir().unwrap();
            let pair = Pair::start(KEY, TEST_FILE_ID, 1024, SIZE);
            pair.knobs().legacy = true;
            let bridge = bridge_for(dir.path(), &pair);
            bridge.seams.use_transport(Transport::Api);
            let meta = bridge.api.get_file(TEST_FILE_ID).await.unwrap();
            let (plaintext, transport) = bridge.hydrate_whole_file(TEST_FILE_ID, &meta, None).await.unwrap();
            assert_eq!((plaintext.to_vec(), transport), (pair.plain(), FileTransport::Api));
            assert!(pair.api_chunk_requests().iter().all(|r| r.query().is_empty()), "API path: no pin is invented");

            let dir = tempfile::tempdir().unwrap();
            let pair = Pair::start(KEY, TEST_FILE_ID, 1024, SIZE);
            pair.knobs().legacy = true;
            let bridge = bridge_for(dir.path(), &pair);
            let meta = bridge.api.get_file(TEST_FILE_ID).await.unwrap();
            let (plaintext, transport) = bridge.hydrate_whole_file(TEST_FILE_ID, &meta, None).await.unwrap();
            assert_eq!((plaintext.to_vec(), transport), (pair.plain(), FileTransport::Direct));
            let windows: Vec<serde_json::Value> =
                pair.grant_requests().iter().map(|r| serde_json::from_slice(&r.body).unwrap()).collect();
            assert_eq!(windows[0]["object_version_id"], serde_json::Value::Null, "the first window pins nothing itself");
            let planner_pin = pair.knobs().version.id.clone();
            assert!(windows[1..].iter().all(|w| w["object_version_id"] == planner_pin.as_str()), "later windows: the planner's pin");
            assert!(pair.api_chunk_requests().is_empty());
        }

        #[tokio::test]
        async fn a_range_read_stays_on_the_api() {
            let dir = tempfile::tempdir().unwrap();
            let pair = Pair::start(KEY, TEST_FILE_ID, 1024, SIZE);
            let bridge = bridge_for(dir.path(), &pair);
            let range = bridge.hydrate_file_range(TEST_FILE_ID, 1024, 10).await.unwrap().unwrap();
            assert_eq!(&*range.data, &pair.plain()[1024..2048]);
            assert_eq!(pair.discovery_requests(), 0, "a range read never asks for windows");
            assert_eq!(pair.api_chunk_requests().len(), 2);
            assert_eq!(pair.storage_requests(), 0);
        }

        fn real_env(name: &str) -> String {
            std::env::var(name)
                .unwrap_or_else(|_| panic!("{name} must be set (see the test's doc); this test never passes vacuously"))
        }

        /// The desktop engine against a REAL local server: the API path, the direct path and a
        /// forced fallback (every storage fetch fails, so every chunk is renewed three times against
        /// the real grant route and then fetched through the real pinned per-chunk route) all return
        /// the same bytes. Ignored by default; it starts nothing and refuses any non-loopback URL.
        ///
        /// ```text
        /// BB_REAL_SERVER_URL=http://127.0.0.1:3994   # beebeeb-api, its own Postgres, Redis, MinIO
        /// BB_REAL_SESSION_TOKEN=<token>              # a session of a fresh test account
        /// BB_REAL_FILE_ID=<id>                       # a file that account uploaded with a zero master key
        /// BB_REAL_SHA256=<hex>                       # the sha256 of its plaintext
        /// BB_REAL_EXPECT_TRANSPORT=direct|api        # what the auto run must use
        /// BB_REAL_REPORT=<file>                      # optional: result lines are appended here
        /// ```
        #[tokio::test(flavor = "current_thread")]
        #[ignore = "needs a running server and MinIO: set BB_REAL_SERVER_URL, BB_REAL_SESSION_TOKEN, BB_REAL_FILE_ID, BB_REAL_SHA256, BB_REAL_EXPECT_TRANSPORT"]
        async fn real_server_api_direct_and_forced_fallback_return_identical_bytes() {
            use sha2::Digest;
            let url = real_env("BB_REAL_SERVER_URL");
            assert!(
                url.starts_with("http://127.0.0.1:") || url.starts_with("http://localhost:"),
                "refusing to run against {url}: only a loopback http server"
            );
            let token = real_env("BB_REAL_SESSION_TOKEN");
            let file_id = real_env("BB_REAL_FILE_ID");
            let want = real_env("BB_REAL_SHA256");
            let expect = real_env("BB_REAL_EXPECT_TRANSPORT");
            assert!(expect == "direct" || expect == "api", "BB_REAL_EXPECT_TRANSPORT is direct or api");
            let fallback = if expect == "direct" { "mixed" } else { "api" };
            // (label, transport, every storage fetch fails, transport the run must report)
            let cells = [
                ("auto", Transport::Auto, false, expect.as_str()),
                ("forced-api", Transport::Api, false, "api"),
                ("forced-fallback", Transport::Auto, true, fallback),
            ];
            let (report, logs) = capture_logs(async {
                let mut report = Vec::new();
                for (label, transport, fail_storage, expected) in cells {
                    let dir = tempfile::tempdir().unwrap();
                    let db = Arc::new(StateDb::open(dir.path().join("state.db")).unwrap());
                    // A fresh client per cell: each asks discovery itself. The 32 zero bytes are the
                    // test account's master key (the server never sees it).
                    let api = Arc::new(ApiClient::new(url.clone(), token.clone(), [0u8; 32]));
                    let bridge = EngineBridge::new(db, api);
                    bridge.seams.use_transport(transport);
                    if fail_storage {
                        bridge.seams.fail_storage(503);
                    }
                    let meta = bridge.api.get_file(&file_id).await.expect("file metadata");
                    let started = std::time::Instant::now();
                    let (plaintext, used) = bridge
                        .hydrate_whole_file(&file_id, &meta, None)
                        .await
                        .unwrap_or_else(|e| panic!("{label}: {e:#}"));
                    let got: String = sha2::Sha256::digest(&*plaintext).iter().map(|b| format!("{b:02x}")).collect();
                    let line = format!(
                        "{label}: transport={} sha256={got} bytes={} wire={} ms={}",
                        used.as_str(),
                        plaintext.len(),
                        bridge.wire.download_bytes.load(Ordering::Relaxed),
                        started.elapsed().as_millis()
                    );
                    println!("{line}");
                    assert_eq!(got, want, "{label}: different bytes");
                    assert_eq!(used.as_str(), expected, "{label}: another transport");
                    report.push(line);
                }
                report
            })
            .await;
            let leaks = logs.lines().filter(|l| l.contains(SIGNATURE) || l.contains(SECRET)).count();
            let line = format!("log lines captured={} with {SIGNATURE}={leaks}", logs.lines().count());
            println!("{line}");
            assert_eq!(leaks, 0, "a signed link reached the log");
            if let Ok(path) = std::env::var("BB_REAL_REPORT") {
                let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path).unwrap();
                for l in report.iter().chain(std::iter::once(&line)) {
                    writeln!(f, "{l}").unwrap();
                }
            }
        }
    }

