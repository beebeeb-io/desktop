//! Wave 3: a device sign-in through the fake server (`bb-fake-server`), the way a native check on the build VM drives it.
//!
//! The engine asks to be signed in by another device (DW-A07 to DW-A10) on the fake's own address, exactly as a TEST build
//! started with `--api-base http://127.0.0.1:<port>` does: the socket, the code, the wait. The approving device is the
//! fake's second device, driven with one HTTP call as a check script drives it. Nothing in these tests gives the engine the
//! password or the master key: the key it ends with came sealed to its own request.
//!
//! The TEST identity is refused on macOS (`EngineIdentity::refusal_on`), so this file is not compiled there.

use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use beebeeb_engine::app::bindings::device_request::{
    ANOTHER_DEVICE, DeviceRequestEvent, DeviceRequestRefused, DeviceSignedIn,
};
use beebeeb_engine::app::{Engine, EngineConfig};
use beebeeb_engine::host::ChannelHost;
use beebeeb_engine::identity::TEST;
use beebeeb_engine::signin::gate::GateState;
use beebeeb_engine::signin::memory_store::MemoryCredentials;
use beebeeb_engine::test_api::FakeApi;
use beebeeb_engine::test_api::fake_account::FakeAccount;
use beebeeb_engine::test_api::server::{self, FIXTURE_PASSWORD, FIXTURE_USER_ID, Switches};
use serde_json::{Value, json};

use super::test_state_dir;

/// The second device approves (control route, no token; `server::DEVICE_APPROVE_ROUTE`).
const APPROVE: &str = "POST /fake-control/device-approve";
/// How many requests wait (`server::DEVICE_REQUESTS_ROUTE`).
const WAITING: &str = "GET /fake-control/device-requests";
/// The longest one step waits for the other side.
const WAIT: Duration = Duration::from_secs(30);

/// One engine test at a time: they share the process's one state folder (each keeps its own local database).
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

fn fixture() -> Arc<FakeAccount> {
    static ACCOUNT: OnceLock<Arc<FakeAccount>> = OnceLock::new();
    ACCOUNT.get_or_init(server::fixture_account).clone()
}

/// The fixture account served on loopback the way `bb-fake-server` serves it, socket included; its access lines.
fn start_server() -> (FakeApi, Arc<Mutex<Vec<String>>>) {
    let lines = Arc::new(Mutex::new(Vec::<String>::new()));
    let sink = lines.clone();
    let listener = server::bind(server::loopback_listen("127.0.0.1:0").unwrap()).unwrap();
    let fake = server::start_fixture(
        listener,
        fixture(),
        Arc::new(move |line: &str| sink.lock().unwrap().push(line.to_string())),
        Arc::new(AtomicU64::new(0)),
        Switches::default(),
    )
    .unwrap();
    (fake, lines)
}

/// One plain HTTP/1.1 call, as a check script makes it: the status (`0`: no answer) and the JSON body.
fn control(base: &str, route: &str, body: Value) -> (u16, Value) {
    use std::io::{Read, Write};
    let (method, path) = route.split_once(' ').unwrap();
    let host = base.trim_start_matches("http://");
    let mut stream = std::net::TcpStream::connect(host).unwrap();
    let body = body.to_string();
    let head = format!(
        "{method} {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).unwrap();
    stream.write_all(body.as_bytes()).unwrap();
    let mut answer = Vec::new();
    let _ = stream.read_to_end(&mut answer);
    let text = String::from_utf8_lossy(&answer).to_string();
    let Some(status) = text.split_whitespace().nth(1).and_then(|s| s.parse::<u16>().ok()) else {
        return (0, Value::Null);
    };
    let body = text.split_once("\r\n\r\n").map_or("", |(_, b)| b);
    (status, serde_json::from_str(body).unwrap_or(Value::Null))
}

/// An engine on the fake's address, signed out, with a local database of its own.
fn engine_on(base: &str, name: &str) -> (Arc<Engine<MemoryCredentials>>, tempfile::TempDir) {
    let config = EngineConfig {
        account_id: format!("fake-relay-{name}"),
        state_dir: test_state_dir(),
        api_base: base.to_string(),
    };
    let (host, _messages) = ChannelHost::new(|| {});
    let engine = Arc::new(Engine::open(&TEST, config, host, MemoryCredentials::default()).expect("engine opens"));
    let local = tempfile::tempdir().unwrap();
    engine.use_local_state_for_test(local.path().join("state.db"));
    assert_eq!(engine.restore(), GateState::SignedOut);
    (engine, local)
}

/// A request running on a thread of its own, as the window's worker runs it.
struct Running {
    events: mpsc::Receiver<DeviceRequestEvent>,
    done: mpsc::Receiver<Result<DeviceSignedIn, DeviceRequestRefused>>,
    _stop: Arc<AtomicBool>,
}

fn ask(engine: &Arc<Engine<MemoryCredentials>>) -> Running {
    let turn = engine.session_turn();
    let (events_tx, events) = mpsc::channel();
    let (done_tx, done) = mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    let (engine, stop_in) = (engine.clone(), stop.clone());
    std::thread::spawn(move || {
        let result = engine.device_request_for(
            turn,
            move |event| {
                let _ = events_tx.send(event);
            },
            stop_in,
        );
        let _ = done_tx.send(result);
    });
    Running {
        events,
        done,
        _stop: stop,
    }
}

/// The code the engine was shown (the first thing a request hears), and the address it was given with it.
fn the_code(running: &Running) -> (String, String) {
    match running.events.recv_timeout(WAIT) {
        Ok(DeviceRequestEvent::Code {
            code, verification_uri, ..
        }) => (code, verification_uri),
        other => panic!(
            "no code reached the engine ({other:?}); the request ended {:?}",
            running.done.try_recv()
        ),
    }
}

fn waiting(base: &str) -> u64 {
    let (status, body) = control(base, WAITING, Value::Null);
    assert_eq!(status, 200, "{body}");
    body["waiting"].as_u64().expect("a count")
}

#[test]
fn a_device_request_on_the_fake_is_approved_by_the_second_device_and_the_engine_opens_the_sealed_key() {
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let (fake, lines) = start_server();
    let (engine, _local) = engine_on(&fake.base, "approve");
    let running = ask(&engine);

    let (code, address) = the_code(&running);
    let shape =
        code.len() == 9 && code.as_bytes()[4] == b'-' && code.chars().all(|c| c == '-' || c.is_ascii_alphanumeric());
    assert!(shape, "a code of the server's shape");
    assert!(
        !address.contains(&code) && !address.contains(&code.replace('-', "")),
        "the address holds no code"
    );
    assert_eq!(waiting(&fake.base), 1);

    // The second device approves the code this PC shows: one HTTP call.
    let (status, approved) = control(&fake.base, APPROVE, json!({ "code": code }));
    assert_eq!(
        (status, approved["approved"].as_bool()),
        (200, Some(true)),
        "{approved}"
    );

    let ended = running.done.recv_timeout(WAIT).expect("the request ended");
    assert_eq!(
        ended,
        Ok(DeviceSignedIn {
            by: ANOTHER_DEVICE.to_string(),
            vault_unlocked: true,
        })
    );
    // Open, on the fixture's account, with no password and no phrase given to it.
    assert_eq!(engine.gate(), GateState::Ready);
    assert_eq!(engine.user_id().as_deref(), Some(FIXTURE_USER_ID));
    assert_eq!(waiting(&fake.base), 0, "the code is gone once relayed");

    let written = lines.lock().unwrap().clone();
    assert!(
        written.iter().any(|line| line == "GET /api/v1/auth/cli -> 101"),
        "the socket's access line: {written:?}"
    );
    let secrets = [
        code.as_str(),
        &code.replace('-', ""),
        &fixture().session_token,
        FIXTURE_PASSWORD,
    ];
    let leaking = written
        .iter()
        .filter(|line| secrets.iter().any(|s| line.contains(s)))
        .count();
    assert_eq!(
        leaking, 0,
        "a code, a token or the password in the access lines: {written:?}"
    );
}

#[test]
fn a_wrong_approver_is_refused_and_the_request_waits_for_the_right_one() {
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let (fake, _lines) = start_server();
    let (engine, _local) = engine_on(&fake.base, "wrong-approver");
    let running = ask(&engine);
    let (code, _) = the_code(&running);

    // Another account, signed in on the second device: it is shown the request, but its claim and its relay are refused.
    let (status, refused) = control(&fake.base, APPROVE, json!({ "code": code, "approver": "stranger" }));
    assert_eq!((status, refused["approved"].as_bool()), (403, Some(false)), "{refused}");
    // A code nobody was shown (never issued: `0` is not in the codes' alphabet).
    let (status, unknown) = control(&fake.base, APPROVE, json!({ "code": "0000-0000" }));
    assert_eq!(status, 404, "{unknown}");
    // Nothing reached this PC and nothing was burnt: the request still waits.
    std::thread::sleep(Duration::from_millis(300));
    assert!(running.done.try_recv().is_err(), "the request is still open");
    assert_eq!(engine.gate(), GateState::SignedOut);
    assert_eq!(waiting(&fake.base), 1);

    // The right approver, without naming the code (the one waiting request).
    let (status, approved) = control(&fake.base, APPROVE, json!({}));
    assert_eq!(
        (status, approved["approved"].as_bool()),
        (200, Some(true)),
        "{approved}"
    );
    let ended = running.done.recv_timeout(WAIT).expect("the request ended");
    assert!(ended.is_ok(), "signed in: {ended:?}");
    assert_eq!(engine.gate(), GateState::Ready);
}
