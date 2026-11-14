//! The first-time guide's mark at the engine: whose it is, what a sign-out, another account, a reset and a session
//! change do to it. Sessions are put in memory as a sign-in leaves them; each test has a state database of its own.
//! Every name is invented.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use super::*;
use crate::account::SessionTransition;
use crate::app::EngineConfig;
use crate::host::ChannelHost;
use crate::identity::REAL;
use crate::session::Session;
use crate::signin::memory_store::MemoryCredentials;
use crate::state_db::{FileEntry, FileStatus, ItemKind};

type TestEngine = Engine<MemoryCredentials>;

const A: (Option<&str>, Option<&str>) = (Some("user-a"), Some("a@example.eu"));
const B: (Option<&str>, Option<&str>) = (Some("user-b"), Some("b@example.eu"));

fn engine() -> Arc<TestEngine> {
    let state_dir = crate::state_paths::init_for_test();
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "an-invented-slot".into(),
        state_dir,
        api_base: "http://127.0.0.1:9".into(),
    };
    Arc::new(Engine::open(&REAL, config, host, MemoryCredentials::default()).expect("the handle opens"))
}

/// A session in memory, as a sign-in and an unlock leave it. Its number.
fn signed_in(engine: &TestEngine, who: (Option<&str>, Option<&str>)) -> SessionGeneration {
    {
        let _writes = engine.writes.write().unwrap();
        engine
            .acct
            .advance_session_generation_holding_session_write_lock(SessionTransition::Write);
        *engine.acct.session.lock().unwrap() = Some(Session {
            token: "an-invented-token".into(),
            master_key: [7; 32],
            email: who.1.map(str::to_string),
        });
        engine.remember_session_user_id(who.0.map(str::to_string));
        engine.set_auth_present(true);
    }
    engine.session_turn()
}

fn signed_out(engine: &TestEngine) {
    engine.forget_session(SessionTransition::SignOut);
    engine.remember_session_user_id(None);
    engine.set_auth_present(false);
}

/// A state database recorded as `owner`'s (`None`: no record), with one file row when `rows`.
fn state(dir: &Path, owner: Option<(Option<&str>, Option<&str>)>, rows: bool) -> PathBuf {
    let path = dir.join("state.db");
    let db = StateDb::open(&path).unwrap();
    if rows {
        let entry = FileEntry {
            file_id: "f-1".into(),
            path: "/an-invented-name.txt".into(),
            status: FileStatus::CloudOnly,
            size_bytes: 1_200,
            modified_at: 1_794_000_000,
            content_hash: None,
            remote_updated_at: 1_794_000_000,
            parent_id: None,
            item_kind: ItemKind::File,
        };
        db.upsert_file_with_contract(&entry, |contract| contract.item_kind = entry.item_kind.clone())
            .unwrap();
    }
    if let Some((user_id, email)) = owner {
        db.set_owner(&Identity::new(user_id, email)).unwrap();
    }
    path
}

/// Every guide row in the database, by key.
fn guide_rows(path: &Path) -> Vec<String> {
    let conn = rusqlite::Connection::open(path).unwrap();
    let mut rows = conn
        .prepare("SELECT key FROM sync_state WHERE key LIKE 'guide_done:%' ORDER BY key")
        .unwrap();
    rows.query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

#[test]
fn the_mark_is_the_accounts_own_and_a_sign_out_and_sign_in_keep_it() {
    let engine = engine();
    let dir = tempfile::tempdir().unwrap();
    let path = state(dir.path(), Some(A), true);
    engine.use_local_state_for_test(path.clone());

    let session = signed_in(&engine, A);
    assert_eq!(engine.guide_mark_for(session), GuideMark::NotDone);
    engine
        .mark_guide_done_for(session)
        .expect("the account's own state takes it");
    assert_eq!(engine.guide_mark_for(session), GuideMark::Done);
    assert_eq!(
        guide_rows(&path),
        ["guide_done:user-a"],
        "keyed by the user id, never the email"
    );
    // Again: nothing changes.
    engine.mark_guide_done_for(session).expect("idempotent");
    assert_eq!(guide_rows(&path).len(), 1);

    // A sign-out that keeps local data (the native app's), and the same account again.
    signed_out(&engine);
    let again = signed_in(&engine, A);
    assert_eq!(engine.guide_mark_for(again), GuideMark::Done, "done once per account");

    // Another account on the same computer: the state is not its own, so nothing is read and nothing is written.
    signed_out(&engine);
    let other = signed_in(&engine, B);
    assert_eq!(
        engine.guide_mark_for(other),
        GuideMark::NotKnown(LocalStateRefused::OtherAccount)
    );
    assert_eq!(engine.mark_guide_done_for(other), Err(LocalStateRefused::OtherAccount));
    assert_eq!(
        guide_rows(&path),
        ["guide_done:user-a"],
        "the first account's row alone"
    );
}

/// A sign-out that clears the account's rows leaves a state with no owner: the handle reads nothing there, and the
/// mark is still the account's when it signs in again, before anything has synced.
#[test]
fn a_sign_out_that_clears_the_accounts_rows_keeps_the_mark_and_no_other_account_reads_it() {
    let engine = engine();
    let dir = tempfile::tempdir().unwrap();
    let path = state(dir.path(), Some(A), true);
    engine.use_local_state_for_test(path.clone());
    let session = signed_in(&engine, A);
    engine.mark_guide_done_for(session).expect("marked");
    signed_out(&engine);
    {
        let db = StateDb::open(&path).unwrap();
        db.clear_account_data(false).unwrap();
        assert_eq!(
            db.owner().unwrap().filter(Identity::is_known),
            None,
            "no owner any more"
        );
        assert!(
            !db.has_account_data().unwrap(),
            "the kept mark is not account data: the next sign-in binds as on a new computer"
        );
    }
    assert_eq!(guide_rows(&path), ["guide_done:user-a"], "the sign-out kept the mark");

    let again = signed_in(&engine, A);
    assert_eq!(
        engine.local_state_refusal(),
        Some(LocalStateRefused::NothingSynced),
        "the handle hands out nothing of an unowned state"
    );
    assert_eq!(
        engine.guide_mark_for(again),
        GuideMark::Done,
        "and the mark is still the account's"
    );
    signed_out(&engine);

    // Another account on that cleared state: its own row is not there, and the first account's is not read as its.
    let other = signed_in(&engine, B);
    assert_eq!(
        engine.guide_mark_for(other),
        GuideMark::NotDone,
        "it gets the guide once"
    );
    assert_eq!(
        engine.mark_guide_done_for(other),
        Err(LocalStateRefused::NothingSynced),
        "nothing is written into a state before it is proven the session's"
    );
    assert_eq!(guide_rows(&path), ["guide_done:user-a"]);
    signed_out(&engine);

    // Once the state is the second account's, its mark is kept beside the first one's.
    StateDb::open(&path)
        .unwrap()
        .set_owner(&Identity::new(B.0, B.1))
        .unwrap();
    let other = signed_in(&engine, B);
    engine.mark_guide_done_for(other).expect("its own state takes it");
    assert_eq!(engine.guide_mark_for(other), GuideMark::Done);
    assert_eq!(guide_rows(&path), ["guide_done:user-a", "guide_done:user-b"]);
    signed_out(&engine);

    // A reset of local data removes every account's mark.
    StateDb::open(&path).unwrap().forget_share_trust_rows().unwrap();
    assert!(guide_rows(&path).is_empty());
    let other = signed_in(&engine, B);
    assert_eq!(engine.guide_mark_for(other), GuideMark::NotDone);
}

#[test]
fn nothing_is_read_or_written_where_it_cannot_be_the_accounts() {
    // No database on this computer: not done, and none is created for the mark.
    let engine = engine();
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("state.db");
    engine.use_local_state_for_test(missing.clone());
    let session = signed_in(&engine, A);
    assert_eq!(engine.guide_mark_for(session), GuideMark::NotDone);
    assert_eq!(
        engine.mark_guide_done_for(session),
        Err(LocalStateRefused::NothingSynced)
    );
    assert!(!missing.exists(), "no database is created for the mark");
    signed_out(&engine);

    // Data with no record of whose it is: not known.
    let unowned = tempfile::tempdir().unwrap();
    let path = state(unowned.path(), None, true);
    engine.use_local_state_for_test(path.clone());
    let session = signed_in(&engine, A);
    assert_eq!(
        engine.guide_mark_for(session),
        GuideMark::NotKnown(LocalStateRefused::Unowned)
    );
    assert_eq!(engine.mark_guide_done_for(session), Err(LocalStateRefused::Unowned));
    signed_out(&engine);

    // A session with no user id: an email never stands in for the key.
    let owned = tempfile::tempdir().unwrap();
    let path = state(owned.path(), Some((None, A.1)), true);
    engine.use_local_state_for_test(path.clone());
    let session = signed_in(&engine, (None, A.1));
    assert!(
        engine.local_state().is_ok(),
        "the state itself is handed to its email's session"
    );
    assert_eq!(
        engine.guide_mark_for(session),
        GuideMark::NotKnown(LocalStateRefused::IdentityUnknown)
    );
    assert_eq!(
        engine.mark_guide_done_for(session),
        Err(LocalStateRefused::IdentityUnknown)
    );
    assert!(guide_rows(&path).is_empty(), "no row keyed by anything else");
    signed_out(&engine);

    // Nobody signed in.
    assert_eq!(
        engine.guide_mark_for(engine.session_turn()),
        GuideMark::NotKnown(LocalStateRefused::SessionChanged)
    );
}

#[test]
fn a_mark_is_read_and_written_only_under_the_session_that_asked() {
    let engine = engine();
    let dir = tempfile::tempdir().unwrap();
    let path = state(dir.path(), Some(A), true);
    engine.use_local_state_for_test(path.clone());

    // The same account, another session (a lock and an unlock, a sign-out and a sign-in).
    let asked_under = signed_in(&engine, A);
    signed_out(&engine);
    let now = signed_in(&engine, A);
    assert_ne!(asked_under, now);
    assert_eq!(
        engine.mark_guide_done_for(asked_under),
        Err(LocalStateRefused::SessionChanged)
    );
    assert_eq!(
        engine.guide_mark_for(asked_under),
        GuideMark::NotKnown(LocalStateRefused::SessionChanged)
    );
    assert!(
        guide_rows(&path).is_empty(),
        "nothing was written under a session that moved on"
    );
    assert_eq!(
        engine.guide_mark_for(now),
        GuideMark::NotDone,
        "the control: the session there now reads"
    );

    // While a sign-out is ending the session (its number has moved, the session is still in memory), asked under
    // the number it has at that moment: only the session-write lock can refuse.
    let (during, said) = (engine.clone(), Arc::new(Mutex::new(None)));
    let heard = said.clone();
    engine.during_next_forget_for_test(Box::new(move || {
        let still = during.session_turn();
        *heard.lock().unwrap() = Some((still, during.guide_mark_for(still), during.mark_guide_done_for(still)));
    }));
    signed_out(&engine);
    let (still, read, written) = said.lock().unwrap().take().expect("the hook ran");
    assert_ne!(still, now, "the number had moved when the hook asked");
    assert_eq!(read, GuideMark::NotKnown(LocalStateRefused::SessionChanged));
    assert_eq!(written, Err(LocalStateRefused::SessionChanged));
    assert!(guide_rows(&path).is_empty(), "an ending session writes nothing");
}
