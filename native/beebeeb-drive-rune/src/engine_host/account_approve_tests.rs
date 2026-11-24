//! Approving another device's sign-in through the window's source (`EngineAccountSource`, slice A5) on the real
//! engine: a real sign-in and unlock against the engine's fake account on loopback, the approval routes added here,
//! and a scripted Windows Hello. What is (not) sent, when, and which answers reach the window. Every name, code and
//! password is invented.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use beebeeb_engine::app::EngineConfig;
use beebeeb_engine::host::ChannelHost;
use beebeeb_engine::identity::TEST;
use beebeeb_engine::signin::memory_store::MemoryCredentials;
use beebeeb_engine::test_api::fake_account::{FakeAccount, TEST_PHRASE, account_handler};
use beebeeb_engine::test_api::{ConfirmTokens, FakeApi, Reply, Request};
use serde_json::json;

use super::*;
use crate::engine_host::hello::HelloDone;
use crate::frames::account::approve::{
    BOARD_REQUEST_CODE, CANCELLED_REQUEST_CODE, DROPPED_REQUEST_CODE, EXPIRED_REQUEST_CODE, FOUND_LATER_REQUEST_CODE,
    HANDLED_REQUEST_CODE, OFFLINE_REQUEST_CODE, UNKNOWN_REQUEST_CODE, UNUSUAL_REQUEST_CODE,
};

const PASSWORD: &str = "an invented password for the approval";
const CODE: &str = "K7QM-4WTD";
/// The P-256 generator point, SEC1 uncompressed (65 bytes), in base64: a valid public key (its private key is 1), so
/// core can seal to it. The real requester's key, and opening the relay with it, are the engine's test
/// (`approve_sends_the_master_key_only_sealed`).
const REQUESTER_KEY_B64: &str =
    "BGsX0fLhLEJH+Lzm5WOkQPJ3A32BLeszoPShOUXYmMKWT+NC4v4af5uO5+tKfA+eFivOM1drMV7Oy7ZAaDe/UfU=";
const LIVE: &str = "2999-01-01T00:00:00Z";
const GONE: &str = "2001-01-01T00:00:00Z";

/// One registered account for every test here (a registration runs Argon2id once), and its recovery phrase.
fn account() -> (Arc<FakeAccount>, String) {
    static ACCOUNT: OnceLock<(Arc<FakeAccount>, String)> = OnceLock::new();
    ACCOUNT
        .get_or_init(|| {
            let (account, phrase) = FakeAccount::register("approve-window@example.eu", PASSWORD, TEST_PHRASE);
            (Arc::new(account), phrase)
        })
        .clone()
}

/// The fake API with the approval routes in front of the account's own fake.
struct Server {
    fake: FakeApi,
    account: Arc<FakeAccount>,
    phrase: String,
    /// When the request the code names runs out, as Beebeeb says it.
    expires_at: Arc<Mutex<&'static str>>,
    /// An engine whose screen locks the moment the relay is taken (the app lock, after the keys left).
    lock_at_relay: Arc<Mutex<Option<TestEngine>>>,
    /// The relay is taken and its answer never comes (the connection closes): the keys left, the ending is not known.
    relay_lost: Arc<AtomicBool>,
}

fn server() -> Server {
    let (account, phrase) = account();
    let tokens = Arc::new(ConfirmTokens::default());
    let expires_at = Arc::new(Mutex::new(LIVE));
    let lock_at_relay: Arc<Mutex<Option<TestEngine>>> = Arc::default();
    let (ours, rest, expiry) = (account.clone(), account_handler(account.clone()), expires_at.clone());
    let relay_lock = lock_at_relay.clone();
    let relay_lost: Arc<AtomicBool> = Arc::default();
    let lost = relay_lost.clone();
    let fake = FakeApi::start(Arc::new(move |request: &Request| {
        let route = request.route();
        let approval = route.starts_with("POST /api/v1/auth/confirm-opaque-") || route.contains("/api/v1/auth/cli-");
        if !approval {
            return rest(request);
        }
        if request.bearer() != Some(ours.session_token.as_str()) {
            return Reply::error(401, "unauthorized");
        }
        let body = request.json();
        match route.as_str() {
            "GET /api/v1/auth/cli-pubkey" if request.query("code") == Some(CODE) => Reply::ok(json!({
                "ecdh_public_key_b64": REQUESTER_KEY_B64,
                "request": {
                    "requested_at": "2026-11-15T09:41:12Z",
                    "expires_at": *expiry.lock().unwrap(),
                    "ip": "203.0.113.47",
                    "country": "de",
                    "client": "drive",
                    "client_version": "1.0",
                    "device_name": "An invented phone",
                    "os": "iOS"
                }
            })),
            "POST /api/v1/auth/confirm-opaque-start" => ours.opaque.login_start(&body),
            "POST /api/v1/auth/confirm-opaque-finish" => {
                let bound = body["purpose"] == "cli_device_approval" && body["cli_code"] == CODE;
                if ours.opaque.login_finish(&body) && bound {
                    Reply::ok(json!({ "confirmation_token": tokens.mint(), "expires_at": "soon" }))
                } else {
                    Reply::error(401, "invalid_credentials")
                }
            }
            "POST /api/v1/auth/cli-session" if body["user_code"] == CODE && tokens.spend(request) => {
                Reply::ok(json!({ "session_token": "session-for-the-other-device" }))
            }
            "POST /api/v1/auth/cli-session" => Reply::error(403, "step_up_required"),
            "POST /api/v1/auth/cli-authorize" if body["user_code"] == CODE => {
                if let Some(engine) = relay_lock.lock().unwrap().take() {
                    engine.lock_screen().expect("the screen locks");
                }
                if lost.load(Ordering::SeqCst) {
                    return Reply::hang_up();
                }
                Reply::ok(json!({ "ok": true }))
            }
            "POST /api/v1/auth/cli-deny" if body["user_code"] == CODE => Reply::ok(json!({ "denied": true })),
            _ => Reply::error(404, "not_found"),
        }
    }));
    Server {
        fake,
        account,
        phrase,
        expires_at,
        lock_at_relay,
        relay_lost,
    }
}

type TestEngine = Arc<Engine<MemoryCredentials>>;
type Source = EngineAccountSource<MemoryCredentials>;

/// A handle of its own on the fake, signed in and unlocked.
fn signed_in(server: &Server) -> TestEngine {
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "approve-test-slot".into(),
        state_dir: crate::engine_host::tests::process_state_dir(),
        api_base: server.fake.base.clone(),
    };
    let engine = Engine::open(&TEST, config, host, MemoryCredentials::default()).expect("the engine opens");
    engine.restore();
    engine
        .sign_in_password(&server.account.email, Zeroizing::new(PASSWORD.into()))
        .expect("the fake accepts the password");
    engine
        .unlock_with_phrase(Zeroizing::new(server.phrase.clone()))
        .expect("the phrase unlocks");
    Arc::new(engine)
}

/// A scripted Windows Hello: every prompt waits until the test answers it.
#[derive(Clone, Default)]
struct Prompts {
    up: Arc<Mutex<Vec<HelloDone>>>,
    asked: Arc<AtomicUsize>,
    taken_away: Arc<AtomicUsize>,
}

impl HelloPrompter for Prompts {
    fn start(&self, done: HelloDone) -> Result<HelloCancel, ()> {
        self.asked.fetch_add(1, Ordering::SeqCst);
        self.up.lock().unwrap().push(done);
        let taken_away = self.taken_away.clone();
        Ok(Box::new(move || {
            taken_away.fetch_add(1, Ordering::SeqCst);
        }))
    }
}

impl Prompts {
    fn asked(&self) -> usize {
        self.asked.load(Ordering::SeqCst)
    }

    fn taken_away(&self) -> usize {
        self.taken_away.load(Ordering::SeqCst)
    }

    /// The oldest prompt ends with `result` (on the prompt's own thread, in the app).
    fn answer(&self, result: HelloResult) {
        let done = self.up.lock().unwrap().remove(0);
        done(result);
    }
}

fn with_prompts(server: &Server) -> (TestEngine, Source, Prompts) {
    let engine = signed_in(server);
    let prompts = Prompts::default();
    let source = Source::new(engine.clone()).with_approval_hello(Box::new(prompts.clone()));
    (engine, source, prompts)
}

/// The request the code names, as the review shows it. Typed as a person types it.
fn found(source: &mut Source) -> SignInRequest {
    match source.find_sign_in_request("k7qm 4wtd") {
        Lookup::Found(request) => request,
        other => panic!("the code's request is found: {other:?}"),
    }
}

fn password() -> Password<'static> {
    Password::from_field(PASSWORD)
}

/// The next ending, waited for (at most 10 s).
fn answer_of(source: &mut Source) -> (String, Approval) {
    let began = Instant::now();
    loop {
        if let Some(answer) = source.approval_answer() {
            return answer;
        }
        assert!(began.elapsed() < Duration::from_secs(10), "no answer came");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Whether a late approval's toast is in the source's line (said, for any turn).
fn said_late(source: &Source) -> bool {
    source
        .notices
        .iter()
        .any(|(_, said)| said.words == APPROVED_LATE || said.words == UNKNOWN_LATE)
}

/// Approve with Windows Hello verified, while the screen locks the moment the relay is taken: the keys left, and the
/// ending comes under the locked turn. Taken in until it is kept (at most 10 s); every look hands nothing out.
fn approve_with_a_lock_at_the_relay(server: &Server, engine: &TestEngine, source: &mut Source, prompts: &Prompts) {
    let request = found(source);
    source.begin_approval(request.id(), CODE, password());
    *server.lock_at_relay.lock().unwrap() = Some(engine.clone());
    prompts.answer(HelloResult::Verified);
    let began = Instant::now();
    while source.account_approve.late.is_none() {
        assert_eq!(source.approval_answer(), None, "the wait of the old turn is over");
        assert!(began.elapsed() < Duration::from_secs(10), "the ending never came");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(asked(server, "POST /api/v1/auth/cli-authorize"), 1, "the keys left");
}

fn asked(server: &Server, route: &str) -> usize {
    server.fake.routes().iter().filter(|r| r.as_str() == route).count()
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && haystack.windows(needle.len()).any(|window| window == needle)
}

#[test]
fn the_approval_pages_answer_while_signed_in() {
    let server = server();
    let (engine, source, _) = with_prompts(&server);
    assert_eq!(source.pane_state(Pane::ApproveCode), PaneState::Ready);
    assert_eq!(source.pane_state(Pane::ApproveReview), PaneState::Ready);
    let _ = engine.lock();
    assert_eq!(source.pane_state(Pane::ApproveCode), PaneState::SignedOut);
}

/// The whole way through the window's source: the lookup (once per code), Windows Hello first, then the approval.
#[test]
fn a_verified_approval_reaches_the_other_device() {
    let server = server();
    let (_engine, mut source, prompts) = with_prompts(&server);
    let request = found(&mut source);
    assert_eq!(
        (request.product(), request.app(), request.country(), request.address()),
        ("Drive", "Drive 1.0", "DE", "203.0.113.47")
    );
    assert!(!request.unusual_place(), "Beebeeb does not say so: never claimed");
    assert!(
        !request.id().contains("K7QM") && !request.id().contains("4WTD"),
        "the id is this PC's own, never the code"
    );
    // The same code again (the review asks on every edit): answered from what is held.
    assert!(matches!(source.find_sign_in_request(CODE), Lookup::Found(again) if again.id() == request.id()));
    assert_eq!(asked(&server, "GET /api/v1/auth/cli-pubkey"), 1, "one lookup per code");

    source.begin_approval(request.id(), CODE, password());
    assert_eq!(prompts.asked(), 1, "Windows Hello first");
    assert_eq!(source.approval_answer(), None, "nothing before Windows answers");
    assert_eq!(
        asked(&server, "POST /api/v1/auth/confirm-opaque-start"),
        0,
        "and nothing is sent before it"
    );
    prompts.answer(HelloResult::Verified);
    assert_eq!(answer_of(&mut source), (request.id().to_owned(), Approval::Approved));
    assert_eq!(asked(&server, "POST /api/v1/auth/cli-session"), 1);
    assert_eq!(asked(&server, "POST /api/v1/auth/cli-authorize"), 1);
    // A Deny after it: this PC's approval got there first.
    assert_eq!(source.deny_sign_in(request.id()), Denial::ApprovedHere);
    assert_eq!(asked(&server, "POST /api/v1/auth/cli-deny"), 0);
}

/// Rule 2 of slice A5: Windows Hello cancelled, failed or not set up, or no prompt at all: nothing is claimed.
#[test]
fn approve_refuses_without_hello() {
    let server = server();
    let (engine, mut source, prompts) = with_prompts(&server);
    let request = found(&mut source);
    let id = request.id().to_owned();

    source.begin_approval(&id, CODE, password());
    prompts.answer(HelloResult::Cancelled);
    assert_eq!(answer_of(&mut source), (id.clone(), Approval::Cancelled));

    source.begin_approval(&id, CODE, password());
    prompts.answer(HelloResult::Failed);
    assert_eq!(answer_of(&mut source), (id.clone(), Approval::Cancelled));

    source.begin_approval(&id, CODE, password());
    prompts.answer(HelloResult::Unavailable);
    assert_eq!(answer_of(&mut source), (id.clone(), Approval::NoHello));

    // A source the shell gave no prompt (no window): no approval either.
    let mut bare = Source::new(engine.clone());
    let other = found(&mut bare);
    bare.begin_approval(other.id(), CODE, password());
    assert_eq!(answer_of(&mut bare), (other.id().to_owned(), Approval::NoHello));

    assert_eq!(prompts.asked(), 3);
    assert_eq!(
        asked(&server, "POST /api/v1/auth/confirm-opaque-start"),
        0,
        "nothing was sent"
    );
    assert_eq!(
        asked(&server, "POST /api/v1/auth/cli-session"),
        0,
        "nothing was claimed"
    );
    assert_eq!(asked(&server, "POST /api/v1/auth/cli-authorize"), 0);
}

/// A locked app (app lock on, the screen locked) raises no prompt and sends nothing.
#[test]
fn approve_refuses_when_locked() {
    let server = server();
    let (engine, mut source, prompts) = with_prompts(&server);
    let request = found(&mut source);
    engine.lock_screen().expect("the screen locks");
    assert_eq!(
        engine.sign_in_approval_allowed_for(engine.session_turn()),
        Err(ApproveError::Locked)
    );
    source.begin_approval(request.id(), CODE, password());
    assert_eq!(answer_of(&mut source), (request.id().to_owned(), Approval::Cancelled));
    assert_eq!(prompts.asked(), 0, "no prompt over a locked app");
    assert_eq!(
        asked(&server, "POST /api/v1/auth/confirm-opaque-start"),
        0,
        "nothing was sent"
    );
    assert_eq!(asked(&server, "POST /api/v1/auth/cli-session"), 0);
}

/// An answer that comes under another turn of the session (a lock and an unlock, a sign-out, another sign-in) is
/// dropped, the prompt of the old turn is taken away, and its "verified" starts nothing.
#[test]
fn approval_answer_dropped_after_session_change() {
    let server = server();
    let (engine, mut source, prompts) = with_prompts(&server);

    // The turn moves while the prompt is up; the late "verified" comes after the window looked.
    let request = found(&mut source);
    source.begin_approval(request.id(), CODE, password());
    assert_eq!(prompts.asked(), 1);
    engine.lock_screen().unwrap();
    engine.unlock_screen_platform_verified(engine.session_turn()).unwrap();
    assert_eq!(source.approval_answer(), None, "the wait of the old turn is over");
    assert_eq!(prompts.taken_away(), 1, "and its prompt was taken away");
    prompts.answer(HelloResult::Verified);
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(source.approval_answer(), None, "a late answer is dropped");

    // The "verified" is already in when the turn moves, before the window took it in.
    let request = found(&mut source);
    source.begin_approval(request.id(), CODE, password());
    prompts.answer(HelloResult::Verified);
    engine.lock_screen().unwrap();
    engine.unlock_screen_platform_verified(engine.session_turn()).unwrap();
    assert_eq!(source.approval_answer(), None);
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(source.approval_answer(), None, "nothing was started for the old turn");

    assert_eq!(
        asked(&server, "POST /api/v1/auth/confirm-opaque-start"),
        0,
        "nothing was sent"
    );
    assert_eq!(
        asked(&server, "POST /api/v1/auth/cli-session"),
        0,
        "nothing was claimed"
    );
}

/// A prompt taken away and asked again for the same request: the first prompt's late "verified" is not the second's.
#[test]
fn an_earlier_prompts_answer_is_not_the_next_ones() {
    let server = server();
    let (_engine, mut source, prompts) = with_prompts(&server);
    let request = found(&mut source);
    source.begin_approval(request.id(), CODE, password());
    source.cancel_approval(request.id());
    source.begin_approval(request.id(), CODE, password());
    assert_eq!(prompts.asked(), 2);
    // The first prompt had already decided "verified" when it was taken away.
    prompts.answer(HelloResult::Verified);
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(source.approval_answer(), None, "the second prompt is still up");
    assert_eq!(
        asked(&server, "POST /api/v1/auth/confirm-opaque-start"),
        0,
        "nothing was sent"
    );
    prompts.answer(HelloResult::Cancelled);
    assert_eq!(answer_of(&mut source), (request.id().to_owned(), Approval::Cancelled));
    assert_eq!(asked(&server, "POST /api/v1/auth/cli-session"), 0);
}

/// A Deny ends the request with the code alone: no step-up, no token or key in its body. Denied while Windows Hello
/// is up, the prompt goes first and its late "verified" claims nothing.
#[test]
fn deny_sends_nothing_secret() {
    let server = server();
    let (_engine, mut source, prompts) = with_prompts(&server);
    let request = found(&mut source);
    assert_eq!(source.deny_sign_in(request.id()), Denial::Denied);
    let denials: Vec<Request> = server
        .fake
        .requests()
        .into_iter()
        .filter(|r| r.route() == "POST /api/v1/auth/cli-deny")
        .collect();
    assert_eq!(denials.len(), 1);
    assert_eq!(
        denials[0].json(),
        json!({ "user_code": CODE }),
        "the code and nothing else"
    );
    assert_eq!(
        denials[0].header("x-confirm-token"),
        None,
        "denying grants nothing: no step-up"
    );

    // Denied while the prompt is up (the window cancels it first, then denies).
    let request = found(&mut source);
    source.begin_approval(request.id(), CODE, password());
    source.cancel_approval(request.id());
    assert_eq!(prompts.taken_away(), 1, "the prompt went first");
    assert_eq!(source.deny_sign_in(request.id()), Denial::Denied);
    prompts.answer(HelloResult::Verified);
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(source.approval_answer(), None, "the late \"verified\" starts nothing");

    let (key, token) = (server.account.master_key, server.account.session_token.as_bytes());
    for request in server.fake.requests() {
        let route = request.route();
        assert!(!contains(&request.body, &key), "the key left in {route}");
        assert!(!contains(&request.body, token), "the token in the body of {route}");
        assert!(
            !contains(&request.body, PASSWORD.as_bytes()),
            "the password left in {route}"
        );
    }
    assert_eq!(asked(&server, "POST /api/v1/auth/confirm-opaque-start"), 0);
    assert_eq!(asked(&server, "POST /api/v1/auth/cli-session"), 0);
}

/// DW-A17: a request whose time is up is gone for the review (no network), and no longer waits.
#[test]
fn expired_request_shows_expired() {
    let server = server();
    let (engine, mut source, _) = with_prompts(&server);
    *server.expires_at.lock().unwrap() = GONE;
    let request = found(&mut source);
    // Only the approval's own routes: the signed-in engine's background work (sync, quota) may still be asking the
    // fake for its own routes at this moment, and that is not the review's network.
    let review_routes = |server: &Server| {
        server
            .fake
            .routes()
            .into_iter()
            .filter(|r| r.contains("/api/v1/auth/cli-") || r.contains("/api/v1/auth/confirm-opaque-"))
            .count()
    };
    let before = review_routes(&server);
    assert_eq!(source.sign_in_request_gone(request.id()), Some(Denial::Expired));
    assert_eq!(source.waiting_sign_in_request(request.id()), None);
    assert_eq!(review_routes(&server), before, "read from the request: no network");

    *server.expires_at.lock().unwrap() = LIVE;
    let mut fresh = Source::new(engine.clone());
    let live = found(&mut fresh);
    assert_eq!(fresh.sign_in_request_gone(live.id()), None);
    assert_eq!(
        fresh.waiting_sign_in_request(live.id()).map(|r| r.id().to_owned()),
        Some(live.id().to_owned())
    );
    assert_eq!(
        fresh.sign_in_request_gone("req-not-held"),
        None,
        "an id it never gave out"
    );
}

/// The code is never logged: this slice's log lines are fixed words, the engine's approval has no log call, and the
/// log file's allow-list (the layer that decides, fail closed) holds no word a code can be.
///
/// What this does NOT prove: that no other file logs a code (only this slice's two files are read).
#[test]
fn code_never_logged() {
    let vocabulary: Vec<String> = include_str!("../log_file/vocabulary.txt")
        .lines()
        .chain(include_str!("../log_file/vocabulary_os.txt").lines())
        .filter(|line| !line.starts_with('#'))
        .map(|line| line.trim().to_ascii_lowercase())
        .collect();
    for code in [
        CODE,
        BOARD_REQUEST_CODE,
        EXPIRED_REQUEST_CODE,
        HANDLED_REQUEST_CODE,
        CANCELLED_REQUEST_CODE,
        UNUSUAL_REQUEST_CODE,
        OFFLINE_REQUEST_CODE,
        DROPPED_REQUEST_CODE,
        UNKNOWN_REQUEST_CODE,
        FOUND_LATER_REQUEST_CODE,
    ] {
        let lower = code.to_ascii_lowercase();
        assert!(!vocabulary.contains(&lower), "{code} would be written as it is");
    }
    // A code is eight letters and digits, shown XXXX-XXXX. A word of that shape in the allow-list is a code the log
    // file would write as it is, if one were ever logged. The ones there today are pinned: a new one turns this red.
    let could_be_a_code = |word: &str| {
        let chars: Vec<char> = word.chars().collect();
        chars.len() == 9
            && chars[4] == '-'
            && chars
                .iter()
                .enumerate()
                .all(|(at, c)| at == 4 || c.is_ascii_alphanumeric())
    };
    let codes: Vec<&String> = vocabulary.iter().filter(|w| could_be_a_code(w)).collect();
    assert_eq!(
        codes,
        ["half-made", "keep-mine", "open-time", "read-only", "sync-root"],
        "words of the allow-list in a code's shape"
    );
    // This slice: no tracing call, and no log line with a value in it.
    let slice = include_str!("account_approve.rs");
    let engine_side = include_str!("../../../../crates/beebeeb-engine/src/app/bindings/approve_sign_in.rs");
    for (name, source) in [("account_approve.rs", slice), ("approve_sign_in.rs", engine_side)] {
        assert!(!source.contains("tracing::"), "{name} has a tracing call");
        assert!(!source.contains("log(format!"), "{name} logs a value");
        assert!(!source.contains("eprintln!"), "{name} prints");
        assert!(!source.contains("println!"), "{name} prints");
    }
}

/// Review of #463, P3-4: an approval whose keys left while the app locked (its review ended with the lock) is told
/// after the unlock, once, as the toast over the account page the window shows; nothing is said over the lock.
#[test]
fn an_approval_that_lands_after_a_lock_is_told_after_the_unlock() {
    let server = server();
    let (engine, mut source, prompts) = with_prompts(&server);
    // An account page shows: the approval's own section, where the window lands when a lock ends its flow.
    source.opened(Pane::Security);
    approve_with_a_lock_at_the_relay(&server, &engine, &mut source, &prompts);
    assert_eq!(engine.gate(), GateState::ScreenLocked);
    assert_eq!(source.approval_answer(), None);
    assert!(!said_late(&source), "nothing is said over the lock");

    engine
        .unlock_screen_platform_verified(engine.session_turn())
        .expect("the screen opens");
    assert_eq!(source.approval_answer(), None, "the review it was for is over");
    assert_eq!(source.said().map(|said| said.words), Some(APPROVED_LATE));
    assert!(source.account_approve.late.is_none(), "said once");
}

/// The kept ending is the account's: a closed vault (or a sign-out, or another account) wipes it unsaid.
#[test]
fn a_late_approval_is_never_told_after_the_session_ended() {
    let server = server();
    let (engine, mut source, prompts) = with_prompts(&server);
    source.opened(Pane::Security);
    approve_with_a_lock_at_the_relay(&server, &engine, &mut source, &prompts);
    let _ = engine.lock_vault();
    assert_eq!(source.approval_answer(), None);
    assert!(source.account_approve.late.is_none(), "wiped with the session");
    assert!(!said_late(&source));

    // The rule, on its own: the same named account with its session open, and nothing else.
    let now = stamp(&engine);
    let mine: Stamp = (now.0, Some("approve-window@example.eu".to_owned()));
    let another: Stamp = (now.0, Some("another-account@example.eu".to_owned()));
    let unnamed: Stamp = (now.0, None);
    assert!(keeps_late(&mine, &mine, true));
    assert!(!keeps_late(&mine, &another, true), "another account");
    assert!(!keeps_late(&mine, &mine, false), "a session that ended");
    assert!(!keeps_late(&unnamed, &unnamed, true), "no account named");
}

/// Review of #463, P3-5: a retry after an approval that did not reach the claim asks Beebeeb for the code's request
/// again, and goes on only when it is the request the review shows. Another one ends the review: no prompt, nothing
/// sent.
#[test]
fn a_retry_approves_only_the_request_the_review_shows() {
    let server = server();
    let (_engine, mut source, prompts) = with_prompts(&server);
    let request = found(&mut source);
    let id = request.id().to_owned();
    let wrong = || Password::from_field("not the account's password");

    // A wrong password: nothing was claimed, the request stays for another try.
    source.begin_approval(&id, CODE, wrong());
    prompts.answer(HelloResult::Verified);
    assert_eq!(answer_of(&mut source), (id.clone(), Approval::WrongPassword));

    // The retry: Beebeeb is asked again, says the same, and Windows Hello is asked.
    source.begin_approval(&id, CODE, wrong());
    assert_eq!(asked(&server, "GET /api/v1/auth/cli-pubkey"), 2, "asked again");
    assert_eq!(prompts.asked(), 2, "the request on screen: Windows Hello is asked");
    prompts.answer(HelloResult::Verified);
    assert_eq!(answer_of(&mut source), (id.clone(), Approval::WrongPassword));
    let started = asked(&server, "POST /api/v1/auth/confirm-opaque-start");

    // Beebeeb now says something else about the code's request (another expiry): not the one on screen.
    *server.expires_at.lock().unwrap() = "2998-01-01T00:00:00Z";
    source.begin_approval(&id, CODE, password());
    assert_eq!(answer_of(&mut source), (id.clone(), Approval::Expired));
    assert_eq!(asked(&server, "GET /api/v1/auth/cli-pubkey"), 3);
    assert_eq!(prompts.asked(), 2, "no prompt for it");
    assert_eq!(
        asked(&server, "POST /api/v1/auth/confirm-opaque-start"),
        started,
        "nothing was sent for it"
    );
    assert_eq!(
        asked(&server, "POST /api/v1/auth/cli-session"),
        0,
        "nothing was claimed"
    );
    assert!(source.waiting_sign_in_request(&id).is_none(), "the review ended");
}

/// The window's own order: the review page shows while the approval runs (no account page is `showing`), and the
/// screen lock took the window off the account pages. After the unlock nothing is said until the frame, having ended
/// the flow, clears the toasts and opens the section's page: said then, never into a line about to be cleared.
#[test]
fn a_late_approval_waits_for_the_account_page() {
    let server = server();
    let (engine, mut source, prompts) = with_prompts(&server);
    source.opened(Pane::ApproveReview);
    approve_with_a_lock_at_the_relay(&server, &engine, &mut source, &prompts);
    engine
        .unlock_screen_platform_verified(engine.session_turn())
        .expect("the screen opens");
    assert_eq!(source.approval_answer(), None);
    assert!(!said_late(&source), "no account page shows yet: nothing said");
    assert!(source.account_approve.late.is_some(), "still kept");

    // The frame ends the approval's flow and lands on the section's page (`end_flows`, then its poll).
    source.clear_notice();
    source.opened(Pane::Security);
    assert_eq!(source.approval_answer(), None);
    assert_eq!(source.said().map(|said| said.words), Some(APPROVED_LATE));
}

/// The relay left under the lock and no answer came: told after the unlock as not confirmed, drawn without the mark
/// of a thing that worked.
#[test]
fn a_late_approval_not_known_is_told_as_not_confirmed() {
    let server = server();
    let (engine, mut source, prompts) = with_prompts(&server);
    source.opened(Pane::Security);
    server.relay_lost.store(true, Ordering::SeqCst);
    approve_with_a_lock_at_the_relay(&server, &engine, &mut source, &prompts);
    assert!(matches!(source.account_approve.late, Some((_, Approval::Unknown))));
    engine
        .unlock_screen_platform_verified(engine.session_turn())
        .expect("the screen opens");
    assert_eq!(source.approval_answer(), None);
    let said = source.said().expect("said after the unlock");
    assert_eq!(said.words, UNKNOWN_LATE);
    assert!(said.failed, "never drawn as a thing that worked");
}
