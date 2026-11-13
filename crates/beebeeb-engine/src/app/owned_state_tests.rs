//! The account-bound handle on the local state (task 2488), at the engine: who is handed the state, who is not and
//! why, and that a handle does not outlive its session. Sessions are put in memory as a sign-in leaves them; the
//! state database is one of each test's own (`Engine::use_local_state_for_test`). The sign-in, a real sync, the
//! sign-out and a second account end to end are `tests/the_local_state_follows_the_account.rs`. Every name is
//! invented.

use std::path::Path;
use std::sync::Arc;

use super::*;
use crate::account::SessionTransition;
use crate::app::EngineConfig;
use crate::drive_v2::model::Sort;
use crate::host::ChannelHost;
use crate::identity::REAL;
use crate::session::Session;
use crate::signin::memory_store::MemoryCredentials;
use crate::state_db::{FileEntry, FileStatus, ItemKind};

type TestEngine = Engine<MemoryCredentials>;

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

/// A session in memory, as a sign-in and an unlock leave it: the email, and the user id recorded for it.
fn signed_in(engine: &TestEngine, user_id: Option<&str>, email: Option<&str>) {
    let _writes = engine.writes.lock().unwrap();
    engine
        .acct
        .advance_session_generation_holding_session_write_lock(SessionTransition::Write);
    *engine.acct.session.lock().unwrap() = Some(Session {
        token: "an-invented-token".into(),
        master_key: [7; 32],
        email: email.map(str::to_string),
    });
    engine.remember_session_user_id(user_id.map(str::to_string));
    engine.set_auth_present(true);
}

/// As a sign-out leaves memory (the handle's own `forget_session`, and the fields the sign-out clears after it).
fn signed_out(engine: &TestEngine) {
    engine.forget_session(SessionTransition::SignOut);
    engine.remember_session_user_id(None);
    engine.set_auth_present(false);
}

fn row(db: &StateDb, id: &str, path: &str) {
    let entry = FileEntry {
        file_id: id.into(),
        path: path.into(),
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

/// A state database with one row, recorded as `owner`'s (`None`: no record).
fn state(dir: &Path, owner: Option<(Option<&str>, Option<&str>)>, rows: bool) -> std::path::PathBuf {
    let path = dir.join("state.db");
    let db = StateDb::open(&path).unwrap();
    if rows {
        row(&db, "f-1", "/an-invented-name.txt");
    }
    if let Some((user_id, email)) = owner {
        db.set_owner(&Identity::new(user_id, email)).unwrap();
    }
    path
}

fn refusal(engine: &TestEngine) -> Option<LocalStateRefused> {
    engine.local_state_refusal()
}

/// What every reader a shell has answers for this engine now: the handle, the handle for changes, the listing.
fn every_door(engine: &TestEngine) -> [bool; 3] {
    [
        engine.local_state().is_ok(),
        engine.local_state_for_changes().is_ok(),
        engine.listing(None, Sort::default(), 0, 10).is_ok(),
    ]
}

/// Each kind of session against each kind of local state has one answer, and every door gives the same one.
#[test]
fn whose_the_local_state_is_decides_every_read() {
    let a = (Some("user-a"), Some("a@example.eu"));
    type Who<'a> = (Option<&'a str>, Option<&'a str>);
    let cases: [(&str, Option<Who>, bool, Who, Option<LocalStateRefused>); 12] = [
        ("the owner, by id and email", Some(a), true, a, None),
        (
            "the owner after an email change: the id decides",
            Some((Some("user-a"), Some("old@example.eu"))),
            true,
            (Some("user-a"), Some("new@example.eu")),
            None,
        ),
        (
            "another account that now has the owner's old email: the id decides",
            Some(a),
            true,
            (Some("user-b"), Some("a@example.eu")),
            Some(LocalStateRefused::OtherAccount),
        ),
        (
            "another account",
            Some(a),
            true,
            (Some("user-b"), Some("b@example.eu")),
            Some(LocalStateRefused::OtherAccount),
        ),
        (
            "another account, known by its email alone",
            Some(a),
            true,
            (None, Some("b@example.eu")),
            Some(LocalStateRefused::OtherAccount),
        ),
        (
            "an owner recorded by email alone (an older record): the canonical email decides",
            Some((None, Some("a@example.eu"))),
            true,
            (Some("user-a"), Some(" A@Example.eu ")),
            None,
        ),
        (
            "an owner by id alone and a session by email alone cannot be compared",
            Some((Some("user-a"), None)),
            true,
            (None, Some("a@example.eu")),
            Some(LocalStateRefused::IdentityUnknown),
        ),
        (
            "a session that does not know who it is",
            Some(a),
            true,
            (None, None),
            Some(LocalStateRefused::IdentityUnknown),
        ),
        (
            "account data with no record of an owner",
            None,
            true,
            a,
            Some(LocalStateRefused::Unowned),
        ),
        (
            "no owner and no data: nothing has synced",
            None,
            false,
            a,
            Some(LocalStateRefused::NothingSynced),
        ),
        ("the owner's state with no rows yet", Some(a), false, a, None),
        (
            "the owner, whose stored session names the id and no email",
            Some(a),
            true,
            (Some("user-a"), None),
            None,
        ),
    ];
    let engine = engine();
    for (name, owner, rows, session, expected) in cases {
        let dir = tempfile::tempdir().unwrap();
        engine.use_local_state_for_test(state(dir.path(), owner, rows));
        signed_in(&engine, session.0, session.1);
        assert_eq!(refusal(&engine), expected, "{name}");
        assert_eq!(every_door(&engine), [expected.is_none(); 3], "{name}: every door");
        signed_out(&engine);
        assert_eq!(
            refusal(&engine),
            Some(LocalStateRefused::SignedOut),
            "{name}, signed out"
        );
        assert_eq!(every_door(&engine), [false; 3], "{name}, signed out: every door");
    }
}

/// No session, no state: signed out, locked (a token is stored, no keys in memory), nothing on disk, a file that is
/// not a state database.
#[test]
fn without_a_session_or_a_readable_state_nothing_is_handed_out() {
    let engine = engine();
    let dir = tempfile::tempdir().unwrap();
    engine.use_local_state_for_test(state(dir.path(), Some((Some("user-a"), Some("a@example.eu"))), true));
    assert_eq!(refusal(&engine), Some(LocalStateRefused::SignedOut));
    engine.set_auth_present(true);
    assert_eq!(refusal(&engine), Some(LocalStateRefused::Locked));

    signed_in(&engine, Some("user-a"), Some("a@example.eu"));
    assert_eq!(refusal(&engine), None, "the control: this session is handed its state");
    engine.use_local_state_for_test(dir.path().join("not-there.db"));
    assert_eq!(refusal(&engine), Some(LocalStateRefused::NothingSynced));
    let garbage = dir.path().join("garbage.db");
    std::fs::write(&garbage, b"this is not a database").unwrap();
    engine.use_local_state_for_test(garbage);
    assert_eq!(refusal(&engine), Some(LocalStateRefused::Unreadable));
    assert_eq!(every_door(&engine), [false; 3]);
    signed_out(&engine);
}

/// A handle answers for the session it was made for and no other: a lock, an unlock of the same account and a
/// sign-out each turn an older handle into a refusal, while a new handle of the same account reads at once.
#[test]
fn a_handle_does_not_outlive_its_session() {
    let engine = engine();
    let dir = tempfile::tempdir().unwrap();
    engine.use_local_state_for_test(state(dir.path(), Some((Some("user-a"), Some("a@example.eu"))), true));
    let rows = |state: &OwnedState<'_>| state.with(|db| db.list_files().map(|files| files.len()).unwrap_or(0));

    signed_in(&engine, Some("user-a"), Some("a@example.eu"));
    let first = engine.local_state().expect("the owner's handle");
    assert_eq!(rows(&first), Ok(1));

    // Locked: the handle is refused, and so is a new one.
    engine.forget_session(SessionTransition::Lock);
    assert_eq!(rows(&first), Err(LocalStateRefused::SessionChanged));
    assert_eq!(refusal(&engine), Some(LocalStateRefused::Locked));

    // Unlocked again, the same account: a new handle reads at once; the one from before the lock stays refused.
    signed_in(&engine, Some("user-a"), Some("a@example.eu"));
    let second = engine.local_state().expect("the same account after an unlock");
    assert_eq!(rows(&second), Ok(1));
    assert_eq!(rows(&first), Err(LocalStateRefused::SessionChanged));

    // Signed out, and another account signed in: the owner's handle answers nothing under the other session.
    signed_out(&engine);
    assert_eq!(rows(&second), Err(LocalStateRefused::SessionChanged));
    signed_in(&engine, Some("user-b"), Some("b@example.eu"));
    assert_eq!(rows(&second), Err(LocalStateRefused::SessionChanged));
    assert_eq!(rows(&first), Err(LocalStateRefused::SessionChanged));
    assert_eq!(refusal(&engine), Some(LocalStateRefused::OtherAccount));
    signed_out(&engine);
}

/// An answer that was read while the session changed is dropped, not returned.
#[test]
fn an_answer_read_across_a_session_change_is_dropped() {
    let engine = engine();
    let dir = tempfile::tempdir().unwrap();
    engine.use_local_state_for_test(state(dir.path(), Some((Some("user-a"), Some("a@example.eu"))), true));
    signed_in(&engine, Some("user-a"), Some("a@example.eu"));
    let handle = engine.local_state().expect("the owner's handle");
    let answer = handle.with(|db| {
        let names: Vec<String> = db.list_files().unwrap().into_iter().map(|file| file.path).collect();
        engine.forget_session(SessionTransition::Lock);
        names
    });
    assert_eq!(answer, Err(LocalStateRefused::SessionChanged));
    signed_out(&engine);
}

/// A session the server ended (its token refused again and again) reads nothing, through a handle it already holds
/// too; a sign-out ends that streak, so the next session is not taken for the ended one.
#[test]
fn a_session_the_server_ended_reads_nothing() {
    let engine = engine();
    let dir = tempfile::tempdir().unwrap();
    engine.use_local_state_for_test(state(dir.path(), Some((Some("user-a"), Some("a@example.eu"))), true));
    signed_in(&engine, Some("user-a"), Some("a@example.eu"));
    let handle = engine.local_state().expect("the owner's handle");
    assert_eq!(handle.with(|_| ()), Ok(()));

    for _ in 0..3 {
        let refused = anyhow::anyhow!("401 Unauthorized");
        engine.acct.auth_health.note_result(Some(&refused));
    }
    assert!(engine.acct.auth_health.is_expired(), "precondition: the session ended");
    assert_eq!(handle.with(|_| ()), Err(LocalStateRefused::SessionEnded));
    assert_eq!(refusal(&engine), Some(LocalStateRefused::SessionEnded));
    assert_eq!(every_door(&engine), [false; 3]);

    engine.sign_out_keeping_local_data().expect("the sign-out");
    assert!(
        !engine.acct.auth_health.is_expired(),
        "the streak was the ended session's"
    );
    assert_eq!(refusal(&engine), Some(LocalStateRefused::SignedOut));
}

/// Nothing is written into a state that is not the session's: the handle for changes is refused before a
/// read-write connection exists, and a request to the sync engine is not recorded.
#[test]
fn nothing_is_written_into_a_state_that_is_not_the_sessions() {
    let engine = engine();
    let dir = tempfile::tempdir().unwrap();
    let path = state(dir.path(), Some((Some("user-a"), Some("a@example.eu"))), true);
    let asked = |path: &Path| {
        StateDb::open(path)
            .unwrap()
            .peek_resnapshot_request()
            .unwrap()
            .is_some()
    };
    assert!(!asked(&path), "precondition: nothing asked yet");
    engine.use_local_state_for_test(path.clone());

    signed_in(&engine, Some("user-b"), Some("b@example.eu"));
    assert_eq!(
        engine.local_state_for_changes().err(),
        Some(LocalStateRefused::OtherAccount)
    );
    engine.request_resnapshot();
    assert!(!asked(&path), "another account's request is not written");
    signed_out(&engine);

    signed_in(&engine, Some("user-a"), Some("a@example.eu"));
    engine.request_resnapshot();
    assert!(asked(&path), "the control: the owner's request is written");
    signed_out(&engine);
}

/// The refusal's words are fixed: nothing of an account can be in them.
#[test]
fn a_refusal_is_a_fixed_word() {
    let all = [
        LocalStateRefused::SignedOut,
        LocalStateRefused::Locked,
        LocalStateRefused::SessionEnded,
        LocalStateRefused::NothingSynced,
        LocalStateRefused::Unowned,
        LocalStateRefused::OtherAccount,
        LocalStateRefused::IdentityUnknown,
        LocalStateRefused::SessionChanged,
        LocalStateRefused::Unreadable,
    ];
    let mut kinds: Vec<&str> = all.iter().map(|refused| refused.kind()).collect();
    assert!(
        kinds
            .iter()
            .all(|kind| kind.chars().all(|c| c.is_ascii_lowercase() || c == '_'))
    );
    assert_eq!(
        all.map(|refused| refused.to_string()),
        all.map(|refused| refused.kind().to_string())
    );
    kinds.sort_unstable();
    kinds.dedup();
    assert_eq!(kinds.len(), all.len(), "one word per refusal");
    assert_eq!(
        all.iter().filter(|refused| refused.not_this_accounts()).count(),
        3,
        "unowned, another account's, not comparable"
    );
}
