//! A session the server ended, closed by the session watcher, leaves none of the account's recent searches on this
//! computer, as a sign-out does (CP2 row 2.1, privacy). The refusal run (three refused calls in a row) marks the session
//! ended; from then on every door of the account-bound handle refuses as `SessionEnded`, and the watcher's close is the
//! one step that may still wipe the ended session's OWN recent searches through it. Another account's row in the same
//! state database stays, and nothing is written for the ended session.
//!
//! Mutant: the wipe removed from `close_session_the_server_refused` (or made through the ordinary handle, which refuses
//! the ended session): the row stays and this test fails.

mod common;

#[cfg(not(target_os = "macos"))]
#[test]
fn a_session_the_watcher_closes_leaves_no_recent_searches() {
    use std::time::Duration;

    use beebeeb_engine::app::LocalStateRefused;
    use beebeeb_engine::app::lock_api::LockState;
    use beebeeb_engine::state_db::StateDb;

    /// Another account's user id, with a row of its own in the same state database (invented).
    const OTHER: &str = "99999999-9999-4999-8999-999999999999";

    let s = common::setup();
    s.sign_in();
    s.unlock();
    s.state_db_with_one_row(&s.this_account());
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

    // The control: the account's searches are kept, through the engine.
    let turn = s.engine.session_turn();
    s.engine.remember_search_for(turn, "contract").unwrap();
    s.engine.remember_search_for(turn, "invoice 2026").unwrap();
    StateDb::open(s.state_db())
        .unwrap()
        .remember_search(OTHER, "holiday", 10)
        .unwrap();
    assert_eq!(
        on_disk(&user_id),
        (true, words(&["invoice 2026", "contract"])),
        "the control: kept"
    );
    assert_eq!(
        on_disk(OTHER),
        (true, words(&["holiday"])),
        "the control: the other account's row"
    );

    // The refusal run: the session is held as ended, and every ordinary door refuses it (nothing else is allowed).
    assert!(
        !s.engine.account_session_refused(),
        "precondition: the session is accepted"
    );
    for _ in 0..3 {
        s.engine.note_call_for_test(true);
    }
    assert!(s.engine.account_session_refused(), "precondition: the session ended");
    assert_eq!(s.engine.local_state_refusal(), Some(LocalStateRefused::SessionEnded));
    assert_eq!(
        s.engine.remember_search_for(turn, "after the end"),
        Err(LocalStateRefused::SessionEnded),
        "an ended session writes no search"
    );
    assert_eq!(
        s.engine.recent_searches_for(turn),
        Err(LocalStateRefused::SessionEnded),
        "an ended session reads no search"
    );
    assert_eq!(
        on_disk(&user_id),
        (true, words(&["invoice 2026", "contract"])),
        "the refused doors changed nothing"
    );

    // The session watcher closes the vault, as in the running app.
    s.engine.watch_session_every(Duration::from_millis(50));
    let closed = common::wait_until(20, || s.engine.lock_state() == LockState::VaultClosed);
    assert!(closed, "the watcher closed the vault: {:?}", s.engine.lock_state());

    assert_eq!(
        on_disk(&user_id),
        (false, Vec::new()),
        "the session the watcher closed left no recent search"
    );
    assert_eq!(
        on_disk(OTHER),
        (true, words(&["holiday"])),
        "another account's row is not touched"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn the_test_build_is_refused_on_macos() {
    common::assert_the_test_build_is_refused();
}
