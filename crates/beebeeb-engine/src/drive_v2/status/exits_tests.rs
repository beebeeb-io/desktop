//! Task 2530 (wave 2, slice S3): every park reason, and the two held kinds, lists the ways out the contract gives it
//! (`spec-parked-exit.md` §"Contract"), read through the real queue read (`StateDb::unsent_changes`) and
//! [`sync_problems`]. Try again is listed exactly where it is an exit. Every name is invented.

use super::*;
use crate::state_db::{FileEntry, FileStatus, ItemKind, StateDb};

fn db() -> (tempfile::TempDir, StateDb) {
    let dir = tempfile::tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    (dir, db)
}

fn file(db: &StateDb, file_id: &str, version: i64) {
    db.upsert_file(&FileEntry {
        file_id: file_id.to_string(),
        path: format!("Invented/{file_id}.txt"),
        status: FileStatus::Error,
        size_bytes: 1,
        modified_at: 1,
        content_hash: None,
        remote_updated_at: 1,
        parent_id: None,
        item_kind: ItemKind::File,
    })
    .unwrap();
    db.test_execute(&format!(
        "UPDATE files SET current_version = {version} WHERE file_id = '{file_id}'"
    ))
    .unwrap();
}

/// A parked content write of `file_id` on version 1, as the claim parks it. `write_id`: a viewer save's (or a File
/// Provider write's); `None`: a sync-folder edit's. `staged`: its staged copy's path.
fn park(db: &StateDb, file_id: &str, class: &str, write_id: Option<&str>, staged: &str) {
    let write = write_id.map_or("NULL".to_string(), |w| format!("'{w}'"));
    db.test_execute(&format!(
        "INSERT INTO operation_queue (op_id, kind, file_id, target_path, metadata_json, payload_path, base_version,
                                      attempts, max_attempts, next_retry_at, last_error, last_error_class,
                                      created_at, updated_at, write_id, write_origin)
         VALUES ('op-{file_id}', 'upload_version', '{file_id}', 'Invented/{file_id}.txt', '{{}}', '{staged}', 1,
                 25, 25, 0, '{class}', '{class}', 5, 5, {write}, 'minted')"
    ))
    .unwrap();
}

fn exits_by_id(db: &StateDb) -> Vec<(String, Vec<Exit>, bool, &'static str)> {
    let queue = db.unsent_changes(0, 500, 10).unwrap().items;
    let mut problems = sync_problems(&queue);
    problems.extend(held_edit_problems(&db.held_edit_overview(500).unwrap()));
    let mut rows: Vec<_> = problems
        .into_iter()
        .map(|p| (p.id, p.exits, p.retryable, p.reason))
        .collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    rows
}

#[test]
fn each_park_reason_lists_its_exits() {
    let (dir, db) = db();
    let here = dir.path().join("staged-here");
    std::fs::write(&here, b"this PC's bytes").unwrap();
    let here = here.to_string_lossy().into_owned();
    let gone = dir.path().join("staged-gone").to_string_lossy().into_owned();

    // A viewer save on a stale base: the file is past it.
    file(&db, "stale", 2);
    park(&db, "stale", "stale_base", Some("w-stale"), &here);
    // A sync-folder edit on a stale base (no write id): nothing in this build settles it (wave 3).
    file(&db, "stale-sync", 2);
    park(&db, "stale-sync", "stale_base", None, &here);
    file(&db, "lost", 1);
    park(&db, "lost", "predecessor_lost", Some("w-lost"), &here);
    file(&db, "refused", 1);
    park(&db, "refused", "permission_denied", Some("w-refused"), &here);
    file(&db, "rekey", 1);
    park(&db, "rekey", "rekey_failed", Some("w-rekey"), &here);
    file(&db, "waits", 1);
    park(&db, "waits", "predecessor_parked", Some("w-waits"), &here);
    file(&db, "missing", 1);
    park(&db, "missing", "payload_missing", Some("w-missing"), &gone);
    file(&db, "unknown", 0);
    park(&db, "unknown", "base_unknown", Some("w-unknown"), &here);
    // Task 2524's held kinds.
    file(&db, "held", 1);
    db.hold_unsent_edit("held", "base_unknown", "fp", 3).unwrap();
    file(&db, "unchecked", 1);
    db.hold_unsent_edit("unchecked", "content_unknown", "fp", 4).unwrap();

    let keeps = vec![Exit::KeepBoth, Exit::KeepOther, Exit::KeepThisPcs];
    let remove = if cfg!(target_os = "macos") {
        vec![]
    } else {
        vec![Exit::RemoveFromList]
    };
    let want: Vec<(&str, Vec<Exit>)> = vec![
        ("held:content_unknown", vec![]),
        ("held:held", vec![]),
        ("op-lost", vec![Exit::KeepBoth, Exit::KeepOther]),
        ("op-missing", remove),
        ("op-refused", vec![Exit::KeepOther]),
        ("op-rekey", vec![Exit::TryAgain]),
        ("op-stale", keeps),
        ("op-stale-sync", vec![]),
        ("op-unknown", vec![]),
        ("op-waits", vec![]),
    ];
    let got = exits_by_id(&db);
    assert_eq!(
        got.iter().map(|r| (r.0.as_str(), r.1.clone())).collect::<Vec<_>>(),
        want,
        "the contract table, row by row"
    );
    for (id, exits, retryable, reason) in &got {
        assert_eq!(
            *retryable,
            exits.contains(&Exit::TryAgain),
            "{id}: Try again exactly where it is an exit"
        );
        assert!(!reason.is_empty(), "{id} says why");
    }
    let words = |id: &str| got.iter().find(|r| r.0 == id).unwrap().3;
    // R4: a write that waits names the change it waits for, and promises no button.
    assert!(words("op-waits").contains("an earlier change to this file"));
    assert!(words("op-missing").contains("no longer has the bytes of this change"));
    assert!(words("op-stale").contains("Choose which version to keep"));
    assert!(
        !words("op-stale-sync").contains("Choose"),
        "no choice promised where none is offered"
    );
}

/// A choice is made against the file's version the list was read at: a row with a way out carries it.
#[test]
fn a_row_with_a_way_out_carries_the_version_read() {
    let (dir, db) = db();
    let here = dir.path().join("staged-here");
    std::fs::write(&here, b"this PC's bytes").unwrap();
    file(&db, "stale", 7);
    park(&db, "stale", "stale_base", Some("w-stale"), &here.to_string_lossy());
    file(&db, "waits", 3);
    park(
        &db,
        "waits",
        "predecessor_parked",
        Some("w-waits"),
        &here.to_string_lossy(),
    );
    let queue = db.unsent_changes(0, 500, 10).unwrap().items;
    let problems = sync_problems(&queue);
    let version = |id: &str| problems.iter().find(|p| p.id == id).unwrap().version;
    assert_eq!(version("op-stale"), Some(7));
    assert_eq!(version("op-waits"), None, "no way out, no version");
}

/// The Uploads panel offers no way out of a parked change: its words never promise a choice.
#[test]
fn the_uploads_panel_never_promises_a_choice() {
    let (dir, db) = db();
    let here = dir.path().join("staged-here");
    std::fs::write(&here, b"this PC's bytes").unwrap();
    file(&db, "stale", 2);
    park(&db, "stale", "stale_base", Some("w-stale"), &here.to_string_lossy());
    let queue = db.unsent_changes(0, 500, 10).unwrap().items;
    let rows = transfer_rows(&[], &queue);
    let reason = rows.iter().find(|r| r.id == "op-stale").and_then(|r| r.reason).unwrap();
    assert!(!reason.contains("Choose"), "{reason}");
    assert!(sync_problems(&queue)[0].reason.contains("Choose which version to keep"));
}
