//! `Engine::set_kept` with the engine's own parts: a real handle (`Engine::open`, in-memory credentials, the process's
//! test state folder, which nothing here writes) refuses while no one is signed in; the pin change it makes
//! (`EngineBridge::set_recursive_pin` on a tokenless client) reaches a real state database in a folder of the test's
//! own for the whole folder, and comes off again. The shared config sandbox is never written. Every name is invented.

use std::sync::Arc;

use super::super::{Engine, EngineConfig};
use super::pin;
use crate::host::ChannelHost;
use crate::identity::REAL;
use crate::signin::gate::GateState;
use crate::signin::memory_store::MemoryCredentials;
use crate::state_db::{FileEntry, FileStatus, ItemKind, StateDb};

fn put(db: &StateDb, id: &str, path: &str, folder: bool) {
    let entry = FileEntry {
        file_id: id.into(),
        path: path.into(),
        status: FileStatus::Local,
        size_bytes: if folder { 0 } else { 10 },
        modified_at: 1,
        content_hash: None,
        remote_updated_at: 1,
        parent_id: None,
        item_kind: if folder { ItemKind::Folder } else { ItemKind::File },
    };
    db.upsert_file_with_contract(&entry, |contract| contract.item_kind = entry.item_kind.clone())
        .unwrap();
}

#[test]
fn set_kept_is_refused_while_no_one_is_signed_in() {
    let state_dir = crate::state_paths::init_for_test();
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "an-invented-slot".into(),
        state_dir,
        api_base: "http://127.0.0.1:9".into(),
    };
    let engine = Arc::new(Engine::open(&REAL, config, host, MemoryCredentials::default()).expect("the handle opens"));
    assert_eq!(engine.view().gate, GateState::SignedOut);
    assert_eq!(
        engine.set_kept("f-1", true).err().as_deref(),
        Some(super::NOT_SIGNED_IN)
    );
}

#[test]
fn the_pin_change_keeps_a_folder_with_its_contents_and_releases_it() {
    let dir = tempfile::tempdir().unwrap();
    let sync_root = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let db = StateDb::open(&path).unwrap();
    put(&db, "d-a", "A", true);
    put(&db, "f-1", "A/one.txt", false);
    put(&db, "f-2", "other.txt", false);
    let open = || StateDb::open(&path).unwrap();
    pin(&REAL, "http://127.0.0.1:9", open(), sync_root.path(), "d-a", true).expect("the pin");
    let kept: Vec<String> = db.kept_rows().unwrap().into_iter().map(|e| e.file_id).collect();
    assert_eq!(kept, ["d-a", "f-1"]);
    pin(&REAL, "http://127.0.0.1:9", open(), sync_root.path(), "d-a", false).expect("the release");
    assert!(db.kept_rows().unwrap().is_empty());
}
