//! A device sign-in on the fake account (wave 3, "a device sign-in relay in bb-fake-server"): one device asks to be
//! signed in by another, the fake issues the code, keeps the request and relays the approving device's sealed answer,
//! on the same wire as the API. Invented keys only; it rides on the server's own loopback listener; a request is gone
//! once its socket ends.
//!
//! The wire, as a client sees it:
//! 1. `GET /api/v1/auth/cli`: a WebSocket on the server's own port, no token. The first text frame is
//!    `{"ecdh_public_key_b64", "client", "client_version", "device_name", "os"}`: the requester's P-256 public key
//!    (SEC1 uncompressed, standard base64) and what it says about itself. The answer is `{"user_code": "XXXX-XXXX",
//!    "verification_uri", "expires_in"}`; the address holds no code. A key that is not a P-256 point is answered
//!    `{"error": "invalid_public_key"}` and the socket closes; so does a socket that sends no first frame in 10 s.
//! 2. The approving device, on its session: `GET /api/v1/auth/cli-pubkey?code=` answers `{"ecdh_public_key_b64",
//!    "request": {requested_at, expires_at, ip, country, user_agent, client, client_version, device_name, os}}`, and
//!    records that this account was shown the request; 404 for a code that is not waiting.
//! 3. Its password step-up for that code: `POST /api/v1/auth/confirm-opaque-start`, then `-finish` with
//!    `{"purpose": "cli_device_approval", "cli_code"}`: `{"confirmation_token", "expires_at"}`, bound to this account and
//!    this code, spent once.
//! 4. `POST /api/v1/auth/cli-session {"user_code"}` with that token as `X-Confirm-Token`: the claim, once per code: 201
//!    `{"session_id", "session_token", "expires_at", "key_state"}`. Without a token bound to this code and this account:
//!    403 `confirmation_required` (the refusal spends nothing); already claimed: 409 `cli_code_claimed`; not waiting: 404.
//! 5. `POST /api/v1/auth/cli-authorize {"user_code", "nonce_b64", "encrypted_payload_b64", "browser_ecdh_public_b64"}`:
//!    only from the account that holds the claim. Anyone else, an unknown or an ended code: 404, and the request keeps
//!    waiting. The socket then gets the three fields as sent and closes.
//! 6. `POST /api/v1/auth/cli-deny {"user_code"}`: from the account that holds the claim or, while nobody does, one that
//!    was shown the request (step 2): `{"denied": true}`, and the socket gets `{"error": "denied"}`. Anyone else: 404,
//!    nothing burnt. At the code's end the socket gets `{"error": "timeout"}`.
//!
//! Where the fake is simpler than the API:
//! - One account. The claim hands over the account's one session token (every route of the fake reads that one), not
//!   a session of its own; the "one token" rule of `server.rs` holds for a device sign-in too.
//! - A second, invented account exists for these routes only: [`STRANGER_USER_ID`], signed in on the second device
//!   with a token of this handler's own. It holds no key and has no password.
//! - The step-up legs are answered only while a request waits, and the finish only for this purpose; every other
//!   step-up is not served, as before this file.
//! - No count of wrong guesses, no rate limit, no notification, no audit entry. The address shown is the requester's
//!   loopback address; no country.
//!
//! The second device (loopback control, no token; the routes are in `server.rs`): [`DeviceSignIn::control`].
//!
//! Must not: write a code, a key, a token or a payload to a line or stderr; keep a request after its socket ended or
//! its code ran out.

use std::collections::HashMap;
use std::io::{self, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, mpsc};
use std::time::{Duration, Instant};

use base64::Engine as _;
use beebeeb_core::auth_flow::device::{Approval, ApproveStep, CLAIM_PATH, RELAY_PATH};
use beebeeb_core::auth_flow::{AuthFlowError, HttpRequest, HttpResponse, normalize_device_code};
use beebeeb_core::cli_auth::{CliEphemeralKey, encrypt_for_requester};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::handshake::derive_accept_key;
use tokio_tungstenite::tungstenite::protocol::Role;
use tokio_tungstenite::tungstenite::{self, Message, WebSocket};

use super::fake_account::FakeAccount;
use super::server::{
    DEVICE_APPROVE_ROUTE, DEVICE_CODE_LIFE_ROUTE, DEVICE_DENY_ROUTE, DEVICE_REQUESTS_ROUTE, access_line,
};
use super::{FAKE_API_MARK, Reply, Request, SocketTake};

/// The socket a device opens to ask.
pub const SOCKET_ROUTE: &str = "GET /api/v1/auth/cli";
const PUBKEY_ROUTE: &str = "GET /api/v1/auth/cli-pubkey";
const CLAIM_ROUTE: &str = "POST /api/v1/auth/cli-session";
const RELAY_ROUTE: &str = "POST /api/v1/auth/cli-authorize";
const DENY_ROUTE: &str = "POST /api/v1/auth/cli-deny";
const STEP_UP_START_ROUTE: &str = "POST /api/v1/auth/confirm-opaque-start";
const STEP_UP_FINISH_ROUTE: &str = "POST /api/v1/auth/confirm-opaque-finish";
/// The step-up purpose of an approval.
const PURPOSE: &str = "cli_device_approval";

/// The second, invented account of the second device's `"approver": "stranger"` (see the module doc).
pub const STRANGER_USER_ID: &str = "57a2a000-2475-4000-8000-000000000002";
/// Where the code is typed on the other device: a reserved name that can never be reached (RFC 2606), holding no code.
pub const VERIFICATION_URI: &str = "https://app.example.invalid/cli-auth";
/// A code's life when nothing else was asked for (the API's five minutes).
pub const DEFAULT_CODE_LIFE_SECS: u64 = 300;
/// The codes' letters: no 0, 1, I or O.
const CODE_ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
/// How long a socket may stay open without its first frame.
const FIRST_FRAME: Duration = Duration::from_secs(10);
/// The first frame is one small JSON object.
const MAX_FIRST_FRAME_BYTES: usize = 4096;
/// Every wait on a socket looks at the server's stop, the answer and the peer this often.
const POLL: Duration = Duration::from_millis(50);

/// A request that waits for the other device.
struct Waiting {
    public_key_b64: String,
    facts: Value,
    /// The account that claimed the code (step 4), if any.
    claimed_by: Option<String>,
    /// The accounts that were shown the request (step 2).
    viewed_by: Vec<String>,
    /// The socket's one answer: the sealed relay, or the denial.
    answer: mpsc::Sender<Value>,
    ends: Instant,
}

/// A step-up token for this purpose: the account and the code it was minted for.
struct Grant {
    token: String,
    user: String,
    code: String,
}

/// The device sign-in routes of ONE fake server (its own requests: two servers never share one).
pub struct DeviceSignIn {
    /// The password the second device approves with (the account's, public in `server.rs` for the fixture).
    approver_password: String,
    requests: Mutex<HashMap<String, Waiting>>,
    grants: Mutex<Vec<Grant>>,
    /// The session token of [`STRANGER_USER_ID`] on this server.
    stranger_token: String,
    code_life_secs: AtomicU64,
}

impl DeviceSignIn {
    /// The routes, with the second device approving with `approver_password`.
    pub fn new(approver_password: &str) -> Self {
        Self {
            approver_password: approver_password.to_string(),
            requests: Mutex::new(HashMap::new()),
            grants: Mutex::new(Vec::new()),
            stranger_token: format!("fake-stranger-session-{}", uuid::Uuid::new_v4()),
            code_life_secs: AtomicU64::new(DEFAULT_CODE_LIFE_SECS),
        }
    }

    /// How many requests wait now.
    pub fn waiting(&self) -> usize {
        self.live().len()
    }

    /// The requests that still wait (the ended ones are dropped here).
    fn live(&self) -> MutexGuard<'_, HashMap<String, Waiting>> {
        let mut requests = self.requests.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let now = Instant::now();
        requests.retain(|_, waiting| waiting.ends > now);
        requests
    }

    fn forget(&self, code: &str) {
        self.requests
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(code);
    }

    /// The account a session token belongs to.
    fn user(&self, account: &FakeAccount, request: &Request) -> Option<String> {
        match request.bearer()? {
            token if token == account.session_token => Some(account.user_id.clone()),
            token if token == self.stranger_token => Some(STRANGER_USER_ID.to_string()),
            _ => None,
        }
    }

    /// The answer to `request` when it is one of the API routes of a device sign-in (steps 2 to 6); `None` for any
    /// other route. The socket (step 1) is [`Self::socket`].
    pub fn answer(&self, account: &FakeAccount, request: &Request) -> Option<Reply> {
        let route = request.route();
        let reply = match route.as_str() {
            PUBKEY_ROUTE | CLAIM_ROUTE | RELAY_ROUTE | DENY_ROUTE => {
                let Some(user) = self.user(account, request) else {
                    return Some(Reply::error(401, "unauthorized"));
                };
                match route.as_str() {
                    PUBKEY_ROUTE => self.show(&user, request.query("code")),
                    CLAIM_ROUTE => self.claim(account, &user, request),
                    RELAY_ROUTE => self.relay(&user, &request.json()),
                    _ => self.deny(&user, &request.json()),
                }
            }
            STEP_UP_START_ROUTE if self.waiting() > 0 => match self.user(account, request) {
                Some(user) if user == account.user_id => account.opaque.login_start(&request.json()),
                _ => Reply::error(401, "invalid_credentials"),
            },
            STEP_UP_FINISH_ROUTE if is_device_step_up(&request.json()) => self.step_up_finish(account, request),
            _ => return None,
        };
        Some(reply)
    }

    /// Step 2: the request a code names, shown to `user`.
    fn show(&self, user: &str, code: Option<&str>) -> Reply {
        let Some(code) = code.and_then(normalize_device_code) else {
            return not_waiting();
        };
        let mut requests = self.live();
        let Some(waiting) = requests.get_mut(&code) else {
            return not_waiting();
        };
        if !waiting.viewed_by.iter().any(|seen| seen == user) {
            waiting.viewed_by.push(user.to_string());
        }
        Reply::ok(json!({ "ecdh_public_key_b64": waiting.public_key_b64, "request": waiting.facts }))
    }

    /// Step 3, the finish leg for this purpose: the password proven, a token bound to this account and this code.
    fn step_up_finish(&self, account: &FakeAccount, request: &Request) -> Reply {
        let body = request.json();
        if self.user(account, request).as_deref() != Some(account.user_id.as_str()) {
            return Reply::error(401, "invalid_credentials");
        }
        if body
            .get("purpose")
            .and_then(Value::as_str)
            .is_some_and(|p| p != PURPOSE)
        {
            return Reply::error(400, "cli_code is only valid with purpose cli_device_approval");
        }
        let Some(code) = body
            .get("cli_code")
            .and_then(Value::as_str)
            .and_then(normalize_device_code)
        else {
            return Reply::error(400, "purpose cli_device_approval needs a cli_code");
        };
        if !account.opaque.login_finish(&body) {
            return Reply::error(401, "invalid_credentials");
        }
        let token = format!("fake-confirm-{}", uuid::Uuid::new_v4());
        self.grants
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(Grant {
                token: token.clone(),
                user: account.user_id.clone(),
                code,
            });
        let expires_at = (chrono::Utc::now() + chrono::Duration::seconds(300)).to_rfc3339();
        Reply::ok(json!({ "confirmation_token": token, "expires_at": expires_at }))
    }

    /// Spend the step-up token `sent` when it was minted for `user` and `code`; a token for anyone or anything else
    /// is not spent.
    fn spend_grant(&self, sent: Option<&str>, user: &str, code: &str) -> bool {
        let Some(sent) = sent else {
            return false;
        };
        let mut grants = self.grants.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(at) = grants
            .iter()
            .position(|grant| grant.token == sent && grant.user == user && grant.code == code)
        else {
            return false;
        };
        grants.remove(at);
        true
    }

    /// Step 4: the claim.
    fn claim(&self, account: &FakeAccount, user: &str, request: &Request) -> Reply {
        let body = request.json();
        let Some(code) = body
            .get("user_code")
            .and_then(Value::as_str)
            .and_then(normalize_device_code)
        else {
            return not_waiting();
        };
        if !self.spend_grant(request.header("x-confirm-token"), user, &code) {
            return Reply::json(
                403,
                json!({ "error": "confirmation_required", "code": "confirmation_required" }),
            );
        }
        let mut requests = self.live();
        let Some(waiting) = requests.get_mut(&code) else {
            return not_waiting();
        };
        if waiting.claimed_by.is_some() {
            return Reply::json(
                409,
                json!({
                    "code": "cli_code_claimed",
                    "error": "This code has already been used. Run the sign-in on your device again.",
                }),
            );
        }
        waiting.claimed_by = Some(user.to_string());
        // The fake account's one session token (see the module doc); the stranger's device gets a token of nothing.
        let session_token = if user == account.user_id {
            account.session_token.clone()
        } else {
            format!("fake-unused-session-{}", uuid::Uuid::new_v4())
        };
        let expires_at = (chrono::Utc::now() + chrono::Duration::days(30)).to_rfc3339();
        Reply::json(
            201,
            json!({
                "session_id": uuid::Uuid::new_v4().to_string(),
                "session_token": session_token,
                "expires_at": expires_at,
                "key_state": "legacy",
            }),
        )
    }

    /// Step 5: the relay, from the claim's holder only.
    fn relay(&self, user: &str, body: &Value) -> Reply {
        let Some(code) = body
            .get("user_code")
            .and_then(Value::as_str)
            .and_then(normalize_device_code)
        else {
            return not_waiting();
        };
        let field = |name: &str| body.get(name).and_then(Value::as_str).map(str::to_string);
        let (Some(nonce), Some(payload), Some(approver)) = (
            field("nonce_b64"),
            field("encrypted_payload_b64"),
            field("browser_ecdh_public_b64"),
        ) else {
            return Reply::error(422, "nonce_b64, encrypted_payload_b64 and browser_ecdh_public_b64");
        };
        let mut requests = self.live();
        // Anyone but the holder of the claim, an unknown or an ended code: the same 404, and nothing is burnt.
        let holds = requests
            .get(&code)
            .is_some_and(|waiting| waiting.claimed_by.as_deref() == Some(user));
        if !holds {
            return not_waiting();
        }
        let Some(waiting) = requests.remove(&code) else {
            return not_waiting();
        };
        let _ = waiting.answer.send(json!({
            "nonce_b64": nonce,
            "encrypted_payload_b64": payload,
            "browser_ecdh_public_b64": approver,
        }));
        Reply::ok(json!({ "authorized": true }))
    }

    /// Step 6: the denial.
    fn deny(&self, user: &str, body: &Value) -> Reply {
        let Some(code) = body
            .get("user_code")
            .and_then(Value::as_str)
            .and_then(normalize_device_code)
        else {
            return not_waiting();
        };
        let mut requests = self.live();
        let may = requests.get(&code).is_some_and(|waiting| match &waiting.claimed_by {
            Some(holder) => holder == user,
            None => waiting.viewed_by.iter().any(|seen| seen == user),
        });
        if !may {
            return not_waiting();
        }
        if let Some(waiting) = requests.remove(&code) {
            let _ = waiting.answer.send(json!({ "error": "denied" }));
        }
        Reply::ok(json!({ "denied": true }))
    }

    /// Step 1: the socket, when `request` is its upgrade. The connection is the returned closure's from then on.
    pub fn socket(self: &Arc<Self>, request: &Request) -> Option<SocketTake> {
        if request.route() != SOCKET_ROUTE
            || !request
                .header("upgrade")
                .is_some_and(|value| value.trim().eq_ignore_ascii_case("websocket"))
        {
            return None;
        }
        let key = request.header("sec-websocket-key")?.trim().to_string();
        let user_agent = request.header("user-agent").map(|agent| clean(agent, 256));
        let this = self.clone();
        Some(Box::new(move |stream: TcpStream, stop: &AtomicBool| {
            this.serve_socket(stream, &key, user_agent, stop);
        }))
    }

    fn serve_socket(&self, mut stream: TcpStream, key: &str, user_agent: Option<String>, stop: &AtomicBool) {
        let address = stream.peer_addr().map(|peer| peer.ip().to_string()).unwrap_or_default();
        let head = format!(
            "HTTP/1.1 101 Switching Protocols\r\nServer: {FAKE_API_MARK}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n",
            derive_accept_key(key.as_bytes())
        );
        if stream.write_all(head.as_bytes()).and_then(|()| stream.flush()).is_err() {
            return;
        }
        // Short reads from here on: every wait looks at the server's stop between them.
        let _ = stream.set_read_timeout(Some(POLL));
        let mut socket = WebSocket::from_raw_socket(stream, Role::Server, None);
        let Some(first) = first_text(&mut socket, stop) else {
            close(&mut socket);
            return;
        };
        let init: Value = serde_json::from_str(&first).unwrap_or(Value::Null);
        let public = init
            .get("ecdh_public_key_b64")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if !is_p256_point(public) {
            let _ = socket.send(Message::Text(json!({ "error": "invalid_public_key" }).to_string()));
            close(&mut socket);
            return;
        }
        let (code, life, answers) = self.open_request(public, &init, address, user_agent);
        let issued = json!({ "user_code": code, "verification_uri": VERIFICATION_URI, "expires_in": life });
        if socket.send(Message::Text(issued.to_string())).is_err() {
            self.forget(&code);
            return;
        }
        let ends = Instant::now() + Duration::from_secs(life);
        let answer = loop {
            if stop.load(Ordering::SeqCst) {
                self.forget(&code);
                close(&mut socket);
                return;
            }
            // The requester went away: its request goes with it (a new socket is a new code).
            match socket.read() {
                Ok(Message::Close(_)) => {
                    self.forget(&code);
                    close(&mut socket);
                    return;
                }
                Ok(_) => {}
                Err(tungstenite::Error::Io(error)) if is_timeout(&error) => {}
                Err(_) => {
                    self.forget(&code);
                    return;
                }
            }
            match answers.try_recv() {
                Ok(answer) => break answer,
                // The code ran out: the waiting request was dropped with its sender, or is dropped now.
                Err(mpsc::TryRecvError::Disconnected) => break json!({ "error": "timeout" }),
                Err(mpsc::TryRecvError::Empty) if Instant::now() >= ends => {
                    self.forget(&code);
                    break json!({ "error": "timeout" });
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        };
        let _ = socket.send(Message::Text(answer.to_string()));
        close(&mut socket);
    }

    /// Keep a new request for `public` (a P-256 point) and issue its code: the code, its life, and where its one
    /// answer will arrive.
    fn open_request(
        &self,
        public: &str,
        init: &Value,
        address: String,
        user_agent: Option<String>,
    ) -> (String, u64, mpsc::Receiver<Value>) {
        let life = self.code_life_secs.load(Ordering::SeqCst);
        let now = chrono::Utc::now();
        let said = |name: &str, cap: usize| {
            init.get(name)
                .and_then(Value::as_str)
                .map(|text| clean(text, cap))
                .filter(|text| !text.is_empty())
        };
        let facts = json!({
            "requested_at": now.to_rfc3339(),
            "expires_at": (now + chrono::Duration::seconds(life as i64)).to_rfc3339(),
            "ip": address,
            "country": Value::Null,
            "user_agent": user_agent,
            "client": said("client", 16).filter(|client| client == "desktop" || client == "cli"),
            "client_version": said("client_version", 32),
            "device_name": said("device_name", 64),
            "os": said("os", 16),
        });
        let (answer, answers) = mpsc::channel();
        let mut requests = self.live();
        let code = loop {
            let code = new_code();
            if !requests.contains_key(&code) {
                break code;
            }
        };
        requests.insert(
            code.clone(),
            Waiting {
                public_key_b64: public.to_string(),
                facts,
                claimed_by: None,
                viewed_by: Vec::new(),
                answer,
                ends: Instant::now() + Duration::from_secs(life),
            },
        );
        (code, life, answers)
    }

    /// The second device's control routes (no token, loopback only), `None` for any other route:
    /// - `POST /fake-control/device-approve` `{"code"?, "approver"?}`: the second device approves the request `code`
    ///   names (no code: the one request that waits; 404 when none does, 409 when several do) the way an approving
    ///   app does: it is shown the request, steps up with the password for that code, claims it and relays the
    ///   account's key sealed to the requester's key. 200 `{"approved": true}`; a refusal answers its status and
    ///   `{"approved": false, "refused": <the step's code>}`. With `"approver": "stranger"` the second device is
    ///   signed in to ANOTHER account (no password, no key): it is shown the request, its claim (no step-up of its own)
    ///   and its relay are refused, and the request keeps waiting: 403 `{"approved": false, "claim", "relay"}`.
    /// - `POST /fake-control/device-deny` `{"code"?, "approver"?}`: it is shown the request and denies it (DW-A09).
    /// - `POST /fake-control/device-code-life` `{"seconds": 1 to 300}`: the life of every code issued after it (DW-A10).
    /// - `GET /fake-control/device-requests`: `{"waiting": <how many requests wait>}`, never a code.
    ///
    /// Each request the second device sends is written to `lines` as an access line, prefixed `second device:`.
    pub fn control(&self, account: &FakeAccount, request: &Request, lines: &dyn Fn(&str)) -> Option<Reply> {
        let reply = match request.route().as_str() {
            DEVICE_REQUESTS_ROUTE => Reply::ok(json!({ "waiting": self.waiting() })),
            DEVICE_CODE_LIFE_ROUTE => {
                let seconds = request
                    .json()
                    .get("seconds")
                    .and_then(Value::as_u64)
                    .filter(|seconds| (1..=DEFAULT_CODE_LIFE_SECS).contains(seconds));
                let Some(seconds) = seconds else {
                    return Some(Reply::error(400, "seconds: 1 to 300"));
                };
                self.code_life_secs.store(seconds, Ordering::SeqCst);
                Reply::ok(json!({ "seconds": seconds }))
            }
            DEVICE_APPROVE_ROUTE => self.second_device(account, &request.json(), lines, false),
            DEVICE_DENY_ROUTE => self.second_device(account, &request.json(), lines, true),
            _ => return None,
        };
        Some(reply)
    }

    fn second_device(&self, account: &FakeAccount, body: &Value, lines: &dyn Fn(&str), deny: bool) -> Reply {
        let code = match body.get("code").and_then(Value::as_str) {
            Some(typed) => match normalize_device_code(typed) {
                Some(code) => code,
                None => return Reply::error(400, "code: XXXX-XXXX"),
            },
            None => {
                let codes: Vec<String> = self.live().keys().cloned().collect();
                match codes.as_slice() {
                    [one] => one.clone(),
                    [] => return Reply::error(404, "no request is waiting"),
                    _ => return Reply::error(409, "more than one request is waiting: name its code"),
                }
            }
        };
        let stranger = match body.get("approver").and_then(Value::as_str) {
            None | Some("fixture") => false,
            Some("stranger") => true,
            Some(_) => return Reply::error(400, "approver: fixture or stranger"),
        };
        let token = if stranger {
            self.stranger_token.clone()
        } else {
            account.session_token.clone()
        };
        let send = |request: HttpRequest| self.send_as(account, &token, request, lines);
        let said = if deny { "denied" } else { "approved" };
        if stranger && !deny {
            return self.stranger_approves(&code, &send);
        }
        let outcome = (|| -> Result<(), AuthFlowError> {
            let (lookup, request) = Approval::lookup(&code)?;
            let pending = lookup.on_response(send(request))?;
            if deny {
                let (denial, request) = pending.deny();
                return denial.on_response(send(request));
            }
            let (mut run, mut request) =
                pending.approve(&code, self.approver_password.as_bytes(), &account.master_key)?;
            loop {
                match run.on_response(send(request))? {
                    ApproveStep::Send(next) => request = next,
                    ApproveStep::Approved => return Ok(()),
                }
            }
        })();
        let mut answer = serde_json::Map::new();
        answer.insert(said.to_string(), Value::Bool(outcome.is_ok()));
        match outcome {
            Ok(()) => Reply::ok(Value::Object(answer)),
            Err(error) => {
                answer.insert("refused".to_string(), Value::from(error.code()));
                Reply::json(refusal_status(&error), Value::Object(answer))
            }
        }
    }

    /// The stranger's approval: shown the request (as any signed-in account may be), then a claim with no step-up of
    /// its own and a relay sealed to the requester's key anyway. Both must be refused.
    fn stranger_approves(&self, code: &str, send: &dyn Fn(HttpRequest) -> HttpResponse) -> Reply {
        let shown = match Approval::lookup(code) {
            Ok((_, request)) => send(request),
            Err(error) => {
                return Reply::json(
                    refusal_status(&error),
                    json!({ "approved": false, "refused": error.code() }),
                );
            }
        };
        let Some(public) = shown
            .body
            .get("ecdh_public_key_b64")
            .and_then(Value::as_str)
            .and_then(|b64| base64::engine::general_purpose::STANDARD.decode(b64).ok())
        else {
            return Reply::json(404, json!({ "approved": false, "refused": "request_not_found" }));
        };
        let claim = send(HttpRequest::session_post(
            CLAIM_PATH.to_string(),
            json!({ "user_code": code }),
        ));
        let Ok(sealed) = encrypt_for_requester(&public, b"{}") else {
            return Reply::json(404, json!({ "approved": false, "refused": "request_not_found" }));
        };
        let b64 = base64::engine::general_purpose::STANDARD;
        let relay = send(HttpRequest::session_post(
            RELAY_PATH.to_string(),
            json!({
                "user_code": code,
                "nonce_b64": b64.encode(sealed.nonce),
                "encrypted_payload_b64": b64.encode(&sealed.ciphertext),
                "browser_ecdh_public_b64": b64.encode(&sealed.approver_public),
            }),
        ));
        let status = if claim.ok() || relay.ok() { 200 } else { 403 };
        Reply::json(
            status,
            json!({ "approved": relay.ok(), "claim": claim.status, "relay": relay.status }),
        )
    }

    /// One request of the second device, on `token`, answered by these routes (as the server would over loopback),
    /// with its access line.
    fn send_as(&self, account: &FakeAccount, token: &str, request: HttpRequest, lines: &dyn Fn(&str)) -> HttpResponse {
        let mut headers = vec![("Authorization".to_string(), format!("Bearer {token}"))];
        if let Some(confirm) = request.confirm_token_header() {
            headers.push(("X-Confirm-Token".to_string(), confirm.to_string()));
        }
        let request = Request {
            method: request.method.as_str().to_string(),
            path: request.path.clone(),
            headers,
            body: request
                .body
                .as_ref()
                .map(|body| body.to_string().into_bytes())
                .unwrap_or_default(),
        };
        let reply = self
            .answer(account, &request)
            .unwrap_or_else(|| Reply::error(404, "not_found"));
        lines(&format!("second device: {}", access_line(&request, reply.status)));
        HttpResponse::new(reply.status, serde_json::from_slice(&reply.body).unwrap_or(Value::Null))
    }
}

fn not_waiting() -> Reply {
    Reply::error(404, "session not found or expired")
}

/// A finish leg of this purpose (it names the purpose or a code).
fn is_device_step_up(body: &Value) -> bool {
    body.get("purpose").and_then(Value::as_str) == Some(PURPOSE) || body.get("cli_code").is_some()
}

/// The control's status for a refused step of the second device.
fn refusal_status(error: &AuthFlowError) -> u16 {
    match error {
        AuthFlowError::RequestNotFound => 404,
        AuthFlowError::AlreadyHandled => 409,
        AuthFlowError::Expired => 410,
        AuthFlowError::InvalidCredentials => 401,
        AuthFlowError::InvalidInput(_) => 400,
        _ => 502,
    }
}

/// `text` without control characters, at most `cap` characters, trimmed.
fn clean(text: &str, cap: usize) -> String {
    text.chars()
        .filter(|c| !c.is_control())
        .take(cap)
        .collect::<String>()
        .trim()
        .to_string()
}

/// A new `XXXX-XXXX` code.
fn new_code() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let mut letter = || CODE_ALPHABET[rng.gen_range(0..CODE_ALPHABET.len())] as char;
    let first: String = (0..4).map(|_| letter()).collect();
    let second: String = (0..4).map(|_| letter()).collect();
    format!("{first}-{second}")
}

/// Whether `b64` is standard base64 of an uncompressed P-256 point that is on the curve.
fn is_p256_point(b64: &str) -> bool {
    let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(b64) else {
        return false;
    };
    bytes.len() == 65 && bytes[0] == 0x04 && CliEphemeralKey::generate().shared_secret(&bytes).is_ok()
}

/// A read that ran into the socket's short timeout (`WouldBlock` on Linux, `TimedOut` on Windows).
fn is_timeout(error: &io::Error) -> bool {
    matches!(error.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut)
}

/// The first text frame, within [`FIRST_FRAME`] and before the server's stop; `None` for anything else.
fn first_text(socket: &mut WebSocket<TcpStream>, stop: &AtomicBool) -> Option<String> {
    let deadline = Instant::now() + FIRST_FRAME;
    while !stop.load(Ordering::SeqCst) && Instant::now() < deadline {
        match socket.read() {
            Ok(Message::Text(text)) => return (text.len() <= MAX_FIRST_FRAME_BYTES).then_some(text),
            Ok(Message::Close(_)) => return None,
            Ok(_) => {}
            Err(tungstenite::Error::Io(error)) if is_timeout(&error) => {}
            Err(_) => return None,
        }
    }
    None
}

/// Close the socket: the close frame, then the peer's (or a second at most).
fn close(socket: &mut WebSocket<TcpStream>) {
    let _ = socket.close(None);
    let until = Instant::now() + Duration::from_secs(1);
    while Instant::now() < until {
        match socket.read() {
            Ok(_) => {}
            Err(tungstenite::Error::Io(error)) if is_timeout(&error) => {}
            Err(_) => break,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::OnceLock;

    use crate::test_api::server::{FIXTURE_PASSWORD, fixture_account};

    fn fixture() -> Arc<FakeAccount> {
        static ACCOUNT: OnceLock<Arc<FakeAccount>> = OnceLock::new();
        ACCOUNT.get_or_init(fixture_account).clone()
    }

    fn call(token: &str, route: &str, body: Value, confirm: Option<&str>) -> Request {
        let (method, path) = route.split_once(' ').unwrap();
        let mut headers = vec![("Authorization".to_string(), format!("Bearer {token}"))];
        if let Some(confirm) = confirm {
            headers.push(("X-Confirm-Token".to_string(), confirm.to_string()));
        }
        Request {
            method: method.into(),
            path: path.into(),
            headers,
            body: body.to_string().into_bytes(),
        }
    }

    /// A request kept as the socket keeps one, for a real P-256 key.
    fn waiting(device: &DeviceSignIn) -> (String, mpsc::Receiver<Value>) {
        let public = base64::engine::general_purpose::STANDARD.encode(CliEphemeralKey::generate().public_key_bytes());
        let (code, _, answers) =
            device.open_request(&public, &json!({ "client": "desktop" }), "127.0.0.1".into(), None);
        (code, answers)
    }

    fn grant(device: &DeviceSignIn, user: &str, code: &str) -> String {
        let token = format!("fake-confirm-{}", uuid::Uuid::new_v4());
        device.grants.lock().unwrap().push(Grant {
            token: token.clone(),
            user: user.to_string(),
            code: code.to_string(),
        });
        token
    }

    fn relay_body(code: &str) -> Value {
        json!({ "user_code": code, "nonce_b64": "bm9uY2U=", "encrypted_payload_b64": "c2VhbGVk", "browser_ecdh_public_b64": "a2V5" })
    }

    #[test]
    fn only_the_claim_holder_relays_and_a_refused_claim_or_relay_burns_nothing() {
        let account = fixture();
        let device = DeviceSignIn::new(FIXTURE_PASSWORD);
        let (code, answers) = waiting(&device);
        let (owner, stranger) = (account.session_token.clone(), device.stranger_token.clone());
        let status = |request: Request| device.answer(&account, &request).unwrap().status;

        // A claim with no token, with a token minted for another code, and with the owner's token sent by the stranger.
        let other_code = grant(&device, &account.user_id, "ABCD-EFGH");
        let owners = grant(&device, &account.user_id, &code);
        let claims = [
            status(call(&owner, CLAIM_ROUTE, json!({ "user_code": code }), None)),
            status(call(
                &owner,
                CLAIM_ROUTE,
                json!({ "user_code": code }),
                Some(&other_code),
            )),
            status(call(
                &stranger,
                CLAIM_ROUTE,
                json!({ "user_code": code }),
                Some(&owners),
            )),
        ];
        assert_eq!(claims, [403, 403, 403]);
        // The stranger relays before anyone claimed: refused, nothing burnt.
        assert_eq!(status(call(&stranger, RELAY_ROUTE, relay_body(&code), None)), 404);
        // The owner claims with its token for this code (not spent by the refusals above), once.
        assert_eq!(
            status(call(&owner, CLAIM_ROUTE, json!({ "user_code": code }), Some(&owners))),
            201
        );
        let again = grant(&device, &account.user_id, &code);
        assert_eq!(
            status(call(&owner, CLAIM_ROUTE, json!({ "user_code": code }), Some(&again))),
            409
        );
        // The stranger relays a claimed code that is not its own: refused, nothing burnt.
        assert_eq!(status(call(&stranger, RELAY_ROUTE, relay_body(&code), None)), 404);
        assert_eq!(device.waiting(), 1);
        assert!(answers.try_recv().is_err(), "nothing reached the socket");
        // The holder relays: the socket gets the three fields as sent, and the code is gone.
        assert_eq!(status(call(&owner, RELAY_ROUTE, relay_body(&code), None)), 200);
        let relayed = answers.try_recv().expect("the relay reached the socket");
        assert_eq!(
            (
                relayed["nonce_b64"].as_str(),
                relayed["browser_ecdh_public_b64"].as_str()
            ),
            (Some("bm9uY2U="), Some("a2V5"))
        );
        assert_eq!(device.waiting(), 0);
        assert_eq!(status(call(&owner, RELAY_ROUTE, relay_body(&code), None)), 404);
    }

    #[test]
    fn only_a_viewer_or_the_holder_denies_and_the_step_up_is_served_only_while_a_request_waits() {
        let account = fixture();
        let device = DeviceSignIn::new(FIXTURE_PASSWORD);
        let start = call(
            &account.session_token,
            STEP_UP_START_ROUTE,
            json!({ "client_message": "AA==" }),
            None,
        );
        assert!(
            device.answer(&account, &start).is_none(),
            "no request waits: the step-up is not served here"
        );
        let (code, answers) = waiting(&device);
        assert_eq!(
            device.answer(&account, &start).map(|reply| reply.status),
            Some(400),
            "served: OPAQUE reads the message"
        );
        // A generic finish is never this file's.
        let generic = call(
            &account.session_token,
            STEP_UP_FINISH_ROUTE,
            json!({ "server_state": "x" }),
            None,
        );
        assert!(device.answer(&account, &generic).is_none());

        let deny = |token: &str| {
            device
                .answer(&account, &call(token, DENY_ROUTE, json!({ "user_code": code }), None))
                .unwrap()
                .status
        };
        // The stranger was never shown it: refused, nothing burnt.
        assert_eq!(deny(&device.stranger_token.clone()), 404);
        assert_eq!(device.waiting(), 1);
        // Shown (step 2), it may deny while nobody holds the claim.
        let shown = device
            .answer(
                &account,
                &call(
                    &device.stranger_token.clone(),
                    &format!("GET /api/v1/auth/cli-pubkey?code={}", code.to_lowercase()),
                    Value::Null,
                    None,
                ),
            )
            .unwrap();
        assert_eq!(shown.status, 200);
        assert_eq!(deny(&device.stranger_token.clone()), 200);
        assert_eq!(answers.try_recv().unwrap(), json!({ "error": "denied" }));
        assert_eq!(device.waiting(), 0);
    }
}
