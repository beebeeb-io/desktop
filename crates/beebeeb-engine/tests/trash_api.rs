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
use beebeeb_engine::app::trash_api::NotTrashed;
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
/// A move to Trash is answered as Beebeeb answers for a protected folder.
const PROTECTED: u8 = 3;
/// A move to Trash is made and answered 204 with no body.
const NO_BODY: u8 = 4;

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
        let restoring = is_restore(req) || is_trash(req);
        if !restoring && !(req.route() == "GET /api/v1/files" && req.query("trashed") == Some("true")) {
            return rest(req);
        }
        match mode_in.load(Ordering::SeqCst) {
            SERVER_ERROR => return Reply::json(500, json!({ "error": BODY_MARKER })),
            SESSION_ENDED => return Reply::json(401, json!({ "error": BODY_MARKER })),
            PROTECTED if is_trash(req) => {
                return Reply::json(409, json!({ "error": "protected_folder", "message": BODY_MARKER }));
            }
            NO_BODY if is_trash(req) => {
                let made = rest(req);
                return if made.status == 200 { Reply::empty(204) } else { made };
            }
            _ => {}
        }
        if restoring {
            // The fake's own restore (it refuses a token that is not the account's), then the hook: the session
            // may end while the answer is on its way back.
            let answer = rest(req);
            if let Some(hook) = during_in.lock().unwrap().take() {
                hook();
            }
            return answer;
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

fn is_restore(req: &Request) -> bool {
    req.route().starts_with("POST /api/v1/files/") && req.route().ends_with("/restore")
}

fn is_trash(req: &Request) -> bool {
    req.route().starts_with("DELETE /api/v1/files/")
}

/// Every move-to-Trash request that reached the fake, whatever token it carried (counted before the fake looks at
/// one).
fn trashes(fake: &FakeApi) -> usize {
    fake.requests().iter().filter(|r| is_trash(r)).count()
}

/// Every restore request that reached the fake, whatever token it carried (counted before the fake looks at one).
fn restores(fake: &FakeApi) -> usize {
    fake.requests().iter().filter(|r| is_restore(r)).count()
}

fn trashed(engine: &Engine<MemoryCredentials>) -> Vec<String> {
    let mut ids: Vec<String> = engine
        .trash_listing()
        .expect("the fake answers")
        .items
        .into_iter()
        .map(|item| item.id)
        .collect();
    ids.sort();
    ids
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
    engine
        .restore_items_for(engine.session_turn(), &[CONTRACT])
        .expect("the fake restores it");
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

/// A listing asked while a lock is ending its session is never handed over, not even under the turn the lock
/// leaves behind: the call ends as "not signed in". The listing is asked while the lock is held half done.
#[cfg(not(target_os = "macos"))]
#[test]
fn a_listing_asked_while_its_session_is_ending_is_never_handed_over() {
    const WAIT: std::time::Duration = std::time::Duration::from_secs(30);
    let account = account("trash-ending@example.eu");
    account
        .vault
        .add_item(CONTRACT, None, "Invented contract.pdf", false, 1_200);
    account.vault.trash_item(CONTRACT);
    let engine = Arc::new(signed_in(&account));
    assert_eq!(
        engine
            .trash_listing()
            .expect("the control: the fake answers")
            .items
            .len(),
        1
    );

    // Since a person's call takes its session through the one choke point, a listing asked while the lock is under way
    // is refused before any request leaves: it never reaches the fake, so there is no reply to hold back.
    let (answered, answer) = std::sync::mpsc::channel();
    let reader = engine.clone();
    engine.during_next_forget_for_test(Box::new(move || {
        let (ended, ended_in) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = ended.send(reader.trash_listing().map(|listing| listing.items.len()));
        });
        // The lock stays half done until the listing's call ended.
        let _ = answered.send(ended_in.recv_timeout(WAIT).expect("the listing call ended"));
    }));
    let _ = engine.lock();
    assert!(!engine.has_session(), "locked");
    assert_eq!(
        answer.recv_timeout(WAIT).expect("the lock saw the listing end"),
        Err(AccountCallError::NotSignedIn),
        "a listing for a session that ended is not handed over"
    );
}

/// A listing's `{:?}` says how many items, never a name or an id.
#[cfg(not(target_os = "macos"))]
#[test]
fn a_listing_prints_no_name() {
    let account = account("trash-debug@example.eu");
    account
        .vault
        .add_item(CONTRACT, None, "Invented contract.pdf", false, 1_200);
    account.vault.trash_item(CONTRACT);
    let engine = signed_in(&account);
    let listing = engine.trash_listing().expect("the fake answers");
    assert_eq!(listing.items[0].name, "Invented contract.pdf");
    let printed = format!("{listing:?} {:?}", listing.items);
    assert!(
        !printed.contains("Invented") && !printed.contains(CONTRACT),
        "{printed}"
    );
    assert!(printed.contains("items: 1"), "{printed}");
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

/// A restore is sent only under the session it was pressed in: after a lock and an unlock (the same account, its
/// token still good), after a sign-out and a sign-in, and while a sign-out is ending the session, nothing is sent;
/// under the session that is still there each item is sent once. Sends are counted at the fake as they arrive.
#[cfg(not(target_os = "macos"))]
#[test]
fn a_restore_pressed_under_one_session_is_never_sent_under_another() {
    let account = account("trash-restore-turn@example.eu");
    for id in [DOCS, CONTRACT, OLD] {
        account.vault.add_item(id, None, "An invented name", false, 1);
        account.vault.trash_item(id);
    }
    let engine = Arc::new(signed_in(&account));

    let pressed_under = engine.session_turn();
    let _ = engine.lock();
    engine
        .unlock_with_phrase(Zeroizing::new(account.phrase.clone()))
        .expect("the phrase unlocks");
    assert_eq!(
        engine
            .restore_items_for(pressed_under, &[CONTRACT])
            .expect_err("the session moved on"),
        AccountCallError::NotSignedIn,
        "a lock and an unlock in between"
    );
    assert_eq!(restores(&account.fake), 0, "nothing was sent after the unlock");

    let pressed_under = engine.session_turn();
    engine.sign_out_keeping_local_data().expect("signed out");
    engine
        .sign_in_password(&account.email, Zeroizing::new(PASSWORD.into()))
        .expect("the fake accepts the password");
    engine
        .unlock_with_phrase(Zeroizing::new(account.phrase.clone()))
        .expect("the phrase unlocks");
    assert_eq!(
        engine
            .restore_items_for(pressed_under, &[CONTRACT])
            .expect_err("the session moved on"),
        AccountCallError::NotSignedIn,
        "a sign-out and a sign-in in between"
    );
    assert_eq!(restores(&account.fake), 0, "nothing was sent after the sign-in");
    assert_eq!(trashed(&engine), [DOCS, CONTRACT, OLD], "everything is still in Trash");

    // Under the session that is still there: sent once, and Beebeeb no longer lists it. No item: nothing is asked.
    let now = engine.session_turn();
    engine.restore_items_for(now, &[]).expect("nothing to do");
    assert_eq!(restores(&account.fake), 0, "no item, no request");
    engine
        .restore_items_for(now, &[CONTRACT])
        .expect("the session is the one of the press");
    assert_eq!(restores(&account.fake), 1, "sent once");
    assert_eq!(trashed(&engine), [DOCS, OLD]);

    // While a sign-out is ending the session (it is not yet gone from memory): nothing is sent.
    let (during, said) = (engine.clone(), Arc::new(Mutex::new(None)));
    let heard = said.clone();
    engine.during_next_forget_for_test(Box::new(move || {
        *heard.lock().unwrap() = Some(during.restore_items_for(now, &[DOCS]));
    }));
    engine.sign_out_keeping_local_data().expect("signed out");
    assert_eq!(
        said.lock().unwrap().take().expect("the hook ran").expect_err("ending"),
        AccountCallError::NotSignedIn,
        "a session that is ending"
    );
    assert_eq!(restores(&account.fake), 1, "and nothing more was sent");
}

/// The session is asked again before EACH item: a lock while the first item is answered stops the second.
#[cfg(not(target_os = "macos"))]
#[test]
fn a_session_that_ends_between_two_items_stops_the_restore_there() {
    let account = account("trash-restore-items@example.eu");
    for id in [DOCS, CONTRACT] {
        account.vault.add_item(id, None, "An invented name", false, 1);
        account.vault.trash_item(id);
    }
    let engine = Arc::new(signed_in(&account));
    let pressed_under = engine.session_turn();
    let locker = engine.clone();
    *account.during.lock().unwrap() = Some(Box::new(move || {
        let _ = locker.lock();
    }));
    assert_eq!(
        engine
            .restore_items_for(pressed_under, &[DOCS, CONTRACT])
            .expect_err("the session ended after the first item"),
        AccountCallError::NotSignedIn
    );
    assert_eq!(
        restores(&account.fake),
        1,
        "the first item was sent, the second was not"
    );
    let sent = account
        .fake
        .requests()
        .into_iter()
        .find(is_restore)
        .expect("one restore");
    assert_eq!(sent.route(), format!("POST /api/v1/files/{DOCS}/restore"));
}

/// The session-bound restore on every other answer: a server error and a refused session each their own kind (never
/// the body), an item Beebeeb does not have in this account's Trash is refused, a handle with no session asks
/// nothing, and Beebeeb gone is offline.
#[cfg(not(target_os = "macos"))]
#[test]
fn a_session_bound_restore_fails_by_kind() {
    let account = account("trash-restore-kinds@example.eu");
    account.vault.add_item(CONTRACT, None, "An invented name", false, 1);
    account.vault.trash_item(CONTRACT);
    let engine = signed_in(&account);
    let now = engine.session_turn();

    account.mode.store(SERVER_ERROR, Ordering::SeqCst);
    let error = engine
        .restore_items_for(now, &[CONTRACT])
        .expect_err("a 500 is an error");
    assert_eq!(error, AccountCallError::ServerError);
    assert!(!format!("{error} {error:?}").contains(BODY_MARKER));

    account.mode.store(SESSION_ENDED, Ordering::SeqCst);
    let error = engine
        .restore_items_for(now, &[CONTRACT])
        .expect_err("a 401 is an error");
    assert_eq!(error, AccountCallError::SessionEnded);
    assert!(!format!("{error} {error:?}").contains(BODY_MARKER));
    account.mode.store(ANSWER, Ordering::SeqCst);
    assert_eq!(trashed(&engine), [CONTRACT], "refused: still in Trash");

    assert_eq!(
        engine
            .restore_items_for(now, &[OLD])
            .expect_err("not an item of this account"),
        AccountCallError::ServerError,
        "an id Beebeeb does not have for this account"
    );

    let signed_out = engine_at(&account.fake.base);
    let before = restores(&account.fake);
    assert_eq!(
        signed_out
            .restore_items_for(signed_out.session_turn(), &[CONTRACT])
            .expect_err("signed out"),
        AccountCallError::NotSignedIn
    );
    assert_eq!(restores(&account.fake), before, "nothing is asked without a session");

    let Account { fake, .. } = account;
    drop(fake);
    assert_eq!(
        engine.restore_items_for(now, &[CONTRACT]).expect_err("no answer"),
        AccountCallError::Offline
    );
}

/// A move to Trash is sent only under the session it was pressed under ([`Engine::trash_items_for`]): after a lock
/// and an unlock, after a sign-out and a sign-in, and while a sign-out is ending the session, nothing is sent; under
/// the session that is still there each item is sent once. Sends are counted at the fake as they arrive.
#[cfg(not(target_os = "macos"))]
#[test]
fn a_trash_pressed_under_one_session_is_never_sent_under_another() {
    let account = account("trash-move-turn@example.eu");
    for id in [DOCS, CONTRACT, OLD] {
        account.vault.add_item(id, None, "An invented name", false, 1);
    }
    let engine = Arc::new(signed_in(&account));
    let gone = NotTrashed::Call(AccountCallError::NotSignedIn);

    let pressed_under = engine.session_turn();
    let _ = engine.lock();
    engine
        .unlock_with_phrase(Zeroizing::new(account.phrase.clone()))
        .expect("the phrase unlocks");
    assert_eq!(
        engine.trash_items_for(pressed_under, &[CONTRACT]),
        Err(gone),
        "a lock and an unlock in between"
    );
    assert_eq!(trashes(&account.fake), 0, "nothing was sent after the unlock");

    let pressed_under = engine.session_turn();
    engine.sign_out_keeping_local_data().expect("signed out");
    engine
        .sign_in_password(&account.email, Zeroizing::new(PASSWORD.into()))
        .expect("the fake accepts the password");
    engine
        .unlock_with_phrase(Zeroizing::new(account.phrase.clone()))
        .expect("the phrase unlocks");
    assert_eq!(
        engine.trash_items_for(pressed_under, &[CONTRACT]),
        Err(gone),
        "a sign-out and a sign-in in between"
    );
    assert_eq!(trashes(&account.fake), 0, "nothing was sent after the sign-in");
    assert_eq!(trashed(&engine), Vec::<String>::new(), "nothing is in Trash");

    // Under the session that is still there: sent once each, in order, and Beebeeb lists them in Trash. No item:
    // nothing is asked.
    let now = engine.session_turn();
    engine.trash_items_for(now, &[]).expect("nothing to do");
    assert_eq!(trashes(&account.fake), 0, "no item, no request");
    engine
        .trash_items_for(now, &[CONTRACT, OLD])
        .expect("the session is the one of the press");
    assert_eq!(trashes(&account.fake), 2, "sent once each");
    assert_eq!(trashed(&engine), [CONTRACT, OLD]);
    let routes: Vec<String> = account
        .fake
        .requests()
        .iter()
        .filter(|r| is_trash(r))
        .map(|r| r.route())
        .collect();
    assert_eq!(
        routes,
        [
            format!("DELETE /api/v1/files/{CONTRACT}"),
            format!("DELETE /api/v1/files/{OLD}")
        ],
        "the file's own route: never the permanent delete"
    );

    // While a sign-out is ending the session (it is not yet gone from memory): nothing is sent.
    let (during, said) = (engine.clone(), Arc::new(Mutex::new(None)));
    let heard = said.clone();
    engine.during_next_forget_for_test(Box::new(move || {
        *heard.lock().unwrap() = Some(during.trash_items_for(now, &[DOCS]));
    }));
    engine.sign_out_keeping_local_data().expect("signed out");
    assert_eq!(
        said.lock().unwrap().take().expect("the hook ran"),
        Err(gone),
        "a session that is ending"
    );
    assert_eq!(trashes(&account.fake), 2, "and nothing more was sent");
}

/// The session is asked again before EACH item: a lock while the first item is answered stops the second.
#[cfg(not(target_os = "macos"))]
#[test]
fn a_session_that_ends_between_two_items_stops_the_trash_there() {
    let account = account("trash-move-items@example.eu");
    for id in [DOCS, CONTRACT] {
        account.vault.add_item(id, None, "An invented name", false, 1);
    }
    let engine = Arc::new(signed_in(&account));
    let pressed_under = engine.session_turn();
    let locker = engine.clone();
    *account.during.lock().unwrap() = Some(Box::new(move || {
        let _ = locker.lock();
    }));
    assert_eq!(
        engine.trash_items_for(pressed_under, &[DOCS, CONTRACT]),
        Err(NotTrashed::Call(AccountCallError::NotSignedIn)),
        "the session ended after the first item"
    );
    assert_eq!(trashes(&account.fake), 1, "the first item was sent, the second was not");
    let sent = account.fake.requests().into_iter().find(is_trash).expect("one trash");
    assert_eq!(sent.route(), format!("DELETE /api/v1/files/{DOCS}"));
}

/// The session-bound move to Trash on every other answer: a server error and a refused session each their own kind
/// (never the body), a protected folder its own, an item Beebeeb does not have for this account is refused, a
/// handle with no session asks nothing, and Beebeeb gone is offline. A failure stops the items after it.
#[cfg(not(target_os = "macos"))]
#[test]
fn a_session_bound_trash_fails_by_kind() {
    let account = account("trash-move-kinds@example.eu");
    for id in [DOCS, CONTRACT] {
        account.vault.add_item(id, None, "An invented name", false, 1);
    }
    let engine = signed_in(&account);
    let now = engine.session_turn();

    for (mode, kind) in [
        (SERVER_ERROR, NotTrashed::Call(AccountCallError::ServerError)),
        (SESSION_ENDED, NotTrashed::Call(AccountCallError::SessionEnded)),
        (PROTECTED, NotTrashed::Protected),
    ] {
        account.mode.store(mode, Ordering::SeqCst);
        let before = trashes(&account.fake);
        let error = engine.trash_items_for(now, &[CONTRACT, DOCS]).expect_err("refused");
        assert_eq!(error, kind);
        assert!(!format!("{error:?}").contains(BODY_MARKER));
        assert_eq!(
            trashes(&account.fake),
            before + 1,
            "the item after a failure is not sent"
        );
    }
    account.mode.store(ANSWER, Ordering::SeqCst);
    assert_eq!(trashed(&engine), Vec::<String>::new(), "refused: nothing is in Trash");

    // Made, and answered without a body to read: it is in Trash, and the next item is sent.
    account.mode.store(NO_BODY, Ordering::SeqCst);
    engine
        .trash_items_for(now, &[CONTRACT, DOCS])
        .expect("a 2xx is a trash that was made");
    account.mode.store(ANSWER, Ordering::SeqCst);
    assert_eq!(trashed(&engine), [DOCS, CONTRACT]);
    for id in [DOCS, CONTRACT] {
        assert!(account.vault.restore_item(id));
    }

    assert_eq!(
        engine.trash_items_for(now, &[OLD]),
        Err(NotTrashed::Call(AccountCallError::ServerError)),
        "an id Beebeeb does not have for this account"
    );

    let signed_out = engine_at(&account.fake.base);
    let before = trashes(&account.fake);
    assert_eq!(
        signed_out.trash_items_for(signed_out.session_turn(), &[CONTRACT]),
        Err(NotTrashed::Call(AccountCallError::NotSignedIn))
    );
    assert_eq!(trashes(&account.fake), before, "nothing is asked without a session");

    let Account { fake, .. } = account;
    drop(fake);
    assert_eq!(
        engine.trash_items_for(now, &[CONTRACT]),
        Err(NotTrashed::Call(AccountCallError::Offline))
    );
}
