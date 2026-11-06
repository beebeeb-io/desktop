//! LD-0113 D2 (spec §6.3): the File Provider payload of a protected item (fixture rows only).

use super::*;
use crate::backup_protect::{OwnBackupFolder, OwnBackups};
use beebeeb_core::backup::{MSG_DELETE, MSG_MOVE, MSG_RENAME, own_backup_refusal};

const ROOT_ID: &str = "0b5ec0de-0000-4000-8000-0000000000d1";
const TAKEN_AWAY: u32 = CAP_DELETE | CAP_TRASH | CAP_RENAME | CAP_REPARENT;

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

#[test]
fn payload_marks_root_and_enabled_own_subtree_protected() {
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    let root = row(&db, ROOT_ID, "Backups", ItemKind::Folder);
    assert!(db.set_system_role(ROOT_ID, Some(beebeeb_types::backup::SystemRole::BackupsRoot)).unwrap());
    let own_file = row(&db, "0b5ec0de-0000-4000-8000-0000000000d2", "Backups/GLegion/Pictures/a.jpg", ItemKind::File);
    let off_file = row(&db, "0b5ec0de-0000-4000-8000-0000000000d3", "Backups/GLegion/Documents/b.txt", ItemKind::File);
    let ordinary = row(&db, "0b5ec0de-0000-4000-8000-0000000000d4", "Docs/c.txt", ItemKind::File);
    let own = OwnBackups {
        device_folder: Some("GLegion".into()),
        folders: vec![OwnBackupFolder {
            display_name: "Pictures".into(),
            source_path: source(),
        }],
    };

    // The root, through the builder Finder's enumeration uses (its protection needs no own backups).
    let root_payload = file_entry_payload_for_db(&db, &root, "NSFileProviderRootContainerItemIdentifier");
    assert!(root_payload.protected, "the Backups root is protected");
    assert_eq!(root_payload.capabilities & TAKEN_AWAY, 0, "no delete, trash, rename or move");
    assert_ne!(root_payload.capabilities & CAP_READ, 0, "still readable");
    assert_ne!(root_payload.capabilities & CAP_ADD_SUBITEMS, 0, "moving things INTO the root stays allowed");
    assert_eq!(root_payload.protected_reason.as_deref(), Some(MSG_DELETE));
    assert_eq!(root_payload.protected_rename_reason.as_deref(), Some(MSG_RENAME));
    assert_eq!(root_payload.protected_move_reason.as_deref(), Some(MSG_MOVE));
    assert!(root_payload.metadata_version.as_deref().unwrap().ends_with(":protected"));
    let wire = serde_json::to_value(&root_payload).unwrap();
    assert_eq!(wire["protected"], true);
    assert_eq!(wire["protected_reason"], MSG_DELETE);

    // This device's own backup of a folder it backs up now.
    let mut payload = file_entry_payload_without_contract(&own_file, "parent");
    apply_protection(&db, &own, &own_file, &mut payload);
    let words = own_backup_refusal(&source().join("a.jpg").to_string_lossy());
    assert!(payload.protected);
    assert_eq!(payload.capabilities & TAKEN_AWAY, 0);
    assert_ne!(payload.capabilities & CAP_WRITE, 0, "a content edit is not intercepted (spec §6.2)");
    for reason in [&payload.protected_reason, &payload.protected_rename_reason, &payload.protected_move_reason] {
        assert_eq!(reason.as_deref(), Some(words.as_str()));
    }

    // A folder that is not on, and an ordinary file, are unchanged.
    for entry in [&off_file, &ordinary] {
        let unprotected = file_entry_payload_without_contract(entry, "parent");
        let mut payload = unprotected.clone();
        apply_protection(&db, &own, entry, &mut payload);
        assert!(!payload.protected, "{}", entry.path);
        assert_eq!(payload.capabilities, unprotected.capabilities, "{}", entry.path);
        assert_eq!(payload.metadata_version, unprotected.metadata_version, "{}", entry.path);
        assert_eq!(payload.protected_reason, None);
        let wire = serde_json::to_value(&payload).unwrap();
        assert_eq!(wire["protected"], false);
        assert!(wire.get("protected_reason").is_none(), "no words on the wire for an ordinary item");
    }
}
