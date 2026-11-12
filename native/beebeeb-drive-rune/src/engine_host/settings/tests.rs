//! Settings on the real engine (`EngineSettingsSource`), against the engine's fake account on loopback with the
//! storage-location routes added here: what General and Storage location draw, a change sent once and shown only
//! after Beebeeb took it, every way a read can fail, a slow reply, the theme across a restart, the window's poll,
//! and no sample value in the running app's Settings. An engine handle is on the process's state folder (the
//! engine refuses another), which nothing here writes in. Every name is invented.

use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
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

/// One account on a fake of its own, with the storage-location routes: the account's handler answers the rest.
struct Account {
    fake: FakeApi,
    mode: Arc<AtomicU8>,
    /// How long the region read waits before it answers, in ms.
    delay_ms: Arc<AtomicU64>,
    writes: Arc<Mutex<Vec<Value>>>,
    email: String,
    phrase: String,
}

fn account(email: &str) -> Account {
    let (account, phrase) = FakeAccount::register(email, PASSWORD, TEST_PHRASE);
    let account = Arc::new(account);
    let token = account.session_token.clone();
    let rest = account_handler(account);
    let (mode, delay_ms, writes) = (
        Arc::new(AtomicU8::new(ANSWER)),
        Arc::new(AtomicU64::new(0)),
        Arc::new(Mutex::new(Vec::new())),
    );
    let (mode_in, delay_in, writes_in) = (mode.clone(), delay_ms.clone(), writes.clone());
    let stored = Arc::new(Mutex::new(json!({ "preferred_region": "de", "region_strict": true })));
    let fake = FakeApi::start(Arc::new(move |req: &Request| {
        let route = req.route();
        if !route.ends_with(" /api/v1/me/region") {
            return rest(req);
        }
        match mode_in.load(Ordering::SeqCst) {
            SERVER_ERROR => return Reply::json(500, json!({ "error": "a server body" })),
            SESSION_ENDED => return Reply::json(401, json!({ "error": "a server body" })),
            _ => {}
        }
        if req.bearer() != Some(token.as_str()) {
            return Reply::json(401, json!({}));
        }
        std::thread::sleep(Duration::from_millis(delay_in.load(Ordering::SeqCst)));
        if route.starts_with("PUT") {
            let body = req.json();
            writes_in.lock().unwrap().push(body.clone());
            *stored.lock().unwrap() = json!({
                "preferred_region": body["preferred_region"],
                "region_strict": body["region_strict"].as_bool().unwrap_or(false),
            });
            return Reply::ok(stored.lock().unwrap().clone());
        }
        let choices = if mode_in.load(Ordering::SeqCst) == EMPTY {
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
        let now = stored.lock().unwrap().clone();
        Reply::ok(json!({
            "preferred_region": now["preferred_region"],
            "region_strict": now["region_strict"],
            "regions": [],
            "choices": choices,
        }))
    }));
    Account {
        fake,
        mode,
        delay_ms,
        writes,
        email: email.to_string(),
        phrase,
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
    assert!(!source.wired(Pane::About) && !source.wired(Pane::Notifications) && !source.wired(Pane::Recent));
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
    assert_eq!(settle(&mut source), [CHOSEN]);
    let now = source.locations().unwrap();
    assert_eq!((now.chosen, now.only_here), (2, true), "Finland, still only there");
    assert_eq!(source.general().storage_location, "Finland");
    assert_eq!(
        account.writes.lock().unwrap().clone(),
        [json!({ "preferred_region": "fi", "region_strict": true })],
        "sent once, with the strict rule it had"
    );

    // Refused: the old choice stands, and the toast says nothing changed.
    account.mode.store(SERVER_ERROR, Ordering::SeqCst);
    source.choose_location(1);
    assert_eq!(settle(&mut source), [NOT_CHOSEN]);
    assert_eq!(source.locations().unwrap().chosen, 2);
    source.set_only_here(false);
    assert_eq!(settle(&mut source), [NOT_CHOSEN]);
    assert!(source.locations().unwrap().only_here, "the switch is put back");

    account.mode.store(ANSWER, Ordering::SeqCst);
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
        account.mode.store(mode, Ordering::SeqCst);
        let mut source = EngineSettingsSource::new(signed_in(&account));
        settle(&mut source);
        assert!(source.locations().is_none(), "{why}: the list could not load");
        assert!(!source.locations_loading(), "{why}: not loading any more");
        assert_eq!(source.general().storage_location, "", "{why}: no guess in General");
        // Try again asks again, and an answer then shows.
        account.mode.store(ANSWER, Ordering::SeqCst);
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
    account.delay_ms.store(400, Ordering::SeqCst);
    source.choose_location(2);
    engine.sign_out_keeping_local_data().expect("signs out");
    settle(&mut source);
    assert!(source.locations().is_none(), "no list of an account that signed out");
    assert_eq!(source.general().storage_location, "", "and no place in General");
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

fn window_on(source: EngineSettingsSource<MemoryCredentials>) -> Runtime<State> {
    let mut state = crate::init();
    state.settings.source = Box::new(source);
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
    account.delay_ms.store(300, Ordering::SeqCst);
    let mut rt = window_on(EngineSettingsSource::new(signed_in(&account)));
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
    let mut rt = window_on(EngineSettingsSource::new(signed_in(&account)));
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
    for (frame, pane) in [
        ("DW-0751", "About"),
        ("DW-0750", "Notifications"),
        ("DW-0712", "Storage"),
    ] {
        rt.update(move |s, _| crate::frames::settings::go(s, frame));
        rt.frame(0.016);
        assert!(
            rt.elements().iter().any(|e| e.key.as_deref() == Some("not-built")),
            "{pane}: the shell's not-built frame"
        );
        check(&rt, pane);
    }
}
