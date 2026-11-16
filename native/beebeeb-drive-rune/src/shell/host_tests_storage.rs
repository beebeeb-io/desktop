//! The storage figure follows the account (`shell::storage_view`): a sign-out clears it, another account never
//! shows the earlier account's figure (not even while its own is being read), and a lock of the same account keeps it.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, Instant};

use super::*;
use crate::source::StorageSummary;

const FIRST_USED: i64 = 111_000_000;
const OTHER_USED: i64 = 999_000_000;

struct App {
    engine: Arc<FixtureEngine>,
    rt: Runtime<State>,
    /// While true, a read waits (the server has not answered yet).
    hold: Arc<AtomicBool>,
    reads: Arc<AtomicU32>,
}

/// A live-style window on the fixture engine: the first account, signed in, its figure on screen. The reader answers
/// per account, as the server does, and can be held.
fn app() -> App {
    let engine = Arc::new(FixtureEngine::ready());
    let mut state = state::init_with(Shell::fixture_on(engine.clone(), Arc::new(FakePlatform::default())));
    let (hold, reads) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicU32::new(0)));
    let (asks, held, counted) = (engine.clone(), hold.clone(), reads.clone());
    state.storage_reader = Some(Arc::new(move || {
        counted.fetch_add(1, Ordering::SeqCst);
        let account = crate::shell::actions::EngineActions::session(asks.as_ref()).account;
        let until = Instant::now() + Duration::from_secs(10);
        while held.load(Ordering::SeqCst) && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(2));
        }
        Some(StorageSummary {
            used_bytes: if account.as_deref() == Some("fixture-account") {
                FIRST_USED
            } else {
                OTHER_USED
            },
            quota_bytes: 1_000_000_000_000,
        })
    }));
    state.storage = StorageSummary {
        used_bytes: FIRST_USED,
        quota_bytes: 1_000_000_000_000,
    };
    state.storage_known = true;
    let mut rt = runtime(state);
    settle(&mut rt);
    settle(&mut rt);
    App {
        engine,
        rt,
        hold,
        reads,
    }
}

/// Polls until no read is in flight (a worker answers within a few polls).
fn answered(rt: &mut Runtime<State>) {
    let until = Instant::now() + Duration::from_secs(10);
    while rt.state.storage_pending.is_some() && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(5));
        settle(rt);
    }
    assert!(rt.state.storage_pending.is_none(), "a read never answered");
}

#[test]
fn a_sign_out_clears_the_figure() {
    let mut app = app();
    assert_eq!(
        (app.rt.state.storage.used_bytes, app.rt.state.storage_known),
        (FIRST_USED, true)
    );
    app.engine.set_gate(GateState::SignedOut);
    settle(&mut app.rt);
    assert!(!app.rt.state.storage_known, "the figure is unknown after a sign-out");
    assert_eq!(app.rt.state.storage.used_bytes, 0);
    assert_eq!(app.rt.state.storage_split, None);
    assert_eq!(app.reads.load(Ordering::SeqCst), 0, "nothing is read for nobody");
}

#[test]
fn another_account_never_shows_the_earlier_figure_and_then_its_own() {
    let mut app = app();
    app.hold.store(true, Ordering::SeqCst);
    app.engine.set_gate(GateState::SignedOut);
    settle(&mut app.rt);
    app.engine.set_account("user-b");
    app.engine.set_gate(GateState::Ready);
    settle(&mut app.rt);
    // The read is in flight: in between, the first account's figure is nowhere.
    assert!(
        app.rt.state.storage_pending.is_some(),
        "a read for the new account was asked"
    );
    assert!(!app.rt.state.storage_known, "no figure while it is read");
    assert_ne!(
        app.rt.state.storage.used_bytes, FIRST_USED,
        "the first account's figure is gone"
    );
    app.hold.store(false, Ordering::SeqCst);
    answered(&mut app.rt);
    assert_eq!(
        (app.rt.state.storage.used_bytes, app.rt.state.storage_known),
        (OTHER_USED, true)
    );
}

#[test]
fn another_account_under_an_open_gate_is_cleared_and_read_again() {
    let mut app = app();
    app.hold.store(true, Ordering::SeqCst);
    app.engine.set_account("user-b");
    settle(&mut app.rt);
    assert!(!app.rt.state.storage_known);
    assert_ne!(app.rt.state.storage.used_bytes, FIRST_USED);
    app.hold.store(false, Ordering::SeqCst);
    answered(&mut app.rt);
    assert_eq!(app.rt.state.storage.used_bytes, OTHER_USED);
}

#[test]
fn a_lock_of_the_same_account_keeps_the_figure() {
    let mut app = app();
    app.engine.set_gate(GateState::Locked);
    settle(&mut app.rt);
    assert_eq!(
        (app.rt.state.storage.used_bytes, app.rt.state.storage_known),
        (FIRST_USED, true)
    );
    app.engine.set_gate(GateState::Ready);
    settle(&mut app.rt);
    settle(&mut app.rt);
    assert_eq!(
        (app.rt.state.storage.used_bytes, app.rt.state.storage_known),
        (FIRST_USED, true)
    );
    assert_eq!(
        app.reads.load(Ordering::SeqCst),
        0,
        "the same account is not asked again"
    );
}

#[test]
fn an_answer_that_arrives_after_a_sign_out_is_not_shown() {
    let mut app = app();
    // Another account signs in and its read is held; its owner signs out before the server answers.
    app.hold.store(true, Ordering::SeqCst);
    app.engine.set_account("user-b");
    settle(&mut app.rt);
    assert!(app.rt.state.storage_pending.is_some());
    app.engine.set_gate(GateState::SignedOut);
    settle(&mut app.rt);
    app.hold.store(false, Ordering::SeqCst);
    std::thread::sleep(Duration::from_millis(50));
    settle(&mut app.rt);
    assert!(
        !app.rt.state.storage_known,
        "a late answer for a signed-out account is dropped"
    );
    assert_eq!(app.rt.state.storage.used_bytes, 0);
}

/// The stamp is its own guard: an answer stamped with another account than the one signed in is never taken, even
/// if nothing cleared the read (`session_moved` normally drops it first).
#[test]
fn an_answer_stamped_with_another_account_is_not_taken() {
    let mut app = app();
    crate::shell::storage_view::clear(&mut app.rt.state);
    let (send, answer) = std::sync::mpsc::channel();
    send.send(Some(StorageSummary {
        used_bytes: OTHER_USED,
        quota_bytes: 1,
    }))
    .unwrap();
    app.rt.state.storage_pending = Some(crate::shell::storage_view::Pending::stamped("user-b", answer));
    assert!(!crate::shell::storage_view::poll(&mut app.rt.state));
    assert!(!app.rt.state.storage_known);
    assert!(app.rt.state.storage_pending.is_none());
}
