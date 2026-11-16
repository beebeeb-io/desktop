//! `Engine::uploads_for`, `retry_upload_for` and `cancel_upload_for` with the engine's own parts: a real handle, a
//! session put in memory, a state database of the test's own, a board of the host. Every name is invented.

use std::sync::Arc;

use super::super::{Engine, EngineConfig};
use super::*;
use crate::account::SessionTransition;
use crate::account_binding::Identity;
use crate::host::ChannelHost;
use crate::identity::TEST;
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
        Arc::new(Engine::open(&TEST, config, host, MemoryCredentials::default()).expect("the handle opens")),
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
        status: FileStatus::Uploading,
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

/// A copy of its own for one queued upload, in a directory that lives as long as the process (the test binary itself
/// can be replaced by a rebuild during a full run, and a queued upload's copy that is gone is refused).
fn payload_copy(id: &str) -> String {
    static DIR: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let dir = DIR.get_or_init(|| tempfile::tempdir().expect("a directory for the uploads' copies"));
    let copy = dir.path().join(format!(
        "{id}-{}",
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::write(&copy, b"an invented copy").expect("a copy to upload");
    copy.to_string_lossy().into_owned()
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
        payload_path: Some(payload_copy(id)),
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
    std::fs::create_dir(dir.path().join("sync")).unwrap();
    engine.use_sync_root_for_test(dir.path().join("sync"));
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

/// Review of desktop#423, P1 #2: the runner's own claim, on its own connection, at the moment the copy goes to the
/// bin (the `recycle` closure is the hook between the cancel's steps). It must find nothing it can run: the cancel
/// holds the queued change, or has dropped it, before the copy moves. A claim that waited would only find it gone,
/// so the connection waits for nothing (no timing in the test).
#[test]
fn the_engine_cannot_take_the_upload_while_its_copy_goes_to_the_bin() {
    let (engine, _board) = engine("an-invented-uploads-slot-8");
    signed_in(&engine, "uploads-one@example.eu");
    let (_dir, db) = local_state(&engine, "uploads-one@example.eu", three);
    db.hold_lock_for_test().busy_timeout(std::time::Duration::ZERO).unwrap();
    let claimed = std::cell::RefCell::new(None);
    let recycle = |_: &Path| {
        *claimed.borrow_mut() = Some(db.claim_operation("op-f-waits", unix_now()));
        Ok(())
    };
    let outcome = engine
        .cancel_upload_for(engine.session_turn(), "op-f-waits", &recycle)
        .unwrap();
    let claimed = claimed.into_inner().expect("the copy was asked to go to the bin");
    assert!(
        !matches!(claimed, Ok(crate::state_db::ClaimOutcome::Claimed(_))),
        "the engine took the upload of a file whose copy is in the bin (cancel answered {outcome:?})"
    );
    assert_eq!(
        outcome,
        CancelOutcome::Cancelled,
        "the copy went to the bin: the upload is cancelled"
    );
    let ids: Vec<String> = db
        .unsent_changes(0, 10, unix_now())
        .unwrap()
        .items
        .into_iter()
        .map(|c| c.id)
        .collect();
    assert!(!ids.contains(&"op-f-waits".to_string()), "{ids:?}");
}

/// `C:\a\b` as the computer's own admin share, `\\localhost\C$\a\b`: the same file, on a path with no Recycle Bin.
#[cfg(target_os = "windows")]
fn on_the_admin_share(path: &Path) -> std::path::PathBuf {
    let text = path.to_str().expect("a temp path is text");
    let text = text.strip_prefix(r"\\?\").unwrap_or(text);
    let (drive, rest) = text.split_once(':').expect("a drive letter");
    std::path::PathBuf::from(format!(r"\\localhost\{drive}${rest}"))
}

/// Review of desktop#423, P1 #1 (Windows rung): a file whose volume has no Recycle Bin is never deleted for good.
/// The share is the computer's own `C$`, which the build VM serves; elsewhere it may not be reachable.
#[cfg(target_os = "windows")]
#[test]
#[ignore = "needs the computer's own admin share (\\\\localhost\\C$): run on the Windows build VM with --ignored"]
fn a_copy_on_a_path_without_a_recycle_bin_is_refused_and_kept() {
    let dir = tempfile::tempdir().unwrap();
    let local = dir.path().join("Invented only copy.txt");
    std::fs::write(&local, b"the only copy").unwrap();
    let share = on_the_admin_share(&local);
    assert!(
        std::fs::metadata(&share).is_ok(),
        "the admin share is not reachable here: this rung proves nothing"
    );
    let moved = move_to_recycle_bin(&share);
    assert!(
        local.exists(),
        "the only copy was deleted for good (the move answered {moved:?})"
    );
    assert!(moved.is_err(), "a refusal must be said");
}

/// The same rung's other half: on this PC's own drive the copy goes to the Recycle Bin.
#[cfg(target_os = "windows")]
#[test]
fn a_copy_on_this_pcs_drive_goes_to_the_recycle_bin() {
    let dir = tempfile::tempdir().unwrap();
    let local = dir.path().join("Invented to the bin.txt");
    std::fs::write(&local, b"recoverable").unwrap();
    move_to_recycle_bin(&local).expect("moved to the bin");
    assert!(!local.exists());
    // A copy that is already gone counts as moved.
    move_to_recycle_bin(&local).expect("already gone");
}
