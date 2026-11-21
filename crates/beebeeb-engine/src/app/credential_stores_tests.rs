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
use crate::identity::TEST;
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
    Engine::open(&TEST, config, host, credentials.clone())
}

fn opened(credentials: &MemoryCredentials) -> TestEngine {
    open(credentials).expect("the handle opens")
}

fn slot(credentials: &MemoryCredentials) -> MemoryStore {
    credentials.store_for(SLOT)
}

fn flat(credentials: &MemoryCredentials) -> MemoryStore {
    credentials
        .legacy_flat(&TEST)
        .expect("the test stores have a flat layout")
        .install_slot()
        .clone()
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
        PlatformCredentials.legacy_flat(&TEST).is_some(),
        cfg!(any(target_os = "windows", target_os = "macos"))
    );
}

// ── CP1 rows S1 and S2 (batch 3, `b3-engine-credentials`) ─────────────────────────────────────────────────────────────

use std::sync::atomic::{AtomicBool, Ordering};

use zeroize::Zeroizing;

use super::FLAT_LAYOUT_RETIRED;
use crate::identity::EngineIdentity;
use crate::keychain::{AuthResult, AuthStoreError, LockFlag};
use crate::test_api::FakeApi;
use crate::test_api::fake_account::{FakeAccount, TEST_PHRASE, account_handler};

/// A handle on `stores` at `api_base` (a clone shares the stored items: a new handle on it is a restart).
fn open_on<C: CredentialStores + Clone>(stores: &C, api_base: &str) -> Engine<C> {
    let state_dir = crate::state_paths::init_for_test();
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: SLOT.into(),
        state_dir,
        api_base: api_base.into(),
    };
    Engine::open(&TEST, config, host, stores.clone()).expect("the handle opens")
}

/// No server listens there; nothing in these tests needs one.
const NO_SERVER: &str = "http://127.0.0.1:9";

/// The test stores with a flat layout whose removals can be refused (a platform store can refuse one: on a Mac, an
/// item another build of the app created), and a record of every save and removal asked of the flat layout, in order.
#[derive(Clone, Default)]
struct FlatProbe {
    items: MemoryCredentials,
    refuse_flat_deletes: Arc<AtomicBool>,
    asked: Arc<Mutex<Vec<String>>>,
}

impl FlatProbe {
    fn refuse_flat_deletes(&self, refuse: bool) {
        self.refuse_flat_deletes.store(refuse, Ordering::SeqCst);
    }

    fn asked(&self) -> Vec<String> {
        self.asked.lock().unwrap().clone()
    }

    /// The account's slot, as the test reads and seeds it (not recorded).
    fn slot_items(&self) -> MemoryStore {
        self.items.store_for(SLOT)
    }

    /// The flat layout, as the test reads and seeds it (not recorded).
    fn flat_items(&self) -> MemoryStore {
        self.items.store_for(LEGACY_FLAT_SLOT)
    }
}

impl CredentialStores for FlatProbe {
    type Store = ProbedStore;
    fn for_account(&self, _identity: &'static EngineIdentity, account_id: &str) -> Self::Store {
        ProbedStore {
            store: self.items.store_for(account_id),
            probe: None,
        }
    }
    fn legacy_flat(&self, _identity: &'static EngineIdentity) -> Option<Self::Store> {
        Some(ProbedStore {
            store: self.items.store_for(LEGACY_FLAT_SLOT),
            probe: Some(self.clone()),
        })
    }
}

/// One slot of a [`FlatProbe`]; `probe` is set on the flat layout only.
struct ProbedStore {
    store: MemoryStore,
    probe: Option<FlatProbe>,
}

impl ProbedStore {
    /// Record a save or a removal asked of the flat layout; a removal is refused while the probe says so.
    fn ask(&self, what: &str, removal: bool) -> AuthResult<()> {
        let Some(probe) = &self.probe else {
            return Ok(());
        };
        probe.asked.lock().unwrap().push(what.to_string());
        if removal && probe.refuse_flat_deletes.load(Ordering::SeqCst) {
            return Err(AuthStoreError::Backend(
                "the test store refuses this removal".to_string(),
            ));
        }
        Ok(())
    }
}

impl AuthSecretStore for ProbedStore {
    fn save_session_token(&self, token: &SessionToken) -> AuthResult<()> {
        self.ask("save session-token", false)?;
        self.store.save_session_token(token)
    }
    fn load_session_token(&self) -> AuthResult<Option<SessionToken>> {
        self.store.load_session_token()
    }
    fn delete_session_token(&self) -> AuthResult<()> {
        self.ask("delete session-token", true)?;
        self.store.delete_session_token()
    }
    fn save_wrapped_master_key(&self, wrapped: SecretBytes) -> AuthResult<()> {
        self.ask("save vault-key", false)?;
        self.store.save_wrapped_master_key(wrapped)
    }
    fn load_wrapped_master_key(&self) -> AuthResult<Option<SecretBytes>> {
        self.store.load_wrapped_master_key()
    }
    fn delete_wrapped_master_key(&self) -> AuthResult<()> {
        self.ask("delete vault-key", true)?;
        self.store.delete_wrapped_master_key()
    }
    fn save_account_email(&self, email: &str) -> AuthResult<()> {
        self.ask("save account-email", false)?;
        self.store.save_account_email(email)
    }
    fn load_account_email(&self) -> AuthResult<Option<String>> {
        self.store.load_account_email()
    }
    fn delete_account_email(&self) -> AuthResult<()> {
        self.ask("delete account-email", true)?;
        self.store.delete_account_email()
    }
    fn save_vault_key_owner(&self, user_id: &str) -> AuthResult<()> {
        self.ask("save vault-key-owner", false)?;
        self.store.save_vault_key_owner(user_id)
    }
    fn load_vault_key_owner(&self) -> AuthResult<Option<String>> {
        self.store.load_vault_key_owner()
    }
    fn delete_vault_key_owner(&self) -> AuthResult<()> {
        self.ask("delete vault-key-owner", true)?;
        self.store.delete_vault_key_owner()
    }
    fn save_session_owner(&self, user_id: &str) -> AuthResult<()> {
        self.ask("save session-owner", false)?;
        self.store.save_session_owner(user_id)
    }
    fn load_session_owner(&self) -> AuthResult<Option<String>> {
        self.store.load_session_owner()
    }
    fn delete_session_owner(&self) -> AuthResult<()> {
        self.ask("delete session-owner", true)?;
        self.store.delete_session_owner()
    }
    fn save_lock_flag(&self, flag: LockFlag, bytes: &[u8]) -> AuthResult<()> {
        self.ask("save lock flag", false)?;
        self.store.save_lock_flag(flag, bytes)
    }
    fn load_lock_flag(&self, flag: LockFlag) -> AuthResult<Option<Vec<u8>>> {
        self.store.load_lock_flag(flag)
    }
    fn delete_lock_flag(&self, flag: LockFlag) -> AuthResult<()> {
        self.ask("delete lock flag", true)?;
        self.store.delete_lock_flag(flag)
    }
}

/// Row S1 (security). A move that failed before its token write keeps the flat trio; the person then signs in by
/// password (the slot gets a new token and its owner tag, no key) and quits before the twelve words. The next open
/// never replaces that sign-in with the flat session, and the flat trio is deleted. Mutant S1-M1:
/// `credential_stores.rs`, `must_not_move`'s last line `holds_native_sign_in(slot)` → `Ok(false)` (the move's
/// "partial write" path writes the flat session over the sign-in).
#[test]
fn a_native_sign_in_after_a_failed_move_is_never_replaced_by_the_flat_session() {
    const PASSWORD: &str = "an invented password of the native sign-in";
    const SIGNED_IN_EMAIL: &str = "native.sign-in@example.eu";
    let (account, _phrase) = FakeAccount::register(SIGNED_IN_EMAIL, PASSWORD, TEST_PHRASE);
    let api = FakeApi::start(account_handler(Arc::new(account)));
    let credentials = MemoryCredentials::default();
    store_session(&flat(&credentials), OLD_TOKEN, OLD_KEY, OLD_EMAIL);

    credentials.refuse_token_writes(Some("an invented refusal"));
    let engine = open_on(&credentials, &api.base);
    credentials.refuse_token_writes(None);
    assert_eq!(
        held(&slot(&credentials)),
        NOTHING,
        "the precondition: the move wrote nothing into the slot"
    );
    assert_eq!(
        held(&flat(&credentials)),
        session(OLD_TOKEN, OLD_KEY, OLD_EMAIL),
        "the precondition: the flat trio is kept"
    );

    assert_eq!(engine.restore(), GateState::SignedOut);
    engine
        .sign_in_password(SIGNED_IN_EMAIL, Zeroizing::new(PASSWORD.into()))
        .expect("the fake accepts the password");
    let signed_in = held(&slot(&credentials));
    let owner = slot(&credentials).load_session_owner().unwrap();
    assert!(
        signed_in.0.is_some() && signed_in.0.as_deref() != Some(OLD_TOKEN),
        "the precondition: the sign-in stored a token of its own"
    );
    assert_eq!(
        signed_in.1, None,
        "the precondition: no key yet (the twelve words were not given)"
    );
    assert!(
        owner.is_some(),
        "the precondition: the sign-in tagged its session's owner"
    );
    drop(engine);

    let _restarted = open_on(&credentials, &api.base);

    assert_eq!(held(&slot(&credentials)), signed_in, "the native sign-in is unchanged");
    assert_eq!(
        slot(&credentials).load_session_owner().unwrap(),
        owner,
        "its owner tag is unchanged"
    );
    assert_eq!(held(&flat(&credentials)), NOTHING, "the flat trio is gone");
}

/// Row S1, the second test: a flat trio beside a sign-in of this engine (a token with its owner tag and no key yet) is
/// deleted at open, and nothing is written into the slot. Mutant S1-M1, as above.
#[test]
fn a_flat_trio_beside_a_native_sign_in_is_deleted_and_nothing_is_written() {
    const OWNER: &str = "an-invented-user-id";
    let credentials = MemoryCredentials::default();
    let account = slot(&credentials);
    account
        .save_session_token(&SessionToken::new(NEW_TOKEN).expect("a token"))
        .unwrap();
    account.save_account_email(NEW_EMAIL).unwrap();
    account.save_session_owner(OWNER).unwrap();
    store_session(&flat(&credentials), OLD_TOKEN, OLD_KEY, OLD_EMAIL);
    let snapshot = |store: &MemoryStore| {
        (
            held(store),
            store.load_session_owner().unwrap(),
            store.load_vault_key_owner().unwrap(),
        )
    };
    let before = snapshot(&account);
    assert_eq!(
        before,
        (
            (Some(NEW_TOKEN.to_string()), None, Some(NEW_EMAIL.to_string())),
            Some(OWNER.to_string()),
            None
        ),
        "the control: what was seeded"
    );

    let _engine = opened(&credentials);

    assert_eq!(snapshot(&account), before, "the slot is unchanged");
    assert_eq!(held(&flat(&credentials)), NOTHING, "the flat trio is gone");
    assert_eq!(
        credentials.len(),
        3,
        "nothing else is stored: the slot's token, email and owner tag"
    );
}

/// Row S2 (decision D-B3-1, lead 2026-11-15 20:16). A sign-out whose clear of the flat layout fails still returns
/// `Ok`, and the next open does not bring the old session back. Mutant S2-M1: `credential_stores.rs`,
/// `clear_legacy_flat` without its `mark_retired` call (the next open moves the flat session in, signed in).
#[test]
fn a_failed_flat_clear_then_the_next_open_does_not_bring_the_old_session_back() {
    let credentials = FlatProbe::default();
    store_session(&credentials.slot_items(), NEW_TOKEN, NEW_KEY, NEW_EMAIL);
    let engine = open_on(&credentials, NO_SERVER);
    // After the open, as in rule 4: the open's own move sweeps flat items beside a complete slot.
    store_session(&credentials.flat_items(), OLD_TOKEN, OLD_KEY, OLD_EMAIL);
    assert_eq!(
        engine.restore(),
        GateState::ScreenLocked,
        "the precondition: signed in, so the sign-out reaches the clear"
    );

    credentials.refuse_flat_deletes(true);
    assert_eq!(
        engine.sign_out_keeping_local_data(),
        Ok(()),
        "a failed flat clear never fails the sign-out"
    );
    assert_eq!(
        held(&credentials.flat_items()),
        session(OLD_TOKEN, OLD_KEY, OLD_EMAIL),
        "the precondition: the flat clear failed"
    );
    drop(engine);
    credentials.refuse_flat_deletes(false);
    let restarted = open_on(&credentials, NO_SERVER);

    assert_eq!(
        restarted.restore(),
        GateState::SignedOut,
        "no old session is moved back in"
    );
    assert_eq!(held(&credentials.slot_items()), NOTHING, "the account's slot is empty");
    assert_eq!(held(&credentials.flat_items()), NOTHING, "the flat session was deleted");
    assert!(
        credentials.items.is_empty(),
        "{} items are left in the credential store",
        credentials.items.len()
    );
}

/// Row S2 (D-B3-1): the sign-out writes the retired mark before it tries the clear, and a clear that succeeds leaves
/// nothing behind (the mark goes once the flat items are gone). Mutant S2-M1, as above: the first thing asked of the
/// flat layout is a removal.
#[test]
fn the_retired_mark_is_written_before_the_flat_clear_is_tried() {
    let credentials = FlatProbe::default();
    store_session(&credentials.slot_items(), NEW_TOKEN, NEW_KEY, NEW_EMAIL);
    let engine = open_on(&credentials, NO_SERVER);
    store_session(&credentials.flat_items(), OLD_TOKEN, OLD_KEY, OLD_EMAIL);
    assert_eq!(engine.restore(), GateState::ScreenLocked, "the precondition: signed in");
    assert_eq!(
        credentials.asked(),
        Vec::<String>::new(),
        "the control: nothing was saved into or removed from the flat layout yet"
    );

    assert_eq!(engine.sign_out_keeping_local_data(), Ok(()));

    let asked = credentials.asked();
    assert_eq!(
        asked.first().map(String::as_str),
        Some("save session-owner"),
        "the mark goes in first: {asked:?}"
    );
    let marked = asked.iter().position(|step| step == "save session-owner");
    let cleared = asked.iter().position(|step| step == "delete session-token");
    assert!(
        marked.is_some() && cleared.is_some() && marked < cleared,
        "the mark before the clear: {asked:?}"
    );
    drop(engine);
    assert!(
        credentials.items.is_empty(),
        "{} items are left in the credential store",
        credentials.items.len()
    );
}

/// Row S2 (D-B3-1): a flat session found after the mark is deleted at open and never moved in. Mutant S2-M2:
/// `credential_stores.rs`, `must_not_move`'s `if is_retired(legacy)? {` → `if false {` (the flat session moves into
/// the empty slot, signed in).
#[test]
fn a_flat_trio_found_after_the_mark_is_deleted_not_adopted() {
    let credentials = MemoryCredentials::default();
    store_session(&flat(&credentials), OLD_TOKEN, OLD_KEY, OLD_EMAIL);
    flat(&credentials).save_session_owner(FLAT_LAYOUT_RETIRED).unwrap();

    let engine = opened(&credentials);

    assert_eq!(
        engine.restore(),
        GateState::SignedOut,
        "the flat session is not moved in"
    );
    assert_eq!(held(&slot(&credentials)), NOTHING, "nothing is written into the slot");
    assert_eq!(held(&flat(&credentials)), NOTHING, "the flat session is deleted");
    assert!(
        credentials.is_empty(),
        "{} items are left (the mark goes with the flat session)",
        credentials.len()
    );
}

/// R4 (014's real check): one kept key per ACCOUNT, not per device. A switch A -> B -> A never asks A for the twelve
/// words again; removing B leaves A's slot. Ids, tokens and keys are invented.
mod key_slot_per_account {
    use super::*;
    use crate::keychain::AuthVault;
    use crate::signin::key_owner::{self, KeyOwnership};

    const USER_A: &str = "0f6a3c1e-aaaa-4aaa-8aaa-000000000001";
    const USER_B: &str = "0f6a3c1e-bbbb-4bbb-8bbb-000000000002";
    const KEY_A: &[u8; 32] = b"an-invented-32-byte-vault-key-A1";
    const KEY_B: &[u8; 32] = b"an-invented-32-byte-vault-key-B2";

    fn vault(credentials: &MemoryCredentials) -> AuthVault<<MemoryCredentials as CredentialStores>::Store> {
        AuthVault::new(credentials.for_account(&TEST, SLOT))
    }

    /// A password sign-in as the app stores it (`Engine::store_first_sign_in`).
    fn sign_in(credentials: &MemoryCredentials, user: &str, email: &str) {
        let mut vault = vault(credentials);
        key_owner::remove_foreign_key(&mut vault, user, email).expect("no foreign key left");
        vault
            .install_session(SessionToken::new(format!("token-of-{user}")).unwrap())
            .unwrap();
        vault.store_account_email(email).unwrap();
        vault.bind_session_owner(user).unwrap();
    }

    /// The twelve words, proven (`Engine::unlock_with_phrase`).
    fn unlock_with_words(credentials: &MemoryCredentials, key: &[u8; 32]) {
        let vault = vault(credentials);
        vault.store_wrapped_master_key(SecretBytes::new_master_key(key)).unwrap();
        assert!(key_owner::bind_key_to_session_owner(&vault).unwrap());
    }

    fn sign_out(credentials: &MemoryCredentials) {
        vault(credentials).clear_session().unwrap();
    }

    /// Does the signed-in account unlock without the words, and with which key?
    fn unlocks_without_words(credentials: &MemoryCredentials) -> Option<Vec<u8>> {
        let mut vault = vault(credentials);
        if key_owner::stored_key_state(&vault) != key_owner::StoredKey::Usable {
            return None;
        }
        assert_eq!(key_owner::stored_key_ownership(&vault).unwrap(), KeyOwnership::Matches);
        vault.unlock().ok()?;
        Some(vault.master_key().unwrap().to_vec())
    }

    fn kept(credentials: &MemoryCredentials, user: &str) -> Option<Vec<u8>> {
        credentials
            .for_account(&TEST, SLOT)
            .load_account_key(user)
            .unwrap()
            .map(|key| key.expose_for_crypto().to_vec())
    }

    #[test]
    fn a_switch_back_needs_no_twelve_words_and_signing_out_of_b_leaves_a() {
        let credentials = MemoryCredentials::default();
        sign_in(&credentials, USER_A, "a@example.eu");
        unlock_with_words(&credentials, KEY_A);
        assert_eq!(unlocks_without_words(&credentials).as_deref(), Some(&KEY_A[..]));

        // Switch to B: A is not signed out (Guus, 2026-11-21), so nothing of A is forgotten.
        sign_in(&credentials, USER_B, "b@example.eu");
        assert_eq!(unlocks_without_words(&credentials), None, "B never gets A's key");
        unlock_with_words(&credentials, KEY_B);
        assert_eq!(unlocks_without_words(&credentials).as_deref(), Some(&KEY_B[..]));

        // And back to A: no twelve words.
        sign_in(&credentials, USER_A, "a@example.eu");
        assert_eq!(unlocks_without_words(&credentials).as_deref(), Some(&KEY_A[..]));

        // Both slots exist, each with its own key.
        assert_eq!(kept(&credentials, USER_A).as_deref(), Some(&KEY_A[..]));
        assert_eq!(kept(&credentials, USER_B).as_deref(), Some(&KEY_B[..]));

        // Removing B removes only B's slot.
        vault(&credentials).forget_account_key(USER_B).unwrap();
        assert_eq!(kept(&credentials, USER_B), None);
        assert_eq!(kept(&credentials, USER_A).as_deref(), Some(&KEY_A[..]));
        assert_eq!(unlocks_without_words(&credentials).as_deref(), Some(&KEY_A[..]));
    }

    /// A sign-out leaves nothing of the account signing out (task 2447 d), and keeps the slot of an account that was
    /// only switched away from.
    #[test]
    fn signing_out_removes_that_accounts_slot_only() {
        let credentials = MemoryCredentials::default();
        sign_in(&credentials, USER_A, "a@example.eu");
        unlock_with_words(&credentials, KEY_A);
        sign_in(&credentials, USER_B, "b@example.eu");
        unlock_with_words(&credentials, KEY_B);

        sign_out(&credentials);
        assert_eq!(kept(&credentials, USER_B), None, "B signed out: its key is gone");
        assert_eq!(kept(&credentials, USER_A).as_deref(), Some(&KEY_A[..]), "A was switched away from");
        let install = credentials.store_for(SLOT);
        assert!(install.load_wrapped_master_key().unwrap().is_none());
        assert!(install.load_session_token().unwrap().is_none());

        sign_in(&credentials, USER_B, "b@example.eu");
        assert_eq!(unlocks_without_words(&credentials), None, "B asks for its words again");
        sign_in(&credentials, USER_A, "a@example.eu");
        assert_eq!(unlocks_without_words(&credentials).as_deref(), Some(&KEY_A[..]));

        // A signs out too: nothing of either account is left.
        sign_out(&credentials);
        assert!(credentials.is_empty(), "a sign-out leaves nothing");
    }

    /// dc-014's case, with A's working key left behind by B's sign-out: A's key stays A's (it was not A signing out).
    #[test]
    fn a_sign_out_of_b_beside_a_working_key_of_a_keeps_it_in_as_slot() {
        let credentials = MemoryCredentials::default();
        sign_in(&credentials, USER_A, "a@example.eu");
        unlock_with_words(&credentials, KEY_A);
        sign_in(&credentials, USER_B, "b@example.eu");
        sign_out(&credentials);
        assert_eq!(kept(&credentials, USER_A).as_deref(), Some(&KEY_A[..]));
        sign_in(&credentials, USER_A, "a@example.eu");
        assert_eq!(unlocks_without_words(&credentials).as_deref(), Some(&KEY_A[..]));
    }

    /// dc-014's case: B signs in while A's tagged key is still the working key (A's clear failed, or a store that
    /// keeps it). B's unlock never takes A's only copy; A comes back to its own key.
    #[test]
    fn b_unlocking_over_a_working_key_of_a_keeps_a() {
        let credentials = MemoryCredentials::default();
        // An earlier build: one slot, A's key tagged A, no account slots yet.
        let store = credentials.store_for(SLOT);
        store.save_wrapped_master_key(SecretBytes::new_master_key(KEY_A)).unwrap();
        store.save_vault_key_owner(USER_A).unwrap();
        assert_eq!(kept(&credentials, USER_A), None);

        sign_in(&credentials, USER_B, "b@example.eu");
        assert_eq!(unlocks_without_words(&credentials), None, "B never gets A's key");
        unlock_with_words(&credentials, KEY_B);
        assert_eq!(kept(&credentials, USER_A).as_deref(), Some(&KEY_A[..]), "A's key moved into A's slot");

        sign_in(&credentials, USER_A, "a@example.eu");
        assert_eq!(unlocks_without_words(&credentials).as_deref(), Some(&KEY_A[..]));
        assert_eq!(kept(&credentials, USER_B).as_deref(), Some(&KEY_B[..]));
    }

    /// The migration: the single slot of an earlier build moves into the signed-in account's slot at its first run.
    #[test]
    fn the_single_slot_moves_into_the_signed_in_accounts_slot_on_first_run() {
        let credentials = MemoryCredentials::default();
        let store = credentials.store_for(SLOT);
        store
            .save_session_token(&SessionToken::new("token-of-a").unwrap())
            .unwrap();
        store.save_session_owner(USER_A).unwrap();
        store.save_wrapped_master_key(SecretBytes::new_master_key(KEY_A)).unwrap();
        store.save_vault_key_owner(USER_A).unwrap();

        assert_eq!(unlocks_without_words(&credentials).as_deref(), Some(&KEY_A[..]));
        assert_eq!(kept(&credentials, USER_A).as_deref(), Some(&KEY_A[..]));
        assert_eq!(kept(&credentials, USER_B), None);
    }

    /// An untagged working key is never replaced by a kept one (it may be this account's key, written a moment ago
    /// for this session), and a kept key never reaches another account's session.
    #[test]
    fn a_kept_key_never_replaces_an_untagged_key_nor_reaches_another_account() {
        let credentials = MemoryCredentials::default();
        sign_in(&credentials, USER_A, "a@example.eu");
        unlock_with_words(&credentials, KEY_A);

        // A device approval of B (a switch: A stays signed in on this PC): token and key, untagged until bound.
        let vault = vault(&credentials);
        vault
            .install_session(SessionToken::new("token-of-b").unwrap())
            .unwrap();
        vault.store_wrapped_master_key(SecretBytes::new_master_key(KEY_B)).unwrap();
        vault.bind_session_owner(USER_B).unwrap();
        assert_eq!(
            credentials.store_for(SLOT).load_wrapped_master_key().unwrap().unwrap().expose_for_crypto(),
            &KEY_B[..]
        );
        vault.bind_vault_key_owner(USER_B).unwrap();
        assert_eq!(unlocks_without_words(&credentials).as_deref(), Some(&KEY_B[..]));
        assert_eq!(kept(&credentials, USER_B).as_deref(), Some(&KEY_B[..]));
    }

    #[test]
    fn an_id_that_is_not_a_server_user_id_names_no_slot() {
        assert!(super::super::account_key_slot_name("").is_err());
        assert!(super::super::account_key_slot_name("a/b").is_err());
        assert!(super::super::account_key_slot_name("a@example.eu").is_err());
        assert_eq!(super::super::account_key_slot_name(USER_A).unwrap(), format!("account-key-{USER_A}"));
    }
}
