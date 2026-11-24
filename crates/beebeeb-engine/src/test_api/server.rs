//! The fake account as a process of its own (task 2475): `bb-fake-server` serves ONE fixture account on loopback, so
//! a TEST build started with `--api-base http://127.0.0.1:<port>` can sign in on a computer with no server (the native
//! checks on the build VM), never against production.
//!
//! What it serves is [`account_handler`] over [`fixture_account`]: real OPAQUE against the fixture's own password file,
//! a recovery check derived from [`TEST_PHRASE`], the sample tree, and a FIXED user id, so a TEST build's local data
//! stays bound to the same account when the server restarts (the build also binds its data to the origin, port
//! included: give the same `--listen` again). In front of it, [`fixture_handler`] answers a sign-in for any other email
//! as a wrong password before OPAQUE runs, answers every call with a session token `401` once a line
//! [`END_SESSION_LINE`] came on standard input (a native check of the app's session watcher), refuses the device route
//! once a line [`HEARTBEAT_OFF_LINE`] came (so only the sync tick feeds that watcher), and writes one [`access_line`]
//! per answer. The end-session switch models "the server refuses this account's sessions", not "this one session was
//! revoked": the account has one token, so a password sign-in after the end gets the same refused token back.
//!
//! It also serves a device sign-in ([`start_fixture`], `fake_device_signin`): the socket on its own port that a TEST
//! build opens to be signed in by another device, the approving device's routes, and a second device that a check
//! script drives with one control call (approve, deny as the fixture, or try to approve as another account).
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
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Duration;

use serde_json::{Value, json};

use super::fake_account::{FakeAccount, TEST_PHRASE, account_handler, add_sample_tree};
use super::fake_device_signin::DeviceSignIn;
use super::{FAKE_API_MARK, FakeApi, Handler, Reply, Request, SocketRoute};

/// The fixture account's email: a reserved name that can never be a real address (RFC 2606 `.invalid`).
pub const FIXTURE_EMAIL: &str = "fixture@example.invalid";
/// Its password: a fixture, public in this file. The server never receives it (OPAQUE) and never prints it.
pub const FIXTURE_PASSWORD: &str = "fixture password 2475";
/// Its user id, the same at every start: a TEST build binds its local data to it.
pub const FIXTURE_USER_ID: &str = "f1c70000-2475-4000-8000-000000000001";
/// The line on standard input that ends the fixture account's session (see [`fixture_handler`]).
pub const END_SESSION_LINE: &str = "end-session";
/// The line on standard input that refuses the device route (see [`fixture_handler`]).
pub const HEARTBEAT_OFF_LINE: &str = "heartbeat-off";
/// The line on standard input that makes the upload routes unreachable (see [`fixture_handler`]).
pub const UPLOADS_OFF_LINE: &str = "uploads-off";
/// The control route that stores a newer version of a file, as another device's save (no token: a loopback control).
pub const NEWER_VERSION_ROUTE: &str = "POST /fake-control/newer-version";
/// The control route that turns the refusal of uploads on or off.
pub const UPLOADS_REFUSED_ROUTE: &str = "POST /fake-control/uploads-refused";
/// The control route where the second device approves a waiting device sign-in (`fake_device_signin`).
pub const DEVICE_APPROVE_ROUTE: &str = "POST /fake-control/device-approve";
/// The control route where the second device denies a waiting device sign-in.
pub const DEVICE_DENY_ROUTE: &str = "POST /fake-control/device-deny";
/// The control route that sets the life of the device sign-in codes issued after it.
pub const DEVICE_CODE_LIFE_ROUTE: &str = "POST /fake-control/device-code-life";
/// The control route that says how many device sign-ins wait.
pub const DEVICE_REQUESTS_ROUTE: &str = "GET /fake-control/device-requests";
/// `--max-seconds` when it is not given.
pub const DEFAULT_MAX_SECONDS: u64 = 3600;
const MAX_SECONDS: u64 = 86_400;

pub const USAGE: &str =
    "usage: bb-fake-server [--listen 127.0.0.1:<port> | --listen [::1]:<port>] [--max-seconds <1-86400>]
Serves the engine's fixture account (a fake of the API, for tests) on loopback only; the default address is
127.0.0.1:0 (a free port). Once it listens it prints ONE line on stdout: `bb-fake-server listening http://<address>`.
It stops when its standard input closes, on a line `stop`, or after --max-seconds (default 3600), and says why on
stderr. Two more lines are switches, each set until the server stops:
`end-session` refuses this account's session token: from then on every request with a session token is answered
401, as a server that no longer accepts the session.
`heartbeat-off` refuses the device route (POST /api/v1/clients/devices) with 503, so the app's heartbeat never starts.
`uploads-off` makes every upload route (/api/v1/uploads/...) hang up without an answer, as a server that cannot be
reached for them (the same switch: POST /fake-control/uploads-refused with a JSON body {refused: true or false} sets it either way).
Control, one HTTP call each, no token, loopback only: POST /fake-control/newer-version with a JSON body of file_id
or path, and content_text or content_base64, saves a newer version of that file as another device would.
A device sign-in: the app's socket (GET /api/v1/auth/cli) is served on this port and shows a code. A second device
approves it: POST /fake-control/device-approve with {code: XXXX-XXXX} (or {} for the one waiting request); with
{approver: stranger} another account tries and is refused. POST /fake-control/device-deny denies it the same way.
POST /fake-control/device-code-life with {seconds: 1-300} sets the life of the codes issued after it (default 300).
GET /fake-control/device-requests answers {waiting: n}. No code is written to a line.
A development binary: built only with --features fake-server, never in a release profile, never in an app.";

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
    account.serve_security_routes(true);
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

/// The two switches a line on standard input sets (each stays set until the server stops).
#[derive(Debug, Clone, Default)]
pub struct Switches {
    /// A line [`UPLOADS_OFF_LINE`], or the control route: every upload route hangs up (off at the start).
    pub uploads_off: Arc<AtomicBool>,
    /// A line [`END_SESSION_LINE`]: the server refuses this account's session token on every call.
    pub session_ended: Arc<AtomicBool>,
    /// A line [`HEARTBEAT_OFF_LINE`]: the device route (`POST /api/v1/clients/devices`) is refused, so the app's
    /// heartbeat never starts and only the sync tick feeds its session watcher.
    pub heartbeat_off: Arc<AtomicBool>,
}

/// The fixture account's routes ([`account_handler`]) behind the server's four rules: once `session_ended` is set (a
/// line [`END_SESSION_LINE`]), every request with a session token is answered `401` before any route runs, as a server
/// that no longer accepts the session (the sign-in legs carry no token and are not touched); once `heartbeat_off` is set
/// (a line [`HEARTBEAT_OFF_LINE`]), the device route is answered `503`; a sign-in leg for any
/// other email is answered as a wrong password (`401`, what core reads as "Invalid email or password") before OPAQUE
/// runs; and every answer is counted in `served` and written as an [`access_line`].
pub fn fixture_handler(
    account: Arc<FakeAccount>,
    lines: LineWriter,
    served: Arc<AtomicU64>,
    switches: Switches,
) -> Handler {
    fixture_handler_with(
        account,
        lines,
        served,
        switches,
        Arc::new(DeviceSignIn::new(FIXTURE_PASSWORD)),
    )
}

/// The fixture account served as `bb-fake-server` serves it: [`fixture_handler`]'s routes, and the device sign-in's
/// socket on the same listener ([`DeviceSignIn::socket`]; its access line is `GET /api/v1/auth/cli -> 101`).
pub fn start_fixture(
    listener: TcpListener,
    account: Arc<FakeAccount>,
    lines: LineWriter,
    served: Arc<AtomicU64>,
    switches: Switches,
) -> io::Result<FakeApi> {
    let device = Arc::new(DeviceSignIn::new(FIXTURE_PASSWORD));
    let handler = fixture_handler_with(account, lines.clone(), served.clone(), switches, device.clone());
    let sockets: SocketRoute = Arc::new(move |request: &Request| {
        let take = device.socket(request)?;
        served.fetch_add(1, Ordering::SeqCst);
        lines(&access_line(request, 101));
        Some(take)
    });
    FakeApi::start_on_with_sockets(listener, handler, sockets)
}

/// [`fixture_handler`] with the device sign-in routes of `device` (the API's and the second device's controls).
fn fixture_handler_with(
    account: Arc<FakeAccount>,
    lines: LineWriter,
    served: Arc<AtomicU64>,
    switches: Switches,
    device: Arc<DeviceSignIn>,
) -> Handler {
    let routes = account_handler(account.clone());
    // The photo-preview routes (PREV-1), with one line per stored preview for the native check.
    let previews = super::fake_previews::PreviewRoutes::new(account.clone());
    Arc::new(move |request: &Request| {
        let reply = if let Some(reply) = device.control(&account, request, &*lines) {
            reply
        } else if let Some(reply) = control(&account, &switches, request) {
            reply
        } else if switches.uploads_off.load(Ordering::SeqCst) && is_upload_call(request) {
            Reply::hang_up()
        } else if switches.session_ended.load(Ordering::SeqCst) && request.bearer().is_some() {
            Reply::error(401, "unauthorized")
        } else if switches.heartbeat_off.load(Ordering::SeqCst) && request.route() == "POST /api/v1/clients/devices" {
            Reply::error(503, "unavailable")
        } else if is_sign_in_leg(request) && !names_email(request, &account.email) {
            Reply::json(401, json!({}))
        } else if let Some(reply) = previews.answer(request) {
            reply
        } else if let Some(reply) = device.answer(&account, request) {
            reply
        } else {
            routes(request)
        };
        served.fetch_add(1, Ordering::SeqCst);
        lines(&access_line(request, reply.status));
        if let Some(line) = previews.note(request, reply.status) {
            lines(&line);
        }
        reply
    })
}

/// An upload session route: `init`, a chunk, `complete`, `heartbeat` (the picture and abandon routes are not uploads).
fn is_upload_call(request: &Request) -> bool {
    matches!(request.method.as_str(), "POST" | "PUT") && request.route().contains(" /api/v1/uploads/")
}

/// The control routes: they answer without a token and are never part of the API a client sees.
fn control(account: &FakeAccount, switches: &Switches, request: &Request) -> Option<Reply> {
    use base64::Engine as _;
    match request.route().as_str() {
        NEWER_VERSION_ROUTE => {
            let body = request.json();
            let id = match (
                body.get("file_id").and_then(Value::as_str),
                body.get("path").and_then(Value::as_str),
            ) {
                (Some(id), _) => Some(id.to_string()),
                (None, Some(path)) => account.find_by_path(path),
                (None, None) => return Some(Reply::error(400, "file_id or path")),
            };
            let Some(id) = id else {
                return Some(Reply::error(404, "no such file"));
            };
            let content = match (
                body.get("content_text").and_then(Value::as_str),
                body.get("content_base64").and_then(Value::as_str),
            ) {
                (Some(text), _) => text.as_bytes().to_vec(),
                (None, Some(encoded)) => match base64::engine::general_purpose::STANDARD.decode(encoded) {
                    Ok(bytes) => bytes,
                    Err(_) => return Some(Reply::error(400, "content_base64")),
                },
                (None, None) => return Some(Reply::error(400, "content_text or content_base64")),
            };
            Some(match account.push_newer_version(&id, &content) {
                Ok(made) => Reply::ok(json!({
                    "file_id": made.file_id,
                    "version_number": made.version_number,
                    "size_bytes": made.size_bytes,
                    "updated_at": made.updated_at,
                })),
                Err(why) => Reply::json(409, json!({ "error": why })),
            })
        }
        UPLOADS_REFUSED_ROUTE => {
            let refused = request.json().get("refused").and_then(Value::as_bool);
            let Some(refused) = refused else {
                return Some(Reply::error(400, "refused"));
            };
            switches.uploads_off.store(refused, Ordering::SeqCst);
            Some(Reply::ok(json!({ "refused": refused })))
        }
        _ => None,
    }
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

/// Read `input` on a thread of its own until it ends or a line `stop` comes; the first of that and `deadline` is the
/// answer. A line [`END_SESSION_LINE`] or [`HEARTBEAT_OFF_LINE`] sets its switch (and says so on stderr, once);
/// other lines are ignored.
pub fn wait_for_stop<R: BufRead + Send + 'static>(
    input: R,
    deadline: Duration,
    switches: Switches,
) -> io::Result<Stop> {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("fake-server-input".into())
        .spawn(move || {
            let _ = tx.send(read_until_stop(input, &switches));
        })?;
    Ok(match rx.recv_timeout(deadline) {
        Ok(stop) => stop,
        Err(mpsc::RecvTimeoutError::Timeout) => Stop::Deadline,
        Err(mpsc::RecvTimeoutError::Disconnected) => Stop::InputClosed,
    })
}

fn read_until_stop<R: BufRead>(mut input: R, switches: &Switches) -> Stop {
    let mut line = String::new();
    loop {
        line.clear();
        match input.read_line(&mut line) {
            Ok(0) | Err(_) => return Stop::InputClosed,
            Ok(_) if line.trim() == "stop" => return Stop::StopLine,
            Ok(_) if line.trim() == END_SESSION_LINE => {
                if !switches.session_ended.swap(true, Ordering::SeqCst) {
                    say(
                        "bb-fake-server: the session was ended (a line `end-session`): every request with a session token is answered 401 from now on",
                    );
                }
            }
            Ok(_) if line.trim() == HEARTBEAT_OFF_LINE => {
                if !switches.heartbeat_off.swap(true, Ordering::SeqCst) {
                    say(
                        "bb-fake-server: the device route is refused (a line `heartbeat-off`): the heartbeat cannot start from now on",
                    );
                }
            }
            Ok(_) if line.trim() == UPLOADS_OFF_LINE => {
                if !switches.uploads_off.swap(true, Ordering::SeqCst) {
                    say(
                        "bb-fake-server: the upload routes are unreachable (a line `uploads-off`): they hang up from now on",
                    );
                }
            }
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
    let switches = Switches::default();
    let lines: LineWriter = Arc::new(say);
    let account = fixture_account();
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
    let fake = match start_fixture(listener, account, lines, served.clone(), switches.clone()) {
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
        switches,
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
            "192.0.2.13:80",
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
            Switches::default(),
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
        let wait = |text: &'static str| {
            wait_for_stop(io::Cursor::new(text), Duration::from_secs(10), Switches::default()).unwrap()
        };
        assert_eq!(wait(""), Stop::InputClosed);
        assert_eq!(wait("hello\nnot stop\n"), Stop::InputClosed);
        assert_eq!(wait("hello\n stop \nlater\n"), Stop::StopLine);
        let (_held, rx) = mpsc::channel::<()>();
        let open = io::BufReader::new(OpenInput(rx));
        assert_eq!(
            wait_for_stop(open, Duration::from_millis(300), Switches::default()).unwrap(),
            Stop::Deadline
        );
    }

    #[test]
    fn a_line_end_session_ends_the_session_and_only_that_line_does() {
        let ended_by = |text: &'static str| {
            let switches = Switches::default();
            let stop = wait_for_stop(io::Cursor::new(text), Duration::from_secs(10), switches.clone()).unwrap();
            (stop, switches.session_ended.load(Ordering::SeqCst))
        };
        assert_eq!(ended_by("hello\nend\nend-sessions\n"), (Stop::InputClosed, false));
        assert_eq!(ended_by(" end-session \n"), (Stop::InputClosed, true));
        assert_eq!(ended_by("end-session\nend-session\nstop\n"), (Stop::StopLine, true));
        // `stop` first: the server stops before it reads the line after it.
        assert_eq!(ended_by("stop\nend-session\n"), (Stop::StopLine, false));
    }

    #[test]
    fn an_ended_session_is_refused_on_every_call_with_a_token_and_the_sign_in_legs_are_not_touched() {
        let account = fixture();
        let switches = Switches::default();
        let ended = switches.session_ended.clone();
        let handler = fixture_handler(
            account.clone(),
            Arc::new(|_: &str| {}),
            Arc::new(AtomicU64::new(0)),
            switches,
        );
        let with_token = |method: &str, path: &str, body: Value| Request {
            method: method.into(),
            path: path.into(),
            headers: vec![("Authorization".into(), format!("Bearer {}", account.session_token))],
            body: body.to_string().into_bytes(),
        };
        let calls = [
            with_token("GET", "/api/v1/sync/snapshot", Value::Null),
            with_token("GET", "/api/v1/auth/me", Value::Null),
            with_token("POST", "/api/v1/clients/devices", json!({ "hostname": "fixture-pc" })),
        ];
        // A message OPAQUE cannot read, for the fixture's email: OPAQUE's own 400, before and after the end.
        let sign_in_leg = post(
            "/api/v1/opaque/login-start",
            json!({ "email": FIXTURE_EMAIL, "client_message": "AA==" }),
        );
        let before: Vec<u16> = calls.iter().map(|r| handler(r).status).collect();
        let leg_before = handler(&sign_in_leg).status;
        ended.store(true, Ordering::SeqCst);
        let after: Vec<u16> = calls.iter().map(|r| handler(r).status).collect();
        let leg_after = handler(&sign_in_leg).status;
        assert_eq!(
            (before, leg_before, after, leg_after),
            (vec![200, 200, 200], 400, vec![401, 401, 401], 400),
            "calls with the token before, the sign-in leg before, calls after the end, the sign-in leg after"
        );
    }
}
