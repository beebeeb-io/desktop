//! The upgrade from a released build: a state database that desktop 0.8.25 itself created (its schema,
//! its bundled SQLite 3.46.0 under rusqlite 0.32, its core) opens under this build's rusqlite 0.41, is
//! migrated, and still holds its rows; the file that release sealed is still read by this core.
//!
//! The fixture (`tests/fixtures/state-db-0.8.25/`) was written by the code at tag `desktop-v0.8.25`
//! with invented rows only: `closed/` is the database after a clean close, `wal/` is the image a crash
//! leaves (the write-ahead log not checkpointed). `fixture.json` names what was put in.

use super::*;
use crate::content_record::ContentRecord;

const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/state-db-0.8.25");

fn fixture_facts() -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(Path::new(FIXTURE).join("fixture.json")).unwrap()).unwrap()
}

fn unhex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&text[at..at + 2], 16).unwrap())
        .collect()
}

/// A private copy of one fixture image; a test never opens the committed files themselves.
fn copy_of(image: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let mut copied = 0;
    for entry in std::fs::read_dir(Path::new(FIXTURE).join(image)).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(entry.path(), dir.path().join(entry.file_name())).unwrap();
        copied += 1;
    }
    assert!(copied >= 1, "the {image} image has files");
    let path = dir.path().join("state.db");
    (dir, path)
}

/// `SQLITE_VERSION_NUMBER` of the library that last wrote the file (header bytes 96..100).
fn written_by_sqlite(path: &Path) -> u32 {
    let header = std::fs::read(path).unwrap();
    u32::from_be_bytes(header[96..100].try_into().unwrap())
}

/// Both images open, migrate and keep every row the release wrote. The premise is checked first: the
/// file on disk was written by SQLite 3.46.0, and this build links a newer one.
/// Mutant: a migration that rebuilds `upload_resume` or `owned_key_records` (the rows are gone).
#[test]
fn a_state_database_of_0_8_25_opens_and_keeps_its_rows() {
    let facts = fixture_facts();
    let id = facts["file_id"].as_str().unwrap();
    let folder = facts["folder_id"].as_str().unwrap();
    assert_eq!(facts["sqlite_version"], "3.46.0");

    for (image, wal_files) in [("closed", 0), ("wal", 1)] {
        let (dir, path) = copy_of(image);
        assert_eq!(
            written_by_sqlite(&path),
            3_046_000,
            "{image}: the premise, a file last written by SQLite 3.46.0"
        );
        assert_eq!(
            usize::from(dir.path().join("state.db-wal").exists()),
            wal_files,
            "{image}: the image is the one its name says"
        );

        let db = StateDb::open(&path).unwrap_or_else(|e| panic!("{image}: {e:#}"));

        let file = db.get_file(id).unwrap().expect("the release's file row");
        assert_eq!(file.path, "Documents/fixture.txt", "{image}");
        assert_eq!(file.size_bytes, 41, "{image}");
        assert_eq!(file.status, FileStatus::CloudOnly, "{image}");
        assert_eq!(file.item_kind, ItemKind::File, "{image}");
        let parent = db.get_file(folder).unwrap().expect("the release's folder row");
        assert_eq!(
            (parent.path.as_str(), parent.item_kind),
            ("Documents", ItemKind::Folder)
        );

        let record = db.get_owned_key_record(id).unwrap().expect("the release's key record");
        assert_eq!(
            record.name_encrypted,
            facts["name_encrypted"].as_str().unwrap(),
            "{image}"
        );

        // The session the release left behind, exactly as it left it, and with no completion name:
        // the new column is NULL on an old row.
        let resume = db
            .get_upload_resume("fixture-op")
            .unwrap()
            .expect("the release's session");
        assert_eq!(resume.upload_session_id, "fixture-session");
        assert_eq!((resume.acked_chunks, resume.chunk_count), (2, 3), "{image}");
        assert!(resume.self_minted && !resume.is_create && resume.metadata_applied);
        assert_eq!(db.upload_completion_name("fixture-op").unwrap(), None, "{image}");
        // The resume rule on that row: some of its chunks are acknowledged, as v1 frames. A v2
        // writer never continues it; the upload starts again in a fresh session.
        assert_eq!(
            upload_v2::resume_step(true, resume.acked_chunks, resume.chunk_count),
            upload_v2::ResumeStep::Restart
        );

        // The tables this build adds exist and take a write; the file is sound afterwards.
        assert_eq!(db.highest_file_version(id).unwrap(), None);
        db.observe_file_version(id, 1, Some("sealed")).unwrap();
        assert_eq!(db.highest_file_version(id).unwrap(), Some(1));
        drop(db);

        let raw = rusqlite::Connection::open(&path).unwrap();
        let marker: String = raw
            .query_row("SELECT value FROM sync_state WHERE key = 'fixture_marker'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(marker, "written by desktop-v0.8.25", "{image}");
        let (files, sessions, keys): (i64, i64, i64) = raw
            .query_row(
                "SELECT (SELECT COUNT(*) FROM files), (SELECT COUNT(*) FROM upload_resume), \
                 (SELECT COUNT(*) FROM owned_key_records)",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            (files, sessions, keys),
            (2, 1, 1),
            "{image}: 2 files, 1 session, 1 key record"
        );
        let sound: String = raw.query_row("PRAGMA integrity_check", [], |row| row.get(0)).unwrap();
        assert_eq!(sound, "ok", "{image}");
        // This build's SQLite is the newer one (rusqlite 0.41 bundles 3.5x): the open above was an
        // upgrade, not the same library reading its own file.
        let linked: i64 = raw
            .query_row("SELECT sqlite_version()", [], |row| row.get::<_, String>(0))
            .map(|text| {
                let mut parts = text.split('.').map(|part| part.parse::<i64>().unwrap());
                parts.next().unwrap() * 1_000_000 + parts.next().unwrap() * 1_000 + parts.next().unwrap()
            })
            .unwrap();
        assert!(linked > 3_046_000, "{image}: linked SQLite {linked}");
        drop(raw);
        assert!(
            written_by_sqlite(&path) > 3_046_000,
            "{image}: this build wrote the file"
        );
    }
}

/// What 0.8.25 sealed, this core still opens: the name under the file key (it records no format and
/// no commitment) and the three v1 frames, whole and seeking. The bytes are the release's own output,
/// not a v1 file this build made for itself.
/// Mutant: a reader that insists on a commitment (every existing file would be refused).
#[test]
fn the_file_0_8_25_sealed_is_still_read() {
    let facts = fixture_facts();
    let id = facts["file_id"].as_str().unwrap();
    let master: [u8; 32] = unhex(facts["master_key_hex"].as_str().unwrap()).try_into().unwrap();
    let key = || hydration_test_key(master, id);
    let plain = unhex(facts["plaintext_hex"].as_str().unwrap());
    let frames: Vec<Vec<u8>> = facts["frames_hex"]
        .as_array()
        .unwrap()
        .iter()
        .map(|frame| unhex(frame.as_str().unwrap()))
        .collect();
    assert_eq!((plain.len(), frames.len()), (41, 3));

    let opened =
        beebeeb_core::encrypt::decrypt_file_metadata_with_key(&key(), facts["name_encrypted"].as_str().unwrap())
            .unwrap();
    assert_eq!(opened.name, "fixture.txt");
    assert_eq!(opened.mime_type.as_deref(), Some("text/plain"));
    let record = ContentRecord::of(&opened);
    assert_eq!(record, ContentRecord::UNRECORDED);

    let mut reader = record.decryptor(key(), id);
    let mut whole = Vec::new();
    for frame in &frames {
        whole.extend_from_slice(&reader.push_frame(frame).unwrap().data);
    }
    reader.finish().unwrap();
    assert_eq!(whole, plain);
    let last = record.open_chunk(&key(), id, 2, true, &frames[2]).unwrap();
    assert_eq!(last, &plain[32..]);

    // A rename by this build keeps the file a v1 file: still no commitment, still read.
    let renamed = record
        .seal_name(&key(), "renamed.txt", opened.mime_type.as_deref())
        .unwrap();
    let after = beebeeb_core::encrypt::decrypt_file_metadata_with_key(&key(), &renamed).unwrap();
    assert_eq!(ContentRecord::of(&after), ContentRecord::UNRECORDED);
}
