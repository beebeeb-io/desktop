//! A sign-out that did not finish is never taken for one that did (`app/unfinished_sign_out.rs`).
//!
//! - A sign-out refused because the sync engine's stop was not confirmed took the engine out of its slot: its task may
//!   still run with the keys. Every retry is refused the same way (the slot only LOOKS idle), and so is every later
//!   stop, until Beebeeb restarts. After a restart the sign-out clears the stored session and ends it at the server.
//!   Ports the second half of 0.8.x `signout_teardown_tests.rs`
//!   `clear_session_retry_after_unconfirmed_engine_stop_cannot_skip_the_stop_gate`; its first half is in
//!   `start_and_sign_out_from_shell.rs`:
//!   `a_sign_out_whose_engine_stop_is_unconfirmed_is_refused_and_keeps_the_session`.
//! - A clear of the stored session that was cut short (a crash, a power loss, a removal that failed) leaves a key or an
//!   owner tag with no session token. The next start finishes it before the stored session is read, as 0.8.x's
//!   Windows start did. A start that finds a stored session, or nothing left over, changes nothing.
//!
//! Everything is invented: the account, its password and its 12 words (`test_api::fake_account`), the key bytes, the
//! ids and the address below. Nothing reaches the network or the OS credential store. Each scenario runs in a process
//! of its own (`in_own_process`): the state folder is process-wide.

mod common;

/// Runs `scenario` in a child process of this test binary (the state folder is set once per process).
#[cfg(not(target_os = "macos"))]
fn in_own_process(name: &str, scenario: impl FnOnce()) {
    /// Set in the child: the scenario it runs.
    const SCENARIO: &str = "BB_UNFINISHED_SIGN_OUT_SCENARIO";
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

/// The same engine as `common::setup`'s, opened again on the same credential stores, state folder and fake API: what
/// a restart of Beebeeb finds (a new handle has a new account runtime; nothing of the old one's memory). Restored.
#[cfg(not(target_os = "macos"))]
fn restarted(
    s: &common::Setup,
) -> (
    beebeeb_engine::app::Engine<beebeeb_engine::signin::memory_store::MemoryCredentials>,
    std::sync::mpsc::Receiver<beebeeb_engine::host::HostMessage>,
) {
    use beebeeb_engine::app::{Engine, EngineConfig};
    use beebeeb_engine::host::ChannelHost;
    use beebeeb_engine::identity::TEST;

    let (host, messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: s.account_id.clone(),
        state_dir: s.state_dir.clone(),
        api_base: s.fake.base.clone(),
    };
    let engine = Engine::open(&TEST, config, host, s.stores.clone()).expect("the engine opens again");
    engine.restore();
    (engine, messages)
}

/// An engine task the stop cannot end in time (a blocking loop no abort reaches), in the account's engine slot, on a
/// runtime of the test's own. It ends by itself after 15 s, so it does not outlive the scenario's process by much.
#[cfg(not(target_os = "macos"))]
fn hold_an_engine_that_cannot_stop(s: &common::Setup) -> tokio::runtime::Runtime {
    use std::time::{Duration, Instant};

    use beebeeb_engine::runner::EngineRunner;

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
    let acct = s.engine.accounts().active_account().expect("the account");
    *acct.engine.blocking_lock() = Some(EngineRunner::for_test_with_task(task));
    runtime
}

/// A sign-out refused because the engine's stop was not confirmed, then pressed again: the retry is refused with the
/// same words (quit and reopen), nothing is cleared, the session is not ended at the server and stays in memory. After
/// a restart the same sign-out clears all five entries and ends the session at the server.
/// Mutant U-A1: `unfinished_sign_out::stop_of_an_empty_slot` answers `WasNotRunning` whatever the record says (the
/// retry clears the store and ends the session: red). Mutant U-A2: `Engine::stop_runner` (`app.rs`) does not record an
/// unconfirmed stop (no `engine_stop_unconfirmed.store(true, …)`): the retry goes through: red.
#[cfg(not(target_os = "macos"))]
#[test]
fn a_sign_out_retry_after_an_unconfirmed_stop_is_refused_until_a_restart() {
    in_own_process(
        "a_sign_out_retry_after_an_unconfirmed_stop_is_refused_until_a_restart",
        || {
            use std::sync::atomic::Ordering;

            use beebeeb_engine::account_binding::ENGINE_STOP_UNCONFIRMED;
            use beebeeb_engine::app::lock_api::LockState;

            let s = common::setup();
            s.sign_in();
            s.unlock();
            let acct = s.engine.accounts().active_account().expect("the account");
            let stuck = hold_an_engine_that_cannot_stop(&s);
            let from = s.fake.requests().len();

            assert_eq!(
                s.engine.sign_out_keeping_local_data(),
                Err(ENGINE_STOP_UNCONFIRMED.to_string()),
                "the premise: the first sign-out is refused"
            );
            assert!(
                acct.engine.blocking_lock().is_none(),
                "the premise: the stop took the engine out of its slot"
            );
            assert!(
                acct.engine_stop_unconfirmed.load(Ordering::SeqCst),
                "the premise: the stop is held as unconfirmed"
            );

            assert_eq!(
                s.engine.sign_out_keeping_local_data(),
                Err(ENGINE_STOP_UNCONFIRMED.to_string()),
                "the retry is refused the same way: an empty slot is not a confirmed stop"
            );
            assert_eq!(s.entries(), common::ENTRIES.to_vec(), "nothing was cleared");
            assert!(
                !s.routes_from(from).iter().any(|route| route.contains("/auth/logout")),
                "the session was not ended at the server: {:?}",
                s.routes_from(from)
            );
            assert_eq!(s.engine.lock_state(), LockState::Open, "the session stays in memory");
            stuck.shutdown_background();

            let (again, _messages) = restarted(&s);
            let from = s.fake.requests().len();
            assert_eq!(
                again.sign_out_keeping_local_data(),
                Ok(()),
                "after a restart the sign-out goes through"
            );
            assert_eq!(s.entries(), Vec::<&str>::new(), "all five entries are cleared");
            assert!(
                s.routes_from(from).iter().any(|route| route.contains("/auth/logout")),
                "the session was ended at the server: {:?}",
                s.routes_from(from)
            );
        },
    );
}

/// After a stop that was not confirmed, every later stop of this engine says `Unconfirmed`, never `WasNotRunning`: the
/// plain stop, and the vault lock's (its `stopped`, which the lock reports). Mutants U-A1 and U-A2, as above.
#[cfg(not(target_os = "macos"))]
#[test]
fn every_stop_after_an_unconfirmed_one_says_unconfirmed() {
    in_own_process("every_stop_after_an_unconfirmed_one_says_unconfirmed", || {
        use beebeeb_engine::app::SyncStop;

        let s = common::setup();
        s.sign_in();
        s.unlock();
        let stuck = hold_an_engine_that_cannot_stop(&s);

        assert_eq!(s.engine.stop(), SyncStop::Unconfirmed, "the premise");
        assert_eq!(
            s.engine.stop(),
            SyncStop::Unconfirmed,
            "a second stop: the task may still run"
        );
        assert_eq!(
            s.engine.lock(),
            SyncStop::Unconfirmed,
            "the vault lock's stop says the same"
        );
        stuck.shutdown_background();
    });
}

/// The account's slot holds a vault key, its owner tag, the account's address and the session's owner tag, but no
/// session token: a clear that was cut short after its first removal. The flat layout of a 0.8.x PC holds the same
/// leftovers (a key and an address, and the mark that says it is retired), with no token. The next start finishes
/// both clears: nothing is left in the credential store, and the window says signed out.
/// Mutant U-B1: `Engine::vault_at_start` does not call `key_owner::complete_pending_clear` (the account's slot keeps
/// its four leftovers: red). Mutant U-B2: the finish skips the flat layout (its three items stay: red).
#[cfg(not(target_os = "macos"))]
#[test]
fn a_clear_cut_short_is_finished_at_the_next_start() {
    in_own_process("a_clear_cut_short_is_finished_at_the_next_start", || {
        use beebeeb_engine::keychain::{AuthSecretStore, SecretBytes};
        use beebeeb_engine::signin::gate::GateState;

        /// The flat layout's slot in the test stores (`app::credential_stores::LEGACY_FLAT_SLOT`).
        const FLAT: &str = "<flat layout>";
        let s = common::setup_with(|handler, stores, account_id| {
            let slot = stores.store_for(&account_id);
            slot.save_wrapped_master_key(SecretBytes::new_master_key(&[7u8; 32]))
                .unwrap();
            slot.save_vault_key_owner("00000000-0000-4000-8000-00000000000a")
                .unwrap();
            slot.save_account_email("left.behind@example.eu").unwrap();
            slot.save_session_owner("00000000-0000-4000-8000-00000000000a").unwrap();
            let flat = stores.store_for(FLAT);
            flat.save_wrapped_master_key(SecretBytes::new_master_key(&[8u8; 32]))
                .unwrap();
            flat.save_account_email("left.behind@example.eu").unwrap();
            flat.save_session_owner("flat layout retired").unwrap();
            assert_eq!(stores.len(), 7, "the premise: seven items left over, no token");
            handler
        });

        assert_eq!(
            s.entries(),
            Vec::<&str>::new(),
            "the account's slot is cleared at the start"
        );
        let flat = s.stores.store_for(FLAT);
        assert!(
            flat.load_wrapped_master_key().unwrap().is_none()
                && flat.load_account_email().unwrap().is_none()
                && flat.load_session_owner().unwrap().is_none(),
            "the flat layout is cleared at the start"
        );
        assert!(
            s.stores.is_empty(),
            "{} items are left in the credential store",
            s.stores.len()
        );
        assert_eq!(s.engine.gate(), GateState::SignedOut);
    });
}

/// A start finds nothing to finish: with nothing stored it writes nothing; with a whole stored session (token, key,
/// address, both owner tags) it keeps all five, and the account opens signed in.
/// Mutant U-B3: the finish clears the slot whatever it holds (`vault.clear_session()` in place of
/// `complete_pending_clear`): the stored session is gone after the restart: red.
#[cfg(not(target_os = "macos"))]
#[test]
fn a_start_with_a_stored_session_or_nothing_left_changes_nothing() {
    in_own_process("a_start_with_a_stored_session_or_nothing_left_changes_nothing", || {
        use beebeeb_engine::app::lock_api::LockState;

        let s = common::setup();
        assert!(s.stores.is_empty(), "nothing stored, nothing written at the start");

        s.sign_in();
        s.unlock();
        assert_eq!(s.entries(), common::ENTRIES.to_vec(), "the premise: a whole session");
        let (again, _messages) = restarted(&s);
        assert_eq!(s.entries(), common::ENTRIES.to_vec(), "all five stay");
        assert_ne!(again.lock_state(), LockState::SignedOut, "it opens signed in");
    });
}

#[cfg(target_os = "macos")]
#[test]
fn the_test_build_is_refused_on_macos() {
    common::assert_the_test_build_is_refused();
}
