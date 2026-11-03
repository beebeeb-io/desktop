//! Presentation-read warnings, refused-write and hydrate logging tests.

use super::*;


    /// Everything `tracing` emits on this thread while `body` runs, as text.
    /// A presentation read that fails (here the one bug row, a held
    /// write id without its base) presents the item from its contract, as round 3
    /// did: never a spurious "0", and the contract's capabilities are kept.
    #[test]
    fn a_failed_presentation_read_presents_the_contract_and_logs_it() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed_1694_item(
            &db,
            "half-set",
            FileStatus::Local,
            PERMISSION_READ | PERMISSION_WRITE,
            ItemKind::File,
        );
        let mut contract = db.get_file_contract_state("half-set").unwrap().unwrap();
        contract.current_version = 4;
        db.set_file_contract_state(&contract).unwrap();
        let entry = db.get_file("half-set").unwrap().unwrap();
        let healthy = file_entry_payload_for_db(&db, &entry, FP_ROOT_APPLE);
        assert_eq!(healthy.content_version.as_deref(), Some("4"));

        db.set_held_pair_for_test("half-set", Some(&"a".repeat(32)), None);
        assert!(db.item_presentation("half-set").is_err(), "the bug row is a read error");
        let mut presented = None;
        let logs = capture_logs(|| presented = Some(file_entry_payload_for_db(&db, &entry, FP_ROOT_APPLE)));
        let presented = presented.unwrap();
        assert_eq!(
            serde_json::to_value(&presented).unwrap(),
            serde_json::to_value(&healthy).unwrap(),
            "the item is presented from its contract, never as \"0\""
        );
        let lines: Vec<&str> = logs
            .lines()
            .filter(|line| line.contains("presentation_read_failed"))
            .collect();
        assert_eq!(lines.len(), 1, "one line per failed read:\n{logs}");
        assert!(lines[0].contains("WARN") && lines[0].contains("half-set"), "{logs}");
    }

    /// The limiter at its cap: 1024 items, each with every kind of failure (`error_category`
    /// has 16). It stays bounded, a new kind on an item it holds is still logged at once without
    /// dropping anything, a new item is still logged, and only items whose every kind is past
    /// the interval are dropped to make room.
    #[test]
    fn the_presentation_read_limiter_stays_bounded_at_its_cap() {
        const KINDS: [&str; 16] = [
            "not_found",
            "permission_denied",
            "disk_full",
            "read_only",
            "io",
            "network",
            "other",
            "database_busy",
            "database_full",
            "database_corrupt",
            "database_read_only",
            "database_cannot_open",
            "database_io",
            "database_constraint",
            "database_schema",
            "database",
        ];
        let start = std::time::Instant::now();
        let at = |secs| start + std::time::Duration::from_secs(secs);
        let entries = |seen: &PresentationWarned| seen.values().map(|kinds| kinds.len()).sum::<usize>();

        // Every item but its last kind, then that kind on an item already held.
        let mut seen = PresentationWarned::new();
        for i in 0..PRESENTATION_WARN_MAX_ITEMS {
            for kind in &KINDS[..15] {
                assert_eq!(
                    presentation_warn_due(&mut seen, &format!("item-{i}"), kind, at(0)),
                    Some(0)
                );
            }
        }
        assert_eq!(seen.len(), PRESENTATION_WARN_MAX_ITEMS);
        assert_eq!(entries(&seen), PRESENTATION_WARN_MAX_ITEMS * 15);
        assert_eq!(
            presentation_warn_due(&mut seen, "item-0", KINDS[15], at(1)),
            Some(0),
            "the next new kind on a held item is logged at once"
        );
        assert_eq!(
            seen.len(),
            PRESENTATION_WARN_MAX_ITEMS,
            "and nothing was dropped for it"
        );
        assert_eq!(entries(&seen), PRESENTATION_WARN_MAX_ITEMS * 15 + 1);
        for i in 1..PRESENTATION_WARN_MAX_ITEMS {
            presentation_warn_due(&mut seen, &format!("item-{i}"), KINDS[15], at(1));
        }
        assert_eq!(
            entries(&seen),
            PRESENTATION_WARN_MAX_ITEMS * KINDS.len(),
            "full: every item, every kind"
        );

        // A new item while every entry is still in its interval: logged, and the map stays bounded.
        assert_eq!(
            presentation_warn_due(&mut seen, "item-new", KINDS[0], at(2)),
            Some(0),
            "a new item at the cap is still logged"
        );
        assert!(seen.len() <= PRESENTATION_WARN_MAX_ITEMS, "{} items", seen.len());
        assert!(seen.contains_key("item-new"));

        // Room is made from expired entries first: an item goes only when every kind it has is
        // past the interval, and an item with one fresh kind keeps that kind.
        let half = PRESENTATION_WARN_INTERVAL.as_secs() / 2;
        let after = PRESENTATION_WARN_INTERVAL.as_secs() + 1;
        let mut seen = PresentationWarned::new();
        for i in 0..PRESENTATION_WARN_MAX_ITEMS / 2 {
            presentation_warn_due(&mut seen, &format!("old-{i}"), KINDS[0], at(0));
        }
        for i in 0..PRESENTATION_WARN_MAX_ITEMS / 2 {
            presentation_warn_due(&mut seen, &format!("fresh-{i}"), KINDS[0], at(half));
        }
        presentation_warn_due(&mut seen, "old-0", KINDS[1], at(half));
        assert_eq!(seen.len(), PRESENTATION_WARN_MAX_ITEMS);
        assert_eq!(
            presentation_warn_due(&mut seen, "item-new", KINDS[0], at(after)),
            Some(0)
        );
        assert_eq!(
            seen.len(),
            PRESENTATION_WARN_MAX_ITEMS / 2 + 2,
            "the fresh items, old-0 for its fresh kind, and the new item"
        );
        assert_eq!(
            seen["old-0"].keys().copied().collect::<Vec<_>>(),
            vec![KINDS[1]],
            "old-0 keeps only its fresh kind"
        );
    }

    /// A failed presentation read after the item read cleanly again starts a new episode: it is
    /// logged at once, not held back by the earlier failure's interval.
    #[test]
    fn a_failed_presentation_read_after_a_clean_one_is_logged_at_once() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed_1694_item(
            &db,
            "half-set-episode",
            FileStatus::Local,
            PERMISSION_READ | PERMISSION_WRITE,
            ItemKind::File,
        );
        let entry = db.get_file("half-set-episode").unwrap().unwrap();
        let failed_lines = |logs: &str| {
            logs.lines()
                .filter(|line| line.contains("presentation_read_failed"))
                .count()
        };
        db.set_held_pair_for_test("half-set-episode", Some(&"c".repeat(32)), None);
        let first = capture_logs(|| {
            for _ in 0..2 {
                let _ = file_entry_payload_for_db(&db, &entry, FP_ROOT_APPLE);
            }
        });
        assert_eq!(failed_lines(&first), 1, "the first episode, once:\n{first}");
        db.set_held_pair_for_test("half-set-episode", None, None);
        assert!(
            db.item_presentation("half-set-episode").is_ok(),
            "precondition: a clean read"
        );
        let _ = file_entry_payload_for_db(&db, &entry, FP_ROOT_APPLE);
        db.set_held_pair_for_test("half-set-episode", Some(&"d".repeat(32)), None);
        let second = capture_logs(|| {
            let _ = file_entry_payload_for_db(&db, &entry, FP_ROOT_APPLE);
        });
        assert_eq!(failed_lines(&second), 1, "a new episode is logged at once:\n{second}");
        assert!(second.contains("held_back=0"), "{second}");
    }

    /// A row whose presentation read keeps failing is built again by the
    /// change feed, every enumeration and every item lookup. It is logged once per item per
    /// interval, and the next line says how many failed reads went unlogged meanwhile.
    #[test]
    fn a_repeated_failed_presentation_read_is_logged_once_per_item_per_interval() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        for id in ["half-set-again", "half-set-other"] {
            seed_1694_item(
                &db,
                id,
                FileStatus::Local,
                PERMISSION_READ | PERMISSION_WRITE,
                ItemKind::File,
            );
            db.set_held_pair_for_test(id, Some(&"b".repeat(32)), None);
        }
        let again = db.get_file("half-set-again").unwrap().unwrap();
        let other = db.get_file("half-set-other").unwrap().unwrap();
        let logs = capture_logs(|| {
            for _ in 0..3 {
                let _ = file_entry_payload_for_db(&db, &again, FP_ROOT_APPLE);
            }
            let _ = file_entry_payload_for_db(&db, &other, FP_ROOT_APPLE);
        });
        let lines: Vec<&str> = logs
            .lines()
            .filter(|line| line.contains("presentation_read_failed"))
            .collect();
        assert_eq!(lines.len(), 2, "one line per item:\n{logs}");
        assert!(
            lines[0].contains("half-set-again") && lines[1].contains("half-set-other"),
            "{logs}"
        );

        assert!(
            lines[0].contains("kind=\"database_schema\""),
            "the kind of failure: {logs}"
        );

        // The limiter itself, on a clock of its own.
        let mut seen = PresentationWarned::new();
        let start = std::time::Instant::now();
        let later = |secs| start + std::time::Duration::from_secs(secs);
        let schema = "database_schema";
        assert_eq!(
            presentation_warn_due(&mut seen, "f", schema, start),
            Some(0),
            "the first read is logged"
        );
        assert_eq!(presentation_warn_due(&mut seen, "f", schema, later(1)), None);
        assert_eq!(presentation_warn_due(&mut seen, "f", schema, later(2)), None);
        assert_eq!(
            presentation_warn_due(&mut seen, "g", schema, later(2)),
            Some(0),
            "another item has its own"
        );
        assert_eq!(
            presentation_warn_due(&mut seen, "f", "database_busy", later(3)),
            Some(0),
            "another kind of failure on the same item is logged at once"
        );
        assert_eq!(presentation_warn_due(&mut seen, "f", "database_busy", later(4)), None);
        let after = PRESENTATION_WARN_INTERVAL.as_secs() + 1;
        assert_eq!(
            presentation_warn_due(&mut seen, "f", schema, later(after)),
            Some(2),
            "after the interval, logged again with the two it held back"
        );
        assert_eq!(presentation_warn_due(&mut seen, "f", schema, later(after + 1)), None);
        presentation_read_ok(&mut seen, "f");
        assert_eq!(
            presentation_warn_due(&mut seen, "f", schema, later(after + 2)),
            Some(0),
            "a clean read ends the episode"
        );
    }

    /// Spec 2026-10-09 §7.3: a queued write whose row cannot be read back is refused, never
    /// answered without an item (for a modify the system reads a nil item as "delete the item
    /// on disk"). Only an ignored write is answered without one. macOS only: other unix builds
    /// keep the item-less reply.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_queued_write_whose_row_cannot_be_read_is_refused() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        let queued = |file_id: Option<&str>| {
            Ok(crate::engine_bridge::FpWrite::plain(
                crate::engine_bridge::FinderWriteOutcome::Queued {
                    op_id: "op-1".into(),
                    file_id: file_id.map(str::to_string),
                    kind: crate::state_db::OperationKind::UploadVersion,
                    ignored: false,
                    message: "queued for encrypted sync".into(),
                },
            ))
        };
        let logs = capture_logs(|| {
            for (op, file_id) in [
                ("create", Some("3f2a9c1e-0000-4000-8000-00000000dead")),
                ("modify", Some("3f2a9c1e-0000-4000-8000-00000000dead")),
                ("modify", None),
            ] {
                let reply = write_outcome_response(op, &db, queued(file_id));
                let IpcResponse::Error { message } = &reply else {
                    panic!("{op} {file_id:?}: an error, never an item-less reply: {reply:?}")
                };
                // What really happens: the write stays queued and uploads; the system does not
                // retry the reply (`cannotSynchronize`) until the item changes again.
                assert!(message.contains("will upload"), "{message}");
                assert!(message.contains("until the file changes again"), "{message}");
                assert!(!message.contains("try again"), "no retry is promised: {message}");
            }
            let reply = write_outcome_response(
                "create",
                &db,
                Ok(crate::engine_bridge::FpWrite::plain(
                    crate::engine_bridge::FinderWriteOutcome::Ignored {
                        message: "ignored".into(),
                    },
                )),
            );
            assert!(
                matches!(
                    reply,
                    IpcResponse::WriteQueued {
                        item: None,
                        ignored: true,
                        ..
                    }
                ),
                "an ignored write is still answered without an item: {reply:?}"
            );
        });
        assert_eq!(logs.matches("row_unreadable").count(), 3, "{logs}");
        assert_eq!(logs.matches("Finder write refused").count(), 3, "{logs}");
    }

    /// Spec 2026-10-09 §7.3 covers creates and modifies: a nil item there deletes the item on
    /// disk. A delete's reply item is never read (the extension discards it), so a trash whose
    /// row is gone when the reply is built (the queue finished it meanwhile) stays a success.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_delete_whose_row_is_gone_at_the_reply_is_not_refused() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        let logs = capture_logs(|| {
            let reply = write_outcome_response(
                "delete",
                &db,
                Ok(crate::engine_bridge::FpWrite::plain(
                    crate::engine_bridge::FinderWriteOutcome::Queued {
                        op_id: "op-trash".into(),
                        file_id: Some("3f2a9c1e-0000-4000-8000-00000000dead".into()),
                        kind: crate::state_db::OperationKind::TrashFile,
                        ignored: false,
                        message: "queued for encrypted sync".into(),
                    },
                )),
            );
            assert!(
                matches!(
                    reply,
                    IpcResponse::WriteQueued {
                        item: None,
                        ignored: false,
                        ..
                    }
                ),
                "a queued delete is a success: {reply:?}"
            );
        });
        assert!(!logs.contains("Finder write refused"), "{logs}");
    }

    #[test]
    fn every_refused_write_is_logged_once_with_its_category_and_never_the_path() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        let (root, staging) = upload_staging_fixture();
        let outside = write_file(&root.path().join("secret-report.txt"), b"x");
        let logs = capture_logs(|| {
            let reply = write_outcome_response(
                "create",
                &db,
                Err(anyhow::anyhow!(
                    "Upload source is not a file: /Users/someone/secret-report.txt"
                )),
            );
            assert!(matches!(reply, IpcResponse::Error { .. }));
            let reply = write_outcome_response(
                "modify",
                &db,
                Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "secret-report.txt").into()),
            );
            assert!(matches!(reply, IpcResponse::Error { .. }));
            // Not a refusal: must not be logged as one.
            let reply = write_outcome_response(
                "create",
                &db,
                Ok(crate::engine_bridge::FpWrite::plain(
                    crate::engine_bridge::FinderWriteOutcome::Ignored {
                        message: "ignored".into(),
                    },
                )),
            );
            assert!(matches!(reply, IpcResponse::WriteQueued { .. }));
            let policy = WriteContentsPolicy::StagingDir(staging.clone());
            assert!(admit_write_contents(&policy, "create", Some(outside.clone())).is_err());
        });
        let refusals: Vec<&str> = logs
            .lines()
            .filter(|line| line.contains("Finder write refused"))
            .collect();
        assert_eq!(refusals.len(), 3, "one warning per refused write, got:\n{logs}");
        assert!(refusals.iter().all(|line| line.contains("WARN")), "{logs}");
        assert!(
            refusals[0].contains("create") && refusals[0].contains("contents_not_a_file"),
            "{logs}"
        );
        assert!(
            refusals[1].contains("modify") && refusals[1].contains("\"io\""),
            "{logs}"
        );
        assert!(refusals[2].contains("outside_staging"), "{logs}");
        assert!(
            !logs.contains("secret-report"),
            "no file name or path may reach the log:\n{logs}"
        );
        assert!(!logs.contains("/Users/"), "no path may reach the log:\n{logs}");
    }

    /// A real `reqwest` error carrying `status`, from a one-shot local server.
    fn http_status_error(status: &str) -> anyhow::Error {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/api/v1/files/file-404", listener.local_addr().unwrap());
        let reply = format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 1024];
            let _ = std::io::Read::read(&mut stream, &mut request);
            std::io::Write::write_all(&mut stream, reply.as_bytes()).unwrap();
        });
        let error = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async { reqwest::get(&url).await.unwrap().error_for_status().unwrap_err() });
        server.join().unwrap();
        anyhow::Error::new(error)
    }

    #[test]
    fn a_failed_hydrate_is_logged_with_its_category_and_never_the_path() {
        let not_found = http_status_error("404 Not Found");
        let logs = capture_logs(|| {
            let reply = hydrate_failure_reply("3f2a9c1e-0000-4000-8000-000000000404", not_found);
            assert!(matches!(reply, IpcResponse::Error { .. }));
            let reply = hydrate_failure_reply(
                "3f2a9c1e-0000-4000-8000-0000000000f0",
                std::io::Error::new(std::io::ErrorKind::PermissionDenied, "/Users/someone/secret-report.txt").into(),
            );
            assert!(matches!(reply, IpcResponse::Error { .. }));
        });
        let failures: Vec<&str> = logs
            .lines()
            .filter(|line| line.contains("Finder hydrate failed"))
            .collect();
        assert_eq!(failures.len(), 2, "one warning per failed hydrate, got:\n{logs}");
        assert!(failures.iter().all(|line| line.contains("WARN")), "{logs}");
        assert!(
            failures[0].contains("3f2a9c1e-0000-4000-8000-000000000404") && failures[0].contains("not_found"),
            "{logs}"
        );
        assert!(
            failures[1].contains("3f2a9c1e-0000-4000-8000-0000000000f0") && failures[1].contains("\"io\""),
            "{logs}"
        );
        assert!(
            !logs.contains("secret-report"),
            "no file name may reach the log:\n{logs}"
        );
        assert!(!logs.contains("/Users/"), "no path may reach the log:\n{logs}");
        assert!(!logs.contains("/api/v1"), "no URL may reach the log:\n{logs}");
    }

    /// Spec 2026-10-09 §7.4: a fetch of a still-queued write whose copy is missing fails with
    /// its own category, and the reply names no path.
    #[test]
    fn a_fetch_whose_queued_copy_is_missing_is_logged_by_its_own_category() {
        let logs = capture_logs(|| {
            let reply = hydrate_failure_reply(
                "3f2a9c1e-0000-4000-8000-0000000000f1",
                anyhow::Error::new(crate::engine_bridge::QueuedCopyMissing),
            );
            let IpcResponse::Error { message } = &reply else {
                panic!("{reply:?}")
            };
            assert!(!message.contains('/'), "{message}");
        });
        let failures: Vec<&str> = logs
            .lines()
            .filter(|line| line.contains("Finder hydrate failed"))
            .collect();
        assert_eq!(failures.len(), 1, "{logs}");
        assert!(
            failures[0].contains("3f2a9c1e-0000-4000-8000-0000000000f1") && failures[0].contains("queued_copy_missing"),
            "{logs}"
        );
    }

    /// Review M5: an accept that fails because the disk or the state database is full is
    /// transient: `WriteRetryLater` with a fixed text, so the system retries the write once there
    /// is room. Any other failure is still an `Error`.
    #[test]
    fn a_full_disk_or_database_at_the_accept_is_retried_later() {
        let dir = tempfile::tempdir().unwrap();
        let db = crate::state_db::StateDb::open(dir.path().join("state.db")).unwrap();
        let disk_full = anyhow::Error::new(std::io::Error::from(std::io::ErrorKind::StorageFull));
        let database_full = anyhow::Error::new(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_FULL),
            None,
        ));
        for (category, error) in [("disk_full", disk_full), ("database_full", database_full)] {
            assert_eq!(
                crate::engine_bridge::error_category(error.as_ref()),
                category,
                "precondition"
            );
            match write_outcome_response("modify", &db, Err(error)) {
                IpcResponse::WriteRetryLater { message } => {
                    assert!(!message.contains('/'), "{category}: a fixed text: {message}")
                }
                other => panic!("{category}: {other:?}"),
            }
        }
        let other = anyhow::Error::new(std::io::Error::from(std::io::ErrorKind::PermissionDenied));
        assert!(
            matches!(
                write_outcome_response("modify", &db, Err(other)),
                IpcResponse::Error { .. }
            ),
            "any other failure is still an error"
        );
    }

    /// U45: the extension is told to retry, as for a copy an engine-start sweep removed: for a save, a rename or move,
    /// a folder create and a delete alike.
    #[test]
    fn a_closed_accept_answers_write_retry_later() {
        let dir = tempfile::tempdir().unwrap();
        let db = crate::state_db::StateDb::open(dir.path().join("state.db")).unwrap();
        let error = anyhow::Error::new(crate::engine_bridge::AcceptsClosed);
        assert_eq!(write_refusal_category(&error), "accepts_closed");
        let ops = ["create", "modify", "delete"];
        let logs = capture_logs(|| {
            for op in ops {
                let response =
                    write_outcome_response(op, &db, Err(anyhow::Error::new(crate::engine_bridge::AcceptsClosed)));
                assert!(
                    matches!(response, IpcResponse::WriteRetryLater { .. }),
                    "{op}: {response:?}"
                );
            }
        });
        // Each refusal leaves one warning with its operation and its category.
        let refusals: Vec<&str> = logs
            .lines()
            .filter(|line| line.contains("Finder write refused"))
            .collect();
        assert_eq!(refusals.len(), ops.len(), "one refusal logged per write:\n{logs}");
        for (line, op) in refusals.iter().zip(ops) {
            assert!(
                line.contains("WARN")
                    && line.contains(&format!("op=\"{op}\""))
                    && line.contains("reason=\"accepts_closed\""),
                "{op}: {line}"
            );
        }
    }
