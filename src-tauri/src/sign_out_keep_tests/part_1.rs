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

    // The wrapper moved to the engine crate with the purge's deletion gate (`local_state.rs`).
    let engine_local_state =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../crates/beebeeb-engine/src/local_state.rs");
    let production = crate::source_pin::without_items(
        &std::fs::read_to_string(&engine_local_state)
            .expect("read the engine's local_state.rs")
            .replace("\r\n", "\n"),
        &crate::source_census::TEST_ITEM_GATES,
    );
    let wrapper = super::finder_setup_command_tests::body_between(
        &production,
        "pub fn disposable_cache_roots() -> Vec<PathBuf> {",
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
             &crate::engine_bridge::FinderStagingBases::current())",
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

async fn sign_out_with(state: &AppState, keep: Option<&str>, seam: SignOutSeam) -> Result<SignedOut, SignOutFailure> {
    SIGN_OUT_SEAM
        .scope(
            seam,
            clear_session_impl(state, false, keep, crate::UnknownFilesArg::Keep, None),
        )
        .await
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
            .scope(
                no_seam(),
                clear_session_impl(&state, true, None, crate::UnknownFilesArg::Keep, None),
            )
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
