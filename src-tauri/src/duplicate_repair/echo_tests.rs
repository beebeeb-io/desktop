//! Task 2274, plan Task 6: `check-echo` (spec phase 4, I4, Review Focus 3) on a real state DB that stays open, as the
//! running app holds it. The platform is injected, so these run on every OS.
use super::echo::{EchoResult, check_echo};
use super::list::{Observed, list_pairs};
use super::model::{Applied, Outcome, Reason, RunFile};
use crate::state_db::{FileEntry, FileStatus, ItemKind, StateDb};
use crate::unknown_check::{OnDisk, PlatformView};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const A: &str = "a1";
const B: &str = "b1";
const PATH: &str = "dup.txt";
const ROOT: &str = "C:/root";
/// The app's cursor when `apply-one` ran, and the seq of B's `file_trash` op it found.
const CURSOR: i64 = 7;
const SEQ: i64 = 12;
const POLL: Duration = Duration::from_millis(5);

/// This PC right after `apply-one 1`: B's row forgotten, B trashed at `SEQ`, the app (this `StateDb`) still running.
struct Pc {
    dir: tempfile::TempDir,
    db: StateDb,
    run: RunFile,
}

impl Pc {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        row(&db, A, Some("aa"));
        row(&db, B, None);
        db.set_sync_cursor(CURSOR).unwrap();
        let listing = list_pairs(&db, Path::new(ROOT), &|_: &Path| at_a()).unwrap();
        let mut run = RunFile { pairs: listing.pairs, more_than_two: listing.more_than_two };
        run.pairs[0].applied = Some(Applied {
            outcome: Outcome::Trashed { seq: SEQ, b_trashed_at: None },
            cursor: CURSOR,
            backup: "state-1-0.db".into(),
            at: 0,
        });
        db.delete_file(B).unwrap();
        Self { dir, db, run }
    }

    fn db_path(&self) -> PathBuf {
        self.dir.path().join("state.db")
    }

    fn check(&self, observed: Observed, wait: Duration) -> anyhow::Result<EchoResult> {
        check_echo(&self.db_path(), Path::new(ROOT), &self.run, 1, wait, POLL, &move |_: &Path| observed.clone())
    }

    fn outcome(&mut self, outcome: Option<Outcome>) {
        self.run.pairs[0].applied = outcome.map(|outcome| Applied { outcome, cursor: CURSOR, backup: "b".into(), at: 0 });
    }

    /// The state DB's files, name and bytes; the `-shm` index only by name (every WAL reader, a read-only one too,
    /// writes its read mark there).
    fn files(&self) -> Vec<(String, Vec<u8>)> {
        let mut files: Vec<_> = std::fs::read_dir(self.dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.is_file())
            .map(|path| {
                let name = path.file_name().unwrap().to_string_lossy().into_owned();
                let bytes = if name.ends_with("-shm") { Vec::new() } else { std::fs::read(&path).unwrap() };
                (name, bytes)
            })
            .collect();
        files.sort();
        files
    }
}

fn row(db: &StateDb, id: &str, local_hash: Option<&str>) {
    db.upsert_file(&FileEntry {
        file_id: id.into(),
        path: PATH.into(),
        status: if local_hash.is_some() { FileStatus::Local } else { FileStatus::CloudOnly },
        size_bytes: 10,
        modified_at: 1,
        content_hash: None,
        remote_updated_at: 1,
        parent_id: None,
        item_kind: ItemKind::File,
    })
    .unwrap();
    if let Some(hash) = local_hash {
        db.record_local_baseline(id, 1, hash).unwrap();
    }
}

/// A's placeholder, in sync and wholly on disk, as the list saw it.
fn at_a() -> Observed {
    Observed {
        view: PlatformView::Placeholder { on_disk: OnDisk::Wholly, in_sync: true, identity: Some(A.into()) },
        size: Some(10),
        modified: Some(7),
        read_only: Some(true),
    }
}

/// The echo applied (the app's cursor at B's op) and the path as it was: unchanged.
#[test]
fn the_cursor_at_the_seq_and_the_same_view_is_unchanged() {
    let pc = Pc::new();
    pc.db.set_sync_cursor(SEQ).unwrap();
    assert_eq!(pc.check(at_a(), Duration::ZERO).unwrap(), EchoResult::Unchanged);
}

/// Review Focus 3: the cursor already past B's op (another device moved the log on) counts as the echo applied, and
/// the path is still checked.
#[test]
fn a_cursor_already_past_the_seq_still_checks_the_path() {
    let pc = Pc::new();
    pc.db.set_sync_cursor(SEQ + 30).unwrap();
    assert_eq!(pc.check(at_a(), Duration::ZERO).unwrap(), EchoResult::Unchanged);
    let grown = Observed { size: Some(11), ..at_a() };
    assert_eq!(pc.check(grown, Duration::ZERO).unwrap(), EchoResult::Changed(vec!["size"]));
}

/// The app applies the echo while the tool waits: the tool polls the cursor until it gets there.
#[test]
fn an_echo_applied_during_the_wait_is_waited_for() {
    let pc = Pc::new();
    let db_path = pc.db_path();
    let app = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(80));
        StateDb::open(&db_path).unwrap().set_sync_cursor(SEQ).unwrap();
    });
    let started = Instant::now();
    assert_eq!(pc.check(at_a(), Duration::from_secs(10)).unwrap(), EchoResult::Unchanged);
    assert!(started.elapsed() >= Duration::from_millis(80), "it waited for the echo: {:?}", started.elapsed());
    app.join().unwrap();
}

/// A cursor that does not reach B's op within the wait stops the run; the path is not judged.
#[test]
fn a_cursor_that_never_reaches_the_seq_stops_the_run() {
    let pc = Pc::new();
    pc.db.set_sync_cursor(SEQ - 1).unwrap();
    let started = Instant::now();
    assert_eq!(pc.check(at_a(), Duration::from_millis(60)).unwrap(), EchoResult::CursorNeverReached);
    assert!(started.elapsed() >= Duration::from_millis(60), "it waited the whole wait: {:?}", started.elapsed());
}

/// I4: every fact of the file at the path that differs from the list is named, and only those.
#[test]
fn every_fact_that_differs_is_named() {
    let pc = Pc::new();
    pc.db.set_sync_cursor(SEQ).unwrap();
    let placeholder = |on_disk, in_sync, identity: &str| Observed {
        view: PlatformView::Placeholder { on_disk, in_sync, identity: Some(identity.into()) },
        ..at_a()
    };
    let cases: Vec<(Observed, Vec<&str>)> = vec![
        (placeholder(OnDisk::Wholly, true, B), vec!["identity"]),
        (placeholder(OnDisk::Partly, true, A), vec!["on-disk"]),
        (placeholder(OnDisk::Wholly, false, A), vec!["in-sync"]),
        (Observed { size: Some(9), ..at_a() }, vec!["size"]),
        (Observed { modified: Some(8), ..at_a() }, vec!["modified"]),
        (Observed { read_only: Some(false), ..at_a() }, vec!["read-only"]),
        (Observed { view: PlatformView::PlainFile, ..at_a() }, vec!["kind", "on-disk", "in-sync", "identity"]),
        (
            Observed { view: PlatformView::Missing, size: None, modified: None, read_only: None },
            vec!["kind", "on-disk", "in-sync", "identity", "size", "modified", "read-only"],
        ),
    ];
    for (observed, names) in cases {
        let label = format!("{observed:?}");
        assert_eq!(pc.check(observed, Duration::ZERO).unwrap(), EchoResult::Changed(names), "{label}");
    }
}

/// B's row back at the path (a snapshot that still lists B) is a change too: the path must hold A's row alone.
#[test]
fn b_row_back_at_the_path_is_a_change() {
    let pc = Pc::new();
    pc.db.set_sync_cursor(SEQ).unwrap();
    row(&pc.db, B, None);
    assert_eq!(pc.check(at_a(), Duration::ZERO).unwrap(), EchoResult::Changed(vec!["rows"]));
}

/// A forgotten pair (B already in the trash or gone) has no echo of the tool's to wait for: the path is checked at once.
#[test]
fn a_forgotten_pair_is_checked_without_waiting() {
    let mut pc = Pc::new();
    pc.outcome(Some(Outcome::Forgotten));
    let started = Instant::now();
    assert_eq!(pc.check(at_a(), Duration::from_secs(10)).unwrap(), EchoResult::Unchanged);
    assert!(started.elapsed() < Duration::from_secs(5), "no wait: {:?}", started.elapsed());
}

/// A pair with no confirmed trash or forget, or a pair number not in the run file, has nothing to check.
#[test]
fn a_pair_with_nothing_applied_is_refused() {
    let mut pc = Pc::new();
    pc.db.set_sync_cursor(SEQ).unwrap();
    for outcome in [
        None,
        Some(Outcome::TrashFailed { status: Some(500) }),
        Some(Outcome::Unexpected { a_live: true, b_trashed: false, seq: Some(SEQ) }),
        Some(Outcome::Skipped { reason: Reason::ChangedOnServer }),
    ] {
        pc.outcome(outcome.clone());
        let error = pc.check(at_a(), Duration::ZERO).unwrap_err().to_string();
        assert!(error.contains("pair 1"), "{outcome:?}: {error}");
    }
    pc.outcome(Some(Outcome::Forgotten));
    for n in [0, 2] {
        let error = check_echo(&pc.db_path(), Path::new(ROOT), &pc.run, n, Duration::ZERO, POLL, &|_: &Path| at_a())
            .unwrap_err()
            .to_string();
        assert!(error.contains(&format!("pair {n}")), "{error}");
    }
}

/// The app keeps running while the tool reads: the tool opens the state DB read-only and leaves its files as they
/// were. The open flags are pinned in the source, since SQLite opens a write-protected file read-only by itself.
#[test]
fn the_state_db_is_only_read() {
    let pc = Pc::new();
    pc.db.set_sync_cursor(SEQ).unwrap();
    let before = pc.files();
    assert_eq!(pc.check(at_a(), Duration::ZERO).unwrap(), EchoResult::Unchanged);
    assert_eq!(pc.files(), before, "the state DB's files are byte for byte as they were");
    let source = include_str!("echo.rs");
    assert_eq!(source.matches("Connection::open_with_flags(").count(), 1, "one open");
    assert_eq!(source.matches("Connection::open(").count(), 0, "never a read-write open");
    assert!(source.contains("OpenFlags::SQLITE_OPEN_READ_ONLY"), "opened read-only");
    assert!(!source.contains("SQLITE_OPEN_READ_WRITE"), "never read-write");
}
