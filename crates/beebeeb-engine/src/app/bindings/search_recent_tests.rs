//! The recent searches of Search (slice A8) against a real state database of each test's own
//! (`Engine::use_local_state_for_test`), recorded as an invented account's: kept per account and per session, newest
//! first, once, at most `SEARCHES_KEPT`; Clear deletes the row; the sign-out deletes the row of the account that signs
//! out (lead ruling 2026-11-15 17:25), and a second account reads none; the account purge wipes them; nothing is kept
//! where nothing has synced. Every search and every account is invented.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::*;
use crate::account::SessionTransition;
use crate::account_binding::Identity;
use crate::app::EngineConfig;
use crate::host::ChannelHost;
use crate::identity::REAL;
use crate::session::Session;
use crate::signin::memory_store::MemoryCredentials;
use crate::state_db::StateDb;

type TestEngine = Engine<MemoryCredentials>;

const A: (&str, &str) = ("user-a-search", "a-search@example.eu");
const B: (&str, &str) = ("user-b-search", "b-search@example.eu");

fn engine() -> Arc<TestEngine> {
    let state_dir = crate::state_paths::init_for_test();
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "an-invented-search-slot".into(),
        state_dir,
        api_base: "http://127.0.0.1:9".into(),
    };
    Arc::new(Engine::open(&REAL, config, host, MemoryCredentials::default()).expect("the handle opens"))
}

/// A session in memory, as a sign-in and an unlock leave it (as `owned_state_tests::signed_in`). Answers its turn.
fn signed_in(engine: &TestEngine, (user_id, email): (&str, &str)) -> SessionGeneration {
    {
        let _writes = engine.writes.write().unwrap();
        engine
            .acct
            .advance_session_generation_holding_session_write_lock(SessionTransition::Write);
        *engine.acct.session.lock().unwrap() = Some(Session {
            token: "an-invented-token".into(),
            master_key: [7; 32],
            email: Some(email.to_string()),
        });
    }
    engine.remember_session_user_id(Some(user_id.to_string()));
    engine.set_auth_present(true);
    engine.session_turn()
}

/// As a sign-out leaves memory, with nothing deleted (`owned_state_tests::signed_out`).
fn signed_out(engine: &TestEngine) {
    engine.forget_session(SessionTransition::SignOut);
    engine.remember_session_user_id(None);
    engine.set_auth_present(false);
}

/// A state database recorded as `owner`'s, in `dir`.
fn state(dir: &Path, (user_id, email): (&str, &str)) -> PathBuf {
    let path = dir.join("state.db");
    let db = StateDb::open(&path).unwrap();
    db.set_owner(&Identity::new(Some(user_id), Some(email))).unwrap();
    path
}

/// What is on disk for `user_id`, read past the engine: (a row exists, the list it holds).
fn on_disk(path: &Path, user_id: &str) -> (bool, Vec<String>) {
    let db = StateDb::open(path).unwrap();
    (
        db.has_search_recent_row(user_id).unwrap(),
        db.search_recent(user_id).unwrap(),
    )
}

fn words(list: &[&str]) -> Vec<String> {
    list.iter().map(|word| word.to_string()).collect()
}

/// Each account reads and writes its own recent searches and no other's: another account signed in over this
/// state is refused (task 2488's handle), and an account that owns the state reads none of a row keyed by another
/// account's user id. A press made under an earlier session reads and writes nothing.
#[test]
fn recent_is_per_account() {
    let engine = engine();
    let dir = tempfile::tempdir().unwrap();
    let path = state(dir.path(), A);
    engine.use_local_state_for_test(path.clone());

    let a = signed_in(&engine, A);
    assert_eq!(engine.remember_search_for(a, "contract"), Ok(words(&["contract"])));
    assert_eq!(
        engine.remember_search_for(a, "Lisbon"),
        Ok(words(&["Lisbon", "contract"]))
    );
    assert_eq!(engine.recent_searches_for(a), Ok(words(&["Lisbon", "contract"])));

    // Another account, signed in over the first account's state (a sign-out keeps it): nothing read, nothing written.
    signed_out(&engine);
    let b = signed_in(&engine, B);
    assert_eq!(engine.recent_searches_for(b), Err(LocalStateRefused::OtherAccount));
    assert_eq!(
        engine.remember_search_for(b, "invoice"),
        Err(LocalStateRefused::OtherAccount)
    );
    assert_eq!(
        engine.clear_recent_searches_for(b),
        Err(LocalStateRefused::OtherAccount)
    );
    assert_eq!(
        on_disk(&path, A.0),
        (true, words(&["Lisbon", "contract"])),
        "the first account's are untouched"
    );
    assert_eq!(
        on_disk(&path, B.0),
        (false, Vec::new()),
        "nothing was written for the second"
    );

    // The state recorded as the second account's (as a reset for it would leave it, the first account's row kept
    // here on purpose): the second account reads none of the first account's searches, and keeps its own apart.
    StateDb::open(&path)
        .unwrap()
        .set_owner(&Identity::new(Some(B.0), Some(B.1)))
        .unwrap();
    assert_eq!(
        engine.recent_searches_for(b),
        Ok(Vec::new()),
        "a row of another user id is never read"
    );
    assert_eq!(engine.remember_search_for(b, "invoice"), Ok(words(&["invoice"])));
    assert_eq!(on_disk(&path, A.0).1, words(&["Lisbon", "contract"]));

    // A press made under the session before: refused, and nothing written.
    assert_eq!(engine.recent_searches_for(a), Err(LocalStateRefused::SessionChanged));
    assert_eq!(
        engine.remember_search_for(a, "lisbon"),
        Err(LocalStateRefused::SessionChanged)
    );
    assert_eq!(on_disk(&path, B.0).1, words(&["invoice"]));
}

/// The sign-out DELETES the recent searches of the account that signs out (the row, not only its words), and a
/// second account signed in after it reads none; the first account, back, has none either.
#[test]
fn recent_cleared_on_sign_out() {
    let engine = engine();
    let dir = tempfile::tempdir().unwrap();
    let path = state(dir.path(), A);
    engine.use_local_state_for_test(path.clone());

    let a = signed_in(&engine, A);
    engine.remember_search_for(a, "contract").unwrap();
    engine.remember_search_for(a, "invoice 2026").unwrap();
    assert_eq!(
        on_disk(&path, A.0),
        (true, words(&["invoice 2026", "contract"])),
        "the control: kept"
    );

    assert!(engine.sign_out_for(a).is_ok(), "the sign-out");
    assert_eq!(on_disk(&path, A.0), (false, Vec::new()), "the sign-out deleted the row");
    assert_eq!(engine.recent_searches_for(a), Err(LocalStateRefused::SignedOut));

    let b = signed_in(&engine, B);
    assert_eq!(
        engine.recent_searches_for(b),
        Err(LocalStateRefused::OtherAccount),
        "the second account reads none"
    );
    signed_out(&engine);

    let back = signed_in(&engine, A);
    assert_eq!(
        engine.recent_searches_for(back),
        Ok(Vec::new()),
        "nothing comes back for the first account"
    );
}

/// Clear deletes the account's row; what is searched after it is the only recent search. A Clear pressed under an
/// earlier session deletes nothing.
#[test]
fn clear_recent_clears() {
    let engine = engine();
    let dir = tempfile::tempdir().unwrap();
    let path = state(dir.path(), A);
    engine.use_local_state_for_test(path.clone());

    let a = signed_in(&engine, A);
    engine.remember_search_for(a, "contract").unwrap();
    engine.remember_search_for(a, "lisbon").unwrap();
    assert_eq!(engine.clear_recent_searches_for(a), Ok(()));
    assert_eq!(engine.recent_searches_for(a), Ok(Vec::new()));
    assert_eq!(
        on_disk(&path, A.0),
        (false, Vec::new()),
        "the row is deleted, not emptied"
    );
    assert_eq!(
        engine.clear_recent_searches_for(a),
        Ok(()),
        "a second Clear is no error"
    );
    assert_eq!(engine.remember_search_for(a, "invoice"), Ok(words(&["invoice"])));

    // The next session of the same account: a Clear pressed under the one before deletes nothing.
    signed_out(&engine);
    let again = signed_in(&engine, A);
    assert_eq!(
        engine.clear_recent_searches_for(a),
        Err(LocalStateRefused::SessionChanged)
    );
    assert_eq!(engine.recent_searches_for(again), Ok(words(&["invoice"])));
}

/// The rule of the list: newest first, the same words in any case once (the newest spelling), trimmed, an empty
/// search not kept, at most `SEARCHES_KEPT`.
#[test]
fn remember_keeps_the_newest_first_once_and_at_most_ten() {
    let engine = engine();
    let dir = tempfile::tempdir().unwrap();
    engine.use_local_state_for_test(state(dir.path(), A));
    let a = signed_in(&engine, A);

    for n in 0..12 {
        engine.remember_search_for(a, &format!("search {n}")).unwrap();
    }
    let kept = engine.recent_searches_for(a).unwrap();
    assert_eq!(kept.len(), SEARCHES_KEPT);
    assert_eq!(kept.first().map(String::as_str), Some("search 11"));
    assert_eq!(kept.last().map(String::as_str), Some("search 2"));

    let list = engine.remember_search_for(a, "  SEARCH 5 ").unwrap();
    assert_eq!(
        list.first().map(String::as_str),
        Some("SEARCH 5"),
        "trimmed, newest spelling first"
    );
    assert_eq!(
        list.iter().filter(|q| q.to_lowercase() == "search 5").count(),
        1,
        "once"
    );
    assert_eq!(list.len(), SEARCHES_KEPT);

    let list = engine.remember_search_for(a, "Ärger").unwrap();
    assert_eq!(list.first().map(String::as_str), Some("Ärger"));
    let list = engine.remember_search_for(a, "ärger").unwrap();
    assert_eq!(
        list.iter().filter(|q| q.to_lowercase() == "ärger").count(),
        1,
        "any case, not only ASCII"
    );

    assert_eq!(
        engine.remember_search_for(a, "   "),
        Ok(list.clone()),
        "an empty search is not kept"
    );
    assert_eq!(engine.recent_searches_for(a), Ok(list));
}

/// The purge of an account's rows (a reset of this computer's local data) wipes every account's recent searches.
#[test]
fn the_account_purge_wipes_recent_searches() {
    let dir = tempfile::tempdir().unwrap();
    let path = state(dir.path(), A);
    let db = StateDb::open(&path).unwrap();
    db.remember_search(A.0, "contract", SEARCHES_KEPT).unwrap();
    db.remember_search(B.0, "lisbon", SEARCHES_KEPT).unwrap();
    db.clear_account_data(false).unwrap();
    assert!(!db.has_search_recent_row(A.0).unwrap());
    assert!(!db.has_search_recent_row(B.0).unwrap());
}

/// On a computer where nothing has synced yet no database is made for a search: nothing is kept, and the refusal
/// says why.
#[test]
fn nothing_is_kept_where_nothing_has_synced() {
    let engine = engine();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    engine.use_local_state_for_test(path.clone());
    let a = signed_in(&engine, A);
    assert_eq!(
        engine.remember_search_for(a, "contract"),
        Err(LocalStateRefused::NothingSynced)
    );
    assert_eq!(engine.recent_searches_for(a), Err(LocalStateRefused::NothingSynced));
    assert!(!path.exists(), "no database was made");
}

/// The lock as a restored session with its key leaves it (core's start rule, app lock off): Open.
fn open_lock(engine: &TestEngine) {
    *engine.lock_flow.lock().unwrap() = beebeeb_api::flows::LockFlow::at_start(beebeeb_api::flows::StartFacts {
        session_stored: true,
        ask_password_at_start: Some(false),
        vault_closed: Some(false),
        app_lock: Some(false),
    });
}

/// Batch 3 row S7 (mutant S7-M1: no delete in `close_session_the_server_refused`): a session the server ended leaves
/// none of the account's recent searches on this computer, as a sign-out. Through the CONFIRMED close (a 401 that a
/// second question confirmed, `confirmed_at`). Not covered, and not deleted: the close after the refusal run (the
/// session watcher's `close_ended_session`), where the account-bound handle refuses as `SessionEnded`.
#[test]
fn a_session_the_server_ended_leaves_no_recent_searches() {
    let engine = engine();
    let dir = tempfile::tempdir().unwrap();
    let path = state(dir.path(), A);
    engine.use_local_state_for_test(path.clone());
    let a = signed_in(&engine, A);
    engine.remember_search_for(a, "contract").unwrap();
    engine.remember_search_for(a, "invoice 2026").unwrap();
    assert_eq!(
        on_disk(&path, A.0),
        (true, words(&["invoice 2026", "contract"])),
        "the control: kept"
    );
    open_lock(&engine);

    let closed = engine.close_session_the_server_refused(Some(a));

    assert!(closed.is_some(), "the premise: the server's word closed the vault");
    assert_eq!(
        on_disk(&path, A.0),
        (false, Vec::new()),
        "the session the server ended deleted the row"
    );
}

/// Batch 3 row S7 (mutant S7-M2: no delete in `sign_out_keeping_local_data_step`): the sign-out that keeps the local
/// files and the local state deletes the account's recent searches all the same.
#[test]
fn the_sign_out_that_keeps_local_data_deletes_the_recent_searches() {
    let engine = engine();
    let dir = tempfile::tempdir().unwrap();
    let path = state(dir.path(), A);
    engine.use_local_state_for_test(path.clone());
    let a = signed_in(&engine, A);
    engine.remember_search_for(a, "contract").unwrap();
    engine.remember_search_for(a, "invoice 2026").unwrap();
    assert_eq!(
        on_disk(&path, A.0),
        (true, words(&["invoice 2026", "contract"])),
        "the control: kept"
    );

    assert!(engine.sign_out_keeping_local_data().is_ok(), "the sign-out");

    assert_eq!(on_disk(&path, A.0), (false, Vec::new()), "the sign-out deleted the row");
    assert!(path.exists(), "the local state itself stays");
}

/// The control for S7: a screen lock keeps the recent searches (the same person unlocks), and so does the vault lock.
#[test]
fn a_screen_lock_keeps_the_recent_searches() {
    let engine = engine();
    let dir = tempfile::tempdir().unwrap();
    let path = state(dir.path(), A);
    engine.use_local_state_for_test(path.clone());
    let a = signed_in(&engine, A);
    engine.remember_search_for(a, "contract").unwrap();
    engine.remember_search_for(a, "invoice 2026").unwrap();
    open_lock(&engine);
    let kept = (true, words(&["invoice 2026", "contract"]));

    engine.lock_screen().expect("an open lock locks the screen");
    assert_eq!(on_disk(&path, A.0), kept, "the screen lock kept them");
    let asked = engine.session_turn();
    engine
        .unlock_screen_platform_verified(asked)
        .expect("the person at the keyboard opens the screen lock");
    let back = engine.session_turn();
    assert_eq!(
        engine.recent_searches_for(back),
        Ok(words(&["invoice 2026", "contract"])),
        "the same person reads them after the unlock"
    );

    let _ = engine.lock();
    assert_eq!(on_disk(&path, A.0), kept, "the vault lock kept them");
}
