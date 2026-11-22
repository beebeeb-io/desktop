//! Tests of the session-end hook (a file of their own: the lock census does not read `_tests.rs`).

use super::*;
use crate::account::{SessionGeneration, SessionTransition};
use crate::host::ChannelHost;
use crate::identity::TEST;
use crate::session::Session;
use crate::signin::memory_store::MemoryCredentials;
use beebeeb_api::flows::LockState;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use zeroize::Zeroizing;

/// The tests share one state dir (and so one hook slot and one scratch folder): one at a time.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

const FILE: &str = "11111111-1111-4111-8111-111111111111";

#[test]
fn the_hook_of_a_state_dir_runs_with_the_ending_and_a_panic_is_caught() {
    let dir = (PathBuf::from("/nonexistent/session-end-a"), 1usize);
    let seen = Arc::new(AtomicUsize::new(0));
    let s = seen.clone();
    set_in(
        &dir,
        Arc::new(move |e| {
            s.fetch_add(if e == SessionEnding::Lock { 1 } else { 10 }, Ordering::SeqCst);
        }),
    );
    run_in(&dir, SessionEnding::Lock);
    run_in(&dir, SessionEnding::SignOut);
    run_in(&(PathBuf::from("/nonexistent/other"), 2), SessionEnding::Lock);
    assert_eq!(seen.load(Ordering::SeqCst), 11);
    set_in(&dir, Arc::new(|_| panic!("boom")));
    run_in(&dir, SessionEnding::Lock);
}

fn engine_with_a_session(account: &str) -> Arc<Engine<MemoryCredentials>> {
    let (host, _messages) = ChannelHost::new(|| {});
    let config = super::super::EngineConfig {
        account_id: account.into(),
        state_dir: crate::state_paths::init_for_test(),
        api_base: "http://127.0.0.1:9".into(),
    };
    let engine = Arc::new(Engine::open(&TEST, config, host, MemoryCredentials::default()).expect("the handle opens"));
    {
        let _writes = engine.writes.write().unwrap();
        engine
            .acct
            .advance_session_generation_holding_session_write_lock(SessionTransition::Write);
        *engine.acct.session.lock().unwrap() = Some(Session {
            token: "an-invented-token".into(),
            master_key: [7; 32],
            email: None,
        });
    }
    engine
}

/// A package the recovery store's kind check accepts.
fn package() -> Zeroizing<Vec<u8>> {
    let mut v = b"the last words".to_vec();
    v.extend_from_slice(b"word/document.xml");
    Zeroizing::new(v)
}

fn files_under(dir: &Path) -> usize {
    std::fs::read_dir(dir).map(|rd| rd.count()).unwrap_or(0)
}

/// A vault lock calls the hook while the session is still in memory (the put works), keeps the blob, and ends the
/// scratch key and folder right after.
#[test]
fn a_lock_calls_the_hook_with_the_session_still_in_memory_and_ends_the_scratch() {
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|p| p.into_inner());
    let engine = engine_with_a_session("an-invented-slot-lock");
    let session: SessionGeneration = engine.acct.session_generation();
    let scratch = engine.scratch_for(session).expect("a scratch store");
    let spilled = scratch.put(&vec![9u8; 4096]).expect("spilled");
    let state_dir = engine.config.state_dir.clone();
    let sealed_in_the_hook = Arc::new(AtomicUsize::new(0));
    let (probe, seen) = (engine.clone(), sealed_in_the_hook.clone());
    engine.set_office_session_end_hook(Arc::new(move |ending| {
        assert_eq!(ending, SessionEnding::Lock);
        // The spilled copy still opens, and the blob goes in: the session is still in memory.
        assert!(spilled.read().is_ok(), "scratch not ended yet");
        if probe.put_office_snapshot_for(session, FILE, 3, &package()).is_ok() {
            seen.fetch_add(1, Ordering::SeqCst);
        }
    }));
    engine.lock_vault();
    assert_eq!(
        sealed_in_the_hook.load(Ordering::SeqCst),
        1,
        "the snapshot was sealed before the key left"
    );
    assert!(
        engine.office_snapshots_for(session).is_err(),
        "locked: the key is gone, nothing opens"
    );
    assert!(
        files_under(&state_dir.join("office-recovery")) > 0,
        "the sealed blob stays through a lock, for the offer after unlock"
    );
    assert!(scratch.put(b"x").is_err(), "the scratch key ended with the lock");
    assert_eq!(files_under(&state_dir.join("office-scratch")), 0);
}

/// A sign-out calls the hook first, and then no recovery blob and no scratch file is left.
#[test]
fn a_sign_out_leaves_no_recovery_blob_and_no_scratch_file() {
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|p| p.into_inner());
    let engine = engine_with_a_session("an-invented-slot-out");
    let session = engine.acct.session_generation();
    let state_dir = engine.config.state_dir.clone();
    engine
        .put_office_snapshot_for(session, FILE, 3, &package())
        .expect("kept");
    let blobs_before = files_under(&state_dir.join("office-recovery"));
    assert!(blobs_before > 0);
    let scratch = engine.scratch_for(session).expect("a scratch store");
    let _spilled = scratch.put(&vec![9u8; 4096]).expect("spilled");
    let called = Arc::new(AtomicUsize::new(0));
    let c = called.clone();
    engine.set_office_session_end_hook(Arc::new(move |ending| {
        assert_eq!(ending, SessionEnding::SignOut);
        c.fetch_add(1, Ordering::SeqCst);
    }));
    engine.sign_out_keeping_local_data().expect("signed out");
    assert_eq!(called.load(Ordering::SeqCst), 1);
    assert!(
        files_under(&state_dir.join("office-recovery")) < blobs_before,
        "the account's folder of blobs is gone"
    );
    assert!(engine.office_snapshots_for(session).is_err());
    assert_eq!(files_under(&state_dir.join("office-scratch")), 0, "no scratch file");
}

/// A real handle signed in to the engine's fake account and unlocked (a screen lock needs a password to open it).
fn signed_in_engine(tag: &str) -> (Engine<MemoryCredentials>, crate::test_api::FakeApi) {
    use crate::test_api::FakeApi;
    use crate::test_api::fake_account::{FakeAccount, TEST_PHRASE, account_handler};
    let (account, phrase) = FakeAccount::register(&format!("{tag}@example.eu"), "a-test-password-1", TEST_PHRASE);
    let account = Arc::new(account);
    let api = FakeApi::start(account_handler(account.clone()));
    let (host, _messages) = ChannelHost::new(|| {});
    let config = super::super::EngineConfig {
        account_id: format!("{tag}-slot"),
        state_dir: crate::state_paths::init_for_test(),
        api_base: api.base.clone(),
    };
    let engine = Engine::open(&TEST, config, host, MemoryCredentials::default()).expect("the engine opens");
    engine.restore();
    engine
        .sign_in_password(&account.email, Zeroizing::new("a-test-password-1".into()))
        .expect("the fake accepts the password");
    engine
        .unlock_with_phrase(Zeroizing::new(phrase))
        .expect("the phrase unlocks");
    (engine, api)
}

/// A screen lock: the hook is called after the lock is in place (nothing is open to the person), and the put of the
/// session that was open still works for that moment (the grant); the blob is kept; a vault lock right after ends it.
#[test]
fn a_screen_lock_calls_the_hook_after_the_lock_and_the_open_session_may_still_seal() {
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|p| p.into_inner());
    let (engine, _api) = signed_in_engine("screen-flush");
    let engine = Arc::new(engine);
    let session: SessionGeneration = engine.acct.session_generation();
    let state_dir = engine.config.state_dir.clone();
    assert!(
        engine.put_office_snapshot_for(session, FILE, 3, &package()).is_ok(),
        "open: the put works"
    );
    let _ = engine.discard_office_snapshot_for(session, FILE);

    let locked_when_called = Arc::new(AtomicUsize::new(0));
    let sealed = Arc::new(AtomicUsize::new(0));
    let (probe, seen_locked, seen_sealed) = (engine.clone(), locked_when_called.clone(), sealed.clone());
    engine.set_office_session_end_hook(Arc::new(move |ending| {
        assert_eq!(ending, SessionEnding::ScreenLock);
        if probe.lock_state() == LockState::ScreenLocked {
            seen_locked.fetch_add(1, Ordering::SeqCst);
        }
        if probe.put_office_snapshot_for(session, FILE, 3, &package()).is_ok() {
            seen_sealed.fetch_add(1, Ordering::SeqCst);
        }
    }));
    engine.lock_screen().expect("the screen locks");
    assert_eq!(
        locked_when_called.load(Ordering::SeqCst),
        1,
        "the lock was already in place"
    );
    assert_eq!(
        sealed.load(Ordering::SeqCst),
        1,
        "the open session sealed through the grant"
    );
    assert!(
        files_under(&state_dir.join("office-recovery")) > 0,
        "the blob is on disk"
    );
    // The grant is for that one session only: another turn's put, and a put after the vault closed, are refused.
    assert!(
        engine.office_snapshots_for(session).is_err(),
        "a locked screen refuses everything else"
    );
    engine.set_office_session_end_hook(Arc::new(|_| {}));
    engine.lock_vault();
    assert!(
        engine.put_office_snapshot_for(session, FILE, 4, &package()).is_err(),
        "no put after the vault locked"
    );
}

/// Two engines on one state dir keep a hook each: a lock of one never runs (or replaces) the other's.
#[test]
fn two_engines_on_one_state_dir_keep_their_own_hook() {
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|p| p.into_inner());
    let (a, b) = (
        engine_with_a_session("an-invented-slot-a"),
        engine_with_a_session("an-invented-slot-b"),
    );
    let (hit_a, hit_b) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
    let (x, y) = (hit_a.clone(), hit_b.clone());
    a.set_office_session_end_hook(Arc::new(move |_| {
        x.fetch_add(1, Ordering::SeqCst);
    }));
    b.set_office_session_end_hook(Arc::new(move |_| {
        y.fetch_add(1, Ordering::SeqCst);
    }));
    a.office_before_session_end(SessionEnding::Lock);
    assert_eq!((hit_a.load(Ordering::SeqCst), hit_b.load(Ordering::SeqCst)), (1, 0));
    drop(b);
    a.office_before_session_end(SessionEnding::Lock);
    assert_eq!(hit_a.load(Ordering::SeqCst), 2);
}

/// A sign-out with an open editor that holds unsaved edits is refused: nothing is ended, wiped or signed out, the
/// hook is not called. Only the explicit discard signs out (and then the hook runs and the blobs go).
#[test]
fn a_sign_out_with_unsaved_edits_is_refused_and_only_a_discard_signs_out() {
    use super::super::account_actions_api::ActionError;
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|p| p.into_inner());
    let engine = engine_with_a_session("an-invented-slot-unsaved");
    let session = engine.acct.session_generation();
    let state_dir = engine.config.state_dir.clone();
    engine
        .put_office_snapshot_for(session, FILE, 3, &package())
        .expect("kept");
    let called = Arc::new(AtomicUsize::new(0));
    let c = called.clone();
    engine.set_office_session_end_hook(Arc::new(move |_| {
        c.fetch_add(1, Ordering::SeqCst);
    }));
    let unsaved = Arc::new(AtomicUsize::new(1));
    let u = unsaved.clone();
    engine.set_office_unsaved_probe(Arc::new(move || u.load(Ordering::SeqCst)));

    assert_eq!(engine.sign_out_for(session), Err(ActionError::UnsavedEdits));
    assert_eq!(called.load(Ordering::SeqCst), 0, "no editor was ended");
    assert!(engine.has_session(), "still signed in");
    assert!(
        files_under(&state_dir.join("office-recovery")) > 0,
        "the recovery blob stays"
    );

    assert_eq!(engine.sign_out_discarding_unsaved_for(session), Ok(()));
    assert_eq!(called.load(Ordering::SeqCst), 1);
    assert!(!engine.has_session());
}

/// Nothing unsaved (every editor saved, or none open): the sign-out goes through as before.
#[test]
fn a_sign_out_with_nothing_unsaved_goes_through() {
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|p| p.into_inner());
    let engine = engine_with_a_session("an-invented-slot-saved");
    let session = engine.acct.session_generation();
    engine.set_office_unsaved_probe(Arc::new(|| 0));
    assert_eq!(engine.sign_out_for(session), Ok(()));
}
