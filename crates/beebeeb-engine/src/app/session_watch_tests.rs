//! Task 2535: the account's session watch (`session_watch.rs`). The fake answers `GET /api/v1/auth/me` as each test
//! sets it. A session is put in memory and taken out as the engine's transitions do: the generation moves first, then
//! the session is written or removed. No sync runner runs in any of these tests: only the watch talks to the fake.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::json;

use super::{SESSION_WATCH_PERIOD, SessionWatch, Step};
use crate::account::{AccountId, AccountRuntime, SessionTransition};
use crate::api_client::DesktopApiError;
use crate::session::Session;
use crate::test_api::{FakeApi, Handler, Reply, Request};

const TOKEN: &str = "an-invented-session-token";
/// A period no test reaches: a feed, or a stamp the watch must not make, stays "within" it for the whole test.
const LONG: Duration = Duration::from_secs(600);
/// The period of a running watch in these tests.
const PERIOD: Duration = Duration::from_millis(50);
/// How long the fake takes to answer in the slow-link test: more than the paused tick's 2 s cutoff.
const SLOW: Duration = Duration::from_millis(5_500);

/// What the fake answers to `GET /api/v1/auth/me`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Me {
    Accepts,
    Refuses,
    Unavailable,
    Suspended,
    HangsUp,
}

/// One session check the fake received: when it arrived, and the token it carried.
struct Seen {
    at: Instant,
    bearer: Option<String>,
}

/// A fake API whose session check the test switches, and that keeps every check it received ([`Seen`]).
struct Fake {
    api: FakeApi,
    me: Arc<Mutex<Me>>,
    delay: Arc<Mutex<Duration>>,
    seen: Arc<Mutex<Vec<Seen>>>,
    release: Arc<AtomicBool>,
}

impl Fake {
    fn start(me: Me) -> Self {
        let me = Arc::new(Mutex::new(me));
        let delay = Arc::new(Mutex::new(Duration::ZERO));
        let seen: Arc<Mutex<Vec<Seen>>> = Arc::default();
        let release = Arc::new(AtomicBool::new(false));
        let handler: Handler = {
            let (me, delay, seen, release) = (me.clone(), delay.clone(), seen.clone(), release.clone());
            Arc::new(move |request: &Request| {
                if request.route() != "GET /api/v1/auth/me" {
                    return Reply::error(404, "not_found");
                }
                seen.lock().unwrap().push(Seen {
                    at: Instant::now(),
                    bearer: request.bearer().map(str::to_string),
                });
                // The delay is waited in short steps and read again at each: a test that sets it to zero, or ends
                // (`release`), lets every waiting answer go at once.
                let arrived = Instant::now();
                while arrived.elapsed() < *delay.lock().unwrap() && !release.load(Ordering::SeqCst) {
                    std::thread::sleep(Duration::from_millis(10));
                }
                match *me.lock().unwrap() {
                    Me::Accepts => Reply::ok(json!({ "id": "u-watch" })),
                    Me::Refuses => Reply::error(401, "unauthorized"),
                    Me::Unavailable => Reply::error(503, "unavailable"),
                    Me::Suspended => Reply::error(403, "account_suspended"),
                    Me::HangsUp => Reply::hang_up(),
                }
            })
        };
        Self {
            api: FakeApi::start(handler),
            me,
            delay,
            seen,
            release,
        }
    }

    fn set_answer(&self, me: Me) {
        *self.me.lock().unwrap() = me;
    }

    fn set_delay(&self, delay: Duration) {
        *self.delay.lock().unwrap() = delay;
    }

    /// How many session checks reached the fake.
    fn checks(&self) -> usize {
        self.seen.lock().unwrap().len()
    }

    fn watch(&self, acct: &Arc<AccountRuntime>, period: Duration) -> SessionWatch {
        SessionWatch::new(acct.clone(), self.api.base.clone(), period)
    }
}

impl Drop for Fake {
    fn drop(&mut self) {
        self.release.store(true, Ordering::SeqCst);
    }
}

fn account() -> Arc<AccountRuntime> {
    Arc::new(AccountRuntime::new(AccountId("a-session-watch-test".into())))
}

/// A session in memory, as a sign-in puts it there: the generation moves, then the session is written.
fn sign_in(acct: &AccountRuntime, token: &str) {
    acct.advance_session_generation_holding_session_write_lock(SessionTransition::Write);
    *acct.session.lock().unwrap() = Some(Session {
        token: token.to_string(),
        master_key: [7; 32],
        email: None,
    });
}

/// The session leaves memory, as a vault lock or a sign-out takes it: the generation moves, then the session goes.
fn take_out(acct: &AccountRuntime, by: SessionTransition) {
    acct.advance_session_generation_holding_session_write_lock(by);
    *acct.session.lock().unwrap() = None;
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
}

/// Beebeeb refused the session: a 401, as the API client returns it.
fn refused() -> anyhow::Error {
    anyhow::Error::from(DesktopApiError::HttpStatus {
        status: reqwest::StatusCode::UNAUTHORIZED,
        snippet: String::new(),
    })
}

fn wait_until(limit: Duration, mut done: impl FnMut() -> bool) -> bool {
    let until = Instant::now() + limit;
    while Instant::now() < until {
        if done() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    done()
}

/// Rule 1. The vault is open, no runner runs, and the server ended the session: three steps in a row are three
/// refusals, and the session is held as ended (what the session watcher closes the vault on). The running watch gets
/// there by itself in three checks, the first one period after its start, and asks nothing after.
/// M1 (the watch does not note its answer): never held as ended. M1b (`note_watch_answer` stamps the feed time): the
/// second and third steps find the run fed by the watch's own answer and ask nothing (with a period it would take five
/// periods instead of three).
#[test]
fn the_watch_closes_an_ended_session_with_no_runner() {
    let rt = runtime();
    let fake = Fake::start(Me::Refuses);
    let acct = account();
    sign_in(&acct, TOKEN);

    // Steps back to back, under a period no step reaches: only the watch's own answers could make one look fed.
    let watch = fake.watch(&acct, LONG);
    let steps: Vec<Step> = (0..3).map(|_| rt.block_on(watch.step())).collect();
    assert_eq!(
        steps,
        [Step::Refused; 3],
        "nothing fed the run, so every step asks (M1b)"
    );
    assert!(
        acct.auth_health.is_expired(),
        "three refusals in a row: the session is held as ended (M1)"
    );
    assert_eq!(fake.checks(), 3, "one check per step");
    assert!(
        fake.seen
            .lock()
            .unwrap()
            .iter()
            .all(|seen| seen.bearer.as_deref() == Some(TOKEN)),
        "each check carried the session in memory's token"
    );
    assert_eq!(
        rt.block_on(watch.step()),
        Step::AlreadyEnded,
        "a session held as ended is not asked again"
    );
    assert_eq!(fake.checks(), 3);

    // The running watch, on another account (a run of its own), at the test period.
    let acct = account();
    sign_in(&acct, TOKEN);
    let before = fake.checks();
    let started = Instant::now();
    let _running = rt.spawn(fake.watch(&acct, PERIOD).run());
    assert!(
        wait_until(Duration::from_secs(20), || acct.auth_health.is_expired()),
        "the running watch held the ended session as ended ({} checks)",
        fake.checks() - before
    );
    let took = started.elapsed();
    std::thread::sleep(PERIOD * 6);
    let checks: Vec<Instant> = fake.seen.lock().unwrap()[before..].iter().map(|seen| seen.at).collect();
    assert_eq!(
        checks.len(),
        3,
        "three checks made the run, and none came after it ({took:?})"
    );
    assert!(
        checks[0].duration_since(started) >= PERIOD,
        "the first check waits one period after the start"
    );
    println!(
        "held as ended {took:?} after the start, after {} checks at a {PERIOD:?} period",
        checks.len()
    );
}

/// Rule 2. Offline, a 5xx and a suspended account's 403 (decision 2531) never count: they neither add to the run nor
/// clear it. Two refusals are in the run; after every one of those answers it is still two, so the next refusal is
/// the third. (A timeout is the same kind of failure as a hang-up, an answer with no status, and is not waited for
/// here: the client's own timeout is 30 s.)
#[test]
fn the_watch_never_counts_offline_or_5xx() {
    let rt = runtime();
    let fake = Fake::start(Me::Unavailable);
    let acct = account();
    sign_in(&acct, TOKEN);
    let watch = fake.watch(&acct, LONG);
    for _ in 0..2 {
        acct.auth_health.note_watch_answer(Some(&refused()));
    }
    assert!(
        !acct.auth_health.is_expired(),
        "precondition: two refusals are not yet the run"
    );

    for me in [Me::Unavailable, Me::Suspended, Me::HangsUp] {
        fake.set_answer(me);
        for _ in 0..3 {
            assert_eq!(rt.block_on(watch.step()), Step::NoAnswer, "{me:?}");
        }
        assert!(!acct.auth_health.is_expired(), "{me:?} added to the run");
    }
    assert!(
        fake.checks() >= 9,
        "the instrument: every step asked the fake ({})",
        fake.checks()
    );

    // No server at all: nothing listens on the discard port.
    let offline = SessionWatch::new(acct.clone(), "http://127.0.0.1:9".to_string(), LONG);
    for _ in 0..3 {
        assert_eq!(rt.block_on(offline.step()), Step::NoAnswer, "offline");
    }
    assert!(!acct.auth_health.is_expired(), "offline added to the run");

    // The run was kept, not cleared: one more refusal is the third.
    fake.set_answer(Me::Refuses);
    assert_eq!(rt.block_on(watch.step()), Step::Refused);
    assert!(
        acct.auth_health.is_expired(),
        "the two refusals were still in the run: none of those answers cleared it"
    );
}

/// Rule 3. A feed of the engine's own (a tick, a beat, a call that worked) within the period means the watch asks
/// nothing; once that feed is older than the period, it asks again. M2 (the watch ignores `fed_within`): the fed steps
/// ask, and the fake counts them.
#[test]
fn the_watch_is_quiet_when_a_tick_fed_the_count() {
    let rt = runtime();
    let fake = Fake::start(Me::Accepts);
    let acct = account();
    sign_in(&acct, TOKEN);

    // A tick whose pull came back, as the runner notes it (`note_tick` stamps and clears the same way).
    acct.auth_health.note_result(None);
    let quiet = fake.watch(&acct, LONG);
    for _ in 0..3 {
        assert_eq!(rt.block_on(quiet.step()), Step::Fed);
    }
    assert_eq!(fake.checks(), 0, "a fed run asks nothing (M2)");

    // The same feed, now older than a short period: the watch asks.
    let short = fake.watch(&acct, PERIOD);
    std::thread::sleep(PERIOD * 3);
    assert_eq!(rt.block_on(short.step()), Step::Accepted);
    assert_eq!(
        fake.checks(),
        1,
        "a feed older than the period no longer keeps the watch quiet"
    );
}

/// Rule 4. A slow link: an answer that takes longer than the paused tick's 2 s cutoff still counts (M3: the check
/// wrapped in a 2 s timeout gives up first, and the run stays at two). A stop ends a check in flight at once: a
/// session that moved on (a lock) drops it, and so does the engine's stop (its runtime dropped); nothing is counted
/// for either, also when the answer comes later.
#[test]
fn a_slow_answer_still_counts_and_a_stop_ends_the_check() {
    let rt = runtime();
    let fake = Fake::start(Me::Refuses);

    // A refusal after 5.5 s is the third of the run.
    let acct = account();
    sign_in(&acct, TOKEN);
    for _ in 0..2 {
        acct.auth_health.note_watch_answer(Some(&refused()));
    }
    fake.set_delay(SLOW);
    let started = Instant::now();
    assert_eq!(
        rt.block_on(fake.watch(&acct, LONG).step()),
        Step::Refused,
        "the slow answer was read"
    );
    assert!(
        started.elapsed() >= Duration::from_secs(5),
        "the instrument: the answer took {:?}",
        started.elapsed()
    );
    assert!(acct.auth_health.is_expired(), "the slow refusal counted (M3)");

    // A lock while the check is in flight: the step ends at once and counts nothing.
    fake.set_delay(Duration::from_secs(50));
    let acct = account();
    sign_in(&acct, TOKEN);
    for _ in 0..2 {
        acct.auth_health.note_watch_answer(Some(&refused()));
    }
    let watch = Arc::new(fake.watch(&acct, LONG));
    let before = fake.checks();
    let stepping = {
        let watch = watch.clone();
        rt.spawn(async move { watch.step().await })
    };
    assert!(
        wait_until(Duration::from_secs(10), || fake.checks() > before),
        "the check reached the fake"
    );
    let locked = Instant::now();
    take_out(&acct, SessionTransition::Lock);
    let ended = rt.block_on(async { tokio::time::timeout(Duration::from_secs(2), stepping).await });
    let took = locked.elapsed();
    assert_eq!(
        ended.ok().and_then(Result::ok),
        Some(Step::Moved),
        "the check ended when the session moved on, not when the answer came ({took:?})"
    );
    assert!(
        !acct.auth_health.is_expired(),
        "an answer for a session that moved on counts nothing"
    );

    // The engine's stop drops its runtime: the watch, and the check in flight with it, end at once.
    let acct = account();
    sign_in(&acct, TOKEN);
    for _ in 0..2 {
        acct.auth_health.note_watch_answer(Some(&refused()));
    }
    let engine_runtime = runtime();
    let before = fake.checks();
    let _running = engine_runtime.spawn(fake.watch(&acct, PERIOD).run());
    assert!(
        wait_until(Duration::from_secs(10), || fake.checks() > before),
        "the running watch's check reached the fake"
    );
    let stopping = Instant::now();
    drop(engine_runtime);
    let took = stopping.elapsed();
    assert!(took < Duration::from_secs(2), "the stop waited for the check: {took:?}");
    // The fake answers the dropped checks now (this one and the locked session's): nobody reads them.
    fake.set_delay(Duration::ZERO);
    std::thread::sleep(PERIOD * 6);
    assert!(!acct.auth_health.is_expired(), "the stopped watch counted nothing");
    assert_eq!(fake.checks(), before + 1, "and asked nothing more");
}

/// Rule 5. Signed out or with the vault closed (no session in memory), the watch asks nothing, at once or running.
/// The instrument first: with a session in memory the same watch asks.
#[test]
fn the_watch_checks_nothing_while_locked() {
    let rt = runtime();
    let fake = Fake::start(Me::Accepts);
    let acct = account();
    sign_in(&acct, TOKEN);
    let watch = fake.watch(&acct, LONG);
    assert_eq!(
        rt.block_on(watch.step()),
        Step::Accepted,
        "the instrument: with a session the watch asks"
    );
    assert_eq!(fake.checks(), 1);

    take_out(&acct, SessionTransition::Lock);
    assert_eq!(rt.block_on(watch.step()), Step::NoSession, "the vault is closed");
    sign_in(&acct, TOKEN);
    take_out(&acct, SessionTransition::SignOut);
    assert_eq!(rt.block_on(watch.step()), Step::NoSession, "signed out");

    let _running = rt.spawn(fake.watch(&acct, PERIOD).run());
    std::thread::sleep(PERIOD * 8);
    assert_eq!(fake.checks(), 1, "no session in memory: nothing was asked");
}

/// The engine starts ONE watch, in `Engine::open` (where its runtime is made), at the real period; never one per
/// sign-in.
#[test]
fn the_engine_starts_one_watch_when_it_opens() {
    let app = include_str!("../app.rs").replace("\r\n", "\n");
    assert_eq!(
        app.matches("session_watch::start(").count(),
        1,
        "app.rs starts the watch once"
    );
    let open = app.find("    pub fn open(").expect("Engine::open");
    let next = app[open + 1..]
        .find("\n    pub fn ")
        .map_or(app.len(), |at| open + 1 + at);
    assert!(
        app[open..next].contains("session_watch::start(&runtime, &acct, &config.api_base);"),
        "the start is in Engine::open"
    );
    let watch = include_str!("session_watch.rs").replace("\r\n", "\n");
    assert_eq!(watch.matches(".spawn(").count(), 1, "one task per start");
    assert_eq!(SESSION_WATCH_PERIOD, Duration::from_secs(30));
}
