//! Notifications and Recent notifications on the real engine (`EngineSettingsSource`, its notifications part),
//! against the engine's fake accounts on loopback with the notification routes answered here in the server's shape:
//! the list and its count (newest first, a type with no place left out), every state (empty, a server error, a
//! session that ended, unreadable, signed out), what is read when (only when the pane opens, never from a draw
//! call), the account's switches (a change sent alone, shown only after Beebeeb took it, a refusal and an unknown
//! result drawn as a toast), this PC's switches (in its settings file, across a restart), leaving the page (only what
//! it showed as new is marked; a refusal brings "New" back), the session rule (a lock, another account on the same
//! handle, an answer that outlived its session), and the window's element tree after another account signed in.
//! Every name, address and id is invented.

use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use beebeeb_engine::app::{Engine, EngineConfig};
use beebeeb_engine::host::ChannelHost;
use beebeeb_engine::identity::TEST;
use beebeeb_engine::signin::memory_store::MemoryCredentials;
use beebeeb_engine::test_api::fake_account::{FakeAccount, TEST_PHRASE, account_handler};
use beebeeb_engine::test_api::{FakeApi, Handler, Reply, Request};
use chrono::Local;
use rune::{Env, Runtime};
use serde_json::{Value, json};
use zeroize::Zeroizing;

use super::super::EngineSettingsSource;
use super::*;
use crate::State;
use crate::frames::settings::source::{Pumped, ReadState, SettingsSource};
use crate::shell::app::{Event, SettingsPane};

/// The read states, the rows' places on screen, and writes bound to the session of their press.
mod orders;
mod states;

/// Test hooks of the part (here, not beside it: the part's file is at its size limit).
impl<C: CredentialStores + 'static> NotificationsPart<C> {
    /// What it holds is old: the next opening reads again.
    pub(super) fn expire(&mut self) {
        self.read_at = None;
    }

    /// The next write (a switch's change, a mark) waits on its worker, before it calls the engine, until the
    /// returned sender is sent to or dropped.
    pub(super) fn hold_next_write(&mut self) -> mpsc::Sender<()> {
        let (release, held) = mpsc::channel();
        self.hold_next_write = Some(held);
        release
    }

    /// The next read waits on its worker, before it calls the engine, until the returned sender is sent to or
    /// dropped.
    pub(super) fn hold_next_read(&mut self) -> mpsc::Sender<()> {
        let (release, held) = mpsc::channel();
        self.hold_next_read = Some(held);
        release
    }
}

const PASSWORD: &str = "an invented password for notifications";

const ANSWER: u8 = 0;
const SERVER_ERROR: u8 = 1;
const SESSION_ENDED: u8 = 2;
const EMPTY: u8 = 3;
const UNREADABLE: u8 = 4;
/// A change only: something in front of Beebeeb answers what does not read, and nothing was stored.
const UNREADABLE_NOT_TAKEN: u8 = 5;
/// The switches' read answers as the list's does.
const AS_THE_LIST: u8 = 255;

const FIRST_SHARE: &str = "Invented Person shared Invented holiday photos with you";
const FIRST_SENDER: &str = "invented.sender@example.eu";
const FIRST_SIGN_IN: &str = "New sign-in from 192.0.2.10";
const FIRST_WELCOME: &str = "Invented welcome";
const FIRST_REGION: &str = "Invented country can't take new files right now";
const SECOND_CLAIM: &str = "Second account share claim pending";

/// One account's notifications and switches, as the fake holds them.
struct Held {
    list: Vec<Value>,
    switches: Value,
}

/// Two accounts on one fake: a sign-in leg goes to the account its email names, anything else to the account whose
/// token it carries.
struct Fake {
    fake: FakeApi,
    first: Arc<FakeAccount>,
    first_phrase: String,
    second: Arc<FakeAccount>,
    second_phrase: String,
    /// How the list and the switches' read answer.
    read_mode: Arc<AtomicU8>,
    /// How the switches' read answers, when not as the list's ([`AS_THE_LIST`]).
    switches_mode: Arc<AtomicU8>,
    /// A switch change that reaches the fake waits here, before it is stored, until this is sent to or dropped.
    put_gate: Arc<Mutex<Option<std::sync::mpsc::Receiver<()>>>>,
    /// How a switch change answers.
    put_mode: Arc<AtomicU8>,
    /// How a mark answers.
    mark_mode: Arc<AtomicU8>,
    /// How long the list waits before it answers (with what was held when it was asked), in ms.
    delay_ms: Arc<AtomicU64>,
    /// How long a switch change waits before the fake stores it, in ms.
    put_delay_ms: Arc<AtomicU64>,
    /// The writes the fake received: the route and its body.
    writes: Arc<Mutex<Vec<(String, Value)>>>,
    /// A mark of this id is refused (500); the others are taken.
    refuse_mark: Arc<Mutex<Option<String>>>,
}

/// Seconds before now, as the server writes a time.
fn ago(seconds: i64) -> String {
    (chrono::Utc::now() - chrono::Duration::seconds(seconds)).to_rfc3339()
}

fn row(id: &str, kind: &str, title: &str, body: Option<&str>, read: bool, at: &str) -> Value {
    json!({ "id": id, "type": kind, "title": title, "body": body, "data": null, "read": read, "created_at": at })
}

fn names_email(req: &Request, email: &str) -> bool {
    req.json()
        .get("email")
        .and_then(Value::as_str)
        .is_some_and(|sent| sent.eq_ignore_ascii_case(email))
}

fn mode_reply(mode: u8) -> Option<Reply> {
    match mode {
        SERVER_ERROR => Some(Reply::json(500, json!({ "error": "a server body" }))),
        SESSION_ENDED => Some(Reply::json(401, json!({ "error": "a server body" }))),
        UNREADABLE | UNREADABLE_NOT_TAKEN => Some(Reply::ok(json!("not the expected shape"))),
        _ => None,
    }
}

fn fake(tag: &str) -> Fake {
    fake_with(tag, Vec::new())
}

/// `fake`, with `extra` added to the first account's list.
fn fake_with(tag: &str, extra: Vec<Value>) -> Fake {
    let (first, first_phrase) = FakeAccount::register(&format!("{tag}-one@example.eu"), PASSWORD, TEST_PHRASE);
    let (second, second_phrase) = FakeAccount::register(&format!("{tag}-two@example.eu"), PASSWORD, TEST_PHRASE);
    let (first, second) = (Arc::new(first), Arc::new(second));
    let (one, two): (Handler, Handler) = (account_handler(first.clone()), account_handler(second.clone()));
    let (second_email, second_token) = (second.email.clone(), second.session_token.clone());
    let held = Arc::new(Mutex::new([
        Held {
            list: vec![
                row(
                    "n-1",
                    "share_received",
                    FIRST_SHARE,
                    Some(FIRST_SENDER),
                    false,
                    &ago(60),
                ),
                row("n-2", "welcome", FIRST_WELCOME, None, false, &ago(120)),
                row("n-3", "region_strict.incident", FIRST_REGION, None, false, &ago(180)),
                row(
                    "n-4",
                    "security",
                    FIRST_SIGN_IN,
                    Some("If this wasn't you, change your password."),
                    true,
                    &ago(3 * 86_400),
                ),
            ]
            .into_iter()
            .chain(extra)
            .collect(),
            switches: json!({ "file_updated": false, "share_received": true, "storage_warning": true, "new_device_login": true, "backup_complete": false }),
        },
        Held {
            list: vec![row("m-1", "share_claimed", SECOND_CLAIM, None, false, &ago(30))],
            switches: json!({ "file_updated": true, "share_received": false, "storage_warning": false, "new_device_login": true, "backup_complete": true }),
        },
    ]));
    let (read_mode, put_mode, mark_mode, delay_ms, put_delay_ms, writes) = (
        Arc::new(AtomicU8::new(ANSWER)),
        Arc::new(AtomicU8::new(ANSWER)),
        Arc::new(AtomicU8::new(ANSWER)),
        Arc::new(AtomicU64::new(0)),
        Arc::new(AtomicU64::new(0)),
        Arc::new(Mutex::new(Vec::new())),
    );
    let (read_in, put_in, mark_in, delay_in, put_delay_in, writes_in) = (
        read_mode.clone(),
        put_mode.clone(),
        mark_mode.clone(),
        delay_ms.clone(),
        put_delay_ms.clone(),
        writes.clone(),
    );
    let refuse_mark: Arc<Mutex<Option<String>>> = Arc::default();
    let refuse_in = refuse_mark.clone();
    let switches_mode = Arc::new(AtomicU8::new(AS_THE_LIST));
    let switches_in = switches_mode.clone();
    let put_gate: Arc<Mutex<Option<std::sync::mpsc::Receiver<()>>>> = Arc::default();
    let gate_in = put_gate.clone();
    let fake = FakeApi::start(Arc::new(move |req: &Request| {
        let route = req.route();
        let sign_in = route.starts_with("POST /api/v1/opaque/");
        let theirs = if sign_in {
            names_email(req, &second_email)
        } else {
            req.bearer() == Some(second_token.as_str())
        };
        if !route.contains(" /api/v1/notifications") {
            return if theirs { two(req) } else { one(req) };
        }
        let who = usize::from(theirs);
        match route.as_str() {
            "GET /api/v1/notifications" => {
                if let Some(reply) = mode_reply(read_in.load(Ordering::SeqCst)) {
                    return reply;
                }
                let mut list = if read_in.load(Ordering::SeqCst) == EMPTY {
                    Vec::new()
                } else {
                    held.lock().unwrap()[who].list.clone()
                };
                // Newest first, as the server orders it.
                list.sort_by(|a, b| b["created_at"].as_str().cmp(&a["created_at"].as_str()));
                std::thread::sleep(Duration::from_millis(delay_in.load(Ordering::SeqCst)));
                let unread = list.iter().filter(|n| n["read"] == false).count();
                Reply::ok(json!({ "notifications": list, "unread_count": unread }))
            }
            "GET /api/v1/notifications/preferences" => {
                let mode = match switches_in.load(Ordering::SeqCst) {
                    AS_THE_LIST => read_in.load(Ordering::SeqCst),
                    own => own,
                };
                if let Some(reply) = mode_reply(mode) {
                    return reply;
                }
                Reply::ok(json!({ "preferences": held.lock().unwrap()[who].switches.clone() }))
            }
            "PUT /api/v1/notifications/preferences" => {
                let mode = put_in.load(Ordering::SeqCst);
                if mode != UNREADABLE
                    && let Some(reply) = mode_reply(mode)
                {
                    return reply;
                }
                std::thread::sleep(Duration::from_millis(put_delay_in.load(Ordering::SeqCst)));
                let gate = gate_in.lock().unwrap().take();
                if let Some(gate) = gate {
                    let _ = gate.recv();
                }
                let body = req.json();
                writes_in.lock().unwrap().push((route.clone(), body.clone()));
                let mut held = held.lock().unwrap();
                if let (Some(now), Some(changes)) = (held[who].switches.as_object_mut(), body.as_object()) {
                    for (key, value) in changes {
                        now.insert(key.clone(), value.clone());
                    }
                }
                // Unreadable: stored, then an answer that does not read (the change landed).
                if let Some(reply) = mode_reply(mode) {
                    return reply;
                }
                Reply::ok(json!({ "preferences": held[who].switches.clone() }))
            }
            patch if patch.starts_with("PATCH /api/v1/notifications/") => {
                if let Some(reply) = mode_reply(mark_in.load(Ordering::SeqCst)) {
                    return reply;
                }
                let id = patch.rsplit('/').next().unwrap_or_default();
                if refuse_in.lock().unwrap().as_deref() == Some(id) {
                    return Reply::json(500, json!({ "error": "a server body" }));
                }
                writes_in.lock().unwrap().push((route.clone(), req.json()));
                let mut held = held.lock().unwrap();
                match held[who].list.iter_mut().find(|n| n["id"] == id) {
                    Some(n) => {
                        n["read"] = json!(true);
                        Reply::ok(json!({ "message": "updated" }))
                    }
                    None => Reply::json(404, json!({ "error": "not found" })),
                }
            }
            _ => Reply::json(404, json!({ "error": "not found" })),
        }
    }));
    Fake {
        fake,
        first,
        first_phrase,
        second,
        second_phrase,
        read_mode,
        switches_mode,
        put_gate,
        put_mode,
        mark_mode,
        delay_ms,
        put_delay_ms,
        writes,
        refuse_mark,
    }
}

fn engine_at(base: &str) -> Arc<Engine<MemoryCredentials>> {
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "notifications-test-slot".into(),
        state_dir: crate::engine_host::tests::process_state_dir(),
        api_base: base.to_string(),
    };
    let engine = Engine::open(&TEST, config, host, MemoryCredentials::default()).expect("the engine opens");
    engine.restore();
    Arc::new(engine)
}

fn sign_in(engine: &Engine<MemoryCredentials>, account: &FakeAccount, phrase: &str) {
    engine
        .sign_in_password(&account.email, Zeroizing::new(PASSWORD.into()))
        .expect("the fake accepts the password");
    engine
        .unlock_with_phrase(Zeroizing::new(phrase.to_string()))
        .expect("the phrase unlocks");
}

fn signed_in(fake: &Fake) -> Arc<Engine<MemoryCredentials>> {
    let engine = engine_at(&fake.fake.base);
    sign_in(&engine, &fake.first, &fake.first_phrase);
    engine
}

type Source = EngineSettingsSource<MemoryCredentials>;

/// Pump until nothing is out (at most 10 s); every toast on the way, in order.
fn settle(source: &mut Source) -> Vec<&'static str> {
    let began = Instant::now();
    let mut toasts = Vec::new();
    loop {
        let pumped: Pumped = source.pump();
        toasts.extend(pumped.toast);
        if !pumped.busy {
            return toasts;
        }
        assert!(began.elapsed() < Duration::from_secs(10), "an answer did not come back");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Pump until `done` says so (at most 10 s); every toast on the way, in order.
fn pump_until(source: &mut Source, done: impl Fn(&Source) -> bool) -> Vec<&'static str> {
    let began = Instant::now();
    let mut toasts = Vec::new();
    loop {
        toasts.extend(source.pump().toast);
        if done(source) {
            return toasts;
        }
        assert!(began.elapsed() < Duration::from_secs(10), "it did not come");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn opened(source: &mut Source, pane: Pane) -> Vec<&'static str> {
    source.opened(pane);
    settle(source)
}

fn asked(fake: &FakeApi, route: &str) -> usize {
    fake.requests().iter().filter(|r| r.route() == route).count()
}

const LIST: &str = "GET /api/v1/notifications";
const SWITCHES: &str = "GET /api/v1/notifications/preferences";

/// Every title on the Recent page, with its "New".
fn titles(source: &Source) -> Vec<(String, bool)> {
    source
        .recent_alerts()
        .into_iter()
        .flat_map(|(_, alerts)| alerts.into_iter().map(|a| (a.title, a.new)))
        .collect()
}

/// The groups whose switches are read (every group and row is always in its place; the account's rows are drawn
/// without a switch until its switches are read).
fn group_titles(source: &Source) -> Vec<String> {
    source
        .notifications()
        .groups
        .into_iter()
        .enumerate()
        .filter(|(group, _)| source.alerts_state(*group) == ReadState::Ready)
        .map(|(_, (title, _))| title)
        .collect()
}

/// Every group with its count of rows.
fn places(source: &Source) -> Vec<(String, usize)> {
    source
        .notifications()
        .groups
        .iter()
        .map(|(title, rows)| (title.clone(), rows.len()))
        .collect()
}

fn all_places() -> Vec<(String, usize)> {
    vec![
        ("Your account".to_string(), 3),
        ("Sync on this PC".to_string(), 4),
        ("Backups on this PC".to_string(), 2),
    ]
}

/// The switch at `group`, `index` now (it is read).
fn switch(source: &Source, group: usize, index: usize) -> bool {
    source.notifications().groups[group].1[index]
        .on
        .expect("the switch is read")
}

#[test]
fn recent_is_the_accounts_newest_first_and_a_type_with_no_place_is_left_out() {
    let fake = fake("recent");
    let mut source = Source::new(signed_in(&fake));
    // Drawing never reads: the window may draw a thousand times before the pane opens.
    for _ in 0..20 {
        let _ = (source.recent_alerts(), source.notifications());
    }
    settle(&mut source);
    assert_eq!(asked(&fake.fake, LIST), 0, "nothing is read before the pane opens");
    assert!(source.recent_alerts().is_empty());
    assert_eq!(source.notifications().new_recent, 0);

    assert!(opened(&mut source, Pane::Notifications).is_empty());
    let days = source.recent_alerts();
    let now = Local::now();
    let day = |seconds: i64| day_of(now - chrono::Duration::seconds(seconds), now);
    let heads: Vec<_> = days.iter().map(|(day, alerts)| (day.clone(), alerts.len())).collect();
    assert_eq!(heads, [(day(60), 1), (day(3 * 86_400), 1)], "by day, newest first");
    let share = &days[0].1[0];
    assert_eq!(
        (
            share.kind,
            share.leads_to,
            share.title.as_str(),
            share.says.as_str(),
            share.new
        ),
        (AlertKind::Share, AlertTarget::Shared, FIRST_SHARE, FIRST_SENDER, true)
    );
    assert_eq!(share.time.len(), 5, "the time of day: {}", share.time);
    let sign_in = days.last().unwrap().1.last().unwrap();
    assert_eq!(
        (sign_in.kind, sign_in.leads_to, sign_in.title.as_str(), sign_in.new),
        (AlertKind::Security, AlertTarget::SecurityLog, FIRST_SIGN_IN, false)
    );
    let all = format!("{days:?}");
    assert!(
        !all.contains(FIRST_WELCOME) && !all.contains(FIRST_REGION),
        "a type with no place is left out"
    );
    assert_eq!(
        source.notifications().new_recent,
        1,
        "the count is the shown alerts marked New (2 unread are left out)"
    );

    // Fresh: opening Recent right after asks nothing.
    let before = asked(&fake.fake, LIST);
    opened(&mut source, Pane::Recent);
    assert_eq!(asked(&fake.fake, LIST), before, "a fresh list is not read again");
    source.alerts.expire();
    opened(&mut source, Pane::Recent);
    assert_eq!(asked(&fake.fake, LIST), before + 1, "an old one is");
}

#[test]
fn an_empty_list_is_nothing_to_show_and_no_count() {
    let fake = fake("empty");
    fake.read_mode.store(EMPTY, Ordering::SeqCst);
    let mut source = Source::new(signed_in(&fake));
    assert!(opened(&mut source, Pane::Recent).is_empty());
    assert!(source.recent_alerts().is_empty());
    assert_eq!(
        source.recent_state(),
        ReadState::Ready,
        "read, and empty: the page may say so"
    );
    assert_eq!(source.notifications().new_recent, 0);
    assert_eq!(
        group_titles(&source),
        ["Your account", "Sync on this PC", "Backups on this PC"],
        "the switches read: every group is drawn"
    );
}

#[test]
fn a_failed_read_draws_nothing_of_the_account_and_says_so() {
    for mode in [SERVER_ERROR, UNREADABLE] {
        let fake = fake(&format!("failed-{mode}"));
        fake.read_mode.store(mode, Ordering::SeqCst);
        let mut source = Source::new(signed_in(&fake));
        assert_eq!(opened(&mut source, Pane::Notifications), [NOT_LOADED], "mode {mode}");
        assert!(source.recent_alerts().is_empty());
        assert_eq!(
            group_titles(&source),
            ["Sync on this PC", "Backups on this PC"],
            "the account's switches were not read: never a guess"
        );
        assert_eq!(
            source.alerts_state(0),
            ReadState::Failed,
            "said as failed, not as loading"
        );
        assert_eq!(
            source.recent_state(),
            ReadState::Failed,
            "never an empty list nobody read"
        );
        assert_eq!(places(&source), all_places(), "the rows keep their places");
        // The next opening tries again (a failed read is never fresh), and an answer is drawn.
        fake.read_mode.store(ANSWER, Ordering::SeqCst);
        assert!(opened(&mut source, Pane::Notifications).is_empty());
        assert_eq!(group_titles(&source)[0], "Your account");
        assert_eq!(source.notifications().new_recent, 1);
    }
}

#[test]
fn a_session_that_ended_shows_nothing_and_signed_out_asks_nothing() {
    let fake = fake("ended");
    let mut source = Source::new(signed_in(&fake));
    opened(&mut source, Pane::Recent);
    assert_eq!(source.notifications().new_recent, 1);
    fake.read_mode.store(SESSION_ENDED, Ordering::SeqCst);
    source.alerts.expire();
    assert!(
        opened(&mut source, Pane::Recent).is_empty(),
        "the banner says the session ended, not a toast"
    );
    assert!(source.recent_alerts().is_empty());
    assert_eq!(source.notifications().new_recent, 0);
    assert_eq!(group_titles(&source), ["Sync on this PC", "Backups on this PC"]);

    let signed_out = engine_at(&fake.fake.base);
    let before = asked(&fake.fake, LIST) + asked(&fake.fake, SWITCHES);
    let mut source = Source::new(signed_out);
    assert!(opened(&mut source, Pane::Notifications).is_empty());
    assert_eq!(
        asked(&fake.fake, LIST) + asked(&fake.fake, SWITCHES),
        before,
        "nothing is asked without a session"
    );
    assert!(source.recent_alerts().is_empty());
}

#[test]
fn an_account_switch_is_sent_alone_and_shown_only_after_beebeeb_took_it() {
    let fake = fake("switch");
    let mut source = Source::new(signed_in(&fake));
    opened(&mut source, Pane::Notifications);
    let alerts = &source.notifications().groups[0].1;
    let seen: Vec<_> = alerts.iter().map(|a| (a.title.as_str(), a.on.expect("read"))).collect();
    assert_eq!(
        seen,
        [
            ("Shared with you", true),
            ("File updates", false),
            ("Storage warnings", true)
        ],
        "the account's switches as Beebeeb holds them"
    );

    // Long enough that the second change is made while the first is on its way.
    fake.put_delay_ms.store(500, Ordering::SeqCst);
    source.set_alert(0, 1, true);
    assert!(!switch(&source, 0, 1), "not shown before Beebeeb took it");
    source.set_alert(0, 2, false);
    assert_eq!(settle(&mut source), [STILL_SAVING, ALERT_SAVED], "one change at a time");
    assert!(switch(&source, 0, 1), "shown once taken");
    assert!(switch(&source, 0, 2), "the change that had to wait was not sent");
    assert_eq!(
        fake.writes.lock().unwrap().clone(),
        [(
            "PUT /api/v1/notifications/preferences".to_string(),
            json!({ "file_updated": true })
        )],
        "only the switch that changed; never new_device_login"
    );
}

#[test]
fn a_refused_account_switch_stays_and_an_unknown_result_is_read_again() {
    let fake = fake("refused");
    let mut source = Source::new(signed_in(&fake));
    opened(&mut source, Pane::Notifications);
    for mode in [SERVER_ERROR, SESSION_ENDED] {
        fake.put_mode.store(mode, Ordering::SeqCst);
        source.set_alert(0, 0, false);
        assert_eq!(
            settle(&mut source),
            [ALERT_NOT_SAVED],
            "mode {mode}: the refusal is drawn"
        );
        assert!(switch(&source, 0, 0), "mode {mode}: nothing changed");
    }
    fake.put_mode.store(UNREADABLE, Ordering::SeqCst);
    let before = asked(&fake.fake, SWITCHES);
    source.set_alert(0, 0, false);
    assert_eq!(
        settle(&mut source),
        [ALERT_UNKNOWN, ALERT_SAVED],
        "checking, and then the check's answer: the change had landed"
    );
    // The same, and nothing was stored: the check says the switch is as it was. Never "updated".
    fake.put_mode.store(UNREADABLE_NOT_TAKEN, Ordering::SeqCst);
    let held = switch(&source, 0, 1);
    source.set_alert(0, 1, !held);
    assert_eq!(
        settle(&mut source),
        [ALERT_UNKNOWN, ALERT_NOT_SAVED],
        "checking, and then the check's answer: nothing changed"
    );
    assert_eq!(switch(&source, 0, 1), held);
    fake.put_mode.store(UNREADABLE, Ordering::SeqCst);
    assert_eq!(
        asked(&fake.fake, SWITCHES),
        before + 1,
        "what Beebeeb has is read again"
    );
    assert!(
        !switch(&source, 0, 0),
        "the fake stored it before the unreadable answer: the re-read shows it"
    );
}

#[test]
fn this_pcs_switches_are_in_its_settings_file_across_a_restart() {
    let fake = fake("pc");
    let engine = signed_in(&fake);
    let mut source = Source::new(engine.clone());
    opened(&mut source, Pane::Notifications);
    let (allow, backup) = (source.notifications().allow, switch(&source, 2, 0));

    source.set_allow_notifications(!allow);
    source.set_alert(2, 0, !backup);
    assert_eq!(source.notifications().allow, !allow, "applies at once");
    assert_eq!(switch(&source, 2, 0), !backup);
    assert!(settle(&mut source).is_empty());
    let file = DesktopConfig::load_for(crate::shell::identity::engine_identity()).expect("the file reads");
    assert_eq!((file.notify_allow, file.notify_backup_finished), (!allow, !backup));
    assert!(
        fake.writes.lock().unwrap().is_empty(),
        "nothing of this PC is sent to Beebeeb"
    );

    let mut restarted = Source::new(engine.clone());
    opened(&mut restarted, Pane::Notifications);
    assert_eq!(
        (restarted.notifications().allow, switch(&restarted, 2, 0)),
        (!allow, !backup)
    );
    restarted.set_allow_notifications(allow);
    restarted.set_alert(2, 0, backup);
    settle(&mut restarted);
    let file = DesktopConfig::load_for(crate::shell::identity::engine_identity()).expect("the file reads");
    assert_eq!((file.notify_allow, file.notify_backup_finished), (allow, backup));
}

#[test]
fn leaving_recent_marks_only_what_it_showed_as_new() {
    let fake = fake("seen");
    let mut source = Source::new(signed_in(&fake));
    opened(&mut source, Pane::Recent);
    assert_eq!(source.notifications().new_recent, 1);
    source.mark_recent_seen();
    assert_eq!(source.notifications().new_recent, 0, "at once");
    assert!(titles(&source).iter().all(|(_, new)| !new));
    assert!(settle(&mut source).is_empty());
    assert_eq!(
        fake.writes.lock().unwrap().clone(),
        [("PATCH /api/v1/notifications/n-1".to_string(), json!({ "read": true }))],
        "only the alert shown as new; never the two the page left out"
    );
    source.mark_recent_seen();
    settle(&mut source);
    assert_eq!(fake.writes.lock().unwrap().len(), 1, "nothing new: nothing sent");
    source.alerts.expire();
    opened(&mut source, Pane::Recent);
    assert_eq!(source.notifications().new_recent, 0, "Beebeeb says it is read");
}

#[test]
fn a_refused_mark_brings_new_back_and_says_so() {
    let fake = fake("mark-refused");
    let mut source = Source::new(signed_in(&fake));
    opened(&mut source, Pane::Recent);
    fake.mark_mode.store(SERVER_ERROR, Ordering::SeqCst);
    source.mark_recent_seen();
    assert_eq!(source.notifications().new_recent, 0);
    assert_eq!(settle(&mut source), [NOT_MARKED], "the refusal is drawn");
    assert_eq!(
        source.notifications().new_recent,
        1,
        "Beebeeb still says it is new: so does the page"
    );
    assert_eq!(titles(&source)[0], (FIRST_SHARE.to_string(), true));
}

#[test]
fn a_lock_drops_everything_at_once_and_another_account_never_sees_it() {
    let fake = fake("session");
    let engine = signed_in(&fake);
    let mut source = Source::new(engine.clone());
    opened(&mut source, Pane::Recent);
    assert_eq!(titles(&source).len(), 2);

    let _ = engine.lock();
    // Before any poll: the accessors already hand out nothing of the locked session.
    assert!(source.recent_alerts().is_empty());
    assert_eq!(source.notifications().new_recent, 0);
    assert_eq!(group_titles(&source), ["Sync on this PC", "Backups on this PC"]);
    assert!(source.pump().changed, "the poll drops what it held");

    engine.sign_out_keeping_local_data().expect("signed out");
    sign_in(&engine, &fake.second, &fake.second_phrase);
    assert!(source.recent_alerts().is_empty(), "nothing of the first account");
    source.mark_recent_seen();
    settle(&mut source);
    assert!(
        fake.writes.lock().unwrap().is_empty(),
        "nothing of the first account is marked"
    );
    assert_eq!(
        titles(&source),
        [(SECOND_CLAIM.to_string(), true)],
        "the page was showing: the new session's own list is read once, without a reopening"
    );
    let reads = asked(&fake.fake, LIST);
    settle(&mut source);
    assert_eq!(asked(&fake.fake, LIST), reads, "once per session, not on every poll");
    opened(&mut source, Pane::Recent);
    let claim = &source.recent_alerts()[0].1[0];
    assert_eq!(
        (claim.kind, claim.leads_to),
        (AlertKind::Request, AlertTarget::SharedRequests)
    );
    let mine: Vec<_> = source.notifications().groups[0].1.iter().map(|a| a.on).collect();
    assert_eq!(
        mine,
        [Some(false), Some(true), Some(false)],
        "the second account's own switches"
    );
}

#[test]
fn an_answer_made_for_a_session_that_ended_is_dropped() {
    let fake = fake("late");
    let engine = signed_in(&fake);
    let mut source = Source::new(engine.clone());
    fake.delay_ms.store(500, Ordering::SeqCst);
    source.opened(Pane::Recent);
    std::thread::sleep(Duration::from_millis(100));
    let _ = engine.lock();
    settle(&mut source);
    assert!(
        source.recent_alerts().is_empty(),
        "an answer that outlived its session is never drawn"
    );
    assert_eq!(source.notifications().new_recent, 0);
    assert_eq!(group_titles(&source), ["Sync on this PC", "Backups on this PC"]);
}

/// Every text and label in the window's element tree.
fn words(rt: &Runtime<State>) -> String {
    rt.elements()
        .into_iter()
        .filter(|e| !e.exiting)
        .flat_map(|e| [e.label.unwrap_or_default(), e.text.unwrap_or_default()])
        .collect::<Vec<_>>()
        .join("\n")
}

/// Poll settings as the window does until nothing is out, then draw.
fn poll(rt: &mut Runtime<State>) {
    let began = Instant::now();
    loop {
        rt.update(|s, tx| {
            crate::frames::settings::poll(s, tx);
        });
        rt.frame(0.0);
        rt.settle_all();
        rt.frame(0.0);
        if !rt.state.settings.source.pump().busy {
            // What that last look took in is drawn too.
            rt.update(|s, tx| {
                crate::frames::settings::poll(s, tx);
            });
            rt.frame(0.0);
            rt.settle_all();
            rt.frame(0.0);
            return;
        }
        assert!(began.elapsed() < Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn go(rt: &mut Runtime<State>, frame: &'static str) {
    rt.update(move |s, _| crate::frames::settings::go(s, frame));
    poll(rt);
}

#[test]
fn the_window_shows_nothing_of_the_account_before() {
    let fake = fake("window");
    let engine = signed_in(&fake);
    let state = crate::shell::state::init_on_engine(crate::shell::host::Shell::fixture(), engine.clone());
    let env = Env {
        system_dark: false,
        scale: 2.0,
    };
    let mut rt = Runtime::new(state, crate::view, (958.0, 638.0), env);
    rt.frame(0.0);
    rt.update(|s, _| {
        crate::shell::host::dispatch(s, Event::OpenSettings(SettingsPane::Notifications));
    });
    go(&mut rt, "DW-0750");
    go(&mut rt, "DW-0791");
    let seen = words(&rt);
    assert!(seen.contains(FIRST_SHARE), "the first account's alert is drawn: {seen}");
    assert!(seen.contains(FIRST_SENDER));

    let _ = engine.lock();
    poll(&mut rt);
    let seen = words(&rt);
    assert!(
        !seen.contains(FIRST_SHARE) && !seen.contains(FIRST_SENDER),
        "drawn after the lock"
    );

    engine.sign_out_keeping_local_data().expect("signed out");
    sign_in(&engine, &fake.second, &fake.second_phrase);
    go(&mut rt, "DW-0750");
    let seen = words(&rt);
    assert!(seen.contains("1 new"), "the second account's own count: {seen}");
    go(&mut rt, "DW-0791");
    let seen = words(&rt);
    for first in [FIRST_SHARE, FIRST_SENDER, FIRST_SIGN_IN] {
        assert!(
            !seen.contains(first),
            "the first account's {first} is drawn for the second"
        );
    }
    assert!(seen.contains(SECOND_CLAIM), "the second account's alert: {seen}");
}

#[test]
fn a_day_reads_as_today_yesterday_or_its_date() {
    let at = |m: u32, d: u32, h: u32| Local.with_ymd_and_hms(2026, m, d, h, 0, 0).single().expect("a time");
    let now = at(11, 5, 12);
    assert_eq!(day_of(at(11, 5, 1), now), "Today");
    assert_eq!(day_of(at(11, 4, 23), now), "Yesterday");
    assert_eq!(day_of(at(11, 3, 9), now), "3 Nov");
    let new_year = Local.with_ymd_and_hms(2027, 1, 2, 9, 0, 0).single().unwrap();
    assert_eq!(day_of(at(12, 30, 9), new_year), "30 Dec 2026");
}

#[test]
fn a_press_while_the_read_lands_changes_only_the_switch_drawn_there() {
    let fake = fake("press");
    // The read is out long enough to press while it is.
    fake.delay_ms.store(800, Ordering::SeqCst);
    let mut source = Source::new(signed_in(&fake));
    source.opened(Pane::Notifications);
    assert_eq!(
        places(&source),
        all_places(),
        "every group and row in its place before the read lands"
    );
    assert_eq!(
        source.alerts_state(0),
        ReadState::Loading,
        "the account's switches are not read: said so, never a guess"
    );
    assert_eq!(source.recent_state(), ReadState::Loading);
    assert_eq!(
        [source.alerts_state(1), source.alerts_state(2)],
        [ReadState::Ready; 2],
        "this PC's switches need no read"
    );
    let stopped = switch(&source, 1, 0);
    // A press on the account's group before it is read changes nothing; one on "Sync stopped" changes that one.
    source.set_alert(0, 0, false);
    source.set_alert(1, 0, !stopped);
    assert!(settle(&mut source).is_empty());
    assert_eq!(places(&source), all_places(), "the read landed: nothing moved");
    assert_eq!(source.alerts_state(0), ReadState::Ready);
    assert_eq!(source.recent_state(), ReadState::Ready);
    assert_eq!(switch(&source, 1, 0), !stopped, "the switch drawn there changed");
    assert!(switch(&source, 0, 0), "Shared with you is as Beebeeb holds it");
    // The same press after the read lands still means the same switch.
    source.set_alert(1, 0, stopped);
    assert!(settle(&mut source).is_empty());
    assert_eq!(switch(&source, 1, 0), stopped);
    assert!(
        fake.writes.lock().unwrap().is_empty(),
        "no account setting was sent: every press was on this PC's switch"
    );
}

#[test]
fn a_mark_that_fails_part_way_brings_new_back_only_on_what_beebeeb_holds_unread() {
    const SECOND_SHARE: &str = "Invented Colleague shared Invented plans with you";
    let fake = fake_with(
        "mark-part",
        vec![row("n-5", "share_received", SECOND_SHARE, None, false, &ago(90))],
    );
    let mut source = Source::new(signed_in(&fake));
    opened(&mut source, Pane::Recent);
    assert_eq!(source.notifications().new_recent, 2);
    // The newest is marked, the second is refused: the engine stops there.
    *fake.refuse_mark.lock().unwrap() = Some("n-5".into());
    source.mark_recent_seen();
    assert_eq!(settle(&mut source), [NOT_MARKED], "the refusal is drawn");
    assert_eq!(
        titles(&source),
        [
            (FIRST_SHARE.to_string(), false),
            (SECOND_SHARE.to_string(), true),
            (FIRST_SIGN_IN.to_string(), false)
        ],
        "read again: New only on the one Beebeeb still holds unread"
    );
    assert_eq!(source.notifications().new_recent, 1);

    *fake.refuse_mark.lock().unwrap() = None;
    source.mark_recent_seen();
    assert!(settle(&mut source).is_empty());
    let marked: Vec<_> = fake
        .writes
        .lock()
        .unwrap()
        .iter()
        .map(|(route, _)| route.clone())
        .collect();
    assert_eq!(
        marked,
        ["PATCH /api/v1/notifications/n-1", "PATCH /api/v1/notifications/n-5"],
        "the one already marked is not sent again"
    );
}
