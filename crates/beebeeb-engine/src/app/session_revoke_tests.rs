//! The server-side logout of a sign-out ([`revoke`], task 1886): it gives up after [`REVOKE_BOUND`], the server hears
//! it only once the stored session is gone, nothing the server does changes the sign-out, and the token reaches no
//! log line (the meaning of the Tauri `sign_out_revoke_tests.rs`). The server is a `TcpListener` on loopback per test:
//! it never answers, answers with a fixed status, closes each connection at once, or is not there. The sign-out runs
//! on a real handle with an invented session in [`MemoryCredentials`]. Every token is invented.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::{REVOKE_BOUND, revoke};
use crate::account::SessionTransition;
use crate::app::{Engine, EngineConfig};
use crate::host::ChannelHost;
use crate::identity::TEST;
use crate::keychain::{AuthSecretStore, SessionToken};
use crate::session::Session;
use crate::signin::memory_store::MemoryCredentials;

type TestEngine = Engine<MemoryCredentials>;

const SLOT: &str = "an-invented-revoke-slot";

/// The test's own limit on one revoke: well under the client's 30 s, so a revoke without its bound fails here
/// instead of hanging the run.
const GUARD: Duration = Duration::from_secs(20);

/// A revoke ended by its bound, and a sign-out with it, ends under this (the Tauri test's window: 3 s <= took < 6 s).
const WITHIN: Duration = Duration::from_secs(6);

/// What the test server does with each connection.
#[derive(Clone, Copy, Debug)]
enum Server {
    /// Reads the request and keeps the connection open without a word.
    NeverAnswers,
    /// Reads the request and answers with this status, no body.
    Answers(u16),
    /// Closes each connection the moment it is accepted.
    Closes,
    /// Nothing listens on the port.
    Absent,
}

/// Every server of rule 4.
const EVERY_SERVER: [Server; 6] = [
    Server::Answers(200),
    Server::Answers(401),
    Server::Answers(500),
    Server::NeverAnswers,
    Server::Closes,
    Server::Absent,
];

/// One request the server read up to its blank line.
#[derive(Clone)]
struct Heard {
    /// For example `POST /api/v1/auth/logout HTTP/1.1`.
    request_line: String,
    /// The `Authorization: Bearer …` value, if any.
    bearer: Option<String>,
    /// Whether the credential store held a session token when the request was read (`None`: no store watched).
    store_held_a_token: Option<bool>,
}

/// The head of an answer after its status line: no body, and the connection ends with it.
const ANSWER_TAIL: &str = "Content-Length: 0\r\nConnection: close\r\n\r\n";

/// "Does the store hold a session token now", asked on the server's thread the moment a request is read.
type StoreProbe = Box<dyn Fn() -> bool + Send>;

struct TestServer {
    base: String,
    heard: Arc<Mutex<Vec<Heard>>>,
    accepted: Arc<AtomicUsize>,
}

impl TestServer {
    /// A server on a loopback port of its own. Its thread lives as long as the test process, so the connections of
    /// [`Server::NeverAnswers`] stay open for the whole test and are never reset.
    fn start(behaviour: Server, store: Option<StoreProbe>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback port");
        let base = format!("http://{}", listener.local_addr().expect("its address"));
        let heard: Arc<Mutex<Vec<Heard>>> = Arc::default();
        let accepted: Arc<AtomicUsize> = Arc::default();
        let server = Self {
            base,
            heard: heard.clone(),
            accepted: accepted.clone(),
        };
        if let Server::Absent = behaviour {
            drop(listener);
            return server;
        }
        std::thread::spawn(move || {
            let mut held: Vec<TcpStream> = Vec::new();
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                accepted.fetch_add(1, Ordering::SeqCst);
                if let Server::Closes = behaviour {
                    drop(stream);
                    continue;
                }
                let head = read_head(&mut stream);
                // Asked the moment the request is read, before any answer: rule 3's order.
                let store_held_a_token = store.as_ref().map(|probe| probe());
                heard.lock().unwrap().push(heard_of(&head, store_held_a_token));
                match behaviour {
                    Server::Answers(status) => {
                        let answer = format!("HTTP/1.1 {status} Invented\r\n{ANSWER_TAIL}");
                        let _ = stream.write_all(answer.as_bytes());
                        let _ = stream.flush();
                    }
                    Server::NeverAnswers => held.push(stream),
                    Server::Closes | Server::Absent => {}
                }
            }
        });
        server
    }

    /// The requests the server read, after waiting up to `wait` for the first one.
    fn heard_within(&self, wait: Duration) -> Vec<Heard> {
        let deadline = Instant::now() + wait;
        loop {
            let heard = self.heard.lock().unwrap().clone();
            if !heard.is_empty() || Instant::now() >= deadline {
                return heard;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// How many connections the server accepted, after waiting up to `wait` for the first one.
    fn accepted_within(&self, wait: Duration) -> usize {
        let deadline = Instant::now() + wait;
        loop {
            let accepted = self.accepted.load(Ordering::SeqCst);
            if accepted > 0 || Instant::now() >= deadline {
                return accepted;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

/// The request's head, up to its blank line, read with a time-out so a client that sends nothing cannot hold the
/// server's thread.
fn read_head(stream: &mut TcpStream) -> String {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut head = Vec::new();
    let mut chunk = [0u8; 1024];
    while !head.windows(4).any(|window| window == b"\r\n\r\n") {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(read) => head.extend_from_slice(&chunk[..read]),
        }
    }
    String::from_utf8_lossy(&head).into_owned()
}

fn heard_of(head: &str, store_held_a_token: Option<bool>) -> Heard {
    let mut lines = head.split("\r\n");
    let request_line = lines.next().unwrap_or_default().to_string();
    let bearer = lines.find_map(|line| {
        let (name, value) = line.split_once(':')?;
        if !name.trim().eq_ignore_ascii_case("authorization") {
            return None;
        }
        value.trim().strip_prefix("Bearer ").map(str::to_string)
    });
    Heard {
        request_line,
        bearer,
        store_held_a_token,
    }
}

/// A token no other test or log has: `tok-revoke-` and 16 random hex digits.
fn invented_token() -> String {
    format!("tok-revoke-{:016x}", rand::random::<u64>())
}

/// [`revoke`] against `base` with the client a handle sends it with (`auth_transport::client_for`: 30 s), on a
/// runtime driven from this thread (a line it logs is this thread's), under [`GUARD`]. Answers how long it took and
/// whether it ended before the guard did.
fn timed_revoke(base: &str, token: &str) -> (Duration, bool) {
    let client = crate::auth_transport::client_for(base).expect("the auth client");
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("a runtime");
    let started = Instant::now();
    // Made inside the runtime: a tokio timer outside one panics.
    let guarded = runtime.block_on(async {
        let revoked = revoke(&client, base, token);
        tokio::time::timeout(GUARD, revoked).await
    });
    (started.elapsed(), guarded.is_ok())
}

/// A handle signed in against a server: the token in the credential store as a sign-in stores it, and the session
/// in memory as an unlock leaves it. No engine was started, so the sign-out finds none to stop.
struct SignedIn {
    engine: Arc<TestEngine>,
    credentials: MemoryCredentials,
    server: TestServer,
}

impl SignedIn {
    fn on(behaviour: Server, token: &str) -> Self {
        let credentials = MemoryCredentials::default();
        credentials
            .store_for(SLOT)
            .save_session_token(&SessionToken::new(token).expect("an invented token"))
            .expect("the token is stored");
        let watched = credentials.store_for(SLOT);
        let probe: StoreProbe = Box::new(move || matches!(watched.load_session_token(), Ok(Some(_))));
        let server = TestServer::start(behaviour, Some(probe));
        let (host, _messages) = ChannelHost::new(|| {});
        let config = EngineConfig {
            account_id: SLOT.into(),
            state_dir: crate::state_paths::init_for_test(),
            api_base: server.base.clone(),
        };
        let engine = Arc::new(Engine::open(&TEST, config, host, credentials.clone()).expect("the handle opens"));
        {
            let _writes = engine.writes.write().unwrap();
            engine
                .acct
                .advance_session_generation_holding_session_write_lock(SessionTransition::Write);
            *engine.acct.session.lock().unwrap() = Some(Session {
                token: token.to_string(),
                master_key: [7; 32],
                email: None,
            });
        }
        engine.set_auth_present(true);
        Self {
            engine,
            credentials,
            server,
        }
    }

    fn store_holds_a_token(&self) -> bool {
        matches!(self.credentials.store_for(SLOT).load_session_token(), Ok(Some(_)))
    }
}

/// Everything `body` logs through `tracing`, at every level, on this thread (as `backup_devices_api_tests`).
fn capture_logs<T>(body: impl FnOnce() -> T) -> (T, String) {
    #[derive(Clone)]
    struct Capture(Arc<Mutex<Vec<u8>>>);
    impl std::io::Write for Capture {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let buffer: Arc<Mutex<Vec<u8>>> = Arc::default();
    let writer = Capture(buffer.clone());
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .finish();
    let guard = tracing::subscriber::set_default(subscriber);
    let out = body();
    drop(guard);
    let text = String::from_utf8_lossy(&buffer.lock().unwrap()).into_owned();
    (out, text)
}

/// Rule 1: a server that takes the request and never answers holds the revoke for [`REVOKE_BOUND`], not for the
/// client's 30 s. M1 (the bound removed from `revoke`): the guard ends it at 20 s, red.
#[test]
fn the_revoke_gives_up_after_three_seconds() {
    let server = TestServer::start(Server::NeverAnswers, None);
    let token = invented_token();
    let (took, ended) = timed_revoke(&server.base, &token);
    assert!(
        ended,
        "the revoke still waited after {GUARD:?}: it has no bound of its own"
    );
    assert!(
        took >= REVOKE_BOUND,
        "took {took:?}: it ended before the bound, so not by it"
    );
    assert!(took < WITHIN, "took {took:?}");
    // The bound ended a request the server took, not one that never left.
    let heard = server.heard_within(Duration::from_secs(1));
    assert_eq!(heard.len(), 1, "the server read the logout once");
    assert!(heard[0].request_line.starts_with("POST /api/v1/auth/logout "));
    assert!(
        heard[0].bearer.as_deref() == Some(token.as_str()),
        "the logout carried the token"
    );
}

/// Rule 2, the control of rule 1: a server that is not there, or closes the connection at once, ends the revoke at
/// once, so rule 1's three seconds are the bound's and not a refused or closed connection's (on Windows, a port nobody
/// listens on ends within the bound: the OS retries the refused connect). Nothing is returned, so nothing changes.
#[test]
fn a_closed_port_ends_the_revoke_at_once() {
    let absent = TestServer::start(Server::Absent, None);
    let (took, ended) = timed_revoke(&absent.base, &invented_token());
    assert!(ended);
    // Windows itself retries a refused loopback connect for about 2 s (CP1 row R1): there the absent port gets the bound.
    let absent_within = if cfg!(windows) {
        REVOKE_BOUND
    } else {
        Duration::from_secs(1)
    };
    assert!(took < absent_within, "no server: took {took:?}");

    let server = TestServer::start(Server::Closes, None);
    let (took, ended) = timed_revoke(&server.base, &invented_token());
    assert!(ended);
    assert!(took < Duration::from_secs(1), "a closed connection: took {took:?}");
    assert!(
        server.accepted_within(Duration::from_secs(1)) >= 1,
        "the revoke reached the server"
    );
}

/// Rule 3: signing out against a server that never answers returns `Ok(())` within [`WITHIN`] (no engine runs here,
/// so its stop takes no time), and the server hears the logout only once the stored session is gone. M2 (the
/// `revoke` call moved above `clear_session` in `app.rs`): the store still holds the token when the server reads the
/// request, red.
#[test]
fn the_sign_out_clears_the_store_before_the_server_hears_it_and_returns_in_time() {
    let token = invented_token();
    let rig = SignedIn::on(Server::NeverAnswers, &token);
    let started = Instant::now();
    let signed_out = rig.engine.sign_out_keeping_local_data();
    let took = started.elapsed();
    assert_eq!(signed_out, Ok(()));
    assert!(took < WITHIN, "the sign-out took {took:?}");
    let heard = rig.server.heard_within(Duration::from_secs(1));
    assert_eq!(heard.len(), 1, "the server read one logout");
    assert!(heard[0].request_line.starts_with("POST /api/v1/auth/logout "));
    assert!(
        heard[0].bearer.as_deref() == Some(token.as_str()),
        "the logout carried the signed-out session's token"
    );
    assert_eq!(
        heard[0].store_held_a_token,
        Some(false),
        "the store still held the session when the server read the logout"
    );
    assert!(!rig.store_holds_a_token());
    assert!(!rig.engine.unlocked());
}

/// Rule 4: whatever the server does, the sign-out ends the same: `Ok(())`, no session in the store or in memory, the
/// same items left in the store as after a 200. M3 (an answer other than 2xx, or none, made an error that the
/// sign-out returns): red.
#[test]
fn every_answer_of_the_server_leaves_the_same_sign_out() {
    let mut ends = Vec::new();
    for behaviour in EVERY_SERVER {
        let token = invented_token();
        let rig = SignedIn::on(behaviour, &token);
        let signed_out = rig.engine.sign_out_keeping_local_data();
        // The logout was sent: a green here is never a revoke that did not run.
        match behaviour {
            Server::Answers(_) | Server::NeverAnswers => {
                let heard = rig.server.heard_within(Duration::from_secs(1));
                assert_eq!(heard.len(), 1, "{behaviour:?}: the server read one logout");
                assert!(
                    heard[0].bearer.as_deref() == Some(token.as_str()),
                    "{behaviour:?}: the logout carried the token"
                );
                assert_eq!(heard[0].store_held_a_token, Some(false), "{behaviour:?}");
            }
            Server::Closes => {
                let accepted = rig.server.accepted_within(Duration::from_secs(1));
                assert!(accepted >= 1, "{behaviour:?}: the logout reached the server");
            }
            Server::Absent => {}
        }
        let end = (
            signed_out,
            rig.store_holds_a_token(),
            rig.engine.unlocked(),
            rig.credentials.len(),
        );
        ends.push((behaviour, end));
    }
    let after_a_200 = ends[0].1.clone();
    assert_eq!(after_a_200, (Ok(()), false, false, after_a_200.3), "after a 200");
    for (behaviour, end) in ends {
        assert_eq!(
            end, after_a_200,
            "{behaviour:?}: (result, token stored, unlocked, items stored)"
        );
    }
}

/// Rule 5: no line logged while rules 1–4 run carries the token: the revoke and the sign-out against every server,
/// all driven from this thread (the capture is this thread's). M4 (`tracing::warn!("{base} {token}")` in `revoke`):
/// red.
#[test]
fn no_token_in_any_log_line() {
    let token = invented_token();
    let ((), logs) = capture_logs(|| {
        tracing::info!("capture-sentinel");
        for behaviour in EVERY_SERVER {
            let server = TestServer::start(behaviour, None);
            let _ = timed_revoke(&server.base, &token);
            let rig = SignedIn::on(behaviour, &token);
            let _ = rig.engine.sign_out_keeping_local_data();
        }
    });
    assert!(logs.contains("capture-sentinel"), "the capture reads this thread");
    let secret = token.trim_start_matches("tok-revoke-");
    assert_eq!(secret.len(), 16, "the random part of the token");
    assert!(!logs.contains(secret), "a log line carries the token");
}
