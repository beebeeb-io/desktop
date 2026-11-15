//! `Engine::files_facts` with the engine's own parts: a real handle, a session put in memory as a sign-in and an
//! unlock leave it, and a state database of the test's own recorded as one account's. Every name is invented.

use std::sync::Arc;

use super::super::{Engine, EngineConfig};
use super::*;
use crate::account::SessionTransition;
use crate::account_binding::Identity;
use crate::host::ChannelHost;
use crate::identity::REAL;
use crate::session::Session;
use crate::signin::memory_store::MemoryCredentials;
use crate::state_db::{FileEntry, FileStatus, ItemKind, StateDb};

type TestEngine = Engine<MemoryCredentials>;

fn engine() -> Arc<TestEngine> {
    let state_dir = crate::state_paths::init_for_test();
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "an-invented-facts-slot".into(),
        state_dir,
        api_base: "http://127.0.0.1:9".into(),
    };
    Arc::new(Engine::open(&REAL, config, host, MemoryCredentials::default()).expect("the handle opens"))
}

fn signed_in(engine: &TestEngine, email: &str) {
    let _writes = engine.writes.write().unwrap();
    engine
        .acct
        .advance_session_generation_holding_session_write_lock(SessionTransition::Write);
    *engine.acct.session.lock().unwrap() = Some(Session {
        token: "an-invented-token".into(),
        master_key: [7; 32],
        email: Some(email.into()),
    });
    engine.set_auth_present(true);
}

fn row(db: &StateDb, id: &str, path: &str, size: i64, folder: bool) {
    let entry = FileEntry {
        file_id: id.into(),
        path: path.into(),
        status: FileStatus::Local,
        size_bytes: size,
        modified_at: 1,
        content_hash: None,
        remote_updated_at: 1,
        parent_id: None,
        item_kind: if folder { ItemKind::Folder } else { ItemKind::File },
    };
    db.upsert_file_with_contract(&entry, |contract| contract.item_kind = entry.item_kind.clone())
        .unwrap();
}

fn local_state(engine: &TestEngine, owner: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let db = StateDb::open(&path).unwrap();
    row(&db, "d-trip", "Trip", 0, true);
    row(&db, "f-note", "Trip/Invented note.txt", 1_000, false);
    row(&db, "f-plan", "Invented plan.pdf", 500, false);
    row(&db, "f-shot", "Trip/Invented shot.jpg", 30_000, false);
    row(&db, "f-clip", "Invented clip.mov", 200_000, false);
    row(&db, "d-backups", "Backups", 0, true);
    db.set_system_role("d-backups", Some(beebeeb_types::backup::SystemRole::BackupsRoot))
        .unwrap();
    row(
        &db,
        "f-backed",
        "Backups/Invented phone/Invented backed.jpg",
        9_000_000,
        false,
    );
    // The folder is kept: both files in it are, and the folder itself is no file.
    db.set_recursive_pin("d-trip", true, 1).unwrap();
    db.set_owner(&Identity::new(None, Some(owner))).unwrap();
    engine.use_local_state_for_test(path);
    dir
}

#[test]
fn the_facts_count_the_kept_files_and_split_the_bytes_outside_backups() {
    let engine = engine();
    signed_in(&engine, "facts-one@example.eu");
    let _dir = local_state(&engine, "facts-one@example.eu");
    let facts = engine.files_facts().expect("the facts");
    assert_eq!(facts.session, engine.session_turn());
    assert_eq!(facts.available_offline, 2, "the note and the shot, not their folder");
    assert_eq!(facts.files_bytes, 1_500, "the note and the plan");
    assert_eq!(
        facts.photos_bytes, 230_000,
        "the shot and the clip; what is in Backups is neither"
    );
}

#[test]
fn the_facts_answer_nothing_signed_out_or_for_another_accounts_state() {
    let engine = engine();
    assert_eq!(engine.files_facts(), Err(LocalStateRefused::SignedOut));
    signed_in(&engine, "facts-two@example.eu");
    let _dir = local_state(&engine, "facts-one@example.eu");
    assert_eq!(engine.files_facts(), Err(LocalStateRefused::OtherAccount));
}
