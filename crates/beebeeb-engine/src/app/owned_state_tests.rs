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
    engine_on(MemoryCredentials::default())
}

/// A handle on `credentials` (a clone shares the stored items: a new handle on it is a restart).
fn engine_on(credentials: MemoryCredentials) -> Arc<TestEngine> {
    let state_dir = crate::state_paths::init_for_test();
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: SLOT.into(),
        state_dir,
        api_base: "http://127.0.0.1:9".into(),
    };
    Arc::new(Engine::open(&REAL, config, host, credentials).expect("the handle opens"))
}

const SLOT: &str = "an-invented-slot";

/// A session in memory, as a sign-in and an unlock leave it: the email, and the user id recorded for it.
fn signed_in(engine: &TestEngine, user_id: Option<&str>, email: Option<&str>) {
    let _writes = engine.writes.write().unwrap();
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

/// A handle made while a lock is ending its session (the turn has moved on, the session is not yet gone from memory)
/// answers nothing once the lock is done.
#[test]
fn a_handle_made_while_its_session_is_ending_answers_nothing_after() {
    let engine = engine();
    let dir = tempfile::tempdir().unwrap();
    engine.use_local_state_for_test(state(dir.path(), Some((Some("user-a"), Some("a@example.eu"))), true));
    signed_in(&engine, Some("user-a"), Some("a@example.eu"));

    // The first half of a lock, as `forget_session` does it.
    {
        let _writes = engine.writes.write().unwrap();
        engine
            .acct
            .advance_session_generation_holding_session_write_lock(SessionTransition::Lock);
    }
    let handle = engine.local_state().expect("the session is still in memory");
    // The second half: the session leaves memory.
    *engine.acct.session.lock().unwrap() = None;
    assert_eq!(
        handle.with(|db| db.list_files().map(|files| files.len()).unwrap_or(0)),
        Err(LocalStateRefused::SessionChanged)
    );
    signed_out(&engine);
}

/// An operation of many steps stops at the step after a lock: the steps before it ran, none after it does.
#[test]
fn an_operation_of_many_steps_takes_no_step_after_a_lock() {
    let engine = engine();
    let dir = tempfile::tempdir().unwrap();
    engine.use_local_state_for_test(state(dir.path(), Some((Some("user-a"), Some("a@example.eu"))), true));
    signed_in(&engine, Some("user-a"), Some("a@example.eu"));
    let handle = engine.local_state_for_changes().expect("the owner's handle");
    let mut reached = Vec::new();
    let answer = handle.with_while(|_db, go_on| {
        crate::free_up_space::each_while(["first", "second", "third"], go_on, |file| {
            reached.push(file);
            if file == "first" {
                // Between two files: the vault is locked.
                engine.forget_session(SessionTransition::Lock);
            }
        })
    });
    assert_eq!(reached, ["first"], "a file was reached after the lock");
    assert_eq!(answer, Err(LocalStateRefused::SessionChanged));
    signed_out(&engine);

    // The control: with no lock every file is reached.
    signed_in(&engine, Some("user-a"), Some("a@example.eu"));
    let handle = engine.local_state_for_changes().expect("the owner's handle");
    let mut reached = Vec::new();
    let all = handle.with_while(|_db, go_on| {
        crate::free_up_space::each_while(["first", "second", "third"], go_on, |file| reached.push(file))
    });
    assert_eq!((all, reached.len()), (Ok(true), 3));
    signed_out(&engine);
}

/// Readers do not refuse each other: while another reader asks whether its session is still there, this one is
/// answered. Only a session change makes a reader yield.
#[test]
fn a_reader_is_not_refused_because_another_reader_is_asking() {
    let engine = engine();
    let dir = tempfile::tempdir().unwrap();
    engine.use_local_state_for_test(state(dir.path(), Some((Some("user-a"), Some("a@example.eu"))), true));
    signed_in(&engine, Some("user-a"), Some("a@example.eu"));
    let handle = engine.local_state().expect("the owner's handle");
    {
        // As another reader holds it, in the middle of its own question.
        let _another_reader = engine.writes.read().unwrap();
        assert_eq!(
            handle.with(|db| db.list_files().map(|files| files.len()).unwrap_or(0)),
            Ok(1)
        );
        assert!(engine.local_state().is_ok());
    }
    {
        // A session change under way: the reader yields.
        let _change = engine.writes.write().unwrap();
        assert_eq!(
            handle.with(|db| db.list_files().map(|files| files.len()).unwrap_or(0)),
            Err(LocalStateRefused::SessionChanged)
        );
    }
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

/// After a restart the session's identity is completed from the user id stored with the session, never decided by
/// the email alone: a stored session of ANOTHER user id with the SAME email as the state's recorded owner is
/// refused; the same user id is handed the state. The restart is a new handle on the stored items (`restore`, no
/// profile in memory); the unlock after it leaves the session's email alone in memory.
#[test]
fn after_a_restart_the_stored_user_id_decides_not_the_email() {
    use crate::keychain::{AuthSecretStore, SessionToken};
    let dir = tempfile::tempdir().unwrap();
    let path = state(dir.path(), Some((Some("user-a"), Some("same@example.eu"))), true);
    for (stored_user_id, expected) in [("user-b", Some(LocalStateRefused::OtherAccount)), ("user-a", None)] {
        let credentials = MemoryCredentials::default();
        let store = credentials.store_for(SLOT);
        store
            .save_session_token(&SessionToken::new("an-invented-token").unwrap())
            .unwrap();
        store.save_session_owner(stored_user_id).unwrap();
        let engine = engine_on(credentials);
        engine.restore();
        {
            let _writes = engine.writes.write().unwrap();
            engine
                .acct
                .advance_session_generation_holding_session_write_lock(SessionTransition::Write);
            *engine.acct.session.lock().unwrap() = Some(Session {
                token: "an-invented-token".into(),
                master_key: [7; 32],
                email: Some("same@example.eu".into()),
            });
        }
        engine.use_local_state_for_test(path.clone());
        assert_eq!(refusal(&engine), expected, "stored user id {stored_user_id}");
        let handed_out = expected.is_none();
        assert_eq!(every_door(&engine), [handed_out; 3], "stored user id {stored_user_id}");
    }
}

fn local(db: &StateDb, id: &str, path: &str, kind: ItemKind) {
    held(db, id, path, kind, FileStatus::Local, false);
}

/// A row with this status, kept on this computer or not.
fn held(db: &StateDb, id: &str, path: &str, kind: ItemKind, status: FileStatus, kept: bool) {
    let entry = FileEntry {
        file_id: id.into(),
        path: path.into(),
        status,
        size_bytes: 10,
        modified_at: 1,
        content_hash: None,
        remote_updated_at: 1,
        parent_id: None,
        item_kind: kind,
    };
    db.upsert_file_with_contract(&entry, |contract| {
        contract.item_kind = entry.item_kind.clone();
        if kept {
            contract.pin_state = crate::state_db::PinState::Pinned;
        }
    })
    .unwrap();
}

fn ids(ids: &[&str]) -> Vec<String> {
    ids.iter().map(|id| id.to_string()).collect()
}

/// A reclaim as the real one records it: these files are online only in the rows afterwards.
fn frees<'a>(db: &'a StateDb, these: &'a [&'a str]) -> impl FnOnce(Vec<(String, String, i64)>) + 'a {
    move |wanted| {
        let freed: Vec<String> = wanted
            .into_iter()
            .map(|(id, _, _)| id)
            .filter(|id| these.contains(&id.as_str()))
            .collect();
        db.mark_cloud_only_after_dehydrate(&freed, 2).unwrap();
    }
}

/// A selection names its own files alone, a folder everything in it (never a neighbour whose name starts the same),
/// and each selected id gets its own answer, read from the rows after the reclaim.
#[test]
fn a_selection_reclaims_its_own_files_and_answers_per_item() {
    use crate::free_up_space::{Reclaimed, free_selection};
    let dir = tempfile::tempdir().unwrap();
    let db = StateDb::open(&dir.path().join("state.db")).unwrap();
    local(&db, "d-a", "/A", ItemKind::Folder);
    local(&db, "f-1", "/A/one.txt", ItemKind::File);
    local(&db, "f-2", "/A/two.txt", ItemKind::File);
    local(&db, "f-3", "/AB.txt", ItemKind::File);
    local(&db, "f-4", "/other.txt", ItemKind::File);
    let candidates = || -> Vec<(String, String, i64)> {
        [
            ("d-a", "/A"),
            ("f-1", "/A/one.txt"),
            ("f-2", "/A/two.txt"),
            ("f-3", "/AB.txt"),
            ("f-4", "/other.txt"),
        ]
        .iter()
        .map(|(id, path)| (id.to_string(), path.to_string(), 10))
        .collect()
    };
    let mut handed = Vec::new();
    // The folder, one file outside it and an id no row has; the reclaim frees one of the folder's two files.
    let answer = free_selection(&db, &ids(&["d-a", "f-4", "f-gone"]), candidates(), |wanted| {
        handed = wanted.iter().map(|(id, _, _)| id.clone()).collect();
        frees(&db, &["f-1", "f-4"])(wanted);
    });
    assert_eq!(
        handed,
        ["f-1", "f-2", "f-4"],
        "the folder's row, a neighbour or an unselected file was handed over"
    );
    assert_eq!(
        answer,
        [
            ("d-a".to_string(), Reclaimed::Left),
            ("f-4".to_string(), Reclaimed::Freed),
            ("f-gone".to_string(), Reclaimed::NoSuchItem)
        ]
    );
    // The neighbour whose name starts the same was not touched.
    assert_eq!(db.get_file("f-3").unwrap().unwrap().status, FileStatus::Local);
    // Everything of the folder freed: the folder is done.
    let answer = free_selection(&db, &ids(&["d-a"]), candidates(), frees(&db, &["f-1", "f-2"]));
    assert_eq!(answer, [("d-a".to_string(), Reclaimed::Freed)]);
    // A file that is online only already: nothing is asked, and nothing of it is here.
    let answer = free_selection(&db, &ids(&["f-4"]), Vec::new(), |_| panic!("nothing to reclaim"));
    assert_eq!(answer, [("f-4".to_string(), Reclaimed::Freed)]);
}

/// An id with nothing to hand to the reclaim is not "freed" while its bytes are on this computer: the answer is what
/// the rows say afterwards, with the reason. (It was `true` for every one of these: "all of nothing was freed".)
#[test]
fn what_could_not_be_freed_is_never_answered_as_freed() {
    use crate::free_up_space::{Reclaimed, free_selection};
    let dir = tempfile::tempdir().unwrap();
    let db = StateDb::open(&dir.path().join("state.db")).unwrap();
    held(&db, "f-up", "/up.txt", ItemKind::File, FileStatus::Uploading, false);
    held(&db, "f-conf", "/conf.txt", ItemKind::File, FileStatus::Conflict, false);
    held(&db, "f-err", "/err.txt", ItemKind::File, FileStatus::Error, false);
    // Kept after the selection was read: the candidates no longer hold it.
    held(&db, "f-kept", "/kept.txt", ItemKind::File, FileStatus::Local, true);
    // A candidate the reclaim could not free (open in an app).
    local(&db, "f-open", "/open.txt", ItemKind::File);
    // A folder: one file is freed, one is uploading.
    local(&db, "d-k", "/K", ItemKind::Folder);
    local(&db, "f-k1", "/K/one.txt", ItemKind::File);
    held(&db, "f-k2", "/K/two.txt", ItemKind::File, FileStatus::Uploading, false);
    // A folder: one file is kept, one could not be freed.
    local(&db, "d-m", "/M", ItemKind::Folder);
    held(&db, "f-m1", "/M/one.txt", ItemKind::File, FileStatus::Local, true);
    local(&db, "f-m2", "/M/two.txt", ItemKind::File);
    // As the engine's query answers: downloaded and not kept.
    let candidates: Vec<(String, String, i64)> = [
        ("f-open", "/open.txt"),
        ("d-k", "/K"),
        ("f-k1", "/K/one.txt"),
        ("d-m", "/M"),
        ("f-m2", "/M/two.txt"),
    ]
    .iter()
    .map(|(id, path)| (id.to_string(), path.to_string(), 10))
    .collect();
    let mut handed = Vec::new();
    let selection = ids(&["f-up", "f-conf", "f-err", "f-kept", "f-open", "d-k", "d-m"]);
    let answer = free_selection(&db, &selection, candidates, |wanted| {
        handed = wanted.iter().map(|(id, _, _)| id.clone()).collect();
        handed.sort();
        frees(&db, &["f-k1"])(wanted);
    });
    assert_eq!(handed, ["f-k1", "f-m2", "f-open"]);
    let said: Vec<(&str, Reclaimed)> = answer.iter().map(|(id, what)| (id.as_str(), *what)).collect();
    assert_eq!(
        said,
        [
            ("f-up", Reclaimed::Unsent),
            ("f-conf", Reclaimed::Unsent),
            ("f-err", Reclaimed::Unsent),
            ("f-kept", Reclaimed::Kept),
            ("f-open", Reclaimed::Left),
            ("d-k", Reclaimed::Unsent),
            ("d-m", Reclaimed::Kept),
        ]
    );
    // Nothing but the one file the reclaim freed changed.
    assert_eq!(db.get_file("f-k1").unwrap().unwrap().status, FileStatus::CloudOnly);
    assert_eq!(db.get_file("f-up").unwrap().unwrap().status, FileStatus::Uploading);
}

/// `Engine::free_up_space_of` asks who is signed in and whose the local state is before anything is touched.
#[test]
fn free_up_space_for_a_selection_is_the_owners_alone() {
    let engine = engine();
    let dir = tempfile::tempdir().unwrap();
    let root = tempfile::tempdir().unwrap();
    engine.use_local_state_for_test(state(dir.path(), Some((Some("user-a"), Some("a@example.eu"))), true));
    engine.use_sync_root_for_test(root.path().to_path_buf());
    let ids = vec!["f-1".to_string()];
    // Signed out. (The gate is the view's: it is `Ready` only after a real sign-in, so this refusal stands for every
    // session these tests put in memory; the handle's own answers are the cases below it.)
    assert_eq!(
        engine.free_up_space_of(&ids).err().as_deref(),
        Some(crate::app::kept_api::NOT_SIGNED_IN)
    );
    // The handle for changes, as `free_up_space_of` takes it: the owner's reaches the selection's reclaim; another
    // account's is refused before it.
    signed_in(&engine, Some("user-a"), Some("a@example.eu"));
    let mine = engine.local_state_for_changes().expect("the owner's handle");
    let answer =
        mine.with_while(|db, go_on| crate::free_up_space::free_up_space_of(&REAL, db, root.path(), &ids, go_on));
    #[cfg(not(target_os = "windows"))]
    assert_eq!(
        answer,
        Ok(Err(crate::free_up_space::NO_SELECTION_RECLAIM.to_string())),
        "no reclaim for a selection off Windows"
    );
    // The row is online only: nothing to free, so it is done.
    #[cfg(target_os = "windows")]
    assert_eq!(
        answer,
        Ok(Ok(vec![("f-1".to_string(), crate::free_up_space::Reclaimed::Freed)]))
    );
    signed_out(&engine);
    signed_in(&engine, Some("user-b"), Some("b@example.eu"));
    assert!(
        engine.local_state_for_changes().is_err(),
        "another account was handed the state"
    );
    signed_out(&engine);
}
