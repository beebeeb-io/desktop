//! LD-0113 in the app's window: every action at every place in the Backups tree, against core's words; then the
//! same rule read from a state database, and the listing (manifest hidden, root first, each row marked).

use super::*;
use crate::backup_protect::OwnBackupFolder;
use crate::drive_v2::model::{self, Sort, SortDir, SortKey};
use crate::state_db::{FileEntry, FileStatus, ItemKind};
use beebeeb_core::backup::{MANIFEST_NAME, MSG_DELETE, MSG_MOVE, MSG_RENAME};
use beebeeb_types::backup::SystemRole;
use std::path::PathBuf;

use Action::*;
use BackupPosition::*;

fn no(words: &str) -> Verdict {
    Verdict::Refused(words.to_string())
}

fn subject(position: BackupPosition, is_folder: bool) -> Subject<'static> {
    Subject {
        position,
        is_folder,
        own_active_source: None,
    }
}

/// The eleven answers for one row, in [`Action::ALL`]'s order.
fn answers(subject: &Subject<'_>) -> Vec<(Action, Verdict)> {
    Action::ALL.into_iter().map(|a| (a, verdict(subject, a))).collect()
}

fn expect(row: &str, subject: &Subject<'_>, want: [Verdict; 11]) {
    let want: Vec<(Action, Verdict)> = Action::ALL.into_iter().zip(want).collect();
    assert_eq!(answers(subject), want, "{row}");
}

#[test]
fn the_words_that_are_not_cores_are_the_web_apps() {
    assert_eq!(
        MSG_NO_FOLDER_SHARE,
        "Folders in Backups can't be shared. You can share a single file."
    );
    assert_eq!(MSG_DELETE_BACKUP_STEP_UP, "Confirm it's you to delete this backup.");
}

/// Order: Trash, DeleteForever, Rename, Move, MoveInto, Share, UploadInto, NewFolderIn, Edit, FreeUpSpace,
/// DeleteBackup.
#[test]
fn every_action_at_every_place_in_the_tree() {
    let read_only = || no(MSG_READ_ONLY);
    let ok = || Verdict::Allowed;
    for is_folder in [true, false] {
        expect(
            "an ordinary row",
            &subject(Outside, is_folder),
            [
                ok(),
                ok(),
                ok(),
                ok(),
                ok(),
                ok(),
                ok(),
                ok(),
                ok(),
                ok(),
                Verdict::NotABackup,
            ],
        );
    }
    expect(
        "the Backups root",
        &subject(Root, true),
        [
            no(MSG_DELETE),
            no(MSG_DELETE),
            no(MSG_RENAME),
            no(MSG_MOVE),
            read_only(),
            no(MSG_NO_FOLDER_SHARE),
            read_only(),
            read_only(),
            read_only(),
            ok(),
            no(MSG_DELETE),
        ],
    );
    // Another device's folder, another device's backup of one folder, and this device's backup of a folder
    // that is turned off (no own source): a backup, deleted only through its own call.
    for (row, position) in [
        ("a device's folder", DeviceFolder),
        ("one backed-up folder", CategoryFolder),
    ] {
        expect(
            row,
            &subject(position, true),
            [
                Verdict::UseDeleteBackup,
                Verdict::UseDeleteBackup,
                read_only(),
                read_only(),
                read_only(),
                no(MSG_NO_FOLDER_SHARE),
                read_only(),
                read_only(),
                read_only(),
                ok(),
                Verdict::NeedsPassword,
            ],
        );
    }
    expect(
        "a file inside a backup",
        &subject(Inside, false),
        [
            ok(),
            ok(),
            read_only(),
            read_only(),
            read_only(),
            ok(),
            read_only(),
            read_only(),
            read_only(),
            ok(),
            Verdict::NotABackup,
        ],
    );
    expect(
        "a folder inside a backup",
        &subject(Inside, true),
        [
            ok(),
            ok(),
            read_only(),
            read_only(),
            read_only(),
            no(MSG_NO_FOLDER_SHARE),
            read_only(),
            read_only(),
            read_only(),
            ok(),
            Verdict::NotABackup,
        ],
    );
    expect(
        "a device's manifest",
        &subject(Manifest, false),
        [
            read_only(),
            read_only(),
            read_only(),
            read_only(),
            read_only(),
            read_only(),
            read_only(),
            read_only(),
            read_only(),
            ok(),
            read_only(),
        ],
    );
}

#[test]
fn this_devices_own_active_backup_says_where_the_original_is() {
    let own = || no(&own_backup_refusal("C:\\Users\\a\\Pictures"));
    for (position, is_folder) in [(CategoryFolder, true), (Inside, true), (Inside, false)] {
        let subject = Subject {
            position,
            is_folder,
            own_active_source: Some("C:\\Users\\a\\Pictures"),
        };
        let share = if is_folder {
            no(MSG_NO_FOLDER_SHARE)
        } else {
            Verdict::Allowed
        };
        expect(
            "own active backup",
            &subject,
            [
                own(),
                own(),
                own(),
                own(),
                own(),
                share,
                own(),
                own(),
                own(),
                Verdict::Allowed,
                own(),
            ],
        );
    }
    // The root keeps its own words even when a caller passes a source for it.
    let root = Subject {
        position: Root,
        is_folder: true,
        own_active_source: Some("C:\\x"),
    };
    assert_eq!(verdict(&root, Trash), no(MSG_DELETE));
    assert_eq!(verdict(&root, Rename), no(MSG_RENAME));
}

#[test]
fn a_menu_offers_what_is_not_refused() {
    let offered = |subject: &Subject<'_>| -> Vec<Action> {
        Action::ALL
            .into_iter()
            .filter(|a| verdict(subject, *a).is_offered())
            .collect()
    };
    assert_eq!(offered(&subject(Root, true)), vec![FreeUpSpace]);
    assert_eq!(
        offered(&subject(DeviceFolder, true)),
        vec![Trash, DeleteForever, FreeUpSpace, DeleteBackup]
    );
    assert_eq!(
        offered(&subject(Inside, false)),
        vec![Trash, DeleteForever, Share, FreeUpSpace]
    );
    assert_eq!(offered(&subject(Outside, true)).len(), 10);
}

const ROOT_ID: &str = "0b5ec0de-0000-4000-8000-0000000000b1";

fn row(db: &StateDb, file_id: &str, path: &str, parent_id: Option<&str>, kind: ItemKind) {
    let entry = FileEntry {
        file_id: file_id.into(),
        path: path.into(),
        status: FileStatus::CloudOnly,
        size_bytes: 3,
        modified_at: 0,
        content_hash: None,
        remote_updated_at: 0,
        parent_id: parent_id.map(str::to_string),
        item_kind: kind,
    };
    // A row and its contract (parent, kind) in one transaction: the writer the engine's pull uses.
    db.upsert_file_with_contract(&entry, |contract| {
        contract.parent_id = entry.parent_id.clone();
        contract.item_kind = entry.item_kind.clone();
    })
    .unwrap();
}

/// `Backups` (flagged) with this device's folder `GLegion` (manifest, `Pictures` on, `Music` off) and another
/// device's `Mac/Desktop`; beside it `Apple` and `zebra.txt`.
fn fixture() -> (tempfile::TempDir, StateDb, OwnBackups) {
    let dir = tempfile::tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    row(&db, ROOT_ID, "Backups", None, ItemKind::Folder);
    row(&db, "dev-own", "Backups/GLegion", Some(ROOT_ID), ItemKind::Folder);
    let manifest = format!("Backups/GLegion/{MANIFEST_NAME}");
    row(&db, "manifest", &manifest, Some("dev-own"), ItemKind::File);
    row(
        &db,
        "pictures",
        "Backups/GLegion/Pictures",
        Some("dev-own"),
        ItemKind::Folder,
    );
    row(
        &db,
        "photo",
        "Backups/GLegion/Pictures/a.jpg",
        Some("pictures"),
        ItemKind::File,
    );
    row(&db, "music", "Backups/GLegion/Music", Some("dev-own"), ItemKind::Folder);
    row(&db, "dev-mac", "Backups/Mac", Some(ROOT_ID), ItemKind::Folder);
    row(
        &db,
        "mac-desktop",
        "Backups/Mac/Desktop",
        Some("dev-mac"),
        ItemKind::Folder,
    );
    row(&db, "apple", "Apple", None, ItemKind::Folder);
    row(&db, "zebra", "zebra.txt", None, ItemKind::File);
    assert!(db.set_system_role(ROOT_ID, Some(SystemRole::BackupsRoot)).unwrap());
    let own = OwnBackups {
        device_folder: Some("GLegion".into()),
        folders: vec![OwnBackupFolder {
            display_name: "Pictures".into(),
            source_path: PathBuf::from("/home/a/Pictures"),
        }],
    };
    (dir, db, own)
}

#[test]
fn check_reads_the_rows_place_and_this_devices_own_backups() {
    let (_dir, db, own) = fixture();
    let ask = |id: &str, action: Action| check(&db, &own, id, action).unwrap().expect("the row exists");
    let own_words = no(&own_backup_refusal(
        &PathBuf::from("/home/a/Pictures").display().to_string(),
    ));
    assert_eq!(ask(ROOT_ID, Trash), no(MSG_DELETE));
    assert_eq!(ask(ROOT_ID, NewFolderIn), no(MSG_READ_ONLY));
    assert_eq!(ask("dev-mac", Trash), Verdict::UseDeleteBackup);
    assert_eq!(ask("dev-mac", DeleteBackup), Verdict::NeedsPassword);
    assert_eq!(ask("mac-desktop", DeleteBackup), Verdict::NeedsPassword);
    assert_eq!(ask("mac-desktop", Share), no(MSG_NO_FOLDER_SHARE));
    // This device's folder holds several backups, not one: it has no single original.
    assert_eq!(ask("dev-own", DeleteBackup), Verdict::NeedsPassword);
    // On: the original is named. Off (`Music`): a backup like any other device's.
    assert_eq!(ask("pictures", DeleteBackup), own_words);
    assert_eq!(ask("pictures", Trash), own_words);
    assert_eq!(ask("music", DeleteBackup), Verdict::NeedsPassword);
    let photo = no(&own_backup_refusal(
        &PathBuf::from("/home/a/Pictures").join("a.jpg").display().to_string(),
    ));
    assert_eq!(ask("photo", Edit), photo);
    assert_eq!(ask("photo", Share), Verdict::Allowed);
    assert_eq!(ask("manifest", Trash), no(MSG_READ_ONLY));
    assert_eq!(ask("apple", Rename), Verdict::Allowed);
    assert_eq!(ask("apple", UploadInto), Verdict::Allowed);
    assert_eq!(check(&db, &own, "no-such-row", Trash), Ok(None));
    assert_eq!(
        allowed_actions(&db, &own, ROOT_ID).unwrap().unwrap(),
        vec![FreeUpSpace],
        "the root's menu"
    );
    assert_eq!(allowed_actions(&db, &own, "zebra").unwrap().unwrap().len(), 10);
}

#[test]
fn the_listing_hides_the_manifest_marks_each_row_and_puts_the_root_first_in_every_sort() {
    let (_dir, db, _own) = fixture();
    for key in [SortKey::Name, SortKey::Modified, SortKey::Size, SortKey::Kind] {
        for dir in [SortDir::Asc, SortDir::Desc] {
            let (rows, total) = model::children_of(&db, None, Sort { key, dir }, 0, usize::MAX).unwrap();
            assert_eq!(total, 3);
            assert_eq!(rows[0].name, "Backups", "{key:?} {dir:?}: the root is the first row");
            assert!(rows[0].backup.is_protected);
            assert_eq!(rows[0].backup.position, Root);
            assert!(rows[1..].iter().all(|r| r.backup == BackupMark::default()));
        }
    }
    // Without the rule a descending name sort puts `zebra.txt`... after the folders, and `Backups` after `Apple`.
    let (ascending, _) = model::children_of(&db, None, Sort::default(), 0, usize::MAX).unwrap();
    let names: Vec<&str> = ascending.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, ["Backups", "Apple", "zebra.txt"]);

    let (device, total) = model::children_of(&db, Some("dev-own"), Sort::default(), 0, usize::MAX).unwrap();
    let names: Vec<&str> = device.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, ["Music", "Pictures"], "the manifest is not listed");
    assert_eq!(total, 2, "and not counted");
    assert!(
        device
            .iter()
            .all(|r| r.backup.position == CategoryFolder && !r.backup.is_protected)
    );
    let (devices, _) = model::children_of(&db, Some(ROOT_ID), Sort::default(), 0, usize::MAX).unwrap();
    assert!(devices.iter().all(|r| r.backup.position == DeviceFolder));
    // The web view's listing is as it was: no mark, the manifest listed, plain order.
    let listing = model::list(&db, Some("dev-own"), Sort::default(), false).unwrap();
    assert_eq!(listing.items.len(), 3);
    // The mark is not in the JSON the web view reads.
    let json = serde_json::to_value(&ascending[0]).unwrap();
    assert!(json.get("backup").is_none(), "{json}");
}
