//! `Engine::downloads_for` with a real handle, a state database of the test's own and a board of the host.
//! Every name is invented.

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
        status: FileStatus::Downloading,
        size_bytes: size,
        modified_at: 1,
        content_hash: None,
        remote_updated_at: 0,
        parent_id: None,
        item_kind: ItemKind::File,
    };
    db.upsert_file_with_contract(&entry, |contract| contract.item_kind = entry.item_kind.clone())
        .unwrap();
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

fn held(db: &StateDb) {
    file(db, "f-down", "Docs/Invented down.pdf", 9_000);
    file(db, "f-up", "Invented up.txt", 300);
    file(db, "f-stream", "Invented stream.mp4", 5_000);
    file(db, "f-broken", "Invented broken.bin", 70);
}

#[test]
fn a_download_is_listed_with_how_far_it_has_come() {
    let (engine, board) = engine("an-invented-downloads-slot-1");
    signed_in(&engine, "downloads-one@example.eu");
    let (_dir, _db) = local_state(&engine, "downloads-one@example.eu", held);
    let down = board.begin_download("f-down", "Docs/Invented down.pdf", 9_000);
    down.update(4_000);
    let read = engine.downloads_for(engine.session_turn()).expect("the read");
    assert_eq!(
        read,
        vec![DownloadRow {
            file_id: "f-down".into(),
            done: 4_000,
            total: 9_000
        }]
    );
    drop(down);
}

#[test]
fn an_upload_a_stream_and_a_failed_row_are_not_downloads() {
    let (engine, board) = engine("an-invented-downloads-slot-2");
    signed_in(&engine, "downloads-one@example.eu");
    let (_dir, _db) = local_state(&engine, "downloads-one@example.eu", held);
    let _up = board.begin_named("f-up", "Invented up.txt", Direction::Up, 300);
    let _stream = board.begin_named("f-stream", "Invented stream.mp4", Direction::Stream, 5_000);
    let _broken = board.begin_download("f-broken", "Invented broken.bin", 70);
    board.mark_failed("f-broken");
    let read = engine.downloads_for(engine.session_turn()).expect("the read");
    assert!(read.is_empty(), "no copy is coming down: {read:?}");
}

#[test]
fn a_board_row_of_a_file_this_account_does_not_hold_is_not_listed() {
    let (engine, board) = engine("an-invented-downloads-slot-3");
    signed_in(&engine, "downloads-one@example.eu");
    let (_dir, _db) = local_state(&engine, "downloads-one@example.eu", held);
    let _theirs = board.begin_download("f-theirs", "Invented theirs.txt", 100);
    let read = engine.downloads_for(engine.session_turn()).expect("the read");
    assert!(read.is_empty());
}

#[test]
fn downloads_for_a_session_that_moved_on_are_refused() {
    let (engine, board) = engine("an-invented-downloads-slot-4");
    signed_in(&engine, "downloads-one@example.eu");
    let (_dir, _db) = local_state(&engine, "downloads-one@example.eu", held);
    let _down = board.begin_download("f-down", "Docs/Invented down.pdf", 9_000);
    let before = engine.session_turn();
    signed_in(&engine, "downloads-one@example.eu");
    assert_eq!(
        engine.downloads_for(before).unwrap_err(),
        LocalStateRefused::SessionChanged
    );
}
