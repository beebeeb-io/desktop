//! LD-0113 D2 (spec §6.3, Amendment 6): the File Provider payload of a protected item (fixture rows
//! only).

use super::*;
use crate::backup_protect::{OwnBackupFolder, OwnBackups};
use crate::state_db::FpChangeKind;
use beebeeb_core::backup::{MSG_DELETE, MSG_MANAGED_IN_APP, MSG_MOVE, MSG_RENAME, own_backup_refusal};

const ROOT_ID: &str = "0b5ec0de-0000-4000-8000-0000000000d1";
/// What a protected item loses: delete, trash, rename and move, and (a folder) adding items, so Finder
/// offers no New Folder and refuses a drop into it (Amendment 6).
const TAKEN_AWAY: u32 = CAP_DELETE | CAP_TRASH | CAP_RENAME | CAP_REPARENT | CAP_ADD_SUBITEMS;

fn row(db: &StateDb, file_id: &str, path: &str, kind: ItemKind) -> FileEntry {
    let entry = FileEntry {
        file_id: file_id.into(),
        path: path.into(),
        status: FileStatus::Local,
        size_bytes: 1,
        modified_at: 1,
        content_hash: None,
        remote_updated_at: 1,
        parent_id: None,
        item_kind: kind.clone(),
    };
    db.upsert_file(&entry).unwrap();
    let mut contract = db.get_file_contract_state(file_id).unwrap().unwrap();
    contract.item_kind = kind;
    db.set_file_contract_state(&contract).unwrap();
    db.get_file(file_id).unwrap().unwrap()
}

fn source() -> std::path::PathBuf {
    std::path::PathBuf::from("/Users/qa/Pictures")
}

/// Guus, 2026-11-06 17:49, "Protect the whole tree": every item at or under the root is protected in
/// Finder, with core's words for what it is; an item outside the tree is unchanged.
#[test]
fn payload_marks_whole_backups_tree_protected() {
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    let root = row(&db, ROOT_ID, "Backups", ItemKind::Folder);
    assert!(db.set_system_role(ROOT_ID, Some(beebeeb_types::backup::SystemRole::BackupsRoot)).unwrap());
    let device_folder = row(&db, "0b5ec0de-0000-4000-8000-0000000000d5", "Backups/GLegion", ItemKind::Folder);
    let category = row(&db, "0b5ec0de-0000-4000-8000-0000000000d6", "Backups/GLegion/Documents", ItemKind::Folder);
    let other_device = row(&db, "0b5ec0de-0000-4000-8000-0000000000d3", "Backups/OtherPC/Pictures/b.jpg", ItemKind::File);
    let own_file = row(&db, "0b5ec0de-0000-4000-8000-0000000000d2", "Backups/GLegion/Pictures/a.jpg", ItemKind::File);
    let outside = row(&db, "0b5ec0de-0000-4000-8000-0000000000d4", "Docs", ItemKind::Folder);
    let own = OwnBackups {
        device_folder: Some("GLegion".into()),
        folders: vec![OwnBackupFolder {
            display_name: "Pictures".into(),
            source_path: source(),
        }],
    };
    // The item as the builder makes it, then protected with this test's own backups (the process-wide
    // ones `file_entry_payload_for_db` reads are not this test's).
    let unprotected = |entry: &FileEntry| file_entry_payload(entry, &db.get_file_contract_state(&entry.file_id).unwrap().unwrap(), "parent");
    let built = |entry: &FileEntry| {
        let mut payload = unprotected(entry);
        apply_protection(&db, &own, entry, &mut payload);
        payload
    };
    assert_ne!(unprotected(&device_folder).capabilities & CAP_ADD_SUBITEMS, 0, "a live folder offers New Folder until protected");

    // The root, through the builder Finder's enumeration uses (its protection needs no own backups).
    let root_payload = file_entry_payload_for_db(&db, &root, "NSFileProviderRootContainerItemIdentifier");
    assert!(root_payload.protected, "the Backups root is protected");
    assert_eq!(root_payload.capabilities & TAKEN_AWAY, 0, "no delete, trash, rename, move or New Folder");
    assert_ne!(root_payload.capabilities & CAP_READ, 0, "still readable");
    assert_eq!(root_payload.protected_reason.as_deref(), Some(MSG_DELETE));
    assert_eq!(root_payload.protected_rename_reason.as_deref(), Some(MSG_RENAME));
    assert_eq!(root_payload.protected_move_reason.as_deref(), Some(MSG_MOVE));
    assert!(root_payload.metadata_version.as_deref().unwrap().ends_with(":protected"));
    let wire = serde_json::to_value(&root_payload).unwrap();
    assert_eq!(wire["protected"], true);
    assert_eq!(wire["protected_reason"], MSG_DELETE);

    // A device folder, a category folder and a file under another device's folder: managed in the app.
    for entry in [&device_folder, &category, &other_device] {
        let payload = built(entry);
        assert!(payload.protected, "{}", entry.path);
        assert_eq!(payload.capabilities & TAKEN_AWAY, 0, "{}", entry.path);
        assert_ne!(payload.capabilities & CAP_READ, 0, "{} stays readable", entry.path);
        for reason in [&payload.protected_reason, &payload.protected_rename_reason, &payload.protected_move_reason] {
            assert_eq!(reason.as_deref(), Some(MSG_MANAGED_IN_APP), "{}", entry.path);
        }
        assert!(payload.metadata_version.as_deref().unwrap().ends_with(":protected"), "{}", entry.path);
    }

    // This device's own backup of a folder it backs up now says where the original is.
    let payload = built(&own_file);
    let words = own_backup_refusal(&source().join("a.jpg").to_string_lossy());
    assert!(payload.protected);
    assert_eq!(payload.capabilities & TAKEN_AWAY, 0);
    assert_ne!(payload.capabilities & CAP_WRITE, 0, "a content edit is not intercepted (spec §6.2)");
    for reason in [&payload.protected_reason, &payload.protected_rename_reason, &payload.protected_move_reason] {
        assert_eq!(reason.as_deref(), Some(words.as_str()));
    }

    // A folder outside the tree is unchanged, adding items into it included.
    let payload = built(&outside);
    assert!(!payload.protected);
    assert_eq!(payload.capabilities, unprotected(&outside).capabilities);
    assert_ne!(payload.capabilities & CAP_ADD_SUBITEMS, 0, "a drop into an ordinary folder stays allowed");
    assert_eq!(payload.metadata_version, unprotected(&outside).metadata_version);
    assert_eq!(payload.protected_reason, None);
    let wire = serde_json::to_value(&payload).unwrap();
    assert_eq!(wire["protected"], false);
    assert!(wire.get("protected_reason").is_none(), "no words on the wire for an ordinary item");
}

/// The protection of everything under the root starts (and ends) with the root's role, and Finder
/// re-reads an item's capabilities only when the change log names it: a Backups folder that existed
/// before the server flagged it (an iOS backup) must not keep offering Delete on what is in it.
#[test]
fn flagging_the_root_tells_finder_about_everything_under_it() {
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    row(&db, ROOT_ID, "Backups", ItemKind::Folder);
    let under = ["0b5ec0de-0000-4000-8000-0000000000d7", "0b5ec0de-0000-4000-8000-0000000000d8", "0b5ec0de-0000-4000-8000-0000000000da"];
    row(&db, under[0], "Backups/iPhone", ItemKind::Folder);
    row(&db, under[1], "Backups/iPhone/Camera Roll/IMG_0001.HEIC", ItemKind::File);
    // Review #2: a child stored with a leading slash, as `untrash_subtree` allows.
    row(&db, under[2], "/Backups/iPhone/Camera Roll/IMG_0002.HEIC", ItemKind::File);
    row(&db, "0b5ec0de-0000-4000-8000-0000000000d9", "BackupsX/c.txt", ItemKind::File);
    let changed_by = |role| {
        let (_, anchor) = db.list_file_changes(None).unwrap().unwrap();
        assert!(db.set_system_role(ROOT_ID, role).unwrap());
        let (changes, _) = db.list_file_changes(anchor.as_deref()).unwrap().unwrap();
        assert!(changes.iter().all(|change| change.kind == FpChangeKind::Modified), "{changes:?}");
        let mut ids: Vec<String> = changes.into_iter().map(|change| change.file_id).collect();
        ids.sort();
        ids
    };
    let mut want: Vec<String> = [ROOT_ID, under[0], under[1], under[2]].iter().map(|id| id.to_string()).collect();
    want.sort();
    assert_eq!(changed_by(Some(beebeeb_types::backup::SystemRole::BackupsRoot)), want, "flagged: the root and all under it");
    // Review #7: the same role again writes nothing and tells Finder nothing.
    let (_, anchor) = db.list_file_changes(None).unwrap().unwrap();
    assert!(!db.set_system_role(ROOT_ID, Some(beebeeb_types::backup::SystemRole::BackupsRoot)).unwrap());
    assert!(db.list_file_changes(anchor.as_deref()).unwrap().unwrap().0.is_empty(), "an unchanged role records no change");
    assert_eq!(changed_by(None), want, "cleared: the same items, once each");
}
