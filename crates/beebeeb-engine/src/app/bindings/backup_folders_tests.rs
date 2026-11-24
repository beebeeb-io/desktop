//! Slice A7's engine rules (DW-0738, DW-1410) on the engine's own parts: the rule of what may be chosen, the settings'
//! add and removal, a real state database in a folder of the test's own (a layout, this device's backed-up folders,
//! pending uploads), and the removal's sequence with the move to Trash handed in. The shared config sandbox is never
//! written: the settings file is a `DesktopConfig` value here. Every name is invented.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::*;
use crate::account_binding::Identity;
use crate::app::backups_api::{FolderState, StateRead};
use crate::app::EngineConfig;
use crate::host::ChannelHost;
use crate::identity::TEST;
use crate::signin::memory_store::MemoryCredentials;
use crate::state_db::{BackupLayout, FileEntry, FileStatus, ItemKind, OperationKind, PendingOperation};

const ROOT: &str = "0193b1a6-0000-7000-8000-000000000001";
const DEVICE: &str = "0193b1a6-0000-7000-8000-000000000002";
const PROJECTS: &str = "0193b1a6-0000-7000-8000-000000000003";
const RECIPES: &str = "0193b1a6-0000-7000-8000-000000000004";
const DOCUMENTS: &str = "0193b1a6-0000-7000-8000-000000000005";
const OTHER_DEVICE: &str = "0193b1a6-0000-7000-8000-000000000006";
const OTHER_PROJECTS: &str = "0193b1a6-0000-7000-8000-000000000007";
const PROJECTS_AGAIN: &str = "0193b1a6-0000-7000-8000-000000000008";
/// A second folder named `Projects` in this device's backup (a kept backup and a new one share the name).
const PROJECTS_TWIN: &str = "0193b1a6-0000-7000-8000-000000000009";

/// The account the test's local state is bound to.
fn me() -> Identity {
    Identity::new(Some("u-invented"), None)
}

fn owned_db(dir: &Path) -> StateDb {
    let db = StateDb::open(dir.join("state.db")).unwrap();
    db.set_owner(&me()).unwrap();
    db
}

fn folder_row(db: &StateDb, file_id: &str, path: &str, parent_id: Option<&str>) {
    let entry = FileEntry {
        file_id: file_id.into(),
        path: path.into(),
        status: FileStatus::CloudOnly,
        size_bytes: 0,
        modified_at: 0,
        content_hash: None,
        remote_updated_at: 0,
        parent_id: parent_id.map(str::to_string),
        item_kind: ItemKind::Folder,
    };
    db.upsert_file_with_contract(&entry, |contract| {
        contract.parent_id = entry.parent_id.clone();
        contract.item_kind = entry.item_kind.clone();
    })
    .unwrap();
}

/// This device's backup (`Backups/Invented-Laptop/{Projects, Recipes, Documents}`) and another device's, which has a
/// `Projects` of its own.
fn laid_out(dir: &Path) -> StateDb {
    let db = owned_db(dir);
    db.set_backup_layout(&BackupLayout {
        root_id: ROOT.into(),
        device_folder_id: DEVICE.into(),
        device_folder_name: "Invented-Laptop".into(),
    })
    .unwrap();
    folder_row(&db, ROOT, "Backups", None);
    folder_row(&db, DEVICE, "Backups/Invented-Laptop", Some(ROOT));
    folder_row(&db, PROJECTS, "Backups/Invented-Laptop/Projects", Some(DEVICE));
    folder_row(&db, RECIPES, "Backups/Invented-Laptop/Recipes", Some(DEVICE));
    folder_row(&db, DOCUMENTS, "Backups/Invented-Laptop/Documents", Some(DEVICE));
    folder_row(&db, OTHER_DEVICE, "Backups/Invented-Mac", Some(ROOT));
    folder_row(&db, OTHER_PROJECTS, "Backups/Invented-Mac/Projects", Some(OTHER_DEVICE));
    db
}

/// The tag an earlier build wrote for a folder added while its owner's id was not known: the plain digest of
/// `email:<canonical address>`, spelled out here (not through the code under test).
fn old_address_tag(canonical: &str) -> String {
    crate::hexfmt::encode(&Sha256::digest(format!("email:{canonical}").as_bytes()))
}

/// Known folders' locations for the tests: Documents at `documents`, every other known folder nowhere.
fn documents_at(documents: &Path) -> impl Fn(&str) -> KnownFolderPaths + '_ {
    move |key| KnownFolderPaths {
        default: (key == "documents").then(|| documents.to_path_buf()),
        current: None,
    }
}

/// A folder [`me`] chose.
fn chosen(name: &str, path: PathBuf) -> ChosenFolder {
    ChosenFolder {
        name: name.into(),
        path,
        owner: owner_tag(&me()),
        folder_id: None,
    }
}

/// A folder [`me`] chose whose pass recorded its backup's folder `folder_id`.
fn passed(name: &str, path: PathBuf, folder_id: &str) -> ChosenFolder {
    ChosenFolder {
        folder_id: Some(folder_id.into()),
        ..chosen(name, path)
    }
}

/// Settings holding `folders`, each on.
fn settings_with(folders: &[&ChosenFolder]) -> DesktopConfig {
    let mut config = DesktopConfig::default();
    for folder in folders {
        config.backup_chosen_folders.push((*folder).clone());
        config.set_known_folder(&folder.key(), true);
    }
    config
}

fn backup_op(id: &str, key: &str) -> PendingOperation {
    PendingOperation {
        op_id: id.into(),
        kind: OperationKind::UploadVersion,
        file_id: Some(format!("file-{id}")),
        parent_id: None,
        target_path: Some(format!("/{id}.bin")),
        metadata_json: Some(r#"{"operation":"upload_version"}"#.into()),
        payload_path: None,
        base_version: None,
        base_object_version_id: None,
        attempts: 0,
        max_attempts: 5,
        next_retry_at: 0,
        last_error: None,
        backup_source_key: Some(key.into()),
        created_at: 100,
        updated_at: 100,
    }
}

fn pending(db: &StateDb) -> Vec<String> {
    db.list_due_operations(999)
        .unwrap()
        .into_iter()
        .map(|op| op.op_id)
        .collect()
}

/// The engine's `leave` step, on a settings value instead of the file.
fn leave_from<'a>(
    config: &'a RefCell<DesktopConfig>,
    db: &'a StateDb,
    key: &'a str,
) -> impl FnOnce() -> Result<Removed, BackupsError> + 'a {
    move || {
        let removed = remove_from(&mut config.borrow_mut(), key).ok_or(BackupsError::UnknownFolder)?;
        leave_set(&TEST, db, key)?;
        Ok(removed)
    }
}

/// Every file under `dir` with its bytes.
fn snapshot(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut found = BTreeMap::new();
    let mut todo = vec![dir.to_path_buf()];
    while let Some(next) = todo.pop() {
        for entry in std::fs::read_dir(&next).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                todo.push(path);
            } else {
                found.insert(path.clone(), std::fs::read(&path).unwrap());
            }
        }
    }
    found
}

// ── Rule 1: Beebeeb's own folder and anything inside it (DW-0738) ─────────────────────────────────────────────────

/// The sync folder itself can't be chosen. (The sync folder is not named like the app's own folder here, so only the
/// inside-the-root check can refuse it as inside: the mutant that drops that check answers `HoldsSyncFolder`.)
#[test]
fn the_sync_root_cannot_be_chosen() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("Invented Drive");
    std::fs::create_dir_all(&root).unwrap();
    assert_eq!(check_chosen(&root, &root, &[]), Err(AddRefused::InsideSyncFolder));
    assert_eq!(
        AddRefused::InsideSyncFolder.words(),
        "Beebeeb's own folder, and any folder inside it, can't be chosen. Its files are in Beebeeb already."
    );
}

/// A folder inside the sync folder can't be chosen, at any depth; a sibling whose name only starts the same way can,
/// and a folder that holds the sync folder can't (its own refusal).
#[test]
fn a_folder_inside_the_sync_root_cannot_be_chosen() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("Invented Drive");
    let inside = root.join("Photos").join("2026");
    let sibling = dir.path().join("Invented Drive Photos");
    for folder in [&inside, &sibling] {
        std::fs::create_dir_all(folder).unwrap();
    }
    assert_eq!(check_chosen(&inside, &root, &[]), Err(AddRefused::InsideSyncFolder));
    assert_eq!(
        check_chosen(&root.join("Photos"), &root, &[]),
        Err(AddRefused::InsideSyncFolder)
    );
    assert_eq!(
        check_chosen(&sibling, &root, &[]),
        Ok(()),
        "a name prefix is not a folder"
    );
    assert_eq!(check_chosen(dir.path(), &root, &[]), Err(AddRefused::HoldsSyncFolder));
}

/// A folder backed up already, a file, a folder that is not there and a relative path are refused too.
#[test]
fn a_folder_backed_up_already_or_not_a_folder_cannot_be_chosen() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("Invented Drive");
    let projects = dir.path().join("Projects");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&projects).unwrap();
    std::fs::write(dir.path().join("notes.txt"), b"invented").unwrap();
    assert_eq!(
        check_chosen(&projects, &root, std::slice::from_ref(&projects)),
        Err(AddRefused::AlreadyBackedUp)
    );
    assert_eq!(
        check_chosen(&dir.path().join("notes.txt"), &root, &[]),
        Err(AddRefused::NotAFolder)
    );
    assert_eq!(
        check_chosen(&dir.path().join("Gone"), &root, &[]),
        Err(AddRefused::NotAFolder)
    );
    assert_eq!(
        check_chosen(Path::new("Projects"), &root, &[]),
        Err(AddRefused::NotAFolder)
    );
    assert_eq!(check_chosen(&projects, &root, &[]), Ok(()));
}

/// An add names the folder once (core's custom name, clear of the names already in this device's backup) and switches
/// it on; the same folder again is refused and changes nothing.
#[test]
fn an_add_names_the_folder_once_and_switches_it_on() {
    let dir = tempfile::tempdir().unwrap();
    let projects = dir.path().join("Projects");
    std::fs::create_dir_all(&projects).unwrap();
    let mut config = DesktopConfig::default();
    let key = add_to(&mut config, &projects, &["Projects".to_string()], &me()).unwrap();
    assert_eq!(
        key, "custom:Projects (2)",
        "a kept backup of an earlier Projects is never written into"
    );
    assert_eq!(
        config.backup_chosen_folders,
        vec![chosen("Projects (2)", projects.clone())]
    );
    assert!(config.known_folder_enabled(&key));
    let before = config.backup_chosen_folders.clone();
    assert_eq!(
        add_to(&mut config, &projects, &[], &me()),
        Err(AddRefused::AlreadyBackedUp)
    );
    assert_eq!(config.backup_chosen_folders, before);
}

// ── Rule 2: Stop backing up keeps, or moves only that folder's backup (DW-1410) ───────────────────────────────────

/// "Stop and keep the backup": the folder leaves the backup set and its pending uploads go, its backup stays in
/// Backups (nothing is sent to Trash), and another chosen folder is untouched.
#[test]
fn remove_keep_leaves_the_copy() {
    let dir = tempfile::tempdir().unwrap();
    let db = laid_out(dir.path());
    let projects = passed("Projects", dir.path().join("Projects"), PROJECTS);
    let recipes = passed("Recipes", dir.path().join("Recipes"), RECIPES);
    let config = RefCell::new(settings_with(&[&projects, &recipes]));
    db.enqueue_operation(&backup_op("p-1", &projects.key())).unwrap();
    db.enqueue_operation(&backup_op("r-1", &recipes.key())).unwrap();
    let key = projects.key();
    let trashed = RefCell::new(Vec::new());

    let removed = remove_steps(
        None,
        Some(PROJECTS),
        leave_from(&config, &db, &key),
        |copy, _| {
            trashed.borrow_mut().push(copy.to_string());
            Ok(())
        },
        |_: Removed| panic!("nothing is put back"),
    );

    assert_eq!(removed, Ok(()));
    assert!(trashed.borrow().is_empty(), "nothing is moved to Trash");
    let config = config.into_inner();
    assert_eq!(config.backup_chosen_folders, vec![recipes.clone()]);
    assert!(!config.known_folder_enabled(&key), "it is no longer backed up");
    assert!(config.known_folder_enabled(&recipes.key()));
    assert_eq!(
        pending(&db),
        vec!["r-1".to_string()],
        "only its own pending uploads went"
    );
    assert_eq!(
        backup_copy_of(&db, &projects).unwrap().as_deref(),
        Some(PROJECTS),
        "its backup is still in Backups"
    );
}

/// "Stop and move the backup to Trash": exactly that folder's backup on this device is the one sent, after its folder
/// left the set; another chosen folder's backup, the device's folder and another device's same-named folder are not.
/// A password the server refuses puts the folder back; an answer that may have reached Beebeeb leaves it out.
#[test]
fn remove_trash_moves_only_that_folders_copy() {
    let dir = tempfile::tempdir().unwrap();
    let db = laid_out(dir.path());
    let projects = passed("Projects", dir.path().join("Projects"), PROJECTS);
    let recipes = passed("Recipes", dir.path().join("Recipes"), RECIPES);
    assert_eq!(backup_copy_of(&db, &projects).unwrap().as_deref(), Some(PROJECTS));
    assert_eq!(backup_copy_of(&db, &recipes).unwrap().as_deref(), Some(RECIPES));
    assert_eq!(
        backup_copy_of(&db, &chosen("Never passed", dir.path().join("New"))).unwrap(),
        None
    );
    let key = projects.key();

    // Sent: only Projects, with the password.
    let config = RefCell::new(settings_with(&[&projects, &recipes]));
    let trashed = RefCell::new(Vec::new());
    let copy = backup_copy_of(&db, &projects).unwrap();
    let removed = remove_steps(
        Some(b"invented password"),
        copy.as_deref(),
        leave_from(&config, &db, &key),
        |copy, password| {
            // The folder left the set before its backup is sent.
            assert!(!config.borrow().known_folder_enabled(&key));
            trashed.borrow_mut().push((copy.to_string(), password.to_vec()));
            Ok(())
        },
        |_: Removed| panic!("nothing is put back"),
    );
    assert_eq!(removed, Ok(()));
    assert_eq!(
        trashed.into_inner(),
        vec![(PROJECTS.to_string(), b"invented password".to_vec())]
    );
    assert_eq!(config.borrow().backup_chosen_folders, vec![recipes.clone()]);
    assert!(config.borrow().known_folder_enabled(&recipes.key()));

    // The password refused: nothing was sent, the folder is back where it was, on.
    let config = RefCell::new(settings_with(&[&projects, &recipes]));
    let removed = remove_steps(
        Some(b"wrong"),
        copy.as_deref(),
        leave_from(&config, &db, &key),
        |_, _| Err(DeleteBackupError::WrongPassword),
        |removed: Removed| put_back_into(&mut config.borrow_mut(), &removed),
    );
    assert_eq!(
        removed,
        Err(RemoveRefused::PasswordRefused(DeleteBackupError::WrongPassword))
    );
    assert_eq!(
        config.borrow().backup_chosen_folders,
        vec![projects.clone(), recipes.clone()]
    );
    assert!(config.borrow().known_folder_enabled(&key));
    assert_eq!(
        RemoveRefused::PasswordRefused(DeleteBackupError::WrongPassword).words(),
        "That password isn't right. Nothing was changed."
    );

    // No answer: it may have moved, so the folder stays out of the set, and the words say so.
    let config = RefCell::new(settings_with(&[&projects, &recipes]));
    let removed = remove_steps(
        Some(b"invented password"),
        copy.as_deref(),
        leave_from(&config, &db, &key),
        |_, _| Err(DeleteBackupError::Offline),
        |_: Removed| panic!("a move that may have happened is not undone"),
    );
    assert_eq!(removed, Err(RemoveRefused::TrashNotDone(DeleteBackupError::Offline)));
    assert!(!config.borrow().known_folder_enabled(&key));
}

/// A chosen folder's backup is the folder whose id its entry holds, never another folder of its name: with a kept
/// backup `Projects` and the folder's own `Projects` both in this device's backup, "Stop and move the backup to Trash"
/// sends the folder's own. With no id (it never passed) there is none, whatever folders of its name the rows hold; an
/// id the rows do not hold under this device's folder (another device's) is none too.
#[test]
fn the_copy_is_the_folder_whose_id_it_holds_never_a_twin_of_its_name() {
    let dir = tempfile::tempdir().unwrap();
    let db = laid_out(dir.path());
    folder_row(&db, PROJECTS_TWIN, "Backups/Invented-Laptop/Projects", Some(DEVICE));
    let mut projects = passed("Projects", dir.path().join("Projects"), PROJECTS_TWIN);
    assert_eq!(backup_copy_of(&db, &projects).unwrap().as_deref(), Some(PROJECTS_TWIN));
    projects.folder_id = None;
    assert_eq!(backup_copy_of(&db, &projects).unwrap(), None, "no id: no backup");
    projects.folder_id = Some(OTHER_PROJECTS.into());
    assert_eq!(backup_copy_of(&db, &projects).unwrap(), None, "another device's folder");
}

/// An add starts a new backup: a folder the local state remembers for its key (a removed folder of the same name
/// passed into it) is forgotten, so the new folder's pass never continues in that kept backup.
#[test]
fn an_add_forgets_the_folder_a_removed_one_of_its_name_had() {
    let dir = tempfile::tempdir().unwrap();
    let db = laid_out(dir.path());
    db.set_backup_category_folder("custom:Projects", Some(PROJECTS))
        .unwrap();
    start_over(&TEST, &db, "custom:Projects").unwrap();
    assert_eq!(db.backup_category_folder("custom:Projects").unwrap(), None);
}

// ── Rule 3: nothing on this PC is touched ─────────────────────────────────────────────────────────────────────────

/// Adding a folder, then both ways of stopping it, read nothing but whether it is a folder and change nothing in it:
/// every file is still there with the same bytes.
#[test]
fn remove_never_touches_local_files() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("Invented Drive");
    let folder = dir.path().join("Projects");
    std::fs::create_dir_all(folder.join("Drafts")).unwrap();
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(folder.join("plan.txt"), b"the person's plan").unwrap();
    std::fs::write(folder.join("Drafts").join("first.txt"), b"a first draft").unwrap();
    let before = snapshot(&folder);
    assert_eq!(before.len(), 2);
    let db = laid_out(dir.path());

    for password in [None, Some(&b"invented password"[..])] {
        let mut config = DesktopConfig::default();
        check_chosen(&folder, &root, &taken_paths(&config, &me())).unwrap();
        let key = add_to(&mut config, &folder, &backup_folder_names(&db).unwrap(), &me()).unwrap();
        start_over(&TEST, &db, &key).unwrap();
        let mut added = config.backup_chosen_folders[0].clone();
        if password.is_some() {
            // It passed once: its backup is in this device's backup, so the move to Trash has one to move (P3-9).
            folder_row(
                &db,
                PROJECTS_AGAIN,
                "Backups/Invented-Laptop/Projects (2)",
                Some(DEVICE),
            );
            added.folder_id = Some(PROJECTS_AGAIN.into());
        }
        let config = RefCell::new(config);
        let copy = backup_copy_of(&db, &added).unwrap();
        assert_eq!(copy.is_some(), password.is_some());
        let removed = remove_steps(
            password,
            copy.as_deref(),
            leave_from(&config, &db, &key),
            |_, _| Ok(()),
            |_: Removed| {},
        );
        assert_eq!(removed, Ok(()));
        assert!(folder.is_dir(), "the folder is still on this PC");
        assert_eq!(snapshot(&folder), before, "every file is still there, unchanged");
    }
}

// ── The runner and the list take a chosen folder like a known one ─────────────────────────────────────────────────

/// A chosen folder is passed while it is on and still a folder outside the sync folder; its row says the same, and a
/// folder gone from this PC stays listed (so it can be removed) as unavailable and not on.
#[test]
fn a_chosen_folder_is_passed_and_listed_like_a_known_one() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("Invented Drive");
    let projects = chosen("Projects", dir.path().join("Projects"));
    let gone = chosen("Gone", dir.path().join("Gone"));
    let off = chosen("Recipes", dir.path().join("Recipes"));
    let moved_in = chosen("Inside", root.join("Inside"));
    for folder in [&projects.path, &off.path, &moved_in.path] {
        std::fs::create_dir_all(folder).unwrap();
    }
    let all = [projects.clone(), gone.clone(), off.clone(), moved_in.clone()];
    let enabled = vec![projects.key(), gone.key(), moved_in.key()];

    let passed = crate::backup::sources::chosen_sources(&all, &enabled, Some(&root));
    let passed: Vec<_> = passed.iter().map(|s| (s.category.key(), s.path.clone())).collect();
    assert_eq!(passed, vec![(projects.key(), projects.path.clone())]);

    let state = StateRead {
        last_pass: BTreeMap::from([(projects.key(), 1_700_000_000_000)]),
        ..StateRead::default()
    };
    let rows = crate::app::backups_api::chosen_rows(&all[..3], &enabled, &state, |_| Some(46));
    let read: Vec<_> = rows
        .iter()
        .map(|r| (r.key.as_str(), r.on, r.state, r.items, r.chosen_by_you()))
        .collect();
    assert_eq!(
        read,
        vec![
            (projects.key().as_str(), true, FolderState::On, Some(46), true),
            (gone.key().as_str(), false, FolderState::Unavailable, None, true),
            (off.key().as_str(), false, FolderState::Off, None, true),
        ]
    );
    assert_eq!(rows[0].last_pass_at_ms, Some(1_700_000_000_000));
    assert_eq!(rows[0].name, "Projects");
}

// ── Session-bound, and S11 ────────────────────────────────────────────────────────────────────────────────────────

/// Asked for a session that is not the one in memory, or with no one signed in, nothing is added, removed or chosen,
/// and the settings file is not written.
#[test]
fn a_call_for_another_session_writes_nothing() {
    let state_dir = crate::state_paths::init_for_test();
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "an-invented-slot".into(),
        state_dir,
        api_base: "http://127.0.0.1:9".into(),
    };
    let engine = Arc::new(Engine::open(&TEST, config, host, MemoryCredentials::default()).expect("the handle opens"));
    let dir = tempfile::tempdir().unwrap();
    let before = DesktopConfig::load_for(&TEST)
        .ok()
        .map(|c| (c.backup_chosen_folders, c.known_folder_backup, c.known_folder_source));

    let another = SessionGeneration::for_test(u64::MAX);
    assert_ne!(another, engine.session_turn());
    assert_eq!(
        engine.add_backup_folder_for(another, dir.path()),
        Err(AddRefused::Backups(BackupsError::Refused(
            LocalStateRefused::SessionChanged
        )))
    );
    assert_eq!(
        engine.add_backup_folder_for(engine.session_turn(), dir.path()),
        Err(AddRefused::Backups(BackupsError::Refused(LocalStateRefused::SignedOut)))
    );
    assert_eq!(
        engine.remove_backup_folder_for(another, "custom:Projects", Some(b"invented password")),
        Err(RemoveRefused::Backups(BackupsError::Refused(
            LocalStateRefused::SessionChanged
        )))
    );
    // P3-14: bound to the session of the press. A key that is not one of the six is refused first (nothing read);
    // then a session that is not the one in memory, before the local state is asked (that would answer `SignedOut`).
    assert_eq!(
        engine.choose_backup_source_for(another, "not-a-folder", KnownFolderSource::OneDrive),
        Err(BackupsError::UnknownFolder)
    );
    assert_eq!(
        engine.choose_backup_source_for(another, "documents", KnownFolderSource::OneDrive),
        Err(BackupsError::Refused(LocalStateRefused::SessionChanged))
    );
    assert_eq!(
        engine.choose_backup_source_for(engine.session_turn(), "documents", KnownFolderSource::OneDrive),
        Err(BackupsError::Refused(LocalStateRefused::SignedOut))
    );
    let after = DesktopConfig::load_for(&TEST)
        .ok()
        .map(|c| (c.backup_chosen_folders, c.known_folder_backup, c.known_folder_source));
    assert_eq!(before, after, "the settings file is not written for a refusal");
}

/// S11: the answer for a moved known folder settles its question in the state database.
#[test]
fn choosing_a_moved_folders_source_settles_its_question() {
    let dir = tempfile::tempdir().unwrap();
    let db = owned_db(dir.path());
    assert!(db
        .ask_backup_source_question("documents", "/invented/Documents", "/invented/OneDrive/Documents", 5)
        .unwrap());
    assert_eq!(db.backup_source_questions().unwrap().len(), 1);
    source_chosen(&TEST, &db, "documents").unwrap();
    assert!(
        db.backup_source_questions().unwrap().is_empty(),
        "the question is settled"
    );
}

/// Never backed up twice: a folder inside one that is backed up (a chosen one, or a known folder's place such as
/// Documents) and a folder that holds one are refused, at any depth; a sibling whose name only starts the same way is
/// not nested. The settings' own add refuses it too (a racing add).
#[test]
fn a_folder_inside_or_holding_a_backed_up_folder_cannot_be_chosen() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("Invented Drive");
    let documents = dir.path().join("Documents");
    let deep = documents.join("Work").join("2026");
    let parent = dir.path().join("Everything");
    let held = parent.join("Notes");
    let sibling = dir.path().join("Documents Old");
    for folder in [&root, &deep, &held, &sibling] {
        std::fs::create_dir_all(folder).unwrap();
    }
    let taken = [documents.clone(), held.clone()];
    assert_eq!(check_chosen(&deep, &root, &taken), Err(AddRefused::InsideBackedUp));
    assert_eq!(
        check_chosen(&documents.join("Work"), &root, &taken),
        Err(AddRefused::InsideBackedUp)
    );
    assert_eq!(check_chosen(&parent, &root, &taken), Err(AddRefused::HoldsBackedUp));
    assert_eq!(
        check_chosen(&documents, &root, &taken),
        Err(AddRefused::AlreadyBackedUp)
    );
    assert_eq!(
        check_chosen(&sibling, &root, &taken),
        Ok(()),
        "a name prefix is not nesting"
    );

    let mut config = DesktopConfig::default();
    add_to(&mut config, &documents, &[], &me()).unwrap();
    assert_eq!(add_to(&mut config, &deep, &[], &me()), Err(AddRefused::InsideBackedUp));
    assert_eq!(
        add_to(&mut config, dir.path(), &[], &me()),
        Err(AddRefused::HoldsBackedUp)
    );
    assert_eq!(config.backup_chosen_folders.len(), 1, "a refused add writes nothing");
    assert_eq!(
        AddRefused::InsideBackedUp.words(),
        "This folder is inside a folder that is backed up already."
    );
}

// ── Review of #463: P3-8 (per account), P3-9, P3-10, P3-12 ─────────────────────────────────────────────────────────

/// P3-8: a chosen folder is its account's alone. Only that account's folders are passed, listed and removed; a tagged
/// folder of A is never passed for B. An untagged folder (written by a test build before folders were per account) is
/// nobody's, and its folder can be added again under a name of its own. The tag holds
/// neither the id nor the email in clear.
#[test]
fn chosen_folders_are_the_choosing_accounts_alone() {
    let dir = tempfile::tempdir().unwrap();
    let other = Identity::new(Some("u-other"), Some("other@example.invalid"));
    let mine = chosen("Projects", dir.path().join("Projects"));
    let theirs = ChosenFolder {
        owner: owner_tag(&other),
        ..chosen("Recipes", dir.path().join("Recipes"))
    };
    let earlier = ChosenFolder {
        owner: None,
        ..chosen("Letters", dir.path().join("Letters"))
    };
    let all = vec![mine.clone(), theirs.clone(), earlier.clone()];

    // Each account its own; nobody without an owner; an untagged folder for no account.
    assert_eq!(chosen_for(&all, Some(&me())), vec![mine.clone()]);
    assert_eq!(chosen_for(&all, Some(&other)), vec![theirs.clone()]);
    assert_eq!(chosen_for(&all, None), Vec::<ChosenFolder>::new());
    assert!(!is_owners(&earlier, Some(&me())) && !is_owners(&earlier, Some(&other)));

    // P3-5: no tag is made from an address, and an earlier build's address tag is nobody's on a read (its own
    // account's start re-tags it: `an_address_tag_is_retagged_by_its_own_accounts_id_alone`).
    let by_email = Identity::new(None, Some("Other@Example.invalid"));
    assert_eq!(owner_tag(&by_email), None, "no tag from an address");
    let tagged_by_email = ChosenFolder {
        owner: Some(old_address_tag("other@example.invalid")),
        ..chosen("Maps", dir.path().join("Maps"))
    };
    assert!(!is_owners(&tagged_by_email, Some(&other)));
    assert!(!is_owners(&tagged_by_email, Some(&me())));
    let tag = owner_tag(&other).unwrap();
    assert!(
        !tag.contains("u-other") && !tag.contains("example"),
        "no id or email in clear"
    );
    assert_eq!(tag.len(), 64);

    // An add is tagged as its account's; its name is clear of every name in the file (their switch keys share one
    // backup set); the same folder is "backed up already" only for the same account.
    let shared = dir.path().join("Recipes");
    std::fs::create_dir_all(&shared).unwrap();
    let mut config = settings_with(&[&theirs]);
    let key = add_to(&mut config, &shared, &[], &me()).unwrap();
    assert_eq!(key, "custom:Recipes (2)");
    assert_eq!(config.backup_chosen_folders[1].owner, owner_tag(&me()));
    assert_eq!(
        add_to(&mut config, &shared, &[], &me()),
        Err(AddRefused::AlreadyBackedUp)
    );
    assert_eq!(
        add_to(&mut config, &shared, &[], &Identity::new(None, None)),
        Err(AddRefused::Backups(BackupsError::Refused(
            LocalStateRefused::IdentityUnknown
        ))),
        "an owner without a tag writes nothing"
    );
    assert_eq!(config.backup_chosen_folders.len(), 2);
    assert_eq!(taken_paths(&config, &other).first(), Some(&shared));

    // The untagged folder's own folder is added again: not "backed up already", and under a name of its own, so it
    // never shares the untagged entry's switch key.
    let letters = dir.path().join("Letters");
    std::fs::create_dir_all(&letters).unwrap();
    let mut config = settings_with(&[&earlier]);
    assert!(!taken_paths(&config, &me()).contains(&letters));
    let key = add_to(&mut config, &letters, &[], &me()).unwrap();
    assert_eq!(key, "custom:Letters (2)");
    assert_eq!(chosen_for(&config.backup_chosen_folders, Some(&me())).len(), 1);
}

/// P3-8, the runner's read: only the folders of the account this local data is bound to are passed, never an untagged
/// one, not even for that owner; another account's local data passes its own; a local state with no owner recorded
/// passes nothing.
#[test]
fn the_runner_passes_the_local_datas_owners_folders_alone() {
    let dir = tempfile::tempdir().unwrap();
    let other = Identity::new(Some("u-other"), None);
    let mine = chosen("Projects", dir.path().join("Projects"));
    let theirs = ChosenFolder {
        owner: owner_tag(&other),
        ..chosen("Recipes", dir.path().join("Recipes"))
    };
    let earlier = ChosenFolder {
        owner: None,
        ..chosen("Letters", dir.path().join("Letters"))
    };
    let config = settings_with(&[&mine, &theirs, &earlier]);

    let a = tempfile::tempdir().unwrap();
    assert_eq!(runner_chosen(&owned_db(a.path()), &config), vec![mine.clone()]);
    let b = tempfile::tempdir().unwrap();
    let theirs_db = StateDb::open(b.path().join("state.db")).unwrap();
    theirs_db.set_owner(&other).unwrap();
    assert_eq!(runner_chosen(&theirs_db, &config), vec![theirs.clone()]);
    let c = tempfile::tempdir().unwrap();
    let unowned = StateDb::open(c.path().join("state.db")).unwrap();
    assert_eq!(runner_chosen(&unowned, &config), Vec::<ChosenFolder>::new());
}

/// P3-9: "Stop and move the backup to Trash" for a folder whose backup this PC has not found is refused before
/// anything: the folder is not taken out of the set, nothing is sent, nothing is answered as done.
#[test]
fn trash_without_a_found_backup_changes_nothing() {
    let removed = remove_steps(
        Some(b"invented password"),
        None,
        || -> Result<(), BackupsError> { panic!("the folder is not taken out of the set") },
        |_, _| panic!("nothing is sent"),
        |_: ()| panic!("nothing to put back"),
    );
    assert_eq!(removed, Err(RemoveRefused::NoCopyFound));
    assert!(!RemoveRefused::NoCopyFound.stopped_backing_up());
    assert_eq!(
        RemoveRefused::NoCopyFound.words(),
        crate::os_words::for_this_os(
            "This PC hasn't found this folder's backup yet, so it can't be moved to Trash. Nothing was changed."
        )
    );
    // "Stop and keep the backup" needs no backup found: the folder leaves the set, nothing is sent.
    let left = std::cell::Cell::new(false);
    let kept = remove_steps(
        None,
        None,
        || -> Result<(), BackupsError> {
            left.set(true);
            Ok(())
        },
        |_, _| panic!("nothing is sent"),
        |_: ()| panic!("nothing to put back"),
    );
    assert_eq!(kept, Ok(()));
    assert!(left.get());
}

/// P3-12: a move to Trash answered "not found" moved nothing (refused before it was sent, or the server found nothing
/// to move), so the folder is put back as it was and the words say nothing changed.
#[test]
fn a_not_found_puts_the_folder_back() {
    let dir = tempfile::tempdir().unwrap();
    let db = laid_out(dir.path());
    let projects = passed("Projects", dir.path().join("Projects"), PROJECTS);
    let key = projects.key();
    let config = RefCell::new(settings_with(&[&projects]));
    let copy = backup_copy_of(&db, &projects).unwrap();
    let removed = remove_steps(
        Some(b"invented password"),
        copy.as_deref(),
        leave_from(&config, &db, &key),
        |_, _| Err(DeleteBackupError::NotFound),
        |removed: Removed| put_back_into(&mut config.borrow_mut(), &removed),
    );
    assert_eq!(
        removed,
        Err(RemoveRefused::PasswordRefused(DeleteBackupError::NotFound))
    );
    assert_eq!(config.borrow().backup_chosen_folders, vec![projects.clone()]);
    assert!(config.borrow().known_folder_enabled(&key), "back on, as it was");
    assert!(!removed.unwrap_err().stopped_backing_up());
}

/// P3-10: before this PC knows its backup layout, a new folder's name is kept clear of every name the local rows know
/// in any device's backup, so it never takes the name of an earlier folder that was stopped and kept (its pass would
/// not adopt that folder, which it finds by its own id alone, but two folders of one name would show side by side).
#[test]
fn with_no_layout_a_new_name_skips_every_devices_backup_names() {
    let dir = tempfile::tempdir().unwrap();
    let db = owned_db(dir.path());
    assert!(db.backup_layout().unwrap().is_none());
    assert_eq!(
        backup_folder_names(&db).unwrap(),
        Vec::<String>::new(),
        "no rows: no name"
    );
    folder_row(&db, ROOT, "Backups", None);
    assert!(db.mark_backups_root_for_test(ROOT).unwrap());
    folder_row(&db, DEVICE, "Backups/Invented-Laptop", Some(ROOT));
    folder_row(&db, PROJECTS, "Backups/Invented-Laptop/Projects", Some(DEVICE));
    folder_row(&db, OTHER_DEVICE, "Backups/Invented-Mac", Some(ROOT));
    folder_row(&db, OTHER_PROJECTS, "Backups/Invented-Mac/Recipes", Some(OTHER_DEVICE));
    // A folder called "Backups" without the role is an ordinary folder: its subfolders are not backups.
    folder_row(&db, RECIPES, "Elsewhere", None);
    folder_row(&db, DOCUMENTS, "Elsewhere/Documents", Some(RECIPES));
    assert_eq!(backup_folder_names(&db).unwrap(), ["Projects", "Recipes"]);

    let projects = dir.path().join("Projects");
    std::fs::create_dir_all(&projects).unwrap();
    let mut config = DesktopConfig::default();
    let key = add_to(&mut config, &projects, &backup_folder_names(&db).unwrap(), &me()).unwrap();
    assert_eq!(key, "custom:Projects (2)", "the kept Projects is never written into");
}

// ── Review of #464: P3-3 (the rule follows the known folder's switch), P3-5 (c) (tags by id alone), P3-6 ───────────

/// P3-6: the rule's one comparison. The same folder, inside, holding, or apart (a sibling whose name only starts the
/// same way); the same folder is found before a folder that holds it, whatever their order.
#[test]
fn nesting_is_the_same_folder_inside_or_holding_and_the_same_comes_first() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("Invented Drive");
    let home = dir.path().join("Home");
    let documents = home.join("Documents");
    let work = documents.join("Work");
    let sibling = home.join("Documents Old");
    for folder in [&root, &work, &sibling] {
        std::fs::create_dir_all(folder).unwrap();
    }
    assert_eq!(nesting(&documents, &documents), Some(Nesting::Same));
    assert_eq!(nesting(&work, &documents), Some(Nesting::Inside));
    assert_eq!(nesting(&home, &documents), Some(Nesting::Holds));
    assert_eq!(nesting(&sibling, &documents), None, "a name prefix is not nesting");
    assert_eq!(nesting(&documents, Path::new("Documents")), None);

    let taken = [home.clone(), documents.clone()];
    let found = nested_with(&documents, &taken, PathBuf::as_path);
    assert_eq!(found, Some((Nesting::Same, &documents)), "the same folder first");
    assert_eq!(
        check_chosen(&documents, &root, &taken),
        Err(AddRefused::AlreadyBackedUp)
    );
}

/// P3-3, the add: a known folder that is off backs nothing up, so a folder inside it, or one that holds it, may be
/// chosen; while it is on, both are refused, and the add's re-check under the settings lock reads the switch from the
/// file it holds. The known folder itself is switched, never added: refused with words that are true whether it is
/// on or off.
#[test]
fn a_folder_inside_a_known_folder_follows_its_switch() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("Invented Drive");
    let home = dir.path().join("Home");
    let documents = home.join("Documents");
    let projects = documents.join("Projects");
    for folder in [&root, &projects] {
        std::fs::create_dir_all(folder).unwrap();
    }
    let paths = || documents_at(&documents);

    // Off: inside it, and holding it, may be chosen; the folder itself is "in the list already".
    let mut off = DesktopConfig::default();
    let taken = taken_paths_with(&off, &me(), paths());
    assert!(taken.is_empty(), "a known folder that is off takes no folder");
    assert_eq!(check_chosen(&projects, &root, &taken), Ok(()));
    assert_eq!(check_chosen(&home, &root, &taken), Ok(()));
    assert_eq!(
        add_to_with(&mut off.clone(), &documents, &[], &me(), paths()),
        Err(AddRefused::KnownFolderOff)
    );
    assert_eq!(
        AddRefused::KnownFolderOff.words(),
        "This folder is in the list already. Switch it on there to back it up."
    );
    let key = add_to_with(&mut off, &projects, &[], &me(), paths()).unwrap();
    assert_eq!(key, "custom:Projects");

    // On: refused both ways, and the folder itself is backed up already.
    let mut on = DesktopConfig::default();
    on.set_known_folder("documents", true);
    let taken = taken_paths_with(&on, &me(), paths());
    assert_eq!(taken, vec![documents.clone()]);
    assert_eq!(check_chosen(&projects, &root, &taken), Err(AddRefused::InsideBackedUp));
    assert_eq!(check_chosen(&home, &root, &taken), Err(AddRefused::HoldsBackedUp));
    assert_eq!(
        check_chosen(&documents, &root, &taken),
        Err(AddRefused::AlreadyBackedUp)
    );

    // The rule ran while Documents was off, and the switch landed before the add's write: the re-check refuses.
    assert_eq!(
        add_to_with(&mut on, &projects, &[], &me(), paths()),
        Err(AddRefused::InsideBackedUp)
    );
    assert_eq!(
        add_to_with(&mut on, &home, &[], &me(), paths()),
        Err(AddRefused::HoldsBackedUp)
    );
    assert!(on.backup_chosen_folders.is_empty(), "a refused add writes nothing");
    assert_eq!(
        AddRefused::InsideBackedUp.words(),
        "This folder is inside a folder that is backed up already."
    );
}

/// P3-3, the switch: a known folder is not switched on while a chosen folder of the account's inside it (or holding
/// it) is backed up, and the refusal names that folder. A stopped chosen folder does not block, and "Back up again" on
/// it is then refused while the known folder is on, naming the known folder. Another account's chosen folder and an
/// untagged one are never compared nor named.
#[test]
fn a_known_folder_is_not_switched_on_over_a_chosen_folder_that_is_backed_up() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("Home");
    let documents = home.join("Documents");
    let projects = documents.join("Projects");
    std::fs::create_dir_all(&projects).unwrap();
    let paths = || documents_at(&documents);
    let mine = chosen("Projects", projects.clone());

    // Projects is backed up: Documents is refused, naming it.
    let config = settings_with(&[&mine]);
    let refused = switch_on_refusal_with(&config, "documents", Some(&me()), paths()).unwrap();
    assert_eq!(
        refused,
        SwitchRefused {
            other: "Projects".into(),
            how: Nesting::Holds
        }
    );
    assert_eq!(
        refused.words(),
        "This folder holds Projects, which is backed up already. Stop backing up Projects first, so nothing is \
         backed up twice."
    );
    // A chosen folder that holds Documents blocks it too.
    let everything = chosen("Home", home.clone());
    let refused = switch_on_refusal_with(&settings_with(&[&everything]), "documents", Some(&me()), paths()).unwrap();
    assert_eq!((refused.other.as_str(), refused.how), ("Home", Nesting::Inside));

    // Projects stopped (its switch off): Documents may be switched on; then "Back up again" on Projects is refused.
    let mut stopped = settings_with(&[&mine]);
    stopped.set_known_folder("custom:Projects", false);
    assert_eq!(
        switch_on_refusal_with(&stopped, "documents", Some(&me()), paths()),
        None
    );
    stopped.set_known_folder("documents", true);
    let refused = switch_on_refusal_with(&stopped, "custom:Projects", Some(&me()), paths()).unwrap();
    assert_eq!(
        refused,
        SwitchRefused {
            other: "Documents".into(),
            how: Nesting::Inside
        }
    );
    assert_eq!(
        refused.words(),
        "This folder is inside Documents, which is backed up already, so its files are in that backup."
    );

    // Another account's folder, or an untagged one, inside Documents: never compared, never named.
    let other = Identity::new(Some("u-other"), None);
    let theirs = ChosenFolder {
        owner: owner_tag(&other),
        ..chosen("Projects", projects.clone())
    };
    let untagged = ChosenFolder {
        owner: None,
        ..chosen("Projects", projects.clone())
    };
    for folder in [&theirs, &untagged] {
        let config = settings_with(&[folder]);
        assert_eq!(switch_on_refusal_with(&config, "documents", Some(&me()), paths()), None);
    }
    // No owner known: no chosen folder is compared.
    assert_eq!(switch_on_refusal_with(&config, "documents", None, paths()), None);
    // A switch off is never this rule's (the caller asks only for on); a folder apart from Documents is allowed.
    let apart = chosen("Recipes", dir.path().join("Recipes"));
    assert_eq!(
        switch_on_refusal_with(&settings_with(&[&apart]), "documents", Some(&me()), paths()),
        None
    );
}

/// No tag is made from an address, and an earlier build's address tag is
/// nobody's on every read. Its own account's start re-tags it by that account's id, once; another account's start
/// never does; a folder it would back up twice stays nobody's. A re-tagged entry holds no digest of the address, so a
/// guessed address no longer confirms its account; an entry left as it was keeps its digest and stays nobody's.
#[test]
fn an_address_tag_is_retagged_by_its_own_accounts_id_alone() {
    let dir = tempfile::tempdir().unwrap();
    let a = Identity::new(Some("u-a"), Some("  Alex@Example.invalid "));
    let b = Identity::new(Some("u-b"), Some("sam@example.invalid"));
    let a_address = old_address_tag("alex@example.invalid");
    let b_address = old_address_tag("sam@example.invalid");
    let with = |name: &str, owner: Option<&str>| ChosenFolder {
        owner: owner.map(str::to_string),
        ..chosen(name, dir.path().join(name))
    };
    let a_tag = owner_tag(&a).unwrap();
    let projects = with("Projects", Some(a_tag.as_str()));
    let maps = with("Maps", Some(a_address.as_str()));
    let notes = ChosenFolder {
        owner: Some(a_address.clone()),
        ..chosen("Notes", dir.path().join("Projects").join("Notes"))
    };
    let recipes = with("Recipes", Some(b_address.as_str()));
    let letters = with("Letters", None);
    let mut config = settings_with(&[&projects, &maps, &notes, &recipes, &letters]);
    let tags = |config: &DesktopConfig| -> Vec<Option<String>> {
        config
            .backup_chosen_folders
            .iter()
            .map(|folder| folder.owner.clone())
            .collect()
    };

    // Before any start: an address tag is nobody's.
    assert_eq!(
        chosen_for(&config.backup_chosen_folders, Some(&a)),
        vec![projects.clone()]
    );
    assert_eq!(
        chosen_for(&config.backup_chosen_folders, Some(&b)),
        Vec::<ChosenFolder>::new()
    );

    // B's start re-tags B's own folder alone.
    assert_eq!(retag_by_id(&mut config, &b), 1);
    let b_tag = owner_tag(&b).unwrap();
    assert_eq!(
        tags(&config),
        vec![
            Some(a_tag.clone()),
            Some(a_address.clone()),
            Some(a_address.clone()),
            Some(b_tag.clone()),
            None
        ],
        "another account's address-tagged folders are untouched"
    );

    // A's start: Maps takes A's id tag; Notes (inside A's Projects) stays nobody's; nothing else changes.
    assert_eq!(retag_by_id(&mut config, &a), 1);
    assert_eq!(
        tags(&config),
        vec![
            Some(a_tag.clone()),
            Some(a_tag.clone()),
            Some(a_address.clone()),
            Some(b_tag.clone()),
            None
        ]
    );
    let names = |owner: &Identity| -> Vec<String> {
        chosen_for(&config.backup_chosen_folders, Some(owner))
            .into_iter()
            .map(|folder| folder.name)
            .collect()
    };
    assert_eq!(
        names(&a),
        ["Projects", "Maps"],
        "the address tag left is still nobody's"
    );
    assert_eq!(names(&b), ["Recipes"]);
    assert_ne!(config.backup_chosen_folders[1].owner, Some(a_address.clone()));
    assert!(
        !tags(&config).contains(&Some(b_address.clone())),
        "a guessed address no longer confirms B's folder"
    );

    // Once: a second start changes nothing. Without the id, or without the address, nothing is re-tagged.
    assert_eq!(retag_by_id(&mut config, &a), 0);
    let before = tags(&config);
    assert_eq!(
        retag_by_id(&mut config, &Identity::new(None, Some("alex@example.invalid"))),
        0
    );
    assert_eq!(retag_by_id(&mut config, &Identity::new(Some("u-a"), None)), 0);
    assert_eq!(tags(&config), before);

    // No path makes an address tag: an add for an account whose id is not known writes nothing.
    let folder = dir.path().join("Drafts");
    std::fs::create_dir_all(&folder).unwrap();
    assert_eq!(
        add_to(
            &mut config,
            &folder,
            &[],
            &Identity::new(None, Some("alex@example.invalid"))
        ),
        Err(AddRefused::Backups(BackupsError::Refused(
            LocalStateRefused::IdentityUnknown
        )))
    );
    assert_eq!(config.backup_chosen_folders.len(), 5);
}

/// P3-5 (c), the start's write: the re-tag runs on the settings file under its lock, writes once, and a second start
/// finds nothing to change.
#[test]
fn the_start_retags_the_settings_file_once() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("desktop.toml");
    let a = Identity::new(Some("u-a"), Some("alex@example.invalid"));
    let maps = ChosenFolder {
        owner: Some(old_address_tag("alex@example.invalid")),
        ..chosen("Maps", dir.path().join("Maps"))
    };
    DesktopConfig::update_at(&file, |config| {
        config.backup_chosen_folders = vec![maps.clone()];
        (true, ())
    })
    .unwrap();
    assert_eq!(retag_in(&file, &a), Ok(1));
    let read = DesktopConfig::update_at(&file, |config| (false, config.backup_chosen_folders.clone())).unwrap();
    assert_eq!(read[0].owner, owner_tag(&a));
    assert_eq!(retag_in(&file, &a), Ok(0));
}
