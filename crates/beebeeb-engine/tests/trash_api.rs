//! The account's Trash on the engine handle (`app::trash_api`) against the fake Beebeeb API on loopback: every item
//! listed with its name decrypted in the engine, an empty Trash, a refusal as its kind (never the server's body), a
//! handle with no session asks nothing, and Beebeeb gone is offline. Every account, name and key here is invented.
//!
//! The fake's own `GET /api/v1/files` answers `items` and `cursor`; the API client reads `files` and `next_cursor`
//! (the server's shape). This file's handler answers the Trash listing in the server's shape, from the fake's items.
//!
//! The TEST identity is refused on macOS (`EngineIdentity::refusal_on`), so the tests that open an Engine are gated
//! with `#[cfg(not(target_os = "macos"))]`.
#![cfg_attr(target_os = "macos", allow(unused_imports, dead_code))]

use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use beebeeb_engine::app::settings_api::AccountCallError;
use beebeeb_engine::app::{Engine, EngineConfig};
use beebeeb_engine::host::ChannelHost;
use beebeeb_engine::identity::TEST;
use beebeeb_engine::signin::memory_store::MemoryCredentials;
use beebeeb_engine::state_paths;
use beebeeb_engine::test_api::fake_account::{FakeAccount, TEST_PHRASE, account_handler, assert_loopback};
use beebeeb_engine::test_api::{FakeApi, Reply, Request};
use serde_json::{Value, json};
use zeroize::Zeroizing;

const PASSWORD: &str = "an invented password for trash";
const BODY_MARKER: &str = "server-body-must-not-leak";

const ANSWER: u8 = 0;
const SERVER_ERROR: u8 = 1;
const SESSION_ENDED: u8 = 2;

const DOCS: &str = "a0000000-0000-4000-8000-000000000101";
const CONTRACT: &str = "a0000000-0000-4000-8000-000000000102";
const OLD: &str = "a0000000-0000-4000-8000-000000000103";

type Hook = Arc<Mutex<Option<Box<dyn FnOnce() + Send>>>>;

struct Account {
    fake: FakeApi,
    vault: Arc<FakeAccount>,
    mode: Arc<AtomicU8>,
    /// The listing answers one item per page, `next_cursor` to the next.
    paged: Arc<AtomicBool>,
    /// Runs once, inside the next listing request, before it answers.
    during: Hook,
    email: String,
    phrase: String,
}

fn process_state_dir() -> std::path::PathBuf {
    static DIR: OnceLock<std::path::PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let base = tempfile::tempdir().expect("a temp folder").keep();
        state_paths::init_from_app_local_data_dir(&TEST, &base).expect("the state folder");
        state_paths::beebeeb_state_dir_for(&TEST).expect("the state folder")
    })
    .clone()
}

fn account(email: &str) -> Account {
    let (vault, phrase) = FakeAccount::register(email, PASSWORD, TEST_PHRASE);
    let vault = Arc::new(vault);
    let token = vault.session_token.clone();
    let rest = account_handler(vault.clone());
    let mode = Arc::new(AtomicU8::new(ANSWER));
    let paged = Arc::new(AtomicBool::new(false));
    let during: Hook = Arc::new(Mutex::new(None));
    let (mode_in, paged_in, during_in) = (mode.clone(), paged.clone(), during.clone());
    let fake = FakeApi::start(Arc::new(move |req: &Request| {
        if !(req.route() == "GET /api/v1/files" && req.query("trashed") == Some("true")) {
            return rest(req);
        }
        match mode_in.load(Ordering::SeqCst) {
            SERVER_ERROR => return Reply::json(500, json!({ "error": BODY_MARKER })),
            SESSION_ENDED => return Reply::json(401, json!({ "error": BODY_MARKER })),
            _ => {}
        }
        if req.bearer() != Some(token.as_str()) {
            return Reply::json(401, json!({ "error": "unauthorized" }));
        }
        if let Some(hook) = during_in.lock().unwrap().take() {
            hook();
        }
        let answer = rest(req);
        let body: Value = serde_json::from_slice(&answer.body).expect("the fake's listing");
        let mut items = body["items"].as_array().cloned().unwrap_or_default();
        items.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
        if !paged_in.load(Ordering::SeqCst) {
            return Reply::ok(json!({ "files": items, "next_cursor": null }));
        }
        // One item per page: the cursor is the index of the next one.
        let at: usize = req.query("cursor").and_then(|c| c.parse().ok()).unwrap_or(0);
        let page: Vec<Value> = items.iter().skip(at).take(1).cloned().collect();
        let next = (at + 1 < items.len()).then(|| (at + 1).to_string());
        Reply::ok(json!({ "files": page, "next_cursor": next }))
    }));
    assert_loopback(&fake.base);
    Account {
        fake,
        vault,
        mode,
        paged,
        during,
        email: email.to_string(),
        phrase,
    }
}

fn engine_at(base: &str) -> Engine<MemoryCredentials> {
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "trash-api-slot".into(),
        state_dir: process_state_dir(),
        api_base: base.to_string(),
    };
    let engine = Engine::open(&TEST, config, host, MemoryCredentials::default()).expect("the engine opens");
    engine.restore();
    engine
}

fn signed_in(account: &Account) -> Engine<MemoryCredentials> {
    let engine = engine_at(&account.fake.base);
    engine
        .sign_in_password(&account.email, Zeroizing::new(PASSWORD.into()))
        .expect("the fake accepts the password");
    engine
        .unlock_with_phrase(Zeroizing::new(account.phrase.clone()))
        .expect("the phrase unlocks");
    engine
}

fn listings(fake: &FakeApi) -> usize {
    fake.requests()
        .iter()
        .filter(|r| r.route() == "GET /api/v1/files" && r.query("trashed") == Some("true"))
        .count()
}

#[cfg(not(target_os = "macos"))]
#[test]
fn the_trash_is_listed_with_names_decrypted_in_the_engine() {
    let account = account("trash-list@example.eu");
    account.vault.add_item(DOCS, None, "Invented documents", true, 0);
    account
        .vault
        .add_item(CONTRACT, None, "Invented contract.pdf", false, 1_200);
    account.vault.add_item(OLD, None, "Still here.txt", false, 10);
    account.vault.trash_item(DOCS);
    account.vault.trash_item(CONTRACT);
    let engine = signed_in(&account);

    let mut items = engine.trash_listing().expect("the fake answers").items;
    items.sort_by(|a, b| a.id.cmp(&b.id));
    let rows: Vec<_> = items
        .iter()
        .map(|i| (i.id.as_str(), i.name.as_str(), i.is_folder, i.size_bytes))
        .collect();
    assert_eq!(
        rows,
        [
            (DOCS, "Invented documents", true, 0),
            (CONTRACT, "Invented contract.pdf", false, 1_200),
        ],
        "only what is in Trash, each name decrypted with the vault key"
    );
    assert!(items.iter().all(|i| !i.updated_at.is_empty()));
    assert_eq!(listings(&account.fake), 1);

    // Restored on Beebeeb: no longer listed.
    engine.restore_items(&[CONTRACT]).expect("the fake restores it");
    let items = engine.trash_listing().expect("the fake answers").items;
    assert_eq!(items.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), [DOCS]);
}

#[cfg(not(target_os = "macos"))]
#[test]
fn an_empty_trash_is_an_empty_list() {
    let account = account("trash-empty@example.eu");
    account.vault.add_item(OLD, None, "Still here.txt", false, 10);
    let engine = signed_in(&account);
    assert!(engine.trash_listing().expect("the fake answers").items.is_empty());
}

#[cfg(not(target_os = "macos"))]
#[test]
fn refusals_are_kinds_signed_out_asks_nothing_and_gone_is_offline() {
    let account = account("trash-refused@example.eu");
    let engine = signed_in(&account);

    account.mode.store(SERVER_ERROR, Ordering::SeqCst);
    let error = engine.trash_listing().expect_err("a 500 is an error");
    assert_eq!(error, AccountCallError::ServerError);
    assert!(!format!("{error} {error:?}").contains(BODY_MARKER));

    account.mode.store(SESSION_ENDED, Ordering::SeqCst);
    let error = engine.trash_listing().expect_err("a 401 is an error");
    assert_eq!(error, AccountCallError::SessionEnded);
    assert!(!format!("{error} {error:?}").contains(BODY_MARKER));
    account.mode.store(ANSWER, Ordering::SeqCst);

    let signed_out = engine_at(&account.fake.base);
    let before = listings(&account.fake);
    assert_eq!(
        signed_out.trash_listing().expect_err("signed out"),
        AccountCallError::NotSignedIn
    );
    assert_eq!(listings(&account.fake), before, "nothing is asked without a session");

    let Account { fake, .. } = account;
    drop(fake);
    assert_eq!(
        engine.trash_listing().expect_err("no answer"),
        AccountCallError::Offline
    );
}

#[cfg(not(target_os = "macos"))]
#[test]
fn every_page_of_the_trash_is_listed() {
    let account = account("trash-pages@example.eu");
    for id in [DOCS, CONTRACT, OLD] {
        account.vault.add_item(id, None, "An invented name", false, 1);
        account.vault.trash_item(id);
    }
    account.paged.store(true, Ordering::SeqCst);
    let engine = signed_in(&account);
    let mut ids: Vec<_> = engine
        .trash_listing()
        .expect("the fake answers")
        .items
        .into_iter()
        .map(|i| i.id)
        .collect();
    ids.sort();
    assert_eq!(ids, [DOCS, CONTRACT, OLD], "three pages, one item each");
    assert_eq!(listings(&account.fake), 3, "the cursor is followed to the end");
}

#[cfg(not(target_os = "macos"))]
#[test]
fn a_listing_is_bound_to_its_session_and_one_that_outlives_it_is_dropped() {
    let account = account("trash-session@example.eu");
    account
        .vault
        .add_item(CONTRACT, None, "Invented contract.pdf", false, 1_200);
    account.vault.trash_item(CONTRACT);
    let engine = Arc::new(signed_in(&account));
    assert!(engine.has_session());
    let listing = engine.trash_listing().expect("the fake answers");
    assert_eq!(
        listing.session,
        engine.session_turn(),
        "the listing names the session it was made in"
    );

    // A lock while the call is under way: the answer is dropped, never handed over.
    let locker = engine.clone();
    *account.during.lock().unwrap() = Some(Box::new(move || {
        let _ = locker.lock();
    }));
    assert_eq!(
        engine.trash_listing().expect_err("the session ended during the call"),
        AccountCallError::NotSignedIn
    );
    assert!(!engine.has_session());
    assert_ne!(listing.session, engine.session_turn(), "a lock moves the session on");
}

#[cfg(not(target_os = "macos"))]
#[test]
fn wake_host_wakes_the_hosts_loop() {
    let woken = Arc::new(AtomicUsize::new(0));
    let count = woken.clone();
    let (host, _messages) = ChannelHost::new(move || {
        count.fetch_add(1, Ordering::SeqCst);
    });
    let config = EngineConfig {
        account_id: "trash-api-wake-slot".into(),
        state_dir: process_state_dir(),
        api_base: "http://127.0.0.1:9".into(),
    };
    let engine = Engine::open(&TEST, config, host, MemoryCredentials::default()).expect("the engine opens");
    let before = woken.load(Ordering::SeqCst);
    engine.wake_host();
    assert_eq!(woken.load(Ordering::SeqCst), before + 1);
    assert!(
        !engine.has_session(),
        "a handle that was never signed in has no session"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn the_test_identity_is_refused_on_macos() {
    assert!(TEST.refusal_on("macos").is_some());
}
