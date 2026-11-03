//! `refusals_3` engine tests, split out of `engine_bridge.rs`.

use super::*;
    /// Spec §10.2 with §6.3.1: an earlier build could queue a replace with no base. Once marked,
    /// it carries a write id, so the `init` guard parks it `base_unknown` at its first attempt,
    /// with its bytes and its base left as they are, and nothing is sent: no replace of a known
    /// file goes out without a base (§10.5).
    #[tokio::test]
    async fn an_earlier_builds_modify_without_a_base_parks_base_unknown_and_sends_nothing() {
        let (dir, sync_root, server, bridge) = rule3_setup([103u8; 32]);
        seed_uploaded_row(&bridge, &server, "no-base");
        // The way an earlier build queued it: through the shared entry point, no write id, no base.
        let contents = dir.path().join("no-base.txt");
        std::fs::write(&contents, b"an earlier build's save, no base").unwrap();
        bridge
            .queue_finder_modify(finder_file_target(Some("no-base"), "notes.txt", &contents, None))
            .unwrap();
        let op = bridge.db.list_operations_for_file("no-base").unwrap().remove(0);
        assert_eq!(op.base_version, None, "precondition: queued without a base");
        assert_eq!(bridge.db.mark_earlier_build_uploads().unwrap(), 1);

        let logs = capture_logs_async(async {
            drain_upload_queue(&bridge, &sync_root).await;
        })
        .await;
        let state = server.finish();

        assert_eq!(state.requests.len(), 0, "nothing is sent: {:?}", state.requests);
        let parked = bridge.db.get_operation(&op.op_id).unwrap().expect("kept in the queue");
        assert_eq!(parked.attempts, parked.max_attempts, "parked at its first attempt");
        assert_eq!(parked.last_error.as_deref(), Some("base_unknown"));
        assert_eq!(parked.base_version, None, "its base is left as it is");
        assert_eq!(
            std::fs::read(parked.payload_path.as_deref().unwrap()).unwrap(),
            b"an earlier build's save, no base",
            "its bytes are kept"
        );
        assert_eq!(
            logs.lines()
                .filter(|line| line.contains("upload parked")
                    && line.contains(&op.op_id)
                    && line.contains("base_unknown"))
                .count(),
            1,
            "{logs}"
        );
    }

    /// Spec §10.2 holds only while every upload a Mac queues without a write id comes from an
    /// earlier build. The File Provider's content writes are queued through the entry points that
    /// mint (`queue_file_provider_*`, §8.8); the shared entry points mint nothing. This census reads
    /// every source of a module a Mac compiles, as a Mac compiles it, and allows each call of a
    /// shared entry point only where it cannot queue content on a Mac. A new call fails here: route
    /// a macOS content write through the minting entry points, or list the call below with the
    /// reason it queues no content.
    #[test]
    fn no_macos_path_queues_content_through_the_entry_points_that_mint_nothing() {
        use crate::finder_setup_command_tests::macos_compiled;
        const SHARED: [&str; 4] = [
            "queue_finder_create_from",
            "queue_finder_modify_from",
            "queue_finder_create",
            "queue_finder_modify",
        ];
        // (file, the call, the function it is in, lines between that function's start and the
        // call, why it queues no content on a Mac)
        let allowed: [(&str, &str, &str, &[&str], &str); 8] = [
            (
                "engine_bridge/finder_create.rs",
                "self.queue_finder_create_from(target, None)",
                "pub fn queue_finder_create(",
                &[],
                "a wrapper: its callers are counted under its own name",
            ),
            (
                "engine_bridge/finder_edit.rs",
                "self.queue_finder_modify_from(target, None)",
                "pub fn queue_finder_modify(",
                &[],
                "a wrapper: its callers are counted under its own name",
            ),
            (
                "engine_bridge/finder_create.rs",
                "return self.queue_finder_create_from(target, contents).map(FpWrite::plain);",
                "pub fn queue_file_provider_create_from(",
                &["if target.kind == FinderWriteItemKind::Folder {"],
                "a folder: a create_folder op",
            ),
            (
                "engine_bridge/finder_edit.rs",
                "return self.queue_finder_modify_from(target, contents).map(|outcome| FpWrite {",
                "pub fn queue_file_provider_modify_from(",
                &["let Some(contents_path) = target.contents_path.clone() else {"],
                "no contents: a rename or move op",
            ),
            (
                "engine_bridge/local_layout.rs",
                "match self.queue_finder_create(target) {",
                "pub fn ensure_local_folder(",
                &["kind: FinderWriteItemKind::Folder,", "contents_path: None,"],
                "a folder",
            ),
            (
                "watcher.rs",
                "match bridge.queue_finder_create(target) {",
                "pub(crate) fn dispatch_local_create_verdict(",
                &[],
                "the upload driver, which a Mac never starts (asserted below)",
            ),
            (
                "watcher.rs",
                "return match bridge.queue_finder_modify(target) {",
                "pub(crate) fn dispatch_local_create_verdict(",
                &[],
                "the upload driver's edit of a synced file (task 1640), which a Mac never starts (asserted below)",
            ),
            (
                "watcher.rs",
                "None => bridge.queue_finder_modify(modify),",
                "pub(crate) async fn handle_rename(",
                &[],
                "the upload driver, which a Mac never starts (asserted below)",
            ),
        ];
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let read = |path: &Path| std::fs::read_to_string(path).unwrap().replace("\r\n", "\n");
        // The top-level modules a Mac compiles: their `mod` lines survive in `lib.rs`.
        let lib = macos_compiled(&read(&src.join("lib.rs")));
        let compiled = |module: &str| {
            lib.lines().map(str::trim).any(|line| {
                line == format!("mod {module};")
                    || line == format!("pub mod {module};")
                    || line == format!("mod {module} {{")
            })
        };
        let is_call = |line: &str, name: &str| {
            let needle = format!("{name}(");
            !line.contains(&format!("fn {needle}"))
                && line.match_indices(&needle).any(|(at, _)| {
                    !line[..at]
                        .chars()
                        .next_back()
                        .is_some_and(|c| c.is_alphanumeric() || c == '_')
                })
        };
        let starts_fn = |line: &str| {
            [
                "fn ",
                "pub fn ",
                "pub(crate) fn ",
                "async fn ",
                "pub async fn ",
                "pub(crate) async fn ",
            ]
            .iter()
            .any(|prefix| line.starts_with(prefix))
        };
        let mut sources = Vec::new();
        let mut stack = vec![src.clone()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|ext| ext == "rs") {
                    let rel = path.strip_prefix(&src).unwrap().to_string_lossy().replace('\\', "/");
                    if rel == "tests.rs"
                        || rel.ends_with("/tests.rs")
                        || rel.ends_with("_tests.rs")
                        || rel.split('/').any(|part| part == "tests")
                    {
                        continue; // test-only files are not production
                    }
                    let module = rel.split('/').next().unwrap().trim_end_matches(".rs").to_string();
                    if matches!(module.as_str(), "lib" | "main") || compiled(&module) {
                        sources.push((rel, macos_compiled(&read(&path))));
                    }
                }
            }
        }
        assert!(
            sources.iter().any(|(rel, _)| rel == "ipc_socket.rs")
                && !sources.iter().any(|(rel, _)| rel.starts_with("windows_cf/")),
            "the module filter sees what a Mac compiles"
        );
        let mut matched = [0usize; 8];
        for (rel, mac) in &sources {
            let lines: Vec<&str> = mac.lines().map(str::trim).collect();
            for (i, line) in lines.iter().enumerate() {
                if line.starts_with("//") || !SHARED.iter().any(|name| is_call(line, name)) {
                    continue;
                }
                let start = (0..i).rev().find(|&j| starts_fn(lines[j])).unwrap_or(0);
                let Some(k) = allowed.iter().position(|(file, call, function, guards, _)| {
                    file == rel
                        && call == line
                        && lines[start].starts_with(function)
                        && guards.iter().all(|guard| lines[start..i].contains(guard))
                }) else {
                    panic!(
                        "{rel}: `{line}` (in `{}`) calls an entry point that mints nothing, in code a Mac \
                         compiles: queue a macOS content write through `queue_file_provider_*`",
                        lines[start]
                    );
                };
                matched[k] += 1;
            }
        }
        for (k, (file, call, _, _, why)) in allowed.iter().enumerate() {
            assert_eq!(matched[k], 1, "{file}: `{call}` ({why}) is found once");
        }
        // The upload driver starts only through `watcher::spawn`, and nothing a Mac compiles calls it.
        let starts: Vec<String> = sources
            .iter()
            .filter(|(rel, _)| rel != "watcher.rs")
            .flat_map(|(rel, mac)| {
                mac.lines()
                    .map(str::trim)
                    .filter(|line| !line.starts_with("//") && line.contains("watcher::spawn("))
                    .map(move |line| format!("{rel}: {line}"))
            })
            .collect();
        assert!(starts.is_empty(), "a Mac starts the upload driver: {starts:?}");
        assert!(
            read(&src.join("runner.rs")).contains("crate::watcher::spawn("),
            "the upload driver keeps its start on Windows"
        );
    }

    // ── Rule 7: every new log line is captured and clean (spec §11, M5) ──────

    /// The planted name and the staging path must never reach a log line.
    pub(super) const PLANTED: &str = "Quarterly secret plan.txt";

    /// The first line carrying `message`: it has every field in `fields`, and neither the
    /// planted name, a URL, a home path nor the test's directory. In a test build the staging
    /// folder is under the build's `target/` (`test_sandbox`), so `/Users/` covers it on a Mac.
    pub(super) fn assert_clean_line(logs: &str, message: &str, fields: &[&str], dir: &Path) -> String {
        let line = logs
            .lines()
            .find(|line| line.contains(message))
            .unwrap_or_else(|| panic!("no `{message}` line:\n{logs}"))
            .to_string();
        for field in fields {
            assert!(
                line.contains(&format!(" {field}=")),
                "`{message}` lacks `{field}`: {line}"
            );
        }
        let dir = dir.to_string_lossy();
        // The test build's staging folder, and the per-process sandbox it lives in. On a Mac
        // `/Users/` covers them too; on Linux and Windows only these entries catch a staged
        // copy's path.
        let staging = default_finder_staging_root().expect("the test build's staging root");
        let staging = staging.to_string_lossy();
        let sandbox = crate::test_sandbox::dir("data").expect("the unit-test sandbox");
        let sandbox = sandbox.parent().expect("the sandbox root").to_string_lossy();
        for leak in [
            "Quarterly",
            "secret",
            "/api/v1",
            "127.0.0.1",
            "/Users/",
            dir.as_ref(),
            staging.as_ref(),
            sandbox.as_ref(),
        ] {
            assert!(!line.contains(leak), "`{message}` leaks `{leak}`: {line}");
        }
        line
    }

    /// T37: a write to an id with no row and no alias is refused with its reason and the
    /// requested id (spec §11, "Unknown id").
    #[cfg(unix)] // names `crate::ipc_socket::write_outcome_response`
    #[tokio::test]
    async fn log_finder_write_refused_unknown_item() {
        let (dir, _sync_root, server, bridge) = rule3_setup([100u8; 32]);
        let requested = "3f2a9c1e-0000-4000-8000-00000000beef";
        let contents = dir.path().join(PLANTED);
        std::fs::write(&contents, b"edit").unwrap();
        let logs = capture_logs_async(async {
            let result = bridge.queue_file_provider_modify(finder_file_target(
                Some(requested),
                PLANTED,
                &contents,
                Some("1".into()),
            ));
            let _ = crate::ipc_socket::write_outcome_response("modify", &bridge.db, result);
        })
        .await;
        let line = assert_clean_line(&logs, "Finder write refused", &["op", "reason", "id"], dir.path());
        assert!(
            line.contains("\"unknown_item\"") && line.contains(&format!(" id={requested}")),
            "{line}"
        );
        drop(server.finish());
    }

    /// T37: an `init` refused with 409 and retried names its class (T25's scenario).
    #[tokio::test]
    async fn log_upload_refused_with_its_class() {
        let (dir, sync_root, server, bridge) = rule3_setup([102u8; 32]);
        seed_uploaded_row(&bridge, &server, "busy");
        server
            .state
            .lock()
            .unwrap()
            .conflict_init_once
            .insert("busy".into(), crate::api_client::IN_PROGRESS_MESSAGE.into());
        fp_save(&bridge, dir.path(), "busy", PLANTED, b"edit", "1");
        let logs = capture_logs_async(async {
            drain_upload_queue(&bridge, &sync_root).await;
        })
        .await;
        let line = assert_clean_line(
            &logs,
            "upload refused by the server (409 Conflict); will retry",
            &["op_id", "file_id", "attempt", "max_attempts", "class"],
            dir.path(),
        );
        assert!(line.contains("\"in_progress\"") && line.contains("busy"), "{line}");
        assert_eq!(server.finish().latest_plaintext("busy", [102u8; 32]), b"edit");
    }

    /// The scenario of T23 (`i4_a_doomed_earlier_write_never_blocks_hours`), named `PLANTED`:
    /// W's staged copy is gone, so W parks and N takes over. Returns W's op id and the logs.
    async fn doomed_predecessor_logs(master_key: [u8; 32]) -> (tempfile::TempDir, String, String) {
        let (dir, sync_root, server, bridge) = rule3_setup(master_key);
        seed_uploaded_row(&bridge, &server, "doomed");
        let w = fp_save(&bridge, dir.path(), "doomed", PLANTED, b"W", "1");
        fp_save(
            &bridge,
            dir.path(),
            "doomed",
            PLANTED,
            b"W N",
            w.token.as_deref().unwrap(),
        );
        let w_op = bridge.db.list_operations_for_file("doomed").unwrap().remove(0);
        std::fs::remove_file(w_op.payload_path.as_deref().unwrap()).unwrap();
        let logs = capture_logs_async(async {
            drain_upload_queue(&bridge, &sync_root).await;
        })
        .await;
        let state = server.finish();
        assert_eq!(state.latest_plaintext("doomed", master_key), b"W N", "N landed");
        (dir, w_op.op_id, logs)
    }

    /// T37: the predecessor's park names its reason (T23).
    #[tokio::test]
    async fn log_parked_with_its_reason() {
        let (dir, w_op_id, logs) = doomed_predecessor_logs([103u8; 32]).await;
        let line = assert_clean_line(
            &logs,
            "upload parked with its bytes kept in the queue",
            &["op_id", "file_id", "reason"],
            dir.path(),
        );
        assert!(
            line.contains("\"payload_missing\"") && line.contains(&w_op_id),
            "{line}"
        );
    }

    /// T37: the successor's hand-over names both ops (T23).
    #[tokio::test]
    async fn log_took_over() {
        let (dir, w_op_id, logs) = doomed_predecessor_logs([104u8; 32]).await;
        let line = assert_clean_line(
            &logs,
            "queued write took over a parked one",
            &["op_id", "file_id", "parked_op_id"],
            dir.path(),
        );
        assert!(line.contains(&format!("parked_op_id={w_op_id}")), "{line}");
    }

    /// T37: a modify under P after its create landed resolves through the alias (T15). New
    /// creates no longer swap ids (SF-0001 L13), so the alias is seeded as an older client
    /// left it.
    #[tokio::test]
    async fn log_alias_resolved() {
        let (dir, _sync_root, server, bridge) = rule3_setup([105u8; 32]);
        let s = "3f2a9c1e-0000-4000-8000-0000000000c5".to_string();
        let p = "3f2a9c1e-0000-4000-8000-0000000000c6".to_string();
        seed_uploaded_row(&bridge, &server, &s);
        bridge.db.insert_alias_for_test(&p, &s, now_secs());
        let logs = capture_logs_async(async {
            fp_save(
                &bridge,
                dir.path(),
                &p,
                PLANTED,
                b"created bytes, edited",
                &held_content_version(&bridge, &s),
            );
        })
        .await;
        let line = assert_clean_line(
            &logs,
            "provisional id resolved to the server id",
            &["request", "provisional_id", "file_id"],
            dir.path(),
        );
        assert!(
            line.contains("\"modify\"")
                && line.contains(&format!("provisional_id={p}"))
                && line.contains(&format!("file_id={s}")),
            "{line}"
        );
        drop(server.finish());
    }

    /// T37: a delete under P that does not name S's held token keeps S (T58). New creates no
    /// longer swap ids (SF-0001 L13), so the alias is seeded as an older client left it.
    #[tokio::test]
    async fn log_alias_delete_kept() {
        let (dir, sync_root, server, bridge) = rule3_setup([106u8; 32]);
        let s = "3f2a9c1e-0000-4000-8000-0000000000c7".to_string();
        let p = "3f2a9c1e-0000-4000-8000-0000000000c8".to_string();
        seed_uploaded_row(&bridge, &server, &s);
        bridge.db.insert_alias_for_test(&p, &s, now_secs());
        let logs = capture_logs_async(async {
            bridge.queue_finder_delete(&p, Some("1".into())).unwrap();
            drain_upload_queue(&bridge, &sync_root).await;
        })
        .await;
        let line = assert_clean_line(
            &logs,
            "delete of a provisional id not applied to the server file",
            &["provisional_id", "file_id"],
            dir.path(),
        );
        assert!(
            line.contains(&format!("provisional_id={p}")) && line.contains(&format!("file_id={s}")),
            "{line}"
        );
        assert!(server.finish().trashes.is_empty(), "S kept");
    }

    /// T37: a queued write waiting for the version takes the one the snapshot reports (T11,
    /// with `apply_snapshot` called directly). The node carries the planted name.
    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn log_base_from_snapshot() {
        let master_key = [107u8; 32];
        let (dir, sync_root, server, bridge) = rule3_setup(master_key);
        seed_version_zero_row(&bridge, &server, "legacy", FileStatus::Local);
        bridge.db.set_sync_cursor(0).unwrap();
        fp_save(&bridge, dir.path(), "legacy", PLANTED, b"edit", "0");
        drain_upload_queue(&bridge, &sync_root).await;
        assert!(
            server.state.lock().unwrap().inits.is_empty(),
            "precondition: no init until the version is known"
        );
        let op = bridge.db.list_operations_for_file("legacy").unwrap().remove(0);
        let snapshot = crate::api_client::SyncSnapshot {
            seq_id: 1,
            nodes: vec![node(&master_key, "legacy", PLANTED, 1, false)],
        };
        let logs = capture_logs_async(async {
            apply_snapshot(&bridge, &sync_root, &snapshot, now_secs(), now_secs(), &mut Vec::new()).unwrap();
        })
        .await;
        let line = assert_clean_line(
            &logs,
            "queued write based on the version the snapshot reported",
            &["op_id", "file_id", "base_version"],
            dir.path(),
        );
        assert!(line.contains(&op.op_id) && line.contains("base_version=1"), "{line}");
        drop(server.finish());
    }

