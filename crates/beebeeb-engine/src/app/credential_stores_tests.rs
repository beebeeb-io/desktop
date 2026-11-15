//! The flat layout of a 0.8.x PC at the engine handle (`credential_stores`): its session moves into the account's slot
//! at open, a failed move never fails the open, every sign-out clears it, and nothing of it reaches the log. Rule 5
//! (another account's flat session beside this PC's local data) needs a state folder of its own, so it is an
//! integration test: `tests/a_flat_session_of_another_account_never_takes_the_local_data.rs`. Every token, key and
//! email here is invented.

use std::sync::{Arc, Mutex};

use base64::Engine as _;

use super::{CredentialStores, LEGACY_FLAT_SLOT, PlatformCredentials};
use crate::app::{Engine, EngineConfig};
use crate::host::ChannelHost;
use crate::identity::REAL;
use crate::keychain::{AuthSecretStore, SecretBytes, SessionToken};
use crate::signin::gate::GateState;
use crate::signin::memory_store::{MemoryCredentials, MemoryStore};

type TestEngine = Engine<MemoryCredentials>;

/// What a store holds of a session: its token, its vault key, its email.
type Held = (Option<String>, Option<Vec<u8>>, Option<String>);

const NOTHING: Held = (None, None, None);

/// The account's slot (an invented id; the app's ids are UUIDs, `the_flat_slot_is_never_an_account_id`).
const SLOT: &str = "an-invented-slot";
const OLD_TOKEN: &str = "an-invented-flat-layout-token";
const OLD_KEY: &[u8; 32] = b"an-invented-32-byte-vault-key-01";
const OLD_EMAIL: &str = "flat.layout@example.eu";
const NEW_TOKEN: &str = "an-invented-account-slot-token";
const NEW_KEY: &[u8; 32] = b"an-invented-32-byte-vault-key-02";
const NEW_EMAIL: &str = "account.slot@example.eu";

/// A handle on `credentials` (a clone shares the stored items: a new handle on it is a restart).
fn open(credentials: &MemoryCredentials) -> Result<TestEngine, String> {
    let state_dir = crate::state_paths::init_for_test();
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: SLOT.into(),
        state_dir,
        api_base: "http://127.0.0.1:9".into(),
    };
    Engine::open(&REAL, config, host, credentials.clone())
}

fn opened(credentials: &MemoryCredentials) -> TestEngine {
    open(credentials).expect("the handle opens")
}

fn slot(credentials: &MemoryCredentials) -> MemoryStore {
    credentials.store_for(SLOT)
}

fn flat(credentials: &MemoryCredentials) -> MemoryStore {
    credentials
        .legacy_flat(&REAL)
        .expect("the test stores have a flat layout")
}

/// A session put in `store`: its token, its vault key, its email.
fn store_session(store: &MemoryStore, token: &str, key: &[u8; 32], email: &str) {
    store
        .save_session_token(&SessionToken::new(token).expect("a token"))
        .unwrap();
    store.save_wrapped_master_key(SecretBytes::new_master_key(key)).unwrap();
    store.save_account_email(email).unwrap();
}

fn held(store: &MemoryStore) -> Held {
    (
        store
            .load_session_token()
            .unwrap()
            .map(|token| token.expose_for_request().to_string()),
        store
            .load_wrapped_master_key()
            .unwrap()
            .map(|key| key.expose_for_crypto().to_vec()),
        store.load_account_email().unwrap(),
    )
}

fn session(token: &str, key: &[u8; 32], email: &str) -> Held {
    (Some(token.to_string()), Some(key.to_vec()), Some(email.to_string()))
}

/// Every line the engine logs while `body` runs on this thread, at every level (as `sign_out_revoke_tests` did in the
/// shell). Other crates' lines are left out: the engine's own log calls are under test.
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
    // tracing caches each callsite's interest for the whole process. While only ONE dispatcher is alive, tracing-core
    // takes that interest from the registering thread's own default, so a callsite first hit by another test thread,
    // which has no subscriber, is cached as "never" and this capture misses its events. A second live dispatcher keeps
    // registration on the all-dispatchers path.
    let _second_dispatcher = tracing::Dispatch::new(tracing_subscriber::registry());
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_ansi(false)
        .with_env_filter(tracing_subscriber::EnvFilter::new("beebeeb_engine=trace"))
        .finish();
    let guard = tracing::subscriber::set_default(subscriber);
    let out = body();
    drop(guard);
    let text = String::from_utf8_lossy(&buffer.lock().unwrap()).into_owned();
    (out, text)
}

/// Rule 1. Mutant M1: no `adopt_legacy_flat` call in `Engine::open` (the open is signed out, the flat items stay).
#[test]
fn a_flat_layout_session_opens_signed_in_and_moves_into_the_accounts_slot() {
    let credentials = MemoryCredentials::default();
    store_session(&flat(&credentials), OLD_TOKEN, OLD_KEY, OLD_EMAIL);

    let engine = opened(&credentials);

    // The engine's own rule for what follows the move: the move drops the slot's owner tags (task 2447), an untagged
    // key loads as it always did, and core's start rule (`LockFlow::at_start`) opens a stored session screen-locked
    // with the key in memory (the app lock is on when nothing says it is off).
    assert_eq!(engine.restore(), GateState::ScreenLocked);
    assert_eq!(engine.account_email().as_deref(), Some(OLD_EMAIL));
    assert_eq!(held(&slot(&credentials)), session(OLD_TOKEN, OLD_KEY, OLD_EMAIL));
    assert_eq!(held(&flat(&credentials)), NOTHING, "the flat layout is empty");
}

/// Rule 2. Mutant M2: the move's short-circuit (`keychain/legacy.rs`, "Migration complete") returns without
/// `sweep_legacy` (the leftover flat items stay).
#[test]
fn a_leftover_flat_trio_beside_a_complete_slot_is_swept_at_open() {
    let credentials = MemoryCredentials::default();
    store_session(&slot(&credentials), NEW_TOKEN, NEW_KEY, NEW_EMAIL);
    store_session(&flat(&credentials), OLD_TOKEN, OLD_KEY, OLD_EMAIL);

    let _engine = opened(&credentials);

    assert_eq!(held(&flat(&credentials)), NOTHING, "the leftover flat items are gone");
    assert_eq!(
        held(&slot(&credentials)),
        session(NEW_TOKEN, NEW_KEY, NEW_EMAIL),
        "the account's own session is unchanged"
    );
}

/// Rule 3. Mutant M3: `Engine::open` returns the move's error (`?` instead of the logged refusal).
#[test]
fn a_failed_read_back_keeps_the_flat_trio_and_open_still_succeeds() {
    let credentials = MemoryCredentials::default();
    store_session(&flat(&credentials), OLD_TOKEN, OLD_KEY, OLD_EMAIL);
    credentials.lose_key_writes(true);

    let engine = open(&credentials);
    credentials.lose_key_writes(false);

    assert!(
        engine.is_ok(),
        "a failed move failed the open: {:?}",
        engine.as_ref().err()
    );
    assert_eq!(
        held(&flat(&credentials)),
        session(OLD_TOKEN, OLD_KEY, OLD_EMAIL),
        "the flat items are kept"
    );
    // What the move wrote before its read-back: the token and the email, not the key.
    assert_eq!(
        held(&slot(&credentials)),
        (Some(OLD_TOKEN.to_string()), None, Some(OLD_EMAIL.to_string()))
    );
}

/// Rule 4, at the one production site that removes the stored token: `Engine::sign_out_holding_step`, which every
/// sign-out passes (`sign_out_keeping_local_data`, `signin_api`, `signup_api`, `account_actions_api`). Mutant M4: no
/// `clear_legacy_flat` call there (the next open moves the old session back in, signed in).
#[test]
fn a_sign_out_after_a_failed_move_leaves_nothing_to_sign_back_in() {
    let credentials = MemoryCredentials::default();
    store_session(&slot(&credentials), NEW_TOKEN, NEW_KEY, NEW_EMAIL);
    let engine = opened(&credentials);
    // The same account's flat items beside the complete slot: what a move whose read-back failed leaves once the
    // person signed in again. They are put there after the open, because the open's own move sweeps flat items beside
    // a complete slot (rule 2), and this test is about the sign-out.
    store_session(&flat(&credentials), OLD_TOKEN, NEW_KEY, NEW_EMAIL);
    assert_eq!(
        engine.restore(),
        GateState::ScreenLocked,
        "the precondition: signed in, so the sign-out reaches the clear"
    );

    engine.sign_out_keeping_local_data().expect("the sign-out");
    drop(engine);
    let restarted = opened(&credentials);

    assert_eq!(
        restarted.restore(),
        GateState::SignedOut,
        "no old session is moved back in"
    );
    assert_eq!(held(&slot(&credentials)), NOTHING, "the account's slot is empty");
    assert_eq!(held(&flat(&credentials)), NOTHING, "the flat layout is empty");
    assert!(
        credentials.is_empty(),
        "{} items are left in the credential store",
        credentials.len()
    );
}

/// Rule 6. Mutant M6: `LEGACY_FLAT_SLOT` in an account id's form (for example `00000000-0000-4000-8000-000000000000`).
#[test]
fn the_flat_slot_is_never_an_account_id() {
    // `DesktopConfig::ensure_account_id_at` mints `uuid::Uuid::new_v4().to_string()`: 36 characters, lower-case hex
    // digits and `-`.
    let in_an_id = |c: char| c.is_ascii_digit() || ('a'..='f').contains(&c) || c == '-';
    assert!(
        uuid::Uuid::parse_str(LEGACY_FLAT_SLOT).is_err(),
        "the flat slot reads as a UUID"
    );
    assert!(
        !LEGACY_FLAT_SLOT.chars().all(in_an_id),
        "every character of the flat slot can be in an account id"
    );
    let dir = tempfile::tempdir().expect("a temp folder");
    for n in 0..1_000 {
        let path = dir.path().join(format!("desktop-{n}.toml"));
        let id = crate::config::DesktopConfig::ensure_account_id_at(&path).expect("an account id is minted");
        assert!(
            id.len() == 36 && id.chars().all(in_an_id),
            "an account id outside the format: {id}"
        );
        assert_ne!(id, LEGACY_FLAT_SLOT);
    }
}

/// Rule 7: no line the engine logs while rules 1 to 4 run holds the token, the key (as text, hex or base64) or the
/// email, also when the store's own error text holds them. Mutant M7: `refused` logs the error's text (`%error`),
/// here a refusal whose text holds the token and the email.
#[test]
fn nothing_secret_reaches_the_log() {
    let ((), logs) = capture_logs(|| {
        // Rule 1, then rule 4: a move, and a sign-out with flat items beside the slot.
        let credentials = MemoryCredentials::default();
        store_session(&flat(&credentials), OLD_TOKEN, OLD_KEY, OLD_EMAIL);
        let engine = opened(&credentials);
        assert_eq!(engine.restore(), GateState::ScreenLocked);
        store_session(&flat(&credentials), OLD_TOKEN, OLD_KEY, OLD_EMAIL);
        engine.sign_out_keeping_local_data().expect("the sign-out");
        drop(engine);
        // Rule 2: leftover flat items beside a complete slot.
        let credentials = MemoryCredentials::default();
        store_session(&slot(&credentials), OLD_TOKEN, OLD_KEY, OLD_EMAIL);
        store_session(&flat(&credentials), OLD_TOKEN, OLD_KEY, OLD_EMAIL);
        drop(opened(&credentials));
        // Rule 3: a read-back that fails.
        let credentials = MemoryCredentials::default();
        store_session(&flat(&credentials), OLD_TOKEN, OLD_KEY, OLD_EMAIL);
        credentials.lose_key_writes(true);
        drop(opened(&credentials));
        // A refusal whose own text names the token and the email (a platform error can name what it touched).
        let credentials = MemoryCredentials::default();
        store_session(&flat(&credentials), OLD_TOKEN, OLD_KEY, OLD_EMAIL);
        let words = format!("refused {OLD_TOKEN} of {OLD_EMAIL}");
        credentials.refuse_token_writes(Some(words.as_str()));
        drop(opened(&credentials));
    });

    assert_eq!(
        logs.matches("the old credential layout was not moved or cleared")
            .count(),
        2,
        "the control: the two refused moves were logged:\n{logs}"
    );
    let hex: String = OLD_KEY.iter().map(|byte| format!("{byte:02x}")).collect();
    let secrets = [
        OLD_TOKEN.to_string(),
        OLD_EMAIL.to_string(),
        String::from_utf8(OLD_KEY.to_vec()).unwrap(),
        hex.clone(),
        hex.to_uppercase(),
        base64::engine::general_purpose::STANDARD_NO_PAD.encode(OLD_KEY),
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(OLD_KEY),
    ];
    for secret in secrets {
        assert!(!logs.contains(&secret), "the log holds {secret:?}");
    }
}

/// Rule 8: the platform stores have a flat layout on Windows and macOS only; Linux's store is a fail-closed stub that
/// would log a warning at every open.
#[test]
fn the_platform_flat_layout_is_asked_on_windows_and_macos_only() {
    assert_eq!(
        PlatformCredentials.legacy_flat(&REAL).is_some(),
        cfg!(any(target_os = "windows", target_os = "macos"))
    );
}
