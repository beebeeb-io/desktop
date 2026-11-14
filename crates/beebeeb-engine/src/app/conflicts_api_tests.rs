//! Two versions at the engine, against the fake account on loopback: which rows are listed and from what (the file
//! on disk, the server's current version), what is left out, what an answer refuses, and the session rule. Every
//! name is invented.

use std::path::PathBuf;
use std::sync::Arc;

use super::*;
use crate::account::SessionTransition;
use crate::account_binding::Identity;
use crate::app::EngineConfig;
use crate::host::ChannelHost;
use crate::identity::REAL;
use crate::session::Session;
use crate::signin::memory_store::MemoryCredentials;
use crate::state_db::{FileEntry, FileStatus, ItemKind, StateDb};
use crate::test_api::fake_account::{FakeAccount, TEST_PHRASE, account_handler};
use crate::test_api::{FakeApi, Request};

const WAITING: &str = "a0000000-0000-4000-8000-000000000901";
const SYNCED: &str = "a0000000-0000-4000-8000-000000000902";
const GONE: &str = "a0000000-0000-4000-8000-000000000903";

type TestEngine = Engine<MemoryCredentials>;

struct Rig {
    engine: Arc<TestEngine>,
    account: Arc<FakeAccount>,
    fake: FakeApi,
    session: SessionGeneration,
    root: PathBuf,
    _dir: tempfile::TempDir,
}

fn put(db: &StateDb, id: &str, name: &str, status: FileStatus) {
    let entry = FileEntry {
        file_id: id.to_string(),
        path: format!("/{name}"),
        status,
        size_bytes: 10,
        modified_at: 1_794_000_000,
        content_hash: None,
        remote_updated_at: 1_794_000_000,
        parent_id: None,
        item_kind: ItemKind::File,
    };
    db.upsert_file_with_contract(&entry, |contract| contract.item_kind = entry.item_kind.clone())
        .unwrap();
}

/// A session in memory, as a sign-in and an unlock leave it: the fake's own token and key.
fn sign_in(engine: &TestEngine, account: &FakeAccount) -> SessionGeneration {
    {
        let _writes = engine.writes.write().unwrap();
        engine
            .acct
            .advance_session_generation_holding_session_write_lock(SessionTransition::Write);
        *engine.acct.session.lock().unwrap() = Some(Session {
            token: account.session_token.clone(),
            master_key: account.master_key,
            email: Some(account.email.clone()),
        });
        engine.remember_session_user_id(Some(account.user_id.clone()));
        engine.set_auth_present(true);
    }
    engine.session_turn()
}

fn sign_out(engine: &TestEngine) {
    engine.forget_session(SessionTransition::SignOut);
    engine.remember_session_user_id(None);
    engine.set_auth_present(false);
}

fn rig() -> Rig {
    let (account, _phrase) = FakeAccount::register("conflicts-api@example.eu", "an invented password", TEST_PHRASE);
    let account = Arc::new(account);
    for (id, name) in [
        (WAITING, "Invented draft.txt"),
        (SYNCED, "Invented fine.txt"),
        (GONE, "Invented gone.txt"),
    ] {
        account.add_item(id, None, name, false, 10);
        account.add_version(id, &format!("{id}-old"), "2026-11-01T10:00:00Z", 11);
        account.add_version(id, &format!("{id}-new"), "2026-11-03T10:00:00Z", 2_048);
    }
    let handler = account_handler(account.clone());
    let fake = FakeApi::start(Arc::new(move |req: &Request| handler(req)));
    let state_dir = crate::state_paths::init_for_test();
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "conflicts-api-test-slot".into(),
        state_dir,
        api_base: fake.base.clone(),
    };
    let engine = Arc::new(Engine::open(&REAL, config, host, MemoryCredentials::default()).expect("the handle opens"));
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("sync");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("Invented draft.txt"), b"my edit on this pc").unwrap();
    std::fs::write(root.join("Invented fine.txt"), b"fine").unwrap();
    let state_path = dir.path().join("state.db");
    let db = StateDb::open(&state_path).unwrap();
    put(&db, WAITING, "Invented draft.txt", FileStatus::Conflict);
    put(&db, SYNCED, "Invented fine.txt", FileStatus::Local);
    // In conflict, but its file is not on this PC: a side that cannot be read.
    put(&db, GONE, "Invented gone.txt", FileStatus::Conflict);
    db.set_owner(&Identity::new(Some(&account.user_id), Some(&account.email)))
        .unwrap();
    drop(db);
    engine.use_local_state_for_test(state_path);
    engine.use_sync_root_for_test(root.clone());
    let session = sign_in(&engine, &account);
    Rig {
        engine,
        account,
        fake,
        session,
        root,
        _dir: dir,
    }
}

fn requests(rig: &Rig) -> usize {
    rig.fake
        .requests()
        .iter()
        .map(Request::route)
        .filter(|route| route.contains("/versions") || route.contains("/files/"))
        .count()
}

#[test]
fn only_conflict_rows_are_listed_with_both_sides_read() {
    let rig = rig();
    let listed = rig.engine.conflicts_for(rig.session).expect("listed");
    assert_eq!(listed.session, rig.session);
    assert_eq!(listed.files.len(), 1, "one file waits and has both sides");
    let one = &listed.files[0];
    assert_eq!(one.file_id, WAITING);
    assert_eq!(one.name, "Invented draft.txt");
    assert_eq!(one.here_size, 18, "the file on this PC, by its length on disk");
    assert!(
        one.here_modified.ends_with('Z') && one.here_modified.len() == 20,
        "{}",
        one.here_modified
    );
    assert_eq!(
        one.there_created, "2026-11-03T10:00:00Z",
        "the server's current version"
    );
    assert_eq!(one.there_size, 2_048);
    assert_eq!(
        listed.unread, 1,
        "the file that is not on this PC is counted, not drawn"
    );
    assert!(!format!("{listed:?}").contains("Invented"), "a {{:?}} prints no name");
}

#[test]
fn nothing_is_asked_of_the_server_when_no_file_waits() {
    let rig = rig();
    {
        let db = StateDb::open(&rig.root.parent().unwrap().join("state.db")).unwrap();
        db.set_status(WAITING, FileStatus::Local).unwrap();
        db.set_status(GONE, FileStatus::Local).unwrap();
    }
    let listed = rig.engine.conflicts_for(rig.session).expect("listed");
    assert!(listed.files.is_empty() && listed.unread == 0);
    assert_eq!(requests(&rig), 0, "no request");
}

#[test]
fn a_file_that_does_not_wait_is_never_resolved_and_nothing_is_sent() {
    let rig = rig();
    for id in [SYNCED, "a0000000-0000-4000-8000-000000000999"] {
        for choice in [
            ConflictChoice::KeepHere,
            ConflictChoice::KeepThere,
            ConflictChoice::KeepBoth,
        ] {
            assert_eq!(
                rig.engine.resolve_conflict_for(rig.session, id, choice).unwrap_err(),
                ResolveRefused::NotWaiting,
                "{id} {choice:?}"
            );
        }
    }
    assert_eq!(requests(&rig), 0, "nothing was sent");
    assert_eq!(std::fs::read(rig.root.join("Invented fine.txt")).unwrap(), b"fine");
}

#[test]
fn a_session_that_moved_on_lists_and_resolves_nothing() {
    let rig = rig();
    sign_out(&rig.engine);
    let again = sign_in(&rig.engine, &rig.account);
    assert_ne!(again, rig.session);
    assert_eq!(
        rig.engine.conflicts_for(rig.session).unwrap_err(),
        ConflictsRefused::NotSignedIn
    );
    assert_eq!(
        rig.engine
            .resolve_conflict_for(rig.session, WAITING, ConflictChoice::KeepThere)
            .unwrap_err(),
        ResolveRefused::NotSignedIn
    );
    assert_eq!(requests(&rig), 0, "nothing was sent");
    assert_eq!(
        std::fs::read(rig.root.join("Invented draft.txt")).unwrap(),
        b"my edit on this pc",
        "this PC's file is untouched"
    );
    assert_eq!(
        std::fs::read_dir(&rig.root).unwrap().count(),
        2,
        "no copy was made either"
    );
}

#[test]
fn the_replaced_copy_is_made_beside_the_file_and_never_over_another() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("Invented draft.txt");
    std::fs::write(&file, b"only copy").unwrap();
    let first = keep_replaced_copy(&file).unwrap();
    let second = keep_replaced_copy(&file).unwrap();
    assert_eq!(first.file_name().unwrap(), "Invented draft (replaced on this PC).txt");
    assert_eq!(
        second.file_name().unwrap(),
        "Invented draft (replaced on this PC) 2.txt"
    );
    assert_eq!(std::fs::read(&first).unwrap(), b"only copy");
    assert_eq!(std::fs::read(&second).unwrap(), b"only copy");
    assert_eq!(
        std::fs::read(&file).unwrap(),
        b"only copy",
        "the file itself is untouched"
    );
}
