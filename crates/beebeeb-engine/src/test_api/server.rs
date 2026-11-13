//! The fake account as a process of its own (task 2475): `bb-fake-server` serves ONE fixture account on loopback, so
//! a TEST build started with `--api-base http://127.0.0.1:<port>` can sign in on a computer with no server (the native
//! checks on the build VM), never against production.
//!
//! What it serves is [`account_handler`] over [`fixture_account`]: real OPAQUE against the fixture's own password file,
//! a recovery check derived from [`TEST_PHRASE`], the sample tree, and a FIXED user id, so a TEST build's local data
//! stays bound to the same account when the server restarts (the build also binds its data to the origin, port
//! included: give the same `--listen` again). In front of it, [`fixture_handler`] answers a sign-in for any other email
//! as a wrong password before OPAQUE runs, and writes one [`access_line`] per answer.
//!
//! The binary (`src/test_api/bin_main.rs`) is built only with `--features fake-server`, which implies `test-support`:
//! the default feature set never builds it, a release profile refuses to compile it (`lib.rs`), and no app enables it
//! (`tests/fake_server.rs` reads the manifests and the packaging scripts).
//!
//! Must not: listen anywhere but 127.0.0.1 or ::1 ([`loopback_listen`], [`bind`]); send a request of its own (there is
//! no client here); keep a request after answering it ([`FakeApi::start_on`]); write a password, a phrase, a token, a
//! header, a query or a body (stdout: one line, the address; stderr: its own words and the [`access_line`]s).

use std::io::{self, BufRead, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpListener};
use std::process::ExitCode;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Duration;

use serde_json::{Value, json};

use super::fake_account::{FakeAccount, TEST_PHRASE, account_handler, add_sample_tree};
use super::{FAKE_API_MARK, FakeApi, Handler, Reply, Request};

/// The fixture account's email: a reserved name that can never be a real address (RFC 2606 `.invalid`).
pub const FIXTURE_EMAIL: &str = "fixture@example.invalid";
/// Its password: a fixture, public in this file. The server never receives it (OPAQUE) and never prints it.
pub const FIXTURE_PASSWORD: &str = "fixture password 2475";
/// Its user id, the same at every start: a TEST build binds its local data to it.
pub const FIXTURE_USER_ID: &str = "f1c70000-2475-4000-8000-000000000001";
/// `--max-seconds` when it is not given.
pub const DEFAULT_MAX_SECONDS: u64 = 3600;
const MAX_SECONDS: u64 = 86_400;

pub const USAGE: &str =
    "usage: bb-fake-server [--listen 127.0.0.1:<port> | --listen [::1]:<port>] [--max-seconds <1-86400>]
Serves the engine's fixture account (a fake of the API, for tests) on loopback only; the default address is
127.0.0.1:0 (a free port). Once it listens it prints ONE line on stdout: `bb-fake-server listening http://<address>`.
It stops when its standard input closes, on a line `stop`, or after --max-seconds (default 3600), and says why on
stderr. A development binary: built only with --features fake-server, never in a release profile, never in an app.";

/// What the command line asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    pub listen: SocketAddr,
    pub max_seconds: u64,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            listen: SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
            max_seconds: DEFAULT_MAX_SECONDS,
        }
    }
}

/// Why `--listen` was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListenRefused {
    /// Not `<ip>:<port>`. A host name is never resolved: `localhost` can be made to point anywhere.
    NotAnAddress,
    /// Any address but 127.0.0.1 and ::1: 0.0.0.0, `::`, a LAN address, the rest of 127.0.0.0/8, `::ffff:127.0.0.1`.
    NotLoopback,
}

/// Why the command line was refused. The words name the argument, never its value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArgError {
    Help,
    /// A flag without its value.
    Missing(&'static str),
    /// Anything else on the line, `--listen=<x>` included (the value goes after a space).
    Unknown,
    Listen(ListenRefused),
    /// Not a whole number of seconds from 1 to 86400.
    MaxSeconds,
}

impl std::fmt::Display for ArgError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Help => write!(f, "help"),
            Self::Missing(flag) => write!(f, "{flag} needs a value after it"),
            Self::Unknown => write!(
                f,
                "an argument it does not read (only --listen, --max-seconds and --help)"
            ),
            Self::Listen(ListenRefused::NotAnAddress) => {
                write!(
                    f,
                    "--listen takes <ip>:<port> (127.0.0.1:<port> or [::1]:<port>), never a name"
                )
            }
            Self::Listen(ListenRefused::NotLoopback) => {
                write!(f, "--listen was refused: it listens on 127.0.0.1 or ::1 only")
            }
            Self::MaxSeconds => write!(f, "--max-seconds takes a whole number from 1 to {MAX_SECONDS}"),
        }
    }
}

/// Read the command line (without the program name).
pub fn parse_args<I: IntoIterator<Item = String>>(args: I) -> Result<Options, ArgError> {
    let mut options = Options::default();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => return Err(ArgError::Help),
            "--listen" => {
                let value = args.next().ok_or(ArgError::Missing("--listen"))?;
                options.listen = loopback_listen(&value).map_err(ArgError::Listen)?;
            }
            "--max-seconds" => {
                let value = args.next().ok_or(ArgError::Missing("--max-seconds"))?;
                options.max_seconds = value
                    .parse::<u64>()
                    .ok()
                    .filter(|seconds| (1..=MAX_SECONDS).contains(seconds))
                    .ok_or(ArgError::MaxSeconds)?;
            }
            _ => return Err(ArgError::Unknown),
        }
    }
    Ok(options)
}

/// `text` as the address to listen on: an IP literal and a port, and the IP exactly 127.0.0.1 or ::1 (the two loopback
/// addresses a TEST build's `--api-base` takes as IPs; its `localhost` is a name, and a name is not read here).
pub fn loopback_listen(text: &str) -> Result<SocketAddr, ListenRefused> {
    let address: SocketAddr = text.parse().map_err(|_| ListenRefused::NotAnAddress)?;
    if is_app_loopback(address.ip()) {
        Ok(address)
    } else {
        Err(ListenRefused::NotLoopback)
    }
}

fn is_app_loopback(ip: IpAddr) -> bool {
    ip == IpAddr::V4(Ipv4Addr::LOCALHOST) || ip == IpAddr::V6(Ipv6Addr::LOCALHOST)
}

/// Bind `listen`, refusing anything but 127.0.0.1 and ::1 before the bind and checking what was bound after it.
pub fn bind(listen: SocketAddr) -> io::Result<TcpListener> {
    let refused = || io::Error::new(io::ErrorKind::InvalidInput, "it listens on 127.0.0.1 or ::1 only");
    if !is_app_loopback(listen.ip()) {
        return Err(refused());
    }
    let listener = TcpListener::bind(listen)?;
    if !is_app_loopback(listener.local_addr()?.ip()) {
        return Err(refused());
    }
    Ok(listener)
}

/// The fixture account: registered with core's own OPAQUE functions (Argon2id runs once, here), with the phrase
/// [`TEST_PHRASE`], the fixed user id and the sample tree (2 folders, 5 files).
pub fn fixture_account() -> Arc<FakeAccount> {
    let (mut account, _phrase) = FakeAccount::register(FIXTURE_EMAIL, FIXTURE_PASSWORD, TEST_PHRASE);
    account.user_id = FIXTURE_USER_ID.to_string();
    add_sample_tree(&account);
    Arc::new(account)
}

/// One line per answer, as the server writes it: `METHOD /path -> status`. The query is cut (an id, a cursor), and
/// nothing else of the request is read: no header (the bearer token), no body (an email, an OPAQUE message).
pub fn access_line(request: &Request, status: u16) -> String {
    let path = request.path.split('?').next().unwrap_or_default();
    format!("{} {path} -> {status}", request.method)
}

/// Where the [`access_line`]s go (stderr in the binary, a list in a test).
pub type LineWriter = Arc<dyn Fn(&str) + Send + Sync>;

/// The fixture account's routes ([`account_handler`]) behind the server's two rules: a sign-in leg for any other email
/// is answered as a wrong password (`401`, what core reads as "Invalid email or password") before OPAQUE runs, and
/// every answer is counted in `served` and written as an [`access_line`].
pub fn fixture_handler(account: Arc<FakeAccount>, lines: LineWriter, served: Arc<AtomicU64>) -> Handler {
    let routes = account_handler(account.clone());
    Arc::new(move |request: &Request| {
        let reply = if is_sign_in_leg(request) && !names_email(request, &account.email) {
            Reply::json(401, json!({}))
        } else {
            routes(request)
        };
        served.fetch_add(1, Ordering::SeqCst);
        lines(&access_line(request, reply.status));
        reply
    })
}

fn is_sign_in_leg(request: &Request) -> bool {
    matches!(
        request.route().as_str(),
        "POST /api/v1/opaque/login-start" | "POST /api/v1/opaque/login-finish"
    )
}

fn names_email(request: &Request, email: &str) -> bool {
    request
        .json()
        .get("email")
        .and_then(Value::as_str)
        .is_some_and(|sent| sent.trim().eq_ignore_ascii_case(email))
}

/// Why the server stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stop {
    /// Standard input ended: the pipe the caller held was closed (or there was none).
    InputClosed,
    /// A line `stop` on standard input.
    StopLine,
    /// `--max-seconds` passed.
    Deadline,
}

impl Stop {
    pub fn words(self) -> &'static str {
        match self {
            Self::InputClosed => "standard input closed",
            Self::StopLine => "a line `stop`",
            Self::Deadline => "--max-seconds passed",
        }
    }
}

/// Read `input` on a thread of its own until it ends or a line `stop` comes (other lines are ignored); the first of
/// that and `deadline` is the answer.
pub fn wait_for_stop<R: BufRead + Send + 'static>(input: R, deadline: Duration) -> io::Result<Stop> {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("fake-server-input".into())
        .spawn(move || {
            let _ = tx.send(read_until_stop(input));
        })?;
    Ok(match rx.recv_timeout(deadline) {
        Ok(stop) => stop,
        Err(mpsc::RecvTimeoutError::Timeout) => Stop::Deadline,
        Err(mpsc::RecvTimeoutError::Disconnected) => Stop::InputClosed,
    })
}

fn read_until_stop<R: BufRead>(mut input: R) -> Stop {
    let mut line = String::new();
    loop {
        line.clear();
        match input.read_line(&mut line) {
            Ok(0) | Err(_) => return Stop::InputClosed,
            Ok(_) if line.trim() == "stop" => return Stop::StopLine,
            Ok(_) => {}
        }
    }
}

/// A line on stderr. A closed stderr is not a reason to stop answering (`eprintln!` would panic on it).
fn say(text: &str) {
    let mut err = io::stderr().lock();
    let _ = writeln!(err, "{text}");
}

/// The whole server, for `main`: read the command line, bind, print the one stdout line, serve until [`wait_for_stop`],
/// stop (every thread joined), say how many requests were answered. Exit 0 after a stop, 2 for a refused command
/// line, 1 when it could not listen or print its address.
pub fn run<I: IntoIterator<Item = String>>(args: I) -> ExitCode {
    let options = match parse_args(args) {
        Ok(options) => options,
        Err(ArgError::Help) => {
            let _ = writeln!(io::stdout().lock(), "{USAGE}");
            return ExitCode::SUCCESS;
        }
        Err(error) => {
            say(&format!("bb-fake-server: {error}\n{USAGE}"));
            return ExitCode::from(2);
        }
    };
    say(&format!(
        "bb-fake-server: {FAKE_API_MARK}; fixture account {FIXTURE_EMAIL}; registering it (Argon2id) before listening"
    ));
    // Register the fixture account (Argon2id, ~27 s) BEFORE binding, so the port does not accept connections while
    // the handler is not ready (P2-3 of the PR #338 review).
    let served = Arc::new(AtomicU64::new(0));
    let lines: LineWriter = Arc::new(say);
    let account = fixture_account();
    let handler = fixture_handler(account, lines, served.clone());
    let listener = match bind(options.listen) {
        Ok(listener) => listener,
        Err(error) => {
            say(&format!(
                "bb-fake-server: could not listen on {}: {error}",
                options.listen
            ));
            return ExitCode::from(1);
        }
    };
    let fake = match FakeApi::start_on(listener, handler) {
        Ok(fake) => fake,
        Err(error) => {
            say(&format!("bb-fake-server: could not start: {error}"));
            return ExitCode::from(1);
        }
    };
    let mut out = io::stdout().lock();
    if writeln!(out, "bb-fake-server listening {}", fake.base)
        .and_then(|()| out.flush())
        .is_err()
    {
        say("bb-fake-server: could not print its address on stdout; stopping");
        return ExitCode::from(1);
    }
    drop(out);
    let stop = match wait_for_stop(
        io::BufReader::new(io::stdin()),
        Duration::from_secs(options.max_seconds),
    ) {
        Ok(stop) => stop,
        Err(error) => {
            say(&format!(
                "bb-fake-server: could not read standard input ({error}); stopping"
            ));
            return ExitCode::from(1);
        }
    };
    // Stops the listener and joins every thread it started.
    drop(fake);
    say(&format!(
        "bb-fake-server: stopped ({}): {} requests answered",
        stop.words(),
        served.load(Ordering::SeqCst)
    ));
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read as _;
    use std::sync::{Mutex, OnceLock};

    /// One fixture account for this module (each registration runs Argon2id).
    fn fixture() -> Arc<FakeAccount> {
        static ACCOUNT: OnceLock<Arc<FakeAccount>> = OnceLock::new();
        ACCOUNT.get_or_init(fixture_account).clone()
    }

    fn post(path: &str, body: Value) -> Request {
        Request {
            method: "POST".into(),
            path: path.into(),
            headers: Vec::new(),
            body: body.to_string().into_bytes(),
        }
    }

    #[test]
    fn listen_takes_exactly_127_0_0_1_and_the_ipv6_loopback() {
        let accepted = ["127.0.0.1:0", "127.0.0.1:47475", "[::1]:0", "[::1]:65535"];
        let not_loopback = [
            "0.0.0.0:0",
            "[::]:0",
            "10.0.5.13:80",
            "192.168.1.2:80",
            "127.0.0.2:80",
            "127.1.2.3:80",
            "[::ffff:127.0.0.1]:80",
            "[fe80::1]:80",
            "255.255.255.255:80",
        ];
        let not_an_address = [
            "localhost:80",
            "example.invalid:80",
            "127.0.0.1",
            "::1:80",
            "",
            " 127.0.0.1:80",
            "http://127.0.0.1:80",
            "127.0.0.1:99999",
        ];
        let ok = accepted.iter().filter(|t| loopback_listen(t).is_ok()).count();
        let refused = not_loopback
            .iter()
            .filter(|t| loopback_listen(t) == Err(ListenRefused::NotLoopback))
            .count();
        let unread = not_an_address
            .iter()
            .filter(|t| loopback_listen(t) == Err(ListenRefused::NotAnAddress))
            .count();
        assert_eq!(
            (ok, refused, unread),
            (4, 9, 8),
            "accepted, not loopback, not an address"
        );
    }

    #[test]
    fn bind_refuses_a_non_loopback_address_and_binds_loopback() {
        assert_eq!(
            bind(SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0)))
                .map(|_| ())
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
        let listener = bind(loopback_listen("127.0.0.1:0").unwrap()).expect("bind 127.0.0.1");
        assert_eq!(listener.local_addr().unwrap().ip(), IpAddr::V4(Ipv4Addr::LOCALHOST));
    }

    #[test]
    fn the_command_line() {
        let parse = |args: &[&str]| parse_args(args.iter().map(|a| a.to_string()));
        assert_eq!(parse(&[]), Ok(Options::default()));
        assert_eq!(Options::default().listen.to_string(), "127.0.0.1:0");
        assert_eq!(
            parse(&["--listen", "[::1]:47475", "--max-seconds", "900"]),
            Ok(Options {
                listen: "[::1]:47475".parse().unwrap(),
                max_seconds: 900
            })
        );
        let refused = [
            (vec!["--listen"], ArgError::Missing("--listen")),
            (
                vec!["--listen", "0.0.0.0:1"],
                ArgError::Listen(ListenRefused::NotLoopback),
            ),
            (
                vec!["--listen", "localhost:1"],
                ArgError::Listen(ListenRefused::NotAnAddress),
            ),
            (vec!["--listen=127.0.0.1:1"], ArgError::Unknown),
            (vec!["--max-seconds", "0"], ArgError::MaxSeconds),
            (vec!["--max-seconds", "86401"], ArgError::MaxSeconds),
            (vec!["--max-seconds"], ArgError::Missing("--max-seconds")),
            (vec!["--port", "1"], ArgError::Unknown),
            (vec!["--help"], ArgError::Help),
        ];
        let matched = refused
            .iter()
            .filter(|(args, want)| parse(args.as_slice()) == Err(*want))
            .count();
        assert_eq!(matched, refused.len());
        // The words name the argument, never the value.
        assert!(
            !ArgError::Listen(ListenRefused::NotLoopback)
                .to_string()
                .contains("0.0.0.0")
        );
    }

    #[test]
    fn an_access_line_is_the_method_the_path_and_the_status_only() {
        let request = Request {
            method: "POST".into(),
            path: "/api/v1/opaque/login-start?cursor=QUERY-TEXT".into(),
            headers: vec![("Authorization".into(), "Bearer HEADER-TEXT".into())],
            body: br#"{"email":"someone@example.invalid","client_message":"BODY-TEXT"}"#.to_vec(),
        };
        assert_eq!(access_line(&request, 401), "POST /api/v1/opaque/login-start -> 401");
    }

    #[test]
    fn another_email_is_answered_as_a_wrong_password_before_opaque_runs() {
        let lines = Arc::new(Mutex::new(Vec::<String>::new()));
        let sink = lines.clone();
        let served = Arc::new(AtomicU64::new(0));
        let handler = fixture_handler(
            fixture(),
            Arc::new(move |line: &str| sink.lock().unwrap().push(line.to_string())),
            served.clone(),
        );
        // A message OPAQUE cannot read: OPAQUE answers it with 400, the email rule with 401 before OPAQUE is asked.
        let start = |email: &str| {
            post(
                "/api/v1/opaque/login-start",
                json!({ "email": email, "client_message": "AA==" }),
            )
        };
        let finish = |email: &str| {
            post(
                "/api/v1/opaque/login-finish",
                json!({ "email": email, "client_message": "AA==", "server_state": "none" }),
            )
        };
        let statuses = [
            handler(&start("someone@example.invalid")).status,
            handler(&finish("someone@example.invalid")).status,
            handler(&post("/api/v1/opaque/login-start", json!({ "client_message": "AA==" }))).status,
            handler(&start(FIXTURE_EMAIL)).status,
            handler(&start(" FIXTURE@Example.Invalid ")).status,
            handler(&finish(FIXTURE_EMAIL)).status,
        ];
        assert_eq!(statuses, [401, 401, 401, 400, 400, 401]);
        assert_eq!(served.load(Ordering::SeqCst), 6);
        let written = lines.lock().unwrap().clone();
        assert_eq!(written.len(), 6);
        assert_eq!(written[0], "POST /api/v1/opaque/login-start -> 401");
        assert_eq!(written[3], "POST /api/v1/opaque/login-start -> 400");

        // P2-1: the login-finish email guard, distinguished from OPAQUE's own answer. Start with the fixture's
        // email to get a VALID server_state (OPAQUE 200), then send finish with that state but another email:
        // the email guard returns 401, while OPAQUE itself would return 200 (the state is valid for that session)
        // or 400 (bad_request). If the guard is removed, this line turns green at the wrong status.
        // The start needs a message OPAQUE can read (`start` above sends one it cannot, and gets 400 with no state).
        use base64::Engine as _;
        let readable = beebeeb_core::opaque_protocol::client_login_start(b"any password")
            .expect("client login start")
            .message;
        let real_start = post(
            "/api/v1/opaque/login-start",
            json!({ "email": FIXTURE_EMAIL, "client_message": base64::engine::general_purpose::STANDARD.encode(readable) }),
        );
        let started = handler(&real_start);
        assert_eq!(
            started.status, 200,
            "a readable start for the fixture's email is answered by OPAQUE"
        );
        let start_resp: Value = serde_json::from_slice(&started.body).unwrap();
        let server_state = start_resp["server_state"].as_str().unwrap().to_string();
        let hijack = post(
            "/api/v1/opaque/login-finish",
            json!({ "email": "someone@example.invalid", "client_message": "AA==", "server_state": server_state }),
        );
        assert_eq!(
            handler(&hijack).status,
            401,
            "the email guard on login-finish returns 401 for another email"
        );
    }

    #[test]
    fn the_fixture_account_is_the_same_at_every_start() {
        let account = fixture();
        assert_eq!(
            (account.email.as_str(), account.user_id.as_str()),
            (FIXTURE_EMAIL, FIXTURE_USER_ID)
        );
        let routes = account_handler(account.clone());
        let mut snapshot = Request {
            method: "GET".into(),
            path: "/api/v1/sync/snapshot".into(),
            headers: vec![("Authorization".into(), format!("Bearer {}", account.session_token))],
            body: Vec::new(),
        };
        let nodes = serde_json::from_slice::<Value>(&routes(&snapshot).body).unwrap()["nodes"]
            .as_array()
            .map(Vec::len);
        assert_eq!(nodes, Some(7), "the sample tree: 2 folders, 5 files");
        snapshot.headers.clear();
        assert_eq!(routes(&snapshot).status, 401, "without the session token");
    }

    /// GET `path` from `base` over a raw socket; the whole answer as text.
    fn get_raw(base: &str, path: &str) -> String {
        let address = base.strip_prefix("http://").unwrap();
        let mut stream = std::net::TcpStream::connect(address).unwrap();
        stream
            .write_all(format!("GET {path} HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer x\r\n\r\n").as_bytes())
            .unwrap();
        let mut answer = String::new();
        stream.read_to_string(&mut answer).unwrap();
        answer
    }

    #[test]
    fn a_started_server_keeps_no_request_and_names_itself_in_every_answer() {
        let quiet = FakeApi::start_on(
            bind(loopback_listen("127.0.0.1:0").unwrap()).unwrap(),
            Arc::new(|_: &Request| Reply::ok(json!({}))),
        )
        .unwrap();
        let answer = get_raw(&quiet.base, "/api/v1/notifications");
        // The tests' fake keeps every request; the server's keeps none (the instrument sees one where there is one).
        let kept = FakeApi::start(Arc::new(|_: &Request| Reply::ok(json!({}))));
        get_raw(&kept.base, "/api/v1/notifications");
        assert_eq!(
            (
                answer.starts_with("HTTP/1.1 200 "),
                answer.matches(&format!("\r\nServer: {FAKE_API_MARK}\r\n")).count(),
                quiet.requests().len(),
                kept.requests().len()
            ),
            (true, 1, 0, 1),
            "answered, Server headers with the mark, requests the server kept, requests the tests' fake kept"
        );
    }

    /// Blocks every read until the test drops its sender: a standard input nobody closes.
    struct OpenInput(mpsc::Receiver<()>);

    impl io::Read for OpenInput {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            let _ = self.0.recv();
            Ok(0)
        }
    }

    #[test]
    fn it_stops_when_the_input_closes_on_a_stop_line_or_at_the_deadline() {
        let wait = |text: &'static str| wait_for_stop(io::Cursor::new(text), Duration::from_secs(10)).unwrap();
        assert_eq!(wait(""), Stop::InputClosed);
        assert_eq!(wait("hello\nnot stop\n"), Stop::InputClosed);
        assert_eq!(wait("hello\n stop \nlater\n"), Stop::StopLine);
        let (_held, rx) = mpsc::channel::<()>();
        let open = io::BufReader::new(OpenInput(rx));
        assert_eq!(wait_for_stop(open, Duration::from_millis(300)).unwrap(), Stop::Deadline);
    }
}
