//! `Engine::uploads_for`, `retry_upload_for` and `cancel_upload_for` with the engine's own parts: a real handle, a
//! session put in memory, a state database of the test's own, a board of the host. Every name is invented.

use std::sync::Arc;

use super::super::{Engine, EngineConfig};
use super::*;
use crate::account::SessionTransition;
use crate::account_binding::Identity;
use crate::host::ChannelHost;
use crate::identity::REAL;
use crate::session::Session;
use crate::signin::memory_store::MemoryCredentials;
use crate::state_db::{FileEntry, FileStatus, ItemKind, OperationKind, PendingOperation, StateDb};
use crate::transfer_progress::TransferBoard;

type TestEngine = Engine<MemoryCredentials>;

fn engine(slot: &str) -> (Arc<TestEngine>, Arc<TransferBoard>) {
    let state_dir = crate::state_paths::init_for_test();
    let (host, _messages) = ChannelHost::new(|| {});
    let board = host.transfers();
    let config = EngineConfig {
        account_id: slot.into(),
        state_dir,
        api_base: "http://127.0.0.1:9".into(),
    };
    (
        Arc::new(Engine::open(&REAL, config, host, MemoryCredentials::default()).expect("the handle opens")),
        board,
    )
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

fn file(db: &StateDb, id: &str, path: &str, size: i64) {
    let entry = FileEntry {
        file_id: id.into(),
        path: path.into(),
        status: FileStatus::Local,
        size_bytes: size,
        modified_at: 1,
        content_hash: None,
        remote_updated_at: 1,
        parent_id: None,
        item_kind: ItemKind::File,
    };
    db.upsert_file_with_contract(&entry, |contract| contract.item_kind = entry.item_kind.clone())
        .unwrap();
}

/// A queued upload of `id`: `new` a new file's, else an edit; `attempts` of 5 is a give-up.
fn queued(db: &StateDb, id: &str, new: bool, attempts: i64, at: i64) {
    let op = PendingOperation {
        op_id: format!("op-{id}"),
        kind: OperationKind::UploadVersion,
        file_id: Some(id.into()),
        parent_id: None,
        target_path: None,
        metadata_json: Some(format!(
            r#"{{"operation":"{}"}}"#,
            if new { "create_file" } else { "update_file" }
        )),
        payload_path: None,
        base_version: None,
        base_object_version_id: None,
        attempts,
        max_attempts: 5,
        next_retry_at: 0,
        last_error: None,
        backup_source_key: None,
        created_at: at,
        updated_at: at,
    };
    db.enqueue_operation(&op).unwrap();
}

fn local_state(engine: &TestEngine, owner: &str, build: impl FnOnce(&StateDb)) -> (tempfile::TempDir, StateDb) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let db = StateDb::open(&path).unwrap();
    build(&db);
    db.set_owner(&Identity::new(None, Some(owner))).unwrap();
    engine.use_local_state_for_test(path);
    (dir, db)
}

fn three(db: &StateDb) {
    file(db, "f-waits", "Invented waits.txt", 300);
    file(db, "f-moves", "Trip/Invented moves.mov", 9_000);
    file(db, "f-stops", "Invented stops.pdf", 40);
    queued(db, "f-waits", true, 0, 30);
    queued(db, "f-moves", true, 0, 20);
    queued(db, "f-stops", true, 5, 10);
}

#[test]
fn uploads_list_this_accounts_stopped_moving_and_waiting_rows() {
    let (engine, board) = engine("an-invented-uploads-slot-1");
    signed_in(&engine, "uploads-one@example.eu");
    let (_dir, _db) = local_state(&engine, "uploads-one@example.eu", three);
    let guard = board.begin_named("f-moves", "Trip/Invented moves.mov", Direction::Up, 9_000);
    guard.update(4_000);
    let read = engine.uploads_for(engine.session_turn()).expect("the read");
    let seen: Vec<(&str, &str)> = read
        .rows
        .iter()
        .map(|r| (r.file_id.as_str(), r.stands_word()))
        .collect();
    assert_eq!(
        seen,
        vec![("f-stops", "stopped"), ("f-moves", "moving"), ("f-waits", "waiting")]
    );
    assert_eq!(
        read.rows[1].stands,
        UploadStands::Moving {
            sent: 4_000,
            total: 9_000
        }
    );
    assert_eq!(read.rows[1].name, "Invented moves.mov");
    assert_eq!(read.rows[1].folder, "Trip");
    assert_eq!(read.rows[2].size, 300);
    assert!(
        matches!(&read.rows[0].stands, UploadStands::Stopped { words, retryable: true } if words.starts_with("Stopped after"))
    );
    assert!(!read.paused);
    drop(guard);
}

#[test]
fn a_board_row_of_another_accounts_file_is_not_listed() {
    let (engine, board) = engine("an-invented-uploads-slot-2");
    signed_in(&engine, "uploads-one@example.eu");
    let (_dir, _db) = local_state(&engine, "uploads-one@example.eu", three);
    let _theirs = board.begin_named("f-theirs", "Invented theirs.txt", Direction::Up, 100);
    let read = engine.uploads_for(engine.session_turn()).expect("the read");
    assert_eq!(read.rows.len(), 3);
    assert!(read.rows.iter().all(|r| r.file_id != "f-theirs"));
}

#[test]
fn uploads_for_a_session_that_moved_on_are_refused() {
    let (engine, _board) = engine("an-invented-uploads-slot-3");
    signed_in(&engine, "uploads-one@example.eu");
    let (_dir, _db) = local_state(&engine, "uploads-one@example.eu", three);
    let before = engine.session_turn();
    signed_in(&engine, "uploads-one@example.eu");
    assert_eq!(
        engine.uploads_for(before).unwrap_err(),
        LocalStateRefused::SessionChanged
    );
    assert_eq!(
        engine.retry_upload_for(before, "op-f-stops").unwrap_err(),
        LocalStateRefused::SessionChanged
    );
}

#[test]
fn retry_makes_one_change_due_and_leaves_the_others() {
    let (engine, _board) = engine("an-invented-uploads-slot-4");
    signed_in(&engine, "uploads-one@example.eu");
    let (_dir, db) = local_state(&engine, "uploads-one@example.eu", |db| {
        three(db);
        file(db, "f-gone", "Invented gone.txt", 5);
        queued(db, "f-gone", true, 5, 40);
    });
    let session = engine.session_turn();
    assert_eq!(
        engine.retry_upload_for(session, "op-f-stops").unwrap(),
        RetryOutcome::Retried
    );
    assert_eq!(
        engine.retry_upload_for(session, "op-f-stops").unwrap(),
        RetryOutcome::AlreadyQueued
    );
    // The other given-up row was not touched.
    let page = db.unsent_changes(0, 10, unix_now()).unwrap();
    let gone = page.items.iter().find(|c| c.id == "op-f-gone").unwrap();
    assert!(
        gone.retryable,
        "still stopped, still retryable: only the one row changed"
    );
    assert_eq!(
        engine.retry_upload_for(session, "op-nothing").unwrap(),
        RetryOutcome::Unknown
    );
}

#[test]
fn cancel_takes_a_new_files_copy_to_the_bin_and_drops_its_change_only() {
    let (engine, _board) = engine("an-invented-uploads-slot-5");
    signed_in(&engine, "uploads-one@example.eu");
    let (_dir, db) = local_state(&engine, "uploads-one@example.eu", |db| {
        three(db);
        file(db, "f-edit", "Invented edit.txt", 8);
        queued(db, "f-edit", false, 0, 50);
    });
    let session = engine.session_turn();
    let asked = std::cell::RefCell::new(Vec::new());
    let recycle = |p: &Path| {
        asked.borrow_mut().push(p.to_path_buf());
        Ok(())
    };
    assert_eq!(
        engine.cancel_upload_for(session, "op-f-waits", &recycle).unwrap(),
        CancelOutcome::Cancelled
    );
    assert_eq!(asked.borrow().len(), 1);
    assert!(asked.borrow()[0].ends_with("Invented waits.txt"));
    let ids: Vec<String> = db
        .unsent_changes(0, 10, unix_now())
        .unwrap()
        .items
        .into_iter()
        .map(|c| c.id)
        .collect();
    assert_eq!(ids.len(), 3, "one change fewer, the others stay: {ids:?}");
    assert!(!ids.contains(&"op-f-waits".to_string()));
    assert!(
        db.sizes_of(&["f-waits".to_string()]).unwrap().is_empty(),
        "its row went"
    );
    // An edit is never cancelled here, and its copy is never asked for.
    assert_eq!(
        engine.cancel_upload_for(session, "op-f-edit", &recycle).unwrap(),
        CancelOutcome::NotNew
    );
    assert_eq!(asked.borrow().len(), 1);
}

#[test]
fn a_copy_that_cannot_go_to_the_bin_leaves_everything_as_it_was() {
    let (engine, _board) = engine("an-invented-uploads-slot-6");
    signed_in(&engine, "uploads-one@example.eu");
    let (_dir, db) = local_state(&engine, "uploads-one@example.eu", three);
    let refused = |_: &Path| Err(std::io::Error::other("no"));
    assert_eq!(
        engine
            .cancel_upload_for(engine.session_turn(), "op-f-waits", &refused)
            .unwrap(),
        CancelOutcome::RecycleFailed
    );
    assert_eq!(db.unsent_changes(0, 10, unix_now()).unwrap().total, 3);
    assert_eq!(db.sizes_of(&["f-waits".to_string()]).unwrap().len(), 1);
}

#[test]
fn cancel_for_a_session_that_moved_on_changes_nothing() {
    let (engine, _board) = engine("an-invented-uploads-slot-7");
    signed_in(&engine, "uploads-one@example.eu");
    let (_dir, db) = local_state(&engine, "uploads-one@example.eu", three);
    let before = engine.session_turn();
    signed_in(&engine, "uploads-one@example.eu");
    let ok = |_: &Path| Ok(());
    assert_eq!(
        engine.cancel_upload_for(before, "op-f-waits", &ok).unwrap_err(),
        LocalStateRefused::SessionChanged
    );
    assert_eq!(db.unsent_changes(0, 10, unix_now()).unwrap().total, 3);
}
