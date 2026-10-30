//! Task 2274, plan Task 2: the read-only list (spec phase 1, rows L0-L3b). The platform is injected, so these run on
//! every OS; `observe_tests` (Windows) reads real placeholders.
use super::list::{Observed, list_pairs};
use super::model::Class;
use crate::state_db::{
    FileEntry, FileStatus, ItemKind, KeptCopyInput, KeptReason, OperationKind, PendingOperation, StateDb, UploadFinalization,
};
use crate::unknown_check::{OnDisk, PlatformView};
use std::path::Path;

fn db() -> (tempfile::TempDir, StateDb) {
    let dir = tempfile::tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    (dir, db)
}

fn row(db: &StateDb, id: &str, path: &str, local_hash: Option<&str>) {
    db.upsert_file(&FileEntry {
        file_id: id.into(),
        path: path.into(),
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

fn queue(db: &StateDb, op_id: &str, file_id: &str) {
    db.enqueue_operation(&PendingOperation {
        op_id: op_id.into(),
        kind: OperationKind::UploadVersion,
        file_id: Some(file_id.into()),
        parent_id: None,
        target_path: None,
        metadata_json: None,
        payload_path: None,
        base_version: None,
        base_object_version_id: None,
        attempts: 0,
        max_attempts: 25,
        next_retry_at: 0,
        last_error: None,
        backup_source_key: None,
        created_at: 0,
        updated_at: 0,
    })
    .unwrap();
}

fn kept(db: &StateDb, file_id: &str) {
    db.record_kept_copy(&KeptCopyInput {
        original_file_id: file_id,
        original_path: "a.txt",
        copy_path: "a (this PC).txt",
        copied_hash: "h",
        original_removed: false,
        reason: KeptReason::Copy,
        kept_at: 0,
    })
    .unwrap();
}

fn placeholder(identity: Option<&str>, in_sync: bool) -> Observed {
    Observed {
        view: PlatformView::Placeholder { on_disk: OnDisk::Wholly, in_sync, identity: identity.map(str::to_string) },
        size: Some(10),
        modified: Some(7),
        read_only: Some(true),
    }
}

fn listed(db: &StateDb, observed: Observed) -> super::list::Listing {
    list_pairs(db, Path::new("C:/root"), &move |_: &Path| observed.clone()).unwrap()
}

#[test]
fn a_pair_whose_file_names_the_hashed_row_is_listed_for_the_server_read() {
    // Both id orders: A is the row the file names, whichever the state DB returns first.
    for (old, new) in [("old", "new"), ("a-old", "b-new")] {
        let (_dir, db) = db();
        row(&db, old, "Docs/a.txt", None);
        row(&db, new, "Docs/a.txt", Some("h1"));
        let listing = listed(&db, placeholder(Some(new), true));
        assert_eq!(listing.pairs.len(), 1);
        let pair = &listing.pairs[0];
        assert_eq!((pair.n, pair.a.file_id.as_str(), pair.b.file_id.as_str()), (1, new, old));
        assert_eq!(pair.class, Class::Unknown);
        assert_eq!(pair.view.identity_names.as_deref(), Some(new));
        assert_eq!((pair.view.size, pair.view.modified, pair.view.read_only), (Some(10), Some(7), Some(true)));
    }
}

#[test]
fn a_file_bound_to_neither_row_or_to_nothing_is_unbound() {
    for observed in [
        placeholder(Some("someone-else"), true),
        placeholder(None, true),
        Observed { view: PlatformView::PlainFile, ..placeholder(None, true) },
        Observed { view: PlatformView::Missing, ..placeholder(None, true) },
        Observed { view: PlatformView::Unreadable, ..placeholder(None, true) },
    ] {
        let (_dir, db) = db();
        row(&db, "old", "a.txt", None);
        row(&db, "new", "a.txt", Some("h1"));
        assert_eq!(listed(&db, observed.clone()).pairs[0].class, Class::Unbound, "{observed:?}");
    }
}

#[test]
fn a_file_bound_to_the_row_with_no_known_content_is_bound_to_b() {
    let (_dir, db) = db();
    row(&db, "old", "a.txt", None);
    row(&db, "new", "a.txt", Some("h1"));
    let pair = &listed(&db, placeholder(Some("old"), true)).pairs[0];
    assert_eq!(pair.class, Class::BoundToB);
}

#[test]
fn a_busy_upload_copy_is_left_for_a_later_run() {
    // No known content on either row.
    let (_dir, db) = db();
    row(&db, "old", "a.txt", None);
    row(&db, "new", "a.txt", None);
    assert_eq!(listed(&db, placeholder(Some("new"), true)).pairs[0].class, Class::ABusy);
    // Not in sync on the PC.
    let (_dir, db) = self::db();
    row(&db, "old", "a.txt", None);
    row(&db, "new", "a.txt", Some("h1"));
    assert_eq!(listed(&db, placeholder(Some("new"), false)).pairs[0].class, Class::ABusy);
    // An operation queued for it.
    let (_dir, db) = self::db();
    row(&db, "old", "a.txt", None);
    row(&db, "new", "a.txt", Some("h1"));
    queue(&db, "op-1", "new");
    assert_eq!(listed(&db, placeholder(Some("new"), true)).pairs[0].class, Class::ABusy);
}

#[test]
fn b_named_by_any_pending_work_is_busy() {
    for table in ["operation_queue", "kept_copies", "id_aliases"] {
        let (_dir, db) = db();
        row(&db, "old", "a.txt", None);
        row(&db, "new", "a.txt", Some("h1"));
        match table {
            "operation_queue" => queue(&db, "op-1", "old"),
            "kept_copies" => kept(&db, "old"),
            _ => db.insert_alias_for_test("provisional", "old", 0),
        }
        assert_eq!(listed(&db, placeholder(Some("new"), true)).pairs[0].class, Class::BBusy, "{table}");
    }
}

#[test]
fn a_path_with_more_than_two_rows_is_only_counted_and_slashes_fold() {
    let (_dir, db) = db();
    for id in ["x1", "x2", "x3"] {
        row(&db, id, "three.txt", None);
    }
    // One row stored with each separator: still one path with two rows.
    row(&db, "old", "Docs\\b.txt", None);
    row(&db, "new", "Docs/b.txt", Some("h1"));
    let listing = listed(&db, placeholder(Some("new"), true));
    assert_eq!(listing.more_than_two, 1);
    assert_eq!(listing.pairs.len(), 1);
    assert_eq!(listing.pairs[0].class, Class::Unknown);
}

/// Review P2 (lead, 2026-10-30): a path where either row is a folder on this PC is never a pair the tool acts on, as
/// a trash of a folder trashes everything under it.
#[test]
fn a_path_where_a_row_is_a_folder_is_not_a_file_pair() {
    for folder in ["old", "new"] {
        let (_dir, db) = db();
        row(&db, "old", "Docs", None);
        row(&db, "new", "Docs", Some("h1"));
        // The kind is part of the row's contract, which `upsert_file` leaves alone.
        let entry = db.get_file(folder).unwrap().unwrap();
        db.upsert_file_with_contract(&entry, |contract| contract.item_kind = ItemKind::Folder).unwrap();
        assert_eq!(db.get_file(folder).unwrap().unwrap().item_kind, ItemKind::Folder, "{folder}");
        let listing = listed(&db, placeholder(Some("new"), true));
        assert_eq!(listing.pairs.len(), 1, "{folder}");
        assert_eq!(listing.pairs[0].class, Class::NotAFile, "{folder}");
        assert_eq!(listing.pairs[0].action, super::model::Action::None, "{folder}");
    }
}

/// GL-0003: on a case-insensitive sync root two rows whose paths differ only in case are one file, but the list reads
/// paths exactly. Such a group is counted, never listed, so the person reviewing the list sees the gap.
#[test]
fn rows_whose_paths_differ_only_in_case_are_counted_not_listed() {
    let (_dir, db) = db();
    row(&db, "old", "Docs/Foo.txt", None);
    row(&db, "new", "docs/foo.txt", Some("h1"));
    let listing = listed(&db, placeholder(Some("new"), true));
    assert_eq!(listing.case_only, 1);
    assert!(listing.pairs.is_empty(), "{:?}", listing.pairs);
}

/// GL-0003's count folds case beyond ASCII and folds separators; an exact pair is listed and is not a case group.
#[test]
fn the_case_only_count_folds_unicode_and_separators_and_skips_exact_pairs() {
    let (_dir, db) = db();
    row(&db, "e1", "Caf\u{c9}/\u{c9}t\u{c9}.txt", None);
    row(&db, "e2", "caf\u{e9}/\u{e9}t\u{e9}.txt", Some("h1"));
    row(&db, "s1", "Docs\\Bar.txt", None);
    row(&db, "s2", "docs/bar.txt", Some("h2"));
    row(&db, "p1", "same.txt", None);
    row(&db, "p2", "same.txt", Some("h3"));
    let listing = listed(&db, placeholder(Some("p2"), true));
    assert_eq!(listing.case_only, 2);
    assert_eq!(listing.pairs.iter().map(|pair| pair.path.as_str()).collect::<Vec<_>>(), ["same.txt"]);
}

#[test]
fn pairs_are_numbered_from_one_in_path_order() {
    let (_dir, db) = db();
    for path in ["b.txt", "a.txt", "c.txt"] {
        row(&db, &format!("old-{path}"), path, None);
        row(&db, &format!("new-{path}"), path, Some("h"));
    }
    let listing = list_pairs(&db, Path::new("C:/root"), &|path: &Path| {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        placeholder(Some(&format!("new-{name}")), true)
    })
    .unwrap();
    let order: Vec<(u32, &str)> = listing.pairs.iter().map(|pair| (pair.n, pair.path.as_str())).collect();
    assert_eq!(order, [(1, "a.txt"), (2, "b.txt"), (3, "c.txt")]);
}

#[test]
fn an_unknown_content_mark_is_not_pending_work_on_either_row() {
    // B's mark acts on B's path (A's file) and goes with B's row; A's is stale once A has known content.
    let (_dir, db) = db();
    row(&db, "old", "a.txt", None);
    row(&db, "new", "a.txt", Some("h1"));
    db.set_mark_pending("old", true).unwrap();
    db.set_mark_pending("new", true).unwrap();
    assert_eq!(listed(&db, placeholder(Some("new"), true)).pairs[0].class, Class::Unknown);
}

/// Review F7: pending work that names the pair's path rather than A or B (an op queued with no file id, a finalization,
/// a kept copy from or at the path, an orphan placeholder at the path) leaves the pair busy. The path matches with
/// either separator, a leading slash, and in any case (Windows paths are case-insensitive); work at another path does
/// not count.
#[test]
fn work_at_the_path_that_names_neither_row_leaves_the_pair_busy() {
    let queue_at = |db: &StateDb, target: &str| {
        db.enqueue_operation(&PendingOperation {
            op_id: "op-path".into(),
            kind: OperationKind::UploadFile,
            file_id: None,
            parent_id: None,
            target_path: Some(target.into()),
            metadata_json: None,
            payload_path: None,
            base_version: None,
            base_object_version_id: None,
            attempts: 0,
            max_attempts: 25,
            next_retry_at: 0,
            last_error: None,
            backup_source_key: None,
            created_at: 0,
            updated_at: 0,
        })
        .unwrap()
    };
    let kept_at = |db: &StateDb, original: &str, copy: &str| {
        db.record_kept_copy(&KeptCopyInput {
            original_file_id: "other",
            original_path: original,
            copy_path: copy,
            copied_hash: "h",
            original_removed: false,
            reason: KeptReason::Copy,
            kept_at: 0,
        })
        .unwrap();
    };
    let cases: [(&str, &dyn Fn(&StateDb), Class); 9] = [
        ("queued at the path", &|db| queue_at(db, "Docs/a.txt"), Class::ABusy),
        ("queued, backslash", &|db| queue_at(db, "Docs\\a.txt"), Class::ABusy),
        ("queued, leading slash and case", &|db| queue_at(db, "/docs/A.TXT"), Class::ABusy),
        ("queued elsewhere", &|db| queue_at(db, "Docs/b.txt"), Class::Unknown),
        (
            "a finalization",
            &|db| {
                db.put_upload_finalization(&UploadFinalization {
                    op_id: "op-f".into(),
                    local_file_id: "local-x".into(),
                    server_file_id: "server-x".into(),
                    target_path: "Docs/a.txt".into(),
                    payload_path: "payload".into(),
                    stamped: false,
                    review: None,
                })
                .unwrap()
            },
            Class::ABusy,
        ),
        ("a kept copy from the path", &|db| kept_at(db, "Docs/a.txt", "Docs/a (this PC).txt"), Class::ABusy),
        ("a kept copy at the path", &|db| kept_at(db, "Docs/old.txt", "Docs/a.txt"), Class::ABusy),
        ("an orphan at the path", &|db| db.put_unknown_orphan("Docs/a.txt", "other", true).unwrap(), Class::ABusy),
        ("an orphan elsewhere", &|db| db.put_unknown_orphan("Docs/c.txt", "other", true).unwrap(), Class::Unknown),
    ];
    for (case, work, class) in cases {
        let (_dir, db) = db();
        row(&db, "old", "Docs/a.txt", None);
        row(&db, "new", "Docs/a.txt", Some("h1"));
        work(&db);
        assert_eq!(listed(&db, placeholder(Some("new"), true)).pairs[0].class, class, "{case}");
    }
}
