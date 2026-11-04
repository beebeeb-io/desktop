    use super::*;
    use crate::config::RealConfigFs;
    use crate::engine_bridge::FinderStagingBases;
    use crate::sign_out::copy::{
        E1_UNSENT_CHANGES, E2_UNSENT_COPY_FAILED, E3_UNSENT_COUNT_FAILED, UNSENT_CHANGES_CODE, UNSENT_COPY_FAILED_CODE,
        UNSENT_COUNT_FAILED_CODE, coded,
    };
    use crate::sign_out::keep::{Acknowledge, KeepInputs, KeptSet, keep_unsent};
    use crate::sign_out::kept_names::{APP_BUNDLE_ID, kept_root};
    use crate::sign_out::kept_record::{saved_kept_unsent_at, unfinished_at};
    use crate::sign_out::kept_writer::SystemClone;
    use crate::sign_out::snapshot::AskSnapshot;
    use crate::sign_out::waiting::WaitingChanges;
    use std::path::Path;

    /// A state dir with `state.db` where the sign-out's purge looks for it, and a config file of its own.
    struct PurgeFixture {
        dir: tempfile::TempDir,
        db: state_db::StateDb,
    }

    impl PurgeFixture {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let db = state_db::StateDb::open(state_paths::state_db_path_from_state_dir(dir.path())).unwrap();
            Self { dir, db }
        }
        fn config(&self) -> PathBuf {
            self.dir.path().join("desktop.toml")
        }
        fn staged(&self, name: &str) -> PathBuf {
            self.dir.path().join("staging").join(name)
        }
        fn write(&self, op_id: &str, file_id: &str, bytes: &[u8]) -> PathBuf {
            let staged = self.staged(op_id);
            std::fs::create_dir_all(staged.parent().unwrap()).unwrap();
            std::fs::write(&staged, bytes).unwrap();
            self.db
                .hold_lock_for_test()
                .execute_batch(&format!(
                    "INSERT OR IGNORE INTO files (file_id, path) VALUES ('{file_id}', '{file_id}.txt');
                     INSERT INTO operation_queue (op_id, kind, file_id, payload_path, write_id, write_origin)
                     VALUES ('{op_id}', 'upload_version', '{file_id}', '{}', '{op_id}', 'minted');
                     INSERT INTO staged_payloads (path, completed) VALUES ('{}', 0);",
                    staged.display(),
                    staged.display()
                ))
                .unwrap();
            staged
        }
        fn keep(&self) -> KeptSet {
            self.keep_into(&self.config())
        }
        /// The keep step, recording the folder in `config`.
        fn keep_into(&self, config: &Path) -> KeptSet {
            let root = self.dir.path().join("Documents").join("Changes not sent");
            keep_unsent(KeepInputs {
                db: &self.db,
                acknowledge: Acknowledge::Nothing,
                root: &root,
                event_name: "event".into(),
                config_path: config,
                clone: &SystemClone,
                config_fs: &RealConfigFs,
            })
            .unwrap()
        }
        /// `body` with the app's `desktop.toml` (`DesktopConfig::path()`, the file the sign-out's purge clears the marker
        /// in) in this fixture's own directory, never in the sandbox other tests share. `body` gets that file's path.
        fn with_own_config<R>(&self, body: impl FnOnce(&Path) -> R) -> R {
            let dir = self.dir.path().join("config");
            std::fs::create_dir_all(&dir).unwrap();
            crate::config::CONFIG_DIR_OVERRIDE.sync_scope(dir, || body(&DesktopConfig::path().unwrap()))
        }
        fn queued(&self) -> usize {
            self.db.waiting_rows().unwrap().len()
        }
    }

    /// Everything `tracing` emits on this thread while `body` runs, as text.
    fn capture_logs(body: impl FnOnce()) -> String {
        #[derive(Clone)]
        struct Capture(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
        impl std::io::Write for Capture {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let buffer = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let writer = Capture(buffer.clone());
        // tracing caches each callsite's interest for the whole process. While only ONE dispatcher is alive,
        // tracing-core takes that interest from the registering thread's own default, so a callsite first hit by another
        // test thread, which has no subscriber, is cached as "never" and this capture misses its events. A second live
        // dispatcher keeps registration on the all-dispatchers path.
        let _second_dispatcher = tracing::Dispatch::new(tracing_subscriber::registry());
        let subscriber = tracing_subscriber::fmt()
            .with_writer(move || writer.clone())
            .with_ansi(false)
            .with_max_level(tracing::Level::TRACE)
            .finish();
        tracing::subscriber::with_default(subscriber, body);
        let bytes = buffer.lock().unwrap().clone();
        String::from_utf8(bytes).unwrap()
    }

    /// U12 on the release layout (spec §7.1, amended for `184ac962`).
    #[test]
    fn the_kept_folder_is_out_of_every_purges_reach() {
        let container = Path::new("/Users/a/Library/Containers/io.beebeeb.app/Data");
        let data = container.join("Library").join("Application Support");
        let cache = container.join("Library").join("Caches");
        let staging = FinderStagingBases {
            data: Some(data.clone()),
            cache: Some(cache.clone()),
            temp: container.join("tmp"),
        };
        let hydrate = PathBuf::from("/Users/a/Library/Group Containers/R8352WDJJR.io.beebeeb.app.fileprovider/hydrate");
        let roots = disposable_cache_roots_in(container.join("tmp"), Some(cache.clone()), Some(hydrate), &staging);
        let staging_list = staging.candidates();
        // The lists' shape first: a whole data dir in either list would otherwise show only as the unsandboxed root
        // being under it.
        let durable = data.join("beebeeb").join("finder-writes");
        assert!(
            roots.contains(&durable),
            "the purge's allow-list holds the durable staging root"
        );
        assert!(staging_list.contains(&durable), "the orphan sweep holds it too");
        assert!(
            !roots.contains(&data) && !staging_list.contains(&data),
            "never the whole data dir"
        );
        assert!(
            staging_list.contains(&cache.join("beebeeb").join("finder-writes")),
            "the old Caches root stays"
        );
        let kept = [
            kept_root(Some(&container.join("Documents")), Some(&data), APP_BUNDLE_ID).unwrap(),
            kept_root(Some(Path::new("/Users/a/Documents")), Some(&data), APP_BUNDLE_ID).unwrap(),
        ];
        for kept in &kept {
            for root in roots.iter().chain(staging_list.iter()) {
                assert!(
                    !kept.starts_with(root),
                    "{} is under {}",
                    kept.display(),
                    root.display()
                );
            }
        }

        let production = super::finder_setup_command_tests::production_source();
        let wrapper = super::finder_setup_command_tests::body_between(
            &production,
            "fn disposable_cache_roots() -> Vec<PathBuf> {",
            "\n}\n",
        );
        assert_eq!(wrapper.matches("disposable_cache_roots_in(").count(), 1, "{wrapper}");
        assert!(
            !wrapper.contains("roots.push") && !wrapper.contains("vec!["),
            "the wrapper builds nothing itself: {wrapper}"
        );
        // The whole body is the one call: a root added after the core (the whole data dir, say) would put the kept
        // folder's fallback under the purge, and the core's own test would not see it.
        let body: String = wrapper
            .split_once('{')
            .map_or("", |(_, body)| body)
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        assert_eq!(
            body.replace(",)", ")"),
            "disposable_cache_roots_in(std::env::temp_dir(),dirs::cache_dir(),macos_hydrate_cache_root(),\
             &engine_bridge::FinderStagingBases::current())",
            "the wrapper is one call to its core and nothing else: {wrapper}"
        );
    }

    /// U14: a waiting write, then an other change nobody acknowledged, each queued after the keep step.
    #[test]
    fn the_purge_guard_runs_before_any_file_is_deleted() {
        let f = PurgeFixture::new();
        let a = f.write("wa", "a", b"a");
        let kept = f.keep();
        let late = f.write("late", "late", b"late"); // the test seam: a waiting write after the keep step
        let error = purge_local_state_files_as(&f.db, None, &[], None, Some(&kept))
            .err()
            .expect("the guard refuses");
        assert_eq!(
            error,
            coded(UNSENT_COPY_FAILED_CODE, E2_UNSENT_COPY_FAILED),
            "the refusal is E2 with its code"
        );
        assert!(late.exists() && a.exists(), "no staged copy was touched");
        let folder = kept.folder.clone().unwrap();
        assert!(folder.join("a.txt").exists(), "the kept folder stays");
        assert_eq!(
            saved_kept_unsent_at(&f.config()).unwrap(),
            Some(folder.to_string_lossy().into_owned())
        );
        assert_eq!(f.queued(), 2, "no queue row is gone");

        // A rename queued after the keep step: an other change, and nobody was asked about it.
        let f = PurgeFixture::new();
        let a = f.write("wa", "a", b"a");
        let kept = f.keep();
        f.db.hold_lock_for_test()
            .execute(
                "INSERT INTO operation_queue (op_id, kind, file_id, target_path) VALUES ('rn', 'rename_file', 'a', 'b.txt')",
                [],
            )
            .unwrap();
        let error = purge_local_state_files_as(&f.db, None, &[], None, Some(&kept))
            .err()
            .expect("the guard refuses an other change nobody acknowledged");
        assert_eq!(error, coded(UNSENT_COPY_FAILED_CODE, E2_UNSENT_COPY_FAILED));
        assert!(a.exists(), "no staged copy was touched");
        assert!(kept.folder.unwrap().join("a.txt").exists(), "the kept folder stays");
        assert_eq!(f.queued(), 2, "no queue row is gone");
    }

    /// Spec §2: an upload whose completion is recorded is not a waiting write. The keep step does not count it, so the
    /// guard does not ask for it: it neither holds the purge nor survives it.
    #[test]
    fn a_completed_upload_does_not_hold_the_purge() {
        let f = PurgeFixture::new();
        let a = f.write("wa", "a", b"a");
        let done = f.write("done", "d", b"done");
        f.db.hold_lock_for_test()
            .execute(
                "INSERT INTO upload_resume (op_id, payload_path, payload_size, payload_mtime_ns, upload_session_id,
                   server_file_id, object_version_id, chunk_size_bytes, chunk_count, completed_version)
                 VALUES ('done', ?1, 4, 0, 's', 'd', 'v', 4, 1, 2)",
                rusqlite::params![done.to_str().unwrap()],
            )
            .unwrap();
        let kept = f.keep();
        assert!(
            !kept.keeps("done"),
            "the keep step does not account for a completed upload"
        );
        purge_local_state_files_as(&f.db, None, &[], None, Some(&kept))
            .expect("a completed upload does not hold the purge");
        assert!(!a.exists() && !done.exists(), "both copies are removed");
        assert_eq!(f.queued(), 0);
    }

    /// U39 (a) and (b); (c) is the async test below.
    #[test]
    fn a_purge_failure_after_the_keep_step_spares_waiting_copies_and_is_named_next_time() {
        // (a) A removal fails on an orphan: every waiting copy is still there.
        let f = PurgeFixture::new();
        let a = f.write("wa", "a", b"a");
        let b = f.write("wb", "b", b"b");
        let orphan = f.staged("orphan");
        std::fs::write(&orphan, b"orphan").unwrap();
        f.db.hold_lock_for_test()
            .execute(
                "INSERT INTO staged_payloads (path, completed) VALUES (?1, 1)",
                rusqlite::params![orphan.to_str().unwrap()],
            )
            .unwrap();
        let kept = f.keep();
        let fail_on_orphan = |path: &Path| {
            if path == orphan {
                Err(std::io::Error::other("injected"))
            } else {
                std::fs::remove_file(path)
            }
        };
        assert!(purge_local_state_files_with(&f.db, None, &[], None, Some(&kept), &fail_on_orphan).is_err());
        assert!(a.exists() && b.exists(), "the waiting writes' copies go last");
        assert_eq!(f.queued(), 2);
        let folder = kept.folder.unwrap().to_string_lossy().into_owned();
        assert_eq!(
            saved_kept_unsent_at(&f.config()).unwrap().as_deref(),
            Some(folder.as_str()),
            "the kept folder is recorded"
        );
        assert_eq!(
            unfinished_at(&f.config()).unwrap(),
            [folder],
            "saved by the keep step, before the purge"
        );

        // (b) A removal fails on the second waiting copy: the first is gone, and the next count shows it missing.
        let f = PurgeFixture::new();
        let a = f.write("wa", "a", b"a");
        let b = f.write("wb", "b", b"b");
        let kept = f.keep();
        let fail_on_b = |path: &Path| {
            if path == b {
                Err(std::io::Error::other("injected"))
            } else {
                std::fs::remove_file(path)
            }
        };
        assert!(purge_local_state_files_with(&f.db, None, &[], None, Some(&kept), &fail_on_b).is_err());
        assert!(!a.exists() && b.exists());
        let counts = crate::sign_out::waiting::WaitingChanges::read(&f.db).unwrap().counts();
        assert_eq!(counts.missing_files, 1);
        assert_eq!(unfinished_at(&f.config()).unwrap().len(), 1);
    }

    /// U39 (c): a later sign-out that completes clears the list and never touches 1882's key.
    #[tokio::test]
    async fn a_completed_purge_clears_the_unfinished_list() {
        let f = PurgeFixture::new();
        f.write("wa", "a", b"a");
        let config_dir = f.dir.path().join("config");
        std::fs::create_dir_all(&config_dir).unwrap();
        crate::config::CONFIG_DIR_OVERRIDE
            .scope(config_dir, async {
                let config = DesktopConfig::path().unwrap();
                crate::finder_removal::remember_kept_folder_at(&config, "/macos/kept").unwrap();
                let root = f.dir.path().join("Documents").join("Changes not sent");
                let kept = keep_unsent(KeepInputs {
                    db: &f.db,
                    acknowledge: Acknowledge::Nothing,
                    root: &root,
                    event_name: "event".into(),
                    config_path: &config,
                    clone: &SystemClone,
                    config_fs: &RealConfigFs,
                })
                .unwrap();
                assert_eq!(unfinished_at(&config).unwrap().len(), 1);
                purge_local_data_for_sign_out(Ok(f.dir.path().to_path_buf()), None, false, Some(&kept)).unwrap();
                let cfg = DesktopConfig::load_from(&config).unwrap();
                assert!(
                    cfg.kept_unsent_unfinished.is_empty(),
                    "the rows are cleared, so nothing can look missing"
                );
                assert_eq!(cfg.kept_unsynced_folder.as_deref(), Some("/macos/kept"));
                assert!(cfg.kept_unsent_folder.is_some(), "the row stays until dismissed");
            })
            .await;
    }

    /// W13: only a purge that cleared the rows clears the unfinished list. A sign-out purge that stops, on a file it
    /// cannot remove or at the guard, returns its own error and leaves the list and the row naming the kept folder.
    /// (A purge that completes clears the list: `a_completed_purge_clears_the_unfinished_list`.)
    #[test]
    fn a_sign_out_purge_that_stops_leaves_the_unfinished_list() {
        // A file that is not a waiting write's copy cannot be removed (a directory where a file is expected).
        let f = PurgeFixture::new();
        let a = f.write("wa", "a", b"a");
        let stuck = f.staged("stuck");
        std::fs::create_dir_all(stuck.join("inner")).unwrap();
        f.db.hold_lock_for_test()
            .execute(
                "INSERT INTO staged_payloads (path, completed) VALUES (?1, 1)",
                rusqlite::params![stuck.to_str().unwrap()],
            )
            .unwrap();
        f.with_own_config(|config| {
            let kept = f.keep_into(config);
            let folder = kept.folder.clone().unwrap().to_string_lossy().into_owned();
            let error = purge_local_data_for_sign_out(Ok(f.dir.path().to_path_buf()), None, false, Some(&kept))
                .expect_err("the purge stops");
            assert!(
                error.starts_with(SIGN_OUT_PURGE_FAILED) && error.contains("could not be removed"),
                "{error}"
            );
            assert!(a.exists(), "the waiting write's copy goes last, so it is still there");
            assert_eq!(f.queued(), 1, "no row is cleared");
            assert_eq!(
                unfinished_at(config).unwrap(),
                [folder.as_str()],
                "a purge that failed leaves the list"
            );
            assert_eq!(saved_kept_unsent_at(config).unwrap(), Some(folder), "and the row");
        });

        // The guard refuses a waiting write queued after the keep step.
        let f = PurgeFixture::new();
        f.write("wa", "a", b"a");
        f.with_own_config(|config| {
            let kept = f.keep_into(config);
            let folder = kept.folder.clone().unwrap().to_string_lossy().into_owned();
            let late = f.write("late", "late", b"late");
            let error = purge_local_data_for_sign_out(Ok(f.dir.path().to_path_buf()), None, false, Some(&kept))
                .expect_err("the guard refuses");
            assert_eq!(
                error,
                coded(UNSENT_COPY_FAILED_CODE, E2_UNSENT_COPY_FAILED),
                "the refusal reaches the caller as it is, never wrapped"
            );
            assert!(late.exists(), "nothing was deleted");
            assert_eq!(
                unfinished_at(config).unwrap(),
                [folder.as_str()],
                "a refused purge leaves the list"
            );
            assert_eq!(saved_kept_unsent_at(config).unwrap(), Some(folder), "and the row");
        });
    }

    /// W13 and W10: a marker clear that fails does not stop a purge that cleared the rows (spec §7.3: best effort), and
    /// its warning names the step that failed, never the error: a read error quotes a line of the file, and the list's
    /// lines are kept folders' paths.
    #[test]
    fn a_failed_marker_clear_is_logged_without_the_kept_folder() {
        let f = PurgeFixture::new();
        f.write("wa", "a", b"a");
        f.with_own_config(|config| {
            let kept = f.keep_into(config);
            let folder = kept.folder.clone().unwrap().to_string_lossy().into_owned();
            // A line the parser stops at, and it names the folder (a hand edit, say): the clear cannot read the file.
            let mut text = std::fs::read_to_string(config).unwrap();
            text.push_str(&format!("\nnote = \"{folder}\" and more\n"));
            std::fs::write(config, text).unwrap();
            let mut purged = None;
            let logs = capture_logs(|| {
                purged = Some(purge_local_data_for_sign_out(
                    Ok(f.dir.path().to_path_buf()),
                    None,
                    false,
                    Some(&kept),
                ));
            });
            assert_eq!(purged, Some(Ok(())), "a clear that fails does not stop the sign-out");
            assert_eq!(f.queued(), 0, "the rows are cleared");
            let warning = logs
                .lines()
                .find(|line| line.contains("could not clear the unfinished sign-out marker"))
                .unwrap_or_else(|| panic!("the failed clear is logged:\n{logs}"));
            assert!(
                !logs.contains(&folder),
                "no log carries the kept folder's path:\n{logs}"
            );
            assert!(warning.contains("step=read"), "the warning names the step: {warning}");
        });
    }

    const U42: &str = "sign_out_keep_tests::an_interrupted_sign_out_still_names_its_folder_after_a_restart";

    /// The layout the parent and the child share: `state.db`, staging, the kept root and the config base.
    struct CrashDir {
        dir: PathBuf,
    }

    impl CrashDir {
        fn db(&self) -> state_db::StateDb {
            state_db::StateDb::open(state_paths::state_db_path_from_state_dir(&self.dir)).unwrap()
        }
        fn kept_root(&self) -> PathBuf {
            self.dir.join("kept")
        }
        async fn config(&self) -> PathBuf {
            crate::config::CONFIG_DIR_OVERRIDE
                .scope(self.dir.join("config"), async { DesktopConfig::path().unwrap() })
                .await
        }
        fn seed(&self) {
            let db = self.db();
            for op in ["w1", "w2", "w3"] {
                let staged = self.dir.join("staging").join(op);
                std::fs::create_dir_all(staged.parent().unwrap()).unwrap();
                std::fs::write(&staged, op.as_bytes()).unwrap();
                db.hold_lock_for_test()
                    .execute_batch(&format!(
                        "INSERT INTO files (file_id, path) VALUES ('{op}', '{op}.txt');
                         INSERT INTO operation_queue (op_id, kind, file_id, payload_path, write_id, write_origin)
                         VALUES ('{op}', 'upload_version', '{op}', '{p}', '{op}', 'minted');
                         INSERT INTO staged_payloads (path, completed) VALUES ('{p}', 0);",
                        p = staged.display()
                    ))
                    .unwrap();
            }
        }
        /// One sign-out's keep and purge, acknowledging the writes already missing.
        async fn sign_out(&self) -> Result<KeptSet, String> {
            let db = self.db();
            let config = self.config().await;
            let waiting = crate::sign_out::waiting::WaitingChanges::read(&db).unwrap();
            let snapshot = AskSnapshot::of(&waiting, "o", Vec::new());
            let kept = keep_unsent(KeepInputs {
                db: &db,
                acknowledge: Acknowledge::Asked {
                    others: &snapshot.others,
                    missing: &snapshot.missing,
                },
                root: &self.kept_root(),
                event_name: "event".into(),
                config_path: &config,
                clone: &SystemClone,
                config_fs: &RealConfigFs,
            })
            .map_err(|failure| failure.message())?;
            purge_local_state_files_with(&db, None, &[], None, Some(&kept), &|path| std::fs::remove_file(path))?;
            Ok(kept)
        }
        /// Run the child to `point`; require that it reached it and died by a signal. The child writes its output to a
        /// file in this directory: on this process's output, its harness's lines would be read as a second test run
        /// by whatever reads `cargo test`'s output (the count guard does).
        fn crash_at(&self, point: &str) {
            use std::os::unix::process::ExitStatusExt;
            let reached = self.dir.join("reached");
            let _ = std::fs::remove_file(&reached);
            let log = self.dir.join("child.log");
            let output = std::fs::File::create(&log).unwrap();
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", U42, "--test-threads=1", "--nocapture"])
                .env("BB_TEST_CRASH_CHILD", &self.dir)
                .env("BB_TEST_CRASH_AT", point)
                .env("BB_TEST_CRASH_REACHED", &reached)
                .env("HOME", self.dir.join("home"))
                .stdout(output.try_clone().unwrap())
                .stderr(output)
                .status()
                .unwrap();
            // Each line marked, so a failure message never carries a line that reads as this harness's own.
            let child: String = std::fs::read_to_string(&log)
                .unwrap_or_default()
                .lines()
                .map(|line| format!("child| {line}\n"))
                .collect();
            assert_eq!(
                std::fs::read_to_string(&reached).ok().as_deref(),
                Some(format!("reached:{point}").as_str()),
                "the child reached {point}; its output:\n{child}"
            );
            assert_eq!(
                status.signal(),
                Some(libc::SIGABRT),
                "the child died there, with no return; its output:\n{child}"
            );
        }
        /// After a "restart": the count from what the child left on disk.
        async fn count(&self) -> (usize, Vec<String>, Option<String>) {
            let config = self.config().await;
            let waiting = crate::sign_out::waiting::WaitingChanges::read(&self.db()).unwrap();
            let unfinished = crate::sign_out::kept_record::unfinished_at(&config).unwrap();
            let snapshot = AskSnapshot::of(&waiting, "o", unfinished);
            let row = DesktopConfig::load_from(&config).unwrap().kept_unsent_folder;
            (snapshot.counts.missing_files, snapshot.counts.earlier_folders, row)
        }
    }

    /// U42.
    #[tokio::test]
    async fn an_interrupted_sign_out_still_names_its_folder_after_a_restart() {
        if let Ok(dir) = std::env::var("BB_TEST_CRASH_CHILD") {
            let _ = CrashDir {
                dir: PathBuf::from(dir),
            }
            .sign_out()
            .await;
            return; // reached only if the crash point was never hit; the parent then fails on `reached`
        }
        let f1 = |c: &CrashDir| c.kept_root().join("event").to_string_lossy().into_owned();
        // Resolved: the writer records its resolved root, and these compare with what it recorded.
        let crash_dir = |scratch: &tempfile::TempDir| CrashDir {
            dir: std::fs::canonicalize(scratch.path()).unwrap(),
        };
        for (point, missing) in [
            ("AfterKeepSave", 0),
            ("AfterFirstGroup", 0),
            ("BetweenWaitingCopies", 1),
            ("BeforeRowClear", 3),
        ] {
            let scratch = tempfile::tempdir().unwrap();
            let c = crash_dir(&scratch);
            c.seed();
            c.crash_at(point);
            let (missing_files, earlier, row) = c.count().await;
            assert_eq!(missing_files, missing, "{point}");
            let expected: Vec<String> = if missing > 0 { vec![f1(&c)] } else { Vec::new() };
            assert_eq!(earlier, expected, "{point}");
            assert_eq!(row.as_deref(), Some(f1(&c).as_str()), "{point}");
            assert_eq!(
                crate::sign_out::kept_record::unfinished_at(&c.config().await).unwrap(),
                [f1(&c)],
                "{point}"
            );
            assert_eq!(
                std::fs::read(Path::new(&f1(&c)).join("w1.txt")).unwrap(),
                b"w1",
                "{point}: F1 holds the bytes"
            );
        }
        // (e) From (c)'s leftovers, a second sign-out dies at (c) again: the list holds both, oldest first.
        let scratch = tempfile::tempdir().unwrap();
        let c = crash_dir(&scratch);
        c.seed();
        c.crash_at("BetweenWaitingCopies");
        c.crash_at("BetweenWaitingCopies");
        let f2 = c.kept_root().join("event (2)").to_string_lossy().into_owned();
        let config = c.config().await;
        assert_eq!(
            crate::sign_out::kept_record::unfinished_at(&config).unwrap(),
            [f1(&c), f2.clone()]
        );
        assert_eq!(c.count().await.2.as_deref(), Some(f2.as_str()));
        assert_eq!(
            std::fs::read(Path::new(&f1(&c)).join("w1.txt")).unwrap(),
            b"w1",
            "F1 still holds its files"
        );
        // (f) From (e)'s leftovers, a sign-out that completes clears the list and never touches 1882's key. Its keep
        // makes the newest folder, F3 (F2 is (e)'s), and the row names it.
        crate::finder_removal::remember_kept_folder_at(&config, "/macos/kept").unwrap();
        let kept = {
            let db = c.db();
            let waiting = crate::sign_out::waiting::WaitingChanges::read(&db).unwrap();
            let snapshot = AskSnapshot::of(&waiting, "o", Vec::new());
            keep_unsent(KeepInputs {
                db: &db,
                acknowledge: Acknowledge::Asked {
                    others: &snapshot.others,
                    missing: &snapshot.missing,
                },
                root: &c.kept_root(),
                event_name: "event".into(),
                config_path: &config,
                clone: &SystemClone,
                config_fs: &RealConfigFs,
            })
            .unwrap()
        };
        let f3 = c.kept_root().join("event (3)");
        assert_eq!(
            kept.folder.as_deref(),
            Some(f3.as_path()),
            "a file still had a copy, so (f) kept it in a third folder"
        );
        crate::config::CONFIG_DIR_OVERRIDE
            .scope(
                c.dir.join("config"),
                purge_local_data_for_sign_out_async(c.dir.clone(), kept),
            )
            .await;
        let text = std::fs::read_to_string(&config).unwrap();
        assert!(!text.contains("kept_unsent_unfinished"), "{text}");
        let after = DesktopConfig::load_from(&config).unwrap();
        assert_eq!(
            after.kept_unsent_folder.as_deref(),
            Some(f3.to_string_lossy().as_ref()),
            "the row names the newest folder"
        );
        assert_eq!(after.kept_unsynced_folder.as_deref(), Some("/macos/kept"));
    }

    async fn purge_local_data_for_sign_out_async(dir: PathBuf, kept: KeptSet) {
        purge_local_data_for_sign_out(Ok(dir), None, false, Some(&kept)).unwrap();
    }

    /// U27, the purge half: the purge removes both staging roots' sources after keeping them.
    #[test]
    fn the_purge_removes_sources_from_every_staging_root() {
        let f = PurgeFixture::new();
        let caches = f.dir.path().join("Library/Caches/beebeeb/finder-writes/x");
        let support = f.dir.path().join("Library/Application Support/beebeeb/finder-writes/y");
        for path in [&caches, &support] {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, b"bytes").unwrap();
        }
        f.db.hold_lock_for_test()
            .execute_batch(&format!(
                "INSERT INTO operation_queue (op_id, kind, file_id, payload_path, write_id, write_origin) VALUES
                   ('ox', 'upload_version', 'x', '{}', 'ox', 'minted'), ('oy', 'upload_version', 'y', '{}', 'oy', 'minted');",
                caches.display(),
                support.display()
            ))
            .unwrap();
        let kept = f.keep();
        purge_local_state_files_as(&f.db, None, &[], None, Some(&kept)).unwrap();
        assert!(!caches.exists() && !support.exists());
        assert_eq!(f.queued(), 0);
    }

    /// `body` with a state dir and a config dir of its own (`state_paths::STATE_DIR_OVERRIDE`,
    /// `config::CONFIG_DIR_OVERRIDE`), both new and both removed when it ends, panicking or not. A sign-out on a Mac
    /// reads the queue before it changes anything, so a row queued in the state dir that the other sign-out tests share
    /// would refuse their sign-outs, and a row a failed test left there would refuse every later one. Here no other
    /// test's sign-out sees these rows, and no other test's config write meets the keys these tests check.
    async fn in_own_dirs<T>(body: impl std::future::Future<Output = T>) -> T {
        let dir = tempfile::tempdir().unwrap();
        let (state, config) = (dir.path().join("state"), dir.path().join("config"));
        std::fs::create_dir_all(&state).unwrap();
        std::fs::create_dir_all(&config).unwrap();
        state_paths::STATE_DIR_OVERRIDE
            .scope(state, crate::config::CONFIG_DIR_OVERRIDE.scope(config, body))
            .await
    }

    /// The database in the test's own state dir (`in_own_dirs`), the one its sign-out reads and purges.
    fn state_db_in_scratch() -> state_db::StateDb {
        let dir = state_paths::beebeeb_state_dir().unwrap();
        state_db::StateDb::open(state_paths::state_db_path_from_state_dir(&dir)).unwrap()
    }

    /// The database and config as a completed sign-out leaves them: no rows and no kept-folder keys.
    fn clean_scratch(db: &state_db::StateDb) {
        let _ = db.purge_all_local_state();
        let _ = db.clear_account_data(false);
        let _ = DesktopConfig::update(|cfg| {
            let changed = cfg.kept_unsent_folder.take().is_some() | !cfg.kept_unsent_unfinished.is_empty();
            cfg.kept_unsent_unfinished.clear();
            (changed, ())
        });
    }

    /// `signout_teardown_tests::test_account`, which that module keeps private.
    fn test_account(state: &AppState, id: &str) -> Arc<crate::account::AccountRuntime> {
        synthesize_single_account(state, AccountId(id.to_string()));
        state.active_account().expect("synthesized account resolves")
    }

    /// Records the reconciler's removals and triggers, and confirms every removal and Lock.
    fn recording_finder(state: &AppState) -> Arc<std::sync::Mutex<Vec<String>>> {
        use crate::finder_setup::driver::{Event, FinderSetupHandle, FinderSetupView};
        let (handle, mut rx) = FinderSetupHandle::for_test(FinderSetupView::initial(
            crate::finder_setup::launch_location::LaunchLocation::Applications,
        ));
        let _ = state.finder_setup.set(handle);
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = seen.clone();
        tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                match event {
                    Event::Remove { ack, .. } => {
                        log.lock().unwrap().push("remove".to_string());
                        let _ = ack.send((Ok(()), finder_removal::KeptFolder::default()));
                    }
                    Event::Lock { ack } => {
                        let _ = ack.send(());
                    }
                    Event::Trigger(trigger) => log.lock().unwrap().push(format!("{trigger:?}")),
                    _ => {}
                }
            }
        });
        seen
    }

    /// A waiting save and its staged copy. The copy is in the test's own state dir (`in_own_dirs`): these tests run
    /// at once and use the same op ids, and one of them makes its copy unreadable.
    fn queue_write(db: &state_db::StateDb, op_id: &str, file_id: &str, bytes: &[u8]) -> PathBuf {
        let staged = state_paths::beebeeb_state_dir()
            .unwrap()
            .join(format!("bb-1887-{op_id}"));
        std::fs::write(&staged, bytes).unwrap();
        db.hold_lock_for_test()
            .execute_batch(&format!(
                "INSERT OR IGNORE INTO files (file_id, path) VALUES ('{file_id}', '{file_id}.bin');
                 INSERT INTO operation_queue (op_id, kind, file_id, payload_path, write_id, write_origin)
                 VALUES ('{op_id}', 'upload_version', '{file_id}', '{}', '{op_id}', 'minted');",
                staged.display()
            ))
            .unwrap();
        staged
    }

    fn queue_rename(db: &state_db::StateDb, op_id: &str) {
        db.hold_lock_for_test()
            .execute(
                "INSERT INTO operation_queue (op_id, kind, file_id) VALUES (?1, 'rename_file', 'f')",
                rusqlite::params![op_id],
            )
            .unwrap();
    }

    fn ask(state: &AppState, owner: &str) -> String {
        let snapshot = count_ask(owner).unwrap();
        let id = snapshot.ask_id().to_string();
        state.sign_out.snapshots.record(snapshot);
        id
    }

    fn signed_out_state() -> (AppState, Arc<crate::account::AccountRuntime>) {
        let state = AppState::default();
        let acct = test_account(&state, "signout-1887");
        set_auth_present(&state, false);
        (state, acct)
    }

    async fn sign_out_with(
        state: &AppState,
        keep: Option<&str>,
        seam: SignOutSeam,
    ) -> Result<SignedOut, SignOutFailure> {
        SIGN_OUT_SEAM.scope(seam, clear_session_impl(state, false, keep, crate::UnknownFilesArg::Keep, None)).await
    }

    fn no_seam() -> SignOutSeam {
        Arc::new(|_| Ok(()))
    }

    /// U6. The switch's sign-out (`forget_email`) is the one that would forget the email prefill, so the refusal is
    /// tested on it: the gate comes first, and nothing is touched (the Keychain session and the email prefill too).
    #[tokio::test]
    async fn clear_session_refuses_while_changes_wait() {
        in_own_dirs(async {
            let db = state_db_in_scratch();
            let staged = queue_write(&db, "w1", "f1", b"bytes");
            let state = AppState::default();
            let acct = test_account(&state, "signout-1887-u6");
            let events = recording_finder(&state);
            *acct.session.lock().unwrap() = Some(super::Session {
                token: "t".into(),
                master_key: [7u8; 32],
                email: Some("a@beebeeb.io".into()),
            });
            persist_session_to_keychain(
                &SessionWrite::for_test(),
                "signout-1887-u6",
                "tok-u6",
                &[7u8; 32],
                Some("a@beebeeb.io"),
            )
            .unwrap();
            DesktopConfig::update(|cfg| {
                cfg.last_signed_in_email = Some("prefill@beebeeb.io".into());
                (true, ())
            })
            .unwrap();
            set_auth_present(&state, true);
            let failure = SIGN_OUT_SEAM
                .scope(no_seam(), clear_session_impl(&state, true, None, crate::UnknownFilesArg::Keep, None))
                .await
                .unwrap_err();
            assert!(failure.message.starts_with("unsent_changes: "), "{}", failure.message);
            assert_eq!(db.waiting_rows().unwrap().len(), 1, "the queue is intact");
            assert!(staged.exists(), "the staged copy is intact");
            assert!(acct.session.lock().unwrap().is_some(), "the keys are in memory");
            assert!(
                load_session_from_keychain("signout-1887-u6", None).unwrap().is_some(),
                "the Keychain session is untouched"
            );
            assert_eq!(
                DesktopConfig::load().unwrap().last_signed_in_email.as_deref(),
                Some("prefill@beebeeb.io"),
                "the email prefill is untouched"
            );
            assert!(
                events.lock().unwrap().iter().all(|event| event != "remove"),
                "the reconciler got no removal"
            );
            let _ = clear_keychain_session("signout-1887-u6");
            DesktopConfig::update(|cfg| (cfg.last_signed_in_email.take().is_some(), ())).unwrap();
        })
        .await;
    }

    /// U8 and U8b through the sign-out: the kept set is kept, one file per file, and then the queue is empty. The keep
    /// step's own tests hold its half (names and bytes).
    #[tokio::test]
    async fn a_sign_out_keeps_the_kept_set_and_empties_the_queue() {
        in_own_dirs(async {
            let db = state_db_in_scratch();
            queue_write(&db, "s1", "report", b"first save");
            queue_write(&db, "s2", "report", b"second save");
            queue_write(&db, "c1", "notes", b"notes");
            let (state, _acct) = signed_out_state();
            recording_finder(&state);
            let id = ask(&state, "main");
            let done = sign_out_with(&state, Some(&id), no_seam()).await.unwrap();
            let kept = done.kept_unsent.expect("a kept folder");
            let folder = Path::new(&kept.folder);
            assert_eq!(kept.files, 2, "two saves of one file keep one file (U8b)");
            assert_eq!(std::fs::read(folder.join("report.bin")).unwrap(), b"second save");
            assert_eq!(std::fs::read(folder.join("notes.bin")).unwrap(), b"notes");
            assert_eq!(
                std::fs::read_dir(folder).unwrap().count(),
                2,
                "no earlier-save copy of a minted save"
            );
            assert!(db.waiting_rows().unwrap().is_empty(), "the queue is empty");
        })
        .await;
    }

    /// U7.
    #[test]
    fn the_gate_comes_before_anything_changes() {
        let production = super::finder_setup_command_tests::production_source();
        let clear =
            super::finder_setup_command_tests::body_between(&production, "async fn clear_session_impl(", "\n}\n");
        let gate = clear.find("gate_sign_out(").expect("the gate");
        for later in [
            "forget_last_signed_in_email(",
            "end_sessions_in_flight(",
            "finder_remove_for(",
            "take_slot_and_stop_engine_for_sign_out(",
            "purge_local_data_for_sign_out(",
        ] {
            assert!(
                gate < clear.find(later).unwrap_or_else(|| panic!("{later}")),
                "the gate comes before {later}"
            );
        }
    }

    /// U9: the gate counted nothing; a write that arrives before the keep step is kept and counted.
    #[tokio::test]
    async fn a_write_that_arrives_after_the_count_is_kept_too() {
        in_own_dirs(async {
            let db = state_db_in_scratch();
            let (state, _acct) = signed_out_state();
            recording_finder(&state);
            let seam: SignOutSeam = Arc::new(move |point| {
                if point == "before_keep" {
                    let db = state_db_in_scratch();
                    queue_write(&db, "late", "late", b"late bytes");
                }
                Ok(())
            });
            let done = sign_out_with(&state, None, seam).await.unwrap();
            let kept = done.kept_unsent.expect("a kept folder");
            assert_eq!(kept.files, 1);
            assert_eq!(
                std::fs::read(Path::new(&kept.folder).join("late.bin")).unwrap(),
                b"late bytes"
            );
            assert_eq!(db.waiting_rows().unwrap().len(), 0, "the queue is empty");
        })
        .await;
    }

    /// U10 (spec, amended for `184ac962`).
    #[test]
    fn nothing_that_deletes_runs_between_stop_keep_and_purge() {
        let production = super::finder_setup_command_tests::production_source();
        let clear =
            super::finder_setup_command_tests::body_between(&production, "async fn clear_session_impl(", "\n}\n");
        let slot = clear
            .find("let mut held_engine_slot = if already_signed_out {")
            .expect("the stop expression");
        let second_stop = clear.rfind("take_slot_and_stop_engine_for_sign_out(").unwrap();
        let close = slot + clear[slot..].find("\n    };\n").expect("the expression's end");
        let keep = clear.find("keep_for_sign_out(").expect("the keep call");
        let purge = clear.find("purge_local_data_for_sign_out(").unwrap();
        assert!(
            second_stop < close && close < keep && keep < purge,
            "stop arms, then `}};`, then keep, then purge"
        );
        let between = &clear[second_stop..purge];
        for denied in [
            "remove_file",
            "remove_dir",
            "purge_all_local_state(",
            "purge_macos_hydrate_cache(",
            "clear_session_holding_slot(",
            "clear_keychain_session(",
            "forget_last_signed_in_email(",
            "start_engine",
            "KeysArrived",
        ] {
            assert!(!between.contains(denied), "{denied} between the stop and the purge");
        }
    }

    /// U13, the sign-out half: a failed copy leaves the keys in memory and asks the reconciler to restart sync.
    #[tokio::test]
    async fn a_failed_copy_keeps_the_keys_and_restarts_sync() {
        in_own_dirs(async {
            use std::os::unix::fs::PermissionsExt;
            let db = state_db_in_scratch();
            let staged = queue_write(&db, "w1", "f1", b"bytes");
            std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o000)).unwrap();
            let state = AppState::default();
            let acct = test_account(&state, "signout-1887-u13");
            let events = recording_finder(&state);
            *acct.session.lock().unwrap() = Some(super::Session {
                token: "t".into(),
                master_key: [7u8; 32],
                email: None,
            });
            set_auth_present(&state, true);
            let id = ask(&state, "main");
            let mut failure = None;
            let logs = capture_logs_async(async {
                failure = Some(sign_out_with(&state, Some(&id), no_seam()).await.unwrap_err());
            })
            .await;
            let failure = failure.unwrap();
            std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o600)).unwrap();
            assert!(
                failure.message.starts_with("unsent_copy_failed: "),
                "{}",
                failure.message
            );
            assert!(
                logs.contains("sign-out stopped: the waiting changes could not be kept; nothing was purged"),
                "the sign-out's failed keep is logged as the sign-out's: {logs}"
            );
            assert!(acct.session.lock().unwrap().is_some(), "the keys are in memory");
            // `std::time::Duration`: `lib.rs` imports no `Duration` at the crate level.
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            assert!(
                events.lock().unwrap().iter().any(|event| event == "KeysArrived"),
                "sync restarts: {:?}",
                events.lock().unwrap()
            );
            assert_eq!(db.waiting_rows().unwrap().len(), 1);
        })
        .await;
    }

    /// U29 (b): a copy deleted before the call is a missing write the snapshot did not hold.
    #[tokio::test]
    async fn a_copy_missing_before_the_call_asks_again() {
        in_own_dirs(async {
            let db = state_db_in_scratch();
            let staged = queue_write(&db, "w1", "f1", b"bytes");
            let (state, _acct) = signed_out_state();
            recording_finder(&state);
            let id = ask(&state, "main");
            std::fs::remove_file(&staged).unwrap();
            let failure = sign_out_with(&state, Some(&id), no_seam()).await.unwrap_err();
            assert!(
                failure.message.starts_with("unsent_changes: "),
                "the gate refuses, the ask is shown again"
            );
            let fresh = count_ask("main").unwrap();
            assert_eq!(fresh.counts.missing_files, 1);
        })
        .await;
    }

    /// U29 (c), through the sign-out: a copy missing at the ask and held by the snapshot is acknowledged; the sign-out
    /// completes, the purge's guard lets the missing write go, and the other write is kept.
    #[tokio::test]
    async fn a_copy_missing_at_the_ask_is_let_go_and_the_sign_out_completes() {
        in_own_dirs(async {
            let db = state_db_in_scratch();
            let gone = queue_write(&db, "w1", "f1", b"gone");
            queue_write(&db, "w2", "f2", b"kept");
            std::fs::remove_file(&gone).unwrap();
            let (state, _acct) = signed_out_state();
            recording_finder(&state);
            let id = ask(&state, "main");
            let done = sign_out_with(&state, Some(&id), no_seam())
                .await
                .expect("the ask showed the missing write, so the sign-out completes");
            let kept = done.kept_unsent.expect("a kept folder");
            assert_eq!(kept.files, 1, "the other write is kept");
            assert_eq!(std::fs::read(Path::new(&kept.folder).join("f2.bin")).unwrap(), b"kept");
            assert!(db.waiting_rows().unwrap().is_empty(), "the queue is empty");
        })
        .await;
    }

    /// U30: the record is on disk before the purge, and only the keep step writes it. Two aborts: right after the keep
    /// step, both keys name the folder; right after the purge, the rows are gone, so the marker is cleared and the row
    /// stays (§7.3, "When it is cleared").
    #[tokio::test]
    async fn the_kept_folder_is_recorded_before_the_purge() {
        in_own_dirs(async {
            let db = state_db_in_scratch();
            for (point, marker_kept) in [("after_keep", true), ("after_purge", false)] {
                queue_write(&db, "w1", "f1", b"bytes");
                let (state, _acct) = signed_out_state();
                recording_finder(&state);
                let id = ask(&state, "main");
                let seam: SignOutSeam = Arc::new(move |at| {
                    if at == point {
                        Err("aborted by the test".into())
                    } else {
                        Ok(())
                    }
                });
                let failure = sign_out_with(&state, Some(&id), seam).await.unwrap_err();
                let folder = failure.kept_unsent.expect("the failure carries the folder").folder;
                let config = DesktopConfig::load().unwrap();
                assert_eq!(config.kept_unsent_folder.as_deref(), Some(folder.as_str()), "{point}");
                let expected: Vec<String> = if marker_kept { vec![folder.clone()] } else { Vec::new() };
                assert_eq!(
                    config.kept_unsent_unfinished, expected,
                    "{point}: saved in the keep step's one write; cleared with the rows"
                );
                clean_scratch(&db);
            }
        })
        .await;
        let production = super::finder_setup_command_tests::production_source();
        let clear =
            super::finder_setup_command_tests::body_between(&production, "async fn clear_session_impl(", "\n}\n");
        assert!(
            !clear.contains("save_kept_unsent_durably("),
            "the Err arm writes neither key; only the keep step does"
        );
        // The keep step's save is its last act, after its last flush: `finish()` (every `sync_all`) comes before it,
        // and nothing after it copies or flushes.
        let keep = include_str!("sign_out/keep.rs");
        let keep = &keep[..keep.find("#[cfg(test)]\nmod tests").expect("the tests")];
        let save = keep
            .find("save_kept_unsent_durably(inputs.config_path")
            .expect("the save");
        assert!(
            keep.find("writer.finish()").expect("the flushes") < save,
            "the flushes come first"
        );
        let after = &keep[save..];
        assert!(
            !after.contains("writer.keep(") && !after.contains(".finish()") && !after.contains("sync_all"),
            "{after}"
        );
        let writer = include_str!("sign_out/kept_writer.rs");
        let finish = &writer[writer.find("pub(crate) fn finish(self)").expect("finish")..];
        assert!(
            finish[..finish.find("\n    }\n").unwrap()].contains("sync_all()"),
            "finish flushes"
        );
        assert!(
            clear.find("keep_for_sign_out(").expect("the keep call")
                < clear.find("purge_local_data_for_sign_out(").expect("the purge")
        );
    }

    /// U32, the macOS half: every new step is macOS only.
    #[test]
    fn the_unsent_gate_is_macos_only() {
        let production = super::finder_setup_command_tests::production_source();
        let clear =
            super::finder_setup_command_tests::body_between(&production, "async fn clear_session_impl(", "\n}\n");
        for call in ["gate_sign_out(", "keep_for_sign_out("] {
            let at = clear.find(call).unwrap_or_else(|| panic!("{call}"));
            let before = &clear[..at];
            let cfg = before.rfind("#[cfg(").unwrap();
            assert!(
                before[cfg..].starts_with("#[cfg(target_os = \"macos\")]"),
                "{call} is under cfg(macos)"
            );
        }
        for definition in ["fn gate_sign_out(", "fn keep_for_sign_out(", "fn count_ask("] {
            let at = production.find(definition).unwrap_or_else(|| panic!("{definition}"));
            let before = &production[..at];
            assert!(
                before[before.rfind("#[cfg(").unwrap()..].starts_with("#[cfg(target_os = \"macos\")]"),
                "{definition}"
            );
        }
    }

    /// U37, the keep step's snapshot: the one the gate accepted, by value, never a newer one.
    #[tokio::test]
    async fn the_keep_step_uses_the_snapshot_the_gate_accepted() {
        in_own_dirs(async {
            let db = state_db_in_scratch();
            queue_rename(&db, "r1");
            let (state, _acct) = signed_out_state();
            recording_finder(&state);
            let id = ask(&state, "main");
            let store = state.sign_out.snapshots.clone();
            let seam: SignOutSeam = Arc::new(move |point| {
                if point == "before_keep" {
                    queue_rename(&state_db_in_scratch(), "r2");
                    store.record(count_ask("main").unwrap()); // S′ holds both renames
                }
                Ok(())
            });
            let failure = sign_out_with(&state, Some(&id), seam).await.unwrap_err();
            assert!(failure.message.starts_with("unsent_changes: "), "{}", failure.message);
            assert_eq!(db.waiting_rows().unwrap().len(), 2, "nothing purged");
            // Second run: S′ recorded without changing the queue; the sign-out completes and empties the store.
            db.hold_lock_for_test()
                .execute("DELETE FROM operation_queue WHERE op_id = 'r2'", [])
                .unwrap();
            let id = ask(&state, "main");
            let store = state.sign_out.snapshots.clone();
            let seam: SignOutSeam = Arc::new(move |point| {
                if point == "before_keep" {
                    store.record(count_ask("main").unwrap());
                }
                Ok(())
            });
            sign_out_with(&state, Some(&id), seam).await.unwrap();
            assert_eq!(
                state.sign_out.snapshots.newest_ask_id(),
                None,
                "a completed sign-out empties the store"
            );
        })
        .await;
    }

    /// U41: entry point 7, the already-signed-out arm, keeps before it purges.
    #[tokio::test]
    async fn an_already_signed_out_sign_out_keeps_before_it_purges() {
        in_own_dirs(async {
            let db = state_db_in_scratch();
            queue_write(&db, "w1", "f1", b"already signed out");
            let (state, _acct) = signed_out_state();
            recording_finder(&state);
            let id = ask(&state, "main");
            let done = sign_out_with(&state, Some(&id), no_seam()).await.unwrap();
            assert_eq!(
                done.outcome,
                SignOutOutcome::NotSignedIn,
                "the already-signed-out arm ran"
            );
            let kept = done.kept_unsent.unwrap();
            assert_eq!(
                std::fs::read(Path::new(&kept.folder).join("f1.bin")).unwrap(),
                b"already signed out"
            );
            assert!(db.waiting_rows().unwrap().is_empty());
        })
        .await;
    }

    /// U14 through the sign-out: the guard's refusal reaches the caller with its code, and the folder rides it.
    #[tokio::test]
    async fn a_guard_refusal_reaches_the_caller_with_its_code() {
        in_own_dirs(async {
            let db = state_db_in_scratch();
            queue_write(&db, "w1", "f1", b"bytes");
            let (state, _acct) = signed_out_state();
            recording_finder(&state);
            let id = ask(&state, "main");
            let seam: SignOutSeam = Arc::new(|point| {
                if point == "after_keep" {
                    queue_write(&state_db_in_scratch(), "late", "late", b"late"); // only a test seam can add one now
                }
                Ok(())
            });
            let failure = sign_out_with(&state, Some(&id), seam).await.unwrap_err();
            assert!(
                failure.message.starts_with("unsent_copy_failed: "),
                "never wrapped: {}",
                failure.message
            );
            assert!(failure.kept_unsent.is_some(), "the kept folder rides the failure");
            assert_eq!(db.waiting_rows().unwrap().len(), 2, "nothing deleted");
        })
        .await;
    }

    /// Two sign-outs from one ask (a double click, or the menu's while Settings signs out): one sign-out, one folder.
    /// The first one's gate takes the snapshot out of the store, so a second gate on the same ask refuses while the
    /// first sign-out is still running, not only after its completion empties the store.
    #[tokio::test]
    async fn a_second_sign_out_with_the_same_ask_keeps_nothing_twice() {
        in_own_dirs(async {
            let db = state_db_in_scratch();
            queue_write(&db, "w1", "f1", b"once");
            let (state, _acct) = signed_out_state();
            recording_finder(&state);
            let id = ask(&state, "main");
            let (store, same_ask) = (state.sign_out.snapshots.clone(), id.clone());
            let second_gate = Arc::new(std::sync::Mutex::new(None));
            let seen = second_gate.clone();
            let seam: SignOutSeam = Arc::new(move |point| {
                if point == "before_keep" {
                    // The second click's gate, on the queue as it is now, while the first sign-out runs.
                    let waiting = crate::sign_out::waiting::WaitingChanges::read(&state_db_in_scratch()).unwrap();
                    *seen.lock().unwrap() = Some(store.gate(&waiting, Some(&same_ask)));
                }
                Ok(())
            });
            let first = sign_out_with(&state, Some(&id), seam).await.unwrap();
            assert!(first.kept_unsent.is_some());
            assert_eq!(
                *second_gate.lock().unwrap(),
                Some(Err(crate::sign_out::snapshot::GateRefusal)),
                "the first sign-out took the snapshot: a second gate on the same ask refuses while it runs"
            );
            queue_write(&db, "w2", "f2", b"again"); // something waits again, so the stale ask matters
            let second = sign_out_with(&state, Some(&id), no_seam()).await.unwrap_err();
            assert!(second.message.starts_with("unsent_changes: "), "{}", second.message);
            assert_eq!(second.kept_unsent, None, "no second folder");
        })
        .await;
    }

    /// U39 (b), the ask's half: an interrupted sign-out's folder is named when the ask shows a missing write.
    #[tokio::test]
    async fn the_ask_names_the_earlier_folders_only_with_a_missing_write() {
        in_own_dirs(async {
            let db = state_db_in_scratch();
            let staged = queue_write(&db, "w1", "f1", b"bytes");
            DesktopConfig::update(|cfg| {
                cfg.kept_unsent_unfinished = vec!["/kept/earlier".into()];
                (true, ())
            })
            .unwrap();
            assert!(
                count_ask("o").unwrap().counts.earlier_folders.is_empty(),
                "no missing write: no A3e"
            );
            std::fs::remove_file(&staged).unwrap();
            assert_eq!(count_ask("o").unwrap().counts.earlier_folders, ["/kept/earlier"]);
            DesktopConfig::update(|cfg| {
                cfg.kept_unsent_unfinished.clear();
                (true, ())
            })
            .unwrap();
        })
        .await;
    }

    /// The sign-out of a person who is signed in, the usual one: it ends `Completed`, its result carries the kept folder
    /// (the alert names it, W9), and it empties the store, also of a newer count another window recorded meanwhile.
    #[tokio::test]
    async fn a_signed_in_sign_out_completes_with_its_folder_and_empties_the_store() {
        in_own_dirs(async {
            let db = state_db_in_scratch();
            queue_write(&db, "w1", "f1", b"signed in");
            let state = AppState::default();
            let acct = test_account(&state, "signout-1887-completed");
            recording_finder(&state);
            *acct.session.lock().unwrap() = Some(super::Session {
                token: "t".into(),
                master_key: [7u8; 32],
                email: None,
            });
            set_auth_present(&state, true);
            let id = ask(&state, "main");
            let store = state.sign_out.snapshots.clone();
            let seam: SignOutSeam = Arc::new(move |point| {
                if point == "before_keep" {
                    store.record(count_ask("onboarding").unwrap()); // another window counts while this one signs out
                }
                Ok(())
            });
            let done = sign_out_with(&state, Some(&id), seam).await.unwrap();
            assert_eq!(done.outcome, SignOutOutcome::Completed, "the signed-in arm ran");
            let kept = done.kept_unsent.expect("the Completed arm carries the kept folder");
            assert_eq!(kept.files, 1);
            assert_eq!(
                std::fs::read(Path::new(&kept.folder).join("f1.bin")).unwrap(),
                b"signed in"
            );
            assert_eq!(
                state.sign_out.snapshots.newest_ask_id(),
                None,
                "a completed signed-in sign-out empties the store"
            );
            assert!(acct.session.lock().unwrap().is_none(), "the keys are cleared");
            assert!(db.waiting_rows().unwrap().is_empty(), "the queue is empty");
        })
        .await;
    }

    /// W1 and W6 with no ask: nothing waited at the gate, so the sign-out passed no `keepUnsent`. An other change (a
    /// rename, a move, a new folder or a deletion) that arrives before the keep step stops the sign-out with
    /// `unsent_changes`, so the ask is shown, and nothing is purged. (A write that arrives then is kept: U9.)
    #[tokio::test]
    async fn a_late_other_change_with_no_ask_stops_the_sign_out() {
        in_own_dirs(async {
            let db = state_db_in_scratch();
            let (state, _acct) = signed_out_state();
            recording_finder(&state);
            let seam: SignOutSeam = Arc::new(|point| {
                if point == "before_keep" {
                    queue_rename(&state_db_in_scratch(), "late-rename");
                }
                Ok(())
            });
            let result = sign_out_with(&state, None, seam).await;
            assert_eq!(
                db.waiting_rows().unwrap().len(),
                1,
                "the late rename is not purged unasked: {result:?}"
            );
            let failure = result.expect_err("the sign-out stops");
            assert_eq!(
                failure.message,
                coded(UNSENT_CHANGES_CODE, E1_UNSENT_CHANGES),
                "the ask is shown again"
            );
            assert_eq!(failure.kept_unsent, None, "nothing was kept");
        })
        .await;
    }

    /// `clear_session` hands `keepUnsent: { askId }` on to the gate (spec §5.1, the contract). A unit test cannot build
    /// the command's `AppHandle`, so this checks the name the webview sends and the command's call.
    /// Task 2191 (review thread on #157): a Windows sign-out that fails after its purge kept
    /// copies names their folder: every failure from the purge on carries it.
    #[test]
    fn task_2191_a_failure_after_the_purge_names_the_kept_folder() {
        let kept = Some(super::KeptUnsent { folder: "C:\\Beebeeb kept files".into(), files: 3 });
        let failure = super::sign_out_failure_after_purge("Could not unregister Cloud Files: x".into(), &None, &None, &kept);
        assert_eq!(failure.kept_unsent, kept);
        let production = super::finder_setup_command_tests::production_source();
        let body = crate::source_pin::squeeze(super::finder_setup_command_tests::body_between(
            &production,
            "async fn clear_session_impl(",
            "\n}\n",
        ));
        let after = &body[body.find("windows_cf::signout::purge(").expect("the purge")..];
        // Task 1886: the logout request is no step that can fail the sign-out any more (it is best effort on every OS,
        // `end_dropped_sessions`), so its "Could not initialize logout request" failure is gone.
        for step in ["Could not unregister Cloud Files", "Could not remove Explorer registration"] {
            assert!(
                after.contains(&crate::source_pin::squeeze(&format!("failed_after_purge(format!(\"{step}"))),
                "{step} carries the kept folder"
            );
        }
        assert!(
            after.contains(&crate::source_pin::squeeze(
                "let kept_with = |message: String| sign_out_failure_after_purge(message, &preserved_location, &kept_unsent, &windows_kept);"
            )),
            "the later failures carry it too"
        );
        assert!(after.contains(&crate::source_pin::squeeze(
            "clear_keychain_session_into(acct.id.as_str(), &mut dropped).map_err(&kept_with)?;"
        )));
    }

    #[test]
    fn clear_session_hands_keep_unsent_on_to_the_gate() {
        let arg: KeepUnsentArg =
            serde_json::from_value(serde_json::json!({ "askId": "a1" })).expect("the webview sends askId");
        assert_eq!(arg.ask_id, "a1");
        assert!(
            serde_json::from_value::<KeepUnsentArg>(serde_json::json!({ "ask_id": "a1" })).is_err(),
            "askId is the only name it takes"
        );
        let production = super::finder_setup_command_tests::production_source();
        let command = crate::source_pin::squeeze(super::finder_setup_command_tests::body_between(
            &production,
            "async fn clear_session(",
            "\n}\n",
        ));
        for expected in [
            // Tauri names the argument after the parameter, in camelCase: `keepUnsent`.
            "keep_unsent: Option<KeepUnsentArg>",
            "clear_session_impl(&state, forget_email.unwrap_or(false), keep_unsent.as_ref().map(|keep| keep.ask_id.as_str()),",
            // Task 2191: and the Windows answer for files of unknown content, kept when absent.
            "unknown_files.unwrap_or(UnknownFilesArg::Keep),",
            "kept_folder.map(std::path::PathBuf::from))",
        ] {
            assert!(
                command.contains(&crate::source_pin::squeeze(expected)),
                "clear_session passes keepUnsent on: {expected}\n{command}"
            );
        }
    }

    /// Spec §5.3, "4. Gate | Read fails | E3 | Nothing changed": a gate that cannot read the queue answers E3 before the
    /// Finder removal and the engine stop, and the keys stay in memory.
    #[tokio::test]
    async fn a_gate_that_cannot_read_the_queue_changes_nothing() {
        in_own_dirs(async {
            // A directory where `state.db` should be: it exists, and it cannot be opened.
            let dir = state_paths::beebeeb_state_dir().unwrap();
            std::fs::create_dir_all(state_paths::state_db_path_from_state_dir(&dir)).unwrap();
            let state = AppState::default();
            let acct = test_account(&state, "signout-1887-unread");
            let events = recording_finder(&state);
            *acct.session.lock().unwrap() = Some(super::Session {
                token: "t".into(),
                master_key: [7u8; 32],
                email: None,
            });
            set_auth_present(&state, true);
            let failure = sign_out_with(&state, None, no_seam()).await.unwrap_err();
            assert!(
                events.lock().unwrap().iter().all(|event| event != "remove"),
                "the reconciler got no removal: {:?} ({})",
                events.lock().unwrap(),
                failure.message
            );
            assert_eq!(
                failure.message,
                coded(UNSENT_COUNT_FAILED_CODE, E3_UNSENT_COUNT_FAILED),
                "E3 with its code"
            );
            assert!(acct.session.lock().unwrap().is_some(), "the keys are in memory");
        })
        .await;
    }

    /// The keep step opens `state.db` itself. When it cannot, the queue cannot be read: it answers E3 with its code, as a
    /// failed read inside the keep step does, never the database's error, whose text names the database's absolute
    /// path; and its log names the step, never the path (W10).
    #[tokio::test]
    async fn a_keep_step_that_cannot_open_the_queue_answers_e3_and_names_no_path() {
        in_own_dirs(async {
            let dir = state_paths::beebeeb_state_dir().unwrap();
            std::fs::create_dir_all(state_paths::state_db_path_from_state_dir(&dir)).unwrap();
            let mut kept = None;
            let logs = capture_logs(|| kept = Some(keep_for_sign_out(None)));
            let error = match kept.expect("the keep step ran") {
                Ok(_) => panic!("a state.db that cannot be opened keeps nothing"),
                Err(error) => error,
            };
            assert_eq!(
                error,
                coded(UNSENT_COUNT_FAILED_CODE, E3_UNSENT_COUNT_FAILED),
                "E3 with its code"
            );
            let path = dir.to_string_lossy().into_owned();
            assert!(!logs.contains(&path), "no log carries the path:\n{logs}");
            let line = logs
                .lines()
                .find(|line| line.contains("the waiting changes could not be read"))
                .unwrap_or_else(|| panic!("the stop is logged:\n{logs}"));
            assert!(line.contains("step=state_db"), "the log names the step: {line}");
        })
        .await;
    }

    /// An ask that shows a missing write names the earlier folders (A3e). When the list cannot be read, the ask still
    /// counts, without them, and its warning names the step that failed, never the error: a read error quotes a line of
    /// the file, and the list's lines are kept folders' paths (W10).
    #[tokio::test]
    async fn an_ask_whose_unfinished_list_cannot_be_read_still_counts_and_logs_the_step() {
        in_own_dirs(async {
            let db = state_db_in_scratch();
            let staged = queue_write(&db, "w1", "f1", b"bytes");
            std::fs::remove_file(&staged).unwrap(); // a missing write: the ask reads the list
            let config = DesktopConfig::path().unwrap();
            // A line the parser stops at, and it names a kept folder (a hand edit, say).
            std::fs::write(
                &config,
                "kept_unsent_unfinished = [\"/kept/earlier\"]\nnote = \"/kept/earlier\" and more\n",
            )
            .unwrap();
            let mut counted = None;
            let logs = capture_logs(|| counted = Some(count_ask("o")));
            let snapshot = counted.expect("the count ran").expect("the ask still counts");
            assert_eq!(snapshot.counts.missing_files, 1);
            assert!(snapshot.counts.earlier_folders.is_empty(), "no folder could be read");
            let warning = logs
                .lines()
                .find(|line| line.contains("the unfinished sign-out list could not be read"))
                .unwrap_or_else(|| panic!("the failed read is logged:\n{logs}"));
            assert!(
                !logs.contains("/kept/earlier") && !logs.contains(&*config.to_string_lossy()),
                "no log carries a kept folder or the file's path:\n{logs}"
            );
            assert!(warning.contains("step=read"), "the warning names the step: {warning}");
        })
        .await;
    }

    /// U45 (spec 2026-10-10 §15 question 3): every sign-out that ends without purging leaves Finder accepts open at
    /// once.
    #[tokio::test]
    async fn a_cancelled_or_failed_sign_out_leaves_accepts_open() {
        in_own_dirs(async {
            use std::os::unix::fs::PermissionsExt;
            let db = state_db_in_scratch();
            let staged = queue_write(&db, "w1", "f1", b"bytes");
            let (state, _acct) = signed_out_state();
            recording_finder(&state);
            // Cancel: the person was asked and chose Cancel, so no `clear_session` ran at all. The ask itself closes
            // nothing.
            ask(&state, "main");
            assert!(
                !db.finder_accepts_closed().unwrap(),
                "a cancelled sign-out: the ask closes nothing"
            );
            // The gate refuses: changes wait and no `keepUnsent`.
            assert!(sign_out_with(&state, None, no_seam()).await.is_err());
            assert!(!db.finder_accepts_closed().unwrap(), "refused at the gate");
            // The keep step fails: a staged copy it cannot read.
            std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o000)).unwrap();
            let id = ask(&state, "main");
            let failed = sign_out_with(&state, Some(&id), no_seam()).await;
            std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o600)).unwrap();
            assert!(failed.unwrap_err().message.starts_with("unsent_copy_failed: "));
            assert!(!db.finder_accepts_closed().unwrap(), "a failed keep");
            // The purge guard refuses after a completed keep (only a test seam can add a write then). The seam records
            // that the keep step left accepts closed, so the reopen checked last is the guard's.
            let id = ask(&state, "main");
            let closed_after_the_keep = Arc::new(std::sync::Mutex::new(None));
            let seen = closed_after_the_keep.clone();
            let seam: SignOutSeam = Arc::new(move |point| {
                if point == "after_keep" {
                    let db = state_db_in_scratch();
                    *seen.lock().unwrap() = Some(db.finder_accepts_closed().unwrap());
                    queue_write(&db, "late", "late", b"late");
                }
                Ok(())
            });
            let refused = sign_out_with(&state, Some(&id), seam).await.unwrap_err();
            assert_eq!(
                *closed_after_the_keep.lock().unwrap(),
                Some(true),
                "the completed keep left accepts closed for the purge"
            );
            assert_eq!(
                refused.message,
                coded(UNSENT_COPY_FAILED_CODE, E2_UNSENT_COPY_FAILED),
                "the purge guard refused (E2), never wrapped"
            );
            assert!(
                refused.kept_unsent.is_some(),
                "a refusal after a completed keep carries its folder"
            );
            assert!(!db.finder_accepts_closed().unwrap(), "a refused purge");
        })
        .await;
    }

    /// U45: the rule holds whatever the keep step kept. With nothing waiting, the keep step still closes accepts (an
    /// accept could commit after its read), and a return before the purge reopens them, as after a keep that made a
    /// folder.
    #[tokio::test]
    async fn a_sign_out_that_kept_nothing_and_stops_reopens_accepts() {
        in_own_dirs(async {
            let db = state_db_in_scratch();
            let (state, _acct) = signed_out_state();
            recording_finder(&state);
            let closed_after_the_keep = Arc::new(std::sync::Mutex::new(None));
            let seen = closed_after_the_keep.clone();
            let seam: SignOutSeam = Arc::new(move |point| {
                if point == "after_keep" {
                    *seen.lock().unwrap() = Some(state_db_in_scratch().finder_accepts_closed().unwrap());
                    return Err("stopped by the test".to_string());
                }
                Ok(())
            });
            let failure = sign_out_with(&state, None, seam).await.unwrap_err();
            assert_eq!(
                (failure.message.as_str(), &failure.kept_unsent),
                ("stopped by the test", &None),
                "precondition: stopped right after a keep that kept nothing"
            );
            assert_eq!(
                *closed_after_the_keep.lock().unwrap(),
                Some(true),
                "the keep step closed accepts, though nothing waited"
            );
            assert!(
                !db.finder_accepts_closed().unwrap(),
                "a sign-out that kept nothing and stopped reopens accepts"
            );
        })
        .await;
    }

    /// U45: a quit between the keep step and the purge leaves the key on disk; the next launch reopens accepts first,
    /// before the restore can start an engine. The reopen is the first statement of `restore_session_inner`, never in
    /// the wrapper, whose two statements the startup-restore pin holds exact.
    #[test]
    fn accepts_reopen_when_the_app_starts() {
        let dir = tempfile::tempdir().unwrap();
        let db = state_db::StateDb::open(state_paths::state_db_path_from_state_dir(dir.path())).unwrap();
        db.close_finder_accepts().unwrap();
        reopen_finder_accepts_in(dir.path());
        assert!(!db.finder_accepts_closed().unwrap());
        let production = super::finder_setup_command_tests::production_source();
        let restore =
            super::finder_setup_command_tests::body_between(&production, "async fn restore_session_inner(", "\n}\n");
        let reopen = restore
            .find("reopen_finder_accepts_at_startup()")
            .expect("the launch reopens accepts");
        assert!(
            reopen < restore.find("restore_stored_session(").expect("the restore"),
            "before the restore"
        );
        assert!(
            reopen < restore.find("start_engine_if_possible(").expect("the engine start"),
            "before any engine can start"
        );
        let first = restore
            .lines()
            .skip(1)
            .map(str::trim)
            .find(|line| !line.is_empty() && !line.starts_with("//"))
            .unwrap();
        assert_eq!(
            first, "#[cfg(target_os = \"macos\")]",
            "the reopen opens the body:\n{restore}"
        );
    }

    /// U45: the launch's reopen acts on the app's own state database (here the test's, through `in_own_dirs`), not
    /// only on a directory its caller names, and it clears the key alone: the R10 owner record and a Finder removal
    /// debt stay, or the restore's engine start would reset the local data as unowned.
    #[tokio::test]
    async fn the_launch_reopens_accepts_in_the_apps_state_db() {
        in_own_dirs(async {
            let db = state_db_in_scratch();
            let owner = crate::account_binding::Identity::new(Some("u-a"), Some("a@beebeeb.io"));
            db.set_owner(&owner).unwrap();
            db.set_finder_removal_owed(true).unwrap();
            db.close_finder_accepts().unwrap();
            reopen_finder_accepts_at_startup();
            assert!(
                !db.finder_accepts_closed().unwrap(),
                "the launch reopens accepts in the app's state.db"
            );
            assert_eq!(db.owner().unwrap(), Some(owner), "the launch's reopen leaves the owner");
            assert!(
                db.finder_removal_owed().unwrap(),
                "the launch's reopen leaves a removal debt"
            );
        })
        .await;
    }

    /// The launch's reopen is best effort. A state database it cannot open is logged by its step, never by the error,
    /// whose text names the database's path (W10).
    #[tokio::test]
    async fn a_launch_reopen_that_cannot_open_the_state_db_logs_the_step() {
        in_own_dirs(async {
            let dir = state_paths::beebeeb_state_dir().unwrap();
            std::fs::create_dir_all(state_paths::state_db_path_from_state_dir(&dir)).unwrap();
            let logs = capture_logs(reopen_finder_accepts_at_startup);
            let path = dir.to_string_lossy().into_owned();
            assert!(!logs.contains(&path), "no log carries the path:\n{logs}");
            let warning = logs
                .lines()
                .find(|line| line.contains("could not reopen Finder accepts"))
                .unwrap_or_else(|| panic!("the failed open is logged:\n{logs}"));
            assert!(
                warning.contains("step=state_db"),
                "the warning names the step: {warning}"
            );
        })
        .await;
    }

    /// U26.
    #[test]
    fn show_in_finder_reveals_only_the_saved_folder() {
        assert_eq!(reveal_target(Some("/kept/a"), "/kept/a"), Ok("/kept/a"));
        assert!(reveal_target(Some("/kept/a"), "/Users/a/Desktop").is_err());
        assert!(reveal_target(None, "/kept/a").is_err());
    }

    /// U31.
    #[test]
    fn show_in_finder_uses_the_opener_reveal() {
        let production = super::finder_setup_command_tests::production_source();
        let body = super::finder_setup_command_tests::body_between(&production, "fn show_kept_unsent_folder(", "\n}\n");
        assert!(body.contains("reveal_item_in_dir("), "{body}");
        assert!(
            !body.contains("Command::new(\"open\")") && !body.contains("\"/usr/bin/open\""),
            "{body}"
        );
        assert!(
            body.contains("\"kept folder reveal requested\""),
            "D2 reads this line from stdout"
        );
    }

    /// Everything `tracing` emits on this thread at `info` and above while `body` runs, as text: a copy of
    /// `engine_bridge.rs`'s `capture_logs_async`. `#[tokio::test]` runs the future on the test's own thread, so a
    /// thread-local subscriber sees all of it.
    async fn capture_logs_async(body: impl std::future::Future<Output = ()>) -> String {
        #[derive(Clone)]
        struct Capture(Arc<Mutex<Vec<u8>>>);
        impl std::io::Write for Capture {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let writer = Capture(buffer.clone());
        // tracing caches each callsite's interest for the whole process. While only ONE dispatcher is alive,
        // tracing-core takes that interest from the registering thread's own default (`has_just_one`), so a callsite
        // first hit by another test thread, which has no subscriber, is cached as "never" and this capture misses its
        // events. A second live dispatcher keeps registration on the all-dispatchers path.
        let _second_dispatcher = tracing::Dispatch::new(tracing_subscriber::registry());
        let subscriber = tracing_subscriber::fmt()
            .with_writer(move || writer.clone())
            .with_ansi(false)
            .with_max_level(tracing::Level::INFO)
            .finish();
        let guard = tracing::subscriber::set_default(subscriber);
        body.await;
        drop(guard);
        let bytes = buffer.lock().unwrap().clone();
        String::from_utf8(bytes).unwrap()
    }

    /// U25 across a keeping sign-out: the keep step's own capture (`sign_out::keep`'s test) misses every line the
    /// sign-out logs around it. No line of the whole sign-out names a kept path or a file name.
    #[tokio::test]
    async fn a_keeping_sign_out_logs_no_path_or_name() {
        in_own_dirs(async {
            let db = state_db_in_scratch();
            db.hold_lock_for_test()
                .execute(
                    "INSERT INTO files (file_id, path) VALUES ('secret', 'Secret Project/plan for Q4.docx')",
                    [],
                )
                .unwrap();
            queue_write(&db, "ws", "secret", b"plan"); // its `INSERT OR IGNORE` keeps the row above
            let (state, _acct) = signed_out_state();
            recording_finder(&state);
            let id = ask(&state, "main");
            let logs = capture_logs_async(async {
                let done = sign_out_with(&state, Some(&id), no_seam()).await.unwrap();
                assert_eq!(done.kept_unsent.map(|kept| kept.files), Some(1));
            })
            .await;
            assert!(logs.contains("kept unsent changes"), "{logs}");
            for leaked in ["Secret Project", "plan for Q4", ".docx", "Changes not sent"] {
                assert!(!logs.contains(leaked), "{leaked} in {logs}");
            }
        })
        .await;
    }

    /// W9, §9: both sign-out routes surface both folders, through one function.
    #[test]
    fn both_sign_out_routes_surface_both_folders() {
        let production = super::finder_setup_command_tests::production_source();
        let command = super::finder_setup_command_tests::body_between(&production, "async fn clear_session(", "\n}\n");
        assert!(command.contains("surface_sign_out_folders(&app, &result)"), "{command}");
        let menu = &production[production.find("DesktopMenuAction::SignOut =>").unwrap()..];
        let menu = &menu[..menu.find("DesktopMenuAction::Quit").unwrap()];
        assert!(menu.contains("surface_sign_out_folders(&app, &result)"), "{menu}");
    }

    /// W9 and §9, inside the one function both routes call: macOS's folder is saved for its row (which tells the
    /// windows), ours (the keep step recorded it) only tells them, and the one alert names both, ours first. The
    /// function needs the app's dialog, so its body is read; the folders it reads come out of the result, checked here
    /// for a sign-out that completed and one that stopped after the keep step.
    #[test]
    fn the_sign_out_surface_names_our_folder_and_saves_macos_folder_for_its_row() {
        let production = super::finder_setup_command_tests::production_source();
        let surface = crate::source_pin::squeeze(super::finder_setup_command_tests::body_between(
            &production,
            "fn surface_sign_out_folders(",
            "\n}\n",
        ));
        for step in [
            "let (macos, ours) = sign_out_folders(result);",
            "Some(location) => save_kept_folder_for_row(app, location),",
            "None if ours.is_some() => emit_kept_folder_changed(app),",
            "sign_out::copy::sign_out_alert(ours.map(|kept| (kept.folder.as_str(), kept.files)), macos)",
        ] {
            assert!(
                surface.contains(&crate::source_pin::squeeze(step)),
                "surface_sign_out_folders: {step}\n{surface}"
            );
        }
        let kept = KeptUnsent {
            folder: "/ours".into(),
            files: 2,
        };
        let done: Result<SignedOut, SignOutFailure> = Ok(SignedOut {
            preserved_location: Some("/macos".into()),
            kept_unsent: Some(kept.clone()),
            ..SignedOut::confirmed(SignOutOutcome::Completed)
        });
        assert_eq!(
            sign_out_folders(&done),
            (Some("/macos"), Some(&kept)),
            "a sign-out that completed"
        );
        let stopped: Result<SignedOut, SignOutFailure> = Err(SignOutFailure {
            message: "stopped after the keep step".into(),
            preserved_location: Some("/macos".into()),
            kept_unsent: Some(kept.clone()),
        });
        assert_eq!(
            sign_out_folders(&stopped),
            (Some("/macos"), Some(&kept)),
            "a sign-out that stopped after the keep step"
        );
    }

    /// §9: "Show in Finder" reveals a path only when it equals the saved record, so the webview cannot have the app
    /// reveal any other path. U26 holds the comparison; this holds the command to it: it reads our record (never
    /// 1882's), checks the path against it, and reveals only what the check returned.
    #[test]
    fn show_in_finder_reveals_only_what_the_saved_record_allows() {
        let production = super::finder_setup_command_tests::production_source();
        let body = crate::source_pin::squeeze(super::finder_setup_command_tests::body_between(
            &production,
            "fn show_kept_unsent_folder(",
            "\n}\n",
        ));
        let squeeze = crate::source_pin::squeeze;
        let read = body
            .find(&squeeze(
                "let saved = sign_out::kept_record::saved_kept_unsent_at(&DesktopConfig::path()?)?;",
            ))
            .unwrap_or_else(|| panic!("our record is read: {body}"));
        let check = body
            .find(&squeeze("let target = reveal_target(saved.as_deref(), &path)?;"))
            .unwrap_or_else(|| panic!("the path is checked against it: {body}"));
        let reveal = body
            .find(".reveal_item_in_dir(target)")
            .unwrap_or_else(|| panic!("only the checked path is revealed: {body}"));
        assert!(read < check && check < reveal, "read, check, then reveal: {body}");
        assert_eq!(body.matches("reveal_item_in_dir(").count(), 1, "one reveal: {body}");
    }

    /// §9, the row's other two commands: the row reads our record (never 1882's), its "Dismiss" goes through our
    /// record's rule (only the folder the row showed) and then tells the windows, and the webview can call all three.
    #[test]
    fn the_rows_commands_use_our_record_and_are_registered() {
        let production = super::finder_setup_command_tests::production_source();
        let squeeze = crate::source_pin::squeeze;
        let read = squeeze(super::finder_setup_command_tests::body_between(
            &production,
            "fn kept_unsent_folder(",
            "\n}\n",
        ));
        assert!(
            read.contains(&squeeze(
                "sign_out::kept_record::saved_kept_unsent_at(&DesktopConfig::path()?)"
            )),
            "the row reads our record: {read}"
        );
        let dismiss = squeeze(super::finder_setup_command_tests::body_between(
            &production,
            "fn dismiss_kept_unsent_folder(",
            "\n}\n",
        ));
        let cleared = dismiss
            .find(&squeeze(
                "sign_out::kept_record::dismiss_saved_kept_unsent_at(&DesktopConfig::path()?, &path)?;",
            ))
            .unwrap_or_else(|| panic!("the dismiss goes through our record's rule: {dismiss}"));
        let told = dismiss
            .find("emit_kept_folder_changed(&app);")
            .unwrap_or_else(|| panic!("the windows are told: {dismiss}"));
        assert!(cleared < told, "dismissed, then told: {dismiss}");
        let handler = super::finder_setup_command_tests::body_between(&production, "tauri::generate_handler![", "])");
        for command in [
            "kept_unsent_folder",
            "dismiss_kept_unsent_folder",
            "show_kept_unsent_folder",
        ] {
            assert_eq!(
                handler
                    .lines()
                    .filter(|line| line.trim() == format!("{command},"))
                    .count(),
                1,
                "{command} is registered once"
            );
        }
    }

    /// U34 (f), and (a)'s disarm.
    #[tokio::test]
    async fn the_send_phase_is_owned_by_rust() {
        in_own_dirs(async {
            let db = state_db_in_scratch();
            queue_write(&db, "w1", "f1", b"bytes");
            // Backing off far into the future (2100): only the phase's start makes it due now (§6.2).
            db.hold_lock_for_test()
                .execute(
                    "UPDATE operation_queue SET next_retry_at = 4102444800 WHERE op_id = 'w1'",
                    [],
                )
                .unwrap();
            let next_retry_at = || -> i64 {
                db.hold_lock_for_test()
                    .query_row(
                        "SELECT next_retry_at FROM operation_queue WHERE op_id = 'w1'",
                        [],
                        |r| r.get(0),
                    )
                    .unwrap()
            };
            let state = AppState::default();
            let id = ask(&state, "main");
            let link = Arc::new(runner::SendPhaseLink::default());
            let stale = start_send_phase(&state.sign_out, "stale", "main", Some(link.clone()));
            assert!(stale.err().unwrap().starts_with("unsent_changes: "));
            assert_eq!(next_retry_at(), 4102444800, "a refused start makes nothing due");
            let run = start_send_phase(&state.sign_out, &id, "main", Some(link.clone())).unwrap();
            assert!(run.is_some() && link.is_armed());
            assert!(
                next_retry_at() <= now_unix_seconds(),
                "the start made the live write due now"
            );
            assert!(
                start_send_phase(&state.sign_out, &id, "main", Some(link.clone()))
                    .unwrap()
                    .is_none(),
                "a second start starts nothing"
            );
            end_send_phase(&state.sign_out, Some(&link));
            assert!(!link.is_armed() && state.sign_out.phase.lock().unwrap().is_none());
        })
        .await;
    }

    /// PR #130 review, "Do not treat a dead runner handle as a running engine": a runner whose task ended at startup
    /// (the state dir, the database or the lock failed) leaves its handle in the slot. That is not a running engine:
    /// `send_conditions` must say so, hand the phase no link, and the count must go straight to the honest ask
    /// (`engine_stopped`), not offer a send nobody would run.
    #[tokio::test]
    async fn a_runner_that_ended_is_not_a_running_engine() {
        use sign_out::waiting::{NotSending, readiness};
        let state = AppState::default();
        synthesize_single_account(&state, AccountId("dead-runner".to_string()));
        let acct = state.active_account().unwrap();
        *acct.session.lock().unwrap() = Some(Session {
            token: "tok".into(),
            master_key: [9u8; 32],
            email: None,
        });

        // A runner whose task is still going: running, with its link.
        let (hold, held) = tokio::sync::oneshot::channel::<()>();
        *acct.engine.lock().await = Some(EngineRunner::for_test_with_task(tokio::spawn(async move {
            let _ = held.await;
        })));
        let (conditions, link) = send_conditions(&state).await;
        assert!(conditions.engine_running && link.is_some(), "a live runner is a running engine");
        assert_eq!(readiness(conditions, false, true), None, "and it can send");

        // The same slot, its task gone while the handle stays in it.
        let _ = hold.send(());
        let task = tokio::spawn(async {});
        while !task.is_finished() {
            tokio::task::yield_now().await;
        }
        *acct.engine.lock().await = Some(EngineRunner::for_test_with_task(task));
        let (conditions, link) = send_conditions(&state).await;
        assert!(!conditions.engine_running, "a runner whose task ended is not a running engine");
        assert!(link.is_none(), "and the phase gets no link to arm");
        assert_eq!(
            readiness(conditions, false, true),
            Some(NotSending::EngineStopped),
            "the count goes straight to the ask with the engine-stopped line"
        );
    }

    /// U46, the chain past the bridge: what the runner noted on the phase's link ends the phase at the driver's first
    /// observation of it, long before the limit: `account` for a refusal the account cures, and `offline` and
    /// `session` for an attempt's link failure and refused session (U16 reads those from an attempt alone).
    /// `observe_phase` is the driver's own step, so this is the driver's reading, not a copy of it.
    #[test]
    fn an_attempts_refusal_ends_the_phase_at_its_first_observation() {
        use crate::engine_bridge::TransferLoopOutcome;
        use sign_out::send_phase::{PhaseEnd, PhaseStep, SIGN_OUT_SEND_LIMIT, SendPhase};
        let waiting = sign_out::waiting::WaitingChanges {
            files: Vec::new(),
            others: vec![sign_out::waiting::OtherChange {
                op_id: "r1".into(),
                live: true,
            }],
        };
        let good = sign_out::waiting::SendConditions {
            engine_running: true,
            vault_unlocked: true,
            session_expired: false,
            link_failed: false,
            sync_paused: false,
        };
        for (outcome, end) in [
            (
                TransferLoopOutcome {
                    account_refused: true,
                    ..Default::default()
                },
                PhaseEnd::Account,
            ),
            (
                TransferLoopOutcome {
                    link_failed: true,
                    ..Default::default()
                },
                PhaseEnd::Offline,
            ),
            (
                TransferLoopOutcome {
                    session_refused: true,
                    ..Default::default()
                },
                PhaseEnd::Session,
            ),
        ] {
            let link = Arc::new(runner::SendPhaseLink::default());
            link.arm();
            let run = PhaseRun {
                owner: "main".into(),
                link: Some(link.clone()),
                stop: Default::default(),
                closed: Default::default(),
                started: std::time::Instant::now(),
                changes_at_start: 1,
            };
            let mut phase = SendPhase::default();
            assert_eq!(
                phase.observe(&observe_phase(&run, &waiting, good)),
                PhaseStep::Progress(1),
                "nothing noted yet: it runs"
            );
            link.note(&outcome);
            let observation = observe_phase(&run, &waiting, good);
            assert!(observation.elapsed < SIGN_OUT_SEND_LIMIT);
            assert_eq!(phase.observe(&observation), PhaseStep::End(end), "{outcome:?}");
            assert_eq!(
                serde_json::to_value(end).unwrap(),
                end.as_str(),
                "the reason the ask's A5 line is chosen by"
            );
        }
    }

    /// §5.1, the contract: `sign_out_count`'s answer.
    #[test]
    fn the_count_answers_the_contract_shape() {
        let snapshot = AskSnapshot::of(&WaitingChanges::default(), "o", Vec::new());
        let json = serde_json::to_value(SignOutCount {
            counts: snapshot.counts,
            can_send: false,
            not_sending: Some(sign_out::waiting::NotSending::Offline),
        })
        .unwrap();
        for key in [
            "askId",
            "files",
            "copiedFiles",
            "others",
            "missingFiles",
            "earlierFolders",
            "canSend",
            "notSending",
        ] {
            assert!(json.get(key).is_some(), "{key} in {json}");
        }
        assert_eq!(json["notSending"], "offline");
    }

    fn phase_owned_by(state: &AppState, owner: &str) -> Arc<std::sync::atomic::AtomicBool> {
        let closed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        *state.sign_out.phase.lock().unwrap() = Some(crate::sign_out::send_phase::RunningPhase {
            ask_id: "a".into(),
            owner: owner.into(),
            stop: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            closed: closed.clone(),
        });
        closed
    }

    /// U34 (b)-(e), (g), (h).
    #[tokio::test]
    async fn the_send_phase_is_owned_by_rust_and_a_close_ends_the_flow() {
        in_own_dirs(async {
            let db = state_db_in_scratch();
            queue_write(&db, "w1", "f1", b"bytes");
            let state = AppState::default();
            // (b) a close during that window's phase: ended `closed`, its snapshot dropped; a later Sign out
            // refuses.
            let id = ask(&state, "main");
            let closed = phase_owned_by(&state, "main");
            let outcome = flow_window_closed(&state.sign_out, "main");
            assert!(outcome.phase_closed && closed.load(Ordering::SeqCst) && outcome.snapshot_dropped);
            let refused = sign_out_with(&state, Some(&id), no_seam()).await.unwrap_err();
            assert!(refused.message.starts_with("unsent_changes: "));
            // (c) at the ask, no phase: the snapshot is dropped.
            *state.sign_out.phase.lock().unwrap() = None;
            ask(&state, "main");
            assert!(flow_window_closed(&state.sign_out, "main").snapshot_dropped);
            // (d) another window's close leaves a phase and a snapshot of `main` alone.
            ask(&state, "main");
            let closed = phase_owned_by(&state, "main");
            for other in ["settings", "onboarding"] {
                let outcome = flow_window_closed(&state.sign_out, other);
                assert!(!outcome.phase_closed && !outcome.snapshot_dropped, "{other}");
            }
            assert!(!closed.load(Ordering::SeqCst));
            *state.sign_out.phase.lock().unwrap() = None;
            // (d), its other half: a close of `main` leaves a snapshot `onboarding` owns alone. (g) A close
            // of `onboarding` drops it.
            ask(&state, "onboarding");
            let outcome = flow_window_closed(&state.sign_out, "main");
            assert!(!outcome.snapshot_dropped, "(d) onboarding's snapshot stays");
            assert!(
                flow_window_closed(&state.sign_out, "onboarding").snapshot_dropped,
                "(g)"
            );
            // (h) running when its window closes, then failing: the dialog, once.
            state.sign_out.late_results.begin("main");
            assert_eq!(flow_window_closed(&state.sign_out, "main").show, None);
            assert!(
                state
                    .sign_out
                    .late_results
                    .complete(
                        "main",
                        Some(crate::sign_out::flow_window::Shown::Failure("x".into()))
                    )
                    .is_some()
            );
        })
        .await;
    }

    /// U34's pins: the hook is called on every close, destroy and reload, with the window's own label; and its premise.
    /// Each call is read inside its own arm, so one arm's call cannot stand in for the other's.
    #[test]
    fn every_close_destroy_and_reload_reaches_the_hook() {
        let production = super::finder_setup_command_tests::production_source();
        let handler = &production[production
            .find(".on_window_event(|window, event| {")
            .expect("the global handler")..];
        let handler = &handler[..handler.find("\n        })\n").unwrap()];
        let block = |source: &'static str, text: &str, opening: &str, closing: &str| -> String {
            let at = text
                .find(opening)
                .unwrap_or_else(|| panic!("{source}: {opening}\n{text}"));
            let end = text[at..]
                .find(closing)
                .unwrap_or_else(|| panic!("{source}: {opening} ends"));
            text[at..at + end].to_string()
        };
        for arm in [
            "if let tauri::WindowEvent::CloseRequested { .. } = event {",
            "if let tauri::WindowEvent::Destroyed = event {",
        ] {
            let arm_block = block("the global handler", handler, arm, "\n            }\n");
            assert!(
                arm_block.contains("sign_out_flow_window_closed(window.app_handle(), window.label())"),
                "{arm}\n{arm_block}"
            );
        }
        let page_load = &production[production.find(".on_page_load(").expect("an on_page_load registration")..];
        let page_load = &page_load[..page_load.find("\n        })\n").unwrap()];
        let started = block(
            "on_page_load",
            page_load,
            "if matches!(payload.event(), tauri::webview::PageLoadEvent::Started) {",
            "\n            }\n",
        );
        assert!(
            started.contains("sign_out_flow_window_closed(webview.app_handle(), webview.label())"),
            "{page_load}"
        );
        // Task 1683 slice 1 (two-surface §8.2): the flow's window is now the Beebeeb window, and a close destroys
        // it. The premise moved: the hook runs on `CloseRequested` AND on `Destroyed` (both arms pinned above), so a
        // destroyed window still ends its flow; spec 1887 §5.1, "A close ends the flow", already covers a destroy.
        assert_eq!(
            surfaces::policy::close_policy(surfaces::policy::Platform::Macos, "main"),
            surfaces::registry::ClosePolicy::Destroy,
            "the premise: a close destroys the Beebeeb window"
        );
    }

    /// U32, extended: the late result is macOS only, defined and registered. It reads the first definition of each
    /// name, so every macOS command sits above its non-macOS twin, and that item's own attribute lines, so the `cfg`
    /// of the item above cannot stand in for its own; and each call's own statement.
    #[test]
    fn the_late_result_is_macos_only() {
        let production = super::finder_setup_command_tests::production_source();
        let macos = "#[cfg(target_os = \"macos\")]";
        for definition in [
            "fn sign_out_result_ack(",
            "fn sign_out_flow_window_closed(",
            "fn sign_out_count(",
            "fn sign_out_send_start(",
        ] {
            let at = production.find(definition).unwrap_or_else(|| panic!("{definition}"));
            let line = production[..at].rfind('\n').map_or(0, |end| end + 1);
            let attributes: Vec<&str> = production[..line]
                .lines()
                .rev()
                .map(str::trim)
                .take_while(|above| above.starts_with("#[") || above.starts_with("///"))
                .collect();
            assert!(attributes.contains(&macos), "{definition}: {attributes:?}");
        }
        let handler = super::finder_setup_command_tests::body_between(&production, "tauri::generate_handler![", "])");
        assert_eq!(
            handler
                .lines()
                .filter(|line| line.trim() == "sign_out_result_ack,")
                .count(),
            1,
            "registered once"
        );
        // Read squeezed, as rustfmt lays the `complete` statement over several lines.
        let squeeze = crate::source_pin::squeeze;
        let command = squeeze(super::finder_setup_command_tests::body_between(
            &production,
            "async fn clear_session(",
            "\n}\n",
        ));
        for statement in [
            "state.sign_out.late_results.begin(",
            "if let Some(shown) = state.sign_out.late_results.complete(",
        ] {
            let at = command
                .find(&squeeze(statement))
                .unwrap_or_else(|| panic!("{statement}\n{command}"));
            assert!(command[..at].ends_with(&squeeze(macos)), "{statement}\n{command}");
        }
    }

    /// U44 (b) and (e), through the classification `clear_session` hands to the record (§5.1 "A late result"): a
    /// failure and a completed sign-out's warning need showing, each with its own text; an `unsent_changes:` refusal
    /// and a completed sign-out with no warning do not, and leave nothing for a close to show.
    #[test]
    fn a_late_result_is_shown_only_when_it_needs_showing() {
        use crate::sign_out::flow_window::{LateResults, Shown};
        let failed = |message: &str| -> Result<SignedOut, SignOutFailure> {
            Err(SignOutFailure {
                message: message.into(),
                preserved_location: None,
                kept_unsent: None,
            })
        };
        let done = |warning| -> Result<SignedOut, SignOutFailure> {
            Ok(SignedOut {
                warning,
                ..SignedOut::confirmed(SignOutOutcome::Completed)
            })
        };
        let refused = coded(UNSENT_CHANGES_CODE, E1_UNSENT_CHANGES);
        let not_kept = coded(UNSENT_COPY_FAILED_CODE, E2_UNSENT_COPY_FAILED);
        let cases = [
            (failed(&refused), None),
            (failed(&not_kept), Some(Shown::Failure(not_kept.clone()))),
            (
                failed(SIGN_OUT_PURGE_FAILED),
                Some(Shown::Failure(SIGN_OUT_PURGE_FAILED.into())),
            ),
            (
                done(Some(ActionWarning::FinderRemovalUnconfirmed)),
                Some(Shown::Warning(FINDER_SIGN_OUT_UNCONFIRMED_WARNING.into())),
            ),
            (done(None), None),
        ];
        for (result, expected) in cases {
            assert_eq!(late_result_of(&result), expected, "{result:?}");
            // What a close then finds: that result once, or nothing.
            let late = LateResults::default();
            late.begin("main");
            assert_eq!(late.complete("main", late_result_of(&result)), None);
            assert_eq!(
                late.window_closed("main"),
                expected,
                "a close after {result:?}"
            );
            assert_eq!(late.window_closed("main"), None);
        }
    }

    /// §5.1 "A late result", the glue the pure tests cannot run (it needs the app's dialog and a Wry window), read from
    /// its source: the call's record begins before the sign-out and completes after it with the classified result,
    /// and what that returns is shown; the hook reads the closed window's own flow state, tells that window, and shows
    /// what it took out; the acknowledgement names the calling window; the dialog keeps the menu's two titles.
    #[test]
    fn the_late_result_reaches_the_dialog_from_the_call_and_the_hook() {
        use crate::source_pin::squeeze;
        let production = super::finder_setup_command_tests::production_source();
        let body = |start: &str| {
            squeeze(super::finder_setup_command_tests::body_between(
                &production,
                start,
                "\n}\n",
            ))
        };
        let command = body("async fn clear_session(");
        let begin = command
            .find(&squeeze("state.sign_out.late_results.begin(window.label());"))
            .unwrap_or_else(|| panic!("the record begins: {command}"));
        let sign_out = command
            .find("clear_session_impl(")
            .unwrap_or_else(|| panic!("the sign-out: {command}"));
        let complete = command
            .find(&squeeze(
                "if let Some(shown) = state.sign_out.late_results.complete(window.label(), late_result_of(&result)) {
                    show_late_result(&app, shown);
                }",
            ))
            .unwrap_or_else(|| {
                panic!("the record completes with the classified result, and a Some is shown: {command}")
            });
        assert!(
            begin < sign_out && sign_out < complete,
            "begin, the sign-out, then complete: {command}"
        );
        let hook = body("fn sign_out_flow_window_closed(");
        for step in [
            "let closed = flow_window_closed(&state.sign_out, label);",
            "let _ = app.emit_to(label, \"sign-out-flow-closed\", ());",
            "if let Some(shown) = closed.show { show_late_result(app, shown); }",
        ] {
            assert!(hook.contains(&squeeze(step)), "the hook: {step}\n{hook}");
        }
        let ack = body("fn sign_out_result_ack(");
        assert!(
            ack.contains(&squeeze("state.sign_out.late_results.acknowledge(window.label());")),
            "the acknowledgement names the calling window: {ack}"
        );
        let dialog = body("fn show_late_result(");
        for step in [
            "Shown::Failure(message) => (\"Sign-out paused\", message, tauri_plugin_dialog::MessageDialogKind::Error)",
            "Shown::Warning(sentence) => (\"Signed out\", sentence, tauri_plugin_dialog::MessageDialogKind::Info)",
            ".message(message).title(title).kind(kind).show(",
        ] {
            assert!(dialog.contains(&squeeze(step)), "the dialog: {step}\n{dialog}");
        }
    }
    /// U19 (macOS half; the frontend half is F8, the Linux case is F8's last line).
    #[tokio::test]
    async fn the_switch_carries_a_snapshot_on_macos_only() {
        in_own_dirs(async {
            let db = state_db_in_scratch();
            queue_write(&db, "w1", "f1", b"bytes");
            queue_rename(&db, "r1");
            let state = AppState::default();
            let unsent = switch_snapshot(&state).unwrap().expect("a snapshot on macOS");
            assert_eq!((unsent.files, unsent.others), (1, 1));
            assert_eq!(
                state.sign_out.snapshots.newest_ask_id().as_deref(),
                Some(unsent.ask_id.as_str()),
                "recorded as the newest"
            );
            assert!(state.sign_out.phase.lock().unwrap().is_none(), "no send is started");
            let json = serde_json::to_value(LoginOutcome::account_mismatch(2, Some(unsent), None)).unwrap();
            assert_eq!(json["account_mismatch"]["pending_changes"], 2);
            assert!(json["account_mismatch"]["unsent"]["askId"].is_string());
            assert!(
                json["account_mismatch"].get("unsent_error").is_none(),
                "absent while the count worked"
            );
            assert!(
                state
                    .sign_out
                    .snapshots
                    .drop_owned_by(surfaces::policy::main_window_label(surfaces::policy::Platform::current())),
                "owned by the window the switch step runs in (the Beebeeb window on a Mac, not the retired onboarding window)"
            );
            let without = serde_json::to_value(LoginOutcome::account_mismatch(2, None, None)).unwrap();
            assert_eq!(
                without["account_mismatch"],
                serde_json::json!({ "pending_changes": 2 }),
                "elsewhere: unchanged"
            );
        })
        .await;
    }

    /// Spec §5.3 row 1: a switch count that cannot be read is E3, a line in the switch step. The sign-in settles as a
    /// switch, with no snapshot and the coded E3; it is never a failed sign-in.
    #[test]
    fn a_switch_count_that_fails_is_a_line_in_the_step() {
        let e3 = sign_out::copy::coded(
            sign_out::copy::UNSENT_COUNT_FAILED_CODE,
            sign_out::copy::E3_UNSENT_COUNT_FAILED,
        );
        let SignInSettlement::AccountMismatch {
            pending_changes,
            unsent,
            unsent_error,
        } = switch_settlement(3, Err(e3.clone()))
        else {
            panic!("a switch, not a failure");
        };
        assert_eq!(
            (pending_changes, unsent, unsent_error.as_deref()),
            (3, None, Some(e3.as_str()))
        );
        let json = serde_json::to_value(LoginOutcome::account_mismatch(3, None, Some(e3.clone()))).unwrap();
        assert_eq!(
            json["account_mismatch"],
            serde_json::json!({ "pending_changes": 3, "unsent_error": e3 })
        );
        let production = super::finder_setup_command_tests::production_source();
        let settle = super::finder_setup_command_tests::body_between(&production, "async fn settle_sign_in(", "\n}\n");
        assert!(
            settle.contains("switch_settlement(pending_changes, switch_snapshot(state))"),
            "{settle}"
        );
    }

    /// U21: the menu never purges a waiting write and never shows the error dialog for it.
    #[test]
    fn the_menu_never_purges_a_waiting_write() {
        let state = AppState::default();
        state.sign_out.flow_request.store(true, Ordering::SeqCst);
        state.sign_out.flow_request.store(true, Ordering::SeqCst);
        assert!(take_sign_out_flow_request(&state.sign_out), "one request");
        assert!(!take_sign_out_flow_request(&state.sign_out), "not two");
        let production = super::finder_setup_command_tests::production_source();
        let menu = &production[production.find("DesktopMenuAction::SignOut =>").unwrap()..];
        let menu = &menu[..menu.find("DesktopMenuAction::Quit").unwrap()];
        let flow = menu
            .find("Err(error) if error.message.starts_with(\"unsent_changes: \") => open_sign_out_flow(&app),")
            .expect("the flow arm");
        let paused = menu.find(".title(\"Sign-out paused\")").unwrap();
        assert!(flow < paused, "the flow arm comes before the error dialog's arm");
        assert!(
            menu.contains("clear_session_impl(&state, false, None, crate::UnknownFilesArg::Keep, None)"),
            "the menu never keeps on its own"
        );
    }

    /// F9's Rust half: the compact page's request survives a Settings window that has not loaded yet.
    #[test]
    fn a_flow_request_waits_for_settings_to_consume_it() {
        let state = AppState::default();
        assert!(!take_sign_out_flow_request(&state.sign_out));
        assert!(
            !state.sign_out.flow_request.load(Ordering::SeqCst),
            "asking when nothing waits leaves nothing"
        );
        state.sign_out.flow_request.store(true, Ordering::SeqCst);
        assert!(take_sign_out_flow_request(&state.sign_out));
    }

    /// U19, from the count to the window (W11): the switch settles with the snapshot it counted (a Mac) or none
    /// (elsewhere); both password sign-ins hand the settlement's snapshot and E3 on to `LoginOutcome`; and the browser
    /// sign-in hands them on in the same call (spec C C-S1, task 1747 Part 1): it returns the `LoginOutcome` the password
    /// paths return, where it used to put each in an `account_mismatch` event and fail with a raw code. The sign-ins need
    /// a server, so their text is read.
    #[test]
    fn the_switch_count_reaches_the_sign_in_window() {
        let counts = sign_out::snapshot::AskCounts {
            ask_id: "ask-1".to_string(),
            files: 1,
            copied_files: 1,
            others: 1,
            missing_files: 0,
            earlier_folders: Vec::new(),
        };
        assert_eq!(
            switch_settlement(2, Ok(Some(counts.clone()))),
            SignInSettlement::AccountMismatch {
                pending_changes: 2,
                unsent: Some(counts),
                unsent_error: None,
            },
            "a Mac: the snapshot rides the settlement"
        );
        assert_eq!(
            switch_settlement(2, Ok(None)),
            SignInSettlement::AccountMismatch {
                pending_changes: 2,
                unsent: None,
                unsent_error: None,
            },
            "elsewhere: the count alone"
        );
        let squeeze = crate::source_pin::squeeze;
        let production = super::finder_setup_command_tests::production_source();
        for login in ["async fn desktop_login(", "async fn desktop_login_2fa("] {
            let body = squeeze(super::finder_setup_command_tests::body_between(
                &production,
                login,
                "\n}\n",
            ));
            assert!(
                body.contains(&squeeze(
                    "return Ok(LoginOutcome::account_mismatch(pending_changes, unsent, unsent_error));"
                )),
                "{login} hands both on"
            );
        }
        // The browser handoff holds the same hand-off as the two password sign-ins, in the same call, through its returned
        // outcome, and no raw `account_mismatch` event or error string is left in it (C-S1).
        let browser_source = include_str!("browser_login.rs").replace("\r\n", "\n");
        // The handoff's own code, not its tests (which hold the words of what must not be there).
        let browser = squeeze(&browser_source[..browser_source.find("#[cfg(test)]").expect("the tests module")]);
        assert!(
            browser.contains(&squeeze(
                "return Ok(crate::LoginOutcome::account_mismatch(pending_changes, unsent, unsent_error));"
            )),
            "the browser handoff hands both on"
        );
        assert!(
            !browser.contains(concat!("\"account", "_mismatch\"")),
            "no raw code is left in the handoff, as an event or as an error"
        );
    }

    /// Task 1747 Part 1 (review P3-5 on #143): the switch snapshot is owned by the window the switch step runs in. On a Mac
    /// that is `main`, which the app destroys on a close, so a close of `main` drops it, and another window's close leaves
    /// it. It used to be owned by the label `onboarding`, a window a Mac no longer builds, so nothing dropped it.
    #[tokio::test]
    async fn the_switch_snapshot_is_dropped_by_the_window_it_runs_in_and_by_no_other() {
        in_own_dirs(async {
            let db = state_db_in_scratch();
            queue_write(&db, "w1", "f1", b"bytes");
            let state = AppState::default();
            let owner = surfaces::policy::main_window_label(surfaces::policy::Platform::current());
            #[cfg(target_os = "macos")]
            assert_eq!(owner, "main", "the sign-in window of a Mac is the Beebeeb window");
            if switch_snapshot(&state).unwrap().is_some() {
                for other in ["settings", "onboarding", "popover", "conflict-7f3a"] {
                    assert!(
                        !flow_window_closed(&state.sign_out, other).snapshot_dropped,
                        "{other}: another window's close leaves the switch snapshot"
                    );
                }
                assert!(state.sign_out.snapshots.newest_ask_id().is_some());
                assert!(
                    flow_window_closed(&state.sign_out, owner).snapshot_dropped,
                    "the window the step runs in drops it"
                );
                assert_eq!(state.sign_out.snapshots.newest_ask_id(), None);
            }
        })
        .await;
    }

    #[test]
    fn the_switch_snapshot_is_counted_for_the_window_label_not_a_literal() {
        let production = super::finder_setup_command_tests::production_source();
        let snapshot = super::finder_setup_command_tests::body_between(&production, "fn switch_snapshot(", "\n}\n");
        assert!(
            snapshot.contains("count_ask(surfaces::policy::main_window_label(surfaces::policy::Platform::current()))?"),
            "{snapshot}"
        );
        assert!(!snapshot.contains("\"onboarding\""), "no window label as a literal:\n{snapshot}");
    }

    // ---- the fix round after the /code-review of the Mac sign-in PR (task 1683 slice 2) ----

    /// Review P2-3: ⌘W destroys the window and its flow with it, and a window opened again must be able to resume “Set up
    /// this Mac”. Rust keeps the step for the process, answers it only while it is still true, and does not use it up by
    /// reading it (the window may be closed again).
    #[test]
    fn a_pending_set_up_step_is_resumed_only_while_it_is_still_true() {
        let alice = account_binding::Identity::new(Some("user-alice"), Some("alice@example.eu"));
        let slot = Mutex::new(None);
        assert_eq!(resume_pending_setup(&slot, true, false, &alice), None, "nothing was set");
        remember_pending_setup(&slot, true, alice.clone());
        assert_eq!(resume_pending_setup(&slot, true, false, &alice), Some(PendingSetup { key_replaced: true }));
        assert_eq!(
            resume_pending_setup(&slot, true, false, &alice),
            Some(PendingSetup { key_replaced: true }),
            "reading it does not use it up"
        );
        assert_eq!(resume_pending_setup(&slot, true, true, &alice), None, "the keys are here: nothing is owed");
        assert_eq!(resume_pending_setup(&slot, false, false, &alice), None, "nobody is signed in: nothing to resume");
        remember_pending_setup(&slot, false, alice.clone());
        assert_eq!(
            resume_pending_setup(&slot, true, false, &alice),
            Some(PendingSetup { key_replaced: false }),
            "the newest sign-in's step replaces an older one"
        );
        forget_pending_setup(&slot);
        assert_eq!(resume_pending_setup(&slot, true, false, &alice), None);
    }

    /// Review P3-4 of the delta review: the step belongs to the account it was set for. A signs in without a key, A signs out
    /// (the sign-out code is untouched and leaves the slot), B signs in and is signed in and locked: B must not be sent to
    /// A's recovery phrase. Only the same account resumes; an identity that cannot be compared resumes nothing.
    #[test]
    fn a_pending_set_up_step_belongs_to_the_account_it_was_set_for() {
        let alice = account_binding::Identity::new(Some("user-alice"), Some("alice@example.eu"));
        let bob = account_binding::Identity::new(Some("user-bob"), Some("bob@example.eu"));
        let slot = Mutex::new(None);
        remember_pending_setup(&slot, true, alice.clone());
        assert_eq!(resume_pending_setup(&slot, true, false, &alice), Some(PendingSetup { key_replaced: true }));
        assert_eq!(resume_pending_setup(&slot, true, false, &bob), None, "another account is signed in and locked");
        assert_eq!(
            resume_pending_setup(&slot, true, false, &account_binding::Identity::default()),
            None,
            "an account that cannot be named is not compared, so nothing resumes"
        );
        // The same user under another spelling of its email, with no user id on one side: the canonical email decides.
        let by_email = account_binding::Identity::new(None, Some(" Alice@Example.eu "));
        let named_by_email = Mutex::new(None);
        remember_pending_setup(&named_by_email, false, account_binding::Identity::new(None, Some("alice@example.eu")));
        assert_eq!(resume_pending_setup(&named_by_email, true, false, &by_email), Some(PendingSetup { key_replaced: false }));
        // B enters the step in its turn: the slot now answers B and no longer A.
        remember_pending_setup(&slot, false, bob.clone());
        assert_eq!(resume_pending_setup(&slot, true, false, &bob), Some(PendingSetup { key_replaced: false }));
        assert_eq!(resume_pending_setup(&slot, true, false, &alice), None);
    }

    #[test]
    fn arriving_keys_forget_the_pending_set_up_step() {
        let alice = account_binding::Identity::new(Some("user-alice"), Some("alice@example.eu"));
        for from in [KeysFrom::Unlock, KeysFrom::SignIn] {
            let state = AppState::default();
            remember_pending_setup(&state.pending_setup, false, alice.clone());
            keys_arrived(&state, from);
            assert_eq!(resume_pending_setup(&state.pending_setup, true, false, &alice), None, "{from:?}");
        }
    }

    /// The window reads `{ "key_replaced": bool }` strictly (`flowForPendingSetup`), and an absent step is JSON null.
    #[test]
    fn the_pending_set_up_step_is_the_json_the_window_reads() {
        assert_eq!(
            serde_json::to_value(PendingSetup { key_replaced: true }).unwrap(),
            serde_json::json!({ "key_replaced": true })
        );
        assert_eq!(serde_json::to_value(Option::<PendingSetup>::None).unwrap(), serde_json::Value::Null);
    }

    #[test]
    fn the_pending_set_up_commands_are_registered_and_touch_no_secret() {
        let production = super::finder_setup_command_tests::production_source();
        let handler = super::finder_setup_command_tests::body_between(&production, "tauri::generate_handler![", "])");
        // Whole lines: `set_pending_setup,` contains `pending_setup,`, so a substring test cannot tell the two apart.
        for name in ["set_pending_setup,", "pending_setup,"] {
            assert!(handler.lines().any(|line| line.trim() == name), "{name} is registered");
        }
        for signature in ["fn set_pending_setup(", "fn pending_setup("] {
            let body = super::finder_setup_command_tests::body_between(&production, signature, "\n}\n");
            for forbidden in ["keychain", "Keychain", "master_key", "token"] {
                assert!(!body.contains(forbidden), "{signature} must not touch {forbidden}:\n{body}");
            }
        }
        // It asks the same thing `sync_status` does (a session in memory, or a token present) and nothing more.
        let ask = super::finder_setup_command_tests::body_between(&production, "fn pending_setup(", "\n}\n");
        assert!(
            ask.contains("resume_pending_setup(\n        &state.pending_setup,\n        vault_unlocked || auth_present,\n        vault_unlocked,\n        &pending_setup_identity(&acct),\n    )"),
            "{ask}"
        );
        // Both commands name the account the same way: the one whose credentials this Mac holds.
        let set = super::finder_setup_command_tests::body_between(&production, "fn set_pending_setup(", "\n}\n");
        assert!(set.contains("pending_setup_identity(&acct)"), "{set}");
    }

    /// Review P2-3 b: the Keychain holds a token and no key. `unlock_vault` says the approved Set up this Mac sentence on a
    /// Mac (the window recognises it; `tests/signInModel.test.ts` pins the TypeScript copy equal), not the Keychain's words.
    #[test]
    fn unlock_vault_says_the_set_up_sentence_when_the_key_is_not_there() {
        let keychain = "unlock vault key from Keychain: secret not found".to_string();
        #[cfg(target_os = "macos")]
        {
            assert_eq!(unlock_failure(keychain), NO_VAULT_KEY_UNLOCK_ERROR);
            assert_eq!(
                NO_VAULT_KEY_UNLOCK_ERROR,
                "This Mac does not have your encryption keys yet. Restore them to continue."
            );
        }
        #[cfg(not(target_os = "macos"))]
        assert_eq!(unlock_failure(keychain.clone()), keychain, "Windows and Linux keep what they showed");
        // Any other failure is shown as it is.
        for other in ["load session from Keychain: the Keychain is locked", "Sign in before unlocking the vault.", ""] {
            assert_eq!(unlock_failure(other.to_string()), other);
        }
    }

    #[test]
    fn unlock_vault_maps_only_the_keychain_load_and_never_the_session_it_installs() {
        let production = super::finder_setup_command_tests::production_source();
        let unlock = super::finder_setup_command_tests::body_between(&production, "async fn unlock_vault(", "\n}\n");
        assert!(
            unlock.contains("load_session_from_keychain(acct.id.as_str(), email)\n            .map_err(unlock_failure)?"),
            "{unlock}"
        );
        // The startup restore's own test of the same failure is untouched: it reads the Keychain's words.
        let restore = super::finder_setup_command_tests::body_between(&production, "async fn restore_session_inner(", "\n}\n");
        assert!(!restore.contains("unlock_failure"), "{restore}");
    }

    // ---- task 1747 Part 2: the Keychain-presence signal (`vault_unlock_route`) ----

    /// A store that answers ONE question, “is a vault key stored?”, and panics on anything that reads or writes a secret: a
    /// route that read the key (or anything else) would fail the test that uses it.
    struct HoldsAKey(fn() -> keychain::AuthResult<bool>);

    impl keychain::AuthSecretStore for HoldsAKey {
        fn save_session_token(&self, _: &SessionToken) -> keychain::AuthResult<()> {
            unreachable!("the route writes nothing")
        }
        fn load_session_token(&self) -> keychain::AuthResult<Option<SessionToken>> {
            unreachable!("the route reads no token")
        }
        fn delete_session_token(&self) -> keychain::AuthResult<()> {
            unreachable!("the route deletes nothing")
        }
        fn save_wrapped_master_key(&self, _: SecretBytes) -> keychain::AuthResult<()> {
            unreachable!("the route writes nothing")
        }
        fn load_wrapped_master_key(&self) -> keychain::AuthResult<Option<SecretBytes>> {
            unreachable!("the route never reads the key")
        }
        fn delete_wrapped_master_key(&self) -> keychain::AuthResult<()> {
            unreachable!("the route deletes nothing")
        }
        fn holds_wrapped_master_key(&self) -> keychain::AuthResult<bool> {
            (self.0)()
        }
    }

    const ABSENT: HoldsAKey = HoldsAKey(|| Ok(false));
    const PRESENT: HoldsAKey = HoldsAKey(|| Ok(true));
    const NOT_FOUND: HoldsAKey = HoldsAKey(|| Err(keychain::AuthStoreError::NotFound));
    const UNREADABLE: HoldsAKey = HoldsAKey(|| Err(keychain::AuthStoreError::Backend("the Keychain is locked".into())));

    fn route_over(id_keyed: &HoldsAKey, legacy: &HoldsAKey) -> VaultUnlockRoute {
        unlock_route(vault_key_in_either_layout(id_keyed, legacy))
    }

    /// Item 1, through the real account-slot function and the in-memory Keychain a test build resolves.
    #[test]
    fn the_route_is_keychain_when_the_key_is_there() {
        use keychain::AuthSecretStore as _;
        let id = "route-the-key-is-there";
        AuthVault::new(platform_keychain_store_for(id))
            .store_wrapped_master_key(SecretBytes::new_master_key(&[7u8; 32]))
            .unwrap();
        assert_eq!(vault_unlock_route_for(id), VaultUnlockRoute::Keychain);
        assert_eq!(route_over(&PRESENT, &ABSENT), VaultUnlockRoute::Keychain);
        platform_keychain_store_for(id).delete_wrapped_master_key().unwrap();
    }

    /// Item 2: a session and no key (the interrupted first run) is the recovery phrase. `NotFound` and a store that cannot hold
    /// secrets say the same as an empty one.
    #[test]
    fn the_route_is_recovery_phrase_when_neither_layout_holds_a_key() {
        use keychain::AuthSecretStore as _;
        let id = "route-no-key-anywhere";
        platform_keychain_store_for(id)
            .save_session_token(&SessionToken::new("tok-no-key".to_string()).unwrap())
            .unwrap();
        assert_eq!(vault_unlock_route_for(id), VaultUnlockRoute::RecoveryPhrase, "a token is not a key");
        assert_eq!(route_over(&ABSENT, &ABSENT), VaultUnlockRoute::RecoveryPhrase);
        assert_eq!(route_over(&NOT_FOUND, &NOT_FOUND), VaultUnlockRoute::RecoveryPhrase);
        platform_keychain_store_for(id).delete_session_token().unwrap();
    }

    /// Item 3: a key that only the legacy flat layout holds is a key (the unlock reads it as a fallback).
    #[test]
    fn the_route_reads_the_legacy_layout_too() {
        assert_eq!(route_over(&ABSENT, &PRESENT), VaultUnlockRoute::Keychain);
        assert_eq!(route_over(&PRESENT, &PRESENT), VaultUnlockRoute::Keychain);
    }

    /// Item 4, pinned so nobody “fixes” it: a Keychain that cannot be read routes to the Keychain unlock, whose own error then
    /// reaches the person. Set up this Mac would end by writing a key over one that may well be there.
    #[test]
    fn an_unreadable_keychain_routes_to_the_keychain_unlock() {
        assert_eq!(route_over(&UNREADABLE, &ABSENT), VaultUnlockRoute::Keychain);
        assert_eq!(route_over(&ABSENT, &UNREADABLE), VaultUnlockRoute::Keychain);
        assert_eq!(route_over(&UNREADABLE, &UNREADABLE), VaultUnlockRoute::Keychain);
    }

    /// What the window parses: snake_case strings, and no other fields.
    #[test]
    fn the_route_is_a_snake_case_string_on_the_wire() {
        assert_eq!(serde_json::to_value(VaultUnlockRoute::Keychain).unwrap(), serde_json::json!("keychain"));
        assert_eq!(
            serde_json::to_value(VaultUnlockRoute::RecoveryPhrase).unwrap(),
            serde_json::json!("recovery_phrase")
        );
    }

    /// Item 5: the command reaches the Keychain only through `keychain_vault_key_present`, which asks both layouts and never
    /// unlocks (`the_vault_key_probe_asks_both_layouts_and_never_unlocks` holds that helper); a worker that failed to answer is
    /// the Keychain unlock; and the command is registered on a Mac only.
    #[test]
    fn the_route_never_reads_the_key() {
        let production = super::finder_setup_command_tests::production_source();
        let body = |signature: &str| super::finder_setup_command_tests::body_between(&production, signature, "\n}\n").to_string();
        let route_for = body("fn vault_unlock_route_for(");
        assert!(route_for.contains("unlock_route(keychain_vault_key_present(account_id))"), "{route_for}");
        let command = body("async fn vault_unlock_route(");
        assert!(command.contains("vault_unlock_route_for(&account_id)"), "{command}");
        assert!(command.contains(".unwrap_or(VaultUnlockRoute::Keychain)"), "fails closed:\n{command}");
        for (name, text) in [("the command", &command), ("the account function", &route_for), ("the decision", &body("fn unlock_route("))] {
            for forbidden in ["load_", "AuthVault", "unlock_vault", "master_key", "unlock(", "session", "token", ".ok()", "unwrap_or(false)"] {
                assert!(!text.contains(forbidden), "{name} must not touch {forbidden}:\n{text}");
            }
        }
        let handler = super::finder_setup_command_tests::body_between(&production, "tauri::generate_handler![", "])");
        let at = handler.find("vault_unlock_route,").expect("registered");
        assert!(
            handler[..at].trim_end().ends_with("#[cfg(target_os = \"macos\")]"),
            "registered on a Mac only"
        );
    }

    /// Item 6 (plan D5): the route is its own command; the 1 Hz `sync_status` poll never reaches the Keychain for it.
    #[test]
    fn sync_status_does_not_read_the_keychain() {
        let production = super::finder_setup_command_tests::production_source();
        let body = super::finder_setup_command_tests::body_between(&production, "async fn sync_status(", "\n}\n")
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n")
            .to_lowercase();
        for forbidden in ["keychain", "vault_unlock_route", "unlock_route", "holds_vault_key", "vault_key_in_either_layout"] {
            assert!(!body.contains(forbidden), "sync_status must not reach {forbidden}");
        }
    }

    /// Review P3-1: `open_reauth_window` on a Mac asks the window for “Sign in again”, whatever the session says; the
    /// sign-in itself still lets the window decide.
    #[test]
    fn open_reauth_window_asks_the_beebeeb_window_for_sign_in_again() {
        assert_eq!(serde_json::to_value(MainNav::Reauth).unwrap(), serde_json::json!({ "to": "reauth" }));
        let production = super::finder_setup_command_tests::production_source();
        let window = super::finder_setup_command_tests::body_between(&production, "async fn open_reauth_window(", "\n}\n");
        let mac = super::finder_setup_command_tests::body_between(window, "#[cfg(target_os = \"macos\")]\n    {", "\n    }\n");
        assert!(mac.contains("show_reauth_in_beebeeb_window(&app)"), "{mac}");
        let ask = super::finder_setup_command_tests::body_between(&production, "fn show_reauth_in_beebeeb_window(", "\n}\n");
        assert!(ask.contains("show_main_window(app, MainNav::Reauth);"), "{ask}");
        // The plain sign-in is unchanged: Home, and the window decides.
        let plain = super::finder_setup_command_tests::body_between(&production, "fn show_sign_in_in_beebeeb_window(app", "\n}\n");
        assert!(plain.contains("show_main_window(app, MainNav::Home);"), "{plain}");
    }

    /// §6.4 (review I8): the switch's count logs the line `sign_out_count` logs, with counts only. A device step reads
    /// it to prove a change was waiting (D1).
    #[tokio::test]
    async fn the_switch_count_logs_counts_only() {
        in_own_dirs(async {
            let db = state_db_in_scratch();
            let staged = queue_write(&db, "w1", "f1", b"bytes");
            queue_rename(&db, "r1");
            let state = AppState::default();
            let mut counted = None;
            let logs = capture_logs(|| counted = Some(switch_snapshot(&state)));
            assert!(counted.expect("the count ran").unwrap().is_some(), "a snapshot");
            let line = logs
                .lines()
                .find(|line| line.contains("sign-out flow counted"))
                .unwrap_or_else(|| panic!("the switch's count is logged:\n{logs}"));
            let fields: Vec<&str> = line.split_whitespace().collect();
            for field in [
                "files=1",
                "copied_files=1",
                "others=1",
                "missing_files=0",
                "can_send=false",
                "earlier_folder=false",
            ] {
                assert!(fields.contains(&field), "{field}: {line}");
            }
            assert!(
                !logs.contains("f1.bin") && !logs.contains(&*staged.to_string_lossy()),
                "no name and no path:\n{logs}"
            );
        })
        .await;
    }

    /// §5.1, the entry points' side of the request. They need the app's windows, so their text is read: the request is
    /// stored before Settings opens and is told, so a window that has not loaded yet finds it when it mounts; the
    /// compact page's command opens the flow; Settings' command takes the one request; both are registered once.
    #[test]
    fn a_flow_request_is_stored_before_settings_opens_and_is_told() {
        let squeeze = crate::source_pin::squeeze;
        let production = super::finder_setup_command_tests::production_source();
        let body = |start: &str| {
            squeeze(super::finder_setup_command_tests::body_between(
                &production,
                start,
                "\n}\n",
            ))
        };
        let open = body("fn open_sign_out_flow(");
        let at = |step: &str| {
            open.find(&squeeze(step))
                .unwrap_or_else(|| panic!("open_sign_out_flow: {step}\n{open}"))
        };
        let stored = at("app.state::<AppState>().sign_out.flow_request.store(true, Ordering::SeqCst);");
        // Task 1683 slice 1 (two-surface §8.1, §8.2): the flow's owner is the Beebeeb window, on Settings › Account.
        let shown = at("show_main_window(app, MainNav::settings(Some(\"account\")));");
        let told = at("let _ = app.emit_to(\"main\", \"menu:sign-out-flow\", ());");
        assert!(stored < shown && shown < told, "store, open, then tell: {open}");
        assert!(
            body("fn request_sign_out_flow(").contains(&squeeze("open_sign_out_flow(&app);")),
            "the compact page's command opens the flow"
        );
        assert!(
            body("fn consume_sign_out_flow_request(").contains(&squeeze("take_sign_out_flow_request(&state.sign_out)")),
            "Settings takes the one request"
        );
        let handler = super::finder_setup_command_tests::body_between(&production, "tauri::generate_handler![", "])");
        for command in ["request_sign_out_flow,", "consume_sign_out_flow_request,"] {
            assert_eq!(
                handler.lines().filter(|line| line.trim() == command).count(),
                1,
                "{command} registered once"
            );
        }
    }

    /// U20 (W12): spec A's R10 reset keeps every waiting write before its purge, asking nobody: the other changes are
    /// dropped with their count in its log line, and the purge runs with the guard. A failed keep fails the reset and
    /// deletes nothing. A reset whose purge fails after a completed keep still hands its folder on to be surfaced, as
    /// the recorded folder is on disk either way.
    #[tokio::test]
    async fn the_r10_reset_keeps_before_it_purges() {
        in_own_dirs(async {
            let state_dir = state_paths::beebeeb_state_dir().unwrap();
            let sync_root = tempfile::tempdir().unwrap();
            let db = state_db_in_scratch();
            queue_write(&db, "w1", "f1", b"another account's save");
            queue_rename(&db, "r1");
            let data =
                StateDbLocalData::for_engine_start(&LocalDataPaths::for_test(&state_dir, sync_root.path())).unwrap();
            let logs = capture_logs_async(async {
                account_binding::LocalData::reset(&data).unwrap();
            })
            .await;
            let kept = data.kept.lock().unwrap().clone().expect("the reset kept the write");
            let folder = kept.folder.clone().unwrap();
            assert_eq!(std::fs::read(folder.join("f1.bin")).unwrap(), b"another account's save");
            assert_eq!(
                DesktopConfig::load().unwrap().kept_unsent_folder,
                Some(folder.to_string_lossy().into_owned()),
                "recorded"
            );
            assert!(kept.others_acknowledged.contains("r1"));
            assert!(
                logs.contains("others_dropped=1"),
                "the dropped rename is counted in the reset's line: {logs}"
            );
            assert!(
                db.waiting_rows().unwrap().is_empty(),
                "then the purge ran, with the guard"
            );
            assert!(
                DesktopConfig::load().unwrap().kept_unsent_unfinished.is_empty(),
                "a completed reset clears the unfinished list, as every completed purge does (§7.3)"
            );
            let production = super::finder_setup_command_tests::production_source();
            let reset = super::finder_setup_command_tests::body_between(
                &production,
                "    fn reset(&self) -> Result<(), String> {",
                "\n    }\n",
            );
            let squeezed = crate::source_pin::squeeze(reset);
            let call = crate::source_pin::squeeze(
                "purge_local_state_files_in(&self.db, self.sync_root.as_deref(), &self.staging_dirs, kept.as_ref())",
            );
            assert!(squeezed.contains(&call), "the guard is in the reset's path: {reset}");
            // W13 and W10: the reset clears the unfinished list with the sign-out purge's own statement, so a clear
            // that fails is logged by the step that failed, never by an error's text.
            let marker_clear = crate::source_pin::squeeze(concat!(
                "#[cfg(target_os = \"macos\")] if let Err(step) = DesktopConfig::path()",
                ".map_err(|_| \"path\".to_string())",
                ".and_then(|path| sign_out::kept_record::clear_unfinished_at(&path))",
                " { tracing::warn!(step = %step, \"could not clear the unfinished sign-out marker\"); }",
            ));
            let sign_out_purge = crate::source_pin::squeeze(super::finder_setup_command_tests::body_between(
                &production,
                "fn purge_local_data_for_sign_out(",
                "\n}\n",
            ));
            assert!(
                sign_out_purge.contains(&marker_clear) && squeezed.contains(&marker_clear),
                "the reset clears the marker as the sign-out does, its warning naming the step: {reset}"
            );
            // A failing copy: the reset fails, nothing is deleted, no record.
            use std::os::unix::fs::PermissionsExt;
            clean_scratch(&db);
            let staged = queue_write(&db, "w2", "f2", b"bytes");
            std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o000)).unwrap();
            let data =
                StateDbLocalData::for_engine_start(&LocalDataPaths::for_test(&state_dir, sync_root.path())).unwrap();
            let mut failed = None;
            let logs = capture_logs_async(async {
                failed = Some(account_binding::LocalData::reset(&data));
            })
            .await;
            // A copy the reset deleted cannot have its permissions put back: the next line names that failure.
            let _ = std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o600));
            assert!(staged.exists(), "nothing deleted: the staged copy is still there");
            assert!(failed.unwrap().unwrap_err().starts_with("unsent_copy_failed: "));
            let stopped = "R10: local data reset stopped: the waiting changes could not be kept; nothing was purged";
            assert!(
                logs.contains(stopped),
                "the reset's failed keep is logged as the reset's: {logs}"
            );
            assert!(!logs.contains("sign-out stopped"), "a reset is not a sign-out: {logs}");
            assert!(
                !db.finder_accepts_closed().unwrap(),
                "a reset that stops reopens Finder accepts (spec §15 question 3)"
            );
            assert_eq!(db.waiting_rows().unwrap().len(), 1, "nothing deleted");
            assert_eq!(DesktopConfig::load().unwrap().kept_unsent_folder, None, "no record");
            // W12: a reset whose purge fails after a completed keep still hands its folder on to be surfaced (the alert
            // and the row), as the recorded folder is on disk either way.
            clean_scratch(&db);
            let blocker = std::env::temp_dir().join(format!("bb-1887-{}-blocker", std::process::id()));
            std::fs::create_dir_all(&blocker).unwrap();
            db.track_staged_payload(blocker.to_str().unwrap(), None, true).unwrap(); // a "file" no removal can take
            queue_write(&db, "w3", "f3", b"kept, then the purge failed");
            let data =
                StateDbLocalData::for_engine_start(&LocalDataPaths::for_test(&state_dir, sync_root.path())).unwrap();
            let failed = account_binding::LocalData::reset(&data);
            std::fs::remove_dir(&blocker).unwrap();
            assert!(failed.is_err(), "the purge failed on the folder at a journal path");
            assert!(
                !db.finder_accepts_closed().unwrap(),
                "a reset whose purge failed reopens Finder accepts (spec §15 question 3)"
            );
            let surfaced = AppState::default();
            hand_on_reset_kept(&surfaced, &data);
            let (folder, files) = surfaced
                .sign_out
                .reset_kept
                .lock()
                .unwrap()
                .clone()
                .expect("the recorded folder is handed on");
            assert_eq!(files, 1);
            assert_eq!(
                std::fs::read(Path::new(&folder).join("f3.bin")).unwrap(),
                b"kept, then the purge failed"
            );
            assert_eq!(
                DesktopConfig::load().unwrap().kept_unsent_unfinished,
                [folder.as_str()],
                "a reset that stops after its keep leaves its folder named (W13)"
            );
            let bind =
                super::finder_setup_command_tests::body_between(&production, "fn bind_data_to_session(", "\n}\n");
            let ran = bind.find("bind_before_engine_start(").expect("the binding runs");
            let handed = bind
                .find("hand_on_reset_kept(state, data);")
                .expect("the binding hands the folder on");
            assert!(
                ran < handed && handed < bind.find("match bound?").expect("before it looks at the result"),
                "the folder is handed on after the binding ran, before its answer is looked at: {bind}"
            );
        })
        .await;
    }

    /// W12 through the binding (U20): an R10 reset that completes, for another account's data and its waiting save,
    /// hands its kept folder to the state, where the engine start surfaces it.
    #[tokio::test]
    async fn a_completed_r10_reset_hands_its_folder_on_through_the_binding() {
        in_own_dirs(async {
            let state_dir = state_paths::beebeeb_state_dir().unwrap();
            let sync_root = tempfile::tempdir().unwrap();
            let db = state_db_in_scratch();
            let alice = account_binding::Identity::new(Some("u-alice"), Some("alice@beebeeb.io"));
            let bob = account_binding::Identity::new(Some("u-bob"), Some("bob@beebeeb.io"));
            db.set_owner(&alice).unwrap();
            queue_write(&db, "w1", "f1", b"alice's save");
            let data =
                StateDbLocalData::for_engine_start(&LocalDataPaths::for_test(&state_dir, sync_root.path())).unwrap();
            let state = AppState::default();
            assert_eq!(bind_data_to_session(&state, &data, &bob), Ok(()), "the reset completed");
            assert_eq!(db.owner().unwrap(), Some(bob), "the data is the new account's");
            let (folder, files) = state
                .sign_out
                .reset_kept
                .lock()
                .unwrap()
                .clone()
                .expect("a completed reset hands its folder on through the binding");
            assert_eq!(files, 1);
            assert_eq!(
                std::fs::read(Path::new(&folder).join("f1.bin")).unwrap(),
                b"alice's save"
            );
        })
        .await;
    }

    /// W12 through the binding (U20): an R10 reset whose purge fails after a completed keep answers an error, so
    /// nothing starts, and the binding still hands the kept folder to the state: it is recorded and on disk either way.
    #[tokio::test]
    async fn an_r10_reset_whose_purge_fails_hands_its_folder_on_through_the_binding() {
        in_own_dirs(async {
            let state_dir = state_paths::beebeeb_state_dir().unwrap();
            let sync_root = tempfile::tempdir().unwrap();
            let db = state_db_in_scratch();
            let alice = account_binding::Identity::new(Some("u-alice"), Some("alice@beebeeb.io"));
            let bob = account_binding::Identity::new(Some("u-bob"), Some("bob@beebeeb.io"));
            db.set_owner(&alice).unwrap();
            // A journal path that holds a directory: a removal the purge cannot make. In the test's own state dir.
            let blocker = state_dir.join("bb-1887-blocker");
            std::fs::create_dir_all(&blocker).unwrap();
            db.track_staged_payload(blocker.to_str().unwrap(), None, true).unwrap();
            queue_write(&db, "w1", "f1", b"kept, then the purge failed");
            let data =
                StateDbLocalData::for_engine_start(&LocalDataPaths::for_test(&state_dir, sync_root.path())).unwrap();
            let state = AppState::default();
            let bound = bind_data_to_session(&state, &data, &bob);
            assert!(
                bound
                    .as_ref()
                    .is_err_and(|error| error.contains("could not be removed")),
                "the purge failed, so nothing starts: {bound:?}"
            );
            assert_eq!(db.owner().unwrap(), Some(alice), "no new owner over data still there");
            let (folder, files) = state
                .sign_out
                .reset_kept
                .lock()
                .unwrap()
                .clone()
                .expect("a failed purge's folder is handed on through the binding");
            assert_eq!(files, 1);
            assert_eq!(
                std::fs::read(Path::new(&folder).join("f1.bin")).unwrap(),
                b"kept, then the purge failed"
            );
        })
        .await;
    }

    /// Spec §15 question 3 for the R10 reset's last step: a reset whose row clear fails, after its keep and its purge,
    /// answers that error and reopens Finder accepts at once, as a reset whose purge fails does.
    #[tokio::test]
    async fn an_r10_reset_whose_row_clear_fails_reopens_finder_accepts() {
        in_own_dirs(async {
            let state_dir = state_paths::beebeeb_state_dir().unwrap();
            let sync_root = tempfile::tempdir().unwrap();
            let db = state_db_in_scratch();
            queue_write(&db, "w1", "f1", b"a save");
            let data =
                StateDbLocalData::for_engine_start(&LocalDataPaths::for_test(&state_dir, sync_root.path())).unwrap();
            // A row only the row clear deletes, and a trigger on the reset's own connection that refuses that delete.
            data.db
                .hold_lock_for_test()
                .execute_batch(
                    "INSERT INTO sync_state (key, value) VALUES ('bb-1887-clear-blocker', '1');
                     CREATE TEMP TRIGGER bb_1887_clear_fails BEFORE DELETE ON sync_state
                     WHEN OLD.key = 'bb-1887-clear-blocker'
                     BEGIN SELECT RAISE(ABORT, 'the row clear fails'); END;",
                )
                .unwrap();
            let error = account_binding::LocalData::reset(&data).expect_err("the row clear failed");
            assert!(error.starts_with("clear the local data: "), "{error}");
            assert!(
                db.waiting_rows().unwrap().is_empty(),
                "the keep and the purge ran first"
            );
            assert!(
                !db.finder_accepts_closed().unwrap(),
                "a reset whose row clear failed reopens Finder accepts (spec §15 question 3)"
            );
        })
        .await;
    }

    /// U20, the surfacing (W12, §9): an R10 reset's kept folder reaches the person the way the app-start sweep's does.
    /// The engine start that ran the reset needs the app, so its text is read: the start takes the handed-on folder
    /// after the bound start returned, whatever it returned, and surfaces it once; the surface tells the windows (the
    /// row), then shows one alert that names our folder alone.
    #[test]
    fn the_r10_reset_folder_is_surfaced_after_the_start_returns() {
        let squeeze = crate::source_pin::squeeze;
        let production = super::finder_setup_command_tests::production_source();
        // A function's code without its comment lines, squeezed: a comment can neither meet nor break a step.
        let body = |start: &str| {
            let code: Vec<&str> = super::finder_setup_command_tests::body_between(&production, start, "\n}\n")
                .lines()
                .filter(|line| !line.trim_start().starts_with("//"))
                .collect();
            squeeze(&code.join("\n"))
        };
        let gate = body("fn spawn_bound_engine(");
        let at = |step: &str| {
            gate.find(&squeeze(step))
                .unwrap_or_else(|| panic!("spawn_bound_engine: {step}\n{gate}"))
        };
        let cloned = at("let surface_app = app.clone();");
        let started = at("let started = start_engine_bound(");
        let taken = at("let reset_kept = state.sign_out.reset_kept.lock().unwrap_or_else(|p| p.into_inner()).take();");
        let surfaced =
            at("if let Some((folder, files)) = reset_kept { surface_reset_folder(&surface_app, &folder, files); }");
        assert!(
            cloned < started && started < taken && taken < surfaced,
            "clone the app, start, then take and surface: {gate}"
        );
        let between = &gate[started..surfaced];
        assert!(
            !between.contains('?') && !between.contains("return") && gate.ends_with("started"),
            "whatever the start returned, nothing returns before the surfacing; the start's result comes last: {gate}"
        );
        let surface = body("fn surface_reset_folder(");
        let told = surface
            .find(&squeeze("emit_kept_folder_changed(app);"))
            .unwrap_or_else(|| panic!("the windows are told: {surface}"));
        let alert = surface
            .find(&squeeze("sign_out::copy::sign_out_alert(Some((folder, files)), None)"))
            .unwrap_or_else(|| panic!("one alert, our folder alone: {surface}"));
        let shown = surface
            .find(".dialog()")
            .unwrap_or_else(|| panic!("the alert is shown: {surface}"));
        assert!(told < alert && alert < shown, "told, then the alert: {surface}");
    }
