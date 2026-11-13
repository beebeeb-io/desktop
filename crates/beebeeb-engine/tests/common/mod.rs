//! The shared setup of the sync-start tests (contract `docs/superpowers/specs/2026-11-12-native-sync-start-and-sign-out.md`,
//! §8 "Fake-API tests, isolation"). Each test binary is its own process: its own state folder (a process-wide
//! global), its own settings sandbox, its own credential stores in memory and its own fake API on loopback. A count
//! of requests, "no `state.db`" or "the folder is unchanged" is that test's alone.
//!
//! Everything is invented: the account, its password and its 12 words (`test_api::fake_account::test_account`), the
//! folders. Nothing reaches the OS credential store or the network.
//!
//! The test build is refused on macOS (`EngineIdentity::refusal_on`), so a binary's scenario runs on Linux and
//! Windows and its macOS test asserts that refusal: every binary executes exactly one test on every OS.

#![allow(dead_code)] // each test binary uses its own part of this

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use beebeeb_engine::app::{Engine, EngineConfig};
use beebeeb_engine::config::DesktopConfig;
use beebeeb_engine::host::{ChannelHost, HostMessage};
use beebeeb_engine::identity::TEST;
use beebeeb_engine::keychain::AuthSecretStore;
use beebeeb_engine::signin::gate::NextAfterSignIn;
use beebeeb_engine::signin::memory_store::MemoryCredentials;
use beebeeb_engine::state_db::StateDb;
use beebeeb_engine::state_paths;
use beebeeb_engine::sync_root_owner::Registration;
use beebeeb_engine::sync_root_port::SyncRootPort;
use beebeeb_engine::test_api::fake_account::{FakeAccount, account_handler, test_account};
use beebeeb_engine::test_api::{FakeApi, Handler};
use zeroize::Zeroizing;

/// The invented account's password (`test_account` registers it).
pub const PASSWORD: &str = "correct horse battery";
/// A valid 12-word phrase (BIP-39's all-ones test vector) that is not the account's: it derives a key, and only
/// the server's check can say it is the wrong one.
pub const WRONG_PHRASE: &str = "zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo wrong";
/// The 5 entries of an account's credential slot.
pub const ENTRIES: [&str; 5] = [
    "session token",
    "vault key",
    "account email",
    "vault key owner",
    "session owner",
];

pub struct Setup {
    pub account: Arc<FakeAccount>,
    pub phrase: String,
    pub fake: FakeApi,
    pub state_dir: PathBuf,
    /// A folder of this test's own, outside the state folder: sync folders go here.
    pub home: PathBuf,
    pub stores: MemoryCredentials,
    pub account_id: String,
    pub engine: Arc<Engine<MemoryCredentials>>,
    pub messages: std::sync::mpsc::Receiver<HostMessage>,
}

/// The engine of this process on the fake account, restored (signed out).
pub fn setup() -> Setup {
    setup_with(|handler, _, _| handler)
}

/// [`setup`] with the fake's handler from `wrap`: it gets the account's own handler, the credential stores the engine
/// uses and the account slot id, and returns the handler the fake serves.
pub fn setup_with(wrap: impl FnOnce(Handler, MemoryCredentials, String) -> Handler) -> Setup {
    let (account, phrase, first) = test_account();
    drop(first);
    let home = tempfile::tempdir().expect("a temp folder").keep();
    state_paths::init_from_app_local_data_dir(&TEST, &home.join("local")).expect("the state folder");
    let state_dir = state_paths::beebeeb_state_dir_for(&TEST).expect("the state folder");
    let account_id = DesktopConfig::ensure_account_id_for(&TEST).expect("the sandboxed settings file");
    let stores = MemoryCredentials::default();
    let fake = FakeApi::start(wrap(
        account_handler(account.clone()),
        stores.clone(),
        account_id.clone(),
    ));
    let (host, messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: account_id.clone(),
        state_dir: state_dir.clone(),
        api_base: fake.base.clone(),
    };
    let engine = Arc::new(Engine::open(&TEST, config, host, stores.clone()).expect("the engine opens"));
    engine.restore();
    Setup {
        account,
        phrase,
        fake,
        state_dir,
        home,
        stores,
        account_id,
        engine,
        messages,
    }
}

impl Setup {
    /// Signed in with the password: this PC holds no key yet, so the phrase is next.
    pub fn sign_in(&self) {
        let next = self
            .engine
            .sign_in_password(&self.account.email, Zeroizing::new(PASSWORD.into()))
            .expect("the password signs in");
        assert_eq!(next, NextAfterSignIn::RecoveryPhrase);
    }

    /// The account's 12 words: the gate is Ready.
    pub fn unlock(&self) {
        self.engine
            .unlock_with_phrase(Zeroizing::new(self.phrase.clone()))
            .expect("the account's phrase opens the vault");
    }

    /// Which of the slot's 5 entries are stored now.
    pub fn entries(&self) -> Vec<&'static str> {
        let store = self.stores.store_for(&self.account_id);
        let stored = [
            store.load_session_token().ok().flatten().is_some(),
            store.load_wrapped_master_key().ok().flatten().is_some(),
            store.load_account_email().ok().flatten().is_some(),
            store.load_vault_key_owner().ok().flatten().is_some(),
            store.load_session_owner().ok().flatten().is_some(),
        ];
        ENTRIES
            .iter()
            .zip(stored)
            .filter_map(|(name, there)| there.then_some(*name))
            .collect()
    }

    /// The routes the fake answered from request `from` on.
    pub fn routes_from(&self, from: usize) -> Vec<String> {
        self.fake.routes().split_off(from.min(self.fake.routes().len()))
    }

    pub fn state_db(&self) -> PathBuf {
        state_paths::state_db_path_from_state_dir(&self.state_dir)
    }

    /// The state folder's `state.db`, made with the engine's own writers, holding one row and owned by `owner`.
    pub fn state_db_with_one_row(&self, owner: &beebeeb_engine::account_binding::Identity) {
        std::fs::create_dir_all(&self.state_dir).unwrap();
        let db = StateDb::open(self.state_db()).unwrap();
        db.set_owner(owner).unwrap();
        let entry = beebeeb_engine::state_db::FileEntry {
            file_id: "f0000000-0000-4000-8000-000000000001".into(),
            path: "/notes.txt".into(),
            status: beebeeb_engine::state_db::FileStatus::CloudOnly,
            size_bytes: 1_200,
            modified_at: 1_794_000_000,
            content_hash: None,
            remote_updated_at: 1_794_000_000,
            parent_id: None,
            item_kind: beebeeb_engine::state_db::ItemKind::File,
        };
        db.upsert_file_with_contract(&entry, |contract| contract.item_kind = entry.item_kind.clone())
            .unwrap();
        assert!(db.has_account_data().unwrap(), "the row counts as the account's data");
    }

    /// The account this test signs in to, as the binding records an owner.
    pub fn this_account(&self) -> beebeeb_engine::account_binding::Identity {
        beebeeb_engine::account_binding::Identity::new(Some(&self.account.user_id), Some(&self.account.email))
    }
}

/// Every file and folder under `root` with its size, sorted: "the folder is unchanged" is this, before = after.
pub fn listing(root: &Path) -> Vec<(String, u64)> {
    let mut found = Vec::new();
    let mut todo = vec![root.to_path_buf()];
    while let Some(dir) = todo.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            found.push((path.strip_prefix(root).unwrap().to_string_lossy().into_owned(), size));
            if path.is_dir() {
                todo.push(path);
            }
        }
    }
    found.sort();
    found
}

/// Waits until `state.db` lists at least `rows` files, at most 30 s. Returns the count it saw last.
pub fn wait_for_rows(state_db: &Path, rows: usize) -> usize {
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut seen = 0;
    while Instant::now() < deadline {
        if let Ok(db) = StateDb::open(state_db)
            && let Ok(files) = db.list_files()
        {
            seen = files.len();
            if seen >= rows {
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    seen
}

/// Waits until `done` holds, at most `seconds`. Returns whether it did.
pub fn wait_until(seconds: u64, mut done: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    while Instant::now() < deadline {
        if done() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    done()
}

/// The OS's three answers as a test makes them, and how often the start asked to register.
pub struct Port {
    pub registration: Registration,
    pub register: Result<(), Option<i32>>,
    pub lock_held: bool,
    pub register_calls: std::sync::atomic::AtomicU32,
}

impl Port {
    /// Nobody's folder, registration succeeds, no lock: a start may go on.
    pub fn free() -> Port {
        Port {
            registration: Registration::Nobody,
            register: Ok(()),
            lock_held: false,
            register_calls: Default::default(),
        }
    }

    pub fn registers(&self) -> u32 {
        self.register_calls.load(std::sync::atomic::Ordering::SeqCst)
    }
}

impl SyncRootPort for Port {
    fn registration(&self, _folder: &Path) -> Registration {
        self.registration.clone()
    }
    fn register(&self, _folder: &Path) -> Result<(), Option<i32>> {
        self.register_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.register
    }
    fn engine_lock_held(&self, _state_dir: &Path) -> bool {
        self.lock_held
    }
}

/// macOS: the test build's engine is refused before it opens (it would share the real app's data there).
pub fn assert_the_test_build_is_refused() {
    let home = tempfile::tempdir().expect("a temp folder").keep();
    state_paths::init_from_app_local_data_dir(&TEST, &home).expect("the state folder");
    let config = EngineConfig {
        account_id: DesktopConfig::ensure_account_id_for(&TEST).expect("the sandboxed settings file"),
        state_dir: state_paths::beebeeb_state_dir_for(&TEST).expect("the state folder"),
        api_base: "http://127.0.0.1:9".into(),
    };
    let (host, _messages) = ChannelHost::new(|| {});
    let refused = Engine::open(&TEST, config, host, MemoryCredentials::default()).err();
    assert!(
        refused.as_deref().is_some_and(|words| words.contains("macOS")),
        "the test build must be refused on macOS: {refused:?}"
    );
}
