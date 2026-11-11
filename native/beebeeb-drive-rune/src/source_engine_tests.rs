//! `EngineSource` on a real state.db, built here with the engine's own writers. Every name is invented.

use std::path::Path;
use std::time::Instant;

use beebeeb_engine::state_db::{FileEntry, FileStatus, ItemKind, StateDb};

use super::*;

fn row(id: &str, path: &str, parent: Option<&str>, folder: bool, n: i64) -> FileEntry {
    FileEntry {
        file_id: id.to_string(),
        path: path.to_string(),
        status: FileStatus::CloudOnly,
        size_bytes: if folder { 0 } else { 1_000 + n },
        modified_at: 1_794_000_000 + n,
        content_hash: None,
        remote_updated_at: 1_794_000_000 + n,
        parent_id: parent.map(str::to_string),
        item_kind: if folder { ItemKind::Folder } else { ItemKind::File },
    }
}

/// 3 nested folders (Projects / Alpha / Drafts), 200 files (120 at the root, 50 in Projects, 20 in Alpha,
/// 10 in Drafts) and one folder, Archive, with 5,000 files. 5,204 rows.
fn build(path: &Path) {
    let db = StateDb::open(path).expect("create the state db");
    put(&db, row("d-projects", "/Projects", None, true, 1));
    put(&db, row("d-alpha", "/Projects/Alpha", Some("d-projects"), true, 2));
    put(&db, row("d-drafts", "/Projects/Alpha/Drafts", Some("d-alpha"), true, 3));
    put(&db, row("d-archive", "/Archive", None, true, 4));
    let spread: [(&str, Option<&str>, usize); 4] = [
        ("", None, 120),
        ("/Projects", Some("d-projects"), 50),
        ("/Projects/Alpha", Some("d-alpha"), 20),
        ("/Projects/Alpha/Drafts", Some("d-drafts"), 10),
    ];
    let mut n = 0;
    for (dir, parent, count) in spread {
        for i in 0..count {
            n += 1;
            let id = format!("f-{n:04}");
            put(
                &db,
                row(&id, &format!("{dir}/note-{i:03}.txt"), parent, false, n as i64),
            );
        }
    }
    assert_eq!(n, 200);
    for i in 0..5_000 {
        let id = format!("a-{i:04}");
        put(
            &db,
            row(&id, &format!("/Archive/scan-{i:04}.pdf"), Some("d-archive"), false, i),
        );
    }
}

/// A row and its contract (parent, kind) in one transaction: the writer the engine's pull uses.
fn put(db: &StateDb, entry: FileEntry) {
    db.upsert_file_with_contract(&entry, |contract| {
        contract.parent_id = entry.parent_id.clone();
        contract.item_kind = entry.item_kind.clone();
    })
    .unwrap();
}

fn names(items: &[DriveItem]) -> Vec<&str> {
    items.iter().map(|i| i.name.as_str()).collect()
}

fn crumb_names(path: &[Crumb]) -> Vec<&str> {
    path.iter().map(|c| c.name.as_str()).collect()
}

#[test]
fn lists_the_root_opens_folders_and_pages_through_5000_children() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    build(&path);
    let source = EngineSource::open(&path).expect("open read-only");

    // The root: 2 folders first (name order), then the 120 files in name order.
    let root = source.list(None).unwrap();
    assert_eq!(root.items.len(), 122);
    assert_eq!(names(&root.items[..3]), ["Archive", "Projects", "note-000.txt"]);
    assert_eq!(root.items[121].name, "note-119.txt");
    assert_eq!(crumb_names(&root.path), ["Files"]);
    assert_eq!(root.items[0].child_count, Some(5_000));
    assert_eq!(root.items[1].child_count, Some(51), "Projects: Alpha and 50 files");
    assert_eq!(root.items[2].size, Some(1_001));
    assert_eq!(root.items[2].mime.as_deref(), Some("text/plain"));
    assert!(root.items[0].is_folder && root.items[0].size.is_none());

    // Open a folder, then the one inside it, then the one inside that: counts and crumbs.
    let projects = source.list(Some("d-projects")).unwrap();
    assert_eq!(projects.items.len(), 51);
    assert_eq!(projects.items[0].name, "Alpha");
    assert_eq!(crumb_names(&projects.path), ["Files", "Projects"]);
    let drafts = source.list(Some("d-drafts")).unwrap();
    assert_eq!(drafts.items.len(), 10);
    assert_eq!(crumb_names(&drafts.path), ["Files", "Projects", "Alpha", "Drafts"]);
    assert_eq!(drafts.folder.id.as_deref(), Some("d-drafts"));
    assert_eq!(drafts.path[1].id.as_deref(), Some("d-projects"));

    // Page through the 5,000-child folder, 500 at a time: 10 full pages, in name order, no row twice.
    let mut seen = Vec::new();
    for page_no in 0..10 {
        let page = source
            .page(Some("d-archive"), Sort::default(), page_no * 500, 500)
            .unwrap();
        assert_eq!(page.total, 5_000);
        assert_eq!(page.items.len(), 500, "page {page_no}");
        assert_eq!(crumb_names(&page.path), ["Files", "Archive"]);
        seen.extend(page.items.into_iter().map(|i| i.name));
    }
    assert_eq!(seen.len(), 5_000);
    assert_eq!(seen[0], "scan-0000.pdf");
    assert_eq!(seen[4_999], "scan-4999.pdf");
    assert!(
        seen.windows(2).all(|w| w[0] < w[1]),
        "ascending, so no row is repeated or skipped"
    );
    let past_the_end = source.page(Some("d-archive"), Sort::default(), 5_000, 500).unwrap();
    assert_eq!((past_the_end.items.len(), past_the_end.total), (0, 5_000));

    // Sort: largest first (sizes rise with the scan number).
    let by_size = Sort {
        key: SortKey::Size,
        dir: SortDir::Desc,
    };
    let largest = source.page(Some("d-archive"), by_size, 0, 2).unwrap();
    assert_eq!(names(&largest.items), ["scan-4999.pdf", "scan-4998.pdf"]);

    // Details of one file: its row, the crumbs to its folder, one version.
    let details = source.details("f-0200").unwrap();
    assert_eq!(details.item.name, "note-009.txt");
    assert_eq!(crumb_names(&details.path), ["Files", "Projects", "Alpha", "Drafts"]);
    assert_eq!(details.version_count, 1);

    // An id that is not there, and a file opened as a folder: an error from both calls, never an empty folder.
    assert_eq!(
        source.page(Some("nope"), Sort::default(), 0, 10).unwrap_err(),
        DriveError::NotFound
    );
    assert_eq!(
        source.page(Some("f-0001"), Sort::default(), 0, 10).unwrap_err(),
        DriveError::NotFound
    );
    assert_eq!(source.list(Some("nope")).err(), Some(SourceError::NotFound));
    assert_eq!(source.list(Some("f-0001")).err(), Some(SourceError::NotFound));

    // No figure without the server.
    assert!(source.storage().is_none());
}

#[test]
fn the_source_never_writes_and_refuses_what_is_not_a_state_db() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    build(&path);
    let before = std::fs::read(&path).unwrap();
    let source = EngineSource::open(&path).unwrap();
    source.list(None).unwrap();
    source.page(Some("d-archive"), Sort::default(), 0, 100).unwrap();
    drop(source);
    assert_eq!(
        std::fs::read(&path).unwrap(),
        before,
        "the database file is byte-identical after reading"
    );

    // Each refusal keeps its own reason (the engine's tests cover the older-schema and locked cases).
    assert_eq!(
        EngineSource::open(&dir.path().join("missing.db")).err(),
        Some(ReadOnlyOpenError::NotFound)
    );
    let text = dir.path().join("notes.txt");
    std::fs::write(
        &text,
        b"this is a text file, long enough to be read as a database header",
    )
    .unwrap();
    assert_eq!(EngineSource::open(&text).err(), Some(ReadOnlyOpenError::NotADatabase));
    let other = dir.path().join("other.db");
    rusqlite::Connection::open(&other)
        .unwrap()
        .execute_batch("CREATE TABLE t(x);")
        .unwrap();
    assert_eq!(
        EngineSource::open(&other).err(),
        Some(ReadOnlyOpenError::NotAStateDatabase)
    );
}

/// A database that stops being readable after the window opened on it shows as an error, never as an empty folder.
#[test]
fn a_read_error_after_opening_is_an_error_state_not_an_empty_root() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    build(&path);
    let source = EngineSource::open(&path).unwrap();
    assert_eq!(source.list(None).unwrap().items.len(), 122, "control: readable first");
    // Another connection takes the table away under the open source.
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute_batch("ALTER TABLE files RENAME TO files_gone;")
        .unwrap();
    assert_eq!(source.list(None).err(), Some(SourceError::Unreadable));
    assert_eq!(source.list(Some("d-archive")).err(), Some(SourceError::Unreadable));
    assert!(source.page(None, Sort::default(), 0, 10).is_err());
}

/// The measurement the PR quotes (`cargo test -- --nocapture timings`): a 500-row page and the whole
/// 5,000-child folder, on the 5,204-row database and on one with 70,000 rows; and SQLite's plan for the
/// listing query. Nothing is asserted about the time (a loaded machine must not fail this); the printed numbers are
/// the record, and the plan is the assertion.
#[test]
fn timings_and_query_plan_for_5000_children_in_a_70000_row_db() {
    let dir = tempfile::tempdir().unwrap();
    let small = dir.path().join("small.db");
    build(&small);
    let big = dir.path().join("big.db");
    build(&big);
    {
        // 64,796 more rows in 13 other folders: 70,000 in all.
        let db = StateDb::open(&big).unwrap();
        let mut n = 0;
        'fill: for folder in 0..13 {
            let id = format!("d-bulk-{folder:02}");
            put(&db, row(&id, &format!("/Bulk {folder:02}"), None, true, folder));
            n += 1;
            for i in 0..5_000 {
                if n == 64_796 {
                    break 'fill;
                }
                let file = format!("b-{folder:02}-{i:04}");
                put(
                    &db,
                    row(
                        &file,
                        &format!("/Bulk {folder:02}/item-{i:04}.bin"),
                        Some(&id),
                        false,
                        i,
                    ),
                );
                n += 1;
            }
        }
    }
    for (label, path, rows) in [("5,204-row db", &small, 5_204i64), ("70,000-row db", &big, 70_000)] {
        let conn = rusqlite::Connection::open(path).unwrap();
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0)).unwrap();
        assert_eq!(count, rows);
        let source = EngineSource::open(path).unwrap();
        let best = |f: &dyn Fn() -> usize| {
            (0..5)
                .map(|_| {
                    let start = Instant::now();
                    assert_eq!(f(), 5_000);
                    start.elapsed()
                })
                .min()
                .unwrap()
        };
        let page = best(&|| {
            source
                .page(Some("d-archive"), Sort::default(), 2_500, 500)
                .unwrap()
                .total
        });
        let whole = best(&|| source.list(Some("d-archive")).unwrap().items.len());
        println!(
            "TIMING {label}: 500-row page of the 5,000-child folder {:.1} ms; whole folder {:.1} ms (best of 5)",
            page.as_secs_f64() * 1e3,
            whole.as_secs_f64() * 1e3
        );

        // The statement `StateDb::children_paged` runs (its WHERE, ORDER BY and LIMIT; `state_db/files.rs`).
        let plan: Vec<String> = conn
            .prepare(
                "EXPLAIN QUERY PLAN SELECT file_id, path FROM files
                 WHERE parent_id IS ?1 AND status != 'trashing'
                 ORDER BY item_kind ASC, path ASC LIMIT ?2 OFFSET ?3",
            )
            .unwrap()
            .query_map(rusqlite::params!["d-archive", 500, 0], |r| r.get::<_, String>(3))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        println!("PLAN {label}: {plan:?}");
        assert!(
            plan.iter().any(|step| step.contains("idx_files_parent_id")),
            "the listing reads through the parent index: {plan:?}"
        );
    }
}

/// Writes the same invented database to the NEW file `BB_NATIVE_FIXTURE_DB` names, for a headless render of the
/// window (`beebeeb-drive --db <path> --snap <png>`). Does nothing when the variable is not set, and refuses a
/// path where something already is: this never deletes or overwrites a file.
#[test]
fn writes_the_fixture_db_for_a_render() {
    if let Some(path) = std::env::var_os("BB_NATIVE_FIXTURE_DB") {
        let path = std::path::PathBuf::from(path);
        assert!(
            !path.exists(),
            "BB_NATIVE_FIXTURE_DB names a file that exists; give a new path"
        );
        build(&path);
    }
}
