//! LD-0113 (spec §4.2 rule 7, §6.2): the protection rule, on fixture rows in a scratch state DB.

use super::*;
use crate::state_db::{FileEntry, FileStatus, ItemKind};
use beebeeb_core::backup::{MSG_DELETE, MSG_MOVE, MSG_RENAME};

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

const ALL: [Mutation; 4] = [Mutation::Trash, Mutation::DeletePermanently, Mutation::Rename, Mutation::Move];

#[test]
fn the_words_are_the_contracts() {
    assert_eq!(MSG_DELETE, "Backups is a system folder and can't be deleted.");
    assert_eq!(MSG_MOVE, "Backups is a system folder and can't be moved.");
    assert_eq!(MSG_RENAME, "Backups is a system folder and can't be renamed.");
    assert_eq!(own_backup_refusal("C:\\x"), "This is a backup of C:\\x. Change the original instead.");
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
    assert_eq!(protected_reason(&db, &own, "/Backups", Mutation::Trash).as_deref(), Some(MSG_DELETE));
    assert_eq!(protected_reason(&db, &own, "backups", Mutation::Trash).as_deref(), Some(MSG_DELETE));
}

#[test]
fn children_look_alikes_and_other_folders_are_ordinary() {
    let (_dir, db) = fixture();
    let own = pictures_on();
    for path in ["Backups/OtherPC", "Backups/OtherPC/Pictures/a.jpg", "Old/Backups", "Documents", "BackupsX", ""] {
        for m in ALL {
            assert_eq!(protected_reason(&db, &own, path, m), None, "{path} {m:?}");
        }
    }
}

#[test]
fn own_enabled_subtree_refuses_with_the_original_path() {
    let (_dir, db) = fixture();
    let own = pictures_on();
    let words = |p: PathBuf| Some(own_backup_refusal(&p.to_string_lossy()));
    for m in ALL {
        assert_eq!(protected_reason(&db, &own, "Backups/GLegion/Pictures", m), words(source("Pictures")));
        assert_eq!(
            protected_reason(&db, &own, "Backups/GLegion/Pictures/2024/a.jpg", m),
            words(source("Pictures").join("2024").join("a.jpg"))
        );
        assert_eq!(
            protected_reason(&db, &own, "backups/glegion/pictures/a.jpg", m),
            words(source("Pictures").join("a.jpg")),
            "names compare as the file system does"
        );
        // A folder that is not on, another device's backup, and a look-alike device folder are ordinary.
        assert_eq!(protected_reason(&db, &own, "Backups/GLegion/Documents/a.txt", m), None);
        assert_eq!(protected_reason(&db, &own, "Backups/GLegion2/Pictures/a.jpg", m), None);
        assert_eq!(protected_reason(&db, &own, "Backups/OtherPC/Pictures/a.jpg", m), None);
    }
}

#[test]
fn own_disabled_subtree_is_an_ordinary_archive() {
    let (_dir, db) = fixture();
    let off = OwnBackups {
        device_folder: Some("GLegion".into()),
        folders: Vec::new(),
    };
    for path in ["Backups/GLegion", "Backups/GLegion/Pictures", "Backups/GLegion/Pictures/a.jpg"] {
        for m in ALL {
            assert_eq!(protected_reason(&db, &off, path, m), None, "{path} {m:?}");
        }
    }
    // The root itself stays protected.
    assert_eq!(protected_reason(&db, &off, "Backups", Mutation::Trash).as_deref(), Some(MSG_DELETE));
}

#[test]
fn the_device_folder_names_every_folder_it_backs_up() {
    let (_dir, db) = fixture();
    let mut own = pictures_on();
    assert_eq!(
        protected_reason(&db, &own, "Backups/GLegion", Mutation::Trash),
        Some(own_backup_refusal(&source("Pictures").to_string_lossy()))
    );
    own.folders.insert(
        0,
        OwnBackupFolder {
            display_name: "Documents".into(),
            source_path: source("Documents"),
        },
    );
    assert_eq!(
        protected_reason(&db, &own, "Backups/GLegion", Mutation::Move),
        Some(own_backup_refusal(&format!(
            "{} and {}",
            source("Documents").to_string_lossy(),
            source("Pictures").to_string_lossy()
        )))
    );
    assert_eq!(join_sources(&["A".into(), "B".into(), "C".into()]), "A, B and C");
}

#[test]
fn without_a_flagged_root_nothing_is_protected() {
    let dir = tempfile::tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    folder(&db, ROOT_ID, "Backups");
    let own = pictures_on();
    for path in ["Backups", "Backups/GLegion", "Backups/GLegion/Pictures/a.jpg"] {
        assert_eq!(protected_reason(&db, &own, path, Mutation::Trash), None, "{path}: the name alone protects nothing");
    }
}

/// One delete that Windows asks about four times is one notice; other words wait out the floor; the
/// same words come back once the quiet window has passed.
#[test]
fn one_refused_delete_is_one_notice() {
    let mut gate = NoticeGate::default();
    let t0 = std::time::Instant::now();
    let a = own_backup_refusal("C:\\Users\\qa\\Pictures\\a.jpg");
    let b = own_backup_refusal("C:\\Users\\qa\\Pictures\\b.jpg");
    let shown: Vec<bool> = (0..4).map(|i| gate.due(&a, t0 + std::time::Duration::from_millis(i * 30))).collect();
    assert_eq!(shown, vec![true, false, false, false], "four callbacks for one delete, one notice");
    assert!(!gate.due(&b, t0 + std::time::Duration::from_millis(500)), "other words within the floor wait");
    assert!(gate.due(&b, t0 + NoticeGate::FLOOR), "and are shown after it");
    assert!(!gate.due(&a, t0 + NoticeGate::FLOOR * 3), "the same words within the quiet window are not shown again");
    assert!(gate.due(&a, t0 + NoticeGate::QUIET + NoticeGate::FLOOR), "after it they are");
}

#[test]
fn a_rename_is_a_trash_a_move_or_a_rename() {
    let root = std::env::temp_dir().join("ld0113-root");
    let recycle = std::env::temp_dir().join("$RECYCLE.BIN").join("S-1-5-21").join("$R1A2B3C");
    assert_eq!(rename_mutation(&root.join("Backups"), &recycle), Mutation::Trash, "Explorer's Delete");
    assert_eq!(rename_mutation(&root.join("Backups"), &root.join("Docs").join("Backups")), Mutation::Move);
    assert_eq!(rename_mutation(&root.join("Backups"), &root.join("Old backups")), Mutation::Rename);
}
