//! Slice K5: sign in on this PC by approving it on another device. The engine's fake account answers HTTP only (its
//! profile, the logout), so the server's socket is this file's own: a loopback `tokio::net::TcpListener` with
//! `tokio_tungstenite::accept_async`, scripted by the test (the code, then the answer). The approving side is core's own
//! (`encrypt_for_requester`). The requests run through `device_request_at` with the socket's `ws://` address; the
//! address a handle derives from its API base is `ws_url`'s own test. Every account, name and code is invented.

use std::path::PathBuf;
use std::pin::Pin;
use std::sync::mpsc;
use std::task::{Context, Poll};
use std::time::Instant;

use beebeeb_core::cli_auth::encrypt_for_requester;
use serde_json::json;

use super::*;
use crate::account::SessionTransition;
use crate::app::EngineConfig;
use crate::host::ChannelHost;
use crate::identity::TEST;
use crate::keychain::SessionToken;
use crate::signin::gate::GateState;
use crate::signin::memory_store::MemoryCredentials;
use crate::test_api::FakeApi;
use crate::test_api::fake_account::{FakeAccount, TEST_PHRASE, account_handler};

type TestEngine = Engine<MemoryCredentials>;

/// The code the fake server issues, and the page its address opens (no code in it).
const CODE: &str = "KQ7M-4TXP";
const PAGE: &str = "https://app.example.eu/cli-auth";
/// The longest a step of a test waits for the other side.
const WAIT: Duration = Duration::from_secs(10);

/// One handle on the fake account, with a local state database of its own (none on disk until a test makes one).
struct Rig {
    engine: Arc<TestEngine>,
    account: Arc<FakeAccount>,
    fake: FakeApi,
    local_state: PathBuf,
    _dir: tempfile::TempDir,
}

fn rig(name: &str) -> Rig {
    let (account, _phrase) = FakeAccount::register(&format!("{name}@example.eu"), "an invented password", TEST_PHRASE);
    let account = Arc::new(account);
    let fake = FakeApi::start(account_handler(account.clone()));
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: format!("device-request-{name}"),
        state_dir: crate::state_paths::init_for_test(),
        api_base: fake.base.clone(),
    };
    let engine = Arc::new(Engine::open(&TEST, config, host, MemoryCredentials::default()).expect("the handle opens"));
    let dir = tempfile::tempdir().unwrap();
    let local_state = dir.path().join("state.db");
    engine.use_local_state_for_test(local_state.clone());
    Rig {
        engine,
        account,
        fake,
        local_state,
        _dir: dir,
    }
}

/// What the test tells its socket to do next.
enum Say {
    Text(String),
    Close,
}

/// The server's socket: one connection, its first frame handed to the test, then what the test says.
struct Socket {
    url: String,
    init: mpsc::Receiver<String>,
    say: tokio::sync::mpsc::UnboundedSender<Say>,
    _runtime: tokio::runtime::Runtime,
}

fn ws_server() -> Socket {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let listener = runtime.block_on(tokio::net::TcpListener::bind("127.0.0.1:0")).unwrap();
    let address = listener.local_addr().unwrap();
    let (init_tx, init) = mpsc::channel();
    let (say, mut said) = tokio::sync::mpsc::unbounded_channel::<Say>();
    runtime.spawn(async move {
        let Ok((stream, _)) = listener.accept().await else {
            return;
        };
        let Ok(mut ws) = tokio_tungstenite::accept_async(stream).await else {
            return;
        };
        while let Some(Ok(message)) = ws.next().await {
            if let Message::Text(text) = message {
                let _ = init_tx.send(text);
                break;
            }
        }
        while let Some(next) = said.recv().await {
            match next {
                Say::Text(text) => {
                    if ws.send(Message::Text(text)).await.is_err() {
                        return;
                    }
                }
                Say::Close => {
                    let _ = ws.send(Message::Close(None)).await;
                    return;
                }
            }
        }
    });
    Socket {
        url: format!("ws://{address}{DEVICE_SOCKET_PATH}"),
        init,
        say,
        _runtime: runtime,
    }
}

/// A request on a thread of its own, as a host runs it.
struct Running {
    events: mpsc::Receiver<DeviceRequestEvent>,
    done: mpsc::Receiver<Result<DeviceSignedIn, DeviceRequestRefused>>,
    stop: Arc<AtomicBool>,
}

fn start(rig: &Rig, socket: &Socket) -> Running {
    let turn = rig.engine.session_turn();
    let (events_tx, events) = mpsc::channel();
    let (done_tx, done) = mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    let (engine, url, stop_in) = (rig.engine.clone(), socket.url.clone(), stop.clone());
    std::thread::spawn(move || {
        let tell = move |event: DeviceRequestEvent| {
            let _ = events_tx.send(event);
        };
        let result = engine.device_request_at(&url, turn, &tell, &stop_in);
        engine.publish();
        let _ = done_tx.send(result);
    });
    Running { events, done, stop }
}

fn code_frame(expires_in: u64) -> String {
    json!({ "user_code": CODE, "verification_uri": PAGE, "expires_in": expires_in }).to_string()
}

/// The server's half up to the code: the requester's first frame (returned, parsed), then the code, which the host
/// hears first.
fn issue_code(socket: &Socket, running: &Running, expires_in: u64) -> serde_json::Value {
    let init = socket
        .init
        .recv_timeout(WAIT)
        .expect("the requester sent its first frame");
    socket.say.send(Say::Text(code_frame(expires_in))).unwrap();
    match running.events.recv_timeout(WAIT).expect("the code reached the host") {
        DeviceRequestEvent::Code {
            code,
            verification_uri,
            expires_in: life,
        } => assert_eq!(
            (code.as_str(), verification_uri.as_str(), life),
            (CODE, PAGE, expires_in)
        ),
        other => panic!("the code comes first, got {other:?}"),
    }
    serde_json::from_str(&init).expect("the first frame is json")
}

/// What the approving device relays: `{session_token, master_key_b64}` sealed to the requester's key by core.
fn sealed_answer(init: &serde_json::Value, token: &str, key: &[u8; 32]) -> String {
    let public = B64.decode(init["ecdh_public_key_b64"].as_str().unwrap()).unwrap();
    let plaintext = json!({
        "session_token": token,
        "master_key_b64": B64.encode(key),
        "email": "not-read@example.eu",
    })
    .to_string();
    let relay = encrypt_for_requester(&public, plaintext.as_bytes()).unwrap();
    json!({
        "nonce_b64": B64.encode(relay.nonce),
        "encrypted_payload_b64": B64.encode(&relay.ciphertext),
        "browser_ecdh_public_b64": B64.encode(&relay.approver_public),
    })
    .to_string()
}

fn finish(running: &Running) -> Result<DeviceSignedIn, DeviceRequestRefused> {
    running.done.recv_timeout(WAIT).expect("the request ended")
}

fn asked(rig: &Rig, route: &str) -> usize {
    rig.fake.routes().iter().filter(|seen| seen.ends_with(route)).count()
}

fn stored_token(rig: &Rig) -> Option<String> {
    rig.engine
        .vault()
        .session_token()
        .unwrap()
        .map(|token| token.expose_for_request().to_string())
}

fn nothing_stored(rig: &Rig) {
    assert_eq!(stored_token(rig), None, "no session token was stored");
    assert!(rig.engine.acct.session.lock().unwrap().is_none(), "no key is in memory");
    assert_eq!(rig.engine.gate(), GateState::SignedOut);
}

fn signed_in() -> Result<DeviceSignedIn, DeviceRequestRefused> {
    Ok(DeviceSignedIn {
        by: ANOTHER_DEVICE.to_string(),
        vault_unlocked: true,
    })
}

// ── The rules (each with its mutant in the branch notes) ─────────────────────────────────────────────────────────────

/// Rule 1, the payload half (M1: `read_answer` reads the payload bytes without `decrypt_payload`). The first answer holds
/// credentials the fake accepts, NOT sealed: a request that skipped the opening would read them and sign in. The same
/// credentials sealed to the request's key sign in, with the handed-over key in memory.
#[test]
fn device_request_installs_only_a_payload_that_decrypts() {
    let garbage = rig("device-garbage");
    let socket = ws_server();
    let running = start(&garbage, &socket);
    let _init = issue_code(&socket, &running, 60);
    let plaintext = json!({
        "session_token": garbage.account.session_token,
        "master_key_b64": B64.encode(garbage.account.master_key),
    })
    .to_string();
    let not_the_requests = CliEphemeralKey::generate();
    let unsealed = json!({
        "nonce_b64": B64.encode([0u8; 12]),
        "encrypted_payload_b64": B64.encode(plaintext.as_bytes()),
        "browser_ecdh_public_b64": B64.encode(not_the_requests.public_key_bytes()),
    })
    .to_string();
    socket.say.send(Say::Text(unsealed)).unwrap();
    assert_eq!(
        finish(&running),
        Err(DeviceRequestRefused::KeysNotReceived {
            by: ANOTHER_DEVICE.to_string()
        })
    );
    nothing_stored(&garbage);
    assert_eq!(
        asked(&garbage, "/auth/me"),
        0,
        "nothing was asked for an answer that did not open"
    );

    let sealed = rig("device-sealed");
    let socket = ws_server();
    let running = start(&sealed, &socket);
    let init = issue_code(&socket, &running, 60);
    socket
        .say
        .send(Say::Text(sealed_answer(
            &init,
            &sealed.account.session_token,
            &sealed.account.master_key,
        )))
        .unwrap();
    assert_eq!(finish(&running), signed_in());
    assert_eq!(
        stored_token(&sealed).as_deref(),
        Some(sealed.account.session_token.as_str())
    );
    let in_memory = sealed
        .engine
        .acct
        .session
        .lock()
        .unwrap()
        .as_ref()
        .map(|session| session.master_key);
    assert_eq!(
        in_memory,
        Some(sealed.account.master_key),
        "the handed-over key is the one in memory"
    );
    assert_eq!(sealed.engine.gate(), GateState::Ready);
    assert_eq!(
        sealed.engine.user_id().as_deref(),
        Some(sealed.account.user_id.as_str())
    );
    assert_eq!(
        asked(&sealed, "/auth/logout"),
        0,
        "a session that was installed is not ended"
    );
}

/// Rule 1, the turn half (M2: `install_device_session` re-reads the turn at its top, `let turn =
/// self.acct.session_generation();`, or drops the `session_unchanged_since(turn)` check). The session moved while the
/// other device was asked: the payload opens, and nothing is stored; the session the server made is ended.
#[test]
fn device_request_refuses_after_the_session_moved() {
    let rig = rig("device-moved");
    let socket = ws_server();
    let running = start(&rig, &socket);
    let init = issue_code(&socket, &running, 60);
    {
        // Something else changed the session meanwhile (a sign-in, a lock, a sign-out moves the turn).
        let _writes = rig.engine.writes.write().unwrap();
        rig.engine
            .acct
            .advance_session_generation_holding_session_write_lock(SessionTransition::Write);
    }
    socket
        .say
        .send(Say::Text(sealed_answer(
            &init,
            &rig.account.session_token,
            &rig.account.master_key,
        )))
        .unwrap();
    assert_eq!(finish(&running), Err(DeviceRequestRefused::Overtaken));
    nothing_stored(&rig);
    assert_eq!(asked(&rig, "/auth/me"), 0, "the arriving account was not even read");
    assert_eq!(
        asked(&rig, "/auth/logout"),
        1,
        "the session made for this request was ended at the server"
    );
}

/// Rule 2 (M3: the `stopped(stop)` arm of the wait's `select!` is dropped). A stop while the code is shown ends the wait
/// at once (well under a tick), with nothing stored and nothing asked of the server.
#[test]
fn device_request_stop_ends_the_wait() {
    let rig = rig("device-stop");
    let socket = ws_server();
    let running = start(&rig, &socket);
    let _init = issue_code(&socket, &running, 120);
    let asked_at = Instant::now();
    running.stop.store(true, Ordering::SeqCst);
    let ended = running
        .done
        .recv_timeout(Duration::from_secs(5))
        .expect("the stop ended the wait within 5 s");
    let took = asked_at.elapsed();
    assert_eq!(ended, Err(DeviceRequestRefused::Stopped));
    assert!(
        took < Duration::from_millis(900),
        "the stop is raced against the wait, not read at a tick: {took:?}"
    );
    nothing_stored(&rig);
    assert_eq!(asked(&rig, "/auth/me"), 0);
}

/// Rule 3 (M4: `arriving_account_fits` answers `Ok(true)` instead of the binding's decision on the local data). This
/// PC's local data is another account's: the arriving session is refused, nothing is installed, the new session is
/// ended, and the local data is as it was.
#[test]
fn device_request_other_accounts_data_is_a_mismatch() {
    let rig = rig("device-other-data");
    let other = Identity::new(Some("an-invented-other-user"), Some("someone-else@example.eu"));
    StateDb::open(&rig.local_state).unwrap().set_owner(&other).unwrap();
    let socket = ws_server();
    let running = start(&rig, &socket);
    let init = issue_code(&socket, &running, 60);
    socket
        .say
        .send(Say::Text(sealed_answer(
            &init,
            &rig.account.session_token,
            &rig.account.master_key,
        )))
        .unwrap();
    assert_eq!(finish(&running), Err(DeviceRequestRefused::AccountMismatch));
    nothing_stored(&rig);
    assert_eq!(
        asked(&rig, "/auth/logout"),
        1,
        "the arriving session was ended at the server"
    );
    let owner = match StateDb::open_read_only(&rig.local_state) {
        Ok(db) => db.owner().unwrap(),
        Err(_) => panic!("the local data is still there"),
    };
    assert_eq!(owner, Some(other), "nothing was reset, adopted or recorded");
}

/// Rule 3, the stored half: a session of another account is stored here (signed in, no key): it stays, and the
/// arriving one is ended.
#[test]
fn device_request_a_stored_session_of_another_account_is_a_mismatch() {
    let rig = rig("device-other-session");
    {
        let vault = rig.engine.vault();
        vault
            .install_session(SessionToken::new("an-invented-other-token").unwrap())
            .unwrap();
        vault.bind_session_owner("an-invented-other-user").unwrap();
    }
    let socket = ws_server();
    let running = start(&rig, &socket);
    let init = issue_code(&socket, &running, 60);
    socket
        .say
        .send(Say::Text(sealed_answer(
            &init,
            &rig.account.session_token,
            &rig.account.master_key,
        )))
        .unwrap();
    assert_eq!(finish(&running), Err(DeviceRequestRefused::AccountMismatch));
    assert_eq!(stored_token(&rig).as_deref(), Some("an-invented-other-token"));
    assert!(rig.engine.acct.session.lock().unwrap().is_none());
    assert_eq!(asked(&rig, "/auth/logout"), 1);
}

/// The request asked from the twelve words: this same account is signed in here without its key. The arriving session
/// replaces the stored one (which is ended at the server), and the handed-over key opens the vault.
#[test]
fn device_request_on_this_accounts_keyless_session_opens_the_vault() {
    let rig = rig("device-words");
    {
        let vault = rig.engine.vault();
        vault
            .install_session(SessionToken::new("an-earlier-token-of-this-account").unwrap())
            .unwrap();
        vault.bind_session_owner(&rig.account.user_id).unwrap();
        vault.store_account_email(&rig.account.email).unwrap();
    }
    rig.engine.set_auth_present(true);
    rig.engine.remember_session_user_id(Some(rig.account.user_id.clone()));
    let socket = ws_server();
    let running = start(&rig, &socket);
    let init = issue_code(&socket, &running, 60);
    socket
        .say
        .send(Say::Text(sealed_answer(
            &init,
            &rig.account.session_token,
            &rig.account.master_key,
        )))
        .unwrap();
    assert_eq!(finish(&running), signed_in());
    assert_eq!(stored_token(&rig).as_deref(), Some(rig.account.session_token.as_str()));
    assert_eq!(rig.engine.gate(), GateState::Ready);
    assert_eq!(
        asked(&rig, "/auth/logout"),
        1,
        "the replaced session was ended at the server"
    );
}

/// Rule 4: the first frame carries the public key and what this PC can say about itself, exactly those fields, and no
/// code (none exists yet; the server issues it after).
#[test]
fn device_request_init_frame_carries_the_key_and_no_code() {
    let frame = init_frame("cHVia2V5", Some("Invented-PC".to_string()));
    assert_eq!(frame["ecdh_public_key_b64"], "cHVia2V5");
    assert_eq!(frame["client"], "desktop");
    assert!(!client_version().is_empty());
    assert_eq!(frame["client_version"], client_version().as_str());
    assert_eq!(frame["device_name"], "Invented-PC");
    assert_eq!(frame["os"], std::env::consts::OS);

    let rig = rig("device-init");
    let socket = ws_server();
    let running = start(&rig, &socket);
    let init = issue_code(&socket, &running, 60);
    let mut fields: Vec<&str> = init.as_object().unwrap().keys().map(String::as_str).collect();
    fields.sort_unstable();
    let mut expected = vec!["client", "client_version", "ecdh_public_key_b64", "os"];
    if device_request_name().is_some() {
        expected.push("device_name");
        expected.sort_unstable();
    }
    assert_eq!(fields, expected);
    // The key the server takes: 65 bytes, uncompressed, on P-256 (core seals to it).
    let public = B64.decode(init["ecdh_public_key_b64"].as_str().unwrap()).unwrap();
    assert_eq!((public.len(), public[0]), (65, 0x04));
    assert!(encrypt_for_requester(&public, b"x").is_ok());
    let sent = init.to_string();
    assert!(!sent.contains(CODE) && !sent.contains("KQ7M4TXP"), "{sent}");
    running.stop.store(true, Ordering::SeqCst);
    assert_eq!(finish(&running), Err(DeviceRequestRefused::Stopped));
}

/// Rule 4, the address half (task 1734): an address that holds the code, or that is not a web page, is not read; a
/// plain one is, with the code's life bounded.
#[test]
fn device_request_an_address_that_holds_the_code_is_refused() {
    for address in [
        "https://app.example.eu/cli-auth?code=KQ7M-4TXP",
        "https://app.example.eu/cli-auth#kq7m4txp",
        "javascript:alert(1)",
        "ftp://app.example.eu/cli-auth",
    ] {
        let frame = json!({ "user_code": CODE, "verification_uri": address, "expires_in": 300 }).to_string();
        assert_eq!(read_code(&frame).err(), Some(DeviceRequestRefused::Failed), "{address}");
    }
    for refused in [
        r#"{"error":"invalid_public_key"}"#,
        r#"{"verification_uri":"https://app.example.eu/cli-auth"}"#,
        r#"{"user_code":"KQ7M-4TXP"}"#,
        "not json",
    ] {
        assert_eq!(
            read_code(refused).err(),
            Some(DeviceRequestRefused::Failed),
            "{refused}"
        );
    }
    let issued = read_code(&code_frame(300)).expect("a plain address is read");
    assert_eq!(
        (
            issued.code.as_str(),
            issued.verification_uri.as_str(),
            issued.expires_in
        ),
        (CODE, PAGE, 300)
    );
    let unnamed = read_code(&json!({ "user_code": CODE, "verification_uri": PAGE }).to_string()).unwrap();
    assert_eq!(unnamed.expires_in, DEFAULT_EXPIRES_IN);
    let long = read_code(&code_frame(86_400)).unwrap();
    assert_eq!(long.expires_in, MAX_WAIT_SECS);
}

/// A macro call from its name to its closing parenthesis (parentheses inside string literals do not count).
fn macro_call(from: &str) -> &str {
    let open = from.find('(').expect("the call's arguments");
    let (mut depth, mut quoted, mut escaped) = (0usize, false, false);
    for (i, c) in from[open..].char_indices() {
        match c {
            _ if escaped => escaped = false,
            '\\' if quoted => escaped = true,
            '"' => quoted = !quoted,
            '(' if !quoted => depth += 1,
            ')' if !quoted => {
                depth -= 1;
                if depth == 0 {
                    return &from[..open + i + 1];
                }
            }
            _ => {}
        }
    }
    panic!("the call is not closed: {from}")
}

/// Rule 5: no log call of this module, or of the key-storing helper it calls, names the payload, the token, the key or
/// the code (its fixed words aside), an event has no field that could hold one, and an event's `Debug` leaves the code
/// out.
#[test]
fn device_request_secrets_never_reach_a_log_line_or_an_event() {
    let source = include_str!("device_request.rs").replace("\r\n", "\n");
    let production = &source[..source.find("#[cfg(test)]").expect("the tests module")];
    // The handed-over key is stored by the one helper the passkey sign-in shares (`passkeys_api::keep_handed_over_key`):
    // its log calls are this request's too, and are scanned with the same words.
    let shared = include_str!("../passkeys_api.rs").replace("\r\n", "\n");
    let helper = &shared[shared
        .find("fn keep_handed_over_key")
        .expect("the shared key-storing helper")..];
    let helper = &helper[..helper.find("\n    }\n").expect("the helper's end")];
    let mut calls = 0;
    for (at, scanned) in [production, helper]
        .into_iter()
        .flat_map(|scanned| scanned.match_indices("tracing::").map(move |(at, _)| (at, scanned)))
    {
        let call = macro_call(&scanned[at..]);
        // What the call writes besides its fixed words: its fields and arguments, with the literals taken out.
        let outside: String = call
            .split('"')
            .enumerate()
            .filter(|(i, _)| i % 2 == 0)
            .map(|(_, part)| part)
            .collect::<Vec<_>>()
            .concat()
            .to_lowercase();
        for secret in [
            "token",
            "key",
            "payload",
            "plaintext",
            "credentials",
            "code",
            "secret",
            "received",
            "text",
            "frame",
            "init",
        ] {
            assert!(!outside.contains(secret), "a log call names `{secret}`: {call}");
        }
        calls += 1;
    }
    // 3 in this module and 1 in the key-storing helper.
    assert!(calls >= 4, "the scan found the log calls: {calls}");
    let event = &production[production.find("pub enum DeviceRequestEvent").expect("the events")..];
    let event = &event[..event.find("\n}\n").expect("their end")];
    for secret in ["token", "key", "payload", "plaintext"] {
        assert!(!event.contains(secret), "an event field could hold `{secret}`");
    }
    let shown = format!(
        "{:?}",
        DeviceRequestEvent::Code {
            code: CODE.into(),
            verification_uri: PAGE.into(),
            expires_in: 300,
        }
    );
    assert!(!shown.contains(CODE), "{shown}");
}

// ── The endings ──────────────────────────────────────────────────────────────────────────────────────────────────────

/// The server's answers, read as it means them: the owner's no (with the name the server never sends), the code's end,
/// an error it has no word for, and anything else that is not a payload that opens.
#[test]
fn device_request_answers_are_read_as_the_server_means_them() {
    let key = CliEphemeralKey::generate();
    let denied = DeviceRequestRefused::Denied {
        by: ANOTHER_DEVICE.to_string(),
    };
    let not_received = DeviceRequestRefused::KeysNotReceived {
        by: ANOTHER_DEVICE.to_string(),
    };
    assert_eq!(read_answer(&key, r#"{"error":"denied"}"#).err(), Some(denied));
    assert_eq!(
        read_answer(&key, r#"{"error":"timeout"}"#).err(),
        Some(DeviceRequestRefused::Expired)
    );
    assert_eq!(
        read_answer(&key, r#"{"error":"expired"}"#).err(),
        Some(DeviceRequestRefused::Expired)
    );
    assert_eq!(
        read_answer(&key, r#"{"error":"busy"}"#).err(),
        Some(DeviceRequestRefused::Failed)
    );
    for unread in ["not json", "{}", r#"{"nonce_b64":"AAAA"}"#] {
        assert_eq!(read_answer(&key, unread).err(), Some(not_received.clone()), "{unread}");
    }
    // A sealed payload whose key is not 32 bytes, or whose token is empty, is not read either.
    let init = json!({ "ecdh_public_key_b64": B64.encode(key.public_key_bytes()) });
    let public = B64.decode(init["ecdh_public_key_b64"].as_str().unwrap()).unwrap();
    for plaintext in [
        json!({ "session_token": "t", "master_key_b64": B64.encode([1u8; 16]) }),
        json!({ "session_token": " ", "master_key_b64": B64.encode([1u8; 32]) }),
        json!({ "master_key_b64": B64.encode([1u8; 32]) }),
    ] {
        let relay = encrypt_for_requester(&public, plaintext.to_string().as_bytes()).unwrap();
        let frame = json!({
            "nonce_b64": B64.encode(relay.nonce),
            "encrypted_payload_b64": B64.encode(&relay.ciphertext),
            "browser_ecdh_public_b64": B64.encode(&relay.approver_public),
        })
        .to_string();
        assert_eq!(
            read_answer(&key, &frame).err(),
            Some(not_received.clone()),
            "{plaintext}"
        );
    }
    let opened = read_answer(&key, &sealed_answer(&init, "an-invented-token", &[7u8; 32])).expect("a sealed payload");
    assert_eq!(opened.token.as_str(), "an-invented-token");
    assert_eq!(*opened.key, [7u8; 32]);
    assert_eq!(opened.by, ANOTHER_DEVICE);
}

/// DW-A09 on the wire: the owner said no. No session was made, so nothing is ended either.
#[test]
fn device_request_a_denial_ends_the_request_with_nothing_stored() {
    let rig = rig("device-denied");
    let socket = ws_server();
    let running = start(&rig, &socket);
    let _init = issue_code(&socket, &running, 60);
    socket.say.send(Say::Text(r#"{"error":"denied"}"#.to_string())).unwrap();
    assert_eq!(
        finish(&running),
        Err(DeviceRequestRefused::Denied {
            by: ANOTHER_DEVICE.to_string()
        })
    );
    nothing_stored(&rig);
    assert_eq!(asked(&rig, "/auth/logout"), 0);
}

/// The countdown is this PC's: a tick a second from the code's life, and the request ends at zero (DW-A10).
#[test]
fn device_request_counts_down_and_expires() {
    let rig = rig("device-expires");
    let socket = ws_server();
    let running = start(&rig, &socket);
    let _init = issue_code(&socket, &running, 2);
    let mut ticks = Vec::new();
    while let Ok(event) = running.events.recv_timeout(WAIT) {
        match event {
            DeviceRequestEvent::Tick { seconds_left } => ticks.push(seconds_left),
            other => panic!("only ticks after the code, got {other:?}"),
        }
        if ticks.last() == Some(&0) {
            break;
        }
    }
    assert_eq!(ticks, vec![1, 0]);
    assert_eq!(finish(&running), Err(DeviceRequestRefused::Expired));
    nothing_stored(&rig);
}

/// A socket that drops while the code is shown is said (`Connection(false)`), and the request ends: a new socket would
/// be a new code.
#[test]
fn device_request_a_dropped_socket_is_said_and_ends_the_request() {
    let rig = rig("device-dropped");
    let socket = ws_server();
    let running = start(&rig, &socket);
    let _init = issue_code(&socket, &running, 60);
    socket.say.send(Say::Close).unwrap();
    let mut said = false;
    while let Ok(event) = running.events.recv_timeout(WAIT) {
        match event {
            DeviceRequestEvent::Connection(false) => {
                said = true;
                break;
            }
            DeviceRequestEvent::Tick { .. } => {}
            other => panic!("ticks, then the drop: got {other:?}"),
        }
    }
    assert!(said, "the drop was said");
    assert_eq!(finish(&running), Err(DeviceRequestRefused::ConnectionLost));
    nothing_stored(&rig);
}

/// A press whose session moved on before the request began starts nothing: no socket, no key.
#[test]
fn device_request_a_press_of_an_earlier_turn_starts_nothing() {
    let rig = rig("device-late-press");
    let turn = rig.engine.session_turn();
    {
        let _writes = rig.engine.writes.write().unwrap();
        rig.engine
            .acct
            .advance_session_generation_holding_session_write_lock(SessionTransition::Write);
    }
    let stop = AtomicBool::new(false);
    let result = rig
        .engine
        .device_request_at("ws://127.0.0.1:9/never", turn, &|_| panic!("no event"), &stop);
    assert_eq!(result, Err(DeviceRequestRefused::Overtaken));
}

// ── Ported from the Tauri shell (`browser_login.rs` tests) ───────────────────────────────────────────────────────────

#[test]
fn device_request_ws_url_follows_the_api_base() {
    assert_eq!(ws_url("https://api.beebeeb.io"), "wss://api.beebeeb.io/api/v1/auth/cli");
    assert_eq!(
        ws_url("https://api.beebeeb.io/"),
        "wss://api.beebeeb.io/api/v1/auth/cli"
    );
    assert_eq!(ws_url("http://127.0.0.1:3001"), "ws://127.0.0.1:3001/api/v1/auth/cli");
}

/// Task 1436: the upgrade request carries the writer-provenance headers every other request of this app sends.
#[test]
fn device_request_ws_upgrade_request_carries_provenance_headers() {
    let request = build_ws_request("wss://api.beebeeb.io/api/v1/auth/cli").expect("a wss address builds");
    assert_eq!(
        request.headers().get("x-beebeeb-client").map(|v| v.to_str().unwrap()),
        Some("desktop")
    );
    assert_eq!(
        request
            .headers()
            .get("x-beebeeb-client-version")
            .map(|v| v.to_str().unwrap().to_string()),
        Some(client_version())
    );
}

/// Task 1734: what this PC tells the server about itself is a hint the approval page labels as such.
#[test]
fn device_request_omits_a_device_name_when_none() {
    let frame = init_frame("cHVia2V5", None);
    assert!(frame.get("device_name").is_none(), "{frame}");
    let blank = init_frame("cHVia2V5", Some("  \t ".to_string()));
    assert!(blank.get("device_name").is_none(), "{blank}");
}

type WsItem = Result<Message, tokio_tungstenite::tungstenite::Error>;

/// A server that accepted the socket and never sends the code.
struct StalledStream;

impl futures_util::Stream for StalledStream {
    type Item = WsItem;
    fn poll_next(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Poll::Pending
    }
}

/// A server that sends its frames (ping and pong first, as a real one may), then nothing.
struct HealthyStream {
    frames: Vec<Message>,
}

impl futures_util::Stream for HealthyStream {
    type Item = WsItem;
    fn poll_next(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.frames.is_empty() {
            Poll::Ready(None)
        } else {
            Poll::Ready(Some(Ok(self.frames.remove(0))))
        }
    }
}

/// Task 0983: the first round trip is bounded by its caller's timeout, not only the connect.
#[tokio::test]
async fn device_request_recv_times_out_when_server_stalls() {
    let mut ws = StalledStream;
    let outcome = tokio::time::timeout(Duration::from_millis(50), recv_text(&mut ws)).await;
    assert!(outcome.is_err(), "a stalled socket is bounded by the timeout");
}

#[tokio::test]
async fn device_request_recv_returns_promptly_on_healthy_connection() {
    let mut ws = HealthyStream {
        frames: vec![
            Message::Ping(Vec::new()),
            Message::Pong(Vec::new()),
            Message::Text(code_frame(300)),
        ],
    };
    let text = tokio::time::timeout(DEVICE_CODE_TIMEOUT, recv_text(&mut ws))
        .await
        .expect("no timeout on a healthy socket")
        .expect("the text frame arrives");
    assert!(text.contains("user_code"));
}

#[tokio::test]
async fn device_request_errors_immediately_on_closed_connection() {
    let mut ws = HealthyStream { frames: vec![] };
    let result = recv_text(&mut ws).await;
    assert!(result.unwrap_err().contains("Connection closed"));
}

// ── LD-0139: the handshake is core's `beebeeb-cli-auth-v1`, pinned to core's published vectors ───────────────────────
// (`beebeeb-core/tests/cli_auth_vectors.rs`). Each check drives this module's own functions (`ephemeral_public_b64`,
// `init_frame`, `decrypt_payload`), as the Tauri shell's `core_cli_auth_vectors` did.

const CLI_PRIV_HEX: &str = "1111111111111111111111111111111111111111111111111111111111111111";
/// HKDF-SHA256(ikm = ECDH x, salt = None, info = "beebeeb-cli-auth-v1"): the v1 AES key.
const EXPECTED_AES_KEY_HEX: &str = "8bd295090ce8a6412b831704b601b55d60ba3cae056f5e1337a11b5f9eceac35";
/// The raw ECDH x-coordinate (the old v0.4 web app's AES key, refused since core 0.3.0).
const EXPECTED_SHARED_HEX: &str = "ccfc261f58193c98ca4ad4a53bbac6f0ee29bc4d48438090446908622ca79af6";
const EXPECTED_CLI_PUB_HEX: &str = concat!(
    "040217e617f0b6443928278f96999e69a23a4f2c152bdf6d6cdf66e5b80282d4ed",
    "194a7debcb97712d2dda3ca85aa8765a56f45fc758599652f2897c65306e5794"
);
const EXPECTED_BROWSER_PUB_HEX: &str = concat!(
    "04d65a93977caa3d1b081852ff57a79e465f1660577304baead505dd3a48589cf3",
    "50185e895372df6221ea3a137557e473fddb6755f05bd507c3c533fce9c91285"
);
const BROWSER_PUB_COMPRESSED_HEX: &str = "03d65a93977caa3d1b081852ff57a79e465f1660577304baead505dd3a48589cf3";
const OFF_CURVE_COMPRESSED_HEX: &str = "020000000000000000000000000000000000000000000000000000000000000001";

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

fn vector_key() -> CliEphemeralKey {
    let scalar: [u8; 32] = unhex(CLI_PRIV_HEX).try_into().unwrap();
    CliEphemeralKey::from_secret_bytes(&scalar).unwrap()
}

/// What the approving side sends under `key_hex`: AES-256-GCM, everything base64 as on the wire.
fn vector_frame(key_hex: &str, nonce: [u8; 12], plaintext: &[u8]) -> (String, String) {
    use aes_gcm::aead::Aead;
    use aes_gcm::aead::generic_array::GenericArray;
    use aes_gcm::{Aes256Gcm, KeyInit};
    let cipher = Aes256Gcm::new(GenericArray::from_slice(&unhex(key_hex)));
    let ciphertext = cipher.encrypt(GenericArray::from_slice(&nonce), plaintext).unwrap();
    (B64.encode(nonce), B64.encode(ciphertext))
}

#[test]
fn device_request_wire_matches_core_vectors() {
    // The init frame's key is the pinned uncompressed point.
    let sent = ephemeral_public_b64(&vector_key());
    assert_eq!(B64.decode(&sent).unwrap(), unhex(EXPECTED_CLI_PUB_HEX));
    assert_eq!(init_frame(&sent, None)["ecdh_public_key_b64"], sent.as_str());
    // A v1 payload under the pinned HKDF key opens.
    let plaintext = br#"{"session_token":"s","master_key_b64":"AAAA","email":"a@example.eu"}"#;
    let (nonce, ciphertext) = vector_frame(EXPECTED_AES_KEY_HEX, [0xAB; 12], plaintext);
    let browser_pub = B64.encode(unhex(EXPECTED_BROWSER_PUB_HEX));
    assert_eq!(
        decrypt_payload(&vector_key(), &browser_pub, &nonce, &ciphertext).unwrap(),
        plaintext
    );
    // A compressed approving key reaches the same secret.
    let (nonce, ciphertext) = vector_frame(EXPECTED_AES_KEY_HEX, [0x01; 12], b"compressed");
    let compressed = B64.encode(unhex(BROWSER_PUB_COMPRESSED_HEX));
    assert_eq!(
        decrypt_payload(&vector_key(), &compressed, &nonce, &ciphertext).unwrap(),
        b"compressed"
    );
}

/// Core 0.3.0: only the HKDF key opens a payload; one sealed under the raw shared secret is refused, and a bad frame is
/// an error, never a panic.
#[test]
fn device_request_wire_refuses_a_legacy_or_bad_payload() {
    let key = vector_key();
    let browser_pub = B64.encode(unhex(EXPECTED_BROWSER_PUB_HEX));
    let (nonce, ciphertext) = vector_frame(EXPECTED_SHARED_HEX, [0xCD; 12], b"legacy-v0.4");
    assert!(decrypt_payload(&key, &browser_pub, &nonce, &ciphertext).is_err());
    let (nonce, ciphertext) = vector_frame(EXPECTED_AES_KEY_HEX, [0xCD; 12], b"legacy-v0.4");
    assert_eq!(
        decrypt_payload(&key, &browser_pub, &nonce, &ciphertext).unwrap(),
        b"legacy-v0.4",
        "the control: the same frame under the HKDF key opens"
    );
    let (nonce, ciphertext) = vector_frame(EXPECTED_AES_KEY_HEX, [0x02; 12], b"x");
    let off_curve = B64.encode(unhex(OFF_CURVE_COMPRESSED_HEX));
    assert_eq!(
        decrypt_payload(&key, &off_curve, &nonce, &ciphertext).unwrap_err(),
        "Invalid browser public key (not on curve)."
    );
    let short_nonce = B64.encode([0u8; 11]);
    assert_eq!(
        decrypt_payload(&key, &browser_pub, &short_nonce, &ciphertext).unwrap_err(),
        "Invalid nonce: 11 bytes, expected 12."
    );
    let (_, other_ciphertext) = vector_frame(&"00".repeat(32), [0x02; 12], b"x");
    assert_eq!(
        decrypt_payload(&key, &browser_pub, &nonce, &other_ciphertext).unwrap_err(),
        "Decryption failed — key mismatch or corrupted credentials."
    );
}
