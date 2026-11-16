//! Rule 5 of the flat-layout slice (`app::credential_stores`): the flat layout of a 0.8.x PC holds ANOTHER account's
//! session, beside local data recorded as this PC's account's. The session moves into the account's slot at open and
//! opens, and the sync start is refused by the binding (R10: `SyncStartRefused::OtherAccountsData`, a reset is never
//! allowed). The owner record stays, the local data and the folder stay as they were, the fake gets no request, and no
//! line the engine logs holds the session's token, key or email. Its own process: the state folder is process-wide.
//! Mutant: `account_binding::decide` proceeds for another account (`Some(false) => Binding::Proceed`): the start goes
//! on and records the other account as the owner.

mod common;

#[cfg(not(target_os = "macos"))]
#[test]
fn a_flat_session_of_another_account_never_takes_the_local_data() {
    use std::sync::{Arc, Mutex};

    use base64::Engine as _;
    use beebeeb_engine::account_binding::Identity;
    use beebeeb_engine::app::{CredentialStores, Engine, EngineConfig, SyncStartRefused};
    use beebeeb_engine::host::ChannelHost;
    use beebeeb_engine::identity::TEST;
    use beebeeb_engine::keychain::{AuthSecretStore, SecretBytes, SessionToken};
    use beebeeb_engine::signin::gate::GateState;
    use beebeeb_engine::state_db::StateDb;

    /// Every line the engine logs while `body` runs on this thread (as `app/credential_stores_tests.rs`).
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
        // A second live dispatcher keeps the callsites' interest on the all-dispatchers path (see the unit test's helper).
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

    const TOKEN: &str = "an-invented-flat-token-of-another-account";
    const KEY: &[u8; 32] = b"an-invented-32-byte-vault-key-03";
    const EMAIL: &str = "another.account@example.eu";

    // The engine `setup` opened stays signed out and idle: the handle under test is opened below, after the seeds.
    let s = common::setup();
    let owner = Identity::new(Some("u-invented-owner"), Some("owner@example.eu"));
    s.state_db_with_one_row(&owner);
    let flat = s.stores.legacy_flat(&TEST).expect("the test stores have a flat layout");
    flat.save_session_token(&SessionToken::new(TOKEN).expect("a token"))
        .unwrap();
    flat.save_wrapped_master_key(SecretBytes::new_master_key(KEY)).unwrap();
    flat.save_account_email(EMAIL).unwrap();
    // The folder is there (a missing one with account data is refused before the binding is asked).
    let folder = s.home.join("Beebeeb Test");
    std::fs::create_dir_all(folder.join("Plans")).unwrap();
    std::fs::write(folder.join("Plans").join("budget.ods"), b"twelve bytes").unwrap();
    let before = common::listing(&folder);
    let from = s.fake.requests().len();

    let ((gate, start), logs) = capture_logs(|| {
        let (host, _messages) = ChannelHost::new(|| {});
        let config = EngineConfig {
            account_id: s.account_id.clone(),
            state_dir: s.state_dir.clone(),
            api_base: s.fake.base.clone(),
        };
        let engine = Engine::open(&TEST, config, host, s.stores.clone()).expect("the engine opens");
        let gate = engine.restore();
        let start = engine.start_sync_with(folder.clone(), &common::Port::free());
        (gate, start)
    });

    assert_eq!(
        gate,
        GateState::ScreenLocked,
        "the precondition: the flat session moved into the slot and opened with its key"
    );
    assert_eq!(start, Err(SyncStartRefused::OtherAccountsData));
    let db = StateDb::open(s.state_db()).unwrap();
    assert_eq!(db.owner().unwrap(), Some(owner), "the owner record is unchanged");
    assert_eq!(db.list_files().unwrap().len(), 1, "the local data is kept");
    assert_eq!(common::listing(&folder), before, "the folder is unchanged");
    assert_eq!(s.routes_from(from), Vec::<String>::new(), "0 requests to the fake");
    assert!(
        logs.contains("sync did not start"),
        "the control: the refused start was logged:\n{logs}"
    );
    let hex: String = KEY.iter().map(|byte| format!("{byte:02x}")).collect();
    let secrets = [
        TOKEN.to_string(),
        EMAIL.to_string(),
        String::from_utf8(KEY.to_vec()).unwrap(),
        hex.clone(),
        hex.to_uppercase(),
        base64::engine::general_purpose::STANDARD_NO_PAD.encode(KEY),
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(KEY),
    ];
    for secret in secrets {
        assert!(!logs.contains(&secret), "the log holds {secret:?}");
    }
}

#[cfg(target_os = "macos")]
#[test]
fn the_test_build_is_refused_on_macos() {
    common::assert_the_test_build_is_refused();
}
