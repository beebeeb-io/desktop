//! The account's settings calls of the engine handle (`app::settings_api`) against the fake Beebeeb API on loopback:
//! each method answered, refused with a server error, refused as a session that ended, and unreachable; a handle
//! with no session asks nothing. A failure comes back as its kind with fixed words, never the server's body. Every
//! account, name and key here is invented; nothing touches the OS credential store or another host.
//!
//! The TEST identity is refused on macOS (`EngineIdentity::refusal_on`), so the 4 tests that open an Engine
//! are gated with `#[cfg(not(target_os = "macos"))]`; the platform-neutral test runs everywhere.
#![cfg_attr(target_os = "macos", allow(unused_imports, dead_code))]

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use beebeeb_engine::account_dto::NotificationPreferencesUpdate;
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

const PASSWORD: &str = "an invented password for settings";
/// What a refusing server puts in its body: it must never come back in an error.
const BODY_MARKER: &str = "server-body-must-not-leak";

/// How the fake answers the settings routes.
const ANSWER: u8 = 0;
const SERVER_ERROR: u8 = 1;
const SESSION_ENDED: u8 = 2;

/// One account on a fake of its own, with the settings routes added: the account's handler answers the rest.
struct Account {
    fake: FakeApi,
    mode: Arc<AtomicU8>,
    /// The bodies of the writes the fake received, in order.
    writes: Arc<Mutex<Vec<(String, Value)>>>,
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
    let (account, phrase) = FakeAccount::register(email, PASSWORD, TEST_PHRASE);
    let account = Arc::new(account);
    let token = account.session_token.clone();
    let rest = account_handler(account);
    let mode = Arc::new(AtomicU8::new(ANSWER));
    let writes = Arc::new(Mutex::new(Vec::new()));
    let (mode_in, writes_in) = (mode.clone(), writes.clone());
    let prefs = Arc::new(Mutex::new(json!({
        "file_updated": true, "share_received": true, "storage_warning": false,
        "new_device_login": true, "backup_complete": false,
    })));
    let fake = FakeApi::start(Arc::new(move |req: &Request| {
        let route = req.route();
        let ours = matches!(
            route.as_str(),
            "GET /api/v1/me/region"
                | "PUT /api/v1/me/region"
                | "GET /api/v1/notifications"
                | "GET /api/v1/notifications/preferences"
                | "PUT /api/v1/notifications/preferences"
        ) || route.starts_with("PATCH /api/v1/notifications/");
        if !ours {
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
        match route.as_str() {
            "GET /api/v1/me/region" => Reply::ok(json!({
                "preferred_region": "de",
                "region_strict": true,
                "regions": [],
                "choices": [
                    { "id": "europe", "kind": "region", "name": "Europe", "status": "live", "selectable": true, "is_default": true },
                    { "id": "de", "kind": "country", "name": "Germany", "country_code": "DE", "status": "live", "selectable": true, "is_default": false, "city": "an-invented-city" },
                    { "id": "nl", "kind": "country", "name": "Netherlands", "country_code": "NL", "status": "coming_soon", "selectable": false, "is_default": false },
                ],
            })),
            "PUT /api/v1/me/region" => {
                let body = req.json();
                writes_in.lock().unwrap().push((route.clone(), body.clone()));
                Reply::ok(json!({
                    "preferred_region": body["preferred_region"],
                    "region_strict": body["region_strict"].as_bool().unwrap_or(false),
                }))
            }
            "GET /api/v1/notifications" => Reply::ok(json!({
                "notifications": [
                    { "id": "n-2", "type": "share_received", "title": "An invented share", "read": false, "created_at": "2026-11-12T08:12:00Z" },
                    { "id": "n-1", "type": "new_device_login", "title": "An invented sign-in", "read": true, "created_at": "2026-11-11T16:05:00Z" },
                ],
                "unread_count": 1,
            })),
            "GET /api/v1/notifications/preferences" => {
                Reply::ok(json!({ "preferences": prefs.lock().unwrap().clone() }))
            }
            // One notification seen; `n-gone` is one the server no longer has.
            seen if seen.starts_with("PATCH ") => {
                writes_in.lock().unwrap().push((route.clone(), req.json()));
                if seen.ends_with("/n-gone") {
                    Reply::json(404, json!({ "error": BODY_MARKER }))
                } else {
                    Reply::ok(json!({ "message": "updated" }))
                }
            }
            _ => {
                let body = req.json();
                writes_in.lock().unwrap().push((route.clone(), body.clone()));
                let mut now = prefs.lock().unwrap();
                if let (Some(now), Some(changes)) = (now.as_object_mut(), body.as_object()) {
                    for (key, value) in changes {
                        now.insert(key.clone(), value.clone());
                    }
                }
                Reply::ok(json!({ "preferences": now.clone() }))
            }
        }
    }));
    assert_loopback(&fake.base);
    Account {
        fake,
        mode,
        writes,
        email: email.to_string(),
        phrase,
    }
}

/// A handle of its own on `base`, signed out.
fn engine_at(base: &str) -> Engine<MemoryCredentials> {
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "settings-api-slot".into(),
        state_dir: process_state_dir(),
        api_base: base.to_string(),
    };
    let engine = Engine::open(&TEST, config, host, MemoryCredentials::default()).expect("the engine opens");
    engine.restore();
    engine
}

/// A handle signed in and unlocked on `account`'s fake: the session is in memory.
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

/// The routes this file adds, as the fake received them.
fn asked(fake: &FakeApi) -> Vec<String> {
    fake.requests()
        .iter()
        .map(Request::route)
        .filter(|route| route.contains("/me/region") || route.contains("/notifications"))
        .collect()
}

/// A server error and a session that ended, for one method of a signed-in handle: each its own kind, and the
/// error's words never carry the server's body. The fake answers again afterwards.
fn refusals<T: std::fmt::Debug>(
    name: &str,
    account: &Account,
    engine: &Engine<MemoryCredentials>,
    call: impl Fn(&Engine<MemoryCredentials>) -> Result<T, AccountCallError>,
) {
    account.mode.store(SERVER_ERROR, Ordering::SeqCst);
    let error = call(engine).expect_err("a server error is an error");
    assert_eq!(error, AccountCallError::ServerError, "{name}: a 500");
    assert!(
        !format!("{error} {error:?}").contains(BODY_MARKER),
        "{name}: no body in the error"
    );

    account.mode.store(SESSION_ENDED, Ordering::SeqCst);
    let error = call(engine).expect_err("a refused session is an error");
    assert_eq!(error, AccountCallError::SessionEnded, "{name}: a 401");
    assert!(
        !format!("{error} {error:?}").contains(BODY_MARKER),
        "{name}: no body in the error"
    );
    account.mode.store(ANSWER, Ordering::SeqCst);
}

/// Signed out on the same fake: nothing is asked, and the kind says so.
fn not_signed_in<T: std::fmt::Debug>(
    name: &str,
    account: &Account,
    call: impl Fn(&Engine<MemoryCredentials>) -> Result<T, AccountCallError>,
) {
    let engine = engine_at(&account.fake.base);
    let before = asked(&account.fake).len();
    assert_eq!(
        call(&engine).expect_err("signed out"),
        AccountCallError::NotSignedIn,
        "{name}"
    );
    assert_eq!(
        asked(&account.fake).len(),
        before,
        "{name}: nothing is asked without a session"
    );
}

/// Beebeeb gone: the fake stops listening, and every call after that is offline.
fn gone(account: Account) {
    let Account { fake, .. } = account;
    drop(fake);
}

#[cfg(not(target_os = "macos"))]
#[test]
fn the_storage_location_is_read_by_country_and_refusals_are_kinds() {
    let account = account("region-read@example.eu");
    let engine = signed_in(&account);
    let region = engine.storage_region().expect("the fake answers");
    assert_eq!(region.preferred_region.as_deref(), Some("de"));
    assert!(region.region_strict);
    let names: Vec<_> = region
        .choices
        .iter()
        .map(|c| (c.id.as_str(), c.status.as_str()))
        .collect();
    assert_eq!(names, [("europe", "live"), ("de", "live"), ("nl", "coming_soon")]);
    let read_back = serde_json::to_string(&region).unwrap();
    assert!(
        !read_back.contains("an-invented-city"),
        "a city the server sends is never read"
    );
    assert_eq!(asked(&account.fake), ["GET /api/v1/me/region"]);

    refusals("storage_region", &account, &engine, |engine| engine.storage_region());
    not_signed_in("storage_region", &account, |engine| engine.storage_region());
    gone(account);
    assert_eq!(
        engine.storage_region().expect_err("no answer"),
        AccountCallError::Offline
    );
}

#[cfg(not(target_os = "macos"))]
#[test]
fn a_new_storage_location_is_sent_with_its_strict_rule() {
    let account = account("region-write@example.eu");
    let engine = signed_in(&account);
    let stored = engine
        .set_storage_region(Some("de"), Some(true))
        .expect("the fake stores it");
    assert_eq!(
        (stored.preferred_region.as_deref(), stored.region_strict),
        (Some("de"), true)
    );
    engine
        .set_storage_region(Some("europe"), Some(false))
        .expect("the fake stores it");
    let writes = account.writes.lock().unwrap().clone();
    assert_eq!(
        writes,
        [
            (
                "PUT /api/v1/me/region".to_string(),
                json!({ "preferred_region": "de", "region_strict": true })
            ),
            (
                "PUT /api/v1/me/region".to_string(),
                json!({ "preferred_region": "europe", "region_strict": false })
            ),
        ]
    );

    let set = |engine: &Engine<MemoryCredentials>| engine.set_storage_region(Some("fi"), Some(false));
    refusals("set_storage_region", &account, &engine, set);
    not_signed_in("set_storage_region", &account, set);
    gone(account);
    assert_eq!(set(&engine).expect_err("no answer"), AccountCallError::Offline);
}

/// A choice of where files are stored is sent only under the session it was made in: after a lock and an unlock
/// (the same account, its token still good), after a sign-out and a sign-in, and while a sign-out is ending the
/// session, nothing is sent; under the session that is still there it is sent once.
#[cfg(not(target_os = "macos"))]
#[test]
fn a_storage_location_chosen_under_one_session_is_never_sent_under_another() {
    let account = account("region-turn@example.eu");
    let engine = Arc::new(signed_in(&account));
    let puts = || account.writes.lock().unwrap().len();

    let chosen_under = engine.session_turn();
    let _ = engine.lock();
    engine
        .unlock_with_phrase(Zeroizing::new(account.phrase.clone()))
        .expect("the phrase unlocks");
    assert_eq!(
        engine
            .set_storage_region_for(chosen_under, Some("de"), Some(true))
            .expect_err("the session moved on"),
        AccountCallError::NotSignedIn,
        "a lock and an unlock in between"
    );
    assert_eq!(puts(), 0, "nothing was sent after the unlock");

    let chosen_under = engine.session_turn();
    engine.sign_out_keeping_local_data().expect("signed out");
    engine
        .sign_in_password(&account.email, Zeroizing::new(PASSWORD.into()))
        .expect("the fake accepts the password");
    engine
        .unlock_with_phrase(Zeroizing::new(account.phrase.clone()))
        .expect("the phrase unlocks");
    assert_eq!(
        engine
            .set_storage_region_for(chosen_under, Some("de"), Some(true))
            .expect_err("the session moved on"),
        AccountCallError::NotSignedIn,
        "a sign-out and a sign-in in between"
    );
    assert_eq!(puts(), 0, "nothing was sent after the sign-in");

    // Under the session that is still there: sent once, with what was chosen.
    let now = engine.session_turn();
    let stored = engine
        .set_storage_region_for(now, Some("de"), Some(true))
        .expect("the session is the one of the choice");
    assert_eq!(
        (stored.preferred_region.as_deref(), stored.region_strict),
        (Some("de"), true)
    );
    assert_eq!(
        account.writes.lock().unwrap().clone(),
        [(
            "PUT /api/v1/me/region".to_string(),
            json!({ "preferred_region": "de", "region_strict": true })
        )]
    );

    // While a sign-out is ending the session (it is not yet gone from memory): nothing is sent.
    let (during, said) = (engine.clone(), Arc::new(Mutex::new(None)));
    let heard = said.clone();
    engine.during_next_forget_for_test(Box::new(move || {
        *heard.lock().unwrap() = Some(during.set_storage_region_for(now, Some("europe"), Some(false)));
    }));
    engine.sign_out_keeping_local_data().expect("signed out");
    assert_eq!(
        said.lock().unwrap().take().expect("the hook ran").expect_err("ending"),
        AccountCallError::NotSignedIn,
        "a session that is ending"
    );
    assert_eq!(puts(), 1, "and nothing more was sent");
}

/// The session-bound change against the fake on every other answer: a server error and a refused session each
/// their own kind (never the body), a handle with no session asks nothing, and Beebeeb gone is offline.
#[cfg(not(target_os = "macos"))]
#[test]
fn a_session_bound_storage_location_change_fails_by_kind() {
    let account = account("region-turn-kinds@example.eu");
    let engine = signed_in(&account);
    let set = |engine: &Engine<MemoryCredentials>| {
        engine.set_storage_region_for(engine.session_turn(), Some("fi"), Some(false))
    };
    refusals("set_storage_region_for", &account, &engine, set);
    not_signed_in("set_storage_region_for", &account, set);
    assert!(account.writes.lock().unwrap().is_empty(), "none of them was stored");
    gone(account);
    assert_eq!(set(&engine).expect_err("no answer"), AccountCallError::Offline);
}

#[cfg(not(target_os = "macos"))]
#[test]
fn the_accounts_notifications_are_read_with_the_unread_count() {
    let account = account("notifications@example.eu");
    let engine = signed_in(&account);
    let list = engine.notifications().expect("the fake answers");
    assert_eq!(list.unread_count, 1);
    let ids: Vec<_> = list.notifications.iter().map(|n| (n.id.as_str(), n.read)).collect();
    assert_eq!(ids, [("n-2", false), ("n-1", true)]);

    refusals("notifications", &account, &engine, |engine| engine.notifications());
    not_signed_in("notifications", &account, |engine| engine.notifications());
    gone(account);
    assert_eq!(
        engine.notifications().expect_err("no answer"),
        AccountCallError::Offline
    );
}

#[cfg(not(target_os = "macos"))]
#[test]
fn the_accounts_alert_switches_are_read_and_one_change_is_sent_alone() {
    let account = account("preferences@example.eu");
    let engine = signed_in(&account);
    let now = engine.notification_preferences().expect("the fake answers");
    assert!(now.preferences.file_updated && !now.preferences.storage_warning);

    let update = NotificationPreferencesUpdate {
        storage_warning: Some(true),
        ..Default::default()
    };
    let after = engine
        .set_notification_preferences(&update)
        .expect("the fake stores it");
    assert!(
        after.preferences.storage_warning,
        "the answer is the server's set after the change"
    );
    assert!(after.preferences.file_updated, "a switch left out keeps its value");
    assert_eq!(
        account.writes.lock().unwrap().clone(),
        [(
            "PUT /api/v1/notifications/preferences".to_string(),
            json!({ "storage_warning": true })
        )],
        "only the switch that changed is sent"
    );

    let read = |engine: &Engine<MemoryCredentials>| engine.notification_preferences();
    let write = |engine: &Engine<MemoryCredentials>| engine.set_notification_preferences(&update);
    refusals("notification_preferences", &account, &engine, read);
    refusals("set_notification_preferences", &account, &engine, write);
    not_signed_in("notification_preferences", &account, read);
    not_signed_in("set_notification_preferences", &account, write);
    gone(account);
    assert_eq!(read(&engine).expect_err("no answer"), AccountCallError::Offline);
    assert_eq!(write(&engine).expect_err("no answer"), AccountCallError::Offline);
}

/// The two notification writes a window sends, bound to the session of their press: sent once under that session;
/// after a lock and an unlock (the same account, its token still good) nothing is sent; and while a sign-out is
/// ENDING the session (its number has moved, the session and its token are still in memory) nothing is sent
/// either, even for a caller that names the number it has now: the question "is it this session" and the copy of
/// its token are one step that no session change can be half-way through.
#[cfg(not(target_os = "macos"))]
#[test]
fn a_notification_write_is_sent_only_under_its_own_session_and_never_while_it_is_ending() {
    let account = account("alerts-turn@example.eu");
    let engine = Arc::new(signed_in(&account));
    let sent = || account.writes.lock().unwrap().clone();
    let update = NotificationPreferencesUpdate {
        storage_warning: Some(true),
        ..Default::default()
    };
    let ids = ["n-2".to_string()];

    let pressed_under = engine.session_turn();
    let _ = engine.lock();
    engine
        .unlock_with_phrase(Zeroizing::new(account.phrase.clone()))
        .expect("the phrase unlocks");
    assert_eq!(
        engine
            .set_notification_preferences_for(pressed_under, &update)
            .expect_err("the session moved on"),
        AccountCallError::NotSignedIn
    );
    assert_eq!(
        engine.mark_notifications_read_for(pressed_under, &ids),
        Err(AccountCallError::NotSignedIn)
    );
    assert!(sent().is_empty(), "nothing was sent after the unlock");

    // Under the session that is still there: each sent once, as pressed.
    let now = engine.session_turn();
    let after = engine
        .set_notification_preferences_for(now, &update)
        .expect("the session is the one of the press");
    assert!(after.preferences.storage_warning);
    engine
        .mark_notifications_read_for(now, &ids)
        .expect("the session is the one of the page");
    assert_eq!(
        sent(),
        [
            (
                "PUT /api/v1/notifications/preferences".to_string(),
                json!({ "storage_warning": true })
            ),
            ("PATCH /api/v1/notifications/n-2".to_string(), json!({ "read": true })),
        ]
    );

    // While a sign-out is ending the session: asked from inside it, under the number the session still has.
    let (during, said) = (engine.clone(), Arc::new(Mutex::new(None)));
    let (heard, change, marks) = (said.clone(), update.clone(), ids.clone());
    engine.during_next_forget_for_test(Box::new(move || {
        let still = during.session_turn();
        let switched = during.set_notification_preferences_for(still, &change).map(|_| ());
        let marked = during.mark_notifications_read_for(still, &marks);
        *heard.lock().unwrap() = Some((still, switched, marked));
    }));
    engine.sign_out_keeping_local_data().expect("signed out");
    let (still, switched, marked) = said.lock().unwrap().take().expect("the hook ran");
    assert_ne!(
        still, now,
        "the session's number had moved, and both writes named the number it has NOW, with the session still in \
         memory: only the session-write lock can have refused them"
    );
    assert_eq!(switched, Err(AccountCallError::NotSignedIn), "a session that is ending");
    assert_eq!(marked, Err(AccountCallError::NotSignedIn), "a session that is ending");
    assert_eq!(sent().len(), 2, "and nothing more was sent");
}

/// The session-bound notification writes against the fake on every other answer: each kind its own, never the body.
#[cfg(not(target_os = "macos"))]
#[test]
fn a_session_bound_notification_write_fails_by_kind() {
    let account = account("alerts-turn-kinds@example.eu");
    let engine = signed_in(&account);
    let update = NotificationPreferencesUpdate {
        file_updated: Some(false),
        ..Default::default()
    };
    let ids = ["n-2".to_string()];
    let write =
        |engine: &Engine<MemoryCredentials>| engine.set_notification_preferences_for(engine.session_turn(), &update);
    let mark = |engine: &Engine<MemoryCredentials>| engine.mark_notifications_read_for(engine.session_turn(), &ids);
    refusals("set_notification_preferences_for", &account, &engine, write);
    refusals("mark_notifications_read_for", &account, &engine, mark);
    not_signed_in("set_notification_preferences_for", &account, write);
    not_signed_in("mark_notifications_read_for", &account, mark);
    assert!(account.writes.lock().unwrap().is_empty(), "none of them was stored");
    gone(account);
    assert_eq!(write(&engine).expect_err("no answer"), AccountCallError::Offline);
    assert_eq!(mark(&engine).expect_err("no answer"), AccountCallError::Offline);
}

#[cfg(not(target_os = "macos"))]
#[test]
fn notifications_are_marked_seen_one_by_one_and_one_that_is_gone_counts_as_seen() {
    let account = account("seen@example.eu");
    let engine = signed_in(&account);
    let ids = ["n-2".to_string(), "n-gone".to_string(), "n-1".to_string()];
    engine.mark_notifications_read(&ids).expect("the fake marks them");
    assert_eq!(
        account.writes.lock().unwrap().clone(),
        [
            ("PATCH /api/v1/notifications/n-2".to_string(), json!({ "read": true })),
            (
                "PATCH /api/v1/notifications/n-gone".to_string(),
                json!({ "read": true })
            ),
            ("PATCH /api/v1/notifications/n-1".to_string(), json!({ "read": true })),
        ],
        "each id once, in order, as read"
    );
    engine
        .mark_notifications_read(&[])
        .expect("nothing to mark asks nothing");
    assert_eq!(account.writes.lock().unwrap().len(), 3);

    let mark = |engine: &Engine<MemoryCredentials>| engine.mark_notifications_read(&ids);
    refusals("mark_notifications_read", &account, &engine, mark);
    not_signed_in("mark_notifications_read", &account, mark);
    gone(account);
    assert_eq!(mark(&engine).expect_err("no answer"), AccountCallError::Offline);
}

#[test]
fn each_kind_has_its_own_words() {
    let all = [
        AccountCallError::NotSignedIn,
        AccountCallError::Offline,
        AccountCallError::SessionEnded,
        AccountCallError::ServerError,
        AccountCallError::Unreadable,
    ];
    let words: std::collections::BTreeSet<_> = all.iter().map(|kind| kind.to_string()).collect();
    assert_eq!(words.len(), all.len(), "{words:?}");
}
