//! Task 2524: a sync-folder edit the engine holds (`held_edits`) is listed with true words and no action, one test per
//! held reason, read through the state database the way the window reads it.

use super::*;
use crate::state_db::{FileEntry, FileStatus, ItemKind, StateDb};

fn db_with_held(file_id: &str, path: &str, reason: &str, at: i64) -> (tempfile::TempDir, StateDb) {
    let dir = tempfile::tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    db.upsert_file(&FileEntry {
        file_id: file_id.to_string(),
        path: path.to_string(),
        status: FileStatus::Local,
        size_bytes: 1,
        modified_at: 1,
        content_hash: None,
        remote_updated_at: 1,
        parent_id: None,
        item_kind: ItemKind::File,
    })
    .unwrap();
    db.hold_unsent_edit(file_id, reason, "fp", at).unwrap();
    (dir, db)
}

#[test]
fn an_edit_whose_base_is_unknown_is_listed_with_its_words_and_no_action() {
    let (_dir, db) = db_with_held("f-1", "Invented/Plan.txt", "base_unknown", 1_800_000_000);
    let problems = held_edit_problems(&db.list_held_edits().unwrap());
    assert_eq!(problems.len(), 1, "one held edit, one problem");
    let p = &problems[0];
    assert_eq!((p.name.as_str(), p.folder.as_str()), ("Plan.txt", "Invented"));
    assert_eq!(p.id, "held:f-1");
    assert_eq!(p.file_id.as_deref(), Some("f-1"));
    assert!(!p.retryable, "the engine has no action for a held edit");
    assert!(!p.backup);
    assert!(p.at.is_some());
    assert!(p.reason.contains("which version"), "{}", p.reason);
    assert!(!p.reason.contains("0.8.13"), "{}", p.reason);
}

#[test]
fn a_copy_whose_content_is_unknown_is_listed_without_a_made_up_cause() {
    let (_dir, db) = db_with_held("f-2", "Invented/Sheet.xlsx", "content_unknown", 1_800_000_000);
    let problems = held_edit_problems(&db.list_held_edits().unwrap());
    assert_eq!(problems.len(), 1);
    let p = &problems[0];
    assert_eq!(p.id, "held:f-2");
    assert!(!p.retryable);
    assert!(p.reason.contains("can’t tell"), "{}", p.reason);
    assert!(!p.reason.contains("0.8.13"), "never the pre-0.8.13 label: {}", p.reason);
    assert!(!p.reason.contains("older version"), "{}", p.reason);
}

#[test]
fn a_reason_this_build_does_not_know_is_still_listed() {
    let (_dir, db) = db_with_held("f-3", "Loose.md", "a_reason_from_a_later_build", 5);
    let problems = held_edit_problems(&db.list_held_edits().unwrap());
    assert_eq!(problems.len(), 1);
    assert_eq!(problems[0].folder, "");
    assert!(problems[0].reason.starts_with("Not sent"), "{}", problems[0].reason);
}

#[test]
fn nothing_held_lists_nothing() {
    assert!(held_edit_problems(&[]).is_empty());
}
