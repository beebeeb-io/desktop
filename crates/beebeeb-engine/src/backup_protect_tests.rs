//! LD-0113 (spec §4.2 rule 7, §6.2, Amendment 6): the drive's protection rule, on fixture rows in a
//! scratch state DB.

use super::*;
use crate::state_db::{FileEntry, FileStatus, ItemKind};
use beebeeb_core::backup::{
    MANIFEST_NAME, MSG_DELETE, MSG_MANAGED_IN_APP, MSG_MOVE, MSG_READ_ONLY, MSG_RENAME, own_backup_refusal,
};
use beebeeb_types::backup::SystemRole;

const ROOT_ID: &str = "0b5ec0de-0000-4000-8000-0000000000b1";

fn folder(db: &StateDb, file_id: &str, path: &str) {
    db.upsert_file(&FileEntry {
        file_id: file_id.into(),
        path: path.into(),
        status: FileStatus::CloudOnly,
        size_bytes: 0,
        modified_at: 0,
        content_hash: None,
        remote_updated_at: 0,
        parent_id: None,
        item_kind: ItemKind::Folder,
    })
    .unwrap();
}

/// A state DB with a flagged root at `Backups`, an unflagged look-alike at `Old/Backups`, and an
/// ordinary folder.
fn fixture() -> (tempfile::TempDir, StateDb) {
    let dir = tempfile::tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    folder(&db, ROOT_ID, "Backups");
    folder(&db, "0b5ec0de-0000-4000-8000-0000000000b2", "Old/Backups");
    folder(&db, "0b5ec0de-0000-4000-8000-0000000000b3", "Documents");
    assert!(db.set_system_role(ROOT_ID, Some(SystemRole::BackupsRoot)).unwrap());
    (dir, db)
}

fn source(name: &str) -> PathBuf {
    std::env::temp_dir().join("ld0113-src").join(name)
}

fn pictures_on() -> OwnBackups {
    OwnBackups {
        device_folder: Some("GLegion".into()),
        folders: vec![OwnBackupFolder {
            display_name: "Pictures".into(),
            source_path: source("Pictures"),
        }],
    }
}

const ALL: [Mutation; 4] = [
    Mutation::Trash,
    Mutation::DeletePermanently,
    Mutation::Rename,
    Mutation::Move,
];

#[test]
fn the_words_are_the_contracts() {
    assert_eq!(MSG_DELETE, "Backups is a system folder and can't be deleted.");
    assert_eq!(MSG_MOVE, "Backups is a system folder and can't be moved.");
    assert_eq!(MSG_RENAME, "Backups is a system folder and can't be renamed.");
    assert_eq!(
        own_backup_refusal("C:\\x"),
        "This is a backup of C:\\x. Change the original instead."
    );
    assert_eq!(
        MSG_MANAGED_IN_APP,
        "Backups are managed in the Beebeeb app. Open Beebeeb to delete a backup."
    );
    assert_eq!(
        MSG_READ_ONLY,
        "Backups are read-only: each device writes its own. Change the file on that device, or save a copy outside Backups."
    );
}

#[test]
fn the_root_refuses_each_mutation_with_cores_words() {
    let (_dir, db) = fixture();
    let own = OwnBackups::default();
    let reason = |m| protected_reason(&db, &own, "Backups", m);
    assert_eq!(reason(Mutation::Trash).as_deref(), Some(MSG_DELETE));
    assert_eq!(reason(Mutation::DeletePermanently).as_deref(), Some(MSG_DELETE));
    assert_eq!(reason(Mutation::Rename).as_deref(), Some(MSG_RENAME));
    assert_eq!(reason(Mutation::Move).as_deref(), Some(MSG_MOVE));
    // The path shapes the DB and the callbacks use, and a case the file system treats as the same.
    assert_eq!(
        protected_reason(&db, &own, "/Backups", Mutation::Trash).as_deref(),
        Some(MSG_DELETE)
    );
    assert_eq!(
        protected_reason(&db, &own, "backups", Mutation::Trash).as_deref(),
        Some(MSG_DELETE)
    );
}

#[test]
fn look_alikes_and_other_folders_are_ordinary() {
    let (_dir, db) = fixture();
    let own = pictures_on();
    for path in [
        "Old/Backups",
        "Old/Backups/GLegion/Pictures/a.jpg",
        "Documents",
        "BackupsX",
        "BackupsX/a.txt",
        "",
    ] {
        for m in ALL {
            assert_eq!(protected_reason(&db, &own, path, m), None, "{path} {m:?}");
        }
    }
}

/// Guus, 2026-11-06 17:49, "Protect the whole tree": in the drive nothing under the root is deleted,
/// renamed or moved. What is not this device's own active backup has core's managed-in-the-app words.
#[test]
fn everything_under_the_root_is_managed_in_the_app() {
    let (_dir, db) = fixture();
    let own = pictures_on();
    for path in [
        "Backups/OtherPC",
        "Backups/OtherPC/Pictures/a.jpg",
        "Backups/GLegion",
        "Backups/GLegion/Documents/a.txt",
        "Backups/GLegion2/Pictures/a.jpg",
        "backups/otherpc",
    ] {
        for m in ALL {
            assert_eq!(
                protected_reason(&db, &own, path, m).as_deref(),
                Some(MSG_MANAGED_IN_APP),
                "{path} {m:?}"
            );
        }
    }
}

#[test]
fn own_enabled_subtree_refuses_with_the_original_path() {
    let (_dir, db) = fixture();
    let own = pictures_on();
    let words = |p: PathBuf| Some(own_backup_refusal(&p.to_string_lossy()));
    for m in ALL {
        assert_eq!(
            protected_reason(&db, &own, "Backups/GLegion/Pictures", m),
            words(source("Pictures"))
        );
        assert_eq!(
            protected_reason(&db, &own, "Backups/GLegion/Pictures/2024/a.jpg", m),
            words(source("Pictures").join("2024").join("a.jpg"))
        );
        assert_eq!(
            protected_reason(&db, &own, "backups/glegion/pictures/a.jpg", m),
            words(source("Pictures").join("a.jpg")),
            "names compare as the file system does"
        );
    }
}

/// A turned-off folder's backup is no longer deletable in the drive either (it was an ordinary archive
/// before Guus's 17:49 ruling); the device folder is not one backup, so it has the managed words too.
#[test]
fn a_turned_off_backup_and_the_device_folder_are_managed_in_the_app() {
    let (_dir, db) = fixture();
    let off = OwnBackups {
        device_folder: Some("GLegion".into()),
        folders: Vec::new(),
    };
    for own in [&off, &pictures_on()] {
        for m in ALL {
            assert_eq!(
                protected_reason(&db, own, "Backups/GLegion", m).as_deref(),
                Some(MSG_MANAGED_IN_APP),
                "{m:?}"
            );
        }
    }
    for path in ["Backups/GLegion/Pictures", "Backups/GLegion/Pictures/a.jpg"] {
        for m in ALL {
            assert_eq!(
                protected_reason(&db, &off, path, m).as_deref(),
                Some(MSG_MANAGED_IN_APP),
                "{path} {m:?}"
            );
        }
    }
    // The root keeps its own words.
    assert_eq!(
        protected_reason(&db, &off, "Backups", Mutation::Trash).as_deref(),
        Some(MSG_DELETE)
    );
}

/// Moving an item from elsewhere in the drive INTO the tree is refused too (Amendment 6): core's
/// `Create` on the folder it would go into, whose words are read-only (Amendment 8).
#[test]
fn a_move_into_the_tree_is_refused() {
    let (_dir, db) = fixture();
    for target in [
        "Backups/report.pdf",
        "Backups/OtherPC/report.pdf",
        "backups/GLegion/Pictures/a.jpg",
    ] {
        assert_eq!(
            protected_destination(&db, target).as_deref(),
            Some(MSG_READ_ONLY),
            "{target}"
        );
    }
    for target in [
        "Documents/report.pdf",
        "Old/Backups/report.pdf",
        "BackupsX/report.pdf",
        "report.pdf",
        "",
    ] {
        assert_eq!(protected_destination(&db, target), None, "{target}");
    }
}

/// The rename and move decision the Windows callback and the watcher share, on every side of the tree.
#[test]
fn a_move_is_refused_into_out_of_and_within_the_tree_only() {
    let (_dir, db) = fixture();
    let own = OwnBackups::default();
    let refusal = |source: &str, target: Option<&str>| move_refusal(&db, &own, source, Mutation::Move, target);
    let managed = Some(MSG_MANAGED_IN_APP.to_string());
    let read_only = Some(MSG_READ_ONLY.to_string());
    assert_eq!(
        refusal("Documents/report.pdf", Some("Backups/OtherPC/report.pdf")),
        read_only,
        "into the tree"
    );
    assert_eq!(
        refusal("Documents/report.pdf", Some("Backups/report.pdf")),
        read_only,
        "into the root itself"
    );
    assert_eq!(
        refusal("Backups/OtherPC/a.txt", Some("Documents/a.txt")),
        managed,
        "out of the tree"
    );
    assert_eq!(
        refusal("Backups/OtherPC/a.txt", Some("Backups/GLegion/a.txt")),
        managed,
        "within the tree"
    );
    assert_eq!(
        refusal("Backups/OtherPC/a.txt", None),
        managed,
        "to the Recycle Bin, or an unread target"
    );
    assert_eq!(
        refusal("Backups", Some("Documents/Backups")).as_deref(),
        Some(MSG_MOVE),
        "the root has its own words"
    );
    assert_eq!(
        refusal("Documents/report.pdf", Some("Old/Backups/report.pdf")),
        None,
        "a look-alike is ordinary"
    );
    assert_eq!(
        refusal("Documents/report.pdf", Some("Documents/Sub/report.pdf")),
        None,
        "outside the tree"
    );
    assert_eq!(
        refusal("Documents/report.pdf", None),
        None,
        "an ordinary item to the Recycle Bin"
    );
}

/// Core v0.2.1's positions (spec Amendment 8): the depth below the flagged root and the name decide
/// them, and a content change or a new item anywhere in the tree is read-only (Amendment 7). A
/// device's manifest is refused everything, with the read-only words, and is hidden from every list.
#[test]
fn positions_follow_the_depth_below_the_root() {
    let (_dir, db) = fixture();
    let tree = BackupsTree::read(&db);
    let manifest = format!("Backups/OtherPC/{MANIFEST_NAME}");
    for (path, position) in [
        ("Backups", BackupPosition::Root),
        ("/Backups/", BackupPosition::Root),
        ("Backups/OtherPC", BackupPosition::DeviceFolder),
        (manifest.as_str(), BackupPosition::Manifest),
        ("Backups/OtherPC/Pictures", BackupPosition::CategoryFolder),
        ("Backups/OtherPC/Pictures/a.jpg", BackupPosition::Inside),
        ("Backups/OtherPC/Pictures/2024/.device.json", BackupPosition::Inside),
        ("Old/Backups/OtherPC", BackupPosition::Outside),
        ("Documents", BackupPosition::Outside),
    ] {
        assert_eq!(tree.position(path), position, "{path}");
    }
    assert!(tree.is_hidden(&manifest));
    assert!(tree.is_hidden(&format!("/{manifest}")), "the leading-slash form");
    for path in [
        "Backups/OtherPC/Pictures/2024/.device.json",
        "Documents/OtherPC/.device.json",
        "Backups/.device.json",
        "Backups/OtherPC",
    ] {
        assert!(!tree.is_hidden(path), "{path}");
    }
    let own = pictures_on();
    for m in Mutation::ALL {
        assert_eq!(
            protected_reason(&db, &own, &manifest, m).as_deref(),
            Some(MSG_READ_ONLY),
            "the manifest, {m:?}"
        );
    }
    for path in [
        "Backups/OtherPC",
        "Backups/OtherPC/Pictures",
        "Backups/OtherPC/Pictures/a.jpg",
    ] {
        for m in [Mutation::Create, Mutation::Write] {
            assert_eq!(
                protected_reason(&db, &own, path, m).as_deref(),
                Some(MSG_READ_ONLY),
                "{path} {m:?}"
            );
        }
    }
    assert_eq!(
        protected_reason(&db, &own, "Backups", Mutation::Create).as_deref(),
        Some(MSG_READ_ONLY),
        "a new item in the root"
    );
    // This device's own active backup keeps naming its original, whatever the change.
    let words = Some(own_backup_refusal(&source("Pictures").join("a.jpg").to_string_lossy()));
    assert_eq!(
        protected_reason(&db, &own, "Backups/GLegion/Pictures/a.jpg", Mutation::Write),
        words
    );
    for m in [Mutation::Create, Mutation::Write] {
        assert_eq!(
            protected_reason(&db, &own, "Documents/a.txt", m),
            None,
            "outside, {m:?}"
        );
    }
}

/// Review #3 and #4: rows stored with a leading slash (`/Backups/…`, as `untrash_subtree` allows) are in
/// the subtree, and at start-up every downloaded row of the tree whose file is gone is cloud-only
/// again; a file still on disk, a look-alike folder's row and a row outside the tree are untouched.
#[test]
fn leading_slash_rows_are_found_and_put_back_at_start() {
    let (dir, db) = fixture();
    let sync_root = dir.path().join("root");
    std::fs::create_dir_all(sync_root.join("Backups").join("OtherPC")).unwrap();
    std::fs::write(sync_root.join("Backups").join("OtherPC").join("kept.jpg"), b"kept").unwrap();
    let local = |id: &str, path: &str| {
        db.upsert_file(&FileEntry {
            file_id: id.into(),
            path: path.into(),
            status: FileStatus::Local,
            size_bytes: 4,
            modified_at: 0,
            content_hash: None,
            remote_updated_at: 0,
            parent_id: None,
            item_kind: ItemKind::File,
        })
        .unwrap();
    };
    let gone_in_tree = [
        ("0b5ec0de-0000-4000-8000-0000000000f1", "/Backups/OtherPC/a.jpg"),
        ("0b5ec0de-0000-4000-8000-0000000000f2", "Backups/OtherPC/b.jpg"),
        ("0b5ec0de-0000-4000-8000-0000000000f3", "/Backups/OtherPCX/c.jpg"),
    ];
    for (id, path) in gone_in_tree {
        local(id, path);
    }
    let kept = ("0b5ec0de-0000-4000-8000-0000000000f4", "Backups/OtherPC/kept.jpg");
    let outside = ("0b5ec0de-0000-4000-8000-0000000000f5", "Docs/e.txt");
    local(kept.0, kept.1);
    local(outside.0, outside.1);

    let mut under: Vec<String> = db
        .local_rows_at_or_under("Backups/OtherPC")
        .unwrap()
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    under.sort();
    assert_eq!(
        under,
        vec![gone_in_tree[0].0, gone_in_tree[1].0, kept.0],
        "both path forms, not the look-alike OtherPCX"
    );

    reconcile_backups_tree(&db, &sync_root);
    let status = |id: &str| db.get_file(id).unwrap().unwrap().status;
    for (id, path) in gone_in_tree {
        assert_eq!(status(id), FileStatus::CloudOnly, "{path} is put back");
    }
    assert_eq!(status(kept.0), FileStatus::Local, "a file still on disk is untouched");
    assert_eq!(
        status(outside.0),
        FileStatus::Local,
        "a row outside the tree is untouched"
    );
}

#[test]
fn without_a_flagged_root_nothing_is_protected() {
    let dir = tempfile::tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    folder(&db, ROOT_ID, "Backups");
    let own = pictures_on();
    for path in ["Backups", "Backups/GLegion", "Backups/GLegion/Pictures/a.jpg"] {
        assert_eq!(
            protected_reason(&db, &own, path, Mutation::Trash),
            None,
            "{path}: the name alone protects nothing"
        );
        assert_eq!(protected_destination(&db, path), None, "{path}");
    }
}

/// One delete that Windows asks about four times is one notice; other words wait out the floor; the
/// same words come back once the quiet window has passed.
#[test]
fn one_refused_delete_is_one_notice() {
    let mut gate = NoticeGate::default();
    let t0 = std::time::Instant::now();
    let a = own_backup_refusal("C:\\home\\qa\\Pictures\\a.jpg");
    let b = own_backup_refusal("C:\\home\\qa\\Pictures\\b.jpg");
    let shown: Vec<bool> = (0..4)
        .map(|i| gate.due(&a, t0 + std::time::Duration::from_millis(i * 30)))
        .collect();
    assert_eq!(
        shown,
        vec![true, false, false, false],
        "four callbacks for one delete, one notice"
    );
    assert!(
        !gate.due(&b, t0 + std::time::Duration::from_millis(500)),
        "other words within the floor wait"
    );
    assert!(gate.due(&b, t0 + NoticeGate::FLOOR), "and are shown after it");
    assert!(
        !gate.due(&a, t0 + NoticeGate::FLOOR * 3),
        "the same words within the quiet window are not shown again"
    );
    assert!(
        gate.due(&a, t0 + NoticeGate::QUIET + NoticeGate::FLOOR),
        "after it they are"
    );
}

#[test]
fn a_rename_is_a_trash_a_move_or_a_rename() {
    let root = std::env::temp_dir().join("ld0113-root");
    let recycle = std::env::temp_dir()
        .join("$RECYCLE.BIN")
        .join("S-1-5-21")
        .join("$R1A2B3C");
    assert_eq!(
        rename_mutation(&root.join("Backups"), &recycle),
        Mutation::Trash,
        "Explorer's Delete"
    );
    assert_eq!(
        rename_mutation(&root.join("Backups"), &root.join("Docs").join("Backups")),
        Mutation::Move
    );
    assert_eq!(
        rename_mutation(&root.join("Backups"), &root.join("Old backups")),
        Mutation::Rename
    );
}

/// Amendment 7 (Windows): the read-only state goes on every row of the tree, whatever its status, in
/// both stored path forms, and on nothing beside it.
#[test]
fn paths_at_or_under_cover_the_whole_tree_only() {
    let (_dir, db) = fixture();
    folder(&db, "0b5ec0de-0000-4000-8000-0000000000e1", "Backups/OtherPC");
    folder(&db, "0b5ec0de-0000-4000-8000-0000000000e2", "/Backups/OtherPC/Pictures");
    folder(&db, "0b5ec0de-0000-4000-8000-0000000000e3", "BackupsX/a");
    folder(&db, "0b5ec0de-0000-4000-8000-0000000000e4", "Backups0");
    db.set_status("0b5ec0de-0000-4000-8000-0000000000e1", FileStatus::Local)
        .unwrap();
    assert_eq!(
        db.paths_at_or_under("/Backups/").unwrap(),
        vec![
            "Backups".to_string(),
            "Backups/OtherPC".to_string(),
            "/Backups/OtherPC/Pictures".to_string()
        ]
    );
}

/// LD-0113 (lead 2026-11-06 20:01; core's `is_hidden_backup_item`): quick search, the Files overview
/// and the tray's recent list never offer a device folder's manifest, which each backup pass rewrites
/// (it would lead every recent list). A file of the same name elsewhere is ordinary.
#[test]
fn the_manifest_is_never_listed_in_the_app() {
    let (_dir, db) = fixture();
    let file = |file_id: &str, path: &str, modified_at: i64| {
        db.upsert_file(&FileEntry {
            file_id: file_id.into(),
            path: path.into(),
            status: FileStatus::CloudOnly,
            size_bytes: 10,
            modified_at,
            content_hash: None,
            remote_updated_at: modified_at,
            parent_id: None,
            item_kind: ItemKind::File,
        })
        .unwrap();
    };
    let manifest = "0b5ec0de-0000-4000-8000-0000000000f1";
    file(manifest, &format!("Backups/OtherPC/{MANIFEST_NAME}"), 2_000_000_000);
    file(
        "0b5ec0de-0000-4000-8000-0000000000f2",
        "Backups/OtherPC/Pictures/a.jpg",
        1_900_000_000,
    );
    file(
        "0b5ec0de-0000-4000-8000-0000000000f3",
        "Documents/.device.json",
        1_800_000_000,
    );

    let search = crate::desktop_search::build_local_index(&db).unwrap();
    assert!(
        !search.records.contains_key(manifest),
        "quick search never offers the manifest"
    );
    for offered in [
        "0b5ec0de-0000-4000-8000-0000000000f2",
        "0b5ec0de-0000-4000-8000-0000000000f3",
    ] {
        assert!(
            search.records.contains_key(offered),
            "the backed-up photo and the ordinary file are offered"
        );
    }

    let recent: Vec<String> = crate::status_views::recent_files(&db, 2)
        .unwrap()
        .into_iter()
        .map(|f| f.path)
        .collect();
    assert_eq!(
        recent,
        vec![
            "Backups/OtherPC/Pictures/a.jpg".to_string(),
            "Documents/.device.json".to_string()
        ],
        "the tray's list of 2 leaves the manifest out and still holds 2"
    );
    let overview: Vec<String> = crate::status_views::listable_rows(&db, db.file_overview_rows().unwrap())
        .into_iter()
        .map(|(entry, _)| entry.path)
        .collect();
    assert!(
        !overview
            .iter()
            .any(|path| path.ends_with(MANIFEST_NAME) && path.starts_with("Backups/")),
        "{overview:?}"
    );
    assert!(
        overview.iter().any(|path| path == "Documents/.device.json"),
        "the overview keeps the ordinary file"
    );
}
