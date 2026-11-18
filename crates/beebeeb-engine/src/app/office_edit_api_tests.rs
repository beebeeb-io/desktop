//! An Office save at the engine (`office_edit_api.rs`), against the local state alone: the closed loopback port means
//! sync does not run, so every save is stored here and answers at once. Every name, key and byte is invented.

use std::sync::Arc;

use zeroize::Zeroizing;

use super::*;
use crate::account::SessionTransition;
use crate::account_binding::Identity;
use crate::app::{Engine, EngineConfig};
use crate::host::ChannelHost;
use crate::identity::TEST;
use crate::session::Session;
use crate::signin::memory_store::MemoryCredentials;
use crate::state_db::{FileEntry, FileStatus, ItemKind, PERMISSION_READ, PERMISSION_WRITE, StateDb};

type TestEngine = Engine<MemoryCredentials>;

const DOC: &str = "b0000000-0000-4000-8000-000000000601";
const FOLDER: &str = "b0000000-0000-4000-8000-000000000602";

/// A stored (uncompressed) zip of `names`, each holding a few invented bytes: local headers, the directory, the end.
fn zip_of(names: &[&str]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut dir = Vec::new();
    for name in names {
        let at = out.len() as u32;
        out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        out.extend_from_slice(&[20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        out.extend_from_slice(&0u32.to_le_bytes()); // crc
        out.extend_from_slice(&3u32.to_le_bytes());
        out.extend_from_slice(&3u32.to_le_bytes());
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(b"xyz");
        dir.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        dir.extend_from_slice(&[20, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        dir.extend_from_slice(&0u32.to_le_bytes());
        dir.extend_from_slice(&3u32.to_le_bytes());
        dir.extend_from_slice(&3u32.to_le_bytes());
        dir.extend_from_slice(&(name.len() as u16).to_le_bytes());
        dir.extend_from_slice(&[0; 8]); // extra, comment, disk, internal attrs
        dir.extend_from_slice(&0u32.to_le_bytes()); // external attrs
        dir.extend_from_slice(&at.to_le_bytes());
        dir.extend_from_slice(name.as_bytes());
    }
    let dir_at = out.len() as u32;
    out.extend_from_slice(&dir);
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&(names.len() as u16).to_le_bytes());
    out.extend_from_slice(&(names.len() as u16).to_le_bytes());
    out.extend_from_slice(&(dir.len() as u32).to_le_bytes());
    out.extend_from_slice(&dir_at.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

fn package() -> Zeroizing<Vec<u8>> {
    Zeroizing::new(zip_of(&["[Content_Types].xml", "word/document.xml"]))
}

fn engine() -> Arc<TestEngine> {
    let state_dir = crate::state_paths::init_for_test();
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "an-invented-office-slot".into(),
        state_dir,
        // A closed loopback port: nothing is sent, every save stays queued here.
        api_base: "http://127.0.0.1:9".into(),
    };
    Arc::new(Engine::open(&TEST, config, host, MemoryCredentials::default()).expect("the handle opens"))
}

const USER: &str = "an-invented-office-user";
const EMAIL: &str = "office-save@example.eu";

fn sign_in(engine: &TestEngine) {
    let _writes = engine.writes.write().unwrap();
    engine
        .acct
        .advance_session_generation_holding_session_write_lock(SessionTransition::Write);
    *engine.acct.session.lock().unwrap() = Some(Session {
        token: "an-invented-token".into(),
        master_key: [7; 32],
        email: Some(EMAIL.into()),
    });
    engine.remember_session_user_id(Some(USER.into()));
    engine.set_auth_present(true);
}

fn row(db: &StateDb, id: &str, path: &str, folder: bool) {
    let entry = FileEntry {
        file_id: id.into(),
        path: path.into(),
        status: FileStatus::Local,
        size_bytes: 3,
        modified_at: 1_794_000_000,
        content_hash: None,
        remote_updated_at: 1_794_000_000,
        parent_id: None,
        item_kind: if folder { ItemKind::Folder } else { ItemKind::File },
    };
    db.upsert_file_with_contract(&entry, |contract| {
        contract.item_kind = entry.item_kind.clone();
        contract.permission_bits |= PERMISSION_READ | PERMISSION_WRITE;
    })
    .unwrap();
}

/// A signed-in engine with a writable document at version 4 and a folder, on a state of its own.
fn setup() -> (Arc<TestEngine>, tempfile::TempDir, tempfile::TempDir, StateDb) {
    let engine = engine();
    sign_in(&engine);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let db = StateDb::open(&path).unwrap();
    row(&db, FOLDER, "Invented folder", true);
    row(&db, DOC, "Invented doc.docx", false);
    let mut contract = db.get_file_contract_state(DOC).unwrap().unwrap();
    contract.current_version = 4;
    db.set_file_contract_state(&contract).unwrap();
    db.set_owner(&Identity::new(Some(USER), Some(EMAIL))).unwrap();
    engine.use_local_state_for_test(path);
    let root = tempfile::tempdir().unwrap();
    engine.use_sync_root_for_test(root.path().to_path_buf());
    (engine, dir, root, db)
}

#[test]
fn office_edit_a_save_from_the_current_version_queues_a_new_version() {
    let (engine, _d, _r, db) = setup();
    let session = engine.acct.session_generation();
    let opening = engine.viewer_opening(session, DOC);
    let end = engine.save_office_for(session, DOC, &package(), 4, opening);
    assert_eq!(end, Ok(SaveEnd::Queued { base: 4 }));
    assert_eq!(db.list_operations_for_file(DOC).unwrap().len(), 1, "one upload queued");
}

#[test]
fn office_edit_a_save_from_an_older_base_is_a_conflict_and_writes_nothing() {
    let (engine, _d, _r, db) = setup();
    let session = engine.acct.session_generation();
    let opening = engine.viewer_opening(session, DOC);
    let end = engine.save_office_for(session, DOC, &package(), 3, opening);
    assert_eq!(end, Ok(SaveEnd::Conflict { theirs: 4 }));
    assert!(db.list_operations_for_file(DOC).unwrap().is_empty(), "nothing queued");
    assert!(db.held_write(DOC).unwrap().is_none(), "no write held");
}

#[test]
fn office_edit_keep_both_makes_the_conflict_copy_name() {
    let (engine, _d, _r, db) = setup();
    let session = engine.acct.session_generation();
    let copy = engine
        .save_office_copy_for(session, DOC, &package(), "Invented PC", "10.15")
        .expect("the copy is stored");
    assert_eq!(copy.name, "Invented doc (Invented PC, 10.15).docx");
    assert_eq!(db.list_operations_for_file(&copy.id).unwrap().len(), 1, "its upload is queued");
    assert!(
        db.list_operations_for_file(DOC).unwrap().is_empty(),
        "the original is untouched"
    );
}

#[test]
fn office_edit_a_damaged_package_is_refused_and_nothing_is_written() {
    let (engine, _d, _r, db) = setup();
    let session = engine.acct.session_generation();
    let opening = engine.viewer_opening(session, DOC);
    let good = zip_of(&["[Content_Types].xml", "word/document.xml"]);
    let mut cut = good.clone();
    cut.truncate(good.len() - 10);
    let bad = [
        Vec::new(),
        b"not a zip at all, only invented text".to_vec(),
        cut,
        zip_of(&["word/document.xml"]),
        zip_of(&["[Content_Types].xml", "docProps/core.xml"]),
    ];
    for bytes in bad {
        let bytes = Zeroizing::new(bytes);
        assert_eq!(
            engine.save_office_for(session, DOC, &bytes, 4, opening),
            Err(EditRefused::Failed)
        );
        assert_eq!(
            engine.save_office_copy_for(session, DOC, &bytes, "d", "1.00").err(),
            Some(EditRefused::Failed)
        );
        assert_eq!(
            engine.create_office_for(session, None, "New.docx", &bytes).err(),
            Some(EditRefused::Failed)
        );
    }
    assert!(db.list_operations_for_file(DOC).unwrap().is_empty());
    assert!(db.held_write(DOC).unwrap().is_none());
}

#[test]
fn office_edit_a_package_over_the_cap_is_refused() {
    let mut big = zip_of(&["[Content_Types].xml", "word/document.xml"]);
    assert_eq!(check_package(&big), Ok(()));
    big.resize(MAX_PACKAGE_BYTES as usize + 1, 0);
    assert_eq!(check_package(&big), Err(PackageRefused::TooLarge));
    // A comment after the end record is part of the package: it still reads.
    let mut commented = zip_of(&["[Content_Types].xml", "xl/workbook.xml"]);
    let end = commented.len() - 2;
    commented[end] = 3;
    commented.extend_from_slice(b"abc");
    assert_eq!(check_package(&commented), Ok(()));
}

#[test]
fn office_edit_a_save_after_sign_out_is_refused() {
    let (engine, _d, _r, db) = setup();
    let session = engine.acct.session_generation();
    let opening = engine.viewer_opening(session, DOC);
    // Another session takes its place (a sign-out and a sign-in): the press's session is gone.
    sign_in(&engine);
    assert_eq!(
        engine.save_office_for(session, DOC, &package(), 4, opening),
        Err(EditRefused::NotSignedIn)
    );
    assert_eq!(
        engine.save_office_copy_for(session, DOC, &package(), "d", "1.00").err(),
        Some(EditRefused::NotSignedIn)
    );
    assert_eq!(
        engine.create_office_for(session, None, "New.docx", &package()).err(),
        Some(EditRefused::NotSignedIn)
    );
    assert!(db.list_operations_for_file(DOC).unwrap().is_empty());
}

#[test]
fn office_edit_create_refuses_a_name_that_exists_and_queues_a_new_one() {
    let (engine, _d, _r, db) = setup();
    let session = engine.acct.session_generation();
    // The top holds "Invented doc.docx" (any case) and the folder "Invented folder".
    for name in ["Invented doc.docx", "invented DOC.docx", "Invented folder"] {
        assert_eq!(
            engine.create_office_for(session, None, name, &package()).err(),
            Some(EditRefused::Failed),
            "{name}"
        );
    }
    for name in ["", "a/b.docx", ".."] {
        assert_eq!(
            engine.create_office_for(session, None, name, &package()).err(),
            Some(EditRefused::Failed)
        );
    }
    let id = engine
        .create_office_for(session, None, "Invented new.docx", &package())
        .expect("a new name is created");
    assert_eq!(db.list_operations_for_file(&id).unwrap().len(), 1);
    // Now it exists.
    assert_eq!(
        engine
            .create_office_for(session, None, "invented NEW.docx", &package())
            .err(),
        Some(EditRefused::Failed)
    );
    // In a folder, the same name is free; a file is no folder.
    assert!(
        engine
            .create_office_for(session, Some(FOLDER), "Invented new.docx", &package())
            .is_ok()
    );
    assert_eq!(
        engine
            .create_office_for(session, Some(DOC), "Inside.docx", &package())
            .err(),
        Some(EditRefused::Unavailable)
    );
}
