//! The re-sign-in rules that the Tauri shell's own tests pinned (`src-tauri/src/reauth_tests/part_1.rs` to `part_5.rs`,
//! ruling R8 and spec §5.6), ported to the engine handle a host calls, on the fake API (`tests/common`). The shell
//! settled a sign-in on a computer that already held an account (in place, a switch, or a kept key replaced) on macOS
//! and Linux; the engine works one account at a time on every OS (`Engine::sign_in_password`'s refusal while one is
//! signed in), and the same account signs in again through the strong unlock (`Engine::unlock_vault_with_password`,
//! decision 2495) after its session ended or over a locked screen. These tests pin what of R8 the engine has on that
//! path: only the session token changes, the replaced session is ended at the server, a sign-in that cannot be stored
//! leaves nothing behind, and a sign-in while another account is signed in asks nothing and changes nothing.
//!
//! One test is ignored until its fix; it names its task.
//!
//! Each test runs in a process of its own (`in_own_process`): the state folder is a process-wide global
//! (`state_paths`), and `tests/common` makes a new one per setup. Everything is invented; nothing reaches the network
//! or the OS credential store. The test build is refused on macOS, so there the binary asserts that refusal only.
//!
//! The mutant each test turns red is named on the test.

mod common;

/// The shell's `a_re_sign_in_never_purges_removes_or_touches_keys`, `a_new_token_replaces_only_the_token_in_memory`,
/// `a_same_account_re_sign_in_swaps_only_the_token_and_changes_nothing_else` and
/// `a_same_account_re_sign_in_after_a_revoked_token_reloads_the_key_this_mac_kept` (R8: "Finder, keys, cache and
/// pending edits all stay"), on the engine's re-sign-in of the same account: the strong unlock, once after the server
/// ended the session (the vault closed, the stored session kept: spec Q5) and once over a locked screen (the key in
/// memory). Each time only the session token changes: the new one is stored, the replaced one is ended at the server,
/// the kept key is in memory again with the same bytes and the same owner, the email, the session's owner, the local
/// data's owner, its rows and its queue are as they were, and the unlock starts no engine itself.
///
/// The fake issues one token per account, so the token the replaced session held is put in the store first (an
/// invented one), as the ended session's own token would be there.
/// Mutants: (a) `key_owner::remove_foreign_key` removes the key of the same account too (`key_owner.rs:167`, the
/// `Ok(Some(true))` arm deletes): the unlock asks for the twelve words; (b) no revoke of the replaced session
/// (`app.rs:617-620`): no logout with the replaced token.
#[cfg(not(target_os = "macos"))]
#[test]
fn a_strong_unlock_of_the_same_account_changes_only_the_session_token() {
    in_own_process(
        "a_strong_unlock_of_the_same_account_changes_only_the_session_token",
        || {
            use beebeeb_engine::signin::gate::{GateState, NextAfterSignIn};

            let s = common::setup();
            s.sign_in();
            s.unlock();
            s.state_db_with_one_row(&s.this_account());
            pc::queue_one_change(&s);
            let before = pc::Kept::now(&s);
            assert!(before.key.is_some(), "the premise: a key is kept on this computer");
            assert_eq!(before.rows, 1, "the premise: one row");
            assert_eq!(before.due, 1, "the premise: one queued change");

            // After the server ended the session: the vault is closed, the stored session and key stay.
            pc::put_the_replaced_token(&s, "an-invented-token-of-the-ended-session");
            pc::end_the_session(&s);
            let from = s.fake.requests().len();
            assert_eq!(
                s.engine
                    .unlock_vault_with_password(zeroize::Zeroizing::new(common::PASSWORD.into())),
                Ok(NextAfterSignIn::Drive),
                "the same account signs in again with the key this computer kept: no twelve words"
            );
            pc::only_the_token_changed(&s, &before, from, "an-invented-token-of-the-ended-session");
            assert_eq!(s.engine.gate(), GateState::Ready);

            // Over a locked screen: the key is in memory, and the same proof replaces the session.
            assert_eq!(s.engine.lock_screen(), Ok(()));
            assert_eq!(s.engine.gate(), GateState::ScreenLocked);
            pc::put_the_replaced_token(&s, "an-invented-token-of-the-locked-session");
            let from = s.fake.requests().len();
            assert_eq!(
                s.engine
                    .unlock_vault_with_password(zeroize::Zeroizing::new(common::PASSWORD.into())),
                Ok(NextAfterSignIn::Drive)
            );
            pc::only_the_token_changed(&s, &before, from, "an-invented-token-of-the-locked-session");
            assert_eq!(s.engine.gate(), GateState::Ready);
        },
    );
}

/// The shell's `a_same_account_re_sign_in_without_a_key_here_asks_for_the_recovery_phrase`: the same account signs in
/// again, and this computer holds no key of it. The token is replaced (the replaced one is ended at the server), no key
/// is invented, the twelve words are asked, and the phrase then proves and stores the key for the NEW token.
/// Mutant: `store_first_sign_in` installs no token (`app.rs:638-640` dropped): the replaced token stays stored.
#[cfg(not(target_os = "macos"))]
#[test]
fn a_strong_unlock_with_no_key_here_replaces_the_token_and_asks_for_the_phrase() {
    in_own_process(
        "a_strong_unlock_with_no_key_here_replaces_the_token_and_asks_for_the_phrase",
        || {
            use beebeeb_engine::keychain::AuthSecretStore;
            use beebeeb_engine::signin::gate::{GateState, NextAfterSignIn};

            let s = common::setup();
            s.sign_in();
            assert_eq!(
                s.engine.gate(),
                GateState::NeedsRecoveryPhrase,
                "the premise: no key here"
            );
            pc::put_the_replaced_token(&s, "an-invented-token-of-the-keyless-session");
            let from = s.fake.requests().len();

            assert_eq!(
                s.engine
                    .unlock_vault_with_password(zeroize::Zeroizing::new(common::PASSWORD.into())),
                Ok(NextAfterSignIn::RecoveryPhrase)
            );
            let stored = s.stores.store_for(&s.account_id).load_session_token().unwrap().unwrap();
            assert!(
                stored.expose_for_request() == s.account.session_token,
                "the new token is stored"
            );
            assert!(
                pc::logouts_from(&s, from)
                    .iter()
                    .any(|token| token == "an-invented-token-of-the-keyless-session"),
                "the replaced session is ended at the server"
            );
            assert!(!s.entries().contains(&"vault key"), "no key is invented");
            assert_eq!(s.engine.gate(), GateState::NeedsRecoveryPhrase);

            s.unlock();
            assert_eq!(
                s.engine.gate(),
                GateState::Ready,
                "the phrase proves the key for the new token"
            );
            assert!(s.entries().contains(&"vault key"));
        },
    );
}

/// The shell's `every_sign_in_method_is_checked_before_it_stores_anything_and_windows_never_reaches_the_check`,
/// `another_account_before_the_first_engine_starts_is_a_switch`, `a_mismatch_changes_nothing_on_this_mac` and
/// `signing_in_twice_as_the_same_account_before_the_first_engine_starts_is_in_place`, as the engine has them: one
/// account at a time, on every OS. While an account is signed in (here A, first without a key, then unlocked), a
/// password sign-in as another account (B) or as A again is refused before anything is asked of the server: no request
/// is made, and A's stored session, email, owner, key and memory are exactly as they were. (Leaving a half-done
/// sign-in to start another is `host_contract.rs:leaving_a_half_done_sign_in_forgets_the_code_step_and_signs_a_keyless_session_out`.)
/// Mutant: the guard at `app.rs:485-489` dropped: B's password legs reach the server and B's token is stored.
#[cfg(not(target_os = "macos"))]
#[test]
fn a_sign_in_while_an_account_is_signed_in_asks_nothing_and_changes_nothing() {
    in_own_process(
        "a_sign_in_while_an_account_is_signed_in_asks_nothing_and_changes_nothing",
        || {
            use std::sync::Arc;

            use beebeeb_engine::signin::gate::{GateState, SignInProblem};
            use beebeeb_engine::test_api::fake_account::FakeAccount;
            use zeroize::Zeroizing;

            let (second, _) = FakeAccount::register("second@example.eu", common::PASSWORD, pc::SECOND_PHRASE);
            let second = Arc::new(second);
            let s = {
                let second = second.clone();
                common::setup_with(move |to_first, _, _| pc::both_accounts(to_first, second))
            };

            for unlocked in [false, true] {
                if unlocked {
                    s.unlock();
                } else {
                    s.sign_in();
                }
                let gate = s.engine.gate();
                let before = pc::Kept::now(&s);
                let entries = s.entries();
                let from = s.fake.requests().len();
                for email in [second.email.as_str(), s.account.email.as_str()] {
                    assert_eq!(
                        s.engine
                            .sign_in_password(email, Zeroizing::new(common::PASSWORD.into())),
                        Err(SignInProblem::Other),
                        "one account at a time (unlocked: {unlocked})"
                    );
                }
                assert_eq!(
                    s.fake.requests().len(),
                    from,
                    "nothing is asked of the server (unlocked: {unlocked})"
                );
                assert_eq!(s.entries(), entries, "the stored session is unchanged");
                before.is_unchanged(&pc::Kept::now(&s), "a refused sign-in");
                let stored = pc::stored_token(&s);
                assert!(stored.as_deref() == Some(s.account.session_token.as_str()), "A's token");
                assert_eq!(s.engine.account_email().as_deref(), Some(s.account.email.as_str()));
                assert_eq!(s.engine.user_id().as_deref(), Some(s.account.user_id.as_str()));
                assert_eq!(s.engine.gate(), gate);
            }
            assert_eq!(s.engine.gate(), GateState::Ready);
            assert_eq!(
                s.engine.keys_for_engine_start().map(|identity| identity.user_id),
                Ok(Some(s.account.user_id.clone())),
                "A's keys, A's session"
            );
        },
    );
}

/// The shell's `every_arm_that_does_not_sign_in_revokes_the_new_session_and_changes_nothing`,
/// `a_re_sign_in_that_fails_says_whether_its_token_was_stored` and
/// `the_swap_calls_its_token_stored_exactly_when_the_keychain_write_has_succeeded`: a sign-in whose token cannot be
/// stored (the credential store refuses the write) stores nothing of the new session, and the session the server just
/// made for it is ended there; once the store takes writes again, the same sign-in goes through.
/// Mutant: no revoke on the store-failure arm (`app.rs:595-598`): no logout with the new token.
#[cfg(not(target_os = "macos"))]
#[test]
fn a_sign_in_whose_token_cannot_be_stored_ends_its_new_session_and_stores_nothing() {
    in_own_process(
        "a_sign_in_whose_token_cannot_be_stored_ends_its_new_session_and_stores_nothing",
        || {
            use beebeeb_engine::signin::gate::{GateState, NextAfterSignIn};
            use zeroize::Zeroizing;

            let s = common::setup();
            s.stores
                .refuse_token_writes(Some("the credential store refused the write"));
            let from = s.fake.requests().len();
            assert!(
                s.engine
                    .sign_in_password(&s.account.email, Zeroizing::new(common::PASSWORD.into()))
                    .is_err(),
                "a token that cannot be stored is no sign-in"
            );
            assert!(s.entries().is_empty(), "nothing of the new session is stored");
            assert_eq!(s.engine.gate(), GateState::SignedOut);
            assert_eq!(s.engine.account_email(), None);
            assert!(
                pc::logouts_from(&s, from)
                    .iter()
                    .any(|token| token == &s.account.session_token),
                "the session the server made for it is ended"
            );

            s.stores.refuse_token_writes(None);
            assert_eq!(
                s.engine
                    .sign_in_password(&s.account.email, Zeroizing::new(common::PASSWORD.into())),
                Ok(NextAfterSignIn::RecoveryPhrase),
                "trying again is safe"
            );
        },
    );
}

/// The shell's `an_unlock_writes_only_while_the_account_it_started_with_is_still_stored` (its sign-out arm): the
/// phrase was being proved for the stored token, and meanwhile that token left the store (as a sign-out's clear would
/// take it). The proof's answer writes nothing: no key next to no session, and the vault does not open. (A token
/// REPLACED meanwhile is `a_token_replaced_during_the_proof_stores_nothing.rs`; a world that did not change is the
/// ordinary unlock.)
/// Mutant: the "still the stored token" check at `app.rs:691-698` dropped: the key is stored with no token beside it.
#[cfg(not(target_os = "macos"))]
#[test]
fn a_token_removed_during_the_phrase_proof_stores_nothing() {
    in_own_process("a_token_removed_during_the_phrase_proof_stores_nothing", || {
        use std::sync::Arc;

        use beebeeb_engine::keychain::AuthSecretStore;
        use beebeeb_engine::signin::gate::GateState;
        use beebeeb_engine::test_api::Handler;
        use zeroize::Zeroizing;

        let s = common::setup_with(|real, stores, slot| {
            let handler: Handler = Arc::new(move |request| {
                if request.route() == "POST /api/v1/auth/verify-recovery-check" {
                    stores.store_for(&slot).delete_session_token().unwrap();
                }
                real(request)
            });
            handler
        });
        s.sign_in();

        assert!(
            s.engine.unlock_with_phrase(Zeroizing::new(s.phrase.clone())).is_err(),
            "the proof was for a token that is no longer stored"
        );
        assert!(!s.entries().contains(&"vault key"), "no key entry");
        assert_ne!(s.engine.gate(), GateState::Ready);
    });
}

/// The shell's `an_unreadable_state_db_is_a_trace_and_settles_nothing` and
/// `an_unreadable_owner_record_or_state_db_settles_nothing_and_the_same_account_then_stays_in_place`, where the engine
/// reads this computer's record of its account: the start. A `state.db` that cannot be read is neither "no owner" nor
/// another account's: the start is refused, the file is left exactly as it was, nothing is registered and no engine
/// runs. Once the record reads again, the same account's start (paused, so it sends nothing) keeps its row and queue.
/// The second start, with the same folder and port, is what shows the first was refused for the unreadable file.
/// Mutant: `LocalState::for_engine_start` answers an open error by removing the file and opening a new one: the first
/// start runs on an empty `state.db` and the file's bytes are gone.
#[cfg(not(target_os = "macos"))]
#[test]
fn an_unreadable_state_db_settles_nothing_and_the_same_account_then_keeps_its_data() {
    in_own_process(
        "an_unreadable_state_db_settles_nothing_and_the_same_account_then_keeps_its_data",
        || {
            use beebeeb_engine::app::SyncStart;
            use beebeeb_engine::config::DesktopConfig;
            use beebeeb_engine::identity::TEST;

            const UNREADABLE: &[u8] = b"an invented file that is not a database, so nothing can read an owner from it";
            let s = common::setup();
            s.sign_in();
            s.unlock();
            DesktopConfig::update_for(&TEST, |cfg| {
                cfg.pause_sync = true;
                (true, ())
            })
            .expect("the pause is written");
            let folder = s.home.join("Beebeeb Test");
            std::fs::create_dir_all(&folder).unwrap();
            assert!(!s.state_db().exists(), "the premise: no state.db yet");
            std::fs::create_dir_all(&s.state_dir).unwrap();
            std::fs::write(s.state_db(), UNREADABLE).unwrap();
            let port = common::Port::free();

            assert!(
                s.engine.start_sync_with(folder.clone(), &port).is_err(),
                "an unreadable record of this computer's account settles nothing"
            );
            assert!(
                std::fs::read(s.state_db()).unwrap() == UNREADABLE,
                "the file is left as it was"
            );
            assert_eq!(port.registers(), 0, "nothing is registered");
            assert!(!s.engine.is_syncing(), "no engine runs");

            // The record reads again: this account's own data, one row and one queued change.
            std::fs::remove_file(s.state_db()).unwrap();
            s.state_db_with_one_row(&s.this_account());
            pc::queue_one_change(&s);
            assert_eq!(
                s.engine.start_sync_with(folder, &port),
                Ok(SyncStart::Started),
                "the same account starts"
            );
            let _ = s.engine.stop();
            let after = pc::Kept::now(&s);
            assert_eq!(after.owner, Some(s.this_account()), "the owner is still this account");
            assert_eq!((after.rows, after.due), (1, 1), "its row and its queue are kept");
        },
    );
}

/// A kept vault key that the server no longer accepts never opens the vault, and the local data stays.
#[cfg(not(target_os = "macos"))]
#[test]
fn a_kept_key_the_server_no_longer_accepts_is_never_installed() {
    in_own_process("a_kept_key_the_server_no_longer_accepts_is_never_installed", || {
        use std::sync::atomic::Ordering;

        use beebeeb_engine::signin::gate::{GateState, NextAfterSignIn};
        use beebeeb_engine::signin::start::EngineStartBlocked;
        use beebeeb_engine::test_api::Reply;

        let (s, key_check) = pc::with_a_key_check_switch(|| Reply::error(400, "invalid_recovery_phrase"));
        s.sign_in();
        s.unlock();
        s.state_db_with_one_row(&s.this_account());
        pc::queue_one_change(&s);
        let before = pc::Kept::now(&s);

        key_check.store(true, Ordering::SeqCst);
        pc::end_the_session(&s);
        assert_ne!(
            s.engine
                .unlock_vault_with_password(zeroize::Zeroizing::new(common::PASSWORD.into())),
            Ok(NextAfterSignIn::Drive),
            "a key the server no longer accepts never opens the vault"
        );
        assert_eq!(
            s.engine.keys_for_engine_start().map(|identity| identity.user_id),
            Err(EngineStartBlocked::NoSession),
            "and never reaches memory"
        );
        assert_ne!(s.engine.gate(), GateState::Ready);
        assert!(!s.engine.is_syncing());
        let after = pc::Kept::now(&s);
        assert_eq!(after.owner, before.owner, "the local data's owner stays");
        assert_eq!(
            (after.rows, after.due),
            (before.rows, before.due),
            "its rows and its queue stay"
        );
    });
}

/// When the server cannot say whether the kept vault key is still the account's, nothing changes.
#[cfg(not(target_os = "macos"))]
#[test]
fn a_kept_key_proof_that_cannot_complete_changes_nothing() {
    in_own_process("a_kept_key_proof_that_cannot_complete_changes_nothing", || {
        use std::sync::atomic::Ordering;

        use beebeeb_engine::app::lock_api::LockState;
        use beebeeb_engine::signin::gate::GateState;
        use beebeeb_engine::test_api::Reply;

        let (s, key_check) = pc::with_a_key_check_switch(|| Reply::error(400, "invalid_recovery_check_format"));
        s.sign_in();
        s.unlock();
        let before = pc::Kept::now(&s);

        key_check.store(true, Ordering::SeqCst);
        pc::put_the_replaced_token(&s, "an-invented-token-of-the-ended-session");
        pc::end_the_session(&s);
        let from = s.fake.requests().len();
        assert!(
            s.engine
                .unlock_vault_with_password(zeroize::Zeroizing::new(common::PASSWORD.into()))
                .is_err(),
            "no answer about the kept key decides nothing"
        );
        assert!(
            pc::stored_token(&s).as_deref() == Some("an-invented-token-of-the-ended-session"),
            "the replaced session stays the stored one"
        );
        before.is_unchanged(&pc::Kept::now(&s), "a proof that could not complete");
        assert_eq!(s.engine.lock_state(), LockState::VaultClosed);
        assert_eq!(s.engine.gate(), GateState::Locked);
        assert!(
            pc::logouts_from(&s, from)
                .iter()
                .any(|token| token == &s.account.session_token),
            "the session the server just made is ended"
        );
    });
}

/// Task 2544: a sign-out ends at the server the session it took off this computer, also when its clear failed.
#[cfg(not(target_os = "macos"))]
#[test]
#[ignore = "task 2544: un-ignore with its fix"]
fn a_sign_out_whose_clear_fails_still_ends_the_session_it_took_off_this_computer() {
    in_own_process(
        "a_sign_out_whose_clear_fails_still_ends_the_session_it_took_off_this_computer",
        || {
            let s = common::setup();
            s.sign_in();
            s.unlock();
            s.stores.fail_lock_flags(true);
            let from = s.fake.requests().len();

            let _ = s.engine.sign_out_keeping_local_data();
            assert!(
                pc::stored_token(&s).is_none(),
                "the premise: the token left this computer"
            );
            assert!(
                pc::logouts_from(&s, from)
                    .iter()
                    .any(|token| token == &s.account.session_token),
                "the session that token belongs to is ended at the server"
            );
        },
    );
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
    const SCENARIO: &str = "BB_REAUTH_FROM_SHELL_SCENARIO";
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
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    use beebeeb_engine::account_binding::Identity;
    use beebeeb_engine::keychain::{AuthSecretStore, SessionToken};
    use beebeeb_engine::state_db::{OperationKind, PendingOperation, StateDb};
    use beebeeb_engine::test_api::fake_account::{FakeAccount, account_handler};
    use beebeeb_engine::test_api::{Handler, Reply, Request};
    use zeroize::Zeroizing;

    use crate::common::{self, Setup};

    /// A second valid 12-word phrase (a BIP-39 test vector), for account B.
    pub const SECOND_PHRASE: &str = "legal winner thank year wave sausage worth useful legal winner thank yellow";
    /// The row `Setup::state_db_with_one_row` writes.
    const FILE_ID: &str = "f0000000-0000-4000-8000-000000000001";
    /// The change [`queue_one_change`] queues.
    const OP_ID: &str = "0a1e0000-0000-4000-8000-0000000000d1";

    /// What a re-sign-in of the same account must leave as it was: the kept key (its bytes, never printed) and its
    /// owner, the session's owner, the stored email, and the local data's owner, rows and queue.
    pub struct Kept {
        pub key: Option<Zeroizing<Vec<u8>>>,
        pub key_owner: Option<String>,
        pub session_owner: Option<String>,
        pub email: Option<String>,
        pub owner: Option<Identity>,
        pub rows: usize,
        pub due: usize,
    }

    impl Kept {
        pub fn now(s: &Setup) -> Kept {
            let store = s.stores.store_for(&s.account_id);
            let (owner, rows, due) = if s.state_db().exists() {
                let db = StateDb::open(s.state_db()).unwrap();
                (
                    db.owner().unwrap(),
                    db.list_files().unwrap().len(),
                    db.list_due_operations(i64::MAX).unwrap().len(),
                )
            } else {
                (None, 0, 0)
            };
            Kept {
                key: store
                    .load_wrapped_master_key()
                    .unwrap()
                    .map(|key| Zeroizing::new(key.expose_for_crypto().to_vec())),
                key_owner: store.load_vault_key_owner().unwrap(),
                session_owner: store.load_session_owner().unwrap(),
                email: store.load_account_email().unwrap(),
                owner,
                rows,
                due,
            }
        }

        /// Every field equal to `after`'s. The key is compared, never formatted.
        pub fn is_unchanged(&self, after: &Kept, what: &str) {
            assert!(self.key == after.key, "{what}: the kept key's bytes are the same");
            assert_eq!(self.key_owner, after.key_owner, "{what}: the key's owner");
            assert_eq!(self.session_owner, after.session_owner, "{what}: the session's owner");
            assert_eq!(self.email, after.email, "{what}: the stored email");
            assert_eq!(self.owner, after.owner, "{what}: the local data's owner");
            assert_eq!(self.rows, after.rows, "{what}: the local data's rows");
            assert_eq!(self.due, after.due, "{what}: the queued changes");
        }
    }

    /// The stored session token, as text (an invented one; never printed by these tests).
    pub fn stored_token(s: &Setup) -> Option<String> {
        s.stores
            .store_for(&s.account_id)
            .load_session_token()
            .unwrap()
            .map(|token| token.expose_for_request().to_string())
    }

    /// The token the replaced session holds, in the store, in place of the fake's one token (which every sign-in
    /// gets back), so a test can tell the replaced token from the new one. Its owner tag is left as it is.
    pub fn put_the_replaced_token(s: &Setup, token: &str) {
        s.stores
            .store_for(&s.account_id)
            .save_session_token(&SessionToken::new(token.to_string()).unwrap())
            .unwrap();
    }

    /// The server ended the session: three refused calls (as the sync loop counts them), then the close the session
    /// watcher makes. The vault is closed; the stored session and key stay (spec Q5).
    pub fn end_the_session(s: &Setup) {
        for _ in 0..3 {
            s.engine.note_call_for_test(true);
        }
        assert!(s.engine.account_session_refused(), "the premise: the session ended");
        assert!(s.engine.close_ended_session().is_some(), "the vault closes");
        assert!(stored_token(s).is_some(), "the stored session stays");
    }

    /// The tokens of the `POST /api/v1/auth/logout` requests the fake got from request `from` on.
    pub fn logouts_from(s: &Setup, from: usize) -> Vec<String> {
        s.fake
            .requests()
            .into_iter()
            .skip(from)
            .filter(|request| request.route() == "POST /api/v1/auth/logout")
            .filter_map(|request| request.bearer().map(str::to_string))
            .collect()
    }

    /// After a strong unlock of the same account: the new token is stored, the replaced one was ended at the server
    /// and the new one was not, the kept key is in memory under the account's id, everything [`Kept`] lists is as
    /// `before`, and the unlock started no engine.
    pub fn only_the_token_changed(s: &Setup, before: &Kept, from: usize, replaced: &str) {
        assert!(
            stored_token(s).as_deref() == Some(s.account.session_token.as_str()),
            "the new token is stored"
        );
        let ended = logouts_from(s, from);
        assert!(
            ended.iter().any(|token| token == replaced),
            "the replaced session is ended at the server"
        );
        assert!(
            !ended.iter().any(|token| token == &s.account.session_token),
            "the new session is not ended"
        );
        before.is_unchanged(&Kept::now(s), "a strong unlock");
        assert_eq!(
            s.engine.keys_for_engine_start().map(|identity| identity.user_id),
            Ok(Some(s.account.user_id.clone())),
            "the kept key is in memory, for this account"
        );
        assert!(!s.engine.is_syncing(), "the unlock starts no engine itself");
    }

    /// One change queued for the row `state_db_with_one_row` wrote, due now, through the engine's own writer.
    pub fn queue_one_change(s: &Setup) {
        let db = StateDb::open(s.state_db()).unwrap();
        db.enqueue_operation(&PendingOperation {
            op_id: OP_ID.into(),
            kind: OperationKind::TrashFile,
            file_id: Some(FILE_ID.into()),
            parent_id: None,
            target_path: Some("/notes.txt".into()),
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
        })
        .unwrap();
    }

    /// The fake account, with a switch: once it is on, the recovery check (`POST /api/v1/auth/verify-recovery-check`,
    /// the route the engine's key proof uses) is answered with `answer` for any key, as a server answers after the
    /// account's key was changed on another device (or cannot answer). Off, the account answers it itself.
    pub fn with_a_key_check_switch(answer: fn() -> Reply) -> (Setup, Arc<AtomicBool>) {
        let switch = Arc::new(AtomicBool::new(false));
        let on = switch.clone();
        let s = common::setup_with(move |real, _, _| {
            let handler: Handler = Arc::new(move |request: &Request| {
                if on.load(Ordering::SeqCst) && request.route() == "POST /api/v1/auth/verify-recovery-check" {
                    return answer();
                }
                real(request)
            });
            handler
        });
        (s, switch)
    }

    /// One fake API for both accounts: by the session's token, and a sign-in by the email it names
    /// (as `upgrade_from_08x.rs` serves them).
    pub fn both_accounts(to_first: Handler, second: Arc<FakeAccount>) -> Handler {
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
