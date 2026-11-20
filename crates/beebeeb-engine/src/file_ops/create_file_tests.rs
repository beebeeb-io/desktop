//! New file on a sync root (DW-1522): the 0.8 desktop's test of it, and one test per refusal it had no test for (an
//! 8.3 short name, a full disk, the size limit, an entry that appeared after the checks).

use std::io;
use std::path::{Path, PathBuf};

use super::{CREATE_FILE_LIMIT, create_file, create_file_with, write_and_sync, write_new};
use crate::drive_v2::DriveError;
use crate::file_ops::resolve;
use crate::name_alias;
use crate::state_db::{self, FileEntry, FileStatus, ItemKind, Namespace, StateDb};

struct Fixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
    db: StateDb,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("Beebeeb");
    std::fs::create_dir(&root).unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    Fixture { _dir: dir, root, db }
}

fn row(db: &StateDb, id: &str, path: &str, parent: Option<&str>, folder: bool, status: FileStatus) {
    row_in(
        db,
        id,
        path,
        parent,
        folder,
        status,
        Namespace::MyFiles,
        state_db::PERMISSION_OWNER,
    );
}

#[allow(clippy::too_many_arguments)]
fn row_in(
    db: &StateDb,
    id: &str,
    path: &str,
    parent: Option<&str>,
    folder: bool,
    status: FileStatus,
    namespace: Namespace,
    bits: i64,
) {
    let kind = || if folder { ItemKind::Folder } else { ItemKind::File };
    let entry = FileEntry {
        file_id: id.to_string(),
        path: path.to_string(),
        status,
        size_bytes: if folder { 0 } else { 10 },
        modified_at: 100,
        content_hash: None,
        remote_updated_at: 100,
        parent_id: parent.map(str::to_string),
        item_kind: kind(),
    };
    db.upsert_file_with_contract(&entry, |c| {
        c.parent_id = parent.map(str::to_string);
        c.item_kind = kind();
        c.namespace = namespace;
        c.permission_bits = bits;
        c.current_version = 3;
    })
    .unwrap();
}

/// The 0.8 desktop's drive: Docs (a cloud-only `Report.pdf`, a local `notes.txt`), a file at the top, and a folder
/// shared with this person that they may only read.
fn drive(f: &Fixture) {
    let db = &f.db;
    row(db, "d0c5", "Docs", None, true, FileStatus::Local);
    row(db, "a1", "Docs/Report.pdf", Some("d0c5"), false, FileStatus::CloudOnly);
    row(db, "a2", "Docs/notes.txt", Some("d0c5"), false, FileStatus::Local);
    row(db, "b1", "zebra.txt", None, false, FileStatus::Local);
    row_in(
        db,
        "s1",
        "Shared/Plans",
        None,
        true,
        FileStatus::CloudOnly,
        Namespace::SharedWithMe,
        state_db::PERMISSION_READ,
    );
    std::fs::create_dir(f.root.join("Docs")).unwrap();
    std::fs::write(f.root.join("Docs").join("notes.txt"), "hello").unwrap();
    std::fs::write(f.root.join("zebra.txt"), "z").unwrap();
}

fn in_docs(f: &Fixture, name: &str) -> PathBuf {
    f.root.join("Docs").join(name)
}

fn names_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// Ported as is: the file is made with its bytes, in a folder and at the top; a taken name (on disk, or a cloud-only
/// name in any case) is refused and the file there keeps its bytes; a bad name and a read-only share refuse.
#[test]
fn drive_v2_create_file_on_the_sync_root() {
    let f = fixture();
    drive(&f);
    let docs = resolve(&f.db, "d0c5").unwrap();
    assert_eq!(
        create_file(&f.db, &f.root, Some(&docs), "Notes.md", b"# Notes\n").unwrap(),
        "Docs/Notes.md"
    );
    assert_eq!(std::fs::read(in_docs(&f, "Notes.md")).unwrap(), b"# Notes\n");
    assert_eq!(
        create_file(&f.db, &f.root, None, "Untitled.txt", b"").unwrap(),
        "Untitled.txt",
        "an empty file in the root"
    );
    // A name already on disk is refused, and the file there keeps its bytes (never truncated).
    std::fs::write(in_docs(&f, "keep.txt"), b"old").unwrap();
    assert_eq!(
        create_file(&f.db, &f.root, Some(&docs), "KEEP.txt", b"new").unwrap_err(),
        DriveError::NameTaken
    );
    assert_eq!(std::fs::read(in_docs(&f, "keep.txt")).unwrap(), b"old");
    assert_eq!(
        create_file(&f.db, &f.root, Some(&docs), "report.PDF", b"x").unwrap_err(),
        DriveError::NameTaken,
        "a cloud-only name counts, any case"
    );
    for bad in ["", "a/b", "..", "con.txt", "x?.md", "trailing."] {
        assert_eq!(
            create_file(&f.db, &f.root, None, bad, b"").unwrap_err(),
            DriveError::InvalidName,
            "{bad:?}"
        );
    }
    let shared = resolve(&f.db, "s1").unwrap();
    assert_eq!(
        create_file(&f.db, &f.root, Some(&shared), "x.md", b"").unwrap_err(),
        DriveError::ReadOnly
    );
}

/// Task 2407: a name that reaches a different entry on Windows (an 8.3 short name such as `REPORT~1.PDF`) is a taken
/// name, and a name that could not be checked is refused. Neither makes a file. The stubbed names exist nowhere,
/// on disk or in the mirror, so only the short-name check can refuse them.
#[test]
fn create_file_refuses_an_8_3_short_name_and_a_name_it_could_not_check() {
    let f = fixture();
    drive(&f);
    let docs = resolve(&f.db, "d0c5").unwrap();
    let short = in_docs(&f, "REPORT~1.PDF");
    let _alias = name_alias::stub::alias(&short);
    assert_eq!(
        create_file(&f.db, &f.root, Some(&docs), "REPORT~1.PDF", b"new").unwrap_err(),
        DriveError::NameTaken
    );
    let unread = in_docs(&f, "plans.md");
    let _failing = name_alias::stub::failing(&unread);
    assert_eq!(
        create_file(&f.db, &f.root, Some(&docs), "plans.md", b"").unwrap_err(),
        DriveError::Failed("name check")
    );
    assert!(!short.exists() && !unread.exists(), "nothing was made");
    assert_eq!(names_in(&f.root.join("Docs")), ["notes.txt"]);
}

/// A disk that fills part way through the write: the half file is removed (it would upload as the file, and would
/// hold the name), the answer is the disk's, and the same name can be made right after.
#[test]
fn create_file_on_a_full_disk_leaves_no_half_file() {
    let f = fixture();
    drive(&f);
    let docs = resolve(&f.db, "d0c5").unwrap();
    let bytes = b"# Plans for the spring\n".as_slice();
    let full = |file: &mut std::fs::File, bytes: &[u8]| -> io::Result<()> {
        io::Write::write_all(file, &bytes[..bytes.len() / 2])?;
        Err(io::Error::from(io::ErrorKind::StorageFull))
    };
    assert_eq!(
        create_file_with(&f.db, &f.root, Some(&docs), "Plans.md", bytes, CREATE_FILE_LIMIT, full).unwrap_err(),
        DriveError::Quota
    );
    assert!(!in_docs(&f, "Plans.md").exists(), "the half file was removed");
    assert_eq!(
        create_file(&f.db, &f.root, Some(&docs), "Plans.md", bytes).unwrap(),
        "Docs/Plans.md",
        "the name is free for the retry"
    );
    assert_eq!(std::fs::read(in_docs(&f, "Plans.md")).unwrap(), bytes);
}

/// More bytes than the limit: refused before anything is made. At the limit: made. The limit is the 0.8 desktop's.
#[test]
fn create_file_over_its_limit_is_too_large() {
    assert_eq!(CREATE_FILE_LIMIT, 256 * 1024 * 1024);
    let f = fixture();
    drive(&f);
    assert_eq!(
        create_file_with(&f.db, &f.root, None, "big.txt", b"12345", 4, write_and_sync).unwrap_err(),
        DriveError::TooLarge
    );
    assert!(!f.root.join("big.txt").exists(), "nothing was made");
    assert_eq!(
        create_file_with(&f.db, &f.root, None, "big.txt", b"1234", 4, write_and_sync).unwrap(),
        "big.txt"
    );
    assert_eq!(std::fs::read(f.root.join("big.txt")).unwrap(), b"1234");
}

/// An entry that appeared after the name checks (another app, File Explorer) is a taken name: the file is opened
/// `create_new`, so what is there keeps its bytes.
#[test]
fn create_file_never_truncates_an_entry_that_appeared_after_the_checks() {
    let f = fixture();
    let there = f.root.join("raced.txt");
    std::fs::write(&there, b"theirs").unwrap();
    assert_eq!(
        write_new(&there, b"ours", write_and_sync).unwrap_err(),
        DriveError::NameTaken
    );
    assert_eq!(std::fs::read(&there).unwrap(), b"theirs");
}

/// The bytes are written under a name the watcher ignores, never at the final name: while the write runs the final
/// name does not exist, and the name being written is one the scan skips.
#[test]
fn create_file_writes_under_a_name_the_watcher_ignores() {
    let f = fixture();
    let seen = std::cell::RefCell::new(Vec::new());
    let look = |file: &mut std::fs::File, bytes: &[u8]| -> io::Result<()> {
        seen.borrow_mut().extend(names_in(&f.root));
        io::Write::write_all(file, bytes)
    };
    create_file_with(&f.db, &f.root, None, "Plans.md", b"hello", CREATE_FILE_LIMIT, look).unwrap();
    let seen = seen.into_inner();
    assert_eq!(seen.len(), 1, "one entry during the write: {seen:?}");
    assert!(
        crate::engine_bridge::is_ignored_finder_name(&seen[0]),
        "the name being written is ignored by the watcher: {seen:?}"
    );
    assert_eq!(names_in(&f.root), vec!["Plans.md".to_string()], "only the file stays");
    assert_eq!(std::fs::read(f.root.join("Plans.md")).unwrap(), b"hello");
}

/// A write that fails half way leaves nothing: not at the final name, and not under the partial name either.
#[test]
fn create_file_that_fails_half_way_leaves_nothing_for_the_watcher() {
    let f = fixture();
    let full = |file: &mut std::fs::File, bytes: &[u8]| -> io::Result<()> {
        io::Write::write_all(file, &bytes[..bytes.len() / 2])?;
        Err(io::Error::from(io::ErrorKind::StorageFull))
    };
    assert_eq!(
        create_file_with(
            &f.db,
            &f.root,
            None,
            "Plans.md",
            b"half a file",
            CREATE_FILE_LIMIT,
            full
        )
        .unwrap_err(),
        DriveError::Quota
    );
    assert_eq!(names_in(&f.root), Vec::<String>::new(), "no entry of any name is left");
}

/// A name taken after the checks is refused and keeps its bytes, and the partial file goes.
#[test]
fn create_file_does_not_replace_a_name_taken_after_the_checks() {
    let f = fixture();
    let theirs = f.root.join("raced.txt");
    let take = |file: &mut std::fs::File, bytes: &[u8]| -> io::Result<()> {
        io::Write::write_all(file, bytes)?;
        std::fs::write(&theirs, b"theirs")
    };
    assert_eq!(
        create_file_with(&f.db, &f.root, None, "raced.txt", b"ours", CREATE_FILE_LIMIT, take).unwrap_err(),
        DriveError::NameTaken
    );
    assert_eq!(std::fs::read(&theirs).unwrap(), b"theirs");
    assert_eq!(
        names_in(&f.root),
        vec!["raced.txt".to_string()],
        "the partial file was removed"
    );
}

/// w36 (Guus's PC, bff1af87): New file made a 0-byte file that no list showed until the folder scan found it, and a
/// second New file of that name was refused by the file on disk. The file is listed and queued by the call itself:
/// its row is in its folder's rows with no scan, and a second New file of the name is refused as a taken name.
/// Mutant: `list_new_file` returning `false` without the dispatch -> red (no row).
#[test]
fn new_file_is_listed_and_queued_at_once_without_a_scan() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("Beebeeb");
    std::fs::create_dir(&root).unwrap();
    let db = std::sync::Arc::new(StateDb::open(dir.path().join("state.db")).unwrap());
    let api = std::sync::Arc::new(crate::api_client::ApiClient::new(
        "https://api.beebeeb.io".into(),
        "token".into(),
        [17u8; 32],
    ));
    let bridge = crate::engine_bridge::EngineBridge::new(&crate::identity::TEST, db.clone(), api);

    let rel = create_file(&db, &root, None, "Beebeeb typing test.txt", &[]).unwrap();
    assert!(db.get_file_by_path(&rel).unwrap().is_none(), "made on disk only so far");
    assert!(super::list_new_file(&bridge, &root, &rel), "queued by the call");

    let (rows, _) = db.children_paged(None, 0, i64::MAX).unwrap();
    let listed: Vec<&str> = rows.iter().map(|(entry, _, _)| entry.path.as_str()).collect();
    assert_eq!(listed, ["Beebeeb typing test.txt"], "the folder's rows hold it, with no scan");
    assert_eq!(rows[0].0.status, FileStatus::Uploading);
    assert_eq!(rows[0].0.size_bytes, 0, "an empty file is a file");
    let queued = db.list_due_operations(i64::MAX).unwrap();
    assert_eq!(queued.len(), 1, "its upload is queued once: {queued:?}");
    assert_eq!(queued[0].target_path.as_deref(), Some("Beebeeb typing test.txt"));

    // The scan that comes later queues nothing more for it.
    crate::watcher::run_one_scan(&bridge, &root);
    assert_eq!(db.list_due_operations(i64::MAX).unwrap().len(), 1, "never queued twice");
    // A second New file of the name: a taken name, and the first file is as it was.
    assert_eq!(
        create_file(&db, &root, None, "Beebeeb typing test.txt", &[]),
        Err(DriveError::NameTaken)
    );
}
