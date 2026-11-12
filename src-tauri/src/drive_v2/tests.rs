//! Tests for the drive_v2 commands: one per command over the logic it runs (a temporary state DB
//! and sync root stand in for the engine's), plus the DTO no-secret guard (spec 2.1).

use super::DriveError;
use super::commands::{check_trashable, frame, remember_undo, save_copy_source, take_undo, unframe};
use super::dto::*;
use super::fsops;
use super::model::{self, Sort, SortDir, SortKey};
use crate::state_db::{self, FileEntry, FileStatus, ItemKind, Namespace, StateDb};

struct Fixture {
    _dir: tempfile::TempDir,
    root: std::path::PathBuf,
    db: StateDb,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("Beebeeb");
    std::fs::create_dir(&root).unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    Fixture { _dir: dir, root, db }
}

fn row(
    db: &StateDb,
    id: &str,
    path: &str,
    parent: Option<&str>,
    folder: bool,
    size: i64,
    modified: i64,
    status: FileStatus,
) {
    row_in(
        db,
        id,
        path,
        parent,
        folder,
        size,
        modified,
        status,
        Namespace::MyFiles,
        None,
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
    size: i64,
    modified: i64,
    status: FileStatus,
    namespace: Namespace,
    owner: Option<&str>,
    bits: i64,
) {
    let entry = FileEntry {
        file_id: id.to_string(),
        path: path.to_string(),
        status,
        size_bytes: size,
        modified_at: modified,
        content_hash: None,
        remote_updated_at: modified,
        parent_id: None,
        item_kind: if folder { ItemKind::Folder } else { ItemKind::File },
    };
    db.upsert_file_with_contract(&entry, |c| {
        c.parent_id = parent.map(str::to_string);
        c.item_kind = if folder { ItemKind::Folder } else { ItemKind::File };
        c.namespace = namespace;
        c.owner_email = owner.map(str::to_string);
        c.permission_bits = bits;
        c.current_version = 3;
    })
    .unwrap();
}

/// Files/, Files/Docs (2 children), a file at the root, a photo, one being trashed, a shared root.
fn drive(f: &Fixture) {
    let db = &f.db;
    row(db, "d0c5", "Docs", None, true, 0, 100, FileStatus::Local);
    row(
        db,
        "a1",
        "Docs/Report.pdf",
        Some("d0c5"),
        false,
        2_000,
        300,
        FileStatus::CloudOnly,
    );
    row(
        db,
        "a2",
        "Docs/notes.txt",
        Some("d0c5"),
        false,
        10,
        200,
        FileStatus::Local,
    );
    row(db, "b1", "zebra.txt", None, false, 50, 400, FileStatus::Local);
    row(db, "b2", "Beach.jpg", None, false, 9_000, 500, FileStatus::Downloading);
    row(db, "gone", "old.txt", None, false, 1, 600, FileStatus::Trashing);
    row_in(
        db,
        "s1",
        "Shared/Plans",
        None,
        true,
        0,
        50,
        FileStatus::CloudOnly,
        Namespace::SharedWithMe,
        Some("ana.lima@example.eu"),
        state_db::PERMISSION_READ,
    );
    std::fs::create_dir(f.root.join("Docs")).unwrap();
    std::fs::write(f.root.join("Docs").join("notes.txt"), "hello").unwrap();
    std::fs::write(f.root.join("zebra.txt"), "z").unwrap();
}

fn names(items: &[DriveItem]) -> Vec<&str> {
    items.iter().map(|i| i.name.as_str()).collect()
}

#[test]
fn drive_v2_list_shows_one_folder_from_the_mirror() {
    let f = fixture();
    drive(&f);
    let root = model::list(&f.db, None, Sort::default(), false).unwrap();
    assert_eq!(
        names(&root.items),
        ["Docs", "Beach.jpg", "zebra.txt"],
        "folders first, the trashing row and the shared root left out"
    );
    assert_eq!(root.path.len(), 1);
    assert_eq!(root.folder.name, "Files");
    assert_eq!(root.items[0].child_count, Some(2));
    assert_eq!(root.items[1].local, Some(LocalState::Syncing));
    assert_eq!(root.items[1].mime.as_deref(), Some("image/jpeg"));

    let docs = model::list(
        &f.db,
        Some("d0c5"),
        Sort {
            key: SortKey::Size,
            dir: SortDir::Desc,
        },
        true,
    )
    .unwrap();
    assert_eq!(names(&docs.items), ["Report.pdf", "notes.txt"]);
    assert_eq!(docs.items[0].local, Some(LocalState::OnlineOnly));
    assert_eq!(
        docs.path.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
        ["Files", "Docs"]
    );
    assert!(docs.stale);
    assert_eq!(
        model::list(&f.db, Some("a1"), Sort::default(), false).unwrap_err(),
        DriveError::NotFound,
        "a file is not a folder"
    );
}

#[test]
fn drive_v2_details_names_the_path_and_versions() {
    let f = fixture();
    drive(&f);
    let d = model::details(&f.db, "a1").unwrap();
    assert_eq!(d.item.name, "Report.pdf");
    assert_eq!(d.path.len(), 2);
    assert_eq!(d.version_count, 3);
    assert_eq!(model::details(&f.db, "d0c5").unwrap().version_count, 0);
    assert_eq!(model::details(&f.db, "gone").unwrap_err(), DriveError::NotFound);
}

#[test]
fn drive_v2_thumbnail_resolves_ids_and_never_a_trashing_row() {
    let f = fixture();
    drive(&f);
    assert_eq!(model::resolve(&f.db, "b2").unwrap().file_id, "b2");
    assert_eq!(
        model::resolve(&f.db, "local:Docs/notes.txt").unwrap().file_id,
        "a2",
        "a provisional id resolves by path"
    );
    assert_eq!(model::resolve(&f.db, "gone").unwrap_err(), DriveError::NotFound);
    assert!(
        model::list(&f.db, None, Sort::default(), false)
            .unwrap()
            .items
            .iter()
            .filter(|i| i.has_thumbnail)
            .count()
            == 1
    );
}

#[test]
fn drive_v2_versions_marks_the_current_one() {
    let body = serde_json::json!({
        "current_version": 3,
        "versions": [
            { "id": "v3", "version_number": 3, "size_bytes": 9000, "created_at": "2026-07-25T12:03:00Z", "storage_pool_id": "pool-1" },
            { "id": "v2", "version_number": 2, "size_bytes": 7000, "created_at": "2026-07-25T12:02:00Z" }
        ]
    });
    let v = model::versions_from_payload(&body).unwrap();
    assert_eq!(v.len(), 2);
    assert!(v[0].current && !v[1].current);
    assert_eq!(v[1].size, 7000);
    assert!(model::versions_from_payload(&serde_json::json!({})).is_err());
}

#[test]
fn drive_v2_search_matches_names_in_the_local_index() {
    let f = fixture();
    drive(&f);
    let hits = model::search(&f.db, "report").unwrap();
    assert_eq!(names(&hits), ["Report.pdf"]);
    assert!(model::search(&f.db, "  ").unwrap().is_empty());
}

#[test]
fn drive_v2_recent_is_newest_files_first() {
    let f = fixture();
    drive(&f);
    let recent = model::recent(&f.db, 3).unwrap();
    assert_eq!(
        names(&recent),
        ["Beach.jpg", "zebra.txt", "Report.pdf"],
        "folders and the trashing row left out"
    );
}

#[test]
fn drive_v2_shared_lists_with_me_and_by_me() {
    let f = fixture();
    drive(&f);
    let with_me = model::shared_with_me(&f.db).unwrap();
    assert_eq!(with_me.len(), 1);
    assert_eq!(with_me[0].item.name, "Plans");
    assert_eq!(with_me[0].item.read_only, Some(true));
    assert_eq!(with_me[0].people[0].role, ShareRole::Viewer);

    let sent = serde_json::json!({ "invites": [
        { "file_id": "d0c5", "recipient_email": "bram@example.eu", "status": "approved", "permission": "write", "file_name_encrypted": "Y2lwaGVy" },
        { "file_id": "d0c5", "recipient_email": "noor@example.eu", "status": "pending", "permission": "read" },
        { "file_id": "a1", "recipient_email": "x@example.eu", "status": "revoked", "permission": "read" },
        { "file_id": "not-in-mirror", "recipient_email": "y@example.eu", "status": "approved", "permission": "read" }
    ]});
    let by_me = model::shared_by_me(&f.db, &sent).unwrap();
    assert_eq!(by_me.len(), 1, "revoked and unknown items left out");
    assert_eq!(
        by_me[0].item.name, "Docs",
        "the name comes from the mirror, never the invite's ciphertext"
    );
    assert_eq!(by_me[0].people.len(), 2);
    assert_eq!(by_me[0].people[0].role, ShareRole::Editor);
}

#[test]
fn drive_v2_photos_pages_images_and_videos() {
    let f = fixture();
    drive(&f);
    for i in 0..(model::PHOTOS_PAGE + 1) {
        row(
            &f.db,
            &format!("p{i}"),
            &format!("Camera/IMG_{i}.HEIC"),
            None,
            false,
            1,
            1_000 + i as i64,
            FileStatus::CloudOnly,
        );
    }
    let first = model::photos(&f.db, None).unwrap();
    assert_eq!(first.entries.len(), model::PHOTOS_PAGE);
    assert_eq!(
        first.entries[0].item.name,
        format!("IMG_{}.HEIC", model::PHOTOS_PAGE),
        "newest first"
    );
    let second = model::photos(&f.db, first.next.as_deref()).unwrap();
    assert_eq!(second.entries.len(), 2, "the last HEIC and Beach.jpg");
    assert!(second.next.is_none());
}

#[test]
fn drive_v2_trash_says_where_each_item_was() {
    let f = fixture();
    drive(&f);
    let raw = vec![
        crate::account_dto::DesktopTrashItem {
            id: "t1".into(),
            name: "Draft.docx".into(),
            is_folder: false,
            size_bytes: 5,
            updated_at: "2026-11-01T10:00:00Z".into(),
            parent_id: Some("d0c5".into()),
        },
        crate::account_dto::DesktopTrashItem {
            id: "t2".into(),
            name: "Old".into(),
            is_folder: true,
            size_bytes: 0,
            updated_at: "2026-11-02T10:00:00Z".into(),
            parent_id: None,
        },
    ];
    let t = model::trash_entries(Some(&f.db), raw);
    assert_eq!(t.len(), 2);
    assert_eq!(t[0].original_path, "Files/Docs");
    assert_eq!(t[1].original_path, "Files");
    assert_eq!(t[1].item.size, None);
}

#[test]
fn drive_v2_storage_splits_files_and_photos() {
    let f = fixture();
    drive(&f);
    let (files, photos, backups) = model::storage_split(&f.db).unwrap();
    assert_eq!(
        (files, photos, backups),
        (2_000 + 10 + 50, 9_000, 0),
        "the trashing row is not counted"
    );
}

#[test]
fn drive_v2_sync_summary_phase() {
    let counts = |moving, conflicts| model::SyncCounts {
        on_device: 4,
        online_only: 4,
        moving,
        conflicts,
        on_hold: 1,
    };
    assert_eq!(
        model::sync_phase(true, "idle", false, &counts(0, 0)),
        SyncPhase::UpToDate
    );
    assert_eq!(
        model::sync_phase(true, "idle", false, &counts(2, 0)),
        SyncPhase::Syncing
    );
    assert_eq!(
        model::sync_phase(true, "running", false, &counts(0, 1)),
        SyncPhase::NeedsDecision
    );
    assert_eq!(model::sync_phase(true, "idle", true, &counts(0, 0)), SyncPhase::Paused);
    assert_eq!(
        model::sync_phase(true, "offline", false, &counts(0, 0)),
        SyncPhase::Offline
    );
    assert_eq!(model::sync_phase(true, "", false, &counts(0, 0)), SyncPhase::Checking);
    assert_eq!(
        model::sync_phase(false, "idle", false, &counts(0, 0)),
        SyncPhase::Stopped
    );
    let s = model::sync_summary(SyncPhase::UpToDate, &counts(2, 0), 3);
    assert_eq!(
        (s.on_device, s.available_offline, s.online_only, s.pending),
        (4, 3, 4, 3)
    );
}

#[test]
fn drive_v2_create_folder_on_the_sync_root() {
    let f = fixture();
    drive(&f);
    let docs = model::resolve(&f.db, "d0c5").unwrap();
    assert_eq!(
        fsops::create_folder(&f.db, &f.root, Some(&docs), "Invoices 2026").unwrap(),
        "Docs/Invoices 2026"
    );
    assert!(f.root.join("Docs").join("Invoices 2026").is_dir());
    assert_eq!(
        fsops::create_folder(&f.db, &f.root, Some(&docs), "REPORT.pdf").unwrap_err(),
        DriveError::NameTaken,
        "a cloud-only name counts, any case"
    );
    for bad in ["", "a/b", "..", "con", "x?", "trailing."] {
        assert_eq!(
            fsops::create_folder(&f.db, &f.root, None, bad).unwrap_err(),
            DriveError::InvalidName,
            "{bad:?}"
        );
    }
    let shared = model::resolve(&f.db, "s1").unwrap();
    assert_eq!(
        fsops::create_folder(&f.db, &f.root, Some(&shared), "x").unwrap_err(),
        DriveError::ReadOnly
    );
}

#[test]
fn drive_v2_create_file_on_the_sync_root() {
    let f = fixture();
    drive(&f);
    let docs = model::resolve(&f.db, "d0c5").unwrap();
    assert_eq!(
        fsops::create_file(&f.db, &f.root, Some(&docs), "Notes.md", b"# Notes\n").unwrap(),
        "Docs/Notes.md"
    );
    assert_eq!(
        std::fs::read(f.root.join("Docs").join("Notes.md")).unwrap(),
        b"# Notes\n"
    );
    assert_eq!(
        fsops::create_file(&f.db, &f.root, None, "Untitled.txt", b"").unwrap(),
        "Untitled.txt",
        "an empty file in the root"
    );
    // A name already on disk is refused, and the file there keeps its bytes (never truncated).
    std::fs::write(f.root.join("Docs").join("keep.txt"), b"old").unwrap();
    assert_eq!(
        fsops::create_file(&f.db, &f.root, Some(&docs), "KEEP.txt", b"new").unwrap_err(),
        DriveError::NameTaken
    );
    assert_eq!(std::fs::read(f.root.join("Docs").join("keep.txt")).unwrap(), b"old");
    assert_eq!(
        fsops::create_file(&f.db, &f.root, Some(&docs), "report.PDF", b"x").unwrap_err(),
        DriveError::NameTaken,
        "a cloud-only name counts, any case"
    );
    for bad in ["", "a/b", "..", "con.txt", "x?.md", "trailing."] {
        assert_eq!(
            fsops::create_file(&f.db, &f.root, None, bad, b"").unwrap_err(),
            DriveError::InvalidName,
            "{bad:?}"
        );
    }
    let shared = model::resolve(&f.db, "s1").unwrap();
    assert_eq!(
        fsops::create_file(&f.db, &f.root, Some(&shared), "x.md", b"").unwrap_err(),
        DriveError::ReadOnly
    );
}

#[test]
fn drive_v2_rename_on_the_sync_root() {
    let f = fixture();
    drive(&f);
    let notes = model::resolve(&f.db, "a2").unwrap();
    assert_eq!(
        fsops::rename(&f.db, &f.root, &notes, "Notes.txt").unwrap(),
        "Docs/Notes.txt",
        "a change of case is allowed"
    );
    assert!(f.root.join("Docs").join("Notes.txt").exists());
    let zebra = model::resolve(&f.db, "b1").unwrap();
    assert_eq!(
        fsops::rename(&f.db, &f.root, &zebra, "docs").unwrap_err(),
        DriveError::NameTaken
    );
}

#[test]
fn drive_v2_move_on_the_sync_root() {
    let f = fixture();
    drive(&f);
    let docs = model::resolve(&f.db, "d0c5").unwrap();
    let zebra = model::resolve(&f.db, "b1").unwrap();
    fsops::move_items(&f.db, &f.root, &[zebra], Some(&docs)).unwrap();
    assert!(f.root.join("Docs").join("zebra.txt").exists());
    assert!(!f.root.join("zebra.txt").exists());
    assert_eq!(
        fsops::move_items(&f.db, &f.root, std::slice::from_ref(&docs), Some(&docs)).unwrap_err(),
        DriveError::Conflict,
        "never into itself"
    );
}

#[test]
fn drive_v2_duplicate_names_the_copy() {
    let f = fixture();
    drive(&f);
    assert_eq!(fsops::copy_name("Report.pdf", 1), "Report - Copy.pdf");
    assert_eq!(fsops::copy_name("Report.pdf", 2), "Report - Copy (2).pdf");
    assert_eq!(fsops::copy_name("Docs", 1), "Docs - Copy");
    let zebra = model::resolve(&f.db, "b1").unwrap();
    assert_eq!(fsops::duplicate(&f.db, &f.root, &zebra).unwrap(), "zebra - Copy.txt");
    assert_eq!(std::fs::read_to_string(f.root.join("zebra - Copy.txt")).unwrap(), "z");
    let docs = model::resolve(&f.db, "d0c5").unwrap();
    assert_eq!(fsops::duplicate(&f.db, &f.root, &docs).unwrap(), "Docs - Copy");
    assert!(
        f.root.join("Docs - Copy").join("notes.txt").exists(),
        "a folder is copied with its contents"
    );
}

#[test]
fn drive_v2_trash_items_refuses_what_the_server_does_not_have() {
    let f = fixture();
    drive(&f);
    assert!(check_trashable(&f.db, &model::resolve(&f.db, "b1").unwrap()).is_ok());
    row(&f.db, "up", "draft.txt", None, false, 1, 700, FileStatus::Uploading);
    assert_eq!(
        check_trashable(&f.db, &model::resolve(&f.db, "up").unwrap()).unwrap_err(),
        DriveError::Conflict
    );
    assert_eq!(
        check_trashable(&f.db, &model::resolve(&f.db, "s1").unwrap()).unwrap_err(),
        DriveError::ReadOnly
    );
}

#[test]
fn drive_v2_undo_trash_token_works_once() {
    // Ids with letters a UUID never has: "a1" is in about one random UUID in nine, and failed CI that way.
    let ids = ["row-a1".to_string(), "row-b1".to_string()];
    let token = remember_undo(ids.to_vec());
    assert!(
        ids.iter().all(|id| !token.contains(id.as_str())),
        "the token names no file"
    );
    assert_eq!(take_undo(&token), Some(ids.to_vec()));
    assert_eq!(take_undo(&token), None);
}

#[test]
fn drive_v2_upload_copies_in_with_unique_names() {
    let f = fixture();
    drive(&f);
    let outside = tempfile::tempdir().unwrap();
    let a = outside.path().join("zebra.txt");
    std::fs::write(&a, "new").unwrap();
    let folder = outside.path().join("Album");
    std::fs::create_dir(&folder).unwrap();
    std::fs::write(folder.join("1.jpg"), "x").unwrap();
    let inside = f.root.join("zebra.txt");
    let started = fsops::copy_in(
        &f.db,
        &f.root,
        None,
        &[a, folder, inside, outside.path().join("missing")],
    )
    .unwrap();
    assert_eq!(
        started, 2,
        "a file inside the Beebeeb folder and a missing path are skipped"
    );
    assert_eq!(std::fs::read_to_string(f.root.join("zebra 2.txt")).unwrap(), "new");
    assert!(f.root.join("Album").join("1.jpg").exists());
}

#[test]
fn drive_v2_open_and_reveal_stay_inside_the_sync_root() {
    let f = fixture();
    assert!(fsops::full_path(&f.root, "Docs/notes.txt").is_ok());
    assert!(
        fsops::full_path(&f.root, "/etc/passwd").unwrap().starts_with(&f.root),
        "a state DB path's leading slash stays inside"
    );
    for bad in ["../outside.txt", "Docs/../../x", "C:\\x", "a\\..\\..\\x"] {
        assert!(fsops::full_path(&f.root, bad).is_err(), "{bad}");
    }
    assert_eq!(
        fsops::full_path(&f.root, "../outside.txt").unwrap_err(),
        DriveError::NotFound
    );
}

mod drive_save_copy {
    use super::*;

    #[test]
    fn copy_out_writes_the_bytes() {
        let f = fixture();
        drive(&f);
        let src = f.root.join("Docs").join("notes.txt");
        let out = tempfile::tempdir().unwrap();
        let dest = out.path().join("notes.txt");
        fsops::copy_out(&f.root, &src, &dest).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), std::fs::read(&src).unwrap());
        assert!(!out.path().join("notes.txt.bb-tmp").exists(), "no temp left behind");
    }

    #[test]
    fn copy_out_refuses_an_existing_file() {
        let f = fixture();
        drive(&f);
        let src = f.root.join("Docs").join("notes.txt");
        let out = tempfile::tempdir().unwrap();
        let dest = out.path().join("notes.txt");
        std::fs::write(&dest, "mine").unwrap();
        assert_eq!(fsops::copy_out(&f.root, &src, &dest).unwrap_err(), DriveError::Conflict);
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), "mine", "never an overwrite");
    }

    #[test]
    fn copy_out_leaves_no_temp_behind() {
        let f = fixture();
        drive(&f);
        let src = f.root.join("Docs").join("notes.txt");
        let out = tempfile::tempdir().unwrap();
        // A copy that succeeds leaves no temp either.
        let ok = out.path().join("ok.txt");
        fsops::copy_out(&f.root, &src, &ok).unwrap();
        assert!(!out.path().join("ok.txt.bb-tmp").exists(), "no temp left behind");
        // An unwritable destination directory: a file stands where the folder must be.
        let blocker = out.path().join("blocker");
        std::fs::write(&blocker, "file").unwrap();
        let blocked = blocker.join("notes.txt");
        assert!(fsops::copy_out(&f.root, &src, &blocked).is_err());
        assert!(!blocked.exists(), "no partial destination");
        // A failure after the temp exists (a folder source): the temp is removed again.
        let dest = out.path().join("Docs");
        assert!(fsops::copy_out(&f.root, &f.root.join("Docs"), &dest).is_err());
        assert!(!dest.exists(), "no partial destination");
        assert!(!out.path().join("Docs.bb-tmp").exists(), "no temp left behind");
    }

    #[test]
    fn copy_out_refuses_a_folder_entry() {
        let f = fixture();
        drive(&f);
        assert_eq!(
            save_copy_source(&f.db, &f.root, "d0c5").unwrap_err(),
            DriveError::NotFound,
            "a folder is never saved as a copy"
        );
        assert_eq!(
            save_copy_source(&f.db, &f.root, "missing").unwrap_err(),
            DriveError::NotFound
        );
    }

    #[test]
    fn copy_out_refuses_a_destination_inside_the_sync_root() {
        let f = fixture();
        drive(&f);
        let src = f.root.join("Docs").join("notes.txt");
        for dest in [f.root.join("copy.txt"), f.root.join("Docs").join("copy.txt")] {
            assert_eq!(
                fsops::copy_out(&f.root, &src, &dest).unwrap_err(),
                DriveError::Failed("inside the sync folder")
            );
            assert!(!dest.exists(), "nothing written");
        }
        assert!(!f.root.join("copy.txt.bb-tmp").exists());
    }
}

#[test]
fn drive_v2_text_save_refuses_a_stale_base() {
    let f = fixture();
    drive(&f);
    let notes = model::resolve(&f.db, "a2").unwrap();
    let path = f.root.join("Docs").join("notes.txt");
    let base = fsops::version_tag(3, &fsops::read_capped(&path, fsops::TEXT_LIMIT).unwrap());
    let next = fsops::save_checked(&f.db, &f.root, &notes, &base, b"hello again", fsops::TEXT_LIMIT).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello again");
    assert_ne!(next, base);
    // Someone else's edit lands; the editor's old base is now stale.
    std::fs::write(&path, "theirs").unwrap();
    assert_eq!(
        fsops::save_checked(&f.db, &f.root, &notes, &next, b"mine", fsops::TEXT_LIMIT).unwrap_err(),
        DriveError::Conflict
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "theirs", "never an overwrite");
    assert_eq!(fsops::read_capped(&path, 2).unwrap_err(), DriveError::TooLarge);
}

#[test]
fn drive_v2_bytes_frames_round_trip() {
    let framed = frame(&serde_json::json!({ "versionId": "v3-ab" }), &[0, 1, 2, 255]);
    let (header, bytes) = unframe(&framed).unwrap();
    assert_eq!(header["versionId"], "v3-ab");
    assert_eq!(bytes, [0, 1, 2, 255]);
    assert!(unframe(&[9, 0, 0, 0, b'{']).is_err());
    assert!(unframe(&[]).is_err());
}

#[test]
fn drive_v2_errors_carry_a_code_and_no_detail_from_outside() {
    assert_eq!(String::from(DriveError::NameTaken), "name-taken: drive");
    assert_eq!(DriveError::from_text("vault is locked"), DriveError::Locked);
    assert_eq!(
        DriveError::from_text("HTTP 404 for /api/v1/files/x"),
        DriveError::NotFound
    );
    assert_eq!(
        DriveError::from_text("error sending request for url"),
        DriveError::Offline
    );
    assert_eq!(DriveError::from_text("413 Payload Too Large"), DriveError::Quota);
    assert_eq!(
        String::from(DriveError::from_text("C:\\Users\\guus\\secret.txt broke")),
        "failed: request"
    );
}

/// Spec 2.1: no DTO carries a field named like a secret.
pub(super) fn assert_no_secret_fields(value: &serde_json::Value, at: &str) {
    const FORBIDDEN: [&str; 5] = ["key", "token", "url", "nonce", "ciphertext"];
    match value {
        serde_json::Value::Object(map) => {
            for (name, child) in map {
                let lower = name.to_ascii_lowercase();
                // `undoToken` is the one allowed name: an opaque handle into this process's undo table.
                let allowed = lower == "undotoken";
                assert!(
                    allowed || !(FORBIDDEN.iter().any(|f| lower.contains(f)) || lower.starts_with("wrapped")),
                    "{at}.{name} is a secret-shaped field"
                );
                assert_no_secret_fields(child, &format!("{at}.{name}"));
            }
        }
        serde_json::Value::Array(items) => items.iter().for_each(|i| assert_no_secret_fields(i, at)),
        _ => {}
    }
}

#[test]
fn drive_v2_no_dto_has_a_secret_shaped_field() {
    let f = fixture();
    drive(&f);
    let item = model::details(&f.db, "a1").unwrap().item;
    let shared = DriveItem {
        shared: SharedState::People {
            people: vec![Person {
                name: "Ana".into(),
                initials: "A".into(),
            }],
        },
        ..item.clone()
    };
    let sync = model::sync_summary(
        SyncPhase::UpToDate,
        &model::SyncCounts {
            on_device: 1,
            online_only: 1,
            moving: 0,
            conflicts: 0,
            on_hold: 0,
        },
        1,
    );
    let storage = StorageSummary {
        used_bytes: 1,
        quota_bytes: 2,
        files_bytes: 1,
        photos_bytes: 0,
        backups_bytes: 0,
    };
    let dtos: Vec<(&str, serde_json::Value)> = vec![
        ("DriveItem", serde_json::to_value(&shared).unwrap()),
        (
            "Listing",
            serde_json::to_value(model::list(&f.db, None, Sort::default(), false).unwrap()).unwrap(),
        ),
        (
            "ItemDetails",
            serde_json::to_value(model::details(&f.db, "a1").unwrap()).unwrap(),
        ),
        (
            "VersionEntry",
            serde_json::to_value(VersionEntry {
                id: "v".into(),
                created_at: "t".into(),
                size: 1,
                current: true,
            })
            .unwrap(),
        ),
        (
            "TrashEntry",
            serde_json::to_value(TrashEntry {
                item: item.clone(),
                trashed_at: "t".into(),
                original_path: "Files".into(),
            })
            .unwrap(),
        ),
        ("StorageSummary", serde_json::to_value(&storage).unwrap()),
        ("SyncSummary", serde_json::to_value(&sync).unwrap()),
        (
            "ShareEntry",
            serde_json::to_value(ShareEntry {
                item: item.clone(),
                direction: ShareDirection::ByMe,
                people: vec![SharePerson {
                    name: "a".into(),
                    email: Some("a@b.eu".into()),
                    role: ShareRole::Editor,
                }],
                has_link: true,
            })
            .unwrap(),
        ),
        (
            "PhotoPage",
            serde_json::to_value(PhotoPage {
                entries: vec![PhotoEntry {
                    item: item.clone(),
                    taken_at: "t".into(),
                }],
                next: Some("1".into()),
            })
            .unwrap(),
        ),
        (
            "UndoTicket",
            serde_json::to_value(UndoTicket { undo_token: "u".into() }).unwrap(),
        ),
        (
            "UploadStarted",
            serde_json::to_value(UploadStarted { started: 1 }).unwrap(),
        ),
        (
            "TextDocument",
            serde_json::to_value(TextDocument {
                text: "x".into(),
                version_id: "v1-ab".into(),
            })
            .unwrap(),
        ),
        (
            "DriveEvent::Changed",
            serde_json::to_value(DriveEvent::Changed {
                folder_id: Some("d".into()),
            })
            .unwrap(),
        ),
        (
            "DriveEvent::Item",
            serde_json::to_value(DriveEvent::Item { item }).unwrap(),
        ),
        (
            "DriveEvent::Removed",
            serde_json::to_value(DriveEvent::Removed { id: "x".into() }).unwrap(),
        ),
        (
            "DriveEvent::Sync",
            serde_json::to_value(DriveEvent::Sync { sync }).unwrap(),
        ),
        (
            "DriveEvent::Storage",
            serde_json::to_value(DriveEvent::Storage { storage }).unwrap(),
        ),
    ];
    assert_eq!(dtos.len(), 17);
    for (name, value) in &dtos {
        assert_no_secret_fields(value, name);
    }
    // The wire names match the TypeScript contract.
    let listing = &dtos[1].1;
    assert!(listing.get("asOf").is_some() && listing["items"][0].get("isFolder").is_some());
    assert_eq!(dtos[12].1["kind"], "changed");
    assert_eq!(dtos[12].1["folderId"], "d");
    assert_eq!(dtos[6].1["phase"], "up-to-date");
}

#[test]
fn drive_v2_list_uses_indexed_children_and_matches_full_scan() {
    let f = fixture();
    drive(&f);
    // The indexed list must produce the same items as the full-scan version.
    let root = model::list(&f.db, None, Sort::default(), false).unwrap();
    assert_eq!(
        names(&root.items),
        ["Docs", "Beach.jpg", "zebra.txt"],
        "indexed list matches expected root children"
    );
    let docs = model::list(
        &f.db,
        Some("d0c5"),
        Sort {
            key: SortKey::Name,
            dir: SortDir::Asc,
        },
        false,
    )
    .unwrap();
    assert_eq!(names(&docs.items), ["notes.txt", "Report.pdf"]);
    assert_eq!(docs.items[0].is_folder, false);
}

#[test]
fn drive_v2_children_of_paged() {
    let f = fixture();
    drive(&f);
    let (page, total) = model::children_of(&f.db, None, Sort::default(), 0, 2).unwrap();
    assert_eq!(total, 3, "root has 3 visible children");
    assert_eq!(page.len(), 2, "page 0 with limit 2");
    let (page2, total2) = model::children_of(&f.db, None, Sort::default(), 2, 2).unwrap();
    assert_eq!(total2, 3);
    assert_eq!(page2.len(), 1, "page 1 with limit 2");
    // The paged items match the full list in order.
    let full = model::list(&f.db, None, Sort::default(), false).unwrap();
    assert_eq!(page[0].id, full.items[0].id);
    assert_eq!(page[1].id, full.items[1].id);
    assert_eq!(page2[0].id, full.items[2].id);
}

#[test]
fn drive_v2_empty_path_row_shows_blocked_label() {
    let f = fixture();
    // Insert a file with an empty path (simulating a row written with a locked vault).
    row(
        &f.db,
        "empty1",
        "",
        Some("d0c5"),
        false,
        500,
        100,
        FileStatus::CloudOnly,
    );
    row(&f.db, "d0c5", "Docs", None, true, 0, 100, FileStatus::Local);
    let listing = model::list(&f.db, Some("d0c5"), Sort::default(), false).unwrap();
    let item = listing.items.iter().find(|i| i.id == "empty1").unwrap();
    assert!(
        item.name.starts_with("Unavailable file "),
        "an empty-path file shows the blocked label: {}",
        item.name
    );
}
