//! S22a, the in-place upgrade (R10, lead ruling 8): "an install from before the binding keeps its queue. The account
//! whose vault key this computer's Keychain holds is recorded as the owner at startup, and the same account's first
//! engine start then keeps everything." The engine's own unit tests pin the adoption TABLE
//! (`account_binding::tests::adoption_happens_once_and_never_replaces_an_owner`); these tests go through the start a
//! person makes: `Engine::open` → `restore` → `start_sync`, on a private copy of a `state.db` that desktop 0.8.25
//! itself wrote (`tests/fixtures/state-db-0.8.25/closed`). The Tauri tests whose meaning this ports
//! (`src-tauri/src/account_binding_tests/part_3.rs`) ran on macOS, where another account's start PURGES; on Windows
//! the same rule reads: another account is REFUSED, and the queue and the files stay, for their owner.
//!
//! The premise, made in the copy (the committed files are never opened): the 0.8.25 image has no owner record (the
//! release had none) and its account rows (2 file rows, 1 staged payload), but no queued operation. One due
//! operation is added through the engine's own writer (`StateDb::enqueue_operation`), and the sync folder's local
//! items (`Documents/` with the file the image names) are made by the test. That first open of the copy is the
//! test's own `StateDb::open`, so the engine's start finds it migrated; the release's raw first open is
//! `engine_bridge::tests::upgrade_from_0_8_25`'s.
//!
//! The credential store, as 0.8.25 left it: the engine's own sign-in on the fake account writes it (the password,
//! and the 12 words where a test names a vault key), on a first handle that is then dropped. The upgraded app is a
//! new handle on the same store and the same state folder. The fake API is HTTP on loopback; nothing reaches the
//! network or the OS credential store.
//!
//! Each test runs in a process of its own (`in_own_process`): the state folder is a process-wide global
//! (`state_paths`), set once per process, and `tests/common` makes a new one per setup.
//!
//! Mutants: M1 no candidate at `account_binding::adopt_unbound`; M2' a Windows start that may reset (the start's
//! bind with `reset_allowed` true AND `LocalState::reset` answering `Ok(())`); M3 `close_adoption_window` a no-op;
//! M5 the bind after the registration.

mod common;

// The adoption (`app/own_data_start.rs`): the candidate is whose vault key the store holds at the first native
// startup, the window shuts at that startup, and the start adopts for that candidate. Here the store is written by a
// native handle that restores BEFORE the 0.8.25 `state.db` is put in place, so the first startup that finds the
// release's `state.db` is the upgraded app's.

/// Rule 1: the store holds A's complete session and the 0.8.25 data has no owner. A's first start keeps
/// everything: `Started`, A recorded as the owner, the due operation still queued, every local file still there.
/// Then the adoption is over: a later one adopts nobody.
/// Mutant M1 (no candidate at `account_binding.rs:225`): owner `None`, the start refused.
#[cfg(not(target_os = "macos"))]
#[test]
fn an_upgraded_pc_keeps_its_queue_for_the_account_whose_session_it_held() {
    in_own_process(
        "an_upgraded_pc_keeps_its_queue_for_the_account_whose_session_it_held",
        || {
            let mut pc = pc::Pc::signed_in_before_the_upgrade(None, true);
            let files = pc.put_the_0_8_25_image();
            assert_eq!(
                pc.open_and_restore(),
                beebeeb_engine::signin::gate::GateState::ScreenLocked,
                "the premise: the stored key is A's and is in memory"
            );
            pc.unlock_the_screen();
            pc.keeps_everything_at_its_first_start(&files);
            let db = pc.db();
            let other = beebeeb_engine::account_binding::Identity::new(
                Some("99999999-9999-4999-8999-999999999999"),
                Some("other@example.eu"),
            );
            assert!(
                !db.adopt_owner_once(Some(&other)).unwrap(),
                "the adoption happened once: a later one adopts nobody"
            );
            assert_eq!(
                db.owner().unwrap(),
                Some(pc.this_account()),
                "and never replaces the owner"
            );
        },
    );
}

/// Rule 2: after rule 1's adoption, A signs out keeping the local data (the Windows sign-out), B signs in and
/// unlocks. B's start is refused with `OtherAccountsData`; the owner, the queue and the files are A's as they were.
/// A comes back: `Started`, the queue intact.
/// Mutant M2' (a Windows start that may reset: `reset_allowed` true at the start's bind AND `LocalState::reset`
/// `Ok(())`): B's start runs and B becomes the owner.
#[cfg(not(target_os = "macos"))]
#[test]
fn another_account_on_an_upgraded_pc_is_refused_and_takes_nothing() {
    in_own_process("another_account_on_an_upgraded_pc_is_refused_and_takes_nothing", || {
        use beebeeb_engine::app::{SyncStart, SyncStartRefused};
        use beebeeb_engine::signin::gate::{GateState, NextAfterSignIn};
        use beebeeb_engine::test_api::fake_account::FakeAccount;
        use std::sync::Arc;
        use zeroize::Zeroizing;

        let (second, _) = FakeAccount::register("second@example.eu", common::PASSWORD, pc::SECOND_PHRASE);
        let second = Arc::new(second);
        let mut pc = pc::Pc::signed_in_before_the_upgrade(Some(second.clone()), true);
        let files = pc.put_the_0_8_25_image();
        assert_eq!(
            pc.open_and_restore(),
            GateState::ScreenLocked,
            "the premise: A's key is in memory"
        );
        pc.unlock_the_screen();
        pc.keeps_everything_at_its_first_start(&files);

        // A signs out, keeping the local data; B signs in and unlocks on the same computer.
        pc.engine().sign_out_keeping_local_data().expect("A's sign-out");
        assert_eq!(
            pc.engine()
                .sign_in_password(&second.email, Zeroizing::new(common::PASSWORD.into())),
            Ok(NextAfterSignIn::RecoveryPhrase)
        );
        pc.engine()
            .unlock_with_phrase(Zeroizing::new(pc::SECOND_PHRASE.into()))
            .expect("B's phrase");
        assert_eq!(
            pc.engine().start_sync(pc.folder()),
            Err(SyncStartRefused::OtherAccountsData),
            "B is refused"
        );
        assert!(!pc.engine().is_syncing(), "no engine for B");
        assert_eq!(pc.owner(), Some(pc.this_account()), "the owner is still A");
        assert_eq!(pc.due_operations(), 1, "A's queue is unchanged");
        pc.files_are_still_there(&files);

        // B leaves; A comes back and keeps everything.
        pc.engine().sign_out_keeping_local_data().expect("B's sign-out");
        pc.sign_in_and_unlock();
        assert_eq!(
            pc.engine().start_sync(pc.folder()),
            Ok(SyncStart::Started),
            "A starts again"
        );
        let _ = pc.engine().stop();
        assert_eq!(pc.owner(), Some(pc.this_account()));
        assert_eq!(pc.due_operations(), 1, "the queue is intact");
        pc.files_are_still_there(&files);
    });
}

/// Rule 3: the store holds A's token and email but NO vault key (a password sign-in that never got the 12 words).
/// The first start has nothing in memory: `Ok(NoSession)`, the engine's rule (contract §6, I6), and nobody adopted.
/// Then the person enters the 12 words (the store is complete), and the next startup's start still adopts nobody:
/// on Windows it is refused with `OtherAccountsData`. The adoption window is shut by then: an adoption asked after it
/// adopts nobody.
/// Mutant M3 (`close_adoption_window` a no-op): the window is still open after the second start.
/// Mutant M2' (see rule 2): the second start resets the unowned data and records A.
/// Mutants MU1 (`fix_upgrade_candidate_at_startup` returns at once) and MU2 (no `user_version` mark at the startup's
/// shut): the window is still open after the first startup, before any start.
#[cfg(not(target_os = "macos"))]
#[test]
fn no_vault_key_no_adoption_and_the_window_shuts_after_the_first_start() {
    in_own_process(
        "no_vault_key_no_adoption_and_the_window_shuts_after_the_first_start",
        || {
            use beebeeb_engine::app::{SyncStart, SyncStartRefused};
            use beebeeb_engine::signin::gate::GateState;
            use zeroize::Zeroizing;

            let mut pc = pc::Pc::signed_in_before_the_upgrade(None, false);
            let files = pc.put_the_0_8_25_image();
            assert_eq!(
                pc.open_and_restore(),
                GateState::NeedsRecoveryPhrase,
                "the premise: signed in, no vault key on this PC"
            );
            // The FIRST startup shut the window, before any start (A2's mutant MU1, the startup step skipped, leaves it
            // open here; the start's own close would hide that at the end).
            {
                let db = pc.db();
                assert!(
                    !db.adopt_owner_once(Some(&pc.this_account())).unwrap(),
                    "the first startup shut the adoption window: an adoption asked before any start adopts nobody"
                );
                assert_eq!(db.owner().unwrap(), None);
            }
            assert_eq!(
                pc.engine().start_sync(pc.folder()),
                Ok(SyncStart::NoSession),
                "no keys in memory: nothing starts"
            );
            assert!(!pc.engine().is_syncing());
            assert_eq!(pc.owner(), None, "nobody adopted at the first start");

            // The 12 words complete the store.
            pc.engine()
                .unlock_with_phrase(Zeroizing::new(pc.phrase.clone()))
                .expect("the account's phrase");
            assert!(pc.store_holds_a_vault_key(), "the store is complete now");

            // The next startup, with the complete store.
            assert_eq!(
                pc.open_and_restore(),
                GateState::ScreenLocked,
                "the stored key is in memory"
            );
            assert_eq!(
                pc.engine().start_sync(pc.folder()),
                Err(SyncStartRefused::OtherAccountsData),
                "unowned data that was never adopted is refused on Windows"
            );
            assert!(!pc.engine().is_syncing(), "no engine");
            assert_eq!(pc.owner(), None, "a key that came after the upgrade adopts nobody");
            assert_eq!(pc.due_operations(), 1, "nothing reset");
            pc.files_are_still_there(&files);
            let db = pc.db();
            assert!(
                !db.adopt_owner_once(Some(&pc.this_account())).unwrap(),
                "the adoption window is shut: an adoption asked now adopts nobody"
            );
            assert_eq!(db.owner().unwrap(), None);
        },
    );
}

/// Rule 4: the store holds a token and a key but NO email and no owner tags (a session stored before the email was
/// kept), and the new handle has no cached profile. Open + restore + start: the start is refused with
/// `IdentityUnknown`, nothing is adopted, nothing is purged, no engine exists. An unknown identity stays refused with
/// `IdentityUnknown`: it is never compared with, adopted into or reset against anything.
#[cfg(not(target_os = "macos"))]
#[test]
fn an_unidentified_session_adopts_nothing() {
    in_own_process("an_unidentified_session_adopts_nothing", || {
        use beebeeb_engine::account_binding::Identity;
        use beebeeb_engine::app::SyncStartRefused;

        let mut pc = pc::Pc::signed_in_before_the_upgrade(None, true);
        pc.forget_whose_session_it_is();
        let files = pc.put_the_0_8_25_image();
        let rows = pc.rows();
        pc.open_and_restore();
        assert_eq!(
            pc.engine().keys_for_engine_start(),
            Ok(Identity::default()),
            "the premise: keys in memory, and nobody named for them"
        );
        assert_eq!(
            pc.engine().start_sync(pc.folder()),
            Err(SyncStartRefused::IdentityUnknown)
        );
        assert!(!pc.engine().is_syncing(), "no engine");
        assert_eq!(pc.owner(), None, "nobody adopted");
        assert_eq!(pc.rows(), rows, "nothing purged");
        assert_eq!(pc.due_operations(), 1, "the queue is untouched");
        pc.files_are_still_there(&files);
    });
}

/// Rule 5: the adoption happens INSIDE the start, after the keys were read and before any engine exists. A start
/// whose folder registration fails (after the bind, step 9) refuses with Windows' code and makes no engine, and the
/// owner is recorded all the same.
/// Mutant M5 (the bind after `port.register`): the failed registration comes first and nobody is recorded.
#[cfg(not(target_os = "macos"))]
#[test]
fn the_adoption_happens_in_the_start_before_any_engine() {
    in_own_process("the_adoption_happens_in_the_start_before_any_engine", || {
        use beebeeb_engine::app::SyncStartRefused;
        use beebeeb_engine::signin::gate::GateState;

        /// `HRESULT_FROM_WIN32(ERROR_CLOUD_FILE_INVALID_REQUEST)`, an invented refusal.
        const CODE: i32 = 0x8007_017C_u32 as i32;
        let mut pc = pc::Pc::signed_in_before_the_upgrade(None, true);
        let files = pc.put_the_0_8_25_image();
        assert_eq!(
            pc.open_and_restore(),
            GateState::ScreenLocked,
            "the premise: A's key is in memory"
        );
        assert_eq!(pc.owner(), None, "nobody before the start");
        let port = common::Port {
            register: Err(Some(CODE)),
            ..common::Port::free()
        };

        let refused = pc.engine().start_sync_with(pc.folder(), &port);

        assert_eq!(refused, Err(SyncStartRefused::RegistrationFailed { code: Some(CODE) }));
        assert_eq!(port.registers(), 1, "the start reached the registration");
        assert!(!pc.engine().is_syncing(), "no engine");
        assert_eq!(
            pc.owner(),
            Some(pc.this_account()),
            "recorded in the start, before the registration and any engine"
        );
        assert_eq!(pc.due_operations(), 1);
        pc.files_are_still_there(&files);
    });
}

#[cfg(target_os = "macos")]
#[test]
fn the_test_build_is_refused_on_macos() {
    common::assert_the_test_build_is_refused();
}

/// Runs `scenario` in a process of its own: this test binary again, filtered to the one test `name`, with
/// [`SCENARIO`] naming it. The child must report exactly one test passed (a filter that matched nothing is a red).
#[cfg(not(target_os = "macos"))]
fn in_own_process(name: &str, scenario: impl FnOnce()) {
    /// Set in the child: the scenario it runs.
    const SCENARIO: &str = "BB_UPGRADE_08X_SCENARIO";
    if std::env::var(SCENARIO).ok().as_deref() == Some(name) {
        scenario();
        return;
    }
    let out = std::process::Command::new(std::env::current_exe().expect("this test binary"))
        .args([name, "--exact", "--nocapture", "--include-ignored", "--test-threads=1"])
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

#[cfg(not(target_os = "macos"))]
mod pc {
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    use beebeeb_engine::account_binding::Identity;
    use beebeeb_engine::app::{Engine, EngineConfig, SyncStart};
    use beebeeb_engine::config::DesktopConfig;
    use beebeeb_engine::host::{ChannelHost, HostMessage};
    use beebeeb_engine::identity::TEST;
    use beebeeb_engine::keychain::AuthSecretStore;
    use beebeeb_engine::signin::gate::NextAfterSignIn;
    use beebeeb_engine::signin::memory_store::MemoryCredentials;
    use beebeeb_engine::state_db::{OperationKind, PendingOperation, StateDb};
    use beebeeb_engine::state_paths;
    use beebeeb_engine::test_api::fake_account::{FakeAccount, account_handler};
    use beebeeb_engine::test_api::{FakeApi, Handler, Request};
    use zeroize::Zeroizing;

    use crate::common;

    /// A second valid 12-word phrase (a BIP-39 test vector), for account B.
    pub const SECOND_PHRASE: &str = "legal winner thank year wave sausage worth useful legal winner thank yellow";
    /// The release's image after a clean close (no write-ahead log beside it).
    const IMAGE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/state-db-0.8.25/closed");
    /// The file row the release wrote (`fixture.json`: `file_id`, `Documents/fixture.txt`, 41 bytes).
    const FILE_ID: &str = "a0000000-0000-4000-8000-0000000000f1";
    /// The release's plaintext of that file (`fixture.json`: `plaintext_hex`), 41 bytes.
    const FILE_TEXT: &[u8] = b"abcdefghijklmnopqrstuvwxyzabcdefghijklmno";
    /// The due operation the premise adds.
    const OP_ID: &str = "08250000-0000-4000-8000-0000000000d1";

    /// One computer across the upgrade: the fake account, the credential store and the state folder stay; the
    /// engine handle is the app, opened again for every startup.
    pub struct Pc {
        pub account: Arc<FakeAccount>,
        pub phrase: String,
        fake: FakeApi,
        state_dir: PathBuf,
        home: PathBuf,
        stores: MemoryCredentials,
        account_id: String,
        engine: Option<Arc<Engine<MemoryCredentials>>>,
        _messages: Vec<std::sync::mpsc::Receiver<HostMessage>>,
    }

    impl Pc {
        /// Before the upgrade: A signed in with the password, and (`with_key`) the 12 words, so the store holds the
        /// token, the email and the session's owner, and with the key also the wrapped vault key and its owner. The
        /// engine's own sign-in on the fake account writes them. Then that app quits: its handle is dropped.
        /// `second`: B, served by the same fake API (by its token, and a sign-in by its email).
        pub fn signed_in_before_the_upgrade(second: Option<Arc<FakeAccount>>, with_key: bool) -> Pc {
            let s = match second {
                None => common::setup(),
                Some(second) => common::setup_with(move |to_first, _, _| both_accounts(to_first, second)),
            };
            s.sign_in();
            if with_key {
                s.unlock();
            }
            let common::Setup {
                account,
                phrase,
                fake,
                state_dir,
                home,
                stores,
                account_id,
                engine,
                messages,
            } = s;
            drop(engine);
            Pc {
                account,
                phrase,
                fake,
                state_dir,
                home,
                stores,
                account_id,
                engine: None,
                _messages: vec![messages],
            }
        }

        /// The release's `state.db` (a private copy) becomes this PC's, with one due operation added (the image
        /// has none), and the sync folder's local items are made. Returns the folder's items, to compare with.
        pub fn put_the_0_8_25_image(&self) -> Vec<(String, u64)> {
            std::fs::create_dir_all(&self.state_dir).unwrap();
            let state_db = self.state_db();
            for leftover in ["", "-wal", "-shm"] {
                let path = PathBuf::from(format!("{}{leftover}", state_db.display()));
                match std::fs::remove_file(&path) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => panic!("clear {leftover:?}: {error}"),
                }
            }
            let mut copied = 0;
            for entry in std::fs::read_dir(IMAGE).unwrap() {
                let entry = entry.unwrap();
                std::fs::copy(entry.path(), self.state_dir.join(entry.file_name())).unwrap();
                copied += 1;
            }
            assert_eq!(copied, 1, "the closed image is one file");

            let db = self.db();
            assert_eq!(db.owner().unwrap(), None, "the premise: the release recorded no owner");
            assert!(
                db.has_account_data().unwrap(),
                "the premise: the release's rows are account data"
            );
            assert!(
                db.get_file(FILE_ID).unwrap().is_some(),
                "the premise: the release's file row"
            );
            assert_eq!(
                db.list_due_operations(i64::MAX).unwrap().len(),
                0,
                "the image queues nothing"
            );
            db.enqueue_operation(&due_change()).unwrap();
            assert_eq!(self.due_operations(), 1, "the premise: one due operation");
            drop(db);

            let documents = self.folder().join("Documents");
            std::fs::create_dir_all(&documents).unwrap();
            std::fs::write(documents.join("fixture.txt"), FILE_TEXT).unwrap();
            let files = common::listing(&self.folder());
            assert_eq!(files.len(), 2, "the premise: the folder and its file");
            files
        }

        /// The app starts: a new handle on the same store and state folder, restored. The handle before it is
        /// dropped first, so two handles never share the state folder.
        pub fn open_and_restore(&mut self) -> beebeeb_engine::signin::gate::GateState {
            self.engine = None;
            let (host, messages) = ChannelHost::new(|| {});
            let config = EngineConfig {
                account_id: self.account_id.clone(),
                state_dir: self.state_dir.clone(),
                api_base: self.fake.base.clone(),
            };
            let engine = Engine::open(&TEST, config, host, self.stores.clone()).expect("the engine opens");
            self._messages.push(messages);
            let gate = engine.restore();
            self.engine = Some(Arc::new(engine));
            gate
        }

        pub fn engine(&self) -> &Engine<MemoryCredentials> {
            self.engine.as_deref().expect("the app is open")
        }

        /// The person opens the locked screen (Windows Hello's answer, as the lock screen's own tests give it).
        pub fn unlock_the_screen(&self) {
            let asked = self.engine().session_turn();
            assert_eq!(self.engine().unlock_screen_platform_verified(asked), Ok(()));
        }

        /// A signs in again on this handle: the password, then the 12 words (the sign-out removed the key).
        pub fn sign_in_and_unlock(&self) {
            assert_eq!(
                self.engine()
                    .sign_in_password(&self.account.email, Zeroizing::new(common::PASSWORD.into())),
                Ok(NextAfterSignIn::RecoveryPhrase)
            );
            self.engine()
                .unlock_with_phrase(Zeroizing::new(self.phrase.clone()))
                .expect("the account's phrase");
        }

        /// Rule 1 on this handle: a paused start (so the runner sends nothing and the queue count is the start's
        /// alone) keeps everything for A.
        pub fn keeps_everything_at_its_first_start(&self, files: &[(String, u64)]) {
            DesktopConfig::update_for(&TEST, |cfg| {
                cfg.pause_sync = true;
                (true, ())
            })
            .expect("the pause is written");
            assert_eq!(self.owner(), None, "nobody before the first start");
            assert_eq!(
                self.engine().start_sync(self.folder()),
                Ok(SyncStart::Started),
                "the account whose session the store held starts"
            );
            let _ = self.engine().stop();
            assert!(!self.engine().is_syncing());
            assert_eq!(self.owner(), Some(self.this_account()), "A is recorded as the owner");
            assert_eq!(self.due_operations(), 1, "the queue is kept");
            assert!(self.db().get_operation(OP_ID).unwrap().is_some(), "the same operation");
            assert!(
                self.db().get_file(FILE_ID).unwrap().is_some(),
                "the release's file row is kept"
            );
            self.files_are_still_there(files);
        }

        /// A session stored before the email was kept: no email and no owner tags beside the token and the key.
        pub fn forget_whose_session_it_is(&self) {
            let store = self.stores.store_for(&self.account_id);
            store.delete_account_email().unwrap();
            store.delete_session_owner().unwrap();
            store.delete_vault_key_owner().unwrap();
            assert!(store.load_session_token().ok().flatten().is_some(), "the token stays");
            assert!(self.store_holds_a_vault_key(), "the key stays");
        }

        pub fn store_holds_a_vault_key(&self) -> bool {
            self.stores
                .store_for(&self.account_id)
                .load_wrapped_master_key()
                .ok()
                .flatten()
                .is_some()
        }

        pub fn folder(&self) -> PathBuf {
            self.home.join("Beebeeb Test")
        }

        pub fn state_db(&self) -> PathBuf {
            state_paths::state_db_path_from_state_dir(&self.state_dir)
        }

        pub fn db(&self) -> StateDb {
            StateDb::open(self.state_db()).unwrap()
        }

        pub fn owner(&self) -> Option<Identity> {
            self.db().owner().unwrap()
        }

        pub fn due_operations(&self) -> usize {
            self.db().list_due_operations(i64::MAX).unwrap().len()
        }

        pub fn rows(&self) -> usize {
            self.db().list_files().unwrap().len()
        }

        /// A, as the binding records an owner.
        pub fn this_account(&self) -> Identity {
            Identity::new(Some(&self.account.user_id), Some(&self.account.email))
        }

        /// Every item of `before` is still in the folder, every file with its size.
        pub fn files_are_still_there(&self, before: &[(String, u64)]) {
            for (relative, size) in before {
                let path = self.folder().join(relative);
                let found = std::fs::metadata(&path).unwrap_or_else(|_| panic!("{relative} is still there"));
                if found.is_file() {
                    assert_eq!(found.len(), *size, "{relative} is unchanged");
                }
            }
        }
    }

    /// A change the release queued and never sent: the trash of its file, due now.
    fn due_change() -> PendingOperation {
        PendingOperation {
            op_id: OP_ID.into(),
            kind: OperationKind::TrashFile,
            file_id: Some(FILE_ID.into()),
            parent_id: None,
            target_path: Some("Documents/fixture.txt".into()),
            metadata_json: None,
            payload_path: None,
            base_version: None,
            base_object_version_id: None,
            attempts: 0,
            max_attempts: 5,
            next_retry_at: 0,
            last_error: None,
            backup_source_key: None,
            created_at: 1,
            updated_at: 1,
        }
    }

    /// One fake API for both accounts: by the session's token, and a sign-in by the email it names
    /// (as `the_local_state_follows_the_account.rs` serves them).
    fn both_accounts(to_first: Handler, second: Arc<FakeAccount>) -> Handler {
        let to_second = account_handler(second.clone());
        let signing_in_second = Mutex::new(false);
        let handler: Handler = Arc::new(move |request: &Request| {
            let second_asked = match request.bearer() {
                Some(token) => token == second.session_token,
                None => {
                    let mut signing_in = signing_in_second.lock().unwrap();
                    if let Some(email) = request.json().get("email").and_then(|email| email.as_str()) {
                        *signing_in = email == second.email;
                    }
                    *signing_in
                }
            };
            if second_asked {
                to_second(request)
            } else {
                to_first(request)
            }
        });
        handler
    }
}
