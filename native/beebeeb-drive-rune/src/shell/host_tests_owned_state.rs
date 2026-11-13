//! Task 2488 at the window: the local state is reached through the engine's account-bound handle, so a session
//! sees the local state only while it is recorded as that session's account's.
//!
//! A real engine handle on credential stores in memory and the engine's fake API on loopback, with two invented
//! accounts. The local state is a database of this test's own (`Engine::use_local_state_for_test`), written with
//! the engine's writers and recorded as the first account's. The process's one state folder is shared by every test
//! of this binary, so no sync is started here: the start's refusal is put in the view as a start leaves it. The
//! real start and its refusal run in the engine crate's `the_local_state_follows_the_account` test.

use std::sync::Mutex;

use beebeeb_engine::account_binding::Identity;
use beebeeb_engine::app::{Engine, EngineConfig, LocalStateRefused, SyncStartRefused};
use beebeeb_engine::host::ChannelHost;
use beebeeb_engine::identity::TEST;
use beebeeb_engine::signin::gate::GateState as EngineGate;
use beebeeb_engine::signin::memory_store::MemoryCredentials;
use beebeeb_engine::state_db::{FileEntry, FileStatus, ItemKind, StateDb, TransferActivityInput};
use beebeeb_engine::sync_view::Start;
use beebeeb_engine::test_api::fake_account::{FakeAccount, TEST_PHRASE, account_handler};
use beebeeb_engine::test_api::{FakeApi, Handler, Request};
use zeroize::Zeroizing;

use super::*;
use crate::engine_host::open::Opened;
use crate::engine_host::{EngineDriveSource, EngineTraySource, LiveEngine};
use crate::frames::global::sync_off::Refused;
use crate::frames::tray::source::TraySource;
use crate::shell::actions::{ActionError, EngineActions, HostActions};
use crate::shell::app::{Dialog, Session};
use crate::shell::instance::ProfileHolder;
use crate::source::{DriveSource, SourceError};

const PASSWORD: &str = "an invented password for two accounts";
/// A second valid 12-word phrase (a BIP-39 test vector), for the second account.
const OTHER_PHRASE: &str = "legal winner thank year wave sausage worth useful legal winner thank yellow";

/// What the first account's local state holds. Every name is invented and long enough to be found nowhere else.
const FOLDER: &str = "Ledger-of-the-first-account-2488";
const FILE: &str = "notes-of-the-first-account-2488.txt";
const INNER: &str = "inner-page-of-the-first-account-2488.ods";
const FILE_BYTES: i64 = 4_815_162;

/// One fake API for two accounts: a request with a session's token goes to that session's account, a sign-in to
/// the account whose email it names (and its second leg to the same one).
fn two_accounts(first: Arc<FakeAccount>, second: Arc<FakeAccount>) -> FakeApi {
    let (to_first, to_second) = (account_handler(first.clone()), account_handler(second.clone()));
    let signing_in_second = Mutex::new(false);
    let handler: Handler = Arc::new(move |request: &Request| {
        let second_asked = match request.bearer() {
            Some(token) => token == second.session_token,
            None => {
                let mut signing_in = signing_in_second.lock().unwrap();
                if let Some(email) = request.json().get("email").and_then(|email| email.as_str()) {
                    *signing_in = email == second.email;
                }
                *signing_in
            }
        };
        if second_asked {
            to_second(request)
        } else {
            to_first(request)
        }
    });
    FakeApi::start(handler)
}

/// The engine behind the shell, with a start that is not carried out (see the module's doc): a start asked while
/// the local state is not the session's account's is refused in the view, as the engine's own start leaves it.
struct NoRealStart {
    live: Arc<LiveEngine<MemoryCredentials>>,
    engine: Arc<Engine<MemoryCredentials>>,
}

impl EngineActions for NoRealStart {
    fn session(&self) -> Session {
        self.live.session()
    }
    fn tray_inputs(&self) -> TrayInputs {
        self.live.tray_inputs()
    }
    fn pause_sync(&self) -> Result<(), ActionError> {
        Ok(())
    }
    fn resume_sync(&self) -> Result<(), ActionError> {
        Ok(())
    }
    fn open_in_file_explorer(&self) -> Result<(), ActionError> {
        Ok(())
    }
    fn profile_holder(&self) -> Option<ProfileHolder> {
        None
    }
    fn start_sync(&self) -> Result<(), ActionError> {
        if self
            .engine
            .local_state_refusal()
            .is_some_and(LocalStateRefused::not_this_accounts)
        {
            self.engine.set_view_for_test(|view| {
                view.start = Start::Refused(SyncStartRefused::OtherAccountsData);
                view.attempt += 1;
            });
        }
        Ok(())
    }
    fn stop_for_quit(&self) {}
    fn resume_after_cancelled_quit(&self) {}
    fn sync_refusal(&self) -> Option<crate::frames::global::sync_off::SyncOff> {
        self.live.sync_refusal()
    }
}

fn put(db: &StateDb, id: &str, path: &str, parent: Option<&str>, kind: ItemKind, bytes: i64) {
    let entry = FileEntry {
        file_id: id.to_string(),
        path: path.to_string(),
        status: FileStatus::CloudOnly,
        size_bytes: bytes,
        modified_at: 1_794_000_000,
        content_hash: None,
        remote_updated_at: 1_794_000_000,
        parent_id: parent.map(str::to_string),
        item_kind: kind,
    };
    db.upsert_file_with_contract(&entry, |contract| {
        contract.parent_id = entry.parent_id.clone();
        contract.item_kind = entry.item_kind.clone();
    })
    .unwrap();
}

/// Everything the window holds as text: each element's text and its accessibility label.
fn everything_drawn(rt: &Runtime<State>) -> String {
    rt.elements()
        .into_iter()
        .flat_map(|e| [e.text, e.label].into_iter().flatten())
        .collect::<Vec<_>>()
        .join("\n")
}

fn sign_in(engine: &Engine<MemoryCredentials>, account: &FakeAccount, phrase: &str) {
    engine
        .sign_in_password(&account.email, Zeroizing::new(PASSWORD.into()))
        .expect("the fake accepts the password");
    engine
        .unlock_with_phrase(Zeroizing::new(phrase.to_string()))
        .expect("the account's phrase unlocks");
    assert_eq!(engine.gate(), EngineGate::Ready);
}

/// The first account's local state stays on this PC after its sign-out. The second account, signed in and
/// unlocked, is shown the refusal and nothing of the first account: not in the window's elements (text or label),
/// not in the flyout, not in a source's answer. The first account, back, has its list again.
#[test]
fn another_accounts_local_state_is_refused_and_nothing_of_it_is_shown() {
    let (first, _) = FakeAccount::register("first-2488@example.eu", PASSWORD, TEST_PHRASE);
    let (second, _) = FakeAccount::register("second-2488@example.eu", PASSWORD, OTHER_PHRASE);
    let (first, second) = (Arc::new(first), Arc::new(second));
    let fake = two_accounts(first.clone(), second.clone());

    let (host, messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "owned-state-window-slot".into(),
        state_dir: crate::engine_host::tests::process_state_dir(),
        api_base: fake.base.clone(),
    };
    let engine = Arc::new(Engine::open(&TEST, config, host, MemoryCredentials::default()).expect("the engine opens"));
    engine.restore();

    // The first account's local state, as its sync left it: rows, a finished transfer, and the owner's record.
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("state.db");
    let db = StateDb::open(&db_path).unwrap();
    put(&db, "d-ledger", &format!("/{FOLDER}"), None, ItemKind::Folder, 0);
    put(&db, "f-notes", &format!("/{FILE}"), None, ItemKind::File, FILE_BYTES);
    put(
        &db,
        "f-inner",
        &format!("/{FOLDER}/{INNER}"),
        Some("d-ledger"),
        ItemKind::File,
        1_200,
    );
    db.record_transfer_activity(TransferActivityInput {
        direction: "up",
        file_id: Some("f-notes".into()),
        file_name: FILE.into(),
        rel_path: Some(FILE.into()),
        bytes: FILE_BYTES,
        occurred_at: chrono::Utc::now().timestamp() - 60,
    })
    .unwrap();
    db.set_owner(&Identity::new(Some(&first.user_id), Some(&first.email)))
        .unwrap();
    drop(db);
    engine.use_local_state_for_test(db_path);

    let opened = Opened {
        engine: engine.clone(),
        identity: &TEST,
        gate: engine.gate(),
    };
    let live = Arc::new(LiveEngine::new(&opened, messages));
    let platform: Arc<FakePlatform> = Arc::new(FakePlatform::default());
    let actions = Arc::new(NoRealStart {
        live: live.clone(),
        engine: engine.clone(),
    });
    let shell = Shell::new(
        Arc::new(HostActions::new(actions, platform.clone())),
        platform,
        Wiring::default(),
    );
    let mut rt = runtime(state::init_on_engine(shell, engine.clone()));
    let drive = EngineDriveSource::new(engine.clone());
    let tray = EngineTraySource::new(live.clone());
    let names =
        |rt: &Runtime<State>| -> Vec<String> { rt.state.listing.items.iter().map(|i| i.name.clone()).collect() };
    let size = crate::source::format_size(FILE_BYTES);
    let needles = [FOLDER, FILE, INNER, size.as_str(), "4815162", "4,815,162"];
    // What must not be anywhere while the session is not the first account's.
    let assert_nothing_of_the_first = |rt: &Runtime<State>, when: &str| {
        let drawn = everything_drawn(rt);
        let flyout = format!(
            "{:?} {:?}",
            tray.snapshot(),
            tray.facts().local.map(|local| local.rows.len())
        );
        for needle in needles {
            assert!(!drawn.contains(needle), "{when}: the window draws {needle:?}");
            assert!(!flyout.contains(needle), "{when}: the flyout holds {needle:?}");
        }
        assert!(rt.state.listing.items.is_empty(), "{when}: the window holds rows");
        assert!(tray.facts().local.is_none(), "{when}: the flyout read the local state");
        assert!(drive.list(None).is_err(), "{when}: the root is listed");
        assert!(
            drive.list(Some("d-ledger")).is_err(),
            "{when}: a folder is listed by its id"
        );
    };

    // Signed out: nothing.
    settle(&mut rt);
    assert_eq!(engine.local_state_refusal(), Some(LocalStateRefused::SignedOut));
    assert_nothing_of_the_first(&rt, "signed out");

    // The first account: its list, its flyout rows. (The control: the needles ARE drawn when they may be.)
    sign_in(&engine, &first, TEST_PHRASE);
    settle(&mut rt);
    assert_eq!(engine.local_state_refusal(), None);
    assert_eq!(names(&rt), [FOLDER, FILE]);
    assert_eq!(rt.state.list_error, None);
    let drawn = everything_drawn(&rt);
    assert!(
        drawn.contains(FOLDER) && drawn.contains(FILE),
        "the first account sees its own list"
    );
    assert!(drawn.contains(&size), "and its sizes");
    let local = tray
        .facts()
        .local
        .expect("the first account's flyout reads its local state");
    assert!(local.rows.iter().any(|row| row.path == FILE), "and lists its transfer");
    let inner = drive.list(Some("d-ledger")).expect("a folder of its own lists");
    assert_eq!(inner.items[0].name, INNER);
    let crumbs: Vec<_> = inner.path.iter().map(|crumb| crumb.name.clone()).collect();
    assert_eq!(crumbs[1..], [FOLDER], "with the engine's crumbs");
    assert_eq!(inner.folder.id.as_deref(), Some("d-ledger"));
    assert_eq!(drive.list(Some("gone")).err(), Some(SourceError::NotFound));
    assert!(rt.state.shell.sync_off.is_none(), "no refusal for the owner");

    // Signed out, keeping the local data: nothing is shown, nothing was deleted.
    engine.sign_out_keeping_local_data().expect("the sign-out");
    settle(&mut rt);
    assert_nothing_of_the_first(&rt, "after the first account's sign-out");
    assert!(
        StateDb::open(dir.path().join("state.db"))
            .unwrap()
            .has_account_data()
            .unwrap()
    );

    // The second account, signed in and unlocked: the refusal, and nothing of the first account.
    sign_in(&engine, &second, OTHER_PHRASE);
    settle(&mut rt);
    settle(&mut rt);
    assert_eq!(rt.state.shell.app.gate(), GateState::Ready);
    assert_eq!(engine.local_state_refusal(), Some(LocalStateRefused::OtherAccount));
    assert_eq!(drive.list(None).err(), Some(SourceError::NotThisAccounts));
    assert_eq!(rt.state.list_error, Some(SourceError::NotThisAccounts));
    assert!(
        rt.find("file-list-error-text").is_some(),
        "the window says why there is no list"
    );
    assert!(everything_drawn(&rt).contains(SourceError::NotThisAccounts.message()));
    assert_eq!(
        rt.state.shell.sync_off.as_ref().map(|refusal| refusal.kind),
        Some(Refused::OtherAccountsData),
        "the refused start's modal"
    );
    assert_eq!(rt.state.shell.app.dialog(), Some(&Dialog::SyncOff));
    assert_nothing_of_the_first(&rt, "the second account, signed in and unlocked");
    // A lock and an unlock of the second account change nothing of that.
    engine.lock();
    settle(&mut rt);
    assert_eq!(engine.local_state_refusal(), Some(LocalStateRefused::Locked));
    assert_nothing_of_the_first(&rt, "the second account, locked");

    // The second account leaves, the first is back: its list is as it was. Nothing was lost.
    engine.sign_out_keeping_local_data().expect("the second sign-out");
    settle(&mut rt);
    assert_nothing_of_the_first(&rt, "after the second account's sign-out");
    sign_in(&engine, &first, TEST_PHRASE);
    settle(&mut rt);
    settle(&mut rt);
    assert_eq!(engine.local_state_refusal(), None);
    assert_eq!(rt.state.list_error, None);
    assert_eq!(names(&rt), [FOLDER, FILE], "the first account's list is back");
    assert!(
        tray.facts()
            .local
            .is_some_and(|local| local.rows.iter().any(|row| row.path == FILE))
    );
    assert_eq!(drive.list(Some("d-ledger")).unwrap().items.len(), 1);
    drop(fake);
}
