//! Settings on the real engine (`EngineSettingsSource`), against the engine's fake account on loopback with the
//! storage-location routes added here: what General and Storage location draw, a change sent once and shown only
//! after Beebeeb took it, every way a read can fail, a slow reply, the theme across a restart, the window's poll,
//! and no sample value in the running app's Settings. An engine handle is on the process's state folder (the
//! engine refuses another), which nothing here writes in. Every name is invented.

use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use beebeeb_engine::app::{Engine, EngineConfig};
use beebeeb_engine::host::ChannelHost;
use beebeeb_engine::identity::TEST;
use beebeeb_engine::signin::memory_store::MemoryCredentials;
use beebeeb_engine::test_api::fake_account::{FakeAccount, TEST_PHRASE, account_handler};
use beebeeb_engine::test_api::{FakeApi, Reply, Request};
use rune::{Env, Runtime, Tx};
use serde_json::{Value, json};
use zeroize::Zeroizing;

use super::*;
use crate::State;
use crate::shell::app::{Event, SettingsPane};
use crate::shell::host;

const PASSWORD: &str = "an invented password for settings";
const CITY: &str = "an-invented-city";

const ANSWER: u8 = 0;
const SERVER_ERROR: u8 = 1;
const SESSION_ENDED: u8 = 2;
const EMPTY: u8 = 3;
/// A 2xx with a body that does not parse as the expected shape.
const UNREADABLE: u8 = 4;
/// Close the connection before answering (timeout / transport error).
const DROP_CONNECTION: u8 = 5;

/// One account on a fake of its own, with the storage-location routes: the account's handler answers the rest.
struct Account {
    fake: FakeApi,
    /// How the GET route answers (ANSWER, SERVER_ERROR, SESSION_ENDED, EMPTY, UNREADABLE, DROP_CONNECTION).
    get_mode: Arc<AtomicU8>,
    /// How the PUT route answers (same values). Separate from `get_mode` so a write can fail while the
    /// re-read it starts succeeds, with no timing dependency.
    put_mode: Arc<AtomicU8>,
    /// How long the region read waits before it answers (with what the server held when it was asked), in ms.
    delay_ms: Arc<AtomicU64>,
    /// How long a change waits before the server stores it, in ms.
    put_delay_ms: Arc<AtomicU64>,
    writes: Arc<Mutex<Vec<Value>>>,
    /// Every change that reached the fake, counted BEFORE any check of its token: which account's token it
    /// carried (0 the first, 1 the second, 2 neither) and its body.
    puts: Arc<Mutex<Vec<(usize, Value)>>>,
    /// The next change is stored and then waits here before it is answered.
    put_hold: Arc<Mutex<Option<mpsc::Receiver<()>>>>,
    /// The next read waits here before it is answered (with what the fake holds when it is let go).
    get_hold: Arc<Mutex<Option<mpsc::Receiver<()>>>>,
    /// How many reads reached the fake.
    gets: Arc<AtomicU64>,
    email: String,
    phrase: String,
    /// Another account on the same fake (its storage location: Finland, not strict).
    second: Arc<FakeAccount>,
    second_phrase: String,
}

impl Account {
    /// The next change is stored by the fake and then waits, unanswered, until the returned sender is sent to.
    fn hold_next_put(&self) -> mpsc::Sender<()> {
        let (release, held) = mpsc::channel();
        *self.put_hold.lock().unwrap() = Some(held);
        release
    }

    /// The next read waits at the fake, unanswered, until the returned sender is sent to.
    fn hold_next_get(&self) -> mpsc::Sender<()> {
        let (release, held) = mpsc::channel();
        *self.get_hold.lock().unwrap() = Some(held);
        release
    }

    fn gets(&self) -> u64 {
        self.gets.load(Ordering::SeqCst)
    }

    /// How many changes reached the fake, under any token, and whose token each carried.
    fn puts(&self) -> Vec<usize> {
        self.puts.lock().unwrap().iter().map(|(who, _)| *who).collect()
    }
}

fn names_email(req: &Request, email: &str) -> bool {
    req.json()
        .get("email")
        .and_then(Value::as_str)
        .is_some_and(|sent| sent.eq_ignore_ascii_case(email))
}

fn sign_in(engine: &Engine<MemoryCredentials>, email: &str, phrase: &str) {
    engine
        .sign_in_password(email, Zeroizing::new(PASSWORD.into()))
        .expect("the fake accepts the password");
    engine
        .unlock_with_phrase(Zeroizing::new(phrase.to_string()))
        .expect("the phrase unlocks");
}

fn account(email: &str) -> Account {
    let (account, phrase) = FakeAccount::register(email, PASSWORD, TEST_PHRASE);
    let account = Arc::new(account);
    let token = account.session_token.clone();
    let rest = account_handler(account);
    let (second, second_phrase) = FakeAccount::register(&format!("second-{email}"), PASSWORD, TEST_PHRASE);
    let second = Arc::new(second);
    let (second_email, second_token) = (second.email.clone(), second.session_token.clone());
    let rest_of_second = account_handler(second.clone());
    let (puts, put_hold) = (Arc::new(Mutex::new(Vec::new())), Arc::new(Mutex::new(None)));
    let (puts_in, put_hold_in): (Arc<Mutex<Vec<(usize, Value)>>>, Arc<Mutex<Option<mpsc::Receiver<()>>>>) =
        (puts.clone(), put_hold.clone());
    let (get_hold, gets) = (Arc::new(Mutex::new(None)), Arc::new(AtomicU64::new(0)));
    let (get_hold_in, gets_in): (Arc<Mutex<Option<mpsc::Receiver<()>>>>, Arc<AtomicU64>) =
        (get_hold.clone(), gets.clone());
    let (get_mode, put_mode, delay_ms, put_delay_ms, writes) = (
        Arc::new(AtomicU8::new(ANSWER)),
        Arc::new(AtomicU8::new(ANSWER)),
        Arc::new(AtomicU64::new(0)),
        Arc::new(AtomicU64::new(0)),
        Arc::new(Mutex::new(Vec::new())),
    );
    let (get_mode_in, put_mode_in, delay_in, put_delay_in, writes_in) = (
        get_mode.clone(),
        put_mode.clone(),
        delay_ms.clone(),
        put_delay_ms.clone(),
        writes.clone(),
    );
    let stored = Arc::new(Mutex::new([
        json!({ "preferred_region": "de", "region_strict": true }),
        json!({ "preferred_region": "fi", "region_strict": false }),
    ]));
    let fake = FakeApi::start(Arc::new(move |req: &Request| {
        let route = req.route();
        if !route.ends_with(" /api/v1/me/region") {
            let theirs = if route.starts_with("POST /api/v1/opaque/") {
                names_email(req, &second_email)
            } else {
                req.bearer() == Some(second_token.as_str())
            };
            return if theirs { rest_of_second(req) } else { rest(req) };
        }
        let is_put = route.starts_with("PUT");
        let who = match req.bearer() {
            Some(bearer) if bearer == token.as_str() => 0,
            Some(bearer) if bearer == second_token.as_str() => 1,
            _ => 2,
        };
        if is_put {
            puts_in.lock().unwrap().push((who, req.json()));
        } else {
            gets_in.fetch_add(1, Ordering::SeqCst);
            let hold = get_hold_in.lock().unwrap().take();
            if let Some(hold) = hold {
                let _ = hold.recv();
            }
        }
        let mode = if is_put {
            put_mode_in.load(Ordering::SeqCst)
        } else {
            get_mode_in.load(Ordering::SeqCst)
        };
        match mode {
            SERVER_ERROR => return Reply::json(500, json!({ "error": "a server body" })),
            SESSION_ENDED => return Reply::json(401, json!({ "error": "a server body" })),
            UNREADABLE => return Reply::ok(json!("not the expected shape")),
            DROP_CONNECTION => return Reply::empty(0),
            _ => {}
        }
        if who > 1 {
            return Reply::json(401, json!({}));
        }
        if is_put {
            std::thread::sleep(Duration::from_millis(put_delay_in.load(Ordering::SeqCst)));
            let body = req.json();
            writes_in.lock().unwrap().push(body.clone());
            let now = json!({
                "preferred_region": body["preferred_region"],
                "region_strict": body["region_strict"].as_bool().unwrap_or(false),
            });
            stored.lock().unwrap()[who] = now.clone();
            // Stored; the answer waits when a test holds it.
            let hold = put_hold_in.lock().unwrap().take();
            if let Some(hold) = hold {
                let _ = hold.recv();
                // A held answer follows the mode of the moment it is let go: stored, then not readable.
                if put_mode_in.load(Ordering::SeqCst) == UNREADABLE {
                    return Reply::ok(json!("not the expected shape"));
                }
            }
            return Reply::ok(now);
        }
        let choices = if get_mode_in.load(Ordering::SeqCst) == EMPTY {
            json!([])
        } else {
            json!([
                { "id": "europe", "kind": "region", "name": "Europe", "status": "live", "selectable": true, "is_default": true },
                { "id": "de", "kind": "country", "name": "Germany", "status": "live", "selectable": true, "is_default": false, "city": CITY },
                { "id": "fi", "kind": "country", "name": "Finland", "status": "live", "selectable": true, "is_default": false },
                { "id": "nl", "kind": "country", "name": "Netherlands", "status": "coming_soon", "selectable": false, "is_default": false },
                { "id": "dc-1", "kind": "datacenter", "name": CITY, "status": "live", "selectable": true, "is_default": false },
            ])
        };
        let now = stored.lock().unwrap()[who].clone();
        std::thread::sleep(Duration::from_millis(delay_in.load(Ordering::SeqCst)));
        Reply::ok(json!({
            "preferred_region": now["preferred_region"],
            "region_strict": now["region_strict"],
            "regions": [],
            "choices": choices,
        }))
    }));
    Account {
        fake,
        get_mode,
        put_mode,
        delay_ms,
        put_delay_ms,
        writes,
        puts,
        put_hold,
        get_hold,
        gets,
        email: email.to_string(),
        phrase,
        second,
        second_phrase,
    }
}

/// A handle of its own on `base`, signed out.
fn engine_at(base: &str) -> Arc<Engine<MemoryCredentials>> {
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "settings-test-slot".into(),
        state_dir: crate::engine_host::tests::process_state_dir(),
        api_base: base.to_string(),
    };
    let engine = Engine::open(&TEST, config, host, MemoryCredentials::default()).expect("the engine opens");
    engine.restore();
    Arc::new(engine)
}

fn signed_in(account: &Account) -> Arc<Engine<MemoryCredentials>> {
    let engine = engine_at(&account.fake.base);
    engine
        .sign_in_password(&account.email, Zeroizing::new(PASSWORD.into()))
        .expect("the fake accepts the password");
    engine
        .unlock_with_phrase(Zeroizing::new(account.phrase.clone()))
        .expect("the phrase unlocks");
    engine
}

/// Pump until nothing is out any more (at most 10 s); every toast on the way, in order.
fn settle(source: &mut EngineSettingsSource<MemoryCredentials>) -> Vec<&'static str> {
    let began = Instant::now();
    let mut toasts = Vec::new();
    loop {
        let pumped = source.pump();
        toasts.extend(pumped.toast);
        if !pumped.busy {
            return toasts;
        }
        assert!(began.elapsed() < Duration::from_secs(10), "an answer did not come back");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn names(locations: &Locations) -> Vec<(&str, bool)> {
    locations.choices.iter().map(|c| (c.name.as_str(), c.live)).collect()
}

#[test]
fn general_and_storage_location_draw_what_beebeeb_says_by_country() {
    let account = account("settings-read@example.eu");
    let mut source = EngineSettingsSource::new(signed_in(&account));
    settle(&mut source);
    let locations = source.locations().expect("the list loaded");
    assert_eq!(
        names(&locations),
        [
            ("Default", true),
            ("Germany", true),
            ("Finland", true),
            ("Netherlands", false)
        ],
        "by country; the Netherlands is coming soon; an older server's data-center row is left out"
    );
    assert_eq!(
        (locations.chosen, locations.only_here, locations.unavailable),
        (1, true, false)
    );
    assert!(!format!("{locations:?}").contains(CITY), "never a city");
    let general = source.general();
    assert_eq!(general.storage_location, "Germany");
    assert_eq!(general.version, version_words());
    assert_eq!(
        general.update,
        Update::Current,
        "no updater: never says a newer version exists"
    );
    assert!(source.wired(Pane::General) && source.wired(Pane::StorageLocation));
    assert!(!source.wired(Pane::About));
    assert!(source.wired(Pane::Notifications) && source.wired(Pane::Recent));
    assert!(
        !source.opens("DW-0782"),
        "no support bundle behind Report a problem yet"
    );
    assert!(!source.choose_location(3), "a place that is not live is never chosen");
    assert!(account.writes.lock().unwrap().is_empty());
}

#[test]
fn a_choice_is_sent_once_and_shown_only_after_beebeeb_took_it() {
    let account = account("settings-choose@example.eu");
    let mut source = EngineSettingsSource::new(signed_in(&account));
    settle(&mut source);

    assert!(!source.choose_location(2), "nothing is taken when the call returns");
    assert!(!source.choose_location(0), "a second change waits for the first");
    assert_eq!(
        source.locations().unwrap().chosen,
        1,
        "still the old choice before the answer"
    );
    assert_eq!(
        settle(&mut source),
        [STILL_SAVING, CHOSEN],
        "the waiting change is said, not dropped"
    );
    let now = source.locations().unwrap();
    assert_eq!((now.chosen, now.only_here), (2, true), "Finland, still only there");
    assert_eq!(source.general().storage_location, "Finland");
    assert_eq!(
        account.writes.lock().unwrap().clone(),
        [json!({ "preferred_region": "fi", "region_strict": true })],
        "sent once, with the strict rule it had"
    );

    // Refused: the old choice stands, and the toast says nothing changed.
    account.put_mode.store(SERVER_ERROR, Ordering::SeqCst);
    source.choose_location(1);
    assert_eq!(settle(&mut source), [NOT_CHOSEN]);
    assert_eq!(source.locations().unwrap().chosen, 2);
    source.set_only_here(false);
    assert_eq!(settle(&mut source), [NOT_CHOSEN]);
    assert!(source.locations().unwrap().only_here, "the switch is put back");

    account.put_mode.store(ANSWER, Ordering::SeqCst);
    source.set_only_here(false);
    assert!(settle(&mut source).len() == 1 && !source.locations().unwrap().only_here);
    // The default never keeps "only here".
    source.choose_location(0);
    settle(&mut source);
    assert_eq!(
        account.writes.lock().unwrap().last(),
        Some(&json!({ "preferred_region": "europe", "region_strict": false }))
    );
    assert_eq!(source.general().storage_location, "Europe");
}

#[test]
fn a_read_that_fails_or_answers_nothing_says_it_could_not_load() {
    for (mode, why) in [
        (SERVER_ERROR, "a server error"),
        (SESSION_ENDED, "a session that ended"),
        (EMPTY, "no place"),
    ] {
        let account = account(&format!("settings-fails-{mode}@example.eu"));
        account.get_mode.store(mode, Ordering::SeqCst);
        let mut source = EngineSettingsSource::new(signed_in(&account));
        settle(&mut source);
        assert!(source.locations().is_none(), "{why}: the list could not load");
        assert!(!source.locations_loading(), "{why}: not loading any more");
        assert_eq!(source.general().storage_location, "", "{why}: no guess in General");
        // Try again asks again, and an answer then shows.
        account.get_mode.store(ANSWER, Ordering::SeqCst);
        source.reload_locations();
        settle(&mut source);
        assert!(source.locations().is_some(), "{why}: Try again loads it");
    }
    // Offline: Beebeeb does not answer at all.
    let account = account("settings-offline@example.eu");
    let engine = signed_in(&account);
    let Account { fake, .. } = account;
    drop(fake);
    let mut source = EngineSettingsSource::new(engine);
    settle(&mut source);
    assert!(source.locations().is_none() && source.general().storage_location.is_empty());
    // Signed out: nothing to ask with.
    let mut source = EngineSettingsSource::new(engine_at("http://127.0.0.1:9"));
    settle(&mut source);
    assert!(source.locations().is_none());
}

#[test]
fn a_slow_reply_never_holds_the_window() {
    let account = account("settings-slow@example.eu");
    account.delay_ms.store(1_500, Ordering::SeqCst);
    let engine = signed_in(&account);
    let began = Instant::now();
    let mut source = EngineSettingsSource::new(engine);
    let general = source.general();
    assert!(
        source.locations().is_none() && source.locations_loading(),
        "loading, said as loading"
    );
    assert!(source.pump().busy, "the window looks again");
    assert!(
        began.elapsed() < Duration::from_millis(500),
        "nothing waited: {:?}",
        began.elapsed()
    );
    assert_eq!(general.storage_location, "");
    settle(&mut source);
    assert_eq!(source.general().storage_location, "Germany");
}

/// The session ends while a change is on its way: nothing of the account shows any more, and the late answer
/// changes nothing on screen.
#[test]
fn a_session_that_ends_mid_change_shows_nothing_of_the_account() {
    let account = account("settings-ended@example.eu");
    let engine = signed_in(&account);
    let mut source = EngineSettingsSource::new(engine.clone());
    settle(&mut source);
    assert_eq!(source.general().storage_location, "Germany");
    account.put_delay_ms.store(400, Ordering::SeqCst);
    source.choose_location(2);
    engine.sign_out_keeping_local_data().expect("signs out");
    settle(&mut source);
    assert!(source.locations().is_none(), "no list of an account that signed out");
    assert_eq!(source.general().storage_location, "", "and no place in General");
}

/// Pump until `done` (at most 10 s), whatever is still on its way.
fn pump_until(
    source: &mut EngineSettingsSource<MemoryCredentials>,
    what: &str,
    done: impl Fn(&EngineSettingsSource<MemoryCredentials>) -> bool,
) -> Vec<&'static str> {
    let began = Instant::now();
    let mut toasts = Vec::new();
    loop {
        toasts.extend(source.pump().toast);
        if done(source) {
            return toasts;
        }
        assert!(began.elapsed() < Duration::from_secs(10), "never: {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Task 2494: a choice made under one account is never sent under the next. The press is made; its worker has not
/// called the engine yet when the session changes. Counted at the fake before any check of the token.
#[test]
fn a_choice_made_under_one_account_is_never_sent_under_the_next() {
    let account = account("settings-press-turn@example.eu");
    let engine = signed_in(&account);
    let mut source = EngineSettingsSource::for_identity(engine.clone(), &TEST);
    settle(&mut source);
    assert_eq!(source.locations().unwrap().chosen, 1, "the first account: Germany");

    // Another account signs in between the press and the worker's call.
    let release = source.hold_next_write();
    source.choose_location(0);
    assert!(source.writing, "the press started its worker");
    engine.sign_out_keeping_local_data().expect("signed out");
    sign_in(&engine, &account.second.email, &account.second_phrase);
    release.send(()).expect("the worker is waiting");
    let toasts = settle(&mut source);
    assert_eq!(
        account.puts(),
        [0usize; 0],
        "the first account's choice reached no account"
    );
    assert_eq!(toasts, [""; 0], "nothing is said to the account that did not press");
    source.opened(Pane::StorageLocation);
    settle(&mut source);
    let theirs = source.locations().expect("the second account's own list");
    assert_eq!((theirs.chosen, theirs.only_here), (2, false), "Finland, as it was");

    // The same with a lock and an unlock of the same account in between: not sent, and said.
    let release = source.hold_next_write();
    source.choose_location(1);
    assert!(source.writing, "the press started its worker");
    let _ = engine.lock();
    engine
        .unlock_with_phrase(Zeroizing::new(account.second_phrase.clone()))
        .expect("the phrase unlocks");
    release.send(()).expect("the worker is waiting");
    assert_eq!(
        settle(&mut source),
        [NOT_CHOSEN],
        "the person who pressed is told nothing changed"
    );
    assert_eq!(
        account.puts(),
        [0usize; 0],
        "a press made before the lock is not sent after the unlock"
    );

    // "Only store in this country" goes the same way.
    source.opened(Pane::StorageLocation);
    settle(&mut source);
    let release = source.hold_next_write();
    source.set_only_here(true);
    assert!(source.writing, "the switch started its worker");
    engine.sign_out_keeping_local_data().expect("signed out");
    sign_in(&engine, &account.email, &account.phrase);
    release.send(()).expect("the worker is waiting");
    settle(&mut source);
    assert_eq!(account.puts(), [0usize; 0], "the switch reached no account");

    // A press under the session that is still there is sent once, under that account, held or not.
    sign_in_again_as(&engine, &account.second.email, &account.second_phrase);
    source.opened(Pane::StorageLocation);
    settle(&mut source);
    let release = source.hold_next_write();
    source.choose_location(1);
    assert!(source.writing, "the press started its worker");
    release.send(()).expect("the worker is waiting");
    assert_eq!(settle(&mut source), [CHOSEN]);
    assert_eq!(account.puts(), [1], "once, with the second account's token");
    assert_eq!(source.general().storage_location, "Germany");
}

fn sign_in_again_as(engine: &Engine<MemoryCredentials>, email: &str, phrase: &str) {
    engine.sign_out_keeping_local_data().expect("signed out");
    sign_in(engine, email, phrase);
}

/// Task 2494: a change Beebeeb took, answered after the session moved on, is never drawn under the next session:
/// not on another account's list, and not as "updated" after a lock and an unlock of the same account (the pane
/// then reads what Beebeeb holds).
#[test]
fn a_late_answer_is_never_drawn_under_another_session() {
    let account = account("settings-late-answer@example.eu");
    let engine = signed_in(&account);
    let mut source = EngineSettingsSource::for_identity(engine.clone(), &TEST);
    settle(&mut source);

    // The first account's change is with Beebeeb, unanswered, when the second account signs in.
    let release = account.hold_next_put();
    source.choose_location(0);
    pump_until(&mut source, "the change reaches the fake", |_| account.puts() == [0]);
    engine.sign_out_keeping_local_data().expect("signed out");
    sign_in(&engine, &account.second.email, &account.second_phrase);
    source.opened(Pane::StorageLocation);
    pump_until(&mut source, "the second account's list", |source| {
        source.locations().is_some()
    });
    release.send(()).expect("the fake is waiting");
    let toasts = settle(&mut source);
    assert_eq!(toasts, [""; 0], "the first account's answer says nothing here");
    let theirs = source.locations().expect("the second account's own list");
    assert_eq!((theirs.chosen, theirs.only_here), (2, false), "Finland, as it was");
    assert_eq!(account.puts(), [0], "sent once, under the account that chose");

    // The same account, a lock and an unlock while its change is unanswered.
    let release = account.hold_next_put();
    source.choose_location(1);
    pump_until(&mut source, "the change reaches the fake", |_| account.puts() == [0, 1]);
    let _ = engine.lock();
    engine
        .unlock_with_phrase(Zeroizing::new(account.second_phrase.clone()))
        .expect("the phrase unlocks");
    release.send(()).expect("the fake is waiting");
    let toasts = settle(&mut source);
    assert!(
        !toasts.contains(&CHOSEN),
        "an answer for the session before the lock is not drawn: {toasts:?}"
    );
    assert_eq!(
        source.locations().expect("read afresh").chosen,
        1,
        "the pane shows what Beebeeb holds now"
    );
    assert_eq!(account.puts(), [0, 1]);

    // The same, and the late answer cannot be read: the change may have landed, which is said, and the pane reads
    // what Beebeeb holds (it did land).
    let release = account.hold_next_put();
    source.choose_location(2);
    pump_until(&mut source, "the change reaches the fake", |_| {
        account.puts() == [0, 1, 1]
    });
    let _ = engine.lock();
    engine
        .unlock_with_phrase(Zeroizing::new(account.second_phrase.clone()))
        .expect("the phrase unlocks");
    account.put_mode.store(UNREADABLE, Ordering::SeqCst);
    release.send(()).expect("the fake is waiting");
    assert_eq!(settle(&mut source), [UNKNOWN_CHANGE]);
    account.put_mode.store(ANSWER, Ordering::SeqCst);
    assert_eq!(
        source.locations().expect("read afresh").chosen,
        2,
        "Finland: it had landed"
    );

    // The first account's late answer lands while the second account's first read is still out: that read is
    // the second account's own and is taken.
    sign_in_again_as(&engine, &account.email, &account.phrase);
    source.opened(Pane::StorageLocation);
    settle(&mut source);
    let release = account.hold_next_put();
    source.choose_location(1);
    pump_until(&mut source, "the change reaches the fake", |_| {
        account.puts().len() == 4
    });
    sign_in_again_as(&engine, &account.second.email, &account.second_phrase);
    let (reads, release_read) = (account.gets(), account.hold_next_get());
    source.opened(Pane::StorageLocation);
    pump_until(&mut source, "the second account's read reaches the fake", |_| {
        account.gets() == reads + 1
    });
    release.send(()).expect("the fake is waiting");
    pump_until(&mut source, "the first account's answer is taken in", |source| {
        !source.writing
    });
    release_read.send(()).expect("the fake is waiting");
    assert_eq!(settle(&mut source), [""; 0]);
    assert_eq!(
        source.locations().expect("the second account's list").chosen,
        2,
        "its own read was not dropped for the first account's change"
    );
    assert_eq!(account.gets(), reads + 1, "and was the only one asked");
}

/// Answers that are taken in while the vault is locked: nothing of them is lost by the time it is unlocked. A
/// refusal is said then; a change Beebeeb took, and a read, are asked for afresh.
#[test]
fn answers_taken_in_while_locked_are_made_good_after_the_unlock() {
    let account = account("settings-locked-pump@example.eu");
    let engine = signed_in(&account);
    let mut source = EngineSettingsSource::for_identity(engine.clone(), &TEST);
    settle(&mut source);
    let unlock = || {
        engine
            .unlock_with_phrase(Zeroizing::new(account.phrase.clone()))
            .expect("the phrase unlocks")
    };

    // A press the engine refuses (the vault locked before its worker ran), answered while still locked.
    let release = source.hold_next_write();
    source.choose_location(2);
    assert!(source.writing, "the press started its worker");
    let _ = engine.lock();
    release.send(()).expect("the worker is waiting");
    let toasts = pump_until(&mut source, "the refusal is taken in", |source| !source.writing);
    assert_eq!(toasts, [""; 0], "nothing is said to a locked vault");
    unlock();
    assert_eq!(settle(&mut source), [NOT_CHOSEN], "said once the person is back");
    assert_eq!(account.puts(), [0usize; 0]);
    assert_eq!(source.locations().unwrap().chosen, 1, "Germany, as it was");

    // A change Beebeeb took, answered while locked: after the unlock the pane shows it, never the choice before.
    let release = account.hold_next_put();
    source.choose_location(2);
    pump_until(&mut source, "the change reaches the fake", |_| account.puts() == [0]);
    let _ = engine.lock();
    release.send(()).expect("the fake is waiting");
    pump_until(&mut source, "the answer is taken in", |source| !source.writing);
    unlock();
    let toasts = settle(&mut source);
    assert!(!toasts.contains(&CHOSEN), "{toasts:?}");
    assert_eq!(
        source.locations().expect("read afresh").chosen,
        2,
        "Finland: what Beebeeb holds"
    );

    // A read answered while locked: asked again after the unlock.
    let (reads, release) = (account.gets(), account.hold_next_get());
    source.reload_locations();
    pump_until(&mut source, "the read reaches the fake", |_| {
        account.gets() == reads + 1
    });
    let _ = engine.lock();
    release.send(()).expect("the fake is waiting");
    pump_until(&mut source, "the answer is taken in", |source| !source.reading);
    assert!(source.locations().is_none(), "locked: no list");
    unlock();
    settle(&mut source);
    assert_eq!(account.gets(), reads + 2, "asked once more");
    assert_eq!(source.locations().expect("read afresh").chosen, 2);
}

/// A read asked under one session and answered after a lock and an unlock is not taken as the new session's: the
/// storage location is asked for again, once.
#[test]
fn a_read_answered_after_the_session_moved_on_is_asked_again() {
    let account = account("settings-late-read@example.eu");
    let engine = signed_in(&account);
    let mut source = EngineSettingsSource::for_identity(engine.clone(), &TEST);
    settle(&mut source);
    let before = account.gets();
    let release = account.hold_next_get();
    source.reload_locations();
    pump_until(&mut source, "the read reaches the fake", |_| {
        account.gets() == before + 1
    });
    let _ = engine.lock();
    assert!(source.locations().is_none(), "locked: no list");
    engine
        .unlock_with_phrase(Zeroizing::new(account.phrase.clone()))
        .expect("the phrase unlocks");
    release.send(()).expect("the fake is waiting");
    settle(&mut source);
    assert_eq!(
        account.gets(),
        before + 2,
        "the answer of the session before the lock is not taken: asked once more"
    );
    assert_eq!(source.general().storage_location, "Germany");
}

/// P2-1 (task 2473): "Nothing changed" is said only after an HTTP refusal. A timeout after the request left, a
/// transport error mid-flight, or a 2xx with an unreadable body each say the result is unknown and start a
/// re-read, so the pane shows what the server has.
#[test]
fn a_timeout_or_unreadable_2xx_says_unknown_and_re_reads() {
    // Case 1: an HTTP refusal (500) says "Nothing changed" and does NOT re-read.
    let account = account("settings-p2-1@example.eu");
    let mut source = EngineSettingsSource::new(signed_in(&account));
    settle(&mut source);
    assert_eq!(source.general().storage_location, "Germany");
    account.put_mode.store(SERVER_ERROR, Ordering::SeqCst);
    source.choose_location(2);
    assert_eq!(settle(&mut source), [NOT_CHOSEN], "a 500 says nothing changed");
    assert_eq!(
        source.general().storage_location,
        "Germany",
        "the old choice stands after a 500"
    );
    // No re-read was started: the source is idle.
    assert!(!source.pump().busy, "a 500 does not start a re-read");

    // Case 2: a transport error (offline / timeout) says unknown and re-reads.
    // PUT fails, GET stays ANSWER so the re-read succeeds: no timing dependency.
    account.put_mode.store(DROP_CONNECTION, Ordering::SeqCst);
    source.choose_location(2);
    let toasts = settle(&mut source);
    assert_eq!(toasts.first(), Some(&UNKNOWN_CHANGE), "a transport error says unknown");
    // The re-read succeeded, so the pane shows what the server has.
    assert_eq!(source.general().storage_location, "Germany");
    assert!(!source.pump().busy);

    // Case 3: a 2xx with an unreadable body says unknown and re-reads.
    account.put_mode.store(UNREADABLE, Ordering::SeqCst);
    source.choose_location(2);
    let toasts = settle(&mut source);
    assert_eq!(toasts.first(), Some(&UNKNOWN_CHANGE), "an unreadable 2xx says unknown");
    assert_eq!(source.general().storage_location, "Germany");
    assert!(!source.pump().busy);
}

/// A read answered for one account is not shown after a sign-out: another session on the same engine
/// never sees the first account's storage location.
#[test]
fn a_read_answered_for_one_account_is_not_shown_after_a_sign_out() {
    let account = account("settings-account-a@example.eu");
    let engine = signed_in(&account);
    let mut source = EngineSettingsSource::for_identity(engine.clone(), &TEST);
    settle(&mut source);
    assert_eq!(source.general().storage_location, "Germany");

    // Sign out: the source no longer shows the list or the location.
    engine.sign_out_keeping_local_data().expect("signs out");
    assert!(source.locations().is_none(), "signed out: no list");
    assert_eq!(source.general().storage_location, "");
    // Even pumping again does not bring it back (the data is still in `region` but `region_of` no
    // longer matches the now-absent account).
    source.pump();
    assert!(source.locations().is_none(), "still no list after pump");
}

/// A panic in a worker thread clears the in-flight flag and shows the failed state; the source does not hang.
#[test]
fn a_worker_panic_clears_the_in_flight_flag() {
    // Build a source whose engine is signed in, then inject a panic through a custom handler.
    let account = account("settings-panic@example.eu");
    let engine = signed_in(&account);
    let mut source = EngineSettingsSource::for_identity(engine.clone(), &TEST);
    settle(&mut source);
    assert_eq!(source.general().storage_location, "Germany");
    assert!(!source.reading && !source.writing);

    // Drop the fake so the engine call panics or errors. Either way the flag must clear.
    let Account { fake, .. } = account;
    drop(fake);
    source.read_region();
    assert!(source.reading, "a read is in flight");
    let toasts = settle(&mut source);
    assert!(!source.reading, "the read flag is cleared after the worker returns");
    assert!(!source.pump().busy, "the source is idle");

    // A write that encounters a transport error also clears.
    source.choose_location(2); // nothing to send to (the fake is gone), so it will error
    settle(&mut source);
    assert!(!source.writing, "the write flag is cleared");
}

/// A read that began before a change Beebeeb took lands late, with what the server held before it: it never
/// undoes the change on screen.
#[test]
fn a_read_that_began_before_a_change_never_undoes_it() {
    let account = account("settings-race@example.eu");
    let mut source = EngineSettingsSource::new(signed_in(&account));
    settle(&mut source);
    account.delay_ms.store(800, Ordering::SeqCst);
    source.reload_locations();
    std::thread::sleep(Duration::from_millis(150)); // the read has seen Germany
    source.choose_location(2);
    settle(&mut source);
    assert_eq!(source.general().storage_location, "Finland");
    assert_eq!(source.locations().unwrap().chosen, 2);
}

#[test]
fn the_theme_applies_at_once_and_is_there_after_a_restart() {
    let engine = engine_at("http://127.0.0.1:9");
    let mut source = EngineSettingsSource::new(engine.clone());
    let before = source.general().theme;
    let other = if before == ThemeChoice::Dark {
        ThemeChoice::Light
    } else {
        ThemeChoice::Dark
    };
    source.set_theme(other);
    assert_eq!(source.general().theme, other, "applies at once");
    assert!(settle(&mut source).is_empty(), "saved without a word");
    drop(source);
    let restarted = EngineSettingsSource::new(engine.clone());
    assert_eq!(
        restarted.general().theme,
        other,
        "read back from this PC's settings file"
    );
    let mut restarted = restarted;
    restarted.set_theme(before);
    settle(&mut restarted);
    assert_eq!(EngineSettingsSource::new(engine).general().theme, before);
}

/// The running app's window on `engine` (`state::init_on_engine`, as `shell::run` builds it), in Settings.
fn window_on(engine: Arc<Engine<MemoryCredentials>>) -> Runtime<State> {
    let state = crate::shell::state::init_on_engine(crate::shell::host::Shell::fixture(), engine);
    let env = Env {
        system_dark: false,
        scale: 2.0,
    };
    let mut rt = Runtime::new(state, crate::view, (958.0, 638.0), env);
    rt.on_key = Some(host::on_key);
    // Only settings' own poll: what it asks of the loop is all there is to see.
    rt.poll = Some(|s: &mut State, _: f64, tx: &mut Tx| crate::frames::settings::poll(s, tx));
    rt.frame(0.0);
    rt.update(|s, _| {
        host::dispatch(s, Event::OpenSettings(SettingsPane::General));
    });
    rt.frame(0.016);
    rt
}

fn shows(rt: &Runtime<State>, words: &str) -> bool {
    rt.elements()
        .iter()
        .any(|e| !e.exiting && e.text.as_deref() == Some(words))
}

/// What `frames::settings::poll` returns on this window now, run as the shell's poll runs it.
fn polled(rt: &mut Runtime<State>) -> bool {
    let out = Arc::new(Mutex::new(false));
    let into = out.clone();
    rt.update(move |s, tx| *into.lock().unwrap() = crate::frames::settings::poll(s, tx));
    *out.lock().unwrap()
}

/// The window's poll: an answer that arrives returns true and is drawn; with nothing out it returns false and
/// asks no wake.
#[test]
fn the_poll_draws_an_arriving_answer_and_asks_no_wake_when_idle() {
    let account = account("settings-poll@example.eu");
    // Long enough that building and drawing the window (slow on a CI runner) is over before it answers.
    account.delay_ms.store(3_000, Ordering::SeqCst);
    let mut rt = window_on(signed_in(&account));
    rt.poll = None; // only the polls below
    assert!(!polled(&mut rt), "nothing arrived yet");
    assert!(rt.wake_deadline().is_some(), "an answer is out: the loop looks again");
    assert!(!shows(&rt, "Germany"), "not known yet");
    let began = Instant::now();
    while !polled(&mut rt) {
        assert!(began.elapsed() < Duration::from_secs(10), "the answer never arrived");
        std::thread::sleep(Duration::from_millis(20));
    }
    rt.frame(0.1);
    assert!(shows(&rt, "Germany"), "the arriving answer is drawn");
    rt.frame(0.1);
    assert!(!polled(&mut rt), "an idle poll returns false");
    rt.frame(0.1);
    assert_eq!(rt.wake_deadline(), None, "and asks no wake");
}

/// The running app's Settings: General draws what it read, a pane nothing answers for is the shell's frame, and
/// no sample value of the board shows anywhere.
#[test]
fn the_running_apps_settings_show_no_sample_value() {
    let account = account("settings-samples@example.eu");
    let mut rt = window_on(signed_in(&account));
    let began = Instant::now();
    while !shows(&rt, "Germany") {
        assert!(began.elapsed() < Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(20));
        rt.frame(0.1);
    }
    let samples = [
        "0.9.1", "0.9.2", "311", "2084927", "09:12", "Anna", "Lotte", "Priya", "2 new", "240 GB",
    ];
    let check = |rt: &Runtime<State>, place: &str| {
        for e in rt.elements() {
            let words = format!("{} {}", e.text.unwrap_or_default(), e.label.unwrap_or_default());
            for sample in samples {
                assert!(!words.contains(sample), "{place} says {sample}: {words}");
            }
        }
    };
    check(&rt, "General");
    // Notifications is the engine's (`settings::notifications`): drawn, with no sample value in it.
    rt.update(|s, _| crate::frames::settings::go(s, "DW-0750"));
    rt.frame(0.016);
    assert!(shows(&rt, "Allow notifications"), "Notifications is drawn");
    check(&rt, "Notifications");
    for (frame, pane) in [("DW-0751", "About"), ("DW-0712", "Storage")] {
        rt.update(move |s, _| crate::frames::settings::go(s, frame));
        rt.frame(0.016);
        assert!(
            rt.elements().iter().any(|e| e.key.as_deref() == Some("not-built")),
            "{pane}: the shell's not-built frame"
        );
        check(&rt, pane);
    }
}
