//! Sealed staging, end to end: an Office save goes through the sealed staged file to the fake server and lands as a
//! new version with exactly the saved bytes; a restart between staging and upload resumes it (or leaves it sealed and
//! unsent); a blob of another account or another document is refused and nothing is uploaded. Invented names, keys and
//! bytes throughout.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use beebeeb_core::chunk_stream::ChunkDecryptor;
use beebeeb_core::encrypt::decrypt_file_metadata_with_key;
use beebeeb_core::kdf::{MasterKey, derive_file_key};
use zeroize::Zeroizing;

use super::*;
use crate::account::SessionTransition;
use crate::account_binding::Identity;
use crate::app::{Engine, EngineConfig};
use crate::engine_bridge::EngineBridge;
use crate::host::ChannelHost;
use crate::identity::TEST;
use crate::session::Session;
use crate::signin::memory_store::MemoryCredentials;
use crate::state_db::{FileEntry, FileStatus, ItemKind, PERMISSION_READ, PERMISSION_WRITE, StateDb};
use crate::test_api::FakeApi;
use crate::test_api::fake_account::{FakeAccount, test_account};

type TestEngine = Engine<MemoryCredentials>;

const DOC: &str = "b0000000-0000-4000-8000-000000000a01";
const OTHER_DOC: &str = "b0000000-0000-4000-8000-000000000a02";
const MARKER: &str = "word/INVENTED-STAGING-MARKER-5519.xml";
const USER: &str = "an-invented-staging-user";
const EMAIL: &str = "staging@example.eu";

fn zip_of(names: &[&str]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut dir = Vec::new();
    for name in names {
        let at = out.len() as u32;
        out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        out.extend_from_slice(&[20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        out.extend_from_slice(&0u32.to_le_bytes());
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
        dir.extend_from_slice(&[0; 8]);
        dir.extend_from_slice(&0u32.to_le_bytes());
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
    Zeroizing::new(zip_of(&["[Content_Types].xml", "word/document.xml", MARKER]))
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

fn row(db: &StateDb, id: &str, path: &str) {
    let entry = FileEntry {
        file_id: id.into(),
        path: path.into(),
        status: FileStatus::Local,
        size_bytes: 8,
        modified_at: 1_794_000_000,
        content_hash: None,
        remote_updated_at: 1_794_000_000,
        parent_id: None,
        item_kind: ItemKind::File,
    };
    db.upsert_file_with_contract(&entry, |contract| {
        contract.item_kind = ItemKind::File;
        contract.permission_bits |= PERMISSION_READ | PERMISSION_WRITE;
    })
    .unwrap();
    let mut contract = db.get_file_contract_state(id).unwrap().unwrap();
    contract.current_version = 1;
    db.set_file_contract_state(&contract).unwrap();
}

struct Rig {
    engine: Arc<TestEngine>,
    account: Arc<FakeAccount>,
    fake: FakeApi,
    state_path: PathBuf,
    root: tempfile::TempDir,
    _dir: tempfile::TempDir,
}

/// A signed-in engine on the fake server's account, holding two writable documents at version 1 (also on the server).
fn rig() -> Rig {
    let (account, _phrase, fake) = test_account();
    for (id, name) in [(DOC, "Invented doc.docx"), (OTHER_DOC, "Other doc.docx")] {
        account.add_file_with_content(id, None, name, b"version1");
    }
    let state_dir = crate::state_paths::init_for_test();
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "an-invented-staging-slot".into(),
        state_dir,
        api_base: fake.base.clone(),
    };
    let engine = Arc::new(Engine::open(&TEST, config, host, MemoryCredentials::default()).expect("the handle opens"));
    {
        let _writes = engine.writes.write().unwrap();
        engine
            .acct
            .advance_session_generation_holding_session_write_lock(SessionTransition::Write);
        *engine.acct.session.lock().unwrap() = Some(Session {
            token: account.session_token.clone(),
            master_key: account.master_key,
            email: Some(EMAIL.into()),
        });
        engine.remember_session_user_id(Some(USER.into()));
        engine.set_auth_present(true);
    }
    let dir = tempfile::tempdir().unwrap();
    let state_path = dir.path().join("state.db");
    let db = StateDb::open(&state_path).unwrap();
    row(&db, DOC, "Invented doc.docx");
    row(&db, OTHER_DOC, "Other doc.docx");
    db.set_owner(&Identity::new(Some(USER), Some(EMAIL))).unwrap();
    drop(db);
    engine.use_local_state_for_test(state_path.clone());
    let root = tempfile::tempdir().unwrap();
    engine.use_sync_root_for_test(root.path().to_path_buf());
    Rig {
        engine,
        account,
        fake,
        state_path,
        root,
        _dir: dir,
    }
}

/// A bridge as a fresh start builds it: the state reopened from its file, a client for `key`.
fn bridge(rig: &Rig, key: [u8; 32]) -> EngineBridge {
    let db = Arc::new(StateDb::open(&rig.state_path).unwrap());
    let api = Arc::new(ApiClient::new(rig.fake.base.clone(), rig.account.session_token.clone(), key));
    EngineBridge::new(&TEST, db, api)
}

fn drive(bridge: &EngineBridge, root: &Path) -> crate::engine_bridge::TransferLoopOutcome {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(bridge.process_due_operations(root, crate::state_db::now_secs()))
        .expect("the pass runs")
}

/// The stored content of `id`, opened against the stored name the way a reader does.
fn stored_plain(account: &FakeAccount, id: &str) -> Vec<u8> {
    let master = MasterKey::from_bytes(account.master_key);
    let key = derive_file_key(&master, id.as_bytes());
    let meta = decrypt_file_metadata_with_key(&key, &account.stored_name(id).unwrap()).unwrap();
    let mut reader = ChunkDecryptor::for_current_version(&master, id, &meta);
    let mut out = Vec::new();
    for frame in account.stored_frames(id).unwrap() {
        out.extend_from_slice(&reader.push_frame(&frame).unwrap().data);
    }
    reader.finish().unwrap();
    out
}

fn save(rig: &Rig, id: &str, bytes: &[u8]) -> Result<SaveEnd, EditRefused> {
    let session = rig.engine.acct.session_generation();
    let opening = rig.engine.viewer_opening(session, id);
    rig.engine.save_office_for(session, id, bytes, 1, opening)
}

fn staged_path_of(rig: &Rig, id: &str) -> PathBuf {
    let db = StateDb::open(&rig.state_path).unwrap();
    let ops = db.list_operations_for_file(id).unwrap();
    assert_eq!(ops.len(), 1, "one op is queued");
    PathBuf::from(ops[0].payload_path.clone().unwrap())
}

#[test]
fn a_real_office_save_goes_through_the_sealed_staged_file_and_lands_as_a_new_version_with_the_right_bytes() {
    let rig = rig();
    let bytes = package();
    assert_eq!(save(&rig, DOC, &bytes), Ok(SaveEnd::Queued { base: 1 }));
    let staged = staged_path_of(&rig, DOC);
    let on_disk = std::fs::read(&staged).unwrap();
    assert!(!contains(&on_disk, MARKER.as_bytes()), "the staged copy holds a byte of the document");

    let outcome = drive(&bridge(&rig, rig.account.master_key), rig.root.path());
    assert_eq!(outcome.completed_op_ids.len(), 1, "{outcome:?}");
    assert!(outcome.retried_op_ids.is_empty() && outcome.paused_op_ids.is_empty(), "{outcome:?}");
    // The server holds version 2, and it opens to exactly the saved package.
    assert_eq!(rig.account.item_state(DOC).unwrap().0, 2);
    assert_eq!(stored_plain(&rig.account, DOC), &bytes[..]);
    // The other document is untouched.
    assert_eq!(rig.account.item_state(OTHER_DOC).unwrap().0, 1);
    assert_eq!(stored_plain(&rig.account, OTHER_DOC), b"version1");
    // The queue is empty and the local record moved to version 2.
    let db = StateDb::open(&rig.state_path).unwrap();
    assert!(db.list_operations_for_file(DOC).unwrap().is_empty());
    assert_eq!(db.get_file_contract_state(DOC).unwrap().unwrap().current_version, 2);
    // Nothing of the document is left on disk in the staging place either.
    if let Some(dir) = staged.parent() {
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            if entry.path().is_file() {
                let data = std::fs::read(entry.path()).unwrap_or_default();
                assert!(!contains(&data, MARKER.as_bytes()), "plaintext left at {:?}", entry.path());
            }
        }
    }
}

#[test]
fn a_restart_between_staging_and_upload_resumes_from_the_sealed_file_alone() {
    let rig = rig();
    let bytes = package();
    assert_eq!(save(&rig, DOC, &bytes), Ok(SaveEnd::Queued { base: 1 }));
    let staged = staged_path_of(&rig, DOC);
    // The restart: the engine, its memory and its handles are gone; only the state file, the sealed blob and the
    // account key (from the keychain) remain.
    drop(rig.engine);
    let after = Rig {
        engine: Arc::new(
            Engine::open(
                &TEST,
                EngineConfig {
                    account_id: "an-invented-staging-slot".into(),
                    state_dir: crate::state_paths::init_for_test(),
                    api_base: rig.fake.base.clone(),
                },
                ChannelHost::new(|| {}).0,
                MemoryCredentials::default(),
            )
            .unwrap(),
        ),
        ..rig
    };
    assert!(staged.exists());
    assert!(!contains(&std::fs::read(&staged).unwrap(), MARKER.as_bytes()));
    let outcome = drive(&bridge(&after, after.account.master_key), after.root.path());
    assert_eq!(outcome.completed_op_ids.len(), 1, "{outcome:?}");
    assert_eq!(after.account.item_state(DOC).unwrap().0, 2);
    assert_eq!(stored_plain(&after.account, DOC), &bytes[..]);
    assert_eq!(after.account.item_state(OTHER_DOC).unwrap().0, 1);
}

#[test]
fn a_restart_under_another_accounts_key_uploads_nothing_and_leaves_the_blob_sealed() {
    let rig = rig();
    let bytes = package();
    assert_eq!(save(&rig, DOC, &bytes), Ok(SaveEnd::Queued { base: 1 }));
    let staged = staged_path_of(&rig, DOC);
    let before = std::fs::read(&staged).unwrap();
    let outcome = drive(&bridge(&rig, [9; 32]), rig.root.path());
    assert!(outcome.completed_op_ids.is_empty(), "{outcome:?}");
    assert_eq!(rig.account.item_state(DOC).unwrap().0, 1, "no version landed");
    assert_eq!(stored_plain(&rig.account, DOC), b"version1");
    let db = StateDb::open(&rig.state_path).unwrap();
    let ops = db.list_operations_for_file(DOC).unwrap();
    assert_eq!(ops.len(), 1, "the op stays queued: nothing is dropped");
    assert_eq!(ops[0].last_error.as_deref(), Some("staged upload payload could not be read"));
    assert_eq!(std::fs::read(&staged).unwrap(), before, "the blob is untouched and still sealed");
    assert!(!contains(&before, MARKER.as_bytes()));
}

#[test]
fn a_blob_staged_for_one_document_is_never_uploaded_as_another() {
    let rig = rig();
    let bytes = package();
    assert_eq!(save(&rig, DOC, &bytes), Ok(SaveEnd::Queued { base: 1 }));
    assert_eq!(save(&rig, OTHER_DOC, &zip_of(&["[Content_Types].xml", "word/document.xml"])), Ok(SaveEnd::Queued { base: 1 }));
    // Swap the two staged blobs on disk: each op now points at the other document's sealed copy.
    let (a, b) = (staged_path_of_one(&rig, DOC), staged_path_of_one(&rig, OTHER_DOC));
    let (blob_a, blob_b) = (std::fs::read(&a).unwrap(), std::fs::read(&b).unwrap());
    std::fs::write(&a, &blob_b).unwrap();
    std::fs::write(&b, &blob_a).unwrap();
    let outcome = drive(&bridge(&rig, rig.account.master_key), rig.root.path());
    assert!(outcome.completed_op_ids.is_empty(), "{outcome:?}");
    for id in [DOC, OTHER_DOC] {
        assert_eq!(rig.account.item_state(id).unwrap().0, 1, "no wrong-document version landed");
        assert_eq!(stored_plain(&rig.account, id), b"version1");
    }
}

fn staged_path_of_one(rig: &Rig, id: &str) -> PathBuf {
    let db = StateDb::open(&rig.state_path).unwrap();
    PathBuf::from(db.list_operations_for_file(id).unwrap()[0].payload_path.clone().unwrap())
}

#[test]
fn a_save_from_an_ended_session_stages_nothing() {
    let rig = rig();
    let session = rig.engine.acct.session_generation();
    let opening = rig.engine.viewer_opening(session, DOC);
    {
        let _writes = rig.engine.writes.write().unwrap();
        rig.engine
            .acct
            .advance_session_generation_holding_session_write_lock(SessionTransition::Write);
    }
    assert_eq!(
        rig.engine.save_office_for(session, DOC, &package(), 1, opening),
        Err(EditRefused::NotSignedIn)
    );
    let db = StateDb::open(&rig.state_path).unwrap();
    assert!(db.list_operations_for_file(DOC).unwrap().is_empty());
}
