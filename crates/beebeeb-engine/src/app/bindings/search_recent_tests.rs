//! The recent searches of Search (slice A8) against a real state database of each test's own
//! (`Engine::use_local_state_for_test`), recorded as an invented account's: kept per account and per session, newest
//! first, once, at most `SEARCHES_KEPT`; Clear deletes the row; the sign-out deletes the row of the account that signs
//! out, and a second account reads none; the account purge wipes them; nothing is kept
//! where nothing has synced. Every search and every account is invented.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::*;
use crate::account::SessionTransition;
use crate::account_binding::Identity;
use crate::app::EngineConfig;
use crate::host::ChannelHost;
use crate::identity::TEST;
use crate::keychain::{AuthSecretStore, SessionToken};
use crate::session::Session;
use crate::signin::gate::GateState;
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
    Arc::new(Engine::open(&TEST, config, host, MemoryCredentials::default()).expect("the handle opens"))
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
/// second question confirmed, `confirmed_at`). The close after the refusal run (the session watcher's
/// `close_ended_session`) is `tests/a_session_the_watcher_closes_leaves_no_recent_searches.rs`; a sign-out pressed
/// before the watcher fires is `tests/a_sign_out_before_the_watcher_fires_leaves_no_recent_searches.rs`.
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

/// Batch 3 row S7 (mutant S7-M2: no delete in `sign_out_holding_step`, where it is made past the stop gate): the
/// sign-out that keeps the local files and the local state deletes the account's recent searches all the same.
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

/// A sign-out refused because an engine task may still run (its stop was never confirmed: the account's record)
/// deletes nothing: the recent searches stay. The delete is made past the stop gate (`sign_out_holding_step`).
/// Mutant U-S1: the delete made before the gate again, in `sign_out_keeping_local_data_step`: the row is gone: red.
#[test]
fn a_refused_sign_out_keeps_the_recent_searches() {
    let engine = engine();
    let dir = tempfile::tempdir().unwrap();
    let path = state(dir.path(), A);
    engine.use_local_state_for_test(path.clone());
    let a = signed_in(&engine, A);
    engine.remember_search_for(a, "contract").unwrap();
    engine
        .acct
        .engine_stop_unconfirmed
        .store(true, std::sync::atomic::Ordering::SeqCst);

    assert_eq!(
        engine.sign_out_keeping_local_data(),
        Err(crate::account_binding::ENGINE_STOP_UNCONFIRMED.to_string())
    );
    assert_eq!(on_disk(&path, A.0), (true, words(&["contract"])), "nothing was deleted");
}

/// The same through the account page's sign-out (`sign_out_for`). Mutant U-S2: the delete made before the gate again,
/// in `account_actions_api::sign_out_for`: the row is gone: red.
#[test]
fn a_refused_sign_out_from_the_account_page_keeps_the_recent_searches() {
    let engine = engine();
    let dir = tempfile::tempdir().unwrap();
    let path = state(dir.path(), A);
    engine.use_local_state_for_test(path.clone());
    let a = signed_in(&engine, A);
    engine.remember_search_for(a, "contract").unwrap();
    engine
        .acct
        .engine_stop_unconfirmed
        .store(true, std::sync::atomic::Ordering::SeqCst);

    assert_eq!(
        engine.sign_out_for(a),
        Err(crate::app::account_actions_api::ActionError::Failed)
    );
    assert_eq!(on_disk(&path, A.0), (true, words(&["contract"])), "nothing was deleted");
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

// ── a session end's delete that fails is logged, and made at the next open ──────────────────

/// The log line of a session end whose delete of the recent searches failed.
const NOT_DELETED: &str = "search query row not deleted at session end";
/// The log line of that delete, made at the next open.
const DELETED_AT_OPEN: &str = "pending search query row deleted at open";
/// The log line of a start with nobody signed in that deleted recent searches.
const DELETED_AT_START: &str = "search query rows deleted at start";

/// Three refusals in a row: the session is held as ended (as the sync loop holds it after three 401s).
fn the_refusal_run(engine: &TestEngine) {
    let refused = anyhow::anyhow!("401 Unauthorized");
    for _ in 0..3 {
        engine.acct.auth_health.note_result(Some(&refused));
    }
    assert!(engine.acct.auth_health.is_expired(), "precondition: the session ended");
}

/// The account's next session after an ended one, open: the strong unlock clears the refusal run before its session
/// is installed (`close_ended_session`'s doc). Answers its turn.
fn next_session(engine: &TestEngine, account: (&str, &str)) -> SessionGeneration {
    engine.acct.auth_health.note_result(None);
    let turn = signed_in(engine, account);
    open_lock(engine);
    turn
}

/// The state database held by a writer in exclusive mode, as a long write of the sync engine holds it (the trick of
/// `state_db::read_only`'s tests): the handle's proof cannot read it.
fn held(path: &Path) -> rusqlite::Connection {
    let holder = rusqlite::Connection::open(path).unwrap();
    holder
        .execute_batch("PRAGMA locking_mode = EXCLUSIVE; BEGIN EXCLUSIVE; DELETE FROM files WHERE file_id = 'none';")
        .unwrap();
    holder
}

fn released(holder: rusqlite::Connection) {
    holder.execute_batch("ROLLBACK;").unwrap();
    drop(holder);
}

/// Everything `body` logs through `tracing`, at every level, on this thread (as `session_revoke_tests`).
fn capture_logs<T>(body: impl FnOnce() -> T) -> (T, String) {
    #[derive(Clone)]
    struct Capture(Arc<std::sync::Mutex<Vec<u8>>>);
    impl std::io::Write for Capture {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let buffer: Arc<std::sync::Mutex<Vec<u8>>> = Arc::default();
    let writer = Capture(buffer.clone());
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .finish();
    let guard = tracing::subscriber::set_default(subscriber);
    let out = body();
    drop(guard);
    let text = String::from_utf8_lossy(&buffer.lock().unwrap()).into_owned();
    (out, text)
}

/// The watcher's close while the sync engine holds the state database: the delete fails, and that is logged once
/// (never dropped silently). The account's next session reads none of the ended session's searches: the next open
/// deletes the row first, and a new search starts a new list. Mutants: the log line removed (the log count is 0); the
/// delete at the next open removed (the next read shows the old list).
#[test]
fn a_failed_delete_at_the_watchers_close_is_logged_and_made_at_the_next_open() {
    let engine = engine();
    let dir = tempfile::tempdir().unwrap();
    let path = state(dir.path(), A);
    engine.use_local_state_for_test(path.clone());
    let a = signed_in(&engine, A);
    engine.remember_search_for(a, "contract").unwrap();
    engine.remember_search_for(a, "invoice 2026").unwrap();
    open_lock(&engine);
    the_refusal_run(&engine);

    let holder = held(&path);
    let (closed, logs) = capture_logs(|| engine.close_ended_session());
    released(holder);
    assert!(closed.is_some(), "the premise: the watcher's close closed the vault");
    assert_eq!(
        on_disk(&path, A.0),
        (true, words(&["invoice 2026", "contract"])),
        "the premise: the delete failed while the database was held"
    );
    assert_eq!(
        logs.matches(NOT_DELETED).count(),
        1,
        "the failed delete is logged once: {logs}"
    );

    let a2 = next_session(&engine, A);
    let (read, logs) = capture_logs(|| engine.recent_searches_for(a2));
    assert_eq!(
        read,
        Ok(Vec::new()),
        "the next read shows none of the ended session's searches"
    );
    assert_eq!(
        logs.matches(DELETED_AT_OPEN).count(),
        1,
        "the delete at the next open: {logs}"
    );
    assert_eq!(
        on_disk(&path, A.0),
        (false, Vec::new()),
        "the next open deleted the row"
    );
    assert_eq!(
        engine.remember_search_for(a2, "new"),
        Ok(words(&["new"])),
        "a new search starts a new list"
    );
}

/// The same for a sign-out (the sign-in flow's, which keeps the local data): the failed delete is logged, and the
/// account's next WRITE is made into a new list, the old row deleted first at its open. Mutant: the delete at a
/// change handle's open removed (the new search is refused while the delete is due, or joins the old list).
#[test]
fn a_failed_delete_at_a_sign_out_is_logged_and_made_at_the_next_open() {
    let engine = engine();
    let dir = tempfile::tempdir().unwrap();
    let path = state(dir.path(), A);
    engine.use_local_state_for_test(path.clone());
    let a = signed_in(&engine, A);
    engine.remember_search_for(a, "contract").unwrap();
    engine.remember_search_for(a, "invoice 2026").unwrap();
    open_lock(&engine);
    the_refusal_run(&engine);

    let holder = held(&path);
    let (signed_out, logs) = capture_logs(|| engine.sign_out_keeping_local_data());
    released(holder);
    assert_eq!(signed_out, Ok(()), "the premise: signed out");
    assert_eq!(
        on_disk(&path, A.0),
        (true, words(&["invoice 2026", "contract"])),
        "the premise: the delete failed while the database was held"
    );
    assert_eq!(
        logs.matches(NOT_DELETED).count(),
        1,
        "the failed delete is logged once: {logs}"
    );

    let a2 = next_session(&engine, A);
    let (kept, logs) = capture_logs(|| engine.remember_search_for(a2, "new"));
    assert_eq!(kept, Ok(words(&["new"])), "the first write after it starts a new list");
    assert_eq!(
        logs.matches(DELETED_AT_OPEN).count(),
        1,
        "the delete at the next open: {logs}"
    );
    assert_eq!(on_disk(&path, A.0), (true, words(&["new"])));
}

/// The control: a session end's delete that works logs neither line, and leaves nothing due for the next open.
#[test]
fn a_delete_that_works_at_the_watchers_close_logs_nothing_and_leaves_nothing_due() {
    let engine = engine();
    let dir = tempfile::tempdir().unwrap();
    let path = state(dir.path(), A);
    engine.use_local_state_for_test(path.clone());
    let a = signed_in(&engine, A);
    engine.remember_search_for(a, "contract").unwrap();
    open_lock(&engine);
    the_refusal_run(&engine);

    let (closed, logs) = capture_logs(|| engine.close_ended_session());
    assert!(closed.is_some(), "the premise: the watcher's close closed the vault");
    assert_eq!(on_disk(&path, A.0), (false, Vec::new()), "deleted at the close");
    assert_eq!(logs.matches(NOT_DELETED).count(), 0, "{logs}");

    let a2 = next_session(&engine, A);
    let (kept, logs) = capture_logs(|| engine.remember_search_for(a2, "new"));
    assert_eq!(kept, Ok(words(&["new"])));
    assert_eq!(logs.matches(DELETED_AT_OPEN).count(), 0, "nothing was due: {logs}");
}

// ── CP2 row 2.1: an engine start with nobody signed in wipes every recent search on this PC ──────

/// An engine on `credentials` (as [`engine`]): a restart of the app, when the credentials are the ones it stored.
fn engine_with(credentials: MemoryCredentials) -> Arc<TestEngine> {
    let state_dir = crate::state_paths::init_for_test();
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "an-invented-search-slot".into(),
        state_dir,
        api_base: "http://127.0.0.1:9".into(),
    };
    Arc::new(Engine::open(&TEST, config, host, credentials).expect("the handle opens"))
}

/// The lead's case: a sign-out's delete fails (the state was held), the delete's due mark lives in memory and goes
/// with the process, and the app starts again with nobody signed in: that start owns nobody's searches and deletes
/// every row still there, another account's too. Mutant: the start's delete removed (both rows stay).
#[test]
fn a_failed_delete_then_a_signed_out_restart_leaves_no_recent_searches() {
    let engine = engine();
    let dir = tempfile::tempdir().unwrap();
    let path = state(dir.path(), A);
    engine.use_local_state_for_test(path.clone());
    let a = signed_in(&engine, A);
    engine.remember_search_for(a, "contract").unwrap();
    engine.remember_search_for(a, "invoice 2026").unwrap();
    open_lock(&engine);
    let holder = held(&path);
    let (signed_out, logs) = capture_logs(|| engine.sign_out_keeping_local_data());
    released(holder);
    assert_eq!(signed_out, Ok(()), "the premise: signed out");
    assert_eq!(
        on_disk(&path, A.0),
        (true, words(&["invoice 2026", "contract"])),
        "the premise: the sign-out's delete failed while the database was held"
    );
    assert_eq!(logs.matches(NOT_DELETED).count(), 1, "{logs}");
    StateDb::open(&path)
        .unwrap()
        .remember_search(B.0, "holiday", 10)
        .unwrap();
    drop(engine);

    // The restart: new credentials (the sign-out cleared the stored session), nobody signed in.
    let restarted = engine_with(MemoryCredentials::default());
    restarted.use_local_state_for_test(path.clone());
    let (gate, logs) = capture_logs(|| restarted.restore());

    assert_eq!(gate, GateState::SignedOut, "the premise: a start with nobody signed in");
    assert_eq!(
        on_disk(&path, A.0),
        (false, Vec::new()),
        "the signed-out start deleted the row the failed delete left"
    );
    assert_eq!(
        on_disk(&path, B.0),
        (false, Vec::new()),
        "and every other account's row"
    );
    assert_eq!(logs.matches(DELETED_AT_START).count(), 1, "{logs}");
    assert!(path.exists(), "the local state itself stays");
}

/// The control: a start under the same account (its session stored) keeps its recent searches: the same person.
#[test]
fn a_restart_under_the_same_account_keeps_the_recent_searches() {
    let dir = tempfile::tempdir().unwrap();
    let path = state(dir.path(), A);
    StateDb::open(&path)
        .unwrap()
        .remember_search(A.0, "contract", 10)
        .unwrap();
    let credentials = MemoryCredentials::default();
    let store = credentials.store_for("an-invented-search-slot");
    store
        .save_session_token(&SessionToken::new("an-invented-token").unwrap())
        .unwrap();
    store.save_session_owner(A.0).unwrap();

    let restarted = engine_with(credentials);
    restarted.use_local_state_for_test(path.clone());
    let (gate, logs) = capture_logs(|| restarted.restore());

    assert_ne!(
        gate,
        GateState::SignedOut,
        "the premise: the account's session is stored"
    );
    assert_eq!(
        on_disk(&path, A.0),
        (true, words(&["contract"])),
        "kept for the same person"
    );
    assert_eq!(logs.matches(DELETED_AT_START).count(), 0, "{logs}");
}

/// The control: a signed-out start where nothing has synced makes no state database to delete from.
#[test]
fn a_signed_out_start_where_nothing_synced_makes_no_database() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let restarted = engine();
    restarted.use_local_state_for_test(path.clone());
    assert_eq!(restarted.restore(), GateState::SignedOut, "the premise");
    assert!(!path.exists(), "no database was made");
}
