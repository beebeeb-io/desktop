//! Task 2524: the engine holds an edit it will not send (`held_edits`); This PC lists it with true words and no action.
//! A held edit whose base is unknown is one row; the files whose agreed content is not recorded are ONE summary row.

use super::*;
use crate::state_db::{FileEntry, FileStatus, ItemKind, StateDb};

const LIMIT: u32 = 500;

fn entry(file_id: &str, path: &str) -> FileEntry {
    FileEntry {
        file_id: file_id.to_string(),
        path: path.to_string(),
        status: FileStatus::Local,
        size_bytes: 1,
        modified_at: 1,
        content_hash: None,
        remote_updated_at: 1,
        parent_id: None,
        item_kind: ItemKind::File,
    }
}

fn db() -> (tempfile::TempDir, StateDb) {
    let dir = tempfile::tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    (dir, db)
}

fn hold(db: &StateDb, file_id: &str, path: &str, reason: &str, at: i64) {
    db.upsert_file(&entry(file_id, path)).unwrap();
    db.hold_unsent_edit(file_id, reason, "fp", at).unwrap();
}

/// `count` unchecked files `Invented/File 0000.txt`.. found at 1_800_000_000 + n, in two statements (a row at a time
/// is one fsync each).
fn hold_many(db: &StateDb, count: i64) {
    let rows = format!(
        "WITH RECURSIVE n(i) AS (SELECT 0 UNION ALL SELECT i + 1 FROM n WHERE i < {})",
        count - 1
    );
    db.test_execute(&format!(
        "INSERT INTO files (file_id, path, status) {rows}
         SELECT printf('f-%04d', i), printf('Invented/File %04d.txt', i), 'local' FROM n"
    ))
    .unwrap();
    db.test_execute(&format!(
        "INSERT INTO held_edits (file_id, reason, fingerprint, detected_at) {rows}
         SELECT printf('f-%04d', i), 'content_unknown', 'fp', 1800000000 + i FROM n"
    ))
    .unwrap();
}

fn problems(db: &StateDb) -> Vec<SyncProblem> {
    held_edit_problems(&db.held_edit_overview(LIMIT).unwrap())
}

#[test]
fn an_edit_whose_base_is_unknown_is_one_row_with_its_words_and_no_action() {
    let (_dir, db) = db();
    hold(&db, "f-1", "Invented/Plan.txt", "base_unknown", 1_800_000_000);
    let problems = problems(&db);
    assert_eq!(problems.len(), 1, "one held edit, one problem");
    let p = &problems[0];
    assert_eq!((p.name.as_str(), p.folder.as_str()), ("Plan.txt", "Invented"));
    assert_eq!(p.id, "held:f-1");
    assert_eq!(p.file_id.as_deref(), Some("f-1"));
    assert!(!p.retryable, "the engine has no action for a held edit");
    assert!(!p.backup);
    assert!(p.at.is_some());
    assert!(p.reason.contains("which version"), "{}", p.reason);
}

#[test]
fn five_thousand_unchecked_files_are_one_row_with_the_count_and_the_first_names() {
    let (_dir, db) = db();
    hold_many(&db, 5_000);
    let problems = problems(&db);
    assert_eq!(problems.len(), 1, "never a row per file");
    let p = &problems[0];
    assert_eq!(p.id, "held:content_unknown");
    assert!(p.name.starts_with("5000 files not checked yet ("), "{}", p.name);
    for n in 0..5 {
        assert!(p.name.contains(&format!("File {n:04}.txt")), "{}", p.name);
    }
    assert!(!p.name.contains("File 0005.txt"), "only the first five: {}", p.name);
    assert!(p.name.ends_with("and 4995 more)"), "{}", p.name);
    assert!(!p.retryable);
    assert!(p.at.is_some());
    assert!(p.reason.contains("Nothing to do now"), "{}", p.reason);
    assert!(p.reason.contains("until they are checked"), "{}", p.reason);
    assert!(!p.reason.contains("0.8.13"), "{}", p.reason);
}

#[test]
fn one_unchecked_file_says_one_file_and_no_more() {
    let (_dir, db) = db();
    hold(&db, "f-2", "Invented/Sheet.xlsx", "content_unknown", 5);
    let problems = problems(&db);
    assert_eq!(problems.len(), 1);
    assert_eq!(problems[0].name, "1 file not checked yet (Sheet.xlsx)");
}

#[test]
fn unchecked_files_and_held_edits_are_counted_apart_and_the_read_is_capped() {
    let (_dir, db) = db();
    hold(&db, "u-1", "A.txt", "content_unknown", 1);
    hold(&db, "u-2", "B.txt", "content_unknown", 2);
    for n in 0..7 {
        hold(&db, &format!("b-{n}"), &format!("Held {n}.txt"), "base_unknown", 10 + n);
    }
    let all = db.held_edit_overview(LIMIT).unwrap();
    assert_eq!((all.each.len(), all.unchecked), (7, 2));
    let capped = db.held_edit_overview(3).unwrap();
    assert_eq!(capped.each.len(), 3, "the per-file read is capped");
    assert_eq!(capped.unchecked, 2, "the count is not");
    assert_eq!(
        held_edit_problems(&capped).len(),
        4,
        "3 held edits + the one summary row"
    );
    assert_eq!(capped.detected_at("held:content_unknown"), 1);
    assert_eq!(capped.detected_at("held:b-0"), 10);
}

#[test]
fn a_reason_this_build_does_not_know_says_only_that_it_is_held_back() {
    let (_dir, db) = db();
    hold(&db, "f-3", "Loose.md", "a_reason_from_a_later_build", 5);
    let problems = problems(&db);
    assert_eq!(problems.len(), 1);
    assert_eq!(problems[0].folder, "");
    assert_eq!(
        problems[0].reason,
        crate::os_words::for_this_os("Not sent: this PC is holding this change back.")
    );
}

#[test]
fn nothing_held_lists_nothing() {
    assert!(held_edit_problems(&HeldEdits::default()).is_empty());
}
