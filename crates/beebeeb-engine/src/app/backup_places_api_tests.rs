//! The places of the Backups tree at the engine: what the app rule refuses (asked, never re-derived), which device
//! folder is this PC's, and the session rule. Sessions are put in memory as a sign-in leaves them; each test has a
//! state database of its own. Every name is invented.

use std::sync::Arc;

use super::*;
use crate::account::SessionTransition;
use crate::account_binding::Identity;
use crate::app::EngineConfig;
use crate::host::ChannelHost;
use crate::identity::REAL;
use crate::session::Session;
use crate::signin::memory_store::MemoryCredentials;
use crate::state_db::{BackupLayout, FileStatus, ItemKind};
use crate::test_api::fake_account::{FakeAccount, TEST_PHRASE};

type TestEngine = Engine<MemoryCredentials>;

const ROOT: &str = "b0000000-0000-4000-8000-000000000001";
const THIS_PC: &str = "b0000000-0000-4000-8000-000000000002";
const OTHER_PC: &str = "b0000000-0000-4000-8000-000000000003";
const DOCS: &str = "b0000000-0000-4000-8000-000000000004";
const NOTE: &str = "b0000000-0000-4000-8000-000000000005";
const MANIFEST: &str = "b0000000-0000-4000-8000-000000000006";
const PLAIN: &str = "b0000000-0000-4000-8000-000000000007";
const PLAIN_FILE: &str = "b0000000-0000-4000-8000-000000000008";
const OTHER_DOCS: &str = "b0000000-0000-4000-8000-000000000009";

struct Rig {
    engine: Arc<TestEngine>,
    account: FakeAccount,
    session: SessionGeneration,
    path: std::path::PathBuf,
    _dir: tempfile::TempDir,
}

fn put(db: &StateDb, id: &str, path: &str, parent: Option<&str>, kind: ItemKind) {
    let entry = FileEntry {
        file_id: id.to_string(),
        path: path.to_string(),
        status: FileStatus::CloudOnly,
        size_bytes: 10,
        modified_at: 1_794_000_000,
        content_hash: None,
        remote_updated_at: 1_794_000_000,
        parent_id: parent.map(str::to_string),
        item_kind: kind,
    };
    db.upsert_file_with_contract(&entry, |contract| {
        contract.item_kind = entry.item_kind.clone();
        contract.parent_id = entry.parent_id.clone();
    })
    .unwrap();
}

fn sign_in(engine: &TestEngine, account: &FakeAccount) -> SessionGeneration {
    {
        let _writes = engine.writes.write().unwrap();
        engine
            .acct
            .advance_session_generation_holding_session_write_lock(SessionTransition::Write);
        *engine.acct.session.lock().unwrap() = Some(Session {
            token: account.session_token.clone(),
            master_key: account.master_key,
            email: Some(account.email.clone()),
        });
        engine.remember_session_user_id(Some(account.user_id.clone()));
        engine.set_auth_present(true);
    }
    engine.session_turn()
}

/// Backups, with this PC's backup (Documents, a note, the manifest) and another device's, and a plain folder.
fn rigged(layout: bool) -> Rig {
    let (account, _phrase) = FakeAccount::register("places-api@example.eu", "an invented password", TEST_PHRASE);
    let state_dir = crate::state_paths::init_for_test();
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: format!("places-api-test-slot-{layout}"),
        state_dir,
        api_base: "http://127.0.0.1:9".into(),
    };
    let engine = Arc::new(Engine::open(&REAL, config, host, MemoryCredentials::default()).expect("the handle opens"));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let db = StateDb::open(&path).unwrap();
    put(&db, ROOT, "/Backups", None, ItemKind::Folder);
    db.mark_backups_root_for_test(ROOT).unwrap();
    put(&db, THIS_PC, "/Backups/Invented Laptop", Some(ROOT), ItemKind::Folder);
    put(&db, OTHER_PC, "/Backups/Invented Phone", Some(ROOT), ItemKind::Folder);
    put(
        &db,
        DOCS,
        "/Backups/Invented Laptop/Documents",
        Some(THIS_PC),
        ItemKind::Folder,
    );
    put(
        &db,
        NOTE,
        "/Backups/Invented Laptop/Documents/note.txt",
        Some(DOCS),
        ItemKind::File,
    );
    put(
        &db,
        MANIFEST,
        "/Backups/Invented Laptop/.device.json",
        Some(THIS_PC),
        ItemKind::File,
    );
    put(
        &db,
        OTHER_DOCS,
        "/Backups/Invented Phone/Photos",
        Some(OTHER_PC),
        ItemKind::Folder,
    );
    put(&db, PLAIN, "/Plain", None, ItemKind::Folder);
    put(&db, PLAIN_FILE, "/Plain/plain.txt", Some(PLAIN), ItemKind::File);
    db.set_owner(&Identity::new(Some(&account.user_id), Some(&account.email)))
        .unwrap();
    if layout {
        db.set_backup_layout(&BackupLayout {
            root_id: ROOT.into(),
            device_folder_id: THIS_PC.into(),
            device_folder_name: "Invented Laptop".into(),
        })
        .unwrap();
    }
    drop(db);
    engine.use_local_state_for_test(path.clone());
    let session = sign_in(&engine, &account);
    Rig {
        engine,
        account,
        session,
        path,
        _dir: dir,
    }
}

fn place(places: &BackupPlaces, id: &str) -> BackupPlace {
    places
        .rows
        .iter()
        .find(|(row, _)| row == id)
        .unwrap_or_else(|| panic!("row not listed: {id}"))
        .1
        .clone()
}

#[test]
fn places_follow_the_app_rule() {
    let rig = rigged(true);
    let root = rig.engine.backup_places_for(rig.session, Some(ROOT)).unwrap();
    assert_eq!(root.of_folder, BackupPlace::Root);
    assert_eq!(
        place(&root, THIS_PC),
        BackupPlace::Device {
            id: THIS_PC.into(),
            this_pc: true
        }
    );
    assert_eq!(
        place(&root, OTHER_PC),
        BackupPlace::Device {
            id: OTHER_PC.into(),
            this_pc: false
        }
    );
    let device = rig.engine.backup_places_for(rig.session, Some(THIS_PC)).unwrap();
    assert_eq!(
        device.of_folder,
        BackupPlace::Device {
            id: THIS_PC.into(),
            this_pc: true
        }
    );
    assert_eq!(
        place(&device, DOCS),
        BackupPlace::Inside {
            device: THIS_PC.into(),
            this_pc: true
        }
    );
    assert!(
        !device.rows.iter().any(|(id, _)| id == MANIFEST),
        "the hidden manifest is not listed"
    );
    let docs = rig.engine.backup_places_for(rig.session, Some(DOCS)).unwrap();
    assert_eq!(
        docs.of_folder,
        BackupPlace::Inside {
            device: THIS_PC.into(),
            this_pc: true
        }
    );
    assert_eq!(
        place(&docs, NOTE),
        BackupPlace::Inside {
            device: THIS_PC.into(),
            this_pc: true
        }
    );
    let other = rig.engine.backup_places_for(rig.session, Some(OTHER_PC)).unwrap();
    assert_eq!(
        place(&other, OTHER_DOCS),
        BackupPlace::Inside {
            device: OTHER_PC.into(),
            this_pc: false
        }
    );
    // Outside the tree, and the root of Files.
    let plain = rig.engine.backup_places_for(rig.session, Some(PLAIN)).unwrap();
    assert_eq!(plain.of_folder, BackupPlace::Outside);
    assert_eq!(place(&plain, PLAIN_FILE), BackupPlace::Outside);
    let top = rig.engine.backup_places_for(rig.session, None).unwrap();
    assert_eq!(top.of_folder, BackupPlace::Outside);
    assert_eq!(place(&top, ROOT), BackupPlace::Root);
    assert_eq!(place(&top, PLAIN), BackupPlace::Outside);
    assert_eq!(top.folder, None);
}

#[test]
fn unreadable_roots_are_an_error_never_outside() {
    let rig = rigged(true);
    // The state's `files` table can no longer say which row is a Backups root.
    let raw = rusqlite::Connection::open(&rig.path).unwrap();
    raw.execute_batch("ALTER TABLE files RENAME COLUMN system_role TO system_role_gone;")
        .unwrap();
    drop(raw);
    assert!(rig.engine.backup_places_for(rig.session, Some(DOCS)).is_err());
    assert!(rig.engine.backup_places_for(rig.session, None).is_err());
}

#[test]
fn this_pc_is_the_layouts_device_folder_only() {
    let rig = rigged(true);
    let root = rig.engine.backup_places_for(rig.session, Some(ROOT)).unwrap();
    let this_pc: Vec<&str> = root
        .rows
        .iter()
        .filter(|(_, place)| matches!(place, BackupPlace::Device { this_pc: true, .. }))
        .map(|(id, _)| id.as_str())
        .collect();
    assert_eq!(this_pc, [THIS_PC]);
    // A device named exactly as the layout's name, but not the layout's folder, is not this PC.
    let db = StateDb::open(&rig.path).unwrap();
    put(
        &db,
        "b0000000-0000-4000-8000-0000000000aa",
        "/Backups/Invented Laptop 2",
        Some(ROOT),
        ItemKind::Folder,
    );
    drop(db);
    let again = rig.engine.backup_places_for(rig.session, Some(ROOT)).unwrap();
    let count = again
        .rows
        .iter()
        .filter(|(_, place)| matches!(place, BackupPlace::Device { this_pc: true, .. }))
        .count();
    assert_eq!(count, 1);
    // No layout: nothing is this PC's.
    let none = rigged(false);
    let places = none.engine.backup_places_for(none.session, Some(ROOT)).unwrap();
    assert!(
        places
            .rows
            .iter()
            .all(|(_, place)| !matches!(place, BackupPlace::Device { this_pc: true, .. }))
    );
}

#[test]
fn places_for_a_session_that_moved_on_are_refused() {
    let rig = rigged(true);
    assert!(rig.engine.backup_places_for(rig.session, Some(ROOT)).is_ok());
    rig.engine.forget_session(SessionTransition::SignOut);
    rig.engine.remember_session_user_id(None);
    rig.engine.set_auth_present(false);
    assert!(rig.engine.backup_places_for(rig.session, Some(ROOT)).is_err());
    // A press of an older turn, with a session in memory again: refused, nothing read.
    let later = sign_in(&rig.engine, &rig.account);
    assert_ne!(later, rig.session);
    assert_eq!(
        rig.engine.backup_places_for(rig.session, Some(ROOT)),
        Err(LocalStateRefused::SessionChanged)
    );
}
