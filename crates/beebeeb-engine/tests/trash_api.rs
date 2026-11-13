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

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, OnceLock};

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

struct Account {
    fake: FakeApi,
    vault: Arc<FakeAccount>,
    mode: Arc<AtomicU8>,
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
    let mode_in = mode.clone();
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
        let answer = rest(req);
        let body: Value = serde_json::from_slice(&answer.body).expect("the fake's listing");
        Reply::ok(json!({ "files": body["items"], "next_cursor": null }))
    }));
    assert_loopback(&fake.base);
    Account {
        fake,
        vault,
        mode,
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

    let mut items = engine.trash_listing().expect("the fake answers");
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
    let items = engine.trash_listing().expect("the fake answers");
    assert_eq!(items.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), [DOCS]);
}

#[cfg(not(target_os = "macos"))]
#[test]
fn an_empty_trash_is_an_empty_list() {
    let account = account("trash-empty@example.eu");
    account.vault.add_item(OLD, None, "Still here.txt", false, 10);
    let engine = signed_in(&account);
    assert!(engine.trash_listing().expect("the fake answers").is_empty());
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

#[cfg(target_os = "macos")]
#[test]
fn the_test_identity_is_refused_on_macos() {
    assert!(TEST.refusal_on("macos").is_some());
}
