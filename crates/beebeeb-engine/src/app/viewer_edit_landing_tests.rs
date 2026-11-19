//! The fate of a viewer save's own upload (`viewer_edit_landing.rs`), against the local state alone. Every name, key and
//! byte is invented.

use std::sync::Arc;

use super::viewer_edit_landing::{Fate, look, look_with_head};
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

const DOC: &str = "b0000000-0000-4000-8000-000000000701";
const USER: &str = "an-invented-landing-user";
const EMAIL: &str = "landing@example.eu";

fn engine() -> Arc<TestEngine> {
    let state_dir = crate::state_paths::init_for_test();
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "an-invented-landing-slot".into(),
        state_dir,
        // A closed loopback port: nothing is sent, every save stays queued here.
        api_base: "http://127.0.0.1:9".into(),
    };
    Arc::new(Engine::open(&TEST, config, host, MemoryCredentials::default()).expect("the handle opens"))
}

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

fn set_version(db: &StateDb, version: i64) {
    let mut contract = db.get_file_contract_state(DOC).unwrap().unwrap();
    contract.current_version = version;
    db.set_file_contract_state(&contract).unwrap();
}

/// A signed-in engine with a writable document at version 4, and one save from version 4 queued.
fn queued_save() -> (Arc<TestEngine>, tempfile::TempDir, tempfile::TempDir, StateDb, QueuedSave) {
    let engine = engine();
    sign_in(&engine);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let db = StateDb::open(&path).unwrap();
    let entry = FileEntry {
        file_id: DOC.into(),
        path: "Invented doc.txt".into(),
        status: FileStatus::Local,
        size_bytes: 3,
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
    set_version(&db, 4);
    db.set_owner(&Identity::new(Some(USER), Some(EMAIL))).unwrap();
    engine.use_local_state_for_test(path);
    let root = tempfile::tempdir().unwrap();
    engine.use_sync_root_for_test(root.path().to_path_buf());
    let session = engine.acct.session_generation();
    let opening = engine.viewer_opening(session, DOC);
    assert_eq!(engine.save_text_for(session, DOC, "an invented edit", 4, opening), Ok(SaveEnd::Queued { base: 4 }));
    let op = db.list_operations_for_file(DOC).unwrap().remove(0);
    let token = db.held_write(DOC).unwrap().map(|held| held.token().to_string());
    let queued = QueuedSave { session, epoch: opening, base: 4, op_id: op.op_id, token };
    (engine, dir, root, db, queued)
}

fn park_stale(db: &StateDb, op_id: &str) {
    db.test_execute(&format!(
        "UPDATE operation_queue SET attempts = max_attempts, last_error = 'stale_base', last_error_class = 'stale_base'
         WHERE op_id = '{op_id}'"
    ))
    .unwrap();
}

#[test]
fn a_queued_save_is_still_waiting() {
    let (_e, _d, _r, db, queued) = queued_save();
    assert_eq!(look(&db, DOC, &queued, 4).unwrap(), Fate::Waiting);
}

/// v5 bug 8: the server answered 409 on the upload's init (another client saved first). The save is a conflict, and it
/// leaves nothing in the queue, so Keep mine can queue a new save.
#[test]
fn a_save_the_server_refused_as_stale_is_a_conflict_never_saved() {
    let (engine, _d, _r, db, queued) = queued_save();
    park_stale(&db, &queued.op_id);
    // The pull has not seen the newer version yet: not saved, not yet a question either.
    assert_eq!(look(&db, DOC, &queued, 4).unwrap(), Fate::ParkedStale);
    set_version(&db, 5);
    assert_eq!(look(&db, DOC, &queued, 4).unwrap(), Fate::Conflict { theirs: 5 });
    assert!(db.list_operations_for_file(DOC).unwrap().is_empty(), "the refused save is out of the queue");
    assert!(db.held_write(DOC).unwrap().is_none(), "and no write is held for it");
    // Keep mine: saved from the version the file is at now, queued behind nothing.
    let session = engine.acct.session_generation();
    assert_eq!(engine.save_text_for(session, DOC, "an invented edit", 5, queued.epoch), Ok(SaveEnd::Queued { base: 5 }));
}

#[test]
fn a_save_parked_for_another_reason_is_not_saved() {
    let (_e, _d, _r, db, queued) = queued_save();
    db.test_execute(&format!(
        "UPDATE operation_queue SET attempts = max_attempts, last_error = 'permission_denied' WHERE op_id = '{}'",
        queued.op_id
    ))
    .unwrap();
    assert_eq!(look(&db, DOC, &queued, 4).unwrap(), Fate::Failed);
    assert_eq!(db.list_operations_for_file(DOC).unwrap().len(), 1, "it stays, for Sync history");
}

#[test]
fn an_op_that_left_the_queue_without_landing_is_not_saved() {
    let (_e, _d, _r, db, queued) = queued_save();
    db.test_execute(&format!("DELETE FROM operation_queue WHERE op_id = '{}'", queued.op_id)).unwrap();
    assert_eq!(look(&db, DOC, &queued, 4).unwrap(), Fate::Gone);
}

/// w27: the other client saved ~7 s before Ctrl+S, so this PC's pull has not seen the newer version (the file is still
/// at 4 here) while the server refused the upload as stale. The server's own answer (head 5) is a conflict AT ONCE, the
/// refused save is out of the queue, and Keep mine saves from 5 behind nothing, without waiting for the pull.
#[test]
fn a_stale_refusal_is_a_conflict_at_once_when_the_server_names_the_head() {
    let (engine, _d, _r, db, queued) = queued_save();
    park_stale(&db, &queued.op_id);
    // Without the server's answer: still not decided (the pull has not seen version 5).
    assert_eq!(look_with_head(&db, DOC, &queued, 4, None).unwrap(), Fate::ParkedStale);
    // With it: the question, now.
    assert_eq!(look_with_head(&db, DOC, &queued, 4, Some((5, 3))).unwrap(), Fate::Conflict { theirs: 5 });
    assert!(db.list_operations_for_file(DOC).unwrap().is_empty(), "the refused save is out of the queue");
    assert!(db.held_write(DOC).unwrap().is_none(), "and no write is held for it");
    assert_eq!(db.get_file_contract_state(DOC).unwrap().unwrap().current_version, 5, "this PC now knows the head");
    // A head that is not past the base decides nothing.
    let (_e2, _d2, _r2, db2, queued2) = queued_save();
    park_stale(&db2, &queued2.op_id);
    assert_eq!(look_with_head(&db2, DOC, &queued2, 4, Some((4, 3))).unwrap(), Fate::ParkedStale);
    // Keep mine: from the head, queued behind nothing.
    let session = engine.acct.session_generation();
    assert_eq!(engine.save_text_for(session, DOC, "an invented edit", 5, queued.epoch), Ok(SaveEnd::Queued { base: 5 }));
}
