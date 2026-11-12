//! A local fake of the Beebeeb API for tests: a loopback HTTP server whose routes are one closure, and a real
//! OPAQUE server side (core's `opaque_protocol`) for the sign-in and step-up legs.
//!
//! Tests only (`test` or the `test-support` feature). Invented accounts and keys; it never talks to another host.
//! Must not: be reachable from a release build, or hold anything a test did not put there.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use base64::Engine as _;
use beebeeb_core::opaque_protocol as opaque;
use serde_json::{Value, json};

/// One request the fake received.
#[derive(Debug, Clone)]
pub struct Request {
    pub method: String,
    /// Path with its query, as sent.
    pub path: String,
    headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// The bearer token of `Authorization`, if any.
    pub fn bearer(&self) -> Option<&str> {
        self.header("authorization")?.strip_prefix("Bearer ")
    }

    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }

    /// `"GET /api/v1/x"` (the query cut off): what a route matches on.
    pub fn route(&self) -> String {
        format!("{} {}", self.method, self.path.split('?').next().unwrap_or_default())
    }

    /// The value of the query parameter `name`.
    pub fn query(&self, name: &str) -> Option<&str> {
        let query = self.path.split_once('?')?.1;
        query
            .split('&')
            .filter_map(|pair| pair.split_once('='))
            .find(|(k, _)| *k == name)
            .map(|(_, v)| v)
    }
}

/// What the fake answers.
#[derive(Debug, Clone)]
pub struct Reply {
    pub status: u16,
    pub content_type: &'static str,
    pub body: Vec<u8>,
}

impl Reply {
    pub fn json(status: u16, body: Value) -> Self {
        Self {
            status,
            content_type: "application/json",
            body: body.to_string().into_bytes(),
        }
    }

    pub fn ok(body: Value) -> Self {
        Self::json(200, body)
    }

    pub fn bytes(status: u16, body: Vec<u8>) -> Self {
        Self {
            status,
            content_type: "application/octet-stream",
            body,
        }
    }

    pub fn empty(status: u16) -> Self {
        Self::json(status, Value::Null)
    }

    pub fn error(status: u16, code: &str) -> Self {
        Self::json(status, json!({ "error": code }))
    }
}

pub type Handler = Arc<dyn Fn(&Request) -> Reply + Send + Sync>;

/// The running fake. Dropping it stops the listener and joins every thread it started.
pub struct FakeApi {
    /// `http://127.0.0.1:<port>`, no trailing slash.
    pub base: String,
    log: Arc<Mutex<Vec<Request>>>,
    stop: Arc<AtomicBool>,
    accept: Option<JoinHandle<()>>,
}

impl FakeApi {
    pub fn start(handler: Handler) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind the fake API");
        listener.set_nonblocking(true).expect("nonblocking listener");
        let base = format!("http://{}", listener.local_addr().expect("local addr"));
        let log = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (log_in, stop_in) = (log.clone(), stop.clone());
        let accept = std::thread::Builder::new()
            .name("fake-api".into())
            .spawn(move || {
                let mut workers: Vec<JoinHandle<()>> = Vec::new();
                while !stop_in.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            let (handler, log) = (handler.clone(), log_in.clone());
                            workers.push(std::thread::spawn(move || serve(stream, &handler, &log)));
                        }
                        Err(_) => std::thread::sleep(std::time::Duration::from_millis(5)),
                    }
                    workers.retain(|w| !w.is_finished());
                }
                for worker in workers {
                    let _ = worker.join();
                }
            })
            .expect("spawn the fake API");
        Self {
            base,
            log,
            stop,
            accept: Some(accept),
        }
    }

    /// Every request so far, in arrival order.
    pub fn requests(&self) -> Vec<Request> {
        self.log.lock().expect("fake API log").clone()
    }

    /// The routes of every request so far (`"METHOD /path"`).
    pub fn routes(&self) -> Vec<String> {
        self.requests().iter().map(Request::route).collect()
    }
}

impl Drop for FakeApi {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(accept) = self.accept.take() {
            let _ = accept.join();
        }
    }
}

fn serve(mut stream: TcpStream, handler: &Handler, log: &Mutex<Vec<Request>>) {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(10)));
    let Some(request) = read_request(&mut stream) else {
        return;
    };
    log.lock().expect("fake API log").push(request.clone());
    let reply = handler(&request);
    let reason = if (200..300).contains(&reply.status) {
        "OK"
    } else {
        "ERR"
    };
    let date = chrono::Utc::now().format("%a, %d %b %Y %H:%M:%S GMT");
    let head = format!(
        "HTTP/1.1 {} {reason}\r\nDate: {date}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        reply.status,
        reply.content_type,
        reply.body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(&reply.body);
    let _ = stream.flush();
}

fn read_request(stream: &mut TcpStream) -> Option<Request> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 8192];
    let header_end = loop {
        let read = stream.read(&mut chunk).ok()?;
        if read == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(pos) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
            break pos + 4;
        }
    };
    let head = String::from_utf8_lossy(&buffer[..header_end]).into_owned();
    let mut lines = head.lines();
    let mut first = lines.next()?.split_whitespace();
    let method = first.next()?.to_string();
    let path = first.next()?.to_string();
    let headers: Vec<(String, String)> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(n, v)| (n.trim().to_string(), v.trim().to_string()))
        .collect();
    let length = headers
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = buffer[header_end..].to_vec();
    while body.len() < length {
        let read = stream.read(&mut chunk).ok()?;
        if read == 0 {
            return None;
        }
        body.extend_from_slice(&chunk[..read]);
    }
    body.truncate(length);
    Some(Request {
        method,
        path,
        headers,
        body,
    })
}

fn b64() -> base64::engine::general_purpose::GeneralPurpose {
    base64::engine::general_purpose::STANDARD
}

/// The server side of OPAQUE for ONE invented account: a server setup and the password file of its password, made
/// with core's own registration functions (the production suite, Argon2id included).
pub struct OpaqueAccount {
    setup: Vec<u8>,
    password_file: Vec<u8>,
    username: Vec<u8>,
    /// Server login states by the `server_state` handle given to the client.
    states: Mutex<HashMap<String, Vec<u8>>>,
}

impl OpaqueAccount {
    pub fn register(username: &str, password: &str) -> Self {
        let setup = opaque::create_server_setup();
        let start = opaque::client_registration_start(password.as_bytes()).expect("registration start");
        let response =
            opaque::server_registration_start(&setup, &start.message, username.as_bytes()).expect("server start");
        let upload = opaque::client_registration_finish(&start.state, password.as_bytes(), &response)
            .expect("registration finish");
        let password_file = opaque::server_registration_finish(&upload).expect("password file");
        Self {
            setup,
            password_file,
            username: username.as_bytes().to_vec(),
            states: Mutex::new(HashMap::new()),
        }
    }

    /// The start leg: `{client_message}` in, `{server_message, server_state, ksf_version}` out.
    pub fn login_start(&self, body: &Value) -> Reply {
        let Some(message) = body
            .get("client_message")
            .and_then(Value::as_str)
            .and_then(|m| b64().decode(m).ok())
        else {
            return Reply::error(400, "bad_request");
        };
        let Ok(started) = opaque::server_login_start(&self.setup, &self.password_file, &message, &self.username) else {
            return Reply::error(400, "bad_request");
        };
        let handle = uuid::Uuid::new_v4().to_string();
        self.states
            .lock()
            .expect("opaque states")
            .insert(handle.clone(), started.state);
        Reply::ok(json!({
            "server_message": b64().encode(&started.message),
            "server_state": handle,
            "ksf_version": 1,
        }))
    }

    /// The finish leg: whether `{client_message, server_state}` proves the password. A state is used once.
    pub fn login_finish(&self, body: &Value) -> bool {
        let state = body
            .get("server_state")
            .and_then(Value::as_str)
            .and_then(|handle| self.states.lock().expect("opaque states").remove(handle));
        let message = body
            .get("client_message")
            .and_then(Value::as_str)
            .and_then(|m| b64().decode(m).ok());
        match (state, message) {
            (Some(state), Some(message)) => opaque::server_login_finish(&state, &message).is_ok(),
            _ => false,
        }
    }
}

/// Single-use confirmation tokens, as the server mints them after a step-up.
#[derive(Default)]
pub struct ConfirmTokens(Mutex<Vec<String>>);

impl ConfirmTokens {
    pub fn mint(&self) -> String {
        let token = format!("confirm-{}", uuid::Uuid::new_v4());
        self.0.lock().expect("confirm tokens").push(token.clone());
        token
    }

    /// Spend the request's `X-Confirm-Token`: true once per minted token.
    pub fn spend(&self, request: &Request) -> bool {
        let Some(sent) = request.header("x-confirm-token") else {
            return false;
        };
        let mut tokens = self.0.lock().expect("confirm tokens");
        match tokens.iter().position(|t| t == sent) {
            Some(at) => {
                tokens.remove(at);
                true
            }
            None => false,
        }
    }
}
