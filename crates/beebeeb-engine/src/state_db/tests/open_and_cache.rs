//! Tests for opening the database and the page-cache fit, split out of `state_db.rs`.

use super::*;

/// Task 2278: a fresh state database opened by several connections at the same moment is migrated once and
/// every open succeeds. `ensure_column` checked `has_column` and then ran `ALTER TABLE ... ADD COLUMN` with no
/// transaction around it, so two first opens could both see `fetched_min_version` missing and the second ALTER
/// failed with "duplicate column name: fetched_min_version" — the macOS CI flake of 2026-10-29. 50 rounds because
/// the window between the check and the ALTER is short; one round names the real code in its failure message.
#[test]
fn every_concurrent_first_open_of_a_fresh_database_succeeds() {
    const THREADS: usize = 8;
    const ROUNDS: usize = 50;
    // Eight threads on fewer cores still interleave between `has_column` and the `ALTER`; the count only has to be
    // enough for two openers to meet. No CPU-count premise is asserted: CI runners have four.
    for round in 0..ROUNDS {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(THREADS));
        let failures: Vec<String> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..THREADS)
                .map(|_| {
                    let path = path.clone();
                    let barrier = barrier.clone();
                    scope.spawn(move || {
                        barrier.wait();
                        StateDb::open(&path).map(|_| ())
                    })
                })
                .collect();
            handles
                .into_iter()
                .filter_map(|handle| handle.join().expect("no opener panics").err().map(|e| e.to_string()))
                .collect()
        });
        assert!(
            failures.is_empty(),
            "round {round}: {THREADS} threads opened one fresh database and {} of them failed: {failures:?}",
            failures.len()
        );
    }
}

/// Task 2278, belt and braces: the ALTER that loses a column race is tolerated, and nothing else is. The window is
/// made here instead of waited for: the column is added by another connection before `add_column` runs, exactly what
/// a second opener did between `has_column` and the `ALTER`. Remove the tolerance and this test goes red on the
/// duplicate; a missing table must still be an error, so the tolerance cannot swallow a real failure.
#[test]
fn a_column_added_by_another_connection_is_not_an_error_but_a_missing_table_is() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let db = StateDb::open(&path).unwrap();
    let conn = Connection::open(&path).unwrap();
    {
        let reader = db.0.lock().unwrap();
        assert!(
            !has_column(&reader, "files", "race_probe").unwrap(),
            "the premise: the column is missing"
        );
        // The other connection wins the race between that check and the ALTER.
        conn.execute("ALTER TABLE files ADD COLUMN race_probe TEXT", [])
            .unwrap();
        add_column(&reader, "files", "race_probe", "TEXT").expect("a duplicate column is success");
        assert!(has_column(&reader, "files", "race_probe").unwrap());
        let error =
            add_column(&reader, "no_such_table", "race_probe", "TEXT").expect_err("a real failure still propagates");
        assert!(format!("{error}").contains("no such table"), "{error}");
    }
    // And the migration's own entry point is idempotent over it, on this database too.
    ensure_column(&conn, "files", "race_probe", "TEXT").unwrap();
}

/// The Windows sign-out leaves no account data behind, whatever `ACCOUNT_TABLES` and `DEVICE_TABLES` list: one row
/// goes into every listed table it does not refuse on (a dummy value per column, from the schema), and after
/// `finish_windows_signout` every listed table is empty and `has_account_data` is false. A table added to either list
/// and not cleared, or listed but missing from the schema, fails here. The three tables it refuses on (an unsent
/// change is never discarded) must already be empty; the tests above cover each refusal.
#[test]
fn a_windows_sign_out_empties_every_account_table() {
    const REFUSED_ON: [&str; 3] = ["operation_queue", "staged_payloads", "upload_finalizations"];
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let db = StateDb::open(&path).unwrap();
    {
        let conn = Connection::open(&path).unwrap();
        for table in ACCOUNT_TABLES.iter().chain(DEVICE_TABLES.iter()) {
            let columns: Vec<String> = conn
                .prepare(&format!("PRAGMA table_info({table})"))
                .unwrap()
                .query_map([], |row| {
                    Ok(format!(
                        "{}|{}",
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?.to_ascii_uppercase()
                    ))
                })
                .unwrap()
                .collect::<std::result::Result<_, _>>()
                .unwrap();
            assert!(
                !columns.is_empty(),
                "{table} is listed in ACCOUNT_TABLES or DEVICE_TABLES but the schema has no such table"
            );
            if REFUSED_ON.contains(table) {
                continue;
            }
            let (names, values): (Vec<&str>, Vec<&str>) = columns
                .iter()
                .map(|column| {
                    let (name, kind) = column.split_once('|').unwrap();
                    let value = if kind.contains("INT") {
                        "1"
                    } else if kind.contains("REAL") {
                        "1.0"
                    } else if kind.contains("BLOB") {
                        "x'00'"
                    } else {
                        "'x'"
                    };
                    (name, value)
                })
                .unzip();
            conn.execute(
                &format!(
                    "INSERT INTO {table} ({}) VALUES ({})",
                    names.join(", "),
                    values.join(", ")
                ),
                [],
            )
            .unwrap();
        }
    }
    assert!(
        db.has_account_data().unwrap(),
        "the premise: the account tables hold rows"
    );
    db.finish_windows_signout().expect("nothing waits to upload");
    let conn = Connection::open(&path).unwrap();
    for table in ACCOUNT_TABLES.iter().chain(DEVICE_TABLES.iter()) {
        let rows: i64 = conn
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| row.get(0))
            .unwrap();
        assert_eq!(rows, 0, "the Windows sign-out left rows in {table}");
    }
    assert!(!db.has_account_data().unwrap(), "no account data is left");
}

/// Task 2240 (D2, T1): the limit is the DB's size plus an eighth, within [2,000 KiB, 128 MiB].
#[test]
fn task_2240_the_page_cache_limit_follows_the_db_size() {
    for (db_bytes, kib) in [
        (0, 2_000),
        (1 << 20, 2_000),
        (30_707_712, 33_736),
        (120 << 20, 131_072),
        (2_i64 << 30, 131_072),
    ] {
        assert_eq!(page_cache_limit_kib(db_bytes), kib, "{db_bytes} bytes");
    }
}

/// Task 2240 (T2): with SQLite's default cache, the scan's second pass reads its pages from the file again; with
/// the cache fitted it reads none. The default handle is measured first and closed: with one shared LRU, it would
/// otherwise borrow the fitted handle's pages.
#[test]
fn task_2240_a_fitted_cache_reads_nothing_on_the_second_pass() {
    task_2240_in_own_process("task_2240_a_fitted_cache_reads_nothing_on_the_second_pass", || {
        let (_dir, path) = task_2240_db();
        let default = StateDb::open(&path).unwrap();
        task_2240_pass(&default);
        let unfitted = task_2240_pass(&default);
        assert!(unfitted > 0, "precondition: the default cache cannot hold the pass");
        drop(default);
        let fitted = StateDb::open(&path).unwrap();
        fitted.fit_page_cache().unwrap();
        let cold = task_2240_pass(&fitted);
        let warm = task_2240_pass(&fitted);
        assert_eq!(warm, 0, "a fitted cache holds every page the pass reads");
        format!("default cache: second pass read {unfitted} pages; fitted: {cold} then {warm}")
    });
}

/// Task 2240 (T3, premise P1): every UI command opens the DB on its own connection. Opening it commits nothing, so
/// the scan's cache survives it. An `open` that writes would empty that cache on every poll.
#[test]
fn task_2240_a_ui_open_between_passes_keeps_the_cache() {
    task_2240_in_own_process("task_2240_a_ui_open_between_passes_keeps_the_cache", || {
        let (_dir, path) = task_2240_db();
        let engine = StateDb::open(&path).unwrap();
        engine.fit_page_cache().unwrap();
        task_2240_pass(&engine);
        drop(StateDb::open(&path).unwrap());
        let warm = task_2240_pass(&engine);
        assert_eq!(warm, 0, "opening the DB elsewhere leaves the scan's cache warm");
        format!("pass after a UI open read {warm} pages")
    });
}

/// Task 2240 (T4, I4): a commit by another connection empties the cache; the next pass reads cold once, and the
/// one after it reads nothing again.
#[test]
fn task_2240_a_commit_elsewhere_costs_one_cold_pass() {
    task_2240_in_own_process("task_2240_a_commit_elsewhere_costs_one_cold_pass", || {
        let (_dir, path) = task_2240_db();
        let engine = StateDb::open(&path).unwrap();
        engine.fit_page_cache().unwrap();
        task_2240_pass(&engine);
        Connection::open(&path)
            .unwrap()
            .execute(
                "UPDATE files SET last_opened_at = last_opened_at + 1 WHERE file_id = 'f0'",
                [],
            )
            .unwrap();
        let cold = task_2240_pass(&engine);
        assert!(cold > 0, "a commit elsewhere empties the cache");
        let warm = task_2240_pass(&engine);
        assert_eq!(warm, 0, "one pass warms it again");
        format!("after a commit elsewhere: {cold} pages, then {warm}")
    });
}

/// Task 2240 (I6): a UI command that reads more than its own 2,000 KiB on its connection takes the scan's oldest
/// pages (one shared LRU). The next pass re-reads at most what that command read, never more than a cold pass, and
/// the one after it reads nothing.
#[test]
fn task_2240_a_large_ui_read_costs_at_most_one_partial_pass() {
    task_2240_in_own_process("task_2240_a_large_ui_read_costs_at_most_one_partial_pass", || {
        let (_dir, path) = task_2240_db();
        let engine = StateDb::open(&path).unwrap();
        engine.fit_page_cache().unwrap();
        let full = task_2240_pass(&engine);
        let ui = StateDb::open(&path).unwrap();
        let before = ui.page_reads();
        let _: i64 =
            ui.0.lock()
                .unwrap()
                .query_row("SELECT sum(length(content_type)) FROM files", [], |row| row.get(0))
                .unwrap();
        let ui_read = ui.page_reads() - before;
        drop(ui);
        let partial = task_2240_pass(&engine);
        assert!(partial > 0, "precondition: the UI read took some of the scan's pages");
        assert!(
            partial <= ui_read && partial <= full,
            "{partial} pages, the UI read {ui_read}, a cold pass {full}"
        );
        let warm = task_2240_pass(&engine);
        assert_eq!(warm, 0, "one pass warms it again");
        format!("cold pass {full}; UI read {ui_read}; next pass {partial}, then {warm}")
    });
}

/// Task 2240 (T5, I2): only the connection that is fitted gets the larger cache; a UI command's fresh one keeps
/// SQLite's default.
#[test]
fn task_2240_ui_handles_keep_the_default_cache() {
    let (_dir, path) = task_2240_db();
    let engine = StateDb::open(&path).unwrap();
    let fitted = engine.fit_page_cache().unwrap();
    assert!(
        fitted > PAGE_CACHE_FLOOR_KIB,
        "precondition: this DB fits above the floor"
    );
    assert_eq!(task_2240_cache_size(&engine), -fitted);
    assert_eq!(
        task_2240_cache_size(&StateDb::open(&path).unwrap()),
        -PAGE_CACHE_FLOOR_KIB
    );
}

/// Task 2240 (T6, D3): a DB that grows gets a larger cache at the next fit.
#[test]
fn task_2240_the_cache_is_refitted_when_the_db_grows() {
    let (_dir, path) = task_2240_db();
    let engine = StateDb::open(&path).unwrap();
    let first = engine.fit_page_cache().unwrap();
    task_2240_add_rows(&path, TASK_2240_ROWS, 10_000, 4_000);
    let second = engine.fit_page_cache().unwrap();
    assert!(
        second > first,
        "the limit grows with the DB: {first} KiB, then {second} KiB"
    );
    assert_eq!(task_2240_cache_size(&engine), -second);
}

/// Task 2229: the enumeration scan looks every file and folder on disk up by path on every
/// pass, so the lookup steps no row of a full scan of `files`, whether the path is there or
/// not. That holds on a DB opened from a release before the index (0.8.15 and earlier),
/// because opening adds it. The counters read are those of the app's own cached statement,
/// so the lookup `get_file_by_path` runs is the one measured.
#[test]
fn a_path_lookup_reads_through_an_index_not_the_whole_files_table() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    {
        StateDb::open(&path).unwrap();
        let mut conn = Connection::open(&path).unwrap();
        let tx = conn.transaction().unwrap();
        for i in 0..2000 {
            tx.execute(
                "INSERT INTO files (file_id, path) VALUES (?1, ?2)",
                params![format!("f{i}"), format!("dir/{i}.txt")],
            )
            .unwrap();
        }
        tx.commit().unwrap();
        // The schema 0.8.15 shipped: no index on `files.path`.
        conn.execute("DROP INDEX IF EXISTS idx_files_path", []).unwrap();
    }
    let db = StateDb::open(&path).unwrap();
    for (probe, found) in [("dir/1999.txt", Some("f1999")), ("dir/missing.txt", None)] {
        assert_eq!(
            db.get_file_by_path(probe).unwrap().map(|e| e.file_id),
            found.map(str::to_string)
        );
        assert_eq!(db.file_row_stamp_by_path(probe).unwrap().is_some(), found.is_some());
        let conn = db.0.lock().unwrap();
        for sql in [FILE_BY_PATH_SQL, FILE_ROW_BY_PATH_SQL] {
            let stmt = conn.prepare_cached(sql).unwrap();
            assert!(
                stmt.get_status(rusqlite::StatementStatus::Run) > 0,
                "the app did not run {sql}"
            );
            let full_scan_steps = stmt.reset_status(rusqlite::StatementStatus::FullscanStep);
            stmt.reset_status(rusqlite::StatementStatus::Run);
            assert_eq!(
                full_scan_steps, 0,
                "looking up {probe} stepped {full_scan_steps} rows of a full scan of files: {sql}"
            );
        }
    }
}

/// Task 2229: the scan skips a settled file while the stamp of its row is unchanged, so every
/// write to the row changes the stamp, and either stored slash shape finds it.
#[test]
fn a_row_stamp_changes_with_any_write_to_the_row() {
    let dir = tempfile::tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    assert_eq!(
        db.file_row_stamp_by_path("docs/a.txt").unwrap(),
        None,
        "no row, no stamp"
    );
    {
        let conn = db.0.lock().unwrap();
        conn.execute("INSERT INTO files (file_id, path) VALUES ('f1', '/docs/a.txt')", [])
            .unwrap();
    }
    let first = db.file_row_stamp_by_path("docs/a.txt").unwrap();
    assert!(first.is_some(), "the leading-slash shape is found too");
    assert_eq!(db.file_row_stamp_by_path("/docs/a.txt").unwrap(), first);
    assert_eq!(
        db.file_row_stamp_by_path("docs/a.txt").unwrap(),
        first,
        "unchanged row, same stamp"
    );
    for write in [
        "UPDATE files SET status = 'local' WHERE file_id = 'f1'",
        "UPDATE files SET local_hash = 'h1' WHERE file_id = 'f1'",
        "UPDATE files SET local_hash = NULL WHERE file_id = 'f1'",
        "UPDATE files SET local_base_version = 2 WHERE file_id = 'f1'",
        "UPDATE files SET held_write_id = 'w1' WHERE file_id = 'f1'",
    ] {
        let before = db.file_row_stamp_by_path("docs/a.txt").unwrap();
        db.0.lock().unwrap().execute(write, []).unwrap();
        assert_ne!(db.file_row_stamp_by_path("docs/a.txt").unwrap(), before, "{write}");
    }
}

/// Task 2271: when several rows share a path the scan may judge the bytes by any of them, so a write to the second
/// row changes the stamp too, and `file_rows_at_path` lists them all in the order `get_file_by_path` meets them.
#[test]
fn a_row_stamp_covers_every_row_at_the_path() {
    let dir = tempfile::tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    {
        let conn = db.0.lock().unwrap();
        conn.execute("INSERT INTO files (file_id, path) VALUES ('first', 'docs/a.txt')", [])
            .unwrap();
        conn.execute("INSERT INTO files (file_id, path) VALUES ('second', 'docs/a.txt')", [])
            .unwrap();
    }
    let ids = |path: &str| {
        db.file_rows_at_path(path)
            .unwrap()
            .into_iter()
            .map(|e| e.file_id)
            .collect::<Vec<_>>()
    };
    assert_eq!(ids("docs/a.txt"), ["first", "second"]);
    assert_eq!(
        ids("/docs/a.txt"),
        ["first", "second"],
        "the other slash shape finds them too"
    );
    assert_eq!(db.get_file_by_path("docs/a.txt").unwrap().unwrap().file_id, "first");
    let before = db.file_row_stamp_by_path("docs/a.txt").unwrap();
    db.0.lock()
        .unwrap()
        .execute("UPDATE files SET local_hash = 'h2' WHERE file_id = 'second'", [])
        .unwrap();
    assert_ne!(
        db.file_row_stamp_by_path("docs/a.txt").unwrap(),
        before,
        "a write to the second row"
    );
}
