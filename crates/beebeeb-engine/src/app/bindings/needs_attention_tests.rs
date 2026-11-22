//! LD-0105 Amendment 4 (desktop #225) in the engine: the needs-attention rows (`transfer_progress::needs_attention`,
//! `with_attention`, `AttentionCache`, `status_views::shown_transfers`) and `Engine::{needs_attention_for,
//! discard_and_download_again_for}` with a real handle, a state database of the test's own and the host's board.
//! The first four tests are the shell's LD-0105 tests, moved here; the rest are the engine call's. Every name is
//! invented.

use std::cell::Cell;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use super::*;
use crate::account::SessionTransition;
use crate::account_binding::Identity;
use crate::app::EngineConfig;
use crate::host::ChannelHost;
use crate::identity::TEST;
use crate::session::Session;
use crate::signin::memory_store::MemoryCredentials;
use crate::state_db::{FileEntry, FileStatus, ItemKind, StateDb};
use crate::transfer_progress::{AttentionCache, attention_for_poll, with_attention};

type TestEngine = Engine<MemoryCredentials>;

const OWNER: &str = "attention-one@example.eu";

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

fn local_state(engine: &TestEngine, owner: &str, build: impl FnOnce(&StateDb)) -> (tempfile::TempDir, StateDb) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let db = StateDb::open(&path).unwrap();
    build(&db);
    db.set_owner(&Identity::new(None, Some(owner))).unwrap();
    engine.use_local_state_for_test(path);
    (dir, db)
}

/// A files row for `id` at `path`, as the failed download left it.
fn file(db: &StateDb, id: &str, path: &str) {
    db.upsert_file(&FileEntry {
        file_id: id.into(),
        path: path.into(),
        status: FileStatus::Error,
        size_bytes: 10,
        modified_at: 0,
        content_hash: None,
        remote_updated_at: 0,
        parent_id: None,
        item_kind: ItemKind::File,
    })
    .unwrap();
}

/// A poisoned file that changed on this device since: it needs the person.
fn changed(db: &StateDb, id: &str, path: &str) {
    file(db, id, path);
    db.record_poison(id, "corrupt", Some((1, 10)), 1).unwrap();
    db.mark_poison_needs_attention(id).unwrap();
}

/// A discard that reaches its step ends with a forget on the process's kept rows ([`ATTENTION_CACHE`]): the tests
/// whose step answers `Ok` take this, so one never raises the fence under the other's read.
static KEPT_ROWS: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// What the discard's step was handed, and how often.
#[derive(Default)]
struct Steps {
    reached: Cell<usize>,
    last: std::cell::RefCell<String>,
}

impl Steps {
    fn step(&self) -> impl FnOnce(&str) -> Result<(), DiscardRefusal> + '_ {
        move |id: &str| {
            self.reached.set(self.reached.get() + 1);
            *self.last.borrow_mut() = id.to_string();
            Ok(())
        }
    }
}

// ---- The shell's LD-0105 tests, moved (the source names are in the branch notes) ----

/// A poisoned file that changed on this device is one row with its reason and its one action; a poison the sweep
/// may still empty is no row of its own. Red on the mutant "every poison is a needs-attention row" (the
/// `needs_attention` filter removed).
#[test]
fn a_changed_poisoned_file_is_one_row_with_its_reason_and_its_one_action() {
    let dir = tempfile::tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    for (id, path) in [("changed", "Films/holiday.mp4"), ("emptying", "Films/other.mp4")] {
        file(&db, id, path);
        db.record_poison(id, "corrupt", Some((1, 10)), 1).unwrap();
    }
    db.mark_poison_needs_attention("changed").unwrap();

    let attention = crate::transfer_progress::needs_attention(&db).expect("the read");
    assert_eq!(
        attention,
        vec![NeedsAttention {
            file_id: "changed".into(),
            path: "Films/holiday.mp4".into(),
            size: 10
        }],
        "only the changed one needs the person"
    );
    let rows = with_attention(Vec::new(), &attention);
    assert_eq!(rows.len(), 1);
    assert_eq!(
        serde_json::to_value(&rows[0]).unwrap(),
        serde_json::json!({"fileId": "changed", "name": "holiday.mp4", "folder": "Films", "direction": "down", "done": 0,
            "total": 10, "buffered": 0, "speedBps": 0, "attention": "changed_on_this_device",
            "action": "discard_and_download_again", "failed": false, "failedAtSecs": null})
    );
}

/// The board carries a needs-attention row until the poison row ends (the discard's last step); a file on the board
/// carries it on its own row; a poison the sweep may still empty is no row; a poison with no file row is no row; an
/// upload row never offers the discard. Red on the mutants "the board alone" (no attention rows), "every poison is a
/// needs-attention row" and "an upload row offers the discard".
#[test]
fn the_board_carries_a_needs_attention_row_until_the_discard_ends_it() {
    let dir = tempfile::tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    for (id, path) in [
        ("changed", "Films/holiday.mp4"),
        ("emptying", "Films/other.mp4"),
        ("live", "clip.mp4"),
        ("uploading", "notes.txt"),
    ] {
        file(&db, id, path);
    }
    // "orphan" has no files row: it could never be discarded (`missing`), so it is no row.
    for id in ["changed", "emptying", "uploading", "orphan"] {
        db.record_poison(id, "corrupt", Some((1, 10)), 1).unwrap();
    }
    for id in ["changed", "uploading", "orphan"] {
        db.mark_poison_needs_attention(id).unwrap();
    }
    let board = TransferBoard::new();
    let _live = board.begin_named("live", "clip.mp4", Direction::Stream, 10);
    let _up = board.begin_named("uploading", "notes.txt", Direction::Up, 10);
    let json = |db: Option<&StateDb>| serde_json::to_value(board.snapshot_with_attention(db)).unwrap();

    let rows = json(Some(&db));
    let rows = rows.as_array().unwrap();
    assert_eq!(
        rows.len(),
        3,
        "the live transfer, the upload and the changed file (no orphan row): {rows:?}"
    );
    assert!(
        rows.iter().all(|r| r["fileId"] != "orphan"),
        "a poison with no file row is no row"
    );
    let up = rows.iter().find(|r| r["fileId"] == "uploading").unwrap();
    assert!(
        up.get("attention").is_none() && up.get("action").is_none(),
        "an upload row keeps its own shape: {up}"
    );
    let changed = rows
        .iter()
        .find(|r| r["fileId"] == "changed")
        .expect("the changed file has a row");
    assert_eq!(
        *changed,
        serde_json::json!({"fileId": "changed", "name": "holiday.mp4", "folder": "Films", "direction": "down", "done": 0,
            "total": 10, "buffered": 0, "speedBps": 0, "attention": "changed_on_this_device",
            "action": "discard_and_download_again", "failed": false, "failedAtSecs": null})
    );
    let live = rows.iter().find(|r| r["fileId"] == "live").unwrap();
    assert!(
        live.get("attention").is_none() && live.get("action").is_none(),
        "a healthy row keeps its shape: {live}"
    );

    // The discard's last step ends the poison row: the row goes with it.
    assert!(db.clear_poison("changed").unwrap());
    let rows = json(Some(&db));
    assert_eq!(
        rows.as_array().unwrap().len(),
        2,
        "the live transfer and the upload: {rows}"
    );

    // A file on the board that needs the person carries it on its own row (no second row).
    db.record_poison("live", "corrupt", Some((1, 10)), 1).unwrap();
    db.mark_poison_needs_attention("live").unwrap();
    let rows = json(Some(&db));
    let rows = rows.as_array().unwrap();
    assert_eq!(rows.len(), 2);
    let live = rows.iter().find(|r| r["fileId"] == "live").unwrap();
    assert_eq!(
        (live["direction"].as_str(), live["attention"].as_str()),
        (Some("stream"), Some("changed_on_this_device"))
    );

    // The read itself names no orphan either: every needs-attention row has an exit.
    let read = crate::transfer_progress::needs_attention(&db).expect("the read");
    assert!(read.iter().all(|r| r.file_id != "orphan"), "no row without a file row");
    assert!(read.iter().any(|r| r.file_id == "live"));

    // No DB: the board alone.
    assert!(
        json(None)
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r.get("attention").is_none())
    );
}

/// No file name while the vault is locked: the transfers shown add the needs-attention rows only while unlocked, and
/// the engine's read of them is refused on a locked vault (the shell's `vault_unlocked` half). Red on the mutants
/// "`shown_transfers` ignores the lock" and "`needs_attention_for` reads past the person's door".
#[test]
fn a_locked_vault_shows_no_needs_attention_row() {
    let attention = [NeedsAttention {
        file_id: "changed".into(),
        path: "Films/holiday.mp4".into(),
        size: 10,
    }];
    let board = TransferBoard::new();
    let shown = |unlocked| {
        serde_json::to_string(&crate::status_views::shown_transfers(
            board.snapshot_dtos(),
            unlocked,
            &attention,
        ))
        .unwrap()
    };
    let locked = shown(false);
    assert!(
        !locked.contains("holiday.mp4") && !locked.contains("changed_on_this_device"),
        "locked: no name, no row: {locked}"
    );
    assert!(
        shown(true).contains("holiday.mp4"),
        "the control: unlocked, the row shows"
    );

    let (engine, _board) = engine("an-invented-attention-slot-locked-read");
    let (_dir, _db) = local_state(&engine, OWNER, |db| changed(db, "f-changed", "Invented/holiday.mp4"));
    // A token is stored, no keys are in memory: locked.
    engine.set_auth_present(true);
    assert_eq!(
        engine.needs_attention_for(engine.session_turn()).unwrap_err(),
        LocalStateRefused::Locked
    );
    // The control: the same state, unlocked, names the file.
    signed_in(&engine, OWNER);
    let rows = engine.needs_attention_for(engine.session_turn()).expect("the read");
    assert_eq!(
        rows.iter().map(|r| r.path.as_str()).collect::<Vec<_>>(),
        vec!["Invented/holiday.mp4"]
    );
}

/// The needs-attention rows kept for a slow read never reach another session. A's rows are kept under engine
/// generation 1; a sign-out or an account switch starts a new engine (generation 2), whose poll with a slow read
/// shows nothing of A's. No engine, or a lock, clears them. A discarded row never comes back from an older read. Red
/// on the mutants "the kept rows ignore the generation", "a lock or no engine keeps the rows", "no fence".
#[test]
fn kept_needs_attention_rows_never_reach_another_session() {
    let a = vec![NeedsAttention {
        file_id: "a-file".into(),
        path: "A/holiday.mp4".into(),
        size: 1,
    }];
    let mut cache = AttentionCache::new();
    cache.store(1, 1, a.clone());
    assert_eq!(
        attention_for_poll(&mut cache, true, Some(1), Some(a.clone())),
        a,
        "a fresh read"
    );
    assert_eq!(
        attention_for_poll(&mut cache, true, Some(1), None),
        a,
        "a slow read: the rows kept for this engine"
    );
    assert!(
        attention_for_poll(&mut cache, true, Some(2), None).is_empty(),
        "B (a new engine) never sees A's rows"
    );

    cache.store(1, 2, a.clone());
    assert!(
        attention_for_poll(&mut cache, true, None, None).is_empty(),
        "no engine: nothing"
    );
    assert!(
        attention_for_poll(&mut cache, true, Some(1), None).is_empty(),
        "no engine cleared the kept rows"
    );

    cache.store(1, 3, a.clone());
    assert!(
        attention_for_poll(&mut cache, false, Some(1), None).is_empty(),
        "locked: nothing"
    );
    assert!(
        attention_for_poll(&mut cache, true, Some(1), None).is_empty(),
        "a lock cleared the kept rows"
    );

    cache.store(1, 10, a.clone());
    cache.forget("a-file", 11);
    cache.store(1, 10, a.clone());
    assert!(
        attention_for_poll(&mut cache, true, Some(1), None).is_empty(),
        "a read started before the discard does not bring the row back"
    );
    cache.store(1, 11, a.clone());
    assert_eq!(
        attention_for_poll(&mut cache, true, Some(1), None),
        a,
        "a later read is kept"
    );
}

// ---- The engine's calls ----

/// Delta review P2-1 of desktop #225: an upload of the file on the board is the person's change being sent, so the
/// discard refuses it and its step is never reached; the poison stays. The control, in the same state: a file with a
/// download row (not an upload) reaches the step. Red on the mutant "drop the upload-row refusal" (the step then
/// runs for the uploading file).
#[test]
fn an_upload_of_the_file_is_never_discarded() {
    let _kept_rows = KEPT_ROWS.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let (engine, board) = engine("an-invented-attention-slot-upload");
    signed_in(&engine, OWNER);
    let (_dir, db) = local_state(&engine, OWNER, |db| {
        changed(db, "f-sending", "Invented/sending.txt");
        changed(db, "f-coming", "Invented/coming.txt");
    });
    let _up = board.begin_named("f-sending", "Invented/sending.txt", Direction::Up, 10);
    let _down = board.begin_download("f-coming", "Invented/coming.txt", 10);
    let steps = Steps::default();

    let answer = engine.discard_with(engine.session_turn(), "f-sending", steps.step());
    assert_eq!(answer, Err(DiscardRefusal::Uploading));
    assert_eq!(steps.reached.get(), 0, "nothing was emptied");
    assert!(db.poison("f-sending").unwrap().is_some(), "the poison stays");

    let answer = engine.discard_with(engine.session_turn(), "f-coming", steps.step());
    assert_eq!(answer, Ok(()), "the control: no upload row, the step runs");
    assert_eq!(steps.reached.get(), 1);
    assert_eq!(*steps.last.borrow(), "f-coming");
}

/// A press of an earlier session (a lock and an unlock, a sign-out and a sign-in since) discards nothing.
#[test]
fn a_discard_for_a_session_that_moved_on_reaches_no_step() {
    let (engine, _board) = engine("an-invented-attention-slot-moved");
    signed_in(&engine, OWNER);
    let (_dir, db) = local_state(&engine, OWNER, |db| changed(db, "f-changed", "Invented/holiday.mp4"));
    let before = engine.session_turn();
    signed_in(&engine, OWNER);
    let steps = Steps::default();
    assert_eq!(
        engine.discard_with(before, "f-changed", steps.step()),
        Err(DiscardRefusal::LocalState(LocalStateRefused::SessionChanged))
    );
    assert_eq!(steps.reached.get(), 0);
    assert!(db.poison("f-changed").unwrap().is_some());
}

/// A locked vault discards nothing: the person's door refuses before anything is read.
#[test]
fn a_locked_vault_reaches_no_step() {
    let (engine, _board) = engine("an-invented-attention-slot-locked");
    let (_dir, db) = local_state(&engine, OWNER, |db| changed(db, "f-changed", "Invented/holiday.mp4"));
    engine.set_auth_present(true);
    let steps = Steps::default();
    assert_eq!(
        engine.discard_with(engine.session_turn(), "f-changed", steps.step()),
        Err(DiscardRefusal::LocalState(LocalStateRefused::Locked))
    );
    assert_eq!(steps.reached.get(), 0);
    assert!(db.poison("f-changed").unwrap().is_some());
}

/// Only a poison in this account's own state is discarded: a file with no poison, an id with no row, and a local
/// state recorded as another account's never reach the step.
#[test]
fn only_a_poison_in_this_accounts_own_state_reaches_the_step() {
    let (other, _other_board) = engine("an-invented-attention-slot-other");
    let (engine, _board) = engine("an-invented-attention-slot-own");
    signed_in(&engine, OWNER);
    let (_dir, _db) = local_state(&engine, OWNER, |db| file(db, "f-fine", "Invented/fine.txt"));
    let steps = Steps::default();
    for id in ["f-fine", "no-such-row"] {
        assert_eq!(
            engine.discard_with(engine.session_turn(), id, steps.step()),
            Err(DiscardRefusal::NotPoisoned),
            "{id}"
        );
    }
    assert_eq!(steps.reached.get(), 0);

    signed_in(&other, OWNER);
    let (_dir, _db) = local_state(&other, "attention-two@example.eu", |db| {
        changed(db, "f-theirs", "Invented/theirs.txt")
    });
    let answer = other.discard_with(other.session_turn(), "f-theirs", steps.step());
    assert!(
        matches!(answer, Err(DiscardRefusal::LocalState(refused)) if refused.not_this_accounts()),
        "another account's state: {answer:?}"
    );
    assert_eq!(steps.reached.get(), 0);
}

/// A discard that ended the row takes it out of the kept rows, and a read that started before it does not bring it
/// back; a refused discard leaves the kept rows as they were. The process's kept rows
/// are shared, so this test keeps its own generation. Red on the mutant "no forget after the discard".
#[test]
fn a_discard_that_ended_the_row_takes_it_out_of_the_kept_rows() {
    const GENERATION: u64 = 0x0a77_e417_10e5;
    let row = |id: &str| NeedsAttention {
        file_id: id.into(),
        path: format!("Invented/{id}.txt"),
        size: 10,
    };
    let _kept_rows = KEPT_ROWS.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let (engine, board) = engine("an-invented-attention-slot-kept");
    signed_in(&engine, OWNER);
    let (_dir, _db) = local_state(&engine, OWNER, |db| {
        changed(db, "f-done", "Invented/f-done.txt");
        changed(db, "f-sending", "Invented/f-sending.txt");
    });
    let _up = board.begin_named("f-sending", "Invented/f-sending.txt", Direction::Up, 10);
    let started = ATTENTION_READS.fetch_add(1, Ordering::SeqCst);
    ATTENTION_CACHE
        .lock()
        .unwrap()
        .store(GENERATION, started, vec![row("f-done"), row("f-sending")]);
    let kept = || {
        let mut ids: Vec<String> = ATTENTION_CACHE
            .lock()
            .unwrap()
            .rows_for(GENERATION)
            .into_iter()
            .map(|r| r.file_id)
            .collect();
        ids.sort();
        ids
    };

    let steps = Steps::default();
    assert_eq!(
        engine.discard_with(engine.session_turn(), "f-sending", steps.step()),
        Err(DiscardRefusal::Uploading)
    );
    assert_eq!(kept(), vec!["f-done", "f-sending"], "a refused discard forgets nothing");

    assert_eq!(
        engine.discard_with(engine.session_turn(), "f-done", steps.step()),
        Ok(())
    );
    assert_eq!(kept(), vec!["f-sending"], "the discarded row left the kept rows");
    ATTENTION_CACHE
        .lock()
        .unwrap()
        .store(GENERATION, started, vec![row("f-done"), row("f-sending")]);
    assert_eq!(
        kept(),
        vec!["f-sending"],
        "a read started before the discard does not bring it back"
    );
}

/// The engine's read names the changed files of this account only: not a poison the sweep may still empty, not a
/// poison with no file row; a session that moved on is refused.
#[test]
fn needs_attention_for_names_only_the_changed_files_of_this_session() {
    let (engine, _board) = engine("an-invented-attention-slot-read");
    signed_in(&engine, OWNER);
    let (_dir, _db) = local_state(&engine, OWNER, |db| {
        changed(db, "f-changed", "Invented/holiday.mp4");
        file(db, "f-emptying", "Invented/other.mp4");
        db.record_poison("f-emptying", "corrupt", Some((1, 10)), 1).unwrap();
        db.record_poison("f-orphan", "corrupt", Some((1, 10)), 1).unwrap();
        db.mark_poison_needs_attention("f-orphan").unwrap();
    });
    let rows = engine.needs_attention_for(engine.session_turn()).expect("the read");
    assert_eq!(
        rows,
        vec![NeedsAttention {
            file_id: "f-changed".into(),
            path: "Invented/holiday.mp4".into(),
            size: 10
        }]
    );
    let before = engine.session_turn();
    signed_in(&engine, OWNER);
    assert_eq!(
        engine.needs_attention_for(before).unwrap_err(),
        LocalStateRefused::SessionChanged
    );
}

/// No board shared with the host: an upload could not be ruled out, so nothing is discarded. A download row is no
/// upload.
#[test]
fn needs_attention_no_board_or_an_upload_row_refuses_the_discard() {
    assert_eq!(refuse_an_upload(None, "f-any"), Err(DiscardRefusal::NoBoard));
    let board = TransferBoard::new();
    assert_eq!(refuse_an_upload(Some(&board), "f-any"), Ok(()));
    let _down = board.begin_download("f-coming", "Invented/coming.txt", 10);
    assert_eq!(refuse_an_upload(Some(&board), "f-coming"), Ok(()));
    let _up = board.begin_named("f-sending", "Invented/sending.txt", Direction::Up, 10);
    assert_eq!(
        refuse_an_upload(Some(&board), "f-sending"),
        Err(DiscardRefusal::Uploading)
    );
}

/// Off Windows no file is ever poisoned by the provider: the public call answers "not supported" after its checks,
/// and nothing is cleared.
#[cfg(not(target_os = "windows"))]
#[test]
fn off_windows_the_discard_is_not_supported() {
    let (engine, _board) = engine("an-invented-attention-slot-platform");
    signed_in(&engine, OWNER);
    let (_dir, db) = local_state(&engine, OWNER, |db| changed(db, "f-changed", "Invented/holiday.mp4"));
    assert_eq!(
        engine.discard_and_download_again_for(engine.session_turn(), "f-changed"),
        Err(DiscardRefusal::NotSupported)
    );
    assert_eq!(DiscardRefusal::NotSupported.kind(), "not_supported");
    assert!(db.poison("f-changed").unwrap().is_some());
}
