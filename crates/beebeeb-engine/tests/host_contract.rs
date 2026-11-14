//! The contract of the engine handle for a host without Tauri (`beebeeb_engine::app::Engine`), against the local
//! fake API and the in-memory credential store: restore, password sign-in, the recovery phrase, a first sync of
//! production-shaped rows, the listing, pause, resume, stop, a restart, and what is refused.
//!
//! One process, one fake server, invented accounts. `BB_API_BASE` is set once, before anything reads it.

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use base64::Engine as _;
use beebeeb_engine::app::lock_api::{Call, CallRefused, LockRefused, LockState, LockedAction};
use beebeeb_engine::app::{Engine, EngineConfig, LocalStateRefused, SyncStart, SyncStop};
use beebeeb_engine::backup_actions::BackupMark;
use beebeeb_engine::config::DesktopConfig;
use beebeeb_engine::drive_v2::model::Sort;
use beebeeb_engine::host::{ChannelHost, EngineEvent, HostMessage};
use beebeeb_engine::keychain::{AuthSecretStore, AuthVault, LockFlag, SecretBytes};
use beebeeb_engine::signin::gate::{self, GateState, NextAfterSignIn, SignInProblem};
use beebeeb_engine::signin::key_owner::FOREIGN_KEY_NOT_REMOVED;
use beebeeb_engine::signin::memory_store::MemoryCredentials;
use beebeeb_engine::signin::start::EngineStartBlocked;
use beebeeb_engine::test_api::{FakeApi, OpaqueAccount, Reply, Request};
use serde_json::{Value, json};
use zeroize::Zeroizing;

const PASSWORD: &str = "correct horse battery staple";
const EMAIL: &str = "anna@example.test";
const EMAIL_2FA: &str = "bram@example.test";
const USER: &str = "11111111-1111-4111-8111-111111111111";
const USER_2FA: &str = "22222222-2222-4222-8222-222222222222";
const TOTP: &str = "482019";
/// A checksum-valid 12-word phrase (BIP-39's first test vector): the invented account's.
const PHRASE: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
/// Another valid phrase: not the account's.
const OTHER_PHRASE: &str = "legal winner thank year wave sausage worth useful legal winner thank yellow";

const BACKUPS: &str = "b0000000-0000-4000-8000-000000000001";
const DOCS: &str = "f0000000-0000-4000-8000-000000000001";
const REPORT: &str = "a0000000-0000-4000-8000-000000000001";
const APPLE: &str = "a0000000-0000-4000-8000-000000000002";

/// The account's key, derived once (the phrase's key derivation is slow by design).
fn master_key() -> beebeeb_core::kdf::MasterKey {
    static KEY: OnceLock<[u8; 32]> = OnceLock::new();
    beebeeb_core::kdf::MasterKey::from_bytes(
        *KEY.get_or_init(|| *beebeeb_core::recovery::recover_from_phrase(PHRASE).unwrap().to_bytes()),
    )
}

/// A production-shaped snapshot row: null request fields, an RFC 3339 `updated_at` (task 2445).
fn prod_row(id: &str, parent_id: Option<&str>, name: &str, is_folder: bool) -> Value {
    #[allow(deprecated)]
    let name_encrypted = beebeeb_core::encrypt::encrypt_name(&master_key(), id, name, None).unwrap();
    json!({
        "id": id,
        "parent_id": parent_id,
        "name_encrypted": name_encrypted,
        "file_request_id": null,
        "sender_ephemeral_pubkey": null,
        "wrapped_content_key": null,
        "acted_via_share_id": null,
        "uploaded_by": null,
        "owner_wrapped_file_key": null,
        "chunk_count": if is_folder { 0 } else { 1 },
        "size_bytes": if is_folder { 0 } else { 2000 },
        "is_folder": is_folder,
        "is_uploading": false,
        "content_hash": null,
        "has_thumbnail": false,
        "version_number": 1,
        "storage_pool_id": null,
        "is_trashed": false,
        "updated_at": "2026-11-10T12:00:00Z"
    })
}

fn snapshot() -> Value {
    let mut backups = prod_row(BACKUPS, None, "Backups", true);
    backups["system_role"] = json!("backups_root");
    json!({
        "seq_id": 7,
        "nodes": [
            backups,
            prod_row(DOCS, None, "Docs", true),
            prod_row(REPORT, Some(DOCS), "Report.pdf", false),
            prod_row(APPLE, None, "apple.txt", false),
        ]
    })
}

/// The fake server's memory: which session token belongs to which account, and the 2FA challenges.
#[derive(Default)]
struct World {
    /// OPAQUE `server_state` handle → the email that started it.
    logins: Vec<(String, String)>,
    /// session token → (user id, email)
    sessions: Vec<(String, &'static str, &'static str)>,
    partials: Vec<String>,
    revoked: Vec<String>,
}

/// How many requests of the sync engine the fake has answered.
fn sync_requests() -> usize {
    fake_api()
        .routes()
        .iter()
        .filter(|route| route.starts_with("GET /api/v1/sync/"))
        .count()
}

fn sign_ins() -> usize {
    fake_api()
        .routes()
        .iter()
        .filter(|route| route.as_str() == "POST /api/v1/opaque/login-start")
        .count()
}

fn sign_outs() -> usize {
    fake_api()
        .routes()
        .iter()
        .filter(|route| route.as_str() == "POST /api/v1/auth/logout")
        .count()
}

fn account_of(email: &str) -> Option<(&'static str, &'static str)> {
    match email {
        EMAIL => Some((USER, EMAIL)),
        EMAIL_2FA => Some((USER_2FA, EMAIL_2FA)),
        _ => None,
    }
}

fn mint(world: &mut World, user: &'static str, email: &'static str) -> String {
    let token = format!("session-{}", uuid::Uuid::new_v4());
    world.sessions.push((token.clone(), user, email));
    token
}

fn fake_api() -> &'static FakeApi {
    static API: OnceLock<FakeApi> = OnceLock::new();
    API.get_or_init(|| {
        let opaque = OpaqueAccount::register("every-test-account", PASSWORD);
        let world = Mutex::new(World::default());
        let snapshot = snapshot();
        let recovery_check = base64::engine::general_purpose::STANDARD
            .encode(&*beebeeb_core::opaque::compute_recovery_check(&master_key()));
        FakeApi::start(Arc::new(move |request: &Request| {
            let body = request.json();
            let mut world = world.lock().unwrap();
            let session = request
                .bearer()
                .and_then(|token| world.sessions.iter().find(|(t, _, _)| t == token).cloned())
                .filter(|(token, _, _)| !world.revoked.contains(token));
            match request.route().as_str() {
                "POST /api/v1/opaque/login-start" => {
                    let reply = opaque.login_start(&body);
                    if let Some(handle) = serde_json::from_slice::<Value>(&reply.body)
                        .ok()
                        .and_then(|v| v["server_state"].as_str().map(str::to_string))
                    {
                        let email = body["email"].as_str().unwrap_or_default().to_string();
                        world.logins.push((handle, email));
                    }
                    reply
                }
                "POST /api/v1/opaque/login-finish" => {
                    let handle = body["server_state"].as_str().unwrap_or_default().to_string();
                    let email = world
                        .logins
                        .iter()
                        .find(|(h, _)| *h == handle)
                        .map(|(_, email)| email.clone())
                        .unwrap_or_default();
                    if !opaque.login_finish(&body) {
                        return Reply::error(401, "invalid_credentials");
                    }
                    match account_of(&email) {
                        Some((_, EMAIL_2FA)) => {
                            let partial = format!("partial-{}", uuid::Uuid::new_v4());
                            world.partials.push(partial.clone());
                            Reply::ok(json!({ "requires_2fa": true, "partial_token": partial }))
                        }
                        Some((user, email)) => Reply::ok(json!({ "session_token": mint(&mut world, user, email) })),
                        None => Reply::error(401, "invalid_credentials"),
                    }
                }
                "POST /api/v1/auth/2fa/verify" => {
                    let partial = body["partial_token"].as_str().unwrap_or_default();
                    if world.partials.iter().any(|p| p == partial) && body["code"] == TOTP {
                        Reply::ok(json!({ "session_token": mint(&mut world, USER_2FA, EMAIL_2FA) }))
                    } else {
                        Reply::error(401, "invalid_code")
                    }
                }
                _ if session.is_none() => Reply::error(401, "unauthorized"),
                "GET /api/v1/auth/me" => {
                    let (_, user, email) = session.unwrap();
                    Reply::ok(json!({
                        "user_id": user, "email": email, "email_verified": true,
                        "created_at": "2026-01-01T00:00:00Z"
                    }))
                }
                "POST /api/v1/auth/verify-recovery-check" => {
                    if body["recovery_check"].as_str() == Some(recovery_check.as_str()) {
                        Reply::ok(json!({ "valid": true }))
                    } else {
                        Reply::error(400, "invalid_recovery_phrase")
                    }
                }
                "POST /api/v1/auth/logout" => {
                    world.revoked.push(session.unwrap().0);
                    Reply::ok(json!({}))
                }
                "GET /api/v1/sync/snapshot" => Reply::ok(snapshot.clone()),
                "GET /api/v1/sync/ops" => {
                    let since = request.query("since").and_then(|s| s.parse::<i64>().ok()).unwrap_or(0);
                    Reply::ok(json!({ "ops": [], "since": since }))
                }
                _ => Reply::json(404, json!({})),
            }
        }))
    })
}

/// Everything here shares one process-wide API base, config sandbox and state dir: one test at a time.
fn serial() -> MutexGuard<'static, ()> {
    static SERIAL: Mutex<()> = Mutex::new(());
    let guard = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    static ENV: OnceLock<()> = OnceLock::new();
    ENV.get_or_init(|| {
        // `BB_TEST_LOG=1` shows the engine's own log lines (they never carry a secret).
        if std::env::var_os("BB_TEST_LOG").is_some() {
            let _ = tracing_subscriber::fmt().with_test_writer().try_init();
        }
        // SAFETY: set once, under the lock, before any engine thread exists in this process.
        unsafe { std::env::set_var("BB_API_BASE", &fake_api().base) };
    });
    guard
}

struct Host {
    engine: Engine<MemoryCredentials>,
    messages: Receiver<HostMessage>,
    wakes: Arc<std::sync::atomic::AtomicUsize>,
}

fn open(credentials: &MemoryCredentials) -> Host {
    let wakes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counted = wakes.clone();
    let (host, messages) = ChannelHost::new(move || {
        counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    });
    let config = EngineConfig {
        account_id: "acct-host-contract".to_string(),
        state_dir: beebeeb_engine::state_paths::init_for_test(),
        api_base: beebeeb_engine::runner::api_base_url(),
    };
    assert_eq!(
        config.api_base,
        fake_api().base,
        "the handle talks to the fake API, never to production"
    );
    Host {
        engine: Engine::open(&beebeeb_engine::identity::REAL, config, host, credentials.clone()).unwrap(),
        messages,
        wakes,
    }
}

fn secret(text: &str) -> Zeroizing<String> {
    Zeroizing::new(text.to_string())
}

fn names(host: &Host, folder: Option<&str>) -> Vec<String> {
    match host.engine.listing(folder, Sort::default(), 0, 100) {
        Ok((items, _)) => items.into_iter().map(|item| item.name).collect(),
        Err(_) => Vec::new(),
    }
}

fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(90);
    while !done() {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {what}; the fake API saw {:?}",
            fake_api().routes()
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(target_os = "linux")]
fn engine_threads() -> usize {
    std::fs::read_dir("/proc/self/task")
        .unwrap()
        .filter_map(|task| std::fs::read_to_string(task.ok()?.path().join("comm")).ok())
        .filter(|name| name.trim() == "beebeeb-engine")
        .count()
}

#[test]
fn a_host_restores_signs_in_unlocks_syncs_lists_and_restarts() {
    let _serial = serial();
    let credentials = MemoryCredentials::default();
    let sync_root: PathBuf = tempfile::tempdir().unwrap().keep();
    DesktopConfig::update_for(&beebeeb_engine::identity::REAL, |cfg| {
        cfg.sync_root = Some(sync_root.clone());
        (true, ())
    })
    .unwrap();

    let host = open(&credentials);
    // Nothing stored: signed out, and nothing was asked of the server.
    assert_eq!(host.engine.restore(), GateState::SignedOut);
    assert!(credentials.is_empty());

    // The password: signed in (a token, no key on this computer), so the phrase is next and no engine may start.
    assert_eq!(
        host.engine
            .sign_in_password(&format!(" {} ", EMAIL.to_uppercase()), secret(PASSWORD)),
        Ok(NextAfterSignIn::RecoveryPhrase)
    );
    assert_eq!(host.engine.gate(), GateState::NeedsRecoveryPhrase);
    assert_eq!(host.engine.account_email().as_deref(), Some(EMAIL));
    assert_eq!(host.engine.user_id().as_deref(), Some(USER));
    assert_eq!(host.engine.keys_for_engine_start(), Err(EngineStartBlocked::NoSession));
    assert_eq!(host.engine.start_sync(sync_root.clone()), Ok(SyncStart::NoSession));
    assert!(!host.engine.is_syncing());

    // A valid phrase of another account opens nothing; the account's own unlocks.
    assert_eq!(
        host.engine.unlock_with_phrase(secret(OTHER_PHRASE)),
        Err(gate::WRONG_PHRASE)
    );
    assert_eq!(host.engine.gate(), GateState::NeedsRecoveryPhrase);
    assert_eq!(host.engine.unlock_with_phrase(secret(PHRASE)), Ok(()));
    assert_eq!(host.engine.gate(), GateState::Ready);
    let identity = host.engine.keys_for_engine_start().expect("keys in memory");
    assert!(identity.is_known());

    // The first sync: the snapshot's rows land with their real, nested names.
    assert_eq!(host.engine.start_sync(sync_root.clone()), Ok(SyncStart::Started));
    assert_eq!(host.engine.start_sync(sync_root.clone()), Ok(SyncStart::AlreadyRunning));
    wait_for("the first snapshot", || names(&host, None).len() == 3);
    // The Backups root first, then by name.
    assert_eq!(names(&host, None), ["Backups", "Docs", "apple.txt"]);
    assert_eq!(names(&host, Some(DOCS)), ["Report.pdf"]);
    let (root, total) = host.engine.listing(None, Sort::default(), 0, 100).unwrap();
    assert_eq!(total, 3);
    assert!(root[0].backup.is_protected, "the Backups root is marked");
    assert_eq!(root[1].backup, BackupMark::default());

    // Typed events arrived through the queue, each with a wake, in the engine's order: a status first.
    let mut events = Vec::new();
    while let Ok(message) = host.messages.try_recv() {
        events.push(message);
    }
    assert!(
        matches!(events.first(), Some(HostMessage::Event(EngineEvent::Status(_)))),
        "the first message is a status: {} message(s)",
        events.len()
    );
    assert!(host.wakes.load(std::sync::atomic::Ordering::SeqCst) >= events.len());
    for message in &events {
        if let HostMessage::Event(event) = message {
            assert!(
                !EngineEvent::NOT_TYPED_YET.contains(&event.name()),
                "{} is untyped",
                event.name()
            );
        }
    }

    // Pause, resume, stop.
    host.engine.pause().unwrap();
    assert!(host.engine.is_paused() && host.engine.is_syncing());
    assert!(
        DesktopConfig::load_for(&beebeeb_engine::identity::REAL)
            .unwrap()
            .pause_sync,
        "the pause is written, so a restart stays paused"
    );
    host.engine.resume().unwrap();
    assert!(!host.engine.is_paused());
    assert!(
        !DesktopConfig::load_for(&beebeeb_engine::identity::REAL)
            .unwrap()
            .pause_sync
    );
    let pauses: Vec<String> = host
        .messages
        .try_iter()
        .filter_map(|message| match message {
            HostMessage::Event(EngineEvent::PauseChanged(change)) => Some(serde_json::to_string(&change).unwrap()),
            _ => None,
        })
        .collect();
    assert_eq!(pauses, [r#"{"state":"paused"}"#, r#"{"state":"idle"}"#]);
    assert_eq!(host.engine.stop(), SyncStop::Stopped);
    assert!(!host.engine.is_syncing());
    assert_eq!(host.engine.stop(), SyncStop::WasNotRunning);
    let routes = fake_api().routes();
    assert!(routes.iter().any(|r| r == "GET /api/v1/sync/snapshot"), "{routes:?}");

    // The process ends: nothing of the engine is left running.
    drop(host);
    #[cfg(target_os = "linux")]
    assert_eq!(engine_threads(), 0, "no engine thread outlives the handle");

    // A restart, no password asked: the app starts SCREEN-LOCKED. The stored key (this account's) is in memory, so
    // sync starts and runs; the window is shown nothing of the account.
    let again = open(&credentials);
    assert_eq!(again.engine.restore(), GateState::ScreenLocked);
    assert_eq!(again.engine.lock_state(), LockState::ScreenLocked);
    assert_eq!(again.engine.view().gate, GateState::ScreenLocked);
    let synced_before = sync_requests();
    assert_eq!(again.engine.start_sync(sync_root.clone()), Ok(SyncStart::Started));
    wait_for("the engine asks the server while the screen is locked", || {
        sync_requests() > synced_before
    });
    assert!(again.engine.is_syncing(), "sync runs behind a locked screen");
    assert_eq!(again.engine.check_call(Call::Work), Ok(()));
    // A person's read is refused as locked (never as an error of its own), and nothing is listed.
    assert_eq!(
        again.engine.check_call(Call::Person(LockedAction::MutateFiles)),
        Err(CallRefused::ScreenLocked)
    );
    assert_eq!(again.engine.local_state_refusal(), Some(LocalStateRefused::Locked));
    assert!(again.engine.listing(None, Sort::default(), 0, 100).is_err());
    assert!(
        again.engine.storage_info().is_err(),
        "nothing is asked of the server for a person"
    );
    // The two things a locked screen allows: unlock and sign out. The password path is off in this build.
    assert_eq!(again.engine.check_call(Call::Person(LockedAction::SignOut)), Ok(()));
    assert_eq!(
        again.engine.unlock_screen_with_password(secret(PASSWORD)),
        Err(LockRefused::PasswordCheckUnavailable)
    );
    assert_eq!(
        again.engine.set_ask_password_at_start(true),
        Err(LockRefused::PasswordCheckUnavailable)
    );
    assert_eq!(again.engine.ask_password_at_start(), Some(false), "off by default");

    // Windows verified the person (the host's statement, stamped when the prompt was asked for): the screen opens,
    // and sync was never interrupted.
    let asked = again.engine.session_turn();
    let wakes = again.wakes.load(Ordering::SeqCst);
    assert_eq!(again.engine.unlock_screen_platform_verified(asked), Ok(()));
    assert!(again.wakes.load(Ordering::SeqCst) > wakes, "the window is woken");
    assert_eq!(again.engine.gate(), GateState::Ready);
    assert_eq!(again.engine.lock_state(), LockState::Open);
    assert!(again.engine.is_syncing(), "an unlock neither stops nor restarts sync");
    assert_eq!(again.engine.account_email().as_deref(), Some(EMAIL));
    assert_eq!(names(&again, None), ["Backups", "Docs", "apple.txt"]);
    assert_eq!(
        again
            .engine
            .unlock_screen_platform_verified(again.engine.session_turn()),
        Err(LockRefused::NotAtThisLevel)
    );

    // Lock now: the screen locks, sync runs on, and an answer of a prompt asked before this lock opens nothing.
    assert_eq!(again.engine.lock_screen(), Ok(()));
    assert_eq!(again.engine.gate(), GateState::ScreenLocked);
    assert!(again.engine.is_syncing());
    assert_ne!(
        again.engine.session_turn(),
        asked,
        "what was asked before the lock is another turn's"
    );
    assert_eq!(
        again.engine.unlock_screen_platform_verified(asked),
        Err(LockRefused::Overtaken)
    );
    assert_eq!(again.engine.gate(), GateState::ScreenLocked);
    assert!(names(&again, None).is_empty());
    assert_eq!(
        again
            .engine
            .unlock_screen_platform_verified(again.engine.session_turn()),
        Ok(())
    );

    // The vault lock: the mark is stored, sync stops, the keys leave memory; the stored key stays.
    let locked = again.engine.lock_vault();
    assert_eq!(locked.stopped, SyncStop::Stopped);
    assert!(locked.survives_restart);
    assert_eq!(again.engine.gate(), GateState::Locked);
    assert_eq!(again.engine.lock_state(), LockState::VaultClosed);
    assert!(!again.engine.is_syncing());
    assert_eq!(again.engine.keys_for_engine_start(), Err(EngineStartBlocked::NoSession));
    assert_eq!(again.engine.check_call(Call::Work), Err(CallRefused::VaultClosed));
    assert_eq!(again.engine.start_sync(sync_root.clone()), Ok(SyncStart::NoSession));
    // No screen proof opens a closed vault, and a closed vault is not screen-locked.
    assert_eq!(
        again
            .engine
            .unlock_screen_platform_verified(again.engine.session_turn()),
        Err(LockRefused::NeedsThePassword)
    );
    assert_eq!(again.engine.lock_screen(), Err(LockRefused::NotAtThisLevel));
    // It survives a restart, and the key entry is not read at all while it is closed: not by a start, not by the
    // gate's question.
    let key_reads = credentials.key_reads();
    assert_eq!(again.engine.restore(), GateState::Locked);
    let third = open(&credentials);
    assert_eq!(third.engine.restore(), GateState::Locked);
    assert_eq!(third.engine.gate(), GateState::Locked);
    assert_eq!(third.engine.keys_for_engine_start(), Err(EngineStartBlocked::NoSession));
    assert_eq!(
        credentials.key_reads(),
        key_reads,
        "a closed vault's key entry is not read"
    );
    drop(third);
    // Sign-out removes the token, the key, their owners and the lock's flags, and keeps the files' state.
    assert_eq!(again.engine.sign_out_keeping_local_data(), Ok(()));
    assert_eq!(again.engine.gate(), GateState::SignedOut);
    assert!(
        credentials.is_empty(),
        "nothing of the account is left in the credential store"
    );
    // Signed out, the handle lists nothing (the local state is the signed-in account's alone, task 2488); the
    // state on disk is as it was.
    assert_eq!(
        again.engine.local_state_refusal(),
        Some(beebeeb_engine::app::LocalStateRefused::SignedOut)
    );
    assert!(names(&again, None).is_empty(), "nothing is listed without a session");
    let state_db =
        beebeeb_engine::state_paths::state_db_path_from_state_dir(&beebeeb_engine::state_paths::init_for_test());
    let rows = beebeeb_engine::state_db::StateDb::open(state_db)
        .unwrap()
        .list_files()
        .unwrap()
        .len();
    assert_eq!(
        rows, 4,
        "sign-out with keep deletes no local state (3 at the root, 1 inside a folder)"
    );
}

#[test]
fn wrong_inputs_map_to_the_gates_words_and_change_nothing() {
    let _serial = serial();
    let credentials = MemoryCredentials::default();
    let host = open(&credentials);
    assert_eq!(host.engine.restore(), GateState::SignedOut);

    assert_eq!(
        host.engine.sign_in_password(EMAIL, secret("not the password")),
        Err(SignInProblem::Invalid)
    );
    assert_eq!(
        host.engine.sign_in_password("nobody@example.test", secret(PASSWORD)),
        Err(SignInProblem::Invalid)
    );
    assert_eq!(host.engine.gate(), GateState::SignedOut);
    assert!(credentials.is_empty(), "a refused sign-in stores nothing");
    // Five wrong tries in a row: the lock hint.
    for _ in 0..2 {
        let _ = host.engine.sign_in_password(EMAIL, secret("still not it"));
    }
    assert_eq!(
        host.engine.sign_in_password(EMAIL, secret("still not it")),
        Err(SignInProblem::LockedHint)
    );

    // A two-factor account: a wrong code keeps the challenge, the right one signs in.
    assert_eq!(
        host.engine.sign_in_password(EMAIL_2FA, secret(PASSWORD)),
        Ok(NextAfterSignIn::TwoFactor)
    );
    assert!(credentials.is_empty(), "nothing is stored before the second factor");
    assert_eq!(host.engine.two_factor(secret("000000")), Err(gate::WRONG_2FA_CODE));
    assert_eq!(host.engine.gate(), GateState::SignedOut);
    assert_eq!(
        host.engine.two_factor(secret(TOTP)),
        Ok(NextAfterSignIn::RecoveryPhrase)
    );
    assert_eq!(host.engine.user_id().as_deref(), Some(USER_2FA));
    // This account's phrase is not the fake's recorded one.
    assert_eq!(
        host.engine
            .unlock_with_phrase(secret("twelve words that are not a phrase at all so no")),
        Err(gate::phrase_error(""))
    );
    assert_eq!(host.engine.sign_out_keeping_local_data(), Ok(()));
    assert!(credentials.is_empty());
}

/// Task 2447 through the handle's entry points: a vault key that is another account's is never loaded next to
/// this account's session, and a sign-in removes it before it stores its token.
#[test]
fn another_accounts_stored_key_is_refused_and_removed() {
    let _serial = serial();
    let credentials = MemoryCredentials::default();
    let store = credentials.store_for("acct-host-contract");
    let foreign = [9u8; 32];

    // Restore: a token whose owner is this account, next to a key tagged for another user. Not loaded.
    {
        let vault = AuthVault::new(store.clone());
        vault
            .install_session(beebeeb_engine::keychain::SessionToken::new("stored-token".to_string()).unwrap())
            .unwrap();
        vault.bind_session_owner(USER).unwrap();
        vault
            .store_wrapped_master_key(SecretBytes::new_master_key(&foreign))
            .unwrap();
        vault
            .bind_vault_key_owner("99999999-9999-4999-8999-999999999999")
            .unwrap();
    }
    let host = open(&credentials);
    assert_eq!(host.engine.restore(), GateState::NeedsRecoveryPhrase);
    assert_eq!(host.engine.keys_for_engine_start(), Err(EngineStartBlocked::NoSession));
    assert!(
        store.load_wrapped_master_key().unwrap().is_some(),
        "restore deletes nothing"
    );
    assert_eq!(host.engine.sign_out_keeping_local_data(), Ok(()));
    drop(host);

    // Sign-in: a key left behind with another owner's tag (no token) goes before the token is stored.
    {
        let vault = AuthVault::new(store.clone());
        vault
            .store_wrapped_master_key(SecretBytes::new_master_key(&foreign))
            .unwrap();
        vault
            .bind_vault_key_owner("99999999-9999-4999-8999-999999999999")
            .unwrap();
    }
    let host = open(&credentials);
    assert_eq!(host.engine.restore(), GateState::SignedOut);
    assert_eq!(
        host.engine.sign_in_password(EMAIL, secret(PASSWORD)),
        Ok(NextAfterSignIn::RecoveryPhrase)
    );
    assert!(
        store.load_wrapped_master_key().unwrap().is_none(),
        "the foreign key is gone"
    );
    assert_eq!(host.engine.keys_for_engine_start(), Err(EngineStartBlocked::NoSession));
    assert_eq!(gate::problem_from(FOREIGN_KEY_NOT_REMOVED, 0), SignInProblem::StaleKey);
    assert_eq!(host.engine.sign_out_keeping_local_data(), Ok(()));
}

/// Signed in as `email` and unlocked with the phrase: the key is in the store and in memory, the app is open.
fn open_vault(credentials: &MemoryCredentials, email: &str) -> Host {
    let host = open(credentials);
    assert_eq!(host.engine.restore(), GateState::SignedOut);
    match host.engine.sign_in_password(email, secret(PASSWORD)) {
        Ok(NextAfterSignIn::TwoFactor) => {
            assert_eq!(
                host.engine.two_factor(secret(TOTP)),
                Ok(NextAfterSignIn::RecoveryPhrase)
            );
        }
        next => assert_eq!(next, Ok(NextAfterSignIn::RecoveryPhrase)),
    }
    assert_eq!(host.engine.unlock_with_phrase(secret(PHRASE)), Ok(()));
    assert_eq!(host.engine.lock_state(), LockState::Open);
    host
}

/// [`open_vault`], then the vault lock: the key is in the store, not in memory, and the closed mark is set.
fn locked(credentials: &MemoryCredentials) -> Host {
    let host = open_vault(credentials, EMAIL);
    assert_eq!(host.engine.lock(), SyncStop::WasNotRunning);
    assert_eq!(host.engine.gate(), GateState::Locked);
    host
}

fn flag(credentials: &MemoryCredentials, flag: LockFlag) -> Option<Vec<u8>> {
    credentials
        .store_for("acct-host-contract")
        .load_lock_flag(flag)
        .unwrap()
}

#[test]
fn a_closed_vault_opens_with_the_accounts_password_proven_in_a_new_sign_in() {
    let _serial = serial();
    let credentials = MemoryCredentials::default();
    let host = locked(&credentials);
    assert_eq!(flag(&credentials, LockFlag::VaultClosed).as_deref(), Some(&b"1"[..]));
    let store = credentials.store_for("acct-host-contract");
    let closed_token = store.load_session_token().unwrap().unwrap();
    let wakes = host.wakes.load(Ordering::SeqCst);

    // Not the password: refused in the sign-in form's one sentence; the vault stays closed and marked.
    assert_eq!(
        host.engine.unlock_vault_with_password(secret("not the password")),
        Err(SignInProblem::Invalid)
    );
    assert_eq!(host.engine.gate(), GateState::Locked);
    assert_eq!(host.engine.keys_for_engine_start(), Err(EngineStartBlocked::NoSession));
    assert_eq!(flag(&credentials, LockFlag::VaultClosed).as_deref(), Some(&b"1"[..]));
    assert!(
        host.wakes.load(Ordering::SeqCst) > wakes,
        "a refusal wakes the window too"
    );

    // The password: a new session for the same account, the stored key in memory under it, the mark cleared, and
    // the session it replaced ended at the server.
    let ended = sign_outs();
    assert_eq!(
        host.engine.unlock_vault_with_password(secret(PASSWORD)),
        Ok(NextAfterSignIn::Drive)
    );
    assert_eq!(host.engine.gate(), GateState::Ready);
    assert_eq!(host.engine.lock_state(), LockState::Open);
    assert_eq!(
        host.engine.keys_for_engine_start().map(|identity| identity.user_id),
        Ok(Some(USER.to_string()))
    );
    assert_eq!(
        flag(&credentials, LockFlag::VaultClosed),
        None,
        "only the strong proof clears the mark"
    );
    let new_token = store.load_session_token().unwrap().unwrap();
    assert_ne!(new_token.expose_for_request(), closed_token.expose_for_request());
    assert_eq!(sign_outs(), ended + 1, "the replaced session is ended at the server");
    // The next start is the default one again: screen-locked, the key loaded.
    let next = open(&credentials);
    assert_eq!(next.engine.restore(), GateState::ScreenLocked);
    drop(next);
    assert_eq!(host.engine.sign_out_keeping_local_data(), Ok(()));
    assert!(credentials.is_empty(), "the lock's flags leave with the session");
}

#[test]
fn the_strong_unlock_asks_the_authenticator_code_and_opens_a_locked_screen_too() {
    let _serial = serial();
    let credentials = MemoryCredentials::default();
    // An account with an authenticator: the password alone opens nothing, the code finishes it.
    let host = open_vault(&credentials, EMAIL_2FA);
    assert_eq!(host.engine.lock(), SyncStop::WasNotRunning);
    assert_eq!(
        host.engine.unlock_vault_with_password(secret(PASSWORD)),
        Ok(NextAfterSignIn::TwoFactor)
    );
    assert_eq!(host.engine.gate(), GateState::Locked);
    assert_eq!(host.engine.lock_state(), LockState::VaultClosed);
    assert_eq!(host.engine.two_factor(secret(TOTP)), Ok(NextAfterSignIn::Drive));
    assert_eq!(host.engine.gate(), GateState::Ready);
    assert_eq!(
        host.engine.keys_for_engine_start().map(|identity| identity.user_id),
        Ok(Some(USER_2FA.to_string()))
    );
    drop(host);

    // From a locked screen (a PC without Windows Hello): the same proof opens it. The session it replaces is
    // ended, and nothing keeps working under it: the key left memory before the new session was stored.
    let host = open(&credentials);
    assert_eq!(host.engine.restore(), GateState::ScreenLocked);
    let before = host.engine.session_turn();
    let ended = sign_outs();
    assert_eq!(
        host.engine.unlock_vault_with_password(secret(PASSWORD)),
        Ok(NextAfterSignIn::TwoFactor)
    );
    assert_eq!(
        host.engine.gate(),
        GateState::ScreenLocked,
        "a password without its code opens nothing"
    );
    assert_eq!(host.engine.two_factor(secret(TOTP)), Ok(NextAfterSignIn::Drive));
    assert_eq!(host.engine.gate(), GateState::Ready);
    assert_eq!(host.engine.lock_state(), LockState::Open);
    assert_ne!(host.engine.session_turn(), before);
    assert_eq!(sign_outs(), ended + 1);
    assert_eq!(host.engine.sign_out_keeping_local_data(), Ok(()));
}

#[test]
fn a_password_unlocks_only_the_account_whose_session_is_stored() {
    let _serial = serial();
    let credentials = MemoryCredentials::default();
    let host = open(&credentials);
    assert_eq!(host.engine.restore(), GateState::SignedOut);
    let asked = sign_ins();
    // Signed out: nothing to unlock, and nothing is asked of the server.
    assert_eq!(
        host.engine.unlock_vault_with_password(secret(PASSWORD)),
        Err(SignInProblem::Other)
    );
    assert_eq!(sign_ins(), asked);
    drop(host);

    // The stored session is recorded as another account's than the one the server names for the proof: nothing is
    // stored, the new session is ended, the vault stays closed and marked.
    let host = locked(&credentials);
    let store = credentials.store_for("acct-host-contract");
    let closed_token = store.load_session_token().unwrap().unwrap();
    store
        .save_session_owner("99999999-9999-4999-8999-999999999999")
        .unwrap();
    let ended = sign_outs();
    assert_eq!(
        host.engine.unlock_vault_with_password(secret(PASSWORD)),
        Err(SignInProblem::Other)
    );
    assert_eq!(host.engine.gate(), GateState::Locked);
    assert_eq!(
        store.load_session_token().unwrap().unwrap().expose_for_request(),
        closed_token.expose_for_request(),
        "the stored session is not replaced"
    );
    assert_eq!(sign_outs(), ended + 1, "the session the refused proof made is ended");
    assert_eq!(flag(&credentials, LockFlag::VaultClosed).as_deref(), Some(&b"1"[..]));
    store.save_session_owner(USER).unwrap();
    assert_eq!(host.engine.sign_out_keeping_local_data(), Ok(()));
}

#[test]
fn a_start_is_closed_by_the_setting_the_mark_or_a_flag_that_cannot_be_read_and_reads_no_key() {
    let _serial = serial();
    let credentials = MemoryCredentials::default();
    let host = open_vault(&credentials, EMAIL);
    drop(host);
    let store = credentials.store_for("acct-host-contract");
    let start = |expected: GateState| {
        let reads = credentials.key_reads();
        let host = open(&credentials);
        assert_eq!(host.engine.restore(), expected);
        assert_eq!(host.engine.gate(), expected);
        let read = credentials.key_reads() - reads;
        if expected == GateState::Locked {
            assert_eq!(host.engine.lock_state(), LockState::VaultClosed);
            assert_eq!(host.engine.keys_for_engine_start(), Err(EngineStartBlocked::NoSession));
        }
        read
    };
    // The default: no flag was ever written, the key is loaded.
    assert!(start(GateState::ScreenLocked) > 0);

    // The lock's flags cannot be read: closed, and the key entry is not touched. The mark could not be written
    // either, so this closing lasts for that run only.
    credentials.fail_lock_flags(true);
    assert_eq!(start(GateState::Locked), 0);
    credentials.fail_lock_flags(false);
    assert_eq!(flag(&credentials, LockFlag::VaultClosed), None);
    assert!(start(GateState::ScreenLocked) > 0);

    // Bytes that are neither flag value: unreadable, so closed; and a closed start writes the mark.
    store.save_lock_flag(LockFlag::AskPasswordAtStart, b"yes").unwrap();
    assert_eq!(start(GateState::Locked), 0);
    assert_eq!(flag(&credentials, LockFlag::VaultClosed).as_deref(), Some(&b"1"[..]));
    // The mark alone keeps every later start closed, whatever the setting says by then.
    store.save_lock_flag(LockFlag::AskPasswordAtStart, b"0").unwrap();
    assert_eq!(start(GateState::Locked), 0);

    // The strong proof opens it and clears the mark; with the setting on, the next start is closed again.
    let host = open(&credentials);
    assert_eq!(host.engine.restore(), GateState::Locked);
    assert_eq!(
        host.engine.unlock_vault_with_password(secret(PASSWORD)),
        Ok(NextAfterSignIn::Drive)
    );
    drop(host);
    assert!(start(GateState::ScreenLocked) > 0);
    store.save_lock_flag(LockFlag::AskPasswordAtStart, b"1").unwrap();
    assert_eq!(start(GateState::Locked), 0);
    assert_eq!(flag(&credentials, LockFlag::VaultClosed).as_deref(), Some(&b"1"[..]));

    let host = open(&credentials);
    host.engine.restore();
    assert_eq!(host.engine.sign_out_keeping_local_data(), Ok(()));
    assert!(credentials.is_empty());
}

#[test]
fn leaving_a_half_done_sign_in_forgets_the_code_step_and_signs_a_keyless_session_out() {
    let _serial = serial();
    let credentials = MemoryCredentials::default();
    let host = open(&credentials);
    assert_eq!(host.engine.restore(), GateState::SignedOut);

    // A proven password that waits for its code: forgotten, so the right code no longer signs in.
    assert_eq!(
        host.engine.sign_in_password(EMAIL_2FA, secret(PASSWORD)),
        Ok(NextAfterSignIn::TwoFactor)
    );
    assert_eq!(host.engine.discard_partial_sign_in(), Ok(()));
    assert_eq!(host.engine.two_factor(secret(TOTP)), Err(gate::TWO_FACTOR_EXPIRED));
    assert_eq!(host.engine.gate(), GateState::SignedOut);
    assert!(credentials.is_empty(), "nothing was stored");

    // Signed in without a key here (the twelve words were asked): signed out, so another account can sign in.
    assert_eq!(
        host.engine.sign_in_password(EMAIL, secret(PASSWORD)),
        Ok(NextAfterSignIn::RecoveryPhrase)
    );
    assert_eq!(host.engine.discard_partial_sign_in(), Ok(()));
    assert_eq!(host.engine.gate(), GateState::SignedOut);
    assert!(credentials.is_empty(), "the token left the store");
    assert_eq!(host.engine.account_email(), None);
    assert_eq!(
        host.engine.sign_in_password(EMAIL_2FA, secret(PASSWORD)),
        Ok(NextAfterSignIn::TwoFactor)
    );
    assert_eq!(
        host.engine.two_factor(secret(TOTP)),
        Ok(NextAfterSignIn::RecoveryPhrase)
    );
    assert_eq!(host.engine.user_id().as_deref(), Some(USER_2FA));
    assert_eq!(host.engine.sign_out_keeping_local_data(), Ok(()));
    drop(host);

    // A locked vault keeps its session and its key: leaving a screen never signs it out.
    let credentials = MemoryCredentials::default();
    let host = locked(&credentials);
    assert_eq!(host.engine.discard_partial_sign_in(), Ok(()));
    assert_eq!(host.engine.gate(), GateState::Locked);
    assert!(!credentials.is_empty());
    assert_eq!(host.engine.sign_out_keeping_local_data(), Ok(()));
}
