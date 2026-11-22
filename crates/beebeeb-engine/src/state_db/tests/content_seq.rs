//! Task 2199 (dc-103b): `files.content_seq`, the N7 hash clear on a restore, and the candidate wipe after a
//! dehydrate (design 1640 "Not done" (a)).

use super::*;

const FILE: &str = "f97d7f90-511b-52b5-9303-7201b53df85f";

fn seeded() -> (tempfile::TempDir, std::path::PathBuf, StateDb) {
    let dir = tempdir().unwrap();
    let path = dir.path().join("state.db");
    let db = StateDb::open(&path).unwrap();
    db.upsert_file(&FileEntry {
        file_id: FILE.into(),
        path: "notes.txt".into(),
        status: FileStatus::Local,
        size_bytes: 1,
        modified_at: 1,
        content_hash: None,
        remote_updated_at: 1,
        parent_id: None,
        item_kind: ItemKind::File,
    })
    .unwrap();
    (dir, path, db)
}

fn seq(db: &StateDb) -> i64 {
    db.content_seq(FILE).unwrap().unwrap()
}

fn agreed(db: &StateDb) -> (i64, Option<String>) {
    let contract = db.get_file_contract_state(FILE).unwrap().unwrap();
    (contract.local_base_version, contract.local_hash)
}

/// A database from 0.8.25 has no `content_seq` and no trigger (its `migrate_schema` is this one without them:
/// `git diff desktop-v0.8.25 -- state_db/schema.rs` adds neither another column nor another table). The column is
/// added at 0 on every row, the row's content kept, and a second and third open change nothing.
#[test]
fn content_seq_is_added_to_a_0_8_25_database_once() {
    let (_dir, path, db) = seeded();
    db.mark_no_fetched_content(FILE).unwrap();
    db.record_fetched_version(FILE, 3, "h3").unwrap();
    drop(db);
    {
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "DROP TRIGGER files_content_seq_follows_content;
             ALTER TABLE files DROP COLUMN content_seq;",
        )
        .unwrap();
        let has: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM pragma_table_info('files') WHERE name = 'content_seq')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(!has, "the 0.8.25 shape");
    }
    for _ in 0..3 {
        let db = StateDb::open(&path).unwrap();
        assert_eq!(seq(&db), 0, "an older row starts at 0");
        assert_eq!(db.fetched_content(FILE).unwrap(), Some((3, Some("h3".into()))), "its content kept");
    }
    let db = StateDb::open(&path).unwrap();
    db.record_fetched_version(FILE, 4, "h4").unwrap();
    assert!(seq(&db) > 0, "the migrated row's trigger and fetch records count");
}

/// Every fetch recorded moves it on, also one that changes no column (the same version again: its bytes are still
/// landing); every change of the agreed content or the candidate moves it on; anything else does not.
#[test]
fn content_seq_moves_on_every_fetch_and_every_content_change() {
    let (_dir, _path, db) = seeded();
    let start = seq(&db);
    db.record_fetched_version(FILE, 3, "h3").unwrap();
    let after_first = seq(&db);
    assert!(after_first > start, "a fetch");
    db.record_fetched_version(FILE, 3, "h3").unwrap();
    let after_same = seq(&db);
    assert!(after_same > after_first, "the same version again, no column changed");
    db.record_fetch_candidate(FILE, 3).unwrap();
    let after_range = seq(&db);
    assert!(after_range > after_same, "a range session's record");
    db.mark_content_unknown(FILE).unwrap();
    let after_unknown = seq(&db);
    assert!(after_unknown > after_range, "a download that named no version");
    db.mark_content_unknown(FILE).unwrap();
    assert!(seq(&db) > after_unknown, "again, no column changed");
    let before_baseline = seq(&db);
    db.record_local_baseline(FILE, 5, "h5").unwrap();
    assert!(seq(&db) > before_baseline, "a baseline (the trigger)");
    let settled = seq(&db);
    db.set_status(FILE, FileStatus::Downloading).unwrap();
    db.mark_cloud_only_after_dehydrate(&[], 9).unwrap();
    assert_eq!(seq(&db), settled, "no content column changed");
}

/// N7: a restore moves the base to a version whose bytes are not on disk. The old hash is never left paired with
/// the new base: it joins the candidate at its own version, so the old bytes are agreed only as THAT version and an
/// edit of them is sent on it.
#[test]
fn content_seq_restore_clears_the_agreed_hash_n7() {
    let (_dir, _path, db) = seeded();
    db.record_local_baseline(FILE, 1, "h1").unwrap();
    let before = seq(&db);
    db.apply_restore_response(FILE, Some(2), Some("object-v2")).unwrap();
    assert_eq!(agreed(&db), (2, None), "no agreed content at the restored version");
    assert_eq!(
        db.fetched_content(FILE).unwrap(),
        Some((1, Some("h1".into()))),
        "the bytes on disk are still version 1's"
    );
    assert!(seq(&db) > before);
    // A reply that names the version the row already agrees on changes no content.
    db.record_local_baseline(FILE, 2, "h2").unwrap();
    db.apply_restore_response(FILE, Some(2), None).unwrap();
    assert_eq!(agreed(&db), (2, Some("h2".into())));
}

/// The wipe: nothing recorded since the dehydrate began, so the candidate (or baseline) is emptied, and the next
/// range fetch names ONE version instead of mixing in one whose bytes are gone.
#[test]
fn range_wipe_after_a_dehydrate_lets_the_next_fetch_name_one_version() {
    let (_dir, _path, db) = seeded();
    db.mark_no_fetched_content(FILE).unwrap();
    db.record_fetched_version(FILE, 3, "h3").unwrap();
    let before = seq(&db);
    // (CfDehydratePlaceholder here.)
    assert!(db.wipe_fetched_content_if_unchanged(FILE, before).unwrap());
    assert_eq!(db.fetched_content(FILE).unwrap(), None, "nothing fetched on disk");
    db.record_fetched_version(FILE, 4, "h4").unwrap();
    assert_eq!(
        db.fetched_content(FILE).unwrap(),
        Some((4, Some("h4".into()))),
        "one version, not mixed at 3"
    );
}

/// A baseline is wiped too (Free up space frees agreed files), and the cloud-only fold after it then has nothing to
/// fold in.
#[test]
fn range_wipe_empties_a_baseline_before_the_cloud_only_fold() {
    let (_dir, _path, db) = seeded();
    db.record_local_baseline(FILE, 2, "h2").unwrap();
    let before = seq(&db);
    assert!(db.wipe_fetched_content_if_unchanged(FILE, before).unwrap());
    db.mark_cloud_only_after_dehydrate(&[FILE.to_string()], 5).unwrap();
    assert_eq!(agreed(&db).1, None);
    assert_eq!(db.fetched_content(FILE).unwrap(), None, "nothing folded in");
    db.record_fetched_version(FILE, 3, "h3").unwrap();
    assert_eq!(db.fetched_content(FILE).unwrap(), Some((3, Some("h3".into()))));
}

/// A fetch recorded after the dehydrate began (an opener broke the oplock) keeps its record: its bytes may be
/// landing. Also when it recorded the very version the candidate already had.
#[test]
fn range_wipe_is_refused_after_a_fetch_recorded_since() {
    let (_dir, _path, db) = seeded();
    db.mark_no_fetched_content(FILE).unwrap();
    db.record_fetched_version(FILE, 3, "h3").unwrap();
    let before = seq(&db);
    db.record_fetched_version(FILE, 3, "h3").unwrap();
    assert!(!db.wipe_fetched_content_if_unchanged(FILE, before).unwrap());
    assert_eq!(db.fetched_content(FILE).unwrap(), Some((3, Some("h3".into()))));
    let before = seq(&db);
    db.record_fetch_candidate(FILE, 3).unwrap();
    assert!(!db.wipe_fetched_content_if_unchanged(FILE, before).unwrap());
    assert_eq!(db.fetched_content(FILE).unwrap(), Some((3, None)), "kept, mixed");
}

/// A file whose content is not known (`-1`, `-2`) is left as it is: task 2191 decides those.
#[test]
fn range_wipe_leaves_unknown_content_alone() {
    let (_dir, _path, db) = seeded();
    let before = seq(&db);
    assert!(!db.wipe_fetched_content_if_unchanged(FILE, before).unwrap(), "-1");
    db.mark_content_unknown(FILE).unwrap();
    let before = seq(&db);
    assert!(!db.wipe_fetched_content_if_unchanged(FILE, before).unwrap(), "-2");
    assert_eq!(db.list_guarded().unwrap().len(), 1, "still guarded");
}
