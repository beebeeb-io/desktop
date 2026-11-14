//! A file's versions at the engine, against the fake account on loopback: what is listed and in which order, what is
//! sent and when, and the session rule (a press belongs to the session it was made under). Sessions are put in memory
//! as a sign-in leaves them, with the fake account's own token and key; each test has a state database of its own.
//! Requests are counted at the fake as they arrive. Every name is invented.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

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

const WAIT: Duration = Duration::from_secs(30);
const FILE: &str = "a0000000-0000-4000-8000-000000000701";
const FOLDER: &str = "a0000000-0000-4000-8000-000000000702";
const NOT_HELD: &str = "a0000000-0000-4000-8000-000000000799";

type TestEngine = Engine<MemoryCredentials>;

struct Rig {
    engine: Arc<TestEngine>,
    account: Arc<FakeAccount>,
    fake: FakeApi,
    session: SessionGeneration,
    state_path: PathBuf,
    /// The next `GET .../versions` says on the first that it reached the fake, then waits on the second.
    hold: Arc<Mutex<Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>>>,
    _dir: tempfile::TempDir,
}

fn put(db: &StateDb, id: &str, name: &str, kind: ItemKind) {
    let entry = FileEntry {
        file_id: id.to_string(),
        path: format!("/{name}"),
        status: FileStatus::CloudOnly,
        size_bytes: 10,
        modified_at: 1_794_000_000,
        content_hash: None,
        remote_updated_at: 1_794_000_000,
        parent_id: None,
        item_kind: kind,
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

/// A file with three versions (added oldest-but-one, newest, oldest... so the server's order is not the date's):
/// `ver-a` of 1 Nov, `ver-c` of 3 Nov, `ver-b` of 2 Nov, the last one added being the current one.
fn rig() -> Rig {
    let (account, _phrase) = FakeAccount::register("versions-api@example.eu", "an invented password", TEST_PHRASE);
    let account = Arc::new(account);
    account.add_item(FILE, None, "Invented notes.txt", false, 10);
    account.add_item(FOLDER, None, "Invented folder", true, 0);
    account.add_version(FILE, "ver-a", "2026-11-01T10:00:00Z", 11);
    account.add_version(FILE, "ver-c", "2026-11-03T10:00:00Z", 13);
    account.add_version(FILE, "ver-b", "2026-11-02T10:00:00Z", 12);
    let hold: Arc<Mutex<Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>>> = Arc::default();
    let (handler, held) = (account_handler(account.clone()), hold.clone());
    let fake = FakeApi::start(Arc::new(move |req: &Request| {
        if req.route().starts_with("GET ") && req.route().ends_with("/versions") {
            let waiting = held.lock().unwrap().take();
            if let Some((reached, release)) = waiting {
                let _ = reached.send(());
                let _ = release.recv_timeout(WAIT);
            }
        }
        handler(req)
    }));
    let state_dir = crate::state_paths::init_for_test();
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "versions-api-test-slot".into(),
        state_dir,
        api_base: fake.base.clone(),
    };
    let engine = Arc::new(Engine::open(&REAL, config, host, MemoryCredentials::default()).expect("the handle opens"));
    let dir = tempfile::tempdir().unwrap();
    let state_path = dir.path().join("state.db");
    let db = StateDb::open(&state_path).unwrap();
    put(&db, FILE, "Invented notes.txt", ItemKind::File);
    put(&db, FOLDER, "Invented folder", ItemKind::Folder);
    db.set_owner(&Identity::new(Some(&account.user_id), Some(&account.email)))
        .unwrap();
    drop(db);
    engine.use_local_state_for_test(state_path.clone());
    let session = sign_in(&engine, &account);
    Rig {
        engine,
        account,
        fake,
        session,
        state_path,
        hold,
        _dir: dir,
    }
}

/// The requests about versions that reached the fake, whatever token they carried.
fn version_requests(rig: &Rig) -> Vec<String> {
    rig.fake
        .requests()
        .iter()
        .map(Request::route)
        .filter(|route| route.contains("/versions"))
        .collect()
}

fn ids(listed: &FileVersions) -> Vec<(&str, bool)> {
    listed.versions.iter().map(|v| (v.id.as_str(), v.current)).collect()
}

#[test]
fn versions_are_listed_current_first() {
    let rig = rig();
    let listed = rig.engine.file_versions_for(rig.session, FILE).expect("listed");
    assert_eq!(listed.session, rig.session);
    // The server wrote them oldest first, the current one last: here the current first, then the newest.
    assert_eq!(ids(&listed), [("ver-b", true), ("ver-c", false), ("ver-a", false)]);
    assert_eq!(listed.versions[1].size, 13);
    assert_eq!(listed.versions[1].created_at, "2026-11-03T10:00:00Z");
    assert!(!format!("{listed:?}").contains("ver-"), "a {{:?}} prints no version");
    assert_eq!(version_requests(&rig).len(), 1, "one request");
}

#[test]
fn versions_of_an_id_the_state_does_not_hold_are_not_found_and_nothing_is_sent() {
    let rig = rig();
    for id in [NOT_HELD, FOLDER] {
        assert_eq!(
            rig.engine.file_versions_for(rig.session, id).unwrap_err(),
            VersionsRefused::NotFound,
            "{id}"
        );
        assert_eq!(
            rig.engine
                .restore_file_version_for(rig.session, id, "ver-a")
                .unwrap_err(),
            VersionsRefused::NotFound,
            "{id}"
        );
    }
    assert_eq!(version_requests(&rig), Vec::<String>::new(), "nothing was sent");
}

#[test]
fn versions_for_a_session_that_moved_on_send_nothing() {
    let rig = rig();
    sign_out(&rig.engine);
    let again = sign_in(&rig.engine, &rig.account);
    assert_ne!(again, rig.session);
    assert_eq!(
        rig.engine.file_versions_for(rig.session, FILE).unwrap_err(),
        VersionsRefused::NotSignedIn,
        "a sign-out and a sign-in in between"
    );
    // While a sign-out is ending the session (it is not yet gone from memory).
    let (during, said) = (rig.engine.clone(), Arc::new(Mutex::new(None)));
    let heard = said.clone();
    let now = rig.engine.session_turn();
    rig.engine.during_next_forget_for_test(Box::new(move || {
        *heard.lock().unwrap() = Some(during.file_versions_for(now, FILE).map(|_| ()));
    }));
    sign_out(&rig.engine);
    assert_eq!(
        said.lock().unwrap().take().expect("the hook ran"),
        Err(VersionsRefused::NotSignedIn),
        "a session that is ending"
    );
    assert_eq!(version_requests(&rig), Vec::<String>::new(), "nothing was sent");
}

#[test]
fn a_list_that_outlives_its_session_is_not_handed_over() {
    let rig = rig();
    let (reached, was_reached) = mpsc::channel();
    let (release, released) = mpsc::channel();
    *rig.hold.lock().unwrap() = Some((reached, released));
    let (engine, session) = (rig.engine.clone(), rig.session);
    let (answered, answer) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = answered.send(
            engine
                .file_versions_for(session, FILE)
                .map(|listed| listed.versions.len()),
        );
    });
    was_reached.recv_timeout(WAIT).expect("the request reached the fake");
    sign_out(&rig.engine);
    release.send(()).expect("the fake holds the reply");
    assert_eq!(
        answer.recv_timeout(WAIT).expect("the call ended"),
        Err(VersionsRefused::NotSignedIn)
    );
}

#[test]
fn a_restore_is_made_once_and_asks_for_a_resnapshot() {
    let rig = rig();
    assert_eq!(rig.state_ask(), None, "no ask yet");
    rig.engine
        .restore_file_version_for(rig.session, FILE, "ver-a")
        .expect("restored");
    assert_eq!(rig.account.restore_requests(), 1, "sent once");
    assert_eq!(
        version_requests(&rig)
            .iter()
            .filter(|route| route.ends_with("/restore"))
            .count(),
        1
    );
    let (current, count) = rig.account.version_state(FILE).unwrap();
    assert_eq!(count, 4, "the server minted one more version");
    assert!(current.contains("ver-a"), "made from the one chosen: {current}");
    assert!(
        rig.state_ask().is_some(),
        "the sync engine was asked to read the server again"
    );
}

#[test]
fn a_restore_the_server_refused_was_not_made() {
    let rig = rig();
    rig.account.refuse_next_restore_with(409);
    assert_eq!(
        rig.engine
            .restore_file_version_for(rig.session, FILE, "ver-a")
            .unwrap_err(),
        VersionsRefused::Call(AccountCallError::ServerError)
    );
    assert_eq!(rig.account.version_state(FILE).unwrap().1, 3, "nothing was made");
    assert_eq!(rig.state_ask(), None, "and nothing to read again");
    rig.account.refuse_next_restore_with(401);
    assert_eq!(
        rig.engine
            .restore_file_version_for(rig.session, FILE, "ver-a")
            .unwrap_err(),
        VersionsRefused::Call(AccountCallError::SessionEnded)
    );
}

#[test]
fn a_restore_whose_answer_was_lost_is_unconfirmed() {
    let rig = rig();
    rig.account.make_next_restore_then_hang_up();
    assert_eq!(
        rig.engine
            .restore_file_version_for(rig.session, FILE, "ver-a")
            .unwrap_err(),
        VersionsRefused::Unconfirmed
    );
    assert_eq!(
        rig.account.version_state(FILE).unwrap().1,
        4,
        "it was made, and said so to nobody"
    );
    assert_eq!(rig.account.restore_requests(), 1, "and never sent again");
}

#[test]
fn a_restore_answered_502_is_unconfirmed() {
    let rig = rig();
    rig.account.refuse_next_restore_with(502);
    assert_eq!(
        rig.engine
            .restore_file_version_for(rig.session, FILE, "ver-a")
            .unwrap_err(),
        VersionsRefused::Unconfirmed
    );
    assert_eq!(rig.account.restore_requests(), 1);
}

#[test]
fn a_restore_for_a_session_that_moved_on_sends_nothing() {
    let rig = rig();
    sign_out(&rig.engine);
    sign_in(&rig.engine, &rig.account);
    assert_eq!(
        rig.engine
            .restore_file_version_for(rig.session, FILE, "ver-a")
            .unwrap_err(),
        VersionsRefused::NotSignedIn,
        "a sign-out and a sign-in in between"
    );
    let (during, said) = (rig.engine.clone(), Arc::new(Mutex::new(None)));
    let heard = said.clone();
    let now = rig.engine.session_turn();
    rig.engine.during_next_forget_for_test(Box::new(move || {
        *heard.lock().unwrap() = Some(during.restore_file_version_for(now, FILE, "ver-a"));
    }));
    sign_out(&rig.engine);
    assert_eq!(
        said.lock().unwrap().take().expect("the hook ran"),
        Err(VersionsRefused::NotSignedIn),
        "a session that is ending"
    );
    assert_eq!(rig.account.restore_requests(), 0, "nothing was sent");
}

impl Rig {
    /// The sync engine's request to read the server again, as the state database holds it.
    fn state_ask(&self) -> Option<i64> {
        StateDb::open(&self.state_path)
            .unwrap()
            .peek_resnapshot_request()
            .unwrap()
    }
}
