//! dc-168: `files.has_thumbnail`, the server's thumbnail flag of a row, per version.

use super::*;

const FILE: &str = "9b1e2c64-1d6a-4a8e-9f7e-0c6f1d2a3b4c";

fn seeded() -> (tempfile::TempDir, std::path::PathBuf, StateDb) {
    let dir = tempdir().unwrap();
    let path = dir.path().join("state.db");
    let db = StateDb::open(&path).unwrap();
    db.upsert_file(&FileEntry {
        file_id: FILE.into(),
        path: "photo.jpg".into(),
        status: FileStatus::CloudOnly,
        size_bytes: 1,
        modified_at: 1,
        content_hash: None,
        remote_updated_at: 1,
        parent_id: None,
        item_kind: ItemKind::File,
    })
    .unwrap();
    set_version(&path, 3);
    (dir, path, db)
}

fn set_version(path: &std::path::Path, version: i64) {
    Connection::open(path)
        .unwrap()
        .execute(
            "UPDATE files SET current_version = ?2 WHERE file_id = ?1",
            params![FILE, version],
        )
        .unwrap();
}

/// A 0.8.25-era database has neither column (`content_seq` arrived after it, so its shape is today's without
/// those columns too). Both are added NULL on every row, the row kept, the flag not known; opening again changes
/// nothing.
#[test]
fn has_thumbnail_is_added_to_a_0_8_25_database_once() {
    let (_dir, path, db) = seeded();
    drop(db);
    {
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "DROP TRIGGER files_content_seq_follows_content;
             ALTER TABLE files DROP COLUMN content_seq;
             ALTER TABLE files DROP COLUMN has_thumbnail;
             ALTER TABLE files DROP COLUMN has_thumbnail_version;",
        )
        .unwrap();
    }
    for _ in 0..3 {
        let db = StateDb::open(&path).unwrap();
        assert!(db.get_file(FILE).unwrap().is_some(), "the row kept");
        assert_eq!(
            db.server_has_thumbnail(FILE, 3).unwrap(),
            None,
            "not known on an older row"
        );
    }
    let db = StateDb::open(&path).unwrap();
    assert!(db.set_server_has_thumbnail(FILE, true).unwrap());
    assert_eq!(db.server_has_thumbnail(FILE, 3).unwrap(), Some(true));
}

/// The flag holds for the version it was stored at only: a new version makes it not known again, and asking for
/// another version than the row's is not known either.
#[test]
fn has_thumbnail_holds_for_its_version_only() {
    let (_dir, path, db) = seeded();
    assert_eq!(db.server_has_thumbnail(FILE, 3).unwrap(), None);
    assert!(db.set_server_has_thumbnail(FILE, true).unwrap());
    assert_eq!(db.server_has_thumbnail(FILE, 3).unwrap(), Some(true));
    assert_eq!(db.server_has_thumbnail(FILE, 2).unwrap(), None, "another version");
    assert!(
        db.set_server_has_thumbnail(FILE, true).unwrap(),
        "unchanged, the row is there"
    );
    set_version(&path, 4);
    assert_eq!(db.server_has_thumbnail(FILE, 4).unwrap(), None, "a new version");
    assert_eq!(
        db.server_has_thumbnail(FILE, 3).unwrap(),
        None,
        "the old one is no longer the row's"
    );
    assert!(db.set_server_has_thumbnail(FILE, false).unwrap());
    assert_eq!(db.server_has_thumbnail(FILE, 4).unwrap(), Some(false));
    assert!(!db.set_server_has_thumbnail("unknown", true).unwrap(), "an unknown id");
}
