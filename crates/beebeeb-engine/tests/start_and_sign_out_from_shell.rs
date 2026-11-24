//! The start path, the owner record, the sign-out and the keys, as the Tauri shell's tests pinned them, through the
//! engine a native host drives (census CP2 §T, slice `t-session-start`). Each test here ports one rule of a shell test
//! that has no engine or native pin yet; the shell test it replaces is named in its doc, with the mutant (a one-line
//! change in the engine) that turns it red. Rules the engine already pins are not repeated here.
//!
//! Everything is invented: the accounts, their passwords and their 12 words (`test_api::fake_account`), the folders.
//! The fake API is HTTP on loopback; nothing reaches the network or the OS credential store.
//!
//! Each test runs in a process of its own (`in_own_process`): the state folder is a process-wide global
//! (`state_paths`), set once per process, and `tests/common` makes a new one per setup.

mod common;

/// Runs `scenario` in a child process of this test binary (the state folder is set once per process).
#[cfg(not(target_os = "macos"))]
fn in_own_process(name: &str, scenario: impl FnOnce()) {
    /// Set in the child: the scenario it runs.
    const SCENARIO: &str = "BB_START_AND_SIGN_OUT_SCENARIO";
    if std::env::var(SCENARIO).ok().as_deref() == Some(name) {
        scenario();
        return;
    }
    let out = std::process::Command::new(std::env::current_exe().expect("this test binary"))
        .args([name, "--exact", "--nocapture", "--test-threads=1"])
        .env(SCENARIO, name)
        .output()
        .expect("the scenario's own process");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.contains("test result: ok. 1 passed;"),
        "{name}, in its own process ({}):\n--- stdout\n{stdout}\n--- stderr\n{}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
}

// ── The owner record in the credential store (`session_owner.rs`) ────────────────────────────────────────────────────

/// `session_owner.rs`: `bind_session_owner_in_keychain` (the token's owner is the id the server named for that
/// session, written in the sign-in's turn) and `bind_vault_key_to_session_owner_in_keychain` (the key the 12 words
/// proved takes the stored session's owner). The engine's rules are `key_owner::bind_*` (unit-tested in
/// `signin/key_owner_tests.rs`); this pins that the engine's own sign-in and unlock call them.
/// Mutant: drop `key_owner::bind_key_to_session_owner(&vault)` in `Engine::unlock_with_phrase_step` (`app.rs`): the key
/// stays untagged and the second assertion is red.
#[cfg(not(target_os = "macos"))]
#[test]
fn a_password_sign_in_and_the_phrase_tag_the_session_and_the_key_with_the_servers_id() {
    in_own_process(
        "a_password_sign_in_and_the_phrase_tag_the_session_and_the_key_with_the_servers_id",
        || {
            use beebeeb_engine::keychain::AuthSecretStore;

            let s = common::setup();
            let store = s.stores.store_for(&s.account_id);
            s.sign_in();
            assert_eq!(
                store.load_session_owner().unwrap().as_deref(),
                Some(s.account.user_id.as_str()),
                "the session's owner is the id the server named"
            );
            assert_eq!(
                store.load_vault_key_owner().unwrap(),
                None,
                "no key yet, so no key owner"
            );
            s.unlock();
            assert_eq!(
                store.load_vault_key_owner().unwrap().as_deref(),
                Some(s.account.user_id.as_str()),
                "the key the 12 words proved takes the session's owner"
            );
            assert_eq!(s.entries(), common::ENTRIES.to_vec(), "all five entries, nothing else");
        },
    );
}

// ── Sign in with another device (`browser_login.rs`, folded into the device request by D-W3-3) ──────────────────────

/// `browser_login.rs:an_unconfirmed_browser_sign_in_stays_a_retryable_error`: a sign-in whose account could not be
/// confirmed ends the new session at the server and is an error the person can try again, never a switch. The device
/// request's rule (`app/bindings/device_request.rs`, `install_device_session`): the arriving session's account is read
/// from the server first; when that read fails the session is ended (`end_unstored`) and the request ends with
/// `NotStored` (not `AccountMismatch`), nothing stored. A second request, with the server answering, signs in: the
/// first left nothing that stands in the way.
/// Mutants: drop `self.end_unstored(&token).await` in the `fetch_profile` error arm (no logout: red); answer that arm
/// with `AccountMismatch` (the refusal: red).
#[cfg(not(target_os = "macos"))]
#[test]
fn a_device_sign_in_whose_account_cannot_be_read_is_ended_and_can_be_tried_again() {
    in_own_process(
        "a_device_sign_in_whose_account_cannot_be_read_is_ended_and_can_be_tried_again",
        || {
            use std::sync::atomic::Ordering;

            use beebeeb_engine::app::bindings::device_request::{ANOTHER_DEVICE, DeviceRequestRefused, DeviceSignedIn};
            use beebeeb_engine::signin::gate::GateState;

            let pc = device::Pc::open();
            pc.profile_refused.store(true, Ordering::SeqCst);

            let refused = pc.request_answered_with_the_accounts_keys();
            assert_eq!(
                refused,
                Err(DeviceRequestRefused::NotStored),
                "a retryable refusal, not a switch"
            );
            assert!(pc.stores.is_empty(), "nothing was stored");
            assert_eq!(pc.engine.gate(), GateState::SignedOut);
            assert_eq!(pc.asked("GET /api/v1/auth/me"), 1, "the account was asked for once");
            assert_eq!(
                pc.logouts_of(&pc.account.session_token),
                1,
                "the new session was ended at the server"
            );

            pc.profile_refused.store(false, Ordering::SeqCst);
            assert_eq!(
                pc.request_answered_with_the_accounts_keys(),
                Ok(DeviceSignedIn {
                    by: ANOTHER_DEVICE.to_string(),
                    vault_unlocked: true
                }),
                "tried again, it signs in"
            );
            assert_eq!(pc.engine.gate(), GateState::Ready);
            assert_eq!(
                pc.logouts_of(&pc.account.session_token),
                1,
                "the installed session is not ended"
            );
        },
    );
}

/// One PC whose API base is a loopback front: the device request's socket (`/api/v1/auth/cli`) is the front's own,
/// scripted by the test; every other request goes through to the fake account's HTTP API unchanged.
#[cfg(not(target_os = "macos"))]
mod device {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, mpsc};
    use std::time::Duration;

    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD as B64;
    use beebeeb_engine::app::bindings::device_request::{DeviceRequestEvent, DeviceRequestRefused, DeviceSignedIn};
    use beebeeb_engine::app::{Engine, EngineConfig};
    use beebeeb_engine::config::DesktopConfig;
    use beebeeb_engine::host::ChannelHost;
    use beebeeb_engine::identity::TEST;
    use beebeeb_engine::signin::memory_store::MemoryCredentials;
    use beebeeb_engine::state_paths;
    use beebeeb_engine::test_api::fake_account::{FakeAccount, account_handler, test_account};
    use beebeeb_engine::test_api::{FakeApi, Handler, Reply, Request};
    use futures_util::{SinkExt, StreamExt};
    use serde_json::json;
    use tokio::io::AsyncWriteExt;
    use tokio_tungstenite::tungstenite::Message;

    /// The socket's path (core's `auth_flow::device::DEVICE_SOCKET_PATH`).
    const SOCKET: &str = "/api/v1/auth/cli";
    /// The longest a step waits for the other side.
    const WAIT: Duration = Duration::from_secs(20);

    pub struct Pc {
        pub account: Arc<FakeAccount>,
        pub stores: MemoryCredentials,
        pub engine: Arc<Engine<MemoryCredentials>>,
        /// While set, the fake answers the account read (`GET /api/v1/auth/me`) with a 503.
        pub profile_refused: Arc<AtomicBool>,
        fake: FakeApi,
        sockets: mpsc::Receiver<Socket>,
        _front: tokio::runtime::Runtime,
    }

    /// One device request's socket, as the front accepted it.
    struct Socket {
        init: String,
        say: tokio::sync::mpsc::UnboundedSender<String>,
    }

    impl Pc {
        pub fn open() -> Pc {
            let (account, _phrase, first) = test_account();
            drop(first);
            let profile_refused = Arc::new(AtomicBool::new(false));
            let handler: Handler = {
                let (inner, refused) = (account_handler(account.clone()), profile_refused.clone());
                Arc::new(move |request: &Request| {
                    if refused.load(Ordering::SeqCst) && request.route() == "GET /api/v1/auth/me" {
                        return Reply::error(503, "unavailable");
                    }
                    inner(request)
                })
            };
            let fake = FakeApi::start(handler);
            let (front, base, sockets) = front_of(&fake.base);
            let home = tempfile::tempdir().expect("a temp folder").keep();
            state_paths::init_from_app_local_data_dir(&TEST, &home.join("local")).expect("the state folder");
            let config = EngineConfig {
                account_id: DesktopConfig::ensure_account_id_for(&TEST).expect("the sandboxed settings file"),
                state_dir: state_paths::beebeeb_state_dir_for(&TEST).expect("the state folder"),
                api_base: base,
            };
            let stores = MemoryCredentials::default();
            let (host, _messages) = ChannelHost::new(|| {});
            let engine = Arc::new(Engine::open(&TEST, config, host, stores.clone()).expect("the engine opens"));
            engine.restore();
            Pc {
                account,
                stores,
                engine,
                profile_refused,
                fake,
                sockets,
                _front: front,
            }
        }

        /// A device request from this PC, which the other device answers with the account's own session and key,
        /// sealed to the request's key as core seals them.
        pub fn request_answered_with_the_accounts_keys(&self) -> Result<DeviceSignedIn, DeviceRequestRefused> {
            let (events_tx, events) = mpsc::channel();
            let (done_tx, done) = mpsc::channel();
            let turn = self.engine.session_turn();
            let engine = self.engine.clone();
            std::thread::spawn(move || {
                let tell = move |event: DeviceRequestEvent| {
                    let _ = events_tx.send(event);
                };
                let _ = done_tx.send(engine.device_request_for(turn, tell, Arc::new(AtomicBool::new(false))));
            });
            let socket = self.sockets.recv_timeout(WAIT).expect("the request opened its socket");
            socket
                .say
                .send(json!({ "user_code": "KQ7M-4TXP", "verification_uri": "https://app.example.eu/cli-auth", "expires_in": 60 }).to_string())
                .unwrap();
            match events.recv_timeout(WAIT).expect("the code reached the host") {
                DeviceRequestEvent::Code { .. } => {}
                other => panic!("the code comes first, got {other:?}"),
            }
            let init: serde_json::Value = serde_json::from_str(&socket.init).expect("the first frame is json");
            let public = B64
                .decode(init["ecdh_public_key_b64"].as_str().expect("the request's key"))
                .unwrap();
            let plaintext = json!({
                "session_token": self.account.session_token,
                "master_key_b64": B64.encode(self.account.master_key),
            })
            .to_string();
            let relay = beebeeb_core::cli_auth::encrypt_for_requester(&public, plaintext.as_bytes()).unwrap();
            socket
                .say
                .send(
                    json!({
                        "nonce_b64": B64.encode(relay.nonce),
                        "encrypted_payload_b64": B64.encode(&relay.ciphertext),
                        "browser_ecdh_public_b64": B64.encode(&relay.approver_public),
                    })
                    .to_string(),
                )
                .unwrap();
            done.recv_timeout(WAIT).expect("the request ended")
        }

        /// How often the fake answered `route` (`"GET /api/v1/x"`).
        pub fn asked(&self, route: &str) -> usize {
            self.fake.requests().iter().filter(|r| r.route() == route).count()
        }

        /// How often the session `token` was ended at the server.
        pub fn logouts_of(&self, token: &str) -> usize {
            self.fake
                .requests()
                .iter()
                .filter(|r| r.route() == "POST /api/v1/auth/logout" && r.bearer() == Some(token))
                .count()
        }
    }

    /// The front on loopback: its base, and each device socket it accepts (its first frame read, then what the test
    /// says). A connection that opens with anything else is piped to the fake at `fake_base` as it is.
    fn front_of(fake_base: &str) -> (tokio::runtime::Runtime, String, mpsc::Receiver<Socket>) {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        let listener = runtime.block_on(tokio::net::TcpListener::bind("127.0.0.1:0")).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let fake = fake_base.trim_start_matches("http://").to_string();
        let (sockets_tx, sockets) = mpsc::channel();
        runtime.spawn(async move {
            while let Ok((inbound, _)) = listener.accept().await {
                let (fake, sockets_tx) = (fake.clone(), sockets_tx.clone());
                tokio::spawn(async move {
                    // The request line decides: the device socket is ours, the rest is the fake's.
                    let mut head = [0u8; 64];
                    let mut seen = 0;
                    for _ in 0..200 {
                        seen = inbound.peek(&mut head).await.unwrap_or(0);
                        if seen >= 4 + SOCKET.len() || head[..seen].contains(&b'\n') {
                            break;
                        }
                        tokio::time::sleep(Duration::from_millis(5)).await;
                    }
                    if head[..seen].starts_with(format!("GET {SOCKET}").as_bytes()) {
                        accept_socket(inbound, sockets_tx).await;
                    } else if let Ok(mut outbound) = tokio::net::TcpStream::connect(&fake).await {
                        let mut inbound = inbound;
                        let _ = tokio::io::copy_bidirectional(&mut inbound, &mut outbound).await;
                        let _ = outbound.shutdown().await;
                    }
                });
            }
        });
        (runtime, base, sockets)
    }

    async fn accept_socket(stream: tokio::net::TcpStream, sockets: mpsc::Sender<Socket>) {
        let Ok(mut ws) = tokio_tungstenite::accept_async(stream).await else {
            return;
        };
        let mut init = None;
        while let Some(Ok(message)) = ws.next().await {
            if let Message::Text(text) = message {
                init = Some(text.to_string());
                break;
            }
        }
        let Some(init) = init else { return };
        let (say, mut said) = tokio::sync::mpsc::unbounded_channel::<String>();
        if sockets.send(Socket { init, say }).is_err() {
            return;
        }
        while let Some(text) = said.recv().await {
            if ws.send(Message::Text(text)).await.is_err() {
                return;
            }
        }
    }
}

// ── Who a session is (`account_binding_tests/part_2.rs`, `part_3.rs`) ──────────────────────────────────────────────

/// `part_2.rs:the_session_identity_takes_the_profile_only_when_it_is_this_sessions`, the same rows: a session is named by
/// its OWN email, and the cached profile's user id counts only when that profile's email is exactly the session's.
/// Mutants (`signin/start.rs`, `identity_of_session`): drop the profile filter (`let profile = profile;`): the rows
/// "another email" and "no session email" are red; compare with `same_email`: the case-variant row is red.
#[test]
fn the_session_identity_takes_the_profile_only_when_it_is_this_sessions() {
    use beebeeb_engine::account_binding::Identity;
    use beebeeb_engine::signin::start::identity_of_session;

    let profile = |email: &str| profile_of("u-a", email);
    assert_eq!(
        identity_of_session(Some("a@example.eu"), Some(&profile("a@example.eu"))),
        Identity::new(Some("u-a"), Some("a@example.eu"))
    );
    assert_eq!(
        identity_of_session(Some("a@example.eu"), Some(&profile("A@example.eu"))),
        Identity::new(None, Some("a@example.eu")),
        "a profile whose email differs only in case is not folded into the session"
    );
    assert_eq!(
        identity_of_session(Some("a@example.eu"), Some(&profile("someone-else@example.eu"))),
        Identity::new(None, Some("a@example.eu")),
        "a profile of another email is ignored"
    );
    assert_eq!(
        identity_of_session(Some("a@example.eu"), None),
        Identity::new(None, Some("a@example.eu")),
        "offline: the email alone"
    );
    assert_eq!(
        identity_of_session(None, Some(&profile("a@example.eu"))),
        Identity::default(),
        "no session email: a cached profile is nobody's to borrow"
    );
    assert_eq!(
        identity_of_session(Some("  "), Some(&profile("a@example.eu"))),
        Identity::default(),
        "an empty email is no email"
    );
}

/// `part_3.rs:an_engine_start_error_that_is_not_a_binding_refusal_records_nothing`, its table half: only the exact two
/// binding sentences are binding refusals (the engine records them for the window, `app.rs` `of_binding`); any other
/// error, a sentence with more after it, or nothing, is none. (Its other half, a start into an occupied slot, is
/// `Ok(AlreadyRunning)` in the engine: `tests/host_contract.rs:a_host_restores_signs_in_unlocks_syncs_lists_and_restarts`.)
/// Mutant: `Refusal::of` compares with `error.starts_with(refusal.sentence())`: the "(detail)" row is red.
#[test]
fn only_the_two_exact_binding_sentences_are_binding_refusals() {
    use beebeeb_engine::account_binding::{IDENTITY_UNKNOWN, OTHER_ACCOUNT_ON_WINDOWS, Refusal};

    assert_eq!(Refusal::of(IDENTITY_UNKNOWN), Some(Refusal::IdentityUnknown));
    assert_eq!(
        Refusal::of(OTHER_ACCOUNT_ON_WINDOWS),
        Some(Refusal::OtherAccountOnWindows)
    );
    assert_eq!(Refusal::of("read the local data owner: disk I/O error"), None);
    assert_eq!(
        Refusal::of(&format!("{IDENTITY_UNKNOWN} (detail)")),
        None,
        "an exact sentence only"
    );
    assert_eq!(Refusal::of(""), None);
}

/// A profile as the server's `/api/v1/auth/me` sends it.
fn profile_of(user_id: &str, email: &str) -> beebeeb_engine::account_dto::AccountProfile {
    serde_json::from_value(serde_json::json!({
        "user_id": user_id,
        "email": email,
        "email_verified": true,
        "created_at": "2026-01-01T00:00:00Z",
    }))
    .expect("a profile")
}

/// `part_2.rs:a_start_pairs_the_keys_with_the_identity_of_the_same_session`: the start names the session by the
/// session's own email, never by the account mirrors another sign-in left (`auth_email`, the cached profile). Here the
/// local data is owner A's; B signs in, and A's profile and email are left in the mirrors. B's start is refused
/// (`OtherAccountsData`), and A's owner record and row stay. (The Windows refusal itself is pinned by
/// `tests/a_refused_start_makes_no_request.rs`; the mirrors were not.)
/// Mutant: drop the profile filter in `identity_of_session` (`signin/start.rs`): the identity takes A's id with B's
/// email, the ids match A's, and B binds into A's data: red.
#[cfg(not(target_os = "macos"))]
#[test]
fn a_start_pairs_the_keys_with_the_identity_of_the_same_session() {
    in_own_process("a_start_pairs_the_keys_with_the_identity_of_the_same_session", || {
        use beebeeb_engine::account_binding::Identity;
        use beebeeb_engine::app::SyncStartRefused;
        use beebeeb_engine::state_db::StateDb;

        let s = common::setup();
        let owner_a = Identity::new(Some("a0000000-0000-4000-8000-00000000000a"), Some("owner@example.eu"));
        s.state_db_with_one_row(&owner_a);
        let folder = s.home.join("Beebeeb Test");
        std::fs::create_dir_all(&folder).unwrap();
        s.sign_in();
        s.unlock();
        let acct = s.engine.accounts().active_account().expect("the account");
        *acct.cached_profile.lock().unwrap() = Some(profile_of(
            owner_a.user_id.as_deref().unwrap(),
            owner_a.email.as_deref().unwrap(),
        ));
        *acct.auth_email.lock().unwrap() = owner_a.email.clone();

        assert_eq!(
            s.engine.keys_for_engine_start(),
            Ok(Identity::new(None, Some(&s.account.email))),
            "the session's own email, no id from A's profile"
        );
        assert_eq!(
            s.engine.start_sync_with(folder.clone(), &common::Port::free()),
            Err(SyncStartRefused::OtherAccountsData)
        );
        assert!(!s.engine.is_syncing(), "no engine for B");
        let db = StateDb::open(s.state_db()).unwrap();
        assert_eq!(db.owner().unwrap(), Some(owner_a), "A still owns the data");
        assert_eq!(db.list_files().unwrap().len(), 1, "A's row stays");
    });
}

// ── The engine stop that did not confirm (`part_2.rs`, `signout_teardown_tests.rs`) ──────────────────────────────────

/// `part_2.rs:a_start_refuses_while_an_earlier_stop_is_unconfirmed_before_it_binds`: while an earlier engine stop is
/// unconfirmed (its task may still run with the keys), a start refuses before it binds or opens anything: no
/// `state.db`, no registration, no engine, and the refusal is recorded for the window.
/// Mutant: delete `engine_start_refusal(acct).map_err(EngineStartBlocked::Refused)?;` in `keys_for_engine_start`
/// (`signin/start.rs`): the start goes on and makes `state.db`: red.
#[cfg(not(target_os = "macos"))]
#[test]
fn a_start_refuses_while_an_earlier_stop_is_unconfirmed_before_it_binds() {
    in_own_process(
        "a_start_refuses_while_an_earlier_stop_is_unconfirmed_before_it_binds",
        || {
            use std::sync::atomic::Ordering;

            use beebeeb_engine::account_binding::Refusal;
            use beebeeb_engine::app::SyncStartRefused;

            let s = common::setup();
            s.sign_in();
            s.unlock();
            let acct = s.engine.accounts().active_account().expect("the account");
            acct.engine_stop_unconfirmed.store(true, Ordering::SeqCst);
            let port = common::Port::free();
            assert_eq!(
                s.engine.start_sync_with(s.home.join("Beebeeb Test"), &port),
                Err(SyncStartRefused::StopUnconfirmed)
            );
            assert!(!s.state_db().exists(), "no state.db was made");
            assert_eq!(port.registers(), 0, "nothing was registered");
            assert!(!s.engine.is_syncing(), "no engine");
            assert_eq!(
                *acct.engine_refusal.lock().unwrap(),
                Some(Refusal::EngineStopUnconfirmed),
                "the window is told to restart"
            );
        },
    );
}

/// `signout_teardown_tests.rs:clear_session_retry_after_unconfirmed_engine_stop_cannot_skip_the_stop_gate`, its first
/// half: a sign-out whose engine stop is not confirmed is refused, and the session stays (all five entries, no logout).
/// The engine's task here never stops in time (a blocking task the abort cannot end). This test pins the first attempt.
/// Mutant: answer `SyncStop::Unconfirmed` like `WasNotRunning` in `sign_out_holding_step` (`app.rs`): the sign-out
/// clears the store: red.
#[cfg(not(target_os = "macos"))]
#[test]
fn a_sign_out_whose_engine_stop_is_unconfirmed_is_refused_and_keeps_the_session() {
    in_own_process(
        "a_sign_out_whose_engine_stop_is_unconfirmed_is_refused_and_keeps_the_session",
        || {
            use std::sync::atomic::Ordering;
            use std::time::{Duration, Instant};

            use beebeeb_engine::account_binding::ENGINE_STOP_UNCONFIRMED;
            use beebeeb_engine::runner::EngineRunner;

            let s = common::setup();
            s.sign_in();
            s.unlock();
            let acct = s.engine.accounts().active_account().expect("the account");
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .enable_all()
                .build()
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(15);
            let task = runtime.spawn_blocking(move || {
                while Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(50));
                }
            });
            *acct.engine.blocking_lock() = Some(EngineRunner::for_test_with_task(task));
            let from = s.fake.requests().len();

            assert_eq!(
                s.engine.sign_out_keeping_local_data(),
                Err(ENGINE_STOP_UNCONFIRMED.to_string())
            );
            assert!(
                acct.engine_stop_unconfirmed.load(Ordering::SeqCst),
                "the stop is held as unconfirmed"
            );
            assert_eq!(s.entries(), common::ENTRIES.to_vec(), "the session stays stored");
            assert!(
                !s.routes_from(from).iter().any(|route| route.contains("/auth/logout")),
                "the session was not ended at the server: {:?}",
                s.routes_from(from)
            );
            runtime.shutdown_background();
        },
    );
}

// ── The adoption window (`part_2.rs`) ─────────────────────────────────────────────────────────────────────────────────

/// `PRAGMA user_version` of the `state.db` at `path` (1: the upgrade's adoption window is shut), read only.
#[cfg(not(target_os = "macos"))]
fn adoption_mark(path: &std::path::Path) -> i64 {
    let db = rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    db.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
        .unwrap()
}

/// `part_2.rs:a_database_the_engine_start_creates_never_opens_the_adoption_window`: a start on a PC with no `state.db`
/// makes one, and it is shut at once: it has no data from before 0.9.0 to adopt. Unowned data that turns up in it
/// later is never adopted.
/// Mutant: delete `data.db.close_adoption_window()…?;` in the start (`app/own_data_start.rs`): the mark is 0 and the
/// adoption below takes the data: red.
#[cfg(not(target_os = "macos"))]
#[test]
fn a_database_the_engine_start_creates_never_opens_the_adoption_window() {
    in_own_process(
        "a_database_the_engine_start_creates_never_opens_the_adoption_window",
        || {
            use beebeeb_engine::account_binding::Identity;
            use beebeeb_engine::app::SyncStart;
            use beebeeb_engine::config::DesktopConfig;
            use beebeeb_engine::identity::TEST;
            use beebeeb_engine::state_db::StateDb;

            let s = common::setup();
            DesktopConfig::update_for(&TEST, |config| {
                config.pause_sync = true;
                (true, ())
            })
            .expect("paused");
            assert!(!s.state_db().exists(), "the premise: a PC with no state.db");
            s.sign_in();
            s.unlock();
            let folder = s.home.join("Beebeeb Test");
            std::fs::create_dir_all(&folder).unwrap();
            assert_eq!(
                s.engine.start_sync_with(folder, &common::Port::free()),
                Ok(SyncStart::Started)
            );
            let _ = s.engine.stop();
            assert_eq!(
                adoption_mark(&s.state_db()),
                1,
                "the start shut the window of the database it made"
            );

            // The owner record removed and an unowned row added: still nobody is adopted.
            let raw = rusqlite::Connection::open(s.state_db()).unwrap();
            raw.execute(
                "DELETE FROM sync_state WHERE key IN ('owner_user_id', 'owner_email')",
                [],
            )
            .unwrap();
            drop(raw);
            let db = StateDb::open(s.state_db()).unwrap();
            let entry = beebeeb_engine::state_db::FileEntry {
                file_id: "f0000000-0000-4000-8000-000000000002".into(),
                path: "/later.txt".into(),
                status: beebeeb_engine::state_db::FileStatus::CloudOnly,
                size_bytes: 10,
                modified_at: 1_794_000_000,
                content_hash: None,
                remote_updated_at: 1_794_000_000,
                parent_id: None,
                item_kind: beebeeb_engine::state_db::ItemKind::File,
            };
            db.upsert_file_with_contract(&entry, |contract| contract.item_kind = entry.item_kind.clone())
                .unwrap();
            assert_eq!(db.owner().unwrap(), None, "the premise: no owner recorded");
            let other = Identity::new(Some("b0000000-0000-4000-8000-00000000000b"), Some("other@example.eu"));
            assert!(!db.adopt_owner_once(Some(&other)).unwrap(), "nobody is adopted");
            assert_eq!(db.owner().unwrap(), None);
        },
    );
}

/// `part_2.rs:an_engine_start_shuts_an_open_adoption_window_on_an_existing_database`: a `state.db` the startup did not
/// see (it turned up after the app opened) still has its window open; the next start shuts it, whatever it decides
/// about the data, so the data is never adopted later. The start's refusal is asserted only as a refusal: which one
/// (another account's data, or the ownership proof Windows asks for) depends on the OS, and this test is about the mark.
/// Mutant: delete `data.db.close_adoption_window()…?;` in the start (`app/own_data_start.rs`): the mark stays 0 and
/// the adoption takes the data: red.
#[cfg(not(target_os = "macos"))]
#[test]
fn an_engine_start_shuts_an_open_adoption_window_on_an_existing_database() {
    in_own_process(
        "an_engine_start_shuts_an_open_adoption_window_on_an_existing_database",
        || {
            use beebeeb_engine::state_db::StateDb;

            let s = common::setup();
            // After the startup: one row, no owner, the window open.
            std::fs::create_dir_all(&s.state_dir).unwrap();
            let db = StateDb::open(s.state_db()).unwrap();
            let entry = beebeeb_engine::state_db::FileEntry {
                file_id: "f0000000-0000-4000-8000-000000000003".into(),
                path: "/found.txt".into(),
                status: beebeeb_engine::state_db::FileStatus::CloudOnly,
                size_bytes: 10,
                modified_at: 1_794_000_000,
                content_hash: None,
                remote_updated_at: 1_794_000_000,
                parent_id: None,
                item_kind: beebeeb_engine::state_db::ItemKind::File,
            };
            db.upsert_file_with_contract(&entry, |contract| contract.item_kind = entry.item_kind.clone())
                .unwrap();
            drop(db);
            assert_eq!(adoption_mark(&s.state_db()), 0, "the premise: the window is open");
            let folder = s.home.join("Beebeeb Test");
            std::fs::create_dir_all(&folder).unwrap();
            s.sign_in();
            s.unlock();
            let start = s.engine.start_sync_with(folder, &common::Port::free());
            assert!(start.is_err(), "unowned data is not taken by a start: {start:?}");
            assert_eq!(adoption_mark(&s.state_db()), 1, "the start shut the window");
            let db = StateDb::open(s.state_db()).unwrap();
            assert!(
                !db.adopt_owner_once(Some(&s.this_account())).unwrap(),
                "and nobody is adopted after it"
            );
            assert_eq!(db.owner().unwrap(), None);
        },
    );
}

// ── An unidentified session, named later (`part_2.rs`, `part_3.rs`) ────────────────────────────────────────────────────

/// A new handle on the same credential stores, state folder and API base (the app opened again), restored. The
/// earlier handle must be dropped first.
#[cfg(not(target_os = "macos"))]
fn reopened(
    account_id: &str,
    state_dir: &std::path::Path,
    api_base: &str,
    stores: &beebeeb_engine::signin::memory_store::MemoryCredentials,
) -> std::sync::Arc<beebeeb_engine::app::Engine<beebeeb_engine::signin::memory_store::MemoryCredentials>> {
    use beebeeb_engine::app::{Engine, EngineConfig};
    use beebeeb_engine::host::ChannelHost;
    use beebeeb_engine::identity::TEST;

    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: account_id.to_string(),
        state_dir: state_dir.to_path_buf(),
        api_base: api_base.to_string(),
    };
    let engine =
        std::sync::Arc::new(Engine::open(&TEST, config, host, stores.clone()).expect("the engine opens again"));
    engine.restore();
    engine
}

/// The session as a build from before the email and the owner tags were kept left it: signed in and unlocked, then the
/// account email and both owner tags removed from the store (the token and the key stay). Returns the setup with its
/// handle dropped: reopen it with [`reopened`].
#[cfg(not(target_os = "macos"))]
fn an_unnamed_session(s: &common::Setup) {
    use beebeeb_engine::keychain::AuthSecretStore;

    s.sign_in();
    s.unlock();
    let store = s.stores.store_for(&s.account_id);
    store.delete_account_email().unwrap();
    store.delete_session_owner().unwrap();
    store.delete_vault_key_owner().unwrap();
    assert_eq!(
        s.entries(),
        vec!["session token", "vault key"],
        "the premise: token and key, nobody named"
    );
}

/// `part_2.rs:an_unidentified_session_is_refused_and_takes_no_cached_profile_for_its_own`,
/// `part_2.rs:a_later_fetch_that_succeeds_names_the_session_and_the_start_binds_normally` and
/// `part_3.rs:an_unidentified_sessions_refusal_shows_in_sync_status_until_the_naming_start_runs`: a restored session
/// nobody is named for is refused (`IdentityUnknown`, shown as refused), also with the owner's profile cached beside
/// it. Beebeeb's answers that name nobody (a 500, a profile with no id) change nothing; the answer that names the
/// account lets the next start keep the owner's data. Each ask is from a new handle (an ask waits a minute per handle).
/// Mutants: delete `self.remember_session_user_id(Some(user_id.to_string()));` in `settle_session_owner_now`
/// (`app/session_owner_api.rs`): the last start stays `IdentityUnknown`: red; drop the profile filter in
/// `identity_of_session`: the first start binds: red.
#[cfg(not(target_os = "macos"))]
#[test]
fn an_unnamed_session_borrows_no_profile_and_starts_once_beebeeb_names_it() {
    in_own_process(
        "an_unnamed_session_borrows_no_profile_and_starts_once_beebeeb_names_it",
        || {
            use std::sync::Arc;
            use std::sync::atomic::{AtomicU8, Ordering};

            use beebeeb_engine::account_binding::Identity;
            use beebeeb_engine::app::account_api::OwnerSettled;
            use beebeeb_engine::app::{SyncStart, SyncStartRefused};
            use beebeeb_engine::config::DesktopConfig;
            use beebeeb_engine::identity::TEST;
            use beebeeb_engine::signin::gate::GateState;
            use beebeeb_engine::state_db::StateDb;
            use beebeeb_engine::sync_view::Start;
            use beebeeb_engine::test_api::{Reply, Request};

            /// How the fake answers `GET /api/v1/auth/me`.
            const PASS: u8 = 0;
            const FAILS: u8 = 1;
            const NAMES_NOBODY: u8 = 2;
            let me = Arc::new(AtomicU8::new(PASS));
            let s = {
                let me = me.clone();
                common::setup_with(move |inner, _, _| {
                    Arc::new(move |request: &Request| {
                        match (request.route() == "GET /api/v1/auth/me", me.load(Ordering::SeqCst)) {
                            (true, FAILS) => Reply::error(500, "internal"),
                            (true, NAMES_NOBODY) => Reply::ok(serde_json::json!({})),
                            _ => inner(request),
                        }
                    })
                })
            };
            DesktopConfig::update_for(&TEST, |config| {
                config.pause_sync = true;
                (true, ())
            })
            .expect("paused");
            s.state_db_with_one_row(&s.this_account());
            let folder = s.home.join("Beebeeb Test");
            std::fs::create_dir_all(&folder).unwrap();
            an_unnamed_session(&s);
            let common::Setup {
                account,
                fake,
                state_dir,
                stores,
                account_id,
                engine,
                ..
            } = s;
            drop(engine);
            let owner = Identity::new(Some(&account.user_id), Some(&account.email));
            let data_unchanged = || {
                let db = StateDb::open(beebeeb_engine::state_paths::state_db_path_from_state_dir(&state_dir)).unwrap();
                assert_eq!(db.owner().unwrap(), Some(owner.clone()), "the owner is unchanged");
                assert_eq!(db.list_files().unwrap().len(), 1, "the owner's row is unchanged");
            };
            let open_screen = || {
                let engine = reopened(&account_id, &state_dir, &fake.base, &stores);
                assert_eq!(
                    engine.gate(),
                    GateState::ScreenLocked,
                    "the premise: the key is in memory"
                );
                engine
                    .unlock_screen_platform_verified(engine.session_turn())
                    .expect("the screen opens");
                engine
            };

            // Nobody named; the owner's profile cached beside the session is not borrowed.
            let engine = open_screen();
            *engine
                .accounts()
                .active_account()
                .unwrap()
                .cached_profile
                .lock()
                .unwrap() = Some(profile_of(&account.user_id, &account.email));
            assert_eq!(engine.keys_for_engine_start(), Ok(Identity::default()));
            assert_eq!(
                engine.start_sync_with(folder.clone(), &common::Port::free()),
                Err(SyncStartRefused::IdentityUnknown)
            );
            assert_eq!(engine.view().start, Start::Refused(SyncStartRefused::IdentityUnknown));
            data_unchanged();
            drop(engine);

            // Beebeeb answers, but names nobody: twice nothing changes.
            for (answer, what) in [(FAILS, "a 500"), (NAMES_NOBODY, "a profile with no id")] {
                me.store(answer, Ordering::SeqCst);
                let engine = open_screen();
                assert_eq!(engine.settle_session_owner(), OwnerSettled::NotNow, "{what}");
                assert_eq!(
                    engine.start_sync_with(folder.clone(), &common::Port::free()),
                    Err(SyncStartRefused::IdentityUnknown),
                    "{what}"
                );
                data_unchanged();
                drop(engine);
            }

            // Beebeeb names the account: the next start keeps the owner's data.
            me.store(PASS, Ordering::SeqCst);
            let engine = open_screen();
            assert_eq!(engine.settle_session_owner(), OwnerSettled::Recorded);
            assert_eq!(
                engine.start_sync_with(folder.clone(), &common::Port::free()),
                Ok(SyncStart::Started)
            );
            assert_eq!(engine.view().start, Start::Running);
            let _ = engine.stop();
            data_unchanged();
        },
    );
}

/// `part_3.rs:an_answer_is_dropped_when_the_session_changed_while_the_fetch_ran`: Beebeeb's answer to "whose session
/// is this" counts only for the session that asked. Here the vault is locked while Beebeeb answers: nothing is
/// recorded, in memory or in the store.
/// Mutant: drop the `session_unchanged_since(turn)` check in `settle_session_owner_now` (`app/session_owner_api.rs`):
/// the answer is `Recorded`: red.
#[cfg(not(target_os = "macos"))]
#[test]
fn an_owner_answer_is_dropped_when_the_session_moved_while_beebeeb_answered() {
    in_own_process(
        "an_owner_answer_is_dropped_when_the_session_moved_while_beebeeb_answered",
        || {
            use std::sync::atomic::{AtomicBool, Ordering};
            use std::sync::{Arc, Mutex};

            use beebeeb_engine::app::Engine;
            use beebeeb_engine::app::account_api::OwnerSettled;
            use beebeeb_engine::keychain::AuthSecretStore;
            use beebeeb_engine::signin::memory_store::MemoryCredentials;
            use beebeeb_engine::test_api::Request;

            type Handle = Arc<Engine<MemoryCredentials>>;
            let locker: Arc<Mutex<Option<Handle>>> = Arc::default();
            let lock_once = Arc::new(AtomicBool::new(false));
            let s = {
                let (locker, lock_once) = (locker.clone(), lock_once.clone());
                common::setup_with(move |inner, _, _| {
                    Arc::new(move |request: &Request| {
                        if request.route() == "GET /api/v1/auth/me" && lock_once.swap(false, Ordering::SeqCst) {
                            let engine = locker.lock().unwrap().clone();
                            if let Some(engine) = engine {
                                engine.lock();
                            }
                        }
                        inner(request)
                    })
                })
            };
            an_unnamed_session(&s);
            let common::Setup {
                fake,
                state_dir,
                stores,
                account_id,
                engine,
                ..
            } = s;
            drop(engine);
            let engine = reopened(&account_id, &state_dir, &fake.base, &stores);
            engine
                .unlock_screen_platform_verified(engine.session_turn())
                .expect("the screen opens");
            *locker.lock().unwrap() = Some(engine.clone());
            lock_once.store(true, Ordering::SeqCst);

            assert_eq!(engine.settle_session_owner(), OwnerSettled::NotNow);
            assert!(
                !lock_once.load(Ordering::SeqCst),
                "the instrument: the lock ran during the answer"
            );
            assert_eq!(
                stores.store_for(&account_id).load_session_owner().unwrap(),
                None,
                "no owner tag was written"
            );
            locker.lock().unwrap().take();
        },
    );
}

// ── The sign-in names the session by the server (`part_2.rs`, `part_3.rs`) ─────────────────────────────────────────────

/// `part_3.rs:an_unidentified_sign_in_removes_the_previous_accounts_email_from_the_keychain`,
/// `part_3.rs:an_identified_sign_in_stores_its_own_email_in_place_of_the_previous_accounts`, and the half of
/// `part_2.rs:the_session_email_is_the_servers_spelling_and_nothing_else` for a server email that differs from the
/// typed one. An earlier account's email was left in the store (no token).
/// A sign-in whose account Beebeeb cannot name stores nothing and ends the new session. The next sign-in, named by
/// Beebeeb, stores the server's spelling of the email in place of the earlier one.
/// Mutants (`app.rs`): replace the `fetch_profile` error arm of `finish_sign_in` with a default profile (the first
/// sign-in stores a token: red); `let email = typed_email;` (the typed spelling is stored: red); delete the
/// `store_account_email` call in `store_first_sign_in` (the earlier email stays: red).
#[cfg(not(target_os = "macos"))]
#[test]
fn a_sign_in_is_named_only_by_the_server_and_replaces_an_earlier_email() {
    in_own_process(
        "a_sign_in_is_named_only_by_the_server_and_replaces_an_earlier_email",
        || {
            use std::sync::Arc;
            use std::sync::atomic::{AtomicBool, Ordering};

            use beebeeb_engine::keychain::AuthSecretStore;
            use beebeeb_engine::signin::gate::GateState;
            use beebeeb_engine::test_api::{Reply, Request};
            use zeroize::Zeroizing;

            const EARLIER: &str = "earlier@example.eu";
            let refused = Arc::new(AtomicBool::new(true));
            let s = {
                let refused = refused.clone();
                common::setup_with(move |inner, _, _| {
                    Arc::new(move |request: &Request| {
                        if request.route() != "GET /api/v1/auth/me" {
                            return inner(request);
                        }
                        if refused.load(Ordering::SeqCst) {
                            return Reply::error(503, "unavailable");
                        }
                        // The server's spelling of the address differs from what the person typed.
                        let mut reply = inner(request);
                        let mut profile: serde_json::Value = serde_json::from_slice(&reply.body).unwrap();
                        let spelled = profile["email"].as_str().unwrap().to_uppercase();
                        profile["email"] = serde_json::Value::String(spelled);
                        reply.body = profile.to_string().into_bytes();
                        reply
                    })
                })
            };
            let store = s.stores.store_for(&s.account_id);
            store.save_account_email(EARLIER).unwrap();

            let first = s
                .engine
                .sign_in_password(&s.account.email, Zeroizing::new(common::PASSWORD.into()));
            assert!(
                first.is_err(),
                "an account Beebeeb cannot name is not signed in: {first:?}"
            );
            assert_eq!(s.entries(), vec!["account email"], "nothing was stored");
            assert_eq!(store.load_account_email().unwrap().as_deref(), Some(EARLIER));
            assert_eq!(s.engine.gate(), GateState::SignedOut);
            let logouts = |s: &common::Setup| {
                s.fake
                    .requests()
                    .iter()
                    .filter(|r| r.route() == "POST /api/v1/auth/logout")
                    .count()
            };
            assert_eq!(logouts(&s), 1, "the new session was ended at the server");

            refused.store(false, Ordering::SeqCst);
            s.sign_in();
            let spelled = s.account.email.to_uppercase();
            assert_eq!(
                store.load_account_email().unwrap().as_deref(),
                Some(spelled.as_str()),
                "the server's spelling, in place of the earlier email"
            );
            assert_eq!(s.engine.account_email().as_deref(), Some(spelled.as_str()));
            assert_eq!(
                store.load_session_owner().unwrap().as_deref(),
                Some(s.account.user_id.as_str())
            );
            assert_eq!(logouts(&s), 1, "the installed session is not ended");
        },
    );
}

// ── The startup and the sign-out (`startup_session_tests.rs`, `signout_teardown_tests.rs`) ─────────────────────────────

/// `startup_session_tests.rs:probe_startup_session_fails_open_on_5xx_and_connection_refused`: a server that cannot be
/// reached, or answers 5xx, at startup never signs the person out. The engine's restore asks the server nothing (its
/// contract, `app.rs` `restore`: "No network"): the stored session opens screen-locked with its key in memory, all five
/// entries stay, and the failing server got no request.
/// Mutant: a fail-closed probe in `restore` (`if present && fetch_profile(...).await.is_err() { clear_session }`):
/// the restore is `SignedOut`: red.
#[cfg(not(target_os = "macos"))]
#[test]
fn a_start_with_the_server_down_or_failing_keeps_the_stored_session() {
    in_own_process(
        "a_start_with_the_server_down_or_failing_keeps_the_stored_session",
        || {
            use std::sync::Arc;

            use beebeeb_engine::signin::gate::GateState;
            use beebeeb_engine::test_api::{FakeApi, Reply};

            let s = common::setup();
            s.sign_in();
            s.unlock();
            assert_eq!(s.entries(), common::ENTRIES.to_vec());
            let common::Setup {
                fake,
                state_dir,
                stores,
                account_id,
                engine,
                ..
            } = s;
            drop(engine);
            let entries = |stores: &beebeeb_engine::signin::memory_store::MemoryCredentials| {
                use beebeeb_engine::keychain::AuthSecretStore;
                let store = stores.store_for(&account_id);
                [
                    store.load_session_token().ok().flatten().is_some(),
                    store.load_wrapped_master_key().ok().flatten().is_some(),
                    store.load_account_email().ok().flatten().is_some(),
                    store.load_vault_key_owner().ok().flatten().is_some(),
                    store.load_session_owner().ok().flatten().is_some(),
                ]
            };

            // The server cannot be reached: the fake is gone and its port is closed.
            let dead = fake.base.clone();
            drop(fake);
            let engine = reopened(&account_id, &state_dir, &dead, &stores);
            assert_eq!(
                engine.gate(),
                GateState::ScreenLocked,
                "offline: still signed in, key in memory"
            );
            assert_eq!(entries(&stores), [true; 5], "offline: every entry stays");
            drop(engine);

            // The server answers every request with a 503.
            let failing = FakeApi::start(Arc::new(|_| Reply::error(503, "unavailable")));
            let engine = reopened(&account_id, &state_dir, &failing.base, &stores);
            assert_eq!(
                engine.gate(),
                GateState::ScreenLocked,
                "5xx: still signed in, key in memory"
            );
            assert_eq!(entries(&stores), [true; 5], "5xx: every entry stays");
            assert_eq!(
                failing.routes(),
                Vec::<String>::new(),
                "the restore asked the server nothing"
            );
        },
    );
}

/// `startup_session_tests.rs:the_account_profile_cache_is_written_only_in_its_turn` (the half the engine keeps) and the
/// engine half of `signout_teardown_tests.rs:sign_out_clears_the_storage_summary`: after a sign-out nothing of the
/// account is named or counted: no user id, no email, and the storage figure cannot be read (no request is made for
/// it). This is the engine's side; the window's display of the figure is the native shell's.
/// Mutant: delete `*profile = None;` in `sign_out_holding_step` (`app.rs`): `user_id()` still names the account: red.
#[cfg(not(target_os = "macos"))]
#[test]
fn a_sign_out_leaves_nothing_of_the_account_named_or_counted() {
    in_own_process("a_sign_out_leaves_nothing_of_the_account_named_or_counted", || {
        let s = common::setup();
        s.sign_in();
        s.unlock();
        assert_eq!(s.engine.user_id().as_deref(), Some(s.account.user_id.as_str()));
        assert_eq!(s.engine.account_email().as_deref(), Some(s.account.email.as_str()));
        assert!(
            s.engine.storage_info().is_ok(),
            "the premise: the figure is read while signed in"
        );

        assert_eq!(s.engine.sign_out_keeping_local_data(), Ok(()));
        let from = s.fake.requests().len();
        assert_eq!(s.engine.user_id(), None, "no user id");
        assert_eq!(s.engine.account_email(), None, "no email");
        assert!(s.engine.storage_info().is_err(), "no storage figure");
        assert!(
            !s.routes_from(from).iter().any(|route| route.contains("/billing/usage")),
            "nothing was asked for it: {:?}",
            s.routes_from(from)
        );
    });
}

/// `startup_session_tests.rs:the_key_proof_decides_only_on_the_two_exact_codes_and_any_other_400_cannot_verify`, the
/// same answers: only `invalid_recovery_phrase` (the key is not the account's) and `recovery_check_missing` (no check
/// on file) decide; any other 400 says nothing about the key and is an error, never a wrong phrase. The unlock's
/// question accepts only a match.
/// Mutants (`signin/recovery.rs`): answer any other 400 with `Ok(RecoveryCheckAnswer::Mismatch)` (the near misses:
/// red); `recovered_key_matches_account` as `answer != RecoveryCheckAnswer::Mismatch` (no check on file accepted: red).
#[test]
fn the_key_proof_decides_only_on_the_two_exact_codes_and_any_other_400_cannot_verify() {
    use std::sync::{Arc, Mutex};

    use beebeeb_engine::signin::recovery::{RecoveryCheckAnswer, recovered_key_matches_account, recovery_check_answer};
    use beebeeb_engine::test_api::{FakeApi, Reply};

    let next: Arc<Mutex<Reply>> = Arc::new(Mutex::new(Reply::empty(500)));
    let fake = {
        let next = next.clone();
        FakeApi::start(Arc::new(move |_| next.lock().unwrap().clone()))
    };
    let answer_with = |status: u16, body: &str| {
        *next.lock().unwrap() = Reply {
            status,
            content_type: "application/json",
            body: body.as_bytes().to_vec(),
            retry_after: None,
        };
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap();
    let key = beebeeb_core::kdf::MasterKey::from_bytes([6u8; 32]);
    let ask = || runtime.block_on(recovery_check_answer(&client, &fake.base, "tok", &key));
    let matches = || runtime.block_on(recovered_key_matches_account(&client, &fake.base, "tok", &key));

    for (status, body, expected) in [
        (
            400,
            r#"{"error":"recovery_check_missing"}"#,
            RecoveryCheckAnswer::NoCheckOnFile,
        ),
        (
            400,
            r#"{"error":"invalid_recovery_phrase"}"#,
            RecoveryCheckAnswer::Mismatch,
        ),
        (200, r#"{"valid":true}"#, RecoveryCheckAnswer::Matches),
    ] {
        answer_with(status, body);
        assert_eq!(ask(), Ok(expected), "{status} {body}");
    }
    for body in [
        r#"{"error":"recovery_check_missing_or_other"}"#,
        r#"{"error":"Recovery_Check_Missing"}"#,
        r#"{"error":"Invalid_Recovery_Phrase"}"#,
        r#"{"error":"invalid_recovery_phrase "}"#,
        r#"{"error":"bad_request"}"#,
        r#"{"error":42}"#,
        r#"{}"#,
        "not json",
        "",
        "Failed to deserialize the JSON body into the target type: missing field `recovery_check`",
        "<html><body><h1>400 Bad Request</h1></body></html>",
    ] {
        answer_with(400, body);
        assert!(
            ask().is_err(),
            "a 400 without one of the two codes cannot verify, never a mismatch: {body:?}"
        );
    }
    answer_with(400, r#"{"error":"recovery_check_missing"}"#);
    assert_eq!(
        matches(),
        Ok(false),
        "no check on file proves nothing: the phrase is not accepted"
    );
    answer_with(400, r#"{"error":"invalid_recovery_phrase"}"#);
    assert_eq!(matches(), Ok(false), "a key that is not the account's is not accepted");
    answer_with(400, "not json");
    assert!(
        matches().is_err(),
        "any other 400 cannot verify, so it is not called a wrong phrase"
    );
}

// ── Source pins over engine files that stay ─────────────────────────────────────────────────────────────────────────

/// The engine's source text with its `mod tests` block cut off, lower case.
fn production_text(source: &str) -> String {
    let source = source.replace("\r\n", "\n");
    source[..source.find("#[cfg(test)]\nmod tests {").unwrap_or(source.len())].to_ascii_lowercase()
}

/// `part_2.rs:no_public_comment_announces_something_still_open`, for the two engine files it read that stay
/// (`account_binding.rs`, and `state_db.rs`, which is now `state_db.rs` + `state_db/*.rs`): no comment in the public
/// code says a weakness is open. (The needles are split so this file does not match itself.)
/// Mutant: add `// a known weakness` above `pub fn decide` in `account_binding.rs`: red.
#[test]
fn no_public_comment_in_the_binding_or_the_state_db_announces_something_still_open() {
    let needles = [
        concat!("is still", " open"),
        concat!("not", " closed"),
        concat!("un", "mitigated"),
        concat!("known", " weakness"),
        concat!("un", "fixed"),
    ];
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = vec![src.join("account_binding.rs"), src.join("state_db.rs")];
    let mut parts: Vec<_> = std::fs::read_dir(src.join("state_db"))
        .expect("the state_db folder")
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "rs"))
        .collect();
    parts.sort();
    assert!(
        parts.len() >= 10,
        "the instrument: state_db/*.rs found ({})",
        parts.len()
    );
    files.extend(parts);
    for file in &files {
        let text = production_text(&std::fs::read_to_string(file).expect("the source"));
        for needle in needles {
            assert!(!text.contains(needle), "{} says {needle:?}", file.display());
        }
    }
}

/// `startup_session_tests.rs:the_older_key_and_check_copies_are_wiped`, which read these two ENGINE files from the
/// shell: the loader's copy of the vault key and the recovery check's text are wiped when they go
/// (`Zeroizing`), the session's copy is the only one that outlives the loader, and the check's body is serialized from
/// a borrow (no second copy in a JSON value).
/// Mutant: in `signin/store.rs` `load_session_from_vault`, `zeroize::Zeroizing::new([0u8; 32])` → `Box::new([0u8; 32])`:
/// red.
#[test]
fn the_older_key_and_check_copies_are_wiped() {
    use beebeeb_engine::source_pin::{body_between, squeeze};

    let store = include_str!("../src/signin/store.rs").replace("\r\n", "\n");
    let load = body_between(&store, "fn load_session_from_vault<", "\n}\n");
    assert!(
        load.contains("let mut master_key = zeroize::Zeroizing::new([0u8; 32]);"),
        "{load}"
    );
    assert!(
        load.contains("master_key: *master_key,"),
        "the session's copy is the only one that outlives it:\n{load}"
    );
    let recovery = include_str!("../src/signin/recovery.rs").replace("\r\n", "\n");
    let check = body_between(&recovery, "pub async fn recovery_check_answer(", "\n}\n");
    assert!(
        check.contains("let recovery_check = zeroize::Zeroizing::new(encode_base64("),
        "{check}"
    );
    assert!(
        squeeze(check).contains(&squeeze(
            ".json(&RecoveryCheckBody { recovery_check: &recovery_check })"
        )),
        "{check}"
    );
    assert!(
        !check.contains("serde_json::json!({ \"recovery_check\""),
        "no second copy in a JSON value:\n{check}"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn the_test_build_is_refused_on_macos() {
    common::assert_the_test_build_is_refused();
}
