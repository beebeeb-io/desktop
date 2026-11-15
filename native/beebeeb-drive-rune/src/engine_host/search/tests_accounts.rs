//! Task 2488 for Search (slice A8), on the REAL engine handle: credential stores in memory and the engine's fake API on
//! loopback, with two invented accounts. The local state is a database of this test's own
//! (`Engine::use_local_state_for_test`), written with the engine's writers and recorded as the first account's. The
//! first account finds its own name and keeps its search; it signs out keeping the local data; the second account,
//! signed in and unlocked, is answered with none of the first account's names, facts or searches. The owner check
//! is the engine's own (`Engine::local_state`), so this test turns red when that check is skipped (the test double of
//! `tests.rs` carries a check of its own and cannot). Every name is invented.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use beebeeb_engine::account_binding::Identity;
use beebeeb_engine::app::{Engine, EngineConfig, LocalStateRefused};
use beebeeb_engine::host::ChannelHost;
use beebeeb_engine::signin::gate::GateState as EngineGate;
use beebeeb_engine::signin::memory_store::MemoryCredentials;
use beebeeb_engine::state_db::{FileEntry, FileStatus, ItemKind, StateDb};
use beebeeb_engine::test_api::fake_account::{FakeAccount, TEST_PHRASE, account_handler};
use beebeeb_engine::test_api::{FakeApi, Handler, Request};
use zeroize::Zeroizing;

use super::*;
use crate::shell::identity::engine_identity;

const PASSWORD: &str = "an invented password for two searching accounts";
/// A second valid 12-word phrase (a BIP-39 test vector), for the second account.
const OTHER_PHRASE: &str = "legal winner thank year wave sausage worth useful legal winner thank yellow";

/// The one name only the first account has, and the word searched for it. Invented, found nowhere else.
const NAME: &str = "Quarterly minutes of the first account 2488.txt";
const NEEDLE: &str = "minutes";

const WAIT_FOR: Duration = Duration::from_secs(30);

/// One fake API for two accounts (as `shell::host_tests_owned_state`): a request with a session's token goes to that
/// session's account, a sign-in to the account whose email it names (and its second leg to the same one).
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

fn sign_in(engine: &Engine<MemoryCredentials>, account: &FakeAccount, phrase: &str) {
    engine
        .sign_in_password(&account.email, Zeroizing::new(PASSWORD.into()))
        .expect("the fake accepts the password");
    engine
        .unlock_with_phrase(Zeroizing::new(phrase.to_string()))
        .expect("the account's phrase unlocks");
    assert_eq!(engine.gate(), EngineGate::Ready);
}

fn put(db: &StateDb, id: &str, path: &str) {
    let entry = FileEntry {
        file_id: id.to_string(),
        path: path.to_string(),
        status: FileStatus::Local,
        size_bytes: 48_213,
        modified_at: 1_794_000_000,
        content_hash: None,
        remote_updated_at: 1_794_000_000,
        parent_id: None,
        item_kind: ItemKind::File,
    };
    db.upsert_file_with_contract(&entry, |contract| {
        contract.item_kind = entry.item_kind.clone();
    })
    .unwrap();
}

type Source = EngineSearchSource<Engine<MemoryCredentials>>;

/// Pump until no build, no query and no work on the recent searches is on its way (at most 30 s).
fn settle(source: &mut Source) {
    let began = Instant::now();
    while source.pump().busy {
        assert!(began.elapsed() < WAIT_FOR, "an answer did not come back");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// The ids found for `query` on every item, after the worker answered; `None`: no result list (not available).
fn found(source: &mut Source, query: &str) -> Option<Vec<String>> {
    source.ask(query, Narrow::All);
    settle(source);
    match source.search(query, Narrow::All) {
        Answer::Found { hits, .. } => Some(hits.into_iter().map(|hit| hit.id).collect()),
        Answer::Searching => panic!("the query was not answered"),
        Answer::Unavailable => None,
    }
}

/// 2488's verification for Search: sign out keeping local data, sign in as another account, search a name only the
/// first account has: no result, no fact, no recent search, and no index of the first account's state in memory.
#[test]
fn search_never_answers_another_accounts_names() {
    let (first, _) = FakeAccount::register("first-search-2488@example.eu", PASSWORD, TEST_PHRASE);
    let (second, _) = FakeAccount::register("second-search-2488@example.eu", PASSWORD, OTHER_PHRASE);
    let (first, second) = (Arc::new(first), Arc::new(second));
    let fake = two_accounts(first.clone(), second.clone());

    let (channel, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "search-2488-window-slot".into(),
        state_dir: crate::engine_host::tests::process_state_dir(),
        api_base: fake.base.clone(),
    };
    let engine = Arc::new(
        Engine::open(engine_identity(), config, channel, MemoryCredentials::default()).expect("the engine opens"),
    );
    engine.restore();

    // The first account's local state, as its sync left it: a row only it has, and the owner's record.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let db = StateDb::open(&path).unwrap();
    put(&db, "f-minutes", &format!("/Board/{NAME}"));
    put(&db, "f-other", "/notes.txt");
    db.set_owner(&Identity::new(Some(&first.user_id), Some(&first.email)))
        .unwrap();
    drop(db);
    engine.use_local_state_for_test(path.clone());
    let mut source: Source = EngineSearchSource::new(engine.clone());

    // The first account: its own name is found (the control: the name IS found when it may be), its search kept.
    sign_in(&engine, &first, TEST_PHRASE);
    source.opened();
    settle(&mut source);
    assert!(source.wired(), "the first account's index");
    assert_eq!(
        found(&mut source, NEEDLE),
        Some(vec!["f-minutes".to_string()]),
        "the control: the first account finds its own name"
    );
    assert_eq!(
        source.details("f-minutes").map(|facts| facts.where_),
        Some("Board".to_string())
    );
    source.remember(NEEDLE);
    settle(&mut source);
    assert_eq!(source.recent(), [NEEDLE], "the control: its search is kept");

    // Signed out (the window's sign-out), keeping the local data: nothing of it in memory, its searches deleted.
    assert!(engine.sign_out_for(engine.session_turn()).is_ok(), "the sign-out");
    assert_eq!(
        source.search(NEEDLE, Narrow::All),
        Answer::Unavailable,
        "at once, before a pump"
    );
    assert!(source.recent().is_empty());
    source.pump();
    assert!(source.held.is_none() && source.found.is_none() && source.recent.is_none());
    let db = StateDb::open(&path).unwrap();
    assert!(db.has_account_data().unwrap(), "the local data was kept");
    assert!(
        !db.has_search_recent_row(&first.user_id).unwrap(),
        "the sign-out deleted the first account's recent searches"
    );
    drop(db);

    // The second account, signed in and unlocked over the first account's local state.
    sign_in(&engine, &second, OTHER_PHRASE);
    source.opened();
    settle(&mut source);
    let answer = found(&mut source, NEEDLE);
    assert!(
        answer.as_ref().is_none_or(|ids| ids.is_empty()),
        "the second account is answered with the first account's names: {} result(s)",
        answer.as_ref().map_or(0, Vec::len)
    );
    assert_eq!(
        answer, None,
        "no index is built of another account's state: not available"
    );
    assert!(!source.wired());
    assert!(
        source.held.is_none(),
        "no index of the first account's state is in memory"
    );
    assert_eq!(source.details("f-minutes"), None);
    assert_eq!(source.details("f-other"), None);
    assert!(source.recent().is_empty(), "no recent search of the first account");
    assert_eq!(engine.local_state_refusal(), Some(LocalStateRefused::OtherAccount));
    assert!(SearchHost::index(&*engine).is_err(), "the engine builds no index of it");
    assert_eq!(
        engine.recent_searches_for(engine.session_turn()),
        Err(LocalStateRefused::OtherAccount)
    );

    // Its own search is not kept in the first account's state either.
    source.remember("lisbon");
    settle(&mut source);
    assert!(source.recent().is_empty());
    let db = StateDb::open(&path).unwrap();
    assert!(!db.has_search_recent_row(&second.user_id).unwrap());
    drop(db);
    drop(fake);
}
