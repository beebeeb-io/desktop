//! A sign-out pressed after the refusal run marked the session ended, before the session watcher closed it, leaves
//! none of the account's recent searches on this computer (CP2 row 2.1, lead ruling 22:22, privacy). The watcher has
//! nothing to close after that sign-out, so the sign-out's own delete is the only one: it goes through the same
//! one-step exemption as the watcher's close (it removes the ended session's own row, writes nothing for another
//! account). Both sign-outs: the one the sign-in flow uses (`sign_out_keeping_local_data`) and Account's Sign out
//! (`sign_out_for`), the second held across a tick of the real watcher.
//!
//! Mutants: either sign-out's delete line removed, or its delete made through the ordinary handle (which refuses the
//! ended session): that sign-out leaves the row and this test fails.

mod common;

#[cfg(not(target_os = "macos"))]
#[test]
fn a_sign_out_before_the_watcher_fires_leaves_no_recent_searches() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};

    use beebeeb_engine::app::LocalStateRefused;
    use beebeeb_engine::app::lock_api::LockState;
    use beebeeb_engine::state_db::StateDb;

    /// Another account's user id, with a row of its own in the same state database (invented).
    const OTHER: &str = "99999999-9999-4999-8999-999999999999";

    let s = common::setup();
    let user_id = s.account.user_id.clone();
    // What is on disk, read past the engine: (a row exists, the list it holds).
    let on_disk = |user_id: &str| -> (bool, Vec<String>) {
        let db = StateDb::open(s.state_db()).unwrap();
        (
            db.has_search_recent_row(user_id).unwrap(),
            db.search_recent(user_id).unwrap(),
        )
    };
    let words = |list: &[&str]| -> Vec<String> { list.iter().map(|word| word.to_string()).collect() };
    // Signed in and unlocked, two searches kept; then the refusal run marks the session ended and the ordinary doors
    // refuse it. Answers the session's turn.
    let searches_then_the_refusal_run = || {
        s.sign_in();
        s.unlock();
        let turn = s.engine.session_turn();
        s.engine.remember_search_for(turn, "contract").unwrap();
        s.engine.remember_search_for(turn, "invoice 2026").unwrap();
        assert_eq!(
            on_disk(&user_id),
            (true, words(&["invoice 2026", "contract"])),
            "the control: kept"
        );
        assert!(
            !s.engine.account_session_refused(),
            "precondition: the session is accepted"
        );
        for _ in 0..3 {
            s.engine.note_call_for_test(true);
        }
        assert!(s.engine.account_session_refused(), "precondition: the session ended");
        assert_eq!(
            s.engine.remember_search_for(turn, "after the end"),
            Err(LocalStateRefused::SessionEnded),
            "an ended session writes no search"
        );
        turn
    };

    s.state_db_with_one_row(&s.this_account());
    StateDb::open(s.state_db())
        .unwrap()
        .remember_search(OTHER, "holiday", 10)
        .unwrap();

    // 1. The sign-in flow's sign-out (keeps the local data), before any watcher runs.
    searches_then_the_refusal_run();
    assert_eq!(s.engine.sign_out_keeping_local_data(), Ok(()));
    assert_eq!(
        on_disk(&user_id),
        (false, Vec::new()),
        "the sign-out the sign-in flow uses left no recent search"
    );
    assert_eq!(
        s.engine.close_ended_session(),
        None,
        "the watcher's tick after it finds nothing to close"
    );
    assert_eq!(
        on_disk(OTHER),
        (true, words(&["holiday"])),
        "another account's row stays"
    );

    // 2. Account's Sign out, held across a tick of the real watcher: the watcher starts while the sign-out holds the
    //    step (the session ending, not yet gone from memory), its tick asks for the step and waits, and finds nothing
    //    to close once the sign-out let go. Last: the watcher cannot be stopped.
    let turn = searches_then_the_refusal_run();
    let tick_waited = Arc::new(AtomicBool::new(false));
    {
        let (engine, tick_waited) = (s.engine.clone(), tick_waited.clone());
        s.engine.during_next_forget_for_test(Box::new(move || {
            let asked = engine.session_steps_asked();
            engine.watch_session_every(Duration::from_millis(50));
            let until = Instant::now() + Duration::from_secs(5);
            while Instant::now() < until {
                if engine.session_steps_asked() > asked {
                    tick_waited.store(true, Ordering::SeqCst);
                    return;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        }));
    }
    assert_eq!(s.engine.sign_out_for(turn), Ok(()));
    assert!(
        tick_waited.load(Ordering::SeqCst),
        "the instrument: the watcher's tick asked for the step while the sign-out held it"
    );
    // A few more ticks: still signed out, nothing came back.
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(s.engine.lock_state(), LockState::SignedOut);
    assert_eq!(
        on_disk(&user_id),
        (false, Vec::new()),
        "Account's Sign out, held across the watcher's tick, left no recent search"
    );
    assert_eq!(
        on_disk(OTHER),
        (true, words(&["holiday"])),
        "another account's row stays"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn the_test_build_is_refused_on_macos() {
    common::assert_the_test_build_is_refused();
}
