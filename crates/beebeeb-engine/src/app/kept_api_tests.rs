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
    pin(
        &REAL,
        "http://127.0.0.1:9",
        std::sync::Arc::new(open()),
        sync_root.path(),
        "d-a",
        true,
    )
    .expect("the pin");
    let kept: Vec<String> = db.kept_rows().unwrap().into_iter().map(|e| e.file_id).collect();
    assert_eq!(kept, ["d-a", "f-1"]);
    pin(
        &REAL,
        "http://127.0.0.1:9",
        std::sync::Arc::new(open()),
        sync_root.path(),
        "d-a",
        false,
    )
    .expect("the release");
    assert!(db.kept_rows().unwrap().is_empty());
}

/// Two server items at one place on disk (`drive_v2::case_place`): Keep on this PC, and Remove from this PC, of the row
/// whose entry it is not are refused and write nothing (the pin, the queued change for other devices); the row whose
/// spelling the entry has, and a row alone, keep both. Linux tells case apart, so the case pair plays a volume that
/// does not; the two items with the very same name are one entry on every volume.
#[test]
fn keep_on_this_pc_of_a_twin_is_refused_and_writes_nothing() {
    use crate::drive_v2::case_place::{CaseRule, with_rule_for_test};
    use crate::state_db::PinState;
    let dir = tempfile::tempdir().unwrap();
    let sync_root = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let db = StateDb::open(&path).unwrap();
    put(&db, "upper", "Report.pdf", false);
    put(&db, "lower", "report.pdf", false);
    put(&db, "same-a", "Photo.jpg", false);
    put(&db, "same-b", "Photo.jpg", false);
    put(&db, "alone", "alone.txt", false);
    // `lower` was kept before it had a twin: Remove from this PC must not release the other row's entry either.
    db.upsert_file_with_contract(&db.get_file("lower").unwrap().unwrap(), |contract| {
        contract.item_kind = ItemKind::File;
        contract.pin_state = PinState::Pinned;
    })
    .unwrap();
    for name in ["Report.pdf", "Photo.jpg", "alone.txt"] {
        std::fs::write(sync_root.path().join(name), b"one entry").unwrap();
    }
    let ops = || -> usize {
        ["lower", "same-a", "same-b"]
            .iter()
            .map(|id| db.list_operations_for_file(id).unwrap().len())
            .sum()
    };
    assert_eq!(ops(), 0);
    let pin_of = |id: &str, kept: bool| {
        pin(
            &REAL,
            "http://127.0.0.1:9",
            Arc::new(StateDb::open(&path).unwrap()),
            sync_root.path(),
            id,
            kept,
        )
    };
    let kept = || -> Vec<String> { db.kept_rows().unwrap().into_iter().map(|e| e.file_id).collect() };
    with_rule_for_test(CaseRule::Insensitive, || {
        for id in ["same-a", "same-b"] {
            assert_eq!(pin_of(id, true).err().as_deref(), Some(super::SHARED_PLACE), "{id}");
        }
        assert_eq!(
            pin_of("lower", false).err().as_deref(),
            Some(super::SHARED_PLACE_BUT_CASE)
        );
        assert_eq!(
            pin_of("lower", true).err().as_deref(),
            Some(super::SHARED_PLACE_BUT_CASE)
        );
        assert_eq!(kept(), ["lower"], "nothing pinned, nothing released");
        assert_eq!(ops(), 0, "no change queued for other devices");
        pin_of("upper", true).expect("the row whose spelling the entry has");
        pin_of("alone", true).expect("a row alone at its place");
        assert_eq!(kept(), ["upper", "alone", "lower"], "by path, byte order");
        pin_of("upper", false).expect("released again");
    });
}
